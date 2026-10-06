//! IME 状態の読み取り方針の決定(FCIS F1、`runtime/ime_refresh.rs::ir_decide_read_strategy` の核)。
//!
//! 元の関数は「OS・グローバルを読む」ことと「どう読むかを決める」ことが 1 つの手続きに混ざっていた。
//! ここでは、読み取り(`observe`、`runtime/ime_refresh.rs`)が作った所有型の [`ReadStrategyFacts`] だけを受け、
//! 決定と理由([`ReadDecision`])を返す純粋関数に切り出した。OS・時計・グローバルに触れない。
//!
//! 挙動は元の関数と同じ: 最後のキー活動から [`TYPING_IDLE_MS`] 以内（打鍵中）は、明示的な IME 操作の
//! 検証を要するときを除き読まない。Shift 変換の安全網中も読まない。それ以外は、IMM を問い合わせられない
//! アプリなら `Blacklist`、そうでなければ `OsPoll`。

use crate::tuning::TYPING_IDLE_MS;

/// IME 読み取り方針の決定結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImeReadStrategy {
    /// タイピング中 — IMM/TSF を一切呼ばない
    SkipTyping,
    /// 既知ブラックリストクラス — shadow SSOT のみ使う
    Blacklist,
    /// OS をポーリングする通常パス
    OsPoll,
}

/// 決定の理由(なぜその方針になったか。journal・ログに載せる)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReadReason {
    /// 打鍵中で、明示的な IME 操作の検証も要らない。
    TypingActive,
    /// Shift 変換の安全網のブリップ中、または左 Shift 単独タップの半角英数持続トグル中。
    ShiftConvGuard,
    /// IMM を問い合わせられないアプリ(TsfNative/Blacklist)。
    ImmQuerySkipped,
    /// 通常の OS ポーリング。
    OsPoll,
}

/// 決定の結果と理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReadDecision {
    pub strategy: ImeReadStrategy,
    pub reason: ReadReason,
    /// 打鍵中だが、明示的な IME 操作の検証のために打鍵アイドルガードを迂回した。
    pub typing_guard_bypassed: bool,
}

/// `decide_read_strategy` が参照する事実(`observe` が 1 回の決定につき 1 度だけ読んで作る)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReadStrategyFacts {
    /// 最後のキー活動(物理キー押下または VK/TSF 出力)からの経過 ms。
    pub idle_ms: u64,
    /// このアプリでは IMM を問い合わせない(`FocusInfo::skip_imm_query`)。
    pub skip_imm_query: bool,
    /// 通過マーク(ADR-187)が有効。打鍵中のときだけ読む(読むとフォアグラウンド変更で失効する副作用があるため、
    /// 元の関数が読んでいた条件と同じ)。打鍵中でなければ `false` で、使われない。
    pub mode_key_pass_live: bool,
    /// 明示的な意図(`explicit_intent`)が存在する。
    pub explicit_intent_present: bool,
    /// 適用済み状態(`applied`)が `Unknown` ではない。
    pub applied_known: bool,
    /// Shift 変換の安全網のブリップ中、または半角英数の持続トグル中。
    pub shift_conv_guard_active: bool,
}

/// 打鍵中か(最後のキー活動から [`TYPING_IDLE_MS`] 未満)。
///
/// 読み取り方針(`observe` と `decide_read_strategy`)での打鍵中判定の唯一の定義。
/// `observe`(`ir_observe_read_strategy_facts`)は通過マークの有効判定を打鍵中のときだけ読むため、
/// `decide_read_strategy` は決定のために、それぞれこの関数を呼ぶ。片方だけ式を変えると、
/// 通過マークを読まないまま打鍵中扱いの `SkipTyping` に落ちる
/// (`tests/architecture_guard.rs` が observe 側の経由を固定している)。
///
/// 対象外: `src/engine/idle_check.rs` の打鍵停止判定(idle-conv-check)は、起点が
/// `output_in_flight_ms`・境界が `<=` で、この関数とは別の判定。寄せると idle=500ms の扱いと起点が変わる。
#[must_use]
pub const fn is_typing(idle_ms: u64) -> bool {
    idle_ms < TYPING_IDLE_MS
}

