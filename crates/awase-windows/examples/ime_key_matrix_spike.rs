//! IME モードキー動作マトリクス計測スパイク（awase 非依存）。
//!
//! 目的: 「直接入力 / IME ON・入力なし / IME ON・入力中」の各状態で、無変換・変換・
//! ひらがな・英数・カタカナ・半角/全角などのキーを押したとき、GJI の
//! **IME 開閉**と**変換モード（ひらがな/半角英数など）**がどう変化するかを、
//! 複数の観測経路で並べて記録する。公開 Mozc ソースの keymap と実機 GJI の挙動が
//! 食い違う（ATOK プリセット）ことが分かったため、実機の測定で表を作る。
//!
//! 設計方針（ADR-176 較正ウィザードの反省）:
//! - **IME の状態は変えようとしない。** 状態はユーザーがキーで作り、本アプリは
//!   それを観測して表示・記録するだけ。
//! - キー押下は `WH_KEYBOARD_LL`（awase と同じ層）で捕捉し、押下前スナップショットと
//!   押下 +400ms / +1500ms のスナップショットの差分を1行に記録する。
//! - 観測経路（すべて並べて記録し、食い違い自体を情報にする）:
//!   - A: `ImmGetContext` + `ImmGetOpenStatus` / `ImmGetConversionStatus` /
//!     `ImmGetCompositionStringW(GCS_COMPSTR)`
//!   - B: `ImmGetDefaultIMEWnd` + `WM_IME_CONTROL`（`IMC_GETOPENSTATUS` /
//!     `IMC_GETCONVERSIONMODE`）— awase 本体と同型
//!   - T: TSF **スレッド** compartment（`ITfThreadMgr` を `ITfCompartmentMgr` に cast）
//!     の `KEYBOARD_OPENCLOSE` / `KEYBOARD_INPUTMODE_CONVERSION`
//!   - G: TSF **グローバル** compartment（旧スパイク手法 C。比較用）
//!
//! ## 使い方（Windows 実機）
//! 1. **awase を止める**（生キーの GJI 単体挙動を測るため）。
//! 2. `cargo build --example ime_key_matrix_spike -p awase-windows` を実行し、
//!    `target/debug/examples/ime_key_matrix_spike.exe` を起動。
//! 3. 上段の入力欄にフォーカスした状態で、3 状態それぞれで各キーを 1 回ずつ押す。
//!    - 直接入力: Ctrl+無変換 等で IME OFF にする（下の状態表示が `直接入力`）
//!    - IME ON・入力なし: かなキー等で ON にする（`IME ON・入力なし`）
//!    - IME ON・入力中: ひらがなで `ka` と打った未確定状態（`IME ON・入力中`）
//! 4. 各キー押下ごとに 1 行がログ欄と `ime_key_matrix_spike.log`（exe と同じ
//!    ディレクトリ）に追記される。

#![windows_subsystem = "windows"]
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::fmt::Write as _;
use std::io::Write as _;

use windows::core::{w, Interface, Result as WinResult, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetContext, ImmGetConversionStatus, ImmGetDefaultIMEWnd,
    ImmGetOpenStatus, ImmReleaseContext, IME_COMPOSITION_STRING, IME_CONVERSION_MODE,
    IME_SENTENCE_MODE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetFocus, SetFocus};
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITfCompartmentMgr, ITfThreadMgr,
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW,
    GetWindowTextLengthW, GetWindowTextW, KillTimer, MessageBoxW, PostQuitMessage, RegisterClassW,
    SendMessageTimeoutW, SendMessageW, SetTimer, SetWindowTextW, SetWindowsHookExW, ShowWindow,
    TranslateMessage, CW_USEDEFAULT, KBDLLHOOKSTRUCT, MB_ICONERROR, MB_OK, MSG, SMTO_ABORTIFHUNG,
    SW_SHOW, WH_KEYBOARD_LL, WINDOW_STYLE, WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_SETFOCUS,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE, WS_VSCROLL,
};

const WM_IME_CONTROL: u32 = 0x0283;
const IMC_GETCONVERSIONMODE: usize = 0x0001;
const IMC_GETOPENSTATUS: usize = 0x0005;
const GCS_COMPSTR: u32 = 0x0008;

