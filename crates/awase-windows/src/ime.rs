use std::mem::size_of;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetConversionStatus, ImmGetOpenStatus, IME_COMPOSITION_STRING,
    IME_CONVERSION_MODE, IME_SENTENCE_MODE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, MapVirtualKeyW, SendInput, INPUT, MAPVK_VK_TO_VSC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_KEYDOWN, WM_KEYUP,
};

use crate::focus::class_names::is_tsf_native_window;
use crate::imm::{
    IMC_GETCONVERSIONMODE, IMC_GETOPENSTATUS, IMC_SETCONVERSIONMODE, IMC_SETOPENSTATUS,
    IME_CMODE_NATIVE, IME_CMODE_ROMAN,
};
use crate::win32::HwndExt as _;

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
    let t0 = std::time::Instant::now();
    let gui_result =
        crate::win32::get_gui_thread_info_with_timeout(std::time::Duration::from_millis(150));
    let gui_elapsed = t0.elapsed();
    let Some(hwnd) = gui_result.focused_hwnd else {
        log::debug!(
            "set_ime_open_cross_process: open={open} gui_elapsed={}ms → no focused hwnd, abort",
            gui_elapsed.as_millis()
        );
        return false;
    };
    // SAFETY: hwnd は get_gui_thread_info_with_timeout が返した有効なフォーカスウィンドウハンドル。
    //         get_ime_wnd は内部で ImmGetDefaultIMEWnd を呼ぶ安全なラッパーであり、NULL を返す場合は
    //         直後の `?` でショートサーキットするため問題ない。
    let Some(ime_wnd) = (unsafe { crate::imm::get_ime_wnd(hwnd) }) else {
        log::debug!(
            "set_ime_open_cross_process: hwnd={hwnd:?} open={open} gui_elapsed={}ms → no IME wnd, abort",
            gui_elapsed.as_millis()
        );
        return false;
    };
    // SAFETY: ime_wnd は get_ime_wnd が返した有効な IME ウィンドウハンドル。
    //         send_ime_control は SendMessageTimeoutW のラッパーであり、タイムアウト付きのため
    //         相手プロセスがハングしても指定時間後に制御が戻る。
    // タイムアウト 150ms: IME OFF (open=false) は composition tear-down と IME UI 隠蔽が走るため
    // 50ms では時々取りこぼす（Ctrl+無変換 が「時々」効かない症状の原因）。Get 系の照会は短く
    // 維持し、Set 系のみ余裕を持たせる。
    let t_send = std::time::Instant::now();
    let success =
        unsafe { crate::imm::send_ime_control(ime_wnd, IMC_SETOPENSTATUS, isize::from(open), 150) }
            .is_some();
    let send_elapsed = t_send.elapsed();
    // 診断: Ctrl+無変換 で前文字消失調査用。タイムアウトに近いケースと partial commit の
    // 関係を切り分けるため、GetGUIThreadInfo / send_ime_control の所要時間と現時点で
    // observer 側が把握している candidate (composition 可視) を一緒に出す。
    let candidate_visible = crate::tsf::observer::gji_candidate_visible_now();
    log::debug!(
        "set_ime_open_cross_process: hwnd={hwnd:?} ime_wnd={ime_wnd:?} open={open} success={success} \
         gui_elapsed={}ms send_elapsed={}ms candidate_visible={candidate_visible}",
        gui_elapsed.as_millis(),
        send_elapsed.as_millis()
    );
    success
}

/// 修飾キー（Ctrl / Shift / Alt）の押下状態スナップショット。
///
/// `SendInput` で修飾なしキーを届ける際の解放・復元シーケンス構築に使う。
/// 3つの IME キー送信関数（VK_KANJI / F13 / F14）が同じパターンを共有する。
struct HeldModifiers {
    ctrl: bool,
    shift: bool,
    alt: bool,
}

