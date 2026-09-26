//! IME モードキー動作マトリクス計測スパイク（awase 非依存）。
//!
//! **名前は「spike」だが、現在は CI（`.github/workflows/e2e-ime.yml`）の全構成が使う実質的な本番ハーネス**である
//! （ADR-186 の実機E2E、ADR-190 の操作シナリオ、ADR-191 の学習/検証ラウンドを、フラグで切り替えて1つの exe が担う）。
//! ファイル名の改名は、CI 構成・ドキュメントからの参照が多いため行っていない（改名案: `ime_key_harness` 等）。
//! 以下の「フラグ一覧」が現行の全 CLI フラグ（`run()` の引数解析と一致させること。フラグを足したらここも更新する）。
//!
//! ## フラグ一覧
//! 区分: [ADR-186] 実機E2E(判定は check*.py) / [ADR-190] 操作シナリオ / [学習] 格子・通知(ADR-191、判定せず
//! ログ回収、解析は tools/e2e/ime_key_matrix/grid_learn.py 等) / [検証] 打鍵時予測の検証 walk / [共通]。
//! - 手順の選択: `--script`(固定手順を案内、awase 起動中の A/B) / `--auto`(固定手順をスパイク自身が SendInput で注入) /
//!   `--free`(案内なし、押したキーと実 IME 状態の推移だけ記録) / `--round2`(RichEdit のラウンドから開始) [ADR-186]
//! - `--charthumb=CHAR,THUMB`(文字→親指の順に押し文字を先に離して親指を押し続ける。ADR-199 T10 決定A、判定は check_charthumb.py) /
//!   `--seq=F2,F0,A0,...`(VK16進の任意キー列。前提状態なしで押す) / `--hz`(半角/全角 0xF3/0xF4 の交互・連続) /
//!   `--resync`, `--resync-gap=MS`(Ctrl+無変換/変換のリセット操作の2打間隔。既定100) / `--cold`(`--walk` と併用: 明示意図なしで
//!   いきなり無変換/変換) / `--key=henkan`(手順の「無変換」を「変換」に) / `--shiftmuh`(無変換を Shift+無変換に) /
//!   `--vkprobe`(ひらがな系の正しい VK 調べ) / `--diag`(どのキーで GJI が ON になるかの診断) [ADR-186/190]
//! - `--walk`(値なし: ADR-186 の固定キー列 WALK) / `--walk=N --seed=S`(ランダムなキーを N 回注入。seed は線形合同法。
//!   awase 有り=検証ラウンド `cal-verify-*`、awase 無し=学習ログ) [検証/学習]
//! - `--grid=s1..s4`(状態×キーの格子。各シャードは状態×キーの部分集合)、`--grid-trials=N`(各セルの試行数上限)、
//!   `--grid-setup=keys|keys-immreset`(状態のセットアップ方式。省略=IMM 書き込みで作る第1版(誤りを含むので学習には使わない)、
//!   `keys`=リセットも含めキーだけで到達(第3版)、`keys-immreset`=リセットだけ IMM(第2版))、
//!   `--grid-adaptive`(1パス目は全セル1回、2パス目は前回の非決定セル・監査標本・履歴依存ブロックだけ再試行)、
//!   `--grid-retry-file=PATH`(再試行対象セルの一覧、grid-tables/nondet-*.txt)、`--grid-audit-pct=N`(監査標本の割合。既定10) [学習]
//! - 高速化: `--fast`(+1500ms の観測を省く) / `--speed=K`(手順間の待ちを K 倍速。既定1) /
//!   `--snap100`(観測を +100ms だけにして、通知の静止で待ちを終える) [学習]
//! - 通知: `--notify`(TSF スレッド compartment の変更通知を待ちの終了条件に使う。開閉/変換モードの待ちだけ) /
//!   `--notify-quiet=MS`(最後の通知からこの間静かなら確定。既定40) / `--notify-nochg=MS`(通知が来なければ「変化なし」と見なす待ち。
//!   既定150) / `--notify-comp`(入力中/変換中/確定のイベント=WM_IME_*/EN_CHANGE を記録する。ログだけ) [学習]
//! - 共通: `--activate-gji`(CI 用: GJI プロファイルを有効化し、フックを遅延して張る) / `--msime`(有効化する IME を Microsoft IME に) /
//!   `--hold=MS`(注入キーの保持時間。既定80) / `--repeat=N`(全手順をこのプロセス内で N 回繰り返す)
//!
//! ## ログのタグ
//! - `KEY [...]`: 押下 1 件の記録(押下前と +100/+400/+1500ms の A/B/T/G 観測。`--fast` は +1500ms なし)。
//! - `[GRID-BEGIN]`/`[GRID-PRE]`(セットアップ検証)/`[GRID-SKIP]`(セットアップ不能・到達不能で飛ばした)/`[GRID-PRUNE]`
//!   (到達不能状態の試行を実行前に除外)/`[GRID-ABORT]`(セットアップが連続 20 回不能で打ち切り。CI では rc=3=INVALID)/
//!   `[GRID-RESET]`(リセット結果)/`[GRID-EXPLORE]`・`[GRID-SETUP]`・`[GRID-EDGE]`(キー到達の探索と遷移グラフ)/
//!   `[GRID-ADAPTIVE]`(適応再試行の内訳)/`[GRID]`(完了)。
//! - `[NOTIFY]`(compartment 通知の受信・購読)/`[NOTIFY-STATS]`(通知で早く終わった待ち・変化なしで終わった待ち・上限まで待った待ち)/
//!   `[COMP]`(`--notify-comp` の WM_IME_*/EN_CHANGE 記録)/`[WALK]`(walk 完了)/`[AUTO]`(自動手順中のフォーカス復帰など)/`[FATAL]`(panic・引数エラー。`--auto` ではモーダルを出さずログだけ)/
//!   `[init]`(起動時の情報・警告。未知の引数は警告、`--grid=`/`--grid-setup=`/数値の不正は `[FATAL] 引数エラー` で終了)。
//!
//! ## 実行例
//! - 学習(awase なし、ATOK): `ime_key_matrix_spike.exe --auto --hold=180 --activate-gji --grid=s1 --grid-setup=keys --fast --notify --grid-adaptive`
//! - 検証(awase 有り、`AWASE_TEST_INJECTION=1`): `ime_key_matrix_spike.exe --auto --hold=180 --activate-gji --walk=100 --seed=1`
//!
//! ---
//! 以下は、もともとの「対話的な実機計測スパイク」としての説明（`--script`/`--free` 系の使い方）。
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
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, VIRTUAL_KEY,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, CLSID_TF_ThreadMgr, ITfCompartmentMgr,
    ITfInputProcessorProfileMgr, ITfThreadMgr, GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
    GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, GUID_TFCAT_TIP_KEYBOARD,
};

/// `--notify-comp`(ADR-191/193): 入力中/変換中/確定のイベントが CI(GJI・標準 Edit)で届くかを測るログだけを出す
/// (WM_IME_STARTCOMPOSITION/COMPOSITION/ENDCOMPOSITION/NOTIFY を Edit のサブクラスで、EN_CHANGE を親の WM_COMMAND で受ける)。
static NOTIFY_COMP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static EDIT_ORIG_PROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

extern "system" fn edit_sub_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let name = match msg {
        0x010D => Some("WM_IME_STARTCOMPOSITION"),
        0x010E => Some("WM_IME_ENDCOMPOSITION"),
        0x010F => Some("WM_IME_COMPOSITION"),
        0x0282 => Some("WM_IME_NOTIFY"),
        0x0281 => Some("WM_IME_SETCONTEXT"),
        0x0286 => Some("WM_IME_CHAR"),
        _ => None,
    };
    if let Some(n) = name {
        if NOTIFY_COMP.load(std::sync::atomic::Ordering::Relaxed) {
            append_log(&format!(
                "[COMP] {n} wp=0x{:X} lp=0x{:X} t={}",
                wparam.0,
                lparam.0,
                now_ms()
            ));
            // composition の開始/更新/終了も「通知」として待ちの静止判定に使う(--notify と併用時)。
            if matches!(msg, 0x010D..=0x010F) {
                NOTIFY_LAST.with(|n| *n.borrow_mut() = now_ms());
            }
        }
    }
    let orig = EDIT_ORIG_PROC.load(std::sync::atomic::Ordering::Relaxed);
    // SAFETY: orig は SetWindowLongPtrW が返した元のウィンドウプロシージャ(0 なら既定へ)。
    unsafe {
        if orig == 0 {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        } else {
            CallWindowProcW(
                Some(std::mem::transmute::<
                    isize,
                    unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
                >(orig)),
                hwnd,
                msg,
                wparam,
                lparam,
            )
        }
    }
}

/// `--notify`(ADR-191/193): TSF スレッド compartment の変更通知(`ITfCompartmentEventSink`)を購読し、
/// キー注入後の固定待ちを「通知が来て静かになるまで(上限は従来の固定待ち)」に置き換える。
/// 開閉/変換モードだけが対象。入力中・変換中・確定(composition と入力欄末尾)は compartment に出ないので固定待ちのまま。
#[allow(clippy::ref_as_ptr, clippy::inline_always)]
mod notify_sink {
    use windows::core::{implement, Interface, GUID};
    use windows::Win32::UI::TextServices::{
        ITfCompartmentEventSink, ITfCompartmentEventSink_Impl, ITfCompartmentMgr, ITfSource,
        GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
        GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
    };

    #[implement(ITfCompartmentEventSink)]
    pub(crate) struct NotifySink;

    impl ITfCompartmentEventSink_Impl for NotifySink_Impl {
        fn OnChange(&self, rguid: *const GUID) -> windows::core::Result<()> {
            // SAFETY: rguid はこのコールバックの実行中のみ有効な、TSF ランタイムが用意したポインタ。
            let name = match unsafe { rguid.as_ref() } {
                Some(g) if *g == GUID_COMPARTMENT_KEYBOARD_OPENCLOSE => "OPENCLOSE",
                Some(g) if *g == GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION => "CONVERSION",
                Some(g) if *g == GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE => "SENTENCE",
                _ => "OTHER",
            };
            // extern "system"(非 unwind ABI)なので、panic が越境すると abort する。ここで止める。
            let _ = std::panic::catch_unwind(|| super::note_notify(name));
            Ok(())
        }
    }

