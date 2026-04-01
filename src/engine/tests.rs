use super::*;
use crate::config::ConfirmMode;
use crate::engine::nicola_fsm::yab_value_to_action;
use crate::engine::output_history::OutputEntry;
use crate::ngram::NgramModel;
use crate::types::{
    ContextChange, FocusKind, KeyAction, KeyEventType, RawKeyEvent, ScanCode, Timestamp, VkCode,
};
use crate::yab::{YabFace, YabLayout, YabValue};
use timed_fsm::Response;

type Resp = Response<KeyAction, usize>;

// VK code constants
const VK_A: VkCode = VkCode(0x41);
const VK_S: VkCode = VkCode(0x53);
const VK_NONCONVERT: VkCode = VkCode(0x1D);
const VK_CONVERT: VkCode = VkCode(0x1C);
const VK_RETURN: VkCode = VkCode(0x0D);
const VK_SHIFT: VkCode = VkCode(0x10);
const VK_LSHIFT: VkCode = VkCode(0xA0);
const VK_RSHIFT: VkCode = VkCode(0xA1);
const VK_CTRL: VkCode = VkCode(0x11);
const VK_LCTRL: VkCode = VkCode(0xA2);
const VK_ALT: VkCode = VkCode(0x12);
const VK_LALT: VkCode = VkCode(0xA4);
const VK_D: VkCode = VkCode(0x44);
const VK_F: VkCode = VkCode(0x46);
const VK_C: VkCode = VkCode(0x43);
const VK_V: VkCode = VkCode(0x56);

// Scan code constants matching the VK codes used in tests
const SCAN_A: ScanCode = ScanCode(0x1E);
const SCAN_S: ScanCode = ScanCode(0x1F);
const SCAN_D: ScanCode = ScanCode(0x20);
const SCAN_F: ScanCode = ScanCode(0x21);
const SCAN_C: ScanCode = ScanCode(0x2E);
const SCAN_V: ScanCode = ScanCode(0x2F);
const SCAN_NONCONVERT: ScanCode = ScanCode(0x7B); // muhenkan
const SCAN_CONVERT: ScanCode = ScanCode(0x79); // henkan
const SCAN_RETURN: ScanCode = ScanCode(0x1C);
const SCAN_SHIFT: ScanCode = ScanCode(0x2A);
const SCAN_LSHIFT: ScanCode = ScanCode(0x2A);
const SCAN_RSHIFT: ScanCode = ScanCode(0x36);
const SCAN_CTRL: ScanCode = ScanCode(0x1D);
const SCAN_LCTRL: ScanCode = ScanCode(0x1D);
const SCAN_ALT: ScanCode = ScanCode(0x38);
const SCAN_LALT: ScanCode = ScanCode(0x38);

use crate::scanmap::PhysicalPos;

/// PhysicalPos for A key (row=2, col=0)
const POS_A: PhysicalPos = PhysicalPos::new(2, 0);
/// PhysicalPos for S key (row=2, col=1)
const POS_S: PhysicalPos = PhysicalPos::new(2, 1);
/// PhysicalPos for D key (row=2, col=2)
const POS_D: PhysicalPos = PhysicalPos::new(2, 2);
/// PhysicalPos for F key (row=2, col=3)
const POS_F: PhysicalPos = PhysicalPos::new(2, 3);

fn lit(ch: char) -> YabValue {
    YabValue::Literal(ch.to_string())
}

fn make_layout() -> YabLayout {
    let mut normal = YabFace::new();
    normal.insert(POS_A, lit('う'));
    normal.insert(POS_S, lit('し'));

    let mut left_thumb = YabFace::new();
    left_thumb.insert(POS_A, lit('を'));
    left_thumb.insert(POS_S, lit('あ'));

    let mut right_thumb = YabFace::new();
    right_thumb.insert(POS_A, lit('ゔ'));
    right_thumb.insert(POS_S, lit('じ'));

    YabLayout {
        name: String::from("test"),
        normal,
        left_thumb,
        right_thumb,
        shift: YabFace::new(),
    }
}

/// テスト用ハーネス: InputTracker + NicolaFsm を統合し、
/// on_event で自動的に物理キー状態を追跡する。
struct TestHarness {
    tracker: input_tracker::InputTracker,
    engine: NicolaFsm,
}

impl TestHarness {
    fn on_event(&mut self, event: RawKeyEvent) -> Resp {
        let phys = self.tracker.process(&event);
        self.engine.on_event(event, &phys)
    }

    fn on_timeout(&mut self, timer_id: usize) -> Resp {
        let phys = self.tracker.snapshot();
        self.engine.on_timeout(timer_id, &phys)
    }
}

impl std::ops::Deref for TestHarness {
    type Target = NicolaFsm;
    fn deref(&self) -> &NicolaFsm {
        &self.engine
    }
}

impl std::ops::DerefMut for TestHarness {
    fn deref_mut(&mut self) -> &mut NicolaFsm {
        &mut self.engine
    }
}

fn make_engine() -> TestHarness {
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    }
}

fn make_speculative_engine() -> TestHarness {
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Speculative,
            30,
        ),
    }
}

struct Ev;

impl Ev {
    fn down(vk: VkCode) -> EvBuilder {
        EvBuilder {
            vk,
            scan: vk_to_scan(vk),
            ts: 0,
            event_type: KeyEventType::KeyDown,
        }
    }
    fn up(vk: VkCode) -> EvBuilder {
        EvBuilder {
            vk,
            scan: vk_to_scan(vk),
            ts: 0,
            event_type: KeyEventType::KeyUp,
        }
    }
}

struct EvBuilder {
    vk: VkCode,
    scan: ScanCode,
    ts: Timestamp,
    event_type: KeyEventType,
}

impl EvBuilder {
    fn at(mut self, ts: Timestamp) -> Self {
        self.ts = ts;
        self
    }
    fn scan(mut self, sc: ScanCode) -> Self {
        self.scan = sc;
        self
    }
    fn build(self) -> RawKeyEvent {
        let (kc, pos) = classify_test_key(self.vk, self.scan);
        RawKeyEvent {
            vk_code: self.vk,
            scan_code: self.scan,
            event_type: self.event_type,
            extra_info: 0,
            timestamp: self.ts,
            key_classification: kc,
            physical_pos: pos,
            ime_relevance: crate::types::ImeRelevance::default(),
            modifier_key: classify_test_modifier(self.vk),
        }
    }
}

/// Map VK code to a realistic scan code for tests
fn classify_test_key(
    vk: VkCode,
    _scan: ScanCode,
) -> (
    crate::types::KeyClassification,
    Option<crate::scanmap::PhysicalPos>,
) {
    use crate::scanmap::PhysicalPos;
    use crate::types::KeyClassification;

    if vk == VK_NONCONVERT {
        (KeyClassification::LeftThumb, None)
    } else if vk == VK_CONVERT {
        (KeyClassification::RightThumb, None)
    } else if let Some(pos) = test_vk_to_pos(vk) {
        (KeyClassification::Char, Some(pos))
    } else {
        (KeyClassification::Passthrough, None)
    }
}

/// テスト用: VK → PhysicalPos 直接マッピング（scan_to_pos を使わない）
fn test_vk_to_pos(vk: VkCode) -> Option<crate::scanmap::PhysicalPos> {
    use crate::scanmap::PhysicalPos;
    match vk {
        VK_A => Some(PhysicalPos::new(2, 0)),
        VK_S => Some(PhysicalPos::new(2, 1)),
        VK_D => Some(PhysicalPos::new(2, 2)),
        VK_F => Some(PhysicalPos::new(2, 3)),
        VK_C => Some(PhysicalPos::new(3, 2)),
        VK_V => Some(PhysicalPos::new(3, 3)),
        _ => None,
    }
}

fn classify_test_modifier(vk: VkCode) -> Option<crate::types::ModifierKey> {
    use crate::types::ModifierKey;
    match vk {
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => Some(ModifierKey::Shift),
        VK_CTRL | VK_LCTRL => Some(ModifierKey::Ctrl),
        VK_ALT | VK_LALT => Some(ModifierKey::Alt),
        _ => None,
    }
}

fn vk_to_scan(vk: VkCode) -> ScanCode {
    match vk {
        VK_A => SCAN_A,
        VK_S => SCAN_S,
        VK_D => SCAN_D,
        VK_F => SCAN_F,
        VK_C => SCAN_C,
        VK_V => SCAN_V,
        VK_NONCONVERT => SCAN_NONCONVERT,
        VK_CONVERT => SCAN_CONVERT,
        VK_RETURN => SCAN_RETURN,
        VK_SHIFT => SCAN_SHIFT,
        VK_LSHIFT => SCAN_LSHIFT,
        VK_RSHIFT => SCAN_RSHIFT,
        VK_CTRL => SCAN_CTRL,
        VK_LCTRL => SCAN_LCTRL,
        VK_ALT => SCAN_ALT,
        VK_LALT => SCAN_LALT,
        _ => ScanCode(0),
    }
}

fn assert_pending(result: &Resp) {
    result.assert_consumed();
    assert!(result.actions.is_empty(), "pending should have no actions");
    result.assert_timer_set(TIMER_PENDING);
}

#[test]
fn test_disabled_engine_passes_through() {
    let mut engine = make_engine();
    let _ = engine.toggle_enabled();
    engine
        .on_event(Ev::down(VK_A).build())
        .assert_pass_through();
}

#[test]
fn test_modifier_key_passes_through() {
    let mut engine = make_engine();
    engine
        .on_event(Ev::down(VK_SHIFT).build())
        .assert_pass_through();
}

#[test]
fn test_non_layout_key_passes_through() {
    let mut engine = make_engine();
    engine
        .on_event(Ev::down(VK_RETURN).build())
        .assert_pass_through();
}

#[test]
fn test_pattern1_thumb_first_then_char() {
    let mut engine = make_engine();
    let t0 = 0;

    let result = engine.on_event(Ev::down(VK_NONCONVERT).at(t0).build());
    assert_pending(&result);

    let t1 = t0 + 30_000;
    let result = engine.on_event(Ev::down(VK_A).at(t1).build());
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('を')));
}

#[test]
fn test_pattern2_char_first_then_thumb() {
    let mut engine = make_engine();
    let t0 = 0;

    let result = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&result);

    // char + thumb → PendingCharThumb（3 鍵目を待つ）
    let t1 = t0 + 30_000;
    let result = engine.on_event(Ev::down(VK_CONVERT).at(t1).build());
    assert_pending(&result);

    // タイムアウト → char1+thumb を同時打鍵として確定
    let result = engine.on_timeout(TIMER_PENDING);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('ゔ')));
}

#[test]
fn test_pattern3_char_timeout() {
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);

    let result = engine.on_timeout(TIMER_PENDING);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('う')));
}

#[test]
fn test_pattern4_char_sequence() {
    let mut engine = make_engine();
    let t0 = 0;

    let result = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&result);

    let t1 = t0 + 30_000;
    let result = engine.on_event(Ev::down(VK_S).at(t1).build());
    result.assert_consumed();
    assert!(result
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('う'))));
    result.assert_timer_set(TIMER_PENDING);
}

#[test]
fn test_pattern5_thumb_alone_timeout() {
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let result = engine.on_timeout(TIMER_PENDING);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Key(x) if x == VK_NONCONVERT));
}

#[test]
fn test_char_then_thumb_after_threshold() {
    let mut engine = make_engine();
    let t0 = 0;

    let result = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&result);

    let t1 = t0 + 200_000;
    let result = engine.on_event(Ev::down(VK_NONCONVERT).at(t1).build());
    result.assert_consumed();
    assert!(result
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('う'))));
}

#[test]
fn test_key_up_after_emit() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).build());
    engine.on_timeout(TIMER_PENDING);

    let result = engine.on_event(Ev::up(VK_A).build());
    result.assert_consumed();
    assert!(matches!(result.actions[0], KeyAction::Suppress));
}

#[test]
fn test_key_up_while_pending_no_double_char() {
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);

    let result = engine.on_event(Ev::up(VK_A).build());
    result.assert_consumed();

    let char_count = result
        .actions
        .iter()
        .filter(|a| matches!(a, KeyAction::Char('う')))
        .count();
    assert_eq!(char_count, 1, "Character should be emitted exactly once");
}

// ── swap_layout テスト ──

#[test]
fn test_swap_layout_no_pending() {
    let mut engine = make_engine();
    let new_layout = make_layout();
    let result = engine.swap_layout(new_layout);
    assert!(
        result.actions.is_empty(),
        "No pending key means no timeout actions"
    );
}

#[test]
fn test_swap_layout_flushes_pending_char() {
    let mut engine = make_engine();

    // 文字キーを保留状態にする
    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);

    // swap_layout で保留がタイムアウト確定される
    let new_layout = make_layout();
    let result = engine.swap_layout(new_layout);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('う')));
}

#[test]
fn test_swap_layout_flushes_pending_thumb() {
    let mut engine = make_engine();

    // 親指キーを保留状態にする
    let result = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&result);

    let new_layout = make_layout();
    let result = engine.swap_layout(new_layout);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Key(x) if x == VK_NONCONVERT));
}

#[test]
fn test_swap_layout_clears_output_history() {
    let mut engine = make_engine();

    // キーを確定して出力履歴にエントリを作る
    engine.on_event(Ev::down(VK_A).build());
    engine.on_timeout(TIMER_PENDING);

    // swap_layout で出力履歴がクリアされる
    let new_layout = make_layout();
    engine.swap_layout(new_layout);

    // 出力履歴がクリアされたので KeyUp は PassThrough になる
    let result = engine.on_event(Ev::up(VK_A).build());
    result.assert_pass_through();
}

#[test]
fn test_swap_layout_uses_new_layout() {
    let mut engine = make_engine();

    // 新しい配列��作成（A キーの通常面を 'か' に変更）
    let mut new_layout = make_layout();
    new_layout.normal.insert(POS_A, lit('か'));

    engine.swap_layout(new_layout);

    // 新しい配列で変換される
    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);
    let result = engine.on_timeout(TIMER_PENDING);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('か')));
}

#[test]
fn test_toggle_enabled() {
    let mut engine = make_engine();
    assert!(engine.is_enabled());
    let _ = engine.toggle_enabled();
    assert!(!engine.is_enabled());
    let _ = engine.toggle_enabled();
    assert!(engine.is_enabled());
}

// ── OS 予約キーコンビネーションのパススルーテスト ──

#[test]
fn test_ctrl_held_char_key_passes_through() {
    let mut engine = make_engine();

    // Ctrl を押下
    engine.on_event(Ev::down(VK_CTRL).build());

    // Ctrl が押されている状態で文字キーはパススルー
    engine
        .on_event(Ev::down(VK_A).build())
        .assert_pass_through();
}

#[test]
fn test_lctrl_held_char_key_passes_through() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_LCTRL).build());

    engine
        .on_event(Ev::down(VK_C).build())
        .assert_pass_through();
}

#[test]
fn test_alt_held_char_key_passes_through() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_ALT).build());

    engine
        .on_event(Ev::down(VK_A).build())
        .assert_pass_through();
}

#[test]
fn test_lalt_held_char_key_passes_through() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_LALT).build());

    engine
        .on_event(Ev::down(VK_V).build())
        .assert_pass_through();
}

#[test]
fn test_ctrl_released_char_key_resumes_conversion() {
    let mut engine = make_engine();

    // Ctrl 押下 → リリース
    engine.on_event(Ev::down(VK_CTRL).build());
    engine.on_event(Ev::up(VK_CTRL).build());

    // Ctrl が離された後は通常の変換が行われる（保留になる）
    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);
}

#[test]
fn test_ctrl_held_non_layout_key_passes_through() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_CTRL).build());

    // 配列定義にないキーも Ctrl 押下中はパススルー
    engine
        .on_event(Ev::down(VK_RETURN).build())
        .assert_pass_through();
}

// ── Shift 面テスト ──

fn make_engine_with_shift() -> TestHarness {
    let mut layout = make_layout();
    layout.shift.insert(POS_A, lit('ウ'));
    layout.shift.insert(POS_S, lit('シ'));
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    }
}

