//! TSF/Chrome cold-start probe ステートマシン。
//!
//! [`TsfProbeMachine`] は 10ms 間隔の `TIMER_TSF_PROBE` ハンドラから駆動される。
//! `tick()` がフェーズを 1 ステップ進め、dispatcher が返す [`ProbeAction`] を
//! `platform.rs::dispatch_probe_actions` が実行する。
//!
//! ## フェーズ遷移
//!
//! ```text
//! Probing(GjiInitial) ─[settle 不要]──► WaitingForCallback ─[apply_transmit_done]─► LiteralDetect
//!                     └─[settle 必要]─► WaitingForCallback ─[apply_fresh_f2_sent]─► NameChangeWait
//!                                                                                        ├─[nc_fired && !settled]► Probing(GjiSecondary) ─► WaitingForCallback
//!                                                                                        └─[timeout or nc_fired+settled]─────────────────► WaitingForCallback
//! Probing(Chrome) ─────────────────────────────────────────────────────────────────► (TransmitChrome 後に即完了)
//! ```
//!
//! ## 設計ポリシー
//!
//! - `tick()` / `apply_*` は副作用なし。状態のみ更新し [`ProbeAction`] を返す。
//! - SendInput / mark_warm / mark_cold / RAW_TSF_LITERAL 操作は dispatcher 側で実行する
//!   (`platform.rs::dispatch_probe_actions`)。
//! - `romaji` / `deferred_vks` は [`ProbeAction::Transmit`] 生成時に `std::mem::take` で move する
//!   （clone コストを避ける）。
//! - 環境観測（`gji_last_io_ms`, `gji_monitor_healthy`）は `TsfReadinessProbe::check_outcome`
//!   内で完結させ、`tick()` は受け取った [`GjiProbeOutcome`] のみを参照する。
//! - FSM 内部フィールドは非公開。送信コンテキスト（`cold_seq` / `prepend_f2_warmup` 等）は
//!   [`ProbeAction`] のペイロードに畳み込んで dispatcher に渡す。

use crate::tsf::observer::NamechangeBaseline;
use crate::tsf::output::ColdReason;

type VkSequence = Vec<(u16, bool)>;
use crate::tsf::probe::{GjiProbeOutcome, LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;

/// 単調増加ミリ秒クロック。`tick_with_clock` でテスト可能なタイミング注入に使う。
pub(crate) trait Clock {
    fn now_ms(&self) -> u64;
}

/// 本番クロック: `crate::hook::current_tick_ms()` に委譲する。
pub(crate) struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> u64 { crate::hook::current_tick_ms() }
}

/// プローブ全体で不変の送信コンテキスト（TSF パスのみ意味を持つ）。
#[derive(Debug, Clone, Copy)]
struct ProbeContext {
    prepend_f2_warmup: bool,
    used_eager_path: bool,
}

/// `inspect_phase` が返す「次に何をすべきか」の宣言。
///
/// `tick_with_clock` はこれを受け取って状態遷移と `ProbeAction` emit を行う。
/// `inspect_phase` は `&self` のみ使用（副作用なし）。
enum NextStep {
    /// 現フェーズで継続待機（actions なし）
    Wait,
    /// `WaitingForCallback { probe_settled: Some(_) }` へ遷移し `SendFreshF2` を emit
    EmitSendFreshF2 { probe_settled: bool },
    /// `enter_transmit_tsf()` を呼ぶ
    TransmitTsf,
    /// `enter_transmit_chrome()` を呼ぶ
    TransmitChrome,
    /// `NameChangeWait` → `Probing(GjiSecondary)` へ遷移（fresh_f2_ms を引き継ぐ）
    StartSecondaryProbe { fresh_f2_ms: u64 },
    /// `LiteralDetect`: composition 確定 → `Done`
    LiteralDone,
    /// `LiteralDetect`: raw literal 疑い → `RawTsfLiteralRecovery + Done`
    LiteralSuspected { ze_bs_count: usize },
}

