#![cfg(windows)]

use awase::engine::{InputModeState, ModifierState};

#[test]
fn build_input_context_preserves_thumb_down_timestamps() {
    let modifiers = ModifierState {
        shift: true,
        ..ModifierState::default()
    };
    let ctx = awase_windows::runtime::build_input_context(
        true,
        InputModeState::ObservedRomaji,
        true,
        false,
        &modifiers,
        Some(123),
        Some(456),
    );

    assert_eq!(ctx.left_thumb_down, Some(123));
    assert_eq!(ctx.right_thumb_down, Some(456));
    assert!(ctx.modifiers.shift);
}
