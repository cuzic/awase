//! ウィンドウクラス名による分類定数と判定関数。
//!
//! `classify.rs`・`ime.rs`・`focus_observer.rs` で重複していたクラス名リストと
//! 判定ロジックを一元管理する。

use crate::focus::AppKind;

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
    "TeamsWebView",
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
    match profile {
        AppImeProfile::Standard | AppImeProfile::Imm32Unavailable => {
            is_tsf_native_window(class_name)
        }
        AppImeProfile::InputRelay => false,
        AppImeProfile::TsfNative => true,
    }
}

/// このプロファイルで、awase が実 OS IME ON/OFF 状態を確実に問い合わせられないか。
///
/// `Imm32Unavailable`（Chrome/Edge 等、VK_KANJI 制御）と実質 TSF ネイティブ
/// （WezTerm/Windows Terminal 等）はいずれも `ImmGet*` 系 API が使えないか
/// 不安定なため、belief（shadow state）が実状態と乖離しても自力では気付けない
/// （BUG-33/BUG-37 参照）。
#[must_use]
pub fn cannot_verify_real_ime_state(profile: AppImeProfile, class_name: &str) -> bool {
    match profile {
        AppImeProfile::Standard => is_effectively_tsf_native(profile, class_name),
        // InputRelay は is_effectively_tsf_native 経由ではなく独立したアームで
        // true にする（issue #136 / BUG-90 決定4）——is_effectively_tsf_native
        // 側を true にすると、その6箇所の本番 consumer（TSF warmup/composition
        // 再初期化ロジック等）が中継ウィンドウにも誤って走ってしまう。
        AppImeProfile::Imm32Unavailable | AppImeProfile::TsfNative | AppImeProfile::InputRelay => {
            true
        }
    }
}

