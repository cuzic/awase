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
//!                                                                                        ├─[nc_fired && !settled]─► Probing(GjiSecondary) ─► WaitingForCallback
//!                                                                                        └─[その他]────────────────────────────────────────────────────────► WaitingForCallback
//! Probing(Chrome) ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────► (TransmitChrome 後に即完了)
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
    /// NameChangeWait フェーズの deadline budget (ms)。GjiFsm の ColdKind 由来。
    ncwait_budget_ms: u64,
    /// F2 をバッチに強制同梱するか（GjiFsm の ColdKind::Medium/Long で true）。
    /// `SendFreshF2` dispatch 時に追加 F2 (F2×2) を送るかどうかにも使う。
    forces_prepend_f2: bool,
    /// GjiFsm の ColdKind::Long（≥10s idle）か。
    /// `literal_detect_ms` 延長（RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE）の判定に使う。
    is_long_cold: bool,
    /// プローブ開始前に VK_DBE_HIRAGANA pair が送信済みか（ReWarmup / FreshF2 / non-eager）。
    /// true のとき TSF モードでバッチへの F2 重複送信を抑制する。
    fresh_f2_at_probe_start: bool,
}

/// `tick()` 呼び出し時に注入する環境観測値のスナップショット。
/// テストでは任意値を注入できる。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TsfEnvSnapshot {
    pub is_tsf_mode: bool,
    pub gji_active: bool,
    /// `LiteralDetect` の部分リテラル検出で参照する現在 composition 先頭文字。
    /// `crate::ime::get_foreground_comp_str_char()` のスナップショット。
    /// `None` = HIMC 未取得（テスト・LiteralDetect 非活性時）→ 部分リテラル検出をスキップ。
    pub foreground_comp_char: Option<char>,
    /// `NameChangeWait` の早期脱出判定。`GoogleJapaneseInputCandidateWindow` が現在表示中なら true。
    /// true = composition active → OBJ_NAMECHANGE を待たず即 transmit（WezTerm 連続入力の 300ms 削減）。
    /// `crate::tsf::observer::gji_candidate_visible_now()` のスナップショット。
    pub gji_candidate_visible: bool,
}

/// probe 中に観測した事実。GjiFsm bridge・WarmupPath 分類に使う。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProbeObservations {
    pub nc_fired: bool,
    pub gji_resumed: bool,
}

/// `decide_transmit_plan` が確定した実行方針。dispatcher がそのまま実行する。
#[derive(Debug, Clone, Copy)]
pub(crate) struct TransmitPlan {
    /// バッチ送信に F2 (VK_DBE_HIRAGANA) を先行させるか。
    pub should_prepend_f2: bool,
    /// VK ローマ字パス（true）か Unicode TSF パス（false）か。
    pub used_eager_path: bool,
    /// raw TSF literal の LiteralDetect フェーズを有効にするか。
    pub needs_literal: bool,
    /// LiteralDetect のタイムアウト期間（ms）。
    pub literal_detect_ms: u64,
}

/// probe 観測値・環境スナップショット・コンテキストから送信方針を決定する純粋計算関数。
///
/// FSM 内の `enter_transmit_tsf` から呼ばれるが、単独でテストも可能。
/// 副作用なし。
pub(crate) fn decide_transmit_plan(
    initial_prepend_f2: bool,
    initial_used_eager: bool,
    obs: ProbeObservations,
    env: &TsfEnvSnapshot,
    deferred_empty: bool,
    forces_prepend_f2: bool,
    is_long_cold: bool,
) -> TransmitPlan {
    // nc_fired=false + TSF mode (WezTerm) + !forces_prepend_f2 (Short cold):
    // SendFreshF2 action が ~300ms 前に fresh F2 を送信済み → WezTerm の TSF context は
    // 既に初期化済み。バッチに再び F2 を含めると WezTerm が TSF reinit を再トリガーし、
    // 同一バッチの先頭 VK が reinit 中に届いてリテラル化する race が起きる。
    // Medium/Long cold (forces_prepend_f2=true): GJI が F2×2 で起動中 or 無応答のため F2 同梱が必要。
    let should_prepend_f2 = initial_prepend_f2
        && (obs.nc_fired || !env.is_tsf_mode || forces_prepend_f2);

    // gji_resumed=true: GJI が F2×2 に I/O 応答 → VK path 強制。
    // nc_fired=true: IME モード確認済み（Medium/Long cold かつ deferred なし → VK path）。
    // nc_fired=false + TSF mode: VK path 固定（unicode は GJI composition をバイパスし "nお" race が起きる）。
    let used_eager_path = if obs.gji_resumed {
        false
    } else if obs.nc_fired {
        (initial_used_eager || forces_prepend_f2) && deferred_empty
    } else if env.is_tsf_mode {
        false
    } else {
        initial_used_eager || forces_prepend_f2
    };

    // gji_resumed=true: GJI I/O 応答確認済み → composition 成功が確定。
    // Long cold (is_long_cold=true) 後は候補ウィンドウ表示に >500ms かかり false positive になる
    // （BS 誤送信 → context 破壊）。Medium cold は 7-10s で gji_long_idle_probe=true のため対象外。
    //
    // nc_fired=false + TSF mode 節:
    // NameChangeWait タイムアウトで IME 準備が未確認のまま transmit したケースを
    // LiteralDetect で回収する。F2 をバッチに含めない場合も IME が cold の可能性がある。
    // （seつぞく バグ: gji_idle ~1.4s で 300ms NameChangeWait が間に合わなかったケース）
    let needs_literal = (should_prepend_f2
        && env.gji_active
        && (!is_long_cold || env.is_tsf_mode)
        && !obs.gji_resumed)
        || (!obs.nc_fired && env.is_tsf_mode && env.gji_active && !obs.gji_resumed);

    let literal_detect_ms = if is_long_cold && env.is_tsf_mode {
        crate::tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE
    } else {
        crate::tuning::RAW_TSF_LITERAL_DETECT_MS
    };

    TransmitPlan {
        should_prepend_f2,
        used_eager_path,
        needs_literal,
        literal_detect_ms,
    }
}

