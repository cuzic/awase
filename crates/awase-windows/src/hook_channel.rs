#![allow(unsafe_code)]
//! フックコールバックからエンジンスレッドへ物理キーを渡す SPSC リング。

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use awase::types::RawKeyEvent;

/// リング容量。`RawKeyEvent` は Copy な POD (数十バイト程度) のため、
/// 256→1024 への引き上げは static 領域を数十KB増やすだけで済む
/// (タイミング定数ではないため `tuning-constants.md` の実測義務対象外)。
pub(crate) const CAP: usize = 1024;
const MASK: usize = CAP - 1;

/// `overflow_state` の下位32bitは dropped カウント、bit32 は overflow ラッチ。
/// この2つを常に単一の `AtomicU64` として一緒に更新・読取することで、
/// 「dropped は反映されたが latch はまだ（またはその逆）」という半端な状態が
/// 他スレッドから観測されないようにする（コードレビュー指摘1）。
const OVERFLOW_COUNT_MASK: u64 = (1u64 << 32) - 1;
const OVERFLOW_LATCH_BIT: u64 = 1u64 << 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProduceResult {
    Accepted,
    Overflow,
}

pub struct HookKeyRing {
    slots: [UnsafeCell<MaybeUninit<RawKeyEvent>>; CAP],
    head: AtomicUsize,
    tail: AtomicUsize,
    /// dropped カウント（下位32bit）と overflow ラッチ（bit32）を1語に詰めた
    /// 状態。フックコールバックはラッチが立っている間、新規キーをリングに
    /// 積まず OS へ直接パススルーする（バッファ再生とパススルーが1打鍵ごとに
    /// 交互混在する順序崩れを防ぐ）。詳細はモジュール冒頭の定数コメント参照。
    overflow_state: AtomicU64,
    /// 直近ダンプ以降に観測した最大占有数（overflow の頻度を実測するため）。
    max_occupancy: AtomicU32,
}

