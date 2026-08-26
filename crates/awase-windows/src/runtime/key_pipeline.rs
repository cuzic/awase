#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! キーイベント処理パイプライン
//!
//! `on_key_event_impl` の処理を段階的に分割したもの。
//! フックコールバック本体から `Runtime::process_key_event` を呼ぶことで
//! 同じ動作をより読みやすい形で表現する。

use crate::hook;
use crate::hook::CallbackResult;
use crate::state::evidence::IntentWitness;
use crate::state::observation_store::FocusProbeOpenStatus;
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

        let (left_thumb_down, right_thumb_down) = hook::thumb_down_timestamps();
        let ctx = super::build_input_context(
            self.platform_state.ime.effective_open(),
            self.platform_state.ime.input_mode(),
            self.platform_state.ime.belief.is_japanese_ime(),
            crate::tsf::observer::ime_composition_active_now(),
            &event.modifier_snapshot,
            left_thumb_down,
            right_thumb_down,
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
             pending_drain={} gate_active={} \
             [diag-ctx] ime_on={} japanese={} input_mode={:?} composing={}",
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
            ctx.ime_on,
            ctx.is_japanese_ime,
            ctx.input_mode,
            ctx.composing,
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
            self.schedule_settle_retry("SetOpen stripped from kp_run_inner decision");
        }
        let state_after = self.engine.debug_state_label();
        // 配送判断(physical)をここで一度だけ確定させ、KeyInput journal 記録と
        // kp_stage_execute の実処理の両方に同じ値を渡す（BUG-90 調査: 以前は
        // journal 記録用に独立して再計算しており、理論上わずかな乖離窓が
        // あった。詳細は `PhysicalKeyDisposition::suppress_reason` のコメント
        // 参照。decision だけでは実際に OS へ届いたかが journal から
        // 分からなかった点が調査のきっかけ）。
        let profile = self.platform.current_app_profile();
        let physical = crate::runtime::PhysicalKeyDisposition::plan(
            &event,
            profile,
            shadow_toggled,
            self.platform.is_tsf_mode(),
            self.platform.output.f2_warmup_owned(),
            crate::tsf::observer::tsf_obs().active_ime_kind(),
            self.dbe_mode_key_policy,
        );
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::KeyInput {
                event: crate::journal::KeyEventSummary::from_raw(&event),
                state_before,
                state_after,
                decision: crate::journal::DecisionKind::from_decision(&decision),
                physical: crate::journal::PhysicalDispositionSummary::new(
                    physical.suppress_reason(&event, profile),
                ),
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

        let callback = self.kp_stage_execute(decision, &event, profile, physical);
        for entry in self.platform.drain_journal_entries() {
            self.platform_state.ime.journal.absorb(entry);
        }
        callback
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
        // spawn 時にチケットをキャプチャ。apply_focus_probe 完了時に epoch/hwnd を照合し
        // stale な観測を棄却する（ADR-106 決定3）。
        let ticket = crate::state::probe_admission::ImmLikeTicket {
            focus_epoch: self.platform_state.focus.focus_epoch,
            hwnd: self.focus_hwnd(),
        };

        win32_async::spawn_local(async move {
            let probe = crate::ime::read_ime_state_fast_async().await;
            let _ = crate::with_app(|app| {
                crate::state::probe_admission::admit_epoch_in_app(
                    app,
                    ticket,
                    "[FocusProbe] epoch rejected (focus changed since probe spawn)",
                    |app, accepted| {
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
                    },
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
        let _ = self.kp_stage_idle_conv_check_inner(event, false, None);
    }

    /// フォーカス復帰後 resync（report `01M0VGJ2M5KQHD1D9V7HAMBHNT`）のトリガー用。
    ///
    /// `app/mod.rs` の trigger 分岐で `RawKeyEvent::starts_focus_resync()` が true の
    /// 最初のキーが到着したとき、そのキーを `INPUT_DEFER` へ退避すると同時に呼ぶ。
    /// 通常の `kp_stage_idle_conv_check` と同じガード・同じ conv 読み取りチェーンを
    /// 使うが、ガード3（タイピング停止判定）のみバイパスする
    /// （`is_first_key_after_focus=true`）。ガード1・2・4は必ず効く。
    ///
    /// `generation` は `FocusResyncGate::consume_and_close()` が返した世代番号。
    /// conv 読み取り完了時、この世代がまだ現行（＝ハード期限タイマーがまだ
    /// 先に閉じていない）ことを確認してから belief を適用し、gate を閉じて
    /// drain を post する。世代が古ければ（期限が先に発火済み）結果を破棄する。
    ///
    /// 戻り値: 呼び出し元（`app/mod.rs`）はこの `true` のときだけハード期限
    /// タイマーを武装すること。ガード（shift ガード／`should_run_idle_conv_check`
    /// 不通過）で同期的に弾かれた場合は既に gate が閉じているため `false` を返す
    /// ——期限タイマーを張っても無害（`open_if_current` が世代不一致で無視する）だが、
    /// 「成功パスでは無駄な `WM_TIMER` を残さない」という設計意図と矛盾するため、
    /// 呼び出し元が武装をスキップできるようにする（BUG-77 code review 追補2巡目）。
    pub(crate) fn kp_trigger_focus_resync(&mut self, event: &RawKeyEvent, generation: u64) -> bool {
        self.kp_stage_idle_conv_check_inner(event, true, Some(generation))
    }

    /// 戻り値: 非同期 conv 読み取りを spawn した（＝resync の場合、gate がまだ
    /// active で呼び出し元がハード期限タイマーを武装すべき）なら `true`。
    /// いずれかのガードで同期的に return した場合は `false`。
    fn kp_stage_idle_conv_check_inner(
        &mut self,
        event: &RawKeyEvent,
        is_first_key_after_focus: bool,
        resync_generation: Option<u64>,
    ) -> bool {
        // Shift conv 安全網のブリップ中、または左Shift単独タップによる半角英数
        // 持続トグル中（`kp_stage_shift_conv_guard`）は凍結する。conv=0x00000000 は
        // awase 自身が意図的に設定した状態であり、ObservedEisu → DirectInput
        // （IME OFF 落ち）に反応させてはならない。Shift 解放時の復元が
        // explicit IME action として抑止を引き継ぐ。
        if self.platform_state.gate.shift_conv_guard_pending
            || self.platform_state.gate.half_width_alnum_toggle_active
        {
            self.close_focus_resync_gate_if_current(resync_generation);
            return false;
        }
        let output_idle_ms_at_spawn = self.platform.output_in_flight_ms();
        let now_tick_at_spawn = crate::state::TickMs(hook::current_tick_ms());
        let explicit_action_ms_at_spawn = self.platform_state.ime.last_explicit_ime_action_ms_raw();
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
            is_first_key_after_focus,
        ) {
            // explicit IME 操作直後のスキップのみデバッグログを残す
            // （KeyDown・TsfNative・idle の 3 条件を通過した上で explicit_age だけが残っている場合）
            if matches!(event.event_type, KeyEventType::KeyDown)
                && is_tsf_native
                && (output_idle_ms_at_spawn > crate::tuning::TYPING_IDLE_MS
                    || is_first_key_after_focus)
                && explicit_age < crate::tuning::EXPLICIT_IME_SUPPRESS_MS
            {
                log::debug!(
                    "[idle-conv-check] TsfNative: explicit IME action {}ms ago → スキップ (suppress={}ms)",
                    explicit_age,
                    crate::tuning::EXPLICIT_IME_SUPPRESS_MS,
                );
            }
            self.close_focus_resync_gate_if_current(resync_generation);
            return false;
        }

        // BUG-34（docs/known-bugs.md）: get_ime_conversion_mode_raw_timeout は
        // SendMessageTimeoutW(SMTO_ABORTIFHUNG) ベースで、ハング判定が確定するまで
        // 実質 timeout_ms を無視して ~5s ブロックしうる。エンジンスレッド上で同期に
        // 呼ぶとメッセージループが詰まり打鍵が消える。offload してワーカースレッドで
        // 実行する。
        //
        // 多重 in-flight 防止: GJI が本当にハングしている間、断続的なタイピングで
        // offload 呼び出しが積み上がるのを防ぐ（1 件 in-flight の間は新規 spawn しない）。
        //
        // BUG-34 横展開レビュー指摘: 完了 closure（下記 `with_app`）が再入で `None` を
        // 返した場合、フラグを戻す機会が失われ、以後 idle-conv-check が永久に発火
        // しなくなる恐れがあった。spawn 時刻を持たせ、
        // `IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS` を超えていれば「放棄された」とみなして
        // 新規 spawn を許可することで自己回復させる。
        // BUG-77 code review 追補: この in-flight フラグは「通常の idle-conv-check」の
        // 多重 spawn 防止専用であり、resync（`resync_generation.is_some()`）はこれを
        // 共有しない。resync は `FocusResyncGate` の one-shot 消費（`consume_and_close`
        // を呼べるのはフォーカス変更ごとに1回だけ）で既に多重 spawn しない構造になって
        // おり、この spam ガードを必要としない。共有すると、無関係な通常キー
        // （例: Ctrl+V）が偶然先に in-flight を掴んでいた場合、resync 対象キーの
        // 読み取りが「in-flight のためスキップ」に落ちて gate が即座に閉じられ、
        // resync が一度も実際の conv を読まないまま defer 中のキーが stale な belief
        // で drain されてしまう（BUG-77 の再発）。resync は常に自分専用の読み取りを
        // spawn し、共有フラグの読み書きを一切行わない。
        let now_ms_for_gate = hook::current_tick_ms();
        if resync_generation.is_none() {
            if let Some(since) = self.platform_state.gate.idle_conv_check_in_flight_since_ms {
                let elapsed = now_ms_for_gate.saturating_sub(since);
                if elapsed < crate::state::platform_state::IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS {
                    log::debug!(
                        "[idle-conv-check] 前回の conv 読み取りが in-flight のためスキップ"
                    );
                    return false;
                }
                log::warn!(
                    "[idle-conv-check] 前回の in-flight が {elapsed}ms 未解放 → 放棄されたとみなし再武装"
                );
            }
            self.platform_state.gate.idle_conv_check_in_flight_since_ms = Some(now_ms_for_gate);
        }

        // spawn 時にチケットをキャプチャ。apply_idle_conv_check 完了時に epoch/hwnd を照合し
        // フォーカスが変わっていれば stale な観測を棄却する（kp_stage_focus_probe と同型、
        // ADR-106 決定3）。`accepted.hwnd`（decision3 で追加）を decision4 の
        // `ConvModeMgr::observe()` monotonic guard にもそのまま使う——`ImmLikeTicket` は
        // 元々 epoch のみを追跡していたため、同一プロセス内でウィンドウだけが変わる
        // ケース（`focus_epoch` はプロセス変更でのみ進む）を捕まえるために ImmLikeTicket
        // 自体に hwnd を持たせた。
        let ticket = crate::state::probe_admission::ImmLikeTicket {
            focus_epoch: self.platform_state.focus.focus_epoch,
            hwnd: self.focus_hwnd(),
        };
        // BUG-34 横展開 Step0-a: 自己出力の再検証を conv_mutation_seq のビット一致に
        // 一本化する（旧 output_in_flight_ms ベースの last_send 比較は
        // apply_idle_conv_check 側で撤去、下記 doc 参照）。
        let conv_mutation_seq_at_spawn = crate::conv_mutation::current();
        win32_async::spawn_local(async move {
            let conv = crate::ime::get_ime_conversion_mode_raw_timeout_async(10).await;
            let _ = crate::with_app(|app| {
                if resync_generation.is_none() {
                    app.platform_state.gate.idle_conv_check_in_flight_since_ms = None;
                }
                // BUG-77 code review 追補(2巡目): 世代照合は belief 適用の**前**に行う。
                // 以前は apply_idle_conv_check を呼んだ後に世代照合していたため、
                // ハード期限が先に発火した後（BUG-34 で conv 読みが数秒ブロックした
                // ケース）に遅れて届いた結果がそのまま belief に書き込まれてしまい、
                // タイピング中に突然 force-ON が飛ぶ BUG-31/BUG-70 系の事故を生んでいた。
                // `close_focus_resync_gate_if_current` は「resync_generation が None
                // （通常呼び出し、gate に無関係）なら常に true」「Some(gen) なら
                // gate を閉じられた（＝自分の世代がまだ現行だった）ときだけ true」を
                // 返すので、false の場合は conv の有無を問わず belief 適用ごと破棄する。
                if !app.close_focus_resync_gate_if_current(resync_generation) {
                    return;
                }
                let Some(conv) = conv else { return };
                crate::state::probe_admission::admit_epoch_in_app(
                    app,
                    ticket,
                    "[idle-conv-check] epoch rejected (focus changed since read spawn)",
                    |app, accepted| {
                        app.apply_idle_conv_check(
                            conv,
                            conv_mutation_seq_at_spawn,
                            explicit_action_ms_at_spawn,
                            accepted.focus_epoch,
                            accepted.hwnd,
                        );
                    },
                );
            });
        });
        true
    }

    /// resync 対象キーの conv チェックが完了・スキップ・棄却されたとき呼ぶ。
    ///
    /// `resync_generation` が `None`（通常の idle-conv-check 呼び出し）なら resync
    /// gate に一切関与せず常に `true` を返す。`Some(gen)` のときは
    /// `FocusResyncGate::open_if_current` で世代照合する——world-first として
    /// gate を閉じられた（＝自分の世代がまだ現行だった）場合のみ `true` を返し、
    /// ハード期限タイマーを kill してから drain を post する
    /// （`should_post_drain` で `OUTPUT_GATE` に委譲するかどうかも判定する）。
    ///
    /// **呼び出し元は戻り値が `false` のとき、conv 結果があっても belief 適用
    /// （`apply_idle_conv_check`）を行ってはならない**（BUG-77 code review 追補
    /// 2巡目: 世代が古い＝ハード期限が先に発火済みの結果は破棄する契約）。
    fn close_focus_resync_gate_if_current(&mut self, resync_generation: Option<u64>) -> bool {
        let Some(generation) = resync_generation else {
            return true;
        };
        let opened = crate::focus_resync::FOCUS_RESYNC.open_if_current(generation);
        if opened {
            self.platform.timer.kill(crate::TIMER_FOCUS_RESYNC);
            if crate::state::focus_resync_policy::should_post_drain(
                crate::tsf::probe_bridge::OUTPUT_GATE.is_active(),
            ) {
                crate::tsf::probe_bridge::post_drain_output_queue();
            }
        }
        opened
    }

    /// `get_ime_conversion_mode_raw_timeout_async` の結果を self に適用する
    /// （`with_app` 内で呼ぶ）。`kp_stage_idle_conv_check` の旧同期ロジックを
    /// async 完了後に実行する版。
    ///
    /// 読み取りが in-flight の間（旧同期コードでは起こり得なかった隙間）に、
    /// awase 自身が (a) shift ガードを立てる、(b) explicit IME 操作を記録する、
    /// (c) conv ワードを変えうる書き込みを送る、のいずれかを行っていたら、
    /// 読み取った `conv` は awase 自身の遷移途中を拾った汚染値の可能性がある。
    /// spawn 時のスナップショット（`conv_mutation_seq_at_spawn` /
    /// `explicit_action_ms_at_spawn`）と apply 時点を突き合わせて、これらが
    /// 起きていないことを再確認してから適用する。
    fn apply_idle_conv_check(
        &mut self,
        conv: u32,
        conv_mutation_seq_at_spawn: u64,
        explicit_action_ms_at_spawn: u64,
        spawn_focus_epoch: crate::state::probe_admission::FocusEpoch,
        spawn_hwnd: crate::state::ime_event::HwndId,
    ) {
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

        // (b) explicit IME 操作の再検証（値の一致比較）: spawn 後に Ctrl+変換/無変換 や
        // shift-conv-guard の note_explicit_ime_action が発生していないかを、経過時間
        // ではなく `last_explicit_ime_action_ms` の値そのものの一致で判定する。
        //
        // 旧実装は apply 時点の経過時間 (`explicit_ime_action_age_ms`) を
        // `EXPLICIT_IME_SUPPRESS_MS` と比較していたが、`get_ime_conversion_mode_raw_timeout_async`
        // が BUG-34（`SendMessageTimeoutW(SMTO_ABORTIFHUNG)` が指定タイムアウトを無視して
        // 数秒ブロックしうる、docs/known-bugs.md）で長時間ブロックすると、spawn 直後に
        // 明示操作（Shift 押下による shift-conv-guard 突入等）が起きても、apply 時点では
        // 経過時間が閾値を超えてしまい素通りしていた。値の一致比較なら、遅延の長さに
        // 関わらず「spawn〜apply の間に明示操作があった」事実だけで確実に棄却できる。
        if self.platform_state.ime.last_explicit_ime_action_ms_raw() != explicit_action_ms_at_spawn
        {
            log::debug!(
                "[idle-conv-check] apply 時に spawn 後の explicit IME action を検出 → \
                 読み取り結果 conv=0x{conv:08X} を破棄 (spawn={explicit_action_ms_at_spawn}ms)"
            );
            return;
        }
        // この age 比較は spawn〜apply 間の変化検出には使わない（そちらは上の (b1) の
        // ビット一致が担う）。ここは「suppress window 内かどうか」という独立した
        // 業務ルールの判定であり、年齢は足切りとしてのみ使う
        // （BUG-34 追補が否定したのは「経過時間で spawn〜apply 間の変化を判定する」
        // パターンであり、この suppress window 判定はそれとは別軸）。
        let explicit_age = self.platform_state.ime.explicit_ime_action_age_ms(now_tick);
        if explicit_age < crate::tuning::EXPLICIT_IME_SUPPRESS_MS {
            log::debug!(
                "[idle-conv-check] apply 時に explicit IME action {explicit_age}ms 前 → \
                 読み取り結果 conv=0x{conv:08X} を破棄 (suppress={}ms)",
                crate::tuning::EXPLICIT_IME_SUPPRESS_MS,
            );
            return;
        }

        // (c) 自己出力の再検証: spawn〜apply の間に awase 自身が conv ワードを
        // 変えうる書き込み（VK_DBE_*/VK_KANA/VK_CONVERT 送信）を行っていれば、
        // conv は遷移途中を拾った可能性が高い。
        //
        // BUG-34 横展開 Step0-a: 旧実装は `output_in_flight_ms()`（最終送信からの
        // 経過 ms）を絶対時刻に換算して突き合わせていたが、`Output::send_keys` が
        // 冒頭・末尾で呼ぶ `mark_send()` は **NICOLA の通常の文字出力（conv を
        // 一切変えない）でも呼ばれる**ため、打鍵のたびにこの fence が誤って
        // 落ちていた。しかも `send_eager_tsf_warmup` が呼ぶ `send_eager_warmup_vk_pair`
        // （本来検出すべき自己出力の代表例）は `mark_send` を一切通らないため、
        // 検出すべきものを1つも捕捉できていなかった（過剰かつ不足の二重の
        // 誤判定）。`conv_mutation_seq`（`win32::send_input_safe` の唯一のゲート、
        // conv ワードを変えうる VK でのみ増分）のビット一致に置き換える。
        let conv_mutation_seq_now = crate::conv_mutation::current();
        if conv_mutation_seq_now != conv_mutation_seq_at_spawn {
            log::debug!(
                "[idle-conv-check] apply 時に自己出力(conv変異)を検出 \
                 (conv_mutation_seq {conv_mutation_seq_at_spawn}→{conv_mutation_seq_now}) → \
                 読み取り結果 conv=0x{conv:08X} を破棄"
            );
            return;
        }
        let in_flight = self.platform.output_in_flight_ms();

        // 変換モードを更新: idle-conv-check が conv を読んだタイミングで ConvModeMgr に通知する。
        // warmup の先頭 VK 選択と ImmSetConversionStatus の目標値決定に使われる。
        // ADR-106 決定4: 観測は spawn 時点の focus_epoch/hwnd を運び、現在の値と
        // 異なれば（フォーカスが変わっていれば）棄却される（monotonic guard）。
        let current_epoch = self.platform_state.focus.focus_epoch;
        let current_hwnd = crate::state::ime_event::HwndId(self.platform.focus.current.hwnd);
        let conv_mode_changed = self.platform.output.conv_mode.observe(
            crate::state::conv_mode::ConvObservation {
                mode: awase::engine::ConvMode::from_u32(conv),
                read_at: now_tick,
                focus_epoch: spawn_focus_epoch,
                hwnd: spawn_hwnd,
                source: crate::state::conv_mode::ConvReadSource::IdleCheck,
            },
            current_epoch,
            current_hwnd,
        );

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
        // `observe()` 済みのデバウンス確定値）を渡す。BUG-19: `conv` を直接
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
                // このブランチは desired ≠ observed の乖離を記録するだけで自らは
                // actuate しない（上記コメント通り BUG-19 対策）。実際の補正判断は
                // `ir_apply_drift_correction`（`TIMER_IME_REFRESH` 発火時のみ実行）に
                // 委ねられるが、TsfNative では `explicit_intent` 確定後にこのタイマーが
                // 恒久停止する設計のため、ここで明示的に蹴らないと乖離が無期限に
                // 検出されないまま残る（2026-08-04, BUG-51: 実機で最大8分放置された
                // 不具合）。ここで記録した観測は今まさに取得したばかりで新鮮なため
                // （`DRIFT_CORRECTION_OBS_MAX_AGE_MS` に対して十分に新しい）、
                // `may_change_ime` パススルーと同じ 20ms 遅延で安全に確認できる。
                self.schedule_ime_refresh(20);
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
        if matches!(engine, EngineSync::DirectInput) {
            // DirectInput: desired_open=false の belief 書き込みが
            // is_eligible_for_ime_force_on() 経由で force-ON 3経路（ADR-086
            // conv_mode_policy=force 実機ソーク中の経路含む）・
            // last_user_explicit_off_ms・from_explicit_off_intent を支えている
            // load-bearing な書き込みのため、従来どおり handle_engine_set_open を使う
            // （BUG-51 追補 v3 pre-mortem #2、「なぜ DirectInput を変えないか」）。
            self.platform_state
                .ime
                .handle_engine_set_open(target, false, false, generation, now_tick);
            // conv の英数モード観測は IME-ON の確証。direct belief で already_matched を
            // バイパスして apply する。
            let belief = crate::output::OpenBelief {
                effective_open: true,
                confident: true,
            };
            // ADR-090 §2.A A-1（shadow）。
            let order = self.issue_actuation_order(false, "idle_conv_check_direct_input");
            let outcome = self
                .platform
                .apply_ime_open_with_belief(order, None, belief);
            self.on_ime_apply_complete(
                false,
                outcome,
                None,
                crate::state::ime_event::OpenApplyReason::DriftCorrection,
            );
        } else {
            // SetOpen(RomajiRecovered): conv 観測からの自動同期であり、ユーザーの
            // 明示操作ではない。発火条件が effective_open==true を要求するため
            // desired_open へ書くと desired_open := effective_open という循環 echo
            // （ime_model.rs の EngineActivationSync arm が明文で禁じるパターン）に
            // なる。BUG-48 の ActivationSync 経路（last_intent/desired_open/
            // IntentStore を書かず actuation は同一）を使う（BUG-51 追補 v3）。
            self.platform_state
                .ime
                .handle_engine_activation_sync(target, false, false, generation, now_tick);
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
        // ADR-093: VK_DBE_ALPHANUMERIC/KATAKANA/HIRAGANA/SBCSCHAR/DBCSCHAR
        // (0xF0-0xF4) は通常の物理キーボードには存在しない IME 専用の合成 VK
        // コードであり、この KeyDown が届くこと自体が「何らかの IME がこの
        // キーを処理・報告している」証拠になる。is_japanese_ime() は probe
        // ベースの確率的信念で、スリープ復帰/フォーカス変更直後の grace 期間中
        // 一時的に false を誤答しうる既知の弱点がある
        // (apply_focus_probe の non_ascii コメント参照)。この5 VK の受信を
        // is_japanese_ime() の即時 true 更新トリガーに使い、grace 中の
        // false 誤答を訂正する。
        //
        // false へのダウングレードには一切関与しない — この5 VK が「来ない」
        // ことは「日本語 IME でない」ことの証拠にはならない（既存の probe
        // ベースの downgrade 経路は変更しない）。
        //
        // **物理（非注入）イベントに限定する**（Opus コードレビュー指摘で
        // 追加した制約、当初案は BUG-14 の注入イベント除外より前に無条件で
        // 置いていた）。`is_japanese_ime()` は force-ON actuation ゲート
        // `is_eligible_for_ime_force_on()`（`state/platform_state.rs:599`、
        // `is_japanese_ime() && effective_open()`）を含む約10箇所の消費者を
        // 持つグローバルな belief であり、ここを true にすると次の打鍵から
        // force-ON 等の actuation 経路が新たに解禁される。注入イベント
        // （外部プロセスの SendInput、BUG-14 の実例では MS-IME/CTF 自身）を
        // 信頼して actuation の根拠にすると、BUG-14 と同じ「注入イベントを
        // 過度に信頼する」失敗の再発になりうるため、この upgrade は物理
        // キー入力のみに限定する（`event.injected` の判定自体は下記 BUG-14
        // ブロックの早期 return より前に確認する必要があるため、ここで直接見る）。
        if crate::vk::should_upgrade_is_japanese_ime(event.injected, event.vk_code) {
            self.platform_state.ime.set_is_japanese_ime(true);
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
        // 診断ログ (2026-08-05 "IME OFF 後 FocusChange 無しで Engine が勝手に ON へ
        // 戻る" 再発報告の切り分け用): このステージが last_intent を書き換える唯一
        // 経路の一つでありながら、従来ここには INFO ログが一切無く、実機ログだけでは
        // どの VK がこの昇格を発火させたか判別できなかった。挙動は変更しない。
        log::info!(
            "[shadow-toggle] intent 昇格: vk=0x{:02X} scan=0x{:02X} action={:?} \
             kind={:?} injected={} {}→{}",
            event.vk_code,
            event.scan_code,
            action,
            kind,
            event.injected,
            current,
            new_val,
        );
        // witness は「注入されていない実キーイベント」の存在証明（BUG-14 の
        // 型化、ADR-089 §2.2）。上の `event.injected` 早期 return と同じ条件を
        // 型側でも要求するため、ここで None になることは無い。
        match kind {
            IntentKind::SyncKey => {
                let Some(witness) = IntentWitness::from_sync_key(event) else {
                    return false;
                };
                self.platform_state
                    .ime
                    .write_sync_key(witness, new_val, tick_ms);
            }
            IntentKind::PhysicalImeKey => {
                let Some(witness) = IntentWitness::from_physical(event) else {
                    return false;
                };
                self.platform_state
                    .ime
                    .write_physical_key(witness, new_val, tick_ms);
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
                    self.apply_input_mode_correction(
                        new_mode,
                        crate::state::ime_event::InputModeApplyStrategy::UserTurnOnEisuReset,
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
                self.apply_input_mode_correction(
                    new_mode,
                    crate::state::ime_event::InputModeApplyStrategy::UserImeOnEisuReset,
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
            let imm_first =
                crate::ime_controller::ImeController::imm_cross_is_first_applicable(&view);
            if imm_first {
                // async 完了前から ImeModel を OFF に確定させる（ADR-098 決定5/6-a:
                // 旧コメント「楽観的 C」は実体（Confirmed）と食い違っていたため訂正。
                // 直前の `!effective_open()` 確認 + 直後の実 ImmCross apply を伴う
                // ため、belief laundering ではなく正当な pre-actuation write）。
                self.platform_state.ime.record_confirmed(false, tick_ms.0);
                // ADR-090 §2.A A-1（shadow）: 起案は spawn_local の**外**で行う
                // ——future の中では `with_app` 再入で `ImeStateHub` に届かない
                // （ADR-090 §4.2）。
                let order = self.issue_actuation_order(false, "shadow_toggle_off");
                let guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
                win32_async::spawn_local(async move {
                    // ADR-089 §2.3 Phase B: ImmCross を機構チェーンの**要素**と
                    // して実行する。`Failed` のときのフォールスルー（旧
                    // `apply_skipping_imm`）は `run_chain_async` が行う。
                    // 宛先の捕獲（ADR-086 INV-14）はこの経路では未移行のため
                    // `Untargeted` のまま（Phase C）。
                    let outcome = crate::runtime::open_chain::run_open_chain_async(
                        order,
                        crate::runtime::open_chain::ImmCrossOp::Untargeted,
                    )
                    .await;
                    // B+C(ts更新)+D(noop)+E
                    let _ = crate::with_app(|app| {
                        app.on_ime_apply_complete(
                            false,
                            outcome,
                            None,
                            crate::state::ime_event::OpenApplyReason::ShadowToggle,
                        );
                    });
                    drop(guard);
                });
            } else {
                let order = self.issue_actuation_order(false, "shadow_toggle_off_sync");
                let outcome = crate::ime_controller::ImeController::apply(order, &view);
                // B+C+D(noop)+E
                self.on_ime_apply_complete(
                    false,
                    outcome,
                    None,
                    crate::state::ime_event::OpenApplyReason::ShadowToggle,
                );
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
        if let Some((new_ime_on, origin)) = decision.find_ime_set_open_with_origin() {
            // IME-ON コンボ（既定: Ctrl+変換）は現在の IME 状態によらず SetOpen(true) を
            // 無条件で再発行する（`build_ime_set_open_decision` の「二重 enqueue 防止」
            // コメント参照）。このため handle_engine_set_open で belief を更新する前に
            // 「このイベント処理前は既に IME ON だったか」を控えておく必要がある
            // （2026-07-11 ユーザー要望: IME-ON コンボを IME ON 中に押したら、ひらがな +
            // ローマ字入力 + CapsLock OFF へリセットする。既に OFF→ON の場合は従来通り
            // 単純に ON にするだけで良い）。
            let was_open_before = self.platform_state.ime.effective_open();
            // 診断ログ用スナップショット (2026-08-05): handle_engine_set_open/
            // handle_engine_activation_sync 呼び出し前の last_intent を控えておく。
            // これらの呼び出しが last_intent を書き換えるため、後で「遷移直前は
            // 本当に明示意図があったか」を確認するには呼び出し前に読む必要がある。
            let last_intent_before = self.platform_state.ime.explicit_intent();
            self.platform.timer.kill(TIMER_IME_REFRESH);
            let generation = self.platform_state.ime.allocate_event_generation();
            let tick_ms = crate::state::TickMs(hook::current_tick_ms());
            // `origin` で belief 更新の経路を分ける（`SetOpenOrigin` の doc / 2026-08-04
            // 「IME OFF・Engine ON」再発対策参照）。
            // - ExplicitUserAction: IME/エンジン ON/OFF コンボ等、本物のユーザー操作。
            //   `last_intent` を設定してよい（`handle_engine_set_open`）。
            // - ActivationSync: `check_active_transition` が対称性のために自動発行した
            //   echo（`ctx.ime_on` の観測駆動な変化だけでも起こりうる）。`last_intent` を
            //   設定すると、この echo が「ユーザーの本物の意図」として固定化され、
            //   以後の drift correction が効かなくなる（IME OFF 直後に Engine が勝手に
            //   ON へ戻る再発の根本原因だった）。`handle_engine_activation_sync` で
            //   `desired_open` のみ更新する。
            let applied = match origin {
                awase::engine::SetOpenOrigin::ExplicitUserAction => {
                    let applied = self.platform_state.ime.handle_engine_set_open(
                        new_ime_on,
                        event.modifier_snapshot.ctrl,
                        focus_transition_was_pending,
                        generation,
                        tick_ms,
                    );
                    if applied {
                        // IntentStore（BUG-51 追補 v3）: IME/エンジン ON/OFF コンボ等、
                        // 本物のユーザー操作であることが origin から確定している場合のみ
                        // 記録する。`applied` ゲートは v1 の意味論（chord/focus-settle
                        // フィルタで belief 書き込み自体がスキップされた場合は記録しない）を
                        // そのまま保存する。記録を**この arm の中**に置くことで、
                        // `ActivationSync`（conv 由来の対称 echo）が偽の明示意図を
                        // 永続化する経路が構造的に存在しなくなる。
                        self.platform_state.ime.record_explicit_intent(
                            new_ime_on,
                            crate::state::ime_event::UserIntentSource::Command,
                            tick_ms,
                        );
                    }
                    applied
                }
                awase::engine::SetOpenOrigin::ActivationSync => {
                    self.platform_state.ime.handle_engine_activation_sync(
                        new_ime_on,
                        event.modifier_snapshot.ctrl,
                        focus_transition_was_pending,
                        generation,
                        tick_ms,
                    )
                }
            };
            // 2026-08-05: 実機再発報告（IME OFF 後 FocusChange 無しで Engine が勝手に
            // ON へ戻る）の切り分けのため debug → info に格上げし、遷移直前の
            // last_intent 内訳を追加した。この分岐は Engine の active/inactive が実際に
            // 遷移した時だけ通るため、毎 tick 出るログではない（低頻度）。
            log::info!(
                "IME control: preconditions.ime_on = {new_ime_on} (SetOpenRequest, origin={origin:?}), \
                 was_open_before={was_open_before} last_intent_before={last_intent_before:?} \
                 poll suspended{}",
                if applied { "" } else { " [chord barrier active → skipped]" }
            );

            // IME-ON コンボの既定値 `Ctrl+変換`（Shift/Alt/Win 無し）と一致する場合のみ
            // ひらがな＋ローマ字＋CapsLock OFF へのリセットを行う。
            //
            // 注意: `origin==ExplicitUserAction` は IME-ON コンボだけでなく
            // `Ctrl+Shift+変換`（EngineOn コンボ、`apply_active_transition` 経由）等の
            // 他の明示操作も含む（`SetOpenOrigin` の doc 参照）。ActivationSync の echo
            // を弾くのは `origin` チェックの役目だが、EngineOn コンボ等の
            // "ExplicitUserAction だが IME-ON コンボそのものではない" ケースを弾いて
            // いるのは `is_default_ime_on_combo` の VK/modifier 判定（特に `!shift`）
            // のほうであり、こちらは削除できない。`keys.ime_on` をカスタマイズした
            // 場合はこの判定も合わせて更新すること。
            let is_default_ime_on_combo = event.vk_code == crate::vk::VK_CONVERT
                && event.modifier_snapshot.ctrl
                && !event.modifier_snapshot.shift
                && !event.modifier_snapshot.alt
                && !event.modifier_snapshot.win;
            // BUG-50 (2026-08-05): 当初 `was_open_before`（belief）単独でこのリセット
            // を条件付けていたが、belief が drift で誤って false になっている
            // （IME は既にカタカナへ入っているのに belief は「まだ閉じている」と
            // 誤認している）ケースでリセットが発火せず、カタカナから永久に復旧
            // できないデッドロックになっていた（`docs/known-bugs.md` BUG-50）。
            // 当時は「実際に観測されたカタカナ」を追加条件にして凌いでいたが、
            // charset 軸の観測自体を 2026-08-17 ADR-094 で撤去したのに伴い、
            // `was_open_before` の判定も含めて撤去し、IME-ON コンボ押下では
            // 常にひらがなへ寄せる（この破壊的リセットは冪等——既にひらがな
            // なら実質no-op）。BUG-50 の originally-undetermined だった発生原因は
            // 追補（2026-08-17）で BUG-52 の機構と特定・修正済みであり、この
            // 無条件化はその機構への対症療法ではなく、charset 軸撤去の帰結。
            if applied
                && matches!(origin, awase::engine::SetOpenOrigin::ExplicitUserAction)
                && new_ime_on
                && is_default_ime_on_combo
            {
                Self::kp_reset_to_hiragana_romaji_capsoff(
                    self.platform.output.ime_mode_focus_gen.get(),
                );
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
                    self.apply_input_mode_correction(
                        new_mode,
                        crate::state::ime_event::InputModeApplyStrategy::PostSetOpenEisuReset,
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
    /// `focus_gen`（`Output::ime_mode_focus_gen`）は呼び出し元が起案時点で読んで渡す
    /// （ADR-086 §7-3: `ime.rs` は Runtime/Output の内部状態に依存しないため）。
    fn kp_reset_to_hiragana_romaji_capsoff(focus_gen: u32) {
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
        // （target を丸ごと置換するので、事前に現在値を読んでマスク計算してから渡す）。
        //
        // ADR-086 INV-14: capture はブロック先頭・他の await より前に置く
        // （opus レビュー指摘 2026-08-08、executor.rs/cold_warmup.rs で capture を
        // 後置していたバグの教訓）。read（get_ime_conv_for_target）と write
        // （set_ime_conv_for_target）の両方に同じ検証済み target を使い回すことで、
        // 「別窓の conv を読んで自窓に書く」不整合を避ける。
        //
        // スコープ注意: この経路は conv_mutation_allowed ゲートも
        // actuate_conv_mode の unconfirm() も通っていない（既存のまま）。
        // INV-14（ターゲット同一性）は満たすが INV-1（単一窓口経由）は
        // 未達のまま — ConvModeTarget に read-modify-write 用の variant が無く、
        // actuate_conv_mode 経由にすると conv_mutation_allowed 却下が新たに
        // 効いて挙動が変わってしまうため、今回は見送る。
        win32_async::spawn_local(async move {
            let Some(target) = crate::ime::ActuationTarget::capture(focus_gen).await else {
                log::debug!("[ime-on-combo] capture 失敗（フォーカス無し） → リセット中止");
                return;
            };
            let current = crate::ime::get_ime_conv_for_target(target, 50).await;
            let set_mask = crate::imm::IME_CMODE_NATIVE
                | crate::imm::IME_CMODE_FULLSHAPE
                | crate::imm::IME_CMODE_ROMAN;
            let clear_mask = crate::imm::IME_CMODE_KATAKANA;
            let mask_target = current.map_or(set_mask, |c| (c | set_mask) & !clear_mask);
            let outcome = crate::ime::set_ime_conv_for_target(target, Some(mask_target), || {
                crate::with_app(|runtime| runtime.platform.output.ime_mode_focus_gen.get())
                    .unwrap_or_else(|| focus_gen.wrapping_add(1))
            })
            .await;
            if !matches!(outcome, crate::ime::ActuationOutcome::Written) {
                log::warn!(
                    "[ime-on-combo] ひらがな＋ローマ字リセットの conv write に失敗: {outcome:?}"
                );
            }
        });
    }

    /// 左Shift単独タップによる「IME-ON 半角英数」持続トグル判定
    /// （BUG-15 撤去 + BUG-25 新機能、2026-07-11。チョード安全網は 2026-08-09 撤去、
    /// 詳細は known-bugs.md BUG-15 追補9参照）。
    ///
    /// # チョード安全網は撤去済み（2026-08-09）
    ///
    /// かつては MS-IME の（設定で無効化不可能な）「Shift 単独タップで英数モードに
    /// 切替える」誤検知対策として、Shift+文字キーのチョード（`.yab` Shift 面）を
    /// engine が consume する際にも無条件で conv を英数へ→かなへ書き戻していた
    /// （BUG-15）。しかし BUG-25 で ASCII 素通し経路（`shift_plane_halfwidth`）を
    /// 撤去して以降、Shift 面は `should_use_shift_plane`/`shift_face_reduce` が
    /// 常に `.yab` の値をそのまま Unicode 直接注入する（IME 経由の
    /// 素通しは発生しない）ため、この安全網はチョードの出力そのものには
    /// 一切必要なくなっていた。それにもかかわらず conv を一時的に英数へ倒す
    /// 副作用だけが残り、(a) LINE（Qt/ImmCross）でその窓に着弾した全角記号
    /// （`'！'` 等）が半角化される実害と、(b) BUG-58（チョードのたびに
    /// `OutputActiveGuard` と shift-conv-guard 復元が循環待ちして毎回 ~5 秒
    /// フリーズする）の直接の引き金という2つの実害を生んでいた。チョードに対する
    /// 先書き込みそのものを撤去したことで、どちらも構造的に発生しなくなる
    /// （BUG-58 は案E で既に緩和済みだったが、根本原因はこの先書き込み自体だった）。
    ///
    /// # 左Shift単独タップの持続トグル（維持）
    ///
    /// 左Shift の押下→解放の間に他の非注入物理キーが一切来なかった場合のみ
    /// 「単独タップ」と判定し、`half_width_alnum_toggle_active` を立てる
    /// （IME-ON 半角英数の持続トグルへ移行）。**conv=0x0000 の実書き込みは
    /// この確定した瞬間（`kp_shift_conv_guard_key_up`）に初めて行う** —
    /// 旧実装のように Shift down 時点で判別未確定のまま先書き込みはしない。
    /// もう一度単独タップしたら `kp_restore_kana_from_half_width` でかな入力へ
    /// 復元してトグルを解除する。右Shift単独タップはトグルの「緊急解除」として
    /// 働く（トグル非アクティブ時の右Shiftタップ・チョードは何もしない）。
    ///
    /// ADR-097 の親指+小指シフト複合面は、この関数が `kp_stage_execute` より前に
    /// 非 Shift の KeyDown で `left_shift_tap_candidate` を折る順序にも依存する。
    /// この順序を変える場合は、Shift+親指+文字が左Shift単独タップとして
    /// 誤判定されないことを再確認すること。
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

        // Ctrl/Alt/Win チョード（ショートカット）では判定自体を発動しない。
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
            // 既に conv=0x0000 のはず。何もしない。
            return;
        }

        // かな入力コンテキストのみ: IME ON・engine 有効・conv 書込権限。左Shift
        // 単独タップによる持続トグルは、この条件を満たさない限り
        // `kp_shift_conv_guard_key_up` 側でも確定させない（かつては conv の
        // 先書き込みをここで行っていたための早期 override クリアだったが、
        // 先書き込み自体を撤去したので pending を落とすだけで足りる）。
        if !self.platform_state.ime.effective_open()
            || !self.platform_state.ime.belief.is_japanese_ime()
            || !self.engine.is_user_enabled()
            || !self.platform.output.conv_mutation_allowed.get()
        {
            self.platform_state.gate.shift_conv_guard_pending = false;
        }

        // NOTE（2026-08-09、known-bugs.md BUG-15 追補9）: 以前はここで
        // 判別未確定のまま無条件に conv=0x0000（IME-ON 半角英数）を先書き込み
        // していた（Shift+文字キーのチョード全般への MS-IME 誤検知対策
        // 「安全網」、BUG-15/BUG-25）。BUG-25 で ASCII 素通し経路
        // （`shift_plane_halfwidth`）を撤去して以降、Shift 面のチョードは
        // `shift_face_reduce` が `.yab` の値をそのまま Unicode 直接注入する
        // だけになっており、半角英数モードへの実際の需要は無かった。それにも
        // かかわらずこの先書き込みが (a) LINE（Qt/ImmCross）で全角記号
        // （`'！'` 等）がその窓に着弾すると半角化される実害、(b) BUG-58
        // （チョードのたびに `OutputActiveGuard` と復元が循環待ちして
        // 毎回 ~5 秒フリーズする）の直接の引き金、という2つの実害を生んでいた
        // ため撤去した。左Shift単独タップによる持続トグル（BUG-25）の
        // conv=0x0000 書き込みは `kp_shift_conv_guard_key_up` の
        // 「本物の単独タップと確定した瞬間」に一本化した。
    }

    fn kp_shift_conv_guard_key_up(&mut self, event: &RawKeyEvent) {
        if !std::mem::take(&mut self.platform_state.gate.shift_conv_guard_pending) {
            return;
        }
        // GJI には entry 機構が無い（BUG-25 追補3）ため、左Shift単独タップでも
        // 持続トグルへは絶対に移行しない（移行すると engine が pass-through に
        // なり、生ローマ字キーが GJI 自身の未切替のひらがな変換エンジンへ
        // そのまま入ってかな入力が壊れる）。
        let toggle_entry_supported = crate::tsf::observer::tsf_obs().active_ime_kind()
            == crate::tsf::observer::ActiveImeKind::MicrosoftIme;

        let is_left_shift_tap = event.vk_code == crate::vk::VK_LSHIFT
            && std::mem::take(&mut self.platform_state.gate.left_shift_tap_candidate);
        self.platform_state.gate.left_shift_tap_candidate = false;

        if is_left_shift_tap
            && toggle_entry_supported
            && !self.platform_state.gate.half_width_alnum_toggle_active
        {
            // 本物の単独タップ、1回目 → 半角英数トグルへ移行。conv=0x0000 の
            // 実書き込みはここで初めて行う（2026-08-09、known-bugs.md BUG-15
            // 追補9: チョード安全網の先書き込み撤去に伴い、持続トグルの entry
            // write もここへ一本化した。旧実装は Shift down 時点で判別未確定の
            // まま先書きしていた）。
            self.platform_state.gate.half_width_alnum_toggle_active = true;
            log::info!(
                "[shift-conv-guard] 左Shift単独タップ → 半角英数トグルON (conv=0x0000 書き込み)"
            );
            let now_tick = crate::state::TickMs(hook::current_tick_ms());
            self.platform_state.ime.note_explicit_ime_action(now_tick);
            // ADR-084 P1/INV-1/INV-2: 書き込みと belief 無効化を
            // `Runtime::actuate_conv_mode` に集約（`runtime/conv_actuation.rs`）。
            let _ = self.actuate_conv_mode(
                crate::state::ConvModeTarget::HalfWidthAlnum,
                crate::state::ConvMutationReason::ShiftSoloTapCounter,
                now_tick,
            );
            // ADR-084（BUG-49 追補2）: confirm-then-transmit ゲート（BUG-13、
            // `Output::ms_ime_gate_defer`）の期限を、トグルON中は
            // `SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS` 分だけ延長する（詳細は
            // 旧 entry 実装のコメントを参照、known-bugs.md BUG-15 追補9）。
            self.platform.output.confirm_gate_deadline_override_ms.set(
                hook::current_tick_ms() + crate::tuning::SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS,
            );
            self.platform.output.bump_shift_conv_guard_gen();
            // 診断用: 送信直後に conv を読み取ってログに残す。
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
            self.apply_input_mode_correction(
                InputModeState::ObservedEisu,
                crate::state::ime_event::InputModeApplyStrategy::UserHalfWidthAlnumToggle,
                now_tick,
            );
            return;
        }

        if self.platform_state.gate.half_width_alnum_toggle_active {
            // 2回目の左Shiftタップ（トグルOFF）・右Shift（トグルの緊急解除）:
            // 復元を実行する。
            //
            // ADR-084（BUG-49 追補2、round-6 レビュー指摘）: entry でキャップ付き
            // に延長した confirm-then-transmit ゲートの期限を、ここ（hold 終了）
            // を起点とするフレッシュな猶予に差し替える。値の導出根拠は
            // `SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS` の doc コメント（tuning.rs）
            // 参照。MS-IME 限定（GJI には entry/持続トグル機構が無い）。
            if toggle_entry_supported {
                self.platform.output.confirm_gate_deadline_override_ms.set(
                    hook::current_tick_ms() + crate::tuning::SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS,
                );
            }
            self.kp_restore_kana_from_half_width(true);
        }

        // チョード（Shift+文字キー）でトグル非アクティブ: conv には一切
        // 触れていないため何もしない（2026-08-09、known-bugs.md BUG-15
        // 追補9でチョード安全網を撤去）。
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
        // ADR-084 INV-7（BUG-49 追補）: entry（kp_shift_conv_guard_key_down）は
        // MS-IME 限定でしか conv を書き込まない（GJI には entry 機構が無い、
        // BUG-25 追補2/3）。復元側だけ IME 種別を問わず無条件に実行するのは
        // 非対称であり、GJI に対しては「そもそも書いていないものを復元する」
        // 無意味な副作用（VK_DBE_HIRAGANA 注入・IMC write リトライ）でしかない。
        // 以下の実際の OS 書き込みは entry と対称に MS-IME 限定にする
        // （belief 更新の `apply_input_mode_correction` は IME 種別を問わず
        // 必要なため、この分岐の外で無条件に行う）。
        let active_ime_kind = crate::tsf::observer::tsf_obs().active_ime_kind();
        if active_ime_kind == crate::tsf::observer::ActiveImeKind::MicrosoftIme {
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
            // ADR-086 INV-13 の例外（確定、§4 INV-13/§5 Phase 3 item 3 参照）:
            // 以下の VK_DBE_HIRAGANA 注入は SendInput ベースであり、IMC write と
            // 違って宛先 hwnd を指定できない（SendInput は配送時点のフォーカス先へ
            // 届く）。`ActuationTarget` のターゲット同一性検証（INV-14）は
            // この経路には構造的に適用できないと判断済み——`SendInput` を使う
            // 書き込み全般に共通する構造的な制約であり、この箇所固有の先送りではない。
            // Win/Alt が押下中は VK_DBE_HIRAGANA 注入自体をスキップする。
            //
            // Win: `tsf/send.rs::send_eager_warmup_vk_pair` と同じ理由
            // （Win を押したまま送ると Win+F2 として届き、Win↑ 時にスタート
            // メニューが開く）。あちらは唯一の判定点 `hook::win_key_held()`
            // を使っており、ここも同じ関数を使うことで判定基準を統一する。
            //
            // Alt: 実機診断（2026-08-17、専用プローブツールでの検証）で、
            // Alt 押下中に合成 VK_DBE_HIRAGANA を送ると MS-IME の「Alt+かな」
            // ローマ字⇔JISかな直接入力切替ショートカット（BUG-61/62 で
            // 物理キー押下について確認済みの機構）と同様に解釈され、実際に
            // JIS かな直接入力へ切り替わることを確認した（`hook.rs` の
            // 既存 Alt+かなガードは `VK_KANA`/`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`
            // のみを対象とし、自己注入キー〈`is_self_injected`〉は無条件に
            // 素通しするため、awase 自身のこの注入は対象外だった）。
            //
            // Shift のように synthetic な modifier-up を同一バッチへ前置する
            // 案も検討したが、Alt/Win は単独タップで OS のメニュー系機能
            // （`SC_KEYMENU`/スタートメニュー）を起動する特殊な扱いを受けており、
            // 実機未検証のままそれを回避する細工を追加するリスクを避け、
            // 検証済みの Win ガードと同じ「スキップ」方式に統一した。
            // スキップした場合、直後の IMC write リトライ（保険、上記コメント
            // 参照）だけが残るため実モードの復元は保証されないが、これは
            // Win ガード導入時から許容されている既存のトレードオフと同じ。
            let blocking_modifier_held = hook::win_key_held() || hook::alt_key_held();
            if self.platform_state.ime.effective_open() && !blocking_modifier_held {
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
                log::debug!(
                    "[shift-conv-guard] VK_DBE_HIRAGANA (scan 付き) 注入 → ひらがなモード復元"
                );
            } else if blocking_modifier_held {
                log::debug!(
                    "[shift-conv-guard] Win/Alt 押下中のため VK_DBE_HIRAGANA 注入をスキップ \
                     (IMC write のみ。Alt+かな誤発火/Win+F2のスタートメニュー起動を防ぐ)"
                );
            } else {
                log::debug!(
                    "[shift-conv-guard] effective_open()=false のため VK_DBE_HIRAGANA 注入をスキップ \
                     (IMC write のみ、BUG-15 追補7の教訓)"
                );
            }
            // charset 軸（ひらがな/カタカナ等）の追跡を 2026-08-17 ADR-094 で撤去した
            // のに伴い、切替前がカタカナだったかに関わらず常にローマ字ひらがなへ復元する
            // （かつては KATAKANA ビット込みで復元していたが、ADR-091 決定3 §D3.1 の
            // 「charset 軸は追跡しない」を徹底する）。
            let target_conv = crate::imm::IME_CMODE_NATIVE
                | crate::imm::IME_CMODE_FULLSHAPE
                | crate::imm::IME_CMODE_ROMAN;
            log::info!("[shift-conv-guard] かな入力へ復元 (target=0x{target_conv:08X})");

            // pass-5 レビュー指摘（blocking）: このリトライタスクは detached
            // (`spawn_local`) で、完了は Shift タップ間隔より遅れうる。起動時点の
            // 世代を捕獲し、以後の全 override 書き込み（延長・クリア双方）の
            // 前提条件にする。捕獲後に新しい hold が始まる（=
            // `shift_conv_guard_gen` が進む）と、このタスクは即座に
            // 自分がもう override の所有者でないと分かり、上書きも
            // 無関係な conv write も一切行わずに終了する。
            let owner_gen = self.platform.output.shift_conv_guard_gen.get();
            // ADR-086 INV-14: hwnd は起案時点（＝今、owner_gen 捕獲と同一の同期
            // 区間）で1回だけ capture し、全リトライ試行で使い回す（opus
            // アドバーサリアルレビュー 2026-08-08: 毎試行 capture は capture/verify
            // が数 ms 差のライブクエリ2連発になりほぼ確実に一致するため INV-14 が
            // 事実上 no-op 化する。1回の capture + 各試行での verify_still_current
            // が正しい設計）。`ime_mode_focus_gen` は `shift_conv_guard_gen` と
            // 同一イベント（`on_ime_mode_focus_changed`）で同時に bump されるため、
            // フォーカス変更は既存の `still_owner` チェックで先に検知される
            // （時間軸のフェンスは既存。ADR-086 が足すのは hwnd＝空間軸の検証）。
            let focus_gen = self.platform.output.ime_mode_focus_gen.get();

            win32_async::spawn_local(async move {
                // MS-IME の誤切替は shift up の後いつ来るか不定（実測: 478ms 後の
                // idle-conv-check で観測 = 上限 478ms）。冪等な IMC write を
                // 160ms 間隔で最大 4 回（0/160/320/480ms、実測上限をカバー）打ち、
                // NATIVE が確認できた時点で打ち切る。
                const RETRY_INTERVAL_MS: u32 = 160;
                const MAX_TRIES: u32 = 4;
                let Some(target) = crate::ime::ActuationTarget::capture(focus_gen).await else {
                    log::debug!("[shift-conv-guard] capture 失敗（フォーカス無し） → 復元中止");
                    return;
                };
                for attempt in 0..MAX_TRIES {
                    // ADR-084（BUG-49 追補2、Opus レビュー指摘1・pass-5 指摘）:
                    // リトライが続いている限り confirm-gate の猶予を「今から
                    // SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS」へ押し出し続ける。
                    // key_up 冒頭の一度きりの設定だけでは、このループ自体が
                    // 実測 ~478ms・設計上最大 ~640ms かかりうることに追従できず、
                    // 復元完了前に confirm-gate の期限が切れて BUG-49 が
                    // release 側で再発しうる（実際に一度この形で再発を確認済み）。
                    // 世代が捕獲時点と一致する場合のみ書く。不一致なら別の hold
                    // が既に開始しているということであり、このループは即座に
                    // 打ち切る（override 書き込みはおろか、以後の IMC write も
                    // 行わない — 対象ウィンドウ/hold が既に自分のものではない）。
                    let extend_result = crate::with_app(|runtime| {
                        runtime.platform.output.extend_confirm_gate_override(
                            owner_gen,
                            hook::current_tick_ms()
                                + crate::tuning::SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS,
                        )
                    });
                    let still_owner = extend_result.unwrap_or_else(|| {
                        // `with_app` の再入は起きないはずだが（この呼び出しは
                        // await 前・他の with_app のネスト無し）、万一 None が
                        // 返った場合に沈黙で猶予が更新されないと BUG-49 が
                        // 無警告で再発するため、痕跡を残す。
                        log::warn!(
                            "[shift-conv-guard] 復元リトライ #{attempt}: with_app 再入 \
                             (None) により override 更新をスキップ"
                        );
                        false
                    });
                    if !still_owner {
                        log::debug!(
                            "[shift-conv-guard] 復元リトライ #{attempt}: 新しい hold が \
                             開始された (gen 不一致) ため中断"
                        );
                        return;
                    }
                    let outcome =
                        crate::ime::set_ime_conv_for_target(target, Some(target_conv), || {
                            crate::with_app(|runtime| {
                                runtime.platform.output.ime_mode_focus_gen.get()
                            })
                            .unwrap_or_else(|| focus_gen.wrapping_add(1))
                        })
                        .await;
                    match outcome {
                        crate::ime::ActuationOutcome::Written => {}
                        crate::ime::ActuationOutcome::Failed => {
                            log::warn!("[shift-conv-guard] conv 復元 write #{attempt} 失敗");
                        }
                        crate::ime::ActuationOutcome::Aborted(reason) => {
                            // GenStale はこの直前の still_owner チェックとほぼ同じ条件
                            // （ime_mode_focus_gen と shift_conv_guard_gen は同一イベントで
                            // 同時に bump される）だが、念のための多重防御として残す。
                            // TargetMoved は capture 後にフォーカスだけが動いた
                            // （gen 更新が伴わない）ケースの検知。どちらも同じ
                            // capture 済み target のまま次の試行を続ける（write は冪等）。
                            log::warn!(
                                "[shift-conv-guard] conv 復元 write #{attempt}: Aborted({reason:?})"
                            );
                        }
                    }
                    win32_async::sleep_ms(RETRY_INTERVAL_MS).await;
                    // opus レビュー指摘（2026-08-08）: write は capture 済み target に
                    // 固定したのに、打ち切り判定の read だけライブクエリのままだと
                    // write 先と read 先が別ウィンドウになりうる（write が Aborted
                    // でフォーカスが別窓 B へ移っていた場合、ライブ read は B の
                    // conv を読んで「NATIVE 確認 → 復元完了」と誤判定し、A の復元を
                    // 打ち切ってしまう）。read も同じ target（write と同一 hwnd）を
                    // 使う。
                    let conv = crate::ime::get_ime_conv_for_target(target, 10).await;
                    if let Some(c) = conv {
                        if c & crate::imm::IME_CMODE_NATIVE != 0 {
                            log::debug!(
                                "[shift-conv-guard] conv=0x{c:08X} NATIVE 確認 (#{attempt}) → 復元完了"
                            );
                            // 復元完了。confirm-gate は通常どおり
                            // `is_native_ready()` で即座に解決するため override は
                            // もう不要 — 次回無関係な hold まで残らないよう戻す
                            // （ただし自分がまだ所有者の場合のみ。既に次の hold が
                            // 始まっていればそちらの override を壊してはならない）。
                            let _ = crate::with_app(|runtime| {
                                runtime
                                    .platform
                                    .output
                                    .clear_confirm_gate_override(owner_gen);
                            });
                            return;
                        }
                    }
                }
                log::warn!("[shift-conv-guard] conv 復元 {MAX_TRIES} 回で NATIVE 未確認のまま終了");
                // 復元が最終的に失敗した場合は override を解除し、通常の安全弁
                // （IMC 未確認なら give-up latch）へ戻す。延長したまま放置すると
                // 本当に IMC が読めない環境でも give-up が永久に立たなくなる。
                // 世代が一致する場合のみ（次の hold の override を壊さない）。
                let _ = crate::with_app(|runtime| {
                    runtime
                        .platform
                        .output
                        .clear_confirm_gate_override(owner_gen);
                });
            });
        } else {
            log::debug!(
                "[shift-conv-guard] GJI経路: entry 機構が無いため復元 write もスキップ \
                 (ADR-084 INV-7、entry/restore の IME 種別ゲートを対称化)"
            );
        }

        self.apply_input_mode_correction(
            InputModeState::AssumedRomaji {
                reason: awase::engine::AssumedReason::UserHalfWidthAlnumToggleOff,
            },
            crate::state::ime_event::InputModeApplyStrategy::UserHalfWidthAlnumToggle,
            now_tick,
        );
    }

    /// Effects の実行（フックからキューに委譲）
    ///
    /// `profile`/`physical`（物理 IME キーを OS に届けるかの配送判断、Decision とは
    /// 独立）は呼び出し元（`kp_run_inner`）が KeyInput journal 記録と共有するため
    /// 既に確定済みの値を受け取る（BUG-90 調査: 以前はここで独立に再計算しており
    /// journal 記録値との理論上の乖離窓があった）。判断ロジック自体は
    /// `PhysicalKeyDisposition::plan` のドキュメントコメント参照:
    /// - Imm32Unavailable (Chrome/Edge) / TsfNative (WezTerm/Windows Terminal) で GJI/MS-IME
    ///   が actuate する場合: KeyDown は shadow_toggle 発火時のみ、KeyUp は常に Suppress。
    ///   awase 自身が apply-ime で VK_IME_ON/OFF 等を SendInput 済みなので物理キーを
    ///   届けると二重制御になる（TsfNative + GJI の実例: BUG-46）。
    /// - ImmCross (LINE/Qt): Down/Up 共に Suppress。set_ime_open_cross_process で IME 制御済み。
    fn kp_stage_execute(
        &mut self,
        decision: awase::engine::Decision,
        event: &RawKeyEvent,
        profile: crate::focus::class_names::AppImeProfile,
        physical: crate::runtime::PhysicalKeyDisposition,
    ) -> CallbackResult {
        if let Some(reason) = physical.suppress_reason(event, profile) {
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
            // ADR-098 決定1-b: 生値ではなく warmup_ime_on()（`applied ?? belief`）。
            let warmup_ime_on = self.platform_state.ime.warmup_ime_on();
            self.platform.composition_native_f2_down(warmup_ime_on);
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
    /// `FocusProbeOpenStatus::Read` から得た値だけを受け取る（ADR-106 決定2）。
    /// belief 由来の `bool`（`effective_open()` 等）は型が合わず渡せない
    /// ——`ObservedOpenValue` は `FocusProbeOpenStatus::classify` の `Read` 分岐
    /// でしか構築できない。
    fn apply_effective_ime(
        &mut self,
        effective: crate::state::observation_store::ObservedOpenValue,
        tick_ms: crate::state::TickMs,
        accepted: crate::state::probe_admission::AcceptedObservation,
    ) {
        if effective.get() {
            self.platform_state.ime.reset_detect_state();
        }
        self.platform_state
            .ime
            .write_focus_probe(effective, tick_ms, accepted);
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
        // ADR-106 決定2: 観測できない状況（TsfNative/Imm32Unavailable、または probe 自体が
        // ime_on=None を返した場合）を bool へ潰さず型で運ぶ。`probe_ime_on` はログ表示
        // 専用の派生値であり、書き込み経路には使わない（下の match が Read/NotObservable
        // で完全に分岐する）。
        let status = FocusProbeOpenStatus::classify(probe.ime_on, current_profile);
        let probe_ime_on: Option<bool> = match status {
            FocusProbeOpenStatus::Read(v) => Some(v.get()),
            FocusProbeOpenStatus::NotObservable(_) => None,
        };
        if probe.ime_on.is_some() && probe_ime_on.is_none() {
            log::debug!(
                "FocusProbe: profile={current_profile:?} は IMM32 open status 非対応のため \
                 probe.ime_on={:?} を破棄",
                probe.ime_on
            );
        }

        // TsfNative/Imm32Unavailable では open status を信用しない。
        let used_shadow_fallback =
            matches!(status, FocusProbeOpenStatus::NotObservable(_)) && probe.is_japanese_ime;

        let suppressed_reason: Option<&'static str> = match status {
            FocusProbeOpenStatus::Read(on) => {
                let effective = on.effective(probe.is_japanese_ime);
                if !effective.get() && signals.any() {
                    Some(signals.primary_reason())
                } else {
                    self.apply_effective_ime(effective, now_tick_ms, accepted);
                    None
                }
            }
            FocusProbeOpenStatus::NotObservable(_profile) => {
                // TsfNative/Imm32Unavailable: IMM32 非対応のため観測できない。
                // ADR-106 決定2（BUG-92）: 旧実装はここで shadow の apply 値
                // （belief 由来）を代替観測として focus_probe スロットに書き込んで
                // いたが、これは「API を叩いていない値を観測として記録する」
                // laundering であり、この観測は定義上 desired と一致するため
                // drift correction が構造的に一度も発火しなくなっていた
                // （BUG-33 で確定済み）。観測の記録自体は撤去し、guard 解除の
                // 副作用（旧 `apply_effective_ime(shadow_on)` が `shadow_on==true`
                // のとき呼んでいた `reset_detect_state`）だけを独立して維持する。
                if probe.is_japanese_ime && shadow_on {
                    self.platform_state.ime.reset_detect_state();
                }
                None
            }
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
                // BUG-34 横展開 C: この読み取りは apply_focus_probe 内で完全に同期
                // （直前・直後に await 点が無い）なため、conv_mutation_seq のような
                // spawn-to-apply 型の fence を足す意味はない（比較対象となる
                // 「spawn 時と apply 時」の間に何も起こり得ない）。一方で
                // SendMessageTimeoutW ベースの同期呼び出しであることは変わらないため、
                // Step0-c の SendHealth ブレーカで直近 slow 判定後は発行を見送る。
                //
                // 【完全な修正を見送った理由】この読み取りを probe の await と
                // 並行実行（join）に切り出せば、より応答性の良い設計にできる
                // （round-2 premortem の C 是正案）。ただしその場合は
                // ImmLikeTicket/focus-epoch 照合を新タスクにも引き継がせる必要があり
                // （既存の epoch fence を落とす退行を避けるため）、実機ソーク無しに
                // ここだけ先走ると新しい race を作り込む恐れがある。E-prep
                // （open_chain.rs::fallback_write）と同じ理由で、今回は見送る。
                if crate::send_health::blocking_allowed(hook::current_tick_ms()) {
                    // SAFETY: メッセージループスレッドから呼ぶ。10ms タイムアウト。
                    if let Some(conv) =
                        unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(10) }
                    {
                        // ADR-106 決定4: この読み取りは完全に同期（await 点無し）なので、
                        // 観測の focus_epoch/hwnd（= FocusProbe admission 済みの
                        // accepted.focus_epoch/accepted.hwnd、ADR-106 決定3）は「現在」と
                        // 常に一致する——将来 focus-conv-check を非同期化する際も、
                        // observe() の monotonic guard がそのまま効くようにするための
                        // 前提工事。
                        self.platform.output.conv_mode.observe(
                            crate::state::conv_mode::ConvObservation {
                                mode: awase::engine::ConvMode::from_u32(conv),
                                read_at: now_tick_ms,
                                focus_epoch: accepted.focus_epoch,
                                hwnd: accepted.hwnd,
                                source: crate::state::conv_mode::ConvReadSource::FocusCheck,
                            },
                            accepted.focus_epoch,
                            accepted.hwnd,
                        );
                        self.platform_state.ime.set_prev_conversion_mode(Some(conv));
                        log::debug!(
                            "[focus-conv-check] TsfNative: conv=0x{conv:08X} 読み取り（belief 更新なし、\
                             フォーカス変更直後の値はユーザー意図の signal ではないため idle-conv-check に一任）"
                        );
                    }
                } else {
                    // warn: バグ報告に添付する awase.log（info レベル既定）に残す
                    // ため。BUG-34 横展開の切り分け材料。
                    log::warn!("[focus-conv-check] SendHealth degrade で conv 読み取りを見送り");
                }
            }
        }

        // ImmCross アプリ（Qt/LINE 等）: FocusProbe は top-level hwnd の IMC を読むが、
        // GJI 使用時は child hwnd と IME 状態が異なる場合がある（Qt の IME コンテキスト分割）。
        // read_ime_state_full_async で child hwnd を正確に読み、High confidence 観測として記録する。
        // これにより FocusProbe (Low) が誤って false を返しても derive_any() で正しく上書きされる。
        //
        // エポック/hwnd 照合: FocusProbe の admit() 済み値を引き継ぐ（ADR-106 決定3）。
        // apply_focus_probe の呼び出し前に admission を通過しているため
        // accepted.focus_epoch/accepted.hwnd は現在の値と等しいことが保証済み。
        if matches!(
            self.platform.current_app_profile(),
            crate::focus::classify::AppImeProfile::Standard,
        ) && probe.is_japanese_ime
        {
            let ticket = crate::state::probe_admission::ImmLikeTicket {
                focus_epoch: accepted.focus_epoch,
                hwnd: accepted.hwnd,
            };
            win32_async::spawn_local(async move {
                // SAFETY: read_ime_state_full_async は offload 済み — メインスレッド不要。
                let snap = crate::ime::read_ime_state_full_async().await;
                if let Some(open) = snap.ime_on {
                    let _ = crate::with_app(|app| {
                        crate::state::probe_admission::admit_epoch_in_app(
                            app,
                            ticket,
                            "[ImmCrossProbe] epoch rejected (focus changed since probe spawn)",
                            |app, inner_accepted| {
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
                                let update =
                                    crate::observer::ime_observer::classify_fetched_snapshot(
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
                            },
                        );
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
                        // opus レビュー指摘（2026-08-08）: `set_ime_romaji_mode_async`
                        // （ライブクエリ版、ADR-086 削除対象の
                        // `set_ime_romaji_mode_with_target_async` と同じ危険を持つ）
                        // への呼び出しが未移行のまま残っていた。focus_gen も
                        // should_restore と同じ with_app 呼び出しでまとめて読み、
                        // ネストした spawn_local の**先頭**で capture する
                        // （この外側ブロックは probe 読み取りが最初の await のため、
                        // capture をここに直接置くと「ブロック先頭で capture」の
                        // 規律から外れてしまう）。
                        let (should_restore, focus_gen) = crate::with_app(|app| {
                            let ime = &app.platform_state.ime;
                            let should_restore = ime.effective_open()
                                && !matches!(ime.input_mode(), InputModeState::ObservedKana);
                            (should_restore, app.platform.output.ime_mode_focus_gen.get())
                        })
                        .unwrap_or((false, 0));
                        if should_restore {
                            log::debug!(
                                "[ImmCrossProbe] kana mode (conv=0x{conv:08X}) + IME ON \
                                 → romaji 修正 (MS-IME かなモード修正)"
                            );
                            win32_async::spawn_local(async move {
                                let Some(target) =
                                    crate::ime::ActuationTarget::capture(focus_gen).await
                                else {
                                    log::debug!(
                                        "[ImmCrossProbe] romaji 修正: capture 失敗（フォーカス無し）"
                                    );
                                    return;
                                };
                                let outcome =
                                    crate::ime::set_ime_conv_for_target(target, None, || {
                                        crate::with_app(|runtime| {
                                            runtime.platform.output.ime_mode_focus_gen.get()
                                        })
                                        .unwrap_or_else(|| focus_gen.wrapping_add(1))
                                    })
                                    .await;
                                if !matches!(outcome, crate::ime::ActuationOutcome::Written) {
                                    log::warn!("[ImmCrossProbe] romaji 修正に失敗: {outcome:?}");
                                }
                            });
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
    fn focus_probe_open_status_is_not_observable_for_imm32_unavailable() {
        assert!(matches!(
            FocusProbeOpenStatus::classify(Some(false), AppImeProfile::Imm32Unavailable),
            FocusProbeOpenStatus::NotObservable(AppImeProfile::Imm32Unavailable)
        ));
    }

    #[test]
    fn focus_probe_open_status_is_not_observable_for_tsf_native() {
        assert!(matches!(
            FocusProbeOpenStatus::classify(Some(false), AppImeProfile::TsfNative),
            FocusProbeOpenStatus::NotObservable(AppImeProfile::TsfNative)
        ));
    }

    #[test]
    fn focus_probe_open_status_is_read_for_standard() {
        let status = FocusProbeOpenStatus::classify(Some(false), AppImeProfile::Standard);
        let FocusProbeOpenStatus::Read(value) = status else {
            panic!("expected Read, got {status:?}");
        };
        assert!(!value.get());
    }

    #[test]
    fn focus_probe_open_status_is_not_observable_when_probe_returns_none_even_for_standard() {
        // Standard は can_read_imm32_open_status()==true だが、probe 自体が
        // ime_on=None を返す（未フォーカス等）場合は NotObservable になる。
        assert!(matches!(
            FocusProbeOpenStatus::classify(None, AppImeProfile::Standard),
            FocusProbeOpenStatus::NotObservable(AppImeProfile::Standard)
        ));
    }
}
