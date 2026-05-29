use awase::engine::{AssumedReason, EngineCommand, InputModeState};
use awase::platform::PlatformRuntime;

use crate::focus::classifier::ImmCapability;
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

/// stage_focus() の戻り値: フォーカス検出結果
struct FocusInfo {
    focus_changed: bool,
    skip_imm_query: bool,
}

// ── ImeRefreshPipeline ──

pub(super) struct ImeRefreshPipeline<'a> {
    rt: &'a mut Runtime,
}

impl<'a> ImeRefreshPipeline<'a> {
    pub(super) const fn new(rt: &'a mut Runtime) -> Self {
        Self { rt }
    }

    pub(super) fn run(self) {
        self.execute(IoMode::Sync);
    }

    /// pre-fetch 済みデータを使ってパイプラインを実行（blocking なし）。
    /// spawn_local タスクから呼ぶ。
    pub(super) fn run_with_prefetched(
        self,
        focus_probe: Option<crate::focus::probe::FocusSnapshot>,
        ime_snap: &crate::ime::ImeSnapshot,
    ) {
        self.execute(IoMode::Prefetched { focus: focus_probe, ime: ime_snap });
    }

    fn execute(mut self, mode: IoMode<'_>) {
        // IoMode を分解して各ステージに必要な型として渡す。
        // focus_probe: None = 同期取得、Some(x) = pre-fetch 済み
        // ime_snap:    None = 同期ポーリング、Some(s) = pre-fetch 済みスナップショット
        let (focus_probe, ime_snap) = match mode {
            IoMode::Sync => (None, None),
            IoMode::Prefetched { focus, ime } => (Some(focus), Some(ime)),
        };
        let focus = self.stage_focus(focus_probe);
        let strategy = self.stage_strategy(&focus);
        self.stage_observe(&focus, &strategy, ime_snap);
        self.stage_notify();
    }

    // ── Stage 1: フォーカス検出 ──
    //
    // Phase 1: フォーカス先の検出・分類
    // Phase 2.5: IMM ブリッジ非対応クラスの判定（Phase 2 の前に実行する必要あり）
    // Phase 2: プロセス変更時は Engine に FocusChanged（flush あり）

    fn stage_focus(
        &mut self,
        focus_probe: Option<Option<crate::focus::probe::FocusSnapshot>>,
    ) -> FocusInfo {
        // Phase 1: フォーカス先の検出・分類
        let focus_changed = match focus_probe {
            None => unsafe { self.rt.detect_and_update_focus() },
            Some(probe) => self.rt.apply_focus_probe_result(probe),
        };

        // Phase 2.5: IMM ブリッジ非対応クラスの判定
        //
        // Chrome / UWP / Electron 等はクロスプロセス IMM 問い合わせ（WM_IME_CONTROL）が
        // 動作しないか、無期限ブロックする恐れがある。既知のクラス名なら事前にスキップし、
        // シャドウ状態（hook から追跡）のみで IME 状態を管理する。
        //
        // FocusChanged で build_ctx() が呼ばれる際、input_mode が stale な ObservedKana だと
        // engine が inactive になってしまうため、先に補正する。
        let skip_imm_query = self.resolve_skip_imm_query();

        // Phase 2: プロセス変更時は Engine に FocusChanged（flush あり）
        if focus_changed {
            self.notify_focus_changed(skip_imm_query);
        }

        FocusInfo { focus_changed, skip_imm_query }
    }

    // ── Stage 2: 読み取り方針の決定 ──

    fn stage_strategy(&self, focus: &FocusInfo) -> ImeReadStrategy {
        self.decide_read_strategy(focus.skip_imm_query)
    }

    // ── Stage 3: IME 状態の観測 ──
    //
    // Phase 3: IME 状態の再取得
    // Phase 3.1: IMM 能力の学習
    // Phase 3.5: 未知 Imm32Unavailable アプリ向け一時 force-ON（初回ブートストラップ）
    // Phase 3.7: 診断スナップショット（フォーカス変更後）

