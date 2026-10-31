//! 閉ループのテスト（`closed_loop_scenarios.rs`・`pseudo_ime_exhaustive.rs`）の支援モジュール
//! （擬似 IME・ハーネス・不変条件・防御の目録）。
#![allow(unreachable_pub, dead_code)]
pub mod cold_drop;
pub mod defenses;
pub mod harness;
pub mod invariants;
pub mod journal_scenario;
pub mod pseudo_ime;
pub mod pseudo_mozc;
pub mod pseudo_zoom;
pub mod sprime;
