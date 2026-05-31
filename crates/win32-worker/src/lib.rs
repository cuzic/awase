//! RAII worker thread with Win32 Event-based graceful shutdown.
//!
//! [`WorkerThread`] wraps a background thread and a Win32 manual-reset event
//! object. When the handle is dropped (or [`WorkerThread::shutdown`] is called
//! explicitly), the event fires and the thread is joined — no leaked threads.
//!
//! The worker receives a [`ShutdownToken`] and calls [`ShutdownToken::sleep_ms`]
//! instead of [`std::thread::sleep`]. The sleep returns [`ControlFlow::Break`]
//! immediately when shutdown is signalled, so the thread can exit cleanly without
//! waiting for the full sleep interval to expire.
//!
//! # Quick start
//!
//! ```no_run
//! use win32_worker::WorkerThread;
//!
//! let worker = WorkerThread::spawn("my-worker", |token| {
//!     loop {
//!         // ... do work ...
//!
//!         // Sleep for 100 ms, but wake immediately on shutdown.
//!         if token.sleep_ms(100).is_break() {
//!             break;
//!         }
//!     }
//! });
//!
//! // Dropping `worker` signals shutdown and joins the thread.
//! ```
//!
//! # Why Win32 Event instead of `AtomicBool`?
//!
//! `WaitForSingleObject` with a manual-reset event gives **precise
//! millisecond-level sleep interruption** without polling. With `AtomicBool` +
//! `thread::sleep`, the earliest the thread can notice shutdown is after the
//! current sleep interval expires. With [`ShutdownToken::sleep_ms`], the thread
//! wakes within the OS timer resolution (typically ≤ 15 ms).

#![cfg(windows)]
#![allow(unsafe_code)]

use std::ops::ControlFlow;
use std::sync::Arc;
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};

// ── Internal: Arc wrapper for a Win32 Event handle ──

struct EventHandle(HANDLE);

// HANDLE is a process-global resource and safe to send across threads.
unsafe impl Send for EventHandle {}
unsafe impl Sync for EventHandle {}

impl Drop for EventHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

// ── Public API ──

/// Shutdown notification token passed to the worker closure.
///
/// Use [`sleep_ms`](Self::sleep_ms) instead of [`std::thread::sleep`] so the
/// worker can be interrupted immediately when shutdown is requested.
///
/// `ShutdownToken` is [`Clone`], so it can be shared across helper functions
/// or subtasks within the same worker thread.
#[derive(Clone)]
pub struct ShutdownToken(Arc<EventHandle>);

impl ShutdownToken {
    /// Sleep for `ms` milliseconds, or until shutdown is signalled.
    ///
    /// Returns [`ControlFlow::Break(())`] if shutdown was signalled before
    /// the timeout elapsed, or [`ControlFlow::Continue(())`] on normal timeout.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use win32_worker::WorkerThread;
    /// WorkerThread::spawn("example", |token| {
    ///     loop {
    ///         // work ...
    ///         if token.sleep_ms(50).is_break() { break; }
    ///     }
    /// });
    /// ```
    pub fn sleep_ms(&self, ms: u32) -> ControlFlow<()> {
        let result = unsafe { WaitForSingleObject((self.0).0, ms) };
        if result == WAIT_OBJECT_0 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    /// Returns `true` if shutdown has already been signalled (non-blocking).
    ///
    /// Useful for checking shutdown at the top of a loop that does not sleep:
    ///
    /// ```no_run
    /// # use win32_worker::WorkerThread;
    /// WorkerThread::spawn("checker", |token| {
    ///     while !token.is_shutdown() {
    ///         // fast poll work ...
    ///     }
    /// });
    /// ```
    #[must_use]
    pub fn is_shutdown(&self) -> bool {
        unsafe { WaitForSingleObject((self.0).0, 0) == WAIT_OBJECT_0 }
    }
}

impl std::fmt::Debug for ShutdownToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShutdownToken").finish_non_exhaustive()
    }
}

/// RAII handle for a named worker thread.
///
/// Dropping this handle (or calling [`shutdown`](Self::shutdown)) signals the
/// worker to stop and blocks until it has exited.
///
/// # Example
///
/// ```no_run
/// use win32_worker::WorkerThread;
///
/// {
///     let _worker = WorkerThread::spawn("poller", |token| {
///         while token.sleep_ms(10).is_continue() {
///             // poll every 10 ms
///         }
///     });
///     // ... do other work ...
/// } // _worker drops here → shutdown signalled → thread joined
/// ```
pub struct WorkerThread {
    event: Arc<EventHandle>,
    handle: Option<JoinHandle<()>>,
}

impl WorkerThread {
    /// Spawn a named worker thread.
    ///
    /// `f` receives a [`ShutdownToken`] and should check it regularly via
    /// [`ShutdownToken::sleep_ms`] or [`ShutdownToken::is_shutdown`].
    ///
    /// # Panics
    ///
    /// Panics if `CreateEventW` or `thread::Builder::spawn` fails.
    pub fn spawn(name: &str, f: impl FnOnce(ShutdownToken) + Send + 'static) -> Self {
        // Manual-reset event, initially non-signalled.
        let raw = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        assert!(!raw.is_null(), "CreateEventW failed");

        let event = Arc::new(EventHandle(raw));
        let token = ShutdownToken(Arc::clone(&event));

        let handle = std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || f(token))
            .unwrap_or_else(|e| panic!("failed to spawn worker thread '{name}': {e}"));

        Self {
            event,
            handle: Some(handle),
        }
    }

    /// Signal shutdown and join the thread.
    ///
    /// Equivalent to dropping the handle, but makes the intent explicit.
    pub fn shutdown(mut self) {
        self.do_shutdown();
    }

    fn do_shutdown(&mut self) {
        unsafe { SetEvent((self.event).0) };
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for WorkerThread {
    fn drop(&mut self) {
        self.do_shutdown();
    }
}

impl std::fmt::Debug for WorkerThread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerThread")
            .field(
                "running",
                &self.handle.as_ref().is_some_and(|h| !h.is_finished()),
            )
            .finish_non_exhaustive()
    }
}