    fn stage_observe(
        &mut self,
        focus: &FocusInfo,
        strategy: &ImeReadStrategy,
        ime_snap: Option<&crate::ime::ImeSnapshot>,
    ) {
        log::debug!(
            "[stage-observe] strategy={:?} belief_on={} explicit_intent={:?}",
            strategy,
            self.rt.platform_state.ime_on(),
            self.rt.platform_state.explicit_intent(),
        );
        match strategy {
            ImeReadStrategy::SkipTyping => {
                // タイピング中は何もしない
            }
            ImeReadStrategy::Blacklist => {
                log::debug!("Skipping IMM query for known-broken class (shadow state SSOT)");
                let obs = crate::observer::gji_observer::observe_gji_after_focus(
                    self.rt.platform_state.focus.last_focus_change_ms,
                );
                log::debug!(
                    "[stage-observe] gji_result={:?}",
                    obs.observer_poll_value
                );
                if let Some(v) = obs.observer_poll_value {
                    self.rt.platform_state.write_observer_poll(
                        v, obs.now_ms, self.rt.engine.is_user_enabled(),
                    );
                }
            }
            ImeReadStrategy::OsPoll => {
                // Phase 3: IME 状態の再取得
                let miss_before = self.rt.platform_state.ime_detect_miss_count();
                self.poll_and_learn(miss_before, ime_snap);
            }
        }

        // Phase 3.7: 診断スナップショット（フォーカス変更確定直後）
        if focus.focus_changed {
            self.post_focus_change_snapshot(focus.skip_imm_query);
        }
    }

    // ── Stage 4: Engine 通知と次回スケジュール ──
    //
    // Phase 4: Engine に RefreshState（active 遷移検知）
    // Phase 5: 次回ポーリングをスケジュール

    fn stage_notify(&mut self) {
        // Phase 4a: IMM-broken アプリの force-ON（Blacklist パス専用）
        self.apply_force_on_for_imm_broken();
        // Phase 4: Engine に RefreshState（active 遷移検知）
        self.notify_engine_refresh();

        // Phase 5: 次回ポーリングをスケジュール
        self.reschedule();
    }

    // ── IMM ブリッジ非対応クラスの判定 ──

    fn resolve_skip_imm_query(&self) -> bool {
        // IMM32 クロスプロセス制御が使えないアプリ（Imm32Unavailable / TsfNative）では
        // クロスプロセス IMM 問い合わせ（WM_IME_CONTROL）が動作しないか、
        // 無期限ブロックする恐れがあるためスキップする。
        !self
            .rt
            .executor
            .platform
            .focus
            .current_app_profile()
            .can_use_imm32_cross_process()
    }

    // ── フォーカス変更通知 ──

    fn notify_focus_changed(&mut self, skip_imm_query: bool) {
        // IMM broken アプリ（Chrome 等）に切り替わった際に input_mode が
        // 前ウィンドウの stale な ObservedKana を引き継いでいると、FocusChanged の ctx で
        // engine が inactive になる。broken アプリでは入力モードを検出できないため、
        // ime_on=true のとき AssumedRomaji と仮定して補正する。
        if skip_imm_query
            && self.rt.platform_state.ime_on()
            && !self.rt.platform_state.input_mode().is_romaji_capable()
        {
            log::info!(
                "FocusChanged: input_mode assumed romaji (IMM broken, stale kana from prev window)"
            );
            self.rt.platform_state.set_input_mode(
                InputModeState::AssumedRomaji { reason: AssumedReason::ImmBridgeBroken }
            );
        }
        let ctx = self.rt.build_ctx();
        let decision = self.rt.engine.on_command(EngineCommand::FocusChanged, &ctx);
        // フォーカス変化起因の状態遷移では engine_state_ime_key を送らない（フィードバックループ防止）。
        self.rt.executor.platform.suppress_engine_state_key = true;
        self.rt.execute_decision(decision);
        self.rt.executor.platform.suppress_engine_state_key = false;
    }

