//! `--sw-mode=`(**検証専用、ci/ ブランチだけ。develop に入れない**): PR #809(TsfGate の撤去)で「実機未確認」とした
//! 窓切替直後の打鍵(レビュー M1)と、Mozc × Windows Terminal のひらがなキー直後(M2、BUG-209)を、通常の試行ループ
//! (期待文字列との比較・check_typing_stress.py の分類)にそのまま載せて測る。
//!
//! - `b-off`: 窓 B(別プロセスの EDIT)へ移って B の IME を OFF にし、入力先 A(IME ON のまま)へ戻って `delay` ms 後に打つ。
//!   期待 = かな(A の IME 状態)。前の窓の belief(OFF)のまま処理されると崩れる。
//! - `b-on`: A の IME を OFF にし、B へ移って B の IME を ON にし、A へ戻って `delay` ms 後に打つ。期待 = 素の英字(A は直接入力)。
//!   前の窓の belief(ON)のまま処理されると、NICOLA 変換したローマ字が A に出る。
//! - `f2`: A で物理ひらがなキー(VK_DBE_HIRAGANA、マーカー付き=物理扱い)を押して `delay` ms 後に打つ(warm のまま)。
//!
//! `delay` は `--sw-delays=0_30_60`(`_` 区切り)を試行ごとに巡回する。基準時刻は「A が前面に戻ったのを観測した時刻」
//! (`f2` はキーを離した時刻)。試行ごとに `{"type":"sw",...}` を記録する(`kind`・`n` で trial と突き合わせる)。

#[allow(clippy::wildcard_imports)]
use super::*;

#[link(name = "user32")]
extern "system" {
    fn MapVirtualKeyW(code: u32, map_type: u32) -> u32;
}

static B: AtomicIsize = AtomicIsize::new(0);
static HELPER: std::sync::Mutex<Vec<std::process::Child>> = std::sync::Mutex::new(Vec::new());
const TITLE_B: &str = "SW809_B";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    BOff,
    BOn,
    F2,
}

pub(crate) fn mode() -> Option<Mode> {
    match arg_value("--sw-mode=").as_deref() {
        Some("b-off") => Some(Mode::BOff),
        Some("b-on") => Some(Mode::BOn),
        Some("f2") => Some(Mode::F2),
        _ => None,
    }
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::BOff => "b-off",
        Mode::BOn => "b-on",
        Mode::F2 => "f2",
    }
}

fn delays() -> Vec<u64> {
    let v: Vec<u64> = arg_value("--sw-delays=")
        .unwrap_or_else(|| "0_30_60_100_150_300_500_1500".into())
        .split('_')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if v.is_empty() {
        vec![0]
    } else {
        v
    }
}

fn find_b() -> Option<HWND> {
    let t = wide(TITLE_B);
    // SAFETY: タイトルとクラスで探すだけ。`t` は呼び出しの間生きている。
    unsafe { FindWindowW(w!("EDIT"), PCWSTR(t.as_ptr())).ok() }.filter(|h| !h.0.is_null())
}

fn window_b() -> Option<HWND> {
    let cur = B.load(Ordering::SeqCst);
    if cur != 0 {
        return Some(HWND(cur as *mut _));
    }
    let exe = std::env::current_exe().ok()?;
    let child = std::process::Command::new(exe)
        .arg("--fr-helper")
        .arg(format!("--fr-title={TITLE_B}"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    HELPER.lock().ok()?.push(child);
    for _ in 0..40 {
        if let Some(h) = find_b() {
            B.store(h.0 as isize, Ordering::SeqCst);
            return Some(h);
        }
        sleep_ms(250);
    }
    None
}

pub(crate) fn kill_b() {
    if let Ok(mut g) = HELPER.lock() {
        for mut c in g.drain(..) {
            let _ = c.kill();
        }
    }
}

fn to_b(b: HWND) -> bool {
    for _ in 0..3 {
        raise_foreign(b);
        sleep_ms(200);
        // SAFETY: 前面窓の取得と、SetForegroundWindow が拒否される CI 向けのフォールバック。
        unsafe {
            if GetForegroundWindow() == b {
                return true;
            }
            SwitchToThisWindow(b, true);
        }
        sleep_ms(200);
        // SAFETY: 前面窓の取得のみ。
        if unsafe { GetForegroundWindow() } == b {
            return true;
        }
    }
    false
}

/// A を前面に戻し、前面になったのを観測した時刻からの経過の基準を返す(待たずに戻る)。
/// 自前の窓は入力欄へのフォーカスまで、他プロセスの窓は前面窓までを見る。最大 1.5 秒。
fn back_to_a_fast() -> (bool, u64) {
    let t0 = Instant::now();
    if target().owns_himc() {
        // SAFETY: 自プロセスの最上位窓への PostMessage のみ(own_refocus と同じ。400ms の待ちだけ省く)。
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(&TOP)), WM_TS_FRONT, WPARAM(0), LPARAM(0));
        }
    } else {
        raise_foreign(hwnd_of(&TOP));
    }
    let mut retried = false;
    while t0.elapsed() < Duration::from_millis(1500) {
        let ok = if target().owns_himc() {
            own_focus_ok()
        } else {
            // SAFETY: 前面窓の取得のみ。
            unsafe { GetForegroundWindow() == hwnd_of(&TOP) }
        };
        if ok {
            return (true, u64::try_from(t0.elapsed().as_millis()).unwrap_or(0));
        }
        if !retried && t0.elapsed() > Duration::from_millis(500) {
            retried = true;
            // SAFETY: SetForegroundWindow が拒否される CI 向けのフォールバック。
            unsafe { SwitchToThisWindow(hwnd_of(&TOP), true) };
        }
        sleep_ms(1);
    }
    (false, u64::try_from(t0.elapsed().as_millis()).unwrap_or(0))
}

