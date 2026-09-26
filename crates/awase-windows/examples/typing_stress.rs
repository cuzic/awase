//! 高速打鍵ストレス E2E ハーネス(awase 本体は変更しない、検出専用)。
//!
//! ユーザー報告「Zoom のチャット画面で超高速タイピングすると、キーを受け取りきれず不具合が出る」を、
//! CI(GitHub Actions windows-latest)で再現できるかを切り分けるためのハーネス。Zoom 本物は CI で扱えないので、
//! **人間より速い間隔**で NICOLA の打鍵列(単打・親指シフト同時打鍵・混在)を `SendInput` で注入し、
//! 入力先のテキストを読み戻して**期待文字列**と比べる。崩れ方(消失/入れ替わり/リテラル化/置換/余計な文字)は
//! `tools/e2e/ime_key_matrix/check_typing_stress.py` が分類する。このファイルは注入と記録だけを行う。
//!
//! ## 入力先(`--form=`)
//! - `edit`  : 素の Win32 単行 EDIT(IMM32 系。`ime_key_matrix_spike` の入力欄1と同じ)。
//! - `multi` : 複数行 EDIT(`ES_MULTILINE`+縦スクロール。同じ EDIT クラスだが別の編集実装経路)。
//! - `rich`  : 素の `RICHEDIT50W`(Msftedit。TSF text store を自前で持つ)。
//! - `tsf`   : `RICHEDIT50W` を `Chrome_RenderWidgetHostHWND` へスーパークラス化(ADR-193)。awase から
//!   `AppKind::TsfNative` 相当に見える決定的な入力先(親窓も `Chrome_WidgetWin_1`)。
//! CI で安定して動かせない Chrome・Zoom・UWP は対象外(フォーカス/起動が不確定でストレスと切り分けられない)。
//!
//! ## フラグ
//! `--form=edit|multi|rich|tsf` / `--mode=nicola|raw` / `--interval=MS`(1文字あたりの間隔。既定20) /
//! `--trials=N`(種別ごとの試行数。既定4) / `--len=N`(1試行の文字数。既定40) / `--seed=S` /
//! `--kinds=single,thumb,mixed` / `--layout=PATH`(.yab。既定 layout/nicola_keytop.yab) /
//! `--activate-gji`(GJI/MS-IME のプロファイルを有効化。CI 用) / `--msime`(有効化する IME を Microsoft IME に) /
//! `--no-awase`(awase を待たない。`--mode=raw` の対照実験用) / `--log=PATH`。
//! `--mode=raw` は awase なしで、期待文字列と同じ内容をローマ字の生キーで同じ速度で注入する対照実験
//! (入力先+IME 単体がその速度を受けられるかを、awase と切り離して見る)。
//!
//! ## 注入の作法
//! `dwExtraInfo = hook::TEST_INJECTION_MARKER`(`AWASE_TEST_INJECTION=1` の debug ビルド awase が物理キー扱い)。
//! 注入は QueryPerformanceCounter 相当の `Instant` の busy-wait で 1 イベントずつ行う。
//! 自己検証として、各試行で(1)予定時刻に対する実注入の遅れ、(2)`SendInput` の戻り値(落ち)、
//! (3)自プロセスの `WH_KEYBOARD_LL` フックに届いたイベント数と配送遅延を記録する。
//! フックは awase より後に張る(先に呼ばれる)ので、awase が食う前の配送を数えられる(awase が再インストールで
//! 前に出た場合は届数が減るので、届数 < 送信数は「注入の落ち」と断定せず参考値として扱うこと)。
//!
//! ## ログ
//! `[TS-JSON] {...}` の行(1行1JSON、type = config/focus/ready/trial/inject/abort/done)が機械可読の記録。
//! 完走マーカーは `=== 完了 ===`。

#![windows_subsystem = "windows"]
#![allow(unsafe_code)]