impl HeldModifiers {
    /// `GetAsyncKeyState` で現在の物理押下状態を読み取る。
    ///
    /// # Safety
    /// Win32 API を呼び出す。
    unsafe fn read() -> Self {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
        };
        Self {
            ctrl: unsafe { GetAsyncKeyState(i32::from(VK_CONTROL.0)) } < 0,
            shift: unsafe { GetAsyncKeyState(i32::from(VK_SHIFT.0)) } < 0,
            alt: unsafe { GetAsyncKeyState(i32::from(VK_MENU.0)) } < 0,
        }
    }

    /// 押下中の修飾キーを解放する `INPUT` イベントを追加する。
    fn push_release(&self, inputs: &mut Vec<INPUT>) {
        use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};
        use crate::vk::{VK_CONTROL, VK_MENU, VK_SHIFT};
        if self.ctrl {
            inputs.push(make_key_input_ex(VK_CONTROL, true, IME_KANJI_MARKER));
        }
        if self.shift {
            inputs.push(make_key_input_ex(VK_SHIFT, true, IME_KANJI_MARKER));
        }
        if self.alt {
            inputs.push(make_key_input_ex(VK_MENU, true, IME_KANJI_MARKER));
        }
    }

    /// 物理的にまだ押下中の修飾キーを復元する `INPUT` イベントを追加し、復元した状態を返す。
    ///
    /// # Safety
    /// Win32 API を呼び出す。
    unsafe fn push_restore(&self, inputs: &mut Vec<INPUT>) -> Self {
        use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};
        use crate::vk::{
            VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_MENU, VK_RMENU, VK_RSHIFT, VK_SHIFT,
        };
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let still = Self {
            ctrl: self.ctrl
                && (unsafe { GetAsyncKeyState(i32::from(VK_LCONTROL.0)) } < 0
                    || unsafe { GetAsyncKeyState(i32::from(VK_CONTROL.0)) } < 0),
            shift: self.shift
                && (unsafe { GetAsyncKeyState(i32::from(VK_LSHIFT.0)) } < 0
                    || unsafe { GetAsyncKeyState(i32::from(VK_RSHIFT.0)) } < 0),
            alt: self.alt
                && (unsafe { GetAsyncKeyState(i32::from(VK_LMENU.0)) } < 0
                    || unsafe { GetAsyncKeyState(i32::from(VK_RMENU.0)) } < 0),
        };
        if still.ctrl {
            inputs.push(make_key_input_ex(VK_CONTROL, false, IME_KANJI_MARKER));
        }
        if still.shift {
            inputs.push(make_key_input_ex(VK_SHIFT, false, IME_KANJI_MARKER));
        }
        if still.alt {
            inputs.push(make_key_input_ex(VK_MENU, false, IME_KANJI_MARKER));
        }
        still
    }
}

/// IMM32 クロスプロセス制御が使えないアプリ（Chrome/Edge 等）向け IME トグル実装。
///
/// `WM_IME_CONTROL` が効かない `Imm32Unavailable` アプリに対して `SendInput(VK_KANJI)` で IME をトグルする。
///
/// VK_KANJI はトグルキーのため **呼び出し元は last_applied_ime_on != desired を事前確認すること**。
/// `dwExtraInfo` に `IME_KANJI_MARKER` を付けるため awase 自身のフックが再インターセプトしない
/// （フック先頭の自己注入チェックで即パススルー、shadow toggle もスキップ）。
///
/// Ctrl/Shift/Alt が押下中の場合、VK_KANJI を bare（修飾なし）で届けるために先に KeyUp を注入し、
/// 送信後も物理的に押下中の修飾キーは KeyDown で復元する。
///
/// 候補ウィンドウ表示中は VK_KANJI が候補窓に吸われて IME OFF に失敗する場合があるが、
/// 以前の「Ctrl+Enter で候補確定後に VK_KANJI」方式は Chrome フォームを submit させる
/// 副作用があったため廃止。GJI 環境では GjiDirectStrategy (F14) が先行するため、
/// この関数に到達するのは GJI 以外か GJI fallback 時のみ。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
// 変数名が意図的に似ているため similar_names を抑制する（gas_lctrl/gks_lctrl 等）。
#[allow(clippy::similar_names)]
pub unsafe fn post_kanji_toggle_to_focused() {
    use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};
    use crate::vk::{
        VK_CONTROL, VK_KANJI, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_RCONTROL, VK_RMENU, VK_RSHIFT,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState};

    // SAFETY: GetAsyncKeyState / GetKeyState はスレッドセーフで任意のスレッドから呼び出せる。
    let held = unsafe { HeldModifiers::read() };

    // 診断: L/R 個別キー状態（VK_KANJI 受信時の Edge 挙動把握用）
    // GetAsyncKeyState = 物理キー状態、GetKeyState = メッセージキュー処理済み状態。
    let (
        gas_lctrl,
        gas_rctrl,
        gks_ctrl,
        gks_lctrl,
        gks_rctrl,
        gas_lshift,
        gas_rshift,
        gas_lalt,
        gas_ralt,
    ) = (
        unsafe { GetAsyncKeyState(i32::from(VK_LCONTROL.0)) } < 0,
        unsafe { GetAsyncKeyState(i32::from(VK_RCONTROL.0)) } < 0,
        unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0,
        unsafe { GetKeyState(i32::from(VK_LCONTROL.0)) } < 0,
        unsafe { GetKeyState(i32::from(VK_RCONTROL.0)) } < 0,
        unsafe { GetAsyncKeyState(i32::from(VK_LSHIFT.0)) } < 0,
        unsafe { GetAsyncKeyState(i32::from(VK_RSHIFT.0)) } < 0,
        unsafe { GetAsyncKeyState(i32::from(VK_LMENU.0)) } < 0,
        unsafe { GetAsyncKeyState(i32::from(VK_RMENU.0)) } < 0,
    );
    log::debug!(
        "[ime-fallback] key-state pre-send: \
         ctrl(gas={} L={gas_lctrl} R={gas_rctrl}) \
         gks(ctrl={gks_ctrl} L={gks_lctrl} R={gks_rctrl}) \
         shift(gas={} L={gas_lshift} R={gas_rshift}) \
         alt(gas={} L={gas_lalt} R={gas_ralt})",
        held.ctrl,
        held.shift,
        held.alt
    );

    let mut inputs = Vec::with_capacity(8);
    held.push_release(&mut inputs);
    inputs.push(make_key_input_ex(VK_KANJI, false, IME_KANJI_MARKER));
    inputs.push(make_key_input_ex(VK_KANJI, true, IME_KANJI_MARKER));

    // SAFETY: GetAsyncKeyState はスレッドセーフで任意のスレッドから呼び出せる。
    let still = unsafe { held.push_restore(&mut inputs) };

    log::debug!(
        "[ime-fallback] SendInput VK_KANJI toggle: \
         release(ctrl={} shift={} alt={}) \
         restore(ctrl={} shift={} alt={}) total={} events",
        held.ctrl,
        held.shift,
        held.alt,
        still.ctrl,
        still.shift,
        still.alt,
        inputs.len()
    );
    // SAFETY: inputs は make_key_input_ex で正しく初期化された INPUT の Vec であり、
    //         size_of::<INPUT>() は正確な構造体サイズを返す。
    //         SendInput はスレッドセーフで任意のスレッドから呼び出せる。
    let candidate_pre = crate::tsf::observer::gji_candidate_visible_now();
    let t_send = std::time::Instant::now();
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    let send_elapsed = t_send.elapsed();
    let candidate_post = crate::tsf::observer::gji_candidate_visible_now();
    log::debug!(
        "[ime-fallback] SendInput VK_KANJI done: send_elapsed={}ms candidate_pre={candidate_pre} candidate_post={candidate_post} sent={sent}/{}",
        send_elapsed.as_millis(),
        inputs.len()
    );
    if sent as usize != inputs.len() {
        log::warn!(
            "[ime-fallback] SendInput(VK_KANJI) sent {sent}/{} events",
            inputs.len()
        );
    }
}

