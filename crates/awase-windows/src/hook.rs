use std::cell::UnsafeCell;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_SYSKEYDOWN,
};

use crate::output::INJECTED_MARKER;
use crate::scanmap::scan_to_pos;
use crate::{HookConfig, HookRoutingState};
use awase::scanmap::PhysicalPos;
use awase::types::{
    ImeRelevance, KeyClassification, KeyEventType, ModifierKey, RawKeyEvent, ScanCode,
    ShadowImeAction, Timestamp, VkCode,
};

/// Windows VK + ScanCode からキー分類と物理位置を生成する
#[must_use] 
pub fn classify_key(vk: VkCode, scan: ScanCode, config: &HookConfig) -> (KeyClassification, Option<PhysicalPos>) {
    use crate::vk;

    let left_thumb = VkCode(config.left_thumb_vk);
    let right_thumb = VkCode(config.right_thumb_vk);

    if vk == left_thumb {
        (KeyClassification::LeftThumb, None)
    } else if vk == right_thumb {
        (KeyClassification::RightThumb, None)
    } else if vk::is_passthrough(vk) {
        (KeyClassification::Passthrough, None)
    } else if let Some(pos) = scan_to_pos(scan) {
        (KeyClassification::Char, Some(pos))
    } else {
        (KeyClassification::Passthrough, None)
    }
}

/// Windows VK コードから修飾キー分類を生成する
#[must_use] 
pub const fn classify_modifier(vk: VkCode) -> Option<ModifierKey> {
    match vk.0 {
        0x10 | 0xA0 | 0xA1 => Some(ModifierKey::Shift),
        0x11 | 0xA2 | 0xA3 => Some(ModifierKey::Ctrl),
        0x12 | 0xA4 | 0xA5 => Some(ModifierKey::Alt),
        0x5B | 0x5C => Some(ModifierKey::Meta),
        _ => None,
    }
}

/// Shift 以外の修飾キー（Ctrl, Alt, Win）かどうかを判定する。
///
/// これらのキーは NICOLA 処理に関与しないため、Engine をバイパスして
/// 常に OS に直接渡す。KeyDown/KeyUp ペアの保証により Ctrl スタックを防止する。
const fn is_non_shift_modifier(vk: u16) -> bool {
    matches!(
        vk,
        0x11 | 0xA2 | 0xA3  // VK_CONTROL, VK_LCONTROL, VK_RCONTROL
        | 0x12 | 0xA4 | 0xA5  // VK_MENU, VK_LMENU, VK_RMENU
        | 0x5B | 0x5C          // VK_LWIN, VK_RWIN
    )
}

/// Windows VK コードから IME 関連の事前分類情報を生成する
#[must_use] 
pub fn classify_ime_relevance(vk: VkCode) -> ImeRelevance {
    use crate::vk;

    let ime_key = vk::ImeKeyKind::from_vk(vk);
    let shadow_action = ime_key.map(|k| match k.shadow_effect() {
        vk::ShadowImeEffect::TurnOn => ShadowImeAction::TurnOn,
        vk::ShadowImeEffect::TurnOff => ShadowImeAction::TurnOff,
        vk::ShadowImeEffect::Toggle => ShadowImeAction::Toggle,
    });

    // Note: is_sync_key and sync_direction are set later by the runtime
    // when it has access to the config. This function only classifies
    // hardware-level IME properties.
    ImeRelevance {
        may_change_ime: ime_key.is_some() || vk::may_change_ime(vk),
        shadow_action,
        is_sync_key: false,   // set by runtime with config
        sync_direction: None, // set by runtime with config
        is_ime_control: vk::is_ime_control(vk),
    }
}

/// 親指キー VK コードを設定する（config 読み込み後に呼ぶ）
pub fn set_thumb_vk_codes(config: &mut HookConfig, left: VkCode, right: VkCode) {
    config.left_thumb_vk = u16::from(left);
    config.right_thumb_vk = u16::from(right);
}