use std::io::Write as _;
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use awase::kana_table::KanaTable;
use awase::scanmap::{KeyboardModel, PhysicalPos};
use awase::types::VkCode;
use awase::yab::{FullwidthStrExt, YabFace, YabLayout, YabValue};
use serde_json::json;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThread, GetCurrentThreadId, SetThreadPriority,
    THREAD_PRIORITY_TIME_CRITICAL,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, CLSID_TF_ThreadMgr, ITfInputProcessorProfileMgr, ITfThreadMgr,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW,
    GetClassInfoExW, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, GetMessageW,
    GetWindowThreadProcessId, PostMessageW, PostQuitMessage, RegisterClassExW, SendMessageW,
    SetForegroundWindow, SetWindowsHookExW, ShowWindow, TranslateMessage, CW_USEDEFAULT,
    GUITHREADINFO, KBDLLHOOKSTRUCT, MSG, SW_SHOW, WH_KEYBOARD_LL, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_APP, WM_CLOSE, WM_DESTROY, WM_GETTEXT, WM_GETTEXTLENGTH, WM_KEYDOWN, WM_KEYUP, WM_SETTEXT,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSEXW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    WS_VSCROLL,
};

#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
}

/// スパイク/プローブと同じ目印(二重定義しない)。
const MARKER: usize = awase_windows::hook::TEST_INJECTION_MARKER;
const WM_TS_FRONT: u32 = WM_APP + 1;
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_AUTOHSCROLL: u32 = 0x0080;

const VK_MUHENKAN: u32 = 0x1D;
const SCAN_MUHENKAN: u16 = 0x7B;
const VK_HENKAN: u32 = 0x1C;
const SCAN_HENKAN: u16 = 0x79;
const VK_RETURN: u32 = 0x0D;
const VK_IME_OFF: u32 = 0x1A;
const VK_DBE_HIRAGANA: u32 = 0xF2;
const VK_IME_ON: u32 = 0x16;

/// IME を ON にするキーの候補(試す順)。GJI は VK_IME_ON(0x16)が確実(richedit_tsf_probe の実測)、
/// MS-IME 本体はひらがなキー(0xF2)で ON になる(sc-* 構成の実績)。効かなければ次の候補へ進む。
fn ime_on_key(step: usize) -> u32 {
    let order = if has_flag("--msime") {
        [VK_DBE_HIRAGANA, VK_IME_ON, 0x1C]
    } else {
        [VK_IME_ON, VK_DBE_HIRAGANA, 0x1C]
    };
    order[step % order.len()]
}

/// IME を OFF にそろえてから、`step` 番目の候補キーで ON にする(awase の belief と実状態をそろえる)。
fn turn_ime_on(step: usize) {
    press(VK_IME_OFF, 0x70, 50);
    sleep_ms(600);
    press(ime_on_key(step), 0x70, 50);
    sleep_ms(1500);
}

static TOP: AtomicIsize = AtomicIsize::new(0);
static CHILD: AtomicIsize = AtomicIsize::new(0);
static EPOCH: OnceLock<Instant> = OnceLock::new();
static LOG_PATH: OnceLock<String> = OnceLock::new();
static HOOK_EVENTS: Mutex<Vec<HookEv>> = Mutex::new(Vec::new());
static FOREIGN_EVENTS: AtomicU64 = AtomicU64::new(0);

fn hwnd_of(v: &AtomicIsize) -> HWND {
    HWND(v.load(Ordering::SeqCst) as *mut core::ffi::c_void)
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn epoch_us() -> u64 {
    u64::try_from(EPOCH.get_or_init(Instant::now).elapsed().as_micros()).unwrap_or(u64::MAX)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn utc_stamp() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let secs = (t / 1000) % 86_400;
    format!(
        "[{:02}:{:02}:{:02}.{:03}Z]",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60,
        t % 1000
    )
}

fn log(line: &str) {
    let path = LOG_PATH
        .get()
        .map_or("typing_stress.log", String::as_str)
        .to_string();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{} {line}", utc_stamp());
    }
}

/// 機械可読の記録(1行1JSON)。
fn rec(v: &serde_json::Value) {
    log(&format!("[TS-JSON] {v}"));
}

fn arg_value(key: &str) -> Option<String> {
    std::env::args().find_map(|a| a.strip_prefix(key).map(str::to_string))
}

fn has_flag(flag: &str) -> bool {
    std::env::args().any(|a| a == flag)
}

// ---------------------------------------------------------------- 入力先の窓

#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    Edit,
    Multi,
    Rich,
    Tsf,
}

impl Form {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "edit" => Some(Self::Edit),
            "multi" => Some(Self::Multi),
            "rich" => Some(Self::Rich),
            "tsf" => Some(Self::Tsf),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Multi => "multi",
            Self::Rich => "rich",
            Self::Tsf => "tsf",
        }
    }
}

fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..usize::try_from(n).unwrap_or(0)])
}

unsafe extern "system" fn top_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TS_FRONT => {
                front_and_focus(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// 前面化(前面スレッドへ入力をアタッチする定番の回避策)と入力欄へのフォーカス。メインスレッドで呼ぶ。
fn front_and_focus(top: HWND) {
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
        let _ = BringWindowToTop(top);
        let _ = SetForegroundWindow(top);
        let child = hwnd_of(&CHILD);
        if !child.0.is_null() {
            let _ = SetFocus(Some(child));
        }
        if attached {
            let _ = AttachThreadInput(my_tid, fg_tid, false);
        }
    }
}

fn create_form(form: Form) -> HWND {
    unsafe {
        let _ = LoadLibraryW(w!("Msftedit.dll"));
        let instance = GetModuleHandleW(None).expect("module");
        let top_class = if form == Form::Tsf {
            "Chrome_WidgetWin_1"
        } else {
            "TypingStressTop"
        };
        let top_w = wide(top_class);
        let top_wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(top_proc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(top_w.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&raw const top_wc);
        let top = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(top_w.as_ptr()),
            w!("typing stress"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            760,
            360,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("top window");

        let (class, style, w_px, h_px): (String, u32, i32, i32) = match form {
            Form::Edit => ("EDIT".into(), WS_BORDER.0 | ES_AUTOHSCROLL, 700, 28),
            Form::Multi => (
                "EDIT".into(),
                WS_BORDER.0 | ES_MULTILINE | ES_AUTOVSCROLL | WS_VSCROLL.0,
                700,
                240,
            ),
            Form::Rich => ("RICHEDIT50W".into(), WS_BORDER.0 | ES_AUTOHSCROLL, 700, 240),
            Form::Tsf => {
                // RICHEDIT50W をスーパークラス化して、Chrome の描画窓のクラス名で登録し直す(ADR-193)。
                let mut wc = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    ..Default::default()
                };
                let got = GetClassInfoExW(None, w!("RICHEDIT50W"), &raw mut wc);
                log(&format!(
                    "[init] GetClassInfoExW(RICHEDIT50W) ok={}",
                    got.is_ok()
                ));
                let nm = wide("Chrome_RenderWidgetHostHWND");
                wc.lpszClassName = PCWSTR(nm.as_ptr());
                wc.hInstance = instance.into();
                let atom = RegisterClassExW(&raw const wc);
                log(&format!("[init] superclass atom={atom}"));
                (
                    "Chrome_RenderWidgetHostHWND".into(),
                    WS_BORDER.0 | ES_AUTOHSCROLL,
                    700,
                    240,
                )
            }
        };
        let cw = wide(&class);
        let child = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(cw.as_ptr()),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | style),
            10,
            10,
            w_px,
            h_px,
            Some(top),
            None,
            Some(instance.into()),
            None,
        )
        .unwrap_or_else(|e| {
            log(&format!("[FATAL] 入力欄の作成に失敗: class={class} {e}"));
            std::process::exit(2);
        });
        TOP.store(top.0 as isize, Ordering::SeqCst);
        CHILD.store(child.0 as isize, Ordering::SeqCst);
        let _ = ShowWindow(top, SW_SHOW);
        let _ = SetFocus(Some(child));
        top
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

/// 前面窓が `top`、かつそのスレッドのフォーカスが入力欄にあるか。
fn focus_ok() -> bool {
    unsafe {
        if GetForegroundWindow() != hwnd_of(&TOP) {
            return false;
        }
        let mut gi = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(0, &raw mut gi).is_ok() && gi.hwndFocus == hwnd_of(&CHILD)
    }
}

fn focus_report() -> serde_json::Value {
    unsafe {
        let fg = GetForegroundWindow();
        let mut gi = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let got = GetGUIThreadInfo(0, &raw mut gi).is_ok();
        json!({"type":"focus","fg_class":class_of(fg),"focus_class":class_of(gi.hwndFocus),
               "gui_thread_info_ok":got,"on_target":focus_ok()})
    }
}

fn refocus() {
    unsafe {
        let _ = PostMessageW(Some(hwnd_of(&TOP)), WM_TS_FRONT, WPARAM(0), LPARAM(0));
    }
    sleep_ms(400);
}

// ---------------------------------------------------------------- IME プロファイル

/// GJI(既定)または Microsoft IME(`--msime`)の TSF プロファイルをセッション内で有効化する(spike と同じ手順)。
fn activate_profile() {
    let (clsid, profile) = if has_flag("--msime") {
        (
            windows::core::GUID::from_u128(0x03B5835F_F03C_411B_9CE2_AA23E1171E36),
            windows::core::GUID::from_u128(0xA76C93D9_5523_4E90_AAFA_4DB112F9AC76),
        )
    } else {
        (
            windows::core::GUID::from_u128(0xD5A86FD5_5308_47EA_AD16_9C4EB160EC3C),
            windows::core::GUID::from_u128(0x773EB24E_CA1D_4B1B_B420_FA985BB0B80D),
        )
    };
    const TF_PROFILETYPE_INPUTPROCESSOR: u32 = 1;
    const TF_IPPMF_ENABLEPROFILE: u32 = 0x1;
    const TF_IPPMF_FORSESSION: u32 = 0x2000_0000;
    unsafe {
        let mgr: windows::core::Result<ITfInputProcessorProfileMgr> =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER);
        match mgr {
            Ok(m) => {
                let r = m.ActivateProfile(
                    TF_PROFILETYPE_INPUTPROCESSOR,
                    0x0411,
                    &clsid,
                    &profile,
                    windows::Win32::UI::Input::KeyboardAndMouse::HKL(std::ptr::null_mut()),
                    TF_IPPMF_ENABLEPROFILE | TF_IPPMF_FORSESSION,
                );
                log(&format!("[init] IMEプロファイルをアクティブ化: {r:?}"));
                sleep_ms(1500);
            }
            Err(e) => log(&format!("[init] ITfInputProcessorProfileMgr取得失敗: {e}")),
        }
    }
}

// ---------------------------------------------------------------- キー注入

struct Ev {
    t_us: u64,
    vk: u32,
    scan: u16,
    down: bool,
}

fn send_key(vk: u32, scan: u16, down: bool) -> bool {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(u16::try_from(vk).unwrap_or(0)),
                wScan: scan,
                dwFlags: if down {
                    KEYBD_EVENT_FLAGS(0)
                } else {
                    KEYEVENTF_KEYUP
                },
                time: 0,
                dwExtraInfo: MARKER,
            },
        },
    };
    unsafe { SendInput(&[input], size_of::<INPUT>() as i32) == 1 }
}