    // ── 読み取り方針の決定 ──
    //
    // 最後のキー活動（物理キー押下 または VK/TSF 出力）から TYPING_IDLE_MS 以内は
    // IMM との SendMessage を一切行わない。

    fn decide_read_strategy(&self, skip_imm_query: bool) -> ImeReadStrategy {
        let last_activity = self.rt.platform_state.last_hook_activity_ms
            .max(crate::tsf::probe_bridge::OUTPUT_GATE.last_vk_output_ms.load(std::sync::atomic::Ordering::Relaxed));
        let idle_ms = crate::hook::current_tick_ms()
            .saturating_sub(last_activity);
        let is_typing = idle_ms < TYPING_IDLE_MS;

        if is_typing {
            log::debug!("Skipping observer/SSOT write: typing active (idle={idle_ms}ms)");
            ImeReadStrategy::SkipTyping
        } else if skip_imm_query {
            ImeReadStrategy::Blacklist
        } else {
            ImeReadStrategy::OsPoll
        }
    }

    // ── IME 状態のポーリングと学習 ──

    fn poll_and_learn(&mut self, miss_before: u32, ime_snap: Option<&crate::ime::ImeSnapshot>) {
        // [診断] observe 前のスナップショット（差分検出用）
        let ime_on_before_poll = self.rt.platform_state.ime_on();
        let input_mode_before_poll = self.rt.platform_state.input_mode();

        let observer_out = match ime_snap {
            None => unsafe {
                crate::observer::ime_observer::poll_and_classify_ime(
                    self.rt.platform_state.ime_on(),
                    self.rt.platform_state.is_force_on_guard_active(),
                    self.rt.platform_state.input_mode(),
                    self.rt.platform_state.prev_conversion_mode(),
                )
            },
            Some(snap) => {
                let now_ms = crate::hook::current_tick_ms();
                crate::observer::ime_observer::classify_fetched_snapshot(
                    snap,
                    now_ms,
                    self.rt.platform_state.ime_on(),
                    self.rt.platform_state.is_force_on_guard_active(),
                    self.rt.platform_state.input_mode(),
                    self.rt.platform_state.prev_conversion_mode(),
                )
            }
        };
        self.rt.platform_state.apply_ime_update(&observer_out, self.rt.engine.is_user_enabled());

        let miss_after = self.rt.platform_state.ime_detect_miss_count();

        self.log_poll_diff(
            ime_on_before_poll,
            input_mode_before_poll,
            miss_before,
            miss_after,
        );

        // IMM 能力の学習
        self.learn_imm_capability_from_result(miss_before, miss_after);

        // 未知 Imm32Unavailable アプリ向け一時 force-ON（初回ブートストラップ）
        self.try_force_on_bootstrap();
    }

