//! 観測値ストア (Step 3)
//!
//! `ime_observations.rs` の `focus_probe` + `observer_poll` 等を、
//! **per-source の構造化ストア** に置換する。
//! 単一の `latest` スロットに圧縮するのではなく、reducer が
//! 「複数ソースの合意」「観測の鮮度」「ドリフト継続時間」を判断材料に
//! 使えるよう情報を保持する。
//!
//! ## 絶対ルール
//!
//! `observed.latest = Some(obs)` はする。
//! `desired_open = obs.open` は禁止。
//! Observer は health checker / drift detector の役割に徹する。

use std::time::{Duration, Instant};

use super::evidence::{ActuatingPool, AnyObservation, BeliefPool, Observed, OpenEvidence};
use super::ime_actuation::{ConvergedReceipt, Resolution};
use super::ime_event::{HwndId, ObservationAuthority, ObservationConfidence, ObservationSource};
use super::probe_admission::FocusEpoch;

/// 読み戻しの**問い**（ADR-080 不変条件6 / ADR-089 INV-46 / ADR-090 §2.B）。
///
/// `ObservationStore::read_back` の引数。「since 以降の観測を見る」という
/// 同じ機械的操作でも、drift correction が問うている意味は 2 通りあり、
/// **その区別を型として宣言させる**のがこの enum の役割である。
/// `ime_refresh.rs:640` 付近のコメントが説明しているとおり、この 2 つを
/// 取り違えると「give-up が即座に無効化される」か「無限再送」のどちらかに
/// 落ちる（BUG-33 / BUG-43 ファミリー）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadBackQuery {
    /// `since` 以降の trusted 観測が `desired` と一致したか（`Read` の収束確認）。
    ///
    /// 一致 → `Resolution::Confirmed`、それ以外 → `Resolution::Pending`。
    Converged { desired: bool },
    /// `since` 以降に trusted 観測が**何であれ**記録されたか
    /// （`Blind` give-up からの復旧判定。**値ではなく鮮度だけを見る**）。
    ///
    /// 記録あり → `Resolution::ExternalChange`、無し → `Resolution::GaveUp`。
    ///
    /// **値で判定してはならない。** drift 補正は `observed != desired` が続く
    /// 間しか走らず、open/close は bool なので「間違った値」は `!desired` の
    /// 1 通りしか存在しない。「target と異なる値の観測が来たら復旧」は
    /// ほぼ毎 tick 真になり give-up を即座に無効化する（乖離の定義そのもの
    /// だから）。意味のある信号は「諦めた時刻以降に**新しい**観測が record
    /// されたか」＝世界で何かが動いたか、である。
    AnyFreshEvidence,
}

/// 単一の観測値レコード。受理済みの観測のみがここに格納される。
///
/// `focus_epoch` は観測が受理された時点のフォーカスエポック。
/// 同期 probe は呼び出し時点のエポック（= 現在のフォーカス）を持つ。
/// 非同期 probe は `ImmLikeTicket::admit()` が照合したエポックを持つ。
///
/// # なぜ `#[non_exhaustive]` なのか（ADR-090 §2.C、INV-49）
///
/// `#[non_exhaustive]` は **crate 外からの構造体リテラル構築と網羅的分配束縛を
/// 禁じ、フィールドの読み取りは許す**。ADR-089 Phase B が
/// `PerSourceObservations::set` を `pub(crate)` へ縮小した時点では、
/// `store.per_source.observer_poll = Some(ImeObservation { source: .., .. })`
/// というフィールド直接代入が crate 外に残っていた（`set` は「フィールド代入の
/// 便利メソッド」であって唯一の入口ではなかった）。`per_source` の
/// `pub(crate)` 化（下記）と本属性の 2 つで、crate 外から任意の
/// `ObservationSource` / `ObservationConfidence` を名乗る観測を注入する経路が
/// 構造的に消える。
///
/// **フィールドを private にはしない**——読み取り側（`tests/golden_scenarios.rs`、
/// `derive_*`、drift 判定）まで壊れてアクセサを 7 本生やすことになる
/// （ADR-090 §4.5）。欲しい保証（構築だけを禁じる）と過不足なく一致するのが
/// `#[non_exhaustive]` である。
///
/// # compile-fail ケース（ADR-089 §9-14 の「通る双子」併記規約）
///
/// 通る双子（crate 外からの**読み取り**は塞がない）:
///
/// ```
/// use awase_windows::state::ime_event::ObservationSource;
/// use awase_windows::state::observation_store::ObservationStore;
///
/// let store = ObservationStore::default();
/// // 読み取り専用アクセサは crate 外から使える。
/// assert!(store.observation(ObservationSource::ObserverPoll).is_none());
/// ```
///
/// crate 外からは構築できない:
///
/// ```compile_fail
/// use awase_windows::state::ime_event::{HwndId, ObservationConfidence, ObservationSource};
/// use awase_windows::state::observation_store::ImeObservation;
///
/// // error[E0639]: cannot create non-exhaustive struct using struct expression
/// let _obs = ImeObservation {
///     open: true,
///     source: ObservationSource::ImmGetOpenStatus,
///     at: std::time::Instant::now(),
///     hwnd: HwndId(1),
///     confidence: ObservationConfidence::High,
///     expires_at: None,
///     focus_epoch: 0,
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImeObservation {
    pub open: bool,
    pub source: ObservationSource,
    /// 観測タイムスタンプ (鮮度・経過時間計算用)
    pub at: Instant,
    /// どのウィンドウで観測したか (フォーカス変更後の stale 検出用)
    pub hwnd: HwndId,
    /// 観測の信頼度 (profile 別の judge に使う)
    pub confidence: ObservationConfidence,
    /// この観測値の有効期限 (フォーカス変更で expire させたい場合等)
    pub expires_at: Option<Instant>,
    /// 観測が受理されたフォーカスエポック。診断・デバッグ用。
    /// 同期 probe = 呼び出し時の現在エポック。
    /// 非同期 probe = admit() が照合したエポック。
    pub focus_epoch: FocusEpoch,
}

impl ImeObservation {
    /// 有効期限を過ぎていないか
    #[must_use]
    pub fn is_expired(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|exp| now >= exp)
    }

    /// `now` からの経過時間
    #[must_use]
    pub fn age(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.at)
    }
}

/// ソース別の最新観測値。各ソースで独立に最新値を保持する。
///
/// **全フィールドは ADR-090 §2.C（INV-49）で `pub(crate)` へ縮小した。**
/// crate 外からの読み取りは `ObservationStore::observation(source)` を使うこと
/// （`PerSourceObservations::get` と同じもの）。書き込み口は crate 内でも
/// `set`（`record_any` の 1 呼び出しのみ）に限定されており、フィールドへの
/// 直接代入が本番コードに存在しないことを
/// `tests/architecture_guard.rs::per_source_fields_are_not_assigned_directly`
/// が固定する。
#[derive(Debug, Default, Clone)]
pub struct PerSourceObservations {
    pub(crate) focus_probe: Option<ImeObservation>,
    pub(crate) observer_poll: Option<ImeObservation>,
    pub(crate) gji: Option<ImeObservation>,
    pub(crate) imm_get_open_status: Option<ImeObservation>,
    pub(crate) tsf: Option<ImeObservation>,
    pub(crate) hwnd_cache: Option<ImeObservation>,
    /// フォーカス変更後の ImmCross 非同期プローブ（Qt/LINE 等の child hwnd 高信頼読み取り）
    pub(crate) imm_cross_probe: Option<ImeObservation>,
    /// 観測が一切ない場合の安全デフォルト推測（常に Low confidence）
    pub(crate) heuristic_default: Option<ImeObservation>,
    /// conv ビットからの open 状態推定（`KatakanaShadowOff`/`NativeToggleShadowOff`）。
    /// `ConvBitsInference`（input_mode 専用）とは別枠で、こちらは open/close の
    /// 観測として正式に扱う。常に `ObservationConfidence::Medium` 以下で record される。
    pub(crate) conv_open_inference: Option<ImeObservation>,
}

