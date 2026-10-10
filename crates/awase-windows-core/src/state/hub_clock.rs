//! `ImeStateHub` が時刻を読む口。実機は実時計、テスト・閉ループは手動で進める仮想時計。
//!
//! # なぜ必要か
//!
//! `ImeStateHub` の判定は2種類の時刻を使う: `Instant`（観測の鮮度・settle・タイムアウト。
//! `ImeEventLog` が付ける `EventTime::monotonic` と `effective_open` の根拠判定）と、
//! `GetTickCount64` 由来の ms（`IntentStore` の TTL）。これまで前者は `Instant::now()`、
//! 後者は `hook::current_tick_ms()` を各所で直接読んでいたため、`TickMs` を注入しても
//! `Instant` 側は壁時計のままで、仮想時間で動かすハーネス（`tests/support/harness.rs`）は
//! 本番の `ImeStateHub` と別の時間軸を持っていた（閉ループが本番の写しである理由の一つ）。
//!
//! この型は純粋で `cfg(windows)` ではない。実時計の tick の読み方は構築側が関数ポインタで渡す
//! （state 層が `hook` に依存しない規約、`TickMs` の doc を参照）。

use std::time::{Duration, Instant};

/// 時刻の供給元。
#[derive(Debug, Clone, Copy)]
pub enum HubClock {
    /// 実時計。`tick` は `GetTickCount64` 相当、`instant` は `Instant::now` を返す関数
    /// （どちらも構築側＝殻が渡す。核は壁時計を直接読まない。ADR-229 段階 B）。
    Wall {
        tick: fn() -> u64,
        instant: fn() -> Instant,
    },
    /// 手動の仮想時計。`advance_ms` でだけ進む。
    Manual {
        base: Instant,
        base_tick: u64,
        elapsed_ms: u64,
    },
}

impl HubClock {
    /// 実時計。
    #[must_use]
    pub const fn wall(tick: fn() -> u64, instant: fn() -> Instant) -> Self {
        Self::Wall { tick, instant }
    }

    /// 仮想時計。`base_tick` が経過 0ms の tick、`base` が経過 0ms の `Instant`
    /// （呼び出し側が渡す。テストは `Instant::now()` でよい）。
    #[must_use]
    pub const fn manual(base_tick: u64, base: Instant) -> Self {
        Self::Manual {
            base,
            base_tick,
            elapsed_ms: 0,
        }
    }

    /// 現在の `Instant`。
    #[must_use]
    pub fn now_instant(&self) -> Instant {
        match *self {
            Self::Wall { instant, .. } => instant(),
            Self::Manual {
                base, elapsed_ms, ..
            } => base + Duration::from_millis(elapsed_ms),
        }
    }

    /// 現在の tick（ms）。
    #[must_use]
    pub fn now_tick(&self) -> u64 {
        match *self {
            Self::Wall { tick, .. } => tick(),
            Self::Manual {
                base_tick,
                elapsed_ms,
                ..
            } => base_tick + elapsed_ms,
        }
    }

    /// 仮想時計を進める。実時計では何もしない（実時間は止められない）。
    pub fn advance_ms(&mut self, ms: u64) {
        if let Self::Manual { elapsed_ms, .. } = self {
            *elapsed_ms += ms;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_instant_and_tick_together_only_when_advanced() {
        let mut c = HubClock::manual(10_000, Instant::now());
        let (i0, t0) = (c.now_instant(), c.now_tick());
        assert_eq!(c.now_instant(), i0, "進めない限り止まっている");
        assert_eq!(c.now_tick(), t0);
        c.advance_ms(250);
        assert_eq!(c.now_instant() - i0, Duration::from_millis(250));
        assert_eq!(c.now_tick() - t0, 250, "Instant と tick は同じ量だけ進む");
    }

    #[test]
    fn wall_clock_reads_the_given_tick_function_and_ignores_advance() {
        let mut c = HubClock::wall(|| 42, Instant::now);
        c.advance_ms(1_000);
        assert_eq!(c.now_tick(), 42);
    }
}
