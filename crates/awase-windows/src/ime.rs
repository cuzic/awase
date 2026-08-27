#![allow(unsafe_code)] // Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetConversionStatus, ImmGetOpenStatus, IME_COMPOSITION_STRING,
    IME_CONVERSION_MODE, IME_SENTENCE_MODE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, INPUT};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use crate::focus::class_names::is_tsf_native_window;
use crate::imm::{
    IMC_GETCONVERSIONMODE, IMC_GETOPENSTATUS, IMC_SETCONVERSIONMODE, IMC_SETOPENSTATUS,
    IME_CMODE_FULLSHAPE, IME_CMODE_KATAKANA, IME_CMODE_NATIVE, IME_CMODE_ROMAN,
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
/// 物理 IME キー（Ctrl+無変換 等）のように「今まさにフォーカスされているウィンドウ」を
/// 対象にしたい呼び出し元向け。トレイメニュー等、対象ウィンドウを別途確定済みの
/// 呼び出し元は [`set_ime_open_for_target`] を使うこと。
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
    unsafe { set_ime_open_for_target(hwnd, open) }
}

/// [`set_ime_open_cross_process`] のターゲット指定版。
///
/// 呼び出し時点の `GetGUIThreadInfo` を問い合わせず、`hwnd` に対して直接
/// `ImmGetDefaultIMEWnd` + `WM_IME_CONTROL / IMC_SETOPENSTATUS` を発行する。
///
/// トレイメニューのコマンドは、メニューを表示するために awase 自身のトレイ
/// ウィンドウへ `SetForegroundWindow` している（`tray::handle_tray_message`）ため、
/// コマンド実行時点で `set_ime_open_cross_process` の live query を使うと
/// トレイ自身（またはメニューの一時ウィンドウ）を対象にしてしまい、ユーザーが
/// 実際に入力していたアプリには何も効かない（2026-07-24 実機で「IME ON・Engine
/// OFF から何をしても復旧できない」として観測）。メニュー表示前に捕捉した
/// フォーカスウィンドウ（`tray::menu_target_hwnd()`）をここに渡すことで、
/// 正しい対象に対して操作できる。
///
/// # Safety
/// Calls Win32 APIs. Must be called from the main thread.
#[must_use]
pub unsafe fn set_ime_open_for_target(hwnd: HWND, open: bool) -> bool {
    // SAFETY: hwnd は呼び出し元が特定した有効なウィンドウハンドル。
    //         get_ime_wnd は内部で ImmGetDefaultIMEWnd を呼ぶ安全なラッパーであり、NULL を返す場合は
    //         直後の `?` でショートサーキットするため問題ない。
    let Some(ime_wnd) = (unsafe { crate::imm::get_ime_wnd(hwnd) }) else {
        log::debug!("set_ime_open_for_target: hwnd={hwnd:?} open={open} → no IME wnd, abort");
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
    // 関係を切り分けるため、send_ime_control の所要時間と現時点で observer 側が把握している
    // candidate (composition 可視) を一緒に出す。
    let candidate_visible = crate::tsf::observer::gji_candidate_visible_now();
    log::debug!(
        "set_ime_open_for_target: hwnd={hwnd:?} ime_wnd={ime_wnd:?} open={open} success={success} \
         send_elapsed={}ms candidate_visible={candidate_visible}",
        send_elapsed.as_millis()
    );
    success
}

/// 修飾キー（Ctrl / Shift / Alt）の押下状態スナップショット。
///
/// `SendInput` で修飾なしキーを届ける際の解放・復元シーケンス構築に使う。
/// 3つの IME キー送信関数（VK_KANJI / VK_IME_ON / VK_IME_OFF）が同じパターンを共有する。
#[derive(Clone, Copy)]
struct HeldModifiers {
    ctrl: bool,
    shift: bool,
    alt: bool,
}

impl HeldModifiers {
    /// 物理キー状態 (`PHYSICAL_KEY_STATE`) で修飾キーの押下状態を読み取る。
    ///
    /// `GetAsyncKeyState` は直前に注入した synthetic KeyUp の影響を受けて汚染される場合があるため、
    /// SendInput 非影響の物理キー状態で読み取ることで CTRL MISMATCH を防ぐ。
    fn read() -> Self {
        use crate::vk::{VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_RCONTROL, VK_RMENU, VK_RSHIFT};
        Self {
            ctrl: crate::hook::is_physical_key_down(VK_LCONTROL)
                || crate::hook::is_physical_key_down(VK_RCONTROL),
            shift: crate::hook::is_physical_key_down(VK_LSHIFT)
                || crate::hook::is_physical_key_down(VK_RSHIFT),
            alt: crate::hook::is_physical_key_down(VK_LMENU)
                || crate::hook::is_physical_key_down(VK_RMENU),
        }
    }

    /// 押下中の修飾キーを解放する `INPUT` イベントを追加する。
    fn push_release(self, inputs: &mut Vec<INPUT>) {
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
    unsafe fn push_restore(self, inputs: &mut Vec<INPUT>) -> Self {
        use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};
        use crate::vk::{
            VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_MENU, VK_RMENU, VK_RSHIFT, VK_SHIFT,
        };
        // GetAsyncKeyState は直前に注入した synthetic Ctrl↑ の影響を受けるため、
        // SendInput 非影響の物理キー状態 (PHYSICAL_KEY_STATE) で判定する。
        // これにより Ctrl+W 等のショートカットを押したまま IME キーが注入された場合でも
        // Ctrl が正しく復元され、Chrome へ Ctrl+W が届く。
        let still = Self {
            ctrl: self.ctrl
                && (crate::hook::is_physical_key_down(VK_LCONTROL)
                    || crate::hook::is_physical_key_down(crate::vk::VK_RCONTROL)),
            shift: self.shift
                && (crate::hook::is_physical_key_down(VK_LSHIFT)
                    || crate::hook::is_physical_key_down(VK_RSHIFT)),
            alt: self.alt
                && (crate::hook::is_physical_key_down(VK_LMENU)
                    || crate::hook::is_physical_key_down(VK_RMENU)),
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
/// 副作用があったため廃止。GJI 環境では GjiDirectStrategy (VK_IME_OFF) が先行するため、
/// この関数に到達するのは GJI 以外か GJI fallback 時のみ。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
// 変数名が意図的に似ているため similar_names を抑制する（gas_lctrl/gks_lctrl 等）。
#[expect(clippy::similar_names)]
pub unsafe fn post_kanji_toggle_to_focused() {
    use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};
    use crate::vk::{
        VK_CONTROL, VK_KANJI, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_RCONTROL, VK_RMENU, VK_RSHIFT,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState};

    let held = HeldModifiers::read();

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
    let candidate_pre = crate::tsf::observer::gji_candidate_visible_now();
    let t_send = std::time::Instant::now();
    let sent = crate::win32::send_input_safe(&inputs);
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

/// 冪等 IME ON: VK_IME_ON (0x16) を送信して DirectInput → IME ON に切り替える。
///
/// GJI・MS-IME ともにネイティブに処理する Windows 標準キー。
/// 既に ON の場合は no-op（冪等）。VK_KANJI トグルと異なり shadow desync の影響を受けない。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn post_ime_on_direct() {
    // SAFETY: send_ime_mode_key は Win32 API を呼び出す unsafe fn。
    let _ = unsafe { send_ime_mode_key(crate::vk::VK_IME_ON) };
}

/// 冪等 IME OFF: VK_IME_OFF (0x1A) を送信して DirectInput（直接入力）へ移行する。
///
/// GJI・MS-IME ともにネイティブに処理する Windows 標準キー。
/// 既に DirectInput の場合は no-op（冪等）。VK_KANJI トグルと異なり shadow desync の影響を受けない。
/// Chrome/Edge など Imm32Unavailable アプリは VK_IME_OFF を無視するため KanjiToggleStrategy が担当する。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn post_ime_off_direct() {
    // SAFETY: send_ime_mode_key は Win32 API を呼び出す unsafe fn。
    let _ = unsafe { send_ime_mode_key(crate::vk::VK_IME_OFF) };
}

/// GJI 専用 IME ON（後方互換エイリアス）。新規コードは `post_ime_on_direct` を使うこと。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn post_gji_ime_on() {
    unsafe { post_ime_on_direct() }
}

/// GJI 専用 IME OFF（後方互換エイリアス）。新規コードは `post_ime_off_direct` を使うこと。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn post_gji_ime_off() {
    unsafe { post_ime_off_direct() }
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
/// 戻り値: 実際に注入した場合 `true`。Win キー押下中でスキップした場合 `false`。
/// **呼び出し元は `false` を「apply していない」として扱うこと** — スキップを
/// Applied 扱いで applied_snapshot にラッチすると、以降の force-ON/再試行が全て
/// 「適用済み」no-op になり belief ON × 実 IME OFF が固定される（2026-07-07 実機:
/// ロック解除 → Win+Ctrl+→ デスクトップ切替中の IME ON apply がここでスキップ
/// されたのに Applied 記録され、Terminal で「これで」→「korede」化。BUG-16 追補）。
///
/// **ADR-086 INV-13 の例外（§4 INV-13/§5 Phase 3 item 3）**: 本関数は
/// `SendInput` を使うため宛先 hwnd を指定できず（配送時点のフォーカス先へ届く）、
/// `ActuationTarget` によるターゲット同一性検証（INV-14）を構造的に適用できない。
/// force-ON（`Runtime::consume_force_open_pending`）はこの制約の代わりに
/// `Output::ime_mode_focus_gen` の照合（時間軸フェンス）のみで守る。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
#[must_use]
pub unsafe fn send_ime_mode_key(vk: awase::types::VkCode) -> bool {
    use crate::tsf::output::{make_key_input_ex, IME_KANJI_MARKER};

    // Win キー押下中は注入をスキップする。
    // Win+VK_IME_ON/OFF は OS に未認識ショートカットとして届き、Win↑ のタイミングで
    // スタートメニューを誤起動させる原因になる。
    // Win を SendInput で解放すると Win 自体がスタートメニューを開くため、
    // Alt と同様にスキップ（解放しない）が正しい対処。
    if crate::hook::win_key_held() {
        log::debug!(
            "[ime-mode] skipped vk=0x{vk:02X} (Win key held — Win+VK_IME triggers Start Menu on Win↑)"
        );
        return false;
    }

    let held = HeldModifiers::read();
    // VK_IME_ON/OFF は Windows 標準の冪等 IME キー（GJI / MS-IME がネイティブに処理）。
    // ALT を解放すると ALT+TAB スイッチャーが確定してしまうため、ALT は解放しない。
    let held_skip_alt = HeldModifiers { alt: false, ..held };
    let mut inputs: Vec<INPUT> = Vec::with_capacity(6);
    held_skip_alt.push_release(&mut inputs);
    inputs.push(make_key_input_ex(vk, false, IME_KANJI_MARKER));
    inputs.push(make_key_input_ex(vk, true, IME_KANJI_MARKER));
    // SAFETY: push_restore は Win32 SendInput を呼ぶ。
    let still = unsafe { held_skip_alt.push_restore(&mut inputs) };

    log::debug!(
        "[ime-mode] SendInput vk=0x{vk:02X} \
         release(ctrl={} shift={} alt=false(skipped)) \
         restore(ctrl={} shift={} alt=false(skipped)) phys_alt={} total={} events",
        held.ctrl,
        held.shift,
        still.ctrl,
        still.shift,
        held.alt,
        inputs.len()
    );
    let sent = crate::win32::send_input_safe(&inputs);
    if sent as usize != inputs.len() {
        log::warn!(
            "[ime-mode] SendInput(vk=0x{vk:02X}) sent {sent}/{} events",
            inputs.len()
        );
    }
    true
}

/// IME モード切り替えキーを、必要なら synthetic Shift↑ を前置して送信する。
///
/// BUG-25 GJI 半角英数 entry/exit 用。`prepend_synthetic_shift_up` が真のときは
/// OS/IME 視点に残っている Shift 押下を同一 `SendInput` バッチ内で先に解除する。
/// `VK_DBE_ALPHANUMERIC` は CapsLock scan 衝突を避けるため scan=0 で送り、
/// `VK_DBE_HIRAGANA` は既存の MS-IME 復元経路と同じ scan 付きで送る。
///
/// 戻り値: 実際に注入した場合 `true`。Win キー押下中でスキップした場合 `false`。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
#[must_use]
pub unsafe fn send_ime_mode_key_with_shift_release_prefix(
    vk: awase::types::VkCode,
    prepend_synthetic_shift_up: bool,
) -> bool {
    use crate::tsf::output::{make_key_input_ex, make_scan_key_input, IME_KANJI_MARKER};
    use crate::vk::{VK_DBE_HIRAGANA, VK_LSHIFT, VK_RSHIFT};

    if crate::hook::win_key_held() {
        log::debug!(
            "[ime-mode] skipped vk=0x{vk:02X} (Win key held — Win+VK_IME triggers Start Menu on Win↑)"
        );
        return false;
    }

    let held = HeldModifiers::read();
    let held_skip_alt = HeldModifiers { alt: false, ..held };
    let mut inputs: Vec<INPUT> = Vec::with_capacity(8);
    if prepend_synthetic_shift_up {
        // 呼び出し元は「どちら側の物理 Shift が押されたか」を運ばない
        // （左Shift単独タップ＝entry、左/右いずれかの緊急解除＝exit、いずれも
        // ここには vk_code が届かない）。汎用 VK_SHIFT(0x10) 単体で release を
        // 送ると `MapVirtualKeyW` は左Shift の scan(0x2A) しか返さず、Windows の
        // 内部キー状態は VK_LSHIFT のみ更新され VK_RSHIFT 側は更新されない
        // （awase 自身のフックが実キー up を CallNextHookEx 前に消費/遅延させて
        // いるため、OS 側の状態は synthetic 側でしか更新できない）。右Shift
        // 起点の緊急解除で OS/GJI から見た Shift が押下中のまま残ると、決定0
        // M4 が実機で確定させた「Shift 押下中は DBE キーの KeyDown 自体が
        // フックに配送されない」条件を踏み、脱出経路そのものが不発になる。
        // 左右両方の synthetic Shift up を送る（KeyUp の重複は無害、既存の
        // 復元経路と同じ根拠）。
        inputs.push(make_scan_key_input(VK_LSHIFT, true, IME_KANJI_MARKER));
        inputs.push(make_scan_key_input(VK_RSHIFT, true, IME_KANJI_MARKER));
    }
    held_skip_alt.push_release(&mut inputs);
    if vk == VK_DBE_HIRAGANA {
        inputs.push(make_scan_key_input(vk, false, IME_KANJI_MARKER));
        inputs.push(make_scan_key_input(vk, true, IME_KANJI_MARKER));
    } else {
        inputs.push(make_key_input_ex(vk, false, IME_KANJI_MARKER));
        inputs.push(make_key_input_ex(vk, true, IME_KANJI_MARKER));
    }
    // SAFETY: push_restore は Win32 SendInput を呼ぶ。
    let still = unsafe { held_skip_alt.push_restore(&mut inputs) };

    log::debug!(
        "[ime-mode] SendInput vk=0x{vk:02X} prepend_shift_up={prepend_synthetic_shift_up} \
         release(ctrl={} shift={} alt=false(skipped)) \
         restore(ctrl={} shift={} alt=false(skipped)) phys_alt={} total={} events",
        held.ctrl,
        held.shift,
        still.ctrl,
        still.shift,
        held.alt,
        inputs.len()
    );
    let sent = crate::win32::send_input_safe(&inputs);
    if sent as usize != inputs.len() {
        log::warn!(
            "[ime-mode] SendInput(vk=0x{vk:02X}, prepend_shift_up={prepend_synthetic_shift_up}) \
             sent {sent}/{} events",
            inputs.len()
        );
    }
    true
}

/// 現在フォーカスされているウィンドウの IME 変換モード生値を返す（診断ログ専用）。
///
/// ビット定義: NATIVE=0x0001 KATAKANA=0x0002 FULLSHAPE=0x0008 ROMAN=0x0010
///
/// # Safety
/// Calls Win32 APIs.
#[must_use]
pub unsafe fn get_ime_conversion_mode_raw() -> Option<u32> {
    // SAFETY: get_focused_hwnd は unsafe fn で GetGUIThreadInfo().hwndFocus を優先し
    //         GetForegroundWindow にフォールバックする（BUG-55 参照）。NULL を返す可能性が
    //         あるが detect_ime_conversion_for_hwnd 内の non_null() チェックで処理される。
    detect_ime_conversion_for_hwnd(unsafe { get_focused_hwnd() })
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
    // SAFETY: get_focused_hwnd は unsafe fn。NULL を返す場合は non_null() が `?` で None を返す。
    //
    // BUG-55（2026-08-07 実機）: 以前は GetForegroundWindow()（トップレベル）を使っていたが、
    // Windows Terminal のような「トップレベルとは別の子ウィンドウ
    // (Windows.UI.Input.InputSite.WindowClass) が実際の TSF composition を持つ」アプリでは
    // ImmGetDefaultIMEWnd(トップレベル) がその composition とは無関係な（プロセス/スレッド
    // 単位のグローバルな）互換ウィンドウを返し、conv 読み取りが実態と乖離していた。
    // read_ime_state_full と同じ GetGUIThreadInfo().hwndFocus 基準（get_focused_hwnd）に
    // 揃えることで、実際にテキスト入力を受けている子ウィンドウを対象にする。
    let hwnd = unsafe { get_focused_hwnd() }.non_null()?;
    unsafe { get_ime_conversion_mode_for_hwnd(hwnd, timeout_ms) }
}

/// `get_ime_conversion_mode_raw_timeout` の実体（hwnd 指定版）。
///
/// [`get_ime_conv_for_target`] と共有する（ADR-086 §2.3 P7 と同じ「hwnd 解決を
/// 呼び出し元の責務として切り離す」パターン。`set_ime_romaji_mode_for_hwnd` 参照）。
///
/// # Safety
/// Calls Win32 APIs.
unsafe fn get_ime_conversion_mode_for_hwnd(hwnd: HWND, timeout_ms: u32) -> Option<u32> {
    // SAFETY: hwnd は呼び出し元が確定した有効なウィンドウハンドル。
    let ime_wnd = unsafe { crate::imm::get_ime_wnd(hwnd) };
    log::debug!("[idle-conv-check-diag] focused_hwnd={hwnd:?} ime_wnd={ime_wnd:?}");
    let ime_wnd = ime_wnd?;
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

/// conv mode の「取得 → 変換 → （差分があれば）反映」という手続きを共通化するヘルパー。
///
/// `ime_wnd` に対して `IMC_GETCONVERSIONMODE`（タイムアウト 50ms）で現在の変換モードを
/// 取得し、`f` に渡して新しい値を計算する。取得に失敗した場合（タイムアウト等）は `None`。
///
/// 新しい値が現在値と同じ場合は `IMC_SETCONVERSIONMODE` を呼ばずに
/// `Some((conv, conv, false, true))` を返す（`changed=false` で「既に目的の状態だった」ことを
/// 呼び出し元に伝える。ログを出すかどうかは呼び出し元ごとに異なるため、ここでは出さない）。
/// 新しい値が異なる場合のみ `IMC_SETCONVERSIONMODE`（タイムアウト 50ms）を発行し、
/// `Some((変更前conv, 変更後conv, true, 送信成功))` を返す。
///
/// # Safety
/// Win32 API を呼び出す。
unsafe fn modify_conv_mode(
    ime_wnd: HWND,
    f: impl FnOnce(u32) -> u32,
) -> Option<(u32, u32, bool, bool)> {
    // SAFETY: ime_wnd は呼び出し元が get_ime_wnd から取得した有効な IME ウィンドウハンドル。
    //         タイムアウト 50ms 内に制御が戻ることが保証される。
    let current = unsafe { crate::imm::send_ime_control(ime_wnd, IMC_GETCONVERSIONMODE, 0, 50) }?;
    let conv = current as u32;
    let new_conv = f(conv);
    if new_conv == conv {
        return Some((conv, new_conv, false, true));
    }
    // SAFETY: ime_wnd は呼び出し元が get_ime_wnd から取得した有効な IME ウィンドウハンドル。
    //         new_conv は取得した conv を f で変換した値であり有効な変換モード値。
    let success = unsafe {
        crate::imm::send_ime_control(ime_wnd, IMC_SETCONVERSIONMODE, new_conv as isize, 50)
    }
    .is_some();
    Some((conv, new_conv, true, success))
}

// 旧 `set_ime_romaji_mode()` / `set_ime_romaji_mode_async()`（宛先をライブクエリで
// 自己決定する同期 IMC write）は **ADR-089 §6 Phase C item 12（= ADR-086 INV-14 の
// 未移行分の是正）で削除した**。呼び出し元は `ime_controller.rs` の
// `ImmCrossProcessStrategy::apply` / `MsImeDirectStrategy::apply` の 2 箇所だけで、
// どちらも `ime_controller::apply_mechanism` の ROMAN 補完ステップ
// （`ActuationTarget::capture_blocking` → `set_ime_romaji_mode_for_target_blocking`）
// へ移行済み。
//
// `set_ime_romaji_mode_async` は移行前から呼び出し元ゼロ（`set_ime_romaji_mode` を
// offload するだけのラッパー）だったため、同時に削除した。
//
// **再実装しないこと。** ライブクエリ版を復活させると、ROMAN 補完の宛先と
// 実際に開く/キーを送る宛先が別ウィンドウになりうる（ADR-086 §1.2 欠陥1 と同型）。
// `tests/architecture_guard.rs::sync_romaji_write_goes_through_a_captured_target` が
// 書き込み口の数を固定する。

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

/// `win32_async::offload` にクロージャを渡し `.await` する、という 8 箇所の
/// `xxx_async` ラッパーで共通の定型文を集約するヘルパー。
///
/// `f` 自体の呼び出しは安全だが、各呼び出し元は `f` の中身を
/// `|| unsafe { xxx(..) }` の形にして Win32 API 呼び出しを包む
/// (`unsafe` は呼び出し対象の unsafe fn ごとに必要なため、ここでは
/// 二重に `unsafe` で包まない — 包むと `unused_unsafe` 警告になる)。
async fn offload_unsafe<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    win32_async::offload(f).await
}

/// `read_ime_state_full` の async 版（ワーカースレッドで実行）
pub async fn read_ime_state_full_async() -> ImeSnapshot {
    // SAFETY: read_ime_state_full は unsafe fn。win32_async::offload はワーカースレッドで実行するが
    //         IMM32 API はワーカースレッドからも呼び出し可能。
    offload_unsafe(|| unsafe { read_ime_state_full() }).await
}

/// `read_ime_state_fast` の async 版（ワーカースレッドで実行）
pub async fn read_ime_state_fast_async() -> FastImeProbeResult {
    // SAFETY: read_ime_state_fast は unsafe fn。win32_async::offload はワーカースレッドで実行するが
    //         IMM32 API はワーカースレッドからも呼び出し可能。
    offload_unsafe(|| unsafe { read_ime_state_fast() }).await
}

/// `set_ime_open_cross_process` の async 版（ワーカースレッドで実行）
pub async fn set_ime_open_cross_process_async(open: bool) -> bool {
    // SAFETY: set_ime_open_cross_process は unsafe fn。win32_async::offload はワーカースレッドで実行するが
    //         SendMessageTimeoutW はクロスプロセス呼び出しのためスレッドに依存しない。
    offload_unsafe(move || unsafe { set_ime_open_cross_process(open) }).await
}

/// 目標 conv 指定付き ROMAN 補完の実体（hwnd 指定版）。
///
/// ADR-086 §5 Phase1b step6（2026-08-08）: 宛先をライブクエリで自己決定する
/// `set_ime_romaji_mode_with_target`/`_async`（target identity を持たない、
/// ADR-086 §1.2 欠陥1）は、全 6 呼び出し経路が [`ActuationTarget`]/
/// [`set_ime_conv_for_target`] 経由へ移行完了したため削除した。この関数
/// （`ime_wnd` 解決以降のロジックのみ）は [`set_ime_conv_for_target`]/
/// [`set_ime_open_then_conv_for_target`] が hwnd 解決済みの状態で共有する
/// 実体として残る。
///
/// `target_conv` が `Some(v)` の場合は `v` をそのまま `ImmSetConversionStatus` に設定する。
/// `None` の場合は現在の conv に ROMAN ビットを追加する（`set_ime_romaji_mode` 相当）。
///
/// # Safety
/// Calls Win32 APIs. Must be called from the main thread or worker thread via offload.
unsafe fn set_ime_romaji_mode_for_hwnd(hwnd: HWND, target_conv: Option<u32>) -> bool {
    use crate::imm::IME_CMODE_ROMAN;
    let Some(ime_wnd) = (unsafe { crate::imm::get_ime_wnd(hwnd) }) else {
        return false;
    };
    let Some((conv, new_conv, changed, success)) = (unsafe {
        modify_conv_mode(ime_wnd, |conv| {
            target_conv.unwrap_or(conv | IME_CMODE_ROMAN)
        })
    }) else {
        return false;
    };
    if changed {
        log::debug!(
            "[imm-romaji] conv 0x{conv:08X} → 0x{new_conv:08X} success={success} target={:?}",
            target_conv.map(|v| format!("0x{v:08X}")),
        );
    }
    success
}

/// クロスプロセスで IME の変換モードをひらがなに強制する。
///
/// `IMC_GETCONVERSIONMODE` で現在の変換モードを取得し、
/// `IME_CMODE_KATAKANA` ビットを落として `IME_CMODE_NATIVE | IME_CMODE_FULLSHAPE` を立てる。
/// 半角カタカナ・全角カタカナ状態でパニックリセットしたときでもひらがなに戻る。
///
/// Returns `true` if the operation succeeded or mode was already hiragana.
///
/// # Safety
/// Calls Win32 APIs.
#[must_use]
pub unsafe fn set_ime_hiragana_mode_cross_process() -> bool {
    let gui_result =
        crate::win32::get_gui_thread_info_with_timeout(std::time::Duration::from_millis(150));
    let Some(hwnd) = gui_result.focused_hwnd else {
        log::debug!("set_ime_hiragana_mode_cross_process: no focused hwnd, abort");
        return false;
    };
    let Some(ime_wnd) = (unsafe { crate::imm::get_ime_wnd(hwnd) }) else {
        log::debug!("set_ime_hiragana_mode_cross_process: hwnd={hwnd:?} no IME wnd, abort");
        return false;
    };
    // ローマ字ひらがなモード = NATIVE + FULLSHAPE + ROMAN、KATAKANA ビットなし。
    // かな入力の半角カタカナ (NATIVE|KATAKANA、ROMAN/FULLSHAPEなし) でリセットした場合も
    // ROMAN を補完してローマ字ひらがなに戻す。awase はローマ字 VK を送るため ROMAN が必要。
    let Some((conv, new_conv, changed, success)) = (unsafe {
        modify_conv_mode(ime_wnd, |conv| {
            (conv | IME_CMODE_NATIVE | IME_CMODE_FULLSHAPE | IME_CMODE_ROMAN) & !IME_CMODE_KATAKANA
        })
    }) else {
        log::debug!("set_ime_hiragana_mode_cross_process: IMC_GETCONVERSIONMODE timeout");
        return false;
    };
    if changed {
        log::debug!(
            "set_ime_hiragana_mode_cross_process: hwnd={hwnd:?} \
             conv 0x{conv:08X} → 0x{new_conv:08X} success={success}"
        );
    }
    success
}

/// `set_ime_hiragana_mode_cross_process` の async 版（ワーカースレッドで実行）
pub async fn set_ime_hiragana_mode_cross_process_async() -> bool {
    // SAFETY: set_ime_hiragana_mode_cross_process は unsafe fn。
    //         SendMessageTimeoutW はクロスプロセス呼び出しのためスレッドに依存しない。
    offload_unsafe(|| unsafe { set_ime_hiragana_mode_cross_process() }).await
}

/// `get_ime_conversion_mode_raw_timeout` の async 版（ワーカースレッドで実行）
///
/// `with_app` 再入を避けるためにワーカースレッドへオフロードする。加えて、
/// `SendMessageTimeoutW(SMTO_ABORTIFHUNG)` は送信先スレッドが既にハング判定済みの
/// 場合のみ `timeout_ms` で打ち切られ、そうでなければ Windows の `HungAppTimeout`
/// （既定 ~5000ms）までブロックしうる（`timeout_ms` は保証されない、既知の Win32 の
/// 誤解）。エンジンスレッド（メッセージループ）上で同期呼び出しすると、その間キー入力が
/// フックに消費されたまま処理されず「文字が消えて数秒後にバーストで出る」症状になる
/// （`docs/known-bugs.md` 参照）。呼び出し元は必ずこの async 版を使うこと。
pub async fn get_ime_conversion_mode_raw_timeout_async(timeout_ms: u32) -> Option<u32> {
    // SAFETY: get_ime_conversion_mode_raw_timeout は unsafe fn。win32_async::offload はワーカースレッドで実行するが
    //         SendMessageTimeoutW はクロスプロセス呼び出しのためスレッドに依存しない。
    offload_unsafe(move || unsafe { get_ime_conversion_mode_raw_timeout(timeout_ms) }).await
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

/// `get_focused_hwnd` の async 版（ワーカースレッドで実行）。
///
/// [`ActuationTarget`] の捕獲・再検証の両方がこれを使う（ADR-086 §7-3）。
/// `get_focused_hwnd` は内部で `get_gui_thread_info_with_timeout`（さらに別の
/// ワーカースレッドへ offload してタイムアウト付きで待つラッパー）を呼ぶため、
/// フック駆動の同期処理経路から直接同期呼びすると BUG-34（フックスレッドを
/// ブロックできない制約）を再現する。同期版を公開しないのはそのため。
pub async fn get_focused_hwnd_async() -> HWND {
    // HWND（`*mut c_void` を包むだけ）は Send ではないため offload_unsafe の
    // `T: Send` 境界を満たせない。win32.rs::get_gui_thread_info_with_timeout の
    // `SendableResult` と同じ手法でラップする（Win32 ウィンドウハンドルは
    // プロセス内で有効なグローバルリソースであり、スレッド間で安全に送信できる）。
    struct SendableHwnd(HWND);
    // SAFETY: HWND はプロセス内で有効なグローバルリソースへのハンドル（ポインタ値）
    //         であり、スレッド間で共有しても安全。
    unsafe impl Send for SendableHwnd {}

    let SendableHwnd(hwnd) = offload_unsafe(|| SendableHwnd(unsafe { get_focused_hwnd() })).await;
    hwnd
}

/// ある actuation（conv-mode 書き込み等の外部 IME 状態変更）が「どのウィンドウへ
/// 向けたものか」を運ぶ値（ADR-086 §2.3 P7 / §4 INV-14）。
///
/// 起案時点で [`ActuationTarget::capture`] により hwnd を確定し、実際の Win32
/// 書き込みの直前に [`ActuationTarget::verify_still_current`] で再検証する。
/// フィールドは private —— `verify_still_current` を経由せずに hwnd を取り出す
/// ことはできない（ADR-086 §6 段1、`ForceGuardSet.guards` を private 化した
/// のと同じ手法）。
///
/// フォーカス世代番号（`focus_gen`）は `Output::ime_mode_focus_gen` 相当の
/// カウンタを想定しているが、`ime.rs` は Runtime/Output の内部状態に依存しない
/// レイヤーであるため、gen 値そのものはこの型・この関数群では読まない。
/// 呼び出し元（`runtime::conv_actuation` 等）が `capture`/`verify_still_current`
/// の引数として供給する。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ActuationTarget {
    hwnd: HWND,
    focus_gen: u32,
}

impl ActuationTarget {
    /// 起案時点の hwnd を捕獲する。hwnd が取得できない場合（フォーカス無し等）は
    /// `None`。
    pub(crate) async fn capture(focus_gen: u32) -> Option<Self> {
        let hwnd = get_focused_hwnd_async().await;
        hwnd.non_null().map(|hwnd| Self { hwnd, focus_gen })
    }

    /// [`Self::capture`] の同期版（ADR-089 §6 Phase C item 12）。
    ///
    /// **`ImeOpenStrategy::apply` の呼び出しチェーンは完全に同期的**であり
    /// （`apply_force_on_for_imm_broken` / `consume_force_open_pending` /
    /// `ir_apply_drift_correction` / `kp_stage_shadow_ime_toggle` 等、
    /// `spawn_local` を使わない経路から直接呼ばれる）、async 版の `capture` を
    /// そのまま使うことはできない。ADR-086 Phase 3 はこの制約を理由に
    /// 「`ImeOpenStrategy::apply` 自体の非同期化が要る」として ROMAN 補完の
    /// `ActuationTarget` 化を見送っていたが、**`capture` が実際にやっている
    /// ことは `get_focused_hwnd()` 1 回であり**、旧
    /// `set_ime_romaji_mode()` が内部で行っていたライブクエリと同一である。
    /// したがって「捕獲を write の外へ出す」だけなら同期のままでできる。
    ///
    /// # 実測上の注意
    /// `get_focused_hwnd` は内部で `get_gui_thread_info_with_timeout(30ms)` を
    /// 呼び、失敗時は `GetForegroundWindow()` にフォールバックする。
    /// **旧 `set_ime_romaji_mode()` と同じ関数・同じタイムアウト・同じ
    /// フォールバック**であり、Win32 往復の回数も 1 回のままである
    /// （ADR-089 §6「Phase C 実施記録」の C-7）。
    ///
    /// # Safety
    /// Win32 API を呼び出す。メインスレッドから呼ぶこと。
    pub(crate) unsafe fn capture_blocking(focus_gen: u32) -> Option<Self> {
        // SAFETY: 呼び出し元がメインスレッドであることを保証する。
        let hwnd = unsafe { get_focused_hwnd() };
        hwnd.non_null().map(|hwnd| Self { hwnd, focus_gen })
    }

    /// 同期経路用の再検証。**hwnd のライブクエリを行わず、フォーカス世代だけを
    /// 照合する**（ADR-089 §6 Phase C item 12）。
    ///
    /// [`Self::verify_still_current`] は `get_focused_hwnd_async().await` で
    /// hwnd を読み直すが、同期経路では
    ///
    /// 1. 捕獲から書き込みまでの間に await 点が無く、hwnd が動く余地が構造的に
    ///    無い（同一フレーム内の連続した文である）。
    /// 2. 読み直すと Win32 往復が 1 回から 2 回へ倍増し、打鍵ホットパスに乗る
    ///    force-ON 経路のレイテンシが変わる（`.claude/rules/tuning-constants.md`
    ///    が実測を要求する軸）。
    ///
    /// ため、世代照合のみを行う。**この照合は現状では常に一致する**——
    /// `consume_force_open_pending` が同じ理由で置いている gen フェンス
    /// （「本経路は完全に同期的なため実際には常に一致するが、将来この経路に
    /// 非同期処理が挟まれた場合の回帰を防ぐため明示的に確認する」）と同じ
    /// 性質のガードである。
    const fn verify_gen_only(self, current_focus_gen: u32) -> TargetVerifyOutcome {
        if self.focus_gen == current_focus_gen {
            TargetVerifyOutcome::Current(self.hwnd)
        } else {
            TargetVerifyOutcome::GenStale
        }
    }

    /// 実行直前に呼ぶ。`read_current_focus_gen` は呼び出し元が最新の gen 値
    /// （典型的には `Cell<u64>::get`）を読むためのクロージャで、**hwnd の
    /// ライブクエリが完了した直後**（このメソッド内の唯一の await の後）に
    /// 呼ばれる。gen を先に固定してから ~30ms かかりうる hwnd クエリを待つ
    /// 順序だと、そのクエリ中に新たな FocusChange が起きても検知できない
    /// 窓が残るため（opus レビュー指摘、2026-08-08）、gen は「実際に hwnd を
    /// 読み終えた瞬間」にできるだけ近いタイミングで読む。
    ///
    /// [`TargetVerifyOutcome`] 参照。呼び出し元は `Current` 以外なら書き込まずに
    /// `Aborted` として扱うこと（INV-14）。
    // `self`（HWND を含み Send でない）を await をまたいで保持するため、この
    // 関数の Future は Send にならない。呼び出し元は win32_async::spawn_local
    // （ローカル・シングルスレッド実行、offload_unsafe 自体と同じ前提）経由でのみ
    // 実行されるため実害はない。HWND（`*mut c_void` を包むだけ）が Send でない
    // ことは Win32 API の性質であり、get_focused_hwnd_async 等の既存 async 関数
    // 群と同じ制約。
    #[allow(clippy::future_not_send)]
    pub(crate) async fn verify_still_current(
        self,
        read_current_focus_gen: impl FnOnce() -> u32,
    ) -> TargetVerifyOutcome {
        let current_hwnd = get_focused_hwnd_async().await;
        if self.focus_gen != read_current_focus_gen() {
            return TargetVerifyOutcome::GenStale;
        }
        Self::compare(self.hwnd, current_hwnd)
    }

    /// 純粋比較ロジック（Win32 呼び出しを含まないため単体テストで固定できる、
    /// ADR-086 §6 段4 の「トリガー条件と消費回数は純粋ロジックとして必ず
    /// テストで固定する」と同じ趣旨）。gen の比較は `verify_still_current` 側で
    /// 行うため、ここでは hwnd の一致のみを見る。
    fn compare(captured: HWND, current: HWND) -> TargetVerifyOutcome {
        if captured == current {
            TargetVerifyOutcome::Current(current)
        } else {
            TargetVerifyOutcome::TargetMoved
        }
    }
}

/// [`ActuationTarget::verify_still_current`] の結果。
///
/// `GenStale`/`TargetMoved` を区別して返すのは、`runtime::conv_actuation`
/// 側で `ActuationOutcome::Aborted { reason }`（タスク #12）へそのまま
/// マッピングできるようにするため。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetVerifyOutcome {
    /// 起案時点と一致。書き込んでよい。
    Current(HWND),
    /// フォーカス世代が起案時点から進んでいる（`ime_mode_focus_gen` 相当）。
    GenStale,
    /// フォーカス世代は同じだが、実際の hwnd が起案時点と異なる。
    TargetMoved,
}

