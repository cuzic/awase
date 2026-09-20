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
//! 3. 画面中段の案内に従って、状態を作り、指定されたキーを 1 回だけ押す。
//!    押した後は 3 秒待つ（自動で次のステップを案内する）。
//!    全 2 ラウンド（標準 EDIT / RichEdit 5.0）× 20 ステップ
//!    （4 状態 × 5 キー）。`--round2` で RichEdit から開始。そのキーが無い場合は
//!    Ctrl+Shift+F12 でスキップ。
//! 4. 各キー押下ごとに 1 件がログ欄と `ime_key_matrix_spike.log`（exe と同じ
//!    ディレクトリ）に追記される。`[STEP ...]` タグ付きが案内どおりの測定、
//!    `[準備/その他]` は状態を作るための押下。

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
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetContext, ImmGetConversionStatus, ImmGetDefaultIMEWnd,
    ImmGetOpenStatus, ImmReleaseContext, IME_COMPOSITION_STRING, IME_CONVERSION_MODE,
    IME_SENTENCE_MODE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetFocus, SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, CLSID_TF_ThreadMgr, ITfCompartmentMgr,
    ITfInputProcessorProfileMgr, ITfThreadMgr, GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
    GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, GUID_TFCAT_TIP_KEYBOARD,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW,
    GetForegroundWindow, GetMessageW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, KillTimer, MessageBoxW, PostMessageW, PostQuitMessage,
    RegisterClassW, SendMessageTimeoutW, SendMessageW, SetForegroundWindow, SetTimer,
    SetWindowTextW, SetWindowsHookExW, ShowWindow, TranslateMessage, CW_USEDEFAULT,
    KBDLLHOOKSTRUCT, MB_ICONERROR, MB_OK, MSG, SMTO_ABORTIFHUNG, SW_SHOW, WH_KEYBOARD_LL,
    WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_SETFOCUS, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    WS_VSCROLL,
};

/// `--auto` が注入するキーの dwExtraInfo（自分の注入を、他の注入と区別してステップ照合に使う）。
const AUTO_MARKER: usize = awase_windows::hook::TEST_INJECTION_MARKER;

const WM_IME_CONTROL: u32 = 0x0283;
const IMC_GETCONVERSIONMODE: usize = 0x0001;
const IMC_GETOPENSTATUS: usize = 0x0005;
const GCS_COMPSTR: u32 = 0x0008;

