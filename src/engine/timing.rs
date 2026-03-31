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

    // ── ThreeKeyResult enum variants ──

    #[test]
    fn three_key_result_variants_are_distinct() {
        assert_ne!(ThreeKeyResult::PairWithChar1, ThreeKeyResult::PairWithChar2);
    }

    #[test]
    fn three_key_result_clone_and_copy() {
        let a = ThreeKeyResult::PairWithChar1;
        let b = a; // Copy
        let c = a.clone(); // Clone
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn three_key_result_debug() {
        let s = format!("{:?}", ThreeKeyResult::PairWithChar1);
        assert!(s.contains("PairWithChar1"));
    }

    // ── TimingJudge construction ──

    #[test]
    fn timing_judge_construction_zero_threshold() {
        let judge = TimingJudge::new(0, None, vec![]);
        // With zero threshold, nothing is simultaneous (elapsed must be < 0, impossible for u64)
        assert!(!judge.is_simultaneous(0, 0, None));
        assert!(!judge.is_simultaneous(0, 1, None));
    }

    #[test]
    fn timing_judge_construction_with_model_and_context() {
        let model = sample_model();
        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ', 'り']);
        // Should work normally with non-empty context
        assert!(judge.is_simultaneous(0, 50_000, None));
    }

    #[test]
    fn timing_judge_debug() {
        let judge = TimingJudge::new(100_000, None, vec![]);
        let s = format!("{:?}", judge);
        assert!(s.contains("TimingJudge"));
    }

    // ── is_simultaneous edge cases ──

    #[test]
    fn is_simultaneous_equal_timestamps() {
        // elapsed = 0, threshold = 100_000 → 0 < 100_000 → true
        let judge = TimingJudge::new(100_000, None, vec![]);
        assert!(judge.is_simultaneous(5000, 5000, None));
    }

    #[test]
    fn is_simultaneous_exactly_at_threshold() {
        // elapsed == threshold → not simultaneous (strict <)
        let judge = TimingJudge::new(100_000, None, vec![]);
        assert!(!judge.is_simultaneous(0, 100_000, None));
    }

    #[test]
    fn is_simultaneous_one_below_threshold() {
        let judge = TimingJudge::new(100_000, None, vec![]);
        assert!(judge.is_simultaneous(0, 99_999, None));
    }

    #[test]
    fn is_simultaneous_with_ngram_adjusted_threshold() {
        let model = sample_model();
        // 'あ' → 'り' has bigram score 1.5, which should widen the threshold
        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ']);
        // With model adjustment, the threshold may differ from raw 100_000
        // Just verify it doesn't panic and returns a value
        let _result = judge.is_simultaneous(0, 100_000, Some('り'));
    }

    #[test]
    fn is_simultaneous_saturating_sub_no_panic() {
        // If pending_ts > new_ts, saturating_sub returns 0
        let judge = TimingJudge::new(100_000, None, vec![]);
        assert!(judge.is_simultaneous(200_000, 100_000, None));
    }

    // ── should_speculate edge cases ──

    #[test]
    fn should_speculate_all_none_no_model() {
        let judge = TimingJudge::new(100_000, None, vec![]);
        assert!(!judge.should_speculate(None, None, None));
    }

    #[test]
    fn should_speculate_all_none_with_model() {
        let model = sample_model();
        let judge = TimingJudge::new(100_000, Some(&model), vec![]);
        // normal=None → score 0, both thumbs None → NEG_INFINITY → thumb_score=0
        // 0 - 0 = 0 which is not > 0.5
        assert!(!judge.should_speculate(None, None, None));
    }

    #[test]
    fn should_speculate_only_right_thumb() {
        let model = sample_model();
        // recent = ['あ'], normal = 'り' (1.5), right_thumb = 'x' (0)
        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ']);
        assert!(judge.should_speculate(Some('り'), None, Some('x')));
    }

    #[test]
    fn should_speculate_both_thumbs_high() {
        let model = sample_model();
        // recent = ['し'], normal = 'x' (0), left_thumb = 'た' (1.8), right = None
        let judge = TimingJudge::new(100_000, Some(&model), vec!['し']);
        assert!(!judge.should_speculate(Some('x'), Some('た'), None));
    }

    // ── three_key_pairing with ngram ──

    #[test]
    fn three_key_pairing_with_ngram_timing_dominates() {
        let model = sample_model();
        let judge = TimingJudge::new(100_000, Some(&model), vec![]);
        // d1=10, d2=90 → margin = 30_000, d1+margin=30_010 < 90 is false for small ts
        // Use large timestamps so margin matters
        // d1=10_000, d2=80_000 → margin=30_000, d1+margin=40_000 < 80_000 → PairWithChar1
        assert_eq!(
            judge.three_key_pairing(0, 10_000, 90_000, None, None, None),
            ThreeKeyResult::PairWithChar1
        );
    }

    #[test]
    fn three_key_pairing_with_ngram_d2_dominates() {
        let model = sample_model();
        let judge = TimingJudge::new(100_000, Some(&model), vec![]);
        // d1=80_000, d2=10_000 → margin=30_000, d2+margin=40_000 < 80_000 → PairWithChar2
        assert_eq!(
            judge.three_key_pairing(0, 80_000, 90_000, None, None, None),
            ThreeKeyResult::PairWithChar2
        );
    }

    #[test]
    fn three_key_pairing_equal_d1_d2_no_ngram() {
        let judge = TimingJudge::new(100_000, None, vec![]);
        // d1 == d2 → d1 < d2 is false → PairWithChar2
        assert_eq!(
            judge.three_key_pairing(0, 50, 100, None, None, None),
            ThreeKeyResult::PairWithChar2
        );
    }

    #[test]
    fn three_key_pairing_ngram_score_decides_close_timing() {
        let model = sample_model();
        // recent = ['あ'], char1_thumb_kana = 'り' (score 1.5), char1_single = None, char2_thumb = None
        // score_a = 1.5, score_b = NEG_INFINITY → score_a > score_b → PairWithChar1
        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ']);
        // d1=50_000, d2=50_000 → margin=30_000, d1+margin=80_000 not < 50_000 → close timing
        assert_eq!(
            judge.three_key_pairing(0, 50_000, 100_000, Some('り'), None, None),
            ThreeKeyResult::PairWithChar1
        );
    }

    #[test]
    fn three_key_pairing_ngram_prefers_char2_when_score_b_higher() {
        let model = sample_model();
        // recent = ['し'], char1_thumb_kana = 'x' (0), char1_single = 'し', char2_thumb = 'た' (score 1.8 with context ['し'])
        let judge = TimingJudge::new(100_000, Some(&model), vec!['し']);
        assert_eq!(
            judge.three_key_pairing(0, 50_000, 100_000, Some('x'), Some('し'), Some('た')),
            ThreeKeyResult::PairWithChar2
        );
    }

    #[test]
    fn three_key_pairing_ngram_score_b_with_only_char2_thumb() {
        let model = sample_model();
        // char1_single = None, char2_thumb = 'り' → uses recent_kana directly
        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ']);
        // score_a from char1_thumb_kana=None → NEG_INFINITY
        // score_b from char2_thumb='り' with context=['あ'] → 1.5
        // score_b > score_a → PairWithChar2
        assert_eq!(
            judge.three_key_pairing(0, 50_000, 100_000, None, None, Some('り')),
            ThreeKeyResult::PairWithChar2
        );
    }
}
