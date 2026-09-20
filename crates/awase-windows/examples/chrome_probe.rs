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
//! 使い方: `chrome_probe [--repeat=N] [--no-awase] [--chrome=<chrome.exe>] [--log=<path>]`
//!   `--no-awase`: awase を止めた対照実験（かなのとき `か` を期待）。既定は awase 起動中（NICOLA を期待）。
//! 実行中は Windows 機のキーボード・マウスに触らない。

use std::io::{Read, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, FindWindowW, GetForegroundWindow, GetWindowThreadProcessId,
    SetForegroundWindow,
};

/// スパイクと同じ目印。`AWASE_TEST_INJECTION=1` の awase は、この目印の注入を物理キーとして扱う。
const AUTO_MARKER: usize = 0x5350_494B;

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
                .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
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
    let body = String::from_utf8_lossy(&buf[head_end..head_end + content_len.min(buf.len() - head_end)]).into_owned();
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
        0x1D => 0x7B, // 無変換
        0x1C => 0x79, // 変換
        0xF2 => 0x70, // ひらがな
        0x4B => 0x25, // K
        0x41 => 0x1E, // A
        0xA0 => 0x2A, // LShift
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
                dwFlags: if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP },
                time: 0,
                dwExtraInfo: AUTO_MARKER,
            },
        },
    };
    unsafe {
        let _ = SendInput(&[input], size_of::<INPUT>() as i32);
    }
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
    Empty,
    Other,
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Self::Plain => "ka(英数/直接)",
            Self::RomajiKana => "か(かな・Engine素通し)",
            Self::Nicola => "NICOLA文字(Engine ON)",
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
    Class::Other
}

struct Probe {
    shared: Arc<Mutex<Shared>>,
    log: Log,
    focus_lost: bool,
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
            sleep(40);
            send_key(0xA0, false);
        }
        self.log.line(&format!("KEY vk=0x{vk:02X}{} (auto)", if shift { " +Shift" } else { "" }));
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
        let _ = self.command("clear", "cleared");
        sleep(150);
        (classify(&text), text, process)
    }

    fn probe_logged(&mut self, what: &str) -> Class {
        let (c, text, process) = self.probe();
        self.log
            .line(&format!("PROBE {what}: {} text={text:?} Process(229)={process}", c.label()));
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
    let kana_ok = |c: Class| if awase { c == Class::Nicola } else { c == Class::RomajiKana };
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
            p.probe_logged("setup:ひらがな(かな→半角英数)後") == Class::Plain
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

const CASES: [Case; 8] = [
    Case { name: "かな→無変換=IME OFF", setup: Setup::Kana, vk: 0x1D, shift: false, expect_kana: false },
    Case { name: "直接入力→無変換=かなON", setup: Setup::Off, vk: 0x1D, shift: false, expect_kana: true },
    Case { name: "かな→ひらがな=半角英数", setup: Setup::Kana, vk: 0xF2, shift: false, expect_kana: false },
    Case { name: "半角英数→ひらがな=かな", setup: Setup::Alnum, vk: 0xF2, shift: false, expect_kana: true },
    Case { name: "かな→変換=IME OFF", setup: Setup::Kana, vk: 0x1C, shift: false, expect_kana: false },
    Case { name: "直接入力→変換=かなON", setup: Setup::Off, vk: 0x1C, shift: false, expect_kana: true },
    Case { name: "かな→Shift+無変換=半角英数", setup: Setup::Kana, vk: 0x1D, shift: true, expect_kana: false },
    Case { name: "直接入力→Shift+無変換=直接入力のまま", setup: Setup::Off, vk: 0x1D, shift: true, expect_kana: false },
];

fn find_chrome(arg: Option<String>) -> Option<String> {
    if let Some(a) = arg {
        return Some(a);
    }
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_string(),
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".to_string(),
        format!(r"{local}\Google\Chrome\Application\chrome.exe"),
    ]
    .into_iter()
    .find(|p| std::path::Path::new(p).exists())
}

fn bring_to_front() -> bool {
    unsafe {
        let hwnd = FindWindowW(PCWSTR::null(), w!("IMEPROBE")).unwrap_or_default();
        if hwnd.0.is_null() {
            return false;
        }
        let fg = GetForegroundWindow();
        let fg_tid = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
        let my_tid = GetCurrentThreadId();
        let attached = fg_tid != 0 && fg_tid != my_tid && AttachThreadInput(my_tid, fg_tid, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let ok = SetForegroundWindow(hwnd).as_bool();
        if attached {
            let _ = AttachThreadInput(my_tid, fg_tid, false);
        }
        ok || GetForegroundWindow() == hwnd
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let repeat: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("--repeat=").and_then(|v| v.parse().ok()))
        .unwrap_or(3);
    let awase = !args.iter().any(|a| a == "--no-awase");
    let chrome_arg = args.iter().find_map(|a| a.strip_prefix("--chrome=").map(str::to_string));
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
    let shared = Arc::new(Mutex::new(Shared { events: Vec::new(), cmd: None }));
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
    log.line(&format!("chrome={chrome} port={port} awase={awase} repeat={repeat}"));
    let mut child = std::process::Command::new(&chrome)
        .args([
            &format!("--user-data-dir={}", profile.display()),
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-sync",
            &format!("--app=http://127.0.0.1:{port}/"),
        ])
        .spawn()
        .expect("chrome を起動できません");

    // ページの ready を待つ。
    let start = Instant::now();
    while !shared.lock().unwrap().events.iter().any(|e| e.kind == "ready") {
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

    let mut p = Probe { shared, log, focus_lost: false };
    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut invalid = 0usize;
    for r in 1..=repeat {
        for (i, c) in CASES.iter().enumerate() {
            p.log.line(&format!("[CASE {}/{} run {r}/{repeat}] {}", i + 1, CASES.len(), c.name));
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
            sleep(500);
            let got = p.probe_logged("action後");
            let want_ok = if c.expect_kana {
                if awase { got == Class::Nicola } else { got == Class::RomajiKana }
            } else {
                got == Class::Plain
            };
            if p.focus_lost {
                p.log.line("RESULT INVALID: ページのフォーカスが外れた");
                invalid += 1;
            } else if want_ok {
                p.log.line("RESULT PASS");
                pass += 1;
            } else {
                p.log.line(&format!("RESULT FAIL: 期待={} 実際={}", if c.expect_kana { "かな" } else { "ka" }, got.label()));
                fail += 1;
            }
        }
    }
    p.log.line(&format!("SUMMARY PASS={pass} FAIL={fail} INVALID={invalid}"));
    p.log.line("=== 全ケース完了 ===");
    let _ = child.kill();
}
