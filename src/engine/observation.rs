//! OS 非依存の観測結果型定義。
//!
//! Observer レイヤー（Win32 依存）が OS API を呼び出して取得した結果を、
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
    /// - クロスプロセス検出成功 → その値を採用
    /// - Unreliable で Off 検出 → shadow にフォールバック
    /// - 検出不可 → shadow にフォールバック
    #[must_use]
    pub fn resolve(self, shadow_ime_on: bool) -> Option<bool> {
        if !self.is_japanese {
            return Some(false);
        }

        // Unreliable な環境で Off 検出した場合は shadow にフォールバック
        let cross_process =
            if self.cross_process == Some(false) && self.reliability != ImeReliability::Reliable {
                None
            } else {
                self.cross_process
            };

        Some(cross_process.unwrap_or(shadow_ime_on))
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