const TIMER_ID: usize = 1;
/// 観測スナップショットを更新する周期。
const TIMER_INTERVAL_MS: u32 = 50;
/// 押下後に取る 2 回のスナップショットまでの遅延。
const AFTER_MS_FULL: [u64; 3] = [100, 400, 1500];
/// `--fast` 用: 判定に使わない +1500ms の観測を省く。
const AFTER_MS_FAST: [u64; 2] = [100, 400];

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

    /// マトリクスの状態軸（開閉は A/B 一致、かな/英数は conv の NATIVE ビット）。
    fn st(&self) -> St {
        let open = match (self.a_open, self.b_open) {
            (Some(a), Some(b)) if a == b => Some(a),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            _ => None,
        };
        let composing = self.comp.as_deref().is_some_and(|s| !s.is_empty());
        let conv = self.b_conv.or(self.a_conv);
        match open {
            None => St::Unknown,
            Some(false) => St::Direct,
            Some(true) if composing => St::OnKanaComp,
            Some(true) => match conv {
                Some(c) if c & 1 == 0 => St::OnAlnum,
                _ => St::OnKana,
            },
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
    static KEY_QUEUE: RefCell<Vec<KeyEvt>> = const { RefCell::new(Vec::new()) };
    static RICH_HWND: RefCell<Option<HWND>> = const { RefCell::new(None) };
    /// 全ステップ通しの現在位置（0 始まり。ラウンド = idx / steps().len()）。
    static STEP_IDX: RefCell<usize> = const { RefCell::new(0) };
    /// この時刻（now_ms）までは次のステップを案内しない（押下の効果が落ち着くまで待つ）。
    static HOLD_UNTIL: RefCell<u64> = const { RefCell::new(0) };
    /// Ctrl+Shift+F12 によるスキップ要求。
    static SKIP_REQ: RefCell<bool> = const { RefCell::new(false) };
    static LAST_ROUND: RefCell<Option<usize>> = const { RefCell::new(None) };
    /// `--auto`: 手順のキーをスパイク自身が SendInput で注入する（RPA的な自動実行）。
    static AUTO_MODE: RefCell<bool> = const { RefCell::new(false) };
    /// (実行時刻ms, VK, KeyDownか) の注入予約。
    static AUTO_QUEUE: RefCell<Vec<(u64, u32, bool)>> = const { RefCell::new(Vec::new()) };
    static AUTO_NEXT: RefCell<u64> = const { RefCell::new(0) };
    static AUTO_LAST_SI: RefCell<usize> = const { RefCell::new(usize::MAX) };
    static AUTO_TRIES: RefCell<usize> = const { RefCell::new(0) };
    static AUTO_PREP: RefCell<usize> = const { RefCell::new(0) };
    static AUTO_DONE: RefCell<bool> = const { RefCell::new(false) };
    /// `--repeat=N`: 全手順をこのプロセス内でN回繰り返す(起動・終了・ログ取得の往復を省く)。
    static REPEAT_N: RefCell<usize> = const { RefCell::new(1) };
    static REPEAT_DONE: RefCell<usize> = const { RefCell::new(0) };
    /// `--shiftmuh`: 手順の「無変換」押下を Shift+無変換 にする(ADR-186 残る問題2の観測用)。
    static SHIFT_MUH: RefCell<bool> = const { RefCell::new(false) };
    /// `--fast`: +1500ms の観測を省く。
    static FAST_MODE: RefCell<bool> = const { RefCell::new(false) };
    /// `--speed=K`: 手順間の待ち時間をK倍速にする(既定1=従来どおり)。
    static SPEED: RefCell<u64> = const { RefCell::new(1) };
    /// `--hold=NNN`: 注入キーの保持時間ms(既定80)。人の押下(>100ms)でだけ通るタイマー経路を再現する。
    static HOLD_MS_INJ: RefCell<u64> = const { RefCell::new(80) };
    /// `--key=henkan`: 手順の「無変換」を「変換」(0x1C)に置き換える。
    static TOGGLE_VK: RefCell<u32> = const { RefCell::new(0x1D) };
    /// 全手順完了後、この時刻(now_ms)にウィンドウを閉じて終了する(0=予約なし)。
    static AUTO_CLOSE_AT: RefCell<u64> = const { RefCell::new(0) };
    /// `--script`: ADR-186 の実機A/B用の固定手順（awase 起動中に、押すキーと期待を順に案内）。
    static SCRIPT_MODE: RefCell<bool> = const { RefCell::new(false) };
    static SCRIPT_IDX: RefCell<usize> = const { RefCell::new(0) };
    /// `--free`: 案内なしの自由測定モード（awase を起動したまま実IME状態を記録する）。
    static FREE_MODE: RefCell<bool> = const { RefCell::new(false) };
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
    /// 押下中の VK（オートリピート抑止用）。
    static DOWN_KEYS: RefCell<std::collections::HashMap<u32, u64>> = RefCell::new(std::collections::HashMap::new());
    static START: RefCell<Option<std::time::Instant>> = const { RefCell::new(None) };
}

/// 押下後の観測時点(ms)。`--fast` なら +1500ms を省く。
fn after_ms() -> &'static [u64] {
    if FAST_MODE.with(|f| *f.borrow()) {
        &AFTER_MS_FAST
    } else {
        &AFTER_MS_FULL
    }
}

/// `--speed=K` で手順間の待ち時間を縮める。
fn scaled(ms: u64) -> u64 {
    ms / SPEED.with(|s| *s.borrow()).max(1)
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

/// 現在時刻(UTC)を `HH:MM:SS.mmmZ` で返す（awase のログ時刻と突き合わせるため）。
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

// ─── 案内付きステップ ────────────────────────────────────────────────────

/// フックがキューへ積むキー押下。
struct KeyEvt {
    label: String,
    vk: u32,
    ctrl: bool,
    shift: bool,
}

/// マトリクスの「状態」軸。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Direct,
    OnKana,
    OnKanaComp,
    OnAlnum,
    Unknown,
}

impl St {
    fn label(self) -> &'static str {
        match self {
            Self::Direct => "直接入力",
            Self::OnKana => "IME ON・かな・入力なし",
            Self::OnKanaComp => "IME ON・入力中(未確定あり)",
            Self::OnAlnum => "IME ON・半角英数・入力なし",
            Self::Unknown => "不明",
        }
    }
}

struct Step {
    state: St,
    key_name: &'static str,
    vks: &'static [u32],
    /// Shift 併用の押下も、このステップの押下として受け付ける（カタカナは Shift 併用でのみ届く）。
    allow_shift: bool,
}

/// 英数(0xF0)は ROUND1 でこの環境に物理キーが無いことが分かったため対象外。
const KEYS: [(&str, &[u32], bool); 5] = [
    ("無変換", &[0x1D], false),
    ("変換", &[0x1C], false),
    ("ひらがな", &[0xF2], false),
    (
        "Shift+ひらがな(カタカナ入力。0xF1として届く)",
        &[0xF1],
        true,
    ),
    ("半角/全角", &[0xF3, 0xF4], false),
];

const STATES: [St; 4] = [St::Direct, St::OnKana, St::OnKanaComp, St::OnAlnum];

/// 1 ラウンド分（状態4 × キー5 = 20 ステップ）。
fn steps() -> Vec<Step> {
    let mut v = Vec::new();
    for st in STATES {
        for (name, vks, allow_shift) in KEYS {
            v.push(Step {
                state: st,
                key_name: name,
                vks,
                allow_shift,
            });
        }
    }
    v
}