fn press(vk: u32, scan: u16, hold_ms: u64) {
    send_key(vk, scan, true);
    sleep_ms(hold_ms);
    send_key(vk, scan, false);
}

fn wait_until(target: Instant) {
    loop {
        let now = Instant::now();
        if now >= target {
            return;
        }
        let rem = target - now;
        if rem > Duration::from_micros(2500) {
            std::thread::sleep(rem - Duration::from_micros(2000));
        } else {
            std::hint::spin_loop();
        }
    }
}

#[derive(Default)]
struct InjectStats {
    planned: usize,
    sent_ok: usize,
    late_us: Vec<u64>,
    span_us: u64,
    /// 注入時刻(`epoch_us`)を予定順に。フック到着との突き合わせ用。
    inject_at: Vec<u64>,
}

fn run_schedule(evs: &[Ev]) -> InjectStats {
    let mut st = InjectStats {
        planned: evs.len(),
        ..Default::default()
    };
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
    }
    let t0 = Instant::now() + Duration::from_millis(20);
    for e in evs {
        let target = t0 + Duration::from_micros(e.t_us);
        wait_until(target);
        let now = Instant::now();
        st.late_us
            .push(u64::try_from(now.saturating_duration_since(target).as_micros()).unwrap_or(0));
        st.inject_at.push(epoch_us());
        if send_key(e.vk, e.scan, e.down) {
            st.sent_ok += 1;
        }
    }
    st.span_us =
        u64::try_from(Instant::now().saturating_duration_since(t0).as_micros()).unwrap_or(0);
    st
}

fn percentile(v: &mut [u64], p: usize) -> u64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[(v.len() - 1) * p / 100]
}

