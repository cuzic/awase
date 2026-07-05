// UI test: verify RESTRICTED_IME_EVENT_CONSTRUCTION fires on ImeEvent::PanicReset /
// HwndCacheRestored construction outside their designated function names.

/// Minimal inline mock matching the shape of `awase_windows::state::ime_event::ImeEvent`.
mod ime_event {
    #[derive(Debug, Clone)]
    pub enum ImeEvent {
        UserImeSetIntent { target: bool },
        PanicReset { target: bool },
        HwndCacheRestored { target: bool },
    }
}

use ime_event::ImeEvent;

fn apply_panic_reset() -> ImeEvent {
    // Should NOT trigger: this is the designated function.
    ImeEvent::PanicReset { target: true }
}

fn apply_hwnd_cache_restore() -> ImeEvent {
    // Should NOT trigger: this is the designated function.
    ImeEvent::HwndCacheRestored { target: false }
}

fn reset_stale_thing_for_some_new_reason() -> ImeEvent {
    // Should trigger: PanicReset constructed outside apply_panic_reset.
    ImeEvent::PanicReset { target: false } //~ WARN constructing `ImeEvent::PanicReset`
}

fn some_other_helper() -> ImeEvent {
    // Should NOT trigger: unrelated event variant.
    ImeEvent::UserImeSetIntent { target: true }
}

fn main() {
    let _ = apply_panic_reset();
    let _ = apply_hwnd_cache_restore();
    let _ = reset_stale_thing_for_some_new_reason();
    let _ = some_other_helper();
}