const ROUNDS: usize = 2;
const ROUND_NAMES: [&str; ROUNDS] = ["EDIT(標準コントロール)", "RichEdit 5.0(TSFネイティブ)"];
const HOLD_MS: u64 = 3000;

/// 物理キーに近いスキャンコードを付けて注入するための対応（JIS配列）。
fn scan_for(vk: u32) -> u16 {
    match vk {
        0x1D => 0x7B,
        0x1C => 0x79,
        0xF2 => 0x70,
        0x4B => 0x25,
        0x1B => 0x01,
        _ => 0,
    }
}

/// `SendInput` で1イベントを注入する（`AUTO_MARKER` 付き）。
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

/// `--key=henkan` のとき、手順の無変換(0x1D)を変換(0x1C)に置き換える。
fn script_vk(vk: u32) -> u32 {
    if vk == 0x1D {
        TOGGLE_VK.with(|t| *t.borrow())
    } else {
        vk
    }
}

/// 押して離す（`--hold` ms 保持）を、`at` を起点に予約する。
fn queue_press(at: u64, vk: u32) {
    AUTO_QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        q.push((at, vk, true));
        let hold = HOLD_MS_INJ.with(|h| *h.borrow());
        q.push((at + hold, vk, false));
    });
}

/// 準備の状態遷移に使うキー（`script_hint` と同じ方針）。
fn auto_hint_vk(cur: St, need: St) -> Option<u32> {
    match (cur, need) {
        (St::Unknown, _) => None,
        (St::OnKanaComp, _) => Some(0x1B),
        (St::Direct, _) | (St::OnKana | St::OnAlnum, St::Direct) => Some(0x1C),
        (St::OnAlnum, St::OnKana) | (St::OnKana, St::OnAlnum) => Some(0xF2),
        _ => None,
    }
}

/// `--auto` の1tick分の駆動。予約済みの注入を実行し、次の手順（または準備）を予約する。
fn auto_drive(now: u64, cur: St, hwnd: HWND) {
    let due: Vec<(u64, u32, bool)> = AUTO_QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        let all: Vec<_> = q.drain(..).collect();
        let (d, rest): (Vec<_>, Vec<_>) = all.into_iter().partition(|(t, _, _)| *t <= now);
        *q = rest;
        d
    });
    for (_, vk, down) in due {
        send_key(vk, down);
    }
    // 全手順完了の少し後に、自動でウィンドウを閉じる(ログはファイルへ保存済み)。
    let close_at = AUTO_CLOSE_AT.with(|c| *c.borrow());
    if close_at != 0 && now >= close_at {
        AUTO_CLOSE_AT.with(|c| *c.borrow_mut() = 0);
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        return;
    }
    if AUTO_QUEUE.with(|q| !q.borrow().is_empty())
        || now < HOLD_UNTIL.with(|h| *h.borrow())
        || now < AUTO_NEXT.with(|n| *n.borrow())
    {
        return;
    }
    // 注入は前面ウィンドウに届くので、スパイクを前面・入力欄フォーカスに保つ。
    unsafe {
        if GetForegroundWindow() != hwnd {
            // バックグラウンドから起動したプロセスは、素の SetForegroundWindow を Windows に拒否される
            // (フォアグラウンドロック)。前面スレッドへ入力をアタッチしてから前面化する定番の回避策。
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
            if attached {
                let _ = AttachThreadInput(my_tid, fg_tid, false);
            }
            if let Some(e) = EDIT_HWND.with(|e| *e.borrow()) {
                let _ = SetFocus(Some(e));
            }
            append_log("[AUTO] フォーカス復帰(前面ウィンドウ)");
            AUTO_NEXT.with(|n| *n.borrow_mut() = now + 600);
            return;
        }
    }
    // フォーカスが入力欄から外れる(ログ欄へ移る等)と実IME状態が読めず手順が進まなくなる(実機で発生)。
    // 前面ウィンドウだけでなく、入力欄へのフォーカスも毎回確認して戻す。
    if let Some(edit) = EDIT_HWND.with(|e| *e.borrow()) {
        unsafe {
            if GetFocus() != edit {
                let _ = SetFocus(Some(edit));
                append_log("[AUTO] フォーカス復帰(入力欄)");
                AUTO_NEXT.with(|n| *n.borrow_mut() = now + 300);
                return;
            }
        }
    }
    let si = SCRIPT_IDX.with(|i| *i.borrow());
    if si >= SCRIPT.len() {
        let done = REPEAT_DONE.with(|d| {
            *d.borrow_mut() += 1;
            *d.borrow()
        });
        let total = REPEAT_N.with(|n| *n.borrow());
        if done < total {
            append_log(&format!("[RUN {done}/{total} 完了]"));
            SCRIPT_IDX.with(|i| *i.borrow_mut() = 0);
            AUTO_LAST_SI.with(|l| *l.borrow_mut() = usize::MAX);
            append_log(&format!("[RUN {}/{total} 開始]", done + 1));
            AUTO_NEXT.with(|n| *n.borrow_mut() = now + scaled(1500));
            return;
        }
        if total > 1 {
            append_log(&format!("[RUN {done}/{total} 完了]"));
        }
        if !AUTO_DONE.with(|d| std::mem::replace(&mut *d.borrow_mut(), true)) {
            append_log("[AUTO] 全手順完了（1.5秒後に自動で閉じます）");
            AUTO_CLOSE_AT.with(|c| *c.borrow_mut() = now + 1500);
        }
        return;
    }
    if AUTO_LAST_SI.with(|l| std::mem::replace(&mut *l.borrow_mut(), si)) != si {
        AUTO_TRIES.with(|t| *t.borrow_mut() = 0);
        AUTO_PREP.with(|t| *t.borrow_mut() = 0);
    }
    let (name, vk, _, _, need) = SCRIPT[si];
    if cur != need {
        let prep = AUTO_PREP.with(|p| {
            *p.borrow_mut() += 1;
            *p.borrow()
        });
        match auto_hint_vk(cur, need) {
            Some(h) if prep <= 6 => {
                append_log(&format!(
                    "[AUTO] 準備 STEP {}: 現在={} 必要={} → VK 0x{h:02X} を注入",
                    si + 1,
                    cur.label(),
                    need.label()
                ));
                queue_press(now, h);
                AUTO_NEXT.with(|n| *n.borrow_mut() = now + scaled(2500));
            }
            _ => {
                append_log(&format!(
                    "[AUTO] STEP {} {name}: 前提状態にできずスキップ(現在={})",
                    si + 1,
                    cur.label()
                ));
                SCRIPT_IDX.with(|i| *i.borrow_mut() = si + 1);
            }
        }
        return;
    }
    let tries = AUTO_TRIES.with(|t| {
        *t.borrow_mut() += 1;
        *t.borrow()
    });
    if tries > 3 {
        append_log(&format!(
            "[AUTO] STEP {} {name}: 注入したがステップに一致せず(3回)→スキップ",
            si + 1
        ));
        SCRIPT_IDX.with(|i| *i.borrow_mut() = si + 1);
        return;
    }
    if SHIFT_MUH.with(|m| *m.borrow()) && script_vk(vk) == 0x1D {
        // Shift を先に押し、無変換を押して離し、その後 Shift を離す(LShift = 0xA0)。
        let hold = HOLD_MS_INJ.with(|h| *h.borrow());
        AUTO_QUEUE.with(|q| {
            let mut q = q.borrow_mut();
            q.push((now, 0xA0, true));
            q.push((now + 40, 0x1D, true));
            q.push((now + 40 + hold, 0x1D, false));
            q.push((now + 40 + hold + 40, 0xA0, false));
        });
    } else {
        queue_press(now, script_vk(vk));
    }
    queue_press(now + scaled(700), 0x4B); // k
    queue_press(now + scaled(1200), 0x1B); // ESC
    AUTO_NEXT.with(|n| *n.borrow_mut() = now + scaled(1800));
}

