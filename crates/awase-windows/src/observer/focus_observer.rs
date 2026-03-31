//! フォーカス変更の観測 — Win32 API + 分類ロジックを使って `FocusObservation` を返す。

use std::sync::atomic::Ordering;

use awase::engine::{FocusObservation, ModifierState};
use awase::types::FocusKind;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use crate::focus;
use crate::runtime::FocusDetector;
use crate::{FOCUS_DEBOUNCE_MS, FOCUS_KIND, TIMER_FOCUS_DEBOUNCE};

/// `GetAsyncKeyState` で現在の修飾キー状態を取得する。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
unsafe fn read_os_modifiers() -> ModifierState {
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
    cached_engine_enabled: Option<bool>,
    os_modifiers: Option<ModifierState>,
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
        cached_engine_enabled,
        os_modifiers,
    }
}

/// Win32 API とキャッシュを使ってフォーカス変更を観測し、OS 非依存の `FocusObservation` を返す。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn observe(
    hwnd: HWND,
    process_id: u32,
    class_name: &str,
    focus: &FocusDetector,
) -> FocusObservation {
    let debounce_ms = u64::from(FOCUS_DEBOUNCE_MS.load(Ordering::Relaxed));

    // 新ウィンドウのキャッシュ済みエンジン状態を取得
    let cached_engine_enabled = focus.cache.get_engine_state(process_id, class_name);

    // OS から修飾キー状態を取得（フォーカス変更時の同期用）
    let os_modifiers = Some(read_os_modifiers());

    // 同一フォアグラウンドウィンドウ内での TextInput → Undetermined 降格を防止
    if let Some(obs) = check_same_process_skip(process_id, class_name, focus, debounce_ms) {
        return obs;
    }

    // Config オーバーライド（最高優先度、キャッシュより先に判定）
    if let Some(obs) = check_overrides(
        process_id,
        class_name,
        focus,
        debounce_ms,
        cached_engine_enabled,
        os_modifiers,
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
            cached_engine_enabled,
            os_modifiers,
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
            cached_engine_enabled,
            os_modifiers,
        );
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
        cached_engine_enabled,
        os_modifiers,
    )
}

/// 同一プロセス内の TextInput 降格防止チェック
unsafe fn check_same_process_skip(
    process_id: u32,
    class_name: &str,
    focus: &FocusDetector,
    debounce_ms: u64,
) -> Option<FocusObservation> {
    let fg = GetForegroundWindow();
    let current_kind = FocusKind::load(&FOCUS_KIND);
    if current_kind != FocusKind::TextInput {
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
        None, // skip=true なので復元不要
        None, // skip=true なので修飾キー同期不要
    ))
}

/// Config オーバーライドチェック
fn check_overrides(
    process_id: u32,
    class_name: &str,
    focus: &FocusDetector,
    debounce_ms: u64,
    cached_engine_enabled: Option<bool>,
    os_modifiers: Option<ModifierState>,
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
                cached_engine_enabled,
                os_modifiers,
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
                cached_engine_enabled,
                os_modifiers,
            ));
        }
    }

    None
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
