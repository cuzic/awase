//! OS 非依存の観測結果型定義。
//!
//! Observer レイヤー（プラットフォーム依存）が OS API を呼び出して取得した結果を、
//! これらの型に変換して Engine に渡す。Engine は OS API に一切依存せず判断を行う。

use crate::types::FocusKind;

/// フォーカス変更の観測結果（OS 非依存）
///
/// デバウンス後に確定したフォーカス先の情報。
/// 前面プロセスが変わった場合のみ Engine に送られる（ADR 028）。
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
}

