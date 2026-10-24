//! ペース付き送信のキュー(BUG-217)。**メッセージループを止めずに**、送信の間に待ちを置くための純粋な状態。
//!
//! Zoom のチャット窓は、注入キーの「↓から↑まで」が短いと一部を取りこぼす。待ちを `thread::sleep` で置くと
//! メッセージループが止まるので、「送る → 待つ → 次を送る」を、キューと(呼び出し側の)非同期タイマーで進める。
//!
//! 1 つの要素は「今送る中身」と「送ったあとの待ち(ms)」。待っている間(`pacing`)に来た要素は、待ちが 0 でも
//! 後ろに並べる。**送信の順序は、提出の順序のまま変わらない**。
//!
//! 呼び出し側の使い方:
//! 1. `submit(item, delay)`。`SendNow` なら今すぐ送る。`arm_wait_ms` が `Some(ms)` なら、`ms` 待ってから `next()` を呼ぶ
//!    タスクを 1 つ起動する(`Queued` ならすでにタスクが動いているので何もしない)。
//! 2. タスクは `ms` 待ち → `next()` を呼ぶ。`Send` なら送り、`wait_ms > 0` ならその間待ってからまた `next()`、
//!    `wait_ms == 0` ならすぐ `next()`。`Idle` ならタスクを終える。

use std::collections::VecDeque;

/// [`PacedQueue::submit`] の結果。
#[derive(Debug, PartialEq, Eq)]
pub enum Submit<T> {
    /// いま送る。`arm_wait_ms` が `Some` なら、その時間待ってから [`PacedQueue::next`] を呼ぶタスクを起動する。
    SendNow { item: T, arm_wait_ms: Option<u32> },
    /// 待っている間なので、後ろに並べた(タスクはすでに動いている)。
    Queued,
}

/// [`PacedQueue::next`] の結果。
#[derive(Debug, PartialEq, Eq)]
pub enum Next<T> {
    /// `item` を送る。送ったあと `wait_ms` だけ待つ(0 ならすぐ次を取る)。
    Send { item: T, wait_ms: u32 },
    /// 並んでいるものは無い。待ちは終わり、いまは送り出しの途中ではない。
    Idle,
}

/// 送信の順序を保つキュー。
#[derive(Debug)]
pub struct PacedQueue<T> {
    waiting: VecDeque<(T, u32)>,
    /// タスクが動いていて、次の送信まで待っている(または待ちを終えて取りに来る)間。
    pacing: bool,
}

impl<T> Default for PacedQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PacedQueue<T> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            waiting: VecDeque::new(),
            pacing: false,
        }
    }

    /// タスクが動いている(待っている途中)か。`true` の間に提出された要素は、待ちが 0 でも後ろに並ぶ。
    #[must_use]
    pub const fn is_pacing(&self) -> bool {
        self.pacing
    }

    /// 要素を出す。`delay_after_ms` は、送ったあとに次の要素まで空ける時間。
    pub fn submit(&mut self, item: T, delay_after_ms: u32) -> Submit<T> {
        if self.pacing {
            self.waiting.push_back((item, delay_after_ms));
            return Submit::Queued;
        }
        if delay_after_ms == 0 {
            // 待ちが要らず、送り出しの途中でもない: そのまま送って終わり(タスクは起こさない)。
            return Submit::SendNow {
                item,
                arm_wait_ms: None,
            };
        }
        self.pacing = true;
        Submit::SendNow {
            item,
            arm_wait_ms: Some(delay_after_ms),
        }
    }

    /// 待ちが終わったあとに呼ぶ。次に送るものを返す。何も無ければ送り出しを終える。
    pub fn next(&mut self) -> Next<T> {
        match self.waiting.pop_front() {
            Some((item, wait_ms)) => Next::Send { item, wait_ms },
            None => {
                self.pacing = false;
                Next::Idle
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_delay_when_idle_sends_now_without_a_task() {
        let mut q = PacedQueue::new();
        assert_eq!(
            q.submit("a", 0),
            Submit::SendNow {
                item: "a",
                arm_wait_ms: None
            }
        );
        assert!(!q.is_pacing());
    }

    #[test]
    fn delay_when_idle_sends_now_and_arms_a_wait() {
        let mut q = PacedQueue::new();
        assert_eq!(
            q.submit("down", 10),
            Submit::SendNow {
                item: "down",
                arm_wait_ms: Some(10)
            }
        );
        assert!(q.is_pacing());
    }

    #[test]
    fn items_during_a_wait_queue_behind_even_with_zero_delay() {
        let mut q = PacedQueue::new();
        let _ = q.submit("down", 10);
        assert_eq!(q.submit("up", 0), Submit::Queued);
        assert_eq!(q.submit("other", 0), Submit::Queued);
        assert_eq!(
            q.next(),
            Next::Send {
                item: "up",
                wait_ms: 0
            }
        );
        assert_eq!(
            q.next(),
            Next::Send {
                item: "other",
                wait_ms: 0
            }
        );
        assert_eq!(q.next(), Next::Idle);
        assert!(!q.is_pacing());
    }

    #[test]
    fn order_is_the_submission_order_across_waits() {
        // k↓(10) k↑(0) a↓(10) a↑(0) を続けて出す。送信順は k↓ k↑ a↓ a↑ のまま、待ちは ↓ の後ろだけ。
        let mut q = PacedQueue::new();
        let mut sent = vec![];
        let mut waits = vec![];
        match q.submit("k↓", 10) {
            Submit::SendNow { item, arm_wait_ms } => {
                sent.push(item);
                waits.push(arm_wait_ms.unwrap());
            }
            Submit::Queued => unreachable!(),
        }
        for (item, d) in [("k↑", 0), ("a↓", 10), ("a↑", 0)] {
            assert_eq!(q.submit(item, d), Submit::Queued);
        }
        while let Next::Send { item, wait_ms } = q.next() {
            sent.push(item);
            if wait_ms > 0 {
                waits.push(wait_ms);
            }
        }
        assert_eq!(sent, ["k↓", "k↑", "a↓", "a↑"]);
        assert_eq!(waits, [10, 10]);
        assert!(!q.is_pacing());
    }

    #[test]
    fn after_idle_a_new_item_starts_fresh() {
        let mut q = PacedQueue::new();
        let _ = q.submit("x", 10);
        assert_eq!(q.next(), Next::Idle);
        assert_eq!(
            q.submit("y", 0),
            Submit::SendNow {
                item: "y",
                arm_wait_ms: None
            }
        );
    }
}
