//! キーイベント処理パイプライン
//!
//! `on_key_event_impl` の処理を段階的に分割したもの。
//! フックコールバック本体から `Runtime::process_key_event` を呼ぶことで
//! 同じ動作をより読みやすい形で表現する。

use crate::hook;
use crate::hook::CallbackResult;
use crate::win32::post_to_main_thread;
use crate::{Runtime, TIMER_IME_REFRESH, WM_EXECUTE_EFFECTS};
use awase::engine::{AssumedReason, InputModeState};
use awase::platform::PlatformRuntime as _;
use awase::types::{KeyEventType, RawKeyEvent, ShadowImeAction};

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
            let is_ctrl_up = matches!(event.event_type, KeyEventType::KeyUp)
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
        self.kp_stage_idle_conv_check(&event);
        let shadow_toggled = self.kp_stage_shadow_ime_toggle(&event);

        let ctx = super::build_input_context(
            self.platform_state.ime.effective_open(),
            self.platform_state.ime.input_mode(),
            self.platform_state.ime.belief.is_japanese_ime(),
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
            && matches!(event.event_type, KeyEventType::KeyDown)
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
        if !matches!(event.event_type, KeyEventType::KeyDown)
            && crate::vk::is_ctrl_variant(event.vk_code)
        {
            let tick_ms = crate::state::TickMs(hook::current_tick_ms());
            self.platform_state.ime.on_ctrl_key_up(event.vk_code, tick_ms);
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
        let obs = crate::state::ObservedState::from_snapshot(crate::tsf::observer::tsf_obs());
        let gji_last_io_ms = obs.gji_last_io_ms;
        let active_ime_kind = obs.active_ime_kind;
        let last_focus_change_ms = self.platform_state.focus.last_focus_change_ms;
        // Imm32Unavailable (Chrome 等) は probe.ime_on が常に None のため、
        // shadow_on がフォールバック値として使われる。
        // applied_open() は前ウィンドウの状態を引き継ぐことがあるため
        // (例: UWP の applied=true が Chrome フォーカス後もリセットされない)、
        // フォーカス変更後にキャッシュリストア済みの desired を反映する
        // effective_open() を使う。
        let shadow_on = self.platform_state.ime.effective_open();
        // エポックを spawn 時にキャプチャ。apply_focus_probe 内でフォーカスが
        // 変わっていないか照合し、stale な観測を棄却する。
        let focus_epoch_at_spawn = self.platform_state.focus.focus_epoch;

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
                    active_ime_kind,
                    focus_epoch_at_spawn,
                );
            });
        });
    }

    /// TsfNative アイドル時の変換モード確認
    ///
    /// TsfNative (WezTerm 等) は通常ポーリングが無効のため、タスクバーから入力モードを
    /// 変更しても `belief.input_mode` が更新されない。
    /// TYPING_IDLE_MS 以上アイドル後の最初の KeyDown でのみ conv を読み、
    /// モード変化を検出したら belief を更新する。
    ///
    /// ## cold start の特別処理
    ///
    /// `output_in_flight_ms() == u64::MAX`（まだ一度も awase が文字を送信していない）の場合、
    /// IMM32 ブリッジが WezTerm 等で ROMAN ビットをローマ字モードでも正しく報告しないことがある。
    /// この状態では NATIVE/ROMAN の組み合わせが曖昧になるため、明確に判定できる
    /// 英数モード（ROMAN=0 かつ NATIVE=0）のみ検出し、それ以外はスキップする。
    ///
    /// awase が一度でも warmup を行い `ImmSetConversionStatus(conv | ROMAN)` を確立した後は
    /// ROMAN ビット変化を「ユーザーによるモード切替」として信頼できる。
    fn kp_stage_idle_conv_check(&mut self, event: &RawKeyEvent) {
        let in_flight = self.platform.output_in_flight_ms();
        let now_tick = crate::state::TickMs(hook::current_tick_ms());
        let explicit_age = self.platform_state.ime.explicit_ime_action_age_ms(now_tick);
        let is_tsf_native = matches!(
            self.platform.current_app_profile(),
            crate::focus::class_names::AppImeProfile::TsfNative
        );
        if !awase::engine::should_run_idle_conv_check(
            matches!(event.event_type, KeyEventType::KeyDown),
            is_tsf_native,
            in_flight,
            explicit_age,
            crate::tuning::TYPING_IDLE_MS,
            crate::tuning::EXPLICIT_IME_SUPPRESS_MS,
        ) {
            // explicit IME 操作直後のスキップのみデバッグログを残す
            // （KeyDown・TsfNative・idle の 3 条件を通過した上で explicit_age だけが残っている場合）
            if matches!(event.event_type, KeyEventType::KeyDown)
                && is_tsf_native
                && in_flight > crate::tuning::TYPING_IDLE_MS
                && explicit_age < crate::tuning::EXPLICIT_IME_SUPPRESS_MS
            {
                log::debug!(
                    "[idle-conv-check] TsfNative: explicit IME action {}ms ago → スキップ (suppress={}ms)",
                    explicit_age,
                    crate::tuning::EXPLICIT_IME_SUPPRESS_MS,
                );
            }
            return;
        }
        // SAFETY: フォアグラウンドウィンドウの IME 変換モードを 10ms タイムアウトで読む。
        let Some(conv) = (unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(10) }) else {
            return;
        };
        // 変換モードを更新: idle-conv-check が conv を読んだタイミングで ConvModeMgr に通知する。
        // warmup の先頭 VK 選択と ImmSetConversionStatus の目標値決定に使われる。
        let conv_mode_changed = self.platform.output.conv_mode.update_from_conv(conv, now_tick);

        // prev_conversion_mode を更新し、次回 input_mode_from_conversion が使えるようにする
        self.platform_state.ime.set_prev_conversion_mode(Some(conv));

        let current = self.platform_state.ime.input_mode();
        let is_cold = in_flight == u64::MAX;
        // kp_stage_idle_conv_check は TsfNative 専用（should_run_idle_conv_check のガード 2）。
        // TsfNative では ROMAN ビットが常に 0 のため is_roman_reliable=false。
        // これにより classify_idle はひらがな conv で ObservedKana への downgrade を行わず、
        // romaji-capable でない場合は AssumedRomaji { ImmBridgeBroken } に回復する。
        let was_romaji_capable = current.is_romaji_capable();
        let new_mode_opt =
            awase::engine::ConvMode::from_u32(conv).classify_idle(is_cold, current, false);

        match new_mode_opt {
            None => {
                log::debug!(
                    "[idle-conv-check] TsfNative: conv=0x{:08X}{} → belief {:?} 変更なし",
                    conv,
                    if is_cold && conv & crate::imm::IME_CMODE_ROMAN != 0 { " cold-start" } else { "" },
                    current,
                );
                // conv 変化なしかつ belief 変化なし → 通常は何もすることがない。
                // ただし カタカナ(NATIVE+KATAKANA)+shadow=OFF の場合は例外:
                //   - AssumedRomaji は romaji_capable のため classify_idle が常に None を返す
                //   - conv 不変が続く限りこの arm に来続けるため、ここが唯一の回復経路
                if !conv_mode_changed {
                    if conv & crate::imm::IME_CMODE_KATAKANA != 0
                        && conv & crate::imm::IME_CMODE_NATIVE != 0
                        && !self.platform_state.ime.effective_open()
                    {
                        self.platform.timer.kill(TIMER_IME_REFRESH);
                        let generation = self.platform_state.ime.event_log.next_seq();
                        self.platform_state
                            .ime
                            .handle_engine_set_open(true, false, generation, now_tick);
                        log::info!(
                            "[idle-conv-check] TsfNative: カタカナ+shadow=OFF → IME ON 同期 \
                             (conv=0x{conv:08X} 不変)"
                        );
                    }
                    return;
                }
            }
            Some(new_mode) => {
                log::info!(
                    "[idle-conv-check] TsfNative: conv=0x{:08X} → belief {:?}→{:?}",
                    conv, current, new_mode,
                );
                self.platform_state.ime.dispatch_event(
                    crate::state::ime_event::ImeEvent::InputModeObserved {
                        mode: new_mode,
                        source: crate::state::ime_event::ObservationSource::ImmGetOpenStatus,
                        at: now_tick,
                    },
                    now_tick,
                );

                // カタカナへの切替 + shadow=OFF → engine ON 同期
                if new_mode == InputModeState::ObservedRomaji
                    && conv & crate::imm::IME_CMODE_KATAKANA != 0
                    && !self.platform_state.ime.effective_open()
                {
                    self.platform.timer.kill(TIMER_IME_REFRESH);
                    let generation = self.platform_state.ime.event_log.next_seq();
                    self.platform_state
                        .ime
                        .handle_engine_set_open(true, false, generation, now_tick);
                    log::info!("[idle-conv-check] TsfNative: カタカナ検出 + shadow=OFF → IME ON 同期");
                }

                // belief が romaji 不可→可 に変化 かつ shadow=ON → engine を再起動する。
                // (例: HanAlpha→Hiragana で ObservedKana → AssumedRomaji に回復したとき)
                if !was_romaji_capable
                    && new_mode.is_romaji_capable()
                    && self.platform_state.ime.effective_open()
                {
                    self.platform.timer.kill(TIMER_IME_REFRESH);
                    let generation = self.platform_state.ime.event_log.next_seq();
                    self.platform_state
                        .ime
                        .handle_engine_set_open(true, false, generation, now_tick);
                    log::info!(
                        "[idle-conv-check] TsfNative: belief romaji 回復 + shadow=ON → engine 再起動"
                    );
                }

                // ObservedEisu 検出 → 自動 DirectInput 切替
                // 半角英数（IME-ON の 0x10 モード）は使用しない設計。shadow ON/OFF を問わず
                // VK_IME_OFF / VK_KANJI で DirectInput へ落とす。
                // conv_mode=0x10 の観測は IME-ON の確証なので effective_open=true を直接注入する。
                if new_mode == InputModeState::ObservedEisu {
                    log::info!(
                        "[idle-conv-check] TsfNative: ObservedEisu 検出 → DirectInput"
                    );
                    self.platform.timer.kill(TIMER_IME_REFRESH);
                    let generation = self.platform_state.ime.event_log.next_seq();
                    self.platform_state
                        .ime
                        .handle_engine_set_open(false, false, generation, now_tick);
                    // conv=0x10 (ROMAN bit) が観測済み → IME-ON 確定。direct belief で already_matched をバイパス。
                    let belief = crate::output::OpenBelief { effective_open: true, confident: true };
                    let outcome = self.platform.apply_ime_open_with_belief(false, None, belief);
                    self.on_ime_apply_complete(false, outcome);
                }
            }
        }

        // ひらがな/カタカナ(NATIVE=1)+shadow=OFF → conv 変化時に engine ON 同期。
        // classify_idle が None を返す場合（belief 変化なし）も含む。
        // カタカナ+conv 不変は上記 None arm で処理済み。
        if conv_mode_changed
            && conv & crate::imm::IME_CMODE_NATIVE != 0
            && !self.platform_state.ime.effective_open()
        {
            self.platform.timer.kill(TIMER_IME_REFRESH);
            let generation = self.platform_state.ime.event_log.next_seq();
            self.platform_state
                .ime
                .handle_engine_set_open(true, false, generation, now_tick);
            log::info!(
                "[idle-conv-check] TsfNative: {}切替検出 + shadow=OFF → IME ON 同期",
                if conv & crate::imm::IME_CMODE_KATAKANA != 0 { "カタカナ" } else { "ひらがな" }
            );
        }
    }

    /// Shadow IME トグル処理
    ///
    /// IME ON/OFF が変化したら `true` を返す。`kp_stage_execute` がこの値を見て
    /// Imm32Unavailable アプリで物理 IME キーを抑止すべきか判定する。
    fn kp_stage_shadow_ime_toggle(&mut self, event: &RawKeyEvent) -> bool {
        if !matches!(event.event_type, KeyEventType::KeyDown) {
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
        let tick_ms = crate::state::TickMs(hook::current_tick_ms());
        match kind {
            IntentKind::SyncKey => self.platform_state.ime.write_sync_key(new_val, tick_ms),
            IntentKind::PhysicalImeKey => {
                self.platform_state.ime.write_physical_key(new_val, tick_ms);
            }
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
                self.platform_state.ime.mirror_applied_open(false, tick_ms);
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
            let tick_ms = crate::state::TickMs(hook::current_tick_ms());
            let applied = self.platform_state.ime.handle_engine_set_open(
                new_ime_on,
                event.modifier_snapshot.ctrl,
                generation,
                tick_ms,
            );
            log::debug!(
                "IME control: preconditions.ime_on = {new_ime_on} (SetOpenRequest), poll suspended{}",
                if applied { "" } else { " [chord barrier active → skipped]" }
            );

            // SetOpen(true) 後 input_mode=ObservedEisu が残ると engine が NotRomajiInput で
            // inactive になり、VK_KANJI 送信後も 1500ms 間 NICOLA が処理されない。
            // VK_KANJI 送信により GJI はひらがなへ遷移するため ObservedEisu は stale。
            // AssumedRomaji にリセットして engine を即座に活性化する。
            // (1500ms 後の idle-conv-check で GJI 実状態を再確認・訂正する)
            if applied && new_ime_on
                && self.platform_state.ime.input_mode() == InputModeState::ObservedEisu
            {
                self.platform_state.ime.dispatch_event(
                    crate::state::ime_event::ImeEvent::InputModeObserved {
                        mode: InputModeState::AssumedRomaji {
                            reason: AssumedReason::AppKindExcluded,
                        },
                        source: crate::state::ime_event::ObservationSource::ImmGetOpenStatus,
                        at: tick_ms,
                    },
                    tick_ms,
                );
                log::info!(
                    "[post-decision] SetOpen(true) + ObservedEisu → AssumedRomaji にリセット \
                     (engine 即活性化)"
                );
            }
        }

        if !decision.is_consumed()
            && event.ime_relevance.may_change_ime
            && matches!(event.event_type, KeyEventType::KeyDown)
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
            && matches!(event.event_type, KeyEventType::KeyDown)
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
        // H-3-e: executor が set_conv_mode_authority() で格納した保留値を ImeStateHub に dispatch。
        self.sync_conv_mode_authority();

        if result.has_pending {
            post_to_main_thread(WM_EXECUTE_EFFECTS);
        }

        result.callback
    }
}

