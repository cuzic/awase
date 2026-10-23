//! ME-3(`docs/tasks/mech-ecrs-2026-10-21/INDEX.md`): actuation の純粋な判断の**決定表の全列挙 golden**。
//!
//! 撤去・統合の PR の前後で「決定が 1 つも変わらない」ことを、数秒の Linux テストで判定するための道具
//! (`tests/explicit_press_exhaustive.rs` の方式。complexity-budget の発効条件 TH1e の代わり)。
//! 入力の組を全列挙し、出力の全文を `tests/golden/actuation_decision_table.txt` に固定する。
//!
//! 対象(すべて Facts を受けて決定を返す純粋関数):
//! - `DecisionInputs`(profile 4 × kind 2 × shadow_on 3 × belief_input_mode 11 × candidate_was_seen 2 = 528 通り)ごとに
//!   `decide_gate`・`decide_chain`・`mechanism_applicable`(機構 3)・`imm_cross_is_first_applicable`、
//!   向き 2 ごとに `decide_dispatch_conv_after_open` と `decide_attempt`(機構 3 × `DecisionSite` 8)。
//! - `plan_set_open`(`PressClaim` 7 × `imm_first` 2)。
//! - `explicit_press_applied_pair`(applied 3 × open 2 × has_press 2)・`engine_press_unknowns_applied`(2)。
//!
//! `decide_attempt` の行は、同じ結果になる `DecisionSite` をまとめて書く(まとめ方は結果から機械的に決まり、情報は落とさない)。
//! enum に variant が増えると下の `match` がコンパイルエラーになる(列挙の取りこぼし防止)。
//!
//! 再生成: `UPDATE_GOLDEN=1 cargo test -p awase-windows --lib sync_actuation::exhaustive`
//! (ローカルで走らせない運用では、CI の失敗ログの `----- BEGIN actuation_decision_table -----` から
//! `----- END actuation_decision_table -----` までの間を golden に写す。)

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use awase::engine::{AssumedReason, InputModeState};

use super::{imm_cross_is_first_applicable, mechanism_applicable};
use crate::focus::class_names::AppImeProfile;
use crate::state::actuation_chain::WriteMechanism;
use crate::state::ime_actuation_decision::{
    decide_attempt, decide_chain, decide_dispatch_conv_after_open, decide_gate,
    engine_press_unknowns_applied, explicit_press_applied_pair, DecisionInputs, DecisionSite,
    MechanismCommand,
};
use crate::state::ime_kind::ImeKindId;
use crate::state::ime_set_open_plan::{plan_set_open, SetOpenFacts};
use crate::state::press_ledger::PressClaim;

const PROFILES: [AppImeProfile; 4] = [
    AppImeProfile::Standard,
    AppImeProfile::Imm32Unavailable,
    AppImeProfile::TsfNative,
    AppImeProfile::InputRelay,
];

const SHADOW: [Option<bool>; 3] = [None, Some(false), Some(true)];

const ASSUMED_REASONS: [AssumedReason; 7] = [
    AssumedReason::ImmBridgeBroken,
    AssumedReason::FocusTransition,
    AssumedReason::AppKindExcluded,
    AssumedReason::ForceOnGuardActive,
    AssumedReason::UserHalfWidthAlnumToggleOff,
    AssumedReason::KeyEffectPrediction,
    AssumedReason::Startup,
];

const SITES: [DecisionSite; 8] = [
    DecisionSite::Sync,
    DecisionSite::ImmCrossWrite,
    DecisionSite::FallbackWrite,
    DecisionSite::RunOpenChainAsync,
    DecisionSite::DispatchImeSetOpen,
    DecisionSite::ShadowToggleOff,
    DecisionSite::ShadowToggleOn,
    DecisionSite::BlacklistDriftCorrection,
];

const CLAIMS: [PressClaim; 7] = [
    PressClaim::Unpressed,
    PressClaim::Fresh,
    PressClaim::Duplicate,
    PressClaim::ConflictEngineWins { reserved: false },
    PressClaim::ConflictEngineWins { reserved: true },
    PressClaim::ConflictKept { reserved: false },
    PressClaim::ConflictKept { reserved: true },
];

/// variant が増えたらコンパイルエラーにして、上の列挙表の更新を強いる。
const fn covered_profile(p: AppImeProfile) {
    match p {
        AppImeProfile::Standard
        | AppImeProfile::Imm32Unavailable
        | AppImeProfile::TsfNative
        | AppImeProfile::InputRelay => {}
    }
}