/// GJI 専用 IME ON: F13 を送信してひらがなモードに切り替える。
///
/// GJI の直接入力モード（IME OFF 状態）で F13 を押すとひらがな入力に切り替わる。
/// 既に ON の場合は no-op（冪等）。候補ウィンドウ表示中も no-op（入力は継続）。
///
/// VK_KANJI と異なりトグルではないため shadow desync の影響を受けない。
/// F13 は実キーボードに存在しないためアプリ側 shortcut との衝突リスクがない。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn post_gji_ime_on() {
    use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};
    use crate::vk::VK_F13;

    // SAFETY: GetAsyncKeyState はスレッドセーフで任意のスレッドから呼び出せる。
    let held = unsafe { HeldModifiers::read() };
    let mut inputs: Vec<INPUT> = Vec::with_capacity(8);

    held.push_release(&mut inputs);
    inputs.push(make_key_input_ex(VK_F13, false, IME_KANJI_MARKER));
    inputs.push(make_key_input_ex(VK_F13, true, IME_KANJI_MARKER));
    let still = unsafe { held.push_restore(&mut inputs) };

    log::debug!(
        "[gji-on] F13: release(ctrl={} shift={} alt={}) \
         restore(ctrl={} shift={} alt={}) total={} events",
        held.ctrl,
        held.shift,
        held.alt,
        still.ctrl,
        still.shift,
        still.alt,
        inputs.len()
    );
    // SAFETY: inputs は make_key_input_ex で正しく初期化された INPUT の Vec。
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        log::warn!("[gji-on] SendInput F13 sent {sent}/{}", inputs.len());
    }
}

