//! 入力レイヤー: 物理キー状態の追跡
//!
//! キーの分類（`classify`）と修飾キー・親指キーの押下状態を管理する。
//! FSM（処理レイヤー）とは独立して動作し、IME チェック等で FSM がスキップ
//! される場合でも物理キー状態を正確に追跡し続ける。

use crate::types::{KeyClassification, KeyEventType, RawKeyEvent, ScanCode, Timestamp, VkCode};

use super::decision::InputContext;
use super::fsm_types::{ClassifiedEvent, KeyClass, ModifierState};

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
                is_ime_control: false,
            },
            modifiers: ModifierState::default(),
            left_thumb_down: None,
            right_thumb_down: None,
        }
    }

    /// InputContext と RawKeyEvent から PhysicalKeyState を構築する。
    ///
    /// Platform 層の InputTracker が物理キー状態を追跡済みなので、
    /// Engine は InputContext からスナップショットを構築するだけでよい。
    #[must_use]
    pub fn from_ctx(ctx: &InputContext, event: &RawKeyEvent) -> Self {
        Self {
            classified: InputTracker::classify(event),
            modifiers: ctx.modifiers,
            left_thumb_down: ctx.left_thumb_down,
            right_thumb_down: ctx.right_thumb_down,
        }
    }

    /// InputContext からタイマー用スナップショットを構築する（イベントなし）。
    #[must_use]
    pub fn from_ctx_snapshot(ctx: &InputContext) -> Self {
        Self {
            classified: ClassifiedEvent {
                key_class: KeyClass::Passthrough,
                pos: None,
                scan_code: ScanCode(0),
                vk_code: VkCode(0),
                timestamp: 0,
                is_ime_control: false,
            },
            modifiers: ctx.modifiers,
            left_thumb_down: ctx.left_thumb_down,
            right_thumb_down: ctx.right_thumb_down,
        }
    }
}

/// 入力レイヤー: 物理キー状態の追跡
///
/// 全キーイベントに対して [`process()`](Self::process) を無条件に呼ぶこと。
/// IME チェックやエンジン有効/無効に関係なく、常に正確な物理キー状態を保持する。
#[derive(Debug)]
pub struct InputTracker {
    modifiers: ModifierState,
    left_thumb_down: Option<Timestamp>,
    right_thumb_down: Option<Timestamp>,
}

