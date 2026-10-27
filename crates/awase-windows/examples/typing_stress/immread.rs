//! `--mode=immread`(BUG-219 の調査、ci/imm-read-matrix 専用。develop には入れない): 入力先の窓ごとに、
//! IMM で読む IME の開閉が実状態と一致するかを測る。awase は起動しない(`--no-awase`)。読み取りだけ。
//!
//! 1 試行 = 実 IME を「開」または「閉」にそろえる → 1.2 秒間、100ms ごとに 2 通りで読む → 最後に k,a を打って実状態を確かめる。
//!  - `prod`: awase の prefetch と同じ関数 `read_ime_state_full_with_timeout(300ms)`(クラス名の門前払い=TSF ネイティブ窓の
//!    スキップを含む。本番で awase が見る値)。
//!  - `raw`: 門前払いを外した素の読み。前面スレッドの hwndFocus → `ImmGetDefaultIMEWnd` → `WM_IME_CONTROL`
//!    (`IMC_GETOPENSTATUS`=5 / `IMC_GETCONVERSIONMODE`=1)。「門前払いを外せば読めるか」を見る。
//!
//! 実状態(truth)は最後の打鍵で決める: 本文に「か」=開、`ka`=閉、どちらでもなければ unknown。
//! 記録は `immread` 行(集計は check_immread.py)。

use std::io::Write as _;
use std::process::{Child, Command};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, SendMessageTimeoutW,
    GUITHREADINFO, SEND_MESSAGE_TIMEOUT_FLAGS,
};

use crate::{
    arg_value, clear_text, focus_ok, press, read_text_maybe_settled, rec, refocus, sleep_ms,
    turn_ime_on, utc_hms,
};

const VK_IME_OFF: u32 = 0x1A;
/// (VK, scan)。スキャンコードは JIS 配列の物理位置。
const KEY_K: (u32, u16) = (0x4B, 0x25);
const KEY_A: (u32, u16) = (0x41, 0x1E);
const KEY_RETURN: (u32, u16) = (0x0D, 0x1C);

/// 素の読み: 前面スレッドの hwndFocus の既定 IME 窓へ `WM_IME_CONTROL`。(open, conv, 読んだ窓のクラスの有無)。
fn raw_read() -> (Option<bool>, Option<u32>) {
    const WM_IME_CONTROL: u32 = 0x0283;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    // SAFETY: 前面窓の既定 IME 窓へ短いタイムアウト付きで同期送信するだけ(読み取り専用)。
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return (None, None);
        }
        let tid = GetWindowThreadProcessId(fg, None);
        let mut info = GUITHREADINFO {
            cbSize: u32::try_from(size_of::<GUITHREADINFO>()).unwrap_or(0),
            ..Default::default()
        };
        let target: HWND =
            if GetGUIThreadInfo(tid, &raw mut info).is_ok() && !info.hwndFocus.0.is_null() {
                info.hwndFocus
            } else {
                fg
            };
        let ime_wnd = ImmGetDefaultIMEWnd(target);
        if ime_wnd.0.is_null() {
            return (None, None);
        }
        let send = |cmd: usize| -> Option<isize> {
            let mut res = 0usize;
            let r = SendMessageTimeoutW(
                ime_wnd,
                WM_IME_CONTROL,
                WPARAM(cmd),
                LPARAM(0),
                SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG),
                300,
                Some(&raw mut res),
            );
            (r.0 != 0).then_some(res as isize)
        };
        (
            send(0x0005).map(|v| v != 0),
            send(0x0001).and_then(|v| u32::try_from(v).ok()),
        )
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0))
}

fn tri(v: Option<bool>) -> char {
    match v {
        Some(true) => '1',
        Some(false) => '0',
        None => 'n',
    }
}

/// 別プロセスの読み手(`--immread-reader=<path>`)。窓を持たず、100ms ごとに prod と raw で読んで `<ms> <prod> <raw>` を追記する。
/// 読み取り先は「前面の窓」なので、入力先と別プロセスから読む条件(awase と同じ)になる。親が kill する。
pub(crate) fn reader_main(path: &str) {
    let every: u64 = arg_value("--immread-reader-ms=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    loop {
        // SAFETY: awase の prefetch と同じ読み取り専用の IMM 呼び出し。
        let s = unsafe {
            awase_windows::ime::read_ime_state_full_with_timeout(Duration::from_millis(300))
        };
        let (raw_open, _) = raw_read();
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{} {} {}", now_ms(), tri(s.ime_on), tri(raw_open));
        }
        sleep_ms(every);
    }
}