/// GJI 専用 IME OFF: F14 を送信して IME を無効化する。
///
/// GJI の config1.db に `Precomposition/Composition/Conversion\tF14\tIMEOff` を
/// 登録することで、物理キーボードに存在しない F14（VK=0x7D）を IME OFF キーとして使用する。
///
/// F14 は実キーボードに存在せずブラウザショートカットとも衝突しないため、
/// 旧実装の Ctrl+Shift+Delete（Edge の「閲覧履歴削除」ショートカットと衝突）を置き換える。
///
/// GJI config で Conversion\tF14\tIMEOff が設定されているため、候補ウィンドウ表示中でも
/// F14 を直接送れば IME OFF になる。
/// GJI は DirectInput に F14 を登録していないため、IME が既に OFF の場合は
/// F14 がアプリにパススルーされるが、F14 は無害（ブラウザショートカットなし）。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn post_gji_ime_off() {
    use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};
    use crate::vk::VK_F14;

    // SAFETY: GetAsyncKeyState はスレッドセーフで任意のスレッドから呼び出せる。
    let held = unsafe { HeldModifiers::read() };
    let mut inputs: Vec<INPUT> = Vec::with_capacity(8);

    held.push_release(&mut inputs);
    inputs.push(make_key_input_ex(VK_F14, false, IME_KANJI_MARKER));
    inputs.push(make_key_input_ex(VK_F14, true, IME_KANJI_MARKER));
    let still = unsafe { held.push_restore(&mut inputs) };

    log::debug!(
        "[gji-off] F14: release(ctrl={} shift={} alt={}) \
         restore(ctrl={} shift={} alt={}) total={} events",
        held.ctrl,
        held.shift,
        held.alt,
        still.ctrl,
        still.shift,
        still.alt,
        inputs.len()
    );
    // SAFETY: inputs は make_key_input_ex で正しく初期化された INPUT の Vec。
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        log::warn!("[gji-off] SendInput F14 sent {sent}/{}", inputs.len());
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
/// Ctrl/Shift/Alt が押下中の場合（例: ユーザが Ctrl+無変換 で IME OFF を指示し、
/// その Ctrl がまだ OS に保持されている瞬間）、修飾なしで mode key を届けるために
/// 先に KeyUp を注入し、送信後に物理的に押下中の修飾キーは KeyDown で復元する。
/// これを行わないと OS/IME/アプリが `Ctrl+<mode key>` の組み合わせとして解釈し、
/// 想定外のショートカット発火を招く（`post_kanji_toggle_to_focused` と同じ理由）。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn send_ime_mode_key(vk: awase::types::VkCode) {
    use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};

    // SAFETY: GetAsyncKeyState はスレッドセーフ。
    let held = unsafe { HeldModifiers::read() };
    let mut inputs: Vec<INPUT> = Vec::with_capacity(8);
    held.push_release(&mut inputs);
    inputs.push(make_key_input_ex(vk, false, IME_KANJI_MARKER));
    inputs.push(make_key_input_ex(vk, true, IME_KANJI_MARKER));
    // SAFETY: GetAsyncKeyState はスレッドセーフ。
    let still = unsafe { held.push_restore(&mut inputs) };

    log::debug!(
        "[ime-mode] SendInput vk=0x{vk:02X} \
         release(ctrl={} shift={} alt={}) \
         restore(ctrl={} shift={} alt={}) total={} events",
        held.ctrl,
        held.shift,
        held.alt,
        still.ctrl,
        still.shift,
        still.alt,
        inputs.len()
    );
    // SAFETY: inputs は make_key_input_ex で正しく初期化された INPUT の Vec であり、
    //         size_of::<INPUT>() は正確な構造体サイズを返す。
    //         SendInput はスレッドセーフで任意のスレッドから呼び出せる。
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        log::warn!(
            "[ime-mode] SendInput(vk=0x{vk:02X}) sent {sent}/{} events",
            inputs.len()
        );
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
    //         detect_ime_conversion_for_hwnd 内の non_null() チェックで処理される。
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
    // SAFETY: GetForegroundWindow はスレッドセーフで、NULL を返す場合は non_null() が `?` で None を返す。
    let hwnd = unsafe { GetForegroundWindow() }.non_null()?;
    // SAFETY: hwnd は non_null() で NULL チェック済みの有効なウィンドウハンドル。
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
    // SAFETY: GetForegroundWindow はスレッドセーフで、NULL を返す場合は non_null() が None を返し
    //         早期リターンする。
    let Some(hwnd) = unsafe { GetForegroundWindow() }.non_null() else {
        return "null".to_string();
    };
    let class = crate::focus::classify::get_class_name_string(hwnd);
    if class.is_empty() {
        "unknown".to_string()
    } else {
        class
    }
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
    // SAFETY: GetForegroundWindow はスレッドセーフで、NULL を返す場合は non_null() が None を返し
    //         早期リターンする。
    let Some(hwnd) = unsafe { GetForegroundWindow() }.non_null() else {
        return false;
    };
    // SAFETY: hwnd は non_null() で NULL チェック済みの有効なウィンドウハンドル。
    let Some(ime_wnd) = (unsafe { crate::imm::get_ime_wnd(hwnd) }) else {
        return false;
    };

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
    let success = unsafe {
        crate::imm::send_ime_control(ime_wnd, IMC_SETCONVERSIONMODE, new_conv as isize, 50)
    }
    .is_some();
    log::debug!("[imm-romaji] conv 0x{conv:08X} → 0x{new_conv:08X} success={success}");
    success
}

