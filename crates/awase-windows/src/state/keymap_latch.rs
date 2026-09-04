//! `[[keymap]]` の KeyUp 回収・自動リピート抑制用 latch テーブル（ADR-114 決定4）。
//!
//! KeyDown で `[[keymap]]` にマッチした vk を、対応する物理 KeyUp が来るまで
//! 記録する。KeyUp 回収と自動リピート抑制の両方をこの1つの latch テーブルの
//! 有無で判定する（`message_handlers.rs::deliver_key_event` ステップ1 が
//! 唯一の呼び出し元、`runtime/key_pipeline.rs` は #[cfg(windows)] のため
//! 非 Windows では未使用になる、`conv_classify.rs` 等と同じ局所抑制パターン）。
//!
//! latch は vk 単位のキーであり、マッチしたルールへの参照を保持しない。その
//! ため `reload_config()` で `KeymapTable` が丸ごと差し替わっても、進行中の
//! KeyUp 待ちはそのまま latch テーブルの記録どおりに回収される（BUG-100 が
//! 「テーブル参照方式では config reload 中に latch が破綻する」と指摘した
//! 問題を、この設計そのものによって回避する）。

use awase::types::VkCode;

/// latch されている vk の集合。同時に latch される件数は小さいため線形探索で
/// 十分（キャパシティ上限は設けない）。
#[derive(Debug, Clone, Default)]
pub(crate) struct KeymapLatch(Vec<VkCode>);

impl KeymapLatch {
    #[must_use]
    pub(crate) fn is_latched(&self, vk: VkCode) -> bool {
        self.0.contains(&vk)
    }

    /// vk を latch する。既に latch 済みなら no-op（冪等）。
    ///
    /// repeat 判定の根拠は呼び出し側の `is_latched(vk)` チェックであって
    /// この関数の戻り値ではない——`deliver_key_event` のステップ1 が
    /// `is_latched` で先に分岐して return する設計上、`find_match` 照合
    /// （ステップ2）に到達する時点でその vk は必ず未 latch である。
    pub(crate) fn latch(&mut self, vk: VkCode) {
        if !self.0.contains(&vk) {
            self.0.push(vk);
        }
    }

    /// vk の latch を解放する。latch されていれば `true`、なければ `false`。
    pub(crate) fn release(&mut self, vk: VkCode) -> bool {
        let before = self.0.len();
        self.0.retain(|&v| v != vk);
        self.0.len() != before
    }

    /// latch テーブルを空にする（ADR-114 decision4「latch 漏れ対策」経路3・5）。
    ///
    /// **`target_vk` の KeyUp は注入しない**（テーブルを空にするだけ）。ADR-114
    /// 決定3および ADR-130 決定3が、`to` の各ステップを KeyDown 側で Down+Up
    /// ペアとして即時完結させる設計であるため、`target_vk` が「押されたまま」
    /// 残ることは構造的に無い。ADR-110 の `release_all_latched_remap_targets()`
    /// （`target_vk` を押しっぱなしにする設計だったため KeyUp 注入が必要だった）
    /// とは前提が異なる。
    pub(crate) fn release_all(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vk(n: u16) -> VkCode {
        VkCode(n)
    }

    #[test]
    fn latch_and_is_latched_roundtrip() {
        let mut latch = KeymapLatch::default();
        assert!(!latch.is_latched(vk(1)));
        latch.latch(vk(1));
        assert!(latch.is_latched(vk(1)));
        assert!(!latch.is_latched(vk(2)));
    }

    #[test]
    fn latch_is_idempotent() {
        let mut latch = KeymapLatch::default();
        latch.latch(vk(1));
        latch.latch(vk(1));
        assert!(latch.release(vk(1)));
        // 二重 latch されていないので、1回の release で完全に解放される。
        assert!(!latch.is_latched(vk(1)));
    }

    #[test]
    fn release_returns_false_when_not_latched() {
        let mut latch = KeymapLatch::default();
        assert!(!latch.release(vk(1)));
    }

    #[test]
    fn release_all_clears_every_entry() {
        let mut latch = KeymapLatch::default();
        latch.latch(vk(1));
        latch.latch(vk(2));
        latch.release_all();
        assert!(!latch.is_latched(vk(1)));
        assert!(!latch.is_latched(vk(2)));
    }

    #[test]
    fn multiple_vks_are_tracked_independently() {
        let mut latch = KeymapLatch::default();
        latch.latch(vk(1));
        latch.latch(vk(2));
        assert!(latch.release(vk(1)));
        assert!(latch.is_latched(vk(2)));
        assert!(!latch.is_latched(vk(1)));
    }
}
