//! Double-click detection example.
//!
//! Demonstrates a timed state machine that distinguishes single clicks
//! from double clicks based on timing.

use std::time::Duration;
use timed_fsm::{Response, TimedStateMachine};

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClickOutput {
    Single,
    Double,
}

enum ClickState {
    Idle,
    WaitingForSecond,
}

struct DoubleClickDetector {
    state: ClickState,
    threshold: Duration,
}

impl DoubleClickDetector {
    fn new(threshold: Duration) -> Self {
        Self {
            state: ClickState::Idle,
            threshold,
        }
    }
}

impl TimedStateMachine for DoubleClickDetector {
    type Event = (); // click event (no payload)
    type Action = ClickOutput;
    type TimerId = ();

    fn on_event(&mut self, _: ()) -> Response<ClickOutput, ()> {
        match self.state {
            ClickState::Idle => {
                // First click: wait for a possible second click
                self.state = ClickState::WaitingForSecond;
                Response::consume().with_timer((), self.threshold)
            }
            ClickState::WaitingForSecond => {
                // Second click within threshold: double click!
                self.state = ClickState::Idle;
                Response::emit_one(ClickOutput::Double).with_kill_timer(())
            }
        }
    }

    fn on_timeout(&mut self, _: ()) -> Response<ClickOutput, ()> {
        match self.state {
            ClickState::WaitingForSecond => {
                // No second click arrived: single click
                self.state = ClickState::Idle;
                Response::emit_one(ClickOutput::Single)
            }
            ClickState::Idle => Response::pass_through(),
        }
    }
}

fn main() {
    let mut detector = DoubleClickDetector::new(Duration::from_millis(300));

    println!("=== Double click ===");
    let r = detector.on_event(());
    println!("click 1: consumed={}, actions={:?}", r.consumed, r.actions);

    let r = detector.on_event(());
    println!("click 2: consumed={}, actions={:?}", r.consumed, r.actions);

    println!("\n=== Single click (timeout) ===");
    let r = detector.on_event(());
    println!("click 1: consumed={}, waiting...", r.consumed);

    let r = detector.on_timeout(());
    println!("timeout: actions={:?}", r.actions);
}
