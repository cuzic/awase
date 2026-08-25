//! literal-detect の判定結果を journal へ持ち上げるための純粋データ型。

use crate::state::event_origin::Generation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum LiteralVerdict {
    CompositionConfirmed,
    SuspectedLiteral,
    StaleConfirm,
    VetoExpired,
    SessionSkip,
    PlanSkippedLiteral,
    AbortedNoVerdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DetectRoute {
    CheckNow,
    VisibleFencing,
    SessionFlag,
    PlanDecision,
    ProbeEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DetectPath {
    PerVk,
    Word,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DetectTarget {
    Chrome,
    Tsf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct DetectEvidence {
    pub show_changed: bool,
    pub candidate_visible: bool,
    pub write_delta: u64,
    pub evidence_fresh: bool,

    // ── ここから診断専用フィールド（BUG-75、2026-08-25 追加）──
    //
    // BUG-75（GJI StaleConfirm 回収がromaji全体を再送し「っつかって」のように
    // 促音が増える不具合）の対話設計（Sonnet + Opus 2体、6ラウンド）で検討した
    // 複数の恒久対策案が、それぞれ以下のような実機データを必要とすると判明した。
    // suffix再送方式（一度実装しdevelopにマージしたが致命的欠陥が見つかり
    // revertした、docs/known-bugs.md BUG-75 追補参照）のように「証拠のない
    // 仮定」で実装してから壊れることを繰り返さないため、まずはこのフィールド
    // 群を判定ロジックに一切使わず記録するだけに留め、タスクトレイ不具合報告
    // （ADR-095）経由で実機データが集まってから設計判断する。
    //
    // 各フィールドがどの案の判断材料かは以下の通り:
    /// `GetProcessIoCounters`（既存の public Win32 API、`gji_monitor.rs` が
    /// 既に10msごとにサンプリングしている）の `WriteOperationCount` 差分
    /// （送信前ベースラインから、この verdict 確定時点まで）。
    ///
    /// `write_delta`（バイト量、350B閾値で cold/warm を区別する既存の確認
    /// シグナル）と違い、書き込み"回数"は量に依存しない。子音単体の
    /// per-VK confirm は write_delta が閾値に届かないことがある（BUG-27
    /// 追補5）ため、より粒度の細かい確認シグナルとして機能しうるか実機で
    /// 検証する（write_ops活用案）。
    pub write_ops_delta: u64,
    /// 同 `ReadOperationCount` 差分。cold-start の辞書再読込等の補助情報。
    pub read_ops_delta: u64,
    /// 同 `OtherOperationCount` 差分（パイプ・セクション経由 IPC 等が計上される）。
    pub other_ops_delta: u64,
    /// `gji_last_write_ms()` の生値（verdict 確定時点）。0 = 未観測。
    /// `epoch_send_ms`/`deadline_ms` と突き合わせることで、grace延長案の
    /// 判断材料（実際どれだけの遅延で write evidence が追いついたか、
    /// または追いつかなかったか）になる。
    pub last_write_ms: u64,
    /// この detector の送信時刻（`LiteralDetector::epoch_send_ms`）。
    pub epoch_send_ms: u64,
    /// この VK/word の literal-detect deadline（`plan.literal_detect_ms` 由来）。
    /// grace（`LiteralDetector::EPOCH_FENCE_GRACE_MS`）が deadline に先んじて
    /// verdict を確定させたか、deadline 到達で確定したかを区別する判断材料。
    pub deadline_ms: u64,
    /// SHOW-only confirm の猶予（`EPOCH_FENCE_GRACE_MS`）を実際にどれだけ
    /// 保持してから verdict が確定したか（ms）。`None` = 猶予自体に入らな
    /// かった（write_confirmed 等で即断、または `check_now`/
    /// `visible_fencing_verdict` を経由しない verdict）。
    ///
    /// grace延長案（`EPOCH_FENCE_GRACE_MS` を実測ベースで延ばす）の判断材料。
    /// 現行値20msに対し実際どれだけ待てば`evidence_fresh`になっていたかを
    /// 複数の実機報告から集計できる。
    pub grace_hold_ms: Option<u64>,
    /// この verdict 確定時点で、同一 `cold_seq`（コンポジションセッション）内の
    /// 別のモーラが既に confirm 済みだったか
    /// （`crate::tsf::observer::literal_session_confirmed`）。
    ///
    /// 「session内で最初のモーラだけ ESC 先行が安全」という案（session状態
    /// ベース判断案）の判断材料。BUG-39 により、フォーカス変更等をまたいで
    /// stale になりうる既知の不正確さがあることに注意。
    pub literal_session_confirmed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LiteralDetectFacts {
    pub verdict: LiteralVerdict,
    pub route: DetectRoute,
    pub path: DetectPath,
    pub target: DetectTarget,
    pub vk: Option<u16>,
    pub idx: u16,
    pub last_idx: u16,
    pub evidence: DetectEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LiteralDetectRecord {
    pub cold_seq: Generation,
    pub facts: LiteralDetectFacts,
    pub consecutive_before: u32,
    pub gave_up: bool,
    pub backs: usize,
    pub escape_composition: bool,
    pub session_marked: bool,
    /// BUG-74/ADR-100 決定3 案L: `RawTsfLiteralRecovery`（初回疑い・give-up 双方）で
    /// 送信対象だった romaji。`None` はこの verdict が romaji を持たない（`Composition
    /// Confirmed`/`LiteralDetectNote`/`PlanSkippedLiteral`/`AbortedNoVerdict`）ことを表す
    /// — 空文字列との混同（「記録し忘れ」なのか「そもそも romaji を持たない verdict」
    /// なのか区別できなくなる）を避けるため、`String::new()` ではなく `Option` にする。
    ///
    /// give-up（`gave_up=true`）で romaji が失われる（backspace のみ、再送なし）場合
    /// でも、この記録には**送信予定だった元の romaji**を残す。ADR-100 決定3 が
    /// 「give-up 分岐に reinit 完了確認後の retry を追加する」提案2 を却下した代わりに
    /// 採用した対策（完了通知経路が存在しない・focus 世代照合が未整備 (F6) 等、
    /// 却下理由の詳細は ADR-100 参照）。次に同種の文字消失が報告されたとき、
    /// journal からどの romaji が失われたかを機械可読に復元できるようにする。
    pub romaji: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum LiteralDetectTraceItem {
    VkSent {
        cold_seq: u64,
        vk: u16,
        idx: u16,
        last_idx: u16,
        target: DetectTarget,
    },
    Verdict(LiteralDetectRecord),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct LiteralDetectTrace(pub(crate) Vec<LiteralDetectTraceItem>);
