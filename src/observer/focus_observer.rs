//! フォーカス変更の観測 — Win32 API + 分類ロジックを使って `FocusObservation` を返す。

use std::sync::atomic::Ordering;

use awase::engine::FocusObservation;
use awase::types::FocusKind;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use crate::focus;
use crate::runtime::FocusDetector;
use crate::{FOCUS_DEBOUNCE_MS, FOCUS_KIND, TIMER_FOCUS_DEBOUNCE};

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
    ) {
        return obs;
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
    ))
}

/// Config オーバーライドチェック
fn check_overrides(
    process_id: u32,
    class_name: &str,
    focus: &FocusDetector,
    debounce_ms: u64,
    cached_engine_enabled: Option<bool>,
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
            ));
        }
    }

    None
}
