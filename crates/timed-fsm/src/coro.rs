//! タイマー駆動コルーチン基盤。
//!
//! [`StepCoro<I, Y>`] は Rust の async/await を利用した最小コルーチン実装。
//! `unsafe`・nightly・外部クレートなし（`std` のみ、MSRV 1.85）。
//!
//! ## [`TimedStateMachine`] との使い分け
//!
//! | 場面 | 向いているモデル |
//! |------|-----------------|
//! | どの状態でも同じイベントセットを受け付ける | [`TimedStateMachine`] |
//! | フェーズが直線的に進む多段ワークフロー | [`StepCoro`] |
//!
//! `StepCoro` はフェーズを async 関数の制御フローで表現するため、
//! 明示的な状態 enum と遷移テーブルが不要になる。
//!
//! ## 動作原理
//!
//! - `step(input)` → future を次の yield 点まで poll → [`CoroStep`] を返す（[`StepCoro::step`] 参照）
//! - `yield_step(ch, output).await` → `output` を外へ渡して中断し、再開時に `input` を受け取る（[`yield_step`] 参照）
//! - [`Waker::noop`] を使う（タイマー駆動のため wake 通知は不要）
//!
//! ## 最初の step について
//!
//! `step(input_1)` は future を最初の yield 点まで進めるが、
//! `input_1` 自体は消費されない（次の `step(input_2)` で最初の yield 点が `input_2` を読む）。
//! タイマー駆動では 1 ティック分のロスは動作に影響しない。
//!
//! ## 使用例
//!
//! ```rust
//! use std::rc::Rc;
//! use timed_fsm::coro::{Channel, CoroStep, StepCoro, yield_step};
//!
//! async fn phase_body(ch: Rc<Channel<u32, String>>) {
//!     let n = yield_step(ch.clone(), "phase1".to_owned()).await;
//!     let _ = yield_step(ch, format!("phase2: got {n}")).await;
//! }
//!
//! let mut coro: StepCoro<u32, String> = StepCoro::new(phase_body);
//!
//! // 最初の step: future を 1 つ目の yield 点まで進める（input は未消費）
//! let CoroStep::Yielded(out) = coro.step(0) else { panic!() };
//! assert_eq!(out, "phase1");
//!
//! // 2 回目: "phase1" yield が input=42 を受け取り、"phase2: got 42" を yield する
//! let CoroStep::Yielded(out) = coro.step(42) else { panic!() };
//! assert_eq!(out, "phase2: got 42");
//!
//! // 3 回目: コルーチン本体が return → Complete
//! let CoroStep::Complete = coro.step(0) else { panic!() };
//! ```
//!
//! [`TimedStateMachine`]: crate::TimedStateMachine
//! [`Waker::noop`]: std::task::Waker::noop

use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

// ── Channel ───────────────────────────────────────────────────────────────────

/// コルーチン駆動側とコルーチン本体の間でデータを受け渡す単一スロットチャネル。
///
/// コルーチン本体は `Rc<Channel<I, Y>>` を受け取り、[`yield_step`] に渡す。
/// フィールドは非公開で、[`StepCoro::step`] と [`yield_step`] がすべて管理する。
pub struct Channel<I, Y> {
    input: Cell<Option<I>>,
    output: Cell<Option<Y>>,
}

// ── SuspendOnce ───────────────────────────────────────────────────────────────

/// 最初の poll で output を書いて `Pending`、次の poll で input を読んで `Ready`。
struct SuspendOnce<I, Y> {
    channel: Rc<Channel<I, Y>>,
    output: Option<Y>,
}

// SuspendOnce に自己参照フィールドはなく、移動は常に安全。
impl<I, Y> Unpin for SuspendOnce<I, Y> {}

impl<I, Y> Future for SuspendOnce<I, Y> {
    type Output = I;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<I> {
        let this = self.get_mut(); // Unpin なので safe
        if let Some(y) = this.output.take() {
            // 1 回目 poll: output を書いて中断
            this.channel.output.set(Some(y));
            Poll::Pending
        } else {
            // 2 回目 poll: input を読んで再開
            Poll::Ready(
                this.channel
                    .input
                    .take()
                    .expect("StepCoro: input が設定されていません"),
            )
        }
    }
}

// ── yield_step ────────────────────────────────────────────────────────────────

/// コルーチン本体から呼ぶ yield 点。
///
/// `output` を駆動側へ渡して中断し、再開時に次の `input` を受け取る。
///
/// `Rc` を使うため生成される future は `!Send`。これはタイマー駆動の単一スレッド設計による意図的な制約。
#[expect(clippy::future_not_send)]
pub async fn yield_step<I, Y>(channel: Rc<Channel<I, Y>>, output: Y) -> I {
    SuspendOnce { channel, output: Some(output) }.await
}

// ── CoroStep ──────────────────────────────────────────────────────────────────

/// [`StepCoro::step`] の返り値。
#[must_use]
pub enum CoroStep<Y> {
    /// コルーチンが yield した。`Y` は今ステップの出力。
    Yielded(Y),
    /// コルーチンが return した（完了）。
    Complete,
}

// ── StepCoro ──────────────────────────────────────────────────────────────────

/// タイマー駆動の 1 ステップコルーチン。
///
/// [`StepCoro::step`] を 1 回呼ぶごとに future を次の yield 点まで進める。
/// コルーチン本体は [`yield_step`] で出力を書き、再開時に入力を受け取る。
pub struct StepCoro<I: 'static, Y: 'static> {
    channel: Rc<Channel<I, Y>>,
    future: Pin<Box<dyn Future<Output = ()>>>,
}

impl<I: 'static, Y: 'static> StepCoro<I, Y> {
    /// コルーチンを生成する。
    ///
    /// `fut_fn` は `Rc<Channel<I, Y>>` を受け取って async 関数（またはブロック）を返すクロージャ。
    /// Rust 1.85 以降の async クロージャ（`async move |ch| { ... }`）を直接渡すこともできる。
    pub fn new<Fut>(fut_fn: impl FnOnce(Rc<Channel<I, Y>>) -> Fut) -> Self
    where
        Fut: Future<Output = ()> + 'static,
    {
        let channel = Rc::new(Channel {
            input: Cell::new(None),
            output: Cell::new(None),
        });
        Self {
            future: Box::pin(fut_fn(Rc::clone(&channel))),
            channel,
        }
    }

    /// コルーチンを 1 ステップ進める。
    ///
    /// `input` を channel に書いてから future を poll する。
    /// - `Pending` → channel から output を取り出して [`CoroStep::Yielded`] を返す。
    /// - `Ready(())` → [`CoroStep::Complete`] を返す。
    ///
    /// # Panics
    ///
    /// コルーチン本体が [`yield_step`] を呼ばずに `Poll::Pending` を返した場合（内部実装エラー）。
    pub fn step(&mut self, input: I) -> CoroStep<Y> {
        self.channel.input.set(Some(input));
        let mut cx = Context::from_waker(Waker::noop());
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
