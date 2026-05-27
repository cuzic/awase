//! キーイベント処理パイプライン
//!
//! `on_key_event_impl` の処理を段階的に分割したもの。
//! フックコールバック本体から `KeyEventPipeline::run` を呼ぶことで
//! 同じ動作をより読みやすい形で表現する。

use awase::types::{RawKeyEvent, ShadowImeAction};
use crate::hook;
use crate::runtime;
use crate::hook::CallbackResult;
use crate::win32::{post_to_main_thread};
use crate::{Runtime, ShadowSource, TIMER_IME_REFRESH, WM_EXECUTE_EFFECTS};

/// キーイベント処理パイプライン
pub(super) struct KeyEventPipeline<'a> {
    pub app: &'a mut Runtime,
}

impl KeyEventPipeline<'_> {
    /// パイプラインを実行し、`CallbackResult` を返す
    pub(super) fn run(mut self, event: RawKeyEvent) -> CallbackResult {
        let mut event = event;
        self.app.enrich_ime_relevance(&mut event);

        // TsfGate: PendingWarmup 中はキーを保留し TSF モード確定を待つ。
        // run_with_prefetched 完了後に OUTPUT_PENDING_QUEUE 経由で再処理される。
        if self.app.executor.platform.output.try_hold_key(event) {
            return CallbackResult::Consumed;
        }

        self.stage_focus_probe(&mut event);
        let shadow_toggled = self.stage_shadow_ime_toggle(&event);

        let ctx = runtime::build_input_context(
            self.app.platform_state.belief(),
            &event.modifier_snapshot,
        );
        // [engine-input] order-bug 調査用: drain と inline の処理順序を可視化する。
        // event.timestamp はユーザー押下時刻(us)、now はエンジン入力到達時刻(us)。
        // delay_ms が大きいほど drain 経由（古い event_ts が遅延処理されている）。
        // state は on_input 直前の FSM 状態、pending_drain は INPUT_DEFER の未処理件数。
        //
        // Ctrl 残留調査用: modifier_snapshot は hook 時点 (capture 時) の Ctrl/Shift/Alt/Win、
        // gas_ctrl は engine 入力到達時の `GetAsyncKeyState(VK_CONTROL)` 生値 (= OS が思う
        // 物理 Ctrl)、extra は injection marker (0=物理キー、INJECTED/IME_KANJI_MARKER=自己注入)。
        // この 3 つが揃うと「engine 認識/OS 認識/由来」が一行で判別でき、SendInput 後に
        // OS 側 Ctrl がスタックしているか、modifier_snapshot が古い値で残っているかを切り分けられる。
        let now_us = hook::now_timestamp_us();
        let delay_ms = now_us.saturating_sub(event.timestamp) / 1000;
        let pending_drain = crate::INPUT_DEFER.pending_len_nonblocking().unwrap_or(0);
        let gate_active = crate::OUTPUT_GATE.is_active();
        let mods = event.modifier_snapshot;
        // SAFETY: GetAsyncKeyState はスレッドセーフで任意のスレッドから呼べる。
        let gas_ctrl = unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
            GetAsyncKeyState(i32::from(crate::vk::VK_CONTROL.0)) as u16 & 0x8000 != 0
        };
        log::debug!(
            "[engine-input] vk=0x{:02X} {:?} ts={}us delay={}ms state={} \
             mods(c={} s={} a={} w={}) gas_ctrl={} extra=0x{:X} \
             pending_drain={} gate_active={}",
            event.vk_code, event.event_type, event.timestamp,
            delay_ms,
            self.app.engine.debug_state_label(),
            mods.ctrl, mods.shift, mods.alt, mods.win,
            gas_ctrl,
            event.extra_info,
            pending_drain, gate_active,
        );
        let decision = self.app.engine.on_input(event, &ctx);

        self.stage_post_decision(&decision, &event);

        self.stage_execute(decision, &event, shadow_toggled)
    }

    /// フォーカス切替直後の非同期プローブ
    fn stage_focus_probe(&mut self, _event: &mut RawKeyEvent) {
        if !self.app.platform_state.focus.focus_transition_pending {
            return;
        }
        self.app.platform_state.focus.focus_transition_pending = false;

        // async probe が完了する前に最初のキーが来た場合、warm epoch を抑制して
        // is_composition_warm() が前ウィンドウの stale な warm 状態を返さないようにする。
        // eager_warmup_sent_ms は保持し、phase3_7 が送信した eager F2 のタイムスタンプを
        // 消さないことで non-eager 1500ms パスへの劣化を防ぐ。
        self.app.executor.platform.output.reset_warm_epoch();
        // キャプチャ（async タスク内で使う）
        let probe_started_ms = hook::current_tick_ms();
        let warmup_ms = self.app.executor.platform.output.eager_warmup_sent_ms();
        let obs = crate::state::ObservedState::capture_now();
        let gji_last_io_ms = obs.gji_last_io_ms;
        let last_focus_change_ms = self.app.platform_state.focus.last_focus_change_ms;
        let shadow_on = self.app.executor.platform.output.last_applied_ime_on();

        win32_async::spawn_local(async move {
            let probe = crate::ime::read_ime_state_fast_async().await;
            let _ = crate::with_app(|app| {
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
    ///
    /// IME ON/OFF が変化したら `true` を返す。`stage_execute` がこの値を見て
    /// Imm32Unavailable アプリで物理 IME キーを抑止すべきか判定する。
    fn stage_shadow_ime_toggle(&mut self, event: &RawKeyEvent) -> bool {
        if !matches!(event.event_type, awase::types::KeyEventType::KeyDown) {
            return false;
        }
        let sync_source = event.ime_relevance.sync_direction.map(|a| (a, ShadowSource::SyncKey));
        let phys_source = if self.app.platform_state.is_japanese_ime() {
            event.ime_relevance.shadow_action.map(|a| (a, ShadowSource::PhysicalImeKey))
        } else {
            None
        };
        let action_with_source = sync_source.or(phys_source);
        let Some((action, source)) = action_with_source else { return false; };

        let current = self.app.platform_state.ime_on();
        let new_val = match action {
            ShadowImeAction::Toggle => !current,
            ShadowImeAction::TurnOn => true,
            ShadowImeAction::TurnOff => false,
        };
        let ms = hook::current_tick_ms();
        let user_enabled = self.app.engine.is_user_enabled();
        match source {
            ShadowSource::SyncKey => self.app.platform_state.write_sync_key(new_val, ms, user_enabled),
            ShadowSource::PhysicalImeKey => self.app.platform_state.write_physical_key(new_val, ms, user_enabled),
            _ => {}
        }
        if self.app.platform_state.ime_on() == current {
            return false;
        }
        self.app.platform_state.reset_ime_detect_state();

        // ON→OFF の場合、OS IME を明示的に OFF にする。
        // activation (inactive→active) が ImeEffect::SetOpen(true) を生成して OS IME を
        // 強制 ON するのと対称な処理。deactivation は SetOpen(false) を生成しないため、
        // TSF モード (WezTerm 等) では物理キー reinject だけでは OS IME が OFF にならない。
        //
        // Imm32Unavailable (Chrome/Edge) では VK_KANJI が唯一の IME クローズ手段であり、
        // KanjiToggleStrategy が shadow_on (latch) を見て送信するかを決める。
        // ここでは latch が true のうちに strategy chain を起動することで VK_KANJI が
        // 確実に送られる。
        //
        // IMM クロスプロセス対応アプリ (WezTerm 等の TSF mode) は SendMessageTimeoutW を
        // 含む sync `set_ime_open_cross_process` がフック内で `with_app` 再入を引き起こす
        // ため、async に spawn_local + OutputActiveGuard で dispatch する。
        // それ以外 (GjiDirect / KanjiToggle) は SendInput-only で非ブロッキングなので sync。
        if !self.app.platform_state.ime_on() {
            let view = self.app.executor.platform.build_ime_control_view();
            let imm_first =
                crate::ime_controller::CONTROLLER.imm_cross_is_first_applicable(&view);
            if imm_first {
                let guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
                win32_async::spawn_local(async move {
                    let ok = crate::ime::set_ime_open_cross_process_async(false).await;
                    if !ok {
                        log::debug!(
                            "[shadow-toggle] ImmCross failed (async), trying fallback"
                        );
                        let _ = crate::with_app(|app| {
                            let view = app.executor.platform.build_ime_control_view();
                            crate::ime_controller::CONTROLLER.apply_skipping_imm(false, &view)
                        });
                    }
                    drop(guard);
                });
            } else {
                let _ = crate::ime_controller::CONTROLLER.apply(false, &view);
            }
            self.app.executor.platform.output.set_ime_apply_latch(false);
            log::debug!("[shadow-toggle] ON→OFF: apply_ime_open(false) dispatched + latch=false");
        }
        log::debug!(
            "Shadow IME toggle: {} → {} (vk=0x{:02X}, source={:?})",
            if current { "ON" } else { "OFF" },
            if self.app.platform_state.ime_on() { "ON" } else { "OFF" },
            event.vk_code,
            source,
        );
        true
    }

    /// Engine 判断後の後処理（IME 制御キー検出 + may_change_ime パススルー）
    fn stage_post_decision(&mut self, decision: &awase::engine::Decision, event: &RawKeyEvent) {
        if let Some(new_ime_on) = decision.find_ime_set_open() {
            let ms = hook::current_tick_ms();
            self.app.platform_state.write_set_open_request(new_ime_on, ms, self.app.engine.is_user_enabled());
            self.app.platform_state.reset_ime_detect_state();
            self.app.executor.platform.timer.kill(TIMER_IME_REFRESH);
            self.app.platform_state.hook.set_ctrl_bypass_hold(true);
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
    fn stage_execute(
        self,
        decision: awase::engine::Decision,
        event: &RawKeyEvent,
        shadow_toggled: bool,
    ) -> CallbackResult {
        // 物理 IME キー（VK_KANJI 等）を OS に届けないアプリ（Imm32Unavailable: Chrome/Edge）で
        // shadow が今変化した場合、apply_ime_open が VK_KANJI を送信済み。
        // 物理キーをそのまま届けると VK_KANJI + 物理キーの二重制御になるため抑止する。
        // PassThrough / PassThroughWith → Consume に変換して reinject/passthrough をスキップする。
        let suppress_physical = shadow_toggled
            && !self
                .app
                .executor
                .platform
                .focus
                .current_app_profile()
                .should_pass_physical_key();
        let decision = if suppress_physical {
            match decision {
                awase::engine::Decision::PassThrough => {
                    log::debug!("[imm32-off] physical IME key consume (was PassThrough, double-send prevented)");
                    awase::engine::Decision::Consume { effects: vec![].into() }
                }
                awase::engine::Decision::PassThroughWith { effects } => {
                    log::debug!("[imm32-off] physical IME key consume (was PassThroughWith, double-send prevented)");
                    awase::engine::Decision::Consume { effects }
                }
                other => other,
            }
        } else {
            decision
        };

        let hook_result = self.app.executor.execute_from_hook(decision, event);

        if hook_result.has_pending {
            post_to_main_thread(WM_EXECUTE_EFFECTS);
        }

        hook_result.callback
    }
}

/// フォーカスプローブの IME 更新抑制シグナルをまとめた値
struct FocusProbeGraceFlags {
    warmup_grace_active: bool,
    gji_grace_active: bool,
    shadow_grace_active: bool,
    warmup_elapsed: u64,
    gji_idle_ms: u64,
}

impl FocusProbeGraceFlags {
    const fn any(&self) -> bool {
        self.warmup_grace_active || self.gji_grace_active || self.shadow_grace_active
    }

    const fn primary_reason(&self) -> &'static str {
        if self.warmup_grace_active {
            "warmup"
        } else if self.gji_grace_active {
            "gji-io"
        } else {
            "shadow"
        }
    }
}

const fn compute_focus_probe_grace(
    now_ms: u64,
    probe_age_ms: u64,
    warmup_ms: u64,
    gji_last_io_ms: u64,
    last_focus_change_ms: u64,
    shadow_on: bool,
) -> FocusProbeGraceFlags {
    let warmup_elapsed = if warmup_ms > 0 {
        now_ms.saturating_sub(warmup_ms)
    } else {
        u64::MAX
    };
    let warmup_grace_active = warmup_elapsed < crate::tuning::WARMUP_GRACE_MS;

    let gji_active_after_focus = gji_last_io_ms > 0 && gji_last_io_ms >= last_focus_change_ms;
    let gji_idle_ms = if gji_last_io_ms > 0 {
        now_ms.saturating_sub(gji_last_io_ms)
    } else {
        u64::MAX
    };
    let gji_grace_active = gji_active_after_focus && gji_idle_ms < crate::tuning::GJI_SETTLE_GRACE_MS;

    let shadow_grace_active = shadow_on && probe_age_ms < crate::tuning::SHADOW_GRACE_MS;

    FocusProbeGraceFlags { warmup_grace_active, gji_grace_active, shadow_grace_active, warmup_elapsed, gji_idle_ms }
}

fn apply_effective_ime(app: &mut Runtime, effective: bool) {
    let ms = hook::current_tick_ms();
    if effective {
        app.platform_state.reset_ime_detect_state();
    }
    app.platform_state.write_focus_probe(effective, ms, app.engine.is_user_enabled());
}

#[allow(clippy::option_if_let_else)]
fn build_ime_on_suffix(
    probe_ime_on: Option<bool>,
    suppressed_reason: Option<&'static str>,
    signals: &FocusProbeGraceFlags,
    probe_age_ms: u64,
) -> String {
    if let Some(reason) = suppressed_reason {
        let detail = match reason {
            "warmup" => format!("warmup:{}ms", signals.warmup_elapsed),
            "gji-io" => format!("gji-io:{}ms", signals.gji_idle_ms),
            _ => format!("shadow:{probe_age_ms}ms"),
        };
        format!("(suppressed:{detail})")
    } else if probe_ime_on.is_none() {
        "(stale)".to_string()
    } else {
        String::new()
    }
}

/// read_ime_state_fast_async の結果を app に適用する（with_app 内で呼ぶ）。
/// stage_focus_probe の旧同期ロジックを async 完了後に実行する版。
#[allow(clippy::needless_pass_by_value, clippy::option_if_let_else)]
fn apply_focus_probe_to_app(
    app: &mut Runtime,
    probe: crate::ime::FastImeProbeResult,
    probe_started_ms: u64,
    warmup_ms: u64,
    gji_last_io_ms: u64,
    last_focus_change_ms: u64,
    shadow_on: bool,
) {
    let probe_age_ms = hook::current_tick_ms().saturating_sub(probe_started_ms);
    let ime_on_before_probe = app.platform_state.ime_on();

    app.platform_state.set_is_japanese_ime(probe.is_japanese_ime);

    let now_ms = hook::current_tick_ms();
    let signals = compute_focus_probe_grace(
        now_ms, probe_age_ms, warmup_ms, gji_last_io_ms, last_focus_change_ms, shadow_on,
    );

    let suppressed_reason: Option<&'static str> = if let Some(on) = probe.ime_on {
        let effective = on && probe.is_japanese_ime;
        if !effective && signals.any() {
            Some(signals.primary_reason())
        } else {
            apply_effective_ime(app, effective);
            None
        }
    } else {
        None
    };

    let ime_on_after_probe = app.platform_state.ime_on();
    let input_mode_after_probe = app.platform_state.input_mode();
    let ime_on_suffix = build_ime_on_suffix(probe.ime_on, suppressed_reason, &signals, probe_age_ms);

    log::info!(
        "FocusProbe +{}ms: ime_on={}{} mode={:?} [gji_io={}ms sig1={} sig2={} sig3={}]",
        probe_age_ms,
        ime_on_after_probe,
        ime_on_suffix,
        input_mode_after_probe,
        if signals.gji_idle_ms == u64::MAX { "never".to_string() } else { signals.gji_idle_ms.to_string() },
        signals.warmup_grace_active,
        signals.gji_grace_active,
        signals.shadow_grace_active,
    );

    match suppressed_reason {
        Some(reason) => log::debug!(
            "FocusProbe: imc_open=false を抑制 (reason={reason}) — Engine deactivation を防止"
        ),
        None if probe.ime_on.is_none() => log::warn!(
            "FocusProbe: ime_on 未検出 — stale値 {ime_on_before_probe} が ObserverPoll まで持続 \
             [probe_age={probe_age_ms}ms, A/B判断: ime_on stale頻度を確認]",
        ),
        None => {}
    }
}
