//! Chrome(TsfNative)向け ADR-186 実機E2Eプローブ（awase 非依存の観測側）。
//!
//! 目的: Win32 EDIT ではなく **実際の Chrome** で、無変換/変換/ひらがな/Shift+無変換 を押したときの
//! IME の状態を測る。Chrome(TsfNative)では IMM が使えず、他プロセスの TSF の中身も読めないため、
//! IME の内部状態を読むのをやめ、**ユーザー要件の結果**（打った文字）で状態を判定する:
//!   - Engine ON(NICOLA)       : `k`,`a` を打つと NICOLA の文字が出る（`か`でも`ka`でもない）
//!   - IME ON・かな, Engine OFF: `k`,`a` → `か`（ローマ字かな変換）
//!   - IME ON・半角英数/直接入力: `k`,`a` → `ka` のまま
//!
//! 仕組み: ローカルの小さな HTTP サーバー（標準ライブラリのみ）が検証ページを配り、ページの JS が
//! `keydown`/`compositionstart`/`beforeinput` 等を記録してサーバーへ送る。キーは `SendInput`
//! （`AWASE_TEST_INJECTION=1` の awase が物理キー扱いする目印付き）で注入する。専用プロファイルで
//! Chrome を起動するので、ユーザーの Chrome には触れない。
//!
//! `--keymatrix=<key>=<kind>:<gap>,...`(ADR-208 L3b): 「ずれの作り方 × 明示キー」行列。形式・記録は `typing_stress/keymatrix.rs` と同じ(ここでは `KM {json}` の行で出す)。
//! 判定は check_keymatrix.py。実 Chrome は IME の開閉を API で読めない(TsfNative)ので、押下ごとの打鍵(k,a)の結果を主証拠にする。
//!
//! `--offrca=<action>:<prep>,...`(MS-IME × 実 Chrome の OFF が閉じない件の原因切り分け): 各セルで IME を開いてから `action` で閉じようとし、
//! `IMC_GETOPENSTATUS` を 20ms 周期で `--or-poll=<ms>`(既定 4000)の間**打鍵せずに**ポーリングして閉じるまでの時間(遅延か、永久に閉じないか)を測る。
//! その後 k,a を打って実際の IME 状態(打鍵結果)を確認する。`--or-ladder` は閉じなかった試行で別手段(再送・IMC・0xF3・0x19・TSF 大域 compartment)を順に試す。
//! `--or-relaunch` は試行ごとに Chrome を起動し直す(ページ状態の蓄積の影響を切り分ける)。判定は check_offrca.py(`OFFRCA {json}` 行)。
//!
//! 使い方: `chrome_probe [--repeat=N] [--no-awase] [--f13] [--chrome=<chrome.exe>] [--log=<path>]`
//!   `--no-awase`: awase を止めた対照実験（かなのとき `か` を期待）。既定は awase 起動中（NICOLA を期待）。
//! 実行中は Windows 機のキーボード・マウスに触らない。

use std::io::{Read, Write as _};
use std::net::{TcpListener, TcpStream};
use std::os::windows::process::CommandExt as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, ITfInputProcessorProfileMgr,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateWindowExW, DispatchMessageW, FindWindowW, GetForegroundWindow,
    GetMessageW, GetWindowThreadProcessId, SendMessageW, SetForegroundWindow, SwitchToThisWindow,
    TranslateMessage, MSG, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

/// スパイクと同じ目印。`AWASE_TEST_INJECTION=1` の awase は、この目印の注入を物理キーとして扱う。
const AUTO_MARKER: usize = awase_windows::hook::TEST_INJECTION_MARKER;

const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><title>IMEPROBE</title>
<style>body{font:14px sans-serif;margin:8px}textarea{width:95%;height:140px;font-size:18px}</style>
<div>ADR-186 Chrome probe（触らないでください）</div>
<textarea id="t" autofocus></textarea>
<script>
const t = document.getElementById('t');
let seq = 0;
const enc = s => encodeURIComponent(s == null ? '' : String(s));
function ev(kind, o) {
  o = o || {};
  const f = [seq++, Date.now(), kind, document.hasFocus() ? 1 : 0, o.key, o.kc, o.comp ? 1 : 0, o.data, t.value, o.it].map(enc);
  fetch('/log', {method: 'POST', body: f.join('|'), keepalive: true});
}
for (const k of ['keydown', 'keyup'])
  t.addEventListener(k, e => ev(k, {key: e.key, kc: e.keyCode, comp: e.isComposing}));
for (const k of ['compositionstart', 'compositionupdate', 'compositionend'])
  t.addEventListener(k, e => ev(k, {data: e.data}));
for (const k of ['beforeinput', 'input'])
  t.addEventListener(k, e => ev(k, {data: e.data, it: e.inputType}));
async function poll() {
  try {
    const c = await (await fetch('/cmd')).text();
    if (c === 'clear') { t.blur(); t.value = ''; t.focus(); ev('cleared'); }
    else if (c === 'snap') { ev('snap'); }
  } catch (e) {}
  setTimeout(poll, 30);
}
window.addEventListener('focus', () => t.focus());
t.focus();
poll();
ev('ready');
</script>"#;