/// `ActuationTarget` が捕獲した hwnd に対して conv-mode を読み取る（read-modify-write
/// の read 側、ADR-086 §2.3 P7）。
///
/// [`set_ime_conv_for_target`] と異なり `verify_still_current` を**行わない**。
/// 読み取りは非破壊であり、検証を write の1箇所に集約したほうが Win32 往復も
/// 競合窓も増えないため（opus 設計相談 2026-08-08）。呼び出し元は
/// `ActuationTarget::capture` で得た同じ `target` を、この後の
/// `set_ime_conv_for_target` にもそのまま渡すこと（read/write で同じ hwnd を
/// 使い回すことが目的）。
// `target`（HWND を含み Send でない）を引数に取るため Future が Send にならない。
// verify_still_current/set_ime_conv_for_target と同じ理由で実害なし
// （win32_async::spawn_local 経由でのみ呼ばれる前提）。
#[allow(clippy::future_not_send)]
pub(crate) async fn get_ime_conv_for_target(
    target: ActuationTarget,
    timeout_ms: u32,
) -> Option<u32> {
    // HWND は Send でないため get_focused_hwnd_async と同じ SendableHwnd イディオムで包む。
    struct SendableHwnd(HWND);
    // SAFETY: HWND はプロセス内で有効なグローバルリソースへのハンドルであり、
    //         スレッド間で共有しても安全。
    unsafe impl Send for SendableHwnd {}
    let target_hwnd = SendableHwnd(target.hwnd);
    offload_unsafe(move || {
        let target_hwnd = target_hwnd;
        let SendableHwnd(hwnd) = target_hwnd;
        unsafe { get_ime_conversion_mode_for_hwnd(hwnd, timeout_ms) }
    })
    .await
}

