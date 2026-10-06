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

pub const DEFAULT_CAPACITY: usize = 2048;
pub const STATE_LANE_CAPACITY: usize = LaneKind::State.capacity();
pub const TIMING_LANE_CAPACITY: usize = LaneKind::Timing.capacity();
pub const ACTUATION_LANE_CAPACITY: usize = LaneKind::Actuation.capacity();
pub const KEY_INPUT_LANE_CAPACITY: usize = LaneKind::KeyInput.capacity();

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
        use awase::types::KeyEventType;
        Self {
            vk_code: event.vk_code.0,
            scan_code: event.scan_code.0,
            is_down: matches!(event.event_type, KeyEventType::KeyDown),
            injected: event.injected,
            timestamp_us: event.timestamp,
            key_class: variant_name(event.key_classification),
            alt: event.modifier_snapshot.alt,
            ctrl: event.modifier_snapshot.ctrl,
            shift: event.modifier_snapshot.shift,
        }
    }
}

/// drift correction が Blind GiveUp に到達した瞬間の診断情報。
#[derive(Debug, Clone, Serialize)]
pub struct DriftGiveUpDiagnosticRecord {
    pub desired_open: bool,
    pub observed_open: bool,
    pub drift_duration_ms: u64,
    pub observation_source: Option<crate::state::ime_event::ObservationSource>,
    pub observation_confidence: Option<crate::state::ime_event::ObservationConfidence>,
    pub sent_vk: Vec<ImeVkDiagnostic>,
    pub intent_source: Option<crate::state::ime_event::UserIntentSource>,
    pub layout_name: String,
    pub half_width_alnum_toggle_active: bool,
}

/// awase が IME 制御目的で送った VK の診断用サマリ。
#[derive(Debug, Clone, Serialize)]
pub struct ImeVkDiagnostic {
    pub vk_code: u16,
    pub kind: &'static str,
    pub source: &'static str,
}

/// low-level hook の `[hook] IME-mode` ログと同じ情報。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct HookImeModeDiagnosticRecord {
    pub vk_code: u16,
    pub is_down: bool,
    pub self_injected: bool,
    pub injected: bool,
    pub scan: u32,
    pub since_prev_ime_mode_ms: Option<u64>,
}

/// `Decision` の種別サマリ
#[derive(strum::IntoStaticStr, Debug, Serialize)]
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

/// `runtime::transport::PhysicalKeyDisposition`（`pub(crate)`）の journal 記録用
/// サマリ。`DecisionKind` と同じブリッジパターン: `JournalEntry` は `pub` だが
/// 元の型は crate 内部専用のため、公開できる形に変換して持つ。
///
/// `decision`（`DecisionKind`、engine の意味論的判断）とは独立した配送判断。
/// BUG-90（PowerToys Mouse Without Borders 使用中に「英数」キーが効かない
/// 不具合）の調査で、`decision` だけでは `VK_DBE_ALPHANUMERIC` 等の DBE
/// モードキーが `PhysicalKeyDisposition::plan` によって Suppress されたか
/// （ImmCross プロファイルの無条件 Suppress、または GJI/MS-IME 稼働時の
/// `is_dbe_mode_key_down` 条件による Suppress）が journal から見えないこと
/// が判明したため追加した。
#[derive(strum::IntoStaticStr, Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind")]
pub enum PhysicalDispositionSummary {
    /// 元の物理キーイベントをそのまま OS に通した
    Allow,
    /// 元の物理キーイベントを消費した（OS に届けない）。
    /// `reason`: "imm-cross" / "imm32-off"
    /// （`PhysicalKeyDisposition::suppress_reason` 参照）。
    Suppress { reason: &'static str },
}

impl PhysicalDispositionSummary {
    /// `PhysicalKeyDisposition::suppress_reason` の戻り値をそのまま受け取る。
    /// `Some(reason)` なら `Suppress`、`None` なら `Allow`（disposition と reason は
    /// 定義上 1:1 に決まるため、disposition 自体を別引数で渡す必要はない）。
    ///
    /// 呼び出し元（`runtime/`）は `#[cfg(windows)]` のため、同じ条件で定義する。
    #[cfg(windows)]
    #[must_use]
    pub(crate) fn new(reason: Option<&'static str>) -> Self {
        reason.map_or(Self::Allow, |reason| Self::Suppress { reason })
    }
}

/// `Output::flush_raw_tsf_literal_recovery` の結果サマリ（ADR-123）。
///
/// `DecisionKind`/`PhysicalDispositionSummary` と同じく、`output` モジュール
/// 内部の型（`output::RawRecoveryOutcome`、architecture guard
/// `output_and_tsf_production_code_do_not_reference_journal_directly` により
/// `output`/`tsf` 側からこの型を直接参照できないため）を journal 向けに
/// 変換して持つブリッジ型。`DecisionKind::from_decision`/
/// `PhysicalDispositionSummary::new` と異なりこちら向けの `from_*` は無く、
/// 変換は唯一の呼び出し元（`platform.rs::flush_raw_tsf_literal_recovery`）
/// のインライン `match` で行う——値をそのまま運ぶだけで判断ロジックを
/// 含まないため、`output` 側に変換関数を置く必要がない。
#[derive(strum::IntoStaticStr, Debug, Clone, Copy, Serialize)]
#[serde(tag = "kind")]
pub enum DeferredRecoveryOutcomeSummary {
    /// give-up 検出時と drain 処理時でフォーカス世代が変わっていたため、
    /// backspace/romaji/`pending_deferred` を丸ごと破棄した。
    ///
    /// `backs`/`romaji_present` は破棄した `RAW_TSF_LITERAL` の中身、
    /// `deferred_vk_count` は破棄した `pending_deferred` の VK 数。
    /// `deferred_vk_count` だけでは「`pending_deferred` が元々空だった」と
    /// 「そもそも何も破棄しなかった」が journal 上で区別できないため、
    /// 3つとも独立に保持する（ADR-123 `/code-review` 指摘）。
    DiscardedStale {
        backs: usize,
        romaji_present: bool,
        deferred_vk_count: usize,
    },
    /// 無関係な別の give-up 由来の GJI reinit retry が polling 中だったため、
    /// `pending_deferred` の flush を見送った（次の flush 機会に委ねる）。
    SkippedWhilePolling,
    /// `pending_deferred` を実際に flush した（0 件なら「取り残しなし」）。
    Flushed { vk_count: usize },
}

/// `send_input_safe` が送った 1 キーボードイベントの記録（不具合報告用、journal の
/// `SentInput` へ変換される）。`INPUT` の生値のうち、送信内容の再構成に要るものだけを持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SentKeyEvent {
    /// `wVk`。Unicode 送信（`KEYEVENTF_UNICODE`）では 0。
    pub vk: u16,
    /// `wScan`。Unicode 送信では UTF-16 code unit そのもの。
    pub scan: u16,
    pub up: bool,
    pub unicode: bool,
    /// `dwExtraInfo`（自己注入マーカー。どの送信経路かの識別に使う）。
    pub marker: usize,
}

/// [`JournalEntry::SentInput`] の 1 イベント。[`SentKeyEvent`] の書き出し用の形で、
/// 1 報告に数千件載るため、既定値のフィールドは出さずに JSON を小さく保つ。
#[derive(Debug, Serialize)]
pub struct SentKeyEventSummary {
    /// `wVk`。Unicode 送信では 0。
    pub vk: u16,
    /// `wScan`（Unicode 送信では UTF-16 code unit）。
    pub scan: u16,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub up: bool,
    /// Unicode 送信（`KEYEVENTF_UNICODE`）。`ch` はその code unit が単独の文字なら入る。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unicode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ch: Option<char>,
    /// `dwExtraInfo`（自己注入マーカー）。
    pub marker: usize,
}

