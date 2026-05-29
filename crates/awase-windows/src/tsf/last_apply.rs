//! `apply_ime_open` が OS に最後に送ったコマンド値を記録するログ。
//!
//! `apply_ime_open` が VK_KANJI や ImmSetOpenStatus を送った直後の値を記録し、
//! `KanjiToggleStrategy` が「直前に何を送ったか」を知るために使う。
//!
//! ## 役割の限定 — これは IME 状態の SSOT ではない
//!
//! これは「OS 側の現在 IME 状態」を追跡するものではない。
//! 真の SSOT（Single Source of Truth）は [`crate::state::Preconditions::ime_on`] であり、
//! 複数の観測ソースをマージした Engine 向け意図値を保持する。
//! この型はあくまで `apply_ime_open` で送信したキー操作の最新値を残すログにすぎず、
//! 診断・`KanjiToggleStrategy` の重複送信回避のために使う。
//!
//! ## ライフサイクル
//!
//! - `set(value)` — `apply_ime_open` 直後に呼ぶ（`applied_ms` を更新）
//! - `soft_set(value)` — フォーカス変更後プリシンク用（value のみ更新、`applied_ms` は 0 のまま）
//! - `invalidate()` — フォーカス変更時にクリア
//! - `get_or(fallback)` — `KanjiToggleStrategy` が直前送信値を読むときに使う
//!
//! `applied_ms > 0` は「フォーカス変更後に実 apply が 1 回以上完了した」ことを示す。
//! `soft_set` のみ呼ばれた状態（プリシンクのみ）は `applied_ms == 0` となり、
//! latch の信頼度が「実 apply 確認済み」と区別される。

/// `apply_ime_open()` が最後に OS に送ったコマンド値を記録するログ。
///
/// これは IME 状態の SSOT ではない。SSOT は [`crate::state::Preconditions::ime_on`]。
/// この型は `apply_ime_open` で OS に送信したキー操作の最新値を保持するのみで、
/// 診断・`KanjiToggleStrategy` の重複送信回避用途。
#[derive(Debug)]
pub struct LastAppliedImeState {
    value: std::cell::Cell<Option<bool>>,
    applied_ms: std::cell::Cell<u64>,
}

impl LastAppliedImeState {
    pub const fn new() -> Self {
        Self { value: std::cell::Cell::new(None), applied_ms: std::cell::Cell::new(0) }
    }

    /// `apply_ime_open` の完了後に呼ぶ。
    pub fn set(&self, value: bool) {
        log::debug!("[last-applied-ime] set({value})");
        self.value.set(Some(value));
        self.applied_ms.set(crate::hook::current_tick_ms());
    }

    /// フォーカス変更後プリシンク用。value のみ更新し `applied_ms` は変えない。
    ///
    /// `applied_ms == 0` を維持することで「フォーカス変更後に実 apply がまだない不確定状態」
    /// を示す。`desired=false` のプリシンク（新ウィンドウで IME が ON の可能性あり）に使う。
    pub fn soft_set(&self, value: bool) {
        log::debug!("[last-applied-ime] soft_set({value}) (pre-sync, applied_ms unchanged)");
        self.value.set(Some(value));
        // applied_ms は更新しない: 0 のまま → 「未確認」状態を維持
    }

    /// フォーカス変更時にクリアする。
    pub fn invalidate(&self) {
        log::debug!("[last-applied-ime] invalidate (focus changed)");
        self.value.set(None);
        self.applied_ms.set(0);
    }

    /// 記録値を返す。未設定（フォーカス変更直後など）は `fallback` を使う。
    pub fn get_or(&self, fallback: bool) -> bool {
        self.value.get().unwrap_or(fallback)
    }

    /// 最後の `set` 呼び出し時刻（ms）を返す。未設定は 0。
    pub fn applied_ms(&self) -> u64 {
        self.applied_ms.get()
    }
}

impl Default for LastAppliedImeState {
    fn default() -> Self {
        Self::new()
    }
}
