//! ウィンドウクラス名による分類定数と判定関数。
//!
//! `classify.rs`・`ime.rs`・`focus_observer.rs` で重複していたクラス名リストと
//! 判定ロジックを一元管理する。

use awase::types::AppKind;

/// IMM32 クロスプロセス制御（`WM_IME_CONTROL` / `ImmSetOpenStatus`）が使えない
/// または不安定なウィンドウクラス。
///
/// これらのクラスにフォーカスがあるとき、`ImmGet*` / `SendMessage(WM_IME_CONTROL)` は
/// 反応しなかったり無期限にブロックする恐れがあるため、IME 状態検出をスキップする。
/// シャドウ状態（hook から追跡）のみで IME 状態を管理する。
///
/// 検知できないケース:
/// - 言語バーのマウス操作による IME 切り替え
/// - アプリ内の IME ボタンクリック
///   しかし、これらは非常に稀なので割り切る。
const IMM32_UNAVAILABLE_CLASSES: &[&str] = &[
    // Chromium 系（Chrome, Edge, Brave, Opera 等）
    "Chrome_RenderWidgetHostHWND",
    "Chrome_WidgetWin_0",
    "Chrome_WidgetWin_1",
    "Intermediate D3D Window",
    // UWP / WinUI
    "Windows.UI.Core.CoreWindow",
    "ApplicationFrameWindow",
    // XAML ホスト（Windows 11 エクスプローラー、タスクバー等）
    // IMM クロスプロセスクエリがタイムアウトし ~200ms ブロックするため除外。
    "XamlExplorerHostIslandWindow",
    // Console 系
    "PseudoConsoleWindow",
    "CASCADIA_HOSTING_WINDOW_CLASS",
];

/// 指定クラスが TSF ネイティブウィンドウかどうか判定する。
///
/// TSF ネイティブウィンドウでは `ImmGetContext` が NULL を返すが、
/// これは IME が OFF であることを意味しない（TSF text store で直接管理）。
/// 対象:
/// - Windows.UI.Core.CoreWindow: UWP / WinUI
/// - XamlExplorerHostIslandWindow: XAML ホスト
/// - Windows.UI.Input.InputSite.WindowClass: Windows Terminal の InputSite 子ウィンドウ
/// - CASCADIA_HOSTING_WINDOW_CLASS: Cascadia（Windows Terminal 上位ホスト）
/// - org.wezfurlong.wezterm: WezTerm（独自 TSF 実装、himc_null=true）
///
/// 注: IMM32_UNAVAILABLE_CLASSES より後に評価されるため、同クラスが両方に含まれる場合は
/// Imm32Unavailable が優先される（`from_class_name` 参照）。
#[must_use]
pub fn is_tsf_native_window(class_name: &str) -> bool {
    matches!(
        class_name,
        "Windows.UI.Core.CoreWindow"
            | "XamlExplorerHostIslandWindow"
            | "Windows.UI.Input.InputSite.WindowClass"
            | "CASCADIA_HOSTING_WINDOW_CLASS"
            | "org.wezfurlong.wezterm"
    )
}

/// `profile == AppImeProfile::TsfNative` の代わりに使うべき「実質的に TSF ネイティブか」判定。
///
/// `AppImeProfile::from_class_name` は `IMM32_UNAVAILABLE_CLASSES` を `is_tsf_native_window`
/// より優先して評価するため、CASCADIA_HOSTING_WINDOW_CLASS のような「両方に該当するクラス」
/// では `AppImeProfile::TsfNative` が一切現れず、代わりに `Imm32Unavailable` になる
/// （`from_class_name` のドキュメント参照）。そのため `profile` の値だけを見て
/// `matches!(profile, AppImeProfile::TsfNative)` と判定すると、Windows Terminal のような
/// 実質 TSF ネイティブなウィンドウを取りこぼす（2026-07-05 実機ログで確認: フォーカス着地直後の
/// "enforce IME OFF" ブロックが、Windows Terminal を非 TSF ネイティブと誤判定して発火した）。
///
/// 「このウィンドウは TSF ネイティブとして扱うべきか」を判定したい呼び出し元は、
/// `profile == AppImeProfile::TsfNative` ではなく必ずこの関数を使うこと。
#[must_use]
pub fn is_effectively_tsf_native(profile: AppImeProfile, class_name: &str) -> bool {
    profile == AppImeProfile::TsfNative || is_tsf_native_window(class_name)
}

