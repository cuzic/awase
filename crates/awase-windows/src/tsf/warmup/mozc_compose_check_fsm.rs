//! ADR-270: Mozc で、最初のローマ字を送ったあと、入力中の文字ができたかを確かめる FSM。
//!
//! 入力中の文字ができるはずの状態（`state::mozc_shm::should_check_compose`）で、awase が最初のローマ字を VK で送った
//! 直後に張る。10ms ごとに（`TIMER_TSF_PROBE`）送った先のスレッドの共有メモリを読み、`state::mozc_shm::judge_compose` の
//! 結論に従う。
//!
//! - **入力中の文字ができた**（`phase >= 1`）→ `CompositionConfirmed` + `Done`。連続失敗の数え直しになる。
//! - **期限（`MOZC_COMPOSE_CHECK_MS`）までにできず、Mozc サーバーが応答できる状態** → リテラルと判断し、
//!   `RawTsfLiteralRecovery`（送ったローマ字の文字数だけ Backspace して再送、cold の印付け）+ `Done`。GJI の回収と同じ
//!   経路・同じ連続失敗の打ち切り（2 回目は消すだけで再送しない）を使う。
//! - **判断しない**（読めない・IME が閉じた・サーバーが応答できない）→ `Done` だけ。サーバーが応答できないときは、
//!   キーが握りつぶされた（文書に出ていない）可能性があり、Backspace で直前の文字を消すおそれがあるので消さない。
//!
//! この FSM が張られている間（`pending_tsf`）、後続のキーは保留される（`defer_vks_if_probe_or_recovery_in_flight`）。
//! 消す文字数を「送ったローマ字の文字数」と決められるのは、確認の対象を最初の 1 回の送信だけにして、その間に別の
//! キーが文書に出ないから。待ちは、正常なときは最初の打鍵から約 0.06 秒以内（実測、ADR-270）。

use crate::state::event_origin::Generation;
use crate::state::mozc_shm::{
    compose_literal_backs, judge_compose, ComposeReading, ComposeSkip, ComposeVerdict,
};
use crate::tsf::literal_facts::{
    DetectEvidence, DetectPath, DetectRoute, DetectTarget, LiteralDetectFacts, LiteralVerdict,
};
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{ProbeAction, TsfEnvSnapshot};
use crate::tsf::warmup::tickable_fsm::TickableFsm;

/// 確認の対象（送った先のスレッドとローマ字）。
#[derive(Debug, Clone)]
pub(crate) struct MozcComposeTarget {
    pub(crate) cold_seq: Generation,
    pub(crate) romaji: String,
    pub(crate) thread_id: u32,
    pub(crate) process_id: u32,
}

pub(crate) struct MozcComposeCheckFsm {
    target: MozcComposeTarget,
    /// 送信した時刻（`GetTickCount64`）
    sent_ms: u64,
    /// 送信から待つ時間（ms）
    check_ms: u64,
    /// RAII guard — drop で `OUTPUT_GATE` を戻す
    _guard: OutputActiveGuard,
}

impl MozcComposeCheckFsm {
    pub(crate) fn new(target: MozcComposeTarget, check_ms: u64) -> Self {
        Self {
            target,
            sent_ms: crate::hook::current_tick_ms(),
            check_ms,
            _guard: OutputActiveGuard::begin(),
        }
    }

    /// 結論と、それに対応する action。Win32 を呼ばない（読みは引数で受ける）。
    fn step(&self, now_ms: u64, reading: ComposeReading) -> (ComposeVerdict, Vec<ProbeAction>) {
        let elapsed_ms = now_ms.saturating_sub(self.sent_ms);
        let verdict = judge_compose(elapsed_ms, self.check_ms, reading);
        let cold_seq = self.target.cold_seq;
        let actions = match verdict {
            ComposeVerdict::Pending => Vec::new(),
            ComposeVerdict::Confirmed => vec![
                ProbeAction::CompositionConfirmed {
                    cold_seq,
                    mark_literal_session: false,
                    facts: facts(LiteralVerdict::CompositionConfirmed),
                },
                ProbeAction::Done,
            ],
            ComposeVerdict::Literal => vec![
                ProbeAction::RawTsfLiteralRecovery {
                    cold_seq,
                    backs: compose_literal_backs(&self.target.romaji),
                    romaji: self.target.romaji.clone(),
                    escape_composition: false,
                    facts: facts(LiteralVerdict::SuspectedLiteral),
                },
                ProbeAction::Done,
            ],
            ComposeVerdict::Skip(_) => vec![ProbeAction::Done],
        };
        (verdict, actions)
    }
}