const TIMER_ID: usize = 1;
/// 観測スナップショットを更新する周期。
const TIMER_INTERVAL_MS: u32 = 50;
/// 押下後に取る 2 回のスナップショットまでの遅延。
const AFTER_MS: [u64; 2] = [400, 1500];

const ES_MULTILINE: u32 = 0x0004;
const ES_READONLY: u32 = 0x0800;
const ES_AUTOVSCROLL: u32 = 0x0040;
const MAX_LOG_CHARS: usize = 20_000;
const EM_SETSEL: u32 = 177;
const EM_REPLACESEL: u32 = 194;

/// ある瞬間の観測値（`None`=取得失敗）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Snapshot {
    a_open: Option<bool>,
    a_conv: Option<u32>,
    b_open: Option<bool>,
    b_conv: Option<u32>,
    t_open: Option<i32>,
    t_conv: Option<i32>,
    g_open: Option<i32>,
    g_conv: Option<i32>,
    /// 未確定文字列（GCS_COMPSTR）。
    comp: Option<String>,
    /// 入力欄末尾の文字（確定済みテキストの観測）。
    edit_tail: String,
}

impl Snapshot {
    /// 押下前状態の自動分類（マトリクスの行ラベル）。
    /// 開閉は A と B の多数決を取り、割れたら `不明` にする。
    fn state_label(&self) -> &'static str {
        let open = match (self.a_open, self.b_open) {
            (Some(a), Some(b)) if a == b => Some(a),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            _ => None,
        };
        let composing = self.comp.as_deref().is_some_and(|s| !s.is_empty());
        match (open, composing) {
            (Some(false), _) => "直接入力",
            (Some(true), false) => "IME ON・入力なし",
            (Some(true), true) => "IME ON・入力中",
            (None, _) => "不明(A/B不一致)",
        }
    }

    fn compact(&self) -> String {
        let mut s = String::new();
        let _ = write!(
            s,
            "A(open={} conv={}) B(open={} conv={}) T(open={} conv={}) G(open={} conv={}) comp={:?} tail={:?}",
            fmt_bool(self.a_open),
            fmt_hex(self.a_conv),
            fmt_bool(self.b_open),
            fmt_hex(self.b_conv),
            fmt_i32(self.t_open),
            fmt_i32_hex(self.t_conv),
            fmt_i32(self.g_open),
            fmt_i32_hex(self.g_conv),
            self.comp.as_deref().unwrap_or("?"),
            self.edit_tail,
        );
        s
    }
}

fn fmt_bool(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "1",
        Some(false) => "0",
        None => "?",
    }
}
fn fmt_hex(v: Option<u32>) -> String {
    v.map_or_else(|| "?".to_string(), |x| format!("0x{x:02X}"))
}
fn fmt_i32(v: Option<i32>) -> String {
    v.map_or_else(|| "?".to_string(), |x| x.to_string())
}
fn fmt_i32_hex(v: Option<i32>) -> String {
    v.map_or_else(|| "?".to_string(), |x| format!("0x{x:02X}"))
}

/// 1 回のキー押下に対する記録待ちエントリ。
struct Pending {
    label: String,
    started_ms: u64,
    before: Snapshot,
    /// `AFTER_MS` の各時点で取れたスナップショット。
    afters: Vec<Snapshot>,
}

struct TsfState {
    _thread_mgr: ITfThreadMgr,
    thread_cmgr: Option<ITfCompartmentMgr>,
    global_cmgr: Option<ITfCompartmentMgr>,
}

thread_local! {
    static TSF_STATE: RefCell<Option<TsfState>> = const { RefCell::new(None) };
    static EDIT_HWND: RefCell<Option<HWND>> = const { RefCell::new(None) };
    static STATUS_HWND: RefCell<Option<HWND>> = const { RefCell::new(None) };
    static LOG_HWND: RefCell<Option<HWND>> = const { RefCell::new(None) };
    static LOG_BUF: RefCell<String> = const { RefCell::new(String::new()) };
    /// 直近の周期スナップショット（押下前状態として使う）。
    static LAST_SNAP: RefCell<Snapshot> = RefCell::new(Snapshot::default());
    /// フックが積んだ未処理のキー押下（タイマーで処理する）。
    static KEY_QUEUE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
    /// 押下中の VK（オートリピート抑止用）。
    static DOWN_KEYS: RefCell<std::collections::HashSet<u32>> = RefCell::new(std::collections::HashSet::new());
    static START: RefCell<Option<std::time::Instant>> = const { RefCell::new(None) };
}