/// フォーカスプローブの IME 更新抑制シグナルをまとめた値
///
/// shadow_grace は probe_admission の FocusEpoch 照合に置き換え済みのため
/// このフラグには含まれない。
struct FocusProbeGraceFlags {
    warmup_grace_active: bool,
    gji_grace_active: bool,
    warmup_elapsed: u64,
    gji_idle_ms: u64,
}

impl FocusProbeGraceFlags {
    const fn any(&self) -> bool {
        self.warmup_grace_active || self.gji_grace_active
    }

    const fn primary_reason(&self) -> &'static str {
        if self.warmup_grace_active {
            "warmup"
        } else {
            "gji-io"
        }
    }
}

const fn compute_focus_probe_grace(
    now_ms: u64,
    warmup_ms: u64,
    gji_last_io_ms: u64,
    last_focus_change_ms: u64,
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

    FocusProbeGraceFlags {
        warmup_grace_active,
        gji_grace_active,
        warmup_elapsed,
        gji_idle_ms,
    }
}

impl Runtime {
    fn apply_effective_ime(&mut self, effective: bool, tick_ms: crate::state::TickMs) {
        if effective {
            self.platform_state.ime.reset_detect_state();
        }
        self.platform_state.ime.write_focus_probe(effective, tick_ms);
    }
}