const fn covered_reason(r: AssumedReason) {
    match r {
        AssumedReason::ImmBridgeBroken
        | AssumedReason::FocusTransition
        | AssumedReason::AppKindExcluded
        | AssumedReason::ForceOnGuardActive
        | AssumedReason::UserHalfWidthAlnumToggleOff
        | AssumedReason::KeyEffectPrediction
        | AssumedReason::Startup => {}
    }
}

const fn covered_mode(m: InputModeState) {
    match m {
        InputModeState::ObservedRomaji
        | InputModeState::ObservedKana
        | InputModeState::ObservedEisu
        | InputModeState::AssumedRomaji { .. }
        | InputModeState::Unknown => {}
    }
}

const fn covered_site(s: DecisionSite) {
    match s {
        DecisionSite::Sync
        | DecisionSite::ImmCrossWrite
        | DecisionSite::FallbackWrite
        | DecisionSite::RunOpenChainAsync
        | DecisionSite::DispatchImeSetOpen
        | DecisionSite::ShadowToggleOff
        | DecisionSite::ShadowToggleOn
        | DecisionSite::BlacklistDriftCorrection => {}
    }
}

const fn covered_claim(c: PressClaim) {
    match c {
        PressClaim::Unpressed
        | PressClaim::Fresh
        | PressClaim::Duplicate
        | PressClaim::ConflictEngineWins { .. }
        | PressClaim::ConflictKept { .. } => {}
    }
}

fn input_modes() -> Vec<InputModeState> {
    let mut modes = vec![
        InputModeState::ObservedRomaji,
        InputModeState::ObservedKana,
        InputModeState::ObservedEisu,
    ];
    modes.extend(
        ASSUMED_REASONS
            .iter()
            .map(|&reason| InputModeState::AssumedRomaji { reason }),
    );
    modes.push(InputModeState::Unknown);
    modes
}

fn all_inputs() -> Vec<DecisionInputs> {
    let modes = input_modes();
    let mut out = Vec::new();
    for profile in PROFILES {
        for kind in ImeKindId::ALL {
            for shadow_on in SHADOW {
                for &mode in &modes {
                    for candidate_was_seen in [false, true] {
                        out.push(DecisionInputs::from_facts(
                            profile,
                            kind,
                            shadow_on,
                            mode,
                            candidate_was_seen,
                        ));
                    }
                }
            }
        }
    }
    out
}

const fn tf(b: bool) -> &'static str {
    if b {
        "T"
    } else {
        "F"
    }
}

fn fmt_shadow(s: Option<bool>) -> &'static str {
    match s {
        None => "?",
        Some(b) => tf(b),
    }
}

fn fmt_attempt(pre: bool, cmd: Option<MechanismCommand>) -> String {
    let cmd = cmd.map_or_else(|| "skip".to_string(), |c| format!("{c:?}"));
    if pre {
        format!("romaji+{cmd}")
    } else {
        cmd
    }
}

/// 1 機構ぶんの `decide_attempt` を、同じ結果の site をまとめて書く(全 site が同じなら `*`)。
fn fmt_mechanism(inputs: DecisionInputs, mechanism: WriteMechanism, open: bool) -> String {
    let mut groups: Vec<(String, Vec<DecisionSite>)> = Vec::new();
    for site in SITES {
        covered_site(site);
        let (pre, cmd) = decide_attempt(inputs, site, mechanism, open);
        let key = fmt_attempt(pre, cmd);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, sites)) => sites.push(site),
            None => groups.push((key, vec![site])),
        }
    }
    let body: Vec<String> = groups
        .into_iter()
        .map(|(key, sites)| {
            let names = if sites.len() == SITES.len() {
                "*".to_string()
            } else {
                sites
                    .iter()
                    .map(|s| format!("{s:?}"))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            format!("{names}=>{key}")
        })
        .collect();
    format!("{mechanism:?}{{{}}}", body.join("; "))
}

