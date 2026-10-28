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
//! - **入力中の文字ができたが、`key_seq` の進みが送った文字数より少ない**（先頭の文字を TIP が見ていない: ADR-284、BUG-225）→
//!   毎 tick 読み直し、揃えば（2 打目の処理が遅いだけだった）通常の確認。期限（送信から `MOZC_COMPOSE_CHECK_MS`）まで
//!   足りないままで、スレッドが応答し、もう一度読んでも足りなければ、足りない分の文字が文書へリテラルで届いたと判断する。
//!   `RawTsfLiteralRecovery`（Escape で入力中の文字を捨て、足りない文字数だけ Backspace して全体を再送、cold の印付け）+ `Done`。
//!   待つ間も後続のキーは保留されるので、進みは他のキーで増えない。
//! - **判断しない**（読めない・IME が閉じた・サーバーが応答できない・宛先の窓が変わった・対象のスレッドが止まっている）→
//!   `Done` だけ。サーバーが応答できないとき、または対象のスレッドが IME の同期呼び出しなどで止まっているときは、キーが
//!   握りつぶされた・まだ処理されていない可能性があり、Backspace で直前の文字を消すおそれがあるので消さない。
//!
//! **per-VK（ADR-285）**: 張るときの最初の送信は、ローマ字の**最初の 1 キーだけ**（残りは `MozcComposeTarget::rest`）。
//! 確認できたら（判断しない場合も）`TransmitRest` で残りを送る。リテラルなら素通りしているのは最初の 1 キーだけなので
//! Backspace は 1 回で、全体を送り直す。ADR-284 の `key_seq` による部分リテラルの検出は、最初の 1 キーを確かめたあとの
//! 残りには起きないはずなので安全網として残す（実機で効かなければ撤去する）。
//!
//! この FSM が張られている間（`pending_tsf`）、後続のキーは保留される（`defer_vks_if_probe_or_recovery_in_flight`）。
//! 消す文字数を「送ったローマ字の文字数」と決められるのは、確認の対象を最初の 1 回の送信だけにして、その間に別の
//! キーが文書に出ないから。待ちは、正常なときは最初の打鍵から約 0.06 秒以内（実測、ADR-270）。

use crate::state::event_origin::Generation;
use crate::state::mozc_shm::{
    compose_literal_backs, judge_compose, judge_settled, ComposeReading, ComposeSkip,
    ComposeVerdict, SettleVerdict,
};
use crate::state::raw_recovery_plan::{plan_raw_recovery, RawRecoveryDisposition, StageOrigin};
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
    /// 実際に送ったローマ字。最初の 1 キーだけ送ったとき（ADR-285）はその 1 文字、そうでなければ全体。
    pub(crate) romaji: String,
    /// 最初の 1 キーだけ送ったときの、まだ送っていない残り（空なら全体を送った）。確認が終わったら `TransmitRest` で送る。
    pub(crate) rest: String,
    /// 送った先のフォーカスの窓（hwnd のアドレス）。止まっていないかの確認に使う。
    pub(crate) hwnd_addr: usize,
    pub(crate) thread_id: u32,
    pub(crate) process_id: u32,
    /// 送信の直前の `key_seq`（ログ用。リテラルと判断した件で、TIP がキーを見たかを後から区別する）。
    pub(crate) key_seq_before: u32,
}

impl MozcComposeTarget {
    /// 再送するローマ字の全体（送った分 + 残り）。リテラルの回収は、送った分だけ Backspace して全体を送り直す。
    fn full_romaji(&self) -> String {
        format!("{}{}", self.romaji, self.rest)
    }
}

