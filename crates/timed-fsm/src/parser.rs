//! Shift-reduce parser framework with timer support.
//!
//! A streaming parser that receives tokens one at a time and produces
//! output actions. Tokens can be buffered (Shift), recognized as patterns
//! (Reduce), or passed through. Timers enable "timeout = forced reduce"
//! semantics.
//!
//! # What is a shift-reduce parser?
//!
//! A shift-reduce parser builds up a stack (or buffer) of tokens and
//! only decides how to interpret them once it has seen enough context.
//! This is necessary whenever the correct interpretation of a token
//! depends on tokens that have **not arrived yet**.
//!
//! Classic example: the first key of a two-key chord. When you see key A,
//! you do not know yet whether:
//! - The user intends a chord `A+B` (requires waiting for B), or
//! - The user intends a single-key stroke `A` (requires a timeout).
//!
//! A plain [`TimedStateMachine`](crate::TimedStateMachine) handles one
//! token per transition. This module's [`ShiftReduceParser`] lets the
//! grammar say "buffer this token and wait" (`Shift`) or "I have
//! recognized a complete pattern" (`Reduce`), with a timer forcing a
//! `Reduce` when no further token arrives in time.
//!
//! # When to use `ShiftReduceParser` vs `TimedStateMachine`
//!
//! | Scenario | Use |
//! |----------|-----|
//! | Each event is self-contained (debounce, key repeat) | [`TimedStateMachine`](crate::TimedStateMachine) |
//! | A pattern spans multiple tokens (chord, double-click) | [`ShiftReduceParser`] |
//! | You need to re-process a token after a partial reduce | [`ShiftReduceParser`] with [`ReduceAndContinue`](ParseAction::ReduceAndContinue) |
//!
//! # Example: two-key chord detection
//!
//! The following sketch detects a simultaneous press of keys `A` and `B`
//! within a 50 ms window and emits a `Chord` action; a single `A` or `B`
//! press outside the window passes through.
//!
//! ```
//! use std::time::Duration;
//! use timed_fsm::parser::{ShiftReduceParser, ParseAction};
//! use timed_fsm::TimerCommand;
//!
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! enum Key { A, B, Other(u8) }
//!
//! #[derive(Debug, PartialEq)]
//! enum Out { Chord, Key(Key) }
//!
//! struct ChordParser { pending: Option<Key> }
//! impl ChordParser {
//!     fn new() -> Self { Self { pending: None } }
//! }
//!
//! impl ShiftReduceParser for ChordParser {
//!     type Action      = Out;
//!     type Token       = Key;
//!     type TimerId     = ();
//!     type ReduceRecord = ();
//!
//!     fn decide(&mut self, token: &Key)
//!         -> ParseAction<Out, Key, (), ()>
//!     {
//!         match (self.pending, token) {
//!             // No pending key yet — shift and open a 50 ms chord window.
//!             (None, &k @ (Key::A | Key::B)) => {
//!                 self.pending = Some(k);
//!                 ParseAction::Shift {
//!                     timers: vec![TimerCommand::Set {
//!                         id: (),
//!                         duration: Duration::from_millis(50),
//!                     }],
//!                 }
//!             }
//!             // Second key arrived inside the window — chord recognized.
//!             (Some(Key::A), Key::B) | (Some(Key::B), Key::A) => {
//!                 self.pending = None;
//!                 ParseAction::Reduce {
//!                     actions: vec![Out::Chord],
//!                     record: (),
//!                     timers: vec![TimerCommand::Kill { id: () }],
//!                 }
//!             }
//!             // Unrelated key — pass through.
//!             _ => ParseAction::PassThrough { timers: vec![] },
//!         }
//!     }
//!
//!     fn on_reduce(&mut self, (): ()) {}
//! }
//!
//! // Both keys pressed in sequence → chord.
//! let mut p = ChordParser::new();
//! let _ = p.parse(Key::A);  // Shift
//! let r = p.parse(Key::B);  // Reduce → Chord
//! assert_eq!(r.actions, vec![Out::Chord]);
//! ```

use crate::response::{Response, TimerCommand};

