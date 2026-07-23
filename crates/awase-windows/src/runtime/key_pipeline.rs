//! キーイベント処理パイプライン
//!
//! `on_key_event_impl` の処理を段階的に分割したもの。
//! フックコールバック本体から `Runtime::process_key_event` を呼ぶことで
//! 同じ動作をより読みやすい形で表現する。

use crate::hook;
use crate::hook::CallbackResult;
use crate::win32::post_to_main_thread;
use crate::{Runtime, TIMER_IME_REFRESH, WM_EXECUTE_EFFECTS};
use awase::engine::InputModeState;
use awase::platform::TsfComposition as _;
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

        // kp_stage_focus_probe が FocusTransition barrier を consume する前に
        // settle 状態をスナップショットしておく（post_decision で使う。
        // 消費後に読むと常に false になり判断できないため）。
        let focus_transition_was_pending = self
            .platform_state
            .ime
            .is_focus_transition_settling(std::time::Instant::now());

        self.kp_stage_focus_probe(&mut event);
        self.kp_stage_idle_conv_check(&event);
        let shadow_toggled = self.kp_stage_shadow_ime_toggle(&event);

        let ctx = super::build_input_context(
            self.platform_state.ime.effective_open(),
            self.platform_state.ime.input_mode(),
            self.platform_state.ime.belief.is_japanese_ime(),
            crate::tsf::observer::ime_composition_active_now(),
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
        let mut decision = self.engine.on_input(event, &ctx);
        // キーボード経路の一次フィルタ（decision 除去の単一実装は executor 側ヘルパ）。
        // フォーカス遷移直後（settle 期間内）に Engine が発行した SetOpen effect は、
        // 実行(kp_stage_execute → 実際の SendInput)まで到達する前にここで取り除く。
        // handle_engine_set_open 側のフィルタは belief の書き込み(desired_open等)を防ぐ
        // 最終防衛線で意図が異なり、decision.effects に残った SetOpen は kp_stage_execute
        // 経由で無条件に実行されてしまうため、effect 自体を落とすこの一次フィルタが必須
        // （2026-07-05: 前回の修正が効かなかった原因）。
        // この経路は kp_stage_focus_probe が barrier を consume 済みのため、live 評価ではなく
        // イベント開始時にスナップショットした focus_transition_was_pending を settle 判定に使う。
        let stripped_set_open = crate::runtime::executor::strip_ime_set_open_if_settling(
            &mut decision,
            focus_transition_was_pending,
        );
        if stripped_set_open.is_some() {
            // settle 中に握りつぶした SetOpen は自然には再発行されない
            // （Engine::prev_activation は遷移確定済みのため）。既存の
            // apply_force_on_for_imm_broken 等と同じ「settle 明けに refresh で再試行」
            // パターンで確実に一度だけ再同期する
            // （2026-07-08: GjiFsm が resync できず「このせっけい」の文字欠落に至った実機ログから判明）。
            let retry_ms = self.platform_state.ime.focus_settle_ms() + 50;
            log::debug!(
                "[focus-settle] SetOpen stripped from kp_run_inner decision → \
                 {retry_ms}ms 後に refresh で再試行"
            );
            self.schedule_ime_refresh(retry_ms);
        }
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

        self.kp_stage_post_decision(&decision, &event, focus_transition_was_pending);

        // Ctrl 系 KeyUp で chord barrier を解除する。
        // chord 状態の判断は ImeStateHub.on_ctrl_key_up() に集約（パイプラインは VK 分類のみ担う）。
        if !matches!(event.event_type, KeyEventType::KeyDown)
            && crate::vk::is_ctrl_variant(event.vk_code)
        {
            let tick_ms = crate::state::TickMs(hook::current_tick_ms());
            self.platform_state
                .ime
                .on_ctrl_key_up(event.vk_code, tick_ms);
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
        // spawn 時にチケットをキャプチャ。apply_focus_probe 完了時に epoch 照合し stale な観測を棄却する。
        let ticket = crate::state::probe_admission::ImmLikeTicket {
            focus_epoch: self.platform_state.focus.focus_epoch,
        };

        win32_async::spawn_local(async move {
            let probe = crate::ime::read_ime_state_fast_async().await;
            let _ = crate::with_app(|app| {
                let current_epoch = app.platform_state.focus.focus_epoch;
                let crate::state::probe_admission::Admission::Accept(accepted) =
                    ticket.admit(current_epoch)
                else {
                    log::debug!("[FocusProbe] epoch rejected (focus changed since probe spawn)");
                    return;
                };
                app.apply_focus_probe(
                    probe,
                    probe_started_ms,
                    warmup_ms,
                    gji_last_io_ms,
                    last_focus_change_ms,
                    shadow_on,
                    active_ime_kind,
                    accepted,
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
        // Shift conv 安全網のブリップ中、または左Shift単独タップによる半角英数
        // 持続トグル中（`kp_stage_shift_conv_guard`）は凍結する。conv=0x00000000 は
        // awase 自身が意図的に設定した状態であり、ObservedEisu → DirectInput
        // （IME OFF 落ち）に反応させてはならない。Shift 解放時の復元が
        // explicit IME action として抑止を引き継ぐ。
        if self.platform_state.gate.shift_conv_guard_pending
            || self.platform_state.gate.half_width_alnum_toggle_active
        {
            return;
        }
        let output_idle_ms_at_spawn = self.platform.output_in_flight_ms();
        let now_tick_at_spawn = crate::state::TickMs(hook::current_tick_ms());
        let explicit_age = self
            .platform_state
            .ime
            .explicit_ime_action_age_ms(now_tick_at_spawn);
        let is_tsf_native = crate::focus::class_names::is_effectively_tsf_native(
            self.platform.current_app_profile(),
            self.platform.focus.class_name(),
        );
        if !awase::engine::should_run_idle_conv_check(
            matches!(event.event_type, KeyEventType::KeyDown),
            is_tsf_native,
            output_idle_ms_at_spawn,
            explicit_age,
            crate::tuning::TYPING_IDLE_MS,
            crate::tuning::EXPLICIT_IME_SUPPRESS_MS,
        ) {
            // explicit IME 操作直後のスキップのみデバッグログを残す
            // （KeyDown・TsfNative・idle の 3 条件を通過した上で explicit_age だけが残っている場合）
            if matches!(event.event_type, KeyEventType::KeyDown)
                && is_tsf_native
                && output_idle_ms_at_spawn > crate::tuning::TYPING_IDLE_MS
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

        // BUG-34（docs/known-bugs.md）: get_ime_conversion_mode_raw_timeout は
        // SendMessageTimeoutW(SMTO_ABORTIFHUNG) ベースで、ハング判定が確定するまで
        // 実質 timeout_ms を無視して ~5s ブロックしうる。エンジンスレッド上で同期に
        // 呼ぶとメッセージループが詰まり打鍵が消える。offload してワーカースレッドで
        // 実行する。
        //
        // 多重 in-flight 防止: GJI が本当にハングしている間、断続的なタイピングで
        // offload 呼び出しが積み上がるのを防ぐ（1 件 in-flight の間は新規 spawn しない）。
        if self.platform_state.gate.idle_conv_check_in_flight {
            log::debug!("[idle-conv-check] 前回の conv 読み取りが in-flight のためスキップ");
            return;
        }
        self.platform_state.gate.idle_conv_check_in_flight = true;

        // spawn 時にチケットをキャプチャ。apply_idle_conv_check 完了時に epoch 照合し
        // フォーカスが変わっていれば stale な観測を棄却する（kp_stage_focus_probe と同型）。
        let ticket = crate::state::probe_admission::ImmLikeTicket {
            focus_epoch: self.platform_state.focus.focus_epoch,
        };
        win32_async::spawn_local(async move {
            let conv = crate::ime::get_ime_conversion_mode_raw_timeout_async(10).await;
            let _ = crate::with_app(|app| {
                app.platform_state.gate.idle_conv_check_in_flight = false;
                let Some(conv) = conv else { return };
                let current_epoch = app.platform_state.focus.focus_epoch;
                let crate::state::probe_admission::Admission::Accept(_) =
                    ticket.admit(current_epoch)
                else {
                    log::debug!(
                        "[idle-conv-check] epoch rejected (focus changed since read spawn)"
                    );
                    return;
                };
                app.apply_idle_conv_check(conv, output_idle_ms_at_spawn, now_tick_at_spawn);
            });
        });
    }

    /// `get_ime_conversion_mode_raw_timeout_async` の結果を self に適用する
    /// （`with_app` 内で呼ぶ）。`kp_stage_idle_conv_check` の旧同期ロジックを
    /// async 完了後に実行する版。
    ///
    /// 読み取りが in-flight の間（旧同期コードでは起こり得なかった隙間）に、
    /// awase 自身が (a) shift ガードを立てる、(b) explicit IME 操作を記録する、
    /// (c) warmup 等で実際に出力を送る、のいずれかを行っていたら、読み取った
    /// `conv` は awase 自身の遷移途中を拾った汚染値の可能性がある。spawn 時の
    /// スナップショット（`output_idle_ms_at_spawn` / `now_tick_at_spawn`）と
    /// apply 時点を突き合わせて、これらが起きていないことを再確認してから適用する。
    fn apply_idle_conv_check(
        &mut self,
        conv: u32,
        output_idle_ms_at_spawn: u64,
        now_tick_at_spawn: crate::state::TickMs,
    ) {
        // JISかな化復元（Apply(3)）のレート制限。この関数末尾の restore_roman 分岐でのみ使う。
        const ROMAN_RESTORE_MIN_INTERVAL_MS: u64 = 3_000;

        // (a) shift ガード再検証: spawn 後に kp_stage_shift_conv_guard が立てた可能性がある。
        if self.platform_state.gate.shift_conv_guard_pending
            || self.platform_state.gate.half_width_alnum_toggle_active
        {
            log::debug!(
                "[idle-conv-check] apply 時に shift ガードが有効 → 読み取り結果 conv=0x{conv:08X} を破棄"
            );
            return;
        }

        let now_tick = crate::state::TickMs(hook::current_tick_ms());

        // (b) explicit IME 操作の再検証: spawn 後に Ctrl+変換/無変換 等が発生した場合。
        let explicit_age = self.platform_state.ime.explicit_ime_action_age_ms(now_tick);
        if explicit_age < crate::tuning::EXPLICIT_IME_SUPPRESS_MS {
            log::debug!(
                "[idle-conv-check] apply 時に explicit IME action {explicit_age}ms 前 → \
                 読み取り結果 conv=0x{conv:08X} を破棄 (suppress={}ms)",
                crate::tuning::EXPLICIT_IME_SUPPRESS_MS,
            );
            return;
        }

        // (c) 自己出力の再検証: spawn〜apply の間に awase 自身が SendInput/warmup で
        // 出力していれば、conv は遷移途中を拾った可能性が高い。`output_in_flight_ms()`
        // （最終送信からの経過 ms）を spawn 時と apply 時で絶対時刻に換算して突き合わせ、
        // 最終送信時刻そのものが変わっていないかを確認する（経過 ms の単純比較では
        // 待機時間の分だけ必ず増えるため使えない）。
        let output_idle_ms_now = self.platform.output_in_flight_ms();
        let last_send_abs_ms_at_spawn = (output_idle_ms_at_spawn != u64::MAX)
            .then(|| now_tick_at_spawn.0.saturating_sub(output_idle_ms_at_spawn));
        let last_send_abs_ms_now =
            (output_idle_ms_now != u64::MAX).then(|| now_tick.0.saturating_sub(output_idle_ms_now));
        if last_send_abs_ms_at_spawn != last_send_abs_ms_now {
            log::debug!(
                "[idle-conv-check] apply 時に自己出力を検出 (last_send {last_send_abs_ms_at_spawn:?}→{last_send_abs_ms_now:?}) → \
                 読み取り結果 conv=0x{conv:08X} を破棄 (mid-warmup 汚染の可能性)"
            );
            return;
        }
        let in_flight = output_idle_ms_now;

        // 変換モードを更新: idle-conv-check が conv を読んだタイミングで ConvModeMgr に通知する。
        // warmup の先頭 VK 選択と ImmSetConversionStatus の目標値決定に使われる。
        let conv_mode_changed = self
            .platform
            .output
            .conv_mode
            .update_from_conv(conv, now_tick);

        // prev_conversion_mode を更新し、次回 input_mode_from_conversion が使えるようにする
        self.platform_state.ime.set_prev_conversion_mode(Some(conv));

        let current = self.platform_state.ime.input_mode();
        let is_cold = in_flight == u64::MAX;
        // Pure: conv ビットの解釈と engine 同期判断を conv_classify に委譲する。
        // kp_stage_idle_conv_check は TsfNative 専用（should_run_idle_conv_check のガード 2）で
        // ROMAN ビットが常に 0 のため is_roman_reliable=false。これにより classify_idle は
        // ひらがな conv で ObservedKana への downgrade を行わず、romaji-capable でない場合は
        // AssumedRomaji { ImmBridgeBroken } に回復する。
        //
        // `conv`（この tick で読んだ生値）ではなく `ConvModeMgr::get()`（直前の
        // `update_from_conv` 済みのデバウンス確定値）を渡す。BUG-19: `conv` を直接
        // 渡すと、`GetForegroundWindow` 基準の読み取りが候補ウィンドウ等から一発だけ
        // 誤ったカタカナ conv を拾った際、warmup 側（ConvModeMgr 消費）は保護されても
        // こちら（belief 更新・engine 同期）は無防備なままになる。
        let effective_open = self.platform_state.ime.effective_open();
        let cm = self
            .platform
            .output
            .conv_mode
            .get()
            .unwrap_or_else(|| awase::engine::ConvMode::from_u32(conv));
        let transition = crate::state::conv_classify::classify_conv_transition(
            cm,
            current,
            is_cold,
            effective_open,
            conv_mode_changed,
            false,
        );
        // P1: リプレイ回帰基盤用に呼び出し全体を構造化記録する。実機でこの周辺の
        // バグに気づいたらダンプし、tests/journals/ のフィクスチャへ転記する
        // （docs/journal-replay-guide.md 参照）。
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::ConvClassifyCall {
                conv,
                current,
                is_cold,
                effective_open,
                conv_mode_changed,
                is_roman_reliable: false,
                result: transition,
            });

        // Apply(1): input_mode belief の更新を dispatch する。
        // ここが key_pipeline 内で唯一の idle-conv-check InputModeObserved 構築点。
        match transition.input_mode_update {
            None => {
                log::debug!(
                    "[idle-conv-check] TsfNative: conv=0x{:08X}{} → belief {:?} 変更なし",
                    conv,
                    if is_cold && conv & crate::imm::IME_CMODE_ROMAN != 0 {
                        " cold-start"
                    } else {
                        ""
                    },
                    current,
                );
            }
            Some(new_mode) => {
                log::info!(
                    "[idle-conv-check] TsfNative: conv=0x{conv:08X} → belief {current:?}→{new_mode:?}"
                );
                // source=ConvBitsInference: 実態は conv ビット（ImmGetConversionStatus 由来）
                // からの input_mode 推定であり、ImmGetOpenStatus API 観測ではない。conv の
                // 読み取り自体は直接 API 成功なので confidence は High（sibling の focus-conv-check
                // FocusProbe/High・ImmCrossProbe/High と揃える）。source だけを正直に分離する。
                self.platform_state.ime.dispatch_event(
                    crate::state::ime_event::ImeEvent::InputModeObserved {
                        mode: new_mode,
                        source: crate::state::ime_event::ObservationSource::ConvBitsInference,
                        confidence: crate::state::ime_event::ObservationConfidence::High,
                        at: now_tick,
                    },
                    now_tick,
                );
            }
        }

        // Apply(2): engine 同期を 1 経路で dispatch する（従来 5 箇所の
        // handle_engine_set_open をここに集約）。
        self.kp_apply_conv_engine_sync(transition.engine, conv, now_tick);

        // Apply(3): JISかな化（ひらがな conv で ROMAN 無し）→ ローマ字入力の復元（BUG-08）。
        // 外部注入 VK_KANA 等でかなロックがトグルされると GJI/MS-IME がかな入力に
        // 反転し、engine の romaji VK 出力が壊滅する。awase が conv を所有する
        // ウィンドウ（conv_mutation_allowed）でのみ、非同期・冪等に ROMAN を立て直す。
        //
        // レート制限: classify は steady-state の JISかな conv でも復元を要求する
        // （変化検出は別経路が先に消費するため頼れない）ので、送信間隔をここで抑える。
        // 復元が成功すれば次回 conv=ROMAN 付きになり要求自体が止まる。復元が効かない
        // 環境でも最悪この間隔で IMC_SETCONVERSIONMODE を打つだけ（冪等・非同期）。
        if transition.restore_roman
            && self.platform.output.conv_mutation_allowed.get()
            && now_tick.saturating_sub(self.platform.output.last_roman_restore_ms.get())
                >= ROMAN_RESTORE_MIN_INTERVAL_MS
        {
            self.platform.output.last_roman_restore_ms.set(now_tick.0);
            log::info!(
                "[idle-conv-check] JISかな化を検出 (conv=0x{conv:08X}, ROMAN 喪失) → \
                 ローマ字入力を復元"
            );
            win32_async::spawn_local(async {
                let ok = crate::ime::set_ime_romaji_mode_with_target_async(None).await;
                if !ok {
                    log::warn!("[idle-conv-check] ローマ字入力復元に失敗 (IMC_SETCONVERSIONMODE)");
                }
            });
        }
    }

    /// idle-conv-check の engine 同期を単一経路で適用する。
    ///
    /// 従来 `kp_stage_idle_conv_check` の 5 箇所に散っていた `handle_engine_set_open`
    /// 呼び出しを、純関数 `classify_conv_transition` が返す `EngineSync` に基づく
    /// 1 箇所の dispatch に集約する。
    fn kp_apply_conv_engine_sync(
        &mut self,
        engine: crate::state::conv_classify::EngineSync,
        conv: u32,
        now_tick: crate::state::TickMs,
    ) {
        use crate::state::conv_classify::EngineSync;
        let target = match engine {
            EngineSync::None => return,
            EngineSync::ReportOpenInference(reason) => {
                // KatakanaShadowOff/NativeToggleShadowOff: engine を actuate せず
                // ObserverReported として記録するだけにとどめる。desired_open は
                // 変更されないため、実際に補正が必要かは既存の drift correction
                // 経路（check_drift_correction、BUG-20 で OFF 方向も修正済み）に
                // 委ねる（2026-07-08 BUG-19 再発対策）。
                log::info!(
                    "[idle-conv-check] TsfNative: conv observation open=true reason={reason:?} \
                     (conv=0x{conv:08X}) → ObserverReported として記録 (engine は actuate しない)"
                );
                self.platform_state
                    .ime
                    .report_conv_open_inference(true, reason, now_tick);
                return;
            }
            EngineSync::SetOpen(reason) => {
                log::info!(
                    "[idle-conv-check] TsfNative: engine ON 同期 (conv=0x{conv:08X}, reason={reason:?})"
                );
                true
            }
            EngineSync::DirectInput => {
                log::info!("[idle-conv-check] TsfNative: ObservedEisu 検出 → DirectInput (conv=0x{conv:08X})");
                false
            }
        };
        self.platform.timer.kill(TIMER_IME_REFRESH);
        let generation = self.platform_state.ime.allocate_event_generation();
        self.platform_state
            .ime
            .handle_engine_set_open(target, false, false, generation, now_tick);
        if matches!(engine, EngineSync::DirectInput) {
            // conv の英数モード観測は IME-ON の確証。direct belief で already_matched を
            // バイパスして apply する。
            let belief = crate::output::OpenBelief {
                effective_open: true,
                confident: true,
            };
            let outcome = self
                .platform
                .apply_ime_open_with_belief(false, None, belief);
            self.on_ime_apply_complete(false, outcome, None);
        }
    }

    /// Shadow IME トグル処理
    ///
    /// IME ON/OFF が変化したら `true` を返す。`kp_stage_execute` がこの値を見て
    /// Imm32Unavailable アプリで物理 IME キーを抑止すべきか判定する。
    // shadow IME belief トグルは分岐が本質的に多い。分割は挙動変更リスクが高いため
    // 複雑度警告のみ抑制する。
    #[expect(clippy::cognitive_complexity)]
    fn kp_stage_shadow_ime_toggle(&mut self, event: &RawKeyEvent) -> bool {
        if !matches!(event.event_type, KeyEventType::KeyDown) {
            return false;
        }
        // BUG-14: 注入イベント (LLKHF_INJECTED、awase 自身のマーカーなし = MS-IME/CTF 等の
        // SendInput) はユーザーの物理操作ではないため、SyncKey / PhysicalImeKey の
        // ユーザー意図に昇格させない。2026-07-06 実機: 外部注入 VK_DBE_HIRAGANA down+up
        // (hook 上では 0xF0 up + 0xF2 down に翻訳、0.5ms 間隔) が PhysicalImeKey と
        // 誤読され、ユーザーの Ctrl+無変換 IME OFF が Engine ON で上書きされ続けた。
        // OS への配送 (passthrough) は従来どおり維持し、実 IME 状態の追従は
        // may_change_ime → schedule_ime_refresh の観測経路に委ねる。
        // hook 層での swallow は不可 (MS-IME 自身の機能的注入を壊す、experiments.md
        // エントリ 04 で実証済み)。
        if event.injected {
            if event.ime_relevance.sync_direction.is_some()
                || event.ime_relevance.shadow_action.is_some()
            {
                log::info!(
                    "[shadow-toggle] injected IME キー vk=0x{:02X} はユーザー意図に昇格させない \
                     (BUG-14) — belief 追従は may_change_ime refresh 観測に委譲",
                    event.vk_code,
                );
            }
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
            // 診断ログ (2026-07-06 "IME-OFF Engine-ON" 報告の切り分け用): belief が
            // 既に new_val と一致しているため apply-ime/dispatch-ime に到達せず、
            // 実 OS IME が別経路 (物理キー直結等) で乖離していても訂正されない。
            // hook.rs の [hook] IME-mode ログと突き合わせ、直前に対応する KeyDown
            // (vk=0xF0 等) が self_injected=false で到達していたか確認すること。
            log::debug!(
                "[shadow-toggle] no-op: vk=0x{:02X} action={:?} source={:?} \
                 effective_open は既に {} → apply-ime 見送り",
                event.vk_code,
                action,
                kind,
                current,
            );

            // TurnOn 系キー（ひらがな/かな 等）は IME が既に open でも「英数から
            // ひらがなへ戻す」ユーザー操作として意味を持つ。OFF→ON 遷移が起きない
            // ためこの上の eisu_reset_on_ime_on（UserImeOnEisuReset）は発火しないので、
            // ここで同様の stale ObservedEisu 救済を別途行う（2026-07-09 MS Edge/MS-IME
            // で実発生: IME open のまま conv だけ Eisu に固着すると、ひらがなキーを
            // 押しても復帰できなかった）。
            if let Some(new_mode) = crate::state::eisu_recovery::eisu_reset_on_turn_on_while_open(
                matches!(action, ShadowImeAction::TurnOn),
                self.platform_state.ime.input_mode(),
            ) {
                // 半角英数持続トグルON中は、通常のObservedEisu→AssumedRomaji書き戻しを
                // スキップしてトグルOFF処理そのものを呼ぶ。実convとbeliefの整合を保つ
                // ため（2026-07-11 codexレビュー: 単に書き戻すとbeliefだけromaji-capable
                // に戻り実convは半角英数のままの壊れた中間状態になる）。
                if self.platform_state.gate.half_width_alnum_toggle_active {
                    log::info!("[shadow-toggle] TurnOn（半角英数トグルON中）→ トグルOFF処理へ委譲");
                    self.kp_restore_kana_from_half_width(false);
                } else {
                    self.platform_state.ime.dispatch_event(
                        crate::state::ime_event::ImeEvent::InputModeApplied {
                            mode: new_mode,
                            strategy:
                                crate::state::ime_event::InputModeApplyStrategy::UserTurnOnEisuReset,
                            result: crate::state::ime_event::InputModeApplyResult::Applied,
                            at: tick_ms,
                        },
                        tick_ms,
                    );
                    log::info!(
                        "[shadow-toggle] TurnOn (IME既にopen) + ObservedEisu → AssumedRomaji に \
                         リセット (UserTurnOnEisuReset)"
                    );
                }
            }
            return false;
        }
        self.platform_state.ime.on_ime_toggled();

        // OFF→ON の場合、stale な ObservedEisu を先回りで訂正する。
        // ObservedEisu は engine activation を NotRomajiInput で塞ぎ、activation 側の
        // 救済 (PostSetOpenEisuReset) は Decision 経由 SetOpen(true) 限定のため、
        // この経路で訂正しないと Imm32Unavailable アプリ（観測経路なし）では
        // engine が永久に inactive のままになる（2026-07-06 MS Edge で実発生）。
        // ユーザーが明示的に IME を ON にした時点で IME はひらがなモードで再開する
        // ため、過去の英数観測は stale（eisu guard の保護対象と衝突しない）。
        if let Some(new_mode) = crate::state::eisu_recovery::eisu_reset_on_ime_on(
            !current && self.platform_state.ime.effective_open(),
            self.platform_state.ime.input_mode(),
        ) {
            // 半角英数持続トグルON中は、通常のObservedEisu→AssumedRomaji書き戻しを
            // スキップしてトグルOFF処理そのものを呼ぶ（E節の理由は上の分岐と同じ）。
            if self.platform_state.gate.half_width_alnum_toggle_active {
                log::info!("[shadow-toggle] IME ON（半角英数トグルON中）→ トグルOFF処理へ委譲");
                self.kp_restore_kana_from_half_width(false);
            } else {
                self.platform_state.ime.dispatch_event(
                    crate::state::ime_event::ImeEvent::InputModeApplied {
                        mode: new_mode,
                        strategy:
                            crate::state::ime_event::InputModeApplyStrategy::UserImeOnEisuReset,
                        result: crate::state::ime_event::InputModeApplyResult::Applied,
                        at: tick_ms,
                    },
                    tick_ms,
                );
                log::info!(
                    "[shadow-toggle] IME ON + ObservedEisu → AssumedRomaji にリセット \
                     (UserImeOnEisuReset, engine 即活性化)"
                );
            }
        }

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
                        app.on_ime_apply_complete(false, outcome, None);
                    });
                    drop(guard);
                });
            } else {
                let outcome = crate::ime_controller::CONTROLLER.apply(false, &view);
                // B+C+D(noop)+E
                self.on_ime_apply_complete(false, outcome, None);
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
    ///
    /// `focus_transition_was_pending`: この event 処理開始時点で FocusTransition
    /// barrier が settle 期間内だったか（`kp_run_inner` でのスナップショット）。
    fn kp_stage_post_decision(
        &mut self,
        decision: &awase::engine::Decision,
        event: &RawKeyEvent,
        focus_transition_was_pending: bool,
    ) {
        if let Some(new_ime_on) = decision.find_ime_set_open() {
            // IME-ON コンボ（既定: Ctrl+変換）は現在の IME 状態によらず SetOpen(true) を
            // 無条件で再発行する（`build_ime_set_open_decision` の「二重 enqueue 防止」
            // コメント参照）。このため handle_engine_set_open で belief を更新する前に
            // 「このイベント処理前は既に IME ON だったか」を控えておく必要がある
            // （2026-07-11 ユーザー要望: IME-ON コンボを IME ON 中に押したら、ひらがな +
            // ローマ字入力 + CapsLock OFF へリセットする。既に OFF→ON の場合は従来通り
            // 単純に ON にするだけで良い）。
            let was_open_before = self.platform_state.ime.effective_open();
            self.platform.timer.kill(TIMER_IME_REFRESH);
            let generation = self.platform_state.ime.allocate_event_generation();
            let tick_ms = crate::state::TickMs(hook::current_tick_ms());
            let applied = self.platform_state.ime.handle_engine_set_open(
                new_ime_on,
                event.modifier_snapshot.ctrl,
                focus_transition_was_pending,
                generation,
                tick_ms,
            );
            log::debug!(
                "IME control: preconditions.ime_on = {new_ime_on} (SetOpenRequest), poll suspended{}",
                if applied { "" } else { " [chord barrier active → skipped]" }
            );

            // `decision.find_ime_set_open()==Some(true)` は IME-ON コンボ以外からも
            // 発火しうる（例: `Ctrl+Shift+変換` = EngineOn コンボで engine が
            // Inactive→Active に遷移する際も `transition_activation` が同じ
            // `SetOpen{open:true}` を無条件で出す、`engine.rs` L171）。「IME が既に
            // ONだった」だけでなく、実際に押されたキーが IME-ON コンボの既定値
            // `Ctrl+変換`（Shift/Alt/Win 無し）と一致することも確認し、無関係な
            // コンボでリセットが誤発火しないようにする。`keys.ime_on` をカスタマイズ
            // した場合はこの判定も合わせて更新すること。
            let is_default_ime_on_combo = event.vk_code == crate::vk::VK_CONVERT
                && event.modifier_snapshot.ctrl
                && !event.modifier_snapshot.shift
                && !event.modifier_snapshot.alt
                && !event.modifier_snapshot.win;
            if applied && new_ime_on && was_open_before && is_default_ime_on_combo {
                Self::kp_reset_to_hiragana_romaji_capsoff();
            }

            // SetOpen(true) 後 input_mode=ObservedEisu が残ると engine が NotRomajiInput で
            // inactive になり、VK_KANJI 送信後も 1500ms 間 NICOLA が処理されない。
            // VK_KANJI 送信により GJI はひらがなへ遷移するため ObservedEisu は stale。
            // AssumedRomaji にリセットして engine を即座に活性化する。
            // (1500ms 後の idle-conv-check で GJI 実状態を再確認・訂正する)
            // 判定は shadow toggle 経路 (UserImeOnEisuReset) と共通の純関数に集約。
            if let Some(new_mode) = crate::state::eisu_recovery::eisu_reset_on_ime_on(
                applied && new_ime_on,
                self.platform_state.ime.input_mode(),
            ) {
                // 半角英数持続トグルON中は、通常のObservedEisu→AssumedRomaji書き戻しを
                // スキップしてトグルOFF処理そのものを呼ぶ（E節、shadow_ime_toggle側の
                // 2箇所と同じ理由）。
                if self.platform_state.gate.half_width_alnum_toggle_active {
                    log::info!(
                        "[post-decision] SetOpen(true)（半角英数トグルON中）→ トグルOFF処理へ委譲"
                    );
                    self.kp_restore_kana_from_half_width(false);
                } else {
                    // これは外部観測ではなく、awase 自身が直前に発行した SetOpen(true) の
                    // 帰結を先読みする能動的な訂正のため InputModeApplied で表現する
                    // (InputModeObserved を使うと「ImmGetOpenStatus で観測した」という
                    // 存在しない API 呼び出しを偽装することになる)。
                    self.platform_state.ime.dispatch_event(
                        crate::state::ime_event::ImeEvent::InputModeApplied {
                            mode: new_mode,
                            strategy:
                                crate::state::ime_event::InputModeApplyStrategy::PostSetOpenEisuReset,
                            result: crate::state::ime_event::InputModeApplyResult::Applied,
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
        }

        if !decision.is_consumed()
            && event.ime_relevance.may_change_ime
            && matches!(event.event_type, KeyEventType::KeyDown)
        {
            self.schedule_ime_refresh(20);
            log::debug!("may_change_ime key passed through → IME refresh scheduled (20ms)");
        }

        self.kp_stage_shift_conv_guard(event);
    }

    /// IME-ON コンボ（既定: Ctrl+変換）を IME 既に ON の状態で押した場合のリセット動作。
    ///
    /// ユーザー要望 (2026-07-11):「Ctrl+変換 で IME-OFF のときは IME-ON だけど、
    /// IME-ON のときは ひらがな・ローマ字・Caps OFF にしてほしい」。IME-OFF→ON は
    /// 既存の `SetOpen(true)` のみで達成されるため変更不要。ここでは「既に ON だった」
    /// 場合にだけ追加で: ひらがな＋ローマ字入力へ conv を寄せ（全角/半角・記号入力等の
    /// 無関係なビットは保持しつつ NATIVE|FULLSHAPE|ROMAN を立てて KATAKANA を落とす）、
    /// 実 OS の Caps Lock を OFF にする。トレイメニューの「状態をリセット」
    /// (`TrayCommand::ResetState`、`tray.rs`) と同じ変換モードのマスクを使う。
    fn kp_reset_to_hiragana_romaji_capsoff() {
        // Caps Lock はトグル表示灯の読み取り (GetKeyState) + 条件付き SendInput のみで、
        // クロスプロセス IMM 呼び出しを含まないためフックスレッドから直接呼んで安全
        // （`is_physical_key_down`/`GetAsyncKeyState` 等、他の同期呼び出しと同水準）。
        // SAFETY: is_caps_lock_on / toggle_caps_lock は Win32 API を呼ぶのみ。
        // メインスレッド（フックスレッド）から呼んでいる。
        if unsafe { crate::ime::is_caps_lock_on() } {
            unsafe { crate::ime::toggle_caps_lock() };
            log::info!("[ime-on-combo] IME 既に ON → Caps Lock を OFF に");
        }
        log::info!(
            "[ime-on-combo] IME 既に ON → ひらがな＋ローマ字入力へリセット \
             (NATIVE|FULLSHAPE|ROMAN を立て KATAKANA を落とす)"
        );
        // IMC read/write はクロスプロセスメッセージを含むため、フックスレッドから直接
        // 呼ばず shift-conv-guard と同じパターンで spawn_local する。現在の conv を
        // 読んでから mask するのは、記号入力等の無関係なビットを保持するため
        // （`set_ime_romaji_mode_with_target_async` は target を丸ごと置換するので、
        // 事前に現在値を読んでマスク計算してから渡す）。
        win32_async::spawn_local(async {
            let current = win32_async::offload(|| unsafe {
                crate::ime::get_ime_conversion_mode_raw_timeout(50)
            })
            .await;
            let set_mask = crate::imm::IME_CMODE_NATIVE
                | crate::imm::IME_CMODE_FULLSHAPE
                | crate::imm::IME_CMODE_ROMAN;
            let clear_mask = crate::imm::IME_CMODE_KATAKANA;
            let target = current.map_or(set_mask, |c| (c | set_mask) & !clear_mask);
            let ok = crate::ime::set_ime_romaji_mode_with_target_async(Some(target)).await;
            if !ok {
                log::warn!("[ime-on-combo] ひらがな＋ローマ字リセットの conv write に失敗");
            }
        });
    }

    /// Shift 押下→解放間の IME conv 安全網、および左Shift単独タップによる
    /// 「IME-ON 半角英数」持続トグル判定（BUG-15 撤去 + 新機能、2026-07-11）。
    ///
    /// # 安全網（両Shift・チョード問わず無条件）
    ///
    /// MS-IME は（設定で無効化不可能な）「Shift 単独タップで英数モードに切替える」
    /// 誤検知を持つ。awase が Shift+文字キーのチョード（`.yab` Shift 面）を engine で
    /// consume すると、OS からは「Shift down→（何も見えない）→Shift up」に見え、
    /// この誤検知が発火する。これを打ち消すため、物理 Shift（L/R 問わず）の
    /// 押下→解放のたびに無条件で conv を英数へ→かなへ書き戻す（BUG-15 旧
    /// `kp_stage_shift_eisu_hold` の無条件挙動を維持）。engine の `input_mode`
    /// belief には触れないため、engine は常時アクティブなまま Shift+文字のチョード
    /// （`should_use_shift_plane`/`shift_face_reduce`）を通常通り処理し続ける。
    ///
    /// # 左Shift単独タップの持続トグル
    ///
    /// 左Shift の押下→解放の間に他の非注入物理キーが一切来なかった場合のみ
    /// 「単独タップ」と判定し、上記の復元を**キャンセル**して代わりに
    /// `half_width_alnum_toggle_active` を立てる（IME-ON 半角英数の持続トグルへ
    /// 移行）。もう一度単独タップしたら通常の復元を実行してトグルを解除する。
    /// 右Shift単独タップ・チョードの場合は常に安全網通り即座に復元する（右Shift
    /// タップはトグルの「緊急解除」としても働く）。
    fn kp_stage_shift_conv_guard(&mut self, event: &RawKeyEvent) {
        use awase::types::ModifierKey;

        // Shift 以外の物理キー（VK_LSHIFT 自身を除く）の KeyDown で単独タップ候補を
        // 折る。VK_RSHIFT も対象（LShift down → RShift down → LShift up を誤って
        // 単独タップ扱いしないため、2026-07-11 codex レビュー指摘）。自己注入は対象外
        // （BUG-14 と同じ理由: 他プロセスの SendInput をユーザーの物理操作として
        // 扱わない）。
        if matches!(event.event_type, KeyEventType::KeyDown)
            && !event.injected
            && event.vk_code != crate::vk::VK_LSHIFT
        {
            self.platform_state.gate.left_shift_tap_candidate = false;
        }

        if event.modifier_key != Some(ModifierKey::Shift) || event.injected {
            return;
        }
        if matches!(event.event_type, KeyEventType::KeyDown) {
            self.kp_shift_conv_guard_key_down(event);
            return;
        }
        self.kp_shift_conv_guard_key_up(event);
    }

    fn kp_shift_conv_guard_key_down(&mut self, event: &RawKeyEvent) {
        // 左Shift・他modifier無し → タップ候補開始。
        if event.vk_code == crate::vk::VK_LSHIFT
            && !event.modifier_snapshot.ctrl
            && !event.modifier_snapshot.alt
            && !event.modifier_snapshot.win
        {
            self.platform_state.gate.left_shift_tap_candidate = true;
        }

        // Ctrl/Alt/Win チョード（ショートカット）では安全網自体を発動しない。
        if event.modifier_snapshot.ctrl
            || event.modifier_snapshot.alt
            || event.modifier_snapshot.win
        {
            return;
        }

        // 常に pending を立てる: `half_width_alnum_toggle_active` 中でも、この
        // Shift down に対応する KeyUp でトグルOFF/右Shift緊急解除の判定を走らせる
        // 必要がある。立て忘れると KeyUp 側の `take()` が false になり、2回目の
        // 左Shiftタップも右Shift緊急解除も一切発火しなくなる
        // （2026-07-11 codex レビューで発覚）。
        self.platform_state.gate.shift_conv_guard_pending = true;

        if self.platform_state.gate.half_width_alnum_toggle_active {
            // 既に conv=0x0000 のはず。再送はしない（冪等化、無駄な書き込み回避）。
            return;
        }

        // かな入力コンテキストのみ: IME ON・engine 有効・conv 書込権限。
        if !self.platform_state.ime.effective_open()
            || !self.platform_state.ime.belief.is_japanese_ime()
            || !self.engine.is_user_enabled()
            || !self.platform.output.conv_mutation_allowed.get()
        {
            self.platform_state.gate.shift_conv_guard_pending = false;
            return;
        }

        let now_tick = crate::state::TickMs(hook::current_tick_ms());
        self.platform_state.ime.note_explicit_ime_action(now_tick);
        log::info!("[shift-conv-guard] Shift 押下 → IME-ON 半角英数へ切替 (conv→0x00000000)");

        let active_ime_kind = crate::tsf::observer::tsf_obs().active_ime_kind();
        log::debug!("[shift-conv-guard] entry: active_ime_kind={active_ime_kind:?}");

        // 入口機構は IME 種別で分岐する。
        //
        // MS-IME: IMC write（`set_ime_romaji_mode_with_target_async`）。この経路は
        // 元々の（BUG-15由来の）実装で実績があり、変更しない。
        //
        // GJI: **entry 機構は現状存在しない（撤回・保留、2026-07-11）**。
        // 試した2つの機構がいずれも実機で機能しないことを確認済み:
        //
        // 1. **IMC write**（一度は `success=true` かつ verify-read で
        //    `conv=0x0` を確認したが、実際の打鍵ではひらがなのまま変化しなかった。
        //    mozc 本家ソース調査で、conversion-mode compartment への書き込みは
        //    GJI の TIP（`win32/tip/tip_text_service.cc`）にとって UI 表示同期
        //    専用の一方向ミラーで、実コンポーザへは一切伝播しないと判明——
        //    read-back の成功は無意味。BUG-25 追補2参照）。
        // 2. **scan 付き `VK_DBE_ALPHANUMERIC` 注入**（scan=0x3A=物理CapsLock位置
        //    → CapsLock 汚染で撤回、追補1／scan=0=非衝突値でも `[hook] IME-mode
        //    vk=0xF0` のログが一度も出現せず、`SendInput` 自体が
        //    `sent=2/2`（OS的には成功）でも awase 自身のフックにすら届かない
        //    ——scan の値によらず SendInput 経由の `VK_DBE_ALPHANUMERIC` 注入
        //    そのものが機能しないと判断。BUG-25 追補3参照）。
        //
        // GJI に対しては entry を試みない（SendInput も送らない）。無理に
        // `half_width_alnum_toggle_active` を立てて engine を pass-through に
        // すると、GJI の実 conv は ひらがな のまま変化しないため、素通しした
        // 生ローマ字キーが GJI 自身の未切替のひらがな変換エンジンにそのまま
        // 入ってしまい、`かな入力が壊れる`という新たな実害が出る（実機確認:
        // 「こんにちはあいうえお」が素通し中もひらがなのまま出力された）。
        // 次の候補は `ITfLangBarItemMgr`/`ITfLangBarItemButton` 経由の
        // 言語バーボタン起動（mozc の `TipTextService` が実登録しており、
        // 本物のクリック起動と同じ `SwitchInputModeAsync` 経路を通るはず）。
        // 未着手・未検証。
        if active_ime_kind == crate::tsf::observer::ActiveImeKind::MicrosoftIme {
            log::info!("[shift-conv-guard] MS-IME経路: IMC write (conv=0x0000) 送信");
            win32_async::spawn_local(async {
                let ok = crate::ime::set_ime_romaji_mode_with_target_async(Some(0)).await;
                log::info!("[shift-conv-guard] IMC write 結果: ok={ok}");
            });

            // 診断用（2026-07-11）: 送信直後に conv を読み取ってログに残す。
            // MS-IME はこの IMC write 自体が実効的な経路なので、この読み取りは
            // 有効な確認になる（GJI では entry を試みないため verify も行わない）。
            win32_async::spawn_local(async {
                win32_async::sleep_ms(150).await;
                let conv = win32_async::offload(|| unsafe {
                    crate::ime::get_ime_conversion_mode_raw_timeout(50)
                })
                .await;
                match conv {
                    Some(c) => {
                        let native = c & crate::imm::IME_CMODE_NATIVE != 0;
                        log::info!(
                            "[shift-conv-guard] entry verify (150ms後): conv=0x{c:08X} \
                             NATIVE={native} ({})",
                            if native {
                                "未だひらがな側 → 半角英数化は未反映"
                            } else {
                                "英数モードに変化した"
                            }
                        );
                    }
                    None => {
                        log::info!(
                            "[shift-conv-guard] entry verify (150ms後): conv 読み取り失敗 (None)"
                        );
                    }
                }
            });
        } else {
            log::info!(
                "[shift-conv-guard] GJI経路: entry 機構なし (未対応、BUG-25 追補3) → \
                 タップ判定はするがトグルは発動しない"
            );
        }
    }

    fn kp_shift_conv_guard_key_up(&mut self, event: &RawKeyEvent) {
        if !std::mem::take(&mut self.platform_state.gate.shift_conv_guard_pending) {
            return;
        }
        let is_left_shift_tap = event.vk_code == crate::vk::VK_LSHIFT
            && std::mem::take(&mut self.platform_state.gate.left_shift_tap_candidate);
        self.platform_state.gate.left_shift_tap_candidate = false;

        // GJI には entry 機構が無い（BUG-25 追補3）ため、左Shift単独タップでも
        // 持続トグルへは絶対に移行しない（移行すると engine が pass-through に
        // なり、生ローマ字キーが GJI 自身の未切替のひらがな変換エンジンへ
        // そのまま入ってかな入力が壊れる）。GJI では常に安全網の復元のみ実行する。
        let toggle_entry_supported = crate::tsf::observer::tsf_obs().active_ime_kind()
            == crate::tsf::observer::ActiveImeKind::MicrosoftIme;

        if is_left_shift_tap
            && toggle_entry_supported
            && !self.platform_state.gate.half_width_alnum_toggle_active
        {
            // 本物の単独タップ、1回目 → 復元をスキップして持続トグルへ移行する。
            self.platform_state.gate.half_width_alnum_toggle_active = true;
            log::info!(
                "[shift-conv-guard] 左Shift単独タップ → 半角英数トグルON (conv=0x0000 維持)"
            );
            let now_tick = crate::state::TickMs(hook::current_tick_ms());
            self.platform_state.ime.dispatch_event(
                crate::state::ime_event::ImeEvent::InputModeApplied {
                    mode: InputModeState::ObservedEisu,
                    strategy:
                        crate::state::ime_event::InputModeApplyStrategy::UserHalfWidthAlnumToggle,
                    result: crate::state::ime_event::InputModeApplyResult::Applied,
                    at: now_tick,
                },
                now_tick,
            );
            return;
        }

        // 2回目の左Shiftタップ（トグルOFF）・チョード・右Shift（トグルの緊急解除も
        // 兼ねる）: 常に安全網の復元を実行する。
        self.kp_restore_kana_from_half_width(true);
    }

    /// 「IME-ON 半角英数」からかな入力への復元（トグルOFF・安全網の復元の共通処理）。
    ///
    /// 責務は belief 更新 + 復元注入 + `half_width_alnum_toggle_active=false` に
    /// 限定する（2026-07-11 codex レビュー: `kp_stage_shadow_ime_toggle` /
    /// `kp_stage_post_decision` 側の既存の物理キー disposition 処理と二重に
    /// 物理キーを扱わないようにするため）。
    ///
    /// `prepend_synthetic_shift_up`: 呼び出し元がまだ物理 Shift up の reinject を
    /// 行っていない（＝ OS 視点でまだ Shift 押下中）場合は true にする。
    /// `kp_shift_conv_guard_key_up` から呼ぶ場合は常に true、フォーカス変更や他の
    /// IME-ON キー起点（E/F 節）から呼ぶ場合は物理 Shift が押されているとは
    /// 限らないため false。
    pub(crate) fn kp_restore_kana_from_half_width(&mut self, prepend_synthetic_shift_up: bool) {
        self.platform_state.gate.half_width_alnum_toggle_active = false;
        let now_tick = crate::state::TickMs(hook::current_tick_ms());
        // idle-conv-check が復元途中の conv=0x0000 を読んで ObservedEisu →
        // DirectInput に落とさないよう、明示的 IME 操作として抑止する。
        self.platform_state.ime.note_explicit_ime_action(now_tick);
        // 次の kana 送信は msime-ready ゲートに IMC の NATIVE を確認させる
        // （MS-IME の誤切替が復元 write より後に来ても先頭文字をリテラル化させない）。
        self.platform
            .output
            .ime_mode_fsm
            .borrow_mut()
            .unconfirm("shift-conv-guard release");
        // IMC の conv write だけでは新 MS-IME (TSF-native) の実モードが英数から戻らない
        // （2026-07-07 実機: [shift-release] の IMC write/read は 0x19/NATIVE を返すのに
        // 実モードは半角英数のままで、ユーザーが物理かなキーを押すと復帰した。
        // 英数→かな方向の IMM→TSF 反映だけが壊れている。かな→英数方向の hold 側は
        // IMC write で実際に効く）。ユーザーの手動回復と同じ VK_DBE_HIRAGANA を注入する。
        //
        // 注入は scan code 付き（make_tsf_key_input, MapVirtualKeyW → JIS で 0x70）で
        // 送ること。scan=0x0 の send_ime_mode_key では MS-IME (TSF) がモードキーとして
        // 処理しない（2026-07-07 実機: [ime-mode] SendInput vk=0xF2 scan=0x0 発火後も
        // 半角英数のまま。物理かなキーの reinject (scan=0x70) と TSF warmup の F2
        // (make_tsf_key_input) は効く — 差分は scan の有無のみ）。
        // 下の IMC write/verify は保険として残す（GJI では未検証、無効でも実害は
        // ログ警告のみ）。
        //
        // ただし scan 付き VK_DBE_HIRAGANA 注入自体にもハザードがある
        // （known-bugs.md BUG-15 追補7: 「解放側 F2=scan 0x70 も実 OFF でかなロック
        // トグルの同族ハザード」）。実 IME が確実に ON でない限り注入してはならない
        // という追補7の教訓を、hold 中より窓が長い持続トグルにも徹底するため、
        // `effective_open()==false` の場合は注入をスキップし IMC write のみに
        // 留める（フォーカス変更で他アプリに切り替わった直後等を想定）。
        if self.platform_state.ime.effective_open() {
            let mut f2_inputs = Vec::with_capacity(3);
            if prepend_synthetic_shift_up {
                // 呼び出し元が物理 Shift up の reinject をまだ行っていない場合、OS
                // 視点ではまだ Shift 押下中。Shift+ひらがなキー = カタカナ切替に
                // 化けないよう、synthetic Shift up を同一バッチの先頭に入れる
                // （物理は解放済みなので restore 不要。後続の本物の Shift up
                // reinject と二重になるが KeyUp の重複は無害）。
                f2_inputs.push(crate::tsf::output::make_tsf_key_input(
                    crate::vk::VK_SHIFT,
                    true,
                ));
            }
            f2_inputs.push(crate::tsf::output::make_tsf_key_input(
                crate::vk::VK_DBE_HIRAGANA,
                false,
            ));
            f2_inputs.push(crate::tsf::output::make_tsf_key_input(
                crate::vk::VK_DBE_HIRAGANA,
                true,
            ));
            let _ = crate::win32::send_input_safe(&f2_inputs);
            log::debug!("[shift-conv-guard] VK_DBE_HIRAGANA (scan 付き) 注入 → ひらがなモード復元");
        } else {
            log::debug!(
                "[shift-conv-guard] effective_open()=false のため VK_DBE_HIRAGANA 注入をスキップ \
                 (IMC write のみ、BUG-15 追補7の教訓)"
            );
        }
        // カタカナ入力中は KATAKANA ビット込みで復元、それ以外はローマ字ひらがな。
        // 注意: 半角英数中の conv 読み取りで conv_mode が HanAlpha に更新されている
        // 場合、imm_conv_target は None → ひらがな target になる（切替前がカタカナ
        // だった記憶は失われる。エッジケースとして許容）。
        let target = self
            .platform
            .output
            .conv_mode
            .get()
            .and_then(awase::engine::ConvMode::imm_conv_target)
            .unwrap_or(
                crate::imm::IME_CMODE_NATIVE
                    | crate::imm::IME_CMODE_FULLSHAPE
                    | crate::imm::IME_CMODE_ROMAN,
            );
        log::info!("[shift-conv-guard] かな入力へ復元 (target=0x{target:08X})");

        self.platform_state.ime.dispatch_event(
            crate::state::ime_event::ImeEvent::InputModeApplied {
                mode: InputModeState::AssumedRomaji {
                    reason: awase::engine::AssumedReason::UserHalfWidthAlnumToggleOff,
                },
                strategy: crate::state::ime_event::InputModeApplyStrategy::UserHalfWidthAlnumToggle,
                result: crate::state::ime_event::InputModeApplyResult::Applied,
                at: now_tick,
            },
            now_tick,
        );

        win32_async::spawn_local(async move {
            // MS-IME の誤切替は shift up の後いつ来るか不定（実測: 478ms 後の
            // idle-conv-check で観測 = 上限 478ms）。冪等な IMC write を
            // 160ms 間隔で最大 4 回（0/160/320/480ms、実測上限をカバー）打ち、
            // NATIVE が確認できた時点で打ち切る。
            const RETRY_INTERVAL_MS: u32 = 160;
            const MAX_TRIES: u32 = 4;
            for attempt in 0..MAX_TRIES {
                let ok = crate::ime::set_ime_romaji_mode_with_target_async(Some(target)).await;
                if !ok {
                    log::warn!("[shift-conv-guard] conv 復元 write #{attempt} 失敗");
                }
                win32_async::sleep_ms(RETRY_INTERVAL_MS).await;
                let conv = win32_async::offload(|| unsafe {
                    crate::ime::get_ime_conversion_mode_raw_timeout(10)
                })
                .await;
                if let Some(c) = conv {
                    if c & crate::imm::IME_CMODE_NATIVE != 0 {
                        log::debug!(
                            "[shift-conv-guard] conv=0x{c:08X} NATIVE 確認 (#{attempt}) → 復元完了"
                        );
                        return;
                    }
                }
            }
            log::warn!("[shift-conv-guard] conv 復元 {MAX_TRIES} 回で NATIVE 未確認のまま終了");
        });
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
        // F2 (VK_DBE_HIRAGANA) を Suppress してよいのは、warmup 戦略が F2 を自前送信
        // （GJI: needs_f2_probe=true）して物理キーの代替になる場合のみ。MsImeStrategy は
        // F2 warmup を送らないため、Suppress すると物理ひらがなキーが食い逃げされて
        // IME ON にならない（BUG-10）。
        let f2_warmup_owned = self.platform.output.f2_warmup_owned();
        let physical = crate::runtime::PhysicalKeyDisposition::plan(
            event,
            profile,
            shadow_toggled,
            is_tsf_mode,
            f2_warmup_owned,
        );
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
    fn apply_effective_ime(
        &mut self,
        effective: bool,
        tick_ms: crate::state::TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        if effective {
            self.platform_state.ime.reset_detect_state();
        }
        self.platform_state
            .ime
            .write_focus_probe(effective, tick_ms, accepted);
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

fn sanitize_focus_probe_open_status(
    probe_ime_on: Option<bool>,
    current_profile: crate::focus::class_names::AppImeProfile,
) -> Option<bool> {
    if current_profile.can_read_imm32_open_status() {
        probe_ime_on
    } else {
        None
    }
}

impl Runtime {
    /// read_ime_state_fast_async の結果を self に適用する（with_app 内で呼ぶ）。
    /// kp_stage_focus_probe の旧同期ロジックを async 完了後に実行する版。
    // FocusProbe 完了後の belief 適用は分岐が本質的に多い。分割・引数構造体化は
    // 挙動変更リスクが高いため複雑度・引数数の警告のみ抑制する。
    #[expect(clippy::needless_pass_by_value)]
    #[expect(clippy::cognitive_complexity)]
    #[allow(clippy::too_many_arguments)]
    fn apply_focus_probe(
        &mut self,
        probe: crate::ime::FastImeProbeResult,
        probe_started_ms: u64,
        warmup_ms: u64,
        gji_last_io_ms: u64,
        last_focus_change_ms: u64,
        shadow_on: bool,
        active_ime_kind: crate::tsf::observer::ActiveImeKind,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        // epoch 照合は呼び出し元 (kp_stage_focus_probe の with_app 内) で完了済み。
        // ここではキャプチャ済みの AcceptedObservation をそのまま使う。

        let now_tick_ms = crate::state::TickMs(hook::current_tick_ms());
        let probe_age_ms = now_tick_ms.saturating_sub(probe_started_ms);
        let ime_on_before_probe = self.platform_state.ime.effective_open();

        let now_ms = now_tick_ms.0;
        let signals =
            compute_focus_probe_grace(now_ms, warmup_ms, gji_last_io_ms, last_focus_change_ms);

        // スリープ復帰後など grace 期間中は read_ime_state_fast が一時的に
        // is_japanese_ime=false を返すことがある。
        // false へのダウングレードは grace active 中は行わない（true はいつでも更新）。
        if probe.is_japanese_ime || !signals.any() {
            self.platform_state
                .ime
                .set_is_japanese_ime(probe.is_japanese_ime);
        }

        let current_profile = self.platform.current_app_profile();
        let probe_ime_on = sanitize_focus_probe_open_status(probe.ime_on, current_profile);
        if probe.ime_on.is_some() && probe_ime_on.is_none() {
            log::debug!(
                "FocusProbe: profile={current_profile:?} は IMM32 open status 非対応のため \
                 probe.ime_on={:?} を破棄",
                probe.ime_on
            );
        }

        // TsfNative/Imm32Unavailable では open status を信用しない。
        // この場合は shadow の apply 値を代替観測として記録し drift 追跡を維持する。
        let used_shadow_fallback = probe_ime_on.is_none() && probe.is_japanese_ime;

        let suppressed_reason: Option<&'static str> = if let Some(on) = probe_ime_on {
            let effective = on && probe.is_japanese_ime;
            if !effective && signals.any() {
                Some(signals.primary_reason())
            } else {
                self.apply_effective_ime(effective, now_tick_ms, accepted);
                None
            }
        } else {
            // TsfNative/Imm32Unavailable: IMM32 非対応のため probe は常に None を返す。
            // shadow の apply 値を代替観測として focus_probe スロットに記録する。
            if probe.is_japanese_ime {
                self.apply_effective_ime(shadow_on, now_tick_ms, accepted);
            }
            None
        };

        // TsfNative フォーカス復帰時: conv mode を読んで ConvModeMgr（warmup 用）と
        // prev_conversion_mode を更新する。
        //
        // 【belief は更新しない】フォーカス変更直後に読んだ conv 値は、そのウィンドウの
        // 「たまたまの残留状態」であり、ユーザーが今このモードを望んでいるという signal
        // ではない（ALT+TAB でウィンドウを切り替えただけで、以前そのウィンドウが JIS かな
        // 等の状態で放置されていた場合、fresh read でそれを拾って belief をひらがな/ローマ字
        // から巻き戻してしまうバグの温床だった）。同一ウィンドウ内でタスクバーからモードを
        // 変更した場合は idle-conv-check（TYPING_IDLE_MS 経過後の次キー入力で発火）が
        // 正当なユーザー操作として拾うため、そちらに一本化する。
        if crate::focus::class_names::is_effectively_tsf_native(
            self.platform.current_app_profile(),
            self.platform.focus.class_name(),
        ) && probe.is_japanese_ime
        {
            let in_flight = self.platform.output_in_flight_ms();
            // cold start: ROMAN ビットが信頼できないためスキップ
            if in_flight != u64::MAX {
                // SAFETY: メッセージループスレッドから呼ぶ。10ms タイムアウト。
                if let Some(conv) = unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(10) } {
                    self.platform
                        .output
                        .conv_mode
                        .update_from_conv(conv, now_tick_ms);
                    self.platform_state.ime.set_prev_conversion_mode(Some(conv));
                    log::debug!(
                        "[focus-conv-check] TsfNative: conv=0x{conv:08X} 読み取り（belief 更新なし、\
                         フォーカス変更直後の値はユーザー意図の signal ではないため idle-conv-check に一任）"
                    );
                }
            }
        }

        // ImmCross アプリ（Qt/LINE 等）: FocusProbe は top-level hwnd の IMC を読むが、
        // GJI 使用時は child hwnd と IME 状態が異なる場合がある（Qt の IME コンテキスト分割）。
        // read_ime_state_full_async で child hwnd を正確に読み、High confidence 観測として記録する。
        // これにより FocusProbe (Low) が誤って false を返しても derive_open() で正しく上書きされる。
        //
        // エポック照合: FocusProbe の admit() 済み epoch を引き継ぐ。
        // apply_focus_probe の呼び出し前に epoch チェックを通過しているため
        // accepted.focus_epoch は現在の epoch と等しいことが保証済み。
        if matches!(
            self.platform.current_app_profile(),
            crate::focus::classify::AppImeProfile::Standard,
        ) && probe.is_japanese_ime
        {
            let ticket = crate::state::probe_admission::ImmLikeTicket {
                focus_epoch: accepted.focus_epoch,
            };
            win32_async::spawn_local(async move {
                // SAFETY: read_ime_state_full_async は offload 済み — メインスレッド不要。
                let snap = crate::ime::read_ime_state_full_async().await;
                if let Some(open) = snap.ime_on {
                    let _ = crate::with_app(|app| {
                        let current_epoch = app.platform_state.focus.focus_epoch;
                        let crate::state::probe_admission::Admission::Accept(inner_accepted) =
                            ticket.admit(current_epoch)
                        else {
                            log::debug!(
                                "[ImmCrossProbe] epoch rejected (focus changed since probe spawn)"
                            );
                            return;
                        };
                        let tick_ms = crate::state::TickMs(hook::current_tick_ms());
                        let ime = &mut app.platform_state.ime;
                        // ON/OFF: High confidence (ImmCrossProbe source)
                        ime.write_imm_cross_probe(open, tick_ms, inner_accepted);
                        log::debug!(
                            "[ImmCrossProbe] child-hwnd IME={open} → High confidence 観測記録"
                        );
                        // input_mode: Observe → pure decision → belief
                        // classify_fetched_snapshot = classify_ime_snapshot の同期 wrapper。
                        // ObservedEisu stale 回復を含む全 input_mode 判定をここに集約する。
                        let update = crate::observer::ime_observer::classify_fetched_snapshot(
                            &snap,
                            tick_ms.0,
                            ime.effective_open(),
                            ime.is_force_on_guard_active(),
                            ime.input_mode(),
                            ime.belief.prev_conversion_mode(),
                        );
                        if let Some(mode) = update.new_input_mode {
                            use crate::state::ime_event::{
                                ImeEvent, ObservationConfidence, ObservationSource,
                            };
                            ime.dispatch_event(
                                ImeEvent::InputModeObserved {
                                    mode,
                                    source: ObservationSource::ImmCrossProbe,
                                    confidence: ObservationConfidence::High,
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
                if let (Some(true), Some(conv)) = (snap.ime_on, snap.conversion_mode) {
                    let mode = awase::engine::ConvMode::from_u32(conv);
                    if !mode.is_eisu() && !mode.romaji {
                        let should_restore = crate::with_app(|app| {
                            let ime = &app.platform_state.ime;
                            ime.effective_open()
                                && !matches!(ime.input_mode(), InputModeState::ObservedKana)
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
            });
        }

        let ime_on_after_probe = self.platform_state.ime.effective_open();
        let input_mode_after_probe = self.platform_state.ime.input_mode();
        let ime_on_suffix = build_ime_on_suffix(
            probe_ime_on,
            suppressed_reason,
            &signals,
            probe_age_ms,
            used_shadow_fallback,
        );

        let gji_fields =
            if active_ime_kind == crate::tsf::observer::ActiveImeKind::GoogleJapaneseInput {
                format!(
                    " gji_io={}ms sig2={}",
                    if signals.gji_idle_ms == u64::MAX {
                        "never".to_string()
                    } else {
                        signals.gji_idle_ms.to_string()
                    },
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
                "FocusProbe: TsfNative/Imm32Unavailable — shadow 値 {shadow_on} を代替観測として記録 \
                 [probe_age={probe_age_ms}ms]"
            ),
            None if probe.ime_on.is_none() => log::warn!(
                "FocusProbe: ime_on 未検出 — stale値 {ime_on_before_probe} \
                 [probe_age={probe_age_ms}ms]",
            ),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::class_names::AppImeProfile;

    #[test]
    fn focus_probe_open_status_is_ignored_for_imm32_unavailable() {
        assert_eq!(
            sanitize_focus_probe_open_status(Some(false), AppImeProfile::Imm32Unavailable),
            None
        );
    }

    #[test]
    fn focus_probe_open_status_is_ignored_for_tsf_native() {
        assert_eq!(
            sanitize_focus_probe_open_status(Some(false), AppImeProfile::TsfNative),
            None
        );
    }

    #[test]
    fn focus_probe_open_status_is_kept_for_standard() {
        assert_eq!(
            sanitize_focus_probe_open_status(Some(false), AppImeProfile::Standard),
            Some(false)
        );
    }
}