impl Default for InputTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl InputTracker {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            modifiers: ModifierState {
                ctrl: false,
                alt: false,
                shift: false,
                win: false,
            },
            left_thumb_down: None,
            right_thumb_down: None,
        }
    }

    /// 現在の修飾キー状態を返す。
    #[must_use]
    pub const fn modifiers(&self) -> ModifierState {
        self.modifiers
    }

    /// OS から取得した修飾キー状態で上書きする。
    ///
    /// フォーカス変更・セッション復帰等で、フックが取りこぼした
    /// 修飾キーの押下/解放を補正するために使う。
    pub const fn set_modifiers(&mut self, mods: ModifierState) {
        self.modifiers = mods;
    }

    /// 現在の物理キー状態スナップショットを返す（イベントなし）。
    ///
    /// タイマー発火時など、キーイベントを伴わない場面で最新の物理状態を取得する。
    /// `classified` は直前の `process()` で設定された値がそのまま残る。
    #[must_use]
    pub const fn snapshot(&self) -> PhysicalKeyState {
        PhysicalKeyState {
            classified: ClassifiedEvent {
                key_class: KeyClass::Passthrough,
                pos: None,
                scan_code: ScanCode(0),
                vk_code: VkCode(0),
                timestamp: 0,
                is_ime_control: false,
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
        let classified = Self::classify(event);
        self.update_thumb_state(&classified, event);
        PhysicalKeyState {
            classified,
            modifiers: self.modifiers,
            left_thumb_down: self.left_thumb_down,
            right_thumb_down: self.right_thumb_down,
        }
    }

    /// プラットフォーム層が事前分類したキー情報から ClassifiedEvent を構築する
    pub(crate) const fn classify(event: &RawKeyEvent) -> ClassifiedEvent {
        let key_class = match event.key_classification {
            KeyClassification::Char => KeyClass::Char,
            KeyClassification::LeftThumb => KeyClass::LeftThumb,
            KeyClassification::RightThumb => KeyClass::RightThumb,
            KeyClassification::Passthrough => KeyClass::Passthrough,
        };

        ClassifiedEvent {
            key_class,
            pos: event.physical_pos,
            scan_code: event.scan_code,
            vk_code: event.vk_code,
            timestamp: event.timestamp,
            is_ime_control: event.ime_relevance.is_ime_control,
        }
    }

    /// 親指キーの押下/解放状態を更新する
    fn update_thumb_state(&mut self, ev: &ClassifiedEvent, event: &RawKeyEvent) {
        let is_down = matches!(event.event_type, KeyEventType::KeyDown);
        if ev.key_class.is_left_thumb() {
            self.left_thumb_down = if is_down { Some(ev.timestamp) } else { None };
        } else if ev.key_class == KeyClass::RightThumb {
            self.right_thumb_down = if is_down { Some(ev.timestamp) } else { None };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ImeRelevance, KeyClassification, ModifierKey};

    fn make_event(event_type: KeyEventType) -> RawKeyEvent {
        RawKeyEvent {
            vk_code: VkCode(0),
            scan_code: ScanCode(0),
            event_type,
            extra_info: 0,
            timestamp: 0,
            key_classification: KeyClassification::Passthrough,
            physical_pos: None,
            ime_relevance: ImeRelevance::default(),
            modifier_key: None,
        }
    }

    #[test]
    fn new_creates_default_state() {
        let tracker = InputTracker::new();
        let mods = tracker.modifiers();
        assert!(!mods.ctrl);
        assert!(!mods.alt);
        assert!(!mods.shift);
        assert!(!mods.win);
    }

    #[test]
    fn modifiers_returns_default_all_false() {
        let tracker = InputTracker::default();
        let mods = tracker.modifiers();
        assert!(!mods.ctrl && !mods.alt && !mods.shift && !mods.win);
    }

    #[test]
    fn set_modifiers_updates_state() {
        let mut tracker = InputTracker::new();
        let mods = ModifierState {
            ctrl: true,
            alt: false,
            shift: true,
            win: false,
        };
        tracker.set_modifiers(mods);
        let got = tracker.modifiers();
        assert!(got.ctrl);
        assert!(got.shift);
        assert!(!got.alt);
        assert!(!got.win);
    }

    #[test]
    fn process_char_key_down_returns_correct_classified_event() {
        let mut tracker = InputTracker::new();
        let mut event = make_event(KeyEventType::KeyDown);
        event.key_classification = KeyClassification::Char;
        event.vk_code = VkCode(0x41); // 'A'
        event.scan_code = ScanCode(30);
        event.timestamp = 1000;

        let phys = tracker.process(&event);
        assert_eq!(phys.classified.key_class, KeyClass::Char);
        assert_eq!(phys.classified.vk_code, VkCode(0x41));
        assert_eq!(phys.classified.scan_code, ScanCode(30));
        assert_eq!(phys.classified.timestamp, 1000);
    }

    #[test]
    fn process_modifier_key_down_updates_modifier_state() {
        let mut tracker = InputTracker::new();
        let mut event = make_event(KeyEventType::KeyDown);
        event.modifier_key = Some(ModifierKey::Ctrl);

        let phys = tracker.process(&event);
        assert!(phys.modifiers.ctrl);
        assert!(!phys.modifiers.shift);

        // Verify tracker internal state also updated
        assert!(tracker.modifiers().ctrl);
    }

    #[test]
    fn process_thumb_key_tracks_thumb_down_timestamps() {
        let mut tracker = InputTracker::new();

        // Left thumb down
        let mut event = make_event(KeyEventType::KeyDown);
        event.key_classification = KeyClassification::LeftThumb;
        event.timestamp = 500;
        let phys = tracker.process(&event);
        assert_eq!(phys.left_thumb_down, Some(500));
        assert_eq!(phys.right_thumb_down, None);

        // Right thumb down
        let mut event = make_event(KeyEventType::KeyDown);
        event.key_classification = KeyClassification::RightThumb;
        event.timestamp = 600;
        let phys = tracker.process(&event);
        assert_eq!(phys.right_thumb_down, Some(600));
        assert_eq!(phys.left_thumb_down, Some(500));

        // Left thumb up
        let mut event = make_event(KeyEventType::KeyUp);
        event.key_classification = KeyClassification::LeftThumb;
        event.timestamp = 700;
        let phys = tracker.process(&event);
        assert_eq!(phys.left_thumb_down, None);
        assert_eq!(phys.right_thumb_down, Some(600));
    }

    #[test]
    fn snapshot_returns_current_state_without_event() {
        let mut tracker = InputTracker::new();

        // Set some state first
        let mut event = make_event(KeyEventType::KeyDown);
        event.modifier_key = Some(ModifierKey::Shift);
        tracker.process(&event);

        let mut event = make_event(KeyEventType::KeyDown);
        event.key_classification = KeyClassification::LeftThumb;
        event.timestamp = 999;
        tracker.process(&event);

        let snap = tracker.snapshot();
        assert!(snap.modifiers.shift);
        assert_eq!(snap.left_thumb_down, Some(999));
        assert_eq!(snap.right_thumb_down, None);
        // snapshot classified is always Passthrough placeholder
        assert_eq!(snap.classified.key_class, KeyClass::Passthrough);
    }

    #[test]
    fn physical_key_state_empty_returns_neutral_state() {
        let empty = PhysicalKeyState::empty();
        assert_eq!(empty.classified.key_class, KeyClass::Passthrough);
        assert!(empty.classified.pos.is_none());
        assert_eq!(empty.classified.scan_code, ScanCode(0));
        assert_eq!(empty.classified.vk_code, VkCode(0));
        assert_eq!(empty.classified.timestamp, 0);
        assert!(!empty.classified.is_ime_control);
        assert!(!empty.modifiers.ctrl);
        assert!(!empty.modifiers.alt);
        assert!(!empty.modifiers.shift);
        assert!(!empty.modifiers.win);
        assert!(empty.left_thumb_down.is_none());
        assert!(empty.right_thumb_down.is_none());
    }
}
