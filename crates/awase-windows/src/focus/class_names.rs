//! ウィンドウクラス名による分類定数と判定関数。
//!
//! `classify.rs`・`ime.rs`・`focus_observer.rs` で重複していたクラス名リストと
//! 判定ロジックを一元管理する。

use awase::types::AppKind;

/// IMM ブリッジ（WM_IME_CONTROL）が動作しない、または不安定なウィンドウクラス。
///
/// これらのクラスにフォーカスがあるとき、`ImmGet*` / `SendMessage(WM_IME_CONTROL)` は
/// 反応しなかったり無期限にブロックする恐れがあるため、IME 状態検出をスキップする。
/// シャドウ状態（hook から追跡）のみで IME 状態を管理する。
///
/// 検知できないケース:
/// - 言語バーのマウス操作による IME 切り替え
/// - アプリ内の IME ボタンクリック
///   しかし、これらは非常に稀なので割り切る。
pub const IMM_BRIDGE_BROKEN_CLASSES: &[&str] = &[
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

/// 指定クラスが IMM ブリッジ非対応かどうか判定する。
#[must_use]
pub fn is_imm_bridge_broken(class_name: &str) -> bool {
    IMM_BRIDGE_BROKEN_CLASSES.contains(&class_name)
}

/// 指定クラスが TSF ネイティブウィンドウかどうか判定する。
///
/// TSF ネイティブウィンドウでは `ImmGetContext` が NULL を返すが、
/// これは IME が OFF であることを意味しない（TSF text store で直接管理）。
/// 対象:
/// - Windows.UI.Core.CoreWindow: UWP / WinUI
/// - XamlExplorerHostIslandWindow: XAML ホスト
/// - Windows.UI.Input.InputSite.WindowClass: Windows Terminal の InputSite 子ウィンドウ
/// - CASCADIA_HOSTING_WINDOW_CLASS: Cascadia（Windows Terminal 上位ホスト）
#[must_use]
pub fn is_tsf_native_window(class_name: &str) -> bool {
    matches!(
        class_name,
        "Windows.UI.Core.CoreWindow"
            | "XamlExplorerHostIslandWindow"
            | "Windows.UI.Input.InputSite.WindowClass"
            | "CASCADIA_HOSTING_WINDOW_CLASS"
    )
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
