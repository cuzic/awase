use awase::config::AppConfig;
use awase::engine::{
    Decision, Effect, Engine, InputContext, InputEffect, NicolaFsm,
    SpecialKeyCombos, TIMER_PENDING,
};
use awase::scanmap::{KeyboardModel, PhysicalPos};
use awase::types::{
    ImeRelevance, KeyAction, KeyClassification, KeyEventType, RawKeyEvent,
    ScanCode, Timestamp, VkCode,
};
use awase::yab::YabLayout;

// ── Helper functions ─────────────────────────────────────────

fn make_nicola_engine() -> Engine {
    let config = AppConfig::load(std::path::Path::new("config.toml")).unwrap();

    let content = std::fs::read_to_string("layout/nicola.yab")
        .expect("Failed to read layout/nicola.yab");
    let layout =
        YabLayout::parse(&content, KeyboardModel::Jis).expect("Failed to parse .yab layout");

    let fsm = NicolaFsm::new(
        layout,
        VK_NONCONVERT,
        VK_CONVERT,
        config.general.simultaneous_threshold_ms,
        config.general.confirm_mode,
        config.general.speculative_delay_ms,
    );

    let mut engine = Engine::new(
        fsm,
        SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
        },
    );
    engine.set_prev_active(true);
    engine
}

/// Default InputContext for tests (IME ON)
fn ctx() -> InputContext {
    InputContext {
        ime_on: true,
        is_romaji: true,
        is_japanese_ime: true,
        modifiers: awase::engine::ModifierState { ctrl: false, alt: false, shift: false, win: false },
        left_thumb_down: None,
        right_thumb_down: None,
    }
}

/// VK code -> scan code (JIS keyboard) for scenario tests
fn vk_to_scan(vk: VkCode) -> ScanCode {
    ScanCode(match vk.0 {
        0x41 => 0x1E, // A
        0x42 => 0x30, // B
        0x43 => 0x2E, // C
        0x44 => 0x20, // D
        0x45 => 0x12, // E
        0x46 => 0x21, // F
        0x47 => 0x22, // G
        0x48 => 0x23, // H
        0x49 => 0x17, // I
        0x4A => 0x24, // J
        0x4B => 0x25, // K
        0x4C => 0x26, // L
        0x4D => 0x32, // M
        0x4E => 0x31, // N
        0x4F => 0x18, // O
        0x50 => 0x19, // P
        0x51 => 0x10, // Q
        0x52 => 0x13, // R
        0x53 => 0x1F, // S
        0x54 => 0x14, // T
        0x55 => 0x16, // U
        0x56 => 0x2F, // V
        0x57 => 0x11, // W
        0x58 => 0x2D, // X
        0x59 => 0x15, // Y
        0x5A => 0x2C, // Z
        0x1D => 0x7B, // VK_NONCONVERT -> muhenkan
        0x1C => 0x79, // VK_CONVERT -> henkan
        _ => 0,
    })
}

/// Classify VK code into KeyClassification and PhysicalPos
fn classify_key(vk: VkCode) -> (KeyClassification, Option<PhysicalPos>) {
    if vk == VK_NONCONVERT {
        (KeyClassification::LeftThumb, None)
    } else if vk == VK_CONVERT {
        (KeyClassification::RightThumb, None)
    } else if let Some(pos) = vk_to_pos(vk) {
        (KeyClassification::Char, Some(pos))
    } else {
        (KeyClassification::Passthrough, None)
    }
}

/// VK -> PhysicalPos mapping for test keys
fn vk_to_pos(vk: VkCode) -> Option<PhysicalPos> {
    // Row/col positions matching nicola.yab layout
    match vk.0 {
        0x41 => Some(PhysicalPos::new(2, 0)), // A
        0x53 => Some(PhysicalPos::new(2, 1)), // S
        0x44 => Some(PhysicalPos::new(2, 2)), // D
        0x46 => Some(PhysicalPos::new(2, 3)), // F
        0x43 => Some(PhysicalPos::new(3, 2)), // C
        0x56 => Some(PhysicalPos::new(3, 3)), // V
        _ => None,
    }
}

