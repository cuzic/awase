use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT,
    WM_SYSKEYDOWN,
};

use crate::output::INJECTED_MARKER;

/// Alt 物理押下中または WM_SYSKEYDOWN コンテキスト（メニューモード）を示すフラグ
const LLKHF_ALTDOWN: u32 = 0x20;
use crate::scanmap::scan_to_pos;
use crate::HookConfig;
use awase::scanmap::PhysicalPos;
use awase::types::{
    ImeRelevance, KeyClassification, KeyEventType, RawKeyEvent, ScanCode, ShadowImeAction,
    Timestamp, VkCode,
};

/// Windows VK + ScanCode からキー分類と物理位置を生成する
#[must_use]
pub fn classify_key(
    vk: VkCode,
    scan: ScanCode,
    config: &HookConfig,
) -> (KeyClassification, Option<PhysicalPos>) {
    use crate::vk::VkCodeExt;

    let left_thumb = config.left_thumb_vk;
    let right_thumb = config.right_thumb_vk;

    if vk == left_thumb {
        (KeyClassification::LeftThumb, None)
    } else if vk == right_thumb {
        (KeyClassification::RightThumb, None)
    } else if vk.is_passthrough() {
        (KeyClassification::Passthrough, None)
    } else if let Some(pos) = scan_to_pos(scan) {
        (KeyClassification::Char, Some(pos))
    } else {
        (KeyClassification::Passthrough, None)
    }
}

/// Windows VK コードから IME 関連の事前分類情報を生成する
#[must_use]
pub fn classify_ime_relevance(vk: VkCode) -> ImeRelevance {
    use crate::vk::{self, VkCodeExt};

    let ime_key = vk.ime_kind();
    let shadow_action = ime_key.map(|k| match k.shadow_effect() {
        vk::ShadowImeEffect::TurnOn => ShadowImeAction::TurnOn,
        vk::ShadowImeEffect::TurnOff => ShadowImeAction::TurnOff,
        vk::ShadowImeEffect::Toggle => ShadowImeAction::Toggle,
    });

    // Note: is_sync_key and sync_direction are set later by the runtime
    // when it has access to the config. This function only classifies
    // hardware-level IME properties.
    ImeRelevance {
        may_change_ime: ime_key.is_some() || vk.may_change_ime(),
        shadow_action,
        is_sync_key: false,   // set by runtime with config
        sync_direction: None, // set by runtime with config
        is_ime_control: vk.is_ime_control(),
    }
}

/// RUNTIME 借用なしで classify_key を呼ぶために親指 VK を AtomicU32 にキャッシュする。
/// 上位 16bit = left_thumb_vk、下位 16bit = right_thumb_vk。
static CACHED_THUMB_VKS: AtomicU32 = AtomicU32::new(0);

/// フックコールバックの最終活動タイムスタンプ（ウォッチドッグ用、クロススレッド対応）
///
/// 自己注入キー含む全コールバックで更新する。エンジンスレッドの watchdog がここを読む。
pub static HOOK_ALIVE_TICK_MS: AtomicU64 = AtomicU64::new(0);

/// install_hook がフックスレッドからの TID 通知を待つスロット
/// 0 = 待機中、u32::MAX = SetWindowsHookExW 失敗、それ以外 = フックスレッド TID
static HOOK_TID_INIT_SLOT: AtomicU32 = AtomicU32::new(0);

/// VK ごとの物理押下状態。non-self-injected な KeyDown/KeyUp で更新する。
///
/// 用途: `send_vk_pair` が合成 `LSHIFT↑` を送ったあと、OS state を物理状態に
/// 再同期するために物理 Shift が押下中か判定する。`GetAsyncKeyState` は
/// SendInput の影響も受けるため、物理状態の判定には使えない。
static PHYSICAL_KEY_STATE: [AtomicBool; 256] = [const { AtomicBool::new(false) }; 256];

