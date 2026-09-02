#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! WM_* メッセージハンドラ
//!
//! `run_message_loop` の `match msg.message` 各 arm を関数として切り出したもの。
//! すべて `pub(crate)` で主に `app/mod.rs` から呼ばれる。ただし
//! `handle_wm_app_tray` / `handle_wm_command` は `WndProc` に同期配送される
//! sent message（`GetMessageW` の戻り値に現れない）に対応するため、
//! `tray::tray_wnd_proc` からも呼ばれる（詳細は `tray_wnd_proc` の doc 参照）。

use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO};

use crate::focus::FocusKind;
use crate::hook;
use crate::hook::CallbackResult;
use crate::runtime::engine_window::PumpContext;
use crate::state::post_bypass::{classify_post_bypass_key, PostBypassKey};
use crate::state::scoped_latch::ScopeCheck;
use crate::tray;
use crate::vk::VkCodeExt;
use crate::win32::post_to_main_thread;
use crate::{
    with_app, with_app_ref, Runtime, TIMER_GJI_LONG_IDLE, TIMER_HOOK_WATCHDOG, TIMER_IME_REFRESH,
    TIMER_OUTPUT_GUARD, TIMER_POWER_RESUME, TIMER_TSF_GATE, TIMER_TSF_PROBE, WM_EXECUTE_EFFECTS,
};
use awase::platform::ImeOpenOutcome;
use awase::types::{ContextChange, VkCode};

static DRAIN_PENDING: AtomicBool = AtomicBool::new(false);
static DRAIN_RERUN_PENDING: AtomicBool = AtomicBool::new(false);

fn recover_pending_drain_request() {
    if DRAIN_PENDING.load(Ordering::Acquire) {
        return;
    }
    if DRAIN_RERUN_PENDING.swap(false, Ordering::AcqRel) {
        log::debug!("[drain] recovering deferred drain request");
        post_to_main_thread(crate::tsf::probe_bridge::WM_DRAIN_OUTPUT_QUEUE);
    }
}

