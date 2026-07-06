//! FSM 内部で使用する型定義

use std::time::Duration;

use smallvec::SmallVec;

use crate::scanmap::PhysicalPos;
use crate::types::{KeyAction, ScanCode, Timestamp, VkCode};

/// 同時打鍵判定用タイマー ID
pub const TIMER_PENDING: usize = 1;

/// TwoPhase モード: Phase 1（短い待機）→ Phase 2（投機出力）遷移用タイマー ID
pub const TIMER_SPECULATIVE: usize = 2;

/// キーの分類（フック受信時に一度だけ決定）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyClass {
    /// 文字キー（配列変換の対象）
    Char,
    /// 左親指キー
    LeftThumb,
    /// 右親指キー
    RightThumb,
    /// パススルー（修飾キー、Fキー、ナビゲーション等）
    Passthrough,
}

impl KeyClass {
    #[must_use]
    pub const fn is_thumb(self) -> bool {
        matches!(self, Self::LeftThumb | Self::RightThumb)
    }

    #[must_use]
    pub const fn is_left_thumb(self) -> bool {
        matches!(self, Self::LeftThumb)
    }
}

/// classify() の結果。キー分類と物理位置を一度に計算する。
#[derive(Debug, Clone, Copy)]
pub struct ClassifiedEvent {
    pub key_class: KeyClass,
    /// 物理位置（Char キーの場合のみ Some）
    pub pos: Option<PhysicalPos>,
    /// 元のイベントデータ（プラットフォーム固有、Engine は直接検査しない）
    pub scan_code: ScanCode,
    pub vk_code: VkCode,
    pub timestamp: Timestamp,
    /// IME 制御キーか（保留フラッシュ判定用、プラットフォーム層が事前分類）
    pub is_ime_control: bool,
}

impl ClassifiedEvent {
    /// タイマー用ダミーイベント（イベントなしスナップショット構築に使う）
    #[must_use]
    pub const fn dummy() -> Self {
        Self {
            key_class: KeyClass::Passthrough,
            pos: None,
            scan_code: ScanCode(0),
            vk_code: VkCode(0),
            timestamp: 0,
            is_ime_control: false,
        }
    }
}

/// 配列の面を表す列挙型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    Normal,
    LeftThumb,
    RightThumb,
    Shift,
}

impl Face {
    /// KeyClass の親指キーから対応する Face を取得
    #[must_use]
    pub const fn from_thumb(key_class: KeyClass) -> Self {
        match key_class {
            KeyClass::LeftThumb => Self::LeftThumb,
            KeyClass::RightThumb => Self::RightThumb,
            _ => Self::Normal, // fallback
        }
    }

    #[must_use]
    pub const fn from_thumb_bool(is_left: bool) -> Self {
        if is_left {
            Self::LeftThumb
        } else {
            Self::RightThumb
        }
    }
}

/// resolve_* メソッドの戻り値：アクション列と出力履歴の更新指示
#[derive(Debug)]
pub struct ResolvedAction {
    pub actions: SmallVec<[KeyAction; 2]>,
    pub output: OutputUpdate,
}

impl ResolvedAction {
    /// `ParseAction::ReduceAndContinue` に変換する。
    #[must_use]
    pub fn into_reduce_and_continue(self, remaining: ClassifiedEvent) -> ParseAction {
        ParseAction::ReduceAndContinue {
            actions: self.actions,
            record: self.output,
            remaining,
        }
    }
}

/// パーサーアクション: FSM の1ステップの判断結果。
///
/// `timed_fsm::ParseAction` と同構造だが、タイマー指示に `TimerIntent` を使用する。
/// `ShiftReduceParser::decide()` 実装で `timed_fsm::ParseAction` に変換される。
#[derive(Debug)]
pub enum ParseAction {
    /// トークンをバッファして追加入力を待つ。
    Shift { timer: TimerIntent },
    /// パターンを認識して出力を生成する。
    Reduce {
        actions: SmallVec<[KeyAction; 2]>,
        record: OutputUpdate,
        timer: TimerIntent,
    },
    /// パターンを部分認識し、出力を生成してから残りのトークンを再処理する。
    ReduceAndContinue {
        actions: SmallVec<[KeyAction; 2]>,
        record: OutputUpdate,
        remaining: ClassifiedEvent,
    },
    /// このパーサーでは処理しない。次のハンドラにパススルーする。
    PassThrough { timer: TimerIntent },
}

