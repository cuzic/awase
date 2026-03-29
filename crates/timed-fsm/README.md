# timed-fsm

[![Crates.io](https://img.shields.io/crates/v/timed-fsm.svg)](https://crates.io/crates/timed-fsm)
[![docs.rs](https://docs.rs/timed-fsm/badge.svg)](https://docs.rs/timed-fsm)
[![License](https://img.shields.io/crates/l/timed-fsm.svg)](LICENSE-MIT)

A timed finite state machine framework where **timer commands are declarative transition outputs**.

Zero dependencies. No async runtime. No platform coupling.

## The problem

A regular FSM transitions on `(State, Event) → (State, Action)`. It has no way to express *"if no event arrives within 100ms, do X"* — the **absence** of an event is not an input.

You need a timer. But who manages it?

| Approach | Problem |
|----------|---------|
| FSM calls `set_timer()` directly | Side effects inside the FSM; untestable |
| Caller manages timers based on FSM output | Timer logic leaks outside the FSM; fragile |
| **FSM returns timer commands in `Response`** | **Declarative; testable; no side effects** |

`timed-fsm` takes the third approach. The state machine returns a [`Response`] containing actions, timer commands, and a `consumed` flag. The runtime reads the `Response` and executes the side effects. The state machine itself is **pure**.

## Quick start

```rust
use std::time::Duration;
use timed_fsm::{TimedStateMachine, Response};

struct Debounce {
    pending: Option<bool>,
}

impl TimedStateMachine for Debounce {
    type Event = bool;       // GPIO level
    type Action = bool;      // Confirmed level
    type TimerId = ();

    fn on_event(&mut self, level: bool) -> Response<bool, ()> {
        self.pending = Some(level);
        Response::consume()
            .with_timer((), Duration::from_millis(20))
    }

    fn on_timeout(&mut self, _: ()) -> Response<bool, ()> {
        self.pending.take()
            .map_or_else(Response::pass_through, Response::emit_one)
    }
}
```

## Testing without platform dependencies

The key advantage: you can test timer logic without mocking OS APIs.

```rust
let mut d = Debounce { pending: None };

// Noisy signal
let r = d.on_event(true);
r.assert_consumed();        // event was handled
r.assert_timer_set(());     // timer was requested

let r = d.on_event(false);  // overwrites pending
let r = d.on_event(true);   // overwrites again

// Timeout fires — last value wins
let r = d.on_timeout(());
assert_eq!(r.actions, vec![true]);
```

No `SetTimer`. No `sleep`. No mock clock. Just call `on_event` / `on_timeout` and inspect the `Response`.

## Connecting to a runtime

The [`dispatch`] function bridges the pure state machine to platform-specific side effects:

```rust
use timed_fsm::{dispatch, TimerRuntime, ActionExecutor};

// Implement these two traits for your platform
impl TimerRuntime for MyPlatform {
    type TimerId = ();
    fn set_timer(&mut self, id: (), duration: Duration) { /* OS timer */ }
    fn kill_timer(&mut self, id: ()) { /* cancel OS timer */ }
}

impl ActionExecutor for MyPlatform {
    type Action = bool;
    fn execute(&mut self, actions: &[bool]) { /* handle output */ }
}

// In your event loop:
let response = state_machine.on_event(event);
let consumed = dispatch(&response, &mut platform, &mut platform);
```

## API overview

| Type | Role |
|------|------|
| [`TimedStateMachine`] | Core trait: `on_event` + `on_timeout` → `Response` |
| [`Response<A, T>`] | Transition result: actions + timer commands + consumed flag |
| [`TimerCommand<T>`] | `Set { id, duration }` or `Kill { id }` |
| [`dispatch()`] | Execute a `Response` against a runtime |
| [`TimerRuntime`] | Trait for platform timer operations |
| [`ActionExecutor`] | Trait for platform action execution |

### Response builder

```rust
// Consume the event, emit one action, set a timer, kill another
Response::emit_one(action)
    .with_timer(TIMER_A, Duration::from_millis(100))
    .with_kill_timer(TIMER_B)

// Consume the event, no output yet (pending state)
Response::consume()
    .with_timer(PENDING, Duration::from_millis(100))

// Don't consume — let the event propagate
Response::pass_through()
```

### Test assertion helpers

```rust
response.assert_consumed();
response.assert_pass_through();
response.assert_timer_set(timer_id);
response.assert_timer_kill(timer_id);
response.assert_action_count(n);
```

All assertion methods use `#[track_caller]` for clear error locations.

## Use cases

`timed-fsm` is useful whenever a state transition depends on **the absence of an event within a time window**:

| Domain | Event | Action | Timer role |
|--------|-------|--------|------------|
| Keyboard input (thumb shift) | Key press | Character output | Simultaneous key detection window |
| UI interaction | Mouse click | Click type (single/double) | Double-click threshold |
| Hardware debounce | GPIO change | Confirmed level | Bounce settling period |
| Network protocol | Packet received | ACK / retransmit | Retransmission timeout |
| Game input | Button press | Combo / special move | Input combo window |
| Gesture recognition | Touch event | Gesture type | Swipe completion timeout |

## Design principles

- **Zero dependencies** — only `std::time::Duration` from the standard library
- **No side effects** — the state machine never calls platform APIs
- **`consumed` flag** — supports event interception (keyboard hooks, MIDI filters, etc.)
- **Multiple timer IDs** — use `()` for one timer, an enum for many
- **Infallible transitions** — `on_event` / `on_timeout` always return a `Response`

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option.
