use std::time::Duration;

use crate::response::{Response, TimerCommand};

/// A runtime that can set and kill timers.
///
/// The state machine produces [`TimerCommand`]s in its [`Response`].
/// The runtime implements this trait to translate those commands into
/// actual platform timer operations.
///
/// # Platform examples
///
/// | Platform | `set_timer` | `kill_timer` |
/// |----------|-------------|--------------|
/// | Windows  | `SetTimer()` | `KillTimer()` |
/// | Linux    | `timerfd_settime()` | `timerfd_settime(0)` |
/// | macOS    | `CFRunLoopTimerSetNextFireDate()` | invalidate timer |
/// | Test     | record to `Vec` | record to `Vec` |
pub trait TimerRuntime {
    /// The timer identifier type (must match the state machine's `TimerId`).
    type TimerId;

    /// Start or restart a timer.
    ///
    /// If a timer with the same ID is already active, it must be reset
    /// to the new duration.
    fn set_timer(&mut self, id: Self::TimerId, duration: Duration);

    /// Stop an active timer. No-op if the timer is not active.
    fn kill_timer(&mut self, id: Self::TimerId);
}

/// A runtime that can execute actions produced by the state machine.
///
/// # Platform examples
///
/// | Platform | Implementation |
/// |----------|----------------|
/// | Windows  | `SendInput()` for key injection |
/// | Linux    | `uinput` write |
/// | Test     | record to `Vec` |
pub trait ActionExecutor {
    /// The action type (must match the state machine's `Action`).
    type Action;

    /// Execute a batch of actions, in order.
    fn execute(&mut self, actions: &[Self::Action]);
}

/// Dispatch a [`Response`] to a runtime.
///
/// Processes timer commands first, then executes actions.
/// Returns the `consumed` flag from the response.
///
/// This is the primary integration point between the state machine
/// (which is pure and side-effect-free) and the runtime (which
/// performs actual I/O).
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
