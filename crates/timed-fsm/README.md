# timed-fsm

[![Crates.io](https://img.shields.io/crates/v/timed-fsm.svg)](https://crates.io/crates/timed-fsm)
[![docs.rs](https://docs.rs/timed-fsm/badge.svg)](https://docs.rs/timed-fsm)
[![License](https://img.shields.io/crates/l/timed-fsm.svg)](LICENSE-MIT)

A timed finite state machine framework where **timer commands are declarative transition outputs**.

Zero dependencies by default. No platform coupling. Async `tokio` integration is opt-in
(see the `tokio` feature below), never on by default.

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

Implement `TimerRuntime` and `ActionExecutor` for your platform, then call `Response::dispatch`
after every transition:

```rust
use std::time::Duration;
use timed_fsm::{Response, TimerRuntime, ActionExecutor};

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
let response = Response::<bool, ()>::emit_one(true);
let consumed = response.dispatch(&mut MyPlatform, &mut MyPlatform);
// If consumed is false, pass the event to the next handler in the chain.
```

## Async actions, and stopping the driver on a dead resource

`ActionExecutor::execute` is synchronous and gets the whole action batch at once — fine
for actions that never need to `.await` (`SendInput()`, `uinput` writes, …), but not for
actions that acquire a socket, probe a peer, or otherwise need to await something.
`Response::dispatch_async` + `AsyncActionExecutor` cover that: actions run one at a time,
each `.await`ed in turn. No dependency on any particular async runtime — just
`core::future::Future`.

Running an action can also surface something the state machine has no way to know: that
the resource it depends on is already gone (a closed socket, a dropped connection, …).
That's not a state transition — the FSM never touched the resource — but the driver loop
still needs to react, usually by shutting down instead of waiting on events that can never
arrive again. Report `ActionOutcome::Stop` from `execute_one` for exactly this:
`dispatch_async` skips whatever actions are left in the current response and returns a
`DispatchOutcome { consumed, stop: true }` for the driver's own loop to check.

```rust
use std::time::Duration;
use timed_fsm::{ActionOutcome, AsyncActionExecutor, Response, TimerRuntime};

struct MyPlatform;

impl TimerRuntime for MyPlatform {
    type TimerId = ();
    fn set_timer(&mut self, _id: (), _duration: Duration) {}
    fn kill_timer(&mut self, _id: ()) {}
}

impl AsyncActionExecutor for MyPlatform {
    type Action = &'static str;
    async fn execute_one(&mut self, action: &&'static str) -> ActionOutcome {
        if *action == "socket-closed" {
            return ActionOutcome::Stop;
        }
        // ... await the real I/O here ...
        ActionOutcome::Continue
    }
}

// In your event loop:
let response = Response::emit_one("socket-closed");
let outcome = response.dispatch_async(&mut MyPlatform, &mut MyPlatform).await;
if outcome.stop {
    // break out of the driver loop — no more events/timeouts will come.
}
```

## `tokio` feature

`TimerRuntime::set_timer` is synchronous — it says nothing about how the caller finds
out a timer fired. On `tokio`, that's always the same shape: spawn a sleep task, keep the
handle so a `Kill` can abort it, and report back over a channel the event loop can
`select!` on. Enable the `tokio` feature (off by default) to get that written once, as
`TokioTimerRuntime<T>`:

```toml
[dependencies]
timed-fsm = { version = "0.4", features = ["tokio"] }
```

```rust,ignore
use timed_fsm::tokio_support::TokioTimerRuntime;

let mut timers = TokioTimerRuntime::new();
loop {
    let response = tokio::select! {
        Some(event) = input_rx.recv() => machine.on_event(event),
        Some(timer_id) = timers.recv() => machine.on_timeout(timer_id),
    };
    // apply response.timers via timers.set_timer/kill_timer, run response.actions, ...
}
```

This is additive: the crate's core (`TimedStateMachine`, `Response`, `TimerRuntime`, ...)
stays dependency-free either way.

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

The `parser` module provides a `ShiftReduceParser` trait and a `ShiftReduceParser::parse`
driver that buffer tokens until a pattern is recognized or a timer forces a decision. See the
[API docs](https://docs.rs/timed-fsm) for details and a worked example.

## Coroutine extension

For sequential multi-phase workflows where a plain FSM would need an explicit state enum,
the `coro` module provides `StepCoro<I, Y>`: a minimal coroutine backed by `async`/`await`
(no external runtime, `!Send`, MSRV 1.85).

```rust
use std::rc::Rc;
use timed_fsm::coro::{Channel, CoroStep, StepCoro, yield_step};

async fn phases(ch: Rc<Channel<u32, String>>) {
    let n = yield_step(ch.clone(), "phase1".to_owned()).await;
    let _ = yield_step(ch, format!("phase2: got {n}")).await;
}

let mut coro: StepCoro<u32, String> = StepCoro::new(phases);
let CoroStep::Yielded(out) = coro.step(0) else { panic!() };
assert_eq!(out, "phase1");
let CoroStep::Yielded(out) = coro.step(42) else { panic!() };
assert_eq!(out, "phase2: got 42");
```

`step(0)`'s `0` above is discarded (see the module docs' "最初の step について" /
"About the first step") — the first yield point is only ever fed by the *second*
`step()` call. If you need to consume that discarded first step before storing the
coroutine somewhere it will start receiving real ticks (so a real tick right after
install can't be silently swallowed), use `StepCoro::prime()` instead of inventing a
dummy input value:

```rust
# use std::rc::Rc;
# use timed_fsm::coro::{Channel, CoroStep, StepCoro, yield_step};
# async fn phases(ch: Rc<Channel<u32, String>>) {
#     let n = yield_step(ch.clone(), "phase1".to_owned()).await;
#     let _ = yield_step(ch, format!("phase2: got {n}")).await;
# }
let mut coro: StepCoro<u32, String> = StepCoro::new(phases);
let CoroStep::Yielded(out) = coro.prime() else { panic!() }; // no dummy input needed
assert_eq!(out, "phase1");
```

## Clock abstraction

The `clock` module provides a `Clock` trait for injecting time into deadline-based FSMs.
`MonotonicClock` wraps `std::time::Instant` for production; `ManualClock` lets tests
set arbitrary timestamps without sleeping.

```rust
use timed_fsm::clock::{Clock, ManualClock};

assert_eq!(ManualClock(42).now_ms(), 42);
```

## Holding gate

The `gate` module provides `HoldingGate<M, T>`: a wrapper that buffers items while a
`TimedStateMachine<Action = GateAction>` is in hold mode and drains them on demand.

```rust
use timed_fsm::{GateAction, HoldingGate, Response, TimedStateMachine};

struct AlwaysHold;
impl TimedStateMachine for AlwaysHold {
    type Event  = ();
    type Action = GateAction;
    type TimerId = ();
    fn on_event(&mut self, _: ()) -> Response<GateAction, ()> {
        Response::emit_one(GateAction::InitiateHold)
    }
    fn on_timeout(&mut self, _: ()) -> Response<GateAction, ()> {
        Response::pass_through()
    }
}

let mut gate: HoldingGate<AlwaysHold, u32> = HoldingGate::new(AlwaysHold, 8);
gate.on_event(());   // InitiateHold
gate.try_hold(42);
```

## API overview

| Type / function | Role |
|-----------------|------|
| `TimedStateMachine` | Core trait: `on_event` + `on_timeout` → `Response` |
| `Response<A, T>` | Transition result: actions + timer commands + consumed flag |
| `TimerCommand<T>` | `Set { id, duration }` or `Kill { id }` |
| `Response::dispatch` | Execute a `Response` against a runtime |
| `TimerRuntime` | Trait for platform timer operations |
| `ActionExecutor` | Trait for platform action execution |
| `Response::dispatch_async` | Async counterpart to `dispatch`, for actions that must `.await` |
| `AsyncActionExecutor` | Trait for async, one-action-at-a-time execution |
| `ActionOutcome` / `DispatchOutcome` | Lets an action signal "stop the driver loop" (resource gone) |
| `ShiftReduceParser` | Extension: shift-reduce grammar with timer support |
| `ShiftReduceParser::parse` | Main loop for a `ShiftReduceParser` |
| `StepCoro<I, Y>` | Coroutine for sequential multi-phase workflows |
| `StepCoro::prime` | Consume the discarded first `step()` without a dummy input value |
| `Clock` / `MonotonicClock` / `ManualClock` | Clock abstraction for deadline-based FSMs |
| `HoldingGate<M, T>` / `GateAction` | Item-buffering gate controlled by an FSM |
| `tokio_support::TokioTimerRuntime<T>` | Async `TimerRuntime` for `tokio` (`tokio` feature, off by default) |

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

- **Zero dependencies by default** — only `std::time::Duration` from the standard library,
  unless the opt-in `tokio` feature is enabled
- **No side effects** — the state machine never calls platform APIs
- **`consumed` flag** — supports event interception (keyboard hooks, MIDI filters, etc.)
- **Multiple timer IDs** — use `()` for one timer, an enum for many
- **Infallible transitions** — `on_event` / `on_timeout` always return a `Response`

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option.
