//! `ImeEffect::SetOpen` の dispatch（`runtime/executor.rs::dispatch_ime_set_open`）の判断部分（FCIS の核、ADR-229）。
//!
//! 元の関数は「gate → 押下の予約（claim）→ ImmCross 先頭か否か → async/sync の書き込み」を 1 本に持っていた。
//! 判断（何を・どの経路で・省くか、その理由）だけを純粋関数に出し、殻（executor）は
//! Facts を作る（shell-in）→ ここで決める → 実行する（shell-out）だけにする。
//!
//! gate（InputRelay、`decide_gate` の結果 `GateResult`。ADR-119/180、BUG-90 決定4）は呼び出し元が先に見て、
//! `NotOwned` ならここへ来ない。**claim は `ImeStateHub` の台帳を書き換える**ため、gate が `NotOwned` のときは呼んではならず
//! （元の挙動: 書かない窓では予約しない）、gate の後にしか Facts が揃わない。この順序は呼び出し元
//! （ADR-241 決定2 で殻から核の `state/sync_actuation.rs::dispatch_set_open` へ移した）が守り、
//! [`plan_set_open`] は claim の結果と `imm_first` から、省く／async／sync を決める。
//!
//! 3 関数（`open_chain.rs` の `imm_cross_write`/`fallback_write`/`run_open_chain_async`）が各自 gate を再検出する設計
//! （ADR-180、INV-45）には触れない。ここは executor 入口の判断だけ。

use crate::state::press_ledger::PressClaim;

/// 判断に渡す所有型の Facts。
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

/// claim と ImmCross 先頭判定から、省く／async／sync を決める。
#[must_use]
pub(crate) const fn plan_set_open(facts: SetOpenFacts) -> SetOpenPlan {
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

    /// claim ごとの期待値を直接書いた表（実装の `writes()` の写しではない）。(claim, 書くか)
    #[test]
    fn plan_table_is_exhaustive() {
        use SetOpenPlan::{AsyncImmCross, SkipAlreadyClaimed, SyncChain};
        let table: [(PressClaim, bool); 7] = [
            (PressClaim::Unpressed, true),
            (PressClaim::Fresh, true),
            (PressClaim::Duplicate, false),
            (PressClaim::ConflictEngineWins { reserved: true }, true),
            (PressClaim::ConflictEngineWins { reserved: false }, true),
            (PressClaim::ConflictKept { reserved: true }, false),
            (PressClaim::ConflictKept { reserved: false }, false),
        ];
        for (claim, writes) in table {
            let (on_async, on_sync) = if writes {
                (AsyncImmCross, SyncChain)
            } else {
                (SkipAlreadyClaimed, SkipAlreadyClaimed)
            };
            assert_eq!(
                plan_set_open(SetOpenFacts {
                    claim,
                    imm_first: true
                }),
                on_async,
                "{claim:?} imm_first=true"
            );
            assert_eq!(
                plan_set_open(SetOpenFacts {
                    claim,
                    imm_first: false
                }),
                on_sync,
                "{claim:?} imm_first=false"
            );
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
                    plan_set_open(SetOpenFacts { claim, imm_first }),
                    SetOpenPlan::SkipAlreadyClaimed
                );
            }
        }
    }
}
