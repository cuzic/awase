use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::sleep::{sleep_ms, SleepFuture};

/// [`race_with_timeout`] が返す Future。
#[must_use = "futures do nothing unless awaited"]
pub struct RaceWithTimeout<F> {
    inner: Pin<Box<F>>,
    sleep: SleepFuture,
}

impl<F: Future> std::fmt::Debug for RaceWithTimeout<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RaceWithTimeout").finish_non_exhaustive()
    }
}

#[expect(clippy::future_not_send)]
impl<T, F: Future<Output = T>> Future for RaceWithTimeout<F> {
    type Output = Option<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        // 内側の Future が完了したら Some(value) を返す
        if let Poll::Ready(v) = self.inner.as_mut().poll(cx) {
            return Poll::Ready(Some(v));
        }
        // タイムアウトしたら None を返す（SleepFuture::drop が KillTimer を呼ぶ）
        if Pin::new(&mut self.sleep).poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    }
}

/// `fut` と `ms` ミリ秒のタイマーを競走させる。
///
/// - `fut` が先に完了すれば `Some(value)` を返す。
/// - タイムアウトが先に発火すれば `None` を返す。タイマーは [`SleepFuture::drop`] で
///   自動キャンセルされるため、呼び出し元でクリーンアップ不要。
///
/// # 注意
/// タイムアウト時に `fut` がワーカースレッドを起動している場合（[`offload`] 等）、
/// そのスレッドはバックグラウンドで実行継続する。[`offload_timeout`] を参照のこと。
///
/// [`offload`]: crate::offload
/// [`offload_timeout`]: crate::offload_timeout
#[must_use = "futures do nothing unless awaited"]
pub fn race_with_timeout<T, F: Future<Output = T>>(ms: u32, fut: F) -> RaceWithTimeout<F> {
    RaceWithTimeout {
        inner: Box::pin(fut),
        sleep: sleep_ms(ms),
    }
}
