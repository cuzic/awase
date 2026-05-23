//! パニックリセット連打検出。
//!
//! `with_app` 再入中（フック再帰）でも確実に動作するよう、
//! ライブラリ層に配置してフックコールバックから直接呼べるようにする。

use crate::SingleThreadCell;

/// IME 関連キー押下のタイムスタンプ（循環バッファ）。
///
/// フックコールバックはメインスレッドで実行されるため `SingleThreadCell` で十分。
/// bootstrap で `.set(RapidPressTracker::new())` を呼ぶこと。
pub static RAPID_IME_TIMESTAMPS: SingleThreadCell<RapidPressTracker> = SingleThreadCell::new();

/// 連打検出用の軽量トラッカー
#[derive(Debug)]
pub struct RapidPressTracker {
    buf: [u64; 3],
    cursor: usize,
    count: usize,
}

impl RapidPressTracker {
    const THRESHOLD: usize = 3;
    const WINDOW_MS: u64 = 1000;

    pub const fn new() -> Self {
        Self {
            buf: [0; Self::THRESHOLD],
            cursor: 0,
            count: 0,
        }
    }

    /// タイムスタンプを記録し、連打が検出されたら `true` を返す。
    pub fn push(&mut self, now_ms: u64) -> bool {
        self.buf[self.cursor] = now_ms;
        self.cursor = (self.cursor + 1) % Self::THRESHOLD;
        if self.count < Self::THRESHOLD {
            self.count += 1;
        }
        if self.count < Self::THRESHOLD {
            return false;
        }
        let oldest = *self.buf.iter().min().unwrap_or(&0);
        now_ms.saturating_sub(oldest) < Self::WINDOW_MS
    }

    /// バッファをクリアする（発動後のリセット用）
    pub fn clear(&mut self) {
        self.buf = [0; Self::THRESHOLD];
        self.cursor = 0;
        self.count = 0;
    }
}

/// IME 関連キー KeyDown をパニック連打カウンタに記録する。
///
/// `with_app` 再入中でも安全に呼べる。連打閾値に達したら `WM_PANIC_RESET` を post する。
pub fn record_ime_keydown(now_ms: u64) {
    // with_app 再入中でも安全に呼べる（try_with_mut は RefCell の借用を試み、
    // 失敗した場合は何もせず返る）。
    RAPID_IME_TIMESTAMPS.try_with_mut(|tracker| {
        if tracker.push(now_ms) {
            tracker.clear();
            log::warn!("Rapid IME key press detected — requesting panic reset");
            crate::win32::post_to_main_thread(crate::WM_PANIC_RESET);
        }
    });
}
