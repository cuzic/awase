//! `TsfProbeCoro` の `TickableFsm` ラッパー配線（`tick`/`apply_vk_sent`/prime）。
//!
//! もとは `TsfProbeCoro`（`probe_fsm.rs`）と Tsf 注入モード用の `GjiWarmupCoro` が共有していた
//! 部分を切り出したもの。`GjiWarmupCoro` は Tsf 注入経路の撤去（2026-10-22）で削除したため、
//! 使い手は `TsfProbeCoro` だけになった。`OutputActiveGuard` の管理（construction 時から drop まで
//! 固定）は `TsfProbeCoro` 側が持つ。

use crate::state::event_origin::Generation;
use crate::tsf::probe::LiteralDetector;
use crate::tsf::warmup::probe_fsm::{ProbeAction, ProbeTickInput, TsfEnvSnapshot, VkSentPayload};
use timed_fsm::coro::{CoroStep, StepCoro};

/// `TsfProbeCoro` の `TickableFsm` 状態。
///
/// `pending_vk_sent` は dispatcher からの `apply_vk_sent` 呼び出しで
/// セットされ、次の `tick()` で `ProbeTickInput` に載せて消費される（BUG-27 参照）。
pub(crate) struct ProbeCoroState {
    coro: StepCoro<ProbeTickInput, Vec<ProbeAction>>,
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
            pending_vk_sent: None,
            cold_seq,
        }
    }

    pub(crate) fn cold_seq(&self) -> Generation {
        self.cold_seq
    }

    /// BUG-27 調査用ログ: 通常 tick（Phase 1 の 10ms ポーリング等）は毎回
    /// vk_sent が None のため、Some の場合のみ出す
    /// （毎 tick 出すと Phase 1 のポーリングだけでログが埋まる）。`log_tag` は
    /// 呼び出し元（`tsf-probe-vk-sent-trace`/`gji-coro-vk-sent-trace`）で書き分ける。
    pub(crate) fn tick(&mut self, env: TsfEnvSnapshot, log_tag: &str) -> Vec<ProbeAction> {
        if self.pending_vk_sent.is_some() {
            tracing::debug!(
                "[{log_tag}] cold={} tick consuming pending_vk_sent={} t={}ms",
                self.cold_seq.value(),
                self.pending_vk_sent.is_some(),
                crate::clock::current_tick_ms(),
            );
        }
        let input = ProbeTickInput {
            env,
            vk_sent: self.pending_vk_sent.take(),
        };
        match self.coro.step(input) {
            CoroStep::Yielded(actions) => actions,
            CoroStep::Complete => vec![ProbeAction::Done],
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
            crate::clock::current_tick_ms(),
        );
        self.pending_vk_sent = Some(VkSentPayload {
            detector,
            deadline_ms,
        });
    }
}
