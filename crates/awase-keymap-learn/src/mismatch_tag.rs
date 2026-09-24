//! 内蔵表との不一致の分布タグ付け（ADR-196 決定1b項目9）。
//!
//! 不一致がキーに集中すれば「版ずれ寄り」、状態に集中・分散すれば「パイプライン疑い」という
//! 参考タグを付ける。単独の採否条件にはしない（[`crate::judgement`]は参照しない）。

use std::collections::BTreeSet;

use crate::model::{KeyId, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchTag {
    /// 不一致なし。
    NoMismatch,
    /// 少数のキーに、複数の状態をまたいで集中している（IME側の版ずれ寄り）。
    VersionDriftLeaning,
    /// 少数の状態に、複数のキーをまたいで集中している、または内蔵表と同じ版なのに不一致
    /// （学習パイプラインの系統的バグ疑い）。
    PipelineSuspect,
    /// どちらにも偏らない。
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MismatchDistribution {
    pub tag: MismatchTag,
    pub mismatched: u32,
    pub distinct_keys: u32,
    pub distinct_statuses: u32,
}

/// `version_matches`は「ユーザーのIME版＝内蔵表の版か」（不明なら`None`）。`Some(true)`で
/// 不一致があれば、分布に関わらずパイプライン疑いとする。
#[must_use]
pub fn tag_mismatches(
    mismatched: &[(Status, KeyId)],
    version_matches: Option<bool>,
) -> MismatchDistribution {
    let keys: BTreeSet<KeyId> = mismatched.iter().map(|&(_, k)| k).collect();
    let statuses: BTreeSet<Status> = mismatched.iter().map(|&(s, _)| s).collect();
    let (nk, ns) = (keys.len(), statuses.len());
    let tag = if mismatched.is_empty() {
        MismatchTag::NoMismatch
    } else if version_matches == Some(true) || ns < nk {
        MismatchTag::PipelineSuspect
    } else if nk < ns {
        MismatchTag::VersionDriftLeaning
    } else {
        MismatchTag::Mixed
    };
    MismatchDistribution {
        tag,
        mismatched: u32::try_from(mismatched.len()).unwrap_or(u32::MAX),
        distinct_keys: u32::try_from(nk).unwrap_or(u32::MAX),
        distinct_statuses: u32::try_from(ns).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(mode: u8) -> Status {
        Status {
            open: true,
            mode,
            composing: false,
        }
    }

    #[test]
    fn empty_is_no_mismatch() {
        let d = tag_mismatches(&[], Some(true));
        assert_eq!(d.tag, MismatchTag::NoMismatch);
        assert_eq!(d.mismatched, 0);
    }

    #[test]
    fn one_key_across_states_leans_version_drift() {
        let m = [(st(0), KeyId(1)), (st(1), KeyId(1)), (st(2), KeyId(1))];
        let d = tag_mismatches(&m, None);
        assert_eq!(d.tag, MismatchTag::VersionDriftLeaning);
        assert_eq!((d.distinct_keys, d.distinct_statuses), (1, 3));
    }

    #[test]
    fn one_state_across_keys_is_pipeline_suspect() {
        let m = [(st(0), KeyId(1)), (st(0), KeyId(2)), (st(0), KeyId(3))];
        assert_eq!(tag_mismatches(&m, None).tag, MismatchTag::PipelineSuspect);
    }

    #[test]
    fn balanced_is_mixed() {
        let m = [(st(0), KeyId(1)), (st(1), KeyId(2))];
        assert_eq!(tag_mismatches(&m, Some(false)).tag, MismatchTag::Mixed);
    }

    #[test]
    fn same_version_overrides_distribution() {
        let m = [(st(0), KeyId(1)), (st(1), KeyId(1))];
        assert_eq!(
            tag_mismatches(&m, Some(true)).tag,
            MismatchTag::PipelineSuspect
        );
        assert_eq!(
            tag_mismatches(&m, Some(false)).tag,
            MismatchTag::VersionDriftLeaning
        );
    }
}
