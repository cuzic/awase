//! キーイベント処理パイプライン
//!
//! `on_key_event_impl` の処理を段階的に分割したもの。
//! フックコールバック本体から `Runtime::process_key_event` を呼ぶことで
//! 同じ動作をより読みやすい形で表現する。

use crate::hook;
use crate::hook::CallbackResult;
use crate::win32::post_to_main_thread;
use crate::{Runtime, TIMER_IME_REFRESH, WM_EXECUTE_EFFECTS};
use awase::platform::PlatformRuntime as _;
use awase::types::{RawKeyEvent, ShadowImeAction};

/// Shadow IME トグルの意図ソース (この pipeline 内のローカル routing 用)。
#[derive(Debug, Clone, Copy)]
enum IntentKind {
    /// config 由来の同期キー
    SyncKey,
    /// 物理 KANJI キー
    PhysicalImeKey,
}

impl Runtime {
    /// キーイベント処理エントリポイント
    pub(crate) fn process_key_event(&mut self, event: RawKeyEvent) -> CallbackResult {
        self.kp_run_inner(event, false)
    }

    /// TIMER_IME_OFF_RESCUE 満了時の再処理エントリポイント。
    /// 救済窓 defer をスキップして即時処理する（無限ループ防止）。
    pub(crate) fn replay_ime_off_rescue_event(&mut self, event: RawKeyEvent) -> CallbackResult {
        self.kp_run_inner(event, true)
    }

