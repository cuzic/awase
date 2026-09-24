//! 「バージョン相当の情報」不一致を「要再検証」として扱う判定
//! ([ADR-196](../../../docs/adr/196-keymap-learn-truth-priority.md) 決定3a、
//! [docs/tasks/adr196-t5-revalidation-not-invalidation.md](../../../docs/tasks/adr196-t5-revalidation-not-invalidation.md))。
//!
//! [`crate::staleness`] が扱う「失効」(キーマップ設定自体の変更・永続化スキーマ版の不一致、
//! いずれも即時失効のまま)とは**別枠**の判定である。ここで扱う「バージョン相当の情報」は
//! GJI Converter本体のファイルバージョン、または Microsoft IME 本体の OS ビルド番号・
//! レガシー互換モードフラグ・`keystyle`・キー再割り当て検出の4値であり、内蔵表より
//! 新しいという保証が無いため、不一致を検出しても即座に表を捨てず「要再検証」(次回の
//! 学習機会に段階2単独の軽量再検証を促す)にとどめる。
//!
//! 「要再検証」自体は保存しないフラグで、保存されたバージョンと現在のバージョンを毎回
//! 比較して導出する。実際に永続化フィールドへ配線する処理・軽量再検証の合否判定
//! ([ADR196-T2](../../../docs/tasks/adr196-t2-mismatch-adjudication.md)の95%閾値)は
//! 別タスクの担当であり、本モジュールは比較の純粋ロジックのみを提供する。
//!
//! [`crate::staleness::FingerprintProbe`]と見た目が似た「3値・不明ならfail open」
//! パターンだが、**fail openの規則が異なる**——共有関数化はしていない(code-review指摘、
//! 2026-09-23)。`staleness::check`は「保存側にそもそも指紋が無い(`None`)」なら現在側が
//! 何であってもFresh扱いだが、本モジュールの[`needs_revalidation`]は「どちらか一方でも
//! [`EnvVersionProbe::Unconfirmed`]」なら他方の状態に関わらず常に要再検証にする(規則は
//! [`needs_revalidation`]のdoc参照)。`Unconfirmed`は`staleness`側の型には存在しない
//! 状態(「取得できたが信頼できない」)であり、`staleness`の「保存側が無ければ常にFresh」
//! という規則をそのまま流用すると「未確定」を「不明」と取り違えて見逃す。どちらか一方の
//! fail open規則だけを将来変更する際は、もう一方に同じ変更が必要か必ず確認すること。

use serde::{Deserialize, Serialize};

use crate::judgement::{RejectedReason, ScoredVerification, TableJudgement};
use crate::model::KeyId;
use crate::persist::PersistedTable;
use crate::table::Table;

/// GJI/Microsoft IME本体の「バージョン相当の情報」を凝縮した不透明な4値。
/// 値の意味はIME種別ごとに異なる(GJIは`VS_FIXEDFILEINFO`の
/// `dwFileVersionMS`/`dwFileVersionLS`から得る4値、Microsoft IME本体はOSビルド番号・
/// レガシー互換モードフラグ・`keystyle`・再割り当て検出結果の4値)——本モジュールは
/// 比較にしか関心が無いため区別しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnvVersion(pub [u32; 4]);

/// 永続化ファイルへ書き出す側の値。「方式が無い/取得できなかった」は`Option`の`None`
/// (フィールド自体を省略)で表すため、ここには「取得できた」場合の2種類だけを持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoredEnvVersion {
    /// 学習時点でConverter実行ファイルの最終更新時刻が学習プロセス自身の起動時刻より
    /// 新しく、学習中に版が変わっていない保証ができなかった。以後どの版と比較しても
    /// 常に不一致(要再検証)として扱う。
    Unconfirmed,
    /// 取得できた版。
    Known(EnvVersion),
}

