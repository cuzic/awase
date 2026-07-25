//! IME actuation の feedback（収束確認）方針と帰結の純データ型（ADR-080）。
//!
//! `FeedbackPolicy` はプロファイルごとの feedback 方針テンプレートで、`Copy` な
//! 純データ。`AppImePolicy`（`state/app_ime_policy.rs`）が `default_feedback` として
//! 保持できるよう、実行中の試行状態（attempts 等）は一切持たない。実行時状態を
//! 伴う `Actuation` は runtime 層（`runtime/ime_actuation.rs`）が別途持つ。

use super::ime_event::ObservationSource;

/// Feedback（収束確認）方針。プロファイルごとに `AppImePolicy::default_feedback` として持つ。
///
/// `serde` 導出は ADR-082「第一歩」2. の `DriftCorrectionFixture`（BUG-43 の実機ログを
/// JSON フィクスチャとして固定化する）が `decide_actuation_action` の実引数をそのまま
/// 往復できるようにするため（`ConvClassifyFixture` が `ConvTransition` 等の本番型を
/// 直接シリアライズする既存パターンと同じ）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FeedbackPolicy {
    /// 実読み戻しが可能（ImmCross 等）。
    Read {
        source: ObservationSource,
        deadline: std::time::Duration,
    },
    /// 読み戻し手段が構造的に存在しない（Imm32Unavailable / TsfNative）。
    /// 有限回で必ず打ち切る。
    Blind {
        max_attempts: u32,
        backoff: std::time::Duration,
    },
}

/// actuation 試行の帰結。`GaveUp`/deadline超過時は observations ストアへ一切書き込まない
/// （これを破るとBUG-33と同型の収束偽装バグになる — 絶対に守ること）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Confirmed,
    GaveUp,
}

/// `decide_actuation_action` の判定結果。次に actuate すべきか、打ち切るべきか。
///
/// `serde` 導出は `DriftCorrectionFixture`（下記）の `expected` フィールド用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
// wired up in a follow-up task（runtime 層への配線）。それまでは tests のみが参照する。
#[allow(dead_code)]
pub enum ActuationAction {
    /// まだ試行回数に余裕がある、実際に actuate してよい。
    Send,
    /// `Blind` の `max_attempts` 到達、`Resolution::GaveUp` にする。
    GiveUp,
}

/// `Blind` の有界終端を判定する純粋関数（`runtime`層がLinuxでテストできないため、
/// この核心ロジックだけ`state`層に切り出してある。ADR-080 / BUG-43 参照）。
///
/// `Blind` は `attempts >= max_attempts` で厳密に打ち切る（それ未満では決して諦めず、
/// それ以上でも決して `Send` に戻らない）。`Read` は試行回数だけでは打ち切らず常に
/// `Send` を返す（収束は観測確認で成立し、その終端は別処理が担う）。
#[must_use]
// wired up in a follow-up task（runtime 層への配線）。それまでは tests のみが参照する。
#[allow(dead_code)]
pub fn decide_actuation_action(policy: FeedbackPolicy, attempts: u32) -> ActuationAction {
    match policy {
        FeedbackPolicy::Blind { max_attempts, .. } => {
            if attempts >= max_attempts {
                ActuationAction::GiveUp
            } else {
                ActuationAction::Send
            }
        }
        FeedbackPolicy::Read { .. } => ActuationAction::Send,
    }
}

// ── ジャーナル・リプレイ回帰基盤（ADR-082 第一歩 2./3.）────────────────────────────
//
// BUG-43（`docs/known-bugs.md`）: TsfNative/Blacklist パス（Windows Terminal +
// GJI）で drift correction が observation store にフィードバックされず、675ms の間に
// `apply_ime_open(false)` (`VK_IME_OFF`) を 16 回連続送信し続けた。ADR-080 Phase1 は
// `Actuation`/`FeedbackPolicy::Blind` で試行回数を有界にする型強制を実装済み
// （`IME_ACTUATION_BLIND_MAX_ATTEMPTS`、`state/app_ime_policy.rs`）。
//
// `decide_actuation_action` は journal に記録される実際の呼び出し (`Actuation` 経由)
// ではなく、まだ配線されていない当時の生ログから手で書き起こしたフィクスチャで
// リプレイする（`ConvClassifyFixture` と同じ「実機で観測済みの入力を固定化する」
// 考え方だが、こちらは journal ダンプ経由ではなく known-bugs.md の記述から手で
// 再構成している — 詳細は `DriftCorrectionFixture` のドキュメントコメント参照）。

