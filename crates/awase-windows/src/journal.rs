//! 統合イベントジャーナル: エンジン + IME 両イベントを時系列で記録するリングバッファ。
//!
//! ダンプトリガー（Alt+変換→Alt+無変換 を 2 回連続）で
//! `%TEMP%/awase_journal_<tick_ms>.json` に書き出す。
//!
//! タイムスタンプは `quanta::Clock` 由来（注入可能、テスト時はモック化可能）。

use std::collections::VecDeque;
use std::time::Duration;

use serde::Serialize;

pub const DEFAULT_CAPACITY: usize = 2048;

const TRIGGER_WINDOW: Duration = Duration::from_millis(3000);

// ── DumpError ─────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum DumpError {
    #[error("シリアライズ失敗: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("ファイル書き込み失敗 {path}: {source}")]
    Write {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

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
    /// ジャーナル作成からの経過ミリ秒（quanta::Clock 由来）
    pub elapsed_ms: u64,
    pub entry: JournalEntry,
}

// ── UnifiedJournal ────────────────────────────────────────────────────────────

/// 統合イベントジャーナル。
///
/// タイムスタンプは注入された `quanta::Clock` で自己採取するため、
/// 呼び出し側は時刻を渡す必要がない。テスト時は `new_with_clock` でモック化可能。
pub struct UnifiedJournal {
    clock: quanta::Clock,
    start: quanta::Instant,
    buffer: VecDeque<JournalEnvelope>,
    next_seq: u64,
    capacity: usize,
}

impl std::fmt::Debug for UnifiedJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnifiedJournal")
            .field("len", &self.buffer.len())
            .field("capacity", &self.capacity)
            .field("next_seq", &self.next_seq)
            .finish()
    }
}

impl UnifiedJournal {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let clock = quanta::Clock::new();
        let start = clock.now();
        Self {
            clock,
            start,
            buffer: VecDeque::with_capacity(capacity),
            next_seq: 0,
            capacity,
        }
    }

    /// テスト用: 外部から `quanta::Clock` を注入してジャーナルを作成する。
    #[must_use]
    pub fn new_with_clock(capacity: usize, clock: quanta::Clock) -> Self {
        let start = clock.now();
        Self {
            clock,
            start,
            buffer: VecDeque::with_capacity(capacity),
            next_seq: 0,
            capacity,
        }
    }

    /// エントリを記録する。タイムスタンプは内部クロックで自己採取。容量超過時は最古を破棄。
    pub fn record(&mut self, entry: JournalEntry) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        let elapsed_ms = (self.clock.now() - self.start).as_millis() as u64;
        if self.buffer.len() == self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(JournalEnvelope {
            seq,
            elapsed_ms,
            entry,
        });
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
    pub fn to_json(&self) -> Result<String, DumpError> {
        let entries: Vec<&JournalEnvelope> = self.buffer.iter().collect();
        Ok(serde_json::to_string_pretty(&entries)?)
    }

    /// `%TEMP%/awase_journal_<tick_ms>.json` に書き出す。
    pub fn dump_to_file(&self) -> Result<std::path::PathBuf, DumpError> {
        let tick = crate::hook::current_tick_ms();
        let path = std::env::temp_dir().join(format!("awase_journal_{tick}.json"));
        let json = self.to_json()?;
        std::fs::write(&path, &json).map_err(|source| DumpError::Write {
            path: path.clone(),
            source,
        })?;
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
/// タイムアウト判定は注入された `quanta::Clock` で行う。
/// テスト時は `with_clock` でモック化可能。
///
/// ステップ: 0=idle → 1=Alt+変換① → 2=Alt+無変換① → 3=Alt+変換② → 0(+dump発動)
pub struct DumpTriggerTracker {
    clock: quanta::Clock,
    step: u8,
    last_instant: Option<quanta::Instant>,
}

impl std::fmt::Debug for DumpTriggerTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DumpTriggerTracker")
            .field("step", &self.step)
            .finish()
    }
}

