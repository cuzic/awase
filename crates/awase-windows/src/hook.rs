use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::output::INJECTED_MARKER;
use crate::scanmap::scan_to_pos;
use awase::scanmap::PhysicalPos;
use awase::types::{
    ImeRelevance, KeyClassification, KeyEventType, ModifierKey, RawKeyEvent, ScanCode,
    ShadowImeAction, Timestamp, VkCode,
};

/// Windows VK + ScanCode からキー分類と物理位置を生成する
fn classify_key(vk: VkCode, scan: ScanCode) -> (KeyClassification, Option<PhysicalPos>) {
    use crate::vk;

    let left_thumb = VkCode(LEFT_THUMB_VK.load(Ordering::Relaxed));
    let right_thumb = VkCode(RIGHT_THUMB_VK.load(Ordering::Relaxed));

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

/// 左親指キーの VK コード（config から設定）
static LEFT_THUMB_VK: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0x1D); // VK_NONCONVERT

/// 右親指キーの VK コード（config から設定）
static RIGHT_THUMB_VK: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0x1C); // VK_CONVERT

/// 親指キー VK コードを設定する（config 読み込み後に呼ぶ）
pub fn set_thumb_vk_codes(left: VkCode, right: VkCode) {
    LEFT_THUMB_VK.store(left.0, Ordering::Relaxed);
    RIGHT_THUMB_VK.store(right.0, Ordering::Relaxed);
}

/// フックコールバックが最後に呼ばれた時刻（`GetTickCount64` ミリ秒）。
/// 0 はまだ一度も呼ばれていないことを意味する。
static LAST_HOOK_ACTIVITY: AtomicU64 = AtomicU64::new(0);

/// フックコールバックの累積呼び出し回数。
/// ウォッチドッグが前回チェック時の値と比較して、増えていなければフック消失。
static HOOK_EVENT_COUNT: AtomicU64 = AtomicU64::new(0);

/// フックイベントカウンタの現在値を返す
pub fn hook_event_count() -> u64 {
    HOOK_EVENT_COUNT.load(Ordering::Relaxed)
}

/// フックコールバックの最終活動時刻を返す（`GetTickCount64` ミリ秒、0 = 未活動）。
pub fn last_hook_activity_ms() -> u64 {
    LAST_HOOK_ACTIVITY.load(Ordering::Relaxed)
}

/// 現在時刻を `GetTickCount64` ミリ秒で返す。
pub fn current_tick_ms() -> u64 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

/// フックが応答しているかを判定する。
///
/// `timeout_ms` 以内にフックコールバックが呼ばれていれば `true`。
/// まだ一度も呼ばれていない場合も `true`（起動直後はキー入力がない）。
pub fn is_hook_responsive(timeout_ms: u64) -> bool {
    let last = LAST_HOOK_ACTIVITY.load(Ordering::Relaxed);
    if last == 0 {
        return true; // 起動直後: まだキー入力がない
    }
    let now = current_tick_ms();
    (now - last) < timeout_ms
}

/// フックの生存確認用 ping を送信する。
///
/// `INJECTED_MARKER` 付きの VK_NONAME (0xFC) KeyDown+KeyUp を SendInput で送信する。
/// フックが生きていればコールバックが呼ばれ、`LAST_HOOK_ACTIVITY` が更新される。
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
                LAST_HOOK_ACTIVITY.store(current_tick_ms(), Ordering::Relaxed);
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

/// グローバルなフックハンドル
static HOOK_HANDLE: SingleThreadCell<HHOOK> = SingleThreadCell::new(HHOOK(std::ptr::null_mut()));

/// フックコールバックで使うコールバック関数
static KEY_EVENT_CALLBACK: SingleThreadCell<Option<Box<dyn FnMut(RawKeyEvent) -> CallbackResult>>> =
    SingleThreadCell::new(None);

