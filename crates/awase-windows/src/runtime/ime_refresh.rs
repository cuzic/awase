#![allow(unsafe_code)] // Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
use awase::engine::{EngineCommand, InputModeState, KanaLockHysteresis};

use super::ime_actuation::Actuation;
use super::Runtime;
use crate::state::drift_plan::{
    decide_drift_plan, resolve_actuation, DriftFacts, DriftPlan, DriftStep, GiveUpPark, SendPath,
};
use crate::state::ime_actuation::FeedbackPolicy;
use crate::state::ime_read_strategy::{
    decide_read_strategy, is_typing, ImeReadStrategy, ReadReason, ReadStrategyFacts,
};

// ── IoMode ──

/// IME リフレッシュパイプラインの入出力モード。
///
/// - `Sync`: 同期モード。`detect_and_update_focus` + `poll_and_classify_ime` を直接呼ぶ。
/// - `Prefetched`: pre-fetch 済みモード。`apply_focus_probe_result` + `classify_fetched_snapshot` を使う。
enum IoMode<'m> {
    Sync,
    Prefetched {
        focus: Option<crate::focus::probe::FocusSnapshot>,
        ime: &'m crate::ime::ImeSnapshot,
    },
}

// ── FocusInfo ──

/// ir_stage_focus() の戻り値: フォーカス検出結果
struct FocusInfo {
    focus_changed: bool,
    skip_imm_query: bool,
}

// ── IME リフレッシュ（impl Runtime） ──

impl Runtime {
    pub(super) fn run_ime_refresh(&mut self) {
        self.ir_execute(IoMode::Sync);
    }

    /// pre-fetch 済みデータを使ってパイプラインを実行（blocking なし）。
    /// spawn_local タスクから呼ぶ。
    pub(super) fn run_ime_refresh_with_prefetched(
        &mut self,
        focus_probe: Option<crate::focus::probe::FocusSnapshot>,
        ime_snap: &crate::ime::ImeSnapshot,
    ) {
        self.ir_execute(IoMode::Prefetched {
            focus: focus_probe,
            ime: ime_snap,
        });
    }

    fn ir_execute(&mut self, mode: IoMode<'_>) {
        let (focus_probe, ime_snap) = match mode {
            IoMode::Sync => (None, None),
            IoMode::Prefetched { focus, ime } => (Some(focus), Some(ime)),
        };
        let focus = self.ir_stage_focus(focus_probe);

        // `disable_apps`（既定 mstsc.exe）にマッチするアプリへフォーカス中は、
        // ここで IME リフレッシュを打ち切る（BUG-78 対策、ユーザー判断により
        // 例外なく無効化する）。observe/notify/drift correction/warmup/probe が
        // 全停止し、無効化先アプリへ VK_KANJI 等の IME 制御キーが送られなくなる。
        // `ir_stage_focus` は必ず先に呼ぶ — フォーカス検出自体を止めると、
        // 無効アプリからの離脱を検知できなくなる。
        if self.platform_state.focus.app_disabled {
            return;
        }

        let strategy = self.ir_stage_strategy(&focus);
        self.ir_stage_observe(&focus, strategy, ime_snap);
        self.ir_stage_notify();
    }

    // ── Stage 1: フォーカス検出 ──
    //
    // Phase 1: フォーカス先の検出・分類
    // Phase 2.5: IMM ブリッジ非対応クラスの判定（Phase 2 の前に実行する必要あり）
    // Phase 2: プロセス変更時は Engine に FocusChanged（flush あり）

    #[expect(clippy::option_option)]
    fn ir_stage_focus(
        &mut self,
        focus_probe: Option<Option<crate::focus::probe::FocusSnapshot>>,
    ) -> FocusInfo {
        let focus_changed = match focus_probe {
            None => unsafe { self.detect_and_update_focus() },
            Some(probe) => self.apply_focus_probe_result(probe),
        };

        // Phase 2.5: IMM ブリッジ非対応クラスの判定
        //
        // Chrome / UWP / Electron 等はクロスプロセス IMM 問い合わせ（WM_IME_CONTROL）が
        // 動作しないか、無期限ブロックする恐れがある。既知のクラス名なら事前にスキップし、
        // シャドウ状態（hook から追跡）のみで IME 状態を管理する。
        //
        // FocusChanged で build_ctx() が呼ばれる際、input_mode が stale な ObservedKana だと
        // engine が inactive になってしまうため、先に補正する。
        let skip_imm_query = self.ir_resolve_skip_imm_query();

        // Phase 2: プロセス変更時は Engine に FocusChanged（flush あり）
        if focus_changed {
            self.ir_notify_focus_changed(skip_imm_query);
            // かな入力ロック検知は romaji VK 送信直前にのみサンプリングするため
            // （runtime/key_pipeline.rs::kp_stage_kana_lock_warn）、フォーカスが
            // 別アプリへ移ると新たな観測が発生しなくなる。フォーカス変更のたびに
            // ヒステリシスとトレイ表示をリセットし、切り替え先アプリで新たに
            // 検知し直せるようにする（issue #137 4周目のレビューで指摘: リセット
            // 手段が engine 無効化時のみで、フォーカス変更後に警告が固着したまま
            // 二度と晴れないケースがあった）。
            self.kana_lock_hysteresis = KanaLockHysteresis::new();
            self.drift_giveup_notified_this_focus = false;
            self.drift_giveup_started_at = None;
            self.platform.tray.set_kana_lock_warned(false);
        }

        FocusInfo {
            focus_changed,
            skip_imm_query,
        }
    }

    // ── Stage 2: 読み取り方針の決定 ──

    fn ir_stage_strategy(&mut self, focus: &FocusInfo) -> ImeReadStrategy {
        self.ir_decide_read_strategy(focus.skip_imm_query)
    }

    // ── Stage 3: IME 状態の観測 ──
    //
    // Phase 3: IME 状態の再取得
    // Phase 3.1: IMM 能力の学習
    // Phase 3.5: 未知 Imm32Unavailable アプリ向け一時 force-ON（初回ブートストラップ）
    // Phase 3.7: 診断スナップショット（フォーカス変更後）

