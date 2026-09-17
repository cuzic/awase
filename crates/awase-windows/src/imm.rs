#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! IMM32 (Input Method Manager) 低レベルユーティリティ。
//!
//! IME 制御定数・RAII コンテキストガード・クロスプロセスクエリヘルパーを一元管理する。
//! `ime.rs` / `ime_diagnostic.rs` / `observer/ime_observer.rs` に分散していた重複を集約。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmGetDefaultIMEWnd, ImmReleaseContext, HIMC};
use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, SMTO_ABORTIFHUNG};

// ─── IME 制御メッセージ・定数 ────────────────────────────────────

pub(crate) const WM_IME_CONTROL: u32 = 0x0283;
pub(crate) const IMC_GETOPENSTATUS: usize = 0x0005;
pub(crate) const IMC_SETOPENSTATUS: usize = 0x0006;
pub(crate) const IMC_GETCONVERSIONMODE: usize = 0x0001;
pub(crate) const IMC_SETCONVERSIONMODE: usize = 0x0002;

/// ローマ字入力モードフラグ（0x0010）
pub(crate) const IME_CMODE_ROMAN: u32 = 0x0010;
/// 日本語ネイティブ入力モードフラグ（0x0001）
pub(crate) const IME_CMODE_NATIVE: u32 = 0x0001;
/// カタカナ入力モードフラグ（0x0002）
pub(crate) const IME_CMODE_KATAKANA: u32 = 0x0002;
/// 全角モードフラグ（0x0008）
pub(crate) const IME_CMODE_FULLSHAPE: u32 = 0x0008;

// ─── GCS_* (ImmGetCompositionStringW のインデックス) ────────────────
//
// MSDN: <https://learn.microsoft.com/en-us/windows/win32/api/imm/nf-imm-immgetcompositionstringw>

/// GCS_COMPREADSTR: composition の読み（ローマ字相当）
pub(crate) const GCS_COMPREADSTR: u32 = 0x0001;
/// GCS_COMPSTR: 現在 composition 中の文字列
pub(crate) const GCS_COMPSTR: u32 = 0x0008;
/// GCS_COMPATTR: 各文字の属性（0=入力 / 1=変換中 / 2=変換済 / 3=固定）
pub(crate) const GCS_COMPATTR: u32 = 0x0010;
/// GCS_CURSORPOS: カーソル位置
pub(crate) const GCS_CURSORPOS: u32 = 0x0080;
/// GCS_RESULTREADSTR: 確定済みの読み
pub(crate) const GCS_RESULTREADSTR: u32 = 0x0200;
/// GCS_RESULTSTR: 確定済みの文字列
pub(crate) const GCS_RESULTSTR: u32 = 0x0800;

/// HKL (`HKEYBOARDLAYOUT`) の下位 16bit から `LANGID` を抽出する。
#[must_use]
pub(crate) const fn lang_id_from_hkl(hkl: u32) -> u32 {
    hkl & 0xFFFF
}

/// IME 変換モード生値が指定フラグを含むかどうかを返す（診断ログ等で使う）。
#[must_use]
pub(crate) const fn cmode_has(mode: u32, flag: u32) -> bool {
    mode & flag != 0
}

// ─── RAII コンテキストガード ─────────────────────────────────────

/// `ImmGetContext` / `ImmReleaseContext` の RAII ガード。
///
/// `new()` で取得し、`Drop` で自動リリースする。
/// `himc.is_invalid()` の場合は `None` を返す。
pub(crate) struct ImmContextGuard {
    hwnd: HWND,
    himc: HIMC,
}

impl ImmContextGuard {
    /// # Safety
    /// `hwnd` は有効なウィンドウハンドルでなければならない。
    pub(crate) unsafe fn new(hwnd: HWND) -> Option<Self> {
        // SAFETY: hwnd は呼出元でチェック済みの有効なウィンドウハンドル。
        //         ImmReleaseContext は Drop で必ず呼ばれる RAII パターン。
        let himc = unsafe { ImmGetContext(hwnd) };
        if himc.is_invalid() {
            None
        } else {
            Some(Self { hwnd, himc })
        }
    }

    pub(crate) const fn himc(&self) -> HIMC {
        self.himc
    }
}

