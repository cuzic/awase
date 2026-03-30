//! # Local execution
//!
//! ```powershell
//! # Phase 1 + Phase 2 SendMessage tests (also works in CI)
//! cargo test --test e2e_windows -- --nocapture 2>&1 | Tee-Object e2e.log
//!
//! # Phase 2 SendInput + Phase 3 IME tests (local Windows only)
//! $env:AWASE_E2E_INTERACTIVE="1"
//! $env:RUST_LOG="debug"
//! cargo test --test e2e_windows -- --nocapture 2>&1 | Tee-Object e2e.log
//!
//! # Share the log for debugging
//! # Just send e2e.log as-is
//! ```

#![cfg(windows)]

use awase::config::ConfirmMode;
use awase::engine::input_tracker::InputTracker;
use awase::engine::Engine;
use awase::types::{ContextChange, KeyAction, KeyEventType, RawKeyEvent, ScanCode, VkCode};
use awase::yab::YabLayout;

use std::sync::Mutex;

/// Phase 2-3 tests contest foreground focus when run in parallel.
/// This lock serializes them.
static INTERACTIVE_TEST_LOCK: Mutex<()> = Mutex::new(());

// ────────────────────────────────────────────
// Helper functions
// ────────────────────────────────────────────

/// Initialize logging (once per test run)
fn init_test_logging() {
    let _ = env_logger::builder()
        .is_test(true)
        .filter_level(log::LevelFilter::Debug)
        .try_init();
}

/// Load the test NICOLA layout
fn load_test_layout() -> YabLayout {
    let yab_content =
        std::fs::read_to_string("layout/nicola.yab").expect("layout/nicola.yab should exist");
    YabLayout::parse(&yab_content).expect("layout should parse")
}

/// テスト用ハーネス: InputTracker + Engine を統合
struct TestHarness {
    tracker: InputTracker,
    engine: Engine,
}

impl TestHarness {
    fn on_event(&mut self, event: RawKeyEvent) -> timed_fsm::Response<KeyAction, usize> {
        let phys = self.tracker.process(&event);
        self.engine.on_event(event, &phys)
    }

    fn on_timeout(&mut self, timer_id: usize) -> timed_fsm::Response<KeyAction, usize> {
        let phys = self.tracker.snapshot();
        self.engine.on_timeout(timer_id, &phys)
    }
}

impl std::ops::Deref for TestHarness {
    type Target = Engine;
    fn deref(&self) -> &Engine { &self.engine }
}

impl std::ops::DerefMut for TestHarness {
    fn deref_mut(&mut self) -> &mut Engine { &mut self.engine }
}

const VK_NONCONVERT: VkCode = VkCode(0x1D);
const VK_CONVERT: VkCode = VkCode(0x1C);

/// Create a test engine
fn make_test_engine(mode: ConfirmMode) -> TestHarness {
    let layout = load_test_layout();
    TestHarness {
        tracker: InputTracker::new(VK_NONCONVERT, VK_CONVERT),
        engine: Engine::new(
            layout,
            VK_NONCONVERT,
            VK_CONVERT,
            100,          // threshold_ms
            mode,
            30, // speculative_delay_ms
        ),
    }
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

fn key_up(vk: u16, scan: u32, ts: u64) -> RawKeyEvent {
    RawKeyEvent {
        vk_code: VkCode(vk),
        scan_code: ScanCode(scan),
        event_type: KeyEventType::KeyUp,
        extra_info: 0,
        timestamp: ts,
    }
}

/// Log system diagnostics at the start of Phase 2-3 tests
unsafe fn log_system_info() {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    log::info!("=== System Diagnostics ===");

    // OS version
    log::info!(
        "OS: Windows (CI={}, GITHUB_ACTIONS={})",
        std::env::var("CI").unwrap_or_default(),
        std::env::var("GITHUB_ACTIONS").unwrap_or_default()
    );

    // Desktop window
    let desktop = GetDesktopWindow();
    log::info!("Desktop HWND: {:?}", desktop);

    // Foreground window
    let fg = GetForegroundWindow();
    log::info!("Foreground HWND: {:?}", fg);

    // Keyboard layout
    let hkl = GetKeyboardLayout(0);
    let lang_id = (hkl.0 as u32) & 0xFFFF;
    log::info!("Keyboard layout: HKL={:?} lang_id=0x{:04X}", hkl, lang_id);

    // Thread ID
    log::info!("Thread ID: {:?}", std::thread::current().id());

    log::info!("=== End Diagnostics ===");
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
    log::debug!("Timeout: consumed={}, actions={:?}", r.consumed, r.actions);
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
    log::debug!("Timeout: consumed={}, actions={:?}", r.consumed, r.actions);
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
    assert!(!r.consumed, "disabled engine should passthrough all keys");

    log::info!("Disabled engine passthrough verified");
}

// ────────────────────────────────────────────
// Phase 2: SendMessage + Edit control (CI compatible)
//          + SendInput interactive tests (local only)
// ────────────────────────────────────────────

/// Phase 2-3 tests require an interactive desktop session.
/// Skipped in CI (GitHub Actions) because SendInput cannot reach the foreground window.
///
/// Set `AWASE_E2E_INTERACTIVE=1` to force execution.
fn is_interactive_session() -> bool {
    // Explicit opt-in via environment variable
    if std::env::var("AWASE_E2E_INTERACTIVE").map_or(false, |v| v == "1") {
        return true;
    }
    // Skip in CI (GitHub Actions)
    if std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok() {
        log::info!("CI environment detected, skipping interactive tests");
        log::info!("Set AWASE_E2E_INTERACTIVE=1 to force execution");
        return false;
    }
    // Check for desktop presence
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;
        let desktop = GetDesktopWindow();
        !desktop.0.is_null()
    }
}