fn spawn_reader(path: &str, every_ms: u64) -> Option<Child> {
    let exe = std::env::current_exe().ok()?;
    let _ = std::fs::remove_file(path);
    Command::new(exe)
        .arg(format!("--immread-reader={path}"))
        .arg(format!("--immread-reader-ms={every_ms}"))
        .spawn()
        .ok()
}

/// 別プロセスの読み手の記録から、`[from, to]`(ms)の読みを数える。(prod, raw)。
fn external_tally(path: &str, from: u64, to: u64) -> (Tally, Tally) {
    let (mut prod, mut raw) = (Tally::default(), Tally::default());
    let text = std::fs::read_to_string(path).unwrap_or_default();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(ms), Some(p), Some(r)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let Ok(ms) = ms.parse::<u64>() else {
            continue;
        };
        if ms < from || ms > to {
            continue;
        }
        let conv = |c: &str| match c {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        };
        prod.add(conv(p));
        raw.add(conv(r));
    }
    (prod, raw)
}

#[derive(Default)]
struct Tally {
    on: u32,
    off: u32,
    none: u32,
}

impl Tally {
    fn add(&mut self, v: Option<bool>) {
        match v {
            Some(true) => self.on += 1,
            Some(false) => self.off += 1,
            None => self.none += 1,
        }
    }
    fn json(&self) -> serde_json::Value {
        json!({"on":self.on,"off":self.off,"none":self.none})
    }
}

pub(crate) fn immread_scenario(child: HWND) {
    let trials: usize = arg_value("--trials=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    let reads: usize = arg_value("--immread-reads=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);
    let settle = crate::has_flag("--settle-read");
    rec(&json!({"type":"immread_config","trials":trials,"reads":reads}));
    // 別プロセスの読み手(awase と同じ「入力先と別プロセスから読む」条件)。--immread-external のときだけ。
    let reader_path = "immread_reader.txt";
    let mut reader = if crate::has_flag("--immread-external") {
        spawn_reader(reader_path, 100)
    } else {
        None
    };
    rec(&json!({"type":"immread_reader","external":reader.is_some()}));
    for n in 0..trials {
        for kind in ["open", "closed"] {
            if !focus_ok() {
                refocus();
            }
            if !focus_ok() {
                rec(
                    &json!({"type":"abort","reason":format!("immread 試行前にフォーカスが外れた n={n} kind={kind}")}),
                );
                return;
            }
            if kind == "open" {
                turn_ime_on(0);
            } else {
                press(VK_IME_OFF, 0x70, 50);
                sleep_ms(1500);
            }
            clear_text(child);
            sleep_ms(300);
            let t0 = utc_hms();
            let read_from = now_ms();
            let (mut prod, mut raw) = (Tally::default(), Tally::default());
            let mut prod_convs: Vec<String> = Vec::new();
            let mut raw_convs: Vec<String> = Vec::new();
            let mut class = String::new();
            let mut tsf_native = false;
            for _ in 0..reads {
                // SAFETY: awase の prefetch と同じ読み取り専用の IMM 呼び出し。
                let s = unsafe {
                    awase_windows::ime::read_ime_state_full_with_timeout(Duration::from_millis(300))
                };
                prod.add(s.ime_on);
                if let Some(c) = s.conversion_mode {
                    let c = format!("{c:#x}");
                    if !prod_convs.contains(&c) {
                        prod_convs.push(c);
                    }
                }
                if let Some(c) = s.focused_class.as_deref() {
                    class = c.to_owned();
                }
                tsf_native |= s.is_tsf_native;
                let (o, c) = raw_read();
                raw.add(o);
                if let Some(c) = c {
                    let c = format!("{c:#x}");
                    if !raw_convs.contains(&c) {
                        raw_convs.push(c);
                    }
                }
                sleep_ms(100);
            }
            let read_to = now_ms();
            // 実状態: k,a を打って Enter で確定し、本文で決める(か=開、ka=閉)。未確定のままだと Edit 等では本文に出ない。
            press(KEY_K.0, KEY_K.1, 30);
            sleep_ms(60);
            press(KEY_A.0, KEY_A.1, 30);
            sleep_ms(700);
            press(KEY_RETURN.0, KEY_RETURN.1, 50);
            sleep_ms(700);
            let text = read_text_maybe_settled(child, settle);
            let truth = if text.contains('か') {
                "open"
            } else if text.to_ascii_lowercase().contains("ka") {
                "closed"
            } else {
                "unknown"
            };
            let (ext_prod, ext_raw) = external_tally(reader_path, read_from, read_to);
            rec(
                &json!({"type":"immread","n":n,"kind":kind,"utc":t0,"truth":truth,"text":text,
                "class":class,"tsf_native":tsf_native,
                "prod":prod.json(),"prod_convs":prod_convs,
                "raw":raw.json(),"raw_convs":raw_convs,
                "ext_prod":ext_prod.json(),"ext_raw":ext_raw.json()}),
            );
        }
    }
    if let Some(mut c) = reader.take() {
        let _ = c.kill();
    }
}