/// 再入ガード
static IN_CALLBACK: SingleThreadCell<bool> = SingleThreadCell::new(false);

/// Engine に送った KeyDown を記録するビットセット（VK コードは 0-255）。
/// KeyUp は KeyDown の判定に自動追随し、KeyDown/KeyUp ペアを構造的に保証する。
static SENT_TO_ENGINE: SingleThreadCell<[u64; 4]> = SingleThreadCell::new([0u64; 4]);

/// キーの処理先を決定する。
///
/// Engine に送るか、OS にそのまま渡すかを一元的に判定する。
/// KeyUp は KeyDown の判定に自動追随する（ペア保証）。
///
/// # Safety
/// `GetAsyncKeyState` を呼び出す。フックコールバック内から呼ぶこと。
unsafe fn classify_route(vk: u16, is_keydown: bool) -> KeyRoute {
    // KeyUp は KeyDown の判定に従う（ペア保証）
    if !is_keydown {
        let bits = SENT_TO_ENGINE.get_mut();
        let idx = (vk as usize) / 64;
        let bit = 1u64 << ((vk as usize) % 64);
        return if idx < 4 && (bits[idx] & bit) != 0 {
            KeyRoute::Engine
        } else {
            KeyRoute::Bypass
        };
    }

    // KeyDown の分類
    if is_non_shift_modifier(vk) {
        return KeyRoute::Bypass;
    }

    // Ctrl/Alt/Win が押されている間のキーはショートカット → Bypass
    // 例外: 親指キーは Engine のコンボキー用
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let ctrl = (GetAsyncKeyState(0x11).cast_unsigned() & 0x8000) != 0;
        let alt = (GetAsyncKeyState(0x12).cast_unsigned() & 0x8000) != 0;
        let win = (GetAsyncKeyState(0x5B).cast_unsigned() & 0x8000) != 0
            || (GetAsyncKeyState(0x5C).cast_unsigned() & 0x8000) != 0;
        if ctrl || alt || win {
            let left_thumb = LEFT_THUMB_VK.load(Ordering::Relaxed);
            let right_thumb = RIGHT_THUMB_VK.load(Ordering::Relaxed);
            if vk != left_thumb && vk != right_thumb {
                return KeyRoute::Bypass;
            }
        }
    }

    KeyRoute::Engine
}

/// classify_route の結果
enum KeyRoute {
    /// Engine で NICOLA 処理する
    Engine,
    /// OS にそのまま渡す
    Bypass,
}

/// SENT_TO_ENGINE ビットセットにキーを記録する
unsafe fn mark_sent_to_engine(vk: u16) {
    let bits = SENT_TO_ENGINE.get_mut();
    let idx = (vk as usize) / 64;
    let bit = 1u64 << ((vk as usize) % 64);
    if idx < 4 {
        bits[idx] |= bit;
    }
}

/// SENT_TO_ENGINE ビットセットからキーを削除する
unsafe fn clear_sent_to_engine(vk: u16) {
    let bits = SENT_TO_ENGINE.get_mut();
    let idx = (vk as usize) / 64;
    let bit = 1u64 << ((vk as usize) % 64);
    if idx < 4 {
        bits[idx] &= !bit;
    }
}

/// SENT_TO_ENGINE ビットセットを OS のキー状態と同期する。
///
/// `GetAsyncKeyState` で実際に押されていないのにビットが残っているキーをクリアする。
/// 500ms ポーリングで呼び出すことで、KeyUp 取りこぼしによるゴミを回収する。
///
/// # Safety
/// `GetAsyncKeyState` を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn sync_sent_to_engine() {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    let bits = SENT_TO_ENGINE.get_mut();
    let mut cleared = 0u32;
    for (idx, word) in bits.iter_mut().enumerate() {
        if *word == 0 {
            continue;
        }
        let mut remaining = *word;
        while remaining != 0 {
            let bit_pos = remaining.trailing_zeros() as usize;
            let vk = (idx * 64 + bit_pos) as i32;
            // OS でキーが離されている → ビットをクリア
            if (GetAsyncKeyState(vk).cast_unsigned() & 0x8000) == 0 {
                *word &= !(1u64 << bit_pos);
                cleared += 1;
            }
            remaining &= remaining - 1; // 最下位ビットを消す
        }
    }
    if cleared > 0 {
        log::debug!("sync_sent_to_engine: cleared {cleared} stale bit(s)");
    }
}

