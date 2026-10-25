//! ペース付き送信の状態機械(BUG-217)。**メッセージループを止めずに**、送信の間に待ちを置くための純粋な部品。
//!
//! Zoom のチャット窓は、注入キーの「↓から↑まで」が短いと一部を取りこぼす。待ちを `thread::sleep` で置くと
//! メッセージループが止まるので、「送る → 待つ → 次を送る」を、この状態機械と(殻の)非同期タイマーで進める。
//!
//! # 状態
//! ```text
//!            submit(待ち 0)                       submit(待ち > 0)
//!   ┌──────── Idle ────────┐                ┌────────────────────────┐
//!   │ (そのまま送って終わり) │                ▼                        │
//!   └──────────────────────┘          Pacing { waiting, last_progress, ticket }
//!                                       │  ▲ submit: waiting の後ろに並べる(待ち 0 でも)
//!                    wait_elapsed(ticket)│  │ wait_elapsed(ticket): 次を取り出して送る
//!                      waiting が空 ▼    │  │
//!                                     Idle   進まないまま `stall_ms` を超えたら submit が復帰(Recovered)
//! ```
//! 送信の順序は、提出の順のまま変わらない。
//!
//! # 殻との約束(タスクと札)
//! - 待ちが要る送信(`arm` が `Some`)のとき、殻は `arm.wait_ms` 待ってから [`PacedQueue::wait_elapsed`] を呼ぶタスクを
//!   **1 つ**起こす。`Queued` ならタスクはすでに動いている。
//! - 札(`Ticket`)は、タスクが今の `Pacing` の持ち主かを確かめる。復帰(`Recovered`)で作り直した後に、古いタスクが
//!   遅れて起きても、札が違うので何もしない(`Superseded`)。
//! - 復帰: タイマーが動かない等でタスクが進まなくなっても、次の `submit` が、溜まった分を流して `Idle` に戻す。

use std::collections::VecDeque;

/// 待ちの持ち主を表す札。[`PacedQueue::submit`] が新しい `Pacing` を始めるたびに 1 つ進む。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket(u64);

/// 「`wait_ms` 待ってから、この札で [`PacedQueue::wait_elapsed`] を呼べ」という指示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arm {
    pub ticket: Ticket,
    pub wait_ms: u32,
}

/// [`PacedQueue::submit`] の結果。
#[derive(Debug, PartialEq, Eq)]
pub enum Submit<T> {
    /// いま送る。`arm` が `Some` なら、タスクを起こす。
    SendNow { item: T, arm: Option<Arm> },
    /// 待っている間なので、後ろに並べた(タスクはすでに動いている)。
    Queued,
    /// 待ちが止まっていた(`stall_ms` 進まなかった)ので復帰する。`flushed`(溜まっていた分、順序どおり)を送り、
    /// そのあとに `item` を送る。`arm` が `Some` ならタスクを起こす。`None` なら、殻は待ちの後始末(ガードの解放)をする。
    Recovered {
        flushed: Vec<T>,
        item: T,
        arm: Option<Arm>,
    },
}

/// [`PacedQueue::wait_elapsed`] の結果。
#[derive(Debug, PartialEq, Eq)]
pub enum Next<T> {
    /// `item` を送る。送ったあと `wait_ms` だけ待つ(0 ならすぐ次を取る)。
    Send { item: T, wait_ms: u32 },
    /// 並んでいるものは無い。いまは送り出しの途中ではない(`Idle`)。殻は待ちの後始末をする。
    Idle,
    /// このタスクはもう持ち主ではない(復帰で作り直された、または `Idle`)。何もせずに終える。
    Superseded,
}

#[derive(Debug)]
enum State<T> {
    Idle,
    Pacing {
        waiting: VecDeque<(T, u32)>,
        /// 最後に進んだ時刻(待ちを始めた、または `wait_elapsed` が呼ばれた)。
        last_progress_ms: u64,
        ticket: Ticket,
    },
}

/// 送信の順序を保つ、ペース付き送信の状態機械。
#[derive(Debug)]
pub struct PacedQueue<T> {
    state: State<T>,
    next_ticket: u64,
    /// `Pacing` のまま、これより長く進まなければ止まったと見なす (ms)。
    stall_ms: u64,
}

impl<T> PacedQueue<T> {
    #[must_use]
    pub const fn new(stall_ms: u64) -> Self {
        Self {
            state: State::Idle,
            next_ticket: 0,
            stall_ms,
        }
    }

