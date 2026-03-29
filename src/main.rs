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

mod hook;
mod ime;
mod output;
mod single_thread_cell;
mod tray;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::core::{Interface, VARIANT};
use windows::Win32::UI::Accessibility::{
    AccessibleObjectFromWindow, CUIAutomation, IAccessible, IUIAutomation,
    IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationValuePattern,
    SetWinEventHook, HWINEVENTHOOK,
    UIA_ButtonControlTypeId, UIA_DocumentControlTypeId,
    UIA_EditControlTypeId, UIA_HyperlinkControlTypeId, UIA_ImageControlTypeId,
    UIA_ListItemControlTypeId, UIA_MenuBarControlTypeId, UIA_MenuControlTypeId,
    UIA_MenuItemControlTypeId, UIA_ProgressBarControlTypeId, UIA_ScrollBarControlTypeId,
    UIA_SeparatorControlTypeId, UIA_SliderControlTypeId, UIA_StatusBarControlTypeId,
    UIA_TabControlTypeId, UIA_TabItemControlTypeId, UIA_TextControlTypeId,
    UIA_TextPatternId, UIA_TitleBarControlTypeId, UIA_ToolBarControlTypeId,
    UIA_TreeItemControlTypeId, UIA_ValuePatternId,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext, ImmSetOpenStatus};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetClassNameW, GetGUIThreadInfo, GetMessageW, GetWindowLongW,
    GetWindowThreadProcessId, GWL_EXSTYLE, GWL_STYLE, GUITHREADINFO, KillTimer, PostMessageW,
    PostQuitMessage, SetTimer, MSG, WM_APP, WM_COMMAND, WM_HOTKEY, WM_INPUTLANGCHANGE, WM_TIMER,
};

use awase::config::{vk_name_to_code, AppConfig, FocusOverrides};
use awase::engine::{ContextChange, Engine, FocusKind, TIMER_PENDING, TIMER_SPECULATIVE};
use awase::ngram::NgramModel;
use awase::types::{KeyEventType, RawKeyEvent};
use awase::yab::YabLayout;
use timed_fsm::{dispatch, ActionExecutor, TimedStateMachine, TimerRuntime};

use crate::hook::CallbackResult;
use crate::ime::{HybridProvider, ImeProvider};
use crate::output::Output;
use crate::single_thread_cell::SingleThreadCell;
use crate::tray::SystemTray;

/// 有効/無効切り替えホットキー ID
const HOTKEY_ID_TOGGLE: i32 = 1;

/// 手動フォーカスオーバーライドホットキー ID (Ctrl+Shift+F11)
const HOTKEY_ID_FOCUS_OVERRIDE: i32 = 2;

/// 設定リロード用カスタムメッセージ（設定 GUI から `PostMessageW` で送信される）
const WM_RELOAD_CONFIG: u32 = WM_APP + 10;

/// IME 制御キー後の遅延キー再処理用カスタムメッセージ
const WM_PROCESS_DEFERRED: u32 = WM_APP + 11;

/// UIA 非同期判定完了通知用カスタムメッセージ
const WM_FOCUS_KIND_UPDATE: u32 = WM_APP + 12;

/// Undetermined + IME ON バッファリングのタイムアウト用カスタムメッセージ
/// （将来的に `PostMessageW` で明示送信する場合に使用）
#[allow(dead_code)]
const WM_BUFFER_TIMEOUT: u32 = WM_APP + 13;

/// Undetermined + IME ON バッファリングのタイマー ID
const TIMER_UNDETERMINED_BUFFER: usize = 100;

/// IME の未確定状態を確認する（engine から呼び出し用）
#[allow(dead_code)]
pub fn check_ime_composing() -> bool {
    unsafe { IME.get_ref().map_or(false, |ime| ime.is_composing()) }
}

static ENGINE: SingleThreadCell<Engine> = SingleThreadCell::new();
static OUTPUT: SingleThreadCell<Output> = SingleThreadCell::new();
static IME: SingleThreadCell<HybridProvider> = SingleThreadCell::new();
static TRAY: SingleThreadCell<SystemTray> = SingleThreadCell::new();

/// 利用可能な配列の一覧（名前, `YabLayout`, 左親指VK, 右親指VK）
static LAYOUTS: SingleThreadCell<Vec<(String, YabLayout, u16, u16)>> = SingleThreadCell::new();

/// IME 制御キー直後のガードフラグ（true: 後続キーを遅延処理する）
static IME_TRANSITION_GUARD: SingleThreadCell<bool> = SingleThreadCell::new();

/// ガード中に遅延されたキーイベントのバッファ
static DEFERRED_KEYS: SingleThreadCell<Vec<RawKeyEvent>> = SingleThreadCell::new();

/// UIA 非同期判定リクエスト送信チャネル（メインスレッドからのみアクセス）
static UIA_SENDER: SingleThreadCell<mpsc::Sender<SendableHwnd>> = SingleThreadCell::new();

/// タイピングパターン検出用トラッカー
static KEY_PATTERN_TRACKER: SingleThreadCell<KeyPatternTracker> = SingleThreadCell::new();

/// Undetermined + IME ON 時のバッファリング中フラグ
static UNDETERMINED_BUFFERING: SingleThreadCell<bool> = SingleThreadCell::new();

/// IME OFF 時の記憶バッファ（PassThrough 済みキー）
static PASSTHROUGH_MEMORY: SingleThreadCell<Vec<RawKeyEvent>> = SingleThreadCell::new();

/// フォーカス判定結果のキャッシュ（メインスレッドからのみアクセス）
static FOCUS_CACHE: SingleThreadCell<FocusCache> = SingleThreadCell::new();

/// config.toml の永続フォーカスオーバーライド設定
static FOCUS_OVERRIDES: SingleThreadCell<FocusOverrides> = SingleThreadCell::new();

/// UIA 非同期判定完了時にキャッシュを更新するための直前のフォーカス情報
static LAST_FOCUS_INFO: SingleThreadCell<(u32, String)> = SingleThreadCell::new();

/// フォーカス判定キャッシュのエントリ
/// 判定結果のソース（TTL と優先順位を決定する）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DetectionSource {
    /// Phase 1-2 同期判定（TTL: 5分、優先度: 最低）
    Automatic = 0,
    /// Phase 3 UIA 非同期判定（TTL: 5分）
    UiaAsync = 1,
    /// タイピングパターン推定（TTL: 30分）
    TypingPatternInferred = 2,
    /// IME/親指キー検出による推定（TTL: 30分）
    ImeKeyInferred = 3,
    /// ユーザー手動オーバーライド（TTL: 24時間、優先度: 最高）
    UserOverride = 4,
}

impl DetectionSource {
    /// ソースに応じた TTL（秒）
    const fn ttl_secs(self) -> u64 {
        match self {
            Self::Automatic | Self::UiaAsync => 300,         // 5分
            Self::TypingPatternInferred | Self::ImeKeyInferred => 1800, // 30分
            Self::UserOverride => 86400,                     // 24時間
        }
    }
}

struct FocusCacheEntry {
    kind: FocusKind,
    source: DetectionSource,
    timestamp: Instant,
}

/// フォーカス判定結果のキャッシュ
///
/// `(process_id, class_name)` をキーとして判定結果を保持する。
/// 同じコントロールへの再フォーカス時に UIA 非同期判定を省略できる。
/// ソース別の TTL と優先順位により、高優先エントリは低優先で上書きされない。
struct FocusCache {
    entries: HashMap<(u32, String), FocusCacheEntry>,
}

impl FocusCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// キャッシュを検索する。未登録または期限切れなら `None` を返す。
    fn get(&self, process_id: u32, class_name: &str) -> Option<FocusKind> {
        let key = (process_id, class_name.to_string());
        self.entries.get(&key).and_then(|entry| {
            if entry.timestamp.elapsed().as_secs() < entry.source.ttl_secs() {
                Some(entry.kind)
            } else {
                None
            }
        })
    }

    /// 判定結果をキャッシュに格納する。
    ///
    /// - `Undetermined` は格納しない。
    /// - 既存エントリより低優先のソースでは上書きしない（有効期限内の場合）。
    fn insert(
        &mut self,
        process_id: u32,
        class_name: String,
        kind: FocusKind,
        source: DetectionSource,
    ) {
        if kind == FocusKind::Undetermined {
            return;
        }
        let key = (process_id, class_name);
        // 既存エントリが高優先かつ有効期限内なら上書きしない
        if let Some(existing) = self.entries.get(&key) {
            if existing.source > source
                && existing.timestamp.elapsed().as_secs() < existing.source.ttl_secs()
            {
                return;
            }
        }
        self.entries.insert(
            key,
            FocusCacheEntry {
                kind,
                source,
                timestamp: Instant::now(),
            },
        );
        // エントリ数が上限を超えたら期限切れのみ削除
        if self.entries.len() > 1000 {
            self.entries
                .retain(|_, v| v.timestamp.elapsed().as_secs() < v.source.ttl_secs());
        }
    }
}

