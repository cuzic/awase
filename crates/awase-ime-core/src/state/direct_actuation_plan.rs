//! EXPERIMENTAL（awase `exp/mozc-direct-actuation`、未マージ）: Mozc Readable の直接actuationコマンド
//! （version 6、未リリース。`readable/docs/shared-memory.md` 6-5 節）を使った、閉ループの実行計画を決める
//! 純粋関数。
//!
//! 背景: Mozc Readable を使っている間、合成キー送信＋共有メモリのポーリングで成功を**推測**する開ループ
//! ではなく、送った要求ごとに成否を**確認**し、確認できなければ再送するか諦めるかを決める閉ループに
//! したい（所有者方針、2026-11-05）。
//!
//! 送信（`PostMessage`）と観測（共有メモリの読み取り）は殻の仕事。この module は、
//!
//! 1. `nonce` を送った時刻・結果（[`DirectActuationAck`]）・いま観測している開閉状態を [`Facts`] として渡す。
//! 2. [`decide`]（核、この module）が、待つ・確認済み・拒否・再送・諦めのどれかを [`Plan`] で返す。
//! 3. 殻が、計画どおりに再送（新しい nonce で `PostMessage`）するか、諦めて他の経路（従来のキー送信など）
//!    へ落とすかを行う。
//!
//! 核は OS・壁時計・グローバルに触れない。時刻は [`Facts::now`]/[`Facts::sent_at`] で受ける（`drift_plan.rs`
//! と同じ流儀）。

use super::mozc_shm::{DirectActuationAck, DirectActuationResult};

/// 0 は「要求なし」の意味（ワイヤー契約、`readable/docs/shared-memory.md` 6-5 節）なので nonce には使えない。
/// `current` の次の値（0 を飛ばす、ラップあり）。
#[must_use]
pub const fn next_nonce(current: u32) -> u32 {
    let next = current.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

/// [`decide`] への入力（所有型の事実）。
#[derive(Debug, Clone, Copy)]
pub struct Facts {
    pub now: std::time::Instant,
    /// 最後に送った（まだ結果待ちの）要求を送信した時刻。
    pub sent_at: std::time::Instant,
    /// 最後に送った要求の nonce（0 ではない）。
    pub nonce: u32,
    /// 望む開閉状態。
    pub desired_open: bool,
    /// 共有メモリから読んだ、最新の直接actuationコマンドの ack（読めなければ `None`）。
    pub ack: Option<DirectActuationAck>,
    /// 共有メモリから読んだ、最新の開閉状態（`tsf_open`。ADR-265 D3 既定、読めなければ `None`）。
    pub observed_open: Option<bool>,
    /// この `nonce` を送るまでに、同じ目標ですでに送った回数（最初の送信を含む。1 から）。
    pub attempts: u32,
    /// これを超えて再送しない上限。
    pub max_attempts: u32,
    /// `sent_at` からこれだけ経っても ack が自分の nonce にならなければ、タイムアウトとして再送・諦めを判断する。
    pub timeout: std::time::Duration,
}

/// [`decide`] の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// まだ結果を待てる（タイムアウトしていない）。何もしない。
    Wait,
    /// 自分の nonce が `Applied` になり、観測している開閉状態も望む値と一致した。閉ループの成功。
    Confirmed,
    /// 自分の nonce の結果が付いたが、`Applied` ではなく、タイミングの問題ではない拒否（フォーカス無し・
    /// server round trip 失敗・予約ビット）。すぐに再送してよいか、通常の再送上限に従うかは呼び出し元が決める
    /// （この型はその判断の材料として理由を返すだけ）。
    Rejected(DirectActuationResult),
    /// 自分の nonce が `Applied` になったが、観測している開閉状態が望む値と食い違う（ack と観測の読み取り元が
    /// ずれている可能性がある。例えば ack は読めたが開閉状態の読み取りが古い）。再送ではなく、もう一度観測
    /// を読み直すべき状況。
    AppliedButStateMismatch,
    /// タイムアウトし、再送の余地がある。呼び出し元は新しい nonce で送り直す。
    TimedOutRetry,
    /// タイムアウトし、再送の上限に達した。呼び出し元は諦めて別の経路（従来のキー送信など）へ落とすか、
    /// 「不明」として扱う。
    TimedOutGiveUp,
}

/// 何をすべきかを決める（核）。
#[must_use]
pub fn decide(facts: &Facts) -> Plan {
    if let Some(ack) = facts.ack {
        if ack.last_cmd_nonce == facts.nonce {
            return match ack.last_cmd_result {
                DirectActuationResult::Applied => {
                    if facts.observed_open == Some(facts.desired_open) {
                        Plan::Confirmed
                    } else {
                        Plan::AppliedButStateMismatch
                    }
                }
                DirectActuationResult::None => timeout_or_wait(facts),
                other => Plan::Rejected(other),
            };
        }
    }
    timeout_or_wait(facts)
}

