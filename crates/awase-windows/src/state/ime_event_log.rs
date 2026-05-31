//! IME event のリングバッファ (Step 0)
//!
//! 全 IME 状態変更 event を時系列で記録する。
//! Step 0 では本番判定には使わず、診断・将来の Shadow Reducer の入力源として保持する。

use std::collections::VecDeque;
use std::time::Instant;

use super::ime_event::{EventTime, ImeEvent, ImeEventEnvelope};

/// デフォルトの保持容量。古い event は drop される。
pub const DEFAULT_CAPACITY: usize = 512;

/// IME event のリングバッファ。
///
/// `record(event)` で末尾に追加し、容量を超えたら先頭から drop する。
/// `seq` は全 event を通じて単調増加する番号で、reducer の順序判断に使う。
#[derive(Debug)]
pub struct ImeEventLog {
    buffer: VecDeque<ImeEventEnvelope>,
    next_seq: u64,
    capacity: usize,
}

impl ImeEventLog {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            next_seq: 0,
            capacity,
        }
    }

    /// Event を記録し、付与された `EventTime` を返す。
    ///
    /// `seq` は単調増加、`monotonic` は `Instant::now()`、
    /// `tick_ms` は `GetTickCount64()` 由来。
    pub fn record(&mut self, event: ImeEvent) -> EventTime {
        let time = EventTime {
            seq: self.next_seq,
            monotonic: Instant::now(),
            tick_ms: crate::hook::current_tick_ms(),
        };
        self.next_seq += 1;

        log::trace!("[ime-event seq={}] {:?}", time.seq, event,);

        let envelope = ImeEventEnvelope { time, event };
        if self.buffer.len() == self.capacity {
            let dropped = self.buffer.pop_front();
            if let Some(env) = &dropped {
                log::trace!(
                    "[ime-event-log] capacity={} reached, dropping oldest seq={}",
                    self.capacity,
                    env.time.seq,
                );
            }
        }
        self.buffer.push_back(envelope);
        time
    }

    /// 次に割り振られる `seq` を返す (現在記録されている最大 seq + 1)。
    ///
    /// 外部で先に `seq` を予約してから event を構築したい場合に使う。
    #[must_use]
    pub const fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// 記録済みの event 数を返す。
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// 直近 `n` 件の event を新しい順に返す。
    pub fn recent(&self, n: usize) -> impl Iterator<Item = &ImeEventEnvelope> {
        self.buffer.iter().rev().take(n)
    }

    /// 直近 `n` 件を新しい順に `Vec` で返す。
    ///
    /// テスト / snapshot 用。`recent` の `Iterator` 版より allocation が必要。
    #[must_use]
    pub fn recent_vec(&self, n: usize) -> Vec<&ImeEventEnvelope> {
        self.recent(n).collect()
    }

    /// 全 event を古い順に返す (デバッグ用、性能に注意)。
    pub fn iter(&self) -> impl Iterator<Item = &ImeEventEnvelope> {
        self.buffer.iter()
    }
}

impl Default for ImeEventLog {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ime_event::IntentSource;

    fn intent(target: bool) -> ImeEvent {
        ImeEvent::UserImeSetIntent {
            target,
            source: IntentSource::SyncKey,
        }
    }

    #[test]
    fn record_assigns_increasing_seq() {
        let mut log = ImeEventLog::new(10);
        let t0 = log.record(intent(true));
        let t1 = log.record(intent(false));
        assert_eq!(t0.seq, 0);
        assert_eq!(t1.seq, 1);
        assert_eq!(log.next_seq(), 2);
    }

    #[test]
    fn capacity_drops_oldest() {
        let mut log = ImeEventLog::new(3);
        for _ in 0..5 {
            log.record(intent(true));
        }
        assert_eq!(log.len(), 3);
        // seq は drop されても増え続ける
        assert_eq!(log.next_seq(), 5);
        // 残るのは seq 2, 3, 4
        let seqs: Vec<u64> = log.iter().map(|e| e.time.seq).collect();
        assert_eq!(seqs, vec![2, 3, 4]);
    }

    #[test]
    fn recent_returns_newest_first() {
        let mut log = ImeEventLog::new(10);
        for _ in 0..5 {
            log.record(intent(true));
        }
        let recent_seqs: Vec<u64> = log.recent(3).map(|e| e.time.seq).collect();
        assert_eq!(recent_seqs, vec![4, 3, 2]);
    }

    #[test]
    fn recent_vec_returns_newest_first() {
        let mut log = ImeEventLog::new(10);
        for _ in 0..5 {
            log.record(intent(true));
        }
        let recent: Vec<u64> = log.recent_vec(3).iter().map(|e| e.time.seq).collect();
        assert_eq!(recent, vec![4, 3, 2]);
    }

    #[test]
    fn monotonic_timestamps_are_non_decreasing() {
        let mut log = ImeEventLog::new(10);
        let t0 = log.record(intent(true));
        let t1 = log.record(intent(false));
        assert!(t1.monotonic >= t0.monotonic);
    }
}
