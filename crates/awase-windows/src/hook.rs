#![allow(unsafe_code)] // Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
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

// decide_alt_impersonation / resolve_thumb_key / classify_alt_side は
// state::alt_impersonation へ移設した（ADR-082 決定1実施記録の次の一歩、BUG-41）。
// 判定ロジック本体はそちらを参照。resolve_thumb_key は既存呼び出し元
// （app/bootstrap.rs 等の `hook::resolve_thumb_key`）を変更せずに済むよう
// ここで再エクスポートする。
pub use crate::state::alt_impersonation::resolve_thumb_key;
use crate::state::alt_impersonation::{classify_alt_side, decide_alt_impersonation};

/// Left/Right Alt キーのなりすまし処理（グローバル状態の読み書きを伴う副作用あり）。
/// 判定ロジック本体は `decide_alt_impersonation`（純粋関数）に委譲する。
///
/// `vk` が Left/Right Alt でない場合、または対応する設定が OFF の場合は
/// `vk` をそのまま返す。`extended` は `classify_alt_side` 参照。
#[must_use]
fn apply_alt_impersonation(
    vk: VkCode,
    is_keydown: bool,
    extended: bool,
    config: HookConfig,
) -> VkCode {
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

/// Alt なりすまし適用後の左右親指キー押下時刻（µs）。0 = 押下されていない。
static LEFT_THUMB_DOWN_AT_US: AtomicU64 = AtomicU64::new(0);
static RIGHT_THUMB_DOWN_AT_US: AtomicU64 = AtomicU64::new(0);

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

/// Win キー（左右どちらか）が「新鮮に」押下中かを返す。
///
/// `is_physical_key_down(VK_LWIN/VK_RWIN)` の単純な OR ではなく、
/// `tuning::WIN_KEY_HELD_STALE_MS` 以上「押されたまま」の値は stale として
/// 無視する（2026-08-06 実機: Win キー押下で検索UIが開いた際に KeyUp が
/// `WH_KEYBOARD_LL` フックチェーンの前段で消費され awase に届かず、
/// `PHYSICAL_KEY_STATE` が恒久的に「押されたまま」スタックし、以後
/// `VK_IME_ON`/`VK_IME_OFF` の実送信が `win_key_held()` により無期限に
/// スキップされ続けた不具合の対策。原因の確度は「推測」— `WH_KEYBOARD_LL`
/// 自体は他キーには正常に応答していたため全面停止ではなく、Win キー固有の
/// 経路でのみ KeyUp が失われたと考えられる）。
///
/// `tsf/send.rs::send_eager_warmup_vk_pair` と `ime.rs::send_ime_mode_key`
/// の両方が使う唯一の判定点（旧実装は各所で `is_physical_key_down` の OR を
/// 個別に重複記述していた）。
#[must_use]
pub fn win_key_held() -> bool {
    use crate::state::win_key_guard::is_held_fresh;
    use crate::vk::{VK_LWIN, VK_RWIN};
    is_held_fresh(
        physical_key_held_ms(VK_LWIN),
        crate::tuning::WIN_KEY_HELD_STALE_MS,
    ) || is_held_fresh(
        physical_key_held_ms(VK_RWIN),
        crate::tuning::WIN_KEY_HELD_STALE_MS,
    )
}

/// Alt キー（左右どちらか）が「新鮮に」押下中かを返す（BUG-62）。
///
/// `win_key_held()` と全く同型の対策（`is_held_fresh` を共有し、判定点も
/// 同じ関数として集約）。BUG-48 が Win キーで踏んだ「KeyUp が
/// `WH_KEYBOARD_LL` フックチェーンの前段で消費され awase に届かず
/// `PHYSICAL_KEY_STATE` が恒久的に「押されたまま」スタックする」不具合は、
/// メカニズム自体が Win キー固有ではなく「何らかの OS/シェル側 UI が
/// 一瞬でもキーイベントを横取りする」一般的なリスクである。BUG-62（Alt+かな
/// swallow）実装後、ユーザーから「Alt down はあるが Alt up が（ログにすら）
/// 一切出ない不具合があるのでは」という指摘があり、これは正にログに残らない
/// 種類の不具合（フックの前段で消費されるため）で、報告時点では実機ログでの
/// 直接確認ができない。BUG-48 と同じ防御を先回りで適用する:
/// **これ自体は BUG-62 の Alt 押下判定（かなキー swallow の可否）が、Alt が
/// 本当にスタックした場合に恒久的に true を返し続け、以後の単独「かな」
/// キー（IME ON）まで誤って swallow してしまう二次被害を防ぐ目的もある。**
///
/// `WIN_KEY_HELD_STALE_MS` をそのまま再利用する（新規タイミング定数は実測
/// 無しに追加しない、`.claude/rules/tuning-constants.md`）。この値自体は
/// Win キー固有の実測ではなく「人間のチョード操作は通常数百ms 以内に完了する」
/// という定性的推論に基づく暫定値であり、対象キーを問わず適用可能な性質の
/// ものと判断した。
#[must_use]
pub fn alt_key_held() -> bool {
    use crate::state::win_key_guard::is_held_fresh;
    use crate::vk::{VK_LMENU, VK_MENU, VK_RMENU};
    is_held_fresh(
        physical_key_held_ms(VK_MENU),
        crate::tuning::WIN_KEY_HELD_STALE_MS,
    ) || is_held_fresh(
        physical_key_held_ms(VK_LMENU),
        crate::tuning::WIN_KEY_HELD_STALE_MS,
    ) || is_held_fresh(
        physical_key_held_ms(VK_RMENU),
        crate::tuning::WIN_KEY_HELD_STALE_MS,
    )
}

/// 合成 IME モードキー（`VK_DBE_*`/`VK_KANJI` 等）を注入してよいかの
/// Win/Alt ガード。`false` の場合、呼び出し元は注入をスキップすべき。
///
/// Win: Win 押下中に送ると Win+VK として届き、Win↑ 時にスタートメニューが
/// 開く（`tsf/send.rs::send_eager_warmup_vk_pair` と同じ判定点）。
///
/// Alt: Alt 押下中に合成 `VK_DBE_HIRAGANA` 等を送ると MS-IME の
/// 「Alt+かな」ローマ字⇔JISかな直接入力切替ショートカット（BUG-61/62）と
/// 同様に解釈され、実際に JIS かな直接入力へ切り替わることを実機診断で
/// 確認済み（2026-08-17）。この危険はGJI/MS-IMEを問わず同じOSレベルの
/// ショートカット解釈に起因するため、IME種別を問わず適用する
/// （`kp_restore_kana_from_half_width`のMS-IME分岐と
/// `Output::send_gji_half_width_alnum_toggle`のGJI分岐、両方がこの
/// 判定点を共有する）。
#[must_use]
pub fn ime_mode_key_injection_blocked_by_modifier() -> bool {
    win_key_held() || alt_key_held()
}

/// Alt が押下中に、Alt が「何も修飾しなかった」ように見える形でキーを丸ごと
/// swallow するときに呼ぶ（BUG-62 追補2・3）。
///
/// Windows は Alt を単独で離すとシステムメニュー（`SC_KEYMENU`、アクセラレータ
/// 探索モード）を起動する仕様があり、これが起きると以後の入力がメニュー
/// ナビゲーションとして食われる（AutoHotkey の `#MenuMaskKey` と同じ問題設定）。
/// ダミーの Ctrl down+up を自己注入し、OS に「Alt は何かを修飾した」と認識させて
/// `SC_KEYMENU` の発火を防ぐ（AutoHotkey の既定マスクキーと同じ選択: Ctrl は
/// 可視の副作用を持たない）。呼び出し元は対象キーの KeyDown 時点で1回だけ
/// 呼ぶこと（KeyUp 側での重複注入は不要）。dwExtraInfo は `INJECTED_MARKER` —
/// 自己注入として hook 冒頭の `is_self_injected` で弾かれ、エンジンには渡らない。
fn inject_alt_menu_mask() {
    let mask_inputs = [
        crate::tsf::output::make_key_input_ex(crate::vk::VK_CONTROL, false, INJECTED_MARKER),
        crate::tsf::output::make_key_input_ex(crate::vk::VK_CONTROL, true, INJECTED_MARKER),
    ];
    let sent = crate::win32::send_input_safe(&mask_inputs);
    log::info!("[hook] inject_alt_menu_mask: ダミー Ctrl down+up 注入 sent={sent}/2");
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
    LEFT_THUMB_DOWN_AT_US.store(0, Ordering::Relaxed);
    RIGHT_THUMB_DOWN_AT_US.store(0, Ordering::Relaxed);
    ALT_L_IMPERSONATING.store(false, Ordering::Relaxed);
    ALT_R_IMPERSONATING.store(false, Ordering::Relaxed);
    ALT_L_WAS_DOWN.store(false, Ordering::Relaxed);
    ALT_R_WAS_DOWN.store(false, Ordering::Relaxed);
    log::info!("[hook] PHYSICAL_KEY_STATE をリセット（全 VK を解放状態に）");
}

/// `disable_apps` へ出入りする際にフックローカルなラッチを後始末する（BUG-78 対策）。
///
/// `reset_physical_key_state()`（全 256 VK を無条件クリア）とは意図的に別系統にする。
/// フォーカス遷移は高頻度に起きるため、無条件の全クリアを持ち込むと Alt+Tab で
/// 無効アプリへ出入りする瞬間（Alt が物理押下中であることが多い）に
/// `alt_key_held()` を偽らせ、BUG-62 の「Alt+かな で JIS かな直接入力へ不可逆に
/// 切り替わる」保護を無効化中でなくても壊しかねない（設計段階の premortem で
/// 指摘され、この分離に至った）。
///
/// - Enter/Leave 共通: Alt なりすまし・チョード関連の一時ラッチのみを force-false
///   する。`ALT_L/R_WAS_DOWN`・`ALT_L/R_IMPERSONATING`・`CTRL_CONSUMED_SINCE_DOWN`・
///   親指キー押下タイムスタンプが対象。**`PHYSICAL_KEY_STATE`（Alt/Win を含む）
///   本体には一切触れない。**
///   無効アプリに入った瞬間に pending だったチョードは呼び出し元
///   （`runtime/focus_tracking.rs`）が engine 側の flush で別途処理する。
/// - Leave のみ追加: `PHYSICAL_KEY_STATE`/`PHYSICAL_KEY_DOWN_AT_MS` のうち
///   Ctrl/Shift の 6 スロット（`VK_CONTROL`/`VK_LCONTROL`/`VK_RCONTROL`/
///   `VK_SHIFT`/`VK_LSHIFT`/`VK_RSHIFT`）だけを force-false する。無効化対象
///   アプリ（既定で mstsc.exe）滞在中は KeyUp がフックに届かず Ctrl/Shift が
///   スタックする既知問題（`docs/known-bugs.md` BUG-78）への対策。
///   **Alt/Win は対象外**（Alt+Tab の最悪ケースを避けるため）。Ctrl/Shift は
///   Alt+Tab 中に押されていることが稀なうえ、誤ってクリアしても次の物理
///   KeyDown/KeyUp で自己修復する安全側の誤りである（stuck-true は BUG-48 型の
///   恒久障害を生む危険側だが、stuck-false はそうならない）。
pub(crate) fn clear_hook_latches_for_app_disable(
    edge: crate::state::app_suppression::SuppressionEdge,
) {
    use crate::state::app_suppression::SuppressionEdge;
    use crate::vk::{VK_CONTROL, VK_LCONTROL, VK_LSHIFT, VK_RCONTROL, VK_RSHIFT, VK_SHIFT};

    if matches!(edge, SuppressionEdge::None) {
        return;
    }

    ALT_L_WAS_DOWN.store(false, Ordering::Relaxed);
    ALT_R_WAS_DOWN.store(false, Ordering::Relaxed);
    ALT_L_IMPERSONATING.store(false, Ordering::Relaxed);
    ALT_R_IMPERSONATING.store(false, Ordering::Relaxed);
    CTRL_CONSUMED_SINCE_DOWN.store(false, Ordering::Relaxed);
    LEFT_THUMB_DOWN_AT_US.store(0, Ordering::Relaxed);
    RIGHT_THUMB_DOWN_AT_US.store(0, Ordering::Relaxed);

    if matches!(edge, SuppressionEdge::Leave) {
        for vk in [
            VK_CONTROL,
            VK_LCONTROL,
            VK_RCONTROL,
            VK_SHIFT,
            VK_LSHIFT,
            VK_RSHIFT,
        ] {
            if let Some(slot) = PHYSICAL_KEY_STATE.get(vk.0 as usize) {
                slot.store(false, Ordering::Relaxed);
            }
            if let Some(slot) = PHYSICAL_KEY_DOWN_AT_MS.get(vk.0 as usize) {
                slot.store(0, Ordering::Relaxed);
            }
        }
        log::info!("[app-disable] Leave: Ctrl/Shift の PHYSICAL_KEY_STATE をクリア（BUG-78対策）");
    }
    log::info!("[app-disable] {edge:?}: hook latches をクリア");
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

/// `config.app_overrides.disable_apps` にマッチするアプリへ現在フォーカス中かの
/// キャッシュ。メインスレッドのフォーカス追跡（`runtime/focus_tracking.rs`）が
/// `set_focus_app_disabled()` で書き込み、フックスレッドが `hook_callback` 冒頭で
/// 読む（`CACHED_ENGINE_ENABLED` と同型の受け渡しパターン）。
///
/// マッチしている間、`hook_callback` は生のキーイベントを一切消費せず
/// `CallNextHookEx` でそのまま OS に通す（awase を丸ごとバイパスする）。
/// 既存の `force_bypass`（`FocusKind::NonText` → `SendInput` で再注入）と異なり
/// `LLKHF_INJECTED` の付かない生イベントが届くため、injected input を無視する
/// ゲーム（DirectInput/Raw Input 系）にも通用する。
static FOCUS_APP_DISABLED: AtomicBool = AtomicBool::new(false);

/// `GeneralConfig::swallow_alt_kana_input_method_switch` のキャッシュ（BUG-62 追補5）。
/// 既定値は `true`（安全側）で、config 読み込み前に発火しても常に swallow する。
static CACHED_SWALLOW_ALT_KANA_MODE_SWITCH: AtomicBool = AtomicBool::new(true);

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
    LEFT_THUMB_DOWN_AT_US.store(0, Ordering::Relaxed);
    RIGHT_THUMB_DOWN_AT_US.store(0, Ordering::Relaxed);
}

/// 現在押下中の左右親指キーの KeyDown 時刻（µs）を返す。
#[must_use]
pub fn thumb_down_timestamps() -> (Option<Timestamp>, Option<Timestamp>) {
    let to_option = |value| (value != 0).then_some(value);
    (
        to_option(LEFT_THUMB_DOWN_AT_US.load(Ordering::Relaxed)),
        to_option(RIGHT_THUMB_DOWN_AT_US.load(Ordering::Relaxed)),
    )
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

/// 現在フォーカス中のアプリが `disable_apps` にマッチしているかを設定する
/// （`runtime/focus_tracking.rs` のフォーカス変更処理から呼ぶ）。
pub fn set_focus_app_disabled(disabled: bool) {
    FOCUS_APP_DISABLED.store(disabled, Ordering::Release);
}

/// 現在フォーカス中のアプリで awase が無効化されているか。
#[must_use]
pub fn is_focus_app_disabled() -> bool {
    FOCUS_APP_DISABLED.load(Ordering::Acquire)
}

/// `GeneralConfig::swallow_alt_kana_input_method_switch` を設定する（config 読み込み後に呼ぶ）。
pub fn set_swallow_alt_kana_mode_switch(enabled: bool) {
    CACHED_SWALLOW_ALT_KANA_MODE_SWITCH.store(enabled, Ordering::Release);
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

/// overflow ラッチ中（HOOK_KEYS の resync 待ち）にキーを OS へ渡す/飲み込む
/// かの判定を1箇所に集約する（コードレビュー指摘5）。以前は overflow ラッチの
/// 早期return分岐と `ProduceResult::Overflow` の match アームにほぼ同一の
/// ロジックが重複していた。
///
/// Alt なりすまし発動中は `CallNextHookEx` に本物の `KBDLLHOOKSTRUCT`（本物の
/// Alt）が渡ってしまい、Alt 単独タップとしてシステムメニューが起動しうる
/// ため、この場合のみ飲み込む（`LRESULT(1)`）。それ以外は OS へパススルーする。
fn passthrough_or_swallow_for_impersonation(
    hook_handle: HHOOK,
    ncode: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if is_alt_impersonation_active() {
        LRESULT(1)
    } else {
        unsafe { CallNextHookEx(Some(hook_handle), ncode, wparam, lparam) }
    }
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

    // `disable_apps`（既定 mstsc.exe）にマッチするアプリへフォーカス中は、
    // ここで生キーイベントをそのまま OS に通す（awase を丸ごとバイパスする、
    // BUG-78 対策）。`PHYSICAL_KEY_STATE` の更新（上のブロック）より後に置く —
    // 前に置くと無効アプリに入る直前から押していたキーの KeyUp が記録されず、
    // 今回対策したいスタックをこの分岐自体が新規に生んでしまう。
    // VK_KANA/Alt なりすまし等の以降の変換系ロジックより前に置くことで、
    // それらの介入（BUG-08/BUG-61/BUG-62 対策含む）も無効化中は一切効かなくする
    // （ユーザー判断により例外なく無効化する）。
    if FOCUS_APP_DISABLED.load(Ordering::Relaxed) {
        return CallNextHookEx(Some(hook_handle), ncode, wparam, lparam);
    }

    // VK_KANA down/up は OS のかなロックをトグルし、GJI/MS-IME がローマ字入力→JISかな
    // 入力に反転して NICOLA の romaji VK 出力が壊滅する（2026-07-06 実機: down→up
    // 135µs〜1ms の合成 VK_KANA ペアが 2 回到達し Windows Terminal が JISかな化。
    // docs/known-bugs.md BUG-08。注入元は BUG-14 調査で LLKHF_INJECTED 付き SendInput
    // と確定、MS-IME/CTF 自身が第一容疑）。
    // - LLKHF_INJECTED 付き（SendInput 由来・awase 自身のマーカーなし）: swallow する。
    // - Alt 押下中の物理押下（BUG-62）: MS-IME の公式ショートカット「Alt+かな
    //   （カタカナ ひらがな ローマ字）キー」は入力方式（ローマ字変換 vs JIS かな
    //   直接入力）そのものを切り替える。BUG-61 の実機調査で、いったん JIS かな側へ
    //   切り替わると `ImmSetConversionStatus`（IMC write）・`VK_DBE_ROMAN` 注入の
    //   どちらでも復旧不能と確定した（Windows にこの入力方式を外部から戻す公式
    //   API が存在しないため）。「通しても後で直せる」という以前の前提が誤りだった
    //   ため、この組み合わせだけは未然に swallow して OS に一切渡さない。
    // - フラグなし・Alt 非押下（物理押下 or ドライバレベル注入）: 従来どおり通すが、
    //   注入元特定のため必ず INFO ログを残す（VK_KANA は稀なキーなのでログコストは
    //   無視できる）。単独の VK_KANA は「IME ON」ショートカットであり、
    //   Alt+VK_KANA（入力方式切替）とは異なる操作のため引き続き通過させる。
    //
    // BUG-62 追補3（2026-08-09、git bisect で特定）: 上記2つの swallow 分岐は
    // いずれも Alt 押下中に発火すると、かな キー自体を OS へ一切渡さないため、
    // OS 視点では「Alt が何も修飾せず単独でタップされた」ことと区別がつかない
    // （Windows は Alt を単独で離すとシステムメニュー `SC_KEYMENU` を起動し、
    // 以後の入力がメニューナビゲーションとして食われる）。foreign-injected 分岐
    // （本ブロックの原型、BUG-08 由来）は Alt の状態を見ずに常時 swallow して
    // いたため、この副作用は BUG-62 で Alt 押下判定を導入するより前から存在した
    // 可能性が高い——ユーザー報告「Alt+かな の後は何も入力できなくなる、以前は
    // 無かった」を `git bisect` で追ったところ、原因はまさにこの分岐を新設した
    // コミット（`b38d67f8`、2026-07-05）に一致した。両分岐に同じマスク対策
    // （`inject_alt_menu_mask`）を適用する。
    if vk == crate::vk::VK_KANA {
        let dir = if is_keydown { "down" } else { "up" };
        let alt_held = alt_key_held();
        if is_injected {
            log::info!(
                "[hook] foreign-injected VK_KANA {dir} を swallow\
                 （kana-lock 汚染防止, scan=0x{:X}, extra=0x{:X}, alt_held={alt_held}）",
                kb.scanCode,
                kb.dwExtraInfo,
            );
            if is_keydown && alt_held {
                inject_alt_menu_mask();
            }
            return LRESULT(1);
        }
        if alt_held {
            log::info!(
                "[hook] Alt+VK_KANA {dir} を swallow（BUG-62: MS-IME の Alt+かな＝\
                 ローマ字/JISかな入力方式切替ショートカット。BUG-61 で復旧不能と\
                 確定済みのため未然に防ぐ, scan=0x{:X}, extra=0x{:X}）",
                kb.scanCode,
                kb.dwExtraInfo,
            );
            if is_keydown {
                inject_alt_menu_mask();
            }
            return LRESULT(1);
        }
        log::info!(
            "[hook] VK_KANA {dir} 到達 (injected=false, scan=0x{:X}, extra=0x{:X}) \
             — かなロックをトグルする可能性 (BUG-08 注入元調査ログ)",
            kb.scanCode,
            kb.dwExtraInfo,
        );
    }

    // BUG-62 追補4（2026-08-09、実機ログで確定）: 追補1〜3 はいずれも VK_KANA
    // (0x15) のみを見ており効果が無かった。実際に物理 Alt+かな を押した際、
    // Windows のキーボードレイアウトドライバは VK_KANA ではなく
    // VK_DBE_ROMAN (0xF5) / VK_DBE_NOROMAN (0xF6) を hook_callback に渡す
    // （ユーザー提供ログで vk=0xF5 up → vk=0xF6 down が Alt 押下中に
    // PassThrough で素通りし、直後に IME の入力方式が実際に切り替わったことを
    // 確認済み）。この2つは BUG-61 の実機調査で「一度切り替わると
    // ImmSetConversionStatus・VK_DBE_ROMAN 注入のどちらでも復旧不能」と
    // 確定済みのキーそのものなので、既定では常に未然に swallow する。
    // VK_KANA 分岐と同じ理由（Alt 押下中に丸ごと swallow すると OS からは
    // 「Alt 単独タップ」に見え SC_KEYMENU が起動しうる）で `inject_alt_menu_mask`
    // を適用する。
    //
    // 追補5（2026-08-09）: JIS かな直接入力を意図的に使いたい（= awase の
    // Engine を OFF にして使う）ユーザー向けに、
    // `GeneralConfig::swallow_alt_kana_input_method_switch` で無効化できる
    // ようにした。既定値は `true`（従来どおり常時 swallow）。
    if (vk == crate::vk::VK_DBE_ROMAN || vk == crate::vk::VK_DBE_NOROMAN)
        && CACHED_SWALLOW_ALT_KANA_MODE_SWITCH.load(Ordering::Acquire)
    {
        let dir = if is_keydown { "down" } else { "up" };
        let name = if vk == crate::vk::VK_DBE_ROMAN {
            "VK_DBE_ROMAN"
        } else {
            "VK_DBE_NOROMAN"
        };
        let alt_held = alt_key_held();
        log::info!(
            "[hook] {name} {dir} を swallow（BUG-62追補4: Alt+かな の実際のキー\
             コード。BUG-61 で復旧不能と確定済みのため未然に防ぐ, scan=0x{:X}, \
             extra=0x{:X}, alt_held={alt_held}）",
            kb.scanCode,
            kb.dwExtraInfo,
        );
        if is_keydown && alt_held {
            inject_alt_menu_mask();
        }
        return LRESULT(1);
    }

    // HOOK_KEYS の overflow ラッチが立っている間（エンジンスレッドが resync
    // するまで）は、以降の分類・なりすまし処理を一切行わず OS へ直接パス
    // スルーする。バッファ再生とパススルーが1打鍵ごとに交互混在する
    // 順序崩れを防ぐため（指摘2-3）。
    //
    // 上の VK_KANA / VK_DBE_ROMAN / VK_DBE_NOROMAN swallow ガード（BUG-08/61/62
    // 対策、「一度切り替わると復旧不能」と確定済み）より**後**に置く（コード
    // レビュー指摘2）。以前はこのラッチ判定が上記ガードより手前にあったため、
    // overflow ラッチ中はこれらのキーが無条件で OS へ素通りし、ガードが防いで
    // いたはずの復旧不能な破損が起こりえた。overflow は稀にしか起きない上
    // 一時的な状態なので、破損防止ガードを常に優先する。
    if crate::hook_channel::HOOK_KEYS.is_overflow_latched() {
        return passthrough_or_swallow_for_impersonation(hook_handle, ncode, wparam, lparam);
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

    if !is_injected {
        let update_thumb = |slot: &AtomicU64| {
            if is_keydown {
                let prev = slot.load(Ordering::Relaxed);
                if prev == 0 {
                    slot.store(now_timestamp(), Ordering::Relaxed);
                }
            } else {
                slot.store(0, Ordering::Relaxed);
            }
        };
        if vk == config.left_thumb_vk {
            update_thumb(&LEFT_THUMB_DOWN_AT_US);
        }
        if vk == config.right_thumb_vk {
            update_thumb(&RIGHT_THUMB_DOWN_AT_US);
        }
    }

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

    let produce_result = crate::hook_channel::HOOK_KEYS.produce(event);
    crate::hook_channel::request_engine_wake();
    match produce_result {
        // 通常時: 常に消費（engine thread が PassThrough 判定して reinject する）。
        crate::hook_channel::ProduceResult::Accepted => LRESULT(1),
        // overflow時（指摘2-1）: リングに積めなかったキーを黙って消し去るより、
        // OS へそのままパススルーする方が実害が小さい。ただし Alt なりすまし中は
        // 上の overflow ラッチ分岐と同じ理由で飲み込む（dropped 計上のみ）。
        crate::hook_channel::ProduceResult::Overflow => {
            passthrough_or_swallow_for_impersonation(hook_handle, ncode, wparam, lparam)
        }
    }
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

// alt_impersonation_tests は state::alt_impersonation::tests へ移設した
// （既存5件 + 新規の網羅テーブルテスト2件、ADR-082 決定1実施記録の次の一歩）。
