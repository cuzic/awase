//! ADR-192決定2: 状態依存IMEモードキーの警告内容と一度きり判定。

use awase::types::VkCode;

use super::key_effect_predictor::{Cell, KeyEffectKeymap};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningDialogAction {
    OpenAwaseSettings,
    OpenMsImeSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarningDialogRequest {
    pub warning: ModeKeyWarning,
    pub action: WarningDialogAction,
}

/// ADR-192決定2bのユーザー向けダイアログを、同一内容につきプロセス内で一度だけにする。
/// ログ用の[`WarningTracker`]とは責務を分け、そちらのデデュープ挙動を変えない。
#[derive(Debug, Default)]
pub struct WarningDialogTracker {
    shown: Vec<(WarningKind, Vec<VkCode>, WarningDialogAction)>,
}

impl WarningDialogTracker {
    #[must_use]
    pub fn select(
        &mut self,
        google_ime: bool,
        gji_stamp: Option<(u64, u64)>,
        warnings: &[ModeKeyWarning],
    ) -> Vec<WarningDialogRequest> {
        // config1.dbを読めなかったGJIではWarningTrackerのsource同一性が成立せず、
        // kind変更通知のたびに警告が再生成されるため、ダイアログは出さない。
        if google_ime && gji_stamp.is_none() {
            return Vec::new();
        }

        warnings
            .iter()
            .filter_map(|warning| {
                let action = match warning.kind {
                    WarningKind::OpenAxis => WarningDialogAction::OpenAwaseSettings,
                    WarningKind::ThumbConflict if google_ime => {
                        WarningDialogAction::OpenAwaseSettings
                    }
                    WarningKind::ThumbConflict => WarningDialogAction::OpenMsImeSettings,
                    WarningKind::Composition | WarningKind::UserOverride => return None,
                };
                let identity = (warning.kind, warning.keys.clone(), action);
                if self.shown.contains(&identity) {
                    return None;
                }
                self.shown.push(identity);
                Some(WarningDialogRequest {
                    warning: warning.clone(),
                    action,
                })
            })
            .collect()
    }
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
    gji_stamp: Option<(u64, u64)>,
    msime_bits: Option<u8>,
    learned_active: Option<bool>,
    composition_keys: Vec<VkCode>,
}

impl WarningTracker {
    #[must_use]
    pub fn detect_gji(
        &mut self,
        enabled: bool,
        stamp: Option<(u64, u64)>,
        keymap: Option<&KeyEffectKeymap>,
        learned: Option<&[Cell]>,
        thumb_keys: [VkCode; 2],
    ) -> Vec<ModeKeyWarning> {
        if !enabled {
            return Vec::new();
        }
        let warnings = detect(keymap, learned, thumb_keys);
        self.deduplicate(warnings, stamp, None, learned.is_some())
    }

    #[must_use]
    pub fn detect_msime(
        &mut self,
        enabled: bool,
        packed_assignment_bits: u8,
        keymap: Option<&KeyEffectKeymap>,
        learned: Option<&[Cell]>,
        thumb_keys: [VkCode; 2],
    ) -> Vec<ModeKeyWarning> {
        if !enabled {
            return Vec::new();
        }
        let warnings = detect(keymap, learned, thumb_keys);
        self.deduplicate(
            warnings,
            None,
            Some(packed_assignment_bits),
            learned.is_some(),
        )
    }

    fn deduplicate(
        &mut self,
        warnings: Vec<ModeKeyWarning>,
        gji_stamp: Option<(u64, u64)>,
        msime_bits: Option<u8>,
        learned_active: bool,
    ) -> Vec<ModeKeyWarning> {
        // 学習表の採用有無が変わると判定の根拠が変わるため、同一ソース扱いにしない。
        let same_learned = self.learned_active.replace(learned_active) == Some(learned_active);
        let same_stamp = if let Some(stamp) = gji_stamp {
            self.gji_stamp.replace(stamp) == Some(stamp)
        } else if let Some(bits) = msime_bits {
            self.msime_bits.replace(bits) == Some(bits)
        } else {
            false
        };
        let same_source = same_learned && same_stamp;
        let composition_keys = warnings
            .iter()
            .find(|warning| warning.kind == WarningKind::Composition)
            .map_or_else(Vec::new, |warning| warning.keys.clone());
        let same_composition = self.composition_keys == composition_keys;
        self.composition_keys = composition_keys;

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
pub fn detect(
    keymap: Option<&KeyEffectKeymap>,
    learned: Option<&[Cell]>,
    thumb_keys: [VkCode; 2],
) -> Vec<ModeKeyWarning> {
    let mut open = Vec::new();
    let mut composition = Vec::new();
    let mut overrides = Vec::new();
    let mut thumbs = Vec::new();

    for vk in TARGET_VKS {
        let code = VkCode(vk);
        let is_thumb = thumb_keys.contains(&code);
        match classify_state_dependent_mode_key(keymap, vk, learned) {
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
            Some(Classification::CannotPredict(CannotPredictReason::UserOverride)) => {
                // 親指キーの場合は決定2の分岐どおりThumbConflict側へ回す。ここを
                // `is_thumb`で弾いて無視すると、無変換/変換キーをユーザー固有の
                // 上書きに割り当てている親指シフトユーザー（本ADRが最初に想定した
                // ケースそのもの）に何の警告も出なくなる（/code-review PR #249指摘）。
                if is_thumb {
                    thumbs.push(code);
                } else {
                    overrides.push(code);
                }
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
        let warnings = detect(Some(&atok()), None, [VkCode(0), VkCode(0)]);
        assert!(warnings
            .iter()
            .any(|w| w.kind == WarningKind::OpenAxis && w.message.contains("モードがずれる")));
        assert!(warnings.iter().any(|w| w.kind == WarningKind::Composition
            && w.message.contains("置き換えだけでは解決しません")));
    }

    #[test]
    fn thumb_keys_are_routed_to_existing_conflict_style_warning() {
        let warnings = detect(Some(&atok()), None, [VkCode(0x1C), VkCode(0x1D)]);
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
        assert!(detect(Some(&custom), None, [VkCode(0), VkCode(0)])
            .iter()
            .any(|w| w.kind == WarningKind::UserOverride));
        let native = KeyEffectKeymap::for_msime_native(false, None, None);
        assert!(detect(Some(&native), None, [VkCode(0), VkCode(0)]).is_empty());
        assert!(detect(None, None, [VkCode(0), VkCode(0)]).is_empty());
    }

    #[test]
    fn thumb_key_with_user_override_warns_as_thumb_conflict_not_silence() {
        // 無変換/変換キーがユーザー固有のIME上書きに割り当てられ、かつそれを親指シフト
        // として使っているケース（本ADRが最初に想定したシナリオそのもの）。`is_thumb`で
        // CannotPredict(UserOverride)を弾くと何の警告も出ない退行になる（/code-review
        // PR #249指摘）。
        let custom =
            KeyEffectKeymap::from_config(Some(2), Some("DirectInput\tHenkan\tIMEOn".into()), &[])
                .unwrap();
        let warnings = detect(Some(&custom), None, [VkCode(0x1C), VkCode(0)]);
        assert!(
            warnings
                .iter()
                .any(|w| w.kind == WarningKind::ThumbConflict),
            "thumb key with UserOverride must not be silently dropped: {warnings:?}"
        );
        assert!(!warnings.iter().any(|w| w.kind == WarningKind::UserOverride));
    }

    #[test]
    fn warnings_are_once_per_source_but_repeat_after_identity_change() {
        let mut tracker = WarningTracker::default();
        let thumbs = [VkCode(0), VkCode(0)];
        assert!(!tracker
            .detect_gji(true, Some((1, 1)), Some(&atok()), None, thumbs)
            .is_empty());
        assert!(tracker
            .detect_gji(true, Some((1, 1)), Some(&atok()), None, thumbs)
            .is_empty());
        assert!(!tracker
            .detect_gji(true, Some((2, 1)), Some(&atok()), None, thumbs)
            .is_empty());
        assert!(tracker
            .detect_gji(false, Some((3, 1)), Some(&atok()), None, thumbs)
            .is_empty());
    }

    #[test]
    fn adr192_t5_warning_follows_learned_table_and_repeats_when_adoption_changes() {
        use super::super::key_effect_predictor::{cell, Conv, Disp, Stage, TableKey};
        // 変換キーが常にONになる（状態非依存）と示す学習表。ATOK同梱表では変換キーは開閉依存。
        let learned = [
            cell(
                false,
                None,
                Stage::None,
                TableKey::Henkan,
                true,
                Some(Conv::C19),
                Disp::None,
            ),
            cell(
                true,
                Some(Conv::C10),
                Stage::None,
                TableKey::Henkan,
                true,
                Some(Conv::C10),
                Disp::None,
            ),
        ];
        let thumbs = [VkCode(0), VkCode(0)];
        let henkan_open_axis = |warnings: &[ModeKeyWarning]| {
            warnings
                .iter()
                .any(|w| w.kind == WarningKind::OpenAxis && w.keys.contains(&VkCode(0x1C)))
        };
        assert!(henkan_open_axis(&detect(Some(&atok()), None, thumbs)));
        assert!(!henkan_open_axis(&detect(
            Some(&atok()),
            Some(&learned),
            thumbs
        )));

        // 同じ設定ファイルの版でも、学習表の採用有無が変わったら警告を出し直す。
        let mut tracker = WarningTracker::default();
        let stamp = Some((1, 1));
        assert!(!tracker
            .detect_gji(true, stamp, Some(&atok()), None, thumbs)
            .is_empty());
        assert!(tracker
            .detect_gji(true, stamp, Some(&atok()), None, thumbs)
            .is_empty());
        assert!(!tracker
            .detect_gji(true, stamp, Some(&atok()), Some(&learned), thumbs)
            .is_empty());
    }

    #[test]
    fn composition_identity_is_the_actual_notified_key_set() {
        let mut tracker = WarningTracker::default();
        let first = tracker.detect_gji(
            true,
            Some((1, 1)),
            Some(&atok()),
            None,
            [VkCode(0), VkCode(0)],
        );
        assert!(first.iter().any(|w| w.kind == WarningKind::Composition));
        let changed = tracker.detect_gji(
            true,
            Some((2, 1)),
            Some(&atok()),
            None,
            [VkCode(0x19), VkCode(0)],
        );
        assert!(changed.iter().any(|w| w.kind == WarningKind::Composition));
    }

    #[test]
    fn only_open_axis_and_thumb_conflict_are_dialog_targets() {
        let warnings = [
            ModeKeyWarning::new(WarningKind::OpenAxis, vec![VkCode(0x1c)]),
            ModeKeyWarning::new(WarningKind::Composition, vec![VkCode(0x19)]),
            ModeKeyWarning::new(WarningKind::UserOverride, vec![VkCode(0x1d)]),
            ModeKeyWarning::new(WarningKind::ThumbConflict, vec![VkCode(0xf3)]),
        ];
        let selected = WarningDialogTracker::default().select(false, None, &warnings);
        assert_eq!(selected.len(), 2);
        assert!(selected.iter().all(|request| matches!(
            request.warning.kind,
            WarningKind::OpenAxis | WarningKind::ThumbConflict
        )));
    }

    #[test]
    fn gji_dialog_requires_config_stamp_and_same_content_is_only_shown_once() {
        let warning = ModeKeyWarning::new(WarningKind::OpenAxis, vec![VkCode(0x1c)]);
        let mut tracker = WarningDialogTracker::default();
        assert!(tracker
            .select(true, None, std::slice::from_ref(&warning))
            .is_empty());
        assert_eq!(
            tracker
                .select(true, Some((1, 1)), std::slice::from_ref(&warning))
                .len(),
            1
        );
        assert!(tracker
            .select(true, Some((2, 1)), std::slice::from_ref(&warning))
            .is_empty());
    }

    #[test]
    fn thumb_conflict_destination_depends_on_ime_kind() {
        let warning = ModeKeyWarning::new(WarningKind::ThumbConflict, vec![VkCode(0xf3)]);
        let gji = WarningDialogTracker::default().select(
            true,
            Some((1, 1)),
            std::slice::from_ref(&warning),
        );
        let msime = WarningDialogTracker::default().select(false, None, &[warning]);
        assert_eq!(gji[0].action, WarningDialogAction::OpenAwaseSettings);
        assert_eq!(msime[0].action, WarningDialogAction::OpenMsImeSettings);
    }
}