/// `inspect_phase` が返す「次に何をすべきか」の宣言。
///
/// `tick_with_clock` はこれを受け取って状態遷移と `ProbeAction` emit を行う。
/// `inspect_phase` は `&self` のみ使用（副作用なし）。
enum NextStep {
    /// 現フェーズで継続待機（actions なし）
    Wait,
    /// `WaitingForCallback(FreshF2Sent { .. })` へ遷移し `SendFreshF2` を emit
    EmitSendFreshF2 {
        probe_settled: bool,
    },
    /// `enter_transmit_tsf()` を呼ぶ
    TransmitTsf { nc_fired: bool, gji_resumed: bool },
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
        /// NameChangeWait フェーズの deadline budget (ms)。
        /// `ProbeContext::ncwait_budget_ms`（GjiFsm ColdKind 由来）をコピーして保持する。
        budget_ms: u64,
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
        /// SHOW 発火時に IMM32 composition と突き合わせる期待かな文字。
        /// None の場合は IMM32 検証をスキップして CompositionConfirmed を返す。
        expected_kana: Option<char>,
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
        /// FSM が確定した実行方針。dispatcher はそのまま実行する（再導出不要）。
        plan: TransmitPlan,
        /// probe 中に観測した事実。GjiFsm bridge・WarmupPath 分類に使う。
        observations: ProbeObservations,
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
    /// RAII guard。drop で `OUTPUT_GATE.active=false`。
    ///
    /// GJI probe では `None`（guard は `Output::gji_probe_guard` で管理する）。
    /// Chrome / LiteralDetect probe では `Some` を保持する。
    _guard: Option<OutputActiveGuard>,
    /// 送信コンテキスト（`enter_transmit_*` で `ProbeAction` に畳み込む）
    ctx: ProbeContext,
    /// 現在フェーズ
    phase: ProbePhase,
}

