//! 公開 API 型定義: Decision, Effect, InputContext, EngineCommand

use std::time::Duration;

use smallvec::SmallVec;

use crate::config::ParsedKeyCombo;
use crate::types::{ContextChange, KeyAction, RawKeyEvent, Timestamp};
use crate::yab::YabLayout;

use super::fsm_types::ModifierState;

// ── DecisionOrigin ──

/// Engine が `ImeEffect::SetOpen` を発行した理由の粒度付き分類。
///
/// `platform::EffectOrigin` より細かい粒度を持ち、engine 内部でどの経路から
/// IME 制御が要求されたかを Platform 層に伝えられる。
/// Platform 側では `From<DecisionOrigin> for EffectOrigin` を使って
/// 粗い `EffectOrigin` に変換する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionOrigin {
    /// NICOLA FSM (同時打鍵判定) の出力として IME 制御が必要になった
    NicolaFsm,
    /// 投機的送信（タイマー確定前の先行出力）
    Speculative,
    /// ペンディングタイマー満了による確定出力
    PendingTimer,
    /// 特殊キーコンボ（Ctrl+無変換等）による IME 制御バイパス
    Bypass,
    /// 文脈不明（初期化直後・観測同期等）
    Unknown,
}

// ── 副作用モデル（Effect / Decision / InputContext）──

/// ヒープ確保なしで 0〜4 個の Effect を格納できるインライン Vec。
pub type EffectVec = SmallVec<[Effect; 4]>;

/// 入力・出力に関する副作用
#[derive(Debug, Clone)]
pub enum InputEffect {
    /// キーアクションを出力する
    SendKeys(Vec<KeyAction>),
    /// キーをそのまま再注入する（IME OFF 時の deferred key 用）
    ReinjectKey(RawKeyEvent),
}

/// タイマーに関する副作用
#[derive(Debug, Clone)]
pub enum TimerEffect {
    /// タイマーを設定する
    Set { id: usize, duration: Duration },
    /// タイマーをキャンセルする
    Kill(usize),
}

/// IME 制御に関する副作用
#[derive(Debug, Clone)]
pub enum ImeEffect {
    /// IME の ON/OFF を設定する。
    ///
    /// `origin` で「Engine の意図」か「観測同期」かを区別する。
    /// Platform 側はこれを見てフォールバックキー送信（VK_KANJI 等）を
    /// 適用するか判断できる。
    SetOpen { open: bool, origin: DecisionOrigin },
    /// IME 状態更新を要求する (PostMessageW)
    RequestRefresh,
}

/// UI に関する副作用
#[derive(Debug, Clone)]
pub enum UiEffect {
    /// エンジンの有効/無効が変わった。
    /// `send_ime_key=false` の場合、IME モードキー送信を抑制する
    /// （NotRomajiInput 等、ユーザーが既に望むモードを選択済みの場合）。
    EngineStateChanged { enabled: bool, send_ime_key: bool },
}

/// アプリケーション全体の副作用を表す宣言型。
/// Engine は Effect を返すだけで、実行は呼び出し側が行う。
#[derive(Debug, Clone)]
pub enum Effect {
    Input(InputEffect),
    Timer(TimerEffect),
    Ime(ImeEffect),
    Ui(UiEffect),
}

// ── Activation 状態モデル ──

/// Engine の実効有効状態（3値）。
///
/// `Pending` を導入することで「フォーカス変更直後の観測待ち」を
/// false に落とさずに表現できる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationState {
    Active,
    Inactive(InactiveReason),
    /// 一時的に判断保留。直前が Active なら grace 期間中は Active 扱い。
    Pending(PendingReason),
}

/// 不活性の確定理由
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InactiveReason {
    /// ユーザーがホットキー等で明示的に無効化
    UserDisabled,
    /// IME が OFF（shadow=OFF が確定）
    ImeOff,
    /// ローマ字以外の入力方式（かな入力等）
    NotRomajiInput,
    /// 日本語以外の IME（英語、中国語等）
    NotJapaneseIme,
    /// フォーカスが非テキスト領域
    NonTextFocus,
}

/// 判断保留の理由
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingReason {
    /// フォーカス変更直後で probe 結果待ち
    FocusTransition { since_ms: u64 },
    /// observe() が連続失敗しているが閾値未満
    ObservationMissing { miss_count: u32 },
    /// IMM ブリッジ broken アプリで初回検出待ち
    ImmBridgeUnknown,
}