    /// パイプライン実装。`skip_rescue_defer=true` で救済窓 defer をスキップ。
    #[expect(clippy::cognitive_complexity)]
    #[expect(clippy::too_many_lines)]
    fn kp_run_inner(&mut self, mut event: RawKeyEvent, skip_rescue_defer: bool) -> CallbackResult {
        self.enrich_ime_relevance(&mut event);

        // TsfGate: PendingWarmup 中はキーを保留し TSF モード確定を待つ。
        // run_with_prefetched 完了後に OUTPUT_PENDING_QUEUE 経由で再処理される。
        if self.platform.try_hold_key(event) {
            log::debug!(
                "[tsf-gate-hold] vk=0x{:02X} {:?} held by TsfGate (PendingWarmup)",
                event.vk_code,
                event.event_type
            );
            return CallbackResult::Consumed;
        }

        // Phase A: 既存の pending IME OFF rescue を解決する。
        // 現在 event が Ctrl↑ なら保留キーを破棄（thumb shift 防止）、
        // それ以外なら救済中止（原 event を発火 → IME OFF）。
        // Ctrl↑ 以外は skip_rescue_defer=true でネスト呼び出しし、
        // 再 defer による無限ループを防ぐ。
        if let Some(pending_event) = self.take_ime_off_rescue_pending() {
            let is_ctrl_up = matches!(event.event_type, awase::types::KeyEventType::KeyUp)
                && crate::vk::is_ctrl_variant(event.vk_code);
            if is_ctrl_up {
                // Ctrl↑ within 50ms: 「Ctrl+他キー中の誤打 無変換」を破棄する。
                // ctrl=false で発火すると NICOLA FSM が PendingThumb に入り thumb shift に
                // 化けてしまうため、無変換を消費する（IME OFF も発火しない）。
                log::info!(
                    "[ime-off-rescue] Ctrl↑ within 50ms → 無変換 vk=0x{:02X} を破棄（thumb shift 防止）",
                    pending_event.vk_code
                );
                // 続けて現在 event (Ctrl↑) を通常処理する
            } else {
                log::info!(
                    "[ime-off-rescue] non-Ctrl↑ event 到着 → 保留 vk=0x{:02X} を IME OFF として発火",
                    pending_event.vk_code
                );
                let inner_result = self.kp_run_inner(pending_event, true);
                // PassThrough なら reinject + WM_EXECUTE_EFFECTS（フックコールバックと同じ後処理）
                if matches!(inner_result, CallbackResult::PassThrough) {
                    self.executor.enqueue_reinject(pending_event);
                    post_to_main_thread(WM_EXECUTE_EFFECTS);
                }
                // 続けて現在 event を通常処理する
            }
        }

        self.kp_stage_focus_probe(&mut event);
        let shadow_toggled = self.kp_stage_shadow_ime_toggle(&event);

        let ctx = super::build_input_context(
            self.platform_state.ime.effective_open(),
            &self.platform_state.ime.belief,
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
        let pending_drain = crate::INPUT_DEFER.pending_len_nonblocking();
        let gate_active = crate::OUTPUT_GATE.is_active();
        let mods = event.modifier_snapshot;
        // SAFETY: GetAsyncKeyState はスレッドセーフで任意のスレッドから呼べる。
        let gas_ctrl = unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
            GetAsyncKeyState(i32::from(crate::vk::VK_CONTROL.0)) < 0
        };
        // phys_ctrl は PHYSICAL_KEY_STATE (SendInput 非影響) での Ctrl 押下状態。
        // gas_ctrl と乖離する場合、synthetic KeyUp が SendInput されて
        // GetAsyncKeyState が汚染されている可能性がある。
        let phys_ctrl = hook::is_physical_key_down(crate::vk::VK_LCONTROL)
            || hook::is_physical_key_down(crate::vk::VK_RCONTROL);
        log::debug!(
            "[engine-input] vk=0x{:02X} {:?} ts={}us delay={}ms state={} \
             mods(c={} s={} a={} w={}) gas_ctrl={} phys_ctrl={} extra=0x{:X} \
             pending_drain={} gate_active={}",
            event.vk_code,
            event.event_type,
            event.timestamp,
            delay_ms,
            self.engine.debug_state_label(),
            mods.ctrl,
            mods.shift,
            mods.alt,
            mods.win,
            gas_ctrl,
            phys_ctrl,
            event.extra_info,
            pending_drain.map_or_else(|| "?".to_owned(), |n| n.to_string()),
            gate_active,
        );
        if !mods.ctrl && phys_ctrl {
            log::warn!(
                "[engine-input] CTRL MISMATCH: mods.ctrl=false だが phys_ctrl=true (vk=0x{:02X} {:?}) \
                 → synthetic Ctrl↑ が GetAsyncKeyState を汚染した可能性がある",
                event.vk_code, event.event_type,
            );
        }
        // Phase B: Ctrl+無変換 IME OFF ミスタイプ救済の defer 判定。
        // 「Ctrl↓ → 他キー consume → 無変換↓」の並びなら 50ms 救済窓を設けて defer する。
        // 「Ctrl↓ → 直後に 無変換↓」の意図的チョードでは ctrl_consumed_since_down=false なので
        // ここを通過せず engine が即 IME OFF を発火する。
        if !skip_rescue_defer
            && matches!(event.event_type, awase::types::KeyEventType::KeyDown)
            && event.modifier_snapshot.ctrl
            && hook::ctrl_consumed_since_down()
            && self.engine.matches_ime_off(&ctx, &event)
        {
            log::debug!(
                "[ime-off-rescue] vk=0x{:02X} を 50ms 保留 (Ctrl consumed)",
                event.vk_code
            );
            self.set_ime_off_rescue_pending(event);
            return CallbackResult::Consumed;
        }

        let state_before = self.engine.debug_state_label();
        let decision = self.engine.on_input(event, &ctx);
        let state_after = self.engine.debug_state_label();
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::KeyInput {
                event: crate::journal::KeyEventSummary::from_raw(&event),
                state_before,
                state_after,
                decision: crate::journal::DecisionKind::from_decision(&decision),
            });

        self.kp_stage_post_decision(&decision, &event);

        // Ctrl 系 KeyUp で chord barrier を解除する。
        // chord 状態の判断は ImeStateHub.on_ctrl_key_up() に集約（パイプラインは VK 分類のみ担う）。
        if !matches!(event.event_type, awase::types::KeyEventType::KeyDown)
            && crate::vk::is_ctrl_variant(event.vk_code)
        {
            self.platform_state.ime.on_ctrl_key_up(event.vk_code);
        }

