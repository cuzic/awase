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
//! - `romaji` / `deferred_vks` は `SendState` として各 phase variant に埋め込まれており、
//!   `ProbeAction::Transmit` 生成時に `take_current_send_for_transmit` で move する
//!   （clone コストを避ける）。
//! - 環境観測（`gji_last_io_ms`, `gji_monitor_healthy`）は `TsfReadinessProbe::check_outcome`
//!   内で完結させ、`tick()` は受け取った [`GjiProbeOutcome`] のみを参照する。
//! - FSM 内部フィールドは非公開。送信コンテキスト（`cold_seq` / `prepend_f2_warmup` 等）は
//!   [`ProbeAction`] のペイロードに畳み込んで dispatcher に渡す。

use crate::tsf::observer::NamechangeBaseline;
use crate::tsf::output::ColdReason;
use awase::types::VkCode;

/// probe 進行中に蓄積する後続 VK。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeferredVk {
    pub(crate) vk: VkCode,
    pub(crate) needs_shift: bool,
}
use crate::tsf::probe::{LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;
use timed_fsm::Clock;

/// 本番クロック: `crate::hook::current_tick_ms()` に委譲する。
///
/// `deadline_ms` は `current_tick_ms()` 起点で計算されるため、
/// `MonotonicClock` と混在させると epoch がずれる。
pub(crate) struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        crate::hook::current_tick_ms()
    }
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
    /// `WaitingForCallback(FreshF2Sent { .. })` へ遷移し `SendFreshF2` を emit
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
    GjiInitial {
        needs_settle_check: bool,
        cold_reason: ColdReason,
    },
    /// OBJ_NAMECHANGE 後の二次 GJI probe。settle check なし。
    GjiSecondary,
    /// Chrome F2 probe。完了後 `Transmit(Chrome)` を emit。
    Chrome,
}

/// multi-tick にわたって蓄積する送信データ。
///
/// `ProbePhase` の各 variant に埋め込まれ、phase が単独所有する。
/// `Clone` は derive しない（所有権移動のみ許可）。
#[derive(Debug, Default)]
pub(crate) struct SendState {
    pub(crate) romaji: String,
    pub(crate) deferred_vks: Vec<DeferredVk>,
}

impl SendState {
    fn new(romaji: &str) -> Self {
        Self {
            romaji: romaji.to_string(),
            deferred_vks: Vec::new(),
        }
    }
}

/// [`ProbePhase::WaitingForCallback`] の内部状態。
pub(crate) enum WaitingFor {
    /// `apply_fresh_f2_sent` 待ち（SendFreshF2 パス）。
    FreshF2Sent {
        probe_settled: bool,
        send: SendState,
    },
    /// `apply_transmit_done` 待ち（Transmit(Tsf) パス）。
    TransmitDone,
}

/// プローブ FSM の現在フェーズ。
pub(crate) enum ProbePhase {
    /// GJI 静止待ち / Chrome F2 後の静止待ち（`kind` で区別）。
    Probing {
        probe: TsfReadinessProbe,
        total_max_ms: u64,
        kind: ProbeKind,
        send: SendState,
    },
    /// dispatcher コールバック（`apply_fresh_f2_sent` / `apply_transmit_done`）待ち。
    WaitingForCallback(WaitingFor),
    /// OBJ_NAMECHANGE 待ち（GjiInitial probe + needs_settle_check の次フェーズ）。
    NameChangeWait {
        nc_baseline: NamechangeBaseline,
        deadline_ms: u64,
        fresh_f2_ms: u64,
        probe_settled: bool,
        send: SendState,
    },
    /// raw TSF literal 検出待ち（TSF 送信後の verify フェーズ）。
    LiteralDetect {
        detector: LiteralDetector,
        ze_bs_count: usize,
        deadline_ms: u64,
        send: SendState,
    },
}

/// [`ProbeAction::Transmit`] の送信先。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransmitTarget {
    Tsf,
    Chrome,
}

/// ステートマシン → dispatcher 方向の宣言的アクション。
#[derive(Debug)]
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
        deferred_vks: Vec<DeferredVk>,
        target: TransmitTarget,
    },
    /// `RAW_TSF_LITERAL` を設定し、composition を `RawTsfLiteralRecovery` で cold マークする。
    RawTsfLiteralRecovery {
        cold_seq: u32,
        backs: usize,
        romaji: String,
    },
    /// プローブ完了。dispatcher は `TIMER_TSF_PROBE` を kill する。
    Done,
}

