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

/// Engine が出力した内容の履歴
///
/// 押下中キーの追跡、直近出力ローマ字/かな、出力記録を統合管理する。
/// Speculative モードの BS 回数計算、n-gram 文脈取得、KeyUp 整合性を一元管理。
#[derive(Debug, Default)]
pub struct OutputHistory {
    entries: Vec<OutputEntry>,
}

impl OutputHistory {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// 出力を記録する
    pub fn push(&mut self, entry: OutputEntry) {
        self.entries.push(entry);
    }

    /// 最後の出力を取り消す（Speculative retraction 用）
    pub fn retract_last(&mut self) -> Option<OutputEntry> {
        self.entries.pop()
    }

    /// 取り消しに必要な BS 回数
    /// 完全なローマ字は IME で 1 composition unit になるため、常に 1。
    #[must_use]
    #[allow(clippy::bool_to_int_with_if)] // usize::from(bool) is not const-stable
    pub const fn retract_bs_count(&self) -> usize {
        if self.entries.is_empty() {
            0
        } else {
            1
        }
    }

    /// n-gram 用の直近かな文字列（古い順）
    #[must_use]
    pub fn recent_kana(&self, n: usize) -> Vec<char> {
        let mut result: Vec<char> = self
            .entries
            .iter()
            .rev()
            .filter_map(|e| e.kana)
            .take(n)
            .collect();
        result.reverse();
        result
    }

    /// scan_code に対応するアクションを検索（KeyUp 用）
    #[must_use]
    pub fn find_action_by_scan(&self, scan_code: ScanCode) -> Option<&KeyAction> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.scan_code == scan_code)
            .map(|e| &e.action)
    }

    /// scan_code に対応するエントリを除去して返す（KeyUp 用）
    pub fn remove_by_scan(&mut self, scan_code: ScanCode) -> Option<OutputEntry> {
        self.entries
            .iter()
            .position(|e| e.scan_code == scan_code)
            .map(|pos| self.entries.remove(pos))
    }

    /// GUI プレビュー用: 出力テキスト
    #[must_use]
    pub fn display_text(&self) -> String {
        self.entries.iter().filter_map(|e| e.kana).collect()
    }

    /// エントリ数
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// 空かどうか
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 全エントリをクリア
    pub fn clear(&mut self) {
        self.entries.clear();
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
        assert_eq!(h.len(), 2);

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
            action: KeyAction::Key(VkCode(0x10)),
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
}
