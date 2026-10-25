//! TsfNative 窓へ入るときの、HWND キャッシュの ON 復元の判断(BUG-128、ADR-165、BUG-218)。純粋。
//!
//! `runtime/focus_tracking.rs::on_focus_process_changed` の TsfNative の分岐が使う。
//!
//! # 復元する条件
//! 1. キャッシュが ON で、`desired_open` が OFF で、キャッシュの hwnd が入場先の hwnd と一致する
//!    (BUG-128、ADR-165。同じ窓へ戻ってきた場合だけ。時間条件は設けない)。
//! 2. **(BUG-218 で追加)** そのキャッシュより新しい明示 OFF が、入場先と同じプロセスで押されていない。
//!
//! # 2 の理由
//! フォーカスの検出は周期の取りこぼしで遅れることがある(フォーカスの読みは refresh の周期で動く)。
//! 遅れている間に、ユーザーはすでに入場先の窓で Ctrl+無変換 を押している。このとき明示 OFF は
//! 「入場先の窓の中で」押されたもので、その窓のキャッシュ(窓を離れたときの古い ON)より新しい。
//! 古い ON で OFF を上書きすると、IME は実際には OFF のままなのにエンジンだけ ON になり、
//! 以後の打鍵が NICOLA として変換される(実機: `zoom` と打ったつもりが `zotumo` になった)。
//!
//! 時間だけで抑制する案は、ADR-165 の案 A として却下されている(「Chrome で OFF → Windows Terminal へ戻る」という
//! 35230fd の救済を壊す)。弁別子は時間ではなく、**OFF を押したのが入場先と同じプロセスか**: 35230fd の場面は
//! OFF が別のプロセス(Chrome)なので復元を続け、今回は同じプロセス(Windows Terminal)なので復元しない。
//! OFF を押した時点のプロセスが不明(`pid == 0`)のときは、従来どおり復元する(安全側は従来の挙動)。

use super::hwnd_cache::HwndImeSnapshot;

/// 直近の、ユーザーによる明示 IME OFF。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplicitOff {
    /// 押した時刻 (tick ms)。0 = 無い。
    pub at_ms: u64,
    /// 押した時点の前面プロセスの ID。0 = 不明。
    pub pid: u32,
}

impl ExplicitOff {
    /// 明示 OFF が無い、または前面プロセスが不明。
    pub const NONE: Self = Self { at_ms: 0, pid: 0 };
}

/// 復元の判断とその理由(ログに残す)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsfCacheDecision {
    /// キャッシュの ON を復元する。
    Restore,
    SkipNoCache,
    SkipCacheNotOn,
    SkipDesiredOpen,
    /// 同じ `(pid, class_name)` でも別の窓インスタンス(BUG-128)。
    SkipHwndMismatch,
    /// キャッシュより新しい明示 OFF が、入場先と同じプロセスで押されている(BUG-218)。
    SkipOffNewerInSameProcess,
}

impl TsfCacheDecision {
    #[must_use]
    pub const fn restores(self) -> bool {
        matches!(self, Self::Restore)
    }
}

/// TsfNative 窓への入場で、キャッシュの ON を復元するかを決める。
#[must_use]
pub fn plan_tsf_cache_restore(
    snap: Option<&HwndImeSnapshot>,
    desired_open: bool,
    new_hwnd: usize,
    new_pid: u32,
    explicit_off: ExplicitOff,
) -> TsfCacheDecision {
    let Some(snap) = snap else {
        return TsfCacheDecision::SkipNoCache;
    };
    if !snap.ime_on {
        return TsfCacheDecision::SkipCacheNotOn;
    }
    if desired_open {
        return TsfCacheDecision::SkipDesiredOpen;
    }
    if snap.hwnd != new_hwnd {
        return TsfCacheDecision::SkipHwndMismatch;
    }
    if explicit_off.pid != 0 && explicit_off.pid == new_pid && explicit_off.at_ms > snap.recorded_ms
    {
        return TsfCacheDecision::SkipOffNewerInSameProcess;
    }
    TsfCacheDecision::Restore
}

#[cfg(test)]
mod tests {
    use super::*;
    use awase::engine::InputModeState;

