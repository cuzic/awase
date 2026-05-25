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
/// | Method | Meaning |
/// |--------|---------|
/// | [`on_event`](Self::on_event) | An external event arrived (key press, byte received, …) |
/// | [`on_timeout`](Self::on_timeout) | A previously requested timer fired — the absence of events was detected |
///
/// Both methods return the same [`Response`] type, so the runtime can
/// handle them uniformly via [`dispatch`](crate::dispatch::dispatch).
///
/// # Timer IDs
///
/// The [`TimerId`](Self::TimerId) type identifies timers within the
/// state machine. Choose the type that fits your needs:
///
/// | `TimerId` | When to use |
/// |-----------|-------------|
/// | `()` | Exactly one timer — simplest case |
/// | `enum Timer { … }` | Multiple named timers with distinct roles |
/// | `u8` / `u32` | Indexed timers generated dynamically |
///
/// # Invariants
///
/// - `on_event` and `on_timeout` **must not produce side effects**.
///   All effects are expressed through the returned [`Response`].
/// - A `Response` with `consumed = false` means the event was not
///   handled by this machine and should be forwarded to the next
///   handler in the chain. The machine's internal state must remain
///   unchanged in that case (as if `on_event` was never called).
/// - Implementations should be deterministic: the same sequence of
///   inputs always produces the same sequence of outputs.
///
/// # Example: single timer
///
/// ```
/// use std::time::Duration;
/// use timed_fsm::{TimedStateMachine, Response};
///
/// struct DebounceFilter {
///     pending: Option<bool>,
/// }
///
/// impl DebounceFilter {
///     fn new() -> Self { Self { pending: None } }
/// }
///
/// impl TimedStateMachine for DebounceFilter {
///     type Event = bool;       // GPIO level: true = high, false = low
///     type Action = bool;      // Confirmed level
///     type TimerId = ();       // One debounce timer
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
///
/// # Example: multiple timers
///
/// When a state machine needs more than one concurrent timer, use an
/// enum as `TimerId` so each timer has a descriptive name.
///
/// ```
/// use std::time::Duration;
/// use timed_fsm::{TimedStateMachine, Response};
///
/// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// enum Timer { Debounce, RepeatDelay, RepeatInterval }
///
/// struct KeyRepeater { key: Option<u8> }
///
/// impl KeyRepeater {
///     fn new() -> Self { Self { key: None } }
/// }
///
/// impl TimedStateMachine for KeyRepeater {
///     type Event  = Option<u8>;  // Some(key) = pressed, None = released
///     type Action = u8;
///     type TimerId = Timer;
///
///     fn on_event(&mut self, event: Option<u8>) -> Response<u8, Timer> {
///         match event {
///             Some(key) => {
///                 self.key = Some(key);
///                 Response::consume()
///                     .with_timer(Timer::Debounce,     Duration::from_millis(10))
///                     .with_kill_timer(Timer::RepeatDelay)
///                     .with_kill_timer(Timer::RepeatInterval)
///             }
///             None => {
///                 self.key = None;
///                 Response::consume()
///                     .with_kill_timer(Timer::Debounce)
///                     .with_kill_timer(Timer::RepeatDelay)
///                     .with_kill_timer(Timer::RepeatInterval)
///             }
///         }
///     }
///
///     fn on_timeout(&mut self, id: Timer) -> Response<u8, Timer> {
///         match id {
///             Timer::Debounce => match self.key {
///                 Some(k) => Response::emit_one(k)
///                     .with_timer(Timer::RepeatDelay, Duration::from_millis(500)),
///                 None => Response::pass_through(),
///             },
///             Timer::RepeatDelay => match self.key {
///                 Some(k) => Response::emit_one(k)
///                     .with_timer(Timer::RepeatInterval, Duration::from_millis(50)),
///                 None => Response::pass_through(),
///             },
///             Timer::RepeatInterval => match self.key {
///                 Some(k) => Response::emit_one(k)
///                     .with_timer(Timer::RepeatInterval, Duration::from_millis(50)),
///                 None => Response::pass_through(),
///             },
///         }
///     }
/// }
/// ```
pub trait TimedStateMachine {
    /// The event type fed into the state machine via [`on_event`](Self::on_event).
    type Event;

    /// The action type produced by transitions.
    ///
    /// Actions are collected in [`Response::actions`](crate::Response::actions)
    /// and forwarded to [`ActionExecutor::execute`](crate::ActionExecutor::execute)
    /// by the runtime.
    type Action;

    /// The timer identifier type.
    ///
    /// Use `()` if only one timer is needed.
    /// Use an enum or integer for multiple concurrent timers.
    ///
    /// Must implement `Copy + Eq + Debug` so the runtime and assertion
    /// helpers can compare and display timer IDs.
    type TimerId: Copy + Eq + core::fmt::Debug;

    /// Process an external event and return the transition result.
    ///
    /// The returned [`Response`] describes what actions to emit and
    /// which timers to set or kill. The runtime then executes those
    /// effects via [`dispatch`](crate::dispatch::dispatch).
    fn on_event(&mut self, event: Self::Event) -> Response<Self::Action, Self::TimerId>;

    /// Process a timer timeout and return the transition result.
    ///
    /// Called by the runtime when a timer previously requested via
    /// [`TimerCommand::Set`](crate::response::TimerCommand::Set) fires.
    /// The `timer_id` identifies which timer expired, allowing the state
    /// machine to distinguish multiple concurrent timers.
    fn on_timeout(&mut self, timer_id: Self::TimerId) -> Response<Self::Action, Self::TimerId>;
}
