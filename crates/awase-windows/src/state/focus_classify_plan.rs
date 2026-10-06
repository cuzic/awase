//! フォーカス窓の同期分類（ウィンドウスタイル・クラス名）の決定（純粋）。
//!
//! `focus/classify.rs::classify_focus` に埋め込まれていた判断表（`WS_EX_NOIME`・`ES_READONLY`・
//! 既知のテキスト/非テキストのクラス名）を、OS を読まない純粋関数として切り出したもの
//! （FCIS F5b、ADR-229）。`classify_focus` は `observe`（`GetWindowLongW`・`GetClassNameW`）→
//! [`decide_by_ex_style`] / [`decide_by_class`] → `execute`（判定不能なら `msaa_classify`）の殻になる。
//! 読み取りの順序と回数は元のまま（拡張スタイルで確定したらクラス名は読まず、`GWL_STYLE` は
//! クラス名が `Edit` のときだけ読む）。判定の順序・表の中身も不変。
//!
//! `ClassifyResult` / `ClassifyReason`（判定の結果と根拠、journal・ログに出る文字列）もここに置き、
//! `focus/classify.rs` から再公開する。

use crate::focus::FocusKind;

/// `WS_EX_NOIME` (0x0040_0000) — IME 入力を受け付けないウィンドウスタイル
const WS_EX_NOIME: i32 = 0x0040_0000;

/// `ES_READONLY` (0x0800) — 読み取り専用 Edit コントロール
const ES_READONLY: i32 = 0x0800;

/// フォーカス判定の結果と根拠
#[derive(Debug, PartialEq, Eq)]
pub struct ClassifyResult {
    pub kind: FocusKind,
    pub reason: ClassifyReason,
}

/// 判定根拠
#[derive(Debug, PartialEq, Eq)]
pub enum ClassifyReason {
    /// hwnd が NULL
    NullHwnd,
    /// WS_EX_NOIME ウィンドウスタイル
    NoImeStyle,
    /// Edit コントロールの ES_READONLY
    ReadOnlyEdit,
    /// 既知のテキスト入力クラス名
    KnownTextClass(String),
    /// 既知の非テキストクラス名
    KnownNonTextClass(String),
    /// MSAA ロールによる判定
    MsaaRole(String),
    /// Phase 1-2 で判定不能
    Undetermined,
}

impl std::fmt::Display for ClassifyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NullHwnd => write!(f, "NullHwnd"),
            Self::NoImeStyle => write!(f, "NoImeStyle"),
            Self::ReadOnlyEdit => write!(f, "ReadOnlyEdit"),
            Self::KnownTextClass(c) => write!(f, "KnownTextClass({c})"),
            Self::KnownNonTextClass(c) => write!(f, "KnownNonTextClass({c})"),
            Self::MsaaRole(r) => write!(f, "MsaaRole({r})"),
            Self::Undetermined => write!(f, "Undetermined"),
        }
    }
}

/// 段階 1: 拡張スタイルが `WS_EX_NOIME` なら NonText で確定。`None` なら次の段階へ。
pub(crate) fn decide_by_ex_style(ex_style: i32) -> Option<ClassifyResult> {
    (ex_style & WS_EX_NOIME != 0).then_some(ClassifyResult {
        kind: FocusKind::NonText,
        reason: ClassifyReason::NoImeStyle,
    })
}

/// クラス名が `GWL_STYLE` の読み取りを要するか（元のコードで `Edit` のときだけ読んでいた）。
pub(crate) fn needs_edit_style(class_name: &str) -> bool {
    class_name == "Edit"
}

