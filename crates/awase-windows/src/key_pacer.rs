//! ペース付き送信(BUG-217)の殻。注入キーの「↓から↑まで」に時間を空けないと取りこぼす窓(Zoom のチャット)に向けて、
//! **メッセージループを止めずに**、送信の間に待ちを置く。
//!
//! 判断は純粋な状態機械 `state::paced_queue::PacedQueue` にあり、ここはそれに「実際の送信」と「待ち」を配線するだけ:
//!
//! ```text
//!   send_paced ─ submit ─▶ PacedQueue ─ SendNow ──▶ send_direct (SendInput)  ┐ arm があればタスクを 1 つ起こす
//!                              │  └──── Queued ──▶ (後ろに並べる)             │
//!                              │                                              ▼
//!   win32::send_input_safe ─ divert ─▶ 待っている間だけ、他の送信も後ろに並べる   run_pacing: sleep_ms ─▶ advance
//!                                                                              advance: wait_elapsed を回して
//!                                                                              次の待ちまで(または空になるまで)送る
//! ```
//!
//! 順序の守り方:
//! - 待っている間(`Pacing`)に来た送信は、`win32::send_input_safe` の口で [`divert`] がキューの後ろに回す。
//!   `send_input_safe` を直接呼ぶ他の経路(IME キー・Backspace など)も、追い越さない。
//! - 待っている間は `OutputActiveGuard` を持つ。物理キーはフックで保留され、待ちが終わってから流れる
//!   (Enter が、まだ並んでいる文字を追い越さない)。
//!
//! 安全弁: タイマーが動かない等でタスクが進まなくなっても、[`STALL_RECOVERY_MS`] 後の次の送信が、溜まった分を
//! 流して `Idle` に戻し、ガードも作り直す(物理キーが保留されたままにならない)。
//!
//! 状態はスレッドごと(主スレッド用)。フックのスレッドが直接送るもの(修飾キーのマスク・カナリア)はキューを通らない。
//! 待ちの粒度は `SetTimer` なので実効は約 15.6ms 単位(指定の `ms` 以上になる。Zoom に必要なのは下限なので問題ない)。

use crate::state::paced_queue::{Arm, Next, PacedQueue, Submit, Ticket};
use crate::tsf::probe_bridge::OutputActiveGuard;
use std::cell::{Cell, RefCell};
use windows::Win32::UI::Input::KeyboardAndMouse::INPUT;

/// 待ちが進まないまま、これだけ経ったら止まったと見なして復帰する (ms)。
///
/// **測定値ではなく、設計上の安全弁**。通常の待ちは [`crate::tuning::ZOOM_KEY_HOLD_MS`](約 10〜16ms)で、キューが
/// 進むたびに時計が更新されるので、1 秒進まないのはタイマーが死んでいるときだけ。短くしすぎると、重い処理で
/// メッセージループが遅れただけの正常な待ちを壊すので、通常の待ちの 2 桁上に置いた。
const STALL_RECOVERY_MS: u64 = 1_000;

/// 主スレッドのペース付き送信の状態。
struct Pacer {
    queue: RefCell<PacedQueue<Vec<INPUT>>>,
    /// 待っている間(`Pacing`)だけ持つ。物理キーをフックで保留する。
    gate: RefCell<Option<OutputActiveGuard>>,
    /// このモジュール自身が `send_input_safe` を呼んでいる間 `true`(その送信を [`divert`] が並べ直さない)。
    direct: Cell<bool>,
}

thread_local! {
    static PACER: Pacer = const {
        Pacer {
            queue: RefCell::new(PacedQueue::new(STALL_RECOVERY_MS)),
            gate: RefCell::new(None),
            direct: Cell::new(false),
        }
    };
}

/// `inputs` を送り、そのあと `delay_after_ms` だけ次の送信を待たせる(0 なら待たせない)。
///
/// すでに待っている間なら、キューの後ろに並べるだけ(順序は提出の順)。
pub(crate) fn send_paced(inputs: Vec<INPUT>, delay_after_ms: u32) {
    let now = crate::clock::current_tick_ms();
    let submit = PACER.with(|p| p.queue.borrow_mut().submit(inputs, delay_after_ms, now));
    match submit {
        Submit::Queued => {}
        Submit::SendNow { item, arm } => {
            send_direct(&item);
            start_or_release(arm);
        }
        Submit::Recovered { flushed, item, arm } => {
            tracing::warn!(
                "[key-pacer] 待ちが {STALL_RECOVERY_MS}ms 進まなかったので復帰する(溜まっていた {} 件を流す)",
                flushed.len()
            );
            for stale in &flushed {
                send_direct(stale);
            }
            send_direct(&item);
            start_or_release(arm);
        }
    }
}

/// 待っている間に `send_input_safe` が呼ばれたら、その送信をキューの後ろに回す。回したら `true`(呼び出し側は送らない)。
pub(crate) fn divert(inputs: &[INPUT]) -> bool {
    PACER
        .try_with(|p| {
            if p.direct.get() {
                return false;
            }
            let mut queue = p.queue.borrow_mut();
            // 待っている間だけ回す(待っていなければ、`submit` は何も並べずに `SendNow` を返してしまう)。
            queue.is_pacing()
                && matches!(
                    queue.submit(inputs.to_vec(), 0, crate::clock::current_tick_ms()),
                    Submit::Queued
                )
        })
        .unwrap_or(false)
}

/// キューを通さずに、実際に `SendInput` へ送る(`divert` に並べ直されない)。
fn send_direct(inputs: &[INPUT]) {
    let _ = PACER.try_with(|p| p.direct.set(true));
    let _ = crate::win32::send_input_safe(inputs);
    let _ = PACER.try_with(|p| p.direct.set(false));
}

/// 待ちが要るならガードを持ってタスクを起こす。要らなければ(復帰の後始末として)ガードを手放す。
fn start_or_release(arm: Option<Arm>) {
    match arm {
        Some(arm) => {
            // 新しいガードを先に作ってから古いものを手放す(深度が 0 を通らず、保留中の物理キーが途中で流れない)。
            let guard = OutputActiveGuard::begin();
            let _ = PACER.with(|p| p.gate.borrow_mut().replace(guard));
            win32_async::spawn_local(run_pacing(arm));
        }
        None => release_gate(),
    }
}

/// ガードを手放す。保留していた物理キーの再配送が始まる。
fn release_gate() {
    let guard = PACER.with(|p| p.gate.borrow_mut().take());
    drop(guard);
}

/// 待つ → 進める、を、キューが空になる(または持ち主でなくなる)まで繰り返すタスク。
async fn run_pacing(arm: Arm) {
    let mut wait_ms = arm.wait_ms;
    loop {
        win32_async::sleep_ms(wait_ms).await;
        match advance(arm.ticket) {
            Some(next_wait_ms) => wait_ms = next_wait_ms,
            None => return,
        }
    }
}

/// 待ちが終わった状態機械を進め、次の待ち (ms) を返す。`None` なら、タスクはここで終わり。
fn advance(ticket: Ticket) -> Option<u32> {
    loop {
        let next = PACER.with(|p| {
            p.queue
                .borrow_mut()
                .wait_elapsed(ticket, crate::clock::current_tick_ms())
        });
        match next {
            Next::Send { item, wait_ms } => {
                send_direct(&item);
                if wait_ms > 0 {
                    return Some(wait_ms);
                }
            }
            Next::Idle => {
                release_gate();
                return None;
            }
            // 復帰で作り直された。ガードは新しい持ち主のものなので、触らない。
            Next::Superseded => return None,
        }
    }
}