pub(crate) struct MozcComposeCheckFsm {
    target: MozcComposeTarget,
    /// 終わる段の宛先（最初の送信より前に採った値）。宛先の窓が変わったら確認も回収もやめる。
    origin: Option<StageOrigin>,
    /// 送信した時刻（`GetTickCount64`）
    sent_ms: u64,
    /// 送信から待つ時間（ms）
    check_ms: u64,
    /// 確認はできたが `key_seq` の進みが足りなかった（一部のキーを TIP が見ていない疑い）。`true` の間は `check_ms`
    /// （送信から）まで毎 tick 読み直して結論を出す（それまで `Done` にしない）。
    settling: bool,
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
            settling: false,
            _guard: OutputActiveGuard::begin(),
        }
    }

    /// 診断: TIP がキーを受け取る側になった時刻などを、送信の時刻からの差で出す（mozc-readable の version 5）。
    fn log_focus_ticks(&self) {
        if let Some(ticks) =
            crate::tsf::read_mozc_focus_ticks(self.target.thread_id, self.target.process_id)
        {
            tracing::info!(
                "[mozc-focus] cold={} relative to the send: {}",
                self.target.cold_seq.value(),
                ticks.describe_relative_to(self.sent_ms)
            );
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
                    romaji: self.target.full_romaji(),
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

impl MozcComposeCheckFsm {
    /// 確認の後の読み直しの結論と action。足りないままで期限（送信から `check_ms`）前なら `None`（まだ待つ）。
    /// 揃っていれば期限を待たずに通常の確認。Win32 を呼ばない。
    fn settle_step(
        &self,
        now_ms: u64,
        reading: ComposeReading,
    ) -> Option<(SettleVerdict, Vec<ProbeAction>)> {
        let cold_seq = self.target.cold_seq;
        let sent = compose_literal_backs(&self.target.romaji);
        let verdict = match reading {
            // 入力中の文字が消えている（phase 0）・IME が閉じた・読めないときは、足りない分の行き先が分からない。判断しない。
            ComposeReading::Block(b) if b.active && b.effective_open && b.phase >= 1 => {
                judge_settled(sent, b.key_seq.wrapping_sub(self.target.key_seq_before))
            }
            _ => SettleVerdict::Skip,
        };
        // 本物の素通りは何 ms 待っても進みが増えないが、2 打目の処理が遅いだけなら待てば揃う。破壊的な回収は期限まで待つ。
        if matches!(verdict, SettleVerdict::PartialLiteral { .. })
            && now_ms.saturating_sub(self.sent_ms) < self.check_ms
        {
            return None;
        }
        let actions = match verdict {
            SettleVerdict::Complete => vec![
                ProbeAction::CompositionConfirmed {
                    cold_seq,
                    mark_literal_session: false,
                    facts: facts(LiteralVerdict::CompositionConfirmed),
                },
                ProbeAction::Done,
            ],
            // Escape で入力中の文字（TIP が見た分）を捨て、Backspace で文書に出た分（足りない文字数）を消し、全体を再送する。
            // 先頭の文字を落とす型（`ko` の `k`）を想定する。入力中の文字の長さはローマ字の長さと一致しない
            // （`o`→`お`）ので、Backspace で入力中の文字を消そうとせず Escape に任せる。
            // `facts.idx` を最後の文字にするのは、連続 2 回目の諦める分岐が「疑いのあった文字より後ろ」（`romaji_after`）を
            // 送るため。全体を 1 回で送る Mozc では後ろの文字は TIP が受け取り済みで、送ると `kおお` になる（Opus M3）。
            SettleVerdict::PartialLiteral { missing } => vec![
                ProbeAction::RawTsfLiteralRecovery {
                    cold_seq,
                    backs: missing,
                    romaji: self.target.full_romaji(),
                    escape_composition: true,
                    facts: facts_at(
                        LiteralVerdict::SuspectedLiteral,
                        u16::try_from(sent.saturating_sub(1)).unwrap_or(u16::MAX),
                    ),
                },
                ProbeAction::Done,
            ],
            SettleVerdict::Skip => vec![ProbeAction::Done],
        };
        Some((verdict, actions))
    }

    /// 読み直しの待ちの tick。結論が出たら action を返す（`PartialLiteral` は応答できるスレッドで、読み直しても足りない
    /// ときだけ回収する）。
    fn tick_settled(&self, now_ms: u64, reading: ComposeReading) -> Vec<ProbeAction> {
        let Some((verdict, actions)) = self.settle_step(now_ms, reading) else {
            return Vec::new();
        };
        let cold = self.target.cold_seq.value();
        let before = self.target.key_seq_before;
        let sent = compose_literal_backs(&self.target.romaji);
        let delta_of = |r: ComposeReading| match r {
            ComposeReading::Block(b) => b.key_seq.wrapping_sub(before),
            ComposeReading::Unreadable => 0,
        };
        match verdict {
            SettleVerdict::Complete => {
                tracing::info!(
                    "[mozc-compose] cold={cold} settled: sent {sent} chars, key_seq delta {} (complete, the 2nd key was only late)",
                    delta_of(reading)
                );
                actions
            }
            SettleVerdict::PartialLiteral { .. } => {
                // 対象のスレッドが止まっていると、キーはまだ処理されていないだけかもしれない。Backspace で
                // 直前の正しい文字を消すおそれがあるので、止まっているなら消さない（`Literal` と同じ）。
                let response_check_ms =
                    u32::try_from(crate::tuning::MOZC_COMPOSE_RESPONSE_CHECK_MS)
                        .unwrap_or(u32::MAX);
                if !crate::tsf::window_thread_responds(self.target.hwnd_addr, response_check_ms) {
                    tracing::info!(
                        "[mozc-compose] cold={cold} partial literal skipped: target thread not responding (not deleting) (key_seq delta {})",
                        delta_of(reading)
                    );
                    return vec![ProbeAction::Done];
                }
                // 応答の確認に時間がかかった間に、遅れていた 2 打目が処理されたかもしれない。確認のあとの読みで決め直す
                // （古い読みで回収すると、直前の正しい文字を Backspace で消す。Opus M1）。
                let again =
                    crate::tsf::read_mozc_compose(self.target.thread_id, self.target.process_id);
                let Some((verdict2, actions2)) = self.settle_step(now_ms, again) else {
                    return Vec::new();
                };
                if let SettleVerdict::PartialLiteral { missing } = verdict2 {
                    tracing::warn!(
                        "[mozc-compose] cold={cold} partial literal: sent {:?} ({sent} chars) but the TIP saw {} → escape + backspace ×{missing} + re-send",
                        self.target.romaji,
                        delta_of(again)
                    );
                } else {
                    tracing::info!(
                        "[mozc-compose] cold={cold} partial literal withdrawn after the response check ({verdict2:?}, key_seq delta {})",
                        delta_of(again)
                    );
                }
                actions2
            }
            SettleVerdict::Skip => {
                tracing::info!(
                    "[mozc-compose] cold={cold} settle skipped (no composition, IME closed or unreadable; key_seq delta {})",
                    delta_of(reading)
                );
                actions
            }
        }
    }
}

/// journal に残す判定の記録。証拠（GJI の I/O 等）は無いので既定値。
fn facts(verdict: LiteralVerdict) -> LiteralDetectFacts {
    facts_at(verdict, 0)
}

/// `idx`（疑いが出たキーの位置）を指定する版。連続 2 回目の諦める分岐が `romaji_after(idx)` で後ろの文字を送るので、
/// 部分リテラルでは最後の文字の位置にして何も送らせない。
fn facts_at(verdict: LiteralVerdict, idx: u16) -> LiteralDetectFacts {
    LiteralDetectFacts {
        verdict,
        route: DetectRoute::ProbeEnd,
        path: DetectPath::Word,
        target: DetectTarget::Tsf,
        vk: None,
        idx,
        last_idx: idx,
        evidence: DetectEvidence::default(),
    }
}

impl MozcComposeCheckFsm {
    /// 段を終える action（`Done`）を、残りの送信（`TransmitRest`）に置き換える（ADR-285）。
    ///
    /// 最初の 1 キーだけ送ったとき、確認が「確認できた」でも「判断しない」でも、残りは必ず送る。リテラルの回収
    /// （`RawTsfLiteralRecovery`）のときは、回収が全体を送り直すので、残りは送らない（二重にしない）。
    /// まだ待つ（`Done` が無い）ときは何もしない。
    fn attach_rest(&self, mut actions: Vec<ProbeAction>) -> Vec<ProbeAction> {
        if self.target.rest.is_empty()
            || actions
                .iter()
                .any(|a| matches!(a, ProbeAction::RawTsfLiteralRecovery { .. }))
        {
            return actions;
        }
        if matches!(actions.last(), Some(ProbeAction::Done)) {
            actions.pop();
            actions.push(ProbeAction::TransmitRest {
                romaji: self.target.rest.clone(),
            });
        }
        actions
    }
}

impl TickableFsm for MozcComposeCheckFsm {
    fn tick(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.tick_inner(env)
    }

    fn cold_seq_hint(&self) -> Generation {
        self.target.cold_seq
    }
}

impl MozcComposeCheckFsm {
    fn tick_inner(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        let now_ms = crate::clock::current_tick_ms();
        let elapsed_ms = now_ms.saturating_sub(self.sent_ms);
        // 宛先の窓が変わったら、確認も回収もやめる（変わった窓へ Backspace を送らない）。
        if self.target_moved(env.stage_origin_now) {
            tracing::info!(
                "[mozc-compose] cold={} skipped after {elapsed_ms}ms: target window changed",
                self.target.cold_seq.value()
            );
            // 残りは送らない: 変わった窓へ文字を送らない（最初の 1 キーは元の窓に入っている）。
            return vec![ProbeAction::Done];
        }
        let reading = crate::tsf::read_mozc_compose(self.target.thread_id, self.target.process_id);
        if self.settling {
            let actions = self.tick_settled(now_ms, reading);
            return self.attach_rest(actions);
        }
        let (verdict, mut actions) = self.step(now_ms, reading);
        let (key_seq, composition_key_seq) = match reading {
            ComposeReading::Block(b) => (b.key_seq, b.composition_key_seq),
            ComposeReading::Unreadable => (0, 0),
        };
        // 診断（判断には使わない）: 確認の結論が出たとき、キーボードフォーカスの時刻を送信の時刻と並べて出す。
        if matches!(verdict, ComposeVerdict::Confirmed | ComposeVerdict::Literal) {
            self.log_focus_ticks();
        }
        match verdict {
            ComposeVerdict::Pending => {}
            ComposeVerdict::Confirmed => {
                // 送った文字数より `key_seq` の進みが少なければ、先頭の文字を TIP が見ていない疑いがある（例: `ko` の `k`
                // だけ文書へ素通りし、`o` から入力中になった、BUG-225）。
                tracing::info!(
                    "[mozc-compose] cold={} composition confirmed after {elapsed_ms}ms (sent {} chars, key_seq {}→{}, composition_key_seq {composition_key_seq})",
                    self.target.cold_seq.value(),
                    compose_literal_backs(&self.target.romaji),
                    self.target.key_seq_before,
                    key_seq
                );
                // 確認は入力中の文字ができた時点（最短 16ms）で終わるので、2 打目がまだ数えられていないだけかもしれない。
                // 一部のキーを見ていない疑い（進みが 1 以上で足りない）なら、毎 tick 読み直して結論を出す（それまで `Done`
                // にしない）。進みが 0 の矛盾した読みは待たず、従来どおり確認として終える。
                let sent = compose_literal_backs(&self.target.romaji);
                if matches!(
                    judge_settled(sent, key_seq.wrapping_sub(self.target.key_seq_before)),
                    SettleVerdict::PartialLiteral { .. }
                ) {
                    self.settling = true;
                    actions = Vec::new();
                }
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
        self.attach_rest(actions)
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
                rest: String::new(),
            },
            origin: None,
            sent_ms,
            check_ms: 300,
            settling: false,
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

    fn block_seq(phase: u32, key_seq: u32) -> ComposeReading {
        ComposeReading::Block(ComposeBlock {
            active: true,
            effective_open: true,
            effective_mode: 1,
            phase,
            scope_forced_mode: 0,
            server_ready: true,
            key_seq,
            composition_key_seq: key_seq,
        })
    }

    fn fsm_with_rest(rest: &str) -> MozcComposeCheckFsm {
        let mut f = fsm(1_000);
        f.target.romaji = "k".to_string();
        f.target.rest = rest.to_string();
        f
    }

    /// ADR-285: 最初の 1 キーだけ送ったとき、確認できても判断しなくても、`Done` は残りの送信に変わる。
    #[test]
    fn attach_rest_turns_done_into_transmit_rest() {
        let f = fsm_with_rest("o");
        for before in [
            vec![
                ProbeAction::CompositionConfirmed {
                    cold_seq: Generation::INITIAL,
                    mark_literal_session: false,
                    facts: facts(LiteralVerdict::CompositionConfirmed),
                },
                ProbeAction::Done,
            ],
            vec![ProbeAction::Done],
        ] {
            let after = f.attach_rest(before);
            assert!(
                matches!(after.last(), Some(ProbeAction::TransmitRest { romaji }) if romaji == "o"),
                "{after:?}"
            );
            assert!(!after.iter().any(|a| matches!(a, ProbeAction::Done)));
        }
    }

    /// 回収（`RawTsfLiteralRecovery`）は全体を送り直すので、残りは送らない。まだ待つ間（`Done` なし）も何もしない。
    #[test]
    fn attach_rest_leaves_recovery_and_pending_alone() {
        let f = fsm_with_rest("o");
        assert!(f.attach_rest(Vec::new()).is_empty());
        let (_, actions) = f.step(1_300, block(0, true));
        let after = f.attach_rest(actions);
        assert!(matches!(
            after.as_slice(),
            [ProbeAction::RawTsfLiteralRecovery { .. }, ProbeAction::Done]
        ));
    }

    /// 最初の 1 キー（`k`）がリテラルなら、素通りは 1 文字: Backspace ×1 + 全体（`ko`）の再送。
    #[test]
    fn literal_of_the_first_key_backs_once_and_resends_the_whole_romaji() {
        let f = fsm_with_rest("o");
        let (verdict, actions) = f.step(1_300, block(0, true));
        assert_eq!(verdict, ComposeVerdict::Literal);
        match actions.as_slice() {
            [ProbeAction::RawTsfLiteralRecovery { backs, romaji, .. }, ProbeAction::Done] => {
                assert_eq!(*backs, 1);
                assert_eq!(romaji, "ko");
            }
            other => panic!("unexpected actions: {other:?}"),
        }
    }

    /// 残りが無い（全体を送った）ときは何も変えない。
    #[test]
    fn attach_rest_is_a_no_op_without_a_rest() {
        let f = fsm(1_000);
        let after = f.attach_rest(vec![ProbeAction::Done]);
        assert!(matches!(after.as_slice(), [ProbeAction::Done]));
    }

    #[test]
    fn settle_waits_for_the_deadline_while_the_progress_is_short() {
        // sent_ms=1000, check_ms=300: 足りないままの間は期限（1300）まで回収しない
        let f = fsm(1_000);
        assert!(f.settle_step(1_020, block_seq(1, 1)).is_none());
        assert!(f.settle_step(1_299, block_seq(1, 1)).is_none());
        assert!(f.settle_step(1_300, block_seq(1, 1)).is_some());
    }

    #[test]
    fn settle_complete_as_soon_as_the_second_key_is_counted() {
        // 確認の時点では 1 だったが、2 打目が遅れて数えられた → 期限を待たず通常の確認
        let (v, a) = fsm(1_000).settle_step(1_020, block_seq(1, 2)).unwrap();
        assert_eq!(v, SettleVerdict::Complete);
        assert!(matches!(
            a.as_slice(),
            [ProbeAction::CompositionConfirmed { .. }, ProbeAction::Done]
        ));
    }

    #[test]
    fn settle_partial_literal_escapes_and_backs_the_missing_chars_then_resends() {
        // BUG-225（2026-10-28）: 2 文字（フィクスチャは `wo`、実機は `ko`）を送って TIP が 1 つしか見ていない
        // → 足りない 1 文字を消して全体を再送
        let (v, a) = fsm(1_000).settle_step(1_300, block_seq(1, 1)).unwrap();
        assert_eq!(v, SettleVerdict::PartialLiteral { missing: 1 });
        match a.as_slice() {
            [ProbeAction::RawTsfLiteralRecovery {
                backs,
                romaji,
                escape_composition,
                facts,
                ..
            }, ProbeAction::Done] => {
                assert_eq!(*backs, 1);
                assert_eq!(romaji, "wo");
                assert!(*escape_composition);
                assert_eq!(facts.verdict, LiteralVerdict::SuspectedLiteral);
                // 諦める分岐の `romaji_after(romaji, idx)` が空になる位置（最後の文字）。`kおお` を防ぐ（Opus M3）
                assert_eq!(facts.idx, 1);
            }
            other => panic!("unexpected actions: {other:?}"),
        }
    }

    #[test]
    fn settle_never_recovers_when_the_composition_is_gone_or_nothing_was_counted() {
        // 入力中の文字が消えている（phase 0）・キーが 1 つも数えられていない・読めない → 消さない（期限を待たず終える）
        for reading in [block_seq(0, 1), block_seq(1, 0), ComposeReading::Unreadable] {
            let (v, a) = fsm(1_000).settle_step(1_020, reading).unwrap();
            assert_eq!(v, SettleVerdict::Skip);
            assert!(matches!(a.as_slice(), [ProbeAction::Done]));
        }
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
