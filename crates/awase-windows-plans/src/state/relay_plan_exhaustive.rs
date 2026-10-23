//! ME-3(`docs/tasks/mech-ecrs-2026-10-21/INDEX.md`): `relay_plan`(FCIS F3)と `warm_send_plan`(FCIS F6b)の
//! **決定表の全列挙 golden**。
//!
//! 撤去・統合の PR の前後で「executor の即時/キュー/ガードの判断と TSF 送信の warm/cold の判断が 1 つも変わらない」
//! ことを、数秒の Linux テストで判定するための道具(`crates/awase-windows/src/state/sync_actuation_exhaustive.rs`
//! 〈#808〉と同じ方式)。入力の組を列挙し、出力の全文を `tests/golden/relay_plan_table.txt` に固定する。
//!
//! 対象と列挙する軸:
//! - `plan_relay`: `RelayDecisionKind` 3 × `PhysicalKeyDisposition` 2 = 6(全域)。
//! - `plan_consume_effect`: `is_timer` 2(全域)。
//! - `output_in_flight`(= `output_guard_remaining_ms` の `is_some`)と `reinject_wait_remaining`:
//!   `guard_ms` 3(0・1・`OUTPUT_GUARD_MS`)× `elapsed_ms`(0・guard-1・guard・guard+1・`u64::MAX` の重複を除いたもの)、
//!   `reinject_wait_remaining` はさらに `confirm_held_by_tsf` 2。
//! - `drain_needs_wait_check`: `DrainItem` 3 × `guard_passed` 2 = 6(全域)。
//! - `plan_drain_step`: `DrainItem` 3 × `guard_passed` 2 × `wait_remaining` 4(None・Some(0)・Some(1)・Some(`OUTPUT_GUARD_MS`))= 24。
//! - `plan_warmth`: `warm` 2 × `needs_f2_probe` 2 = 4(全域)。
//!
//! 列挙しない軸と理由: `elapsed_ms`・`guard_ms`・`wait_remaining` は `u64` で全域を取れない。判断は
//! `elapsed_ms < guard_ms` の比較と `Some`/`None` の場合分けだけなので、比較の境界(guard-1・guard・guard+1)と
//! 端(0・`u64::MAX`)、`Some` の値は写されるだけなので 0・1・実際の定数だけを取る。
//! enum に variant が増えると下の `match` がコンパイルエラーになる(列挙の取りこぼし防止)。
//!
//! 再生成: `UPDATE_GOLDEN=1 cargo test -p awase-windows-plans --lib relay_plan::exhaustive`
//! (ローカルで走らせない運用では、CI の失敗ログの `----- BEGIN relay_plan_table -----` から
//! `----- END relay_plan_table -----` までの間を golden に写す。)

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::{
    drain_needs_wait_check, output_in_flight, plan_consume_effect, plan_drain_step, plan_relay,
    reinject_wait_remaining, DrainItem, ReinjectWaitFacts, RelayDecisionKind, RelayFacts,
};
use crate::state::physical_disposition::PhysicalKeyDisposition;
use crate::state::warm_send_plan::{plan_warmth, WarmthFacts};
use crate::tuning::OUTPUT_GUARD_MS;

const KINDS: [RelayDecisionKind; 3] = [
    RelayDecisionKind::PassThrough,
    RelayDecisionKind::PassThroughWith,
    RelayDecisionKind::Consume,
];

const PHYS: [PhysicalKeyDisposition; 2] = [
    PhysicalKeyDisposition::Allow,
    PhysicalKeyDisposition::Suppress,
];

const ITEMS: [DrainItem; 3] = [
    DrainItem::Held,
    DrainItem::QueuedReinject,
    DrainItem::QueuedOther,
];

const GUARDS: [u64; 3] = [0, 1, OUTPUT_GUARD_MS];

const WAITS: [Option<u64>; 4] = [None, Some(0), Some(1), Some(OUTPUT_GUARD_MS)];

/// variant が増えたらコンパイルエラーにして、上の列挙表の更新を強いる。
const fn covered_kind(k: RelayDecisionKind) {
    match k {
        RelayDecisionKind::PassThrough
        | RelayDecisionKind::PassThroughWith
        | RelayDecisionKind::Consume => {}
    }
}

const fn covered_phys(p: PhysicalKeyDisposition) {
    match p {
        PhysicalKeyDisposition::Allow | PhysicalKeyDisposition::Suppress => {}
    }
}

const fn covered_item(i: DrainItem) {
    match i {
        DrainItem::Held | DrainItem::QueuedReinject | DrainItem::QueuedOther => {}
    }
}

const fn tf(b: bool) -> &'static str {
    if b {
        "T"
    } else {
        "F"
    }
}

/// `guard` に対する `elapsed` の境界値(重複を除く)。
fn elapsed_values(guard: u64) -> Vec<u64> {
    let mut v = vec![0, guard.saturating_sub(1), guard, guard + 1, u64::MAX];
    v.dedup();
    v
}

fn fmt_opt(v: Option<u64>) -> String {
    v.map_or_else(|| "None".to_string(), |x| format!("Some({x})"))
}