/// [`set_ime_conv_for_target`] の結果（ADR-086 §2.3 P7）。`#[must_use]` は
/// `Aborted` の握り潰し防止（ADR-086 §6 段1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub(crate) enum ActuationOutcome {
    /// 書き込み成功。
    Written,
    /// ターゲット不一致のため書き込まなかった（INV-14）。`applied` 系キャッシュを
    /// 更新してはならない — 「成功」として記録すると乖離を検出できなくなる。
    Aborted(AbortReason),
    /// ターゲットは一致したが Win32 呼び出し自体が失敗した
    /// （IME ウィンドウが取得できない等）。
    Failed,
}

/// [`ActuationOutcome::Aborted`] の理由。[`TargetVerifyOutcome`] の
/// `GenStale`/`TargetMoved` をそのまま引き継ぐ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbortReason {
    /// フォーカス世代が起案時点から進んでいる。
    GenStale,
    /// フォーカス世代は同じだが、実際の hwnd が起案時点と異なる。
    TargetMoved,
}

/// 捕獲済みターゲットへ ROMAN ビットを補完する**同期**版
/// （ADR-089 §6 Phase C item 12 = ADR-086 INV-14 の未移行分の是正）。
///
/// 旧 `set_ime_romaji_mode()`（宛先をライブクエリで自己決定していた）の置き換え。
/// 呼び出し元は `ime_controller::apply_mechanism` の ROMAN 補完ステップ 1 箇所のみ
/// （`tests/architecture_guard.rs::sync_romaji_write_goes_through_a_captured_target`
/// が固定）。
///
/// [`set_ime_conv_for_target`]（非同期版）との差:
///
/// | | 非同期版 | 本関数（同期版） |
/// |---|---|---|
/// | 再検証 | `verify_still_current`（hwnd ライブクエリ + gen 照合） | [`ActuationTarget::verify_gen_only`]（gen 照合のみ） |
/// | Win32 往復 | 捕獲 1 + 再検証 1 + write | 捕獲 1 + write |
/// | 呼び出し文脈 | `spawn_local` の中（await 点あり） | 完全同期（await 点なし） |
///
/// hwnd を読み直さない理由は [`ActuationTarget::verify_gen_only`] の doc を参照。
/// **`Aborted` を成功として扱わないこと**（INV-14）——呼び出し元は結果を必ず
/// ログに残す。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub(crate) unsafe fn set_ime_romaji_mode_for_target_blocking(
    target: ActuationTarget,
    current_focus_gen: u32,
) -> ActuationOutcome {
    let hwnd = match target.verify_gen_only(current_focus_gen) {
        TargetVerifyOutcome::Current(hwnd) => hwnd,
        TargetVerifyOutcome::GenStale => {
            log::debug!(
                "[imm-romaji] Aborted(GenStale): captured_gen={} current_gen={current_focus_gen}",
                target.focus_gen
            );
            return ActuationOutcome::Aborted(AbortReason::GenStale);
        }
        TargetVerifyOutcome::TargetMoved => {
            // `verify_gen_only` は hwnd を読み直さないため構造的に返らないが、
            // `TargetVerifyOutcome` の網羅性のために残す。
            log::debug!("[imm-romaji] Aborted(TargetMoved)");
            return ActuationOutcome::Aborted(AbortReason::TargetMoved);
        }
    };
    // SAFETY: hwnd は capture_blocking が non_null() で検証済みのウィンドウハンドル。
    //         set_ime_romaji_mode_for_hwnd は SendMessageTimeoutW ベースでタイムアウト付き。
    if unsafe { set_ime_romaji_mode_for_hwnd(hwnd, None) } {
        ActuationOutcome::Written
    } else {
        ActuationOutcome::Failed
    }
}

