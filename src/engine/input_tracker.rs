//! 入力レイヤー: 物理キー状態の追跡
//!
//! キーの分類（`classify`）と修飾キー・親指キーの押下状態を管理する。
//! FSM（処理レイヤー）とは独立して動作し、IME チェック等で FSM がスキップ
//! される場合でも物理キー状態を正確に追跡し続ける。

use crate::scanmap::scan_to_pos;
use crate::types::{KeyEventType, RawKeyEvent, ScanCode, Timestamp, VkCode};
use crate::vk;

use super::types::{ClassifiedEvent, KeyClass, ModifierState};

/// 物理キー状態のスナップショット
///
/// `InputTracker::process()` が返す不変のスナップショット。
/// 処理レイヤー（Engine FSM）はこれを参照するだけで、書き換えない。
#[derive(Debug, Clone, Copy)]
pub struct PhysicalKeyState {
    /// 分類済みイベント（KeyClass, PhysicalPos 等）
    pub classified: ClassifiedEvent,
    /// このイベント適用後の修飾キー状態
    pub modifiers: ModifierState,
    /// 左親指キー押下時刻（None = 非押下）
    pub left_thumb_down: Option<Timestamp>,
    /// 右親指キー押下時刻（None = 非押下）
    pub right_thumb_down: Option<Timestamp>,
}

impl PhysicalKeyState {
    /// 中立な（何も押されていない）状態を返す。
    ///
    /// Engine の初期化時など、まだ実イベントを受け取っていない段階で使う。
    #[must_use]
    pub fn empty() -> Self {
        Self {
            classified: ClassifiedEvent {
                key_class: KeyClass::Passthrough,
                pos: None,
                scan_code: ScanCode(0),
                vk_code: VkCode(0),
                timestamp: 0,
            },
            modifiers: ModifierState::default(),
            left_thumb_down: None,
            right_thumb_down: None,
        }
    }
}

/// 入力レイヤー: 物理キー状態の追跡
///
/// 全キーイベントに対して [`process()`](Self::process) を無条件に呼ぶこと。
/// IME チェックやエンジン有効/無効に関係なく、常に正確な物理キー状態を保持する。
#[derive(Debug)]
pub struct InputTracker {
    left_thumb_vk: VkCode,
    right_thumb_vk: VkCode,
    modifiers: ModifierState,
    left_thumb_down: Option<Timestamp>,
    right_thumb_down: Option<Timestamp>,
}

impl InputTracker {
    #[must_use]
    pub fn new(left_thumb_vk: VkCode, right_thumb_vk: VkCode) -> Self {
        Self {
            left_thumb_vk,
            right_thumb_vk,
            modifiers: ModifierState::default(),
            left_thumb_down: None,
            right_thumb_down: None,
        }
    }

    /// 現在の物理キー状態スナップショットを返す（イベントなし）。
    ///
    /// タイマー発火時など、キーイベントを伴わない場面で最新の物理状態を取得する。
    /// `classified` は直前の `process()` で設定された値がそのまま残る。
    #[must_use]
    pub fn snapshot(&self) -> PhysicalKeyState {
        PhysicalKeyState {
            classified: ClassifiedEvent {
                key_class: KeyClass::Passthrough,
                pos: None,
                scan_code: ScanCode(0),
                vk_code: VkCode(0),
                timestamp: 0,
            },
            modifiers: self.modifiers,
            left_thumb_down: self.left_thumb_down,
            right_thumb_down: self.right_thumb_down,
        }
    }

    /// キーイベントを処理し、更新後の物理キー状態スナップショットを返す。
    ///
    /// **全イベントで無条件に呼ぶこと。** IME チェック前、エンジン処理前の
    /// 最も早い段階でこのメソッドを呼び出す。
    pub fn process(&mut self, event: &RawKeyEvent) -> PhysicalKeyState {
        self.modifiers.update(event);
        let classified = self.classify(event);
        self.update_thumb_state(&classified, event);
        PhysicalKeyState {
            classified,
            modifiers: self.modifiers,
            left_thumb_down: self.left_thumb_down,
            right_thumb_down: self.right_thumb_down,
        }
    }

    /// VK コードからキー分類と物理位置を決定する
    fn classify(&self, event: &RawKeyEvent) -> ClassifiedEvent {
        let key_class = if event.vk_code == self.left_thumb_vk {
            KeyClass::LeftThumb
        } else if event.vk_code == self.right_thumb_vk {
            KeyClass::RightThumb
        } else if vk::is_passthrough(event.vk_code) {
            KeyClass::Passthrough
        } else {
            KeyClass::Char
        };

        let pos = if key_class == KeyClass::Char {
            scan_to_pos(event.scan_code.0)
        } else {
            None
        };

        ClassifiedEvent {
            key_class,
            pos,
            scan_code: event.scan_code,
            vk_code: event.vk_code,
            timestamp: event.timestamp,
        }
    }

    /// 親指キーの押下/解放状態を更新する
    fn update_thumb_state(&mut self, ev: &ClassifiedEvent, event: &RawKeyEvent) {
        let is_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );
        if ev.key_class.is_left_thumb() {
            self.left_thumb_down = if is_down { Some(ev.timestamp) } else { None };
        } else if ev.key_class == KeyClass::RightThumb {
            self.right_thumb_down = if is_down { Some(ev.timestamp) } else { None };
        }
    }
}
