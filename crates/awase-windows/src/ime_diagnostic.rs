//! IME 状態の診断スナップショット（Phase 1: 観測強化）。
//!
//! フォーカス変更直後・出力直前など重要タイミングで IME 周辺の各種状態を
//! 一括で取得して 1 行ログに吐き出す。これにより blind だったデバッグを
//! データ駆動に切り替える。
//!
//! 取得項目:
//! - フォーカス hwnd / pid / class
//! - 現在スレッドの HKL / lang_id
//! - ImmGetDefaultIMEWnd の有無 (IMM bridge availability)
//! - IMC_GETOPENSTATUS / IMC_GETCONVERSIONMODE のクロスプロセス値
//! - awase の shadow ime_on
//! - 解決された注入モード (Vk / Tsf / Unicode)
//! - 最後のフォーカス変更からの経過時間
//!
//! Phase 2 以降で ITfInputProcessorProfileMgr による active TIP 情報の追加を検討。

use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use crate::win32::HwndExt as _;

/// `with_app_ref` クロージャの戻り値を保持する中間構造体。
struct AppStateView {
    focus_hwnd_raw: usize,
    focus_pid: u32,
    focus_class: String,
    shadow_ime_on: bool,
    shadow_is_romaji: bool,
    shadow_is_japanese: bool,
    injection_mode: &'static str,
    ms_since_focus_change: Option<u64>,
    ms_since_last_activity: Option<u64>,
}

/// 各観測点での IME 状態のスナップショット。
///
/// すべてのフィールドが取得失敗を許容する `Option` または `bool`。
/// クロスプロセスクエリは `run_with_timeout` で保護される。
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct ImeDiagnosticSnapshot {
    /// 観測点の識別ラベル（例: "focus_change_done", "send_keys_pre"）
    pub label: &'static str,
    /// 取得時刻（ms, `current_tick_ms`）
    pub timestamp_ms: u64,
    /// フォーカス hwnd（生 ptr 値、表示用）
    pub focus_hwnd_raw: usize,
    /// フォーカスウィンドウのプロセス ID
    pub focus_pid: u32,
    /// フォーカスウィンドウのクラス名
    pub focus_class: String,
    /// フォーカスウィンドウのスレッド ID
    pub focus_thread_id: u32,
    /// HKL（KeyboardLayout）の生値
    pub hkl: u32,
    /// HKL の下位 16bit（言語 ID）
    pub hkl_lang_id: u32,
    /// `ImmGetDefaultIMEWnd` が非 NULL を返したか（IMM bridge の存在）
    pub has_imm_bridge: bool,
    /// クロスプロセス IMC_GETOPENSTATUS の結果（None=タイムアウト/失敗）
    pub imc_open_status: Option<bool>,
    /// クロスプロセス IMC_GETCONVERSIONMODE の生値（None=タイムアウト/失敗）
    pub imc_conversion_mode: Option<u32>,
    /// awase の shadow IME on/off
    pub shadow_ime_on: bool,
    /// shadow が romaji 入力モードと判定しているか
    pub shadow_is_romaji: bool,
    /// shadow が日本語キーボードと判定しているか
    pub shadow_is_japanese: bool,
    /// 解決された注入モード
    pub injection_mode: &'static str,
    /// 最後のフォアグラウンド変更からの経過時間（None=未発生）
    pub ms_since_focus_change: Option<u64>,
    /// 最後のキー活動からの経過時間（typing 判定用）
    pub ms_since_last_activity: Option<u64>,
}