/// 呼び出し側がその場で計算した「現在の環境バージョン」の取得状態。
/// [`StoredEnvVersion`]と違い、比較の相手として「取得元が見つからない」
/// (`Unknown`)も表せる必要があるため3値を持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvVersionProbe {
    /// このIME/構成には版取得の方式が無い、またはConverter等の取得元が見つからない
    /// (比較対象自体が存在しない、fail open)。
    Unknown,
    /// 版取得の方式はあるが、今回は信頼できる値を得られなかった。
    Unconfirmed,
    /// 取得できた。
    Known(EnvVersion),
}

impl StoredEnvVersion {
    /// 学習時に取得した[`EnvVersionProbe`]を永続化用の値へ変換する。`Unknown`は
    /// 「記録しない」(`None`、フィールド自体を省略)。
    #[must_use]
    pub const fn from_probe(probe: EnvVersionProbe) -> Option<Self> {
        match probe {
            EnvVersionProbe::Unknown => None,
            EnvVersionProbe::Unconfirmed => Some(Self::Unconfirmed),
            EnvVersionProbe::Known(v) => Some(Self::Known(v)),
        }
    }
}

impl From<Option<StoredEnvVersion>> for EnvVersionProbe {
    fn from(stored: Option<StoredEnvVersion>) -> Self {
        match stored {
            None => Self::Unknown,
            Some(StoredEnvVersion::Unconfirmed) => Self::Unconfirmed,
            Some(StoredEnvVersion::Known(v)) => Self::Known(v),
        }
    }
}

/// 学習時に記録した環境バージョンと現在の環境バージョンを比較し、「要再検証」かどうかを
/// 判定する。
///
/// 規則([docs/tasks/adr196-t5-revalidation-not-invalidation.md](../../../docs/tasks/adr196-t5-revalidation-not-invalidation.md)
/// 「3a: 状態遷移」に対応):
/// - どちらか一方でも[`EnvVersionProbe::Unconfirmed`]なら、常に要再検証にする
///   (`Unknown`どうしの比較除外の対象外——`Unconfirmed`は`Unknown`より弱い保証しか
///   持たないため、`Unknown`と組み合わさっても「比較不能だから見逃す」扱いにはしない)。
/// - (上記に当てはまらない場合)どちらか一方でも[`EnvVersionProbe::Unknown`]なら、
///   比較不能として要再検証にしない(対称なfail open)。
/// - 両方[`EnvVersionProbe::Known`]で値が異なれば要再検証にする。
#[must_use]
pub fn needs_revalidation(stored: EnvVersionProbe, current: EnvVersionProbe) -> bool {
    match (stored, current) {
        (EnvVersionProbe::Unconfirmed, _) | (_, EnvVersionProbe::Unconfirmed) => true,
        (EnvVersionProbe::Unknown, _) | (_, EnvVersionProbe::Unknown) => false,
        (EnvVersionProbe::Known(a), EnvVersionProbe::Known(b)) => a != b,
    }
}

/// Converter実行ファイルから読んだ版・更新時刻・取得側プロセスの起動時刻から、現在の
/// [`EnvVersionProbe`]を決める(ADR-196決定3b「更新直後の食い違い対策」)。
///
/// - `version`が`None`(Converterが見つからない/版を読めない)なら[`EnvVersionProbe::Unknown`]。
/// - 実行ファイルの最終更新時刻が`process_start`より新しければ、取得した版が学習中に
///   変わった可能性があるため[`EnvVersionProbe::Unconfirmed`]。
/// - 更新時刻が取れない場合は「新しい」と断定できないため`Known`のまま扱う。
#[must_use]
pub fn classify_converter_version(
    version: Option<EnvVersion>,
    exe_modified: Option<std::time::SystemTime>,
    process_start: std::time::SystemTime,
) -> EnvVersionProbe {
    let Some(version) = version else {
        return EnvVersionProbe::Unknown;
    };
    match exe_modified {
        Some(modified) if modified > process_start => EnvVersionProbe::Unconfirmed,
        _ => EnvVersionProbe::Known(version),
    }
}