impl PerSourceObservations {
    /// 指定ソースの最新値を返す
    #[must_use]
    pub const fn get(&self, source: ObservationSource) -> Option<&ImeObservation> {
        match source {
            ObservationSource::FocusProbe => self.focus_probe.as_ref(),
            ObservationSource::ObserverPoll => self.observer_poll.as_ref(),
            ObservationSource::Gji => self.gji.as_ref(),
            ObservationSource::ImmGetOpenStatus => self.imm_get_open_status.as_ref(),
            ObservationSource::Tsf => self.tsf.as_ref(),
            ObservationSource::HwndCache => self.hwnd_cache.as_ref(),
            ObservationSource::ImmCrossProbe => self.imm_cross_probe.as_ref(),
            ObservationSource::HeuristicDefault => self.heuristic_default.as_ref(),
            ObservationSource::ConvOpenInference => self.conv_open_inference.as_ref(),
            // ConvBitsInference / GjiIoInference は input_mode 専用ソースで
            // ON/OFF 観測としては記録されない。
            ObservationSource::ConvBitsInference | ObservationSource::GjiIoInference => None,
        }
    }

    /// 指定ソースの最新値をセットする。
    ///
    /// **ADR-089 Phase B（§9-11、2026-08-12）で `pub(crate)` へ縮小した。**
    /// crate の外（統合テストや将来の別 crate）からこの口を使うと、
    /// `Observed<E>` の witness 構築子（§2.2）も `record`/`record_belief` の
    /// プール分離（§2.1）も経由せずに任意の `ObservationSource` /
    /// `ObservationConfidence` を名乗った観測を注入できてしまう。
    /// crate 内の唯一の本番呼び出し元は `ObservationStore::record_replayed`
    /// （`tests/architecture_guard.rs::per_source_set_is_confined_to_the_store`
    /// が固定する）。
    pub(crate) const fn set(&mut self, source: ObservationSource, obs: ImeObservation) {
        match source {
            ObservationSource::FocusProbe => self.focus_probe = Some(obs),
            ObservationSource::ObserverPoll => self.observer_poll = Some(obs),
            ObservationSource::Gji => self.gji = Some(obs),
            ObservationSource::ImmGetOpenStatus => self.imm_get_open_status = Some(obs),
            ObservationSource::Tsf => self.tsf = Some(obs),
            ObservationSource::HwndCache => self.hwnd_cache = Some(obs),
            ObservationSource::ImmCrossProbe => self.imm_cross_probe = Some(obs),
            ObservationSource::HeuristicDefault => self.heuristic_default = Some(obs),
            ObservationSource::ConvOpenInference => self.conv_open_inference = Some(obs),
            // ConvBitsInference / GjiIoInference は InputModeObserved 専用ソースで
            // ObserverReported（ON/OFF 観測）としては dispatch されないため、
            // この store には記録されない。
            ObservationSource::ConvBitsInference | ObservationSource::GjiIoInference => {}
        }
    }

    /// 全ソースの観測値を iter (Some のみ)
    pub fn iter(&self) -> impl Iterator<Item = &ImeObservation> {
        [
            self.focus_probe.as_ref(),
            self.observer_poll.as_ref(),
            self.gji.as_ref(),
            self.imm_get_open_status.as_ref(),
            self.tsf.as_ref(),
            self.hwnd_cache.as_ref(),
            self.imm_cross_probe.as_ref(),
            self.heuristic_default.as_ref(),
            self.conv_open_inference.as_ref(),
        ]
        .into_iter()
        .flatten()
    }

    /// 全ソースを clear する (フォーカス変更時用)
    pub const fn clear_all(&mut self) {
        self.focus_probe = None;
        self.observer_poll = None;
        self.gji = None;
        self.imm_get_open_status = None;
        self.tsf = None;
        self.hwnd_cache = None;
        self.imm_cross_probe = None;
        self.heuristic_default = None;
        self.conv_open_inference = None;
    }
}

/// desired と observed の乖離追跡 (DriftDetected event の根拠)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImeDrift {
    pub started_at: Instant,
}

/// `derive_any()` / `derive_actuating()` の診断付き結果。「どのソースが決定打だったか」を
/// 保持する（ADR-087 §2.3 P15 Step 3 が `WarrantBasis::DirectRead`/
/// `Corroborated` を構築するために必要）。
///
/// `MediumConsensus` は `first`/`second` の固定2フィールドで表現し `Vec` を
/// 使わない——`effective_open()`/`effective_open_at()` は全 `KeyDown` で
/// 呼ばれる（`key_pipeline.rs::build_input_context`）ホットパスであり、
/// 打鍵ごとのヒープ確保を避けるため（ADR-087 §7 round4 S-A）。3ソース以上が
/// 合意した場合、`second` 以降のソースは診断上「2件以上合意した」ことが
/// わかれば十分なので切り捨てる（`WarrantBasis::Corroborated` も2件しか
/// 持たない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeriveOutcome {
    /// High confidence の単一ソースで即採用。
    HighSingle {
        source: ObservationSource,
        open: bool,
    },
    /// Medium+ ソースの無競合多数決。`second` が `None` なら実質「単独の間接
    /// 観測」、`Some` なら複数ソースの合意（corroboration）。
    MediumConsensus {
        first: ObservationSource,
        second: Option<ObservationSource>,
        open: bool,
    },
}

impl DeriveOutcome {
    #[must_use]
    pub const fn value(&self) -> bool {
        match self {
            Self::HighSingle { open, .. } | Self::MediumConsensus { open, .. } => *open,
        }
    }
}

/// 観測値ストア (Step 3 の SSOT)。
///
/// reducer は以下のような問い合わせができる:
/// - `per_source.get(source)` — 特定ソースの最新値
/// - `most_recent_trusted()` — confidence + age で最も信頼できる観測
/// - `consensus(window)` — 直近 N 内の複数ソース合意
/// - `drift.is_some()` — desired と乖離しているか
/// - `is_source_flapping(source, window)` — 短期間で flapping しているか (今後実装)
#[derive(Debug, Default, Clone)]
pub struct ObservationStore {
    /// **ADR-090 §2.C（INV-49）で `pub` から `pub(crate)` へ縮小した。**
    /// crate 外からの読み取りは `observation(source)` を使う。
    pub(crate) per_source: PerSourceObservations,
    /// desired との乖離追跡
    pub drift: Option<ImeDrift>,
    /// 現在のフォーカスエポック。`FocusChanged` イベントで更新される。
    ///
    /// `derive_any()` が `ImmCrossProbe` / `FocusProbe` 観測を epoch フィルタする際に参照する。
    /// これにより、stale な高信頼観測が意思決定に使われることを防ぐ。
    pub current_focus_epoch: FocusEpoch,
}

impl ObservationStore {
    /// Actuating プールの観測を記録する（ADR-089 §2.1、INV-38）。
    ///
    /// `Pool = ActuatingPool` の evidence 型しか渡せない——BeliefOnly の観測を
    /// actuation 根拠のプールへ入れることは、関連型の不一致としてコンパイル
    /// エラーになる。
    pub fn record<E: OpenEvidence<Pool = ActuatingPool>>(
        &mut self,
        observed: Observed<E>,
        at: Instant,
    ) {
        self.record_any(observed.into(), at);
    }

    /// BeliefOnly プールの観測を記録する（ADR-089 §2.1、INV-38）。
    pub fn record_belief<E: OpenEvidence<Pool = BeliefPool>>(
        &mut self,
        observed: Observed<E>,
        at: Instant,
    ) {
        self.record_any(observed.into(), at);
    }

