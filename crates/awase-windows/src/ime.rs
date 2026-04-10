use windows::core::{implement, Interface, GUID};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetContext, ImmGetConversionStatus, ImmGetDefaultIMEWnd,
    ImmGetOpenStatus, ImmReleaseContext, GCS_COMPSTR, IME_CMODE_FULLSHAPE, IME_CMODE_KATAKANA,
    IME_CMODE_NATIVE, IME_CONVERSION_MODE, IME_SENTENCE_MODE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITfCompartment, ITfCompartmentEventSink,
    ITfCompartmentEventSink_Impl, ITfCompartmentMgr, ITfSource, ITfThreadMgr,
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, PostMessageW, SendMessageTimeoutW,
    GUITHREADINFO, SMTO_ABORTIFHUNG,
};

pub use awase::platform::ImeMode;

/// IME 状態検知のトレイト
#[allow(dead_code)] // is_composing は将来の未確定文字列検出で使用予定
pub trait ImeProvider {
    /// 現在の IME モードを取得する
    fn get_mode(&self) -> ImeMode;

    /// IME が有効（日本語入力可能な状態）かどうか
    fn is_active(&self) -> bool {
        let mode = self.get_mode();
        !matches!(mode, ImeMode::Off | ImeMode::Alphanumeric)
    }

    /// IME が未確定文字列を持っているか（変換中か）
    fn is_composing(&self) -> bool;
}

/// conversion モードビットマスクから `ImeMode` を判定する
const fn conversion_to_ime_mode(open: bool, conversion: u32) -> ImeMode {
    if !open {
        return ImeMode::Off;
    }

    if conversion & IME_CMODE_NATIVE.0 == 0 {
        return ImeMode::Alphanumeric;
    }

    if conversion & IME_CMODE_KATAKANA.0 != 0 {
        if conversion & IME_CMODE_FULLSHAPE.0 != 0 {
            ImeMode::Katakana
        } else {
            ImeMode::HalfKatakana
        }
    } else {
        ImeMode::Hiragana
    }
}

// ─── TSF (Text Services Framework) ───────────────────────────

// ── ITfCompartmentEventSink 実装 ──

/// IME 状態変更のリアルタイム通知を受け取る TSF イベントシンク。
///
/// `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`（IME ON/OFF）と
/// `GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION`（変換モード）の
/// 変更時にコールバックされ、`WM_IME_KEY_DETECTED` をメッセージループに通知する。
#[implement(ITfCompartmentEventSink)]
struct CompartmentEventSink;

