use std::sync::atomic::{AtomicU64, Ordering};

/// 診断用の生存期間カウンタ実装を、各モジュールの static から重複排除する。
///
/// 「増分し続け、drain/読み取りで消費する」累積カウンタ専用。`bump`/`current`で
/// staleness検知に使う単調フェンス（`probe_actuation_fence::ProbeFence::fence_value`・
/// `conv_mutation::CONV_MUTATION_SEQ`・`tsf::observer::ChangeCounter`）とは
/// 意味論が異なるため統合しない——値の減少（`drain`）を伴わず、比較対象の
/// スナップショットとして使う目的のカウンタにはこちらではなく`ChangeCounter`
/// 相当のパターンを使うこと。
pub(crate) struct LifetimeCounter {
    value: AtomicU64,
}

impl LifetimeCounter {
    pub(crate) const fn new() -> Self {
        Self {
            value: AtomicU64::new(0),
        }
    }

    pub(crate) fn increment(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn read(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    pub(crate) fn drain(&self) -> u64 {
        self.value.swap(0, Ordering::Relaxed)
    }
}