/// VK ごとの物理 KeyDown 時刻（`current_tick_ms` 値）。0 = 押下されていない。
///
/// 用途: 「Shift をどれくらい長く押しているか」で再注入の要否を判断する。
/// 短押し（例: 200ms 未満）では Ctrl+I 直後の無変換 で IME-OFF 誤発火を
/// 避けるため修飾解放を生かし、長押しでのみ OS state を物理状態に再同期する。
static PHYSICAL_KEY_DOWN_AT_MS: [AtomicU64; 256] = [const { AtomicU64::new(0) }; 256];

/// 物理 VK が押下中かを返す。SendInput では更新されないため信頼できる物理状態。
#[must_use]
pub fn is_physical_key_down(vk: VkCode) -> bool {
    PHYSICAL_KEY_STATE
        .get(vk.0 as usize)
        .is_some_and(|s| s.load(Ordering::Relaxed))
}

/// 物理 VK の押下経過時間（ms）。押下されていなければ `None`。
#[must_use]
pub fn physical_key_held_ms(vk: VkCode) -> Option<u64> {
    let down_at = PHYSICAL_KEY_DOWN_AT_MS
        .get(vk.0 as usize)?
        .load(Ordering::Relaxed);
    (down_at != 0).then(|| current_tick_ms().saturating_sub(down_at))
}

/// 直近の物理 Ctrl 押下後に他の VK の KeyDown を 1 つでも観測したか。
///
/// 用途: `Ctrl↓ → I↓ I↑ → 無変換↓` のような「Ctrl が既に他キーで consume された」
/// パターンを検知し、無変換↓ で Ctrl+無変換 IME-OFF を即発火せず 50ms 救済窓を設けるため。
/// 「Ctrl↓ → 直後に 無変換↓」の意図的チョードでは false のままなので、即時 IME-OFF できる。
///
/// Ctrl↓/Ctrl↑ で false にリセットされる。
static CTRL_CONSUMED_SINCE_DOWN: AtomicBool = AtomicBool::new(false);

/// 直近の物理 Ctrl 押下以降に他の VK KeyDown を観測したか返す。
#[must_use]
pub fn ctrl_consumed_since_down() -> bool {
    CTRL_CONSUMED_SINCE_DOWN.load(Ordering::Relaxed)
}

fn cached_hook_config() -> HookConfig {
    let packed = CACHED_THUMB_VKS.load(Ordering::Acquire);
    HookConfig {
        left_thumb_vk: VkCode((packed >> 16) as u16),
        right_thumb_vk: VkCode(packed as u16),
    }
}

/// 親指キー VK コードを設定する（config 読み込み後に呼ぶ）
pub fn set_thumb_vk_codes(config: &mut HookConfig, left: VkCode, right: VkCode) {
    config.left_thumb_vk = left;
    config.right_thumb_vk = right;
    CACHED_THUMB_VKS.store(
        (u32::from(left.0) << 16) | u32::from(right.0),
        Ordering::Release,
    );
}

