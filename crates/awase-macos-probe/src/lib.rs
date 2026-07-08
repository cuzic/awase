//! Library facade of `awase-macos-probe`.
//!
//! The probe is primarily a diagnostic binary (`src/main.rs`); this thin
//! library surface exists only so integration tests under `tests/` — which
//! compile as separate crates — can reach the pure, platform-independent logic
//! they need to exercise. Re-export here only what is safe and useful to drive
//! from an external test crate; the binary keeps its own `mod` declarations.
//!
//! `synthetic`'s doc comments link to sibling modules (`crate::output`, …) that
//! live only in the binary, not in this facade, so intra-doc link checking is
//! relaxed for this crate — the authoritative, resolvable docs are the binary's.
#![allow(rustdoc::broken_intra_doc_links)]

pub mod runtime;
pub mod synthetic;