fn now_ms() -> u64 {
    START.with(|s| {
        s.borrow().map_or(0, |t| {
            u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX)
        })
    })
}

fn key_name(vk: u32) -> Option<&'static str> {
    Some(match vk {
        0x1C => "変換",
        0x1D => "無変換",
        0x15 => "VK_KANA(0x15)",
        0x16 => "VK_IME_ON",
        0x1A => "VK_IME_OFF",
        0x19 => "VK_KANJI(0x19)",
        0xF0 => "英数(0xF0)",
        0xF1 => "カタカナ(0xF1)",
        0xF2 => "ひらがな(0xF2)",
        0xF3 => "半角/全角(0xF3)",
        0xF4 => "半角/全角(0xF4)",
        0xF5 => "ローマ字(0xF5)",
        0xF6 => "非ローマ字(0xF6)",
        0x08 => "BS",
        0x0D => "Enter",
        0x1B => "ESC",
        0x20 => "Space",
        _ => return None,
    })
}

// ─── 観測 ────────────────────────────────────────────────────────────────

fn observe_a(hwnd: HWND) -> (Option<bool>, Option<u32>, Option<String>) {
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            return (None, None, None);
        }
        let open = ImmGetOpenStatus(himc).as_bool();
        let mut conv = IME_CONVERSION_MODE::default();
        let mut sent = IME_SENTENCE_MODE::default();
        let conv_ok =
            ImmGetConversionStatus(himc, Some(&raw mut conv), Some(&raw mut sent)).as_bool();
        let len = ImmGetCompositionStringW(himc, IME_COMPOSITION_STRING(GCS_COMPSTR), None, 0);
        let comp = if len > 0 {
            let mut buf = vec![0u16; usize::try_from(len).unwrap_or(0) / 2 + 1];
            let n = ImmGetCompositionStringW(
                himc,
                IME_COMPOSITION_STRING(GCS_COMPSTR),
                Some(buf.as_mut_ptr().cast()),
                u32::try_from(len).unwrap_or(0),
            );
            let n = usize::try_from(n).unwrap_or(0) / 2;
            Some(String::from_utf16_lossy(&buf[..n.min(buf.len())]))
        } else {
            Some(String::new())
        };
        let _ = ImmReleaseContext(hwnd, himc);
        (Some(open), conv_ok.then_some(conv.0), comp)
    }
}

fn observe_b(hwnd: HWND) -> (Option<bool>, Option<u32>) {
    unsafe {
        let ime_wnd = ImmGetDefaultIMEWnd(hwnd);
        if ime_wnd.0.is_null() {
            return (None, None);
        }
        let ask = |cmd: usize| -> Option<usize> {
            let mut result: usize = 0;
            let ok = SendMessageTimeoutW(
                ime_wnd,
                WM_IME_CONTROL,
                WPARAM(cmd),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                100,
                Some(&raw mut result),
            );
            (ok.0 != 0).then_some(result)
        };
        (
            ask(IMC_GETOPENSTATUS).map(|v| v != 0),
            ask(IMC_GETCONVERSIONMODE).and_then(|v| u32::try_from(v).ok()),
        )
    }
}

fn read_compartment(cmgr: &ITfCompartmentMgr, guid: &windows::core::GUID) -> Option<i32> {
    unsafe {
        let c = cmgr.GetCompartment(guid).ok()?;
        let v = c.GetValue().ok()?;
        i32::try_from(&v).ok()
    }
}

fn observe_tsf() -> (Option<i32>, Option<i32>, Option<i32>, Option<i32>) {
    TSF_STATE.with(|s| {
        let s = s.borrow();
        let Some(s) = s.as_ref() else {
            return (None, None, None, None);
        };
        let t = s.thread_cmgr.as_ref();
        let g = s.global_cmgr.as_ref();
        (
            t.and_then(|c| read_compartment(c, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)),
            t.and_then(|c| read_compartment(c, &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION)),
            g.and_then(|c| read_compartment(c, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)),
            g.and_then(|c| read_compartment(c, &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION)),
        )
    })
}