fn finish_drain() {
    DRAIN_PENDING.store(false, Ordering::Release);
    recover_pending_drain_request();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyOrigin {
    Hook(PumpContext),
    DeferredReplay,
    /// `TIMER_IME_OFF_RESCUE` 満了時の再処理（追加発見E）。
    ///
    /// 50ms 救済窓が保留していたキーを、救済窓 defer を再度かけずに
    /// (`kp_run_inner` の `skip_rescue_defer=true` 相当) 即時処理する。
    /// `deliver_key_event` はこの origin のとき `Runtime::process_key_event` の
    /// 代わりに `Runtime::replay_ime_off_rescue_event` を呼ぶ。
    ImeOffRescueReplay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyDelivery {
    Consumed,
    Reinjected,
}

use crate::app::{
    check_keyboard_layout_on_change, launch_bug_report, launch_settings, reload_config,
};

/// `Engine::on_timeout` 呼び出し直後に、ソロ連打緊急 OFF（ADR-055 追補）が
/// 発動していればトレイ通知を出す。
///
/// 通常の `Ctrl+Shift+変換/無変換` による意図的な engine on/off では発動しない
/// ため、ここで毎回チェックしてもユーザーを煩わせない。
fn notify_if_solo_off_triggered(app: &mut Runtime) {
    if app.engine.take_solo_off_notification() {
        app.platform.tray.show_balloon(
            "awase",
            "無変換キーの連打でエンジンを緊急停止しました。\n\
             戻すには Ctrl+Shift+変換 を押してください。",
        );
    }
}

/// OS かな入力ロック警告のトレイ表示を、投函時点の値ではなくディスパッチ時点の
/// `KanaLockHysteresis::warned()` へ同期する。再入で複数回 repost されても
/// 常に最新の値へ収束する（冪等）。
pub(crate) fn handle_wm_kana_lock_warning_changed(app: &mut Runtime) {
    app.platform
        .tray
        .set_kana_lock_warned(app.kana_lock_hysteresis.warned());
}

/// バッチ境界で1回だけ走るべき resync 処理（指摘5）。
///
/// `crate::runtime::engine_window::take_needs_engine_resync()` はモーダルポンプの
/// 出入りごとに一度立つバッチ単位のフラグであり、本来バッチ（=1回の
/// `WM_KEY_FROM_HOOK`、または1回の drain で処理する複数キー）につき1回だけ
/// 消費すればよい。以前は `deliver_key_event` 冒頭（per-event）で呼んでいたため、
/// drain で複数キーをまとめて処理する際に2件目以降でも重複して素通りチェックが
/// 走っていた（実害は無いが無駄な `take_needs_engine_resync()` 呼び出し）。
/// 呼び出し元（`handle_wm_key_from_hook`・`handle_wm_timer`(`TIMER_IME_OFF_RESCUE`)・
/// `handle_wm_drain_output_queue`）がバッチ境界で1回だけ呼ぶこと。
pub(crate) fn begin_key_batch(app: &mut Runtime) {
    if crate::runtime::engine_window::take_needs_engine_resync() {
        let ctx = app.build_ctx();
        let decision = app
            .engine
            .on_command(awase::engine::EngineCommand::FocusChanged, &ctx);
        app.execute_decision_suppressed(decision);
    }
}

/// `deliver_key_event` の戻り値が `Reinjected` なら `WM_EXECUTE_EFFECTS` を要求
/// する（コードレビュー指摘10）。`handle_wm_key_from_hook` と
/// `handle_wm_timer`(`TIMER_IME_OFF_RESCUE`) で重複していた3行パターンを共通化
/// した。`handle_wm_drain_output_queue` はバッチ内の複数キーをまとめて
/// `any_reinject` フラグで判定するため対象外（別パターンのまま）。
fn post_effects_if_reinjected(delivery: KeyDelivery) {
    if matches!(delivery, KeyDelivery::Reinjected) {
        post_to_main_thread(WM_EXECUTE_EFFECTS);
    }
}

/// フックスレッドから転送された物理キーイベントを処理する。
///
/// `WM_EXECUTE_EFFECTS` の post は行わない（指摘8）。`KeyDelivery::Reinjected`
/// を返すだけにとどめ、post は呼び出し元（`handle_wm_key_from_hook`・
/// `handle_wm_drain_output_queue`）の責務にする。以前は複数の早期return分岐が
/// 個別に post していたため、1バッチ内で複数キーが Reinjected になると
/// `WM_EXECUTE_EFFECTS` が N 回投函されていた。
pub(crate) fn deliver_key_event(
    app: &mut Runtime,
    event: awase::types::RawKeyEvent,
    origin: KeyOrigin,
) -> KeyDelivery {
    if matches!(origin, KeyOrigin::Hook(_)) {
        app.platform_state.gate.last_hook_activity_ms = hook::current_tick_ms();
    }

    let is_key_down = matches!(event.event_type, awase::types::KeyEventType::KeyDown);

    // ── `[[keymap]]` latch チェック（ADR-114 決定2 ステップ1、最優先）──
    // `Nested`/`NonText` の早期returnより前、`origin`/`focus_kind` を問わず
    // 必ず実行する。片方だけを先に動かすと、latch が残った状態でこの vk の
    // KeyDown/KeyUp のどちらか一方だけが先に素通りしてしまう Down/Up 非対称
    // （OS 側で「押しっぱなし」に見えるイベント）が生じるため、KeyUp 回収と
    // KeyDown の自動リピート抑制の両方をここに置く。
    if app
        .platform_state
        .keymap
        .keymap_latch
        .is_latched(event.vk_code)
    {
        if is_key_down {
            // 自動リピート抑制: find_match を呼ばず黙って consume する
            // （target_vk は再送しない）。HOOK_KEYS overflow で latch が
            // stale に残っていた場合でも、この分岐に例外を設けない
            // （latch 漏れ対策、T5 参照）。
            return KeyDelivery::Consumed;
        }
        app.platform_state
            .keymap
            .keymap_latch
            .release(event.vk_code);
        return KeyDelivery::Consumed;
    }

    if matches!(origin, KeyOrigin::Hook(PumpContext::Nested)) {
        app.executor.enqueue_reinject(event);
        return KeyDelivery::Reinjected;
    }

    // NonText フォーカス（タスクバー等）はすべて OS にパススルー。
    //
    // ImeOffRescueReplay（コードレビュー指摘3）はこの早期returnの対象外にする。
    // 50ms 救済窓が保留していたキーはユーザーの明示的な IME OFF ジェスチャーで
    // あり、発火時点で focus_kind が（フォーカス遷移中等で一時的・誤って）
    // NonText と分類されていても、黙ってパススルーへ変換しリトライ無しで
    // 無効化してはならない。`replay_ime_off_rescue_event`/`kp_run_inner` へ
    // 確実に到達させる。
    if app.platform_state.focus.focus_kind == FocusKind::NonText
        && !matches!(origin, KeyOrigin::ImeOffRescueReplay)
    {
        app.executor.enqueue_reinject(event);
        return KeyDelivery::Reinjected;
    }

    // ── `[[keymap]]` KeyDown 新規照合（ADR-114 決定2 ステップ2）──
    // NonText パススルーの後・`[[post_bypass]]` 消費の前。ステップ1 で
    // latch 済みと判定された vk はここに到達しない（既に consume 済み）ため、
    // ここで扱うのは「まだ latch されていない vk の新規 KeyDown」のみ。
    // NICOLA エンジンには一切見せない（`Ctrl+I` の `I` が同時打鍵判定に
    // 巻き込まれるのを防ぐ）ため `[[post_bypass]]` より先に評価する。
    //
    // 既知の限界（v1 スコープ）: `FocusKind::NonText` では一切効かない
    // （このチェックが NonText 早期returnの後にあるため）。`[[keymap]]` の
    // 送信（SendInput）が「NonText では awase は一切手を出さない」という
    // 既存の不変条件を破ることになる副作用の監査コストを避けるための
    // 意図的な v1 判断。
    if is_key_down {
        if let Some(delivery) = consume_keymap_match(app, event) {
            return delivery;
        }
    }

    // ── Post-bypass passthrough（Ctrl+J 等 tmux prefix 直後のコマンドキー）──
    // Ctrl+key bypass の直後に non-Ctrl 非修飾キーが来た場合、NICOLA エンジンを
    // スキップして直接 passthrough する（1 キー分のみ）。
    // 例: Ctrl+J (tmux prefix) → n (next-window) で NICOLA が n を横取りするのを防ぐ。
    //
    // ImeOffRescueReplay はこのガードの対象外にしない（コードレビュー指摘3を
    // 踏まえ個別判断）。post-bypass は「直前に Ctrl+key bypass があった」場合
    // にのみ武装され、かつユーザー設定の `[[post_bypass]]` ルールが vk/proc/class
    // で一致した時だけ消費する狭いスコープの latch であり、IME OFF 救済窓の
    // 対象キー（無変換/変換系）と衝突する可能性は低い。NonText のように
    // 「フォーカス分類の誤判定で常時パススルーになる」広いガードとは性質が
    // 異なるため、ここは従来どおり適用する。
    if let Some(delivery) = consume_post_bypass(app, event, is_key_down) {
        return delivery;
    }

    // ImeOffRescueReplay（追加発見E）: 救済窓 defer を再度かけない専用経路
    // （`kp_run_inner` の `skip_rescue_defer=true` 相当）を通す。
    let result = if matches!(origin, KeyOrigin::ImeOffRescueReplay) {
        app.replay_ime_off_rescue_event(event)
    } else {
        app.process_key_event(event)
    };
    if matches!(result, CallbackResult::PassThrough) {
        // GJI 候補ウィンドウが表示中に Ctrl+key がパススルーされる際、
        // GJI が Ctrl+key を IME ショートカットとして横取りしないよう composition を
        // 先にキャンセルする（詳細は cancel_composition_and_arm_post_bypass_on_ctrl の doc）。
        cancel_composition_and_arm_post_bypass_on_ctrl(app, origin, event, is_key_down);
        app.executor.enqueue_reinject(event);
        KeyDelivery::Reinjected
    } else {
        KeyDelivery::Consumed
    }
}

/// `[[keymap]]` の KeyDown 新規照合（ADR-114 決定2 ステップ2）。マッチしなければ
/// `None` を返し、呼び出し元は通常の早期return分岐（`[[post_bypass]]` 等）へ
/// 進む。マッチすれば必ず `Some(KeyDelivery::Consumed)` を返す。
///
/// `deliver_key_event` 本体から分離することで cognitive complexity を抑える
/// （振る舞いは変更なし、`cancel_composition_and_arm_post_bypass_on_ctrl` と
/// 同じ理由）。
fn consume_keymap_match(
    app: &mut Runtime,
    event: awase::types::RawKeyEvent,
) -> Option<KeyDelivery> {
    let matched = app
        .platform_state
        .keymap
        .active_keymaps
        .find_match(event.vk_code, event.modifier_snapshot)?;
    app.platform_state.keymap.keymap_latch.latch(event.vk_code);
    if let Some(target_vk) = matched {
        // IME composition（GJI/MS-IME 問わず）が表示中に target_vk を
        // そのまま SendInput すると、IME が先にそれを自身のショートカットとして
        // 横取りしてしまう（`cancel_composition_and_arm_post_bypass_on_ctrl` が
        // Ctrl+key パススルー時に同じ問題へ対処しているのと同型の問題、ADR-114
        // 実装レビューで発見）。送信前に composition をキャンセルする。
        //
        // `[[keymap]]` の `from` は修飾子必須ではない（decision5 は Ctrl/Shift を
        // 主キーとしてのみ禁止し、修飾子として使うことは許可している）ため、
        // composition 中に無修飾キー1つでも `[[keymap]]` にマッチしうる——
        // このキャンセルは未確定文字列を破棄する（`cancel_ime_composition` が
        // 送る `CPS_CANCEL` の効果）。決定3で明記済み（ADR-114 実装レビュー
        // MA-2、意図的な仕様として受け入れる）。
        //
        // `is_composition_warm_in_tsf()`（GJI 専用の `gji_candidate_visible`）
        // だけでは MS-IME の composition を検出できない（ADR-114 実装レビュー
        // MA-1）ため、IME 非依存の `ime_composition_active_now()`
        // （`build_input_context` 等が使う既存の唯一の一般シグナル）を主に使い、
        // GJI 固有の検出タイミングの違いをカバーするため両方を OR で見る。
        if crate::tsf::observer::ime_composition_active_now()
            || app.platform.is_composition_warm_in_tsf()
        {
            cancel_composition(app, crate::output::ColdReason::KeymapTarget);
        }
        // SAFETY: メインスレッド（エンジンスレッド）から呼ばれる。
        unsafe {
            crate::output::held_modifiers::send_keymap_target(
                event.modifier_snapshot.ctrl,
                event.modifier_snapshot.shift,
                target_vk,
            );
        }
    }
    Some(KeyDelivery::Consumed)
}

/// IME composition をキャンセルする（`cancel_ime_composition` の呼び出し +
/// 内部状態更新をセットで行う唯一の場所）。呼び出し元が「キャンセルすべきか」を
/// 判定し、必要な場合にのみ呼ぶこと（`consume_keymap_match`/
/// `cancel_composition_and_arm_post_bypass_on_ctrl` の両方から呼ぶ、ADR-114
/// 実装レビュー指摘）。`reason` は journal・診断用（実装レビュー m-2、呼び出し元の
/// 文脈を正しく記録する——`[[keymap]]` 起因のキャンセルを `CtrlKeyBypass` として
/// 記録すると cold-start の連鎖を追う際に誤誘導する）。
fn cancel_composition(app: &mut Runtime, reason: crate::output::ColdReason) {
    // SAFETY: メインスレッド（エンジンスレッド）から呼ばれる。
    unsafe { super::cancel_ime_composition() };
    app.platform.on_composition_cancel(reason);
}

/// `CallbackResult::PassThrough` 確定時、Ctrl+非修飾キーによる bypass の直前に
/// GJI 候補ウィンドウの composition をキャンセルし、`[[post_bypass]]` ルールに
/// 一致すれば post-bypass latch を武装する。例: IME ON + め入力中 → Ctrl+J →
/// tmux prefix。Ctrl↓ではなく実際の Ctrl+非修飾キー↓ 時点でキャンセルすることで、
/// 修飾キーのみの押下時に composition を誤ってキャンセルしない。
///
/// origin を `KeyOrigin::Hook(PumpContext::Main)` に限定する（指摘4、低コスト案）。
/// 旧 drain 経路（DeferredReplay）はこのロジックを一切通っておらず、この限定は
/// その挙動と厳密に一致するため退行リスクがゼロ。世代照合による DeferredReplay
/// 対応の完全版は見送り、docs/known-bugs.md に起票のみ行う（gate 中に Ctrl+J 等が
/// 来ても composition キャンセルが効かない既知の隙間）。
///
/// `deliver_key_event` 本体から分離することで cognitive complexity を抑える
/// （振る舞いは変更なし）。
fn cancel_composition_and_arm_post_bypass_on_ctrl(
    app: &mut Runtime,
    origin: KeyOrigin,
    event: awase::types::RawKeyEvent,
    is_key_down: bool,
) {
    if !(matches!(origin, KeyOrigin::Hook(PumpContext::Main))
        && is_key_down
        && event.modifier_snapshot.ctrl
        && !event.vk_code.is_passthrough())
    {
        return;
    }
    // m-1（ADR-114 実装レビュー）: `is_composition_warm_in_tsf()` を1回だけ
    // 読み、ログと分岐判定の両方に同じ値を使う（`Ordering::Relaxed` の
    // atomic 読み取りのため、2回呼ぶと値が食い違いログと実際の動作が矛盾しうる）。
    let candidate_visible = app.platform.is_composition_warm_in_tsf();
    log::debug!(
        "[ctrl-check] vk=0x{:02X} candidate_visible={candidate_visible}",
        event.vk_code
    );
    if candidate_visible {
        cancel_composition(app, crate::output::ColdReason::CtrlKeyBypass);
        log::debug!(
            "[ctrl-bypass] IME composition cancelled (vk=0x{:02X})",
            event.vk_code
        );
    }
    // [[post_bypass]] ルールに一致する場合、次の非修飾キーを NICOLA スキップ。
    // tmux では prefix (Ctrl+J) 後に standalone n/p 等のコマンドキーを入力するため。
    arm_post_bypass_if_matches(app, event.vk_code);
}

/// Post-bypass latch（ADR-103 決定3）の消費判定。`Some` を返した場合、
/// 呼び出し元はその `KeyDelivery` を即座に return すること（`process_key_event`
/// へ進んではならない）。`deliver_key_event` 本体から分離することで
/// cognitive complexity を抑える。`WM_EXECUTE_EFFECTS` の post はここでは
/// 行わない（指摘8、`deliver_key_event` の doc 参照）。
fn consume_post_bypass(
    app: &mut Runtime,
    event: awase::types::RawKeyEvent,
    is_key_down: bool,
) -> Option<KeyDelivery> {
    if !app.platform_state.gate.post_bypass.is_armed() {
        return None;
    }
    let now = crate::win32::foreground_scope();
    match app.platform_state.gate.post_bypass.peek(now) {
        ScopeCheck::NotArmed => None,
        ScopeCheck::Expired => {
            log::debug!("[post-bypass] expired: 前景が変わった → latch 失効");
            None
        }
        ScopeCheck::Live(arm) => {
            log::debug!(
                "[post-bypass] live: armed_focus_epoch={} current_focus_epoch={} \
                 (診断専用、判定には使わない)",
                arm.armed_focus_epoch,
                app.platform_state.focus.focus_epoch,
            );
            let should_reinject = match classify_post_bypass_key(
                is_key_down,
                event.modifier_snapshot.ctrl,
                event.vk_code.classify_modifier().is_some(),
                event.vk_code.is_passthrough(),
            ) {
                PostBypassKey::KeepArmed => false,
                PostBypassKey::ConsumesPrefixSilently => {
                    app.platform_state.gate.post_bypass.disarm();
                    false
                }
                PostBypassKey::ConsumeAndPassthrough => {
                    app.platform_state.gate.post_bypass.disarm();
                    log::debug!(
                        "[post-bypass] consumed: vk=0x{:02X} → direct passthrough (NICOLA skipped)",
                        event.vk_code
                    );
                    true
                }
                PostBypassKey::PassthroughKeepArmed => true,
            };
            if should_reinject {
                app.executor.enqueue_reinject(event);
                Some(KeyDelivery::Reinjected)
            } else {
                None
            }
        }
    }
}

/// Ctrl+bypass 直後、`[[post_bypass]]` ルールに一致すれば post-bypass latch を
/// 武装する（ADR-103 決定3）。`deliver_key_event` 本体から分離することで
/// cognitive complexity を抑える。
fn arm_post_bypass_if_matches(app: &mut Runtime, vk: VkCode) {
    let proc = app.platform.focus.process_name();
    let cls = app.platform.focus.class_name();
    if !app
        .post_bypass_rules
        .iter()
        .any(|r| r.matches(vk, proc, cls))
    {
        return;
    }
    let scope = crate::win32::foreground_scope();
    if scope.is_valid() {
        app.platform_state.gate.post_bypass.arm(
            scope,
            crate::state::platform_state::PostBypassArm {
                armed_focus_epoch: app.platform_state.focus.focus_epoch,
            },
        );
        log::debug!(
            "[ctrl-bypass] post_bypass armed (proc={proc:?} class={cls:?} scope={scope:?})"
        );
    } else {
        log::debug!("[ctrl-bypass] post_bypass not armed: foreground scope unavailable");
    }
}

/// WM_KEY_FROM_HOOK ハンドラ — フックスレッドから転送された物理キーイベントを処理する
pub(crate) fn handle_wm_key_from_hook(app: &mut Runtime, event: awase::types::RawKeyEvent) {
    begin_key_batch(app);
    let delivery = deliver_key_event(
        app,
        event,
        KeyOrigin::Hook(crate::runtime::engine_window::current_pump_context()),
    );
    // WM_EXECUTE_EFFECTS の post は呼び出し元の責務（指摘8、deliver_key_event の doc 参照）。
    post_effects_if_reinjected(delivery);
    recover_pending_drain_request();
}

/// WM_TIMER ハンドラ
#[expect(clippy::cognitive_complexity)]
pub(crate) unsafe fn handle_wm_timer(
    app: &mut Runtime,
    wparam: usize,
    msg: &windows::Win32::UI::WindowsAndMessaging::MSG,
) {
    use windows::Win32::UI::WindowsAndMessaging::DispatchMessageW;
    let logical_id = app.platform.timer.resolve(wparam);
    match logical_id {
        Some(id) if id == TIMER_IME_REFRESH => {
            if app.platform_state.gate.sync_key_gate.is_active()
                || app.platform_state.gate.sync_key_gate.has_deferred_keys()
            {
                app.process_deferred_keys();
            }
            // async タスクをスポーン（with_app を解放してから fetch）
            app.spawn_ime_refresh();
        }
        Some(id) if id == TIMER_POWER_RESUME => {
            app.platform.timer.kill(TIMER_POWER_RESUME);
            log::info!("Power resume recovery");
            app.invalidate_engine_context(ContextChange::InputLanguageChanged);
            app.platform_state.focus.focus_kind = FocusKind::Undetermined;
            app.schedule_ime_refresh(500);
        }
        Some(id) if id == TIMER_OUTPUT_GUARD => {
            let outcomes = app
                .executor
                .on_output_guard_timer(&mut app.platform, &app.platform_state.ime);
            app.dispatch_outcomes(outcomes);
        }
        Some(id) if id == TIMER_TSF_PROBE => {
            // log_composition_probe が with_app_ref (共有借用) を使うが、
            // ここでは RUNTIME が排他借用中で BorrowError になる。
            // diagnostic_snapshot を事前に取得してスレッドローカルに渡す。
            let snap = app.diagnostic_snapshot();
            crate::ime_diagnostic::set_tsf_probe_snap(snap);
            app.platform.advance_tsf_probe();
            crate::ime_diagnostic::clear_tsf_probe_snap();
            for entry in app.platform.drain_journal_entries() {
                app.platform_state.ime.journal.absorb(entry);
            }
        }
        Some(id) if id == TIMER_TSF_GATE => {
            app.platform.timer.kill(TIMER_TSF_GATE);
            let held = app.platform.on_tsf_warmup_timeout();
            if !held.is_empty() {
                log::debug!(
                    "[tsf-gate-timeout] draining {} held keys via INPUT_DEFER",
                    held.len()
                );
                crate::INPUT_DEFER.replay_later(held);
            }
        }
        Some(id) if id == crate::TIMER_IME_OFF_RESCUE => {
            if let Some(pending_event) = app.take_ime_off_rescue_pending() {
                log::info!(
                    "[ime-off-rescue] 50ms timer expired → 保留 vk=0x{:02X} を IME OFF として発火",
                    pending_event.vk_code
                );
                // deliver_key_event（単一入口）経由に統合する（追加発見E）。
                // 以前は Runtime::replay_ime_off_rescue_event を直接呼んでおり、
                // NonText focus パススルー・post-bypass latch 消費等を素通りしていた。
                //
                // begin_key_batch(app) をここでも呼ぶ（コードレビュー指摘4）。
                // 「バッチ境界で1回だけ resync チェックする」契約
                // （begin_key_batch の doc 参照）の対象に、この TIMER 分岐も
                // handle_wm_key_from_hook・handle_wm_drain_output_queue と並ぶ
                // 3箇所目として含める。
                begin_key_batch(app);
                let delivery = deliver_key_event(app, pending_event, KeyOrigin::ImeOffRescueReplay);
                post_effects_if_reinjected(delivery);
            }
        }
        Some(id) if id == TIMER_GJI_LONG_IDLE => {
            app.platform.timer.kill(TIMER_GJI_LONG_IDLE);
            app.platform.gji_on_timer_long_idle();
            for entry in app.platform.drain_journal_entries() {
                app.platform_state.ime.journal.absorb(entry);
            }
        }
        Some(id) if id == crate::TIMER_FOCUS_RESYNC => {
            // フォーカス復帰後 resync のハード期限（report `01M0VGJ2M5KQHD1D9V7HAMBHNT`）。
            // resync（conv 読み取り）がこの期限より先に gate を閉じていれば
            // `open_if_current` は世代不一致/既 close で false を返し、ここでは
            // 何もしない（二重 drain post を防ぐ）。
            app.platform.timer.kill(crate::TIMER_FOCUS_RESYNC);
            let generation = crate::focus_resync::FOCUS_RESYNC.current_generation();
            if crate::focus_resync::FOCUS_RESYNC.open_if_current(generation) {
                log::debug!(
                    "[focus-resync] ハード期限 {}ms 到達 → defer 中のキーを drain",
                    crate::tuning::FOCUS_RESYNC_DEADLINE_MS
                );
                if crate::state::focus_resync_policy::should_post_drain(
                    crate::tsf::probe_bridge::OUTPUT_GATE.is_active(),
                ) {
                    crate::tsf::probe_bridge::post_drain_output_queue();
                }
            }
        }
        Some(id) if id == TIMER_HOOK_WATCHDOG => {
            let last_activity = hook::hook_alive_tick_ms();
            let now = hook::current_tick_ms();
            let stale_ms = now.saturating_sub(last_activity);
            if stale_ms > 5000 {
                log::warn!("Hook watchdog: no activity for {stale_ms}ms");
            } else {
                log::trace!("Hook watchdog: last activity {stale_ms}ms ago");
            }
            crate::hook_channel::recover_stuck_wake_if_needed();
            recover_pending_drain_request();
        }
        Some(timer_id) => {
            log::debug!("WM_TIMER fired: logical_id={timer_id}");
            // OUTPUT_GATE active 中はエンジンタイマー（TIMER_PENDING/TIMER_SPECULATIVE）を
            // drain 後に延期する。
            // OUTPUT_GATE active 期間中、後続キー（親指キー等）は INPUT_DEFER にキューされる。
            // PendingChar タイマーをそのまま発火させると、chord パートナー（親指キー）が
            // drain で処理される前に PendingChar → Idle 遷移が完了してしまい、
            // NICOLA 同時打鍵判定が失敗する（例: K+右親指 = の が き になる）。
            // WM_DRAIN_OUTPUT_QUEUE はユーザー定義メッセージのため WM_TIMER より優先度が高く、
            // drain は必ず再アームタイマーより先に実行される。drain で chord パートナーが
            // 処理されれば engine が Kill(timer_id) を出すため、replay 時に is_active=false と
            // なりスキップされる。
            //
            // BUG-77 code review 追補: `FOCUS_RESYNC` の gate も同じ形の危険を持つ。
            // resync 対象キーが `INPUT_DEFER` へ退避されている間、チョードの相方
            // （親指キー等）は通常どおり FSM に feed され続けるため、この延期を
            // OUTPUT_GATE だけに限定すると、resync 完了/期限（最大
            // `FOCUS_RESYNC_DEADLINE_MS`）前に相方の TIMER_PENDING/TIMER_SPECULATIVE
            // が発火し、同時打鍵判定が失敗してチョードが2つのリテラル文字に分裂しうる
            // （`OutputGate` と全く同じ壊れ方）。`deferred_engine_timers` の replay は
            // `handle_wm_drain_output_queue` が gate の種類を問わず必ず行うため、
            // ここで gate 判定を拡張するだけで両ゲートに対して正しく機能する。
            if crate::OUTPUT_GATE.is_active() || crate::focus_resync::FOCUS_RESYNC.is_gate_active()
            {
                log::debug!(
                    "[engine-timer] OUTPUT_GATE/FOCUS_RESYNC gate active → logical_id={timer_id} (os_id={wparam}) を drain 後に延期"
                );
                app.ime_coordinator
                    .deferred_engine_timers
                    .push((timer_id, wparam));
                return;
            }
            let mut modifiers = unsafe { crate::observer::focus_observer::read_os_modifiers() };
            // Alt なりすまし中の補正（`crate::hook::is_alt_impersonation_active` doc・
            // `Runtime::build_ctx` の同様の補正参照）。タイマー満了経路
            // （timeout_pending_thumb 等）でもここを直さないとなりすましが機能しない。
            if hook::is_alt_impersonation_active() {
                modifiers.alt = false;
            }
            let (left_thumb_down, right_thumb_down) = hook::thumb_down_timestamps();
            let ctx = super::build_input_context(
                app.platform_state.ime.effective_open(),
                app.platform_state.ime.input_mode(),
                app.platform_state.ime.belief.is_japanese_ime(),
                crate::tsf::observer::ime_composition_active_now(),
                &modifiers,
                left_thumb_down,
                right_thumb_down,
            );
            let state_before = app.engine.debug_state_label();
            let decision = app.engine.on_timeout(timer_id, &ctx);
            notify_if_solo_off_triggered(app);
            let state_after = app.engine.debug_state_label();
            app.platform_state
                .ime
                .journal
                .record(crate::journal::JournalEntry::TimerFired {
                    timer_id,
                    state_before,
                    state_after,
                });
            app.execute_decision(decision);
        }
        None => {
            // 未知のタイマー → win32-async や外部 HWND タイマーかもしれないので dispatch
            // SAFETY: msg was filled by GetMessageW and is valid for the calling thread.
            DispatchMessageW(&raw const *msg);
        }
    }
    recover_pending_drain_request();
}

/// WM_EXECUTE_EFFECTS ハンドラ
pub(crate) unsafe fn handle_wm_execute_effects(app: &mut Runtime) {
    let outcomes = app
        .executor
        .drain_deferred(&mut app.platform, &app.platform_state.ime);
    app.dispatch_outcomes(outcomes);
    // H-4-a: Output が send_keys 中に積んだ RuntimeRequest を一括処理する。
    app.drain_runtime_requests();
    recover_pending_drain_request();
}

// ── 非同期 IME apply 完了の WM ルーティング ──────────────────────────────────
//
// sync path は `BatchResult.sync_outcomes` → `dispatch_outcomes` → `on_ime_apply_complete`
// に合流する。async path（ImmCross）も同じ単一入口へ合流させるため、spawn_local の
// future 内で `with_app` を直接握らず、完了 outcome を WM_ASYNC_IME_APPLY_COMPLETE として
// メインスレッドのメッセージループへ投函する。`(open, generation, outcome)` は
// wparam/lparam にパックする。

/// `ImeOpenOutcome` を lParam 用の整数にエンコードする。
///
/// 網羅 match のため、variant 追加時はここがコンパイルエラーになり追従を強制される。
const fn encode_outcome(outcome: ImeOpenOutcome) -> isize {
    match outcome {
        ImeOpenOutcome::Applied => 0,
        ImeOpenOutcome::FallbackSent => 1,
        ImeOpenOutcome::AlreadyMatched => 2,
        ImeOpenOutcome::Failed => 3,
        ImeOpenOutcome::UnsafeToToggle => 4,
        ImeOpenOutcome::NotOwned => 5,
    }
}

/// `encode_outcome` の逆変換。未知値は apply を行わない安全側の `UnsafeToToggle` に倒す。
fn decode_outcome(value: isize) -> ImeOpenOutcome {
    match value {
        0 => ImeOpenOutcome::Applied,
        1 => ImeOpenOutcome::FallbackSent,
        2 => ImeOpenOutcome::AlreadyMatched,
        3 => ImeOpenOutcome::Failed,
        4 => ImeOpenOutcome::UnsafeToToggle,
        5 => ImeOpenOutcome::NotOwned,
        other => {
            log::error!("WM_ASYNC_IME_APPLY_COMPLETE: unknown outcome code {other}");
            ImeOpenOutcome::UnsafeToToggle
        }
    }
}

/// async IME apply の完了を Runtime の単一入口へ届ける WM を投函する。
///
/// spawn_local の future（メインスレッド上でポーリングされる）から呼ぶこと。
/// `with_app` を握らずメッセージループ経由で `on_ime_apply_complete` に合流させる。
///
/// `reason` は wparam の bit1 にエンコードする（BUG-34 横展開 D、2026-08-19）。
/// 以前はこの経路の唯一の生成元が `executor.rs::dispatch_ime_set_open`
/// （常に `EngineDecision`）だったため固定値にしていたが、
/// `try_force_on_bootstrap`（`Bootstrap`）が2つ目の生成元として加わったため、
/// 呼び出し元が申告した reason を実際に運ぶ必要がある。**`EngineDecision` と
/// `Bootstrap` の2値のみエンコードする**（1 bit）。この async 経路に将来
/// 別の `OpenApplyReason` を渡す呼び出し元を追加する場合は、この関数と
/// `decode_reason` のビット幅を拡張すること（さもないと未知の reason が
/// 静かに `EngineDecision` に丸められる）。
pub(crate) fn post_async_ime_apply_complete(
    open: bool,
    outcome: ImeOpenOutcome,
    generation: Option<crate::state::ApplyGeneration>,
    reason: crate::state::ime_event::OpenApplyReason,
) {
    // ADR-106 決定1: `ApplyGeneration` は `NonZeroU64` のため `to_wire` の
    // `0 = None` エンコードは正当な generation 値と絶対に衝突しない
    // （旧 `generation.unwrap_or(0)` は `next_seq()` 由来の `generation == 0`
    // が bootstrap 経路で実際に払い出されうる番兵衝突を抱えていた）。
    let generation = crate::state::ApplyGeneration::to_wire(generation);
    let reason_bit = usize::from(matches!(
        reason,
        crate::state::ime_event::OpenApplyReason::Bootstrap
    ));
    let wparam = ((generation as usize) << 2) | (reason_bit << 1) | usize::from(open);
    // エンジンスレッド上（win32-async の spawn_local タスク完了）から呼ばれるため
    // ログ出力してよい。post が失敗すると ImmCross の非同期 SetOpen 完了通知が
    // 握りつぶされ、pending generation の IME open belief が未解決のまま残る
    // （BUG-09 と同系統の belief/実状態乖離）。再試行機構は持たないため、
    // 発生したことをログに残すだけに留める（Opus敵対的レビュー指摘、2026-08-26。
    // 残存リスクとして docs/known-bugs.md にも記録済み）。
    if !crate::win32::post_to_main_thread_with(
        crate::WM_ASYNC_IME_APPLY_COMPLETE,
        wparam,
        encode_outcome(outcome),
    ) {
        log::warn!(
            "[async-ime-apply] WM_ASYNC_IME_APPLY_COMPLETE の post に失敗しました \
             (open={open} generation={generation} reason={reason:?}) — \
             このIME適用完了通知は失われ、pending generation が未解決のまま残ります"
        );
    }
}

/// [`post_async_ime_apply_complete`] の reason bit の逆変換。
fn decode_reason(wparam: usize) -> crate::state::ime_event::OpenApplyReason {
    if (wparam >> 1) & 1 != 0 {
        crate::state::ime_event::OpenApplyReason::Bootstrap
    } else {
        crate::state::ime_event::OpenApplyReason::EngineDecision
    }
}

/// WM_ASYNC_IME_APPLY_COMPLETE ハンドラ
///
/// ImmCross async apply の完了通知。sync path の `sync_outcomes` と対称に、
/// generation 照合を含む単一入口 `on_ime_apply_complete`（B+C+D+E）へ合流する。
pub(crate) fn handle_wm_async_ime_apply_complete(app: &mut Runtime, wparam: usize, lparam: isize) {
    let open = (wparam & 1) != 0;
    let reason = decode_reason(wparam);
    let generation = crate::state::ApplyGeneration::from_wire((wparam >> 2) as u64);
    let outcome = decode_outcome(lparam);
    if outcome == ImeOpenOutcome::Failed {
        log::warn!("apply_ime_open({open}) failed (async)");
    }
    app.on_ime_apply_complete(open, outcome, generation, reason);
}

/// WM_GJI_REINIT_RETRY_COMPLETE ハンドラ。
pub(crate) fn handle_wm_gji_reinit_retry_complete(app: &mut Runtime, wparam: usize, lparam: isize) {
    let Ok(token) = u32::try_from(wparam) else {
        log::warn!("[chrome-reinit-retry] completion token out of range: {wparam}");
        return;
    };
    let Some(status) = crate::output::GjiReinitPollStatus::decode(lparam) else {
        log::warn!("[chrome-reinit-retry] unknown completion status: {lparam}");
        return;
    };
    app.platform.complete_gji_reinit_retry(token, status);
    app.drain_runtime_requests();
}

/// WM_PANIC_RESET ハンドラ
pub(crate) unsafe fn handle_wm_panic_reset(app: &mut Runtime) {
    app.panic_reset();
}

/// MS-IME レジストリの `KeyAssignmentCtrlSpace`/`KeyAssignmentShiftSpace`
/// （ADR-092 決定D Step4a）と `KeyAssignmentMuhenkan`/`KeyAssignmentHenkan`
/// （決定A・決定D Step4b）を読み、`Engine` へ反映する。呼び出し元は2つ:
/// - `sync_ime_kind_from_observation` から MS-IME 確定のたびに呼ばれる
///   （決定C R2、計算は毎回やり直す）。
/// - `app/mod.rs::reload_config`（設定リロード時、ADR-092 Step4b前提条件3の
///   stale化対策——MS-IME 単独ユーザーはセッション中 IME 種別確定イベントが
///   再発生しないため、設定リロード時にも再読みしないと、ユーザーが
///   Windows の設定画面でレジストリを変更してもセッション中反映されない）。
///
/// `Engine` 側は `special_keys.ime_toggle`（手動設定）の内容に関わらず常に
/// `set_ime_toggle_auto_keys` の結果も併用する（2026-08-16 ユーザー判断、
/// 明示 ∪ 自動）ため、ここでは無条件に呼んでよい。`set_muhenkan/henkan_delegate_to_open_axis`は
/// `muhenkan_solo_tap_dedicated_fn_key`（専用Fnキー）が設定されていれば
/// `Engine`側でそちらが優先されるため、こちらも無条件に呼んでよい。
pub(crate) fn sync_ime_toggle_auto_detect(app: &mut Runtime) {
    let toggle_assignment = crate::msime_key_assignment::read_toggle_assignment_from_registry();
    log::info!("[msime-keyassign] toggle assignment: {toggle_assignment:?}");
    let skip_shift_space = app.space_is_thumb_key();
    app.engine
        .set_ime_toggle_auto_keys(toggle_assignment.to_combos(skip_shift_space));

    let delegate_assignment =
        crate::msime_key_assignment::read_delegate_to_open_axis_assignment_from_registry();
    log::info!("[msime-keyassign] delegate-to-open-axis assignment: {delegate_assignment:?}");
    app.engine
        .set_muhenkan_delegate_to_open_axis(delegate_assignment.muhenkan);
    app.engine
        .set_henkan_delegate_to_open_axis(delegate_assignment.henkan);
}

/// IME 種別を観測値から pull し、warmup 戦略切替 + MS-IME 割当てチェックに反映する。
///
/// IME 種別に依存する副作用の**単一の合流点**。呼び出し元は2つ:
/// - [`handle_wm_ime_kind_changed`] — gji-monitor の CLSID 検出変化時（通常経路）
/// - `run_message_loop` 起動時の pull 同期 — gji-monitor がメッセージループ開始前に
///   post した初回 `WM_IME_KIND_CHANGED` が消失するレースの保険（BUG-09）。
///   実機では保険経路だけが走るケースが常態のため、副作用をここに集約しないと
///   「戦略は切り替わるのに割当てチェックが走らない」片肺になる（2026-07-06 実発生）。
pub(crate) fn sync_ime_kind_from_observation(app: &mut Runtime, source: &str) {
    let obs = crate::tsf::observer::tsf_obs();
    let kind = obs.active_ime_kind();
    let detected = obs.ime_kind_detected();
    log::info!("[runtime] IME kind sync ({source}): {kind:?} (detected={detected})");
    app.platform.output.set_active_ime_kind(kind);
    if matches!(
        kind,
        crate::tsf::observer::ActiveImeKind::GoogleJapaneseInput
    ) && app.platform_state.ime.model().applied.applied_open() == Some(true)
    {
        let mode = app.platform.output.injection_mode;
        log::debug!("[runtime] GJI warmup FSM sync: applied_open=true → ImeOn");
        app.platform.gji_on_ime_on(mode);
        for entry in app.platform.drain_journal_entries() {
            app.platform_state.ime.journal.absorb(entry);
        }
    }

    // GJI 検出時、config1.db から IME ON/OFF/トグルキー（ADR-092 決定D
    // Step4c）を自動判定する。MS-IME 割当てチェックと対称に、この「IME
    // 種別に依存する副作用の単一の合流点」に置き、同じ理由で detected を
    // 見る（未検出時の active_ime_kind() が安全デフォルトとして
    // MicrosoftIme を返す実装詳細に暗黙に依存せず、明示的にゲートする）。
    //
    // **MS-IME 側（次のブロック）より先に呼ぶこと（Opus コードレビュー
    // 指摘、意図的な順序）**: `ime_toggle_auto` は GJI/MS-IME 両方の
    // 自動検出が共有する`Engine`フィールドで、GJI 離脱時に
    // `sync_gji_charset_autodetect` が（自分専用の`ime_on_auto`/
    // `ime_off_auto`と一緒に）`ime_toggle_auto`も解除する。GJI→MS-IME の
    // 遷移で MS-IME 側が先に新しい値を設定してしまうと、後から走る GJI
    // 離脱処理がその値を上書き消去してしまう（実際に発生していた回帰、
    // 詳細は`gji_charset_autodetect.rs`のコメント参照）。GJI 側を先に
    // 走らせれば、GJI→MS-IME遷移時は「GJI離脱で3リストとも解除→直後に
    // MS-IME側が`ime_toggle_auto`を新しい値で上書き」という正しい順序に
    // なる。
    //
    // 専用Fnキー変換（ADR-091 §D3.2）の自動判定・設定支援ポップアップ・
    // config1.db書き込みは、実験的機能のまま撤去し忘れて出荷されていた
    // ため2026-09-02に全撤去した（未実装の再検討はADR-091追補参照）。
    // `muhenkan_solo_tap_dedicated_fn_key` の手動設定（config.toml）による
    // 内部配線は残っている。
    crate::gji_charset_autodetect::sync_gji_charset_autodetect(
        app,
        detected
            && matches!(
                kind,
                crate::tsf::observer::ActiveImeKind::GoogleJapaneseInput
            ),
    );

    // MS-IME と確定したら、無変換/変換キーの IME オン/オフ割り当て（awase と
    // 競合し belief 乖離を起こす）をチェックして解除を案内する
    // （ポップアップは同一内容につき一度、内容が変われば再警告）。
    // detected を見るのは、未検出時の active_ime_kind() が安全デフォルトとして
    // MicrosoftIme を返すため — これを見ないと GJI ユーザーの起動時にも誤発動する。
    if detected && matches!(kind, crate::tsf::observer::ActiveImeKind::MicrosoftIme) {
        crate::msime_key_assignment::check_and_warn();
        sync_ime_toggle_auto_detect(app);
    }
}

/// WM_IME_KIND_CHANGED ハンドラ
///
/// GJI モニタースレッドが IME 種別の変化（GJI 検出 / 消失）を検知したときに呼ばれる。
pub(crate) unsafe fn handle_wm_ime_kind_changed(app: &mut Runtime) {
    sync_ime_kind_from_observation(app, "WM_IME_KIND_CHANGED");
}

/// WM_DUPLICATE_INSTANCE ハンドラ
pub(crate) unsafe fn handle_wm_duplicate_instance(app: &mut Runtime) {
    log::info!("Duplicate instance notification received");
    app.platform
        .tray
        .show_balloon("awase", "awase はすでに起動しています");
}

/// WM_POWERBROADCAST ハンドラ。
///
/// PBT_APMRESUMESUSPEND (7) と PBT_APMRESUMEAUTOMATIC (18) の両方を resume と
/// みなす（ユーザ操作 / 自動復帰の両方をカバー）。
pub(crate) unsafe fn handle_wm_powerbroadcast(app: &mut Runtime, pbt: usize) {
    use windows::Win32::UI::WindowsAndMessaging::{PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND};
    if pbt == PBT_APMRESUMESUSPEND as usize || pbt == PBT_APMRESUMEAUTOMATIC as usize {
        log::info!("Power resume detected (PBT=0x{pbt:02X}), scheduling deferred recovery");
        app.platform.timer.kill(TIMER_IME_REFRESH);
        app.platform
            .timer
            .set(TIMER_POWER_RESUME, std::time::Duration::from_secs(3));
    }
}

/// WM_WTSSESSION_CHANGE ハンドラ
pub(crate) unsafe fn handle_wts_session_change(app: &mut Runtime, session_event: u32) {
    const WTS_SESSION_LOCK: u32 = 7;
    const WTS_SESSION_UNLOCK: u32 = 8;
    match session_event {
        WTS_SESSION_LOCK => {
            log::info!("Session locked, flushing engine state");
            app.invalidate_engine_context(ContextChange::FocusChanged);
        }
        WTS_SESSION_UNLOCK => {
            log::info!("Session unlocked, scheduling deferred recovery");
            // ロック中 (Secure Desktop) は WH_KEYBOARD_LL にイベントが届かないため、
            // ロック直前に押されていた物理キーの KeyUp が失われうる。PHYSICAL_KEY_STATE は
            // OR で左右を合成するため、片側が stuck するだけで mods.shift/ctrl が恒久的に
            // true になる（2026-07-09 実機で確認）。アンロック時点では物理キーはどれも
            // 離されていると仮定してよいため、無条件でリセットする。
            hook::reset_physical_key_state();
            // [[keymap]] latch も同じ理由で解放する（ADR-114 決定4「latch
            // 漏れ対策」経路5）。ロック中に失われた KeyUp が latch を stuck
            // させたままになるのを防ぐ。
            app.platform_state.keymap.keymap_latch.release_all();
            app.platform.timer.kill(TIMER_IME_REFRESH);
            app.platform
                .timer
                .set(TIMER_POWER_RESUME, std::time::Duration::from_secs(3));
        }
        _ => {}
    }
}

/// WM_INPUTLANGCHANGE ハンドラ
pub(crate) unsafe fn handle_wm_inputlangchange(app: &mut Runtime) {
    log::info!("Input language changed, flushing pending state");
    app.invalidate_engine_context(ContextChange::InputLanguageChanged);
    app.refresh_ime_state_cache();
    check_keyboard_layout_on_change();
}

/// WM_FOCUS_KIND_UPDATE ハンドラ
pub(crate) unsafe fn handle_wm_focus_kind_update(app: &mut Runtime, wparam: usize, lparam: isize) {
    let kind_u8 = wparam as u8;
    let app_kind_u8 = (wparam >> 8) as u8;
    let result_hwnd = HWND(lparam as *mut _);
    let kind = FocusKind::from_u8(kind_u8);

    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    // ── UIA 非同期分類の適用は無効化されている（BUG-12）────────────────────────
    //
    // この handler は BUG-09（post_to_main_thread 誤配送）の修正まで一度も実行された
    // ことがなく、配送を直した途端に 2 段階の実害が露出した:
    //
    // 1. キャッシュキー取り違え（BUG-11）: 遅延した platform.focus からキーを取り、
    //    Alt+Tab メニューの NonText を Edge のキーでキャッシュ → Edge 永久 NonText。
    // 2. キー粒度の構造的不一致（BUG-12、BUG-11 修正後も再発 2026-07-06T05:28 実機）:
    //    帰属を result_hwnd から正しく導出しても、ブラウザ（Chrome_WidgetWin_1）の
    //    focus kind は「ウィンドウ内のどの要素にフォーカスがあるか」で毎秒変わる。
    //    ページ本文フォーカス時の**正しい** NonText を (pid, class) でキャッシュした
    //    瞬間、テキスト欄に移っても再分類イベントが来ない（ウィンドウ内クリックは
    //    フォーカス変更として観測できない）ため Edge 全体が永久 NonText になり、
    //    全キーがエンジン素通し（「IME ON・Engine OFF」症状）。
    //
    // UIA 結果を安全に適用するには hwnd 粒度 + ウィンドウ内フォーカス要素の追跡
    // （UIA FocusChanged イベント購読等）が必要で、(pid, class) キャッシュ設計とは
    // 両立しない。それまでは配送修正前の実績ある挙動（結果は届くが適用しない）に
    // 意図的に戻す。sync 分類（既知クラス・WS_EX_NOIME・MSAA）は従来どおり機能する。
    let _ = app;
    if GetGUIThreadInfo(0, &raw mut info).is_ok() && info.hwndFocus != result_hwnd {
        log::debug!("UIA result for stale hwnd, ignoring");
    } else {
        log::debug!(
            "UIA async result received (kind={kind:?} app_kind_u8={app_kind_u8}) — \
             BUG-12 により適用せずログのみ"
        );
    }
}

/// WM_HOTKEY ハンドラ (HOTKEY_ID_TOGGLE)
pub(crate) unsafe fn handle_wm_hotkey_toggle(app: &mut Runtime) {
    app.toggle_engine();
}

/// WM_HOTKEY ハンドラ (HOTKEY_ID_FOCUS_OVERRIDE)
pub(crate) unsafe fn handle_wm_hotkey_focus_override(app: &mut Runtime) {
    app.toggle_app_override();
}

/// WM_APP (トレイメッセージ) ハンドラ
pub(crate) unsafe fn handle_wm_app_tray(hwnd: HWND, lparam: LPARAM) {
    log::debug!(
        "WM_APP received: hwnd={:?} lparam=0x{:016X}",
        hwnd,
        lparam.0
    );
    let (layout_names, current_layout_name, kana_lock_warned): (Vec<String>, String, bool) =
        with_app_ref(|app| {
            (
                app.layouts.iter().map(|e| e.name.clone()).collect(),
                app.platform.tray.current_layout_name().to_string(),
                app.platform.tray.kana_lock_warned(),
            )
        })
        .unwrap_or_default();
    tray::handle_tray_message(
        hwnd,
        lparam,
        &layout_names,
        &current_layout_name,
        crate::is_elevated(),
        kana_lock_warned,
    );
}

/// WM_RELOAD_CONFIG ハンドラ
pub(crate) fn handle_wm_reload_config() {
    log::info!("Config reload requested via WM_RELOAD_CONFIG");
    reload_config();
}

/// WM_COMMAND ハンドラ
pub(crate) unsafe fn handle_wm_command(wparam: WPARAM) {
    // トレイメニューの IME コマンドは、メニュー表示前に捕捉したウィンドウ
    // （`tray::menu_target_hwnd()`）を対象にする。捕捉できていなければ
    // `GetForegroundWindow()` にフォールバックする（メニュー選択後の呼び出し時点の
    // フォアグラウンドはトレイ自身の可能性が高いため、あくまで最終手段）。
    // 理由は `ime::set_ime_open_for_target` の doc を参照。
    let ime_target = tray::menu_target_hwnd().or_else(|| {
        use crate::win32::HwndExt as _;
        windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow().non_null()
    });

    match tray::handle_tray_command(wparam) {
        Some(tray::TrayCommand::Settings) => launch_settings(),
        Some(tray::TrayCommand::RestartAdmin) => tray::restart_as_admin(),
        Some(tray::TrayCommand::Toggle) => {
            let _ = with_app(Runtime::toggle_engine);
        }
        Some(tray::TrayCommand::Exit) => {
            use windows::Win32::UI::WindowsAndMessaging::WM_QUIT;
            crate::request_quit();
            post_to_main_thread(WM_QUIT);
        }
        Some(tray::TrayCommand::SelectLayout(index)) => {
            let _ = with_app(|app| app.switch_layout(index));
        }
        Some(tray::TrayCommand::ToggleAutoStart) => tray::handle_autostart_toggle(),
        Some(tray::TrayCommand::Restart) => tray::restart_self(),
        Some(tray::TrayCommand::About) => tray::show_about_dialog(),
        Some(tray::TrayCommand::BugReport) => {
            let ime_kind = current_bug_report_ime_kind();
            let Some((dump_result, diagnostics)) = with_app(|app| {
                for entry in app.platform.drain_journal_entries() {
                    app.platform_state.ime.journal.absorb(entry);
                }
                app.platform_state
                    .ime
                    .journal
                    .record(crate::journal::JournalEntry::ClockAnchor {
                        tick_ms: hook::current_tick_ms(),
                        hook_us: hook::now_timestamp_us(),
                    });
                app.platform_state
                    .ime
                    .journal
                    .record(crate::journal::JournalEntry::DumpTriggered);
                let dump_result = app
                    .platform_state
                    .ime
                    .journal
                    .dump_to_file_capped(crate::bug_report::LOG_EXCERPT_MAX_BYTES);
                (dump_result, current_bug_report_diagnostics(app))
            }) else {
                log::error!("[bug-report] runtime unavailable");
                return;
            };
            let diagnostics_path = match write_bug_report_diagnostics(&diagnostics) {
                Ok(path) => Some(path),
                Err(e) => {
                    log::warn!("[bug-report] diagnostics dump failed: {e}");
                    None
                }
            };
            let app_log_path = crate::app::bug_report_log_path();
            let app_log_path = app_log_path.exists().then_some(app_log_path.as_path());
            match dump_result {
                Ok(path) => {
                    launch_bug_report(&path, ime_kind, diagnostics_path.as_deref(), app_log_path);
                }
                Err(e) => {
                    log::error!("[bug-report] journal dump failed: {e}");
                    let _ = with_app(|app| {
                        app.platform
                            .tray
                            .show_balloon("awase bug report", "journal の添付準備に失敗しました");
                    });
                }
            }
        }
        Some(tray::TrayCommand::CapsLock) => {
            crate::ime::toggle_caps_lock();
        }
        Some(tray::TrayCommand::ResetState) => {
            let caps_lock_on = crate::ime::is_caps_lock_on();
            if caps_lock_on {
                crate::ime::toggle_caps_lock();
            }
            // 2026-08-17、ADR-094 で charset 軸の追跡を撤去したのに伴い、書き込み
            // マスクから IME_CMODE_ROMAN を外した（ユーザーの明示決定。romaji/JISかな
            // の区別自体は ADR-091 決定3 §D3.4 により別軸として残るが、この
            // リセット操作の対象からは外す）。
            if let Some(hwnd) = ime_target {
                let _ = crate::ime::set_ime_mode_for_target(
                    hwnd,
                    true,
                    crate::imm::IME_CMODE_NATIVE | crate::imm::IME_CMODE_FULLSHAPE,
                    crate::imm::IME_CMODE_KATAKANA,
                );
            }
            // 無変換ソロ連打の緊急停止（ADR-055 追補）等で user_enabled が false に
            // なっていても、この操作で必ず Engine ON まで復帰させる。
            let _ = with_app(Runtime::force_engine_on);
        }
        Some(tray::TrayCommand::KanaLockHelp) => tray::show_kana_lock_help_dialog(),
        Some(tray::TrayCommand::ClearImmCache) | None => {}
    }
}

fn current_bug_report_ime_kind() -> crate::bug_report::BugReportImeKind {
    let obs = crate::tsf::observer::tsf_obs();
    if !obs.ime_kind_detected() {
        return crate::bug_report::BugReportImeKind::Unknown;
    }
    match obs.active_ime_kind() {
        crate::tsf::observer::ActiveImeKind::GoogleJapaneseInput => {
            crate::bug_report::BugReportImeKind::Gji
        }
        crate::tsf::observer::ActiveImeKind::MicrosoftIme => {
            crate::bug_report::BugReportImeKind::MsIme
        }
    }
}

fn current_bug_report_diagnostics(app: &Runtime) -> crate::bug_report::BugReportDiagnostics {
    let (is_japanese, lang_id) = crate::ime::keyboard_layout_info();
    let now_ms = hook::current_tick_ms();
    let resources = process_resource_snapshot();
    let state_snapshot = crate::bug_report::BugReportStateSnapshot {
        desired_open: app.platform_state.ime.desired_open(),
        effective_open: app.platform_state.ime.effective_open(),
        input_mode: format!("{:?}", app.platform_state.ime.input_mode()),
        applied: format!("{:?}", app.platform_state.ime.applied_state()),
        app_kind: format!("{:?}", app.platform_state.focus.app_kind),
        focus_kind: format!("{:?}", app.platform_state.focus.focus_kind),
        gji_state: app.platform.gji_state_label(),
        // BUG-34 横展開の切り分け用（docs/known-bugs.md BUG-34 参照）。
        send_health_last_elapsed_ms: crate::send_health::last_elapsed_ms(),
        send_health_consecutive_slow: crate::send_health::consecutive_slow(),
        send_health_breaker_tripped: !crate::send_health::blocking_allowed(now_ms),
        idle_conv_check_in_flight_ms: app
            .platform_state
            .gate
            .idle_conv_check_in_flight_since_ms
            .map(|since| now_ms.saturating_sub(since)),
        process_uptime_secs: resources.process_uptime_secs,
        working_set_bytes: resources.working_set_bytes,
        handle_count: resources.handle_count,
        gdi_object_count: resources.gdi_object_count,
        user_object_count: resources.user_object_count,
    };
    let (config_toml, layout_yab) =
        crate::app::read_bug_report_attachments(app.platform.tray.current_layout_name());
    // ADR-120 決定0a-report: 打鍵内容を含まない累積カウンタのみを写す。
    let retro_eval_stats = Some(crate::bug_report::BugReportRetroEvalStats::from(
        app.engine.retro_eval_stats(),
    ));
    crate::bug_report::BugReportDiagnostics {
        ime_product_name: crate::tsf::observer::current_ime_product_name(),
        keyboard_model: bug_report_keyboard_model(app.keyboard_model()).to_owned(),
        windows_keyboard_layout: format!("LANGID=0x{lang_id:04X} (Japanese={is_japanese})"),
        competing_software: crate::app::detect_conflicting_software(),
        state_snapshot: Some(state_snapshot),
        config_toml,
        layout_yab,
        retro_eval_stats,
    }
}

/// 「長時間使うと重くなる」報告の切り分け用プロセスリソーススナップショット
/// （BugReportStateSnapshot 参照）。
struct ProcessResourceSnapshot {
    process_uptime_secs: u64,
    working_set_bytes: u64,
    handle_count: u32,
    gdi_object_count: u32,
    user_object_count: u32,
}

fn filetime_to_100ns_units(ft: windows::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

fn process_resource_snapshot() -> ProcessResourceSnapshot {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetGuiResources, GetProcessHandleCount, GetProcessTimes, GR_GDIOBJECTS,
        GR_USEROBJECTS,
    };

    // SAFETY: すべて自プロセスの疑似ハンドル（`GetCurrentProcess()`、クローズ不要）
    // に対する読み取り専用の Win32 呼び出し。out 引数はすべてスタック上のロー
    // カル変数を指し、呼び出し後に読むだけで所有権の受け渡しは発生しない。
    unsafe {
        let process = GetCurrentProcess();

        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let process_uptime_secs = if GetProcessTimes(
            process,
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
        .is_ok()
        {
            let now = filetime_to_100ns_units(GetSystemTimeAsFileTime());
            let created = filetime_to_100ns_units(creation);
            now.saturating_sub(created) / 10_000_000
        } else {
            0
        };

        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).unwrap_or(0),
            ..Default::default()
        };
        let working_set_bytes =
            if GetProcessMemoryInfo(process, &raw mut counters, counters.cb).is_ok() {
                counters.WorkingSetSize as u64
            } else {
                0
            };

        let mut handle_count = 0_u32;
        let handle_count = if GetProcessHandleCount(process, &raw mut handle_count).is_ok() {
            handle_count
        } else {
            0
        };

        ProcessResourceSnapshot {
            process_uptime_secs,
            working_set_bytes,
            handle_count,
            gdi_object_count: GetGuiResources(process, GR_GDIOBJECTS),
            user_object_count: GetGuiResources(process, GR_USEROBJECTS),
        }
    }
}

const fn bug_report_keyboard_model(model: awase::scanmap::KeyboardModel) -> &'static str {
    match model {
        awase::scanmap::KeyboardModel::Jis => "Jis",
        awase::scanmap::KeyboardModel::Us => "Us",
    }
}

