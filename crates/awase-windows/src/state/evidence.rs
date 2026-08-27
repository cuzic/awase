//! IME open 観測の evidence 型（ADR-089 §2.1・§2.2、INV-38/40）。
//!
//! ## 何をコンパイラへ移したか
//!
//! 1. **プール所属の排他**（INV-38）— ある evidence 型が Actuating プールと
//!    BeliefOnly プールの両方に属することは、`OpenEvidence::Pool` 関連型の
//!    コヒーレンス（1 型 1 impl）により**型として表現できない**。
//! 2. **観測の出自**（INV-40）— `Observed<E>` のフィールドは private で、
//!    構築子はソースごとに固有のデータ witness（`&AcceptedObservation` /
//!    `ImePolicyProfile` / `ConvSyncReason`）を要求する。
//!
//! ## `ConvBitsInference` / `GjiIoInference` に evidence 型を作らない理由
//!
//! この 2 値は `PerSourceObservations` に実フィールドを持たず、`get`/`set` が
//! `None`/no-op を返す **input_mode 軸専用**のソースである（ADR-089 §1.3(h)）。
//! `OpenEvidence` を impl すると (a) `record_belief` が黙って no-op になるか、
//! (b) conv 由来の間接推測を open の多数決へ入れる BUG-19 型の経路を新設する
//! かのどちらかになる。
//!
//! ## 実行時 match が残る場所
//!
//! `ImeEvent::ObserverReported` は journal へ直列化される値であり
//! （ADR-082）、reduce はその値から プールを引く。その唯一の口が
//! [`AnyObservation`] と `ObservationStore::record_replayed` である。

use std::marker::PhantomData;

use super::conv_classify::ConvSyncReason;
use super::ime_event::{
    HwndId, ImePolicyProfile, ObservationAuthority, ObservationConfidence, ObservationSource,
};
#[cfg(test)]
use super::probe_admission::FocusFence;
use super::probe_admission::{AcceptedObservation, FocusEpoch};

mod sealed {
    pub trait Sealed {}
}

/// 観測プールの種別。`ActuatingPool` / `BeliefPool` の 2 値のみ。
pub trait PoolKind: sealed::Sealed {
    /// このプールに属する観測が持つ authority。
    ///
    /// `ObservationSource::authority()`（実行時タグ側の SSOT）と食い違うと
    /// `declare_evidence!` の `const _: () = assert!(..)` が**コンパイルを
    /// 失敗させる**（ADR-089 §6 Phase A item 5、INV-38）。
    const AUTHORITY: ObservationAuthority;
}

/// actuation（外部 IME 状態への書き込み）の根拠にしてよい観測のプール。
#[derive(Debug)]
pub struct ActuatingPool;
/// belief（engine の内部挙動決定）にのみ使える観測のプール。
#[derive(Debug)]
pub struct BeliefPool;

impl sealed::Sealed for ActuatingPool {}
impl PoolKind for ActuatingPool {
    const AUTHORITY: ObservationAuthority = ObservationAuthority::Actuating;
}
impl sealed::Sealed for BeliefPool {}
impl PoolKind for BeliefPool {
    const AUTHORITY: ObservationAuthority = ObservationAuthority::BeliefOnly;
}

/// `ObservationAuthority` の const 比較（`PartialEq::eq` は const fn ではない）。
const fn authority_eq(a: ObservationAuthority, b: ObservationAuthority) -> bool {
    matches!(
        (a, b),
        (
            ObservationAuthority::Actuating,
            ObservationAuthority::Actuating
        ) | (
            ObservationAuthority::BeliefOnly,
            ObservationAuthority::BeliefOnly
        )
    )
}

/// open 観測の「根拠としての種別」。1 型につき impl は 1 つしか書けない。
pub trait OpenEvidence: sealed::Sealed {
    /// この evidence がどちらのプールへ入るか。
    type Pool: PoolKind;
    /// journal / 診断ログ用の実行時タグ。`ObservationSource::authority()` と
    /// 一致していることは全数テストで固定する（ADR-089 §6 Phase A item 5）。
    const SOURCE: ObservationSource;
    /// この evidence が名乗ってよい confidence。構築子が固定するため、
    /// 呼び出し元は confidence を選べない。
    const CONFIDENCE: ObservationConfidence;
}

