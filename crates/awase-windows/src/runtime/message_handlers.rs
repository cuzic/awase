//! WM_* メッセージハンドラ
//!
//! `run_message_loop` の `match msg.message` 各 arm を関数として切り出したもの。
//! すべて `pub(crate)` で `app/mod.rs` からのみ呼ばれる。

use std::mem::size_of;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, PostQuitMessage, GUITHREADINFO};

use crate::focus::FocusKind;
use crate::hook;
use crate::hook::CallbackResult;
use crate::tray;
use crate::vk::VkCodeExt;
use crate::win32::post_to_main_thread;
use crate::{
    with_app, with_app_ref, Runtime, TIMER_GJI_LONG_IDLE, TIMER_HOOK_WATCHDOG, TIMER_IME_REFRESH,
    TIMER_OUTPUT_GUARD, TIMER_POWER_RESUME, TIMER_TSF_GATE, TIMER_TSF_PROBE, WM_EXECUTE_EFFECTS,
};
use awase::platform::ImeOpenOutcome;
use awase::types::ContextChange;

use crate::app::{check_keyboard_layout_on_change, launch_settings, reload_config};

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

/// WM_KEY_FROM_HOOK ハンドラ — フックスレッドから転送された物理キーイベントを処理する
pub(crate) fn handle_wm_key_from_hook(app: &mut Runtime, event: awase::types::RawKeyEvent) {
    // ウォッチドッグ・IME ポーリング用アクティビティタイムスタンプ更新（物理キーのみ）
    app.platform_state.gate.last_hook_activity_ms = hook::current_tick_ms();

    // NonText フォーカス（タスクバー等）はすべて OS にパススルー
    if app.platform_state.focus.focus_kind == FocusKind::NonText {
        app.executor.enqueue_reinject(event);
        post_to_main_thread(WM_EXECUTE_EFFECTS);
        return;
    }

    // ── Post-bypass passthrough（Ctrl+J 等 tmux prefix 直後のコマンドキー）──
    // Ctrl+key bypass の直後に non-Ctrl 非修飾キーが来た場合、NICOLA エンジンを
    // スキップして直接 passthrough する（1 キー分のみ）。
    // 例: Ctrl+J (tmux prefix) → n (next-window) で NICOLA が n を横取りするのを防ぐ。
    let is_key_down = matches!(event.event_type, awase::types::KeyEventType::KeyDown);
    if app.platform_state.gate.post_bypass_passthrough
        && !event.modifier_snapshot.ctrl
        && !event.vk_code.is_passthrough()
    {
        if is_key_down {
            // KeyDown でフラグを消費（対応する KeyUp も後続で通常通り PassThrough になる）
            app.platform_state.gate.post_bypass_passthrough = false;
            log::debug!(
                "[post-bypass] consumed: vk=0x{:02X} → direct passthrough (NICOLA skipped)",
                event.vk_code
            );
        }
        app.executor.enqueue_reinject(event);
        post_to_main_thread(WM_EXECUTE_EFFECTS);
        return;
    }

    let result = app.process_key_event(event);
    if matches!(result, CallbackResult::PassThrough) {
        // GJI 候補ウィンドウが表示中に Ctrl+key がパススルーされる際、
        // GJI が Ctrl+key を IME ショートカットとして横取りしないよう composition を
        // 先にキャンセルする。例: IME ON + め入力中 → Ctrl+J → tmux prefix。
        // Ctrl↓ではなく実際の Ctrl+非修飾キー↓ 時点でキャンセルすることで、
        // 修飾キーのみの押下時に composition を誤ってキャンセルしない。
        if is_key_down && event.modifier_snapshot.ctrl && !event.vk_code.is_passthrough() {
            let candidate_visible = app.platform.is_composition_warm_in_tsf();
            log::debug!(
                "[ctrl-check] vk=0x{:02X} candidate_visible={candidate_visible}",
                event.vk_code
            );
            if candidate_visible {
                // SAFETY: メインスレッドから呼ぶ。
                unsafe { super::cancel_ime_composition() };
                app.platform.on_ctrl_bypass_composition_cancel();
                log::debug!(
                    "[ctrl-bypass] IME composition cancelled (vk=0x{:02X})",
                    event.vk_code
                );
            }
            // [[post_bypass]] ルールに一致する場合、次の非修飾キーを NICOLA スキップ。
            // tmux では prefix (Ctrl+J) 後に standalone n/p 等のコマンドキーを入力するため。
            let proc = app.platform.focus.process_name();
            let cls = app.platform.focus.class_name();
            if app
                .post_bypass_rules
                .iter()
                .any(|r| r.matches(event.vk_code, proc, cls))
            {
                app.platform_state.gate.post_bypass_passthrough = true;
                log::debug!(
                    "[ctrl-bypass] post_bypass_passthrough=true (proc={proc:?} class={cls:?})"
                );
            }
        }
        app.executor.enqueue_reinject(event);
        post_to_main_thread(WM_EXECUTE_EFFECTS);
    }
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
                let result = app.replay_ime_off_rescue_event(pending_event);
                if matches!(result, CallbackResult::PassThrough) {
                    app.executor.enqueue_reinject(pending_event);
                    post_to_main_thread(WM_EXECUTE_EFFECTS);
                }
            }
        }
        Some(id) if id == TIMER_GJI_LONG_IDLE => {
            app.platform.timer.kill(TIMER_GJI_LONG_IDLE);
            app.platform.gji_on_timer_long_idle();
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
            if crate::OUTPUT_GATE.is_active() {
                log::debug!(
                    "[engine-timer] OUTPUT_GATE active → logical_id={timer_id} (os_id={wparam}) を drain 後に延期"
                );
                app.ime_coordinator
                    .deferred_engine_timers
                    .push((timer_id, wparam));
                return;
            }
            let modifiers = unsafe { crate::observer::focus_observer::read_os_modifiers() };
            let ctx = super::build_input_context(
                app.platform_state.ime.effective_open(),
                app.platform_state.ime.input_mode(),
                app.platform_state.ime.belief.is_japanese_ime(),
                crate::tsf::observer::ime_composition_active_now(),
                &modifiers,
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
}

/// WM_EXECUTE_EFFECTS ハンドラ
pub(crate) unsafe fn handle_wm_execute_effects(app: &mut Runtime) {
    let outcomes = app
        .executor
        .drain_deferred(&mut app.platform, &app.platform_state.ime);
    app.dispatch_outcomes(outcomes);
    // H-4-a: Output が send_keys 中に積んだ RuntimeRequest を一括処理する。
    app.drain_runtime_requests();
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
pub(crate) fn post_async_ime_apply_complete(
    open: bool,
    outcome: ImeOpenOutcome,
    generation: Option<u64>,
) {
    let generation = generation.unwrap_or(0);
    let wparam = ((generation as usize) << 1) | usize::from(open);
    crate::win32::post_to_main_thread_with(
        crate::WM_ASYNC_IME_APPLY_COMPLETE,
        wparam,
        encode_outcome(outcome),
    );
}

/// WM_ASYNC_IME_APPLY_COMPLETE ハンドラ
///
/// ImmCross async apply の完了通知。sync path の `sync_outcomes` と対称に、
/// generation 照合を含む単一入口 `on_ime_apply_complete`（B+C+D+E）へ合流する。
pub(crate) fn handle_wm_async_ime_apply_complete(app: &mut Runtime, wparam: usize, lparam: isize) {
    let open = (wparam & 1) != 0;
    let generation = match (wparam >> 1) as u64 {
        0 => None,
        generation => Some(generation),
    };
    let outcome = decode_outcome(lparam);
    if outcome == ImeOpenOutcome::Failed {
        log::warn!("apply_ime_open({open}) failed (async)");
    }
    app.on_ime_apply_complete(open, outcome, generation);
}

/// WM_PANIC_RESET ハンドラ
pub(crate) unsafe fn handle_wm_panic_reset(app: &mut Runtime) {
    app.panic_reset();
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
    }

    // MS-IME と確定したら、無変換/変換キーの IME オン/オフ割り当て（awase と
    // 競合し belief 乖離を起こす）をチェックして解除を案内する
    // （ポップアップは同一内容につき一度、内容が変われば再警告）。
    // detected を見るのは、未検出時の active_ime_kind() が安全デフォルトとして
    // MicrosoftIme を返すため — これを見ないと GJI ユーザーの起動時にも誤発動する。
    if detected && matches!(kind, crate::tsf::observer::ActiveImeKind::MicrosoftIme) {
        crate::msime_key_assignment::check_and_warn();
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
    let layout_names: Vec<String> =
        with_app_ref(|app| app.layouts.iter().map(|e| e.name.clone()).collect())
            .unwrap_or_default();
    tray::handle_tray_message(hwnd, lparam, &layout_names, crate::is_elevated());
}

/// WM_RELOAD_CONFIG ハンドラ
pub(crate) fn handle_wm_reload_config() {
    log::info!("Config reload requested via WM_RELOAD_CONFIG");
    reload_config();
}

/// WM_COMMAND ハンドラ
pub(crate) unsafe fn handle_wm_command(wparam: WPARAM) {
    match tray::handle_tray_command(wparam) {
        Some(tray::TrayCommand::Settings) => launch_settings(),
        Some(tray::TrayCommand::RestartAdmin) => tray::restart_as_admin(),
        Some(tray::TrayCommand::Toggle) => {
            let _ = with_app(Runtime::toggle_engine);
        }
        Some(tray::TrayCommand::Exit) => PostQuitMessage(0),
        Some(tray::TrayCommand::SelectLayout(index)) => {
            let _ = with_app(|app| app.switch_layout(index));
        }
        Some(tray::TrayCommand::ToggleAutoStart) => tray::handle_autostart_toggle(),
        Some(tray::TrayCommand::Restart) => tray::restart_self(),
        Some(tray::TrayCommand::CapsLock) => {
            crate::ime::toggle_caps_lock();
        }
        Some(tray::TrayCommand::ImeHiragana) => {
            let _ = crate::ime::set_ime_mode(
                true,
                crate::imm::IME_CMODE_NATIVE | crate::imm::IME_CMODE_FULLSHAPE,
                crate::imm::IME_CMODE_KATAKANA,
            );
        }
        Some(tray::TrayCommand::ImeFullKatakana) => {
            let _ = crate::ime::set_ime_mode(
                true,
                crate::imm::IME_CMODE_NATIVE
                    | crate::imm::IME_CMODE_KATAKANA
                    | crate::imm::IME_CMODE_FULLSHAPE,
                0,
            );
        }
        Some(tray::TrayCommand::ImeFullAlpha) => {
            let _ = crate::ime::set_ime_mode(
                true,
                crate::imm::IME_CMODE_FULLSHAPE,
                crate::imm::IME_CMODE_NATIVE | crate::imm::IME_CMODE_KATAKANA,
            );
        }
        Some(tray::TrayCommand::ImeHalfAlpha) => {
            let _ = crate::ime::set_ime_mode(
                true,
                0,
                crate::imm::IME_CMODE_NATIVE
                    | crate::imm::IME_CMODE_KATAKANA
                    | crate::imm::IME_CMODE_FULLSHAPE,
            );
        }
        Some(tray::TrayCommand::ImeHalfKatakana) => {
            let _ = crate::ime::set_ime_mode(
                true,
                crate::imm::IME_CMODE_NATIVE | crate::imm::IME_CMODE_KATAKANA,
                crate::imm::IME_CMODE_FULLSHAPE,
            );
        }
        Some(tray::TrayCommand::ImeDirect) => {
            let _ = crate::ime::set_ime_mode(false, 0, 0);
        }
        Some(tray::TrayCommand::InputRomaji) => {
            let _ = crate::ime::set_ime_romaji_mode_state(true);
        }
        Some(tray::TrayCommand::InputKana) => {
            let _ = crate::ime::set_ime_romaji_mode_state(false);
        }
        Some(tray::TrayCommand::ResetState) => {
            let caps_lock_on =
                windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x14) & 1 != 0;
            if caps_lock_on {
                crate::ime::toggle_caps_lock();
            }
            let _ = crate::ime::set_ime_mode(
                true,
                crate::imm::IME_CMODE_NATIVE
                    | crate::imm::IME_CMODE_FULLSHAPE
                    | crate::imm::IME_CMODE_ROMAN,
                crate::imm::IME_CMODE_KATAKANA,
            );
            // 無変換ソロ連打の緊急停止（ADR-055 追補）等で user_enabled が false に
            // なっていても、この操作で必ず Engine ON まで復帰させる。
            let _ = with_app(Runtime::force_engine_on);
        }
        Some(tray::TrayCommand::ClearImmCache) | None => {}
    }
}

/// WM_DRAIN_OUTPUT_QUEUE ハンドラ
pub(crate) unsafe fn handle_wm_drain_output_queue() {
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
    });

    // classify 済みイベントを取り出し、enrich_ime_relevance（sync key 判定）のみ with_app 内で補完する。
    let queue = {
        let mut events = crate::INPUT_DEFER.take_all();
        let _ = with_app(|app| {
            for ev in &mut events {
                app.enrich_ime_relevance(ev);
                log::debug!("[drain] vk=0x{:02X} {:?}", ev.vk_code, ev.event_type);
            }
        });
        events
    };

    if !queue.is_empty() {
        let now_us = hook::now_timestamp_us();
        let mut any_reinject = false;
        let _ = with_app(|app| {
            for queued_event in &queue {
                log::debug!(
                    "[output-drain] replay vk=0x{:02X} {:?} event_ts={}us now={}us delta={}ms",
                    queued_event.vk_code,
                    queued_event.event_type,
                    queued_event.timestamp,
                    now_us,
                    now_us.saturating_sub(queued_event.timestamp) / 1000,
                );
                let result = app.process_key_event(*queued_event);
                if matches!(result, CallbackResult::PassThrough) {
                    log::debug!(
                        "[output-drain] PassThrough → enqueue ReinjectKey vk=0x{:02X} {:?} (drain has no hook→OS path)",
                        queued_event.vk_code, queued_event.event_type,
                    );
                    app.executor.enqueue_reinject(*queued_event);
                    any_reinject = true;
                }
            }
        });
        // drain 中に PassThrough → reinject へ昇格させた key がある場合、
        // executor キューを実際に流すために `WM_EXECUTE_EFFECTS` を要求する。
        // on_key_event_impl 単独経路では has_pending が false の場合に通知が
        // 飛ばないため、明示的に post する。
        if any_reinject {
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
    if stats.epoch_mismatch > 0 {
        log::info!(
            "[probe-admission] rejected since last dump: epoch_mismatch={}",
            stats.epoch_mismatch
        );
    }
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