    const WT_PID: u32 = 2228;
    const CHROME_PID: u32 = 7000;

    fn snap(ime_on: bool, hwnd: usize, recorded_ms: u64) -> HwndImeSnapshot {
        HwndImeSnapshot {
            ime_on,
            input_mode: InputModeState::ObservedRomaji,
            recorded_ms,
            from_explicit_off_intent: false,
            hwnd,
        }
    }

    fn off(at_ms: u64, pid: u32) -> ExplicitOff {
        ExplicitOff { at_ms, pid }
    }

    #[test]
    fn restores_when_the_same_window_comes_back() {
        // 35230fd の救済: 同じ窓(同じ hwnd)へ戻ってきた場合は復元する。
        assert_eq!(
            plan_tsf_cache_restore(
                Some(&snap(true, 42, 1_000)),
                false,
                42,
                WT_PID,
                ExplicitOff::NONE
            ),
            TsfCacheDecision::Restore
        );
    }

    #[test]
    fn restores_when_the_explicit_off_was_pressed_in_another_process() {
        // 35230fd のシナリオ: Chrome で Ctrl+無変換(OFF)→ Windows Terminal へ戻る。OFF は別のプロセスなので復元する。
        assert!(plan_tsf_cache_restore(
            Some(&snap(true, 42, 1_000)),
            false,
            42,
            WT_PID,
            off(5_000, CHROME_PID)
        )
        .restores());
    }

    #[test]
    fn skips_when_a_newer_explicit_off_was_pressed_in_the_same_process() {
        // BUG-218: Windows Terminal の中で OFF を押した(5_000)。キャッシュは窓を離れたとき(1_000)の ON で、古い。
        assert_eq!(
            plan_tsf_cache_restore(
                Some(&snap(true, 42, 1_000)),
                false,
                42,
                WT_PID,
                off(5_000, WT_PID)
            ),
            TsfCacheDecision::SkipOffNewerInSameProcess
        );
    }

    #[test]
    fn restores_when_the_same_process_off_is_older_than_the_cache() {
        // 同じプロセスでも、OFF の方が古ければ(その後に窓を離れて ON のまま保存された)キャッシュが新しいので復元してよい。
        assert!(plan_tsf_cache_restore(
            Some(&snap(true, 42, 9_000)),
            false,
            42,
            WT_PID,
            off(5_000, WT_PID)
        )
        .restores());
    }

    #[test]
    fn restores_when_the_explicit_off_process_is_unknown() {
        // 押した時点のプロセスが不明(pid 0)のときは、従来どおり復元する。
        assert!(plan_tsf_cache_restore(
            Some(&snap(true, 42, 1_000)),
            false,
            42,
            WT_PID,
            off(5_000, 0)
        )
        .restores());
    }

    #[test]
    fn skips_when_hwnd_differs() {
        // BUG-128(report 01M27VXD4SPAD4STQ9TG1PZSCD): 同じ (pid, class_name) でも別の窓インスタンスでは復元しない。
        assert_eq!(
            plan_tsf_cache_restore(
                Some(&snap(true, 42, 1_000)),
                false,
                999,
                WT_PID,
                ExplicitOff::NONE
            ),
            TsfCacheDecision::SkipHwndMismatch
        );
    }

    #[test]
    fn skips_when_desired_open_is_already_true() {
        assert_eq!(
            plan_tsf_cache_restore(
                Some(&snap(true, 42, 1_000)),
                true,
                42,
                WT_PID,
                ExplicitOff::NONE
            ),
            TsfCacheDecision::SkipDesiredOpen
        );
    }

    #[test]
    fn skips_when_the_cache_says_off() {
        assert_eq!(
            plan_tsf_cache_restore(
                Some(&snap(false, 42, 1_000)),
                false,
                42,
                WT_PID,
                ExplicitOff::NONE
            ),
            TsfCacheDecision::SkipCacheNotOn
        );
    }

    #[test]
    fn skips_when_there_is_no_cache_entry() {
        assert_eq!(
            plan_tsf_cache_restore(None, false, 42, WT_PID, ExplicitOff::NONE),
            TsfCacheDecision::SkipNoCache
        );
    }
}
