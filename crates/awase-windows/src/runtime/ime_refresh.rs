use awase::engine::{AssumedReason, EngineCommand, InputModeState};

use super::ImmCapability;
use super::Runtime;

// ── ImeReadStrategy ──

/// Phase 2.7: IME 読み取り方針の決定結果
enum ImeReadStrategy {
    /// タイピング中 — IMM/TSF を一切呼ばない
    SkipTyping,
    /// 既知ブラックリストクラス — shadow SSOT のみ使う
    Blacklist,
    /// OS をポーリングする通常パス
    OsPoll,
}

// ── PipelineState ──

/// `ImeRefreshPipeline::run()` の各フェーズ間で共有する中間状態
struct PipelineState {
    focus_changed: bool,
    skip_imm_query: bool,
    miss_before: u32,
}

// ── ImeRefreshPipeline ──

pub(super) struct ImeRefreshPipeline<'a> {
    rt: &'a mut Runtime,
}

impl<'a> ImeRefreshPipeline<'a> {
    pub(super) fn new(rt: &'a mut Runtime) -> Self {
        Self { rt }
    }

    pub(super) fn run(mut self) {
        let mut state = PipelineState {
            focus_changed: false,
            skip_imm_query: false,
            miss_before: 0,
        };

        // Phase 1: フォーカス先の検出・分類
        self.phase1_detect_focus(&mut state);

        // Phase 2.5: IMM ブリッジ非対応クラスの判定
        state.skip_imm_query = self.phase2_5_resolve_skip_imm_query();

        // Phase 2: プロセス変更時は Engine に FocusChanged（flush あり）
        if state.focus_changed {
            self.phase2_notify_focus_changed(&state);
        }

        // Phase 2.7: 読み取り方針の決定
        let strategy = self.phase2_7_decide_read_strategy(state.skip_imm_query);

        match strategy {
            ImeReadStrategy::SkipTyping => {
                // タイピング中は何もしない
            }
            ImeReadStrategy::Blacklist => {
                self.apply_blacklist_ssot();
            }
            ImeReadStrategy::OsPoll => {
                // Phase 3: IME 状態の再取得
                state.miss_before = self.rt.platform_state.preconditions.ime_detect_miss_count;
                self.phase3_poll_and_learn(&mut state);
            }
        }

        // Phase 3.7: 診断スナップショット（フォーカス変更後）
        if state.focus_changed {
            self.phase3_7_post_focus_change_snapshot(&state);
        }

        // Phase 4: Engine に RefreshState（active 遷移検知）
        self.phase4_notify_engine_refresh();

        // Phase 5: 次回ポーリングをスケジュール
        self.phase5_reschedule();
    }

    // ── Phase 1: フォーカス先の検出・分類 ──

    fn phase1_detect_focus(&mut self, state: &mut PipelineState) {
        state.focus_changed = unsafe { self.rt.detect_and_update_focus() };
    }

    // ── Phase 2.5: IMM ブリッジ非対応クラスの判定 ──
    //
    // Chrome / UWP / Electron 等はクロスプロセス IMM 問い合わせ（WM_IME_CONTROL）が
    // 動作しないか、無期限ブロックする恐れがある。既知のクラス名なら事前にスキップし、
    // シャドウ状態（hook から追跡）のみで IME 状態を管理する。
    //
    // Phase 2 の FocusChanged より前に計算する必要がある。
    // FocusChanged で build_ctx() が呼ばれる際、input_mode が stale な ObservedKana だと
    // engine が inactive になってしまうため、先に補正する。

    fn phase2_5_resolve_skip_imm_query(&self) -> bool {
        self.rt
            .executor
            .platform
            .focus
            .last_focus_info
            .as_ref()
            .map_or(false, |(_, class_name)| {
                crate::focus::classify::is_imm_bridge_broken(class_name)
            })
    }

    // ── Phase 2: プロセス変更時は Engine に FocusChanged（flush あり）──

    fn phase2_notify_focus_changed(&mut self, state: &PipelineState) {
        // IMM broken アプリ（Chrome 等）に切り替わった際に input_mode が
        // 前ウィンドウの stale な ObservedKana を引き継いでいると、FocusChanged の ctx で
        // engine が inactive になる。broken アプリでは入力モードを検出できないため、
        // ime_on=true のとき AssumedRomaji と仮定して補正する。
        if state.skip_imm_query
            && self.rt.platform_state.preconditions.ime_on
            && !self.rt.platform_state.preconditions.input_mode.is_romaji_capable()
        {
            log::info!(
                "FocusChanged: input_mode assumed romaji (IMM broken, stale kana from prev window)"
            );
            self.rt.platform_state.preconditions.input_mode =
                InputModeState::AssumedRomaji { reason: AssumedReason::ImmBridgeBroken };
        }
        let ctx = self.rt.build_ctx();
        let decision = self.rt.engine.on_command(EngineCommand::FocusChanged, &ctx);
        self.rt.executor.execute_from_loop(decision);
    }