// ---------------------------------------------------------------- 自己検証フック

#[derive(Clone, Copy)]
struct HookEv {
    at_us: u64,
    down: bool,
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let msg = u32::try_from(wparam.0).unwrap_or(0);
        if kb.dwExtraInfo == MARKER {
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if down || up {
                if let Ok(mut g) = HOOK_EVENTS.lock() {
                    g.push(HookEv {
                        at_us: epoch_us(),
                        down,
                    });
                }
            }
        } else {
            FOREIGN_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn start_hook_thread() {
    std::thread::spawn(|| unsafe {
        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) {
            Ok(_) => log("[init] 自己検証フックを設置(awaseより後=先に呼ばれる)"),
            Err(e) => {
                log(&format!("[init] 自己検証フック失敗: {e}"));
                return;
            }
        }
        let mut msg = MSG::default();
        while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    });
}

// ---------------------------------------------------------------- 打鍵列の生成

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Face {
    Single,
    Left,
    Right,
}

#[derive(Clone)]
struct Cell {
    face: Face,
    vk: u32,
    scan: u16,
    romaji: String,
    kana: char,
}

fn vk_scan_for_pos(pos: PhysicalPos) -> Option<(u32, u16)> {
    let scan = awase_windows::scanmap::pos_to_scan(KeyboardModel::Jis, pos)?;
    // pos → VK は awase-vkmap の逆引き(全 VK を走査)。
    let vk = (0u16..=0xFF).find(|v| awase_vkmap::vk_to_pos(VkCode(*v)) == Some(pos))?;
    Some((u32::from(vk), u16::try_from(scan.0).ok()?))
}

/// .yab の面から、確定文字が単一かなに一意に決まるローマ字セルだけを集める(機械的に決める)。
/// 除外: 小書き(l/x 始まり)・ヴ(v 始まり)・`nn`(直後の母音との結合が IME ごとに違う)。
fn collect_cells(face: &YabFace, kind: Face, table: &KanaTable) -> Vec<Cell> {
    let mut v = Vec::new();
    for row in 0..4u8 {
        for col in 0..13u8 {
            let pos = PhysicalPos::new(row, col);
            let Some(YabValue::Romaji { romaji, .. }) = face.get(&pos) else {
                continue;
            };
            let r = romaji.to_halfwidth_str().to_ascii_lowercase();
            if r.starts_with(['l', 'x', 'v']) || r == "nn" {
                continue;
            }
            let Some(kana) = table.kana_for_romaji(&r) else {
                continue;
            };
            let Some((vk, scan)) = vk_scan_for_pos(pos) else {
                continue;
            };
            v.push(Cell {
                face: kind,
                vk,
                scan,
                romaji: r,
                kana,
            });
        }
    }
    v
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[usize::try_from(self.next() % v.len() as u64).unwrap_or(0)]
    }
}

fn gen_sequence(kind: &str, len: usize, seed: u64, cells: &[Vec<Cell>; 3]) -> Vec<Cell> {
    let mut rng = Rng(seed | 1);
    for _ in 0..8 {
        rng.next();
    }
    (0..len)
        .map(|_| {
            let idx = match kind {
                "single" => 0,
                "thumb" => 1 + usize::try_from(rng.next() % 2).unwrap_or(0),
                _ => usize::try_from(rng.next() % 3).unwrap_or(0),
            };
            rng.pick(&cells[idx]).clone()
        })
        .collect()
}

/// NICOLA の打鍵列を、`iv_us`(1文字あたりの間隔)で並べたイベント列にする。
/// 単打: 押下 0 → 解放 0.5。親指シフト: 親指押下 0 → 文字押下 0.25 → 文字解放 0.6 → 親指解放 0.7(すべて iv 比)。
fn nicola_events(seq: &[Cell], iv_us: u64) -> Vec<Ev> {
    let mut evs = Vec::new();
    for (i, c) in seq.iter().enumerate() {
        let t = i as u64 * iv_us;
        let at = |num: u64| t + iv_us * num / 100;
        match c.face {
            Face::Single => {
                evs.push(Ev {
                    t_us: at(0),
                    vk: c.vk,
                    scan: c.scan,
                    down: true,
                });
                evs.push(Ev {
                    t_us: at(50),
                    vk: c.vk,
                    scan: c.scan,
                    down: false,
                });
            }
            Face::Left | Face::Right => {
                let (tvk, tscan) = if c.face == Face::Left {
                    (VK_MUHENKAN, SCAN_MUHENKAN)
                } else {
                    (VK_HENKAN, SCAN_HENKAN)
                };
                for (num, vk, scan, down) in [
                    (0, tvk, tscan, true),
                    (25, c.vk, c.scan, true),
                    (60, c.vk, c.scan, false),
                    (70, tvk, tscan, false),
                ] {
                    evs.push(Ev {
                        t_us: at(num),
                        vk,
                        scan,
                        down,
                    });
                }
            }
        }
    }
    evs
}

