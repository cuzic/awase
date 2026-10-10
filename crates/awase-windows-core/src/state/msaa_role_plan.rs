//! MSAA ロール値 → フォーカス種別の決定（純粋）。
//!
//! `focus/msaa.rs::msaa_classify` に埋め込まれていた「ロール値の表引き（Text/Document=入力、
//! ツールバー等=非入力、それ以外=判定不能）」を、OS を読まない純粋関数として切り出したもの
//! （FCIS F5、ADR-229）。`msaa_classify` は `observe`（`AccessibleObjectFromWindow` と
//! `get_accRole` でロール値を読む）→ [`decide_msaa_role`] → `execute`（`ClassifyResult` への
//! 変換とログ）の薄い殻になる。挙動不変: 表の中身・判定の優先順（入力ロール → 非入力ロール →
//! 判定不能）は元のまま。

use crate::focus::FocusKind;

/// MSAA アクセシビリティロール（`ROLE_SYSTEM_*` の、判定に使う分だけ）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MsaaRole {
    TitleBar = 1,
    MenuBar = 2,
    ScrollBar = 3,
    MenuPopup = 11,
    MenuItem = 12,
    Document = 15,
    ToolBar = 22,
    StatusBar = 23,
    List = 33,
    ListItem = 34,
    Outline = 35,
    OutlineItem = 36,
    PageTab = 37,
    Indicator = 39,
    Graphic = 40,
    StaticText = 41,
    Text = 42,
    PushButton = 43,
    ProgressBar = 48,
    Slider = 51,
}

impl MsaaRole {
    pub(crate) const fn from_u32(v: u32) -> Option<Self> {
        match v {
            1 => Some(Self::TitleBar),
            2 => Some(Self::MenuBar),
            3 => Some(Self::ScrollBar),
            11 => Some(Self::MenuPopup),
            12 => Some(Self::MenuItem),
            15 => Some(Self::Document),
            22 => Some(Self::ToolBar),
            23 => Some(Self::StatusBar),
            33 => Some(Self::List),
            34 => Some(Self::ListItem),
            35 => Some(Self::Outline),
            36 => Some(Self::OutlineItem),
            37 => Some(Self::PageTab),
            39 => Some(Self::Indicator),
            40 => Some(Self::Graphic),
            41 => Some(Self::StaticText),
            42 => Some(Self::Text),
            43 => Some(Self::PushButton),
            48 => Some(Self::ProgressBar),
            51 => Some(Self::Slider),
            _ => None,
        }
    }

    const fn is_text_input(self) -> bool {
        matches!(self, Self::Text | Self::Document)
    }

    const fn is_non_text(self) -> bool {
        matches!(
            self,
            Self::TitleBar
                | Self::MenuBar
                | Self::ScrollBar
                | Self::MenuPopup
                | Self::MenuItem
                | Self::ToolBar
                | Self::StatusBar
                | Self::List
                | Self::ListItem
                | Self::Outline
                | Self::OutlineItem
                | Self::PageTab
                | Self::Indicator
                | Self::Graphic
                | Self::StaticText
                | Self::PushButton
                | Self::ProgressBar
                | Self::Slider
        )
    }
}

/// [`decide_msaa_role`] の決定。理由（どのロールか／なぜ判定不能か）を型で持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsaaRoleDecision {
    /// テキスト入力ロール（Text / Document）
    TextInput(MsaaRole),
    /// 非テキストロール（ツールバー・メニュー等）
    NonText(MsaaRole),
    /// ロール値は読めたが許可/拒否の表に無い
    UndeterminedUnlisted(u32),
    /// ロール値を読めなかった（`AccessibleObjectFromWindow` / `get_accRole` の失敗）
    UndeterminedUnread,
}

impl MsaaRoleDecision {
    /// この決定が表す `FocusKind`。
    #[must_use]
    pub const fn kind(self) -> FocusKind {
        match self {
            Self::TextInput(_) => FocusKind::TextInput,
            Self::NonText(_) => FocusKind::NonText,
            Self::UndeterminedUnlisted(_) | Self::UndeterminedUnread => FocusKind::Undetermined,
        }
    }
}