impl Drop for ImmContextGuard {
    fn drop(&mut self) {
        // SAFETY: self.hwnd と self.himc は new() で ImmGetContext が返した有効なペア。
        //         ImmReleaseContext は ImmGetContext と必ず対になる RAII パターン。
        unsafe {
            let _ = ImmReleaseContext(self.hwnd, self.himc);
        }
    }
}

// ─── IME ウィンドウヘルパー ───────────────────────────────────────

/// `ImmGetDefaultIMEWnd` の null チェック付きラッパー。
///
/// IMM ブリッジが存在する場合は `Some(ime_hwnd)` を返す。
///
/// # Safety
/// Win32 API を呼び出す。
pub(crate) unsafe fn get_ime_wnd(hwnd: HWND) -> Option<HWND> {
    use crate::win32::HwndExt as _;
    // SAFETY: hwnd は呼出元でチェック済みの有効なウィンドウハンドル。
    //         ImmGetDefaultIMEWnd は hwnd に対応する IME ウィンドウを返すだけで副作用なし。
    unsafe { ImmGetDefaultIMEWnd(hwnd) }.non_null()
}

// ─── クロスプロセス IME コントロール ─────────────────────────────

/// probe系コマンド（`IMC_GET*`）。IME状態を照会するだけで、actuationを起こさない。
pub(crate) enum ProbeCmd {
    GetOpenStatus,
    GetConversionMode,
}

impl ProbeCmd {
    const fn raw(self) -> usize {
        match self {
            Self::GetOpenStatus => IMC_GETOPENSTATUS,
            Self::GetConversionMode => IMC_GETCONVERSIONMODE,
        }
    }
}

/// actuate系コマンド（`IMC_SET*`）。実際にIME状態を変更する。
pub(crate) enum ActuateCmd {
    SetOpenStatus(bool),
    SetConversionMode(u32),
}

impl ActuateCmd {
    fn raw(self) -> (usize, isize) {
        match self {
            Self::SetOpenStatus(open) => (IMC_SETOPENSTATUS, isize::from(open)),
            Self::SetConversionMode(conv) => (IMC_SETCONVERSIONMODE, conv as isize),
        }
    }
}

/// probe呼び出しが`send_health`のサーキットブレーカへ計測をフィードするか。
///
/// ADR-176 176-T9a（決定3 round6 B2対応）: 較正probeは数秒間100ms間隔で
/// 回る測定専用のクロスプロセスprobeであり、本番のグローバルサーキット
/// ブレーカを誤作動させてはならない（`runtime/executor.rs:986-995`に、
/// 診断専用probeが`send_health`を誤作動させたため削除された前例がある）。
/// `send_ime_control_raw`のdocが謳う「probe/actuate双方の計測はすべて
/// ここに集約する」という不変条件を、較正probeに限り意図的に緩める
/// ——`architecture_guard.rs`の`calibration_probe_skips_send_health_at_
/// exactly_one_call_site`がこの唯一の使用箇所を固定する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendHealthFeed {
    /// 通常どおり`send_health::record`へ計測をフィードする（probe/actuate
    /// 双方の既存呼び出し元はすべてこちら）。
    Record,
    /// `send_health::record`を呼ばない（較正probe専用）。
    Skip,
}

/// IME状態の照会（`IMC_GET*`）。actuationを起こさない。
///
/// ADR-159 round4 TJ2 MF2が「関数名だけではcmdの種類を区別できない」という
/// 既知の限界として受容していたSSOT希釈（`send_ime_control`という1関数にactuate/probe
/// 混在）を、ADR-168でこの2関数への分割によって解消した。dylintの
/// `lints/actuation_call_guard::RESTRICTED_CALLS`はこの関数名をキーに許可呼び出し元を
/// 宣言する。
///
/// # Safety
/// Win32 API を呼び出す。
pub(crate) unsafe fn probe_ime_control(
    ime_wnd: HWND,
    cmd: ProbeCmd,
    timeout_ms: u32,
    feed: SendHealthFeed,
) -> Option<usize> {
    // SAFETY: 呼び出し元の安全性要件をそのまま満たす。
    unsafe { send_ime_control_raw(ime_wnd, cmd.raw(), 0, timeout_ms, feed) }
}

