use crate::response::Response;

/// A timed finite state machine.
///
/// Unlike a classic FSM where transitions depend only on `(State, Event)`,
/// a `TimedStateMachine` can express transitions that depend on the
/// **absence** of events within a time window. This is achieved by
/// including [`TimerCommand`](crate::response::TimerCommand)s in the
/// [`Response`], which the runtime interprets to set or kill timers.
///
/// The state machine itself has **no side effects**. It never calls
/// platform timer APIs directly. Instead, it returns a `Response`
/// containing timer commands, and the runtime executes them.
///
/// # Two kinds of input
///
/// - [`on_event`](Self::on_event): an external event (e.g., key press)
/// - [`on_timeout`](Self::on_timeout): a timer fired (the absence of an event was detected)
///
/// Both return the same `Response` type, so the runtime handles them uniformly.
///
/// # Example
///
/// ```
/// use std::time::Duration;
/// use timed_fsm::{TimedStateMachine, Response};
///
/// struct DebounceFilter {
///     pending: Option<bool>,
/// }
///
/// impl TimedStateMachine for DebounceFilter {
///     type Event = bool;       // GPIO level: true = high, false = low
///     type Action = bool;      // Confirmed level
///     type TimerId = ();
///
///     fn on_event(&mut self, event: bool) -> Response<bool, ()> {
///         self.pending = Some(event);
///         Response::consume()
///             .with_timer((), Duration::from_millis(20))
///     }
///
///     fn on_timeout(&mut self, _: ()) -> Response<bool, ()> {
///         match self.pending.take() {
///             Some(level) => Response::emit_one(level),
///             None => Response::pass_through(),
///         }
///     }
/// }
/// ```
pub trait TimedStateMachine {
    /// The event type fed into the state machine.
    type Event;

    /// The action type produced by transitions.
    type Action;

    /// The timer identifier type.
    ///
    /// Use `()` if only one timer is needed.
    /// Use an enum or integer for multiple concurrent timers.
    type TimerId: Copy + Eq + core::fmt::Debug;

    /// Process an external event and return the transition result.
    fn on_event(&mut self, event: Self::Event) -> Response<Self::Action, Self::TimerId>;

    /// Process a timer timeout and return the transition result.
    ///
    /// Called by the runtime when a timer previously requested via
    /// [`TimerCommand::Set`](crate::response::TimerCommand::Set) fires.
    fn on_timeout(&mut self, timer_id: Self::TimerId) -> Response<Self::Action, Self::TimerId>;
}
