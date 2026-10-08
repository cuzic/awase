//! raw TSF literal の回収(ESC/BS/romaji の再送)を、送るか捨てるかの判断(BUG-194、ADR-246)。
//!
//! 回収は literal を打った旧窓宛て。段の開始(最初の VK 送信より前)に採った `(focus 世代, 前景窓)` と、
//! flush 時点のそれが違えば、宛先の窓が変わっているので送らずに捨てる。
//! 世代は debounce と非同期 prefetch の後でしか進まない(OS の前景はもう変わっているのに世代がまだ、という区間がある)ので、
//! 世代だけでは足りず、`GetForegroundWindow` の事実を併用する。

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
/// - 世代が違えば捨てる。
/// - 前景窓が違えば捨てる。前景が取れない(`INVALID`)ときも、記録時に取れていた窓とは等しくないので捨てる側に倒れる。
///   記録時にも取れていなかった(`INVALID` 同士)ときは前景を判断に使わず、世代だけで決める。
#[must_use]
pub fn plan_raw_recovery(
    recorded: Option<StageOrigin>,
    now: StageOrigin,
) -> RawRecoveryDisposition {
    let Some(recorded) = recorded else {
        return RawRecoveryDisposition::Send;
    };
    if recorded.focus_gen != now.focus_gen {
        return RawRecoveryDisposition::DiscardStale;
    }
    if (recorded.foreground.is_valid() || now.foreground.is_valid())
        && recorded.foreground != now.foreground
    {
        return RawRecoveryDisposition::DiscardStale;
    }
    RawRecoveryDisposition::Send
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
    fn gen_advanced_discards() {
        assert_eq!(
            plan_raw_recovery(Some(origin(G0, 1, 10)), origin(G0.next(), 1, 10)),
            RawRecoveryDisposition::DiscardStale
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
            RawRecoveryDisposition::DiscardStale
        );
    }
}