    /// 待っている最中(`Pacing`)か。`true` の間に提出された要素は、待ちが 0 でも後ろに並ぶ。
    #[must_use]
    pub const fn is_pacing(&self) -> bool {
        matches!(self.state, State::Pacing { .. })
    }

    /// 要素を出す。`delay_after_ms` は、送ったあとに次の要素まで空ける時間。`now_ms` は単調な時計。
    pub fn submit(&mut self, item: T, delay_after_ms: u32, now_ms: u64) -> Submit<T> {
        if let State::Pacing {
            waiting,
            last_progress_ms,
            ..
        } = &mut self.state
        {
            if now_ms.saturating_sub(*last_progress_ms) <= self.stall_ms {
                waiting.push_back((item, delay_after_ms));
                return Submit::Queued;
            }
            // 止まっていた: 溜まった分を取り出して Idle に戻し、この要素から始め直す。
            let flushed = waiting.drain(..).map(|(queued, _)| queued).collect();
            self.state = State::Idle;
            let (item, arm) = self.start(item, delay_after_ms, now_ms);
            return Submit::Recovered { flushed, item, arm };
        }
        let (item, arm) = self.start(item, delay_after_ms, now_ms);
        Submit::SendNow { item, arm }
    }

    /// `Idle` からの出発。待ちが要るなら `Pacing` にして札を渡す。
    fn start(&mut self, item: T, delay_after_ms: u32, now_ms: u64) -> (T, Option<Arm>) {
        if delay_after_ms == 0 {
            // 待ちが要らず、送り出しの途中でもない: そのまま送って終わり(タスクは起こさない)。
            return (item, None);
        }
        let ticket = Ticket(self.next_ticket);
        self.next_ticket += 1;
        self.state = State::Pacing {
            waiting: VecDeque::new(),
            last_progress_ms: now_ms,
            ticket,
        };
        (
            item,
            Some(Arm {
                ticket,
                wait_ms: delay_after_ms,
            }),
        )
    }