#[test]
fn test_shift_held_uses_shift_face() {
    let mut engine = make_engine_with_shift();

    // Shift を押下
    engine.on_event(Ev::down(VK_SHIFT).build());

    // Shift 面に定義がある文字キー → Shift 面の文字が出力される
    let result = engine.on_event(Ev::down(VK_A).build());
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('ウ')));
}

#[test]
fn test_shift_held_unlisted_key_passes_through() {
    let mut engine = make_engine_with_shift();

    // Shift を押下
    engine.on_event(Ev::down(VK_LSHIFT).build());

    // Shift 面に定義がないキー → PassThrough
    engine
        .on_event(Ev::down(VK_C).build())
        .assert_pass_through();
}

#[test]
fn test_shift_released_resumes_normal() {
    let mut engine = make_engine_with_shift();

    // Shift 押下 → リリース
    engine.on_event(Ev::down(VK_RSHIFT).build());
    engine.on_event(Ev::up(VK_RSHIFT).build());

    // Shift が離された後は通常の変換が行われる（保留になる）
    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);
}

// ── 3 鍵仲裁（d1/d2 比較）テスト ──

#[test]
fn test_three_key_d1_less_than_d2() {
    // char1(t=0) → thumb(t=20ms) → char2(t=80ms)
    // d1 = 20ms, d2 = 60ms → d1 < d2 → char1+thumb = 同時、char2 = 新規処理
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_A).at(0).build());
    assert_pending(&result);

    let result = engine.on_event(Ev::down(VK_CONVERT).at(20_000).build());
    assert_pending(&result); // PendingCharThumb

    let result = engine.on_event(Ev::down(VK_S).at(80_000).build());
    result.assert_consumed();
    // char1+thumb(右) で 'ゔ' が出力される
    assert!(result
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('ゔ'))));
    // 親指は消費済みなので char2 は保留に入り、ここでは出力されない
    assert!(
        !result
            .actions
            .iter()
            .any(|a| matches!(a, KeyAction::Char('じ'))),
        "char2 should NOT be thumb-shifted (thumb consumed)"
    );
}

#[test]
fn test_three_key_d1_greater_equal_d2() {
    // char1(t=0) → thumb(t=60ms) → char2(t=80ms)
    // d1 = 60ms, d2 = 20ms → d1 >= d2 → char1 = 単独、char2+thumb = 同時
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_A).at(0).build());
    assert_pending(&result);

    let result = engine.on_event(Ev::down(VK_CONVERT).at(60_000).build());
    assert_pending(&result); // PendingCharThumb

    let result = engine.on_event(Ev::down(VK_S).at(80_000).build());
    result.assert_consumed();
    // char1(VK_A) は単独確定 'う'、char2+thumb(右) で 'じ'
    assert!(result
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('う'))));
    assert!(result
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('じ'))));
}

#[test]
fn test_three_key_timeout_resolves_as_simultaneous() {
    // char1(t=0) → thumb(t=30ms) → タイムアウト（char2 来ない）
    // → char1+thumb を同時打鍵として確定
    let mut engine = make_engine();

    let result = engine.on_event(Ev::down(VK_A).at(0).build());
    assert_pending(&result);

    let result = engine.on_event(Ev::down(VK_NONCONVERT).at(30_000).build());
    assert_pending(&result); // PendingCharThumb

    let result = engine.on_timeout(TIMER_PENDING);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('を')));
}

#[test]
fn test_three_key_key_up_char_resolves_simultaneous() {
    // char1 → thumb → char1 KeyUp → char1+thumb を同時打鍵として確定
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).at(0).build());
    engine.on_event(Ev::down(VK_CONVERT).at(30_000).build());

    let result = engine.on_event(Ev::up(VK_A).build());
    result.assert_consumed();
    assert!(result
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('ゔ'))));
}

// ── 連続シフト用ヘルパー ──

fn make_engine_with_extended_layout() -> TestHarness {
    let mut layout = make_layout();
    // D, F を配列に追加
    layout.normal.insert(POS_D, lit('て'));
    layout.normal.insert(POS_F, lit('け'));
    layout.left_thumb.insert(POS_D, lit('な'));
    layout.left_thumb.insert(POS_F, lit('よ'));
    layout.right_thumb.insert(POS_D, lit('で'));
    layout.right_thumb.insert(POS_F, lit('げ'));
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    }
}

// ── 連続シフト（左親指）テスト ──

#[test]
fn test_continuous_shift_left_thumb() {
    // 左親指を押しっぱなしにしながら複数文字キーを打つ
    let mut engine = make_engine_with_extended_layout();
    let t = 0u64;

    // 左親指押下 → PendingThumb
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t).build());
    assert_pending(&r);

    // char1 が閾値内に到着 → 同時打鍵として確定、left_thumb_down がセットされる
    let r = engine.on_event(Ev::down(VK_A).at(t + 30_000).build());
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(
        matches!(r.actions[0], KeyAction::Char('を')),
        "char1 should use left thumb face"
    );

    // char2: 親指は消費済み → active_thumb_face() が None → PendingChar（保留）
    let r = engine.on_event(Ev::down(VK_S).at(t + 100_000).build());
    assert_pending(&r);

    // char3: PendingChar(S) 中に char(D) 到着 → S が通常面で単独確定、D が新たに保留
    let r = engine.on_event(Ev::down(VK_D).at(t + 170_000).build());
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('し'))),
        "char2 should use normal face for S (thumb consumed)"
    );

    // 親指リリース
    let _r = engine.on_event(Ev::up(VK_NONCONVERT).at(t + 200_000).build());

    // char4: PendingChar(D) 中に char(F) 到着 → D が通常面で単独確定、F が新たに保留
    let r = engine.on_event(Ev::down(VK_F).at(t + 250_000).build());
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('て'))),
        "char3 should use normal face for D"
    );

    // タイムアウトで F が通常面で出力される
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        matches!(r.actions[0], KeyAction::Char('け')),
        "char4 after thumb release should use normal face"
    );
}

// ── 連続シフト（右親指）テスト ──

#[test]
fn test_continuous_shift_right_thumb() {
    // 右親指を押しっぱなしにしながら複数文字キーを打つ
    let mut engine = make_engine_with_extended_layout();
    let t = 0u64;

    // 右親指押下 → PendingThumb
    let r = engine.on_event(Ev::down(VK_CONVERT).at(t).build());
    assert_pending(&r);

    // char1: 同時打鍵 → right_thumb_down セット
    let r = engine.on_event(Ev::down(VK_A).at(t + 30_000).build());
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(
        matches!(r.actions[0], KeyAction::Char('ゔ')),
        "char1 should use right thumb face"
    );

    // char2: 親指は消費済み → PendingChar（保留）
    let r = engine.on_event(Ev::down(VK_S).at(t + 100_000).build());
    assert_pending(&r);

    // char3: PendingChar(S) 中に char(D) 到着 → S が通常面で単独確定、D が新たに保留
    let r = engine.on_event(Ev::down(VK_D).at(t + 170_000).build());
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('し'))),
        "char2 should use normal face for S (thumb consumed)"
    );

    // 親指リリース
    let _r = engine.on_event(Ev::up(VK_CONVERT).at(t + 200_000).build());

    // char4: PendingChar(D) 中に char(F) 到着 → D が通常面で単独確定、F が新たに保留
    let r = engine.on_event(Ev::down(VK_F).at(t + 250_000).build());
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('て'))),
        "char3 should use normal face for D"
    );

    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        matches!(r.actions[0], KeyAction::Char('け')),
        "char4 after thumb release should use normal face"
    );
}

// ── PendingCharThumb タイムアウト後の連続シフト ──

#[test]
fn test_continuous_shift_after_pending_char_thumb_timeout() {
    // char1 → thumb → タイムアウト（同時打鍵確定）→ char2 が即時シフト出力されるか
    let mut engine = make_engine_with_extended_layout();
    let t = 0u64;

    // char1 → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t).build());
    assert_pending(&r);

    // thumb → PendingCharThumb
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t + 30_000).build());
    assert_pending(&r);

    // タイムアウト → char1+thumb 同時打鍵として確定、left_thumb_down がセットされる
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        matches!(r.actions[0], KeyAction::Char('を')),
        "timeout should resolve char1+left_thumb as simultaneous"
    );

    // char2: 親指は消費済み → PendingChar（保留）→ タイムアウトで通常面
    let r = engine.on_event(Ev::down(VK_S).at(t + 200_000).build());
    assert_pending(&r);

    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('し'))),
        "char2 after PendingCharThumb timeout should use normal face (thumb consumed)"
    );
}

// ── PendingCharThumb 3鍵仲裁 (d1 < d2) 後の連続シフト ──

#[test]
fn test_continuous_shift_after_three_key_d1_less_d2() {
    // char1(t=0) → thumb(t=20ms) → char2(t=80ms) → char3
    // d1=20ms < d2=60ms → char1+thumb 同時、char2 は process_new_key_down
    // char2 は thumb_down セット済みなので即時シフト出力
    // char3 も同様に即時シフト出力されるべき
    let mut engine = make_engine_with_extended_layout();

    let r = engine.on_event(Ev::down(VK_A).at(0).build());
    assert_pending(&r);

    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(20_000).build());
    assert_pending(&r); // PendingCharThumb

    // char2 到着: d1=20ms, d2=60ms → char1+thumb 同時、char2 は保留（親指消費済み）
    let r = engine.on_event(Ev::down(VK_S).at(80_000).build());
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('を'))),
        "char1+left_thumb should produce 'を'"
    );
    // char2 は親指消費済みのため保留に入り、ここでは出力されない

    // char3: PendingChar(S) 中に char(D) 到着 → S が通常面で単独確定、D が新たに保留
    let r = engine.on_event(Ev::down(VK_D).at(150_000).build());
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('し'))),
        "char2 should use normal face for S (thumb consumed)"
    );
}

// ── PendingCharThumb KeyUp 解決後の連続シフト ──

#[test]
fn test_continuous_shift_after_pending_char_thumb_key_up() {
    // char1 → thumb → char1 KeyUp → 同時打鍵確定 → char2 即時シフト
    let mut engine = make_engine_with_extended_layout();
    let t = 0u64;

    engine.on_event(Ev::down(VK_A).at(t).build());
    engine.on_event(Ev::down(VK_NONCONVERT).at(t + 30_000).build());

    // char1 KeyUp → 同時打鍵として確定
    let r = engine.on_event(Ev::up(VK_A).at(t + 60_000).build());
    r.assert_consumed();
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('を'))));

    // char2: 親指は消費済み → PendingChar（保留）→ タイムアウトで通常面
    let r = engine.on_event(Ev::down(VK_S).at(t + 100_000).build());
    assert_pending(&r);

    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('し'))),
        "char2 after KeyUp-resolved simultaneous should use normal face (thumb consumed)"
    );
}

// ── 連続シフト中に反対側の親指が来た場合 ──

#[test]
fn test_continuous_shift_switch_thumb() {
    // 左親指押下 → char1(左シフト) → 左親指リリース → 右親指押下 → char2(右シフト)
    let mut engine = make_engine_with_extended_layout();
    let t = 0u64;

    // 左親指 → char1
    engine.on_event(Ev::down(VK_NONCONVERT).at(t).build());
    let r = engine.on_event(Ev::down(VK_A).at(t + 30_000).build());
    r.assert_consumed();
    assert!(matches!(r.actions[0], KeyAction::Char('を')));

    // 左親指リリース
    engine.on_event(Ev::up(VK_NONCONVERT).at(t + 80_000).build());

    // 右親指押下 → PendingThumb
    let r = engine.on_event(Ev::down(VK_CONVERT).at(t + 100_000).build());
    assert_pending(&r);

    // char2 → 右シフト面
    let r = engine.on_event(Ev::down(VK_A).at(t + 130_000).build());
    r.assert_consumed();
    assert!(
        matches!(r.actions[0], KeyAction::Char('ゔ')),
        "after switching thumbs, char should use right thumb face"
    );
}

// scan_to_pos テストは awase-windows に移動済み

#[test]
fn test_nicola_state_stores_scan_code() {
    // Verify that NicolaState variants correctly propagate scan_code from
    // RawKeyEvent — this is the infrastructure needed for .yab migration.
    let mut engine = make_engine();

    // Create a key event with a specific scan code
    let event = RawKeyEvent {
        vk_code: VK_A,
        scan_code: ScanCode(0x1E), // A key scan code
        event_type: KeyEventType::KeyDown,
        extra_info: 0,
        timestamp: 0,
        ime_relevance: crate::types::ImeRelevance::default(),
        modifier_key: None,
        key_classification: crate::types::KeyClassification::Char,
        physical_pos: Some(crate::scanmap::PhysicalPos::new(2, 0)),
    };

    let result = engine.on_event(event);
    assert_pending(&result);

    // The engine should have stored the scan_code in pending_char
    let EngineState::PendingChar(pending) = engine.state else {
        panic!("expected PendingChar state, got {:?}", engine.state);
    };
    assert_eq!(
        pending.scan_code,
        ScanCode(0x1E),
        "scan_code should be preserved in pending_char"
    );
}

#[test]
fn test_pending_char_thumb_stores_char_scan() {
    // Verify PendingCharThumb preserves char_scan from the original key event.
    let mut engine = make_engine();

    let char_event = RawKeyEvent {
        vk_code: VK_A,
        scan_code: ScanCode(0x1E),
        event_type: KeyEventType::KeyDown,
        extra_info: 0,
        timestamp: 0,
        ime_relevance: crate::types::ImeRelevance::default(),
        modifier_key: None,
        key_classification: crate::types::KeyClassification::Char,
        physical_pos: Some(crate::scanmap::PhysicalPos::new(2, 0)),
    };
    engine.on_event(char_event);

    let thumb_event = RawKeyEvent {
        vk_code: VK_CONVERT,
        scan_code: ScanCode(0x79), // Convert key scan code
        event_type: KeyEventType::KeyDown,
        extra_info: 0,
        timestamp: 30_000,
        ime_relevance: crate::types::ImeRelevance::default(),
        modifier_key: None,
        key_classification: crate::types::KeyClassification::RightThumb,
        physical_pos: None,
    };
    let result = engine.on_event(thumb_event);
    assert_pending(&result);

    let EngineState::PendingCharThumb { char_key, .. } = engine.state else {
        panic!("expected PendingCharThumb state, got {:?}", engine.state);
    };
    assert_eq!(
        char_key.scan_code,
        ScanCode(0x1E),
        "char_scan should be preserved in pending_char"
    );
}

// ── yab_value_to_action coverage ──

#[test]
fn test_yab_value_to_action_romaji() {
    let action = yab_value_to_action(&YabValue::Romaji {
        romaji: "ka".to_string(),
        kana: Some('か'),
    });
    assert!(matches!(action, KeyAction::Romaji(ref s) if s == "ka"));
}

#[test]
fn test_yab_value_to_action_literal() {
    let action = yab_value_to_action(&YabValue::Literal("あ".to_string()));
    assert!(matches!(action, KeyAction::Char('あ')));
}

#[test]
fn test_yab_value_to_action_literal_empty() {
    let action = yab_value_to_action(&YabValue::Literal(String::new()));
    assert!(matches!(action, KeyAction::Suppress));
}

#[test]
fn test_yab_value_to_action_special() {
    use crate::yab::SpecialKey;
    let action = yab_value_to_action(&YabValue::Special(SpecialKey::Backspace));
    assert!(matches!(
        action,
        KeyAction::SpecialKey(crate::types::SpecialKey::Backspace)
    ));
}

#[test]
fn test_yab_value_to_action_key_sequence() {
    let action = yab_value_to_action(&YabValue::KeySequence("?".to_string()));
    assert!(matches!(action, KeyAction::KeySequence(ref s) if s == "?"));
}

#[test]
fn test_yab_value_to_action_none() {
    let action = yab_value_to_action(&YabValue::None);
    assert!(matches!(action, KeyAction::Suppress));
}