impl DumpTriggerTracker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            clock: quanta::Clock::new(),
            step: 0,
            last_instant: None,
        }
    }

    /// テスト用: 外部から `quanta::Clock` を注入してトラッカーを作成する。
    #[must_use]
    pub fn with_clock(clock: quanta::Clock) -> Self {
        Self {
            clock,
            step: 0,
            last_instant: None,
        }
    }

    /// キーダウンを記録し、パターン完成なら `true` を返す。
    ///
    /// `vk`: VkCode の raw 値, `alt`: Alt 修飾キー状態
    pub fn push(&mut self, vk: u16, alt: bool) -> bool {
        let now = self.clock.now();

        if let Some(last) = self.last_instant {
            if (now - last) > TRIGGER_WINDOW {
                self.step = 0;
            }
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
                self.last_instant = Some(now);
                return true;
            }
            _ => 0,
        };
        self.last_instant = Some(now);
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

    fn mock_tracker() -> (DumpTriggerTracker, std::sync::Arc<quanta::mock::Mock>) {
        let (clock, mock) = quanta::Clock::mock();
        (DumpTriggerTracker::with_clock(clock), mock)
    }

    #[test]
    fn dump_trigger_fires_on_complete_sequence() {
        let (mut t, mock) = mock_tracker();
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1C, true)); // Alt+変換①
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1D, true)); // Alt+無変換①
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1C, true)); // Alt+変換②
        mock.increment(Duration::from_millis(100));
        assert!(t.push(0x1D, true)); // Alt+無変換② → 発動
    }

    #[test]
    fn dump_trigger_requires_alt() {
        let (mut t, mock) = mock_tracker();
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1C, false)); // 変換 (Alt なし) → リセット
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1D, true));
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1C, true));
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1D, true)); // step がリセット済みなので完成しない
    }

    #[test]
    fn dump_trigger_resets_on_timeout() {
        let (mut t, mock) = mock_tracker();
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1C, true));
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1D, true));
        // TRIGGER_WINDOW を超える
        mock.increment(TRIGGER_WINDOW + Duration::from_millis(1));
        assert!(!t.push(0x1C, true)); // タイムアウトでリセット後の Alt+変換①
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1D, true)); // Alt+無変換①のみ（4ステップ未満）
    }

    #[test]
    fn dump_trigger_resets_on_wrong_key() {
        let (mut t, mock) = mock_tracker();
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1C, true));
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1C, true)); // 変換→変換 は不正 → リセット
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1D, true));
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1C, true));
        mock.increment(Duration::from_millis(100));
        assert!(!t.push(0x1D, true)); // step がリセット済みなので完成しない
    }

    // ── UnifiedJournal ────────────────────────────────────────────────────

    fn mock_journal() -> (UnifiedJournal, std::sync::Arc<quanta::mock::Mock>) {
        let (clock, mock) = quanta::Clock::mock();
        (UnifiedJournal::new_with_clock(10, clock), mock)
    }

    fn make_entry() -> JournalEntry {
        JournalEntry::ImeEvent {
            description: "test".to_owned(),
        }
    }

    #[test]
    fn journal_record_increments_seq() {
        let (mut j, _mock) = mock_journal();
        let s0 = j.record(make_entry());
        let s1 = j.record(make_entry());
        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
    }

    #[test]
    fn journal_elapsed_ms_advances_with_clock() {
        let (mut j, mock) = mock_journal();
        j.record(make_entry());
        mock.increment(Duration::from_millis(42));
        j.record(make_entry());
        let elapsed: Vec<u64> = j.buffer.iter().map(|e| e.elapsed_ms).collect();
        assert_eq!(elapsed[0], 0);
        assert_eq!(elapsed[1], 42);
    }

    #[test]
    fn journal_capacity_drops_oldest() {
        let (mut j, _mock) = mock_journal();
        for _ in 0..12 {
            j.record(make_entry());
        }
        assert_eq!(j.len(), 10);
        let seqs: Vec<u64> = j.buffer.iter().map(|e| e.seq).collect();
        assert_eq!(seqs[0], 2);
        assert_eq!(seqs[9], 11);
    }

    #[test]
    fn journal_to_json_produces_array() {
        let (mut j, _mock) = mock_journal();
        j.record(make_entry());
        let json = j.to_json().unwrap();
        assert!(json.starts_with('['));
        assert!(json.contains("ImeEvent"));
        assert!(json.contains("elapsed_ms"));
    }
}