/// `HWND` を `Send` 可能にするラッパー
///
/// `HWND` は `*mut c_void` を含むため `Send` を実装していないが、
/// ウィンドウハンドルの値自体はスレッド間で安全に受け渡せる。
/// UIA ワーカースレッドへの HWND 送信専用。
#[derive(Clone, Copy)]
struct SendableHwnd(HWND);
// Safety: HWND の値（ポインタ値）はスレッド間で安全に共有できる。
// ウィンドウハンドルはプロセス内でグローバルに有効であり、
// 別スレッドから参照しても問題ない。
unsafe impl Send for SendableHwnd {}

/// Ctrl+C 受信フラグ
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// フォーカス中コントロールの種別キャッシュ（Undetermined=2 で初期化）
static FOCUS_KIND: AtomicU8 = AtomicU8::new(FocusKind::Undetermined as u8);

/// `WINEVENT_OUTOFCONTEXT` (0x0000) — コールバックをメッセージループで実行
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;

/// `EVENT_OBJECT_FOCUS` (0x8005) — フォーカス変更イベント
const EVENT_OBJECT_FOCUS: u32 = 0x8005;

/// `WS_EX_NOIME` (0x0040_0000) — IME 入力を受け付けないウィンドウスタイル
const WS_EX_NOIME: i32 = 0x0040_0000;

/// `ES_READONLY` (0x0800) — 読み取り専用 Edit コントロール
const ES_READONLY: i32 = 0x0800;

fn main() -> Result<()> {
    init_logging();
    let config = load_config()?;
    let (layout_names, initial_layout_name) = init_engine(&config)?;
    init_ime();
    init_ngram(&config);

    unsafe {
        OUTPUT.set(Output::new());
        IME_TRANSITION_GUARD.set(false);
        DEFERRED_KEYS.set(Vec::new());
        FOCUS_CACHE.set(FocusCache::new());
        LAST_FOCUS_INFO.set((0, String::new()));
        KEY_PATTERN_TRACKER.set(KeyPatternTracker::new());
        UNDETERMINED_BUFFERING.set(false);
        PASSTHROUGH_MEMORY.set(Vec::new());
        FOCUS_OVERRIDES.set(config.focus_overrides.clone());
    }

    init_tray(&layout_names, &initial_layout_name)?;
    install_hooks_and_hotkeys(&config)?;

    log::info!("Hook installed. Running message loop...");
    log::info!("Press Ctrl+C to exit.");
    install_ctrl_handler();
    install_focus_hook();

    // Phase 3: UIA 非同期判定ワーカースレッドを起動
    let uia_tx = spawn_uia_worker();
    unsafe {
        UIA_SENDER.set(uia_tx);
    }

    run_message_loop();
    cleanup();

    Ok(())
}

/// ログ初期化
fn init_logging() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();
    log::info!("Keyboard Layout Emulator starting...");
}

/// 設定ファイルを読み込む
fn load_config() -> Result<AppConfig> {
    let config_path = find_config_path()?;
    log::info!("Loading config from: {}", config_path.display());
    let config = AppConfig::load(&config_path)?;
    log::info!(
        "Default layout: {}, Threshold: {}ms",
        config.general.default_layout,
        config.general.simultaneous_threshold_ms
    );
    Ok(config)
}

/// 配列の読み込みとエンジン初期化を行い、レイアウト名一覧とデフォルト名を返す
fn init_engine(config: &AppConfig) -> Result<(Vec<String>, String)> {
    let left_thumb_vk = vk_name_to_code(&config.general.left_thumb_key).context(format!(
        "Unknown VK name: {}",
        config.general.left_thumb_key
    ))?;
    let right_thumb_vk = vk_name_to_code(&config.general.right_thumb_key).context(format!(
        "Unknown VK name: {}",
        config.general.right_thumb_key
    ))?;

    let layouts_dir = resolve_relative(&config.general.layouts_dir);
    let layouts = scan_layouts(&layouts_dir, left_thumb_vk, right_thumb_vk)?;
    let layout_names: Vec<String> = layouts.iter().map(|(name, ..)| name.clone()).collect();
    log::info!("Available layouts: {layout_names:?}");

    let (layout, initial_layout_name) = select_default_layout(&layouts, config);
    log::info!(
        "Layout loaded: {} normal keys, {} left thumb keys, {} right thumb keys",
        layout.normal.len(),
        layout.left_thumb.len(),
        layout.right_thumb.len()
    );

    unsafe {
        ENGINE.set(Engine::new(
            layout,
            left_thumb_vk,
            right_thumb_vk,
            config.general.simultaneous_threshold_ms,
            config.general.confirm_mode,
            config.general.speculative_delay_ms,
        ));
        LAYOUTS.set(layouts);
    }

    Ok((layout_names, initial_layout_name))
}

/// デフォルトレイアウトを選択し、YabLayout とレイアウト名を返す
fn select_default_layout(
    layouts: &[(String, YabLayout, u16, u16)],
    config: &AppConfig,
) -> (YabLayout, String) {
    let default_name = config.general.default_layout.trim_end_matches(".yab");
    let index = layouts
        .iter()
        .position(|(name, ..)| name == default_name)
        .unwrap_or(0);
    let (ref name, ref layout, _, _) = layouts[index];
    let copied = YabLayout {
        name: layout.name.clone(),
        normal: layout.normal.clone(),
        left_thumb: layout.left_thumb.clone(),
        right_thumb: layout.right_thumb.clone(),
        shift: layout.shift.clone(),
    };
    (copied, name.clone())
}

/// IME プロバイダ初期化（TSF 優先、IMM32 フォールバック）
fn init_ime() {
    let ime_provider = HybridProvider::new();
    log::info!(
        "IME provider initialized. Japanese keyboard: {}",
        ime::is_japanese_input_language()
    );
    unsafe {
        IME.set(ime_provider);
    }
}

/// n-gram モデルのロード（オプション）
fn init_ngram(config: &AppConfig) {
    let Some(ref ngram_path) = config.general.ngram_file else {
        return;
    };
    let ngram_path = resolve_relative(ngram_path);
    let content = match std::fs::read_to_string(&ngram_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to read n-gram file {}: {e}", ngram_path.display());
            return;
        }
    };
    let base_us = u64::from(config.general.simultaneous_threshold_ms) * 1000;
    let range_us = u64::from(config.general.ngram_adjustment_range_ms) * 1000;
    let min_us = u64::from(config.general.ngram_min_threshold_ms) * 1000;
    let max_us = u64::from(config.general.ngram_max_threshold_ms) * 1000;
    match NgramModel::from_toml(&content, base_us, range_us, min_us, max_us) {
        Ok(model) => {
            log::info!("N-gram model loaded from {}", ngram_path.display());
            unsafe {
                if let Some(engine) = ENGINE.get_mut() {
                    engine.set_ngram_model(model);
                }
            }
        }
        Err(e) => log::warn!("Failed to parse n-gram model: {e}"),
    }
}

/// システムトレイアイコンを作成する
fn init_tray(layout_names: &[String], initial_layout_name: &str) -> Result<()> {
    let mut system_tray = SystemTray::new(true).context("Failed to create system tray icon")?;
    system_tray.set_layout_names(layout_names.to_vec());
    system_tray.set_layout_name(initial_layout_name);
    unsafe {
        TRAY.set(system_tray);
    }
    Ok(())
}

/// フック登録とホットキー登録を行う
fn install_hooks_and_hotkeys(config: &AppConfig) -> Result<()> {
    let callback = Box::new(|event: RawKeyEvent| -> CallbackResult {
        unsafe { on_key_event_callback(event) }
    });
    hook::install_hook(callback).context("Failed to install keyboard hook")?;

    if let Some(ref hotkey_str) = config.general.toggle_hotkey {
        register_toggle_hotkey(hotkey_str);
    }
    register_focus_override_hotkey();
    Ok(())
}

/// トグルホットキーを登録する
fn register_toggle_hotkey(hotkey_str: &str) {
    if let Some((modifiers, vk)) = awase::config::parse_hotkey(hotkey_str) {
        unsafe {
            let result = RegisterHotKey(
                HWND::default(),
                HOTKEY_ID_TOGGLE,
                HOT_KEY_MODIFIERS(modifiers),
                u32::from(vk),
            );
            if result.is_ok() {
                log::info!("Toggle hotkey registered: {hotkey_str}");
            } else {
                log::warn!("Failed to register toggle hotkey: {hotkey_str}");
            }
        }
    } else {
        log::warn!("Invalid toggle hotkey format: {hotkey_str}");
    }
}