/// MSAA ロール値（読めなかったら `None`）から決定を返す。OS には触れない。
#[must_use]
pub const fn decide_msaa_role(role_id: Option<u32>) -> MsaaRoleDecision {
    let Some(id) = role_id else {
        return MsaaRoleDecision::UndeterminedUnread;
    };
    match MsaaRole::from_u32(id) {
        Some(role) if role.is_text_input() => MsaaRoleDecision::TextInput(role),
        Some(role) if role.is_non_text() => MsaaRoleDecision::NonText(role),
        _ => MsaaRoleDecision::UndeterminedUnlisted(id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 元の `msaa_classify` の表（全 20 ロール）を全数で固定する。
    const TABLE: &[(u32, MsaaRole, FocusKind)] = &[
        (1, MsaaRole::TitleBar, FocusKind::NonText),
        (2, MsaaRole::MenuBar, FocusKind::NonText),
        (3, MsaaRole::ScrollBar, FocusKind::NonText),
        (11, MsaaRole::MenuPopup, FocusKind::NonText),
        (12, MsaaRole::MenuItem, FocusKind::NonText),
        (15, MsaaRole::Document, FocusKind::TextInput),
        (22, MsaaRole::ToolBar, FocusKind::NonText),
        (23, MsaaRole::StatusBar, FocusKind::NonText),
        (33, MsaaRole::List, FocusKind::NonText),
        (34, MsaaRole::ListItem, FocusKind::NonText),
        (35, MsaaRole::Outline, FocusKind::NonText),
        (36, MsaaRole::OutlineItem, FocusKind::NonText),
        (37, MsaaRole::PageTab, FocusKind::NonText),
        (39, MsaaRole::Indicator, FocusKind::NonText),
        (40, MsaaRole::Graphic, FocusKind::NonText),
        (41, MsaaRole::StaticText, FocusKind::NonText),
        (42, MsaaRole::Text, FocusKind::TextInput),
        (43, MsaaRole::PushButton, FocusKind::NonText),
        (48, MsaaRole::ProgressBar, FocusKind::NonText),
        (51, MsaaRole::Slider, FocusKind::NonText),
    ];

    #[test]
    fn every_listed_role_maps_to_original_kind() {
        for &(id, role, kind) in TABLE {
            let d = decide_msaa_role(Some(id));
            assert_eq!(d.kind(), kind, "role id {id}");
            match kind {
                FocusKind::TextInput => assert_eq!(d, MsaaRoleDecision::TextInput(role)),
                _ => assert_eq!(d, MsaaRoleDecision::NonText(role)),
            }
            assert_eq!(MsaaRole::from_u32(id), Some(role));
            assert_eq!(role as u32, id);
        }
    }

    #[test]
    fn every_unlisted_role_is_undetermined() {
        let listed: Vec<u32> = TABLE.iter().map(|t| t.0).collect();
        for id in 0..=200u32 {
            if listed.contains(&id) {
                continue;
            }
            assert_eq!(
                decide_msaa_role(Some(id)),
                MsaaRoleDecision::UndeterminedUnlisted(id)
            );
        }
        assert_eq!(
            decide_msaa_role(Some(u32::MAX)),
            MsaaRoleDecision::UndeterminedUnlisted(u32::MAX)
        );
    }

    #[test]
    fn unread_role_is_undetermined() {
        let d = decide_msaa_role(None);
        assert_eq!(d, MsaaRoleDecision::UndeterminedUnread);
        assert_eq!(d.kind(), FocusKind::Undetermined);
    }

    #[test]
    fn debug_name_matches_original_reason_string() {
        // ClassifyReason::MsaaRole(format!("{role:?}")) の文字列が変わらないこと（journal/ログ互換）
        assert_eq!(format!("{:?}", MsaaRole::Document), "Document");
        assert_eq!(format!("{:?}", MsaaRole::PushButton), "PushButton");
    }
}
