//! 入力遅延キュー — 複数の退避経路を 1 つの API に集約する。
//!
//! ## 保持する 2 本のキュー（Step 2 で 1 本に統合予定）
//! - `iwa`（in_with_app）: `with_app` 再入中にフックから届いた生イベント（[`RawHookData`]）
//! - `pending`：OUTPUT_ACTIVE 中・TsfGate drain 等の [`RawKeyEvent`]

use std::sync::Mutex;
use awase::types::RawKeyEvent;
use crate::tsf::probe_bridge::RawHookData;

#[allow(missing_debug_implementations)]
pub struct InputDeferQueue {
    iwa: Mutex<Vec<RawHookData>>,
    pending: Mutex<Vec<RawKeyEvent>>,
}

pub static INPUT_DEFER: InputDeferQueue = InputDeferQueue::new();

impl InputDeferQueue {
    pub const fn new() -> Self {
        Self { iwa: Mutex::new(Vec::new()), pending: Mutex::new(Vec::new()) }
    }

    /// `in_with_app()` = true のときフックから呼ぶ。生データを退避し drain を要求する。
    pub fn defer_from_hook_reentry(&self, raw: RawHookData) {
        if let Ok(mut q) = self.iwa.lock() { q.push(raw); }
        crate::tsf::probe_bridge::post_drain_output_queue();
    }

    /// `OUTPUT_ACTIVE` = true のときフックから呼ぶ。
    /// drain は `OutputActiveGuard::drop` が担うため post しない。
    pub fn defer_during_output(&self, event: RawKeyEvent) {
        if let Ok(mut q) = self.pending.lock() { q.push(event); }
    }

    /// `with_app` 再入（OUTPUT_ACTIVE=false、classify 済み）で退避し drain を要求する。
    pub fn defer_during_with_app(&self, event: RawKeyEvent) {
        if let Ok(mut q) = self.pending.lock() { q.push(event); }
        crate::tsf::probe_bridge::post_drain_output_queue();
    }

    /// TsfGate・タイムアウト等から保留キーをまとめて再投入する。空でも安全（no-op）。
    pub fn replay_later(&self, events: impl IntoIterator<Item = RawKeyEvent>) {
        let mut q = match self.pending.lock() {
            Ok(q) => q,
            Err(e) => e.into_inner(),
        };
        let prev_len = q.len();
        q.extend(events);
        if q.len() > prev_len {
            drop(q);
            crate::tsf::probe_bridge::post_drain_output_queue();
        }
    }

    /// in_with_app キュー全体を取り出す（WM_DRAIN_OUTPUT_QUEUE ハンドラ専用）。
    pub fn take_iwa(&self) -> Vec<RawHookData> {
        let mut q = match self.iwa.lock() { Ok(q) => q, Err(e) => e.into_inner() };
        std::mem::take(&mut *q)
    }

    /// pending キュー全体を取り出す（WM_DRAIN_OUTPUT_QUEUE ハンドラ専用）。
    pub fn take_pending(&self) -> Vec<RawKeyEvent> {
        let mut q = match self.pending.lock() { Ok(q) => q, Err(e) => e.into_inner() };
        std::mem::take(&mut *q)
    }

    /// drain race 検出用（ノンブロッキング）。
    pub fn pending_len_nonblocking(&self) -> Option<usize> {
        self.pending.try_lock().ok().map(|q| q.len())
    }
}