fn render() -> String {
    let inputs = all_inputs();
    let mut out = String::new();
    out.push_str(
        "# actuation の純粋な判断の決定表(全列挙) golden (ME-3)\n\
         #\n\
         # 生成元: crates/awase-windows/src/state/sync_actuation_exhaustive.rs\n\
         # このファイルは自動生成される。更新は UPDATE_GOLDEN=1 で再生成するか、CI の失敗ログの BEGIN/END の間を写すこと。\n\
         # 撤去・統合の PR でこのファイルが変わらなければ、下の関数の決定は全入力で不変。\n\
         #\n",
    );
    let _ = writeln!(
        out,
        "decision_inputs\t{}\tsites\t{}\tmechanisms\t{}\n",
        inputs.len(),
        SITES.len(),
        WriteMechanism::ALL.len()
    );

    out.push_str(
        "## DecisionInputs ごと: decide_gate / decide_chain / mechanism_applicable / imm_cross_is_first_applicable\n\
         ## 向きごと: decide_dispatch_conv_after_open / decide_attempt(機構 × site。romaji+ は romaji_pre_write、skip は送信なし)\n\n",
    );
    for i in &inputs {
        covered_profile(i.profile);
        covered_mode(i.belief_input_mode);
        if let InputModeState::AssumedRomaji { reason } = i.belief_input_mode {
            covered_reason(reason);
        }
        let chain: Vec<String> = decide_chain(*i).iter().map(|m| format!("{m:?}")).collect();
        let applicable: Vec<String> = WriteMechanism::ALL
            .iter()
            .filter(|m| mechanism_applicable(**m, *i))
            .map(|m| format!("{m:?}"))
            .collect();
        let _ = writeln!(
            out,
            "[{:?} {:?} shadow={} mode={:?} cand={}] gate={:?} chain=[{}] applicable=[{}] imm_first={}",
            i.profile,
            i.kind,
            fmt_shadow(i.shadow_on),
            i.belief_input_mode,
            tf(i.candidate_was_seen),
            decide_gate(*i),
            chain.join(","),
            applicable.join(","),
            tf(imm_cross_is_first_applicable(*i)),
        );
        for open in [false, true] {
            let attempts: Vec<String> = WriteMechanism::ALL
                .iter()
                .map(|m| fmt_mechanism(*i, *m, open))
                .collect();
            let _ = writeln!(
                out,
                "  open={} conv={:?} {}",
                tf(open),
                decide_dispatch_conv_after_open(*i, open),
                attempts.join(" ")
            );
        }
    }

    out.push_str("\n## plan_set_open(claim × imm_first)\n\n");
    for claim in CLAIMS {
        covered_claim(claim);
        for imm_first in [false, true] {
            let _ = writeln!(
                out,
                "{claim:?} imm_first={} => {:?}",
                tf(imm_first),
                plan_set_open(SetOpenFacts { claim, imm_first })
            );
        }
    }

    out.push_str("\n## explicit_press_applied_pair(applied × open × has_press)\n\n");
    for applied in SHADOW {
        for open in [false, true] {
            for has_press in [false, true] {
                let _ = writeln!(
                    out,
                    "applied={} open={} has_press={} => {}",
                    fmt_shadow(applied),
                    tf(open),
                    tf(has_press),
                    fmt_shadow(explicit_press_applied_pair(applied, open, has_press))
                );
            }
        }
    }

    out.push_str("\n## engine_press_unknowns_applied(effectively_tsf_native)\n\n");
    for tsf in [false, true] {
        let _ = writeln!(
            out,
            "effectively_tsf_native={} => {}",
            tf(tsf),
            tf(engine_press_unknowns_applied(tsf))
        );
    }
    out
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("actuation_decision_table.txt")
}

#[test]
fn input_space_size_is_as_documented() {
    assert_eq!(input_modes().len(), 11);
    assert_eq!(all_inputs().len(), 528);
}

#[test]
fn actuation_decision_table_matches_golden() {
    let actual = render();
    let path = golden_path();
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
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
    eprintln!("----- BEGIN actuation_decision_table -----");
    eprint!("{actual}");
    eprintln!("----- END actuation_decision_table -----");
    panic!(
        "actuation の決定表が golden({}) と一致しません。最初の差: {first_diff}\n\
         撤去・統合で決定を変えないはずの PR なら、変えた箇所を見直すこと。意図した変更なら\n\
         `UPDATE_GOLDEN=1 cargo test -p awase-windows --lib sync_actuation::exhaustive` で再生成するか、\n\
         上の BEGIN/END の間(現状の全文)を golden に写して、差分を PR で説明すること。",
        path.display()
    );
}