impl ITfCompartmentEventSink_Impl for CompartmentEventSink_Impl {
    fn OnChange(&self, rguid: *const GUID) -> windows::core::Result<()> {
        let guid = unsafe { &*rguid };
        if *guid == GUID_COMPARTMENT_KEYBOARD_OPENCLOSE
            || *guid == GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION
        {
            log::debug!("TSF CompartmentEventSink: IME state changed");
            unsafe {
                let _ = PostMessageW(
                    HWND::default(),
                    crate::WM_IME_KEY_DETECTED,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
        Ok(())
    }
}

/// イベントシンクの登録結果（アンレジスト用の cookie を保持）
#[allow(missing_debug_implementations)]
struct SinkRegistration {
    source: ITfSource,
    cookie: u32,
}

impl Drop for SinkRegistration {
    fn drop(&mut self) {
        unsafe {
            let _ = self.source.UnadviseSink(self.cookie);
        }
    }
}

/// 指定された Compartment に対してイベントシンクを登録する。
fn advise_compartment_sink(
    thread_mgr: &ITfThreadMgr,
    guid: &GUID,
) -> Option<SinkRegistration> {
    unsafe {
        let mgr: ITfCompartmentMgr = thread_mgr.cast().ok()?;
        let compartment: ITfCompartment = mgr.GetCompartment(guid).ok()?;
        let source: ITfSource = compartment.cast().ok()?;
        let sink: ITfCompartmentEventSink = CompartmentEventSink.into();
        let cookie = source
            .AdviseSink(&ITfCompartmentEventSink::IID, &sink)
            .ok()?;
        Some(SinkRegistration { source, cookie })
    }
}

/// TSF ベースの IME 状態検知
#[allow(missing_debug_implementations)]
pub struct TsfProvider {
    thread_mgr: ITfThreadMgr,
    /// OPENCLOSE compartment のイベントシンク登録
    _open_close_sink: Option<SinkRegistration>,
    /// CONVERSION compartment のイベントシンク登録
    _conversion_sink: Option<SinkRegistration>,
}

impl TsfProvider {
    /// TSF を初期化する。失敗した場合は `None` を返す。
    pub fn try_new() -> Option<Self> {
        unsafe {
            // COM 初期化（既に初期化済みでも問題ない）
            let _ = CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);

            let thread_mgr: ITfThreadMgr =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).ok()?;

            // IME 状態変更のイベントシンクを登録
            let open_close_sink = advise_compartment_sink(
                &thread_mgr,
                &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
            );
            let conversion_sink = advise_compartment_sink(
                &thread_mgr,
                &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
            );

            if open_close_sink.is_some() {
                log::info!("TSF CompartmentEventSink registered for OPENCLOSE");
            }
            if conversion_sink.is_some() {
                log::info!("TSF CompartmentEventSink registered for CONVERSION");
            }

            log::info!("TSF provider initialized successfully");
            Some(Self {
                thread_mgr,
                _open_close_sink: open_close_sink,
                _conversion_sink: conversion_sink,
            })
        }
    }

    /// Compartment の値を読み取る
    fn get_compartment_value(&self, guid: &GUID) -> Option<u32> {
        unsafe {
            let mgr: ITfCompartmentMgr = self.thread_mgr.cast().ok()?;
            let compartment: ITfCompartment = mgr.GetCompartment(guid).ok()?;
            let variant = compartment.GetValue().ok()?;
            // VARIANT から i32 を取り出し u32 にキャスト
            let raw = variant.as_raw();
            Some(raw.Anonymous.Anonymous.Anonymous.lVal.cast_unsigned())
        }
    }
}

impl ImeProvider for TsfProvider {
    fn get_mode(&self) -> ImeMode {
        let open = self
            .get_compartment_value(&GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)
            .unwrap_or(0);
        let conversion = self
            .get_compartment_value(&GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION)
            .unwrap_or(0);

        let mode = conversion_to_ime_mode(open != 0, conversion);
        log::trace!("TSF: open={open} conversion=0x{conversion:08X} → {mode:?}");
        mode
    }

    fn is_composing(&self) -> bool {
        // TSF composition detection is complex (requires ITfContextComposition).
        // Fall back to false for now — HybridProvider will use ImmProvider as fallback.
        false
    }
}

// ─── IMM32 (Input Method Manager) ────────────────────────────

/// IMM32 ベースの IME 状態検知
#[derive(Debug)]
pub struct ImmProvider;

impl ImmProvider {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ImeProvider for ImmProvider {
    fn get_mode(&self) -> ImeMode {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd == HWND::default() {
                log::trace!("IMM: GetForegroundWindow returned NULL");
                return ImeMode::Off;
            }

            let himc = ImmGetContext(hwnd);
            if himc.is_invalid() {
                log::trace!("IMM: ImmGetContext({hwnd:?}) returned invalid");
                return ImeMode::Off;
            }

            let mut conversion = IME_CONVERSION_MODE::default();
            let mut sentence = IME_SENTENCE_MODE::default();
            let ok =
                ImmGetConversionStatus(himc, Some(&raw mut conversion), Some(&raw mut sentence));
            let _ = ImmReleaseContext(hwnd, himc);

            if !ok.as_bool() {
                log::trace!("IMM: ImmGetConversionStatus failed for hwnd={hwnd:?}");
                return ImeMode::Off;
            }

            let native = conversion.0 & IME_CMODE_NATIVE.0 != 0;
            let mode = conversion_to_ime_mode(native, conversion.0);
            log::trace!(
                "IMM: hwnd={hwnd:?} conversion=0x{:08X} native={native} → {mode:?}",
                conversion.0,
            );
            mode
        }
    }

