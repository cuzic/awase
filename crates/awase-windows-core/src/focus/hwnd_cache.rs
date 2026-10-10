//! per-HWND IME 状態スナップショットキャッシュ

use std::collections::HashMap;

use awase::engine::InputModeState;

use crate::state::TickMs;
use crate::tuning::HWND_CACHE_MAX_AGE_MS;

/// フォーカス切り替え時の IME 状態スナップショット（per-HWND キャッシュ用）
#[derive(Debug, Clone, Copy)]
pub struct HwndImeSnapshot {
    pub ime_on: bool,
    pub input_mode: InputModeState,
    /// 記録時刻（GetTickCount64 ミリ秒）
    pub recorded_ms: u64,
    /// `ime_on=false` のとき、その状態がユーザーの明示的操作（SyncKey/PhysicalImeKey 等）によるものか。
    ///
    /// `true`: Ctrl+無変換 等の明示的 IME OFF 操作の結果 → このキャッシュは信頼できる。
    /// `false`: 前ウィンドウからの carry-over や Recovery 等の不確かな状態 → stale の可能性あり。
    /// `ime_on=true` のときは常に `false`（使用しない）。
    pub from_explicit_off_intent: bool,
    /// このスナップショットを記録した時点でフォーカスされていたウィンドウの hwnd。
    ///
    /// `(pid, class_name)` キーは同一クラス名を共有する複数の無関係なウィンドウを
    /// 取り違えうる（`Windows.UI.Input.InputSite.WindowClass` 等）ため、
    /// TsfNative 窓への ON 復元時にこの値と入場先の hwnd を比較して同一ウィンドウ
    /// インスタンスへの復帰かどうかを判定する（BUG-128、ADR-165 参照）。
    pub hwnd: usize,
}

/// per-HWND IME 状態スナップショットのキャッシュ。
///
/// save/restore のペアを一つの型で保護し、生の `HashMap` を外部に露出しない。
#[derive(Debug, Default)]
pub struct HwndImeCache(HashMap<(u32, String), HwndImeSnapshot>);

impl HwndImeCache {
    #[must_use]
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// フォーカス離脱時に IME 状態を保存する。
    ///
    /// 古いエントリ（[`crate::tuning::HWND_CACHE_MAX_AGE_MS`] を超えたもの）は
    /// このタイミングでまとめて削除する。
    pub fn save(
        &mut self,
        old_pid: u32,
        old_class: String,
        ime_on: bool,
        input_mode: InputModeState,
        from_explicit_off_intent: bool,
        old_hwnd: usize,
        now: TickMs,
    ) {
        let snapshot = HwndImeSnapshot {
            ime_on,
            input_mode,
            recorded_ms: now.0,
            from_explicit_off_intent,
            hwnd: old_hwnd,
        };
        tracing::debug!(
            "HwndCache: save [{} {}] ime_on={} mode={:?} hwnd={}",
            old_pid,
            old_class,
            snapshot.ime_on,
            snapshot.input_mode,
            snapshot.hwnd,
        );
        let now_ms = now.0;
        self.0
            .retain(|_, v| now_ms.saturating_sub(v.recorded_ms) <= HWND_CACHE_MAX_AGE_MS);
        self.0.insert((old_pid, old_class), snapshot);
    }

