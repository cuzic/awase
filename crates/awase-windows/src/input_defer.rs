//! 入力遅延キュー — 複数の退避経路を 1 つの API に集約する。
//!
//! ## 内部構造（単一キュー）
//!
//! すべての退避経路（hook 再入・OUTPUT_GATE.active 中・TsfGate drain 等）は
//! classify 済みの `RawKeyEvent` として同一キューに積む。
//!
//! hook 再入時のフック側でも `hook::HOOK_CONFIG` グローバルを使って
//! 直接 classify するため、drain 時の classify クロージャは不要になった。
//! `enrich_ime_relevance`（sync key 判定）のみ drain 側で `with_app` 内に実行する。

use std::sync::Mutex;
use awase::types::RawKeyEvent;

#[allow(missing_debug_implementations)]
pub struct InputDeferQueue {
    queue: Mutex<Vec<RawKeyEvent>>,
}

pub static INPUT_DEFER: InputDeferQueue = InputDeferQueue::new();

impl InputDeferQueue {
    pub const fn new() -> Self {
        Self { queue: Mutex::new(Vec::new()) }
    }

    /// `OUTPUT_GATE.active` = true のときフックから呼ぶ。
    /// drain は `OutputActiveGuard::drop` が担うため post しない。
    pub fn defer_during_output(&self, event: RawKeyEvent) {
        if let Ok(mut q) = self.queue.lock() { q.push(event); }
    }

    /// `with_app` 再入（OUTPUT_GATE.active=false、classify 済み）で退避し drain を要求する。
    pub fn defer_during_with_app(&self, event: RawKeyEvent) {
        if let Ok(mut q) = self.queue.lock() { q.push(event); }
        crate::tsf::probe_bridge::post_drain_output_queue();
    }

    /// TsfGate・タイムアウト等から保留キーをまとめて再投入する。空でも安全（no-op）。
    pub fn replay_later(&self, events: impl IntoIterator<Item = RawKeyEvent>) {
        let mut q = match self.queue.lock() {
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

    /// キューを全て取り出してタイムスタンプ昇順で返す（WM_DRAIN_OUTPUT_QUEUE ハンドラ専用）。
    ///
    /// 取り出し後は `enrich_ime_relevance` を `with_app` 内で呼ぶこと。
    pub fn take_all(&self) -> Vec<RawKeyEvent> {
        let mut q = match self.queue.lock() {
            Ok(q) => q,
            Err(e) => e.into_inner(),
        };
        let mut result = std::mem::take(&mut *q);
        result.sort_by_key(|ev| ev.timestamp);
        result
    }

    /// drain race 検出用（ノンブロッキング）。
    pub fn pending_len_nonblocking(&self) -> Option<usize> {
        self.queue.try_lock().ok().map(|q| q.len())
    }
}