    /// 値としての観測を記録する唯一の口（ADR-089 §2.1 末尾）。
    ///
    /// `ImeEvent::ObserverReported`（journal へ直列化される値、ADR-082）の
    /// reduce と journal replay がここを通る。型で消せない実行時 match の残余を
    /// 1 箇所に集めるための関数であり、**本番の観測は必ず `Observed<E>` の
    /// witness 構築子を通ってここへ来る**（`AnyObservation` の他の構築経路は
    /// `restored_from_journal` だけで、本番からは呼ばない）。
    pub fn record_replayed(&mut self, observed: AnyObservation, at: Instant) {
        self.record_any(observed, at);
    }

    fn record_any(&mut self, observed: AnyObservation, at: Instant) {
        let source = observed.source();
        // input_mode 専用の 2 ソースは open 観測プールに構造的に入らない。
        // 現行 `PerSourceObservations::set` の no-op と同一の挙動を保つ
        // （ここだけ挙動を変えると journal リプレイが本番と別の状態を再現する）。
        if matches!(
            source,
            ObservationSource::ConvBitsInference | ObservationSource::GjiIoInference
        ) {
            log::debug!("[observation] {source:?} は open 観測プールに入らないため破棄");
            return;
        }
        self.per_source.set(
            source,
            ImeObservation {
                open: observed.open(),
                source,
                at,
                hwnd: observed.hwnd(),
                confidence: observed.confidence(),
                expires_at: None,
                focus_epoch: observed.focus_epoch(),
            },
        );
    }

    /// 指定ソースの最新観測（**読み取り専用**、ADR-090 §2.C 設計案 1）。
    ///
    /// `per_source` を `pub(crate)` へ縮小した代わりの公開窓口。
    /// 「書き込みの裏口を塞ぐのに読み取りを犠牲にする必要はない」——
    /// crate 外（統合テスト）が特定ソースの観測を検査したいという用途は
    /// 正当なので、読み取りだけを 1 本のアクセサで通す。
    #[must_use]
    pub const fn observation(&self, source: ObservationSource) -> Option<&ImeObservation> {
        self.per_source.get(source)
    }

    /// 全ソースを clear する (フォーカス変更時用)。drift も clear。
    ///
    /// `new_epoch` には `FocusStore::focus_epoch` のインクリメント後の値を渡す。
    /// これ以降 `derive_any()` は古い epoch の ImmCrossProbe / FocusProbe を無視する。
    pub fn clear_on_focus_change(&mut self, new_epoch: FocusEpoch) {
        self.per_source.clear_all();
        self.drift = None;
        self.current_focus_epoch = new_epoch;
    }

    /// desired と observed の乖離を更新する。
    ///
    /// `observed` が `desired` と一致する場合は drift を clear。
    /// 不一致が継続するなら drift を保持し続ける (started_at は更新しない)。
    pub const fn update_drift(&mut self, desired: bool, observed: bool, now: Instant) {
        if desired == observed {
            self.drift = None;
            return;
        }
        if self.drift.is_none() {
            self.drift = Some(ImeDrift { started_at: now });
        }
    }

    /// 乖離継続時間を返す
    #[must_use]
    pub fn drift_duration(&self, now: Instant) -> Option<Duration> {
        self.drift
            .map(|d| now.saturating_duration_since(d.started_at))
    }

    /// `HeuristicDefault` 観測を返す（実在すれば）。ADR-087 §2.3 P15 Step 4a
    /// （`open_warrant.rs`）が使う。
    ///
    /// **`derive_any()` / `derive_actuating()`（Step 3）と異なり `FRESH`（3秒）の鮮度窓を
    /// 適用しない**——`is_expired()`（`expires_at`）のみを見る。これは意図的:
    /// `ObserverReported` 経由で記録される観測は `expires_at: None`（無期限）
    /// が通常であり、`HeuristicDefault` は「観測が一切無いときの安全
    /// デフォルト」という性質上、Step 3 のような鮮度による失効を課さない
    /// （ADR-087 INV-22 撤回の方針: belief/actuation の根拠に鮮度上限を
    /// 追加で持ち込まない、§7 round2 M3・round4 S-C）。将来 Step 3 と
    /// 揃えて鮮度窓を足す変更をする場合は、これが BUG-16 の回復力を削らないか
    /// 確認すること。
    #[must_use]
    pub fn heuristic_default(&self, now: Instant) -> Option<&ImeObservation> {
        self.per_source
            .get(ObservationSource::HeuristicDefault)
            .filter(|o| !o.is_expired(now))
    }

    /// actuation の**読み戻し**の唯一の公開窓口（ADR-080 不変条件6 /
    /// ADR-089 INV-46 / ADR-090 §2.B 設計案 2、INV-52）。
    ///
    /// # なぜ `ConvergedReceipt` を返すのか
    ///
    /// **戻り値は `ImeObservation` ではない**——したがって「読み戻しの産物を
    /// 観測として書き戻す」ことが型として書けない。`ConvergedReceipt` は
    /// `Observed<E>` にも `AnyObservation` にも変換手段を持たないので
    /// （INV-46）、BUG-33 型の収束偽装が構造的に不可能になる。
    ///
    /// ADR-089 Phase C の時点では `ConvergedReceipt` は構築されるだけで
    /// `log::debug!` にしか渡らず、実際の収束判定は
    /// `most_recent_trusted_after` が返す `ImeObservation` が担っていた
    /// （§9-16「効いていない」）。本メソッドと `most_recent_trusted_after` の
    /// module private 化で、初めてコンパイラ強制になる。
    ///
    /// # `most_recent_trusted`（`_after` 無し）との違い
    ///
    /// あちらは **belief のフォールバック専用**（`ime_model.rs` /
    /// `platform_state.rs`）であり、actuation の読み戻しではないので `pub` の
    /// まま残す。**この 2 つを混同しないことが本設計の要点**である
    /// （混同すると belief の読み取りまで receipt 越しになり ADR-078 の
    /// 3 層分離が壊れる）。
    pub fn read_back(
        &self,
        now: Instant,
        since: Instant,
        query: ReadBackQuery,
        attempts: u32,
    ) -> ConvergedReceipt {
        let latest = self.most_recent_trusted_after(now, since);
        let resolution = match query {
            // 旧: `most_recent_trusted_after(now, act_sent_at).is_some_and(|o| o.open == desired)`
            ReadBackQuery::Converged { desired } => {
                if latest.is_some_and(|o| o.open == desired) {
                    Resolution::Confirmed
                } else {
                    Resolution::Pending
                }
            }
            // 旧: `most_recent_trusted_after(now, gave_up_at).is_some()`
            ReadBackQuery::AnyFreshEvidence => {
                if latest.is_some() {
                    Resolution::ExternalChange
                } else {
                    Resolution::GaveUp
                }
            }
        };
        ConvergedReceipt::new(resolution, attempts)
    }

    /// 最も信頼できる観測値を返す (confidence 優先、同 confidence なら新しい方)。
    ///
    /// expire 済みの観測は除外する。
    ///
    /// **`_after` 付きと違い `pub` のまま**（ADR-090 §2.B 設計案 2）。
    /// こちらは belief のフォールバック（`ime_model.rs::resolve_open_at` /
    /// `platform_state.rs`）専用であり、actuation の読み戻しではない。
    #[must_use]
    pub fn most_recent_trusted(&self, now: Instant) -> Option<&ImeObservation> {
        self.per_source
            .iter()
            .filter(|o| !o.is_expired(now))
            .max_by(|a, b| a.confidence.cmp(&b.confidence).then(a.at.cmp(&b.at)))
    }