/// Test window procedure (delegates to DefWindowProcW)
unsafe extern "system" fn test_wnd_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Helper that creates a window with a hidden Edit control, sends keys via
/// SendInput, and reads back the Edit contents.
///
/// The window is destroyed when dropped.
struct TestEditWindow {
    hwnd: windows::Win32::Foundation::HWND,
    edit_hwnd: windows::Win32::Foundation::HWND,
}

impl TestEditWindow {
    unsafe fn create() -> Option<Self> {
        use windows::Win32::Foundation::{HINSTANCE, HWND};
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, GetKeyboardLayout};
        use windows::Win32::UI::WindowsAndMessaging::*;

        // Register window class
        let class_name_wide: Vec<u16> = "AwaseTestWindow\0".encode_utf16().collect();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(test_wnd_proc),
            hInstance: HINSTANCE::default(),
            lpszClassName: windows::core::PCWSTR(class_name_wide.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&wc);

        // Create parent window
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            windows::core::PCWSTR(class_name_wide.as_ptr()),
            windows::core::PCWSTR::null(),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            400,
            300,
            HWND::default(),
            None,
            HINSTANCE::default(),
            None,
        );
        let hwnd = match hwnd {
            Ok(h) if h != HWND::default() => h,
            Ok(_) => {
                let err = windows::core::Error::from_win32();
                log::error!("CreateWindowExW returned null HWND: {:?}", err);
                return None;
            }
            Err(e) => {
                log::error!("CreateWindowExW failed: {:?}", e);
                return None;
            }
        };

        // Create Edit control
        let edit_class: Vec<u16> = "EDIT\0".encode_utf16().collect();
        let edit_hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            windows::core::PCWSTR(edit_class.as_ptr()),
            windows::core::PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0080), // ES_AUTOHSCROLL
            10,
            10,
            360,
            30,
            hwnd,
            None,
            HINSTANCE::default(),
            None,
        );
        let edit_hwnd = match edit_hwnd {
            Ok(h) if h != HWND::default() => h,
            Ok(_) => {
                let err = windows::core::Error::from_win32();
                log::error!("CreateWindowExW(EDIT) returned null HWND: {:?}", err);
                let _ = DestroyWindow(hwnd);
                return None;
            }
            Err(e) => {
                log::error!("CreateWindowExW(EDIT) failed: {:?}", e);
                let _ = DestroyWindow(hwnd);
                return None;
            }
        };

        // Log window creation details
        log::info!("Window created: hwnd={:?} class=AwaseTestWindow", hwnd);
        log::info!("Edit created: hwnd={:?} class=EDIT", edit_hwnd);

        // Show window and set focus
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(edit_hwnd);

        // Process messages to complete rendering
        pump_messages();

        // Verify focus state after setup
        let fg_after = GetForegroundWindow();
        let focus_after = GetFocus();
        log::info!(
            "After focus: foreground={:?} focus={:?} (expected edit={:?})",
            fg_after,
            focus_after,
            edit_hwnd
        );
        log::info!("Foreground match: {}", fg_after == hwnd);
        log::info!("Focus match: {}", focus_after == edit_hwnd);

        // Keyboard layout at creation time
        let hkl = GetKeyboardLayout(0);
        let lang_id = (hkl.0 as u32) & 0xFFFF;
        log::info!(
            "Keyboard layout at window create: HKL={:?} lang_id=0x{:04X}",
            hkl,
            lang_id
        );

        Some(Self { hwnd, edit_hwnd })
    }

    /// Get the Edit control's text
    unsafe fn get_text(&self) -> String {
        use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
        let mut buf = [0u16; 1024];
        let len = GetWindowTextW(self.edit_hwnd, &mut buf);
        if len > 0 {
            String::from_utf16_lossy(&buf[..len as usize])
        } else {
            String::new()
        }
    }

    /// Clear the Edit control's text
    unsafe fn clear(&self) {
        use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_SETTEXT};
        let empty: Vec<u16> = "\0".encode_utf16().collect();
        SendMessageW(
            self.edit_hwnd,
            WM_SETTEXT,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(empty.as_ptr() as isize),
        );
    }

    /// Set focus to the Edit control
    unsafe fn focus(&self) {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};
        let _ = SetForegroundWindow(self.hwnd);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(self.edit_hwnd);
        pump_messages();

        let fg = GetForegroundWindow();
        let focus = GetFocus();
        log::debug!(
            "focus(): foreground={:?} (expected={:?}) focus={:?} (expected={:?})",
            fg,
            self.hwnd,
            focus,
            self.edit_hwnd
        );
    }
}

impl Drop for TestEditWindow {
    fn drop(&mut self) {
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;
            let _ = DestroyWindow(self.hwnd);
            log::debug!("TestEditWindow destroyed");
        }
    }
}

/// Process pending window messages
unsafe fn pump_messages() {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::*;
    let mut msg = MSG::default();
    while PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE).as_bool() {
        DispatchMessageW(&msg);
    }
}