/// [`ActuationTarget`] を通じて conv-mode を書き込む唯一の target-aware 版
/// （ADR-086 §2.3 P7 / §4 INV-14）。
///
/// 実行直前に `target.verify_still_current` で再検証し、起案時点と一致する
/// 場合のみ実際に `IMC_SETCONVERSIONMODE` を発行する。不一致なら書き込まず
/// `Aborted` を返す（INV-14: `Aborted` を成功として記録してはならない）。
/// `Written`/`Aborted`/`Failed` いずれの結果も debug ログに残す（INV-14 が
/// 要求する「`Aborted` は必ずログに残す」を満たす。実機での競合頻度を
/// 事後に測れるようにするのが目的、opus レビュー指摘 2026-08-08）。
///
/// `read_current_focus_gen` は呼び出し元が最新の gen 値を読むためのクロージャ
/// （`ime.rs` は Runtime/Output の内部状態に依存しないため、gen 値そのものは
/// ここでは読まない。`ActuationTarget::capture`/`verify_still_current` と
/// 同じ理由）。`verify_still_current` に転送し、hwnd のライブクエリ完了直後の
/// 最新値として使われる。
// `target`（HWND を含み Send でない）を await をまたいで保持するため Future が
// Send にならない。verify_still_current と同じ理由で実害なし。
#[allow(clippy::future_not_send)]
pub(crate) async fn set_ime_conv_for_target(
    target: ActuationTarget,
    conv: Option<u32>,
    read_current_focus_gen: impl FnOnce() -> u32,
) -> ActuationOutcome {
    match target.verify_still_current(read_current_focus_gen).await {
        TargetVerifyOutcome::GenStale => {
            log::debug!(
                "[conv-actuate] Aborted(GenStale): target={:?}",
                conv.map(|v| format!("0x{v:08X}")),
            );
            ActuationOutcome::Aborted(AbortReason::GenStale)
        }
        TargetVerifyOutcome::TargetMoved => {
            log::debug!(
                "[conv-actuate] Aborted(TargetMoved): target={:?}",
                conv.map(|v| format!("0x{v:08X}")),
            );
            ActuationOutcome::Aborted(AbortReason::TargetMoved)
        }
        TargetVerifyOutcome::Current(hwnd) => {
            // SAFETY: set_ime_romaji_mode_for_hwnd は unsafe fn。offload_unsafe は
            //         ワーカースレッドで実行するが SendMessageTimeoutW はクロス
            //         プロセス呼び出しのためスレッドに依存しない。HWND は SendableHwnd
            //         と同じ理由（プロセス内で有効なグローバルリソース）でスレッド間
            //         送信して安全。
            struct SendableHwnd(HWND);
            // SAFETY: 上記と同じ。
            unsafe impl Send for SendableHwnd {}
            let target_hwnd = SendableHwnd(hwnd);
            let success = offload_unsafe(move || {
                // 2021 edition の disjoint closure capture が `target_hwnd.0`
                // （中の HWND、Send でない）だけを直接キャプチャして
                // `SendableHwnd` の `unsafe impl Send` を迂回してしまうのを防ぐため、
                // 分解する前に一度 `target_hwnd` 全体を束縛し直す（既知のイディオム）。
                let target_hwnd = target_hwnd;
                let SendableHwnd(hwnd) = target_hwnd;
                unsafe { set_ime_romaji_mode_for_hwnd(hwnd, conv) }
            })
            .await;
            if success {
                log::debug!(
                    "[conv-actuate] Written: hwnd={hwnd:?} target={:?}",
                    conv.map(|v| format!("0x{v:08X}")),
                );
                ActuationOutcome::Written
            } else {
                log::debug!(
                    "[conv-actuate] Failed: hwnd={hwnd:?} target={:?}",
                    conv.map(|v| format!("0x{v:08X}")),
                );
                ActuationOutcome::Failed
            }
        }
    }
}