#[expect(clippy::option_if_let_else)]
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
    #[expect(clippy::needless_pass_by_value, clippy::option_if_let_else)]
    fn apply_focus_probe(
        &mut self,
        probe: crate::ime::FastImeProbeResult,
        probe_started_ms: u64,
        warmup_ms: u64,
        gji_last_io_ms: u64,
        last_focus_change_ms: u64,
        shadow_on: bool,
        active_ime_kind: crate::tsf::observer::ActiveImeKind,
        focus_epoch_at_spawn: u64,
    ) {
        // エポック照合: spawn 後にフォーカスが変わっていれば probe 全体を棄却する。
        // 仮想デスクトップ切替中の経由ウィンドウが stale な FocusProbe を送り込む問題を防ぐ。
        let current_epoch = self.platform_state.focus.focus_epoch;
        if current_epoch != focus_epoch_at_spawn {
            log::debug!(
                "[FocusProbe] epoch rejected: {focus_epoch_at_spawn} → {current_epoch} \
                 (focus changed since probe spawn)"
            );
            return;
        }

        let now_tick_ms = crate::state::TickMs(hook::current_tick_ms());
        let probe_age_ms = now_tick_ms.saturating_sub(probe_started_ms);
        let ime_on_before_probe = self.platform_state.ime.effective_open();

        let now_ms = now_tick_ms.0;
        let signals = compute_focus_probe_grace(
            now_ms,
            warmup_ms,
            gji_last_io_ms,
            last_focus_change_ms,
        );

        // スリープ復帰後など grace 期間中は read_ime_state_fast が一時的に
        // is_japanese_ime=false を返すことがある。
        // false へのダウングレードは grace active 中は行わない（true はいつでも更新）。
        if probe.is_japanese_ime || !signals.any() {
            self.platform_state
                .ime
                .set_is_japanese_ime(probe.is_japanese_ime);
        }

        // TsfNative/Imm32Unavailable では probe.ime_on が常に None になる。
        // この場合は shadow の apply 値を代替観測として記録し drift 追跡を維持する。
        let used_shadow_fallback = probe.ime_on.is_none() && probe.is_japanese_ime;

        let suppressed_reason: Option<&'static str> = if let Some(on) = probe.ime_on {
            let effective = on && probe.is_japanese_ime;
            if !effective && signals.any() {
                Some(signals.primary_reason())
            } else {
                self.apply_effective_ime(effective, now_tick_ms);
                None
            }
        } else {
            // TsfNative/Imm32Unavailable: IMM32 非対応のため probe は常に None を返す。
            // shadow の apply 値を代替観測として focus_probe スロットに記録する。
            if probe.is_japanese_ime {
                self.apply_effective_ime(shadow_on, now_tick_ms);
            }
            None
        };

        // TsfNative フォーカス復帰時: HwndCache が最大1時間有効なため、ユーザーが
        // JISかな⇔ローマ字を切り替えていると cached input_mode が stale になる。
        // idle-conv-check は TYPING_IDLE_MS(500ms) ゲートがあり打ち始めのキーで発火しない。
        // ここで即時 conv mode を読んで belief を補正する（FocusProbe ログに反映される）。
        if matches!(
            self.platform.current_app_profile(),
            crate::focus::class_names::AppImeProfile::TsfNative
        ) && probe.is_japanese_ime
        {
            let in_flight = self.platform.output_in_flight_ms();
            // cold start: ROMAN ビットが信頼できないためスキップ
            if in_flight != u64::MAX {
                // SAFETY: メッセージループスレッドから呼ぶ。10ms タイムアウト。
                if let Some(conv) =
                    unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(10) }
                {
                    self.platform.output.conv_mode.update_from_conv(conv, now_tick_ms);
                    self.platform_state.ime.set_prev_conversion_mode(Some(conv));
                    let has_roman = conv & crate::imm::IME_CMODE_ROMAN != 0;
                    let has_native = conv & crate::imm::IME_CMODE_NATIVE != 0;
                    let has_kata = conv & crate::imm::IME_CMODE_KATAKANA != 0;
                    use awase::engine::InputModeState;
                    let current = self.platform_state.ime.input_mode();
                    use crate::state::ime_event::{ImeEvent, ObservationSource};
                    if awase::engine::ConvMode::from_u32(conv).is_eisu() {
                        // 英数モード (HankakuAlpha / ZenkakuAlpha)。ROMAN ビットの有無は問わない。
                        if !matches!(current, InputModeState::ObservedEisu) {
                            log::info!(
                                "[focus-conv-check] TsfNative: conv=0x{:08X} (英数) \
                                 → belief {:?}→ObservedEisu",
                                conv,
                                current,
                            );
                            self.platform_state.ime.dispatch_event(
                                ImeEvent::InputModeObserved {
                                    mode: InputModeState::ObservedEisu,
                                    source: ObservationSource::FocusProbe,
                                    at: now_tick_ms,
                                },
                                now_tick_ms,
                            );
                        }
                    } else if has_native && !has_roman && !has_kata {
                        // JISかな / ひらがな: classify_idle(is_roman_reliable=false) に委ねる。
                        // TsfNative では ROMAN=0 が信頼できないため ObservedKana への downgrade は
                        // 行わず、非 romaji-capable なら AssumedRomaji に回復する。
                        use awase::engine::ConvMode;
                        match ConvMode::from_u32(conv).classify_idle(false, current, false) {
                            Some(new_mode) => {
                                log::info!(
                                    "[focus-conv-check] TsfNative: conv=0x{:08X} (JISかな) \
                                     → belief {:?}→{:?}",
                                    conv, current, new_mode,
                                );
                                self.platform_state.ime.dispatch_event(
                                    ImeEvent::InputModeObserved {
                                        mode: new_mode,
                                        source: ObservationSource::FocusProbe,
                                        at: now_tick_ms,
                                    },
                                    now_tick_ms,
                                );
                            }
                            None => {
                                log::debug!(
                                    "[focus-conv-check] TsfNative: conv=0x{:08X} (JISかな) \
                                     → belief {:?} 変更なし",
                                    conv, current,
                                );
                            }
                        }
                    } else if has_roman && !current.is_romaji_capable() {
                        // ローマ字モードだが belief が kana 系: 訂正。
                        log::info!(
                            "[focus-conv-check] TsfNative: conv=0x{:08X} (ローマ字) \
                             → belief {:?}→ObservedRomaji",
                            conv,
                            current,
                        );
                        self.platform_state.ime.dispatch_event(
                            ImeEvent::InputModeObserved {
                                mode: InputModeState::ObservedRomaji,
                                source: ObservationSource::FocusProbe,
                                at: now_tick_ms,
                            },
                            now_tick_ms,
                        );
                    }
                    // カタカナモードは NICOLA shadow-sync が複雑なため idle-conv-check に委ねる。
                }
            }
        }

        // ImmCross アプリ（Qt/LINE 等）: FocusProbe は top-level hwnd の IMC を読むが、
        // GJI 使用時は child hwnd と IME 状態が異なる場合がある（Qt の IME コンテキスト分割）。
        // read_ime_state_full_async で child hwnd を正確に読み、High confidence 観測として記録する。
        // これにより FocusProbe (Low) が誤って false を返しても derive_open() で正しく上書きされる。
        //
        // エポック照合: spawn 後にフォーカスが変わった場合は棄却する。apply_focus_probe 冒頭の
        // epoch チェックを通過しているため focus_epoch_at_spawn は現在のエポックと等しいことが保証済み。
        if matches!(
            self.platform.current_app_profile(),
            crate::focus::classify::AppImeProfile::Standard,
        ) && probe.is_japanese_ime {
            let ticket = crate::state::probe_admission::ImmLikeTicket {
                focus_epoch: focus_epoch_at_spawn,
            };
            win32_async::spawn_local(async move {
                // SAFETY: read_ime_state_full_async は offload 済み — メインスレッド不要。
                let snap = crate::ime::read_ime_state_full_async().await;
                if let Some(open) = snap.ime_on {
                    let _ = crate::with_app(|app| {
                        let current_epoch = app.platform_state.focus.focus_epoch;
                        if let crate::state::probe_admission::Admission::Reject(reason) =
                            ticket.admit(current_epoch)
                        {
                            log::debug!(
                                "[ImmCrossProbe] epoch rejected: {reason} \
                                 (focus changed since probe spawn)"
                            );
                            return;
                        }
                        let tick_ms = crate::state::TickMs(hook::current_tick_ms());
                        // ON/OFF: High confidence (ImmCrossProbe source)
                        app.platform_state.ime.write_imm_cross_probe(open, tick_ms);
                        log::debug!(
                            "[ImmCrossProbe] child-hwnd IME={open} → High confidence 観測記録"
                        );
                        // input_mode: Observe → pure decision → belief
                        // classify_fetched_snapshot = classify_ime_snapshot の同期 wrapper。
                        // ObservedEisu stale 回復を含む全 input_mode 判定をここに集約する。
                        let update = crate::observer::ime_observer::classify_fetched_snapshot(
                            &snap,
                            tick_ms.0,
                            app.platform_state.ime.effective_open(),
                            app.platform_state.ime.is_force_on_guard_active(),
                            app.platform_state.ime.input_mode(),
                            app.platform_state.ime.belief.prev_conversion_mode(),
                        );
                        if let Some(mode) = update.new_input_mode {
                            use crate::state::ime_event::{ImeEvent, ObservationSource};
                            app.platform_state.ime.dispatch_event(
                                ImeEvent::InputModeObserved {
                                    mode,
                                    source: ObservationSource::ImmCrossProbe,
                                    at: tick_ms,
                                },
                                tick_ms,
                            );
                        }
                    });
                }

                // MS-IME + ImmCross (LINE 等): かなモード (conv=0x09) で IME ON すると
                // JIS かな直接入力になる。ImmCrossProcessStrategy は romaji 修正を
                // 先行実行するが、async probe 完了時点で stale な conv を読む場合に備えて
                // ここでも ROMAN ビットを補完する（二重補正は冪等なので無害）。
                // ObservedKana はユーザーが意図的にかな入力に設定した状態なので上書きしない。
                if snap.ime_on == Some(true) {
                    if let Some(conv) = snap.conversion_mode {
                        let mode = awase::engine::ConvMode::from_u32(conv);
                        if !mode.is_eisu() && !mode.romaji {
                            let should_restore = crate::with_app(|app| {
                                app.platform_state.ime.effective_open()
                                    && !matches!(
                                        app.platform_state.ime.input_mode(),
                                        InputModeState::ObservedKana
                                    )
                            })
                            .unwrap_or(false);
                            if should_restore {
                                log::debug!(
                                    "[ImmCrossProbe] kana mode (conv=0x{conv:08X}) + IME ON \
                                     → romaji 修正 (MS-IME かなモード修正)"
                                );
                                let _ = crate::ime::set_ime_romaji_mode_async().await;
                            }
                        }
                    }
                }
            });
        }

        let ime_on_after_probe = self.platform_state.ime.effective_open();
        let input_mode_after_probe = self.platform_state.ime.input_mode();
        let ime_on_suffix = build_ime_on_suffix(
            probe.ime_on,
            suppressed_reason,
            &signals,
            probe_age_ms,
            used_shadow_fallback,
        );

        let gji_fields = if active_ime_kind == crate::tsf::observer::ActiveImeKind::GoogleJapaneseInput {
            format!(
                " gji_io={}ms sig2={}",
                if signals.gji_idle_ms == u64::MAX { "never".to_string() } else { signals.gji_idle_ms.to_string() },
                signals.gji_grace_active,
            )
        } else {
            String::new()
        };
        log::info!(
            "FocusProbe +{}ms: ime_on={}{} mode={:?} [ime={:?} sig1={}{}]",
            probe_age_ms,
            ime_on_after_probe,
            ime_on_suffix,
            input_mode_after_probe,
            active_ime_kind,
            signals.warmup_grace_active,
            gji_fields,
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
