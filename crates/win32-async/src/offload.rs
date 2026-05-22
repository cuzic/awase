//! ブロッキング処理をワーカースレッドで実行し、正規の Future として待つ。
//!
//! `winmsg_executor` の Waker は `PostMessageA` ベースでスレッドセーフなため、
//! ワーカースレッドから `waker.wake()` を呼ぶとメインスレッドが即座に再 poll される。
//! ポーリングによる遅延は発生しない。

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

struct OffloadState<T> {
    result: Mutex<Option<T>>,
    waker: Mutex<Option<Waker>>,
    done: AtomicBool,
}

/// [`offload`] が返す Future。
///
/// 初回 poll でワーカースレッドを起動し、完了時に Waker 経由でメインスレッドを起こす。
pub struct OffloadFuture<T> {
    state: Arc<OffloadState<T>>,
    spawned: bool,
    f: Option<Box<dyn FnOnce() -> T + Send + 'static>>,
}

impl<T> std::fmt::Debug for OffloadFuture<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OffloadFuture")
            .field("spawned", &self.spawned)
            .field("done", &self.state.done.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

#[allow(clippy::future_not_send)]
impl<T: Send + 'static> Future for OffloadFuture<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let this = self.get_mut();

        if this.state.done.load(Ordering::Acquire) {
            let value = this.state.result.lock().unwrap().take().unwrap();
            return Poll::Ready(value);
        }

        // Waker を保存（再 poll のたびに更新して古い Waker の残留を防ぐ）
        *this.state.waker.lock().unwrap() = Some(cx.waker().clone());

        // 初回 poll でワーカースレッドを起動
        if !this.spawned {
            this.spawned = true;
            let f = this.f.take().expect("invariant: f is present before first spawn");
            let state = Arc::clone(&this.state);
            std::thread::spawn(move || {
                let result = f();
                *state.result.lock().unwrap() = Some(result);
                state.done.store(true, Ordering::Release);
                // PostMessageA でメインスレッドを即起床（winmsg_executor Waker はスレッドセーフ）
                if let Some(waker) = state.waker.lock().unwrap().take() {
                    waker.wake();
                }
            });
        }

        Poll::Pending
    }
}

/// ブロッキング処理をワーカースレッドで実行する Future を返す。
///
/// `winmsg_executor::spawn_local` 内で await すること。
/// ワーカー完了時に Waker 経由でメインスレッドを即起床するため、
/// ポーリングによる遅延は発生しない。
#[must_use]
pub fn offload<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> OffloadFuture<T> {
    OffloadFuture {
        state: Arc::new(OffloadState {
            result: Mutex::new(None),
            waker: Mutex::new(None),
            done: AtomicBool::new(false),
        }),
        spawned: false,
        f: Some(Box::new(f)),
    }
}

/// タイムアウト付きでブロッキング処理をワーカースレッドで実行する Future を返す。
///
/// `ms` ミリ秒以内に完了すれば `Some(T)`、タイムアウトすれば `None` を返す。
/// タイムアウト時はワーカースレッドがバックグラウンドで実行継続する点に注意。
/// ワーカーが完了した時点で Waker 経由の wake が空振りするだけなので
/// メモリリークは発生しないが、スレッドは完了まで保持される。
#[must_use]
pub fn offload_timeout<T: Send + 'static>(
    ms: u32,
    f: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = Option<T>> {
    crate::race_timeout::race_with_timeout(ms, offload(f))
}