/// 現在時刻を `GetTickCount64` ミリ秒で返す。
#[must_use]
pub fn current_tick_ms() -> u64 {
    // SAFETY: GetTickCount64 はどのスレッドからも安全に呼び出せるスレッドセーフな Win32 API。
    //         引数なし・副作用なし・内部ロックにより安全性が保証される。
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
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
/// ドロップ時にフックスレッドへ WM_QUIT を送信し、
/// スレッド終了（および UnhookWindowsHookEx）を待機する。
pub struct HookGuard {
    hook_thread_id: u32,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for HookGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookGuard")
            .field("hook_thread_id", &self.hook_thread_id)
            .finish_non_exhaustive()
    }
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        // フックスレッドに WM_QUIT を送り、GetMessageW ループを終了させる。
        // フックスレッド側で UnhookWindowsHookEx を実行してから終了する。
        // SAFETY: hook_thread_id はフックスレッドの有効な TID。
        unsafe {
            let _ = PostThreadMessageW(self.hook_thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        log::info!("Keyboard hook uninstalled");
    }
}

/// フックを専用スレッドに登録する。
///
/// スポーンした "awase-hook" スレッドが `SetWindowsHookExW` を完了するまで
/// スピン待機してから返る。返された `HookGuard` を保持している間フックが有効。
/// ドロップ時にフックスレッドを終了させる。
///
/// # Errors
/// スレッドのスポーン失敗、または `SetWindowsHookExW` が失敗した場合。
pub fn install_hook() -> windows::core::Result<HookGuard> {
    // 多重呼び出し対策: スロットをリセット
    HOOK_TID_INIT_SLOT.store(0, Ordering::SeqCst);

    let thread = std::thread::Builder::new()
        .name("awase-hook".into())
        .spawn(|| {
            let hook_result =
                unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_callback), None, 0) };
            match hook_result {
                Ok(hook) => {
                    // SAFETY: HOOK_HANDLE はこのスレッドのみがアクセスする。
                    unsafe {
                        HOOK_HANDLE.set(hook);
                    }
                    let tid = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
                    HOOK_TID_INIT_SLOT.store(tid, Ordering::Release);

                    // 軽量メッセージポンプ（WH_KEYBOARD_LL フック用）
                    let mut msg = MSG::default();
                    loop {
                        // SAFETY: msg は有効なスタック上の MSG。
                        let ret = unsafe { GetMessageW(&raw mut msg, None, 0, 0) };
                        if ret.0 <= 0 {
                            break;
                        }
                        // SAFETY: msg は GetMessageW が充填した有効な値。
                        unsafe {
                            DispatchMessageW(&raw const msg);
                        }
                    }

                    // ループ終了（WM_QUIT 受信）: フックを解除
                    // SAFETY: HOOK_HANDLE はこのスレッドのみがアクセスする。
                    unsafe {
                        let h = *HOOK_HANDLE.get_mut();
                        if !h.0.is_null() {
                            let _ = UnhookWindowsHookEx(h);
                            HOOK_HANDLE.set(HHOOK(std::ptr::null_mut()));
                        }
                    }
                    log::info!("Keyboard hook thread exiting cleanly");
                }
                Err(e) => {
                    log::error!("SetWindowsHookExW failed in hook thread: {e}");
                    // u32::MAX でエラーを通知
                    HOOK_TID_INIT_SLOT.store(u32::MAX, Ordering::Release);
                }
            }
        })
        .map_err(|e| {
            log::error!("Failed to spawn awase-hook thread: {e}");
            windows::core::Error::from_thread()
        })?;

    // フックスレッドが SetWindowsHookExW を完了するまでスピン待機
    let hook_tid = loop {
        let t = HOOK_TID_INIT_SLOT.load(Ordering::Acquire);
        if t != 0 {
            break t;
        }
        std::hint::spin_loop();
    };

    if hook_tid == u32::MAX {
        // SetWindowsHookExW がフックスレッド内で失敗
        let _ = thread.join();
        return Err(windows::core::Error::from_thread());
    }

    log::info!("Keyboard hook installed in dedicated thread (tid={hook_tid})");
    Ok(HookGuard {
        hook_thread_id: hook_tid,
        thread: Some(thread),
    })
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
    use crate::vk::VkCodeExt;
    RawKeyEvent {
        vk_code: vk,
        scan_code: scan,
        event_type: if is_keydown {
            KeyEventType::KeyDown
        } else {
            KeyEventType::KeyUp
        },
        extra_info,
        timestamp: now_timestamp(),
        key_classification,
        physical_pos,
        ime_relevance: classify_ime_relevance(vk),
        modifier_key: vk.classify_modifier(),
        modifier_snapshot,
    }
}

/// 自己注入キーかどうかを判定する（無限ループ防止）。
const fn is_self_injected(extra_info: usize) -> bool {
    extra_info == INJECTED_MARKER
        || extra_info == crate::tsf::output::TSF_MARKER
        || extra_info == crate::tsf::output::IME_KANJI_MARKER
}

