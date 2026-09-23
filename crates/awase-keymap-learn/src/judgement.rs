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

/// 決定1a: 正答率がこれ未満なら表全体を不採用にする。
pub const ACCURACY_THRESHOLD: f64 = 0.95;
/// ADR-195 (ii)-1: 縮退率(`1.0 - ScoreReport::confidence()`)がこれを超えたら不採用にする。
/// `key_effect_runtime.rs::MIN_COVERAGE_RATIO`(段階4読み手のセル単位カバレッジ、80%)とは
/// **分母が違う別の量**——こちらはウォークの歩単位(`ScoreReport`)。同じ「20%」に見えて
/// 意味が異なるので混同しないこと(opus-adversarial-consult 2026-09-23 C-4)。
pub const DEGENERATION_THRESHOLD: f64 = 0.20;
/// 決定1a・round2 NM3-2: 判定に使うウォークは、予測した([`ScoreReport::predicted`])
/// ステップ数がこの値以上であること(全ステップ数ではない、C-2)。
pub const MIN_PREDICTED_STEPS: usize = 300;
/// 決定1b項目8: 再測定後も共通セルのこの割合を超えて不一致が残るなら「要確認」にする
/// (系統的バグへの安全弁)。[`ReconciliationSummary::is_systematic_mismatch`]と
/// [`combine`]が使う。再測定オーケストレーション(1b項目7〜9)は未実装のため、
/// 現時点でこの定数を実際に使う呼び出し元はまだ無い。
pub const SYSTEMATIC_MISMATCH_THRESHOLD: f64 = 0.30;

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
    /// 予測したステップ数が[`MIN_PREDICTED_STEPS`]未満（決定1a・round2 NM3-2、C-2）。
    /// 縮退率が高いウォークほど予測が集まりにくいため、この理由自体が縮退の症状で
    /// あることが多い。
    InsufficientSamples,
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
/// round4 M-Dで確定）。標本数不足（`predicted < min_predicted_steps`、C-2）も
/// 同じく`Rejected`側の条件——水増しされた高正答率を信用しないため、
/// 正答率・縮退率のどちらより先にチェックする。すべての不採用条件を
/// くぐり抜けて初めて、Microsoft IME本体は`NeedsConfirmation`
/// （未検証の暫定既定）になる。
#[must_use]
pub fn judge_self_verification(
    score: &ScoreReport,
    is_ms_ime_native: bool,
    accuracy_threshold: f64,
    degeneration_threshold: f64,
    min_predicted_steps: usize,
) -> TableJudgement {
    if score.predicted() < min_predicted_steps {
        return TableJudgement::Rejected(RejectedReason::InsufficientSamples);
    }
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

/// 決定1b項目7〜8: 自己検証の判定([`judge_self_verification`])と、内蔵表との
/// 再測定後の突き合わせ結果（[`ReconciliationSummary`]、あれば）を合成する
/// （opus-adversarial-consult 2026-09-23 C-8）。
///
/// 合成規則:
/// - `self_verification`が既に`Rejected`なら、それを覆さない（系統的不一致が
///   無くても、自己検証で不合格な表を採用してはいけない）。
/// - `reconciliation`が`None`（再測定オーケストレーション未実装、または
///   既知構成でないため突き合わせ自体を行っていない）なら、`self_verification`を
///   そのまま返す。
/// - `reconciliation`が系統的不一致（[`ReconciliationSummary::is_systematic_mismatch`]）を
///   示していれば、`Accepted`だけを`NeedsConfirmation(SystematicMismatch)`へ**下げる**
///   （すでに`NeedsConfirmation(UnverifiedMsImeNative)`だった場合はそちらを残す——
///   理由を上書きしない）。
///
/// 現時点でこの関数を呼ぶ呼び出し元はまだ無い（再測定オーケストレーション未実装、
/// `run_main`は常に`reconciliation: None`で`self_verification`をそのまま使う）。
/// Linux上でテストできるpure関数として先に用意しておくことで、後続PRが
/// `main.rs`（`#[cfg(windows)]`でLinuxのテストが存在しない）に判定ロジックを
/// 書かずに済む。
#[must_use]
pub fn combine(
    self_verification: TableJudgement,
    reconciliation: Option<&ReconciliationSummary>,
    systematic_mismatch_threshold: f64,
) -> TableJudgement {
    let TableJudgement::Accepted = self_verification else {
        return self_verification;
    };
    match reconciliation {
        Some(summary) if summary.is_systematic_mismatch(systematic_mismatch_threshold) => {
            TableJudgement::NeedsConfirmation(NeedsConfirmationReason::SystematicMismatch {
                mismatch_percent: percent(summary.residual_mismatch_rate()),
            })
        }
        _ => self_verification,
    }
}

/// `0.0..=1.0`の比率を`0..=100`のパーセントへ変換する（`NeedsConfirmationReason::
/// SystematicMismatch`が表示用に持つ整数値）。
fn percent(rate: f64) -> u8 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let clamped = (rate * 100.0).round().clamp(0.0, 100.0) as u8;
    clamped
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
            judge_self_verification(&s, true, 0.95, 0.20, 0),
            TableJudgement::Rejected(RejectedReason::LowAccuracy)
        );
        assert_eq!(
            judge_self_verification(&s, false, 0.95, 0.20, 0),
            TableJudgement::Rejected(RejectedReason::LowAccuracy)
        );
    }

    #[test]
    fn high_degeneration_rejects() {
        // 正答率は満点でも、縮退率(1-confidence)が閾値を超えていれば不採用。
        let s = score(50, 0, 250); // predicted 50/300 → degeneration ~83%
        assert_eq!(
            judge_self_verification(&s, false, 0.95, 0.20, 0),
            TableJudgement::Rejected(RejectedReason::HighDegeneration)
        );
    }

    #[test]
    fn ms_ime_native_needs_confirmation_when_thresholds_pass() {
        let s = score(297, 3, 0); // 99% accuracy, 0% degeneration
        assert_eq!(
            judge_self_verification(&s, true, 0.95, 0.20, 300),
            TableJudgement::NeedsConfirmation(NeedsConfirmationReason::UnverifiedMsImeNative)
        );
    }

    #[test]
    fn passes_thresholds_and_not_ms_ime_native_is_accepted() {
        let s = score(297, 3, 0);
        assert_eq!(
            judge_self_verification(&s, false, 0.95, 0.20, 300),
            TableJudgement::Accepted
        );
    }

    /// C-2回帰テスト(opus-adversarial-consult 2026-09-23): 予測したステップ数
    /// (`correct + incorrect`)が最低数未満なら、正答率が100%でも不採用にする
    /// (水増しされた高正答率を信用しない)。
    #[test]
    fn insufficient_predicted_samples_rejects_even_with_perfect_accuracy() {
        let s = score(240, 0, 0); // 100% accuracy, predicted=240 < 300
        assert_eq!(
            judge_self_verification(&s, false, 0.95, 0.20, 300),
            TableJudgement::Rejected(RejectedReason::InsufficientSamples)
        );
    }

    /// C-2回帰テスト: 標本数不足は正答率・縮退率のどちらより先にチェックする
    /// (水増しされた高正答率〈または偶然縮退率をくぐり抜けた値〉を信用しない)。
    #[test]
    fn insufficient_samples_takes_priority_over_accuracy_and_degeneration() {
        let s = score(0, 240, 0); // 0% accuracy, predicted=240 < 300
        assert_eq!(
            judge_self_verification(&s, false, 0.95, 0.20, 300),
            TableJudgement::Rejected(RejectedReason::InsufficientSamples),
            "predicted不足はLowAccuracyより先に報告されるべき"
        );
    }

    #[test]
    fn combine_does_not_override_an_already_rejected_verdict() {
        let rejected = TableJudgement::Rejected(RejectedReason::LowAccuracy);
        let mut summary = ReconciliationSummary::new();
        for _ in 0..100 {
            summary.record(CellReconciliation::NotReproduced);
        }
        assert_eq!(
            combine(rejected, Some(&summary), 0.30),
            rejected,
            "自己検証で不合格な表は、突き合わせ結果に関わらず不採用のまま"
        );
    }

    #[test]
    fn combine_passes_through_accepted_when_reconciliation_is_absent() {
        assert_eq!(
            combine(TableJudgement::Accepted, None, 0.30),
            TableJudgement::Accepted
        );
    }

    #[test]
    fn combine_demotes_accepted_to_needs_confirmation_on_systematic_mismatch() {
        let mut summary = ReconciliationSummary::new();
        for _ in 0..70 {
            summary.record(CellReconciliation::Matched);
        }
        for _ in 0..30 {
            summary.record(CellReconciliation::NotReproduced);
        }
        // ちょうど30%は閾値超過ではない(is_systematic_mismatchと同じ境界)。
        assert_eq!(
            combine(TableJudgement::Accepted, Some(&summary), 0.30),
            TableJudgement::Accepted
        );
        summary.record(CellReconciliation::NotReproduced);
        assert_eq!(
            combine(TableJudgement::Accepted, Some(&summary), 0.30),
            TableJudgement::NeedsConfirmation(NeedsConfirmationReason::SystematicMismatch {
                mismatch_percent: 31,
            })
        );
    }

    #[test]
    fn combine_does_not_override_unverified_ms_ime_native_reason() {
        // 既にUnverifiedMsImeNativeでNeedsConfirmationだった場合、
        // SystematicMismatchで理由を上書きしない(C-8: 元の理由を消さない)。
        let needs_confirmation =
            TableJudgement::NeedsConfirmation(NeedsConfirmationReason::UnverifiedMsImeNative);
        let mut summary = ReconciliationSummary::new();
        for _ in 0..100 {
            summary.record(CellReconciliation::NotReproduced);
        }
        assert_eq!(
            combine(needs_confirmation, Some(&summary), 0.30),
            needs_confirmation
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
}