// ─── hwnd 指定版クロスプロセス検出（read_ime_state_full 専用）─────

unsafe fn detect_ime_open_for_hwnd(hwnd: HWND) -> Option<bool> {
    hwnd.non_null()?;
    // SAFETY: hwnd は non_null() で NULL チェック済みの有効なウィンドウハンドル。
    let ime_wnd = unsafe { crate::imm::get_ime_wnd(hwnd) }?;
    // SAFETY: ime_wnd は get_ime_wnd が返した有効な IME ウィンドウハンドル。
    //         タイムアウト 50ms 付きで呼び出しているため応答なしプロセスでもブロックしない。
    let result = unsafe { crate::imm::send_ime_control(ime_wnd, IMC_GETOPENSTATUS, 0, 50) }?;
    log::trace!("CrossProcess(hwndFocus): ime_wnd={ime_wnd:?} open={result}");
    Some(result != 0)
}

unsafe fn detect_ime_conversion_for_hwnd(hwnd: HWND) -> Option<u32> {
    hwnd.non_null()?;
    // SAFETY: hwnd は non_null() で NULL チェック済みの有効なウィンドウハンドル。
    let ime_wnd = unsafe { crate::imm::get_ime_wnd(hwnd) }?;
    // SAFETY: ime_wnd は get_ime_wnd が返した有効な IME ウィンドウハンドル。
    //         タイムアウト 50ms 付きで呼び出しているため応答なしプロセスでもブロックしない。
    unsafe { crate::imm::send_ime_control(ime_wnd, IMC_GETCONVERSIONMODE, 0, 50) }.map(|v| v as u32)
}