    /// `most_recent_trusted(now)` と同じロジックに、`since` 以降に record された観測のみを
    /// 対象にする条件を追加したもの。actuation を送信した時刻以降の観測だけを見て収束判定
    /// したい場合に使う（BUG-43対策、ADR-080参照）。全ソース対象（`ConvOpenInference`を
    /// 除外しない — BUG-43の直接トリガーだったソースを含めないと意味がないため）。
    ///
    /// **ADR-090 §2.B（INV-52）で `pub` から module private へ縮小した。**
    /// `ImeObservation` を返すこの口が公開されている限り、`read_back()` を
    /// 足しても「読み戻しの産物は `ImeObservation` として手に入らない」
    /// （ADR-080 不変条件6）は成立しない——**B の本体は receipt を作ることでは
    /// なく、この since-fenced 読み口を塞ぐことである**。新しい読み戻し用途が
    /// 出たら `ReadBackQuery` に variant を足すのが正しい形であり、それが
    /// 「読み戻しの意味を宣言させる」という本設計の狙いである（B-R2）。
    #[must_use]
    fn most_recent_trusted_after(&self, now: Instant, since: Instant) -> Option<&ImeObservation> {
        self.per_source
            .iter()
            .filter(|o| !o.is_expired(now) && o.at >= since)
            .max_by(|a, b| a.confidence.cmp(&b.confidence).then(a.at.cmp(&b.at)))
    }

    /// 両プールをマージしてから 1 回だけ判定する、open の best-effort belief
    /// 導出（ADR-089 §2.1、INV-39。旧 `derive_open` / `derive_open_filtered(|_| true)`
    /// 相当）。
    ///
    /// 戻り値は「どのソースが決定打だったか」まで含む `DeriveOutcome` であり、
    /// `bool` へ潰さない——`state/open_warrant.rs` が `WarrantBasis::DirectRead` /
    /// `Corroborated` / `SingleIndirect` の構築に使う（ADR-087）。`bool` だけが
    /// 欲しい呼び出し元は `DeriveOutcome::value()` を使うこと。
    ///
    /// ## 判定順序
    ///
    /// 1. **High confidence** — 単一ソースでも即採用（ImmGetOpenStatus 直接 / ImmCrossProbe）
    /// 2. **Medium+ ソースの無競合多数決** — 複数の間接観測が一致した場合のみ採用
    ///    - 矛盾（true/false 両方あり）の場合は `None`
    ///
    /// `None` の場合は呼び出し側が `desired_open` にフォールバックする。
    ///
    /// ## 鮮度ウィンドウ
    ///
    /// `FRESH` を超えた観測は無視する。フォーカス変更時に `clear_on_focus_change()` が
    /// 呼ばれるため通常は問題にならないが、稀に残留する古い観測を排除するためのガード。
    ///
    /// ## Epoch フィルタ（ImmCrossProbe / FocusProbe のみ）
    ///
    /// これらの probe は async または first-key トリガーのため、フォーカス変更後に
    /// 古いウィンドウの観測が混入するリスクがある。`current_focus_epoch` と照合し、
    /// epoch が異なる観測を排除する。
    /// GJI / ObserverPoll / TSF はイベント駆動または周期同期のため epoch フィルタ対象外。
    #[must_use]
    pub fn derive_any(&self, now: Instant) -> Option<DeriveOutcome> {
        self.derive_filtered(now, |_| true)
    }

    /// Actuating プールの観測だけから導く（ADR-087 §2.3 P15 Step 3 の入力）。
    ///
    /// `derive_any` との差はプールの絞り込みだけで、判定本体（High単独 →
    /// Medium無競合多数決）は共通である。**プール毎に判定してから合成する形は
    /// 提供しない**——BeliefOnly プール単独の結論が Actuating 側の High を
    /// 上書きしうる経路は BUG-19 の再発条件と同型（INV-39、ADR-089 §4.6）。
    #[must_use]
    pub fn derive_actuating(&self, now: Instant) -> Option<DeriveOutcome> {
        self.derive_filtered(now, |source| {
            source.authority() == ObservationAuthority::Actuating
        })
    }

    /// `derive_any` / `derive_actuating` の共通本体。
    ///
    /// 述語を取る形は **private のまま**にする——公開すると呼び出し側が
    /// 任意のプール分割で derive でき、INV-39 が構造的でなくなる。
    fn derive_filtered(
        &self,
        now: Instant,
        accept: impl Fn(ObservationSource) -> bool,
    ) -> Option<DeriveOutcome> {
        const FRESH: Duration = Duration::from_secs(3);
        let current_epoch = self.current_focus_epoch;

        let is_fresh = |o: &ImeObservation| !o.is_expired(now) && o.age(now) <= FRESH;

        // epoch 照合が必要なソース（async/first-key トリガーのスナップショット probe）
        let is_epoch_ok = |o: &ImeObservation| match o.source {
            ObservationSource::ImmCrossProbe | ObservationSource::FocusProbe => {
                o.focus_epoch == current_epoch
            }
            _ => true,
        };

        // 1. High confidence: 単一ソースで即採用（最新のものを選ぶ）
        let high = self
            .per_source
            .iter()
            .filter(|o| {
                accept(o.source)
                    && is_fresh(o)
                    && is_epoch_ok(o)
                    && o.confidence == ObservationConfidence::High
            })
            .max_by_key(|o| o.at);
        if let Some(obs) = high {
            return Some(DeriveOutcome::HighSingle {
                source: obs.source,
                open: obs.open,
            });
        }

        // 2. Medium+ ソースの無競合多数決（1 ソースでも可）。ホットパスのため
        // Vec を使わず、最初の2ソースだけを固定フィールドに保持する
        // （§7 round4 S-A）。`.expect()` に頼らず `Option` の組で分岐できる
        // よう `true_first`/`false_first` の Some/None だけで判定する。
        let mut true_first: Option<ObservationSource> = None;
        let mut true_second: Option<ObservationSource> = None;
        let mut false_first: Option<ObservationSource> = None;
        let mut false_second: Option<ObservationSource> = None;
        for obs in self.per_source.iter() {
            if !accept(obs.source)
                || !is_fresh(obs)
                || !is_epoch_ok(obs)
                || obs.confidence < ObservationConfidence::Medium
            {
                continue;
            }
            if obs.open {
                if true_first.is_none() {
                    true_first = Some(obs.source);
                } else if true_second.is_none() {
                    true_second = Some(obs.source);
                }
            } else if false_first.is_none() {
                false_first = Some(obs.source);
            } else if false_second.is_none() {
                false_second = Some(obs.source);
            }
        }
        match (true_first, false_first) {
            (Some(first), None) => Some(DeriveOutcome::MediumConsensus {
                first,
                second: true_second,
                open: true,
            }),
            (None, Some(first)) => Some(DeriveOutcome::MediumConsensus {
                first,
                second: false_second,
                open: false,
            }),
            _ => None, // 矛盾（両方 Some）または観測なし（両方 None）→ フォールバック
        }
    }