    /// 3つの compartment に購読する。返す値(source, cookie, sink)を保持し続けないと購読が切れる。
    pub(crate) fn advise(
        cmgr: &ITfCompartmentMgr,
    ) -> Vec<(ITfSource, u32, ITfCompartmentEventSink)> {
        let mut v = Vec::new();
        for g in [
            &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
            &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
            &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE,
        ] {
            // SAFETY: cmgr は有効な COM 参照(メインスレッドの STA)。
            unsafe {
                let Ok(comp) = cmgr.GetCompartment(g) else {
                    continue;
                };
                let Ok(source) = comp.cast::<ITfSource>() else {
                    continue;
                };
                let sink: ITfCompartmentEventSink = NotifySink.into();
                if let Ok(cookie) =
                    source.AdviseSink(&<ITfCompartmentEventSink as Interface>::IID, &sink)
                {
                    v.push((source, cookie, sink));
                }
            }
        }
        v
    }
}
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CallNextHookEx, CallWindowProcW, CreateWindowExW, DefWindowProcW,
    DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, KillTimer, MessageBoxW, PostMessageW, PostQuitMessage,
    RegisterClassW, SendMessageTimeoutW, SendMessageW, SetForegroundWindow, SetTimer,
    SetWindowLongPtrW, SetWindowTextW, SetWindowsHookExW, ShowWindow, TranslateMessage,
    CW_USEDEFAULT, GWLP_WNDPROC, KBDLLHOOKSTRUCT, MB_ICONERROR, MB_OK, MSG, SMTO_ABORTIFHUNG,
    SW_SHOW, WH_KEYBOARD_LL, WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_SETFOCUS,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE, WS_VSCROLL,
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
    /// `--charthumb=CHAR,THUMB`: (文字VK, 親指VK, 残りラウンド数)。`auto_drive` がフォーカス確認のあとでラウンドごとに予約する。
    static CHARTHUMB: RefCell<Option<(u32, u32, u32)>> = const { RefCell::new(None) };
    static AUTO_NEXT: RefCell<u64> = const { RefCell::new(0) };
    /// `--activate-gji` 時: キーフックをこの時刻(ms)まで遅らせて張る。0=張り済み/不要。
    /// LLフックは後から張ったものが先に呼ばれる。awase より後に張らないと、awase が消費・再注入した
    /// キー(自己注入)しか見えず、元の押下を検知できなくてステップが進まない(CI run 35483174264)。
    static HOOK_AT: RefCell<u64> = const { RefCell::new(0) };
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
    /// `--snap100`: 観測を +100ms だけにする(CI の格子ログで +100ms と +400ms の値が一致 99.8%)。押下後の待ちは通知の静止で終える。
    static SNAP100: RefCell<bool> = const { RefCell::new(false) };
    /// `--speed=K`: 手順間の待ち時間をK倍速にする(既定1=従来どおり)。
    static SPEED: RefCell<u64> = const { RefCell::new(1) };
    /// `--notify`: compartment 変更通知を待ちの終了条件に使う(開閉/変換モードの待ちだけ。上限は従来の固定待ち)。
    static NOTIFY_MODE: RefCell<bool> = const { RefCell::new(false) };
    /// `--notify-quiet=MS`: 最後の通知からこの時間なにも来なければ確定(既定40)。
    static NOTIFY_QUIET_MS: RefCell<u64> = const { RefCell::new(40) };
    /// `--notify-nochg=MS`: 最後のキーの後この時間通知が来なければ「変化なし」と見なす(既定150。CI実測のP99×1.5以上にする)。
    static NOTIFY_NOCHG_MS: RefCell<u64> = const { RefCell::new(150) };
    /// 直近の通知の時刻(now_ms)。
    static NOTIFY_LAST: RefCell<u64> = const { RefCell::new(0) };
    /// 待ち中の通知待ち: (最後のキーの時刻, 従来の固定待ちの終了時刻=上限)。
    static NOTIFY_WAIT: RefCell<Option<(u64, u64)>> = const { RefCell::new(None) };
    /// 購読を保持する(解放すると購読が切れる)。
    static NOTIFY_SINKS: RefCell<Vec<(windows::Win32::UI::TextServices::ITfSource, u32, windows::Win32::UI::TextServices::ITfCompartmentEventSink)>> = const { RefCell::new(Vec::new()) };
    /// 統計: 通知で早く終わった待ち / 通知が来ず「変化なし」で終わった待ち / 上限まで待った待ち。
    static NOTIFY_STATS: RefCell<[u32; 3]> = const { RefCell::new([0, 0, 0]) };
    /// `--hold=NNN`: 注入キーの保持時間ms(既定80)。人の押下(>100ms)でだけ通るタイマー経路を再現する。
    static HOLD_MS_INJ: RefCell<u64> = const { RefCell::new(80) };
    /// `--grid=SHARD`: 状態(開閉×入力モード×入力中の段階)を IMM で決定的に作り、キーを押して効果を記録する(ADR-191 学習ラウンド)。
    static GRID: RefCell<Option<GridRun>> = const { RefCell::new(None) };
    /// 次に検知するテスト対象キー(VK)と、その KEY 行に付けるタグ。
    static GRID_TAG: RefCell<Option<(u32, String)>> = const { RefCell::new(None) };
    /// `--walk=N`: 固定手順の代わりに、ランダムなキーをN回注入する(効果学習・検証ラウンド用。ADR-191)。
    static WALK_N: RefCell<usize> = const { RefCell::new(0) };
    static WALK_DONE: RefCell<usize> = const { RefCell::new(0) };
    /// `--seed=S`: `--walk=N` の乱数シード(線形合同法)。
    static WALK_RNG: RefCell<u64> = const { RefCell::new(1) };
    /// `--walk`: SCRIPT の代わりに WALK(前提状態なしの固定キー列)を使う。
    static WALK_MODE: RefCell<bool> = const { RefCell::new(false) };
    /// `--cold`(`--walk`と併用): 先頭のひらがなを除き、明示意図が無い状態でいきなり無変換/変換を押す手順にする。
    static COLD_MODE: RefCell<bool> = const { RefCell::new(false) };
    /// `--resync`: ずれた状態から Ctrl+無変換→Ctrl+変換(または逆)を素早く押して、実IMEとEngineが揃うかを見る手順(RESYNC)。
    static RESYNC_MODE: RefCell<bool> = const { RefCell::new(false) };
    /// `--hz`: 半角/全角キー(0xF3/0xF4、GJIではどちらも開閉トグル)を交互・連続で押す手順(HZ)。
    static HZ_MODE: RefCell<bool> = const { RefCell::new(false) };
    /// `--resync-gap=NNN`: リセット操作の2打の間隔 ms(1打目のキーを離してから2打目を押すまで)。
    static RESYNC_GAP_MS: RefCell<u64> = const { RefCell::new(100) };
    /// `--vkprobe`: 未知のキーも記録する(スキャンコードだけ注入したとき、OSがどのVKに変換するかを見る)。
    static VKPROBE_MODE: RefCell<bool> = const { RefCell::new(false) };
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
    if SNAP100.with(|f| *f.borrow()) {
        &[100]
    } else if FAST_MODE.with(|f| *f.borrow()) {
        &AFTER_MS_FAST
    } else {
        &AFTER_MS_FULL
    }
}

/// `--speed=K` で手順間の待ち時間を縮める。
fn scaled(ms: u64) -> u64 {
    ms / SPEED.with(|s| *s.borrow()).max(1)
}

/// compartment 変更通知を受けた(メインスレッドの sink から)。
fn note_notify(name: &str) {
    let t = now_ms();
    NOTIFY_LAST.with(|n| *n.borrow_mut() = t);
    append_log(&format!("[NOTIFY] {name} t={t}"));
}

/// キー注入後の待ちの終了時刻を返す。`--notify` でなければ従来の固定待ち(`cap_next`)。
/// `--notify` なら、最後のキー(`last_key_at`)の後 nochg ms 通知が来なければ終わる暫定の時刻を返し、
/// tick 側(`notify_tick_waiting`)が「通知が来たら静かになるまで、上限は `cap_next`」を判定する。
fn notify_settle_next(last_key_at: u64, cap_next: u64) -> u64 {
    if !NOTIFY_MODE.with(|m| *m.borrow()) {
        return cap_next;
    }
    let nochg = NOTIFY_NOCHG_MS.with(|n| *n.borrow());
    NOTIFY_WAIT.with(|w| *w.borrow_mut() = Some((last_key_at, cap_next)));
    cap_next.min(last_key_at + nochg)
}

/// `--notify` の待ちがまだ続くか(true=まだ待つ)。`AUTO_NEXT` を過ぎた後にだけ呼ぶ。
fn notify_tick_waiting(now: u64) -> bool {
    let Some((key_at, cap)) = NOTIFY_WAIT.with(|w| *w.borrow()) else {
        return false;
    };
    let quiet = NOTIFY_QUIET_MS.with(|n| *n.borrow());
    let nochg = NOTIFY_NOCHG_MS.with(|n| *n.borrow());
    let last = NOTIFY_LAST.with(|n| *n.borrow());
    let (done, kind) = if now >= cap {
        (true, 2)
    } else if last >= key_at {
        (now >= last + quiet, 0)
    } else {
        (now >= key_at + nochg, 1)
    };
    if done {
        NOTIFY_WAIT.with(|w| *w.borrow_mut() = None);
        NOTIFY_STATS.with(|s| s.borrow_mut()[kind] += 1);
    }
    !done
}

fn now_ms() -> u64 {
    START.with(|s| {
        s.borrow().map_or(0, |t| {
            u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX)
        })
    })
}

fn key_name(vk: u32) -> Option<&'static str> {
    // `--walk=N` が注入する英字。記録しないと未確定文字列の発生(状態遷移)が表に載らない。既存の手順のログは変えない。
    if WALK_N.with(|n| *n.borrow()) > 0 {
        match vk {
            0x41 => return Some("a"),
            0x4B => return Some("k"),
            _ => {}
        }
    }
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
        0xA0 => "左Shift(0xA0)",
        0xA1 => "右Shift(0xA1)",
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
    /// `--walk` 用: 前提状態を要求しない(どの状態でも押す)。
    Any,
}

impl St {
    fn label(self) -> &'static str {
        match self {
            Self::Direct => "直接入力",
            Self::OnKana => "IME ON・かな・入力なし",
            Self::OnKanaComp => "IME ON・入力中(未確定あり)",
            Self::OnAlnum => "IME ON・半角英数・入力なし",
            Self::Unknown => "不明",
            Self::Any => "任意",
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
        0xF2 | 0x15 | 0xF1 | 0xF5 | 0xF6 => 0x70,
        0xF3 | 0xF4 | 0x19 => 0x29,
        0xF0 => 0x3A,
        0xA0 => 0x2A,
        0xA1 => 0x36,
        0x4B => 0x25,
        0x41 => 0x1E,
        0x0D => 0x1C,
        0x20 => 0x39,
        0x08 => 0x0E,
        0x1B => 0x01,
        _ => 0,
    }
}

/// `SendInput` で1イベントを注入する（`AUTO_MARKER` 付き）。
/// `--vkprobe` の候補の符号化: `SCAN_ONLY | scan` はスキャンコードだけ(wVk=0)を注入し、OSのキーボードレイアウトに
/// VKを決めさせる。`NOSCAN | vk` はVKだけ(wScan=0)を注入する。
const SCAN_ONLY: u32 = 0x1_0000;
const NOSCAN: u32 = 0x2_0000;