unsafe fn detect_kana_for_hwnd(hwnd: HWND) -> Option<bool> {
    hwnd.non_null()?;
    // SAFETY: hwnd は non_null() で NULL チェック済みの有効なウィンドウハンドル。
    //         ImmContextGuard は ImmGetContext/ImmReleaseContext を RAII で管理し、
    //         NULL HIMC を取得した場合は None を返す。
    let ctx = unsafe { crate::imm::ImmContextGuard::new(hwnd) }?;
    let mut conversion = IME_CONVERSION_MODE::default();
    let mut sentence = IME_SENTENCE_MODE::default();
    // SAFETY: ctx.himc() は ImmContextGuard が保持する有効な HIMC。
    //         conversion と sentence はスタック上の初期化済み変数へのポインタであり呼び出し中は有効。
    let ok = unsafe {
        ImmGetConversionStatus(
            ctx.himc(),
            Some(&raw mut conversion),
            Some(&raw mut sentence),
        )
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
    crate::win32::run_with_timeout(timeout, || unsafe { read_ime_state_full() }).unwrap_or_else(
        || {
            log::warn!("read_ime_state_full timed out, returning empty snapshot");
            ImeSnapshot {
                is_japanese_ime: None,
                ime_on: None,
                is_romaji: None,
                conversion_mode: None,
                is_tsf_native: false,
            }
        },
    )
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
    let result =
        crate::win32::get_gui_thread_info_with_timeout(std::time::Duration::from_millis(200));
    // None（フォーカスウィンドウ不明）の場合は HWND::default() にフォールバックする。
    // detect_ime_open_for_hwnd 等は null HWND を適切に処理して None を返す。
    let focused_hwnd = result.focused_hwnd.unwrap_or_default();
    let thread_id = result.thread_id;

    // 1. Keyboard layout → is_japanese_ime
    let is_japanese_ime = {
        // SAFETY: GetKeyboardLayout はスレッドセーフで任意のスレッドから呼び出せる。
        //         thread_id は get_gui_thread_info_with_timeout が返した値で 0（現在スレッド）も許容される。
        let hkl = unsafe { GetKeyboardLayout(thread_id) };
        let lang_id = crate::imm::lang_id_from_hkl(hkl.0 as u32);
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

/// `send_f2_via_sendmessage` の async 版（ワーカースレッドで実行）
///
/// メインスレッドの `with_app` 再入を避けるため、`SendMessageTimeoutW` (×2) を
/// ワーカースレッドで実行する。メッセージループは await 中も継続する。
#[allow(clippy::future_not_send)]
pub async fn send_f2_via_sendmessage_async() -> bool {
    // SAFETY: send_f2_via_sendmessage は unsafe fn。win32_async::offload はワーカースレッドで実行するが
    //         SendMessageTimeoutW はクロスプロセス呼び出しのためスレッドに依存しない。
    win32_async::offload(|| unsafe { send_f2_via_sendmessage() }).await
}

/// `get_ime_conversion_mode_raw_timeout` の async 版（ワーカースレッドで実行）
///
/// 診断ログ用途で、`with_app` 再入を避けるためにワーカースレッドへオフロードする。
#[allow(clippy::future_not_send)]
pub async fn get_ime_conversion_mode_raw_timeout_async(timeout_ms: u32) -> Option<u32> {
    // SAFETY: get_ime_conversion_mode_raw_timeout は unsafe fn。win32_async::offload はワーカースレッドで実行するが
    //         SendMessageTimeoutW はクロスプロセス呼び出しのためスレッドに依存しない。
    win32_async::offload(move || unsafe { get_ime_conversion_mode_raw_timeout(timeout_ms) }).await
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
        let lang_id = crate::imm::lang_id_from_hkl(hkl.0 as u32);
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
        return FastImeProbeResult {
            is_japanese_ime: false,
            ime_on: Some(false),
        };
    }

    // GetForegroundWindow() はトップレベルウィンドウを返す。
    // read_ime_state_full が使う GetGUIThreadInfo().hwndFocus（子ウィンドウ）と異なり、
    // トップレベル hwnd は TSF 互換ブリッジ経由で IMM32 API に応答できる場合が多い。
    // SAFETY: GetForegroundWindow はスレッドセーフで、NULL を返す場合は non_null() が None を返し
    //         早期リターンする。
    let Some(hwnd) = unsafe { GetForegroundWindow() }.non_null() else {
        return FastImeProbeResult {
            is_japanese_ime: true,
            ime_on: None,
        };
    };

    // クラス名を一度取得して both チェックで使い回す。
    let class_name = crate::focus::classify::get_class_name_string(hwnd);
    let profile = crate::focus::classify::AppImeProfile::from_class_name(&class_name);

    // IMM/TSF いずれの経路でも IMC_GETOPENSTATUS が信頼できないアプリは
    // ime_on=None を返して shadow 状態に委ねる。
    // - TsfNative（Alt/Win 一時オーバーレイ等）: imc_open=false で Engine 誤 deactivate
    // - Imm32Unavailable（Chrome/Edge: Chrome_WidgetWin_1 等）: 常に 0 を返す
    if !profile.can_read_imm32_open_status() {
        log::debug!(
            "read_ime_state_fast: profile={profile:?} class={class_name} → ime_on=None (shadow preserving)"
        );
        return FastImeProbeResult {
            is_japanese_ime: true,
            ime_on: None,
        };
    }

    // SAFETY: hwnd は non_null() で NULL チェック済みの有効なウィンドウハンドル。
    let Some(ime_wnd) = (unsafe { crate::imm::get_ime_wnd(hwnd) }) else {
        return FastImeProbeResult {
            is_japanese_ime: true,
            ime_on: None,
        };
    };

    let imc_open =
        unsafe { crate::imm::send_ime_control(ime_wnd, IMC_GETOPENSTATUS, 0, 20) }.map(|v| v != 0);

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

    FastImeProbeResult {
        is_japanese_ime: true,
        ime_on: imc_open,
    }
}

/// 高速プローブの結果。
///
/// `Imm32Unavailable` / `TsfNative` の判定は `AppKindClassifier::current_app_profile` に集約されており
/// 本構造体には含まない。`ime_on=None` は「OS から信頼できる値を読めなかった」ことを意味する。
#[derive(Debug)]
pub struct FastImeProbeResult {
    pub is_japanese_ime: bool,
    pub ime_on: Option<bool>,
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
    let gui = crate::win32::get_gui_thread_info_with_timeout(std::time::Duration::from_millis(30));
    // SAFETY: GetForegroundWindow はスレッドセーフで任意のスレッドから呼び出せる。
    //         focused_hwnd が None の場合のフォールバックとして使用するため、返り値が NULL の
    //         可能性は呼び出し元が non_null() 等でチェックすること。
    gui.focused_hwnd
        .unwrap_or_else(|| unsafe { GetForegroundWindow() })
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
    // SAFETY: get_focused_hwnd は unsafe fn で GetForegroundWindow または GetGUIThreadInfo から
    //         HWND を返す。non_null() で NULL チェックを行い、NULL なら早期リターンする。
    let Some(hwnd) = unsafe { get_focused_hwnd() }.non_null() else {
        return false;
    };
    // SAFETY: MapVirtualKeyW はスレッドセーフで任意のスレッドから呼び出せる。
    //         VK_DBE_HIRAGANA (0xF2) は有効な仮想キーコードであり MAPVK_VK_TO_VSC は有効な変換タイプ。
    let scan = unsafe { MapVirtualKeyW(u32::from(crate::vk::VK_DBE_HIRAGANA.0), MAPVK_VK_TO_VSC) };
    let lparam_down = LPARAM(1_isize | (isize::try_from(scan).unwrap_or(0) << 16));
    let lparam_up = LPARAM(lparam_down.0 | (1 << 30) | (1_isize << 31));
    let mut result = 0usize;
    // SAFETY: hwnd は non_null() で NULL チェック済みの有効なウィンドウハンドル。
    //         result はスタック上の初期化済み変数へのポインタで呼び出し中は有効。
    //         SMTO_ABORTIFHUNG + タイムアウト 100ms により応答なしプロセスでもブロックしない。
    let ok_down = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_KEYDOWN,
            WPARAM(crate::vk::VK_DBE_HIRAGANA.0 as usize),
            lparam_down,
            SMTO_ABORTIFHUNG,
            100,
            Some(&raw mut result),
        )
    };
    // SAFETY: hwnd は non_null() で NULL チェック済みの有効なウィンドウハンドル。
    //         result はスタック上の初期化済み変数へのポインタで呼び出し中は有効。
    //         SMTO_ABORTIFHUNG + タイムアウト 100ms により応答なしプロセスでもブロックしない。
    let ok_up = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_KEYUP,
            WPARAM(crate::vk::VK_DBE_HIRAGANA.0 as usize),
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
    if hwnd.non_null().is_none() {
        return false;
    }
    // SAFETY: hwnd は non_null() で NULL チェック済みの有効なウィンドウハンドル。
    //         ImmContextGuard は ImmGetContext/ImmReleaseContext を RAII で管理し、
    //         NULL HIMC を取得した場合は None を返す。
    let Some(ctx) = (unsafe { crate::imm::ImmContextGuard::new(hwnd) }) else {
        return false;
    };
    // GCS_COMPSTR: null バッファで呼ぶと composition string のバイト長を返す
    // SAFETY: ctx.himc() は ImmContextGuard が保持する有効な HIMC。
    //         lpBuf=None かつ dwBufLen=0 で呼ぶのは MSDN で明示的に許可されており
    //         バッファオーバーフローの危険はない。
    let len = unsafe {
        ImmGetCompositionStringW(
            ctx.himc(),
            IME_COMPOSITION_STRING(crate::imm::GCS_COMPSTR),
            None,
            0,
        )
    };
    len > 0
}