        self.kp_stage_execute(decision, &event, shadow_toggled)
    }

    /// フォーカス切替直後の非同期プローブ
    fn kp_stage_focus_probe(&mut self, _event: &mut RawKeyEvent) {
        // Step 5: focus_transition_pending: bool は InputBarrier::FocusTransition に置換。
        // 最初のキー入力で barrier を consume する (one-shot 動作維持)。
        if !self.platform_state.ime.consume_focus_barrier() {
            return;
        }

        // キャプチャ（async タスク内で使う）
        let probe_started_ms = hook::current_tick_ms();
        let warmup_ms = self.platform.eager_warmup_sent_ms();
        let obs = crate::state::ObservedState::capture_now();
        let gji_last_io_ms = obs.gji_last_io_ms;
        let last_focus_change_ms = self.platform_state.last_focus_change_ms;
        let shadow_on = self
            .platform_state
            .ime
            .model()
            .applied
            .applied_open()
            .unwrap_or(false);

        win32_async::spawn_local(async move {
            let probe = crate::ime::read_ime_state_fast_async().await;
            let _ = crate::with_app(|app| {
                app.apply_focus_probe(
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
    /// IME ON/OFF が変化したら `true` を返す。`kp_stage_execute` がこの値を見て
    /// Imm32Unavailable アプリで物理 IME キーを抑止すべきか判定する。
    fn kp_stage_shadow_ime_toggle(&mut self, event: &RawKeyEvent) -> bool {
        if !matches!(event.event_type, awase::types::KeyEventType::KeyDown) {
            return false;
        }
        // 同期キー (config sync_direction) > 物理 KANJI (Japanese 限定) の順で意図を採用する。
        let intent_kind = if let Some(a) = event.ime_relevance.sync_direction {
            Some((a, IntentKind::SyncKey))
        } else if self.platform_state.ime.belief.is_japanese_ime() {
            event
                .ime_relevance
                .shadow_action
                .map(|a| (a, IntentKind::PhysicalImeKey))
        } else {
            None
        };
        let Some((action, kind)) = intent_kind else {
            return false;
        };

        let current = self.platform_state.ime.effective_open();
        let new_val = match action {
            ShadowImeAction::Toggle => !current,
            ShadowImeAction::TurnOn => true,
            ShadowImeAction::TurnOff => false,
        };
        match kind {
            IntentKind::SyncKey => self.platform_state.ime.write_sync_key(new_val),
            IntentKind::PhysicalImeKey => self.platform_state.ime.write_physical_key(new_val),
        }
        if self.platform_state.ime.effective_open() == current {
            return false;
        }
        self.platform_state.ime.on_ime_toggled();

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
        if !self.platform_state.ime.effective_open() {
            let view = self.shadow_ime_control_view();
            let imm_first = crate::ime_controller::CONTROLLER.imm_cross_is_first_applicable(&view);
            if imm_first {
                // 楽観的 C: async 完了前から ImeModel を OFF に同期する。
                self.platform_state.ime.mirror_applied_open(false);
                let guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
                win32_async::spawn_local(async move {
                    let ok = crate::ime::set_ime_open_cross_process_async(false).await;
                    let outcome = if ok {
                        awase::platform::ImeOpenOutcome::Applied
                    } else {
                        let actual = unsafe { crate::ime::read_ime_state_fast() }.ime_on;
                        if actual == Some(false) {
                            log::debug!(
                                "[shadow-toggle] ImmCross failed but actual=OFF already, skip fallback"
                            );
                            awase::platform::ImeOpenOutcome::AlreadyMatched
                        } else {
                            log::debug!(
                                "[shadow-toggle] ImmCross failed (async, actual={actual:?}), trying fallback"
                            );
                            crate::with_app(|app| {
                                crate::ime_controller::CONTROLLER
                                    .apply_skipping_imm(false, &app.shadow_ime_control_view())
                            })
                            .unwrap_or(awase::platform::ImeOpenOutcome::Failed)
                        }
                    };
                    // B+C(ts更新)+D(noop)+E
                    let _ = crate::with_app(|app| {
                        app.on_ime_apply_complete(false, outcome);
                    });
                    drop(guard);
                });
            } else {
                let outcome = crate::ime_controller::CONTROLLER.apply(false, &view);
                // B+C+D(noop)+E
                self.on_ime_apply_complete(false, outcome);
            }
            log::debug!("[shadow-toggle] ON→OFF: apply_ime_open(false) dispatched + applied=false");
        }
        log::debug!(
            "Shadow IME toggle: {} → {} (vk=0x{:02X}, source={:?})",
            if current { "ON" } else { "OFF" },
            if self.platform_state.ime.effective_open() {
                "ON"
            } else {
                "OFF"
            },
            event.vk_code,
            kind,
        );
        true
    }

    /// Engine 判断後の後処理（IME 制御キー検出 + may_change_ime パススルー）
    fn kp_stage_post_decision(&mut self, decision: &awase::engine::Decision, event: &RawKeyEvent) {
        if let Some(new_ime_on) = decision.find_ime_set_open() {
            self.platform.timer.kill(TIMER_IME_REFRESH);
            let generation = self.platform_state.ime.event_log.next_seq();
            let applied = self.platform_state.ime.handle_engine_set_open(
                new_ime_on,
                event.modifier_snapshot.ctrl,
                generation,
            );
            log::debug!(
                "IME control: preconditions.ime_on = {new_ime_on} (SetOpenRequest), poll suspended{}",
                if applied { "" } else { " [chord barrier active → skipped]" }
            );
        }

        if !decision.is_consumed()
            && event.ime_relevance.may_change_ime
            && matches!(event.event_type, awase::types::KeyEventType::KeyDown)
        {
            self.schedule_ime_refresh(20);
            log::debug!("may_change_ime key passed through → IME refresh scheduled (20ms)");
        }
    }

    /// Effects の実行（フックからキューに委譲）
    fn kp_stage_execute(
        &mut self,
        decision: awase::engine::Decision,
        event: &RawKeyEvent,
        shadow_toggled: bool,
    ) -> CallbackResult {
        // 物理 IME キー（VK_KANJI / VK_F3 / VK_F4 等）を OS に届けるかは Decision（意味論）
        // とは独立した「配送機構」の判断であり、Decision を書き換えずに
        // PhysicalKeyDisposition で表現する。
        // - Imm32Unavailable (Chrome/Edge): KeyDown は shadow_toggle 発火時のみ、KeyUp は常に
        //   Suppress。VK_KANJI を SendInput 済みなので物理キーを届けると二重制御になり、
        //   0xF3 KeyUp は OS が 0xF4 KeyDown を生成して shadow_toggle が反転する。
        // - ImmCross (LINE/Qt): Down/Up 共に Suppress。set_ime_open_cross_process で IME 制御済み。
        //   物理キー / IMM 注入の KeyUp をアプリに渡すと内部 IME ハンドラが spurious VK_F3/F4 を
        //   生成し shadow_toggle が反転する（IME ON Engine-OFF バグの根本原因）。
        // - TsfNative (WezTerm): TSF が KANJI を正しく処理するため物理キーを通す（従来通り）。
        let profile = self.platform.current_app_profile();
        let is_tsf_mode = self.platform.is_tsf_mode();
        let physical =
            crate::runtime::PhysicalKeyDisposition::plan(event, profile, shadow_toggled, is_tsf_mode);
        if physical == crate::runtime::PhysicalKeyDisposition::Suppress {
            let reason = if event.vk_code == crate::vk::VK_DBE_HIRAGANA {
                "tsf-f2"
            } else if profile.can_use_imm32_cross_process() {
                "imm-cross"
            } else {
                "imm32-off"
            };
            log::debug!(
                "[{reason}] key suppress vk={:#04x} {:?} (physical disposition)",
                event.vk_code,
                event.event_type
            );
        }

        // F2 (VK_DBE_HIRAGANA) KeyDown: CompositionFsm に副作用を委譲。
        // Suppress（TSF mode）・Allow（非 TSF mode）いずれの場合も mark_cold + eager warmup を実行。
        if event.vk_code == crate::vk::VK_DBE_HIRAGANA
            && matches!(event.event_type, awase::types::KeyEventType::KeyDown)
        {
            let applied_open = self.platform_state.ime.model().applied.applied_open();
            self.platform.composition_native_f2_down(applied_open);
        }

        let result = self.executor.execute_from_hook(
            &mut self.platform,
            &self.platform_state.ime,
            decision,
            event,
            physical,
        );
        // sync path の outcome を on_ime_apply_complete（B+C+D+E）に渡す。
        // Filter mode では IME effects がキューへ委譲されるため通常は空。
        self.dispatch_outcomes(result.sync_outcomes);

        if result.has_pending {
            post_to_main_thread(WM_EXECUTE_EFFECTS);
        }

        result.callback
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
    let gji_grace_active =
        gji_active_after_focus && gji_idle_ms < crate::tuning::GJI_SETTLE_GRACE_MS;

    let shadow_grace_active = shadow_on && probe_age_ms < crate::tuning::SHADOW_GRACE_MS;

    FocusProbeGraceFlags {
        warmup_grace_active,
        gji_grace_active,
        shadow_grace_active,
        warmup_elapsed,
        gji_idle_ms,
    }
}

impl Runtime {
    fn apply_effective_ime(&mut self, effective: bool) {
        if effective {
            self.platform_state.ime.reset_detect_state();
        }
        self.platform_state.ime.write_focus_probe(effective);
    }
}

#[allow(clippy::option_if_let_else)]
fn build_ime_on_suffix(
    probe_ime_on: Option<bool>,
    suppressed_reason: Option<&'static str>,
    signals: &FocusProbeGraceFlags,
    probe_age_ms: u64,
    used_shadow_fallback: bool,
) -> String {
    if let Some(reason) = suppressed_reason {
        let detail = match reason {
            "warmup" => format!("warmup:{}ms", signals.warmup_elapsed),
            "gji-io" => format!("gji-io:{}ms", signals.gji_idle_ms),
            _ => format!("shadow:{probe_age_ms}ms"),
        };
        format!("(suppressed:{detail})")
    } else if probe_ime_on.is_none() && used_shadow_fallback {
        "(shadow)".to_string()
    } else if probe_ime_on.is_none() {
        "(stale)".to_string()
    } else {
        String::new()
    }
}

impl Runtime {
    /// read_ime_state_fast_async の結果を self に適用する（with_app 内で呼ぶ）。
    /// kp_stage_focus_probe の旧同期ロジックを async 完了後に実行する版。
    #[allow(clippy::needless_pass_by_value, clippy::option_if_let_else)]
    fn apply_focus_probe(
        &mut self,
        probe: crate::ime::FastImeProbeResult,
        probe_started_ms: u64,
        warmup_ms: u64,
        gji_last_io_ms: u64,
        last_focus_change_ms: u64,
        shadow_on: bool,
    ) {
        let probe_age_ms = hook::current_tick_ms().saturating_sub(probe_started_ms);
        let ime_on_before_probe = self.platform_state.ime.effective_open();

        self.platform_state
            .ime
            .set_is_japanese_ime(probe.is_japanese_ime);

        let now_ms = hook::current_tick_ms();
        let signals = compute_focus_probe_grace(
            now_ms,
            probe_age_ms,
            warmup_ms,
            gji_last_io_ms,
            last_focus_change_ms,
            shadow_on,
        );

        // TsfNative/Imm32Unavailable では probe.ime_on が常に None になる。
        // この場合は shadow の apply 値を代替観測として記録し drift 追跡を維持する。
        let used_shadow_fallback = probe.ime_on.is_none() && probe.is_japanese_ime;

        let suppressed_reason: Option<&'static str> = if let Some(on) = probe.ime_on {
            let effective = on && probe.is_japanese_ime;
            if !effective && signals.any() {
                Some(signals.primary_reason())
            } else {
                self.apply_effective_ime(effective);
                None
            }
        } else {
            // TsfNative/Imm32Unavailable: IMM32 非対応のため probe は常に None を返す。
            // shadow の apply 値を代替観測として focus_probe スロットに記録する。
            if probe.is_japanese_ime {
                self.apply_effective_ime(shadow_on);
            }
            None
        };

        let ime_on_after_probe = self.platform_state.ime.effective_open();
        let input_mode_after_probe = self.platform_state.ime.belief.input_mode();
        let ime_on_suffix = build_ime_on_suffix(
            probe.ime_on,
            suppressed_reason,
            &signals,
            probe_age_ms,
            used_shadow_fallback,
        );

        log::info!(
            "FocusProbe +{}ms: ime_on={}{} mode={:?} [gji_io={}ms sig1={} sig2={} sig3={}]",
            probe_age_ms,
            ime_on_after_probe,
            ime_on_suffix,
            input_mode_after_probe,
            if signals.gji_idle_ms == u64::MAX {
                "never".to_string()
            } else {
                signals.gji_idle_ms.to_string()
            },
            signals.warmup_grace_active,
            signals.gji_grace_active,
            signals.shadow_grace_active,
        );

        match suppressed_reason {
            Some(reason) => log::debug!(
                "FocusProbe: imc_open=false を抑制 (reason={reason}) — Engine deactivation を防止"
            ),
            None if used_shadow_fallback => log::debug!(
                "FocusProbe: TsfNative/Imm32Unavailable — shadow 値 {} を代替観測として記録 \
                 [probe_age={probe_age_ms}ms]",
                shadow_on,
            ),
            None if probe.ime_on.is_none() => log::warn!(
                "FocusProbe: ime_on 未検出 — stale値 {ime_on_before_probe} \
                 [probe_age={probe_age_ms}ms]",
            ),
            None => {}
        }
    }
}
