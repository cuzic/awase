#![allow(unsafe_code)]
//! フックコールバックからエンジンスレッドへ物理キーを渡す SPSC リング。

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use awase::types::RawKeyEvent;

/// リング容量。`RawKeyEvent` は Copy な POD (数十バイト程度) のため、
/// 256→1024 への引き上げは static 領域を数十KB増やすだけで済む
/// (タイミング定数ではないため `tuning-constants.md` の実測義務対象外)。
pub(crate) const CAP: usize = 1024;
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
    /// 直近ダンプ以降に観測した最大占有数（overflow の頻度を実測するため）。
    max_occupancy: AtomicU32,
    /// overflow 発生後、リングが空になり呼び出し元が resync を確認するまで
    /// 立てておくラッチ。フックコールバックはこれが立っている間、新規キーを
    /// リングに積まず OS へ直接パススルーする（バッファ再生とパススルーが
    /// 1打鍵ごとに交互混在する順序崩れを防ぐ）。
    overflow_latched: AtomicBool,
}

impl std::fmt::Debug for HookKeyRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookKeyRing")
            .field("head", &self.head.load(Ordering::Relaxed))
            .field("tail", &self.tail.load(Ordering::Relaxed))
            .field("dropped", &self.dropped.load(Ordering::Relaxed))
            .field(
                "overflow_latched",
                &self.overflow_latched.load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

unsafe impl Sync for HookKeyRing {}

impl HookKeyRing {
    #[must_use]
    // `slots` はこの const fn 経由で `static HOOK_KEYS` にのみ書き込まれ、
    // 呼び出し時の実スタックには積まれない（256→1024 引き上げで clippy の
    // ヒューリスティックが誤検出するようになった、指摘2-2）。
    #[allow(clippy::large_stack_arrays)]
    pub const fn new() -> Self {
        Self {
            slots: [const { UnsafeCell::new(MaybeUninit::uninit()) }; CAP],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped: AtomicU32::new(0),
            max_occupancy: AtomicU32::new(0),
            overflow_latched: AtomicBool::new(false),
        }
    }

    pub fn produce(&self, ev: RawKeyEvent) -> ProduceResult {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let occupancy = head.wrapping_sub(tail);
        self.max_occupancy
            .fetch_max(occupancy as u32, Ordering::Relaxed);
        if occupancy == CAP {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            self.overflow_latched.store(true, Ordering::Release);
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

    /// `consume_all` と同じ手順で1件だけ取り出す（実負荷テスト用）。
    pub fn consume_one(&self) -> Option<RawKeyEvent> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let ev = unsafe { (*self.slots.get_unchecked(tail & MASK).get()).assume_init_read() };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(ev)
    }

    pub fn take_dropped(&self) -> u32 {
        self.dropped.swap(0, Ordering::AcqRel)
    }

    /// 直近ダンプ以降の最大占有数を読み取ってリセットする（`WM_DUMP_JOURNAL` 用）。
    pub fn take_max_occupancy(&self) -> u32 {
        self.max_occupancy.swap(0, Ordering::AcqRel)
    }

    pub fn has_pending(&self) -> bool {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        head != tail
    }

    /// overflow ラッチが立っているか（フックコールバックがパススルー固定中か）。
    #[must_use]
    pub fn is_overflow_latched(&self) -> bool {
        self.overflow_latched.load(Ordering::Acquire)
    }

    /// overflow ラッチを解除する。`WM_KEY_FROM_HOOK` ハンドラが
    /// `dropped > 0` を観測しリングを consume し終えた直後に呼ぶこと。
    pub fn clear_overflow_latch(&self) {
        self.overflow_latched.store(false, Ordering::Release);
    }
}

impl Default for HookKeyRing {
    fn default() -> Self {
        Self::new()
    }
}

pub static HOOK_KEYS: HookKeyRing = HookKeyRing::new();
pub static WAKE_PENDING: AtomicBool = AtomicBool::new(false);
/// フックコールバック上ではログを出せないため、`request_engine_wake` の post 失敗を
/// ここに記録するだけにする。実際のログ出力はエンジンスレッド側のウォッチドッグ
/// （`recover_stuck_wake_if_needed`）が行う。両関数とも `#[cfg(windows)]` 限定なので
/// このstatic自体も同様にゲートする（Linux単体ビルドでの dead_code 警告を避ける）。
#[cfg(windows)]
static WAKE_POST_FAILED: AtomicBool = AtomicBool::new(false);

/// `WH_KEYBOARD_LL` フックコールバックから同期的に呼ぶ。
///
/// ロック取得・アロケーション・ブロッキング呼び出し・ログ出力を一切行わない
/// （`post_to_main_thread_quiet` を使い、失敗してもログは出さずフラグだけ立てる）。
#[cfg(windows)]
pub fn request_engine_wake() {
    if !WAKE_PENDING.swap(true, Ordering::AcqRel)
        && !crate::win32::post_to_main_thread_quiet(crate::WM_KEY_FROM_HOOK)
    {
        WAKE_PENDING.store(false, Ordering::Release);
        WAKE_POST_FAILED.store(true, Ordering::Release);
    }
}

/// エンジンスレッド側のウォッチドッグから呼ぶ。フック側で記録された post 失敗を
/// ここでログに残し、`WAKE_PENDING` が固着していれば回収する。
#[cfg(windows)]
pub fn recover_stuck_wake_if_needed() {
    if WAKE_POST_FAILED.swap(false, Ordering::AcqRel) {
        log::warn!("[hook-ring] request_engine_wake の PostMessageW が失敗した形跡があります");
    }
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
        // CAP (1024) を超える marker 値。範囲内の n と衝突しないこと。
        let overflow_marker: u16 = 0xFFFF;
        assert_eq!(ring.produce(ev(overflow_marker)), ProduceResult::Overflow);
        assert_eq!(ring.take_dropped(), 1);
        let mut got = Vec::new();
        ring.consume_all(&mut |e| got.push(e.vk_code.0));
        assert_eq!(got.len(), CAP);
        assert!(!got.contains(&overflow_marker));
    }

    #[test]
    fn overflow_latches_until_explicitly_cleared() {
        let ring = HookKeyRing::new();
        for n in 0..CAP {
            assert_eq!(ring.produce(ev(n as u16)), ProduceResult::Accepted);
        }
        assert!(!ring.is_overflow_latched());
        assert_eq!(ring.produce(ev(0xFFFF)), ProduceResult::Overflow);
        assert!(ring.is_overflow_latched());
        // consume_all だけではラッチは解除されない（WM_KEY_FROM_HOOK ハンドラが
        // 明示的に clear_overflow_latch を呼ぶまで固定される契約）。
        ring.consume_all(&mut |_| {});
        assert!(ring.is_overflow_latched());
        ring.clear_overflow_latch();
        assert!(!ring.is_overflow_latched());
    }

    #[test]
    fn max_occupancy_tracks_high_water_mark_and_resets_on_take() {
        let ring = HookKeyRing::new();
        for n in 0..10 {
            assert_eq!(ring.produce(ev(n)), ProduceResult::Accepted);
        }
        // 10 件目 produce 時点の occupancy (produce 前) は 9。
        assert_eq!(ring.take_max_occupancy(), 9);
        assert_eq!(ring.take_max_occupancy(), 0, "take 後はリセットされる");

        for _ in 0..5 {
            assert!(ring.consume_one().is_some());
        }
        assert_eq!(ring.produce(ev(100)), ProduceResult::Accepted);
        // consume 後の occupancy (produce 前) は 5。
        assert_eq!(ring.take_max_occupancy(), 5);
    }

    #[test]
    fn consume_one_matches_consume_all_order() {
        let ring = HookKeyRing::new();
        for n in 0..8 {
            assert_eq!(ring.produce(ev(n)), ProduceResult::Accepted);
        }
        for expected in 0..8u16 {
            assert_eq!(ring.consume_one().map(|e| e.vk_code.0), Some(expected));
        }
        assert!(ring.consume_one().is_none());
    }

    /// 2スレッド実負荷テスト（Linux CI で実行可能）。producer が N (≫CAP) 件を
    /// 連番で produce、consumer が consume_one で回収する。overflow による
    /// drop は許容するが、受け取った VK 列は単調増加（順序保存）であること、
    /// 受信件数 + dropped == N（=消失が無いこと）を検証する。
    #[test]
    fn concurrent_producer_consumer_preserves_order_with_no_silent_loss() {
        use std::sync::Arc;
        use std::thread;

        let total: u16 = 5000;
        let ring = Arc::new(HookKeyRing::new());

        let producer = {
            let ring = Arc::clone(&ring);
            thread::spawn(move || {
                for n in 0..total {
                    let _ = ring.produce(ev(n));
                }
            })
        };

        let mut got = Vec::new();
        loop {
            match ring.consume_one() {
                Some(e) => got.push(e.vk_code.0),
                None => {
                    if producer.is_finished() {
                        let mut drained_more = false;
                        while let Some(e) = ring.consume_one() {
                            got.push(e.vk_code.0);
                            drained_more = true;
                        }
                        if !drained_more {
                            break;
                        }
                    } else {
                        thread::yield_now();
                    }
                }
            }
        }
        producer.join().expect("producer thread panicked");

        let dropped =
            u16::try_from(ring.take_dropped()).expect("dropped fits in u16 for this total");
        assert_eq!(
            got.len() as u16 + dropped,
            total,
            "受信件数({}) + dropped({dropped}) は total({total}) と一致するはず（消失なし）",
            got.len(),
        );
        for pair in got.windows(2) {
            assert!(pair[0] < pair[1], "順序が保存されていない: {pair:?}");
        }
    }
}
