//! `--mode=focus-restore`(観測専用): 半角英数の持続トグル(左 Shift 単独タップ、BUG-25/ADR-107)の最中にフォーカスが
//! 別窓へ移ったとき、フォーカス変更時の強制復元(`runtime/ime_refresh.rs::ir_notify_focus_changed` →
//! `runtime/key_pipeline.rs::kp_restore_kana_from_half_width(false)`)が実 IME のどこに効くかを測る。
//!
//! 窓 A = ハーネス自身の入力欄(`--form=edit`)、窓 B = 自分自身を `--fr-helper` で再起動した**別プロセス**の EDIT 窓
//! (awase のフォーカス変更通知はプロセスが変わったときだけ呼ばれるため)。B の IME は別プロセスなので `imc` 経路でしか読めない
//! (`imm` は None)。手順(試行ごと):
//!   pre    : IME を OFF→ON にそろえた直後の A/B の open・conv
//!   tap1   : 左 Shift 単独タップ(半角英数トグル ON。要 `general = half_width_alnum_toggle = "all"`)。A の conv から NATIVE が
//!            消える(半角英数に入った)まで最大 4 回タップし(前の試行のトグルが残っていると最初のタップは解除になる)、入れなければ entered=false で試行を捨てる
//!   away   : B を前面にして +150/+600/+1500ms(`--fr-control` のときは移さず同じ時間だけ待つ。対照)
//!   back   : A を前面に戻して +150/+600/+1500ms
//!   typed  : かな単打(ka)を打って Enter で確定した A の本文(Engine と実 IME が揃っているか)
//!   tap2   : もう一度左 Shift 単独タップ(トグルのラッチが awase に残っているかの手がかり)の +300/+1000ms
//! 各時点で A/B の open・conv を 2 経路(`imm`=ImmGetContext 系、`imc`=既定 IME 窓への WM_IME_CONTROL)で記録する。
//! 判定はしない(観測のみ)。`tools/e2e/ime_key_matrix/check_focusrestore.py` が表にし、awase.log の該当行を添える。
//! 記録: `fr_config`(1回)・`fr_stage`(時点ごと)・`fr_trial`(試行のまとめ)。時刻 `utc` は awase.log と突合せる用。
//!
//! 引数: `--fr-n=N`(試行数。既定3) / `--fr-control`(B へ移さない対照) / `--fr-b-open` / `--fr-sameproc`(B を同一プロセスの別スレッド窓にする。ADR-245 Q4)。
//! ADR-245 PR 3(strict 判定用): `--fr-scenario=<名前>`(下の `Scenario`) / `--fr-expect=resume|rebuild`
//! (resume=戻って最初の打鍵でかなへ復元される IME〈MS-IME 本体・GJI の MS-IME プリセット〉、rebuild=GJI の ATOK プリセット
//! 〈F2 が純粋なトグルなので注入せずトグルを立て直し、戻った A は英数のまま〉)。判定は check_focusrestore.py が `--strict` で行う。

#[allow(clippy::wildcard_imports)]
use super::*;
use windows::core::{w, PCWSTR};

static B: AtomicIsize = AtomicIsize::new(0);
/// A→B→C→A 用の 3 つ目の窓(別プロセス)。
static C: AtomicIsize = AtomicIsize::new(0);
const TITLE_B: &str = "FOCUSRESTORE_B";
const TITLE_C: &str = "FOCUSRESTORE_C";

const VK_LSHIFT: u32 = 0xA0;
const SCAN_LSHIFT: u16 = 0x2A;

/// 窓 B のプロセス(`--fr-helper` で自分自身を再起動したもの)。awase のフォーカス変更通知
/// (`ir_notify_focus_changed`)は**プロセスが変わったとき**だけ呼ばれる(`focus_tracking.rs::advance_focus_tracking` が
/// pid の違いで判定。run 37779401312 で同一プロセス内の窓移動は通知されないことを確認)ので、B は別プロセスにする。
static HELPER: std::sync::Mutex<Vec<std::process::Child>> = std::sync::Mutex::new(Vec::new());

