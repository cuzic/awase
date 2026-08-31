use std::collections::VecDeque;

use crate::types::{KeyAction, ScanCode};

#[cfg(test)]
use crate::types::VkCode;

/// 出力履歴の1エントリ
#[derive(Debug, Clone)]
pub struct OutputEntry {
    /// 物理キーのスキャンコード
    pub scan_code: ScanCode,
    /// 送信したローマ字
    pub romaji: String,
    /// 対応するひらがな（n-gram 用）
    pub kana: Option<char>,
    /// 出力した KeyAction（KeyUp 整合性用）
    pub action: KeyAction,
}

/// `committed` の上限（ADR-112 決定0）。
///
/// n-gram 文脈で使う最大量（`timing::NGRAM_CONTEXT_SIZE`）に十分な余裕を足した値。
/// `NGRAM_CONTEXT_SIZE` は本モジュールから参照できない（`timing` → `nicola_fsm` →
/// `output_history` の依存方向のため）ので、ここでは実際の利用箇所より確実に
/// 大きい固定値を直接持つ。値を変えても `recent_kana` の呼び出し側が要求する
/// 件数（現状 3 件程度）を下回らない限り観測可能な挙動は変わらない。
const COMMITTED_CAPACITY: usize = 32;

/// Engine が出力した内容の履歴
///
/// `pending_releases`（KeyUp 整合性用の解放待ちインデックス）と `committed`
/// （n-gram 文脈・Speculative retraction 用の確定出力ログ）を分離して持つ
/// （ADR-112 決定0）。旧実装はこの2つの責務を単一の無制限 `Vec` に同居させて
/// おり、(a) 誰も除去しないため無制限に増え続ける、(b) 一方の都合（KeyUp 整合性）
/// での除去がもう一方（n-gram 文脈）の内容まで変えてしまう、という2つの欠陥を
/// 抱えていた。詳細は `docs/adr/112-keyup-lifecycle-fsm-delivery.md` 参照。
#[derive(Debug, Default)]
pub struct OutputHistory {
    /// KeyUp 整合性用の解放待ちインデックス。`push` で追加し、対応する物理キーの
    /// KeyUp が届いたときに `remove_by_scan`/`find_action_by_scan` で参照・除去する。
    /// 「今押されている物理キー」の数のオーダーでしか増えないため上限を設けない。
    pending_releases: Vec<OutputEntry>,
    /// n-gram 文脈・Speculative retraction 専用の確定出力ログ。`remove_by_scan` では
    /// 一切変更されない。`COMMITTED_CAPACITY` を超えたら古い方から捨てる。
    committed: VecDeque<OutputEntry>,
}

