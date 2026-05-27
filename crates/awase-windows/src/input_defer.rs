//! 入力遅延キュー — 複数の退避経路を 1 つの API に集約する。
//!
//! ## 内部構造（単一キュー）
//!
//! すべての退避経路（OUTPUT_GATE.active 中・TsfGate drain・with_app 再入セーフネット等）は
//! classify 済みの `RawKeyEvent` として同一キューに積む。
//! `enrich_ime_relevance`（sync key 判定）のみ drain 側で `with_app` 内に実行する。
//!
//! ## defer_during_with_app の現在の役割
//!
//! `in_with_app` 再入ガードは Step 5e で hook.rs から削除済み。本 API は
//! `on_key_event_callback` で `with_app` が None を返した場合のセーフネットとして
//! 残してある（3rd-party フックチェーンが message pump を行った場合の rare path）。

use std::sync::Mutex;
use awase::types::RawKeyEvent;

pub struct InputDeferQueue {
    queue: Mutex<Vec<RawKeyEvent>>,
}

impl std::fmt::Debug for InputDeferQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputDeferQueue").finish_non_exhaustive()
    }
}

pub static INPUT_DEFER: InputDeferQueue = InputDeferQueue::new();

impl Default for InputDeferQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl InputDeferQueue {
    #[must_use] 
    pub const fn new() -> Self {
        Self { queue: Mutex::new(Vec::new()) }
    }

    /// `OUTPUT_GATE.active` = true のときフックから呼ぶ。
    /// drain は `OutputActiveGuard::drop` が担うため post しない。
    pub fn defer_during_output(&self, event: RawKeyEvent) {
        if let Ok(mut q) = self.queue.lock() { q.push(event); }
    }

    /// `with_app` 再入セーフネット (3rd-party フック message pump 等の rare path 用)。
    /// event は classify 済み。OUTPUT_GATE.active=false なので drain も明示的に要求する。
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
        let mut result = {
            let mut q = match self.queue.lock() {
                Ok(q) => q,
                Err(e) => e.into_inner(),
            };
            std::mem::take(&mut *q)
        };
        result.sort_by_key(|ev| ev.timestamp);
        result
    }

    /// drain race 検出用（ノンブロッキング）。
    pub fn pending_len_nonblocking(&self) -> Option<usize> {
        self.queue.try_lock().ok().map(|q| q.len())
    }
}