/// 手動フォーカスオーバーライドホットキー (Ctrl+Shift+F11) を登録する
fn register_focus_override_hotkey() {
    const MOD_CONTROL: u32 = 0x0002;
    const MOD_SHIFT: u32 = 0x0004;
    const VK_F11: u32 = 0x7A;
    unsafe {
        let result = RegisterHotKey(
            HWND::default(),
            HOTKEY_ID_FOCUS_OVERRIDE,
            HOT_KEY_MODIFIERS(MOD_CONTROL | MOD_SHIFT),
            VK_F11,
        );
        if result.is_ok() {
            log::info!("Focus override hotkey registered: Ctrl+Shift+F11");
        } else {
            log::warn!("Failed to register focus override hotkey: Ctrl+Shift+F11");
        }
    }
}

/// 手動フォーカスオーバーライドのトグル処理
///
/// 現在の `FocusKind` を反転し、学習キャッシュに `UserOverride` で記録する。
/// `NonText` への降格時はエンジンコンテキストを無効化し、バッファもクリアする。
///
/// Safety: シングルスレッドからのみ呼び出すこと
unsafe fn toggle_focus_override() {
    let current = FOCUS_KIND.load(Ordering::Acquire);
    let new_kind = if current == FocusKind::TextInput as u8 {
        FocusKind::NonText
    } else {
        FocusKind::TextInput
    };

    FOCUS_KIND.store(new_kind as u8, Ordering::Release);

    // Update learning cache
    if let Some((pid, cls)) = LAST_FOCUS_INFO.get_ref() {
        if let Some(cache) = FOCUS_CACHE.get_mut() {
            cache.insert(*pid, cls.clone(), new_kind, DetectionSource::UserOverride);
        }
    }

    // If demoted to NonText, flush engine pending
    if new_kind == FocusKind::NonText {
        invalidate_engine_context(ContextChange::FocusChanged);
    }

    // Clear any active buffers
    if let Some(buf) = DEFERRED_KEYS.get_mut() {
        buf.clear();
    }
    if let Some(mem) = PASSTHROUGH_MEMORY.get_mut() {
        mem.clear();
    }
    if let Some(buffering) = UNDETERMINED_BUFFERING.get_mut() {
        *buffering = false;
    }

    let mode_str = if new_kind == FocusKind::TextInput {
        "TextInput (engine enabled)"
    } else {
        "NonText (engine bypassed)"
    };
    log::info!("Manual focus override: → {}", mode_str);
}

/// クリーンアップ処理
fn cleanup() {
    hook::uninstall_hook();
    unsafe {
        let _ = UnregisterHotKey(HWND::default(), HOTKEY_ID_TOGGLE);
        let _ = UnregisterHotKey(HWND::default(), HOTKEY_ID_FOCUS_OVERRIDE);
    }
    unsafe {
        if let Some(tray) = TRAY.get_mut() {
            tray.destroy();
        }
        TRAY.clear();
    }
    unsafe {
        ENGINE.clear();
        OUTPUT.clear();
        IME.clear();
        LAYOUTS.clear();
        FOCUS_CACHE.clear();
        LAST_FOCUS_INFO.clear();
        KEY_PATTERN_TRACKER.clear();
        UNDETERMINED_BUFFERING.clear();
        PASSTHROUGH_MEMORY.clear();
    }
    log::info!("Exited cleanly.");
}

/// Win32 タイマーランタイム
struct Win32TimerRuntime;

impl TimerRuntime for Win32TimerRuntime {
    type TimerId = usize;

    fn set_timer(&mut self, id: Self::TimerId, duration: std::time::Duration) {
        let ms = u32::try_from(duration.as_millis()).unwrap_or(u32::MAX);
        unsafe {
            let _ = SetTimer(HWND::default(), id, ms, None);
        }
    }

    fn kill_timer(&mut self, id: Self::TimerId) {
        unsafe {
            let _ = KillTimer(HWND::default(), id);
        }
    }
}

/// `SendInput` アクション実行器
struct SendInputExecutor;

impl ActionExecutor for SendInputExecutor {
    type Action = awase::types::KeyAction;

    fn execute(&mut self, actions: &[Self::Action]) {
        unsafe {
            if let Some(output) = OUTPUT.get_ref() {
                output.send_keys(actions);
            }
        }
    }
}

/// IME コンテキストキー（IME 制御キー + 親指キー）かどうかを判定する。
///
/// `Engine::is_ime_control_vk()` のスーパーセットに親指キー（変換/無変換）を追加。
/// これらのキーが押された場合、ユーザーがテキスト入力コンテキストにいる強いシグナルとなる。
fn is_ime_context_key(vk_code: u16) -> bool {
    matches!(
        vk_code,
        0x15 | 0x16 | 0x17 | 0x19 | 0x1C | 0x1D | 0xE5
    )
}

/// 修飾キー（Ctrl/Alt）が押されていない単独文字キーかどうかを判定する。
///
/// パターン検出およびハイブリッドバッファリングで使用。
fn is_modifier_free_char_key(vk_code: u16) -> bool {
    !Engine::is_ime_control_vk(vk_code)
        && !Engine::is_passthrough_vk(vk_code)
        && vk_code != 0x1C  // VK_CONVERT (右親指)
        && vk_code != 0x1D  // VK_NONCONVERT (左親指)
        && vk_code != 0x08  // VK_BACK（BS は別途追跡）
        && !is_os_modifier_held()
}

/// OS レベルで Ctrl/Alt が押されているかを判定する。
///
/// `GetAsyncKeyState` を使用してリアルタイムの修飾キー状態を取得する。
fn is_os_modifier_held() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    unsafe {
        let ctrl = GetAsyncKeyState(0x11);  // VK_CONTROL
        let alt = GetAsyncKeyState(0x12);   // VK_MENU
        (ctrl & (1 << 15) as i16) != 0 || (alt & (1 << 15) as i16) != 0
    }
}

/// タイピングパターン検出用トラッカー
///
/// 直近のキー入力パターンからテキスト入力コンテキストを推定する。
/// - 1 秒以内に修飾なし文字キーが 5 回 → TextInput に昇格
/// - 文字キー後 2 秒以内に BS が 2 回 → TextInput に昇格（テキスト編集パターン）
struct KeyPatternTracker {
    /// 直近の修飾なし文字キーのタイムスタンプ
    char_timestamps: Vec<Instant>,
    /// 直近の BS キーのタイムスタンプ（文字キー後のみ追跡）
    bs_timestamps: Vec<Instant>,
    /// 最近文字キーが押されたか（BS 追跡用）
    had_recent_chars: bool,
}

impl KeyPatternTracker {
    fn new() -> Self {
        Self {
            char_timestamps: Vec::new(),
            bs_timestamps: Vec::new(),
            had_recent_chars: false,
        }
    }

    /// キーイベントを追跡し、パターンが検出された場合は理由文字列を返す。
    fn on_key(&mut self, vk_code: u16, is_modifier_free_char: bool) -> Option<&'static str> {
        let now = Instant::now();

        if is_modifier_free_char {
            self.char_timestamps.push(now);
            self.char_timestamps
                .retain(|t| now.duration_since(*t).as_millis() < 1000);
            self.had_recent_chars = true;

            if self.char_timestamps.len() >= 5 {
                self.char_timestamps.clear();
                self.bs_timestamps.clear();
                return Some("5 char keys in 1s");
            }
        }

        if vk_code == 0x08 && self.had_recent_chars {
            // VK_BACK
            self.bs_timestamps.push(now);
            self.bs_timestamps
                .retain(|t| now.duration_since(*t).as_millis() < 2000);

            if self.bs_timestamps.len() >= 2 {
                self.char_timestamps.clear();
                self.bs_timestamps.clear();
                return Some("2 BS after chars in 2s");
            }
        }

        // 文字キーでも BS でもないキー → 2 秒経過で recent chars リセット
        if !is_modifier_free_char && vk_code != 0x08 {
            if let Some(last) = self.char_timestamps.last() {
                if now.duration_since(*last).as_millis() > 2000 {
                    self.had_recent_chars = false;
                    self.bs_timestamps.clear();
                }
            }
        }

        None
    }

    /// トラッカーをリセットする（昇格後やフォーカス変更時）
    fn clear(&mut self) {
        self.char_timestamps.clear();
        self.bs_timestamps.clear();
        self.had_recent_chars = false;
    }
}

