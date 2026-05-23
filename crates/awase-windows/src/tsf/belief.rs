//! IME ON/OFF の行動層ラッチ。
//!
//! `apply_ime_open` が VK_KANJI や ImmSetOpenStatus を送った直後の値を保持し、
//! `KanjiToggleStrategy` が「直前に何を送ったか」を知るために使う。
//!
//! ## 役割の限定
//!
//! これは「OS 側の現在 IME 状態」を追跡するものではない。
//! OS 状態の SSOT は `preconditions.ime_on`（観測 → 判断の正規ルートで更新）。
//! このラッチは `apply_ime_open` と次の judgement サイクルの間のギャップを埋めるだけ。
//!
//! ## ライフサイクル
//!
//! - `set(value)` — `apply_ime_open` 直後に呼ぶ
//! - `invalidate()` — フォーカス変更時にクリア
//! - `get_or(fallback)` — `KanjiToggleStrategy` が shadow_on を読むときに使う

/// `apply_ime_open` の直前結果を保持する行動層ラッチ。
#[derive(Debug)]
pub struct ImeApplyLatch {
    value: std::cell::Cell<Option<bool>>,
}

impl ImeApplyLatch {
    pub fn new() -> Self {
        Self { value: std::cell::Cell::new(None) }
    }

    /// `apply_ime_open` の完了後に呼ぶ。
    pub fn set(&self, value: bool) {
        log::debug!("[ime-latch] set({value})");
        self.value.set(Some(value));
    }

    /// フォーカス変更時にクリアする。
    pub fn invalidate(&self) {
        log::debug!("[ime-latch] invalidate (focus changed)");
        self.value.set(None);
    }

    /// ラッチ値を返す。未設定（フォーカス変更直後など）は `fallback` を使う。
    pub fn get_or(&self, fallback: bool) -> bool {
        self.value.get().unwrap_or(fallback)
    }
}

impl Default for ImeApplyLatch {
    fn default() -> Self {
        Self::new()
    }
}
