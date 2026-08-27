/// 左Shift単独タップによる「IME-ON 半角英数」持続トグルの次アクション。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HalfWidthAlnumAction {
    None,
    Enter,
    Exit,
}

/// 半角英数持続トグルの entry/exit を純粋に計画する。
///
/// `toggle_active` が真なら、2回目タップ・右Shift緊急解除・左Shiftチョードを
/// 区別せず常に exit する。composition 中も exit は実行する非対称例外。
///
/// `toggle_active` の判定は `entry_supported` より**常に優先する**——
/// `entry_supported` は「新たに entry してよいか」だけを制御する条件であり、
/// 既に active な状態からの脱出をブロックしてはならない（entry 後に
/// IME 種別・belief・kill switch などが変化して `entry_supported` が
/// false に転じても、緊急解除で必ずかなへ戻れることを保証する）。
#[must_use]
pub const fn plan_half_width_alnum_action(
    is_left_shift_tap: bool,
    toggle_active: bool,
    entry_supported: bool,
    composing: bool,
) -> HalfWidthAlnumAction {
    if toggle_active {
        return HalfWidthAlnumAction::Exit;
    }
    if entry_supported && is_left_shift_tap && !composing {
        return HalfWidthAlnumAction::Enter;
    }
    HalfWidthAlnumAction::None
}

#[cfg(test)]
mod tests {
    use super::{plan_half_width_alnum_action as plan, HalfWidthAlnumAction};

    #[test]
    fn entry_only_on_inactive_left_shift_tap() {
        assert_eq!(plan(true, false, true, false), HalfWidthAlnumAction::Enter);
        assert_eq!(plan(false, false, true, false), HalfWidthAlnumAction::None);
    }

    #[test]
    fn composing_blocks_entry_but_not_exit() {
        assert_eq!(plan(true, false, true, true), HalfWidthAlnumAction::None);
        assert_eq!(plan(true, true, true, true), HalfWidthAlnumAction::Exit);
    }

    #[test]
    fn active_toggle_always_exits_regardless_of_left_shift_tap() {
        assert_eq!(plan(true, true, true, false), HalfWidthAlnumAction::Exit);
        assert_eq!(plan(false, true, true, false), HalfWidthAlnumAction::Exit);
    }

    #[test]
    fn unsupported_entry_blocks_enter_but_never_blocks_exit() {
        // entry_supported=false は新規 entry を止めるだけで、既に active な
        // トグルからの脱出（緊急解除）はブロックしない — entry 後に IME種別
        // 変化・kill switch・belief 変化等で entry_supported が false に
        // 転じても、ユーザーは必ずかなへ戻れる。
        assert_eq!(plan(true, false, false, false), HalfWidthAlnumAction::None);
        assert_eq!(plan(true, true, false, false), HalfWidthAlnumAction::Exit);
        assert_eq!(plan(false, true, false, true), HalfWidthAlnumAction::Exit);
    }
}