/// 試行の前の準備(入力欄のクリアを含む)。戻り値は `sw` レコード(打鍵の後に記録する)。
/// `None` なら従来どおり(呼び出し側が before_trial・clear・start_delay を行う)。
pub(crate) fn before_trial(child: HWND, idx: u64) -> Option<serde_json::Value> {
    let m = mode()?;
    let ds = delays();
    let d = ds[usize::try_from(idx).unwrap_or(0) % ds.len()];
    clear_text(child);
    sleep_ms(300);
    match m {
        Mode::F2 => {
            press(VK_DBE_HIRAGANA, 0x70, 50);
            sleep_ms(d);
            Some(json!({"type":"sw","mode":mode_name(m),"delay_ms":d,"utc":utc_hms()}))
        }
        Mode::BOff | Mode::BOn => {
            let Some(b) = window_b() else {
                return Some(
                    json!({"type":"sw","mode":mode_name(m),"delay_ms":d,"error":"no window B"}),
                );
            };
            if m == Mode::BOn {
                // A の IME を OFF にする(A が前面)。
                press(VK_IME_OFF, 0x70, 50);
                sleep_ms(800);
            }
            clear_window_b(b);
            let b_ok = to_b(b);
            sleep_ms(300);
            if m == Mode::BOff {
                press(VK_IME_OFF, 0x70, 50);
            } else {
                press(ime_on_key(0), 0x70, 50);
            }
            sleep_ms(1200);
            let utc_back = utc_hms();
            let (back_ok, fg_ms) = back_to_a_fast();
            sleep_ms(d);
            Some(
                json!({"type":"sw","mode":mode_name(m),"delay_ms":d,"b_ok":b_ok,"back_ok":back_ok,
                        "fg_wait_ms":fg_ms,"utc_back":utc_back}),
            )
        }
    }
}

fn clear_window_b(h: HWND) {
    const WM_SETTEXT: u32 = 0x000C;
    let empty = [0u16];
    // SAFETY: 空文字列を短いタイムアウト付きで送るだけ。
    unsafe {
        let _ = SendMessageTimeoutW(
            h,
            WM_SETTEXT,
            WPARAM(0),
            LPARAM(empty.as_ptr() as isize),
            windows::Win32::UI::WindowsAndMessaging::SEND_MESSAGE_TIMEOUT_FLAGS(0x0002),
            500,
            None,
        );
    }
}

/// `b-on`(A は直接入力)の期待: 各キーの素の文字(単打のみ。親指シフト面は期待できないので `?` にする)。
pub(crate) fn raw_expect(seq: &[Cell]) -> String {
    seq.iter()
        .map(|c| {
            if c.face != Face::Single {
                return '?';
            }
            // SAFETY: 仮想キー → 文字の変換表を引くだけ。
            let ch = unsafe { MapVirtualKeyW(c.vk, 2) } & 0xFFFF;
            char::from_u32(ch).map_or('?', |c| c.to_ascii_lowercase())
        })
        .collect()
}

pub(crate) fn expects_raw() -> bool {
    mode() == Some(Mode::BOn)
}
