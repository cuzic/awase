//! FSM 内部で使用する型定義

use std::time::Duration;

use crate::scanmap::PhysicalPos;
use crate::types::{KeyAction, KeyEventType, RawKeyEvent, ScanCode, Timestamp, VkCode};

use super::nicola_fsm::{TIMER_PENDING, TIMER_SPECULATIVE};

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
    /// 元のイベントデータ
    pub scan_code: ScanCode,
    pub vk_code: VkCode,
    pub timestamp: Timestamp,
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
    pub actions: Vec<KeyAction>,
    pub output: OutputUpdate,
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
        actions: Vec<KeyAction>,
        record: OutputUpdate,
        timer: TimerIntent,
    },
    /// パターンを部分認識し、出力を生成してから残りのトークンを再処理する。
    ReduceAndContinue {
        actions: Vec<KeyAction>,
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
        match self {
            Self::CancelAll => vec![
                timed_fsm::TimerCommand::Kill { id: TIMER_PENDING },
                timed_fsm::TimerCommand::Kill {
                    id: TIMER_SPECULATIVE,
                },
            ],
            Self::Pending => vec![
                timed_fsm::TimerCommand::Kill { id: TIMER_PENDING },
                timed_fsm::TimerCommand::Kill {
                    id: TIMER_SPECULATIVE,
                },
                timed_fsm::TimerCommand::Set {
                    id: TIMER_PENDING,
                    duration: Duration::from_micros(threshold_us),
                },
            ],
            Self::SpeculativeWait => vec![
                timed_fsm::TimerCommand::Kill { id: TIMER_PENDING },
                timed_fsm::TimerCommand::Kill {
                    id: TIMER_SPECULATIVE,
                },
                timed_fsm::TimerCommand::Set {
                    id: TIMER_SPECULATIVE,
                    duration: Duration::from_micros(speculative_delay_us),
                },
            ],
            Self::Phase2Transition { remaining_us } => vec![
                timed_fsm::TimerCommand::Kill {
                    id: TIMER_SPECULATIVE,
                },
                timed_fsm::TimerCommand::Set {
                    id: TIMER_PENDING,
                    duration: Duration::from_micros(remaining_us),
                },
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

/// 出力履歴に記録する 1 件分のデータ
#[derive(Debug, Clone)]
pub struct OutputRecord {
    pub scan_code: ScanCode,
    pub romaji: String,
    pub kana: Option<char>,
    pub action: KeyAction,
}

/// 出力履歴の更新指示。
///
/// 投機出力の取り消し（speculative retract）では、`step_speculative_thumb()` が
/// `output_history.retract_last()` を直接呼んだ後に `Record` を使用する。
/// `RetractAndRecord` は将来の拡張用に残しているが、現在の投機取り消しパスでは
/// 使用されていない（retract と record が分離されているため）。
#[derive(Debug, Clone)]
pub enum OutputUpdate {
    /// 出力を記録する。
    Record(OutputRecord),
    /// 最後の出力を取り消して新しい出力を記録する。
    ///
    /// 注意: 投機出力の取り消しでは `step_speculative_thumb()` が
    /// `retract_last()` + `Record` の 2 段階で処理するため、この variant は
    /// そのパスでは使用されない。
    RetractAndRecord(OutputRecord),
    /// 変更なし。
    None,
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
    },
    /// 投機出力済み: 通常面の文字を出力したが、同時打鍵で差し替えられる可能性がある
    SpeculativeChar(PendingKey),
}

impl EngineState {
    /// 状態が Idle かどうか
    #[must_use]
    pub const fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }
}

/// 保留中の文字キーデータ
#[derive(Debug, Clone, Copy)]
pub struct PendingKey {
    pub scan_code: ScanCode,
    pub vk_code: VkCode,
    pub timestamp: Timestamp,
}

/// 保留中の親指キーデータ
#[derive(Debug, Clone, Copy)]
pub struct PendingThumbData {
    #[allow(dead_code)] // KeyUp 追跡の将来拡張用
    pub scan_code: ScanCode,
    pub vk_code: VkCode,
    pub is_left: bool,
    pub timestamp: Timestamp,
}

/// 修飾キー（Ctrl / Alt / Shift / Win）の押下状態
#[derive(Debug, Default, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)] // 各修飾キーの物理状態を1:1で表現
pub struct ModifierState {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

impl ModifierState {
    /// Ctrl / Alt / Shift / Win キーの押下状態を更新する
    pub const fn update(&mut self, event: &RawKeyEvent) {
        let is_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );

        match event.vk_code.0 {
            // Ctrl (generic), LCtrl, RCtrl
            0x11 | 0xA2 | 0xA3 => self.ctrl = is_down,
            // Alt (generic), LAlt, RAlt
            0x12 | 0xA4 | 0xA5 => self.alt = is_down,
            // Shift (generic), LShift, RShift
            0x10 | 0xA0 | 0xA1 => self.shift = is_down,
            // Win (LWin, RWin)
            0x5B | 0x5C => self.win = is_down,
            _ => {}
        }
    }

    /// OS 予約キーコンビネーション用の修飾キーが押下中かどうか
    #[must_use]
    pub const fn is_os_modifier_held(self) -> bool {
        self.ctrl || self.alt || self.win
    }
}
