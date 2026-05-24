use awase::engine::InputModeState;

/// `ImeBelief.ime_on` を最後に更新したソース。
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
    /// フォーカス変更直後の高速スナップショット
    FocusSnapshot,
    /// panic_reset（強制リセット）
    PanicReset,
    /// per-HWND IME キャッシュからの復元（フォーカス切り替え時の即時復元）
    HwndCache,
}

/// 観測値から導出した現在の IME 状態への「信念」。
///
/// 複数の観測ソースをマージした結果を保持する。`ime_on` は
/// Engine に渡す判断用の意図値であり、観測ソースの優先度マージ結果である。
/// OS の現在状態を直接反映するとは限らない点に注意。
///
/// FSM 的な累積状態（miss_count, force_on ガード等）は
/// [`ImeRecoveryState`] に分離されている。
#[derive(Debug)]
pub struct ImeBelief {
    /// OS の IME ON/OFF 状態のシャドウ値（Single Source of Truth）。
    /// `LastAppliedImeState` はキー送信ログであり IME 状態ではない。
    ///
    /// 複数の観測ソース
    /// （sync_key > physical_key > set_open_request > focus_probe / observer_poll）
    /// の優先度マージ結果。`resolve_and_clear()` で確定し、Engine に渡される。
    pub(in crate::state) ime_on: bool,
    /// `ime_on` を最後に更新したソース（診断情報）
    pub(in crate::state) ime_on_source: ShadowSource,
    /// 入力モード（ローマ字 / かな / 不明）
    ///
    /// `hook.rs` がフックコールバック内で直接読み取るため `pub(crate)` とする。
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

    /// IME ON/OFF の Engine 向け優先度マージ値を返す。
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

/// IME 回復ロジック用の FSM 累積状態。
///
/// 複数 tick にまたがって蓄積される状態であり、現在の観測値だけから
/// 再構築できない。`ImeBelief`（導出済み）とは分離して管理する。
///
/// - `ime_detect_miss_count`: 連続検出失敗カウンタ
/// - `force_on_broken_app_bootstrap`: 未知 Imm32Unavailable アプリ向けガード
/// - `force_on_panic_reset`: パニックリセット直後の上書き防止ガード
#[derive(Debug)]
pub struct ImeRecoveryState {
    /// IME 状態検出の連続失敗回数。
    ///
    /// `read_ime_state_full()` が `ime_on = None` を返すたびにインクリメントされ、
    /// 検出成功時またはシャドウ更新（ユーザー操作）時にリセットされる。
    pub(in crate::state) ime_detect_miss_count: u32,
    /// 起動直後の Imm32Unavailable アプリ向け強制 IME ON ガード。
    ///
    /// 次の IME 検出成功（`ImeUpdate::clear_force_on_broken_app_bootstrap=true`）で解除される。
    pub(in crate::state) force_on_broken_app_bootstrap: bool,
    /// パニックリセット直後の上書き防止ガード。
    ///
    /// `apply_ime_update` で `ImeUpdate::clear_force_on_panic_reset=true` のとき解除される。
    /// 解除時は `ime_detect_miss_count` も 0 にリセットされる。
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
