//! 同時打鍵のタイミング判定を集約するモジュール。
//!
//! FSM の各 step_* メソッドから呼ばれ、
//! 「2キーは同時打鍵か」「3キーのどちらとペアリングするか」
//! 「投機出力すべきか」を判定する。

use crate::ngram::NgramModel;
use crate::types::Timestamp;

/// 3キー仲裁のタイミングマージン（閾値の30%）
/// d1 と d2 の差がこれ以上ならタイミングだけで判定する
const TIMING_MARGIN_PERCENT: u64 = 30;

/// n-gram 予測で投機出力を選択する最小スコア差
const SPECULATIVE_SCORE_THRESHOLD: f32 = 0.5;

/// n-gram コンテキストウィンドウサイズ
pub(crate) const NGRAM_CONTEXT_SIZE: usize = 3;

/// 3キー仲裁の結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreeKeyResult {
    PairWithChar1,
    PairWithChar2,
}

/// 同時打鍵のタイミング判定器
#[derive(Debug)]
pub struct TimingJudge<'a> {
    threshold_us: u64,
    ngram_model: Option<&'a NgramModel>,
    recent_kana: Vec<char>,
}

impl<'a> TimingJudge<'a> {
    #[must_use]
    pub const fn new(
        threshold_us: u64,
        ngram_model: Option<&'a NgramModel>,
        recent_kana: Vec<char>,
    ) -> Self {
        Self {
            threshold_us,
            ngram_model,
            recent_kana,
        }
    }

    /// 2キー判定: pending_ts と new_ts の間隔が閾値内か。
    /// candidate_kana がある場合、n-gram で閾値を動的調整する。
    #[must_use]
    pub fn is_simultaneous(
        &self,
        pending_ts: Timestamp,
        new_ts: Timestamp,
        candidate_kana: Option<char>,
    ) -> bool {
        let elapsed = new_ts.saturating_sub(pending_ts);
        let threshold = self.adjusted_threshold(candidate_kana);
        elapsed < threshold
    }

    /// 3キー仲裁: char1→thumb→char2 の並びで、thumb をどちらとペアリングするか。
    ///
    /// 判定フロー:
    /// 1. n-gram なし → タイミング比較（d1 < d2 なら char1）
    /// 2. タイミング差が大きい（30%マージン超）→ タイミング優先
    /// 3. タイミングが接近 → n-gram スコアで判定
    #[must_use]
    pub fn three_key_pairing(
        &self,
        char1_ts: Timestamp,
        thumb_ts: Timestamp,
        char2_ts: Timestamp,
        char1_thumb_kana: Option<char>,
        char1_single_kana: Option<char>,
        char2_thumb_kana: Option<char>,
    ) -> ThreeKeyResult {
        let d1 = thumb_ts.saturating_sub(char1_ts);
        let d2 = char2_ts.saturating_sub(thumb_ts);

        let Some(model) = self.ngram_model else {
            return if d1 < d2 {
                ThreeKeyResult::PairWithChar1
            } else {
                ThreeKeyResult::PairWithChar2
            };
        };

        // Phase 1: タイミング差が大きければタイミングだけで決定
        let margin = self.threshold_us * TIMING_MARGIN_PERCENT / 100;
        if d1 + margin < d2 {
            return ThreeKeyResult::PairWithChar1;
        }
        if d2 + margin < d1 {
            return ThreeKeyResult::PairWithChar2;
        }

        // Phase 2: n-gram スコアで判定
        let score_a = char1_thumb_kana.map_or(f32::NEG_INFINITY, |ch| {
            model.frequency_score(&self.recent_kana, ch)
        });

        let score_b = match (char1_single_kana, char2_thumb_kana) {
            (Some(c1), Some(c2)) => {
                let mut extended = self.recent_kana.clone();
                extended.push(c1);
                model.frequency_score(&extended, c2)
            }
            (None, Some(c2)) => model.frequency_score(&self.recent_kana, c2),
            _ => f32::NEG_INFINITY,
        };

        log::trace!(
            "3-key arbitration: d1={d1}µs d2={d2}µs score_a={score_a:.3} score_b={score_b:.3}"
        );

        // スコアが高いほうを選択。同点ならタイミング
        if (score_a - score_b).abs() > f32::EPSILON {
            if score_a > score_b {
                ThreeKeyResult::PairWithChar1
            } else {
                ThreeKeyResult::PairWithChar2
            }
        } else if d1 < d2 {
            ThreeKeyResult::PairWithChar1
        } else {
            ThreeKeyResult::PairWithChar2
        }
    }

