use std::cell::UnsafeCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
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
fn classify_key(vk: VkCode, scan: ScanCode, config: &HookConfig) -> (KeyClassification, Option<PhysicalPos>) {
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
const fn classify_modifier(vk: VkCode) -> Option<ModifierKey> {
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
fn classify_ime_relevance(vk: VkCode) -> ImeRelevance {
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
    config.left_thumb_vk = left.0;
    config.right_thumb_vk = right.0;
}

/// 現在時刻を `GetTickCount64` ミリ秒で返す。
pub fn current_tick_ms() -> u64 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

/// フックが応答しているかを判定する。
///
/// `timeout_ms` 以内にフックコールバックが呼ばれていれば `true`。
/// まだ一度も呼ばれていない場合も `true`（起動直後はキー入力がない）。
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
    unsafe {
        // 旧ハンドルがあれば念のため解除（既に無効な可能性あり）
        let old = *HOOK_HANDLE.get_mut();
        if !old.0.is_null() {
            let _ = UnhookWindowsHookEx(old);
        }

        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_callback), None, 0) {
            Ok(new_handle) => {
                HOOK_HANDLE.set(new_handle);
                // Update last_hook_activity_ms via APP
                if let Some(app) = crate::APP.get_mut() {
                    app.platform_state.last_hook_activity_ms = current_tick_ms();
                }
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

/// フックコールバックで使うコールバック関数（構造的に必要: OS コールバックから参照）
static KEY_EVENT_CALLBACK: SingleThreadCell<Option<Box<dyn FnMut(RawKeyEvent) -> CallbackResult>>> =
    SingleThreadCell::new(None);

/// キーの処理先を決定する。
///
/// Engine に送るか、OS にそのまま渡すかを一元的に判定する。
/// KeyUp は KeyDown の判定に自動追随する（ペア保証）。
///
/// # Safety
/// `GetAsyncKeyState` を呼び出す。フックコールバック内から呼ぶこと。
unsafe fn classify_route(hook: &HookRoutingState, config: &HookConfig, vk: u16, is_keydown: bool) -> KeyRoute {
    // KeyUp は KeyDown の判定に従う（ペア保証）
    if !is_keydown {
        let idx = (vk as usize) / 64;
        let bit = 1u64 << ((vk as usize) % 64);
        if idx >= 4 {
            return KeyRoute::Bypass;
        }
        if (hook.track_only_keys[idx] & bit) != 0 {
            return KeyRoute::TrackOnly;
        }
        if (hook.sent_to_engine[idx] & bit) != 0 {
            return KeyRoute::Engine;
        }
        return KeyRoute::Bypass;
    }

    // KeyDown の分類
    // Ctrl/Alt/Win 自体 → TrackOnly（Engine の InputTracker を更新するが OS には必ず通す）
    if is_non_shift_modifier(vk) {
        return KeyRoute::TrackOnly;
    }

    // Ctrl/Alt/Win が押されている間のキーはショートカット → Bypass
    // 例外1: 親指キーは Engine のコンボキー用
    // 例外2: IME 制御コンボ直後の Ctrl はコンボで消費済み（suppress_ctrl_bypass）
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let ctrl = (GetAsyncKeyState(0x11).cast_unsigned() & 0x8000) != 0;
        let alt = (GetAsyncKeyState(0x12).cast_unsigned() & 0x8000) != 0;
        let win = (GetAsyncKeyState(0x5B).cast_unsigned() & 0x8000) != 0
            || (GetAsyncKeyState(0x5C).cast_unsigned() & 0x8000) != 0;
        let effective_ctrl = ctrl && !(hook.suppress_ctrl_bypass && !alt && !win);
        if effective_ctrl || alt || win {
            if vk != config.left_thumb_vk && vk != config.right_thumb_vk {
                return KeyRoute::Bypass;
            }
        }
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

/// SENT_TO_ENGINE ビットセットに Engine ルートで記録する
fn mark_sent_to_engine(hook: &mut HookRoutingState, vk: u16) {
    let idx = (vk as usize) / 64;
    let bit = 1u64 << ((vk as usize) % 64);
    if idx < 4 {
        hook.sent_to_engine[idx] |= bit;
    }
}

/// SENT_TO_ENGINE / TRACK_ONLY_KEYS ビットセットに TrackOnly ルートで記録する
fn mark_sent_as_track_only(hook: &mut HookRoutingState, vk: u16) {
    let idx = (vk as usize) / 64;
    let bit = 1u64 << ((vk as usize) % 64);
    if idx < 4 {
        hook.sent_to_engine[idx] |= bit;
        hook.track_only_keys[idx] |= bit;
    }
}

/// SENT_TO_ENGINE / TRACK_ONLY_KEYS からキーを削除する
fn clear_sent_to_engine(hook: &mut HookRoutingState, vk: u16) {
    let idx = (vk as usize) / 64;
    let bit = 1u64 << ((vk as usize) % 64);
    if idx < 4 {
        hook.sent_to_engine[idx] &= !bit;
        hook.track_only_keys[idx] &= !bit;
    }
}

/// SENT_TO_ENGINE ビットセットを OS のキー状態と同期する。
///
/// `GetAsyncKeyState` で実際に押されていないのにビットが残っているキーをクリアする。
/// 500ms ポーリングで呼び出すことで、KeyUp 取りこぼしによるゴミを回収する。
///
/// # Safety
/// `GetAsyncKeyState` を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn sync_sent_to_engine(hook: &mut HookRoutingState) {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    let mut cleared = 0u32;
    for idx in 0..4 {
        if hook.sent_to_engine[idx] == 0 {
            continue;
        }
        let mut remaining = hook.sent_to_engine[idx];
        while remaining != 0 {
            let bit_pos = remaining.trailing_zeros() as usize;
            let vk = (idx * 64 + bit_pos) as i32;
            let bit = 1u64 << bit_pos;
            // OS でキーが離されている → ビットをクリア
            if (GetAsyncKeyState(vk).cast_unsigned() & 0x8000) == 0 {
                hook.sent_to_engine[idx] &= !bit;
                hook.track_only_keys[idx] &= !bit;
                cleared += 1;
            }
            remaining &= remaining - 1;
        }
    }
    if cleared > 0 {
        log::debug!("sync_sent_to_engine: cleared {cleared} stale bit(s)");
    }
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
pub fn install_hook(
    callback: Box<dyn FnMut(RawKeyEvent) -> CallbackResult>,
) -> windows::core::Result<HookGuard> {
    unsafe {
        KEY_EVENT_CALLBACK.set(Some(callback));

        let handle = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_callback), None, 0)?;
        HOOK_HANDLE.set(handle);

        log::info!("Keyboard hook installed successfully");
    }
    Ok(HookGuard { _private: () })
}

/// WH_KEYBOARD_LL フックコールバック
///
/// キーイベントの処理先を `classify_route` で一元的に判定し、
/// `HookRoutingState` のビットセットで KeyDown/KeyUp ペアを構造的に保証する。
/// 状態は `APP.platform_state` から読み書きする。
unsafe extern "system" fn hook_callback(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let hook_handle = *HOOK_HANDLE.get_mut();

    if ncode >= 0 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

        // ── 自己注入チェック（無限ループ防止）──
        if kb.dwExtraInfo == INJECTED_MARKER {
            // ハートビートは自己注入でも更新する（ping 応答のため）
            if let Some(app) = crate::APP.get_mut() {
                app.platform_state.last_hook_activity_ms = current_tick_ms();
                app.platform_state.hook_event_count += 1;
            }
            return CallNextHookEx(hook_handle, ncode, wparam, lparam);
        }

        // ── APP からプラットフォーム状態を取得 ──
        let Some(app) = crate::APP.get_mut() else {
            return CallNextHookEx(hook_handle, ncode, wparam, lparam);
        };
        let ps = &mut app.platform_state;

        // ── ハートビート更新（ウォッチドッグ用）──
        ps.last_hook_activity_ms = current_tick_ms();
        ps.hook_event_count += 1;

        // ── かな入力方式バイパス ──
        if !ps.preconditions.is_romaji {
            return CallNextHookEx(hook_handle, ncode, wparam, lparam);
        }

        // ── NonText ウィンドウバイパス ──
        // フォーカスが NonText（システムウィンドウ、タスクバー等）の場合は
        // Engine をバイパスして OS にそのまま通す。
        // フォーカス切替時の中間ウィンドウで Engine が誤作動するのを防止。
        if ps.focus_kind == awase::types::FocusKind::NonText {
            return CallNextHookEx(hook_handle, ncode, wparam, lparam);
        }

        let vk_raw = kb.vkCode as u16;
        let is_keydown = matches!(
            wparam.0 as u32,
            WM_KEYDOWN | WM_SYSKEYDOWN
        );

        // ── 修飾キータイミング追跡（同時押し判定用）──
        update_modifier_timing(&mut ps.modifier_timing, vk_raw, is_keydown);

        // Ctrl KeyUp で suppress_ctrl_bypass を解除
        if !is_keydown && matches!(vk_raw, 0x11 | 0xA2 | 0xA3) && ps.hook.suppress_ctrl_bypass {
            ps.hook.suppress_ctrl_bypass = false;
        }

        // ── 一元的なルーティング判定 ──
        let route = classify_route(&ps.hook, &ps.hook_config, vk_raw, is_keydown);

        match route {
            KeyRoute::Bypass => {
                log::trace!(
                    "Hook: vk=0x{vk_raw:02X} {} → Bypass",
                    if is_keydown { "KeyDown" } else { "KeyUp" }
                );
                return CallNextHookEx(hook_handle, ncode, wparam, lparam);
            }
            KeyRoute::Engine | KeyRoute::TrackOnly => {
                // KeyDown/KeyUp ペア追跡を更新
                if is_keydown {
                    match route {
                        KeyRoute::Engine => mark_sent_to_engine(&mut ps.hook, vk_raw),
                        KeyRoute::TrackOnly => mark_sent_as_track_only(&mut ps.hook, vk_raw),
                        KeyRoute::Bypass => unreachable!(),
                    }
                } else {
                    clear_sent_to_engine(&mut ps.hook, vk_raw);
                }
            }
        }

        // ── 再入ガード ──
        if ps.hook.in_callback {
            return CallNextHookEx(hook_handle, ncode, wparam, lparam);
        }
        ps.hook.in_callback = true;

        let event_type = if is_keydown {
            KeyEventType::KeyDown
        } else {
            KeyEventType::KeyUp
        };

        let vk = VkCode(vk_raw);
        let scan = ScanCode(kb.scanCode);
        let (key_classification, physical_pos) = classify_key(vk, scan, &ps.hook_config);
        let mut event = RawKeyEvent {
            vk_code: vk,
            scan_code: scan,
            event_type,
            extra_info: kb.dwExtraInfo,
            timestamp: now_timestamp(),
            key_classification,
            physical_pos,
            ime_relevance: classify_ime_relevance(vk),
            modifier_key: classify_modifier(vk),
        };

        // Drop ps borrow — subsequent sections re-acquire app from APP
        let _ = ps;

        // ── IME 事前分類の補完（sync key 判定）──
        app.enrich_ime_relevance(&mut event);

        log::trace!(
            "Hook: vk=0x{:02X} scan=0x{:04X} type={:?} → {}",
            event.vk_code.0,
            event.scan_code.0,
            event.event_type,
            if matches!(route, KeyRoute::TrackOnly) { "TrackOnly" } else { "Engine" }
        );

        // ── IME トグルガード ──
        {
            let is_sync_key = event.ime_relevance.is_sync_key;

            // sync key KeyDown → activate guard, flush Engine pending, PassThrough
            if is_keydown && is_sync_key && !app.platform_state.ime_guard.active {
                app.platform_state.ime_guard.active = true;
                log::debug!("IME guard ON (sync key vk=0x{:02X})", vk_raw);

                // Flush engine pending keys
                let ctx = crate::runtime::build_input_context(&app.platform_state.preconditions, &app.platform_state.modifier_timing);
                app.platform_state.hook.in_callback = false;
                let decision = app.engine.on_command(
                    awase::engine::EngineCommand::InvalidateContext(awase::types::ContextChange::ImeOff),
                    &ctx,
                );
                app.executor.execute_from_loop(decision);
                return CallNextHookEx(hook_handle, ncode, wparam, lparam);
            }

            // sync key KeyUp → deactivate guard, PassThrough, request deferred processing
            if !is_keydown && is_sync_key && app.platform_state.ime_guard.active {
                app.platform_state.ime_guard.active = false;
                log::debug!("IME guard OFF (sync key vk=0x{:02X})", vk_raw);
                app.platform_state.hook.in_callback = false;
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    HWND::default(),
                    crate::WM_PROCESS_DEFERRED,
                    WPARAM(0),
                    LPARAM(0),
                );
                return CallNextHookEx(hook_handle, ncode, wparam, lparam);
            }

            // Guard active → buffer key (ただし修飾キーは OS にパススルー)
            if app.platform_state.ime_guard.active {
                // TrackOnly（Ctrl/Alt/Win）は OS にパススルー。
                // バッファすると KeyUp が OS に届かず修飾キーが「押しっぱなし」になる。
                if matches!(route, KeyRoute::TrackOnly) {
                    app.platform_state.hook.in_callback = false;
                    return CallNextHookEx(hook_handle, ncode, wparam, lparam);
                }
                if app.platform_state.ime_guard.deferred_keys.len() < 10 {
                    let phys = awase::engine::input_tracker::PhysicalKeyState::empty();
                    app.platform_state.ime_guard.deferred_keys.push((event, phys));
                    log::trace!("IME guard: buffered key vk=0x{:02X}", vk_raw);
                } else {
                    log::warn!("IME guard forced clear: buffer overflow");
                    app.platform_state.ime_guard.active = false;
                }
                app.platform_state.hook.in_callback = false;
                return LRESULT(1); // Consumed
            }
        }

        // ── Engine コールバック呼び出し ──
        let result = KEY_EVENT_CALLBACK
            .get_mut()
            .as_mut()
            .map_or(CallbackResult::PassThrough, |callback| callback(event));

        // Re-acquire platform_state for in_callback reset
        // (callback may have accessed APP.get_mut() internally)
        if let Some(app) = crate::APP.get_mut() {
            // コンボキー消費後の猶予クリア:
            // Engine が KeyDown を consumed した場合、Ctrl/Alt コンボが成立したことを意味する。
            // 猶予を維持すると直後のキーが OsModifierHeld でバイパスされるため即座にクリア。
            if is_keydown && matches!(result, CallbackResult::Consumed) {
                app.platform_state.modifier_timing.clear_grace();
            }
            app.platform_state.hook.in_callback = false;
        }

        // TrackOnly: Engine の結果を無視して常に PassThrough（修飾キースタック防止）
        if matches!(route, KeyRoute::TrackOnly) {
            return CallNextHookEx(hook_handle, ncode, wparam, lparam);
        }

        match result {
            CallbackResult::Consumed => {
                return LRESULT(1);
            }
            CallbackResult::PassThrough => {}
        }
    }

    CallNextHookEx(hook_handle, ncode, wparam, lparam)
}

/// フックで受け取った修飾キーイベントから `ModifierTiming` を更新する。
fn update_modifier_timing(timing: &mut crate::ModifierTiming, vk: u16, is_keydown: bool) {
    match vk {
        0x11 | 0xA2 | 0xA3 => {
            if is_keydown {
                timing.ctrl_down = true;
            } else {
                timing.ctrl_down = false;
                timing.ctrl_up_tick = current_tick_ms();
            }
        }
        0x12 | 0xA4 | 0xA5 => {
            if is_keydown {
                timing.alt_down = true;
            } else {
                timing.alt_down = false;
                timing.alt_up_tick = current_tick_ms();
            }
        }
        _ => {}
    }
}

/// 起動時点からの経過マイクロ秒を返す（`Instant` を内部的に使用）
fn now_timestamp() -> Timestamp {
    use std::sync::OnceLock;
    use std::time::Instant;
    static BASELINE: OnceLock<Instant> = OnceLock::new();
    let baseline = BASELINE.get_or_init(Instant::now);
    baseline.elapsed().as_micros() as u64
}
