use std::mem::size_of;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetConversionStatus, IME_COMPOSITION_STRING, IME_CONVERSION_MODE,
    IME_SENTENCE_MODE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, MapVirtualKeyW, MAPVK_VK_TO_VSC, SendInput, INPUT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_KEYDOWN, WM_KEYUP,
};

use crate::focus::class_names::is_tsf_native_window;
use crate::imm::{
    IMC_GETCONVERSIONMODE, IMC_GETOPENSTATUS, IMC_SETCONVERSIONMODE, IMC_SETOPENSTATUS,
    IME_CMODE_NATIVE, IME_CMODE_ROMAN,
};

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
#[must_use] 
pub unsafe fn set_ime_open_cross_process(open: bool) -> bool {
    let gui_result = crate::win32::get_gui_thread_info_with_timeout(
        std::time::Duration::from_millis(150),
    );
    let Some(hwnd) = gui_result.focused_hwnd else { return false; };
    // SAFETY: hwnd は get_gui_thread_info_with_timeout が返した有効なフォーカスウィンドウハンドル。
    //         get_ime_wnd は内部で ImmGetDefaultIMEWnd を呼ぶ安全なラッパーであり、NULL を返す場合は
    //         直後の `?` でショートサーキットするため問題ない。
    let Some(ime_wnd) = (unsafe { crate::imm::get_ime_wnd(hwnd) }) else { return false; };
    // SAFETY: ime_wnd は get_ime_wnd が返した有効な IME ウィンドウハンドル。
    //         send_ime_control は SendMessageTimeoutW のラッパーであり、タイムアウト付きのため
    //         相手プロセスがハングしても指定時間後に制御が戻る。
    let success = unsafe {
        crate::imm::send_ime_control(ime_wnd, IMC_SETOPENSTATUS, isize::from(open), 50)
    }
    .is_some();
    log::debug!("set_ime_open_cross_process: hwnd={hwnd:?} ime_wnd={ime_wnd:?} open={open} success={success}");
    success
}

/// TSF ネイティブアプリ（Chrome 等）向け IME トグルフォールバック。
///
/// `WM_IME_CONTROL` が効かない TSF アプリに対して `SendInput(VK_KANJI)` で IME をトグルする。
///
/// VK_KANJI はトグルキーのため **呼び出し元は shadow_ime_on != desired を事前確認すること**。
/// `dwExtraInfo` に `IME_KANJI_MARKER` を付けるため awase 自身のフックが再インターセプトしない
/// （フック先頭の自己注入チェックで即パススルー、shadow toggle もスキップ）。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn post_kanji_toggle_to_focused() {
    use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};
    const VK_KANJI: u16 = 0x19;
    let inputs = [
        make_key_input_ex(VK_KANJI, false, IME_KANJI_MARKER),
        make_key_input_ex(VK_KANJI, true,  IME_KANJI_MARKER),
    ];
    log::debug!("[ime-fallback] SendInput VK_KANJI (0x19) IME toggle");
    // SAFETY: inputs は正しく初期化された INPUT 配列であり、size_of::<INPUT>() は正確な構造体サイズを返す。
    //         SendInput はスレッドセーフで任意のスレッドから呼び出せる。
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent == 0 {
        log::warn!("[ime-fallback] SendInput(VK_KANJI) failed");
    }
}

/// IME モード切り替えキーを `SendInput` で送信する。
///
/// Engine ON/OFF 時に IME の入力モードを強制切り替えするために使う。
/// 代表的な VK コード:
/// - `0xF3` (VK_DBE_SBCSCHAR): 半角モード → Engine OFF 時
/// - `0xF4` (VK_DBE_DBCSCHAR): 全角モード → Engine ON 時
///
/// `dwExtraInfo` に `IME_KANJI_MARKER` を付けるため awase 自身のフックが
/// 再インターセプトしない。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn send_ime_mode_key(vk: u16) {
    use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};
    let inputs = [
        make_key_input_ex(vk, false, IME_KANJI_MARKER),
        make_key_input_ex(vk, true,  IME_KANJI_MARKER),
    ];
    log::debug!("[ime-mode] SendInput vk=0x{vk:02X}");
    // SAFETY: inputs は make_key_input_ex で正しく初期化された INPUT 配列であり、
    //         size_of::<INPUT>() は正確な構造体サイズを返す。
    //         SendInput はスレッドセーフで任意のスレッドから呼び出せる。
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent == 0 {
        log::warn!("[ime-mode] SendInput(vk=0x{vk:02X}) failed");
    }
}

