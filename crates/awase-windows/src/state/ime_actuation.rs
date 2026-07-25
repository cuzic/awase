//! IME actuation の feedback（収束確認）方針と帰結の純データ型（ADR-080）。
//!
//! `FeedbackPolicy` はプロファイルごとの feedback 方針テンプレートで、`Copy` な
//! 純データ。`AppImePolicy`（`state/app_ime_policy.rs`）が `default_feedback` として
//! 保持できるよう、実行中の試行状態（attempts 等）は一切持たない。実行時状態を
//! 伴う `Actuation` は runtime 層（`runtime/ime_actuation.rs`）が別途持つ。

use super::ime_event::ObservationSource;

/// Feedback（収束確認）方針。プロファイルごとに `AppImePolicy::default_feedback` として持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackPolicy {
    /// 実読み戻しが可能（ImmCross 等）。
    Read {
        source: ObservationSource,
        deadline: std::time::Duration,
    },
    /// 読み戻し手段が構造的に存在しない（Imm32Unavailable / TsfNative）。
    /// 有限回で必ず打ち切る。
    Blind {
        max_attempts: u32,
        backoff: std::time::Duration,
    },
}

/// actuation 試行の帰結。`GaveUp`/deadline超過時は observations ストアへ一切書き込まない
/// （これを破るとBUG-33と同型の収束偽装バグになる — 絶対に守ること）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Confirmed,
    GaveUp,
}

/// `decide_actuation_action` の判定結果。次に actuate すべきか、打ち切るべきか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// wired up in a follow-up task（runtime 層への配線）。それまでは tests のみが参照する。
#[allow(dead_code)]
pub enum ActuationAction {
    /// まだ試行回数に余裕がある、実際に actuate してよい。
    Send,
    /// `Blind` の `max_attempts` 到達、`Resolution::GaveUp` にする。
    GiveUp,
}

/// `Blind` の有界終端を判定する純粋関数（`runtime`層がLinuxでテストできないため、
/// この核心ロジックだけ`state`層に切り出してある。ADR-080 / BUG-43 参照）。
///
/// `Blind` は `attempts >= max_attempts` で厳密に打ち切る（それ未満では決して諦めず、
/// それ以上でも決して `Send` に戻らない）。`Read` は試行回数だけでは打ち切らず常に
/// `Send` を返す（収束は観測確認で成立し、その終端は別処理が担う）。
#[must_use]
// wired up in a follow-up task（runtime 層への配線）。それまでは tests のみが参照する。
#[allow(dead_code)]
pub fn decide_actuation_action(policy: FeedbackPolicy, attempts: u32) -> ActuationAction {
    match policy {
        FeedbackPolicy::Blind { max_attempts, .. } => {
            if attempts >= max_attempts {
                ActuationAction::GiveUp
            } else {
                ActuationAction::Send
            }
        }
        FeedbackPolicy::Read { .. } => ActuationAction::Send,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blind(max_attempts: u32) -> FeedbackPolicy {
        FeedbackPolicy::Blind {
            max_attempts,
            backoff: std::time::Duration::from_millis(0),
        }
    }

    fn read() -> FeedbackPolicy {
        FeedbackPolicy::Read {
            source: ObservationSource::ImmGetOpenStatus,
            deadline: std::time::Duration::from_millis(0),
        }
    }

    #[test]
    fn blind_sends_before_reaching_max() {
        for attempts in 0..3 {
            assert_eq!(
                decide_actuation_action(blind(3), attempts),
                ActuationAction::Send,
                "attempts={attempts} は max_attempts=3 未満なので Send のはず"
            );
        }
    }

    #[test]
    fn blind_gives_up_exactly_at_max() {
        assert_eq!(
            decide_actuation_action(blind(3), 3),
            ActuationAction::GiveUp,
            "attempts == max_attempts の厳密境界で GiveUp"
        );
    }

    #[test]
    fn blind_stays_gave_up_past_max() {
        assert_eq!(
            decide_actuation_action(blind(3), 4),
            ActuationAction::GiveUp,
            "境界を越えても Send に戻らない"
        );
    }

    #[test]
    fn read_always_sends() {
        for attempts in [0, 1, 3, 4, 100, u32::MAX] {
            assert_eq!(
                decide_actuation_action(read(), attempts),
                ActuationAction::Send,
                "Read は試行回数で打ち切らない (attempts={attempts})"
            );
        }
    }
}
