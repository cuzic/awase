//! A timed finite state machine framework.
//!
//! `timed-fsm` extends the classic finite state machine model with
//! **declarative timer commands**. Transitions return a [`Response`]
//! that includes not only output actions but also instructions to
//! set or kill timers. This allows the state machine to express
//! "if no event arrives within X ms, take action Y" without any
//! side effects or platform dependencies.
//!
//! # Why not a regular FSM?
//!
//! A regular FSM transitions on `(State, Event) → (State, Action)`.
//! It cannot express "the absence of an event" — there is no input
//! for "nothing happened for 100ms". You need a timer for that, and
//! the question is: who manages the timer?
//!
//! | Approach | Problem |
//! |----------|---------|
//! | FSM calls `set_timer()` directly | Side effects in the FSM; untestable without a platform |
//! | Caller manages timers based on output | Timer logic leaks outside the FSM; grammar is split |
//! | **FSM returns timer commands in `Response`** | **Timer logic stays inside the FSM; caller just executes** |
//!
//! `timed-fsm` takes the third approach.
//!
//! # Core types
//!
//! | Type | Role |
//! |------|------|
//! | [`TimedStateMachine`] | Trait your state machine implements |
//! | [`Response<A, T>`] | Transition result: actions + timer commands + consumed flag |
//! | [`TimerCommand<T>`] | Declarative instruction to set or kill a named timer |
//! | [`Response::dispatch`] | Connects a pure `Response` to runtime side effects |
//! | [`TimerRuntime`] | Trait for platform timer integration (Windows/Linux/macOS/test) |
//! | [`ActionExecutor`] | Trait for executing output actions in order |
//! | [`ShiftReduceParser`] | Extension for token-buffering grammars with timer support |
//! | [`ShiftReduceParser::parse`] | Main loop for a [`ShiftReduceParser`] |
//!
//! # Quick start
//!
//! The following example shows a **debounce filter**: it absorbs rapid
//! signal changes and only emits a confirmed level after a 20 ms quiet
//! period.
//!
//! ```
//! use std::time::Duration;
//! use timed_fsm::{TimedStateMachine, Response};
//!
//! /// A debounce filter that waits 20ms before confirming a level change.
//! struct Debounce {
//!     pending: Option<bool>,
//! }
//!
//! impl Debounce {
//!     fn new() -> Self { Self { pending: None } }
//! }
//!
//! impl TimedStateMachine for Debounce {
//!     type Event = bool;   // GPIO level
//!     type Action = bool;  // Confirmed level
//!     type TimerId = ();   // Only one timer needed
//!
//!     fn on_event(&mut self, level: bool) -> Response<bool, ()> {
//!         // Buffer the level and (re)start the debounce timer.
//!         self.pending = Some(level);
//!         Response::consume()
//!             .with_timer((), Duration::from_millis(20))
//!     }
//!
//!     fn on_timeout(&mut self, _: ()) -> Response<bool, ()> {
//!         // Quiet period elapsed — emit the last buffered level.
//!         match self.pending.take() {
//!             Some(level) => Response::emit_one(level),
//!             None => Response::pass_through(),
//!         }
//!     }
//! }
//! ```
//!
//! # Testing without platform dependencies
//!
//! Because the state machine never calls platform APIs directly, you
//! can test all transitions by calling [`TimedStateMachine::on_event`]
//! and [`TimedStateMachine::on_timeout`] directly — no OS timer
//! infrastructure required.
//!
//! ```
//! # use std::time::Duration;
//! # use timed_fsm::{TimedStateMachine, Response};
//! # struct Debounce { pending: Option<bool> }
//! # impl Debounce { fn new() -> Self { Self { pending: None } } }
//! # impl TimedStateMachine for Debounce {
//! #     type Event = bool;
//! #     type Action = bool;
//! #     type TimerId = ();
//! #     fn on_event(&mut self, level: bool) -> Response<bool, ()> {
//! #         self.pending = Some(level);
//! #         Response::consume().with_timer((), Duration::from_millis(20))
//! #     }
//! #     fn on_timeout(&mut self, _: ()) -> Response<bool, ()> {
//! #         match self.pending.take() {
//! #             Some(level) => Response::emit_one(level),
//! #             None => Response::pass_through(),
//! #         }
//! #     }
//! # }
//! let mut d = Debounce::new();
//!
//! // Noisy signal: high, low, high in quick succession
//! let r = d.on_event(true);
//! r.assert_consumed();
//! r.assert_timer_set(());
//!
//! let r = d.on_event(false);  // overwrites pending
//! let r = d.on_event(true);   // overwrites again
//!
//! // Simulate timeout firing — confirmed as true
//! let r = d.on_timeout(());
//! assert_eq!(r.actions, vec![true]);
//! ```
//!
//! # Connecting to a runtime
//!
//! At the boundary with the OS, implement [`TimerRuntime`] and
//! [`ActionExecutor`], then call [`Response::dispatch`] after every transition.
//!
//! ```
//! use std::time::Duration;
//! use timed_fsm::{Response, TimerRuntime, ActionExecutor};
//!
//! // Minimal in-memory timer stub for illustration.
//! struct MyTimers;
//! impl TimerRuntime for MyTimers {
//!     type TimerId = ();
//!     fn set_timer(&mut self, _id: (), _dur: Duration) {
//!         // e.g. SetTimer() on Windows, timerfd on Linux
//!     }
//!     fn kill_timer(&mut self, _id: ()) {
//!         // e.g. KillTimer() on Windows
//!     }
//! }
//!
//! struct MyExecutor;
//! impl ActionExecutor for MyExecutor {
//!     type Action = bool;
//!     fn execute(&mut self, actions: &[bool]) {
//!         // e.g. SendInput() on Windows, uinput write on Linux
//!         for &a in actions { let _ = a; }
//!     }
//! }
//!
//! // In your event loop:
//! let response = Response::<bool, ()>::emit_one(true);
//! let consumed = response.dispatch(&mut MyTimers, &mut MyExecutor);
//! assert!(consumed);
//! ```
//!
//! # Multiple timers
//!
//! When a state machine needs more than one concurrent timer, use an
//! enum (or any `Copy + Eq + Debug` type) as `TimerId`.
//!
//! ```
//! use std::time::Duration;
//! use timed_fsm::{TimedStateMachine, Response};
//!
//! #[derive(Clone, Copy, Debug, PartialEq, Eq)]
//! enum Timer {
//!     Debounce,
//!     Repeat,
//! }
//!
//! struct KeyFilter {
//!     key: Option<u8>,
//! }
//!
//! impl TimedStateMachine for KeyFilter {
//!     type Event = u8;
//!     type Action = u8;
//!     type TimerId = Timer;
//!
//!     fn on_event(&mut self, key: u8) -> Response<u8, Timer> {
//!         self.key = Some(key);
//!         Response::consume()
//!             .with_timer(Timer::Debounce, Duration::from_millis(10))
//!             .with_kill_timer(Timer::Repeat)
//!     }
//!
//!     fn on_timeout(&mut self, id: Timer) -> Response<u8, Timer> {
//!         match id {
//!             Timer::Debounce => match self.key {
//!                 Some(k) => Response::emit_one(k)
//!                     .with_timer(Timer::Repeat, Duration::from_millis(500)),
//!                 None => Response::pass_through(),
//!             },
//!             Timer::Repeat => match self.key {
//!                 Some(k) => Response::emit_one(k)
//!                     .with_timer(Timer::Repeat, Duration::from_millis(100)),
//!                 None => Response::pass_through(),
//!             },
//!         }
//!     }
//! }
//! ```
//!
//! # Shift-reduce parser extension
//!
//! When the decision about a token depends on tokens that arrive
//! *after* it (e.g., distinguishing a single key press from a chord),
//! a plain `TimedStateMachine` is not enough. The [`parser`] module
//! provides a [`ShiftReduceParser`] trait and a [`parse`] driver that
//! buffer tokens until a pattern is recognized or a timer forces a
//! decision. See the module documentation for details and examples.
//!
//! # Use cases
//!
//! | Domain | Event | Timer role |
//! |--------|-------|------------|
//! | Keyboard firmware | Key press / release | Chord disambiguation timeout |
//! | GPIO debounce | Signal edge | Quiet-period confirmation |
//! | UI input | Mouse click | Double-click detection window |
//! | Protocol framing | Byte received | Inter-frame gap detection |
//! | IME / input method | Composition key | Commit-after-idle timeout |
//!
//! # No dependencies
//!
//! `timed-fsm` has no runtime dependencies beyond `std`.

pub mod clock;
mod dispatch;
mod machine;
pub mod parser;
mod response;

pub use clock::{Clock, ManualClock, MonotonicClock};
pub use dispatch::{ActionExecutor, TimerRuntime};
pub use machine::TimedStateMachine;
pub use parser::{ParseAction, ShiftReduceParser};
pub use response::{Response, TimerCommand};