/// 読み取り方針を決める。
#[must_use]
pub fn decide_read_strategy(facts: &ReadStrategyFacts) -> ReadDecision {
    let typing = is_typing(facts.idle_ms);
    let mut typing_guard_bypassed = false;

    if typing {
        // Ctrl+無変換 等の明示的 IME 操作後、実際に OS 状態が変化したか即時検証する。
        // TsfNative/Blacklist アプリは skip_imm_query=true で弾かれるため対象外。
        let explicit_verify = !facts.skip_imm_query
            && (facts.mode_key_pass_live || (facts.explicit_intent_present && facts.applied_known));
        if !explicit_verify {
            return ReadDecision {
                strategy: ImeReadStrategy::SkipTyping,
                reason: ReadReason::TypingActive,
                typing_guard_bypassed,
            };
        }
        typing_guard_bypassed = true;
    }

    // conv=0x00000000 は awase 自身が意図的に設定した状態であり、観測して belief に反映してはならない。
    if facts.shift_conv_guard_active {
        return ReadDecision {
            strategy: ImeReadStrategy::SkipTyping,
            reason: ReadReason::ShiftConvGuard,
            typing_guard_bypassed,
        };
    }

    if facts.skip_imm_query {
        ReadDecision {
            strategy: ImeReadStrategy::Blacklist,
            reason: ReadReason::ImmQuerySkipped,
            typing_guard_bypassed,
        }
    } else {
        ReadDecision {
            strategy: ImeReadStrategy::OsPoll,
            reason: ReadReason::OsPoll,
            typing_guard_bypassed,
        }
    }
}

/// 試作(実験ブランチ `experiment/monad-style-decision` のみ。develop にマージしない): [`decide_read_strategy`] を
/// monad 風の形で書き直した 3 つの版(A〜C)と、monad 風の型を使わない対照(D)。元の関数と全入力で同じ結果を返すことを
/// `tests::monad_style_versions_match_original` が確かめる。読み比べは
/// `docs/tasks/readability-ideas-study-2026-10-06/monad-prototype-comparison.md`。
#[allow(dead_code)]
pub(crate) mod monad_style {
    use std::ops::ControlFlow::{self, Break, Continue};

    use super::{is_typing, ImeReadStrategy, ReadDecision, ReadReason, ReadStrategyFacts};

    // ---- 4 版で共通の部品 ----

    /// 明示的な IME 操作の検証のために、打鍵中ガードを迂回するか。
    const fn explicit_verify(facts: &ReadStrategyFacts) -> bool {
        !facts.skip_imm_query
            && (facts.mode_key_pass_live || (facts.explicit_intent_present && facts.applied_known))
    }

    const fn skip(reason: ReadReason, typing_guard_bypassed: bool) -> ReadDecision {
        ReadDecision {
            strategy: ImeReadStrategy::SkipTyping,
            reason,
            typing_guard_bypassed,
        }
    }

    /// ガードを全部通ったときの読み方。
    const fn poll_target(facts: &ReadStrategyFacts, typing_guard_bypassed: bool) -> ReadDecision {
        let (strategy, reason) = if facts.skip_imm_query {
            (ImeReadStrategy::Blacklist, ReadReason::ImmQuerySkipped)
        } else {
            (ImeReadStrategy::OsPoll, ReadReason::OsPoll)
        };
        ReadDecision {
            strategy,
            reason,
            typing_guard_bypassed,
        }
    }

    // ---- 対照 D: 共通の部品だけを使い、元と同じ早期 return で書く(monad 風の型は使わない) ----

    /// 部品の切り出しと monad 風の形の効果を分けて比べるための対照。
    #[must_use]
    pub(crate) const fn decide_plain(facts: &ReadStrategyFacts) -> ReadDecision {
        let typing = is_typing(facts.idle_ms);
        if typing && !explicit_verify(facts) {
            return skip(ReadReason::TypingActive, false);
        }
        // ここまで来た打鍵中は、明示的な IME 操作の検証のためにガードを迂回した。
        let bypassed = typing;
        if facts.shift_conv_guard_active {
            return skip(ReadReason::ShiftConvGuard, bypassed);
        }
        poll_target(facts, bypassed)
    }