/// IME状態のactuate（`IMC_SET*`）。このクレートで`WM_IME_CONTROL`経由のIME actuationを
/// 起こす唯一の入口（`lints/actuation_call_guard::RESTRICTED_CALLS`の許可リスト対象）。
///
/// # Safety
/// Win32 API を呼び出す。
pub(crate) unsafe fn actuate_ime_control(
    ime_wnd: HWND,
    cmd: ActuateCmd,
    timeout_ms: u32,
) -> Option<usize> {
    let (raw_cmd, lparam) = cmd.raw();
    // SAFETY: 呼び出し元の安全性要件をそのまま満たす。
    unsafe { send_ime_control_raw(ime_wnd, raw_cmd, lparam, timeout_ms, SendHealthFeed::Record) }
}

/// `WM_IME_CONTROL` を IME ウィンドウに送信し、結果を返す。
///
/// タイムアウトまたはエラー時は `None` を返す。
///
/// probe/actuate 双方の bump・計測・診断ログ（`conv_mutation`/`probe_actuation_fence`/
/// `shadow_send_trace`/`send_health`）はすべてここに集約する——`probe_ime_control`/
/// `actuate_ime_control` の2関数に分散させると、どちらか片方だけ計測が漏れる事故になる
/// （このクレートの全 `SendMessageTimeoutW` 呼び出しが経由する唯一のチョークポイント
/// という性質は分割後も本関数1つが維持する）。
///
/// # Safety
/// Win32 API を呼び出す。
unsafe fn send_ime_control_raw(
    ime_wnd: HWND,
    cmd: usize,
    lparam: isize,
    timeout_ms: u32,
    feed: SendHealthFeed,
) -> Option<usize> {
    let mut result = 0usize;
    // BUG-34 横展開 Step0-c: SMTO_ABORTIFHUNG は呼び出し中に相手がハングし始めた
    // 場合には効かず、宣言 timeout_ms を大幅に超えて HungAppTimeout(~5s)までブロック
    // しうる。実測msを記録して SendHealth のサーキットブレーカへフィードする
    // (このクレートの全 SendMessageTimeoutW 呼び出しは本関数を経由する唯一の
    // チョークポイントのため、ここ1箇所で全サイトを計測できる)。
    //
    // BUG-34 横展開レビュー指摘: conv_mutation_seq（win32::send_input_safe が
    // ゲート）は SendInput 経由の conv 変更しか捕捉しておらず、IMC write
    // （`set_ime_romaji_mode_for_hwnd` 等が使う IMC_SETCONVERSIONMODE）を
    // 1つも数えていなかった——旧last_send fenceが「本来検出すべき自己出力を
    // 1つも捕捉できていなかった」のと同型の欠陥が別経路で再発していた。
    // IMC_SETCONVERSIONMODE はこの関数（唯一のチョークポイント）を必ず通るため、
    // ここでも bump する（成功/失敗を問わない——メッセージが実際に処理されたかは
    // 判定できないため、安全側に倒して常に「変わった可能性がある」として扱う）。
    if cmd == IMC_SETCONVERSIONMODE {
        crate::conv_mutation::bump();
    }
    // ADR-140 Step1 決定B: probe/actuation フェンスの bump は、下の診断ログの
    // `kind=actuation` 判定と完全に同一の条件（probe 系コマンド以外）で行う。
    // `conv_mutation` の bump（IMC_SETCONVERSIONMODE のみ）と異なり、GJI の
    // open 軸（`IMC_SETOPENSTATUS`）もここで数える——`conv_mutation` が
    // 数えていなかった軸そのものが BUG-113 の actuation だったため
    // （`crate::probe_actuation_fence` doc 参照）。実際の `SendMessageTimeoutW`
    // 呼び出しより前に bump することが決定Bの必須要件。
    let is_actuation = !matches!(cmd, IMC_GETOPENSTATUS | IMC_GETCONVERSIONMODE);
    if is_actuation {
        crate::probe_actuation_fence::bump();
    }
    let start_ms = crate::hook::current_tick_ms();
    // ADR-140 Step0診断ログ: send_health用のstart_ms/end_ms（current_tick_ms()基準、
    // サーキットブレーカの既存閾値がms単位で依存しているため単位を変えない）とは
    // 別に、probe/actuation発行タイミングの相関用として高分解能タイムスタンプを
    // 追加で記録する。
    let issue_us = crate::hook::now_timestamp_us();
    // SAFETY: ime_wnd は呼出元が ImmGetDefaultIMEWnd で取得した有効な IME ウィンドウハンドル。
    //         SMTO_ABORTIFHUNG によりハングしたスレッドで無期限にブロックしない。
    //         result はスタック上の有効な usize でポインタ渡しが安全。
    let ok = unsafe {
        SendMessageTimeoutW(
            ime_wnd,
            WM_IME_CONTROL,
            WPARAM(cmd),
            LPARAM(lparam),
            SMTO_ABORTIFHUNG,
            timeout_ms,
            Some(&raw mut result),
        )
    };
    let elapsed_us = crate::hook::now_timestamp_us().saturating_sub(issue_us);
    // ADR-140 コードレビュー指摘（MAJOR）: end_ms は send_health のサーキット
    // ブレーカ計測に使われるため、下の tracing::debug! のフォーマット/I/O コストを
    // その計測窓に含めてはならない——先に end_ms を確定させてから記録する。
    // ADR-159 段階2(TF2)の`shadow_send_trace`記録も同じ理由でここに置く
    // （`is_actuation`は上のbump()と同一条件、`158-implementation-tasks.md`
    // TF2「最小限(1条件分岐)」の要件どおり新しい条件は増やしていない）。
    let end_ms = crate::hook::current_tick_ms();
    tracing::debug!(
        "[ime-io] cross_process cmd=0x{cmd:04X} kind={} ime_wnd={ime_wnd:?} \
         thread={:?} issue_us={issue_us} elapsed_us={elapsed_us}",
        if is_actuation { "actuation" } else { "probe" },
        std::thread::current().id(),
    );
    if is_actuation {
        crate::shadow_send_trace::record_ime_control(cmd, lparam, issue_us);
    }
    if matches!(feed, SendHealthFeed::Record) {
        crate::send_health::record(end_ms.saturating_sub(start_ms), end_ms);
    }
    (ok.0 != 0).then_some(result)
}

