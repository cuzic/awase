//! 異常の分類と、リセット段階・昇格の方針(設計メモ「異常の分類と復帰」)。

use std::collections::VecDeque;

/// 実行中に検出する異常。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Anomaly {
    /// 注入したキーがアプリ側に届かなかった(アプリ側の打鍵イベントで検出)。
    KeyNotDelivered,
    /// 観測経路(例: IMMとTSF compartment)の不一致。
    ChannelMismatch,
    /// 期待した次状態でも既知のグラフの状態でもない(同期喪失)。
    UnexpectedStatus,
    /// リセットしたが既知の初期状態に戻らなかった。
    ResetFailed,
}

/// リセットの段階。軽い順。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResetLevel {
    /// Esc×2と入力欄クリア程度。
    Soft,
    /// IME ONとひらがな往復で既知状態へ(キー到達で検証)。
    Mode,
    /// 窓やプロセスの作り直し、プロファイルの再有効化。
    Hard,
}

impl ResetLevel {
    /// 基準のリセットコスト `reset_ms` に対する倍率。
    pub const fn cost_factor(self) -> f64 {
        match self {
            Self::Soft => 0.25,
            Self::Mode => 1.0,
            Self::Hard => 2.3,
        }
    }

    /// 基準のリセット失敗確率 `r` に対する、そのレベルの成功確率。
    pub fn success_prob(self, reset_fail_prob: f64) -> f64 {
        match self {
            Self::Soft => (1.0 - reset_fail_prob) * 0.7,
            Self::Mode => 1.0 - reset_fail_prob,
            Self::Hard => 1.0,
        }
    }

    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Soft => Some(Self::Mode),
            Self::Mode => Some(Self::Hard),
            Self::Hard => None,
        }
    }
}

/// 異常時の方針。
#[derive(Debug, Clone, Copy)]
pub struct AnomalyPolicy {
    /// キー未着のときの再試行回数。
    pub max_press_retries: u32,
    /// 直近何回の押下を窓として異常を数えるか。
    pub window: usize,
    /// 窓内の異常がこの数に達したら、強制的にリセット(Modeレベル)する。
    pub escalate_threshold: usize,
    /// 通常のリセットで最初に試す段階。
    pub first_reset: ResetLevel,
}

impl Default for AnomalyPolicy {
    fn default() -> Self {
        Self {
            max_press_retries: 1,
            window: 20,
            escalate_threshold: 3,
            first_reset: ResetLevel::Mode,
        }
    }
}

/// 直近の押下ごとの異常の有無を数える。
#[derive(Debug, Clone)]
pub struct AnomalyTracker {
    policy: AnomalyPolicy,
    recent: VecDeque<bool>,
}

impl AnomalyTracker {
    pub fn new(policy: AnomalyPolicy) -> Self {
        Self {
            policy,
            recent: VecDeque::new(),
        }
    }

    pub const fn policy(&self) -> &AnomalyPolicy {
        &self.policy
    }

    /// 1回の押下の結果(異常があったか)を記録する。
    pub fn note(&mut self, anomalous: bool) {
        self.recent.push_back(anomalous);
        while self.recent.len() > self.policy.window {
            self.recent.pop_front();
        }
    }

    /// 強制リセットすべきか(窓内の異常が閾値以上)。
    pub fn should_force_reset(&self) -> bool {
        self.recent.iter().filter(|a| **a).count() >= self.policy.escalate_threshold
    }

    pub fn clear(&mut self) {
        self.recent.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escalates_after_threshold_within_window() {
        let mut t = AnomalyTracker::new(AnomalyPolicy {
            window: 5,
            escalate_threshold: 2,
            ..AnomalyPolicy::default()
        });
        t.note(true);
        assert!(!t.should_force_reset());
        t.note(false);
        t.note(true);
        assert!(t.should_force_reset());
        t.clear();
        assert!(!t.should_force_reset());
    }

    #[test]
    fn old_anomalies_fall_out_of_the_window() {
        let mut t = AnomalyTracker::new(AnomalyPolicy {
            window: 3,
            escalate_threshold: 2,
            ..AnomalyPolicy::default()
        });
        t.note(true);
        t.note(true);
        for _ in 0..3 {
            t.note(false);
        }
        assert!(!t.should_force_reset());
    }

    #[test]
    fn reset_levels_escalate_and_end() {
        assert_eq!(ResetLevel::Soft.next(), Some(ResetLevel::Mode));
        assert_eq!(ResetLevel::Hard.next(), None);
        assert!(ResetLevel::Hard.cost_factor() > ResetLevel::Soft.cost_factor());
    }
}
