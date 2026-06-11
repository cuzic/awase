//! WM_* メッセージハンドラ
//!
//! `run_message_loop` の `match msg.message` 各 arm を関数として切り出したもの。
//! すべて `pub(crate)` で `app/mod.rs` からのみ呼ばれる。

use std::mem::size_of;
use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, PostQuitMessage, GUITHREADINFO};

use crate::focus::cache::DetectionSource;
use crate::hook;
use crate::hook::CallbackResult;
use crate::tray;
use crate::win32::post_to_main_thread;
use crate::{
    with_app, with_app_ref, Runtime, ELEVATED, TIMER_HOOK_WATCHDOG, TIMER_IME_REFRESH,
    TIMER_OUTPUT_GUARD, TIMER_POWER_RESUME, TIMER_TSF_GATE, TIMER_TSF_PROBE, WM_EXECUTE_EFFECTS,
};
use awase::types::{ContextChange, FocusKind};

use crate::app::{check_keyboard_layout_on_change, launch_settings, reload_config};

/// WM_KEY_FROM_HOOK ハンドラ — フックスレッドから転送された物理キーイベントを処理する
pub(crate) fn handle_wm_key_from_hook(app: &mut Runtime, event: awase::types::RawKeyEvent) {
    // ウォッチドッグ・IME ポーリング用アクティビティタイムスタンプ更新（物理キーのみ）
    app.platform_state.last_hook_activity_ms = hook::current_tick_ms();

    // NonText フォーカス（タスクバー等）はすべて OS にパススルー
    if app.platform_state.focus_kind == FocusKind::NonText {
        app.executor.enqueue_reinject(event);
        post_to_main_thread(WM_EXECUTE_EFFECTS);
        return;
    }

    let result = app.process_key_event(event);
    if matches!(result, CallbackResult::PassThrough) {
        app.executor.enqueue_reinject(event);
        post_to_main_thread(WM_EXECUTE_EFFECTS);
    }
}

/// WM_TIMER ハンドラ
#[allow(clippy::cognitive_complexity)]
pub(crate) unsafe fn handle_wm_timer(
    app: &mut Runtime,
    wparam: usize,
    msg: &windows::Win32::UI::WindowsAndMessaging::MSG,
) {
    use windows::Win32::UI::WindowsAndMessaging::DispatchMessageW;
    let logical_id = app.platform.timer.resolve(wparam);
    match logical_id {
        Some(id) if id == TIMER_IME_REFRESH => {
            if app.platform_state.sync_key_gate.is_active()
                || app.platform_state.sync_key_gate.has_deferred_keys()
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
            app.platform_state.focus_kind = FocusKind::Undetermined;
            app.schedule_ime_refresh(500);
        }
        Some(id) if id == TIMER_OUTPUT_GUARD => {
            let outcomes = app.executor.on_output_guard_timer(&mut app.platform);
            app.dispatch_outcomes(outcomes);
        }
        Some(id) if id == TIMER_TSF_PROBE => {
            app.platform.advance_tsf_probe();
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
                    "[ime-off-rescue] 50ms timer expired → 保留 vk=0x{:02X} を IME-OFF として発火",
                    pending_event.vk_code
                );
                let result = app.replay_ime_off_rescue_event(pending_event);
                if matches!(result, CallbackResult::PassThrough) {
                    app.executor.enqueue_reinject(pending_event);
                    post_to_main_thread(WM_EXECUTE_EFFECTS);
                }
            }
        }
        Some(id) if id == TIMER_HOOK_WATCHDOG => {
            let last_activity = hook::HOOK_ALIVE_TICK_MS.load(Ordering::Relaxed);
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
                    "[engine-timer] OUTPUT_GATE active → logical_id={timer_id} を drain 後に延期"
                );
                app.deferred_engine_timers.push(timer_id);
                return;
            }
            let modifiers = unsafe { crate::observer::focus_observer::read_os_modifiers() };
            let ctx = super::build_input_context(
                app.platform_state.ime.effective_open(),
                &app.platform_state.ime.belief,
                &modifiers,
            );
            let decision = app.engine.on_timeout(timer_id, &ctx);
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
    let outcomes = app.executor.drain_deferred(&mut app.platform);
    app.dispatch_outcomes(outcomes);
}

/// WM_PANIC_RESET ハンドラ
pub(crate) unsafe fn handle_wm_panic_reset(app: &mut Runtime) {
    app.panic_reset();
}

/// WM_DUPLICATE_INSTANCE ハンドラ
pub(crate) unsafe fn handle_wm_duplicate_instance(app: &mut Runtime) {
    log::info!("Duplicate instance notification received");
    app.platform
        .tray
        .show_balloon("awase", "awase はすでに起動しています");
}

/// WM_IME_KEY_DETECTED ハンドラ
pub(crate) unsafe fn handle_wm_ime_key_detected(app: &mut Runtime) {
    if app.platform_state.sync_key_gate.is_active()
        || app.platform_state.sync_key_gate.has_deferred_keys()
    {
        app.process_deferred_keys();
    } else {
        app.refresh_ime_state_cache();
    }
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

/// WM_PROCESS_DEFERRED ハンドラ
pub(crate) unsafe fn handle_wm_process_deferred(app: &mut Runtime) {
    app.process_deferred_keys();
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
    if GetGUIThreadInfo(0, &raw mut info).is_ok() && info.hwndFocus != result_hwnd {
        log::debug!("UIA result for stale hwnd, ignoring");
    } else {
        if app_kind_u8 != crate::FOCUS_KIND_UPDATE_NO_APP_KIND {
            let app_kind = awase::types::AppKind::from_u8(app_kind_u8);
            app.platform_state.app_kind = app_kind;
            log::debug!("UIA AppKind update: {app_kind:?}");
        }

        if kind != FocusKind::Undetermined {
            app.platform_state.focus_kind = kind;

            if app.platform.focus.is_focused() {
                let pid = app.platform.focus.pid();
                let cls = app.platform.focus.class_name().to_owned();
                app.platform
                    .focus
                    .cache_insert(pid, cls, kind, DetectionSource::UiaAsync);
            }
            if kind == FocusKind::NonText {
                app.invalidate_engine_context(ContextChange::FocusChanged);
            }
        }
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
    tray::handle_tray_message(
        hwnd,
        lparam,
        &layout_names,
        ELEVATED.load(Ordering::Relaxed),
    );
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
        Some(tray::TrayCommand::GjiSetup) => tray::handle_gji_setup(),
        Some(tray::TrayCommand::ClearImmCache) | None => {}
    }
}

/// WM_DRAIN_OUTPUT_QUEUE ハンドラ
#[expect(clippy::cognitive_complexity)]
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
    // is_active=false となり replay をスキップする（余分な文字が出力されない）。
    let _ = with_app(|app| {
        let deferred = std::mem::take(&mut app.deferred_engine_timers);
        if deferred.is_empty() {
            return;
        }
        let ctx = app.build_ctx();
        for timer_id in deferred {
            if app.platform.timer.is_active(timer_id) {
                log::debug!("[deferred-timer] drain 後に replay logical_id={timer_id}");
                let decision = app.engine.on_timeout(timer_id, &ctx);
                app.execute_decision(decision);
            } else {
                log::debug!(
                    "[deferred-timer] logical_id={timer_id} は drain 中に Kill 済み → skip"
                );
            }
        }
    });
}

/// TaskbarCreated ハンドラ（Explorer 再起動時にトレイアイコンを復元）
pub(crate) unsafe fn handle_taskbar_created(app: &mut Runtime) {
    log::info!("Explorer restarted, re-registering tray icon");
    app.platform.tray.recreate();
}
