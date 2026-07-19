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
/// SendInput / keybd_event 等で注入されたイベントを示すフラグ
const LLKHF_INJECTED: u32 = 0x10;
/// 拡張キー（Right Ctrl/Right Alt・矢印キー等）を示すフラグ。
///
/// `KBDLLHOOKSTRUCT.vkCode` は環境によって Ctrl/Alt を左右区別済みの
/// VK_LMENU/VK_RMENU (0xA4/0xA5) ではなく汎用の VK_MENU (0x12) で届けることがある
/// （`vk.rs` の `classify_modifier`/`is_ctrl_variant` が汎用形・左右specific形の
/// 両方を防御的にマッチしているのはこのため）。汎用形で届いた場合、この拡張キー
/// フラグで Left/Right を判別する（Right Alt/Right Ctrl は拡張キー、Left 側は非拡張）。
const LLKHF_EXTENDED: u32 = 0x01;
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
    } else if let Some(pos) = scan_to_pos(config.keyboard_model, scan) {
        (KeyClassification::Char, Some(pos))
    } else {
        (KeyClassification::Passthrough, None)
    }
}

/// Alt キー1個ぶんの「なりすまし」判定（純粋関数、テスト対象）。
///
/// 新規押下（`was_down=false` の `KeyDown`）時点でのみ `engine_enabled` を見て
/// 判定し直す。auto-repeat の `KeyDown`（`was_down=true`）や `KeyUp` は、直前の
/// 新規押下時点の判定（`was_impersonating`）をそのまま使う。これにより、同一の
/// 押しっぱなしセッション中に設定変更やエンジン ON/OFF 切替が起きても、途中で
/// 判定がズレて Alt が stuck modifier になることを防ぐ。
///
/// 戻り値: `(書き換え後の vk, 次に保持すべき is_impersonating 状態)`
#[must_use]
fn decide_alt_impersonation(
    original_vk: VkCode,
    thumb_vk: VkCode,
    is_keydown: bool,
    was_down: bool,
    was_impersonating: bool,
    engine_enabled: bool,
) -> (VkCode, bool) {
    let is_fresh_press = is_keydown && !was_down;
    let impersonating = if is_fresh_press {
        engine_enabled
    } else {
        was_impersonating
    };
    let vk = if impersonating { thumb_vk } else { original_vk };
    (vk, impersonating)
}

/// `left_thumb_key`/`right_thumb_key` 設定文字列を VkCode に解決し、
/// Alt なりすましが必要かどうかも同時に判定する。
///
/// `"Left Alt"`/`"Right Alt"`（`awase-settings` の `THUMB_KEY_OPTIONS` 参照）は
/// 物理キー名ではなく「エンジン ON 時のみ Left/Right Alt を親指キーとして扱う」
/// という特殊な指示であり、通常の VK 名パーサー（`VkCode::from_name`）には
/// 含めない。なりすまし先の VK は JIS の無変換(左)/変換(右)相当に固定する
/// （`config.rs` の `GeneralConfig::keyboard_model` doc 参照）。
///
/// 戻り値: `(親指キーとして使う VkCode, Alt なりすましを有効にするか)`。
/// 未知のキー名の場合は `None`。
#[must_use]
pub fn resolve_thumb_key(name: &str) -> Option<(VkCode, bool)> {
    use crate::vk::{VkCodeExt, VK_CONVERT, VK_NONCONVERT};
    match name {
        "Left Alt" => Some((VK_NONCONVERT, true)),
        "Right Alt" => Some((VK_CONVERT, true)),
        _ => VkCode::from_name(name).map(|vk| (vk, false)),
    }
}

/// `vk`/`extended` から、この物理キーが Left Alt か Right Alt かを判定する。
///
/// `vk` が既に区別済みの VK_LMENU/VK_RMENU ならそのまま使う。汎用の VK_MENU
/// (0x12) で届いた場合は `extended`（`KBDLLHOOKSTRUCT.flags` の
/// `LLKHF_EXTENDED`）で判別する（Right Alt は拡張キー）。
#[must_use]
const fn classify_alt_side(vk: VkCode, extended: bool) -> (bool, bool) {
    let is_left = vk.0 == crate::vk::VK_LMENU.0 || (vk.0 == crate::vk::VK_MENU.0 && !extended);
    let is_right = vk.0 == crate::vk::VK_RMENU.0 || (vk.0 == crate::vk::VK_MENU.0 && extended);
    (is_left, is_right)
}

