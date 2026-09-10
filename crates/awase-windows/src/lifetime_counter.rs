use std::sync::atomic::{AtomicU64, Ordering};

/// 診断用の生存期間カウンタ実装を、各モジュールの static から重複排除する。
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
