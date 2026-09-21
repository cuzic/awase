//! 待ちモデルと押下のコスト。固定sleepと、イベント通知を待つ方式(`Event`)を比べられるようにする。

/// 押下後に結果が確定するまでの待ち方。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaitModel {
    /// 常に固定時間待つ。変化の遅延がこれを超えると、確定前の古い状態を読む(取りこぼし)。
    Fixed { ms: f64 },
    /// 通知を待つ。変化があれば「遅延+静止時間」(上限 `timeout_ms`)、通知が来なければ「変化なし」と見なして `nochange_ms` で確定する。
    /// 変化の遅延が `timeout_ms` を超えると取りこぼす。
    Event {
        quiet_ms: f64,
        nochange_ms: f64,
        timeout_ms: f64,
    },
}

/// コストモデル(ミリ秒)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostModel {
    /// 1回の押下(注入)そのものにかかる時間。現状の格子ではキー間隔(350ms)に当たる。
    pub press_ms: f64,
    pub wait: WaitModel,
    /// 状態(status)の読み取り1回。
    pub read_ms: f64,
    /// リセット(既知の初期状態へ戻す)。
    pub reset_ms: f64,
    /// S0(試行ごとに状態を作る方式)の、経路のキー間隔。
    pub setup_gap_ms: f64,
    /// S0 の、経路を打ち終えた後の待ち+検証。
    pub setup_settle_ms: f64,
}

impl CostModel {
    /// 現状の格子(固定待ち)。押下後 2300ms、キー間隔 350ms、リセット 1300ms、経路後の待ち+検証 1000ms。
    pub const fn current() -> Self {
        Self {
            press_ms: 350.0,
            wait: WaitModel::Fixed { ms: 2300.0 },
            read_ms: 5.0,
            reset_ms: 1300.0,
            setup_gap_ms: 350.0,
            setup_settle_ms: 1000.0,
        }
    }

    /// イベント通知を待つ方式(通知が来なければ150ms、静止40ms、上限700ms)。
    pub const fn event() -> Self {
        Self {
            press_ms: 10.0,
            wait: WaitModel::Event {
                quiet_ms: 40.0,
                nochange_ms: 150.0,
                timeout_ms: 700.0,
            },
            read_ms: 5.0,
            reset_ms: 1300.0,
            setup_gap_ms: 60.0,
            setup_settle_ms: 200.0,
        }
    }

    /// プランナが辺のコストを見積もるための期待値。`changed` は「事前モデル上で変化があるか」、
    /// `latency_ms` は変化の遅延の見積もり(中央値など)。
    pub fn expected_press_ms(&self, changed: bool, latency_ms: f64) -> f64 {
        match self.wait {
            WaitModel::Fixed { ms } => self.press_ms + ms,
            WaitModel::Event {
                quiet_ms,
                nochange_ms,
                timeout_ms,
            } => {
                if changed {
                    self.press_ms + latency_ms.min(timeout_ms) + quiet_ms
                } else {
                    self.press_ms + nochange_ms
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_is_cheaper_than_fixed_for_both_changed_and_unchanged() {
        let cur = CostModel::current();
        let ev = CostModel::event();
        assert!(ev.expected_press_ms(true, 60.0) < cur.expected_press_ms(true, 60.0));
        assert!(ev.expected_press_ms(false, 60.0) < cur.expected_press_ms(false, 60.0));
    }
}
