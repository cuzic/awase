//! キーイベント処理パイプライン
//!
//! `on_key_event_impl` の処理を段階的に分割したもの。
//! フックコールバック本体から `KeyEventPipeline::run` を呼ぶことで
//! 同じ動作をより読みやすい形で表現する。

use awase::engine::InputModeState;
use awase::types::{RawKeyEvent, ShadowImeAction};
use awase_windows::hook;
use awase_windows::runtime;
use awase_windows::hook::CallbackResult;
use awase_windows::win32::{post_to_main_thread};
use awase_windows::{Runtime, ShadowSource, TIMER_IME_REFRESH, WM_EXECUTE_EFFECTS, WM_PANIC_RESET};
use win32_async;

use super::RAPID_IME_TIMESTAMPS;

/// キーイベント処理パイプライン
pub(super) struct KeyEventPipeline<'a> {
    pub app: &'a mut Runtime,
}

impl<'a> KeyEventPipeline<'a> {
    /// パイプラインを実行し、`CallbackResult` を返す
    pub fn run(mut self, event: RawKeyEvent) -> CallbackResult {
        let mut event = event;
        self.app.enrich_ime_relevance(&mut event);

        // TsfGate: PendingWarmup 中はキーを保留し TSF モード確定を待つ。
        // run_with_prefetched 完了後に OUTPUT_PENDING_QUEUE 経由で再処理される。
        if self.app.executor.platform.output.try_hold_key(event) {
            return CallbackResult::Consumed;
        }

        self.stage_focus_probe(&mut event);
        self.stage_shadow_ime_toggle(&event);
        self.stage_panic_reset_detect(&event);

        let ctx = runtime::build_input_context(
            self.app.platform_state.preconditions(),
            &event.modifier_snapshot,
        );
        let decision = self.app.engine.on_input(event, &ctx);

        self.stage_post_decision(&decision, &event);

        self.stage_execute(decision, &event)
    }

    /// フォーカス切替直後の非同期プローブ
    fn stage_focus_probe(&mut self, _event: &mut RawKeyEvent) {
        if !self.app.platform_state.focus_transition_pending {
            return;
        }
        self.app.platform_state.focus_transition_pending = false;

        // async probe が完了する前に最初のキーが来た場合、warm epoch を抑制して
        // is_composition_warm() が前ウィンドウの stale な warm 状態を返さないようにする。
        // eager_warmup_sent_ms は保持し、phase3_7 が送信した eager F2 のタイムスタンプを
        // 消さないことで non-eager 1500ms パスへの劣化を防ぐ。
        self.app.executor.platform.output.suppress_warm_epoch();
        // キャプチャ（async タスク内で使う）
        let probe_started_ms = hook::current_tick_ms();
        let warmup_ms = self.app.executor.platform.output.eager_warmup_sent_ms();
        let gji_last_io_ms =
            awase_windows::tsf::observer::with_tsf_obs(|obs| obs.gji_last_io_ms());
        let last_focus_change_ms = self.app.platform_state.last_focus_change_ms;
        let shadow_on = self.app.executor.platform.output.shadow_ime_on();

        win32_async::spawn_local(async move {
            let probe = awase_windows::ime::fast_ime_probe_async().await;
            awase_windows::with_app(|app| {
                apply_focus_probe_to_app(
                    app,
                    probe,
                    probe_started_ms,
                    warmup_ms,
                    gji_last_io_ms,
                    last_focus_change_ms,
                    shadow_on,
                );
            });
        });
    }

