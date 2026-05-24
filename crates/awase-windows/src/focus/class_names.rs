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
const IMM_BRIDGE_BROKEN_CLASSES: &[&str] = &[
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

// ── AppImeProfile ──────────────────────────────────────────────

/// フォーカス中アプリの IME 制御プロファイル。
///
/// 「Chrome/Edge 等は IMM API が効かない」「TSF ネイティブは VK_DBE_HIRAGANA が必要」
/// といったアプリ別の特性を 1 つの型に集約し、「クラス名で個別判定」の散在を防ぐ。
/// フォーカス変更時に `from_class_name` で決定して
/// `AppKindClassifier.current_app_profile` にキャッシュし、
/// `current_app_profile()` メソッドで参照する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppImeProfile {
    /// 通常の IMM32 アプリ。IMM API 直接操作可能。
    #[default]
    Standard,
    /// Chrome/Edge 等。IMM API が効かず VK_KANJI で制御、物理キーを抑止する必要がある。
    ImmBroken,
    /// TSF ネイティブ（例: WezTerm/UWP）。`VK_DBE_HIRAGANA` + TSF probe が必要。
    TsfNative,
}

impl AppImeProfile {
    /// クラス名からプロファイルを決定する。
    ///
    /// 優先順:
    /// 1. IMM-broken クラス（Chrome/Edge/UWP/XAML/Console 系）→ `ImmBroken`
    /// 2. TSF ネイティブ専用クラス → `TsfNative`
    /// 3. その他 → `Standard`
    ///
    /// 注: UWP/XAML/Console 系クラスは IMM-broken にも TSF-native にも該当するが、
    /// IME 制御フロー（VK_KANJI + 物理キー抑止）を優先するため `ImmBroken` を返す。
    #[must_use]
    pub fn from_class_name(class_name: &str) -> Self {
        if IMM_BRIDGE_BROKEN_CLASSES.contains(&class_name) {
            Self::ImmBroken
        } else if is_tsf_native_window(class_name) {
            Self::TsfNative
        } else {
            Self::Standard
        }
    }

    /// `IMM32` API で open/close を直接操作できるか。
    ///
    /// `false` のとき `WindowsPlatform::set_ime_open` や `ImmCrossProcessStrategy`
    /// は `ImmSetOpenStatus`/`WM_IME_CONTROL` クロスプロセス呼び出しをスキップする。
    #[must_use]
    pub const fn can_use_imm_direct(&self) -> bool {
        matches!(self, Self::Standard)
    }

    /// VK_KANJI トグルキーで IME を制御するか。
    ///
    /// IMM-broken アプリ（Chrome/Edge 等）の主戦略。`KanjiToggleStrategy` が
    /// この値を `true` にしてフォールバックではなく主経路として VK_KANJI を送る。
    #[must_use]
    pub const fn uses_kanji_toggle(&self) -> bool {
        matches!(self, Self::ImmBroken)
    }

    /// 物理 IME キー（VK_KANJI / 半角/全角 等）を OS に届けてよいか。
    ///
    /// IMM-broken アプリでは `apply_ime_open` が VK_KANJI を送信済みなので、
    /// 物理キーをそのまま届けると二重制御になる。`false` のとき
    /// `KeyEventPipeline::stage_execute` は `Decision::Consume` に変換する。
    #[must_use]
    pub const fn should_pass_physical_key(&self) -> bool {
        !matches!(self, Self::ImmBroken)
    }

    /// IMM で IME open 状態（IMC_GETOPENSTATUS）をクロスプロセス取得できるか。
    ///
    /// `false` のとき `read_ime_state_fast` は `ime_on=None` を返し shadow 状態に委ねる。
    /// IMM-broken / TSF-native ともに信頼できない。
    #[must_use]
    pub const fn can_read_imm_state(&self) -> bool {
        matches!(self, Self::Standard)
    }
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