/// `--fr-helper`: 窓 B(`--fr-title=` で C)だけを作って閉じられるまで待つ。`main` の先頭から呼ぶ。
pub(crate) fn helper_main() {
    let title = wide(&arg_value("--fr-title=").unwrap_or_else(|| TITLE_B.into()));
    // SAFETY: 自スレッドの窓とメッセージループのみ。`title` は呼び出しの間生きている。
    unsafe {
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            520,
            80,
            420,
            220,
            None,
            None,
            None,
            None,
        );
        let mut msg = MSG::default();
        while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
}

fn find_titled(title: &str) -> Option<HWND> {
    let t = wide(title);
    // SAFETY: タイトルとクラスで探すだけ。`t` は呼び出しの間生きている。
    unsafe { FindWindowW(w!("EDIT"), PCWSTR(t.as_ptr())).ok() }.filter(|h| !h.0.is_null())
}

fn window_b() -> Option<HWND> {
    let cur = B.load(Ordering::SeqCst);
    if cur != 0 {
        return Some(HWND(cur as *mut _));
    }
    if has_flag("--fr-sameproc") {
        return window_b_same_process();
    }
    spawn_helper(TITLE_B, &B)
}

/// A→B→C→A 用の窓 C(常に別プロセス)。
fn window_c() -> Option<HWND> {
    let cur = C.load(Ordering::SeqCst);
    if cur != 0 {
        return Some(HWND(cur as *mut _));
    }
    spawn_helper(TITLE_C, &C)
}

fn spawn_helper(title: &str, slot: &AtomicIsize) -> Option<HWND> {
    let exe = std::env::current_exe().ok()?;
    let child = std::process::Command::new(exe)
        .arg("--fr-helper")
        .arg(format!("--fr-title={title}"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    HELPER.lock().ok()?.push(child);
    for _ in 0..40 {
        if let Some(h) = find_titled(title) {
            slot.store(h.0 as isize, Ordering::SeqCst);
            return Some(h);
        }
        sleep_ms(250);
    }
    None
}

/// `--fr-sameproc`(ADR-245 Q4): 窓 B をこのハーネスと**同じプロセス**の別スレッドに作る。awase のフォーカス変更通知は
/// プロセスが変わったときだけ呼ばれる(`advance_focus_tracking` の pid 判定)ので、同一プロセス内の窓移動で
/// 半角英数トグルが実 IME・awase に持ち越されるか(戻った後に打った文字)を測る。
fn window_b_same_process() -> Option<HWND> {
    let (tx, rx) = std::sync::mpsc::channel::<isize>();
    std::thread::spawn(move || {
        // SAFETY: このスレッド専用の窓とメッセージループのみ。
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                w!("FOCUSRESTORE_B"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                520,
                80,
                420,
                220,
                None,
                None,
                None,
                None,
            );
            let _ = tx.send(hwnd.map_or(0, |h| h.0 as isize));
            let mut msg = MSG::default();
            while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&raw const msg);
                DispatchMessageW(&raw const msg);
            }
        }
    });
    let raw = rx.recv_timeout(Duration::from_secs(5)).unwrap_or(0);
    B.store(raw, Ordering::SeqCst);
    (raw != 0).then(|| HWND(raw as *mut _))
}

fn kill_b() {
    if let Ok(mut g) = HELPER.lock() {
        for mut c in g.drain(..) {
            let _ = c.kill();
        }
    }
}