/// 現在のフォアグラウンドウィンドウの IME 変換モード生値を返す（診断ログ専用）。
///
/// ビット定義: NATIVE=0x0001 KATAKANA=0x0002 FULLSHAPE=0x0008 ROMAN=0x0010
///
/// # Safety
/// Calls Win32 APIs.
#[must_use] 
pub unsafe fn get_ime_conversion_mode_raw() -> Option<u32> {
    // SAFETY: GetForegroundWindow はスレッドセーフで、NULL を返す可能性があるが
    //         detect_ime_conversion_for_hwnd 内の non_null_hwnd チェックで処理される。
    detect_ime_conversion_for_hwnd(unsafe { GetForegroundWindow() })
}

/// タイムアウト指定版 IME 変換モード取得（H1 タイミング計測専用）。
///
/// `get_ime_conversion_mode_raw` の 50ms 固定タイムアウトを変更できるバージョン。
/// 短い timeout_ms（例: 10ms）を指定することで、warmup 直後の応答時間を細かく計測できる。
///
/// # Safety
/// Calls Win32 APIs.
#[must_use] 
pub unsafe fn get_ime_conversion_mode_raw_timeout(timeout_ms: u32) -> Option<u32> {
    // SAFETY: GetForegroundWindow はスレッドセーフで、NULL を返す場合は non_null_hwnd が `?` で None を返す。
    let hwnd = crate::win32::non_null_hwnd(unsafe { GetForegroundWindow() })?;
    // SAFETY: hwnd は non_null_hwnd で NULL チェック済みの有効なウィンドウハンドル。
    let ime_wnd = unsafe { crate::imm::get_ime_wnd(hwnd) }?;
    // SAFETY: ime_wnd は get_ime_wnd が返した有効な IME ウィンドウハンドル。
    //         send_ime_control は SendMessageTimeoutW のラッパーで、timeout_ms 内に制御が戻ることが保証される。
    unsafe { crate::imm::send_ime_control(ime_wnd, IMC_GETCONVERSIONMODE, 0, timeout_ms) }
        .map(|v| v as u32)
}

/// フォアグラウンドウィンドウのクラス名を返す（H1 診断ログ専用）。
///
/// # Safety
/// Calls Win32 APIs.
#[must_use] 
pub unsafe fn get_foreground_window_class() -> String {
    // SAFETY: GetForegroundWindow はスレッドセーフで、NULL を返す場合は non_null_hwnd が None を返し
    //         早期リターンする。
    let Some(hwnd) = crate::win32::non_null_hwnd(unsafe { GetForegroundWindow() }) else {
        return "null".to_string();
    };
    let class = crate::focus::classify::get_class_name_string(hwnd);
    if class.is_empty() { "unknown".to_string() } else { class }
}

/// クロスプロセスで IME をローマ字モードに設定する。
///
/// VK_DBE_HIRAGANA (0xF2) による warmup は非同期のため、同一 SendInput バッチ内の
/// 最初の文字が mode switch 完了前に到達し "koの"/"ho助金" 等の cold-start 文字化けが発生する。
/// 本関数は IMM32 の IMC_SETCONVERSIONMODE を使って SendInput 前に同期的にローマ字モードへ切り替える。
///
/// Returns `true` if the operation succeeded or the mode was already correct.
///
/// # Safety
/// Calls Win32 APIs. Must be called from the main thread.
#[must_use] 
pub unsafe fn set_ime_romaji_mode() -> bool {
    // SAFETY: GetForegroundWindow はスレッドセーフで、NULL を返す場合は non_null_hwnd が None を返し
    //         早期リターンする。
    let Some(hwnd) = crate::win32::non_null_hwnd(unsafe { GetForegroundWindow() }) else {
        return false;
    };
    // SAFETY: hwnd は non_null_hwnd で NULL チェック済みの有効なウィンドウハンドル。
    let Some(ime_wnd) = (unsafe { crate::imm::get_ime_wnd(hwnd) }) else { return false; };

    // SAFETY: ime_wnd は get_ime_wnd が返した有効な IME ウィンドウハンドル。
    //         タイムアウト 50ms 内に制御が戻ることが保証される。
    let Some(current) =
        (unsafe { crate::imm::send_ime_control(ime_wnd, IMC_GETCONVERSIONMODE, 0, 50) })
    else {
        return false;
    };
    let conv = current as u32;
    let new_conv = conv | IME_CMODE_ROMAN;
    if new_conv == conv {
        return true; // already romaji
    }

    // SAFETY: ime_wnd は get_ime_wnd が返した有効な IME ウィンドウハンドル。
    //         new_conv は取得した conv に IME_CMODE_ROMAN を OR したものであり有効な変換モード値。
    let success =
        unsafe { crate::imm::send_ime_control(ime_wnd, IMC_SETCONVERSIONMODE, new_conv as isize, 50) }
            .is_some();
    log::debug!("[imm-romaji] conv 0x{conv:08X} → 0x{new_conv:08X} success={success}");
    success
}

