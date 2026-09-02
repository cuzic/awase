//! ADR-120 決定0a: 3キー仲裁（`three_key_pairing`）の判定過程を、
//! 実際の変換結果には一切影響しない形で観測するための集計カウンタ。
//!
//! 新しいスコア関数・適格判定・訂正出力は一切追加しない（決定0aの
//! 「新設計ゼロ」原則）。ここに集まる値は起動からの累積カウンタであり、
//! `retro_ngram_correction` 設定の有効/無効とは独立に常時集計する。

/// 桁スケールの粗いバケット境界（ms）。粒度を細かくしないこと——個人の打鍵
/// ダイナミクス（バイオメトリクス的特徴）に近づけないため、分布形状が分かる
/// 最小限の解像度に留める（ADR-120決定0a-report参照）。
pub const ELAPSED_MS_BUCKETS: [u64; 7] = [50, 100, 200, 400, 800, 1600, u64::MAX];

/// 「直近の決定」を訂正の原因として妥当とみなす最大経過ms。これを超えて
/// stale になった決定は、以後の訂正の分子には計上しない（デノミネータには
/// 決定時点で既に計上済み）。バケット最大値と同じにする。
pub const STALE_ATTRIBUTION_MS: u64 = 1600;

/// ひらがな範囲（U+3041-U+309F）かどうか。ADR-120 決定0a 項目2b専用の粗い判定
/// （拗音・濁点等の厳密な分類は不要、集計目的のため範囲判定で十分）。
#[must_use]
pub const fn is_hiragana(c: char) -> bool {
    ('\u{3041}' <= c) && (c <= '\u{309F}')
}

#[must_use]
pub fn bucket_index(elapsed_ms: u64) -> usize {
    ELAPSED_MS_BUCKETS
        .iter()
        .position(|&bound| elapsed_ms < bound)
        .unwrap_or(ELAPSED_MS_BUCKETS.len() - 1)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetroEvalStats {
    // 項目1: 3値分類（Phase1/Phase2/NoNgramの3群）
    pub three_key_total: u64,
    pub phase2_reached: u64,
    pub phase1_reached: u64,
    pub no_ngram_count: u64,

    // 項目2: score_a/score_b は独立の2値なので別々に3値カウントする
    pub score_a_neg_infinity_count: u64,
    pub score_a_zero_count: u64,
    pub score_a_finite_count: u64,
    pub score_b_neg_infinity_count: u64,
    pub score_b_zero_count: u64,
    pub score_b_finite_count: u64,

    // 項目2b: Phase2到達かつchar2のNormal面かながSome+ひらがな だった回数
    pub char2_normal_hiragana_count: u64,
    // 項目2c: Phase2決定直後の連続2打鍵(k=1の窓)に親指KeyDownが1つも無かった回数。
    // 分母は phase2_reached ではなく、以下の3カウンタの和にすること
    // （nice-to-have所見N2対応——打鍵を止めた場合は過小に、Passthroughキー
    // 混入では過大に、2方向のバイアスが同時にかかるため、`phase2_reached`
    // をそのまま分母にすると原因を事後に分離できない）。
    pub no_thumb_followup_count: u64,
    // 項目2c分母: 窓の途中で親指KeyDownが来て破棄された回数。
    pub thumb_watch_window_thumb_arrived_count: u64,
    // 項目2c分母: 窓が未消化のまま次のPhase2決定に上書きされて破棄された回数。
    pub thumb_watch_window_abandoned_count: u64,

    // 項目4: Phase2決定「自身の出力」をスキップした後の、後続1かな確定までの
    // 経過msヒストグラム（分母は phase2_reached）。update_history_imprecise
    // 経由（タイムアウト/flush等、正確な発生時刻が無い経路）で完了した計測は
    // 記録しない（所見B2対応）。
    pub followup_elapsed_ms_histogram: [u64; 7],
    // 項目4 (nice-to-have所見N1対応): 後続1かな確定を待っている間に次の
    // Phase2決定が来て上書きされ、計測が完了しないまま失われた回数。
    pub followup_overwritten_count: u64,
    // 項目4 (nice-to-have所見NN2対応): update_history_imprecise 経由
    // （タイムアウト/flush等、正確な発生時刻が無い経路）で「後続1かな確定」
    // 自体は起きたが、所見B2対応によりヒストグラムへの記録を意図的に
    // 見送った回数。以下の残差がゼロになることを保証する:
    // `phase2_reached == followup_elapsed_ms_histogram.sum()
    //                  + followup_overwritten_count
    //                  + followup_dropped_imprecise_count
    //                  + (計測未完了で残っている分)`
    pub followup_dropped_imprecise_count: u64,

    // 項目7: 3群 x (分母スカラー + 訂正発生時の経過msヒストグラム)
    // 分母はphase2_reached/phase1_reachedを再利用せず独立カウンタにする
    // （「対照群デノミネータ」はitem7固有の除外条件を持つため、item1の
    // カウンタと値が一致するとは限らない）
    pub phase2_decisions_total: u64,
    pub phase2_correction_histogram: [u64; 7],
    pub phase1_decisions_total: u64,
    pub phase1_correction_histogram: [u64; 7],
    pub baseline_decisions_total: u64,
    pub baseline_correction_histogram: [u64; 7],

    // 項目7(c): Escape出力回数（訂正カウントとは別、離脱と区別するため）
    pub escape_output_count: u64,
}

#[cfg(test)]
mod tests {
    use super::{bucket_index, ELAPSED_MS_BUCKETS};

    #[test]
    fn bucket_index_boundaries() {
        // 各境界値の直前は前のバケット、直後(境界値そのもの)は次のバケット。
        for (i, &bound) in ELAPSED_MS_BUCKETS.iter().enumerate() {
            if bound == u64::MAX {
                // 最終バケット: 直前の境界より大きい値でも最終バケットのまま。
                assert_eq!(bucket_index(bound - 1), i);
                continue;
            }
            assert_eq!(bucket_index(bound - 1), i, "just below bound {bound}");
            assert_eq!(bucket_index(bound), i + 1, "at bound {bound}");
        }
    }

    #[test]
    fn bucket_index_zero() {
        assert_eq!(bucket_index(0), 0);
    }
}