/// FocusKind を TextInput に昇格させる共通ヘルパー。
///
/// キャッシュとログの更新を一元化する。
unsafe fn promote_to_text_input(source: DetectionSource, reason: &str) {
    let current = FOCUS_KIND.load(Ordering::Acquire);
    if current == FocusKind::TextInput as u8 {
        return;
    }
    FOCUS_KIND.store(FocusKind::TextInput as u8, Ordering::Release);
    if let Some(cache) = FOCUS_CACHE.get_mut() {
        if let Some((pid, cls)) = LAST_FOCUS_INFO.get_ref() {
            cache.insert(*pid, cls.clone(), FocusKind::TextInput, source);
        }
    }
    log::info!(
        "Promoting to TextInput: {} (source={:?})",
        reason,
        source
    );
}

/// キー入力パターンを観察し、テキスト入力コンテキストを推定する。
///
/// すべてのキーイベントに対して、FOCUS_KIND バイパスチェックの **前** に呼び出す。
/// パターンが検出されると `promote_to_text_input` で昇格する。
unsafe fn observe_key_pattern(event: &RawKeyEvent) {
    let is_key_down = matches!(
        event.event_type,
        KeyEventType::KeyDown | KeyEventType::SysKeyDown
    );
    if !is_key_down {
        return;
    }

    let current = FOCUS_KIND.load(Ordering::Acquire);
    if current == FocusKind::TextInput as u8 {
        return; // 既に TextInput なら追跡不要
    }

    let is_char = is_modifier_free_char_key(event.vk_code);

    if let Some(tracker) = KEY_PATTERN_TRACKER.get_mut() {
        if let Some(reason) = tracker.on_key(event.vk_code, is_char) {
            promote_to_text_input(DetectionSource::TypingPatternInferred, reason);
            tracker.clear();

            // IME OFF + Undetermined で PassThrough 済みキーがある場合、
            // BS で取り消して再処理する
            retract_passthrough_memory();
        }
    }
}

/// PassThrough 済みキーを BS で取り消し、エンジンで再処理する。
///
/// IME OFF + Undetermined 状態で PassThrough したキーを、
/// TextInput に昇格した後に正しく処理し直すために使用する。
unsafe fn retract_passthrough_memory() {
    let keys = PASSTHROUGH_MEMORY
        .get_mut()
        .map(std::mem::take)
        .unwrap_or_default();

    if keys.is_empty() {
        return;
    }

    log::debug!(
        "Retracting {} passthrough key(s) with BS + re-process",
        keys.len()
    );

    // BS を送信して PassThrough 済みの文字を取り消す
    if let Some(output) = OUTPUT.get_ref() {
        let mut bs_actions: Vec<awase::types::KeyAction> = Vec::new();
        for _ in 0..keys.len() {
            bs_actions.push(awase::types::KeyAction::Key(0x08));   // VK_BACK down
            bs_actions.push(awase::types::KeyAction::KeyUp(0x08)); // VK_BACK up
        }
        output.send_keys(&bs_actions);
    }

    // エンジンで再処理
    for event in keys {
        let ime_active = IME
            .get_ref()
            .map_or(false, |ime| ime.is_active() && ime.get_mode().is_kana_input());

        if ime_active {
            if let Some(engine) = ENGINE.get_mut() {
                let response = engine.on_event(event);
                let mut timer_runtime = Win32TimerRuntime;
                let mut action_executor = SendInputExecutor;
                dispatch(&response, &mut timer_runtime, &mut action_executor);
            }
        }
        // IME OFF のままなら再注入（元々 PassThrough だったので同じ結果）
        // この場合は BS 分が余計だが、IME OFF → パターン検出 → 昇格の流れでは
        // IME が ON になっていることが前提なので通常は engine 経由になる
    }
}

/// Undetermined + IME ON バッファリングのタイムアウトを開始する（初回バッファ時のみ）。
unsafe fn start_buffer_timeout_if_needed() {
    if let Some(buffering) = UNDETERMINED_BUFFERING.get_mut() {
        if !*buffering {
            *buffering = true;
            let _ = SetTimer(HWND::default(), TIMER_UNDETERMINED_BUFFER, 300, None);
        }
    }
}

/// Undetermined + IME ON バッファリングのタイムアウト処理。
///
/// 300ms 以内にパターン検出されなかった場合、バッファされたキーを
/// エンジンで処理する（安全側: TextInput として扱う）。
unsafe fn handle_buffer_timeout() {
    let _ = KillTimer(HWND::default(), TIMER_UNDETERMINED_BUFFER);
    if let Some(buffering) = UNDETERMINED_BUFFERING.get_mut() {
        *buffering = false;
    }

    let keys = DEFERRED_KEYS
        .get_mut()
        .map(std::mem::take)
        .unwrap_or_default();

    if keys.is_empty() {
        return;
    }

    log::debug!(
        "Buffer timeout: promoting to TextInput and processing {} buffered key(s)",
        keys.len()
    );

    // タイムアウト → TextInput に昇格してエンジンで処理
    promote_to_text_input(
        DetectionSource::TypingPatternInferred,
        "buffer timeout (IME ON + Undetermined)",
    );

    for event in keys {
        if let Some(engine) = ENGINE.get_mut() {
            let response = engine.on_event(event);
            let mut timer_runtime = Win32TimerRuntime;
            let mut action_executor = SendInputExecutor;
            dispatch(&response, &mut timer_runtime, &mut action_executor);
        }
    }
}

/// フックコールバックからの Engine 呼び出し
unsafe fn on_key_event_callback(event: RawKeyEvent) -> CallbackResult {
    let is_key_down = matches!(
        event.event_type,
        KeyEventType::KeyDown | KeyEventType::SysKeyDown
    );

    // ── Step 0: パターン観察（すべてのキーイベントに対して、バイパスチェック前に実行） ──
    observe_key_pattern(&event);

    // ── Step 1: IME/親指キー検出による即時 TextInput 昇格 ──
    // IME 制御キーまたは親指キー（変換/無変換）が押された場合、
    // ユーザーがテキスト入力コンテキストにいると判断して昇格する。
    if is_key_down && is_ime_context_key(event.vk_code) {
        let current = FOCUS_KIND.load(Ordering::Acquire);
        if current != FocusKind::TextInput as u8 {
            promote_to_text_input(
                DetectionSource::ImeKeyInferred,
                &format!("IME/thumb key 0x{:02X}", event.vk_code),
            );
            // Undetermined バッファリング中ならバッファを処理
            if let Some(buffering) = UNDETERMINED_BUFFERING.get_mut() {
                if *buffering {
                    *buffering = false;
                    let _ = KillTimer(HWND::default(), TIMER_UNDETERMINED_BUFFER);
                    // バッファされたキーをエンジンで処理
                    let keys = DEFERRED_KEYS
                        .get_mut()
                        .map(std::mem::take)
                        .unwrap_or_default();
                    for buffered in keys {
                        if let Some(engine) = ENGINE.get_mut() {
                            let response = engine.on_event(buffered);
                            let mut timer_runtime = Win32TimerRuntime;
                            let mut action_executor = SendInputExecutor;
                            dispatch(&response, &mut timer_runtime, &mut action_executor);
                        }
                    }
                }
            }
            // PassThrough 済みキーがあれば取り消して再処理
            retract_passthrough_memory();
        }
    }

    // ── Step 2: IME ガード: IME 制御キー直後の後続キーを遅延処理する ──
    // IME 制御キー（半角/全角等）が OS に渡された直後は、IME 状態がまだ反映されて
    // いない可能性がある。後続キーをメッセージループに回すことで、IME 状態が
    // 確実に更新された後に処理する。
    if let Some(guard) = IME_TRANSITION_GUARD.get_mut() {
        if *guard {
            // IME 制御キーの KeyUp でガード解除
            if !is_key_down && Engine::is_ime_control_vk(event.vk_code) {
                *guard = false;
                return CallbackResult::PassThrough;
            }
            // KeyDown はバッファに保存してメッセージループに回す
            if is_key_down {
                if let Some(buf) = DEFERRED_KEYS.get_mut() {
                    buf.push(event);
                    let _ = PostMessageW(HWND::default(), WM_PROCESS_DEFERRED, WPARAM(0), LPARAM(0));
                }
                return CallbackResult::Consumed;
            }
            // ガード中の KeyUp（IME制御キー以外）はパススルー
            return CallbackResult::PassThrough;
        }
    }

    // ── Step 3: IME 制御キーの検出: ガードを有効にしてパススルー ──
    if is_key_down && Engine::is_ime_control_vk(event.vk_code) {
        // エンジンの保留をフラッシュ（engine 側の handle_bypass で実行される）
        if let Some(engine) = ENGINE.get_mut() {
            let response = engine.on_event(event);
            let mut timer_runtime = Win32TimerRuntime;
            let mut action_executor = SendInputExecutor;
            dispatch(&response, &mut timer_runtime, &mut action_executor);
            // engine が passthrough を返す → ガードを有効にして OS に渡す
            if !response.consumed {
                if let Some(guard) = IME_TRANSITION_GUARD.get_mut() {
                    *guard = true;
                }
            }
        }
        // consumed=false の場合は OS にそのまま渡す（CallbackResult::PassThrough）
        // consumed=true の場合はエンジンが処理済み（通常ありえないがsafety）
        return CallbackResult::PassThrough;
    }

    // ── Step 4: フォーカス判定によるハイブリッド戦略 ──
    let Some(engine) = ENGINE.get_mut() else {
        return CallbackResult::PassThrough;
    };

    let focus = FOCUS_KIND.load(Ordering::Acquire);
    match focus {
        f if f == FocusKind::NonText as u8 => {
            // 非テキストコントロール → 常にパススルー
            return CallbackResult::PassThrough;
        }
        f if f == FocusKind::TextInput as u8 => {
            // テキスト入力 → 既存のエンジン処理（下の Step 5 に進む）
        }
        _ => {
            // Undetermined → ハイブリッド戦略
            if is_key_down {
                let ime_on = IME
                    .get_ref()
                    .map_or(false, |ime| ime.is_active() && ime.get_mode().is_kana_input());

                let is_char = is_modifier_free_char_key(event.vk_code);

                if ime_on && is_char {
                    // IME ON + Undetermined + 文字キー → バッファリング
                    if let Some(buf) = DEFERRED_KEYS.get_mut() {
                        buf.push(event);
                    }
                    start_buffer_timeout_if_needed();
                    return CallbackResult::Consumed;
                } else if !ime_on && is_char {
                    // IME OFF + Undetermined + 文字キー → PassThrough + 記憶
                    if let Some(mem) = PASSTHROUGH_MEMORY.get_mut() {
                        mem.push(event);
                        // 記憶バッファの上限（無限に溜めない）
                        if mem.len() > 20 {
                            mem.remove(0);
                        }
                    }
                    return CallbackResult::PassThrough;
                }
            }
            // 文字キー以外や KeyUp → PassThrough
            return CallbackResult::PassThrough;
        }
    }

    // ── Step 5: エンジン処理（TextInput 確定時のみ到達） ──
    // IME OFF / 英数モード → PassThrough（エンジンバイパス）
    if let Some(ime_provider) = IME.get_ref() {
        if !ime_provider.is_active() || !ime_provider.get_mode().is_kana_input() {
            return CallbackResult::PassThrough;
        }
    }

    let response = engine.on_event(event);
    let mut timer_runtime = Win32TimerRuntime;
    let mut action_executor = SendInputExecutor;
    let consumed = dispatch(&response, &mut timer_runtime, &mut action_executor);

    if consumed {
        CallbackResult::Consumed
    } else {
        CallbackResult::PassThrough
    }
}