/// BUG-43 の実機ログを `decide_actuation_action` でリプレイするための固定フィクスチャ。
/// `tests/journals/*.json` に配列として保存し、`tests/drift_correction_replay.rs`
/// （または `journal_replay.rs`）が読み込んで再実行・照合する。
///
/// `ConvClassifyFixture`（`state/conv_classify.rs`）と同じ考え方だが、由来が異なる。
/// `ConvClassifyFixture` は `journal.rs::JournalEntry::ConvClassifyCall` という
/// 専用ジャーナルエントリから実機ダンプをそのまま転記できる。一方 BUG-43 発生当時
/// （ADR-080 Phase1 実装前）は actuation 呼び出しを journal に記録する仕組みが
/// 存在しなかったため、このフィクスチャは `docs/known-bugs.md` BUG-43 節の記述
/// （「675ms の間に16回連続、observe tick 20ms とほぼ同期、`duration_ms` は
/// 84502ms→85176ms と単調増加」）から手で再構成した近似値である。`ticks` の
/// `observed_at_ms` はこの近似の記録用メタデータであり、`decide_actuation_action`
/// の呼び出し自体（`policy`/`attempts` のみが入力）には使わない。
///
/// `decide_actuation_action(policy, attempts) -> ActuationAction` は時刻を取らない
/// 純粋関数なので、このフィクスチャの本質的な入力は `policy` と `ticks[].attempts`
/// の列のみ。BUG-43 は 16 回の drift 検知それぞれが独立に `apply_ime_open(false)` を
/// 送信していた（旧実装は試行回数を数えていなかった）ため、`attempts` はここでは
/// 「ADR-080 Phase1 の `Actuation` があったなら何回目の試行だったか」を
/// 0-origin で振り直した値（0,1,2,...）を入れる。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DriftCorrectionFixture {
    /// 何が起きたバグ/シナリオの記録か（人間可読な短い説明）。
    pub name: String,
    /// 参考: 実機で発生した既知のバグの説明・関連コミット等（任意）。
    #[serde(default)]
    pub note: String,
    /// このアプリ/IME 組み合わせで使われる feedback policy。BUG-43 は
    /// Windows Terminal（TsfNative プロファイル）の `Blind` を使う。
    pub policy: FeedbackPolicy,
    /// 実機ログの連続送信を tick として並べたもの。
    pub ticks: Vec<DriftCorrectionTick>,
}

/// `DriftCorrectionFixture` の1 tick 分。実機ログの1回の
/// `Blacklist drift correction: apply_ime_open(false) → Applied` 発火に対応する。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DriftCorrectionTick {
    /// この tick の時点での累積試行回数（0-origin、tick 開始前の値）。
    pub attempts: u32,
    /// 実機ログの経過時間（ms）。ドキュメント用途のみ（BUG-43 記述からの近似復元、
    /// 上記モジュールコメント参照）、`decide_actuation_action` の判定には使わない。
    #[serde(default)]
    pub observed_at_ms: Option<u64>,
    /// `decide_actuation_action(fixture.policy, attempts)` の期待される結果。
    pub expected: ActuationAction,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blind(max_attempts: u32) -> FeedbackPolicy {
        FeedbackPolicy::Blind {
            max_attempts,
            backoff: std::time::Duration::from_millis(0),
        }
    }

    fn read() -> FeedbackPolicy {
        FeedbackPolicy::Read {
            source: ObservationSource::ImmGetOpenStatus,
            deadline: std::time::Duration::from_millis(0),
        }
    }

    #[test]
    fn blind_sends_before_reaching_max() {
        for attempts in 0..3 {
            assert_eq!(
                decide_actuation_action(blind(3), attempts),
                ActuationAction::Send,
                "attempts={attempts} は max_attempts=3 未満なので Send のはず"
            );
        }
    }

    #[test]
    fn blind_gives_up_exactly_at_max() {
        assert_eq!(
            decide_actuation_action(blind(3), 3),
            ActuationAction::GiveUp,
            "attempts == max_attempts の厳密境界で GiveUp"
        );
    }

    #[test]
    fn blind_stays_gave_up_past_max() {
        assert_eq!(
            decide_actuation_action(blind(3), 4),
            ActuationAction::GiveUp,
            "境界を越えても Send に戻らない"
        );
    }

    #[test]
    fn read_always_sends() {
        for attempts in [0, 1, 3, 4, 100, u32::MAX] {
            assert_eq!(
                decide_actuation_action(read(), attempts),
                ActuationAction::Send,
                "Read は試行回数で打ち切らない (attempts={attempts})"
            );
        }
    }
}
