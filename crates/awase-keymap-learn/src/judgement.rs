//! 学習結果の採否判定（ADR-196決定1a・1b項目7〜9・1e）。
//!
//! 採否は「内蔵表と一致するかどうか」ではなく、学習結果自身の品質
//! （自己検証の正答率）と、系統的な不一致の有無で決める。判定は
//! 学習セッションの末尾（学習プロセス自身）が行い、不採用の場合も
//! 理由付きで表ファイルに書き出す（決定1e）。
//!
//! 自己検証の採点そのもの（`ScoreReport`・`score_walk`）は[`crate::verify`]の
//! 担当。本モジュールはその結果を使って採否を**判定するだけ**——計算と
//! 判定の役割を分けている（横断レビューで見つかった重複の解消、
//! `SelfVerificationScore`は撤回し`verify::ScoreReport`をそのまま使う）。

use serde::{Deserialize, Serialize};

use crate::verify::ScoreReport;

/// 自己検証の採点結果に、採点に使ったウォークの由来情報（乱数シード）を
/// 添えたもの。`ScoreReport`自体は`score_walk()`の純粋な計算結果であり
/// シードを持たない（ウォークをどう生成したかは呼び出し側のメタデータ）ため、
/// 別途この型で束ねる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoredVerification {
    pub score: ScoreReport,
    pub seed: u64,
}

/// 表全体を不採用にする理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectedReason {
    /// 正答率が閾値未満（決定1a）。
    LowAccuracy,
    /// 縮退率（`1.0 - ScoreReport::confidence()`）が閾値超過
    /// （ADR-195 (ii)-1、20%が既存の暫定値）。
    HighDegeneration,
}

/// 表全体を「要確認」（既定では不採用、ユーザーの明示操作でのみ採用）に
/// する理由。1b-8（系統的な不一致）と1a（Microsoft IME本体）は発火条件が
/// 異なるため、UI側で文言を分ける（決定2、round4 S-g）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NeedsConfirmationReason {
    /// 再測定後も共通セルの一定割合を超えて内蔵表と不一致
    /// （決定1b項目8、系統的バグへの安全弁）。
    SystematicMismatch { mismatch_percent: u8 },
    /// Microsoft IME本体は独立ウォークでの採点実績が無く、95%基準の妥当性が
    /// 未検証（決定1a）。
    UnverifiedMsImeNative,
}

/// 表全体の採否判定（決定1a・1b-8・1e）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableJudgement {
    Accepted,
    NeedsConfirmation(NeedsConfirmationReason),
    Rejected(RejectedReason),
}

/// 表全体の採否条件（決定1a）。
///
/// 呼び出し順序が重要: 正答率・縮退率どちらかで不採用条件を満たせば
/// `Rejected`（Microsoft IME本体でも同様——「95%基準を適用しない」は
/// 「常にNeedsConfirmation」ではなく「正答率を見ないわけではない」の意味、
/// round4 M-Dで確定）。両方の不採用条件をくぐり抜けて初めて、Microsoft
/// IME本体は`NeedsConfirmation`（未検証の暫定既定）になる。
#[must_use]
pub fn judge_self_verification(
    score: &ScoreReport,
    is_ms_ime_native: bool,
    accuracy_threshold: f64,
    degeneration_threshold: f64,
) -> TableJudgement {
    if score.accuracy() < accuracy_threshold {
        return TableJudgement::Rejected(RejectedReason::LowAccuracy);
    }
    let degeneration_rate = 1.0 - score.confidence();
    if degeneration_rate > degeneration_threshold {
        return TableJudgement::Rejected(RejectedReason::HighDegeneration);
    }
    if is_ms_ime_native {
        return TableJudgement::NeedsConfirmation(NeedsConfirmationReason::UnverifiedMsImeNative);
    }
    TableJudgement::Accepted
}

/// 内蔵表との突き合わせ1件分の裁定結果（決定1b項目7〜8）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellReconciliation {
    /// 学習表と内蔵表が最初から一致していた（突き合わせ対象外）。
    Matched,
    /// 不一致だったが、再測定で元の学習値が再現した
    /// （内蔵表側の版ずれ・環境差の強い証拠、学習値を採用する）。
    ReconfirmedByRemeasurement,
    /// 不一致で、再測定でも再現しなかった（偶発的な誤りとみなし、
    /// このセルだけ「予測なし」に落とす）。
    NotReproduced,
    /// 学習表・内蔵表の片方にしか存在しないセル（分母に含めない、
    /// 「表にのみ存在」として別途報告する）。
    OnlyInOneTable,
}