#[derive(Clone, Debug)]
struct PageEvent {
    n: u64,
    kind: String,
    focus: bool,
    key: String,
    kc: String,
    data: String,
    value: String,
    recv: Instant,
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_event(body: &str) -> Option<PageEvent> {
    let f: Vec<String> = body.split('|').map(pct_decode).collect();
    if f.len() < 10 {
        return None;
    }
    Some(PageEvent {
        n: f[0].parse().ok()?,
        kind: f[2].clone(),
        focus: f[3] == "1",
        key: f[4].clone(),
        kc: f[5].clone(),
        data: f[7].clone(),
        value: f[8].clone(),
        recv: Instant::now(),
    })
}

struct Shared {
    events: Vec<PageEvent>,
    cmd: Option<&'static str>,
}

fn handle(mut s: TcpStream, shared: &Arc<Mutex<Shared>>) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let (head_end, content_len) = loop {
        let n = match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
            let cl = head
                .lines()
                .find_map(|l| {
                    l.strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            break (p + 4, cl);
        }
    };
    while buf.len() < head_end + content_len {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let first = head.lines().next().unwrap_or("");
    let path = first.split_whitespace().nth(1).unwrap_or("/");
    let body =
        String::from_utf8_lossy(&buf[head_end..head_end + content_len.min(buf.len() - head_end)])
            .into_owned();
    let (ctype, resp): (&str, String) = if path == "/log" {
        if let Some(e) = parse_event(&body) {
            shared.lock().unwrap().events.push(e);
        }
        ("text/plain", String::new())
    } else if path == "/cmd" {
        let c = shared.lock().unwrap().cmd.take().unwrap_or("");
        ("text/plain", c.to_string())
    } else {
        ("text/html; charset=utf-8", PAGE.to_string())
    };
    let out = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{resp}",
        resp.len()
    );
    let _ = s.write_all(out.as_bytes());
}

fn utc_stamp() -> String {
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
}

fn scan_for(vk: u32) -> u16 {
    match vk {
        0x1D => 0x7B,               // 無変換
        0x1C => 0x79,               // 変換
        0xF2 => 0x70,               // ひらがな
        0xF0 => 0x3A,               // 英数
        0xF1 => 0x70,               // カタカナ(Shift 付きのひらがなキー)
        0xF3 | 0xF4 | 0x19 => 0x29, // 半角/全角・漢字
        0x7C => 0x64,               // F13
        0x7D => 0x65,               // F14
        0x4B => 0x25,               // K
        0x41 => 0x1E,               // A
        0xA0 => 0x2A,               // LShift
        0xA2 => 0x1D,               // LCtrl
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

fn send_ctrl_muhenkan() {
    send_key(0xA2, true);
    sleep(40);
    send_key(0x1D, true);
    sleep(60);
    send_key(0x1D, false);
    sleep(30);
    send_key(0xA2, false);
}

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

struct Log(std::fs::File);
impl Log {
    fn line(&mut self, s: &str) {
        let l = format!("[{}] {s}", utc_stamp());
        println!("{l}");
        let _ = writeln!(self.0, "{l}");
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    /// `ka` のまま（半角英数 or 直接入力）。
    Plain,
    /// `か`（IME ON・かな、Engine は素通し）。
    RomajiKana,
    /// それ以外のかな（NICOLA の文字、Engine ON）。
    Nicola,
    /// `kiu` のようなローマ字のまま（IME は英数/直接入力なのに Engine が ON で、NICOLA がローマ字を送っている）。
    NicolaLiteral,
    Empty,
    Other,
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Self::Plain => "ka(英数/直接)",
            Self::RomajiKana => "か(かな・Engine素通し)",
            Self::Nicola => "NICOLA文字(Engine ON)",
            Self::NicolaLiteral => "ローマ字のまま(英数なのにEngine ON=未追随)",
            Self::Empty => "空",
            Self::Other => "その他",
        }
    }
}

fn classify(text: &str) -> Class {
    let t = text.trim();
    if t.is_empty() {
        return Class::Empty;
    }
    if t.eq_ignore_ascii_case("ka") {
        return Class::Plain;
    }
    if t == "か" {
        return Class::RomajiKana;
    }
    if t.chars().any(|c| ('\u{3040}'..='\u{30FF}').contains(&c)) {
        return Class::Nicola;
    }
    if t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic()) {
        return Class::NicolaLiteral;
    }
    Class::Other
}

struct Probe {
    /// Shift+キーの押下で、キーを離してから Shift を離すまでの待ち(ms)。人は Shift を長く押す。
    shift_tail_ms: u64,
    shared: Arc<Mutex<Shared>>,
    log: Log,
    focus_lost: bool,
    /// true なら probe 後にページを空にしない(未確定の composition を残す。`--offrca` の `typed_nc` 用)。
    no_clear: bool,
}

impl Probe {
    fn press(&mut self, vk: u32, shift: bool, hold_ms: u64) {
        if shift {
            send_key(0xA0, true);
            sleep(40);
        }
        send_key(vk, true);
        sleep(hold_ms);
        send_key(vk, false);
        if shift {
            sleep(self.shift_tail_ms);
            send_key(0xA0, false);
        }
        self.log.line(&format!(
            "KEY vk=0x{vk:02X}{} (auto)",
            if shift { " +Shift" } else { "" }
        ));
    }

    fn last_n(&self) -> u64 {
        self.shared.lock().unwrap().events.last().map_or(0, |e| e.n)
    }

    fn command(&mut self, c: &'static str, wait_kind: &str) -> Option<PageEvent> {
        let before = self.shared.lock().unwrap().events.len();
        self.shared.lock().unwrap().cmd = Some(c);
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            sleep(20);
            let g = self.shared.lock().unwrap();
            if let Some(e) = g.events[before..].iter().find(|e| e.kind == wait_kind) {
                return Some(e.clone());
            }
        }
        None
    }

    /// `k`,`a` を打ち、出た文字で状態を判定する。終わったらページを空にする。
    fn probe(&mut self) -> (Class, String, bool) {
        let before = self.shared.lock().unwrap().events.len();
        self.press(0x4B, false, 30);
        sleep(30);
        self.press(0x41, false, 30);
        sleep(350);
        let snap = self.command("snap", "snap");
        let (text, focused) = match &snap {
            Some(e) => (e.value.clone(), e.focus),
            None => (String::new(), false),
        };
        if !focused {
            self.focus_lost = true;
        }
        // Chrome が IME に処理させたキー(`Process`/229)の有無も記録する。
        let process = self.shared.lock().unwrap().events[before..]
            .iter()
            .any(|e| e.kind == "keydown" && (e.key == "Process" || e.kc == "229"));
        if !self.no_clear {
            let _ = self.command("clear", "cleared");
        }
        sleep(150);
        (classify(&text), text, process)
    }

    fn probe_logged(&mut self, what: &str) -> Class {
        let (c, text, process) = self.probe();
        self.log.line(&format!(
            "PROBE {what}: {} text={text:?} Process(229)={process}",
            c.label()
        ));
        c
    }
}

#[derive(Clone, Copy)]
enum Setup {
    Kana,
    Off,
    Alnum,
}

/// 状態を「かな」「直接入力」「半角英数」に持っていく。状態はプローブ(打った文字)で確認する。
fn ensure(p: &mut Probe, setup: Setup, awase: bool) -> bool {
    let kana_ok = |c: Class| {
        if awase {
            c == Class::Nicola
        } else {
            c == Class::RomajiKana
        }
    };
    // 1) かなにする: IME ON → ダメならひらがなキーでかな⇔半角英数を切り替える。
    p.press(0x16, false, 40); // VK_IME_ON(冪等)
    sleep(500);
    let mut c = p.probe_logged("setup:IME_ON後");
    if !kana_ok(c) {
        p.press(0xF2, false, 60);
        sleep(500);
        c = p.probe_logged("setup:ひらがな後");
    }
    if !kana_ok(c) {
        return false;
    }
    match setup {
        Setup::Kana => true,
        Setup::Off => {
            p.press(0x1A, false, 40); // VK_IME_OFF(冪等)。convは開閉をまたいで保存される。
            sleep(500);
            p.probe_logged("setup:IME_OFF後") == Class::Plain
        }
        Setup::Alnum => {
            p.press(0xF2, false, 60);
            sleep(500);
            matches!(
                p.probe_logged("setup:ひらがな(かな→半角英数)後"),
                Class::Plain | Class::NicolaLiteral
            )
        }
    }
}

struct Case {
    name: &'static str,
    setup: Setup,
    vk: u32,
    shift: bool,
    /// true なら「かな」(awase起動中はNICOLA文字、停止中は`か`)、false なら `ka`。
    expect_kana: bool,
}

/// `--f13`(ADR-199 T1(e)、決定18): GJI の CUSTOM 表で F13 をトグル(DirectInput=IMEOn、他=IMEOff)にした構成の、実 Chrome での実タイピング。
/// 表の設定は呼び出し側(ワークフロー)。F13(0x7C)で閉↔開が切り替わり、awase の belief が追随して NICOLA 文字が出るか(`ka` か NICOLA 文字か)を見る。
const F13_CASES: [Case; 2] = [
    Case {
        name: "かな→F13=IME OFF",
        setup: Setup::Kana,
        vk: 0x7C,
        shift: false,
        expect_kana: false,
    },
    Case {
        name: "直接入力→F13=かなON",
        setup: Setup::Off,
        vk: 0x7C,
        shift: false,
        expect_kana: true,
    },
];

