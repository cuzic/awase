//! 統合イベントジャーナル: エンジン + IME 両イベントを時系列で記録するリングバッファ。
//!
//! ダンプトリガー（Alt+変換→Alt+無変換 を 2 回連続）で
//! `%TEMP%/awase_journal_<tick_ms>.json` に書き出す。

use std::collections::VecDeque;

use serde::Serialize;

pub const DEFAULT_CAPACITY: usize = 2048;

// ── JournalEntry ─────────────────────────────────────────────────────────────

/// キーイベントの軽量サマリ（serde 対応）
#[derive(Debug, Serialize)]
pub struct KeyEventSummary {
    pub vk_code: u16,
    pub scan_code: u32,
    pub is_down: bool,
    pub timestamp_us: u64,
    pub key_class: &'static str,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

impl KeyEventSummary {
    pub fn from_raw(event: &awase::types::RawKeyEvent) -> Self {
        use awase::types::{KeyClassification, KeyEventType};
        Self {
            vk_code: event.vk_code.0,
            scan_code: event.scan_code.0,
            is_down: matches!(event.event_type, KeyEventType::KeyDown),
            timestamp_us: event.timestamp,
            key_class: match event.key_classification {
                KeyClassification::Char => "Char",
                KeyClassification::LeftThumb => "LeftThumb",
                KeyClassification::RightThumb => "RightThumb",
                KeyClassification::Passthrough => "Passthrough",
            },
            alt: event.modifier_snapshot.alt,
            ctrl: event.modifier_snapshot.ctrl,
            shift: event.modifier_snapshot.shift,
        }
    }
}

/// `Decision` の種別サマリ
#[derive(Debug, Serialize)]
#[serde(tag = "kind")]
pub enum DecisionKind {
    PassThrough,
    PassThroughWith { effect_count: usize },
    Consume { effect_count: usize },
}

impl DecisionKind {
    pub fn from_decision(decision: &awase::engine::Decision) -> Self {
        use awase::engine::Decision;
        match decision {
            Decision::PassThrough => Self::PassThrough,
            Decision::PassThroughWith { effects } => Self::PassThroughWith {
                effect_count: effects.len(),
            },
            Decision::Consume { effects } => Self::Consume {
                effect_count: effects.len(),
            },
        }
    }
}

/// ジャーナルに記録するイベントの種別
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum JournalEntry {
    /// エンジンのキー入力処理（on_input）
    KeyInput {
        event: KeyEventSummary,
        state_before: String,
        state_after: String,
        decision: DecisionKind,
    },
    /// エンジンのタイマー処理（on_timeout）
    TimerFired {
        timer_id: usize,
        state_before: String,
        state_after: String,
    },
    /// IME 状態変更イベント（dispatch_event 経由の全 ImeEvent）
    ImeEvent { description: String },
    /// ダンプトリガー発動
    DumpTriggered,
}

// ── JournalEnvelope ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct JournalEnvelope {
    pub seq: u64,
    pub tick_ms: u64,
    pub entry: JournalEntry,
}

// ── UnifiedJournal ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct UnifiedJournal {
    buffer: VecDeque<JournalEnvelope>,
    next_seq: u64,
    capacity: usize,
}

impl UnifiedJournal {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            next_seq: 0,
            capacity,
        }
    }

    /// エントリを記録する。容量超過時は最古エントリを破棄。
    pub fn record(&mut self, entry: JournalEntry, tick_ms: u64) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;

        if self.buffer.len() == self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(JournalEnvelope { seq, tick_ms, entry });
        seq
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// 全エントリを JSON 文字列にシリアライズして返す。
    #[must_use]
    pub fn to_json(&self) -> String {
        let entries: Vec<&JournalEnvelope> = self.buffer.iter().collect();
        serde_json::to_string_pretty(&entries)
            .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    /// `%TEMP%/awase_journal_<tick_ms>.json` に書き出す。
    pub fn dump_to_file(&self) -> std::io::Result<std::path::PathBuf> {
        let tick = crate::hook::current_tick_ms();
        let path = std::env::temp_dir().join(format!("awase_journal_{tick}.json"));
        std::fs::write(&path, self.to_json())?;
        Ok(path)
    }
}