impl OutputHistory {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending_releases: Vec::new(),
            committed: VecDeque::new(),
        }
    }

    /// 出力を記録する（`pending_releases`/`committed` の両方に追加）
    pub fn push(&mut self, entry: OutputEntry) {
        self.pending_releases.push(entry.clone());
        self.push_committed(entry);
    }

    fn push_committed(&mut self, entry: OutputEntry) {
        self.committed.push_back(entry);
        while self.committed.len() > COMMITTED_CAPACITY {
            self.committed.pop_front();
        }
    }

    /// 最後の出力を取り消す（Speculative retraction 用、`committed` のみ操作）
    pub fn retract_last(&mut self) -> Option<OutputEntry> {
        self.committed.pop_back()
    }

    /// 取り消しに必要な BS 回数
    /// 完全なローマ字は IME で 1 composition unit になるため、常に 1。
    #[must_use]
    #[allow(clippy::bool_to_int_with_if)] // usize::from(bool) is not const-stable
    pub fn retract_bs_count(&self) -> usize {
        if self.committed.is_empty() {
            0
        } else {
            1
        }
    }

    /// `RetractAndRecord` 専用: `committed` を取り消して新しいエントリを記録すると
    /// 同時に、`pending_releases` 側も同じ scan_code のエントリを新しい内容へ
    /// 置き換える（追記ではなく upsert）。
    ///
    /// 投機出力の訂正は同一物理キー押下に対する最終確定であり、その物理キーの
    /// KeyUp が来たときに参照すべき内容も訂正後のものであるべきなので、
    /// 単純な `push`（追記）だと古いエントリが `pending_releases` に残り続け、
    /// `remove_by_scan` が先に見つけてしまう（`.position()` は先頭から検索）
    /// リスクがある。
    pub fn retract_and_record(&mut self, entry: OutputEntry) -> Option<OutputEntry> {
        let retracted = self.retract_last();
        if let Some(pos) = self
            .pending_releases
            .iter()
            .position(|e| e.scan_code == entry.scan_code)
        {
            self.pending_releases[pos] = entry.clone();
        } else {
            self.pending_releases.push(entry.clone());
        }
        self.push_committed(entry);
        retracted
    }

    /// n-gram 用の直近かな文字列（古い順）。`committed` を参照する。
    #[must_use]
    pub fn recent_kana(&self, n: usize) -> Vec<char> {
        let mut result: Vec<char> = Vec::with_capacity(n.min(self.committed.len()));
        result.extend(self.committed.iter().rev().filter_map(|e| e.kana).take(n));
        result.reverse();
        result
    }

    /// scan_code に対応するアクションを検索（KeyUp 用、`pending_releases` を参照）
    #[must_use]
    pub fn find_action_by_scan(&self, scan_code: ScanCode) -> Option<&KeyAction> {
        self.pending_releases
            .iter()
            .rev()
            .find(|e| e.scan_code == scan_code)
            .map(|e| &e.action)
    }

    /// scan_code に対応するエントリを除去して返す（KeyUp 用、`pending_releases` を操作）
    pub fn remove_by_scan(&mut self, scan_code: ScanCode) -> Option<OutputEntry> {
        self.pending_releases
            .iter()
            .position(|e| e.scan_code == scan_code)
            .map(|pos| self.pending_releases.remove(pos))
    }

    /// GUI プレビュー用: 出力テキスト（`committed` を参照）
    #[must_use]
    pub fn display_text(&self) -> String {
        self.committed.iter().filter_map(|e| e.kana).collect()
    }

    /// エントリ数（`committed` の件数、n-gram/retract の観測対象）
    #[must_use]
    pub fn len(&self) -> usize {
        self.committed.len()
    }

    /// 空かどうか（`committed` 基準）
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.committed.is_empty()
    }

    /// 全エントリをクリア（`pending_releases`/`committed` の両方）
    pub fn clear(&mut self) {
        self.pending_releases.clear();
        self.committed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::KeyAction;

    fn make_entry(scan_code: ScanCode, romaji: &str, kana: Option<char>) -> OutputEntry {
        OutputEntry {
            scan_code,
            romaji: romaji.to_string(),
            kana,
            action: KeyAction::Romaji(romaji.to_string()),
        }
    }

    #[test]
    fn test_push_and_recent_kana() {
        let mut h = OutputHistory::new();
        h.push(make_entry(ScanCode(30), "ka", Some('か')));
        h.push(make_entry(ScanCode(31), "ki", Some('き')));
        h.push(make_entry(ScanCode(32), "ku", Some('く')));

        let kana = h.recent_kana(3);
        assert_eq!(kana, vec!['か', 'き', 'く']);
    }

    #[test]
    fn test_retract_last() {
        let mut h = OutputHistory::new();
        h.push(make_entry(ScanCode(30), "ka", Some('か')));
        h.push(make_entry(ScanCode(31), "ki", Some('き')));

        let retracted = h.retract_last().unwrap();
        assert_eq!(retracted.scan_code, ScanCode(31));
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn test_retract_bs_count_always_one() {
        let mut h = OutputHistory::new();
        assert_eq!(h.retract_bs_count(), 0);

        h.push(make_entry(ScanCode(30), "ka", Some('か')));
        assert_eq!(h.retract_bs_count(), 1);

        h.push(make_entry(ScanCode(31), "ki", Some('き')));
        assert_eq!(h.retract_bs_count(), 1);

        h.push(make_entry(ScanCode(32), "ku", Some('く')));
        assert_eq!(h.retract_bs_count(), 1);
    }

    #[test]
    fn test_find_action_by_scan() {
        let mut h = OutputHistory::new();
        h.push(make_entry(ScanCode(30), "ka", Some('か')));
        h.push(make_entry(ScanCode(31), "ki", Some('き')));

        let action = h.find_action_by_scan(ScanCode(30)).unwrap();
        assert!(matches!(action, KeyAction::Romaji(r) if r == "ka"));

        assert!(h.find_action_by_scan(ScanCode(99)).is_none());
    }

    #[test]
    fn test_remove_by_scan() {
        let mut h = OutputHistory::new();
        h.push(make_entry(ScanCode(30), "ka", Some('か')));
        h.push(make_entry(ScanCode(31), "ki", Some('き')));
        h.push(make_entry(ScanCode(32), "ku", Some('く')));

        let removed = h.remove_by_scan(ScanCode(31)).unwrap();
        assert_eq!(removed.romaji, "ki");

        // Remaining entries should be scan_code 30 and 32
        assert!(h.find_action_by_scan(ScanCode(30)).is_some());
        assert!(h.find_action_by_scan(ScanCode(32)).is_some());
        assert!(h.find_action_by_scan(ScanCode(31)).is_none());

        // Removing non-existent scan_code returns None
        assert!(h.remove_by_scan(ScanCode(99)).is_none());
    }

    #[test]
    fn test_display_text() {
        let mut h = OutputHistory::new();
        h.push(make_entry(ScanCode(30), "ka", Some('か')));
        h.push(OutputEntry {
            scan_code: ScanCode(50),
            romaji: "shift".to_string(),
            kana: None,
            action: KeyAction::Key(VkCode(0)), // dummy platform key
        });
        h.push(make_entry(ScanCode(31), "ki", Some('き')));

        assert_eq!(h.display_text(), "かき");
    }

    #[test]
    fn test_clear() {
        let mut h = OutputHistory::new();
        h.push(make_entry(ScanCode(30), "ka", Some('か')));
        h.push(make_entry(ScanCode(31), "ki", Some('き')));

        assert!(!h.is_empty());
        h.clear();
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
        assert!(h.find_action_by_scan(ScanCode(30)).is_none());
    }

    #[test]
    fn test_recent_kana_ordering() {
        let mut h = OutputHistory::new();
        h.push(make_entry(ScanCode(30), "a", Some('あ')));
        h.push(make_entry(ScanCode(31), "i", Some('い')));
        h.push(make_entry(ScanCode(32), "u", Some('う')));
        h.push(make_entry(ScanCode(33), "e", Some('え')));
        h.push(make_entry(ScanCode(34), "o", Some('お')));

        // recent_kana should return oldest-first order
        let kana = h.recent_kana(3);
        assert_eq!(kana, vec!['う', 'え', 'お']);
    }

    #[test]
    fn test_recent_kana_max_n() {
        let mut h = OutputHistory::new();
        h.push(make_entry(ScanCode(30), "a", Some('あ')));
        h.push(make_entry(ScanCode(31), "i", Some('い')));
        h.push(make_entry(ScanCode(32), "u", Some('う')));

        // Requesting more than available returns all
        let kana = h.recent_kana(10);
        assert_eq!(kana, vec!['あ', 'い', 'う']);

        // Requesting fewer returns only that many (most recent)
        let kana = h.recent_kana(2);
        assert_eq!(kana, vec!['い', 'う']);

        // Requesting 0 returns empty
        let kana = h.recent_kana(0);
        assert!(kana.is_empty());
    }

    // ── ADR-112 決定0: committed の上限 ──

    #[test]
    fn test_committed_capacity_bounds_growth() {
        let mut h = OutputHistory::new();
        for i in 0..(COMMITTED_CAPACITY * 3) {
            h.push(make_entry(
                ScanCode(u32::try_from(i).unwrap()),
                "a",
                Some('あ'),
            ));
        }
        assert_eq!(h.len(), COMMITTED_CAPACITY);
    }

    #[test]
    fn test_committed_capacity_does_not_affect_recent_kana_usage() {
        // NGRAM_CONTEXT_SIZE (実際の利用は数件程度) を大きく上回る件数を
        // push しても、直近の要求件数分は正しく取得できる。
        let mut h = OutputHistory::new();
        for i in 0..(COMMITTED_CAPACITY * 2) {
            h.push(make_entry(
                ScanCode(u32::try_from(i).unwrap()),
                "a",
                char::from_u32(0x3042 + u32::try_from(i % 10).unwrap()),
            ));
        }
        assert_eq!(h.recent_kana(3).len(), 3);
    }

    // ── ADR-112 決定0: pending_releases と committed の分離 ──

    #[test]
    fn test_remove_by_scan_does_not_shrink_recent_kana() {
        // KeyUp 整合性のための remove_by_scan は committed（n-gram 文脈）に
        // 影響してはならない。分離前はこれが同一 Vec を共有していたため、
        // remove_by_scan が到達可能になった瞬間に n-gram 文脈が痩せるという
        // 副作用があった（ADR-112 決定0が解消する対象）。
        let mut h = OutputHistory::new();
        h.push(make_entry(ScanCode(30), "ka", Some('か')));
        h.push(make_entry(ScanCode(31), "ki", Some('き')));
        h.push(make_entry(ScanCode(32), "ku", Some('く')));

        assert_eq!(h.recent_kana(3), vec!['か', 'き', 'く']);
        h.remove_by_scan(ScanCode(31));
        assert_eq!(
            h.recent_kana(3),
            vec!['か', 'き', 'く'],
            "remove_by_scan must not shrink n-gram context"
        );
    }

    #[test]
    fn test_retract_and_record_upserts_pending_release_for_same_scan_code() {
        let mut h = OutputHistory::new();
        h.push(OutputEntry {
            scan_code: ScanCode(30),
            romaji: "u".to_string(),
            kana: Some('う'),
            action: KeyAction::Romaji("u".to_string()),
        });
        h.retract_and_record(OutputEntry {
            scan_code: ScanCode(30),
            romaji: "vu".to_string(),
            kana: Some('ゔ'),
            action: KeyAction::Romaji("vu".to_string()),
        });

        // pending_releases に scan_code 30 のエントリが1件だけ、内容は訂正後のもの
        let action = h.find_action_by_scan(ScanCode(30)).unwrap();
        assert!(matches!(action, KeyAction::Romaji(r) if r == "vu"));
        let removed = h.remove_by_scan(ScanCode(30)).unwrap();
        assert_eq!(removed.romaji, "vu");
        assert!(
            h.remove_by_scan(ScanCode(30)).is_none(),
            "should not have a stale duplicate entry left over"
        );

        // committed 側は訂正後の内容で1件のまま
        assert_eq!(h.len(), 1);
        assert_eq!(h.recent_kana(1), vec!['ゔ']);
    }
}
