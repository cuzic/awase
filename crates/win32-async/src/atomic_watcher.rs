use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll};

/// `AtomicU32` の値が `baseline` から変化するまで await する Future。
///
/// # 使い方
/// ```ignore
/// let baseline = OBS_NAMECHANGE_SEQ.load(Ordering::Acquire);
/// // …warmup F2 送信…
/// let new_seq = AtomicWatcher::new(&OBS_NAMECHANGE_SEQ, baseline).await;
/// ```
///
/// # 注意
/// Waker の登録は `poll` 時のみ行われる。値の変化を検出するには、変化側のコードが
/// [`notify_all`] を呼ぶ必要がある。awase では `observation_event_proc` 内で呼ぶ。
#[derive(Debug)]
pub struct AtomicWatcher<'a> {
    atom: &'a AtomicU32,
    baseline: u32,
}

impl<'a> AtomicWatcher<'a> {
    pub const fn new(atom: &'a AtomicU32, baseline: u32) -> Self {
        Self { atom, baseline }
    }
}

impl Future for AtomicWatcher<'_> {
    type Output = u32;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u32> {
        let current = self.atom.load(Ordering::Acquire);
        if current == self.baseline {
            GLOBAL_WAKERS.with(|wakers| {
                wakers.borrow_mut().push(cx.waker().clone());
            });
            Poll::Pending
        } else {
            Poll::Ready(current)
        }
    }
}

thread_local! {
    static GLOBAL_WAKERS: std::cell::RefCell<Vec<std::task::Waker>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// すべての登録済み Waker を起こす。
///
/// `AtomicU32` を更新したコード（WinEventProc など）が呼ぶこと。
/// メインスレッド上で呼ぶことが前提。
pub fn notify_all() {
    GLOBAL_WAKERS.with(|wakers| {
        for waker in wakers.borrow_mut().drain(..) {
            waker.wake();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::Ordering, Arc};
    use std::sync::atomic::AtomicU32;

    /// すでに値が変化していれば即 Poll::Ready を返す
    #[test]
    fn watcher_returns_immediately_when_changed() {
        let atom = AtomicU32::new(5);
        let result = winmsg_executor::block_on(AtomicWatcher::new(&atom, 0));
        assert_eq!(result, 5);
    }

    /// baseline と同じ値のとき、spawn_local タスクが notify_all() を呼んだら起きる
    #[test]
    fn watcher_wakes_on_notify_all() {
        let shared = Arc::new(AtomicU32::new(0));
        let shared2 = Arc::clone(&shared);

        winmsg_executor::block_on(async {
            // メッセージループ上で 20ms 後に atomic を更新して notify
            winmsg_executor::spawn_local(async move {
                crate::sleep_ms(20).await;
                shared2.store(1, Ordering::Release);
                notify_all();
            });

            // 変化を待つ（notify_all() で即 wake される）
            let result = AtomicWatcher::new(&*shared, 0).await;
            assert_eq!(result, 1);
        });
    }

    /// notify_all() なしでも、poll のタイミングで値が変わっていれば Ready を返す
    #[test]
    fn watcher_returns_ready_if_changed_before_poll() {
        let atom = Arc::new(AtomicU32::new(0));
        let atom2 = Arc::clone(&atom);

        winmsg_executor::block_on(async {
            // 30ms 後に atomic を変化（notify_all なし）
            winmsg_executor::spawn_local(async move {
                crate::sleep_ms(30).await;
                atom2.store(7, Ordering::Release);
                // notify_all() なし。AtomicWatcher は park したまま。
            });

            // 80ms 後に AtomicWatcher を作成して poll → 値は既に 7 → 即 Ready
            crate::sleep_ms(80).await;
            let result = AtomicWatcher::new(&*atom, 0).await;
            assert_eq!(result, 7, "value changed before poll → immediate Ready");
        });
    }
}