/// Parser action: the result of examining a token in the current state.
///
/// Returned by [`ShiftReduceParser::decide`] and consumed by the [`parse`]
/// driver loop.
///
/// | Variant | Loop continues? | Actions accumulated? |
/// |---------|----------------|----------------------|
/// | `Shift` | No (terminal) | No — wait for more input |
/// | `Reduce` | No (terminal) | Yes — emit and finish |
/// | `ReduceAndContinue` | Yes — re-process `remaining` | Yes — partial emit, then loop |
/// | `PassThrough` | No (terminal) | No (or yes if prior reduces accumulated some) |
#[derive(Debug)]
pub enum ParseAction<A, Token, T, R = ()> {
    /// Buffer the token and wait for more input.
    ///
    /// The parser absorbs the token into its internal buffer (if any) and
    /// sets a timer to force a decision if no further token arrives in time.
    /// The [`parse`] loop returns immediately — the token is consumed.
    Shift {
        /// Timer commands to execute (typically a `Set` to open the chord window).
        timers: Vec<TimerCommand<T>>,
    },

    /// Recognize a complete pattern and emit output. Terminal action.
    ///
    /// [`parse`] calls [`ShiftReduceParser::on_reduce`] with `record`, then
    /// returns a consumed [`Response`] with all accumulated
    /// actions and the given timer commands.
    Reduce {
        /// Output actions to emit.
        actions: Vec<A>,
        /// Metadata for bookkeeping (passed to [`ShiftReduceParser::on_reduce`]).
        record: R,
        /// Timer commands to execute (e.g., `Kill` the chord window timer).
        timers: Vec<TimerCommand<T>>,
    },

    /// Recognize a partial pattern, emit output, and re-process the remaining token.
    ///
    /// Use this when two logically independent patterns are packed into a
    /// single token stream and the second pattern starts immediately after
    /// the first ends. For example:
    ///
    /// - A "forced reduce" on timeout that needs to flush buffered tokens and
    ///   then re-process the triggering event through the same grammar.
    /// - A two-byte sequence where the first byte is a complete symbol and the
    ///   second byte starts a new one.
    ///
    /// [`parse`] calls [`on_reduce`](ShiftReduceParser::on_reduce) with `record`,
    /// accumulates `actions`, and loops by calling `decide` on `remaining`.
    /// Because the loop continues, the final `consumed` flag is `true` even if
    /// the eventual terminal action is `PassThrough`.
    ReduceAndContinue {
        /// Output actions to emit for the recognized partial pattern.
        actions: Vec<A>,
        /// Metadata for bookkeeping (passed to [`ShiftReduceParser::on_reduce`]).
        record: R,
        /// The remaining token to re-process in the next loop iteration.
        remaining: Token,
    },

    /// This parser does not handle the current token.
    ///
    /// Pass the event to the next handler in the chain. If no actions have
    /// been accumulated by prior [`ReduceAndContinue`](Self::ReduceAndContinue)
    /// steps, `consumed` will be `false`; otherwise `true` (the partial results
    /// are still returned).
    PassThrough {
        /// Timer commands to execute (usually empty for pass-through).
        timers: Vec<TimerCommand<T>>,
    },
}