impl ActivationState {
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    /// `Inactive` を `ContextChange` にマップする（flush 理由として使用）。
    ///
    /// # Panics
    ///
    /// `Active` または `Pending` 状態で呼ばれた場合にパニックする。
    #[must_use]
    pub const fn to_context_change(self) -> ContextChange {
        match self {
            Self::Inactive(InactiveReason::UserDisabled) => ContextChange::EngineDisabled,
            Self::Inactive(InactiveReason::NonTextFocus) => ContextChange::FocusChanged,
            Self::Inactive(
                InactiveReason::ImeOff
                | InactiveReason::NotRomajiInput
                | InactiveReason::NotJapaneseIme,
            ) => ContextChange::ImeOff,
            Self::Active | Self::Pending(_) => {
                panic!("to_context_change called on non-inactive state")
            }
        }
    }
}

/// Engine の判断結果（副作用なし、値で消費される）。
///
/// `consumed: bool` ではなく enum で意味を固定する。
/// `PassThrough` なのに `SendKeys` が入る、といった不整合を型で防ぐ。
#[derive(Debug)]
pub enum Decision {
    /// キーを素通しする（副作用なし）
    PassThrough,
    /// キーを素通しするが副作用を伴う（例: IME トグルキーの pass-through + キャッシュ更新要求）
    PassThroughWith { effects: EffectVec },
    /// キーを消費する（副作用あり or なし）
    Consume { effects: EffectVec },
}

impl Decision {
    #[must_use]
    pub const fn pass_through() -> Self {
        Self::PassThrough
    }

    #[must_use]
    pub const fn pass_through_with(effects: EffectVec) -> Self {
        Self::PassThroughWith { effects }
    }

    #[must_use]
    pub fn consumed() -> Self {
        Self::Consume {
            effects: EffectVec::new(),
        }
    }

    #[must_use]
    pub const fn consumed_with(effects: EffectVec) -> Self {
        Self::Consume { effects }
    }

    /// Consume バリアントかどうかを返す
    #[must_use]
    pub const fn is_consumed(&self) -> bool {
        matches!(self, Self::Consume { .. })
    }

    /// effects に追加する。PassThrough なら PassThroughWith に昇格。
    pub fn push_effect(&mut self, effect: Effect) {
        self.effects_mut().push(effect);
    }

    /// Effects 内に `ImeEffect::SetOpen` があればその値を返す。
    /// フックコールバックで IME 制御キー検出後に即座に preconditions を更新するために使う。
    #[must_use]
    pub fn find_ime_set_open(&self) -> Option<bool> {
        let effects = match self {
            Self::Consume { effects } | Self::PassThroughWith { effects } => effects,
            Self::PassThrough => return None,
        };
        for effect in effects {
            if let Effect::Ime(ImeEffect::SetOpen { open, .. }) = effect {
                return Some(*open);
            }
        }
        None
    }

    /// effects の先頭に `prefix` を挿入する。
    ///
    /// 空の prefix は no-op。PassThrough なら `effects_mut()` 経由で PassThroughWith に昇格する。
    pub fn prepend_effects(&mut self, prefix: EffectVec) {
        if prefix.is_empty() {
            return;
        }
        let effects = self.effects_mut();
        let mut new_effects = prefix;
        new_effects.extend(effects.drain(..));
        *effects = new_effects;
    }

    /// effects への可変参照。PassThrough なら PassThroughWith に昇格して空 EffectVec を返す。
    #[must_use]
    pub fn effects_mut(&mut self) -> &mut EffectVec {
        match self {
            Self::Consume { effects } | Self::PassThroughWith { effects } => effects,
            Self::PassThrough => {
                // PassThrough に effect を足すと PassThroughWith になる（Consume ではない）。
                // Consume にすると元のキーイベントが OS に渡らなくなり、
                // IME ON/OFF キーが奪われて 2回押しが必要になる等の不具合を引き起こす。
                *self = Self::PassThroughWith {
                    effects: EffectVec::new(),
                };
                let Self::PassThroughWith { effects } = self else {
                    unreachable!("just assigned PassThroughWith")
                };
                effects
            }
        }
    }
}

// ── InputModeState ──