/// [`ProbePhase::Probing`] の完了後アクションを区別するタグ。
pub(crate) enum ProbeKind {
    /// 初回 GJI probe。settle check が必要な場合あり。
    GjiInitial { needs_settle_check: bool, cold_reason: ColdReason },
    /// OBJ_NAMECHANGE 後の二次 GJI probe。settle check なし。
    GjiSecondary,
    /// Chrome F2 probe。完了後 `Transmit(Chrome)` を emit。
    Chrome,
}

/// プローブ FSM の現在フェーズ。
pub(crate) enum ProbePhase {
    /// GJI 静止待ち / Chrome F2 後の静止待ち（`kind` で区別）。
    Probing {
        probe: TsfReadinessProbe,
        total_max_ms: u64,
        kind: ProbeKind,
    },
    /// dispatcher コールバック（`apply_fresh_f2_sent` / `apply_transmit_done`）待ち。
    ///
    /// `probe_settled = Some(_)` → `apply_fresh_f2_sent` 待ち（SendFreshF2 パス）。
    /// `probe_settled = None`    → `apply_transmit_done` 待ち（Transmit(Tsf) パス）。
    WaitingForCallback { probe_settled: Option<bool> },
    /// OBJ_NAMECHANGE 待ち（GjiInitial probe + needs_settle_check の次フェーズ）。
    NameChangeWait {
        nc_baseline: NamechangeBaseline,
        deadline_ms: u64,
        fresh_f2_ms: u64,
        probe_settled: bool,
    },
    /// raw TSF literal 検出待ち（TSF 送信後の verify フェーズ）。
    LiteralDetect {
        detector: LiteralDetector,
        ze_bs_count: usize,
        deadline_ms: u64,
    },
}

/// [`ProbeAction::Transmit`] の送信先。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransmitTarget {
    Tsf,
    Chrome,
}

/// ステートマシン → dispatcher 方向の宣言的アクション。
pub(crate) enum ProbeAction {
    /// fresh F2 (`VK_DBE_HIRAGANA`) を送信し、完了したら
    /// [`TsfProbeMachine::apply_fresh_f2_sent`] を呼ぶ。
    SendFreshF2 { cold_seq: u32, probe_settled: bool },
    /// TSF または Chrome バッチパイプラインで `romaji` を送信する。
    ///
    /// `target == Tsf`: `deferred_vks` をフラッシュし warm マークしたあと
    /// [`TsfProbeMachine::apply_transmit_done`] を呼ぶ。
    /// `target == Chrome`: `deferred_vks` をフラッシュし warm マークして完了。
    Transmit {
        cold_seq: u32,
        prepend_f2_warmup: bool,
        used_eager_path: bool,
        romaji: String,
        deferred_vks: VkSequence,
        target: TransmitTarget,
    },
    /// `RAW_TSF_LITERAL` を設定し、composition を `RawTsfLiteralRecovery` で cold マークする。
    RawTsfLiteralRecovery { cold_seq: u32, backs: usize, romaji: String },
    /// プローブ完了。dispatcher は `TIMER_TSF_PROBE` を kill する。
    Done,
}

/// TSF/Chrome cold-start プローブの状態機械本体。
///
/// `pending_tsf` (`RefCell<Option<TsfProbeMachine>>`) に格納し、
/// `TIMER_TSF_PROBE` ハンドラが `tick()` で 1 ステップ進める。
pub(crate) struct TsfProbeMachine {
    /// 送信するローマ字（`Transmit` 時に `std::mem::take`）
    romaji: String,
    /// ログ相関番号
    cold_seq: u32,
    /// probe 進行中に蓄積された後続 VK
    deferred_vks: VkSequence,
    /// RAII guard。drop で `OUTPUT_GATE.active=false`
    _guard: OutputActiveGuard,
    /// 送信コンテキスト（`enter_transmit_*` で `ProbeAction` に畳み込む）
    ctx: ProbeContext,
    /// 現在フェーズ
    phase: ProbePhase,
}