    // ---- 版 A: 標準の `ControlFlow` と `?` ----

    /// `Break` は決定済み(以降の段は呼ばれない)、`Continue` は次の段へ運ぶ値。
    #[must_use]
    pub(crate) fn decide_cf(facts: &ReadStrategyFacts) -> ReadDecision {
        match decide_cf_steps(facts) {
            Break(decision) | Continue(decision) => decision,
        }
    }

    fn decide_cf_steps(facts: &ReadStrategyFacts) -> ControlFlow<ReadDecision, ReadDecision> {
        let bypassed = typing_guard_cf(facts)?;
        shift_conv_guard_cf(facts, bypassed)?;
        Continue(poll_target(facts, bypassed))
    }

    /// 打鍵中ガード。通れば「迂回したか」を次の段へ運ぶ。
    fn typing_guard_cf(facts: &ReadStrategyFacts) -> ControlFlow<ReadDecision, bool> {
        match (is_typing(facts.idle_ms), explicit_verify(facts)) {
            (false, _) => Continue(false),
            (true, true) => Continue(true),
            (true, false) => Break(skip(ReadReason::TypingActive, false)),
        }
    }

    /// conv=0x00000000 は awase 自身が意図的に設定した状態で、観測して belief に反映してはならない。
    fn shift_conv_guard_cf(facts: &ReadStrategyFacts, bypassed: bool) -> ControlFlow<ReadDecision> {
        if facts.shift_conv_guard_active {
            Break(skip(ReadReason::ShiftConvGuard, bypassed))
        } else {
            Continue(())
        }
    }

    // ---- 版 B: 自前の判定型と小さな combinator ----

