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

/// ADR-270 の確認(Mozc の最初の送信の後に入力中の文字ができたか。0.3 秒で判断して、できていなければ
/// Backspace で消して送り直す)を張ってよい前景窓のクラスか。
///
/// 旧 Tsf 注入モードの既定の対象(`force_tsf` の Windows Terminal・WezTerm)だけに限る。確認は旧来この窓でしか
/// 張られず、期限 0.3 秒(`MOZC_COMPOSE_CHECK_MS`、実測は Windows Terminal の 1 台だけ)の根拠もこの窓にしか無い。
/// Chrome は TSF composition context の再初期化に ~326ms かかる実測(`79134f5`)があり、期限を越えて遅れて
/// できた入力中の文字を Backspace が消し、確定済みの文字まで消しうる(PR #809 レビュー M3)。広げるときは、
/// その窓で `[mozc-compose] ... confirmed after Nms` の分布を取ってからにする。
#[must_use]
pub fn is_mozc_compose_check_window(foreground_class: &str) -> bool {
    matches!(
        foreground_class,
        "CASCADIA_HOSTING_WINDOW_CLASS" | "org.wezfurlong.wezterm"
    )
}

/// ADR-270 の確認を張るか(Mozc 系 × WT/WezTerm の窓)。
///
/// `mozc_compose_precheck` の前段と、物理 F2 で warm でも cold にするか(`physical_f2_marks_cold` の
/// 第 2 引数)を、同じ述語で決める(PR #809 再レビュー N1・N2)。
/// Mozc 系(`gji_evidence_absent`)で、前景窓が [`is_mozc_compose_check_window`] のときだけ真。
/// 範囲を広げるときはここ 1 か所を直す。
#[must_use]
pub fn mozc_compose_check_applies(gji_evidence_absent: bool, foreground_class: &str) -> bool {
    gji_evidence_absent && is_mozc_compose_check_window(foreground_class)
}

/// per-VK の確認の「2 番目以降の疑い」を信用できない窓のクラスか(Zoom のチャット)。
///
/// per-VK の確認は、打鍵の約 313ms 後に GJI の書き込みか候補窓が見えるかで literal を判定する。Zoom のチャットでは
/// 正しく入った文字でもこの証拠が来ない。実機(dragonflyg4、GJI、2026-10-24)で、ポップアップ
/// (`ZConfChatPopupContainerWndClass`)でもメイン会議ウィンドウ内のチャットタブ(`ConfMultiTabContentWndClass`)でも、
/// 2 番目以降のキー(`ki` の `i`)が毎回疑われた。疑いが出ると ESC を送って全体を再送し、1 打鍵で `ki` が 6 回再送された。
/// 直前のキーは確認済みなので、そこで literal になる根拠は弱い。1 番目のキーの判定(IME ON 直後の本物のリテラル化を
/// 実際に検知できた)は変えない。ここに足すのは、誤判定を実測したクラスだけにする。
#[must_use]
pub fn is_later_vk_literal_check_unreliable_window(foreground_class: &str) -> bool {
    matches!(
        foreground_class,
        "ZConfChatPopupContainerWndClass" | "ConfMultiTabContentWndClass"
    )
}

/// 注入したキーの「↓から↑まで」に時間を空けないと、キーを取りこぼす窓のクラスか(Zoom のチャット)。
///
/// awase は、ローマ字を `SendInput` で注入する。↓と↑の間隔がほぼ 0 だと、Zoom のチャット入力欄は、
/// 一部のキー(主に母音)を取りこぼした(実機 dragonflyg4、GJI、2026-10-24。`かくにんをおねがいします` が
/// `かｋにんをんぎします。` になる)。ユーザーの実際の打鍵を awase を通して再生し、入力欄を読んで比べた
/// (各 8 回): 従来 0/8、キーを順に送って押している時間を 5ms にすると 2/8、10ms にすると 8/8。キーを重ねて押すと、
/// 待っても 0/8。人間の打鍵は押している時間が数十 ms あるので、起きない。
/// 待つ長さは `tuning::ZOOM_KEY_HOLD_MS`。ここに足すのは、取りこぼしを実測した窓のクラスだけにする。
#[must_use]
pub fn needs_key_hold_window(foreground_class: &str) -> bool {
    matches!(
        foreground_class,
        "ZConfChatPopupContainerWndClass" | "ConfMultiTabContentWndClass"
    )
}

