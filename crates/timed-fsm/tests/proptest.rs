#![allow(missing_docs, clippy::ignored_unit_patterns)]

use proptest::prelude::*;
use std::time::Duration;
use timed_fsm::{Response, TimedStateMachine, TimerCommand};

/// A simple counter FSM for testing properties.
struct Counter {
    count: i32,
    pending: bool,
}

impl TimedStateMachine for Counter {
    type Event = i32; // increment value
    type Action = i32; // emitted count
    type TimerId = ();

    fn on_event(&mut self, delta: i32) -> Response<i32, ()> {
        if self.pending {
            // Resolve pending, start new
            let output = self.count;
            self.count = self.count.wrapping_add(delta);
            self.pending = true;
            Response::emit_one(output)
                .with_kill_timer(())
                .with_timer((), Duration::from_millis(100))
        } else {
            self.count = self.count.wrapping_add(delta);
            self.pending = true;
            Response::consume().with_timer((), Duration::from_millis(100))
        }
    }

    fn on_timeout(&mut self, (): ()) -> Response<i32, ()> {
        if self.pending {
            self.pending = false;
            Response::emit_one(self.count)
        } else {
            Response::pass_through()
        }
    }
}

proptest! {
    /// Response builder never panics regardless of input.
    #[test]
    fn response_builder_never_panics(
        consumed in any::<bool>(),
        action_count in 0_i32..10,
        timer_count in 0_u8..5,
    ) {
        let actions: Vec<i32> = (0..action_count).collect();
        let mut r = if consumed {
            Response::emit(actions)
        } else {
            Response::pass_through()
        };
        for i in 0..timer_count {
            r = r.with_timer(i, Duration::from_millis(100));
        }
        // Should not panic
        let _ = format!("{r:?}");
    }

    /// FSM never panics on arbitrary event sequences.
    #[test]
    fn counter_never_panics(events in prop::collection::vec(any::<i32>(), 0..100)) {
        let mut fsm = Counter { count: 0, pending: false };
        for event in events {
            let _ = fsm.on_event(event);
        }
        let _ = fsm.on_timeout(());
    }

    /// consumed implies either actions or timer set.
    #[test]
    fn consumed_implies_side_effect(events in prop::collection::vec(any::<i32>(), 1..50)) {
        let mut fsm = Counter { count: 0, pending: false };
        for event in events {
            let r = fsm.on_event(event);
            if r.consumed {
                let has_action = !r.actions.is_empty();
                let has_timer = r.timers.iter().any(|t| matches!(t, TimerCommand::Set { .. }));
                prop_assert!(has_action || has_timer,
                    "consumed response must have actions or set a timer");
            }
        }
    }

    /// pass_through has no actions and no timers.
    #[test]
    fn pass_through_is_clean(events in prop::collection::vec(any::<i32>(), 0..50)) {
        let mut fsm = Counter { count: 0, pending: false };
        // Process events first so state is arbitrary
        for event in events {
            let _ = fsm.on_event(event);
        }
        // Drain pending state
        let _ = fsm.on_timeout(());
        // Further timeouts should pass through
        let r = fsm.on_timeout(());
        if !r.consumed {
            prop_assert!(r.actions.is_empty());
            prop_assert!(r.timers.is_empty());
        }
    }
}
