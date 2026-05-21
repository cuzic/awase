// Windows 専用クレート — 非 Windows では空クレートとしてコンパイルされる
#![cfg(windows)]
// Win32 API (フック, SendInput, SetTimer 等) の使用に unsafe が必須
#![allow(unsafe_code)]
// Win32 API の型キャスト (usize → i32 等) は OS の ABI 制約により不可避
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    // SingleThreadCell は &self → &mut T を返すが、シングルスレッド保証下で安全
    clippy::mut_from_ref,
    // コールバック型定義が複雑になるのは Win32 API の設計上避けられない
    clippy::type_complexity
)]

//! Windows 固有のプラットフォーム実装クレート。
//!
//! キーボードフック、出力、IME 制御、システムトレイ、フォーカス判定など
//! すべての Win32 API 依存コードを集約する。

pub mod autostart;
pub mod executor;
pub mod focus;
pub mod hook;
pub mod ime;
pub mod ime_diagnostic;
pub mod ime_observations;
pub mod observer;
pub mod output;
pub mod platform;
pub mod runtime;
pub mod scanmap;
pub mod timer;
pub mod tray;
pub mod tsf;
pub mod vk;
pub mod win32;

pub use runtime::{LayoutEntry, Runtime};
pub use win32_async::SingleThreadCell;

use std::sync::atomic::AtomicBool;

use awase::engine::InputModeState;
use awase::types::{AppKind, FocusKind, RawKeyEvent};

pub use crate::tsf::probe_bridge::{
    OUTPUT_ACTIVE, OUTPUT_PENDING_QUEUE, WM_DRAIN_OUTPUT_QUEUE, post_drain_output_queue,
};

// ── クロススレッド共有グローバル状態 ──
//
// Ctrl+C ハンドラ（別スレッド）からアクセスされるため、Atomic 型でなければならない。

/// メインスレッド ID（Ctrl+C ハンドラから WM_QUIT を送るため）
pub static MAIN_THREAD_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Ctrl+C 受信フラグ
pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 管理者権限フラグ（起動時に設定、メニュー表示で参照）
pub static ELEVATED: AtomicBool = AtomicBool::new(false);

// OBS_* グローバルは tsf/observer.rs で定義。後方互換のため re-export する。
pub use crate::tsf::observer::{
    COMPOSITION_PROBE_SEQ, OBS_FOCUS_NAMECHANGE_SEQ, OBS_GJI_CANDIDATE_SHOW_SEQ,
    OBS_GJI_CANDIDATE_VISIBLE,
};

/// raw TSF literal 検出後の回収ペイロード。
///
/// バックスペース数とローマ字再送文字列を一括管理する。
/// WM_DRAIN_OUTPUT_QUEUE ハンドラが `flush_raw_tsf_literal_recovery()` で消費する。
#[derive(Debug)]
pub struct RawTsfLiteralPending {
    /// 送信すべきバックスペースの数
    pub backs: std::sync::atomic::AtomicUsize,
    /// 再送すべきローマ字文字列（空文字列 = 再送なし）
    pub romaji: std::sync::Mutex<String>,
}

impl RawTsfLiteralPending {
    const fn new() -> Self {
        Self {
            backs: std::sync::atomic::AtomicUsize::new(0),
            romaji: std::sync::Mutex::new(String::new()),
        }
    }
}

pub static RAW_TSF_LITERAL: RawTsfLiteralPending = RawTsfLiteralPending::new();

// ── PlatformState: シングルスレッド上の全状態を集約 ──

/// IME 状態検出の連続失敗がこの回数以上になると Engine を非活性にする。
///
/// ポーリング間隔 500ms × 3 = 1.5秒。一時的な検出失敗は許容しつつ、
/// 長時間の乖離（実際は IME OFF なのにキャッシュが ON のまま）を防ぐ。
pub const IME_DETECT_MISS_THRESHOLD: u32 = 3;


/// `Preconditions.ime_on` を最後に更新したソース。
///
/// Phase 2 の `ImeObservations + resolve()` で優先度判定に使用する準備として記録する。
/// 現時点では診断・ログ用途のみ（動作への影響なし）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShadowSource {
    /// 初期化値（まだ一度も観測されていない）
    #[default]
    Init,
    /// 物理 IME キー押下（半角/全角等）— ユーザーの明示的操作
    PhysicalImeKey,
    /// config 由来の同期キー（sync_direction）
    SyncKey,
    /// `ImeEffect::SetOpen` (Engine の判断による強制設定)
    SetOpenRequest,
    /// IME observer ポーリング（バックグラウンド観測）
    ObserverPoll,
    /// フォーカス変更直後の高速プローブ
    FocusProbe,
    /// panic_reset（強制リセット）
    PanicReset,
    /// IMM broken アプリ切替補正（Chrome 等）
    ImmBrokenFix,
    /// per-HWND IME キャッシュからの復元（フォーカス切り替え時の即時復元）
    HwndCache,
}