/// BUG-37: 同一プロセス内フォーカス移動（Ctrl+T 新規タブ等）のような、belief を
/// 一切更新しない軽量フォーカスイベントで、次の入力に備えて composition を
/// 再プライム（cold mark）すべきかどうかを判定する純粋関数。
///
/// `cannot_verify_real_ime_state` なプロファイルで belief（`effective_open`）が
/// 既に ON のときだけ true を返す。この種のプロファイルでは、実状態が belief と
/// 無断で乖離しても唯一の訂正チャネルである物理 IME キー押下すら shadow-toggle の
/// no-op（belief==要求値なら何もしない、`kp_stage_shadow_ime_toggle` 参照）に
/// 握り潰されるため、フォーカス移動のたびに「次の入力で再プライムする」フラグを
/// 立てておくことで実状態を belief に追従させる。
#[must_use]
pub fn should_reprime_on_lightweight_focus_sync(
    profile: AppImeProfile,
    class_name: &str,
    belief_effective_open: bool,
) -> bool {
    cannot_verify_real_ime_state(profile, class_name) && belief_effective_open
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
    /// 入力中継ツール（`app_overrides.input_relay_apps` にプロセス名で登録）。
    /// IME actuation を所有せず、物理モードキーも suppress せず、
    /// この窓由来の open 観測も belief に取り込まない。
    InputRelay,
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

    /// クラス名 + プロセス名からプロファイルを決定する（`input_relay_apps`
    /// マッチを最優先）。
    #[must_use]
    pub fn from_class_and_process(
        class_name: &str,
        process_name: &str,
        relay_apps: &[String],
    ) -> Self {
        // `matches_disabled_app` は名前どおり `disable_apps` 用の関数だが、
        // 判定ロジック（大小無視・`.exe` 有無吸収）自体は `disable_apps` に
        // 固有ではないため、`input_relay_apps` 側でも再利用する（issue #136 /
        // BUG-90 決定4）。「無効化」ではなく「IME 制御の非所有」という意味は
        // `InputRelay` variant のドキュメントで表現し、この関数名は変更しない
        // （新しい照合ロジックを増やさないことを優先）。
        if crate::state::app_suppression::matches_disabled_app(relay_apps, process_name) {
            return Self::InputRelay;
        }
        Self::from_class_name(class_name)
    }

    /// `relay_apps` が空なら `process_name` の解決自体を省略する
    /// （`get_process_name` は Win32 プロセスハンドルを開くため高コスト。
    /// `ime.rs::read_ime_state_fast` と `runtime/mod.rs::on_window_focus_event`
    /// にあった同一の分岐ロジックの重複を解消（`/code-review` 指摘）。
    /// `process_name` を遅延クロージャで受けるのは、2箇所の呼び出し元で
    /// pid の取得済み状況が異なる（片方は既に pid 取得済み、片方は hwnd から
    /// 遅延取得）ため、呼び出し元に取得方法の選択を残すため。
    #[must_use]
    pub fn resolve(
        class_name: &str,
        relay_apps: &[String],
        process_name: impl FnOnce() -> String,
    ) -> Self {
        if relay_apps.is_empty() {
            Self::from_class_name(class_name)
        } else {
            Self::from_class_and_process(class_name, &process_name(), relay_apps)
        }
    }

    /// IMM32 クロスプロセス制御（`ImmSetOpenStatus` / `WM_IME_CONTROL`）が使えるか。
    ///
    /// `false` のとき `WindowsPlatform::set_ime_open` や `ImmCrossProcessStrategy`
    /// は IMM32 クロスプロセス呼び出しをスキップする。
    #[must_use]
    pub const fn can_use_imm32_cross_process(&self) -> bool {
        match self {
            Self::Standard => true,
            Self::Imm32Unavailable | Self::TsfNative | Self::InputRelay => false,
        }
    }

    /// VK_KANJI トグルキーで IME を制御するプロファイルか。
    ///
    /// `Imm32Unavailable`（Chrome/Edge 等）のみ `true`。
    /// GJI 稼働時は `GjiDirectStrategy`（VK_IME_ON/OFF）が優先されるため、
    /// このフラグは主に `send_engine_state_ime_key` での mode-key 送信スキップ判定に使用する。
    #[must_use]
    pub const fn uses_kanji_toggle(&self) -> bool {
        match self {
            Self::Imm32Unavailable => true,
            Self::Standard | Self::TsfNative | Self::InputRelay => false,
        }
    }

    /// 物理 IME キー（VK_KANJI / 半角/全角 等）を OS に届けてよいか。
    ///
    /// `Imm32Unavailable` アプリでは `apply_ime_open` が VK_KANJI を送信済みなので、
    /// 物理キーをそのまま届けると二重制御になる。`false` のとき
    /// `KeyEventPipeline::stage_execute` は `Decision::Consume` に変換する。
    #[must_use]
    pub const fn should_pass_physical_key(&self) -> bool {
        match self {
            Self::Standard | Self::TsfNative | Self::InputRelay => true,
            Self::Imm32Unavailable => false,
        }
    }

    /// IMM32 で IME open 状態（`IMC_GETOPENSTATUS`）をクロスプロセス取得できるか。
    ///
    /// `false` のとき `read_ime_state_fast` は `ime_on=None` を返し shadow 状態に委ねる。
    /// `Imm32Unavailable` / `TsfNative` ともに IMM32 の状態値は信頼できない。
    #[must_use]
    pub const fn can_read_imm32_open_status(&self) -> bool {
        match self {
            Self::Standard => true,
            Self::Imm32Unavailable | Self::TsfNative | Self::InputRelay => false,
        }
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
            // InputRelay は Standard と同じ ImmCross に写す（issue #136 / BUG-90
            // 決定4）。actuation の合流点3箇所（ime_controller.rs::
            // ImeController::apply / runtime/open_chain.rs::
            // run_open_chain_async / runtime/open_chain.rs::fallback_write）
            // の gate が先に効いて ImeOpenOutcome::NotOwned を返すため、
            // caps() が持つこの写像先の chain は実行時には使われない
            // （Plain/Unknown と同じ「到達しない安全既定」パターン）。
            //
            // 注: 当初は runtime/executor.rs::dispatch_ime_set_open の1点だけで
            // 足りると判断したが、runtime/key_pipeline.rs の shadow-toggle 経路
            // （物理 IME キー押下 → IME OFF/ON、issue #136 の当該操作そのもの）
            // がそこを通らずバイパスし、物理キーの Allow 素通しと awase 自身の
            // actuate が同時に起きる二重 actuation（BUG-46型）を招いた
            // （コードレビューで発見・修正、docs/known-bugs.md BUG-90 追補・
            // ADR-119 参照）。gate をこの3箇所より減らす変更を検討する前に、
            // 必ずそちらを読むこと。
            AppImeProfile::Standard | AppImeProfile::InputRelay => Self::ImmCross,
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
/// - `Chrome_*` / `TeamsWebView`: Chromium 系（Chrome, Edge, Electron, VS Code, Teams 等）
/// - `MozillaWindowClass`: Firefox（Chromium と同様の入力処理）
/// - `Windows.UI.Core.CoreWindow` / `ApplicationFrameWindow` / `Windows.UI.Input.*`: UWP / XAML 系
/// - その他: Win32 クラシック（ヒューリスティックで Chrome に昇格する場合あり）
#[must_use]
pub fn detect_app_kind(class_name: &str) -> AppKind {
    let class_lower = class_name.to_ascii_lowercase();
    if class_lower.starts_with("chrome_")
        || class_lower == "teamswebview"
        || class_lower == "mozillawindowclass"
    {
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
    fn teams_webview_is_chromium_like_imm32_unavailable() {
        assert_eq!(
            AppImeProfile::from_class_name("TeamsWebView"),
            AppImeProfile::Imm32Unavailable
        );
        assert_eq!(detect_app_kind("TeamsWebView"), AppKind::TsfNative);
    }

    #[test]
    fn standard_class_is_not_effectively_tsf_native() {
        let profile = AppImeProfile::from_class_name("Notepad");
        assert_eq!(profile, AppImeProfile::Standard);
        assert!(!is_effectively_tsf_native(profile, "Notepad"));
    }

    // BUG-37 回帰テスト: Ctrl+T 新規タブ等の同一プロセス内フォーカス移動で、
    // belief=ON かつ実状態を問い合わせられないプロファイルのときだけ再プライムする。

    #[test]
    fn chrome_with_belief_on_should_reprime() {
        let profile = AppImeProfile::from_class_name("Chrome_WidgetWin_1");
        assert!(should_reprime_on_lightweight_focus_sync(
            profile,
            "Chrome_WidgetWin_1",
            true,
        ));
    }

    #[test]
    fn chrome_with_belief_off_should_not_reprime() {
        // belief=OFF なら実状態も OFF のはずで、余計な IME ON 化を起こさない。
        let profile = AppImeProfile::from_class_name("Chrome_WidgetWin_1");
        assert!(!should_reprime_on_lightweight_focus_sync(
            profile,
            "Chrome_WidgetWin_1",
            false,
        ));
    }

    #[test]
    fn cascadia_with_belief_on_should_reprime_via_effectively_tsf_native() {
        // Windows Terminal は profile=Imm32Unavailable だが is_effectively_tsf_native 経由で対象。
        let profile = AppImeProfile::from_class_name("CASCADIA_HOSTING_WINDOW_CLASS");
        assert!(should_reprime_on_lightweight_focus_sync(
            profile,
            "CASCADIA_HOSTING_WINDOW_CLASS",
            true,
        ));
    }

    #[test]
    fn wezterm_with_belief_on_should_reprime() {
        let profile = AppImeProfile::from_class_name("org.wezfurlong.wezterm");
        assert!(should_reprime_on_lightweight_focus_sync(
            profile,
            "org.wezfurlong.wezterm",
            true,
        ));
    }

    #[test]
    fn standard_class_with_belief_on_should_not_reprime() {
        // 通常の IMM32 アプリは実状態を問い合わせられるので、この機構は不要。
        let profile = AppImeProfile::from_class_name("Notepad");
        assert!(!should_reprime_on_lightweight_focus_sync(
            profile, "Notepad", true,
        ));
    }

    /// `AppImeProfile` の4つの bool getter（戦略選択の入口）を真理値表として固定する。
    /// これまで戻り値を直接 assert するテストが無く、反転（`matches!`/`!matches!` の
    /// 取り違え）が mutants で検知されなかった。
    #[test]
    fn app_ime_profile_getters_truth_table() {
        // (profile, can_use_imm32_cross_process, uses_kanji_toggle,
        //  should_pass_physical_key, can_read_imm32_open_status)
        let table = [
            (AppImeProfile::Standard, true, false, true, true),
            (AppImeProfile::Imm32Unavailable, false, true, false, false),
            (AppImeProfile::TsfNative, false, false, true, false),
            (AppImeProfile::InputRelay, false, false, true, false),
        ];
        for (profile, cross_process, kanji_toggle, pass_physical, read_open_status) in table {
            assert_eq!(
                profile.can_use_imm32_cross_process(),
                cross_process,
                "{profile:?}.can_use_imm32_cross_process()"
            );
            assert_eq!(
                profile.uses_kanji_toggle(),
                kanji_toggle,
                "{profile:?}.uses_kanji_toggle()"
            );
            assert_eq!(
                profile.should_pass_physical_key(),
                pass_physical,
                "{profile:?}.should_pass_physical_key()"
            );
            assert_eq!(
                profile.can_read_imm32_open_status(),
                read_open_status,
                "{profile:?}.can_read_imm32_open_status()"
            );
        }
    }

    /// `From<AppImeProfile> for ImePolicyProfile` の3分岐変換を固定する。
    #[test]
    fn app_ime_profile_converts_to_expected_policy_profile() {
        use crate::state::ime_event::ImePolicyProfile;
        assert_eq!(
            ImePolicyProfile::from(AppImeProfile::Standard),
            ImePolicyProfile::ImmCross
        );
        assert_eq!(
            ImePolicyProfile::from(AppImeProfile::Imm32Unavailable),
            ImePolicyProfile::Imm32Unavailable
        );
        assert_eq!(
            ImePolicyProfile::from(AppImeProfile::TsfNative),
            ImePolicyProfile::TsfNative
        );
        assert_eq!(
            ImePolicyProfile::from(AppImeProfile::InputRelay),
            ImePolicyProfile::ImmCross
        );
    }

    #[test]
    fn input_relay_predicates_are_explicit() {
        let profile = AppImeProfile::InputRelay;
        assert!(!profile.can_use_imm32_cross_process());
        assert!(!profile.uses_kanji_toggle());
        assert!(profile.should_pass_physical_key());
        assert!(!profile.can_read_imm32_open_status());
        assert!(!is_effectively_tsf_native(
            profile,
            "Windows.UI.Input.InputSite.WindowClass"
        ));
        assert!(cannot_verify_real_ime_state(profile, "Notepad"));
        assert!(should_reprime_on_lightweight_focus_sync(
            profile, "Notepad", true
        ));
    }

    #[test]
    fn is_chromium_widget_table() {
        assert!(is_chromium_widget("Chrome_WidgetWin_1"));
        assert!(is_chromium_widget("MozillaWindowClass"));
        assert!(!is_chromium_widget("Notepad"));
    }

    /// `detect_app_kind` の3分岐を固定する。`teamswebview_is_chromium_like_imm32_unavailable`
    /// が TsfNative 側は部分的にカバーしているが、Uwp 分岐（CoreWindow/ApplicationFrameWindow/
    /// InputSite）と Win32 フォールバックには直接のテストが無かった。
    #[test]
    fn detect_app_kind_table() {
        let table = [
            ("Chrome_WidgetWin_1", AppKind::TsfNative),
            ("MozillaWindowClass", AppKind::TsfNative),
            ("Windows.UI.Core.CoreWindow", AppKind::Uwp),
            ("ApplicationFrameWindow", AppKind::Uwp),
            ("Windows.UI.Input.InputSite.WindowClass", AppKind::Uwp),
            ("Notepad", AppKind::Win32),
        ];
        for (class_name, expected) in table {
            assert_eq!(detect_app_kind(class_name), expected, "{class_name}");
        }
    }

    // ── クラスタ全体の組合せ網羅 + 独立オラクル ─────────────────────────────
    //
    // `AppImeProfile::from_class_name` / `is_effectively_tsf_native` /
    // `cannot_verify_real_ime_state` / `should_reprime_on_lightweight_focus_sync` /
    // 4つの getter は、いずれも「IMM32制御不可クラスか」「TSFネイティブクラスか」の
    // 2値から導出される派生関数のクラスタである。この2値の掛け合わせ(4通り)を
    // 個別に見るテストがこれまで無く、実際に CASCADIA_HOSTING_WINDOW_CLASS
    // （両方に該当）のケースで `profile == AppImeProfile::TsfNative` という直接比較
    // （4通りのうち「両方該当」を取りこぼす配線ミス）が実機バグを起こした
    // （このファイル冒頭の回帰テスト、2026-07-05）。同種の配線ミスをクラスタ全体で
    // 恒久的に検知するため、本体コードを見ずに doc コメントの規則から独立に
    // 書き起こしたオラクルで全4バケット×belief_effective_open(2)=8通りを突き合わせる。

    /// (is_imm32_unavailable, is_tsf_native_class) の4バケットから、クラスタ全体が
    /// 返すべき値を独立に導出する。
    ///
    /// - `profile`: unavailable が最優先（1）、次に tsf_native（2）、他は Standard（3）
    ///   — `from_class_name` の doc コメント「優先順」をそのまま転記。
    /// - `effectively_tsf_native`: is_tsf_native_class そのもの（`profile==TsfNative` は
    ///   `is_tsf_native_class && !is_unavailable` の場合にしか成立しないため、
    ///   `profile==TsfNative || is_tsf_native_class` を展開すると is_tsf_native_class に潰れる）。
    /// - `cannot_verify`: unavailable か、実質 TSF ネイティブかのいずれか。
    /// - 4つの getter は `profile` のみで決まる真理値表（`app_ime_profile_getters_truth_table`
    ///   と同じ表だが、独立に書き起こす）。
    fn oracle_for(
        is_unavailable: bool,
        is_tsf_native_class: bool,
        is_input_relay: bool,
    ) -> OracleResult {
        let profile = if is_input_relay {
            AppImeProfile::InputRelay
        } else if is_unavailable {
            AppImeProfile::Imm32Unavailable
        } else if is_tsf_native_class {
            AppImeProfile::TsfNative
        } else {
            AppImeProfile::Standard
        };
        let effectively_tsf_native = !is_input_relay && is_tsf_native_class;
        let cannot_verify = is_input_relay || is_unavailable || effectively_tsf_native;
        // TsfNative と InputRelay はこの4述語の期待値がたまたま一致するが、
        // このオラクルは profile ごとの行を明示的に書き下す真理値表として
        // 意図的に保つ（clippy::match_same_arms の統合提案には従わない —
        // 統合すると「どの profile がどの行か」が読みにくくなる）。
        #[allow(clippy::match_same_arms)]
        let (
            can_use_imm32_cross_process,
            uses_kanji_toggle,
            should_pass_physical_key,
            can_read_imm32_open_status,
        ) = match profile {
            AppImeProfile::Standard => (true, false, true, true),
            AppImeProfile::Imm32Unavailable => (false, true, false, false),
            AppImeProfile::TsfNative => (false, false, true, false),
            AppImeProfile::InputRelay => (false, false, true, false),
        };
        OracleResult {
            profile,
            effectively_tsf_native,
            cannot_verify,
            can_use_imm32_cross_process,
            uses_kanji_toggle,
            should_pass_physical_key,
            can_read_imm32_open_status,
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct OracleResult {
        profile: AppImeProfile,
        effectively_tsf_native: bool,
        cannot_verify: bool,
        can_use_imm32_cross_process: bool,
        uses_kanji_toggle: bool,
        should_pass_physical_key: bool,
        can_read_imm32_open_status: bool,
    }

    fn actual_for(class_name: &str, process_name: &str, relay_apps: &[String]) -> OracleResult {
        let profile = AppImeProfile::from_class_and_process(class_name, process_name, relay_apps);
        OracleResult {
            profile,
            effectively_tsf_native: is_effectively_tsf_native(profile, class_name),
            cannot_verify: cannot_verify_real_ime_state(profile, class_name),
            can_use_imm32_cross_process: profile.can_use_imm32_cross_process(),
            uses_kanji_toggle: profile.uses_kanji_toggle(),
            should_pass_physical_key: profile.should_pass_physical_key(),
            can_read_imm32_open_status: profile.can_read_imm32_open_status(),
        }
    }

    /// 4バケットそれぞれを代表するクラス名。
    /// - 両方該当: `CASCADIA_HOSTING_WINDOW_CLASS`（実機バグの震源、回帰テスト参照）
    /// - unavailable のみ: `Chrome_WidgetWin_1`（`is_tsf_native_window` には非該当）
    /// - tsf_native のみ: `org.wezfurlong.wezterm`（`IMM32_UNAVAILABLE_CLASSES` には非該当）
    /// - どちらも非該当: `Notepad`
    #[test]
    fn exhaustive_cluster_matches_independent_oracle() {
        let relay_apps = vec!["relay.exe".to_string()];
        let buckets: [(&str, &str, bool, bool, bool); 5] = [
            (
                "CASCADIA_HOSTING_WINDOW_CLASS",
                "app.exe",
                true,
                true,
                false,
            ),
            ("Chrome_WidgetWin_1", "app.exe", true, false, false),
            ("org.wezfurlong.wezterm", "app.exe", false, true, false),
            ("Notepad", "app.exe", false, false, false),
            (
                "CASCADIA_HOSTING_WINDOW_CLASS",
                "relay.exe",
                true,
                true,
                true,
            ),
        ];

        let mut mismatches = Vec::new();
        for (class_name, process_name, is_unavailable, is_tsf_native_class, is_input_relay) in
            buckets
        {
            // バケット分類そのものが実装の集合と一致しているかも確認する
            // （でなければ以降の突き合わせが無意味になる）。
            assert_eq!(
                IMM32_UNAVAILABLE_CLASSES.contains(&class_name),
                is_unavailable,
                "{class_name}: IMM32_UNAVAILABLE_CLASSES 該当の想定違い"
            );
            assert_eq!(
                is_tsf_native_window(class_name),
                is_tsf_native_class,
                "{class_name}: is_tsf_native_window 該当の想定違い"
            );

            let expected = oracle_for(is_unavailable, is_tsf_native_class, is_input_relay);
            let actual = actual_for(class_name, process_name, &relay_apps);
            if actual != expected {
                mismatches.push(format!(
                    "{class_name}: actual={actual:?} expected(oracle)={expected:?}"
                ));
            }

            // should_reprime_on_lightweight_focus_sync は cannot_verify && belief_open。
            for &belief_open in &[false, true] {
                let expected_reprime = expected.cannot_verify && belief_open;
                let actual_reprime = should_reprime_on_lightweight_focus_sync(
                    actual.profile,
                    class_name,
                    belief_open,
                );
                if actual_reprime != expected_reprime {
                    mismatches.push(format!(
                        "{class_name} belief_open={belief_open}: reprime actual={actual_reprime} \
                         expected(oracle)={expected_reprime}"
                    ));
                }
            }
        }
        assert!(
            mismatches.is_empty(),
            "{} 件不一致:\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }

    #[test]
    fn from_class_and_process_prioritizes_input_relay_over_class_profile() {
        let relay_apps = vec!["powertoys.mousewithoutbordershelper.exe".to_string()];
        assert_eq!(
            AppImeProfile::from_class_and_process(
                "Chrome_WidgetWin_1",
                "PowerToys.MouseWithoutBordersHelper.exe",
                &relay_apps,
            ),
            AppImeProfile::InputRelay
        );
    }

    #[test]
    fn from_class_and_process_matches_from_class_name_when_relay_unmatched() {
        let relay_apps = vec!["relay.exe".to_string()];
        for class_name in [
            "CASCADIA_HOSTING_WINDOW_CLASS",
            "Chrome_WidgetWin_1",
            "org.wezfurlong.wezterm",
            "Notepad",
        ] {
            assert_eq!(
                AppImeProfile::from_class_and_process(class_name, "app.exe", &relay_apps),
                AppImeProfile::from_class_name(class_name),
                "{class_name}"
            );
            assert_eq!(
                AppImeProfile::from_class_and_process(class_name, "app.exe", &[]),
                AppImeProfile::from_class_name(class_name),
                "{class_name} empty relay list"
            );
        }
    }

    /// `IMM32_UNAVAILABLE_CLASSES` の全メンバーが優先順位1（unavailable最優先）通りに
    /// 分類されることを実リストに対して全数確認する（バケット代表1件だけでなく、
    /// リストに新しいクラス名が追加された将来の変更に対しても効く）。
    #[test]
    fn all_imm32_unavailable_classes_are_classified_as_unavailable() {
        for &class_name in IMM32_UNAVAILABLE_CLASSES {
            assert_eq!(
                AppImeProfile::from_class_name(class_name),
                AppImeProfile::Imm32Unavailable,
                "{class_name}"
            );
            assert!(
                cannot_verify_real_ime_state(
                    AppImeProfile::from_class_name(class_name),
                    class_name
                ),
                "{class_name}: unavailable は常に cannot_verify=true のはず"
            );
        }
    }
}