    fn is_composing(&self) -> bool {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd == HWND::default() {
                return false;
            }
            let himc = ImmGetContext(hwnd);
            if himc.is_invalid() {
                return false;
            }
            let len = ImmGetCompositionStringW(himc, GCS_COMPSTR, None, 0);
            let _ = ImmReleaseContext(hwnd, himc);
            len > 0
        }
    }
}

// ─── Cross-process IME detection via ImmGetDefaultIMEWnd ─────

/// WM_IME_CONTROL message constant
const WM_IME_CONTROL: u32 = 0x0283;
/// IMC_GETOPENSTATUS wParam for WM_IME_CONTROL
const IMC_GETOPENSTATUS: usize = 0x0005;
/// IMC_GETCONVERSIONMODE wParam for WM_IME_CONTROL
const IMC_SETOPENSTATUS: usize = 0x0006;
const IMC_GETCONVERSIONMODE: usize = 0x0001;

/// Cross-process IME ON/OFF detection via `ImmGetDefaultIMEWnd`.
///
/// This works across process boundaries (unlike `ImmGetContext` which
/// returns NULL for other processes' windows).
///
/// Returns `Some(true)` = IME ON, `Some(false)` = IME OFF, `None` = detection failed.
///
/// # Safety
/// Calls Win32 APIs. Must be called from the main thread.
pub unsafe fn detect_ime_open_cross_process() -> Option<bool> {
    let hwnd = GetForegroundWindow();
    if hwnd.0.is_null() {
        return None;
    }

    let ime_wnd = ImmGetDefaultIMEWnd(hwnd);
    if ime_wnd.0.is_null() {
        return None; // UWP — no IME window
    }

    let mut result = 0usize;
    let ok = SendMessageTimeoutW(
        ime_wnd,
        WM_IME_CONTROL,
        WPARAM(IMC_GETOPENSTATUS),
        LPARAM(0),
        SMTO_ABORTIFHUNG,
        50, // timeout ms — メッセージループから呼ばれるためブロッキング OK。
        // タイムアウト時は None を返し shadow state にフォールバックする。
        Some(&raw mut result),
    );

    log::trace!("CrossProcess: ime_wnd={ime_wnd:?} open={result:?}");

    if ok.0 == 0 {
        return None; // timeout or error
    }
    Some(result != 0)
}

/// クロスプロセスで IME の ON/OFF を設定する。
///
/// `GetGUIThreadInfo().hwndFocus` で実際のキーボードフォーカスウィンドウを特定し、
/// `ImmGetDefaultIMEWnd` + `WM_IME_CONTROL / IMC_SETOPENSTATUS` で IME 状態を設定する。
/// detect 側と同じ hwndFocus を使うことで、Zoom 等のマルチウィンドウアプリで
/// トップレベルウィンドウと入力ウィンドウの IME context が異なる場合も正しく動作する。
///
/// Returns `true` if the operation succeeded.
///
/// # Safety
/// Calls Win32 APIs. Must be called from the main thread.
pub unsafe fn set_ime_open_cross_process(open: bool) -> bool {
    // detect_ime_state() と同じく hwndFocus を使う
    let hwnd = {
        let mut gui_info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(0, &raw mut gui_info).is_ok() {
            if gui_info.hwndFocus != HWND::default() {
                gui_info.hwndFocus
            } else {
                gui_info.hwndActive
            }
        } else {
            GetForegroundWindow()
        }
    };
    if hwnd.0.is_null() {
        return false;
    }

    let ime_wnd = ImmGetDefaultIMEWnd(hwnd);
    if ime_wnd.0.is_null() {
        return false;
    }

    let mut result = 0usize;
    let ok = SendMessageTimeoutW(
        ime_wnd,
        WM_IME_CONTROL,
        WPARAM(IMC_SETOPENSTATUS),
        LPARAM(isize::from(open)),
        SMTO_ABORTIFHUNG,
        50,
        Some(&raw mut result),
    );

    let success = ok.0 != 0;
    log::debug!("set_ime_open_cross_process: hwnd={hwnd:?} ime_wnd={ime_wnd:?} open={open} success={success}");
    success
}

