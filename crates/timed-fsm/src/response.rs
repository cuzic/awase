use std::time::Duration;

/// The result of a state machine transition.
///
/// A `Response` is a **declarative description** of what should happen:
/// which actions to emit, which timers to set or kill, and whether the
/// original event was consumed.
///
/// The state machine never executes side effects directly. The runtime
/// reads the `Response` and performs the actual timer operations and
/// action execution via [`dispatch`](crate::dispatch::dispatch).
#[derive(Debug)]
pub struct Response<A, T> {
    /// Whether the original event was consumed by the state machine.
    ///
    /// - `true`: the event was handled; the caller should **not** propagate it.
    /// - `false`: the event was not handled; the caller should propagate it
    ///   (e.g., pass through to the next hook in a chain).
    pub consumed: bool,

    /// Actions to emit, in order.
    ///
    /// May be empty if the transition only affects internal state or timers.
    pub actions: Vec<A>,

    /// Timer commands to execute, in order.
    ///
    /// The runtime must process these commands after the actions.
    /// A [`Set`](TimerCommand::Set) with an ID that already has an active
    /// timer should reset (overwrite) that timer.
    pub timers: Vec<TimerCommand<T>>,
}

/// A command to set or kill a timer.
///
/// Timer commands are part of the [`Response`] returned by state machine
/// transitions. The runtime is responsible for translating these into
/// actual platform timer operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerCommand<T> {
    /// Start (or restart) a timer with the given ID and duration.
    ///
    /// If a timer with the same ID is already active, it is reset
    /// to the new duration.
    Set {
        /// Timer identifier.
        id: T,
        /// Duration after which [`TimedStateMachine::on_timeout`](crate::TimedStateMachine::on_timeout) should be called.
        duration: Duration,
    },

    /// Stop an active timer.
    ///
    /// If no timer with this ID is active, this is a no-op.
    Kill {
        /// Timer identifier to stop.
        id: T,
    },
}

// ── Builder methods ──────────────────────────────────────────

impl<A, T> Response<A, T> {
    /// Create a response that consumes the event and emits actions.
    #[must_use]
    pub const fn emit(actions: Vec<A>) -> Self {
        Self {
            consumed: true,
            actions,
            timers: Vec::new(),
        }
    }

    /// Create a response that consumes the event and emits a single action.
    #[must_use]
    pub fn emit_one(action: A) -> Self {
        Self {
            consumed: true,
            actions: vec![action],
            timers: Vec::new(),
        }
    }

    /// Create a response that consumes the event but emits no actions.
    ///
    /// Typically used when the state machine enters a pending state
    /// and will emit actions later (on a subsequent event or timeout).
    #[must_use]
    pub const fn consume() -> Self {
        Self {
            consumed: true,
            actions: Vec::new(),
            timers: Vec::new(),
        }
    }

    /// Create a response that does **not** consume the event.
    ///
    /// The caller should propagate the event as if the state machine
    /// did not exist.
    #[must_use]
    pub const fn pass_through() -> Self {
        Self {
            consumed: false,
            actions: Vec::new(),
            timers: Vec::new(),
        }
    }

    /// Add a timer set command to this response.
    #[must_use]
    pub fn with_timer(mut self, id: T, duration: Duration) -> Self {
        self.timers.push(TimerCommand::Set { id, duration });
        self
    }

    /// Add a timer kill command to this response.
    #[must_use]
    pub fn with_kill_timer(mut self, id: T) -> Self {
        self.timers.push(TimerCommand::Kill { id });
        self
    }
}

// ── Trait implementations ────────────────────────────────────

impl<A, T> Default for Response<A, T> {
    fn default() -> Self {
        Self::pass_through()
    }
}

impl<A: Clone, T: Clone> Clone for Response<A, T> {
    fn clone(&self) -> Self {
        Self {
            consumed: self.consumed,
            actions: self.actions.clone(),
            timers: self.timers.clone(),
        }
    }
}

impl<A: PartialEq, T: PartialEq> PartialEq for Response<A, T> {
    fn eq(&self, other: &Self) -> bool {
        self.consumed == other.consumed
            && self.actions == other.actions
            && self.timers == other.timers
    }
}

impl<A: Eq, T: Eq> Eq for Response<A, T> {}

// ── Assertion helpers ────────────────────────────────────────

impl<A: core::fmt::Debug, T: Copy + Eq + core::fmt::Debug> Response<A, T> {
    /// Assert that the event was consumed.
    ///
    /// # Panics
    ///
    /// Panics if `consumed` is `false`.
    #[track_caller]
    pub fn assert_consumed(&self) {
        assert!(self.consumed, "expected consumed, got pass-through");
    }

    /// Assert that the event was not consumed (pass-through).
    ///
    /// # Panics
    ///
    /// Panics if `consumed` is `true`.
    #[track_caller]
    pub fn assert_pass_through(&self) {
        assert!(!self.consumed, "expected pass-through, got consumed");
    }

