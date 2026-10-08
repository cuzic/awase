//! raw TSF literal の回収(ESC/BS/romaji の再送)を、送るか捨てるかの判断(BUG-194、ADR-246)。
//!
//! 回収は literal を打った旧窓宛て。段の開始に採った `(focus 世代, 前景窓)` と、flush 時点のそれを比べ、
//! 宛先の窓が変わっていれば送らずに捨てる。
//!
//! 主な物差しは**前景窓**(`GetForegroundWindow`、OS の事実)。`ime_mode_focus_gen` は debounce(50ms)と非同期 prefetch の後でしか
//! 進まず、窓切替直後の最初の打鍵が張る段は「旧窓の世代 N・新しい窓 B」で刻まれ、段の途中で N+1 に進む。
//! 世代の不一致だけで捨てると、その B 宛ての正しい回収と後続の打鍵を失う(round 2 N-M1)。
//! そのため、前景窓が記録時・flush 時とも取れていて同じなら世代の不一致では捨てない。世代は前景が取れないときの代用に使う。
//! 代償: 同じ前景窓の中のフォーカス移動(Chrome のアドレスバーとコンテンツ等)は捉えられない。

use super::focus_gen::FocusGen;
use super::foreground_scope::ForegroundScope;

/// 段の開始時(または flush 時)に採る宛先の識別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageOrigin {
    pub focus_gen: FocusGen,
    pub foreground: ForegroundScope,
}

/// 回収をどうするか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawRecoveryDisposition {
    /// そのまま送る。
    Send,
    /// 宛先が変わったので送らず、予約と deferred を捨てる。
    DiscardStale,
}

/// `recorded` は段の開始時に採った値(採れていなければ `None`)、`now` は flush 時点の値。
///
/// - `None` は判断材料が無いので送る(従来の挙動)。
/// - どちらかで前景窓が取れていれば、前景窓だけで決める(違えば捨てる)。取れていた窓が `INVALID` になった場合も、
///   Alt+Tab 画面・UAC・ロック等で BS を送るべきでないので捨てる。
/// - どちらも前景が取れていなければ、世代の不一致で決める。
#[must_use]
pub fn plan_raw_recovery(
    recorded: Option<StageOrigin>,
    now: StageOrigin,
) -> RawRecoveryDisposition {
    let Some(recorded) = recorded else {
        return RawRecoveryDisposition::Send;
    };
    let differs = if recorded.foreground.is_valid() || now.foreground.is_valid() {
        recorded.foreground != now.foreground
    } else {
        recorded.focus_gen != now.focus_gen
    };
    if differs {
        RawRecoveryDisposition::DiscardStale
    } else {
        RawRecoveryDisposition::Send
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G0: FocusGen = FocusGen::INITIAL;

    fn origin(focus_gen: FocusGen, pid: u32, hwnd: isize) -> StageOrigin {
        StageOrigin {
            focus_gen,
            foreground: ForegroundScope { pid, hwnd },
        }
    }

    #[test]
    fn same_gen_and_foreground_sends() {
        assert_eq!(
            plan_raw_recovery(Some(origin(G0, 1, 10)), origin(G0, 1, 10)),
            RawRecoveryDisposition::Send
        );
    }

    #[test]
    fn no_recorded_origin_sends() {
        assert_eq!(
            plan_raw_recovery(None, origin(G0.next(), 2, 20)),
            RawRecoveryDisposition::Send
        );
    }

    #[test]
    fn gen_advanced_in_same_foreground_sends() {
        // N-M1: 窓切替直後の最初の打鍵が張った段は「旧窓の世代 N・新しい窓 B」で刻まれ、段の途中で世代が N+1 に進む。
        // 前景窓 B は変わっていないので、B 宛ての正しい回収と後続の打鍵を捨ててはならない。
        assert_eq!(
            plan_raw_recovery(Some(origin(G0, 1, 10)), origin(G0.next(), 1, 10)),
            RawRecoveryDisposition::Send
        );
    }

    #[test]
    fn foreground_changed_before_gen_advanced_discards() {
        // debounce/prefetch の最中: 前景は変わったが世代はまだ進んでいない。
        assert_eq!(
            plan_raw_recovery(Some(origin(G0, 1, 10)), origin(G0, 2, 20)),
            RawRecoveryDisposition::DiscardStale
        );
        // 同じプロセス内の別 hwnd でも宛先は変わっている。
        assert_eq!(
            plan_raw_recovery(Some(origin(G0, 1, 10)), origin(G0, 1, 11)),
            RawRecoveryDisposition::DiscardStale
        );
    }

    #[test]
    fn foreground_unknown_at_record_but_valid_now_discards() {
        // 記録時に前景が取れず（活性化途中など）、flush 時に取れた: 窓が特定できない回収は捨てる側に倒す。
        let invalid = StageOrigin {
            focus_gen: G0,
            foreground: ForegroundScope::INVALID,
        };
        assert_eq!(
            plan_raw_recovery(Some(invalid), origin(G0, 1, 10)),
            RawRecoveryDisposition::DiscardStale
        );
    }

    #[test]
    fn foreground_lost_after_valid_record_discards() {
        assert_eq!(
            plan_raw_recovery(
                Some(origin(G0, 1, 10)),
                StageOrigin {
                    focus_gen: G0,
                    foreground: ForegroundScope::INVALID
                }
            ),
            RawRecoveryDisposition::DiscardStale
        );
    }

    #[test]
    fn foreground_never_known_falls_back_to_gen_only() {
        let invalid = |g| StageOrigin {
            focus_gen: g,
            foreground: ForegroundScope::INVALID,
        };
        assert_eq!(
            plan_raw_recovery(Some(invalid(G0)), invalid(G0)),
            RawRecoveryDisposition::Send
        );
        assert_eq!(
            plan_raw_recovery(Some(invalid(G0)), invalid(G0.next())),
            RawRecoveryDisposition::DiscardStale,
            "前景が取れないときは世代で決める"
        );
    }
}