fn send_key(vk: u32, down: bool) {
    let up = if down {
        KEYBD_EVENT_FLAGS(0)
    } else {
        KEYEVENTF_KEYUP
    };
    let (w_vk, w_scan, flags) = if vk & SCAN_ONLY != 0 {
        (0, (vk & 0xFF) as u16, KEYEVENTF_SCANCODE | up)
    } else if vk & NOSCAN != 0 {
        ((vk & 0xFFFF) as u16, 0, up)
    } else {
        (u16::try_from(vk).unwrap_or(0), scan_for(vk), up)
    };
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(w_vk),
                wScan: w_scan,
                dwFlags: flags,
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
        || notify_tick_waiting(now)
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
    if let Some((vk_char, vk_thumb, left)) = CHARTHUMB.with(|c| *c.borrow()) {
        if left > 0 {
            // ADR-199 T10 決定A の1ラウンド: VK_IME_ON で IME を ON → 文字↓ → 親指↓(30ms後) → 文字↑(親指↓の2ms後=重なりほぼ無し)
            // → 親指を800ms押し続けて離す(親指の KEY 行の +400ms は保持中、+1500ms は解放後)。
            const IME_ON_SETTLE_MS: u64 = 2500;
            const THUMB_LEAD_MS: u64 = 30;
            const CHAR_UP_MS: u64 = 32;
            const THUMB_HOLD_MS: u64 = 800;
            const ROUND_MS: u64 = 6000;
            CHARTHUMB.with(|c| *c.borrow_mut() = Some((vk_char, vk_thumb, left - 1)));
            queue_press(now, 0x16); // VK_IME_ON(ATOK プリセットの F2 は半角英数へ切り替えるので使わない)
            let t1 = now + IME_ON_SETTLE_MS;
            AUTO_QUEUE.with(|q| {
                let mut q = q.borrow_mut();
                q.push((t1, vk_char, true));
                q.push((t1 + THUMB_LEAD_MS, vk_thumb, true));
                q.push((t1 + CHAR_UP_MS, vk_char, false));
                q.push((t1 + THUMB_LEAD_MS + THUMB_HOLD_MS, vk_thumb, false));
            });
            AUTO_NEXT.with(|n| *n.borrow_mut() = now + ROUND_MS);
            return;
        }
    }
    if GRID.with(|g| g.borrow().is_some()) {
        grid_drive(now, hwnd);
        return;
    }
    if WALK_N.with(|n| *n.borrow()) > 0 {
        walk_drive(now);
        return;
    }
    let si = SCRIPT_IDX.with(|i| *i.borrow());
    if si >= script().len() {
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
    let (name, vk, _, _, need) = script()[si];
    if need != St::Any && cur != need {
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
        queue_step(now, vk);
    }
    // リセット操作(2打)は約 0.7 秒かかるので、k(Engine状態の確認)/ESC は後ろへずらす。
    let (k_at, esc_at, next_at) = if vk == RESYNC_ON || vk == RESYNC_OFF {
        (1200, 1700, 2400)
    } else {
        (700, 1200, 1800)
    };
    queue_press(now + scaled(k_at), 0x4B); // k
    queue_press(now + scaled(esc_at), 0x1B); // ESC
    AUTO_NEXT.with(|n| *n.borrow_mut() = now + scaled(next_at));
}

/// `--resync` 用の手順コード(VKではない)。RESYNC_ON = Ctrl+無変換 → Ctrl+変換(素早く、Ctrlは押したまま)。終わりはIME ON。
/// RESYNC_OFF = Ctrl+変換 → Ctrl+無変換。終わりはIME OFF。awase の既定の ime_off(Ctrl+無変換)/ime_on(Ctrl+変換)。
const RESYNC_ON: u32 = 0xFE01;
const RESYNC_OFF: u32 = 0xFE02;

/// 手順の「最初の押下」として、フックが手順に対応づけるキー(VK, Ctrl併用か)。
fn step_first_key(vk: u32) -> (u32, bool) {
    match vk {
        RESYNC_ON => (0x1D, true),
        RESYNC_OFF => (0x1C, true),
        _ => (script_vk(vk), false),
    }
}

/// 手順1件分の注入を予約する。RESYNC は Ctrl を押したまま2つのキーを間隔 `RESYNC_GAP_MS` で押す。
fn queue_step(now: u64, vk: u32) {
    if vk != RESYNC_ON && vk != RESYNC_OFF {
        queue_press(now, script_vk(vk));
        return;
    }
    let (first, second) = if vk == RESYNC_ON {
        (0x1D, 0x1C)
    } else {
        (0x1C, 0x1D)
    };
    let hold = HOLD_MS_INJ.with(|h| *h.borrow());
    let gap = RESYNC_GAP_MS.with(|g| *g.borrow());
    // Ctrl を先に押し、フックが Ctrl 状態を見られるだけの間(注入は次のtickで実行される)をあけてから1打目を押す。
    // 間が短いと1打目に Ctrl が付かず、手順に対応づけられない(CIで実測: 15ms では失敗)。
    let lead = 200;
    let second_at = now + lead + hold + gap;
    AUTO_QUEUE.with(|q| q.borrow_mut().push((now, 0x11, true)));
    queue_press(now + lead, first);
    queue_press(second_at, second);
    AUTO_QUEUE.with(|q| q.borrow_mut().push((second_at + hold + 15, 0x11, false)));
}

/// `--hz` の手順: 半角/全角キーは、GJIではどちらのVK(0xF3/0xF4)も「開なら閉、閉なら開」のトグル(ADR-186)。
/// awase がVKの種類で方向を決め打つ(0xF3=OFF、0xF4=ON)と、同じVKの連続やF3/F4の順序でずれる。
const HZ: [(&str, u32, bool, &str, St); 8] = [
    ("半角全角F3", 0xF3, false, "開閉トグル", St::Any),
    ("半角全角F3", 0xF3, false, "開閉トグル", St::Any),
    ("半角全角F4", 0xF4, false, "開閉トグル", St::Any),
    ("半角全角F4", 0xF4, false, "開閉トグル", St::Any),
    ("半角全角F3", 0xF3, false, "開閉トグル", St::Any),
    ("半角全角F4", 0xF4, false, "開閉トグル", St::Any),
    ("半角全角F4", 0xF4, false, "開閉トグル", St::Any),
    ("半角全角F3", 0xF3, false, "開閉トグル", St::Any),
];

/// `--resync` の手順: ずれを起こすキー(無変換/変換)と、リセット操作を交互に押す。
const RESYNC: [(&str, u32, bool, &str, St); 10] = [
    ("ひらがなキー", 0xF2, false, "かなON", St::Any),
    (
        "無変換",
        0x1D,
        false,
        "実IMEが閉じる。followが無いとEngineだけONのままずれる",
        St::Any,
    ),
    (
        "resync(ON)",
        RESYNC_ON,
        false,
        "実IME ON(かな) かつ Engine ON",
        St::Any,
    ),
    ("無変換", 0x1D, false, "実IMEが閉じる", St::Any),
    (
        "resync(OFF)",
        RESYNC_OFF,
        false,
        "実IME OFF かつ Engine OFF",
        St::Any,
    ),
    (
        "変換",
        0x1C,
        false,
        "実IMEが開く。followが無いとEngineだけOFFのままずれる",
        St::Any,
    ),
    (
        "resync(ON)",
        RESYNC_ON,
        false,
        "実IME ON(かな) かつ Engine ON",
        St::Any,
    ),
    ("無変換", 0x1D, false, "実IMEが閉じる", St::Any),
    (
        "resync(OFF)",
        RESYNC_OFF,
        false,
        "実IME OFF かつ Engine OFF",
        St::Any,
    ),
    ("ひらがなキー", 0xF2, false, "後片付け", St::Any),
];

/// `--walk=N` が注入するキーの集合（VK, 表示名）。開閉・変換モード・未確定の各状態に
/// 自然に遷移するよう、IMEキー4種 + 入力(k,a) + 確定/取消(Enter,Esc)。
const WALK_KEYS: [(u32, &str); 8] = [
    (0x1D, "無変換"),
    (0x1C, "変換"),
    (0xF2, "ひらがな"),
    (0xF3, "半角/全角"),
    (0x4B, "k"),
    (0x41, "a"),
    (0x0D, "Enter"),
    (0x1B, "Esc"),
];

fn walk_next_index() -> usize {
    WALK_RNG.with(|r| {
        let mut r = r.borrow_mut();
        *r = r
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((*r >> 33) as usize) % WALK_KEYS.len()
    })
}

/// `--walk=N` の1手: ランダムなキーを1つ注入し、効果が落ち着くまで待つ。
fn walk_drive(now: u64) {
    let done = WALK_DONE.with(|d| *d.borrow());
    let total = WALK_N.with(|n| *n.borrow());
    if done >= total {
        if !AUTO_DONE.with(|d| std::mem::replace(&mut *d.borrow_mut(), true)) {
            append_log(&format!(
                "[WALK] {total}手完了（全手順完了。1.5秒後に自動で閉じます）"
            ));
            AUTO_CLOSE_AT.with(|c| *c.borrow_mut() = now + 1500);
        }
        return;
    }
    let (vk, name) = WALK_KEYS[walk_next_index()];
    append_log(&format!(
        "[WALK {}/{total}] 注入: {name} vk=0x{vk:02X}",
        done + 1
    ));
    queue_press(now, vk);
    WALK_DONE.with(|d| *d.borrow_mut() = done + 1);
    AUTO_NEXT.with(|n| *n.borrow_mut() = now + scaled(1900));
}

// ─── --grid: 状態を決定的に作って、キーの効果を格子状に網羅する(ADR-191 学習ラウンド) ─────────

/// 入力中の段階。`k`,`a` で「か」の未確定を作り、続くキーで変換系の段階に入る。
#[derive(Clone, Copy, PartialEq, Eq)]
enum GComp {
    None,
    Typing,
    ConvSpace,
    ConvHenkan,
    ConvMuhenkan,
    /// 履歴依存の確認用: 入力中に ひらがな を押した直後。
    PrevHiragana,
    /// 履歴依存の確認用: 入力中に Esc で消して、入れ直した直後(直前キーは a)。
    PrevEsc,
}

impl GComp {
    const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Typing => "typing",
            Self::ConvSpace => "conv-space",
            Self::ConvHenkan => "conv-henkan",
            Self::ConvMuhenkan => "conv-muhenkan",
            Self::PrevHiragana => "typing-prev-hiragana",
            Self::PrevEsc => "typing-prev-esc",
        }
    }
    /// この段階を作るキー列(VK)。
    fn keys(self) -> &'static [u32] {
        match self {
            Self::None => &[],
            Self::Typing => &[0x4B, 0x41],
            Self::ConvSpace => &[0x4B, 0x41, 0x20],
            Self::ConvHenkan => &[0x4B, 0x41, 0x1C],
            Self::ConvMuhenkan => &[0x4B, 0x41, 0x1D],
            Self::PrevHiragana => &[0x4B, 0x41, 0xF2],
            Self::PrevEsc => &[0x4B, 0x41, 0x1B, 0x4B, 0x41],
        }
    }
}

#[derive(Clone)]
struct GridTrial {
    open: bool,
    conv: u32,
    comp: GComp,
    key: u32,
    trial: usize,
}

