//! 入力インジケーターの証拠源（字形）の画面で選べる上限と実効の段階（ADR-271 D3）。
//!
//! 右クリックメニュー（ADR-264）は実機で信頼できず撤去した（2026-10-22）。
//!
//! 設定ファイルの値（`awase::config::EvidenceLevel`）は丸めずに保存し、**使う時点で**
//! `min(configured, ui_max_level())` にする。`ui_max_level()` を上げるのは、belief への配線 ADR の
//! 入口条件（Medium が High を追い越さない。ADR-233）を満たした PR だけ。

use std::collections::BTreeMap;

use awase::config::{EvidenceLevel, IndicatorEvidenceConfig};

/// 証拠源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceSource {
    /// タスクバーのインジケーターの字形（ADR-261）。
    Glyph,
}

impl EvidenceSource {
    /// この版の実装が対応する最大の段階（設定画面で選べる上限）。
    ///
    /// journal に残す観測（`Observe`）まで。belief への配線（`Unobserved`/`All`）は別 ADR（ADR-273）。
    #[must_use]
    pub const fn ui_max_level(self) -> EvidenceLevel {
        match self {
            Self::Glyph => EvidenceLevel::Observe,
        }
    }

    /// 設定値を `ui_max_level()` で丸めた実効の段階。
    #[must_use]
    pub fn effective(self, configured: EvidenceLevel) -> EvidenceLevel {
        configured.min(self.ui_max_level())
    }
}

/// 起動時に 1 度だけ計算して固定する、証拠源ごとの実効の段階。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveEvidence {
    pub glyph: EvidenceLevel,
    /// ADR-283 §10: T2〜T4 共通の UIA push トリガーを使うか。`glyph` が `Off` なら（ワーカー自体が起動しないので）
    /// 常に意味を持たない `false` に落とす。
    pub glyph_push_trigger: bool,
}

impl EffectiveEvidence {
    #[must_use]
    pub fn from_config(cfg: &IndicatorEvidenceConfig) -> Self {
        let glyph = EvidenceSource::Glyph.effective(cfg.glyph);
        Self {
            glyph,
            glyph_push_trigger: glyph != EvidenceLevel::Off && cfg.glyph_push_trigger,
        }
    }
}

/// 開閉の軸の一致/不一致の件数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgreeCount {
    pub agree: u32,
    pub disagree: u32,
}

/// 字形の取り込みの件数（不具合報告の要約用。画像・画素は含まない。ADR-271 D5）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndicatorGlyphCounters {
    /// 取り込みの結果を受けた回数
    pub reads: u32,
    /// 前処理まで通った（`captured`）回数
    pub captured: u32,
    /// 取り込めなかった理由ごとの回数
    pub miss_reasons: BTreeMap<String, u32>,
    /// 表に照合して採った(`Hit`)回数と、`Miss` の理由ごとの回数
    pub matched: u32,
    pub match_misses: BTreeMap<String, u32>,
    /// 読める窓（`standard`）で、照合の読みが belief（直接の読みに追随）と一致した/しなかった回数
    pub direct_agree: u32,
    pub direct_disagree: u32,
    /// 観測の無い窓（`imm32-unavailable`）で、照合の読みが belief と一致した/しなかった回数（意味が違うので分けて数える）
    pub unobserved_agree: u32,
    pub unobserved_disagree: u32,
    /// 見本として表に足した回数
    pub collected: u32,
    /// 開閉の軸の一致/不一致を、窓の IME プロファイル（`standard`/`imm32-unavailable`/`tsf-native`/`input-relay`）ごとに。
    /// 入力モードが不明でも数えられる。`standard` は直接の読みとの一致、それ以外は belief とのずれ
    pub open_axis: BTreeMap<String, AgreeCount>,
    /// 対象の窓だったとしての段階ごとの回数（ADR-273 D3・Q2、段階 2 の前提。`unobserved`/`all`/`none`）
    pub would_scope: BTreeMap<String, u32>,
    /// 柵を通って報告されたかの結果ごとの回数（`reported`/`miss-changed`/`ignored`/`mozc-window`/`fenced:<理由>`）
    pub would_report: BTreeMap<String, u32>,
    /// `would_report` が `reported` のとき、字形が belief を決めた/決めてかつ値が違った（flip した）回数
    /// （ADR-273 D2。実際には書かない試算）
    pub would_decide: u32,
    pub would_flip: u32,
    /// ADR-283 G2: 候補 Z の試算に使った/かつ Z の値を変えうった（flip した）回数（実際には書かない試算）
    pub would_decide_z: u32,
    pub would_flip_z: u32,
}

