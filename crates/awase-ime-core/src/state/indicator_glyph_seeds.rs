//! ADR-261 決定 3: 字形の**共通の初期見本**（内蔵）。同じ環境の見本が集まるまでの補助。
//!
//! 出所: 実機（dragonflyg4、日本語 Windows 11・Google 日本語入力、DPI 100%、矩形 32×24）で、GJI の全 6 状態
//! （ひらがな・閉・全角カタカナ・全角英数・半角カタカナ・半角英数）を、ライトテーマとダークテーマで撮影した 12 枚
//! （2026-10-21。GJI のカスタムキー表で状態を作り、出現順で状態を決めた）。`indicator_glyph_seeds.bin` は
//! `[クラスの符号(1 バイト) | テーマ(0=ライト・1=ダーク)(1 バイト) | 32×24 のグレースケール(768 バイト)]` の繰り返し。
//! **画素は字形（インジケーターの小さな矩形）だけで、個人情報を含まない**。
//!
//! 別の DPI・テーマ・IME（MS-IME・ATOK 等）の見本は含まない。前処理が正規化するので DPI をまたいでも当たることが
//! 実測されている（ADR-261「類似度による判定の実機測定」）が、距離比の余裕が小さいので、共通の見本が最近傍のときは
//! 距離比の下限を高くする（[`super::indicator_glyph::Thresholds`] の `min_ratio_common`）。

use super::indicator_glyph::{
    preprocess, GlyphClass, GlyphGray, GlyphSample, GlyphTable, SampleOrigin,
};

const SEEDS: &[u8] = include_bytes!("indicator_glyph_seeds.bin");
const SEED_WIDTH: usize = 32;
const SEED_HEIGHT: usize = 24;
const ENTRY_LEN: usize = 2 + SEED_WIDTH * SEED_HEIGHT;

/// 内蔵データの 1 件ずつ（`[クラス|テーマ|画素]`）。
fn entries() -> impl Iterator<Item = &'static [u8]> {
    (0..SEEDS.len() / ENTRY_LEN).map(|i| &SEEDS[i * ENTRY_LEN..(i + 1) * ENTRY_LEN])
}

const fn class_from_code(code: u8) -> Option<GlyphClass> {
    match code {
        0 => Some(GlyphClass::Closed),
        1 => Some(GlyphClass::Hiragana),
        2 => Some(GlyphClass::FullKatakana),
        3 => Some(GlyphClass::FullAlnum),
        4 => Some(GlyphClass::HalfKatakana),
        5 => Some(GlyphClass::HalfAlnum),
        _ => None,
    }
}

/// 内蔵の見本を、前処理した特徴つきで返す（出所は [`SampleOrigin::Common`]）。
#[must_use]
pub fn common_samples() -> Vec<GlyphSample> {
    entries()
        .filter_map(|e| {
            let class = class_from_code(e[0])?;
            let gray = GlyphGray::new(SEED_WIDTH, SEED_HEIGHT, e[2..].to_vec())?;
            let feature = preprocess(&gray).ok()?;
            Some(GlyphSample {
                class,
                feature,
                origin: SampleOrigin::Common,
            })
        })
        .collect()
}

/// 内蔵の見本だけを持つ表。
#[must_use]
pub fn seeded_table() -> GlyphTable {
    GlyphTable::from_common_samples(common_samples())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::indicator_glyph::{
        classify_label, BeliefLabel, GlyphMiss, LabelReading, Thresholds,
    };

    /// 見本のテーマ（0=ライト・1=ダーク）ごとの `(クラス, 特徴)`。
    fn by_theme(theme: u8) -> Vec<(GlyphClass, crate::state::indicator_glyph::GlyphFeature)> {
        entries()
            .filter(|e| e[1] == theme)
            .filter_map(|e| {
                let class = class_from_code(e[0])?;
                let gray = GlyphGray::new(SEED_WIDTH, SEED_HEIGHT, e[2..].to_vec())?;
                Some((class, preprocess(&gray).ok()?))
            })
            .collect()
    }

    #[test]
    fn all_twelve_seeds_load_with_both_themes_and_every_class() {
        assert_eq!(SEEDS.len() % ENTRY_LEN, 0);
        let samples = common_samples();
        assert_eq!(samples.len(), 12);
        for class in [
            GlyphClass::Closed,
            GlyphClass::Hiragana,
            GlyphClass::FullKatakana,
            GlyphClass::FullAlnum,
            GlyphClass::HalfKatakana,
            GlyphClass::HalfAlnum,
        ] {
            assert_eq!(
                samples.iter().filter(|s| s.class == class).count(),
                2,
                "{class:?}"
            );
        }
        assert!(samples.iter().all(|s| s.origin == SampleOrigin::Common));
        assert_eq!(seeded_table().len(), 12);
    }

    #[test]
    fn every_seed_classifies_to_its_own_label_in_the_seeded_table() {
        let table = seeded_table();
        for theme in [0, 1] {
            for (class, feature) in by_theme(theme) {
                match classify_label(&table, &Thresholds::default(), &feature) {
                    LabelReading::Hit(h) => {
                        assert_eq!(h.label, BeliefLabel::from_class(class), "{class:?}")
                    }
                    LabelReading::Miss(m) => panic!("{class:?} theme={theme}: Miss({m:?})"),
                }
            }
        }
    }

    /// 実データの越境テスト: ダークの見本だけの表でライトの見本を読む（逆も）。
    ///
    /// 実測（2026-10-21、dragonflyg4）: かな系（ひらがな・全角カタカナ・半角カタカナ）と半角英数は、距離比 3.4〜14.6 の余裕で
    /// 正しいラベルに当たる。**閉（細い「A」）と全角英数（幅のある「A」）は近く**（ADR-261 が予想した最も近い組）、`Ambiguous`
    /// （Miss）に倒れる。どの場合も**誤ったラベルは返さない**（ADR-261 決定 0）。
    #[test]
    fn cross_theme_hits_the_kana_classes_and_never_returns_a_wrong_label() {
        for (table_theme, probe_theme) in [(1u8, 0u8), (0, 1)] {
            let samples: Vec<GlyphSample> = by_theme(table_theme)
                .into_iter()
                .map(|(class, feature)| GlyphSample {
                    class,
                    feature,
                    origin: SampleOrigin::Common,
                })
                .collect();
            let table = GlyphTable::from_common_samples(samples);
            for (class, feature) in by_theme(probe_theme) {
                let reading = classify_label(&table, &Thresholds::default(), &feature);
                match (class, reading) {
                    // 確かに当たるもの。
                    (
                        GlyphClass::Hiragana
                        | GlyphClass::FullKatakana
                        | GlyphClass::HalfKatakana
                        | GlyphClass::HalfAlnum,
                        LabelReading::Hit(h),
                    ) => assert_eq!(h.label, BeliefLabel::from_class(class), "{class:?} → {h:?}"),
                    // 閉と全角英数は近いので、Miss（曖昧）に倒れるか、当たるなら正しいラベル。
                    (GlyphClass::Closed | GlyphClass::FullAlnum, LabelReading::Miss(m)) => {
                        assert_eq!(m, GlyphMiss::Ambiguous, "{class:?}");
                    }
                    (GlyphClass::Closed | GlyphClass::FullAlnum, LabelReading::Hit(h)) => {
                        assert_eq!(h.label, BeliefLabel::from_class(class), "{class:?} → {h:?}");
                    }
                    (c, LabelReading::Miss(m)) => panic!("{c:?} theme={probe_theme}: Miss({m:?})"),
                }
            }
        }
    }
}
