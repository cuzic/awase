//! Multi-instance independence check for [`SyntheticEventOrigin`]'s per-process
//! random cookie (project tracker task #18).
//!
//! `SyntheticEventOrigin::new()` seeds its cookie from `std::process::id()` +
//! `SystemTime` via `RandomState`, so two probe instances running at once — or
//! a future `awase-macos-probe` + `awase-macos-body` pair — cannot misidentify
//! each other's synthetic events as their own. The per-call distinctness of two
//! sequentially-constructed origins is unit-tested inside `synthetic.rs`; this
//! integration test adds the properties that need an external test crate:
//!
//! 1. **Concurrent instantiation** stays collision-free — many origins built
//!    simultaneously must still get pairwise-distinct cookies (the `RandomState`
//!    seed must not degenerate to a constant under contention).
//! 2. **Cross-origin non-identification** — one origin must never accept
//!    another's cookie as `is_self_event`, which is the actual guarantee that
//!    keeps two simultaneous instances from confusing each other's events.
//!
//! True separate-OS-*process* verification (two real `awase-macos-probe`
//! invocations comparing cookies) is a stretch goal for real hardware
//! (task #19): it needs the CLI to expose a way to print/compare cookies, which
//! `main.rs` does not wire up yet. Threads exercise the concurrency property
//! that is meaningful to check pre-hardware, and this stays on the pure-`std`
//! surface so it runs under `cargo test -p awase-macos-probe` on any host (no
//! `objc2`, no `--target aarch64-apple-darwin`).

use std::collections::HashSet;
use std::sync::mpsc;
use std::thread;

use awase_macos_probe::synthetic::SyntheticEventOrigin;

#[test]
fn concurrent_origins_have_pairwise_distinct_cookies() {
    const N: usize = 64;

    let (tx, rx) = mpsc::channel();
    let handles: Vec<_> = (0..N)
        .map(|_| {
            let tx = tx.clone();
            thread::spawn(move || {
                let origin = SyntheticEventOrigin::new();
                tx.send(origin.cookie()).expect("receiver kept alive");
            })
        })
        .collect();
    drop(tx);

    for handle in handles {
        handle.join().expect("origin-constructing thread panicked");
    }

    let cookies: Vec<i64> = rx.iter().collect();
    assert_eq!(cookies.len(), N, "expected one cookie per thread");

    let unique: HashSet<i64> = cookies.iter().copied().collect();
    assert_eq!(
        unique.len(),
        N,
        "concurrently constructed cookies collided: {cookies:?}"
    );
}

#[test]
fn distinct_origins_do_not_misidentify_each_others_events() {
    let a = SyntheticEventOrigin::new();
    let b = SyntheticEventOrigin::new();

    assert!(
        a.is_self_event(a.cookie()),
        "origin must accept its own cookie"
    );
    assert!(
        b.is_self_event(b.cookie()),
        "origin must accept its own cookie"
    );
    assert!(
        !a.is_self_event(b.cookie()),
        "origin A must not claim origin B's event"
    );
    assert!(
        !b.is_self_event(a.cookie()),
        "origin B must not claim origin A's event"
    );
}