    /// `Done` は決定済み(以降の段は呼ばれない)、`Next(T)` は次の段へ運ぶ値。Haskell の `Either D T` と `>>=` に当たる。
    /// 自前の型には `?` を使えない(`Try` は stable で実装できない)ので、段は `and_then` でつなぐ。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Decision<T, D> {
        Done(D),
        Next(T),
    }

    impl<T, D> Decision<T, D> {
        /// `cond` が真なら `done` で決定して抜け、偽なら `carry` を次の段へ運ぶ。
        fn guard(cond: bool, done: D, carry: T) -> Self {
            if cond {
                Self::Done(done)
            } else {
                Self::Next(carry)
            }
        }

        fn and_then<U>(self, step: impl FnOnce(T) -> Decision<U, D>) -> Decision<U, D> {
            match self {
                Self::Done(d) => Decision::Done(d),
                Self::Next(t) => step(t),
            }
        }

        fn finish(self, last: impl FnOnce(T) -> D) -> D {
            match self {
                Self::Done(d) => d,
                Self::Next(t) => last(t),
            }
        }
    }

    #[must_use]
    pub(crate) fn decide_combinator(facts: &ReadStrategyFacts) -> ReadDecision {
        typing_guard(facts)
            .and_then(|bypassed| shift_conv_guard(facts, bypassed))
            .finish(|bypassed| poll_target(facts, bypassed))
    }

    fn typing_guard(facts: &ReadStrategyFacts) -> Decision<bool, ReadDecision> {
        if is_typing(facts.idle_ms) {
            Decision::guard(
                !explicit_verify(facts),
                skip(ReadReason::TypingActive, false),
                true,
            )
        } else {
            Decision::Next(false)
        }
    }

    fn shift_conv_guard(facts: &ReadStrategyFacts, bypassed: bool) -> Decision<bool, ReadDecision> {
        Decision::guard(
            facts.shift_conv_guard_active,
            skip(ReadReason::ShiftConvGuard, bypassed),
            bypassed,
        )
    }

    // ---- 版 C: 版 B に Writer 風の蓄積(通った段の名前)を足す ----

    /// `trail` を読む使い手はこのリポジトリに無い(試作のための形)。呼ぶたびに `Vec` を確保する。
    #[must_use]
    pub(crate) fn decide_traced(facts: &ReadStrategyFacts) -> (ReadDecision, Vec<&'static str>) {
        let mut trail = vec!["typing_guard"];
        let decision = typing_guard(facts)
            .and_then(|bypassed| {
                trail.push("shift_conv_guard");
                shift_conv_guard(facts, bypassed)
            })
            .finish(|bypassed| {
                trail.push("poll_target");
                poll_target(facts, bypassed)
            });
        (decision, trail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 試作の 4 版が、元の [`decide_read_strategy`] と全入力(128 通り)で同じ結果を返す。
    #[test]
    fn monad_style_versions_match_original() {
        let idles = [0, TYPING_IDLE_MS - 1, TYPING_IDLE_MS, TYPING_IDLE_MS + 1];
        let mut cases = 0;
        for idle_ms in idles {
            for bits in 0u8..32 {
                let facts = ReadStrategyFacts {
                    idle_ms,
                    skip_imm_query: bits & 1 != 0,
                    mode_key_pass_live: bits & 2 != 0,
                    explicit_intent_present: bits & 4 != 0,
                    applied_known: bits & 8 != 0,
                    shift_conv_guard_active: bits & 16 != 0,
                };
                let original = decide_read_strategy(&facts);
                assert_eq!(monad_style::decide_plain(&facts), original, "D {facts:?}");
                assert_eq!(monad_style::decide_cf(&facts), original, "A {facts:?}");
                assert_eq!(
                    monad_style::decide_combinator(&facts),
                    original,
                    "B {facts:?}"
                );
                let (traced, trail) = monad_style::decide_traced(&facts);
                assert_eq!(traced, original, "C {facts:?}");
                let expected_trail: &[&str] = match original.reason {
                    ReadReason::TypingActive => &["typing_guard"],
                    ReadReason::ShiftConvGuard => &["typing_guard", "shift_conv_guard"],
                    ReadReason::ImmQuerySkipped | ReadReason::OsPoll => {
                        &["typing_guard", "shift_conv_guard", "poll_target"]
                    }
                };
                assert_eq!(trail, expected_trail, "C trail {facts:?}");
                cases += 1;
            }
        }
        assert_eq!(cases, 128);
    }

    /// 全入力の組合せ(idle は打鍵中/アイドルの境界を含む 4 値 × 真偽 5 つ)を、
    /// 元の `ir_decide_read_strategy` の分岐を独立に書き直した期待値と突き合わせる。
    #[test]
    fn exhaustive_table_matches_original_branches() {
        let idles = [0, TYPING_IDLE_MS - 1, TYPING_IDLE_MS, TYPING_IDLE_MS + 1];
        let mut cases = 0;
        for idle_ms in idles {
            for bits in 0u8..32 {
                let facts = ReadStrategyFacts {
                    idle_ms,
                    skip_imm_query: bits & 1 != 0,
                    mode_key_pass_live: bits & 2 != 0,
                    explicit_intent_present: bits & 4 != 0,
                    applied_known: bits & 8 != 0,
                    shift_conv_guard_active: bits & 16 != 0,
                };
                let typing = idle_ms < TYPING_IDLE_MS;
                let verify = !facts.skip_imm_query
                    && (facts.mode_key_pass_live
                        || (facts.explicit_intent_present && facts.applied_known));
                let expected = if typing && !verify {
                    (ImeReadStrategy::SkipTyping, ReadReason::TypingActive)
                } else if facts.shift_conv_guard_active {
                    (ImeReadStrategy::SkipTyping, ReadReason::ShiftConvGuard)
                } else if facts.skip_imm_query {
                    (ImeReadStrategy::Blacklist, ReadReason::ImmQuerySkipped)
                } else {
                    (ImeReadStrategy::OsPoll, ReadReason::OsPoll)
                };
                let actual = decide_read_strategy(&facts);
                assert_eq!((actual.strategy, actual.reason), expected, "{facts:?}");
                assert_eq!(actual.typing_guard_bypassed, typing && verify, "{facts:?}");
                cases += 1;
            }
        }
        assert_eq!(cases, 128);
    }
}