/// タイマー操作の指示
#[derive(Debug, Clone, Copy)]
pub enum TimerIntent {
    /// 全タイマー停止（確定完了、Idle へ）
    CancelAll,
    /// TIMER_PENDING を threshold_us で起動
    Pending,
    /// TIMER_SPECULATIVE を speculative_delay_us で起動
    SpeculativeWait,
    /// TIMER_SPECULATIVE 停止 + TIMER_PENDING を残り時間で起動
    Phase2Transition { remaining_us: u64 },
    /// タイマー変更なし
    Keep,
}

impl TimerIntent {
    /// `TimerIntent` を `Vec<TimerCommand<usize>>` に変換する。
    ///
    /// `threshold_us` と `speculative_delay_us` は `NicolaFsm` から渡される。
    #[must_use]
    pub fn to_commands(
        self,
        threshold_us: u64,
        speculative_delay_us: u64,
    ) -> Vec<timed_fsm::TimerCommand<usize>> {
        use timed_fsm::TimerCommand::{Kill, Set};
        match self {
            Self::CancelAll => vec![
                Kill { id: TIMER_PENDING },
                Kill { id: TIMER_SPECULATIVE },
            ],
            Self::Pending => vec![
                Kill { id: TIMER_PENDING },
                Kill { id: TIMER_SPECULATIVE },
                Set { id: TIMER_PENDING, duration: Duration::from_micros(threshold_us) },
            ],
            Self::SpeculativeWait => vec![
                Kill { id: TIMER_PENDING },
                Kill { id: TIMER_SPECULATIVE },
                Set { id: TIMER_SPECULATIVE, duration: Duration::from_micros(speculative_delay_us) },
            ],
            Self::Phase2Transition { remaining_us } => vec![
                Kill { id: TIMER_SPECULATIVE },
                Set { id: TIMER_PENDING, duration: Duration::from_micros(remaining_us) },
            ],
            Self::Keep => vec![],
        }
    }
}

/// Idle 状態でのキー到着時の意図分類。
///
/// `decide_idle()` の前段で `classify_idle_intent()` が返す。
/// 各 variant に応じて適切な処理メソッドにディスパッチされる。
#[derive(Debug, Clone, Copy)]
pub enum IdleIntent {
    /// Shift 面で即時確定する（物理 Shift キー押下中）。
    ShiftPlane,
    /// 未消費の親指キーが押下中で、親指面で即時確定する。
    ActiveThumb(Face),
    /// 配列定義に含まれないキー → OS にパススルー。
    PassThrough,
    /// 確定モードに基づいて保留/投機/即時確定を選択する。
    ConfirmMode,
}

/// 出力履歴の更新指示。
#[derive(Debug, Clone)]
pub enum OutputUpdate {
    /// 出力を記録する。
    Record(crate::engine::output_history::OutputEntry),
    /// 最後の出力を取り消して新しい出力を記録する。
    ///
    /// `retract_and_replace()` が使用する: BACKSPACE + 新文字を出力しつつ、
    /// 履歴を retract + record として `update_history()` でアトミックに更新する。
    RetractAndRecord(crate::engine::output_history::OutputEntry),
    /// 変更なし。
    None,
}

impl OutputUpdate {
    /// `Record` バリアントを構築するコンストラクタ。
    #[must_use]
    pub fn record(scan_code: ScanCode, action: &KeyAction, kana: Option<char>) -> Self {
        Self::Record(crate::engine::output_history::OutputEntry {
            scan_code,
            romaji: action.romaji().to_owned(),
            kana,
            action: action.clone(),
        })
    }
}

/// on_key_down の前段でエンジン処理をバイパスする理由
#[derive(Debug, Clone, Copy)]
pub enum BypassReason {
    /// 修飾キー、ファンクションキー等（変換対象外）
    Passthrough,
    /// IME 制御キー（半角/全角、カタカナ/ひらがな等）
    ImeControl,
    /// OS 予約ショートカット（Ctrl/Alt が押下中）
    OsModifierHeld,
}

