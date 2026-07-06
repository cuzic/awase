//! 入力遅延キュー — OUTPUT_GATE active 中・TsfGate drain など複数の退避経路を集約する。
//!
//! すべての退避経路は classify 済みの `RawKeyEvent` として同一キューに積む。
//! `enrich_ime_relevance`（sync key 判定）のみ drain 側で `with_app` 内に実行する。

use awase::types::RawKeyEvent;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct InputDeferQueue {
    queue: Mutex<VecDeque<RawKeyEvent>>,
    /// 容量超過で drop した event の累積数。`take_all` でリセット。
    overflow_count: AtomicUsize,
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
    /// キューの最大容量。超えると最古の event を drop して新規を末尾に追加する。
    /// 通常は 0〜数件で運用される想定で、到達は drain 詰まりの兆候。
    pub const MAX_CAPACITY: usize = 1024;

    #[must_use]
    pub const fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            overflow_count: AtomicUsize::new(0),
        }
    }

    /// `OUTPUT_GATE.active` = true のときフックから呼ぶ。
    /// drain は `OutputActiveGuard::drop` が担うため post しない。
    /// poison でも復元してキー欠落を防ぐ。
    pub fn defer_during_output(&self, event: RawKeyEvent) {
        let mut q = match self.queue.lock() {
            Ok(q) => q,
            Err(e) => e.into_inner(),
        };
        self.push_with_cap(&mut q, event);
    }

    /// TsfGate・タイムアウト等から保留キーをまとめて再投入する。空でも安全（no-op）。
    pub fn replay_later(&self, events: impl IntoIterator<Item = RawKeyEvent>) {
        let mut q = match self.queue.lock() {
            Ok(q) => q,
            Err(e) => e.into_inner(),
        };
        let prev_len = q.len();
        for event in events {
            self.push_with_cap(&mut q, event);
        }
        if q.len() > prev_len {
            drop(q);
            crate::tsf::probe_bridge::post_drain_output_queue();
        }
    }

    /// キューを全て取り出してタイムスタンプ昇順で返す（WM_DRAIN_OUTPUT_QUEUE ハンドラ専用）。
    ///
    /// 取り出し後は `enrich_ime_relevance` を `with_app` 内で呼ぶこと。
    pub fn take_all(&self) -> Vec<RawKeyEvent> {
        let taken: VecDeque<RawKeyEvent> = {
            let mut q = match self.queue.lock() {
                Ok(q) => q,
                Err(e) => e.into_inner(),
            };
            std::mem::take(&mut *q)
        };
        let mut result: Vec<RawKeyEvent> = taken.into();
        result.sort_by_key(|ev| ev.timestamp);
        let dropped = self.overflow_count.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            log::warn!(
                "[input-defer] drain after overflow: took {}, dropped {} (cap={})",
                result.len(),
                dropped,
                Self::MAX_CAPACITY,
            );
        }
        result
    }

    /// drain race 検出用（ノンブロッキング）。
    /// `None` = ロック競合中で不明。呼び出し側は保守的に pending ありとして扱うこと。
    pub fn pending_len_nonblocking(&self) -> Option<usize> {
        self.queue.try_lock().ok().map(|q| q.len())
    }

    /// 容量超過時は最古の event を drop して新規を末尾に追加。
    /// drop 時は warn ログ (初回 + 2 のべき乗ごと、spam 抑制)。
    fn push_with_cap(&self, q: &mut VecDeque<RawKeyEvent>, event: RawKeyEvent) {
        if q.len() >= Self::MAX_CAPACITY {
            q.pop_front();
            let prev = self.overflow_count.fetch_add(1, Ordering::Relaxed);
            let total = prev + 1;
            if prev == 0 || total.is_power_of_two() {
                log::warn!(
                    "[input-defer] overflow: cap {} reached, dropped oldest (total drops since drain: {})",
                    Self::MAX_CAPACITY, total,
                );
            }
        }
        q.push_back(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use awase::engine::ModifierState;
    use awase::scanmap::PhysicalPos;
    use awase::types::{ImeRelevance, KeyClassification, KeyEventType, ScanCode, VkCode};

    fn evt(ts: u64) -> RawKeyEvent {
        RawKeyEvent {
            vk_code: VkCode(0x41),
            scan_code: ScanCode(0x1E),
            event_type: KeyEventType::KeyDown,
            extra_info: 0,
            timestamp: ts,
            key_classification: KeyClassification::Passthrough,
            physical_pos: None::<PhysicalPos>,
            ime_relevance: ImeRelevance::default(),
            modifier_key: None,
            modifier_snapshot: ModifierState::default(),
            injected: false,
        }
    }

    #[test]
    fn defer_under_cap_keeps_all() {
        let q = InputDeferQueue::new();
        for i in 0..16u64 {
            q.defer_during_output(evt(i));
        }
        let out = q.take_all();
        assert_eq!(out.len(), 16);
        assert_eq!(out.first().unwrap().timestamp, 0);
        assert_eq!(out.last().unwrap().timestamp, 15);
    }

    #[test]
    fn defer_over_cap_drops_oldest() {
        let q = InputDeferQueue::new();
        // cap + 5 を push: 先頭 5 件が drop される想定
        let n = (InputDeferQueue::MAX_CAPACITY + 5) as u64;
        for i in 0..n {
            q.defer_during_output(evt(i));
        }
        let out = q.take_all();
        assert_eq!(out.len(), InputDeferQueue::MAX_CAPACITY);
        // 残るのは ts=5..(cap+5)
        assert_eq!(out.first().unwrap().timestamp, 5);
        assert_eq!(out.last().unwrap().timestamp, n - 1);
    }

    #[test]
    fn replay_later_respects_cap() {
        let q = InputDeferQueue::new();
        // 先に cap いっぱい push
        for i in 0..InputDeferQueue::MAX_CAPACITY as u64 {
            q.defer_during_output(evt(i));
        }
        // replay_later で追加 3 件 → 先頭 3 件が drop
        let extra = vec![evt(10_000), evt(10_001), evt(10_002)];
        q.replay_later(extra);
        let out = q.take_all();
        assert_eq!(out.len(), InputDeferQueue::MAX_CAPACITY);
        // 先頭 3 件 (ts=0,1,2) が drop され、末尾に 10_000 系が並ぶ (sort 後)
        assert_eq!(out.first().unwrap().timestamp, 3);
        assert_eq!(out.last().unwrap().timestamp, 10_002);
    }

    #[test]
    fn take_all_resets_overflow_count() {
        let q = InputDeferQueue::new();
        for i in 0..(InputDeferQueue::MAX_CAPACITY + 2) as u64 {
            q.defer_during_output(evt(i));
        }
        assert_eq!(q.overflow_count.load(Ordering::Relaxed), 2);
        let _ = q.take_all();
        assert_eq!(q.overflow_count.load(Ordering::Relaxed), 0);
    }
}