/// `--henkan-open`(ADR-209): GJI の MS-IME プリセット(keymap=2)で、IME OFF(直接入力)から変換を単独で押したとき、
/// IME が開き(TSF の実 Chrome)、awase の Engine が追随して NICOLA になること。全ケースは keymap=1(ATOK)前提の
/// 期待(「かな→変換=IME OFF」等)を含み MS-IME プリセットでは成り立たないので、このケースだけを走らせる。
const HENKAN_OPEN_CASES: [Case; 1] = [Case {
    name: "直接入力→変換=かなON(ADR-209)",
    setup: Setup::Off,
    vk: 0x1C,
    shift: false,
    expect_kana: true,
}];

const CASES: [Case; 8] = [
    Case {
        name: "かな→無変換=IME OFF",
        setup: Setup::Kana,
        vk: 0x1D,
        shift: false,
        expect_kana: false,
    },
    Case {
        name: "直接入力→無変換=かなON",
        setup: Setup::Off,
        vk: 0x1D,
        shift: false,
        expect_kana: true,
    },
    Case {
        name: "かな→ひらがな=半角英数",
        setup: Setup::Kana,
        vk: 0xF2,
        shift: false,
        expect_kana: false,
    },
    Case {
        name: "半角英数→ひらがな=かな",
        setup: Setup::Alnum,
        vk: 0xF2,
        shift: false,
        expect_kana: true,
    },
    Case {
        name: "かな→変換=IME OFF",
        setup: Setup::Kana,
        vk: 0x1C,
        shift: false,
        expect_kana: false,
    },
    Case {
        name: "直接入力→変換=かなON",
        setup: Setup::Off,
        vk: 0x1C,
        shift: false,
        expect_kana: true,
    },
    Case {
        name: "かな→Shift+無変換=半角英数",
        setup: Setup::Kana,
        vk: 0x1D,
        shift: true,
        expect_kana: false,
    },
    Case {
        name: "直接入力→Shift+無変換=直接入力のまま",
        setup: Setup::Off,
        vk: 0x1D,
        shift: true,
        expect_kana: false,
    },
];

fn find_chrome(arg: Option<String>) -> Option<String> {
    if let Some(a) = arg {
        return Some(a);
    }
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    [
        format!(r"{home}\scoop\apps\googlechrome\current\chrome.exe"),
        r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_string(),
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".to_string(),
        format!(r"{local}\Google\Chrome\Application\chrome.exe"),
    ]
    .into_iter()
    .find(|p| std::path::Path::new(p).exists())
}

/// タスクバーを前面にしてテスト窓からフォーカスを外す(`--refocus`。フォーカス変更イベントを awase に見せる)。
fn sleep_ms_away() {
    std::thread::sleep(std::time::Duration::from_millis(200));
}

/// `--settle-explicit` 用: Chrome 以外の別トップレベル窓(別スレッドの可視窓。CI にはタスクバーへ移せない環境がある)。
/// 作成済みなら使い回す。窓ハンドルは isize で保持する(スレッドをまたぐため)。
fn helper_window() -> Option<windows::Win32::Foundation::HWND> {
    use std::sync::OnceLock;
    static H: OnceLock<isize> = OnceLock::new();
    let raw = *H.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<isize>();
        std::thread::spawn(move || unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("IMEPROBE_AWAY"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                50,
                50,
                400,
                200,
                None,
                None,
                None,
                None,
            );
            let _ = tx.send(hwnd.map_or(0, |h| h.0 as isize));
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or(0)
    });
    (raw != 0).then(|| windows::Win32::Foundation::HWND(raw as *mut _))
}

/// 別窓(`helper_window`)へフォーカスを移し、前面になったことを検証する。
fn focus_away_to_helper() -> bool {
    let Some(hwnd) = helper_window() else {
        return false;
    };
    unsafe {
        for _ in 0..3 {
            let fg = GetForegroundWindow();
            let fg_tid = if fg.0.is_null() {
                0
            } else {
                GetWindowThreadProcessId(fg, None)
            };
            let my_tid = GetCurrentThreadId();
            let attached = fg_tid != 0
                && fg_tid != my_tid
                && AttachThreadInput(my_tid, fg_tid, true).as_bool();
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            SwitchToThisWindow(hwnd, true);
            if attached {
                let _ = AttachThreadInput(my_tid, fg_tid, false);
            }
            sleep(200);
            if GetForegroundWindow() == hwnd {
                return true;
            }
        }
    }
    false
}

fn focus_away() -> bool {
    unsafe {
        let Ok(tray) = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) else {
            return false;
        };
        let fg = GetForegroundWindow();
        let fg_tid = if fg.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let my_tid = GetCurrentThreadId();
        let attached =
            fg_tid != 0 && fg_tid != my_tid && AttachThreadInput(my_tid, fg_tid, true).as_bool();
        let mut ok = SetForegroundWindow(tray).as_bool();
        if !ok {
            // CI では SetForegroundWindow がタスクバーに対して拒否される(chrome_probe run 36530291568 で away=false)。
            SwitchToThisWindow(tray, true);
            sleep_ms_away();
            ok = GetForegroundWindow() == tray;
        }
        if attached {
            let _ = AttachThreadInput(my_tid, fg_tid, false);
        }
        ok
    }
}

fn bring_to_front() -> bool {
    unsafe {
        let hwnd = FindWindowW(PCWSTR::null(), w!("IMEPROBE")).unwrap_or_default();
        if hwnd.0.is_null() {
            return false;
        }
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

/// 前面の Chrome の既定 IME ウィンドウへ `WM_IME_CONTROL` を送る(awase を経由しない外部要因の再現。awase 自身が読む経路と同じ)。
/// `IMC_GETOPENSTATUS`=0x0005 / `IMC_SETOPENSTATUS`=0x0006。IME ウィンドウが取れなければ `None`。
fn ime_control(cmd: usize, value: isize) -> Option<isize> {
    const WM_IME_CONTROL: u32 = 0x0283;
    // SAFETY: 検証ページの窓の既定 IME ウィンドウへ同期 SendMessage するだけ。
    unsafe {
        let hwnd = FindWindowW(PCWSTR::null(), w!("IMEPROBE")).unwrap_or_default();
        let target = if hwnd.0.is_null() {
            GetForegroundWindow()
        } else {
            hwnd
        };
        let ime_wnd = ImmGetDefaultIMEWnd(target);
        if ime_wnd.0.is_null() {
            return None;
        }
        Some(
            SendMessageW(
                ime_wnd,
                WM_IME_CONTROL,
                Some(WPARAM(cmd)),
                Some(LPARAM(value)),
            )
            .0,
        )
    }
}

/// Microsoft IME の TSF プロファイルをこのセッションで有効化する(typing_stress.rs::activate_profile と同じ手順)。
/// 既定の入力方式の上書きだけでは、後から起動した Chrome が日本語 IME のレイアウトにならなかった(CI 観測)ため、
/// Chrome を起動する前に呼ぶ。
fn activate_msime_profile(log: &mut Log) {
    const TF_PROFILETYPE_INPUTPROCESSOR: u32 = 1;
    const TF_IPPMF_ENABLEPROFILE: u32 = 0x1;
    const TF_IPPMF_FORSESSION: u32 = 0x2000_0000;
    let clsid = windows::core::GUID::from_u128(0x03B5835F_F03C_411B_9CE2_AA23E1171E36);
    let profile = windows::core::GUID::from_u128(0xA76C93D9_5523_4E90_AAFA_4DB112F9AC76);
    // SAFETY: COM を初期化してプロファイルマネージャを呼ぶだけ。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
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
                log.line(&format!("MS-IME プロファイルをアクティブ化: {r:?}"));
                sleep(1500);
            }
            Err(e) => log.line(&format!("ITfInputProcessorProfileMgr取得失敗: {e}")),
        }
    }
}

