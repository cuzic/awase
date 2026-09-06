#![allow(unsafe_code)] // Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
use awase::engine::{EngineCommand, InputModeState, KanaLockHysteresis};

use super::Runtime;
use crate::state::ime_actuation::{decide_actuation_action, ActuationAction, FeedbackPolicy};
use crate::tuning::TYPING_IDLE_MS;

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

// ── ImeReadStrategy ──

/// IME 読み取り方針の決定結果
#[derive(Debug)]
enum ImeReadStrategy {
    /// タイピング中 — IMM/TSF を一切呼ばない
    SkipTyping,
    /// 既知ブラックリストクラス — shadow SSOT のみ使う
    Blacklist,
    /// OS をポーリングする通常パス
    OsPoll,
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
        self.ir_stage_observe(&focus, &strategy, ime_snap);
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

    fn ir_stage_strategy(&self, focus: &FocusInfo) -> ImeReadStrategy {
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
        strategy: &ImeReadStrategy,
        ime_snap: Option<&crate::ime::ImeSnapshot>,
    ) {
        log::debug!(
            "[stage-observe] strategy={:?} belief_on={} explicit_intent={:?}",
            strategy,
            self.platform_state.ime.effective_open(),
            self.platform_state.ime.explicit_intent(),
        );
        match strategy {
            ImeReadStrategy::SkipTyping => {}
            ImeReadStrategy::Blacklist => {
                log::debug!("Skipping IMM query for known-broken class (shadow state SSOT)");
                // GJI I/O 観測は active IME が GJI のときに限定する。MS-IME 使用中も
                // GJI Converter プロセスは常駐しており、そのバックグラウンド I/O を
                // 根拠に observer_poll を書くと無関係な belief 汚染になる。
                if crate::tsf::observer::tsf_obs().active_ime_kind()
                    == crate::tsf::observer::ActiveImeKind::GoogleJapaneseInput
                {
                    let obs = crate::observer::gji_observer::observe_gji_after_focus(
                        self.platform_state.focus.last_focus_change_ms,
                        self.platform_state.ime.input_mode(),
                    );
                    log::debug!(
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
                        log::info!(
                            "[stage-observe] GJI I/O 中に belief=ObservedEisu → AssumedRomaji \
                             訂正 (GjiIoInference)"
                        );
                        self.platform_state.ime.dispatch_event(
                            crate::state::ime_event::ImeEvent::InputModeObserved {
                                mode,
                                source: crate::state::ime_event::ObservationSource::GjiIoInference,
                                confidence: crate::state::ime_event::ObservationConfidence::Medium,
                                at: tick_ms,
                            },
                            tick_ms,
                        );
                    }
                } else {
                    log::debug!("[stage-observe] GJI observe skipped (active IME is not GJI)");
                }
            }
            ImeReadStrategy::OsPoll => {
                let miss_before = self.platform_state.ime.detect_miss_count();
                self.ir_poll_and_learn(miss_before, ime_snap);
            }
        }

        // Phase 3.7: 診断スナップショット（フォーカス変更確定直後）
        if focus.focus_changed {
            self.ir_post_focus_change_snapshot(focus.skip_imm_query);
        }
    }

    // ── Stage 4: Engine 通知と次回スケジュール ──
    //
    // Phase 4: Engine に RefreshState（active 遷移検知）
    // Phase 5: 次回ポーリングをスケジュール