impl IndicatorGlyphCounters {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            reads: 0,
            captured: 0,
            miss_reasons: BTreeMap::new(),
            matched: 0,
            match_misses: BTreeMap::new(),
            direct_agree: 0,
            direct_disagree: 0,
            unobserved_agree: 0,
            unobserved_disagree: 0,
            collected: 0,
            open_axis: BTreeMap::new(),
            would_scope: BTreeMap::new(),
            would_report: BTreeMap::new(),
            would_decide: 0,
            would_flip: 0,
            would_decide_z: 0,
            would_flip_z: 0,
        }
    }

    /// journal に残す記録 1 件を数える。
    pub fn record(&mut self, r: &crate::journal::IndicatorGlyphRecord) {
        self.reads = self.reads.saturating_add(1);
        if r.verdict == "captured" {
            self.captured = self.captured.saturating_add(1);
        } else {
            let reason = r.miss_reason.unwrap_or("unknown");
            let n = self.miss_reasons.entry(reason.to_owned()).or_insert(0);
            *n = n.saturating_add(1);
        }
        if r.reading.is_some() {
            self.matched = self.matched.saturating_add(1);
        }
        if let Some(reason) = r.match_miss {
            let n = self.match_misses.entry(reason.to_owned()).or_insert(0);
            *n = n.saturating_add(1);
        }
        if r.collected {
            self.collected = self.collected.saturating_add(1);
        }
        if let Some(scope) = r.would_scope {
            let n = self.would_scope.entry(scope.to_owned()).or_insert(0);
            *n = n.saturating_add(1);
        }
        if let Some(report) = r.would_report {
            let n = self.would_report.entry(report.to_owned()).or_insert(0);
            *n = n.saturating_add(1);
        }
        if r.would_decide == Some(true) {
            self.would_decide = self.would_decide.saturating_add(1);
        }
        if r.would_flip == Some(true) {
            self.would_flip = self.would_flip.saturating_add(1);
        }
        if r.would_decide_z == Some(true) {
            self.would_decide_z = self.would_decide_z.saturating_add(1);
        }
        if r.would_flip_z == Some(true) {
            self.would_flip_z = self.would_flip_z.saturating_add(1);
        }
        if let Some(agree_open) = r.agree_open {
            let c = self.open_axis.entry(r.profile.to_owned()).or_default();
            let slot = if agree_open {
                &mut c.agree
            } else {
                &mut c.disagree
            };
            *slot = slot.saturating_add(1);
        }
        let Some(agree) = r.agree else {
            return;
        };
        let slot = match (r.profile, agree) {
            ("standard", true) => &mut self.direct_agree,
            ("standard", false) => &mut self.direct_disagree,
            ("imm32-unavailable", true) => &mut self.unobserved_agree,
            ("imm32-unavailable", false) => &mut self.unobserved_disagree,
            _ => return,
        };
        *slot = slot.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_is_min_of_configured_and_ui_max() {
        let cfg = IndicatorEvidenceConfig {
            glyph: EvidenceLevel::All,
            ..IndicatorEvidenceConfig::default()
        };
        let e = EffectiveEvidence::from_config(&cfg);
        assert_eq!(e.glyph, EvidenceLevel::Observe);
    }

    #[test]
    fn push_trigger_is_false_when_glyph_is_off_even_if_configured_true() {
        let cfg = IndicatorEvidenceConfig {
            glyph: EvidenceLevel::Off,
            glyph_push_trigger: true,
        };
        let e = EffectiveEvidence::from_config(&cfg);
        assert!(!e.glyph_push_trigger);
    }

    #[test]
    fn push_trigger_follows_config_when_glyph_is_observe_or_above() {
        let cfg = IndicatorEvidenceConfig {
            glyph: EvidenceLevel::Observe,
            glyph_push_trigger: true,
        };
        let e = EffectiveEvidence::from_config(&cfg);
        assert!(e.glyph_push_trigger);
    }

    #[test]
    fn off_stays_off_and_observe_is_kept_for_glyph() {
        assert_eq!(
            EvidenceSource::Glyph.effective(EvidenceLevel::Off),
            EvidenceLevel::Off
        );
        assert_eq!(
            EvidenceSource::Glyph.effective(EvidenceLevel::Observe),
            EvidenceLevel::Observe
        );
    }

    #[test]
    fn default_config_is_all_off() {
        let e = EffectiveEvidence::from_config(&IndicatorEvidenceConfig::default());
        assert_eq!(e.glyph, EvidenceLevel::Off);
    }

    #[test]
    fn glyph_counters_count_captures_and_miss_reasons_without_pixels() {
        let rec = |verdict, miss_reason| crate::journal::IndicatorGlyphRecord {
            verdict,
            miss_reason,
            width: 32,
            height: 24,
            dots: 100,
            read_ms: 50,
            delay_ms: 900,
            profile: "standard",
            belief_open: None,
            ..Default::default()
        };
        let mut c = IndicatorGlyphCounters::default();
        c.record(&rec("captured", None));
        c.record(&rec("miss", Some("empty-glyph")));
        c.record(&rec("miss", Some("empty-glyph")));
        c.record(&rec("miss", Some("no-indicator")));
        assert_eq!((c.reads, c.captured), (4, 1));
        assert_eq!(c.miss_reasons.get("empty-glyph"), Some(&2));
        assert_eq!(c.miss_reasons.get("no-indicator"), Some(&1));
    }
    #[test]
    fn glyph_counters_separate_direct_agreement_from_belief_drift() {
        let rec = |profile, agree, collected, reading| crate::journal::IndicatorGlyphRecord {
            verdict: "captured",
            profile,
            agree,
            collected,
            reading,
            ..Default::default()
        };
        let mut c = IndicatorGlyphCounters::default();
        c.record(&rec("standard", Some(true), true, Some("kana")));
        c.record(&rec("standard", Some(false), false, Some("eisu")));
        c.record(&rec("imm32-unavailable", Some(true), false, Some("closed")));
        c.record(&rec("imm32-unavailable", Some(false), false, Some("kana")));
        // 一致が不明・その他のプロファイルは一致/不一致に数えない。
        c.record(&rec("standard", None, false, None));
        c.record(&rec("tsf-native", Some(true), false, Some("kana")));
        assert_eq!((c.direct_agree, c.direct_disagree), (1, 1));
        assert_eq!((c.unobserved_agree, c.unobserved_disagree), (1, 1));
        assert_eq!((c.matched, c.collected), (5, 1));
        let mut m = crate::journal::IndicatorGlyphRecord::default();
        m.match_miss = Some("empty-table");
        c.record(&m);
        assert_eq!(c.match_misses.get("empty-table"), Some(&1));
    }

    #[test]
    fn glyph_counters_count_the_open_axis_per_profile_even_without_a_belief_mode() {
        let rec = |profile, agree_open| crate::journal::IndicatorGlyphRecord {
            verdict: "captured",
            profile,
            agree_open,
            // 入力モードのラベルは不明（`agree` は None）。
            agree: None,
            ..Default::default()
        };
        let mut c = IndicatorGlyphCounters::default();
        c.record(&rec("tsf-native", Some(false)));
        c.record(&rec("tsf-native", Some(false)));
        c.record(&rec("tsf-native", Some(true)));
        c.record(&rec("standard", Some(true)));
        c.record(&rec("imm32-unavailable", None));
        let tsf = c.open_axis.get("tsf-native").copied().unwrap_or_default();
        assert_eq!((tsf.agree, tsf.disagree), (1, 2));
        assert_eq!(
            c.open_axis.get("standard").map(|a| (a.agree, a.disagree)),
            Some((1, 0))
        );
        assert!(!c.open_axis.contains_key("imm32-unavailable"));
        // 入力モードのラベルの一致は数えていない。
        assert_eq!((c.direct_agree, c.direct_disagree), (0, 0));
    }

    #[test]
    fn glyph_counters_count_would_scope_would_report_and_would_decide_or_flip() {
        let rec =
            |would_scope, would_report, would_decide, would_flip, would_decide_z, would_flip_z| {
                crate::journal::IndicatorGlyphRecord {
                    verdict: "captured",
                    would_scope,
                    would_report,
                    would_decide,
                    would_flip,
                    would_decide_z,
                    would_flip_z,
                    ..Default::default()
                }
            };
        let mut c = IndicatorGlyphCounters::default();
        // 段階2(ADR-273 D2)の試算: 決めた(flipを伴う)・決めた(flipなし)・対象外の3件。
        c.record(&rec(
            Some("unobserved"),
            Some("reported"),
            Some(true),
            Some(true),
            None,
            None,
        ));
        c.record(&rec(
            Some("unobserved"),
            Some("reported"),
            Some(true),
            Some(false),
            None,
            None,
        ));
        c.record(&rec(Some("none"), None, None, None, None, None));
        // 候補Z(ADR-283 G2)の試算: flipを伴う1件。
        c.record(&rec(
            Some("all"),
            Some("reported"),
            None,
            None,
            Some(true),
            Some(true),
        ));
        assert_eq!(c.would_scope.get("unobserved"), Some(&2));
        assert_eq!(c.would_scope.get("none"), Some(&1));
        assert_eq!(c.would_scope.get("all"), Some(&1));
        assert_eq!(c.would_report.get("reported"), Some(&3));
        assert_eq!((c.would_decide, c.would_flip), (2, 1));
        assert_eq!((c.would_decide_z, c.would_flip_z), (1, 1));
    }
}
