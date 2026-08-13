//! IME actuation の feedback（収束確認）方針と帰結の純データ型（ADR-080）。
//!
//! `FeedbackPolicy` はプロファイルごとの feedback 方針テンプレートで、`Copy` な
//! 純データ。`AppImePolicy`（`state/app_ime_policy.rs`）が `default_feedback` として
//! 保持できるよう、実行中の試行状態（attempts 等）は一切持たない。実行時状態を
//! 伴う `Actuation` は runtime 層（`runtime/ime_actuation.rs`）が別途持つ。

use super::event_origin::{EventOrigin, EventSource, Generation};
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
///
/// **ADR-090 §2.B 設計案 1 で 2 値から 4 値へ広げた。** 読み戻し
/// （`ObservationStore::read_back`）の帰結を表すには、旧来の
/// `Confirmed` / `GaveUp` では足りない——`ir_apply_drift_correction` が実際に
/// 分岐している判定は「give-up 後に外界が動いた」（`ExternalChange`）と
/// 「まだ収束していないので再送する」（`Pending`）の 2 つを含んでいた。
/// 旧実装の `ConvergedReceipt` は `converged: bool` しか持たず、
/// `resolution()` がそこから 2 値を再構成する**非可逆**な形だったため、
/// この 2 つを表現できなかった（ADR-089 §9-16）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// 読み戻しが desired と一致した（`Read`）。
    Confirmed,
    /// `max_attempts` 到達で打ち切った（`Blind`）。give-up 後にまだ外界が
    /// 動いていない場合も含む（parked のまま）。
    GaveUp,
    /// give-up 後に「値は不問の」新しい観測が来た（＝外界が動いた）。
    /// 試行を破棄して次 tick でやり直す合図。
    ExternalChange,
    /// まだ収束していない（再送する）。
    Pending,
}

/// 読み戻し（`ReadBack`）の帰結（ADR-089 §2.5、INV-46）。
///
/// # なぜ専用型なのか — 収束偽装を型で不可能にする
///
/// **[`Observed<E>`] にも [`AnyObservation`] にも変換手段を提供しない**
/// （`From` / `Into` / コンストラクタ引数のいずれも無い）。これは ADR-080
/// 不変条件6「`ReadBack` の産物を観測として記録しない」の型化であり、
/// BUG-33 型の**収束偽装**——give-up したのに観測を書いて収束したように
/// 見せる——を構造的に不可能にする。
///
/// 「その API が存在しない」形の保証なので、**release ビルドでも有効で
/// `cfg` にも依存しない**（ADR-089 §8.1）。
///
/// [`Observed<E>`]: super::evidence::Observed
/// [`AnyObservation`]: super::evidence::AnyObservation
///
/// # compile-fail ケース（ADR-089 §7 ケース3）
///
/// 通る双子（witness 構築子を通った観測は `AnyObservation` になれる）:
///
/// ```
/// use awase_windows::state::evidence::{AnyObservation, HeuristicDefault, Observed};
/// use awase_windows::state::ime_event::{HwndId, ImePolicyProfile};
///
/// let observed = Observed::<HeuristicDefault>::at_startup(
///     ImePolicyProfile::ImmCross,
///     true,
///     HwndId(1),
///     0,
/// );
/// let any: AnyObservation = observed.into();
/// assert!(any.open());
/// ```
///
/// `ConvergedReceipt` からは作れない（`Observed<E>` を receipt に
/// 差し替えただけ）:
///
/// ```compile_fail
/// use awase_windows::state::evidence::AnyObservation;
/// use awase_windows::state::ime_actuation::{ConvergedReceipt, Resolution};
///
/// let receipt = ConvergedReceipt::new(Resolution::Confirmed, 3);
/// // error[E0277]: the trait bound `AnyObservation: From<ConvergedReceipt>`
/// //               is not satisfied
/// let _any: AnyObservation = receipt.into();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "ReadBack の帰結は呼び出し元が処理すること（観測へは変換できない、INV-46）"]
pub struct ConvergedReceipt {
    /// **ADR-090 §2.B: `converged: bool` から `Resolution` そのものへ変えた。**
    /// `bool` に潰すと `ExternalChange` / `Pending` が `GaveUp` と区別できず、
    /// `resolution()` が非可逆になる。
    resolution: Resolution,
    attempts: u32,
}

