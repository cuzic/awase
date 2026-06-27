//! タイマー駆動コルーチン基盤。
//!
//! `StepCoro<I, Y>` は Rust の async/await 脱糖を利用した最小コルーチン実装。
//! unsafe・nightly・外部クレートなし（std のみ）。
//!
//! ## 動作原理
//!
//! - `step(input)` → future を 1 ステップ poll → `CoroStep::Yielded(output)` を返す
//! - `yield_step(ch, output).await` → output を書き → Pending → 次 poll で input を読む
//! - `NoopWaker` を使う（外部イベントドリブンではなくタイマー駆動なので wake 不要）
//!
//! ## 最初の step について
//!
//! `step(input_1)` は future を最初の yield 点まで進め `vec![]` を返すが、
//! `input_1` は消費されない（次の `step(input_2)` で最初の yield 点が `input_2` を読む）。
//! 10ms タイマー駆動のため 1 ティック分のロスは動作に影響しない。

use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

// ── Channel ──────────────────────────────────────────────────────────────────

/// コルーチン駆動側とコルーチン本体の間でデータを受け渡す単一スロットチャネル。
pub(crate) struct Channel<I, Y> {
    pub(crate) input: Cell<Option<I>>,
    pub(crate) output: Cell<Option<Y>>,
}

// ── SuspendOnce ───────────────────────────────────────────────────────────────

/// 最初の poll で output を書いて Pending、次の poll で input を読んで Ready。
struct SuspendOnce<I, Y> {
    channel: Rc<Channel<I, Y>>,
    output: Option<Y>,
}

impl<I, Y> Future for SuspendOnce<I, Y> {
    type Output = I;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<I> {
        // Safety: フィールドを move しないため unpin は安全。
        let this = unsafe { self.get_unchecked_mut() };
        if let Some(y) = this.output.take() {
            // 1 回目 poll: output を書いて中断
            this.channel.output.set(Some(y));
            Poll::Pending
        } else {
            // 2 回目 poll: input を読んで再開
            Poll::Ready(this.channel.input.take().expect("StepCoro: input が設定されていません"))
        }
    }
}

// ── yield_step ────────────────────────────────────────────────────────────────

/// コルーチン本体から呼ぶ yield 点。`output` を外へ渡して中断し、再開時に `input` を受け取る。
pub(crate) async fn yield_step<I, Y>(channel: Rc<Channel<I, Y>>, output: Y) -> I {
    SuspendOnce { channel, output: Some(output) }.await
}

// ── NoopWaker ─────────────────────────────────────────────────────────────────

/// タイマー駆動のため wake 通知は不要。RawWaker の vtable は全 no-op。
fn noop_waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |p| RawWaker::new(p, &VTABLE), // clone
        |_| {},                          // wake
        |_| {},                          // wake_by_ref
        |_| {},                          // drop
    );
    // Safety: vtable の全操作が no-op のためポインタ値は任意で良い。
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

// ── CoroStep ──────────────────────────────────────────────────────────────────

/// `StepCoro::step` の返り値。
pub(crate) enum CoroStep<Y> {
    /// コルーチンが yield した。`Y` は今ステップの出力。
    Yielded(Y),
    /// コルーチンが return した（完了）。
    Complete,
}

// ── StepCoro ──────────────────────────────────────────────────────────────────

/// タイマー駆動の 1 ステップコルーチン。
///
/// `step(input)` を 1 回呼ぶごとに future を次の yield 点まで進める。
/// コルーチン本体は `yield_step(ch, output).await` で出力を書き、再開時に入力を受け取る。
pub(crate) struct StepCoro<I: 'static, Y: 'static> {
    channel: Rc<Channel<I, Y>>,
    future: Pin<Box<dyn Future<Output = ()>>>,
}

impl<I: 'static, Y: 'static> StepCoro<I, Y> {
    /// コルーチンを生成する。
    ///
    /// `fut_fn` は `Rc<Channel<I, Y>>` を受け取って async ブロックを返すクロージャ。
    pub(crate) fn new<Fut>(fut_fn: impl FnOnce(Rc<Channel<I, Y>>) -> Fut) -> Self
    where
        Fut: Future<Output = ()> + 'static,
    {
        let channel = Rc::new(Channel {
            input: Cell::new(None),
            output: Cell::new(None),
        });
        let fut = fut_fn(Rc::clone(&channel));
        Self {
            channel,
            future: Box::pin(fut),
        }
    }

    /// コルーチンを 1 ステップ進める。
    ///
    /// `input` を channel に書いてから future を poll する。
    /// - `Pending` → channel から output を取り出して `Yielded(output)` を返す。
    /// - `Ready(())` → `Complete` を返す。
    pub(crate) fn step(&mut self, input: I) -> CoroStep<Y> {
        self.channel.input.set(Some(input));
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        match self.future.as_mut().poll(&mut cx) {
            Poll::Pending => {
                let output = self
                    .channel
                    .output
                    .take()
                    .expect("StepCoro: コルーチンが output を設定せずに Pending を返しました");
                CoroStep::Yielded(output)
            }
            Poll::Ready(()) => CoroStep::Complete,
        }
    }
}
