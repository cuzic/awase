use std::future::Future;
use std::time::Duration;

use crate::response::{Response, TimerCommand};

/// A runtime that can set and kill timers.
///
/// The state machine produces [`TimerCommand`]s in its [`Response`].
/// The runtime implements this trait to translate those commands into
/// actual platform timer operations.
///
/// # Invariant: `Set` resets an existing timer
///
/// Implementations **must** ensure that calling [`set_timer`](Self::set_timer)
/// with an ID that already has an active timer **replaces** that timer rather
/// than creating a second concurrent timer for the same ID. The new timer
/// starts counting from the moment `set_timer` is called.
///
/// # Platform examples
///
/// | Platform | `set_timer` | `kill_timer` |
/// |----------|-------------|--------------|
/// | Windows  | `SetTimer()` (reuses the same `nIDEvent`) | `KillTimer()` |
/// | Linux    | `timerfd_settime()` (overwrites existing armed state) | `timerfd_settime(0)` |
/// | macOS    | `CFRunLoopTimerSetNextFireDate()` (adjusts fire date) | invalidate the `CFRunLoopTimerRef` |
/// | Test     | record to `Vec` | record to `Vec` |
pub trait TimerRuntime {
    /// The timer identifier type (must match the state machine's `TimerId`).
    type TimerId;

    /// Start or restart a timer.
    ///
    /// If a timer with the same ID is already active, it **must** be reset
    /// to the new duration — only one timer per ID may be active at a time.
    fn set_timer(&mut self, id: Self::TimerId, duration: Duration);

    /// Stop an active timer. No-op if the timer is not active.
    fn kill_timer(&mut self, id: Self::TimerId);
}

/// A runtime that can execute actions produced by the state machine.
///
/// # Order guarantee
///
/// The `actions` slice passed to [`execute`](Self::execute) is always in the
/// same order as they appear in [`Response::actions`](crate::Response::actions).
/// Implementations **must** process them in order (first to last) to preserve
/// the intended sequence of output events (e.g., modifier-down before
/// character-down in a key injection scenario).
///
/// # Platform examples
///
/// | Platform | Implementation |
/// |----------|----------------|
/// | Windows  | `SendInput()` for key injection |
/// | Linux    | `uinput` write |
/// | macOS    | `CGEventPost()` |
/// | Test     | record to `Vec` |
pub trait ActionExecutor {
    /// The action type (must match the state machine's `Action`).
    type Action;

    /// Execute a batch of actions, in the order they appear in the slice.
    fn execute(&mut self, actions: &[Self::Action]);
}

/// Shared by [`Response::dispatch`] and [`Response::dispatch_async`]: apply
/// every timer command in order via [`TimerRuntime::set_timer`] /
/// [`TimerRuntime::kill_timer`].
fn apply_timers<T: Copy>(
    commands: &[TimerCommand<T>],
    timers: &mut impl TimerRuntime<TimerId = T>,
) {
    for cmd in commands {
        match *cmd {
            TimerCommand::Set { id, duration } => timers.set_timer(id, duration),
            TimerCommand::Kill { id } => timers.kill_timer(id),
        }
    }
}