impl ConvergedReceipt {
    /// 唯一の構築経路。`Resolution` と試行回数から作る。
    ///
    /// **`pub` のままにする**（ADR-090 §4.4）——receipt を偽造しても
    /// `AnyObservation` へは変換できない（INV-46 がそこを守っている）ので
    /// 害が無く、テストが receipt を組み立てられるほうが有用である。
    pub const fn new(resolution: Resolution, attempts: u32) -> Self {
        Self {
            resolution,
            attempts,
        }
    }

    /// 収束したか（`Resolution::Confirmed`）。
    #[must_use]
    pub const fn converged(&self) -> bool {
        matches!(self.resolution, Resolution::Confirmed)
    }

    /// この episode で消費した試行回数。
    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    /// 元の `Resolution`。
    #[must_use]
    pub const fn resolution(&self) -> Resolution {
        self.resolution
    }
}

/// `decide_actuation_action` の判定結果。次に actuate すべきか、打ち切るべきか。
///
/// `serde` 導出は `DriftCorrectionFixture`（下記）の `expected` フィールド用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

// ── EventOrigin 配線（ADR-082 Phase 0.5）──────────────────────────────────────
//
// drift correction の actuation 試行に「出所（誰が起こしたか）」と「世代（何回目か）」を
// 型として持たせるための構築経路。runtime 層（`ir_apply_drift_correction`）と journal
// リプレイテストの両方がこの純粋関数を通って `EventOrigin` を組み立てる（構築経路の
// 集約、`.claude/rules/ime-belief-architecture.md`）。`runtime/` は Linux で実行検証
// できないため、`EventOrigin` の中身を決めるロジックはここ（`state`）に置き、Linux で
// ユニットテストする。

/// actuation 試行の出所を表す `EventSource::SelfActuated` の `strategy` 識別子。
///
/// `FeedbackPolicy` から一意に導出する。`DriftCorrectionFixture` は `EventSource` 自体を
/// deserialize せず（`&'static str` のため不可、`state/event_origin.rs` 参照）、`policy`
/// からこの関数で `strategy` を再構築するため、ここが `strategy` 文字列の唯一の定義点。
#[must_use]
pub const fn actuation_strategy(policy: FeedbackPolicy) -> &'static str {
    match policy {
        // Imm32Unavailable / TsfNative（実読み戻し不能）。BUG-43 はこちら。
        FeedbackPolicy::Blind { .. } => "drift_correction_blind",
        // ImmCross 等（実読み戻し可能）。
        FeedbackPolicy::Read { .. } => "drift_correction_read",
    }
}

/// actuation 試行1回分の `EventOrigin` を組み立てる。
///
/// `source` は常に `SelfActuated`（awase 自身の能動的訂正）で、`strategy` は
/// `actuation_strategy(policy)`。`epoch` は「この actuation 系列の何回目の試行か」を
/// `Generation` で表す（`Actuation.attempts` と歩調を合わせて単調増加。target が変わって
/// 新しい `Actuation` になると 0 から振り直す）。
#[must_use]
pub fn actuation_origin(policy: FeedbackPolicy, epoch: Generation) -> EventOrigin {
    EventOrigin::new(
        EventSource::SelfActuated {
            strategy: actuation_strategy(policy),
        },
        epoch,
    )
}