/// 現在時刻を `GetTickCount64` ミリ秒で返す。
#[must_use] 
pub fn current_tick_ms() -> u64 {
    // SAFETY: GetTickCount64 はどのスレッドからも安全に呼び出せるスレッドセーフな Win32 API。
    //         引数なし・副作用なし・内部ロックにより安全性が保証される。
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

/// フックが応答しているかを判定する。
///
/// `timeout_ms` 以内にフックコールバックが呼ばれていれば `true`。
/// まだ一度も呼ばれていない場合も `true`（起動直後はキー入力がない）。
#[must_use] 
pub fn is_hook_responsive(ps: &crate::PlatformState, timeout_ms: u64) -> bool {
    let last = ps.last_hook_activity_ms;
    if last == 0 {
        return true; // 起動直後: まだキー入力がない
    }
    let now = current_tick_ms();
    (now - last) < timeout_ms
}

/// フックの生存確認用 ping を送信する。
///
/// `INJECTED_MARKER` 付きの VK_NONAME (0xFC) KeyDown+KeyUp を SendInput で送信する。
/// フックが生きていればコールバックが呼ばれ、`last_hook_activity_ms` が更新される。
/// フックが死んでいれば何も起きない。
///
/// # Panics
/// `INPUT` 構造体のサイズが `i32` に収まらない場合（実際には発生しない）。
///
/// # Safety
/// Win32 API (`SendInput`) を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn send_ping() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VIRTUAL_KEY,
    };

    let vk_noname = 0xFC_u16; // VK_NONAME — 何も入力されない無害なキー
    let inputs = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk_noname),
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: INJECTED_MARKER,
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk_noname),
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: INJECTED_MARKER,
                },
            },
        },
    ];
    SendInput(
        &inputs,
        i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
    );
    log::trace!("Hook ping sent");
}

/// フックを再登録する（OS に無言で削除された場合の自動復旧用）。
///
/// コールバックは既にグローバルに保持されているため、
/// `SetWindowsHookExW` を再度呼んでハンドルを差し替えるだけ。
/// UAC 昇格もプロセス再起動も不要。
pub fn reinstall_hook() -> bool {
    // SAFETY: HOOK_HANDLE はシングルスレッド専用セルであり、本関数はメインスレッドから
    //         のみ呼ばれることが呼出元で保証されている。SetWindowsHookExW / UnhookWindowsHookEx
    //         はグローバルフックハンドルの登録・解除に必要な Win32 API。
    unsafe {
        // 旧ハンドルがあれば念のため解除（既に無効な可能性あり）
        let old = *HOOK_HANDLE.get_mut();
        if !old.0.is_null() {
            let _ = UnhookWindowsHookEx(old);
        }

        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_callback), None, 0) {
            Ok(new_handle) => {
                HOOK_HANDLE.set(new_handle);
                log::info!("Keyboard hook reinstalled successfully");
                true
            }
            Err(e) => {
                log::error!("Failed to reinstall keyboard hook: {e}");
                HOOK_HANDLE.set(HHOOK(std::ptr::null_mut()));
                false
            }
        }
    }
}

/// シングルスレッド専用のグローバルセル（main.rs と同じパターン）
struct SingleThreadCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for SingleThreadCell<T> {}

impl<T> SingleThreadCell<T> {
    const fn new(val: T) -> Self {
        Self(UnsafeCell::new(val))
    }

    unsafe fn get_mut(&self) -> &mut T {
        &mut *self.0.get()
    }

    unsafe fn set(&self, val: T) {
        *self.0.get() = val;
    }
}

/// グローバルなフックハンドル（構造的に必要: OS コールバックから参照）
static HOOK_HANDLE: SingleThreadCell<HHOOK> = SingleThreadCell::new(HHOOK(std::ptr::null_mut()));

/// hook_callback が in_with_app 時に使うフック設定のグローバルコピー。
///
/// APP.get_mut() を呼べない状況でも classify_key が実行できるように、
/// bootstrap 時（および thumb_vk 変更時）に更新する。
static HOOK_CONFIG: SingleThreadCell<HookConfig> = SingleThreadCell::new(HookConfig {
    left_thumb_vk: 0,
    right_thumb_vk: 0,
});

/// グローバル HOOK_CONFIG を更新する。bootstrap と thumb_vk 変更後に呼ぶ。
pub fn update_hook_config(config: HookConfig) {
    // SAFETY: HOOK_CONFIG はシングルスレッド専用セル。本関数はメインスレッドから
    //         のみ呼ばれることが呼出元で保証されており、競合アクセスは発生しない。
    unsafe { HOOK_CONFIG.set(config); }
}

