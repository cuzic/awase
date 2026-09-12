//! per-HWND IME 状態スナップショットキャッシュ

use std::collections::HashMap;

use awase::engine::InputModeState;

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
    ) {
        let snapshot = HwndImeSnapshot {
            ime_on,
            input_mode,
            recorded_ms: crate::hook::current_tick_ms(),
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
        let now_ms = snapshot.recorded_ms;
        self.0
            .retain(|_, v| now_ms.saturating_sub(v.recorded_ms) <= HWND_CACHE_MAX_AGE_MS);
        self.0.insert((old_pid, old_class), snapshot);
    }

    /// フォーカス入場時にキャッシュを参照し、有効なスナップショットを返す。
    ///
    /// キャッシュヒットかつ有効期限内の場合は `Some(HwndImeSnapshot)` を返す。
    /// キャッシュミスまたは期限切れの場合は `None` を返す。
    #[must_use]
    pub fn restore(&self, new_pid: u32, new_class: &str) -> Option<HwndImeSnapshot> {
        let cache_key = (new_pid, new_class.to_string());
        if let Some(&snapshot) = self.0.get(&cache_key) {
            let age_ms = crate::hook::current_tick_ms().saturating_sub(snapshot.recorded_ms);
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
