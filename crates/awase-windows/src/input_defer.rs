//! 入力遅延キュー — 複数の退避経路を 1 つの API に集約する。
//!
//! ## 内部構造（Step 2: 1 本の VecDeque に統合）
//!
//! `iwa`（in_with_app）は hook 再入のため classify 不可のまま `RawHookData` で保持し、
//! `take_all(classify_fn)` 呼び出し時（WM_DRAIN_OUTPUT_QUEUE ハンドラ内、with_app 内）に
//! `RawKeyEvent` へ変換してから `pending` と合流させる。
//!
//! - `iwa`: hook 再入中に到着した生イベント（`RawHookData`、classify は drain 時）
//! - `pending`: OUTPUT_ACTIVE 中・TsfGate drain 等の classify 済みイベント（`RawKeyEvent`）

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

    /// 両キューを合流させて返す（WM_DRAIN_OUTPUT_QUEUE ハンドラの `with_app` 内専用）。
    ///
    /// `classify_fn` は `RawHookData` を `RawKeyEvent` に変換する関数。
    /// `with_app` 内から呼ぶこと（APP への読み取りアクセスが必要）。
    /// iwa 側を先に並べ、時系列上の到着順を保持する。
    pub fn take_all(&self, classify_fn: impl Fn(RawHookData) -> Option<RawKeyEvent>) -> Vec<RawKeyEvent> {
        let iwa = {
            let mut q = match self.iwa.lock() { Ok(q) => q, Err(e) => e.into_inner() };
            std::mem::take(&mut *q)
        };
        let pending = {
            let mut q = match self.pending.lock() { Ok(q) => q, Err(e) => e.into_inner() };
            std::mem::take(&mut *q)
        };
        let mut result = Vec::with_capacity(iwa.len() + pending.len());
        for raw in iwa {
            if let Some(ev) = classify_fn(raw) {
                result.push(ev);
            }
        }
        result.extend(pending);
        result
    }

    /// drain race 検出用（ノンブロッキング）。
    pub fn pending_len_nonblocking(&self) -> Option<usize> {
        self.pending.try_lock().ok().map(|q| q.len())
    }
}