/// エンジンの状態（データ付き enum で不正な状態をコンパイル時に排除）
#[derive(Debug, Clone, Copy)]
pub enum EngineState {
    Idle,
    PendingChar(PendingKey),
    PendingThumb(PendingThumbData),
    /// 文字キー → 親指キーの順に到着し、3 鍵目（char2）を待機中
    PendingCharThumb {
        char_key: PendingKey,
        thumb: PendingThumbData,
        /// char1 が KeyUp で離されたかどうか
        /// true の場合、char2 到着時に必ず PairWithChar2（char1 単独 + char2+thumb 同時）を選択する
        char1_released: bool,
    },
    /// 投機出力済み: 通常面の文字を出力したが、同時打鍵で差し替えられる可能性がある
    SpeculativeChar(PendingKey),
}

macro_rules! impl_expect {
    ($fn_name:ident, $variant:ident, $ty:ty) => {
        #[track_caller]
        #[must_use]
        pub fn $fn_name(self) -> $ty {
            if let Self::$variant(x) = self {
                x
            } else {
                unreachable!(
                    concat!("FSM invariant violation: expected ", stringify!($variant), ", got {:?}"),
                    self
                )
            }
        }
    };
}

impl EngineState {
    /// 状態が Idle かどうか
    #[must_use]
    pub const fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// 診断用: 状態を短い文字列で返す（[engine-input] ログ等で使用）。
    #[must_use]
    pub fn debug_label(&self) -> String {
        match self {
            Self::Idle => "Idle".to_string(),
            Self::PendingChar(k) => format!("PendingChar(vk=0x{:02X})", k.vk_code.0),
            Self::PendingThumb(t) => {
                format!("PendingThumb(vk=0x{:02X},left={})", t.vk_code.0, t.is_left)
            }
            Self::PendingCharThumb {
                char_key,
                thumb,
                char1_released,
            } => format!(
                "PendingCharThumb(char=0x{:02X},thumb=0x{:02X},left={},released={})",
                char_key.vk_code.0, thumb.vk_code.0, thumb.is_left, char1_released
            ),
            Self::SpeculativeChar(k) => format!("SpeculativeChar(vk=0x{:02X})", k.vk_code.0),
        }
    }

    impl_expect!(expect_pending_char, PendingChar, PendingKey);
    impl_expect!(expect_pending_thumb, PendingThumb, PendingThumbData);
    impl_expect!(expect_speculative_char, SpeculativeChar, PendingKey);

    /// `PendingCharThumb` の内容を取り出す。他の状態ならパニック。
    #[track_caller]
    #[must_use]
    pub fn expect_pending_char_thumb(self) -> (PendingKey, PendingThumbData, bool) {
        if let Self::PendingCharThumb { char_key, thumb, char1_released } = self {
            (char_key, thumb, char1_released)
        } else {
            unreachable!("FSM invariant violation: expected PendingCharThumb, got {self:?}")
        }
    }
}

/// 保留中の文字キーデータ
#[derive(Debug, Clone, Copy)]
pub struct PendingKey {
    pub scan_code: ScanCode,
    pub vk_code: VkCode,
    pub pos: Option<PhysicalPos>,
    pub timestamp: Timestamp,
}

impl PendingKey {
    #[must_use]
    pub const fn from_event(ev: &ClassifiedEvent) -> Self {
        Self {
            scan_code: ev.scan_code,
            vk_code: ev.vk_code,
            pos: ev.pos,
            timestamp: ev.timestamp,
        }
    }
}

/// 保留中の親指キーデータ
#[derive(Debug, Clone, Copy)]
pub struct PendingThumbData {
    pub scan_code: ScanCode,
    pub vk_code: VkCode,
    pub is_left: bool,
    pub timestamp: Timestamp,
}

impl PendingThumbData {
    #[must_use]
    pub const fn from_event(ev: &ClassifiedEvent) -> Self {
        Self {
            scan_code: ev.scan_code,
            vk_code: ev.vk_code,
            is_left: ev.key_class.is_left_thumb(),
            timestamp: ev.timestamp,
        }
    }

    /// この親指キーに対応する `Face` を返す。
    #[must_use]
    pub const fn face(self) -> Face {
        Face::from_thumb_bool(self.is_left)
    }
}