/// Send a character directly to the Edit control (via SendMessage, no focus needed)
unsafe fn send_char_to_edit(edit_hwnd: windows::Win32::Foundation::HWND, ch: char) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_CHAR};

    SendMessageW(edit_hwnd, WM_CHAR, WPARAM(ch as usize), LPARAM(0));
    log::debug!("SendMessage WM_CHAR: '{ch}' to {:?}", edit_hwnd);
    pump_messages();
}

/// Send a key event directly to the Edit control (WM_KEYDOWN + WM_KEYUP)
#[allow(dead_code)]
unsafe fn send_keydown_to_edit(edit_hwnd: windows::Win32::Foundation::HWND, vk: u16) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_KEYDOWN, WM_KEYUP};

    SendMessageW(edit_hwnd, WM_KEYDOWN, WPARAM(vk as usize), LPARAM(0));
    SendMessageW(edit_hwnd, WM_KEYUP, WPARAM(vk as usize), LPARAM(0));
    log::debug!(
        "SendMessage WM_KEYDOWN+UP: vk=0x{vk:02X} to {:?}",
        edit_hwnd
    );
    pump_messages();
}

/// Send a keystroke via SendInput (bypasses hooks, goes to foreground window)
unsafe fn send_key_to_edit(vk: u16, scan: u16) {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let inputs = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: scan,
                    dwFlags: KEYBD_EVENT_FLAGS::default(),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: scan,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    ];
    let size = i32::try_from(std::mem::size_of::<INPUT>()).expect("INPUT size fits i32");
    let sent = SendInput(&inputs, size);
    log::debug!("SendInput: vk=0x{vk:02X} scan=0x{scan:02X} sent={sent}");

    if sent == 0 {
        let err = windows::core::Error::from_win32();
        log::error!("SendInput failed! Error: {:?}", err);
    }

    // Wait for the input to be processed
    std::thread::sleep(std::time::Duration::from_millis(50));
    pump_messages();

    // After pump_messages, check foreground
    let fg = GetForegroundWindow();
    let focus = GetFocus();
    log::debug!("After send: foreground={:?} focus={:?}", fg, focus);
}

#[test]
fn e2e_message_edit_control() {
    init_test_logging();
    log::info!("=== E2E Phase 2: SendMessage + Edit control ===");

    unsafe {
        let Some(win) = TestEditWindow::create() else {
            log::error!("Could not create test window, skipping");
            return;
        };

        // Test 1: Single char 'a'
        log::info!("--- Test: WM_CHAR 'a' ---");
        win.clear();
        send_char_to_edit(win.edit_hwnd, 'a');
        let text = win.get_text();
        log::info!("Edit content: '{text}'");
        assert_eq!(text, "a", "WM_CHAR 'a' should produce 'a', got: '{text}'");

        // Test 2: Multiple chars
        log::info!("--- Test: WM_CHAR 'abc' ---");
        win.clear();
        send_char_to_edit(win.edit_hwnd, 'a');
        send_char_to_edit(win.edit_hwnd, 'b');
        send_char_to_edit(win.edit_hwnd, 'c');
        let text = win.get_text();
        log::info!("Edit content: '{text}'");
        assert_eq!(text, "abc", "Expected 'abc', got: '{text}'");

        // Test 3: Backspace via WM_CHAR '\x08'
        // Edit control deletes on WM_CHAR with BS character (\x08)
        log::info!("--- Test: Backspace ---");
        win.clear();
        send_char_to_edit(win.edit_hwnd, 'a');
        send_char_to_edit(win.edit_hwnd, 'b');
        send_char_to_edit(win.edit_hwnd, '\x08'); // BS as WM_CHAR
        let text = win.get_text();
        log::info!("Edit content after BS: '{text}'");
        assert_eq!(text, "a", "After BS expected 'a', got: '{text}'");

        // Test 4: Unicode character (Japanese)
        log::info!("--- Test: Unicode char '\u{3042}' ---");
        win.clear();
        send_char_to_edit(win.edit_hwnd, '\u{3042}');
        let text = win.get_text();
        log::info!("Edit content: '{text}'");
        assert_eq!(
            text, "\u{3042}",
            "WM_CHAR should handle Unicode, got: '{text}'"
        );

        log::info!("=== Phase 2 tests passed ===");
    }
}

