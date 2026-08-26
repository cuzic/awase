#![allow(unsafe_code)]
//! フックコールバックからエンジンスレッドへ物理キーを渡す SPSC リング。

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use awase::types::RawKeyEvent;

const CAP: usize = 256;
const MASK: usize = CAP - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProduceResult {
    Accepted,
    Overflow,
}

pub struct HookKeyRing {
    slots: [UnsafeCell<MaybeUninit<RawKeyEvent>>; CAP],
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped: AtomicU32,
}

impl std::fmt::Debug for HookKeyRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookKeyRing")
            .field("head", &self.head.load(Ordering::Relaxed))
            .field("tail", &self.tail.load(Ordering::Relaxed))
            .field("dropped", &self.dropped.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

unsafe impl Sync for HookKeyRing {}

impl HookKeyRing {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: [const { UnsafeCell::new(MaybeUninit::uninit()) }; CAP],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped: AtomicU32::new(0),
        }
    }

    pub fn produce(&self, ev: RawKeyEvent) -> ProduceResult {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) == CAP {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return ProduceResult::Overflow;
        }
        unsafe {
            (*self.slots.get_unchecked(head & MASK).get()).write(ev);
        }
        self.head.store(head.wrapping_add(1), Ordering::Release);
        ProduceResult::Accepted
    }

    pub fn consume_all(&self, sink: &mut impl FnMut(RawKeyEvent)) {
        let mut tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        while tail != head {
            let ev = unsafe { (*self.slots.get_unchecked(tail & MASK).get()).assume_init_read() };
            sink(ev);
            tail = tail.wrapping_add(1);
        }
        self.tail.store(tail, Ordering::Release);
    }

    pub fn take_dropped(&self) -> u32 {
        self.dropped.swap(0, Ordering::AcqRel)
    }

    pub fn has_pending(&self) -> bool {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        head != tail
    }
}

impl Default for HookKeyRing {
    fn default() -> Self {
        Self::new()
    }
}

pub static HOOK_KEYS: HookKeyRing = HookKeyRing::new();
pub static WAKE_PENDING: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
pub fn request_engine_wake() {
    if !WAKE_PENDING.swap(true, Ordering::AcqRel)
        && !crate::win32::post_to_main_thread(crate::WM_KEY_FROM_HOOK)
    {
        WAKE_PENDING.store(false, Ordering::Release);
    }
}

#[cfg(windows)]
pub fn recover_stuck_wake_if_needed() {
    if HOOK_KEYS.has_pending() && WAKE_PENDING.swap(false, Ordering::AcqRel) {
        log::warn!("[hook-ring] WAKE_PENDING recovered by hook watchdog");
        request_engine_wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use awase::engine::ModifierState;
    use awase::types::{ImeRelevance, KeyClassification, KeyEventType, ScanCode, VkCode};

    fn ev(n: u16) -> RawKeyEvent {
        RawKeyEvent {
            vk_code: VkCode(n),
            scan_code: ScanCode(u32::from(n)),
            event_type: KeyEventType::KeyDown,
            extra_info: 0,
            timestamp: u64::from(n),
            key_classification: KeyClassification::Passthrough,
            physical_pos: None,
            ime_relevance: ImeRelevance::default(),
            modifier_key: None,
            modifier_snapshot: ModifierState::default(),
            injected: false,
        }
    }

    #[test]
    fn consumes_in_produced_order() {
        let ring = HookKeyRing::new();
        for n in 0..CAP {
            assert_eq!(ring.produce(ev(n as u16)), ProduceResult::Accepted);
        }
        let mut got = Vec::new();
        ring.consume_all(&mut |e| got.push(e.vk_code.0));
        assert_eq!(got.len(), CAP);
        assert_eq!(got.first().copied(), Some(0));
        assert_eq!(got.last().copied(), Some((CAP - 1) as u16));
        assert_eq!(ring.take_dropped(), 0);
    }

    #[test]
    fn overflow_drops_newest_and_preserves_existing_order() {
        let ring = HookKeyRing::new();
        for n in 0..CAP {
            assert_eq!(ring.produce(ev(n as u16)), ProduceResult::Accepted);
        }
        assert_eq!(ring.produce(ev(999)), ProduceResult::Overflow);
        assert_eq!(ring.take_dropped(), 1);
        let mut got = Vec::new();
        ring.consume_all(&mut |e| got.push(e.vk_code.0));
        assert_eq!(got.len(), CAP);
        assert!(!got.contains(&999));
    }
}
