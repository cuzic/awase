//! ADR-193 スパイク: RichEdit(TSFネイティブ)をスーパークラス化して、awase の TsfNative 経路を
//! 決定的にテストできるかを確かめる。
//!
//! 仕組み: `RICHEDIT50W`(Msftedit.dll、TSF text store を持つ本物の RichEdit)を `GetClassInfoExW` で取得し、
//! 別のクラス名（既定 `Chrome_RenderWidgetHostHWND`）で `RegisterClassExW` し直す(スーパークラス化)。
//! awase の分類はクラス名の文字列一致(`focus/class_names.rs`)なので、フォーカス窓のクラス名が Chrome のものなら
//! `AppKind::TsfNative` / `AppImeProfile::Imm32Unavailable` として扱われるはず、という仮説の検証。
//! 上位の窓も `Chrome_WidgetWin_1` にする。確定文字列は `WM_GETTEXT` で厳密に読める。
//!
//! 使い方: `richedit_tsf_probe [--class=<クラス名>|--plain] [--repeat=N] [--idle=MS] [--log=<path>]`
//!   `--plain`: スーパークラス化せず素の `RICHEDIT50W`(対照)。
//! キーは `SendInput`（`AWASE_TEST_INJECTION=1` の awase が物理キー扱いする目印付き）で注入する。
//! 実行中は Windows 機のキーボード・マウスに触らない。awase を先に起動しておくこと。

use std::io::Write as _;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClassInfoExW,
    GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, GetMessageW, GetWindowThreadProcessId,
    PostMessageW, PostQuitMessage, RegisterClassExW, SendMessageW, SetForegroundWindow, ShowWindow,
    TranslateMessage, CW_USEDEFAULT, GUITHREADINFO, MSG, SW_SHOW, WM_CLOSE, WM_DESTROY, WM_GETTEXT,
    WM_GETTEXTLENGTH, WM_SETTEXT, WNDCLASSEXW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};

/// スパイク/chrome_probe と同じ目印。`AWASE_TEST_INJECTION=1` の awase は、この目印の注入を物理キーとして扱う。
const AUTO_MARKER: usize = awase_windows::hook::TEST_INJECTION_MARKER;

static RICH: AtomicIsize = AtomicIsize::new(0);
static TOP: AtomicIsize = AtomicIsize::new(0);

fn hwnd_of(v: &AtomicIsize) -> HWND {
    HWND(v.load(Ordering::SeqCst) as *mut core::ffi::c_void)
}

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

struct Log(std::fs::File);
impl Log {
    fn line(&mut self, s: &str) {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let secs = (t / 1000) % 86_400;
        let l = format!(
            "[{:02}:{:02}:{:02}.{:03}Z] {s}",
            secs / 3600,
            (secs / 60) % 60,
            secs % 60,
            t % 1000
        );
        println!("{l}");
        let _ = writeln!(self.0, "{l}");
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..usize::try_from(n).unwrap_or(0)])
}

fn scan_for(vk: u32) -> u16 {
    match vk {
        0x4B => 0x25, // K
        0x41 => 0x1E, // A
        _ => 0,
    }
}

fn send_key(vk: u32, down: bool) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(u16::try_from(vk).unwrap_or(0)),
                wScan: scan_for(vk),
                dwFlags: if down {
                    KEYBD_EVENT_FLAGS(0)
                } else {
                    KEYEVENTF_KEYUP
                },
                time: 0,
                dwExtraInfo: AUTO_MARKER,
            },
        },
    };
    unsafe {
        let _ = SendInput(&[input], size_of::<INPUT>() as i32);
    }
}

fn press(vk: u32, hold_ms: u64) {
    send_key(vk, true);
    sleep(hold_ms);
    send_key(vk, false);
}

fn bring_to_front(hwnd: HWND) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        let fg_tid = if fg.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let my_tid = GetCurrentThreadId();
        let attached =
            fg_tid != 0 && fg_tid != my_tid && AttachThreadInput(my_tid, fg_tid, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let ok = SetForegroundWindow(hwnd).as_bool();
        if attached {
            let _ = AttachThreadInput(my_tid, fg_tid, false);
        }
        ok || GetForegroundWindow() == hwnd
    }
}

fn read_text(h: HWND) -> String {
    unsafe {
        let len = SendMessageW(h, WM_GETTEXTLENGTH, None, None).0;
        let len = usize::try_from(len).unwrap_or(0);
        let mut buf = vec![0u16; len + 2];
        let got = SendMessageW(
            h,
            WM_GETTEXT,
            Some(WPARAM(len + 1)),
            Some(LPARAM(buf.as_mut_ptr() as isize)),
        )
        .0;
        String::from_utf16_lossy(&buf[..usize::try_from(got).unwrap_or(0)])
    }
}

fn clear_text(h: HWND) {
    unsafe {
        let empty = wide("");
        let _ = SendMessageW(h, WM_SETTEXT, None, Some(LPARAM(empty.as_ptr() as isize)));
    }
}

