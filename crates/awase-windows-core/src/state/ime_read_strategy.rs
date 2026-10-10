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
    /// 英数モードの候補(1 回目の英数の読み)が寿命内で、確認の読みを待っている(ADR-238)。打鍵中でも確認の読みを
    /// 通す(打鍵中の除外で確認が上限なく遅れると、誤採用ではなく本物の切替の確定が遅れる)。
    #[serde(default)]
    pub eisu_candidate_pending: bool,
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
            && (facts.mode_key_pass_live
                || facts.eisu_candidate_pending
                || (facts.explicit_intent_present && facts.applied_known));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 全入力の組合せ(idle は打鍵中/アイドルの境界を含む 4 値 × 真偽 5 つ)を、
    /// 元の `ir_decide_read_strategy` の分岐を独立に書き直した期待値と突き合わせる。
    #[test]
    fn exhaustive_table_matches_original_branches() {
        let idles = [0, TYPING_IDLE_MS - 1, TYPING_IDLE_MS, TYPING_IDLE_MS + 1];
        let mut cases = 0;
        for idle_ms in idles {
            for bits in 0u8..64 {
                let facts = ReadStrategyFacts {
                    idle_ms,
                    skip_imm_query: bits & 1 != 0,
                    mode_key_pass_live: bits & 2 != 0,
                    explicit_intent_present: bits & 4 != 0,
                    applied_known: bits & 8 != 0,
                    shift_conv_guard_active: bits & 16 != 0,
                    eisu_candidate_pending: bits & 32 != 0,
                };
                let typing = idle_ms < TYPING_IDLE_MS;
                let verify = !facts.skip_imm_query
                    && (facts.mode_key_pass_live
                        || facts.eisu_candidate_pending
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
        assert_eq!(cases, 256);
    }
}