    fn ir_stage_observe(
        &mut self,
        focus: &FocusInfo,
        strategy: ImeReadStrategy,
        ime_snap: Option<&crate::ime::ImeSnapshot>,
    ) {
        tracing::debug!(
            "[stage-observe] strategy={:?} belief_on={} explicit_intent={:?}",
            strategy,
            self.platform_state.ime.effective_open(),
            self.platform_state.ime.explicit_intent(),
        );
        // ADR-205: 打鍵中（SkipTyping）でも、prefetch 済みの開閉の読みを外部変化の監視窓に照合する（追加 I/O なし）。
        self.ir_follow_external_change(ime_snap);
        // ADR-188: 物理のモードキー通過／FSM 再送出の直接観測の窓の中の読みを belief と照合して追随する。
        self.ir_follow_direct_mode_key_read(ime_snap);
        match strategy {
            ImeReadStrategy::SkipTyping => {}
            ImeReadStrategy::Blacklist => {
                tracing::debug!("Skipping IMM query for known-broken class (shadow state SSOT)");
                // GJI I/O 観測は active IME が GJI のときに限定する。MS-IME 使用中も
                // GJI Converter プロセスは常駐しており、そのバックグラウンド I/O を
                // 根拠に observer_poll を書くと無関係な belief 汚染になる。
                if crate::tsf::observer::tsf_obs().active_ime_kind()
                    == crate::tsf::observer::ActiveImeKind::GoogleJapaneseInput
                {
                    let obs = crate::observer::gji_observer::observe_gji_after_focus(
                        // 外部変化へ追随した直後は、閉じる前の GJI I/O 推測が追随結果を上書きしないよう柵を進める。
                        self.platform_state
                            .focus
                            .last_focus_change_ms
                            .max(self.platform_state.ime.last_external_change_ms()),
                        self.platform_state.ime.input_mode(),
                    );
                    tracing::debug!(
                        "[stage-observe] observer_poll={:?}",
                        obs.observer_poll_value
                    );
                    if let Some(v) = obs.observer_poll_value {
                        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
                        let accepted = crate::state::probe_admission::AcceptedObservation::for_sync(
                            self.focus_fence(),
                        );
                        self.platform_state
                            .ime
                            .write_observer_poll(v, tick_ms, accepted);
                    }
                    // stale ObservedEisu の矛盾証拠（GJI が変換 I/O 中 = 英数ではない）。
                    // Blacklist では他に input_mode を訂正する観測経路がないため、
                    // これが唯一のユーザー操作不要の自己回復経路になる
                    // （state/eisu_recovery.rs の経路×救済対応表を参照）。
                    if let Some(mode) = obs.input_mode_correction {
                        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
                        tracing::info!(
                            "[stage-observe] GJI I/O 中に belief=ObservedEisu → AssumedRomaji \
                             訂正 (GjiIoInference)"
                        );
                        self.platform_state.ime.dispatch_event(
                            crate::state::ime_event::ImeEvent::InputModeObserved {
                                mode,
                                source: crate::state::ime_event::ObservationSource::GjiIoInference,
                                confidence: crate::state::ime_event::ObservationConfidence::Medium,
                            },
                            tick_ms,
                        );
                    }
                } else {
                    tracing::debug!("[stage-observe] GJI observe skipped (active IME is not GJI)");
                }
            }
            ImeReadStrategy::OsPoll => {
                let miss_before = self.platform_state.ime.detect_miss_count();
                let got_observation = self.ir_poll_and_learn(miss_before, ime_snap);
                let now = crate::hook::current_tick_ms();
                let observed = got_observation
                    && crate::state::imm_evidence::poll_counted_no_new_miss(
                        miss_before,
                        self.platform_state.ime.detect_miss_count(),
                    );
                // 直前の読み取りの成否（時間切れも失敗）。通過マークの読み直し間隔の判定に使う。
                self.last_ime_read_ok = observed;
                if observed
                    && self
                        .platform_state
                        .ime
                        .invalidate_intents_if_mode_key_pass_live(now, crate::state::TickMs(now))
                {
                    tracing::info!(
                        "[mode-key-follow] observation arrived after mode key pass: intents invalidated"
                    );
                    // 最初の観測は GJI がキーを処理する前の古い状態を読むことがある。窓が切れるまで読み直す。
                    self.schedule_ime_refresh(crate::tuning::MODE_KEY_PASS_REREAD_MS);
                } else if observed
                    && self
                        .platform_state
                        .ime
                        .align_after_expired_mode_key_pass(now, crate::state::TickMs(now))
                {
                    // 窓の間の観測が全て時間切れだった通過。窓が切れた後の最初の成功観測で desired を揃える。
                    tracing::info!(
                        "[mode-key-follow] first successful observation after the window: desired aligned"
                    );
                }
            }
        }

        // Phase 3.7: 診断スナップショット（フォーカス変更確定直後）
        if focus.focus_changed {
            self.ir_post_focus_change_snapshot(focus.skip_imm_query);
        }
    }

    /// ADR-205（BUG-172）: 読めない窓（`Imm32Unavailable`）で、外部注入の IME キー直後の監視窓の中に、
    /// prefetch 済みの開閉の読みが基準値から変わったら実状態へ追随する。書き込み（開け直し）はしない。
    fn ir_follow_external_change(&mut self, ime_snap: Option<&crate::ime::ImeSnapshot>) {
        // Imm32Unavailable かつ GJI のときだけ（InputRelay・TsfNative・MS-IME は対象外。`external_change_watch_applies` 参照）。
        if !self.external_change_watch_applies() {
            return;
        }
        let read = ime_snap.and_then(|snap| snap.ime_on);
        let now = crate::hook::current_tick_ms();
        let tick_ms = crate::state::TickMs(now);
        let accepted =
            crate::state::probe_admission::AcceptedObservation::for_sync(self.focus_fence());
        if let Some(open) = self
            .platform_state
            .ime
            .follow_external_change(read, now, tick_ms, accepted)
        {
            tracing::info!(
                "[external-change] 監視窓の中で開閉の読みが変わった → 実状態 open={open} へ追随 \
                 (意図を捨て desired を揃える。awase は IME を書かない)"
            );
        }
    }

    /// ADR-188（BUG-149/150 の Chrome 版）: 読めない窓（GJI × `Imm32Unavailable`）で、物理のモードキー（Shift 付き・FSM の
    /// 再送出を含む）の直後の直接観測の窓の中に、prefetch 済みの開閉・conv の読みを belief と照合し、食い違う軸へ追随する。
    /// 基準値は使わず、awase は IME を書かない。awase 自身が窓の後に書いていたら採らない（R3）。
    fn ir_follow_direct_mode_key_read(&mut self, ime_snap: Option<&crate::ime::ImeSnapshot>) {
        if !self.external_change_watch_applies() {
            return;
        }
        let Some(snap) = ime_snap else {
            return;
        };
        let now = crate::hook::current_tick_ms();
        let accepted =
            crate::state::probe_admission::AcceptedObservation::for_sync(self.focus_fence());
        if let Some(follow) = self.platform_state.ime.follow_direct_read(
            snap.ime_on,
            snap.conversion_mode,
            now,
            crate::state::TickMs(now),
            accepted,
        ) {
            tracing::info!(
                "[direct-follow] モードキー直後の窓の中で実状態が belief と違った → open={:?} eisu={:?} へ追随 \
                 (conv={:?}。意図を捨て desired を揃える。awase は IME を書かない)",
                follow.open,
                follow.eisu,
                snap.conversion_mode,
            );
        }
    }