/// フックコールバックで使うコールバック関数（構造的に必要: OS コールバックから参照）
static KEY_EVENT_CALLBACK: SingleThreadCell<Option<Box<dyn FnMut(RawKeyEvent) -> CallbackResult>>> =
    SingleThreadCell::new(None);

/// KeyUp 専用ルート判定（KeyDown の判定に追随する）。
const fn classify_keyup_route(hook: &HookRoutingState, vk: u16) -> KeyRoute {
    if hook.is_track_only(vk) {
        return KeyRoute::TrackOnly;
    }
    if hook.is_engine_sent(vk) {
        return KeyRoute::Engine;
    }
    KeyRoute::Bypass
}

/// Ctrl/Alt/Win が押されている間の非親指キーはショートカット → Bypass。
///
/// `ctrl_bypass_hold` が有効な場合（IME 制御コンボ直後）は Ctrl を有効とみなさない。
const fn is_shortcut_bypass(mods: awase::engine::fsm_types::ModifierState, hook: &HookRoutingState, config: HookConfig, vk: u16) -> bool {
    let effective_ctrl = mods.ctrl && !(hook.ctrl_bypass_hold() && !mods.alt && !mods.win);
    if (effective_ctrl || mods.alt || mods.win)
        && vk != config.left_thumb_vk && vk != config.right_thumb_vk {
            return true;
        }
    false
}

/// キーの処理先を決定する。
///
/// Engine に送るか、OS にそのまま渡すかを一元的に判定する。
/// KeyUp は KeyDown の判定に自動追随する（ペア保証）。
///
/// # Safety
/// `GetAsyncKeyState` を呼び出す。フックコールバック内から呼ぶこと。
unsafe fn classify_route(hook: &HookRoutingState, config: HookConfig, vk: u16, is_keydown: bool) -> KeyRoute {
    if !is_keydown {
        return classify_keyup_route(hook, vk);
    }
    if is_non_shift_modifier(vk) {
        return KeyRoute::TrackOnly;
    }
    let mods = crate::observer::focus_observer::read_os_modifiers();
    if is_shortcut_bypass(mods, hook, config, vk) {
        return KeyRoute::Bypass;
    }
    KeyRoute::Engine
}

/// classify_route の結果
enum KeyRoute {
    /// Engine で NICOLA 処理する（Consume/PassThrough は Engine が決める）
    Engine,
    /// Engine に送って状態追跡させるが、結果を無視して常に OS に PassThrough する。
    /// Ctrl/Alt/Win 用: InputTracker の更新と保留キーのフラッシュに必要だが、
    /// Engine が誤って Consume しても OS には必ず通す（修飾キースタック防止）。
    TrackOnly,
    /// Engine に一切送らず OS にそのまま渡す
    Bypass,
}

/// SENT_TO_ENGINE ビットセットを OS のキー状態と同期する。
///
/// `GetAsyncKeyState` で実際に押されていないのにビットが残っているキーをクリアする。
/// 500ms ポーリングで呼び出すことで、KeyUp 取りこぼしによるゴミを回収する。
///
/// # Safety
/// `GetAsyncKeyState` を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn sync_sent_to_engine(hook: &mut HookRoutingState) {
    // SAFETY: sync_with_os_key_state は内部で GetAsyncKeyState を呼び出す。
    //         呼出元の # Safety 節でメインスレッドからの呼び出しが保証されているため安全。
    unsafe { hook.sync_with_os_key_state() };
}

/// コールバックの戻り値
#[derive(Debug)]
pub enum CallbackResult {
    /// 元キーを握りつぶす（LRESULT(1)）
    Consumed,
    /// 元キーをそのまま通す
    PassThrough,
}

/// フック解除を保証する RAII ガード
///
/// スコープを抜けると自動的に `UnhookWindowsHookEx` を呼び出し、
/// コールバックもクリアする。
#[derive(Debug)]
pub struct HookGuard {
    _private: (), // 外部から直接構築させない
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        // SAFETY: HookGuard は install_hook が返した唯一のインスタンスであり、
        //         drop は1回だけ呼ばれる。HOOK_HANDLE はシングルスレッド専用セルで
        //         メインスレッドからのみアクセスされることが構造的に保証されている。
        unsafe {
            let handle = *HOOK_HANDLE.get_mut();
            if !handle.0.is_null() {
                let _ = UnhookWindowsHookEx(handle);
                HOOK_HANDLE.set(HHOOK(std::ptr::null_mut()));
                log::info!("Keyboard hook uninstalled");
            }
            KEY_EVENT_CALLBACK.set(None);
        }
    }
}