/// 別プロセス(または別スレッド)の EDIT 窓の本文。`WM_GETTEXT` は標準コントロールではプロセスをまたいで渡る。
fn read_window_text(h: HWND) -> String {
    const WM_GETTEXT: u32 = 0x000D;
    let mut buf = vec![0u16; 256];
    let mut res = 0usize;
    // SAFETY: 短いタイムアウト付きの同期送信。`buf` は呼び出しの間生きている。
    unsafe {
        let r = SendMessageTimeoutW(
            h,
            WM_GETTEXT,
            WPARAM(buf.len()),
            LPARAM(buf.as_mut_ptr() as isize),
            windows::Win32::UI::WindowsAndMessaging::SEND_MESSAGE_TIMEOUT_FLAGS(0x0002),
            500,
            Some(&raw mut res),
        );
        if r.0 == 0 {
            return String::new();
        }
    }
    String::from_utf16_lossy(&buf[..res.min(buf.len() - 1)])
}

fn clear_window_text(h: HWND) {
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

/// `ImmGetContext` 経由(同一プロセスの窓)の (open, conv)。
fn imm_state(h: HWND) -> (Option<bool>, Option<u32>) {
    use windows::Win32::UI::Input::Ime::{
        ImmGetConversionStatus, ImmGetOpenStatus, ImmReleaseContext, IME_CONVERSION_MODE,
        IME_SENTENCE_MODE,
    };
    // SAFETY: 自プロセスの窓への IMM 呼び出し。取得した HIMC は必ず解放する。
    unsafe {
        let himc = windows::Win32::UI::Input::Ime::ImmGetContext(h);
        if himc.is_invalid() {
            return (None, None);
        }
        let open = ImmGetOpenStatus(himc).as_bool();
        let mut conv = IME_CONVERSION_MODE::default();
        let mut sent = IME_SENTENCE_MODE::default();
        let ok = ImmGetConversionStatus(himc, Some(&raw mut conv), Some(&raw mut sent)).as_bool();
        let _ = ImmReleaseContext(h, himc);
        (Some(open), ok.then_some(conv.0))
    }
}

/// 既定 IME 窓への `WM_IME_CONTROL`(awase 本体と同型の読み方)。`code` は IMC_GETCONVERSIONMODE=1 / IMC_GETOPENSTATUS=5。
fn imc_read(h: HWND, code: usize) -> Option<usize> {
    const WM_IME_CONTROL: u32 = 0x0283;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    // SAFETY: 既定 IME ウィンドウへ短いタイムアウト付きで同期送信するだけ。
    unsafe {
        let ime_wnd = windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd(h);
        if ime_wnd.0.is_null() {
            return None;
        }
        let mut result = 0usize;
        let r = SendMessageTimeoutW(
            ime_wnd,
            WM_IME_CONTROL,
            WPARAM(code),
            LPARAM(0),
            windows::Win32::UI::WindowsAndMessaging::SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG),
            500,
            Some(&raw mut result),
        );
        (r.0 != 0).then_some(result)
    }
}

fn win_json(h: HWND) -> serde_json::Value {
    let (imm_open, imm_conv) = imm_state(h);
    let imc_open = imc_read(h, 5).map(|v| v != 0);
    let imc_conv = imc_read(h, 1);
    json!({"imm_open":imm_open,"imm_conv":imm_conv,"imc_open":imc_open,"imc_conv":imc_conv})
}

fn fg_name(b: HWND) -> &'static str {
    // SAFETY: 前面窓の取得のみ。
    let fg = unsafe { GetForegroundWindow() };
    if fg == hwnd_of(&TOP) {
        "A"
    } else if fg == b {
        "B"
    } else {
        "other"
    }
}

fn stage(n: u64, name: &str, at_ms: u64, a: HWND, b: HWND) {
    rec(
        &json!({"type":"fr_stage","n":n,"stage":name,"at_ms":at_ms,"utc":utc_hms(),
        "fg":fg_name(b),"A":win_json(a),"B":win_json(b)}),
    );
}

/// 時点列 `offsets`(ms、昇順)で `stage` を記録する。基準はこの関数の呼び出し時刻。
fn stages(n: u64, name: &str, offsets: &[u64], a: HWND, b: HWND) {
    let mut prev = 0;
    for &o in offsets {
        sleep_ms(o - prev);
        prev = o;
        stage(n, name, o, a, b);
    }
}

