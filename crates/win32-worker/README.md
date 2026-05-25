# win32-worker

[![Crates.io](https://img.shields.io/crates/v/win32-worker.svg)](https://crates.io/crates/win32-worker)
[![docs.rs](https://docs.rs/win32-worker/badge.svg)](https://docs.rs/win32-worker)
[![License](https://img.shields.io/crates/l/win32-worker.svg)](LICENSE-MIT)

RAII worker thread with Win32 Event-based graceful shutdown.

## Overview

`win32-worker` provides a `WorkerThread` handle that:

- **Spawns** a named background thread
- **Signals** shutdown via a Win32 manual-reset event object
- **Joins** the thread on drop — no leaked threads

The worker receives a `ShutdownToken` and can call `token.sleep_ms(n)` to sleep
while remaining responsive to shutdown. When the handle is dropped (or
`WorkerThread::shutdown()` is called), the event is set and the thread is joined.

## Quick start

```rust
use win32_worker::WorkerThread;

let worker = WorkerThread::spawn("my-worker", |token| {
    loop {
        do_some_work();

        // Sleep for 100 ms, but wake immediately on shutdown.
        if token.sleep_ms(100).is_break() {
            break;
        }
    }
});

// worker drops here → shutdown event fires → thread is joined
```

## API

### `WorkerThread`

| Method | Description |
|--------|-------------|
| `WorkerThread::spawn(name, f)` | Spawn a named worker thread |
| `WorkerThread::shutdown(self)` | Signal shutdown and join (explicit alternative to drop) |

Drop automatically signals shutdown and joins the thread.

### `ShutdownToken`

| Method | Returns | Description |
|--------|---------|-------------|
| `sleep_ms(ms)` | `ControlFlow<()>` | Sleep for `ms` ms; `Break` if shutdown was signalled |
| `is_shutdown()` | `bool` | Non-blocking check for shutdown |

`ShutdownToken` is `Clone`, so it can be shared across multiple loops or subtasks
within the same worker thread.

## Why Win32 Event instead of `AtomicBool`?

`WaitForSingleObject` with a manual-reset event gives **precise millisecond-level
sleep interruption** without polling. With `AtomicBool` + `thread::sleep`, the
earliest you can detect shutdown is after the current sleep interval expires.
With `ShutdownToken::sleep_ms`, shutdown is detected within the OS timer resolution
(typically 15 ms or less).

## Platform

Windows only (`#[cfg(windows)]`). The crate compiles on other platforms but
exports no items.

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option.
