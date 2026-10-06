#![allow(unsafe_code)]
// Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
//! フォーカス種別（FocusKind）の決定ロジック

use crate::focus::FocusKind;
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
/// 1〜3（Config オーバーライド → キャッシュヒット → エンジンタイマー活性中はスキップ）は
/// `state/focus_resolve_plan.rs::decide_resolution` が決める。
/// 4. 決まらなければ `classify_focus` をワーカースレッドで実行（タイムアウト付き）
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
    use crate::state::focus_resolve_plan::{
        decide_resolution, Resolution, ResolveFacts, ResolveReason,
    };

    // observe: override（get_process_name で OS を読む）→ キャッシュ → engine 活性の順に、
    // 前段が決まらなかったときだけ次を読む（読む順序と回数は元のまま）。
    let config_override = platform.focus.override_check(process_id, class_name);
    let cached = if config_override.is_none() {
        platform.focus.cache_get(process_id, class_name)
    } else {
        None
    };
    let engine_busy =
        config_override.is_none() && cached.is_none() && platform.is_engine_processing();

    // decide（純粋、state/focus_resolve_plan.rs）
    if let Resolution::Resolved {
        kind,
        reason,
        overridden,
    } = decide_resolution(ResolveFacts {
        config_override,
        cached,
        engine_busy,
    }) {
        if reason == ResolveReason::EngineActive {
            tracing::debug!("classify_focus skipped: engine timer active (user typing)");
        }
        return FocusKindResolution {
            kind,
            reason: reason.as_str().to_string(),
            overridden,
        };
    }

    // execute: classify_focus をワーカースレッドで実行
    let hwnd_addr = hwnd.0 as usize;
    let classify_result =
        crate::win32::run_with_timeout(std::time::Duration::from_millis(300), move || {
            let hwnd = HWND(hwnd_addr as *mut _);
            classify::classify_focus(hwnd)
        });
    if let Some(result) = classify_result {
        FocusKindResolution {
            kind: result.kind,
            reason: format!("{}", result.reason),
            overridden: false,
        }
    } else {
        tracing::warn!("classify_focus timed out for hwnd={hwnd:?}");
        FocusKindResolution {
            kind: FocusKind::Undetermined,
            reason: "classify timeout".to_string(),
            overridden: false,
        }
    }
}