/// 軽量再検証(段階2単独、[ADR-196](../../../docs/adr/196-keymap-learn-truth-priority.md)
/// 決定3a)の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevalidationOutcome {
    /// 合格(採否条件を満たした)。表を残し、指紋・採点を書き直す。
    Passed,
    /// 不合格。この場合に初めて失効させる(理由付き)。
    Invalidated(RejectedReason),
}

/// 自己検証の判定([`crate::judgement::judge_self_verification`]の結果)から軽量再検証の
/// 合否を決める。`Rejected`だけが失効で、`Accepted`/`NeedsConfirmation`(Microsoft IME本体の
/// 暫定既定)は合格側——要確認かどうかは元の表の判定を引き継ぐため、ここでは覆さない。
#[must_use]
pub const fn outcome_of_revalidation(self_verification: TableJudgement) -> RevalidationOutcome {
    match self_verification {
        TableJudgement::Rejected(reason) => RevalidationOutcome::Invalidated(reason),
        TableJudgement::Accepted | TableJudgement::NeedsConfirmation(_) => {
            RevalidationOutcome::Passed
        }
    }
}

/// 軽量再検証の結果を、既存の表へ反映した新しい表を返す(書き込みは呼び出し側が
/// 一時ファイル→置換でアトミックに行う)。
///
/// - 合格: `env_version`(現在の版)と`verification`(新しい採点)を書き直す。`judgement`は
///   元のまま(元が要確認なら要確認のまま)。
/// - 失効: `judgement`を`Rejected`へ落とし`verification`だけ更新する。`env_version`は
///   書き直さない(失効した表に現在の版を付けると、次回以降の比較が誤って一致し得る)。
#[must_use]
pub fn apply_revalidation(
    mut table: PersistedTable,
    outcome: RevalidationOutcome,
    current_env_version: Option<StoredEnvVersion>,
    verification: ScoredVerification,
) -> PersistedTable {
    table.verification = Some(verification);
    match outcome {
        RevalidationOutcome::Passed => table.env_version = current_env_version,
        RevalidationOutcome::Invalidated(reason) => {
            table.judgement = Some(TableJudgement::Rejected(reason));
        }
    }
    table
}

