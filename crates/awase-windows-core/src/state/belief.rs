//! IME 補助状態（input_mode / is_japanese_ime / prev_conversion_mode〈直近に観測した conv、予測の `conv_raw` 用〉）。
//!
//! # IME 状態の 3 層モデル（Phase 3e 以降）
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ Layer 1: 生観測 event (ImeEvent::ObserverReported)           │
//! │  各ソース (ObserverPoll / FocusProbe / Gji / Tsf / HwndCache) │
//! │  は ImeEvent を dispatch する。shadow_model.reduce() が記録。  │
//! └────────────────────┬────────────────────────────────────────┘
//!                      │ reduce() → observations.record()
//! ┌────────────────────▼────────────────────────────────────────┐
//! │ Layer 2: shadow_model.desired_open / effective_open()       │
//! │  Engine が前提とすべき IME 状態の SSOT。                     │
//! │  UserImeSetIntent のみが書き換え可能。                       │
//! └────────────────────┬────────────────────────────────────────┘
//!                      │ apply_ime_open() → OS に送信
//! ┌────────────────────▼────────────────────────────────────────┐
//! │ Layer 3: 制御ログ (ImeModel.applied_open / applied_at_ms)   │
//! │  最後に OS に送ったコマンド値。VK_KANJI 重複送信防止専用。   │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! `ImeBelief` は IME ON/OFF 自体は持たず、補助的な属性（input_mode 等）のみを保持する。

/// IME 補助状態 (is_japanese_ime / prev_conversion_mode)。
///
/// IME ON/OFF 自体は [`crate::state::ime_model::ImeModel`] の `desired_open` が SSOT。
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct ImeBelief {
    /// 日本語 IME がアクティブか
    pub(in crate::state) is_japanese_ime: bool,
    /// 直前の conversion_mode（直近に観測した conv。読み手は予測の入力 `conv_raw` だけ。かな切替の検出は ROMAN ビットを直接読む
    /// `is_romaji` が担い、以前の「前回との差分」の分類は撤去した、ADR-239）
    /// None = まだ一度も取得できていない
    pub(in crate::state) prev_conversion_mode: Option<u32>,
    /// 英数モードの「候補」(1 回目の英数の読み。確認の読みで確定する、ADR-238 / BUG-190)。
    /// `prev_conversion_mode` と同じ扱い: 読みの結果として `apply_ime_update` 経由でだけ書き、フォーカス変更で捨てる。
    pub(in crate::state) eisu_candidate: Option<crate::state::eisu_candidate::EisuCandidate>,
}

impl Default for ImeBelief {
    fn default() -> Self {
        Self {
            is_japanese_ime: true,
            prev_conversion_mode: None,
            eisu_candidate: None,
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
impl ImeBelief {
    /// パニックリセット（`PanicReset`）で belief を既定へ戻す（日本語 IME 扱い・直前の conv と英数候補を捨てる）。
    /// 書き手は殻の crate の `ImeStateHub`（crate を分けたので、フィールドを直接書かずこの口を通す。ADR-229 D4）。
    pub fn reset_for_panic(&mut self) {
        self.is_japanese_ime = true;
        self.prev_conversion_mode = None;
        self.eisu_candidate = None;
    }

    /// 日本語 IME がアクティブかを書く（`apply_ime_update` の `is_japanese_ime` 更新）。
    pub fn set_japanese_ime(&mut self, is_jp: bool) {
        self.is_japanese_ime = is_jp;
    }

    /// 直前の conversion_mode を書く（`apply_ime_update` の更新とフォーカス変更時の破棄〔`None`〕）。
    pub fn set_prev_conversion_mode(&mut self, conv: Option<u32>) {
        self.prev_conversion_mode = conv;
    }

    /// 英数モードの候補を更新する（ADR-238）。
    pub fn apply_eisu_candidate_update(
        &mut self,
        update: crate::state::eisu_candidate::CandidateUpdate,
    ) {
        use crate::state::eisu_candidate::CandidateUpdate;
        match update {
            CandidateUpdate::Keep => {}
            CandidateUpdate::Set(c) => self.eisu_candidate = Some(c),
            CandidateUpdate::Clear => self.eisu_candidate = None,
        }
    }

    /// 日本語 IME がアクティブかを返す。
    #[inline]
    #[must_use]
    pub const fn is_japanese_ime(&self) -> bool {
        self.is_japanese_ime
    }

    /// 直前の conversion_mode を返す。
    #[inline]
    #[must_use]
    pub const fn prev_conversion_mode(&self) -> Option<u32> {
        self.prev_conversion_mode
    }

    /// 英数モードの候補を返す(ADR-238)。
    #[inline]
    #[must_use]
    pub const fn eisu_candidate(&self) -> Option<crate::state::eisu_candidate::EisuCandidate> {
        self.eisu_candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::eisu_candidate::EisuCandidate;

    #[test]
    fn accessors_return_the_stored_fields() {
        let belief = ImeBelief {
            is_japanese_ime: false,
            prev_conversion_mode: Some(0x19),
            eisu_candidate: Some(EisuCandidate { at_ms: 7, conv: 0 }),
        };
        assert!(!belief.is_japanese_ime());
        assert_eq!(belief.prev_conversion_mode(), Some(0x19));
        assert_eq!(
            belief.eisu_candidate(),
            Some(EisuCandidate { at_ms: 7, conv: 0 })
        );

        let default = ImeBelief::default();
        assert!(default.is_japanese_ime());
        assert_eq!(default.prev_conversion_mode(), None);
    }
}