/// A shift-reduce parser that processes tokens with timer support.
///
/// Implement this trait to define your parser's grammar (action table).
/// The framework provides the main loop via [`parse()`].
///
/// # Implementing `decide`
///
/// `decide` is the **action table** of the parser. It receives a reference
/// to the current token together with whatever state is stored in `self`,
/// and returns a [`ParseAction`] describing what to do next.
///
/// Typical skeleton:
///
/// ```
/// use timed_fsm::parser::{ShiftReduceParser, ParseAction};
/// use timed_fsm::TimerCommand;
/// use std::time::Duration;
///
/// #[derive(Clone, Copy, Debug, PartialEq)]
/// enum Key { Shift, A, Other }
///
/// struct MyParser { pending_shift: bool }
///
/// impl ShiftReduceParser for MyParser {
///     type Action      = String;
///     type Token       = Key;
///     type TimerId     = ();
///     type ReduceRecord = ();
///
///     fn decide(&mut self, token: &Key)
///         -> ParseAction<String, Key, (), ()>
///     {
///         match (self.pending_shift, token) {
///             // Shift key down — buffer and open chord window.
///             (false, Key::Shift) => {
///                 self.pending_shift = true;
///                 ParseAction::Shift {
///                     timers: vec![TimerCommand::Set {
///                         id: (),
///                         duration: Duration::from_millis(50),
///                     }],
///                 }
///             }
///             // Shift + A chord recognized.
///             (true, Key::A) => {
///                 self.pending_shift = false;
///                 ParseAction::Reduce {
///                     actions: vec!["ShiftA".to_string()],
///                     record: (),
///                     timers: vec![TimerCommand::Kill { id: () }],
///                 }
///             }
///             // Unrecognized — pass through.
///             _ => ParseAction::PassThrough { timers: vec![] },
///         }
///     }
///
///     fn on_reduce(&mut self, (): ()) { /* update history / stats */ }
/// }
///
/// let mut p = MyParser { pending_shift: false };
/// let _ = p.parse(Key::Shift);   // Shift
/// let r = p.parse(Key::A);       // Reduce
/// assert_eq!(r.actions, vec!["ShiftA".to_string()]);
/// ```
pub trait ShiftReduceParser {
    /// Output action type.
    type Action;
    /// Input token type.
    type Token;
    /// Timer ID type.
    type TimerId;
    /// Metadata attached to each `Reduce` step (e.g., history tracking).
    ///
    /// Use `()` if no per-reduce bookkeeping is needed.
    type ReduceRecord;

    /// Cycle-detection cap: the maximum number of `decide` calls [`parse`]
    /// will make within a **single** `parse()` invocation before it assumes
    /// the grammar is stuck in a [`ReduceAndContinue`](ParseAction::ReduceAndContinue)
    /// cycle and stops.
    ///
    /// # Why this exists
    ///
    /// [`ReduceAndContinue`](ParseAction::ReduceAndContinue) re-feeds a
    /// `remaining` token back into `decide`. Termination is therefore a
    /// **grammar obligation**, not a structural guarantee: if a bug makes
    /// `decide` keep returning `ReduceAndContinue` for the same effective
    /// state/token, the driver loop would spin forever, hanging the calling
    /// thread with no panic and no log. In the `awase` thumb-shift keyboard
    /// engine this exact bug once turned a one-line `go_idle()` regression into
    /// a total keyboard-input lockup on a low-level Windows hook thread.
    ///
    /// This cap converts that silent infinite hang into a bounded, observable
    /// failure. When it is exceeded, [`parse`] fails a `debug_assert!` (so the
    /// cycle is caught fast and loudly in tests/CI and under mutation testing),
    /// and in release builds degrades gracefully — returning the partial actions
    /// accumulated so far with no timer commands — rather than crashing the host
    /// process, honoring the crate's *infallible transitions* principle.
    ///
    /// # Choosing a value
    ///
    /// A legitimate `ReduceAndContinue` chain is short: each step should make
    /// progress toward a terminal by consuming its `remaining` token. Real usage
    /// in `awase` never exceeds **2** iterations per `parse()` (measured across
    /// its full test suite). The default of `1024` leaves several orders of
    /// magnitude of headroom for unusually deep grammars while still terminating
    /// in microseconds. Override it only if you have a genuine, measured need for
    /// a longer chain (or, rarely, a smaller cap to fail faster in tests).
    const MAX_REDUCE_CONTINUE_STEPS: usize = 1024;

    /// The action table: given the current state and input token, decide what to do.
    ///
    /// This method may mutate internal state (e.g., enter a pending state,
    /// push a token onto a buffer). It is called once per token (or once per
    /// `remaining` token in a `ReduceAndContinue` loop iteration) by [`parse`].
    fn decide(
        &mut self,
        token: &Self::Token,
    ) -> ParseAction<Self::Action, Self::Token, Self::TimerId, Self::ReduceRecord>;

    /// Called after each `Reduce` or `ReduceAndContinue` to update internal bookkeeping.
    ///
    /// Receives the `record` from the matching [`ParseAction`]. Use this to
    /// maintain reduce-count statistics, history logs, or any per-reduce state
    /// that should not live inside `decide`.
    fn on_reduce(&mut self, record: Self::ReduceRecord);