fn edit_tail() -> String {
    let Some(edit) = EDIT_HWND.with(|e| *e.borrow()) else {
        return String::new();
    };
    unsafe {
        let len = usize::try_from(GetWindowTextLengthW(edit)).unwrap_or(0);
        if len == 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len + 1];
        let n = usize::try_from(GetWindowTextW(edit, &mut buf)).unwrap_or(0);
        let s = String::from_utf16_lossy(&buf[..n]);
        let chars: Vec<char> = s.chars().collect();
        let start = chars.len().saturating_sub(8);
        chars[start..].iter().collect()
    }
}

fn take_snapshot(hwnd: HWND) -> Snapshot {
    let target = unsafe { GetFocus() };
    let target = if target.0.is_null() { hwnd } else { target };
    let (a_open, a_conv, comp) = observe_a(target);
    let (b_open, b_conv) = observe_b(target);
    let (t_open, t_conv, g_open, g_conv) = observe_tsf();
    Snapshot {
        a_open,
        a_conv,
        b_open,
        b_conv,
        t_open,
        t_conv,
        g_open,
        g_conv,
        comp,
        edit_tail: edit_tail(),
    }
}

// ─── ログ ────────────────────────────────────────────────────────────────

fn log_file_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("ime_key_matrix_spike.log")))
        .unwrap_or_else(|| std::path::PathBuf::from("ime_key_matrix_spike.log"))
}

fn append_log(line: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file_path())
    {
        let _ = writeln!(f, "{line}");
    }
    LOG_BUF.with(|buf| {
        let mut buf = buf.borrow_mut();
        buf.push_str(line);
        buf.push_str("\r\n");
        if buf.len() > MAX_LOG_CHARS {
            let cut = buf.len() - MAX_LOG_CHARS;
            let cut = buf
                .char_indices()
                .map(|(i, _)| i)
                .find(|&i| i >= cut)
                .unwrap_or(0);
            buf.drain(0..cut);
        }
    });
    let Some(log_hwnd) = LOG_HWND.with(|h| *h.borrow()) else {
        return;
    };
    unsafe {
        let _ = SendMessageW(
            log_hwnd,
            EM_SETSEL,
            Some(WPARAM(usize::MAX)),
            Some(LPARAM(-1)),
        );
        let mut wide: Vec<u16> = line
            .encode_utf16()
            .chain([u16::from(b'\r'), u16::from(b'\n')])
            .collect();
        wide.push(0);
        let _ = SendMessageW(
            log_hwnd,
            EM_REPLACESEL,
            Some(WPARAM(1)),
            Some(LPARAM(wide.as_ptr() as isize)),
        );
    }
}

fn set_status(text: &str) {
    let Some(h) = STATUS_HWND.with(|h| *h.borrow()) else {
        return;
    };
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = SetWindowTextW(h, PCWSTR(wide.as_ptr()));
    }
}

fn diff_summary(before: &Snapshot, after: &Snapshot) -> String {
    let mut d = Vec::new();
    if before.a_open != after.a_open || before.b_open != after.b_open {
        d.push(format!(
            "open A:{}→{} B:{}→{}",
            fmt_bool(before.a_open),
            fmt_bool(after.a_open),
            fmt_bool(before.b_open),
            fmt_bool(after.b_open)
        ));
    }
    if before.a_conv != after.a_conv || before.b_conv != after.b_conv {
        d.push(format!(
            "conv A:{}→{} B:{}→{}",
            fmt_hex(before.a_conv),
            fmt_hex(after.a_conv),
            fmt_hex(before.b_conv),
            fmt_hex(after.b_conv)
        ));
    }
    if before.t_open != after.t_open || before.t_conv != after.t_conv {
        d.push(format!(
            "tsf-thread open:{}→{} conv:{}→{}",
            fmt_i32(before.t_open),
            fmt_i32(after.t_open),
            fmt_i32_hex(before.t_conv),
            fmt_i32_hex(after.t_conv)
        ));
    }
    if before.comp != after.comp {
        d.push(format!("comp:{:?}→{:?}", before.comp, after.comp));
    }
    if before.edit_tail != after.edit_tail {
        d.push(format!("tail:{:?}→{:?}", before.edit_tail, after.edit_tail));
    }
    if d.is_empty() {
        "Δなし".to_string()
    } else {
        format!("Δ {}", d.join(" / "))
    }
}

