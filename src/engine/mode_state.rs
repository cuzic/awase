//! 入力方式の確度付き状態モデル: `InputModeState` / `AssumedReason`。

/// 入力方式の確度付き状態。
///
/// `bool` では「観測値」と「IMM broken アプリ向け仮定値」を区別できず、
/// Chrome 等で stale な false に上書きされる問題があるため確度を保持する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InputModeState {
    /// IMM クエリ等でローマ字入力と確認できた
    ObservedRomaji,
    /// IMM クエリ等でかな入力と確認できた（ひらがな・JISかな。英数とは区別する）
    ObservedKana,
    /// IMM クエリ等で英数モードと確認できた（半角英数・全角英数）
    ObservedEisu,
    /// 観測不能だが状況証拠から romaji と仮定（Chrome/UWP/Electron 等）
    AssumedRomaji { reason: AssumedReason },
    /// 不明（起動直後、フォーカス確定前等）
    Unknown,
}

/// `AssumedRomaji` の根拠
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AssumedReason {
    /// IMM ブリッジが broken と既知のクラス名（Chrome_WidgetWin_1 等）
    ImmBridgeBroken,
    /// フォーカス変更直後で観測確定前
    FocusTransition,
    /// AppKind が TsfNative/UWP で IMM クエリをスキップしている
    AppKindExcluded,
    /// 強制 ON ガード中（連続検出失敗による）
    ForceOnGuardActive,
}

impl InputModeState {
    /// ローマ字入力と判断できるかどうか。
    /// `ObservedRomaji` と `AssumedRomaji` を true とみなす。
    #[must_use]
    pub const fn is_romaji_capable(self) -> bool {
        matches!(self, Self::ObservedRomaji | Self::AssumedRomaji { .. })
    }
}