/// 対照実験: 同じ文字列を、awase なしでローマ字の生キーとして打つ。1文字ぶんの間隔を各英字で等分する。
fn raw_events(seq: &[Cell], iv_us: u64) -> Vec<Ev> {
    let mut evs = Vec::new();
    for (i, c) in seq.iter().enumerate() {
        let n = c.romaji.len().max(1) as u64;
        for (j, ch) in c.romaji.bytes().enumerate() {
            let vk = u32::from(ch.to_ascii_uppercase());
            let pos = awase_vkmap::vk_to_pos(VkCode(u16::try_from(vk).unwrap_or(0)));
            let scan = pos
                .and_then(|p| awase_windows::scanmap::pos_to_scan(KeyboardModel::Jis, p))
                .and_then(|s| u16::try_from(s.0).ok())
                .unwrap_or(0);
            let t = i as u64 * iv_us + j as u64 * iv_us / n;
            evs.push(Ev {
                t_us: t,
                vk,
                scan,
                down: true,
            });
            evs.push(Ev {
                t_us: t + iv_us / n / 2,
                vk,
                scan,
                down: false,
            });
        }
    }
    evs
}

// ---------------------------------------------------------------- シナリオ

/// awase を起動する CI では、awase.log が現れてからさらに待つ(起動直後の TIP 検出・belief 同期のため)。
fn wait_for_awase() {
    for _ in 0..120 {
        if std::fs::metadata("awase.log").is_ok_and(|m| m.len() > 0) {
            break;
        }
        sleep_ms(500);
    }
    log("[init] awase.log を確認。起動後の落ち着きを待つ");
    sleep_ms(10_000);
}

fn expect_string(seq: &[Cell]) -> String {
    seq.iter().map(|c| c.kana).collect()
}

/// 注入の1イベントごとに、フックに届いた時刻との差(配送遅延)を出す。個数が合わなければ空。
fn delivery_stats(st: &InjectStats, hook: &[HookEv]) -> (usize, u64, u64) {
    if hook.len() != st.inject_at.len() {
        return (hook.len(), 0, 0);
    }
    let mut lat: Vec<u64> = hook
        .iter()
        .zip(&st.inject_at)
        .map(|(h, i)| h.at_us.saturating_sub(*i))
        .collect();
    let p50 = percentile(&mut lat, 50);
    let mx = lat.last().copied().unwrap_or(0);
    (hook.len(), p50, mx)
}

/// ハーネス自身の入力欄の IME が実際に開いているか(同一プロセスの IMM。取れなければ `None`)。
///
/// ready 確認の出力「か」は、awase が Engine ON のまま Unicode で送っても一致する(BUG-166: MS-IME が
/// 最初の 0xF2 を受け付けず閉のままでも ready が通り、以後の単打が生ローマ字になった)ため、
/// 実 IME の開閉を別に確認する。
fn real_ime_open(child: HWND) -> Option<bool> {
    // SAFETY: 自プロセスの入力欄の HWND に対する IMM 呼び出し。取得した HIMC は必ず解放する。
    unsafe {
        let himc = windows::Win32::UI::Input::Ime::ImmGetContext(child);
        if himc.is_invalid() {
            return None;
        }
        let open = windows::Win32::UI::Input::Ime::ImmGetOpenStatus(himc).as_bool();
        let _ = windows::Win32::UI::Input::Ime::ImmReleaseContext(child, himc);
        Some(open)
    }
}