/// 入力方式の確度付き状態。
///
/// `bool` では「観測値」と「IMM broken アプリ向け仮定値」を区別できず、
/// Chrome 等で stale な false に上書きされる問題があるため確度を保持する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputModeState {
    /// IMM クエリ等でローマ字入力と確認できた
    ObservedRomaji,
    /// IMM クエリ等でかな入力と確認できた（ひらがな・JISかな。英数とは区別する）
    ObservedKana,
    /// IMM クエリ等で英数モードと確認できた（半角英数・全角英数）
    ObservedEisu,
    /// 観測不能だが状況証拠から romaji と仮定（Chrome/UWP/Electron 等）
    AssumedRomaji { reason: AssumedReason },
    /// 不明（起動直後、フォーカス確定前等）
    Unknown,
}

/// `AssumedRomaji` の根拠
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssumedReason {
    /// IMM ブリッジが broken と既知のクラス名（Chrome_WidgetWin_1 等）
    ImmBridgeBroken,
    /// フォーカス変更直後で観測確定前
    FocusTransition,
    /// AppKind が TsfNative/UWP で IMM クエリをスキップしている
    AppKindExcluded,
    /// 強制 ON ガード中（連続検出失敗による）
    ForceOnGuardActive,
}

impl InputModeState {
    /// ローマ字入力と判断できるかどうか。
    /// `ObservedRomaji` と `AssumedRomaji` を true とみなす。
    #[must_use]
    pub const fn is_romaji_capable(self) -> bool {
        matches!(self, Self::ObservedRomaji | Self::AssumedRomaji { .. })
    }
}

/// idle 中の conv mode チェック（`kp_stage_idle_conv_check`）を実行すべきか判定する。
///
/// 4 つのガード条件をまとめた純粋関数。Win32 API を呼ばないため Linux でもテスト可能。
///
/// # 引数
/// - `is_key_down`: KeyDown イベントかどうか（KeyUp はスキップ）
/// - `is_tsf_native`: フォーカスアプリが TsfNative プロファイルかどうか
/// - `in_flight_ms`: `output_in_flight_ms()` の値（`u64::MAX` = cold start）
/// - `explicit_age_ms`: `explicit_ime_action_age_ms()` の値（`u64::MAX` = 操作なし）
/// - `typing_idle_ms`: タイピング停止とみなす閾値（`TYPING_IDLE_MS`、通常 500ms）
/// - `explicit_suppress_ms`: 明示的 IME 操作後の抑制窓（`EXPLICIT_IME_SUPPRESS_MS`、通常 1500ms）
#[must_use]
pub fn should_run_idle_conv_check(
    is_key_down: bool,
    is_tsf_native: bool,
    in_flight_ms: u64,
    explicit_age_ms: u64,
    typing_idle_ms: u64,
    explicit_suppress_ms: u64,
) -> bool {
    // ガード 1: KeyDown イベントのみ対象
    if !is_key_down {
        return false;
    }
    // ガード 2: TsfNative アプリのみ（WezTerm 等）
    if !is_tsf_native {
        return false;
    }
    // ガード 3: タイピング停止後のみ（in_flight_ms > typing_idle_ms）
    // u64::MAX（cold start）は typing_idle_ms より大きいため通過する
    if in_flight_ms <= typing_idle_ms {
        return false;
    }
    // ガード 4: 明示的 IME 操作直後はスキップ
    // Ctrl+変換/無変換 後に GJI probe が ROMAN ビットを確立する前に
    // check が走って belief を誤上書きするのを防ぐ
    if explicit_age_ms < explicit_suppress_ms {
        return false;
    }
    true
}

/// Engine が判断に使う外部コンテキスト（読み取り専用）。
///
/// # 設計ルール
/// - OS 由来の「瞬間値」のみを含む（ポーリングで変わる可能性のある値）
/// - Engine 内部で保持できる永続状態は Engine 側に寄せる
/// - 副作用結果を反映したい場合は Effect 経由で表現する
/// - このフィールドを増やす前に、Engine 内部状態で代替できないか検討すること
#[derive(Debug, Clone, Copy)]
pub struct InputContext {
    // ── Environment preconditions ──
    /// IME が ON か（Platform 層がアトミック変数から取得、shadow 反映済み）
    pub ime_on: bool,
    /// 入力方式の確度付き状態（ObservedRomaji / AssumedRomaji / ObservedKana / Unknown）
    pub input_mode: InputModeState,
    /// 日本語 IME がアクティブか（MS-IME, Google, ATOK 等）
    pub is_japanese_ime: bool,
    // ── Physical key state (provided by Platform) ──
    /// 修飾キー状態（OS 実状態 — コンボキー検出・NicolaFsm の OsModifierHeld 判定用）
    pub modifiers: ModifierState,
    /// 左親指キー押下時刻（None = 非押下）
    pub left_thumb_down: Option<Timestamp>,
    /// 右親指キー押下時刻（None = 非押下）
    pub right_thumb_down: Option<Timestamp>,
}