    /// Shadow IME トグル処理
    fn stage_shadow_ime_toggle(&mut self, event: &RawKeyEvent) {
        if !matches!(event.event_type, awase::types::KeyEventType::KeyDown) {
            return;
        }
        let sync_source = event.ime_relevance.sync_direction.map(|a| (a, ShadowSource::SyncKey));
        let phys_source = if self.app.platform_state.is_japanese_ime() {
            event.ime_relevance.shadow_action.map(|a| (a, ShadowSource::PhysicalImeKey))
        } else {
            None
        };
        let action_with_source = sync_source.or(phys_source);
        if let Some((action, source)) = action_with_source {
            let current = self.app.platform_state.ime_on();
            let new_val = match action {
                ShadowImeAction::Toggle => !current,
                ShadowImeAction::TurnOn => true,
                ShadowImeAction::TurnOff => false,
            };
            let ms = hook::current_tick_ms();
            match source {
                ShadowSource::SyncKey => self.app.platform_state.write_sync_key(new_val, ms),
                ShadowSource::PhysicalImeKey => self.app.platform_state.write_physical_key(new_val, ms),
                _ => {}
            }
            self.app.platform_state.apply_ime_observations(self.app.engine.is_user_enabled());
            if self.app.platform_state.ime_on() != current {
                self.app.platform_state.reset_ime_detect_state();
                log::debug!(
                    "Shadow IME toggle: {} → {} (vk=0x{:02X}, source={:?})",
                    if current { "ON" } else { "OFF" },
                    if self.app.platform_state.ime_on() { "ON" } else { "OFF" },
                    event.vk_code.0,
                    source,
                );
            }
        }
    }

    /// パニックリセット検出
    fn stage_panic_reset_detect(&mut self, event: &RawKeyEvent) {
        if !event.ime_relevance.may_change_ime {
            return;
        }
        if !matches!(event.event_type, awase::types::KeyEventType::KeyDown) {
            return;
        }
        let now = hook::current_tick_ms();
        // SAFETY: RAPID_IME_TIMESTAMPS is a SingleThreadCell; the hook callback runs on the main thread.
        unsafe {
            if let Some(tracker) = RAPID_IME_TIMESTAMPS.get_mut() {
                if tracker.push(now) {
                    tracker.clear();
                    log::warn!("Rapid IME key press detected — requesting panic reset");
                    post_to_main_thread(WM_PANIC_RESET);
                }
            }
        }
    }

    /// Engine 判断後の後処理（IME 制御キー検出 + may_change_ime パススルー）
    fn stage_post_decision(&mut self, decision: &awase::engine::Decision, event: &RawKeyEvent) {
        if let Some(new_ime_on) = decision.find_ime_set_open() {
            let ms = hook::current_tick_ms();
            self.app.platform_state.write_set_open_request(new_ime_on, ms);
            self.app.platform_state.apply_ime_observations(self.app.engine.is_user_enabled());
            self.app.platform_state.reset_ime_detect_state();
            self.app.executor.platform.timer.kill(TIMER_IME_REFRESH);
            self.app.platform_state.hook.set_suppress_ctrl_bypass(true);
            log::debug!("IME control: preconditions.ime_on = {new_ime_on} (SetOpenRequest), poll suspended, ctrl bypass suppressed");
        }

        if !decision.is_consumed()
            && event.ime_relevance.may_change_ime
            && matches!(event.event_type, awase::types::KeyEventType::KeyDown)
        {
            self.app.schedule_ime_refresh(20);
            log::debug!("may_change_ime key passed through → IME refresh scheduled (20ms)");
        }
    }

    /// Effects の実行（フックからキューに委譲）
    fn stage_execute(self, decision: awase::engine::Decision, event: &RawKeyEvent) -> CallbackResult {
        let hook_result = self.app.executor.execute_from_hook(decision, event);

        if hook_result.has_pending {
            post_to_main_thread(WM_EXECUTE_EFFECTS);
        }

        hook_result.callback
    }
}