    /// ADR-227(BUG-074): `RawTsfLiteralRecovery` の give-up(否定的証拠 2 回以上・すべて SuspectedLiteral)を、
    /// 「外部から実 IME が閉じられたかもしれない」という**読み直しのきっかけ**にする。give-up を閉の観測として
    /// 書かない(推論しない)。監視窓(ADR-205)を開いて prefetch 済みの読みを照合し、基準値(直近の読み)から
    /// 閉へ変わっていれば `follow_external_change` が追随する(意図を捨て desired を揃える。IME は書かない)。
    /// 条件: GJI × Imm32Unavailable、プローブ開始時と同じ focus 世代、明示意図が ON のまま。
    pub(crate) fn ir_follow_after_literal_giveup(
        &mut self,
        evidence: crate::tsf::literal_facts::GiveUpEvidence,
    ) {
        use crate::tsf::literal_facts::GiveUpFollowDecision;
        let applies = self.external_change_watch_applies();
        let gen_now = self.platform.output.ime_mode_focus_gen.get();
        let intent = self.platform_state.ime.explicit_intent();
        let decision = crate::tsf::literal_facts::giveup_follow_decision(
            applies,
            evidence.focus_gen,
            gen_now,
            intent,
        );
        let baseline = if decision == GiveUpFollowDecision::Arm {
            self.platform_state
                .ime
                .arm_external_change_watch(crate::hook::current_tick_ms());
            self.platform_state.ime.external_change_baseline()
        } else {
            None
        };
        // 実機の不具合報告から追えるよう journal にも残す(attach_log が無くても、追随を試みたか・捨てた理由・基準値が分かる)。
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::GiveUpFollow {
                cold_seq: evidence.cold_seq,
                outcome: decision.outcome(),
                baseline,
            });
        if decision == GiveUpFollowDecision::Arm {
            self.schedule_ime_refresh(crate::tuning::MODE_KEY_PASS_REREAD_MS);
        }
    }

    // ── Stage 4: Engine 通知と次回スケジュール ──
    //
    // Phase 4: Engine に RefreshState（active 遷移検知）
    // Phase 5: 次回ポーリングをスケジュール

    fn ir_stage_notify(&mut self) {
        // Phase 4: Engine に RefreshState（active 遷移検知）
        self.ir_notify_engine_refresh();
        // Phase 4a: 通過マークの窓が切れても観測が一度も成功しなかったなら、古い明示意図を捨てる
        // （意図が残るとポーリングが止まったままになる。BUG-158）。戦略（OsPoll/SkipTyping等）によらず
        // 毎tick確認する。窓の間は`reschedule_ime_refresh`が読み直しを予約し続けるので、窓の直後に必ずここへ来る。
        // 立てた時点から読めない窓では意図を捨てない（読み取りで訂正できず、意図がbeliefの唯一の手がかり。
        // `reschedule_ime_refresh`参照）。立てた時点で読めた窓は、通過の途中で降格しても捨てる
        // （`ModeKeyPassMark::readable_at_arm`、`expire_mode_key_pass_mark`が判定。レビュー round2 A-N2）。
        let now = crate::hook::current_tick_ms();
        if self
            .platform_state
            .ime
            .expire_mode_key_pass_mark(now, crate::state::TickMs(now))
        {
            tracing::info!(
                "[mode-key-follow] window expired without a successful observation: intents invalidated"
            );
        }
        // Phase 4a': 起動時の初期値のままの desired_open を、最初の成功観測へ揃える（BUG-163）。
        self.ir_align_placeholder_desired();
        // Phase 4b: desired ≠ observed ドリフト補正（ImmCross / non-ImmCross 両対応）
        self.ir_apply_drift_correction();
        // Phase 5: 次回ポーリングをスケジュール
        self.reschedule_ime_refresh();
    }

    // ── IMM ブリッジ非対応クラスの判定 ──

    fn ir_resolve_skip_imm_query(&self) -> bool {
        !self.can_use_imm32_cross_process()
    }

    // ── フォーカス変更通知 ──

    fn ir_notify_focus_changed(&mut self, skip_imm_query: bool) {
        // 別アプリ/ウィンドウへ遷移したら進行中の actuation 試行を破棄する
        // （ADR-080 破棄条件2）。新しいフォーカス先では desired/観測前提が変わる
        // ため、attempts を持ち越さず次の tick で作り直す。
        self.discard_actuation();
        // 左Shift単独タップによる「IME-ON 半角英数」持続トグル中にフォーカスが
        // 変わった場合、半角英数状態を他アプリへ持ち越さないよう即座にかな入力へ
        // 復元する（呼び出し自体を遅延させないという意味で「即座」。復元処理自体は
        // 既存同様 spawn_local 経由の非同期 retry ループを含むため、この呼び出しが
        // フォーカス変更処理をブロックすることはない）。物理 Shift が押されている
        // とは限らないため synthetic Shift up の前置は不要（false）。
        if self.platform_state.gate.half_width_alnum.is_toggle_active() {
            tracing::info!("[shift-conv-guard] FocusChanged 中 → 半角英数トグルを強制解除");
            self.kp_restore_kana_from_half_width(false);
        }
        // IMM broken アプリ（Chrome 等）に切り替わった際に input_mode が
        // 前ウィンドウの stale な ObservedKana を引き継いでいると、FocusChanged の ctx で
        // engine が inactive になる。broken アプリでは入力モードを検出できないため、
        // ime_on=true のとき AssumedRomaji と仮定して補正する。
        // ただし ObservedEisu（英数モード確定済み）の場合は補正しない（Engine ON 誤起動防止）。
        if skip_imm_query
            && self.platform_state.ime.effective_open()
            && !self.platform_state.ime.input_mode().is_romaji_capable()
        {
            if let Some(new_mode) = self.platform_state.ime.correction_for_imm_broken() {
                tracing::info!(
                    "FocusChanged: input_mode assumed romaji (IMM broken, stale kana from prev window)"
                );
                let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
                self.apply_input_mode_correction(
                    new_mode,
                    crate::state::ime_event::InputModeApplyStrategy::ImmBrokenCorrection,
                    tick_ms,
                );
            } else {
                // romaji-capable は外側の if で除外済みなので None = ObservedEisu のみ
                tracing::info!(
                    "FocusChanged: input_mode スキップ (belief=ObservedEisu, eisu guard)"
                );
            }
        }
        let ctx = self.build_ctx();
        let decision = self.engine.on_command(EngineCommand::FocusChanged, &ctx);
        self.execute_decision(decision);
    }

    // ── 読み取り方針の決定 ──
    //
    // 最後のキー活動（物理キー押下 または VK/TSF 出力）から TYPING_IDLE_MS 以内は
    // IMM との SendMessage を一切行わない。

    /// `&mut self` なのは、通過マーク(`ModeKeyPassMark`、`ScopedOneShot::peek`)がフォアグラウンド変更を見て自動失効させるため
    /// （ADR-187）。読み取り方針の決定そのものは副作用を持たない（失効は「マークが無効になった」という事実の反映だけ）。
    ///
    /// FCIS F1: `observe`（`ir_observe_read_strategy_facts`）→ 純粋な `decide_read_strategy`
    /// （`state/ime_read_strategy.rs`）→ ここでの理由のログ、の順。
    fn ir_decide_read_strategy(&mut self, skip_imm_query: bool) -> ImeReadStrategy {
        let facts = self.ir_observe_read_strategy_facts(skip_imm_query);
        let decision = decide_read_strategy(&facts);
        if decision.typing_guard_bypassed {
            tracing::debug!(
                "Explicit intent: bypassing typing-idle guard for IME verify (idle={}ms)",
                facts.idle_ms
            );
        }
        match decision.reason {
            ReadReason::TypingActive => tracing::debug!(
                "Skipping observer/SSOT write: typing active (idle={}ms)",
                facts.idle_ms
            ),
            ReadReason::ShiftConvGuard => {
                tracing::debug!("Skipping observer/SSOT write: shift-conv-guard 中");
            }
            ReadReason::ImmQuerySkipped | ReadReason::OsPoll => {}
        }
        decision.strategy
    }

    /// 読み取り方針の決定に要る事実を、グローバル・時計・状態から 1 度だけ読んで所有型にまとめる(shell-in)。
    ///
    /// 通過マークの有効判定(`mode_key_pass_mark_live`)はフォアグラウンド変更で失効する副作用を持つので、
    /// 元の関数と同じ条件（打鍵中のときだけ）、同じ回数（1 回）で読む。
    fn ir_observe_read_strategy_facts(&mut self, skip_imm_query: bool) -> ReadStrategyFacts {
        // 最後のキー活動（物理キー押下 または VK/TSF 出力）からの経過時間。
        let last_activity = self.platform_state.gate.last_hook_activity_ms.max(
            crate::tsf::probe_bridge::OUTPUT_GATE
                .last_vk_output_ms
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        let now = crate::hook::current_tick_ms();
        let idle_ms = now.saturating_sub(last_activity);
        let mode_key_pass_live =
            is_typing(idle_ms) && self.platform_state.ime.mode_key_pass_mark_live(now);
        // Shift conv 安全網のブリップ中、または左Shift単独タップによる半角英数持続トグル中
        // （`kp_stage_shift_conv_guard`）は OS poll を凍結する。
        let shift_conv_guard_active = self.platform_state.gate.half_width_alnum.is_guard_pending()
            || self.platform_state.gate.half_width_alnum.is_toggle_active();
        ReadStrategyFacts {
            idle_ms,
            skip_imm_query,
            mode_key_pass_live,
            explicit_intent_present: self.platform_state.ime.explicit_intent().is_some(),
            applied_known: self.platform_state.ime.model().applied
                != crate::state::ime_model::AppliedImeState::Unknown,
            shift_conv_guard_active,
        }
    }

    // ── IME 状態のポーリングと学習 ──

    /// IME状態を読んでbeliefへ反映し、`imm-learning`へ渡す。戻り値は「この読み取りで観測（`ime_on`）を
    /// 得られたか」。時間切れの空振りは`miss_count`を増やさない（BUG-158追補）ので、`miss_count`の増減では
    /// 「読み取りが成功したか」を判定できない——通過マークの追随（ADR-187）はこの戻り値で判定する。
    fn ir_poll_and_learn(
        &mut self,
        miss_before: u32,
        ime_snap: Option<&crate::ime::ImeSnapshot>,
    ) -> bool {
        let poll = self.platform_state.ime.capture_poll_state();
        let ime_on_before_poll = poll.ime_on;
        let input_mode_before_poll = poll.input_mode;
        // BUG-106追補3・4: awase自身のトレイ/設定画面から読んだ観測を
        // input_mode beliefに採用しないための判定材料（`is_own_ui_window`）。
        let focus_process_name = self.platform.focus.process_name().to_owned();

        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        let mut observer_out = ime_snap.map_or_else(
            || unsafe {
                crate::observer::ime_observer::poll_and_classify_ime(
                    poll.ime_on,
                    poll.force_guard,
                    poll.input_mode,
                    poll.prev_conv,
                    &focus_process_name,
                )
            },
            |snap| {
                crate::observer::ime_observer::classify_fetched_snapshot(
                    snap,
                    tick_ms.0,
                    poll.ime_on,
                    poll.force_guard,
                    poll.input_mode,
                    poll.prev_conv,
                    &focus_process_name,
                )
            },
        );
        // ImmCross アプリ（LINE 等）は awase が ROMAN ビットを立てるまで conv=0x09 がデフォルト。
        // romaji=false → ObservedKana の観測はユーザーの意図ではなく IME のデフォルト状態を
        // 誤って信頼することになるため、ImmCross パスでは ObservedKana の伝播を抑制する。
        // romaji=true（ROMAN ビット確認済み）→ ObservedRomaji はそのまま通す。
        if self.can_use_imm32_cross_process()
            && matches!(
                observer_out.new_input_mode,
                Some(InputModeState::ObservedKana)
            )
        {
            observer_out.new_input_mode = None;
        }
        let observed = observer_out.observer_poll.is_some();
        let accepted =
            crate::state::probe_admission::AcceptedObservation::for_sync(self.focus_fence());
        self.platform_state
            .ime
            .apply_ime_update(&observer_out, tick_ms, accepted);

        let miss_after = self.platform_state.ime.detect_miss_count();

        self.ir_log_poll_diff(
            ime_on_before_poll,
            input_mode_before_poll,
            miss_before,
            miss_after,
        );

        self.learn_imm_capability_from_miss(miss_before, miss_after);
        observed
    }

    /// [診断] フォーカス変更から 10 秒以内で状態が変わった場合にログ出力。
    fn ir_log_poll_diff(
        &self,
        ime_on_before_poll: bool,
        input_mode_before_poll: InputModeState,
        miss_before: u32,
        miss_after: u32,
    ) {
        let age_ms = crate::hook::current_tick_ms()
            .saturating_sub(self.platform_state.focus.last_focus_change_ms);
        if age_ms < 10_000 {
            let ime_on_after = self.platform_state.ime.effective_open();
            let input_mode_after = self.platform_state.ime.input_mode();
            let ime_changed = ime_on_before_poll != ime_on_after;
            let mode_changed = input_mode_before_poll != input_mode_after;
            if ime_changed || mode_changed {
                tracing::info!(
                    "ObserverPoll +{}ms since focus: {}{}",
                    age_ms,
                    if ime_changed {
                        format!(
                            "ime_on {} → {}(intent={:?}) ",
                            ime_on_before_poll,
                            ime_on_after,
                            self.platform_state.ime.explicit_intent(),
                        )
                    } else {
                        String::new()
                    },
                    if mode_changed {
                        format!("mode {input_mode_before_poll:?} → {input_mode_after:?}")
                    } else {
                        String::new()
                    },
                );
            } else if miss_after > 0 {
                tracing::debug!(
                    "ObserverPoll +{age_ms}ms since focus: detection failed (miss={miss_after}), stale ime_on={ime_on_before_poll} mode={input_mode_before_poll:?}",
                );
            }
        }
        let _ = miss_before;
    }

    // ── 診断スナップショット（フォーカス変更確定直後）──

    fn ir_post_focus_change_snapshot(&mut self, skip_imm_query: bool) {
        if !skip_imm_query {
            crate::ime_diagnostic::ImeDiagnosticSnapshot::capture("focus_changed").log();
        }
        tracing::debug!("[composition] focus change → marking cold");

        // `matches!(profile, AppImeProfile::TsfNative)` ではなく `is_effectively_tsf_native`
        // を使うこと。CASCADIA_HOSTING_WINDOW_CLASS (Windows Terminal) 等は
        // `AppImeProfile::from_class_name` の優先順位により `Imm32Unavailable` に分類され
        // `TsfNative` には決してならないため、直接比較だと誤って「非 TSF ネイティブ」と
        // 判定してしまう（2026-07-05: これが原因で enforce IME OFF ブロックが
        // Windows Terminal に対して誤発火していた）。ADR-098 決定1-a のために
        // 算出位置を mirror 書き込みより前へ移した。
        let new_profile_is_tsf_native = self
            .platform
            .current_app_profile()
            .is_effectively_tsf_native(self.platform.focus.class_name());

        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        // ADR-098 決定1-a（BUG-69 F2 の修正）: TsfNative では `applied` を
        // `Unknown` のまま維持する（`focus_tracking.rs` の hard pre-sync が
        // 非 TsfNative について既に守っている不変条件——INV-A97-1——を
        // ここでも適用する）。何も apply していないのに belief を `applied
        // = Confirmed` として書くと、（撤去済みの）`apply_force_on_for_imm_broken` の
        // スパムガードが恒久的に早期 return し、BUG-16 の修正が TsfNative で
        // 一度も実効しない（詳細は known-bugs.md BUG-69 / ADR-098）。
        if !new_profile_is_tsf_native {
            let ime_on_now = self.platform_state.ime.effective_open();
            self.platform_state
                .ime
                .record_confirmed(ime_on_now, tick_ms.0);
        }
        self.platform.mark_composition_cold_focus_change();
        let mode = self.platform.output.injection_mode;
        self.platform.gji_on_focus_change(mode);
        for entry in self.platform.drain_journal_entries() {
            self.platform_state.ime.journal.absorb(entry);
        }

        // ADR-098 決定2: 旧 TsfNative force-on ブロック（GJI VK_IME_ON を
        // shadow_on 無視で強制送信）はここに存在した。決定1-a が `applied` を
        // 偽装しなくなったことで、通常の strategy chain（`shadow_on=false`
        // になる）と、決定1-c で有界化された `apply_force_on_for_imm_broken`
        // （後に `f83084b3` で関数ごと撤去）の両方が正しく VK_IME_ON を送れるようになったため撤去した。撤去の
        // 詳細な根拠は known-bugs.md BUG-69 / ADR-098 決定2 参照。

        // ADR-098 決定1-b: `applied.applied_open()` の生値ではなく `warmup_ime_on()`
        // （`applied ?? belief`）を使う。決定1-a により TsfNative では `applied`
        // が `Unknown` のまま残るため、生値のままだと `unwrap_or(false)` で
        // warmup が握り潰され BUG-02 のリテラル化が再燃する。
        // 旧 eisu_guard（tray で英数／カタカナ等に切り替えた直後の conv を読み、英数なら
        // warmup をスキップする防御）は 2026-08-20、BUG-34 横展開の一環として撤去した。
        //
        // この読み取りは `SendMessageTimeoutW` ベースで、エンジンスレッドを塞ぐ BUG-34
        // の対象そのものだった。これを塞がずに済ませる設計を3ラウンドの Opus premortem
        // レビューで検討したが、いずれも致命的な欠陥に行き着いた:
        //   - 非同期化して warmup 自体を遅らせる案 → WARMUP_GRACE_MS / GJI settle-grace
        //     が無効化され spurious IME OFF を再燃させる。
        //   - focus_epoch でキャッシュの鮮度を判定する案 → epoch は Site A 到達前に
        //     必ず上がっているため、キャッシュは常に「別ウィンドウ由来」と判定され
        //     ガードが恒久的に不発になる。
        //   - pid+timestamp でキャッシュする案 → (a) Site A 自身の書き込みを消すと
        //     直後の idle-conv-check の conv_mode_changed 判定が反転し、
        //     docs/experiments.md エントリ03 に記録済みの spurious Engine ON を
        //     再燃させる、(b) キャッシュ更新契機（idle-conv-check/Site C）はいずれも
        //     打鍵駆動のため、ガードが本来検出すべき「トレイでの無操作切替」を
        //     観測できず陳腐化したキャッシュが warmup を誤スキップさせる、
        //     (c) 対象アプリも WezTerm/Windows Terminal 程度まで縮小する。
        // 詳細は docs/known-bugs.md BUG-34 追補4 参照。
        //
        // 結論として、ユーザーが tray で明示的に半角英数へ切り替えた直後にフォーカス
        // 復帰すると、この warmup で一度だけひらがなへ戻る（既知の制限として受け入れ、
        // ガードでの防御はしない）。
        // フォーカス変更時の eager warmup（VK_IME_ON）は ADR-212 P4 で撤去した。
        // 旧「フォーカス変更時の強制 OFF」（非 TsfNative で belief=OFF なら新窓へ
        // `set_ime_open_ordered(false)`）は 2026-09-25 に撤去した。CI 実測で現 develop では
        // warrant が拒否して書き込まないか発火しないかのどちらかで、撤去前後に差が出なかった
        // （docs/adr/191-calibration-experiments.md「A/B-1」）。
    }

    // ── ドリフト補正 ──
    //
    // desired ≠ observed（`evaluate_drift` が補正を要すると判定したずれ）が続く間、有界に再送する。
    //
    // - IMM32 クロスプロセス対応アプリ（LINE 等 ImmCross）: set_ime_open(desired) を使う。
    // - non-ImmCross（GJI/TsfNative/Blacklist、Chrome/Windows Terminal 等）:
    //   set_ime_open は can_use_imm32_cross_process=false で no-op になるため使えない。
    //   （撤去済みの）apply_force_on_for_imm_broken は ON 方向専用で OFF 方向の乖離は担当しなかったため、
    //   ここで view を構築し、strategy chain 経由の apply_ime_open_with_view（実 VK 送信）を使う
    //   （2026-07-08 実機: Windows Terminal/Chrome + GJI で IME OFF コンボ送信後、
    //   Engine 内部は即 OFF になるが OS 側 IME は ON のまま固定される不具合。
    //   set_ime_open の戻り値を見ずに mirror_applied_open_with_ts で belief だけ
    //   「反映済み」にしていたため、実際には一切再送されていなかった。詳細は
    //   docs/known-bugs.md BUG-20 を参照）。

    /// 起動時の初期値のままの `desired_open` を最初の成功観測へ 1 回だけ揃える（BUG-163、代案A）。
    ///
    /// 揃えた値が「開」で、非 TsfNative なら、フォーカス時にスキップした先同期（`presync_applied_open_on`。GJI への ImeOn
    /// 通知を含む）をここで行う。「閉」なら行わない（IME を閉じて起動したとき awase が開けない）。
    fn ir_align_placeholder_desired(&mut self) {
        if !self.engine.is_user_enabled() || !self.platform_state.ime.belief.is_japanese_ime() {
            return;
        }
        let now_ms = crate::hook::current_tick_ms();
        let tick_ms = crate::state::TickMs(now_ms);
        let Some(open) = self
            .platform_state
            .ime
            .align_placeholder_desired(std::time::Instant::now(), tick_ms)
        else {
            return;
        };
        tracing::info!("[startup-align] desired_open を最初の成功観測へ揃えた: desired={open}");
        let tsf_native = self
            .platform
            .current_app_profile()
            .is_effectively_tsf_native(self.platform.focus.class_name());
        if open && !tsf_native {
            self.presync_applied_open_on(tick_ms);
        }
    }

    fn ir_evaluate_drift(
        &self,
        now: std::time::Instant,
    ) -> Result<
        crate::state::platform_state::DriftCorrection,
        crate::state::drift_correction::NoDrift,
    > {
        self.platform_state.ime.evaluate_drift(now)
    }

    /// drift correction の observe（FCIS F4、殻）。判断に使う事実だけを集める。書かない。
    ///
    /// `imm_cross` 以降の事実は、`drift` があり settle 中でないときだけ読む（それ以外は
    /// `decide_drift_plan` が見ずに返すので、読まない＝元のコードと読む条件が同じ）。
    fn ir_observe_drift_facts(&self, now: std::time::Instant) -> DriftFacts {
        // 稼働条件(`engine_enabled`/`japanese_ime`)が偽でも `evaluate_drift`・`default_feedback` は読む
        // （純粋な読み取りで、影響は debug ログのみ。`decide_drift_plan` が `Idle(EngineDisabled | NotJapaneseIme)` を返す）。
        let drift = self.ir_evaluate_drift(now);
        let settling = drift.is_ok() && self.ime_apply_should_defer();
        let mut facts = DriftFacts {
            now,
            engine_enabled: self.engine.is_user_enabled(),
            japanese_ime: self.platform_state.ime.belief.is_japanese_ime(),
            drift,
            settling,
            active: self.active_actuation.as_ref().map(Actuation::snapshot),
            default_policy: self.platform_state.ime.default_feedback(),
            imm_cross: false,
            warrant_would_block: false,
            diag_already_notified: self.drift_giveup_notified_this_focus,
            converged: false,
            fresh_evidence_after_giveup: false,
        };
        let Some(drift) = drift.ok().filter(|_| !settling) else {
            return facts;
        };
        // 読み戻しの `since` は、決定側と同じ `resolve_actuation` で解決した試行から取る。
        let (act, _) = resolve_actuation(
            facts.active.as_ref(),
            drift.desired,
            facts.default_policy,
            now,
        );
        facts.imm_cross = self.can_use_imm32_cross_process();
        // BUG-163: 書き込み経路（`set_ime_open_ordered`）を通る ImmCross だけが授権を見る。
        facts.warrant_would_block = facts.imm_cross
            && self
                .issue_actuation_order_with_origin(drift.desired, act.origin)
                .would_have_blocked();
        // ADR-090 §2.B（INV-52）: 読み戻しは `ObservationStore::read_back` の 1 本だけを通る。
        // 戻り値は `ConvergedReceipt` で、観測として書き戻せない。
        let observations = &self.platform_state.ime.model().observations;
        match act.policy {
            FeedbackPolicy::Read { .. } => {
                facts.converged = observations
                    .read_back(
                        now,
                        act.sent_at,
                        crate::state::observation_store::ReadBackQuery::Converged {
                            desired: drift.desired,
                        },
                        act.attempts,
                    )
                    .converged();
            }
            FeedbackPolicy::Blind { .. } => {
                if let Some(gave_up_at) = act.gave_up_at {
                    facts.fresh_evidence_after_giveup = observations
                        .read_back(
                            now,
                            gave_up_at,
                            crate::state::observation_store::ReadBackQuery::AnyFreshEvidence,
                            act.attempts,
                        )
                        .resolution()
                        == crate::state::ime_actuation::Resolution::ExternalChange;
                }
            }
        }
        facts
    }

    // `#[tracing::instrument]`（ADR-139決定3）のマクロ展開が実測でcognitive_complexityを
    // 17/15へ押し上げた（Windows実機CIで検出、log→tracingのマクロ名置換自体は
    // ADR-139決定1で影響なしと確認済みだったが、#[instrument]の展開はそれとは
    // 別に複雑度を増やす）。この関数はADR-080不変条件6の対象で
    // architecture_guard.rsのマーカーベーステストが依存する繊細な構造のため、
    // 実行側のロジックの分割はしない。
    //
    // FCIS F4: observe（`ir_observe_drift_facts`）→ 純粋な `decide_drift_plan`
    // （`state/drift_plan.rs`）→ この関数の execute、に分けた。判断の条件・順序・有界の再送は
    // 元のまま（ADR-080 の `FeedbackPolicy`、BUG-43 の無限再送の防止、BUG-163、BUG-68）。
    // IME へ書く呼び出し（`apply_ime_open_with_view`）は `actuation_call_guard` の許可呼び出し元が
    // この関数名なので、execute は別関数に出さずここに置く。
    #[allow(clippy::cognitive_complexity)]
    #[tracing::instrument(level = "debug", skip_all)]
    fn ir_apply_drift_correction(&mut self) {
        // BUG-20 で non-ImmCross（GJI/TsfNative/Blacklist）向けの再送分岐を追加した際、
        // この関数冒頭に残っていた `ir_resolve_skip_imm_query()`（=
        // `!can_use_imm32_cross_process()`）による早期 return を消し忘れていた。
        // 追加した non-ImmCross 分岐はまさにこのガードが true になる場合に実行される
        // はずのコードであり、ガードが残っていたことで一度も到達できない dead code に
        // なっていた（BUG-20 の「実機検証は未実施」という注記通り、実機で一度も
        // 検証されないまま今日まで放置されていた）。詳細は known-bugs.md BUG-20 追補参照。
        let now = std::time::Instant::now();
        let facts = self.ir_observe_drift_facts(now);
        let plan = decide_drift_plan(&facts);
        // E1: 送らない・打ち切る・収束とみなす・保留する決定の根拠（`plan.basis()`）は、殻がログに出すだけ。
        let basis = plan.basis();
        let act = match plan {
            DriftPlan::Idle(reason) => {
                // 毎 tick 通る（ずれが無いのが普通）ので trace。
                tracing::trace!("[drift] 補正しない: {reason:?} basis={basis:?}");
                return;
            }
            DriftPlan::DeferToSettle { drift } => {
                // 他の settle 対応経路（撤去済みの apply_force_on_for_imm_broken 等）と同じく settle 明けに必ず再試行する。
                self.schedule_settle_retry(&format!(
                    "drift correction skipped (settling): desired={} observed={} basis={basis:?}",
                    drift.desired, drift.observed
                ));
                return;
            }
            DriftPlan::Act(act) => act,
        };
        let drift = act.drift;
        let (desired, observed, duration_ms) = (drift.desired, drift.observed, drift.duration_ms);
        let (act_policy, act_attempts, act_origin) = (
            act.actuation.policy,
            act.actuation.attempts,
            act.actuation.origin,
        );

        // ADR-080: actuation を型付きトランザクション（`Actuation`）として扱い、
        // feedback（収束確認）方針を `AppImePolicy::default_feedback` からデータとして
        // 受け取る。これにより BUG-43 の「実送信の結果が observation store に
        // フィードバックされず、observe tick ごとに同じ VK を無限再送するタイトループ」を
        // 手作りクールダウン（旧 `last_drift_correction_send`）に頼らず型レベルで防ぐ。
        //
        // - `Blind`（Imm32Unavailable / TsfNative、実読み戻し不能）: `max_attempts` 到達で
        //   `GiveUp` し、`desired` が変わる（＝新しい `Actuation`）まで再送しない。
        // - `Read`（ImmCross 等、実読み戻し可能）: `sent_at` 以降の trusted 観測が desired と
        //   一致すれば `Confirmed` として破棄、そうでなければ従来同様に再送する。
        //
        // なお `ir_evaluate_drift`（=`evaluate_drift`）の乖離「検知」側は
        // ADR-080 Phase 1 では従来どおり `most_recent_trusted`（since フェンシングなし）を
        // 使い続ける。since フェンシング（`most_recent_trusted_after`）を使うのは
        // `Read` 収束「確認」側のみで、この非対称は ADR-080 が意図的に許容している。
        if act.install_actuation {
            self.install_actuation(&act.actuation);
        }

        // BUG-163: 授権（warrant）が下りない補正は、そもそも書けない（`set_ime_open_ordered` は ADR-090 A-2 で
        // `Unwarranted` を拒否する）。書けないのに「検知」（journal・`DriftDetected`＝`applied` を `Optimistic` に
        // 偽装・試行回数の加算・「IME状態を確認できません」のバルーン）まで進めると、起動直後（明示意図なし・
        // 観測だけが「閉」）に 500ms ごとの空振りと誤通知が続く。書き込み経路（`set_ime_open_ordered`）を通る
        // ImmCross だけを対象にする（Blind の再送・give-up の有界化〈BUG-43〉は従来どおり）。
        // 判定は `decide_drift_plan`（`DriftStep::SkipWarrantWouldBlock`、診断は出さない）。

        if act.notify_diagnostic {
            self.ir_notify_drift_giveup_diagnostic(desired, observed, duration_ms, now);
        }

        let send_path = match act.step {
            DriftStep::SkipWarrantWouldBlock => {
                tracing::debug!(
                    "[drift] 授権が下りないため補正を見送る（検知しない）: desired={desired} \
                     observed={observed} for {duration_ms}ms (source={:?} confidence={:?}) \
                     basis={basis:?}",
                    drift.source,
                    drift.confidence,
                );
                return;
            }
            DriftStep::GiveUp(park) => {
                // ADR-082 Phase 0.5: 打ち切り判定も出所・世代付きで構造化記録する
                // （BUG-43 の「16 回中 5 回だけ送り、残りは GiveUp」を journal から
                // 型で追えるようにする）。observations には書き込まない規約は不変。
                self.platform_state.ime.journal.record(
                    crate::journal::JournalEntry::ImeActuation {
                        record: crate::state::ime_actuation::ActuationRecord::new(
                            act_origin,
                            desired,
                            act_policy,
                            act_attempts,
                        ),
                    },
                );
                // max_attempts 到達。observations には一切書き込まない（BUG-33 型の
                // 収束偽装を避ける）。ただし「一度諦めたら desired が変わるまで永久に
                // 補正しない」硬直（ADR-080「有限 Blind からの復旧条件」）を避けるため、
                // 外部で状況が動いた証拠が来たら試行をやり直す（task #15）。
                //
                // 復旧判定は観測の「値」ではなく「鮮度」で行う（`decide_drift_plan` の
                // `GiveUpPark::Rearm`）。drift 補正は observed != desired（乖離）が続く間
                // しか走らず、open/close は bool のため「間違った値」は !desired の1通り
                // しか存在しない。よって「target と異なる値の観測が来たら復旧」はほぼ毎
                // tick 真になり GiveUp を即座に無効化してしまう。意味のある信号は
                // 「諦めた時刻以降に新しい観測が record されたか」＝世界で何かが動いたか
                // （値は問わない）。BUG-68: クールダウン未経過の間は再武装しない
                // （`GiveUpPark::CooldownPending`。詳細は `blind_rearm_cooldown_elapsed`、
                // tuning.rs の定数コメント）。
                match park {
                    GiveUpPark::FirstTime => {
                        // この tick で初めて GiveUp に到達。境界時刻を刻んで parked に
                        // する。この tick では再送も復旧判定もしない（次 tick 以降で
                        // `now` より後の観測だけを「新しい」とみなせるようにするため）。
                        if let Some(actuation) = self.active_actuation.as_mut() {
                            actuation.gave_up_at = Some(now);
                        }
                        // ADR-089 §2.5（INV-46）: 打ち切りの帰結は
                        // `ConvergedReceipt` で表す。**この型は
                        // `Observed<E>` / `AnyObservation` へ変換できない**
                        // ため、give-up したのに観測を書いて収束したように
                        // 見せる（BUG-33 型の収束偽装）ことが構造的に
                        // 不可能である。
                        let receipt = crate::state::ime_actuation::ConvergedReceipt::new(
                            crate::state::ime_actuation::Resolution::GaveUp,
                            act_attempts,
                        );
                        tracing::debug!(
                            "[drift] actuation gave up (Blind): desired={desired} \
                             observed={observed} converged={} attempts={} basis={basis:?}",
                            receipt.converged(),
                            receipt.attempts()
                        );
                    }
                    GiveUpPark::CooldownPending | GiveUpPark::StillParked => {}
                    GiveUpPark::Rearm => {
                        // 試行を破棄して次 tick の `install_actuation` に attempts=0・新しい
                        // sent_at・gave_up_at=None で作り直させる。実際の再送は次 tick に任せ、
                        // discard した同じ tick では送らない（ロジックを単純に保つ）。
                        tracing::debug!(
                            "[drift] fresh observation after give-up → 試行を破棄して\
                             再試行: desired={desired} observed={observed} attempts={act_attempts} \
                             basis={basis:?}"
                        );
                        self.discard_actuation();
                    }
                }
                return;
            }
            DriftStep::Confirmed => {
                // `sent_at` 以降の trusted 観測が desired と一致していれば収束済み
                // （`Resolution::Confirmed`）。再送不要なので試行を破棄する。
                // 収束の帰結は `ConvergedReceipt`（ADR-089 §2.5、INV-46）で、観測ストアへは
                // 何も書かない。
                tracing::debug!(
                    "[drift] actuation confirmed (Read): desired={desired} \
                     converged=true attempts={act_attempts} → 破棄 basis={basis:?}"
                );
                self.discard_actuation();
                return;
            }
            DriftStep::Send(path) => path,
        };

        tracing::warn!(
            "[drift] correction: observed={observed} ≠ desired={desired} for {duration_ms}ms \
             → set_ime_open({desired}) (source={:?} confidence={:?})",
            drift.source,
            drift.confidence,
        );
        // ADR-082 Phase 0.5: 実送信する試行を出所・世代付きで構造化記録する。
        // `Blind` はここに到達する時点で必ず `Send`（`GiveUp` は上で return 済み）、
        // `Read` は常に `Send`。`action` は `ActuationRecord::new` が
        // `FeedbackPolicy::decide_action` で導出する。
        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::ImeActuation {
                record: crate::state::ime_actuation::ActuationRecord::new(
                    act_origin,
                    desired,
                    act_policy,
                    act_attempts,
                ),
            });
        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        self.platform_state.ime.dispatch_event(
            crate::state::ime_event::ImeEvent::DriftDetected {
                desired,
                observed,
                duration_ms,
            },
            tick_ms,
        );
        // ADR-090 §2.A 設計案 3 / A-1（shadow）。drift correction は既に
        // `EventOrigin`（`act_origin`）を持っているので、それをそのまま
        // order の出所として使う（journal の `ImeActuation` と揃う）。
        let order = self.issue_actuation_order_with_origin(desired, act_origin);
        match send_path {
            SendPath::ImmCross => {
                // ADR-090 §2.A A-2（2026-09-19）: `set_ime_open_ordered`が実際に
                // 書いたときだけ`applied`を`Optimistic`にする。以前は戻り値を
                // 無視して無条件に呼んでおり、A-2導入前（常に書き込む shadow
                // モード）は実害が無かったが、warrant無し（`Unwarranted`）で
                // 書き込みを拒否した場合に「送っていないのに送った体で記録する」
                // 欠陥になっていた（ADR-098が警告する「belief をactuationの
                // 記録として書く」誤用と同型）。
                if self.platform.set_ime_open_ordered(order) {
                    self.platform_state.ime.record_optimistic(desired);
                }
            }
            SendPath::StrategyChain => {
                // set_ime_open は IMM32専用で Blacklist/TsfNative では no-op のため、
                // （撤去済みの）apply_force_on_for_imm_broken と同じ strategy chain 経由の実送信を使う。
                let view = self.platform.build_ime_control_view(None);
                let (outcome, mut record) = self.platform.apply_ime_open_with_view(order, &view);
                // /code-review指摘（PR #201 wave3）: この同期記録点は`caller`が
                // 常に`None`のままで、`site=Sync`の他の呼び出し元と記録上区別
                // できなかった（B-2、PR #201パターンに揃える）。
                record.caller = Some(
                    crate::state::ime_actuation_decision::DecisionSite::BlacklistDriftCorrection,
                );
                self.platform_state
                    .ime
                    .journal
                    .record(crate::journal::JournalEntry::ActuationDecision { record });
                tracing::info!(
                    "Blacklist drift correction: apply_ime_open({desired}) → {outcome:?}"
                );
                self.on_ime_apply_complete(
                    desired,
                    outcome,
                    None,
                    crate::state::ime_event::OpenApplyReason::DriftCorrection,
                );
            }
        }

        // 実送信したので試行回数と世代を1つ進める（`advance_epoch`）。`Blind` はこれが
        // `max_attempts` に達すると次回 `GiveUp` する。`Read` は attempts では打ち切らないが、
        // 一貫性のため同じ送信経路で加算する。`Confirmed`/`GiveUp` で return したパスは
        // ここに到達しないため加算されない。`attempts` と `origin.epoch` を別々に動かして
        // 片方を忘れないよう、両者を同時に進めるメソッドに集約している（ADR-082 Phase 0.5）。
        if let Some(actuation) = self.active_actuation.as_mut() {
            actuation.advance_epoch();
        }
    }

    /// `ir_apply_drift_correction` から切り出したユーザー向け診断通知
    /// （clippy `cognitive_complexity` 対策、敵対的コードレビュー由来の
    /// CI失敗を受けた純粋なリファクタ——挙動は変更しない）。
    ///
    /// ADR-132 Phase 1 follow-up: the user-facing diagnostic must be based on how long
    /// the drift has persisted, not on `FeedbackPolicy::Blind` reaching `GiveUp`.
    /// `FeedbackPolicy::Read` intentionally never gives up by attempts, so tying this
    /// notification to `GiveUp` leaves read-back capable apps silent during long drift.
    ///
    /// Reuse the already-measured drift-correction wait constant instead of adding a new
    /// tuning number: this is the same "how long should drift correction wait before
    /// escalating" context as Blind re-arm, and avoids introducing an unmeasured magic
    /// number under `.claude/rules/tuning-constants.md`.
    fn ir_notify_drift_giveup_diagnostic(
        &mut self,
        desired: bool,
        observed: bool,
        duration_ms: u64,
        now: std::time::Instant,
    ) {
        // 出すかどうか（継続時間・通知済み）は `decide_drift_plan` の `notify_diagnostic`。
        self.show_tray_balloon(
            "awase",
            "このアプリではIME状態を確認できません。\n入力に違和感があれば、該当のIME切替キーをもう一度押してください。",
        );
        self.drift_giveup_notified_this_focus = true;
        self.drift_giveup_started_at = Some(now);

        let trusted = self
            .platform_state
            .ime
            .model()
            .observations
            .most_recent_trusted(now);
        let sent_vk = vec![crate::journal::ImeVkDiagnostic {
            vk_code: if desired {
                crate::vk::VK_IME_ON.0
            } else {
                crate::vk::VK_IME_OFF.0
            },
            kind: if desired { "VK_IME_ON" } else { "VK_IME_OFF" },
            source: "drift-correction-duration-giveup",
        }];
        self.platform_state.ime.journal.record(
            crate::journal::JournalEntry::DriftGiveUpDiagnostic {
                record: crate::journal::DriftGiveUpDiagnosticRecord {
                    desired_open: desired,
                    observed_open: observed,
                    drift_duration_ms: duration_ms,
                    observation_source: trusted.map(|o| o.source),
                    observation_confidence: trusted.map(|o| o.confidence),
                    sent_vk,
                    intent_source: self
                        .platform_state
                        .ime
                        .model()
                        .last_intent
                        .as_ref()
                        .map(|intent| intent.source),
                    layout_name: self.platform.tray.current_layout_name().to_string(),
                    half_width_alnum_toggle_active: self
                        .platform_state
                        .gate
                        .half_width_alnum
                        .is_toggle_active(),
                },
            },
        );
    }

    // ── Engine 通知 ──

    pub(super) fn notify_engine_refresh(&mut self) {
        self.ir_notify_engine_refresh();
    }

    fn ir_notify_engine_refresh(&mut self) {
        let ctx = self.build_ctx();
        tracing::debug!(
            "[notify-refresh] ctx.ime_on={} ctx.is_jp={} explicit_intent={:?}",
            ctx.ime_on,
            ctx.is_japanese_ime,
            self.platform_state.ime.explicit_intent(),
        );
        let decision = self.engine.on_command(EngineCommand::RefreshState, &ctx);
        self.execute_decision(decision);
    }
}
