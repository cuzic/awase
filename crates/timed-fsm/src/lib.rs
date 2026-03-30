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
//! | FSM calls `set_timer()` directly | Side effects in the FSM; untestable |
//! | Caller manages timers based on output | Timer logic leaks outside the FSM |
//! | **FSM returns timer commands in `Response`** | **Timer logic stays inside the FSM; caller just executes** |
//!
//! `timed-fsm` takes the third approach.
//!
//! # Core types
//!
//! - [`TimedStateMachine`] — the trait your state machine implements
//! - [`Response`] — the transition result (actions + timer commands + consumed flag)
//! - [`TimerCommand`] — set or kill a timer
//! - [`dispatch`] — connects a state machine to a runtime
//! - [`TimerRuntime`] / [`ActionExecutor`] — runtime traits for platform integration
//! - [`ShiftReduceParser`] / [`parse`] — shift-reduce parser framework with timer support
//!
//! # Example: debounce filter
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
//!     type Event = bool;
//!     type Action = bool;
//!     type TimerId = ();
//!
//!     fn on_event(&mut self, level: bool) -> Response<bool, ()> {
//!         self.pending = Some(level);
//!         Response::consume()
//!             .with_timer((), Duration::from_millis(20))
//!     }
//!
//!     fn on_timeout(&mut self, _: ()) -> Response<bool, ()> {
//!         match self.pending.take() {
//!             Some(level) => Response::emit_one(level),
//!             None => Response::pass_through(),
//!         }
//!     }
//! }
//!
//! // Test without any platform dependencies
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
//! // Timeout fires → confirmed as true
//! let r = d.on_timeout(());
//! assert_eq!(r.actions, vec![true]);
//! ```

mod dispatch;
mod machine;
pub mod parser;
mod response;

pub use dispatch::{dispatch, ActionExecutor, TimerRuntime};
pub use machine::TimedStateMachine;
pub use parser::{parse, ParseAction, ShiftReduceParser};
pub use response::{Response, TimerCommand};