    /// 直近 `window` 内に複数ソースが同じ値で合意しているか。
    ///
    /// 2 ソース以上が同じ値を見ていれば `Some(value)` を返す。
    /// 値が分かれる、または 1 ソースしかない場合は `None`。
    #[must_use]
    pub fn consensus(&self, window: Duration, now: Instant) -> Option<bool> {
        let mut votes_true = 0;
        let mut votes_false = 0;
        for obs in self.per_source.iter() {
            if obs.age(now) > window || obs.is_expired(now) {
                continue;
            }
            if obs.open {
                votes_true += 1;
            } else {
                votes_false += 1;
            }
        }
        if votes_true >= 2 && votes_false == 0 {
            Some(true)
        } else if votes_false >= 2 && votes_true == 0 {
            Some(false)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用の直接記録。`record`/`record_belief` は evidence 型（= 出自）を
    /// 要求するため、任意のソース・任意の `expires_at` を仕込みたいテストは
    /// per-source ストアへ直接書く（ADR-089 §2.1 の witness 規律は本番経路の
    /// 話であり、ストア自体の単体テストはその外側にある）。
    fn rec(store: &mut ObservationStore, o: ImeObservation) {
        store.per_source.set(o.source, o);
    }

    fn obs(open: bool, source: ObservationSource, at: Instant) -> ImeObservation {
        ImeObservation {
            open,
            source,
            at,
            hwnd: HwndId::NULL,
            confidence: ObservationConfidence::Medium,
            expires_at: None,
            focus_epoch: 0,
        }
    }

    #[test]
    fn per_source_get_and_set() {
        let mut p = PerSourceObservations::default();
        let now = Instant::now();
        let o = obs(true, ObservationSource::Gji, now);
        p.set(ObservationSource::Gji, o);
        assert_eq!(p.get(ObservationSource::Gji).map(|x| x.open), Some(true));
        assert_eq!(p.get(ObservationSource::Tsf), None);
    }

    /// BUG-19 再発対策: `ConvOpenInference` は `ConvBitsInference`/`GjiIoInference`
    /// (input_mode 専用、常に記録されない no-op) とは異なり、正式な open/close
    /// 観測として `PerSourceObservations` に記録・取得できることを固定する。
    #[test]
    fn conv_open_inference_is_recorded_unlike_conv_bits_inference() {
        let mut p = PerSourceObservations::default();
        let now = Instant::now();
        p.set(
            ObservationSource::ConvOpenInference,
            obs(true, ObservationSource::ConvOpenInference, now),
        );
        assert_eq!(
            p.get(ObservationSource::ConvOpenInference).map(|x| x.open),
            Some(true),
            "ConvOpenInference は open 観測として記録される"
        );

        // 対照: ConvBitsInference/GjiIoInference は input_mode 専用のため
        // set() が no-op で get() は常に None のまま (既存の設計)。
        p.set(
            ObservationSource::ConvBitsInference,
            obs(true, ObservationSource::ConvBitsInference, now),
        );
        assert_eq!(p.get(ObservationSource::ConvBitsInference), None);
    }

    /// ADR-089 §6 Phase A item 5 / INV-38: `ConvBitsInference` /
    /// `GjiIoInference` は open 観測プールに構造的に入らない。
    ///
    /// この 2 値に `OpenEvidence` を impl しない判断（§1.3(h)・§2.1）は、
    /// 「impl すると `record_belief` が黙って no-op になる」ことを根拠に
    /// している。その前提——`get` が `None` を返し続け、値経由の
    /// `record_replayed` でも記録されないこと——をここで固定する。
    #[test]
    fn input_mode_only_sources_never_enter_the_open_pool() {
        use super::super::evidence::AnyObservation;
        let now = Instant::now();
        for source in [
            ObservationSource::ConvBitsInference,
            ObservationSource::GjiIoInference,
        ] {
            let mut p = PerSourceObservations::default();
            p.set(source, obs(true, source, now));
            assert_eq!(
                p.get(source),
                None,
                "{source:?} は set しても get で見えない（input_mode 専用）"
            );

            let mut s = ObservationStore::default();
            s.record_replayed(
                AnyObservation::restored_from_journal(
                    true,
                    source,
                    HwndId::NULL,
                    ObservationConfidence::High,
                    0,
                ),
                now,
            );
            assert_eq!(
                s.per_source.iter().count(),
                0,
                "{source:?} は record_replayed でもプールに入らない"
            );
            assert_eq!(s.derive_any(now), None);
        }
    }

    #[test]
    fn store_record_and_clear() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        rec(&mut s, obs(true, ObservationSource::ObserverPoll, now));
        assert!(s.per_source.observer_poll.is_some());
        s.clear_on_focus_change(1);
        assert!(s.per_source.observer_poll.is_none());
    }

    #[test]
    fn conv_open_inference_participates_in_iter_and_clear_all() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        rec(&mut s, obs(true, ObservationSource::ConvOpenInference, now));
        assert_eq!(
            s.per_source
                .iter()
                .filter(|o| o.source == ObservationSource::ConvOpenInference)
                .count(),
            1,
            "iter() が ConvOpenInference を含む"
        );
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            Some(true),
            "Medium confidence 単独で derive_any() に反映される (通常の観測ソースと同待遇)"
        );
        s.clear_on_focus_change(1);
        assert!(
            s.per_source.conv_open_inference.is_none(),
            "clear_all() で ConvOpenInference もクリアされる"
        );
    }

    #[test]
    fn drift_tracking() {
        let mut s = ObservationStore::default();
        let t0 = Instant::now();
        // desired=true, observed=false → drift 開始
        s.update_drift(true, false, t0);
        assert!(s.drift.is_some());
        assert_eq!(s.drift.unwrap().started_at, t0);

        // 同じ desired/observed で再 update → started_at 維持
        let t1 = t0 + Duration::from_millis(50);
        s.update_drift(true, false, t1);
        assert_eq!(s.drift.unwrap().started_at, t0, "started_at 維持");

        // desired と observed が一致 → drift clear
        s.update_drift(true, true, t1);
        assert!(s.drift.is_none());
    }

    #[test]
    fn most_recent_trusted_by_confidence() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let mut low = obs(true, ObservationSource::FocusProbe, now);
        low.confidence = ObservationConfidence::Low;
        let mut high = obs(false, ObservationSource::ImmGetOpenStatus, now);
        high.confidence = ObservationConfidence::High;
        rec(&mut s, low);
        rec(&mut s, high);
        assert_eq!(
            s.most_recent_trusted(now).map(|o| o.open),
            Some(false),
            "High confidence が勝つ"
        );
    }

    #[test]
    fn most_recent_trusted_after_excludes_before_since() {
        let mut s = ObservationStore::default();
        let t0 = Instant::now();
        let since = t0 + Duration::from_millis(100);
        // since より前に record された High confidence の観測は、本来なら勝つはずだが除外される。
        let mut high = obs(true, ObservationSource::ImmGetOpenStatus, t0);
        high.confidence = ObservationConfidence::High;
        rec(&mut s, high);
        assert_eq!(
            s.most_recent_trusted_after(since, since),
            None,
            "since より前の観測は最高 confidence でも除外される"
        );
        // 対照: 通常の most_recent_trusted なら拾える。
        assert_eq!(
            s.most_recent_trusted(since).map(|o| o.open),
            Some(true),
            "since 条件のない most_recent_trusted なら拾える"
        );
    }

    #[test]
    fn most_recent_trusted_after_includes_at_or_after_since() {
        let mut s = ObservationStore::default();
        let since = Instant::now();
        // since 以降の観測は confidence 優先で通常どおり選ばれる。
        let mut low = obs(true, ObservationSource::FocusProbe, since);
        low.confidence = ObservationConfidence::Low;
        let mut high = obs(false, ObservationSource::ImmGetOpenStatus, since);
        high.confidence = ObservationConfidence::High;
        rec(&mut s, low);
        rec(&mut s, high);
        assert_eq!(
            s.most_recent_trusted_after(since, since).map(|o| o.open),
            Some(false),
            "since ちょうどの観測は含まれ、High confidence が勝つ (most_recent_trusted と同じ tie-break)"
        );
    }

    #[test]
    fn most_recent_trusted_after_does_not_exclude_by_source() {
        let mut s = ObservationStore::default();
        let since = Instant::now();
        // ConvOpenInference (間接推論ソース) も since 条件だけで判定され、ソース種別では除外されない。
        rec(
            &mut s,
            obs(true, ObservationSource::ConvOpenInference, since),
        );
        assert_eq!(
            s.most_recent_trusted_after(since, since).map(|o| o.open),
            Some(true),
            "ConvOpenInference もソース種別では除外されず全ソース対象"
        );
    }

    // ── ADR-090 §2.B: `read_back` の全数同値テスト ────────────────────────
    //
    // `ir_apply_drift_correction` の 2 箇所を `most_recent_trusted_after` の
    // 直接呼び出しから `read_back` へ移した。**移行前後の判定が bit-identical**
    // であることを、旧述語をこのテスト内で再現して全数比較する形で固定する
    // （ADR-090 §6 ステップ 2 の 8「移行前後の同値を Linux 全数テストで固定して
    // から書き換えること」/ B-R1）。
    //
    // 軸: `since` の前後（before / at / after）× `desired`（true/false）×
    // confidence 3 値（Low/Medium/High）× 観測の open 値（true/false）。

    /// 旧実装の `Read` 収束述語（`ime_refresh.rs:700` にあったもの）。
    fn legacy_confirmed(s: &ObservationStore, now: Instant, since: Instant, desired: bool) -> bool {
        s.most_recent_trusted_after(now, since)
            .is_some_and(|o| o.open == desired)
    }

    /// 旧実装の `Blind` give-up 復旧述語（`ime_refresh.rs:678` にあったもの）。
    fn legacy_fresh(s: &ObservationStore, now: Instant, since: Instant) -> bool {
        s.most_recent_trusted_after(now, since).is_some()
    }

    #[test]
    fn read_back_converged_matches_the_legacy_predicate_exhaustively() {
        // `Instant` の減算を避けるため基準点を先に取り、`since` をそこから進める
        // （clippy::unchecked_time_subtraction）。
        let base = Instant::now();
        let since = base + Duration::from_millis(10);
        let offsets = [
            ("before", base),
            ("at", since),
            ("after", since + Duration::from_millis(10)),
        ];
        let confidences = [
            ObservationConfidence::Low,
            ObservationConfidence::Medium,
            ObservationConfidence::High,
        ];
        // 観測ゼロの場合も含める（`None` → Pending）。
        {
            let s = ObservationStore::default();
            for desired in [false, true] {
                let receipt = s.read_back(since, since, ReadBackQuery::Converged { desired }, 7);
                assert!(!receipt.converged(), "観測ゼロなら収束していない");
                assert_eq!(receipt.resolution(), Resolution::Pending);
                assert_eq!(receipt.attempts(), 7, "attempts はそのまま receipt に載る");
                assert_eq!(
                    receipt.converged(),
                    legacy_confirmed(&s, since, since, desired)
                );
            }
        }
        for (label, at) in offsets {
            for confidence in confidences {
                for open in [false, true] {
                    for desired in [false, true] {
                        let mut s = ObservationStore::default();
                        let mut o = obs(open, ObservationSource::ObserverPoll, at);
                        o.confidence = confidence;
                        rec(&mut s, o);
                        let now = since + Duration::from_millis(20);
                        let receipt =
                            s.read_back(now, since, ReadBackQuery::Converged { desired }, 3);
                        let legacy = legacy_confirmed(&s, now, since, desired);
                        assert_eq!(
                            receipt.converged(),
                            legacy,
                            "read_back(Converged) が旧述語と食い違った \
                             (at={label} confidence={confidence:?} open={open} desired={desired})"
                        );
                        assert_eq!(
                            receipt.resolution(),
                            if legacy {
                                Resolution::Confirmed
                            } else {
                                Resolution::Pending
                            },
                            "収束していないときの帰結は Pending（GaveUp ではない）"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn read_back_any_fresh_evidence_matches_the_legacy_predicate_exhaustively() {
        let base = Instant::now();
        let since = base + Duration::from_millis(10);
        let offsets = [
            ("before", base),
            ("at", since),
            ("after", since + Duration::from_millis(10)),
        ];
        let confidences = [
            ObservationConfidence::Low,
            ObservationConfidence::Medium,
            ObservationConfidence::High,
        ];
        {
            let s = ObservationStore::default();
            let receipt = s.read_back(since, since, ReadBackQuery::AnyFreshEvidence, 5);
            assert_eq!(receipt.resolution(), Resolution::GaveUp);
            assert!(!legacy_fresh(&s, since, since));
        }
        for (label, at) in offsets {
            for confidence in confidences {
                for open in [false, true] {
                    let mut s = ObservationStore::default();
                    let mut o = obs(open, ObservationSource::ConvOpenInference, at);
                    o.confidence = confidence;
                    rec(&mut s, o);
                    let now = since + Duration::from_millis(20);
                    let receipt = s.read_back(now, since, ReadBackQuery::AnyFreshEvidence, 5);
                    let legacy = legacy_fresh(&s, now, since);
                    assert_eq!(
                        receipt.resolution() == Resolution::ExternalChange,
                        legacy,
                        "read_back(AnyFreshEvidence) が旧述語と食い違った \
                         (at={label} confidence={confidence:?} open={open})"
                    );
                    // **値では判定しない**ことの固定（ADR-080 / BUG-43）。
                    assert_ne!(
                        receipt.resolution(),
                        Resolution::Confirmed,
                        "AnyFreshEvidence は値を見ないので Confirmed にはならない"
                    );
                }
            }
        }
    }

    /// `AnyFreshEvidence` は `open` の値に依存しない——同じ `since` で
    /// `open=true` と `open=false` の観測を入れ替えても帰結が変わらないこと。
    ///
    /// 「target と異なる値の観測が来たら復旧」という素朴な実装に戻すと
    /// give-up が毎 tick 無効化される（乖離の定義そのものだから）。その
    /// 誤りに戻れないようにするための固定（`ime_refresh.rs` の該当コメント）。
    #[test]
    fn read_back_any_fresh_evidence_ignores_the_observed_value() {
        let since = Instant::now();
        let now = since + Duration::from_millis(20);
        let mut a = ObservationStore::default();
        rec(&mut a, obs(true, ObservationSource::Gji, now));
        let mut b = ObservationStore::default();
        rec(&mut b, obs(false, ObservationSource::Gji, now));
        assert_eq!(
            a.read_back(now, since, ReadBackQuery::AnyFreshEvidence, 0),
            b.read_back(now, since, ReadBackQuery::AnyFreshEvidence, 0),
            "AnyFreshEvidence は鮮度だけを見る（値は不問）"
        );
    }

    /// expire 済みの観測は `read_back` にも現れない（`most_recent_trusted_after`
    /// の `is_expired` フィルタをそのまま引き継ぐ）。
    #[test]
    fn read_back_excludes_expired_observations() {
        let since = Instant::now();
        let now = since + Duration::from_millis(20);
        let mut s = ObservationStore::default();
        let mut o = obs(true, ObservationSource::Gji, now);
        o.expires_at = Some(now);
        rec(&mut s, o);
        assert_eq!(
            s.read_back(now, since, ReadBackQuery::AnyFreshEvidence, 0)
                .resolution(),
            Resolution::GaveUp,
            "expire 済みは新しい証拠として数えない"
        );
        assert_eq!(
            s.read_back(now, since, ReadBackQuery::Converged { desired: true }, 0)
                .resolution(),
            Resolution::Pending
        );
    }

    #[test]
    fn consensus_requires_two_sources() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let window = Duration::from_millis(500);

        rec(&mut s, obs(true, ObservationSource::ObserverPoll, now));
        assert_eq!(s.consensus(window, now), None, "1 ソースでは合意なし");

        rec(&mut s, obs(true, ObservationSource::Gji, now));
        assert_eq!(s.consensus(window, now), Some(true), "2 ソース合意");

        rec(&mut s, obs(false, ObservationSource::Tsf, now));
        assert_eq!(s.consensus(window, now), None, "意見が分かれたら合意なし");
    }

    #[test]
    fn expired_observation_excluded() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let mut o = obs(true, ObservationSource::Gji, now);
        o.expires_at = Some(now);
        rec(&mut s, o);
        assert_eq!(s.most_recent_trusted(now), None, "expire 済みは除外");
    }

    // ── derive_any / derive_actuating ──────────────────────────────────────────────────────────

    #[test]
    fn derive_any_empty_returns_none() {
        let s = ObservationStore::default();
        assert_eq!(s.derive_any(Instant::now()), None, "観測なし → None");
    }

    #[test]
    fn derive_any_high_confidence_wins_immediately() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        // High confidence (ImmCrossProbe) が true → Some(true) を即採用
        let mut high = obs(true, ObservationSource::ImmCrossProbe, now);
        high.confidence = ObservationConfidence::High;
        rec(&mut s, high);
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            Some(true),
            "High confidence 即採用"
        );
    }

    #[test]
    fn derive_any_high_wins_over_low() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        // Low confidence の false (FocusProbe) + High confidence の true (ImmCrossProbe)
        // → High が勝ち true を返す（Qt/GJI バグ修正の核心ケース）
        let mut low = obs(false, ObservationSource::FocusProbe, now);
        low.confidence = ObservationConfidence::Low;
        let mut high = obs(true, ObservationSource::ImmCrossProbe, now);
        high.confidence = ObservationConfidence::High;
        rec(&mut s, low);
        rec(&mut s, high);
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            Some(true),
            "High confidence true が Low confidence false を上書き"
        );
    }

    #[test]
    fn derive_any_low_confidence_alone_returns_none() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        // Low confidence だけでは Medium+ ステップでも High ステップでもヒットしない
        let mut low = obs(false, ObservationSource::FocusProbe, now);
        low.confidence = ObservationConfidence::Low;
        rec(&mut s, low);
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            None,
            "Low confidence のみ → fallback するよう None を返す"
        );
    }

    #[test]
    fn derive_any_medium_single_source() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        // Medium 1ソースでも無競合なら採用
        rec(&mut s, obs(true, ObservationSource::ObserverPoll, now));
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            Some(true),
            "Medium 単独 → Some"
        );
    }

    #[test]
    fn derive_any_medium_conflict_returns_none() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        rec(&mut s, obs(true, ObservationSource::ObserverPoll, now));
        rec(&mut s, obs(false, ObservationSource::Gji, now));
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            None,
            "Medium 競合 → None（caller が desired にフォールバック）"
        );
    }

    #[test]
    fn derive_any_stale_observation_ignored() {
        let mut s = ObservationStore::default();
        let past = Instant::now() - Duration::from_secs(10);
        // 10 秒前の Medium obs は FRESH(3s) を超えているため無視される
        let mut old = obs(false, ObservationSource::ObserverPoll, past);
        old.confidence = ObservationConfidence::Medium;
        rec(&mut s, old);
        assert_eq!(
            s.derive_any(Instant::now()),
            None,
            "古い観測（FRESH 超過）は無視"
        );
    }

    /// `derive_open_medium_single_source`（true 側）の対称テスト。mutants で
    /// `match (true_count, false_count) { (0, f) if f >= 1 => Some(false), .. }` の
    /// ガード `f >= 1` が `false` に壊れても既存テストは検知できなかった
    /// （true 側しか検証していなかったため）。
    #[test]
    fn derive_any_medium_single_source_false() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        rec(&mut s, obs(false, ObservationSource::ObserverPoll, now));
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            Some(false),
            "Medium 単独 false → Some(false)"
        );
    }

    /// epoch フィルタ（`ImmCrossProbe`/`FocusProbe` のみ）が実際に効いていることを固定する。
    /// `derive_any()` の `is_epoch_ok` match アームが削除されても、このテストが無ければ
    /// 検知できなかった（stale な High 観測がフォーカス変更後も採用され続ける再発）。
    #[test]
    fn derive_any_high_confidence_stale_epoch_excluded() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let mut stale_high = obs(true, ObservationSource::ImmCrossProbe, now);
        stale_high.confidence = ObservationConfidence::High;
        stale_high.focus_epoch = 0;
        rec(&mut s, stale_high);
        s.current_focus_epoch = 1; // フォーカスが変わって epoch が進んだ
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            None,
            "旧 epoch の ImmCrossProbe(High) は現在の epoch と一致しないため除外される"
        );
    }

    #[test]
    fn consensus_two_sources_agree_false() {
        // `consensus_requires_two_sources` の true 側と対称。false 側の合意判定
        // (`votes_false >= 2 && votes_true == 0`) が未検証だったため、`&&`↔`||`・
        // `>=`↔`<`・`==`↔`!=` の反転が mutants で MISSED になっていた。
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let window = Duration::from_millis(500);

        rec(&mut s, obs(false, ObservationSource::ObserverPoll, now));
        assert_eq!(s.consensus(window, now), None, "1 ソースでは合意なし");

        rec(&mut s, obs(false, ObservationSource::Gji, now));
        assert_eq!(s.consensus(window, now), Some(false), "2 ソース false 合意");
    }

    #[test]
    fn consensus_ignores_observation_older_than_window() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let window = Duration::from_millis(500);
        let old = obs(
            true,
            ObservationSource::ObserverPoll,
            now - Duration::from_secs(1),
        );
        rec(&mut s, old);
        rec(&mut s, obs(true, ObservationSource::Gji, now));
        assert_eq!(
            s.consensus(window, now),
            None,
            "window 外の観測は合意にカウントしない（1 票のみ有効）"
        );
    }

    #[test]
    fn consensus_ignores_expired_observation_within_window() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let window = Duration::from_millis(500);
        let mut expired = obs(true, ObservationSource::ObserverPoll, now);
        expired.expires_at = Some(now);
        rec(&mut s, expired);
        rec(&mut s, obs(true, ObservationSource::Gji, now));
        assert_eq!(
            s.consensus(window, now),
            None,
            "window 内でも expired 観測は合意にカウントしない（1 票のみ有効）"
        );
    }

    #[test]
    fn drift_duration_after_update_drift_returns_elapsed() {
        let mut s = ObservationStore::default();
        let t0 = Instant::now();
        s.update_drift(true, false, t0);
        let t1 = t0 + Duration::from_millis(50);
        assert_eq!(s.drift_duration(t1), Some(Duration::from_millis(50)));
        s.update_drift(true, true, t1); // 収束 → drift clear
        assert_eq!(s.drift_duration(t1), None);
    }

    // ── プール別 derive / DeriveOutcome（ADR-087 §2.3 P15 Step 3、round3 S5） ──

    #[test]
    fn derive_any_includes_belief_only_sources() {
        // ConvOpenInference（BeliefOnly）単独でも derive_any は採用する。
        let mut s = ObservationStore::default();
        let now = Instant::now();
        rec(&mut s, obs(true, ObservationSource::ConvOpenInference, now));
        assert_eq!(s.derive_any(now).map(|o| o.value()), Some(true));
    }

    #[test]
    fn derive_actuating_excludes_belief_only_sources() {
        // ConvOpenInference（BeliefOnly）だけがある場合、Actuating プールの
        // 導出は None になる（ADR-087 Step 3 の入力から外れる）。
        let mut s = ObservationStore::default();
        let now = Instant::now();
        rec(&mut s, obs(true, ObservationSource::ConvOpenInference, now));
        assert_eq!(
            s.derive_actuating(now),
            None,
            "ConvOpenInference は BeliefOnly のため derive_actuating では None"
        );
    }

    #[test]
    fn derive_high_single_reports_source() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let mut o = obs(true, ObservationSource::ImmGetOpenStatus, now);
        o.confidence = ObservationConfidence::High;
        rec(&mut s, o);
        assert_eq!(
            s.derive_any(now),
            Some(DeriveOutcome::HighSingle {
                source: ObservationSource::ImmGetOpenStatus,
                open: true,
            })
        );
    }

    // ── pinned test（ADR-089 §6 Phase A の「先にやること」、§2.1） ──────────────
    //
    // プール分離（`derive_actuating` / `derive_any`）が導出結果を変えていない
    // ことを固定する。オラクルは**リファクタ前**の `derive_open_filtered` の
    // 逐語コピーであり、production 側が変わってもここは変えない
    // （変えるとリファクタ前後の比較という目的が消える）。
    // 比較は `DeriveOutcome` の等値で行う——`.value()` の bool だけを比べると
    // `WarrantBasis` の構築に使う `source` / `first` / `second` の変化を
    // 検出できない（ADR-087）。

    /// リファクタ前の `ObservationStore::derive_open_filtered` の逐語コピー。
    fn legacy_derive_open_filtered(
        store: &ObservationStore,
        now: Instant,
        accept: impl Fn(ObservationSource) -> bool,
    ) -> Option<DeriveOutcome> {
        const FRESH: Duration = Duration::from_secs(3);
        let current_epoch = store.current_focus_epoch;

        let is_fresh = |o: &ImeObservation| !o.is_expired(now) && o.age(now) <= FRESH;

        let is_epoch_ok = |o: &ImeObservation| match o.source {
            ObservationSource::ImmCrossProbe | ObservationSource::FocusProbe => {
                o.focus_epoch == current_epoch
            }
            _ => true,
        };

        let high = store
            .per_source
            .iter()
            .filter(|o| {
                accept(o.source)
                    && is_fresh(o)
                    && is_epoch_ok(o)
                    && o.confidence == ObservationConfidence::High
            })
            .max_by_key(|o| o.at);
        if let Some(obs) = high {
            return Some(DeriveOutcome::HighSingle {
                source: obs.source,
                open: obs.open,
            });
        }

        let mut true_first: Option<ObservationSource> = None;
        let mut true_second: Option<ObservationSource> = None;
        let mut false_first: Option<ObservationSource> = None;
        let mut false_second: Option<ObservationSource> = None;
        for obs in store.per_source.iter() {
            if !accept(obs.source)
                || !is_fresh(obs)
                || !is_epoch_ok(obs)
                || obs.confidence < ObservationConfidence::Medium
            {
                continue;
            }
            if obs.open {
                if true_first.is_none() {
                    true_first = Some(obs.source);
                } else if true_second.is_none() {
                    true_second = Some(obs.source);
                }
            } else if false_first.is_none() {
                false_first = Some(obs.source);
            } else if false_second.is_none() {
                false_second = Some(obs.source);
            }
        }
        match (true_first, false_first) {
            (Some(first), None) => Some(DeriveOutcome::MediumConsensus {
                first,
                second: true_second,
                open: true,
            }),
            (None, Some(first)) => Some(DeriveOutcome::MediumConsensus {
                first,
                second: false_second,
                open: false,
            }),
            _ => None,
        }
    }

    /// `PerSourceObservations` に実フィールドを持つ 9 ソース（ADR-089 §1.3(h)）。
    const RECORDABLE_SOURCES: [ObservationSource; 9] = [
        ObservationSource::ImmGetOpenStatus,
        ObservationSource::ImmCrossProbe,
        ObservationSource::ObserverPoll,
        ObservationSource::Gji,
        ObservationSource::Tsf,
        ObservationSource::ConvOpenInference,
        ObservationSource::HeuristicDefault,
        ObservationSource::HwndCache,
        ObservationSource::FocusProbe,
    ];

    const CONFIDENCES: [ObservationConfidence; 3] = [
        ObservationConfidence::Low,
        ObservationConfidence::Medium,
        ObservationConfidence::High,
    ];

    /// 2 ソースの全組み合わせ（値 × confidence × epoch × 鮮度）でストアを作る。
    fn pinned_matrix() -> Vec<(String, ObservationStore, Instant)> {
        let now = Instant::now();
        let mut out = Vec::new();
        for a in RECORDABLE_SOURCES {
            for b in RECORDABLE_SOURCES {
                for open_a in [true, false] {
                    for open_b in [true, false] {
                        for conf_a in CONFIDENCES {
                            for conf_b in CONFIDENCES {
                                for store_epoch in [0_u64, 1] {
                                    for stale_b in [false, true] {
                                        let mut s = ObservationStore {
                                            current_focus_epoch: store_epoch,
                                            ..Default::default()
                                        };
                                        let mut oa = obs(open_a, a, now);
                                        oa.confidence = conf_a;
                                        let at_b = if stale_b {
                                            now.checked_sub(Duration::from_secs(10)).unwrap()
                                        } else {
                                            now
                                        };
                                        let mut ob = obs(open_b, b, at_b);
                                        ob.confidence = conf_b;
                                        rec(&mut s, oa);
                                        rec(&mut s, ob);
                                        out.push((
                                            format!(
                                                "{a:?}({open_a},{conf_a:?}) + {b:?}({open_b},{conf_b:?},stale={stale_b}) \
                                                 store_epoch={store_epoch}"
                                            ),
                                            s,
                                            now,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn pinned_derive_any_equals_legacy_accept_all() {
        for (label, store, now) in pinned_matrix() {
            assert_eq!(
                store.derive_any(now),
                legacy_derive_open_filtered(&store, now, |_| true),
                "derive_any が旧 derive_open_filtered(|_| true) と乖離: {label}"
            );
        }
    }

    #[test]
    fn pinned_derive_actuating_equals_legacy_actuating_filter() {
        use super::super::ime_event::ObservationAuthority;
        let actuating = |s: ObservationSource| s.authority() == ObservationAuthority::Actuating;
        for (label, store, now) in pinned_matrix() {
            assert_eq!(
                store.derive_actuating(now),
                legacy_derive_open_filtered(&store, now, actuating),
                "derive_actuating が旧 derive_open_filtered(Actuating) と乖離: {label}"
            );
        }
    }

    /// ADR-089 §9-6: epoch フィルタ（`ImmCrossProbe` = Actuating /
    /// `FocusProbe` = BeliefOnly）がプール分離で意味を変えていないこと。
    #[test]
    fn pinned_epoch_filter_applies_across_both_pools() {
        let now = Instant::now();
        for source in [
            ObservationSource::ImmCrossProbe,
            ObservationSource::FocusProbe,
        ] {
            let mut s = ObservationStore::default();
            let mut o = obs(true, source, now);
            o.confidence = ObservationConfidence::High;
            o.focus_epoch = 0;
            rec(&mut s, o);
            s.current_focus_epoch = 1;
            assert_eq!(
                s.derive_any(now),
                None,
                "{source:?}: stale epoch は derive_any でも除外される"
            );
            assert_eq!(
                s.derive_actuating(now),
                None,
                "{source:?}: stale epoch は derive_actuating でも除外される"
            );
        }
    }

    #[test]
    fn derive_medium_consensus_reports_all_sources() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        rec(&mut s, obs(true, ObservationSource::ObserverPoll, now));
        rec(&mut s, obs(true, ObservationSource::Gji, now));
        let outcome = s.derive_any(now).unwrap();
        assert_eq!(
            outcome,
            DeriveOutcome::MediumConsensus {
                first: ObservationSource::ObserverPoll,
                second: Some(ObservationSource::Gji),
                open: true,
            },
            "2 ソースの合意なので corroboration 相当（PerSourceObservations::iter() \
             の宣言順で ObserverPoll が先）"
        );
    }
}
