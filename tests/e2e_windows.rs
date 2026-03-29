#![cfg(windows)]

use awase::config::ConfirmMode;
use awase::engine::Engine;
use awase::types::{ContextChange, KeyAction, KeyEventType, RawKeyEvent, ScanCode, VkCode};
use awase::yab::YabLayout;
use timed_fsm::TimedStateMachine;

// ────────────────────────────────────────────
// ヘルパー関数
// ────────────────────────────────────────────

/// ログ初期化（テスト全体で一度だけ）
fn init_test_logging() {
    let _ = env_logger::builder()
        .is_test(true)
        .filter_level(log::LevelFilter::Debug)
        .try_init();
}

/// テスト用 NICOLA レイアウトを読み込む
fn load_test_layout() -> YabLayout {
    let yab_content =
        std::fs::read_to_string("layout/nicola.yab").expect("layout/nicola.yab should exist");
    YabLayout::parse(&yab_content).expect("layout should parse")
}

/// テスト用エンジンを生成
fn make_test_engine(mode: ConfirmMode) -> Engine {
    let layout = load_test_layout();
    Engine::new(
        layout,
        VkCode(0x1D), // VK_NONCONVERT (left thumb)
        VkCode(0x1C), // VK_CONVERT (right thumb)
        100,           // threshold_ms
        mode,
        30, // speculative_delay_ms
    )
}

fn key_down(vk: u16, scan: u32, ts: u64) -> RawKeyEvent {
    RawKeyEvent {
        vk_code: VkCode(vk),
        scan_code: ScanCode(scan),
        event_type: KeyEventType::KeyDown,
        extra_info: 0,
        timestamp: ts,
    }
}

#[allow(dead_code)]
fn key_up(vk: u16, scan: u32, ts: u64) -> RawKeyEvent {
    RawKeyEvent {
        vk_code: VkCode(vk),
        scan_code: ScanCode(scan),
        event_type: KeyEventType::KeyUp,
        extra_info: 0,
        timestamp: ts,
    }
}

// ────────────────────────────────────────────
// Phase 1: Hook + Engine in-process
// ────────────────────────────────────────────

#[test]
fn e2e_engine_basic_char_input() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Wait);

    log::info!("=== E2E: Basic char input (Wait mode) ===");

    // Press 'A' key (VK_A=0x41, scan=0x1E) → should go pending
    let t0 = 1_000_000u64;
    let r = engine.on_event(key_down(0x41, 0x1E, t0));
    log::debug!(
        "KeyDown A: consumed={}, actions={:?}",
        r.consumed,
        r.actions
    );
    assert!(r.consumed, "char key should be consumed in Wait mode");
    assert!(r.actions.is_empty(), "no output yet (pending)");

    // Timeout → should emit the character
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::debug!(
        "Timeout: consumed={}, actions={:?}",
        r.consumed,
        r.actions
    );
    assert!(
        !r.actions.is_empty(),
        "timeout should emit the pending char"
    );
    log::info!("Output action: {:?}", r.actions[0]);
}

#[test]
fn e2e_engine_simultaneous_keystroke() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Wait);

    log::info!("=== E2E: Simultaneous keystroke (char + thumb) ===");

    let t0 = 1_000_000u64;

    // Press 'A' → pending
    let r = engine.on_event(key_down(0x41, 0x1E, t0));
    log::debug!(
        "KeyDown A: consumed={}, actions={:?}",
        r.consumed,
        r.actions
    );
    assert!(r.consumed);

    // Press left thumb (VK_NONCONVERT) within threshold → simultaneous
    let r = engine.on_event(key_down(0x1D, 0x7B, t0 + 30_000));
    log::debug!(
        "KeyDown NonConvert: consumed={}, actions={:?}",
        r.consumed,
        r.actions
    );
    // Should still be pending (PendingCharThumb waiting for 3rd key or timeout)

    // Timeout → should emit simultaneous result (thumb face)
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::debug!(
        "Timeout: consumed={}, actions={:?}",
        r.consumed,
        r.actions
    );
    assert!(
        !r.actions.is_empty(),
        "simultaneous keystroke should produce output"
    );
    log::info!("Simultaneous output: {:?}", r.actions[0]);
}

#[test]
fn e2e_engine_speculative_mode() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Speculative);

    log::info!("=== E2E: Speculative mode ===");

    let t0 = 1_000_000u64;

    // Press 'A' → immediate output (speculative)
    let r = engine.on_event(key_down(0x41, 0x1E, t0));
    log::debug!(
        "KeyDown A (speculative): consumed={}, actions={:?}",
        r.consumed,
        r.actions
    );
    assert!(r.consumed);
    assert!(
        !r.actions.is_empty(),
        "speculative mode outputs immediately"
    );
    log::info!("Speculative output: {:?}", r.actions[0]);

    // Press left thumb within threshold → should retract + re-emit
    let r = engine.on_event(key_down(0x1D, 0x7B, t0 + 30_000));
    log::debug!(
        "KeyDown NonConvert: consumed={}, actions={:?}",
        r.consumed,
        r.actions
    );
    // Should contain BS + new char
    let has_bs = r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(vk) if *vk == 0x08));
    log::info!("Has BS for retraction: {}", has_bs);
    assert!(has_bs, "speculative retraction should include BS");
}

