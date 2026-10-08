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
//! - `chromebar` / `chromepage` : 本物の Chrome(専用プロファイル。アドレスバー / ページ内 textarea)。UI Automation で読む。
//! - `ext` : 宣言表(tools/e2e/input_forms/forms.toml)の項目。`--ext=<名前>` `--ext-manifest=PATH` `--ext-repo=DIR`。Java・Office・wx・Qt など。
//! - `bugreport` : 本物の `awase-settings.exe --bug-report` の「説明」欄。UI Automation で読む。
//!
//! 入力先ごとの差は `target.rs` の `InputTarget` に閉じ込めてある(読む・空にする・前面へ戻す・フォーカス確認・終了)。
//! 新しい入力先は `InputTarget` を実装して `target::launch` に 1 行足すだけでよく、シナリオ側は触らない。
//! Zoom・UWP は CI で安定して動かせない(フォーカス/起動が不確定でストレスと切り分けられない)ので対象外。
//! 別プロセスの入力先は自プロセスの HIMC を持たないため、`--mode=drift|drift-on|keymatrix` は使えない(abort する)。
//!
//! ## 摂動(`perturb.rs`、すべて既定オフ)
//! 連続打鍵では作れない実利用に近い状況を試行に差し込む: `--cold` / `--pause-after=N --pause-ms=MS` / `--idle=MS` /
//! `--switch-focus` / `--start-delay=MS` / `--interrupt=off_on|off|f2` / `--settle-read`。意味は `perturb.rs` の表を参照。
//! 指定した摂動は `config` レコードの `perturb` に記録される。
//! 相手(GJI の変換サーバ等)を遅くする摂動は `suspend.rs`(`--suspend-proc=NAME --suspend-at-ms=T --suspend-ms=D`)。
//!
//! ## フラグ
//! `--form=edit|multi|rich|tsf|chromebar|chromepage|bugreport`(`--chrome-path=PATH` で Chrome を指定) / `--mode=nicola|raw|drift|drift-on|keymatrix|reopen` / `--interval=MS`(1文字あたりの間隔。既定20) /
//! `--trials=N`(種別ごとの試行数。既定4。`--mode=drift` では試行回数として使う) / `--len=N`(1試行の文字数。既定40) / `--seed=S` /
//! `--kinds=single,thumb,mixed` / `--layout=PATH`(.yab。既定 layout/nicola_keytop.yab) /
//! `--activate-gji`(GJI/MS-IME のプロファイルを有効化。CI 用) / `--msime`(有効化する IME を Microsoft IME に) /
//! `--no-awase`(awase を待たない。`--mode=raw` の対照実験用) / `--log=PATH`。
//! 打鍵の揺らぎ(`jitter.rs`、既定オフ): `--jit-iv=SIGMA` / `--jit-thumb=MIN_MS:MAX_MS` / `--jit-char-first=PCT`。
//! `--mode=raw` は awase なしで、期待文字列と同じ内容をローマ字の生キーで同じ速度で注入する対照実験
//! (入力先+IME 単体がその速度を受けられるかを、awase と切り離して見る)。
//!
//! `--mode=drift`(ADR-191 09-T4、BUG-020 の回帰観測): 打鍵ストレスとは別の手順で、drift correction
//! (`runtime/ime_refresh.rs::ir_apply_drift_correction`)が明示意図の OFF 直後に実 IME を不適切に ON へ
//! 戻す/固定するか(2026-07-08 の実機症状)を観測する。手順は「IME を ON にそろえる → `--drift-off-vk`
//! (既定 0x1D)を単発で押す → +100/+400/+1500ms で `ImmGetOpenStatus` を読む」を `--trials` 回繰り返す。
//! **チョードでない単一キーで OFF を駆動する前提**: `--drift-off-vk` は awase 側の設定
//! `keys.ime_off` をこの単一キーに上書きした状態で使うこと(既定の `Ctrl+無変換` は、`modifier_snapshot.ctrl`
//! が `is_physical_key_down`(PHYSICAL_KEY_STATE)で判定されるため、SendInput 注入では物理 Ctrl 押下として
//! 認識されず駆動できない)。追加フラグ: `--drift-off-vk=0xNN`(既定 0x1D=VK_NONCONVERT)。
//!
//! `--mode=startup`(BUG-163 / D1): 対象窓を awase より先に IME ON/OFF にし、起動直後の初打鍵または
//! 3 秒間の OFF 維持を検証する。`--startup-ime=on|off` は必須。
//! 追加フラグ(MS-IME×Chrome の ON 起動で最初の文字が `ka` になる件の切り分け用): `--no-awase` なら awase がいない対照として
//! NICOLA 単打の代わりに生の `k`,`a` を打つ(閉なら `ka`、開なら `か`)。`--startup-skip-refocus2` は 2 回目の `refocus()` を省く。
//! `startup_typed.real_ime_open_before_type` は打鍵直前に既定 IME 窓へ `IMC_GETOPENSTATUS` を送った値(Chrome 等の別プロセスでも読める)。
//!
//! `--mode=drift-on`(ADR-178 領域A撤去後の回帰観測): reassert/force-on 撤去後、drift correction「だけ」で
//! TsfNative 相当の入力先(`--form=tsf`)の ON 回復が働くかを見る。手順は「IME を ON にそろえる(awase が明示意図 ON を
//! 持つ)→ **ハーネスが自プロセスの入力欄の IME を直接閉じる**(awase を経由しない
//! 「ずれ」。`ImmSetOpenStatus` は別スレッドから失敗するので既定 IME ウィンドウへ `WM_IME_CONTROL` を送る)→ +500/+1500/+3000ms で `ImmGetOpenStatus` を読む → かな単打を1回打って確定し、結果のテキストを読む
//! (API の成功表示だけでなく実タイピングで ON/OFF を確認する)」を `--trials` 回繰り返す。
//! 記録は `drift_on_pre`(`on_key`=ON にしたキー) / `drift_on_close`(`set_ret` は記録のみ) / `drift_on_check` / `drift_on_typed`。
//! pre/close/typed には `utc`(HH:MM:SS.mmm、awase.log の時刻と突合せる用)を付ける。ON キーは awase の明示意図(SyncKey)に
//! なる `VK_IME_ON`(0x16)を先頭にする(MS-IME の 0xF2 は mode-key passthrough で意図が消える)。
//! `--drift-off-ctrl-muhenkan` では直接 close の代わりに、マーカー付き SendInput で
//! Ctrl↓→無変換↓→無変換↑→Ctrl↑を送る。debug awase はこの注入を物理キーとして扱う。
//!
//! `--mode=focus-restore`(観測専用): 半角英数の持続トグル(左 Shift 単独タップ)中にフォーカスが別窓へ移ったとき、強制復元が実 IME のどこに効くか。詳細は `focusrestore.rs` 冒頭。判定はせず check_focusrestore.py が表にする。
//!
//! `--mode=keymatrix`(ADR-208 L3b): 「ずれの作り方 × 明示キー」行列。詳細は `keymatrix.rs` 冒頭。判定は check_keymatrix.py。
//!
//! `--mode=reopen`(ADR-203 e2e (c)、BUG-170 の実機確認): 「OFF 前に1語確定 → 物理 OFF(`VK_IME_OFF`)→ `--reopen-gap`(既定600ms、1秒以内)後に
//! 物理 ON(`--reopen-on-key`、既定は GJI 0x16・MS-IME 0xF2。GJI の ATOK プリセットで 0xF2 は ON にならないことを run 36555043470 で確認)→ 即打鍵(`--reopen-type-delay`、既定0)」を `--trials` 回。別プロセスの入力先(Chrome)でも動く
//! (実 IME の開閉は読まず、入力先のテキストと awase.log で判定する)。記録は `reopen_pre` / `reopen_on` / `reopen_typed`。判定は check_reopen.py。`--settle-read`(Chrome 等の描画遅れ対策)にも対応する。
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
//! `[TS-JSON] {...}` の行(1行1JSON、type = config/focus/ready/trial/inject/abort/done、`--mode=drift`
//! では加えて drift_pre/drift_check)が機械可読の記録。完走マーカーは `=== 完了 ===`。

