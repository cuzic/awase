use awase::engine::InputModeState;

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

/// `ime_force_on_guard` の 2 用途を型レベルで区別する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImeForceOnGuard {
    #[default]
    Inactive,
    BrokenAppBootstrap,
    PanicResetProtect,
}

impl ImeForceOnGuard {
    pub fn is_active(self) -> bool {
        self != Self::Inactive
    }
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
    pub ime_force_on_guard: ImeForceOnGuard,
}

impl Preconditions {
    /// `ime_on` と `ime_on_source` をまとめて更新する。
    pub fn set_ime_on(&mut self, value: bool, source: ShadowSource) {
        self.ime_on = value;
        self.ime_on_source = source;
    }
}