/// フックを登録する
///
/// 返された `HookGuard` を保持している間フックが有効。
/// ドロップ時に自動解除される。
///
/// # Errors
/// `SetWindowsHookExW` が失敗した場合（OS エラー）。
pub fn install_hook(
    callback: Box<dyn FnMut(RawKeyEvent) -> CallbackResult>,
) -> windows::core::Result<HookGuard> {
    // SAFETY: KEY_EVENT_CALLBACK と HOOK_HANDLE はシングルスレッド専用セル。
    //         本関数はメインスレッドから1回のみ呼ばれることが呼出元で保証されており、
    //         SetWindowsHookExW に渡す hook_callback は 'static な unsafe extern fn。
    unsafe {
        KEY_EVENT_CALLBACK.set(Some(callback));

        let handle = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_callback), None, 0)?;
        HOOK_HANDLE.set(handle);

        log::info!("Keyboard hook installed successfully");
    }
    Ok(HookGuard { _private: () })
}

fn build_raw_key_event(
    vk: VkCode,
    scan: ScanCode,
    is_keydown: bool,
    extra_info: usize,
    key_classification: KeyClassification,
    physical_pos: Option<PhysicalPos>,
    modifier_snapshot: awase::engine::ModifierState,
) -> RawKeyEvent {
    RawKeyEvent {
        vk_code: vk,
        scan_code: scan,
        event_type: if is_keydown { KeyEventType::KeyDown } else { KeyEventType::KeyUp },
        extra_info,
        timestamp: now_timestamp(),
        key_classification,
        physical_pos,
        ime_relevance: classify_ime_relevance(vk),
        modifier_key: classify_modifier(vk),
        modifier_snapshot,
    }
}

/// 早期バイパス判定の結果
enum EarlyRoutingDecision {
    /// 即 CallNextHookEx（OS にパススルー）
    BypassToOs,
    /// 通常パイプラインへ
    Continue { route: KeyRoute },
}

/// IME ガード処理の結果
enum RoutingOutcome {
    /// CallNextHookEx（OS にパススルー）
    PassThrough,
    /// LRESULT(1)（消費済み）
    Consumed,
    /// ガード非該当 — Engine コールバックへ転送
    Forward(RawKeyEvent),
}

/// ハートビート更新・早期バイパス・ルーティング・KeyDown/KeyUp 追跡・再入ガードを一括処理。
///
/// # Invariant
/// `Continue` を返した場合は必ず `enter_callback()` を呼んでいる。
/// 呼び出し元は対応する `leave_callback()` を保証すること。
///
/// # Safety
/// 内部で `classify_route`（`GetAsyncKeyState`）を呼ぶ。フックコールバック内から呼ぶこと。
unsafe fn decide_routing_with_tracking(
    ps: &mut crate::PlatformState,
    vk_raw: u16,
    is_keydown: bool,
) -> EarlyRoutingDecision {
    ps.last_hook_activity_ms = current_tick_ms();
    ps.hook_event_count += 1;

    if !ps.input_mode().is_romaji_capable() {
        return EarlyRoutingDecision::BypassToOs;
    }
    // フォーカスが NonText（システムウィンドウ、タスクバー等）の場合はバイパス。
    // フォーカス切替時の中間ウィンドウで Engine が誤作動するのを防止。
    if ps.focus.focus_kind == awase::types::FocusKind::NonText {
        return EarlyRoutingDecision::BypassToOs;
    }

    // Ctrl KeyUp で ctrl_bypass_hold を解除
    if !is_keydown && matches!(vk_raw, 0x11 | 0xA2 | 0xA3) && ps.hook.ctrl_bypass_hold() {
        ps.hook.set_ctrl_bypass_hold(false);
    }

    let route = classify_route(&ps.hook, ps.hook_config, vk_raw, is_keydown);

    match route {
        KeyRoute::Bypass => {
            log::trace!(
                "Hook: vk=0x{vk_raw:02X} {} → Bypass",
                if is_keydown { "KeyDown" } else { "KeyUp" }
            );
            return EarlyRoutingDecision::BypassToOs;
        }
        KeyRoute::Engine | KeyRoute::TrackOnly => {
            if is_keydown {
                match route {
                    KeyRoute::Engine => ps.hook.mark_engine_sent(vk_raw),
                    KeyRoute::TrackOnly => ps.hook.mark_track_only_sent(vk_raw),
                    KeyRoute::Bypass => unreachable!(),
                }
            } else {
                ps.hook.clear_engine_sent(vk_raw);
            }
        }
    }

    if ps.hook.is_in_callback() {
        return EarlyRoutingDecision::BypassToOs;
    }
    ps.hook.enter_callback();

    EarlyRoutingDecision::Continue { route }
}