/// フォーカス変更イベントフックを登録する
///
/// `WINEVENT_OUTOFCONTEXT` を使用するため、コールバックはメッセージループ上で実行される。
/// これにより `classify_focus` が非同期（キーイベントとは別タイミング）で呼ばれる。
fn install_focus_hook() {
    unsafe {
        let hook = SetWinEventHook(
            EVENT_OBJECT_FOCUS,
            EVENT_OBJECT_FOCUS,
            None,
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        if hook.is_invalid() {
            log::warn!("Failed to install focus event hook");
        } else {
            log::info!("Focus event hook installed");
        }
    }
}

/// フォーカス変更イベントのコールバック
///
/// `WINEVENT_OUTOFCONTEXT` により、メッセージループのコンテキストで呼ばれる。
/// フォーカスが移動するたびにバイパス判定を更新し、キャッシュに書き込む。
unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event != EVENT_OBJECT_FOCUS {
        return;
    }

    // Step 0: プロセスID・クラス名を取得し、キャッシュを検索
    let process_id = get_window_process_id(hwnd);
    let class_name = get_class_name_string(hwnd);

    // UIA 非同期結果のキャッシュ更新用に保存
    if let Some(last) = LAST_FOCUS_INFO.get_mut() {
        *last = (process_id, class_name.clone());
    }

    // フォーカス変更時にパターントラッカーと記憶バッファをリセット
    if let Some(tracker) = KEY_PATTERN_TRACKER.get_mut() {
        tracker.clear();
    }
    if let Some(mem) = PASSTHROUGH_MEMORY.get_mut() {
        mem.clear();
    }
    // Undetermined バッファリング中ならキャンセル
    if let Some(buffering) = UNDETERMINED_BUFFERING.get_mut() {
        if *buffering {
            *buffering = false;
            let _ = KillTimer(HWND::default(), TIMER_UNDETERMINED_BUFFER);
            // バッファされたキーは破棄（フォーカスが変わったので無意味）
            if let Some(buf) = DEFERRED_KEYS.get_mut() {
                buf.clear();
            }
        }
    }

    // Config オーバーライド（最高優先度、キャッシュより先に判定）
    if let Some(overrides) = FOCUS_OVERRIDES.get_ref() {
        if !overrides.force_text.is_empty() || !overrides.force_bypass.is_empty() {
            let process_name = get_process_name(process_id);
            for entry in &overrides.force_text {
                if entry.process.eq_ignore_ascii_case(&process_name)
                    && entry.class.eq_ignore_ascii_case(&class_name)
                {
                    log::debug!(
                        "classify_focus: config override force_text ({}, {})",
                        process_name,
                        class_name
                    );
                    FOCUS_KIND.store(FocusKind::TextInput as u8, Ordering::Release);
                    return;
                }
            }
            for entry in &overrides.force_bypass {
                if entry.process.eq_ignore_ascii_case(&process_name)
                    && entry.class.eq_ignore_ascii_case(&class_name)
                {
                    log::debug!(
                        "classify_focus: config override force_bypass ({}, {})",
                        process_name,
                        class_name
                    );
                    FOCUS_KIND.store(FocusKind::NonText as u8, Ordering::Release);
                    invalidate_engine_context(ContextChange::FocusChanged);
                    return;
                }
            }
        }
    }

    // キャッシュヒット → 即座に結果を適用
    if let Some(cached) = FOCUS_CACHE
        .get_ref()
        .and_then(|c| c.get(process_id, &class_name))
    {
        log::trace!(
            "classify_focus: cache hit ({}, {}) → {:?}",
            process_id,
            class_name,
            cached
        );
        FOCUS_KIND.store(cached as u8, Ordering::Release);
        if cached == FocusKind::NonText {
            invalidate_engine_context(ContextChange::FocusChanged);
        }
        return;
    }

    // Step 1: 評価中は安全側（Undetermined）に設定
    FOCUS_KIND.store(FocusKind::Undetermined as u8, Ordering::Release);

    // Step 2: バイパス状態を判定
    let state = classify_focus(hwnd);

    // Step 3: キャッシュに格納し、FOCUS_KIND を更新
    if let Some(cache) = FOCUS_CACHE.get_mut() {
        cache.insert(process_id, class_name.clone(), state, DetectionSource::Automatic);
    }
    FOCUS_KIND.store(state as u8, Ordering::Release);

    // Step 4: NonText ならエンジンの保留状態をフラッシュ
    if state == FocusKind::NonText {
        invalidate_engine_context(ContextChange::FocusChanged);
    }

    // Step 5: Phase 1-2 で判定不能なら UIA 非同期判定をリクエスト
    if state == FocusKind::Undetermined {
        if let Some(tx) = UIA_SENDER.get_ref() {
            let _ = tx.send(SendableHwnd(hwnd));
        }

        // Step 6: Undetermined + 非ブラウザ系 → IME OFF にして安全側に倒す
        // ブラウザ/Electron 系は UIA Phase 3 で正確に判定できるため、IME を維持する。
        // ゲーム/gvim 等の非ブラウザ系は UIA でも判定不能なため、IME OFF で保護する。
        // UIA が後から TextInput を返した場合は IME ON に復帰する（WM_FOCUS_KIND_UPDATE）。
        if !is_browser_or_electron_class(&class_name) {
            set_ime_off(hwnd);
            invalidate_engine_context(ContextChange::FocusChanged);
        }
    }

    log::debug!(
        "Focus changed: hwnd={:?} class={} → {:?}{}",
        hwnd,
        class_name,
        state,
        if state == FocusKind::Undetermined && !is_browser_or_electron_class(&class_name) {
            " (IME auto-OFF)"
        } else {
            ""
        }
    );
}

/// ウィンドウハンドルからプロセス ID を取得する
unsafe fn get_window_process_id(hwnd: HWND) -> u32 {
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    pid
}

/// プロセス ID から実行ファイル名を取得する
unsafe fn get_process_name(process_id: u32) -> String {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) else {
        return String::new();
    };
    let mut buf = [0u16; 260];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
    let _ = CloseHandle(handle);
    if ok.is_ok() && len > 0 {
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().unwrap_or(&path).to_string()
    } else {
        String::new()
    }
}