fn to_b(b: HWND) -> bool {
    for _ in 0..3 {
        raise_foreign(b);
        sleep_ms(200);
        // SAFETY: 前面窓の取得と、SetForegroundWindow が拒否される CI 向けのフォールバック(focus_away と同じ)。
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

/// `--mode=focus-restore`。`ime_ready` の後に呼ぶ。
pub(crate) fn focus_restore_scenario(child: HWND, cells: &[Vec<Cell>; 3]) {
    run(child, cells);
    kill_b();
}

/// `--fr-scenario=`: 戻り方・戻った直後の操作・窓の動かし方の違い(ADR-245 の PR 3)。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scenario {
    /// A→B→A(`refocus`)で戻り、最初の操作は文字キー。
    Basic,
    /// 戻り方を Alt+Tab にする(マーカー付き注入で物理扱い)。
    AltTab,
    /// 戻って最初の操作を左 Shift 単独タップにする。
    LShiftFirst,
    /// 戻って最初の打鍵を Shift+文字にする(カタカナにならないこと)。
    ShiftChar,
    /// A→B→A→B→A の 2 往復。
    Twice,
    /// A→B→C→A(C は別プロセスの 3 つ目の窓)。
    Abc,
    /// B で IME を OFF にしてから A へ戻る(effective_open 偽の Drop。観測)。
    BImeOff,
}

fn scenario() -> Scenario {
    match arg_value("--fr-scenario=").as_deref() {
        Some("alttab") => Scenario::AltTab,
        Some("lshift") => Scenario::LShiftFirst,
        Some("shiftchar") => Scenario::ShiftChar,
        Some("twice") => Scenario::Twice,
        Some("abc") => Scenario::Abc,
        Some("bimeoff") => Scenario::BImeOff,
        _ => Scenario::Basic,
    }
}

fn scenario_name(s: Scenario) -> &'static str {
    match s {
        Scenario::Basic => "basic",
        Scenario::AltTab => "alttab",
        Scenario::LShiftFirst => "lshift",
        Scenario::ShiftChar => "shiftchar",
        Scenario::Twice => "twice",
        Scenario::Abc => "abc",
        Scenario::BImeOff => "bimeoff",
    }
}

/// マーカー付き注入の Alt↓→Tab↓↑→Alt↑(前面窓を 1 つ前に戻す)。
fn press_alt_tab() {
    send_key(0xA4, 0x38, true);
    sleep_ms(80);
    send_key(0x09, 0x0F, true);
    sleep_ms(60);
    send_key(0x09, 0x0F, false);
    sleep_ms(80);
    send_key(0xA4, 0x38, false);
}

/// `probe` を打って Enter で確定し、A の本文を読んで消す。
fn type_probe_in_a(child: HWND, probe: &Cell) -> String {
    press(probe.vk, probe.scan, 60);
    sleep_ms(700);
    press(VK_RETURN, 0x1C, 50);
    sleep_ms(700);
    let text = read_text(child);
    clear_text(child);
    text
}