fn timeout_or_wait(facts: &Facts) -> Plan {
    if facts.now.saturating_duration_since(facts.sent_at) < facts.timeout {
        return Plan::Wait;
    }
    if facts.attempts < facts.max_attempts {
        Plan::TimedOutRetry
    } else {
        Plan::TimedOutGiveUp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn facts(
        now_offset_ms: u64,
        ack: Option<DirectActuationAck>,
        observed_open: Option<bool>,
    ) -> Facts {
        let sent_at = Instant::now();
        Facts {
            now: sent_at + Duration::from_millis(now_offset_ms),
            sent_at,
            nonce: 7,
            desired_open: true,
            ack,
            observed_open,
            attempts: 1,
            max_attempts: 3,
            timeout: Duration::from_millis(500),
        }
    }

    #[test]
    fn next_nonce_skips_zero_and_wraps() {
        assert_eq!(next_nonce(0), 1);
        assert_eq!(next_nonce(1), 2);
        assert_eq!(next_nonce(u32::MAX), 1); // wrapping_add(1) に加えて 0 を飛ばす
        assert_eq!(next_nonce(u32::MAX - 1), u32::MAX);
    }

    #[test]
    fn no_ack_yet_within_timeout_waits() {
        assert_eq!(decide(&facts(10, None, None)), Plan::Wait);
    }

    #[test]
    fn ack_for_a_different_nonce_is_treated_as_no_ack_yet() {
        let ack = DirectActuationAck {
            last_cmd_nonce: 999, // 自分の nonce (7) ではない
            last_cmd_result: DirectActuationResult::Applied,
            cmd_window_handle: 0x1000,
        };
        assert_eq!(decide(&facts(10, Some(ack), Some(true))), Plan::Wait);
    }

    #[test]
    fn applied_with_matching_state_is_confirmed() {
        let ack = DirectActuationAck {
            last_cmd_nonce: 7,
            last_cmd_result: DirectActuationResult::Applied,
            cmd_window_handle: 0x1000,
        };
        assert_eq!(decide(&facts(10, Some(ack), Some(true))), Plan::Confirmed);
    }

    #[test]
    fn applied_but_observed_state_disagrees_is_a_mismatch_not_a_retry() {
        let ack = DirectActuationAck {
            last_cmd_nonce: 7,
            last_cmd_result: DirectActuationResult::Applied,
            cmd_window_handle: 0x1000,
        };
        // desired_open=true（facts ヘルパ既定）だが観測は false
        assert_eq!(
            decide(&facts(10, Some(ack), Some(false))),
            Plan::AppliedButStateMismatch
        );
    }

    #[test]
    fn applied_but_state_not_observed_yet_is_a_mismatch() {
        let ack = DirectActuationAck {
            last_cmd_nonce: 7,
            last_cmd_result: DirectActuationResult::Applied,
            cmd_window_handle: 0x1000,
        };
        assert_eq!(
            decide(&facts(10, Some(ack), None)),
            Plan::AppliedButStateMismatch
        );
    }

    #[test]
    fn non_applied_result_for_own_nonce_is_rejected_with_its_reason() {
        for result in [
            DirectActuationResult::NoFocusedContext,
            DirectActuationResult::SendCommandFailed,
            DirectActuationResult::ReservedBitsSet,
            DirectActuationResult::Unknown(99),
        ] {
            let ack = DirectActuationAck {
                last_cmd_nonce: 7,
                last_cmd_result: result,
                cmd_window_handle: 0x1000,
            };
            assert_eq!(decide(&facts(10, Some(ack), None)), Plan::Rejected(result));
        }
    }

    #[test]
    fn none_result_for_own_nonce_is_treated_as_not_processed_yet() {
        // last_cmd_nonce が自分の値と一致しても result=None は「まだ処理していない」の初期値
        // （読み手が、まだ発行されていない前回の nonce の残骸を同じ値で読んだ場合などを除けば実際には
        // 起きない組み合わせだが、起きても誤って Confirmed/Rejected にしない）。
        let ack = DirectActuationAck {
            last_cmd_nonce: 7,
            last_cmd_result: DirectActuationResult::None,
            cmd_window_handle: 0x1000,
        };
        assert_eq!(decide(&facts(10, Some(ack), None)), Plan::Wait);
    }

    #[test]
    fn timeout_with_attempts_remaining_retries() {
        let mut f = facts(10_000, None, None); // timeout=500ms を大きく超えた
        f.attempts = 1;
        f.max_attempts = 3;
        assert_eq!(decide(&f), Plan::TimedOutRetry);
    }

    #[test]
    fn timeout_at_the_attempt_limit_gives_up() {
        let mut f = facts(10_000, None, None);
        f.attempts = 3;
        f.max_attempts = 3;
        assert_eq!(decide(&f), Plan::TimedOutGiveUp);
    }

    #[test]
    fn exactly_at_the_timeout_boundary_is_already_timed_out() {
        // `< timeout` で Wait を続ける条件を書いているので、ちょうど timeout に達した瞬間はもう
        // Wait ではない（"500ms 経っても ack が来なければ" の "500ms 経っても" を含む側）。
        let sent_at = Instant::now();
        let f = Facts {
            now: sent_at + Duration::from_millis(500),
            sent_at,
            nonce: 7,
            desired_open: true,
            ack: None,
            observed_open: None,
            attempts: 1,
            max_attempts: 3,
            timeout: Duration::from_millis(500),
        };
        assert_eq!(decide(&f), Plan::TimedOutRetry);
    }

    #[test]
    fn just_before_the_timeout_boundary_still_waits() {
        let sent_at = Instant::now();
        let f = Facts {
            now: sent_at + Duration::from_millis(499),
            sent_at,
            nonce: 7,
            desired_open: true,
            ack: None,
            observed_open: None,
            attempts: 1,
            max_attempts: 3,
            timeout: Duration::from_millis(500),
        };
        assert_eq!(decide(&f), Plan::Wait);
    }
}