// ─── hwnd 指定版クロスプロセス検出（read_ime_state_full 専用）─────

unsafe fn detect_ime_open_for_hwnd(hwnd: HWND) -> Option<bool> {
    crate::win32::non_null_hwnd(hwnd)?;
    // SAFETY: hwnd は non_null_hwnd で NULL チェック済みの有効なウィンドウハンドル。
    let ime_wnd = unsafe { crate::imm::get_ime_wnd(hwnd) }?;
    // SAFETY: ime_wnd は get_ime_wnd が返した有効な IME ウィンドウハンドル。
    //         タイムアウト 50ms 付きで呼び出しているため応答なしプロセスでもブロックしない。
    let result = unsafe { crate::imm::send_ime_control(ime_wnd, IMC_GETOPENSTATUS, 0, 50) }?;
    log::trace!("CrossProcess(hwndFocus): ime_wnd={ime_wnd:?} open={result}");
    Some(result != 0)
}

unsafe fn detect_ime_conversion_for_hwnd(hwnd: HWND) -> Option<u32> {
    crate::win32::non_null_hwnd(hwnd)?;
    // SAFETY: hwnd は non_null_hwnd で NULL チェック済みの有効なウィンドウハンドル。
    let ime_wnd = unsafe { crate::imm::get_ime_wnd(hwnd) }?;
    // SAFETY: ime_wnd は get_ime_wnd が返した有効な IME ウィンドウハンドル。
    //         タイムアウト 50ms 付きで呼び出しているため応答なしプロセスでもブロックしない。
    unsafe { crate::imm::send_ime_control(ime_wnd, IMC_GETCONVERSIONMODE, 0, 50) }
        .map(|v| v as u32)
}

unsafe fn detect_kana_for_hwnd(hwnd: HWND) -> Option<bool> {
    crate::win32::non_null_hwnd(hwnd)?;
    // SAFETY: hwnd は non_null_hwnd で NULL チェック済みの有効なウィンドウハンドル。
    //         ImmContextGuard は ImmGetContext/ImmReleaseContext を RAII で管理し、
    //         NULL HIMC を取得した場合は None を返す。
    let ctx = unsafe { crate::imm::ImmContextGuard::new(hwnd) }?;
    let mut conversion = IME_CONVERSION_MODE::default();
    let mut sentence = IME_SENTENCE_MODE::default();
    // SAFETY: ctx.himc() は ImmContextGuard が保持する有効な HIMC。
    //         conversion と sentence はスタック上の初期化済み変数へのポインタであり呼び出し中は有効。
    let ok = unsafe {
        ImmGetConversionStatus(ctx.himc(), Some(&raw mut conversion), Some(&raw mut sentence))
    };
    if !ok.as_bool() {
        return None;
    }
    let is_native = conversion.0 & IME_CMODE_NATIVE != 0;
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
    /// TSF ネイティブウィンドウのため検出をスキップした（true = IMM32 未使用）。
    /// タイムアウト等の一時的失敗と区別し、miss_count を増やさないために使う。
    pub is_tsf_native: bool,
}

