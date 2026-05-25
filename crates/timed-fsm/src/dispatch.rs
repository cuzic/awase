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

/// Dispatch a [`Response`] to a runtime.
///
/// This is the primary integration point between the state machine
/// (which is pure and side-effect-free) and the runtime (which
/// performs actual I/O).
///
/// # Processing order
///
/// 1. **Timer commands** — all [`TimerCommand`]s in `response.timers`
///    are processed in order via [`TimerRuntime::set_timer`] /
///    [`TimerRuntime::kill_timer`].
/// 2. **Actions** — if `response.actions` is non-empty,
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
/// use timed_fsm::{Response, dispatch, TimerRuntime, ActionExecutor};
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
///     .with_timer(1, Duration::from_millis(100));
///
/// let mut timers = MockTimers(vec![]);
/// let mut executor = MockExecutor(vec![]);
/// let consumed = dispatch(&response, &mut timers, &mut executor);
///
/// assert!(consumed);
/// assert_eq!(executor.0, vec![42]);
/// assert_eq!(timers.0.len(), 1);
/// ```
pub fn dispatch<A, T: Copy + Eq + core::fmt::Debug>(
    response: &Response<A, T>,
    timers: &mut impl TimerRuntime<TimerId = T>,
    executor: &mut impl ActionExecutor<Action = A>,
) -> bool {
    for cmd in &response.timers {
        match *cmd {
            TimerCommand::Set { id, duration } => timers.set_timer(id, duration),
            TimerCommand::Kill { id } => timers.kill_timer(id),
        }
    }
    if !response.actions.is_empty() {
        executor.execute(&response.actions);
    }
    response.consumed
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
        let consumed = dispatch(&response, &mut timers, &mut executor);

        assert!(consumed);
        assert_eq!(timers.0.len(), 2);
        assert_eq!(executor.0, vec!["a", "b"]);
    }

    #[test]
    fn dispatch_pass_through_returns_false() {
        let response: Response<&str, u8> = Response::pass_through();
        let mut timers = RecordTimers(vec![]);
        let mut executor = RecordActions(vec![]);
        let consumed = dispatch(&response, &mut timers, &mut executor);

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
        let consumed = dispatch(&response, &mut timers, &mut executor);

        assert!(consumed);
        assert_eq!(timers.0.len(), 1);
        assert!(executor.0.is_empty());
    }
}