fn run(child: HWND, cells: &[Vec<Cell>; 3]) {
    let n_trials: u64 = arg_value("--fr-n=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let control = has_flag("--fr-control");
    let b_open = has_flag("--fr-b-open");
    let scn = scenario();
    let expect = arg_value("--fr-expect=").unwrap_or_else(|| "resume".into());
    let Some(b) = window_b() else {
        rec(&json!({"type":"abort","reason":"focus-restore: 窓 B を作れなかった"}));
        return;
    };
    let c = if scn == Scenario::Abc {
        let Some(c) = window_c() else {
            rec(&json!({"type":"abort","reason":"focus-restore: 窓 C を作れなかった"}));
            return;
        };
        Some(c)
    } else {
        None
    };
    let a = child;
    let Some(probe) = cells[0]
        .iter()
        .find(|c| c.romaji == "ka")
        .cloned()
        .or_else(|| cells[0].first().cloned())
    else {
        rec(&json!({"type":"abort","reason":"focus-restore の打鍵確認に使う単打セルが無い"}));
        return;
    };
    rec(
        &json!({"type":"fr_config","n":n_trials,"control":control,"b_open":b_open,"sameproc":has_flag("--fr-sameproc"),
        "scenario":scenario_name(scn),"expect":expect,
        "class_a":class_of(a),"class_b":class_of(b)}),
    );
    for n in 0..n_trials {
        if !focus_ok() {
            refocus();
        }
        if !focus_ok() {
            rec(
                &json!({"type":"abort","reason":format!("focus-restore 試行前にフォーカスが外れた n={n}")}),
            );
            return;
        }
        let utc0 = utc_hms();
        // 前提: IME を OFF→ON にそろえる(awase の明示意図も ON になる)。
        turn_ime_on(0);
        clear_text(child);
        clear_window_text(b);
        sleep_ms(300);
        // 前の試行の半角英数が実 IME に残っていることがある(強制復元が効かないのがこの調査の主題)ので、
        // A の conv に NATIVE が無ければひらがなキーで戻す。戻せなかった試行は下の entered 判定で捨てる。
        for _ in 0..2 {
            if imm_state(a).1.is_some_and(|c| c & 1 != 0) {
                break;
            }
            press(VK_DBE_HIRAGANA, 0x70, 50);
            sleep_ms(800);
        }
        // `--fr-b-open`: B の IME も開いておく(B で ON キーを打つ)。新しい窓が IME ON のとき、強制復元の
        // ひらがな注入が B へ届くか(effective_open=true の分岐)を見る。トグルはまだ入れていない。
        if b_open && !control {
            let ok = to_b(b);
            sleep_ms(300);
            press(ime_on_key(0), 0x70, 50);
            sleep_ms(1200);
            stage(n, if ok { "b_prep" } else { "b_prep_fail" }, 0, a, b);
            refocus();
            sleep_ms(1000);
        }
        stage(n, "pre", 0, a, b);
        // tap1: 左 Shift 単独タップで半角英数トグルへ。
        // 前の試行のトグルが awase に残っていると最初のタップは「解除」になる(run 37779401312 で観測)ので、
        // A の conv から NATIVE(bit0)が消える(=半角英数に入った)まで最大 4 回タップする。回数を記録する。
        let utc_tap1 = utc_hms();
        let mut entered = false;
        let mut taps = 0u64;
        for k in 0..4u64 {
            let before_native = imm_state(a).1.is_some_and(|c| c & 1 != 0);
            press(VK_LSHIFT, SCAN_LSHIFT, 60);
            taps += 1;
            sleep_ms(700);
            stage(n, &format!("tap1#{k}"), 700, a, b);
            // 押す前に NATIVE があり、押した後に無い = このタップで半角英数に入った。
            if before_native && imm_state(a).1.is_some_and(|c| c & 1 == 0) {
                entered = true;
                break;
            }
            sleep_ms(300);
        }
        if !entered {
            rec(
                &json!({"type":"fr_trial","n":n,"control":control,"utc":utc0,"utc_tap1":utc_tap1,
                "entered":false,"taps":taps,"utc_end":utc_hms()}),
            );
            continue;
        }
        stages(n, "settled", &[300, 1000], a, b);
        // away: B へ移す(対照は移さない)。
        let utc_away = utc_hms();
        let mut away_ok = if control { true } else { to_b(b) };
        match scn {
            Scenario::Twice if !control => {
                // 1 往復目(B へ→A へ)を挟み、もう一度 B へ移す。
                sleep_ms(1200);
                stage(n, "away0", 0, a, b);
                refocus();
                sleep_ms(1200);
                stage(n, "back0", 0, a, b);
                away_ok &= to_b(b);
            }
            Scenario::Abc => {
                // B から C へ移す。最後の away 列は C が前面のとき(fg=other)に記録する。
                sleep_ms(1200);
                stage(n, "away_b", 0, a, b);
                if let Some(c) = c {
                    away_ok &= to_b(c);
                }
            }
            Scenario::BImeOff if !control => {
                // B の IME を OFF にする(A の belief は B の閉を引き継ぐ)。
                sleep_ms(500);
                press(VK_IME_OFF, 0x70, 50);
                sleep_ms(800);
            }
            _ => {}
        }
        stages(n, "away", &[150, 600, 1500], a, b);
        // B-3: B で自分の最初の打鍵を打つ(B に ObservedEisu が持ち越されていれば NICOLA が止まって素通しになる)。
        // B の IME が閉(--fr-b-open なし)のときは素通しが正なので、判定は b_open のときだけ。
        let b_typed = if control || scn == Scenario::Abc || scn == Scenario::BImeOff {
            None
        } else {
            press(probe.vk, probe.scan, 60);
            sleep_ms(700);
            press(VK_RETURN, 0x1C, 50);
            sleep_ms(700);
            let t = read_window_text(b);
            clear_window_text(b);
            Some(t)
        };
        // back: A へ戻す。
        let utc_back = utc_hms();
        let mut alttab_ok = true;
        if !control {
            if scn == Scenario::AltTab {
                press_alt_tab();
                sleep_ms(500);
                alttab_ok = focus_ok();
                if !alttab_ok {
                    refocus();
                }
            } else {
                refocus();
            }
        }
        let back_ok = focus_ok();
        stages(n, "back", &[150, 600, 1500], a, b);
        // 戻って最初の操作。
        let mut first_text: Option<String> = None;
        match scn {
            Scenario::LShiftFirst => {
                press(VK_LSHIFT, SCAN_LSHIFT, 60);
                sleep_ms(700);
                stage(n, "firsttap", 700, a, b);
            }
            Scenario::ShiftChar => {
                send_key(VK_LSHIFT, SCAN_LSHIFT, true);
                sleep_ms(60);
                press(probe.vk, probe.scan, 60);
                sleep_ms(40);
                send_key(VK_LSHIFT, SCAN_LSHIFT, false);
                sleep_ms(700);
                press(VK_RETURN, 0x1C, 50);
                sleep_ms(700);
                first_text = Some(read_text(child));
                clear_text(child);
                stage(n, "shiftchar", 0, a, b);
            }
            _ => {}
        }
        // typed: かな単打を打って確定し、本文を読む。
        let utc_type = utc_hms();
        let text = type_probe_in_a(child, &probe);
        stage(n, "typed", 0, a, b);
        // tap2: もう一度左 Shift 単独タップ。
        let utc_tap2 = utc_hms();
        press(VK_LSHIFT, SCAN_LSHIFT, 60);
        stages(n, "tap2", &[300, 1000], a, b);
        let utc_end = utc_hms();
        rec(
            &json!({"type":"fr_trial","n":n,"control":control,"scenario":scenario_name(scn),"utc":utc0,"utc_tap1":utc_tap1,
            "entered":true,"taps":taps,"utc_away":utc_away,"utc_back":utc_back,"utc_type":utc_type,"utc_tap2":utc_tap2,"utc_end":utc_end,
            "away_ok":away_ok,"back_ok":back_ok,"alttab_ok":alttab_ok,"b_typed":b_typed,"first_text":first_text,
            "typed":{"text":text,"expect":probe.kana.to_string(),"expect_romaji":probe.romaji.to_string(),"vk":probe.vk,"ok":text.trim() == probe.kana.to_string()}}),
        );
        // 次の試行へ持ち越さない: A を前面に戻し、半角英数のままなら Shift 単独タップで戻す余地は残さず、次の turn_ime_on に任せる。
        if !focus_ok() {
            refocus();
        }
        sleep_ms(500);
    }
}