/// Cross-process IME conversion mode detection via `ImmGetDefaultIMEWnd`.
///
/// Returns the raw conversion mode bits, or `None` if detection failed.
///
/// # Safety
/// Calls Win32 APIs. Must be called from the main thread.
unsafe fn detect_ime_conversion_cross_process() -> Option<u32> {
    let hwnd = GetForegroundWindow();
    if hwnd.0.is_null() {
        return None;
    }

    let ime_wnd = ImmGetDefaultIMEWnd(hwnd);
    if ime_wnd.0.is_null() {
        return None;
    }

    let mut result = 0usize;
    let ok = SendMessageTimeoutW(
        ime_wnd,
        WM_IME_CONTROL,
        WPARAM(IMC_GETCONVERSIONMODE),
        LPARAM(0),
        SMTO_ABORTIFHUNG,
        50,
        Some(&raw mut result),
    );

    if ok.0 == 0 {
        return None;
    }
    Some(result as u32)
}

// ─── hwnd 指定版のクロスプロセス検出 ──────────────────────

/// 指定した hwnd の IME ON/OFF をクロスプロセスで検出する。
///
/// `detect_ime_state()` から呼ばれ、`GetGUIThreadInfo().hwndFocus` を使うことで
/// 実際のキーボードフォーカスウィンドウの IME 状態を取得する。
unsafe fn detect_ime_open_for_hwnd(hwnd: HWND) -> Option<bool> {
    if hwnd.0.is_null() {
        return None;
    }
    let ime_wnd = ImmGetDefaultIMEWnd(hwnd);
    if ime_wnd.0.is_null() {
        return None;
    }
    let mut result = 0usize;
    let ok = SendMessageTimeoutW(
        ime_wnd,
        WM_IME_CONTROL,
        WPARAM(IMC_GETOPENSTATUS),
        LPARAM(0),
        SMTO_ABORTIFHUNG,
        50,
        Some(&raw mut result),
    );
    log::trace!("CrossProcess(hwndFocus): ime_wnd={ime_wnd:?} open={result:?}");
    if ok.0 == 0 {
        return None;
    }
    Some(result != 0)
}

/// 指定した hwnd の IME conversion mode をクロスプロセスで検出する。
unsafe fn detect_ime_conversion_for_hwnd(hwnd: HWND) -> Option<u32> {
    if hwnd.0.is_null() {
        return None;
    }
    let ime_wnd = ImmGetDefaultIMEWnd(hwnd);
    if ime_wnd.0.is_null() {
        return None;
    }
    let mut result = 0usize;
    let ok = SendMessageTimeoutW(
        ime_wnd,
        WM_IME_CONTROL,
        WPARAM(IMC_GETCONVERSIONMODE),
        LPARAM(0),
        SMTO_ABORTIFHUNG,
        50,
        Some(&raw mut result),
    );
    if ok.0 == 0 {
        return None;
    }
    Some(result as u32)
}

/// 指定した hwnd で直接かな入力方式を確認する。
unsafe fn detect_kana_for_hwnd(hwnd: HWND) -> Option<bool> {
    if hwnd == HWND::default() {
        return None;
    }
    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() {
        return None;
    }
    let mut conversion = IME_CONVERSION_MODE::default();
    let mut sentence = IME_SENTENCE_MODE::default();
    let ok = ImmGetConversionStatus(himc, Some(&raw mut conversion), Some(&raw mut sentence));
    let _ = ImmReleaseContext(hwnd, himc);
    if !ok.as_bool() {
        return None;
    }
    let is_native = conversion.0 & IME_CMODE_NATIVE.0 != 0;
    let is_roman = conversion.0 & IME_CMODE_ROMAN != 0;
    log::debug!(
        "detect_kana_for_hwnd: conversion=0x{:08X} native={is_native} roman={is_roman}",
        conversion.0
    );
    if !is_native {
        return Some(false);
    }
    Some(!is_roman)
}

