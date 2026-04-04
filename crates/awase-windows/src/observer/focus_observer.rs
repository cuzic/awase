//! フォーカス変更の観測 — Win32 API + 分類ロジックを使って `FocusObservation` を返す。

use awase::engine::FocusObservation;
use awase::engine::ModifierState;
use awase::types::{AppKind, FocusKind};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use crate::focus;
use crate::runtime::FocusDetector;
use crate::PlatformState;
use crate::TIMER_FOCUS_DEBOUNCE;

/// `GetAsyncKeyState` で現在の修飾キー状態を取得する。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn read_os_modifiers() -> ModifierState {
    // GetAsyncKeyState: 最上位ビットが 1 なら押下中
    let pressed = |vk: i32| -> bool { (GetAsyncKeyState(vk).cast_unsigned() & 0x8000) != 0 };
    ModifierState {
        ctrl: pressed(0x11),                 // VK_CONTROL
        alt: pressed(0x12),                  // VK_MENU
        shift: pressed(0x10),                // VK_SHIFT
        win: pressed(0x5B) || pressed(0x5C), // VK_LWIN / VK_RWIN
    }
}

/// 共通フィールドを設定した `FocusObservation` を生成するヘルパー
#[allow(clippy::too_many_arguments)]
fn make_obs(
    process_id: u32,
    class_name: &str,
    kind: FocusKind,
    reason: String,
    needs_uia: bool,
    overridden: bool,
    skip: bool,
    debounce_ms: u64,
) -> FocusObservation {
    FocusObservation {
        process_id,
        class_name: class_name.to_owned(),
        kind,
        reason,
        needs_uia,
        overridden,
        skip,
        debounce_timer_id: TIMER_FOCUS_DEBOUNCE,
        debounce_ms,
    }
}

/// Win32 API とキャッシュを使ってフォーカス変更を観測し、OS 非依存の `FocusObservation` を返す。
///
/// `platform_state` の `focus_kind`, `app_kind`, `preconditions.ime_on` を更新する。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn observe(
    hwnd: HWND,
    process_id: u32,
    class_name: &str,
    focus: &FocusDetector,
    platform_state: &mut PlatformState,
) -> FocusObservation {
    let debounce_ms = u64::from(platform_state.focus_debounce_ms);

    // IME 状態をフォーカス変更時に一括更新（新ウィンドウの状態に追随）
    {
        let snap = crate::ime::detect_ime_state();
        let ps = &mut platform_state.preconditions;
        ps.is_japanese_ime = snap.is_japanese_ime;
        if let Some(on) = snap.ime_on {
            ps.ime_on = on && snap.is_japanese_ime;
        } else if !snap.is_japanese_ime {
            ps.ime_on = false;
        }
        if let Some(romaji) = snap.is_romaji {
            ps.is_romaji = romaji;
        }
    }

    // 同一フォアグラウンドウィンドウ内での TextInput → Undetermined 降格を防止
    if let Some(obs) = check_same_process_skip(
        process_id,
        class_name,
        focus,
        platform_state.focus_kind,
        debounce_ms,
    ) {
        return obs;
    }

    // Config オーバーライド（最高優先度、キャッシュより先に判定）
    if let Some(obs) = check_overrides(
        process_id,
        class_name,
        focus,
        debounce_ms,
    ) {
        return obs;
    }

    // ビルトイン bypass: スタートメニュー・Windows 検索など、
    // IME 状態を正しく検出できないシステムウィンドウをデフォルトで bypass する。
    // やまぶきR の既知問題「スタートメニュー検索で不正な文字が出力される」への対策。
    if is_builtin_bypass(process_id, class_name) {
        log::debug!("classify_focus: builtin bypass ({class_name})");
        return make_obs(
            process_id,
            class_name,
            FocusKind::NonText,
            "builtin bypass (system search/menu)".to_owned(),
            false,
            false,
            false,
            debounce_ms,
        );
    }

    // キャッシュヒット → 即座に結果を適用
    if let Some(cached) = focus.cache.get(process_id, class_name) {
        log::trace!("classify_focus: cache hit ({process_id}, {class_name}) → {cached:?}");
        return make_obs(
            process_id,
            class_name,
            cached,
            "cache hit".to_owned(),
            false,
            false,
            false,
            debounce_ms,
        );
    }

    // AppKind をクラス名から判定して更新
    let app_kind = classify_app_kind(class_name);
    let prev_app_kind = platform_state.app_kind;
    platform_state.app_kind = app_kind;
    if app_kind != prev_app_kind {
        log::info!("AppKind changed: {prev_app_kind:?} → {app_kind:?} (class={class_name})");
    } else {
        log::debug!("AppKind: {app_kind:?} (class={class_name})");
    }

    // バイパス状態を判定（Win32 API 呼び出し）
    let result = focus::classify::classify_focus(hwnd);
    let state = result.kind;
    let reason = result.reason.to_string();

    log::debug!("Focus changed: hwnd={hwnd:?} class={class_name} reason={reason} → {state:?}");

    make_obs(
        process_id,
        class_name,
        state,
        reason,
        true,
        false,
        false,
        debounce_ms,
    )
}

