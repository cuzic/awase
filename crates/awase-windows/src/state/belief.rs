//! IME 状態の「信念」と回復 FSM 状態。
//!
//! # IME 状態の 4 層モデル
//!
//! このシステムの IME 状態は 4 つの概念的な層に分かれる。
//! 各層は独立したデータ型を持ち、目的が異なる。
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ Layer 1: 生観測スロット (ImeObservations)                    │
//! │  sync_key > physical_key > set_open_request                 │
//! │  > focus_probe > observer_poll                              │
//! │  各ソースが書き込み → resolve_and_clear() で消費される       │
//! └────────────────────┬────────────────────────────────────────┘
//!                      │ resolve_and_clear() + apply_ime_observations()
//! ┌────────────────────▼────────────────────────────────────────┐
//! │ Layer 2: Engine 入力用 belief (ImeBelief.ime_on)            │
//! │  優先度マージ済みの値。Engine::on_input() に渡す SSOT。      │
//! │  「OS の現在状態」ではなく「Engine が前提とすべき状態」。    │
//! └────────────────────┬────────────────────────────────────────┘
//!                      │ Engine の判断結果
//! ┌────────────────────▼────────────────────────────────────────┐
//! │ Layer 3: Engine 出力 (ImeEffect::SetOpen / is_user_enabled) │
//! │  Engine が「IME を ON/OFF したい」と決定した結果。           │
//! └────────────────────┬────────────────────────────────────────┘
//!                      │ apply_ime_open() → OS に送信
//! ┌────────────────────▼────────────────────────────────────────┐
//! │ Layer 4: 制御ログ (Output::last_applied_ime_on)             │
//! │  最後に OS に送ったコマンド値。観測値ではない。              │
//! │  VK_KANJI の重複送信防止にのみ使用する。                    │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## よくある混同
//!
//! - **`ImeBelief.ime_on` ≠ OS の現在 IME 状態**: フォーカス変更直後や
//!   Imm32Unavailable アプリでは OS 実態と乖離することがある。
//!   これは設計上許容された誤差であり、ポーリングで収束する。
//! - **`last_applied_ime_on` ≠ `ImeBelief.ime_on`**: 前者は「送ったコマンド」、
//!   後者は「Engine が前提とする状態」。desync はありうる（それを検出するのが
//!   `KanjiToggleStrategy` の `candidate_visible` による補正）。

use awase::engine::InputModeState;

/// `ImeBelief.ime_on` を最後に更新したソース（診断用）。
///
/// どの Layer 1 スロットが `resolve_and_clear()` で勝ったかを記録する。
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
    /// `ImeEffect::SetOpen`（Engine の判断による強制設定）
    SetOpenRequest,
    /// IME observer ポーリング（バックグラウンド観測）
    ObserverPoll,
    /// フォーカス変更直後の高速スナップショット
    FocusSnapshot,
    /// panic_reset（強制リセット）
    PanicReset,
    /// per-HWND IME キャッシュからの復元（フォーカス切り替え時の即時復元）
    HwndCache,
}

/// Layer 2: 観測値から導出した Engine 入力用の IME 状態。
///
/// `ImeObservations` の優先度スロットを `resolve_and_clear()` でマージした結果を
/// 保持する。Engine::on_input() に渡す入力コンテキストの SSOT。
///
/// ## `ime_on` の意味
///
/// `ime_on` は「OS の現在状態」ではなく「Engine が前提とすべき状態」である。
/// フォーカス変更直後や Imm32Unavailable アプリでは OS 実態と一時的に乖離するが、
/// それは設計上許容された誤差（ポーリングで収束する）。
///
/// Layer 4 の `Output::last_applied_ime_on`（制御ログ）とも異なる。
/// 詳細は [`crate::state::belief`] モジュールドキュメントの 4 層モデルを参照。
///
/// ## FSM 累積状態との分離
///
/// 複数 tick にまたがる累積状態（miss_count, force_on ガード等）は
/// [`ImeRecoveryState`] に分離されている。
#[derive(Debug)]
pub struct ImeBelief {
    /// Layer 2 の SSOT。複数観測ソースの優先度マージ結果。
    ///
    /// 優先度順: sync_key > physical_key > set_open_request > focus_probe > observer_poll。
    /// `resolve_and_clear()` で確定し、`apply_ime_observations()` 経由で更新される。
    pub(in crate::state) ime_on: bool,
    /// `ime_on` を最後に更新した Layer 1 スロット（診断情報）
    pub(in crate::state) ime_on_source: ShadowSource,
    /// 入力モード（ローマ字 / かな / 不明）
    ///
    /// `hook.rs` がフックコールバック内で直接読み取るため `pub(crate)` とする。
    /// 書き込みは `PlatformState::set_input_mode()` 経由で行うこと。
    pub(crate) input_mode: InputModeState,
    /// 日本語 IME がアクティブか
    pub(in crate::state) is_japanese_ime: bool,
    /// 直前の conversion_mode（ROMAN ビット消失によるかな切替検出用）
    /// None = まだ一度も取得できていない
    pub(in crate::state) prev_conversion_mode: Option<u32>,
}

