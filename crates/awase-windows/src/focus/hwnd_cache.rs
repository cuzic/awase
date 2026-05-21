//! per-HWND IME 状態スナップショットキャッシュ

use std::collections::HashMap;

use awase::engine::InputModeState;

use crate::{Preconditions, ShadowSource};

/// フォーカス切り替え時の IME 状態スナップショット（per-HWND キャッシュ用）
#[derive(Debug, Clone, Copy)]
pub struct HwndImeSnapshot {
    pub ime_on: bool,
    pub input_mode: InputModeState,
    /// 記録時刻（GetTickCount64 ミリ秒）
    pub recorded_ms: u64,
}

/// キャッシュ参照の結果
pub enum CacheOutcome {
    Hit,
    Miss,
}

/// キャッシュの最大有効期間（ミリ秒）
pub const HWND_CACHE_MAX_AGE_MS: u64 = 5_000;

/// フォーカス離脱時に preconditions を per-HWND キャッシュに保存する。
///
/// 古いエントリ（`HWND_CACHE_MAX_AGE_MS` を超えたもの）はこのタイミングで
/// まとめて削除する。
pub fn save_on_focus_leave(
    cache: &mut HashMap<(u32, String), HwndImeSnapshot>,
    old_pid: u32,
    old_class: String,
    preconditions: &Preconditions,
) {
    let snapshot = HwndImeSnapshot {
        ime_on: preconditions.ime_on,
        input_mode: preconditions.input_mode,
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

/// フォーカス入場時にキャッシュから IME 状態を復元する。
///
/// キャッシュヒットかつ有効期限内の場合は `preconditions` を即座に更新し
/// [`CacheOutcome::Hit`] を返す。
/// キャッシュミスまたは期限切れの場合は `preconditions` を変更せず
/// [`CacheOutcome::Miss`] を返す。
pub fn restore_on_focus_enter(
    cache: &HashMap<(u32, String), HwndImeSnapshot>,
    new_pid: u32,
    new_class: &str,
    preconditions: &mut Preconditions,
) -> CacheOutcome {
    let cache_key = (new_pid, new_class.to_string());
    if let Some(&snapshot) = cache.get(&cache_key) {
        let age_ms = crate::hook::current_tick_ms()
            .saturating_sub(snapshot.recorded_ms);
        if age_ms <= HWND_CACHE_MAX_AGE_MS {
            preconditions.set_ime_on(snapshot.ime_on, ShadowSource::HwndCache);
            preconditions.input_mode = snapshot.input_mode;
            log::info!(
                "HwndCache: restore [{} {}] ime_on={} mode={:?} ({}ms ago)",
                new_pid, new_class, snapshot.ime_on, snapshot.input_mode, age_ms,
            );
            return CacheOutcome::Hit;
        }
        log::info!(
            "HwndCache: stale [{} {}] ime_on={} mode={:?} ({}ms ago > {}ms) → FocusProbe 待ち",
            new_pid, new_class, snapshot.ime_on, snapshot.input_mode,
            age_ms, HWND_CACHE_MAX_AGE_MS,
        );
    } else {
        log::debug!(
            "HwndCache: no entry for [{} {}], stale until FocusProbe",
            new_pid, new_class,
        );
    }
    CacheOutcome::Miss
}