/// キーイベントのトレース/デバッグログを出力する。
fn log_hook_event(event: &RawKeyEvent, route: &KeyRoute) {
    log::trace!(
        "Hook: vk=0x{:02X} scan=0x{:04X} type={:?} → {}",
        event.vk_code.0,
        event.scan_code.0,
        event.event_type,
        if matches!(route, KeyRoute::TrackOnly) { "TrackOnly" } else { "Engine" }
    );
    // Passthrough class (Enter, Tab, Esc, 矢印キー等) は debug でも常時記録する。
    // 通常の char / thumb は trace 止まり（ノイズ削減）だが、Passthrough は頻度が
    // 低くかつ awase 出力との race condition 解析の鍵になるため debug で残す。
    if matches!(event.key_classification, KeyClassification::Passthrough) {
        log::debug!(
            "[hook-passthrough] vk={:#04x} scan={:#06x} type={:?} route={}",
            event.vk_code.0,
            event.scan_code.0,
            event.event_type,
            if matches!(route, KeyRoute::TrackOnly) { "TrackOnly" } else { "Engine" }
        );
    }
}

/// sync key ガードのステートマシン処理。
///
/// sync key（Kanji/Henkan/Muhenkan 等）の KeyDown 直後から KeyUp までの間、
/// 後続キーをバッファしておき、OS が IME 状態を切り替え終わってからまとめて再評価する。
///
/// # Invariant
/// `PassThrough` または `Consumed` を返した場合は内部で `leave_callback()` を呼んでいる。
/// `Forward` を返した場合は `leave_callback()` を呼ばない（Engine コールバック後に呼ぶ）。
fn apply_sync_key_gate(
    app: &mut crate::Runtime,
    event: RawKeyEvent,
    route: &KeyRoute,
    vk_raw: u16,
    is_keydown: bool,
) -> RoutingOutcome {
    let is_sync_key = event.ime_relevance.is_sync_key;

    // sync key KeyDown → ガードを起動、Engine の保留キーをフラッシュして PassThrough
    if is_keydown && is_sync_key && !app.platform_state.sync_key_gate.is_active() {
        app.platform_state.sync_key_gate.activate();
        log::debug!("sync-key guard ON (vk=0x{vk_raw:02X})");
        let ctx = crate::runtime::build_input_context(
            app.platform_state.preconditions(),
            &event.modifier_snapshot,
        );
        app.platform_state.hook.leave_callback();
        let decision = app.engine.on_command(
            awase::engine::EngineCommand::InvalidateContext(awase::types::ContextChange::ImeOff),
            &ctx,
        );
        app.executor.execute_from_loop(decision);
        return RoutingOutcome::PassThrough;
    }

    // sync key KeyUp → ガードを解除して PassThrough、遅延処理をリクエスト
    if !is_keydown && is_sync_key && app.platform_state.sync_key_gate.is_active() {
        app.platform_state.sync_key_gate.deactivate();
        log::debug!("sync-key guard OFF (vk=0x{vk_raw:02X})");
        app.platform_state.hook.leave_callback();
        crate::win32::post_to_main_thread(crate::WM_PROCESS_DEFERRED);
        return RoutingOutcome::PassThrough;
    }

    // ガードが有効 → キーをバッファ（修飾キーは OS にパススルー）
    if app.platform_state.sync_key_gate.is_active() {
        // TrackOnly（Ctrl/Alt/Win）は OS にパススルー。
        // バッファすると KeyUp が OS に届かず修飾キーが「押しっぱなし」になる。
        if matches!(route, KeyRoute::TrackOnly) {
            app.platform_state.hook.leave_callback();
            return RoutingOutcome::PassThrough;
        }
        let phys = awase::engine::input_tracker::PhysicalKeyState::empty();
        if app.platform_state.sync_key_gate.try_push(event, phys) {
            log::trace!("sync-key guard: buffered key vk=0x{vk_raw:02X}");
        } else {
            log::warn!("sync-key guard forced clear: buffer overflow");
            app.platform_state.sync_key_gate.deactivate();
        }
        app.platform_state.hook.leave_callback();
        return RoutingOutcome::Consumed;
    }

    RoutingOutcome::Forward(event)
}