    /// Process a token through this parser, producing a [`Response`].
    ///
    /// # Loop semantics
    ///
    /// The driver loop calls [`decide`](Self::decide) on the current token.
    /// Depending on the result:
    ///
    /// | Result | Actions accumulated | `on_reduce` called | Next iteration |
    /// |--------|--------------------|--------------------|----------------|
    /// | `Shift` | — | No | Return immediately |
    /// | `Reduce` | Yes | Yes | Return immediately |
    /// | `ReduceAndContinue` | Yes | Yes | Loop with `remaining` |
    /// | `PassThrough` | — | No | Return immediately |
    ///
    /// # `consumed` semantics
    ///
    /// The returned `Response::consumed` flag is `true` if any of the
    /// following is true:
    ///
    /// - The terminal action was `Shift` or `Reduce` (event was handled), **or**
    /// - At least one [`ReduceAndContinue`](ParseAction::ReduceAndContinue) step
    ///   accumulated actions before a `PassThrough` was reached.
    ///
    /// # Termination
    ///
    /// The loop terminates when `decide` returns a terminal variant (`Shift`,
    /// `Reduce`, `PassThrough`), which causes an immediate `return`. A
    /// `ReduceAndContinue` result re-feeds a `remaining` token, so a
    /// **well-formed grammar must guarantee no cycle** — every
    /// `ReduceAndContinue` must make progress toward a terminal.
    ///
    /// As a defensive backstop against a grammar bug that violates this
    /// obligation (which would otherwise hang the calling thread forever with
    /// no diagnostic), the loop is capped at
    /// [`MAX_REDUCE_CONTINUE_STEPS`](Self::MAX_REDUCE_CONTINUE_STEPS) `decide`
    /// calls. Exceeding the cap fails a `debug_assert!` in debug/test builds and
    /// degrades gracefully in release builds (returns the partial actions with no
    /// timers). See that constant's docs for the rationale and the `awase`
    /// incident that motivated it.
    fn parse(&mut self, initial: Self::Token) -> Response<Self::Action, Self::TimerId> {
        let mut actions: Vec<Self::Action> = Vec::new();
        let mut current = Some(initial);

        // Safety net against a buggy grammar whose `decide` returns
        // `ReduceAndContinue` without ever reaching a terminal (see the
        // `MAX_REDUCE_CONTINUE_STEPS` docs). Each loop iteration is one
        // `decide` call; terminal variants `return`, so `steps` only keeps
        // growing while `ReduceAndContinue` is chosen.
        let mut steps: usize = 0;

        while let Some(token) = current.take() {
            steps += 1;
            if steps > Self::MAX_REDUCE_CONTINUE_STEPS {
                // A grammar cycle. In a debug/test build fail loudly and fast so
                // the bug is found at development time (a silent infinite hang is
                // the worst possible symptom — on `awase` it locks up the whole
                // keyboard-hook thread). In a release build we must honor the
                // crate's "infallible transitions" contract and never crash the
                // host process, so we degrade gracefully: return whatever partial
                // actions accumulated, with no timers, and let the caller move on.
                debug_assert!(
                    false,
                    "ShiftReduceParser::parse exceeded MAX_REDUCE_CONTINUE_STEPS \
                     ({}) — decide() is stuck returning ReduceAndContinue for the \
                     same effective state/token (a grammar cycle). This would have \
                     hung the calling thread forever. Fix decide()/on_reduce so the \
                     re-processed `remaining` token makes progress toward a terminal \
                     (Shift/Reduce/PassThrough); see the go_idle() incident noted on \
                     ReduceAndContinue.",
                    Self::MAX_REDUCE_CONTINUE_STEPS
                );
                return build_response(actions, false, Vec::new());
            }
            match self.decide(&token) {
                ParseAction::Shift { timers } => {
                    return build_response(actions, true, timers);
                }
                ParseAction::Reduce {
                    actions: output,
                    record,
                    timers,
                } => {
                    actions.extend(output);
                    self.on_reduce(record);
                    return build_response(actions, true, timers);
                }
                ParseAction::ReduceAndContinue {
                    actions: output,
                    record,
                    remaining,
                } => {
                    actions.extend(output);
                    self.on_reduce(record);
                    current = Some(remaining);
                }
                ParseAction::PassThrough { timers } => {
                    let consumed = !actions.is_empty();
                    return build_response(actions, consumed, timers);
                }
            }
        }

        unreachable!("parse loop must terminate via Shift, Reduce, or PassThrough")
    }
}