// ─── 複合プロバイダ（TSF 優先、IMM32 フォールバック）────────

/// TSF を優先し、失敗時に IMM32 にフォールバックするプロバイダ
#[allow(missing_debug_implementations)]
pub struct HybridProvider {
    tsf: Option<TsfProvider>,
    imm: ImmProvider,
}

impl HybridProvider {
    /// TSF の初期化を試み、成否に関わらず IMM32 もフォールバックとして保持する
    pub fn new() -> Self {
        let tsf = TsfProvider::try_new();
        if tsf.is_none() {
            log::info!("TSF initialization failed, using IMM32 only");
        }
        Self {
            tsf,
            imm: ImmProvider::new(),
        }
    }
}

impl ImeProvider for HybridProvider {
    fn get_mode(&self) -> ImeMode {
        // Layer 1: Cross-process detection via ImmGetDefaultIMEWnd (works for Win32 apps)
        let cross_process_result = unsafe { detect_ime_open_cross_process() };

        if let Some(open) = cross_process_result {
            if !open {
                // IME is definitively OFF
                log::trace!("HybridIME: CrossProcess=OFF → Off");
                return ImeMode::Off;
            }
            // IME is ON — try to get conversion mode for detailed state
            if let Some(conversion) = unsafe { detect_ime_conversion_cross_process() } {
                let mode = conversion_to_ime_mode(true, conversion);
                log::trace!("HybridIME: CrossProcess=ON conversion=0x{conversion:08X} → {mode:?}");
                return mode;
            }
            // Could not get conversion mode — IME is ON but mode unknown, assume Hiragana
            log::trace!("HybridIME: CrossProcess=ON, conversion unavailable → Hiragana");
            return ImeMode::Hiragana;
        }

        // Layer 2: Fall back to existing TSF/IMM (only works for own thread)
        log::trace!("HybridIME: CrossProcess=None, falling back to TSF/IMM");

        let tsf_mode = self.tsf.as_ref().map(ImeProvider::get_mode);
        let imm_mode = self.imm.get_mode();

        // Keyboard layout (HKL) as additional signal
        let hkl = unsafe { GetKeyboardLayout(0) };
        let lang_id = hkl.0 as u32 & 0xFFFF;
        let is_japanese_hkl = lang_id == crate::vk::LANGID_JAPANESE;

        // ImmGetOpenStatus as yet another signal
        let imm_open = unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd == HWND::default() {
                None
            } else {
                let himc = ImmGetContext(hwnd);
                if himc.is_invalid() {
                    None
                } else {
                    let open = ImmGetOpenStatus(himc);
                    let _ = ImmReleaseContext(hwnd, himc);
                    Some(open.as_bool())
                }
            }
        };

        log::trace!(
            "HybridIME: TSF={tsf_mode:?} IMM={imm_mode:?} ImmOpenStatus={imm_open:?} HKL=0x{lang_id:04X} japanese={is_japanese_hkl}",
        );

        // Decision: TSF first, then IMM fallback
        let result = tsf_mode.map_or(imm_mode, |tsf| {
            if tsf == ImeMode::Off {
                // TSF says Off — check IMM as fallback
                imm_mode
            } else {
                tsf
            }
        });

        // Additional fallback: if both say Off but ImmOpenStatus is true,
        // the IME is likely active but in a state we can't detect well.
        // Log this discrepancy for debugging.
        if result == ImeMode::Off && imm_open == Some(true) {
            log::debug!(
                "HybridIME: TSF/IMM say Off but ImmOpenStatus=true — possible detection gap"
            );
        }

        log::trace!("HybridIME: final result={result:?}");
        result
    }

    fn is_composing(&self) -> bool {
        let result = self.imm.is_composing();
        log::trace!("HybridIME: is_composing={result}");
        result
    }
}

/// IME_CMODE_ROMAN: ローマ字入力モードフラグ（0x0010）
///
/// このビットが立っていればローマ字入力方式、
/// 立っていなければ（かつ NATIVE が立っていれば）JIS かな入力方式。
const IME_CMODE_ROMAN: u32 = 0x0010;