/// 段階 2: クラス名（と、`Edit` のときの `GWL_STYLE`）で決める。`None` なら MSAA へ進む。
///
/// `edit_style` は [`needs_edit_style`] が真のときだけ `Some`（それ以外は無視する）。
/// クラス名が空なら（`GetClassNameW` 失敗）常に `None`。
pub(crate) fn decide_by_class(
    class_name: String,
    edit_style: Option<i32>,
) -> Option<ClassifyResult> {
    if class_name.is_empty() {
        return None;
    }
    // 既知のテキスト入力コントロール
    if matches!(
        class_name.as_str(),
        "Edit"
            | "RichEdit"
            | "RichEdit20A"
            | "RichEdit20W"
            | "RICHEDIT50W"
            | "RichEditD2DPT"
            | "Scintilla"
            | "ConsoleWindowClass"
    ) {
        // Edit コントロールの読み取り専用チェック
        if needs_edit_style(&class_name) && edit_style.is_some_and(|s| s & ES_READONLY != 0) {
            return Some(ClassifyResult {
                kind: FocusKind::NonText,
                reason: ClassifyReason::ReadOnlyEdit,
            });
        }
        return Some(ClassifyResult {
            kind: FocusKind::TextInput,
            reason: ClassifyReason::KnownTextClass(class_name),
        });
    }

    // 既知の非テキストコントロール
    if matches!(
        class_name.as_str(),
        "Button"
            | "Static"
            | "SysListView32"
            | "SysTreeView32"
            | "SysHeader32"
            | "ToolbarWindow32"
            | "msctls_statusbar32"
            | "SysTabControl32"
            | "msctls_trackbar32"
            | "msctls_progress32"
    ) {
        return Some(ClassifyResult {
            kind: FocusKind::NonText,
            reason: ClassifyReason::KnownNonTextClass(class_name),
        });
    }

    // Windows シェルインフラ（デスクトップ切替アニメーション中に通過するホストウィンドウ）。
    // Imm32Unavailable プロファイルで IME 制御不能なため NonText とみなす。
    // NonText 判定により on_focus_process_changed の reset_to_off_for_tsf_native_cache_miss
    // が回避され、仮想デスクトップ切替時に LINE 等で Engine OFF になる問題を防ぐ。
    // Windows 11 エクスプローラー・タスクバーの XAML ホスト
    // （IMM クロスプロセスクエリがタイムアウトするため Imm32Unavailable かつ TsfNative）
    if class_name == "XamlExplorerHostIslandWindow" {
        return Some(ClassifyResult {
            kind: FocusKind::NonText,
            reason: ClassifyReason::KnownNonTextClass(class_name),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &[&str] = &[
        "RichEdit",
        "RichEdit20A",
        "RichEdit20W",
        "RICHEDIT50W",
        "RichEditD2DPT",
        "Scintilla",
        "ConsoleWindowClass",
    ];
    const NON_TEXT: &[&str] = &[
        "Button",
        "Static",
        "SysListView32",
        "SysTreeView32",
        "SysHeader32",
        "ToolbarWindow32",
        "msctls_statusbar32",
        "SysTabControl32",
        "msctls_trackbar32",
        "msctls_progress32",
        "XamlExplorerHostIslandWindow",
    ];

    fn r(kind: FocusKind, reason: ClassifyReason) -> Option<ClassifyResult> {
        Some(ClassifyResult { kind, reason })
    }

    #[test]
    fn ex_style_noime_decides_non_text_others_fall_through() {
        assert_eq!(
            decide_by_ex_style(WS_EX_NOIME),
            r(FocusKind::NonText, ClassifyReason::NoImeStyle)
        );
        assert_eq!(
            decide_by_ex_style(WS_EX_NOIME | 0x100),
            r(FocusKind::NonText, ClassifyReason::NoImeStyle)
        );
        assert_eq!(decide_by_ex_style(0), None);
        assert_eq!(decide_by_ex_style(!WS_EX_NOIME), None);
    }

    #[test]
    fn known_text_classes_are_text_input() {
        for c in TEXT {
            for style in [None, Some(0), Some(ES_READONLY)] {
                assert_eq!(
                    decide_by_class((*c).to_string(), style),
                    r(
                        FocusKind::TextInput,
                        ClassifyReason::KnownTextClass((*c).to_string())
                    ),
                    "{c} {style:?}"
                );
            }
        }
    }

    #[test]
    fn edit_is_text_unless_readonly() {
        let text = || {
            r(
                FocusKind::TextInput,
                ClassifyReason::KnownTextClass("Edit".into()),
            )
        };
        assert!(needs_edit_style("Edit"));
        assert!(!needs_edit_style("RichEdit"));
        assert_eq!(decide_by_class("Edit".into(), Some(0)), text());
        assert_eq!(decide_by_class("Edit".into(), Some(0x1000)), text());
        assert_eq!(
            decide_by_class("Edit".into(), Some(ES_READONLY)),
            r(FocusKind::NonText, ClassifyReason::ReadOnlyEdit)
        );
        assert_eq!(
            decide_by_class("Edit".into(), Some(ES_READONLY | 1)),
            r(FocusKind::NonText, ClassifyReason::ReadOnlyEdit)
        );
    }

    #[test]
    fn known_non_text_classes_are_non_text() {
        for c in NON_TEXT {
            assert_eq!(
                decide_by_class((*c).to_string(), None),
                r(
                    FocusKind::NonText,
                    ClassifyReason::KnownNonTextClass((*c).to_string())
                ),
                "{c}"
            );
        }
    }

    #[test]
    fn unknown_or_empty_class_falls_through_to_msaa() {
        for c in [
            "",
            "Chrome_WidgetWin_1",
            "edit",
            "Qt663QWindowIcon",
            "Edit ",
        ] {
            assert_eq!(decide_by_class(c.to_string(), Some(0)), None, "{c:?}");
        }
    }

    #[test]
    fn reason_display_strings_are_stable() {
        assert_eq!(ClassifyReason::NullHwnd.to_string(), "NullHwnd");
        assert_eq!(ClassifyReason::NoImeStyle.to_string(), "NoImeStyle");
        assert_eq!(ClassifyReason::ReadOnlyEdit.to_string(), "ReadOnlyEdit");
        assert_eq!(
            ClassifyReason::KnownNonTextClass("Button".into()).to_string(),
            "KnownNonTextClass(Button)"
        );
        assert_eq!(
            ClassifyReason::KnownTextClass("Edit".into()).to_string(),
            "KnownTextClass(Edit)"
        );
        assert_eq!(
            ClassifyReason::MsaaRole("Text".into()).to_string(),
            "MsaaRole(Text)"
        );
        assert_eq!(ClassifyReason::Undetermined.to_string(), "Undetermined");
    }
}
