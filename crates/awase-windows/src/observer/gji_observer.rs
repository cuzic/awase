//! GJI I/O 観測モジュール — IMM-broken アプリ向けの GJI 活動検出。

use awase::engine::InputModeState;

use crate::tuning::GJI_CONFIRM_WINDOW_MS;

/// `observe_gji_after_focus` の結果。
pub(crate) struct GjiBlacklistObservation {
    /// `observer_poll` スロットに書き込むべき値。`None` = 書き込み不要。
    pub observer_poll_value: Option<bool>,
    /// stale `ObservedEisu` への訂正値。`None` = 訂正不要。
    ///
    /// GJI がフォーカス後に変換 I/O をしている = 英数モードではない、という
    /// 矛盾証拠（`state::eisu_recovery::gji_io_eisu_correction`）。呼び出し元は
    /// `InputModeObserved { source: GjiIoInference, confidence: Medium }` で
    /// dispatch すること。
    pub input_mode_correction: Option<InputModeState>,
}

/// IMM-broken クラス（Chrome/Edge 等）向け GJI I/O 観測。
///
/// フォーカス変更より後の GJI I/O があれば observer_poll=true を返す。
/// フォーカス変更前の GJI I/O は「クロスウィンドウ汚染」として無視する。
///
/// 呼び出し元は active IME が GJI であることを確認してから呼ぶこと
/// （MS-IME 使用中は常駐 GJI Converter プロセスのバックグラウンド I/O を
/// 根拠に belief を書いてしまうため）。
pub(crate) fn observe_gji_after_focus(
    last_focus_change_ms: u64,
    current_input_mode: InputModeState,
) -> GjiBlacklistObservation {
    let now_ms = crate::hook::current_tick_ms();
    let obs = crate::tsf::observer::tsf_obs();
    let last_io = obs.gji_last_io_ms();
    // 接続直後の最初の読みは実 I/O ではない（BUG-176: 初回セッションで GJI Converter が遅れて現れると、
    // 外部から閉じられた直後に「GJI が動いた = ON」と誤読して追随結果を ON へ戻していた）。
    if crate::tsf::observer::gji_io_is_attach_artifact(last_io, obs.gji_attach_ms()) {
        tracing::debug!("[gji-poll] GJI I/O は接続直後の初回読み(実 I/O ではない) → skipped");
        return GjiBlacklistObservation {
            observer_poll_value: None,
            input_mode_correction: None,
        };
    }
    let gji_after_focus = last_io > last_focus_change_ms;

    if last_io > 0 && gji_after_focus && now_ms.saturating_sub(last_io) < GJI_CONFIRM_WINDOW_MS {
        tracing::debug!(
            "[gji-poll] GJI I/O observed {}ms ago (after focus+{}ms) → observer_poll=true",
            now_ms.saturating_sub(last_io),
            last_io.saturating_sub(last_focus_change_ms),
        );
        GjiBlacklistObservation {
            observer_poll_value: Some(true),
            input_mode_correction: crate::state::eisu_recovery::gji_io_eisu_correction(
                true,
                current_input_mode,
            ),
        }
    } else {
        if last_io > 0 && !gji_after_focus {
            tracing::debug!(
                "[gji-poll] GJI I/O {}ms ago predates focus change ({}ms before focus) → skipped",
                now_ms.saturating_sub(last_io),
                last_focus_change_ms.saturating_sub(last_io),
            );
        }
        GjiBlacklistObservation {
            observer_poll_value: None,
            input_mode_correction: None,
        }
    }
}