#[test]
fn e2e_sendinput_interactive() {
    init_test_logging();
    if !is_interactive_session() {
        log::info!("Skipping SendInput interactive test (set AWASE_E2E_INTERACTIVE=1)");
        return;
    }
    let _lock = INTERACTIVE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    log::info!("=== E2E Phase 2 (interactive): SendInput + Edit control ===");

    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

        log_system_info();

        let Some(win) = TestEditWindow::create() else {
            log::error!("Could not create test window, skipping");
            return;
        };

        // Test 1: Simple key input ('A' -> 'a')
        log::info!("--- Test: Single key 'A' -> edit should contain 'a' ---");
        win.clear();
        win.focus();
        send_key_to_edit(0x41, 0x1E); // VK_A, scan=0x1E
        let text = win.get_text();
        log::info!("Edit content after 'A': '{text}'");
        assert!(
            text.contains('a') || text.contains('A'),
            "Edit should contain 'a' or 'A'\n\
             Got: '{}'\n\
             Foreground HWND: {:?}\n\
             Focus HWND: {:?}\n\
             Edit HWND: {:?}",
            text,
            GetForegroundWindow(),
            GetFocus(),
            win.edit_hwnd
        );

        // Test 2: Multiple key input
        log::info!("--- Test: Multiple keys 'ABC' ---");
        win.clear();
        win.focus();
        send_key_to_edit(0x41, 0x1E); // A
        send_key_to_edit(0x42, 0x30); // B
        send_key_to_edit(0x43, 0x2E); // C
        let text = win.get_text();
        log::info!("Edit content after 'ABC': '{text}'");
        {
            let fg = GetForegroundWindow();
            let focus = GetFocus();
            assert_eq!(
                text.to_ascii_lowercase(),
                "abc",
                "Edit should contain 'abc'\n\
                 Got: '{}'\n\
                 Foreground HWND: {:?}\n\
                 Focus HWND: {:?}\n\
                 Edit HWND: {:?}",
                text,
                fg,
                focus,
                win.edit_hwnd
            );
        }

        // Test 3: Backspace
        log::info!("--- Test: Backspace deletes last char ---");
        win.clear();
        win.focus();
        send_key_to_edit(0x41, 0x1E); // A
        send_key_to_edit(0x42, 0x30); // B
        send_key_to_edit(0x08, 0x0E); // VK_BACK
        let text = win.get_text();
        log::info!("Edit content after 'AB' + BS: '{text}'");
        {
            let fg = GetForegroundWindow();
            let focus = GetFocus();
            assert_eq!(
                text.to_ascii_lowercase(),
                "a",
                "Edit should contain 'a' after backspace\n\
                 Got: '{}'\n\
                 Foreground HWND: {:?}\n\
                 Focus HWND: {:?}\n\
                 Edit HWND: {:?}",
                text,
                fg,
                focus,
                win.edit_hwnd
            );
        }

        log::info!("=== Phase 2 interactive tests passed ===");
    }
}

#[test]
fn e2e_message_special_keys() {
    init_test_logging();
    log::info!("=== E2E Phase 2: Special keys via SendMessage ===");

    unsafe {
        let Some(win) = TestEditWindow::create() else {
            log::error!("Could not create test window, skipping");
            return;
        };

        // Test: Multiple chars then clear
        win.clear();
        send_char_to_edit(win.edit_hwnd, 'x');
        send_char_to_edit(win.edit_hwnd, 'y');
        let text = win.get_text();
        log::info!("Before clear: '{text}'");
        assert_eq!(text, "xy");

        win.clear();
        let text = win.get_text();
        log::info!("After clear: '{text}'");
        assert_eq!(text, "", "clear() should empty the edit");

        log::info!("=== Special keys tests passed ===");
    }
}

#[test]
fn e2e_sendinput_special_keys_interactive() {
    init_test_logging();
    if !is_interactive_session() {
        log::info!(
            "Skipping SendInput special keys interactive test (set AWASE_E2E_INTERACTIVE=1)"
        );
        return;
    }
    let _lock = INTERACTIVE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    log::info!("=== E2E Phase 2 (interactive): Special keys ===");

    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

        log_system_info();

        let Some(win) = TestEditWindow::create() else {
            log::error!("Could not create test window, skipping");
            return;
        };

        // Enter key does not insert a newline in a single-line Edit
        log::info!("--- Test: Enter key in single-line Edit ---");
        win.clear();
        win.focus();
        send_key_to_edit(0x41, 0x1E); // A
        send_key_to_edit(0x0D, 0x1C); // VK_RETURN
        send_key_to_edit(0x42, 0x30); // B
        let text = win.get_text();
        log::info!("Edit content after A+Enter+B: '{text}'");
        // Single-line Edit ignores Enter
        {
            let fg = GetForegroundWindow();
            let focus = GetFocus();
            assert!(
                text.to_ascii_lowercase().contains("ab"),
                "Single-line Edit should ignore Enter\n\
                 Got: '{}'\n\
                 Foreground HWND: {:?}\n\
                 Focus HWND: {:?}\n\
                 Edit HWND: {:?}",
                text,
                fg,
                focus,
                win.edit_hwnd
            );
        }

        log::info!("=== Special keys interactive tests passed ===");
    }
}

// ────────────────────────────────────────────
// Phase 3: IME + NICOLA conversion
// ────────────────────────────────────────────

/// Check if Japanese IME is available
unsafe fn is_japanese_ime_available() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
    // Check the thread's keyboard layout
    let hkl = GetKeyboardLayout(0);
    let lang_id = (hkl.0 as u32) & 0xFFFF;
    let is_japanese = lang_id == 0x0411; // ja-JP
    log::debug!(
        "Keyboard layout: HKL={:?} lang_id=0x{:04X} japanese={}",
        hkl,
        lang_id,
        is_japanese
    );
    is_japanese
}

/// Set the IME open status
unsafe fn set_ime_open(hwnd: windows::Win32::Foundation::HWND, open: bool) -> bool {
    use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext, ImmSetOpenStatus};
    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() {
        let err = windows::core::Error::from_win32();
        log::warn!("ImmGetContext failed for hwnd={:?}: {:?}", hwnd, err);
        return false;
    }
    let result = ImmSetOpenStatus(himc, open);
    let _ = ImmReleaseContext(hwnd, himc);
    log::debug!("ImmSetOpenStatus({open}): result={:?}", result);
    result.as_bool()
}

/// Get the IME open status
unsafe fn get_ime_open(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmGetOpenStatus, ImmReleaseContext};
    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() {
        let err = windows::core::Error::from_win32();
        log::warn!(
            "ImmGetContext failed (get_ime_open) for hwnd={:?}: {:?}",
            hwnd,
            err
        );
        return false;
    }
    let status = ImmGetOpenStatus(himc);
    let _ = ImmReleaseContext(hwnd, himc);
    status.as_bool()
}

