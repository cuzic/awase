//! フォーカス種別（FocusKind）の決定ロジック

use awase::engine::{TIMER_PENDING, TIMER_SPECULATIVE};
use awase::types::FocusKind;
use windows::Win32::Foundation::HWND;

/// `resolve_focus_kind` の戻り値
#[derive(Debug)]
pub struct FocusKindResolution {
    pub kind: FocusKind,
    pub reason: String,
    pub overridden: bool,
}

/// `focus_kind` を決定する純粋関数（副作用なし）。
///
/// 1. Config オーバーライドをチェック
/// 2. キャッシュヒットをチェック
/// 3. エンジンタイマー活性中はスキップ
/// 4. `classify_focus` をワーカースレッドで実行（タイムアウト付き）
///
/// # Safety
/// タイムアウト付きワーカースレッドから Win32 API を呼び出す。
pub unsafe fn resolve_focus_kind(
    platform: &crate::platform::WindowsPlatform,
    process_id: u32,
    class_name: &str,
    hwnd: HWND,
) -> FocusKindResolution {
    use crate::focus::classify;

    // 1. Config オーバーライドをチェック
    if let Some(kind) = platform.focus_overrides.check_app_override(process_id, class_name) {
        return FocusKindResolution {
            kind,
            reason: "config override".to_string(),
            overridden: true,
        };
    }

    // 2. キャッシュヒットをチェック
    if let Some(cached) = platform.focus_cache.get(process_id, class_name) {
        return FocusKindResolution {
            kind: cached,
            reason: "cache hit".to_string(),
            overridden: false,
        };
    }

    // 3. エンジンタイマー活性中はスキップ
    let engine_timer_active = {
        let timer = &platform.timer;
        timer.is_active(TIMER_PENDING) || timer.is_active(TIMER_SPECULATIVE)
    };
    if engine_timer_active {
        log::debug!("classify_focus skipped: engine timer active (user typing)");
        return FocusKindResolution {
            kind: FocusKind::Undetermined,
            reason: "skipped (engine active)".to_string(),
            overridden: false,
        };
    }

    // 4. classify_focus をワーカースレッドで実行
    let hwnd_addr = hwnd.0 as usize;
    let classify_result = crate::win32::run_with_timeout(
        std::time::Duration::from_millis(300),
        move || {
            let hwnd = HWND(hwnd_addr as *mut _);
            classify::classify_focus(hwnd)
        },
    );
    if let Some(result) = classify_result { FocusKindResolution {
        kind: result.kind,
        reason: format!("{}", result.reason),
        overridden: false,
    } } else {
        log::warn!("classify_focus timed out for hwnd={hwnd:?}");
        FocusKindResolution {
            kind: FocusKind::Undetermined,
            reason: "classify timeout".to_string(),
            overridden: false,
        }
    }
}