/// 自己注入キーかどうかを判定する（無限ループ防止）。
const fn is_self_injected(extra_info: usize) -> bool {
    extra_info == INJECTED_MARKER
        || extra_info == crate::tsf::output::TSF_MARKER
        || extra_info == crate::tsf::output::IME_KANJI_MARKER
}

/// with_app 再入中にキーイベントを INPUT_DEFER に退避する。
///
/// # Safety
/// グローバルな `HOOK_CONFIG` へのアクセスおよび `read_os_modifiers` を呼ぶ。
/// フックコールバック内から呼ぶこと。
unsafe fn defer_key_during_with_app(vk: VkCode, scan: ScanCode, is_keydown: bool, extra_info: usize) {
    let config = HOOK_CONFIG.get_mut();
    let (key_classification, physical_pos) = classify_key(vk, scan, config);
    // SAFETY: read_os_modifiers は GetAsyncKeyState を呼び出す。
    //         呼出元の # Safety 節でフックコールバック内からの呼び出しが保証されているため安全。
    let modifier_snapshot = unsafe { crate::observer::focus_observer::read_os_modifiers() };
    let event = build_raw_key_event(vk, scan, is_keydown, extra_info, key_classification, physical_pos, modifier_snapshot);
    log::debug!("[in-with-app] queuing vk=0x{:02X} {:?}", vk.0, event.event_type);
    crate::INPUT_DEFER.defer_during_with_app(event);
}

