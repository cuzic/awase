use awase::engine::InputModeState;

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
    /// フォーカス変更直後の高速スナップショット
    FocusSnapshot,
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
    pub(in crate::state) ime_on: bool,
    /// `ime_on` を最後に更新したソース（Phase 2 解決関数向けの診断情報）
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
    /// IME 状態検出の連続失敗回数。
    ///
    /// `read_ime_state_full()` が `ime_on = None` を返すたびにインクリメントされ、
    /// 検出成功時またはシャドウ更新（ユーザー操作）時にリセットされる。
    /// [`crate::tuning::IME_DETECT_MISS_THRESHOLD`] に達すると `refresh_ime_state_cache` が
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
    pub(in crate::state) ime_detect_miss_count: u32,
    /// 起動直後の broken app 向け強制 IME ON ガード（Phase 3.5）。
    ///
    /// 未知アプリへの初回フォーカス時に `set_ime_open(true)` を呼んだ後、
    /// 次の `observe()` が即座に上書きしないよう 1 サイクルだけ保護する。
    /// 次の IME 検出成功（`clear_force_on_guard=true`）で解除される。
    pub(in crate::state) force_on_broken_app_bootstrap: bool,
    /// パニックリセット直後の上書き防止ガード。
    ///
    /// パニックリセットで `ime_on=true` を書き込んだ直後に `refresh_ime_state_cache`
    /// が走ると stale な `observe()` の結果に上書きされてしまう。これを防ぐ。
    /// `apply_ime_observer_output` で `clear_force_on_guard=true` のとき解除される。
    pub(in crate::state) force_on_panic_reset: bool,
}

impl Preconditions {
    /// `ime_on` と `ime_on_source` をまとめて更新する。
    pub(in crate::state) fn set_ime_on(&mut self, value: bool, source: ShadowSource) {
        self.ime_on = value;
        self.ime_on_source = source;
    }

    // ── pub(crate) 読み取り getter ──

    /// IME が ON かどうかを返す。
    #[inline]
    pub(crate) fn ime_on(&self) -> bool {
        self.ime_on
    }

    /// `ime_on` を最後に更新したソースを返す。
    #[inline]
    pub(crate) fn ime_on_source(&self) -> ShadowSource {
        self.ime_on_source
    }

    /// 入力モードを返す。
    #[inline]
    pub(crate) fn input_mode(&self) -> InputModeState {
        self.input_mode
    }

    /// 日本語 IME がアクティブかを返す。
    #[inline]
    pub(crate) fn is_japanese_ime(&self) -> bool {
        self.is_japanese_ime
    }

    /// 直前の conversion_mode を返す。
    #[inline]
    pub(crate) fn prev_conversion_mode(&self) -> Option<u32> {
        self.prev_conversion_mode
    }

    /// IME 状態検出の連続失敗回数を返す。
    #[inline]
    pub(crate) fn ime_detect_miss_count(&self) -> u32 {
        self.ime_detect_miss_count
    }

    /// いずれかの強制 ON ガードが立っているかを返す。
    #[inline]
    pub(crate) fn is_force_on_guard_active(&self) -> bool {
        self.force_on_broken_app_bootstrap || self.force_on_panic_reset
    }
}
