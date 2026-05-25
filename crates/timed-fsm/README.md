# timed-fsm

[![Crates.io](https://img.shields.io/crates/v/timed-fsm.svg)](https://crates.io/crates/timed-fsm)
[![docs.rs](https://docs.rs/timed-fsm/badge.svg)](https://docs.rs/timed-fsm)
[![License](https://img.shields.io/crates/l/timed-fsm.svg)](LICENSE-MIT)

A timed finite state machine framework where **timer commands are declarative transition outputs**.

Zero dependencies. No async runtime. No platform coupling.

## The problem

A regular FSM transitions on `(State, Event) → (State, Action)`. It has no way to express
*"if no event arrives within 100 ms, do X"* — the **absence** of an event is not an input.

You need a timer. But who manages it?

| Approach | Drawback |
|----------|----------|
| FSM calls `set_timer()` directly | Side effects inside the FSM; hard to unit-test |
| Caller manages timers based on FSM output | Timer logic leaks out; grammar split across two places |
| **FSM returns timer commands in `Response`** | **Declarative; pure; testable without mocks** |

`timed-fsm` takes the third approach. The state machine returns a `Response` containing
actions, timer commands, and a `consumed` flag. The runtime reads the `Response` and executes
the side effects. The state machine itself is **pure**.

## Quick start

```rust
use std::time::Duration;
use timed_fsm::{TimedStateMachine, Response};

/// Debounce: ignore rapid signal changes; confirm a level after 20 ms of silence.
struct Debounce {
    pending: Option<bool>,
}

impl TimedStateMachine for Debounce {
    type Event = bool;   // input: raw signal level
    type Action = bool;  // output: confirmed stable level
    type TimerId = ();   // only one timer needed

    fn on_event(&mut self, level: bool) -> Response<bool, ()> {
        // Buffer the level and (re)start the settle timer.
        self.pending = Some(level);
        Response::consume()
            .with_timer((), Duration::from_millis(20))
    }

    fn on_timeout(&mut self, _: ()) -> Response<bool, ()> {
        // No new event for 20 ms: emit the last pending level.
        self.pending.take()
            .map_or_else(Response::pass_through, Response::emit_one)
    }
}
```

## Testing without platform dependencies

The key advantage: test timer logic by calling `on_event` and `on_timeout` directly —
no OS timers, no mock clock, no `sleep`.

```rust
let mut d = Debounce { pending: None };

// Noisy signal: true → false → true in quick succession.
let r = d.on_event(true);
r.assert_consumed();     // event was absorbed
r.assert_timer_set(());  // settle timer was requested

let r = d.on_event(false);  // overwrite pending
let r = d.on_event(true);   // overwrite again

// Simulate the runtime calling on_timeout() when the timer fires.
let r = d.on_timeout(());
assert_eq!(r.actions, vec![true]);  // last pending level wins
```

No `SetTimer`. No `sleep`. No mock clock. Just call `on_event` / `on_timeout` and inspect the `Response`.

## Connecting to a runtime

Implement `TimerRuntime` and `ActionExecutor` for your platform, then call `dispatch` after
every transition:

```rust
use std::time::Duration;
use timed_fsm::{dispatch, TimerRuntime, ActionExecutor};

struct MyPlatform;

impl TimerRuntime for MyPlatform {
    type TimerId = ();
    fn set_timer(&mut self, _id: (), _duration: Duration) {
        // e.g. SetTimer() on Windows, timerfd_settime() on Linux
    }
    fn kill_timer(&mut self, _id: ()) {
        // e.g. KillTimer() on Windows
    }
}

impl ActionExecutor for MyPlatform {
    type Action = bool;
    fn execute(&mut self, _actions: &[bool]) {
        // e.g. SendInput() on Windows, uinput write on Linux
    }
}

// In your event loop:
let response = state_machine.on_event(event);
let consumed = dispatch(&response, &mut platform, &mut platform);
// If consumed is false, pass the event to the next handler in the chain.
```

## Multiple timers

Use an enum (or any `Copy + Eq + Debug` type) as `TimerId` when you need more than one
concurrent timer:

```rust
use std::time::Duration;
use timed_fsm::{TimedStateMachine, Response};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timer { Debounce, Repeat }

struct KeyFilter { key: Option<u8> }

impl TimedStateMachine for KeyFilter {
    type Event = u8;
    type Action = u8;
    type TimerId = Timer;

    fn on_event(&mut self, key: u8) -> Response<u8, Timer> {
        self.key = Some(key);
        Response::consume()
            .with_timer(Timer::Debounce, Duration::from_millis(10))
            .with_kill_timer(Timer::Repeat)
    }

    fn on_timeout(&mut self, id: Timer) -> Response<u8, Timer> {
        match id {
            Timer::Debounce => match self.key {
                Some(k) => Response::emit_one(k)
                    .with_timer(Timer::Repeat, Duration::from_millis(500)),
                None => Response::pass_through(),
            },
            Timer::Repeat => match self.key {
                Some(k) => Response::emit_one(k)
                    .with_timer(Timer::Repeat, Duration::from_millis(100)),
                None => Response::pass_through(),
            },
        }
    }
}
```

## Shift-reduce parser extension

When the decision about a token depends on tokens that arrive *after* it — for example,
detecting whether two keys were pressed simultaneously (chord) or in sequence — a plain
`TimedStateMachine` is not enough.

The `parser` module provides a `ShiftReduceParser` trait and a `parse` driver that buffer
tokens until a pattern is recognized or a timer forces a decision. See the
[API docs](https://docs.rs/timed-fsm) for details and a worked example.

## API overview

| Type | Role |
|------|------|
| `TimedStateMachine` | Core trait: `on_event` + `on_timeout` → `Response` |
| `Response<A, T>` | Transition result: actions + timer commands + consumed flag |
| `TimerCommand<T>` | `Set { id, duration }` or `Kill { id }` |
| `dispatch()` | Execute a `Response` against a runtime |
| `TimerRuntime` | Trait for platform timer operations |
| `ActionExecutor` | Trait for platform action execution |
| `ShiftReduceParser` | Extension: shift-reduce grammar with timer support |
| `parse()` | Main loop for a `ShiftReduceParser` |

### Response builder

```rust
// Consume the event, emit one action, set a timer, kill another
Response::emit_one(action)
    .with_timer(TIMER_A, Duration::from_millis(100))
    .with_kill_timer(TIMER_B)

// Consume the event, no output yet (pending state)
Response::consume()
    .with_timer(PENDING, Duration::from_millis(100))

// Don't consume — let the event propagate to the next handler
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

`timed-fsm` is useful whenever a state transition depends on **the absence of an event
within a time window**:

| Domain | Event | Timer role |
|--------|-------|------------|
| Keyboard firmware | Key press / release | Chord disambiguation timeout |
| Keyboard input (thumb shift) | Key press | Simultaneous key detection window |
| UI gestures | Mouse / touch | Double-click / long-press threshold |
| GPIO debounce | Signal edge | Bounce settling period |
| Network protocols | Packet received | Retransmission timeout |
| Protocol framing | Byte received | Inter-frame gap detection |
| Game input | Button press | Input combo window |
| IME / input method | Composition key | Commit-after-idle timeout |

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