    /// 待ちが終わったあとに、タスクが呼ぶ。次に送るものを返す。何も無ければ `Idle` に戻る。
    pub fn wait_elapsed(&mut self, ticket: Ticket, now_ms: u64) -> Next<T> {
        let State::Pacing {
            waiting,
            last_progress_ms,
            ticket: current,
        } = &mut self.state
        else {
            return Next::Superseded;
        };
        if *current != ticket {
            return Next::Superseded;
        }
        *last_progress_ms = now_ms;
        if let Some((item, wait_ms)) = waiting.pop_front() {
            return Next::Send { item, wait_ms };
        }
        self.state = State::Idle;
        Next::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STALL: u64 = 1_000;

    fn arm_of<T>(s: Submit<T>) -> (T, Arm) {
        match s {
            Submit::SendNow {
                item,
                arm: Some(arm),
            } => (item, arm),
            other => panic!(
                "SendNow(arm あり)のはず: {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn zero_delay_when_idle_sends_now_without_a_task() {
        let mut q = PacedQueue::new(STALL);
        assert_eq!(
            q.submit("a", 0, 0),
            Submit::SendNow {
                item: "a",
                arm: None
            }
        );
        assert!(!q.is_pacing());
    }

    #[test]
    fn delay_when_idle_sends_now_and_arms_a_wait() {
        let mut q = PacedQueue::new(STALL);
        let (item, arm) = arm_of(q.submit("down", 10, 0));
        assert_eq!(item, "down");
        assert_eq!(arm.wait_ms, 10);
        assert!(q.is_pacing());
    }

    #[test]
    fn items_during_a_wait_queue_behind_even_with_zero_delay() {
        let mut q = PacedQueue::new(STALL);
        let (_, arm) = arm_of(q.submit("down", 10, 0));
        assert_eq!(q.submit("up", 0, 1), Submit::Queued);
        assert_eq!(q.submit("other", 0, 2), Submit::Queued);
        assert_eq!(
            q.wait_elapsed(arm.ticket, 10),
            Next::Send {
                item: "up",
                wait_ms: 0
            }
        );
        assert_eq!(
            q.wait_elapsed(arm.ticket, 10),
            Next::Send {
                item: "other",
                wait_ms: 0
            }
        );
        assert_eq!(q.wait_elapsed(arm.ticket, 10), Next::Idle);
        assert!(!q.is_pacing());
    }

    #[test]
    fn order_is_the_submission_order_across_waits() {
        // k↓(10) k↑(0) a↓(10) a↑(0) を続けて出す。送信順は k↓ k↑ a↓ a↑ のまま、待ちは ↓ の後ろだけ。
        let mut q = PacedQueue::new(STALL);
        let (first, arm) = arm_of(q.submit("k↓", 10, 0));
        let mut sent = vec![first];
        let mut waits = vec![arm.wait_ms];
        for (item, d) in [("k↑", 0), ("a↓", 10), ("a↑", 0)] {
            assert_eq!(q.submit(item, d, 0), Submit::Queued);
        }
        while let Next::Send { item, wait_ms } = q.wait_elapsed(arm.ticket, 10) {
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
        let mut q = PacedQueue::new(STALL);
        let (_, arm) = arm_of(q.submit("x", 10, 0));
        assert_eq!(q.wait_elapsed(arm.ticket, 10), Next::Idle);
        assert_eq!(
            q.submit("y", 0, 11),
            Submit::SendNow {
                item: "y",
                arm: None
            }
        );
    }

    #[test]
    fn a_stalled_wait_recovers_and_flushes_in_order() {
        // タイマーが動かず、タスクが進まない。STALL を超えたあとの submit が、溜まった分を順序どおり流して復帰する。
        let mut q = PacedQueue::new(STALL);
        let (_, old) = arm_of(q.submit("down", 10, 0));
        assert_eq!(q.submit("up", 0, 5), Submit::Queued);
        assert_eq!(q.submit("next", 0, 6), Submit::Queued);
        match q.submit("late", 10, 6 + STALL + 1) {
            Submit::Recovered {
                flushed,
                item,
                arm: Some(new),
            } => {
                assert_eq!(flushed, ["up", "next"]);
                assert_eq!(item, "late");
                assert_ne!(new.ticket, old.ticket);
            }
            _ => panic!("Recovered(arm あり)のはず"),
        }
        assert!(q.is_pacing());
    }

    #[test]
    fn recovery_with_zero_delay_ends_pacing() {
        let mut q = PacedQueue::new(STALL);
        let _ = q.submit("down", 10, 0);
        match q.submit("late", 0, STALL + 1) {
            Submit::Recovered {
                flushed,
                item,
                arm: None,
            } => {
                assert!(flushed.is_empty());
                assert_eq!(item, "late");
            }
            _ => panic!("Recovered(arm なし)のはず"),
        }
        assert!(!q.is_pacing());
    }

    #[test]
    fn a_late_old_task_is_superseded_after_recovery() {
        let mut q = PacedQueue::new(STALL);
        let (_, old) = arm_of(q.submit("down", 10, 0));
        let Submit::Recovered { arm: Some(new), .. } = q.submit("late", 10, STALL + 1) else {
            panic!("Recovered(arm あり)のはず")
        };
        let _ = q.submit("more", 0, STALL + 2);
        // 古いタスクが遅れて起きても、何も取り出さない。
        assert_eq!(q.wait_elapsed(old.ticket, STALL + 3), Next::Superseded);
        // 新しいタスクは、溜まった分を取り出せる。
        assert_eq!(
            q.wait_elapsed(new.ticket, STALL + 3),
            Next::Send {
                item: "more",
                wait_ms: 0
            }
        );
    }

    #[test]
    fn progress_postpones_the_stall() {
        // タスクが進んでいる間は、長く続いても止まったと見なさない。
        let mut q = PacedQueue::new(STALL);
        let (_, arm) = arm_of(q.submit("a", 10, 0));
        assert_eq!(q.submit("b", 10, 900), Submit::Queued);
        let _ = q.wait_elapsed(arm.ticket, 1_500); // b を取り出す(進んだ)
        assert_eq!(q.submit("c", 0, 2_400), Submit::Queued); // 最後に進んでから 900ms: まだ止まっていない
    }

    #[test]
    fn a_task_after_idle_is_superseded() {
        let mut q = PacedQueue::new(STALL);
        let (_, arm) = arm_of(q.submit("x", 10, 0));
        assert_eq!(q.wait_elapsed(arm.ticket, 10), Next::Idle);
        assert_eq!(q.wait_elapsed(arm.ticket, 11), Next::Superseded);
    }
}