fn write_bug_report_diagnostics(
    diagnostics: &crate::bug_report::BugReportDiagnostics,
) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let tick = hook::current_tick_ms();
    let path = std::env::temp_dir().join(format!("awase_bug_report_diagnostics_{tick}.json"));
    let json = serde_json::to_string_pretty(diagnostics)?;
    std::fs::write(&path, json)?;
    Ok(path)
}

/// WM_DRAIN_OUTPUT_QUEUE ハンドラ
pub(crate) unsafe fn handle_wm_drain_output_queue() {
    if DRAIN_PENDING.swap(true, Ordering::AcqRel) {
        DRAIN_RERUN_PENDING.store(true, Ordering::Release);
        return;
    }
    // [drain-start] order-bug 調査用: OUTPUT_GATE 解除から drain 開始までのギャップを観測する。
    // この間に届く inline キーが drain 待ちキーを追い越して [engine-input] に流れていないか
    // タイムスタンプで突き合わせるための起点ログ。
    let drain_start_us = hook::now_timestamp_us();
    let queue_len_initial = crate::INPUT_DEFER.pending_len_nonblocking();
    log::debug!(
        "[drain-start] now={}us queue_len={}",
        drain_start_us,
        queue_len_initial.map_or_else(|| "?".to_owned(), |n| n.to_string()),
    );

    let _ = with_app(|runtime| {
        runtime.platform.flush_raw_tsf_literal_recovery();
        for entry in runtime.platform.drain_journal_entries() {
            runtime.platform_state.ime.journal.absorb(entry);
        }
    });

    // classify 済みイベントを取り出し、enrich_ime_relevance（sync key 判定）のみ with_app 内で補完する。
    let queue = with_app(|app| {
        let mut events = crate::INPUT_DEFER.take_all();
        // このバッチ（drain 対象の全イベント）につき1回だけ resync する（指摘5）。
        begin_key_batch(app);
        for ev in &mut events {
            app.enrich_ime_relevance(ev);
            log::debug!("[drain] vk=0x{:02X} {:?}", ev.vk_code, ev.event_type);
        }
        events
    });
    // `with_app` が理由を問わず `None` を返した場合（二重WM_DRAIN_OUTPUT_QUEUEの
    // 再入だけでなく、Runtime が他所で借用中のケースも含む）は、必ず
    // `DRAIN_RERUN_PENDING` を立てて次回の回収に委ねる。旧実装は `take_all()` を
    // `with_app` の外で無条件に呼んでいたためこの再入シェイプで drain を失うことは
    // 無かった——Opus敵対的レビューで再入時にリトライが保証されない経路として
    // 指摘され是正（2026-08-26）。
    let Some(queue) = queue else {
        DRAIN_RERUN_PENDING.store(true, Ordering::Release);
        finish_drain();
        return;
    };

    if !queue.is_empty() {
        let now_us = hook::now_timestamp_us();
        let mut any_reinject = false;
        let processed = with_app(|app| {
            for queued_event in &queue {
                log::debug!(
                    "[output-drain] replay vk=0x{:02X} {:?} event_ts={}us now={}us delta={}ms",
                    queued_event.vk_code,
                    queued_event.event_type,
                    queued_event.timestamp,
                    now_us,
                    now_us.saturating_sub(queued_event.timestamp) / 1000,
                );
                let delivery = deliver_key_event(app, *queued_event, KeyOrigin::DeferredReplay);
                if matches!(delivery, KeyDelivery::Reinjected) {
                    log::debug!(
                        "[output-drain] PassThrough → enqueue ReinjectKey vk=0x{:02X} {:?} (drain has no hook→OS path)",
                        queued_event.vk_code, queued_event.event_type,
                    );
                    any_reinject = true;
                }
            }
        });
        if processed.is_none() {
            // Runtime を掴めず queue を処理できなかった。取り出し済みのイベントを
            // 失わないよう INPUT_DEFER へ戻し、次回の drain 起動でやり直す。
            crate::INPUT_DEFER.replay_later(queue);
            DRAIN_RERUN_PENDING.store(true, Ordering::Release);
        } else if any_reinject {
            // drain 中に PassThrough → reinject へ昇格させた key がある場合、
            // executor キューを実際に流すために `WM_EXECUTE_EFFECTS` を要求する。
            // on_key_event_impl 単独経路では has_pending が false の場合に通知が
            // 飛ばないため、明示的に post する。
            post_to_main_thread(WM_EXECUTE_EFFECTS);
        }
    }

    // OUTPUT_GATE active 中に発火した TIMER_PENDING/TIMER_SPECULATIVE を drain 完了後に replay する。
    // drain で chord パートナー（親指キー等）が処理されて Kill(timer_id) が発行されていた場合は
    // current_os_id が変化（または None）となり replay をスキップする。
    //
    // os_id 照合が重要な理由:
    //   drain 中に「古いタイマー kill → 新タイマー set」が起きると logical_id は
    //   is_active=true のままだが、それは別の文字に属する新規タイマーである。
    //   新規タイマーを deferred replay で早期発火させると文字順が狂う
    //   （例: というのは → とはいうの）。os_id 照合でこれを防ぐ。
    let _ = with_app(|app| {
        let deferred = std::mem::take(&mut app.ime_coordinator.deferred_engine_timers);
        if deferred.is_empty() {
            return;
        }
        let ctx = app.build_ctx();
        for (timer_id, os_id) in deferred {
            let current = app.platform.timer.current_os_id(timer_id);
            if current == Some(os_id) {
                log::debug!(
                    "[deferred-timer] drain 後に replay logical_id={timer_id} (os_id={os_id})"
                );
                let decision = app.engine.on_timeout(timer_id, &ctx);
                notify_if_solo_off_triggered(app);
                app.execute_decision(decision);
            } else {
                log::debug!(
                    "[deferred-timer] logical_id={timer_id} (os_id={os_id}) は drain 中に変化 (current={current:?}) → skip"
                );
            }
        }
    });

    // H-4-a: 全キー処理完了後に RuntimeOutbox を drain する。
    // drain_output_queue 中の process_key_event → send_keys で積まれた RuntimeRequest を実行する。
    let _ = with_app(|app| {
        app.drain_runtime_requests();
    });
    finish_drain();
}

