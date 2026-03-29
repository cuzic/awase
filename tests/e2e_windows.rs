//! # ローカル実行方法
//!
//! ```powershell
//! # Phase 1 のみ（CI と同じ）
//! cargo test --test e2e_windows -- --nocapture 2>&1 | Tee-Object e2e.log
//!
//! # Phase 2-3 含む（ローカル Windows でのみ動作）
//! $env:AWASE_E2E_INTERACTIVE="1"
//! $env:RUST_LOG="debug"
//! cargo test --test e2e_windows -- --nocapture 2>&1 | Tee-Object e2e.log
//!
//! # ログを共有してデバッグ
//! # e2e.log をそのまま渡せば状況が分かる
//! ```

#![cfg(windows)]

use awase::config::ConfirmMode;
use awase::engine::Engine;
use awase::types::{ContextChange, KeyAction, KeyEventType, RawKeyEvent, ScanCode, VkCode};
use awase::yab::YabLayout;
use timed_fsm::TimedStateMachine;

use std::sync::Mutex;

/// Phase 2-3 テストは並列実行するとフォアグラウンドのフォーカスを奪い合う。
/// このロックで直列化する。
static INTERACTIVE_TEST_LOCK: Mutex<()> = Mutex::new(());

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
        100,          // threshold_ms
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

/// Phase 2-3 テスト冒頭でシステム診断情報をログ出力する
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
    log::info!(
        "Keyboard layout: HKL={:?} lang_id=0x{:04X}",
        hkl,
        lang_id
    );

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
// Phase 2: SendInput + Edit control
// ────────────────────────────────────────────

/// Phase 2-3 テストはインタラクティブなデスクトップセッションが必要。
/// CI 環境（GitHub Actions）では SendInput がフォアグラウンドウィンドウに届かないためスキップ。
///
/// 環境変数 `AWASE_E2E_INTERACTIVE=1` を設定すると強制実行。
fn is_interactive_session() -> bool {
    // 環境変数で明示的に有効化
    if std::env::var("AWASE_E2E_INTERACTIVE").map_or(false, |v| v == "1") {
        return true;
    }
    // CI 環境（GitHub Actions）ではスキップ
    if std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok() {
        log::info!("CI environment detected, skipping interactive tests");
        log::info!("Set AWASE_E2E_INTERACTIVE=1 to force execution");
        return false;
    }
    // デスクトップの有無
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;
        let desktop = GetDesktopWindow();
        !desktop.0.is_null()
    }
}

/// テスト用ウィンドウプロシージャ（DefWindowProcW に委譲）
unsafe extern "system" fn test_wnd_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 隠し Edit コントロール付きウィンドウを作成し、SendInput でキーを送信して
/// Edit の内容を読み取るヘルパー。
///
/// テスト終了時にウィンドウを破棄する。
struct TestEditWindow {
    hwnd: windows::Win32::Foundation::HWND,
    edit_hwnd: windows::Win32::Foundation::HWND,
}

impl TestEditWindow {
    unsafe fn create() -> Option<Self> {
        use windows::Win32::Foundation::{HINSTANCE, HWND};
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, GetKeyboardLayout};
        use windows::Win32::UI::WindowsAndMessaging::*;

        // ウィンドウクラス登録
        let class_name_wide: Vec<u16> = "AwaseTestWindow\0".encode_utf16().collect();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(test_wnd_proc),
            hInstance: HINSTANCE::default(),
            lpszClassName: windows::core::PCWSTR(class_name_wide.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&wc);

        // 親ウィンドウ作成
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

        // Edit コントロール作成
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
        log::info!(
            "Window created: hwnd={:?} class=AwaseTestWindow",
            hwnd
        );
        log::info!("Edit created: hwnd={:?} class=EDIT", edit_hwnd);

        // ウィンドウを表示してフォーカス設定
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(edit_hwnd);

        // メッセージを処理して描画を完了
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

    /// Edit コントロールのテキストを取得
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

    /// Edit コントロールのテキストをクリア
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

    /// Edit にフォーカスを設定
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

/// 保留中のウィンドウメッセージを処理する
unsafe fn pump_messages() {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::*;
    let mut msg = MSG::default();
    while PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE).as_bool() {
        DispatchMessageW(&msg);
    }
}

/// SendInput でキーストロークを送信する（フック非経由、直接 Edit に入力）
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

    // 入力が処理されるのを待つ
    std::thread::sleep(std::time::Duration::from_millis(50));
    pump_messages();

    // After pump_messages, check foreground
    let fg = GetForegroundWindow();
    let focus = GetFocus();
    log::debug!("After send: foreground={:?} focus={:?}", fg, focus);
}

#[test]
fn e2e_sendinput_edit_control() {
    init_test_logging();
    if !is_interactive_session() {
        log::warn!("Skipping SendInput test: no interactive desktop session");
        return;
    }
    let _lock = INTERACTIVE_TEST_LOCK.lock().unwrap();
    log::info!("=== E2E Phase 2: SendInput + Edit control ===");

    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

        log_system_info();

        let Some(win) = TestEditWindow::create() else {
            log::error!("Could not create test window, skipping");
            return;
        };

        // テスト 1: 単純なキー入力（'A' → 'a'）
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

        // テスト 2: 複数キー入力
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

        // テスト 3: Backspace
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

        log::info!("=== Phase 2 tests passed ===");
    }
}

