//! 統合イベントジャーナル: エンジン + IME 両イベントを時系列で記録するリングバッファ。
//!
//! ダンプトリガー（Alt+変換→Alt+無変換 を 2 回連続）で
//! `%TEMP%/awase_journal_<tick_ms>.json` に書き出す。
//!
//! タイムスタンプは `quanta::Clock` 由来（注入可能、テスト時はモック化可能）。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

pub use crate::journal_policy::LaneKind;
use crate::journal_policy::{select_tail_within_budget, BudgetItem};

pub const DEFAULT_CAPACITY: usize = 2048;
pub const STATE_LANE_CAPACITY: usize = crate::journal_policy::lane_capacity(LaneKind::State);
pub const TIMING_LANE_CAPACITY: usize = crate::journal_policy::lane_capacity(LaneKind::Timing);
pub const ACTUATION_LANE_CAPACITY: usize =
    crate::journal_policy::lane_capacity(LaneKind::Actuation);
pub const KEY_INPUT_LANE_CAPACITY: usize = crate::journal_policy::lane_capacity(LaneKind::KeyInput);

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
    pub injected: bool,
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
            injected: event.injected,
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
    /// `ImeEvent::FocusChanged` と同じタイミングで、reducer に渡さない診断専用の
    /// アプリ名付きフォーカス遷移を記録する。
    FocusTransition {
        changed: crate::focus::current::FocusChangedAxes,
        from: Option<FocusEndpoint>,
        to: FocusEndpoint,
        dwell_ms: u64,
        profile: String,
    },
    /// GJI FSM の入力イベント/タイムアウト前後の状態。
    GjiFsmTransition {
        trigger: String,
        state_before: String,
        state_after: String,
    },
    /// TSF/GJI probe の開始。
    TsfProbeStarted {
        source: String,
        cold_seq: u64,
        gji_state: String,
        consecutive_at_start: u32,
    },
    /// TSF/GJI probe の完了・中断・学習完了。
    TsfProbeCompleted {
        outcome: String,
        cold_seq: Option<u64>,
        elapsed_ms: u64,
        tick_count: u32,
        gji_state: String,
    },
    /// literal-detect（raw TSF literal 判定）1 回分の結果。
    LiteralDetect {
        record: crate::tsf::literal_facts::LiteralDetectRecord,
        suppressed_confirms: u16,
        since_vk_sent_ms: u64,
    },
    /// `elapsed_ms` / OS tick / hook timestamp の対応を取るためのアンカー。
    ClockAnchor { tick_ms: u64, hook_us: u64 },
    /// 添付用 capped JSON が古い entry を落としたことを示す合成ヘッダ。
    DumpTruncated {
        budget_bytes: usize,
        total_entries: usize,
        emitted_entries: usize,
        dropped_state: usize,
        dropped_timing: usize,
        dropped_actuation: usize,
        dropped_key_input: usize,
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

#[derive(Debug, Serialize)]
pub struct FocusEndpoint {
    pub hwnd: crate::state::ime_event::HwndId,
    pub pid: u32,
    pub process_name: String,
    pub class_name: String,
    pub app_kind: String,
    pub focus_kind: String,
}

#[derive(Debug)]
pub struct CappedJson {
    pub json: String,
    pub total_entries: usize,
    pub emitted_entries: usize,
    pub dropped_by_lane: [(LaneKind, usize); 4],
}

// ── UnifiedJournal ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct LaneCapacities {
    state: usize,
    timing: usize,
    actuation: usize,
    key_input: usize,
}

impl LaneCapacities {
    const DEFAULT: Self = Self {
        state: STATE_LANE_CAPACITY,
        timing: TIMING_LANE_CAPACITY,
        actuation: ACTUATION_LANE_CAPACITY,
        key_input: KEY_INPUT_LANE_CAPACITY,
    };

    const fn uniform(capacity: usize) -> Self {
        Self {
            state: capacity,
            timing: capacity,
            actuation: capacity,
            key_input: capacity,
        }
    }
}

#[derive(Debug)]
struct JournalLane {
    buffer: VecDeque<JournalEnvelope>,
    capacity: usize,
}

impl JournalLane {
    fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn push(&mut self, envelope: JournalEnvelope) {
        if self.capacity == 0 {
            return;
        }
        if self.buffer.len() == self.capacity {
            if self
                .buffer
                .front()
                .is_some_and(|front| envelope.seq < front.seq)
            {
                return;
            }
            self.buffer.pop_front();
        }
        let pos = self
            .buffer
            .iter()
            .rposition(|entry| entry.seq < envelope.seq)
            .map_or(0, |index| index + 1);
        self.buffer.insert(pos, envelope);
    }
}

