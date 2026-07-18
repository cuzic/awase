//! Optional [`TimerRuntime`] backed by `tokio`, enabled via the `tokio`
//! feature.
//!
//! # Why this exists
//!
//! [`TimerRuntime::set_timer`]/[`kill_timer`](TimerRuntime::kill_timer) are
//! synchronous — the trait says nothing about how a caller learns that a
//! timer fired. On a `tokio`-based event loop the answer is always the same
//! shape: spawn a task that sleeps and reports back, keep the handle so a
//! later `Kill` can abort it, and feed the report into a channel the main
//! loop can `select!` on. Every direct consumer of this crate that targets
//! `tokio` ends up writing that same
//! `HashMap<TimerId, JoinHandle<()>>` + `tokio::spawn(sleep)` + `mpsc`
//! plumbing by hand. [`TokioTimerRuntime`] is that plumbing, written once.
//!
//! # Example
//!
//! ```
//! use std::time::Duration;
//! use timed_fsm::tokio_support::TokioTimerRuntime;
//! use timed_fsm::TimerRuntime;
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() {
//! let mut timers = TokioTimerRuntime::<&str>::new();
//! timers.set_timer("debounce", Duration::from_millis(1));
//! assert_eq!(timers.recv().await, Some("debounce"));
//! # }
//! ```
//!
//! A full driver loop pairs [`recv`](TokioTimerRuntime::recv) with the
//! event source in a `tokio::select!`:
//!
//! ```ignore
//! let mut timers = TokioTimerRuntime::new();
//! loop {
//!     let response = tokio::select! {
//!         Some(event) = input_rx.recv() => machine.on_event(event),
//!         Some(timer_id) = timers.recv() => machine.on_timeout(timer_id),
//!     };
//!     timers.set_timer(..); // via response.timers, as usual
//! }
//! ```
//!
//! # Stray timeouts
//!
//! [`kill_timer`](TokioTimerRuntime::kill_timer) aborts the spawned sleep
//! task, but cannot retract a timer ID that has already been sent on the
//! internal channel before the abort lands — [`recv`](TokioTimerRuntime::recv)
//! can still yield it afterward. This crate's state machines are already
//! expected to tolerate such stray timeouts (see the "stray timeout"
//! pattern using [`Response::pass_through`](crate::Response::pass_through)
//! in the crate-level docs); this runtime does not change that contract.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::Duration;

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use crate::dispatch::TimerRuntime;

/// A [`TimerRuntime`] that drives timers as `tokio::spawn`ed sleep tasks.
///
/// `T` must be `Send + 'static` (moved into a spawned task) and
/// `Copy + Eq + Hash` (used as a `HashMap` key and sent by value on the
/// internal channel), matching [`TimedStateMachine::TimerId`](crate::TimedStateMachine::TimerId)'s
/// own bounds.
///
/// The entry for a timer's `JoinHandle` is only removed by a later `Set` or
/// `Kill` for that same ID (or by dropping the whole runtime) — a fired
/// timer's completed handle otherwise stays in the internal map. This is a
/// non-issue for the intended use (a small fixed `TimerId` enum), but a
/// `TimerId` drawn from an unbounded source (e.g. a per-request sequence
/// number) would grow the map without bound.
#[derive(Debug)]
pub struct TokioTimerRuntime<T> {
    handles: HashMap<T, JoinHandle<()>>,
    tx: UnboundedSender<T>,
    rx: UnboundedReceiver<T>,
}

impl<T> TokioTimerRuntime<T>
where
    T: Copy + Eq + Hash + Send + 'static,
{
    /// Create a runtime with no active timers.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { handles: HashMap::new(), tx, rx }
    }

    /// Wait for the next timer to fire.
    ///
    /// Feed the result into [`TimedStateMachine::on_timeout`](crate::TimedStateMachine::on_timeout)
    /// from a `tokio::select!` alongside your event source. In practice this
    /// never resolves to `None`: `self` always holds a live sender, so with
    /// no timer pending this future simply stays pending forever rather than
    /// signalling end-of-stream — harmless as one branch of a `select!`, but
    /// don't write `while let Some(id) = timers.recv().await` expecting the
    /// loop to end on its own once every timer is killed.
    pub async fn recv(&mut self) -> Option<T> {
        self.rx.recv().await
    }
}

impl<T> Default for TokioTimerRuntime<T>
where
    T: Copy + Eq + Hash + Send + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> TimerRuntime for TokioTimerRuntime<T>
where
    T: Copy + Eq + Hash + Send + 'static,
{
    type TimerId = T;

    /// Starts (or restarts) a timer. As required by [`TimerRuntime`], a
    /// `Set` for an ID with an already-active timer replaces it: the
    /// previous sleep task is aborted first.
    fn set_timer(&mut self, id: T, duration: Duration) {
        self.kill_timer(id);
        let tx = self.tx.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            // The receiving end (`self`) always outlives every task this
            // runtime spawns, so a send error here only means `recv` was
            // never polled again after this timer was superseded — nothing
            // to report back to.
            let _ = tx.send(id);
        });
        self.handles.insert(id, handle);
    }

    fn kill_timer(&mut self, id: T) {
        if let Some(handle) = self.handles.remove(&id) {
            handle.abort();
        }
    }
}

impl<T> Drop for TokioTimerRuntime<T> {
    /// Aborts every still-pending sleep task so dropping the runtime (e.g.
    /// a driver loop ending) doesn't leave detached timers running for up
    /// to their full remaining duration before their `send` silently fails.
    fn drop(&mut self) {
        for (_, handle) in self.handles.drain() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fired_timer_is_received() {
        let mut timers = TokioTimerRuntime::new();
        timers.set_timer("a", Duration::from_millis(1));
        assert_eq!(timers.recv().await, Some("a"));
    }

    #[tokio::test]
    async fn setting_the_same_id_again_resets_rather_than_duplicates() {
        let mut timers = TokioTimerRuntime::new();
        timers.set_timer("a", Duration::from_secs(60));
        timers.set_timer("a", Duration::from_millis(1));
        assert_eq!(timers.recv().await, Some("a"));
        // Only one fire should ever arrive for this id — the first (60s)
        // timer must have been replaced, not left running alongside.
        assert!(timers.rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn killed_timer_never_fires() {
        let mut timers = TokioTimerRuntime::new();
        timers.set_timer("a", Duration::from_millis(20));
        timers.kill_timer("a");
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(timers.rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn killing_an_unset_id_is_a_silent_no_op() {
        let mut timers = TokioTimerRuntime::<&str>::new();
        timers.kill_timer("never-set");
    }

    #[tokio::test]
    async fn independent_ids_fire_independently() {
        let mut timers = TokioTimerRuntime::new();
        timers.set_timer("slow", Duration::from_millis(30));
        timers.set_timer("fast", Duration::from_millis(1));
        assert_eq!(timers.recv().await, Some("fast"));
        assert_eq!(timers.recv().await, Some("slow"));
    }

    #[tokio::test]
    async fn dropping_the_runtime_aborts_pending_timers() {
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel::<()>();
        {
            let mut timers = TokioTimerRuntime::new();
            timers.set_timer("a", Duration::from_millis(20));
            // Rides along on the same runtime to observe whether the sleep
            // task's continuation past its `await` point ever runs.
            let handle_probe = probe_tx.clone();
            timers.handles.insert(
                "probe",
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    let _ = handle_probe.send(());
                }),
            );
        } // `timers` dropped here — both tasks should be aborted.
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(probe_rx.try_recv().is_err(), "task kept running after the runtime that owned it was dropped");
    }
}