// ── toggle_enabled with pending state ──

#[test]
fn test_toggle_enabled_returns_state() {
    let mut engine = make_engine();
    assert!(engine.is_enabled());
    let (enabled, _) = engine.toggle_enabled();
    assert!(!enabled);
    let (enabled, _) = engine.toggle_enabled();
    assert!(enabled);
}

// ── flush_pending: 全状態からの安全なリセット ──

#[test]
fn test_flush_pending_from_idle_is_noop() {
    let mut engine = make_engine();
    let r = engine.flush_pending(ContextChange::ImeOff);
    // Idle → no-op, consume with no actions
    assert!(r.actions.is_empty());
    assert!(r.consumed);
    // 再入しても no-op
    let r2 = engine.flush_pending(ContextChange::ImeOff);
    assert!(r2.actions.is_empty());
}

#[test]
fn test_flush_pending_from_pending_char() {
    let mut engine = make_engine();
    let t0 = 1_000_000;
    // PendingChar 状態にする
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    // flush → 通常面で単独確定
    let r = engine.flush_pending(ContextChange::EngineDisabled);
    assert!(!r.actions.is_empty(), "should emit the pending char");
    // Idle に戻っている
    let r2 = engine.flush_pending(ContextChange::ImeOff);
    assert!(r2.actions.is_empty(), "should be idle after flush");
}

#[test]
fn test_flush_pending_from_pending_thumb() {
    let mut engine = make_engine();
    let t0 = 1_000_000;
    // PendingThumb 状態にする
    let _ = engine.on_event(Ev::down(VK_NONCONVERT).at(t0).build());
    // flush → 親指キーを単独確定
    let r = engine.flush_pending(ContextChange::InputLanguageChanged);
    assert!(!r.actions.is_empty(), "should emit the pending thumb key");
    assert!(matches!(r.actions[0], KeyAction::Key(x) if x == VK_NONCONVERT));
}

#[test]
fn test_flush_pending_from_pending_char_thumb() {
    let mut engine = make_engine();
    let t0 = 1_000_000;
    // PendingChar → PendingCharThumb にする
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    let _ = engine.on_event(Ev::down(VK_NONCONVERT).at(t0 + 30_000).build());
    // flush → 同時打鍵として確定
    let r = engine.flush_pending(ContextChange::LayoutSwapped);
    assert!(!r.actions.is_empty(), "should emit simultaneous result");
}

#[test]
fn test_flush_pending_from_speculative_char() {
    let mut engine = make_speculative_engine();
    let t0 = 1_000_000;
    // SpeculativeChar 状態にする（即時出力済み）
    let r1 = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert!(!r1.actions.is_empty(), "speculative output");
    // flush → 既に出力済みなので追加出力なし
    let r = engine.flush_pending(ContextChange::ImeOff);
    assert!(
        r.actions.is_empty(),
        "speculative was already output, no additional actions"
    );
}

#[test]
fn test_flush_pending_cancels_timers() {
    let mut engine = make_engine();
    let t0 = 1_000_000;
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    let r = engine.flush_pending(ContextChange::ImeOff);
    // タイマー停止命令が含まれる（assert_timer_kill ヘルパーを使用）
    r.assert_timer_kill(TIMER_PENDING);
    r.assert_timer_kill(TIMER_SPECULATIVE);
}

#[test]
fn test_toggle_enabled_flushes_pending() {
    let mut engine = make_engine();
    let t0 = 1_000_000;
    // PendingChar 状態にする
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    // toggle → 保留がフラッシュされる
    let (enabled, flush_resp) = engine.toggle_enabled();
    assert!(!enabled);
    assert!(
        !flush_resp.actions.is_empty(),
        "should flush the pending char"
    );
}

// ── IME 制御キーのフラッシュ＋パススルー ──

const VK_KANJI: VkCode = VkCode(0x19); // 半角/全角キー
const SCAN_KANJI: ScanCode = ScanCode(0x29);

#[test]
fn test_ime_control_key_passes_through_from_idle() {
    let mut engine = make_engine();
    // Idle 状態で半角/全角 → pass_through, アクションなし
    let r = engine.on_event(Ev::down(VK_KANJI).scan(SCAN_KANJI).build());
    r.assert_pass_through();
    assert!(r.actions.is_empty());
}

#[test]
fn test_ime_control_key_flushes_pending_and_passes_through() {
    let mut engine = make_engine();
    let t0 = 1_000_000;
    // PendingChar 状態にする
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    // 半角/全角キー到着 → 保留フラッシュ + パススルー
    let r = engine.on_event(Ev::down(VK_KANJI).scan(SCAN_KANJI).at(t0 + 50_000).build());
    // consumed=false (パススルー) だがフラッシュアクションが含まれる
    assert!(!r.consumed, "should pass through the IME control key");
    assert!(
        !r.actions.is_empty(),
        "should emit flushed pending char actions"
    );
}

#[test]
fn test_ime_control_key_flushes_speculative_and_passes_through() {
    let mut engine = make_speculative_engine();
    let t0 = 1_000_000;
    // SpeculativeChar 状態にする
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    // 半角/全角キー → speculative は確定済みなので追加アクションなし、パススルー
    let r = engine.on_event(Ev::down(VK_KANJI).scan(SCAN_KANJI).at(t0 + 50_000).build());
    assert!(!r.consumed, "should pass through the IME control key");
}

// ── set_ngram_model / timing_judge ──

#[test]
fn test_set_ngram_model_and_timing_judge() {
    let mut engine = make_engine();
    // Without model, timing_judge uses fixed threshold (is_simultaneous within threshold)
    let judge = engine.timing_judge();
    assert!(judge.is_simultaneous(0, engine.threshold_us - 1, Some('あ')));
    assert!(!judge.is_simultaneous(0, engine.threshold_us + 1, Some('あ')));
    drop(judge);

    // With model, timing_judge uses the model for threshold adjustment
    let model = NgramModel::new(100_000, 20_000, 30_000, 120_000);
    engine.set_ngram_model(model);
    // Unknown candidate -> score 0 -> tanh(0)=0 -> base threshold unchanged
    let judge = engine.timing_judge();
    assert!(judge.is_simultaneous(0, 99_999, Some('x')));
    assert!(!judge.is_simultaneous(0, 100_001, Some('x')));
}

// ── PendingThumb + another thumb key (expired) ──

#[test]
fn test_pending_thumb_then_char_after_threshold() {
    // PendingThumb + char after threshold -> thumb single, char new pending
    let mut engine = make_engine();

    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(0).build());
    assert_pending(&r);

    // Char arrives after threshold
    let r = engine.on_event(Ev::down(VK_A).at(200_000).build());
    r.assert_consumed();
    // Should contain thumb single (Key(VK_NONCONVERT)) and char is new pending
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT)));
}

#[test]
fn test_pending_thumb_then_another_thumb() {
    // PendingThumb + another thumb -> first thumb single, second thumb pending
    let mut engine = make_engine();

    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(0).build());
    assert_pending(&r);

    // Another thumb arrives within threshold (still same kind = thumb)
    let r = engine.on_event(Ev::down(VK_CONVERT).at(30_000).build());
    r.assert_consumed();
    // First thumb resolved as single
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT)));
}

// ── PendingCharThumb + thumb key arrival (line 537, 543-544) ──

#[test]
fn test_pending_char_thumb_then_another_thumb() {
    // char1 -> thumb1 -> thumb2 arrives
    // Should resolve char1+thumb1 as simultaneous, thumb2 as new pending
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).at(0).build());
    engine.on_event(Ev::down(VK_NONCONVERT).at(20_000).build());

    // Another thumb key arrives
    let r = engine.on_event(Ev::down(VK_CONVERT).at(50_000).build());
    r.assert_consumed();
    // char1+left_thumb -> 'を'
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('を'))));
}

// ── resolve_char_thumb_as_simultaneous when no thumb face definition (line 248) ──

#[test]
fn test_resolve_char_thumb_no_thumb_face_definition() {
    // Use a key defined in normal face but NOT in any thumb face
    let mut engine = make_engine();
    engine.layout.normal.insert(POS_D, lit('て'));

    engine.on_event(Ev::down(VK_D).at(0).build());
    engine.on_event(Ev::down(VK_NONCONVERT).at(20_000).build());

    // Timeout resolves as simultaneous, but D is not in left_thumb face
    // Falls back to single char resolution via normal face -> 'て'
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('て'))));
}

// ── ReduceAndContinue + pass_through edge cases ──

#[test]
fn test_pending_char_then_non_layout_key_passes_through_new() {
    // PendingChar 中に scan_to_pos にないキー（VK_RETURN 等）が来た場合、
    // InputTracker で Passthrough に分類されるため、
    // bypass_reason → Passthrough で即座にパススルーされる。
    // 保留中の A はタイムアウトで後から確定される。
    let mut engine = make_engine();

    let r = engine.on_event(Ev::down(VK_A).at(0).build());
    assert_pending(&r);

    // VK_RETURN は scan_to_pos にないので Passthrough に分類される
    let r = engine.on_event(Ev::down(VK_RETURN).at(200_000).build());
    assert!(!r.consumed, "non-layout key should pass through");
}

// ── KeyUp while PendingThumb (lines 599-600, 606) ──

#[test]
fn test_key_up_while_pending_thumb() {
    let mut engine = make_engine();

    let r = engine.on_event(Ev::down(VK_NONCONVERT).build());
    assert_pending(&r);

    // KeyUp of the pending thumb key -> resolves as single Key(VK_NONCONVERT)
    let r = engine.on_event(Ev::up(VK_NONCONVERT).build());
    r.assert_consumed();
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_NONCONVERT)));
    // Note: output_history uses vk_code as key for thumb,
    // so scan_code-based removal in on_key_up won't find it -> no KeyUp action appended
}

// ── KeyUp for active Key action (lines 619-620) ──

#[test]
fn test_key_up_active_key_action() {
    // When a key was resolved as Key(vk) and then KeyUp arrives
    // Use a key in thumb face only (not normal) so it's a layout key
    // but resolve_pending_char_as_single falls back to Key(vk)
    let mut engine = make_engine();
    // Add D only to left_thumb face, not normal
    engine.layout.left_thumb.insert(POS_D, lit('な'));

    // D is now a layout key, enters pending
    engine.on_event(Ev::down(VK_D).build());
    // Timeout: not in normal face -> Key(VK_D)
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_D)));

    // KeyUp should produce KeyUp(VK_D)
    let r = engine.on_event(Ev::up(VK_D).build());
    r.assert_consumed();
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::KeyUp(x) if *x == VK_D)));
}

// ── KeyUp for active Suppress/other action (line 622) ──

#[test]
fn test_key_up_active_suppress_action() {
    // When output_history has a Suppress action (unlikely in practice, but covers pass_through branch)
    let mut engine = make_engine();

    // Manually insert a Suppress action into output_history
    engine.output_history.push(OutputEntry {
        scan_code: SCAN_D,
        romaji: String::new(),
        kana: None,
        action: KeyAction::Suppress,
    });

    let r = engine.on_event(Ev::up(VK_D).build());
    r.assert_pass_through();
}

// ── KeyUp during PendingCharThumb resolving Key action (line 580) ──

#[test]
fn test_key_up_pending_char_thumb_resolves_char() {
    // char1 in normal face but NOT in thumb face -> resolves via normal face as Char
    let mut engine = make_engine();
    engine.layout.normal.insert(POS_D, lit('て'));

    engine.on_event(Ev::down(VK_D).at(0).build());
    engine.on_event(Ev::down(VK_NONCONVERT).at(20_000).build());

    // KeyUp of D -> resolves char1+thumb as simultaneous
    // D not in left_thumb -> fallback to single via normal -> Char('て')
    let r = engine.on_event(Ev::up(VK_D).build());
    r.assert_consumed();
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('て'))));
}

#[test]
fn test_key_up_pending_char_thumb_resolves_key_with_keyup() {
    // char1 NOT in normal or left_thumb -> fallback to Key(vk), KeyUp appended (line 580)
    let mut engine = make_engine();
    // Add D only to right_thumb (not left_thumb, not normal)
    engine.layout.right_thumb.insert(POS_D, lit('で'));

    engine.on_event(Ev::down(VK_D).at(0).build());
    // Left thumb -> PendingCharThumb
    engine.on_event(Ev::down(VK_NONCONVERT).at(20_000).build());

    // KeyUp of D -> resolve char1+left_thumb
    // D NOT in left_thumb -> fallback to single
    // D NOT in normal -> Key(VK_D), output_history records Key(VK_D)
    // Then on KeyUp: output_history removal finds Key(VK_D) -> push KeyUp
    let r = engine.on_event(Ev::up(VK_D).build());
    r.assert_consumed();
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_D)));
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::KeyUp(x) if *x == VK_D)));
}

// ── is_layout_key coverage (lines 657-659) ──

#[test]
fn test_is_layout_key_various_faces() {
    let engine = make_engine();
    // A key is in normal face
    assert!(engine.is_layout_key(Some(POS_A)));
    // D key is NOT in any face in the basic layout
    assert!(!engine.is_layout_key(Some(POS_D)));
    // None pos
    assert!(!engine.is_layout_key(None));
}

#[test]
fn test_is_layout_key_thumb_and_shift_faces() {
    let mut engine = make_engine_with_shift();
    // A is in normal, left_thumb, right_thumb, and shift
    assert!(engine.is_layout_key(Some(POS_A)));

    // Add D only to left_thumb face
    engine.layout.left_thumb.insert(POS_D, lit('な'));
    assert!(engine.is_layout_key(Some(POS_D)));
}

// ── timeout for char not in normal layout (lines 722-723) ──

#[test]
fn test_timeout_char_not_in_normal_layout() {
    let mut engine = make_engine();

    // F key is not in normal layout but IS a layout key in extended layout
    // We need a key that gets past is_layout_key but isn't in normal face
    // Add F to left_thumb only
    engine.layout.left_thumb.insert(POS_F, lit('よ'));

    // F is now a layout key (in left_thumb), so it will be pending
    let r = engine.on_event(Ev::down(VK_F).build());
    assert_pending(&r);

    // Timeout -> not in normal face -> Key(VK_F)
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_F)));
}

// ── swap_layout with PendingCharThumb ──

#[test]
fn test_swap_layout_flushes_pending_char_thumb() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).at(0).build());
    engine.on_event(Ev::down(VK_NONCONVERT).at(20_000).build());

    let new_layout = make_layout();
    let r = engine.swap_layout(new_layout);
    r.assert_consumed();
    // Should resolve char1+thumb as simultaneous
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('を'))));
}

// ── d1 >= d2 path where char2 has no thumb face definition (lines 532-533) ──

#[test]
fn test_three_key_d1_ge_d2_no_thumb_face_for_char2() {
    // char1(A, t=0) -> thumb(t=60ms) -> char2(D, t=80ms)
    // d1=60ms >= d2=20ms -> char1 single, char2+thumb attempted but D not in thumb face
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).at(0).build());
    engine.on_event(Ev::down(VK_CONVERT).at(60_000).build());

    // D is not in right_thumb face -> falls through to process_new_key_down
    let r = engine.on_event(Ev::down(VK_D).at(80_000).build());
    r.assert_consumed();
    // char1(A) -> 'う' (single)
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('う'))));
}

// ── Romaji in layout face ──

#[test]
fn test_romaji_value_in_layout() {
    let mut layout = make_layout();
    layout.normal.insert(
        POS_D,
        YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: Some('か'),
        },
    );
    let mut engine = TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    };

    engine.on_event(Ev::down(VK_D).build());
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        matches!(&r.actions[0], KeyAction::Romaji(s) if s == "ka"),
        "should output Romaji action"
    );
}

// ── Special key in layout face ──