#[cfg(test)]
pub(crate) mod drain_pending_test_api {
    use super::{recover_pending_drain_request, DRAIN_PENDING, DRAIN_RERUN_PENDING};
    use std::sync::atomic::Ordering;

    pub(crate) fn reset() {
        DRAIN_PENDING.store(false, Ordering::Release);
        DRAIN_RERUN_PENDING.store(false, Ordering::Release);
    }

    pub(crate) fn simulate_reentrant_drain_request() {
        DRAIN_PENDING.store(true, Ordering::Release);
        if DRAIN_PENDING.swap(true, Ordering::AcqRel) {
            DRAIN_RERUN_PENDING.store(true, Ordering::Release);
        }
    }

    pub(crate) fn finish_active_without_recovery() {
        DRAIN_PENDING.store(false, Ordering::Release);
    }

    pub(crate) fn recover() {
        recover_pending_drain_request();
    }

    pub(crate) fn rerun_pending() -> bool {
        DRAIN_RERUN_PENDING.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn drain_pending_reentrant_request_is_recovered_by_next_handler() {
        super::drain_pending_test_api::reset();

        super::drain_pending_test_api::simulate_reentrant_drain_request();
        assert!(super::drain_pending_test_api::rerun_pending());

        super::drain_pending_test_api::finish_active_without_recovery();
        super::drain_pending_test_api::recover();
        assert!(!super::drain_pending_test_api::rerun_pending());

        super::drain_pending_test_api::reset();
    }
}

/// TaskbarCreated ハンドラ（Explorer 再起動時にトレイアイコンを復元）
pub(crate) unsafe fn handle_taskbar_created(app: &mut Runtime) {
    log::info!("Explorer restarted, re-registering tray icon");
    app.platform.tray.recreate();
}

/// WM_DUMP_JOURNAL ハンドラ（Alt+変換→Alt+無変換 ×2 でトリガー）
pub(crate) fn handle_wm_dump_journal(app: &mut Runtime) {
    // プローブ棄却統計をダンプ直前にログ出力してリセット
    let stats = crate::state::probe_admission::drain_stats();
    if stats.epoch_mismatch > 0
        || stats.hwnd_mismatch_same_root > 0
        || stats.hwnd_mismatch_cross_root > 0
    {
        log::info!(
            "[probe-admission] rejected since last dump: epoch_mismatch={} \
             hwnd_mismatch_same_root={} hwnd_mismatch_cross_root={}",
            stats.epoch_mismatch,
            stats.hwnd_mismatch_same_root,
            stats.hwnd_mismatch_cross_root
        );
    }
    // HOOK_KEYS の最大占有数（指摘2-4）: overflow の頻度を実測できるようにする。
    let max_occupancy = crate::hook_channel::HOOK_KEYS.take_max_occupancy();
    if max_occupancy > 0 {
        log::info!(
            "[hook-ring] max occupancy since last dump: {max_occupancy}/{}",
            crate::hook_channel::CAP
        );
    }
    for entry in app.platform.drain_journal_entries() {
        app.platform_state.ime.journal.absorb(entry);
    }
    app.platform_state
        .ime
        .journal
        .record(crate::journal::JournalEntry::ClockAnchor {
            tick_ms: hook::current_tick_ms(),
            hook_us: hook::now_timestamp_us(),
        });
    app.platform_state
        .ime
        .journal
        .record(crate::journal::JournalEntry::DumpTriggered);
    match app.platform_state.ime.journal.dump_to_file() {
        Ok(path) => {
            log::info!("[journal] ダンプ完了: {}", path.display());
            app.platform
                .tray
                .show_balloon("awase journal", &format!("ダンプ完了: {}", path.display()));
        }
        Err(e) => {
            log::error!("[journal] ダンプ失敗: {e}");
        }
    }
}