/// [`set_ime_open_then_conv_for_target`] が `open` 成功後に conv-mode も
/// 書くかどうかの指定。`Option<Option<u32>>` を避けるための3値 enum
/// （`clippy::option_option`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConvAfterOpen {
    /// conv は書かない。
    Skip,
    /// `open` が成功したら続けて書く。`None` は ROMAN ビット確保のみ
    /// （既存 conv に `IME_CMODE_ROMAN` を追加）、`Some(v)` は `v` をそのまま設定。
    Write(Option<u32>),
}

/// [`set_ime_open_then_conv_for_target`] の結果。
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImmCrossOutcome {
    pub open: ActuationOutcome,
    /// `conv_after_open` が `Some` かつ `open` が `Written` だったときのみ `Some`。
    pub conv: Option<ActuationOutcome>,
}

/// IME を ON/OFF し、成功かつ `conv_after_open` が `Some` なら続けて conv-mode を
/// 書き込む（ImmCross async path 専用、ADR-086 §1.2 欠陥1 の是正）。
///
/// [`set_ime_conv_for_target`] と違い、両方の書き込みを**同一の検証済み hwnd・
/// 同一の `offload_unsafe` クロージャ内**で行う。open と conv を別々に
/// `verify_still_current` すると、open の完了を待つ間にフォーカスが動いても
/// `ime_mode_focus_gen` の更新がメインスレッドのキュー処理を経て遅れるため、
/// conv 側の再検証だけでは検知できず、無関係な別ウィンドウへ ROMAN が着弾しうる
/// （opus レビュー指摘 2026-08-08、BUG-59 追補と同じクラスの誤爆）。1回の検証で
/// 得た hwnd を両方の書き込みに使い回すことでこれを構造的に防ぐ。
///
/// `target`/`read_current_focus_gen` は [`ActuationTarget::capture`]/
/// [`set_ime_conv_for_target`] と同じ規約。`open` が `Aborted`/`Failed` のとき、
/// 呼び出し元（`executor.rs`）は SSOT（`ImeModel::applied`）を明示的に巻き戻す
/// 必要はない —— `UnsafeToToggle` を `Runtime::on_ime_apply_complete` に渡すと
/// C/D（SSOT 書き込み）はスキップされる（INV-14: 一度も書いていないので Applied
/// として記録しない）ため、SSOT は元々「未確定」のまま変化しない。ただし
/// `Aborted(GenStale)`（同一ウィンドウのままフォーカス世代だけが進んだケース）は
/// 意図（IME を開く/閉じる）自体は依然として有効なのに、これを再試行する自然な
/// トリガー（新しいフォーカス変更）が発生しない。`on_ime_apply_complete` は
/// `UnsafeToToggle` でも E（`post_ime_refresh`）だけは必ず実行するため、20ms 後の
/// refresh サイクルが乖離を検出し再試行の起点になる（opus レビュー指摘 F3、
/// 2026-08-08 是正）。
#[allow(clippy::future_not_send)]
pub(crate) async fn set_ime_open_then_conv_for_target(
    target: ActuationTarget,
    open: bool,
    conv_after_open: ConvAfterOpen,
    read_current_focus_gen: impl FnOnce() -> u32,
) -> ImmCrossOutcome {
    match target.verify_still_current(read_current_focus_gen).await {
        TargetVerifyOutcome::GenStale => {
            log::debug!("[imm-cross-actuate] Aborted(GenStale): open={open}");
            ImmCrossOutcome {
                open: ActuationOutcome::Aborted(AbortReason::GenStale),
                conv: None,
            }
        }
        TargetVerifyOutcome::TargetMoved => {
            log::debug!("[imm-cross-actuate] Aborted(TargetMoved): open={open}");
            ImmCrossOutcome {
                open: ActuationOutcome::Aborted(AbortReason::TargetMoved),
                conv: None,
            }
        }
        TargetVerifyOutcome::Current(hwnd) => {
            // SAFETY: set_ime_open_for_target/set_ime_romaji_mode_for_hwnd は unsafe fn。
            //         offload_unsafe はワーカースレッドで実行するが両者とも
            //         SendMessageTimeoutW によるクロスプロセス呼び出しのためスレッドに
            //         依存しない。HWND は SendableHwnd と同じ理由（プロセス内で有効な
            //         グローバルリソース）でスレッド間送信して安全。
            struct SendableHwnd(HWND);
            // SAFETY: 上記と同じ。
            unsafe impl Send for SendableHwnd {}
            let target_hwnd = SendableHwnd(hwnd);
            let (open_ok, conv_ok) = offload_unsafe(move || {
                // disjoint closure capture がラッパーを迂回するのを防ぐための
                // 既知のイディオム（set_ime_conv_for_target 参照）。
                let target_hwnd = target_hwnd;
                let SendableHwnd(hwnd) = target_hwnd;
                let open_ok = unsafe { set_ime_open_for_target(hwnd, open) };
                let conv_ok = match (open_ok, conv_after_open) {
                    (true, ConvAfterOpen::Write(target_conv)) => {
                        Some(unsafe { set_ime_romaji_mode_for_hwnd(hwnd, target_conv) })
                    }
                    (false, _) | (true, ConvAfterOpen::Skip) => None,
                };
                (open_ok, conv_ok)
            })
            .await;
            let open_outcome = if open_ok {
                ActuationOutcome::Written
            } else {
                ActuationOutcome::Failed
            };
            let conv_outcome = conv_ok.map(|ok| {
                if ok {
                    ActuationOutcome::Written
                } else {
                    ActuationOutcome::Failed
                }
            });
            log::debug!(
                "[imm-cross-actuate] hwnd={hwnd:?} open={open_outcome:?} conv={conv_outcome:?}"
            );
            ImmCrossOutcome {
                open: open_outcome,
                conv: conv_outcome,
            }
        }
    }
}