impl TsfProbeMachine {
    /// TSF cold warmup (`send_romaji_as_tsf` の cold パス) 用コンストラクタ。
    ///
    /// guard は渡さない。`Output::gji_probe_guard` で管理する（Task 2 以降）。
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
        ncwait_budget_ms: u64,
        forces_prepend_f2: bool,
        is_long_cold: bool,
        fresh_f2_at_probe_start: bool,
    ) -> Self {
        Self {
            cold_seq,
            _guard: None,
            ctx: ProbeContext {
                prepend_f2_warmup,
                used_eager_path,
                ncwait_budget_ms,
                forces_prepend_f2,
                is_long_cold,
                fresh_f2_at_probe_start,
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
            _guard: Some(guard),
            ctx: ProbeContext {
                prepend_f2_warmup: false,
                used_eager_path: false,
                ncwait_budget_ms: crate::tuning::SETTLE_TIMEOUT_MS,
                forces_prepend_f2: false,
                is_long_cold: false,
                fresh_f2_at_probe_start: false,
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
            _guard: Some(guard),
            ctx: ProbeContext {
                prepend_f2_warmup: false,
                used_eager_path: false,
                ncwait_budget_ms: crate::tuning::SETTLE_TIMEOUT_MS,
                forces_prepend_f2: false,
                is_long_cold: false,
                fresh_f2_at_probe_start: false,
            },
            phase: ProbePhase::LiteralDetect {
                detector,
                ze_bs_count,
                deadline_ms,
                send: SendState::new(romaji),
                expected_kana: None,
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
    pub(crate) fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.tick_impl(&SystemClock, env)
    }

    /// `Clock` と `TsfEnvSnapshot` を注入できる `tick` の内部実装（テスト用）。
    ///
    /// `inspect_phase` で副作用なしに次の遷移先を決定し、
    /// その後 `&self` を解放してから状態変化を適用する。
    #[cfg(test)]
    pub(crate) fn tick_with_clock_env<C: Clock>(
        &mut self,
        clock: &C,
        env: &TsfEnvSnapshot,
    ) -> Vec<ProbeAction> {
        self.tick_impl(clock, env)
    }

    fn tick_impl<C: Clock>(&mut self, clock: &C, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        match self.inspect_phase(clock, env) {
            NextStep::Wait => vec![],
            NextStep::EmitSendFreshF2 { probe_settled } => {
                let send = self.take_send_for_fresh_f2();
                self.phase = ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
                    probe_settled,
                    budget_ms: self.ctx.ncwait_budget_ms,
                    send,
                });
                vec![ProbeAction::SendFreshF2 {
                    cold_seq: self.cold_seq,
                    probe_settled,
                }]
            }
            NextStep::TransmitTsf { nc_fired, gji_resumed } => {
                self.enter_transmit_tsf(nc_fired, gji_resumed, env)
            }
            NextStep::TransmitChrome => self.enter_transmit_chrome(env),
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
    fn inspect_phase<C: Clock>(&self, clock: &C, env: &TsfEnvSnapshot) -> NextStep {
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
                            "[tsf-probe] cold={} GjiProbe 完了 ({}ms, gji_idle={}ms, settled={})",
                            self.cold_seq,
                            outcome.elapsed_ms,
                            outcome.gji_idle_ms,
                            outcome.settled,
                        );
                        if *needs_settle_check {
                            let is_ime_init_cold = cold_reason.requires_settle();
                            if (!outcome.settled || is_ime_init_cold) && outcome.monitor_healthy {
                                // GJI がウォームアップ前から既にアイドルだった場合（long_idle）は、
                                // fresh F2 を送っても NAMECHANGE は発火しない（まだキーを打っていないため）。
                                // forces_prepend_f2=true なら F2 はキーバッチに含まれるので安全にスキップできる。
                                let pre_idle = outcome.gji_idle_ms
                                    >= outcome.elapsed_ms.saturating_add(crate::tuning::GJI_IDLE_MS);
                                if !outcome.settled && outcome.monitor_healthy && pre_idle && self.ctx.forces_prepend_f2 {
                                    log::debug!(
                                        "[tsf-probe] cold={} GJI pre-idle (idle={}ms elapsed={}ms forces_f2=true) → skip fresh F2",
                                        self.cold_seq,
                                        outcome.gji_idle_ms,
                                        outcome.elapsed_ms,
                                    );
                                    return NextStep::TransmitTsf {
                                        nc_fired: false,
                                        gji_resumed: false,
                                    };
                                }
                                return NextStep::EmitSendFreshF2 {
                                    probe_settled: outcome.settled,
                                };
                            }
                        }
                        NextStep::TransmitTsf {
                            nc_fired: true,
                            gji_resumed: false,
                        }
                    }
                    ProbeKind::GjiSecondary => {
                        log::debug!(
                            "[tsf-probe] cold={} SecondaryGjiProbe 完了 ({}ms)",
                            self.cold_seq,
                            outcome.elapsed_ms
                        );
                        // GjiSecondary は NameChange 発火後の二次プローブなので nc_fired=true。
                        NextStep::TransmitTsf {
                            nc_fired: true,
                            gji_resumed: false,
                        }
                    }
                    ProbeKind::Chrome => {
                        log::debug!(
                            "[tsf-probe] cold={} ChromeProbe 完了 ({}ms)",
                            self.cold_seq,
                            outcome.elapsed_ms
                        );
                        // F22→F21 を Chrome path で使わない。
                        // F22 (IME OFF) が Chrome TSF context を壊し、F21 が間に合わず na がリテラル化する。
                        // ChromeProbe の F2 (VK_DBE_HIRAGANA) warmup のみで GJI を活性化する。
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

                // candidate 窓が表示中 = composition active → OBJ_NAMECHANGE を待たず即 transmit。
                // fresh F2 は既に送信済みのためバッチに F2 を含めない (nc_fired=false の plan 経路)。
                // LiteralDetect は有効のまま: IME mode が未確認の状態を安全に回収する。
                // （WezTerm 連続入力時に毎文字 ~300ms の NameChangeWait タイムアウトを削減する）
                if env.gji_candidate_visible {
                    let elapsed = now.saturating_sub(*fresh_f2_ms);
                    log::debug!(
                        "[tsf-probe] cold={} NameChangeWait: candidate visible → composition active ({elapsed}ms)",
                        self.cold_seq
                    );
                    return NextStep::TransmitTsf { nc_fired: false, gji_resumed: false };
                }

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
                    // nc_fired=false（タイムアウト）の場合は IME モード切替が未確認。
                    // nc_fired=true && probe_settled=true は上の if に入らないのでここに来る。
                    NextStep::TransmitTsf {
                        nc_fired,
                        gji_resumed: false,
                    }
                }
            }

            ProbePhase::LiteralDetect {
                detector,
                ze_bs_count,
                deadline_ms,
                expected_kana,
                ..
            } => {
                use crate::tsf::probe::DetectionResult;
                let Some(detection) = detector.check_now(*deadline_ms) else {
                    return NextStep::Wait;
                };
                match detection {
                    DetectionResult::CompositionConfirmed => {
                        // 部分リテラル検出: SHOW が発火しても composition 内容が expected_kana と
                        // 異なる場合は「K がリテラル化して O だけが compose された」部分リテラルを示す。
                        // TSF→IMM32 bridge が composition 中に HIMC を更新する環境でのみ有効。
                        // HIMC が NULL の場合（bridge 未対応など）は None → スキップして安全に CompositionConfirmed へ。
                        if let Some(expected) = expected_kana {
                            // env.foreground_comp_char は output/mod.rs が取得して注入したスナップショット。
                            // None = HIMC 未取得（テスト・bridge 非対応環境）→ 部分リテラル検出をスキップ。
                            let actual = env.foreground_comp_char;
                            if actual.map_or(false, |c| c != *expected) {
                                log::warn!(
                                    "[raw-tsf-literal] cold={} partial literal: comp={actual:?} ≠ expected='{expected}' → SuspectedLiteral",
                                    self.cold_seq
                                );
                                crate::ime_diagnostic::log_composition_probe(
                                    self.cold_seq,
                                    "partial-literal",
                                );
                                return NextStep::LiteralSuspected {
                                    ze_bs_count: *ze_bs_count,
                                };
                            }
                        }
                        log::debug!(
                            "[raw-tsf-literal] cold={} composition confirmed",
                            self.cold_seq
                        );
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

    /// `SendFreshF2` dispatch 時に追加 F2 (F2×2) を送るかを示す（probe 生成時の ColdKind 由来）。
    pub(crate) fn forces_prepend_f2_for_extra_f2(&self) -> bool {
        self.ctx.forces_prepend_f2
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
        let (probe_settled, budget_ms, send) = match phase {
            ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
                probe_settled,
                budget_ms,
                send,
            }) => (probe_settled, budget_ms, send),
            other => {
                log::warn!(
                    "[tsf-probe] cold={} apply_fresh_f2_sent: unexpected phase",
                    self.cold_seq
                );
                self.phase = other;
                return;
            }
        };
        // budget_ms は GjiFsm の ColdKind 由来（Short=300ms, Medium=550ms, Long=350ms）。
        let deadline_ms = fresh_f2_ms + budget_ms;
        log::debug!(
            "[tsf-probe] cold={} NameChangeWait deadline {}ms (probe_settled={probe_settled}, budget={budget_ms}ms forces_f2={})",
            self.cold_seq, budget_ms, self.ctx.forces_prepend_f2
        );
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
        literal_detect_ms: u64,
        expected_kana: Option<char>,
    ) -> bool {
        if let Some(detector) = detector {
            let deadline_ms = crate::hook::current_tick_ms() + literal_detect_ms;
            self.phase = ProbePhase::LiteralDetect {
                detector,
                ze_bs_count,
                deadline_ms,
                send: SendState {
                    romaji,
                    deferred_vks: Vec::new(),
                },
                expected_kana,
            };
            false
        } else {
            true
        }
    }

    fn enter_transmit_tsf(
        &mut self,
        nc_fired: bool,
        gji_resumed: bool,
        env: &TsfEnvSnapshot,
    ) -> Vec<ProbeAction> {
        let send = self.take_current_send_for_transmit();
        let cold_seq = self.cold_seq;
        let ctx = self.ctx;
        // ReWarmup / FreshF2 / non-eager パスではプローブ開始前に VK_DBE_HIRAGANA を送信済み。
        // TSF モード（WezTerm）でバッチに F2 を再送すると reinit が起きて先頭 VK がリテラル化する。
        let suppress_f2_in_batch = ctx.fresh_f2_at_probe_start && env.is_tsf_mode;
        if suppress_f2_in_batch {
            log::debug!(
                "[tsf-probe] cold={} fresh_f2_at_probe_start + TSF → F2 バッチ重複を抑制",
                cold_seq
            );
        }
        let effective_prepend_f2 = ctx.prepend_f2_warmup && !suppress_f2_in_batch;
        let observations = ProbeObservations { nc_fired, gji_resumed };
        let plan = decide_transmit_plan(
            effective_prepend_f2,
            ctx.used_eager_path,
            observations,
            env,
            send.deferred_vks.is_empty(),
            ctx.forces_prepend_f2,
            ctx.is_long_cold,
        );
        self.phase = ProbePhase::WaitingForCallback(WaitingFor::TransmitDone);
        vec![ProbeAction::Transmit {
            cold_seq,
            plan,
            observations,
            romaji: send.romaji,
            deferred_vks: send.deferred_vks,
            target: TransmitTarget::Tsf,
        }]
    }

    fn enter_transmit_chrome(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        let send = self.take_current_send_for_transmit();
        let cold_seq = self.cold_seq;
        self.phase = ProbePhase::WaitingForCallback(WaitingFor::TransmitDone);
        vec![ProbeAction::Transmit {
            cold_seq,
            plan: TransmitPlan {
                should_prepend_f2: false,
                used_eager_path: false,
                needs_literal: env.gji_active,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            observations: ProbeObservations { nc_fired: true, gji_resumed: false },
            romaji: send.romaji,
            deferred_vks: send.deferred_vks,
            target: TransmitTarget::Chrome,
        }]
    }

    /// `Transmit` action 生成時に現フェーズから `SendState` を取り出す。
    fn take_current_send_for_transmit(&mut self) -> SendState {
        match &mut self.phase {
            ProbePhase::Probing { send, .. }
            | ProbePhase::NameChangeWait { send, .. } => std::mem::take(send),
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
        make_gji_machine_with_cold(crate::tuning::SETTLE_TIMEOUT_MS, false)
    }

    fn make_gji_machine_with_cold(ncwait_budget_ms: u64, forces_prepend_f2: bool) -> TsfProbeMachine {
        let is_long_cold = ncwait_budget_ms == crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS;
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
            ncwait_budget_ms,
            forces_prepend_f2,
            is_long_cold,
            false,
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

        let actions = machine.tick_with_clock_env(&ManualClock(500), &TsfEnvSnapshot::default());
        assert!(actions.is_empty(), "待機中は空 Vec を返すべき");
        assert_eq!(machine.phase_label(), "NameChangeWait");
    }

    #[test]
    fn namechange_wait_candidate_visible_exits_immediately() {
        // WezTerm 連続入力: candidate 窓が既に表示中 → 300ms タイムアウトを待たず即 transmit。
        // nc_fired=false の plan 経路（prepend_f2=false、needs_literal=true）であることを検証する。
        let mut machine = make_gji_machine();
        machine.force_phase_for_test(make_namechange_wait(10_000, true)); // deadline は遠い未来

        let env = TsfEnvSnapshot {
            gji_candidate_visible: true,
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };
        let actions = machine.tick_with_clock_env(&ManualClock(500), &env);

        assert!(!actions.is_empty(), "candidate visible → 即 Transmit を emit するべき");
        assert!(
            matches!(actions[0], ProbeAction::Transmit { target: TransmitTarget::Tsf, .. }),
            "candidate visible: Transmit(Tsf) を emit するべき: {actions:?}",
        );
        if let ProbeAction::Transmit { observations, plan, .. } = &actions[0] {
            assert!(!observations.nc_fired, "candidate visible 経路: nc_fired=false");
            assert!(!plan.should_prepend_f2, "candidate visible 経路: 余分な F2 はバッチに含めない");
            assert!(plan.needs_literal, "candidate visible 経路: IME mode 未確認 → LiteralDetect 有効");
        }
    }

    #[test]
    fn namechange_wait_timeout_emits_transmit_tsf() {
        let mut machine = make_gji_machine();
        machine.force_phase_for_test(make_namechange_wait(500, true)); // settled=true

        let actions = machine.tick_with_clock_env(&ManualClock(1000), &TsfEnvSnapshot::default());
        assert!(
            !actions.is_empty(),
            "タイムアウト後は action を emit するべき"
        );
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

        let actions = machine.tick_with_clock_env(&ManualClock(1000), &TsfEnvSnapshot::default());
        assert!(
            matches!(actions[0], ProbeAction::Transmit { target: TransmitTarget::Tsf, .. }),
            "タイムアウト(unsettled)でも Transmit(Tsf) を emit するべき: {actions:?}",
        );
        if let ProbeAction::Transmit { observations, .. } = &actions[0] {
            assert!(!observations.nc_fired, "タイムアウトなので nc_fired=false が必須");
        }
    }

    // 再現テスト: cold=7 "このろぐ → kおのろぐ" バグ
    // ColdKind::Medium (7s-10s idle) で apply_fresh_f2_sent が
    // MEDIUM_IDLE_PROBE_TOTAL_MS タイムアウトで NameChangeWait へ遷移することを確認する。
    #[test]
    fn apply_fresh_f2_sent_medium_cold_uses_extended_budget() {
        let baseline = crate::tsf::observer::namechange_baseline();
        // ColdKind::Medium: forces_prepend_f2=true, budget=MEDIUM_IDLE_PROBE_TOTAL_MS
        let mut machine = make_gji_machine_with_cold(
            crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS,
            true,
        );
        machine.force_phase_for_test(ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
            probe_settled: false,
            budget_ms: crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS,
            send: SendState::default(),
        }));
        let fresh_f2_ms: u64 = 1000;
        machine.apply_fresh_f2_sent(baseline, fresh_f2_ms);

        // NameChangeWait に遷移し、延長タイムアウトであることを確認。
        // GJI が F2 への応答に ~325ms かかっても 550ms 以内に OBJ_NAMECHANGE を受け取れる。
        assert_eq!(machine.phase_label(), "NameChangeWait");
        if let ProbePhase::NameChangeWait { deadline_ms, .. } = &machine.phase {
            let expected_deadline = fresh_f2_ms + crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS;
            assert_eq!(
                *deadline_ms, expected_deadline,
                "medium cold タイムアウト = MEDIUM_IDLE_PROBE_TOTAL_MS ({}ms) が必須",
                crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS
            );
        } else {
            panic!("NameChangeWait フェーズになるべき: {:?}", machine.phase_label());
        }
    }

    #[test]
    fn apply_fresh_f2_sent_long_cold_uses_long_probe_timeout() {
        let baseline = crate::tsf::observer::namechange_baseline();
        // ColdKind::Long: forces_prepend_f2=true, budget=GJI_LONG_IDLE_PROBE_TOTAL_MS
        let mut machine = make_gji_machine_with_cold(
            crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS,
            true,
        );
        machine.force_phase_for_test(ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent {
            probe_settled: false,
            budget_ms: crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS,
            send: SendState::default(),
        }));
        let fresh_f2_ms: u64 = 1000;
        machine.apply_fresh_f2_sent(baseline, fresh_f2_ms);

        if let ProbePhase::NameChangeWait { deadline_ms, .. } = &machine.phase {
            let expected_deadline = fresh_f2_ms + crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS;
            assert_eq!(
                *deadline_ms, expected_deadline,
                "long cold タイムアウト = GJI_LONG_IDLE_PROBE_TOTAL_MS ({}ms) が必須",
                crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS
            );
        } else {
            panic!("NameChangeWait フェーズになるべき: {:?}", machine.phase_label());
        }
    }

    // ── WaitingForCallback フェーズテスト ────────────────────────────────────

    #[test]
    fn waiting_for_callback_is_no_op() {
        let mut machine = make_gji_machine();
        machine.force_phase_for_test(ProbePhase::WaitingForCallback(WaitingFor::TransmitDone));

        let actions = machine.tick_with_clock_env(&ManualClock(0), &TsfEnvSnapshot::default());
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
        let actions = machine.tick_with_clock_env(&ManualClock(1000), &TsfEnvSnapshot::default());
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

    // ── decide_transmit_plan 回帰テスト ──────────────────────────────────────

    #[test]
    fn decide_plan_nc_not_fired_tsf_not_long_idle_suppresses_f2_but_keeps_literal() {
        // Bug 1 再発防止: "さいごの" → "sあいごの"
        // nc_fired=false + TSF mode (WezTerm) + Short cold (forces_prepend_f2=false):
        // SendFreshF2 が ~300ms 前に送信済み → バッチに F2 を含めると reinit race でリテラル化する。
        // ただし IME 準備完了が未確認のため LiteralDetect は有効にして回収する。
        // （seつぞく バグ再発防止: gji_idle ~1.4s で nc_fired=false のまま timeout したケース）
        let obs = ProbeObservations { nc_fired: false, gji_resumed: false };
        let env = TsfEnvSnapshot { is_tsf_mode: true, gji_active: true, ..Default::default() };

        let plan = decide_transmit_plan(true, false, obs, &env, true, false, false);

        assert!(!plan.should_prepend_f2, "nc_fired=false + tsf + Short cold: F2 をバッチに含めない");
        assert!(plan.needs_literal, "nc_fired=false + tsf: IME 準備未確認 → LiteralDetect で保護");
    }

    #[test]
    fn decide_plan_gji_resumed_disables_literal_detection() {
        // Bug 2 再発防止: gji_resumed=true 時の false positive BS
        // GJI が F2×2 に I/O 応答済み → composition 成功確定 → LiteralDetect は false positive になる。
        let obs = ProbeObservations { nc_fired: true, gji_resumed: true };
        let env = TsfEnvSnapshot { is_tsf_mode: true, gji_active: true, ..Default::default() };

        let plan = decide_transmit_plan(true, false, obs, &env, true, true, true);

        assert!(!plan.needs_literal, "gji_resumed=true: LiteralDetect をスキップしないと BS 誤送信になる");
    }

    #[test]
    fn decide_plan_nc_not_fired_tsf_long_idle_keeps_f2_and_literal() {
        // Long cold (forces_prepend_f2=true): F2×2 で GJI を起動する必要があるためバッチに F2 が必要。
        // LiteralDetect も有効（GJI 応答未確認）。
        let obs = ProbeObservations { nc_fired: false, gji_resumed: false };
        let env = TsfEnvSnapshot { is_tsf_mode: true, gji_active: true, ..Default::default() };

        let plan = decide_transmit_plan(true, false, obs, &env, true, true, true);

        assert!(plan.should_prepend_f2, "Long cold: F2 バッチ同梱が必要");
        assert!(plan.needs_literal, "gji_resumed=false + tsf: LiteralDetect 有効");
    }

    #[test]
    fn decide_plan_nc_fired_keeps_f2_and_enables_literal_when_gji_active() {
        // nc_fired=true: NameChange 発火確認済み → F2 はバッチに含める。
        let obs = ProbeObservations { nc_fired: true, gji_resumed: false };
        let env = TsfEnvSnapshot { is_tsf_mode: true, gji_active: true, ..Default::default() };

        let plan = decide_transmit_plan(true, false, obs, &env, true, false, false);

        assert!(plan.should_prepend_f2, "nc_fired=true: F2 はバッチに含める");
        assert!(plan.needs_literal, "gji_active + !is_long_cold + !gji_resumed: LiteralDetect 有効");
    }

    #[test]
    fn decide_plan_non_tsf_mode_keeps_f2() {
        // 非 TSF mode（Chrome 等）: nc_fired=false でも F2 バッチ同梱が必要。
        let obs = ProbeObservations { nc_fired: false, gji_resumed: false };
        let env = TsfEnvSnapshot { is_tsf_mode: false, gji_active: true, ..Default::default() };

        let plan = decide_transmit_plan(true, false, obs, &env, true, false, false);

        assert!(plan.should_prepend_f2, "非 TSF mode: nc_fired=false でも F2 を含める");
    }

    #[test]
    fn decide_plan_nc_fired_with_deferred_vks_disables_eager_path() {
        // nc_fired=true でも deferred_vks が存在する場合は unicode に戻さない（nお race 防止）。
        let obs = ProbeObservations { nc_fired: true, gji_resumed: false };
        let env = TsfEnvSnapshot { is_tsf_mode: false, gji_active: false, ..Default::default() };

        let plan = decide_transmit_plan(false, true, obs, &env, false, false, false); // deferred_empty=false

        assert!(!plan.used_eager_path, "deferred_vks あり: unicode TSF パスを使わない");
    }

    // 再現テスト: "かわんないよ → kあわんないよ"
    // Medium cold (forces_prepend_f2=true) で GJI が NameChangeWait 内の F2 に応答せず
    // nc_fired=false のままタイムアウト → forces_prepend_f2=true → should_prepend_f2=true
    // （F2 バッチ同梱で先頭 VK リテラル化を防止）
    #[test]
    fn decide_plan_medium_cold_forces_f2_in_batch_on_ncwait_timeout() {
        let obs = ProbeObservations { nc_fired: false, gji_resumed: false };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true, gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(true, false, obs, &env, true, true, false); // Medium cold

        assert!(
            plan.should_prepend_f2,
            "Medium cold + GJI 無応答: F2 をバッチに含めないと先頭 VK がリテラル化する"
        );
        assert!(plan.needs_literal, "GJI 応答未確認: LiteralDetect 有効");
    }

    // FSM 統合: Medium cold NameChangeWait タイムアウト → F2 バッチ同梱（かわんないよ 修正）
    #[test]
    fn fsm_ncwait_medium_cold_timeout_emits_f2_in_batch() {
        let mut machine = {
            let probe = TsfReadinessProbe::new(0, 0, 0);
            TsfProbeMachine::new_gji(
                "ka",
                0,
                probe,
                0,
                false,
                ColdReason::FocusChange,
                true, // prepend_f2_warmup=true (cold start)
                false,
                crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS, // Medium cold budget
                true,  // forces_prepend_f2
                false, // is_long_cold: Medium ではなく Long のみ true
                false, // fresh_f2_at_probe_start: NameChangeWait パスはプローブ開始時に F2 未送信
            )
        };
        machine.force_phase_for_test(make_namechange_wait(0, false)); // 即タイムアウト

        let env = TsfEnvSnapshot {
            is_tsf_mode: true, gji_active: true,
            ..Default::default()
        };
        let actions = machine.tick_with_clock_env(&ManualClock(1000), &env);

        if let ProbeAction::Transmit { plan, observations, target: TransmitTarget::Tsf, .. } =
            &actions[0]
        {
            assert!(
                plan.should_prepend_f2,
                "Medium cold + nc_fired=false: forces_prepend_f2=true → F2 をバッチに含める"
            );
            assert!(plan.needs_literal, "GJI 未応答: LiteralDetect 有効");
            assert!(!observations.nc_fired, "タイムアウト: nc_fired=false");
        } else {
            panic!("Transmit(Tsf) を emit するべき: {actions:?}");
        }
    }

    // 回帰確認: gji_long_idle_probe=false の通常タイムアウトは F2 なしのまま変わらない
    #[test]
    fn fsm_ncwait_short_idle_timeout_still_skips_f2() {
        let mut machine = {
            let probe = TsfReadinessProbe::new(0, 0, 0);
            TsfProbeMachine::new_gji(
                "sa",
                0,
                probe,
                0,
                false,
                ColdReason::FocusChange,
                true,
                false,
                crate::tuning::SETTLE_TIMEOUT_MS, // Short cold budget
                false, // forces_prepend_f2
                false, // is_long_cold
                false, // fresh_f2_at_probe_start
            )
        };
        machine.force_phase_for_test(make_namechange_wait(0, false)); // Short cold: 即タイムアウト

        let env = TsfEnvSnapshot {
            is_tsf_mode: true, gji_active: true,
            ..Default::default()
        };
        let actions = machine.tick_with_clock_env(&ManualClock(1000), &env);

        if let ProbeAction::Transmit { plan, .. } =
            &actions[0]
        {
            assert!(
                !plan.should_prepend_f2,
                "Short cold (forces_prepend_f2=false) + nc_fired=false: F2 をバッチに含めない (reinit race 防止)"
            );
        } else {
            panic!("Transmit(Tsf) を emit するべき: {actions:?}");
        }
    }

    // FSM 統合: NameChangeWait タイムアウト → Transmit で plan が正しく emit されること
    #[test]
    fn fsm_ncwait_timeout_tsf_mode_emits_correct_plan() {
        // Bug 1 の FSM 統合テスト: nc_fired=false + tsf_mode + !long_idle → should_prepend_f2=false
        // seつぞく バグ再発防止: nc_fired=false でも LiteralDetect を有効にして回収する。
        let mut machine = {
            let probe = TsfReadinessProbe::new(0, 0, 0);
            TsfProbeMachine::new_gji(
                "sa",
                0,
                probe,
                0,
                false,
                ColdReason::FocusChange,
                true,  // prepend_f2_warmup=true (cold start)
                false,
                crate::tuning::SETTLE_TIMEOUT_MS, // Short cold budget
                false, // forces_prepend_f2
                false, // is_long_cold
                false, // fresh_f2_at_probe_start
            )
        };
        machine.force_phase_for_test(make_namechange_wait(0, false)); // 即タイムアウト

        let env = TsfEnvSnapshot { is_tsf_mode: true, gji_active: true, ..Default::default() };
        let actions = machine.tick_with_clock_env(&ManualClock(1000), &env);

        if let ProbeAction::Transmit { plan, observations, target: TransmitTarget::Tsf, .. } = &actions[0] {
            assert!(!plan.should_prepend_f2, "nc_fired=false + tsf + !long_idle: F2 をバッチに含めない");
            assert!(plan.needs_literal, "nc_fired=false + tsf: IME 準備未確認 → LiteralDetect で保護");
            assert!(!observations.nc_fired, "タイムアウト: nc_fired=false");
        } else {
            panic!("Transmit(Tsf) を emit するべき: {actions:?}");
        }
    }
}