impl<A, T: Copy + Eq + core::fmt::Debug> Response<A, T> {
    /// Dispatch this response to a runtime.
    ///
    /// This is the primary integration point between the state machine
    /// (which is pure and side-effect-free) and the runtime (which
    /// performs actual I/O).
    ///
    /// # Processing order
    ///
    /// 1. **Timer commands** — all [`TimerCommand`]s in `self.timers`
    ///    are processed in order via [`TimerRuntime::set_timer`] /
    ///    [`TimerRuntime::kill_timer`].
    /// 2. **Actions** — if `self.actions` is non-empty,
    ///    [`ActionExecutor::execute`] is called once with the full slice.
    ///
    /// Timer commands are applied before actions so that a newly started
    /// timer cannot fire before the actions from the same transition have
    /// been executed (in single-threaded runtimes). In multi-threaded
    /// runtimes, callers are responsible for any necessary synchronization.
    ///
    /// Returns the `consumed` flag from the response.
    ///
    /// # Example
    ///
    /// ```
    /// use std::time::Duration;
    /// use timed_fsm::{Response, TimerRuntime, ActionExecutor};
    ///
    /// struct MockTimers(Vec<String>);
    /// impl TimerRuntime for MockTimers {
    ///     type TimerId = u8;
    ///     fn set_timer(&mut self, id: u8, dur: Duration) {
    ///         self.0.push(format!("set({id}, {dur:?})"));
    ///     }
    ///     fn kill_timer(&mut self, id: u8) {
    ///         self.0.push(format!("kill({id})"));
    ///     }
    /// }
    ///
    /// struct MockExecutor(Vec<i32>);
    /// impl ActionExecutor for MockExecutor {
    ///     type Action = i32;
    ///     fn execute(&mut self, actions: &[i32]) {
    ///         self.0.extend_from_slice(actions);
    ///     }
    /// }
    ///
    /// let response = Response::emit_one(42)
    ///     .with_timer(1u8, Duration::from_millis(100));
    ///
    /// let mut timers = MockTimers(vec![]);
    /// let mut executor = MockExecutor(vec![]);
    /// let consumed = response.dispatch(&mut timers, &mut executor);
    ///
    /// assert!(consumed);
    /// assert_eq!(executor.0, vec![42]);
    /// assert_eq!(timers.0.len(), 1);
    /// ```
    pub fn dispatch(
        &self,
        timers: &mut impl TimerRuntime<TimerId = T>,
        executor: &mut impl ActionExecutor<Action = A>,
    ) -> bool {
        apply_timers(&self.timers, timers);
        if !self.actions.is_empty() {
            executor.execute(&self.actions);
        }
        self.consumed
    }
}

/// What executing one action tells the caller about continuing.
///
/// This exists for drivers whose actions can discover that the resource
/// they depend on is already gone (a closed socket, a dropped connection, …)
/// — a fact the state machine itself has no way to express, since it never
/// touches that resource. [`Response::dispatch_async`] stops processing
/// further actions in the current response as soon as one reports
/// [`Stop`](Self::Stop), and reports that upward so the driver's own event
/// loop can end instead of waiting on events/timeouts that will never come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionOutcome {
    /// Keep going: process any remaining actions in this response, and keep
    /// the driver loop running afterward.
    Continue,
    /// The resource this action touched is gone. Any actions remaining in
    /// the current response are skipped, and [`DispatchOutcome::stop`] is
    /// set so the driver loop can shut down.
    Stop,
}

/// The result of [`Response::dispatch_async`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchOutcome {
    /// Same meaning as the `bool` returned by [`Response::dispatch`]: `true`
    /// unless the response was [`pass_through`](Response::pass_through).
    pub consumed: bool,
    /// `true` if any action reported [`ActionOutcome::Stop`]. The driver
    /// should stop waiting for further events/timeouts once this is set.
    pub stop: bool,
}

/// An async counterpart to [`ActionExecutor`], for runtimes where executing
/// an action needs to `.await` (acquiring a socket, probing a peer, …)
/// rather than complete synchronously.
///
/// Unlike [`ActionExecutor::execute`] (which receives the whole action
/// batch at once), actions are executed **one at a time** via
/// [`execute_one`](Self::execute_one) so that an action reporting
/// [`ActionOutcome::Stop`] can prevent the remaining actions in the same
/// response from running at all — see [`Response::dispatch_async`].
///
/// This trait has no dependency on any particular async runtime; it only
/// requires `core::future::Future`, so it works with `tokio`,
/// `async-std`, `smol`, or any other executor the driver chooses to poll
/// the returned future on.
pub trait AsyncActionExecutor {
    /// The action type (must match the state machine's `Action`).
    type Action;