/// Win32 のポップアップメニュー窓（`TrackPopupMenu` 等が作るコンテキストメニュー）のクラス名。
///
/// どのアプリの右クリックメニューも、この標準クラスの窓として出る。入力インジケーターの
/// 右クリックメニュー（IME が TSF 経由で出す）も同じ。
pub const POPUP_MENU_WINDOW_CLASS: &str = "#32768";

/// ポップアップメニュー窓（[`POPUP_MENU_WINDOW_CLASS`]）か。
///
/// メニューの項目がハイライトされる（マウスのホバー・キーボード）と、メニュー窓へ
/// `EVENT_OBJECT_FOCUS` が出る。**メニューは元の窓にフォーカスを残したまま別の窓（メニュー窓）へ
/// このイベントを出すだけで、入力先の窓は変わらない**。これを窓の切替として扱うと、
/// フォーカス窓の分類（`HwndCache`・UIA）が走る（BUG-202、ADR-264 決定 7。当時は旧 TsfGate が
/// `PendingWarmup` に戻って保持中の打鍵も捨てられたが、TsfGate は 2026-10-22 に撤去した）。
#[must_use]
pub fn is_popup_menu_window_class(class_name: &str) -> bool {
    class_name == POPUP_MENU_WINDOW_CLASS
}