/// エンジン切替・IME 制御の特殊キーコンボを集約する構造体。
#[derive(Debug)]
pub struct SpecialKeyCombos {
    pub engine_on: Vec<ParsedKeyCombo>,
    pub engine_off: Vec<ParsedKeyCombo>,
    pub ime_on: Vec<ParsedKeyCombo>,
    pub ime_off: Vec<ParsedKeyCombo>,
}

/// Engine への外部コマンド
#[derive(Debug)]
pub enum EngineCommand {
    /// エンジンの有効/無効を切り替える
    ToggleEngine,
    /// 外部コンテキスト喪失（IME OFF、言語切替等）
    InvalidateContext(ContextChange),
    /// 配列を切り替える
    SwapLayout(YabLayout),
    /// 特殊キーコンボを再読み込みする
    ReloadKeys { special: SpecialKeyCombos },
    /// FSM パラメータを更新する
    UpdateFsmParams {
        threshold_ms: u32,
        confirm_mode: crate::config::ConfirmMode,
        speculative_delay_ms: u32,
    },
    /// n-gram モデルを設定する
    SetNgramModel(crate::ngram::NgramModel),
    /// IME 状態を再チェックする（Platform 層がアトミック変数を更新済み）
    RefreshState,
    /// 前面プロセスが変更された（デバウンス後に Platform 層が検出、ADR 028）
    FocusChanged,
}

#[cfg(test)]
mod tests {
    use smallvec::smallvec;

    use super::*;

    fn test_effect() -> Effect {
        Effect::Ui(UiEffect::EngineStateChanged { enabled: true, send_ime_key: true })
    }

    // ── Decision factory methods ──

    #[test]
    fn pass_through_creates_pass_through() {
        let d = Decision::pass_through();
        assert!(matches!(d, Decision::PassThrough));
    }

    #[test]
    fn consumed_creates_consume_with_empty_effects() {
        let d = Decision::consumed();
        match d {
            Decision::Consume { effects } => assert!(effects.is_empty()),
            other => panic!("expected Consume, got {:?}", other),
        }
    }

    #[test]
    fn consumed_with_creates_consume_with_effects() {
        let d = Decision::consumed_with(smallvec![test_effect()]);
        match d {
            Decision::Consume { effects } => assert_eq!(effects.len(), 1),
            other => panic!("expected Consume, got {:?}", other),
        }
    }

    #[test]
    fn pass_through_with_creates_pass_through_with() {
        let d = Decision::pass_through_with(smallvec![test_effect()]);
        match d {
            Decision::PassThroughWith { effects } => assert_eq!(effects.len(), 1),
            other => panic!("expected PassThroughWith, got {:?}", other),
        }
    }

    // ── is_consumed ──

    #[test]
    fn is_consumed_true_for_consume() {
        assert!(Decision::consumed().is_consumed());
    }

    #[test]
    fn is_consumed_false_for_pass_through() {
        assert!(!Decision::pass_through().is_consumed());
    }

    #[test]
    fn is_consumed_false_for_pass_through_with() {
        assert!(!Decision::pass_through_with(smallvec![]).is_consumed());
    }

    // ── push_effect ──

    #[test]
    fn push_effect_on_pass_through_promotes_to_pass_through_with() {
        let mut d = Decision::pass_through();
        d.push_effect(test_effect());
        // PassThrough + effect should become PassThroughWith, NOT Consume.
        // Consuming here would steal IME control keys from the OS.
        assert!(!d.is_consumed());
        match d {
            Decision::PassThroughWith { effects } => assert_eq!(effects.len(), 1),
            other => panic!("expected PassThroughWith, got {:?}", other),
        }
    }

    #[test]
    fn push_effect_on_consume_appends() {
        let mut d = Decision::consumed_with(smallvec![test_effect()]);
        d.push_effect(test_effect());
        match d {
            Decision::Consume { effects } => assert_eq!(effects.len(), 2),
            other => panic!("expected Consume, got {:?}", other),
        }
    }