/// 内蔵表との突き合わせ結果の集計（決定1c: 分母は両方の表に存在する
/// セルの共通部分）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconciliationSummary {
    pub matched: u32,
    pub reconfirmed: u32,
    pub not_reproduced: u32,
    pub only_in_one_table: u32,
}

impl ReconciliationSummary {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            matched: 0,
            reconfirmed: 0,
            not_reproduced: 0,
            only_in_one_table: 0,
        }
    }

    pub const fn record(&mut self, outcome: CellReconciliation) {
        match outcome {
            CellReconciliation::Matched => self.matched += 1,
            CellReconciliation::ReconfirmedByRemeasurement => self.reconfirmed += 1,
            CellReconciliation::NotReproduced => self.not_reproduced += 1,
            CellReconciliation::OnlyInOneTable => self.only_in_one_table += 1,
        }
    }

    /// 分母（両方の表に存在するセルの共通部分）。
    #[must_use]
    pub const fn common_cells(&self) -> u32 {
        self.matched + self.reconfirmed + self.not_reproduced
    }

    /// 再測定後もなお不一致のセルの割合（0.0〜1.0）。共通セルが無ければ
    /// `0.0`（判定対象が無いので系統的バグの証拠にならない）。
    #[must_use]
    pub fn residual_mismatch_rate(&self) -> f64 {
        let common = self.common_cells();
        if common == 0 {
            0.0
        } else {
            f64::from(self.not_reproduced) / f64::from(common)
        }
    }

    /// 決定1b項目8: 再測定後も共通セルの一定割合（暫定30%）を超えて
    /// 不一致が残るかどうか。
    #[must_use]
    pub fn is_systematic_mismatch(&self, threshold: f64) -> bool {
        self.residual_mismatch_rate() > threshold
    }
}

/// [`adopt_needs_confirmation`]が拒否した理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptRejected {
    /// 表に採否判定そのものが無い（学習セッションが1a判定まで完走していない）。
    NoJudgement,
    /// 不採用（95%未満・縮退率超過）は、1a「分母の操作によるセル選別での水増しは
    /// 禁止」という安全弁の下でユーザー操作による採用対象にしない。
    Rejected,
}

impl std::fmt::Display for AdoptRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoJudgement => "no_judgement",
            Self::Rejected => "rejected",
        })
    }
}

