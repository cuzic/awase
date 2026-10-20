//! 入力インジケーターの証拠源（字形・右クリックメニュー）の画面で選べる上限と実効の段階（ADR-271 D3）。
//!
//! 設定ファイルの値（`awase::config::EvidenceLevel`）は丸めずに保存し、**使う時点で**
//! `min(configured, ui_max_level())` にする。`ui_max_level()` を上げるのは、belief への配線 ADR の
//! 入口条件（Medium が High を追い越さない。ADR-233）を満たした PR だけ。

use std::collections::BTreeMap;

use awase::config::{EvidenceLevel, IndicatorEvidenceConfig};

use crate::journal::IndicatorMenuRecord;

/// 証拠源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceSource {
    /// タスクバーのインジケーターの字形（ADR-261）。
    Glyph,
    /// インジケーターの右クリックメニュー（ADR-264）。
    Menu,
}

impl EvidenceSource {
    /// この版の実装が対応する最大の段階（設定画面で選べる上限）。
    ///
    /// メニューは journal に残す観測（`Observe`）まで。字形はコードが無いので `Off`。
    #[must_use]
    pub const fn ui_max_level(self) -> EvidenceLevel {
        match self {
            Self::Glyph => EvidenceLevel::Off,
            Self::Menu => EvidenceLevel::Observe,
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
    pub menu: EvidenceLevel,
}

impl EffectiveEvidence {
    #[must_use]
    pub fn from_config(cfg: &IndicatorEvidenceConfig) -> Self {
        Self {
            glyph: EvidenceSource::Glyph.effective(cfg.glyph),
            menu: EvidenceSource::Menu.effective(cfg.menu),
        }
    }
}

/// 右クリックメニューの読みの件数（不具合報告の要約用。内容は含まない。ADR-271 D5）。
///
/// 一致/不一致は、読んだモードの開閉（`open`）とメニューを開いた時点の belief（`belief_open`）の比較。
/// 読める窓（`standard`）では belief は直接の読みに追随しているので「直接の読みとの一致」、観測の無い窓
/// （`imm32-unavailable`）では「belief とのずれ」で、意味が違うので別々に数える。どちらかが不明（`None`）なら数えない。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndicatorMenuCounters {
    /// 読み取りの結果を受けた回数
    pub reads: u32,
    /// 同定できた（`hit`）回数
    pub hits: u32,
    /// 同定できなかった理由ごとの回数
    pub miss_reasons: BTreeMap<String, u32>,
    pub direct_match: u32,
    pub direct_mismatch: u32,
    pub belief_match: u32,
    pub belief_mismatch: u32,
}

impl IndicatorMenuCounters {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            reads: 0,
            hits: 0,
            miss_reasons: BTreeMap::new(),
            direct_match: 0,
            direct_mismatch: 0,
            belief_match: 0,
            belief_mismatch: 0,
        }
    }

    /// journal に残す記録 1 件を数える。
    pub fn record(&mut self, r: &IndicatorMenuRecord) {
        self.reads = self.reads.saturating_add(1);
        if r.verdict == "hit" {
            self.hits = self.hits.saturating_add(1);
        } else {
            let reason = r.miss_reason.unwrap_or("unknown");
            let n = self.miss_reasons.entry(reason.to_owned()).or_insert(0);
            *n = n.saturating_add(1);
        }
        let (Some(open), Some(belief)) = (r.open, r.belief_open) else {
            return;
        };
        let agree = open == belief;
        let slot = match r.profile {
            "standard" => {
                if agree {
                    &mut self.direct_match
                } else {
                    &mut self.direct_mismatch
                }
            }
            "imm32-unavailable" => {
                if agree {
                    &mut self.belief_match
                } else {
                    &mut self.belief_mismatch
                }
            }
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
            menu: EvidenceLevel::All,
        };
        let e = EffectiveEvidence::from_config(&cfg);
        assert_eq!(e.menu, EvidenceLevel::Observe);
        assert_eq!(e.glyph, EvidenceLevel::Off);
    }

    #[test]
    fn off_stays_off_and_observe_is_kept_for_menu() {
        assert_eq!(
            EvidenceSource::Menu.effective(EvidenceLevel::Off),
            EvidenceLevel::Off
        );
        assert_eq!(
            EvidenceSource::Menu.effective(EvidenceLevel::Observe),
            EvidenceLevel::Observe
        );
    }

    #[test]
    fn default_config_is_all_off() {
        let e = EffectiveEvidence::from_config(&IndicatorEvidenceConfig::default());
        assert_eq!((e.glyph, e.menu), (EvidenceLevel::Off, EvidenceLevel::Off));
    }

    fn rec(
        verdict: &'static str,
        open: Option<bool>,
        profile: &'static str,
        belief_open: Option<bool>,
        miss_reason: Option<&'static str>,
    ) -> IndicatorMenuRecord {
        IndicatorMenuRecord {
            ime: "gji",
            verdict,
            mode: None,
            open,
            evidence: None,
            miss_reason,
            item_count: 0,
            read_ms: 0,
            profile,
            belief_open,
        }
    }

    #[test]
    fn counters_separate_direct_agreement_from_belief_drift() {
        let mut c = IndicatorMenuCounters::default();
        c.record(&rec("hit", Some(true), "standard", Some(true), None));
        c.record(&rec("hit", Some(true), "standard", Some(false), None));
        c.record(&rec(
            "hit",
            Some(false),
            "imm32-unavailable",
            Some(false),
            None,
        ));
        c.record(&rec(
            "hit",
            Some(false),
            "imm32-unavailable",
            Some(true),
            None,
        ));
        // 開閉が不明、belief が不明、その他のプロファイルは一致/不一致に数えない。
        c.record(&rec("hit", None, "standard", Some(true), None));
        c.record(&rec("hit", Some(true), "standard", None, None));
        c.record(&rec("hit", Some(true), "tsf-native", Some(true), None));
        assert_eq!((c.reads, c.hits), (7, 7));
        assert_eq!((c.direct_match, c.direct_mismatch), (1, 1));
        assert_eq!((c.belief_match, c.belief_mismatch), (1, 1));
    }

    #[test]
    fn counters_count_miss_reasons_without_content() {
        let mut c = IndicatorMenuCounters::default();
        c.record(&rec(
            "miss",
            None,
            "standard",
            Some(true),
            Some("no-selection"),
        ));
        c.record(&rec(
            "miss",
            None,
            "standard",
            Some(true),
            Some("no-selection"),
        ));
        c.record(&rec("miss", None, "standard", None, Some("empty-menu")));
        assert_eq!((c.reads, c.hits), (3, 0));
        assert_eq!(c.miss_reasons.get("no-selection"), Some(&2));
        assert_eq!(c.miss_reasons.get("empty-menu"), Some(&1));
    }
}
