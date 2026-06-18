//! 複数タイマーティックにわたるキー送信シーケンス。

use std::collections::VecDeque;

use awase::types::VkCode;

/// 複数タイマーティックにわたるキー送信シーケンス。
///
/// 各ステップは直前送信からの待ち時間と VK コードを持つ。
/// タイマーコールバックごとに [`poll`] を呼ぶことで、各キーを所定のタイミングで送信する。
///
/// # 例
///
/// ```
/// // F22 → F21 を 1 ティックずつ送信（間隔 = タイマー間隔 ≒ 10ms）
/// let seq = KeySeq::new(now_ms).key(VK_F22).key(VK_F21);
///
/// // F22 の 50ms 後に F21
/// let seq = KeySeq::new(now_ms).key(VK_F22).wait_key(50, VK_F21);
/// ```
///
/// [`poll`]: KeySeq::poll
pub(crate) struct KeySeq {
    steps: VecDeque<KeySeqStep>,
    /// 直前のキー送信（またはシーケンス開始）時刻 (ms)。
    /// 次ステップの送信タイミングを決める基準点。
    prev_ms: u64,
}

struct KeySeqStep {
    /// 前ステップ送信からこのキーを送信するまでの待ち時間 (ms)。0 = 即次ティック。
    wait_ms: u64,
    vk: VkCode,
}

/// [`KeySeq::poll`] の戻り値。
pub(crate) enum KeySeqPoll {
    /// このティックで `vk` を送信すること。
    Send(VkCode),
    /// 次ステップの待ち時間がまだ経過していない。次ティックで再 poll すること。
    Pending,
    /// 全ステップ完了。最後の送信時刻 (ms) を返す。
    Done(u64),
}

impl KeySeq {
    /// `now_ms` を起点として新しいシーケンスを作る。
    pub(crate) fn new(now_ms: u64) -> Self {
        Self {
            steps: VecDeque::new(),
            prev_ms: now_ms,
        }
    }

    /// 直前送信の直後（wait_ms=0）に `vk` を送信するステップを追加する。
    pub(crate) fn key(self, vk: VkCode) -> Self {
        self.wait_key(0, vk)
    }

    /// 直前送信から `wait_ms` ms 後に `vk` を送信するステップを追加する。
    pub(crate) fn wait_key(mut self, wait_ms: u64, vk: VkCode) -> Self {
        self.steps.push_back(KeySeqStep { wait_ms, vk });
        self
    }

    /// タイマーティックごとに呼ぶ。次に送信すべきキーがあれば [`KeySeqPoll::Send`] を返す。
    ///
    /// `Send(vk)` を受け取ったら呼び出し側が VK を実際に送信すること。
    /// 内部の送信時刻は `now_ms` で更新されるため、次 poll では次ステップの待ち時間が計算される。
    pub(crate) fn poll(&mut self, now_ms: u64) -> KeySeqPoll {
        match self.steps.front() {
            None => KeySeqPoll::Done(self.prev_ms),
            Some(step) if now_ms >= self.prev_ms.saturating_add(step.wait_ms) => {
                let vk = self.steps.pop_front().unwrap().vk;
                self.prev_ms = now_ms;
                KeySeqPoll::Send(vk)
            }
            _ => KeySeqPoll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_seq_is_done_immediately() {
        let mut seq = KeySeq::new(100);
        assert!(matches!(seq.poll(100), KeySeqPoll::Done(100)));
    }

    #[test]
    fn single_key_sends_on_first_poll() {
        let mut seq = KeySeq::new(100).key(VkCode(0x85)); // VK_F22
        // wait_ms=0 なので now(100) >= prev(100)+0 → Send
        let poll = seq.poll(100);
        assert!(matches!(poll, KeySeqPoll::Send(VkCode(0x85))));
        // 次は Done
        assert!(matches!(seq.poll(110), KeySeqPoll::Done(100)));
    }

    #[test]
    fn two_keys_send_on_consecutive_polls() {
        let mut seq = KeySeq::new(100).key(VkCode(0x85)).key(VkCode(0x84)); // F22, F21
        assert!(matches!(seq.poll(100), KeySeqPoll::Send(VkCode(0x85))));
        // prev_ms = 100、次は wait_ms=0 なので now(110) >= 100+0 → Send
        assert!(matches!(seq.poll(110), KeySeqPoll::Send(VkCode(0x84))));
        assert!(matches!(seq.poll(120), KeySeqPoll::Done(110)));
    }

    #[test]
    fn wait_key_blocks_until_delay_elapsed() {
        let mut seq = KeySeq::new(100).key(VkCode(0x85)).wait_key(50, VkCode(0x84));
        assert!(matches!(seq.poll(100), KeySeqPoll::Send(VkCode(0x85)))); // prev_ms = 100
        // 10ms 後: 100+50=150 > 110 → Pending
        assert!(matches!(seq.poll(110), KeySeqPoll::Pending));
        // 50ms 後: 100+50=150 <= 150 → Send
        assert!(matches!(seq.poll(150), KeySeqPoll::Send(VkCode(0x84))));
        assert!(matches!(seq.poll(160), KeySeqPoll::Done(150)));
    }
}