/// evidence 型を宣言する。`type Pool` はここでしか書けないため、
/// 二重所属は構造的に表現できない（INV-38）。
///
/// あわせて **`Pool` の割り当てと `SOURCE.authority()` の一致をコンパイル時に
/// 検査する**（`const _: () = assert!(..)`）。プールを取り違えて宣言すると
/// テストを 1 本も走らせる前に `cargo build` が落ちる。
macro_rules! declare_evidence {
    ($( $(#[$meta:meta])* $name:ident => $pool:ty, $source:ident, $conf:ident );* $(;)?) => {
        $(
            $(#[$meta])*
            #[derive(Debug)]
            pub struct $name;
            impl sealed::Sealed for $name {}
            impl OpenEvidence for $name {
                type Pool = $pool;
                const SOURCE: ObservationSource = ObservationSource::$source;
                const CONFIDENCE: ObservationConfidence = ObservationConfidence::$conf;
            }
            const _: () = assert!(
                authority_eq(
                    <$pool as PoolKind>::AUTHORITY,
                    ObservationSource::$source.authority(),
                ),
                concat!(
                    stringify!($name),
                    " の Pool 割り当てが ObservationSource::",
                    stringify!($source),
                    ".authority() と食い違っている（ADR-089 INV-38）",
                ),
            );
        )*
    };
}

declare_evidence! {
    /// `ImmGetOpenStatus` 直接呼び出し。
    ImmGetOpenStatus => ActuatingPool, ImmGetOpenStatus, High;
    /// フォーカス変更後の ImmCross 非同期プローブ（子 hwnd の高信頼読み取り）。
    ImmCrossProbe => ActuatingPool, ImmCrossProbe, High;
    /// 500ms 周期のバックグラウンドポーリング。
    ObserverPoll => ActuatingPool, ObserverPoll, Medium;
    /// GJI (GetGuiThreadInfo) 由来。
    Gji => ActuatingPool, Gji, Medium;
    /// TSF observer 由来。
    Tsf => ActuatingPool, Tsf, Medium;
    /// conv ビットからの open 状態推定（`report_conv_open_inference` 専用）。
    ConvOpenInference => BeliefPool, ConvOpenInference, Medium;
    /// 観測が一切ない状況での安全デフォルト推測。
    HeuristicDefault => BeliefPool, HeuristicDefault, Low;
    /// per-HWND IME キャッシュからの復元。
    HwndCache => BeliefPool, HwndCache, Medium;
    /// フォーカス変更直後の同期プローブ（`hwndFocus` の IMC。BUG-88 参照:
    /// 真の top-level ウィンドウとは限らない）。
    FocusProbe => BeliefPool, FocusProbe, Low;
}

/// 出自が型で確定している open 観測。
///
/// フィールドは private であり、構築子は evidence ごとに固有の witness を
/// 要求する（INV-40）。`at`（観測時刻）は持たない——時刻は `ImeEventLog` が
/// envelope に付ける値が SSOT であり、ここで別の時刻を持つと journal replay が
/// 本番と別の時刻で再生される。
pub struct Observed<E: OpenEvidence> {
    open: bool,
    hwnd: HwndId,
    focus_epoch: FocusEpoch,
    _evidence: PhantomData<fn() -> E>,
}

impl<E: OpenEvidence> std::fmt::Debug for Observed<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Observed")
            .field("source", &E::SOURCE)
            .field("open", &self.open)
            .field("hwnd", &self.hwnd)
            .field("focus_epoch", &self.focus_epoch)
            .finish()
    }
}

impl<E: OpenEvidence> Clone for Observed<E> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<E: OpenEvidence> Copy for Observed<E> {}

impl<E: OpenEvidence> Observed<E> {
    const fn new(open: bool, hwnd: HwndId, focus_epoch: FocusEpoch) -> Self {
        Self {
            open,
            hwnd,
            focus_epoch,
            _evidence: PhantomData,
        }
    }

    #[must_use]
    pub const fn open(&self) -> bool {
        self.open
    }
}

impl Observed<FocusProbe> {
    /// probe 経路でしか作れない。`AcceptedObservation` は
    /// `state/probe_admission.rs` でしか構築できない。
    ///
    /// `hwnd` 引数は取らず `accepted.hwnd()` を使う（PR 109 コードレビュー
    /// 指摘4の軽微6: 呼び出し元が別途渡す hwnd と `accepted` の hwnd が
    /// epoch とは別の経路でずれうる、`FocusFence` 統合が閉じたい構造その
    /// ものだったため）。
    #[must_use]
    pub const fn from_probe(accepted: &AcceptedObservation, open: bool) -> Self {
        Self::new(open, accepted.hwnd(), accepted.epoch())
    }
}

impl Observed<ImmCrossProbe> {
    /// `ImmLikeTicket::admit()` を通った非同期 probe 専用。
    /// `hwnd` は `accepted.hwnd()` を使う（[`Observed::<FocusProbe>::from_probe`] 参照）。
    #[must_use]
    pub const fn from_cross_probe(accepted: &AcceptedObservation, open: bool) -> Self {
        Self::new(open, accepted.hwnd(), accepted.epoch())
    }
}

impl Observed<ObserverPoll> {
    /// 周期ポーリングの結果専用。
    /// `hwnd` は `accepted.hwnd()` を使う（[`Observed::<FocusProbe>::from_probe`] 参照）。
    #[must_use]
    pub const fn from_poll(accepted: &AcceptedObservation, open: bool) -> Self {
        Self::new(open, accepted.hwnd(), accepted.epoch())
    }
}

impl Observed<HeuristicDefault> {
    /// 引数が起点を限定する。profile なしには作れない。
    ///
    /// 「観測が一切無い」ことを根拠にした安全デフォルトであり、confidence は
    /// `Low` 固定（呼び出し元は選べない）。
    #[must_use]
    pub const fn at_startup(
        _profile: ImePolicyProfile,
        open: bool,
        hwnd: HwndId,
        focus_epoch: FocusEpoch,
    ) -> Self {
        Self::new(open, hwnd, focus_epoch)
    }
}

impl Observed<ConvOpenInference> {
    /// conv ビットを分類した事実そのもの（`ConvSyncReason`）を引数に要求する。
    /// confidence は `Medium` 固定（BUG-19 対策の上限）。
    #[must_use]
    pub const fn from_conv(
        _reason: ConvSyncReason,
        open: bool,
        hwnd: HwndId,
        focus_epoch: FocusEpoch,
    ) -> Self {
        Self::new(open, hwnd, focus_epoch)
    }
}

// `ImmGetOpenStatus` / `Gji` / `Tsf` / `HwndCache` には構築子を置かない。
// これらを open 観測として record する本番経路が現時点で存在しないため
// （`state/ime_event.rs` の `gji_and_tsf_are_actuating_by_type_though_unused_in_practice`
// 参照）。経路を追加するときに、その経路が持つ外部事実を witness にした構築子を
// ここへ足すこと。

/// 出自の型を落とした open 観測。`ImeEvent::ObserverReported` の payload。
///
/// **通常の構築経路は [`Observed<E>`] からの変換のみ**であり、
/// 各 evidence の witness を通らずにこの値を作ることはできない。
/// journal / fixture からの復元だけが [`AnyObservation::restored_from_journal`]
/// を使う（ADR-089 §2.1「型で消せない残余であり、隠さず1箇所に集める」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct AnyObservation {
    open: bool,
    source: ObservationSource,
    hwnd: HwndId,
    confidence: ObservationConfidence,
    focus_epoch: FocusEpoch,
}