#[cfg(test)]
mod actuation_target_tests {
    use super::{ActuationTarget, TargetVerifyOutcome};
    use windows::Win32::Foundation::HWND;

    fn hwnd(raw: isize) -> HWND {
        HWND(raw as *mut core::ffi::c_void)
    }

    #[test]
    fn compare_current_when_hwnd_matches() {
        assert_eq!(
            ActuationTarget::compare(hwnd(1), hwnd(1)),
            TargetVerifyOutcome::Current(hwnd(1))
        );
    }

    #[test]
    fn compare_target_moved_when_hwnd_differs() {
        assert_eq!(
            ActuationTarget::compare(hwnd(1), hwnd(2)),
            TargetVerifyOutcome::TargetMoved
        );
        assert_eq!(
            ActuationTarget::compare(hwnd(0), hwnd(1)),
            TargetVerifyOutcome::TargetMoved
        );
    }
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

/// Caps Lock のロック状態（トグル表示灯）を読む。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
#[must_use]
pub unsafe fn is_caps_lock_on() -> bool {
    windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x14) & 1 != 0
}

/// Caps Lock の状態をトグルする。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn toggle_caps_lock() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    let press = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0x14), // VK_CAPITAL
                wScan: 0,
                dwFlags: KEYBD_EVENT_FLAGS(0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let release = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0x14), // VK_CAPITAL
                wScan: 0,
                dwFlags: KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let _ = crate::win32::send_input_safe(&[press, release]);
}

