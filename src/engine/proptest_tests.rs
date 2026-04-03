//! Property-based tests for NicolaFsm / Engine using proptest.

use proptest::prelude::*;

use crate::config::ConfirmMode;
use crate::engine::decision::{ImeSyncKeys, InputContext, SpecialKeyCombos};
use crate::engine::engine::Engine;
use crate::engine::fsm_types::EngineState;
use crate::engine::input_tracker::InputTracker;
use crate::engine::nicola_fsm::{NicolaFsm, TIMER_PENDING};
use crate::scanmap::PhysicalPos;
use crate::types::{
    KeyClassification, KeyEventType, ModifierKey, RawKeyEvent, ScanCode, VkCode,
};
use crate::yab::{YabFace, YabLayout, YabValue};

use timed_fsm::Response;

// ── Constants ───────────────────────────────────────────────────────────────

const VK_NONCONVERT: VkCode = VkCode(0x1D);
const VK_CONVERT: VkCode = VkCode(0x1C);

/// Realistic VK codes used in test generation.
const VK_POOL: &[u16] = &[
    0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, // A-H
    0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F, 0x50, // I-P
    0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, // Q-X
    0x59, 0x5A, // Y-Z
    0x1C, 0x1D, // Convert, Nonconvert (thumb)
    0x10, 0x11, 0x12, // Shift, Ctrl, Alt
    0x0D, 0x08, 0x20, // Enter, BS, Space
];

// ── Helpers ─────────────────────────────────────────────────────────────────

fn lit(ch: char) -> YabValue {
    YabValue::Literal(ch.to_string())
}

fn make_layout() -> YabLayout {
    let mut normal = YabFace::new();
    normal.insert(PhysicalPos::new(2, 0), lit('う'));
    normal.insert(PhysicalPos::new(2, 1), lit('し'));

    let mut left_thumb = YabFace::new();
    left_thumb.insert(PhysicalPos::new(2, 0), lit('を'));
    left_thumb.insert(PhysicalPos::new(2, 1), lit('あ'));

    let mut right_thumb = YabFace::new();
    right_thumb.insert(PhysicalPos::new(2, 0), lit('ゔ'));
    right_thumb.insert(PhysicalPos::new(2, 1), lit('じ'));

    YabLayout {
        name: String::from("proptest"),
        normal,
        left_thumb,
        right_thumb,
        shift: YabFace::new(),
    }
}

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

/// Create an Engine for high-level tests.
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
    let mut engine = Engine::new(fsm, tracker, empty_sync_keys(), empty_special_keys());
    engine.set_prev_active(true);
    engine
}

/// Harness for NicolaFsm-level tests (direct state access).
struct TestHarness {
    tracker: InputTracker,
    fsm: NicolaFsm,
}

impl TestHarness {
    fn new() -> Self {
        Self {
            tracker: InputTracker::new(),
            fsm: NicolaFsm::new(
                make_layout(),
                VK_NONCONVERT,
                VK_CONVERT,
                100,
                ConfirmMode::Wait,
                30,
            ),
        }
    }

    fn on_event(&mut self, event: RawKeyEvent) -> Response<crate::types::KeyAction, usize> {
        let phys = self.tracker.process(&event);
        self.fsm.on_event(event, &phys)
    }

    fn on_timeout(&mut self, timer_id: usize) -> Response<crate::types::KeyAction, usize> {
        let phys = self.tracker.snapshot();
        self.fsm.on_timeout(timer_id, &phys)
    }
}

fn ime_on_ctx() -> InputContext {
    InputContext {
        ime_on: true,
        is_romaji: true,
        is_japanese_ime: true,
    }
}