    /// Execute a single action and report whether the driver should keep
    /// going.
    ///
    /// Implementations that need to hold data across the `.await` point
    /// should clone what they need out of `&mut self` before constructing
    /// the returned future, so the future itself doesn't borrow `self` any
    /// longer than the lifetime `'a` of this call already requires.
    fn execute_one<'a>(
        &'a mut self,
        action: &'a Self::Action,
    ) -> impl Future<Output = ActionOutcome> + Send + 'a;
}

impl<A, T: Copy + Eq + core::fmt::Debug> Response<A, T> {
    /// Async counterpart to [`dispatch`](Self::dispatch).
    ///
    /// Processing order is the same — timer commands first, then actions —
    /// but actions are awaited one at a time via
    /// [`AsyncActionExecutor::execute_one`], stopping early (and setting
    /// [`DispatchOutcome::stop`]) as soon as one reports
    /// [`ActionOutcome::Stop`].
    ///
    /// # Example
    ///
    /// ```
    /// use std::time::Duration;
    /// use timed_fsm::{ActionOutcome, AsyncActionExecutor, Response, TimerRuntime};
    ///
    /// struct MockTimers(Vec<String>);
    /// impl TimerRuntime for MockTimers {
    ///     type TimerId = u8;
    ///     fn set_timer(&mut self, id: u8, dur: Duration) {
    ///         self.0.push(format!("set({id}, {dur:?})"));
    ///     }
    ///     fn kill_timer(&mut self, _id: u8) {}
    /// }
    ///
    /// struct MockExecutor(Vec<i32>);
    /// impl AsyncActionExecutor for MockExecutor {
    ///     type Action = i32;
    ///     async fn execute_one(&mut self, action: &i32) -> ActionOutcome {
    ///         self.0.push(*action);
    ///         ActionOutcome::Continue
    ///     }
    /// }
    ///
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() {
    /// let response = Response::emit(vec![1, 2])
    ///     .with_timer(1u8, Duration::from_millis(100));
    ///
    /// let mut timers = MockTimers(vec![]);
    /// let mut executor = MockExecutor(vec![]);
    /// let outcome = response.dispatch_async(&mut timers, &mut executor).await;
    ///
    /// assert!(outcome.consumed);
    /// assert!(!outcome.stop);
    /// assert_eq!(executor.0, vec![1, 2]);
    /// # }
    /// ```
    pub async fn dispatch_async(
        &self,
        timers: &mut (impl TimerRuntime<TimerId = T> + Send),
        executor: &mut (impl AsyncActionExecutor<Action = A> + Send),
    ) -> DispatchOutcome
    where
        A: Sync,
        T: Sync,
    {
        apply_timers(&self.timers, timers);
        for action in &self.actions {
            if executor.execute_one(action).await == ActionOutcome::Stop {
                return DispatchOutcome {
                    consumed: self.consumed,
                    stop: true,
                };
            }
        }
        DispatchOutcome {
            consumed: self.consumed,
            stop: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordTimers(Vec<TimerCommand<u8>>);
    impl TimerRuntime for RecordTimers {
        type TimerId = u8;
        fn set_timer(&mut self, id: u8, duration: Duration) {
            self.0.push(TimerCommand::Set { id, duration });
        }
        fn kill_timer(&mut self, id: u8) {
            self.0.push(TimerCommand::Kill { id });
        }
    }

    struct RecordActions(Vec<&'static str>);
    impl ActionExecutor for RecordActions {
        type Action = &'static str;
        fn execute(&mut self, actions: &[&'static str]) {
            self.0.extend_from_slice(actions);
        }
    }

    #[test]
    fn dispatch_processes_timers_then_actions() {
        let response = Response::emit(vec!["a", "b"])
            .with_timer(1, Duration::from_millis(100))
            .with_kill_timer(2);

        let mut timers = RecordTimers(vec![]);
        let mut executor = RecordActions(vec![]);
        let consumed = response.dispatch(&mut timers, &mut executor);

        assert!(consumed);
        assert_eq!(timers.0.len(), 2);
        assert_eq!(executor.0, vec!["a", "b"]);
    }

    #[test]
    fn dispatch_pass_through_returns_false() {
        let response: Response<&str, u8> = Response::pass_through();
        let mut timers = RecordTimers(vec![]);
        let mut executor = RecordActions(vec![]);
        let consumed = response.dispatch(&mut timers, &mut executor);

        assert!(!consumed);
        assert!(timers.0.is_empty());
        assert!(executor.0.is_empty());
    }

    #[test]
    fn dispatch_consume_no_actions() {
        let response: Response<&str, u8> =
            Response::consume().with_timer(0, Duration::from_millis(50));
        let mut timers = RecordTimers(vec![]);
        let mut executor = RecordActions(vec![]);
        let consumed = response.dispatch(&mut timers, &mut executor);

        assert!(consumed);
        assert_eq!(timers.0.len(), 1);
        assert!(executor.0.is_empty());
    }

    /// Executes every action, but reports [`ActionOutcome::Stop`] the first
    /// time it sees an action equal to `stop_on` (if set).
    struct RecordAsyncActions {
        seen: Vec<&'static str>,
        stop_on: Option<&'static str>,
    }

    impl AsyncActionExecutor for RecordAsyncActions {
        type Action = &'static str;
        async fn execute_one(&mut self, action: &&'static str) -> ActionOutcome {
            self.seen.push(*action);
            if self.stop_on == Some(*action) {
                ActionOutcome::Stop
            } else {
                ActionOutcome::Continue
            }
        }
    }

    #[tokio::test]
    async fn dispatch_async_processes_timers_then_actions() {
        let response = Response::emit(vec!["a", "b"])
            .with_timer(1, Duration::from_millis(100))
            .with_kill_timer(2);

        let mut timers = RecordTimers(vec![]);
        let mut executor = RecordAsyncActions {
            seen: vec![],
            stop_on: None,
        };
        let outcome = response.dispatch_async(&mut timers, &mut executor).await;

        assert!(outcome.consumed);
        assert!(!outcome.stop);
        assert_eq!(timers.0.len(), 2);
        assert_eq!(executor.seen, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn dispatch_async_pass_through_returns_unconsumed_and_does_not_stop() {
        let response: Response<&str, u8> = Response::pass_through();
        let mut timers = RecordTimers(vec![]);
        let mut executor = RecordAsyncActions {
            seen: vec![],
            stop_on: None,
        };
        let outcome = response.dispatch_async(&mut timers, &mut executor).await;

        assert!(!outcome.consumed);
        assert!(!outcome.stop);
        assert!(executor.seen.is_empty());
    }

    #[tokio::test]
    async fn dispatch_async_stop_skips_remaining_actions_but_keeps_timers_applied() {
        // Timers are applied up front regardless of what actions do —
        // mirrors `dispatch`'s "timers before actions" ordering guarantee.
        let response = Response::emit(vec!["a", "b", "c"]).with_timer(9, Duration::from_millis(1));

        let mut timers = RecordTimers(vec![]);
        let mut executor = RecordAsyncActions {
            seen: vec![],
            stop_on: Some("b"),
        };
        let outcome = response.dispatch_async(&mut timers, &mut executor).await;

        assert!(outcome.consumed);
        assert!(
            outcome.stop,
            "an action reporting Stop must be reflected in the outcome"
        );
        assert_eq!(
            executor.seen,
            vec!["a", "b"],
            "action after the Stop-reporting one must not run"
        );
        assert_eq!(
            timers.0.len(),
            1,
            "timers still apply even though an action later stops the driver"
        );
    }

    #[tokio::test]
    async fn dispatch_async_all_continue_never_sets_stop() {
        let response = Response::emit(vec!["a", "b", "c"]);
        let mut timers = RecordTimers(vec![]);
        let mut executor = RecordAsyncActions {
            seen: vec![],
            stop_on: None,
        };
        let outcome = response.dispatch_async(&mut timers, &mut executor).await;

        assert!(!outcome.stop);
        assert_eq!(executor.seen, vec!["a", "b", "c"]);
    }
}