/// Left/Right Alt キーのなりすまし処理（グローバル状態の読み書きを伴う副作用あり）。
/// 判定ロジック本体は `decide_alt_impersonation`（純粋関数）に委譲する。
///
/// `vk` が Left/Right Alt でない場合、または対応する設定が OFF の場合は
/// `vk` をそのまま返す。`extended` は `classify_alt_side` 参照。
#[must_use]
fn apply_alt_impersonation(vk: VkCode, is_keydown: bool, extended: bool, config: HookConfig) -> VkCode {
    let (is_left_alt, is_right_alt) = classify_alt_side(vk, extended);
    if config.left_alt_impersonates_thumb_key && is_left_alt {
        let engine_enabled = CACHED_ENGINE_ENABLED.load(Ordering::Relaxed);
        let was_down = ALT_L_WAS_DOWN.load(Ordering::Relaxed);
        let was_impersonating = ALT_L_IMPERSONATING.load(Ordering::Relaxed);
        let (new_vk, impersonating) = decide_alt_impersonation(
            vk,
            config.left_thumb_vk,
            is_keydown,
            was_down,
            was_impersonating,
            engine_enabled,
        );
        ALT_L_IMPERSONATING.store(impersonating, Ordering::Relaxed);
        ALT_L_WAS_DOWN.store(is_keydown, Ordering::Relaxed);
        new_vk
    } else if config.right_alt_impersonates_thumb_key && is_right_alt {
        let engine_enabled = CACHED_ENGINE_ENABLED.load(Ordering::Relaxed);
        let was_down = ALT_R_WAS_DOWN.load(Ordering::Relaxed);
        let was_impersonating = ALT_R_IMPERSONATING.load(Ordering::Relaxed);
        let (new_vk, impersonating) = decide_alt_impersonation(
            vk,
            config.right_thumb_vk,
            is_keydown,
            was_down,
            was_impersonating,
            engine_enabled,
        );
        ALT_R_IMPERSONATING.store(impersonating, Ordering::Relaxed);
        ALT_R_WAS_DOWN.store(is_keydown, Ordering::Relaxed);
        new_vk
    } else {
        vk
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
static HOOK_ALIVE_TICK_MS: AtomicU64 = AtomicU64::new(0);

/// フックコールバックの活動タイムスタンプを現在時刻で更新する
pub(crate) fn tick_hook_alive() {
    HOOK_ALIVE_TICK_MS.store(current_tick_ms(), Ordering::Relaxed);
}

/// フックコールバックの最終活動タイムスタンプ（ms）を返す
pub fn hook_alive_tick_ms() -> u64 {
    HOOK_ALIVE_TICK_MS.load(Ordering::Relaxed)
}

/// install_hook がフックスレッドからの TID 通知を待つスロット
/// 0 = 待機中、u32::MAX = SetWindowsHookExW 失敗、それ以外 = フックスレッド TID
static HOOK_TID_INIT_SLOT: AtomicU32 = AtomicU32::new(0);

fn hook_tid_reset() {
    HOOK_TID_INIT_SLOT.store(0, Ordering::SeqCst);
}
fn hook_tid_set(tid: u32) {
    HOOK_TID_INIT_SLOT.store(tid, Ordering::Release);
}
fn hook_tid_fail() {
    HOOK_TID_INIT_SLOT.store(u32::MAX, Ordering::Release);
}
fn hook_tid_poll() -> u32 {
    HOOK_TID_INIT_SLOT.load(Ordering::Acquire)
}

/// VK ごとの物理押下状態。non-self-injected な KeyDown/KeyUp で更新する。
///
/// 用途: `send_vk_pair` が合成 `LSHIFT↑` を送ったあと、OS state を物理状態に
/// 再同期するために物理 Shift が押下中か判定する。`GetAsyncKeyState` は
/// SendInput の影響も受けるため、物理状態の判定には使えない。
static PHYSICAL_KEY_STATE: [AtomicBool; 256] = [const { AtomicBool::new(false) }; 256];

/// VK ごとの物理 KeyDown 時刻（`current_tick_ms` 値）。0 = 押下されていない。
///
/// 用途: 「Shift をどれくらい長く押しているか」で再注入の要否を判断する。
/// 短押し（例: 200ms 未満）では Ctrl+I 直後の無変換 で IME OFF 誤発火を
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

/// `PHYSICAL_KEY_STATE` / `PHYSICAL_KEY_DOWN_AT_MS` を全 VK ぶん強制的に「離した」状態へ戻す。
///
/// セッションロック中（Secure Desktop 遷移中）は `WH_KEYBOARD_LL` フックにイベントが
/// 一切届かないため、ロックの瞬間に押されていた物理キーの KeyUp が失われ得る。
/// `PHYSICAL_KEY_STATE` は OR 演算で左右を合成する（`observer::focus_observer::read_os_modifiers`）
/// ため、片側が stuck するだけで `mods.shift`/`mods.ctrl` が恒久的に `true` になる
/// （2026-07-09 実機で確認、右 Shift の KeyUp 消失が原因）。
///
/// アンロック時点では OS 側の実際の物理キーはどれも「離されている」と仮定してよい
/// （ロック中ずっと押しっぱなしということはまず無い）ため、全スロットを無条件でクリアする。
///
/// `panic_reset()`（`send_all_modifier_key_ups()` は自己注入 SendInput のため
/// `is_self_injected` フィルタで弾かれ `PHYSICAL_KEY_STATE` を更新できない、ADR-054 由来の
/// 隙間）と `WM_WTSSESSION_CHANGE` の `WTS_SESSION_UNLOCK` から呼ぶ。
pub fn reset_physical_key_state() {
    for slot in &PHYSICAL_KEY_STATE {
        slot.store(false, Ordering::Relaxed);
    }
    for slot in &PHYSICAL_KEY_DOWN_AT_MS {
        slot.store(0, Ordering::Relaxed);
    }
    log::info!("[hook] PHYSICAL_KEY_STATE をリセット（全 VK を解放状態に）");
}

/// 直近の物理 Ctrl 押下後に他の VK の KeyDown を 1 つでも観測したか。
///
/// 用途: `Ctrl↓ → I↓ I↑ → 無変換↓` のような「Ctrl が既に他キーで consume された」
/// パターンを検知し、無変換↓ で Ctrl+無変換 IME OFF を即発火せず 50ms 救済窓を設けるため。
/// 「Ctrl↓ → 直後に 無変換↓」の意図的チョードでは false のままなので、即時 IME OFF できる。
///
/// Ctrl↓/Ctrl↑ で false にリセットされる。
static CTRL_CONSUMED_SINCE_DOWN: AtomicBool = AtomicBool::new(false);

/// 直近の物理 Ctrl 押下以降に他の VK KeyDown を観測したか返す。
#[must_use]
pub fn ctrl_consumed_since_down() -> bool {
    CTRL_CONSUMED_SINCE_DOWN.load(Ordering::Relaxed)
}

/// キーボードモデル（JIS/US）のキャッシュ。RUNTIME 借用なしで `classify_key` から
/// 参照するため `CACHED_THUMB_VKS` と同じ理由でグローバル AtomicBool にキャッシュする。
/// false = Jis（既定）、true = Us。
static CACHED_KEYBOARD_MODEL_IS_US: AtomicBool = AtomicBool::new(false);

/// Alt なりすまし ON/OFF のキャッシュ。`resolve_thumb_key` が
/// `left_thumb_key`/`right_thumb_key` の値（`"Left Alt"`/`"Right Alt"` か否か）
/// から導出した結果を保持する。左右は独立（片方だけの構成もあり得るため）。
static CACHED_LEFT_ALT_IMPERSONATION_ENABLED: AtomicBool = AtomicBool::new(false);
static CACHED_RIGHT_ALT_IMPERSONATION_ENABLED: AtomicBool = AtomicBool::new(false);

/// エンジンの実効有効状態（`UiEffect::EngineStateChanged` の `enabled` と同じ値）の
/// キャッシュ。Alt なりすましの発動条件に使う（`hook_callback` 参照）。
static CACHED_ENGINE_ENABLED: AtomicBool = AtomicBool::new(false);

/// 直近の Left/Right Alt「新規押下」時点で「なりすまし発動中」だったか。
///
/// 新規押下（離された状態からの KeyDown）時点の判定を、以降の auto-repeat
/// KeyDown・KeyUp まで保持するために使う。押しっぱなし中に
/// `left_thumb_key`/`right_thumb_key` の設定変更やエンジン ON/OFF 切替が
/// 起きても、同一の押下セッション内では KeyDown（repeat 含む）/
/// KeyUp が同じ扱い（なりすまし継続 or 通常 Alt 継続）になり、途中で判定がズレて
/// Alt が stuck modifier になる事故を防ぐ（`PHYSICAL_KEY_DOWN_AT_MS` の
/// auto-repeat 対策コメント参照、同種の問題）。
static ALT_L_IMPERSONATING: AtomicBool = AtomicBool::new(false);
static ALT_R_IMPERSONATING: AtomicBool = AtomicBool::new(false);

/// Left/Right Alt が直前のイベント時点で物理的に押下中だったか。
/// KeyDown が「新規押下」か「auto-repeat」かを区別するために使う。
static ALT_L_WAS_DOWN: AtomicBool = AtomicBool::new(false);
static ALT_R_WAS_DOWN: AtomicBool = AtomicBool::new(false);

fn cached_hook_config() -> HookConfig {
    let packed = CACHED_THUMB_VKS.load(Ordering::Acquire);
    let keyboard_model = if CACHED_KEYBOARD_MODEL_IS_US.load(Ordering::Acquire) {
        awase::scanmap::KeyboardModel::Us
    } else {
        awase::scanmap::KeyboardModel::Jis
    };
    HookConfig {
        left_thumb_vk: VkCode((packed >> 16) as u16),
        right_thumb_vk: VkCode(packed as u16),
        keyboard_model,
        left_alt_impersonates_thumb_key: CACHED_LEFT_ALT_IMPERSONATION_ENABLED
            .load(Ordering::Acquire),
        right_alt_impersonates_thumb_key: CACHED_RIGHT_ALT_IMPERSONATION_ENABLED
            .load(Ordering::Acquire),
    }
}

/// 親指キー VK コードを設定する（config 読み込み後に呼ぶ）
pub fn set_thumb_vk_codes(left: VkCode, right: VkCode) {
    CACHED_THUMB_VKS.store(
        (u32::from(left.0) << 16) | u32::from(right.0),
        Ordering::Release,
    );
}

/// キーボードモデル（JIS/US）を設定する（config 読み込み後に呼ぶ）
pub fn set_keyboard_model(model: awase::scanmap::KeyboardModel) {
    CACHED_KEYBOARD_MODEL_IS_US.store(
        model == awase::scanmap::KeyboardModel::Us,
        Ordering::Release,
    );
}

/// Alt なりすましの ON/OFF を設定する（config 読み込み後に呼ぶ）。左右は独立。
pub fn set_alt_impersonation_enabled(left: bool, right: bool) {
    CACHED_LEFT_ALT_IMPERSONATION_ENABLED.store(left, Ordering::Release);
    CACHED_RIGHT_ALT_IMPERSONATION_ENABLED.store(right, Ordering::Release);
}

/// エンジンの実効有効状態を設定する（`UiEffect::EngineStateChanged` 処理箇所から呼ぶ）。
/// Alt なりすましの発動条件（エンジン ON 時のみ発動）に使う。
pub fn set_engine_enabled(enabled: bool) {
    CACHED_ENGINE_ENABLED.store(enabled, Ordering::Release);
}

/// Alt なりすましが現在発動中か（Left/Right いずれか）。
///
/// `InputContext::modifiers`/`RawKeyEvent::modifier_snapshot` を構築する全ての
/// 箇所（`hook.rs` 自身・`runtime/mod.rs::build_ctx`・
/// `runtime/message_handlers.rs` のタイマーハンドラ）で、この値が `true` の間は
/// `modifiers.alt` を強制的に `false` にすること。
///
/// 背景（2026-07-19 実機で発覚）: `apply_alt_impersonation` で vk を書き換えても、
/// `crate::observer::focus_observer::read_os_modifiers()` は `GetAsyncKeyState` で
/// 「本物の Alt が物理的に押されているか」を vk と無関係に直接読むため、
/// なりすまし中も `modifiers.alt` は true のままになる。core engine の
/// `bypass_reason()` は `ev.key_class`（vk 由来、なりすまし後は正しく LeftThumb 等に
/// 分類される）とは**別に** `self.phys.modifiers.is_os_modifier_held()`
/// （ctrl||alt||win）を見て無条件に bypass するため、vk の書き換えだけでは
/// 常に `BypassReason::OsModifierHeld` でチョード判定に一切入らず素通しされ、
/// 「ローマ字入力のような挙動になる」不具合の直接原因になっていた。
#[must_use]
pub fn is_alt_impersonation_active() -> bool {
    ALT_L_IMPERSONATING.load(Ordering::Relaxed) || ALT_R_IMPERSONATING.load(Ordering::Relaxed)
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
    hook_tid_reset();

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
                    hook_tid_set(tid);

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
                    hook_tid_fail();
                }
            }
        })
        .map_err(|e| {
            log::error!("Failed to spawn awase-hook thread: {e}");
            windows::core::Error::from_thread()
        })?;

    // フックスレッドが SetWindowsHookExW を完了するまでスピン待機
    let hook_tid = loop {
        let t = hook_tid_poll();
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
    injected: bool,
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
        injected,
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
#[expect(clippy::cognitive_complexity)]
unsafe extern "system" fn hook_callback(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // ウォッチドッグ用タイムスタンプを更新（自己注入キーも含む全コールバック）
    tick_hook_alive();

    let hook_handle = *HOOK_HANDLE.get_mut();
    if ncode < 0 {
        return CallNextHookEx(Some(hook_handle), ncode, wparam, lparam);
    }

    let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

    let mut vk = VkCode(kb.vkCode as u16);
    let scan = ScanCode(kb.scanCode);
    let is_keydown = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
    let self_injected = is_self_injected(kb.dwExtraInfo);

    let is_injected = (kb.flags.0 & LLKHF_INJECTED) != 0;

    // IME モードキー (VK_KANA/IME_ON/JUNJA/KANJI/IME_OFF/VK_DBE_*) 診断ログ。
    // 「Ctrl+無変換→Ctrl+変換 で IME-OFF Engine-ON になる」報告 (2026-07-06) の切り分け用:
    // 無変換キー (VK_DBE_ALPHANUMERIC=0xF0) の KeyDown が [engine-input] に一度も
    // 現れず KeyUp だけ現れる現象が2回連続で観測された。自己注入として swallow
    // されているのか、そもそもフックに届いていないのかをここで区別する。
    // injected (LLKHF_INJECTED) は BUG-08/BUG-14 の注入元切り分けに必須（BUG-08 発生時は
    // 未記録で特定できなかった）。
    let ime_key_kind = crate::vk::ImeKeyKind::from_vk(vk);
    if ime_key_kind.is_some() {
        let dir = if is_keydown { "down" } else { "up" };
        log::debug!(
            "[hook] IME-mode vk=0x{:02X} {dir} self_injected={self_injected} injected={is_injected} scan=0x{:X} extra=0x{:X}",
            vk.0, kb.scanCode, kb.dwExtraInfo,
        );
    }

    // 自己注入キー（SendInput with INJECTED_MARKER 等）は OS にそのまま通す
    if self_injected {
        return CallNextHookEx(Some(hook_handle), ncode, wparam, lparam);
    }

    // BUG-14 追記 (2026-07-06): ここにあった「foreign-injected IME モードキー全般の
    // swallow」は撤回した。MS-IME × Windows Terminal 実機で、導入直後から一切入力
    // できなくなったため（1 打鍵ごとに foreign-injected VK_KANA down+up ペアが到達
    // = MS-IME 自身の機能的なキー注入で、これを遮断すると IME のモード遷移/かな修飾
    // が壊れる）。foreign-injected IME モードキーは「観測」であって「ユーザー意図」
    // でも「ノイズ」でもない — 遮断ではなく shadow toggle 側で意図として扱わない
    // 方向で対処する（docs/known-bugs.md BUG-14）。VK_KANA のみ従来の BUG-08 swallow
    // を維持する（下のブロック）。
    //
    // PHYSICAL_KEY_STATE はハードウェア由来のイベントのみで更新する。
    // LLKHF_INJECTED 付き（X サーバー・他ツールの synthetic）はスキップし、
    // stuck modifier による汚染を防ぐ。自前の synthetic は上の is_self_injected で既に除外済み。
    if !is_injected {
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
    }
    // VK_KANA down/up は OS のかなロックをトグルし、GJI/MS-IME がローマ字入力→JISかな
    // 入力に反転して NICOLA の romaji VK 出力が壊滅する（2026-07-06 実機: down→up
    // 135µs〜1ms の合成 VK_KANA ペアが 2 回到達し Windows Terminal が JISかな化。
    // docs/known-bugs.md BUG-08。注入元は BUG-14 調査で LLKHF_INJECTED 付き SendInput
    // と確定、MS-IME/CTF 自身が第一容疑）。
    // - LLKHF_INJECTED 付き（SendInput 由来・awase 自身のマーカーなし）: swallow する。
    // - フラグなし（物理押下 or ドライバレベル注入）: 従来どおり通すが、注入元特定の
    //   ため必ず INFO ログを残す（VK_KANA は稀なキーなのでログコストは無視できる）。
    //   通した結果 JISかな化しても idle-conv-check の restore_roman が復元する。
    if vk == crate::vk::VK_KANA {
        let dir = if is_keydown { "down" } else { "up" };
        if is_injected {
            log::info!(
                "[hook] foreign-injected VK_KANA {dir} を swallow\
                 （kana-lock 汚染防止, scan=0x{:X}, extra=0x{:X}）",
                kb.scanCode,
                kb.dwExtraInfo,
            );
            return LRESULT(1);
        }
        log::info!(
            "[hook] VK_KANA {dir} 到達 (injected=false, scan=0x{:X}, extra=0x{:X}) \
             — かなロックをトグルする可能性 (BUG-08 注入元調査ログ)",
            kb.scanCode,
            kb.dwExtraInfo,
        );
    }
    // CTRL_CONSUMED チェックと classify_key で共用するため先に取得する。
    let config = cached_hook_config();

    // Alt なりすまし: Ctrl 消費追跡・classify_key より前に vk を書き換える。
    // これにより後続の全パイプライン（is_os_modifier_held の bypass 判定含む）が
    // 無変換/変換相当のキーとして扱う。PowerToys 等の OS レベルリマップと同じ効果。
    // vk が Left/Right Alt でない、または両設定とも OFF なら vk はそのまま返る。
    // LLKHF_EXTENDED は vk が汎用 VK_MENU (0x12) で届いた場合の Left/Right 判別に使う
    // （classify_alt_side 参照）。
    let alt_extended = (kb.flags.0 & LLKHF_EXTENDED) != 0;
    if matches!(vk.0, 0x12 | 0xA4 | 0xA5) {
        log::debug!(
            "[alt-impersonation] raw vk=0x{:02X} scan=0x{:X} extended={} is_keydown={} \
             left_cfg={} right_cfg={} engine_enabled={}",
            vk.0,
            kb.scanCode,
            alt_extended,
            is_keydown,
            config.left_alt_impersonates_thumb_key,
            config.right_alt_impersonates_thumb_key,
            CACHED_ENGINE_ENABLED.load(Ordering::Relaxed),
        );
    }
    let rewritten_vk = apply_alt_impersonation(vk, is_keydown, alt_extended, config);
    if rewritten_vk != vk {
        log::debug!(
            "[alt-impersonation] impersonating: vk 0x{:02X} -> 0x{:02X}",
            vk.0,
            rewritten_vk.0
        );
    }
    vk = rewritten_vk;

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
    // Alt なりすまし中は modifier_snapshot.alt を強制的に false にする
    // （is_alt_impersonation_active の doc 参照。vk 書き換えだけでは不十分だった
    // 実機バグの修正、2026-07-19）。
    if is_alt_impersonation_active() {
        modifier_snapshot.alt = false;
    }
    let event = build_raw_key_event(
        vk,
        scan,
        is_keydown,
        kb.dwExtraInfo,
        key_classification,
        physical_pos,
        modifier_snapshot,
        is_injected,
    );

    let engine_tid = crate::engine_thread_id();
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

#[cfg(test)]
mod alt_impersonation_tests {
    use super::{classify_alt_side, decide_alt_impersonation, resolve_thumb_key};
    use crate::vk::{VK_CONVERT, VK_LMENU, VK_MENU, VK_NONCONVERT, VK_RMENU, VK_SPACE};

    const LEFT_THUMB: awase::types::VkCode = VK_NONCONVERT;

    /// vk が既に区別済みの VK_LMENU/VK_RMENU で届く環境では、extended フラグに
    /// 関わらずそのまま Left/Right と判定する。
    #[test]
    fn classify_alt_side_specific_vk() {
        assert_eq!(classify_alt_side(VK_LMENU, false), (true, false));
        assert_eq!(classify_alt_side(VK_LMENU, true), (true, false));
        assert_eq!(classify_alt_side(VK_RMENU, false), (false, true));
        assert_eq!(classify_alt_side(VK_RMENU, true), (false, true));
    }

    /// vk が汎用の VK_MENU (0x12) で届く環境（実機で確認された挙動。
    /// `vk.rs` の `classify_modifier`/`is_ctrl_variant` が汎用形も含めて
    /// 防御的にマッチしているのと同じ理由）では、LLKHF_EXTENDED で
    /// Left/Right を判別する（Right Alt が拡張キー）。
    ///
    /// このテストは実機で「Left Alt がなりすまし機能として全く発動しない」
    /// という回帰の再発防止用（vk == VK_LMENU 直接比較のみだと、この
    /// ケースを取りこぼして常に false になっていた）。
    #[test]
    fn classify_alt_side_generic_vk_menu() {
        assert_eq!(classify_alt_side(VK_MENU, false), (true, false));
        assert_eq!(classify_alt_side(VK_MENU, true), (false, true));
    }

    /// Alt 以外の VK はどちらにも該当しない。
    #[test]
    fn classify_alt_side_unrelated_vk_is_neither() {
        assert_eq!(classify_alt_side(VK_SPACE, false), (false, false));
        assert_eq!(classify_alt_side(VK_SPACE, true), (false, false));
    }

    /// "Left Alt"/"Right Alt" は無変換/変換相当の VK に解決され、
    /// なりすましフラグが立つ。
    #[test]
    fn resolve_thumb_key_alt_sentinels() {
        assert_eq!(resolve_thumb_key("Left Alt"), Some((VK_NONCONVERT, true)));
        assert_eq!(resolve_thumb_key("Right Alt"), Some((VK_CONVERT, true)));
    }

    /// 通常の VK 名は従来通り解決され、なりすましフラグは立たない。
    #[test]
    fn resolve_thumb_key_normal_vk_name() {
        assert_eq!(resolve_thumb_key("VK_SPACE"), Some((VK_SPACE, false)));
        assert_eq!(
            resolve_thumb_key("VK_NONCONVERT"),
            Some((VK_NONCONVERT, false))
        );
    }

    /// 未知のキー名は `None`（呼び出し元が `.context(...)` でエラーにする）。
    #[test]
    fn resolve_thumb_key_unknown_name_returns_none() {
        assert_eq!(resolve_thumb_key("Not A Real Key"), None);
    }

    /// エンジン ON・新規押下 → なりすまし発動、vk が親指キーに書き換わる。
    #[test]
    fn fresh_press_engine_on_impersonates() {
        let (vk, impersonating) =
            decide_alt_impersonation(VK_LMENU, LEFT_THUMB, true, false, false, true);
        assert_eq!(vk, LEFT_THUMB);
        assert!(impersonating);
    }

    /// エンジン OFF・新規押下 → なりすましなし、vk は元の Alt のまま。
    #[test]
    fn fresh_press_engine_off_does_not_impersonate() {
        let (vk, impersonating) =
            decide_alt_impersonation(VK_LMENU, LEFT_THUMB, true, false, false, false);
        assert_eq!(vk, VK_LMENU);
        assert!(!impersonating);
    }

    /// 押しっぱなし中（auto-repeat KeyDown）にエンジンが OFF に切り替わっても、
    /// 新規押下時点の判定（なりすまし中）を維持する。
    #[test]
    fn repeat_keydown_keeps_original_decision_even_if_engine_toggled_off() {
        // 新規押下時点: エンジン ON → なりすまし発動
        let (_, impersonating_after_fresh) =
            decide_alt_impersonation(VK_LMENU, LEFT_THUMB, true, false, false, true);
        assert!(impersonating_after_fresh);

        // repeat KeyDown 時点: エンジンが OFF に切り替わっていても was_down=true なので
        // 新規押下時点の判定（なりすまし中）を維持する。
        let (vk, impersonating) = decide_alt_impersonation(
            VK_LMENU,
            LEFT_THUMB,
            true, // is_keydown (repeat)
            true, // was_down
            impersonating_after_fresh,
            false, // engine now OFF
        );
        assert_eq!(
            vk, LEFT_THUMB,
            "repeat KeyDown はなりすまし継続すべき（途中でズレると Alt が stuck する）"
        );
        assert!(impersonating);
    }

    /// KeyUp は新規押下時点の判定をそのまま使う（KeyUp 時点でエンジン状態が
    /// 変わっていても、対応する KeyDown と対称的に扱われる）。
    #[test]
    fn keyup_uses_the_decision_recorded_at_keydown() {
        // KeyDown 時点: エンジン ON → なりすまし発動
        let (_, impersonating_after_down) =
            decide_alt_impersonation(VK_LMENU, LEFT_THUMB, true, false, false, true);

        // KeyUp 時点: エンジンが OFF に切り替わっていても、KeyDown 時点の判定を使う。
        let (vk_up, impersonating_after_up) = decide_alt_impersonation(
            VK_LMENU,
            LEFT_THUMB,
            false, // is_keydown = false (KeyUp)
            true,  // was_down (直前は押下中だった)
            impersonating_after_down,
            false, // engine now OFF
        );
        assert_eq!(
            vk_up, LEFT_THUMB,
            "KeyUp は対応する KeyDown のなりすまし判定と対称であるべき"
        );
        assert!(!impersonating_after_up, "KeyUp 後は押下状態ではないため false");
    }

    /// 押していない状態から始まる通常の Alt 単体タップは、エンジン OFF なら
    /// KeyDown/KeyUp とも通常の Alt のまま（回帰: 常時なりすましにならないこと）。
    #[test]
    fn normal_alt_tap_when_engine_off_stays_as_alt_through_down_and_up() {
        let (vk_down, imp_down) =
            decide_alt_impersonation(VK_LMENU, LEFT_THUMB, true, false, false, false);
        assert_eq!(vk_down, VK_LMENU);
        assert!(!imp_down);

        let (vk_up, imp_up) =
            decide_alt_impersonation(VK_LMENU, LEFT_THUMB, false, true, imp_down, false);
        assert_eq!(vk_up, VK_LMENU);
        assert!(!imp_up);
    }
}