/// 環境前提条件（IME 状態・入力方式・日本語判定）
#[derive(Debug)]
pub struct Preconditions {
    /// IME が ON か（shadow 追跡含む、Observer ポーリングで実際の OS 状態に収束）
    pub ime_on: bool,
    /// `ime_on` を最後に更新したソース（Phase 2 解決関数向けの診断情報）
    pub ime_on_source: ShadowSource,
    /// 入力モード（ローマ字 / かな / 不明）
    pub input_mode: InputModeState,
    /// 日本語 IME がアクティブか
    pub is_japanese_ime: bool,
    /// 直前の conversion_mode（ROMAN ビット消失によるかな切替検出用）
    /// None = まだ一度も取得できていない
    pub prev_conversion_mode: Option<u32>,
    /// IME 状態検出の連続失敗回数。
    ///
    /// `detect_ime_state()` が `ime_on = None` を返すたびにインクリメントされ、
    /// 検出成功時またはシャドウ更新（ユーザー操作）時にリセットされる。
    /// [`IME_DETECT_MISS_THRESHOLD`] に達すると `refresh_ime_state_cache` が
    /// IME を強制 ON にして Engine の活性状態を維持する。
    ///
    /// ## 発火条件の実態
    /// Chrome / WezTerm / Windows Terminal など **既知の** アプリでは発火しない：
    /// - TSF native ウィンドウ（WezTerm, Windows Terminal）: `is_tsf_native` 分岐で
    ///   miss_count のインクリメントをスキップ
    /// - `ImmCapability::Broken` 学習済みクラス: `skip_imm_query=true` で Phase 3 自体を迂回
    /// - Chrome 等 IMM32 が動くアプリ: 検出が普通に成功するので count は増えない
    ///
    /// 実際に増えるのは「**未知の IMM-broken アプリへの初回フォーカス時だけ**」。
    /// 閾値到達後 `ImmCapability::Broken` として学習されると、以降は発火しなくなる。
    pub ime_detect_miss_count: u32,
    /// IME 強制 ON 後のガードフラグ。2 つの独立した用途がある。
    ///
    /// ## 用途 A — 未知 IMM-broken アプリの初回ブートストラップ（Phase 3.5）
    /// 未知アプリへの初回フォーカス時に `set_ime_open(true)` を呼んだ後、
    /// 次の `observe()` が即座に上書きしないよう 1 サイクルだけ保護する。
    /// 検出成功（または `ImmCapability::Broken` 学習完了）後にクリアされる。
    ///
    /// ## 用途 B — `panic_reset()` 直後の上書き防止
    /// パニックリセットで `ime_on=true` を書き込んだ直後に `refresh_ime_state_cache`
    /// が走ると stale な `observe()` の結果に上書きされてしまう。これを防ぐ。
    /// 次の検出成功時に自然にクリアされる。
    ///
    /// いずれも「awase が恒常的に SSOT になる」わけではなく、
    /// **一時的な遷移期間中だけ OS 検出結果を無視する** という設計。
    pub ime_force_on_guard: bool,
}

impl Preconditions {
    /// `ime_on` と `ime_on_source` をまとめて更新する。
    pub fn set_ime_on(&mut self, value: bool, source: ShadowSource) {
        self.ime_on = value;
        self.ime_on_source = source;
    }
}

/// フックルーティング状態（キーペア追跡・再入ガード）
#[derive(Debug)]
pub struct HookRoutingState {
    /// Engine に送った KeyDown を記録するビットセット（VK 0-255）
    pub sent_to_engine: [u64; 4],
    /// TrackOnly で送った KeyDown を記録するビットセット
    pub track_only_keys: [u64; 4],
    /// 再入ガード
    pub in_callback: bool,
    /// IME 制御コンボ直後の Ctrl バイパス抑制フラグ。
    /// Ctrl+Henkan/Muhenkan 消費後、Ctrl がまだ押されている間の文字キーを
    /// ショートカットとして Bypass しない。Ctrl KeyUp で解除。
    pub suppress_ctrl_bypass: bool,
}

/// フック設定（親指キー VK コード）
#[derive(Debug)]
pub struct HookConfig {
    pub left_thumb_vk: u16,
    pub right_thumb_vk: u16,
}