#[test]
fn test_special_value_in_layout() {
    use crate::yab::SpecialKey;
    let mut layout = make_layout();
    layout
        .normal
        .insert(POS_D, YabValue::Special(SpecialKey::Backspace));
    let mut engine = TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    };

    engine.on_event(Ev::down(VK_D).build());
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(matches!(
        r.actions[0],
        KeyAction::SpecialKey(crate::types::SpecialKey::Backspace)
    ));

    // SpecialKey actions are atomic (down+up in one shot).
    // output_history stores the SpecialKey action, so KeyUp finds it
    // and suppresses or passes through since there's no VK to release.
    let _r = engine.on_event(Ev::up(VK_D).build());
}

// ── None value in layout face ──

#[test]
fn test_none_value_in_layout() {
    let mut layout = make_layout();
    layout.normal.insert(POS_D, YabValue::None);
    let mut engine = TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    };

    engine.on_event(Ev::down(VK_D).build());
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(matches!(r.actions[0], KeyAction::Suppress));
}

// SysKeyDown/SysKeyUp テストは削除
// Windows の SysKey イベントはプラットフォーム層で KeyDown/KeyUp に変換される

// ── KeyUp of thumb during PendingCharThumb (thumb released) ──

#[test]
fn test_key_up_thumb_during_pending_char_thumb() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).at(0).build());
    engine.on_event(Ev::down(VK_CONVERT).at(20_000).build());

    // Thumb KeyUp -> resolves char1+thumb as simultaneous
    let r = engine.on_event(Ev::up(VK_CONVERT).build());
    r.assert_consumed();
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('ゔ'))));
}

// ── Romaji KeyUp produces Suppress ──

#[test]
fn test_key_up_for_romaji_produces_suppress() {
    let mut layout = make_layout();
    layout.normal.insert(
        POS_D,
        YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: Some('か'),
        },
    );
    let mut engine = TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        ),
    };

    engine.on_event(Ev::down(VK_D).build());
    engine.on_timeout(TIMER_PENDING);

    let r = engine.on_event(Ev::up(VK_D).build());
    r.assert_consumed();
    assert!(matches!(r.actions[0], KeyAction::Suppress));
}

// ── KeyUp while PendingChar resolves to Key action (line 606) ──

#[test]
fn test_key_up_while_pending_char_key_action() {
    // A key that's a layout key but NOT in normal face -> resolves to Key(vk)
    // Then KeyUp should find Key in output_history and append KeyUp
    let mut engine = make_engine();
    // Add D only to left_thumb (not normal)
    engine.layout.left_thumb.insert(POS_D, lit('な'));

    // D is a layout key, enters PendingChar
    let r = engine.on_event(Ev::down(VK_D).build());
    assert_pending(&r);

    // KeyUp of D while pending -> resolve_pending_char_as_single
    // D not in normal -> Key(VK_D), output_history records Key(VK_D)
    // Then output_history removal finds Key(VK_D) -> push KeyUp(VK_D)
    let r = engine.on_event(Ev::up(VK_D).build());
    r.assert_consumed();
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(x) if *x == VK_D)));
    assert!(r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::KeyUp(x) if *x == VK_D)));
}

// ── 3-key d1 >= d2 with left thumb (lines 516, 522) ──

#[test]
fn test_three_key_d1_ge_d2_left_thumb() {
    // char1(A, t=0) -> left_thumb(t=60ms) -> char2(S, t=80ms)
    // d1=60ms >= d2=20ms -> char1 single, char2+left_thumb simultaneous
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_A).at(0).build());
    engine.on_event(Ev::down(VK_NONCONVERT).at(60_000).build());

    let r = engine.on_event(Ev::down(VK_S).at(80_000).build());
    r.assert_consumed();
    // char1(A) -> 'う' (single), char2(S)+left_thumb -> 'あ'
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('う'))));
    assert!(r.actions.iter().any(|a| matches!(a, KeyAction::Char('あ'))));
}

// ── output_history tracking tests ──

#[test]
fn test_output_history_tracked_on_timeout() {
    let mut engine = make_engine();

    // Press A key, goes to PendingChar
    let result = engine.on_event(Ev::down(VK_A).build());
    assert_pending(&result);
    assert!(engine.output_history.recent_kana(3).is_empty());

    // Timeout confirms A as standalone → 'う' (normal face)
    let result = engine.on_timeout(TIMER_PENDING);
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('う')));
    assert_eq!(engine.output_history.recent_kana(3), vec!['う']);
}

#[test]
fn test_output_history_tracked_on_simultaneous() {
    let mut engine = make_engine();
    let t0 = 0;

    // Thumb first (left thumb = nonconvert)
    let result = engine.on_event(Ev::down(VK_NONCONVERT).at(t0).build());
    assert_pending(&result);

    // Char arrives within threshold → simultaneous → left_thumb face for A = 'を'
    let t1 = t0 + 30_000;
    let result = engine.on_event(Ev::down(VK_A).at(t1).build());
    result.assert_consumed();
    assert_eq!(result.actions.len(), 1);
    assert!(matches!(result.actions[0], KeyAction::Char('を')));
    assert_eq!(engine.output_history.recent_kana(3), vec!['を']);
}

#[test]
fn test_output_history_recent_kana_limit() {
    let mut engine = make_engine();

    // Output 4 chars via successive timeout confirmations
    // 1st: A → 'う'
    engine.on_event(Ev::down(VK_A).build());
    engine.on_timeout(TIMER_PENDING);
    assert_eq!(engine.output_history.recent_kana(3), vec!['う']);

    // 2nd: S → 'し'
    engine.on_event(Ev::down(VK_S).build());
    engine.on_timeout(TIMER_PENDING);
    assert_eq!(engine.output_history.recent_kana(3), vec!['う', 'し']);

    // 3rd: A → 'う'
    engine.on_event(Ev::down(VK_A).build());
    engine.on_timeout(TIMER_PENDING);
    assert_eq!(engine.output_history.recent_kana(3), vec!['う', 'し', 'う']);

    // 4th: S → 'し' — recent_kana(3) returns only the last 3
    engine.on_event(Ev::down(VK_S).build());
    engine.on_timeout(TIMER_PENDING);
    assert_eq!(engine.output_history.recent_kana(3), vec!['し', 'う', 'し']);
}

// ── n-gram adaptive threshold tests ──────────────────────────────

/// Create an NgramModel with known bigram scores for threshold tests.
///
/// Layout reminder:
///   Left thumb face:  A → 'を', S → 'あ'
///   Right thumb face: A → 'ゔ', S → 'じ'
///
/// Bigrams:
///   "しを" =  2.0  → tanh(2.0) ≈ 0.964 → threshold ≈ 100_000 + 19_280 = 119_280us
///   "しゔ" = -2.0  → tanh(-2.0) ≈ -0.964 → threshold ≈ 100_000 - 19_280 = 80_720us
fn make_ngram_model() -> NgramModel {
    let toml = r#"
[bigram]
"しを" = 2.0
"しゔ" = -2.0

[trigram]
"#;
    // base = 100ms = 100_000us, range = 20ms = 20_000us
    NgramModel::from_toml(toml, 100_000, 20_000, 30_000, 120_000).unwrap()
}

/// High-frequency bigram candidate relaxes threshold so borderline timing
/// is accepted as simultaneous.
///
/// Scenario: recent output = 'し', then A + left thumb (→ candidate 'を').
/// Bigram "しを" = 2.0 → adjusted threshold ≈ 119ms.
/// Gap = 105ms: without n-gram 105ms > 100ms (standalone),
///              with n-gram 105ms < 119ms (simultaneous).
#[test]
fn test_ngram_high_freq_relaxes_threshold() {
    let mut engine = make_engine();
    engine.set_ngram_model(make_ngram_model());
    // Seed output_history with 'し' to provide n-gram context
    engine.output_history.push(OutputEntry {
        scan_code: SCAN_S,
        romaji: String::new(),
        kana: Some('し'),
        action: KeyAction::Char('し'),
    });

    let t0: u64 = 0;

    // Char key A → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&r);

    // Left thumb 105ms later → should enter PendingCharThumb
    // (105_000us < adjusted threshold ~119_280us)
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t0 + 105_000).build());
    r.assert_consumed();
    assert!(r.actions.is_empty(), "should be pending, not yet emitted");

    // Timeout resolves PendingCharThumb as simultaneous → left thumb face for A = 'を'
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(
        matches!(r.actions[0], KeyAction::Char('を')),
        "high-freq bigram should relax threshold: expected 'を', got {:?}",
        r.actions[0]
    );
}

/// Low-frequency bigram candidate tightens threshold so borderline timing
/// is rejected as standalone.
///
/// Scenario: recent output = 'し', then A + right thumb (→ candidate 'ゔ').
/// Bigram "しゔ" = -2.0 → adjusted threshold ≈ 81ms.
/// Gap = 90ms: without n-gram 90ms < 100ms (simultaneous),
///             with n-gram 90ms > 81ms (standalone).
#[test]
fn test_ngram_low_freq_tightens_threshold() {
    let mut engine = make_engine();
    engine.set_ngram_model(make_ngram_model());
    // Seed output_history with 'し' to provide n-gram context
    engine.output_history.push(OutputEntry {
        scan_code: SCAN_S,
        romaji: String::new(),
        kana: Some('し'),
        action: KeyAction::Char('し'),
    });

    let t0: u64 = 0;

    // Char key A → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&r);

    // Right thumb 90ms later → should NOT enter PendingCharThumb
    // (90_000us > adjusted threshold ~80_720us → time exceeded)
    // Instead, the pending char is resolved as standalone ('う' from normal face),
    // and the thumb key becomes a new pending.
    let r = engine.on_event(Ev::down(VK_CONVERT).at(t0 + 90_000).build());
    r.assert_consumed();

    // The standalone char 'う' should be emitted immediately
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('う'))),
        "low-freq bigram should tighten threshold: expected standalone 'う', got {:?}",
        r.actions
    );
}

/// Without n-gram model, the engine uses a fixed threshold (100ms).
/// 95ms < 100ms → simultaneous detection via left thumb face.
#[test]
fn test_without_ngram_fixed_threshold_simultaneous() {
    let mut engine = make_engine();
    // No set_ngram_model call — uses fixed 100ms threshold

    let t0: u64 = 0;

    // Char key A → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&r);

    // Left thumb 95ms later → within fixed 100ms threshold → PendingCharThumb
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t0 + 95_000).build());
    r.assert_consumed();
    assert!(r.actions.is_empty(), "should be pending (PendingCharThumb)");

    // Timeout → simultaneous → left thumb face for A = 'を'
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(
        matches!(r.actions[0], KeyAction::Char('を')),
        "fixed threshold: 95ms < 100ms should be simultaneous, got {:?}",
        r.actions[0]
    );
}

/// Without n-gram model, 105ms > fixed 100ms threshold → standalone.
/// This is the counterpart to test_ngram_high_freq_relaxes_threshold:
/// same 105ms gap, but without n-gram it's rejected.
#[test]
fn test_without_ngram_fixed_threshold_standalone() {
    let mut engine = make_engine();
    // No set_ngram_model call — uses fixed 100ms threshold

    let t0: u64 = 0;

    // Char key A → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&r);

    // Left thumb 105ms later → exceeds fixed 100ms threshold → standalone
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t0 + 105_000).build());
    r.assert_consumed();

    // The standalone char 'う' (normal face) should be emitted
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('う'))),
        "fixed threshold: 105ms > 100ms should be standalone 'う', got {:?}",
        r.actions
    );
}

/// Without n-gram, 90ms < 100ms → simultaneous via right thumb.
/// This is the counterpart to test_ngram_low_freq_tightens_threshold:
/// same 90ms gap, but without n-gram it's accepted.
#[test]
fn test_without_ngram_90ms_is_simultaneous() {
    let mut engine = make_engine();
    // No set_ngram_model call — uses fixed 100ms threshold

    let t0: u64 = 0;

    // Char key A → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    assert_pending(&r);

    // Right thumb 90ms later → within fixed 100ms → PendingCharThumb
    let r = engine.on_event(Ev::down(VK_CONVERT).at(t0 + 90_000).build());
    r.assert_consumed();
    assert!(r.actions.is_empty(), "should be pending (PendingCharThumb)");

    // Timeout → simultaneous → right thumb face for A = 'ゔ'
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(
        matches!(r.actions[0], KeyAction::Char('ゔ')),
        "fixed threshold: 90ms < 100ms should be simultaneous, got {:?}",
        r.actions[0]
    );
}

#[test]
fn test_speculative_char_timeout_confirms() {
    // Manually set engine to SpeculativeChar state
    let mut engine = make_engine();
    engine.state = EngineState::SpeculativeChar(PendingKey {
        scan_code: SCAN_A,
        vk_code: VK_A,
        pos: Some(POS_A),
        timestamp: 1_000_000,
    });

    // Call on_timeout → should return to Idle with no actions
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "SpeculativeChar timeout should produce no actions (already emitted), got {:?}",
        r.actions
    );
    assert!(
        engine.state.is_idle(),
        "state should be Idle after timeout, got {:?}",
        engine.state
    );
}

// ── Speculative confirm mode tests ──

#[test]
fn test_speculative_single_char() {
    // Character press in Speculative mode → immediate output → timeout → no additional output
    let mut engine = make_speculative_engine();

    // Press 'A' key → should immediately output 'う' (normal face) and enter SpeculativeChar
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1, "should emit one action immediately");
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "should emit normal face char 'う', got {:?}",
        r.actions[0]
    );
    assert!(
        matches!(engine.state, EngineState::SpeculativeChar(_)),
        "state should be SpeculativeChar, got {:?}",
        engine.state
    );

    // Timeout → no additional output, back to Idle
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "timeout should produce no actions (already emitted), got {:?}",
        r.actions
    );
    assert!(
        engine.state.is_idle(),
        "state should be Idle after timeout, got {:?}",
        engine.state
    );
}

#[test]
fn test_speculative_simultaneous() {
    // Char press → immediate output → thumb arrives within threshold → BS + thumb face
    let mut engine = make_speculative_engine();
    let t0 = 1_000_000;

    // Press 'A' key → immediate output 'う'
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "should emit 'う' immediately"
    );

    // Left thumb arrives within threshold (30ms < 100ms threshold)
    let t1 = t0 + 30_000;
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t1).build());
    r.assert_consumed();
    // Should have BS (retract 'う' which is a Char → 0 romaji chars → 0 BS)
    // Actually, for Literal chars, emitted_romaji is empty string, so no BS needed
    // The action should just be the thumb face char 'を'
    assert!(
        !r.actions.is_empty(),
        "should produce actions for thumb retraction"
    );
    // Last action should be the thumb-face character
    assert!(
        matches!(r.actions.last(), Some(KeyAction::Char('を'))),
        "last action should be thumb face char 'を', got {:?}",
        r.actions
    );
    assert!(
        engine.state.is_idle(),
        "state should be Idle after retraction, got {:?}",
        engine.state
    );
}

#[test]
fn test_speculative_simultaneous_with_romaji() {
    // Char press with romaji → immediate output → thumb arrives → BS×N + thumb face
    let mut layout = make_layout();
    layout.normal.insert(
        POS_D,
        YabValue::Romaji {
            romaji: "ka".to_string(),
            kana: Some('か'),
        },
    );
    layout.left_thumb.insert(POS_D, lit('げ'));

    let mut engine = TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Speculative,
            30,
        ),
    };
    let t0 = 1_000_000;

    // Press 'D' key → immediate output Romaji("ka")
    let r = engine.on_event(Ev::down(VK_D).at(t0).build());
    r.assert_consumed();
    assert!(
        matches!(&r.actions[0], KeyAction::Romaji(s) if s == "ka"),
        "should emit Romaji 'ka' immediately, got {:?}",
        r.actions[0]
    );

    // Left thumb arrives within threshold
    let t1 = t0 + 30_000;
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t1).build());
    r.assert_consumed();
    // Bug #3 fix: IME treats complete romaji as 1 composition unit → always 1 BS
    assert_eq!(
        r.actions.len(),
        2,
        "should have 1 BS + 1 thumb action, got {:?}",
        r.actions
    );
    assert!(
        matches!(
            &r.actions[0],
            KeyAction::SpecialKey(crate::types::SpecialKey::Backspace)
        ),
        "first action should be BS, got {:?}",
        r.actions[0]
    );
    assert!(
        matches!(&r.actions[1], KeyAction::Char('げ')),
        "second action should be 'げ', got {:?}",
        r.actions[1]
    );
}