/// `--keymatrix=` のセル指定(`typing_stress/keymatrix.rs` と同じ書式)。
#[derive(Clone)]
struct KmCell {
    label: String,
    key: String,
    vk: u32,
    ctrl: bool,
    /// `on` / `off` / `tog`
    kind: &'static str,
    /// `sync` / `close` / `open` / `fresh`
    gap: &'static str,
}

impl KmCell {
    /// ずれを作った直後に実 IME がとるべき状態(押す前の状態)。
    fn r0(&self) -> bool {
        match self.gap {
            "close" => false,
            "open" => true,
            _ => self.kind != "on",
        }
    }

    /// キーの意味に一致した状態(トグルは押す前の状態の反転)。
    fn target(&self) -> bool {
        match self.kind {
            "on" => true,
            "off" => false,
            _ => !self.r0(),
        }
    }
}

fn km_parse_cell(spec: &str) -> Result<KmCell, String> {
    let (key, rest) = spec
        .split_once('=')
        .ok_or_else(|| format!("セル指定に '=' が無い: {spec}"))?;
    let (kind, gap) = rest
        .split_once(':')
        .ok_or_else(|| format!("セル指定に ':' が無い: {spec}"))?;
    let ctrl = key.starts_with("ctrl+");
    let hex = key.strip_prefix("ctrl+").unwrap_or(key);
    let vk = u32::from_str_radix(hex.trim_start_matches("0x"), 16)
        .map_err(|e| format!("キーが16進でない: {key}: {e}"))?;
    let kind: &'static str = match kind {
        "on" => "on",
        "off" => "off",
        "tog" => "tog",
        other => return Err(format!("kind が不正: {other}")),
    };
    let gap: &'static str = match gap {
        "sync" => "sync",
        "close" => "close",
        "open" => "open",
        "fresh" => "fresh",
        other => return Err(format!("gap が不正: {other}")),
    };
    if (gap == "close" && kind == "off") || (gap == "open" && kind == "on") {
        return Err(format!(
            "意味と一致した状態から始まる組み合わせは測定にならない: {spec}"
        ));
    }
    Ok(KmCell {
        label: spec.to_string(),
        key: key.to_string(),
        vk,
        ctrl,
        kind,
        gap,
    })
}

fn km_send_key(c: &KmCell) {
    if c.ctrl {
        send_key(0xA2, true);
        sleep(40);
        send_key(c.vk, true);
        sleep(60);
        send_key(c.vk, false);
        sleep(30);
        send_key(0xA2, false);
    } else {
        send_key(c.vk, true);
        sleep(60);
        send_key(c.vk, false);
    }
}

fn km_kill_awase() {
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "awase.exe"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000)
        .status();
}

fn km_start_awase() -> bool {
    let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("awase.exe")))
    else {
        return false;
    };
    let Some(dir) = exe.parent() else {
        return false;
    };
    // RUST_LOG / AWASE_TEST_INJECTION は CI のステップが chrome_probe に渡した環境変数を引き継ぐ。
    std::process::Command::new(&exe)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()
        .is_ok()
}

fn km_open_of(c: Class) -> Option<bool> {
    match c {
        Class::Nicola | Class::RomajiKana => Some(true),
        Class::Plain | Class::NicolaLiteral => Some(false),
        Class::Empty | Class::Other => None,
    }
}

/// `--keymatrix=`。実 Chrome の検証ページを前面にしてから呼ぶ。`awase` が false の対照は対象外(awase を再起動して比べる構成のため)。
fn run_keymatrix(p: &mut Probe, spec: &str, args: &[String]) {
    let arg_u64 = |key: &str, default: u64| -> u64 {
        args.iter()
            .find_map(|a| a.strip_prefix(key).and_then(|v| v.parse().ok()))
            .unwrap_or(default)
    };
    let mut cells = Vec::new();
    for s in spec.split(',').filter(|s| !s.is_empty()) {
        match km_parse_cell(s) {
            Ok(c) => cells.push(c),
            Err(e) => {
                p.log.line(&format!("KM_ABORT {e}"));
                return;
            }
        }
    }
    let n = arg_u64("--km-n=", 10);
    let wait_ms = arg_u64("--km-wait=", 1000);
    let max_press = arg_u64("--km-max-press=", 3);
    let fresh_settle = arg_u64("--km-fresh-settle=", 10_000);
    let msime = args.iter().any(|a| a == "--msime");
    let labels: Vec<String> = cells.iter().map(|c| c.label.clone()).collect();
    p.log.line(&format!(
        "KM_CONFIG {}",
        serde_json::json!({"form":"chrome","ime":if msime {"msime"} else {"gji"},"cells":labels,
            "n":n,"wait_ms":wait_ms,"max_press":max_press,"fresh_settle_ms":fresh_settle,"evidence":"typed"})
    ));
    for cell in &cells {
        for i in 0..n {
            p.focus_lost = false;
            bring_to_front();
            let r0 = cell.r0();
            let target = cell.target();
            let setup = if r0 { Setup::Kana } else { Setup::Off };
            let mut fresh = serde_json::Value::Null;
            let pre_ok = match cell.gap {
                "sync" => ensure(p, setup, true),
                "close" => {
                    let ok = ensure(p, Setup::Kana, true);
                    let _ = ime_control(0x0006, 0);
                    ok
                }
                "open" => {
                    let ok = ensure(p, Setup::Off, true);
                    let _ = ime_control(0x0006, 1);
                    ok
                }
                _ => {
                    // fresh: awase を落とした間に IME 自身に状態をそろえさせ(awase=false の判定で確認)、awase を起動し直す。
                    km_kill_awase();
                    sleep(1500);
                    bring_to_front();
                    let aligned = ensure(p, setup, false);
                    let started = km_start_awase();
                    sleep(fresh_settle);
                    bring_to_front();
                    sleep(500);
                    fresh = serde_json::json!({"aligned":aligned,"started":started});
                    aligned && started
                }
            };
            sleep(wait_ms);
            let pre_api = ime_control(0x0005, 0).map(|v| v != 0);
            let utc0 = utc_stamp();
            let mut presses = Vec::new();
            if pre_ok {
                for pi in 1..=max_press {
                    let utc = utc_stamp();
                    km_send_key(cell);
                    sleep(500);
                    let api500 = ime_control(0x0005, 0).map(|v| v != 0);
                    let got = p.probe_logged("keymatrix 押下後");
                    let api_after = ime_control(0x0005, 0).map(|v| v != 0);
                    let typed_open = km_open_of(got);
                    presses.push(
                        serde_json::json!({"i":pi,"utc":utc,"api500":api500,"api2000":api_after,
                        "typed":got.label(),"typed_open":typed_open}),
                    );
                    if p.focus_lost || typed_open == Some(target) {
                        break;
                    }
                    sleep(1000);
                }
            }
            p.log.line(&format!(
                "KM {}",
                serde_json::json!({"type":"km_trial","cell":cell.label,"key":cell.key,"kind":cell.kind,
                    "gap":cell.gap,"n":i,"r0":r0,"target":target,"utc":utc0,"pre_ok":pre_ok,
                    "pre_api":pre_api,"presses":presses,"focus_lost":p.focus_lost,"fresh":fresh})
            ));
        }
    }
}