/// 決定1b-8: 「学習結果を使う」操作(`awase-settings`)から起動される判定書き換え
/// モードの中核。要確認状態(`NeedsConfirmation`、理由は問わない——系統的不一致と
/// Microsoft IME本体未検証のどちらも対象)の判定だけを`Accepted`へ書き換える。
/// 既に採用済み(`Accepted`)の呼び出しは**冪等な成功**として扱う(code-review指摘:
/// 「学習結果を使う」ボタンの二重クリック・UIが結果を取りこぼして再試行、のいずれも
/// 目的の状態〈採用済み〉に既に到達しているのを失敗扱いするとUI側が誤って
/// エラー表示しうるため)。
///
/// 表ファイルへの書き込みは呼び出し側(`awase-keymap-learn-win`、Windows専用の
/// ファイルI/O)の責務。本関数はメモリ上の判定値を書き換えるだけの純粋関数
/// (ホストでユニットテスト可能、決定1b-8のロジック自体はOS非依存)。
pub fn adopt_needs_confirmation(
    judgement: Option<TableJudgement>,
) -> Result<TableJudgement, AdoptRejected> {
    match judgement {
        Some(TableJudgement::NeedsConfirmation(_) | TableJudgement::Accepted) => {
            Ok(TableJudgement::Accepted)
        }
        Some(TableJudgement::Rejected(_)) => Err(AdoptRejected::Rejected),
        None => Err(AdoptRejected::NoJudgement),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn score(correct: usize, incorrect: usize, not_in_table: usize) -> ScoreReport {
        ScoreReport {
            correct,
            incorrect,
            not_in_table,
        }
    }

    #[test]
    fn low_accuracy_rejects_even_for_ms_ime_native() {
        let s = score(50, 50, 0); // 50% < 95%
        assert_eq!(
            judge_self_verification(&s, true, 0.95, 0.20),
            TableJudgement::Rejected(RejectedReason::LowAccuracy)
        );
        assert_eq!(
            judge_self_verification(&s, false, 0.95, 0.20),
            TableJudgement::Rejected(RejectedReason::LowAccuracy)
        );
    }

    #[test]
    fn high_degeneration_rejects() {
        // 正答率は満点でも、縮退率(1-confidence)が閾値を超えていれば不採用。
        let s = score(50, 0, 250); // predicted 50/300 → degeneration ~83%
        assert_eq!(
            judge_self_verification(&s, false, 0.95, 0.20),
            TableJudgement::Rejected(RejectedReason::HighDegeneration)
        );
    }

    #[test]
    fn ms_ime_native_needs_confirmation_when_thresholds_pass() {
        let s = score(297, 3, 0); // 99% accuracy, 0% degeneration
        assert_eq!(
            judge_self_verification(&s, true, 0.95, 0.20),
            TableJudgement::NeedsConfirmation(NeedsConfirmationReason::UnverifiedMsImeNative)
        );
    }

    #[test]
    fn passes_thresholds_and_not_ms_ime_native_is_accepted() {
        let s = score(297, 3, 0);
        assert_eq!(
            judge_self_verification(&s, false, 0.95, 0.20),
            TableJudgement::Accepted
        );
    }

    #[test]
    fn reconciliation_denominator_excludes_only_in_one_table() {
        let mut summary = ReconciliationSummary::new();
        for _ in 0..90 {
            summary.record(CellReconciliation::Matched);
        }
        for _ in 0..5 {
            summary.record(CellReconciliation::NotReproduced);
        }
        for _ in 0..20 {
            summary.record(CellReconciliation::OnlyInOneTable);
        }
        // 分母は90+5=95(matched+not_reproduced)、OnlyInOneTableの20件は含まない。
        assert_eq!(summary.common_cells(), 95);
        assert!((summary.residual_mismatch_rate() - 5.0 / 95.0).abs() < 1e-9);
    }

    #[test]
    fn systematic_mismatch_threshold() {
        let mut summary = ReconciliationSummary::new();
        for _ in 0..70 {
            summary.record(CellReconciliation::Matched);
        }
        for _ in 0..30 {
            summary.record(CellReconciliation::NotReproduced);
        }
        // ちょうど30%は「超過」ではないので系統的不一致にしない。
        assert!(!summary.is_systematic_mismatch(0.30));
        summary.record(CellReconciliation::NotReproduced);
        assert!(summary.is_systematic_mismatch(0.30));
    }

    #[test]
    fn no_common_cells_is_never_systematic() {
        let mut summary = ReconciliationSummary::new();
        summary.record(CellReconciliation::OnlyInOneTable);
        assert_eq!(summary.common_cells(), 0);
        assert!(!summary.is_systematic_mismatch(0.0));
    }

    #[test]
    fn adopt_accepts_systematic_mismatch_needs_confirmation() {
        let judgement = Some(TableJudgement::NeedsConfirmation(
            NeedsConfirmationReason::SystematicMismatch {
                mismatch_percent: 40,
            },
        ));
        assert_eq!(
            adopt_needs_confirmation(judgement),
            Ok(TableJudgement::Accepted)
        );
    }

    #[test]
    fn adopt_accepts_unverified_ms_ime_native_needs_confirmation() {
        let judgement = Some(TableJudgement::NeedsConfirmation(
            NeedsConfirmationReason::UnverifiedMsImeNative,
        ));
        assert_eq!(
            adopt_needs_confirmation(judgement),
            Ok(TableJudgement::Accepted)
        );
    }

    #[test]
    fn adopt_is_idempotent_for_already_accepted() {
        // code-review指摘: 二重クリック・再試行が目的の状態(採用済み)に既に到達して
        // いるのを失敗扱いしない(冪等な成功)。
        assert_eq!(
            adopt_needs_confirmation(Some(TableJudgement::Accepted)),
            Ok(TableJudgement::Accepted)
        );
    }

    #[test]
    fn adopt_rejects_low_accuracy_rejection() {
        // 不採用(95%未満・縮退率超過)は、水増し禁止の安全弁として
        // ユーザー操作での採用対象にしない(1b-8はNeedsConfirmationだけが対象)。
        assert_eq!(
            adopt_needs_confirmation(Some(TableJudgement::Rejected(RejectedReason::LowAccuracy))),
            Err(AdoptRejected::Rejected)
        );
        assert_eq!(
            adopt_needs_confirmation(Some(TableJudgement::Rejected(
                RejectedReason::HighDegeneration
            ))),
            Err(AdoptRejected::Rejected)
        );
    }

    #[test]
    fn adopt_rejects_missing_judgement() {
        assert_eq!(
            adopt_needs_confirmation(None),
            Err(AdoptRejected::NoJudgement)
        );
    }
}