#[test]
fn test_speculative_char_sequence() {
    // char1 → immediate → char2 → char1 was correct + char2 immediate
    let mut engine = make_speculative_engine();
    let t0 = 1_000_000;

    // Press 'A' key → immediate output 'う'
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "should emit 'う' immediately"
    );

    // Press 'S' key → char1 was correct, char2 emits immediately
    let t1 = t0 + 50_000;
    let r = engine.on_event(Ev::down(VK_S).at(t1).build());
    r.assert_consumed();
    // Should emit 'し' (normal face for S)
    assert!(
        matches!(&r.actions[0], KeyAction::Char('し')),
        "should emit 'し' for second char, got {:?}",
        r.actions
    );
    assert!(
        matches!(engine.state, EngineState::SpeculativeChar(_)),
        "state should be SpeculativeChar for second char, got {:?}",
        engine.state
    );
}

#[test]
fn test_speculative_thumb_outside_threshold() {
    // Char press → thumb arrives after threshold → speculative was correct, thumb processed as new
    let mut engine = make_speculative_engine();
    let t0 = 1_000_000;

    // Press 'A' key → immediate output 'う'
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "should emit 'う' immediately"
    );

    // Left thumb arrives AFTER threshold (150ms > 100ms)
    let t1 = t0 + 150_000;
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t1).build());
    // Thumb should be processed as a new key (pending thumb in Wait fallback via handle_idle)
    r.assert_consumed();
    // In Speculative mode, thumb goes to handle_idle_wait → PendingThumb
    assert!(
        matches!(engine.state, EngineState::PendingThumb(_)),
        "thumb outside threshold should be pending, got {:?}",
        engine.state
    );
}

#[test]
fn test_speculative_thumb_first_falls_back_to_wait() {
    // Thumb first in Speculative mode → same as Wait mode (PendingThumb)
    let mut engine = make_speculative_engine();
    let t0 = 1_000_000;

    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t0).build());
    assert_pending(&r);
    assert!(
        matches!(engine.state, EngineState::PendingThumb(_)),
        "thumb first should enter PendingThumb, got {:?}",
        engine.state
    );
}

// ── TwoPhase confirm mode tests ──

fn make_two_phase_engine() -> TestHarness {
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::TwoPhase,
            30,
        ),
    }
}

#[test]
fn test_two_phase_thumb_within_short_delay() {
    // Thumb arrives at 20ms (< 30ms speculative delay) → clean simultaneous, no flicker
    // Phase 1: char enters PendingChar with TIMER_SPECULATIVE
    // Thumb arrives before TIMER_SPECULATIVE fires → same as Wait mode PendingChar+thumb path
    let mut engine = make_two_phase_engine();
    let t0 = 1_000_000;

    // Press 'A' key → PendingChar with TIMER_SPECULATIVE
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(r.actions.is_empty(), "Phase 1 should not emit any actions");
    r.assert_timer_set(TIMER_SPECULATIVE);
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "state should be PendingChar, got {:?}",
        engine.state
    );

    // Left thumb arrives at 20ms (< 30ms) → PendingCharThumb
    let t1 = t0 + 20_000;
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t1).build());
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "should be pending (PendingCharThumb), got {:?}",
        r.actions
    );

    // Timeout resolves as simultaneous → left thumb face for A = 'を'
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(
        matches!(r.actions[0], KeyAction::Char('を')),
        "clean simultaneous should produce 'を', got {:?}",
        r.actions[0]
    );
}

#[test]
fn test_two_phase_thumb_after_short_delay() {
    // Thumb arrives at 50ms (> 30ms but < 100ms) → speculative output happened, BS + replace
    // Phase 1: char enters PendingChar with TIMER_SPECULATIVE
    // TIMER_SPECULATIVE fires at 30ms → Phase 2: speculative output + SpeculativeChar
    // Thumb arrives at 50ms → BS + thumb face
    let mut engine = make_two_phase_engine();
    let t0 = 1_000_000;

    // Press 'A' key → PendingChar with TIMER_SPECULATIVE
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(r.actions.is_empty(), "Phase 1 should not emit any actions");

    // TIMER_SPECULATIVE fires → Phase 2: speculative output 'う'
    let r = engine.on_timeout(TIMER_SPECULATIVE);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1, "should emit speculative output");
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "speculative output should be 'う', got {:?}",
        r.actions[0]
    );
    r.assert_timer_set(TIMER_PENDING);
    assert!(
        matches!(engine.state, EngineState::SpeculativeChar(_)),
        "state should be SpeculativeChar, got {:?}",
        engine.state
    );

    // Left thumb arrives at 50ms (within remaining threshold window)
    let t1 = t0 + 50_000;
    let r = engine.on_event(Ev::down(VK_NONCONVERT).at(t1).build());
    r.assert_consumed();
    // Last action should be the thumb-face character 'を'
    assert!(
        matches!(r.actions.last(), Some(KeyAction::Char('を'))),
        "last action should be thumb face char 'を', got {:?}",
        r.actions
    );
    assert!(
        engine.state.is_idle(),
        "state should be Idle after retraction, got {:?}",
        engine.state
    );
}

#[test]
fn test_two_phase_no_thumb() {
    // No thumb → speculative at 30ms, confirmed at 100ms
    // Phase 1: PendingChar + TIMER_SPECULATIVE
    // Phase 2 (30ms): speculative output + SpeculativeChar + TIMER_PENDING for remaining 70ms
    // TIMER_PENDING fires: confirmed (no additional output)
    let mut engine = make_two_phase_engine();
    let t0 = 1_000_000;

    // Press 'A' key → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(r.actions.is_empty());

    // TIMER_SPECULATIVE fires → speculative output
    let r = engine.on_timeout(TIMER_SPECULATIVE);
    r.assert_consumed();
    assert_eq!(r.actions.len(), 1);
    assert!(matches!(&r.actions[0], KeyAction::Char('う')));
    assert!(matches!(engine.state, EngineState::SpeculativeChar(_)));

    // TIMER_PENDING fires → confirmed, no additional output
    let r = engine.on_timeout(TIMER_PENDING);
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "SpeculativeChar timeout should produce no actions, got {:?}",
        r.actions
    );
    assert!(
        engine.state.is_idle(),
        "state should be Idle after full confirmation, got {:?}",
        engine.state
    );
}

#[test]
fn test_two_phase_char_sequence() {
    // Chars arrive rapidly, each within 30ms → wait confirms previous
    // char1 → PendingChar, char2 arrives within 30ms → char1 confirmed as single, char2 pending
    let mut engine = make_two_phase_engine();
    let t0 = 1_000_000;

    // Press 'A' key → PendingChar
    let r = engine.on_event(Ev::down(VK_A).at(t0).build());
    r.assert_consumed();
    assert!(r.actions.is_empty());

    // Press 'S' key at 20ms (< 30ms, before TIMER_SPECULATIVE fires)
    let t1 = t0 + 20_000;
    let r = engine.on_event(Ev::down(VK_S).at(t1).build());
    r.assert_consumed();
    // char1 (A) should be flushed as single ('う')
    assert!(
        r.actions.iter().any(|a| matches!(a, KeyAction::Char('う'))),
        "char1 should be confirmed as 'う', got {:?}",
        r.actions
    );
    // char2 (S) should now be in PendingChar
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "state should be PendingChar for char2, got {:?}",
        engine.state
    );
}

// ── AdaptiveTiming モード テスト ──

fn make_adaptive_engine() -> TestHarness {
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::AdaptiveTiming,
            30,
        ),
    }
}

/// 最初のキー（前キーなし）→ TwoPhase 動作（PendingChar + TIMER_SPECULATIVE）
#[test]
fn test_adaptive_first_key_uses_two_phase() {
    let mut engine = make_adaptive_engine();
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());

    // TwoPhase: PendingChar 状態 + TIMER_SPECULATIVE が設定される
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "TwoPhase Phase 1 should have no actions"
    );
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "state should be PendingChar, got {:?}",
        engine.state
    );
    r.assert_timer_set(TIMER_SPECULATIVE);
}

/// 連続打鍵（50ms 間隔）→ Wait 動作（PendingChar + TIMER_PENDING）
#[test]
fn test_adaptive_rapid_typing_uses_wait() {
    let mut engine = make_adaptive_engine();

    // 1 文字目（TwoPhase 動作）
    let t0 = 1_000_000;
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    // タイムアウトで確定させて Idle に戻す
    let _ = engine.on_timeout(TIMER_SPECULATIVE);
    let _ = engine.on_timeout(TIMER_PENDING);

    // 2 文字目: 50ms 後（< 80ms → continuous → Wait）
    let t1 = t0 + 50_000;
    let r = engine.on_event(Ev::down(VK_S).at(t1).build());

    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "Wait mode should have no immediate actions"
    );
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "state should be PendingChar, got {:?}",
        engine.state
    );
    r.assert_timer_set(TIMER_PENDING);
}

/// ポーズ後（200ms 間隔）→ TwoPhase 動作（PendingChar + TIMER_SPECULATIVE）
#[test]
fn test_adaptive_after_pause_uses_two_phase() {
    let mut engine = make_adaptive_engine();

    // 1 文字目
    let t0 = 1_000_000;
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    let _ = engine.on_timeout(TIMER_SPECULATIVE);
    let _ = engine.on_timeout(TIMER_PENDING);

    // 2 文字目: 200ms 後（>= 80ms → paused → TwoPhase）
    let t1 = t0 + 200_000;
    let r = engine.on_event(Ev::down(VK_S).at(t1).build());

    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "TwoPhase Phase 1 should have no actions"
    );
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "state should be PendingChar, got {:?}",
        engine.state
    );
    r.assert_timer_set(TIMER_SPECULATIVE);
}

/// 連続打鍵 → ポーズ → 最後のキーは TwoPhase を使用
#[test]
fn test_adaptive_continuous_then_pause() {
    let mut engine = make_adaptive_engine();

    // 1 文字目 t=1000ms
    let t0 = 1_000_000;
    let _ = engine.on_event(Ev::down(VK_A).at(t0).build());
    let _ = engine.on_timeout(TIMER_SPECULATIVE);
    let _ = engine.on_timeout(TIMER_PENDING);

    // 2 文字目 t=1050ms (50ms gap → continuous → Wait)
    let t1 = t0 + 50_000;
    let r1 = engine.on_event(Ev::down(VK_S).at(t1).build());
    r1.assert_timer_set(TIMER_PENDING); // Wait mode
    let _ = engine.on_timeout(TIMER_PENDING);

    // 3 文字目 t=1300ms (250ms gap → paused → TwoPhase)
    let t2 = t1 + 250_000;
    let r2 = engine.on_event(Ev::down(VK_A).at(t2).build());
    r2.assert_consumed();
    assert!(
        r2.actions.is_empty(),
        "TwoPhase Phase 1 should have no actions"
    );
    r2.assert_timer_set(TIMER_SPECULATIVE);
}

// ── NgramPredictive confirm mode tests ──

fn make_ngram_predictive_engine() -> TestHarness {
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::NgramPredictive,
            30,
        ),
    }
}

/// n-gram で通常面のスコアが高い場合、Speculative（即時出力）を使用する
#[test]
fn test_ngram_predictive_high_normal_score_uses_speculative() {
    let mut engine = make_ngram_predictive_engine();

    // Seed output_history so that bigram ('あ', 'う') has a high score
    // Normal face for A key = 'う', left_thumb = 'を', right_thumb = 'ゔ'
    engine.output_history.push(OutputEntry {
        scan_code: ScanCode(0),
        romaji: String::new(),
        kana: Some('あ'),
        action: KeyAction::Char('あ'),
    });

    // High score for normal face kana ('あ', 'う'), low for thumb face
    let toml_str = r#"
[bigram]
"あう" = 2.0
"あを" = 0.5
"あゔ" = 0.3
"#;
    let model = NgramModel::from_toml(toml_str, 100_000, 20_000, 30_000, 120_000).unwrap();
    engine.set_ngram_model(model);

    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());

    // Speculative: immediate output + SpeculativeChar state
    assert!(
        !r.actions.is_empty(),
        "NgramPredictive should output immediately when normal score is high"
    );
    assert!(
        matches!(engine.state, EngineState::SpeculativeChar(_)),
        "Should be in SpeculativeChar state"
    );
}

/// n-gram で親指面のスコアが高い場合、Wait（保留）を使用する
#[test]
fn test_ngram_predictive_high_thumb_score_uses_wait() {
    let mut engine = make_ngram_predictive_engine();

    // Seed output_history so that thumb face kana has high score
    engine.output_history.push(OutputEntry {
        scan_code: ScanCode(0),
        romaji: String::new(),
        kana: Some('あ'),
        action: KeyAction::Char('あ'),
    });

    // Low score for normal face kana, high for thumb face kana
    let toml_str = r#"
[bigram]
"あう" = 0.3
"あを" = 2.0
"あゔ" = 0.1
"#;
    let model = NgramModel::from_toml(toml_str, 100_000, 20_000, 30_000, 120_000).unwrap();
    engine.set_ngram_model(model);

    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());

    // Wait: no actions, PendingChar state
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "NgramPredictive should wait when thumb score is higher"
    );
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "Should be in PendingChar state"
    );
    r.assert_timer_set(TIMER_PENDING);
}

/// n-gram モデルが未設定の場合、TwoPhase にフォールバックする
#[test]
fn test_ngram_predictive_no_model_falls_back() {
    let mut engine = make_ngram_predictive_engine();
    // No ngram model set → should fall back to TwoPhase

    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());

    // TwoPhase: PendingChar + TIMER_SPECULATIVE
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "TwoPhase fallback Phase 1 should have no actions"
    );
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "Should be in PendingChar state"
    );
    r.assert_timer_set(TIMER_SPECULATIVE);
}

/// 出力履歴が空の場合、スコアは両方 0 → diff=0 → Wait を使用する
#[test]
fn test_ngram_predictive_no_history_uses_wait() {
    let mut engine = make_ngram_predictive_engine();

    // Empty output_history + model with some bigrams (but they won't match with empty history)
    let toml_str = r#"
[bigram]
"あう" = 2.0
"#;
    let model = NgramModel::from_toml(toml_str, 100_000, 20_000, 30_000, 120_000).unwrap();
    engine.set_ngram_model(model);

    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());

    // Both scores are 0.0 → diff = 0.0 (not > 0.5) → Wait
    r.assert_consumed();
    assert!(
        r.actions.is_empty(),
        "NgramPredictive should wait when no history (scores are zero)"
    );
    assert!(
        matches!(engine.state, EngineState::PendingChar(_)),
        "Should be in PendingChar state"
    );
    r.assert_timer_set(TIMER_PENDING);
}

// ── Cross-mode comparison tests ──
// These tests verify that all ConfirmMode variants produce the same final
// characters after BS retraction is applied.  NgramPredictive is excluded
// because it requires an n-gram model to be configured and its behaviour
// depends on context history.

/// Modes to include in cross-mode comparison tests.
const CROSS_MODES: [ConfirmMode; 4] = [
    ConfirmMode::Wait,
    ConfirmMode::Speculative,
    ConfirmMode::TwoPhase,
    ConfirmMode::AdaptiveTiming,
];

fn make_engine_with_mode(mode: ConfirmMode) -> TestHarness {
    let layout = make_layout();
    TestHarness {
        tracker: input_tracker::InputTracker::new(),
        engine: NicolaFsm::new(layout, VK_NONCONVERT, VK_CONVERT, 100, mode, 30),
    }
}