/// 同一プロセス内の TextInput 降格防止チェック
unsafe fn check_same_process_skip(
    process_id: u32,
    class_name: &str,
    focus: &FocusDetector,
    current_focus_kind: FocusKind,
    debounce_ms: u64,
) -> Option<FocusObservation> {
    let fg = GetForegroundWindow();
    if current_focus_kind != FocusKind::TextInput {
        return None;
    }
    let (prev_pid, _) = focus.last_focus_info.as_ref()?;
    let fg_pid = focus::classify::get_window_process_id(fg);
    if fg_pid != *prev_pid {
        return None;
    }
    log::trace!("Keeping TextInput (same process {fg_pid}): class={class_name}");
    Some(make_obs(
        process_id,
        class_name,
        FocusKind::TextInput,
        format!("same process {fg_pid}"),
        false,
        false,
        true,
        debounce_ms,
    ))
}

/// Config オーバーライドチェック
fn check_overrides(
    process_id: u32,
    class_name: &str,
    focus: &FocusDetector,
    debounce_ms: u64,
) -> Option<FocusObservation> {
    if focus.overrides.force_text.is_empty() && focus.overrides.force_bypass.is_empty() {
        return None;
    }

    let process_name = focus::classify::get_process_name(process_id);
    for entry in &focus.overrides.force_text {
        if entry.process.eq_ignore_ascii_case(&process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
        {
            log::debug!(
                "classify_focus: config override force_text ({process_name}, {class_name})"
            );
            return Some(make_obs(
                process_id,
                class_name,
                FocusKind::TextInput,
                format!("config override force_text ({process_name})"),
                false,
                true,
                false,
                debounce_ms,
            ));
        }
    }
    for entry in &focus.overrides.force_bypass {
        if entry.process.eq_ignore_ascii_case(&process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
        {
            log::debug!(
                "classify_focus: config override force_bypass ({process_name}, {class_name})"
            );
            return Some(make_obs(
                process_id,
                class_name,
                FocusKind::NonText,
                format!("config override force_bypass ({process_name})"),
                false,
                true,
                false,
                debounce_ms,
            ));
        }
    }

    None
}

/// ウィンドウクラス名からアプリの UI フレームワーク種別を判定する。
///
/// - `Chrome_WidgetWin_1`: Chromium 系（Chrome, Edge, Electron, VS Code 等）
/// - `MozillaWindowClass`: Firefox（Chromium と同様の入力処理）
/// - `Windows.UI.Core.CoreWindow`: UWP / XAML 系
/// - その他: Win32 クラシック
fn classify_app_kind(class_name: &str) -> AppKind {
    let class_lower = class_name.to_ascii_lowercase();
    if class_lower.starts_with("chrome_") || class_lower == "mozillawindowclass" {
        AppKind::Chrome
    } else if class_lower == "windows.ui.core.corewindow"
        || class_lower == "applicationframewindow"
        || class_lower.starts_with("windows.ui.input.")
    {
        AppKind::Uwp
    } else {
        AppKind::Win32
    }
}

/// IME 状態を正しく検出できないシステムウィンドウを判定する。
///
/// これらのウィンドウは XAML/DirectUI ベースで、クロスプロセス IME 検出が
/// 不正確なため、NICOLA 変換をバイパスする必要がある。
/// ユーザー設定（force_bypass）より低優先度で、config で上書き可能。
fn is_builtin_bypass(_process_id: u32, class_name: &str) -> bool {
    let class_lower = class_name.to_ascii_lowercase();
    matches!(
        class_lower.as_str(),
        // Windows 11 検索 (SearchHost.exe)
        "searchhost"
        // スタートメニュー / Windows 10 検索 / UWP 系システムウィンドウ
        | "windows.ui.core.corewindow"
        // Cortana
        | "cortana"
    )
}