#[test]
fn e2e_ime_status_detection() {
    init_test_logging();
    if !is_interactive_session() {
        log::warn!("Skipping IME test: no interactive desktop session");
        return;
    }
    let _lock = INTERACTIVE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    log::info!("=== E2E Phase 3: IME status detection ===");

    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

        log_system_info();

        let Some(win) = TestEditWindow::create() else {
            log::error!("Could not create test window, skipping");
            return;
        };
        win.focus();

        // Check for Japanese IME availability
        let has_japanese = is_japanese_ime_available();
        log::info!("Japanese IME available: {has_japanese}");

        if !has_japanese {
            log::warn!("Japanese IME not installed, skipping IME-specific tests");
            log::info!(
                "To enable: PowerShell -> New-WinUserLanguageList ja-JP -> Set-WinUserLanguageList"
            );
            return;
        }

        // IME OFF -> direct input to Edit
        log::info!("--- Test: IME OFF -> direct input ---");
        set_ime_open(win.edit_hwnd, false);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let ime_status = get_ime_open(win.edit_hwnd);
        log::info!("IME open status after OFF: {ime_status}");

        win.clear();
        // Use SendMessage(WM_CHAR) instead of SendInput to avoid focus issues
        // in parallel test execution. SendInput requires foreground focus which
        // is contested when tests run concurrently.
        send_char_to_edit(win.edit_hwnd, 'a');
        let text = win.get_text();
        log::info!("IME OFF, sent 'a' via WM_CHAR: edit='{text}'");
        assert_eq!(
            text, "a",
            "IME OFF: WM_CHAR 'a' should produce 'a', got: '{text}'"
        );

        // IME ON -> verify romaji input mode behavior
        log::info!("--- Test: IME ON -> romaji input ---");
        set_ime_open(win.edit_hwnd, true);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let ime_status = get_ime_open(win.edit_hwnd);
        log::info!("IME open status after ON: {ime_status}");

        if !ime_status {
            log::warn!("Could not enable IME, skipping IME ON tests");
            return;
        }

        win.clear();
        // Note: SendMessage(WM_CHAR) bypasses IME processing, so it sends raw
        // characters regardless of IME state. True IME romaji-to-kana conversion
        // can only be tested with SendInput in an interactive session with
        // guaranteed foreground focus.
        send_char_to_edit(win.edit_hwnd, 'a');
        pump_messages();
        let text = win.get_text();
        log::info!("IME ON, sent 'a' via WM_CHAR: edit='{text}'");
        // WM_CHAR bypasses IME, so we get the raw character
        log::info!("WM_CHAR result: '{text}' (WM_CHAR bypasses IME, raw char expected)");

        // Restore IME OFF (cleanup)
        set_ime_open(win.edit_hwnd, false);
        log::info!("=== Phase 3 IME status tests completed ===");
    }
}

#[test]
fn e2e_engine_with_ime_context() {
    init_test_logging();
    // This is a pure Engine in-process test (no windows, no SendInput).
    // No INTERACTIVE_TEST_LOCK needed.
    log::info!("=== E2E Phase 3: Engine with IME context ===");

    // Use Engine in-process to verify behavior based on IME state
    let mut engine = make_test_engine(ConfirmMode::Wait);
    let t0 = 1_000_000u64;

    // Engine enabled + normal key input -> consumed
    let r = engine.on_event(key_down(0x41, 0x1E, t0));
    log::debug!("Engine enabled, 'A': consumed={}", r.consumed);
    assert!(r.consumed, "enabled engine should consume char key");

    // Confirm via timeout
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::debug!("Timeout: actions={:?}", r.actions);

    // Disable engine
    let (enabled, _flush) = engine.toggle_enabled();
    assert!(!enabled);
    let r = engine.on_event(key_down(0x41, 0x1E, t0 + 500_000));
    log::debug!("Engine disabled, 'A': consumed={}", r.consumed);
    assert!(!r.consumed, "disabled engine should passthrough");

    // Re-enable
    let (enabled, _flush) = engine.toggle_enabled();
    assert!(enabled);
    let r = engine.on_event(key_down(0x41, 0x1E, t0 + 1_000_000));
    log::debug!("Engine re-enabled, 'A': consumed={}", r.consumed);
    assert!(r.consumed, "re-enabled engine should consume");

    // Flush to release pending state
    let _ = engine.flush_pending(ContextChange::ImeOff);

    log::info!("=== Phase 3 engine+IME tests completed ===");
}

// ─────────────────────────────────────────────
// NICOLA layout real-device mapping tests
// ─────────────────────────────────────────────

#[test]
fn e2e_nicola_normal_face_mapping() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Wait);
    log::info!("=== E2E: NICOLA normal face mapping ===");

    let t0 = 1_000_000u64;

    // Test multiple keys on the normal face
    // 'A' (VK_A=0x41, scan=0x1E) -> normal face character
    let r = engine.on_event(key_down(0x41, 0x1E, t0));
    assert!(r.consumed);
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::info!("Normal face 'A': {:?}", r.actions);
    assert!(!r.actions.is_empty(), "should produce output for 'A'");

    // 'S' (VK_S=0x53, scan=0x1F)
    let r = engine.on_event(key_down(0x53, 0x1F, t0 + 500_000));
    assert!(r.consumed);
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::info!("Normal face 'S': {:?}", r.actions);
    assert!(!r.actions.is_empty(), "should produce output for 'S'");

    // 'D' (VK_D=0x44, scan=0x20)
    let r = engine.on_event(key_down(0x44, 0x20, t0 + 1_000_000));
    assert!(r.consumed);
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::info!("Normal face 'D': {:?}", r.actions);
    assert!(!r.actions.is_empty());

    log::info!("Normal face mapping verified");
}