// ─── フック ──────────────────────────────────────────────────────────────

/// WH_KEYBOARD_LL コールバック。重い処理はせず、押下だけキューへ積む。
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let vk = kb.vkCode;
        let msg = u32::try_from(wparam.0).unwrap_or(0);
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
        if is_up {
            DOWN_KEYS.with(|d| {
                d.borrow_mut().remove(&vk);
            });
        } else if is_down {
            let first = DOWN_KEYS.with(|d| d.borrow_mut().insert(vk));
            // 監視対象のキー、または Ctrl/Shift 併用の何かは記録するが、
            // 通常の文字キーは (テキスト観測は snapshot に含まれるので) 記録しない。
            if first {
                if let Some(name) = key_name(vk) {
                    let ctrl = unsafe { GetAsyncKeyState(0x11) } < 0;
                    let shift = unsafe { GetAsyncKeyState(0x10) } < 0;
                    let mods = match (ctrl, shift) {
                        (true, true) => "Ctrl+Shift+",
                        (true, false) => "Ctrl+",
                        (false, true) => "Shift+",
                        (false, false) => "",
                    };
                    let label = format!("{mods}{name} vk=0x{vk:02X} scan=0x{:02X}", kb.scanCode);
                    KEY_QUEUE.with(|q| q.borrow_mut().push(label));
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

// ─── タイマー / ウィンドウ ────────────────────────────────────────────────

fn on_timer(hwnd: HWND) {
    let snap = take_snapshot(hwnd);
    let now = now_ms();

    // キュー→Pending。「押下前」は直近の周期スナップショット（キーの効果が出る前）。
    let queued: Vec<String> = KEY_QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    if !queued.is_empty() {
        let before = LAST_SNAP.with(|l| l.borrow().clone());
        for label in queued {
            PENDING.with(|p| {
                p.borrow_mut().push(Pending {
                    label,
                    started_ms: now,
                    before: before.clone(),
                    afters: Vec::new(),
                });
            });
        }
    }

    // 期限が来た after スナップショットを取り、全部揃ったら1行出力する。
    let mut finished: Vec<Pending> = Vec::new();
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        for e in p.iter_mut() {
            let idx = e.afters.len();
            if idx < AFTER_MS.len() && now >= e.started_ms + AFTER_MS[idx] {
                e.afters.push(snap.clone());
            }
        }
        let mut i = 0;
        while i < p.len() {
            if p[i].afters.len() >= AFTER_MS.len() {
                finished.push(p.remove(i));
            } else {
                i += 1;
            }
        }
    });
    for e in finished {
        let stamp = {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = t.as_secs() % 86_400;
            format!(
                "{:02}:{:02}:{:02}.{:03}Z",
                secs / 3600,
                (secs / 60) % 60,
                secs % 60,
                t.subsec_millis()
            )
        };
        append_log(&format!(
            "[{stamp}] KEY {}  状態={}",
            e.label,
            e.before.state_label()
        ));
        append_log(&format!("    前     : {}", e.before.compact()));
        append_log(&format!(
            "    +{}ms: {}",
            AFTER_MS[0],
            e.afters[0].compact()
        ));
        append_log(&format!(
            "    +{}ms: {}",
            AFTER_MS[1],
            e.afters[1].compact()
        ));
        append_log(&format!(
            "    差分(前→+{}ms): {}   (前→+{}ms): {}",
            AFTER_MS[0],
            diff_summary(&e.before, &e.afters[0]),
            AFTER_MS[1],
            diff_summary(&e.before, &e.afters[1]),
        ));
    }

    set_status(&format!(
        "現在の状態: {}   | {}",
        snap.state_label(),
        snap.compact()
    ));
    LAST_SNAP.with(|l| *l.borrow_mut() = snap);
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                on_timer(hwnd);
                LRESULT(0)
            }
            WM_SETFOCUS => {
                if let Some(edit) = EDIT_HWND.with(|e| *e.borrow()) {
                    let _ = SetFocus(Some(edit));
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), TIMER_ID);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn create_child(
    class: PCWSTR,
    parent: HWND,
    instance: windows::Win32::Foundation::HMODULE,
    style_extra: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> WinResult<HWND> {
    unsafe {
        let style = (WS_CHILD | WS_VISIBLE).0 | style_extra;
        CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            class,
            w!(""),
            WINDOW_STYLE(style),
            x,
            y,
            w,
            h,
            Some(parent),
            None,
            Some(instance.into()),
            None,
        )
    }
}

fn create_window() -> WinResult<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class_name = w!("ImeKeyMatrixSpikeWindowClass");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&raw const wc);
        let hwnd = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            class_name,
            w!("IME key matrix spike (awase 非依存)"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            980,
            720,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        // 上段: 打鍵する入力欄。
        let edit = create_child(w!("EDIT"), hwnd, instance, WS_BORDER.0, 10, 10, 940, 30)?;
        EDIT_HWND.with(|e| *e.borrow_mut() = Some(edit));
        // 中段: 現在状態の表示（STATIC、2 行分）。
        let status = create_child(w!("STATIC"), hwnd, instance, 0, 10, 48, 940, 40)?;
        STATUS_HWND.with(|h| *h.borrow_mut() = Some(status));
        // 下段: ログ欄。
        let log = create_child(
            w!("EDIT"),
            hwnd,
            instance,
            WS_BORDER.0 | ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL | WS_VSCROLL.0,
            10,
            96,
            940,
            580,
        )?;
        LOG_HWND.with(|h| *h.borrow_mut() = Some(log));

        let _ = SetFocus(Some(edit));
        let _ = ShowWindow(hwnd, SW_SHOW);
        Ok(hwnd)
    }
}

