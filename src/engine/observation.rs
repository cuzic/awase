//! OS 非依存の観測結果型定義。
//!
//! Observer レイヤー（プラットフォーム依存）が OS API を呼び出して取得した結果を、
//! これらの型に変換して Engine に渡す。Engine は OS API に一切依存せず判断を行う。

use crate::types::{FocusKind, ImeReliability};

use super::fsm_types::ModifierState;

/// IME 状態の観測結果（OS 非依存）
#[derive(Debug, Clone, Copy)]
pub struct ImeObservation {
    /// クロスプロセス検出結果 (Some(true/false) or None=検出不可)
    pub cross_process: Option<bool>,
    /// キーボードレイアウトが日本語か
    pub is_japanese: bool,
    /// IME の信頼度
    pub reliability: ImeReliability,
}

impl ImeObservation {
    /// shadow IME 状態を考慮して最終的な IME ON/OFF を解決する。
    ///
    /// - 日本語レイアウトでない → `false` (Off)
    /// - クロスプロセス検出 Off → Off を採用（信頼度に関わらず）
    /// - クロスプロセス検出 On + Reliable → On を採用
    /// - クロスプロセス検出 On + Unreliable/Unknown → shadow にフォールバック
    /// - 検出不可 (None) → shadow にフォールバック
    ///
    /// # 設計根拠
    /// Off の誤検知（本当は On なのに Off と返す）は稀。
    /// On の誤検知（本当は Off なのに On と返す）は Chrome 等で発生する。
    /// よって Off は常に信頼し、On だけ reliability で検証する。
    #[must_use]
    pub fn resolve(self, shadow_ime_on: bool) -> Option<bool> {
        if !self.is_japanese {
            return Some(false);
        }

        match self.cross_process {
            // Off 検出は常に信頼する（Off の誤検知はまれ）
            Some(false) => Some(false),
            // On 検出は Reliable な環境のみ信頼
            Some(true) if self.reliability == ImeReliability::Reliable => Some(true),
            // On 検出だが Unreliable/Unknown、または検出不可 → shadow にフォールバック
            _ => Some(shadow_ime_on),
        }
    }
}

/// フォーカス変更の観測結果（OS 非依存）
#[derive(Debug, Clone)]
pub struct FocusObservation {
    /// フォーカス先のプロセス ID
    pub process_id: u32,
    /// フォーカス先のクラス名
    pub class_name: String,
    /// 分類結果
    pub kind: FocusKind,
    /// 分類理由（ログ用）
    pub reason: String,
    /// UIA 非同期判定が必要か
    pub needs_uia: bool,
    /// config オーバーライドによる強制か
    pub overridden: bool,
    /// 同一プロセス内の TextInput 降格防止でスキップすべきか
    pub skip: bool,
    /// デバウンスタイマー ID（bin crate 側で設定）
    pub debounce_timer_id: usize,
    /// デバウンス期間（ミリ秒）
    pub debounce_ms: u64,
    /// キャッシュから取得した新ウィンドウのエンジン ON/OFF 状態（None = 未設定）
    pub cached_engine_enabled: Option<bool>,
    /// OS から取得した修飾キー状態（フォーカス変更時の同期用）
    pub os_modifiers: Option<ModifierState>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(
        cross_process: Option<bool>,
        is_japanese: bool,
        reliability: ImeReliability,
    ) -> ImeObservation {
        ImeObservation {
            cross_process,
            is_japanese,
            reliability,
        }
    }

    #[test]
    fn resolve_not_japanese_returns_false() {
        let result = obs(Some(true), false, ImeReliability::Reliable).resolve(true);
        assert_eq!(result, Some(false));
    }

    #[test]
    fn resolve_cross_process_false_returns_false() {
        let result = obs(Some(false), true, ImeReliability::Reliable).resolve(true);
        assert_eq!(result, Some(false));
    }

    #[test]
    fn resolve_cross_process_true_reliable_returns_true() {
        let result = obs(Some(true), true, ImeReliability::Reliable).resolve(false);
        assert_eq!(result, Some(true));
    }

    #[test]
    fn resolve_cross_process_true_unreliable_returns_shadow() {
        // shadow=true
        let result = obs(Some(true), true, ImeReliability::Unreliable).resolve(true);
        assert_eq!(result, Some(true));

        // shadow=false
        let result = obs(Some(true), true, ImeReliability::Unreliable).resolve(false);
        assert_eq!(result, Some(false));
    }

    #[test]
    fn resolve_cross_process_none_returns_shadow() {
        let result = obs(None, true, ImeReliability::Reliable).resolve(true);
        assert_eq!(result, Some(true));

        let result = obs(None, true, ImeReliability::Reliable).resolve(false);
        assert_eq!(result, Some(false));
    }
}