unsafe extern "system" fn top_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_DESTROY {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .find_map(|a| a.strip_prefix(key).map(str::to_string))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let plain = args.iter().any(|a| a == "--plain");
    let child_class =
        arg_value(&args, "--class=").unwrap_or_else(|| "Chrome_RenderWidgetHostHWND".to_string());
    let top_class = arg_value(&args, "--top-class=").unwrap_or_else(|| {
        if plain {
            "RicheditProbeTop".to_string()
        } else {
            "Chrome_WidgetWin_1".to_string()
        }
    });
    let repeat: usize = arg_value(&args, "--repeat=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let idle_ms: u64 = arg_value(&args, "--idle=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let log_path = arg_value(&args, "--log=").unwrap_or_else(|| "richedit_tsf_probe.log".into());
    let _ = std::fs::remove_file(&log_path);
    let mut log = Log(std::fs::File::create(&log_path).expect("log"));

    unsafe {
        // Msftedit.dll が RICHEDIT50W を登録する。
        let lib = LoadLibraryW(w!("Msftedit.dll"));
        log.line(&format!("LoadLibrary(Msftedit.dll) ok={}", lib.is_ok()));
        let instance = GetModuleHandleW(None).expect("module");

        // 上位窓(既定 Chrome_WidgetWin_1)。
        let top_w = wide(&top_class);
        let top_wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(top_proc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(top_w.as_ptr()),
            ..Default::default()
        };
        let top_atom = RegisterClassExW(&raw const top_wc);
        log.line(&format!("top class={top_class} atom={top_atom}"));
        let top = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            PCWSTR(top_w.as_ptr()),
            w!("RichEdit TSF probe"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            700,
            300,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("top window");

        // 子(RichEdit)。--plain は素の RICHEDIT50W、それ以外はスーパークラス化した別名クラス。
        let child_name = if plain {
            "RICHEDIT50W".to_string()
        } else {
            let mut wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                ..Default::default()
            };
            let got = GetClassInfoExW(None, w!("RICHEDIT50W"), &raw mut wc);
            log.line(&format!("GetClassInfoExW(RICHEDIT50W) ok={}", got.is_ok()));
            let nm = wide(&child_class);
            wc.lpszClassName = PCWSTR(nm.as_ptr());
            wc.hInstance = instance.into();
            let atom = RegisterClassExW(&raw const wc);
            log.line(&format!(
                "superclass RICHEDIT50W -> {child_class} atom={atom}"
            ));
            // nm はこのスコープを抜けると解放されるが、クラス名は RegisterClassExW 内でコピーされる。
            child_class.clone()
        };
        let child_w = wide(&child_name);
        let rich = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            PCWSTR(child_w.as_ptr()),
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_BORDER,
            10,
            10,
            660,
            220,
            Some(top),
            None,
            Some(instance.into()),
            None,
        );
        let Ok(rich) = rich else {
            log.line(&format!("CreateWindowExW({child_name}) 失敗: {rich:?}"));
            return;
        };
        log.line(&format!(
            "child class(実際)={} parent class={}",
            class_of(rich),
            class_of(top)
        ));
        let _ = ShowWindow(top, SW_SHOW);
        let _ = SetFocus(Some(rich));
        TOP.store(top.0 as isize, Ordering::SeqCst);
        RICH.store(rich.0 as isize, Ordering::SeqCst);
    }

    let worker = std::thread::spawn(move || {
        let top = hwnd_of(&TOP);
        let rich = hwnd_of(&RICH);
        let mut log = Log(std::fs::OpenOptions::new()
            .append(true)
            .open(
                arg_value(&std::env::args().collect::<Vec<_>>(), "--log=")
                    .unwrap_or_else(|| "richedit_tsf_probe.log".into()),
            )
            .expect("log"));
        sleep(1500);
        let fronted = bring_to_front(top);
        log.line(&format!("前面化: {fronted}"));
        sleep(800);
        unsafe {
            let fg = GetForegroundWindow();
            let mut gi = GUITHREADINFO {
                cbSize: size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            let _ = GetGUIThreadInfo(0, &raw mut gi);
            log.line(&format!(
                "FG class={} / focus class={}",
                class_of(fg),
                class_of(gi.hwndFocus)
            ));
        }
        for i in 1..=repeat {
            clear_text(rich);
            sleep(200);
            press(0x16, 40); // VK_IME_ON(冪等)
            sleep(500);
            if idle_ms > 0 {
                sleep(idle_ms);
            }
            press(0x4B, 30); // K
            sleep(30);
            press(0x41, 30); // A
            sleep(700);
            let text = read_text(rich);
            log.line(&format!("TEXT n={i} idle={idle_ms}ms text={text:?}"));
        }
        log.line("=== 完了 ===");
        unsafe {
            let _ = PostMessageW(Some(top), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    });

    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
    let _ = worker.join();
}