impl Default for UnifiedJournal {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

// ── DumpTriggerTracker ────────────────────────────────────────────────────────

/// Alt+変換 → Alt+無変換 を 2 回連続で検出するトラッカー。
///
/// ステップ: 0=idle → 1=Alt+変換① → 2=Alt+無変換① → 3=Alt+変換② → 0(+dump発動)
#[derive(Debug)]
pub struct DumpTriggerTracker {
    step: u8,
    last_ms: u64,
}

impl DumpTriggerTracker {
    const WINDOW_MS: u64 = 3000;

    #[must_use]
    pub const fn new() -> Self {
        Self { step: 0, last_ms: 0 }
    }

    /// キーダウンを記録し、パターン完成なら `true` を返す。
    ///
    /// `vk`: VkCode の raw 値, `alt`: Alt 修飾キー状態, `now_ms`: GetTickCount64 相当
    pub fn push(&mut self, vk: u16, alt: bool, now_ms: u64) -> bool {
        if now_ms.saturating_sub(self.last_ms) > Self::WINDOW_MS {
            self.step = 0;
        }

        const VK_CONVERT: u16 = 0x1C;
        const VK_NONCONVERT: u16 = 0x1D;

        if !alt {
            self.step = 0;
            return false;
        }

        self.step = match (self.step, vk) {
            (0, VK_CONVERT) => 1,
            (1, VK_NONCONVERT) => 2,
            (2, VK_CONVERT) => 3,
            (3, VK_NONCONVERT) => {
                self.step = 0;
                self.last_ms = now_ms;
                return true;
            }
            _ => 0,
        };
        self.last_ms = now_ms;
        false
    }
}

impl Default for DumpTriggerTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── DumpTriggerTracker ────────────────────────────────────────────────

    fn tracker() -> DumpTriggerTracker {
        DumpTriggerTracker::new()
    }

    #[test]
    fn dump_trigger_fires_on_complete_sequence() {
        let mut t = tracker();
        assert!(!t.push(0x1C, true, 100)); // Alt+変換①
        assert!(!t.push(0x1D, true, 200)); // Alt+無変換①
        assert!(!t.push(0x1C, true, 300)); // Alt+変換②
        assert!(t.push(0x1D, true, 400));  // Alt+無変換② → 発動
    }

    #[test]
    fn dump_trigger_requires_alt() {
        let mut t = tracker();
        assert!(!t.push(0x1C, false, 100)); // 変換 (Alt なし) → リセット
        assert!(!t.push(0x1D, true, 200));
        assert!(!t.push(0x1C, true, 300));
        assert!(!t.push(0x1D, true, 400));  // step が途中でリセットされているので完成しない
    }

    #[test]
    fn dump_trigger_resets_on_timeout() {
        let mut t = tracker();
        assert!(!t.push(0x1C, true, 100));
        assert!(!t.push(0x1D, true, 200));
        assert!(!t.push(0x1C, true, 200 + DumpTriggerTracker::WINDOW_MS + 1)); // タイムアウト
        assert!(!t.push(0x1D, true, 200 + DumpTriggerTracker::WINDOW_MS + 2));
    }

    #[test]
    fn dump_trigger_resets_on_wrong_key() {
        let mut t = tracker();
        assert!(!t.push(0x1C, true, 100));
        assert!(!t.push(0x1C, true, 200)); // 変換→変換 は不正
        assert!(!t.push(0x1D, true, 300));
        assert!(!t.push(0x1C, true, 400));
        assert!(!t.push(0x1D, true, 500)); // 4ステップ目だが step がリセット済み
    }

    // ── UnifiedJournal ────────────────────────────────────────────────────

    fn make_entry() -> JournalEntry {
        JournalEntry::ImeEvent {
            description: "test".to_owned(),
        }
    }

    #[test]
    fn journal_record_increments_seq() {
        let mut j = UnifiedJournal::new(10);
        let s0 = j.record(make_entry(), 0);
        let s1 = j.record(make_entry(), 1);
        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
    }

    #[test]
    fn journal_capacity_drops_oldest() {
        let mut j = UnifiedJournal::new(3);
        for i in 0..5u64 {
            j.record(make_entry(), i);
        }
        assert_eq!(j.len(), 3);
        let seqs: Vec<u64> = j.buffer.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![2, 3, 4]);
    }

    #[test]
    fn journal_to_json_produces_array() {
        let mut j = UnifiedJournal::new(10);
        j.record(make_entry(), 0);
        let json = j.to_json();
        assert!(json.starts_with('['));
        assert!(json.contains("ImeEvent"));
    }
}