/// ウィンドウハンドルからクラス名を取得する
unsafe fn get_class_name_string(hwnd: HWND) -> String {
    let mut class_buf = [0u16; 256];
    let len = GetClassNameW(hwnd, &mut class_buf);
    if len > 0 {
        String::from_utf16_lossy(&class_buf[..len as usize])
    } else {
        String::new()
    }
}

/// ブラウザ/Electron 系のクラス名かどうかを判定する。
///
/// これらのアプリは UIA Phase 3 でテキスト入力を正確に判定できるため、
/// Undetermined 時の自動 IME OFF を適用しない。
fn is_browser_or_electron_class(class_name: &str) -> bool {
    // Chromium 系（Chrome, Edge, Brave, Opera, Vivaldi, 全 Electron アプリ）
    // Firefox 系（Firefox, Waterfox, Tor Browser）
    class_name == "Chrome_WidgetWin_1" || class_name == "MozillaWindowClass"
}

/// 指定ウィンドウの IME を OFF にする。
unsafe fn set_ime_off(hwnd: HWND) {
    let himc = ImmGetContext(hwnd);
    if !himc.is_invalid() {
        let _ = ImmSetOpenStatus(himc, false);
        ImmReleaseContext(hwnd, himc);
        log::debug!("IME auto-OFF for hwnd={:?}", hwnd);
    }
}

/// 指定ウィンドウの IME を ON にする。
unsafe fn set_ime_on(hwnd: HWND) {
    let himc = ImmGetContext(hwnd);
    if !himc.is_invalid() {
        let _ = ImmSetOpenStatus(himc, true);
        ImmReleaseContext(hwnd, himc);
        log::debug!("IME auto-ON for hwnd={:?}", hwnd);
    }
}

/// フォーカス中のウィンドウがテキスト入力を受け付けるかを判定する
///
/// deny-first（バイパスを優先）、allow は確信がある場合のみ。
/// 判定不能なら `Undetermined` を返す。
unsafe fn classify_focus(hwnd: HWND) -> FocusKind {
    if hwnd == HWND::default() {
        return FocusKind::NonText;
    }

    // 1. ImmGetContext == NULL → IME 入力不可
    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() {
        return FocusKind::NonText;
    }
    let _ = ImmReleaseContext(hwnd, himc);

    // 2. WS_EX_NOIME ウィンドウスタイル
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
    if ex_style & WS_EX_NOIME != 0 {
        return FocusKind::NonText;
    }

    // 3. クラス名による判定
    let class_name = get_class_name_string(hwnd);
    if !class_name.is_empty() {

        // 既知のテキスト入力コントロール
        if matches!(
            class_name.as_str(),
            "Edit"
                | "RichEdit"
                | "RichEdit20A"
                | "RichEdit20W"
                | "RICHEDIT50W"
                | "Scintilla"
                | "ConsoleWindowClass"
        ) {
            // Edit コントロールの読み取り専用チェック
            if class_name == "Edit" {
                let style = GetWindowLongW(hwnd, GWL_STYLE);
                if style & ES_READONLY != 0 {
                    return FocusKind::NonText;
                }
            }
            return FocusKind::TextInput;
        }

        // 既知の非テキストコントロール
        if matches!(
            class_name.as_str(),
            "Button"
                | "Static"
                | "SysListView32"
                | "SysTreeView32"
                | "SysHeader32"
                | "ToolbarWindow32"
                | "msctls_statusbar32"
                | "SysTabControl32"
                | "msctls_trackbar32"
                | "msctls_progress32"
        ) {
            return FocusKind::NonText;
        }
    }

    // 4. MSAA (IAccessible) role による判定
    {
        /// `OBJID_CLIENT` — クライアント領域のアクセシブルオブジェクト
        const OBJID_CLIENT: i32 = -4;

        let mut acc: *mut std::ffi::c_void = std::ptr::null_mut();
        let ok = AccessibleObjectFromWindow(
            hwnd,
            OBJID_CLIENT as u32,
            &IAccessible::IID,
            &mut acc,
        );
        if ok.is_ok() && !acc.is_null() {
            let accessible: IAccessible = IAccessible::from_raw(acc);
            let child_self = VARIANT::from(0i32); // CHILDID_SELF
            if let Ok(role) = accessible.get_accRole(&child_self) {
                let role_id = role.as_raw().Anonymous.Anonymous.Anonymous.lVal as u32;

                // テキスト入力ロール
                const ROLE_SYSTEM_TEXT: u32 = 42; // 0x2A — editable text
                const ROLE_SYSTEM_DOCUMENT: u32 = 15; // 0x0F — document window

                if matches!(role_id, ROLE_SYSTEM_TEXT | ROLE_SYSTEM_DOCUMENT) {
                    log::debug!("MSAA: role={} → TextInput", role_id);
                    return FocusKind::TextInput;
                }

                // 非テキストロール
                const ROLE_SYSTEM_TITLEBAR: u32 = 1;
                const ROLE_SYSTEM_MENUBAR: u32 = 2;
                const ROLE_SYSTEM_SCROLLBAR: u32 = 3;
                const ROLE_SYSTEM_MENUPOPUP: u32 = 11;
                const ROLE_SYSTEM_MENUITEM: u32 = 12;
                const ROLE_SYSTEM_TOOLBAR: u32 = 22;
                const ROLE_SYSTEM_STATUSBAR: u32 = 23;
                const ROLE_SYSTEM_LIST: u32 = 33;
                const ROLE_SYSTEM_LISTITEM: u32 = 34;
                const ROLE_SYSTEM_OUTLINE: u32 = 35; // tree view
                const ROLE_SYSTEM_OUTLINEITEM: u32 = 36;
                const ROLE_SYSTEM_PAGETAB: u32 = 37;
                const ROLE_SYSTEM_INDICATOR: u32 = 39;
                const ROLE_SYSTEM_GRAPHIC: u32 = 40;
                const ROLE_SYSTEM_STATICTEXT: u32 = 41;
                const ROLE_SYSTEM_PUSHBUTTON: u32 = 43;
                const ROLE_SYSTEM_PROGRESSBAR: u32 = 48;
                const ROLE_SYSTEM_SLIDER: u32 = 51;

                if matches!(
                    role_id,
                    ROLE_SYSTEM_TITLEBAR
                        | ROLE_SYSTEM_MENUBAR
                        | ROLE_SYSTEM_SCROLLBAR
                        | ROLE_SYSTEM_MENUPOPUP
                        | ROLE_SYSTEM_MENUITEM
                        | ROLE_SYSTEM_TOOLBAR
                        | ROLE_SYSTEM_STATUSBAR
                        | ROLE_SYSTEM_LIST
                        | ROLE_SYSTEM_LISTITEM
                        | ROLE_SYSTEM_OUTLINE
                        | ROLE_SYSTEM_OUTLINEITEM
                        | ROLE_SYSTEM_PAGETAB
                        | ROLE_SYSTEM_INDICATOR
                        | ROLE_SYSTEM_GRAPHIC
                        | ROLE_SYSTEM_STATICTEXT
                        | ROLE_SYSTEM_PUSHBUTTON
                        | ROLE_SYSTEM_PROGRESSBAR
                        | ROLE_SYSTEM_SLIDER
                ) {
                    log::debug!("MSAA: role={} → NonText", role_id);
                    return FocusKind::NonText;
                }

                log::debug!(
                    "MSAA: role={} → Undetermined (not in allow/deny list)",
                    role_id
                );
            }
        }
    }

    // 5. 判定不能 → Undetermined
    FocusKind::Undetermined
}

