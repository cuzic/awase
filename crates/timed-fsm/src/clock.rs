//! Clock abstraction for deadline-based state machines.
//!
//! Provides a [`Clock`] trait that FSMs use to read the current time
//! as a monotonic millisecond counter, plus two implementations:
//! [`MonotonicClock`] for production and [`ManualClock`] for tests.
//!
//! # Why inject a clock?
//!
//! When an FSM stores deadlines as `now_ms + timeout`, testing requires
//! controlling what "now" means. A clock abstraction lets tests set
//! arbitrary timestamps without sleeping or mocking `std::time`.
//!
//! # Example
//!
//! ```
//! use timed_fsm::clock::{Clock, ManualClock};
//!
//! struct Cooldown { deadline_ms: u64 }
//!
//! impl Cooldown {
//!     fn arm(&mut self, clock: &impl Clock, timeout_ms: u64) {
//!         self.deadline_ms = clock.now_ms() + timeout_ms;
//!     }
//!     fn expired(&self, clock: &impl Clock) -> bool {
//!         clock.now_ms() >= self.deadline_ms
//!     }
//! }
//!
//! let mut cd = Cooldown { deadline_ms: 0 };
//! let clock = ManualClock(1000);
//! cd.arm(&clock, 500);
//!
//! assert!(!cd.expired(&ManualClock(1200)));
//! assert!( cd.expired(&ManualClock(1500)));
//! ```

use std::sync::OnceLock;
use std::time::Instant;

/// A monotonic millisecond clock.
///
/// Implement this trait to inject time into deadline-based FSMs,
/// enabling deterministic tests without real timers or OS dependencies.
pub trait Clock {
    /// Returns the current time as a monotonic millisecond counter.
    ///
    /// The epoch (zero point) is unspecified; only differences between
    /// calls are meaningful.
    #[must_use]
    fn now_ms(&self) -> u64;
}

static EPOCH: OnceLock<Instant> = OnceLock::new();

/// Production clock backed by [`std::time::Instant`].
///
/// The epoch is pinned on the first call to [`now_ms`][Clock::now_ms]
/// and stays stable for the process lifetime.
#[derive(Debug, Clone, Copy, Default)]
pub struct MonotonicClock;

impl Clock for MonotonicClock {
    #[allow(clippy::cast_possible_truncation)]
    fn now_ms(&self) -> u64 {
        // Overflows after ~585 million years of uptime.
        EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
    }
}

/// Hand-controlled clock for tests and simulations.
///
/// # Examples
///
/// ```
/// use timed_fsm::clock::{Clock, ManualClock};
///
/// assert_eq!(ManualClock(42).now_ms(), 42);
/// assert_eq!(ManualClock(0).now_ms(), 0);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ManualClock(pub u64);

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0
    }
}
