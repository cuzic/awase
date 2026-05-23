use awase::engine::{AssumedReason, EngineCommand, InputModeState};
use awase::platform::PlatformRuntime;

use crate::focus::classifier::ImmCapability;
use super::Runtime;
use crate::tuning::{TYPING_IDLE_MS, GJI_CONFIRM_WINDOW_MS};

// ── ImeReadStrategy ──

/// IME 読み取り方針の決定結果
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
    pub(super) fn new(rt: &'a mut Runtime) -> Self {
        Self { rt }
    }

    pub(super) fn run(mut self) {
        let focus = self.stage_focus();
        let strategy = self.stage_strategy(&focus);
        self.stage_observe(&focus, strategy);
        self.stage_notify();
    }

    // ── Stage 1: フォーカス検出 ──
    //
    // Phase 1: フォーカス先の検出・分類
    // Phase 2.5: IMM ブリッジ非対応クラスの判定（Phase 2 の前に実行する必要あり）
    // Phase 2: プロセス変更時は Engine に FocusChanged（flush あり）

    fn stage_focus(&mut self) -> FocusInfo {
        // Phase 1: フォーカス先の検出・分類
        let focus_changed = unsafe { self.rt.detect_and_update_focus() };

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
    // Phase 3.5: 未知 IMM-broken アプリ向け一時 force-ON（初回ブートストラップ）
    // Phase 3.7: 診断スナップショット（フォーカス変更後）

    fn stage_observe(&mut self, focus: &FocusInfo, strategy: ImeReadStrategy) {
        match strategy {
            ImeReadStrategy::SkipTyping => {
                // タイピング中は何もしない
            }
            ImeReadStrategy::Blacklist => {
                self.apply_blacklist_ssot();
            }
            ImeReadStrategy::OsPoll => {
                // Phase 3: IME 状態の再取得
                let miss_before = self.rt.platform_state.ime_detect_miss_count();
                self.poll_and_learn(miss_before);
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
        // Phase 4: Engine に RefreshState（active 遷移検知）
        self.notify_engine_refresh();

        // Phase 5: 次回ポーリングをスケジュール
        self.reschedule();
    }

    // ── IMM ブリッジ非対応クラスの判定 ──

    fn resolve_skip_imm_query(&self) -> bool {
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
        self.rt.executor.execute_from_loop(decision);
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

    // ── ブラックリストクラス: OS 読み取りをスキップ ──
    //
    // preconditions.ime_on はシャドウ更新 (hook 経由) が直接書き換える。
    // miss_count はインクリメントしない（既知の失敗なので「検出失敗」ではない）。
    //
    // 書き込みは「shadow が ON のときだけ」に限定する (ADR 029 の force-ON 原則)。

    fn apply_blacklist_ssot(&mut self) {
        log::debug!("Skipping IMM query for known-broken class (shadow state SSOT)");

        // GJI I/O 観測を observer_poll 経由で judgement に流す（観測 → 判断の正規ルート）。
        // バックグラウンドスレッドが TSF_OBS.gji_last_io_ms を更新していれば
        // Chrome の IME が ON であると判断し preconditions.ime_on に反映する。
        {
            let now_ms = crate::hook::current_tick_ms();
            let last_io = crate::tsf::observer::with_tsf_obs(|obs| obs.gji_last_io_ms());
            if last_io > 0 && now_ms.saturating_sub(last_io) < GJI_CONFIRM_WINDOW_MS {
                log::debug!(
                    "[gji-poll] GJI I/O observed {}ms ago → observer_poll=true",
                    now_ms.saturating_sub(last_io)
                );
                self.rt.platform_state.write_observer_poll(
                    true,
                    now_ms,
                    self.rt.engine.is_user_enabled(),
                );
            }
        }

        if self.rt.engine.is_user_enabled()
            && self.rt.platform_state.is_japanese_ime()
            && self.rt.platform_state.ime_on()
        {
            let _success = self.rt.executor.platform.set_ime_open(true);
            log::trace!("Blacklist SSOT write: ime_on=true (force-ON only)");
            // input_mode も SSOT として維持: IMM broken アプリでは検出不能のため
            // stale な ObservedKana が engine を無効化しないよう AssumedRomaji に補正する。
            if !self.rt.platform_state.input_mode().is_romaji_capable() {
                log::info!("Blacklist SSOT: input_mode → AssumedRomaji (IMM broken, ime_on=true)");
                self.rt.platform_state.set_input_mode(
                    InputModeState::AssumedRomaji { reason: AssumedReason::ImmBridgeBroken }
                );
            }
        }
    }

    // ── IME 状態のポーリングと学習 ──

    fn poll_and_learn(&mut self, miss_before: u32) {
        // [診断] observe 前のスナップショット（差分検出用）
        let ime_on_before_poll = self.rt.platform_state.ime_on();
        let input_mode_before_poll = self.rt.platform_state.input_mode();

        let observer_out = unsafe {
            crate::observer::ime_observer::observe(
                self.rt.platform_state.ime_on(),
                self.rt.platform_state.is_force_on_guard_active(),
                self.rt.platform_state.input_mode(),
                self.rt.platform_state.prev_conversion_mode(),
            )
        };
        self.rt.platform_state.apply_ime_update(&observer_out);

        let miss_after = self.rt.platform_state.ime_detect_miss_count();

        // observer_poll → preconditions.ime_on に優先度付き解決
        self.rt.platform_state.apply_ime_observations(self.rt.engine.is_user_enabled());

        self.log_poll_diff(
            ime_on_before_poll,
            input_mode_before_poll,
            miss_before,
            miss_after,
        );

        // IMM 能力の学習
        self.learn_imm_capability_from_result(miss_before, miss_after);

        // 未知 IMM-broken アプリ向け一時 force-ON（初回ブートストラップ）
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
                            "ime_on {} → {}({:?}) ",
                            ime_on_before_poll,
                            ime_on_after,
                            self.rt.platform_state.ime_on_source(),
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
                if prev != Some(ImmCapability::Broken) {
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

    /// 未知 IMM-broken アプリ向け一時 force-ON（初回ブートストラップ）
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
            let success = self.rt.executor.platform.set_ime_open(true);
            // success/failure 問わずガードをセット: IME 非対応ウィンドウ(DirectUIHWND 等)で
            // 失敗し続ける無限ループを防ぐ。ガードはフォーカス変更時に解除される。
            self.rt.platform_state.set_force_on_broken_app_bootstrap();
            if !success {
                log::warn!("set_ime_open failed (no IME window?) — guard set to suppress retry until focus change");
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
        // フォーカス変更直後の IMM 実測値でラッチを初期化する。
        // これにより KanjiToggleStrategy が次回 apply_ime_open を呼ぶまでの
        // shadow_on に preconditions の最新値を使えるようになる。
        self.rt
            .executor
            .platform
            .output
            .set_ime_apply_latch(self.rt.platform_state.ime_on());
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

        // shadow_ime_on=false の場合、新しいウィンドウの IME を明示的に OFF にする。
        // Ctrl+無変換 は発火時点のウィンドウにしか set_ime_open を送らないため、
        // 別ウィンドウに移動すると IME が ON のままになるのを防ぐ。
        if !self.rt.executor.platform.output.shadow_ime_on() {
            let _ = self.rt.executor.platform.set_ime_open(false);
            log::debug!("[composition] FocusChange: set_ime_open(false) called (shadow OFF → enforce IME OFF on new window)");
        }
    }

    // ── Engine 通知 ──

    fn notify_engine_refresh(&mut self) {
        let ctx = self.rt.build_ctx();
        let decision = self.rt.engine.on_command(EngineCommand::RefreshState, &ctx);
        self.rt.executor.execute_from_loop(decision);
    }

    // ── 次回ポーリングのスケジュール ──

    fn reschedule(&mut self) {
        self.rt
            .schedule_ime_refresh(u64::from(self.rt.platform_state.focus.ime_poll_interval_ms));
    }

    /// pre-fetch 済みデータを使ってパイプラインを実行（blocking なし）。
    /// spawn_local タスクから呼ぶ。
    pub(super) fn run_with_prefetched(
        mut self,
        focus_probe: Option<crate::focus::probe::FocusProbe>,
        ime_snap: crate::ime::ImeSnapshot,
    ) {
        // Stage 1: フォーカス検出（pre-fetch 版）
        let focus_changed = self.rt.apply_focus_probe_result(focus_probe);
        let skip_imm_query = self.resolve_skip_imm_query();
        if focus_changed {
            self.notify_focus_changed(skip_imm_query);
        }
        let focus = FocusInfo { focus_changed, skip_imm_query };

        // Stage 2: 読み取り方針の決定
        let strategy = self.stage_strategy(&focus);

        // Stage 3: IME 状態の観測（pre-fetch 版）
        match strategy {
            ImeReadStrategy::SkipTyping => {}
            ImeReadStrategy::Blacklist => {
                self.apply_blacklist_ssot();
            }
            ImeReadStrategy::OsPoll => {
                let miss_before = self.rt.platform_state.ime_detect_miss_count();

                // [診断] apply_snapshot 前のスナップショット（差分検出用）
                let ime_on_before_poll = self.rt.platform_state.ime_on();
                let input_mode_before_poll = self.rt.platform_state.input_mode();

                // apply pre-fetched IME snap
                let now_ms = crate::hook::current_tick_ms();
                let observer_out = {
                    crate::observer::ime_observer::apply_snapshot(
                        &ime_snap,
                        now_ms,
                        self.rt.platform_state.ime_on(),
                        self.rt.platform_state.is_force_on_guard_active(),
                        self.rt.platform_state.input_mode(),
                        self.rt.platform_state.prev_conversion_mode(),
                    )
                };
                self.rt.platform_state.apply_ime_update(&observer_out);

                let miss_after = self.rt.platform_state.ime_detect_miss_count();

                self.rt
                    .platform_state
                    .apply_ime_observations(self.rt.engine.is_user_enabled());

                self.log_poll_diff(
                    ime_on_before_poll,
                    input_mode_before_poll,
                    miss_before,
                    miss_after,
                );
                self.learn_imm_capability_from_result(miss_before, miss_after);
                self.try_force_on_bootstrap();
            }
        }

        if focus.focus_changed {
            self.post_focus_change_snapshot(focus.skip_imm_query);
        }

        // Stage 4: Engine 通知と次回スケジュール
        self.stage_notify();
    }
}
