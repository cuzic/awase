//! ADR-192決定2: 状態依存IMEモードキーの警告内容と一度きり判定。

use awase::types::VkCode;

use super::key_effect_predictor::KeyEffectKeymap;
use super::key_effect_table::{
    classify_state_dependent_mode_key, CannotPredictReason, Classification, StateDependentAxis,
};

const TARGET_VKS: [u16; 7] = [0x1C, 0x1D, 0xF3, 0xF4, 0x19, 0x16, 0x1A];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    OpenAxis,
    Composition,
    UserOverride,
    ThumbConflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeKeyWarning {
    pub kind: WarningKind,
    pub keys: Vec<VkCode>,
    pub message: String,
}

impl ModeKeyWarning {
    fn new(kind: WarningKind, keys: Vec<VkCode>) -> Self {
        let message = match kind {
            WarningKind::OpenAxis => "IMEの状態によってキーの結果が変わるため、awaseとIMEのモードがずれる可能性があります。冪等なIME ON/OFFキーへの変更を推奨します。",
            WarningKind::Composition => "入力中に押すと、変換中の文字が消える、または確定してしまう場合があります。冪等なキーでも起こりうるため、ON/OFFキーへの置き換えだけでは解決しません。",
            WarningKind::UserOverride => "ユーザー固有のIMEキー割り当てがあるため、awaseはこのキーの効果を追随できない可能性があります。",
            WarningKind::ThumbConflict => "IME側の割り当てを解除し、awaseの親指単独タップ設定またはbare親指キーのIME ON/OFF設定に委ねてください。",
        }
        .to_owned();
        Self {
            kind,
            keys,
            message,
        }
    }
}

/// 警告の同一性をプロセス内で保持する。予測器のキャッシュ状態は変更しない。
#[derive(Debug, Default)]
pub struct WarningTracker {
    last_gji_stamp: Option<(u64, u64)>,
    last_msime_bits: Option<u8>,
    last_composition_keys: Vec<VkCode>,
}

impl WarningTracker {
    #[must_use]
    pub fn detect_gji(
        &mut self,
        enabled: bool,
        stamp: Option<(u64, u64)>,
        keymap: Option<&KeyEffectKeymap>,
        thumb_keys: [VkCode; 2],
    ) -> Vec<ModeKeyWarning> {
        if !enabled {
            return Vec::new();
        }
        let warnings = detect(keymap, thumb_keys);
        self.deduplicate(warnings, stamp, None)
    }

    #[must_use]
    pub fn detect_msime(
        &mut self,
        enabled: bool,
        packed_assignment_bits: u8,
        keymap: Option<&KeyEffectKeymap>,
        thumb_keys: [VkCode; 2],
    ) -> Vec<ModeKeyWarning> {
        if !enabled {
            return Vec::new();
        }
        let warnings = detect(keymap, thumb_keys);
        self.deduplicate(warnings, None, Some(packed_assignment_bits))
    }

    fn deduplicate(
        &mut self,
        warnings: Vec<ModeKeyWarning>,
        gji_stamp: Option<(u64, u64)>,
        msime_bits: Option<u8>,
    ) -> Vec<ModeKeyWarning> {
        let same_source = if let Some(stamp) = gji_stamp {
            self.last_gji_stamp.replace(stamp) == Some(stamp)
        } else if let Some(bits) = msime_bits {
            self.last_msime_bits.replace(bits) == Some(bits)
        } else {
            false
        };
        let composition_keys = warnings
            .iter()
            .find(|warning| warning.kind == WarningKind::Composition)
            .map_or_else(Vec::new, |warning| warning.keys.clone());
        let same_composition = self.last_composition_keys == composition_keys;
        self.last_composition_keys = composition_keys;

        warnings
            .into_iter()
            .filter(|warning| match warning.kind {
                WarningKind::Composition => !same_composition && !warning.keys.is_empty(),
                _ => !same_source,
            })
            .collect()
    }
}