    /// Assert that a timer set command exists for the given ID.
    ///
    /// # Panics
    ///
    /// Panics if no `Set` command with the given ID is found.
    #[track_caller]
    pub fn assert_timer_set(&self, id: T) {
        assert!(
            self.timers
                .iter()
                .any(|t| matches!(t, TimerCommand::Set { id: i, .. } if *i == id)),
            "expected TimerCommand::Set with id {id:?}, found {:?}",
            self.timers
        );
    }

    /// Assert that a timer kill command exists for the given ID.
    ///
    /// # Panics
    ///
    /// Panics if no `Kill` command with the given ID is found.
    #[track_caller]
    pub fn assert_timer_kill(&self, id: T) {
        assert!(
            self.timers
                .iter()
                .any(|t| matches!(t, TimerCommand::Kill { id: i } if *i == id)),
            "expected TimerCommand::Kill with id {id:?}, found {:?}",
            self.timers
        );
    }

    /// Assert the number of actions in the response.
    ///
    /// # Panics
    ///
    /// Panics if the action count does not match.
    #[track_caller]
    pub fn assert_action_count(&self, expected: usize) {
        assert_eq!(
            self.actions.len(),
            expected,
            "expected {expected} actions, got {}: {:?}",
            self.actions.len(),
            self.actions
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_one_is_consumed_with_one_action() {
        let r: Response<&str, ()> = Response::emit_one("hello");
        assert!(r.consumed);
        assert_eq!(r.actions, vec!["hello"]);
        assert!(r.timers.is_empty());
    }

    #[test]
    fn pass_through_is_not_consumed() {
        let r: Response<&str, ()> = Response::pass_through();
        assert!(!r.consumed);
        assert!(r.actions.is_empty());
        assert!(r.timers.is_empty());
    }

    #[test]
    fn consume_is_consumed_with_no_actions() {
        let r: Response<&str, ()> = Response::consume();
        assert!(r.consumed);
        assert!(r.actions.is_empty());
    }

    #[test]
    fn builder_chain() {
        let r: Response<i32, u8> = Response::emit_one(42)
            .with_timer(1, Duration::from_millis(100))
            .with_kill_timer(2);

        assert!(r.consumed);
        assert_eq!(r.actions, vec![42]);
        assert_eq!(r.timers.len(), 2);
        assert_eq!(
            r.timers[0],
            TimerCommand::Set {
                id: 1,
                duration: Duration::from_millis(100)
            }
        );
        assert_eq!(r.timers[1], TimerCommand::Kill { id: 2 });
    }

    #[test]
    fn default_is_pass_through() {
        let r: Response<(), ()> = Response::default();
        assert!(!r.consumed);
    }

    #[test]
    fn assert_helpers_pass() {
        let r: Response<i32, u8> = Response::emit_one(1)
            .with_timer(0, Duration::from_millis(50))
            .with_kill_timer(1);

        r.assert_consumed();
        r.assert_action_count(1);
        r.assert_timer_set(0);
        r.assert_timer_kill(1);
    }

    #[test]
    #[should_panic(expected = "expected consumed")]
    fn assert_consumed_panics_on_pass_through() {
        Response::<(), ()>::pass_through().assert_consumed();
    }

    #[test]
    #[should_panic(expected = "expected pass-through")]
    fn assert_pass_through_panics_on_consumed() {
        Response::<(), ()>::consume().assert_pass_through();
    }

    #[test]
    fn clone_preserves_all_fields() {
        let r = Response::emit(vec![1, 2])
            .with_timer(0u8, Duration::from_millis(50))
            .with_kill_timer(1);
        let c = r.clone();
        assert_eq!(r, c);
    }

    #[test]
    fn partial_eq_detects_differences() {
        let a: Response<i32, u8> = Response::emit_one(1);
        let b: Response<i32, u8> = Response::emit_one(2);
        assert_ne!(a, b);

        let c: Response<i32, u8> = Response::consume();
        let d: Response<i32, u8> = Response::pass_through();
        assert_ne!(c, d);
    }

    #[test]
    #[should_panic(expected = "expected TimerCommand::Set")]
    fn assert_timer_set_panics_when_missing() {
        Response::<(), u8>::consume().assert_timer_set(0);
    }

    #[test]
    #[should_panic(expected = "expected TimerCommand::Kill")]
    fn assert_timer_kill_panics_when_missing() {
        Response::<(), u8>::consume().assert_timer_kill(0);
    }

    #[test]
    #[should_panic(expected = "expected 3 actions")]
    fn assert_action_count_panics_on_mismatch() {
        Response::<i32, u8>::emit_one(1).assert_action_count(3);
    }

    #[test]
    fn emit_with_multiple_actions() {
        let r: Response<&str, ()> = Response::emit(vec!["a", "b", "c"]);
        assert!(r.consumed);
        assert_eq!(r.actions.len(), 3);
        r.assert_action_count(3);
    }
}
