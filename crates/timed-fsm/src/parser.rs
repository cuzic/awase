//! Shift-reduce parser framework with timer support.
//!
//! A streaming parser that receives tokens one at a time and produces
//! output actions. Tokens can be buffered (Shift), recognized as patterns
//! (Reduce), or passed through. Timers enable "timeout = forced reduce"
//! semantics.

use crate::response::{Response, TimerCommand};

/// Parser action: the result of examining a token in the current state.
#[derive(Debug)]
pub enum ParseAction<A, Token, T, R = ()> {
    /// Buffer the token and wait for more input. Sets timers.
    Shift {
        /// Timer commands to execute (e.g., set a timeout for forced reduce).
        timers: Vec<TimerCommand<T>>,
    },
    /// Recognize a pattern and emit output. Final action.
    Reduce {
        /// Output actions to emit.
        actions: Vec<A>,
        /// Metadata for bookkeeping (passed to [`ShiftReduceParser::on_reduce`]).
        record: R,
        /// Timer commands to execute after the reduce.
        timers: Vec<TimerCommand<T>>,
    },
    /// Recognize a partial pattern, emit output, and re-process the remaining token.
    ReduceAndContinue {
        /// Output actions to emit for the recognized part.
        actions: Vec<A>,
        /// Metadata for bookkeeping (passed to [`ShiftReduceParser::on_reduce`]).
        record: R,
        /// The remaining token to re-process in the next loop iteration.
        remaining: Token,
    },
    /// Not handled by this parser. Pass through to the next handler.
    PassThrough {
        /// Timer commands to execute (usually empty for pass-through).
        timers: Vec<TimerCommand<T>>,
    },
}

/// A shift-reduce parser that processes tokens with timer support.
///
/// Implement this trait to define your parser's grammar (action table).
/// The framework provides the main loop via [`parse()`].
pub trait ShiftReduceParser {
    /// Output action type
    type Action;
    /// Input token type
    type Token;
    /// Timer ID type
    type TimerId;
    /// Metadata attached to each `Reduce` step (e.g., history tracking)
    type ReduceRecord;

    /// The action table: given the current state and input token, decide what to do.
    ///
    /// This method may mutate internal state (e.g., enter a pending state).
    /// It is called once per token in the parse loop.
    fn decide(
        &mut self,
        token: &Self::Token,
    ) -> ParseAction<Self::Action, Self::Token, Self::TimerId, Self::ReduceRecord>;

    /// Called after each `Reduce` or `ReduceAndContinue` to update internal bookkeeping.
    fn on_reduce(&mut self, record: Self::ReduceRecord);
}

/// Process a token through a shift-reduce parser, producing a Response.
///
/// The loop handles:
/// - Accumulating actions across multiple reduce steps
/// - Calling `on_reduce` after each reduce
/// - Re-processing remaining tokens from `ReduceAndContinue`
/// - Building a final `Response` with accumulated actions and timer commands
pub fn parse<P>(parser: &mut P, initial: P::Token) -> Response<P::Action, P::TimerId>
where
    P: ShiftReduceParser,
{
    let mut actions: Vec<P::Action> = Vec::new();
    let mut current = Some(initial);

    while let Some(token) = current.take() {
        match parser.decide(&token) {
            ParseAction::Shift { timers } => {
                return build_response(actions, true, timers);
            }
            ParseAction::Reduce {
                actions: output,
                record,
                timers,
            } => {
                actions.extend(output);
                parser.on_reduce(record);
                return build_response(actions, true, timers);
            }
            ParseAction::ReduceAndContinue {
                actions: output,
                record,
                remaining,
            } => {
                actions.extend(output);
                parser.on_reduce(record);
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
                    let count = if sum == 0 { 0 } else { 1 };
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
        let r = parse(&mut p, 5);
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
        let r = parse(&mut p, 0);
        assert!(r.consumed);
        assert_eq!(r.actions, vec![10]);
        r.assert_timer_kill(0);
        assert_eq!(p.reduce_count, 1);
    }

    #[test]
    fn pass_through_not_consumed() {
        let mut p = SumParser::new();
        let r = parse(&mut p, -1);
        assert!(!r.consumed);
        assert!(r.actions.is_empty());
    }

    /// A parser that uses ReduceAndContinue.
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

        let r = parse(&mut p, 99);
        assert!(r.consumed);
        assert_eq!(r.actions, vec!["split:3".to_string(), "done".to_string()]);
        // on_reduce called twice (once for ReduceAndContinue, once for Reduce)
        assert_eq!(p.reduce_count, 2);
    }

    #[test]
    fn pass_through_after_reduce_and_continue_is_consumed() {
        /// Parser that does ReduceAndContinue then the remaining token passes through.
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

            fn on_reduce(&mut self, _: ()) {}
        }

        let mut p = RacParser;
        let r = parse(&mut p, 1);
        // Has accumulated actions from the ReduceAndContinue, so consumed is true
        assert!(r.consumed);
        assert_eq!(r.actions, vec!["first"]);
    }
}