// ModifierState は crate::types::ModifierState として定義済み（上の use で import）
pub use crate::types::ModifierState;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanmap::PhysicalPos;
    use crate::types::{KeyEventType, ModifierKey, RawKeyEvent, ScanCode, VkCode};

    // ── ヘルパー ──────────────────────────────────────────────

    fn make_raw_key_event(
        event_type: KeyEventType,
        modifier_key: Option<ModifierKey>,
    ) -> RawKeyEvent {
        RawKeyEvent {
            vk_code: VkCode(0x41),
            scan_code: ScanCode(0x1E),
            event_type,
            extra_info: 0,
            timestamp: 1000,
            key_classification: crate::types::KeyClassification::Char,
            physical_pos: None,
            ime_relevance: crate::types::ImeRelevance::default(),
            modifier_key,
            modifier_snapshot: Default::default(),
            injected: false,
        }
    }

    // ── KeyClass ──────────────────────────────────────────────

    #[test]
    fn key_class_is_thumb_char() {
        assert!(!KeyClass::Char.is_thumb());
    }

    #[test]
    fn key_class_is_thumb_left_thumb() {
        assert!(KeyClass::LeftThumb.is_thumb());
    }

    #[test]
    fn key_class_is_thumb_right_thumb() {
        assert!(KeyClass::RightThumb.is_thumb());
    }

    #[test]
    fn key_class_is_thumb_passthrough() {
        assert!(!KeyClass::Passthrough.is_thumb());
    }

    #[test]
    fn key_class_is_left_thumb_only_left() {
        assert!(KeyClass::LeftThumb.is_left_thumb());
        assert!(!KeyClass::RightThumb.is_left_thumb());
        assert!(!KeyClass::Char.is_left_thumb());
        assert!(!KeyClass::Passthrough.is_left_thumb());
    }

    #[test]
    fn key_class_equality() {
        assert_eq!(KeyClass::Char, KeyClass::Char);
        assert_eq!(KeyClass::LeftThumb, KeyClass::LeftThumb);
        assert_eq!(KeyClass::RightThumb, KeyClass::RightThumb);
        assert_eq!(KeyClass::Passthrough, KeyClass::Passthrough);
        assert_ne!(KeyClass::Char, KeyClass::LeftThumb);
        assert_ne!(KeyClass::LeftThumb, KeyClass::RightThumb);
    }

    // ── Face ──────────────────────────────────────────────────

    #[test]
    fn face_from_thumb_left_thumb() {
        assert_eq!(Face::from_thumb(KeyClass::LeftThumb), Face::LeftThumb);
    }

    #[test]
    fn face_from_thumb_right_thumb() {
        assert_eq!(Face::from_thumb(KeyClass::RightThumb), Face::RightThumb);
    }

    #[test]
    fn face_from_thumb_char_fallback() {
        // Char は thumb ではないが、フォールバックとして Normal が返る
        assert_eq!(Face::from_thumb(KeyClass::Char), Face::Normal);
    }

    #[test]
    fn face_from_thumb_passthrough_fallback() {
        assert_eq!(Face::from_thumb(KeyClass::Passthrough), Face::Normal);
    }

    #[test]
    fn face_from_thumb_bool_true_is_left() {
        assert_eq!(Face::from_thumb_bool(true), Face::LeftThumb);
    }

    #[test]
    fn face_from_thumb_bool_false_is_right() {
        assert_eq!(Face::from_thumb_bool(false), Face::RightThumb);
    }

    #[test]
    fn face_equality() {
        assert_eq!(Face::Normal, Face::Normal);
        assert_eq!(Face::LeftThumb, Face::LeftThumb);
        assert_eq!(Face::RightThumb, Face::RightThumb);
        assert_eq!(Face::Shift, Face::Shift);
        assert_ne!(Face::Normal, Face::LeftThumb);
        assert_ne!(Face::LeftThumb, Face::RightThumb);
        assert_ne!(Face::Normal, Face::Shift);
    }

    // ── TimerIntent::to_commands ──────────────────────────────

    fn find_set_commands(cmds: &[timed_fsm::TimerCommand<usize>]) -> Vec<(usize, Duration)> {
        cmds.iter()
            .filter_map(|c| {
                if let timed_fsm::TimerCommand::Set { id, duration } = c {
                    Some((*id, *duration))
                } else {
                    None
                }
            })
            .collect()
    }

    fn find_kill_ids(cmds: &[timed_fsm::TimerCommand<usize>]) -> Vec<usize> {
        cmds.iter()
            .filter_map(|c| {
                if let timed_fsm::TimerCommand::Kill { id } = c {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn timer_intent_cancel_all_kills_both_timers() {
        let cmds = TimerIntent::CancelAll.to_commands(50_000, 30_000);
        let kills = find_kill_ids(&cmds);
        assert!(
            kills.contains(&TIMER_PENDING),
            "TIMER_PENDING should be killed"
        );
        assert!(
            kills.contains(&TIMER_SPECULATIVE),
            "TIMER_SPECULATIVE should be killed"
        );
        assert!(
            find_set_commands(&cmds).is_empty(),
            "no Set commands expected"
        );
    }

    #[test]
    fn timer_intent_cancel_all_command_count() {
        let cmds = TimerIntent::CancelAll.to_commands(50_000, 30_000);
        assert_eq!(cmds.len(), 2);
    }

    #[test]
    fn timer_intent_pending_sets_pending_timer_with_threshold() {
        let threshold_us = 50_000u64;
        let cmds = TimerIntent::Pending.to_commands(threshold_us, 30_000);
        let sets = find_set_commands(&cmds);
        assert_eq!(sets.len(), 1);
        let (id, dur) = sets[0];
        assert_eq!(id, TIMER_PENDING);
        assert_eq!(dur, Duration::from_micros(threshold_us));
    }

    #[test]
    fn timer_intent_pending_kills_both_before_set() {
        let cmds = TimerIntent::Pending.to_commands(50_000, 30_000);
        let kills = find_kill_ids(&cmds);
        assert!(kills.contains(&TIMER_PENDING));
        assert!(kills.contains(&TIMER_SPECULATIVE));
    }

    #[test]
    fn timer_intent_pending_command_count() {
        let cmds = TimerIntent::Pending.to_commands(50_000, 30_000);
        assert_eq!(cmds.len(), 3);
    }

    #[test]
    fn timer_intent_speculative_wait_sets_speculative_timer() {
        let speculative_us = 20_000u64;
        let cmds = TimerIntent::SpeculativeWait.to_commands(50_000, speculative_us);
        let sets = find_set_commands(&cmds);
        assert_eq!(sets.len(), 1);
        let (id, dur) = sets[0];
        assert_eq!(id, TIMER_SPECULATIVE);
        assert_eq!(dur, Duration::from_micros(speculative_us));
    }

    #[test]
    fn timer_intent_speculative_wait_kills_both_before_set() {
        let cmds = TimerIntent::SpeculativeWait.to_commands(50_000, 20_000);
        let kills = find_kill_ids(&cmds);
        assert!(kills.contains(&TIMER_PENDING));
        assert!(kills.contains(&TIMER_SPECULATIVE));
    }

    #[test]
    fn timer_intent_speculative_wait_command_count() {
        let cmds = TimerIntent::SpeculativeWait.to_commands(50_000, 20_000);
        assert_eq!(cmds.len(), 3);
    }

    #[test]
    fn timer_intent_phase2_transition_kills_speculative_and_sets_pending() {
        let remaining_us = 12_345u64;
        let cmds = TimerIntent::Phase2Transition { remaining_us }.to_commands(50_000, 20_000);
        let kills = find_kill_ids(&cmds);
        assert!(kills.contains(&TIMER_SPECULATIVE));
        assert!(
            !kills.contains(&TIMER_PENDING),
            "TIMER_PENDING should NOT be killed in Phase2"
        );
        let sets = find_set_commands(&cmds);
        assert_eq!(sets.len(), 1);
        let (id, dur) = sets[0];
        assert_eq!(id, TIMER_PENDING);
        assert_eq!(dur, Duration::from_micros(remaining_us));
    }

    #[test]
    fn timer_intent_phase2_transition_command_count() {
        let cmds = TimerIntent::Phase2Transition {
            remaining_us: 10_000,
        }
        .to_commands(50_000, 20_000);
        assert_eq!(cmds.len(), 2);
    }

    #[test]
    fn timer_intent_phase2_transition_zero_remaining() {
        let cmds = TimerIntent::Phase2Transition { remaining_us: 0 }.to_commands(50_000, 20_000);
        let sets = find_set_commands(&cmds);
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].1, Duration::from_micros(0));
    }

    #[test]
    fn timer_intent_keep_returns_empty() {
        let cmds = TimerIntent::Keep.to_commands(50_000, 20_000);
        assert!(cmds.is_empty());
    }

    #[test]
    fn timer_intent_keep_ignores_parameters() {
        // パラメータの値に関わらず空を返す
        let cmds1 = TimerIntent::Keep.to_commands(0, 0);
        let cmds2 = TimerIntent::Keep.to_commands(u64::MAX, u64::MAX);
        assert!(cmds1.is_empty());
        assert!(cmds2.is_empty());
    }

    // ── EngineState ───────────────────────────────────────────

    fn make_pending_key() -> PendingKey {
        PendingKey {
            scan_code: ScanCode(0x1E),
            vk_code: VkCode(0x41),
            pos: Some(PhysicalPos { row: 1, col: 2 }),
            timestamp: 1000,
        }
    }

    fn make_pending_thumb_data(is_left: bool) -> PendingThumbData {
        PendingThumbData {
            scan_code: ScanCode(0x39),
            vk_code: VkCode(0x20),
            is_left,
            timestamp: 2000,
        }
    }

    #[test]
    fn engine_state_idle_is_idle() {
        assert!(EngineState::Idle.is_idle());
    }

    #[test]
    fn engine_state_pending_char_is_not_idle() {
        assert!(!EngineState::PendingChar(make_pending_key()).is_idle());
    }

    #[test]
    fn engine_state_pending_thumb_is_not_idle() {
        assert!(!EngineState::PendingThumb(make_pending_thumb_data(true)).is_idle());
    }

    #[test]
    fn engine_state_pending_char_thumb_is_not_idle() {
        let state = EngineState::PendingCharThumb {
            char_key: make_pending_key(),
            thumb: make_pending_thumb_data(false),
            char1_released: false,
        };
        assert!(!state.is_idle());
    }

    #[test]
    fn engine_state_speculative_char_is_not_idle() {
        assert!(!EngineState::SpeculativeChar(make_pending_key()).is_idle());
    }

    // ── ModifierState ─────────────────────────────────────────

    #[test]
    fn modifier_state_default_all_false() {
        let ms = ModifierState::default();
        assert!(!ms.ctrl);
        assert!(!ms.alt);
        assert!(!ms.shift);
        assert!(!ms.win);
    }

    #[test]
    fn modifier_state_is_os_modifier_held_none_held() {
        let ms = ModifierState {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
        };
        assert!(!ms.is_os_modifier_held());
    }

    #[test]
    fn modifier_state_is_os_modifier_held_shift_only_is_false() {
        // Shift alone does NOT count as an OS modifier
        let ms = ModifierState {
            ctrl: false,
            alt: false,
            shift: true,
            win: false,
        };
        assert!(!ms.is_os_modifier_held());
    }

    #[test]
    fn modifier_state_is_os_modifier_held_ctrl() {
        let ms = ModifierState {
            ctrl: true,
            alt: false,
            shift: false,
            win: false,
        };
        assert!(ms.is_os_modifier_held());
    }

    #[test]
    fn modifier_state_is_os_modifier_held_alt() {
        let ms = ModifierState {
            ctrl: false,
            alt: true,
            shift: false,
            win: false,
        };
        assert!(ms.is_os_modifier_held());
    }

    #[test]
    fn modifier_state_is_os_modifier_held_win() {
        let ms = ModifierState {
            ctrl: false,
            alt: false,
            shift: false,
            win: true,
        };
        assert!(ms.is_os_modifier_held());
    }

    #[test]
    fn modifier_state_is_os_modifier_held_all_held() {
        let ms = ModifierState {
            ctrl: true,
            alt: true,
            shift: true,
            win: true,
        };
        assert!(ms.is_os_modifier_held());
    }

    #[test]
    fn modifier_state_update_ctrl_down() {
        let mut ms = ModifierState::default();
        let ev = make_raw_key_event(KeyEventType::KeyDown, Some(ModifierKey::Ctrl));
        ms.update(&ev);
        assert!(ms.ctrl);
        assert!(!ms.alt);
        assert!(!ms.shift);
        assert!(!ms.win);
    }

    #[test]
    fn modifier_state_update_ctrl_up() {
        let mut ms = ModifierState {
            ctrl: true,
            alt: false,
            shift: false,
            win: false,
        };
        let ev = make_raw_key_event(KeyEventType::KeyUp, Some(ModifierKey::Ctrl));
        ms.update(&ev);
        assert!(!ms.ctrl);
    }

    #[test]
    fn modifier_state_update_alt_down() {
        let mut ms = ModifierState::default();
        let ev = make_raw_key_event(KeyEventType::KeyDown, Some(ModifierKey::Alt));
        ms.update(&ev);
        assert!(ms.alt);
    }

    #[test]
    fn modifier_state_update_shift_down() {
        let mut ms = ModifierState::default();
        let ev = make_raw_key_event(KeyEventType::KeyDown, Some(ModifierKey::Shift));
        ms.update(&ev);
        assert!(ms.shift);
    }

    #[test]
    fn modifier_state_update_meta_down() {
        let mut ms = ModifierState::default();
        let ev = make_raw_key_event(KeyEventType::KeyDown, Some(ModifierKey::Meta));
        ms.update(&ev);
        assert!(ms.win);
    }

    #[test]
    fn modifier_state_update_non_modifier_key_no_change() {
        let mut ms = ModifierState {
            ctrl: true,
            alt: true,
            shift: true,
            win: true,
        };
        let ev = make_raw_key_event(KeyEventType::KeyDown, None);
        ms.update(&ev);
        // None の modifier_key では何も変化しない
        assert!(ms.ctrl);
        assert!(ms.alt);
        assert!(ms.shift);
        assert!(ms.win);
    }

    #[test]
    fn modifier_state_update_shift_up_only_clears_shift() {
        let mut ms = ModifierState {
            ctrl: true,
            alt: true,
            shift: true,
            win: true,
        };
        let ev = make_raw_key_event(KeyEventType::KeyUp, Some(ModifierKey::Shift));
        ms.update(&ev);
        assert!(ms.ctrl);
        assert!(ms.alt);
        assert!(!ms.shift);
        assert!(ms.win);
    }

    // ── OutputUpdate ──────────────────────────────────────────

    #[test]
    fn output_update_none_variant() {
        let u = OutputUpdate::None;
        assert!(matches!(u, OutputUpdate::None));
    }

    #[test]
    fn output_update_record_variant() {
        use crate::engine::output_history::OutputEntry;
        use crate::types::KeyAction;
        let entry = OutputEntry {
            scan_code: ScanCode(0x1E),
            romaji: "a".to_string(),
            kana: Some('あ'),
            action: KeyAction::Char('a'),
        };
        let u = OutputUpdate::Record(entry);
        assert!(matches!(u, OutputUpdate::Record(_)));
    }

    #[test]
    fn output_update_retract_and_record_variant() {
        use crate::engine::output_history::OutputEntry;
        use crate::types::KeyAction;
        let entry = OutputEntry {
            scan_code: ScanCode(0x1E),
            romaji: "ka".to_string(),
            kana: Some('か'),
            action: KeyAction::Romaji("ka".to_string()),
        };
        let u = OutputUpdate::RetractAndRecord(entry);
        assert!(matches!(u, OutputUpdate::RetractAndRecord(_)));
    }

    // ── PendingKey ────────────────────────────────────────────

    #[test]
    fn pending_key_with_pos() {
        let pk = make_pending_key();
        assert_eq!(pk.scan_code, ScanCode(0x1E));
        assert_eq!(pk.vk_code, VkCode(0x41));
        assert!(pk.pos.is_some());
        assert_eq!(pk.timestamp, 1000);
    }

    #[test]
    fn pending_key_without_pos() {
        let pk = PendingKey {
            scan_code: ScanCode(0x01),
            vk_code: VkCode(0x10),
            pos: None,
            timestamp: 500,
        };
        assert!(pk.pos.is_none());
    }

    // ── PendingThumbData ──────────────────────────────────────

    #[test]
    fn pending_thumb_data_left() {
        let td = make_pending_thumb_data(true);
        assert!(td.is_left);
        assert_eq!(td.vk_code, VkCode(0x20));
        assert_eq!(td.timestamp, 2000);
    }

    #[test]
    fn pending_thumb_data_right() {
        let td = make_pending_thumb_data(false);
        assert!(!td.is_left);
    }

    // ── ClassifiedEvent ───────────────────────────────────────

    #[test]
    fn classified_event_char_with_pos() {
        let ev = ClassifiedEvent {
            key_class: KeyClass::Char,
            pos: Some(PhysicalPos { row: 0, col: 3 }),
            scan_code: ScanCode(0x20),
            vk_code: VkCode(0x48),
            timestamp: 3000,
            is_ime_control: false,
        };
        assert_eq!(ev.key_class, KeyClass::Char);
        assert!(ev.pos.is_some());
        assert!(!ev.is_ime_control);
    }

    #[test]
    fn classified_event_thumb_no_pos() {
        let ev = ClassifiedEvent {
            key_class: KeyClass::LeftThumb,
            pos: None,
            scan_code: ScanCode(0x39),
            vk_code: VkCode(0x20),
            timestamp: 4000,
            is_ime_control: false,
        };
        assert!(ev.key_class.is_thumb());
        assert!(ev.pos.is_none());
    }

    #[test]
    fn classified_event_ime_control_flag() {
        let ev = ClassifiedEvent {
            key_class: KeyClass::Passthrough,
            pos: None,
            scan_code: ScanCode(0x70),
            vk_code: VkCode(0xF3),
            timestamp: 5000,
            is_ime_control: true,
        };
        assert!(ev.is_ime_control);
    }

    // ── IdleIntent ────────────────────────────────────────────

    #[test]
    fn idle_intent_active_thumb_carries_face() {
        let intent = IdleIntent::ActiveThumb(Face::LeftThumb);
        if let IdleIntent::ActiveThumb(face) = intent {
            assert_eq!(face, Face::LeftThumb);
        } else {
            panic!("expected ActiveThumb");
        }
    }

    #[test]
    fn idle_intent_variants_debug() {
        // Debug impl が存在することを確認
        let _ = format!("{:?}", IdleIntent::ShiftPlane);
        let _ = format!("{:?}", IdleIntent::ActiveThumb(Face::RightThumb));
        let _ = format!("{:?}", IdleIntent::PassThrough);
        let _ = format!("{:?}", IdleIntent::ConfirmMode);
    }

    // ── BypassReason ──────────────────────────────────────────

    #[test]
    fn bypass_reason_variants_debug() {
        let _ = format!("{:?}", BypassReason::Passthrough);
        let _ = format!("{:?}", BypassReason::ImeControl);
        let _ = format!("{:?}", BypassReason::OsModifierHeld);
    }

    // ── ResolvedAction ────────────────────────────────────────

    #[test]
    fn resolved_action_empty_actions() {
        let ra = ResolvedAction {
            actions: smallvec::smallvec![],
            output: OutputUpdate::None,
        };
        assert!(ra.actions.is_empty());
    }

    #[test]
    fn resolved_action_with_actions() {
        use crate::types::KeyAction;
        let ra = ResolvedAction {
            actions: smallvec::smallvec![KeyAction::Char('a'), KeyAction::Suppress],
            output: OutputUpdate::None,
        };
        assert_eq!(ra.actions.len(), 2);
    }

    // ── ParseAction ───────────────────────────────────────────

    #[test]
    fn parse_action_shift_variant() {
        let pa = ParseAction::Shift {
            timer: TimerIntent::Keep,
        };
        assert!(matches!(pa, ParseAction::Shift { .. }));
    }

    #[test]
    fn parse_action_reduce_variant() {
        use crate::types::KeyAction;
        let pa = ParseAction::Reduce {
            actions: smallvec::smallvec![KeyAction::Char('b')],
            record: OutputUpdate::None,
            timer: TimerIntent::CancelAll,
        };
        assert!(matches!(pa, ParseAction::Reduce { .. }));
    }

    #[test]
    fn parse_action_pass_through_variant() {
        let pa = ParseAction::PassThrough {
            timer: TimerIntent::Keep,
        };
        assert!(matches!(pa, ParseAction::PassThrough { .. }));
    }

    #[test]
    fn parse_action_reduce_and_continue_variant() {
        use crate::types::KeyAction;
        let remaining = ClassifiedEvent {
            key_class: KeyClass::Char,
            pos: None,
            scan_code: ScanCode(1),
            vk_code: VkCode(1),
            timestamp: 0,
            is_ime_control: false,
        };
        let pa = ParseAction::ReduceAndContinue {
            actions: smallvec::smallvec![KeyAction::Suppress],
            record: OutputUpdate::None,
            remaining,
        };
        assert!(matches!(pa, ParseAction::ReduceAndContinue { .. }));
    }
}