fn key_down(vk: VkCode, ts: Timestamp) -> RawKeyEvent {
    let (kc, pos) = classify_key(vk);
    RawKeyEvent {
        vk_code: vk,
        scan_code: vk_to_scan(vk),
        event_type: KeyEventType::KeyDown,
        extra_info: 0,
        timestamp: ts,
        key_classification: kc,
        physical_pos: pos,
        ime_relevance: ImeRelevance::default(),
        modifier_key: None,
    }
}

#[allow(dead_code)]
fn key_up(vk: VkCode, ts: Timestamp) -> RawKeyEvent {
    let (kc, pos) = classify_key(vk);
    RawKeyEvent {
        vk_code: vk,
        scan_code: vk_to_scan(vk),
        event_type: KeyEventType::KeyUp,
        extra_info: 0,
        timestamp: ts,
        key_classification: kc,
        physical_pos: pos,
        ime_relevance: ImeRelevance::default(),
        modifier_key: None,
    }
}

const VK_NONCONVERT: VkCode = VkCode(0x1D);
#[allow(dead_code)]
const VK_CONVERT: VkCode = VkCode(0x1C);

/// Collect output text from a Decision's effects.
fn collect_output(decision: &Decision) -> String {
    let effects = match decision {
        Decision::PassThrough => return String::new(),
        Decision::PassThroughWith { effects } | Decision::Consume { effects } => effects,
    };
    let mut out = String::new();
    for effect in effects {
        if let Effect::Input(InputEffect::SendKeys(actions)) = effect {
            for a in actions {
                match a {
                    KeyAction::Char(ch) => out.push(*ch),
                    KeyAction::Romaji(s) => out.push_str(s),
                    _ => {}
                }
            }
        }
    }
    out
}

// ── Scenario tests ───────────────────────────────────────────
// nicola.yab romaji mappings used in tests:
//   normal:      A -> "u",  S -> "si",  D -> "te"
//   left_thumb:  A -> "wo", S -> "a"
//   right_thumb: S -> "zi"

#[test]
fn scenario_single_chars_sequential() {
    // Type "u" then "si" (VK_A then VK_S) with normal timing, each confirmed by timeout
    let mut engine = make_nicola_engine();
    let mut output = String::new();
    let mut t: Timestamp = 0;

    let r = engine.on_input(key_down(VkCode(0x41), t), &ctx());
    output.push_str(&collect_output(&r));
    t += 120_000; // 120ms later (past threshold)

    // Timeout fires, confirming "u"
    let r = engine.on_timeout(TIMER_PENDING, &ctx());
    output.push_str(&collect_output(&r));

    // VK_S in normal face -> "si"
    let r = engine.on_input(key_down(VkCode(0x53), t), &ctx());
    output.push_str(&collect_output(&r));
    t += 120_000;

    let _ = t;
    let r = engine.on_timeout(TIMER_PENDING, &ctx());
    output.push_str(&collect_output(&r));

    assert_eq!(output, "usi");
}

#[test]
fn scenario_thumb_shift_simultaneous() {
    // Left thumb (VK_NONCONVERT) + VK_A (simultaneous) -> "wo"
    let mut engine = make_nicola_engine();
    let t: Timestamp = 0;

    let r1 = engine.on_input(key_down(VK_NONCONVERT, t), &ctx());
    let r2 = engine.on_input(key_down(VkCode(0x41), t + 30_000), &ctx());
    let r3 = engine.on_timeout(TIMER_PENDING, &ctx());

    let output = format!(
        "{}{}{}",
        collect_output(&r1),
        collect_output(&r2),
        collect_output(&r3)
    );

    assert!(output.contains("wo"), "Expected 'wo' but got {output:?}");
}