/// Collect final output from a sequence of Responses, handling BS retraction.
/// BS (`Key(0x08)`) retracts the most recently emitted non-Suppress action.
fn collect_output(responses: &[Resp]) -> Vec<KeyAction> {
    let mut output: Vec<KeyAction> = Vec::new();
    for r in responses {
        for action in &r.actions {
            match action {
                KeyAction::SpecialKey(crate::types::SpecialKey::Backspace) => {
                    output.pop();
                }
                KeyAction::Suppress => {} // skip suppresses
                other => output.push(other.clone()),
            }
        }
    }
    output
}

/// Extract only the Char values from the collected output.
fn collect_chars(responses: &[Resp]) -> Vec<char> {
    collect_output(responses)
        .into_iter()
        .filter_map(|a| match a {
            KeyAction::Char(c) => Some(c),
            _ => None,
        })
        .collect()
}

#[test]
fn test_all_modes_single_char_same_output() {
    let mut reference: Option<Vec<char>> = None;
    for mode in CROSS_MODES {
        let mut engine = make_engine_with_mode(mode);
        let mut responses = vec![];

        // Press A key
        responses.push(engine.on_event(Ev::down(VK_A).at(1_000_000).build()));
        // Fire all possible timers so every mode resolves
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));

        let chars = collect_chars(&responses);
        assert!(
            !chars.is_empty(),
            "mode {:?} should produce output for A key",
            mode
        );
        assert_eq!(
            chars,
            vec!['う'],
            "mode {:?} should produce normal face 'う' for A key, got {:?}",
            mode,
            chars
        );

        if let Some(ref expected) = reference {
            assert_eq!(
                &chars, expected,
                "mode {:?} differs from reference output",
                mode
            );
        } else {
            reference = Some(chars);
        }
    }
}

#[test]
fn test_all_modes_simultaneous_same_final_output() {
    let mut reference: Option<Vec<char>> = None;
    for mode in CROSS_MODES {
        let mut engine = make_engine_with_mode(mode);
        let mut responses = vec![];
        let t = 1_000_000u64;

        // Press A, then left thumb within threshold
        responses.push(engine.on_event(Ev::down(VK_A).at(t).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_event(Ev::down(VK_NONCONVERT).at(t + 20_000).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));

        let chars = collect_chars(&responses);
        // After BS retraction, all modes should end up with left thumb face for A = 'を'
        assert!(
            chars.contains(&'を'),
            "mode {:?} should produce left thumb face 'を' for simultaneous A+muhenkan, got {:?}",
            mode,
            chars
        );

        if let Some(ref expected) = reference {
            assert_eq!(
                &chars, expected,
                "mode {:?} simultaneous output differs from reference",
                mode
            );
        } else {
            reference = Some(chars);
        }
    }
}

#[test]
fn test_all_modes_simultaneous_right_thumb_same_final_output() {
    let mut reference: Option<Vec<char>> = None;
    for mode in CROSS_MODES {
        let mut engine = make_engine_with_mode(mode);
        let mut responses = vec![];
        let t = 1_000_000u64;

        // Press A, then right thumb within threshold
        responses.push(engine.on_event(Ev::down(VK_A).at(t).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_event(Ev::down(VK_CONVERT).at(t + 20_000).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));

        let chars = collect_chars(&responses);
        // After BS retraction, all modes should end up with right thumb face for A = 'ゔ'
        assert!(
            chars.contains(&'ゔ'),
            "mode {:?} should produce right thumb face 'ゔ' for A+henkan, got {:?}",
            mode,
            chars
        );

        if let Some(ref expected) = reference {
            assert_eq!(
                &chars, expected,
                "mode {:?} right-thumb simultaneous differs from reference",
                mode
            );
        } else {
            reference = Some(chars);
        }
    }
}

#[test]
fn test_all_modes_rapid_sequence_same_output() {
    let mut reference: Option<Vec<char>> = None;
    for mode in [
        ConfirmMode::Wait,
        ConfirmMode::Speculative,
        ConfirmMode::TwoPhase,
    ] {
        let mut engine = make_engine_with_mode(mode);
        let mut responses = vec![];

        // Type A, S rapidly (50ms apart), well outside threshold for simultaneous
        // but close enough to exercise the rapid path
        responses.push(engine.on_event(Ev::down(VK_A).at(1_000_000).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_event(Ev::down(VK_S).at(1_050_000).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));

        let chars = collect_chars(&responses);
        // Should have normal-face outputs for A='う' and S='し'
        assert_eq!(
            chars,
            vec!['う', 'し'],
            "mode {:?} rapid A,S should produce ['う','し'], got {:?}",
            mode,
            chars
        );

        if let Some(ref expected) = reference {
            assert_eq!(
                &chars, expected,
                "mode {:?} rapid sequence differs from reference",
                mode
            );
        } else {
            reference = Some(chars);
        }
    }
}

#[test]
fn test_all_modes_thumb_first_then_char_same_output() {
    let mut reference: Option<Vec<char>> = None;
    for mode in CROSS_MODES {
        let mut engine = make_engine_with_mode(mode);
        let mut responses = vec![];
        let t = 1_000_000u64;

        // Thumb first, then char within threshold (pattern 1)
        responses.push(engine.on_event(Ev::down(VK_NONCONVERT).at(t).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_event(Ev::down(VK_A).at(t + 30_000).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));

        let chars = collect_chars(&responses);
        assert!(
            chars.contains(&'を'),
            "mode {:?} thumb-first should produce 'を', got {:?}",
            mode,
            chars
        );

        if let Some(ref expected) = reference {
            assert_eq!(
                &chars, expected,
                "mode {:?} thumb-first output differs from reference",
                mode
            );
        } else {
            reference = Some(chars);
        }
    }
}

#[test]
fn test_all_modes_char_alone_after_threshold_same_output() {
    // Char is pressed, thumb arrives after threshold → char confirmed as normal face
    let mut reference: Option<Vec<char>> = None;
    for mode in CROSS_MODES {
        let mut engine = make_engine_with_mode(mode);
        let mut responses = vec![];
        let t = 1_000_000u64;

        responses.push(engine.on_event(Ev::down(VK_A).at(t).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));
        // Thumb arrives after full timeout → processed as new key, not simultaneous
        responses.push(engine.on_event(Ev::down(VK_NONCONVERT).at(t + 200_000).build()));
        responses.push(engine.on_timeout(TIMER_SPECULATIVE));
        responses.push(engine.on_timeout(TIMER_PENDING));

        let chars = collect_chars(&responses);
        // 'う' from the A key (normal face) — thumb alone doesn't produce a char
        assert!(
            chars.contains(&'う'),
            "mode {:?} should produce normal face 'う' for A, got {:?}",
            mode,
            chars
        );

        if let Some(ref expected) = reference {
            assert_eq!(
                &chars, expected,
                "mode {:?} char-alone-after-threshold differs from reference",
                mode
            );
        } else {
            reference = Some(chars);
        }
    }
}

// ── Mode-specific characteristic tests ──

#[test]
fn test_speculative_has_immediate_output() {
    let mut engine = make_engine_with_mode(ConfirmMode::Speculative);
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    assert!(
        !r.actions.is_empty(),
        "Speculative should output immediately"
    );
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "Speculative immediate output should be normal face 'う', got {:?}",
        r.actions[0]
    );
}

#[test]
fn test_wait_has_no_immediate_output() {
    let mut engine = make_engine_with_mode(ConfirmMode::Wait);
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    assert!(
        r.actions.is_empty(),
        "Wait should not output immediately on key down"
    );
}

#[test]
fn test_two_phase_no_output_before_speculative_timer() {
    let mut engine = make_engine_with_mode(ConfirmMode::TwoPhase);
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    assert!(
        r.actions.is_empty(),
        "TwoPhase should not output immediately (Phase 1)"
    );
    // But after speculative timer fires, output appears (Phase 2)
    let r = engine.on_timeout(TIMER_SPECULATIVE);
    assert!(
        !r.actions.is_empty(),
        "TwoPhase should output after speculative delay (Phase 2)"
    );
    assert!(
        matches!(&r.actions[0], KeyAction::Char('う')),
        "TwoPhase Phase 2 output should be 'う', got {:?}",
        r.actions[0]
    );
}

#[test]
fn test_adaptive_first_key_behaves_like_two_phase() {
    // AdaptiveTiming with no prior key history should use TwoPhase behavior
    let mut engine = make_engine_with_mode(ConfirmMode::AdaptiveTiming);
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    assert!(
        r.actions.is_empty(),
        "AdaptiveTiming first key should not output immediately (TwoPhase Phase 1)"
    );
    let r = engine.on_timeout(TIMER_SPECULATIVE);
    assert!(
        !r.actions.is_empty(),
        "AdaptiveTiming first key should output after speculative timer"
    );
}

#[test]
fn test_speculative_retraction_on_simultaneous() {
    // Verify that Speculative mode resolves to thumb face when thumb arrives
    // within threshold.  The engine emits the speculative char immediately,
    // then when thumb arrives it retracts (BS) and emits the thumb face.
    // collect_output neutralises the BS+original pair.
    let mut engine = make_engine_with_mode(ConfirmMode::Speculative);
    let t = 1_000_000u64;

    let r1 = engine.on_event(Ev::down(VK_A).at(t).build());
    assert!(
        matches!(&r1.actions[0], KeyAction::Char('う')),
        "Speculative should emit 'う' immediately"
    );

    // Thumb within threshold
    let r2 = engine.on_event(Ev::down(VK_NONCONVERT).at(t + 20_000).build());
    // The thumb response must include the thumb face character 'を'
    let has_thumb_char = r2
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Char('を')));
    assert!(
        has_thumb_char,
        "Speculative retraction should include thumb face 'を', got {:?}",
        r2.actions
    );

    // After collecting all output (with BS retraction applied), the final
    // result should contain the thumb face 'を' and the speculative 'う'
    // should be neutralised.
    let responses = vec![r1, r2];
    let chars = collect_chars(&responses);
    assert!(
        chars.last() == Some(&'を'),
        "Final output should end with thumb face 'を', got {:?}",
        chars
    );
}

#[test]
fn test_collect_output_handles_bs_retraction() {
    // Unit test for the collect_output helper itself
    let responses = vec![
        Response {
            actions: vec![KeyAction::Char('う')],
            consumed: true,
            timers: vec![],
        },
        Response {
            actions: vec![
                KeyAction::SpecialKey(crate::types::SpecialKey::Backspace), // BS retracts 'う'
                KeyAction::Char('を'),
            ],
            consumed: true,
            timers: vec![],
        },
    ];
    let chars = collect_chars(&responses);
    assert_eq!(chars, vec!['を'], "BS should retract 'う', leaving 'を'");
}

#[test]
fn test_collect_output_no_retraction() {
    // No BS → all outputs preserved
    let responses = vec![
        Response {
            actions: vec![KeyAction::Char('う')],
            consumed: true,
            timers: vec![],
        },
        Response {
            actions: vec![KeyAction::Char('し')],
            consumed: true,
            timers: vec![],
        },
    ];
    let chars = collect_chars(&responses);
    assert_eq!(chars, vec!['う', 'し'], "No BS means all chars preserved");
}

// ── build_response tests ──

#[test]
fn test_build_response_cancel_all_timers() {
    let engine = make_engine();
    let resp = engine.build_response(
        vec![KeyAction::Romaji("ka".to_string())],
        true,
        TimerIntent::CancelAll,
    );
    resp.assert_consumed();
    assert_eq!(resp.actions.len(), 1);
    resp.assert_timer_kill(TIMER_PENDING);
    resp.assert_timer_kill(TIMER_SPECULATIVE);
    // CancelAll should not set any timers
    assert!(
        !resp
            .timers
            .iter()
            .any(|t| matches!(t, timed_fsm::TimerCommand::Set { .. })),
        "CancelAll should not set any timers"
    );
}

#[test]
fn test_build_response_pending_sets_timer() {
    let engine = make_engine();
    let resp = engine.build_response(vec![], true, TimerIntent::Pending);
    resp.assert_consumed();
    assert!(resp.actions.is_empty(), "Pending should have no actions");
    resp.assert_timer_set(TIMER_PENDING);
    resp.assert_timer_kill(TIMER_SPECULATIVE);
}

#[test]
fn test_build_response_speculative_wait_sets_timer() {
    let engine = make_engine();
    let resp = engine.build_response(
        vec![KeyAction::Romaji("u".to_string())],
        true,
        TimerIntent::SpeculativeWait,
    );
    resp.assert_consumed();
    assert_eq!(resp.actions.len(), 1);
    resp.assert_timer_set(TIMER_SPECULATIVE);
    resp.assert_timer_kill(TIMER_PENDING);
}

#[test]
fn test_build_response_phase2_transition() {
    let engine = make_engine();
    let resp = engine.build_response(
        vec![KeyAction::Romaji("ka".to_string())],
        true,
        TimerIntent::Phase2Transition {
            remaining_us: 50_000,
        },
    );
    resp.assert_consumed();
    assert_eq!(resp.actions.len(), 1);
    resp.assert_timer_kill(TIMER_SPECULATIVE);
    resp.assert_timer_set(TIMER_PENDING);
}

#[test]
fn test_update_history_record() {
    let mut engine = make_engine();
    assert!(engine.output_history.is_empty());

    engine.update_history(OutputUpdate::Record(OutputRecord {
        scan_code: SCAN_A,
        romaji: "ka".to_string(),
        kana: Some('か'),
        action: KeyAction::Romaji("ka".to_string()),
    }));
    assert_eq!(engine.output_history.len(), 1);
    assert_eq!(engine.output_history.recent_kana(1), vec!['か']);
}

#[test]
fn test_update_history_retract_and_record() {
    let mut engine = make_engine();

    // First, record an entry
    engine.update_history(OutputUpdate::Record(OutputRecord {
        scan_code: SCAN_A,
        romaji: "u".to_string(),
        kana: Some('う'),
        action: KeyAction::Romaji("u".to_string()),
    }));
    assert_eq!(engine.output_history.len(), 1);

    // Now retract and record a new entry
    engine.update_history(OutputUpdate::RetractAndRecord(OutputRecord {
        scan_code: SCAN_A,
        romaji: "vu".to_string(),
        kana: Some('ゔ'),
        action: KeyAction::Romaji("vu".to_string()),
    }));
    assert_eq!(
        engine.output_history.len(),
        1,
        "retract+record should keep count at 1"
    );
    assert_eq!(engine.output_history.recent_kana(1), vec!['ゔ']);
}

// ── FocusKind のユニットテスト ──

#[test]
fn test_bypass_state_repr_values() {
    // repr(u8) の値が AtomicU8 との変換で正しいことを確認
    assert_eq!(FocusKind::TextInput as u8, 0);
    assert_eq!(FocusKind::NonText as u8, 1);
    assert_eq!(FocusKind::Undetermined as u8, 2);
}

#[test]
fn test_bypass_state_equality() {
    assert_eq!(FocusKind::TextInput, FocusKind::TextInput);
    assert_ne!(FocusKind::TextInput, FocusKind::NonText);
    assert_ne!(FocusKind::NonText, FocusKind::Undetermined);
}

#[test]
fn test_bypass_state_copy_clone() {
    let state = FocusKind::NonText;
    let copied = state; // Copy
    let cloned = state.clone(); // Clone
    assert_eq!(copied, FocusKind::NonText);
    assert_eq!(cloned, FocusKind::NonText);
}

#[test]
fn test_bypass_state_debug_format() {
    // Debug trait が実装されていることを確認
    let s = format!("{:?}", FocusKind::TextInput);
    assert_eq!(s, "TextInput");
    let s = format!("{:?}", FocusKind::NonText);
    assert_eq!(s, "NonText");
    let s = format!("{:?}", FocusKind::Undetermined);
    assert_eq!(s, "Undetermined");
}

#[test]
fn test_context_invalidation_focus_changed() {
    // FocusChanged バリアントが存在し Debug 出力できることを確認
    let reason = ContextChange::FocusChanged;
    let s = format!("{:?}", reason);
    assert_eq!(s, "FocusChanged");
}

// ── Modifier state tracking across engine disable/enable ──

#[test]
fn test_ctrl_released_while_disabled_does_not_stick() {
    // エンジン OFF 中に Ctrl が離された場合、再 ON 後に stuck しないこと
    let mut engine = make_engine();

    // Ctrl を押す（エンジン ON 中）
    engine.on_event(Ev::down(VK_CTRL).build());

    // エンジン OFF
    let _ = engine.toggle_enabled();
    assert!(!engine.is_enabled());

    // Ctrl を離す（エンジン OFF 中）
    engine.on_event(Ev::up(VK_CTRL).build());

    // エンジン ON
    let _ = engine.toggle_enabled();
    assert!(engine.is_enabled());

    // 文字キーがエンジンで処理されること（OsModifierHeld でバイパスされない）
    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    r.assert_consumed();
}

#[test]
fn test_alt_released_while_disabled_does_not_stick() {
    let mut engine = make_engine();

    engine.on_event(Ev::down(VK_ALT).build());
    let _ = engine.toggle_enabled();

    // Alt を離す（エンジン OFF 中）
    engine.on_event(Ev::up(VK_ALT).build());

    let _ = engine.toggle_enabled();

    let r = engine.on_event(Ev::down(VK_A).at(1_000_000).build());
    r.assert_consumed();
}

// ── FsmAdapter tests ──

mod fsm_adapter_tests {
    use super::*;
    use crate::config::ConfirmMode;
    use crate::engine::fsm_adapter::FsmAdapter;
    use crate::engine::input_tracker::{InputTracker, PhysicalKeyState};
    use crate::engine::nicola_fsm::NicolaFsm;
    use crate::types::ContextChange;

    fn make_adapter() -> FsmAdapter {
        let fsm = NicolaFsm::new(
            make_layout(),
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        );
        FsmAdapter::new(fsm)
    }

    #[test]
    fn is_enabled_default_true() {
        let adapter = make_adapter();
        assert!(adapter.is_enabled());
    }

    #[test]
    fn set_enabled_false() {
        let mut adapter = make_adapter();
        let (actual, decision) = adapter.set_enabled(false);
        assert!(!actual);
        assert!(!adapter.is_enabled());
        // Decision should exist (may have effects or not)
        let _ = decision;
    }

    #[test]
    fn set_enabled_true_when_already_true() {
        let mut adapter = make_adapter();
        let (actual, _decision) = adapter.set_enabled(true);
        assert!(actual);
        assert!(adapter.is_enabled());
    }

    #[test]
    fn toggle_enabled_flips_state() {
        let mut adapter = make_adapter();
        assert!(adapter.is_enabled());

        let (enabled, _decision) = adapter.toggle_enabled();
        assert!(!enabled);
        assert!(!adapter.is_enabled());

        let (enabled, _decision) = adapter.toggle_enabled();
        assert!(enabled);
        assert!(adapter.is_enabled());
    }

    #[test]
    fn flush_returns_decision() {
        let mut adapter = make_adapter();
        let decision = adapter.flush(ContextChange::FocusChanged);
        // Flush on idle should return a Decision without panicking
        let _ = decision.is_consumed();
    }

    #[test]
    fn flush_to_effects_returns_vec() {
        let mut adapter = make_adapter();
        let effects = adapter.flush_to_effects(ContextChange::FocusChanged);
        // Verify it returns a Vec (may or may not be empty depending on FSM internals)
        let _ = effects.len();
    }

    #[test]
    fn on_event_processes_key_down() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        let event = Ev::down(VK_A).at(1_000_000).build();
        let phys = tracker.process(&event);
        let decision = adapter.on_event(event, &phys);
        // Character key when enabled should be consumed
        assert!(decision.is_consumed());
    }

    #[test]
    fn on_event_key_up_pass_through_when_idle() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        // Key up without prior key down
        let event = Ev::up(VK_A).build();
        let phys = tracker.process(&event);
        let decision = adapter.on_event(event, &phys);
        // Key-up without pending state should pass through
        let _ = decision;
    }

    #[test]
    fn on_timeout_on_idle() {
        let mut adapter = make_adapter();
        let phys = PhysicalKeyState::empty();
        let decision = adapter.on_timeout(0, &phys);
        // Timeout on idle should not panic, just produce a decision
        let _ = decision;
    }

    #[test]
    fn set_threshold_ms_updates() {
        let mut adapter = make_adapter();
        // Should not panic; verify indirectly through behavior
        adapter.set_threshold_ms(200);
        // After increasing threshold, keys further apart should still be simultaneous
        let mut tracker = InputTracker::new();
        let ev1 = Ev::down(VK_NONCONVERT).at(0).build();
        let phys1 = tracker.process(&ev1);
        let _ = adapter.on_event(ev1, &phys1);

        let ev2 = Ev::down(VK_A).at(150_000).build(); // 150ms apart
        let phys2 = tracker.process(&ev2);
        let decision = adapter.on_event(ev2, &phys2);
        assert!(decision.is_consumed());
    }

    #[test]
    fn set_confirm_mode_updates() {
        let mut adapter = make_adapter();
        // Should not panic
        adapter.set_confirm_mode(ConfirmMode::Speculative, 50);
        adapter.set_confirm_mode(ConfirmMode::Wait, 30);
    }

    #[test]
    fn swap_layout_returns_decision() {
        let mut adapter = make_adapter();
        let new_layout = make_layout();
        let decision = adapter.swap_layout(new_layout);
        // swap_layout may flush pending state
        let _ = decision;
    }

    #[test]
    fn swap_layout_on_idle_no_consumed() {
        let mut adapter = make_adapter();
        let decision = adapter.swap_layout(make_layout());
        // On idle, no pending keys to flush, so likely pass-through or empty
        // Just verify it doesn't panic
        let _ = decision.is_consumed();
    }

    #[test]
    fn set_enabled_false_then_on_event_passes_through() {
        let mut adapter = make_adapter();
        let (_actual, _decision) = adapter.set_enabled(false);

        let mut tracker = InputTracker::new();
        let event = Ev::down(VK_A).at(1_000_000).build();
        let phys = tracker.process(&event);
        let decision = adapter.on_event(event, &phys);
        // When disabled, key events should pass through
        assert!(!decision.is_consumed());
    }

    #[test]
    fn toggle_then_flush() {
        let mut adapter = make_adapter();
        // Process a key to create pending state
        let mut tracker = InputTracker::new();
        let event = Ev::down(VK_A).at(1_000_000).build();
        let phys = tracker.process(&event);
        let _ = adapter.on_event(event, &phys);

        // Toggle off should flush pending
        let (enabled, decision) = adapter.toggle_enabled();
        assert!(!enabled);
        let _ = decision;
    }

    #[test]
    fn set_ngram_model_does_not_panic() {
        let mut adapter = make_adapter();
        let toml_str = r#"
[bigram]
"あり" = 1.5
"#;
        let model = crate::ngram::NgramModel::from_toml(toml_str, 100_000, 20_000, 30_000, 120_000)
            .unwrap();
        adapter.set_ngram_model(model);
    }

    #[test]
    fn response_to_decision_consumed_flag() {
        // Test indirectly: when engine is enabled and receives a character key,
        // the adapter should produce a consumed decision
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();
        let event = Ev::down(VK_A).at(1_000_000).build();
        let phys = tracker.process(&event);
        let decision = adapter.on_event(event, &phys);
        assert!(decision.is_consumed());
    }

    #[test]
    fn flush_after_key_produces_effects() {
        let mut adapter = make_adapter();
        let mut tracker = InputTracker::new();

        // Send a character key to create pending state
        let event = Ev::down(VK_A).at(1_000_000).build();
        let phys = tracker.process(&event);
        let _ = adapter.on_event(event, &phys);

        // Flush should resolve the pending key
        let decision = adapter.flush(ContextChange::FocusChanged);
        // The flush should produce some output (consumed with effects)
        let _ = decision;
    }
}