/// awase 自身の UI ウィンドウ（トレイ／設定画面）か。
///
/// ここから読んだ IMM の conv/romaji 値は「ユーザーが編集しているアプリの入力方式」
/// ではないため、`input_mode` belief に採用してはならない（BUG-106 追補3・4）。
/// トレイ・設定画面はどちらも `AppImeProfile::Standard` に分類される（Chrome 等の
/// `Imm32Unavailable` とは別軸）ため、`AppImeProfile` では判別できない。
///
/// `process_name` は `FocusTracker::process_name()`（小文字）を渡すこと——
/// awase-settings は winit 既定の `"Window Class"` という一般的なクラス名を使うため、
/// クラス名だけでは無関係な winit アプリと区別できない
/// （`tray.rs:132-138` の doc が記録している同型の直書き結合、"awase-settings.exe"
/// はここにしか出現しないリテラルなので変更時は grep で追従すること）。
///
/// `crate::tray::WINDOW_CLASS_NAME` を直接参照しない: `tray` モジュールは
/// `#[cfg(windows)]` だが、この `focus` モジュールは Linux ホストでもテストされる
/// （`architecture_guard`/`layer_boundary_guard`）ため、文字列リテラルを直書きする
/// （`tray.rs:138`の値と手で同期させること、変更時はこのdocも更新する）。
#[must_use]
pub fn is_own_ui_window(class_name: &str, process_name: &str) -> bool {
    class_name == "awase_tray_window" || process_name == "awase-settings.exe"
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
#[derive(
    strum::IntoStaticStr,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
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
    /// **読み(`IMC_GETOPENSTATUS`)も同じ述語**: 書けない窓(`Imm32Unavailable`/`TsfNative`/`InputRelay`)は IMM32 の状態値も
    /// 信頼できず、実 IME の ON/OFF を確実に問い合わせられない(belief が実状態と乖離しても自力では気付けない、BUG-33/37)。
    /// 旧 `can_read_imm32_open_status`・`cannot_verify_real_ime_state(class)` は本番で作られるプロファイルすべてでこれの
    /// 真偽と一致したので統合した(ECRS DD3-C、全列挙は `imm_capability_follows_profile_for_every_production_derivation`)。
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

    /// 物理 KANJI 系キーを awase 側で特別扱いするプロファイルか。
    ///
    /// `Imm32Unavailable`（Chrome/Edge 等）のみ `true`。
    /// 名前は古い。かつて `send_engine_state_ime_key` での mode-key 送信スキップ判定に使っていたが（ADR-207 で撤去）、
    /// 現在の本番の呼び出し元は無く、プロファイル分類の API とテストの oracle として残している。
    #[must_use]
    pub const fn uses_kanji_toggle(&self) -> bool {
        match self {
            Self::Imm32Unavailable => true,
            Self::Standard | Self::TsfNative | Self::InputRelay => false,
        }
    }

    /// `*self == AppImeProfile::TsfNative` の代わりに使うべき「実質的に TSF ネイティブか」判定。
    ///
    /// `AppImeProfile::from_class_name` は `IMM32_UNAVAILABLE_CLASSES` を `is_tsf_native_window`
    /// より優先して評価するため、CASCADIA_HOSTING_WINDOW_CLASS のような「両方に該当するクラス」
    /// では `AppImeProfile::TsfNative` が一切現れず、代わりに `Imm32Unavailable` になる
    /// （`from_class_name` のドキュメント参照）。そのため値だけを見て
    /// `matches!(profile, AppImeProfile::TsfNative)` と判定すると、Windows Terminal のような
    /// 実質 TSF ネイティブなウィンドウを取りこぼす（2026-07-05 実機ログで確認: フォーカス着地直後の
    /// "enforce IME OFF" ブロックが、Windows Terminal を非 TSF ネイティブと誤判定して発火した）。
    ///
    /// 「このウィンドウは TSF ネイティブとして扱うべきか」を判定したい呼び出し元は、
    /// `*profile == AppImeProfile::TsfNative` ではなく必ずこのメソッドを使うこと。
    ///
    /// 2026-09-10、自由関数`is_effectively_tsf_native(profile, class_name)`から
    /// メソッドへ変更した（第1引数`AppImeProfile`をselfにせず取り続けていたため）。
    /// 挙動は変更していない。
    #[must_use]
    pub fn is_effectively_tsf_native(&self, class_name: &str) -> bool {
        match self {
            Self::Standard | Self::Imm32Unavailable => is_tsf_native_window(class_name),
            Self::InputRelay => false,
            Self::TsfNative => true,
        }
    }
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
    #[test]
    fn zoom_chat_windows_need_key_hold() {
        assert!(needs_key_hold_window("ZConfChatPopupContainerWndClass"));
        assert!(needs_key_hold_window("ConfMultiTabContentWndClass"));
        assert!(!needs_key_hold_window("Chrome_WidgetWin_1"));
        assert!(!needs_key_hold_window(""));
    }

    #[test]
    fn zoom_chat_windows_have_unreliable_later_vk_literal_check() {
        assert!(is_later_vk_literal_check_unreliable_window(
            "ZConfChatPopupContainerWndClass"
        ));
        assert!(is_later_vk_literal_check_unreliable_window(
            "ConfMultiTabContentWndClass"
        ));
        // Chrome は literal を実測で回収できている窓なので対象外。
        assert!(!is_later_vk_literal_check_unreliable_window(
            "Chrome_WidgetWin_1"
        ));
        assert!(!is_later_vk_literal_check_unreliable_window(""));
    }

    use super::*;

    /// ADR-270 の確認は旧 `force_tsf` の既定の窓(Windows Terminal・WezTerm)だけ(PR #809 レビュー M3)。
    #[test]
    fn mozc_compose_check_window_is_limited_to_old_force_tsf_defaults() {
        assert!(is_mozc_compose_check_window(
            "CASCADIA_HOSTING_WINDOW_CLASS"
        ));
        assert!(is_mozc_compose_check_window("org.wezfurlong.wezterm"));
        for other in [
            "Chrome_WidgetWin_1",
            "Chrome_RenderWidgetHostHWND",
            "OpusApp",
            "Notepad",
            "Windows.UI.Input.InputSite.WindowClass",
            "",
        ] {
            assert!(!is_mozc_compose_check_window(other), "{other}");
        }
    }

    /// Mozc × WT/WezTerm だけ真。GJI(証拠あり)や他の窓は偽(PR #809 再レビュー N1・N2)。
    #[test]
    fn mozc_compose_check_applies_table() {
        assert!(mozc_compose_check_applies(
            true,
            "CASCADIA_HOSTING_WINDOW_CLASS"
        ));
        assert!(mozc_compose_check_applies(true, "org.wezfurlong.wezterm"));
        assert!(!mozc_compose_check_applies(true, "Chrome_WidgetWin_1"));
        assert!(!mozc_compose_check_applies(true, "OpusApp"));
        assert!(!mozc_compose_check_applies(
            false,
            "CASCADIA_HOSTING_WINDOW_CLASS"
        ));
        assert!(!mozc_compose_check_applies(false, "Chrome_WidgetWin_1"));
    }

    // BUG-202: コンテキストメニュー（クラス `#32768`）へのフォーカスは窓の切替ではない。
    // 判定は完全一致(クラス名の部分一致で他の窓を巻き込まないこと)。
    #[test]
    fn popup_menu_window_class_is_exactly_the_win32_menu_class() {
        assert!(is_popup_menu_window_class("#32768"));
        assert_eq!(POPUP_MENU_WINDOW_CLASS, "#32768");
    }

    #[test]
    fn popup_menu_window_class_does_not_match_real_input_windows() {
        for class in [
            "",
            "Edit",
            "ScenWnd",
            "Chrome_WidgetWin_1",
            "CASCADIA_HOSTING_WINDOW_CLASS",
            "Windows.UI.Input.InputSite.WindowClass",
            // 前後に文字が付いたものは別の窓(完全一致であること)
            "#327680",
            " #32768",
            "#32768 ",
            // 別の標準クラス(ダイアログ #32770、デスクトップ #32769)
            "#32770",
            "#32769",
        ] {
            assert!(
                !is_popup_menu_window_class(class),
                "{class:?} はメニュー窓ではない"
            );
        }
    }

    // BUG-106追補3・4: awase自身のトレイ/設定画面から読んだ観測を、
    // ユーザーが編集中のアプリの入力方式として誤ってbeliefに採用していた。
    // 3件目（無関係なwinitアプリを排除しないこと）が本質——過剰排除の回帰を
    // 防ぐのはこのテストだけ。

    #[test]
    fn is_own_ui_window_matches_tray_by_class_name() {
        assert!(is_own_ui_window("awase_tray_window", "unrelated.exe"));
    }

    #[test]
    fn is_own_ui_window_matches_settings_by_process_name() {
        // awase-settings は winit 既定の一般的なクラス名 "Window Class" を使うため
        // プロセス名で判定する。
        assert!(is_own_ui_window("Window Class", "awase-settings.exe"));
    }

    #[test]
    fn is_own_ui_window_does_not_match_unrelated_winit_app() {
        // "Window Class" は winit の既定クラス名で、無関係な他社アプリも使いうる。
        // プロセス名が一致しない限り誤って排除してはならない。
        assert!(!is_own_ui_window("Window Class", "some_other_app.exe"));
    }

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
            profile.is_effectively_tsf_native("CASCADIA_HOSTING_WINDOW_CLASS"),
            "profile が Imm32Unavailable でも is_tsf_native_window で TSF ネイティブと判定できる"
        );
    }

    #[test]
    fn wezterm_is_tsf_native_directly_and_effectively() {
        let profile = AppImeProfile::from_class_name("org.wezfurlong.wezterm");
        assert_eq!(profile, AppImeProfile::TsfNative);
        assert!(profile.is_effectively_tsf_native("org.wezfurlong.wezterm"));
    }

    #[test]
    fn chrome_is_not_effectively_tsf_native() {
        let profile = AppImeProfile::from_class_name("Chrome_WidgetWin_1");
        assert_eq!(profile, AppImeProfile::Imm32Unavailable);
        assert!(
            !profile.is_effectively_tsf_native("Chrome_WidgetWin_1"),
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
        assert!(!profile.is_effectively_tsf_native("Notepad"));
    }

    // BUG-37 回帰テスト: Ctrl+T 新規タブ等の同一プロセス内フォーカス移動で、
    // belief=ON かつ実状態を問い合わせられない窓だけ再プライムする。「問い合わせられない」は
    // `!can_use_imm32_cross_process()`(旧 `cannot_verify_real_ime_state`、ECRS DD3-C で統合)。

    #[test]
    fn unverifiable_windows_are_exactly_the_non_standard_profiles() {
        for (class, expected_unverifiable) in [
            ("Chrome_WidgetWin_1", true),
            // Windows Terminal は profile=Imm32Unavailable(IMM32 不可クラスが TSF ネイティブより優先)。
            ("CASCADIA_HOSTING_WINDOW_CLASS", true),
            ("org.wezfurlong.wezterm", true),
            // 通常の IMM32 アプリは実状態を問い合わせられるので、再プライムの機構は不要。
            ("Notepad", false),
        ] {
            let profile = AppImeProfile::from_class_name(class);
            assert_eq!(
                !profile.can_use_imm32_cross_process(),
                expected_unverifiable,
                "{class}: {profile:?}"
            );
        }
    }

    /// `AppImeProfile` の2つの bool getter（戦略選択の入口）を真理値表として固定する。
    /// これまで戻り値を直接 assert するテストが無く、反転（`matches!`/`!matches!` の
    /// 取り違え）が mutants で検知されなかった。
    #[test]
    fn app_ime_profile_getters_truth_table() {
        // (profile, can_use_imm32_cross_process, uses_kanji_toggle)
        let table = [
            (AppImeProfile::Standard, true, false),
            (AppImeProfile::Imm32Unavailable, false, true),
            (AppImeProfile::TsfNative, false, false),
            (AppImeProfile::InputRelay, false, false),
        ];
        for (profile, cross_process, kanji_toggle) in table {
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
        }
    }

    #[test]
    fn input_relay_predicates_are_explicit() {
        let profile = AppImeProfile::InputRelay;
        assert!(!profile.can_use_imm32_cross_process());
        assert!(!profile.uses_kanji_toggle());
        assert!(!profile.is_effectively_tsf_native("Windows.UI.Input.InputSite.WindowClass"));
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
    // 2つの getter は、いずれも「IMM32制御不可クラスか」「TSFネイティブクラスか」の
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
    /// - 3つの getter は `profile` のみで決まる真理値表（`app_ime_profile_getters_truth_table`
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
        // TsfNative と InputRelay はこの2述語の期待値がたまたま一致するが、
        // このオラクルは profile ごとの行を明示的に書き下す真理値表として
        // 意図的に保つ（clippy::match_same_arms の統合提案には従わない —
        // 統合すると「どの profile がどの行か」が読みにくくなる）。
        #[allow(clippy::match_same_arms)]
        let (can_use_imm32_cross_process, uses_kanji_toggle) = match profile {
            AppImeProfile::Standard => (true, false),
            AppImeProfile::Imm32Unavailable => (false, true),
            AppImeProfile::TsfNative => (false, false),
            AppImeProfile::InputRelay => (false, false),
        };
        OracleResult {
            profile,
            effectively_tsf_native,
            can_use_imm32_cross_process,
            uses_kanji_toggle,
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct OracleResult {
        profile: AppImeProfile,
        effectively_tsf_native: bool,
        can_use_imm32_cross_process: bool,
        uses_kanji_toggle: bool,
    }

    fn actual_for(class_name: &str, process_name: &str, relay_apps: &[String]) -> OracleResult {
        let profile = AppImeProfile::from_class_and_process(class_name, process_name, relay_apps);
        OracleResult {
            profile,
            effectively_tsf_native: profile.is_effectively_tsf_native(class_name),
            can_use_imm32_cross_process: profile.can_use_imm32_cross_process(),
            uses_kanji_toggle: profile.uses_kanji_toggle(),
        }
    }

    /// 旧 `can_read_imm32_open_status`・`cannot_verify_real_ime_state(class)` を `can_use_imm32_cross_process` に統合した
    /// 前提(ECRS DD3-C、2026-10-21)を全列挙で固定する。
    ///
    /// 旧 `cannot_verify_real_ime_state(class)` は `Standard` のときだけクラス名に依存し(`is_effectively_tsf_native`)、
    /// `Standard` + TSF ネイティブクラスでは `!can_use_imm32_cross_process` と食い違う。統合が成り立つのは、本番で
    /// プロファイルを作る経路(`from_class_and_process`〈`focus/current.rs`〉と、`Standard` から動かすだけの学習降格
    /// 〈`focus/tracker.rs::apply_learned_imm_capability`〉)が、`Standard` を「IMM32 不可クラスでも TSF ネイティブ
    /// クラスでもない」ときにしか作らないから。クラス名は IMM32 不可クラスの全部・TSF ネイティブクラスの全部・
    /// 無関係な名前・空文字(`Default` の窓)を列挙する。ここが崩れたら統合の前提が崩れたということ。
    #[test]
    fn imm_capability_follows_profile_for_every_production_derivation() {
        let mut classes: Vec<&str> = IMM32_UNAVAILABLE_CLASSES.to_vec();
        classes.extend([
            "Windows.UI.Core.CoreWindow",
            "XamlExplorerHostIslandWindow",
            "Windows.UI.Input.InputSite.WindowClass",
            "CASCADIA_HOSTING_WINDOW_CLASS",
            "org.wezfurlong.wezterm",
            "Notepad",
            "Edit",
            "",
        ]);
        let relay_sets: [Vec<String>; 2] = [vec![], vec!["relay.exe".to_string()]];
        for class in &classes {
            for relay_apps in &relay_sets {
                for process in ["relay.exe", "app.exe"] {
                    let profile = AppImeProfile::from_class_and_process(class, process, relay_apps);
                    assert_eq!(
                        profile.can_use_imm32_cross_process(),
                        profile == AppImeProfile::Standard,
                        "{profile:?} class={class:?} process={process:?} relay={relay_apps:?}"
                    );
                    if profile == AppImeProfile::Standard {
                        assert!(
                            !IMM32_UNAVAILABLE_CLASSES.contains(class)
                                && !is_tsf_native_window(class),
                            "Standard が IMM32 不可/TSF ネイティブクラスで作られた: {class:?}。\
                             旧 cannot_verify(class) と !can_use_imm32_cross_process が食い違う"
                        );
                    }
                }
            }
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
                !AppImeProfile::from_class_name(class_name).can_use_imm32_cross_process(),
                "{class_name}: unavailable は常に IMM32 を使えないはず"
            );
        }
    }
}
