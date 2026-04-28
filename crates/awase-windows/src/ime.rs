use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmGetConversionStatus, ImmGetDefaultIMEWnd, ImmReleaseContext,
    IME_CMODE_NATIVE, IME_CONVERSION_MODE, IME_SENTENCE_MODE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, SendMessageTimeoutW, SMTO_ABORTIFHUNG,
};

// ─── Cross-process IME control constants ─────────────────────

const WM_IME_CONTROL: u32 = 0x0283;
const IMC_GETOPENSTATUS: usize = 0x0005;
const IMC_SETOPENSTATUS: usize = 0x0006;
const IMC_GETCONVERSIONMODE: usize = 0x0001;

/// ローマ字入力モードフラグ（0x0010）
const IME_CMODE_ROMAN: u32 = 0x0010;

// ─── Cross-process IME 設定 ───────────────────────────────────

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
    let gui_result = crate::win32::get_gui_thread_info_with_timeout(
        std::time::Duration::from_millis(150),
    );
    let hwnd = gui_result.focused_hwnd;
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

// ─── hwnd 指定版クロスプロセス検出（detect_ime_state 専用）─────

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

// ─── 統合 IME 状態スナップショット ────────────────────────────

/// OS から取得した IME 環境の完全なスナップショット
///
/// 全フィールドが `Option<T>` で一貫した 3 値意味論を持つ:
/// - `Some(v)` = 検出成功・値は `v`
/// - `None`    = 検出失敗（タイムアウト、API エラー等）
///
/// `None` は「偽/ゼロ」ではなく「不明」であり、observer はキャッシュ値を維持する。
#[derive(Debug)]
pub struct ImeSnapshot {
    /// キーボードレイアウトが日本語か（None = 検出失敗/タイムアウト）
    pub is_japanese_ime: Option<bool>,
    /// IME が ON か（None = 検出失敗）
    pub ime_on: Option<bool>,
    /// ローマ字入力モードか（None = 検出失敗）
    pub is_romaji: Option<bool>,
    /// 生の conversion mode 値（None = 検出失敗、デバッグ用）
    pub conversion_mode: Option<u32>,
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
        // タイムアウト時はすべて None（不明）。observer 側でキャッシュ値を維持する。
        ImeSnapshot {
            is_japanese_ime: None,
            ime_on: None,
            is_romaji: None,
            conversion_mode: None,
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

    // 3. Cross-process conversion mode → is_romaji + conversion_mode (using focused hwnd)
    let conversion_mode = detect_ime_conversion_for_hwnd(focused_hwnd);

    // 4. Determine is_romaji from cross-process and direct check
    let is_romaji = if let Some(conversion) = conversion_mode {
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
        is_japanese_ime: Some(is_japanese_ime),
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

/// フォーカス切替直後の高速 IME 状態プローブ。
///
/// フックコールバック内で同期的に呼べるよう、高速 API のみ使用する:
/// - `GetKeyboardLayout` (< 1ms) → `is_japanese_ime`
/// - `GetForegroundWindow` (< 1ms) → hwnd
/// - `ImmGetDefaultIMEWnd` (< 1ms) → IMM ブリッジ有無
/// - `SendMessageTimeoutW(20ms)` → `ime_on`
///
/// 最大 ~20ms。ブラックリストアプリ（`ImmGetDefaultIMEWnd` が NULL）なら < 1ms。
///
/// # Safety
/// Win32 API を呼び出す。
pub unsafe fn fast_ime_probe() -> FastImeProbeResult {
    // 1. is_japanese_ime (always fast)
    let (is_japanese_ime, _) = keyboard_layout_info();

    if !is_japanese_ime {
        return FastImeProbeResult {
            is_japanese_ime: false,
            ime_on: Some(false),
            is_romaji: None,
        };
    }

    // 2. ime_on + is_romaji via fast cross-process check
    // GetForegroundWindow() はトップレベルウィンドウを返す。
    // detect_ime_state が使う GetGUIThreadInfo().hwndFocus（子ウィンドウ）と異なり、
    // トップレベル hwnd は TSF 互換ブリッジ経由で IMM32 API に応答できる場合が多い。
    // フォーカス切替直後に子ウィンドウで検出不能な状態でも、ここでは検出できることがある。
    let hwnd = GetForegroundWindow();
    if hwnd.0.is_null() {
        return FastImeProbeResult {
            is_japanese_ime: true,
            ime_on: None,
            is_romaji: None,
        };
    }

    let ime_wnd = ImmGetDefaultIMEWnd(hwnd);
    if ime_wnd.0.is_null() {
        // IMM ブリッジなし（Chrome/UWP 等）→ 検出不能
        return FastImeProbeResult {
            is_japanese_ime: true,
            ime_on: None,
            is_romaji: None,
        };
    }

    let mut result = 0usize;
    let ok = SendMessageTimeoutW(
        ime_wnd,
        WM_IME_CONTROL,
        WPARAM(IMC_GETOPENSTATUS),
        LPARAM(0),
        SMTO_ABORTIFHUNG,
        20, // 20ms タイムアウト（通常のポーリングは 50ms）
        Some(&raw mut result),
    );

    let ime_on = if ok.0 != 0 {
        Some(result != 0)
    } else {
        None // タイムアウトまたはエラー
    };

    // 3. conversion mode → is_romaji
    // ウィンドウ切替直後は detect_ime_state（子 hwnd 使用）が変換モードを取得できない場合があるが、
    // トップレベル hwnd 経由なら取得できることが多い。これにより focus_transition_pending 時の
    // stale な is_romaji をリセットできる。
    let mut conv_result = 0usize;
    let conv_ok = SendMessageTimeoutW(
        ime_wnd,
        WM_IME_CONTROL,
        WPARAM(IMC_GETCONVERSIONMODE),
        LPARAM(0),
        SMTO_ABORTIFHUNG,
        20,
        Some(&raw mut conv_result),
    );

    let is_romaji = if conv_ok.0 != 0 {
        let conv = conv_result as u32;
        let is_native = conv & IME_CMODE_NATIVE.0 != 0;
        let is_roman = conv & IME_CMODE_ROMAN != 0;
        log::debug!("fast_ime_probe: conv=0x{conv:08X} native={is_native} roman={is_roman}");
        if is_native { Some(is_roman) } else { None }
    } else {
        None
    };

    FastImeProbeResult {
        is_japanese_ime: true,
        ime_on,
        is_romaji,
    }
}

/// 高速プローブの結果
#[derive(Debug)]
pub struct FastImeProbeResult {
    pub is_japanese_ime: bool,
    pub ime_on: Option<bool>,
    /// ローマ字入力か（Some(true)=ローマ字, Some(false)=かな, None=検出不能）
    pub is_romaji: Option<bool>,
}