#[test]
fn scenario_rapid_sequence_pattern4() {
    // Rapid sequential typing: A="u", S="si", D="te" (char after char within threshold)
    let mut engine = make_nicola_engine();
    let mut output = String::new();

    let r = engine.on_input(key_down(VkCode(0x41), 0), &ctx());
    output.push_str(&collect_output(&r));

    let r = engine.on_input(key_down(VkCode(0x53), 50_000), &ctx());
    output.push_str(&collect_output(&r));

    let r = engine.on_input(key_down(VkCode(0x44), 100_000), &ctx());
    output.push_str(&collect_output(&r));

    let r = engine.on_timeout(TIMER_PENDING, &ctx());
    output.push_str(&collect_output(&r));

    assert_eq!(output, "usite");
}

#[test]
fn scenario_continuous_shift() {
    // Hold left thumb, type multiple shifted chars in sequence.
    // left_thumb + A = "wo", left_thumb + S = "a"
    let mut engine = make_nicola_engine();
    let mut output = String::new();
    let t: Timestamp = 0;

    let r = engine.on_input(key_down(VK_NONCONVERT, t), &ctx());
    output.push_str(&collect_output(&r));

    let r = engine.on_input(key_down(VkCode(0x41), t + 30_000), &ctx());
    output.push_str(&collect_output(&r));

    let r = engine.on_timeout(TIMER_PENDING, &ctx());
    output.push_str(&collect_output(&r));

    let r = engine.on_input(key_down(VkCode(0x53), t + 200_000), &ctx());
    output.push_str(&collect_output(&r));

    let r = engine.on_timeout(TIMER_PENDING, &ctx());
    output.push_str(&collect_output(&r));

    assert!(
        !output.is_empty(),
        "Should have output characters with thumb shift"
    );
    assert!(
        output.contains("wo"),
        "Expected 'wo' in output but got {output:?}"
    );
}

#[test]
fn scenario_char_then_thumb_within_threshold() {
    // Char key first, then thumb key within threshold -> simultaneous -> "wo"
    let mut engine = make_nicola_engine();
    let mut output = String::new();
    let t: Timestamp = 0;

    let r = engine.on_input(key_down(VkCode(0x41), t), &ctx());
    output.push_str(&collect_output(&r));

    let r = engine.on_input(key_down(VK_NONCONVERT, t + 40_000), &ctx());
    output.push_str(&collect_output(&r));

    let r = engine.on_timeout(TIMER_PENDING, &ctx());
    output.push_str(&collect_output(&r));

    assert!(
        output.contains("wo"),
        "Expected char+thumb to produce 'wo', got {output:?}"
    );
}

#[test]
fn scenario_timeout_confirms_single_char() {
    // A single char key followed by timeout should produce the normal face romaji.
    // D = "te"
    let mut engine = make_nicola_engine();

    let r = engine.on_input(key_down(VkCode(0x44), 0), &ctx());
    let mut output = collect_output(&r);

    assert!(r.is_consumed(), "key_down should be consumed");

    let r = engine.on_timeout(TIMER_PENDING, &ctx());
    output.push_str(&collect_output(&r));

    assert_eq!(output, "te");
}

#[test]
fn scenario_right_thumb_shift() {
    // Right thumb (VK_CONVERT) + VK_S -> "zi" (right thumb face)
    let mut engine = make_nicola_engine();
    let mut output = String::new();
    let t: Timestamp = 0;

    let r = engine.on_input(key_down(VK_CONVERT, t), &ctx());
    output.push_str(&collect_output(&r));

    let r = engine.on_input(key_down(VkCode(0x53), t + 30_000), &ctx());
    output.push_str(&collect_output(&r));

    let r = engine.on_timeout(TIMER_PENDING, &ctx());
    output.push_str(&collect_output(&r));

    assert!(
        output.contains("zi"),
        "Expected right thumb + S = 'zi', got {output:?}"
    );
}
