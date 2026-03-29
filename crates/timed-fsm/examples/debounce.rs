//! Debounce filter example.
//!
//! Demonstrates a minimal timed state machine that filters noisy
//! digital input by waiting for a stable period before confirming
//! a level change.

use std::time::Duration;
use timed_fsm::{Response, TimedStateMachine};

struct Debounce {
    pending: Option<bool>,
    confirmed: Option<bool>,
}

impl Debounce {
    fn new() -> Self {
        Self {
            pending: None,
            confirmed: None,
        }
    }
}

impl TimedStateMachine for Debounce {
    type Event = bool;
    type Action = bool;
    type TimerId = ();

    fn on_event(&mut self, level: bool) -> Response<bool, ()> {
        // If same as confirmed level, ignore (no bounce)
        if self.confirmed == Some(level) {
            self.pending = None;
            return Response::pass_through().with_kill_timer(());
        }
        // New level detected, start debounce timer
        self.pending = Some(level);
        Response::consume().with_timer((), Duration::from_millis(20))
    }

    fn on_timeout(&mut self, _: ()) -> Response<bool, ()> {
        match self.pending.take() {
            Some(level) => {
                self.confirmed = Some(level);
                Response::emit_one(level)
            }
            None => Response::pass_through(),
        }
    }
}

fn main() {
    let mut filter = Debounce::new();

    println!("=== Noisy signal: true, false, true (bounce) ===");
    let r = filter.on_event(true);
    println!(
        "event(true)  -> consumed={}, timers={}",
        r.consumed,
        r.timers.len()
    );

    let r = filter.on_event(false);
    println!(
        "event(false) -> consumed={}, timers={}",
        r.consumed,
        r.timers.len()
    );

    let r = filter.on_event(true);
    println!(
        "event(true)  -> consumed={}, timers={}",
        r.consumed,
        r.timers.len()
    );

    println!("\n=== Timeout fires ===");
    let r = filter.on_timeout(());
    println!("timeout()    -> actions={:?} (confirmed level)", r.actions);

    println!("\n=== Same level again (no bounce) ===");
    let r = filter.on_event(true);
    println!(
        "event(true)  -> consumed={} (ignored, same as confirmed)",
        r.consumed
    );
}
