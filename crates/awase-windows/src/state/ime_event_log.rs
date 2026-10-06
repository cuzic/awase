//! IME event の `seq` 採番器
//!
//! 全 IME 状態変更 event に単調増加の `seq` を付け、`EventTime` を返す。
//! 旧リングバッファ（512 件）は本番に読み手が無かったので ADR-232 D2 で撤去した。
//! 再生・不具合報告に使う記録は `journal`（`event_seq`・`tick_ms` つき）。

use std::time::Instant;

use super::ime_event::{EventTime, ImeEvent};
use super::TickMs;

/// IME event の `seq` 採番器。
///
/// `seq` は全 event を通じて単調増加する番号で、reducer の順序判断に使う。
#[derive(Debug, Default)]
pub struct ImeEventLog {
    next_seq: u64,
}

impl ImeEventLog {
    /// Event に `seq` を割り振り、付与された `EventTime` を返す。
    ///
    /// `seq` は単調増加、`monotonic` は呼び出し元（`ImeStateHub` の `HubClock`）が渡す
    /// （仮想時計で動かすため）、`tick_ms` は呼び出し元が `GetTickCount64()` から取得して渡す。
    pub fn record_at(
        &mut self,
        event: &ImeEvent,
        tick_ms: TickMs,
        monotonic: Instant,
    ) -> EventTime {
        let time = EventTime {
            seq: self.next_seq,
            monotonic,
            tick_ms: tick_ms.0,
        };
        self.next_seq += 1;

        tracing::trace!("[ime-event seq={}] {:?}", time.seq, event);

        time
    }

    /// 次に割り振られる `seq` を返す (現在記録されている最大 seq + 1)。
    ///
    /// 外部で先に `seq` を予約してから event を構築したい場合に使う。
    #[must_use]
    pub const fn next_seq(&self) -> u64 {
        self.next_seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ime_event::UserIntentSource;

    fn intent(target: bool) -> ImeEvent {
        ImeEvent::UserImeSetIntent {
            target,
            source: UserIntentSource::SyncKey,
        }
    }

    #[test]
    fn record_at_assigns_increasing_seq() {
        let mut log = ImeEventLog::default();
        let now = Instant::now();
        let t0 = log.record_at(&intent(true), TickMs(0), now);
        let t1 = log.record_at(&intent(false), TickMs(1), now);
        assert_eq!(t0.seq, 0);
        assert_eq!(t1.seq, 1);
        assert_eq!(log.next_seq(), 2);
    }

    #[test]
    fn record_at_carries_injected_times() {
        let mut log = ImeEventLog::default();
        let now = Instant::now();
        let t = log.record_at(&intent(true), TickMs(7), now);
        assert_eq!(t.tick_ms, 7);
        assert_eq!(t.monotonic, now);
    }
}
