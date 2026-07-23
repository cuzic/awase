use awase::engine::{ConvMode, EngineCommand, InputModeState};
use awase::platform::PlatformRuntime;

use super::Runtime;
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
                            self.platform_state.focus.focus_epoch,
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
        // 左Shift単独タップによる「IME-ON 半角英数」持続トグル中にフォーカスが
        // 変わった場合、半角英数状態を他アプリへ持ち越さないよう即座にかな入力へ
        // 復元する（呼び出し自体を遅延させないという意味で「即座」。復元処理自体は
        // 既存同様 spawn_local 経由の非同期 retry ループを含むため、この呼び出しが
        // フォーカス変更処理をブロックすることはない）。物理 Shift が押されている
        // とは限らないため synthetic Shift up の前置は不要（false）。
        if self.platform_state.gate.half_width_alnum_toggle_active {
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
                self.platform_state.ime.dispatch_event(
                    crate::state::ime_event::ImeEvent::InputModeApplied {
                        mode: new_mode,
                        strategy:
                            crate::state::ime_event::InputModeApplyStrategy::ImmBrokenCorrection,
                        result: crate::state::ime_event::InputModeApplyResult::Applied,
                        at: tick_ms,
                    },
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
        if self.platform_state.gate.shift_conv_guard_pending
            || self.platform_state.gate.half_width_alnum_toggle_active
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
        let accepted = crate::state::probe_admission::AcceptedObservation::for_sync(
            self.platform_state.focus.focus_epoch,
        );
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
        let ime_on_now = self.platform_state.ime.effective_open();
        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        self.platform_state
            .ime
            .mirror_applied_open(ime_on_now, tick_ms);
        self.platform.mark_composition_cold_focus_change();
        let mode = self.platform.output.injection_mode;
        self.platform.gji_on_focus_change(mode);

        // `matches!(profile, AppImeProfile::TsfNative)` ではなく `is_effectively_tsf_native`
        // を使うこと。CASCADIA_HOSTING_WINDOW_CLASS (Windows Terminal) 等は
        // `AppImeProfile::from_class_name` の優先順位により `Imm32Unavailable` に分類され
        // `TsfNative` には決してならないため、直接比較だと誤って「非 TSF ネイティブ」と
        // 判定してしまう（2026-07-05: これが原因で下の「enforce IME OFF」ブロックが
        // Windows Terminal に対して誤発火していた）。
        let new_profile_is_tsf_native = crate::focus::class_names::is_effectively_tsf_native(
            self.platform.current_app_profile(),
            self.platform.focus.class_name(),
        );
        let applied_ime_on = self
            .platform_state
            .ime
            .model()
            .applied
            .applied_open()
            .unwrap_or(false);

        // TsfNative (WezTerm 等) + IME ON でフォーカス入場: GJI VK_IME_ON を shadow_on なしで強制送信。
        //
        // 通常の GjiDirectStrategy は shadow_on=true を見て「既に ON」と判断し VK_IME_ON をスキップする。
        // しかしフォーカス変更時の shadow_on は直前ウィンドウ（Chrome 等）の applied 値が
        // hard pre-sync で引き継がれており、WezTerm の実 GJI IME が OFF でも VK_IME_ON が送られない。
        // apply_ime_open_with_applied(true, None) で shadow_on=∅(false) にして VK_IME_ON を確実に送る。
        // VK_IME_ON は GJI が既に ON の場合も no-op（冪等）なので副作用なし。
        // GJI 未使用環境（MS-IME + TsfNative）で KanjiToggle が誤送信されないよう GJI ガードを設ける。
        //
        // ガードは gji_monitor_ok（GJI プロセスの生存）ではなく active_ime_kind（CLSID ベースの
        // 実際のアクティブ IME 判定）を見ること。GJI プロセスは他ウィンドウ（例: msedge）が
        // 使用中で生きているだけのことがあり、その場合 gji_monitor_ok=true でもこのウィンドウの
        // 実際の IME は MS-IME であり得る。gji_monitor_ok だけで判定すると、通常の
        // belief 駆動 apply-ime（MsImeDirectStrategy）に続けて VK_DBE_HIRAGANA が二重送信され、
        // TSF アクティベーション中の conv mode 破壊（roma→kana 化け）を誘発し得る。
        // 2026-07-05: is_effectively_tsf_native への修正で XamlExplorerHostIslandWindow
        // (Alt+Tab スイッチャーの中間ウィンドウ、is_tsf_native_window に該当) も true になる
        // ようになったため、settle 期間中はこのブロック自体もフィルタする。
        if applied_ime_on && new_profile_is_tsf_native && !self.ime_apply_should_defer() {
            let obs = crate::state::ObservedState::from_snapshot(crate::tsf::observer::tsf_obs());
            if obs.gji_monitor_ok
                && obs.active_ime_kind == crate::tsf::observer::ActiveImeKind::GoogleJapaneseInput
            {
                let _ = self.platform.apply_ime_open_with_applied(true, None);
                log::debug!(
                    "[composition] FocusChange: TsfNative IME ON → GJI VK_IME_ON 強制 (shadow_on を無視)"
                );
            }
        }

        let applied_open = self.platform_state.ime.model().applied.applied_open();
        // tray で英数／カタカナ等に切り替えた直後の conv を読む。
        // 英数モード (is_eisu) なら warmup をスキップする:
        //   NATIVE=0 のまま VK_DBE_HIRAGANA を送るとひらがなモードに戻ってしまうため。
        // 旧 eisu_guard は conv=0x0000 のみを対象としていたが、MS-IME は 0x0010 (ROMAN=1,NATIVE=0)
        // を返すことがあるため is_eisu() に統一する。
        let focus_change_conv = unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(10) };
        if let Some(conv) = focus_change_conv {
            let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
            self.platform
                .output
                .conv_mode
                .update_from_conv(conv, tick_ms);
        }
        let eisu_guard_active = applied_open == Some(true)
            && focus_change_conv.is_some_and(|conv| ConvMode::from_u32(conv).is_eisu());
        if eisu_guard_active {
            log::info!(
                "[composition] FocusChange: applied_open=true だが conv=英数 → warmup スキップ (tray 半角英数 保護)"
            );
        } else {
            self.platform.send_eager_warmup(applied_open);
            log::debug!(
                "[composition] FocusChange: send_eager_tsf_warmup called (guarded by applied_open)"
            );
        }

        if !applied_ime_on && !new_profile_is_tsf_native {
            let _ = self.platform.set_ime_open(false);
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
            let retry_ms = self.platform_state.ime.focus_settle_ms() + 50;
            log::debug!(
                "[focus-settle] drift correction skipped (settling): desired={desired} \
                 observed={observed} → {retry_ms}ms 後に refresh で再試行"
            );
            self.schedule_ime_refresh(retry_ms);
            return;
        }

        log::warn!(
            "[drift] correction: observed={observed} ≠ desired={desired} for {duration_ms}ms \
             → set_ime_open({desired})"
        );
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
            let _ = self.platform.set_ime_open(desired);
            self.platform_state
                .ime
                .mirror_applied_open_with_ts(desired, 0);
        } else {
            // set_ime_open は IMM32専用で Blacklist/TsfNative では no-op のため、
            // apply_force_on_for_imm_broken と同じ strategy chain 経由の実送信を使う。
            let belief = crate::output::OpenBelief {
                effective_open: desired,
                confident: true,
            };
            let outcome = self
                .platform
                .apply_ime_open_with_belief(desired, None, belief);
            log::info!("Blacklist drift correction: apply_ime_open({desired}) → {outcome:?}");
            self.on_ime_apply_complete(desired, outcome, None);
        }
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