/// WH_KEYBOARD_LL フックコールバック（専用フックスレッド上で動作）
///
/// 全ての物理キーを消費し `PostThreadMessageW` でエンジンスレッドに転送する。
/// 自己注入キー（INJECTED_MARKER 等）は `CallNextHookEx` で OS に通す。
/// RUNTIME には一切触れないため、再入バグが構造的に発生しない。
///
/// # Safety
/// OS から `WH_KEYBOARD_LL` フックコールバックとして呼び出される。
/// フックスレッドの GetMessageW ループ内でのみ呼ばれる。
unsafe extern "system" fn hook_callback(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // ウォッチドッグ用タイムスタンプを更新（自己注入キーも含む全コールバック）
    HOOK_ALIVE_TICK_MS.store(current_tick_ms(), Ordering::Relaxed);

    let hook_handle = *HOOK_HANDLE.get_mut();
    if ncode < 0 {
        return CallNextHookEx(Some(hook_handle), ncode, wparam, lparam);
    }

    let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

    // 自己注入キー（SendInput with INJECTED_MARKER 等）は OS にそのまま通す
    if is_self_injected(kb.dwExtraInfo) {
        return CallNextHookEx(Some(hook_handle), ncode, wparam, lparam);
    }

    let vk = VkCode(kb.vkCode as u16);
    let scan = ScanCode(kb.scanCode);
    let is_keydown = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
    if let Some(slot) = PHYSICAL_KEY_STATE.get(vk.0 as usize) {
        slot.store(is_keydown, Ordering::Relaxed);
    }
    if let Some(slot) = PHYSICAL_KEY_DOWN_AT_MS.get(vk.0 as usize) {
        // 同一 VK の auto-repeat KeyDown では down_at を上書きしない
        // （長押し判定が常に「直前」へリセットされてしまうため）。
        let new_value = if is_keydown {
            let prev = slot.load(Ordering::Relaxed);
            if prev == 0 {
                current_tick_ms()
            } else {
                prev
            }
        } else {
            0
        };
        slot.store(new_value, Ordering::Relaxed);
    }
    // CTRL_CONSUMED チェックと classify_key で共用するため先に取得する。
    let config = cached_hook_config();
    // Ctrl consumption tracking
    if crate::vk::is_ctrl_variant(vk) {
        // Ctrl↓/Ctrl↑ どちらでも consumption をリセット（次の Ctrl 押下から再計測）
        CTRL_CONSUMED_SINCE_DOWN.store(false, Ordering::Relaxed);
    } else if is_keydown {
        let ctrl_held = is_physical_key_down(crate::vk::VK_LCONTROL)
            || is_physical_key_down(crate::vk::VK_RCONTROL);
        if ctrl_held {
            // 親指キー自身は "Ctrl consumed" に含めない。
            // Ctrl+無変換 を直接押したとき(他キーなし) rescue が誤発動しないようにするため。
            if vk != config.left_thumb_vk && vk != config.right_thumb_vk {
                CTRL_CONSUMED_SINCE_DOWN.store(true, Ordering::Relaxed);
            }
        }
    }
    let (key_classification, physical_pos) = classify_key(vk, scan, &config);
    // SAFETY: GetAsyncKeyState はスレッドセーフで任意のスレッドから呼べる。
    let mut modifier_snapshot = crate::observer::focus_observer::read_os_modifiers();
    // Alt 物理押下中またはメニューモード（WM_SYSKEYDOWN コンテキスト）のキーは変換しない
    if kb.flags.0 & LLKHF_ALTDOWN != 0 {
        modifier_snapshot.alt = true;
    }
    let event = build_raw_key_event(
        vk,
        scan,
        is_keydown,
        kb.dwExtraInfo,
        key_classification,
        physical_pos,
        modifier_snapshot,
    );

    let engine_tid = crate::ENGINE_THREAD_ID.load(Ordering::Relaxed);
    if engine_tid != 0 {
        let ptr = Box::into_raw(Box::new(event));
        // SAFETY: engine_tid は run_message_loop 先頭で設定された有効なスレッド TID。
        if PostThreadMessageW(
            engine_tid,
            crate::WM_KEY_FROM_HOOK,
            WPARAM(0),
            LPARAM(ptr as isize),
        )
        .is_err()
        {
            // PostThreadMessageW 失敗（キュー満杯等）: メモリリークを防ぐため即座に回収
            let _ = Box::from_raw(ptr);
            log::warn!("[hook] PostThreadMessageW failed vk={vk:#04X}");
        }
    }
    LRESULT(1) // 常に消費（engine thread が PassThrough 判定して reinject する）
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
