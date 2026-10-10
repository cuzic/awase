//! `PlatformState` の実機の構築口（殻）。実時計・OS のフォアグラウンド取得・`quanta` の実時計を
//! 注入して核の `ImeStateHub::with_clock` を呼ぶだけ（FCIS P4a。ADR-229 段階 B で、`PlatformState::new`／
//! `Default` の inherent impl を殻から外した——crate を分けると核の型に殻の inherent impl は書けない）。

use std::time::Instant;

use crate::state::hub_clock::HubClock;
use crate::state::platform_state::{
    FocusStore, GateStore, ImeStateHub, KeymapStore, PlatformState,
};

/// 実機の `PlatformState`（実時計 `hook::current_tick_ms`・`Instant::now`・`quanta::Clock::new()`・
/// `win32::foreground_scope` を注入する）。
#[must_use]
pub(crate) fn new_platform_state() -> PlatformState {
    PlatformState {
        ime: ImeStateHub::with_clock(
            HubClock::wall(crate::hook::current_tick_ms, Instant::now),
            quanta::Clock::new(),
            crate::win32::foreground_scope,
        ),
        focus: FocusStore::new(),
        gate: GateStore::new(),
        keymap: KeymapStore::default(),
    }
}
