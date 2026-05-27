//! 入力遅延キュー — OUTPUT_GATE active 中・TsfGate drain など複数の退避経路を集約する。
//!
//! すべての退避経路は classify 済みの `RawKeyEvent` として同一キューに積む。
//! `enrich_ime_relevance`（sync key 判定）のみ drain 側で `with_app` 内に実行する。

use std::sync::Mutex;
use awase::types::RawKeyEvent;

pub struct InputDeferQueue {
    queue: Mutex<Vec<RawKeyEvent>>,
}

impl std::fmt::Debug for InputDeferQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputDeferQueue").finish_non_exhaustive()
    }
}

pub static INPUT_DEFER: InputDeferQueue = InputDeferQueue::new();

impl Default for InputDeferQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl InputDeferQueue {
    #[must_use] 
    pub const fn new() -> Self {
        Self { queue: Mutex::new(Vec::new()) }
    }

    /// `OUTPUT_GATE.active` = true のときフックから呼ぶ。
    /// drain は `OutputActiveGuard::drop` が担うため post しない。
    pub fn defer_during_output(&self, event: RawKeyEvent) {
        if let Ok(mut q) = self.queue.lock() { q.push(event); }
    }

    /// TsfGate・タイムアウト等から保留キーをまとめて再投入する。空でも安全（no-op）。
    pub fn replay_later(&self, events: impl IntoIterator<Item = RawKeyEvent>) {
        let mut q = match self.queue.lock() {
            Ok(q) => q,
            Err(e) => e.into_inner(),
        };
        let prev_len = q.len();
        q.extend(events);
        if q.len() > prev_len {
            drop(q);
            crate::tsf::probe_bridge::post_drain_output_queue();
        }
    }

    /// キューを全て取り出してタイムスタンプ昇順で返す（WM_DRAIN_OUTPUT_QUEUE ハンドラ専用）。
    ///
    /// 取り出し後は `enrich_ime_relevance` を `with_app` 内で呼ぶこと。
    pub fn take_all(&self) -> Vec<RawKeyEvent> {
        let mut result = {
            let mut q = match self.queue.lock() {
                Ok(q) => q,
                Err(e) => e.into_inner(),
            };
            std::mem::take(&mut *q)
        };
        result.sort_by_key(|ev| ev.timestamp);
        result
    }

    /// drain race 検出用（ノンブロッキング）。
    pub fn pending_len_nonblocking(&self) -> Option<usize> {
        self.queue.try_lock().ok().map(|q| q.len())
    }
}