impl ImeDiagnosticSnapshot {
    /// 現在の状態を捕捉する。
    ///
    /// `crate::APP` グローバル経由で焦点情報・shadow 状態を読み取り、
    /// クロスプロセスクエリは 50ms タイムアウトで保護する。
    /// メインスレッドから呼ぶこと（APP の借用要件）。
    #[must_use] 
    pub fn capture(label: &'static str) -> Self {
        let now = crate::hook::current_tick_ms();

        // ── shadow / app state を APP から取得 ──
        let view = crate::with_app_ref(|app| {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            let (pid, class) = if app.executor.platform.focus.is_focused() {
                (app.executor.platform.focus.pid, app.executor.platform.focus.class_name.clone())
            } else {
                (0u32, String::new())
            };

            let focus_change_ms = app.platform_state.focus.last_focus_change_ms;
            let activity_ms = app.platform_state.last_hook_activity_ms;

            let dt_focus = if focus_change_ms == 0 {
                None
            } else {
                Some(now.saturating_sub(focus_change_ms))
            };
            let dt_activity = if activity_ms == 0 {
                None
            } else {
                Some(now.saturating_sub(activity_ms))
            };

            // フォーカス hwnd は last_focus_info に保存されない（pid/class のみ）
            // ため、現時点のフォアグラウンド hwnd を取得する。
            // SAFETY: GetForegroundWindow はどのスレッドからも安全に呼べる非ブロッキング API。
            let hwnd = unsafe { GetForegroundWindow() };
            let hwnd_raw = hwnd.0 as usize;

            AppStateView {
                focus_hwnd_raw: hwnd_raw,
                focus_pid: pid,
                focus_class: class,
                shadow_ime_on: app.platform_state.ime_on(),
                shadow_is_romaji: app.platform_state.input_mode().is_romaji_capable(),
                shadow_is_japanese: app.platform_state.is_japanese_ime(),
                injection_mode: resolve_injection_mode_label(),
                ms_since_focus_change: dt_focus,
                ms_since_last_activity: dt_activity,
            }
        })
        .unwrap_or(AppStateView {
            focus_hwnd_raw: 0,
            focus_pid: 0,
            focus_class: String::new(),
            shadow_ime_on: false,
            shadow_is_romaji: false,
            shadow_is_japanese: false,
            injection_mode: "Unknown",
            ms_since_focus_change: None,
            ms_since_last_activity: None,
        });

        // ── HKL ──
        // SAFETY: get_gui_thread_info_with_timeout は unsafe fn で内部で Win32 API を呼ぶ。
        //         GetKeyboardLayout(tid) はフォアグラウンドスレッドの HKL を返す読み取り専用 API。
        let (hkl, hkl_lang_id, focus_thread_id) = unsafe {
            let gui = crate::win32::get_gui_thread_info_with_timeout(Duration::from_millis(100));
            let tid = gui.thread_id;
            let layout = GetKeyboardLayout(tid);
            let hkl = layout.0 as u32;
            let lang_id = crate::imm::lang_id_from_hkl(hkl);
            (hkl, lang_id, tid)
        };

        // ── ImmGetDefaultIMEWnd ──
        // SAFETY: focus_hwnd_raw は直前の GetForegroundWindow が返した有効なウィンドウハンドル値。
        //         get_ime_wnd は hwnd が null または無効でも None を返すだけで安全。
        let has_imm_bridge = unsafe {
            let hwnd = HWND(view.focus_hwnd_raw as *mut _);
            crate::imm::get_ime_wnd(hwnd).is_some()
        };

        // ── クロスプロセス IMC_GETOPENSTATUS / IMC_GETCONVERSIONMODE ──
        let (imc_open_status, imc_conversion_mode) = capture_imc(view.focus_hwnd_raw);

        Self {
            label,
            timestamp_ms: now,
            focus_hwnd_raw: view.focus_hwnd_raw,
            focus_pid: view.focus_pid,
            focus_class: view.focus_class,
            focus_thread_id,
            hkl,
            hkl_lang_id,
            has_imm_bridge,
            imc_open_status,
            imc_conversion_mode,
            shadow_ime_on: view.shadow_ime_on,
            shadow_is_romaji: view.shadow_is_romaji,
            shadow_is_japanese: view.shadow_is_japanese,
            injection_mode: view.injection_mode,
            ms_since_focus_change: view.ms_since_focus_change,
            ms_since_last_activity: view.ms_since_last_activity,
        }
    }

    /// 1 行ログ出力（`debug` レベル）。
    ///
    /// 機械的に grep / 正規表現で抽出しやすいように key=value 形式。
    pub fn log(&self) {
        let conv = self
            .imc_conversion_mode
            .map_or_else(|| "-".to_string(), |c| format!("{c:#08x}"));
        let dt_focus = self
            .ms_since_focus_change
            .map_or_else(|| "-".to_string(), |d| format!("{d}"));
        let dt_act = self
            .ms_since_last_activity
            .map_or_else(|| "-".to_string(), |d| format!("{d}"));
        let imc_open = self
            .imc_open_status
            .map_or_else(|| "-".to_string(), |o| if o { "true" } else { "false" }.to_string());

        log::debug!(
            "[ime-diag] label={label} t={t} hwnd={hwnd:#x} pid={pid} tid={tid} class=\"{class}\" \
             hkl={hkl:08x} lang={lang:04x} imm_bridge={bridge} \
             imc_open={imc_open} imc_conv={conv} \
             shadow_on={shadow_on} shadow_romaji={romaji} shadow_jp={jp} \
             inject={inject} dt_focus_ms={dt_focus} dt_activity_ms={dt_act}",
            label = self.label,
            t = self.timestamp_ms,
            hwnd = self.focus_hwnd_raw,
            pid = self.focus_pid,
            tid = self.focus_thread_id,
            class = self.focus_class,
            hkl = self.hkl,
            lang = self.hkl_lang_id,
            bridge = self.has_imm_bridge,
            shadow_on = self.shadow_ime_on,
            romaji = self.shadow_is_romaji,
            jp = self.shadow_is_japanese,
            inject = self.injection_mode,
        );
    }
}