/// `ncode >= 0` のフックイベントを処理する。
///
/// # Safety
/// Win32 API（`CallNextHookEx` 等）を呼び出す。フックコールバック内から呼ぶこと。
unsafe fn process_hook_event(hook_handle: HHOOK, ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    let vk_raw = kb.vkCode as u16;
    let vk = VkCode(vk_raw);
    let scan = ScanCode(kb.scanCode);
    let is_keydown = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);

    // ── 自己注入チェック（無限ループ防止）──
    // in_with_app チェックより先に行う: 注入キーは常にパススルー。
    if is_self_injected(kb.dwExtraInfo) {
        // ハートビートは自己注入でも更新する（ping 応答のため）。
        // with_app 再入中の場合は try_with_mut が None を返してスキップする。
        crate::RUNTIME.try_with_mut(|app| {
            app.platform_state.last_hook_activity_ms = current_tick_ms();
            app.platform_state.hook_event_count += 1;
        });
        return CallNextHookEx(Some(hook_handle), ncode, wparam, lparam);
    }

    // ── パニック連打検出（物理キー1回につき1回、自己注入を除く）──
    // 再入・通常パスのどちらを経由しても物理押下を1度だけカウントする。
    // 再入パスでは defer → replay が発生するため pipeline 側では数えない。
    if is_keydown && classify_ime_relevance(vk).may_change_ime {
        crate::panic_detect::record_ime_keydown(current_tick_ms());
    }

    // ── with_app 再入ガード ──
    // SendMessageTimeoutW (cross-process IME 制御) がメッセージポンプを起動した場合、
    // このコールバックが再呼び出しされる。APP.get_mut() は IN_WITH_APP=true の間
    // 呼べないため（&mut Runtime が二重に存在し UB）、キーを INPUT_DEFER に
    // 退避して drain 後に NICOLA で再処理する（CallNextHookEx での素通しは NICOLA バイパス）。
    if crate::in_with_app() {
        defer_key_during_with_app(vk, scan, is_keydown, kb.dwExtraInfo);
        return LRESULT(1); // Consumed — NICOLA 処理後に drain で再配送
    }

    // ── APP からプラットフォーム状態を取得 ──
    let Some(mut app_borrow) = crate::RUNTIME.try_borrow_mut() else {
        return CallNextHookEx(Some(hook_handle), ncode, wparam, lparam);
    };
    let Some(app) = app_borrow.as_mut() else {
        return CallNextHookEx(Some(hook_handle), ncode, wparam, lparam);
    };

    // ── 早期バイパス・ルーティング・再入ガード ──
    let route = match decide_routing_with_tracking(&mut app.platform_state, vk_raw, is_keydown) {
        EarlyRoutingDecision::BypassToOs => return CallNextHookEx(Some(hook_handle), ncode, wparam, lparam),
        EarlyRoutingDecision::Continue { route } => route,
    };

    // ── イベント構築と IME 事前分類補完（sync key 判定）──
    // フック時点の修飾キー状態を capture（drain 時の context 汚染防止）
    let (key_classification, physical_pos) = classify_key(vk, scan, &app.platform_state.hook_config);
    // SAFETY: read_os_modifiers は GetAsyncKeyState を呼び出す。
    //         本関数はフックコールバックから呼ばれるため、呼び出しコンテキストが保証されている。
    let modifier_snapshot = unsafe { crate::observer::focus_observer::read_os_modifiers() };
    let mut event = build_raw_key_event(vk, scan, is_keydown, kb.dwExtraInfo, key_classification, physical_pos, modifier_snapshot);
    app.enrich_ime_relevance(&mut event);
    log_hook_event(&event, &route);

    // ── sync key ガード ──
    let event = match apply_sync_key_gate(app, event, &route, vk_raw, is_keydown) {
        RoutingOutcome::PassThrough => return CallNextHookEx(Some(hook_handle), ncode, wparam, lparam),
        RoutingOutcome::Consumed    => return LRESULT(1),
        RoutingOutcome::Forward(ev) => ev,
    };

    // app ボローを解放して app_borrow を drop 可能にする（NLL）
    let _ = app;

    // ── RefMut を解放してから Engine コールバックを呼ぶ ──
    // callback 内部で with_app が APP の借用を取得するため、先に解放する。
    drop(app_borrow);

    // ── Engine コールバック呼び出し ──
    let result = KEY_EVENT_CALLBACK
        .get_mut()
        .as_mut()
        .map_or(CallbackResult::PassThrough, |callback| callback(event));

    // コールバック後に leave_callback を実行してフック再入ガードをリセット
    crate::RUNTIME.try_with_mut(|app| {
        app.platform_state.hook.leave_callback();
    });

    // TrackOnly: Engine の結果を無視して常に PassThrough（修飾キースタック防止）
    if matches!(route, KeyRoute::TrackOnly) {
        return CallNextHookEx(Some(hook_handle), ncode, wparam, lparam);
    }

    match result {
        CallbackResult::Consumed => LRESULT(1),
        CallbackResult::PassThrough => CallNextHookEx(Some(hook_handle), ncode, wparam, lparam),
    }
}

/// WH_KEYBOARD_LL フックコールバック
///
/// キーイベントの処理先を `classify_route` で一元的に判定し、
/// `HookRoutingState` のビットセットで KeyDown/KeyUp ペアを構造的に保証する。
/// 状態は `APP.platform_state` から読み書きする。
///
/// # Safety
/// OS から `WH_KEYBOARD_LL` フックコールバックとして呼び出される。
/// `ncode`・`wparam`・`lparam` は OS が保証する有効な値であり、
/// `HOOK_HANDLE` はシングルスレッド専用セルでメインスレッドからのみアクセスされる。
unsafe extern "system" fn hook_callback(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let hook_handle = *HOOK_HANDLE.get_mut();
    if ncode >= 0 {
        return process_hook_event(hook_handle, ncode, wparam, lparam);
    }
    CallNextHookEx(Some(hook_handle), ncode, wparam, lparam)
}


/// 起動時点からの経過マイクロ秒を返す（`Instant` を内部的に使用）。診断用に公開。
#[must_use] 
pub fn now_timestamp_us() -> u64 {
    now_timestamp()
}

/// 起動時点からの経過マイクロ秒を返す（`Instant` を内部的に使用）
fn now_timestamp() -> Timestamp {
    use std::sync::OnceLock;
    use std::time::Instant;
    static BASELINE: OnceLock<Instant> = OnceLock::new();
    let baseline = BASELINE.get_or_init(Instant::now);
    baseline.elapsed().as_micros() as u64
}