/// `read_ime_state_full` をワーカースレッドでタイムアウト付きで実行する。
///
/// 複数のブロッキング IMM32 API（`ImmGetContext`, `ImmGetConversionStatus` 等）を
/// 連鎖的に呼ぶため、メッセージループスレッドから直接呼ぶとハングする恐れがある。
/// ワーカースレッドで実行し、タイムアウトした場合は検出失敗扱いにする。
///
/// # Safety
/// Win32 API を呼び出す。
#[must_use] 
pub unsafe fn read_ime_state_full_with_timeout(timeout: std::time::Duration) -> ImeSnapshot {
    // SAFETY: read_ime_state_full は unsafe fn であり、呼び出し元（本関数）が unsafe コンテキストを
    //         保証する。run_with_timeout はワーカースレッドで実行するが、Win32 IMM32 API は
    //         ワーカースレッドからも呼び出し可能。
    crate::win32::run_with_timeout(timeout, || unsafe { read_ime_state_full() }).unwrap_or_else(|| {
        log::warn!("read_ime_state_full timed out, returning empty snapshot");
        ImeSnapshot {
            is_japanese_ime: None,
            ime_on: None,
            is_romaji: None,
            conversion_mode: None,
            is_tsf_native: false,
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
#[must_use] 
pub unsafe fn read_ime_state_full() -> ImeSnapshot {
    // 0. フォーカスウィンドウを一度解決して全クエリに使う。
    // GetGUIThreadInfo はフォアグラウンドスレッドがハングすると無期限ブロックするため
    // タイムアウト付きヘルパーを使用する。
    let result = crate::win32::get_gui_thread_info_with_timeout(
        std::time::Duration::from_millis(200),
    );
    // None（フォーカスウィンドウ不明）の場合は HWND::default() にフォールバックする。
    // detect_ime_open_for_hwnd 等は null HWND を適切に処理して None を返す。
    let focused_hwnd = result.focused_hwnd.unwrap_or_default();
    let thread_id = result.thread_id;

    // 1. Keyboard layout → is_japanese_ime
    let is_japanese_ime = {
        // SAFETY: GetKeyboardLayout はスレッドセーフで任意のスレッドから呼び出せる。
        //         thread_id は get_gui_thread_info_with_timeout が返した値で 0（現在スレッド）も許容される。
        let hkl = unsafe { GetKeyboardLayout(thread_id) };
        let lang_id = (hkl.0 as u32) & 0xFFFF;
        lang_id == crate::vk::LANGID_JAPANESE
    };

    // 1b. TSF-native ウィンドウ（Windows Terminal の InputSite 等）は IMM32 を使わないため
    // imc_open=false を返すが、これは IME が OFF であることを意味しない。
    {
        let class = crate::focus::classify::get_class_name_string(focused_hwnd);
        log::debug!("read_ime_state_full: focused_hwnd={focused_hwnd:?} class={class:?}");
        if is_tsf_native_window(&class) {
            log::debug!(
                "read_ime_state_full: TSF-native window ({class}) → ime_on=None (preserving state)"
            );
            return ImeSnapshot {
                is_japanese_ime: Some(is_japanese_ime),
                ime_on: None,
                is_romaji: None,
                conversion_mode: None,
                is_tsf_native: true,
            };
        }
    }

    // 2. Cross-process IME ON/OFF → ime_on (using focused hwnd)
    // SAFETY: detect_ime_open_for_hwnd は unsafe fn で、focused_hwnd は get_gui_thread_info_with_timeout
    //         が返した値（NULL の場合は HWND::default() にフォールバック済み）。NULL チェックは内部で行われる。
    let ime_on = unsafe { detect_ime_open_for_hwnd(focused_hwnd) };

    // 3. Cross-process conversion mode → is_romaji + conversion_mode (using focused hwnd)
    // SAFETY: detect_ime_conversion_for_hwnd は unsafe fn で、focused_hwnd は同上の条件を満たす。
    let conversion_mode = unsafe { detect_ime_conversion_for_hwnd(focused_hwnd) };

    // 4. Determine is_romaji from cross-process and direct check
    let is_romaji = conversion_mode.map_or_else(
        || {
            // cross-process 失敗: direct のみで試行
            // SAFETY: detect_kana_for_hwnd は unsafe fn で、focused_hwnd は同上の条件を満たす。
            unsafe { detect_kana_for_hwnd(focused_hwnd) }.map(|is_kana| !is_kana)
        },
        |conversion| {
            let is_native = conversion & IME_CMODE_NATIVE != 0;
            let is_roman = conversion & IME_CMODE_ROMAN != 0;

            if !is_native {
                None
            } else if is_roman {
                Some(true)
            } else {
                // ROMAN フラグなし + NATIVE あり: 直接 API で二重チェック
                // （一部 IME は ROMAN を返さないため）
                // SAFETY: detect_kana_for_hwnd は unsafe fn で、focused_hwnd は同上の条件を満たす。
                let direct = unsafe { detect_kana_for_hwnd(focused_hwnd) };
                log::debug!(
                    "read_ime_state_full: cross native={is_native} roman={is_roman}, direct_kana={direct:?}"
                );
                direct.map(|is_kana| !is_kana)
            }
        },
    );

    ImeSnapshot {
        is_japanese_ime: Some(is_japanese_ime),
        ime_on,
        is_romaji,
        conversion_mode,
        is_tsf_native: false,
    }
}

/// `read_ime_state_full` の async 版（ワーカースレッドで実行）
#[allow(clippy::future_not_send)]
pub async fn read_ime_state_full_async() -> ImeSnapshot {
    // SAFETY: read_ime_state_full は unsafe fn。win32_async::offload はワーカースレッドで実行するが
    //         IMM32 API はワーカースレッドからも呼び出し可能。
    win32_async::offload(|| unsafe { read_ime_state_full() }).await
}

/// `read_ime_state_fast` の async 版（ワーカースレッドで実行）
#[allow(clippy::future_not_send)]
pub async fn read_ime_state_fast_async() -> FastImeProbeResult {
    // SAFETY: read_ime_state_fast は unsafe fn。win32_async::offload はワーカースレッドで実行するが
    //         IMM32 API はワーカースレッドからも呼び出し可能。
    win32_async::offload(|| unsafe { read_ime_state_fast() }).await
}

/// `set_ime_open_cross_process` の async 版（ワーカースレッドで実行）
#[allow(clippy::future_not_send)]
pub async fn set_ime_open_cross_process_async(open: bool) -> bool {
    // SAFETY: set_ime_open_cross_process は unsafe fn。win32_async::offload はワーカースレッドで実行するが
    //         SendMessageTimeoutW はクロスプロセス呼び出しのためスレッドに依存しない。
    win32_async::offload(move || unsafe { set_ime_open_cross_process(open) }).await
}

/// `set_ime_romaji_mode` の async 版（ワーカースレッドで実行）
#[allow(clippy::future_not_send)]
pub async fn set_ime_romaji_mode_async() -> bool {
    // SAFETY: set_ime_romaji_mode は unsafe fn。win32_async::offload はワーカースレッドで実行するが
    //         SendMessageTimeoutW はクロスプロセス呼び出しのためスレッドに依存しない。
    win32_async::offload(|| unsafe { set_ime_romaji_mode() }).await
}

/// 現在のキーボードレイアウトの言語情報を返す。
///
/// Returns `(is_japanese, lang_id)` — 日本語レイアウトかどうかと言語 ID (下位16ビット)。
#[must_use]
pub fn keyboard_layout_info() -> (bool, u32) {
    // SAFETY: GetKeyboardLayout はスレッドセーフで任意のスレッドから呼び出せる。
    //         引数 0 は現在のスレッドのキーボードレイアウトを取得することを意味し、常に有効。
    unsafe {
        let hkl = GetKeyboardLayout(0);
        let lang_id = hkl.0 as u32 & 0xFFFF;
        (lang_id == crate::vk::LANGID_JAPANESE, lang_id)
    }
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
#[must_use] 
pub unsafe fn read_ime_state_fast() -> FastImeProbeResult {
    let (is_japanese_ime, _) = keyboard_layout_info();

    if !is_japanese_ime {
        return FastImeProbeResult { is_japanese_ime: false, ime_on: Some(false), is_imm_bridge_broken: false };
    }

    // GetForegroundWindow() はトップレベルウィンドウを返す。
    // read_ime_state_full が使う GetGUIThreadInfo().hwndFocus（子ウィンドウ）と異なり、
    // トップレベル hwnd は TSF 互換ブリッジ経由で IMM32 API に応答できる場合が多い。
    // SAFETY: GetForegroundWindow はスレッドセーフで、NULL を返す場合は non_null_hwnd が None を返し
    //         早期リターンする。
    let Some(hwnd) = crate::win32::non_null_hwnd(unsafe { GetForegroundWindow() }) else {
        return FastImeProbeResult { is_japanese_ime: true, ime_on: None, is_imm_bridge_broken: false };
    };

    // クラス名を一度取得して both チェックで使い回す。
    let class_name = crate::focus::classify::get_class_name_string(hwnd);

    // Alt/Win キーなどで一時的に現れるシステム UI オーバーレイは imc_open=false を返すため
    // Engine が誤 deactivate される。ime_on=None（既存状態維持）を返して誤検出を防ぐ。
    if is_tsf_native_window(&class_name) {
        log::debug!("read_ime_state_fast: transient system overlay → ime_on=None (preserving state)");
        return FastImeProbeResult { is_japanese_ime: true, ime_on: None, is_imm_bridge_broken: false };
    }

    // IMM-broken ウィンドウ（Chrome/Edge: Chrome_WidgetWin_1 等）は IMC_GETOPENSTATUS が常に 0、
    // IME_CMODE_NATIVE も TSF 管理と非同期で信頼できない。ime_on=None で shadow 状態に委ねる。
    if crate::focus::classify::is_imm_bridge_broken(&class_name) {
        log::debug!("read_ime_state_fast: IMM-broken({class_name}) → ime_on=None (shadow preserving)");
        return FastImeProbeResult { is_japanese_ime: true, ime_on: None, is_imm_bridge_broken: true };
    }

    // SAFETY: hwnd は non_null_hwnd で NULL チェック済みの有効なウィンドウハンドル。
    let Some(ime_wnd) = (unsafe { crate::imm::get_ime_wnd(hwnd) }) else {
        return FastImeProbeResult { is_japanese_ime: true, ime_on: None, is_imm_bridge_broken: false };
    };

    let imc_open =
        unsafe { crate::imm::send_ime_control(ime_wnd, IMC_GETOPENSTATUS, 0, 20) }
            .map(|v| v != 0);

    // 通常パス: conversion mode → 診断ログのみ（is_romaji 更新は read_ime_state_full に委ねる）
    // IMM32 ブリッジは WezTerm 等の TSF アプリでローマ字モードでも ROMAN ビットを
    // 報告しないことがある。ROMAN ビット不在を「かな入力」と断定するのは誤検出を招く。
    // SAFETY: ime_wnd は get_ime_wnd が返した有効な IME ウィンドウハンドル。タイムアウト 20ms 付き。
    if let Some(conv) =
        unsafe { crate::imm::send_ime_control(ime_wnd, IMC_GETCONVERSIONMODE, 0, 20) }
    {
        let conv = conv as u32;
        let is_native = conv & IME_CMODE_NATIVE != 0;
        let is_roman = conv & IME_CMODE_ROMAN != 0;
        log::debug!("read_ime_state_fast: conv=0x{conv:08X} native={is_native} roman={is_roman}");
    }

    FastImeProbeResult { is_japanese_ime: true, ime_on: imc_open, is_imm_bridge_broken: false }
}

/// 高速プローブの結果
#[derive(Debug)]
pub struct FastImeProbeResult {
    pub is_japanese_ime: bool,
    pub ime_on: Option<bool>,
    /// IMM-broken クラス（Chrome/Edge 等）かどうか。
    /// true のとき ime_on は常に None（shadow 状態で管理）。
    pub is_imm_bridge_broken: bool,
}

// ─── TSF probe helpers ────────────────────────────────────────

/// キーボードフォーカスウィンドウの HWND を返す。
///
/// `GetGUIThreadInfo().hwndFocus`（実際のフォーカス子ウィンドウ）を優先し、
/// 取得失敗時は `GetForegroundWindow()` にフォールバックする。
///
/// # Safety
/// Win32 API を呼び出す。
#[must_use] 
pub unsafe fn get_focused_hwnd() -> HWND {
    let gui =
        crate::win32::get_gui_thread_info_with_timeout(std::time::Duration::from_millis(30));
    // SAFETY: GetForegroundWindow はスレッドセーフで任意のスレッドから呼び出せる。
    //         focused_hwnd が None の場合のフォールバックとして使用するため、返り値が NULL の
    //         可能性は呼び出し元が non_null_hwnd 等でチェックすること。
    gui.focused_hwnd.unwrap_or_else(|| unsafe { GetForegroundWindow() })
}

/// VK_DBE_HIRAGANA (F2) を `SendMessageTimeoutW` でフォーカスウィンドウの wndproc に直接届ける。
///
/// `SendInput` は OS 入力キューを経由するため、その後の `SendMessageTimeoutW` による
/// probe よりも低優先度で処理される（QS_SENDMESSAGE > QS_INPUT）。
/// 本関数は入力キューを迂回して wndproc に同期的に届けるため、return 後は
/// Chrome が WM_KEYDOWN を処理済みであることが保証される。
///
/// Returns `true` if both WM_KEYDOWN and WM_KEYUP were delivered without timeout.
///
/// # Safety
/// Calls Win32 APIs. Must be called from the main thread.
#[must_use]
pub unsafe fn send_f2_via_sendmessage() -> bool {
    const VK_DBE_HIRAGANA: u32 = 0xF2;
    // SAFETY: get_focused_hwnd は unsafe fn で GetForegroundWindow または GetGUIThreadInfo から
    //         HWND を返す。non_null_hwnd で NULL チェックを行い、NULL なら早期リターンする。
    let Some(hwnd) = crate::win32::non_null_hwnd(unsafe { get_focused_hwnd() }) else {
        return false;
    };
    // SAFETY: MapVirtualKeyW はスレッドセーフで任意のスレッドから呼び出せる。
    //         VK_DBE_HIRAGANA (0xF2) は有効な仮想キーコードであり MAPVK_VK_TO_VSC は有効な変換タイプ。
    let scan = unsafe { MapVirtualKeyW(VK_DBE_HIRAGANA, MAPVK_VK_TO_VSC) };
    let lparam_down = LPARAM(1_isize | (isize::try_from(scan).unwrap_or(0) << 16));
    let lparam_up = LPARAM(lparam_down.0 | (1 << 30) | (1_isize << 31));
    let mut result = 0usize;
    // SAFETY: hwnd は non_null_hwnd で NULL チェック済みの有効なウィンドウハンドル。
    //         result はスタック上の初期化済み変数へのポインタで呼び出し中は有効。
    //         SMTO_ABORTIFHUNG + タイムアウト 100ms により応答なしプロセスでもブロックしない。
    let ok_down = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_KEYDOWN,
            WPARAM(VK_DBE_HIRAGANA as usize),
            lparam_down,
            SMTO_ABORTIFHUNG,
            100,
            Some(&raw mut result),
        )
    };
    // SAFETY: hwnd は non_null_hwnd で NULL チェック済みの有効なウィンドウハンドル。
    //         result はスタック上の初期化済み変数へのポインタで呼び出し中は有効。
    //         SMTO_ABORTIFHUNG + タイムアウト 100ms により応答なしプロセスでもブロックしない。
    let ok_up = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_KEYUP,
            WPARAM(VK_DBE_HIRAGANA as usize),
            lparam_up,
            SMTO_ABORTIFHUNG,
            100,
            Some(&raw mut result),
        )
    };
    let success = ok_down.0 != 0 && ok_up.0 != 0;
    log::debug!("[f2-sendmsg] hwnd={hwnd:?} scan=0x{scan:02X} success={success}");
    success
}

/// フォーカスウィンドウの IMM32 HIMC に composition string が存在するか確認する。
///
/// TSF warm probe 用。TSF が active な場合、romaji キー到達後に composition string が
/// 非空になる。TSF が cold（未初期化）な場合、キーはリテラルとして抜けるため空のまま。
///
/// クロスプロセスで `ImmGetCompositionStringW`（GCS_COMPSTR）を呼び出す。
/// TSF→IMM32 bridge が HIMC を更新するため、外部プロセスからも読み取り可能。
///
/// # Safety
/// Win32 API を呼び出す。
#[must_use] 
pub unsafe fn check_tsf_composition_active(hwnd: HWND) -> bool {
    if crate::win32::non_null_hwnd(hwnd).is_none() {
        return false;
    }
    // SAFETY: hwnd は non_null_hwnd で NULL チェック済みの有効なウィンドウハンドル。
    //         ImmContextGuard は ImmGetContext/ImmReleaseContext を RAII で管理し、
    //         NULL HIMC を取得した場合は None を返す。
    let Some(ctx) = (unsafe { crate::imm::ImmContextGuard::new(hwnd) }) else {
        return false;
    };
    // GCS_COMPSTR = IME_COMPOSITION_STRING(0x0008): null バッファで呼ぶと composition string のバイト長を返す
    // SAFETY: ctx.himc() は ImmContextGuard が保持する有効な HIMC。
    //         lpBuf=None かつ dwBufLen=0 で呼ぶのは MSDN で明示的に許可されており
    //         バッファオーバーフローの危険はない。
    let len = unsafe {
        ImmGetCompositionStringW(ctx.himc(), IME_COMPOSITION_STRING(0x0008_u32), None, 0)
    };
    len > 0
}