struct GridRun {
    shard: String,
    trials: Vec<GridTrial>,
    idx: usize,
    phase: u8,
    attempt: u8,
    /// 直近の検証(settle 直後)で目標と一致したか。
    verify1_ok: bool,
    /// `--grid-setup=keys`: 変換モードを IMM で書かず、キー列で到達する(探索の結果 `expl.known` の経路を使う)。
    keys_mode: bool,
    /// `--grid-setup=keys-immreset`(第2版): リセットだけ IMM で(開,0x19)へ書く。既定の `keys` は第3版=リセットもキーのみ。
    reset_imm: bool,
    /// 第3版のキーのみリセットの試行回数(観測→キー→観測…)。
    rs_tries: u8,
    expl: GridExplore,
    /// `--grid-adaptive`: 実行中に、セットアップ不能だったセルを1回だけ末尾へ再試行する。
    adaptive: bool,
    /// 実行中に再試行へ回したセル("state|key")。同じセルは1回だけ。
    retried: Vec<String>,
    /// セットアップ不能だった状態(同じ状態の残りの試行は、同じ理由で失敗するので飛ばす)。
    failed_states: Vec<String>,
    /// セットアップに成功して押下まで進んだ試行の数(統計)。
    setup_ok: u32,
    /// セットアップ不能の連続回数(成功したら0に戻る)。`GRID_ABORT_CONSEC_FAILS` に達したら打ち切る。
    consec_fail: u32,
    /// 探索の後に、到達不能な状態の試行を外したか。
    pruned: bool,
    /// 最初の tick で1回だけ出す、適応的な試行の内訳。
    note: Option<String>,
}

/// セットアップの試行(同じ状態の2回目以降は飛ばすので、実質は別々の状態)が連続してこの回数失敗したら格子を打ち切る
/// (リセット基準の食い違い等で全状態が失敗するのに、CIを無駄に走らせない)。1度でも成功すれば数え直すので、途中から全部失敗する
/// 場合も検出でき、冒頭の数状態(入力中/変換中など)だけが作れない通常の実行(ATOK)では打ち切らない。
/// 20 の根拠(実測): ATOK の実ログ(fastgrid 8 本・notifygrid 5 本、成功した完走ラン)で、押下に進まないセットアップ不能が連続した最大は 9 回
/// (on-c09 と conv-muhenkan 系のキー×状態)。その 2 倍強。8 で打ち切ると通常の ATOK も中断された(fastgrid の初回 CI、8 連続)。
const GRID_ABORT_CONSEC_FAILS: u32 = 20;

/// `--grid-setup=keys` の探索(BFS): リセット状態(IME_OFF→IME_ON)から、キーを押して到達できる(開閉, 変換モード)を集める。
#[derive(Default)]
struct GridExplore {
    started: bool,
    done: bool,
    /// (状態, リセットからの経路)。先に見つかった(=短い)経路を残す。
    known: Vec<((bool, u32), Vec<u32>)>,
    /// 未実行の探索(経路, 期待する経路後の状態, 押すキー)。`None` = リセット状態そのものの観測。
    queue: std::collections::VecDeque<(Vec<u32>, Option<(bool, u32)>, Option<u32>)>,
    cur: Option<(Vec<u32>, Option<(bool, u32)>, Option<u32>)>,
    phase: u8,
    probes: usize,
    from_state: Option<(bool, u32)>,
}

/// 探索で押すキー(遷移グラフの辺)。テスト対象のモード系・開閉系。
const GRID_EXPLORE_KEYS: [u32; 9] = [0xF2, 0xF1, 0xF0, 0xF3, 0x19, 0x16, 0x1A, 0x1D, 0x1C];
const GRID_EXPLORE_DEPTH: usize = 3;
const GRID_EXPLORE_MAX_PROBES: usize = 130;

fn grid_state_name(st: (bool, u32)) -> String {
    format!("{}-c{:02X}", if st.0 { "on" } else { "off" }, st.1)
}

fn grid_path_name(path: &[u32]) -> String {
    path.iter()
        .map(|&k| grid_key_name(k))
        .collect::<Vec<_>>()
        .join(",")
}

/// 観測から (開閉, 変換モード) を作る。閉じている間は conv が読めなければ `fallback` を使う。
fn grid_state_of(s: &Snapshot, fallback: u32) -> Option<(bool, u32)> {
    let open = grid_open_of(s)?;
    let conv = s.b_conv.or(s.a_conv).unwrap_or(fallback);
    Some((open, conv))
}

/// シャードごとのテスト対象キー。各シャードの所要時間が近くなるように分ける。
fn grid_shard_keys(shard: &str) -> &'static [u32] {
    match shard {
        "s1" => &[0x1D, 0x1C, 0xF2],       // 無変換 変換 ひらがな
        "s2" => &[0xF1, 0xF0, 0x20],       // カタカナ 英数 Space
        "s3" => &[0x1B, 0x0D, 0x08],       // Esc Enter BS
        "s4" => &[0xF3, 0x19, 0x16, 0x1A], // 半角全角 漢字 IME_ON IME_OFF
        _ => &[],
    }
}

/// 試行回数: 入力中・変換中で結果が割れうるキーは4回、開閉系は2回(再現性)。
fn grid_trials_for(key: u32) -> usize {
    match key {
        0xF3 | 0x19 | 0x16 | 0x1A => 2,
        _ => 4,
    }
}

fn grid_key_name(vk: u32) -> &'static str {
    match vk {
        0x1D => "muhenkan",
        0x1C => "henkan",
        0xF2 => "hiragana",
        0xF1 => "katakana",
        0xF0 => "eisu",
        0xF3 => "hankaku-zenkaku",
        0x19 => "kanji",
        0x16 => "ime-on",
        0x1A => "ime-off",
        0x1B => "esc",
        0x0D => "enter",
        0x20 => "space",
        0x08 => "bs",
        _ => "?",
    }
}

const GRID_CONVS: [u32; 7] = [0x19, 0x1B, 0x13, 0x18, 0x10, 0x09, 0x0B];
const GRID_COMPS: [GComp; 5] = [
    GComp::None,
    GComp::Typing,
    GComp::ConvSpace,
    GComp::ConvHenkan,
    GComp::ConvMuhenkan,
];