// ── AppImeProfile ──────────────────────────────────────────────

/// フォーカス中アプリの IME 制御プロファイル。
///
/// 「Chrome/Edge 等は IMM32 クロスプロセス制御が使えない」
/// 「WezTerm 等 TSF ネイティブは VK_DBE_HIRAGANA が必要」
/// といったアプリ別の特性を 1 つの型に集約し、「クラス名で個別判定」の散在を防ぐ。
/// フォーカス変更時に `from_class_name` で決定して
/// `AppKindClassifier.current_app_profile` にキャッシュし、
/// `current_app_profile()` メソッドで参照する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppImeProfile {
    /// 通常の IMM32 アプリ。IMM32 クロスプロセス制御が使用可能。
    #[default]
    Standard,
    /// Chrome/Edge/UWP 等。IMM32 クロスプロセス制御が使えず VK_KANJI で制御する。
    /// 物理 IME キーの二重送信を防ぐため抑止も必要。
    Imm32Unavailable,
    /// TSF ネイティブ（例: WezTerm/Windows Terminal）。`VK_DBE_HIRAGANA` + TSF probe が必要。
    TsfNative,
}

impl AppImeProfile {
    /// クラス名からプロファイルを決定する。
    ///
    /// 優先順:
    /// 1. IMM32 制御不可クラス（Chrome/Edge/UWP/XAML/Console 系）→ `Imm32Unavailable`
    /// 2. TSF ネイティブ専用クラス → `TsfNative`
    /// 3. その他 → `Standard`
    ///
    /// 注: UWP/XAML/Console 系クラスは `Imm32Unavailable` にも TSF-native にも該当するが、
    /// IME 制御フロー（VK_KANJI + 物理キー抑止）を優先するため `Imm32Unavailable` を返す。
    #[must_use]
    pub fn from_class_name(class_name: &str) -> Self {
        if IMM32_UNAVAILABLE_CLASSES.contains(&class_name) {
            Self::Imm32Unavailable
        } else if is_tsf_native_window(class_name) {
            Self::TsfNative
        } else {
            Self::Standard
        }
    }

    /// IMM32 クロスプロセス制御（`ImmSetOpenStatus` / `WM_IME_CONTROL`）が使えるか。
    ///
    /// `false` のとき `WindowsPlatform::set_ime_open` や `ImmCrossProcessStrategy`
    /// は IMM32 クロスプロセス呼び出しをスキップする。
    #[must_use]
    pub const fn can_use_imm32_cross_process(&self) -> bool {
        matches!(self, Self::Standard)
    }

    /// VK_KANJI トグルキーで IME を制御するプロファイルか。
    ///
    /// `Imm32Unavailable`（Chrome/Edge 等）のみ `true`。
    /// GJI 稼働時は `GjiDirectStrategy`（VK_IME_ON/OFF）が優先されるため、
    /// このフラグは主に `send_engine_state_ime_key` での mode-key 送信スキップ判定に使用する。
    #[must_use]
    pub const fn uses_kanji_toggle(&self) -> bool {
        matches!(self, Self::Imm32Unavailable)
    }

    /// 物理 IME キー（VK_KANJI / 半角/全角 等）を OS に届けてよいか。
    ///
    /// `Imm32Unavailable` アプリでは `apply_ime_open` が VK_KANJI を送信済みなので、
    /// 物理キーをそのまま届けると二重制御になる。`false` のとき
    /// `KeyEventPipeline::stage_execute` は `Decision::Consume` に変換する。
    #[must_use]
    pub const fn should_pass_physical_key(&self) -> bool {
        !matches!(self, Self::Imm32Unavailable)
    }

    /// IMM32 で IME open 状態（`IMC_GETOPENSTATUS`）をクロスプロセス取得できるか。
    ///
    /// `false` のとき `read_ime_state_fast` は `ime_on=None` を返し shadow 状態に委ねる。
    /// `Imm32Unavailable` / `TsfNative` ともに IMM32 の状態値は信頼できない。
    #[must_use]
    pub const fn can_read_imm32_open_status(&self) -> bool {
        matches!(self, Self::Standard)
    }
}