fn ime_ready(raw: bool, cells: &[Vec<Cell>; 3], child: HWND) -> bool {
    // かなキー(NICOLA 単打 `ka`→か、raw なら k,a)を1回、ゆっくり打って確定し、IME と awase が効いているかを確かめる。
    let probe = cells[0]
        .iter()
        .find(|c| c.romaji == "ka")
        .cloned()
        .or_else(|| cells[0].first().cloned());
    let Some(c) = probe else {
        return false;
    };
    for attempt in 1..=3 {
        clear_text(child);
        sleep_ms(200);
        if raw {
            for ch in c.romaji.bytes() {
                let vk = u32::from(ch.to_ascii_uppercase());
                press(vk, 0, 60);
                sleep_ms(60);
            }
        } else {
            press(c.vk, c.scan, 60);
        }
        sleep_ms(700);
        press(VK_RETURN, 0x1C, 50);
        sleep_ms(700);
        let text = read_text(child);
        let open = real_ime_open(child);
        // `None`(取れない)は通す: ts-chrome* は入力欄が別プロセス(Chrome)で HIMC を取れないため。
        // 自プロセスの入力欄(edit/tsf/rich/multi)では CI で 41/41 回とも値が取れた(run 36224603306)。
        let ok = text.trim() == c.kana.to_string() && open != Some(false);
        rec(
            &json!({"type":"ready","attempt":attempt,"text":text,"expect":c.kana.to_string(),"ime_open":open,"ok":ok}),
        );
        if ok {
            clear_text(child);
            return true;
        }
        // 次の候補キーで IME を ON にし直す。
        turn_ime_on(attempt);
    }
    false
}