/// `windows` の `ITfThreadMgr::GetGlobalCompartment` で大域の `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` を書く(TSF 直接。結果を文字列で返す)。
fn tsf_global_set_openclose(v: i32) -> String {
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::TextServices::{
        CLSID_TF_ThreadMgr, ITfThreadMgr, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
    };
    // SAFETY: このスレッド(STA)で COM を初期化して TSF の大域 compartment を読み書きするだけ。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
        let tm: ITfThreadMgr = match CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)
        {
            Ok(t) => t,
            Err(e) => return format!("ThreadMgr作成失敗:{e}"),
        };
        let cid = match tm.Activate() {
            Ok(c) => c,
            Err(e) => return format!("Activate失敗:{e}"),
        };
        let out = (|| -> Result<String, String> {
            let gm = tm
                .GetGlobalCompartment()
                .map_err(|e| format!("GetGlobalCompartment失敗:{e}"))?;
            let c = gm
                .GetCompartment(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)
                .map_err(|e| format!("GetCompartment失敗:{e}"))?;
            let rd = |c: &windows::Win32::UI::TextServices::ITfCompartment| {
                c.GetValue()
                    .ok()
                    .and_then(|x| i32::try_from(&x).ok())
            };
            let before = rd(&c);
            c.SetValue(cid, &VARIANT::from(v))
                .map_err(|e| format!("SetValue失敗:{e} before={before:?}"))?;
            let after = rd(&c);
            Ok(format!("ok before={before:?} after={after:?}"))
        })();
        let _ = tm.Deactivate();
        out.unwrap_or_else(|e| e)
    }
}

fn spawn_chrome(chrome: &str, profile: &std::path::Path, port: u16) -> std::process::Child {
    std::process::Command::new(chrome)
        .args([
            &format!("--user-data-dir={}", profile.display()),
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-sync",
            &format!("--app=http://127.0.0.1:{port}/"),
        ])
        .spawn()
        .expect("chrome を起動できません")
}

fn settle_ms_or(args: &[String]) -> u64 {
    args.iter()
        .find_map(|a| a.strip_prefix("--settle=").and_then(|v| v.parse().ok()))
        .unwrap_or(500)
}

fn or_api() -> Option<bool> {
    ime_control(0x0005, 0).map(|v| v != 0)
}

/// `--offrca` の動作。戻り値は注入したキーの説明。
fn or_do(action: &str) -> String {
    let tap = |vk: u32, hold: u64| {
        send_key(vk, true);
        sleep(hold);
        send_key(vk, false);
    };
    match action {
        "1a" => tap(0x1A, 60),
        "1a_dbl" => {
            tap(0x1A, 60);
            sleep(150);
            tap(0x1A, 60);
        }
        "1a_dbl0" => {
            tap(0x1A, 10);
            tap(0x1A, 10);
        }
        "1a_dbl50" => {
            tap(0x1A, 30);
            sleep(50);
            tap(0x1A, 30);
        }
        "1a_dbl400" => {
            tap(0x1A, 60);
            sleep(400);
            tap(0x1A, 60);
        }
        "1a_imc0" => {
            tap(0x1A, 60);
            sleep(100);
            let r = ime_control(0x0006, 0);
            return format!("1a_imc0 ret={r:?}");
        }
        "19" => tap(0x19, 60),
        "f3" => tap(0xF3, 60),
        "f4" => tap(0xF4, 60),
        "1d" => tap(0x1D, 60),
        "f0" => tap(0xF0, 60),
        "ctrl1d" => send_ctrl_muhenkan(),
        "16" => tap(0x16, 40),
        "ctrl1c" => {
            send_key(0xA2, true);
            sleep(40);
            tap(0x1C, 60);
            sleep(30);
            send_key(0xA2, false);
        }
        "imc0" => {
            let r = ime_control(0x0006, 0);
            return format!("imc0 ret={r:?}");
        }
        "tsf0" => return format!("tsf0 {}", tsf_global_set_openclose(0)),
        other => return format!("未知のaction:{other}"),
    }
    format!("key {action}")
}

fn or_poll(total_ms: u64, stop_when_closed: bool) -> (Option<u64>, Vec<(u64, Option<bool>)>) {
    let t0 = Instant::now();
    let mut series: Vec<(u64, Option<bool>)> = Vec::new();
    let mut closed_ms = None;
    let mut last: Option<Option<bool>> = None;
    while t0.elapsed() < Duration::from_millis(total_ms) {
        let v = or_api();
        let t = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
        if last != Some(v) {
            series.push((t, v));
            last = Some(v);
        }
        if v == Some(false) && closed_ms.is_none() {
            closed_ms = Some(t);
            if stop_when_closed {
                break;
            }
        }
        sleep(20);
    }
    (closed_ms, series)
}