/// IME 遷移ガード状態（IME トグルキー押下中のキーバッファリング）
#[derive(Debug)]
pub struct ImeGuardState {
    pub active: bool,
    pub deferred_keys: Vec<(RawKeyEvent, awase::engine::input_tracker::PhysicalKeyState)>,
}

/// 修飾キーのフック追跡状態（同時押し判定用）
///
/// `GetAsyncKeyState` はフックコールバック内でタイミングにより
/// Ctrl の押下を検出できないことがある。フックが受け取った
/// KeyDown/KeyUp イベントから独自に追跡することで、
/// Ctrl+Henkan 等のコンボキーを確実に検出する。
#[derive(Debug)]
pub struct ModifierTiming {
    pub ctrl_down: bool,
    pub ctrl_up_tick: u64,
    pub alt_down: bool,
    pub alt_up_tick: u64,
}

impl ModifierTiming {
    /// 猶予期間（ミリ秒）: KeyUp 後この期間内なら「まだ押されている」と判定
    pub const GRACE_MS: u64 = 150;

    pub fn new() -> Self {
        Self { ctrl_down: false, ctrl_up_tick: 0, alt_down: false, alt_up_tick: 0 }
    }

    pub fn is_ctrl_active(&self, now_tick: u64) -> bool {
        self.ctrl_down || now_tick.saturating_sub(self.ctrl_up_tick) < Self::GRACE_MS
    }

    pub fn is_alt_active(&self, now_tick: u64) -> bool {
        self.alt_down || now_tick.saturating_sub(self.alt_up_tick) < Self::GRACE_MS
    }

    /// コンボキー消費後に猶予をクリアする。
    ///
    /// Ctrl/Alt コンボが消費されたら猶予を維持する意味がないため、
    /// 直後のキーが `OsModifierHeld` でバイパスされるのを防ぐ。
    pub fn clear_grace(&mut self) {
        self.ctrl_up_tick = 0;
        self.alt_up_tick = 0;
    }
}

/// Platform 層の全状態を集約する構造体。
///
/// シングルスレッド（メインスレッド＋フックコールバック）からのみアクセスされる。
/// `APP: SingleThreadCell<Runtime>` 経由で保持される。
#[derive(Debug)]
pub struct PlatformState {
    pub preconditions: Preconditions,
    pub hook: HookRoutingState,
    pub hook_config: HookConfig,
    pub focus_kind: FocusKind,
    pub app_kind: AppKind,
    pub last_hook_activity_ms: u64,
    pub hook_event_count: u64,
    pub focus_debounce_ms: u32,
    pub ime_poll_interval_ms: u32,
    pub ime_guard: ImeGuardState,
    pub modifier_timing: ModifierTiming,
    /// フォーカス切替直後フラグ。
    ///
    /// フォーカス変更を検知したときに `true` にセットされる。
    /// 次のキーストローク到着時に同期プローブ（高速 IME 状態検出）を実行し、
    /// preconditions を即座に更新してからキーを処理する。
    /// これにより「古い preconditions でキーが処理される」ギャップを解消する。
    pub focus_transition_pending: bool,
    /// 最後にフォアグラウンドプロセスが変わった時刻（ms, GetTickCount 系）。
    /// IME 診断ログで「フォーカス変更からの経過時間」を表示するために使う。
    pub last_focus_change_ms: u64,
    /// OS probe / observe から得た生の IME ON/OFF 観測値（ユーザー意図とは別管理）。
    ///
    /// `fast_ime_probe` や `observe()` の生の結果をここに記録する。
    /// `preconditions.ime_on`（engine_active の判断基準）とは意図的に分離してあり、
    /// `user_enabled=true` のとき OS probe が false を返しても
    /// engine を deactivate しないための根拠として使う。
    pub os_ime_on: Option<bool>,
    /// 各ソースの最新観測値（Phase 2: 観測と判断の分離）。
    ///
    /// `ime_on` の最終値は `ImeObservations::resolve_and_clear()` で一括決定される。
    pub ime_observations: ime_observations::ImeObservations,
}

