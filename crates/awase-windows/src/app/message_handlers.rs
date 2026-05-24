//! WM_* メッセージハンドラ
//!
//! `run_message_loop` の `match msg.message` 各 arm を関数として切り出したもの。
//! すべて `pub(super)` で `app/mod.rs` からのみ呼ばれる。

use std::mem::size_of;
use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    GetGUIThreadInfo, PostQuitMessage, GUITHREADINFO,
};

use awase::types::{ContextChange, FocusKind};
use awase_windows::focus::cache::DetectionSource;
use awase_windows::hook;
use awase_windows::runtime;
use awase_windows::{
    Runtime, ELEVATED, TIMER_HOOK_WATCHDOG, TIMER_IME_REFRESH, TIMER_OUTPUT_GUARD,
    TIMER_POWER_RESUME, TIMER_TSF_GATE, TIMER_TSF_PROBE, with_app, with_app_ref,
};
use awase_windows::tray;

use super::{check_keyboard_layout_on_change, launch_settings, on_key_event_impl, reload_config};

/// WM_TIMER ハンドラ
#[allow(clippy::cognitive_complexity)]
pub(super) unsafe fn handle_wm_timer(app: &mut Runtime, logical_id: Option<usize>, _msg_wparam: usize, msg: &windows::Win32::UI::WindowsAndMessaging::MSG) {
    use windows::Win32::UI::WindowsAndMessaging::DispatchMessageW;
    match logical_id {
        Some(id) if id == TIMER_IME_REFRESH => {
            // sync な部分（blocking なし）
            hook::sync_sent_to_engine(&mut app.platform_state.hook);
            if app.platform_state.sync_key_gate.active
                || !app.platform_state.sync_key_gate.deferred_keys.is_empty()
            {
                app.process_deferred_keys();
            }
            // async タスクをスポーン（with_app を解放してから fetch）
            app.spawn_ime_refresh();
        }
        Some(id) if id == TIMER_POWER_RESUME => {
            app.executor.platform.timer.kill(TIMER_POWER_RESUME);
            log::info!("Power resume recovery: reinstalling hook (lightweight)");
            hook::reinstall_hook();
            app.invalidate_engine_context(ContextChange::InputLanguageChanged);
            app.platform_state.focus.focus_kind = FocusKind::Undetermined;
            app.schedule_ime_refresh(500);
        }
        Some(id) if id == TIMER_OUTPUT_GUARD => {
            app.executor.on_output_guard_timer();
        }
        Some(id) if id == TIMER_TSF_PROBE => {
            app.executor.platform.advance_tsf_probe();
        }
        Some(id) if id == TIMER_TSF_GATE => {
            app.executor.platform.timer.kill(TIMER_TSF_GATE);
            let held = app.executor.platform.output.on_tsf_warmup_timeout();
            if !held.is_empty() {
                log::debug!(
                    "[tsf-gate-timeout] draining {} held keys via INPUT_DEFER",
                    held.len()
                );
                awase_windows::INPUT_DEFER.replay_later(held);
            }
        }
        Some(id) if id == TIMER_HOOK_WATCHDOG => {
            use std::sync::atomic::AtomicU64;
            static PING_SENT_AT: AtomicU64 = AtomicU64::new(0);
            let ping_sent = PING_SENT_AT.load(Ordering::Relaxed);
            let last_activity = app.platform_state.last_hook_activity_ms
                .max(awase_windows::OUTPUT_GATE.last_vk_output_ms_val());

            if ping_sent > 0 && last_activity < ping_sent {
                let stale_ms = hook::current_tick_ms() - last_activity;
                log::error!(
                    "Hook watchdog: ping not received (last activity {stale_ms}ms ago) — reinstalling"
                );
                if hook::reinstall_hook() {
                    app.executor.platform.tray.show_balloon(
                        "awase",
                        "キーボードフックを自動復旧しました",
                    );
                } else {
                    app.executor.platform.tray.show_balloon(
                        "awase",
                        "フック復旧に失敗しました。再起動してください。",
                    );
                }
            }

            PING_SENT_AT.store(hook::current_tick_ms(), Ordering::Relaxed);
            hook::send_ping();
        }
        Some(timer_id) => {
            log::debug!("WM_TIMER fired: logical_id={timer_id}");
            let modifiers = unsafe { awase_windows::observer::focus_observer::read_os_modifiers() };
            let ctx = runtime::build_input_context(
                app.platform_state.preconditions(),
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
pub(super) unsafe fn handle_wm_execute_effects(app: &mut Runtime) {
    app.executor.drain_deferred();
}

/// WM_PANIC_RESET ハンドラ
pub(super) unsafe fn handle_wm_panic_reset(app: &mut Runtime) {
    app.panic_reset();
}

/// WM_DUPLICATE_INSTANCE ハンドラ
pub(super) unsafe fn handle_wm_duplicate_instance(app: &mut Runtime) {
    log::info!("Duplicate instance notification received");
    app.executor
        .platform
        .tray
        .show_balloon("awase", "awase はすでに起動しています");
}

/// WM_IME_KEY_DETECTED ハンドラ
pub(super) unsafe fn handle_wm_ime_key_detected(app: &mut Runtime) {
    if app.platform_state.sync_key_gate.active
        || !app.platform_state.sync_key_gate.deferred_keys.is_empty()
    {
        app.process_deferred_keys();
    } else {
        app.refresh_ime_state_cache();
    }
}

/// WM_POWERBROADCAST ハンドラ
pub(super) unsafe fn handle_wm_powerbroadcast(app: &mut Runtime, pbt: usize) {
    if pbt == 0x12 || pbt == 0x07 {
        log::info!("Power resume detected (PBT=0x{pbt:02X}), scheduling deferred recovery");
        app.executor.platform.timer.kill(TIMER_IME_REFRESH);
        app.executor.platform.timer.set(
            TIMER_POWER_RESUME,
            std::time::Duration::from_secs(3),
        );
    }
}

/// WM_WTSSESSION_CHANGE ハンドラ
pub(super) unsafe fn handle_wts_session_change(app: &mut Runtime, session_event: u32) {
    const WTS_SESSION_LOCK: u32 = 7;
    const WTS_SESSION_UNLOCK: u32 = 8;
    match session_event {
        WTS_SESSION_LOCK => {
            log::info!("Session locked, flushing engine state");
            app.invalidate_engine_context(ContextChange::FocusChanged);
        }
        WTS_SESSION_UNLOCK => {
            log::info!("Session unlocked, scheduling deferred recovery");
            app.executor.platform.timer.kill(TIMER_IME_REFRESH);
            app.executor.platform.timer.set(
                TIMER_POWER_RESUME,
                std::time::Duration::from_secs(3),
            );
        }
        _ => {}
    }
}

/// WM_INPUTLANGCHANGE ハンドラ
pub(super) unsafe fn handle_wm_inputlangchange(app: &mut Runtime) {
    log::info!("Input language changed, flushing pending state");
    app.invalidate_engine_context(ContextChange::InputLanguageChanged);
    app.refresh_ime_state_cache();
    check_keyboard_layout_on_change();
}

/// WM_PROCESS_DEFERRED ハンドラ
pub(super) unsafe fn handle_wm_process_deferred(app: &mut Runtime) {
    app.process_deferred_keys();
}

/// WM_FOCUS_KIND_UPDATE ハンドラ
pub(super) unsafe fn handle_wm_focus_kind_update(app: &mut Runtime, wparam: usize, lparam: isize) {
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
        if app_kind_u8 != 0xFF {
            let app_kind = awase::types::AppKind::from_u8(app_kind_u8);
            app.platform_state.focus.app_kind = app_kind;
            log::debug!("UIA AppKind update: {app_kind:?}");
        }

        if kind != FocusKind::Undetermined {
            app.platform_state.focus.focus_kind = kind;

            if let Some((pid, cls)) = app.executor.platform.focus.last_focus_info.as_ref() {
                app.executor.platform.focus.cache.insert(
                    *pid,
                    cls.clone(),
                    kind,
                    DetectionSource::UiaAsync,
                );
            }
            if kind == FocusKind::NonText {
                app.invalidate_engine_context(ContextChange::FocusChanged);
            }
        }
    }
}

/// WM_HOTKEY ハンドラ (HOTKEY_ID_TOGGLE)
pub(super) unsafe fn handle_wm_hotkey_toggle(app: &mut Runtime) {
    app.toggle_engine();
}

/// WM_HOTKEY ハンドラ (HOTKEY_ID_FOCUS_OVERRIDE)
pub(super) unsafe fn handle_wm_hotkey_focus_override(app: &mut Runtime) {
    app.toggle_app_override();
}

/// WM_APP (トレイメッセージ) ハンドラ
pub(super) unsafe fn handle_wm_app_tray(hwnd: HWND, lparam: LPARAM) {
    log::debug!("WM_APP received: hwnd={:?} lparam=0x{:016X}", hwnd, lparam.0);
    let layout_names: Vec<String> = with_app_ref(|app| {
        app.layouts.iter().map(|e| e.name.clone()).collect()
    })
    .unwrap_or_default();
    tray::handle_tray_message(
        hwnd,
        lparam,
        &layout_names,
        ELEVATED.load(Ordering::Relaxed),
    );
}

/// WM_RELOAD_CONFIG ハンドラ
pub(super) fn handle_wm_reload_config() {
    log::info!("Config reload requested via WM_RELOAD_CONFIG");
    reload_config();
}

/// WM_COMMAND ハンドラ
pub(super) unsafe fn handle_wm_command(wparam: WPARAM) {
    if let Some(cmd) = tray::handle_tray_command(wparam) {
        if cmd == tray::cmd_settings() {
            launch_settings();
        } else if cmd == tray::cmd_restart_admin() {
            tray::restart_as_admin();
        } else if cmd == tray::cmd_toggle() {
            let _ = with_app(Runtime::toggle_engine);
        } else if cmd == tray::cmd_exit() {
            PostQuitMessage(0);
        } else if cmd >= tray::cmd_layout_base() {
            let index = usize::from(cmd - tray::cmd_layout_base());
            let _ = with_app(|app| app.switch_layout(index));
        }
    }
}

/// WM_DRAIN_OUTPUT_QUEUE ハンドラ
pub(super) unsafe fn handle_wm_drain_output_queue() {
    let _ = with_app(|runtime| {
        runtime.executor.platform.flush_raw_tsf_literal_recovery();
    });

    // classify 済みイベントを取り出し、enrich_ime_relevance（sync key 判定）のみ with_app 内で補完する。
    let queue = {
        let mut events = awase_windows::INPUT_DEFER.take_all();
        let _ = with_app(|app| {
            for ev in &mut events {
                app.enrich_ime_relevance(ev);
                log::debug!("[drain] vk=0x{:02X} {:?}", ev.vk_code.0, ev.event_type);
            }
        });
        events
    };

    if !queue.is_empty() {
        let now_us = hook::now_timestamp_us();
        let _ = with_app(|app| {
            let synthetic_keyups = synthesize_missing_keyups(&queue);
            for syn in &synthetic_keyups {
                log::debug!(
                    "[output-drain] vk=0x{:02X} KeyDown has no paired KeyUp in queue → will inject synthetic KeyUp",
                    syn.vk_code.0,
                );
            }

            for queued_event in &queue {
                log::debug!(
                    "[output-drain] replay vk=0x{:02X} {:?} event_ts={}us now={}us delta={}ms",
                    queued_event.vk_code.0,
                    queued_event.event_type,
                    queued_event.timestamp,
                    now_us,
                    now_us.saturating_sub(queued_event.timestamp) / 1000,
                );
                on_key_event_impl(app, *queued_event);
            }

            for keyup in synthetic_keyups {
                log::debug!(
                    "[output-drain] synthetic KeyUp vk=0x{:02X} (KeyDown replayed, KeyUp arrived before drain)",
                    keyup.vk_code.0,
                );
                on_key_event_impl(app, keyup);
            }
        });
    }
}

/// キューの RawKeyEvent リストから、対応する KeyUp を持たない KeyDown に対して
/// synthetic KeyUp を生成して返す。
fn synthesize_missing_keyups(events: &[awase::types::RawKeyEvent]) -> Vec<awase::types::RawKeyEvent> {
    use awase::types::KeyEventType;
    events.iter()
        .filter(|ev| matches!(ev.event_type, KeyEventType::KeyDown))
        .filter(|ev| !events.iter().any(|e| {
            e.vk_code == ev.vk_code
                && matches!(e.event_type, KeyEventType::KeyUp)
                && e.timestamp >= ev.timestamp
        }))
        .map(|ev| {
            let mut keyup = *ev;
            keyup.event_type = KeyEventType::KeyUp;
            keyup
        })
        .collect()
}

/// TaskbarCreated ハンドラ（Explorer 再起動時にトレイアイコンを復元）
pub(super) unsafe fn handle_taskbar_created(app: &mut Runtime) {
    log::info!("Explorer restarted, re-registering tray icon");
    app.executor.platform.tray.recreate();
}
