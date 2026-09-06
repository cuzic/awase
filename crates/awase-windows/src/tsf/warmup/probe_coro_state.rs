//! `TsfProbeCoro` / `GjiWarmupCoro` が共有する `TickableFsm` ラッパー配線。
//!
//! コルーチン本体（`tsf_probe_coro_body` / `gji_coro_body`）は既に
//! `run_per_vk_confirm`（2026-07-17 統合）で共通化済みだったが、その周辺の
//! `tick`/`apply_transmit_done`/`apply_vk_sent`/prime のラッパー配線は
//! `TsfProbeCoro`（`probe_fsm.rs`）と `GjiWarmupCoro`（`gji_warmup_coro.rs`）に
//! byte-identical に近い形で複製されていた。本モジュールはその共通部分だけを
//! 切り出したものであり、各構造体固有の `OutputActiveGuard` 管理
//! （`TsfProbeCoro::_guard` は construction 時から drop まで固定、
//! `GjiWarmupCoro::literal_detect_guard` は LiteralDetect フェーズ突入時に
//! 遅延生成）はここに含めない（各呼び出し元で個別に保持する）。

use crate::state::event_origin::Generation;
use crate::tsf::probe::LiteralDetector;
use crate::tsf::warmup::probe_fsm::{
    ProbeAction, ProbeTickInput, TransmitDonePayload, TsfEnvSnapshot, VkSentPayload,
};
use timed_fsm::coro::{CoroStep, StepCoro};

/// `TsfProbeCoro`/`GjiWarmupCoro` 共有の `TickableFsm` 状態。
///
/// `pending_transmit_done`/`pending_vk_sent` は dispatcher からの `apply_*` 呼び出しで
/// セットされ、次の `tick()` で `ProbeTickInput` に載せて消費される（BUG-27 参照）。
pub(crate) struct ProbeCoroState {
    coro: StepCoro<ProbeTickInput, Vec<ProbeAction>>,
    pending_transmit_done: Option<TransmitDonePayload>,
    pending_vk_sent: Option<VkSentPayload>,
    cold_seq: Generation,
}

impl ProbeCoroState {
    /// `coro` を `prime()` して最初の（input を消費しない）yield を消費してから状態を組み立てる。
    ///
    /// `struct_name` は prime 失敗時の `debug_assert!` メッセージにのみ使う
    /// （どちらの呼び出し元で prime に失敗したか診断しやすくするため）。
    pub(crate) fn new(
        mut coro: StepCoro<ProbeTickInput, Vec<ProbeAction>>,
        cold_seq: Generation,
        struct_name: &str,
    ) -> Self {
        // pending_tsf に格納して外部から本物の tick を受け取り始める前に prime() で
        // 消費しておく。StepCoro の最初の step() は input を消費しないため、これを
        // せずに放置すると install 後に届く最初の tick の入力（deferred VK 等）が
        // 握り潰される。
        let primed = coro.prime();
        debug_assert!(
            matches!(&primed, CoroStep::Yielded(actions) if actions.is_empty()),
            "{struct_name} prime() は空の ProbeAction を yield するはず: {primed:?}"
        );
        Self {
            coro,
            pending_transmit_done: None,
            pending_vk_sent: None,
            cold_seq,
        }
    }

    pub(crate) fn cold_seq(&self) -> Generation {
        self.cold_seq
    }

    /// BUG-27 調査用ログ: 通常 tick（Phase 1 の 10ms ポーリング等）は毎回
    /// vk_sent/transmit_done とも None のため、どちらかが Some の場合のみ出す
    /// （毎 tick 出すと Phase 1 のポーリングだけでログが埋まる）。`log_tag` は
    /// 呼び出し元（`tsf-probe-vk-sent-trace`/`gji-coro-vk-sent-trace`）で書き分ける。
    pub(crate) fn tick(&mut self, env: TsfEnvSnapshot, log_tag: &str) -> Vec<ProbeAction> {
        if self.pending_vk_sent.is_some() || self.pending_transmit_done.is_some() {
            tracing::debug!(
                "[{log_tag}] cold={} tick consuming pending_vk_sent={} \
                 pending_transmit_done={} t={}ms",
                self.cold_seq.value(),
                self.pending_vk_sent.is_some(),
                self.pending_transmit_done.is_some(),
                crate::hook::current_tick_ms(),
            );
        }
        let input = ProbeTickInput {
            env,
            transmit_done: self.pending_transmit_done.take(),
            vk_sent: self.pending_vk_sent.take(),
        };
        match self.coro.step(input) {
            CoroStep::Yielded(actions) => actions,
            CoroStep::Complete => vec![ProbeAction::Done],
        }
    }

    /// dispatcher が `Transmit` を実行した後に呼ぶ。
    ///
    /// `detector` が `Some` なら `LiteralDetect` フェーズへ進む（`false` を返す）。
    /// `None` なら Done（`true` を返す）。呼び出し元が `OutputActiveGuard` を追加で
    /// 確保する必要がある場合は、戻り値 `false`（＝ `detector` が `Some` だった）を
    /// 見て呼び出し元側で行う。
    pub(crate) fn apply_transmit_done(
        &mut self,
        romaji: String,
        ze_bs_count: usize,
        detector: Option<LiteralDetector>,
        literal_detect_ms: u64,
    ) -> bool {
        match detector {
            Some(det) => {
                let deadline_ms = crate::hook::current_tick_ms() + literal_detect_ms;
                self.pending_transmit_done = Some(TransmitDonePayload {
                    romaji,
                    ze_bs_count,
                    detector: det,
                    deadline_ms,
                });
                false
            }
            None => true,
        }
    }

    /// per-VK confirm が1 VK 送信するたびに呼ぶ。`log_tag` は呼び出し元で書き分ける。
    pub(crate) fn apply_vk_sent(
        &mut self,
        detector: LiteralDetector,
        deadline_ms: u64,
        log_tag: &str,
    ) {
        // BUG-27 調査用ログ: overwritten=true なら、前回の apply_vk_sent が
        // まだ tick() に消費されないまま次の apply_vk_sent が来ている
        // （＝1 tick 内で TransmitSingleVk が2回ディスパッチされた等の異常）。
        let overwritten = self.pending_vk_sent.is_some();
        tracing::debug!(
            "[{log_tag}] cold={} apply_vk_sent SET deadline_ms={deadline_ms} \
             overwritten_unconsumed={overwritten} t={}ms",
            self.cold_seq.value(),
            crate::hook::current_tick_ms(),
        );
        self.pending_vk_sent = Some(VkSentPayload {
            detector,
            deadline_ms,
        });
    }
}