impl AnyObservation {
    /// journal / fixture / テストからの復元専用の口。
    ///
    /// **本番コードから呼んではならない**（`tests/architecture_guard.rs::
    /// any_observation_replay_door_is_not_used_in_production` が固定する）。
    /// 本番の観測は必ず `Observed<E>` の witness 構築子を通すこと。
    #[must_use]
    pub const fn restored_from_journal(
        open: bool,
        source: ObservationSource,
        hwnd: HwndId,
        confidence: ObservationConfidence,
        focus_epoch: FocusEpoch,
    ) -> Self {
        Self {
            open,
            source,
            hwnd,
            confidence,
            focus_epoch,
        }
    }

    #[must_use]
    pub const fn open(&self) -> bool {
        self.open
    }
    #[must_use]
    pub const fn source(&self) -> ObservationSource {
        self.source
    }
    #[must_use]
    pub const fn hwnd(&self) -> HwndId {
        self.hwnd
    }
    #[must_use]
    pub const fn confidence(&self) -> ObservationConfidence {
        self.confidence
    }
    #[must_use]
    pub const fn focus_epoch(&self) -> FocusEpoch {
        self.focus_epoch
    }
}

impl<E: OpenEvidence> From<Observed<E>> for AnyObservation {
    fn from(o: Observed<E>) -> Self {
        Self {
            open: o.open,
            source: E::SOURCE,
            hwnd: o.hwnd,
            confidence: E::CONFIDENCE,
            focus_epoch: o.focus_epoch,
        }
    }
}

