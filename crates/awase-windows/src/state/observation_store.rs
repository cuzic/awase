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
use super::probe_admission::{FocusEpoch, FocusFence};
use crate::focus::class_names::AppImeProfile;

// ── FocusProbe open status（ADR-106 決定2） ─────────────────────────────────

/// `FocusProbe`（`read_ime_state_fast`）の open status 判定結果。
///
/// 旧 `sanitize_focus_probe_open_status` は `Option<bool>` を返しており、
/// 「プロファイルが IMM32 open status を読めない」場合と「プロファイルは読める
/// はずだが今回は取得できなかった」場合の両方が `None` に潰れていた。この enum は
/// 前者を `NotObservable` として型で区別する（INV-C の具体化: 観測できない状況を
/// bool へ潰さず運ぶ）。
#[derive(Debug, Clone, Copy)]
pub enum FocusProbeOpenStatus {
    /// IMM32 API から実際に読み取れた値。
    Read(ObservedOpenValue),
    /// このプロファイルでは IMM32 の open status を信頼できない
    /// （`AppImeProfile::can_read_imm32_open_status() == false`、TsfNative /
    /// Imm32Unavailable）。
    NotObservable(AppImeProfile),
}

impl FocusProbeOpenStatus {
    /// `probe.ime_on`（raw な IMM32 読み取り結果）とプロファイルから判定する。
    #[must_use]
    pub const fn classify(probe_ime_on: Option<bool>, profile: AppImeProfile) -> Self {
        if !profile.can_read_imm32_open_status() {
            return Self::NotObservable(profile);
        }
        match probe_ime_on {
            Some(on) => Self::Read(ObservedOpenValue(on)),
            None => Self::NotObservable(profile),
        }
    }
}

/// [`FocusProbeOpenStatus::Read`] からしか得られない値。
///
/// フィールドが private のため、crate 内のどこからも `ObservedOpenValue(x)` を
/// 直接構築できない——`FocusProbeOpenStatus::classify` の `Read` 分岐だけが
/// 構築できる。`apply_effective_ime`/`write_focus_probe` の引数をこの型にする
/// ことで、belief 由来の `shadow_on: bool`（`effective_open()`）を観測として
/// 書き込むコード（旧 `apply_effective_ime(shadow_on, ...)`）はコンパイルエラー
/// になる（BUG-92: BUG-33 の shadow フォールバック laundering を型で閉じた）。
///
/// ```
/// use awase_windows::state::observation_store::{FocusProbeOpenStatus, ObservedOpenValue};
/// use awase_windows::focus::class_names::AppImeProfile;
///
/// // Standard プロファイル + 実際の読み取り値がある場合のみ `Read` が返る。
/// let status = FocusProbeOpenStatus::classify(Some(true), AppImeProfile::Standard);
/// assert!(matches!(status, FocusProbeOpenStatus::Read(_)));
/// let FocusProbeOpenStatus::Read(value) = status else {
///     unreachable!()
/// };
/// assert!(value.get());
/// ```
///
/// `NotObservable` からは作れない（belief 由来の値を観測として偽装できない）:
///
/// ```compile_fail
/// use awase_windows::state::observation_store::{FocusProbeOpenStatus, ObservedOpenValue};
/// use awase_windows::focus::class_names::AppImeProfile;
///
/// let status = FocusProbeOpenStatus::classify(None, AppImeProfile::TsfNative);
/// let FocusProbeOpenStatus::NotObservable(_profile) = status else {
///     unreachable!()
/// };
/// // NotObservable アームは `ObservedOpenValue` を一切持たない。
/// // shadow_on: bool のような belief 由来の値を渡そうとしてもフィールドが
/// // private なため構築できない。
/// let shadow_on = true;
/// let _value = ObservedOpenValue(shadow_on); // error[E0603]: field `0` is private
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedOpenValue(bool);

impl ObservedOpenValue {
    #[must_use]
    pub const fn get(self) -> bool {
        self.0
    }