#[derive(Debug)]
struct JournalLanes {
    state: JournalLane,
    timing: JournalLane,
    actuation: JournalLane,
    key_input: JournalLane,
}

impl JournalLanes {
    fn new(capacities: LaneCapacities) -> Self {
        Self {
            state: JournalLane::new(capacities.state),
            timing: JournalLane::new(capacities.timing),
            actuation: JournalLane::new(capacities.actuation),
            key_input: JournalLane::new(capacities.key_input),
        }
    }
}

impl JournalEntry {
    const fn lane_kind(&self) -> LaneKind {
        match self {
            Self::ImeEvent { .. }
            | Self::ImeOpenApplied { .. }
            | Self::FocusTransition { .. }
            | Self::ClockAnchor { .. }
            | Self::DumpTruncated { .. }
            | Self::DumpTriggered => LaneKind::State,
            Self::GjiFsmTransition { .. }
            | Self::TsfProbeStarted { .. }
            | Self::TsfProbeCompleted { .. }
            | Self::LiteralDetect { .. } => LaneKind::Timing,
            Self::ImeActuation { .. } | Self::ConvClassifyCall { .. } | Self::TimerFired { .. } => {
                LaneKind::Actuation
            }
            Self::KeyInput { .. } => LaneKind::KeyInput,
        }
    }
}

/// 統合イベントジャーナル。
///
/// タイムスタンプは注入された `quanta::Clock` で自己採取するため、
/// 呼び出し側は時刻を渡す必要がない。テスト時は `new_with_clock` でモック化可能。
pub struct UnifiedJournal {
    clock: quanta::Clock,
    start: quanta::Instant,
    lanes: JournalLanes,
    next_seq: Arc<AtomicU64>,
}

#[derive(Debug, Clone)]
pub struct JournalStamper {
    clock: quanta::Clock,
    start: quanta::Instant,
    next_seq: Arc<AtomicU64>,
}

impl JournalStamper {
    #[must_use]
    pub fn stamp(&self, entry: JournalEntry) -> JournalEnvelope {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let elapsed_ms = (self.clock.now() - self.start).as_millis() as u64;
        JournalEnvelope {
            seq,
            elapsed_ms,
            entry,
        }
    }
}

