//! GJI I/O 観測モジュール — IMM-broken アプリ向けの GJI 活動検出。

use crate::tuning::GJI_CONFIRM_WINDOW_MS;

/// `observe_gji_after_focus` の結果。
pub(crate) struct GjiBlacklistObservation {
    /// `observer_poll` スロットに書き込むべき値。`None` = 書き込み不要。
    pub observer_poll_value: Option<bool>,
    /// 観測時刻 (ms)
    pub now_ms: u64,
}

/// IMM-broken クラス（Chrome/Edge 等）向け GJI I/O 観測。
///
/// フォーカス変更より後の GJI I/O があれば observer_poll=true を返す。
/// フォーカス変更前の GJI I/O は「クロスウィンドウ汚染」として無視する。
pub(crate) fn observe_gji_after_focus(last_focus_change_ms: u64) -> GjiBlacklistObservation {
    let now_ms = crate::hook::current_tick_ms();
    let last_io = crate::tsf::observer::tsf_obs().gji_last_io_ms();
    let gji_after_focus = last_io > last_focus_change_ms;

    if last_io > 0 && gji_after_focus && now_ms.saturating_sub(last_io) < GJI_CONFIRM_WINDOW_MS {
        log::debug!(
            "[gji-poll] GJI I/O observed {}ms ago (after focus+{}ms) → observer_poll=true",
            now_ms.saturating_sub(last_io),
            last_io.saturating_sub(last_focus_change_ms),
        );
        GjiBlacklistObservation { observer_poll_value: Some(true), now_ms }
    } else {
        if last_io > 0 && !gji_after_focus {
            log::debug!(
                "[gji-poll] GJI I/O {}ms ago predates focus change ({}ms before focus) → skipped",
                now_ms.saturating_sub(last_io),
                last_focus_change_ms.saturating_sub(last_io),
            );
        }
        GjiBlacklistObservation { observer_poll_value: None, now_ms }
    }
}