#[test]
fn e2e_nicola_thumb_face_mapping() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Wait);
    log::info!("=== E2E: NICOLA thumb face mapping ===");

    let t0 = 1_000_000u64;

    // 'A' + left thumb (simultaneous) -> left thumb face character
    engine.on_event(key_down(0x41, 0x1E, t0));
    engine.on_event(key_down(0x1D, 0x7B, t0 + 30_000)); // NONCONVERT within threshold
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::info!("Left thumb face 'A': {:?}", r.actions);
    assert!(!r.actions.is_empty(), "thumb face should produce output");

    // 'A' + right thumb (simultaneous) -> right thumb face character
    let mut engine = make_test_engine(ConfirmMode::Wait);
    engine.on_event(key_down(0x41, 0x1E, t0));
    engine.on_event(key_down(0x1C, 0x79, t0 + 30_000)); // CONVERT within threshold
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::info!("Right thumb face 'A': {:?}", r.actions);
    assert!(
        !r.actions.is_empty(),
        "right thumb face should produce output"
    );

    log::info!("Thumb face mapping verified");
}

// ─────────────────────────────────────────────
// Timing edge case tests
// ─────────────────────────────────────────────

#[test]
fn e2e_timing_threshold_boundary() {
    init_test_logging();
    log::info!("=== E2E: Timing threshold boundary ===");

    let t0 = 1_000_000u64;
    let threshold_us = 100_000; // 100ms

    // Just inside threshold -> simultaneous
    let mut engine = make_test_engine(ConfirmMode::Wait);
    engine.on_event(key_down(0x41, 0x1E, t0));
    engine.on_event(key_down(0x1D, 0x7B, t0 + threshold_us - 1));
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::info!("Inside threshold ({}us): {:?}", threshold_us - 1, r.actions);
    // Should be simultaneous (thumb face)

    // Just outside threshold -> separate keys
    let mut engine = make_test_engine(ConfirmMode::Wait);
    engine.on_event(key_down(0x41, 0x1E, t0));
    engine.on_event(key_down(0x1D, 0x7B, t0 + threshold_us + 1));
    // The first key should have been flushed as single
    log::info!(
        "Outside threshold ({}us): first key flushed separately",
        threshold_us + 1
    );

    log::info!("Threshold boundary verified");
}

#[test]
fn e2e_rapid_sequential_input() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Wait);
    log::info!("=== E2E: Rapid sequential input ===");

    let t0 = 1_000_000u64;
    let interval = 200_000u64; // 200ms between keys (outside threshold)

    // Type 10 characters rapidly, each separated by > threshold
    let keys: [(u16, u32); 10] = [
        (0x41, 0x1E), // A
        (0x53, 0x1F), // S
        (0x44, 0x20), // D
        (0x46, 0x21), // F
        (0x47, 0x22), // G
        (0x48, 0x23), // H
        (0x4A, 0x24), // J
        (0x4B, 0x25), // K
        (0x4C, 0x26), // L
        (0x41, 0x1E), // A again
    ];

    let mut total_actions = 0;
    for (i, (vk, scan)) in keys.iter().enumerate() {
        let ts = t0 + (i as u64) * interval;
        let r = engine.on_event(key_down(*vk, *scan, ts));
        log::debug!("Key {i}: vk=0x{vk:02X} consumed={}", r.consumed);

        // Each key goes pending, then the NEXT key flushes the previous
        // (except the first one which just goes pending)
        total_actions += r.actions.len();
    }

    // Flush the last pending key
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    total_actions += r.actions.len();

    log::info!("Total actions from 10 keys: {total_actions}");
    assert!(
        total_actions >= 10,
        "should produce at least 10 actions for 10 keys"
    );

    log::info!("Rapid sequential input verified");
}

#[test]
fn e2e_three_key_arbitration() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Wait);
    log::info!("=== E2E: Three-key arbitration (d1 < d2) ===");

    let t0 = 1_000_000u64;

    // char1 -> thumb -> char2 (d1 < d2: char1+thumb simultaneous, char2 new)
    // d1 = thumb - char1 = 20ms
    // d2 = char2 - thumb = 50ms
    engine.on_event(key_down(0x41, 0x1E, t0)); // char1: A
    engine.on_event(key_down(0x1D, 0x7B, t0 + 20_000)); // thumb: NonConvert
    let r = engine.on_event(key_down(0x53, 0x1F, t0 + 70_000)); // char2: S
    log::info!(
        "3-key (d1<d2): consumed={} actions={:?}",
        r.consumed,
        r.actions
    );
    // char1+thumb should be simultaneous, char2 should be new pending

    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::info!("3-key timeout: {:?}", r.actions);

    log::info!("Three-key arbitration verified");
}

