//! 学習1セッションの壁時計時間が、どの段階に費やされたかの内訳計測。
//!
//! 実機の学習は1押下あたり約0.69秒(MS-IME本体、1301秒/1891押下)かかっているのに対し、
//! `settle()`の設計上の待ちは50〜200ms程度のはずで、差の原因が未特定だった。
//! `.claude/rules/tuning-constants.md`は待ち定数を触る前に実測を求めるため、
//! 定数を動かす前にまず内訳を出す(観測専用、挙動は変えない)。
//!
//! OS非依存(`Instant`のみ)なのでLinuxでもユニットテストできる。

use std::cell::Cell;
use std::time::{Duration, Instant};

/// 計測する段階。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// `press()`全体(注入前観測+注入+settle+TSF観測+汚染判定)。
    Press,
    /// `settle()`全体(押下後・セットアップ後の両方)。
    Settle,
    /// `settle()`のうち、開始から最初の状態変化までの時間(変化した場合のみ)。
    SettleFirstChange,
    /// `settle()`のうち、最後の変化から静止確定まで(=QUIET_MSぶんの待ち)。
    SettleQuietTail,
    /// `observe_imm()`1回。
    ObserveImm,
    /// `observe_tsf()`1回。
    ObserveTsf,
    /// `reset()`全体。
    Reset,
    /// `press_setup()`全体(注入+SETUP_GAP_MS)。
    PressSetup,
    /// `clear_edit()`(入力欄クリア+QUIET_MSのpump)。
    ClearEdit,
}

const N: usize = 9;

const ALL: [(Phase, &str); N] = [
    (Phase::Press, "press"),
    (Phase::Settle, "settle"),
    (Phase::SettleFirstChange, "settle_first_change"),
    (Phase::SettleQuietTail, "settle_quiet_tail"),
    (Phase::ObserveImm, "observe_imm"),
    (Phase::ObserveTsf, "observe_tsf"),
    (Phase::Reset, "reset"),
    (Phase::PressSetup, "press_setup"),
    (Phase::ClearEdit, "clear_edit"),
];

/// 段階ごとの累計時間・回数・最大値。`&self`から更新するため`Cell`。
#[derive(Debug, Default)]
pub struct PhaseTimers {
    total_us: [Cell<u64>; N],
    count: [Cell<u32>; N],
    max_us: [Cell<u64>; N],
    /// `settle()`が静止確定でなく上限(`SETTLE_TIMEOUT_MS`)で終わった回数。
    settle_timeouts: Cell<u32>,
    /// `settle()`の間に一度も状態が変化しなかった回数(「変化なし」の確定待ち)。
    settle_no_change: Cell<u32>,
}

impl PhaseTimers {
    pub fn record(&self, phase: Phase, elapsed: Duration) {
        let i = phase as usize;
        let us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        self.total_us[i].set(self.total_us[i].get().saturating_add(us));
        self.count[i].set(self.count[i].get().saturating_add(1));
        if us > self.max_us[i].get() {
            self.max_us[i].set(us);
        }
    }

    /// `f`の所要時間を`phase`へ記録して結果を返す。
    pub fn time<T>(&self, phase: Phase, f: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let out = f();
        self.record(phase, start.elapsed());
        out
    }

    pub fn note_settle_timeout(&self) {
        self.settle_timeouts.set(self.settle_timeouts.get() + 1);
    }

    pub fn note_settle_no_change(&self) {
        self.settle_no_change.set(self.settle_no_change.get() + 1);
    }

    /// 1行1段階の要約(`timing phase=... n=... total_ms=... mean_ms=... max_ms=...`)。
    /// 標準エラーへ出す想定で、機械可読な`key=value`形式にする。
    #[must_use]
    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = ALL
            .iter()
            .map(|&(phase, name)| {
                let i = phase as usize;
                let n = self.count[i].get();
                let total_ms = self.total_us[i].get() as f64 / 1000.0;
                let mean_ms = if n == 0 { 0.0 } else { total_ms / f64::from(n) };
                let max_ms = self.max_us[i].get() as f64 / 1000.0;
                format!(
                    "timing phase={name} n={n} total_ms={total_ms:.0} mean_ms={mean_ms:.1} max_ms={max_ms:.0}"
                )
            })
            .collect();
        lines.push(format!(
            "timing settle_timeouts={} settle_no_change={}",
            self.settle_timeouts.get(),
            self.settle_no_change.get()
        ));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_accumulates_total_count_and_max() {
        let t = PhaseTimers::default();
        t.record(Phase::Settle, Duration::from_millis(10));
        t.record(Phase::Settle, Duration::from_millis(30));
        let line = t
            .summary_lines()
            .into_iter()
            .find(|l| l.contains("phase=settle "))
            .unwrap();
        assert!(line.contains("n=2"), "{line}");
        assert!(line.contains("total_ms=40"), "{line}");
        assert!(line.contains("mean_ms=20.0"), "{line}");
        assert!(line.contains("max_ms=30"), "{line}");
    }

    #[test]
    fn untouched_phase_reports_zero_without_dividing_by_zero() {
        let t = PhaseTimers::default();
        let line = t
            .summary_lines()
            .into_iter()
            .find(|l| l.contains("phase=reset "))
            .unwrap();
        assert!(
            line.contains("n=0") && line.contains("mean_ms=0.0"),
            "{line}"
        );
    }

    #[test]
    fn counters_appear_in_last_line() {
        let t = PhaseTimers::default();
        t.note_settle_timeout();
        t.note_settle_no_change();
        t.note_settle_no_change();
        let last = t.summary_lines().pop().unwrap();
        assert!(
            last.contains("settle_timeouts=1 settle_no_change=2"),
            "{last}"
        );
    }
}