impl From<SentKeyEvent> for SentKeyEventSummary {
    fn from(e: SentKeyEvent) -> Self {
        Self {
            vk: e.vk,
            scan: e.scan,
            up: e.up,
            unicode: e.unicode,
            ch: if e.unicode {
                char::from_u32(u32::from(e.scan))
            } else {
                None
            },
            marker: e.marker,
        }
    }
}

/// ジャーナルに記録するイベントの種別
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum JournalEntry {
    /// エンジンのキー入力処理（on_input）
    ///
    /// `repeat_count`/`last_timestamp_us`/`last_elapsed_ms`（ADR-169）:
    /// OS auto-repeat による同一キーの連続 `KeyInput` は
    /// `UnifiedJournal::record_key_input` が1エントリへ畳み込む。
    /// `repeat_count == 1` は畳み込みなし（通常の単発イベント）。
    /// `last_timestamp_us`/`last_elapsed_ms` は畳み込んだ最後のイベントの
    /// 生時刻（`event.timestamp_us`/envelope の `elapsed_ms` と同じ系）。
    /// 畳み込みが起きても `JournalEnvelope.seq`/`elapsed_ms` は初回のまま
    /// 変更しない（seq 昇順の時刻単調性を壊さないため）。
    KeyInput {
        event: KeyEventSummary,
        state_before: String,
        state_after: String,
        decision: DecisionKind,
        physical: PhysicalDispositionSummary,
        repeat_count: u32,
        last_timestamp_us: u64,
        last_elapsed_ms: u64,
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
    /// 本モジュールに依存せず Linux のリプレイテストからも同じ型で
    /// 構築・検証できる。リプレイは `tests/drift_correction_replay.rs` が
    /// `DriftCorrectionFixture` 経由で行う。`ActuationRecord` は書き出し用に `Serialize`
    /// のみ（`origin` が `&'static str` を含み `Deserialize` 不可のため、fixture 側は
    /// `epoch` のみ保存し `strategy` を `policy` から再構築する）。
    ImeActuation {
        record: crate::state::ime_actuation::ActuationRecord,
    },
    /// ADR-163 Part D: actuation合流点の決定点レコード。
    ///
    /// 既存のActuation laneへ相乗りし、bug reportが既に添付しているjournal JSONから
    /// 実機コーパスを抽出できるようにする。本variantの本番配線は163-T1b以降で行う。
    ActuationDecision {
        record: crate::state::actuation_decision_record::ActuationDecisionRecord,
    },
    /// awase 自身が `SendInput` で送ったキーボードイベント（`win32::send_input_safe` 1 回ぶん）。
    ///
    /// `KeyInput` は物理入力だけ（自己注入はフックで素通しされ記録されない）で、awase が
    /// 実際に何を送ったか（romaji の VK 列・Unicode 文字・`SendInput` の受理件数）は
    /// 従来 journal に無く、「入力と違う文字が出た」報告で原因を切り分けられなかった
    /// （LINE で「いまは」→「いいい」、report 01M43NK5P13Q7EQP7CS0N3X4ED）。
    /// `issue_us` は発行時刻（`KeyInput.timestamp_us` と同じ系）で、journal へ移す時刻とは別。
    /// 入力文字が分かる内容なので、ダンプ時は `LiteralDetect` と同じく直近 10 分に絞る。
    SentInput {
        issue_us: u64,
        accepted: u32,
        events: Vec<SentKeyEventSummary>,
    },
    /// ADR-132 Phase 1: Blind GiveUp 到達時に、次段の設計判断に必要な観測・
    /// 送信・意図・環境情報だけを構造化して残す。
    DriftGiveUpDiagnostic { record: DriftGiveUpDiagnosticRecord },
    /// ADR-132 Phase 1: hook の IME-mode 診断ログを journal にも残す。
    HookImeModeDiagnostic { record: HookImeModeDiagnosticRecord },
    /// ADR-132 Phase 1: GiveUp 通知区間がフォーカス変更で終わったことを記録する。
    DriftGiveUpIntervalEnded {
        reason: &'static str,
        elapsed_ms: u64,
    },
    /// ADR-227(BUG-074): `RawTsfLiteralRecovery` の give-up を契機にした外部クローズの読み直しの判断。
    /// `outcome` は `armed`(監視窓を開いて読み直す)/ `not_applicable` / `stale_focus` / `no_explicit_intent`。
    /// `baseline` は arm した時点の基準値(直近の読み)。追随が起きなかった理由の切り分け用。
    GiveUpFollow {
        cold_seq: u64,
        outcome: &'static str,
        baseline: Option<bool>,
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
    /// 明示キー押下の書き込みの予約（`ImeStateHub::claim_press_write`、ADR-208 決定2 D1）。
    ///
    /// `verdict` は `PressClaim::label`（`fresh`/`duplicate`/`conflict_engine_wins`/`conflict_kept`）。同じ押下（`press`）の
    /// 別経路が衝突した・二重送信を省いたことを、押下 ID つきで後から突合できる。押下 ID の無い order（`unpressed`）は記録しない。
    PressWriteClaim {
        press: u64,
        open: bool,
        source: &'static str,
        verdict: &'static str,
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
        /// `GjiAction::StartProbe` 経由で開始した場合の `ProbeId`（`Some`）。
        ///
        /// ADR-123 round 2（architect レビュー）指摘: `GjiAction::StartProbe`
        /// 起点のこのエントリは、かつて `probe_id` の値をそのまま `cold_seq`
        /// フィールドへ格納していた（`cold_seq` と `probe_id` は別の採番空間
        /// のため、これはログを読み違えさせる実バグだった）。`cold_seq` は
        /// 常に `CompositionState::cold_start_count()` 由来の値に統一し、
        /// `probe_id` を別フィールドとして独立させた。
        probe_id: Option<u64>,
        gji_state: String,
        consecutive_at_start: u32,
        /// この probe を開始する直前の `pending_deferred`（probe 実行中に届いた
        /// 別モーラの VK 退避キュー、`TsfWarmupCoordinator` 所有）の長さ。
        ///
        /// ADR-123: 非ゼロなら、この新しい probe が `pending_deferred` を
        /// まだ flush されていない状態で追い越して開始したことを意味する
        /// （issue #148「たとえば」→「ばたと」の根本原因）。
        pending_deferred_len: usize,
    },
    /// TSF/GJI probe の完了・中断・学習完了。
    TsfProbeCompleted {
        outcome: String,
        cold_seq: Option<u64>,
        /// `GjiAction::StartProbe`/`CancelProbe` 経由で完了した場合の
        /// `ProbeId`（`Some`）。
        ///
        /// ADR-123 `/code-review` 指摘: `TsfProbeStarted` の `cold_seq` を
        /// `probe_id` から本物の `cold_seq` へ切り替えたにもかかわらず、
        /// `GjiAction` 経由の `TsfProbeCompleted`（`UnicodeImmediate`/
        /// `Canceled`）側は `probe_id` を `cold_seq` に入れたままだったため、
        /// Start/Complete のペアが `cold_seq` では突合できなくなっていた
        /// （Started 側は本物の cold_seq、Completed 側は probe_id で
        /// 別の採番空間）。この2経路は `cold_seq: None, probe_id: Some(..)`
        /// とし、`probe_id` を突合キーとして明示する。`step_probe`
        /// （`advance_tsf_probe`）駆動の completion（`Done`/`LearnedTsf` 等）
        /// はこれまで通り `cold_seq: Some(..), probe_id: None`。
        probe_id: Option<u64>,
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
    /// `pending_deferred`（probe 実行中に届いた別モーラの VK 退避キュー）の
    /// flush/discard 結果（ADR-123）。
    ///
    /// `RawTsfLiteralRecovery` の give-up 直後（`trigger="raw_recovery"`、
    /// `platform.rs::flush_raw_tsf_literal_recovery`）と、ADR-128 の
    /// drain-before-send 実 flush（`trigger="drain_before_send"`、
    /// `output/vk_send.rs::drain_pending_deferred_before_send_if_queue_only`）から
    /// 記録する。従来は `tracing::debug!`/`tracing::warn!` の自由文字列でしか残らず、journal
    /// （構造化・容量優先度あり）には現れなかった（issue #148 の調査で
    /// `app_log_excerpt` を直接読まないと確認できず、journal の
    /// `DumpTruncated` で欠落しうる弱点だった）。
    ///
    /// GJI reinit retry 完了後の flush/discard は同じ意味論のデータだが、
    /// `token`/`focus_matches` 等の周辺情報とまとめて記録した方が読みやすい
    /// ため、本 variant ではなく `JournalEntry::GjiReinitRetryCompleted` の
    /// `deferred_flushed`/`deferred_discarded` フィールドに記録する
    /// （`platform.rs::complete_gji_reinit_retry`）。
    DeferredRecoveryFlush {
        trigger: &'static str,
        outcome: DeferredRecoveryOutcomeSummary,
    },
    /// GJI reinit（`VK_IME_OFF`→`VK_IME_ON`、`RawTsfLiteralRecovery` の
    /// give-up 分岐が予約する）retry poll の完了（ADR-123）。
    ///
    /// `origin_focus_gen`（give-up 検出時点のフォーカス世代）と
    /// `current_focus_gen`（poll 完了時点の世代）の一致・不一致が、
    /// `pending_deferred` を安全に flush してよいか（focus_matches）を
    /// 決める。この判定は従来 `tracing::debug!`/`tracing::warn!` のみで、journal
    /// には一切現れなかった。
    GjiReinitRetryCompleted {
        token: u32,
        status: String,
        cold_seq: u64,
        origin_focus_gen: u32,
        current_focus_gen: u32,
        focus_matches: bool,
        retry_romaji_present: bool,
        deferred_flushed: usize,
        deferred_discarded: usize,
    },
    /// `elapsed_ms` / OS tick / hook timestamp の対応を取るためのアンカー。
    ClockAnchor { tick_ms: u64, hook_us: u64 },
    /// ダンプトリガー発動
    ///
    /// `evicted_*`（ADR-169決定1-b）: 各レーンのリングバッファが容量超過で
    /// 完全に失った（`pop_front()`/満杯+古い遅延envelope破棄）累計件数。
    /// **このエントリはダンプのたびに必ず1件生成される**ため、evicted の出力先とする。
    ///
    /// `oldest_elapsed_ms_*`（ADR-222）: ダンプ時点で各レーンの ring に残っている
    /// 最古の entry の `elapsed_ms`（空のレーンは `None`）。**ring 内の値で、報告に載った
    /// 範囲ではない**（打鍵と `LiteralDetect` は、ダンプ時にさらに直近 10 分へ絞られる）。不具合報告は ring の
    /// 中身を全部出す（バイト配分で絞らない）ので、調査する側が「各レーンが
    /// 何分前まで残っているか」を `DumpTriggered` の `elapsed_ms` との差で読める。
    DumpTriggered {
        evicted_state: usize,
        evicted_timing: usize,
        evicted_actuation: usize,
        evicted_key_input: usize,
        oldest_elapsed_ms_state: Option<u64>,
        oldest_elapsed_ms_timing: Option<u64>,
        oldest_elapsed_ms_actuation: Option<u64>,
        oldest_elapsed_ms_key_input: Option<u64>,
    },
}

// /code-review指摘（PR #201、ADR-163 Part D）: 当初「`ActuationDecision`
// （`ActuationDecisionRecord`、`size_of <= 176`）が`JournalEntry`の最大
// variantを更新し、Rustがenumサイズを最大variantに合わせる結果、全4 lane
// （`ActuationDecision`を一切積まないState/Timing/KeyInputも含む）で
// `VecDeque<JournalEnvelope>`の事前確保メモリが増える」という懸念が
// 指摘された。**実測の結果、この懸念は成立しない**——`JournalEntry`の
// サイズは本PR適用前後で264バイトのまま変化していない（develop時点の
// `size_of::<JournalEntry>()`も264、`ActuationDecisionRecord`の176バイトは
// 既存の最大variantを更新しない）。以下は将来variantを追加して264バイトを
// 超えた場合に気付くための回帰ガード（実測値をそのまま固定、
// [tuning-constants](../../.claude/rules/tuning-constants.md)の精神）。
const _: () = assert!(size_of::<JournalEntry>() == 264);

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

/// レーン別の ring 内最古 `elapsed_ms`（ADR-222）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OldestElapsedByLane {
    pub state: Option<u64>,
    pub timing: Option<u64>,
    pub actuation: Option<u64>,
    pub key_input: Option<u64>,
}

/// レーン別 eviction カウンタ（ADR-169決定1-b）。
///
/// `[(LaneKind, usize); 4]` ではなく named struct にする——配列だと
/// 消費側（`evicted[0].1` 等）が `LaneKind` タグを見ずに位置だけで
/// 読むため、将来配列の並び順を変えるとコンパイルエラー無しに
/// 値が別レーンに誤対応する（opus-adversarial-consult コードレビュー指摘）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvictedByLane {
    pub state: usize,
    pub timing: usize,
    pub actuation: usize,
    pub key_input: usize,
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
    /// このレーンから容量超過で完全に失われたエントリ数（ADR-169決定1-b）。
    /// `DumpTruncated.dropped_key_input`（byte予算段の間引き）とは別軸で、
    /// リングバッファ自体からの退避を数える。
    ///
    /// **2つの異なる原因を1つの数値に合算している点に注意**（`/code-review`
    /// round3指摘）: (a) レーンが満杯で最古のエントリを `pop_front()` で
    /// 追い出す本来の意味の「容量超過による退避」、(b) レーンが満杯かつ
    /// 到着した（`absorb()` 経由の遅延）envelope の `seq` がレーン内の
    /// 最古より古い（順序が乱れて遅着した）ため一度もバッファに入らず
    /// 破棄されるケース。どちらも「本来記録されるべきだったエントリが
    /// 失われた」点は同じだが、後者は容量不足ではなく defer 経路の
    /// 順序/遅延の問題であり、`evicted_key_input` が高止まりしていても
    /// 原因は「容量を増やせば直る」とは限らない。原因を区別したい場合は
    /// `push()` の該当2箇所を参照すること。
    evicted: usize,
}

impl JournalLane {
    fn new(capacity: usize) -> Self {
        Self {
            // ADR-222: 打鍵レーンは 8,192 件で、常用時に全量を事前確保すると約 2.3MB を
            // 打鍵が無い時間帯も占有する。伸長は償却コストで足りるので事前確保は控えめにする。
            buffer: VecDeque::with_capacity(capacity.min(512)),
            capacity,
            evicted: 0,
        }
    }

    fn push(&mut self, envelope: JournalEnvelope) {
        if self.capacity == 0 {
            // 容量0のレーンへの記録も「本来記録されるべきだったが失われた」
            // という点で他の2つの喪失経路と同じであり、evicted_by_lane()の
            // 網羅性（ADR-169決定1-b）を保つため計上する（`/code-review
            // opus` round3指摘。本番の各レーン容量は`LaneKind::capacity()`
            // 由来の非ゼロ定数のみで、現状到達しない経路だが、将来
            // capacity:0のレーンが構成された場合に無音の過小計上を防ぐ）。
            self.evicted += 1;
            return;
        }
        if self.buffer.len() == self.capacity {
            if self
                .buffer
                .front()
                .is_some_and(|front| envelope.seq < front.seq)
            {
                self.evicted += 1;
                return;
            }
            self.buffer.pop_front();
            self.evicted += 1;
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
            | Self::DumpTriggered { .. } => LaneKind::State,
            Self::GjiFsmTransition { .. }
            | Self::HookImeModeDiagnostic { .. }
            | Self::TsfProbeStarted { .. }
            | Self::TsfProbeCompleted { .. }
            | Self::LiteralDetect { .. }
            | Self::DeferredRecoveryFlush { .. }
            | Self::GjiReinitRetryCompleted { .. } => LaneKind::Timing,
            Self::ImeActuation { .. }
            | Self::SentInput { .. }
            | Self::ActuationDecision { .. }
            | Self::PressWriteClaim { .. }
            | Self::DriftGiveUpDiagnostic { .. }
            | Self::DriftGiveUpIntervalEnded { .. }
            | Self::GiveUpFollow { .. }
            | Self::ConvClassifyCall { .. }
            | Self::TimerFired { .. } => LaneKind::Actuation,
            Self::KeyInput { .. } => LaneKind::KeyInput,
        }
    }
}

// ── journal → tracing 一方向 fan-out（ADR-139 決定4、Option C） ──────────────
//
// journal を SSOT（決定論的リプレイの正）のまま変えず、`UnifiedJournal::absorb`
// （journal への2系統の入口が最終的に合流する唯一の地点）から、構造化 tracing
// イベントとして journal の内容をそのまま流す。既定レベルは `debug!` に統一する
// （`ImeEvent`/`KeyInput` 等は tick・打鍵ごとに無条件で journal へ落ちるため、
// 一部だけ `info!` に格上げすると既定フィルタ `"info"` の下で awase.log が
// 常時肥大化し、決定2〈ログ肥大防止〉と矛盾する）。
//
// 判別子文字列（`?`/`%` の代わり）は、各 enum に `strum::IntoStaticStr` を derive して
// `variant_name` 経由で取る（以前は journal.rs 内の手書き `match` 対応表だった。
// variant 追加時の更新漏れを derive が型で防ぐ）。`strum` は OS 非依存で、core crate
// の型に derive を足しても ADR-019 の制約（windows-rs / cfg(target_os) / VK 数値の
// 持ち込み禁止）には触れない。値は journal の JSON シリアライズ（serde、variant 名
// そのまま）と表記を揃える。
//
// 深くネストした構造体（`ActuationRecord`/`AnyObservation`/
// `DriftGiveUpDiagnosticRecord` 等）は、当面トップレベルの主要フィールド、
// または粗い判別子のみを記録する（完全な再帰的展開は将来のフォローアップ、
// 決定4 必須条件6）。`match` の網羅性（`_ =>` を書かない）だけは全箇所で守る
// ——将来 variant が増えたときにコンパイルエラーで検知させるための唯一の
// 安全装置。

/// ADR-169: `journal_policy` は `DecisionKind`（本モジュールの型）を直接参照
/// しない設計のため、比較用の局所的な形（`KeyInputDecisionShape`）へここで変換する。
fn decision_kind_shape(d: &DecisionKind) -> crate::journal_policy::KeyInputDecisionShape {
    use crate::journal_policy::KeyInputDecisionShape as Shape;
    match *d {
        DecisionKind::PassThrough => Shape::PassThrough,
        DecisionKind::PassThroughWith { effect_count } => Shape::PassThroughWith { effect_count },
        DecisionKind::Consume { effect_count } => Shape::Consume { effect_count },
    }
}

fn physical_disposition_shape(
    p: &PhysicalDispositionSummary,
) -> crate::journal_policy::KeyInputPhysicalShape {
    use crate::journal_policy::KeyInputPhysicalShape as Shape;
    match *p {
        PhysicalDispositionSummary::Allow => Shape::Allow,
        PhysicalDispositionSummary::Suppress { reason } => Shape::Suppress { reason },
    }
}

/// `record_key_input` の畳み込み判定用に、`JournalEntry::KeyInput` から
/// 識別情報を取り出す。呼び出し契約上、`entry` は必ず `KeyInput` variant。
fn key_input_identity(entry: &JournalEntry) -> crate::journal_policy::KeyInputIdentity<'_> {
    let JournalEntry::KeyInput {
        event,
        state_before,
        state_after,
        decision,
        physical,
        ..
    } = entry
    else {
        unreachable!("key_input_identity は KeyInput variant にのみ呼ばれる")
    };
    crate::journal_policy::KeyInputIdentity {
        vk_code: event.vk_code,
        scan_code: event.scan_code,
        is_down: event.is_down,
        injected: event.injected,
        key_class: event.key_class,
        alt: event.alt,
        ctrl: event.ctrl,
        shift: event.shift,
        state_before,
        state_after,
        decision: decision_kind_shape(decision),
        physical: physical_disposition_shape(physical),
    }
}