/// 別プロセスの読み手の記録から、`[from, to]`(ms)の読みを `dt:prod:raw` の列(t0 からの相対 ms)にする。
fn series(path: &str, t0: u64, from: u64, to: u64) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut out = String::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(ms), Some(p), Some(r)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let Ok(ms) = ms.parse::<u64>() else {
            continue;
        };
        if ms < from || ms > to {
            continue;
        }
        let dt = i64::try_from(ms).unwrap_or(0) - i64::try_from(t0).unwrap_or(0);
        out.push_str(&format!("{dt}:{p}:{r} "));
    }
    out
}

/// `--mode=immtime`(BUG-219 の調査): 静止時ではなく、状態を変えた直後と打鍵の最中の読みを 20ms 刻みで測る。
///  - `key`: 開または閉にそろえ、1.5 秒おいてから IME キーを 1 つ押し、押す 200ms 前〜1.5 秒後を読む。最後に k,a,Enter で実状態を決める。
///  - `typing`: IME を開にして、ローマ字を `--typing-gap` ms 間隔で連続して打ち、その間を読む(IME はずっと開のはず)。
pub(crate) fn immtime_scenario(child: HWND) {
    let trials: usize = arg_value("--trials=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let gap: u64 = arg_value("--typing-gap=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let settle = crate::has_flag("--settle-read");
    let reader_path = "immread_reader.txt";
    let mut reader = spawn_reader(reader_path, 20);
    rec(
        &json!({"type":"immtime_config","trials":trials,"typing_gap":gap,"reader":reader.is_some()}),
    );
    sleep_ms(800);
    // (名前, VK, scan)
    let keys: [(&str, u32, u16); 7] = [
        ("ime_on", 0x16, 0x70),
        ("ime_off", 0x1A, 0x70),
        ("hira_f2", 0xF2, 0x70),
        ("alnum_f0", 0xF0, 0x70),
        ("kanji_19", 0x19, 0x29),
        ("nonconvert_1d", 0x1D, 0x7B),
        ("convert_1c", 0x1C, 0x79),
    ];
    for n in 0..trials {
        for (name, vk, scan) in keys {
            for start in ["open", "closed"] {
                if !focus_ok() {
                    refocus();
                }
                if start == "open" {
                    turn_ime_on(0);
                } else {
                    press(VK_IME_OFF, 0x70, 50);
                    sleep_ms(1500);
                }
                clear_text(child);
                sleep_ms(1500);
                let t0 = now_ms();
                press(vk, scan, 50);
                sleep_ms(1500);
                let t1 = now_ms();
                press(KEY_K.0, KEY_K.1, 30);
                sleep_ms(60);
                press(KEY_A.0, KEY_A.1, 30);
                sleep_ms(700);
                press(KEY_RETURN.0, KEY_RETURN.1, 50);
                sleep_ms(700);
                let text = read_text_maybe_settled(child, settle);
                let truth = if text.contains('か') {
                    "open"
                } else if text.to_ascii_lowercase().contains("ka") {
                    "closed"
                } else {
                    "unknown"
                };
                rec(
                    &json!({"type":"immtime_key","n":n,"key":name,"start":start,"truth_after":truth,"text":text,
                    "series":series(reader_path, t0, t0.saturating_sub(200), t1)}),
                );
            }
        }
        // typing: 開にして連続して打つ。IME はずっと開のはず。
        if !focus_ok() {
            refocus();
        }
        turn_ime_on(0);
        clear_text(child);
        sleep_ms(1500);
        let t0 = now_ms();
        let seq = [
            KEY_K,
            KEY_A,
            (0x53, 0x1F),
            (0x49, 0x17),
            (0x54, 0x14),
            (0x55, 0x16),
        ];
        for i in 0..90 {
            let (vk, sc) = seq[i % seq.len()];
            press(vk, sc, 20);
            sleep_ms(gap);
            if i % 12 == 11 {
                press(KEY_RETURN.0, KEY_RETURN.1, 30);
                sleep_ms(gap);
            }
        }
        let t1 = now_ms();
        sleep_ms(500);
        let text = read_text_maybe_settled(child, settle);
        rec(
            &json!({"type":"immtime_typing","n":n,"gap":gap,"dur_ms":t1.saturating_sub(t0),"text_len":text.chars().count(),
            "text_has_kana":text.chars().any(|c| ('\u{3041}'..='\u{3093}').contains(&c)),
            "series":series(reader_path, t0, t0, t1)}),
        );
    }
    if let Some(mut c) = reader.take() {
        let _ = c.kill();
    }
}