#[allow(clippy::too_many_arguments)]
fn run_offrca(
    p: &mut Probe,
    spec: &str,
    args: &[String],
    awase: bool,
    chrome: &str,
    profile: &std::path::Path,
    port: u16,
    child: &mut std::process::Child,
) {
    let arg_u64 = |key: &str, default: u64| -> u64 {
        args.iter()
            .find_map(|a| a.strip_prefix(key).and_then(|v| v.parse().ok()))
            .unwrap_or(default)
    };
    let n = arg_u64("--or-n=", 10);
    let poll_ms = arg_u64("--or-poll=", 4000);
    let ladder = args.iter().any(|a| a == "--or-ladder");
    let relaunch = args.iter().any(|a| a == "--or-relaunch");
    // `--or-then=<action>`: OFF の動作と待ちの後に ON 側の動作(`16`/`ctrl1c`)を行い、k,a の結果(かな=ON が効いて入力できる)を見る。
    let then = args
        .iter()
        .find_map(|a| a.strip_prefix("--or-then="))
        .map(str::to_string);
    let msime = args.iter().any(|a| a == "--msime");
    p.log.line(&format!(
        "OFFRCA_CONFIG {}",
        serde_json::json!({"ime":if msime {"msime"} else {"gji"},"awase":awase,"cells":spec,
            "n":n,"poll_ms":poll_ms,"ladder":ladder,"relaunch":relaunch})
    ));
    for cell in spec.split(',').filter(|s| !s.is_empty()) {
        let (action, prep) = cell.split_once(':').unwrap_or((cell, "typed"));
        for i in 0..n {
            if relaunch && i > 0 {
                let _ = child.kill();
                let _ = child.wait();
                sleep(1500);
                let before = p
                    .shared
                    .lock()
                    .unwrap()
                    .events
                    .iter()
                    .filter(|e| e.kind == "ready")
                    .count();
                *child = spawn_chrome(chrome, profile, port);
                let start = Instant::now();
                while p
                    .shared
                    .lock()
                    .unwrap()
                    .events
                    .iter()
                    .filter(|e| e.kind == "ready")
                    .count()
                    <= before
                {
                    if start.elapsed() > Duration::from_secs(40) {
                        p.log.line("OFFRCA_ABORT 再起動した Chrome のページが読み込まれない");
                        return;
                    }
                    sleep(100);
                }
                sleep(1500);
            }
            p.focus_lost = false;
            bring_to_front();
            // 準備: IME を開く。
            let prep_tag = prep.split('~').next().unwrap_or(prep);
            let events_before = p.shared.lock().unwrap().events.len();
            let prep_ok = match prep_tag {
                "typed_nc" | "typed_enter" | "typed_esc" => {
                    // composition を残したまま(clear しない)。enter/esc はその後に確定/取消してから page を空にする。
                    let ok = ensure(p, Setup::Kana, awase);
                    p.no_clear = true;
                    let _ = p.probe();
                    p.no_clear = false;
                    match prep_tag {
                        "typed_enter" => {
                            p.press(0x0D, false, 40);
                            sleep(400);
                        }
                        "typed_esc" => {
                            p.press(0x1B, false, 40);
                            sleep(400);
                        }
                        _ => {}
                    }
                    ok
                }
                t if t.starts_with("typed_w") => {
                    let ok = ensure(p, Setup::Kana, awase);
                    let w: u64 = t["typed_w".len()..].parse().unwrap_or(2000);
                    sleep(w);
                    ok
                }
                "notype" => {
                    p.press(0x16, false, 40);
                    sleep(1000);
                    or_api() == Some(true)
                }
                "imc" => {
                    let _ = ime_control(0x0006, 1);
                    sleep(800);
                    or_api() == Some(true)
                }
                _ => ensure(p, Setup::Kana, awase),
            };
            sleep(500);
            let api_pre = or_api();
            let conv_pre = ime_control(0x0001, 0);
            let ev_idx = p.shared.lock().unwrap().events.len();
            let utc = utc_stamp();
            let t_act = Instant::now();
            let desc = or_do(action);
            let (closed_ms, series) = or_poll(poll_ms, false);
            let _ = (t_act, events_before);
            let api_end = or_api();
            let conv_end = ime_control(0x0001, 0);
            // 動作から probe 前までにページが見たイベント(IME がキーを処理したか・composition が終わったか)。
            let page_events: Vec<String> = p.shared.lock().unwrap().events[ev_idx..]
                .iter()
                .take(14)
                .map(|e| format!("{}:{}:{}:{}", e.kind, e.key, e.kc, e.data))
                .collect();
            // 動作直後のページの文字(残った composition が確定されたか)を、打鍵の前に読む。
            let text_post = p.command("snap", "snap").map(|e| e.value);
            let _ = p.command("clear", "cleared");
            sleep(300);
            let got = p.probe_logged("offrca 後");
            let api_after_probe = or_api();
            // 2回目の打鍵: 1回目で古い composition の確定(かの再出現)が混ざっても、ここは現在のモードだけを表す。
            let (got2, text2, _) = p.probe();
            p.log.line(&format!("PROBE offrca 後2回目: {} text={text2:?}", got2.label()));
            let typed_open = km_open_of(got);
            // OFF の次に ON を押して、かなが入力できるか(半角英数に取り残されないか)。
            let then_res = if let Some(t) = &then {
                let _ = p.command("clear", "cleared");
                let d = or_do(t);
                sleep(settle_ms_or(args));
                let (c3, text3, _) = p.probe();
                p.log.line(&format!("PROBE offrca then={t}: {} text={text3:?}", c3.label()));
                serde_json::json!({"then":t,"desc":d,"class":c3.label(),"open":km_open_of(c3),"text":text3,"api":or_api()})
            } else {
                serde_json::Value::Null
            };
            let mut ladder_res = Vec::new();
            if ladder && closed_ms.is_none() && api_end == Some(true) {
                for step in ["1a", "imc0", "f3", "19", "tsf0"] {
                    let d = or_do(step);
                    let (c2, _) = or_poll(1500, true);
                    ladder_res.push(serde_json::json!({"step":step,"desc":d,"closed_ms":c2}));
                    if c2.is_some() {
                        break;
                    }
                }
            }
            let ser: Vec<serde_json::Value> = series
                .iter()
                .map(|(t, v)| serde_json::json!([t, v]))
                .collect();
            p.log.line(&format!(
                "OFFRCA {}",
                serde_json::json!({"type":"or_trial","cell":cell,"action":action,"prep":prep,"n":i,
                    "utc":utc,"prep_ok":prep_ok,"api_pre":api_pre,"desc":desc,"closed_ms":closed_ms,
                    "series":ser,"api_end":api_end,"typed":got.label(),"typed_open":typed_open,
                    "api_after_probe":api_after_probe,"text_post":text_post,"typed2":got2.label(),"typed2_open":km_open_of(got2),"typed2_text":text2,"conv_pre":conv_pre,"conv_end":conv_end,"page_events":page_events,"then":then_res,"ladder":ladder_res,"focus_lost":p.focus_lost,
                    "awase":awase})
            ));
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let repeat: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("--repeat=").and_then(|v| v.parse().ok()))
        .unwrap_or(3);
    let awase = !args.iter().any(|a| a == "--no-awase");
    // モードキーを押してから `k`,`a` を打つまでの待ち(ms)。EXPLICIT_IME_SUPPRESS_MS(1500)の内外を比べる用。
    let settle_ms: u64 = args
        .iter()
        .find_map(|a| a.strip_prefix("--settle=").and_then(|v| v.parse().ok()))
        .unwrap_or(500);
    let chrome_arg = args
        .iter()
        .find_map(|a| a.strip_prefix("--chrome=").map(str::to_string));
    let log_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--log=").map(str::to_string))
        .unwrap_or_else(|| "chrome_probe.log".to_string());
    let _ = std::fs::remove_file(&log_path);
    let mut log = Log(std::fs::File::create(&log_path).expect("log"));

    let Some(chrome) = find_chrome(chrome_arg) else {
        log.line("Chrome が見つかりません(--chrome=<path> で指定)");
        return;
    };
    let shared = Arc::new(Mutex::new(Shared {
        events: Vec::new(),
        cmd: None,
    }));
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    {
        let sh = Arc::clone(&shared);
        std::thread::spawn(move || {
            for s in listener.incoming().flatten() {
                let sh2 = Arc::clone(&sh);
                std::thread::spawn(move || handle(s, &sh2));
            }
        });
    }
    let profile = std::env::temp_dir().join(format!("chrome_probe_profile_{port}"));
    log.line(&format!(
        "chrome={chrome} port={port} awase={awase} repeat={repeat} settle={settle_ms}ms"
    ));
    if args.iter().any(|a| a == "--msime") {
        activate_msime_profile(&mut log);
    }
    let mut child = spawn_chrome(&chrome, &profile, port);

    // ページの ready を待つ。
    let start = Instant::now();
    while !shared
        .lock()
        .unwrap()
        .events
        .iter()
        .any(|e| e.kind == "ready")
    {
        if start.elapsed() > Duration::from_secs(40) {
            log.line("ページが読み込まれませんでした(timeout)");
            let _ = child.kill();
            return;
        }
        sleep(100);
    }
    sleep(1500);
    let fronted = bring_to_front();
    log.line(&format!("前面化: {fronted}"));
    sleep(800);

    let shift_tail_ms: u64 = args
        .iter()
        .find_map(|a| a.strip_prefix("--shift-tail=").and_then(|v| v.parse().ok()))
        .unwrap_or(40);
    let mut p = Probe {
        shift_tail_ms,
        shared,
        log,
        focus_lost: false,
        no_clear: false,
    };
    // `--storm=N`: 親指キー(無変換, NICOLAの既定の親指シフト)を使った通常タイピングをN回行う(BUG-149 レビューB1の確認用)。
    // 文字の判定はせず、awaseログの強制conv読み取りの件数を見る。
    if let Some(n) = args.iter().find_map(|a| {
        a.strip_prefix("--storm=")
            .and_then(|v| v.parse::<usize>().ok())
    }) {
        bring_to_front();
        let _ = ensure(&mut p, Setup::Kana, awase);
        p.log.line(&format!("STORM start n={n}"));
        for i in 0..n {
            // 親指(無変換)を押したまま文字キー(J)を押して離し、親指を離す。その後 450ms 止まる(文節の切れ目相当)。
            send_key(0x1D, true);
            sleep(40);
            send_key(0x4A, true);
            sleep(40);
            send_key(0x4A, false);
            sleep(30);
            send_key(0x1D, false);
            p.log.line(&format!("STORM {i} thumb+J"));
            sleep(450);
        }
        let _ = p.command("clear", "cleared");
        p.log.line("STORM end");
        p.log.line("=== 全ケース完了 ===");
        let _ = child.kill();
        return;
    }
    // `--initial`(ADR-212 P2 調査): IME を一切操作せず、Chrome 起動直後に k,a を打って実 IME の初期状態を見る。
    // `ka`(Plain)=閉で始まった / `か`(RomajiKana)=開で始まった / NICOLA 文字=awase が開いた(書き込み)。
    // awase を先に起動してから Chrome を起動する構成(新しいスレッド)で使う。`--no-awase` の対照は IME の素の初期状態。
    if args.iter().any(|a| a == "--initial") {
        p.focus_lost = false;
        bring_to_front();
        sleep(settle_ms);
        let got = p.probe_logged("起動直後(IME操作なし)");
        p.log.line(&format!("INITIAL class={}", got.label()));
        if got == Class::Plain {
            p.log.line("RESULT PASS");
        } else {
            p.log
                .line(&format!("RESULT FAIL: 期待=ka 実際={}", got.label()));
        }
        p.log.line("=== 全ケース完了 ===");
        let _ = child.kill();
        return;
    }
    // `--keymatrix=<cells>`(ADR-208 L3b): 「ずれの作り方 × 明示キー」行列。形式は `run_keymatrix` を参照。
    if let Some(spec) = args.iter().find_map(|a| a.strip_prefix("--keymatrix=")) {
        bring_to_front();
        run_keymatrix(&mut p, spec, &args);
        p.log.line("=== 全ケース完了 ===");
        let _ = child.kill();
        return;
    }
    // `--offrca=<action>:<prep>,...`: 模式は run_offrca の doc を参照。
    if let Some(spec) = args.iter().find_map(|a| a.strip_prefix("--offrca=")) {
        bring_to_front();
        run_offrca(
            &mut p, spec, &args, awase, &chrome, &profile, port, &mut child,
        );
        p.log.line("=== 全ケース完了 ===");
        let _ = child.kill();
        return;
    }
    // `--settle-explicit=<key>` + `--settle-at=<ms>`(ADR-213 P2d-2 の実測スパイク): 直接入力(IME OFF)にそろえた後、窓を一度フォーカス外し→前面化し、
    // 前面化が返った t=<ms> 後に明示操作(`<key>` = `ctrl+1c`(Ctrl+変換) / `1d`(無変換の単独タップ) / `f3`(物理の半角/全角))を1回押して、
    // その +settle ms 後に k,a を打つ。結果は `Process(229)` と出た文字で測る(かな=受け付けられた、`ka`=無視された)。
    // awase の focus settle は focus_settle_ms 経過で明ける。settle 中の明示操作 SetOpen は P2d-2 以降 awase が落とさない(ADR-213 決定5)。
    if let Some(spec) = args
        .iter()
        .find_map(|a| a.strip_prefix("--settle-explicit="))
    {
        let at_ms: u64 = args
            .iter()
            .find_map(|a| a.strip_prefix("--settle-at=").and_then(|v| v.parse().ok()))
            .unwrap_or(150);
        let (ctrl, hex) = match spec.strip_prefix("ctrl+") {
            Some(h) => (true, h),
            None => (false, spec),
        };
        let vk = u32::from_str_radix(hex.trim_start_matches("0x"), 16).unwrap_or(0);
        let (mut ok, mut bad, mut invalid) = (0usize, 0usize, 0usize);
        for r in 1..=repeat {
            p.log.line(&format!(
                "[CASE 1/1 run {r}/{repeat}] settle直後の明示操作 key={spec} at={at_ms}ms"
            ));
            p.focus_lost = false;
            bring_to_front();
            if !ensure(&mut p, Setup::Off, awase) {
                p.log
                    .line("RESULT INVALID: 前提状態(直接入力)にできなかった");
                invalid += 1;
                continue;
            }
            sleep(1000);
            let away = focus_away_to_helper() || focus_away();
            sleep(600);
            let back = bring_to_front();
            let t0 = Instant::now();
            p.log
                .line(&format!("SETTLE REFOCUS away={away} back={back} (t=0)"));
            if !away || !back {
                p.log.line("RESULT INVALID: フォーカスの外し/戻しに失敗");
                invalid += 1;
                continue;
            }
            sleep(at_ms);
            if ctrl {
                send_key(0xA2, true);
                sleep(40);
            }
            send_key(vk, true);
            sleep(60);
            send_key(vk, false);
            if ctrl {
                sleep(40);
                send_key(0xA2, false);
            }
            p.log.line(&format!(
                "SETTLE KEY {spec} sent at t={}ms (目標 {at_ms}ms)",
                t0.elapsed().as_millis()
            ));
            sleep(settle_ms);
            let got = p.probe_logged("settle直後の操作後");
            let want = if awase {
                got == Class::Nicola
            } else {
                got == Class::RomajiKana
            };
            if p.focus_lost {
                p.log.line("RESULT INVALID: ページのフォーカスが外れた");
                invalid += 1;
            } else if want {
                p.log.line("RESULT PASS: 受け付けられた(かな)");
                ok += 1;
            } else {
                p.log
                    .line(&format!("RESULT FAIL: 無視/未追随 実際={}", got.label()));
                bad += 1;
            }
        }
        p.log.line(&format!(
            "SUMMARY PASS={ok} RECOVER=0 FAIL={bad} INVALID={invalid}"
        ));
        p.log.line("=== 全ケース完了 ===");
        let _ = child.kill();
        return;
    }
    // `--close-ime=N`(BUG-172 の実 Chrome 確認): IME を ON にそろえた後、実 IME を WM_IME_CONTROL で直接閉じ、
    // 3 秒待ってからかな単打(k,a)を打って結果を見る。閉じたまま `ka`/ローマ字が出れば BUG-172 が実 Chrome でも起きる。
    // `open=` は awase と同じ経路(IMC_GETOPENSTATUS)の読み取り値(TsfNative では信頼できない可能性がある)。
    if let Some(n) = args.iter().find_map(|a| {
        a.strip_prefix("--close-ime=")
            .and_then(|v| v.parse::<usize>().ok())
    }) {
        let (mut ok, mut bad, mut invalid) = (0usize, 0usize, 0usize);
        for i in 0..n {
            p.log.line(&format!("[CLOSE {}/{n}]", i + 1));
            p.focus_lost = false;
            bring_to_front();
            if !ensure(&mut p, Setup::Kana, awase) {
                p.log.line("RESULT INVALID: 前提状態(かな)にできなかった");
                invalid += 1;
                continue;
            }
            let before = ime_control(0x0005, 0);
            let ctrl_muhenkan = args.iter().any(|a| a == "--ctrl-muhenkan-off");
            let set_ret = if ctrl_muhenkan {
                send_ctrl_muhenkan();
                None
            } else {
                ime_control(0x0006, 0)
            };
            sleep(50);
            let after = ime_control(0x0005, 0);
            p.log.line(&format!(
                "CLOSE_IME method={} open_before={before:?} set_ret={set_ret:?} open_after={after:?}",
                if ctrl_muhenkan { "ctrl_muhenkan" } else { "direct_close" }
            ));
            let closed_at = Instant::now();
            if args.iter().any(|a| a == "--refocus") {
                let away = focus_away();
                sleep(300);
                let back = bring_to_front();
                p.log.line(&format!("REFOCUS away={away} back={back}"));
            }
            for checkpoint_ms in [500u64, 2000] {
                let remaining =
                    Duration::from_millis(checkpoint_ms).saturating_sub(closed_at.elapsed());
                std::thread::sleep(remaining);
                let open = ime_control(0x0005, 0);
                p.log.line(&format!(
                    "CLOSE_CHECK checkpoint_ms={checkpoint_ms} open={open:?}"
                ));
            }
            let open_late = ime_control(0x0005, 0);
            let got = p.probe_logged("閉じて2秒後");
            p.log
                .line(&format!("CLOSE_IME open_at_probe={open_late:?}"));
            if p.focus_lost {
                p.log.line("RESULT INVALID: ページのフォーカスが外れた");
                invalid += 1;
            } else if ctrl_muhenkan {
                // 物理 Ctrl+無変換 で OFF にする変種: 期待は「英字のまま(ka)」。`gap` は OFF 操作の直後も実 IME が開いたまま
                // だった(awase の OFF 操作と実 IME がずれた)試行。checker は gap と実打鍵で ずれ有無/回復を分ける。
                let gap = matches!(after, Some(v) if v != 0);
                if got == Class::Plain {
                    p.log
                        .line(&format!("RESULT PASS: OFF が効き英字(ka) gap={gap}"));
                    ok += 1;
                } else {
                    p.log.line(&format!(
                        "RESULT FAIL: OFF が効かず 実際={} gap={gap}",
                        got.label()
                    ));
                    bad += 1;
                }
            } else if got == Class::Nicola {
                p.log
                    .line("RESULT PASS: IME が開き直りNICOLA文字が出た(回復)");
                ok += 1;
            } else {
                p.log.line(&format!(
                    "RESULT FAIL: 閉じたまま/未回復 実際={}",
                    got.label()
                ));
                bad += 1;
            }
        }
        p.log.line(&format!(
            "SUMMARY PASS={ok} RECOVER=0 FAIL={bad} INVALID={invalid}"
        ));
        p.log.line("=== 全ケース完了 ===");
        let _ = child.kill();
        return;
    }
    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut invalid = 0usize;
    let mut recover = 0usize;
    // `--key=<hex>`(追随できる条件の調査): 閉状態(直接入力)からそのキーを1回押し、Engine が追随して NICOLA になるかを見る。
    // 追随=PASS、IME は開いたが Engine OFF(`か`)や開かない(`ka`)=FAIL。呼び出し側は expect=observe で結果だけ表に出す。
    let key_cases: Option<&'static [Case]> = args
        .iter()
        .find_map(|a| a.strip_prefix("--key="))
        .and_then(|h| u32::from_str_radix(h.trim_start_matches("0x"), 16).ok())
        .map(|vk| {
            let name: &'static str =
                Box::leak(format!("直接入力→0x{vk:02X}=かなON").into_boxed_str());
            let cases: &'static [Case] = Box::leak(Box::new([Case {
                name,
                setup: Setup::Off,
                vk,
                shift: false,
                expect_kana: true,
            }]));
            cases
        });
    let cases: &[Case] = if let Some(k) = key_cases {
        k
    } else if args.iter().any(|a| a == "--f13") {
        &F13_CASES
    } else if args.iter().any(|a| a == "--henkan-open") {
        &HENKAN_OPEN_CASES
    } else {
        &CASES
    };
    for r in 1..=repeat {
        for (i, c) in cases.iter().enumerate() {
            p.log.line(&format!(
                "[CASE {}/{} run {r}/{repeat}] {}",
                i + 1,
                cases.len(),
                c.name
            ));
            p.focus_lost = false;
            if !bring_to_front() {
                p.log.line("前面化に失敗");
            }
            let ready = ensure(&mut p, c.setup, awase);
            if !ready {
                p.log.line("RESULT INVALID: 前提状態にできなかった");
                invalid += 1;
                continue;
            }
            p.press(c.vk, c.shift, 120);
            sleep(settle_ms);
            let got = p.probe_logged("action後");
            let want_ok = if c.expect_kana {
                if awase {
                    got == Class::Nicola
                } else {
                    got == Class::RomajiKana
                }
            } else {
                got == Class::Plain
            };
            if p.focus_lost {
                p.log.line("RESULT INVALID: ページのフォーカスが外れた");
                invalid += 1;
            } else if want_ok {
                p.log.line("RESULT PASS");
                pass += 1;
            } else if {
                // 1回目が失敗でも、2回目の試行で期待どおりになるか(=読み取りが遅れて追随したか)を記録する。
                sleep(400);
                let again = p.probe_logged("action後2回目");
                if c.expect_kana {
                    if awase {
                        again == Class::Nicola
                    } else {
                        again == Class::RomajiKana
                    }
                } else {
                    again == Class::Plain
                }
            } {
                p.log.line(&format!(
                    "RESULT RECOVER: 1回目は{}、2回目で追随",
                    got.label()
                ));
                recover += 1;
            } else {
                p.log.line(&format!(
                    "RESULT FAIL: 期待={} 実際={}",
                    if c.expect_kana { "かな" } else { "ka" },
                    got.label()
                ));
                fail += 1;
            }
        }
    }
    p.log.line(&format!(
        "SUMMARY PASS={pass} RECOVER={recover} FAIL={fail} INVALID={invalid}"
    ));
    p.log.line("=== 全ケース完了 ===");
    let _ = child.kill();
}