/// `--script` の1手順: (表示名, VK, Shift併用, 期待する結果)。
const SCRIPT: [(&str, u32, bool, &str, St); 10] = [
    (
        "ひらがなキー",
        0xF2,
        false,
        "ONのまま半角英数へ(conv 0x10)。Engine OFF(決定3保留のため遅延の可能性あり)",
        St::OnKana,
    ),
    (
        "無変換",
        0x1D,
        false,
        "IME OFF(直接入力)。Engine OFF",
        St::OnAlnum,
    ),
    (
        "無変換",
        0x1D,
        false,
        "IME ON・半角英数のまま(conv 0x10)。Engine は OFF のまま ← 決定2の核心",
        St::Direct,
    ),
    (
        "ひらがなキー",
        0xF2,
        false,
        "かなに戻る(conv 0x19)。Engine ON(遅延の可能性あり)",
        St::OnAlnum,
    ),
    ("無変換", 0x1D, false, "IME OFF。Engine OFF", St::OnKana),
    ("無変換", 0x1D, false, "IME ON(かな)。Engine ON", St::Direct),
    (
        "ひらがなキー",
        0xF2,
        false,
        "ONのまま半角英数へ。Engine は(遅延で)OFF",
        St::OnKana,
    ),
    ("無変換", 0x1D, false, "IME OFF。Engine OFF", St::OnAlnum),
    (
        "無変換",
        0x1D,
        false,
        "IME ON・半角英数のまま。Engine が ON にならないこと ← 退行窓の確認",
        St::Direct,
    ),
    (
        "ひらがなキー",
        0xF2,
        false,
        "かなに戻る(後片付け)",
        St::OnAlnum,
    ),
];

