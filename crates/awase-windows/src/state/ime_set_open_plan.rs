//! `ImeEffect::SetOpen` の dispatch（`runtime/executor.rs::dispatch_ime_set_open`）の判断部分（FCIS の核、ADR-229）。
//!
//! 元の関数は「gate → 押下の予約（claim）→ ImmCross 先頭か否か → async/sync の書き込み」を 1 本に持っていた。
//! 判断（何を・どの経路で・省くか、その理由）だけを純粋関数に出し、殻（executor）は
//! Facts を作る（shell-in）→ ここで決める → 実行する（shell-out）だけにする。
//!
//! 判断は 2 段に分かれる。**claim は `ImeStateHub` の台帳を書き換える**ため、gate が `NotOwned` のときは呼んではならず
//! （元の挙動: 書かない窓では予約しない）、gate の後にしか Facts が揃わない。段の順序と間の副作用は殻が守る:
//!
//! 1. [`plan_set_open_gate`]: InputRelay の gate（`decide_gate` の結果。ADR-119/180、BUG-90 決定4）。
//! 2. [`plan_set_open`]: claim の結果と `imm_first` から、省く／async／sync を決める。
//!
//! 3 関数（`open_chain.rs` の `imm_cross_write`/`fallback_write`/`run_open_chain_async`）が各自 gate を再検出する設計
//! （ADR-180、INV-45）には触れない。ここは executor 入口の判断だけ。

use crate::state::ime_actuation_decision::GateResult;
use crate::state::press_ledger::PressClaim;

/// 入口の gate の判断（1 段目）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetOpenGatePlan {
    /// InputRelay: awase は actuation を所有しない。記録だけ積んで `NotOwned` で返す（claim はしない）。
    RejectNotOwned,
    /// 次の段（claim）へ進む。
    Proceed,
}

/// 2 段目に渡す所有型の Facts。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SetOpenFacts {
    /// `claim_press_write` の結果（gate を通った後に殻が 1 回だけ呼ぶ）。
    pub claim: PressClaim,
    /// `ImeController::imm_cross_is_first_applicable(&view)`（純粋）。
    pub imm_first: bool,
}

/// 2 段目の判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetOpenPlan {
    /// 同じ押下で既に同じ向きを書いた（BUG-113 の二重送信防止）。書かずに `DUPLICATE_OUTCOME` で返す。
    SkipAlreadyClaimed,
    /// ImmCross が先頭で適用可能: spawn_local の非同期経路（`None` を返す）。
    AsyncImmCross,
    /// それ以外（GjiDirect / MsImeDirect / TsfNative）: 同期の chain。
    SyncChain,
}

/// gate の結果から入口の判断を返す。
#[must_use]
pub(crate) const fn plan_set_open_gate(gate: GateResult) -> SetOpenGatePlan {
    match gate {
        GateResult::NotOwned => SetOpenGatePlan::RejectNotOwned,
        GateResult::Proceed => SetOpenGatePlan::Proceed,
    }
}

/// claim と ImmCross 先頭判定から、省く／async／sync を決める。
#[must_use]
pub(crate) const fn plan_set_open(facts: &SetOpenFacts) -> SetOpenPlan {
    if !facts.claim.writes() {
        SetOpenPlan::SkipAlreadyClaimed
    } else if facts.imm_first {
        SetOpenPlan::AsyncImmCross
    } else {
        SetOpenPlan::SyncChain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAIMS: [PressClaim; 6] = [
        PressClaim::Unpressed,
        PressClaim::Fresh,
        PressClaim::Duplicate,
        PressClaim::ConflictEngineWins { reserved: true },
        PressClaim::ConflictEngineWins { reserved: false },
        PressClaim::ConflictKept { reserved: true },
    ];

    #[test]
    fn gate_maps_one_to_one() {
        assert_eq!(
            plan_set_open_gate(GateResult::NotOwned),
            SetOpenGatePlan::RejectNotOwned
        );
        assert_eq!(
            plan_set_open_gate(GateResult::Proceed),
            SetOpenGatePlan::Proceed
        );
    }

    /// 元の分岐 `if !claim.writes() {skip} else if imm_first {async} else {sync}` と全組合せで一致する。
    #[test]
    fn plan_matches_original_branches_exhaustively() {
        for claim in CLAIMS {
            for imm_first in [false, true] {
                let expected = if !claim.writes() {
                    SetOpenPlan::SkipAlreadyClaimed
                } else if imm_first {
                    SetOpenPlan::AsyncImmCross
                } else {
                    SetOpenPlan::SyncChain
                };
                assert_eq!(
                    plan_set_open(&SetOpenFacts { claim, imm_first }),
                    expected,
                    "{claim:?} imm_first={imm_first}"
                );
            }
        }
    }

    #[test]
    fn skip_never_depends_on_imm_first() {
        for imm_first in [false, true] {
            for claim in [
                PressClaim::Duplicate,
                PressClaim::ConflictKept { reserved: true },
            ] {
                assert_eq!(
                    plan_set_open(&SetOpenFacts { claim, imm_first }),
                    SetOpenPlan::SkipAlreadyClaimed
                );
            }
        }
    }
}
