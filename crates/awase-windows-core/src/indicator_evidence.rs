//! 入力インジケーターの証拠源（字形・右クリックメニュー）の画面で選べる上限と実効の段階（ADR-271 D3）。
//!
//! 設定ファイルの値（`awase::config::EvidenceLevel`）は丸めずに保存し、**使う時点で**
//! `min(configured, ui_max_level())` にする。`ui_max_level()` を上げるのは、belief への配線 ADR の
//! 入口条件（Medium が High を追い越さない。ADR-233）を満たした PR だけ。

use awase::config::{EvidenceLevel, IndicatorEvidenceConfig};

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
}