/// コールバックの戻り値
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
/// `SENT_TO_ENGINE` ビットセットで KeyDown/KeyUp ペアを構造的に保証する。
unsafe extern "system" fn hook_callback(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let hook_handle = *HOOK_HANDLE.get_mut();

    if ncode >= 0 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

        // ── ハートビート更新（ウォッチドッグ用）──
        LAST_HOOK_ACTIVITY.store(current_tick_ms(), Ordering::Relaxed);
        HOOK_EVENT_COUNT.fetch_add(1, Ordering::Relaxed);

        // ── 自己注入チェック（無限ループ防止）──
        if kb.dwExtraInfo == INJECTED_MARKER {
            return CallNextHookEx(hook_handle, ncode, wparam, lparam);
        }

        // ── かな入力方式バイパス ──
        if crate::IME_IS_KANA_INPUT.load(Ordering::Relaxed) {
            return CallNextHookEx(hook_handle, ncode, wparam, lparam);
        }

        let vk_raw = kb.vkCode as u16;
        let is_keydown = matches!(
            wparam.0 as u32,
            WM_KEYDOWN | WM_SYSKEYDOWN
        );

        // ── 一元的なルーティング判定 ──
        match classify_route(vk_raw, is_keydown) {
            KeyRoute::Bypass => {
                log::trace!(
                    "Hook: vk=0x{vk_raw:02X} {} → Bypass",
                    if is_keydown { "KeyDown" } else { "KeyUp" }
                );
                return CallNextHookEx(hook_handle, ncode, wparam, lparam);
            }
            KeyRoute::Engine => {
                // KeyDown/KeyUp ペア追跡を更新
                if is_keydown {
                    mark_sent_to_engine(vk_raw);
                } else {
                    clear_sent_to_engine(vk_raw);
                }
            }
        }

        // ── 再入ガード ──
        let in_callback = IN_CALLBACK.get_mut();
        if *in_callback {
            return CallNextHookEx(hook_handle, ncode, wparam, lparam);
        }
        *in_callback = true;

        let event_type = if is_keydown {
            KeyEventType::KeyDown
        } else {
            KeyEventType::KeyUp
        };

        let vk = VkCode(vk_raw);
        let scan = ScanCode(kb.scanCode);
        let (key_classification, physical_pos) = classify_key(vk, scan);
        let event = RawKeyEvent {
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

        log::trace!(
            "Hook: vk=0x{:02X} scan=0x{:04X} type={:?} → Engine",
            event.vk_code.0,
            event.scan_code.0,
            event.event_type
        );

        // ── Engine コールバック呼び出し ──
        let result = KEY_EVENT_CALLBACK
            .get_mut()
            .as_mut()
            .map_or(CallbackResult::PassThrough, |callback| callback(event));

        *IN_CALLBACK.get_mut() = false;

        match result {
            CallbackResult::Consumed => {
                return LRESULT(1);
            }
            CallbackResult::PassThrough => {}
        }
    }

    CallNextHookEx(hook_handle, ncode, wparam, lparam)
}

/// 起動時点からの経過マイクロ秒を返す（`Instant` を内部的に使用）
fn now_timestamp() -> Timestamp {
    use std::sync::OnceLock;
    use std::time::Instant;
    static BASELINE: OnceLock<Instant> = OnceLock::new();
    let baseline = BASELINE.get_or_init(Instant::now);
    baseline.elapsed().as_micros() as u64
}