/// ADR-176 176-T9a: 較正probe専用のIME open状態照会。`get_ime_wnd`を内部で
/// 呼び、`SendHealthFeed::Skip`で`send_health`への計測フィードを回避する
/// （B2対応）。
///
/// `app_hwnd_addr`はアプリのトップレベルウィンドウハンドルを`usize`化した
/// 値で渡すこと（`HWND`は`Send`でないため、呼び出し元が
/// `win32_async::offload_timeout`のクロージャに直接キャプチャできない
/// ——`usize`のまま渡し、この関数の中で`HWND`を再構成する）。
///
/// タイムアウトは`crate::tuning::CALIBRATION_PROBE_TIMEOUT_MS`を呼び出し元
/// が渡すことを想定するが、この関数自体はタイムアウト値を固定しない
/// （呼び出し元が`tuning.rs`から渡す）。
///
/// # Safety
/// Win32 API を呼び出す。`app_hwnd_addr`は有効なウィンドウハンドルの
/// アドレスであること（無効なら`get_ime_wnd`が`None`を返すだけで
/// 安全に失敗する）。
pub(crate) unsafe fn probe_ime_open_for_calibration(
    app_hwnd_addr: usize,
    timeout_ms: u32,
) -> Option<bool> {
    let app_hwnd = HWND(app_hwnd_addr as *mut core::ffi::c_void);
    // SAFETY: app_hwnd は呼び出し元が渡した値をそのまま再構成したもの。
    //         無効であれば ImmGetDefaultIMEWnd が None を返すだけで安全。
    let ime_wnd = unsafe { get_ime_wnd(app_hwnd) }?;
    // SAFETY: ime_wnd は get_ime_wnd が返した有効なIMEウィンドウハンドル。
    let result = unsafe {
        probe_ime_control(
            ime_wnd,
            ProbeCmd::GetOpenStatus,
            timeout_ms,
            SendHealthFeed::Skip,
        )
    }?;
    Some(result != 0)
}