/// tracing 用の判別子文字列（variant 名）。`strum::IntoStaticStr` の derive が生成する
/// `From<T>`/`From<&T>` 経由で取るため、variant 追加時の対応表の更新漏れが起きない。
/// 値は journal の JSON シリアライズ（serde、variant 名そのまま）と表記が揃う。
fn variant_name<T: Into<&'static str>>(value: T) -> &'static str {
    value.into()
}

/// `Option<MechanismCommand>` の判別子名（`None` は文字列 `"None"`）。
fn mechanism_command_str(
    command: Option<crate::state::ime_actuation_decision::MechanismCommand>,
) -> &'static str {
    command.map_or("None", variant_name)
}

impl JournalEntry {
    /// journal エントリを構造化 tracing イベントとして吐く。呼ぶのは
    /// [`JournalEnvelope::emit_tracing`] の内側だけ（`UnifiedJournal::absorb` 経由）。
    ///
    /// **`_ =>` ワイルドカードを書かない**こと。`architecture_guard.rs` の
    /// `journal_emit_tracing_has_no_debug_display_sigils_or_wildcards` が
    /// `?`/`%` シギルと併せてこれを機械的に禁止する。
    ///
    /// 19 variant を1関数で網羅する構造上、`cognitive_complexity` は必然的に
    /// 高くなる（実測 21/15、Windows実機CIで検出）。`hook_callback` 等
    /// 既存の大規模dispatch関数と同型の許容パターン（本ファイルの他、
    /// `output/mod.rs`・`runtime/key_pipeline.rs`等13箇所に既存）。
    /// variantごとに小関数へ分割すると、match の網羅性チェック
    /// （将来variant追加時のコンパイルエラー検知、この機構の唯一の安全装置）
    /// が複数関数に分散し、かえって見通しが悪くなる。
    #[allow(clippy::cognitive_complexity)]
    fn emit_tracing(&self, seq: u64, elapsed_ms: u64) {
        match self {
            Self::KeyInput {
                event,
                state_before,
                state_after,
                decision,
                physical,
                repeat_count,
                last_timestamp_us,
                last_elapsed_ms,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    vk_code = event.vk_code,
                    is_down = event.is_down,
                    injected = event.injected,
                    key_class = event.key_class,
                    state_before = state_before.as_str(),
                    state_after = state_after.as_str(),
                    decision = variant_name(decision),
                    physical = variant_name(physical),
                    repeat_count,
                    last_timestamp_us,
                    last_elapsed_ms,
                    "key input"
                );
            }
            Self::TimerFired {
                timer_id,
                state_before,
                state_after,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    timer_id,
                    state_before = state_before.as_str(),
                    state_after = state_after.as_str(),
                    "timer fired"
                );
            }
            Self::ImeEvent { event } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    event_kind = variant_name(event),
                    "ime event"
                );
            }
            Self::ConvClassifyCall {
                conv,
                current: _,
                is_cold,
                effective_open,
                conv_mode_changed,
                is_roman_reliable,
                result: _,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    conv,
                    is_cold,
                    effective_open,
                    conv_mode_changed,
                    is_roman_reliable,
                    "conv classify call"
                );
            }
            Self::ImeActuation { record } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    target_open = record.target,
                    attempts = record.attempts,
                    policy = variant_name(record.policy),
                    action = variant_name(record.action),
                    "ime actuation"
                );
            }
            Self::SentInput {
                issue_us,
                accepted,
                events,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    issue_us,
                    accepted,
                    event_count = events.len(),
                    "sent input"
                );
            }
            Self::ActuationDecision { record } => {
                let attempt_limit = record.attempts_len.min(record.attempts.len());
                let first_attempt = record.attempts[..attempt_limit]
                    .iter()
                    .find_map(|attempt| *attempt);
                let first_inputs = first_attempt.map(|attempt| attempt.inputs);
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    site = variant_name(record.site),
                    caller = record.caller.map_or("None", variant_name),
                    open = record.order.open,
                    chain_len = record.chain_len,
                    attempts_len = record.attempts_len,
                    gate_profile = variant_name(record.gate_inputs.profile),
                    gate_kind = variant_name(record.gate_inputs.kind),
                    gate_shadow_known = record.gate_inputs.shadow_on.is_some(),
                    gate_shadow_on = record.gate_inputs.shadow_on.unwrap_or(false),
                    gate_input_mode = variant_name(record.gate_inputs.belief_input_mode),
                    first_attempt_present = first_attempt.is_some(),
                    first_mechanism = first_attempt
                        .map_or("None", |attempt| variant_name(attempt.mechanism)),
                    first_command = first_attempt
                        .map_or("None", |attempt| mechanism_command_str(attempt.command)),
                    first_outcome = first_attempt
                        .map_or("None", |attempt| variant_name(attempt.outcome)),
                    first_with_app_available = first_attempt
                        .is_some_and(|attempt| attempt.with_app_available),
                    first_profile = first_inputs
                        .map_or("None", |inputs| variant_name(inputs.profile)),
                    first_kind = first_inputs.map_or("None", |inputs| variant_name(inputs.kind)),
                    first_input_mode = first_inputs
                        .map_or("None", |inputs| variant_name(inputs.belief_input_mode)),
                    "actuation decision"
                );
            }
            Self::DriftGiveUpDiagnostic { record } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    desired_open = record.desired_open,
                    observed_open = record.observed_open,
                    drift_duration_ms = record.drift_duration_ms,
                    layout_name = record.layout_name.as_str(),
                    half_width_alnum_toggle_active = record.half_width_alnum_toggle_active,
                    "drift give-up diagnostic"
                );
            }
            Self::HookImeModeDiagnostic { record } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    vk_code = record.vk_code,
                    is_down = record.is_down,
                    self_injected = record.self_injected,
                    injected = record.injected,
                    scan = record.scan,
                    "hook ime-mode diagnostic"
                );
            }
            Self::DriftGiveUpIntervalEnded {
                reason,
                elapsed_ms: interval_elapsed_ms,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    reason = *reason,
                    interval_elapsed_ms,
                    "drift give-up interval ended"
                );
            }
            Self::GiveUpFollow {
                cold_seq,
                outcome,
                baseline,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    cold_seq,
                    outcome = *outcome,
                    baseline,
                    "give-up follow"
                );
            }
            Self::ImeOpenApplied {
                open,
                outcome,
                reason,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    open,
                    outcome = variant_name(*outcome),
                    reason = variant_name(*reason),
                    "ime open applied"
                );
            }
            Self::PressWriteClaim {
                press,
                open,
                source,
                verdict,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    press,
                    open,
                    source,
                    verdict,
                    "press write claim"
                );
            }
            Self::FocusTransition {
                changed,
                from: _,
                to: _,
                dwell_ms,
                profile,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    changed_process = changed.process,
                    changed_window = changed.window,
                    changed_app_kind = changed.app_kind,
                    changed_focus_kind = changed.focus_kind,
                    dwell_ms,
                    profile = profile.as_str(),
                    "focus transition"
                );
            }
            Self::GjiFsmTransition {
                trigger,
                state_before,
                state_after,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    trigger = trigger.as_str(),
                    state_before = state_before.as_str(),
                    state_after = state_after.as_str(),
                    "gji fsm transition"
                );
            }
            Self::TsfProbeStarted {
                source,
                cold_seq,
                probe_id,
                gji_state,
                consecutive_at_start,
                pending_deferred_len,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    source = source.as_str(),
                    cold_seq,
                    probe_id = probe_id.unwrap_or(u64::MAX),
                    probe_id_present = probe_id.is_some(),
                    gji_state = gji_state.as_str(),
                    consecutive_at_start,
                    pending_deferred_len,
                    "tsf probe started"
                );
            }
            Self::TsfProbeCompleted {
                outcome,
                cold_seq,
                probe_id,
                elapsed_ms: duration_ms,
                tick_count,
                gji_state,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    outcome = outcome.as_str(),
                    cold_seq = cold_seq.unwrap_or(u64::MAX),
                    cold_seq_present = cold_seq.is_some(),
                    probe_id = probe_id.unwrap_or(u64::MAX),
                    probe_id_present = probe_id.is_some(),
                    duration_ms,
                    tick_count,
                    gji_state = gji_state.as_str(),
                    "tsf probe completed"
                );
            }
            Self::LiteralDetect {
                record,
                suppressed_confirms,
                since_vk_sent_ms,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    verdict = variant_name(record.facts.verdict),
                    consecutive_before = record.consecutive_before,
                    gave_up = record.gave_up,
                    backs = record.backs,
                    suppressed_confirms,
                    since_vk_sent_ms,
                    "literal detect"
                );
            }
            Self::DeferredRecoveryFlush { trigger, outcome } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    trigger = *trigger,
                    outcome = variant_name(outcome),
                    "deferred recovery flush"
                );
            }
            Self::GjiReinitRetryCompleted {
                token,
                status,
                cold_seq,
                origin_focus_gen,
                current_focus_gen,
                focus_matches,
                retry_romaji_present,
                deferred_flushed,
                deferred_discarded,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    token,
                    status = status.as_str(),
                    cold_seq,
                    origin_focus_gen,
                    current_focus_gen,
                    focus_matches,
                    retry_romaji_present,
                    deferred_flushed,
                    deferred_discarded,
                    "gji reinit retry completed"
                );
            }
            Self::ClockAnchor { tick_ms, hook_us } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    tick_ms,
                    hook_us,
                    "clock anchor"
                );
            }
            Self::DumpTriggered {
                evicted_state,
                evicted_timing,
                evicted_actuation,
                evicted_key_input,
                oldest_elapsed_ms_state,
                oldest_elapsed_ms_timing,
                oldest_elapsed_ms_actuation,
                oldest_elapsed_ms_key_input,
            } => {
                tracing::debug!(
                    target: "awase::journal",
                    seq,
                    elapsed_ms,
                    evicted_state,
                    evicted_timing,
                    evicted_actuation,
                    evicted_key_input,
                    oldest_elapsed_ms_state,
                    oldest_elapsed_ms_timing,
                    oldest_elapsed_ms_actuation,
                    oldest_elapsed_ms_key_input,
                    "dump triggered"
                );
            }
        }
    }
}