fn render() -> String {
    let mut out = String::new();
    out.push_str(
        "# relay_plan・warm_send_plan の決定表(全列挙) golden (ME-3)\n\
         #\n\
         # 生成元: crates/awase-windows-plans/src/state/relay_plan_exhaustive.rs\n\
         # このファイルは自動生成される。更新は UPDATE_GOLDEN=1 で再生成するか、CI の失敗ログの BEGIN/END の間を写すこと。\n\
         # 撤去・統合の PR でこのファイルが変わらなければ、下の関数の決定は列挙した全入力で不変。\n\
         #\n",
    );
    let _ = writeln!(out, "OUTPUT_GUARD_MS\t{OUTPUT_GUARD_MS}");

    out.push_str("\n## plan_relay(kind × physical)\n\n");
    for kind in KINDS {
        covered_kind(kind);
        for physical in PHYS {
            covered_phys(physical);
            let _ = writeln!(
                out,
                "{kind:?} {physical:?} => {:?}",
                plan_relay(RelayFacts { kind, physical })
            );
        }
    }

    out.push_str("\n## plan_consume_effect(is_timer)\n\n");
    for is_timer in [false, true] {
        let _ = writeln!(
            out,
            "is_timer={} => {:?}",
            tf(is_timer),
            plan_consume_effect(is_timer)
        );
    }

    out.push_str(
        "\n## output_in_flight(defer 側)と reinject_wait_remaining(drain 側)(guard × elapsed × confirm_held_by_tsf)\n\n",
    );
    for guard_ms in GUARDS {
        for output_elapsed_ms in elapsed_values(guard_ms) {
            let in_flight = output_in_flight(output_elapsed_ms, guard_ms);
            let waits: Vec<String> = [false, true]
                .iter()
                .map(|&confirm_held_by_tsf| {
                    format!(
                        "confirm={}=>{}",
                        tf(confirm_held_by_tsf),
                        fmt_opt(reinject_wait_remaining(ReinjectWaitFacts {
                            confirm_held_by_tsf,
                            output_elapsed_ms,
                            guard_ms,
                        }))
                    )
                })
                .collect();
            let _ = writeln!(
                out,
                "guard={guard_ms} elapsed={output_elapsed_ms} => in_flight={} wait[{}]",
                tf(in_flight),
                waits.join(" ")
            );
        }
    }

    out.push_str("\n## drain_needs_wait_check(item × guard_passed)\n\n");
    for item in ITEMS {
        covered_item(item);
        for guard_passed in [false, true] {
            let _ = writeln!(
                out,
                "{item:?} guard_passed={} => {}",
                tf(guard_passed),
                tf(drain_needs_wait_check(item, guard_passed))
            );
        }
    }

    out.push_str("\n## plan_drain_step(item × guard_passed × wait_remaining)\n\n");
    for item in ITEMS {
        for guard_passed in [false, true] {
            for wait in WAITS {
                let _ = writeln!(
                    out,
                    "{item:?} guard_passed={} wait={} => {:?}",
                    tf(guard_passed),
                    fmt_opt(wait),
                    plan_drain_step(item, guard_passed, wait)
                );
            }
        }
    }

    out.push_str("\n## plan_warmth(warm × needs_f2_probe)\n\n");
    for warm in [false, true] {
        for needs_f2_probe in [false, true] {
            let _ = writeln!(
                out,
                "warm={} needs_f2_probe={} => {:?}",
                tf(warm),
                tf(needs_f2_probe),
                plan_warmth(WarmthFacts {
                    warm,
                    needs_f2_probe
                })
            );
        }
    }
    out
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("relay_plan_table.txt")
}

#[test]
fn input_space_size_is_as_documented() {
    let elapsed: usize = GUARDS.iter().map(|&g| elapsed_values(g).len()).sum();
    // guard=0: {0, 1, MAX}、guard=1: {0, 1, 2, MAX}、guard=50: {0, 49, 50, 51, MAX}。
    assert_eq!(elapsed, 3 + 4 + 5);
}

#[test]
fn relay_plan_table_matches_golden() {
    let actual = render();
    let path = golden_path();
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().expect("golden の親"))
            .expect("golden の親を作れない");
        std::fs::write(&path, &actual).expect("golden を書けない");
        eprintln!("golden を更新しました: {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    if actual == expected {
        return;
    }
    let first_diff = actual
        .lines()
        .zip(expected.lines())
        .enumerate()
        .find(|(_, (a, e))| a != e)
        .map_or_else(
            || {
                format!(
                    "行数が違う(現状 {} 行 / golden {} 行)",
                    actual.lines().count(),
                    expected.lines().count()
                )
            },
            |(n, (a, e))| format!("{} 行目\n  現状:   {a}\n  golden: {e}", n + 1),
        );
    eprintln!("----- BEGIN relay_plan_table -----");
    eprint!("{actual}");
    eprintln!("----- END relay_plan_table -----");
    panic!(
        "relay_plan・warm_send_plan の決定表が golden({}) と一致しません。最初の差: {first_diff}\n\
         撤去・統合で決定を変えないはずの PR なら、変えた箇所を見直すこと。意図した変更なら\n\
         `UPDATE_GOLDEN=1 cargo test -p awase-windows-plans --lib relay_plan::exhaustive` で再生成するか、\n\
         上の BEGIN/END の間(現状の全文)を golden に写して、差分を PR で説明すること。",
        path.display()
    );
}