#![windows_subsystem = "windows"]
#![allow(unsafe_code)]

mod focusrestore;
mod jitter;
mod keymatrix;
mod perturb;
mod preedit;
mod suspend;
mod target;
mod uia;

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
    FindWindowW, GetClassInfoExW, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo,
    GetMessageW, GetWindowThreadProcessId, PostMessageW, PostQuitMessage, RegisterClassExW,
    SendMessageTimeoutW, SendMessageW, SetForegroundWindow, SetWindowsHookExW, ShowWindow,
    SwitchToThisWindow, TranslateMessage, CW_USEDEFAULT, GUITHREADINFO, KBDLLHOOKSTRUCT, MSG,
    SW_SHOW, WH_KEYBOARD_LL, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_DESTROY, WM_GETTEXT,
    WM_GETTEXTLENGTH, WM_KEYDOWN, WM_KEYUP, WM_SETTEXT, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSEXW,
    WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE, WS_VSCROLL,
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
const VK_LCONTROL: u32 = 0xA2;
const SCAN_LCONTROL: u16 = 0x1D;
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

/// `utc_stamp()` の `[` `]` `Z` を除いた `HH:MM:SS.mmm`(awase.log の ISO8601 時刻部分と文字列比較できる)。
fn utc_hms() -> String {
    utc_stamp()
        .trim_matches(|c| c == '[' || c == ']' || c == 'Z')
        .to_string()
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
    ChromeBar,
    ChromePage,
    BugReport,
    Ext,
}

impl Form {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "edit" => Some(Self::Edit),
            "multi" => Some(Self::Multi),
            "rich" => Some(Self::Rich),
            "tsf" => Some(Self::Tsf),
            "chromebar" => Some(Self::ChromeBar),
            "chromepage" => Some(Self::ChromePage),
            "bugreport" => Some(Self::BugReport),
            "ext" => Some(Self::Ext),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Multi => "multi",
            Self::Rich => "rich",
            Self::Tsf => "tsf",
            Self::ChromeBar => "chromebar",
            Self::ChromePage => "chromepage",
            Self::BugReport => "bugreport",
            Self::Ext => "ext",
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

/// 自前の Win32 窓(`edit`/`multi`/`rich`/`tsf`)を作り、`TOP`/`CHILD` を設定する。
fn create_own_window(form: Form) {
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
            Form::ChromeBar | Form::ChromePage | Form::BugReport | Form::Ext => {
                unreachable!("別プロセスの入力先は target::launch が扱う")
            }
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
    }
}