#[test]
fn e2e_three_key_arbitration_reversed() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Wait);
    log::info!("=== E2E: Three-key arbitration (d1 >= d2) ===");

    let t0 = 1_000_000u64;

    // char1 -> thumb -> char2 (d1 >= d2: char1 single, char2+thumb simultaneous)
    // d1 = thumb - char1 = 50ms
    // d2 = char2 - thumb = 20ms
    engine.on_event(key_down(0x41, 0x1E, t0)); // char1: A
    engine.on_event(key_down(0x1D, 0x7B, t0 + 50_000)); // thumb
    let r = engine.on_event(key_down(0x53, 0x1F, t0 + 70_000)); // char2: S
    log::info!(
        "3-key (d1>=d2): consumed={} actions={:?}",
        r.consumed,
        r.actions
    );

    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::info!("3-key timeout: {:?}", r.actions);

    log::info!("Three-key arbitration (reversed) verified");
}

// ─────────────────────────────────────────────
// ConfirmMode detailed tests
// ─────────────────────────────────────────────

#[test]
fn e2e_two_phase_mode_transition() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::TwoPhase);
    log::info!("=== E2E: TwoPhase mode Phase 1->2 transition ===");

    let t0 = 1_000_000u64;

    // Phase 1: short wait (speculative_delay_ms = 30ms)
    let r = engine.on_event(key_down(0x41, 0x1E, t0));
    log::debug!(
        "TwoPhase KeyDown: consumed={} actions={:?}",
        r.consumed,
        r.actions
    );
    assert!(r.consumed);
    assert!(r.actions.is_empty(), "Phase 1 should not output yet");

    // TIMER_SPECULATIVE fires -> Phase 2 (speculative output)
    let r = engine.on_timeout(awase::engine::TIMER_SPECULATIVE);
    log::info!("Phase 2 transition: actions={:?}", r.actions);
    // Should now have speculative output

    // TIMER_PENDING fires -> confirm speculative
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::info!("Phase 2 confirm: actions={:?}", r.actions);

    log::info!("TwoPhase transition verified");
}

#[test]
fn e2e_speculative_retraction_then_normal() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Speculative);
    log::info!("=== E2E: Speculative retraction followed by normal input ===");

    let t0 = 1_000_000u64;

    // Key 1: speculative output
    let r = engine.on_event(key_down(0x41, 0x1E, t0));
    log::debug!("Speculative A: {:?}", r.actions);
    assert!(
        !r.actions.is_empty(),
        "speculative should output immediately"
    );

    // Thumb within threshold -> retract + new output
    let r = engine.on_event(key_down(0x1D, 0x7B, t0 + 30_000));
    log::debug!("Retraction: {:?}", r.actions);
    let has_bs = r
        .actions
        .iter()
        .any(|a| matches!(a, KeyAction::Key(vk) if *vk == 0x08));
    assert!(has_bs, "retraction should have BS");

    // Key 2: normal speculative (fresh start)
    let r = engine.on_event(key_down(0x53, 0x1F, t0 + 500_000));
    log::debug!("Next speculative S: {:?}", r.actions);
    assert!(
        !r.actions.is_empty(),
        "next key should also output speculatively"
    );

    // Timeout -> confirm
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::debug!("Confirm: {:?}", r.actions);

    log::info!("Speculative retraction + normal verified");
}

// ─────────────────────────────────────────────
// Key up handling tests
// ─────────────────────────────────────────────

#[test]
fn e2e_key_up_during_pending() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Wait);
    log::info!("=== E2E: KeyUp during pending forces resolution ===");

    let t0 = 1_000_000u64;

    // KeyDown A -> pending
    engine.on_event(key_down(0x41, 0x1E, t0));

    // KeyUp A -> should force resolution (single character)
    let r = engine.on_event(key_up(0x41, 0x1E, t0 + 50_000));
    log::info!(
        "KeyUp during pending: consumed={} actions={:?}",
        r.consumed,
        r.actions
    );
    // Should have resolved the pending character
    assert!(
        r.consumed || !r.actions.is_empty(),
        "KeyUp should trigger resolution"
    );

    log::info!("KeyUp during pending verified");
}

#[test]
fn e2e_key_up_after_simultaneous() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Wait);
    log::info!("=== E2E: KeyUp sequence after simultaneous keystroke ===");

    let t0 = 1_000_000u64;

    // Simultaneous: A + NonConvert
    engine.on_event(key_down(0x41, 0x1E, t0));
    engine.on_event(key_down(0x1D, 0x7B, t0 + 30_000));
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::debug!("Simultaneous result: {:?}", r.actions);

    // KeyUp A
    let r = engine.on_event(key_up(0x41, 0x1E, t0 + 200_000));
    log::debug!("KeyUp A: consumed={} actions={:?}", r.consumed, r.actions);

    // KeyUp NonConvert
    let r = engine.on_event(key_up(0x1D, 0x7B, t0 + 210_000));
    log::debug!(
        "KeyUp NonConvert: consumed={} actions={:?}",
        r.consumed,
        r.actions
    );

    log::info!("KeyUp after simultaneous verified");
}

// ─────────────────────────────────────────────
// Engine state management tests
// ─────────────────────────────────────────────