/// `--script` で、現在状態から手順の必要状態へ向かう準備の案内（awase 起動中の操作）。
fn script_hint(cur: St, target: St) -> &'static str {
    match (cur, target) {
        (St::Unknown, _) => "状態が読めません。入力欄をクリックしてフォーカスしてください",
        (St::OnKanaComp, _) => "準備: ESC を押して未確定の文字を取り消してください",
        (St::Direct, St::OnKana) => "準備: 変換 を1回押して IME ON にしてください",
        (St::Direct, St::OnAlnum) => {
            "準備: 変換 を1回押して IME ON にしてください(その後 ひらがなキーで半角英数へ)"
        }
        (St::OnKana, St::Direct) | (St::OnAlnum, St::Direct) => {
            "準備: 変換 を1回押して IME OFF(直接入力)にしてください"
        }
        (St::OnAlnum, St::OnKana) => "準備: ひらがなキー を1回押して かな に戻してください",
        (St::OnKana, St::OnAlnum) => "準備: ひらがなキー を1回押して 半角英数 にしてください",
        _ => "準備: 状態を整えてください",
    }
}

/// 現在状態から目標状態へ、次に取るべき 1 手を案内する。
fn hint(cur: St, target: St) -> &'static str {
    match (cur, target) {
        (St::Unknown, _) => "状態が読めません。入力欄をクリックしてフォーカスしてください",
        (St::Direct, _) => "準備: 変換 を1回押して IME ON にしてください",
        (St::OnKana | St::OnAlnum, St::Direct) => {
            "準備: 変換 を1回押して IME OFF(直接入力)にしてください"
        }
        (St::OnKana, St::OnKanaComp) => {
            "準備: ka と入力して未確定のままにしてください(Enter/Space は押さない)"
        }
        (St::OnKana, St::OnAlnum) => "準備: Shift+無変換 を1回押して半角英数にしてください",
        (St::OnKanaComp, _) => "準備: ESC を押して入力を取り消してください",
        (St::OnAlnum, St::OnKana | St::OnKanaComp) => {
            "準備: Shift+無変換 を1回押してかなに戻してください"
        }
        _ => "準備: 状態を整えてください",
    }
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