fn own_read_text(h: HWND) -> String {
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

fn own_clear_text(h: HWND) {
    unsafe {
        let empty = wide("");
        let _ = SendMessageW(h, WM_SETTEXT, None, Some(LPARAM(empty.as_ptr() as isize)));
    }
}

/// 前面窓が `top`、かつそのスレッドのフォーカスが入力欄にあるか(自前の窓用)。
fn own_focus_ok() -> bool {
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

/// タスクバーを前面にして、テスト窓からフォーカスを外す(`--refocus`。フォーカス変更イベントを awase に見せる)。
/// 前面スレッドへ入力をアタッチする定番の回避策を使う。ワーカースレッドから呼ぶ。
fn sleep_ms_away() {
    std::thread::sleep(std::time::Duration::from_millis(200));
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

/// Chrome/自前窓以外の別トップレベル窓(別スレッドの可視窓)。CI ではタスクバーへの `SetForegroundWindow` が拒否される(`focus_away` が
/// `away=false`)ので、`--refocus` はこちらへ先に移す。chrome_probe.rs の同名関数と同じ作り。作成済みなら使い回す。
fn helper_window() -> Option<HWND> {
    static H: OnceLock<isize> = OnceLock::new();
    let raw = *H.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<isize>();
        std::thread::spawn(move || unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("TYPINGSTRESS_AWAY"),
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
            while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&raw const msg);
                DispatchMessageW(&raw const msg);
            }
        });
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or(0)
    });
    (raw != 0).then(|| HWND(raw as *mut _))
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
            sleep_ms(200);
            if GetForegroundWindow() == hwnd {
                return true;
            }
        }
    }
    false
}

fn own_refocus() {
    unsafe {
        let _ = PostMessageW(Some(hwnd_of(&TOP)), WM_TS_FRONT, WPARAM(0), LPARAM(0));
    }
    sleep_ms(400);
}

// 以降のシナリオは入力先の種類を知らず、これらの薄いラッパー越しに `InputTarget` を使う。

static TARGET: OnceLock<Box<dyn target::InputTarget>> = OnceLock::new();

fn target() -> &'static dyn target::InputTarget {
    TARGET.get().expect("入力先は main で起動済み").as_ref()
}

/// `_h` は自前の窓の HWND を渡していた従来の呼び出し形を保つための引数(入力先が決めるので使わない)。
fn read_text(_h: HWND) -> String {
    target().read()
}

fn clear_text(_h: HWND) {
    target().clear();
}

fn focus_ok() -> bool {
    target().focus_ok()
}

fn refocus() {
    target().refocus();
}

/// 別プロセスの窓を前面化する(前面スレッドへ `AttachThreadInput`。入力欄への `SetFocus` は行わない)。
fn raise_foreign(top: HWND) {
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
        if attached {
            let _ = AttachThreadInput(my_tid, fg_tid, false);
        }
    }
}

/// `--switch-focus` で入力先から前面を奪う無関係な窓。メッセージループのあるメインスレッドで作る。
static DISTRACTOR: AtomicIsize = AtomicIsize::new(0);

fn create_distractor() {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module");
        let cls = wide("TypingStressDistractor");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(top_proc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(cls.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&raw const wc);
        if let Ok(h) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(cls.as_ptr()),
            w!("typing stress distractor"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            300,
            120,
            None,
            None,
            Some(instance.into()),
            None,
        ) {
            DISTRACTOR.store(h.0 as isize, Ordering::SeqCst);
        }
    }
}