    // ── Phase 2.7: 読み取り方針の決定 ──
    //
    // 最後のキー活動（物理キー押下 または VK/TSF 出力）から TYPING_IDLE_MS 以内は
    // IMM との SendMessage を一切行わない。

    fn phase2_7_decide_read_strategy(&self, skip_imm_query: bool) -> ImeReadStrategy {
        const TYPING_IDLE_MS: u64 = 500;
        let idle_ms = crate::hook::current_tick_ms()
            .saturating_sub(self.rt.platform_state.last_hook_activity_ms);
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

    // ── ブラックリストクラス: OS 読み取りをスキップ ──
    //
    // preconditions.ime_on はシャドウ更新 (hook 経由) が直接書き換える。
    // miss_count はインクリメントしない（既知の失敗なので「検出失敗」ではない）。
    //
    // 書き込みは「shadow が ON のときだけ」に限定する (ADR 029 の force-ON 原則)。

    fn apply_blacklist_ssot(&mut self) {
        log::debug!("Skipping IMM query for known-broken class (shadow state SSOT)");
        if self.rt.engine.is_user_enabled()
            && self.rt.platform_state.preconditions.is_japanese_ime
            && self.rt.platform_state.preconditions.ime_on
        {
            let _success = self.rt.executor.platform.set_ime_open(true);
            log::trace!("Blacklist SSOT write: ime_on=true (force-ON only)");
            // input_mode も SSOT として維持: IMM broken アプリでは検出不能のため
            // stale な ObservedKana が engine を無効化しないよう AssumedRomaji に補正する。
            if !self.rt.platform_state.preconditions.input_mode.is_romaji_capable() {
                log::info!("Blacklist SSOT: input_mode → AssumedRomaji (IMM broken, ime_on=true)");
                self.rt.platform_state.preconditions.input_mode =
                    InputModeState::AssumedRomaji { reason: AssumedReason::ImmBridgeBroken };
            }
        }
    }

    // ── Phase 3: IME 状態の再取得 ──

    fn phase3_poll_and_learn(&mut self, state: &mut PipelineState) {
        // [診断] observe 前のスナップショット（差分検出用）
        let ime_on_before_poll = self.rt.platform_state.preconditions.ime_on;
        let input_mode_before_poll = self.rt.platform_state.preconditions.input_mode;

        unsafe {
            crate::observer::ime_observer::observe(
                &mut self.rt.platform_state.preconditions,
                &mut self.rt.platform_state.ime_observations,
            );
        }

        let miss_after = self.rt.platform_state.preconditions.ime_detect_miss_count;

        // observe() の生の結果を os_ime_on に記録（miss なし＝成功時のみ更新）
        if miss_after == 0 {
            if let Some(obs) = &self.rt.platform_state.ime_observations.observer_poll {
                self.rt.platform_state.os_ime_on =
                    Some(obs.value && self.rt.platform_state.preconditions.is_japanese_ime);
            }
        }
        // observer_poll → preconditions.ime_on に優先度付き解決
        self.rt.platform_state.apply_ime_observations(self.rt.engine.is_user_enabled());

        self.log_poll_diff(
            ime_on_before_poll,
            input_mode_before_poll,
            state.miss_before,
            miss_after,
        );

        // Phase 3.1: IMM 能力の学習
        self.learn_imm_capability_from_result(state.miss_before, miss_after);

        // Phase 3.5: 未知 IMM-broken アプリ向け一時 force-ON（初回ブートストラップ）
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
            .saturating_sub(self.rt.platform_state.last_focus_change_ms);
        if age_ms < 10_000 {
            let ime_on_after = self.rt.platform_state.preconditions.ime_on;
            let input_mode_after = self.rt.platform_state.preconditions.input_mode;
            let ime_changed = ime_on_before_poll != ime_on_after;
            let mode_changed = input_mode_before_poll != input_mode_after;
            if ime_changed || mode_changed {
                log::info!(
                    "ObserverPoll +{}ms since focus: {}{}",
                    age_ms,
                    if ime_changed {
                        format!(
                            "ime_on {} → {}({:?}) ",
                            ime_on_before_poll,
                            ime_on_after,
                            self.rt.platform_state.preconditions.ime_on_source,
                        )
                    } else {
                        String::new()
                    },
                    if mode_changed {
                        format!("mode {:?} → {:?}", input_mode_before_poll, input_mode_after)
                    } else {
                        String::new()
                    },
                );
            } else if miss_after > 0 {
                log::debug!(
                    "ObserverPoll +{}ms since focus: detection failed (miss={}), stale ime_on={} mode={:?}",
                    age_ms,
                    miss_after,
                    ime_on_before_poll,
                    input_mode_before_poll,
                );
            }
        }
        let _ = miss_before; // suppress unused warning if logging is compiled out
    }

    /// Phase 3.1: 検出結果に基づいて class_name ごとの IMM 能力をキャッシュ。
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
                    .imm_capability_cache
                    .get(&class_name);
                if prev != Some(&ImmCapability::Works) {
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
                    .imm_capability_cache
                    .get(&class_name);
                if prev != Some(&ImmCapability::Broken) {
                    log::info!(
                        "IMM capability learned: {class_name} → Broken (detection failed {} times)",
                        miss_after
                    );
                    self.rt
                        .executor
                        .platform
                        .focus
                        .learn_imm_capability(class_name.clone(), ImmCapability::Broken);
                }
            }
        }
    }

    /// Phase 3.5: 未知 IMM-broken アプリ向け一時 force-ON（初回ブートストラップ）
    ///
    // ここに来るのは「既知でも TSF-native でもないアプリで detect が連続失敗した」
    // 場合だけ。shadow=ON なら SetOpen(true) を呼び engine を active のまま維持する。
    fn try_force_on_bootstrap(&mut self) {
        if self.rt.platform_state.preconditions.ime_detect_miss_count
            >= crate::IME_DETECT_MISS_THRESHOLD
            && self.rt.engine.is_user_enabled()
            && self.rt.platform_state.preconditions.is_japanese_ime
            && self.rt.platform_state.preconditions.ime_on
            && !self.rt.platform_state.preconditions.ime_force_on_guard
        {
            log::warn!(
                "IME detection failed {} times, forcing OS ime_on=true (shadow=ON)",
                self.rt.platform_state.preconditions.ime_detect_miss_count
            );
            let success = self.rt.executor.platform.set_ime_open(true);
            if success {
                self.rt.platform_state.preconditions.ime_force_on_guard = true;
                // miss_count はリセットしない。ガードが検出成功まで保護する。
            }
        }
    }

    // ── Phase 3.7: 診断スナップショット（フォーカス変更確定直後）──
    //
    // フォーカス変更が確定した直後の IME 状態を 1 行ログに吐き出す。
    // ウィンドウ切替直後の cold-start 不具合を解析するための観測点。

    fn phase3_7_post_focus_change_snapshot(&mut self, _state: &PipelineState) {
        crate::ime_diagnostic::ImeDiagnosticSnapshot::capture("focus_changed").log();
        // フォーカス変更時は VK/TSF いずれも composition context が無効化される。
        log::debug!("[composition] focus change → marking cold");
        // shadow_ime_on を最新の IME 状態に同期してから warmup 判定を行う。
        self.rt
            .executor
            .platform
            .output
            .notify_ime_open(self.rt.platform_state.preconditions.ime_on);
        self.rt
            .executor
            .platform
            .output
            .mark_composition_cold(crate::output::ColdReason::FocusChange);

        // TSF モード（WezTerm 等）かつ IME ON の場合、FocusChange 直後に F2 pre-warmup を送信する。
        self.rt.executor.platform.output.send_eager_tsf_warmup();
        log::debug!(
            "[composition] FocusChange: send_eager_tsf_warmup called (guarded by shadow_ime_on)"
        );
    }

    // ── Phase 4: Engine に RefreshState（active 遷移検知）──

    fn phase4_notify_engine_refresh(&mut self) {
        let ctx = self.rt.build_ctx();
        let decision = self.rt.engine.on_command(EngineCommand::RefreshState, &ctx);
        self.rt.executor.execute_from_loop(decision);
    }

    // ── Phase 5: 次回ポーリングを自動スケジュール ──

    fn phase5_reschedule(&mut self) {
        self.rt
            .schedule_ime_refresh(u64::from(self.rt.platform_state.ime_poll_interval_ms));
    }
}