/// IME がかな入力方式（JIS かな）かどうかをクロスプロセスで検出する。
///
/// Returns `Some(true)` = かな入力方式, `Some(false)` = ローマ字入力方式,
/// `None` = 検出失敗（IME OFF など）。
///
/// 注意: 一部の IME（Google 日本語入力等）はクロスプロセス検出で `IME_CMODE_ROMAN`
/// フラグを返さない場合がある。その場合はローマ字入力（デフォルト）として扱う。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn detect_kana_input_method() -> Option<bool> {
    let conversion = detect_ime_conversion_cross_process()?;
    let is_native = conversion & IME_CMODE_NATIVE.0 != 0;
    let is_roman = conversion & IME_CMODE_ROMAN != 0;

    log::debug!(
        "detect_kana_input_method: conversion=0x{conversion:08X} native={is_native} roman={is_roman}"
    );

    if !is_native {
        return Some(false); // IME が日本語モードでなければ、かな入力ではない
    }

    // ROMAN フラグが明示的にセットされている → ローマ字入力
    if is_roman {
        return Some(false);
    }

    // ROMAN フラグなし: ImmGetConversionStatus で再確認する。
    // クロスプロセス検出 (WM_IME_CONTROL) では ROMAN フラグが
    // 返されない IME があるため、直接 API で二重チェック。
    let direct_result = detect_kana_direct();
    log::debug!("detect_kana_input_method: direct_check={direct_result:?}");
    Some(direct_result.unwrap_or(false)) // 不明なら安全側（ローマ字）
}

/// ImmGetConversionStatus で直接かな入力方式を確認する。
///
/// # Safety
/// Win32 API を呼び出す。
unsafe fn detect_kana_direct() -> Option<bool> {
    let hwnd = GetForegroundWindow();
    if hwnd == HWND::default() {
        return None;
    }
    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() {
        return None;
    }
    let mut conversion = IME_CONVERSION_MODE::default();
    let mut sentence = IME_SENTENCE_MODE::default();
    let ok = ImmGetConversionStatus(himc, Some(&raw mut conversion), Some(&raw mut sentence));
    let _ = ImmReleaseContext(hwnd, himc);
    if !ok.as_bool() {
        return None;
    }
    let is_native = conversion.0 & IME_CMODE_NATIVE.0 != 0;
    let is_roman = conversion.0 & IME_CMODE_ROMAN != 0;
    log::debug!(
        "detect_kana_direct: conversion=0x{:08X} native={is_native} roman={is_roman}",
        conversion.0
    );
    if !is_native {
        return Some(false);
    }
    Some(!is_roman)
}

// ─── 統合 IME 状態スナップショット ────────────────────────────

/// OS から取得した IME 環境の完全なスナップショット
#[derive(Debug)]
pub struct ImeSnapshot {
    /// キーボードレイアウトが日本語か
    pub is_japanese_ime: bool,
    /// IME が ON か（None = 検出失敗）
    pub ime_on: Option<bool>,
    /// ローマ字入力モードか（None = 検出失敗）
    pub is_romaji: Option<bool>,
    /// 生の conversion mode 値（デバッグ用）
    pub conversion_mode: u32,
}

/// `detect_ime_state` をワーカースレッドでタイムアウト付きで実行する。
///
/// 複数のブロッキング IMM32 API（`ImmGetContext`, `ImmGetConversionStatus` 等）を
/// 連鎖的に呼ぶため、メッセージループスレッドから直接呼ぶとハングする恐れがある。
/// ワーカースレッドで実行し、タイムアウトした場合は検出失敗扱いにする。
///
/// # Safety
/// Win32 API を呼び出す。
pub unsafe fn detect_ime_state_with_timeout(timeout: std::time::Duration) -> ImeSnapshot {
    crate::win32::run_with_timeout(timeout, || unsafe { detect_ime_state() }).unwrap_or_else(|| {
        log::warn!("detect_ime_state timed out, returning empty snapshot");
        ImeSnapshot {
            is_japanese_ime: false,
            ime_on: None,
            is_romaji: None,
            conversion_mode: 0,
        }
    })
}