/// ユーザー意図の witness（ADR-089 §2.2、INV-40）。
///
/// `UserIntentSource::SyncKey` / `PhysicalImeKey` は、**注入されていない**物理
/// キーイベントが実在したことを引数の型で要求する。BUG-14（外部注入された IME
/// モードキーが意図に昇格し、ユーザーの明示 OFF を上書きし続けた）の型化。
///
/// `UserIntentSource::Command`（engine 内部判断）は witness に載せられる外部
/// 事実が無いため対象外（ADR-089 §2.2・§9-8）。`write_set_open_request` は
/// `tests/architecture_guard.rs::user_intent_source_construction_is_limited_to_typed_writers`
/// が期待値 1 で守り続ける。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntentWitness {
    source: super::ime_event::UserIntentSource,
}

impl IntentWitness {
    /// 物理 IME キー（VK_F3/F4 等）由来の明示意図。
    ///
    /// `injected == true`（他プロセスの SendInput 由来）と、そもそも
    /// shadow_action を持たないキーは `None`。
    #[must_use]
    pub fn from_physical(e: &awase::types::RawKeyEvent) -> Option<Self> {
        (!e.injected && e.ime_relevance.shadow_action.is_some()).then_some(Self {
            source: super::ime_event::UserIntentSource::PhysicalImeKey,
        })
    }

    /// 設定された同期キー（Shift+Space 等）由来の明示意図。
    #[must_use]
    pub fn from_sync_key(e: &awase::types::RawKeyEvent) -> Option<Self> {
        (!e.injected && e.ime_relevance.sync_direction.is_some()).then_some(Self {
            source: super::ime_event::UserIntentSource::SyncKey,
        })
    }