/// journal に残す判定の記録。証拠（GJI の I/O 等）は無いので既定値。
fn facts(verdict: LiteralVerdict) -> LiteralDetectFacts {
    LiteralDetectFacts {
        verdict,
        route: DetectRoute::ProbeEnd,
        path: DetectPath::Word,
        target: DetectTarget::Tsf,
        vk: None,
        idx: 0,
        last_idx: 0,
        evidence: DetectEvidence::default(),
    }
}

impl TickableFsm for MozcComposeCheckFsm {
    fn tick(&mut self, _env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        let now_ms = crate::hook::current_tick_ms();
        let reading =
            crate::observer::mozc_shm::read_compose(self.target.thread_id, self.target.process_id);
        let (verdict, actions) = self.step(now_ms, reading);
        let elapsed_ms = now_ms.saturating_sub(self.sent_ms);
        match verdict {
            ComposeVerdict::Pending => {}
            ComposeVerdict::Confirmed => {
                tracing::info!(
                    "[mozc-compose] cold={} composition confirmed after {elapsed_ms}ms",
                    self.target.cold_seq.value()
                );
            }
            ComposeVerdict::Literal => {
                tracing::warn!(
                    "[mozc-compose] cold={} no composition {elapsed_ms}ms after sending {:?} (server ready) → literal; backspace ×{} + re-send",
                    self.target.cold_seq.value(),
                    self.target.romaji,
                    compose_literal_backs(&self.target.romaji)
                );
            }
            ComposeVerdict::Skip(reason) => {
                let why = match reason {
                    ComposeSkip::Unreadable => "shared memory unreadable",
                    ComposeSkip::ImeNotActive => "TIP inactive or IME closed",
                    ComposeSkip::ServerNotReady => {
                        "Mozc server not ready (key may have been swallowed; not deleting)"
                    }
                };
                tracing::info!(
                    "[mozc-compose] cold={} skipped after {elapsed_ms}ms: {why}",
                    self.target.cold_seq.value()
                );
            }
        }
        actions
    }

    fn cold_seq_hint(&self) -> Generation {
        self.target.cold_seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::mozc_shm::ComposeBlock;

    fn fsm(sent_ms: u64) -> MozcComposeCheckFsm {
        MozcComposeCheckFsm {
            target: MozcComposeTarget {
                cold_seq: Generation::INITIAL,
                romaji: "wo".to_string(),
                thread_id: 1,
                process_id: 2,
            },
            sent_ms,
            check_ms: 300,
            _guard: OutputActiveGuard::noop_for_test(),
        }
    }

    fn block(phase: u32, server_ready: bool) -> ComposeReading {
        ComposeReading::Block(ComposeBlock {
            active: true,
            effective_open: true,
            effective_mode: 1,
            phase,
            scope_forced_mode: 0,
            server_ready,
        })
    }

    #[test]
    fn pending_emits_nothing() {
        let (v, a) = fsm(1_000).step(1_100, block(0, true));
        assert_eq!(v, ComposeVerdict::Pending);
        assert!(a.is_empty());
    }

    #[test]
    fn confirmed_emits_confirmation_then_done() {
        let (v, a) = fsm(1_000).step(1_030, block(1, true));
        assert_eq!(v, ComposeVerdict::Confirmed);
        assert!(matches!(
            a.as_slice(),
            [
                ProbeAction::CompositionConfirmed {
                    mark_literal_session: false,
                    ..
                },
                ProbeAction::Done
            ]
        ));
    }

    #[test]
    fn literal_emits_recovery_with_the_romaji_length_then_done() {
        let (v, a) = fsm(1_000).step(1_300, block(0, true));
        assert_eq!(v, ComposeVerdict::Literal);
        match a.as_slice() {
            [ProbeAction::RawTsfLiteralRecovery {
                backs,
                romaji,
                escape_composition,
                facts,
                ..
            }, ProbeAction::Done] => {
                assert_eq!(*backs, 2);
                assert_eq!(romaji, "wo");
                assert!(!escape_composition);
                assert_eq!(facts.verdict, LiteralVerdict::SuspectedLiteral);
            }
            other => panic!("unexpected actions: {other:?}"),
        }
    }

    #[test]
    fn server_not_ready_never_emits_recovery() {
        // サーバーが応答できないときは、キーが握りつぶされた（文書に出ていない）可能性がある。消さない。
        let (v, a) = fsm(1_000).step(5_000, block(0, false));
        assert_eq!(v, ComposeVerdict::Skip(ComposeSkip::ServerNotReady));
        assert!(matches!(a.as_slice(), [ProbeAction::Done]));
    }

    #[test]
    fn unreadable_skips_with_done_only() {
        let (v, a) = fsm(1_000).step(1_010, ComposeReading::Unreadable);
        assert_eq!(v, ComposeVerdict::Skip(ComposeSkip::Unreadable));
        assert!(matches!(a.as_slice(), [ProbeAction::Done]));
    }
}