#[test]
fn e2e_sendinput_special_keys() {
    init_test_logging();
    if !is_interactive_session() {
        log::warn!("Skipping special keys test: no interactive desktop session");
        return;
    }
    let _lock = INTERACTIVE_TEST_LOCK.lock().unwrap();
    log::info!("=== E2E Phase 2: Special keys ===");

    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

        log_system_info();

        let Some(win) = TestEditWindow::create() else {
            log::error!("Could not create test window, skipping");
            return;
        };

        // Enter キーは Edit で改行にならない（単行）
        log::info!("--- Test: Enter key in single-line Edit ---");
        win.clear();
        win.focus();
        send_key_to_edit(0x41, 0x1E); // A
        send_key_to_edit(0x0D, 0x1C); // VK_RETURN
        send_key_to_edit(0x42, 0x30); // B
        let text = win.get_text();
        log::info!("Edit content after A+Enter+B: '{text}'");
        // 単行 Edit では Enter は無視される
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

        log::info!("=== Special keys tests passed ===");
    }
}

// ────────────────────────────────────────────
// Phase 3: IME + NICOLA conversion
// ────────────────────────────────────────────

/// 日本語 IME が利用可能かチェック
unsafe fn is_japanese_ime_available() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
    // スレッドのキーボードレイアウトを確認
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

/// IME のオープン状態を設定する
unsafe fn set_ime_open(hwnd: windows::Win32::Foundation::HWND, open: bool) -> bool {
    use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext, ImmSetOpenStatus};
    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() {
        let err = windows::core::Error::from_win32();
        log::warn!(
            "ImmGetContext failed for hwnd={:?}: {:?}",
            hwnd,
            err
        );
        return false;
    }
    let result = ImmSetOpenStatus(himc, open);
    let _ = ImmReleaseContext(hwnd, himc);
    log::debug!("ImmSetOpenStatus({open}): result={:?}", result);
    result.as_bool()
}

/// IME のオープン状態を取得する
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
    let _lock = INTERACTIVE_TEST_LOCK.lock().unwrap();
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

        // 日本語 IME の有無を確認
        let has_japanese = is_japanese_ime_available();
        log::info!("Japanese IME available: {has_japanese}");

        if !has_japanese {
            log::warn!("Japanese IME not installed, skipping IME-specific tests");
            log::info!(
                "To enable: PowerShell -> New-WinUserLanguageList ja-JP -> Set-WinUserLanguageList"
            );
            return;
        }

        // IME OFF → Edit に直接入力
        log::info!("--- Test: IME OFF -> direct input ---");
        set_ime_open(win.edit_hwnd, false);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let ime_status = get_ime_open(win.edit_hwnd);
        log::info!("IME open status after OFF: {ime_status}");

        win.clear();
        send_key_to_edit(0x41, 0x1E); // A
        let text = win.get_text();
        log::info!("IME OFF, typed 'A': edit='{text}'");
        {
            let fg = GetForegroundWindow();
            let focus = GetFocus();
            assert!(
                text.contains('a') || text.contains('A'),
                "IME OFF: 'A' should produce 'a'\n\
                 Got: '{}'\n\
                 Foreground HWND: {:?}\n\
                 Focus HWND: {:?}\n\
                 Edit HWND: {:?}\n\
                 IME open: {}",
                text,
                fg,
                focus,
                win.edit_hwnd,
                ime_status
            );
        }

        // IME ON → ローマ字入力モードでの動作確認
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
        // IME ON で 'A' を入力 → IME が処理（ローマ字→かな変換）
        send_key_to_edit(0x41, 0x1E); // A
        std::thread::sleep(std::time::Duration::from_millis(200)); // IME 処理待ち
        pump_messages();
        let text = win.get_text();
        log::info!("IME ON, typed 'A': edit='{text}'");
        // IME ON + ローマ字モードなら 'あ' になるか、未確定文字列として 'a' が表示される
        log::info!("IME conversion result: '{text}' (expected: 'a' or 'a' in composition)");

        // IME OFF に戻す（クリーンアップ）
        set_ime_open(win.edit_hwnd, false);
        log::info!("=== Phase 3 IME status tests completed ===");
    }
}

#[test]
fn e2e_engine_with_ime_context() {
    init_test_logging();
    if !is_interactive_session() {
        log::warn!("Skipping engine+IME test: no interactive desktop session");
        return;
    }
    let _lock = INTERACTIVE_TEST_LOCK.lock().unwrap();
    log::info!("=== E2E Phase 3: Engine with IME context ===");

    unsafe {
        log_system_info();
    }

    // Engine を in-process で使い、IME 状態に応じた動作を確認
    let mut engine = make_test_engine(ConfirmMode::Wait);
    let t0 = 1_000_000u64;

    // エンジン有効 + 通常キー入力 → consumed
    let r = engine.on_event(key_down(0x41, 0x1E, t0));
    log::debug!("Engine enabled, 'A': consumed={}", r.consumed);
    assert!(r.consumed, "enabled engine should consume char key");

    // タイムアウトで確定
    let r = engine.on_timeout(awase::engine::TIMER_PENDING);
    log::debug!("Timeout: actions={:?}", r.actions);

    // エンジン無効化
    let (enabled, _flush) = engine.toggle_enabled();
    assert!(!enabled);
    let r = engine.on_event(key_down(0x41, 0x1E, t0 + 500_000));
    log::debug!("Engine disabled, 'A': consumed={}", r.consumed);
    assert!(!r.consumed, "disabled engine should passthrough");

    // 再有効化
    let (enabled, _flush) = engine.toggle_enabled();
    assert!(enabled);
    let r = engine.on_event(key_down(0x41, 0x1E, t0 + 1_000_000));
    log::debug!("Engine re-enabled, 'A': consumed={}", r.consumed);
    assert!(r.consumed, "re-enabled engine should consume");

    // flush で解放
    let _ = engine.flush_pending(ContextChange::ImeOff);

    log::info!("=== Phase 3 engine+IME tests completed ===");
}
