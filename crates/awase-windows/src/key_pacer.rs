//! ペース付き送信(BUG-217)。注入キーの「↓から↑まで」に時間を空けないと取りこぼす窓(Zoom のチャット)に向けて、
//! **メッセージループを止めずに**、送信の間に待ちを置く。
//!
//! 以前は `thread::sleep` で待っていたので、1 つの音(2 キー)で約 20ms、メッセージループが止まった。
//! ここでは「送る → 待つ → 次を送る」を、純粋なキュー(`state::paced_queue`)と `win32_async::sleep_ms`(`SetTimer`)で進める。
//!
//! 順序の守り方:
//! - 待っている間(`pacing`)に来た送信は、`win32::send_input_safe` の口で [`divert`] がキューの後ろに回す。
//!   `send_input_safe` を直接呼ぶ他の経路(IME キー・Backspace など)も、追い越さない。
//! - 待っている間は `OutputActiveGuard` を持つ。物理キーはフックで保留され、キューが空になってから流れる
//!   (Enter が、まだ並んでいる文字を追い越さない)。
//!
//! 状態はスレッドごと(主スレッド用)。フックのスレッドが直接送るもの(修飾キーのマスク・カナリア)はキューを通らない。
//! 待ちの粒度は `SetTimer` なので実効は約 15.6ms 単位(指定の `ms` 以上になる。Zoom に必要なのは下限なので問題ない)。

use crate::state::paced_queue::{Next, PacedQueue, Submit};
use crate::tsf::probe_bridge::OutputActiveGuard;
use std::cell::{Cell, RefCell};
use windows::Win32::UI::Input::KeyboardAndMouse::INPUT;

thread_local! {
    static QUEUE: RefCell<PacedQueue<Vec<INPUT>>> = const { RefCell::new(PacedQueue::new()) };
    /// 待っている間、物理キーをフックで保留するためのガード(キューが空になったら手放す)。
    static GUARD: RefCell<Option<OutputActiveGuard>> = const { RefCell::new(None) };
    /// このモジュール自身が `send_input_safe` を呼んでいる間は、[`divert`] が素通しする(自分で自分を並べ直さない)。
    static SENDING: Cell<bool> = const { Cell::new(false) };
}

/// `inputs` を送り、そのあと `delay_after_ms` だけ次の送信を待たせる(0 なら待たせない)。
///
/// すでに待っている間なら、キューの後ろに並べるだけ(順序は提出の順)。
pub(crate) fn send_paced(inputs: Vec<INPUT>, delay_after_ms: u32) {
    let submit = QUEUE.with(|q| q.borrow_mut().submit(inputs, delay_after_ms));
    if let Submit::SendNow { item, arm_wait_ms } = submit {
        send_now(&item);
        if let Some(ms) = arm_wait_ms {
            start_pacing_task(ms);
        }
    }
}

/// 待っている間に `send_input_safe` が呼ばれたら、その送信をキューの後ろに回す。回したら `true`(呼び出し側は送らない)。
pub(crate) fn divert(inputs: &[INPUT]) -> bool {
    if SENDING.try_with(Cell::get).unwrap_or(true) {
        return false;
    }
    QUEUE
        .try_with(|q| {
            let mut q = q.borrow_mut();
            q.is_pacing() && matches!(q.submit(inputs.to_vec(), 0), Submit::Queued)
        })
        .unwrap_or(false)
}

fn send_now(inputs: &[INPUT]) {
    let _ = SENDING.try_with(|s| s.set(true));
    let _ = crate::win32::send_input_safe(inputs);
    let _ = SENDING.try_with(|s| s.set(false));
}

/// 最初の `first_wait_ms` を待ってから、キューが空になるまで「次を送る → 待つ」を繰り返すタスクを起こす。
fn start_pacing_task(first_wait_ms: u32) {
    GUARD.with(|g| *g.borrow_mut() = Some(OutputActiveGuard::begin()));
    win32_async::spawn_local(async move {
        let mut wait_ms = first_wait_ms;
        loop {
            win32_async::sleep_ms(wait_ms).await;
            loop {
                let next = QUEUE.with(|q| q.borrow_mut().next());
                match next {
                    Next::Send { item, wait_ms: w } => {
                        send_now(&item);
                        if w > 0 {
                            wait_ms = w;
                            break;
                        }
                    }
                    Next::Idle => {
                        // 手放すと、保留していた物理キーの再配送が始まる。
                        let guard = GUARD.with(|g| g.borrow_mut().take());
                        drop(guard);
                        return;
                    }
                }
            }
        }
    });
}