/// Classify a VK code into (KeyClassification, Option<PhysicalPos>).
fn classify_vk(vk: VkCode) -> (KeyClassification, Option<PhysicalPos>) {
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

fn vk_to_pos(vk: VkCode) -> Option<PhysicalPos> {
    match vk {
        VkCode(0x41) => Some(PhysicalPos::new(2, 0)), // A
        VkCode(0x53) => Some(PhysicalPos::new(2, 1)), // S
        VkCode(0x44) => Some(PhysicalPos::new(2, 2)), // D
        VkCode(0x46) => Some(PhysicalPos::new(2, 3)), // F
        VkCode(0x43) => Some(PhysicalPos::new(3, 2)), // C
        VkCode(0x56) => Some(PhysicalPos::new(3, 3)), // V
        _ => None,
    }
}

fn classify_modifier(vk: VkCode) -> Option<ModifierKey> {
    match vk {
        VkCode(0x10) | VkCode(0xA0) | VkCode(0xA1) => Some(ModifierKey::Shift),
        VkCode(0x11) | VkCode(0xA2) => Some(ModifierKey::Ctrl),
        VkCode(0x12) | VkCode(0xA4) => Some(ModifierKey::Alt),
        _ => None,
    }
}

fn build_event(vk_raw: u16, event_type: KeyEventType, timestamp: u64) -> RawKeyEvent {
    let vk = VkCode(vk_raw);
    let (kc, pos) = classify_vk(vk);
    RawKeyEvent {
        vk_code: vk,
        scan_code: ScanCode(0),
        event_type,
        extra_info: 0,
        timestamp,
        key_classification: kc,
        physical_pos: pos,
        ime_relevance: crate::types::ImeRelevance::default(),
        modifier_key: classify_modifier(vk),
    }
}

// ── Strategies ──────────────────────────────────────────────────────────────

fn arb_vk() -> impl Strategy<Value = u16> {
    prop::sample::select(VK_POOL)
}

fn arb_event_type() -> impl Strategy<Value = KeyEventType> {
    prop::sample::select(vec![KeyEventType::KeyDown, KeyEventType::KeyUp])
}

/// Generate a sequence of key events with monotonically increasing timestamps.
fn arb_event_sequence(max_len: usize) -> impl Strategy<Value = Vec<(u16, KeyEventType, u64)>> {
    prop::collection::vec((arb_vk(), arb_event_type(), 1u64..50_000u64), 1..max_len).prop_map(
        |mut events| {
            let mut ts: u64 = 1_000_000;
            for ev in &mut events {
                ts += ev.2;
                ev.2 = ts;
            }
            events
        },
    )
}

// ── Property tests ──────────────────────────────────────────────────────────

proptest! {
    /// 1. Engine never panics on arbitrary key sequences.
    #[test]
    fn never_panics_on_arbitrary_events(
        events in arb_event_sequence(50)
    ) {
        let mut engine = make_test_engine();
        let ctx = ime_on_ctx();
        for (vk, et, ts) in &events {
            let ev = build_event(*vk, *et, *ts);
            let _ = engine.on_input(ev, &ctx);
        }
    }

    /// 2. Identical event sequences produce identical decisions (determinism).
    #[test]
    fn deterministic_output(
        events in arb_event_sequence(30)
    ) {
        let ctx = ime_on_ctx();

        let mut engine1 = make_test_engine();
        let mut engine2 = make_test_engine();

        for (vk, et, ts) in &events {
            let ev = build_event(*vk, *et, *ts);
            let d1 = engine1.on_input(ev, &ctx);
            let d2 = engine2.on_input(ev, &ctx);
            prop_assert_eq!(d1.is_consumed(), d2.is_consumed(),
                "Divergent consumed/passthrough for vk={:#x} {:?} at ts={}", vk, et, ts);
        }
    }

    /// 3. Timer timeout always resolves pending state to Idle (NicolaFsm level).
    #[test]
    fn timeout_resolves_to_idle(
        vk in arb_vk(),
        _ts_offset in 1_000u64..50_000u64,
    ) {
        let mut h = TestHarness::new();

        // Send a KeyDown to potentially enter a pending state
        let base_ts = 1_000_000u64;
        let ev = build_event(vk, KeyEventType::KeyDown, base_ts);
        let _ = h.on_event(ev);

        // Fire the pending timer
        let _ = h.on_timeout(TIMER_PENDING);

        // After timeout, FSM state must be Idle
        prop_assert!(h.fsm.state.is_idle(),
            "State should be Idle after timeout, got {:?}", h.fsm.state);
    }

    /// 4. FSM state is always a valid variant after any sequence (NicolaFsm level).
    #[test]
    fn state_always_valid(
        events in arb_event_sequence(40)
    ) {
        let mut h = TestHarness::new();

        for (vk, et, ts) in &events {
            let ev = build_event(*vk, *et, *ts);
            let _ = h.on_event(ev);

            // Exhaustive match proves the state is a valid variant.
            match h.fsm.state {
                EngineState::Idle => {}
                EngineState::PendingChar(_) => {}
                EngineState::PendingThumb(_) => {}
                EngineState::PendingCharThumb { .. } => {}
                EngineState::SpeculativeChar(_) => {}
            }
        }

        // After all events, fire timeout to flush pending state
        let _ = h.on_timeout(TIMER_PENDING);
        prop_assert!(h.fsm.state.is_idle(),
            "After full sequence + timeout, should be Idle");
    }

    /// 5. KeyDown followed by KeyUp for the same key: if KeyDown is consumed,
    ///    KeyUp is also consumed (lifecycle balance via Engine).
    #[test]
    fn keydown_keyup_balance(
        vk in arb_vk(),
        ts_base in 1_000_000u64..2_000_000u64,
    ) {
        let mut engine = make_test_engine();
        let ctx = ime_on_ctx();

        let down_ev = build_event(vk, KeyEventType::KeyDown, ts_base);
        let d_down = engine.on_input(down_ev, &ctx);

        // Flush any pending state so the KeyDown is fully resolved
        let _ = engine.on_timeout(TIMER_PENDING, &ctx);

        let up_ev = build_event(vk, KeyEventType::KeyUp, ts_base + 50_000);
        let d_up = engine.on_input(up_ev, &ctx);

        if d_down.is_consumed() {
            prop_assert!(d_up.is_consumed(),
                "KeyUp should be consumed when KeyDown was consumed (vk={:#x})", vk);
        }
    }

    /// 6. Interleaved timeouts never panic and always converge to Idle.
    #[test]
    fn interleaved_timeouts_converge(
        events in arb_event_sequence(30),
        timeout_indices in prop::collection::vec(0usize..30, 0..10),
    ) {
        let mut h = TestHarness::new();

        for (i, (vk, et, ts)) in events.iter().enumerate() {
            if timeout_indices.contains(&i) {
                let _ = h.on_timeout(TIMER_PENDING);
            }
            let ev = build_event(*vk, *et, *ts);
            let _ = h.on_event(ev);
        }

        // Final timeout to flush
        let _ = h.on_timeout(TIMER_PENDING);
        prop_assert!(h.fsm.state.is_idle(),
            "After all events and final timeout, state should be Idle");
    }
}
