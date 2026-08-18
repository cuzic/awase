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

const TRIGGER_WINDOW: Duration = Duration::from_secs(3);

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
    #[must_use]
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
    #[must_use]
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
    /// IME 状態変更イベント（dispatch_event 経由の全 ImeEvent）。
    ///
    /// ADR-082「決定 1」: 旧 `ImeEvent { description: String }`（`format!("{event:?}")`
    /// の自由文字列）を廃止し、実 `state::ime_event::ImeEvent` をそのまま記録する。
    /// これにより journal が「読める」だけでなく「型として取り出せる」形式になる
    /// （`source`/`target`/`confidence` 等を文字列パースなしで参照できる）。
    ImeEvent {
        event: crate::state::ime_event::ImeEvent,
    },
    /// `classify_conv_transition` への呼び出し（引数+戻り値を構造化して記録）。
    ///
    /// リプレイ回帰テスト（`tests/journal_replay.rs`）の主要な入力源。実機で
    /// ダンプしたジャーナルからこのエントリを取り出し、`tests/journals/` の
    /// フィクスチャ形式（`ConvClassifyFixture`、`state/conv_classify.rs` 参照）に
    /// 転記することで、実際に観測された入力の組合せを恒久的な回帰テストに
    /// 変換できる。
    ConvClassifyCall {
        conv: u32,
        current: awase::engine::InputModeState,
        is_cold: bool,
        effective_open: bool,
        conv_mode_changed: bool,
        is_roman_reliable: bool,
        result: crate::state::conv_classify::ConvTransition,
    },
    /// IME actuation 試行（awase 自身の能動的訂正、drift correction 等）1回分の
    /// 構造化記録（ADR-082 Phase 0.5）。
    ///
    /// `ImeEvent { description: String }` の自由文字列と違い、出所（`origin.source`、
    /// actuation は常に `EventSource::SelfActuated`）・世代（`origin.epoch`）・目標値
    /// （`target`）・feedback 方針（`policy`）・累積試行回数（`attempts`）・判定
    /// （`action`）を型として保持する。これにより「誰が・どの世代の要求として・何回目に
    /// 送ったか」を後から型で取り出せる（BUG-43 の無限再送が試行回数で有界化されている
    /// ことの検証など）。
    ///
    /// ペイロード `ActuationRecord`（`state/ime_actuation.rs`）は `state` 層に定義があり、
    /// `#[cfg(windows)]` な本モジュールに依存せず Linux のリプレイテストからも同じ型で
    /// 構築・検証できる。リプレイは `tests/drift_correction_replay.rs` が
    /// `DriftCorrectionFixture` 経由で行う。`ActuationRecord` は書き出し用に `Serialize`
    /// のみ（`origin` が `&'static str` を含み `Deserialize` 不可のため、fixture 側は
    /// `epoch` のみ保存し `strategy` を `policy` から再構築する）。
    ImeActuation {
        record: crate::state::ime_actuation::ActuationRecord,
    },
    /// IME open/close 適用の完了（ADR-086 §4 INV-18、Phase 3 item 2）。
    ///
    /// `record_ime_apply_result` は `generation.is_some()` のときだけ
    /// `ImeEvent::from_apply_outcome`（`ImeEvent` 経由で `JournalEntry::ImeEvent` に
    /// 記録される）を dispatch する。force 系の適用（force-ON・bootstrap・drift
    /// correction）は generation を持たずこの経路を通らないため、`reason` を
    /// 一意に journal へ残す唯一の場所として `Runtime::on_ime_apply_complete` に
    /// 本エントリを追加した。
    ImeOpenApplied {
        open: bool,
        outcome: awase::platform::ImeOpenOutcome,
        reason: crate::state::ime_event::OpenApplyReason,
    },
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
            .finish_non_exhaustive()
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
            .finish_non_exhaustive()
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
    pub const fn with_clock(clock: quanta::Clock) -> Self {
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
        const VK_CONVERT: u16 = crate::vk::VK_CONVERT.0;
        const VK_NONCONVERT: u16 = crate::vk::VK_NONCONVERT.0;

        let now = self.clock.now();

        if let Some(last) = self.last_instant {
            if (now - last) > TRIGGER_WINDOW {
                self.step = 0;
            }
        }

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

    fn mock_tracker() -> (DumpTriggerTracker, std::sync::Arc<quanta::Mock>) {
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

    fn mock_journal() -> (UnifiedJournal, std::sync::Arc<quanta::Mock>) {
        let (clock, mock) = quanta::Clock::mock();
        (UnifiedJournal::new_with_clock(10, clock), mock)
    }

    fn make_entry() -> JournalEntry {
        JournalEntry::ImeEvent {
            event: crate::state::ime_event::ImeEvent::PanicReset { target: true },
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

    #[test]
    fn ime_actuation_entry_serializes_structured_origin() {
        use crate::state::event_origin::Generation;
        use crate::state::ime_actuation::{actuation_origin, ActuationRecord, FeedbackPolicy};

        let policy = FeedbackPolicy::Blind {
            max_attempts: 5,
            backoff: Duration::from_millis(400),
        };
        let (mut j, _mock) = mock_journal();
        // attempts=2 < max_attempts=5 なので action は Send に導出される。
        j.record(JournalEntry::ImeActuation {
            record: ActuationRecord::new(
                actuation_origin(policy, Generation::new(2)),
                false,
                policy,
                2,
            ),
        });
        let json = j.to_json().unwrap();
        // 自由文字列ではなく構造化された出所・世代・判定が型として書き出される。
        assert!(json.contains("ImeActuation"));
        assert!(json.contains("SelfActuated"));
        assert!(json.contains("drift_correction_blind"));
        assert!(json.contains("\"action\": \"Send\""));
    }
}