fn init_tsf() -> WinResult<()> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let thread_mgr: ITfThreadMgr =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
        let _client_id = thread_mgr.Activate()?;
        let thread_cmgr = thread_mgr.cast::<ITfCompartmentMgr>().ok();
        let global_cmgr = thread_mgr.GetGlobalCompartment().ok();
        TSF_STATE.with(|s| {
            *s.borrow_mut() = Some(TsfState {
                _thread_mgr: thread_mgr,
                thread_cmgr,
                global_cmgr,
            });
        });
    }
    Ok(())
}

fn report_fatal(msg: &str) {
    let title: Vec<u16> = "ime_key_matrix_spike: fatal error"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let text: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn run() -> WinResult<()> {
    START.with(|s| *s.borrow_mut() = Some(std::time::Instant::now()));
    let tsf_ok = init_tsf();
    let hwnd = create_window()?;

    append_log("=== IME key matrix spike (awase 非依存) ===");
    append_log("観測: A=IMM(ImmGet*) / B=WM_IME_CONTROL / T=TSFスレッドcompartment / G=TSFグローバルcompartment");
    append_log(
        "conv の目安: 0x19=ひらがな(NATIVE|FULLSHAPE|ROMAN) 0x10=半角英数(ROMAN) 0x00=直接入力系",
    );
    if let Err(e) = tsf_ok {
        append_log(&format!("[init] TSF初期化失敗: {e}（T/Gは使えません）"));
    }
    append_log("手順: awase を止める → 3状態(直接入力/IME ON・入力なし/IME ON・入力中)で各キーを1回ずつ押す");
    append_log("キー: 無変換 変換 ひらがな 英数 カタカナ 半角/全角（Ctrl併用も別途記録される）");
    append_log(&format!("ログファイル: {}", log_file_path().display()));
    append_log("");

    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) };
    if let Err(e) = &hook {
        append_log(&format!(
            "[init] キーフック失敗: {e}（キー押下が記録されません）"
        ));
    }

    unsafe {
        SetTimer(Some(hwnd), TIMER_ID, TIMER_INTERVAL_MS, None);
    }

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
    Ok(())
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        report_fatal(&format!("panic: {info}"));
    }));
    if let Err(e) = run() {
        report_fatal(&format!("error: {e}"));
    }
}