/// OS API を呼び出して IME 状態を一括取得する。
///
/// `GetGUIThreadInfo().hwndFocus` を使って実際のキーボードフォーカスウィンドウの
/// IME 状態を取得する。`GetForegroundWindow()` はトップレベルウィンドウを返すため、
/// 子ウィンドウと異なる IME context を持つ場合（wezterm 等）に不正確になる。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn detect_ime_state() -> ImeSnapshot {
    // 0. Resolve the focused window once and use it for all queries.
    // GetGUIThreadInfo はフォアグラウンドスレッドがハングすると無期限ブロックするため、
    // タイムアウト付きヘルパーを使用する。
    let result = crate::win32::get_gui_thread_info_with_timeout(
        std::time::Duration::from_millis(200),
    );
    let focused_hwnd = result.focused_hwnd;
    let thread_id = result.thread_id;

    // 1. Keyboard layout → is_japanese_ime
    let is_japanese_ime = {
        let hkl = GetKeyboardLayout(thread_id);
        let lang_id = (hkl.0 as u32) & 0xFFFF;
        lang_id == crate::vk::LANGID_JAPANESE
    };

    // 2. Cross-process IME ON/OFF → ime_on (using focused hwnd)
    let ime_on = detect_ime_open_for_hwnd(focused_hwnd);

    // 3. Cross-process conversion mode → is_romaji + raw conversion_mode (using focused hwnd)
    let cross_conversion = detect_ime_conversion_for_hwnd(focused_hwnd);
    let conversion_mode = cross_conversion.unwrap_or(0);

    // 4. Determine is_romaji from cross-process and direct check
    let is_romaji = if let Some(conversion) = cross_conversion {
        let is_native = conversion & IME_CMODE_NATIVE.0 != 0;
        let is_roman = conversion & IME_CMODE_ROMAN != 0;

        if !is_native {
            // IME が日本語モードでなければ、かな/ローマ字の区別は不要
            None
        } else if is_roman {
            // ROMAN フラグが明示的にセット → ローマ字入力
            Some(true)
        } else {
            // ROMAN フラグなし + NATIVE あり: クロスプロセスではかな入力に見える。
            // 直接 API で二重チェック（一部 IME は ROMAN を返さないため）。
            let direct = detect_kana_for_hwnd(focused_hwnd);
            log::debug!(
                "detect_ime_state: cross native={is_native} roman={is_roman}, direct_kana={direct:?}"
            );
            match direct {
                Some(is_kana) => Some(!is_kana),
                // direct が失敗した場合: 判定不能（None を返し、前回値を維持する）。
                // Zoom 等は romaji モードでも ROMAN ビットを報告しないため、
                // ここで Some(false) を返すと Engine が起動しなくなる。
                // 実際のかな切替は observer 側で conversion_mode の ROMAN→非ROMAN 遷移を検出する。
                None => None,
            }
        }
    } else {
        // cross-process 失敗: direct のみで試行
        detect_kana_for_hwnd(focused_hwnd).map(|is_kana| !is_kana)
    };

    ImeSnapshot {
        is_japanese_ime,
        ime_on,
        is_romaji,
        conversion_mode,
    }
}

/// 現在のキーボードレイアウトの言語情報を返す。
///
/// Returns `(is_japanese, lang_id)` — 日本語レイアウトかどうかと言語 ID (下位16ビット)。
#[must_use]
pub fn keyboard_layout_info() -> (bool, u32) {
    unsafe {
        let hkl = GetKeyboardLayout(0);
        // 下位 16 bit が言語 ID。日本語は 0x0411
        let lang_id = hkl.0 as u32 & 0xFFFF;
        (lang_id == crate::vk::LANGID_JAPANESE, lang_id)
    }
}

/// 現在のキーボードレイアウトが日本語かどうかを判定する
#[must_use]
#[allow(dead_code)]
pub fn is_japanese_input_language() -> bool {
    keyboard_layout_info().0
}
