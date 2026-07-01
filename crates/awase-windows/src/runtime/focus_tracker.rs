//! フォーカス追跡・IMM 能力学習の判断ロジックを担う `FocusTracker`。
//!
//! フォーカス *状態* 自体は [`crate::state`] の `FocusStore`（`platform.focus`）が
//! SSOT として保持する。ここではその状態に対する純粋な判断（IMM 能力の学習判定）を
//! `Runtime` から切り出し、テスト可能にする。
//!
//! また IME sync キーのリスト（`sync_toggle_keys` / `sync_on_keys` / `sync_off_keys`）を
//! 保持し、`enrich_ime_relevance` による `RawKeyEvent` の補完ロジックを担う。

use awase::types::{RawKeyEvent, ShadowImeAction, VkCode};

use crate::focus::classifier::ImmCapability;

/// フォーカス追跡に付随する判断ロジックの集約点。
///
/// - IMM 能力学習の純粋な判断（[`Self::decide_imm_capability`]）
/// - IME sync キー情報（`sync_toggle/on/off_keys`）による [`Self::enrich_ime_relevance`]
pub(crate) struct FocusTracker {
    /// トグル系 sync キー（KANJI 等）
    pub(crate) sync_toggle_keys: Vec<VkCode>,
    /// IME ON 系 sync キー
    pub(crate) sync_on_keys: Vec<VkCode>,
    /// IME OFF 系 sync キー（無変換等）
    pub(crate) sync_off_keys: Vec<VkCode>,
}

impl FocusTracker {
    pub(crate) fn new(
        sync_toggle_keys: Vec<VkCode>,
        sync_on_keys: Vec<VkCode>,
        sync_off_keys: Vec<VkCode>,
    ) -> Self {
        Self {
            sync_toggle_keys,
            sync_on_keys,
            sync_off_keys,
        }
    }

    /// IME 関連の事前分類情報を sync key 設定で補完する。
    ///
    /// `Runtime::enrich_ime_relevance` の実処理をここに集約する。
    /// `sync_toggle_keys` / `sync_on_keys` / `sync_off_keys` に基づいて
    /// `event.ime_relevance` の `is_sync_key` / `sync_direction` / `may_change_ime` を設定する。
    pub(crate) fn enrich_ime_relevance(&self, event: &mut RawKeyEvent) {
        let vk = event.vk_code;
        let rel = &mut event.ime_relevance;

        if self.sync_toggle_keys.contains(&vk) {
            rel.is_sync_key = true;
            rel.sync_direction = Some(ShadowImeAction::Toggle);
            rel.may_change_ime = true;
        } else if self.sync_on_keys.contains(&vk) {
            rel.is_sync_key = true;
            rel.sync_direction = Some(ShadowImeAction::TurnOn);
            rel.may_change_ime = true;
        } else if self.sync_off_keys.contains(&vk) {
            rel.is_sync_key = true;
            rel.sync_direction = Some(ShadowImeAction::TurnOff);
            rel.may_change_ime = true;
        }
    }

    /// IMM 検出のミス数遷移から、記録すべき新しい [`ImmCapability`] を判定する。
    ///
    /// `Runtime::learn_imm_capability_from_miss` の純粋な決定部。I/O（クラス名取得・
    /// キャッシュ書き込み・ログ）は呼び出し元に残す。
    ///
    /// - ミスが `>0` から `0` に回復 → `Works`
    /// - ミスが閾値未満から閾値以上に悪化 → `Unavailable`
    /// - それ以外、または既に同じ能力が記録済み → `None`（記録不要）
    #[must_use]
    pub(crate) fn decide_imm_capability(
        miss_before: u32,
        miss_after: u32,
        threshold: u32,
        current: Option<ImmCapability>,
    ) -> Option<ImmCapability> {
        if miss_after == 0 && miss_before > 0 {
            (current != Some(ImmCapability::Works)).then_some(ImmCapability::Works)
        } else if miss_after >= threshold && miss_before < threshold {
            (current != Some(ImmCapability::Unavailable)).then_some(ImmCapability::Unavailable)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u32 = 3;

    #[test]
    fn recovery_learns_works() {
        assert_eq!(
            FocusTracker::decide_imm_capability(2, 0, T, None),
            Some(ImmCapability::Works)
        );
    }

    #[test]
    fn recovery_when_already_works_is_noop() {
        assert_eq!(
            FocusTracker::decide_imm_capability(2, 0, T, Some(ImmCapability::Works)),
            None
        );
    }

    #[test]
    fn crossing_threshold_learns_unavailable() {
        assert_eq!(
            FocusTracker::decide_imm_capability(2, 3, T, None),
            Some(ImmCapability::Unavailable)
        );
    }

    #[test]
    fn already_above_threshold_is_noop() {
        // miss_before すでに閾値以上 → 遷移なし
        assert_eq!(FocusTracker::decide_imm_capability(3, 4, T, None), None);
    }

    #[test]
    fn steady_miss_zero_is_noop() {
        // miss_before==0 なので回復条件を満たさない
        assert_eq!(FocusTracker::decide_imm_capability(0, 0, T, None), None);
    }
}