    #[must_use]
    pub const fn source(&self) -> super::ime_event::UserIntentSource {
        self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `E::Pool`（型）・`E::SOURCE.authority()`（実行時タグ）・手書き期待値の
    /// **3 者**が一致することを evidence 型 9 個の全数で確認する
    /// （ADR-089 §6 Phase A item 5、INV-38）。
    ///
    /// `E::Pool` を**実際に参照する**のが要点。`E::SOURCE.authority()` と
    /// 手書き期待値だけを比べる形だと、`declare_evidence!` のプール割り当てを
    /// 取り違えても（`record`/`record_belief` の呼び出し元がまだ無い Phase A では）
    /// どこも落ちない。なお同じ一致は `declare_evidence!` 内の
    /// `const _: () = assert!(..)` がコンパイル時にも見ているため、取り違えは
    /// **ビルドが通らない**か、この全数テストが落ちるかのいずれかになる。
    fn assert_pool<E: OpenEvidence>(expected: ObservationAuthority) {
        assert_eq!(
            <E::Pool as PoolKind>::AUTHORITY,
            expected,
            "{:?} の OpenEvidence::Pool が期待するプールと違う",
            E::SOURCE
        );
        assert_eq!(
            E::SOURCE.authority(),
            <E::Pool as PoolKind>::AUTHORITY,
            "{:?} の authority() が OpenEvidence::Pool と食い違っている",
            E::SOURCE
        );
    }

    /// プール型そのものの authority 割り当て（上の全数テストの基準点）。
    #[test]
    fn pool_kinds_carry_their_authority() {
        assert_eq!(
            ActuatingPool::AUTHORITY,
            ObservationAuthority::Actuating,
            "ActuatingPool の AUTHORITY"
        );
        assert_eq!(
            BeliefPool::AUTHORITY,
            ObservationAuthority::BeliefOnly,
            "BeliefPool の AUTHORITY"
        );
    }

    #[test]
    fn actuating_pool_evidence_matches_authority() {
        assert_pool::<ImmGetOpenStatus>(ObservationAuthority::Actuating);
        assert_pool::<ImmCrossProbe>(ObservationAuthority::Actuating);
        assert_pool::<ObserverPoll>(ObservationAuthority::Actuating);
        assert_pool::<Gji>(ObservationAuthority::Actuating);
        assert_pool::<Tsf>(ObservationAuthority::Actuating);
    }

    #[test]
    fn belief_pool_evidence_matches_authority() {
        assert_pool::<ConvOpenInference>(ObservationAuthority::BeliefOnly);
        assert_pool::<HeuristicDefault>(ObservationAuthority::BeliefOnly);
        assert_pool::<HwndCache>(ObservationAuthority::BeliefOnly);
        assert_pool::<FocusProbe>(ObservationAuthority::BeliefOnly);
    }

    /// 9 個の evidence 型が 9 個の異なるソースを覆っていること
    /// （`PerSourceObservations` の 9 フィールドと 1:1）。
    #[test]
    fn evidence_sources_are_nine_distinct_recordable_sources() {
        let sources = [
            ImmGetOpenStatus::SOURCE,
            ImmCrossProbe::SOURCE,
            ObserverPoll::SOURCE,
            Gji::SOURCE,
            Tsf::SOURCE,
            ConvOpenInference::SOURCE,
            HeuristicDefault::SOURCE,
            HwndCache::SOURCE,
            FocusProbe::SOURCE,
        ];
        assert_eq!(sources.len(), 9);
        for (i, a) in sources.iter().enumerate() {
            for b in &sources[i + 1..] {
                assert_ne!(a, b, "evidence 型が同じ ObservationSource を名乗っている");
            }
        }
    }

    #[test]
    fn observed_carries_evidence_source_and_confidence() {
        let accepted = AcceptedObservation::for_sync(FocusFence {
            epoch: 7,
            hwnd: HwndId(1),
        });
        let any: AnyObservation = Observed::<FocusProbe>::from_probe(&accepted, true).into();
        assert_eq!(any.source(), ObservationSource::FocusProbe);
        assert_eq!(any.confidence(), ObservationConfidence::Low);
        assert_eq!(any.focus_epoch(), 7);
        assert!(any.open());
        assert_eq!(any.hwnd(), HwndId(1));

        let any: AnyObservation =
            Observed::<ImmCrossProbe>::from_cross_probe(&accepted, false).into();
        assert_eq!(any.confidence(), ObservationConfidence::High);
        assert!(!any.open());
    }

    #[test]
    fn heuristic_default_is_always_low_confidence() {
        let any: AnyObservation = Observed::<HeuristicDefault>::at_startup(
            ImePolicyProfile::Imm32Unavailable,
            true,
            HwndId::NULL,
            0,
        )
        .into();
        assert_eq!(any.confidence(), ObservationConfidence::Low);
    }

    #[test]
    fn conv_open_inference_is_capped_at_medium() {
        let any: AnyObservation = Observed::<ConvOpenInference>::from_conv(
            ConvSyncReason::NativeToggleShadowOff,
            true,
            HwndId::NULL,
            3,
        )
        .into();
        assert_eq!(any.confidence(), ObservationConfidence::Medium);
        assert_eq!(any.source(), ObservationSource::ConvOpenInference);
    }

    // ── IntentWitness（BUG-14 の型化） ──

    fn key_event(injected: bool, shadow: bool, sync: bool) -> awase::types::RawKeyEvent {
        use awase::types::{
            ImeRelevance, KeyClassification, KeyEventType, ModifierState, ScanCode,
            ShadowImeAction, VkCode,
        };
        awase::types::RawKeyEvent {
            vk_code: VkCode(0xF2),
            scan_code: ScanCode(0),
            event_type: KeyEventType::KeyDown,
            extra_info: 0,
            timestamp: 0,
            key_classification: KeyClassification::Passthrough,
            physical_pos: None,
            ime_relevance: ImeRelevance {
                may_change_ime: true,
                shadow_action: shadow.then_some(ShadowImeAction::TurnOn),
                is_sync_key: sync,
                sync_direction: sync.then_some(ShadowImeAction::TurnOn),
                is_ime_control: false,
            },
            modifier_key: None,
            modifier_snapshot: ModifierState::default(),
            injected,
        }
    }

    #[test]
    fn injected_events_never_become_intent_witnesses() {
        let injected = key_event(true, true, true);
        assert!(IntentWitness::from_physical(&injected).is_none());
        assert!(IntentWitness::from_sync_key(&injected).is_none());
    }

    #[test]
    fn physical_and_sync_witnesses_carry_their_source() {
        use crate::state::ime_event::UserIntentSource;
        let physical = key_event(false, true, false);
        assert_eq!(
            IntentWitness::from_physical(&physical).map(|w| w.source()),
            Some(UserIntentSource::PhysicalImeKey)
        );
        assert!(
            IntentWitness::from_sync_key(&physical).is_none(),
            "sync_direction が無いキーは SyncKey 意図になれない"
        );

        let sync = key_event(false, false, true);
        assert_eq!(
            IntentWitness::from_sync_key(&sync).map(|w| w.source()),
            Some(UserIntentSource::SyncKey)
        );
        assert!(
            IntentWitness::from_physical(&sync).is_none(),
            "shadow_action が無いキーは PhysicalImeKey 意図になれない"
        );
    }
}
