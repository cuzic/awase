//! 検証ウォークの1歩ごとの記録(`--trace-walk`)。採点(`verify::score_walk`)が数えた
//! 正答率のばらつきの原因を、「どのセルで・どの結果が割れたか」から調べるための診断出力。
//!
//! 採点自体には関与しない(表示用の整形だけ)。

use crate::model::{Outcome, Status};
use crate::table::Class;

/// 1歩の採点結果。`verify::score_walk`と同じ規則(予測が無ければ`NotInTable`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepVerdict {
    Correct,
    Incorrect,
    NotInTable,
}

impl StepVerdict {
    #[must_use]
    pub fn of(predicted: Option<Outcome>, actual: Outcome) -> Self {
        match predicted {
            Some(p) if p == actual => Self::Correct,
            Some(_) => Self::Incorrect,
            None => Self::NotInTable,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Correct => "correct",
            Self::Incorrect => "incorrect",
            Self::NotInTable => "not_in_table",
        }
    }
}

fn status_label(s: Status) -> String {
    format!(
        "open={}/mode=0x{:02X}/comp={}",
        u8::from(s.open),
        s.mode,
        u8::from(s.composing)
    )
}

fn outcome_label(o: Outcome) -> String {
    format!("{}/disp={:?}", status_label(o.status), o.disp)
}

fn class_label(c: Class) -> &'static str {
    match c {
        Class::Unmeasured => "unmeasured",
        Class::Single(_) => "single",
        Class::Det(_) => "det",
        Class::HistoryDep => "history_dep",
        Class::NonDet => "nondet",
        Class::Conflict => "conflict",
    }
}

/// 学習表の側から見た、踏んだセルの情報。
#[derive(Debug, Clone, Copy)]
pub struct CellInfo {
    pub class: Class,
    /// そのセルの観測件数。
    pub n_obs: usize,
    /// そのセルで観測された結果の種類数(2以上なら学習時点で割れていた)。
    pub n_distinct: usize,
}

/// 1歩を1行(空白区切りの`key=value`、値に空白を含まない)にする。
#[must_use]
pub fn format_walk_step(
    index: usize,
    before: Status,
    key_vk: u32,
    actual: Outcome,
    predicted: Option<Outcome>,
    cell: CellInfo,
) -> String {
    let verdict = StepVerdict::of(predicted, actual);
    format!(
        "[verify-step] i={index} verdict={} before={} key=0x{key_vk:02X} actual={} predicted={} class={} n_obs={} n_distinct={}",
        verdict.label(),
        status_label(before),
        outcome_label(actual),
        predicted.map_or_else(|| "none".to_string(), outcome_label),
        class_label(cell.class),
        cell.n_obs,
        cell.n_distinct,
    )
}

/// 汚染(外部書き込み等)で採点から除外した1歩。
#[must_use]
pub fn format_contaminated_step(index: usize, before: Status, key_vk: u32) -> String {
    format!(
        "[verify-step] i={index} verdict=contaminated before={} key=0x{key_vk:02X}",
        status_label(before)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Disposition;

    fn st(open: bool, mode: u8, composing: bool) -> Status {
        Status {
            open,
            mode,
            composing,
        }
    }

    fn out(open: bool, mode: u8) -> Outcome {
        Outcome {
            status: st(open, mode, false),
            disp: Disposition::None,
        }
    }

    #[test]
    fn verdict_matches_score_walk_rule() {
        assert_eq!(
            StepVerdict::of(Some(out(true, 9)), out(true, 9)),
            StepVerdict::Correct
        );
        assert_eq!(
            StepVerdict::of(Some(out(true, 9)), out(false, 0)),
            StepVerdict::Incorrect
        );
        assert_eq!(
            StepVerdict::of(None, out(false, 0)),
            StepVerdict::NotInTable
        );
    }

    #[test]
    fn line_is_single_line_key_value_without_stray_spaces() {
        let line = format_walk_step(
            3,
            st(true, 0x09, true),
            0x1C,
            out(true, 0x0B),
            Some(out(true, 0x09)),
            CellInfo {
                class: Class::NonDet,
                n_obs: 4,
                n_distinct: 2,
            },
        );
        assert!(!line.contains('\n'));
        assert!(line.contains("verdict=incorrect"));
        assert!(line.contains("key=0x1C"));
        assert!(line.contains("class=nondet n_obs=4 n_distinct=2"));
        // 値の中に空白が無い(空白で分割して`key=value`を取れる)。
        assert!(line.split_whitespace().skip(1).all(|t| t.contains('=')));
    }

    #[test]
    fn contaminated_line_has_no_prediction_fields() {
        let line = format_contaminated_step(7, st(false, 0, false), 0xF2);
        assert!(line.contains("verdict=contaminated"));
        assert!(!line.contains("predicted="));
    }
}