#[must_use]
pub fn detect(keymap: Option<&KeyEffectKeymap>, thumb_keys: [VkCode; 2]) -> Vec<ModeKeyWarning> {
    let mut open = Vec::new();
    let mut composition = Vec::new();
    let mut overrides = Vec::new();
    let mut thumbs = Vec::new();

    for vk in TARGET_VKS {
        let code = VkCode(vk);
        let is_thumb = thumb_keys.contains(&code);
        match classify_state_dependent_mode_key(keymap, vk) {
            Some(Classification::StateDependent(axis)) => {
                if is_thumb {
                    thumbs.push(code);
                } else {
                    if matches!(
                        axis,
                        StateDependentAxis::Open | StateDependentAxis::OpenAndComposition
                    ) {
                        open.push(code);
                    }
                    if matches!(
                        axis,
                        StateDependentAxis::Composition | StateDependentAxis::OpenAndComposition
                    ) {
                        composition.push(code);
                    }
                }
            }
            Some(Classification::CannotPredict(CannotPredictReason::UserOverride)) if !is_thumb => {
                overrides.push(code);
            }
            _ => {}
        }
    }

    [
        (WarningKind::OpenAxis, open),
        (WarningKind::Composition, composition),
        (WarningKind::UserOverride, overrides),
        (WarningKind::ThumbConflict, thumbs),
    ]
    .into_iter()
    .filter(|(_, keys)| !keys.is_empty())
    .map(|(kind, keys)| ModeKeyWarning::new(kind, keys))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atok() -> KeyEffectKeymap {
        KeyEffectKeymap::from_config(Some(1), None, &[]).unwrap()
    }

    #[test]
    fn warning_wording_is_split_by_category() {
        let warnings = detect(Some(&atok()), [VkCode(0), VkCode(0)]);
        assert!(warnings
            .iter()
            .any(|w| w.kind == WarningKind::OpenAxis && w.message.contains("モードがずれる")));
        assert!(warnings.iter().any(|w| w.kind == WarningKind::Composition
            && w.message.contains("置き換えだけでは解決しません")));
    }

    #[test]
    fn thumb_keys_are_routed_to_existing_conflict_style_warning() {
        let warnings = detect(Some(&atok()), [VkCode(0x1C), VkCode(0x1D)]);
        assert!(!warnings.iter().any(|w| w.kind == WarningKind::OpenAxis));
        assert!(warnings
            .iter()
            .any(|w| w.kind == WarningKind::ThumbConflict));
    }

    #[test]
    fn user_override_warns_but_ambiguous_and_insufficient_stay_silent() {
        let custom =
            KeyEffectKeymap::from_config(Some(2), Some("DirectInput\tHenkan\tIMEOn".into()), &[])
                .unwrap();
        assert!(detect(Some(&custom), [VkCode(0), VkCode(0)])
            .iter()
            .any(|w| w.kind == WarningKind::UserOverride));
        let native = KeyEffectKeymap::for_msime_native(false, None, None);
        assert!(detect(Some(&native), [VkCode(0), VkCode(0)]).is_empty());
        assert!(detect(None, [VkCode(0), VkCode(0)]).is_empty());
    }

    #[test]
    fn warnings_are_once_per_source_but_repeat_after_identity_change() {
        let mut tracker = WarningTracker::default();
        let thumbs = [VkCode(0), VkCode(0)];
        assert!(!tracker
            .detect_gji(true, Some((1, 1)), Some(&atok()), thumbs)
            .is_empty());
        assert!(tracker
            .detect_gji(true, Some((1, 1)), Some(&atok()), thumbs)
            .is_empty());
        assert!(!tracker
            .detect_gji(true, Some((2, 1)), Some(&atok()), thumbs)
            .is_empty());
        assert!(tracker
            .detect_gji(false, Some((3, 1)), Some(&atok()), thumbs)
            .is_empty());
    }

    #[test]
    fn composition_identity_is_the_actual_notified_key_set() {
        let mut tracker = WarningTracker::default();
        let first = tracker.detect_gji(true, Some((1, 1)), Some(&atok()), [VkCode(0), VkCode(0)]);
        assert!(first.iter().any(|w| w.kind == WarningKind::Composition));
        let changed =
            tracker.detect_gji(true, Some((2, 1)), Some(&atok()), [VkCode(0x19), VkCode(0)]);
        assert!(changed.iter().any(|w| w.kind == WarningKind::Composition));
    }
}