fn worker(form: Form) {
    let child = hwnd_of(&CHILD);
    let raw = arg_value("--mode=").as_deref() == Some("raw");
    let iv_ms: f64 = arg_value("--interval=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20.0);
    let iv_us = (iv_ms * 1000.0) as u64;
    let trials: usize = arg_value("--trials=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let len: usize = arg_value("--len=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let seed: u64 = arg_value("--seed=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let kinds: Vec<String> = arg_value("--kinds=")
        .unwrap_or_else(|| "single,thumb,mixed".into())
        .split(',')
        .map(str::to_string)
        .collect();
    let layout_path = arg_value("--layout=").unwrap_or_else(|| "layout/nicola_keytop.yab".into());
    let ime = if has_flag("--msime") { "msime" } else { "gji" };

    // 期待文字列の元になるセル(.yab の各面 × かな表)。
    let table = KanaTable::build();
    let layout = std::fs::read_to_string(&layout_path)
        .map_err(|e| e.to_string())
        .and_then(|t| YabLayout::parse(&t, KeyboardModel::Jis).map_err(|e| e.to_string()));
    let layout = match layout {
        Ok(l) => l,
        Err(e) => {
            rec(
                &json!({"type":"abort","reason":format!("layout 読み込み失敗: {layout_path}: {e}")}),
            );
            finish();
            return;
        }
    };
    let cells: [Vec<Cell>; 3] = [
        collect_cells(&layout.normal, Face::Single, &table),
        collect_cells(&layout.left_thumb, Face::Left, &table),
        collect_cells(&layout.right_thumb, Face::Right, &table),
    ];
    rec(
        &json!({"type":"config","form":form.name(),"ime":ime,"mode":if raw {"raw"} else {"nicola"},
        "interval_ms":iv_ms,"len":len,"trials":trials,"seed":seed,"kinds":kinds,
        "layout":layout_path,"cells":[cells[0].len(),cells[1].len(),cells[2].len()],
        "child_class":class_of(child)}),
    );
    if cells.iter().any(Vec::is_empty) {
        rec(&json!({"type":"abort","reason":"候補セルが空(layout の読み取り失敗?)"}));
        finish();
        return;
    }

    sleep_ms(500);
    refocus();
    log("[TS] READY-FOR-AWASE");
    if !has_flag("--no-awase") {
        wait_for_awase();
    }
    refocus();
    rec(&focus_report());
    if !focus_ok() {
        rec(
            &json!({"type":"abort","reason":"前面化またはフォーカスに失敗したためキーを注入しない"}),
        );
        finish();
        return;
    }
    start_hook_thread();
    sleep_ms(500);

    // IME を ON にそろえる(OFF → ひらがな)。
    turn_ime_on(0);
    if !ime_ready(raw, &cells, child) {
        rec(&json!({"type":"abort","reason":"IME/awase の準備確認に失敗(ready の text を参照)"}));
        finish();
        return;
    }

    let mut n_total = 0u64;
    for kind in &kinds {
        for t in 0..trials {
            n_total += 1;
            if !focus_ok() {
                refocus();
            }
            if !focus_ok() {
                rec(
                    &json!({"type":"abort","reason":format!("試行前にフォーカスが外れた kind={kind} n={t}")}),
                );
                finish();
                return;
            }
            let trial_seed = seed
                .wrapping_mul(1_000_003)
                .wrapping_add(n_total * 7919)
                .wrapping_add(kind.len() as u64);
            let seq = gen_sequence(kind, len, trial_seed, &cells);
            let expect = expect_string(&seq);
            let evs = if raw {
                raw_events(&seq, iv_us)
            } else {
                nicola_events(&seq, iv_us)
            };
            clear_text(child);
            sleep_ms(300);
            if let Ok(mut g) = HOOK_EVENTS.lock() {
                g.clear();
            }
            let stats = run_schedule(&evs);
            // 最後の同時打鍵判定・出力の落ち着きを待ってから確定(Enter)。
            sleep_ms(300);
            // 注入したキーだけのフック到着を、確定キー(Enter)を打つ前に確定させる。
            let hook: Vec<HookEv> = HOOK_EVENTS.lock().map(|g| g.clone()).unwrap_or_default();
            sleep_ms(600);
            press(VK_RETURN, 0x1C, 50);
            sleep_ms(1200);
            let actual = read_text(child);
            let (seen, deliv_p50, deliv_max) = delivery_stats(&stats, &hook);
            let downs = hook.iter().filter(|h| h.down).count();
            let mut late = stats.late_us.clone();
            let late_p95 = percentile(&mut late, 95);
            let late_max = late.last().copied().unwrap_or(0);
            let seq_desc: Vec<String> = seq
                .iter()
                .map(|c| {
                    let f = match c.face {
                        Face::Single => "",
                        Face::Left => "L+",
                        Face::Right => "R+",
                    };
                    format!("{f}{}", c.romaji)
                })
                .collect();
            rec(
                &json!({"type":"trial","kind":kind,"n":t,"chars":seq.len(),"expect":expect,
                "actual":actual,"keys":seq_desc.join(" "),"focus_ok":focus_ok()}),
            );
            rec(
                &json!({"type":"inject","kind":kind,"n":t,"planned":stats.planned,"sent_ok":stats.sent_ok,
                "late_p95_us":late_p95,"late_max_us":late_max,"span_ms":stats.span_us / 1000,
                "planned_span_ms":evs.last().map_or(0, |e| e.t_us / 1000),
                "hook_seen":seen,"hook_down":downs,"deliver_p50_us":deliv_p50,"deliver_max_us":deliv_max,
                "foreign_events_total":FOREIGN_EVENTS.load(Ordering::Relaxed)}),
            );
            clear_text(child);
            sleep_ms(300);
        }
    }
    finish();
}

fn finish() {
    rec(&json!({"type":"done"}));
    log("=== 完了 ===");
    unsafe {
        let _ = PostMessageW(Some(hwnd_of(&TOP)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

fn main() {
    let log_path = arg_value("--log=").unwrap_or_else(|| "typing_stress.log".into());
    let _ = LOG_PATH.set(log_path.clone());
    let _ = std::fs::remove_file(&log_path);
    EPOCH.get_or_init(Instant::now);
    std::panic::set_hook(Box::new(|info| {
        log(&format!("[FATAL] panic: {info}"));
    }));
    let form_arg = arg_value("--form=").unwrap_or_else(|| "edit".into());
    let Some(form) = Form::parse(&form_arg) else {
        log(&format!("[FATAL] 引数エラー: --form={form_arg}"));
        std::process::exit(2);
    };
    unsafe {
        timeBeginPeriod(1);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
        // TSF スレッドマネージャを有効化しておく(spike と同じ)。
        let tm: windows::core::Result<ITfThreadMgr> =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER);
        if let Ok(tm) = &tm {
            let _ = tm.Activate();
        }
    }
    let _top = create_form(form);
    if has_flag("--activate-gji") || has_flag("--msime") {
        activate_profile();
    }
    std::thread::spawn(move || worker(form));
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
}
