//! 核の防御の目録（機械可読、ADR-277 決定 4）。
//!
//! 「この防御は要るか」を答えるための表。防御 1 件ごとに、どうやって外すか（`Ablation`）と、外したときに
//! 破れるはずの不変条件を書く。`HarnessSwitch` の防御は `pseudo_ime_exhaustive.rs` が外した状態で事象列を
//! 短い順に全列挙し、最短の違反列を出す（外さなければ違反が無いことも同じ深さまで確かめる）。
//!
//! 新しい防御を足すときの最小スキーマ: id・どこにあるか・外し方・破れる不変条件・根拠（BUG/ADR）。
//! 外し方が `Mutant` のものは `cargo mutants -F <function>` で調べる（Linux で回るのは awase-ime-core 等の
//! ungated な crate だけ）。`NotReached` はハーネスがまだ本物を呼んでいない防御（写しを本物に置き換えるまで、
//! 閉ループは「要る/要らない」を言えない）。

/// 防御の外し方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ablation {
    /// ハーネスの `Defenses` のフィールド名（呼び出し単位で外せる）。
    HarnessSwitch(&'static str),
    /// 核の関数の内側の条件。`cargo mutants` で変異させる（`file`・関数名）。
    Mutant {
        file: &'static str,
        function: &'static str,
    },
    /// ハーネスが本物を通らない（写し・未配線）。理由を書く。
    NotReached(&'static str),
}

/// 防御 1 件。
#[derive(Debug, Clone, Copy)]
pub struct Defense {
    pub id: &'static str,
    /// 本番のどこにあるか（ファイル::関数）。
    pub location: &'static str,
    pub ablation: Ablation,
    /// 外すと破れるはずの不変条件（`support/invariants.rs` の関数名）。
    pub invariant: &'static str,
    /// 根拠（BUG・ADR・コミット）。
    pub evidence: &'static str,
}

/// 目録（2026-10-22 時点。段階 1 は 1 件だけ外せる。残りは ADR-277 の「次の段階」）。
pub const DEFENSES: &[Defense] = &[
    Defense {
        id: "placeholder-align",
        location: "awase-windows/src/state/platform_state.rs::ImeStateHub::align_placeholder_desired",
        ablation: Ablation::HarnessSwitch("placeholder_align"),
        invariant: "p3_startup_aligns_desired_without_drift",
        evidence: "BUG-163 代案A",
    },
    Defense {
        id: "external-close-watch",
        location: "awase-windows/src/state/platform_state.rs::ImeStateHub::follow_external_change_in_scope",
        ablation: Ablation::NotReached(
            "外すスイッチは既にある（Setup::external_close_watch）が、最短列の探索の事象（外部注入・prefetch）を段階 1 では足していない",
        ),
        invariant: "belief_matches_truth_at_end",
        evidence: "ADR-205、BUG-172、closed_loop_scenarios.rs の q4_* 2 本",
    },
    Defense {
        id: "drift-immcross-skip-unwarranted",
        location: "awase-windows/src/runtime/ime_refresh.rs::ir_apply_drift_correction",
        ablation: Ablation::NotReached(
            "ハーネスは decide_drift_plan を呼ばず 1 条件だけを写している（harness.rs の settle）。写しを外しても本番の要否は言えない",
        ),
        invariant: "p3_startup_aligns_desired_without_drift",
        evidence: "BUG-163 の 1 段目 b6ab8980",
    },
    Defense {
        id: "warrant-rejects-non-japanese-ime",
        location: "awase-ime-core/src/state/open_warrant.rs::issue_open_warrant_inner",
        ablation: Ablation::Mutant {
            file: "crates/awase-ime-core/src/state/open_warrant.rs",
            function: "issue_open_warrant_inner",
        },
        invariant: "p1_no_warranted_write_without_intent",
        evidence: "ADR-223、DD3-11。drift_plan の Idle(NotJapaneseIme) と二重（mech-ecrs raw-e 1.3-2）",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defense_ids_are_unique() {
        for (i, d) in DEFENSES.iter().enumerate() {
            assert!(
                DEFENSES[i + 1..].iter().all(|o| o.id != d.id),
                "{} が重複",
                d.id
            );
        }
    }
}