fn build_response<A, T>(
    actions: Vec<A>,
    consumed: bool,
    timers: Vec<TimerCommand<T>>,
) -> Response<A, T> {
    Response {
        consumed: consumed || !actions.is_empty(),
        actions,
        timers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A simple calculator parser for testing.
    /// Tokens are i32 values. When it sees 0, it reduces and emits the sum.
    /// Negative values pass through.
    struct SumParser {
        buffer: Vec<i32>,
        reduce_count: usize,
    }

    impl SumParser {
        fn new() -> Self {
            Self {
                buffer: Vec::new(),
                reduce_count: 0,
            }
        }
    }

    impl ShiftReduceParser for SumParser {
        type Action = i32;
        type Token = i32;
        type TimerId = u8;
        type ReduceRecord = usize; // number of items reduced

        fn decide(&mut self, token: &i32) -> ParseAction<i32, i32, u8, usize> {
            match *token {
                t if t < 0 => ParseAction::PassThrough { timers: vec![] },
                0 => {
                    // Reduce: emit sum of buffer
                    let sum: i32 = self.buffer.drain(..).sum();
                    let count = usize::from(sum != 0);
                    ParseAction::Reduce {
                        actions: if sum == 0 { vec![] } else { vec![sum] },
                        record: count,
                        timers: vec![TimerCommand::Kill { id: 0 }],
                    }
                }
                t => {
                    // Shift: buffer the value
                    self.buffer.push(t);
                    ParseAction::Shift {
                        timers: vec![TimerCommand::Set {
                            id: 0,
                            duration: Duration::from_millis(100),
                        }],
                    }
                }
            }
        }

        fn on_reduce(&mut self, record: usize) {
            self.reduce_count += record;
        }
    }

    #[test]
    fn shift_buffers_and_returns_consumed() {
        let mut p = SumParser::new();
        let r = p.parse(5);
        assert!(r.consumed);
        assert!(r.actions.is_empty());
        assert_eq!(r.timers.len(), 1);
        r.assert_timer_set(0);
        assert_eq!(p.buffer, vec![5]);
    }

    #[test]
    fn reduce_emits_sum() {
        let mut p = SumParser::new();
        p.buffer = vec![3, 7];
        let r = p.parse(0);
        assert!(r.consumed);
        assert_eq!(r.actions, vec![10]);
        r.assert_timer_kill(0);
        assert_eq!(p.reduce_count, 1);
    }

    #[test]
    fn pass_through_not_consumed() {
        let mut p = SumParser::new();
        let r = p.parse(-1);
        assert!(!r.consumed);
        assert!(r.actions.is_empty());
    }

    /// A parser that uses `ReduceAndContinue`.
    /// On token 99, it reduces current buffer and re-processes token 0 (which triggers final reduce).
    struct SplitParser {
        buffer: Vec<i32>,
        reduce_count: usize,
    }

    impl SplitParser {
        fn new() -> Self {
            Self {
                buffer: Vec::new(),
                reduce_count: 0,
            }
        }
    }

    impl ShiftReduceParser for SplitParser {
        type Action = String;
        type Token = i32;
        type TimerId = u8;
        type ReduceRecord = ();

        fn decide(&mut self, token: &i32) -> ParseAction<String, i32, u8, ()> {
            match *token {
                99 => {
                    let msg = format!("split:{}", self.buffer.len());
                    self.buffer.clear();
                    ParseAction::ReduceAndContinue {
                        actions: vec![msg],
                        record: (),
                        remaining: 0,
                    }
                }
                0 => ParseAction::Reduce {
                    actions: vec!["done".to_string()],
                    record: (),
                    timers: vec![],
                },
                t => {
                    self.buffer.push(t);
                    ParseAction::Shift { timers: vec![] }
                }
            }
        }

        fn on_reduce(&mut self, _record: ()) {
            self.reduce_count += 1;
        }
    }

    #[test]
    fn reduce_and_continue_chains() {
        let mut p = SplitParser::new();
        p.buffer = vec![1, 2, 3];

        let r = p.parse(99);
        assert!(r.consumed);
        assert_eq!(r.actions, vec!["split:3".to_string(), "done".to_string()]);
        // on_reduce called twice (once for ReduceAndContinue, once for Reduce)
        assert_eq!(p.reduce_count, 2);
    }

    #[test]
    fn pass_through_after_reduce_and_continue_is_consumed() {
        /// Parser that does `ReduceAndContinue` then the remaining token passes through.
        struct RacParser;

        impl ShiftReduceParser for RacParser {
            type Action = &'static str;
            type Token = i32;
            type TimerId = u8;
            type ReduceRecord = ();

            fn decide(&mut self, token: &i32) -> ParseAction<&'static str, i32, u8, ()> {
                match *token {
                    1 => ParseAction::ReduceAndContinue {
                        actions: vec!["first"],
                        record: (),
                        remaining: -1,
                    },
                    _ => ParseAction::PassThrough { timers: vec![] },
                }
            }

            fn on_reduce(&mut self, (): ()) {}
        }

        let mut p = RacParser;
        let r = p.parse(1);
        // Has accumulated actions from the ReduceAndContinue, so consumed is true
        assert!(r.consumed);
        assert_eq!(r.actions, vec!["first"]);
    }

    /// A deliberately broken parser: `decide` returns `ReduceAndContinue`
    /// forever (the `remaining` token never makes progress). Before the cycle
    /// cap existed, `parse` would loop on this input until the thread was killed
    /// — the exact silent-infinite-hang class of bug that a `go_idle()`
    /// regression triggered in `awase` (a `TIMEOUT`, not a `MISSED`, under
    /// mutation testing).
    ///
    /// The cap is overridden to a tiny value so the test proves the mechanism
    /// without spinning up to the 1024 default.
    struct InfiniteParser {
        reduce_count: usize,
    }

    impl ShiftReduceParser for InfiniteParser {
        type Action = i32;
        type Token = i32;
        type TimerId = u8;
        type ReduceRecord = ();

        // Small cap: prove the backstop fires promptly.
        const MAX_REDUCE_CONTINUE_STEPS: usize = 8;

        fn decide(&mut self, _token: &i32) -> ParseAction<i32, i32, u8, ()> {
            // Never reaches a terminal — always re-processes the same token.
            ParseAction::ReduceAndContinue {
                actions: vec![],
                record: (),
                remaining: 0,
            }
        }

        fn on_reduce(&mut self, (): ()) {
            self.reduce_count += 1;
        }
    }

    /// The cycle cap must make `parse` terminate instead of hanging forever.
    ///
    /// This test itself is the proof of termination: if the backstop were
    /// removed, this test would hang the whole `timed-fsm` suite (caught in CI
    /// as a timeout rather than passing). We accept either landing:
    /// - debug/test builds (`debug_assertions` on): the `debug_assert!` fires
    ///   and the call panics — fast, loud, debuggable.
    /// - release builds: graceful degradation returns a bounded `Response`.
    ///
    /// Either way, control returns quickly; the point is that it does not hang.
    #[test]
    fn reduce_and_continue_cycle_terminates_instead_of_hanging() {
        // Silence the panic hook so the (expected) debug_assert backtrace does
        // not spam test output.
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut p = InfiniteParser { reduce_count: 0 };
            p.parse(0)
        }));
        std::panic::set_hook(prev_hook);

        match outcome {
            // debug builds: the debug_assert! fired. Termination proven by the
            // fact that we got here at all (not a hang).
            Err(_) => {}
            // release builds: graceful degradation. Response is bounded and safe.
            Ok(resp) => {
                assert!(
                    resp.actions.is_empty(),
                    "cycle produced no output actions, so the degraded response carries none"
                );
                assert!(
                    resp.timers.is_empty(),
                    "degraded response must not emit timer commands"
                );
            }
        }
    }
}
