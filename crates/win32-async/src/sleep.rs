#![allow(clippy::module_name_repetitions)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::ptr;
use std::task::{Context, Poll, Waker};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

thread_local! {
    /// 発火待ちタイマー ID → Waker のマップ（同一スレッド専用）
    // HashMap::new() は const fn ではないため const { } 形式は使えない
    static PENDING_WAKERS: RefCell<HashMap<usize, Waker>> = RefCell::new(HashMap::new());
}

/// SetTimer コールバック: タイマー ID に対応する Waker を起こす。
/// `DispatchMessageW` が WM_TIMER を dispatch したときに呼ばれる。
#[allow(unsafe_code)]
unsafe extern "system" fn timer_proc(_hwnd: HWND, _msg: u32, event_id: usize, _dw_time: u32) {
    PENDING_WAKERS.with(|wakers| {
        if let Some(waker) = wakers.borrow_mut().remove(&event_id) {
            waker.wake();
        }
    });
}

/// `ms` ミリ秒後に完了する非同期 sleep。
///
/// # 前提条件
/// awase メッセージループ（`WM_TIMER` `None` arm で `DispatchMessageW` を呼ぶ）が
/// 動いているスレッド上で await すること。
#[must_use]
pub const fn sleep_ms(ms: u32) -> SleepFuture {
    SleepFuture {
        ms,
        timer_id: None,
        done: false,
    }
}

/// [`sleep_ms`] が返す Future。
#[derive(Debug)]
pub struct SleepFuture {
    ms: u32,
    /// null-HWND タイマーの OS 割り当て ID（None = まだ SetTimer していない）
    timer_id: Option<usize>,
    done: bool,
}

impl Future for SleepFuture {
    type Output = ();

    #[allow(unsafe_code)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.done {
            return Poll::Ready(());
        }

        if let Some(timer_id) = self.timer_id {
            // 再 poll: タイマーがすでに発火済みか（PENDING_WAKERS から除去済み）を確認
            let fired = PENDING_WAKERS.with(|w| !w.borrow().contains_key(&timer_id));
            if fired {
                self.done = true;
                Poll::Ready(())
            } else {
                // Waker が変わっている可能性があるので更新
                PENDING_WAKERS.with(|w| {
                    w.borrow_mut().insert(timer_id, cx.waker().clone());
                });
                Poll::Pending
            }
        } else {
            // 初回 poll: SetTimer で OS タイマーを設定する
            // null HWND のとき nIDEvent は無視され、戻り値が割り当て ID になる
            let timer_id = unsafe { SetTimer(ptr::null_mut(), 0, self.ms, Some(timer_proc)) };
            if timer_id == 0 {
                log::warn!("[win32-async] SetTimer failed");
                self.done = true;
                return Poll::Ready(());
            }
            PENDING_WAKERS.with(|w| {
                w.borrow_mut().insert(timer_id, cx.waker().clone());
            });
            self.timer_id = Some(timer_id);
            Poll::Pending
        }
    }
}

impl Drop for SleepFuture {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        if let (Some(timer_id), false) = (self.timer_id, self.done) {
            unsafe {
                KillTimer(ptr::null_mut(), timer_id);
            }
            PENDING_WAKERS.with(|w| {
                w.borrow_mut().remove(&timer_id);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// 50ms sleep が概ね正しい時間で完了する
    #[test]
    fn sleep_50ms_completes_in_range() {
        let start = Instant::now();
        winmsg_executor::block_on(sleep_ms(50));
        let elapsed = start.elapsed().as_millis();
        assert!(
            elapsed >= 30,
            "sleep_ms(50) completed too early: {elapsed}ms"
        );
        assert!(elapsed < 500, "sleep_ms(50) took too long: {elapsed}ms");
    }

    /// 0ms sleep は（最小タイマー解像度ぶん待って）完了する
    #[test]
    fn sleep_zero_completes() {
        winmsg_executor::block_on(sleep_ms(0));
    }

    /// await せずに drop した SleepFuture は KillTimer を呼びパニックしない
    #[test]
    fn sleep_drop_before_fire_no_panic() {
        // 5秒タイマーを仕込んでから即 drop → KillTimer が呼ばれる
        let _ = sleep_ms(5_000);
    }

    /// 複数 sleep を連続で await した場合の合計時間が加算される
    #[test]
    fn sequential_sleeps_accumulate() {
        let start = Instant::now();
        winmsg_executor::block_on(async {
            sleep_ms(30).await;
            sleep_ms(30).await;
        });
        let elapsed = start.elapsed().as_millis();
        assert!(elapsed >= 40, "two 30ms sleeps too short: {elapsed}ms");
        assert!(elapsed < 500, "two 30ms sleeps too long: {elapsed}ms");
    }
}
