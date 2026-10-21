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
//! - **判断しない**（読めない・IME が閉じた・サーバーが応答できない・宛先の窓が変わった・対象のスレッドが止まっている）→
//!   `Done` だけ。サーバーが応答できないとき、または対象のスレッドが IME の同期呼び出しなどで止まっているときは、キーが
//!   握りつぶされた・まだ処理されていない可能性があり、Backspace で直前の文字を消すおそれがあるので消さない。
//!
//! この FSM が張られている間（`pending_tsf`）、後続のキーは保留される（`defer_vks_if_probe_or_recovery_in_flight`）。
//! 消す文字数を「送ったローマ字の文字数」と決められるのは、確認の対象を最初の 1 回の送信だけにして、その間に別の
//! キーが文書に出ないから。待ちは、正常なときは最初の打鍵から約 0.06 秒以内（実測、ADR-270）。

use crate::state::event_origin::Generation;
use crate::state::mozc_shm::{
    compose_literal_backs, judge_compose, ComposeReading, ComposeSkip, ComposeVerdict,
};
use crate::state::raw_recovery_plan::{plan_raw_recovery, RawRecoveryDisposition, StageOrigin};
use crate::tsf::literal_facts::{
    DetectEvidence, DetectPath, DetectRoute, DetectTarget, LiteralDetectFacts, LiteralVerdict,
};
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{ProbeAction, TsfEnvSnapshot};
use crate::tsf::warmup::tickable_fsm::TickableFsm;

/// 確認のあと、`key_seq` の進みをもう一度読むまでの時間（ms）。観測専用（判断には使わない）。
const COMPOSE_SETTLE_RECHECK_MS: u32 = 30;

/// 確認の対象（送った先のスレッドとローマ字）。
#[derive(Debug, Clone)]
pub(crate) struct MozcComposeTarget {
    pub(crate) cold_seq: Generation,
    pub(crate) romaji: String,
    /// 送った先のフォーカスの窓（hwnd のアドレス）。止まっていないかの確認に使う。
    pub(crate) hwnd_addr: usize,
    pub(crate) thread_id: u32,
    pub(crate) process_id: u32,
    /// 送信の直前の `key_seq`（ログ用。リテラルと判断した件で、TIP がキーを見たかを後から区別する）。
    pub(crate) key_seq_before: u32,
}

pub(crate) struct MozcComposeCheckFsm {
    target: MozcComposeTarget,
    /// 終わる段の宛先（最初の送信より前に採った値）。宛先の窓が変わったら確認も回収もやめる。
    origin: Option<StageOrigin>,
    /// 送信した時刻（`GetTickCount64`）
    sent_ms: u64,
    /// 送信から待つ時間（ms）
    check_ms: u64,
    /// RAII guard — drop で `OUTPUT_GATE` を戻す
    _guard: OutputActiveGuard,
}

impl MozcComposeCheckFsm {
    pub(crate) fn new(
        target: MozcComposeTarget,
        check_ms: u64,
        origin: Option<StageOrigin>,
    ) -> Self {
        Self {
            target,
            origin,
            sent_ms: crate::clock::current_tick_ms(),
            check_ms,
            _guard: OutputActiveGuard::begin(),
        }
    }