/// 保存済みの表から、軽量再検証の採点に使う観測表を作る。予測が入っているセルごとに
/// 同じ結果を2件記録し(決定的なセルとして扱われる)、予測が無い(`None`)セルは
/// 未測定のまま(採点で「表に無い」扱い)にする。`key_index`はVKコードから実機ドライバの
/// キー添字への対応(該当が無ければそのセルは飛ばす)。
#[must_use]
pub fn table_from_persisted(
    persisted: &PersistedTable,
    key_index: impl Fn(KeyId) -> Option<usize>,
) -> Table {
    let mut table = Table::new();
    for cell in &persisted.cells {
        let (Some(outcome), Some(key)) = (cell.prediction, key_index(cell.key)) else {
            continue;
        };
        table.record(cell.status, key, None, outcome);
        table.record(cell.status, key, None, outcome);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: EnvVersion = EnvVersion([1, 2, 3, 4]);
    const V2: EnvVersion = EnvVersion([1, 2, 3, 5]);

    #[test]
    fn up_to_date_when_known_versions_match() {
        assert!(!needs_revalidation(
            EnvVersionProbe::Known(V1),
            EnvVersionProbe::Known(V1)
        ));
    }

    #[test]
    fn needs_revalidation_when_known_versions_differ() {
        assert!(needs_revalidation(
            EnvVersionProbe::Known(V1),
            EnvVersionProbe::Known(V2)
        ));
    }

    #[test]
    fn fail_open_when_stored_unknown() {
        assert!(!needs_revalidation(
            EnvVersionProbe::Unknown,
            EnvVersionProbe::Known(V1)
        ));
    }

    #[test]
    fn fail_open_when_current_unknown() {
        assert!(!needs_revalidation(
            EnvVersionProbe::Known(V1),
            EnvVersionProbe::Unknown
        ));
    }

    #[test]
    fn fail_open_when_both_unknown() {
        assert!(!needs_revalidation(
            EnvVersionProbe::Unknown,
            EnvVersionProbe::Unknown
        ));
    }

    #[test]
    fn stored_unconfirmed_always_needs_revalidation_even_against_known() {
        assert!(needs_revalidation(
            EnvVersionProbe::Unconfirmed,
            EnvVersionProbe::Known(V1)
        ));
    }

    #[test]
    fn stored_unconfirmed_needs_revalidation_even_against_unknown() {
        // 「不明」どうしの比較除外(fail open)の対象外——Unconfirmedは弱い保証しか
        // 持たないため、現在側が取得元不明でも「見逃さない」側に倒す。
        assert!(needs_revalidation(
            EnvVersionProbe::Unconfirmed,
            EnvVersionProbe::Unknown
        ));
    }

    #[test]
    fn current_unconfirmed_always_needs_revalidation() {
        assert!(needs_revalidation(
            EnvVersionProbe::Known(V1),
            EnvVersionProbe::Unconfirmed
        ));
    }

    #[test]
    fn both_unconfirmed_needs_revalidation() {
        assert!(needs_revalidation(
            EnvVersionProbe::Unconfirmed,
            EnvVersionProbe::Unconfirmed
        ));
    }

    #[test]
    fn stored_none_converts_to_unknown_probe() {
        assert_eq!(EnvVersionProbe::from(None), EnvVersionProbe::Unknown);
    }

    #[test]
    fn stored_known_round_trips_through_probe_conversion() {
        assert_eq!(
            EnvVersionProbe::from(Some(StoredEnvVersion::Known(V1))),
            EnvVersionProbe::Known(V1)
        );
    }

    #[test]
    fn stored_unconfirmed_converts_to_unconfirmed_probe() {
        assert_eq!(
            EnvVersionProbe::from(Some(StoredEnvVersion::Unconfirmed)),
            EnvVersionProbe::Unconfirmed
        );
    }

    #[test]
    fn from_probe_round_trips_through_stored() {
        for probe in [
            EnvVersionProbe::Unknown,
            EnvVersionProbe::Unconfirmed,
            EnvVersionProbe::Known(V1),
        ] {
            assert_eq!(
                EnvVersionProbe::from(StoredEnvVersion::from_probe(probe)),
                probe
            );
        }
    }

    #[test]
    fn classify_unknown_when_version_missing() {
        let now = std::time::SystemTime::now();
        assert_eq!(
            classify_converter_version(None, Some(now), now),
            EnvVersionProbe::Unknown
        );
    }

    #[test]
    fn classify_unconfirmed_when_exe_newer_than_process_start() {
        let start = std::time::SystemTime::UNIX_EPOCH;
        let modified = start + std::time::Duration::from_secs(1);
        assert_eq!(
            classify_converter_version(Some(V1), Some(modified), start),
            EnvVersionProbe::Unconfirmed
        );
    }

    #[test]
    fn classify_known_when_exe_not_newer_or_mtime_unavailable() {
        let start = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10);
        let older = std::time::SystemTime::UNIX_EPOCH;
        assert_eq!(
            classify_converter_version(Some(V1), Some(older), start),
            EnvVersionProbe::Known(V1)
        );
        assert_eq!(
            classify_converter_version(Some(V1), Some(start), start),
            EnvVersionProbe::Known(V1)
        );
        assert_eq!(
            classify_converter_version(Some(V1), None, start),
            EnvVersionProbe::Known(V1)
        );
    }

    fn persisted_cell(open: bool, vk: u16, pred: Option<bool>) -> crate::persist::PersistedCell {
        use crate::model::{Disposition, Outcome, Status};
        let status = |open| Status {
            open,
            mode: 0,
            composing: false,
        };
        crate::persist::PersistedCell {
            status: status(open),
            key: KeyId(vk),
            prediction: pred.map(|p| Outcome {
                status: status(p),
                disp: Disposition::None,
            }),
        }
    }

    #[test]
    fn table_from_persisted_records_predicted_cells_as_deterministic() {
        use crate::table::Class;
        let persisted = PersistedTable::new(vec![
            persisted_cell(false, 0xF2, Some(true)),
            persisted_cell(true, 0xF2, None),
            persisted_cell(true, 0x99, Some(false)),
        ]);
        let table = table_from_persisted(&persisted, |k| (k.0 == 0xF2).then_some(2));
        let closed = crate::model::Status {
            open: false,
            mode: 0,
            composing: false,
        };
        let open = crate::model::Status {
            open: true,
            ..closed
        };
        assert!(matches!(table.class(closed, 2), Class::Det(_)));
        assert_eq!(table.class(open, 2), Class::Unmeasured);
        assert_eq!(table.covered1(), 1);
    }

    #[test]
    fn only_rejected_self_verification_invalidates() {
        use crate::judgement::NeedsConfirmationReason;
        assert_eq!(
            outcome_of_revalidation(TableJudgement::Accepted),
            RevalidationOutcome::Passed
        );
        assert_eq!(
            outcome_of_revalidation(TableJudgement::NeedsConfirmation(
                NeedsConfirmationReason::UnverifiedMsImeNative
            )),
            RevalidationOutcome::Passed
        );
        assert_eq!(
            outcome_of_revalidation(TableJudgement::Rejected(RejectedReason::LowAccuracy)),
            RevalidationOutcome::Invalidated(RejectedReason::LowAccuracy)
        );
    }

    fn verification(seed: u64) -> ScoredVerification {
        ScoredVerification {
            score: crate::verify::ScoreReport {
                correct: 300,
                incorrect: 0,
                not_in_table: 0,
            },
            seed,
        }
    }

    #[test]
    fn passed_rewrites_env_version_and_verification_but_keeps_judgement() {
        use crate::judgement::NeedsConfirmationReason;
        let old = PersistedTable::new(vec![])
            .with_env_version(Some(StoredEnvVersion::Known(V1)))
            .with_judgement(TableJudgement::NeedsConfirmation(
                NeedsConfirmationReason::UnverifiedMsImeNative,
            ))
            .with_verification(verification(1));
        let new = apply_revalidation(
            old.clone(),
            RevalidationOutcome::Passed,
            Some(StoredEnvVersion::Known(V2)),
            verification(2),
        );
        assert_eq!(new.env_version, Some(StoredEnvVersion::Known(V2)));
        assert_eq!(new.verification, Some(verification(2)));
        assert_eq!(new.judgement, old.judgement);
    }

    #[test]
    fn invalidated_rejects_judgement_and_keeps_old_env_version() {
        let old = PersistedTable::new(vec![])
            .with_env_version(Some(StoredEnvVersion::Known(V1)))
            .with_judgement(TableJudgement::Accepted);
        let new = apply_revalidation(
            old,
            RevalidationOutcome::Invalidated(RejectedReason::LowAccuracy),
            Some(StoredEnvVersion::Known(V2)),
            verification(3),
        );
        assert_eq!(
            new.judgement,
            Some(TableJudgement::Rejected(RejectedReason::LowAccuracy))
        );
        assert_eq!(new.env_version, Some(StoredEnvVersion::Known(V1)));
        assert_eq!(new.verification, Some(verification(3)));
    }

    #[test]
    fn apply_revalidation_keeps_the_stored_fingerprint_in_both_outcomes() {
        // 再検証は指紋を書き換えない(GJIの表を別構成の下で「再検証合格」させる経路を
        // 呼び出し側が先に塞ぐ前提。ここでは書き換えないことだけを固定する)。
        let fp = crate::persist::Fingerprint(7, 9);
        let old = PersistedTable::new(vec![])
            .with_fingerprint(fp)
            .with_judgement(TableJudgement::Accepted);
        let passed = apply_revalidation(
            old.clone(),
            RevalidationOutcome::Passed,
            None,
            verification(1),
        );
        assert_eq!(passed.fingerprint, Some(fp));
        let invalidated = apply_revalidation(
            old,
            RevalidationOutcome::Invalidated(RejectedReason::LowAccuracy),
            None,
            verification(1),
        );
        assert_eq!(invalidated.fingerprint, Some(fp));
    }
}