/// `ImmGetCompositionStringW` の各 index を読み取って診断用スナップショットを返す。
///
/// 部分リテラル検出の実験用。送信した romaji が composition に正しく入ったかを
/// 観測するため、composition の各種情報および IME 状態を取得する。
///
/// 取得失敗時（HIMC NULL、API エラー、空など）は対応フィールドが None になる。
/// Imm32Unavailable / TsfNative プロファイルでは `himc_null=true` となり全フィールドが None。
///
/// # Safety
/// Win32 API を呼び出す。
#[must_use]
pub unsafe fn capture_composition_snapshot(hwnd: HWND) -> CompositionSnapshot {
    use crate::imm::{
        GCS_COMPATTR, GCS_COMPREADSTR, GCS_COMPSTR, GCS_CURSORPOS, GCS_RESULTREADSTR, GCS_RESULTSTR,
    };
    let mut snap = CompositionSnapshot::default();
    if hwnd.non_null().is_none() {
        return snap;
    }
    // SAFETY: hwnd は non_null() で NULL チェック済み。
    let Some(ctx) = (unsafe { crate::imm::ImmContextGuard::new(hwnd) }) else {
        snap.himc_null = true;
        return snap;
    };
    // 現在 composition 中の文字列
    snap.comp_str = unsafe { read_imm_string(ctx.himc(), GCS_COMPSTR) };
    // 確定済みの文字列
    snap.result_str = unsafe { read_imm_string(ctx.himc(), GCS_RESULTSTR) };
    // composition の読み（ローマ字相当）
    snap.comp_read_str = unsafe { read_imm_string(ctx.himc(), GCS_COMPREADSTR) };
    // 確定済みの読み
    snap.result_read_str = unsafe { read_imm_string(ctx.himc(), GCS_RESULTREADSTR) };
    // カーソル位置
    snap.cursor_pos = unsafe { read_imm_i32(ctx.himc(), GCS_CURSORPOS) };
    // 各文字の属性（0=入力/1=変換中/2=変換済/3=固定）
    snap.comp_attr_bytes = unsafe { read_imm_bytes(ctx.himc(), GCS_COMPATTR) };
    // ImmGetOpenStatus: IME 開閉状態
    // SAFETY: ctx.himc() は有効な HIMC。ImmGetOpenStatus はクラッシュしない読み取り API。
    snap.open_status = Some(unsafe { ImmGetOpenStatus(ctx.himc()).as_bool() });
    // ImmGetConversionStatus: 変換モード + 文節モード
    let mut conv = IME_CONVERSION_MODE::default();
    let mut sent = IME_SENTENCE_MODE::default();
    // SAFETY: ctx.himc() は有効。書き込み先は both null でない（&raw mut）。
    let ok =
        unsafe { ImmGetConversionStatus(ctx.himc(), Some(&raw mut conv), Some(&raw mut sent)) };
    if ok.as_bool() {
        snap.conversion_mode = Some(conv.0);
        snap.sentence_mode = Some(sent.0);
    }
    snap
}