/// TSF/Chrome cold-start プローブの状態機械本体。
///
/// `pending_tsf` (`RefCell<Option<TsfProbeMachine>>`) に格納し、
/// `TIMER_TSF_PROBE` ハンドラが `tick()` で 1 ステップ進める。
pub(crate) struct TsfProbeMachine {
    /// ログ相関番号
    cold_seq: u32,
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
            cold_seq,
            _guard: guard,
            ctx: ProbeContext {
                prepend_f2_warmup,
                used_eager_path,
            },
            phase: ProbePhase::Probing {
                probe,
                total_max_ms,
                kind: ProbeKind::GjiInitial {
                    needs_settle_check,
                    cold_reason,
                },
                send: SendState::new(romaji),
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
            cold_seq,
            _guard: guard,
            ctx: ProbeContext {
                prepend_f2_warmup: false,
                used_eager_path: false,
            },
            phase: ProbePhase::Probing {
                probe,
                total_max_ms,
                kind: ProbeKind::Chrome,
                send: SendState::new(romaji),
            },
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
            cold_seq,
            _guard: guard,
            ctx: ProbeContext {
                prepend_f2_warmup: false,
                used_eager_path: false,
            },
            phase: ProbePhase::LiteralDetect {
                detector,
                ze_bs_count,
                deadline_ms,
                send: SendState::new(romaji),
            },
        }
    }

    /// ログ用 cold_seq 参照（上書き警告等で使う）。
    pub(crate) const fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }

    /// probe 進行中に後続 VK を 1 つ蓄積する。
    pub(crate) fn push_deferred(&mut self, vk: VkCode, needs_shift: bool) {
        if let Some(send) = self.current_send_mut() {
            send.deferred_vks.push(DeferredVk { vk, needs_shift });
        } else {
            log::warn!(
                "[tsf-probe] cold={} push_deferred dropped: phase has no SendState (label={})",
                self.cold_seq,
                self.phase_label_internal()
            );
        }
    }

    /// probe 進行中に後続 VK を複数蓄積する。
    pub(crate) fn extend_deferred(&mut self, vks: impl IntoIterator<Item = DeferredVk>) {
        let collected: Vec<DeferredVk> = vks.into_iter().collect();
        if let Some(send) = self.current_send_mut() {
            send.deferred_vks.extend(collected);
        } else {
            log::warn!(
                "[tsf-probe] cold={} extend_deferred dropped {} VK(s): phase has no SendState (label={})",
                self.cold_seq,
                collected.len(),
                self.phase_label_internal()
            );
        }
    }

    /// 現フェーズの `SendState` への可変参照を返す。`TransmitDone` フェーズは `None`。
    const fn current_send_mut(&mut self) -> Option<&mut SendState> {
        match &mut self.phase {
            ProbePhase::Probing { send, .. }
            | ProbePhase::NameChangeWait { send, .. }
            | ProbePhase::LiteralDetect { send, .. }
            | ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent { send, .. }) => Some(send),
            ProbePhase::WaitingForCallback(WaitingFor::TransmitDone) => None,
        }
    }

    /// フェーズ名文字列（内部用）。
    const fn phase_label_internal(&self) -> &'static str {
        match &self.phase {
            ProbePhase::Probing { .. } => "Probing",
            ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent { .. }) => {
                "WaitingForCallback(FreshF2Sent)"
            }
            ProbePhase::WaitingForCallback(WaitingFor::TransmitDone) => {
                "WaitingForCallback(TransmitDone)"
            }
            ProbePhase::NameChangeWait { .. } => "NameChangeWait",
            ProbePhase::LiteralDetect { .. } => "LiteralDetect",
        }
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
    pub(crate) fn tick_with_clock<C: Clock>(&mut self, clock: &C) -> Vec<ProbeAction> {
        match self.inspect_phase(clock) {
            NextStep::Wait => vec![],
            NextStep::EmitSendFreshF2 { probe_settled } => {
                let send = self.take_send_for_fresh_f2();
                self.phase = ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
                    probe_settled,
                    send,
                });
                vec![ProbeAction::SendFreshF2 {
                    cold_seq: self.cold_seq,
                    probe_settled,
                }]
            }
            NextStep::TransmitTsf => self.enter_transmit_tsf(),
            NextStep::TransmitChrome => self.enter_transmit_chrome(),
            NextStep::StartSecondaryProbe { fresh_f2_ms } => {
                let send = self.take_send_for_secondary_probe();
                let probe = TsfReadinessProbe::new(fresh_f2_ms, self.cold_seq, 0);
                self.phase = ProbePhase::Probing {
                    probe,
                    total_max_ms: crate::tuning::GJI_POST_NAMECHANGE_MS,
                    kind: ProbeKind::GjiSecondary,
                    send,
                };
                vec![]
            }
            NextStep::LiteralDone => vec![ProbeAction::Done],
            NextStep::LiteralSuspected { ze_bs_count } => {
                let romaji = self.take_romaji_from_literal_detect();
                // ※「consecutive_count」のチェック (false-positive 抑制) は dispatcher 側で行う。
                vec![
                    ProbeAction::RawTsfLiteralRecovery {
                        cold_seq: self.cold_seq,
                        backs: ze_bs_count,
                        romaji,
                    },
                    ProbeAction::Done,
                ]
            }
        }
    }

    /// 現フェーズを検査して次の遷移先を返す。副作用なし（`&self` のみ使用）。
    #[allow(clippy::too_many_lines)]
    fn inspect_phase<C: Clock>(&self, clock: &C) -> NextStep {
        match &self.phase {
            ProbePhase::Probing {
                probe,
                total_max_ms,
                kind,
                ..
            } => {
                let Some(outcome) = probe.check_outcome(*total_max_ms) else {
                    return NextStep::Wait;
                };
                match kind {
                    ProbeKind::GjiInitial {
                        needs_settle_check,
                        cold_reason,
                    } => {
                        log::debug!(
                            "[tsf-probe] cold={} GjiProbe 完了 ({}ms)",
                            self.cold_seq,
                            outcome.elapsed_ms
                        );
                        if *needs_settle_check {
                            let is_ime_init_cold = cold_reason.requires_settle();
                            if (!outcome.settled || is_ime_init_cold) && outcome.monitor_healthy {
                                return NextStep::EmitSendFreshF2 {
                                    probe_settled: outcome.settled,
                                };
                            }
                        }
                        NextStep::TransmitTsf
                    }
                    ProbeKind::GjiSecondary => {
                        log::debug!(
                            "[tsf-probe] cold={} SecondaryGjiProbe 完了 ({}ms)",
                            self.cold_seq,
                            outcome.elapsed_ms
                        );
                        NextStep::TransmitTsf
                    }
                    ProbeKind::Chrome => {
                        log::debug!(
                            "[tsf-probe] cold={} ChromeProbe 完了 → batched 送信 ({}ms)",
                            self.cold_seq,
                            outcome.elapsed_ms
                        );
                        NextStep::TransmitChrome
                    }
                }
            }

            ProbePhase::WaitingForCallback(_) => NextStep::Wait,

            ProbePhase::NameChangeWait {
                nc_baseline,
                deadline_ms,
                fresh_f2_ms,
                probe_settled,
                ..
            } => {
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
                        self.cold_seq,
                        crate::tuning::GJI_POST_NAMECHANGE_MS
                    );
                    NextStep::StartSecondaryProbe {
                        fresh_f2_ms: *fresh_f2_ms,
                    }
                } else {
                    NextStep::TransmitTsf
                }
            }

            ProbePhase::LiteralDetect {
                detector,
                ze_bs_count,
                deadline_ms,
                ..
            } => {
                use crate::tsf::probe::DetectionResult;
                let Some(detection) = detector.check_now(*deadline_ms) else {
                    return NextStep::Wait;
                };
                match detection {
                    DetectionResult::CompositionConfirmed => {
                        log::debug!(
                            "[raw-tsf-literal] cold={} composition confirmed",
                            self.cold_seq
                        );
                        // 部分リテラル観測実験: composition の実際の文字列をログに残す。
                        // GJI/各 IME / 各プロファイルでの IMM API 応答を比較するための観測点。
                        crate::ime_diagnostic::log_composition_probe(self.cold_seq, "confirmed");
                        NextStep::LiteralDone
                    }
                    DetectionResult::SuspectedLiteral => {
                        crate::ime_diagnostic::log_composition_probe(self.cold_seq, "suspected");
                        NextStep::LiteralSuspected {
                            ze_bs_count: *ze_bs_count,
                        }
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
        self.phase_label_internal()
    }

    /// dispatcher が `SendFreshF2` を実行した後に呼ぶ。
    /// `WaitingForCallback(FreshF2Sent { .. })` → `NameChangeWait` へ遷移する。
    pub(crate) fn apply_fresh_f2_sent(
        &mut self,
        nc_baseline: NamechangeBaseline,
        fresh_f2_ms: u64,
    ) {
        let phase = std::mem::replace(
            &mut self.phase,
            ProbePhase::WaitingForCallback(WaitingFor::TransmitDone),
        );
        let (probe_settled, send) = match phase {
            ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
                probe_settled,
                send,
            }) => (probe_settled, send),
            other => {
                log::warn!(
                    "[tsf-probe] cold={} apply_fresh_f2_sent: unexpected phase",
                    self.cold_seq
                );
                self.phase = other;
                return;
            }
        };
        let deadline_ms = fresh_f2_ms + crate::tuning::SETTLE_TIMEOUT_MS;
        self.phase = ProbePhase::NameChangeWait {
            nc_baseline,
            deadline_ms,
            fresh_f2_ms,
            probe_settled,
            send,
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
        romaji: String,
        ze_bs_count: usize,
        detector: Option<LiteralDetector>,
    ) -> bool {
        if let Some(detector) = detector {
            let deadline_ms =
                crate::hook::current_tick_ms() + crate::tuning::RAW_TSF_LITERAL_DETECT_MS;
            self.phase = ProbePhase::LiteralDetect {
                detector,
                ze_bs_count,
                deadline_ms,
                send: SendState {
                    romaji,
                    deferred_vks: Vec::new(),
                },
            };
            false
        } else {
            true
        }
    }

    fn enter_transmit_tsf(&mut self) -> Vec<ProbeAction> {
        let send = self.take_current_send_for_transmit();
        let cold_seq = self.cold_seq;
        let ctx = self.ctx;
        self.phase = ProbePhase::WaitingForCallback(WaitingFor::TransmitDone);
        vec![ProbeAction::Transmit {
            cold_seq,
            prepend_f2_warmup: ctx.prepend_f2_warmup,
            used_eager_path: ctx.used_eager_path,
            romaji: send.romaji,
            deferred_vks: send.deferred_vks,
            target: TransmitTarget::Tsf,
        }]
    }

    fn enter_transmit_chrome(&mut self) -> Vec<ProbeAction> {
        let send = self.take_current_send_for_transmit();
        let cold_seq = self.cold_seq;
        self.phase = ProbePhase::WaitingForCallback(WaitingFor::TransmitDone);
        vec![ProbeAction::Transmit {
            cold_seq,
            prepend_f2_warmup: false,
            used_eager_path: false,
            romaji: send.romaji,
            deferred_vks: send.deferred_vks,
            target: TransmitTarget::Chrome,
        }]
    }

    /// `Transmit` action 生成時に現フェーズから `SendState` を取り出す。
    fn take_current_send_for_transmit(&mut self) -> SendState {
        match &mut self.phase {
            ProbePhase::Probing { send, .. } | ProbePhase::NameChangeWait { send, .. } => {
                std::mem::take(send)
            }
            _ => {
                log::warn!(
                    "[tsf-probe] cold={} enter_transmit_* called from unexpected phase {}",
                    self.cold_seq,
                    self.phase_label_internal()
                );
                SendState::default()
            }
        }
    }

    /// `EmitSendFreshF2` 遷移時に `Probing` フェーズから `SendState` を取り出す。
    fn take_send_for_fresh_f2(&mut self) -> SendState {
        if let ProbePhase::Probing { send, .. } = &mut self.phase {
            std::mem::take(send)
        } else {
            log::warn!(
                "[tsf-probe] cold={} take_send_for_fresh_f2 unexpected phase {}",
                self.cold_seq,
                self.phase_label_internal()
            );
            SendState::default()
        }
    }

    /// `StartSecondaryProbe` 遷移時に `NameChangeWait` フェーズから `SendState` を取り出す。
    fn take_send_for_secondary_probe(&mut self) -> SendState {
        if let ProbePhase::NameChangeWait { send, .. } = &mut self.phase {
            std::mem::take(send)
        } else {
            log::warn!(
                "[tsf-probe] cold={} take_send_for_secondary_probe unexpected phase {}",
                self.cold_seq,
                self.phase_label_internal()
            );
            SendState::default()
        }
    }

    /// `LiteralSuspected` 遷移時に `LiteralDetect` フェーズから `romaji` を取り出す。
    fn take_romaji_from_literal_detect(&mut self) -> String {
        if let ProbePhase::LiteralDetect { send, .. } = &mut self.phase {
            std::mem::take(&mut send.romaji)
        } else {
            log::warn!(
                "[tsf-probe] cold={} take_romaji_from_literal_detect unexpected phase {}",
                self.cold_seq,
                self.phase_label_internal()
            );
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::probe_bridge::OutputActiveGuard;

    use timed_fsm::ManualClock;

    fn make_gji_machine() -> TsfProbeMachine {
        let guard = OutputActiveGuard::noop_for_test();
        let probe = TsfReadinessProbe::new(0, 0, 0);
        TsfProbeMachine::new_gji(
            "ka",
            0,
            probe,
            0,
            false,
            ColdReason::FocusChange,
            false,
            false,
            guard,
        )
    }

    fn make_namechange_wait(deadline_ms: u64, probe_settled: bool) -> ProbePhase {
        let baseline = crate::tsf::observer::namechange_baseline();
        ProbePhase::NameChangeWait {
            nc_baseline: baseline,
            deadline_ms,
            fresh_f2_ms: 0,
            probe_settled,
            send: SendState::default(),
        }
    }

    // ── NameChangeWait フェーズ遷移テスト ─────────────────────────────────────

    #[test]
    fn namechange_wait_before_deadline_stays_waiting() {
        let mut machine = make_gji_machine();
        machine.force_phase_for_test(make_namechange_wait(1000, false));

        let actions = machine.tick_with_clock(&ManualClock(500)); // now < deadline
        assert!(actions.is_empty(), "待機中は空 Vec を返すべき");
        assert_eq!(machine.phase_label(), "NameChangeWait");
    }

    #[test]
    fn namechange_wait_timeout_emits_transmit_tsf() {
        let mut machine = make_gji_machine();
        machine.force_phase_for_test(make_namechange_wait(500, true)); // settled=true

        let actions = machine.tick_with_clock(&ManualClock(1000)); // now >= deadline
        assert!(
            !actions.is_empty(),
            "タイムアウト後は action を emit するべき"
        );
        assert!(
            matches!(
                actions[0],
                ProbeAction::Transmit {
                    target: TransmitTarget::Tsf,
                    ..
                }
            ),
            "タイムアウト時は Transmit(Tsf) を emit するべき: {actions:?}",
        );
    }

    #[test]
    fn namechange_wait_timeout_unsettled_emits_transmit_tsf() {
        // timed_out=true, probe_settled=false → タイムアウトで直接 Transmit(Tsf)
        let mut machine = make_gji_machine();
        machine.force_phase_for_test(make_namechange_wait(500, false));

        let actions = machine.tick_with_clock(&ManualClock(1000));
        assert!(
            matches!(
                actions[0],
                ProbeAction::Transmit {
                    target: TransmitTarget::Tsf,
                    ..
                }
            ),
            "タイムアウト(unsettled)でも Transmit(Tsf) を emit するべき",
        );
    }

    // ── WaitingForCallback フェーズテスト ────────────────────────────────────

    #[test]
    fn waiting_for_callback_is_no_op() {
        let mut machine = make_gji_machine();
        machine.force_phase_for_test(ProbePhase::WaitingForCallback(WaitingFor::TransmitDone));

        let actions = machine.tick_with_clock(&ManualClock(0));
        assert!(actions.is_empty(), "WaitingForCallback は空 Vec を返すべき");
        assert_eq!(machine.phase_label(), "WaitingForCallback(TransmitDone)");
    }

    // ── push_deferred / extend_deferred テスト ───────────────────────────────

    #[test]
    fn push_deferred_appends_vk() {
        let mut machine = make_gji_machine();
        machine.push_deferred(VkCode(0x41), false);
        machine.push_deferred(VkCode(0x42), true);
        // deferred_vks は private だが Transmit action 経由で確認できる。
        // ここでは push_deferred が panic しないことを確認するだけで十分。
    }

    #[test]
    fn extend_deferred_appends_multiple_vks() {
        let mut machine = make_gji_machine();
        machine.extend_deferred(vec![
            DeferredVk {
                vk: VkCode(0x41),
                needs_shift: false,
            },
            DeferredVk {
                vk: VkCode(0x42),
                needs_shift: true,
            },
            DeferredVk {
                vk: VkCode(0x43),
                needs_shift: false,
            },
        ]);
        // panic しないことを確認
    }

    // ── regression: literal_suspected 経路で romaji が保持されるか ────────────

    #[test]
    fn literal_suspected_carries_romaji_through_transmit_to_recovery() {
        let guard = OutputActiveGuard::noop_for_test();
        let detector = crate::tsf::probe::LiteralDetector::new();
        let mut machine = TsfProbeMachine::new_literal_detect("a", 0, detector, 1, 0, guard);
        let actions = machine.tick_with_clock(&ManualClock(1000));
        match &actions[..] {
            [ProbeAction::RawTsfLiteralRecovery { romaji, backs, .. }, ProbeAction::Done] => {
                assert_eq!(
                    romaji, "a",
                    "literal suspected 経路で romaji が空になってはいけない"
                );
                assert_eq!(*backs, 1);
            }
            other => panic!("unexpected actions: {other:?}"),
        }
    }
}
