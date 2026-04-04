//! 公開 API 型定義: Decision, Effect, InputContext, EngineCommand

use std::time::Duration;

use smallvec::{smallvec, SmallVec};

use crate::config::ParsedKeyCombo;
use crate::types::{ContextChange, KeyAction, RawKeyEvent, Timestamp, VkCode};
use crate::yab::YabLayout;

use super::fsm_types::ModifierState;
use super::input_tracker::PhysicalKeyState;
use super::observation::FocusObservation;

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
    /// IME の ON/OFF を設定する
    SetOpen(bool),
    /// IME 状態更新を要求する (PostMessageW)
    RequestRefresh,
}

/// UI に関する副作用
#[derive(Debug, Clone)]
pub enum UiEffect {
    /// エンジンの有効/無効が変わった
    EngineStateChanged { enabled: bool },
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
    pub fn pass_through() -> Self {
        Self::PassThrough
    }

    #[must_use]
    pub fn pass_through_with(effects: EffectVec) -> Self {
        Self::PassThroughWith { effects }
    }

    #[must_use]
    pub fn consumed() -> Self {
        Self::Consume {
            effects: EffectVec::new(),
        }
    }

    #[must_use]
    pub fn consumed_with(effects: EffectVec) -> Self {
        Self::Consume { effects }
    }

    /// Consume バリアントかどうかを返す
    #[must_use]
    pub const fn is_consumed(&self) -> bool {
        matches!(self, Self::Consume { .. })
    }

    /// effects に追加する。PassThrough なら Consume に昇格。
    pub fn push_effect(&mut self, effect: Effect) {
        match self {
            Self::Consume { effects } | Self::PassThroughWith { effects } => {
                effects.push(effect);
            }
            Self::PassThrough => {
                *self = Self::Consume {
                    effects: smallvec![effect],
                };
            }
        }
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
            if let Effect::Ime(ImeEffect::SetOpen(open)) = effect {
                return Some(*open);
            }
        }
        None
    }

    /// effects への可変参照。PassThrough なら Consume に昇格して空 EffectVec を返す。
    #[must_use]
    pub fn effects_mut(&mut self) -> &mut EffectVec {
        match self {
            Self::Consume { effects } | Self::PassThroughWith { effects } => effects,
            Self::PassThrough => {
                *self = Self::Consume {
                    effects: EffectVec::new(),
                };
                let Self::Consume { effects } = self else {
                    unreachable!("just assigned Consume")
                };
                effects
            }
        }
    }
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
    /// ローマ字入力モードか（false = かな入力）
    pub is_romaji: bool,
    /// 日本語 IME がアクティブか（MS-IME, Google, ATOK 等）
    pub is_japanese_ime: bool,
    // ── Physical key state (provided by Platform) ──
    /// 修飾キー状態
    pub modifiers: ModifierState,
    /// 左親指キー押下時刻（None = 非押下）
    pub left_thumb_down: Option<Timestamp>,
    /// 右親指キー押下時刻（None = 非押下）
    pub right_thumb_down: Option<Timestamp>,
}

/// IME 同期キー（トグル・ON・OFF）を集約する構造体
#[derive(Debug)]
pub struct ImeSyncKeys {
    pub toggle: Vec<VkCode>,
    pub on: Vec<VkCode>,
    pub off: Vec<VkCode>,
}

/// エンジン切替・IME 制御の特殊キーコンボを集約する構造体。
#[derive(Debug)]
pub struct SpecialKeyCombos {
    pub engine_on: Vec<ParsedKeyCombo>,
    pub engine_off: Vec<ParsedKeyCombo>,
    pub ime_on: Vec<ParsedKeyCombo>,
    pub ime_off: Vec<ParsedKeyCombo>,
}

/// キーイベントバッファ管理
///
/// フック → メッセージループ間のキーイベント遅延・バッファリングを管理する。
/// OS 副作用は持たず、Engine メソッドがオーケストレーションを行う。
#[derive(Debug)]
pub struct KeyBuffer {
    /// IME 制御キー直後のガードフラグ（true: 後続キーを遅延処理する）
    pub ime_transition_guard: bool,
    /// ガード中に遅延されたキーイベント + 物理キー状態のバッファ
    pub deferred_keys: Vec<(RawKeyEvent, PhysicalKeyState)>,
}

impl Default for KeyBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyBuffer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ime_transition_guard: false,
            deferred_keys: Vec::new(),
        }
    }

    #[must_use]
    pub const fn is_guarded(&self) -> bool {
        self.ime_transition_guard
    }

    pub const fn set_guard(&mut self, on: bool) {
        self.ime_transition_guard = on;
    }

    pub fn push_deferred(&mut self, event: RawKeyEvent, phys: PhysicalKeyState) {
        self.deferred_keys.push((event, phys));
    }

    pub fn drain_deferred(&mut self) -> Vec<(RawKeyEvent, PhysicalKeyState)> {
        std::mem::take(&mut self.deferred_keys)
    }
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
    ReloadKeys {
        special: SpecialKeyCombos,
    },
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
    /// フォーカスが変更された
    FocusChanged(FocusObservation),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_effect() -> Effect {
        Effect::Ui(UiEffect::EngineStateChanged { enabled: true })
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
    fn push_effect_on_pass_through_promotes_to_consume() {
        let mut d = Decision::pass_through();
        d.push_effect(test_effect());
        assert!(d.is_consumed());
        match d {
            Decision::Consume { effects } => assert_eq!(effects.len(), 1),
            other => panic!("expected Consume, got {:?}", other),
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

    // ── effects_mut ──

    #[test]
    fn effects_mut_on_pass_through_promotes_to_consume() {
        let mut d = Decision::pass_through();
        let effects = d.effects_mut();
        assert!(effects.is_empty());
        effects.push(test_effect());
        assert!(d.is_consumed());
        match d {
            Decision::Consume { effects } => assert_eq!(effects.len(), 1),
            other => panic!("expected Consume, got {:?}", other),
        }
    }

    // ── KeyBuffer ──

    #[test]
    fn key_buffer_new_starts_empty_and_not_guarded() {
        let buf = KeyBuffer::new();
        assert!(!buf.is_guarded());
        assert!(buf.deferred_keys.is_empty());
    }

    #[test]
    fn key_buffer_set_guard_is_guarded_round_trip() {
        let mut buf = KeyBuffer::new();
        buf.set_guard(true);
        assert!(buf.is_guarded());
        buf.set_guard(false);
        assert!(!buf.is_guarded());
    }

    #[test]
    fn key_buffer_push_and_drain_deferred() {
        use crate::engine::input_tracker::PhysicalKeyState;

        let mut buf = KeyBuffer::new();
        let raw = RawKeyEvent {
            vk_code: VkCode(0x41),
            scan_code: crate::types::ScanCode(0x1E),
            event_type: crate::types::KeyEventType::KeyDown,
            extra_info: 0,
            timestamp: 0,
            key_classification: crate::types::KeyClassification::Char,
            physical_pos: None,
            ime_relevance: crate::types::ImeRelevance {
                may_change_ime: false,
                shadow_action: None,
                is_sync_key: false,
                sync_direction: None,
                is_ime_control: false,
            },
            modifier_key: None,
        };
        let phys = PhysicalKeyState::empty();

        buf.push_deferred(raw.clone(), phys);
        buf.push_deferred(raw, phys);
        assert_eq!(buf.deferred_keys.len(), 2);

        let drained = buf.drain_deferred();
        assert_eq!(drained.len(), 2);
        assert!(buf.deferred_keys.is_empty());
    }
}