#[test]
fn e2e_engine_all_confirm_modes() {
    init_test_logging();

    for mode in [
        ConfirmMode::Wait,
        ConfirmMode::Speculative,
        ConfirmMode::TwoPhase,
        ConfirmMode::AdaptiveTiming,
        ConfirmMode::NgramPredictive,
    ] {
        log::info!("=== E2E: Testing {:?} mode ===", mode);
        let mut engine = make_test_engine(mode);

        let t0 = 1_000_000u64;
        let r = engine.on_event(key_down(0x41, 0x1E, t0));
        log::debug!(
            "{:?}: KeyDown A: consumed={}, actions={:?}",
            mode,
            r.consumed,
            r.actions
        );
        assert!(r.consumed, "{:?} mode should consume char key", mode);

        // Ensure timeout works
        let r = engine.on_timeout(awase::engine::TIMER_PENDING);
        log::debug!("{:?}: Timeout: actions={:?}", mode, r.actions);
    }
}

#[test]
fn e2e_engine_flush_pending_all_states() {
    init_test_logging();

    log::info!("=== E2E: flush_pending from all states ===");

    // Idle
    let mut engine = make_test_engine(ConfirmMode::Wait);
    let r = engine.flush_pending(ContextChange::ImeOff);
    log::debug!("Flush from Idle: actions={:?}", r.actions);
    assert!(r.actions.is_empty());

    // PendingChar
    let mut engine = make_test_engine(ConfirmMode::Wait);
    engine.on_event(key_down(0x41, 0x1E, 1_000_000));
    let r = engine.flush_pending(ContextChange::ImeOff);
    log::debug!("Flush from PendingChar: actions={:?}", r.actions);
    assert!(!r.actions.is_empty());

    // PendingThumb
    let mut engine = make_test_engine(ConfirmMode::Wait);
    engine.on_event(key_down(0x1D, 0x7B, 1_000_000));
    let r = engine.flush_pending(ContextChange::EngineDisabled);
    log::debug!("Flush from PendingThumb: actions={:?}", r.actions);
    assert!(!r.actions.is_empty());

    // SpeculativeChar
    let mut engine = make_test_engine(ConfirmMode::Speculative);
    engine.on_event(key_down(0x41, 0x1E, 1_000_000));
    let r = engine.flush_pending(ContextChange::InputLanguageChanged);
    log::debug!("Flush from SpeculativeChar: actions={:?}", r.actions);
    // SpeculativeChar already output, flush should be empty
    assert!(r.actions.is_empty());

    log::info!("All flush_pending states verified");
}

#[test]
fn e2e_engine_passthrough_keys() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Wait);

    log::info!("=== E2E: Passthrough keys ===");

    let t0 = 1_000_000u64;

    // Ctrl key → passthrough
    let r = engine.on_event(key_down(0x11, 0x1D, t0)); // VK_CTRL
    log::debug!("Ctrl: consumed={}, passthrough={}", r.consumed, !r.consumed);
    assert!(!r.consumed, "Ctrl should passthrough");

    // Esc → passthrough
    let r = engine.on_event(key_down(0x1B, 0x01, t0)); // VK_ESCAPE
    log::debug!("Esc: consumed={}", r.consumed);
    assert!(!r.consumed, "Esc should passthrough");

    // F1 → passthrough
    let r = engine.on_event(key_down(0x70, 0x3B, t0)); // VK_F1
    log::debug!("F1: consumed={}", r.consumed);
    assert!(!r.consumed, "F1 should passthrough");

    log::info!("All passthrough keys verified");
}

#[test]
fn e2e_engine_disabled_passthrough() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Wait);

    log::info!("=== E2E: Disabled engine passthrough ===");

    let _ = engine.toggle_enabled();
    assert!(!engine.is_enabled());

    let r = engine.on_event(key_down(0x41, 0x1E, 1_000_000));
    log::debug!("Disabled engine: consumed={}", r.consumed);
    assert!(
        !r.consumed,
        "disabled engine should passthrough all keys"
    );

    log::info!("Disabled engine passthrough verified");
}

// ────────────────────────────────────────────
// Phase 2: SendInput + Edit control
// ────────────────────────────────────────────

/// Phase 2 tests require an actual Windows desktop session.
/// Skip on headless CI if necessary.
fn is_interactive_session() -> bool {
    // Check if we have a desktop
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;
        let desktop = GetDesktopWindow();
        !desktop.0.is_null()
    }
}

#[test]
fn e2e_sendinput_edit_control() {
    init_test_logging();
    if !is_interactive_session() {
        log::warn!("Skipping SendInput test: no interactive desktop session");
        return;
    }
    log::info!("=== E2E Phase 2: SendInput + Edit control ===");
    // TODO: Implement when CI desktop is confirmed working
    log::info!("Phase 2 test placeholder - needs interactive session");
}

// ────────────────────────────────────────────
// Phase 3: IME + NICOLA conversion
// ────────────────────────────────────────────

#[test]
fn e2e_ime_nicola_conversion() {
    init_test_logging();
    if !is_interactive_session() {
        log::warn!("Skipping IME test: no interactive desktop session");
        return;
    }
    // Check if Japanese IME is available
    log::info!("=== E2E Phase 3: IME + NICOLA conversion ===");
    log::info!("Phase 3 test placeholder - needs Japanese IME");
}