impl std::fmt::Debug for UnifiedJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnifiedJournal")
            .field("state_len", &self.lanes.state.buffer.len())
            .field("timing_len", &self.lanes.timing.buffer.len())
            .field("actuation_len", &self.lanes.actuation.buffer.len())
            .field("key_input_len", &self.lanes.key_input.buffer.len())
            .field("next_seq", &self.next_seq.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl UnifiedJournal {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let clock = quanta::Clock::new();
        let capacities = if capacity == DEFAULT_CAPACITY {
            LaneCapacities::DEFAULT
        } else {
            LaneCapacities::uniform(capacity)
        };
        Self::new_with_clock_and_capacities(clock, capacities)
    }

    /// テスト用: 外部から `quanta::Clock` を注入してジャーナルを作成する。
    #[must_use]
    pub fn new_with_clock(capacity: usize, clock: quanta::Clock) -> Self {
        let capacities = if capacity == DEFAULT_CAPACITY {
            LaneCapacities::DEFAULT
        } else {
            LaneCapacities::uniform(capacity)
        };
        Self::new_with_clock_and_capacities(clock, capacities)
    }

    fn new_with_clock_and_capacities(clock: quanta::Clock, capacities: LaneCapacities) -> Self {
        let start = clock.now();
        Self {
            clock,
            start,
            lanes: JournalLanes::new(capacities),
            next_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    #[must_use]
    pub fn stamper(&self) -> JournalStamper {
        JournalStamper {
            clock: self.clock.clone(),
            start: self.start,
            next_seq: Arc::clone(&self.next_seq),
        }
    }

    /// エントリを記録する。タイムスタンプは内部クロックで自己採取。容量超過時はレーン内の最古を破棄。
    pub fn record(&mut self, entry: JournalEntry) -> u64 {
        let envelope = self.stamper().stamp(entry);
        let seq = envelope.seq;
        self.absorb(envelope);
        seq
    }

    /// 発生時に stamp 済みの envelope をレーンへ収める。
    pub fn absorb(&mut self, envelope: JournalEnvelope) {
        let lane = envelope.entry.lane_kind();
        match lane {
            LaneKind::State => self.lanes.state.push(envelope),
            LaneKind::Timing => self.lanes.timing.push(envelope),
            LaneKind::Actuation => self.lanes.actuation.push(envelope),
            LaneKind::KeyInput => self.lanes.key_input.push(envelope),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lanes.state.buffer.len()
            + self.lanes.timing.buffer.len()
            + self.lanes.actuation.buffer.len()
            + self.lanes.key_input.buffer.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 全エントリを JSON 文字列にシリアライズして返す。
    pub fn to_json(&self) -> Result<String, DumpError> {
        let entries = self.entries_by_seq();
        Ok(serde_json::to_string_pretty(&entries)?)
    }

    pub fn to_json_capped(&self, max_bytes: usize) -> Result<CappedJson, DumpError> {
        let entries = self.entries_by_seq();
        let serialized: Vec<SerializedEnvelope> = entries
            .iter()
            .map(|envelope| {
                let json = serde_json::to_string(envelope)?;
                Ok(SerializedEnvelope {
                    seq: envelope.seq,
                    lane: envelope.entry.lane_kind(),
                    json,
                })
            })
            .collect::<Result<_, serde_json::Error>>()?;
        let total_entries = serialized.len();
        let total_json_bytes = json_array_len(serialized.iter().map(|e| e.json.len()));
        if total_json_bytes <= max_bytes {
            return Ok(CappedJson {
                json: join_json_array(serialized.iter().map(|e| e.json.as_str())),
                total_entries,
                emitted_entries: total_entries,
                dropped_by_lane: lane_counts(),
            });
        }

        let mut header_len = 0usize;
        let mut selected = Vec::new();
        for _ in 0..4 {
            let payload_budget = max_bytes.saturating_sub(header_len);
            let items: Vec<BudgetItem> = serialized
                .iter()
                .map(|e| BudgetItem {
                    seq: e.seq,
                    lane: e.lane,
                    bytes: e.json.len(),
                })
                .collect();
            let next_selected = select_tail_within_budget(&items, payload_budget);
            let dropped = dropped_by_lane(&serialized, &next_selected);
            let next_header_len = truncation_header_json(
                selected_min_seq(&serialized, &next_selected).unwrap_or(0),
                max_bytes,
                total_entries,
                next_selected.len(),
                dropped,
            )?
            .len()
                + usize::from(!next_selected.is_empty());
            if next_selected == selected && next_header_len == header_len {
                break;
            }
            selected = next_selected;
            header_len = next_header_len;
        }

        let mut selected_final = selected;
        let dropped = dropped_by_lane(&serialized, &selected_final);
        let mut parts = Vec::with_capacity(selected_final.len() + 1);
        let header = truncation_header_json(
            selected_min_seq(&serialized, &selected_final).unwrap_or(0),
            max_bytes,
            total_entries,
            selected_final.len(),
            dropped,
        )?;
        let header_included = header.len() + 2 <= max_bytes;
        if header_included {
            parts.push(header);
        }
        parts.extend(
            selected_final
                .iter()
                .map(|&index| serialized[index].json.clone()),
        );
        let mut json = join_json_array(parts.iter().map(String::as_str));
        while json.len() > max_bytes && parts.len() > 1 {
            let remove_at = usize::from(header_included);
            parts.remove(remove_at);
            selected_final.remove(0);
            json = join_json_array(parts.iter().map(String::as_str));
        }
        let dropped = dropped_by_lane(&serialized, &selected_final);

        Ok(CappedJson {
            json,
            total_entries,
            emitted_entries: selected_final.len(),
            dropped_by_lane: dropped,
        })
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

    /// `%TEMP%/awase_journal_<tick_ms>.json` に添付用 capped JSON を書き出す。
    pub fn dump_to_file_capped(&self, max_bytes: usize) -> Result<std::path::PathBuf, DumpError> {
        let tick = crate::hook::current_tick_ms();
        let path = std::env::temp_dir().join(format!("awase_journal_{tick}.json"));
        let capped = self.to_json_capped(max_bytes)?;
        std::fs::write(&path, &capped.json).map_err(|source| DumpError::Write {
            path: path.clone(),
            source,
        })?;
        Ok(path)
    }

    fn entries_by_seq(&self) -> Vec<&JournalEnvelope> {
        let mut entries: Vec<&JournalEnvelope> = self
            .lanes
            .state
            .buffer
            .iter()
            .chain(self.lanes.timing.buffer.iter())
            .chain(self.lanes.actuation.buffer.iter())
            .chain(self.lanes.key_input.buffer.iter())
            .collect();
        entries.sort_by_key(|entry| entry.seq);
        entries
    }
}

struct SerializedEnvelope {
    seq: u64,
    lane: LaneKind,
    json: String,
}

fn json_array_len(item_lens: impl Iterator<Item = usize>) -> usize {
    let mut len = 2;
    let mut first = true;
    for item_len in item_lens {
        if !first {
            len += 1;
        }
        len += item_len;
        first = false;
    }
    len
}

fn join_json_array<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let mut json = String::from("[");
    for (index, item) in items.enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str(item);
    }
    json.push(']');
    json
}

fn selected_min_seq(serialized: &[SerializedEnvelope], selected: &[usize]) -> Option<u64> {
    selected.iter().map(|&index| serialized[index].seq).min()
}

fn dropped_by_lane(
    serialized: &[SerializedEnvelope],
    selected: &[usize],
) -> [(LaneKind, usize); 4] {
    let mut total = lane_counts();
    let mut emitted = lane_counts();
    for item in serialized {
        *count_for_lane(&mut total, item.lane) += 1;
    }
    for &index in selected {
        *count_for_lane(&mut emitted, serialized[index].lane) += 1;
    }
    [
        (LaneKind::State, total[0].1.saturating_sub(emitted[0].1)),
        (LaneKind::Timing, total[1].1.saturating_sub(emitted[1].1)),
        (LaneKind::Actuation, total[2].1.saturating_sub(emitted[2].1)),
        (LaneKind::KeyInput, total[3].1.saturating_sub(emitted[3].1)),
    ]
}

fn lane_counts() -> [(LaneKind, usize); 4] {
    [
        (LaneKind::State, 0),
        (LaneKind::Timing, 0),
        (LaneKind::Actuation, 0),
        (LaneKind::KeyInput, 0),
    ]
}

fn count_for_lane(counts: &mut [(LaneKind, usize); 4], lane: LaneKind) -> &mut usize {
    &mut counts
        .iter_mut()
        .find(|(candidate, _)| *candidate == lane)
        .expect("all journal lanes are represented")
        .1
}

fn truncation_header_json(
    seq: u64,
    budget_bytes: usize,
    total_entries: usize,
    emitted_entries: usize,
    dropped: [(LaneKind, usize); 4],
) -> Result<String, DumpError> {
    let envelope = JournalEnvelope {
        seq,
        elapsed_ms: 0,
        entry: JournalEntry::DumpTruncated {
            budget_bytes,
            total_entries,
            emitted_entries,
            dropped_state: dropped[0].1,
            dropped_timing: dropped[1].1,
            dropped_actuation: dropped[2].1,
            dropped_key_input: dropped[3].1,
        },
    };
    Ok(serde_json::to_string(&envelope)?)
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
    use std::sync::Arc;

    // ── DumpTriggerTracker ────────────────────────────────────────────────

    fn mock_tracker() -> (DumpTriggerTracker, Arc<quanta::Mock>) {
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

    fn mock_journal() -> (UnifiedJournal, Arc<quanta::Mock>) {
        let (clock, mock) = quanta::Clock::mock();
        (UnifiedJournal::new_with_clock(10, clock), mock)
    }

    fn make_state_entry() -> JournalEntry {
        JournalEntry::ImeEvent {
            event: crate::state::ime_event::ImeEvent::PanicReset { target: true },
        }
    }

    fn make_timing_entry() -> JournalEntry {
        JournalEntry::GjiFsmTransition {
            trigger: "test".to_owned(),
            state_before: "before".to_owned(),
            state_after: "after".to_owned(),
        }
    }

    fn make_key_input_entry() -> JournalEntry {
        JournalEntry::KeyInput {
            event: KeyEventSummary {
                vk_code: 65,
                scan_code: 30,
                is_down: true,
                injected: true,
                timestamp_us: 123,
                key_class: "Char",
                alt: false,
                ctrl: false,
                shift: false,
            },
            state_before: "engine-before".to_owned(),
            state_after: "engine-after".to_owned(),
            decision: DecisionKind::PassThrough,
        }
    }

    #[test]
    fn journal_record_increments_seq() {
        let (mut j, _mock) = mock_journal();
        let s0 = j.record(make_state_entry());
        let s1 = j.record(make_timing_entry());
        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
    }

    #[test]
    fn journal_elapsed_ms_advances_with_clock() {
        let (mut j, mock) = mock_journal();
        j.record(make_state_entry());
        mock.increment(Duration::from_millis(42));
        j.record(make_state_entry());
        let elapsed: Vec<u64> = j.lanes.state.buffer.iter().map(|e| e.elapsed_ms).collect();
        assert_eq!(elapsed[0], 0);
        assert_eq!(elapsed[1], 42);
    }

    #[test]
    fn journal_lane_capacity_drops_oldest_per_lane() {
        let (clock, _mock) = quanta::Clock::mock();
        let mut j = UnifiedJournal::new_with_clock_and_capacities(
            clock,
            LaneCapacities {
                state: 2,
                timing: 2,
                actuation: 2,
                key_input: 2,
            },
        );
        for _ in 0..3 {
            j.record(make_state_entry());
        }
        for _ in 0..3 {
            j.record(make_key_input_entry());
        }
        assert_eq!(j.len(), 4);
        let state_seqs: Vec<u64> = j.lanes.state.buffer.iter().map(|e| e.seq).collect();
        let key_seqs: Vec<u64> = j.lanes.key_input.buffer.iter().map(|e| e.seq).collect();
        assert_eq!(state_seqs, vec![1, 2]);
        assert_eq!(key_seqs, vec![4, 5]);
    }

    #[test]
    fn journal_to_json_merges_lanes_by_seq() {
        let (mut j, _mock) = mock_journal();
        j.record(make_state_entry());
        j.record(make_key_input_entry());
        j.record(make_timing_entry());
        let json = j.to_json().unwrap();
        let values: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        let seqs: Vec<u64> = values.iter().map(|v| v["seq"].as_u64().unwrap()).collect();
        assert_eq!(seqs, vec![0, 1, 2]);
    }

    #[test]
    fn journal_to_json_produces_array() {
        let (mut j, _mock) = mock_journal();
        j.record(make_state_entry());
        let json = j.to_json().unwrap();
        assert!(json.starts_with('['));
        assert!(json.contains("ImeEvent"));
        assert!(json.contains("elapsed_ms"));
    }

    #[test]
    fn journal_to_json_capped_keeps_newer_tail_and_valid_json() {
        let (mut j, _mock) = mock_journal();
        for _ in 0..20 {
            j.record(make_state_entry());
        }
        let capped = j.to_json_capped(700).unwrap();
        assert!(capped.json.len() <= 700);
        let values: Vec<serde_json::Value> = serde_json::from_str(&capped.json).unwrap();
        let seqs: Vec<u64> = values.iter().map(|v| v["seq"].as_u64().unwrap()).collect();
        assert!(seqs.contains(&19));
        assert!(!seqs.contains(&0));
        assert!(values
            .first()
            .is_some_and(|v| v["entry"]["type"] == "DumpTruncated"));
    }

    #[test]
    fn absorb_orders_delayed_envelopes_by_original_seq() {
        let (clock, _mock) = quanta::Clock::mock();
        let mut j = UnifiedJournal::new_with_clock_and_capacities(
            clock,
            LaneCapacities {
                state: 4,
                timing: 4,
                actuation: 4,
                key_input: 4,
            },
        );
        j.absorb(JournalEnvelope {
            seq: 2,
            elapsed_ms: 20,
            entry: make_state_entry(),
        });
        j.absorb(JournalEnvelope {
            seq: 1,
            elapsed_ms: 10,
            entry: make_state_entry(),
        });
        let seqs: Vec<u64> = j.lanes.state.buffer.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![1, 2]);
    }

    #[test]
    fn absorb_drops_delayed_envelope_that_is_older_than_full_lane() {
        let (clock, _mock) = quanta::Clock::mock();
        let mut j = UnifiedJournal::new_with_clock_and_capacities(
            clock,
            LaneCapacities {
                state: 2,
                timing: 2,
                actuation: 2,
                key_input: 2,
            },
        );
        j.absorb(JournalEnvelope {
            seq: 10,
            elapsed_ms: 10,
            entry: make_state_entry(),
        });
        j.absorb(JournalEnvelope {
            seq: 11,
            elapsed_ms: 11,
            entry: make_state_entry(),
        });
        j.absorb(JournalEnvelope {
            seq: 9,
            elapsed_ms: 9,
            entry: make_state_entry(),
        });
        let seqs: Vec<u64> = j.lanes.state.buffer.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![10, 11]);
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
