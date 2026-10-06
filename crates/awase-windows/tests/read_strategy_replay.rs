#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! FCIS F1: `decide_read_strategy`（`state/ime_read_strategy.rs`、`runtime/ime_refresh.rs::ir_decide_read_strategy`
//! の純粋な核）の再生テスト。`tests/journals/read_strategy/*.json`（1 ディレクトリ = 1 形式）の
//! 各ケースは、`observe` が作る事実 `facts` と、決定 `expected`（方針・理由・打鍵ガード迂回の有無）。
//!
//! 現状の fixture は元の分岐を 1 つずつ踏む手組み。実機の journal からの抽出は、`ir_decide_read_strategy`
//! が journal に事実を載せるようになってから（ADR-229 の E1 方針）。

use awase_windows::state::ime_read_strategy::{
    decide_read_strategy, ImeReadStrategy, ReadDecision, ReadReason, ReadStrategyFacts,
};

#[derive(Debug, serde::Deserialize)]
struct Case {
    name: String,
    note: String,
    facts: ReadStrategyFacts,
    expected: ReadDecision,
}

#[test]
fn replay_read_strategy_fixtures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/journals/read_strategy");
    let report = awase_replay::replay_dir::<Case>(&dir, |case| {
        let actual = decide_read_strategy(&case.facts);
        if actual == case.expected {
            Ok(())
        } else {
            Err(format!(
                "{} ({}):\n  expected: {:?}\n  actual:   {:?}",
                case.name, case.note, case.expected, actual
            ))
        }
    });
    report.assert_ok();
    // fixture が全方針・全理由を踏んでいること（ケースの削り込みで分岐が無検査になるのを防ぐ）。
    let mut seen = Vec::new();
    awase_replay::replay_dir::<Case>(&dir, |case| {
        seen.push((case.expected.strategy, case.expected.reason));
        Ok(())
    });
    for want in [
        (ImeReadStrategy::SkipTyping, ReadReason::TypingActive),
        (ImeReadStrategy::SkipTyping, ReadReason::ShiftConvGuard),
        (ImeReadStrategy::Blacklist, ReadReason::ImmQuerySkipped),
        (ImeReadStrategy::OsPoll, ReadReason::OsPoll),
    ] {
        assert!(seen.contains(&want), "fixture が {want:?} を踏んでいない");
    }
}