fn edit_tail(edit: HWND) -> String {
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
        edit_tail: edit_tail(target),
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
            // ひらがなキーは離したときに 0xF2 の KeyUp が届かず(0xF0 の KeyUp が届く)、KeyUp 待ちだと
            // 2回目以降の押下を取りこぼす。KeyUp が無くても、前回の KeyDown から 300ms 以上
            // 空いていれば新しい押下として扱う(オートリピートは約 30ms 間隔なので区別できる)。
            let now_ms_hook = now_ms();
            let first = DOWN_KEYS.with(|d| {
                let mut d = d.borrow_mut();
                let is_new = d
                    .get(&vk)
                    .is_none_or(|last| now_ms_hook.saturating_sub(*last) > 300);
                d.insert(vk, now_ms_hook);
                is_new
            });
            // 監視対象のキー、または Ctrl/Shift 併用の何かは記録するが、
            // 通常の文字キーは (テキスト観測は snapshot に含まれるので) 記録しない。
            if first {
                let ctrl = unsafe { GetAsyncKeyState(0x11) } < 0;
                let shift = unsafe { GetAsyncKeyState(0x10) } < 0;
                // Ctrl+Shift+F12: 現在のステップをスキップ（そのキーが無い場合など）。
                if vk == 0x7B && ctrl && shift {
                    SKIP_REQ.with(|s| *s.borrow_mut() = true);
                } else if let Some(name) = key_name(vk) {
                    let mods = match (ctrl, shift) {
                        (true, true) => "Ctrl+Shift+",
                        (true, false) => "Ctrl+",
                        (false, true) => "Shift+",
                        (false, false) => "",
                    };
                    let injected = if kb.dwExtraInfo == AUTO_MARKER {
                        " (auto)"
                    } else if kb.flags.0 & 0x10 != 0 {
                        " (injected)"
                    } else {
                        ""
                    };
                    let label = format!(
                        "{mods}{name} vk=0x{vk:02X} scan=0x{:02X} press={}{injected}",
                        kb.scanCode,
                        utc_stamp()
                    );
                    KEY_QUEUE.with(|q| {
                        q.borrow_mut().push(KeyEvt {
                            label,
                            vk,
                            ctrl,
                            shift,
                        });
                    });
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
    let all_steps = steps();
    let per_round = all_steps.len();
    let total = per_round * ROUNDS;

    // スキップ要求。
    if SKIP_REQ.with(|s| std::mem::take(&mut *s.borrow_mut())) {
        let idx = STEP_IDX.with(|i| *i.borrow());
        if idx < total {
            let st = &all_steps[idx % per_round];
            append_log(&format!(
                "[SKIP] STEP {}/{} R{} 状態={} キー={}",
                idx % per_round + 1,
                per_round,
                idx / per_round + 1,
                st.state.label(),
                st.key_name
            ));
            STEP_IDX.with(|i| *i.borrow_mut() = idx + 1);
            HOLD_UNTIL.with(|h| *h.borrow_mut() = now + 500);
        }
    }

    // ラウンドが変わったら、対象コントロールへフォーカスを移す。
    let idx_now = STEP_IDX.with(|i| *i.borrow());
    let round_now = (idx_now / per_round).min(ROUNDS - 1);
    let round_changed = LAST_ROUND.with(|l| {
        let changed = *l.borrow() != Some(round_now);
        *l.borrow_mut() = Some(round_now);
        changed
    });
    if round_changed {
        let target = if round_now == 0 {
            EDIT_HWND.with(|e| *e.borrow())
        } else {
            RICH_HWND.with(|e| *e.borrow())
        };
        if let Some(t) = target {
            unsafe {
                let _ = SetFocus(Some(t));
            }
        }
        append_log(&format!(
            "=== ROUND {}/{}: 対象コントロール = {} ===",
            round_now + 1,
            ROUNDS,
            ROUND_NAMES[round_now]
        ));
    }

    // キュー→Pending。「押下前」は直近の周期スナップショット（キーの効果が出る前）。
    let queued: Vec<KeyEvt> = KEY_QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    if !queued.is_empty() {
        let before = LAST_SNAP.with(|l| l.borrow().clone());
        let before_st = before.st();
        for mut ev in queued {
            // 案内中のステップに一致する押下か判定する。
            let idx = STEP_IDX.with(|i| *i.borrow());
            let mut tag = String::from("[準備/その他]");
            if SCRIPT_MODE.with(|m| *m.borrow()) {
                let si = SCRIPT_IDX.with(|i| *i.borrow());
                if si < SCRIPT.len() && now >= HOLD_UNTIL.with(|h| *h.borrow()) {
                    let (name, vk, shift, expect, need) = SCRIPT[si];
                    let shift_muh = SHIFT_MUH.with(|m| *m.borrow()) && ev.vk == 0x1D;
                    if ev.vk == script_vk(vk)
                        && before_st == need
                        && (ev.shift == shift || (shift_muh && ev.shift))
                        && !ev.ctrl
                        && !ev.label.contains("(injected)")
                    {
                        tag = format!("[SCRIPT {}/{} {name} 期待={expect}]", si + 1, SCRIPT.len());
                        SCRIPT_IDX.with(|i| *i.borrow_mut() = si + 1);
                        HOLD_UNTIL.with(|h| *h.borrow_mut() = now + scaled(HOLD_MS));
                    }
                }
            } else if idx < total && now >= HOLD_UNTIL.with(|h| *h.borrow()) {
                let step = &all_steps[idx % per_round];
                if step.vks.contains(&ev.vk)
                    && !ev.ctrl
                    && (!ev.shift || step.allow_shift)
                    && before_st == step.state
                {
                    tag = format!(
                        "[STEP {}/{} R{} 状態={} キー={}]",
                        idx % per_round + 1,
                        per_round,
                        idx / per_round + 1,
                        step.state.label(),
                        step.key_name
                    );
                    STEP_IDX.with(|i| *i.borrow_mut() = idx + 1);
                    HOLD_UNTIL.with(|h| *h.borrow_mut() = now + scaled(HOLD_MS));
                }
            }
            ev.label = format!("{tag} {}", ev.label);
            PENDING.with(|p| {
                p.borrow_mut().push(Pending {
                    label: ev.label,
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
            if idx < after_ms().len() && now >= e.started_ms + after_ms()[idx] {
                e.afters.push(snap.clone());
            }
        }
        let mut i = 0;
        while i < p.len() {
            if p[i].afters.len() >= after_ms().len() {
                finished.push(p.remove(i));
            } else {
                i += 1;
            }
        }
    });
    for e in finished {
        append_log(&format!(
            "[{}] KEY {}  状態={}",
            utc_stamp(),
            e.label,
            e.before.state_label()
        ));
        append_log(&format!("    前     : {}", e.before.compact()));
        for (i, ms) in after_ms().iter().enumerate() {
            append_log(&format!("    +{ms}ms: {}", e.afters[i].compact()));
        }
        let diffs: Vec<String> = after_ms()
            .iter()
            .enumerate()
            .map(|(i, ms)| format!("(前→+{ms}ms): {}", diff_summary(&e.before, &e.afters[i])))
            .collect();
        append_log(&format!("    差分 {}", diffs.join("   ")));
    }

    // 案内表示。
    let idx = STEP_IDX.with(|i| *i.borrow());
    let cur = snap.st();
    let guide = if SCRIPT_MODE.with(|m| *m.borrow()) {
        let si = SCRIPT_IDX.with(|i| *i.borrow());
        let hold = HOLD_UNTIL.with(|h| *h.borrow());
        if si >= SCRIPT.len() {
            "全手順完了です。お疲れさまでした（ログは自動保存済み）".to_string()
        } else {
            let (name, _, _, expect, need) = SCRIPT[si];
            let action = if now < hold {
                format!("待機中… あと {:.1} 秒", (hold - now) as f64 / 1000.0)
            } else if cur != need {
                format!(
                    "この手順の前提: {}。{}",
                    need.label(),
                    script_hint(cur, need)
                )
            } else {
                format!("▶ 今 [{name}] を1回だけ押し、直後に k を1回打って ESC を押してください（未確定を残さない）")
            };
            format!(
                "SCRIPT {}/{}  現在の実IME: {}\n{}\n期待: {}",
                si + 1,
                SCRIPT.len(),
                cur.label(),
                action,
                expect
            )
        }
    } else if FREE_MODE.with(|f| *f.borrow()) {
        format!(
            "自由測定モード（案内なし）。awase 起動中でも実IME状態を記録します。\n現在: {}\n{}",
            cur.label(),
            snap.compact()
        )
    } else if idx >= total {
        "全ステップ完了です。お疲れさまでした（ログは自動保存済み）".to_string()
    } else {
        let step = &all_steps[idx % per_round];
        let hold = HOLD_UNTIL.with(|h| *h.borrow());
        let head = format!(
            "ROUND {}/{}({})  STEP {}/{}  (通し {}/{})",
            idx / per_round + 1,
            ROUNDS,
            ROUND_NAMES[(idx / per_round).min(ROUNDS - 1)],
            idx % per_round + 1,
            per_round,
            idx + 1,
            total
        );
        let action = if now < hold {
            format!(
                "待機中… あと {:.1} 秒（押した効果が落ち着くのを待っています）",
                (hold - now) as f64 / 1000.0
            )
        } else if cur == step.state {
            format!(
                "▶ 今 [{}] を1回だけ押してください（{}の状態）",
                step.key_name,
                step.state.label()
            )
        } else {
            format!(
                "目標状態: {}  現在: {}\n{}",
                step.state.label(),
                cur.label(),
                hint(cur, step.state)
            )
        };
        format!("{head}\n{action}\n(そのキーが無い場合: Ctrl+Shift+F12 でスキップ)")
    };
    if AUTO_MODE.with(|m| *m.borrow()) {
        auto_drive(now, cur, hwnd);
    }
    set_status(&guide);
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

        // 上段: 打鍵する入力欄1（標準 EDIT）。
        let edit = create_child(w!("EDIT"), hwnd, instance, WS_BORDER.0, 10, 10, 940, 28)?;
        EDIT_HWND.with(|e| *e.borrow_mut() = Some(edit));
        // 上段2: 入力欄2（RichEdit 5.0、TSF ネイティブ）。読み込み失敗時は EDIT で代用する。
        let rich = create_child(
            w!("RICHEDIT50W"),
            hwnd,
            instance,
            WS_BORDER.0,
            10,
            44,
            940,
            28,
        )
        .or_else(|_| create_child(w!("EDIT"), hwnd, instance, WS_BORDER.0, 10, 44, 940, 28))?;
        RICH_HWND.with(|e| *e.borrow_mut() = Some(rich));
        // 中段: 案内表示（STATIC、4 行分）。
        let status = create_child(w!("STATIC"), hwnd, instance, 0, 10, 80, 940, 80)?;
        STATUS_HWND.with(|h| *h.borrow_mut() = Some(status));
        // 下段: ログ欄。
        let log = create_child(
            w!("EDIT"),
            hwnd,
            instance,
            WS_BORDER.0 | ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL | WS_VSCROLL.0,
            10,
            168,
            940,
            510,
        )?;
        LOG_HWND.with(|h| *h.borrow_mut() = Some(log));

        let _ = SetFocus(Some(edit));
        let _ = ShowWindow(hwnd, SW_SHOW);
        Ok(hwnd)
    }
}

/// `--activate-gji`: GJI(Google 日本語入力)のTSFプロファイルを、セッション内でアクティブにする。
/// CI(GitHub Actions)のように、`Set-WinUserLanguageList`が次回サインインまで有効にならない環境用。
fn activate_gji_profile() {
    // GJI(Mozc)のCLSIDとプロファイルGUID、日本語(0x0411)。
    let clsid = windows::core::GUID::from_u128(0xD5A86FD5_5308_47EA_AD16_9C4EB160EC3C);
    let profile = windows::core::GUID::from_u128(0x773EB24E_CA1D_4B1B_B420_FA985BB0B80D);
    const TF_PROFILETYPE_INPUTPROCESSOR: u32 = 1;
    const TF_IPPMF_ENABLEPROFILE: u32 = 0x1;
    const TF_IPPMF_FORSESSION: u32 = 0x2000_0000;
    unsafe {
        let mgr: WinResult<ITfInputProcessorProfileMgr> =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER);
        match mgr {
            Ok(m) => {
                let log_active = |label: &str| {
                    let mut p =
                        windows::Win32::UI::TextServices::TF_INPUTPROCESSORPROFILE::default();
                    match m.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &raw mut p) {
                        Ok(()) => append_log(&format!(
                            "[init] アクティブTIP({label}): clsid={:?} profile={:?} lang=0x{:04X}",
                            p.clsid, p.guidProfile, p.langid
                        )),
                        Err(e) => {
                            append_log(&format!("[init] アクティブTIP({label})取得失敗: {e}"))
                        }
                    }
                };
                log_active("前");
                let r = m.ActivateProfile(
                    TF_PROFILETYPE_INPUTPROCESSOR,
                    0x0411,
                    &clsid,
                    &profile,
                    windows::Win32::UI::Input::KeyboardAndMouse::HKL(std::ptr::null_mut()),
                    TF_IPPMF_ENABLEPROFILE | TF_IPPMF_FORSESSION,
                );
                append_log(&format!("[init] GJIプロファイルをアクティブ化: {r:?}"));
                std::thread::sleep(std::time::Duration::from_millis(1500));
                log_active("後");
            }
            Err(e) => append_log(&format!("[init] ITfInputProcessorProfileMgr取得失敗: {e}")),
        }
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
    for a in std::env::args() {
        if let Some(v) = a.strip_prefix("--hold=") {
            if let Ok(n) = v.parse::<u64>() {
                HOLD_MS_INJ.with(|h| *h.borrow_mut() = n);
            }
        }
        if let Some(v) = a.strip_prefix("--repeat=") {
            if let Ok(n) = v.parse::<usize>() {
                REPEAT_N.with(|r| *r.borrow_mut() = n.max(1));
            }
        }
        if let Some(v) = a.strip_prefix("--speed=") {
            if let Ok(n) = v.parse::<u64>() {
                SPEED.with(|r| *r.borrow_mut() = n.max(1));
            }
        }
        if a == "--shiftmuh" {
            SHIFT_MUH.with(|m| *m.borrow_mut() = true);
        }
        if a == "--fast" {
            FAST_MODE.with(|f| *f.borrow_mut() = true);
        }
        if a == "--key=henkan" {
            TOGGLE_VK.with(|t| *t.borrow_mut() = 0x1C);
        }
    }
    // `--diag`: どのキーで GJI が ON になるかを診断する(CI用)。各キーを3.5秒間隔で注入して状態を記録し、閉じる。
    if std::env::args().any(|a| a == "--diag") {
        AUTO_MODE.with(|m| *m.borrow_mut() = true);
        SCRIPT_MODE.with(|m| *m.borrow_mut() = true);
        STEP_IDX.with(|i| *i.borrow_mut() = steps().len() * ROUNDS);
        SCRIPT_IDX.with(|i| *i.borrow_mut() = SCRIPT.len());
        let base = now_ms() + 9000;
        for (i, vk) in [0x1C_u32, 0xF4, 0xF3, 0xF2, 0x16, 0x19, 0x1D]
            .iter()
            .enumerate()
        {
            queue_press(base + (i as u64) * 3500, *vk);
        }
    }
    // `--auto`: --script の手順を、スパイク自身が SendInput で注入して自動実行する。
    if std::env::args().any(|a| a == "--auto") {
        AUTO_MODE.with(|m| *m.borrow_mut() = true);
        SCRIPT_MODE.with(|m| *m.borrow_mut() = true);
        STEP_IDX.with(|i| *i.borrow_mut() = steps().len() * ROUNDS);
    }
    // `--script`: awase 起動中の固定手順（案内は SCRIPT）。
    if std::env::args().any(|a| a == "--script") {
        SCRIPT_MODE.with(|m| *m.borrow_mut() = true);
        STEP_IDX.with(|i| *i.borrow_mut() = steps().len() * ROUNDS);
    }
    // `--free`: 案内なしで、押したキーと実IME状態の推移だけを記録する。
    if std::env::args().any(|a| a == "--free") {
        FREE_MODE.with(|f| *f.borrow_mut() = true);
        STEP_IDX.with(|i| *i.borrow_mut() = steps().len() * ROUNDS);
    }
    // `--round2`: ROUND1(標準EDIT)を飛ばして RichEdit のラウンドから始める。
    if std::env::args().any(|a| a == "--round2") {
        STEP_IDX.with(|i| *i.borrow_mut() = steps().len());
    }
    // RichEdit 5.0（TSF ネイティブ）のウィンドウクラスは Msftedit.dll が登録する。
    let _ = unsafe { LoadLibraryW(w!("Msftedit.dll")) };
    let tsf_ok = init_tsf();
    let hwnd = create_window()?;
    if std::env::args().any(|a| a == "--activate-gji") {
        activate_gji_profile();
        // awase がアクティブなTIPを検出する(ポーリング周期)まで待ってから、手順を始める。
        AUTO_NEXT.with(|n| *n.borrow_mut() = now_ms() + 8000);
    }

    append_log("=== IME key matrix spike (awase 非依存) ===");
    append_log("観測: A=IMM(ImmGet*) / B=WM_IME_CONTROL / T=TSFスレッドcompartment / G=TSFグローバルcompartment");
    append_log(
        "conv の目安: 0x19=ひらがな(NATIVE|FULLSHAPE|ROMAN) 0x10=半角英数(ROMAN) 0x00=直接入力系",
    );
    if let Err(e) = tsf_ok {
        append_log(&format!("[init] TSF初期化失敗: {e}（T/Gは使えません）"));
    }
    append_log("手順: awase を止める → 画面中段の案内に従ってキーを1回ずつ押す（全 2ラウンド×20ステップ、各押下後は3秒待機）");
    append_log("ROUND1=標準EDIT / ROUND2=RichEdit(TSFネイティブ)。そのキーが無い場合は Ctrl+Shift+F12 でスキップ");
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