/// fast_ime_probe_async の結果を app に適用する（with_app 内で呼ぶ）。
/// stage_focus_probe の旧同期ロジックを async 完了後に実行する版。
fn apply_focus_probe_to_app(
    app: &mut Runtime,
    probe: awase_windows::ime::FastImeProbeResult,
    probe_started_ms: u64,
    warmup_ms: u64,
    gji_last_io_ms: u64,
    last_focus_change_ms: u64,
    shadow_on: bool,
) {
    let probe_age_ms = hook::current_tick_ms().saturating_sub(probe_started_ms);
    let ime_on_before_probe = app.platform_state.ime_on();
    let input_mode_before_probe = app.platform_state.input_mode();

    app.platform_state.set_is_japanese_ime(probe.is_japanese_ime);

    const WARMUP_GRACE_MS: u64 = 300;
    const GJI_SETTLE_GRACE_MS: u64 = 300;
    const SHADOW_GRACE_MS: u64 = 200;

    let now_ms = hook::current_tick_ms();

    let warmup_elapsed = if warmup_ms > 0 { now_ms.saturating_sub(warmup_ms) } else { u64::MAX };
    let sig1_warmup = warmup_elapsed < WARMUP_GRACE_MS;

    let gji_active_after_focus =
        gji_last_io_ms > 0 && gji_last_io_ms >= last_focus_change_ms;
    let gji_idle_ms = if gji_last_io_ms > 0 {
        now_ms.saturating_sub(gji_last_io_ms)
    } else {
        u64::MAX
    };
    let sig2_gji = gji_active_after_focus && gji_idle_ms < GJI_SETTLE_GRACE_MS;

    let sig3_shadow = shadow_on && probe_age_ms < SHADOW_GRACE_MS;

    let mut suppressed = false;
    let mut suppress_reason = "";
    if let Some(on) = probe.ime_on {
        let effective = on && probe.is_japanese_ime;
        if !effective && (sig1_warmup || sig2_gji || sig3_shadow) {
            suppressed = true;
            suppress_reason = if sig1_warmup {
                "warmup"
            } else if sig2_gji {
                "gji-io"
            } else {
                "shadow"
            };
        } else {
            app.platform_state.set_os_ime_on(Some(effective));
            let ms = hook::current_tick_ms();
            app.platform_state.write_focus_probe(effective, ms);
            if effective {
                app.platform_state.reset_ime_detect_state();
            }
            app.platform_state
                .apply_ime_observations(app.engine.is_user_enabled());
        }
    }

    if let Some(romaji) = probe.is_romaji {
        let prev = app.platform_state.input_mode().is_romaji_capable();
        app.platform_state.set_input_mode(if romaji {
            InputModeState::ObservedRomaji
        } else {
            InputModeState::ObservedKana
        });
        if prev != romaji {
            log::info!(
                "FocusProbe +{}ms: mode {} → {}",
                probe_age_ms,
                if prev { "romaji" } else { "kana" },
                if romaji { "romaji" } else { "kana" },
            );
        }
    }

    let ime_on_after_probe = app.platform_state.ime_on();
    let input_mode_after_probe = app.platform_state.input_mode();
    let ime_on_suffix = if suppressed {
        let detail = match suppress_reason {
            "warmup" => format!("warmup:{warmup_elapsed}ms"),
            "gji-io" => format!("gji-io:{gji_idle_ms}ms"),
            _ => format!("shadow:{probe_age_ms}ms"),
        };
        format!("(suppressed:{detail})")
    } else if probe.ime_on.is_none() {
        "(stale)".to_string()
    } else {
        String::new()
    };
    log::info!(
        "FocusProbe +{}ms: ime_on={}{} mode={:?}{} [gji_io={}ms sig1={sig1_warmup} sig2={sig2_gji} sig3={sig3_shadow}]",
        probe_age_ms,
        ime_on_after_probe,
        ime_on_suffix,
        input_mode_after_probe,
        if probe.is_romaji.is_none() { "(stale)" } else { "" },
        if gji_idle_ms == u64::MAX { "never".to_string() } else { gji_idle_ms.to_string() },
    );
    if suppressed {
        log::debug!(
            "FocusProbe: imc_open=false を抑制 (reason={suppress_reason}) — Engine deactivation を防止"
        );
    } else if probe.ime_on.is_none() {
        log::warn!(
            "FocusProbe: ime_on 未検出 — stale値 {} が ObserverPoll まで持続 \
             [probe_age={}ms, A/B判断: ime_on stale頻度を確認]",
            ime_on_before_probe,
            probe_age_ms,
        );
    }
    if probe.is_romaji.is_none() {
        log::warn!(
            "FocusProbe: input_mode 未検出 — stale値 {:?} が ObserverPoll まで持続 \
             [probe_age={}ms, A/B判断: mode stale頻度を確認]",
            input_mode_before_probe,
            probe_age_ms,
        );
    }
}