/// クロスプロセスで IME の ON/OFF を切り替え、変換モードのマスクを適用する。
///
/// 呼び出し時点の `GetForegroundWindow()` を対象にする。トレイメニュー等、対象
/// ウィンドウを別途確定済みの呼び出し元は [`set_ime_mode_for_target`] を使うこと
/// （理由は [`set_ime_open_for_target`] の doc を参照）。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
#[must_use]
pub unsafe fn set_ime_mode(
    ime_on: bool,
    target_conv_mask_to_set: u32,
    target_conv_mask_to_clear: u32,
) -> bool {
    let Some(hwnd) = GetForegroundWindow().non_null() else {
        return false;
    };
    unsafe {
        set_ime_mode_for_target(
            hwnd,
            ime_on,
            target_conv_mask_to_set,
            target_conv_mask_to_clear,
        )
    }
}

/// [`set_ime_mode`] のターゲット指定版。IME open/close と conv mode の両方を `hwnd`
/// に対して発行する（[`set_ime_open_for_target`] を参照）。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
#[must_use]
pub unsafe fn set_ime_mode_for_target(
    hwnd: HWND,
    ime_on: bool,
    target_conv_mask_to_set: u32,
    target_conv_mask_to_clear: u32,
) -> bool {
    let open_ok = unsafe { set_ime_open_for_target(hwnd, ime_on) };
    if !ime_on {
        return open_ok;
    }
    let Some(ime_wnd) = (unsafe { crate::imm::get_ime_wnd(hwnd) }) else {
        return false;
    };
    let Some((_, _, _, success)) = (unsafe {
        modify_conv_mode(ime_wnd, |conv| {
            (conv | target_conv_mask_to_set) & !target_conv_mask_to_clear
        })
    }) else {
        return false;
    };
    success
}

// クロスプロセスで IME のローマ字/かな入力を切り替える `set_ime_romaji_mode_state`/
// `set_ime_romaji_mode_state_for_target`（`ImmSetConversionStatus` ベース）は
// BUG-61 で撤去した。tray の「ローマ字」「かな」コマンドの唯一の呼び出し元で、
// 実機確認の結果 IMC write が実モードに反映されず押しても変化しないことが
// 確認されたため（`docs/known-bugs.md` BUG-61）。代替として試した
// `Runtime::tray_inject_romaji_mode_vk`（Ctrl+Alt+R/K デバッグホットキー、
// `VK_DBE_ROMAN`/`VK_DBE_NOROMAN` の SendInput）も、scan コードを実機で
// 効くと確認済みの値に固定した上で再検証して無反応と確定したため撤去した
// （BUG-61 追補）。Win32 にこの入力方式を外部から切り替える公式手段は
// 存在しないと確定している。