/// `ImmGetCompositionStringW` で composition の各 index を文字列として読み取る。
///
/// 戻り値: 取得成功時は `Some(String)`、API エラー/長さ <=0 のときは `None`、長さ 0 は `Some("")`。
unsafe fn read_imm_string(
    himc: windows::Win32::UI::Input::Ime::HIMC,
    index: u32,
) -> Option<String> {
    // SAFETY: lpBuf=None かつ dwBufLen=0 で呼んでバイト長を取得する公式パターン。
    let byte_len =
        unsafe { ImmGetCompositionStringW(himc, IME_COMPOSITION_STRING(index), None, 0) };
    if byte_len < 0 {
        return None;
    }
    let byte_len = usize::try_from(byte_len).unwrap_or(0);
    if byte_len == 0 {
        return Some(String::new());
    }
    let mut buf = vec![0u16; byte_len.div_ceil(2)];
    // SAFETY: buf は十分なサイズを確保済み。WCHAR バッファとして書き込まれる。
    let written = unsafe {
        ImmGetCompositionStringW(
            himc,
            IME_COMPOSITION_STRING(index),
            Some(buf.as_mut_ptr().cast()),
            u32::try_from(buf.len() * 2).unwrap_or(0),
        )
    };
    if written <= 0 {
        return None;
    }
    let char_count = usize::try_from(written).unwrap_or(0) / 2;
    Some(String::from_utf16_lossy(&buf[..char_count]))
}

/// `ImmGetCompositionStringW` で int (cursor pos など) を読み取る。
unsafe fn read_imm_i32(himc: windows::Win32::UI::Input::Ime::HIMC, index: u32) -> Option<i32> {
    // GCS_CURSORPOS / GCS_DELTASTART は LOWORD に値が入る。null バッファ呼び出しが値を返す。
    let v = unsafe { ImmGetCompositionStringW(himc, IME_COMPOSITION_STRING(index), None, 0) };
    if v < 0 {
        None
    } else {
        Some(v)
    }
}

/// `ImmGetCompositionStringW` で生バイト列（GCS_COMPATTR 等）を読み取る。
unsafe fn read_imm_bytes(
    himc: windows::Win32::UI::Input::Ime::HIMC,
    index: u32,
) -> Option<Vec<u8>> {
    // SAFETY: lpBuf=None / dwBufLen=0 でバイト長取得。
    let byte_len =
        unsafe { ImmGetCompositionStringW(himc, IME_COMPOSITION_STRING(index), None, 0) };
    if byte_len < 0 {
        return None;
    }
    let byte_len = usize::try_from(byte_len).unwrap_or(0);
    if byte_len == 0 {
        return Some(Vec::new());
    }
    let mut buf = vec![0u8; byte_len];
    // SAFETY: buf は十分なサイズ。
    let written = unsafe {
        ImmGetCompositionStringW(
            himc,
            IME_COMPOSITION_STRING(index),
            Some(buf.as_mut_ptr().cast()),
            u32::try_from(buf.len()).unwrap_or(0),
        )
    };
    if written <= 0 {
        return None;
    }
    buf.truncate(usize::try_from(written).unwrap_or(0));
    Some(buf)
}

/// 部分リテラル検出の実験用 composition スナップショット。
#[derive(Debug, Default, Clone)]
pub struct CompositionSnapshot {
    /// HIMC が NULL だった（TSF native / Imm32Unavailable window の典型ケース）
    pub himc_null: bool,
    /// GCS_COMPSTR — 現在 composition 中の文字列
    pub comp_str: Option<String>,
    /// GCS_RESULTSTR — 確定済み文字列
    pub result_str: Option<String>,
    /// GCS_COMPREADSTR — composition の読み（ローマ字 / かな）
    pub comp_read_str: Option<String>,
    /// GCS_RESULTREADSTR — 確定済み文字列の読み
    pub result_read_str: Option<String>,
    /// GCS_CURSORPOS — カーソル位置
    pub cursor_pos: Option<i32>,
    /// GCS_COMPATTR — 各文字の属性バイト配列（0=Input/1=TargetConverted/2=Converted/3=Fixed/4=TargetNotConverted）
    pub comp_attr_bytes: Option<Vec<u8>>,
    /// ImmGetOpenStatus — IME 開閉状態
    pub open_status: Option<bool>,
    /// ImmGetConversionStatus の conversion mode（NATIVE / KATAKANA / FULLSHAPE / ROMAN 等のビットマスク）
    pub conversion_mode: Option<u32>,
    /// ImmGetConversionStatus の sentence mode（自動変換等のフラグ）
    pub sentence_mode: Option<u32>,
}