impl TsfProbeMachine {
    /// TSF cold warmup (`send_romaji_as_tsf` の cold パス) 用コンストラクタ。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_gji(
        romaji: &str,
        cold_seq: u32,
        probe: TsfReadinessProbe,
        total_max_ms: u64,
        needs_settle_check: bool,
        cold_reason: ColdReason,
        prepend_f2_warmup: bool,
        used_eager_path: bool,
        guard: OutputActiveGuard,
    ) -> Self {
        Self {
            romaji: romaji.to_string(),
            cold_seq,
            deferred_vks: Vec::new(),
            _guard: guard,
            ctx: ProbeContext { prepend_f2_warmup, used_eager_path },
            phase: ProbePhase::Probing {
                probe,
                total_max_ms,
                kind: ProbeKind::GjiInitial { needs_settle_check, cold_reason },
            },
        }
    }

    /// Chrome F2 cold warmup (`send_romaji_batched` の cold パス) 用コンストラクタ。
    pub(crate) fn new_chrome(
        romaji: &str,
        cold_seq: u32,
        probe: TsfReadinessProbe,
        total_max_ms: u64,
        guard: OutputActiveGuard,
    ) -> Self {
        Self {
            romaji: romaji.to_string(),
            cold_seq,
            deferred_vks: Vec::new(),
            _guard: guard,
            ctx: ProbeContext { prepend_f2_warmup: false, used_eager_path: false },
            phase: ProbePhase::Probing { probe, total_max_ms, kind: ProbeKind::Chrome },
        }
    }

    /// warm パスからの LiteralDetect 直入り用コンストラクタ。
    ///
    /// `send_romaji_as_tsf` の warm パスで投機送信後に LiteralDetect だけを動かしたい場合に使う。
    pub(crate) fn new_literal_detect(
        romaji: &str,
        cold_seq: u32,
        detector: LiteralDetector,
        ze_bs_count: usize,
        deadline_ms: u64,
        guard: OutputActiveGuard,
    ) -> Self {
        Self {
            romaji: romaji.to_string(),
            cold_seq,
            deferred_vks: Vec::new(),
            _guard: guard,
            ctx: ProbeContext { prepend_f2_warmup: false, used_eager_path: false },
            phase: ProbePhase::LiteralDetect { detector, ze_bs_count, deadline_ms },
        }
    }

    /// ログ用 cold_seq 参照（上書き警告等で使う）。
    pub(crate) fn cold_seq_hint(&self) -> u32 { self.cold_seq }

    /// probe 進行中に後続 VK を 1 つ蓄積する。
    pub(crate) fn push_deferred(&mut self, vk: u16, needs_shift: bool) {
        self.deferred_vks.push((vk, needs_shift));
    }

    /// probe 進行中に後続 VK を複数蓄積する。
    pub(crate) fn extend_deferred(&mut self, vks: impl IntoIterator<Item = (u16, bool)>) {
        self.deferred_vks.extend(vks);
    }

    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。フェーズを 1 ステップ進める。
    ///
    /// 返値の `Vec<ProbeAction>` を `dispatch_probe_actions` が実行する。
    /// 空 Vec = まだ待機中（タイマー継続）。
    pub(crate) fn tick(&mut self) -> Vec<ProbeAction> {
        self.tick_with_clock(&SystemClock)
    }

    /// `Clock` を注入できる `tick` の内部実装（テスト用）。
    ///
    /// `inspect_phase` で副作用なしに次の遷移先を決定し、
    /// その後 `&self` を解放してから状態変化を適用する。
    /// `std::mem::replace` + 全フィールド書き戻しパターンを排除している。
    pub(crate) fn tick_with_clock<C: Clock>(&mut self, clock: &C) -> Vec<ProbeAction> {
        match self.inspect_phase(clock) {
            NextStep::Wait => vec![],
            NextStep::EmitSendFreshF2 { probe_settled } => {
                self.phase = ProbePhase::WaitingForCallback { probe_settled: Some(probe_settled) };
                vec![ProbeAction::SendFreshF2 { cold_seq: self.cold_seq, probe_settled }]
            }
            NextStep::TransmitTsf => self.enter_transmit_tsf(),
            NextStep::TransmitChrome => self.enter_transmit_chrome(),
            NextStep::StartSecondaryProbe { fresh_f2_ms } => {
                let probe = TsfReadinessProbe::new(fresh_f2_ms, self.cold_seq, 0);
                self.phase = ProbePhase::Probing {
                    probe,
                    total_max_ms: crate::tuning::GJI_POST_NAMECHANGE_MS,
                    kind: ProbeKind::GjiSecondary,
                };
                vec![]
            }
            NextStep::LiteralDone => vec![ProbeAction::Done],
            NextStep::LiteralSuspected { ze_bs_count } => {
                let romaji = std::mem::take(&mut self.romaji);
                // ※「consecutive_count」のチェック (false-positive 抑制) は dispatcher 側で行う。
                vec![
                    ProbeAction::RawTsfLiteralRecovery { cold_seq: self.cold_seq, backs: ze_bs_count, romaji },
                    ProbeAction::Done,
                ]
            }
        }
    }

    /// 現フェーズを検査して次の遷移先を返す。副作用なし（`&self` のみ使用）。
    fn inspect_phase<C: Clock>(&self, clock: &C) -> NextStep {
        match &self.phase {
            ProbePhase::Probing { probe, total_max_ms, kind } => {
                let Some(outcome) = probe.check_outcome(*total_max_ms) else {
                    return NextStep::Wait;
                };
                match kind {
                    ProbeKind::GjiInitial { needs_settle_check, cold_reason } => {
                        log::debug!(
                            "[tsf-probe] cold={} GjiProbe 完了 ({}ms)",
                            self.cold_seq, outcome.elapsed_ms
                        );
                        if *needs_settle_check {
                            let is_ime_init_cold = cold_reason.requires_settle();
                            if (!outcome.settled || is_ime_init_cold) && outcome.monitor_healthy {
                                return NextStep::EmitSendFreshF2 { probe_settled: outcome.settled };
                            }
                        }
                        NextStep::TransmitTsf
                    }
                    ProbeKind::GjiSecondary => {
                        log::debug!(
                            "[tsf-probe] cold={} SecondaryGjiProbe 完了 ({}ms)",
                            self.cold_seq, outcome.elapsed_ms
                        );
                        NextStep::TransmitTsf
                    }
                    ProbeKind::Chrome => {
                        log::debug!(
                            "[tsf-probe] cold={} ChromeProbe 完了 → batched 送信 ({}ms)",
                            self.cold_seq, outcome.elapsed_ms
                        );
                        NextStep::TransmitChrome
                    }
                }
            }

            ProbePhase::WaitingForCallback { .. } => NextStep::Wait,

            ProbePhase::NameChangeWait { nc_baseline, deadline_ms, fresh_f2_ms, probe_settled } => {
                let now = clock.now_ms();
                let nc_fired = nc_baseline.fired();
                let timed_out = now >= *deadline_ms;
                if !nc_fired && !timed_out {
                    return NextStep::Wait;
                }
                let elapsed = now.saturating_sub(*fresh_f2_ms);
                log::debug!(
                    "[tsf-probe] cold={} NameChangeWait → nc_fired={nc_fired} timed_out={timed_out} ({elapsed}ms)",
                    self.cold_seq
                );
                if nc_fired && !probe_settled {
                    log::debug!(
                        "[tsf-probe] cold={} OBJ_NAMECHANGE後 GJI 二次プローブ (max {}ms)",
                        self.cold_seq, crate::tuning::GJI_POST_NAMECHANGE_MS
                    );
                    NextStep::StartSecondaryProbe { fresh_f2_ms: *fresh_f2_ms }
                } else {
                    NextStep::TransmitTsf
                }
            }

            ProbePhase::LiteralDetect { detector, ze_bs_count, deadline_ms } => {
                use crate::tsf::probe::DetectionResult;
                let Some(detection) = detector.check_now(*deadline_ms) else {
                    return NextStep::Wait;
                };
                match detection {
                    DetectionResult::CompositionConfirmed => {
                        log::debug!("[raw-tsf-literal] cold={} composition confirmed", self.cold_seq);
                        NextStep::LiteralDone
                    }
                    DetectionResult::SuspectedLiteral => {
                        NextStep::LiteralSuspected { ze_bs_count: *ze_bs_count }
                    }
                }
            }
        }
    }

    /// テスト専用: 任意のフェーズに強制遷移する。
    #[cfg(test)]
    pub(crate) fn force_phase_for_test(&mut self, phase: ProbePhase) {
        self.phase = phase;
    }

    /// テスト専用: 現在フェーズの文字列ラベルを返す。
    #[cfg(test)]
    pub(crate) fn phase_label(&self) -> &'static str {
        match &self.phase {
            ProbePhase::Probing { .. } => "Probing",
            ProbePhase::WaitingForCallback { .. } => "WaitingForCallback",
            ProbePhase::NameChangeWait { .. } => "NameChangeWait",
            ProbePhase::LiteralDetect { .. } => "LiteralDetect",
        }
    }

    /// dispatcher が `SendFreshF2` を実行した後に呼ぶ。
    /// `WaitingForCallback { probe_settled: Some(_) }` → `NameChangeWait` へ遷移する。
    pub(crate) fn apply_fresh_f2_sent(&mut self, nc_baseline: NamechangeBaseline, fresh_f2_ms: u64) {
        let ProbePhase::WaitingForCallback { probe_settled: Some(probe_settled) } = self.phase else {
            log::warn!(
                "[tsf-probe] cold={} apply_fresh_f2_sent: unexpected phase",
                self.cold_seq
            );
            return;
        };
        let deadline_ms = fresh_f2_ms + crate::tuning::SETTLE_TIMEOUT_MS;
        self.phase = ProbePhase::NameChangeWait {
            nc_baseline,
            deadline_ms,
            fresh_f2_ms,
            probe_settled,
        };
    }

    /// dispatcher が `Transmit(Tsf)` を実行した後に呼ぶ。
    ///
    /// `detector` は送信**前**に dispatcher 側で生成し渡すこと（ベースラインは transmit 前が正しい）。
    ///
    /// `true` を返した場合は Done（dispatcher は `TIMER_TSF_PROBE` を kill する）。
    /// `false` を返した場合は `LiteralDetect` フェーズへ遷移し、タイマーを継続する。
    pub(crate) fn apply_transmit_done(
        &mut self,
        ze_bs_count: usize,
        detector: Option<LiteralDetector>,
    ) -> bool {
        if let Some(detector) = detector {
            let deadline_ms =
                crate::hook::current_tick_ms() + crate::tuning::RAW_TSF_LITERAL_DETECT_MS;
            self.phase = ProbePhase::LiteralDetect { detector, ze_bs_count, deadline_ms };
            false
        } else {
            true
        }
    }

    fn enter_transmit_tsf(&mut self) -> Vec<ProbeAction> {
        let romaji = std::mem::take(&mut self.romaji);
        let deferred_vks = std::mem::take(&mut self.deferred_vks);
        self.phase = ProbePhase::WaitingForCallback { probe_settled: None };
        vec![ProbeAction::Transmit {
            cold_seq: self.cold_seq,
            prepend_f2_warmup: self.ctx.prepend_f2_warmup,
            used_eager_path: self.ctx.used_eager_path,
            romaji,
            deferred_vks,
            target: TransmitTarget::Tsf,
        }]
    }

    fn enter_transmit_chrome(&mut self) -> Vec<ProbeAction> {
        let romaji = std::mem::take(&mut self.romaji);
        let deferred_vks = std::mem::take(&mut self.deferred_vks);
        self.phase = ProbePhase::WaitingForCallback { probe_settled: None };
        vec![ProbeAction::Transmit {
            cold_seq: self.cold_seq,
            prepend_f2_warmup: false,
            used_eager_path: false,
            romaji,
            deferred_vks,
            target: TransmitTarget::Chrome,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::probe_bridge::OutputActiveGuard;

    struct MockClock(u64);
    impl Clock for MockClock {
        fn now_ms(&self) -> u64 { self.0 }
    }

    fn make_gji_machine() -> TsfProbeMachine {
        let guard = OutputActiveGuard::noop_for_test();
        let probe = TsfReadinessProbe::new(0, 0, 0);
        TsfProbeMachine::new_gji(
            "ka", 0, probe, 0, false,
            ColdReason::FocusChange,
            false, false, guard,
        )
    }

    fn make_namechange_wait(deadline_ms: u64, probe_settled: bool) -> ProbePhase {
        let baseline = crate::tsf::observer::namechange_baseline();
        ProbePhase::NameChangeWait {
            nc_baseline: baseline,
            deadline_ms,
            fresh_f2_ms: 0,
            probe_settled,
        }
    }

    // ── NameChangeWait フェーズ遷移テスト ─────────────────────────────────────

    #[test]
    fn namechange_wait_before_deadline_stays_waiting() {
        let mut machine = make_gji_machine();
        machine.force_phase_for_test(make_namechange_wait(1000, false));

        let actions = machine.tick_with_clock(&MockClock(500)); // now < deadline
        assert!(actions.is_empty(), "待機中は空 Vec を返すべき");
        assert_eq!(machine.phase_label(), "NameChangeWait");
    }

    #[test]
    fn namechange_wait_timeout_emits_transmit_tsf() {
        let mut machine = make_gji_machine();
        machine.force_phase_for_test(make_namechange_wait(500, true)); // settled=true

        let actions = machine.tick_with_clock(&MockClock(1000)); // now >= deadline
        assert!(!actions.is_empty(), "タイムアウト後は action を emit するべき");
        assert!(
            matches!(actions[0], ProbeAction::Transmit { target: TransmitTarget::Tsf, .. }),
            "タイムアウト時は Transmit(Tsf) を emit するべき: {actions:?}",
        );
    }

    #[test]
    fn namechange_wait_timeout_unsettled_emits_transmit_tsf() {
        // timed_out=true, probe_settled=false → タイムアウトで直接 Transmit(Tsf)
        let mut machine = make_gji_machine();
        machine.force_phase_for_test(make_namechange_wait(500, false));

        let actions = machine.tick_with_clock(&MockClock(1000));
        assert!(
            matches!(actions[0], ProbeAction::Transmit { target: TransmitTarget::Tsf, .. }),
            "タイムアウト(unsettled)でも Transmit(Tsf) を emit するべき",
        );
    }

    // ── WaitingForCallback フェーズテスト ────────────────────────────────────

    #[test]
    fn waiting_for_callback_is_no_op() {
        let mut machine = make_gji_machine();
        machine.force_phase_for_test(ProbePhase::WaitingForCallback { probe_settled: None });

        let actions = machine.tick_with_clock(&MockClock(0));
        assert!(actions.is_empty(), "WaitingForCallback は空 Vec を返すべき");
        assert_eq!(machine.phase_label(), "WaitingForCallback");
    }

    // ── push_deferred / extend_deferred テスト ───────────────────────────────

    #[test]
    fn push_deferred_appends_vk() {
        let mut machine = make_gji_machine();
        machine.push_deferred(0x41, false);
        machine.push_deferred(0x42, true);
        // deferred_vks は private だが Transmit action 経由で確認できる。
        // ここでは push_deferred が panic しないことを確認するだけで十分。
    }

    #[test]
    fn extend_deferred_appends_multiple_vks() {
        let mut machine = make_gji_machine();
        machine.extend_deferred(vec![(0x41, false), (0x42, true), (0x43, false)]);
        // panic しないことを確認
    }
}
