//! ADR-176 176-T10: 較正パネルの状態機械。UI（egui）・Win32からは独立した
//! 純粋ロジックとして分離し、Linux上でユニットテストできるようにする。

use awase_windows::calibration_ipc::CalibrationResultKind;

/// 較正パネルの状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CalibrationPanelState {
    /// 計測前、開始ボタン待ち。
    Idle,
    /// 対象VKが176-T5の`explicit_config_conflict_reason`に引っかかり、
    /// 開始できない（警告理由は呼び出し側がUIで表示、ここでは保持しない）。
    Blocked,
    /// 開始ボタンを押した直後、テキスト欄がまだフォーカスを得ていない。
    WaitingFocus,
    /// テキスト欄にフォーカスがあり、計測中（対象キー押下待ち）。
    Measuring,
    /// 計測中にテキスト欄からフォーカスが外れた（一時停止、案内表示）。
    FocusLost,
    /// 176-T9bから結果を受信し確定した。
    Confirmed(CalibrationResultKind),
}

/// 開始ボタン押下時の遷移。`blocked`はT5の
/// `explicit_config_conflict_reason`が`Some`を返したかどうか。
pub(crate) fn on_start_pressed(blocked: bool) -> CalibrationPanelState {
    if blocked {
        CalibrationPanelState::Blocked
    } else {
        CalibrationPanelState::WaitingFocus
    }
}

/// テキスト欄のフォーカス状態が変化した（または毎フレーム現在値を渡す）
/// ときの遷移。`Idle`/`Blocked`/`Confirmed`中はフォーカス変化を無視する
/// （計測中でなければフォーカス監視の対象外）。
pub(crate) fn on_focus_changed(
    current: CalibrationPanelState,
    has_focus: bool,
) -> CalibrationPanelState {
    match (current, has_focus) {
        (CalibrationPanelState::WaitingFocus | CalibrationPanelState::FocusLost, true) => {
            CalibrationPanelState::Measuring
        }
        (CalibrationPanelState::Measuring, false) => CalibrationPanelState::FocusLost,
        (other, _) => other,
    }
}

/// 176-T9bの結果を受信したときの遷移。計測中（`WaitingFocus`/`Measuring`/
/// `FocusLost`のいずれか、フォーカスが外れていても受信自体は起こりうる）
/// でなければ無視する。
pub(crate) fn on_result_received(
    current: CalibrationPanelState,
    kind: CalibrationResultKind,
) -> CalibrationPanelState {
    match current {
        CalibrationPanelState::WaitingFocus
        | CalibrationPanelState::Measuring
        | CalibrationPanelState::FocusLost => CalibrationPanelState::Confirmed(kind),
        other => other,
    }
}

/// キャンセル・結果確認後の「閉じる」ボタン押下時、常に`Idle`へ戻す。
pub(crate) fn on_cancel_or_close() -> CalibrationPanelState {
    CalibrationPanelState::Idle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_pressed_enters_waiting_or_blocked() {
        assert_eq!(on_start_pressed(false), CalibrationPanelState::WaitingFocus);
        assert_eq!(on_start_pressed(true), CalibrationPanelState::Blocked);
    }

    #[test]
    fn focus_changes_only_affect_measuring_flow() {
        assert_eq!(
            on_focus_changed(CalibrationPanelState::WaitingFocus, true),
            CalibrationPanelState::Measuring
        );
        assert_eq!(
            on_focus_changed(CalibrationPanelState::WaitingFocus, false),
            CalibrationPanelState::WaitingFocus
        );
        assert_eq!(
            on_focus_changed(CalibrationPanelState::Measuring, false),
            CalibrationPanelState::FocusLost
        );
        assert_eq!(
            on_focus_changed(CalibrationPanelState::Measuring, true),
            CalibrationPanelState::Measuring
        );
        assert_eq!(
            on_focus_changed(CalibrationPanelState::FocusLost, true),
            CalibrationPanelState::Measuring
        );
        assert_eq!(
            on_focus_changed(CalibrationPanelState::FocusLost, false),
            CalibrationPanelState::FocusLost
        );
        assert_eq!(
            on_focus_changed(CalibrationPanelState::Idle, true),
            CalibrationPanelState::Idle
        );
        assert_eq!(
            on_focus_changed(CalibrationPanelState::Blocked, true),
            CalibrationPanelState::Blocked
        );
        assert_eq!(
            on_focus_changed(
                CalibrationPanelState::Confirmed(CalibrationResultKind::ConfirmedOn),
                false,
            ),
            CalibrationPanelState::Confirmed(CalibrationResultKind::ConfirmedOn)
        );
    }

    #[test]
    fn result_received_confirms_only_measuring_flow() {
        assert_eq!(
            on_result_received(
                CalibrationPanelState::Measuring,
                CalibrationResultKind::ConfirmedOn,
            ),
            CalibrationPanelState::Confirmed(CalibrationResultKind::ConfirmedOn)
        );
        assert_eq!(
            on_result_received(
                CalibrationPanelState::Measuring,
                CalibrationResultKind::Rejected,
            ),
            CalibrationPanelState::Confirmed(CalibrationResultKind::Rejected)
        );
        assert_eq!(
            on_result_received(
                CalibrationPanelState::WaitingFocus,
                CalibrationResultKind::ConfirmedOn,
            ),
            CalibrationPanelState::Confirmed(CalibrationResultKind::ConfirmedOn)
        );
        assert_eq!(
            on_result_received(
                CalibrationPanelState::FocusLost,
                CalibrationResultKind::Rejected,
            ),
            CalibrationPanelState::Confirmed(CalibrationResultKind::Rejected)
        );
        assert_eq!(
            on_result_received(
                CalibrationPanelState::Idle,
                CalibrationResultKind::ConfirmedOn,
            ),
            CalibrationPanelState::Idle
        );
        assert_eq!(
            on_result_received(
                CalibrationPanelState::Blocked,
                CalibrationResultKind::ConfirmedOn,
            ),
            CalibrationPanelState::Blocked
        );
        assert_eq!(
            on_result_received(
                CalibrationPanelState::Confirmed(CalibrationResultKind::Rejected),
                CalibrationResultKind::ConfirmedOn,
            ),
            CalibrationPanelState::Confirmed(CalibrationResultKind::Rejected)
        );
    }

    #[test]
    fn cancel_or_close_always_returns_idle() {
        assert_eq!(on_cancel_or_close(), CalibrationPanelState::Idle);
    }
}