    /// 宛先の窓が変わったか（段の宛先と、この tick の宛先から、既存の回収と同じ判断）。
    fn target_moved(&self, now: Option<StageOrigin>) -> bool {
        match (self.origin, now) {
            (Some(_), Some(now)) => {
                plan_raw_recovery(self.origin, now) == RawRecoveryDisposition::DiscardStale
            }
            _ => false,
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
    fn tick(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        let now_ms = crate::clock::current_tick_ms();
        let elapsed_ms = now_ms.saturating_sub(self.sent_ms);
        // 宛先の窓が変わったら、確認も回収もやめる（変わった窓へ Backspace を送らない）。
        if self.target_moved(env.stage_origin_now) {
            tracing::info!(
                "[mozc-compose] cold={} skipped after {elapsed_ms}ms: target window changed",
                self.target.cold_seq.value()
            );
            return vec![ProbeAction::Done];
        }
        let reading = crate::tsf::read_mozc_compose(self.target.thread_id, self.target.process_id);
        let (verdict, mut actions) = self.step(now_ms, reading);
        let (key_seq, composition_key_seq) = match reading {
            ComposeReading::Block(b) => (b.key_seq, b.composition_key_seq),
            ComposeReading::Unreadable => (0, 0),
        };
        match verdict {
            ComposeVerdict::Pending => {}
            ComposeVerdict::Confirmed => {
                // 観測だけ（判断は変えない）。送った文字数より `key_seq` の進みが少なければ、先頭の文字を TIP が見ていない
                // （例: 「dえ」のように 1 文字目だけ文書へ素通りし、2 文字目から入力中になった）疑いがある。
                tracing::info!(
                    "[mozc-compose] cold={} composition confirmed after {elapsed_ms}ms (sent {} chars, key_seq {}→{}, composition_key_seq {composition_key_seq})",
                    self.target.cold_seq.value(),
                    compose_literal_backs(&self.target.romaji),
                    self.target.key_seq_before,
                    key_seq
                );
                // 確認は入力中の文字ができた時点（最短 16ms）で終わるので、2 打目がまだ数えられていないだけかもしれない
                // （確認済み 21 回のうち 2 回が送った文字数より 1 少なかった、2026-10-21）。少し置いてもう一度読み、
                // 足りないままか（本物の素通り）、揃うか（読む時点の競合）をログに残す。判断には使わない。
                let cold = self.target.cold_seq.value();
                let (thread_id, process_id) = (self.target.thread_id, self.target.process_id);
                let (before, sent) = (
                    self.target.key_seq_before,
                    compose_literal_backs(&self.target.romaji),
                );
                win32_async::spawn_local(async move {
                    win32_async::sleep_ms(COMPOSE_SETTLE_RECHECK_MS).await;
                    if let ComposeReading::Block(b) =
                        crate::tsf::read_mozc_compose(thread_id, process_id)
                    {
                        // `settled` が送った文字数より多いのは、読み直すまでに次のキーが送られたとき（比較に使えない）。
                        tracing::info!(
                            "[mozc-compose] cold={cold} settled +{COMPOSE_SETTLE_RECHECK_MS}ms: sent {sent} chars, key_seq delta at confirm {}, settled {}, composition_key_seq {}",
                            key_seq.wrapping_sub(before),
                            b.key_seq.wrapping_sub(before),
                            b.composition_key_seq.wrapping_sub(before)
                        );
                    }
                });
            }
            ComposeVerdict::Literal => {
                // 対象のスレッドが IME の同期呼び出し（Mozc サーバーの応答待ち）などで止まっていると、キーはまだ処理されて
                // いない。その状態で Backspace を送ると、後から入力中の文字ができ、最初の Backspace がそれを消し、
                // 2 つ目が文書の直前の正しい文字を消す。止まっているなら消さない。
                let response_check_ms =
                    u32::try_from(crate::tuning::MOZC_COMPOSE_RESPONSE_CHECK_MS)
                        .unwrap_or(u32::MAX);
                if crate::tsf::window_thread_responds(self.target.hwnd_addr, response_check_ms) {
                    tracing::warn!(
                        "[mozc-compose] cold={} no composition {elapsed_ms}ms after sending {:?} (server ready) → literal; backspace ×{} + re-send (key_seq {}→{}, composition_key_seq {composition_key_seq})",
                        self.target.cold_seq.value(),
                        self.target.romaji,
                        compose_literal_backs(&self.target.romaji),
                        self.target.key_seq_before,
                        key_seq
                    );
                } else {
                    tracing::info!(
                        "[mozc-compose] cold={} skipped after {elapsed_ms}ms: target thread not responding (may be blocked in the IME call; not deleting) (key_seq {}→{})",
                        self.target.cold_seq.value(),
                        self.target.key_seq_before,
                        key_seq
                    );
                    actions = vec![ProbeAction::Done];
                }
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
                hwnd_addr: 3,
                thread_id: 1,
                process_id: 2,
                key_seq_before: 0,
            },
            origin: None,
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
            key_seq: 0,
            composition_key_seq: 0,
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

    fn origin(pid: u32, hwnd: isize) -> StageOrigin {
        StageOrigin {
            focus_gen: crate::state::focus_gen::FocusGen::INITIAL,
            foreground: crate::state::foreground_scope::ForegroundScope { pid, hwnd },
        }
    }

    #[test]
    fn target_moved_follows_the_existing_recovery_plan() {
        let mut f = fsm(1_000);
        // 宛先を採っていなければ判断しない（動いていないとみなす）
        assert!(!f.target_moved(Some(origin(1, 10))));
        f.origin = Some(origin(1, 10));
        // 同じ前景窓なら動いていない
        assert!(!f.target_moved(Some(origin(1, 10))));
        // この tick の宛先を採れていなければ判断しない
        assert!(!f.target_moved(None));
        // 前景窓が変わったら動いた（既存の `plan_raw_recovery` と同じ判断）
        assert!(f.target_moved(Some(origin(2, 20))));
    }
}
