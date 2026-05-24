//! per-HWND IME 状態スナップショットキャッシュ

use std::collections::HashMap;

use awase::engine::InputModeState;

use crate::ImeBelief;
use crate::tuning::HWND_CACHE_MAX_AGE_MS;

/// フォーカス切り替え時の IME 状態スナップショット（per-HWND キャッシュ用）
#[derive(Debug, Clone, Copy)]
pub struct HwndImeSnapshot {
    pub ime_on: bool,
    pub input_mode: InputModeState,
    /// 記録時刻（GetTickCount64 ミリ秒）
    pub recorded_ms: u64,
}

/// フォーカス離脱時に belief を per-HWND キャッシュに保存する。
///
/// 古いエントリ（[`crate::tuning::HWND_CACHE_MAX_AGE_MS`] を超えたもの）はこのタイミングで
/// まとめて削除する。
#[allow(clippy::implicit_hasher)]
pub fn save_on_focus_leave(
    cache: &mut HashMap<(u32, String), HwndImeSnapshot>,
    old_pid: u32,
    old_class: String,
    belief: &ImeBelief,
) {
    let snapshot = HwndImeSnapshot {
        ime_on: belief.ime_on(),
        input_mode: belief.input_mode(),
        recorded_ms: crate::hook::current_tick_ms(),
    };
    log::debug!(
        "HwndCache: save [{} {}] ime_on={} mode={:?}",
        old_pid, old_class, snapshot.ime_on, snapshot.input_mode,
    );
    let now_ms = snapshot.recorded_ms;
    cache.retain(|_, v| now_ms.saturating_sub(v.recorded_ms) <= HWND_CACHE_MAX_AGE_MS);
    cache.insert((old_pid, old_class), snapshot);
}

/// フォーカス入場時にキャッシュを参照し、有効なスナップショットを返す。
///
/// キャッシュヒットかつ有効期限内の場合は `Some(HwndImeSnapshot)` を返す。
/// キャッシュミスまたは期限切れの場合は `None` を返す。
///
/// `Preconditions` を直接変更しない。呼び出し元（`PlatformState::apply_hwnd_cache_restore()`）
/// が状態に反映すること。
#[must_use]
#[allow(clippy::implicit_hasher)]
pub fn restore_on_focus_enter(
    cache: &HashMap<(u32, String), HwndImeSnapshot>,
    new_pid: u32,
    new_class: &str,
) -> Option<HwndImeSnapshot> {
    let cache_key = (new_pid, new_class.to_string());
    if let Some(&snapshot) = cache.get(&cache_key) {
        let age_ms = crate::hook::current_tick_ms()
            .saturating_sub(snapshot.recorded_ms);
        if age_ms <= HWND_CACHE_MAX_AGE_MS {
            log::info!(
                "HwndCache: restore [{} {}] ime_on={} mode={:?} ({}ms ago)",
                new_pid, new_class, snapshot.ime_on, snapshot.input_mode, age_ms,
            );
            return Some(snapshot);
        }
        log::info!(
            "HwndCache: stale [{} {}] ime_on={} mode={:?} ({}ms ago > {}ms) → FocusProbe 待ち",
            new_pid, new_class, snapshot.ime_on, snapshot.input_mode,
            age_ms, HWND_CACHE_MAX_AGE_MS,
        );
    } else {
        log::debug!(
            "HwndCache: no entry for [{new_pid} {new_class}], stale until FocusProbe",
        );
    }
    None
}