    fn ir_stage_notify(&mut self) {
        // Phase 4a: IMM-broken アプリの force-ON（Blacklist パス専用）
        self.apply_force_on_for_imm_broken();
        // Phase 4: Engine に RefreshState（active 遷移検知）
        self.ir_notify_engine_refresh();
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
            log::info!("[shift-conv-guard] FocusChanged 中 → 半角英数トグルを強制解除");
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
                log::info!(
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
                log::info!("FocusChanged: input_mode スキップ (belief=ObservedEisu, eisu guard)");
            }
        }
        let ctx = self.build_ctx();
        let decision = self.engine.on_command(EngineCommand::FocusChanged, &ctx);
        self.execute_decision_suppressed(decision);
    }

    // ── 読み取り方針の決定 ──
    //
    // 最後のキー活動（物理キー押下 または VK/TSF 出力）から TYPING_IDLE_MS 以内は
    // IMM との SendMessage を一切行わない。

    fn ir_decide_read_strategy(&self, skip_imm_query: bool) -> ImeReadStrategy {
        let last_activity = self.platform_state.gate.last_hook_activity_ms.max(
            crate::tsf::probe_bridge::OUTPUT_GATE
                .last_vk_output_ms
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        let idle_ms = crate::hook::current_tick_ms().saturating_sub(last_activity);
        let is_typing = idle_ms < TYPING_IDLE_MS;

        if is_typing {
            // Ctrl+無変換 等の明示的 IME 操作後、実際に OS 状態が変化したか即時検証する。
            // ImmCross async が "成功" 扱いでも組み合わせ中は IME が閉じないことがあるため、
            // タイピングアイドルガードを回避して OsPoll を先行させる。
            // TsfNative/Blacklist アプリは skip_imm_query=true で弾かれるため対象外。
            let explicit_verify = !skip_imm_query
                && self.platform_state.ime.explicit_intent().is_some()
                && self.platform_state.ime.model().applied
                    != crate::state::ime_model::AppliedImeState::Unknown;
            if !explicit_verify {
                log::debug!("Skipping observer/SSOT write: typing active (idle={idle_ms}ms)");
                return ImeReadStrategy::SkipTyping;
            }
            log::debug!(
                "Explicit intent: bypassing typing-idle guard for IME verify (idle={idle_ms}ms)"
            );
        }

        // Shift conv 安全網のブリップ中、または左Shift単独タップによる半角英数
        // 持続トグル中（`kp_stage_shift_conv_guard`）は OS poll を凍結する。
        // conv=0x00000000 は awase 自身が意図的に設定した状態であり、観測して
        // belief（input_mode=ObservedEisu 等）に反映してはならない。解放時の復元 +
        // 既存の観測経路が事後に整合させる。
        if self.platform_state.gate.half_width_alnum.is_guard_pending()
            || self.platform_state.gate.half_width_alnum.is_toggle_active()
        {
            log::debug!("Skipping observer/SSOT write: shift-conv-guard 中");
            return ImeReadStrategy::SkipTyping;
        }

        if skip_imm_query {
            ImeReadStrategy::Blacklist
        } else {
            ImeReadStrategy::OsPoll
        }
    }

    // ── IME 状態のポーリングと学習 ──

    fn ir_poll_and_learn(&mut self, miss_before: u32, ime_snap: Option<&crate::ime::ImeSnapshot>) {
        let poll = self.platform_state.ime.capture_poll_state();
        let ime_on_before_poll = poll.ime_on;
        let input_mode_before_poll = poll.input_mode;

        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        let mut observer_out = ime_snap.map_or_else(
            || unsafe {
                crate::observer::ime_observer::poll_and_classify_ime(
                    poll.ime_on,
                    poll.force_guard,
                    poll.input_mode,
                    poll.prev_conv,
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
        self.try_force_on_bootstrap();
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
                log::info!(
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
                log::debug!(
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
        log::debug!("[composition] focus change → marking cold");

        // `matches!(profile, AppImeProfile::TsfNative)` ではなく `is_effectively_tsf_native`
        // を使うこと。CASCADIA_HOSTING_WINDOW_CLASS (Windows Terminal) 等は
        // `AppImeProfile::from_class_name` の優先順位により `Imm32Unavailable` に分類され
        // `TsfNative` には決してならないため、直接比較だと誤って「非 TSF ネイティブ」と
        // 判定してしまう（2026-07-05: これが原因で enforce IME OFF ブロックが
        // Windows Terminal に対して誤発火していた）。ADR-098 決定1-a のために
        // 算出位置を mirror 書き込みより前へ移した。
        let new_profile_is_tsf_native = crate::focus::class_names::is_effectively_tsf_native(
            self.platform.current_app_profile(),
            self.platform.focus.class_name(),
        );

        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        // ADR-098 決定1-a（BUG-69 F2 の修正）: TsfNative では `applied` を
        // `Unknown` のまま維持する（`focus_tracking.rs` の hard pre-sync が
        // 非 TsfNative について既に守っている不変条件——INV-A97-1——を
        // ここでも適用する）。何も apply していないのに belief を `applied
        // = Confirmed` として書くと、`apply_force_on_for_imm_broken` の
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

        let applied_ime_on = self
            .platform_state
            .ime
            .model()
            .applied
            .applied_open()
            .unwrap_or(false);

        // ADR-098 決定2: 旧 TsfNative force-on ブロック（GJI VK_IME_ON を
        // shadow_on 無視で強制送信）はここに存在した。決定1-a が `applied` を
        // 偽装しなくなったことで、通常の strategy chain（`shadow_on=false`
        // になる）と、決定1-c で有界化された `apply_force_on_for_imm_broken`
        // の両方が正しく VK_IME_ON を送れるようになったため撤去した。撤去の
        // 詳細な根拠は known-bugs.md BUG-69 / ADR-098 決定2 参照。

        // ADR-098 決定1-b: `applied.applied_open()` の生値ではなく `warmup_ime_on()`
        // （`applied ?? belief`）を使う。決定1-a により TsfNative では `applied`
        // が `Unknown` のまま残るため、生値のままだと `unwrap_or(false)` で
        // warmup が握り潰され BUG-02 のリテラル化が再燃する。
        let warmup_ime_on = self
            .platform_state
            .ime
            .warmup_ime_on(std::time::Instant::now());
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
        self.platform.send_eager_warmup(warmup_ime_on);
        log::debug!(
            "[composition] FocusChange: send_eager_tsf_warmup called (ime_on via warmup_ime_on())"
        );

        if !applied_ime_on && !new_profile_is_tsf_native {
            // ADR-090 §2.A 設計案 3: トレイトメソッド `set_ime_open` には引数を
            // 足せないため inherent な `set_ime_open_ordered` へ移した。
            let order = self.issue_actuation_order(false, "focus_change_enforce_off");
            let _ = self.platform.set_ime_open_ordered(order);
            log::debug!("[composition] FocusChange: set_ime_open(false) called (applied_open OFF → enforce IME OFF on new window)");
        }
    }

    // ── ドリフト補正 ──
    //
    // desired ≠ observed が DRIFT_CORRECTION_THRESHOLD_MS 以上続いた場合、再送する。
    //
    // - IMM32 クロスプロセス対応アプリ（LINE 等 ImmCross）: set_ime_open(desired) を使う。
    // - non-ImmCross（GJI/TsfNative/Blacklist、Chrome/Windows Terminal 等）:
    //   set_ime_open は can_use_imm32_cross_process=false で no-op になるため使えない。
    //   apply_force_on_for_imm_broken は ON 方向専用で OFF 方向の乖離は担当しないため、
    //   ここで strategy chain 経由の apply_ime_open_with_belief（実 VK 送信）を使う
    //   （2026-07-08 実機: Windows Terminal/Chrome + GJI で IME OFF コンボ送信後、
    //   Engine 内部は即 OFF になるが OS 側 IME は ON のまま固定される不具合。
    //   set_ime_open の戻り値を見ずに mirror_applied_open_with_ts で belief だけ
    //   「反映済み」にしていたため、実際には一切再送されていなかった。詳細は
    //   docs/known-bugs.md BUG-20 を参照）。

    fn ir_check_drift_correction(&self, now: std::time::Instant) -> Option<(bool, bool, u64)> {
        let explicit_intent = self.platform_state.ime.explicit_intent();
        self.platform_state
            .ime
            .check_drift_correction(now, explicit_intent)
    }

    fn ir_apply_drift_correction(&mut self) {
        // BUG-20 で non-ImmCross（GJI/TsfNative/Blacklist）向けの再送分岐を追加した際、
        // この関数冒頭に残っていた `ir_resolve_skip_imm_query()`（=
        // `!can_use_imm32_cross_process()`）による早期 return を消し忘れていた。
        // 追加した non-ImmCross 分岐はまさにこのガードが true になる場合に実行される
        // はずのコードであり、ガードが残っていたことで一度も到達できない dead code に
        // なっていた（BUG-20 の「実機検証は未実施」という注記通り、実機で一度も
        // 検証されないまま今日まで放置されていた）。詳細は known-bugs.md BUG-20 追補参照。
        if !self.engine.is_user_enabled() || !self.platform_state.ime.belief.is_japanese_ime() {
            return;
        }

        let now = std::time::Instant::now();
        let Some((desired, observed, duration_ms)) = self.ir_check_drift_correction(now) else {
            return;
        };
        if self.ime_apply_should_defer() {
            // apply_force_on_for_imm_broken と同じく settle 明けに必ず再試行する。
            self.schedule_settle_retry(&format!(
                "drift correction skipped (settling): desired={desired} observed={observed}"
            ));
            return;
        }

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
        // なお `ir_check_drift_correction`（=`check_drift_correction`）の乖離「検知」側は
        // ADR-080 Phase 1 では従来どおり `most_recent_trusted`（since フェンシングなし）を
        // 使い続ける。since フェンシング（`most_recent_trusted_after`）を使うのは下の
        // `Read` 収束「確認」側のみで、この非対称は ADR-080 が意図的に許容している。
        let policy = self.platform_state.ime.default_feedback();
        let (act_policy, act_attempts, act_sent_at, act_gave_up_at, act_origin) = {
            let actuation = self.actuation_for(desired, policy);
            (
                actuation.policy,
                actuation.attempts,
                actuation.sent_at,
                actuation.gave_up_at,
                actuation.origin,
            )
        };

        self.ir_notify_drift_giveup_diagnostic(desired, observed, duration_ms, now);

        match act_policy {
            FeedbackPolicy::Blind { .. } => {
                let action = decide_actuation_action(act_policy, act_attempts);
                if action == ActuationAction::GiveUp {
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
                    // 復旧判定は観測の「値」ではなく「鮮度」で行う。drift 補正は
                    // observed != desired（乖離）が続く間しか走らず、open/close は bool の
                    // ため「間違った値」は !desired の1通りしか存在しない。よって ADR 当初の
                    // 文言「target と異なる値の観測が来たら復旧」はほぼ毎 tick 真になり
                    // GiveUp を即座に無効化してしまう（乖離の定義そのものだから）。意味の
                    // ある信号は「諦めた時刻以降に新しい観測が record されたか」＝世界で
                    // 何かが動いたか（値は問わない）であり、`most_recent_trusted_after` が
                    // まさにそれを判定する。
                    match act_gave_up_at {
                        None => {
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
                            log::debug!(
                                "[drift] actuation gave up (Blind): desired={desired} \
                                 observed={observed} converged={} attempts={}",
                                receipt.converged(),
                                receipt.attempts()
                            );
                        }
                        Some(gave_up_at) => {
                            // BUG-68: 再武装判定（`AnyFreshEvidence`）は「gave_up_at 以降に
                            // 新しい信頼できる観測が record されたか」（値は不問）だけを見る。
                            // MS-IME × TsfNative では `kp_stage_idle_conv_check` は毎打鍵では
                            // なく `should_run_idle_conv_check`（`src/engine/idle_check.rs`）の
                            // ガード3（awase 自身の最終出力から500ms超）を通過した打鍵でのみ
                            // 実行されるが、drift correction 自身の `VK_IME_OFF` 再送も
                            // その「最終出力」に数えられるため、give-up バーストのたびに
                            // 短時間で次の idle-conv-check が走る。そのたびに同じ
                            // `ConvOpenInference` 観測（IMM32 の NATIVE ビットは開閉状態と
                            // 無関係な持続的な変換モード設定で、`VK_IME_OFF` で閉じても
                            // 消えない）を新しいタイムスタンプで record するため、「鮮度」が
                            // 「新情報」の代理指標として機能せず、gave_up_at を刻んだ直後には
                            // もう「新しい」観測が存在し即座に再武装してしまう（実機ログ、
                            // docs/known-bugs.md BUG-68、tuning.rs の定数コメント参照）。
                            // クールダウン未経過の間は `read_back` 自体を評価しない
                            // （＝再武装しない）ことで、この短周期ループを防ぐ。
                            // クールダウンを空けているだけなので、BUG-51（明示 OFF 後も
                            // 実 IME が閉じないケース）の「いずれ回復する」性質は保たれる。
                            if !crate::state::ime_actuation::blind_rearm_cooldown_elapsed(
                                gave_up_at,
                                now,
                                crate::tuning::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS,
                            ) {
                                return;
                            }
                            // 既に parked。gave_up_at 以降に新しい trusted 観測が record
                            // されていれば（値は不問＝外部で状況が動いた証拠）、試行を破棄
                            // して次 tick の `actuation_for` に attempts=0・新しい sent_at・
                            // gave_up_at=None で作り直させる。実際の再送は次 tick に任せ、
                            // discard した同じ tick では送らない（ロジックを単純に保つ）。
                            //
                            // ADR-090 §2.B（INV-52）: 読み戻しは
                            // `ObservationStore::read_back` の 1 本だけを通る。
                            // 戻り値は `ConvergedReceipt` であって
                            // `ImeObservation` ではないので、復旧判定に使った
                            // 読み取りの産物を観測として書き戻すことが型として
                            // 書けない。述語（`.is_some()`）はそのまま
                            // `ReadBackQuery::AnyFreshEvidence` の中へ移した
                            // だけで、判定は bit-identical である。
                            let receipt = self.platform_state.ime.model().observations.read_back(
                                now,
                                gave_up_at,
                                crate::state::observation_store::ReadBackQuery::AnyFreshEvidence,
                                act_attempts,
                            );
                            if receipt.resolution()
                                == crate::state::ime_actuation::Resolution::ExternalChange
                            {
                                log::debug!(
                                    "[drift] fresh observation after give-up → 試行を破棄して\
                                     再試行: desired={desired} observed={observed} attempts={}",
                                    receipt.attempts()
                                );
                                self.discard_actuation();
                            }
                        }
                    }
                    return;
                }
            }
            FeedbackPolicy::Read { .. } => {
                // `sent_at` 以降の trusted 観測が desired と一致していれば収束済み
                // （`Resolution::Confirmed`）。再送不要なので試行を破棄する。
                //
                // ADR-089 §2.5（INV-46）: 収束の帰結は `ConvergedReceipt`。
                // 観測ストアへは何も書かない（`Confirmed` は「既に観測が
                // desired と一致していた」という読み取りの帰結であって、
                // 新しい観測ではない）。
                // ADR-090 §2.B（INV-52）: その receipt を**読み戻し API から
                // 直接受け取る**形にした。以前は `most_recent_trusted_after` が
                // 返す `ImeObservation` で判定してから receipt を別途組み立てて
                // いたため、receipt は log にしか効いていなかった（§9-16）。
                // 述語（`.is_some_and(|o| o.open == desired)`）はそのまま
                // `ReadBackQuery::Converged` の中へ移しただけで bit-identical。
                let receipt = self.platform_state.ime.model().observations.read_back(
                    now,
                    act_sent_at,
                    crate::state::observation_store::ReadBackQuery::Converged { desired },
                    act_attempts,
                );
                if receipt.converged() {
                    log::debug!(
                        "[drift] actuation confirmed (Read): desired={desired} \
                         converged={} attempts={} → 破棄",
                        receipt.converged(),
                        receipt.attempts()
                    );
                    self.discard_actuation();
                    return;
                }
            }
        }

        log::warn!(
            "[drift] correction: observed={observed} ≠ desired={desired} for {duration_ms}ms \
             → set_ime_open({desired})"
        );
        // ADR-082 Phase 0.5: 実送信する試行を出所・世代付きで構造化記録する。
        // `Blind` はここに到達する時点で必ず `Send`（`GiveUp` は上で return 済み）、
        // `Read` は常に `Send`。`action` は `ActuationRecord::new` が
        // `decide_actuation_action` で導出する。
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
        if self.can_use_imm32_cross_process() {
            // ADR-090 §2.A 設計案 3 / A-1（shadow）。drift correction は既に
            // `EventOrigin`（`act_origin`）を持っているので、それをそのまま
            // order の出所として使う（journal の `ImeActuation` と揃う）。
            let order = self.issue_actuation_order_with_origin(desired, act_origin);
            let _ = self.platform.set_ime_open_ordered(order);
            self.platform_state.ime.record_optimistic(desired);
        } else {
            // set_ime_open は IMM32専用で Blacklist/TsfNative では no-op のため、
            // apply_force_on_for_imm_broken と同じ strategy chain 経由の実送信を使う。
            let belief = crate::output::OpenBelief {
                effective_open: desired,
                confident: true,
            };
            let order = self.issue_actuation_order_with_origin(desired, act_origin);
            let outcome = self
                .platform
                .apply_ime_open_with_belief(order, None, belief);
            log::info!("Blacklist drift correction: apply_ime_open({desired}) → {outcome:?}");
            self.on_ime_apply_complete(
                desired,
                outcome,
                None,
                crate::state::ime_event::OpenApplyReason::DriftCorrection,
            );
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
        if duration_ms < crate::tuning::DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS
            || self.drift_giveup_notified_this_focus
        {
            return;
        }
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

    fn ir_notify_engine_refresh(&mut self) {
        let ctx = self.build_ctx();
        log::debug!(
            "[notify-refresh] ctx.ime_on={} ctx.is_jp={} explicit_intent={:?}",
            ctx.ime_on,
            ctx.is_japanese_ime,
            self.platform_state.ime.explicit_intent(),
        );
        let decision = self.engine.on_command(EngineCommand::RefreshState, &ctx);
        self.execute_decision_suppressed(decision);
    }
}