/// UIA 非同期判定ワーカースレッドを起動する
///
/// 専用スレッドで COM を初期化し、`IUIAutomation` インスタンスを保持する。
/// チャネル経由で HWND を受け取り、`GetFocusedElement` でコントロール種別を判定して
/// `FOCUS_KIND` を更新する。Phase 1-2 で `Undetermined` だったコントロールの解像度を上げる。
fn spawn_uia_worker() -> mpsc::Sender<SendableHwnd> {
    let (tx, rx) = mpsc::channel::<SendableHwnd>();
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }

        let automation: Option<IUIAutomation> = unsafe {
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
        };

        let Some(automation) = automation else {
            log::warn!("UIA: Failed to create IUIAutomation, Phase 3 disabled");
            return;
        };

        log::info!("UIA worker thread started");

        while let Ok(SendableHwnd(hwnd)) = rx.recv() {
            let state = unsafe { uia_classify_focus(&automation, hwnd) };
            if state != FocusKind::Undetermined {
                FOCUS_KIND.store(state as u8, Ordering::Release);
                log::debug!("UIA async: hwnd={:?} → {:?}", hwnd, state);

                // NonText の場合はメインスレッドにエンジンフラッシュを依頼
                if state == FocusKind::NonText {
                    unsafe {
                        let _ = PostMessageW(
                            HWND::default(),
                            WM_FOCUS_KIND_UPDATE,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
        }
    });
    tx
}

/// UIA を使用してフォーカス中コントロールの種別を判定する
///
/// Pattern-first アプローチ:
/// 1. `ValuePattern` → `IsReadOnly` で編集可能なテキストフィールドを検出
/// 2. `TextPattern` の有無でテキスト編集能力を検出
/// 3. `CurrentControlType` をフォールバックとして使用
///
/// Chrome/WPF/UWP など Win32 クラス名では判定できないコントロールに有効。
///
/// Safety: COM が初期化済みのスレッドから呼び出すこと
#[allow(unused_variables)] // hwnd はデバッグ用に保持
unsafe fn uia_classify_focus(automation: &IUIAutomation, hwnd: HWND) -> FocusKind {
    let element: IUIAutomationElement = match automation.GetFocusedElement() {
        Ok(el) => el,
        Err(e) => {
            log::trace!("UIA: GetFocusedElement failed: {:?}", e);
            return FocusKind::Undetermined;
        }
    };

    // 1. ValuePattern → IsReadOnly チェック
    //    「編集可能な値を持つ」が最も強いシグナル
    if let Ok(pattern) = element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
        match pattern.CurrentIsReadOnly() {
            Ok(read_only) if !read_only.as_bool() => {
                log::debug!("UIA: ValuePattern(IsReadOnly=false) → TextInput");
                return FocusKind::TextInput;
            }
            Ok(_) => {
                log::debug!("UIA: ValuePattern(IsReadOnly=true) → NonText");
                return FocusKind::NonText;
            }
            Err(_) => {} // fall through
        }
    }

    // 2. TextPattern チェック
    //    TextPattern をサポートする要素はテキスト編集能力を持つ
    if element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).is_ok() {
        log::debug!("UIA: TextPattern available → TextInput");
        return FocusKind::TextInput;
    }

    // 3. フォールバック: ControlType で確定的な非テキストコントロールを判別
    if let Ok(control_type) = element.CurrentControlType() {
        // テキスト入力系（補助的な確認のみ）
        if matches!(control_type, UIA_EditControlTypeId | UIA_DocumentControlTypeId) {
            log::debug!("UIA: ControlType={:?} → TextInput", control_type);
            return FocusKind::TextInput;
        }

        // 非テキスト系
        if matches!(
            control_type,
            UIA_ButtonControlTypeId
                | UIA_MenuItemControlTypeId
                | UIA_TreeItemControlTypeId
                | UIA_ListItemControlTypeId
                | UIA_TabControlTypeId
                | UIA_TabItemControlTypeId
                | UIA_ToolBarControlTypeId
                | UIA_StatusBarControlTypeId
                | UIA_ProgressBarControlTypeId
                | UIA_SliderControlTypeId
                | UIA_ScrollBarControlTypeId
                | UIA_HyperlinkControlTypeId
                | UIA_ImageControlTypeId
                | UIA_MenuBarControlTypeId
                | UIA_MenuControlTypeId
                | UIA_TitleBarControlTypeId
                | UIA_SeparatorControlTypeId
                | UIA_TextControlTypeId
        ) {
            log::debug!("UIA: ControlType={:?} → NonText", control_type);
            return FocusKind::NonText;
        }
    }

    // 4. 確定的なシグナルなし
    log::debug!("UIA: no definitive signal → Undetermined");
    FocusKind::Undetermined
}

/// メッセージループ
fn run_message_loop() {
    let mut msg = MSG::default();

    loop {
        let ret = unsafe { GetMessageW(&raw mut msg, HWND::default(), 0, 0) };
        if ret.0 <= 0 {
            break; // WM_QUIT or エラー
        }

        match msg.message {
            WM_TIMER if msg.wParam.0 == TIMER_UNDETERMINED_BUFFER => {
                unsafe {
                    handle_buffer_timeout();
                }
            }
            WM_TIMER if msg.wParam.0 == TIMER_PENDING || msg.wParam.0 == TIMER_SPECULATIVE => {
                let timer_id = msg.wParam.0;
                unsafe {
                    // IME が非活性なら on_timeout せず flush（コンテキスト喪失）
                    let ime_active = IME
                        .get_ref()
                        .map_or(true, |ime| ime.is_active() && ime.get_mode().is_kana_input());
                    if !ime_active {
                        invalidate_engine_context(ContextChange::ImeOff);
                    } else if let Some(engine) = ENGINE.get_mut() {
                        let response = engine.on_timeout(timer_id);
                        let mut timer_runtime = Win32TimerRuntime;
                        let mut action_executor = SendInputExecutor;
                        dispatch(&response, &mut timer_runtime, &mut action_executor);
                    }
                }
            }
            WM_INPUTLANGCHANGE => unsafe {
                // 入力言語が変更された（Win+Space 等）→ 保留をフラッシュ + ガード ON
                // 言語切替直後は IME 状態が未反映の可能性があるため、
                // 後続キーをメッセージループに回して確実に更新後に処理する。
                log::info!("Input language changed, flushing pending state and enabling guard");
                invalidate_engine_context(ContextChange::InputLanguageChanged);
                if let Some(guard) = IME_TRANSITION_GUARD.get_mut() {
                    *guard = true;
                }
            },
            WM_PROCESS_DEFERRED => unsafe {
                // IME 制御キー後の遅延キーを再処理する。
                // この時点で IME 状態は確実に更新済み。
                process_deferred_keys();
            },
            WM_FOCUS_KIND_UPDATE => unsafe {
                // UIA 非同期判定完了 → キャッシュ更新 + IME 状態復帰
                let focus = FOCUS_KIND.load(Ordering::Acquire);
                let kind = match focus {
                    x if x == FocusKind::TextInput as u8 => FocusKind::TextInput,
                    x if x == FocusKind::NonText as u8 => FocusKind::NonText,
                    _ => FocusKind::Undetermined,
                };
                // UIA 結果をキャッシュに反映
                if let Some((pid, cls)) = LAST_FOCUS_INFO.get_ref() {
                    if let Some(cache) = FOCUS_CACHE.get_mut() {
                        cache.insert(*pid, cls.clone(), kind, DetectionSource::UiaAsync);
                    }
                }
                if kind == FocusKind::NonText {
                    invalidate_engine_context(ContextChange::FocusChanged);
                }
                // UIA が TextInput を返した場合、IME OFF されていたら ON に復帰
                // （非ブラウザ系で自動 IME OFF された後に UIA が TextInput を返したケース）
                if kind == FocusKind::TextInput {
                    if let Some((_, cls)) = LAST_FOCUS_INFO.get_ref() {
                        if !is_browser_or_electron_class(cls) {
                            // 非ブラウザ系で UIA が TextInput → IME ON に復帰
                            let mut info = GUITHREADINFO {
                                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                                ..Default::default()
                            };
                            if GetGUIThreadInfo(0, &mut info).is_ok()
                                && info.hwndFocus != HWND::default()
                            {
                                set_ime_on(info.hwndFocus);
                            }
                        }
                    }
                }
            },
            WM_HOTKEY if msg.wParam.0 == HOTKEY_ID_TOGGLE as usize => unsafe {
                toggle_engine();
            },
            WM_HOTKEY if msg.wParam.0 == HOTKEY_ID_FOCUS_OVERRIDE as usize => unsafe {
                toggle_focus_override();
            },
            WM_APP => unsafe {
                let layout_names: Vec<String> = LAYOUTS
                    .get_ref()
                    .map(|layouts| layouts.iter().map(|(name, ..)| name.clone()).collect())
                    .unwrap_or_default();
                tray::handle_tray_message(msg.hwnd, msg.lParam, &layout_names);
            },
            WM_RELOAD_CONFIG => unsafe {
                log::info!("Config reload requested via WM_RELOAD_CONFIG");
                reload_config();
            },
            WM_COMMAND => unsafe {
                if let Some(cmd) = tray::handle_tray_command(msg.wParam) {
                    if cmd == tray::cmd_settings() {
                        launch_settings();
                    } else if cmd == tray::cmd_toggle() {
                        toggle_engine();
                    } else if cmd == tray::cmd_exit() {
                        PostQuitMessage(0);
                    } else if cmd >= tray::cmd_layout_base() {
                        let index = usize::from(cmd - tray::cmd_layout_base());
                        switch_layout(index);
                    }
                }
            },
            _ => unsafe {
                DispatchMessageW(&raw const msg);
            },
        }
    }
}