/// `--grid=SHARD` の試行列を作る。同じセルの繰り返しは時間的に離す(試行番号が外側のループ)。
fn grid_cell_hash(s: &str) -> u64 {
    // FNV-1a。決定性の監査用の標本(10%)を、セル名だけから決める(実行ごとに変わらない)。
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn grid_state_key(open: bool, conv: u32, comp: GComp) -> String {
    format!(
        "{}-c{:02X}-{}",
        if open { "on" } else { "off" },
        conv,
        comp.name()
    )
}

/// `--grid-adaptive`: 1パス目は全セル1回。2パス目は (i)前回の表で非決定だったセル(`retry_cells`)、(ii)履歴依存ブロック、
/// (iii)決定的なセルの `audit_pct`% の標本、だけを再試行する。(ii)のセットアップ不能は実行中に足す(GridRun::retried)。
struct GridAdaptive {
    retry_cells: Vec<String>,
    audit_pct: u64,
}

fn grid_build(
    shard: &str,
    cap: Option<usize>,
    adaptive: Option<&GridAdaptive>,
) -> (Vec<GridTrial>, Option<String>) {
    let keys = grid_shard_keys(shard);
    if adaptive.is_some() && cap.is_some_and(|c| c != 1) {
        append_log("[GRID-ADAPTIVE] 注: --grid-adaptive は --grid-trials を無視して1パス目を各セル1回にする");
    }
    let cap = if adaptive.is_some() { Some(1) } else { cap };
    let mut states: Vec<(bool, u32, GComp)> = Vec::new();
    for &conv in &GRID_CONVS {
        for &comp in &GRID_COMPS {
            states.push((true, conv, comp));
        }
    }
    for &conv in &GRID_CONVS {
        states.push((false, conv, GComp::None));
    }
    let mut v = Vec::new();
    let max_n = keys.iter().map(|&k| grid_trials_for(k)).max().unwrap_or(0);
    for t in 0..max_n {
        for &(open, conv, comp) in &states {
            for &key in keys {
                if t < cap.map_or(usize::MAX, |c| c).min(grid_trials_for(key)) {
                    v.push(GridTrial {
                        open,
                        conv,
                        comp,
                        key,
                        trial: t + 1,
                    });
                }
            }
        }
    }
    // 履歴依存の確認(直前キーが結果を変えるか): 入力中で 直前=ひらがな/Esc → Esc・無変換。
    // 適応モードでも、履歴依存は割れうるので4回のまま(cap の1回制限を受けない)。
    let mut note = None;
    if shard == "s3" {
        let n = if adaptive.is_some() {
            4
        } else {
            cap.map_or(4, |c| c.min(4))
        };
        for t in 0..n {
            for comp in [GComp::PrevHiragana, GComp::PrevEsc] {
                for key in [0x1B_u32, 0x1D] {
                    v.push(GridTrial {
                        open: true,
                        conv: 0x19,
                        comp,
                        key,
                        trial: t + 1,
                    });
                }
            }
        }
    }
    if let Some(a) = adaptive {
        // 2パス目。試行番号が外側のループ(同じセルの繰り返しを時間的に離す)。
        let (mut nondet, mut audit) = (0usize, 0usize);
        let max_n = keys.iter().map(|&k| grid_trials_for(k)).max().unwrap_or(0);
        for t in 1..max_n {
            for &(open, conv, comp) in &states {
                for &key in keys {
                    let cell = format!(
                        "{}|{}",
                        grid_state_key(open, conv, comp),
                        grid_key_name(key)
                    );
                    let is_nondet = a.retry_cells.iter().any(|c| *c == cell);
                    let want = if is_nondet {
                        t < grid_trials_for(key)
                    } else {
                        t == 1 && grid_cell_hash(&cell) % 100 < a.audit_pct
                    };
                    if want {
                        v.push(GridTrial {
                            open,
                            conv,
                            comp,
                            key,
                            trial: t + 1,
                        });
                        if is_nondet {
                            nondet += 1;
                        } else {
                            audit += 1;
                        }
                    }
                }
            }
        }
        note = Some(format!(
            "[GRID-ADAPTIVE] pass1=全セル1回 pass2: 前回の非決定セル{}件の再試行 + 監査標本{}件({}%) + 履歴依存ブロック(s3のみ) / 実行中のセットアップ不能セルは末尾に1回だけ再試行",
            nondet, audit, a.audit_pct
        ));
    }
    (v, note)
}

/// 状態を IMM(WM_IME_CONTROL)で書く。閉じる場合は先に変換モードを書いてから閉じる。
fn grid_set_ime(target: HWND, open: bool, conv: u32) {
    const IMC_SETCONVERSIONMODE: usize = 0x0002;
    const IMC_SETOPENSTATUS: usize = 0x0006;
    unsafe {
        let ime_wnd = ImmGetDefaultIMEWnd(target);
        if ime_wnd.0.is_null() {
            return;
        }
        let mut r: usize = 0;
        let seq: [(usize, isize); 2] = if open {
            [
                (IMC_SETOPENSTATUS, 1),
                (IMC_SETCONVERSIONMODE, conv as isize),
            ]
        } else {
            [
                (IMC_SETCONVERSIONMODE, conv as isize),
                (IMC_SETOPENSTATUS, 0),
            ]
        };
        for (cmd, val) in seq {
            let _ = SendMessageTimeoutW(
                ime_wnd,
                WM_IME_CONTROL,
                WPARAM(cmd),
                LPARAM(val),
                SMTO_ABORTIFHUNG,
                200,
                Some(&raw mut r),
            );
        }
    }
}

/// 第3版のキーのみリセット: IMM を一切使わず、観測しながら IME_ON(0x16)/ひらがな(0xF2) を押して(開,0x19)へ戻す。
/// IMM で書いた 0x19 は GJI の内部状態と食い違い、モード切替キーの結果が実際と違った(第2版で実測)ため、キーで戻す。
/// `Some(true)` = (開,0x19) に到達 / `Some(false)` = 試行上限で失敗 / `None` = 続行(`next` に次の観測時刻を入れる)。
fn grid_key_reset_step(
    tries: &mut u8,
    now: u64,
    hwnd: HWND,
    queue: &mut Vec<(u64, u32)>,
    log: &mut Vec<String>,
    next: &mut u64,
) -> Option<bool> {
    let s = take_snapshot(hwnd);
    let st = grid_state_of(&s, 0x19);
    // 基準 = (開, かな入力)。ATOK は 0x19(ローマ字)、MS-IME プリセットの GJI は自然状態が 0x09(ROMANビット無し)で、
    // ひらがなキーは Set 型なので 0x19 へは行かない(CI実測)。どちらもキーだけで戻せる「ひらがな」状態を基準にする。
    if matches!(st, Some((true, 0x19 | 0x09))) {
        log.push(format!("[GRID-RESET] ok tries={}", *tries));
        *tries = 0;
        return Some(true);
    }
    if *tries >= 6 {
        log.push(format!(
            "[GRID-RESET] 失敗 tries={} 最後の観測={}",
            *tries,
            grid_obs(&s)
        ));
        *tries = 0;
        return Some(false);
    }
    *tries += 1;
    match st {
        // 閉 → IME_ON。効かない(MS-IMEプリセットで閉じた 0x1B から、CI実測)ときは半角/全角(閉なら開く)に切り替える。
        Some((false, _)) => queue.push((now, if *tries % 2 == 1 { 0x16 } else { 0xF3 })),
        Some((true, _)) => queue.push((now, 0xF2)), // 開で 0x19 でない → ひらがな(ATOKはトグル、MS-IMEはSet)
        None => {}
    }
    *next = notify_settle_next(now, now + scaled(1300));
    None
}

fn grid_open_of(s: &Snapshot) -> Option<bool> {
    match (s.a_open, s.b_open) {
        (Some(a), Some(b)) if a == b => Some(a),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        _ => None,
    }
}

/// 目標の状態と一致しているか(閉じている状態は conv を問わない)。
fn grid_matches(s: &Snapshot, t: &GridTrial) -> bool {
    let composing = s.comp.as_deref().is_some_and(|c| !c.is_empty());
    if grid_open_of(s) != Some(t.open) || composing != (t.comp != GComp::None) {
        return false;
    }
    !t.open || s.b_conv.or(s.a_conv) == Some(t.conv)
}

fn grid_obs(s: &Snapshot) -> String {
    format!(
        "open={} conv={} comp={:?} tail={:?}",
        fmt_bool(grid_open_of(s)),
        fmt_hex(s.b_conv.or(s.a_conv)),
        s.comp.as_deref().unwrap_or("?"),
        s.edit_tail
    )
}

/// 探索(BFS)の1tick分。1探索 = 後始末→リセット(IMMで開,0x19)→経路のキー→観測→(キーを1つ押す→観測)。
fn grid_explore_step(
    r: &mut GridRun,
    now: u64,
    hwnd: HWND,
    log: &mut Vec<String>,
    queue: &mut Vec<(u64, u32)>,
    next: &mut u64,
) {
    let target = {
        let f = unsafe { GetFocus() };
        if f.0.is_null() {
            hwnd
        } else {
            f
        }
    };
    let e = &mut r.expl;
    if !e.started {
        e.started = true;
        e.queue.push_back((Vec::new(), None, None));
        log.push(format!(
            "[GRID-EXPLORE] 開始(リセット={}、探索キー9種、深さ<=3)",
            if r.reset_imm {
                "IMMで(開,0x19)=第2版"
            } else {
                "キーのみ=第3版"
            }
        ));
    }
    match e.phase {
        0 => {
            let Some(item) = e.queue.pop_front() else {
                e.done = true;
                let mut known = e.known.clone();
                known.sort_by_key(|(st, _)| *st);
                for (st, path) in &known {
                    log.push(format!(
                        "[GRID-SETUP] state={} path={}",
                        grid_state_name(*st),
                        grid_path_name(path)
                    ));
                }
                for &open in &[true, false] {
                    for &conv in &GRID_CONVS {
                        if !known.iter().any(|(st, _)| *st == (open, conv)) {
                            log.push(format!(
                                "[GRID-SETUP] state={} 到達不能",
                                grid_state_name((open, conv))
                            ));
                        }
                    }
                }
                log.push(format!(
                    "[GRID-EXPLORE] 完了 probes={} 到達状態={}",
                    e.probes,
                    known.len()
                ));
                *next = now + scaled(200);
                return;
            };
            if e.probes >= GRID_EXPLORE_MAX_PROBES {
                e.queue.clear();
                *next = now + scaled(100);
                return;
            }
            e.probes += 1;
            e.cur = Some(item);
            queue.push((now, 0x1B));
            queue.push((now + scaled(400), 0x1B));
            unsafe {
                let _ = SetWindowTextW(target, w!(""));
            }
            *next = now + scaled(1100);
            e.phase = 1;
        }
        1 if !r.reset_imm => {
            // 第3版: リセットもキーのみ。(開,0x19) を観測で確認してから経路のキーを押す。
            match grid_key_reset_step(&mut r.rs_tries, now, hwnd, queue, log, next) {
                None => {}
                Some(true) => {
                    *next = now + scaled(300);
                    e.phase = 2;
                }
                Some(false) => {
                    e.phase = 0;
                    *next = now + scaled(100);
                }
            }
        }
        1 => {
            // 第2版(--grid-setup=keys-immreset): 開始状態を揃えるためだけに IMM で (開, 0x19) へ書く。
            grid_set_ime(target, true, 0x19);
            *next = now + scaled(1000);
            e.phase = 2;
        }
        2 => {
            let path = e.cur.as_ref().map(|c| c.0.clone()).unwrap_or_default();
            for (i, &vk) in path.iter().enumerate() {
                queue.push((now + scaled(350) * i as u64, vk));
            }
            let last_key = now + scaled(350) * (path.len() as u64).saturating_sub(1);
            *next = notify_settle_next(last_key, now + scaled(350 * path.len() as u64 + 900));
            e.phase = 3;
        }
        3 => {
            let s = take_snapshot(hwnd);
            let (path, expect, key) = e.cur.clone().unwrap_or_default();
            let fb = expect.map_or(0x19, |x| x.1);
            let st = grid_state_of(&s, fb);
            let Some(st) = st else {
                log.push(format!(
                    "[GRID-EDGE] path={} 観測不能(A/B不一致)",
                    grid_path_name(&path)
                ));
                e.phase = 0;
                *next = now + scaled(100);
                return;
            };
            match key {
                None => {
                    // リセット状態そのものを登録する。
                    log.push(format!("[GRID-EDGE] reset -> {}", grid_state_name(st)));
                    e.known.push((st, Vec::new()));
                    for &k in &GRID_EXPLORE_KEYS {
                        e.queue.push_back((Vec::new(), Some(st), Some(k)));
                    }
                    e.phase = 0;
                    *next = now + scaled(100);
                }
                Some(k) => {
                    if expect.is_some_and(|x| x != st) {
                        log.push(format!(
                            "[GRID-EDGE] path={} 経路後の状態が不一致 expect={} got={}(この探索は捨てる)",
                            grid_path_name(&path),
                            expect.map_or(String::new(), grid_state_name),
                            grid_state_name(st)
                        ));
                        e.phase = 0;
                        *next = now + scaled(100);
                        return;
                    }
                    e.from_state = Some(st);
                    queue.push((now, k));
                    *next = notify_settle_next(now, now + scaled(1400));
                    e.phase = 4;
                }
            }
        }
        _ => {
            let s = take_snapshot(hwnd);
            let (path, _, key) = e.cur.clone().unwrap_or_default();
            let from = e.from_state.unwrap_or((true, 0x19));
            let k = key.unwrap_or(0);
            if let Some(st2) = grid_state_of(&s, from.1) {
                log.push(format!(
                    "[GRID-EDGE] from={} key={} to={} path={}",
                    grid_state_name(from),
                    grid_key_name(k),
                    grid_state_name(st2),
                    grid_path_name(&path)
                ));
                if !e.known.iter().any(|(x, _)| *x == st2) {
                    let mut np = path.clone();
                    np.push(k);
                    e.known.push((st2, np.clone()));
                    if np.len() < GRID_EXPLORE_DEPTH {
                        for &k2 in &GRID_EXPLORE_KEYS {
                            e.queue.push_back((np.clone(), Some(st2), Some(k2)));
                        }
                    }
                }
            }
            e.phase = 0;
            *next = now + scaled(100);
        }
    }
}

/// `--grid` の1tick分の駆動。1試行 = 後始末→IME状態を書く→入力中の段階を作る→検証×2→キーを押す→観測を待つ。
/// セットアップ不能を記録する。その状態の残りの試行は飛ばす(適応モードでは、最初に失敗したセルだけ末尾へ1回再試行)。
/// セットアップが連続して失敗したら格子を打ち切る(呼び出し側が直後に `r.idx += 1` するので、`idx = len` にしておけば全手順完了へ進む)。
fn grid_setup_failed(r: &mut GridRun, tr: GridTrial, log: &mut Vec<String>) {
    let state = grid_state_key(tr.open, tr.conv, tr.comp);
    let cell = format!("{state}|{}", grid_key_name(tr.key));
    if !r.failed_states.contains(&state) {
        r.failed_states.push(state);
    }
    if r.adaptive && !r.retried.contains(&cell) {
        log.push(format!("[GRID-ADAPTIVE] retry-setup cell={cell}"));
        r.retried.push(cell);
        r.trials.push(GridTrial {
            trial: tr.trial + 1,
            ..tr
        });
    }
    r.consec_fail += 1;
    if r.consec_fail >= GRID_ABORT_CONSEC_FAILS {
        log.push(format!(
            "[GRID-ABORT] reason=セットアップが連続して{}回不能(成功{}回、不能な状態{}個。リセット基準の食い違い等)",
            r.consec_fail,
            r.setup_ok,
            r.failed_states.len()
        ));
        r.idx = r.trials.len();
    }
}

fn grid_drive(now: u64, hwnd: HWND) {
    let target = {
        let f = unsafe { GetFocus() };
        if f.0.is_null() {
            hwnd
        } else {
            f
        }
    };
    let mut next = now + scaled(200);
    let mut finish = false;
    let mut log: Vec<String> = Vec::new();
    let mut press: Option<(u32, String)> = None;
    let mut queue: Vec<(u64, u32)> = Vec::new();
    GRID.with(|g| {
        let mut guard = g.borrow_mut();
        let Some(r) = guard.as_mut() else { return };
        if r.idx >= r.trials.len() {
            finish = true;
            return;
        }
        if let Some(n) = r.note.take() {
            log.push(n);
        }
        if r.keys_mode && !r.expl.done {
            grid_explore_step(r, now, hwnd, &mut log, &mut queue, &mut next);
            return;
        }
        if r.keys_mode && !r.pruned {
            // 探索が終わったら、到達不能な状態の試行を先に外す(各試行が Esc 待ちの約1.1秒を無駄にするため)。
            r.pruned = true;
            let known: Vec<(bool, u32)> = r.expl.known.iter().map(|(st, _)| *st).collect();
            if known.is_empty() {
                log.push("[GRID-ABORT] reason=探索で到達できた状態が0(リセット基準に戻れない等)".to_string());
                r.idx = r.trials.len();
                finish = true;
                return;
            }
            let before = r.trials.len();
            r.trials.retain(|t| known.contains(&(t.open, t.conv)));
            log.push(format!(
                "[GRID-PRUNE] 到達不能な状態の試行を外した {before}→{} (到達状態={})",
                r.trials.len(),
                known.len()
            ));
            if r.idx >= r.trials.len() {
                finish = true;
                return;
            }
        }
        let t = &r.trials[r.idx];
        let id = format!("G{:04}", r.idx + 1);
        let state = format!(
            "{}-{}-{}",
            if t.open { "on" } else { "off" },
            format_args!("c{:02X}", t.conv),
            t.comp.name()
        );
        if r.phase == 0 && r.attempt == 0 {
            let st_key = grid_state_key(t.open, t.conv, t.comp);
            let cell = format!("{st_key}|{}", grid_key_name(t.key));
            if r.failed_states.contains(&st_key) && !r.retried.contains(&cell) {
                log.push(format!(
                    "[GRID-SKIP] id={id} state={state} key={} trial={} 同じ状態で既にセットアップ不能(飛ばす)",
                    grid_key_name(t.key),
                    t.trial
                ));
                r.idx += 1;
                next = now + scaled(50);
                return;
            }
        }
        match r.phase {
            0 => {
                if r.attempt == 0 {
                    log.push(format!(
                        "[GRID-BEGIN] id={id} shard={} n={}/{} state={state} key={} trial={}",
                        r.shard,
                        r.idx + 1,
                        r.trials.len(),
                        grid_key_name(t.key),
                        t.trial
                    ));
                }
                // 未確定を空にし、入力欄のテキストも空にする(確定の判定を tail の変化で見るため)。
                queue.push((now, 0x1B));
                queue.push((now + scaled(400), 0x1B));
                unsafe {
                    let _ = SetWindowTextW(target, w!(""));
                }
                next = now + scaled(1100);
                r.phase = 1;
            }
            1 if r.keys_mode => {
                // キーで到達: リセット(IME_OFF→IME_ON)のあと、探索で見つけた経路のキーを押す。
                let path = r
                    .expl
                    .known
                    .iter()
                    .find(|(st, _)| *st == (t.open, t.conv))
                    .map(|(_, p)| p.clone());
                if let Some(path) = path {
                    let mut base = scaled(1000);
                    let mut go = true;
                    if r.reset_imm {
                        grid_set_ime(target, true, 0x19); // 第2版のリセット(探索と同じ開始状態)
                    } else {
                        match grid_key_reset_step(&mut r.rs_tries, now, hwnd, &mut queue, &mut log, &mut next) {
                            None => go = false, // 観測を続ける(次の tick も phase 1)
                            Some(true) => base = scaled(300),
                            Some(false) => {
                                log.push(format!(
                                    "[GRID-SKIP] id={id} state={state} key={} trial={} セットアップ不能(キーのみのリセットで(開,0x19)に戻れない)",
                                    grid_key_name(t.key),
                                    t.trial
                                ));
                                {
                                    let tc = t.clone();
                                    grid_setup_failed(r, tc, &mut log);
                                }
                                r.idx += 1;
                                r.attempt = 0;
                                r.phase = 0;
                                go = false;
                            }
                        }
                    }
                    if go {
                        for (i, &vk) in path.iter().enumerate() {
                            queue.push((now + base + scaled(350) * i as u64, vk));
                        }
                        let last_key = now + base + scaled(350) * (path.len() as u64).saturating_sub(1);
                        next = notify_settle_next(last_key, now + base + scaled(350 * path.len() as u64 + 700));
                        r.phase = 2;
                    }
                } else {
                    log.push(format!(
                        "[GRID-SKIP] id={id} state={state} key={} trial={} 到達不能(キーで到達する経路が無い)",
                        grid_key_name(t.key),
                        t.trial
                    ));
                    r.idx += 1;
                    r.attempt = 0;
                    r.phase = 0;
                }
            }
            1 => {
                grid_set_ime(target, t.open, t.conv);
                next = now + scaled(600);
                r.phase = 2;
            }
            2 => {
                if !t.open {
                    next = now + scaled(300);
                } else {
                    let ks = t.comp.keys();
                    for (i, &vk) in ks.iter().enumerate() {
                        queue.push((now + scaled(350) * i as u64, vk));
                    }
                    // 入力中/変換中の生成は compartment 通知が来ない(冒頭docのとおり固定待ち)ので notify_settle_next は使わない。
                    // 使うと待ちが最後のキー+150ms に縮み、GJI の未確定文字列生成が間に合わないセルが [GRID-PRE] 不合格→SKIP で表から消える。
                    next = now + scaled(350 * ks.len() as u64 + 700);
                }
                r.phase = 3;
            }
            3 => {
                let s = take_snapshot(hwnd);
                r.verify1_ok = grid_matches(&s, t);
                log.push(format!(
                    "[GRID-PRE] id={id} check=1 attempt={} ok={} {}",
                    r.attempt + 1,
                    r.verify1_ok,
                    grid_obs(&s)
                ));
                next = now + scaled(300);
                r.phase = 4;
            }
            4 => {
                let s = take_snapshot(hwnd);
                let ok = r.verify1_ok && grid_matches(&s, t);
                log.push(format!(
                    "[GRID-PRE] id={id} check=2 attempt={} ok={ok} {}",
                    r.attempt + 1,
                    grid_obs(&s)
                ));
                if ok {
                    r.setup_ok += 1;
                    r.consec_fail = 0;
                    press = Some((
                        t.key,
                        format!(
                            "[GRID id={id} shard={} state={state} key={} trial={}]",
                            r.shard,
                            grid_key_name(t.key),
                            t.trial
                        ),
                    ));
                    let cap = now + if FAST_MODE.with(|f| *f.borrow()) { 700 } else { 2300 }; // --fast は +1500ms の観測を省くので +400ms + 余裕まで待てばよい // +1500ms の観測が出揃うまで
                    // --snap100 --notify: +100ms の観測だけを取り、通知(compartment/composition/EN_CHANGE)が静かになるまで(上限は cap)。
                    next = if SNAP100.with(|f| *f.borrow()) && NOTIFY_MODE.with(|m| *m.borrow()) {
                        notify_settle_next(now.max(now + 100), cap)
                    } else {
                        cap
                    };
                    r.phase = 5;
                } else if r.attempt < 1 {
                    r.attempt += 1;
                    r.phase = 0;
                } else {
                    log.push(format!(
                        "[GRID-SKIP] id={id} state={state} key={} trial={} セットアップ不能",
                        grid_key_name(t.key),
                        t.trial
                    ));
                    let tc = t.clone();
                    grid_setup_failed(r, tc, &mut log);
                    r.idx += 1;
                    r.attempt = 0;
                    r.phase = 0;
                }
            }
            _ => {
                r.idx += 1;
                r.attempt = 0;
                r.phase = 0;
                next = now + scaled(100);
            }
        }
    });
    for l in log {
        append_log(&l);
    }
    for (at, vk) in queue {
        queue_press(at, vk);
    }
    if let Some((vk, tag)) = press {
        GRID_TAG.with(|g| *g.borrow_mut() = Some((vk, tag)));
        queue_press(now, vk);
    }
    if finish {
        if !AUTO_DONE.with(|d| std::mem::replace(&mut *d.borrow_mut(), true)) {
            if NOTIFY_MODE.with(|m| *m.borrow()) {
                let st = NOTIFY_STATS.with(|s| *s.borrow());
                append_log(&format!(
                    "[NOTIFY-STATS] 通知で早く終わった待ち={} 通知が来ず変化なしで終わった待ち={} 上限まで待った待ち={}",
                    st[0], st[1], st[2]
                ));
            }
            append_log("[GRID] 全手順完了（1.5秒後に自動で閉じます）");
            AUTO_CLOSE_AT.with(|c| *c.borrow_mut() = now + 1500);
        }
        return;
    }
    AUTO_NEXT.with(|n| *n.borrow_mut() = next);
}

/// `--walk` の手順: プリセット(ATOK/MS-IME等)を問わず、前提状態を要求せずに固定のキー列を押す。
/// 各押下の +1500ms の実IME状態と awase の Engine 状態が一致するか(check_consistency.py)を見る。
/// 半角/全角(0xF3/0xF4)は awase のモデル誤り(ADR-186決定5、別件)が混ざるため含めない。
const WALK: [(&str, u32, bool, &str, St); 12] = [
    ("ひらがなキー", 0xF2, false, "Engine は実IMEに追随", St::Any),
    ("無変換", 0x1D, false, "Engine は実IMEに追随", St::Any),
    ("無変換", 0x1D, false, "Engine は実IMEに追随", St::Any),
    ("変換", 0x1C, false, "Engine は実IMEに追随", St::Any),
    ("変換", 0x1C, false, "Engine は実IMEに追随", St::Any),
    ("ひらがなキー", 0xF2, false, "Engine は実IMEに追随", St::Any),
    ("無変換", 0x1D, false, "Engine は実IMEに追随", St::Any),
    ("ひらがなキー", 0xF2, false, "Engine は実IMEに追随", St::Any),
    ("変換", 0x1C, false, "Engine は実IMEに追随", St::Any),
    ("無変換", 0x1D, false, "Engine は実IMEに追随", St::Any),
    ("変換", 0x1C, false, "Engine は実IMEに追随", St::Any),
    ("ひらがなキー", 0xF2, false, "Engine は実IMEに追随", St::Any),
];

/// `--vkprobe` の候補。`SCAN_ONLY | scan` = スキャンコードだけ、`NOSCAN | vk` = VKだけ、素の値 = VK+標準スキャン。
const VKPROBE_CANDIDATES: [u32; 17] = [
    SCAN_ONLY | 0x70, // ひらがな(カタカナひらがなローマ字)キーの物理スキャンコード
    0x15,             // VK_KANA
    0xF1,             // VK_DBE_KATAKANA
    0xF2,             // VK_DBE_HIRAGANA
    NOSCAN | 0xF2,    // VK_DBE_HIRAGANA、スキャンコード0
    0xF5,             // VK_DBE_ROMAN
    0xF6,             // VK_DBE_NOROMAN
    SCAN_ONLY | 0x29, // 半角/全角の物理スキャンコード
    0xF3,             // VK_DBE_SBCSCHAR
    0xF4,             // VK_DBE_DBCSCHAR
    0x19,             // VK_KANJI
    SCAN_ONLY | 0x3A, // 英数の物理スキャンコード
    0xF0,             // VK_DBE_ALPHANUMERIC
    NOSCAN | 0x16,    // VK_IME_ON
    SCAN_ONLY | 0x79, // 変換の物理スキャンコード
    SCAN_ONLY | 0x7B, // 無変換の物理スキャンコード
    0x1C,             // VK_CONVERT
];

/// `--seq=F2,A0,A0,...` で指定した任意のキー列(VKの16進、`0x`は省略可)。前提状態なしで押し、各押下の実IMEと
/// Engine の一致を check_consistency.py で見る(ワークフローから、コードを変えずに手順を足せる)。
static SEQ_TABLE: std::sync::OnceLock<Vec<(&'static str, u32, bool, &'static str, St)>> =
    std::sync::OnceLock::new();

fn parse_seq(arg: &str) -> Vec<(&'static str, u32, bool, &'static str, St)> {
    arg.split(',')
        .map(|t| {
            // 打ち間違いを黙って捨てると手順が短くなり、それでもPASSしうる。即エラーにする。
            u32::from_str_radix(t.trim().trim_start_matches("0x"), 16).unwrap_or_else(|_| {
                arg_error(&format!("--seq のVKが16進数でない: {t:?} (全体: {arg:?})"))
            })
        })
        .map(|vk| {
            (
                key_name(vk).unwrap_or("キー"),
                vk,
                false,
                "Engine は実IMEに追随",
                St::Any,
            )
        })
        .collect()
}

/// 現在の手順表(`--seq` ならその列、`--walk` なら WALK、なければ SCRIPT)。
fn script() -> &'static [(&'static str, u32, bool, &'static str, St)] {
    if let Some(seq) = SEQ_TABLE.get() {
        seq
    } else if HZ_MODE.with(|h| *h.borrow()) {
        &HZ
    } else if RESYNC_MODE.with(|r| *r.borrow()) {
        &RESYNC
    } else if WALK_MODE.with(|w| *w.borrow()) {
        if COLD_MODE.with(|c| *c.borrow()) {
            &WALK[1..]
        } else {
            &WALK
        }
    } else {
        &SCRIPT
    }
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
                } else if let Some(name) = key_name(vk)
                    .or_else(|| VKPROBE_MODE.with(|m| *m.borrow()).then_some("不明キー"))
                {
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
    let hook_at = HOOK_AT.with(|h| *h.borrow());
    if hook_at != 0 && now >= hook_at {
        HOOK_AT.with(|h| *h.borrow_mut() = 0);
        match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) } {
            Ok(_) => append_log("[init] キーフックを遅延インストール(awaseより後=先に呼ばれる)"),
            Err(e) => append_log(&format!("[init] キーフック失敗: {e}")),
        }
    }

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
                if si < script().len() && now >= HOLD_UNTIL.with(|h| *h.borrow()) {
                    let (name, vk, shift, expect, need) = script()[si];
                    let (want_vk, want_ctrl) = step_first_key(vk);
                    let shift_muh = SHIFT_MUH.with(|m| *m.borrow()) && ev.vk == 0x1D;
                    if ev.vk == want_vk
                        && (need == St::Any || before_st == need)
                        && (ev.shift == shift
                            || (shift_muh && ev.shift)
                            || matches!(want_vk, 0xA0 | 0xA1))
                        && ev.ctrl == want_ctrl
                        && !ev.label.contains("(injected)")
                    {
                        tag = format!(
                            "[SCRIPT {}/{} {name} 期待={expect}]",
                            si + 1,
                            script().len()
                        );
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
            if let Some((gvk, gtag)) = GRID_TAG.with(|g| g.borrow().clone()) {
                if ev.vk == gvk {
                    tag = gtag;
                    GRID_TAG.with(|g| *g.borrow_mut() = None);
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
        if si >= script().len() {
            "全手順完了です。お疲れさまでした（ログは自動保存済み）".to_string()
        } else {
            let (name, _, _, expect, need) = script()[si];
            let action = if now < hold {
                format!("待機中… あと {:.1} 秒", (hold - now) as f64 / 1000.0)
            } else if need != St::Any && cur != need {
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
                script().len(),
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
            0x0111 /* WM_COMMAND */ if NOTIFY_COMP.load(std::sync::atomic::Ordering::Relaxed) => {
                // 上位ワード=通知コード。EN_CHANGE=0x0300, EN_UPDATE=0x0400。
                let code = (wparam.0 >> 16) & 0xFFFF;
                // ログ欄(同じ EDIT 系の子窓)の更新は除き、打鍵先の入力欄(lparam=その HWND)だけを記録する。
                let from_input = EDIT_HWND.with(|e| e.borrow().is_some_and(|h| h.0 as isize == lparam.0));
                if from_input && (code == 0x0300 || code == 0x0400) {
                    append_log(&format!("[COMP] {} t={}", if code == 0x0300 { "EN_CHANGE" } else { "EN_UPDATE" }, now_ms()));
                    NOTIFY_LAST.with(|n| *n.borrow_mut() = now_ms());
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
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
        if NOTIFY_COMP.load(std::sync::atomic::Ordering::Relaxed) {
            // SAFETY: edit は作成したばかりの自スレッドの子窓。元のプロシージャを保持して、ログしてから転送する。
            let orig =
                unsafe { SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_sub_proc as usize as isize) };
            EDIT_ORIG_PROC.store(orig, std::sync::atomic::Ordering::Relaxed);
            append_log("[COMP] Edit をサブクラス化(WM_IME_* を記録)");
        }
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
    // 既定は GJI(Mozc)のCLSIDとプロファイルGUID、日本語(0x0411)。`--msime` なら Microsoft IME(日本語)。
    let (clsid, profile) = if std::env::args().any(|a| a == "--msime") {
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
                append_log(&format!("[init] IMEプロファイルをアクティブ化: {r:?}"));
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
        if NOTIFY_MODE.with(|m| *m.borrow()) {
            if let Some(c) = thread_cmgr.as_ref() {
                let sinks = notify_sink::advise(c);
                append_log(&format!(
                    "[NOTIFY] 購読 {}件(OPENCLOSE/CONVERSION/SENTENCE)",
                    sinks.len()
                ));
                NOTIFY_SINKS.with(|v| *v.borrow_mut() = sinks);
            }
        }
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
    // ログには必ず残す。--auto(CI・自動実行)では MessageBoxW のモーダルで止めない(誰も閉じられず wait を使い切って rc が誤る)。
    let _ = std::panic::catch_unwind(|| append_log(&format!("[FATAL] {msg}")));
    if std::env::args().any(|a| a == "--auto") {
        return;
    }
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

/// 引数の誤りをログに残して終了する(rc は 2)。CI では完走マーカーが無いので collect の判定が INVALID(rc=3)になる。
fn arg_error(msg: &str) -> ! {
    append_log(&format!("[FATAL] 引数エラー: {msg}"));
    eprintln!("ime_key_matrix_spike: 引数エラー: {msg}");
    std::process::exit(2);
}

/// 起動時に引数を検証する。未知の `--` 引数は警告(綴り間違いを黙って無視しない)、値が不正なものは終了する
/// (`--grid-setup=` の綴り間違いが「学習に使ってはいけない」IMM 版へ、`--grid=s5` が0試行の緑へ、無言で化けるのを防ぐ)。
fn validate_args() {
    const FLAGS: &[&str] = &[
        "--activate-gji",
        "--auto",
        "--cold",
        "--diag",
        "--fast",
        "--free",
        "--grid-adaptive",
        "--hz",
        "--msime",
        "--notify",
        "--notify-comp",
        "--resync",
        "--round2",
        "--script",
        "--shiftmuh",
        "--snap100",
        "--vkprobe",
        "--walk",
        "--key=henkan",
    ];
    const VALUE_FLAGS: &[&str] = &[
        "--hold=",
        "--repeat=",
        "--speed=",
        "--notify-quiet=",
        "--notify-nochg=",
        "--grid=",
        "--grid-trials=",
        "--grid-setup=",
        "--grid-retry-file=",
        "--grid-audit-pct=",
        "--walk=",
        "--seed=",
        "--seq=",
        "--charthumb=",
        "--chord=",
        "--chord-at=",
        "--chord-prep=",
        "--resync-gap=",
    ];
    for a in std::env::args().skip(1) {
        if !a.starts_with("--") || FLAGS.contains(&a.as_str()) {
            continue;
        }
        let Some(&pre) = VALUE_FLAGS.iter().find(|p| a.starts_with(**p)) else {
            append_log(&format!("[init] 警告: 未知の引数を無視した: {a}"));
            continue;
        };
        let v = &a[pre.len()..];
        match pre {
            "--grid=" if !matches!(v, "s1" | "s2" | "s3" | "s4") => {
                arg_error(&format!("--grid= は s1..s4 のいずれか: {a}"))
            }
            "--grid-setup=" if !matches!(v, "keys" | "keys-immreset" | "imm") => arg_error(
                &format!("--grid-setup= は keys / keys-immreset / imm のいずれか: {a}"),
            ),
            "--hold=" | "--chord-at=" | "--repeat=" | "--speed=" | "--notify-quiet="
            | "--notify-nochg=" | "--grid-trials=" | "--grid-audit-pct=" | "--walk="
            | "--seed=" | "--resync-gap="
                if v.parse::<u64>().is_err() =>
            {
                arg_error(&format!("数値でない値: {a}"))
            }
            "--grid-retry-file=" if std::fs::read_to_string(v).is_err() => {
                arg_error(&format!("--grid-retry-file を読めない: {a}"))
            }
            _ => {}
        }
    }
}

fn run() -> WinResult<()> {
    START.with(|s| *s.borrow_mut() = Some(std::time::Instant::now()));
    validate_args();
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
        if a == "--snap100" {
            SNAP100.with(|f| *f.borrow_mut() = true);
        }
        if a == "--notify-comp" {
            NOTIFY_COMP.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if a == "--notify" {
            NOTIFY_MODE.with(|f| *f.borrow_mut() = true);
        }
        if let Some(v) = a.strip_prefix("--notify-quiet=") {
            if let Ok(n) = v.parse::<u64>() {
                NOTIFY_QUIET_MS.with(|r| *r.borrow_mut() = n);
            }
        }
        if let Some(v) = a.strip_prefix("--notify-nochg=") {
            if let Ok(n) = v.parse::<u64>() {
                NOTIFY_NOCHG_MS.with(|r| *r.borrow_mut() = n);
            }
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
        SCRIPT_IDX.with(|i| *i.borrow_mut() = script().len());
        let base = now_ms() + 9000;
        for (i, vk) in [0x1C_u32, 0xF4, 0xF3, 0xF2, 0x16, 0x19, 0x1D]
            .iter()
            .enumerate()
        {
            queue_press(base + (i as u64) * 3500, *vk);
        }
    }
    {
        let cap = std::env::args().find_map(|a| {
            a.strip_prefix("--grid-trials=")
                .and_then(|v| v.parse::<usize>().ok())
        });
        if let Some(shard) =
            std::env::args().find_map(|a| a.strip_prefix("--grid=").map(str::to_string))
        {
            let adaptive = std::env::args()
                .any(|a| a == "--grid-adaptive")
                .then(|| GridAdaptive {
                    retry_cells: std::env::args()
                        .find_map(|a| a.strip_prefix("--grid-retry-file=").map(str::to_string))
                        .and_then(|p| std::fs::read_to_string(p).ok())
                        .map(|t| {
                            t.lines()
                                .map(str::trim)
                                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    audit_pct: std::env::args()
                        .find_map(|a| {
                            a.strip_prefix("--grid-audit-pct=")
                                .and_then(|v| v.parse::<u64>().ok())
                        })
                        .unwrap_or(10),
                });
            let (trials, note) = grid_build(&shard, cap, adaptive.as_ref());
            GRID.with(|g| {
                *g.borrow_mut() = Some(GridRun {
                    shard,
                    trials,
                    adaptive: adaptive.is_some(),
                    retried: Vec::new(),
                    failed_states: Vec::new(),
                    setup_ok: 0,
                    consec_fail: 0,
                    pruned: false,
                    note,
                    idx: 0,
                    phase: 0,
                    attempt: 0,
                    verify1_ok: false,
                    keys_mode: std::env::args()
                        .any(|a| a == "--grid-setup=keys" || a == "--grid-setup=keys-immreset"),
                    reset_imm: std::env::args().any(|a| a == "--grid-setup=keys-immreset"),
                    rs_tries: 0,
                    expl: GridExplore::default(),
                });
            });
        }
    }
    for a in std::env::args() {
        if let Some(v) = a.strip_prefix("--walk=") {
            if let Ok(n) = v.parse::<usize>() {
                WALK_N.with(|w| *w.borrow_mut() = n);
            }
        }
        if let Some(v) = a.strip_prefix("--seed=") {
            if let Ok(n) = v.parse::<u64>() {
                WALK_RNG.with(|w| *w.borrow_mut() = n);
            }
        }
    }
    if std::env::args().any(|a| a == "--cold") {
        COLD_MODE.with(|c| *c.borrow_mut() = true);
    }
    if let Some(v) = std::env::args().find_map(|a| a.strip_prefix("--seq=").map(str::to_owned)) {
        let _ = SEQ_TABLE.set(parse_seq(&v));
    }
    if std::env::args().any(|a| a == "--hz") {
        HZ_MODE.with(|h| *h.borrow_mut() = true);
    }
    if std::env::args().any(|a| a == "--resync") {
        RESYNC_MODE.with(|r| *r.borrow_mut() = true);
    }
    for a in std::env::args() {
        if let Some(v) = a.strip_prefix("--resync-gap=") {
            if let Ok(n) = v.parse::<u64>() {
                RESYNC_GAP_MS.with(|g| *g.borrow_mut() = n);
            }
        }
    }
    // `--walk`: --auto の手順を、前提状態なしの固定キー列(WALK)にする。
    if std::env::args().any(|a| a == "--walk") {
        WALK_MODE.with(|w| *w.borrow_mut() = true);
    }
    // `--vkprobe`: ひらがな系キーの「正しいVK」を調べる。候補キーごとに、IME OFF→候補、IME ON→候補 を押し、
    // 実IMEの変化を記録する(スキャンコードだけの注入で、OSがどのVKに変換するかも見る)。
    if std::env::args().any(|a| a == "--vkprobe") {
        AUTO_MODE.with(|m| *m.borrow_mut() = true);
        SCRIPT_MODE.with(|m| *m.borrow_mut() = true);
        VKPROBE_MODE.with(|m| *m.borrow_mut() = true);
        STEP_IDX.with(|i| *i.borrow_mut() = steps().len() * ROUNDS);
        SCRIPT_IDX.with(|i| *i.borrow_mut() = script().len());
        let base = now_ms() + 9000;
        for (i, cand) in VKPROBE_CANDIDATES.iter().enumerate() {
            let t = base + (i as u64) * 14000;
            queue_press(t, 0x1A);
            queue_press(t + 3500, *cand);
            queue_press(t + 7000, 0x16);
            queue_press(t + 10500, *cand);
        }
    }
    // `--chord=VK1,VK2`: VK1を押しっぱなしにした状態でVK2をタップし、VK1を離す
    // (「本物の同時押し」)。`--seq`は各VKを逐次タップするだけで重なりが無く、
    // Ctrl等のOS修飾キーを保持したままの物理チョード(例: Ctrl+変換)を再現できない
    // 制約があった(2026-09-22調査)。`send_key`は`--seq`/`--walk`等と同じ
    // `AUTO_MARKER`(=`TEST_INJECTION_MARKER`)を使うため、`AWASE_TEST_INJECTION=1`の
    // awaseからは他の注入と同じく物理キーとして扱われる
    // (`HOOK_STATE.physical_key_state`もVK1の保持中は更新される。`hook.rs`の
    // `is_test_injection`はVKを区別しないため、Ctrl等の修飾キーにも同じ扱いが及ぶ)。
    if let Some(v) = std::env::args().find_map(|a| a.strip_prefix("--chord=").map(str::to_owned)) {
        let vks: Vec<u32> = v
            .split(',')
            .map(|t| {
                u32::from_str_radix(t.trim().trim_start_matches("0x"), 16).unwrap_or_else(|_| {
                    arg_error(&format!("--chord のVKが16進数でない: {t:?} (全体: {v:?})"))
                })
            })
            .collect();
        let [vk_hold, vk_tap] = vks[..] else {
            arg_error("--chord=VK1,VK2 の形式で2つのVKを指定してください(VK1=保持するキー、VK2=タップするキー)");
        };
        AUTO_MODE.with(|m| *m.borrow_mut() = true);
        SCRIPT_MODE.with(|m| *m.borrow_mut() = true);
        STEP_IDX.with(|i| *i.borrow_mut() = steps().len() * ROUNDS);
        SCRIPT_IDX.with(|i| *i.borrow_mut() = script().len());
        // `--chord-at=MS`: 開始までの待ち(既定3000)。`--activate-gji` の準備(約12秒)のあとに始めるには 20000 程度を指定する。
        // `--chord-prep=VK`: チョードの2.5秒前に注入する準備キー(例: 変換=1C で IME を開いてから Alt+半角/全角、ADR-199 T1(b))。
        let arg_u64 = |name: &str| {
            std::env::args().find_map(|a| a.strip_prefix(name).and_then(|v| v.parse::<u64>().ok()))
        };
        let mut base = now_ms() + arg_u64("--chord-at=").unwrap_or(3000);
        if let Some(prep) = std::env::args().find_map(|a| {
            a.strip_prefix("--chord-prep=")
                .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        }) {
            queue_press(base, prep);
            base += 2500;
        }
        const OVERLAP_MS: u64 = 150; // VK1押下からVK2タップ開始までの重なり
        const TAP_MS: u64 = 60; // VK2の保持時間
        const RELEASE_GAP_MS: u64 = 100; // VK2解放からVK1解放までの猶予
        AUTO_QUEUE.with(|q| {
            let mut q = q.borrow_mut();
            q.push((base, vk_hold, true));
            q.push((base + OVERLAP_MS, vk_tap, true));
            q.push((base + OVERLAP_MS + TAP_MS, vk_tap, false));
            q.push((base + OVERLAP_MS + TAP_MS + RELEASE_GAP_MS, vk_hold, false));
        });
    }
    // `--charthumb=CHAR,THUMB`(ADR-199 T10 決定A): 文字→親指の順に押し、文字を親指より先に離して重なり不足
    // (`min_overlap_margin_percent`>0 の設定で `PendingCharThumb` が同時打鍵と確定しない)にしたまま、親指を
    // 押し続けてタイムアウト(既定100ms)を越えさせ、その後で親指を離す。親指を押している間に awase が IME を
    // 動かしていないか(親指 KEY 行の +400ms の実IME開閉)と、離した後に動くか(+1500ms)を check_charthumb.py が見る。
    // 各ラウンドの頭に VK_IME_ON(0x16)を注入して IME を ON にそろえる(3ラウンド)。ラウンドの予約は `auto_drive` が、
    // 前面化・フォーカス確認のあとで行う(先にキューへ積むと、フォーカスが外れた窓へ注入が届いて IME が ON にならない)。
    if let Some(v) =
        std::env::args().find_map(|a| a.strip_prefix("--charthumb=").map(str::to_owned))
    {
        let vks: Vec<u32> = v
            .split(',')
            .map(|t| {
                u32::from_str_radix(t.trim().trim_start_matches("0x"), 16).unwrap_or_else(|_| {
                    arg_error(&format!(
                        "--charthumb のVKが16進数でない: {t:?} (全体: {v:?})"
                    ))
                })
            })
            .collect();
        let [vk_char, vk_thumb] = vks[..] else {
            arg_error("--charthumb=CHAR,THUMB の形式で2つのVKを指定してください");
        };
        AUTO_MODE.with(|m| *m.borrow_mut() = true);
        SCRIPT_MODE.with(|m| *m.borrow_mut() = true);
        STEP_IDX.with(|i| *i.borrow_mut() = steps().len() * ROUNDS);
        SCRIPT_IDX.with(|i| *i.borrow_mut() = script().len());
        CHARTHUMB.with(|c| *c.borrow_mut() = Some((vk_char, vk_thumb, 3)));
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
        AUTO_NEXT.with(|n| *n.borrow_mut() = now_ms() + 14000);
        // awase の belief(起動時の推定=ON)と実状態(新しい窓=OFF)がずれたままだと、最初の押下で awase が逆向きに
        // actuate して手順が崩れる(CI run 35482240969)。手順の前に VK_IME_OFF を1回注入して、belief も実状態も
        // OFF にそろえる(awase 起動中は物理IMEキーとして belief を更新する。awase なしでも無害)。
        queue_press(now_ms() + 12000, 0x1A);
        HOOK_AT.with(|h| *h.borrow_mut() = now_ms() + 6000);
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

    let hook = if HOOK_AT.with(|h| *h.borrow()) == 0 {
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) }
    } else {
        // 遅延インストール(on_timer で張る)。
        Ok(Default::default())
    };
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
    teardown_tsf();
    Ok(())
}

/// メッセージループ終了後に、compartment の購読を解除(UnadviseSink)し、ITfThreadMgr を Deactivate する。
/// 購読を生かしたまま TLS 破棄で ITfThreadMgr が Release されると、解体中に OnChange が来て破棄済みの TLS を触りうる。
fn teardown_tsf() {
    let sinks = NOTIFY_SINKS.with(|v| std::mem::take(&mut *v.borrow_mut()));
    for (source, cookie, _sink) in sinks {
        // SAFETY: source はメインスレッド(STA)で AdviseSink した ITfSource、cookie はその戻り値。
        let _ = unsafe { source.UnadviseSink(cookie) };
    }
    if let Some(st) = TSF_STATE.with(|s| s.borrow_mut().take()) {
        // SAFETY: Activate したメインスレッドで、ループ終了後に1回だけ呼ぶ。
        let _ = unsafe { st._thread_mgr.Deactivate() };
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        report_fatal(&format!("panic: {info}"));
    }));
    if let Err(e) = run() {
        report_fatal(&format!("error: {e}"));
    }
}
