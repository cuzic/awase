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
            .into_iter()
            .filter_map(|k| k.map(|ch| model.frequency_score(&self.recent_kana, ch)))
            .reduce(f32::max)
            .unwrap_or(0.0);

        normal_score - thumb_score > SPECULATIVE_SCORE_THRESHOLD
    }

    /// n-gram で閾値を動的調整する内部ヘルパー
    fn adjusted_threshold(&self, candidate_kana: Option<char>) -> u64 {
        match (self.ngram_model, candidate_kana) {
            (Some(model), Some(ch)) => {
                model.adjusted_threshold(self.threshold_us, &self.recent_kana, ch)
            }
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
        NgramModel::from_toml(toml_str, 20_000, 30_000, 120_000).unwrap()
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

    // ── margin 計算 (line 91: threshold_us * TIMING_MARGIN_PERCENT / 100) ──
    //
    // 以下のテストは、margin が正しく計算されている（= threshold の 30%）ことを
    // 「d1<d2 の生タイミング比較」と「n-gram スコア比較」の結果が食い違う場面を
    // 選んで確認する。margin の掛け算/割り算を壊す変異（*→+, *→/, /→%, /→*）は、
    // margin の実際の値を大きく変えてしまい、Phase1（タイミングのみで即決）が
    // 発火するかどうかが変わる。Phase1 と Phase2（n-gram スコア）で意図的に
    // 逆の結論になるよう仕込むことで、margin の値そのものを間接的に検証する。

    #[test]
    fn three_key_pairing_char1_branch_fires_with_correct_margin_inflated_margin_would_not() {
        let model = sample_model();
        // recent=['し'], char1_single='し', char2_thumb='た' → score_b = bigram(し,た) = 1.8
        // char1_thumb='x' (未知の組み合わせ) → score_a = 0.0
        // score_b > score_a なので、Phase2 に落ちると PairWithChar2 になる。
        let judge = TimingJudge::new(100_000, Some(&model), vec!['し']);
        // d1 = thumb_ts - char1_ts = 10_000, d2 = char2_ts - thumb_ts = 50_000
        // 正しい margin = 100_000*30/100 = 30_000 → d1+margin=40_000 < d2=50_000 → Phase1 発火
        // → タイミングのみで PairWithChar1（スコアは見ない）
        // margin が肥大化する変異（*→+ で 100_000、/→* で 300_000_000）だと
        // d1+margin が d2 を超え Phase1 が発火せず Phase2（スコア）に落ちて PairWithChar2 になる
        // → 期待値と食い違い、変異を検出できる。
        assert_eq!(
            judge.three_key_pairing(0, 10_000, 60_000, Some('x'), Some('し'), Some('た')),
            ThreeKeyResult::PairWithChar1
        );
    }

    #[test]
    fn three_key_pairing_char1_branch_does_not_fire_with_correct_margin_shrunk_margin_would_fire() {
        let model = sample_model();
        // 同じスコア構成（score_b=1.8 > score_a=0.0）で Phase2 に落ちれば PairWithChar2。
        let judge = TimingJudge::new(100_000, Some(&model), vec!['し']);
        // thumb_ts=10_000, char2_ts=20_500 → d1=10_000, d2=char2_ts-thumb_ts=10_500。
        // 正しい margin=30_000 では d1+margin=40_000 < d2=10_500 は false
        // → Phase1 は発火せず Phase2 に落ちる → PairWithChar2。
        // margin が縮小する変異（*→/ で 33、/→% で 0）だと d1+margin ≈ 10_033/10_000 が
        // d2=10_500 未満になり Phase1 が誤って発火して PairWithChar1 になる → 検出できる
        // （d2 をわざと d1 のすぐ近くに置くことで、逆方向の branch2 (d2+margin<d1) が
        // 縮小後の margin でも発火しないようにしている）。
        assert_eq!(
            judge.three_key_pairing(0, 10_000, 20_500, Some('x'), Some('し'), Some('た')),
            ThreeKeyResult::PairWithChar2
        );
    }

    #[test]
    fn three_key_pairing_char1_branch_exact_margin_boundary_does_not_fire() {
        let model = sample_model();
        let judge = TimingJudge::new(100_000, Some(&model), vec!['し']);
        // thumb_ts=10_000, char2_ts=50_000 → d1=10_000, d2=char2_ts-thumb_ts=40_000。
        // margin=30_000 ちょうどで d2=d1+margin。
        // `<` は非包含なので d1+margin < d2 は false（40_000 < 40_000）→ Phase2 に落ちて PairWithChar2。
        // `<=` や `==` に変異すると true になり Phase1 が発火して PairWithChar1 になる → 検出できる。
        assert_eq!(
            judge.three_key_pairing(0, 10_000, 50_000, Some('x'), Some('し'), Some('た')),
            ThreeKeyResult::PairWithChar2
        );
    }

    #[test]
    fn three_key_pairing_char2_branch_fires_with_correct_margin_inflated_margin_would_not() {
        let model = sample_model();
        // recent=['し'], char1_thumb='た' → score_a = bigram(し,た) = 1.8
        // char1_single=None, char2_thumb='x'(未知) → score_b = 0.0 → score_a > score_b → Phase2 なら PairWithChar1
        let judge = TimingJudge::new(100_000, Some(&model), vec!['し']);
        // d1 = 50_000, d2 = 10_000 → 正しい margin=30_000: d2+margin=40_000 < d1=50_000 → Phase1 発火 → PairWithChar2
        // margin が肥大化すると d2+margin > d1 になり Phase1 が発火せず Phase2 で PairWithChar1 になる → 検出できる。
        assert_eq!(
            judge.three_key_pairing(0, 50_000, 60_000, Some('た'), None, Some('x')),
            ThreeKeyResult::PairWithChar2
        );
    }

    #[test]
    fn three_key_pairing_char2_branch_does_not_fire_with_correct_margin_shrunk_margin_would_fire() {
        let model = sample_model();
        // 同じスコア構成（score_a=1.8 > score_b=0.0）で Phase2 に落ちれば PairWithChar1。
        let judge = TimingJudge::new(100_000, Some(&model), vec!['し']);
        // d1=11_000, d2=10_000 (gap=1_000)。正しい margin=30_000 では
        // d2+margin=40_000 < d1=11_000 は false → Phase1 は発火せず Phase2 に落ちて PairWithChar1。
        // margin が縮小すると d2+margin ≈ 10_033/10_000 が d1=11_000 未満になり
        // Phase1 が誤って発火して PairWithChar2 になる → 検出できる。
        assert_eq!(
            judge.three_key_pairing(0, 11_000, 21_000, Some('た'), None, Some('x')),
            ThreeKeyResult::PairWithChar1
        );
    }

    #[test]
    fn three_key_pairing_char2_branch_exact_margin_boundary_does_not_fire() {
        let model = sample_model();
        let judge = TimingJudge::new(100_000, Some(&model), vec!['し']);
        // d2=10_000, margin=30_000 ちょうどで d1=d2+margin=40_000。
        // `<` は非包含なので d2+margin < d1 は false（40_000 < 40_000）→ Phase2 に落ちて PairWithChar1。
        // `<=` や `==` に変異すると true になり Phase1 が発火して PairWithChar2 になる → 検出できる。
        assert_eq!(
            judge.three_key_pairing(0, 40_000, 50_000, Some('た'), None, Some('x')),
            ThreeKeyResult::PairWithChar1
        );
    }

    // ── score_b の match アーム (line 104-111) ──

    #[test]
    fn three_key_pairing_char2_thumb_only_score_overrides_raw_timing() {
        let model = sample_model();
        // char1_single=None, char2_thumb='り' → (None, Some(c2)) アーム経由で
        // score_b = frequency_score(recent, 'り') = bigram(あ,り) = 1.5 を使う。
        // このアームが削除されると `_` にフォールして score_b = NEG_INFINITY になり、
        // score_a も NEG_INFINITY（char1_thumb=None）なので diff が NaN になり
        // タイブレーク（生の d1<d2 比較）に落ちる。
        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ']);
        // d1=40_000 < d2=50_000（生タイミングなら char1 を選ぶ）が、gap=10_000 は
        // margin=30_000 以内なので Phase2 に入り、score_b=1.5 > score_a=-inf で PairWithChar2。
        // アームが削除されると score_b も -inf になりタイブレークが d1<d2=true→PairWithChar1 になる
        // → 生タイミングと逆の結果になるため検出できる。
        assert_eq!(
            judge.three_key_pairing(0, 40_000, 90_000, None, None, Some('り')),
            ThreeKeyResult::PairWithChar2
        );
    }

    // ── スコア差の比較 (line 118-129) ──

    #[test]
    fn three_key_pairing_score_diff_uses_subtraction_not_addition() {
        // score_a=1.5 (bigram あり), score_b=-1.5 (bigram をぱ, extended=['あ','を']) →
        // diff = 1.5 - (-1.5) = 3.0 > EPSILON → score_a > score_b → PairWithChar1。
        // `-`→`+` に変異すると diff = 1.5 + (-1.5) = 0.0 になり EPSILON 判定を通らず
        // タイブレーク(d1<d2)に落ちる。d1=60_000 > d2=50_000 なので d1<d2=false → PairWithChar2
        // となり、正しい結果 PairWithChar1 と食い違うため検出できる。
        let model = sample_model();
        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ']);
        // gap = |d1-d2| = 10_000 <= margin(30_000) なので Phase2 に入る。
        assert_eq!(
            judge.three_key_pairing(0, 60_000, 110_000, Some('り'), Some('を'), Some('ぱ')),
            ThreeKeyResult::PairWithChar1
        );
    }

    #[test]
    fn three_key_pairing_score_diff_exact_epsilon_boundary_excludes_equal() {
        // diff.abs() が f32::EPSILON にちょうど一致する場合、`>` は false（除外）で
        // タイブレークに落ちるべき。`>=` に変異すると true になりスコア分岐に入ってしまう。
        //
        // f32::EPSILON (= 2^-23 = 1.1920929e-7) を bigram スコアとして直接埋め込み、
        // もう片方を 0.0 (未知の組み合わせ) にすることで diff をちょうど EPSILON にする。
        let toml_str = r#"
[bigram]
"あり" = 1.1920929e-7
"#;
        let model = NgramModel::from_toml(toml_str, 20_000, 30_000, 120_000).unwrap();
        // 前提確認: score_a が本当に f32::EPSILON と bit-exact であること
        assert_eq!(model.frequency_score(&['あ'], 'り'), f32::EPSILON);

        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ']);
        // char1_thumb='り' → score_a=f32::EPSILON。char2_thumb='x'(未知)→score_b=0.0。
        // diff.abs()=EPSILON。正しい `>` では EPSILON>EPSILON=false → タイブレークへ。
        // d1=60_000 > d2=50_000 なので d1<d2=false → PairWithChar2（タイブレークの結果）。
        // `>=` に変異すると true になりスコア分岐へ入り、score_a(EPSILON)>score_b(0.0) なので
        // PairWithChar1 になる → 正しい結果 PairWithChar2 と食い違うため検出できる。
        assert_eq!(
            judge.three_key_pairing(0, 60_000, 110_000, Some('り'), None, Some('x')),
            ThreeKeyResult::PairWithChar2
        );
    }

    #[test]
    fn three_key_pairing_tie_break_exact_equal_timing_excludes_equal() {
        // score_a=score_b=NEG_INFINITY（両方とも kana 未指定）で diff は NaN になり
        // `.abs() > EPSILON` は false（NaN との比較は常に false）→ タイブレーク `d1 < d2` に落ちる。
        // d1=d2=50_000 ちょうど。`<` は非包含なので false → PairWithChar2。
        // `==`/`<=` に変異すると true になり PairWithChar1 になってしまう → 検出できる。
        let model = sample_model();
        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ']);
        // char1_ts=0, thumb_ts=50_000, char2_ts=100_000 → d1=50_000, d2=char2_ts-thumb_ts=50_000。
        // gap=0 は margin の値に関わらず Phase1 のどちらの分岐も発火しないため、
        // margin の変異とは独立に line 125 だけを狙い撃ちできる。
        assert_eq!(
            judge.three_key_pairing(0, 50_000, 100_000, None, None, None),
            ThreeKeyResult::PairWithChar2
        );
    }

    // ── should_speculate: normal_score - thumb_score > SPECULATIVE_SCORE_THRESHOLD (line 152) ──

    #[test]
    fn should_speculate_uses_subtraction_not_division() {
        // normal_score=0.6, thumb_score=0.5 → diff=0.1、閾値0.5を超えないので speculate しない。
        // `-`→`/` に変異すると 0.6/0.5=1.2 になり閾値を超えて speculate してしまう → 検出できる。
        let toml_str = r#"
[bigram]
"あX" = 0.6
"あY" = 0.5
"#;
        let model = NgramModel::from_toml(toml_str, 20_000, 30_000, 120_000).unwrap();
        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ']);
        assert!(!judge.should_speculate(Some('X'), Some('Y'), None));
    }

    #[test]
    fn should_speculate_exact_threshold_boundary_excludes_equal() {
        // normal_score=1.0, thumb_score=0.5 → diff=0.5 ちょうど。`>` は非包含なので speculate しない。
        // `>`→`>=` に変異すると 0.5>=0.5=true になり speculate してしまう → 検出できる。
        let toml_str = r#"
[bigram]
"あX" = 1.0
"あY" = 0.5
"#;
        let model = NgramModel::from_toml(toml_str, 20_000, 30_000, 120_000).unwrap();
        let judge = TimingJudge::new(100_000, Some(&model), vec!['あ']);
        assert!(!judge.should_speculate(Some('X'), Some('Y'), None));
    }
}