impl ImeBelief {
    /// `ime_on` と `ime_on_source` をまとめて更新する。
    pub(in crate::state) const fn set_ime_on(&mut self, value: bool, source: ShadowSource) {
        self.ime_on = value;
        self.ime_on_source = source;
    }

    // ── pub(crate) 読み取り getter ──

    /// Engine 入力用の IME ON/OFF 優先度マージ値を返す（Layer 2 SSOT）。
    #[inline]
    pub(crate) const fn ime_on(&self) -> bool {
        self.ime_on
    }

    /// `ime_on` を最後に更新したソースを返す。
    #[inline]
    pub(crate) const fn ime_on_source(&self) -> ShadowSource {
        self.ime_on_source
    }

    /// 入力モードを返す。
    #[inline]
    pub(crate) const fn input_mode(&self) -> InputModeState {
        self.input_mode
    }

    /// 日本語 IME がアクティブかを返す。
    #[inline]
    pub(crate) const fn is_japanese_ime(&self) -> bool {
        self.is_japanese_ime
    }

    /// 直前の conversion_mode を返す。
    #[inline]
    pub(crate) const fn prev_conversion_mode(&self) -> Option<u32> {
        self.prev_conversion_mode
    }
}

/// Layer 2 付随: IME 回復ロジック用の FSM 累積状態。
///
/// 複数 tick にまたがって蓄積される状態であり、現在の観測値だけから
/// 再構築できない。`ImeBelief`（tick ごとに更新される導出値）とは分離して管理する。
///
/// ## 各フィールドの役割
///
/// - `ime_detect_miss_count`: 連続検出失敗カウンタ。閾値到達で force-ON を発火。
/// - `force_on_broken_app_bootstrap`: 未知 Imm32Unavailable アプリへの初回フォーカス時に
///   IME が OFF 扱いになるのを防ぐ 1 サイクルガード。
/// - `force_on_panic_reset`: パニックリセット直後に stale な poll 結果で
///   `ime_on=true` が上書きされるのを防ぐガード。
///
/// 両ガードは [`ImeRecoveryState::is_force_on_guard_active`] で OR 判定される。
/// セット箇所と解除時の副作用が異なるため別フィールドにしている
/// （詳細は各フィールドの doc を参照）。
#[derive(Debug)]
pub struct ImeRecoveryState {
    /// IME 状態検出の連続失敗回数。
    ///
    /// `read_ime_state_full()` が `ime_on = None` を返すたびにインクリメントされ、
    /// 検出成功時またはユーザー操作（`reset_ime_detect_state`）でリセットされる。
    /// [`crate::IME_DETECT_MISS_THRESHOLD`] 到達で force-ON を発火する。
    pub(in crate::state) ime_detect_miss_count: u32,
    /// Imm32Unavailable アプリ向け強制 IME ON ガード。
    ///
    /// `try_force_on_bootstrap` でセット。
    /// 次の検出成功（`ImeUpdate::clear_force_on_broken_app_bootstrap=true`）で解除。
    /// 解除時に `ime_detect_miss_count` は変更しない。
    pub(in crate::state) force_on_broken_app_bootstrap: bool,
    /// パニックリセット直後の上書き防止ガード。
    ///
    /// `apply_panic_reset` でセット。
    /// `apply_ime_update` で `ImeUpdate::clear_force_on_panic_reset=true` のとき解除。
    /// 解除時は `ime_detect_miss_count` も 0 にリセットされる（bootstrap 側との差分）。
    pub(in crate::state) force_on_panic_reset: bool,
}

impl ImeRecoveryState {
    /// IME 状態検出の連続失敗回数を返す。
    #[inline]
    pub(crate) const fn ime_detect_miss_count(&self) -> u32 {
        self.ime_detect_miss_count
    }

    /// いずれかの強制 ON ガードが立っているかを返す。
    #[inline]
    pub(crate) const fn is_force_on_guard_active(&self) -> bool {
        self.force_on_broken_app_bootstrap || self.force_on_panic_reset
    }
}