/// `AppImeProfile` → `ImePolicyProfile` 変換。
///
/// focus 層でクラス名から決定した `AppImeProfile` を、state 層の event ペイロード型
/// `ImePolicyProfile` に変換する。変換は runtime 境界（`focus_tracking.rs` 等）で行い、
/// state 層が focus 層に直接依存しない設計を維持する。
impl From<AppImeProfile> for crate::state::ime_event::ImePolicyProfile {
    fn from(profile: AppImeProfile) -> Self {
        match profile {
            AppImeProfile::Standard => Self::ImmCross,
            AppImeProfile::Imm32Unavailable => Self::Imm32Unavailable,
            AppImeProfile::TsfNative => Self::TsfNative,
        }
    }
}

/// ブラウザ系・Electron 系のトップレベルウィンドウクラスかどうかを判定する。
///
/// Chrome 系（Chrome/Edge/Brave/Electron 等）および Firefox が対象。
/// IME 制御経路の選択（VK_KANJI 戦略 vs IMM32）に使用する。
#[must_use]
pub fn is_chromium_widget(class_name: &str) -> bool {
    class_name == "Chrome_WidgetWin_1" || class_name == "MozillaWindowClass"
}

/// ウィンドウクラス名からアプリの UI フレームワーク種別を判定する。
///
/// - `Chrome_*`: Chromium 系（Chrome, Edge, Electron, VS Code 等）
/// - `MozillaWindowClass`: Firefox（Chromium と同様の入力処理）
/// - `Windows.UI.Core.CoreWindow` / `ApplicationFrameWindow` / `Windows.UI.Input.*`: UWP / XAML 系
/// - その他: Win32 クラシック（ヒューリスティックで Chrome に昇格する場合あり）
#[must_use]
pub fn detect_app_kind(class_name: &str) -> AppKind {
    let class_lower = class_name.to_ascii_lowercase();
    if class_lower.starts_with("chrome_") || class_lower == "mozillawindowclass" {
        AppKind::TsfNative
    } else if class_lower == "windows.ui.core.corewindow"
        || class_lower == "applicationframewindow"
        || class_lower.starts_with("windows.ui.input.")
    {
        AppKind::Uwp
    } else {
        AppKind::Win32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 回帰テスト (2026-07-05): CASCADIA_HOSTING_WINDOW_CLASS (Windows Terminal) は
    // IMM32_UNAVAILABLE_CLASSES にも is_tsf_native_window にも該当するため、
    // from_class_name は優先順位により Imm32Unavailable を返す。
    // `matches!(profile, AppImeProfile::TsfNative)` という直接比較ではこれを
    // 取りこぼし、Windows Terminal 着地直後の「enforce IME OFF」ブロックが誤発火した。

    #[test]
    fn cascadia_profile_is_masked_to_imm32_unavailable() {
        assert_eq!(
            AppImeProfile::from_class_name("CASCADIA_HOSTING_WINDOW_CLASS"),
            AppImeProfile::Imm32Unavailable,
            "from_class_name の優先順位により TsfNative にはならない"
        );
    }

    #[test]
    fn cascadia_is_effectively_tsf_native_despite_masked_profile() {
        let profile = AppImeProfile::from_class_name("CASCADIA_HOSTING_WINDOW_CLASS");
        assert!(
            is_effectively_tsf_native(profile, "CASCADIA_HOSTING_WINDOW_CLASS"),
            "profile が Imm32Unavailable でも is_tsf_native_window で TSF ネイティブと判定できる"
        );
    }

    #[test]
    fn wezterm_is_tsf_native_directly_and_effectively() {
        let profile = AppImeProfile::from_class_name("org.wezfurlong.wezterm");
        assert_eq!(profile, AppImeProfile::TsfNative);
        assert!(is_effectively_tsf_native(profile, "org.wezfurlong.wezterm"));
    }

    #[test]
    fn chrome_is_not_effectively_tsf_native() {
        let profile = AppImeProfile::from_class_name("Chrome_WidgetWin_1");
        assert_eq!(profile, AppImeProfile::Imm32Unavailable);
        assert!(
            !is_effectively_tsf_native(profile, "Chrome_WidgetWin_1"),
            "Chrome は IMM32Unavailable であって TSF ネイティブではない"
        );
    }

    #[test]
    fn standard_class_is_not_effectively_tsf_native() {
        let profile = AppImeProfile::from_class_name("Notepad");
        assert_eq!(profile, AppImeProfile::Standard);
        assert!(!is_effectively_tsf_native(profile, "Notepad"));
    }
}
