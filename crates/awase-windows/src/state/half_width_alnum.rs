/// 左Shift単独タップによる「IME-ON 半角英数」持続トグルの次アクション。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HalfWidthAlnumAction {
    None,
    Enter,
    Exit,
}

/// このKeyUpを起こしたShiftキーの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftKeyUpKind {
    /// 左Shift、他の物理キーを一切介さない単独タップ。
    LeftTap,
    /// 左Shift、押下中に他の物理キーが挟まった（例: Shift+K のチョード）。
    LeftChord,
    /// 右Shift（タップ・チョードを問わない、常に緊急解除として扱う）。
    Right,
}

/// 半角英数持続トグルの entry/exit を純粋に計画する。
///
/// `toggle_active` が真のとき:
/// - 左Shiftの**単独タップ**（`LeftTap`）→ 2回目タップとして exit（トグルOFF）。
/// - **右Shift**（`Right`）→ タップ・チョードを問わず常に exit（緊急解除）。
/// - 左Shiftの**チョード**（`LeftChord`、例: Shift+K で大文字を打つ）→ **exit しない**。
///   半角英数トグルは「押しながらの他キー入力」を大文字化するための一時的な
///   Shift 修飾として使えるべきで、Shift を離しただけでトグルが解除されては
///   ユーザーが意図せず持続モードから抜けてしまう（実機で報告された不具合）。
///
/// composition 中は exit を妨げない（非対称例外）。
///
/// `toggle_active` の判定は `entry_supported` より**常に優先する**——
/// `entry_supported` は「新たに entry してよいか」だけを制御する条件であり、
/// 既に active な状態からの脱出をブロックしてはならない（entry 後に
/// IME 種別・belief・kill switch などが変化して `entry_supported` が
/// false に転じても、緊急解除で必ずかなへ戻れることを保証する）。
#[must_use]
pub const fn plan_half_width_alnum_action(
    shift_up: ShiftKeyUpKind,
    toggle_active: bool,
    entry_supported: bool,
    composing: bool,
) -> HalfWidthAlnumAction {
    if toggle_active {
        if matches!(shift_up, ShiftKeyUpKind::LeftChord) {
            return HalfWidthAlnumAction::None;
        }
        return HalfWidthAlnumAction::Exit;
    }
    if entry_supported && matches!(shift_up, ShiftKeyUpKind::LeftTap) && !composing {
        return HalfWidthAlnumAction::Enter;
    }
    HalfWidthAlnumAction::None
}

#[cfg(test)]
mod tests {
    use super::{plan_half_width_alnum_action as plan, HalfWidthAlnumAction, ShiftKeyUpKind};

    #[test]
    fn entry_only_on_inactive_left_shift_tap() {
        assert_eq!(
            plan(ShiftKeyUpKind::LeftTap, false, true, false),
            HalfWidthAlnumAction::Enter
        );
        assert_eq!(
            plan(ShiftKeyUpKind::Right, false, true, false),
            HalfWidthAlnumAction::None
        );
        assert_eq!(
            plan(ShiftKeyUpKind::LeftChord, false, true, false),
            HalfWidthAlnumAction::None
        );
    }

    #[test]
    fn composing_blocks_entry_but_not_exit() {
        assert_eq!(
            plan(ShiftKeyUpKind::LeftTap, false, true, true),
            HalfWidthAlnumAction::None
        );
        assert_eq!(
            plan(ShiftKeyUpKind::LeftTap, true, true, true),
            HalfWidthAlnumAction::Exit
        );
    }

    #[test]
    fn active_toggle_second_tap_and_right_shift_exit_but_left_chord_persists() {
        // 2回目の左Shiftタップ・右Shiftは exit。
        assert_eq!(
            plan(ShiftKeyUpKind::LeftTap, true, true, false),
            HalfWidthAlnumAction::Exit
        );
        assert_eq!(
            plan(ShiftKeyUpKind::Right, true, true, false),
            HalfWidthAlnumAction::Exit
        );
        // 左Shiftチョード（Shift+文字キーで大文字を打つ用途）は exit しない
        // — トグル中に Shift を離しただけで持続モードから抜けてしまう
        // 不具合の修正（実機報告）。
        assert_eq!(
            plan(ShiftKeyUpKind::LeftChord, true, true, false),
            HalfWidthAlnumAction::None
        );
        // composition 中でも同様（決定5の exit 側非対称例外は Exit にのみ働く。
        // LeftChord は元々 exit しないので composition の有無は無関係）。
        assert_eq!(
            plan(ShiftKeyUpKind::LeftChord, true, true, true),
            HalfWidthAlnumAction::None
        );
    }

    #[test]
    fn unsupported_entry_blocks_enter_but_never_blocks_tap_or_right_shift_exit() {
        // entry_supported=false は新規 entry を止めるだけで、既に active な
        // トグルからの脱出（緊急解除）はブロックしない — entry 後に IME種別
        // 変化・kill switch・belief 変化等で entry_supported が false に
        // 転じても、ユーザーは必ずかなへ戻れる。
        assert_eq!(
            plan(ShiftKeyUpKind::LeftTap, false, false, false),
            HalfWidthAlnumAction::None
        );
        assert_eq!(
            plan(ShiftKeyUpKind::LeftTap, true, false, false),
            HalfWidthAlnumAction::Exit
        );
        assert_eq!(
            plan(ShiftKeyUpKind::Right, true, false, true),
            HalfWidthAlnumAction::Exit
        );
        // LeftChord は entry_supported の値に関わらず常に None（exitしない
        // という結論自体は entry 可否の設定と無関係）。
        assert_eq!(
            plan(ShiftKeyUpKind::LeftChord, true, false, false),
            HalfWidthAlnumAction::None
        );
    }
}