/// actuation 試行1回分の構造化レコード（ADR-082 Phase 0.5）。
///
/// `journal.rs::JournalEntry::ImeActuation` が運ぶペイロード本体。型定義を `state` 層に
/// 置くことで、`#[cfg(windows)]` な `journal` モジュールに依存せず Linux のリプレイテスト
/// （`tests/drift_correction_replay.rs`）からも同じ型で構築・検証できる。Windows の
/// journal 記録と Linux のリプレイが単一の型定義・単一の構築経路（`new`）を共有する
/// （`.claude/rules/ime-belief-architecture.md`「構築経路を集約する」）。
///
/// `Serialize` のみ（journal 書き出し用）。`origin` が `&'static str` を含み `Deserialize`
/// できないため、リプレイ側は生の `ActuationRecord` を deserialize せず、fixture の
/// `(policy, epoch, attempts)` から `new` で再構築して照合する。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ActuationRecord {
    /// 出所（常に `SelfActuated`）と世代。
    pub origin: EventOrigin,
    /// この試行が目指す IME open 状態。
    pub target: bool,
    /// この試行の feedback 方針。
    pub policy: FeedbackPolicy,
    /// tick 開始前の累積試行回数（0-origin）。
    pub attempts: u32,
    /// `decide_actuation_action(policy, attempts)` の判定（`Send`/`GiveUp`）。
    pub action: ActuationAction,
}

impl ActuationRecord {
    /// 唯一の構築経路。`action` は `decide_actuation_action` で一意に決まるため引数に
    /// 取らず内部で導出する（呼び出し元が origin と食い違う action を渡す事故を防ぐ）。
    #[must_use]
    pub fn new(origin: EventOrigin, target: bool, policy: FeedbackPolicy, attempts: u32) -> Self {
        Self {
            origin,
            target,
            policy,
            attempts,
            action: decide_actuation_action(policy, attempts),
        }
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
    /// この tick の `EventOrigin.epoch`（ADR-082 Phase 0.5）。runtime 側で `attempts` と
    /// 歩調を合わせて単調増加する（`Actuation::advance_epoch`）ため、健全なフィクスチャ
    /// では `epoch == attempts`。リプレイはこの一致を検証し、`JournalEntry::ImeActuation`
    /// に積まれる世代の配線が壊れていないことを固定する。
    pub epoch: Generation,
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

    // ── EventOrigin 配線（ADR-082 Phase 0.5）─────────────────────────────────

    #[test]
    fn actuation_strategy_distinguishes_policy() {
        assert_eq!(actuation_strategy(blind(5)), "drift_correction_blind");
        assert_eq!(actuation_strategy(read()), "drift_correction_read");
    }

    #[test]
    fn actuation_origin_is_self_actuated_with_policy_strategy() {
        let origin = actuation_origin(blind(5), Generation::new(3));
        assert_eq!(
            origin.source,
            EventSource::SelfActuated {
                strategy: "drift_correction_blind",
            },
            "actuation は常に SelfActuated（物理でも外部注入でもない）"
        );
        assert!(!origin.source.is_physical());
        assert!(!origin.source.is_injected());
        assert_eq!(origin.epoch, Generation::new(3));
    }

    #[test]
    fn actuation_origin_epoch_tracks_attempt_generation() {
        // 同じ actuation 系列の連続試行では epoch が単調増加する。
        let prev = actuation_origin(blind(5), Generation::new(0));
        let next = actuation_origin(blind(5), Generation::new(1));
        assert!(next.epoch.is_newer_than(prev.epoch));
    }

    #[test]
    fn actuation_origin_round_trips_strategy_from_policy() {
        // fixture は EventSource を deserialize せず policy から strategy を再構築する。
        // その再構築が生の actuation_origin と一致することを固定する。
        for policy in [blind(5), read()] {
            let epoch = Generation::new(7);
            let rebuilt = actuation_origin(policy, epoch);
            assert_eq!(
                rebuilt.source,
                EventSource::SelfActuated {
                    strategy: actuation_strategy(policy),
                }
            );
            assert_eq!(rebuilt.epoch, epoch);
        }
    }
}