    /// `is_japanese_ime` と合成した「実際に effective か」を返す。
    /// 合成結果も `Read` 由来の値からのみ導出されるため、型としての出自は保たれる。
    #[must_use]
    pub const fn effective(self, is_japanese_ime: bool) -> Self {
        Self(self.0 && is_japanese_ime)
    }
}

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
///
/// `focus_epoch`/`hwnd` は `FocusFence` へ統合していない（ADR-106 決定3の型統合
/// スコープ外、PR 109 コードレビュー指摘4）——`record_any` が単一の `observed` から
/// 両方を同時に埋めるため片側だけ古くなるバグクラスが構造的に発生せず、統合すると
/// journal 直列化形式(ADR-082)に触れ replay 前後比較が必要になり過剰なため。
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
    /// 現在のフォーカス同一性（epoch + hwnd、ADR-106 決定3）。
    ///
    /// `derive_any()` が `ImmCrossProbe` / `FocusProbe` 観測をこの値と照合して
    /// フィルタする。これにより、stale な高信頼観測が意思決定に使われることを
    /// 防ぐ。epoch はプロセス変更でのみ進むため、同一プロセス内でウィンドウだけが
    /// 変わるケースは epoch 単独では検知できず、hwnd も併せて照合する必要がある。
    ///
    /// **private**: 書き込み口は `clear_on_focus_change()`（プロセス変更時、観測
    /// プールごとクリアし両軸を丸ごと差し替え）、`update_focus_window()`（同一
    /// プロセス内でのウィンドウ変化、hwnd のみ更新）、`establish_initial_fence()`
    /// （起動時の初回フォーカススコープ確立、プールを触らず両軸を差し替え。
    /// BUG-102）の3つに限定する。かつて
    /// epoch/hwnd を別々の `pub` フィールドとして持ち回っていたときは、
    /// `update_focus_hwnd()` を呼び忘れると `admit()`（`platform.focus.current.hwnd`
    /// を毎 tick 参照）は新しい hwnd を正しく受理するのに `derive_any()` の
    /// `is_identity_ok` は古い hwnd のまま比較し続け、次にプロセスが変わるまで
    /// 観測を恒久的に拒否し続けるという退行が起きた（code review 2026-08-26 で
    /// 発見）。両軸を1つの `FocusFence` に統合し書き込み口を絞ることで、この
    /// クラスの片側だけ更新し忘れる退行を構造的に防ぐ（PR 109 コードレビュー
    /// 指摘4）。
    current_fence: FocusFence,
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
            tracing::debug!("[observation] {source:?} は open 観測プールに入らないため破棄");
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

    /// 現在のフォーカス同一性（epoch + hwnd）を返す（ADR-106 決定3）。
    #[must_use]
    pub const fn current_fence(&self) -> FocusFence {
        self.current_fence
    }

    /// 全ソースを clear する (フォーカス変更時用)。drift も clear。
    ///
    /// `new_fence` には `FocusStore::focus_epoch` のインクリメント後の値と、
    /// 新しいフォーカス先の hwnd の両方を渡す（ADR-106 決定3）。これ以降
    /// `derive_any()` は古い epoch/hwnd の ImmCrossProbe / FocusProbe を無視する。
    pub fn clear_on_focus_change(&mut self, new_fence: FocusFence) {
        self.per_source.clear_all();
        self.drift = None;
        self.current_fence = new_fence;
    }

    /// 起動時（bootstrap）に確立した最初のフォーカススコープへ fence を合わせる
    /// 3つ目の書き込み口（BUG-102）。`ImeEvent::InitialFocusFenceEstablished` の
    /// reducer からのみ呼ぶ。
    ///
    /// `clear_on_focus_change()` との違い: **観測プールと drift をクリアしない**。
    /// bootstrap はフォーカスの「変更」ではなく、既にフォーカスされているウィンドウに
    /// 名前（epoch + hwnd）を付ける操作であり、捨てるべき「旧窓の観測」が存在しない
    /// （`app/bootstrap.rs::run_all` はこの呼び出しより前にメッセージループを
    /// ポンプしないため、この時点でプールは空である）。
    ///
    /// `update_focus_window()` との違い: **epoch も含めて差し替える**。bootstrap では
    /// `enter_focus_scope` が `FocusStore::focus_epoch` を 0→1 に進めているため、
    /// hwnd だけ合わせても epoch が食い違ったままになり `is_identity_ok` が
    /// `ImmCrossProbe`（High）を導出から外し続ける。
    ///
    /// # 前提と、その担保のしかた
    ///
    /// - **「initial」であること**（`current_fence` がまだ既定値）は下の
    ///   `debug_assert!` が実行時に固定する。同じ値での再確立（no-op）だけは
    ///   許容し、**別の値への差し替えは拒否する** ——それは fence の張り替えで
    ///   あって初回確立ではなく、`clear_on_focus_change()`（観測プールごと
    ///   差し替える）の仕事だから。
    /// - **プールが空であること**は `debug_assert!` にしない。ここでプールが
    ///   空であることは呼び出し元（bootstrap）の性質であって本メソッドの契約では
    ///   なく、assert にすると「プールを消さない」という本メソッドの設計意図
    ///   （`establish_initial_fence_does_not_clear_the_pool_or_drift` が固定）を
    ///   到達不能な分岐にしてしまう。上の fence assert が「bootstrap 以外から
    ///   呼ばれた」場合を既に捕まえるため、重ねる価値も小さい。
    pub fn establish_initial_fence(&mut self, fence: FocusFence) {
        debug_assert!(
            self.current_fence == FocusFence::default() || self.current_fence == fence,
            "establish_initial_fence は fence 未確立（既定値）のうちに1度だけ呼ぶこと。\
             既に動いている fence の張り替えは clear_on_focus_change の役割\
             （現在: {:?} / 要求: {fence:?}、BUG-102）",
            self.current_fence
        );
        tracing::debug!(
            "[focus-fence] establish_initial_fence: {:?} -> {fence:?}",
            self.current_fence
        );
        self.current_fence = fence;
    }

    /// 同一プロセス内でフォーカス hwnd だけが変わった場合の更新口（ADR-106 決定3）。
    ///
    /// `clear_on_focus_change()` と異なり、epoch・観測プール・drift には触れない
    /// ——プロセスは変わっていないため、それらのセマンティクスを保つ。この値だけを
    /// `platform.focus.current.hwnd`（admission 側が毎 tick 参照する生の hwnd）に
    /// 追従させることで、`derive_any()` の `is_identity_ok` が stale な hwnd と
    /// 比較し続けて以後の観測を恒久的に拒否する退行を防ぐ。
    pub fn update_focus_window(&mut self, new_hwnd: HwndId) {
        tracing::debug!(
            "[focus-hwnd-track] update_focus_window: current_fence.hwnd {:?} -> {new_hwnd:?}",
            self.current_fence.hwnd
        );
        self.current_fence.hwnd = new_hwnd;
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
    /// `tracing::debug!` にしか渡らず、実際の収束判定は
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
        let resolution = match query {
            // 旧: `most_recent_trusted_after(now, act_sent_at).is_some_and(|o| o.open == desired)`
            ReadBackQuery::Converged { desired } => {
                let latest = self.most_recent_trusted_after(now, since);
                if latest.is_some_and(|o| o.open == desired) {
                    Resolution::Confirmed
                } else {
                    Resolution::Pending
                }
            }
            // 旧: `most_recent_trusted_after(now, gave_up_at).is_some()`
            //
            // **BUG-114 追補（ADR-134 Finding 5、実機確認済み・2段階で拡張）**:
            // 読み戻し手段が構造的に無いと自ら宣言しているプロファイル
            // （TsfNative/Imm32Unavailable、`Blacklist` 戦略）では、
            // `desired` が実現したかを一切確認していない自己言及的な弱い
            // 代理指標が複数の経路から継続的に record され続ける。これを
            // `AnyFreshEvidence` が無区別に「外界が動いた証拠」として
            // 採用すると、`Blind` の `GiveUp` 後クールダウン（3秒）が実質
            // 意味を失い、`attempts=5` の `GiveUp` から次のバーストまで
            // 数秒おきに永久に繰り返す暴走になる（`docs/known-bugs.md`
            // BUG-114）。実機で確認済みの2ソース:
            // - `ObserverPoll`: `observe_gji_after_focus`（GJI I/O 活動監視）。
            // - `ConvOpenInference`: `kp_stage_idle_conv_check` が conv ビットの
            //   `NativeToggleShadowOff`/`KatakanaShadowOff` から書く open 推測
            //   （`state/platform_state.rs::report_conv_open_inference`）。
            //   これは shadow-toggle 自身が動かした conv 状態を読み返して
            //   いるだけの自己言及的な信号であり、実機で ~250〜380ms 間隔の
            //   高頻度で record され続けることを確認した（[[feedback_conv_mode_unreliable_dont_gate_actuation_on_it]]
            //   と同じ理由でこの用途にも使うべきでない）。
            // この query に限りこの2ソースを鮮度判定から除外する——
            // `Converged`（`Read` policy の収束確認、`OsPoll` 戦略の genuine
            // な `ObserverPoll` に依存）には一切影響しない。
            ReadBackQuery::AnyFreshEvidence => {
                const EXCLUDED_FROM_ANY_FRESH_EVIDENCE: [ObservationSource; 2] = [
                    ObservationSource::ObserverPoll,
                    ObservationSource::ConvOpenInference,
                ];
                let latest = self.most_recent_trusted_after_excluding(
                    now,
                    since,
                    &EXCLUDED_FROM_ANY_FRESH_EVIDENCE,
                );
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
        // `most_recent_trusted_after_excluding(now, since, &[])` に委譲する
        // （code-review指摘、2026-09-05）: 除外リストが空なら
        // `exclude.contains(&o.source)` は常に false のため、フィルタ条件は
        // 元の実装と完全に同値。フィルタ/tie-breakロジックが2箇所に
        // 重複していると、将来どちらか一方だけを変更してしまい
        // BUG-114型の再発（AnyFreshEvidence側だけ古いロジックのまま残る等）
        // を招くリスクがあったため一本化した。
        self.most_recent_trusted_after_excluding(now, since, &[])
    }

    /// [`most_recent_trusted_after`] と同じだが、指定した `ObservationSource`
    /// 群を鮮度判定の対象から除外する。BUG-114 追補（ADR-134 Finding 5）で
    /// `ReadBackQuery::AnyFreshEvidence` 専用に追加した——`ObserverPoll`/
    /// `ConvOpenInference` のように「読み戻し手段が構造的に無いプロファイル
    /// からも自己言及的に書かれうる」ソースを、無区別に「外界が動いた
    /// 証拠」として扱わないため。除外リストは実機で確認され次第拡張する
    /// 前提（`read_back` の呼び出し元コメント参照）——現状は既知の2ソース
    /// のみで、将来別のソースが同種の暴走を起こすと判明したら足すこと。
    #[must_use]
    fn most_recent_trusted_after_excluding(
        &self,
        now: Instant,
        since: Instant,
        exclude: &[ObservationSource],
    ) -> Option<&ImeObservation> {
        self.per_source
            .iter()
            .filter(|o| !o.is_expired(now) && o.at >= since && !exclude.contains(&o.source))
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
    /// 古いウィンドウの観測が混入するリスクがある。`current_fence().epoch` と照合し、
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
        let current_fence = self.current_fence;

        let is_fresh = |o: &ImeObservation| !o.is_expired(now) && o.age(now) <= FRESH;

        // フォーカス同一性照合が必要なソース（async/first-key トリガーのスナップショット
        // probe）。epoch はプロセス変更でのみ進むため、同一プロセス内でウィンドウだけが
        // 変わるケースは epoch 単独では検知できず、hwnd も併せて照合する（ADR-106 決定3）。
        //
        // `ImeObservation` は `focus_epoch`/`hwnd` を個別フィールドのまま持つ
        // （`AnyObservation` と共有する journal 直列化形式(ADR-082)に触れる範囲が
        // 広くなるため、ここでは統合しない——`record_any` が単一の `observed` から
        // 両方を同時に埋めるため、片側だけ古くなるバグクラスは構造的に発生しない）。
        // 比較の瞬間だけ `FocusFence` に組み立てて `current_fence` と照合する。
        let is_identity_ok = |o: &ImeObservation| match o.source {
            ObservationSource::ImmCrossProbe | ObservationSource::FocusProbe => {
                let obs_fence = FocusFence {
                    epoch: o.focus_epoch,
                    hwnd: o.hwnd,
                };
                let epoch_ok = obs_fence.epoch == current_fence.epoch;
                let hwnd_ok = obs_fence.hwnd == current_fence.hwnd;
                if epoch_ok && !hwnd_ok {
                    // ADR-106 決定3: epoch は一致しているのに hwnd だけ不一致で
                    // 除外されるケース（同一プロセス内でのウィンドウ切替）を、
                    // epoch 不一致による除外と区別して実機ログで確認できるようにする。
                    tracing::debug!(
                        "[identity-gate] hwnd不一致で除外: source={:?} obs_hwnd={:?} current_hwnd={:?} confidence={:?}",
                        o.source,
                        obs_fence.hwnd,
                        current_fence.hwnd,
                        o.confidence
                    );
                }
                obs_fence == current_fence
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
                    && is_identity_ok(o)
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
                || !is_identity_ok(obs)
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

    /// issue #136 / BUG-90 決定4: `AppImeProfile::InputRelay`（PowerToys Mouse
    /// Without Borders 等の入力中継ツール）は `can_read_imm32_open_status() ==
    /// false` なので、IMM32 open status を実際に読み取れた場合でも
    /// `NotObservable` になり `ObservedOpenValue` は構築されない。この窓由来の
    /// open 観測は belief に一切取り込まれない（条件(c)、タスク3で
    /// `can_read_imm32_open_status(InputRelay) = false` を固定済み。ここでは
    /// `FocusProbeOpenStatus::classify` がその述語を正しく consume することを
    /// 固定する）。
    #[test]
    fn input_relay_profile_makes_open_status_not_observable_even_with_a_real_reading() {
        let status = FocusProbeOpenStatus::classify(Some(true), AppImeProfile::InputRelay);
        assert!(matches!(
            status,
            FocusProbeOpenStatus::NotObservable(AppImeProfile::InputRelay)
        ));
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
        s.clear_on_focus_change(FocusFence {
            epoch: 1,
            hwnd: HwndId::NULL,
        });
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
        s.clear_on_focus_change(FocusFence {
            epoch: 1,
            hwnd: HwndId::NULL,
        });
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

    /// **BUG-114 追補（ADR-134 Finding 5）で `ConvOpenInference`/`ObserverPoll`
    /// は「旧述語」からの意図的な乖離になったため、ここでは除外対象に
    /// 含まれない `Gji` を使う**（除外対象2ソースの専用テストは
    /// `read_back_any_fresh_evidence_ignores_observer_poll_alone` /
    /// `read_back_any_fresh_evidence_ignores_conv_open_inference_alone` を参照）。
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
                    let mut o = obs(open, ObservationSource::Gji, at);
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

    /// BUG-114 追補（ADR-134 Finding 5）の回帰テスト。
    ///
    /// `ObserverPoll` ソースの観測（`Blacklist` 戦略の GJI I/O 活動監視、
    /// `observe_gji_after_focus` が書く）だけが `since` 以降に record された
    /// 場合、`AnyFreshEvidence` は「外界が動いた証拠」として採用せず
    /// `GiveUp`（再武装しない）のままであることを固定する。実機で
    /// `attempts=5` の `GiveUp` から数秒〜数十秒おきに際限なく再武装する
    /// 暴走が確認されており（`docs/known-bugs.md` BUG-114）、この観測source
    /// が「鮮度だけで無条件に外界の変化とみなされる」ことが原因だった。
    #[test]
    fn read_back_any_fresh_evidence_ignores_observer_poll_alone() {
        let base = Instant::now();
        let since = base;
        let now = since + Duration::from_millis(20);
        let mut s = ObservationStore::default();
        rec(&mut s, obs(true, ObservationSource::ObserverPoll, since));
        let receipt = s.read_back(now, since, ReadBackQuery::AnyFreshEvidence, 5);
        assert_eq!(
            receipt.resolution(),
            Resolution::GaveUp,
            "ObserverPoll 単独の新しい観測は AnyFreshEvidence の再武装条件にしてはならない \
             (BUG-114/ADR-134 Finding 5)"
        );
    }

    /// BUG-114 追補・第2弾（実機で ObserverPoll 除外だけでは不十分と判明）の
    /// 回帰テスト。`ConvOpenInference`（`kp_stage_idle_conv_check` が conv
    /// ビットの `NativeToggleShadowOff` 等から書く open 推測、shadow-toggle
    /// 自身が動かした状態を読み返すだけの自己言及的な信号）が単独で
    /// `since` 以降に record されても再武装しないことを固定する。実機で
    /// ~250〜380ms 間隔の高頻度で record され続け、`ObserverPoll` を除外
    /// しただけでは暴走が別の形（`gave up` は正しく5回で止まるが、3秒
    /// クールダウン明けにこのソースだけでほぼ即座に再武装する）で残った。
    #[test]
    fn read_back_any_fresh_evidence_ignores_conv_open_inference_alone() {
        let base = Instant::now();
        let since = base;
        let now = since + Duration::from_millis(20);
        let mut s = ObservationStore::default();
        rec(
            &mut s,
            obs(true, ObservationSource::ConvOpenInference, since),
        );
        let receipt = s.read_back(now, since, ReadBackQuery::AnyFreshEvidence, 5);
        assert_eq!(
            receipt.resolution(),
            Resolution::GaveUp,
            "ConvOpenInference 単独の新しい観測も AnyFreshEvidence の再武装条件に\
             してはならない (BUG-114/ADR-134 Finding 5 第2弾、実機確認済み)"
        );
    }

    /// 上記2件と対になる確認: 除外対象**以外**のソース（例: `Gji`）が
    /// `since` 以降に記録されていれば、従来どおり `ExternalChange` として
    /// 再武装する。除外対象を `ObserverPoll`/`ConvOpenInference` の2つに
    /// 限定していることの固定。
    #[test]
    fn read_back_any_fresh_evidence_still_reacts_to_non_excluded_sources() {
        let base = Instant::now();
        let since = base;
        let now = since + Duration::from_millis(20);
        let mut s = ObservationStore::default();
        rec(&mut s, obs(true, ObservationSource::Gji, since));
        let receipt = s.read_back(now, since, ReadBackQuery::AnyFreshEvidence, 5);
        assert_eq!(
            receipt.resolution(),
            Resolution::ExternalChange,
            "除外対象以外のソースは従来どおり再武装条件になること"
        );
    }

    /// 除外対象の2ソースと、それ以外の新しい観測が両方存在する場合は、
    /// 後者だけで `ExternalChange` になること（除外は指定ソースの
    /// レコードだけをフィルタし、他の観測の判定に影響しないこと）。
    #[test]
    fn read_back_any_fresh_evidence_reacts_when_excluded_and_other_source_both_fresh() {
        let base = Instant::now();
        let since = base;
        let now = since + Duration::from_millis(20);
        let mut s = ObservationStore::default();
        rec(&mut s, obs(true, ObservationSource::ObserverPoll, since));
        rec(
            &mut s,
            obs(true, ObservationSource::ConvOpenInference, since),
        );
        rec(&mut s, obs(true, ObservationSource::Gji, since));
        let receipt = s.read_back(now, since, ReadBackQuery::AnyFreshEvidence, 5);
        assert_eq!(receipt.resolution(), Resolution::ExternalChange);
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
        let past = Instant::now()
            .checked_sub(Duration::from_secs(10))
            .expect("test instant can be backdated");
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
        s.current_fence.epoch = 1; // フォーカスが変わって epoch が進んだ
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            None,
            "旧 epoch の ImmCrossProbe(High) は現在の epoch と一致しないため除外される"
        );
    }

    /// ADR-106 決定3: `focus_epoch` はプロセス変更でのみ進むため、同一プロセス内で
    /// ウィンドウ（hwnd）だけが変わったケースは epoch 単独では検知できない。
    /// hwnd 照合が実際に効いていることを固定する。
    #[test]
    fn derive_any_high_confidence_stale_hwnd_excluded_even_with_matching_epoch() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let mut stale_high = obs(true, ObservationSource::ImmCrossProbe, now);
        stale_high.confidence = ObservationConfidence::High;
        stale_high.focus_epoch = 0;
        stale_high.hwnd = HwndId(1);
        rec(&mut s, stale_high);
        s.current_fence.epoch = 0; // epoch は同じ（プロセスは変わっていない）
        s.current_fence.hwnd = HwndId(2); // だがウィンドウだけが変わった
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            None,
            "epoch が一致していても hwnd が異なる ImmCrossProbe(High) は除外される"
        );
    }

    /// 対照: epoch も hwnd も一致していれば通常どおり採用される
    /// （hwnd フィルタが誤って全数除外に倒れていないことの固定）。
    #[test]
    fn derive_any_high_confidence_matching_epoch_and_hwnd_accepted() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let mut high = obs(true, ObservationSource::FocusProbe, now);
        high.confidence = ObservationConfidence::High;
        high.focus_epoch = 3;
        high.hwnd = HwndId(42);
        rec(&mut s, high);
        s.current_fence.epoch = 3;
        s.current_fence.hwnd = HwndId(42);
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            Some(true),
            "epoch/hwnd が一致する FocusProbe(High) は採用される"
        );
    }

    /// `clear_on_focus_change` が hwnd も更新することを固定する
    /// （ADR-106 決定3: プロセス変更時の書き込み口）。
    #[test]
    fn clear_on_focus_change_updates_current_focus_hwnd() {
        let mut s = ObservationStore::default();
        assert_eq!(s.current_fence().hwnd, HwndId::NULL);
        s.clear_on_focus_change(FocusFence {
            epoch: 5,
            hwnd: HwndId(99),
        });
        assert_eq!(s.current_fence().epoch, 5);
        assert_eq!(s.current_fence().hwnd, HwndId(99));
    }

    /// `clear_on_focus_change` が epoch・hwnd の両軸を1つの値として原子的に
    /// 差し替えることを固定する（PR 109 コードレビュー指摘4: `FocusFence`
    /// 統合の目的そのもの）。
    #[test]
    fn clear_on_focus_change_replaces_both_axes_atomically() {
        let mut s = ObservationStore::default();
        s.clear_on_focus_change(FocusFence {
            epoch: 1,
            hwnd: HwndId(1),
        });
        let new_fence = FocusFence {
            epoch: 2,
            hwnd: HwndId(2),
        };
        s.clear_on_focus_change(new_fence);
        assert_eq!(s.current_fence(), new_fence);
    }

    /// `update_focus_window` が epoch・観測プールに触れず hwnd だけを更新することを固定する
    /// （ADR-106 決定3: 同一プロセス内でのウィンドウ変化用の書き込み口）。
    #[test]
    fn update_focus_hwnd_updates_hwnd_only() {
        let mut s = ObservationStore::default();
        s.clear_on_focus_change(FocusFence {
            epoch: 5,
            hwnd: HwndId(1),
        });
        let mut high = obs(true, ObservationSource::ObserverPoll, Instant::now());
        high.confidence = ObservationConfidence::High;
        rec(&mut s, high);
        s.update_focus_window(HwndId(2));
        assert_eq!(s.current_fence().epoch, 5, "epoch は変わらない");
        assert_eq!(s.current_fence().hwnd, HwndId(2));
        assert!(
            s.observation(ObservationSource::ObserverPoll).is_some(),
            "観測プールはクリアされない"
        );
    }

    /// `update_focus_window` が epoch を保つことを固定する
    /// （PR 109 コードレビュー指摘4: `clear_on_focus_change` との対称テスト）。
    #[test]
    fn update_focus_window_preserves_epoch() {
        let mut s = ObservationStore::default();
        s.clear_on_focus_change(FocusFence {
            epoch: 7,
            hwnd: HwndId(1),
        });
        s.update_focus_window(HwndId(2));
        assert_eq!(
            s.current_fence(),
            FocusFence {
                epoch: 7,
                hwnd: HwndId(2),
            }
        );
    }

    /// code review 2026-08-26 で発見された退行の再現テスト:
    /// 同一プロセス内で hwnd だけが変わったとき、`update_focus_window()` を
    /// 呼ばずに `current_fence.hwnd` を古いまま放置すると、新しい hwnd で
    /// 記録された高信頼観測が `is_identity_ok` に恒久的に拒否される。
    /// `update_focus_window()` を呼べばこれが解消することを固定する。
    #[test]
    fn update_focus_hwnd_unblocks_derive_any_after_intra_process_window_change() {
        let mut s = ObservationStore::default();
        s.clear_on_focus_change(FocusFence {
            epoch: 5,
            hwnd: HwndId(1),
        }); // プロセス変更（epoch=5, hwnd=1）
        let now = Instant::now();

        // 同一プロセス内でウィンドウが hwnd=2 へ変わり、新しいウィンドウの
        // 高信頼観測が届いた（epoch はプロセス変更でのみ進むため 5 のまま）。
        let mut fresh_high = obs(true, ObservationSource::FocusProbe, now);
        fresh_high.confidence = ObservationConfidence::High;
        fresh_high.focus_epoch = 5;
        fresh_high.hwnd = HwndId(2);
        rec(&mut s, fresh_high);

        // update_focus_window() を呼ばない場合: current_fence.hwnd が古い hwnd=1
        // のままのため、正当な新しい観測が拒否され続ける（退行の再現）。
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            None,
            "update_focus_window() を呼ぶ前は hwnd 不一致で新しい観測が拒否される"
        );

        // update_focus_window() で current_fence.hwnd を追従させると受理される。
        s.update_focus_window(HwndId(2));
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            Some(true),
            "update_focus_window() 後は hwnd が一致し観測が採用される"
        );
    }

    /// BUG-102 の再現＋修正確認: 起動直後（bootstrap）に
    /// `establish_initial_focus_scope` が live 側フェンスを
    /// `{epoch: 1, hwnd: 実 hwnd}` に進めるのに対し、`ObservationStore` 側は
    /// `FocusFence::default()`（`{epoch: 0, hwnd: NULL}`）のまま残っていた。
    /// この状態では、起動時にフォーカスされていたアプリで発生する
    /// `ImmCrossProbe` 観測（live 側フェンスでスタンプされる）が
    /// `is_identity_ok` に恒久的に拒否される（別プロセスへ切り替えて戻り
    /// `FocusChanged` が来るまで直らない）。
    ///
    /// `is_identity_ok` は `FocusProbe` も照合対象にするが、`FocusProbe` は
    /// `Low`（`state/evidence.rs`）で `derive_filtered` の High 分岐にも
    /// Medium 分岐にも元から載らないため、フェンスの一致・不一致で結論が
    /// 変わらない。ここで固定するのは `ImmCrossProbe`（High）1 ソースである。
    ///
    /// なお `derive_any()` が `None` を返しても `ImeModel::resolve_open_at` は
    /// `most_recent_trusted()`（フェンス照合なし）にフォールバックするため、
    /// **belief の値として症状が出るのは競合する fresh な Medium 観測がある場合
    /// だけ**である。そちらは
    /// `state::ime_model::tests::bootstrap_fence_desync_lets_medium_poll_override_high_probe`
    /// が固定する。
    #[test]
    fn establish_initial_fence_unblocks_the_first_probe_after_bootstrap() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        let bootstrap_fence = FocusFence {
            epoch: 1, // enter_focus_scope が 0 -> 1 に進めた
            hwnd: HwndId(0xABCD),
        };

        // 起動時にフォーカスされていたアプリの高信頼観測（live 側フェンスでスタンプ）。
        let mut high = obs(true, ObservationSource::ImmCrossProbe, now);
        high.confidence = ObservationConfidence::High;
        high.focus_epoch = bootstrap_fence.epoch;
        high.hwnd = bootstrap_fence.hwnd;
        rec(&mut s, high);

        // 同期しない場合（退行の再現）: current_fence が既定値のままで棄却される。
        assert_eq!(
            s.current_fence(),
            FocusFence::default(),
            "bootstrap 前の current_fence は既定値（epoch=0, hwnd=NULL）"
        );
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            None,
            "fence を同期しないと、起動直後のアプリの観測が epoch/hwnd 不一致で棄却される"
        );

        // 同期すると受理される。
        s.establish_initial_fence(bootstrap_fence);
        assert_eq!(s.current_fence(), bootstrap_fence);
        assert_eq!(
            s.derive_any(now).map(|o| o.value()),
            Some(true),
            "establish_initial_fence 後は live 側と一致し観測が採用される"
        );
    }

    /// `establish_initial_fence` は fence 以外に触れない（観測プール・drift を
    /// クリアしない）ことを固定する。`clear_on_focus_change` との差はここにある。
    #[test]
    fn establish_initial_fence_does_not_clear_the_pool_or_drift() {
        let mut s = ObservationStore::default();
        let now = Instant::now();
        rec(&mut s, obs(true, ObservationSource::ObserverPoll, now));
        s.update_drift(false, true, now);
        assert!(s.drift.is_some());

        s.establish_initial_fence(FocusFence {
            epoch: 1,
            hwnd: HwndId(7),
        });

        assert!(
            s.observation(ObservationSource::ObserverPoll).is_some(),
            "観測プールはクリアされない（bootstrap は「旧窓」を持たないため）"
        );
        assert!(s.drift.is_some(), "drift もクリアされない");
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
            now.checked_sub(Duration::from_secs(1))
                .expect("test instant can be backdated"),
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
        let current_epoch = store.current_fence().epoch;

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
    ///
    /// hwnd は意図的に `HwndId::NULL` 固定（`obs()` ヘルパの既定値）——
    /// このマトリクスが比較するオラクル `legacy_derive_open_filtered`（本ファイル
    /// L1593 付近）は `store.current_fence().epoch` のみを参照し hwnd を知らない
    /// ため、hwnd 軸をここに足しても `derive_any`/`derive_actuating` との比較が
    /// 恒等的に一致し続け、退行を検知できない。ADR-106 の hwnd フェンスは
    /// 別テスト `identity_gate_matrix_covers_epoch_and_hwnd_independently` が
    /// 担当する。
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
                                            current_fence: FocusFence {
                                                epoch: store_epoch,
                                                ..Default::default()
                                            },
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
            s.current_fence.epoch = 1;
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

    /// ADR-106 決定3: epoch と hwnd がそれぞれ独立に derive を除外できることを
    /// 固定する。`pinned_matrix`（hwnd を `HwndId::NULL` 固定）ではこの軸が
    /// 検知できないため、専用のマトリクスとして分離した（PR 109 コードレビュー
    /// 指摘2）。
    #[test]
    fn identity_gate_matrix_covers_epoch_and_hwnd_independently() {
        use super::super::ime_event::ObservationAuthority;
        let now = Instant::now();
        for source in RECORDABLE_SOURCES {
            for obs_epoch in [0_u64, 1] {
                for store_epoch in [0_u64, 1] {
                    for obs_hwnd in [HwndId(1), HwndId(2)] {
                        for store_hwnd in [HwndId(1), HwndId(2)] {
                            let mut s = ObservationStore {
                                current_fence: FocusFence {
                                    epoch: store_epoch,
                                    hwnd: store_hwnd,
                                },
                                ..Default::default()
                            };
                            let mut o = obs(true, source, now);
                            o.confidence = ObservationConfidence::High;
                            o.focus_epoch = obs_epoch;
                            o.hwnd = obs_hwnd;
                            rec(&mut s, o);

                            let fenced = matches!(
                                source,
                                ObservationSource::ImmCrossProbe | ObservationSource::FocusProbe
                            );
                            let admitted =
                                !fenced || (obs_epoch == store_epoch && obs_hwnd == store_hwnd);

                            let label = format!(
                                "source={source:?} obs_epoch={obs_epoch} store_epoch={store_epoch} \
                                 obs_hwnd={obs_hwnd:?} store_hwnd={store_hwnd:?}"
                            );
                            assert_eq!(
                                s.derive_any(now).is_some(),
                                admitted,
                                "derive_any: {label}"
                            );
                            assert_eq!(
                                s.derive_actuating(now).is_some(),
                                admitted && source.authority() == ObservationAuthority::Actuating,
                                "derive_actuating: {label}"
                            );
                        }
                    }
                }
            }
        }
    }
}
