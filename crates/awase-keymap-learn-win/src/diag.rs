#![allow(unsafe_code)]
//! 汚染（外部書き込み・物理入力・フォーカス喪失）の原因調査用イベントログ。
//!
//! 「汚染を検出(物理入力)」だけでは、どのキーが・どの窓が原因かが分からず
//! 実機で切り分けられなかった。フック・窓プロシージャ・IME通知がここへ1件ずつ
//! 記録し、汚染を検出した時点で`drain`して stderr へ出す（各行の先頭は学習
//! プロセス起動からの経過ms）。フックコールバックはLLフックのタイムアウトに
//! 掛からないよう、整形済み文字列を積むだけにする。

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use windows::Win32::Foundation::{CloseHandle, HWND, MAX_PATH};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
};

/// 直近これだけの件数だけ保持する（汚染が起きる間隔より十分長い）。
const CAPACITY: usize = 256;

static START: OnceLock<Instant> = OnceLock::new();
static EVENTS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

fn elapsed_ms() -> u128 {
    START.get_or_init(Instant::now).elapsed().as_millis()
}

/// 経過時刻の起点を学習プロセスの開始時点に固定する（最初に1回呼ぶ）。
pub fn init() {
    let _ = START.get_or_init(Instant::now);
}

/// イベントを1件記録する。
pub fn record(kind: &str, detail: &str) {
    let line = format!("+{}ms {kind}: {detail}", elapsed_ms());
    if let Ok(mut events) = EVENTS.lock() {
        if events.len() >= CAPACITY {
            events.pop_front();
        }
        events.push_back(line);
    }
}

/// 記録済みイベントを古い順に取り出して空にする。
pub fn drain() -> Vec<String> {
    EVENTS
        .lock()
        .map(|mut events| events.drain(..).collect())
        .unwrap_or_default()
}

fn wide_to_string(buf: &[u16], len: i32) -> String {
    let len = usize::try_from(len).unwrap_or(0).min(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// 窓の `hwnd/class/title/pid/exe` を1行で説明する。
pub fn describe_window(hwnd: HWND) -> String {
    if hwnd.0.is_null() {
        return "hwnd=null".to_owned();
    }
    let mut class = [0u16; 128];
    let class_len = unsafe { GetClassNameW(hwnd, &mut class) };
    let mut title = [0u16; 128];
    let title_len = unsafe { GetWindowTextW(hwnd, &mut title) };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
    format!(
        "hwnd={:?} class={:?} title={:?} pid={pid} exe={:?}",
        hwnd.0,
        wide_to_string(&class, class_len),
        wide_to_string(&title, title_len),
        process_exe(pid),
    )
}

/// 現在の前面窓の説明。
pub fn describe_foreground() -> String {
    describe_window(unsafe { GetForegroundWindow() })
}

fn process_exe(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return "(open failed)".to_owned();
    };
    let mut buf = [0u16; MAX_PATH as usize];
    let mut len = buf.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &raw mut len,
        )
    };
    let _ = unsafe { CloseHandle(handle) };
    if result.is_err() {
        return "(query failed)".to_owned();
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    path.rsplit('\\').next().unwrap_or(&path).to_owned()
}