impl PlatformState {
    /// デフォルト値で初期化する
    pub fn new() -> Self {
        Self {
            preconditions: Preconditions {
                ime_on: true,        // 安全側: ON で初期化
                ime_on_source: ShadowSource::Init,
                input_mode: InputModeState::ObservedRomaji, // デフォルト: ローマ字入力
                is_japanese_ime: true, // デフォルト: 日本語
                prev_conversion_mode: None,
                ime_detect_miss_count: 0,
                ime_force_on_guard: false,
            },
            hook: HookRoutingState {
                sent_to_engine: [0u64; 4],
                track_only_keys: [0u64; 4],
                in_callback: false,
                suppress_ctrl_bypass: false,
            },
            hook_config: HookConfig {
                left_thumb_vk: 0x1D,  // VK_NONCONVERT
                right_thumb_vk: 0x1C, // VK_CONVERT
            },
            focus_kind: FocusKind::Undetermined,
            app_kind: AppKind::Win32,
            last_hook_activity_ms: 0,
            hook_event_count: 0,
            focus_debounce_ms: 50,
            ime_poll_interval_ms: 500,
            ime_guard: ImeGuardState { active: false, deferred_keys: Vec::new() },
            modifier_timing: ModifierTiming::new(),
            focus_transition_pending: false,
            last_focus_change_ms: 0,
            os_ime_on: None,
            ime_observations: ime_observations::ImeObservations::default(),
        }
    }
}

impl Default for PlatformState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformState {
    /// `ime_observations.resolve_and_clear()` を実行して `preconditions.ime_on` を更新する。
    ///
    /// 各観測ソースが値を書き込んだ直後に呼ぶ。これにより `preconditions.ime_on` は
    /// 常に最新の観測値を反映する。
    pub fn apply_ime_observations(&mut self, user_enabled: bool) {
        let current = self.preconditions.ime_on;
        let is_japanese = self.preconditions.is_japanese_ime;
        if let Some((val, src)) = self.ime_observations.resolve_and_clear(current, user_enabled, is_japanese) {
            self.preconditions.set_ime_on(val, src);
        }
    }
}

/// APP グローバル — シングルスレッド専用
pub static APP: SingleThreadCell<Runtime> = SingleThreadCell::new();

/// 統合 IME リフレッシュタイマー ID
///
/// フォーカスデバウンス (50ms) と定期ポーリング (500ms) を統合。
/// `schedule_ime_refresh(delay_ms)` で遅延を指定してリセットする。
/// refresh 完了後に自動的に `ime_poll_interval_ms` で再スケジュールされる。
pub const TIMER_IME_REFRESH: usize = 101;

/// フック消失ウォッチドッグタイマー ID（IME ポーリングとは独立）
pub const TIMER_HOOK_WATCHDOG: usize = 102;

/// スリープ復帰 / セッションアンロック後の遅延リカバリタイマー ID
///
/// 復帰直後は OS や IME サービスがまだ回復途中で、ブロッキング Win32 API が
/// メッセージループをハングさせる恐れがある。2秒遅延して安全に復帰処理を行う。
pub const TIMER_POWER_RESUME: usize = 103;

/// 設定リロード用カスタムメッセージ（設定 GUI から `PostMessageW` で送信される）
pub const WM_RELOAD_CONFIG: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 10;

/// IME 制御キー後の遅延キー再処理用カスタムメッセージ
pub const WM_PROCESS_DEFERRED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 11;

/// UIA 非同期判定完了通知用カスタムメッセージ
pub const WM_FOCUS_KIND_UPDATE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 12;

/// フックで IME 制御キーを検出した際の即時キャッシュ更新要求
pub const WM_IME_KEY_DETECTED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 14;

/// フックコールバックからキューされた Effects の実行要求
pub const WM_EXECUTE_EFFECTS: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 15;

/// パニックリセット要求（IME 関連キー連打検出時にフックから PostMessage）
pub const WM_PANIC_RESET: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 16;

/// 多重起動検出時に新インスタンスから既存インスタンスに送る通知
///
/// 既存インスタンスはこのメッセージを受けるとタスクトレイにバルーン通知を表示し、
/// ユーザーに「すでに起動している」ことを知らせる。
pub const WM_DUPLICATE_INSTANCE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 17;

/// キーイベントを SendInput で再注入する（IME OFF 時の遅延キー用）
///
/// INJECTED_MARKER 付きなのでフックに再捕捉されない。
///
/// # Safety
/// Win32 API (`send_input_safe`) を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn reinject_key(event: &RawKeyEvent) {
    use crate::output::INJECTED_MARKER;
    use awase::types::KeyEventType;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
        VIRTUAL_KEY,
    };

    let is_keyup = matches!(event.event_type, KeyEventType::KeyUp);

    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(event.vk_code.0),
                wScan: event.scan_code.0 as u16,
                dwFlags: if is_keyup {
                    KEYEVENTF_KEYUP | KEYEVENTF_SCANCODE
                } else {
                    KEYEVENTF_SCANCODE
                },
                time: 0,
                dwExtraInfo: INJECTED_MARKER,
            },
        },
    };
    win32::send_input_safe(&[input]);
}