    /// [診断] フォーカス変更から 10 秒以内で状態が変わった場合にログ出力。
    fn log_poll_diff(
        &self,
        ime_on_before_poll: bool,
        input_mode_before_poll: InputModeState,
        miss_before: u32,
        miss_after: u32,
    ) {
        let age_ms = crate::hook::current_tick_ms()
            .saturating_sub(self.rt.platform_state.focus.last_focus_change_ms);
        if age_ms < 10_000 {
            let ime_on_after = self.rt.platform_state.ime_on();
            let input_mode_after = self.rt.platform_state.input_mode();
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
                            self.rt.platform_state.explicit_intent(),
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
        let _ = miss_before; // suppress unused warning if logging is compiled out
    }

    /// 検出結果に基づいて class_name ごとの IMM 能力をキャッシュ。
    fn learn_imm_capability_from_result(&mut self, miss_before: u32, miss_after: u32) {
        if let Some((_, class_name)) = self.rt.executor.platform.focus.last_focus_info.as_ref() {
            let class_name = class_name.clone();
            if miss_after == 0 && miss_before > 0 {
                // 検出成功: IMM ブリッジが動作している
                let prev = self
                    .rt
                    .executor
                    .platform
                    .focus
                    .imm_learning
                    .get(&class_name);
                if prev != Some(ImmCapability::Works) {
                    log::info!("IMM capability learned: {class_name} → Works (detection succeeded)");
                    self.rt
                        .executor
                        .platform
                        .focus
                        .learn_imm_capability(class_name, ImmCapability::Works);
                }
            } else if miss_after >= crate::IME_DETECT_MISS_THRESHOLD
                && miss_before < crate::IME_DETECT_MISS_THRESHOLD
            {
                // 閾値到達: IMM ブリッジが壊れている
                let prev = self
                    .rt
                    .executor
                    .platform
                    .focus
                    .imm_learning
                    .get(&class_name);
                if prev != Some(ImmCapability::Unavailable) {
                    log::info!(
                        "IMM32 capability learned: {class_name} → Unavailable (detection failed {miss_after} times)"
                    );
                    self.rt
                        .executor
                        .platform
                        .focus
                        .learn_imm_capability(class_name, ImmCapability::Unavailable);
                }
            }
        }
    }

    /// 未知 Imm32Unavailable アプリ向け一時 force-ON（初回ブートストラップ）
    ///
    // ここに来るのは「既知でも TSF-native でもないアプリで detect が連続失敗した」
    // 場合だけ。shadow=ON なら SetOpen(true) を呼び engine を active のまま維持する。
    fn try_force_on_bootstrap(&mut self) {
        if self.rt.platform_state.ime_detect_miss_count()
            >= crate::IME_DETECT_MISS_THRESHOLD
            && self.rt.engine.is_user_enabled()
            && self.rt.platform_state.is_japanese_ime()
            && self.rt.platform_state.ime_on()
            && !self.rt.platform_state.is_force_on_guard_active()
        {
            log::warn!(
                "IME detection failed {} times, forcing OS ime_on=true (shadow=ON)",
                self.rt.platform_state.ime_detect_miss_count()
            );
            let dispatched = self.rt.executor.platform.set_ime_open(true);
            // dispatched=true は IMM クロスプロセス対応アプリで async ジョブを起動した意味。
            // 実 SendMessage の成否は spawn_local 内に閉じる。
            // IME 非対応ウィンドウ(DirectUIHWND 等) で失敗し続ける無限ループを防ぐため、
            // 結果に関わらずガードをセットする（フォーカス変更時に解除）。
            self.rt.platform_state.set_force_on_broken_app_bootstrap();
            if !dispatched {
                log::warn!("set_ime_open dispatched=false (profile not IMM-capable) — guard set to suppress retry until focus change");
            }
        }
    }

    // ── 診断スナップショット（フォーカス変更確定直後）──
    //
    // フォーカス変更が確定した直後の IME 状態を 1 行ログに吐き出す。
    // ウィンドウ切替直後の cold-start 不具合を解析するための観測点。

    fn post_focus_change_snapshot(&mut self, skip_imm_query: bool) {
        // IMM ブリッジ非対応クラスでは capture_imc / get_gui_thread_info がタイムアウト
        // して ~150ms ブロックするため診断をスキップする。
        if !skip_imm_query {
            crate::ime_diagnostic::ImeDiagnosticSnapshot::capture("focus_changed").log();
        }
        // フォーカス変更時は VK/TSF いずれも composition context が無効化される。
        log::debug!("[composition] focus change → marking cold");
        // フォーカス変更直後の IMM 実測値で last_applied ログを初期化する。
        // これにより KanjiToggleStrategy が次回 apply_ime_open を呼ぶときに
        // belief の最新値と比較して重複送信を回避できる。
        let ime_on_now = self.rt.platform_state.ime_on();
        self.rt.platform_state.ime.mirror_applied_open(ime_on_now);
        self.rt
            .executor
            .platform
            .output
            .mark_composition_cold(crate::output::ColdReason::FocusChange);

        // TSF モード（WezTerm 等）かつ IME ON の場合、FocusChange 直後に F2 pre-warmup を送信する。
        self.rt.executor.platform.output.send_eager_tsf_warmup();
        log::debug!(
            "[composition] FocusChange: send_eager_tsf_warmup called (guarded by applied_open)"
        );

        // applied_open=false (or None) の場合、新しいウィンドウの IME を明示的に OFF にする。
        // Ctrl+無変換 は発火時点のウィンドウにしか set_ime_open を送らないため、
        // 別ウィンドウに移動すると IME が ON のままになるのを防ぐ。
        //
        // ただし TsfNative プロファイル（Windows Terminal 等）は IMM クロスプロセス制御が
        // 効かず set_ime_open(false) は no-op だが、shadow ime_on の carry over と相まって
        // Engine が活性化不能の trap に陥る。TsfNative では runtime 側で stale をリセット
        // するためここでは enforce OFF を skip し、新ウィンドウの状態に任せる。
        let new_profile_is_tsf_native = matches!(
            self.rt.executor.platform.focus.current_app_profile(),
            crate::focus::classify::AppImeProfile::TsfNative,
        );
        let applied_ime_on = self.rt.platform_state.ime.shadow_model.applied_open.unwrap_or(false);
        if !applied_ime_on && !new_profile_is_tsf_native {
            let _ = self.rt.executor.platform.set_ime_open(false);
            log::debug!("[composition] FocusChange: set_ime_open(false) called (applied_open OFF → enforce IME OFF on new window)");
        }
    }

    // ── IMM-broken アプリの force-ON（Blacklist パス専用）──
    //
    // OS 側の IME が belief と乖離するのを防ぐ。
    // Blacklist 以外のパスでは skip_imm_query=false なので re_check_predicate が false になる。

    fn apply_force_on_for_imm_broken(&mut self) {
        // Blacklist パス以外は何もしない（条件を再チェックして判断）
        if !self.resolve_skip_imm_query() {
            return;
        }
        if !(self.rt.engine.is_user_enabled()
            && self.rt.platform_state.is_japanese_ime()
            && self.rt.platform_state.ime_on())
        {
            return;
        }
        let _success = self.rt.executor.platform.set_ime_open(true);
        log::trace!("Blacklist force-ON: set_ime_open(true)");
        // input_mode も SSOT として維持: IMM broken アプリでは検出不能のため
        // stale な ObservedKana が engine を無効化しないよう AssumedRomaji に補正する。
        if !self.rt.platform_state.input_mode().is_romaji_capable() {
            log::info!("Blacklist force-ON: input_mode → AssumedRomaji (IMM broken, ime_on=true)");
            self.rt.platform_state.set_input_mode(
                InputModeState::AssumedRomaji { reason: AssumedReason::ImmBridgeBroken }
            );
        }
    }

    // ── Engine 通知 ──

    fn notify_engine_refresh(&mut self) {
        let ctx = self.rt.build_ctx();
        log::debug!(
            "[notify-refresh] ctx.ime_on={} ctx.is_jp={} explicit_intent={:?}",
            ctx.ime_on, ctx.is_japanese_ime,
            self.rt.platform_state.explicit_intent(),
        );
        let decision = self.rt.engine.on_command(EngineCommand::RefreshState, &ctx);
        // ポーリング起因の状態遷移では engine_state_ime_key を送らない（フィードバックループ防止）。
        self.rt.executor.platform.suppress_engine_state_key = true;
        self.rt.execute_decision(decision);
        self.rt.executor.platform.suppress_engine_state_key = false;
    }

    // ── 次回ポーリングのスケジュール ──

    fn reschedule(&mut self) {
        self.rt
            .schedule_ime_refresh(u64::from(self.rt.platform_state.focus.ime_poll_interval_ms));
    }
}