impl JournalEnvelope {
    /// [`JournalEntry::emit_tracing`] に委譲する。呼ぶのは
    /// `UnifiedJournal::absorb` の内側だけ（1箇所）。
    fn emit_tracing(&self) {
        self.entry.emit_tracing(self.seq, self.elapsed_ms);
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
    /// `(seq, elapsed_ms)` だけを先に採番する。`stamp` と同じ採番を、entry の中身が決まる
    /// 前（`SendInput` の発行時）に行うためのもの。後で `JournalEnvelope` を同じ値で組み立てる。
    ///
    /// drain 時に採番すると、遅れて送ったキーが次の打鍵の `KeyInput` の後ろに並んだり、
    /// ダンプの 10 分窓が送信時刻でなく drain 時刻で判定されたりする（ADR-096 B-4 が
    /// 是正した「保留キューが drain 時刻で採番される」問題の再発）ため。
    #[must_use]
    pub fn reserve(&self) -> (u64, u64) {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let elapsed_ms = (self.clock.now() - self.start).as_millis() as u64;
        (seq, elapsed_ms)
    }

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
    ///
    /// ADR-139 決定4（Option C）: journal → tracing の一方向 fan-out を
    /// ここ（journal への2系統の入口が最終的に合流する唯一の地点）で行う。
    /// レーン容量超過で `JournalLane::push` が黙って捨てるエントリも
    /// tracing 側には出力される（意図的。tracing は人間向けの、独自フィルタを
    /// 持つ可能性のあるチャネル、journal はリプレイ用の有界リングという役割分担）。
    ///
    /// # Panics
    /// `envelope.entry` が `JournalEntry::KeyInput` の場合（ADR-169、
    /// `record_key_input()` を使うこと）。
    pub fn absorb(&mut self, envelope: JournalEnvelope) {
        // ADR-169: `KeyInput` は `record_key_input()` 専用（畳み込みが依存
        // する「`key_input` レーンの `back()` は直前に記録した `KeyInput`
        // である」という不変条件を、`absorb()` 経由の遅延 envelope が
        // 壊しうるため——round1 Major1 参照）。将来 `KeyInput` が
        // `drain_journal_entries()`/deferred キュー経由でこの経路に
        // 紛れ込むと、無関係なエントリへ `repeat_count` が誤って加算される
        // （時系列の捏造）事故を、静かに再発させず早期に検知する
        // （opus-adversarial-consult コードレビュー指摘）。`debug_assert!`
        // だとリリースビルドで無効化され唯一の安全網が消えるため、通常の
        // `assert!` にする（`matches!` 1回だけの軽量チェックであり、
        // absorb() は per-keystroke のような超高頻度経路ではない）。
        assert!(
            !matches!(envelope.entry, JournalEntry::KeyInput { .. }),
            "KeyInput は absorb() ではなく record_key_input() を使うこと(ADR-169)"
        );
        envelope.emit_tracing();
        self.route_to_lane(envelope);
    }

    /// `entry.lane_kind()` に応じた正しいレーンへ push する（tracing 発行は
    /// 呼び出し元の責務、ここでは行わない）。`absorb()` と
    /// `record_key_input()` の契約違反フォールバックの両方から使う共通経路
    /// （opus-adversarial-consult コードレビュー指摘、`key_input` レーンへ
    /// 無条件 push していた旧実装は、非 `KeyInput` エントリが紛れ込んだ
    /// 場合に `key_input_identity()` の `unreachable!()` を次回呼び出しで
    /// 誘発しうる危険なフォールバックだった）。
    fn route_to_lane(&mut self, envelope: JournalEnvelope) {
        match envelope.entry.lane_kind() {
            LaneKind::State => self.lanes.state.push(envelope),
            LaneKind::Timing => self.lanes.timing.push(envelope),
            LaneKind::Actuation => self.lanes.actuation.push(envelope),
            LaneKind::KeyInput => self.lanes.key_input.push(envelope),
        }
    }

    /// `JournalEntry::KeyInput` 専用の記録経路（ADR-169）。
    ///
    /// `record()`/`absorb()` とは意図的に分離する（`docs/adr/169-*.md`
    /// 「畳み込みの実装場所」参照）: `absorb()` は `drain_journal_entries()`
    /// 経由の遅延 envelope（`JournalStamper` で先に採番済み、seq 順に
    /// `rposition` で挿入し直される）も受け取るため、「`key_input` レーンの
    /// `buffer.back()` は直前に記録した `KeyInput` である」という、この
    /// 畳み込みロジックが依存する不変条件が成り立たない。**この不変条件は
    /// `JournalEntry::KeyInput {` の本番構築点が `runtime/key_pipeline.rs`
    /// の1箇所のみであること、かつこのレーンに `absorb()` 経由の遅延
    /// envelope が流れ込まないことに依存する——どちらかが崩れると
    /// `back()` は「直前の KeyInput」でなくなり、無関係なエントリへ
    /// `repeat_count` が誤って加算される（`tests/architecture_guard.rs`
    /// の出現数固定テストで守る）。**
    ///
    /// 呼び出し側は毎回 `record_key_input` を通し、`record()`/`absorb()` を
    /// `KeyInput` に対して直接呼ばないこと。
    ///
    /// `was_down`: `hook.rs::HOOK_STATE.physical_key_state` の `swap` で
    /// 得た、このイベント直前の物理押下状態（`RawKeyEvent::was_down`）。
    /// `entry` は必ず `JournalEntry::KeyInput` variant で渡すこと。
    ///
    /// emit_tracing は畳み込みの有無に関わらず**毎回**呼ぶ（`app_log_excerpt`
    /// 側から auto-repeat の痕跡が消えないようにするため）。畳み込まれた
    /// repeat にも通常どおり `seq` を採番する——`key_input` レーンの journal
    /// 出力に seq の穴が空くのは畳み込みによるものであり drop ではない
    /// （穴の範囲は、その直前の `KeyInput` エントリの `repeat_count` から
    /// 逆算できる）。
    pub fn record_key_input(&mut self, entry: JournalEntry, was_down: bool) -> u64 {
        debug_assert!(
            matches!(entry, JournalEntry::KeyInput { .. }),
            "record_key_input は JournalEntry::KeyInput 専用"
        );
        let envelope = self.stamper().stamp(entry);
        let seq = envelope.seq;
        envelope.emit_tracing();

        let JournalEntry::KeyInput { event, .. } = &envelope.entry else {
            // 契約違反（KeyInput以外）。`key_input` レーンへ無条件 push
            // すると、次回呼び出しの `key_input_identity()`（back() が
            // 常に KeyInput である前提）で `unreachable!()` を誘発する
            // （opus-adversarial-consult コードレビュー指摘）。
            // `lane_kind()` に基づく本来のレーンへ振り分ける
            // （データを失わない、かつ `key_input` レーンの不変条件も
            // 守る）。
            self.route_to_lane(envelope);
            return seq;
        };
        let injected = event.injected;
        let next_identity = key_input_identity(&envelope.entry);
        let prev_identity = self
            .lanes
            .key_input
            .buffer
            .back()
            .map(|prev| key_input_identity(&prev.entry));
        let outcome = crate::journal_policy::coalesce_key_input(
            prev_identity.as_ref(),
            &next_identity,
            was_down,
            injected,
        );

        match outcome {
            crate::journal_policy::CoalesceOutcome::MergeIntoPrevious => {
                // `event`（1437行目で束縛済み）は `envelope.entry` からの
                // 不変借用として引き続き有効——`opus-adversarial-consult`
                // コードレビュー指摘により、ここで再度 `envelope.entry` を
                // match し直す冗長な分解を削除した。
                let event_timestamp_us = event.timestamp_us;
                let next_elapsed_ms = envelope.elapsed_ms;
                if let Some(back) = self.lanes.key_input.buffer.back_mut() {
                    if let JournalEntry::KeyInput {
                        repeat_count,
                        last_timestamp_us,
                        last_elapsed_ms,
                        ..
                    } = &mut back.entry
                    {
                        *repeat_count += 1;
                        *last_timestamp_us = event_timestamp_us;
                        *last_elapsed_ms = next_elapsed_ms;
                    }
                }
            }
            crate::journal_policy::CoalesceOutcome::NewEntry => {
                let mut envelope = envelope;
                let elapsed_ms = envelope.elapsed_ms;
                if let JournalEntry::KeyInput {
                    event,
                    repeat_count,
                    last_timestamp_us,
                    last_elapsed_ms,
                    ..
                } = &mut envelope.entry
                {
                    *repeat_count = 1;
                    *last_timestamp_us = event.timestamp_us;
                    *last_elapsed_ms = elapsed_ms;
                }
                self.lanes.key_input.push(envelope);
            }
        }
        seq
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

    /// 各レーンの ring に残っている最古の entry の `elapsed_ms`（ADR-222）。
    /// `DumpTriggered.oldest_elapsed_ms_*` の元。空のレーンは `None`。
    #[must_use]
    pub fn oldest_elapsed_ms_by_lane(&self) -> OldestElapsedByLane {
        let oldest = |lane: &JournalLane| lane.buffer.iter().map(|e| e.elapsed_ms).min();
        OldestElapsedByLane {
            state: oldest(&self.lanes.state),
            timing: oldest(&self.lanes.timing),
            actuation: oldest(&self.lanes.actuation),
            key_input: oldest(&self.lanes.key_input),
        }
    }

    /// 各レーンの `evicted`（リングバッファ容量超過による完全消失件数）の
    /// 現在値を snapshot する（ADR-169決定1-b）。
    #[must_use]
    pub fn evicted_by_lane(&self) -> EvictedByLane {
        EvictedByLane {
            state: self.lanes.state.evicted,
            timing: self.lanes.timing.evicted,
            actuation: self.lanes.actuation.evicted,
            key_input: self.lanes.key_input.evicted,
        }
    }

    /// `%TEMP%/awase_journal_<tick_ms>.json` に書き出す。
    ///
    /// 時刻の出所（`hook::current_tick_ms`）が Windows 専用のため `#[cfg(windows)]`。
    #[cfg(windows)]
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

    /// 不具合報告用: ring の中身を**全部**、compact JSON で書き出す（ADR-222。
    /// 旧 `dump_to_file_capped` のバイト配分による間引きは廃止した）。
    ///
    /// 入力文字が分かる entry（打鍵 KeyInput と `LiteralDetect`）だけは、直近
    /// `REPORT_KEY_INPUT_WINDOW_MS`（10 分）に絞る
    /// （所有者が許容した範囲。ring は最大頻度で 10 分が溢れない容量なので、通常の
    /// 頻度では何時間ぶんも溜まっている。Opus round2 B-E1）。他のレーンは打鍵の
    /// 内容を含まないので全件出す。
    #[cfg(windows)]
    pub fn dump_to_file_for_report(&self) -> Result<std::path::PathBuf, DumpError> {
        let started = std::time::Instant::now();
        let tick = crate::hook::current_tick_ms();
        let path = std::env::temp_dir().join(format!("awase_journal_{tick}.json"));
        let now_ms = (self.clock.now() - self.start).as_millis() as u64;
        let entries: Vec<&JournalEnvelope> = self
            .entries_by_seq()
            .into_iter()
            .filter(|envelope| match &envelope.entry {
                JournalEntry::KeyInput {
                    last_elapsed_ms, ..
                } => crate::journal_policy::key_input_in_report_window(
                    envelope.elapsed_ms,
                    *last_elapsed_ms,
                    now_ms,
                    crate::journal_policy::REPORT_KEY_INPUT_WINDOW_MS,
                ),
                // `SentInput` は送ったキーそのもの（入力文字が分かる）なので同じ窓に絞る。
                // `LiteralDetect` の `romaji` は送信予定だった romaji そのもので、`trace` には vk 列が
                // 入る（入力文字が分かる）。KeyInput と同じ窓にしないと、所有者が許容した直近 10 分より
                // 前の入力文字が断片的に送られる（Opus round3 M-E5）。
                JournalEntry::LiteralDetect { .. } | JournalEntry::SentInput { .. } => {
                    crate::journal_policy::key_input_in_report_window(
                        envelope.elapsed_ms,
                        0,
                        now_ms,
                        crate::journal_policy::REPORT_KEY_INPUT_WINDOW_MS,
                    )
                }
                _ => true,
            })
            .collect();
        let json = serde_json::to_string(&entries)?;
        std::fs::write(&path, &json).map_err(|source| DumpError::Write {
            path: path.clone(),
            source,
        })?;
        // ADR-222 D2: メインスレッド（キーボードフックと同じスレッド）で数 MB を
        // シリアライズするため、実機ログで所要時間を確認できるようにする。
        tracing::info!(
            "[journal] report dump: {} entries, {} bytes, {} ms",
            entries.len(),
            json.len(),
            started.elapsed().as_millis()
        );
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
            physical: PhysicalDispositionSummary::Allow,
            repeat_count: 1,
            last_timestamp_us: 123,
            last_elapsed_ms: 0,
        }
    }

    /// `make_key_input_entry()` は `injected: true` 固定なので、
    /// `record_key_input` の畳み込みテスト用に非 injected 版を作る。
    fn make_non_injected_key_input_entry() -> JournalEntry {
        let JournalEntry::KeyInput { mut event, .. } = make_key_input_entry() else {
            unreachable!()
        };
        event.injected = false;
        JournalEntry::KeyInput {
            event,
            state_before: "engine-before".to_owned(),
            state_after: "engine-after".to_owned(),
            decision: DecisionKind::PassThrough,
            physical: PhysicalDispositionSummary::Allow,
            repeat_count: 1,
            last_timestamp_us: 123,
            last_elapsed_ms: 0,
        }
    }

    #[test]
    fn record_key_input_merges_repeated_keydown_when_was_down_and_not_injected() {
        let (mut j, _mock) = mock_journal();
        j.record_key_input(make_non_injected_key_input_entry(), false);
        j.record_key_input(make_non_injected_key_input_entry(), true);
        assert_eq!(
            j.len(),
            1,
            "同一payloadのKeyDown repeatは1エントリへ畳み込まれるはず"
        );
        let entries = j.entries_by_seq();
        let JournalEntry::KeyInput { repeat_count, .. } = &entries[0].entry else {
            panic!("KeyInput以外が記録された");
        };
        assert_eq!(*repeat_count, 2);
    }

    #[test]
    fn record_key_input_does_not_merge_when_was_down_is_false() {
        let (mut j, _mock) = mock_journal();
        j.record_key_input(make_non_injected_key_input_entry(), false);
        j.record_key_input(make_non_injected_key_input_entry(), false);
        assert_eq!(
            j.len(),
            2,
            "was_down=falseなら間にkey-upを挟んだ別打鍵として扱い畳み込まない"
        );
    }

    #[test]
    fn record_key_input_does_not_merge_keyup_even_if_was_down() {
        let (mut j, _mock) = mock_journal();
        j.record_key_input(make_non_injected_key_input_entry(), false);
        let JournalEntry::KeyInput { mut event, .. } = make_non_injected_key_input_entry() else {
            unreachable!()
        };
        event.is_down = false;
        let keyup = JournalEntry::KeyInput {
            event,
            state_before: "engine-before".to_owned(),
            state_after: "engine-after".to_owned(),
            decision: DecisionKind::PassThrough,
            physical: PhysicalDispositionSummary::Allow,
            repeat_count: 1,
            last_timestamp_us: 123,
            last_elapsed_ms: 0,
        };
        j.record_key_input(keyup, true);
        assert_eq!(
            j.len(),
            2,
            "通常のKeyDown→KeyUpタップは畳み込まれてはならない(2026-09-13回帰)"
        );
    }

    #[test]
    #[should_panic(expected = "record_key_input は JournalEntry::KeyInput 専用")]
    fn record_key_input_panics_in_debug_on_contract_violation() {
        let (mut j, _mock) = mock_journal();
        j.record_key_input(make_state_entry(), false);
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
            // ADR-169: KeyInput は record_key_input() 専用（record() は
            // absorb() 経由で assert! に抵触する）。
            j.record_key_input(make_key_input_entry(), false);
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
        // ADR-169: KeyInput は record_key_input() 専用。
        j.record_key_input(make_key_input_entry(), false);
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
    fn report_lane_capacities_keep_ten_minutes_of_key_input() {
        // ADR-222: 打鍵は実測の最大頻度（1 分 475 件）で 10 分ぶん（約 4,750 件）が
        // ring から溢れないこと。この下限を割る変更は、不具合報告から打鍵が
        // 欠ける（report 01M42BME26GDQ3CJ4F5DMGT0MP: 165 秒・73 件）再発になる。
        assert!(KEY_INPUT_LANE_CAPACITY >= 475 * 10);
    }

    #[test]
    fn to_json_emits_every_entry_without_byte_budget() {
        // `mock_journal` は容量 10 の小さな ring。容量内（10 件）なら、バイト配分による
        // 間引きも合成ヘッダ（旧 DumpTruncated）も無く、全件がそのまま出る。
        let (mut j, _mock) = mock_journal();
        for _ in 0..10 {
            j.record(make_state_entry());
        }
        let json = j.to_json().unwrap();
        let values: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(values.len(), 10);
        assert!(values.iter().all(|v| v["entry"]["type"] != "DumpTruncated"));
    }

    #[test]
    fn oldest_elapsed_ms_by_lane_reports_each_lane_separately() {
        let (mut j, mock) = mock_journal();
        let empty = j.oldest_elapsed_ms_by_lane();
        assert_eq!(empty.state, None);
        assert_eq!(empty.key_input, None);
        j.record(make_state_entry());
        mock.increment(Duration::from_millis(5));
        j.record(make_state_entry());
        let oldest = j.oldest_elapsed_ms_by_lane();
        assert_eq!(oldest.state, Some(0));
        assert_eq!(oldest.key_input, None);
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
        use crate::state::ime_actuation::{ActuationRecord, FeedbackPolicy};

        let policy = FeedbackPolicy::Blind {
            max_attempts: 5,
            backoff: Duration::from_millis(400),
        };
        let (mut j, _mock) = mock_journal();
        // attempts=2 < max_attempts=5 なので action は Send に導出される。
        j.record(JournalEntry::ImeActuation {
            record: ActuationRecord::new(policy.origin(Generation::new(2)), false, policy, 2),
        });
        let json = j.to_json().unwrap();
        // 自由文字列ではなく構造化された出所・世代・判定が型として書き出される。
        assert!(json.contains("ImeActuation"));
        assert!(json.contains("SelfActuated"));
        assert!(json.contains("drift_correction_blind"));
        assert!(json.contains("\"action\": \"Send\""));
    }

    #[test]
    fn sent_input_entry_serializes_romaji_vks_and_unicode_chars() {
        let ev = |vk, scan, up, unicode| SentKeyEvent {
            vk,
            scan,
            up,
            unicode,
            marker: 0x5350_494B,
        };
        let (mut j, _mock) = mock_journal();
        j.record(JournalEntry::SentInput {
            issue_us: 123,
            accepted: 3,
            events: vec![
                // romaji の VK 送信（'I' の down/up）と、Unicode 送信（'い'）。
                ev(0x49, 0x17, false, false),
                ev(0x49, 0x17, true, false),
                ev(0, 0x3044, false, true),
            ]
            .into_iter()
            .map(Into::into)
            .collect(),
        });
        let json = j.to_json().unwrap();
        assert!(json.contains("SentInput"));
        assert!(json.contains("\"accepted\": 3"));
        // Unicode 送信は文字そのものが読める（「いいい」のような出力の突き合わせ用）。
        assert!(json.contains("\"ch\": \"い\""));
        // 既定値（up=false/unicode=false/ch=None）は出さず JSON を小さく保つ。
        let down = SentKeyEventSummary::from(ev(0x49, 0x17, false, false));
        let down_json = serde_json::to_string(&down).unwrap();
        assert!(!down_json.contains("up") && !down_json.contains("unicode"));
        assert!(!down_json.contains("ch"));
    }

    #[test]
    fn sent_input_lives_in_actuation_lane() {
        let entry = JournalEntry::SentInput {
            issue_us: 0,
            accepted: 0,
            events: Vec::new(),
        };
        assert_eq!(entry.lane_kind(), LaneKind::Actuation);
    }

    #[test]
    fn sent_input_reserved_at_send_time_keeps_causal_order_when_absorbed_late() {
        let (mut j, _mock) = mock_journal();
        let stamper = j.stamper();
        // 送信時に採番し、その後に別の entry が記録され、最後に（遅れて）SentInput が取り込まれる。
        let (seq, elapsed_ms) = stamper.reserve();
        let later_seq = j.record(make_state_entry());
        assert!(later_seq > seq, "reserve は stamp と同じ連番を共有する");
        j.absorb(JournalEnvelope {
            seq,
            elapsed_ms,
            entry: JournalEntry::SentInput {
                issue_us: 1,
                accepted: 1,
                events: Vec::new(),
            },
        });
        let order: Vec<u64> = j.entries_by_seq().iter().map(|e| e.seq).collect();
        assert_eq!(order, vec![seq, later_seq]);
        assert!(matches!(
            j.entries_by_seq()[0].entry,
            JournalEntry::SentInput { .. }
        ));
    }
}