    #[test]
    fn push_effect_on_pass_through_with_appends() {
        let mut d = Decision::pass_through_with(smallvec![test_effect()]);
        d.push_effect(test_effect());
        match d {
            Decision::PassThroughWith { effects } => assert_eq!(effects.len(), 2),
            other => panic!("expected PassThroughWith, got {:?}", other),
        }
    }

    // ── should_run_idle_conv_check ────────────────────────────────────────────

    const IDLE_MS: u64 = 500;   // TYPING_IDLE_MS
    const SUPPRESS_MS: u64 = 1500; // EXPLICIT_IME_SUPPRESS_MS

    fn run_ok(in_flight: u64, explicit_age: u64) -> bool {
        should_run_idle_conv_check(true, true, in_flight, explicit_age, IDLE_MS, SUPPRESS_MS)
    }

    // ── ガード 1: KeyDown のみ ──
    #[test]
    fn guard1_key_up_skips() {
        assert!(!should_run_idle_conv_check(false, true, u64::MAX, u64::MAX, IDLE_MS, SUPPRESS_MS));
    }

    #[test]
    fn guard1_key_down_passes() {
        assert!(should_run_idle_conv_check(true, true, u64::MAX, u64::MAX, IDLE_MS, SUPPRESS_MS));
    }

    // ── ガード 2: TsfNative のみ ──
    #[test]
    fn guard2_non_tsf_native_skips() {
        assert!(!should_run_idle_conv_check(true, false, u64::MAX, u64::MAX, IDLE_MS, SUPPRESS_MS));
    }

    // ── ガード 3: タイピング停止後のみ ──
    #[test]
    fn guard3_typing_in_progress_skips() {
        // in_flight が IDLE_MS 以下 → タイピング中 → スキップ
        assert!(!run_ok(IDLE_MS, u64::MAX));
        assert!(!run_ok(0, u64::MAX));
        assert!(!run_ok(1, u64::MAX));
    }

    #[test]
    fn guard3_just_above_idle_threshold_passes() {
        // IDLE_MS + 1ms → 停止とみなす
        assert!(run_ok(IDLE_MS + 1, u64::MAX));
    }

    #[test]
    fn guard3_cold_start_passes() {
        // u64::MAX（cold start）は IDLE_MS より大きい → 通過
        assert!(run_ok(u64::MAX, u64::MAX));
    }

    // ── ガード 4: 明示的 IME 操作直後の抑制窓 ──
    #[test]
    fn guard4_within_suppress_window_skips() {
        assert!(!run_ok(u64::MAX, 0));
        assert!(!run_ok(u64::MAX, SUPPRESS_MS - 1));
    }

    #[test]
    fn guard4_at_suppress_boundary_passes() {
        // explicit_age == SUPPRESS_MS → `<` 条件が成立しない → 通過
        assert!(run_ok(u64::MAX, SUPPRESS_MS));
    }

    #[test]
    fn guard4_no_explicit_action_passes() {
        // u64::MAX（操作なし）→ SUPPRESS_MS 以上 → 通過
        assert!(run_ok(u64::MAX, u64::MAX));
    }

    // ── 全条件通過（通常の idle チェック）──
    #[test]
    fn all_guards_pass_for_normal_idle() {
        // 600ms 停止後、2000ms 前に IME 操作 → 通過
        assert!(run_ok(IDLE_MS + 100, SUPPRESS_MS + 500));
    }

    // ── 複合スキップ ──
    #[test]
    fn multiple_guards_fail_still_skips() {
        // KeyUp かつ typing 中 → どちらもスキップ条件
        assert!(!should_run_idle_conv_check(false, true, IDLE_MS, u64::MAX, IDLE_MS, SUPPRESS_MS));
    }

    #[test]
    fn effects_mut_on_pass_through_promotes_to_pass_through_with() {
        let mut d = Decision::pass_through();
        let effects = d.effects_mut();
        assert!(effects.is_empty());
        effects.push(test_effect());
        // PassThrough + effect should become PassThroughWith, NOT Consume.
        assert!(!d.is_consumed());
        match d {
            Decision::PassThroughWith { effects } => assert_eq!(effects.len(), 1),
            other => panic!("expected PassThroughWith, got {:?}", other),
        }
    }
}