/// クロスプロセス IMC クエリ（`IMC_GETOPENSTATUS` / `IMC_GETCONVERSIONMODE`）。
/// IMM bridge が NULL の場合は `(None, None)` を返す。
///
/// SendMessageTimeoutW を含むためメインスレッドで直接実行すると `with_app` 再入の
/// 原因になる。`run_with_timeout` でワーカースレッドへ offload し、メインスレッドの
/// メッセージポンプを止めて待つことで再入を回避する (run_with_timeout 自体は
/// thread::spawn + recv で待機し、メッセージ pump を行わない)。
fn capture_imc(focus_hwnd_raw: usize) -> (Option<bool>, Option<u32>) {
    crate::win32::run_with_timeout(Duration::from_millis(150), move || {
        // SAFETY: focus_hwnd_raw はフォアグラウンドウィンドウの有効なハンドル値。
        //         get_ime_wnd / send_ime_control は内部で SMTO_ABORTIFHUNG を使用し
        //         ワーカースレッドからも安全に呼び出せる。
        unsafe {
            let hwnd = HWND(focus_hwnd_raw as *mut _);
            let Some(_) = hwnd.non_null() else {
                return (None, None);
            };
            let Some(ime_wnd) = crate::imm::get_ime_wnd(hwnd) else {
                return (None, None);
            };
            let open = crate::imm::send_ime_control(ime_wnd, crate::imm::IMC_GETOPENSTATUS, 0, 50)
                .map(|v| v != 0);
            let conv = crate::imm::send_ime_control(ime_wnd, crate::imm::IMC_GETCONVERSIONMODE, 0, 50)
                .map(|v| v as u32);
            (open, conv)
        }
    })
    .unwrap_or((None, None))
}

/// 部分リテラル検出の実験用 1 行ログ。
///
/// `composition confirmed` 直後など、composition の現在値を観測したいタイミングで呼ぶ。
/// 取得した composition 文字列・result 文字列・cursor pos に加えて、現在のフォアグラウンド
/// プロファイル（TsfNative / Imm32Unavailable / Standard）と shadow 状態を 1 行にまとめる。
/// 各プロファイルでの IMM API 反応をログから比較するための観測点。
pub fn log_composition_probe(cold_seq: u32, label: &'static str) {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    // SAFETY: GetForegroundWindow は安全な読み取り API。capture_composition_snapshot は
    //         内部で non_null() / ImmContextGuard を使い NULL 入力にも対応する。
    let (hwnd_raw, snap) = unsafe {
        let hwnd = GetForegroundWindow();
        (hwnd.0 as usize, crate::ime::capture_composition_snapshot(hwnd))
    };

    let view = crate::with_app_ref(|app| {
        let class = app.executor.platform.focus.class_name.clone();
        let profile = app.executor.platform.current_app_profile();
        (
            class,
            format!("{profile:?}"),
            app.platform_state.ime_on(),
            app.platform_state.is_japanese_ime(),
        )
    })
    .unwrap_or((String::new(), "Unknown".to_string(), false, false));

    let comp = snap
        .comp_str
        .as_deref()
        .map_or_else(|| "-".to_string(), |s| format!("{s:?}"));
    let result = snap
        .result_str
        .as_deref()
        .map_or_else(|| "-".to_string(), |s| format!("{s:?}"));
    let comp_read = snap
        .comp_read_str
        .as_deref()
        .map_or_else(|| "-".to_string(), |s| format!("{s:?}"));
    let result_read = snap
        .result_read_str
        .as_deref()
        .map_or_else(|| "-".to_string(), |s| format!("{s:?}"));
    let cursor = snap
        .cursor_pos
        .map_or_else(|| "-".to_string(), |c| c.to_string());
    let attr = snap
        .comp_attr_bytes
        .as_deref()
        .map_or_else(|| "-".to_string(), |b| format!("{b:?}"));
    let open = snap
        .open_status
        .map_or_else(|| "-".to_string(), |b| b.to_string());
    let conv = snap
        .conversion_mode
        .map_or_else(|| "-".to_string(), |v| format!("{v:#06x}"));
    let sent = snap
        .sentence_mode
        .map_or_else(|| "-".to_string(), |v| format!("{v:#06x}"));

    log::info!(
        "[comp-probe] {label} cold={cold_seq} hwnd={hwnd_raw:#x} profile={profile} class=\"{class}\" \
         himc_null={himc_null} open={open} conv={conv} sent={sent} \
         comp={comp} comp_read={comp_read} result={result} result_read={result_read} \
         cursor={cursor} attr={attr} shadow_on={shadow_on} jp={jp}",
        profile = view.1,
        class = view.0,
        himc_null = snap.himc_null,
        shadow_on = view.2,
        jp = view.3,
    );
}

/// 解決される注入モードのラベル文字列を返す（出力経路と同じロジックを参照する）。
fn resolve_injection_mode_label() -> &'static str {
    use awase::types::AppKind;
    use crate::focus::classifier::InjectionHint;

    crate::with_app_ref(|app| {
        match app.executor.platform.injection_hint() {
            InjectionHint::ForceTsf => return "Tsf",
            InjectionHint::ForceVk => return "Vk",
            InjectionHint::Default => {}
        }
        match app.platform_state.focus.app_kind {
            AppKind::TsfNative => "Vk",
            _ => "Unicode",
        }
    })
    .unwrap_or("Unknown")
}