    /// フォーカス入場時にキャッシュを参照し、有効なスナップショットを返す。
    ///
    /// キャッシュヒットかつ有効期限内の場合は `Some(HwndImeSnapshot)` を返す。
    /// キャッシュミスまたは期限切れの場合は `None` を返す。
    #[must_use]
    pub fn restore(&self, new_pid: u32, new_class: &str, now: TickMs) -> Option<HwndImeSnapshot> {
        let cache_key = (new_pid, new_class.to_string());
        if let Some(&snapshot) = self.0.get(&cache_key) {
            let age_ms = now.saturating_sub(snapshot.recorded_ms);
            if age_ms <= HWND_CACHE_MAX_AGE_MS {
                tracing::info!(
                    "HwndCache: restore [{} {}] ime_on={} mode={:?} hwnd={} ({}ms ago)",
                    new_pid,
                    new_class,
                    snapshot.ime_on,
                    snapshot.input_mode,
                    snapshot.hwnd,
                    age_ms,
                );
                return Some(snapshot);
            }
            tracing::info!(
                "HwndCache: stale [{} {}] ime_on={} mode={:?} hwnd={} ({}ms ago > {}ms) → FocusProbe 待ち",
                new_pid,
                new_class,
                snapshot.ime_on,
                snapshot.input_mode,
                snapshot.hwnd,
                age_ms,
                HWND_CACHE_MAX_AGE_MS,
            );
        } else {
            tracing::debug!(
                "HwndCache: no entry for [{new_pid} {new_class}], stale until FocusProbe"
            );
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PID: u32 = 100;
    const CLASS: &str = "TestClass";
    const T0: u64 = 1_000_000;

    fn save_at(cache: &mut HwndImeCache, pid: u32, class: &str, hwnd: usize, at: u64) {
        cache.save(
            pid,
            class.to_string(),
            true,
            InputModeState::ObservedKana,
            false,
            hwnd,
            TickMs(at),
        );
    }

    #[test]
    fn restore_returns_saved_snapshot_within_max_age() {
        let mut cache = HwndImeCache::new();
        save_at(&mut cache, PID, CLASS, 0x10, T0);
        let snap = cache
            .restore(PID, CLASS, TickMs(T0 + HWND_CACHE_MAX_AGE_MS))
            .expect("境界ちょうど（age == MAX_AGE）は有効");
        assert!(snap.ime_on);
        assert_eq!(snap.input_mode, InputModeState::ObservedKana);
        assert_eq!(snap.recorded_ms, T0);
        assert_eq!(snap.hwnd, 0x10);
        assert!(!snap.from_explicit_off_intent);
    }

    #[test]
    fn restore_returns_none_after_max_age() {
        let mut cache = HwndImeCache::new();
        save_at(&mut cache, PID, CLASS, 0x10, T0);
        assert!(cache
            .restore(PID, CLASS, TickMs(T0 + HWND_CACHE_MAX_AGE_MS + 1))
            .is_none());
    }

    #[test]
    fn restore_misses_for_different_pid_or_class() {
        let mut cache = HwndImeCache::new();
        save_at(&mut cache, PID, CLASS, 0x10, T0);
        assert!(cache.restore(PID + 1, CLASS, TickMs(T0)).is_none());
        assert!(cache.restore(PID, "Other", TickMs(T0)).is_none());
        assert!(cache.restore(PID, CLASS, TickMs(T0)).is_some());
    }

    #[test]
    fn save_overwrites_entry_with_same_key_keeping_new_hwnd() {
        let mut cache = HwndImeCache::new();
        save_at(&mut cache, PID, CLASS, 0x10, T0);
        save_at(&mut cache, PID, CLASS, 0x20, T0 + 5);
        let snap = cache.restore(PID, CLASS, TickMs(T0 + 5)).unwrap();
        assert_eq!(snap.hwnd, 0x20);
        assert_eq!(snap.recorded_ms, T0 + 5);
        assert_eq!(cache.0.len(), 1);
    }

    #[test]
    fn save_evicts_only_entries_older_than_max_age() {
        let mut cache = HwndImeCache::new();
        save_at(&mut cache, 1, CLASS, 0x1, T0);
        save_at(&mut cache, 2, CLASS, 0x2, T0 + 10);
        // pid=1 は期限切れ、pid=2 は境界ちょうどで残る時刻に別エントリを保存する。
        save_at(&mut cache, 3, CLASS, 0x3, T0 + 10 + HWND_CACHE_MAX_AGE_MS);
        assert!(!cache.0.contains_key(&(1, CLASS.to_string())));
        assert!(cache.0.contains_key(&(2, CLASS.to_string())));
        assert!(cache.0.contains_key(&(3, CLASS.to_string())));
    }

    #[test]
    fn restore_before_recorded_time_saturates_to_zero_age() {
        let mut cache = HwndImeCache::new();
        save_at(&mut cache, PID, CLASS, 0x10, T0);
        assert!(cache.restore(PID, CLASS, TickMs(T0 - 1)).is_some());
    }
}