    /// 投機出力判定: 通常面のスコアが親指面より十分高ければ投機出力する。
    #[must_use]
    pub fn should_speculate(
        &self,
        normal_kana: Option<char>,
        left_thumb_kana: Option<char>,
        right_thumb_kana: Option<char>,
    ) -> bool {
        let Some(model) = self.ngram_model else {
            return false; // n-gram なし → 投機しない
        };

        let normal_score =
            normal_kana.map_or(0.0, |ch| model.frequency_score(&self.recent_kana, ch));
        let thumb_score = [left_thumb_kana, right_thumb_kana]
            .iter()
            .filter_map(|k| k.map(|ch| model.frequency_score(&self.recent_kana, ch)))
            .fold(f32::NEG_INFINITY, f32::max);
        let thumb_score = if thumb_score == f32::NEG_INFINITY {
            0.0
        } else {
            thumb_score
        };

        normal_score - thumb_score > SPECULATIVE_SCORE_THRESHOLD
    }

    /// n-gram で閾値を動的調整する内部ヘルパー
    fn adjusted_threshold(&self, candidate_kana: Option<char>) -> u64 {
        match (self.ngram_model, candidate_kana) {
            (Some(model), Some(ch)) => model.adjusted_threshold(&self.recent_kana, ch),
            _ => self.threshold_us,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ngram::NgramModel;

    fn sample_model() -> NgramModel {
        let toml_str = r#"
[bigram]
"あり" = 1.5
"した" = 1.8
"をぱ" = -1.5

[trigram]
"ありが" = 2.5
"#;
        NgramModel::from_toml(toml_str, 100_000, 20_000, 30_000, 120_000).unwrap()
    }

    #[test]
    fn is_simultaneous_within_threshold() {
        let judge = TimingJudge::new(100_000, None, vec![]);
        assert!(judge.is_simultaneous(0, 50_000, None));
    }

    #[test]
    fn is_simultaneous_outside_threshold() {
        let judge = TimingJudge::new(100_000, None, vec![]);
        assert!(!judge.is_simultaneous(0, 150_000, None));
    }

    #[test]
    fn three_key_pairing_no_ngram_timing_decides() {
        let judge = TimingJudge::new(100_000, None, vec![]);
        // d1=20, d2=80 → char1
        assert_eq!(
            judge.three_key_pairing(0, 20, 100, None, None, None),
            ThreeKeyResult::PairWithChar1
        );
        // d1=80, d2=20 → char2
        assert_eq!(
            judge.three_key_pairing(0, 80, 100, None, None, None),
            ThreeKeyResult::PairWithChar2
        );
    }

    #[test]
    fn should_speculate_no_model_returns_false() {
        let judge = TimingJudge::new(100_000, None, vec![]);
        assert!(!judge.should_speculate(Some('あ'), Some('ぱ'), None));
    }

    #[test]
    fn should_speculate_with_model_high_normal_score() {
        let model = sample_model();
        // recent = ['あ'], normal = 'り' (score 1.5), thumb candidates have no match (score 0)
        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ']);
        assert!(judge.should_speculate(Some('り'), Some('x'), Some('y')));
    }

    #[test]
    fn should_speculate_with_model_low_difference() {
        let model = sample_model();
        // recent = ['あ'], normal = 'x' (score 0), thumb = 'り' (score 1.5)
        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ']);
        assert!(!judge.should_speculate(Some('x'), Some('り'), None));
    }
}
