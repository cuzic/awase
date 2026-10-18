//! WinPE の VM の中で動かすキー記録アプリ(ADR-257)。キーボード入力を、引数のファイルへ1行ずつ書く。記録は2系統:
//!
//! - `KEY ...`: Raw Input(`MakeCode`・E0・`VKey`)。
//! - `WM ...`: ウィンドウメッセージ(`WM_KEYDOWN` 等。`wParam`・`lParam` のスキャンコードと拡張ビット)。判定はこちら。
//!
//! 観測(ADR-257 事実5・段階1の実測): Raw Input の `MakeCode`・E0 は Scancode Map の**前**の値のまま、`VKey` は**後**の値。
//! `WM_KEYDOWN` の `lParam` のスキャンコードは**後**の値。Raw Input の `VKey` と `wParam` は左右を区別しない(VK_CONTROL 等)が、
//! `WM` の拡張ビットで左右が分かる。`WH_KEYBOARD_LL` フックは WinPE では呼ばれない(登録は成功するがコールバックが0回、Q4)ので使わない。
//!
//! 起動の手順: Raw Input を登録し終えてから窓を出し、フォーカスを取り、`READY` を書く(登録前に届いた打鍵を取りこぼさない)。
//! 窓は緑。番兵キー F11(スキャンコード 0x57、写像の対象外)を受け取るとマゼンタに変わる(ホストが画面で早期終了を判断する補助)。
//! F12(スキャンコード 0x58)のメイクで終了する。F12 の判定は写像前の `MakeCode` に依存するので、シナリオで F12 の位置を
//! 写像しても終了できる(意図した性質)。
#![allow(unsafe_code)]

use std::fs::OpenOptions;
use std::io::Write;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;

use windows::core::{w, Result};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect, UpdateWindow,
    HGDIOBJ, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEKEYBOARD,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
    GetSystemMetrics, PostMessageW, PostQuitMessage, RegisterClassW, ShowWindow,
    TranslateMessage, MSG, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW,
    SetForegroundWindow, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_CLOSE, WM_DESTROY, WM_INPUT, WM_PAINT, WNDCLASSW, WS_EX_TOPMOST, WS_POPUP,
};

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static SENTINEL_SEEN: AtomicBool = AtomicBool::new(false);

fn log(line: &str) {
    if let Ok(mut guard) = LOG.lock() {
        if let Some(file) = guard.as_mut() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
            let _ = file.sync_all();
        }
    }
}

unsafe fn on_raw_input(lparam: LPARAM) {
    let handle = HRAWINPUT(lparam.0 as *mut _);
    let header = size_of::<RAWINPUTHEADER>() as u32;
    let mut size = 0u32;
    GetRawInputData(handle, RID_INPUT, None, &mut size, header);
    let mut buf = vec![0u8; (size as usize).max(size_of::<RAWINPUT>())];
    let got = GetRawInputData(
        handle,
        RID_INPUT,
        Some(buf.as_mut_ptr().cast()),
        &mut size,
        header,
    );
    if got != size {
        log(&format!("ERR GetRawInputData got={got} size={size}"));
        return;
    }
    let raw: RAWINPUT = std::ptr::read_unaligned(buf.as_ptr().cast());
    if raw.header.dwType != RIM_TYPEKEYBOARD.0 {
        return;
    }
    let k = raw.data.keyboard;
    let is_break = k.Flags & 1 != 0;
    let e0 = k.Flags & 2 != 0;
    let e1 = k.Flags & 4 != 0;
    log(&format!(
        "KEY make=0x{:04X} break={} e0={} e1={} vk=0x{:02X} msg=0x{:X}",
        k.MakeCode,
        u8::from(is_break),
        u8::from(e0),
        u8::from(e1),
        k.VKey,
        k.Message
    ));
    // F11(番兵)のメイクで窓をマゼンタにする。
    if k.MakeCode == 0x57 && !is_break {
        SENTINEL_SEEN.store(true, Ordering::SeqCst);
        let hwnd = HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        let _ = InvalidateRect(hwnd, None, true);
    }
    // F12 のメイクで終了する。
    if k.MakeCode == 0x58 && !is_break {
        let hwnd = HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _);
        let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // ウィンドウメッセージ経由の観測(LL フックが動かないときの代わり、かつ Q5 の第三の観測点)。
        // lParam の bit16-23 がスキャンコード、bit24 が拡張、wParam が仮想キー。
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP | WM_CHAR => {
            log(&format!(
                "WM msg=0x{:X} wp=0x{:02X} sc=0x{:02X} ext={}",
                msg,
                wparam.0,
                (lparam.0 >> 16) & 0xFF,
                (lparam.0 >> 24) & 1
            ));
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            // COLORREF は 0x00BBGGRR。緑(準備完了)、F11 を受け取ったらマゼンタ(青は WinPE の背景と紛れる)。
            let color = if SENTINEL_SEEN.load(Ordering::SeqCst) { 0x00FF_00FF } else { 0x0000_FF00 };
            let brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(color));
            FillRect(hdc, &rc, brush);
            let _ = DeleteObject(HGDIOBJ(brush.0));
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_INPUT => {
            on_raw_input(lparam);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() -> Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "probe.log".to_string());
    let file = OpenOptions::new().create(true).append(true).open(&path);
    match file {
        Ok(f) => *LOG.lock().unwrap() = Some(f),
        Err(e) => eprintln!("cannot open {path}: {e}"),
    }
    log("START");
    println!("scancode-probe: log={path}");
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let class = w!("ScancodeProbe");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        // 右下に小さな窓。コンソール(startnet.cmd の出力)を隠さず、ホストが画面から緑のピクセルを探して準備完了を判定する。
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            class,
            w!("probe"),
            WS_POPUP,
            sw - 320,
            sh - 220,
            300,
            200,
            None,
            None,
            hinst,
            None,
        )?;
        MAIN_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        let rid = RAWINPUTDEVICE {
            usUsagePage: 1,
            usUsage: 6,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };
        RegisterRawInputDevices(&[rid], size_of::<RAWINPUTDEVICE>() as u32)?;
        // 登録がすべて終わってから窓を出す。
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(hwnd);
        let _ = UpdateWindow(hwnd);
        log("READY");
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    log("END");
    Ok(())
}