/// IME 制御キー後に遅延されたキーを再処理する。
///
/// メッセージループから呼ばれるため、この時点で IME 制御キーは OS/IME に
/// 渡し済みで、IME 状態は最新に更新されている。
///
/// Safety: シングルスレッドからのみ呼び出すこと
unsafe fn process_deferred_keys() {
    // ガード解除
    if let Some(guard) = IME_TRANSITION_GUARD.get_mut() {
        *guard = false;
    }

    // バッファからキーを取り出す
    let keys = DEFERRED_KEYS
        .get_mut()
        .map(std::mem::take)
        .unwrap_or_default();

    if keys.is_empty() {
        return;
    }

    log::debug!("Processing {} deferred key(s) after IME control", keys.len());

    for event in keys {
        // IME 状態を再チェック（最新の状態で判定）
        let ime_active = IME
            .get_ref()
            .map_or(false, |ime| ime.is_active() && ime.get_mode().is_kana_input());

        if ime_active {
            // IME ON → エンジンで処理
            if let Some(engine) = ENGINE.get_mut() {
                let response = engine.on_event(event);
                let mut timer_runtime = Win32TimerRuntime;
                let mut action_executor = SendInputExecutor;
                dispatch(&response, &mut timer_runtime, &mut action_executor);
            }
        } else {
            // IME OFF → キーをそのまま再注入（INJECTED_MARKER 付き）
            reinject_key(&event);
        }
    }
}

/// キーイベントを SendInput で再注入する（IME OFF 時の遅延キー用）
///
/// INJECTED_MARKER 付きなのでフックに再捕捉されない。
unsafe fn reinject_key(event: &RawKeyEvent) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        KEYEVENTF_SCANCODE, VIRTUAL_KEY,
    };
    use crate::output::INJECTED_MARKER;

    let is_keyup = matches!(
        event.event_type,
        KeyEventType::KeyUp | KeyEventType::SysKeyUp
    );

    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(event.vk_code),
                wScan: event.scan_code as u16,
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
    SendInput(&[input], size_of::<INPUT>() as i32);
}

/// エンジンの有効/無効を切り替え、トレイアイコンを更新する
///
/// Safety: シングルスレッドからのみ呼び出すこと
unsafe fn toggle_engine() {
    if let Some(engine) = ENGINE.get_mut() {
        let (enabled, flush_resp) = engine.toggle_enabled();
        let mut timer_runtime = Win32TimerRuntime;
        let mut action_executor = SendInputExecutor;
        dispatch(&flush_resp, &mut timer_runtime, &mut action_executor);
        log::info!("Engine toggled: {}", if enabled { "ON" } else { "OFF" });
        if let Some(tray) = TRAY.get_mut() {
            tray.set_enabled(enabled);
        }
    }
}

/// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
///
/// IMEオフ、入力言語変更など、エンジンの前提が崩れた場合に呼ぶ。
/// 全てのコンテキスト無効化経路はこの関数を通すこと。
unsafe fn invalidate_engine_context(reason: ContextChange) {
    if let Some(engine) = ENGINE.get_mut() {
        let response = engine.flush_pending(reason);
        let mut timer_runtime = Win32TimerRuntime;
        let mut action_executor = SendInputExecutor;
        dispatch(&response, &mut timer_runtime, &mut action_executor);
    }
}

/// 設定画面 (awase-settings) を起動する
fn launch_settings() {
    let names = if cfg!(windows) {
        vec!["awase-settings.exe"]
    } else {
        vec!["awase-settings"]
    };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in &names {
                let path = dir.join(name);
                if path.exists() {
                    let _ = std::process::Command::new(&path).spawn();
                    return;
                }
            }
        }
    }
    log::warn!("awase-settings not found");
}

/// 設定ファイルを再読み込みし、エンジンのパラメータを更新する
///
/// Safety: シングルスレッドからのみ呼び出すこと
unsafe fn reload_config() {
    let config = match load_config() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to reload config: {e}");
            return;
        }
    };

    if let Some(engine) = ENGINE.get_mut() {
        engine.set_threshold_ms(config.general.simultaneous_threshold_ms);
        engine.set_confirm_mode(
            config.general.confirm_mode,
            config.general.speculative_delay_ms,
        );
        log::info!(
            "Engine parameters updated: threshold={}ms, confirm_mode={:?}, speculative_delay={}ms",
            config.general.simultaneous_threshold_ms,
            config.general.confirm_mode,
            config.general.speculative_delay_ms,
        );
    }

    // n-gram モデルの再読み込み
    init_ngram(&config);

    // フォーカスオーバーライドの再読み込み
    FOCUS_OVERRIDES.set(config.focus_overrides);
    log::info!("Focus overrides reloaded");

    // オーバーライド変更後はキャッシュをクリアして再判定を促す
    if let Some(cache) = FOCUS_CACHE.get_mut() {
        *cache = FocusCache::new();
    }

    log::info!("Config reloaded successfully");
}

/// layouts_dir 内の *.yab を全てスキャンして配列一覧を構築する
fn scan_layouts(
    layouts_dir: &Path,
    left_thumb_vk: u16,
    right_thumb_vk: u16,
) -> Result<Vec<(String, YabLayout, u16, u16)>> {
    let mut layouts = Vec::new();

    if !layouts_dir.is_dir() {
        log::warn!("Layouts directory not found: {}", layouts_dir.display());
        return Ok(layouts);
    }

    let entries = std::fs::read_dir(layouts_dir).with_context(|| {
        format!(
            "Failed to read layouts directory: {}",
            layouts_dir.display()
        )
    })?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "yab") {
            match std::fs::read_to_string(&path) {
                Ok(content) => match YabLayout::parse(&content) {
                    Ok(yab) => {
                        let yab = yab.resolve_kana();
                        log::info!("Discovered layout: {} ({})", yab.name, path.display());
                        layouts.push((yab.name.clone(), yab, left_thumb_vk, right_thumb_vk));
                    }
                    Err(e) => {
                        log::warn!("Failed to parse layout {}: {e}", path.display());
                    }
                },
                Err(e) => {
                    log::warn!("Failed to read layout file {}: {e}", path.display());
                }
            }
        }
    }

    // 名前順にソートして安定した順序にする
    layouts.sort_by(|(a, ..), (b, ..)| a.cmp(b));

    Ok(layouts)
}

/// 相対パスを実行ファイルのディレクトリ基準で解決する
fn resolve_relative(path: &str) -> PathBuf {
    if Path::new(path).is_absolute() {
        return PathBuf::from(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let resolved = dir.join(path);
            if resolved.exists() {
                return resolved;
            }
        }
    }
    // フォールバック: カレントディレクトリからの相対パス
    PathBuf::from(path)
}

/// 配列を動的に切り替える
///
/// Safety: シングルスレッドからのみ呼び出すこと
unsafe fn switch_layout(index: usize) {
    let Some(layouts) = LAYOUTS.get_ref() else {
        return;
    };

    let Some((name, layout_template, _l_vk, _r_vk)) = layouts.get(index) else {
        log::warn!("Layout index {index} out of range");
        return;
    };

    // YabLayout の各面を clone して新しいレイアウトを構築する
    let new_layout = YabLayout {
        name: layout_template.name.clone(),
        normal: layout_template.normal.clone(),
        left_thumb: layout_template.left_thumb.clone(),
        right_thumb: layout_template.right_thumb.clone(),
        shift: layout_template.shift.clone(),
    };

    if let Some(engine) = ENGINE.get_mut() {
        let response = engine.swap_layout(new_layout);
        let mut timer_runtime = Win32TimerRuntime;
        let mut action_executor = SendInputExecutor;
        dispatch(&response, &mut timer_runtime, &mut action_executor);
    }

    if let Some(tray) = TRAY.get_mut() {
        tray.set_layout_name(name);
    }

    log::info!("Switched layout to: {name}");
}

/// 設定ファイルのパスを探す
fn find_config_path() -> Result<PathBuf> {
    // 1. コマンドライン引数
    if let Some(path) = std::env::args().nth(1) {
        return Ok(PathBuf::from(path));
    }

    // 2. 実行ファイルディレクトリ → カレントディレクトリの順で config.toml を探す
    let resolved = resolve_relative("config.toml");
    if resolved.exists() {
        return Ok(resolved);
    }

    anyhow::bail!(
        "Config file not found. Place config.toml next to the executable, \
         or specify path as command line argument."
    )
}

/// Ctrl+C ハンドラを登録（Win32 SetConsoleCtrlHandler）
fn install_ctrl_handler() {
    unsafe extern "system" fn handler(_ctrl_type: u32) -> windows::Win32::Foundation::BOOL {
        QUIT_REQUESTED.store(true, Ordering::SeqCst);
        PostQuitMessage(0);
        windows::Win32::Foundation::BOOL(1) // TRUE = handled
    }

    unsafe {
        windows::Win32::System::Console::SetConsoleCtrlHandler(Some(handler), true).unwrap_or_else(
            |e| {
                log::warn!("Failed to set console ctrl handler: {e}");
            },
        );
    }
}