/// 別窓へ前面を渡す。別窓が無ければ何もせず `false`。
fn front_distractor() -> bool {
    let h = hwnd_of(&DISTRACTOR);
    if h.0.is_null() {
        return false;
    }
    raise_foreign(h);
    true
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

/// settle-explicit と同じ「修飾キーを先に押し、対象キーを離してから修飾キーを離す」順序。
fn press_ctrl_muhenkan() {
    send_key(VK_LCONTROL, SCAN_LCONTROL, true);
    sleep_ms(40);
    send_key(VK_MUHENKAN, SCAN_MUHENKAN, true);
    sleep_ms(60);
    send_key(VK_MUHENKAN, SCAN_MUHENKAN, false);
    sleep_ms(30);
    send_key(VK_LCONTROL, SCAN_LCONTROL, false);
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

fn wait_for_awase_startup() {
    for _ in 0..200 {
        if std::fs::metadata("awase.log").is_ok_and(|m| m.len() > 0) {
            log("[init] awase.log を確認(startup; 安定待ちなし)");
            return;
        }
        sleep_ms(25);
    }
    rec(&json!({"type":"abort","reason":"startup: awase.log を5秒以内に確認できない"}));
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

/// 既定 IME 窓へ `WM_IME_CONTROL(IMC_GETOPENSTATUS)` を送って IME の開閉を読む。別プロセスの入力先(実 Chrome)でも読める
/// (`real_ime_open` は自プロセスの HIMC しか取れず Chrome では `None`)。`child` の既定 IME 窓が無ければ前面窓で試す。
/// 取れない/応答が無いときは `None`。
fn imc_open_status(child: HWND) -> Option<bool> {
    const WM_IME_CONTROL: u32 = 0x0283;
    const IMC_GETOPENSTATUS: usize = 0x0005;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    // SAFETY: 既定 IME ウィンドウへ短いタイムアウト付きで同期送信するだけ。
    unsafe {
        let mut ime_wnd = windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd(child);
        if ime_wnd.0.is_null() {
            ime_wnd = windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd(GetForegroundWindow());
        }
        if ime_wnd.0.is_null() {
            return None;
        }
        let mut result = 0usize;
        let r = SendMessageTimeoutW(
            ime_wnd,
            WM_IME_CONTROL,
            WPARAM(IMC_GETOPENSTATUS),
            LPARAM(0),
            windows::Win32::UI::WindowsAndMessaging::SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG),
            500,
            Some(&raw mut result),
        );
        if r.0 == 0 {
            return None;
        }
        Some(result != 0)
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
        // `None`(取れない)は通す: tsx-chrome* は入力欄が別プロセス(Chrome)で HIMC を取れないため。
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

/// `--mode=drift`(ADR-191 09-T4、BUG-020 の回帰観測)。`ime_ready` で IME/awase の準備を確認した後に呼ぶ。
/// 1試行: IME を ON にそろえる → `off_vk` を単発で押す(awase 側は `keys.ime_off` をこの単一キーに
/// 上書きした設定で起動していること)→ +100/+400/+1500ms で `real_ime_open` を記録する。
fn drift_scenario(child: HWND) {
    let off_vk = arg_value("--drift-off-vk=")
        .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        .unwrap_or(VK_MUHENKAN);
    // 無変換/変換は物理位置(scancode)で分類されるため、既定候補は対応する scan を使う(BUG-131/132 と同型の
    // scan/vk不一致を避ける)。それ以外の VK を指定した場合は scan=0(専用 VK コードは scan を見ない前提)。
    let off_scan = match off_vk {
        VK_MUHENKAN => SCAN_MUHENKAN,
        VK_HENKAN => SCAN_HENKAN,
        _ => 0,
    };
    let trials: usize = arg_value("--trials=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    const CHECKPOINTS_MS: [u64; 3] = [100, 400, 1500];
    for n in 0..trials {
        if !focus_ok() {
            refocus();
        }
        if !focus_ok() {
            rec(&json!({"type":"abort","reason":format!("drift試行前にフォーカスが外れた n={n}")}));
            return;
        }
        turn_ime_on(n);
        let pre_open = real_ime_open(child);
        rec(
            &json!({"type":"drift_pre","n":n,"off_vk":format!("0x{off_vk:02X}"),"real_ime_open":pre_open}),
        );
        press(off_vk, off_scan, 50);
        let mut waited_ms = 0u64;
        for &cp in &CHECKPOINTS_MS {
            sleep_ms(cp - waited_ms);
            waited_ms = cp;
            let open = real_ime_open(child);
            rec(&json!({"type":"drift_check","n":n,"checkpoint_ms":cp,"real_ime_open":open}));
        }
    }
}

/// ハーネス自身の入力欄の IME を、awase を経由せず直接閉じる(外部要因による「ずれ」の再現)。
/// `ImmSetOpenStatus` は HIMC を持つスレッド以外から呼ぶと失敗する(run 36508003461 で `set_ok=false`)ため、
/// 既定 IME ウィンドウへ `WM_IME_CONTROL(IMC_SETOPENSTATUS, 0)` を送る。戻り値は `SendMessage` の戻り値
/// (0=成功)。既定 IME ウィンドウが取れなければ `None`。
fn force_close_real_ime(child: HWND) -> Option<isize> {
    const WM_IME_CONTROL: u32 = 0x0283;
    const IMC_SETOPENSTATUS: usize = 0x0006;
    // SAFETY: 自プロセスの入力欄に対応する既定 IME ウィンドウへ同期 SendMessage するだけ。
    unsafe {
        let ime_wnd = windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd(child);
        if ime_wnd.0.is_null() {
            return None;
        }
        Some(
            SendMessageW(
                ime_wnd,
                WM_IME_CONTROL,
                Some(WPARAM(IMC_SETOPENSTATUS)),
                Some(LPARAM(0)),
            )
            .0,
        )
    }
}

/// `--mode=drift-on`(ADR-178 領域A撤去後の ON 回復の観測)。`ime_ready` の後に呼ぶ。
/// 1試行: IME を ON にそろえる → 実 IME を直接閉じる → +500/+1500/+3000ms で `real_ime_open` を記録 →
/// かな単打を1回打って確定し、入力欄のテキストを記録する(IME が閉じたままなら生ローマ字になる)。
fn drift_on_scenario(child: HWND, cells: &[Vec<Cell>; 3]) {
    let trials: usize = arg_value("--trials=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    const CHECKPOINTS_MS: [u64; 2] = [500, 2000];
    let Some(probe) = cells[0]
        .iter()
        .find(|c| c.romaji == "ka")
        .cloned()
        .or_else(|| cells[0].first().cloned())
    else {
        rec(&json!({"type":"abort","reason":"drift-on の打鍵確認に使う単打セルが無い"}));
        return;
    };
    for n in 0..trials {
        if !focus_ok() {
            refocus();
        }
        if !focus_ok() {
            rec(
                &json!({"type":"abort","reason":format!("drift-on試行前にフォーカスが外れた n={n}")}),
            );
            return;
        }
        // 先頭は毎回 VK_IME_ON(awase の明示意図 ON になるキー)。ON にならなければ最大 3 回そろえ直す(k=1 は
        // `--msime` でなければ 0xF2、`--msime` なら VK_IME_ON の再試行になる)。VK_IME_ON 以外で ON になった試行は
        // 明示意図が消えうるので、checker が on_key!=0x16 を invalid にする。
        let mut on_key = VK_IME_ON;
        let mut on_utc = utc_hms();
        for k in 0..3 {
            on_utc = utc_hms();
            on_key = if k == 0 { VK_IME_ON } else { ime_on_key(k) };
            press(VK_IME_OFF, 0x70, 50);
            sleep_ms(600);
            press(on_key, 0x70, 50);
            sleep_ms(1500);
            if real_ime_open(child) != Some(false) {
                break;
            }
        }
        rec(
            &json!({"type":"drift_on_pre","n":n,"utc":utc_hms(),"on_utc":on_utc,"on_key":format!("0x{on_key:02X}"),
            "real_ime_open":real_ime_open(child)}),
        );
        // 窓の起点は閉じる操作の直前に取る(閉じた直後の観測が窓から漏れないように)。
        let close_utc = utc_hms();
        let ctrl_muhenkan = has_flag("--drift-off-ctrl-muhenkan");
        let set_ret = if ctrl_muhenkan {
            press_ctrl_muhenkan();
            None
        } else {
            force_close_real_ime(child)
        };
        sleep_ms(50);
        rec(
            &json!({"type":"drift_on_close","n":n,"utc":close_utc,"set_ret":set_ret,
                "method":if ctrl_muhenkan {"ctrl_muhenkan"} else {"direct_close"},
                "real_ime_open":real_ime_open(child)}),
        );
        // `--refocus`: 閉じた直後にフォーカスを一度外して戻す(awase のフォーカス変更経路=drift correction 再開の契機を通す)。
        if has_flag("--refocus") {
            let away_ok = focus_away_to_helper() || focus_away();
            sleep_ms(300);
            refocus();
            rec(
                &json!({"type":"drift_on_refocus","n":n,"utc":utc_hms(),"away_ok":away_ok,"on_target":focus_ok(),"real_ime_open":real_ime_open(child)}),
            );
        }
        let mut waited_ms = 0u64;
        for &cp in &CHECKPOINTS_MS {
            sleep_ms(cp - waited_ms);
            waited_ms = cp;
            rec(
                &json!({"type":"drift_on_check","n":n,"checkpoint_ms":cp,"real_ime_open":real_ime_open(child)}),
            );
        }
        let focus_lost = !focus_ok();
        let press_utc = utc_hms();
        clear_text(child);
        sleep_ms(200);
        press(probe.vk, probe.scan, 60);
        sleep_ms(700);
        press(VK_RETURN, 0x1C, 50);
        sleep_ms(700);
        let text = read_text(child);
        rec(
            &json!({"type":"drift_on_typed","n":n,"utc":utc_hms(),"press_utc":press_utc,"focus_lost":focus_lost,"text":text,"expect":probe.kana.to_string(),
            "ok":text.trim() == probe.kana.to_string(),"real_ime_open":real_ime_open(child)}),
        );
        clear_text(child);
    }
}

/// `probe`(NICOLA 単打セル)が生キーで出す1文字(小文字)。awase が Engine を止めて生キーが通ると、この文字がそのまま出る。
fn raw_char_of(probe: &Cell) -> String {
    u8::try_from(probe.vk)
        .map(|b| char::from(b).to_ascii_lowercase().to_string())
        .unwrap_or_default()
}

/// 起動シナリオの1打鍵。awase がいる(通常)ときは NICOLA 単打(`probe.vk`)、`--no-awase` の対照では生のローマ字 `k`,`a`。
fn press_probe(probe: &Cell, no_awase: bool) {
    if no_awase {
        for ch in probe.romaji.bytes() {
            press(u32::from(ch.to_ascii_uppercase()), 0, 60);
            sleep_ms(40);
        }
    } else {
        press(probe.vk, probe.scan, 60);
    }
}

fn startup_scenario(child: HWND, cells: &[Vec<Cell>; 3], initial_on: bool, no_awase: bool) {
    let Some(probe) = cells[0].iter().find(|c| c.romaji == "ka").cloned() else {
        rec(&json!({"type":"abort","reason":"startup の打鍵確認に使う ka セルが無い"}));
        return;
    };
    let detected_utc = utc_hms();
    start_hook_thread();
    if initial_on {
        clear_text(child);
        let open_before_type = imc_open_status(child);
        let press_utc = utc_hms();
        press_probe(&probe, no_awase);
        sleep_ms(700);
        press(VK_RETURN, 0x1C, 50);
        sleep_ms(900);
        let text = read_text_maybe_settled(child, true);
        rec(
            &json!({"type":"startup_typed","initial":"on","detected_utc":detected_utc,
            "press_utc":press_utc,"text":text,"expect":probe.kana.to_string(),
            "ok":text.trim()==probe.kana.to_string(),"real_ime_open":real_ime_open(child),
            "real_ime_open_before_type":open_before_type,"raw_char":raw_char_of(&probe),"no_awase":no_awase}),
        );
    } else {
        sleep_ms(3000);
        let open_after_idle = real_ime_open(child);
        let before_on_utc = utc_hms();
        press(VK_IME_ON, 0x70, 50);
        sleep_ms(800);
        clear_text(child);
        let open_before_type = imc_open_status(child);
        let press_utc = utc_hms();
        press_probe(&probe, no_awase);
        sleep_ms(700);
        press(VK_RETURN, 0x1C, 50);
        sleep_ms(900);
        let text = read_text_maybe_settled(child, true);
        rec(
            &json!({"type":"startup_typed","initial":"off","detected_utc":detected_utc,
            "before_on_utc":before_on_utc,"press_utc":press_utc,"open_after_idle":open_after_idle,
            "text":text,"expect":probe.kana.to_string(),"ok":text.trim()==probe.kana.to_string(),
            "real_ime_open":real_ime_open(child),"real_ime_open_before_type":open_before_type,
            "raw_char":raw_char_of(&probe),"no_awase":no_awase}),
        );
    }
}

/// `--mode=reopen` の物理キー注入で使う scan。無変換/変換は物理位置(scancode)で分類されるので対応する scan を使う(BUG-131/132、
/// `drift_scenario` と同じ)。ひらがな等の IME 専用 VK は scan を見ないので従来どおり 0x70。
fn scan_for_key(vk: u32) -> u16 {
    match vk {
        VK_MUHENKAN => SCAN_MUHENKAN,
        VK_HENKAN => SCAN_HENKAN,
        _ => 0x70,
    }
}

/// `--settle-read` 付きのとき、内容が 800ms 変わらなくなるまで(最大 8 秒)読み直す(Chrome の描画遅れで欠落と誤判定しない)。
fn read_text_maybe_settled(child: HWND, settle: bool) -> String {
    let mut actual = read_text(child);
    if settle {
        let t0 = Instant::now();
        let mut stable_since = Instant::now();
        while t0.elapsed() < Duration::from_secs(8)
            && stable_since.elapsed() < Duration::from_millis(800)
        {
            sleep_ms(200);
            let now = read_text(child);
            if now != actual {
                actual = now;
                stable_since = Instant::now();
            }
        }
    }
    actual
}

/// `--mode=reopen`(ADR-203 の e2e (c)、BUG-170 の実機確認): 「OFF 前に 1 語確定 → 物理 OFF → 1 秒以内に物理 ON → 即打鍵」。
/// `ime_ready` の後に呼ぶ。GjiFsm が OffCold に固着せず、ON 後の最初の語が欠けない/リテラル化しないかを、入力先のテキストと
/// awase.log(checker が `[vk-send]`・`[gji-fsm]` を数える)の両方で見る。
/// 1試行: IME を ON にそろえる → かな単打を1語打って Enter で確定(`reopen_pre`) → `VK_IME_OFF` を押す →
/// `--reopen-gap=MS`(既定 600、1000 未満)待つ → ON キー(`--reopen-on-key=0xNN`、既定は `ime_on_key(0)`=GJI は 0x16・MS-IME は 0xF2)を押す(`reopen_on`) →
/// `--reopen-type-delay=MS`(既定 0=即)後に同じかなを打って確定(`reopen_typed`)。
/// 記録の `utc` は awase.log の時刻(HH:MM:SS.mmm)と突合せる用。ON キー押下から最初の `[vk-send]` までの遅延は checker が出す。
fn reopen_scenario(child: HWND, cells: &[Vec<Cell>; 3]) {
    let trials: usize = arg_value("--trials=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let gap_ms: u64 = arg_value("--reopen-gap=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let type_delay_ms: u64 = arg_value("--reopen-type-delay=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    // 書き間違いを黙って既定キーに戻さない(構成が意図と違うキーで走ったのに PASS する事故を防ぐ)。
    let on_key: u32 = match arg_value("--reopen-on-key=") {
        None => ime_on_key(0),
        Some(v) => {
            match u32::from_str_radix(v.trim_start_matches("0x").trim_start_matches("0X"), 16) {
                Ok(k) => k,
                Err(_) => {
                    rec(
                        &json!({"type":"abort","reason":format!("--reopen-on-key={v} を16進数として読めない")}),
                    );
                    return;
                }
            }
        }
    };
    // ADR-203 (c) は「1秒以内に物理 ON」。1秒以上空けると別のシナリオ(idle 後)になる。
    if gap_ms >= 1000 {
        rec(
            &json!({"type":"abort","reason":format!("--reopen-gap={gap_ms} は 1000 未満にする(ADR-203 (c) は1秒以内)")}),
        );
        return;
    }
    let settle = has_flag("--settle-read");
    let Some(probe) = cells[0]
        .iter()
        .find(|c| c.romaji == "ka")
        .cloned()
        .or_else(|| cells[0].first().cloned())
    else {
        rec(&json!({"type":"abort","reason":"reopen の打鍵確認に使う単打セルが無い"}));
        return;
    };
    let expect = probe.kana.to_string();
    for n in 0..trials {
        if !focus_ok() {
            refocus();
        }
        if !focus_ok() {
            rec(
                &json!({"type":"abort","reason":format!("reopen試行前にフォーカスが外れた n={n}")}),
            );
            return;
        }
        // OFF 前の 1 語: ON にそろえてから打って確定する(BUG-170 の「OFF 前に語を打っている」条件)。
        // この turn_ime_on(OFF→ON)自体も、前の試行の確定語の後の「開き直し」になる。checker は trial_utc〜off_utc の固着も数える(M5)。
        let trial_utc = utc_hms();
        turn_ime_on(0);
        clear_text(child);
        sleep_ms(200);
        press(probe.vk, probe.scan, 60);
        sleep_ms(500);
        press(VK_RETURN, 0x1C, 50);
        sleep_ms(500);
        let pre_text = read_text_maybe_settled(child, settle);
        rec(
            &json!({"type":"reopen_pre","n":n,"trial_utc":trial_utc,"utc":utc_hms(),"text":pre_text,"expect":expect,
            "ok":pre_text.trim() == expect,"real_ime_open":real_ime_open(child)}),
        );
        clear_text(child);
        sleep_ms(200);
        // 物理 OFF → gap → 物理 ON。
        let off_utc = utc_hms();
        press(VK_IME_OFF, 0x70, 50);
        sleep_ms(gap_ms);
        // 物理 OFF が効いたか(ON キーを押す直前の実 IME。別プロセスの入力先では None)。効いていないと無意味な試行になる(M3)。
        let open_before_on = real_ime_open(child);
        let on_utc = utc_hms();
        press(on_key, scan_for_key(on_key), 50);
        rec(
            &json!({"type":"reopen_on","n":n,"off_utc":off_utc,"on_utc":on_utc,"gap_ms":gap_ms,
            "on_key":format!("0x{on_key:02X}"),"type_delay_ms":type_delay_ms,"open_before_on":open_before_on}),
        );
        sleep_ms(type_delay_ms);
        let focus_lost = !focus_ok();
        let press_utc = utc_hms();
        press(probe.vk, probe.scan, 60);
        sleep_ms(700);
        press(VK_RETURN, 0x1C, 50);
        sleep_ms(700);
        let text = read_text_maybe_settled(child, settle);
        rec(
            &json!({"type":"reopen_typed","n":n,"utc":utc_hms(),"on_utc":on_utc,"press_utc":press_utc,"focus_lost":focus_lost,
            "text":text,"expect":expect,"ok":text.trim() == expect,"real_ime_open":real_ime_open(child)}),
        );
        clear_text(child);
    }
}

fn worker(form: Form) {
    let child = hwnd_of(&CHILD);
    let perturb = perturb::Perturbation::from_args();
    let suspend = suspend::Suspend::from_args();
    let jit = jitter::Jitter::from_args();
    let mode_arg = arg_value("--mode=");
    let raw = mode_arg.as_deref() == Some("raw");
    let drift = mode_arg.as_deref() == Some("drift");
    let drift_on = mode_arg.as_deref() == Some("drift-on");
    let reopen = mode_arg.as_deref() == Some("reopen");
    let preedit = mode_arg.as_deref() == Some("preedit");
    let keymatrix = mode_arg.as_deref() == Some("keymatrix");
    let focus_restore = mode_arg.as_deref() == Some("focus-restore");
    let startup = mode_arg.as_deref() == Some("startup");
    let startup_on = arg_value("--startup-ime=").as_deref() == Some("on");
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
        &json!({"type":"config","form":form.name(),"ime":ime,"mode":if startup {"startup"} else if drift {"drift"} else if drift_on {"drift-on"} else if keymatrix {"keymatrix"} else if focus_restore {"focus-restore"} else if reopen {"reopen"} else if preedit {"preedit"} else if raw {"raw"} else {"nicola"},
        "interval_ms":iv_ms,"len":len,"trials":trials,"seed":seed,"kinds":kinds,
        "no_awase":has_flag("--no-awase"),"startup_skip_refocus2":has_flag("--startup-skip-refocus2"),
        "layout":layout_path,"cells":[cells[0].len(),cells[1].len(),cells[2].len()],
        "child_class":class_of(child),"perturb":perturb.describe(),"jitter":jit.describe(),"suspend":suspend.as_ref().map(suspend::Suspend::describe)}),
    );
    if let Some(sp) = &suspend {
        sp.log_candidates();
    }
    if cells.iter().any(Vec::is_empty) {
        rec(&json!({"type":"abort","reason":"候補セルが空(layout の読み取り失敗?)"}));
        finish();
        return;
    }

    sleep_ms(500);
    refocus();
    if startup {
        press(if startup_on { VK_IME_ON } else { VK_IME_OFF }, 0x70, 50);
        sleep_ms(800);
        rec(
            &json!({"type":"startup_pre","initial":if startup_on {"on"} else {"off"},
            "utc":utc_hms(),"real_ime_open":real_ime_open(child)}),
        );
    }
    log("[TS] READY-FOR-AWASE");
    if !has_flag("--no-awase") {
        if startup {
            wait_for_awase_startup();
        } else {
            wait_for_awase();
        }
    }
    if startup && has_flag("--no-awase") {
        // awase なしの対照: 通常は awase の起動を待つ 1〜5 秒の間 IME が置かれるので、同程度の間を置いて打鍵位置を揃える。
        sleep_ms(1500);
    }
    if !(startup && has_flag("--startup-skip-refocus2")) {
        refocus();
    }
    rec(&focus_report());
    if !focus_ok() {
        rec(
            &json!({"type":"abort","reason":"前面化またはフォーカスに失敗したためキーを注入しない"}),
        );
        finish();
        return;
    }
    if startup {
        startup_scenario(child, &cells, startup_on, has_flag("--no-awase"));
        finish();
        return;
    }
    start_hook_thread();
    sleep_ms(500);

    // IME を ON にそろえる(OFF → ひらがな)。
    turn_ime_on(0);
    // --cold: 準備確認(「か」を打って確認・最大3回リトライ)を省く。この確認自体が窓への最初の実際の
    // 確定入力になり、「起動直後にユーザーが最初に打つ文字」を素通りさせてしまうため。
    if perturb.cold {
        rec(&json!({"type":"ready","attempt":0,"skipped":true,"reason":"--cold"}));
    } else if !ime_ready(raw, &cells, child) {
        rec(&json!({"type":"abort","reason":"IME/awase の準備確認に失敗(ready の text を参照)"}));
        finish();
        return;
    }

    if drift {
        drift_scenario(child);
        finish();
        return;
    }
    if drift_on {
        drift_on_scenario(child, &cells);
        finish();
        return;
    }
    if keymatrix {
        keymatrix::keymatrix_scenario(child, &cells);
        finish();
        return;
    }
    if focus_restore {
        focusrestore::focus_restore_scenario(child, &cells);
        finish();
        return;
    }
    if reopen {
        reopen_scenario(child, &cells);
        finish();
        return;
    }
    if preedit {
        preedit::preedit_scenario(child, &cells);
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
            let mut evs = if raw {
                raw_events(&seq, iv_us)
            } else if jit.is_active() {
                jitter::events(&seq, iv_us, &jit, trial_seed)
            } else {
                nicola_events(&seq, iv_us)
            };
            perturb.apply_pause(&mut evs, iv_us);
            perturb.before_trial(target());
            clear_text(child);
            sleep_ms(perturb.start_delay_ms);
            if let Ok(mut g) = HOOK_EVENTS.lock() {
                g.clear();
            }
            let suspender = suspend.as_ref().map(|sp| sp.arm(kind, t));
            let stats = run_schedule(&evs);
            if let Some(h) = suspender {
                let _ = h.join();
            }
            perturb.after_inject(kind, t);
            // 最後の同時打鍵判定・出力の落ち着きを待ってから確定(Enter)。
            sleep_ms(300);
            // 注入したキーだけのフック到着を、確定キー(Enter)を打つ前に確定させる。
            let hook: Vec<HookEv> = HOOK_EVENTS.lock().map(|g| g.clone()).unwrap_or_default();
            sleep_ms(600);
            press(VK_RETURN, 0x1C, 50);
            sleep_ms(1200);
            let actual_at_1200 = read_text(child);
            let mut actual = actual_at_1200.clone();
            // --settle-read を付けなかったときは、読み直していない(0 ではなく null で記録する)。
            let mut settle_ms: Option<u64> = None;
            if perturb.settle_read {
                // 取りこぼしか遅延かを分けるため、内容が 800ms 変わらなくなるまで(最大 8 秒)読み直す。
                let t0 = Instant::now();
                let mut stable_since = Instant::now();
                while t0.elapsed() < Duration::from_secs(8)
                    && stable_since.elapsed() < Duration::from_millis(800)
                {
                    sleep_ms(200);
                    let now = read_text(child);
                    if now != actual {
                        actual = now;
                        stable_since = Instant::now();
                    }
                }
                settle_ms = Some(u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX));
            }
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
                "actual":actual,"actual_at_1200ms":actual_at_1200,"settle_ms":settle_ms,
                "keys":seq_desc.join(" "),"focus_ok":focus_ok()}),
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
    target().shutdown();
    if !target().owns_himc() {
        // 別プロセスの入力先では自前のメッセージループが閉じ窓で終わらないので、ここで終える。
        std::process::exit(0);
    }
}

fn main() {
    // focus-restore の窓 B 用の別プロセス(ログも入力先も作らない)。
    if has_flag("--fr-helper") {
        focusrestore::helper_main();
        return;
    }
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
    // drift 系は自プロセスの窓の HIMC を直接観測/操作するので、別プロセスの入力先とは組み合わせられない。
    // 入力先(Chrome など)を起動する前に弾く。
    if matches!(
        arg_value("--mode=").as_deref(),
        Some("drift" | "drift-on" | "keymatrix" | "focus-restore")
    ) && !matches!(form, Form::Edit | Form::Multi | Form::Rich | Form::Tsf)
    {
        log("[FATAL] 引数エラー: --mode=drift|drift-on|keymatrix|focus-restore は --form=edit|multi|rich|tsf でのみ使える");
        std::process::exit(2);
    }
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
    // 引数の誤り(--interrupt の値など)は、入力先(Chrome など)を起動する前に検出する。
    let perturbation = perturb::Perturbation::from_args();
    let _ = TARGET.set(target::launch(form));
    if perturbation.needs_distractor() {
        create_distractor();
    }
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