impl std::fmt::Debug for HookKeyRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let overflow_state = self.overflow_state.load(Ordering::Relaxed);
        f.debug_struct("HookKeyRing")
            .field("head", &self.head.load(Ordering::Relaxed))
            .field("tail", &self.tail.load(Ordering::Relaxed))
            .field("dropped", &(overflow_state & OVERFLOW_COUNT_MASK))
            .field(
                "overflow_latched",
                &(overflow_state & OVERFLOW_LATCH_BIT != 0),
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
            overflow_state: AtomicU64::new(0),
            max_occupancy: AtomicU32::new(0),
        }
    }

    pub fn produce(&self, ev: RawKeyEvent) -> ProduceResult {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let occupancy = head.wrapping_sub(tail);
        // フックのホットパスで毎打鍵アトミックRMWを行わないよう、まず素の
        // load で新記録かどうかを確認してから必要な時だけ fetch_max する
        // （コードレビュー指摘7）。
        if occupancy as u32 > self.max_occupancy.load(Ordering::Relaxed) {
            self.max_occupancy
                .fetch_max(occupancy as u32, Ordering::Relaxed);
        }
        if occupancy == CAP {
            // dropped カウントの加算と overflow ラッチの起立を単一の CAS
            // ループで行う（コードレビュー指摘1）。別々の非アトミック操作
            // だと、この2つの間にエンジンスレッドの take_dropped_and_clear_latch
            // が割り込み「latch=true だが dropped=0（既に消費済み）」という
            // 状態が生じ、以後誰も clear を呼ばずラッチが恒久固着しえた。
            let _ = self
                .overflow_state
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                    let count = (old & OVERFLOW_COUNT_MASK).wrapping_add(1) & OVERFLOW_COUNT_MASK;
                    Some(count | OVERFLOW_LATCH_BIT)
                });
            return ProduceResult::Overflow;
        }
        unsafe {
            (*self.slots.get_unchecked(head & MASK).get()).write(ev);
        }
        self.head.store(head.wrapping_add(1), Ordering::Release);
        ProduceResult::Accepted
    }

    /// `tail` スロットの値を読み出し `assume_init_read` する（コードレビュー
    /// 指摘8: `consume_all`/`consume_one` 重複のunsafeヘルパー集約）。
    ///
    /// # Safety
    /// 呼び出し元は `tail != head`（= そのスロットが `produce` 済みで
    /// 未消費）であることを保証し、同一スロットを二重に読み出さないこと。
    unsafe fn read_slot(&self, tail: usize) -> RawKeyEvent {
        (*self.slots.get_unchecked(tail & MASK).get()).assume_init_read()
    }

    pub fn consume_all(&self, sink: &mut impl FnMut(RawKeyEvent)) {
        let mut tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        while tail != head {
            let ev = unsafe { self.read_slot(tail) };
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
        let ev = unsafe { self.read_slot(tail) };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(ev)
    }

    /// dropped カウントの読み取りと overflow ラッチの解除を単一のアトミック
    /// swap で行う（コードレビュー指摘1）。`WM_KEY_FROM_HOOK` ハンドラが
    /// リングを consume し終えた直後に呼ぶこと（呼んだ時点でラッチは必ず
    /// 解除され、以後フックコールバックは通常のパイプラインへ復帰する）。
    pub fn take_dropped_and_clear_latch(&self) -> u32 {
        let old = self.overflow_state.swap(0, Ordering::AcqRel);
        (old & OVERFLOW_COUNT_MASK) as u32
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
        self.overflow_state.load(Ordering::Acquire) & OVERFLOW_LATCH_BIT != 0
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
        assert_eq!(ring.take_dropped_and_clear_latch(), 0);
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
        assert_eq!(ring.take_dropped_and_clear_latch(), 1);
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
        // 明示的に take_dropped_and_clear_latch を呼ぶまで固定される契約）。
        ring.consume_all(&mut |_| {});
        assert!(ring.is_overflow_latched());
        ring.take_dropped_and_clear_latch();
        assert!(!ring.is_overflow_latched());
    }

    /// コードレビュー指摘1の回帰テスト: overflow 直後、dropped カウントと
    /// overflow ラッチが必ず同時に（不可分に）観測できること。
    #[test]
    fn overflow_sets_dropped_and_latch_together() {
        let ring = HookKeyRing::new();
        for n in 0..CAP {
            assert_eq!(ring.produce(ev(n as u16)), ProduceResult::Accepted);
        }
        assert_eq!(ring.produce(ev(0xFFFF)), ProduceResult::Overflow);
        assert_eq!(ring.produce(ev(0xFFFE)), ProduceResult::Overflow);
        // is_overflow_latched() は take_dropped_and_clear_latch() より前に
        // 観測しても必ず true（別々の非アトミック操作だった旧実装ではこの
        // 不変条件が壊れ、latch=true・dropped=0 の恒久固着が起こり得た）。
        assert!(ring.is_overflow_latched());
        assert_eq!(
            ring.take_dropped_and_clear_latch(),
            2,
            "2回の overflow がどちらも dropped に計上されていること"
        );
        assert!(!ring.is_overflow_latched());
    }

    /// コードレビュー指摘1の回帰テスト: take_dropped_and_clear_latch() で
    /// 一度クリアした後、新たな overflow で再びラッチが立てられること
    /// （固着したままにならないこと）。
    #[test]
    fn overflow_can_relatch_after_clear() {
        let ring = HookKeyRing::new();
        for n in 0..CAP {
            assert_eq!(ring.produce(ev(n as u16)), ProduceResult::Accepted);
        }
        assert_eq!(ring.produce(ev(0xFFFF)), ProduceResult::Overflow);
        assert_eq!(ring.take_dropped_and_clear_latch(), 1);
        assert!(!ring.is_overflow_latched());

        ring.consume_all(&mut |_| {});
        for n in 0..CAP {
            assert_eq!(ring.produce(ev(n as u16)), ProduceResult::Accepted);
        }
        assert_eq!(ring.produce(ev(0xFFFE)), ProduceResult::Overflow);
        assert!(
            ring.is_overflow_latched(),
            "clear 後の新規 overflow でラッチが再び立つこと"
        );
        assert_eq!(ring.take_dropped_and_clear_latch(), 1);
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

        let dropped = u16::try_from(ring.take_dropped_and_clear_latch())
            .expect("dropped fits in u16 for this total");
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