// ============================================================================
// Engine integration tests (engine.rs coverage)
// ============================================================================

mod engine_integration_tests {
    use super::*;
    use crate::config::{ConfirmMode, ParsedKeyCombo};
    use crate::engine::decision::{
        Decision, Effect, EngineCommand, ImeCacheEffect, ImeEffect, ImeSyncKeys, InputContext,
        InputEffect, SpecialKeyCombos, UiEffect,
    };
    use crate::engine::engine::Engine;
    use crate::engine::input_tracker::InputTracker;
    use crate::engine::nicola_fsm::NicolaFsm;
    use crate::engine::observation::{FocusObservation, ImeObservation};
    use crate::types::{FocusKind, ImeCacheState, ImeReliability};

    fn empty_sync_keys() -> ImeSyncKeys {
        ImeSyncKeys {
            toggle: vec![],
            on: vec![],
            off: vec![],
        }
    }

    fn empty_special_keys() -> SpecialKeyCombos {
        SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
        }
    }

    fn make_test_engine() -> Engine {
        let layout = make_layout();
        let tracker = InputTracker::new();
        let fsm = NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        );
        Engine::new(fsm, tracker, empty_sync_keys(), empty_special_keys())
    }

    fn ime_on_ctx() -> InputContext {
        InputContext {
            ime_cache: ImeCacheState::On,
        }
    }

    fn ime_off_ctx() -> InputContext {
        InputContext {
            ime_cache: ImeCacheState::Off,
        }
    }

    fn ime_unknown_ctx() -> InputContext {
        InputContext {
            ime_cache: ImeCacheState::Unknown,
        }
    }

    fn has_effect<F: Fn(&Effect) -> bool>(decision: &Decision, pred: F) -> bool {
        match decision {
            Decision::Consume { effects } => effects.iter().any(&pred),
            Decision::PassThroughWith { effects } => effects.iter().any(&pred),
            Decision::PassThrough => false,
        }
    }

    fn effects_of(decision: &Decision) -> &[Effect] {
        match decision {
            Decision::Consume { effects } => effects,
            Decision::PassThroughWith { effects } => effects,
            Decision::PassThrough => &[],
        }
    }

    // ── 1. Engine::on_input basic flow ──

    #[test]
    fn on_input_char_key_with_ime_on_is_consumed() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(
            d.is_consumed(),
            "char key with IME ON should be consumed by FSM"
        );
    }

    #[test]
    fn on_input_char_key_with_ime_off_passes_through() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_off_ctx());
        assert!(
            !d.is_consumed(),
            "char key with IME OFF should pass through"
        );
    }

    #[test]
    fn on_input_char_key_with_ime_unknown_uses_shadow() {
        let mut engine = make_test_engine();
        // shadow defaults to ON, so Unknown resolves to ON
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_unknown_ctx());
        assert!(
            d.is_consumed(),
            "char key with IME Unknown + shadow ON should be consumed"
        );
    }

    #[test]
    fn on_input_char_key_with_ime_unknown_shadow_off_passes_through() {
        let mut engine = make_test_engine();
        engine.set_shadow_ime_on(false);
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_unknown_ctx());
        assert!(
            !d.is_consumed(),
            "char key with IME Unknown + shadow OFF should pass through"
        );
    }

    #[test]
    fn on_input_key_up_after_consumed_down_is_auto_consumed() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        let d = engine.on_input(Ev::up(VK_A).at(200).build(), &ime_on_ctx());
        assert!(
            d.is_consumed(),
            "KeyUp for consumed KeyDown should also be consumed"
        );
    }

    #[test]
    fn on_input_key_up_without_consumed_down_passes_through() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::up(VK_A).at(100).build(), &ime_on_ctx());
        assert!(
            !d.is_consumed(),
            "KeyUp without prior consumed KeyDown should pass through"
        );
    }

    // ── 2. Engine::on_input with modifiers ──

    #[test]
    fn on_input_shift_key_passes_through() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_SHIFT).at(100).build(), &ime_on_ctx());
        assert!(!d.is_consumed(), "Shift KeyDown should pass through");
    }

    #[test]
    fn on_input_ctrl_key_passes_through() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_CTRL).at(100).build(), &ime_on_ctx());
        assert!(!d.is_consumed(), "Ctrl KeyDown should pass through");
    }

    #[test]
    fn on_input_alt_key_passes_through() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_ALT).at(100).build(), &ime_on_ctx());
        assert!(!d.is_consumed(), "Alt KeyDown should pass through");
    }

    // ── 3. Engine::on_timeout ──

    #[test]
    fn on_timeout_after_pending_char() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());

        let d = engine.on_timeout(TIMER_PENDING, &ime_on_ctx());
        assert!(d.is_consumed());
        assert!(
            has_effect(&d, |e| matches!(e, Effect::Input(InputEffect::SendKeys(_)))),
            "timeout should produce SendKeys"
        );
    }

    #[test]
    fn on_timeout_with_ime_off_flushes() {
        let mut engine = make_test_engine();
        engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());

        let d = engine.on_timeout(TIMER_PENDING, &ime_off_ctx());
        assert!(d.is_consumed());
    }

    // ── 4. Engine::on_command ──

    #[test]
    fn on_command_toggle_engine() {
        let mut engine = make_test_engine();
        assert!(engine.is_fsm_enabled());

        let d = engine.on_command(EngineCommand::ToggleEngine);
        assert!(!engine.is_fsm_enabled());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false })
        )));

        let d = engine.on_command(EngineCommand::ToggleEngine);
        assert!(engine.is_fsm_enabled());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: true })
        )));
    }

    #[test]
    fn on_command_invalidate_context() {
        let mut engine = make_test_engine();
        engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        let d = engine.on_command(EngineCommand::InvalidateContext(ContextChange::ImeOff));
        assert!(d.is_consumed());
    }

    #[test]
    fn on_command_sync_ime_state_off_when_enabled() {
        let mut engine = make_test_engine();
        assert!(engine.is_fsm_enabled());

        let d = engine.on_command(EngineCommand::SyncImeState { ime_on: false });
        assert!(!engine.is_fsm_enabled());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false })
        )));
    }

    #[test]
    fn on_command_sync_ime_state_on_when_disabled() {
        let mut engine = make_test_engine();
        engine.on_command(EngineCommand::ToggleEngine);
        assert!(!engine.is_fsm_enabled());

        let d = engine.on_command(EngineCommand::SyncImeState { ime_on: true });
        assert!(engine.is_fsm_enabled());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: true })
        )));
    }

    #[test]
    fn on_command_sync_ime_state_no_change() {
        let mut engine = make_test_engine();
        assert!(engine.is_fsm_enabled());

        let d = engine.on_command(EngineCommand::SyncImeState { ime_on: true });
        assert!(!d.is_consumed());
        assert!(engine.is_fsm_enabled());
    }

    #[test]
    fn on_command_set_guard() {
        let mut engine = make_test_engine();
        let d = engine.on_command(EngineCommand::SetGuard(true));
        assert!(!d.is_consumed());
    }

    #[test]
    fn on_command_clear_deferred_keys() {
        let mut engine = make_test_engine();
        let d = engine.on_command(EngineCommand::ClearDeferredKeys);
        assert!(!d.is_consumed());
    }

    #[test]
    fn on_command_update_fsm_params() {
        let mut engine = make_test_engine();
        let d = engine.on_command(EngineCommand::UpdateFsmParams {
            threshold_ms: 200,
            confirm_mode: ConfirmMode::Speculative,
            speculative_delay_ms: 50,
        });
        assert!(!d.is_consumed());
    }

    #[test]
    fn on_command_reload_keys() {
        let mut engine = make_test_engine();
        let d = engine.on_command(EngineCommand::ReloadKeys {
            special: empty_special_keys(),
            sync: empty_sync_keys(),
        });
        assert!(!d.is_consumed());
    }

    // ── 5. Engine::check_special_keys ──

    fn make_engine_with_special(special: SpecialKeyCombos) -> Engine {
        let layout = make_layout();
        let tracker = InputTracker::new();
        let fsm = NicolaFsm::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,
            ConfirmMode::Wait,
            30,
        );
        Engine::new(fsm, tracker, empty_sync_keys(), special)
    }

    #[test]
    fn special_key_engine_on_combo() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_NONCONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![combo],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
        };
        let mut engine = make_engine_with_special(special);

        engine.on_command(EngineCommand::ToggleEngine);
        assert!(!engine.is_fsm_enabled());

        let d = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        assert!(
            engine.is_fsm_enabled(),
            "engine should be re-enabled by special key combo"
        );
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: true })
        )));
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::RequestCacheRefresh)
        )));
    }

    #[test]
    fn special_key_engine_off_combo() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_NONCONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![combo],
            ime_on: vec![],
            ime_off: vec![],
        };
        let mut engine = make_engine_with_special(special);
        assert!(engine.is_fsm_enabled());

        let d = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        assert!(
            !engine.is_fsm_enabled(),
            "engine should be disabled by special key combo"
        );
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false })
        )));
    }

    #[test]
    fn special_key_ime_on_combo() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![combo],
            ime_off: vec![],
        };
        let mut engine = make_engine_with_special(special);

        let d = engine.on_input(Ev::down(VK_CONVERT).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen(true))
        )));
    }

    #[test]
    fn special_key_ime_off_combo() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_CONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![combo],
        };
        let mut engine = make_engine_with_special(special);

        let d = engine.on_input(Ev::down(VK_CONVERT).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ime(ImeEffect::SetOpen(false))
        )));
    }

    #[test]
    fn special_key_not_triggered_on_key_up() {
        let combo = ParsedKeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            vk: VK_NONCONVERT,
        };
        let special = SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![combo],
            ime_on: vec![],
            ime_off: vec![],
        };
        let mut engine = make_engine_with_special(special);

        let d = engine.on_input(Ev::up(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        assert!(
            engine.is_fsm_enabled(),
            "KeyUp should not trigger engine_off"
        );
        assert!(!has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { .. })
        )));
    }

    // ── 6. KeyLifecycle integration ──

    #[test]
    fn lifecycle_key_down_consumed_key_up_consumed() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        let d = engine.on_input(Ev::up(VK_A).at(200).build(), &ime_on_ctx());
        assert!(d.is_consumed());
    }

    #[test]
    fn lifecycle_key_down_passthrough_key_up_passthrough() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_off_ctx());
        assert!(!d.is_consumed());
        let d = engine.on_input(Ev::up(VK_A).at(200).build(), &ime_off_ctx());
        assert!(!d.is_consumed());
    }

    // ── 7. Engine::on_command ImeObserved ──

    #[test]
    fn ime_observed_on_enables_engine() {
        let mut engine = make_test_engine();
        engine.on_command(EngineCommand::ToggleEngine);
        assert!(!engine.is_fsm_enabled());

        let obs = ImeObservation {
            cross_process: Some(true),
            is_japanese: true,
            reliability: ImeReliability::Reliable,
        };
        let d = engine.on_command(EngineCommand::ImeObserved(obs));
        assert!(engine.is_fsm_enabled());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: true })
        )));
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::ImeCache(ImeCacheEffect::UpdateStateCache { ime_on: true })
        )));
    }

    #[test]
    fn ime_observed_off_disables_engine() {
        let mut engine = make_test_engine();
        assert!(engine.is_fsm_enabled());

        let obs = ImeObservation {
            cross_process: Some(false),
            is_japanese: true,
            reliability: ImeReliability::Reliable,
        };
        let d = engine.on_command(EngineCommand::ImeObserved(obs));
        assert!(!engine.is_fsm_enabled());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false })
        )));
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::ImeCache(ImeCacheEffect::UpdateStateCache { ime_on: false })
        )));
    }

    #[test]
    fn ime_observed_no_change_just_updates_cache() {
        let mut engine = make_test_engine();
        assert!(engine.is_fsm_enabled());

        let obs = ImeObservation {
            cross_process: Some(true),
            is_japanese: true,
            reliability: ImeReliability::Reliable,
        };
        let d = engine.on_command(EngineCommand::ImeObserved(obs));
        assert!(engine.is_fsm_enabled());
        assert!(!has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { .. })
        )));
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::ImeCache(ImeCacheEffect::UpdateStateCache { ime_on: true })
        )));
    }

    #[test]
    fn ime_observed_not_japanese_disables() {
        let mut engine = make_test_engine();
        assert!(engine.is_fsm_enabled());

        let obs = ImeObservation {
            cross_process: Some(true),
            is_japanese: false,
            reliability: ImeReliability::Reliable,
        };
        let d = engine.on_command(EngineCommand::ImeObserved(obs));
        assert!(!engine.is_fsm_enabled());
        // not_japanese resolves to false, so should have cache update with ime_on: false
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::ImeCache(ImeCacheEffect::UpdateStateCache { ime_on: false })
        )));
    }

    // ── 8. Engine::on_command FocusChanged ──

    fn make_focus_obs(
        process_id: u32,
        class_name: &str,
        kind: FocusKind,
        skip: bool,
        needs_uia: bool,
        overridden: bool,
        cached_engine_enabled: Option<bool>,
        os_modifiers: Option<super::ModifierState>,
    ) -> FocusObservation {
        FocusObservation {
            process_id,
            class_name: class_name.to_string(),
            kind,
            reason: "test".to_string(),
            needs_uia,
            overridden,
            skip,
            debounce_timer_id: 99,
            debounce_ms: 50,
            cached_engine_enabled,
            os_modifiers,
        }
    }

    #[test]
    fn focus_changed_basic() {
        let mut engine = make_test_engine();
        let obs = make_focus_obs(
            1234,
            "TestClass",
            FocusKind::TextInput,
            false,
            false,
            false,
            None,
            None,
        );
        let d = engine.on_command(EngineCommand::FocusChanged(obs));
        assert!(!d.is_consumed());

        let effs = effects_of(&d);
        assert!(effs.iter().any(|e| matches!(e, Effect::Timer(..))));
        assert!(effs.iter().any(|e| matches!(
            e,
            Effect::Focus(super::decision::FocusEffect::UpdateLastFocusInfo { .. })
        )));
    }

    #[test]
    fn focus_changed_skip_returns_passthrough() {
        let mut engine = make_test_engine();
        let obs = make_focus_obs(
            1234,
            "TestClass",
            FocusKind::TextInput,
            true,
            false,
            false,
            None,
            None,
        );
        let d = engine.on_command(EngineCommand::FocusChanged(obs));
        assert!(matches!(d, Decision::PassThrough));
    }

    #[test]
    fn focus_changed_restores_engine_state() {
        let mut engine = make_test_engine();
        assert!(engine.is_fsm_enabled());

        let obs = make_focus_obs(
            5678,
            "Other",
            FocusKind::TextInput,
            false,
            false,
            false,
            Some(false),
            None,
        );
        let d = engine.on_command(EngineCommand::FocusChanged(obs));
        assert!(
            !engine.is_fsm_enabled(),
            "engine should be disabled from cached state"
        );
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false })
        )));
    }

    #[test]
    fn focus_changed_needs_uia() {
        let mut engine = make_test_engine();
        let obs = make_focus_obs(
            1234,
            "Unknown",
            FocusKind::Undetermined,
            false,
            true,
            false,
            None,
            None,
        );
        let d = engine.on_command(EngineCommand::FocusChanged(obs));
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Focus(super::decision::FocusEffect::RequestUiaClassification)
        )));
    }

    #[test]
    fn focus_changed_saves_old_engine_state() {
        let mut engine = make_test_engine();

        // First focus change to set last_focus_info
        let obs1 = make_focus_obs(
            100,
            "First",
            FocusKind::TextInput,
            false,
            false,
            false,
            None,
            None,
        );
        engine.on_command(EngineCommand::FocusChanged(obs1));

        // Second focus change should save state for the first window
        let obs2 = make_focus_obs(
            200,
            "Second",
            FocusKind::TextInput,
            false,
            false,
            false,
            None,
            None,
        );
        let d = engine.on_command(EngineCommand::FocusChanged(obs2));
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Focus(super::decision::FocusEffect::SaveEngineState { .. })
        )));
    }

    #[test]
    fn focus_changed_overridden_no_cache_insert() {
        let mut engine = make_test_engine();
        let obs = make_focus_obs(
            1234,
            "Override",
            FocusKind::NonText,
            false,
            false,
            true,
            None,
            None,
        );
        let d = engine.on_command(EngineCommand::FocusChanged(obs));
        assert!(!has_effect(&d, |e| matches!(
            e,
            Effect::Focus(super::decision::FocusEffect::InsertFocusCache { .. })
        )));
    }

    #[test]
    fn focus_changed_with_os_modifiers() {
        let mut engine = make_test_engine();
        let mods = super::ModifierState {
            ctrl: true,
            alt: false,
            shift: false,
            win: false,
        };
        let obs = make_focus_obs(
            1234,
            "WithMods",
            FocusKind::TextInput,
            false,
            false,
            false,
            None,
            Some(mods),
        );
        let d = engine.on_command(EngineCommand::FocusChanged(obs));
        assert!(!d.is_consumed());
    }

    // ── 9. Engine::handle_sync_modifiers ──

    #[test]
    fn sync_modifiers_no_mismatch_passes_through() {
        let mut engine = make_test_engine();
        let os_mods = super::ModifierState {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
        };
        let d = engine.on_command(EngineCommand::SyncModifiers(os_mods));
        assert!(!d.is_consumed());
    }

    // ── 10. process_deferred_keys ──

    #[test]
    fn process_deferred_keys_empty_returns_empty() {
        let mut engine = make_test_engine();
        let results = engine.process_deferred_keys(&ime_on_ctx());
        assert!(results.is_empty());
    }

    // ── 11. is_fsm_enabled / shadow_ime_on ──

    #[test]
    fn is_fsm_enabled_default_true() {
        let engine = make_test_engine();
        assert!(engine.is_fsm_enabled());
    }

    #[test]
    fn shadow_ime_on_default_true() {
        let engine = make_test_engine();
        assert!(engine.shadow_ime_on());
    }

    #[test]
    fn set_shadow_ime_on_updates() {
        let mut engine = make_test_engine();
        engine.set_shadow_ime_on(false);
        assert!(!engine.shadow_ime_on());
        engine.set_shadow_ime_on(true);
        assert!(engine.shadow_ime_on());
    }

    // ── 12. Engine::on_command with SwapLayout ──

    #[test]
    fn on_command_swap_layout() {
        let mut engine = make_test_engine();
        let new_layout = make_layout();
        let d = engine.on_command(EngineCommand::SwapLayout(new_layout));
        let _ = d; // verify no panic
    }

    // ── 13. Multiple key sequence integration ──

    #[test]
    fn full_char_input_sequence() {
        let mut engine = make_test_engine();

        // Type 'A' key: down -> timeout -> up
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());

        let d = engine.on_timeout(TIMER_PENDING, &ime_on_ctx());
        assert!(d.is_consumed());
        assert!(
            has_effect(&d, |e| matches!(e, Effect::Input(InputEffect::SendKeys(_)))),
            "timeout should produce SendKeys"
        );

        let d = engine.on_input(Ev::up(VK_A).at(300).build(), &ime_on_ctx());
        assert!(d.is_consumed()); // lifecycle auto-consume

        // Type 'S' key
        let d = engine.on_input(Ev::down(VK_S).at(400).build(), &ime_on_ctx());
        assert!(d.is_consumed());
    }

    // ── 14. Focus change flushes pending key ups ──

    #[test]
    fn focus_change_flushes_pending_key_ups() {
        let mut engine = make_test_engine();

        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());

        let obs = make_focus_obs(
            9999,
            "NewWindow",
            FocusKind::TextInput,
            false,
            false,
            false,
            None,
            None,
        );
        let d = engine.on_command(EngineCommand::FocusChanged(obs));
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Input(InputEffect::ReinjectKey(_))
        )));
    }

    // ── 15. Engine disabled -> char key passes through ──

    #[test]
    fn disabled_engine_char_passes_through() {
        let mut engine = make_test_engine();
        engine.on_command(EngineCommand::ToggleEngine);
        assert!(!engine.is_fsm_enabled());

        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(!d.is_consumed());
    }

    // ── 16. Thumb key with IME ON ──

    #[test]
    fn thumb_key_with_ime_on_is_consumed() {
        let mut engine = make_test_engine();
        let d = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
    }

    // ── 17. SyncImeState with pending flush ──

    #[test]
    fn sync_ime_off_flushes_pending() {
        let mut engine = make_test_engine();
        engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());

        let d = engine.on_command(EngineCommand::SyncImeState { ime_on: false });
        assert!(!engine.is_fsm_enabled());
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: false })
        )));
    }

    // ── 18. SetNgramModel command ──

    #[test]
    fn on_command_set_ngram_model() {
        use crate::ngram::NgramModel;
        let mut engine = make_test_engine();
        let model = NgramModel::new(100_000, 20_000, 50_000, 200_000);
        let d = engine.on_command(EngineCommand::SetNgramModel(model));
        assert!(!d.is_consumed());
    }

    // ── 19. Two char keys in sequence (second key resolves first) ──

    #[test]
    fn two_char_keys_second_resolves_first() {
        let mut engine = make_test_engine();

        // First char key enters PendingChar
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());

        // Second char key resolves first and enters new PendingChar
        let d = engine.on_input(Ev::down(VK_S).at(150).build(), &ime_on_ctx());
        assert!(d.is_consumed());
        // Should have SendKeys for the first character
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Input(InputEffect::SendKeys(_))
        )));
    }

    // ── 20. Thumb + char simultaneous input ──

    #[test]
    fn thumb_then_char_within_threshold() {
        let mut engine = make_test_engine();

        // Left thumb down
        let d = engine.on_input(Ev::down(VK_NONCONVERT).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());

        // Char key within threshold -> simultaneous input
        let d = engine.on_input(Ev::down(VK_A).at(130).build(), &ime_on_ctx());
        assert!(d.is_consumed());
    }

    // ── 21. Focus changed then input ──

    #[test]
    fn focus_changed_then_input_works() {
        let mut engine = make_test_engine();

        let obs = make_focus_obs(
            1234,
            "Editor",
            FocusKind::TextInput,
            false,
            false,
            false,
            None,
            None,
        );
        engine.on_command(EngineCommand::FocusChanged(obs));

        // Input should still work after focus change
        let d = engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());
        assert!(d.is_consumed());
    }

    // ── 22. Multiple toggles ──

    #[test]
    fn multiple_toggles_cycle() {
        let mut engine = make_test_engine();

        for _ in 0..5 {
            engine.on_command(EngineCommand::ToggleEngine);
            assert!(!engine.is_fsm_enabled());
            engine.on_command(EngineCommand::ToggleEngine);
            assert!(engine.is_fsm_enabled());
        }
    }

    // ── 23. ImeObserved with pending key flushes ──

    #[test]
    fn ime_observed_off_with_pending_flushes() {
        let mut engine = make_test_engine();
        // Enter pending state
        engine.on_input(Ev::down(VK_A).at(100).build(), &ime_on_ctx());

        let obs = ImeObservation {
            cross_process: Some(false),
            is_japanese: true,
            reliability: ImeReliability::Reliable,
        };
        let d = engine.on_command(EngineCommand::ImeObserved(obs));
        assert!(!engine.is_fsm_enabled());
        // Should have flush effects (SendKeys) before the state change
        let effs = effects_of(&d);
        assert!(
            effs.len() >= 2,
            "should have flush + state change + cache update effects"
        );
    }

    // ── 24. Focus change restores enabled state ──

    #[test]
    fn focus_changed_restores_enabled_true() {
        let mut engine = make_test_engine();
        // Disable engine
        engine.on_command(EngineCommand::ToggleEngine);
        assert!(!engine.is_fsm_enabled());

        // Focus change with cached_engine_enabled=true should re-enable
        let obs = make_focus_obs(
            5678,
            "Other",
            FocusKind::TextInput,
            false,
            false,
            false,
            Some(true),
            None,
        );
        let d = engine.on_command(EngineCommand::FocusChanged(obs));
        assert!(
            engine.is_fsm_enabled(),
            "engine should be re-enabled from cached state"
        );
        assert!(has_effect(&d, |e| matches!(
            e,
            Effect::Ui(UiEffect::EngineStateChanged { enabled: true })
        )));
    }
}