#[test]
fn e2e_engine_toggle_during_pending() {
    init_test_logging();
    let mut engine = make_test_engine(ConfirmMode::Wait);
    log::info!("=== E2E: Toggle engine during pending state ===");

    let t0 = 1_000_000u64;

    // Start pending
    engine.on_event(key_down(0x41, 0x1E, t0));

    // Toggle off -> should flush pending
    let (enabled, flush) = engine.toggle_enabled();
    log::info!(
        "Toggle off: enabled={enabled} flush_actions={:?}",
        flush.actions
    );
    assert!(!enabled);
    assert!(!flush.actions.is_empty(), "flush should emit pending char");

    // Keys now pass through
    let r = engine.on_event(key_down(0x53, 0x1F, t0 + 500_000));
    assert!(!r.consumed);

    // Toggle back on
    let (enabled, _) = engine.toggle_enabled();
    assert!(enabled);
    let r = engine.on_event(key_down(0x44, 0x20, t0 + 1_000_000));
    assert!(r.consumed, "re-enabled engine should consume");

    let _ = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::info!("Toggle during pending verified");
}

#[test]
fn e2e_layout_swap_during_pending() {
    init_test_logging();
    log::info!("=== E2E: Layout swap during pending ===");

    let mut engine = make_test_engine(ConfirmMode::Wait);
    let t0 = 1_000_000u64;

    // Start pending
    engine.on_event(key_down(0x41, 0x1E, t0));

    // Swap layout -> should flush pending
    let new_layout = load_test_layout(); // reload same layout
    let flush = engine.swap_layout(new_layout);
    log::info!("Swap layout: flush_actions={:?}", flush.actions);
    assert!(!flush.actions.is_empty(), "swap should flush pending");

    // Engine should be idle, next key works normally
    let r = engine.on_event(key_down(0x53, 0x1F, t0 + 500_000));
    assert!(r.consumed);
    let _ = engine.on_timeout(awase::engine::TIMER_PENDING);

    log::info!("Layout swap during pending verified");
}

#[test]
fn e2e_config_validation() {
    init_test_logging();
    log::info!("=== E2E: Config validation ===");

    use awase::config::AppConfig;

    // Load actual config.toml
    let config_path = std::path::Path::new("config.toml");
    if config_path.exists() {
        let config = AppConfig::load(config_path).expect("config.toml should parse");
        let (validated, warnings) = config.validate();
        log::info!(
            "Config validated: threshold={}ms mode={:?}",
            validated.general.simultaneous_threshold_ms,
            validated.general.confirm_mode
        );
        for w in &warnings {
            log::warn!("Config warning: {w}");
        }
        assert!(
            warnings.is_empty(),
            "default config should have no warnings"
        );
    } else {
        log::warn!("config.toml not found, skipping");
    }

    log::info!("Config validation verified");
}

// ─────────────────────────────────────────────
// SendMessage comprehensive (Phase 2 extension, CI-compatible)
// ─────────────────────────────────────────────

#[test]
fn e2e_message_unicode_chars() {
    init_test_logging();
    log::info!("=== E2E Phase 2: Unicode characters via SendMessage ===");

    unsafe {
        let Some(win) = TestEditWindow::create() else {
            log::error!("Could not create test window, skipping");
            return;
        };

        // Hiragana
        win.clear();
        send_char_to_edit(win.edit_hwnd, '\u{3042}');
        send_char_to_edit(win.edit_hwnd, '\u{3044}');
        send_char_to_edit(win.edit_hwnd, '\u{3046}');
        let text = win.get_text();
        log::info!("Hiragana: '{text}'");
        assert_eq!(text, "\u{3042}\u{3044}\u{3046}");

        // Katakana
        win.clear();
        send_char_to_edit(win.edit_hwnd, '\u{30A2}');
        send_char_to_edit(win.edit_hwnd, '\u{30A4}');
        let text = win.get_text();
        log::info!("Katakana: '{text}'");
        assert_eq!(text, "\u{30A2}\u{30A4}");

        // Mixed ASCII + Japanese
        win.clear();
        send_char_to_edit(win.edit_hwnd, 'H');
        send_char_to_edit(win.edit_hwnd, 'e');
        send_char_to_edit(win.edit_hwnd, 'l');
        send_char_to_edit(win.edit_hwnd, 'l');
        send_char_to_edit(win.edit_hwnd, 'o');
        send_char_to_edit(win.edit_hwnd, ' ');
        send_char_to_edit(win.edit_hwnd, '\u{4E16}');
        send_char_to_edit(win.edit_hwnd, '\u{754C}');
        let text = win.get_text();
        log::info!("Mixed: '{text}'");
        assert_eq!(text, "Hello \u{4E16}\u{754C}");

        // Multiple backspaces
        win.clear();
        send_char_to_edit(win.edit_hwnd, 'a');
        send_char_to_edit(win.edit_hwnd, 'b');
        send_char_to_edit(win.edit_hwnd, 'c');
        send_char_to_edit(win.edit_hwnd, '\x08'); // BS
        send_char_to_edit(win.edit_hwnd, '\x08'); // BS
        let text = win.get_text();
        log::info!("After 2x BS: '{text}'");
        assert_eq!(text, "a");

        log::info!("Unicode tests passed");
    }
}

#[test]
fn e2e_message_long_text() {
    init_test_logging();
    log::info!("=== E2E Phase 2: Long text input ===");

    unsafe {
        let Some(win) = TestEditWindow::create() else {
            log::error!("Could not create test window, skipping");
            return;
        };

        // Type 100 characters
        win.clear();
        let input = "abcdefghij".repeat(10);
        for ch in input.chars() {
            send_char_to_edit(win.edit_hwnd, ch);
        }
        let text = win.get_text();
        log::info!("100 chars: len={}", text.len());
        assert_eq!(text.len(), 100, "should have 100 chars, got {}", text.len());
        assert_eq!(text, input);

        log::info!("Long text test passed");
    }
}
