//! エンジン内部で使用する型定義

use crate::scanmap::PhysicalPos;
use crate::types::{KeyAction, KeyEventType, RawKeyEvent, Timestamp};

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
    pub(crate) const fn is_thumb(self) -> bool {
        matches!(self, Self::LeftThumb | Self::RightThumb)
    }

    pub(crate) const fn is_left_thumb(self) -> bool {
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
    pub scan_code: u32,
    pub vk_code: u16,
    pub timestamp: Timestamp,
}

/// 配列の面を表す列挙型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Face {
    Normal,
    LeftThumb,
    RightThumb,
    Shift,
}

impl Face {
    /// KeyClass の親指キーから対応する Face を取得
    pub(crate) const fn from_thumb(key_class: KeyClass) -> Self {
        match key_class {
            KeyClass::LeftThumb => Self::LeftThumb,
            KeyClass::RightThumb => Self::RightThumb,
            _ => Self::Normal, // fallback
        }
    }

    pub(crate) const fn from_thumb_bool(is_left: bool) -> Self {
        if is_left {
            Self::LeftThumb
        } else {
            Self::RightThumb
        }
    }
}

/// resolve_* メソッドの戻り値：アクション列と出力履歴の更新指示
#[derive(Debug)]
pub(crate) struct ResolvedAction {
    pub(crate) actions: Vec<KeyAction>,
    pub(crate) output: OutputUpdate,
}

/// Engine の唯一の出口に渡す実行計画
#[derive(Debug)]
pub struct FinalizePlan {
    /// 出力アクション（空なら consume）
    pub actions: Vec<KeyAction>,
    /// タイマー指示
    pub timer: TimerIntent,
    /// 出力履歴の更新
    pub output: OutputUpdate,
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

/// 出力履歴に記録する 1 件分のデータ
#[derive(Debug, Clone)]
pub struct OutputRecord {
    pub scan_code: u32,
    pub romaji: String,
    pub kana: Option<char>,
    pub action: KeyAction,
}

/// 出力履歴の更新指示
#[derive(Debug, Clone)]
pub enum OutputUpdate {
    /// 出力を記録
    Record(OutputRecord),
    /// 最後の出力を取り消して新しい出力を記録
    RetractAndRecord(OutputRecord),
    /// 変更なし
    None,
}

/// on_key_down の前段でエンジン処理をバイパスする理由
#[derive(Debug, Clone, Copy)]
pub(crate) enum BypassReason {
    /// 修飾キー、ファンクションキー等（変換対象外）
    Passthrough,
    /// IME 制御キー（半角/全角、カタカナ/ひらがな等）
    ImeControl,
    /// OS 予約ショートカット（Ctrl/Alt が押下中）
    OsModifierHeld,
}

/// エンジンのフェーズ（状態タグ）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnginePhase {
    Idle,
    PendingChar,
    PendingThumb,
    /// 文字キー → 親指キーの順に到着し、3 鍵目（char2）を待機中
    PendingCharThumb,
    /// 投機出力済み: 通常面の文字を出力したが、同時打鍵で差し替えられる可能性がある
    SpeculativeChar,
}

/// 保留中の文字キーデータ
#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingKey {
    pub(crate) scan_code: u32,
    pub(crate) vk_code: u16,
    pub(crate) timestamp: Timestamp,
}

/// 保留中の親指キーデータ
#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingThumbData {
    #[allow(dead_code)] // KeyUp 追跡の将来拡張用
    pub(crate) scan_code: u32,
    pub(crate) vk_code: u16,
    pub(crate) is_left: bool,
    pub(crate) timestamp: Timestamp,
}

/// 修飾キー（Ctrl / Alt / Shift）の押下状態
#[derive(Debug, Default)]
pub(crate) struct ModifierState {
    pub(crate) ctrl: bool,
    pub(crate) alt: bool,
    pub(crate) shift: bool,
}

impl ModifierState {
    /// Ctrl / Alt / Shift キーの押下状態を更新する
    pub(crate) const fn update(&mut self, event: &RawKeyEvent) {
        let is_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );

        match event.vk_code {
            // Ctrl (generic), LCtrl, RCtrl
            0x11 | 0xA2 | 0xA3 => self.ctrl = is_down,
            // Alt (generic), LAlt, RAlt
            0x12 | 0xA4 | 0xA5 => self.alt = is_down,
            // Shift (generic), LShift, RShift
            0x10 | 0xA0 | 0xA1 => self.shift = is_down,
            _ => {}
        }
    }

    /// OS 予約キーコンビネーション用の修飾キーが押下中かどうか
    pub(crate) const fn is_os_modifier_held(&self) -> bool {
        self.ctrl || self.alt
    }
}
