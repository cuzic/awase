//! OpenWarrant — 外部 IME open/close 状態への書き込み（actuation）の授権
//! （ADR-087 §2.3 P11・P12・P15・P16、§4 INV-20〜28）。
//!
//! ## 位置づけ
//!
//! `ImeModel::effective_open()`（`ime_model.rs`）は NICOLA engine の**内部挙動**
//! （かな変換するか否か）を決める belief であり、`derive_any()` の Medium 単独
//! 多数決を含め、誤りが可逆・低コストな弱い証拠で決めてよい（BUG-26 がこれに
//! 依拠している、`ime_model.rs::resolve_open_at` の doc 参照）。
//!
//! `issue_open_warrant()` はこれとは別に、**OS 側 IME への実際の書き込み**
//! （不可逆・高コスト）を許可してよいかを判定する。両者は意図的に異なる
//! ロジックを持ち、`issue_open_warrant()` は `effective_open()` を一切参照しない
//! （ADR-087 §2.3 P11: belief と actuation warrant は同じ bool を共有しない）。
//!
//! ## Step 0〜4 の評価順序（§2.3 P15、round3 で確定）
//!
//! ```text
//! Step 0: override 権限を持つ真の安全弁（PanicReset/ProfilePolicy）が active
//!         → SafetyValve（意図より先に評価する。ForceGuardSet::effective_open()
//!           の意味論そのもの。§7 round3 M2）
//! Step 1: IntentStore に対象への有効な明示意図がある
//!         → ExplicitUserIntent（成立すれば以降を評価しない）
//! Step 3: authority()==Actuating な観測が derive_any() 相当の判定を満たす
//!         → DirectRead / Corroborated
//! Step 4a: HeuristicDefault 観測が実在する → HeuristicGuess(Observation)
//! Step 4b: override 権限を持たないヒューリスティック guard が active
//!          （BrokenAppBootstrap 等）→ HeuristicGuess(Guard)
//! Step 4c: policy.default_feedback == Blind（実 IME の open 状態を直接観測する
//!          手段が構造的に無いプロファイル）→ OwnSsot（desired_open を採用）
//! ```
//!
//! （旧 Step 2 は Step 0 に統合済み。round2 で「述語を `ForceGuardSet::
//! effective_open()` と同じにする」と修正したが、優先順位付き評価では
//! Step 1 の後に置くと `has_explicit_intent` が既に false 確定のため述語が
//! `requires_on()` に潰れて no-op だった。§7 round3 M4。）
//!
//! `requested: bool`（呼び出し元が実際に書き込みたい値）と、各 Step が示す値が
//! 一致しない場合は `None` を返す（ADR-087 §4 INV-20、Codex round1 #1）。

use super::app_ime_policy::AppImePolicy;
use super::force_guard::{ForceGuardSet, ForceOnReason};
use super::ime_actuation::FeedbackPolicy;
use super::ime_event::{HwndId, ObservationSource};
use super::intent_store::{IntentStore, RecordedTargetIntent};
use super::observation_store::{DeriveOutcome, ObservationStore};
use super::TickMs;
use std::time::Instant;

/// 「実 IME を外部から書き換えてよい」という授権。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWarrant {
    /// この warrant が正当化する open 値。
    pub target: bool,
    pub basis: WarrantBasis,
}

/// warrant の根拠。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarrantBasis {
    /// ユーザーが SyncKey/PhysicalImeKey/Command で明示的に操作した。
    ExplicitUserIntent(RecordedTargetIntent),
    /// High confidence の直接 API 読み取り（`ImmGetOpenStatus`/`ImmCrossProbe` 等）
    /// **のみ**。**observation-based correction 経路専用**（ADR-086 §2.1 の
    /// force-write には使わない、INV-25）。
    DirectRead(ObservationSource),
    /// Medium confidence の**単独**ソース（間接観測、他に合意する observation
    /// が無い）。`DirectRead` とは意味が違うため別 variant にする——
    /// `ObserverPoll`（500ms 周期のポーリング）等は「直接読んだ」わけではない
    /// （ADR-087 §7 round4 S-B: 旧実装は Medium 単独ソースも `DirectRead` に
    /// 分類しており、journal を読む人が「直接 API を読んだ」と誤解しうる名前
    /// だった）。`DirectRead` 同様 observation-based correction 専用。
    SingleIndirect(ObservationSource),
    /// 独立した2ソース以上の Medium 合意。`DirectRead`/`SingleIndirect` 同様
    /// observation-based correction 専用。
    Corroborated {
        a: ObservationSource,
        b: ObservationSource,
    },
    /// override 権限を持つ真の安全弁（`PanicReset`/`ProfilePolicy`）。
    SafetyValve(ForceOnReason),
    /// 観測が一切無い状況での、profile 依存の安全デフォルト推測。
    HeuristicGuess(HeuristicGuessSource),
    /// awase 自身の意図（`desired_open`）を、`HeuristicGuess` すら成立しない
    /// 状況での最終的な根拠として使う。`policy.default_feedback == Blind`
    /// （実 IME の open 状態を直接観測する手段が構造的に無いプロファイル）
    /// でのみ発行される——**プロファイル名の raw な一致では分岐しない**
    /// （ADR-087 §7 round3 M1: `AppImeProfile` で分岐すると
    /// `CASCADIA_HOSTING_WINDOW_CLASS` のような「Imm32Unavailable にも
    /// is_tsf_native にも該当するクラス」を誤判定し、BUG-16 を再発させる）。
    OwnSsot,
}

/// `WarrantBasis::HeuristicGuess` の内訳。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeuristicGuessSource {
    /// `HeuristicDefault` 観測（`reset_stale_ime_on_for_imm_broken()` が
    /// Imm32Unavailable 入場時に記録するもの）に基づく。
    Observation(ObservationSource),
    /// override 権限を持たないヒューリスティック guard（`BrokenAppBootstrap` 等）
    /// に基づく。
    Guard(ForceOnReason),
}

/// `issue_open_warrant()` が参照する状態一式。1回の呼び出しの間は不変
/// （呼び出し元が `Instant::now()`/`TickMs` を含め全て事前に確定させて渡す、
/// INV-23）。
///
/// `requested`/`target` を含めなかったのは、この2つだけは同一の状態
/// スナップショットに対して複数回変えて呼ばれうるため（例: drift correction
/// は `desired` を動的に変えながら同じ状態に対して問い合わせる）。
/// ADR-087 §7 round4 N-A: 実配線（Phase 3）で `runtime/mod.rs` の3つの
/// 呼び出し元がそれぞれ同じ組み立てを繰り返すのを避けるため、状態部分を
/// 1つの構造体にまとめた。
#[derive(Debug, Clone, Copy)]
pub struct WarrantContext<'a> {
    pub intent_store: &'a IntentStore,
    pub obs: &'a ObservationStore,
    pub guards: &'a ForceGuardSet,
    pub policy: &'a AppImePolicy,
    /// `ImeModel.desired_open` 相当。`OwnSsot`（Step 4c）の根拠にのみ使う。
    pub desired_open: bool,
    /// スコープ判定（観測ではないため INV-25 の対象外）。
    pub is_japanese_ime: bool,
    pub now: Instant,
    pub now_ms: TickMs,
}

/// 唯一の発行点。純粋関数（`ctx.now`/`ctx.now_ms` を通じて時刻を受け取り、
/// `Instant::now()` を内部で呼ばない。ADR-087 §4 INV-23）。
///
/// - `requested`: 呼び出し元が実際に書き込みたい値。各 Step が示す値と一致
///   しない場合は `None` を返す（INV-20）。
/// - `target`: 書き込み先の識別子。`ctx.intent_store.lookup()` の対象一致に使う。
#[must_use]
pub fn issue_open_warrant(
    requested: bool,
    target: HwndId,
    ctx: &WarrantContext<'_>,
) -> Option<OpenWarrant> {
    if !ctx.is_japanese_ime {
        return None;
    }

    // Step 0: 真の安全弁。明示意図より先に評価する（§7 round3 M2）。
    if let Some(reason) = ctx.guards.active_override_reason() {
        return finalize(requested, true, WarrantBasis::SafetyValve(reason));
    }

    // Step 1: 明示意図（IntentStore、対象一致・TTL は lookup 内部で判定済み）。
    if let Some(intent) = ctx.intent_store.lookup(target, ctx.now_ms) {
        return finalize(
            requested,
            intent.open,
            WarrantBasis::ExplicitUserIntent(*intent),
        );
    }

    // Step 3: authority()==Actuating な観測。
    if let Some(outcome) = ctx.obs.derive_actuating(ctx.now) {
        let basis = match outcome {
            DeriveOutcome::HighSingle { source, .. } => WarrantBasis::DirectRead(source),
            DeriveOutcome::MediumConsensus {
                first,
                second: Some(second),
                ..
            } => WarrantBasis::Corroborated {
                a: first,
                b: second,
            },
            DeriveOutcome::MediumConsensus {
                first,
                second: None,
                ..
            } => WarrantBasis::SingleIndirect(first),
        };
        return finalize(requested, outcome.value(), basis);
    }

    // Step 4a: HeuristicDefault 観測が実在する（鮮度窓は適用しない、
    // ObservationStore::heuristic_default() の doc 参照、§7 round4 S-C）。
    if let Some(o) = ctx.obs.heuristic_default(ctx.now) {
        return finalize(
            requested,
            o.open,
            WarrantBasis::HeuristicGuess(HeuristicGuessSource::Observation(
                ObservationSource::HeuristicDefault,
            )),
        );
    }

    // Step 4b: override 権限を持たないヒューリスティック guard。
    if let Some(reason) = ctx.guards.active_heuristic_reason() {
        return finalize(
            requested,
            true,
            WarrantBasis::HeuristicGuess(HeuristicGuessSource::Guard(reason)),
        );
    }

    // Step 4c: OwnSsot（`default_feedback == Blind` のときのみ）。
    if matches!(ctx.policy.default_feedback, FeedbackPolicy::Blind { .. }) {
        return finalize(requested, ctx.desired_open, WarrantBasis::OwnSsot);
    }

    None
}

/// 各 Step が示した値（`resolved`）が `requested` と一致するときのみ
/// `OpenWarrant` を発行する（ADR-087 §4 INV-20）。
fn finalize(requested: bool, resolved: bool, basis: WarrantBasis) -> Option<OpenWarrant> {
    if resolved == requested {
        Some(OpenWarrant {
            target: resolved,
            basis,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::force_guard::ForceGuard;
    use crate::state::ime_actuation::FeedbackPolicy;
    use crate::state::ime_event::{ImePolicyProfile, ObservationConfidence};
    use crate::state::observation_store::ImeObservation;

    const TARGET: HwndId = HwndId(0x1234);

    #[allow(clippy::too_many_arguments)]
    fn ctx<'a>(
        intent_store: &'a IntentStore,
        obs: &'a ObservationStore,
        guards: &'a ForceGuardSet,
        policy: &'a AppImePolicy,
        desired_open: bool,
        is_japanese_ime: bool,
        now: Instant,
        now_ms: TickMs,
    ) -> WarrantContext<'a> {
        WarrantContext {
            intent_store,
            obs,
            guards,
            policy,
            desired_open,
            is_japanese_ime,
            now,
            now_ms,
        }
    }

    /// テスト用の直接記録（ADR-089 §2.1: 本番経路は `Observed<E>` の witness
    /// 構築子を通るが、ストアの単体テストは任意ソースを仕込む必要がある）。
    fn rec(store: &mut ObservationStore, o: ImeObservation) {
        store.per_source.set(o.source, o);
    }

    fn obs_at(
        open: bool,
        source: ObservationSource,
        confidence: ObservationConfidence,
        at: Instant,
    ) -> ImeObservation {
        ImeObservation {
            open,
            source,
            at,
            hwnd: TARGET,
            confidence,
            expires_at: None,
            focus_epoch: 0,
        }
    }

    #[test]
    fn is_japanese_ime_false_always_none() {
        let store = IntentStore::default();
        let obs = ObservationStore::default();
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::TsfNative);
        let now = Instant::now();
        assert_eq!(
            issue_open_warrant(
                true,
                TARGET,
                &ctx(&store, &obs, &guards, &policy, true, false, now, TickMs(0))
            ),
            None
        );
    }

    #[test]
    fn step0_safety_valve_wins_even_over_explicit_off_intent() {
        // ADR-087 §7 round3 M2: PanicReset は明示 OFF 意図があっても override する。
        let mut store = IntentStore::default();
        store.record(
            TARGET,
            false,
            crate::state::ime_event::UserIntentSource::PhysicalImeKey,
            TickMs(0),
        );
        let obs = ObservationStore::default();
        let mut guards = ForceGuardSet::default();
        guards.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: 1,
        });
        let policy = AppImePolicy::from_profile(ImePolicyProfile::TsfNative);
        let now = Instant::now();
        let warrant = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, false, true, now, TickMs(0)),
        );
        assert_eq!(
            warrant,
            Some(OpenWarrant {
                target: true,
                basis: WarrantBasis::SafetyValve(ForceOnReason::PanicReset),
            })
        );
    }

    #[test]
    fn step0_broken_app_bootstrap_does_not_override_explicit_off_intent() {
        // BrokenAppBootstrap は override 権限を持たないため、Step 0 では発行されない。
        let mut store = IntentStore::default();
        store.record(
            TARGET,
            false,
            crate::state::ime_event::UserIntentSource::PhysicalImeKey,
            TickMs(0),
        );
        let obs = ObservationStore::default();
        let mut guards = ForceGuardSet::default();
        guards.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 1,
        });
        let policy = AppImePolicy::from_profile(ImePolicyProfile::Imm32Unavailable);
        let now = Instant::now();
        let warrant = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, true, true, now, TickMs(0)),
        );
        assert_eq!(
            warrant, None,
            "Step1 の明示 OFF 意図が勝ち、requested=true とは一致しないため None"
        );
    }

    #[test]
    fn step1_explicit_intent_blocks_actuating_observation() {
        let mut store = IntentStore::default();
        store.record(
            TARGET,
            false,
            crate::state::ime_event::UserIntentSource::PhysicalImeKey,
            TickMs(0),
        );
        let mut obs = ObservationStore::default();
        let now = Instant::now();
        rec(
            &mut obs,
            obs_at(
                true,
                ObservationSource::ImmGetOpenStatus,
                ObservationConfidence::High,
                now,
            ),
        );
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::ImmCross);
        let warrant = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, true, true, now, TickMs(0)),
        );
        assert_eq!(
            warrant, None,
            "Step1 の明示 OFF 意図が Step3 の High 観測より優先される"
        );
    }

    #[test]
    fn step3_direct_read_high_single() {
        let store = IntentStore::default();
        let mut obs = ObservationStore::default();
        let now = Instant::now();
        rec(
            &mut obs,
            obs_at(
                true,
                ObservationSource::ImmGetOpenStatus,
                ObservationConfidence::High,
                now,
            ),
        );
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::ImmCross);
        let warrant = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, true, true, now, TickMs(0)),
        );
        assert_eq!(
            warrant,
            Some(OpenWarrant {
                target: true,
                basis: WarrantBasis::DirectRead(ObservationSource::ImmGetOpenStatus),
            })
        );
    }

    #[test]
    fn step3_corroborated_when_two_medium_sources_agree() {
        let store = IntentStore::default();
        let mut obs = ObservationStore::default();
        let now = Instant::now();
        rec(
            &mut obs,
            obs_at(
                true,
                ObservationSource::ObserverPoll,
                ObservationConfidence::Medium,
                now,
            ),
        );
        rec(
            &mut obs,
            obs_at(
                true,
                ObservationSource::Gji,
                ObservationConfidence::Medium,
                now,
            ),
        );
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::ImmCross);
        let warrant = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, true, true, now, TickMs(0)),
        )
        .unwrap();
        match warrant.basis {
            WarrantBasis::Corroborated { .. } => {}
            other => panic!("Corroborated を期待したが {other:?}"),
        }
    }

    #[test]
    fn step3_conv_open_inference_never_wins_actuation() {
        // ADR-087 発端バグ（mise→くした）の actuation 側の再現。
        let store = IntentStore::default();
        let mut obs = ObservationStore::default();
        let now = Instant::now();
        rec(
            &mut obs,
            obs_at(
                true,
                ObservationSource::ConvOpenInference,
                ObservationConfidence::Medium,
                now,
            ),
        );
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::TsfNative);
        let warrant = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, false, true, now, TickMs(0)),
        );
        assert_eq!(
            warrant, None,
            "ConvOpenInference は BeliefOnly のため Step3 は素通りし、\
             desired_open=false から OwnSsot が false を返すため requested=true とは不一致で None"
        );
    }

    #[test]
    fn step4c_own_ssot_for_blind_profile_matches_bug16() {
        // BUG-16 非退行: desired_open=true、明示意図なし、observations 空、
        // Blind プロファイル → OwnSsot(true)。
        let store = IntentStore::default();
        let obs = ObservationStore::default();
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::TsfNative);
        let now = Instant::now();
        let warrant = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, true, true, now, TickMs(0)),
        );
        assert_eq!(
            warrant,
            Some(OpenWarrant {
                target: true,
                basis: WarrantBasis::OwnSsot,
            })
        );
    }

    #[test]
    fn step4c_does_not_fire_for_read_profile() {
        // ADR-087 §7 round3 M5: ImePolicyProfile::{Plain,Unknown} も ImmCross
        // 同様 Read にマップされる。起動直後（ImeModel::new() の初期値）も
        // Read のため、OwnSsot は発行されない。
        let store = IntentStore::default();
        let obs = ObservationStore::default();
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::ImmCross);
        assert!(matches!(
            policy.default_feedback,
            FeedbackPolicy::Read { .. }
        ));
        let now = Instant::now();
        let warrant = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, true, true, now, TickMs(0)),
        );
        assert_eq!(
            warrant, None,
            "Read プロファイルでは Step4c(OwnSsot) が発行されない"
        );
    }

    #[test]
    fn step4c_branches_on_default_feedback_not_raw_profile_value() {
        // ADR-087 §7 round3 M1 の直接的な pinned test:
        // CASCADIA_HOSTING_WINDOW_CLASS 相当（実際には Imm32Unavailable と
        // 分類されうるが default_feedback は Blind になりうる想定）でも、
        // 分岐が「AppImeProfile の値」ではなく default_feedback で行われる
        // ことを、Imm32Unavailable プロファイルでも確認する。
        let store = IntentStore::default();
        let obs = ObservationStore::default();
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::Imm32Unavailable);
        assert!(matches!(
            policy.default_feedback,
            FeedbackPolicy::Blind { .. }
        ));
        let now = Instant::now();
        let warrant = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, true, true, now, TickMs(0)),
        );
        assert_eq!(
            warrant,
            Some(OpenWarrant {
                target: true,
                basis: WarrantBasis::OwnSsot,
            }),
            "HeuristicDefault 観測も guard も無い Imm32Unavailable は \
             Step4a/4b をすり抜けて Step4c(OwnSsot) に到達する（default_feedback \
             ベースの分岐が正しく機能している証拠）"
        );
    }

    #[test]
    fn step4a_heuristic_default_observation_wins_before_own_ssot() {
        let store = IntentStore::default();
        let mut obs = ObservationStore::default();
        let now = Instant::now();
        rec(
            &mut obs,
            obs_at(
                false,
                ObservationSource::HeuristicDefault,
                ObservationConfidence::Low,
                now,
            ),
        );
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::Imm32Unavailable);
        let warrant = issue_open_warrant(
            false,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, true, true, now, TickMs(0)),
        );
        assert_eq!(
            warrant,
            Some(OpenWarrant {
                target: false,
                basis: WarrantBasis::HeuristicGuess(HeuristicGuessSource::Observation(
                    ObservationSource::HeuristicDefault
                )),
            })
        );
    }

    #[test]
    fn step4b_broken_app_bootstrap_fires_without_explicit_intent() {
        let store = IntentStore::default();
        let obs = ObservationStore::default();
        let mut guards = ForceGuardSet::default();
        guards.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 1,
        });
        let policy = AppImePolicy::from_profile(ImePolicyProfile::Imm32Unavailable);
        let now = Instant::now();
        let warrant = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, false, true, now, TickMs(0)),
        );
        assert_eq!(
            warrant,
            Some(OpenWarrant {
                target: true,
                basis: WarrantBasis::HeuristicGuess(HeuristicGuessSource::Guard(
                    ForceOnReason::BrokenAppBootstrap
                )),
            }),
            "明示意図が無ければ BrokenAppBootstrap も既定推測として発火する"
        );
    }

    #[test]
    fn requested_mismatch_returns_none() {
        // INV-20: basis が示す値と requested が食い違えば None。
        let store = IntentStore::default();
        let obs = ObservationStore::default();
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::TsfNative);
        let now = Instant::now();
        // desired_open=true な OwnSsot だが requested=false を尋ねている。
        let warrant = issue_open_warrant(
            false,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, true, true, now, TickMs(0)),
        );
        assert_eq!(warrant, None);
    }

    #[test]
    fn none_when_nothing_matches() {
        let store = IntentStore::default();
        let obs = ObservationStore::default();
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::ImmCross);
        let now = Instant::now();
        let warrant = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, false, true, now, TickMs(0)),
        );
        assert_eq!(
            warrant, None,
            "Read プロファイル・観測なし・意図なしでは何も根拠が無い"
        );
    }

    // ── now/now_ms が実際に効いていることの検証（ADR-087 §7 round4 M-B） ──

    #[test]
    fn now_argument_gates_step3_via_fresh_window() {
        let store = IntentStore::default();
        let mut obs = ObservationStore::default();
        let t0 = Instant::now();
        rec(
            &mut obs,
            obs_at(
                true,
                ObservationSource::ImmGetOpenStatus,
                ObservationConfidence::High,
                t0,
            ),
        );
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::ImmCross);

        let fresh = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, false, true, t0, TickMs(0)),
        );
        assert_eq!(
            fresh,
            Some(OpenWarrant {
                target: true,
                basis: WarrantBasis::DirectRead(ObservationSource::ImmGetOpenStatus),
            }),
            "FRESH ウィンドウ内では Step3 が発火する"
        );

        let stale = issue_open_warrant(
            true,
            TARGET,
            &ctx(
                &store,
                &obs,
                &guards,
                &policy,
                false,
                true,
                t0 + std::time::Duration::from_secs(4),
                TickMs(0),
            ),
        );
        assert_eq!(
            stale, None,
            "FRESH(3s) を超えた now を渡すと Step3 は素通りし、Read プロファイルは \
             Step4c(OwnSsot) も発火しないため None——now が本当に効いている証拠"
        );
    }

    #[test]
    fn now_ms_argument_gates_step1_via_intent_ttl() {
        let mut store = IntentStore::default();
        store.record(
            TARGET,
            true,
            crate::state::ime_event::UserIntentSource::PhysicalImeKey,
            TickMs(0),
        );
        let obs = ObservationStore::default();
        let guards = ForceGuardSet::default();
        let policy = AppImePolicy::from_profile(ImePolicyProfile::TsfNative);
        let now = Instant::now();
        let ttl = crate::tuning::EXPLICIT_ON_INTENT_TTL_MS;

        let within_ttl = issue_open_warrant(
            true,
            TARGET,
            &ctx(&store, &obs, &guards, &policy, true, true, now, TickMs(ttl)),
        );
        assert!(
            matches!(
                within_ttl,
                Some(OpenWarrant {
                    basis: WarrantBasis::ExplicitUserIntent(_),
                    ..
                })
            ),
            "TTL ちょうどはまだ Step1 の明示意図が有効"
        );

        let after_ttl = issue_open_warrant(
            true,
            TARGET,
            &ctx(
                &store,
                &obs,
                &guards,
                &policy,
                true,
                true,
                now,
                TickMs(ttl + 1),
            ),
        );
        assert_eq!(
            after_ttl,
            Some(OpenWarrant {
                target: true,
                basis: WarrantBasis::OwnSsot,
            }),
            "TTL 超過後は Step1 が失効し、Blind プロファイルの Step4c(OwnSsot) に \
             フォールバックする（desired_open=true をそのまま採用）——\
             now_ms が本当に効いている証拠"
        );
    }

    // ── 網羅的な組み合わせテスト ──────────────────────────────────────────────
    //
    // round1〜3 は「シナリオを手で選んで」テストしており、round2 の修正が
    // round3 で新たなバグを生む（選ばれなかった組み合わせで発生する）ことが
    // 繰り返された（M2 の Step 順序バグはまさにこの型）。
    //
    // `issue_open_warrant` の入力は実質すべて有限個の離散値の組み合わせ
    // なので、シナリオを選ばずに全組み合わせを生成し、個別の期待値ではなく
    // 「独立に書いたオラクル関数」の予測と突き合わせる。オラクルは
    // `issue_open_warrant` の実装をコピーせず、ADR §2.3 P15 の Step 0〜4 の
    // 仕様文をそのまま素直に書き下したもの——実装とオラクルが独立している
    // ことで、片方だけの取り違え（M2 のような Step 順序の逆転）を検出できる。

    /// Step 3 の観測パターン（オラクル用の簡略表現）。
    ///
    /// `MediumAgree`（2ソースが合意、`Corroborated` に対応）を round4 最終確認
    /// （Opus G1）で追加した。旧版は `MediumConflict`（矛盾）しか2ソース
    /// ケースを持たず、`Corroborated` 分岐がどの組み合わせからも到達
    /// されていなかった。
    #[derive(Debug, Clone, Copy)]
    enum Step3Case {
        None,
        High(bool),
        MediumSingle(bool),
        MediumAgree(bool),
        MediumConflict,
    }

    const ALL_STEP3_CASES: [Step3Case; 8] = [
        Step3Case::None,
        Step3Case::High(true),
        Step3Case::High(false),
        Step3Case::MediumSingle(true),
        Step3Case::MediumSingle(false),
        Step3Case::MediumAgree(true),
        Step3Case::MediumAgree(false),
        Step3Case::MediumConflict,
    ];

    /// `issue_open_warrant` の実装から独立して書いた、Step 0〜4 のオラクル。
    /// `WarrantBasis` の具体的な variant ではなく粗いカテゴリ名を返す
    /// （実装側は `category()` で同じ粒度に落とす）。`High`/`MediumSingle`/
    /// `MediumAgree` をそれぞれ別カテゴリにするのは round4 最終確認
    /// （Opus G1）——3つを1カテゴリに畳むと `SingleIndirect`/`Corroborated`
    /// への分岐（round4 S-B で新設）が網羅の外に置き去りになる。
    ///
    /// 時間軸は本テストでは凍結する（`now`/`now_ms` は全ケース共通の固定値）。
    /// FRESH ウィンドウ・TTL の境界は `now_argument_gates_step3_via_fresh_window`/
    /// `now_ms_argument_gates_step1_via_intent_ttl` 等の個別テストが担当する
    /// （round4 最終確認 Opus G3）。
    #[allow(clippy::fn_params_excessive_bools, clippy::too_many_arguments)]
    fn oracle(
        override_guard: bool,
        heuristic_guard: bool,
        intent: Option<bool>,
        step3: Step3Case,
        heuristic_default_obs: Option<bool>,
        feedback_blind: bool,
        desired_open: bool,
        is_japanese_ime: bool,
        requested: bool,
    ) -> Option<(bool, &'static str)> {
        // 最優先: 日本語 IME でなければ何があっても None（スコープ判定）。
        if !is_japanese_ime {
            return None;
        }
        // Step 0: 真の安全弁。他の全てより先に評価し、成立すれば以降を見ない。
        if override_guard {
            return if requested {
                Some((true, "SafetyValve"))
            } else {
                None
            };
        }
        // Step 1: 明示意図。
        if let Some(v) = intent {
            return if requested == v {
                Some((v, "ExplicitUserIntent"))
            } else {
                None
            };
        }
        // Step 3: Actuating な観測（矛盾していれば None のまま Step4 へ）。
        match step3 {
            Step3Case::High(v) => {
                return if requested == v {
                    Some((v, "DirectRead"))
                } else {
                    None
                };
            }
            Step3Case::MediumSingle(v) => {
                return if requested == v {
                    Some((v, "SingleIndirect"))
                } else {
                    None
                };
            }
            Step3Case::MediumAgree(v) => {
                return if requested == v {
                    Some((v, "Corroborated"))
                } else {
                    None
                };
            }
            Step3Case::None | Step3Case::MediumConflict => {}
        }
        // Step 4a: HeuristicDefault 観測。
        if let Some(v) = heuristic_default_obs {
            return if requested == v {
                Some((v, "HeuristicGuess"))
            } else {
                None
            };
        }
        // Step 4b: override 権限を持たないヒューリスティック guard（常に ON 側）。
        if heuristic_guard {
            return if requested {
                Some((true, "HeuristicGuess"))
            } else {
                None
            };
        }
        // Step 4c: OwnSsot（Blind のときのみ、desired_open を採用）。
        if feedback_blind {
            return if requested == desired_open {
                Some((desired_open, "OwnSsot"))
            } else {
                None
            };
        }
        None
    }

    /// 実際の `WarrantBasis` をオラクルと同じ粒度のカテゴリ名に落とす。
    const fn category(basis: &WarrantBasis) -> &'static str {
        match basis {
            WarrantBasis::ExplicitUserIntent(_) => "ExplicitUserIntent",
            WarrantBasis::SafetyValve(_) => "SafetyValve",
            WarrantBasis::DirectRead(_) => "DirectRead",
            WarrantBasis::SingleIndirect(_) => "SingleIndirect",
            WarrantBasis::Corroborated { .. } => "Corroborated",
            WarrantBasis::HeuristicGuess(_) => "HeuristicGuess",
            WarrantBasis::OwnSsot => "OwnSsot",
        }
    }

    #[test]
    fn exhaustive_step_priority_matches_independently_written_oracle() {
        use crate::state::ime_event::UserIntentSource;

        let mut checked = 0u32;
        let mut mismatches = Vec::new();

        for &override_guard in &[false, true] {
            for &heuristic_guard in &[false, true] {
                for &intent in &[None, Some(true), Some(false)] {
                    for &step3 in &ALL_STEP3_CASES {
                        for &heuristic_default_obs in &[None, Some(true), Some(false)] {
                            for &feedback_blind in &[false, true] {
                                for &desired_open in &[false, true] {
                                    for &is_japanese_ime in &[false, true] {
                                        for &requested in &[false, true] {
                                            checked += 1;

                                            // ── 実際の状態を組み立てる ──
                                            let mut guards = ForceGuardSet::default();
                                            if override_guard {
                                                guards.add(ForceGuard {
                                                    reason: ForceOnReason::PanicReset,
                                                    expires_at: None,
                                                    generation: 1,
                                                });
                                            }
                                            if heuristic_guard {
                                                guards.add(ForceGuard {
                                                    reason: ForceOnReason::BrokenAppBootstrap,
                                                    expires_at: None,
                                                    generation: 1,
                                                });
                                            }

                                            let mut store = IntentStore::default();
                                            if let Some(v) = intent {
                                                store.record(
                                                    TARGET,
                                                    v,
                                                    UserIntentSource::PhysicalImeKey,
                                                    TickMs(0),
                                                );
                                            }

                                            let now = Instant::now();
                                            let mut obs = ObservationStore::default();
                                            match step3 {
                                                Step3Case::None => {}
                                                Step3Case::High(v) => rec(
                                                    &mut obs,
                                                    obs_at(
                                                        v,
                                                        ObservationSource::ImmGetOpenStatus,
                                                        ObservationConfidence::High,
                                                        now,
                                                    ),
                                                ),
                                                Step3Case::MediumSingle(v) => rec(
                                                    &mut obs,
                                                    obs_at(
                                                        v,
                                                        ObservationSource::ObserverPoll,
                                                        ObservationConfidence::Medium,
                                                        now,
                                                    ),
                                                ),
                                                Step3Case::MediumAgree(v) => {
                                                    rec(
                                                        &mut obs,
                                                        obs_at(
                                                            v,
                                                            ObservationSource::ObserverPoll,
                                                            ObservationConfidence::Medium,
                                                            now,
                                                        ),
                                                    );
                                                    rec(
                                                        &mut obs,
                                                        obs_at(
                                                            v,
                                                            ObservationSource::Gji,
                                                            ObservationConfidence::Medium,
                                                            now,
                                                        ),
                                                    );
                                                }
                                                Step3Case::MediumConflict => {
                                                    rec(
                                                        &mut obs,
                                                        obs_at(
                                                            true,
                                                            ObservationSource::ObserverPoll,
                                                            ObservationConfidence::Medium,
                                                            now,
                                                        ),
                                                    );
                                                    rec(
                                                        &mut obs,
                                                        obs_at(
                                                            false,
                                                            ObservationSource::Gji,
                                                            ObservationConfidence::Medium,
                                                            now,
                                                        ),
                                                    );
                                                }
                                            }
                                            if let Some(v) = heuristic_default_obs {
                                                rec(
                                                    &mut obs,
                                                    obs_at(
                                                        v,
                                                        ObservationSource::HeuristicDefault,
                                                        ObservationConfidence::Low,
                                                        now,
                                                    ),
                                                );
                                            }

                                            let policy =
                                                AppImePolicy::from_profile(if feedback_blind {
                                                    ImePolicyProfile::TsfNative
                                                } else {
                                                    ImePolicyProfile::ImmCross
                                                });

                                            let warrant = issue_open_warrant(
                                                requested,
                                                TARGET,
                                                &ctx(
                                                    &store,
                                                    &obs,
                                                    &guards,
                                                    &policy,
                                                    desired_open,
                                                    is_japanese_ime,
                                                    now,
                                                    TickMs(0),
                                                ),
                                            );

                                            let expected = oracle(
                                                override_guard,
                                                heuristic_guard,
                                                intent,
                                                step3,
                                                heuristic_default_obs,
                                                feedback_blind,
                                                desired_open,
                                                is_japanese_ime,
                                                requested,
                                            );

                                            let actual = warrant
                                                .as_ref()
                                                .map(|w| (w.target, category(&w.basis)));

                                            if actual != expected {
                                                mismatches.push(format!(
                                                "override_guard={override_guard} heuristic_guard={heuristic_guard} \
                                                 intent={intent:?} step3={step3:?} heuristic_default_obs={heuristic_default_obs:?} \
                                                 feedback_blind={feedback_blind} desired_open={desired_open} \
                                                 is_japanese_ime={is_japanese_ime} requested={requested} \
                                                 → actual={actual:?} expected={expected:?}"
                                            ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        assert!(
            mismatches.is_empty(),
            "{}/{} combinations mismatched オラクルとの不一致 {} 件:\n{}",
            mismatches.len(),
            checked,
            mismatches.len(),
            mismatches[..mismatches.len().min(20)].join("\n")
        );
    }

    // ── 差分オラクル: 旧ゲート is_eligible_for_ime_force_on() vs 新 issue_open_warrant() ──
    //
    // ADR-087 §5 Phase 3 item15 は `state/platform_state.rs::is_eligible_for_ime_force_on()`
    // （`belief.is_japanese_ime() && effective_open()`）を `issue_open_warrant()` 経由に
    // 差し替える予定だが、実機なしでは「差し替えたときに実際に何が変わるか」を検証できない。
    // このテストは両者を同じ入力（explicit intent / observation / force guard / desired_open /
    // is_japanese_ime）に対して並べて評価し、不一致になる組合せを事前に数え上げて固定する
    // （Opus 相談: 「実差し替え時に実機で何が変わるかを事前に表化する」、ADR-081 の教訓＝
    // 並走配線ではなく純粋テストで parity を示す、の直接適用）。
    //
    // 完全一致を要求しない: 両者は構造的に異なる（新は explicit intent を IntentStore
    // 経由・BeliefOnly 権限の観測を除外・safety valve を intent より先に評価するが、
    // 旧は last_intent 経由・観測の権限を区別しない・guard override を intent の後に
    // 適用する）。不一致数を凍結し、新しい不一致パターンが増えたら気づけるようにする。
    //
    // `effective_open()`（belief.rs の `ImeBelief::is_japanese_ime()`）は `ImeStateHub`
    // 経由の値だが、実体は `is_japanese_ime: bool && ImeModel::effective_open_at(now)` の
    // 単純な AND なので、`ImeBelief` を経由せずこのテストでは直接 bool として与える。
    fn old_is_eligible_for_ime_force_on(
        model: &super::super::ime_model::ImeModel,
        is_japanese_ime: bool,
        now: Instant,
    ) -> bool {
        is_japanese_ime && model.effective_open_at(now)
    }

    #[derive(Debug, Clone, Copy)]
    enum ExplicitIntentDim {
        None,
        On,
        Off,
    }

    #[derive(Debug, Clone, Copy)]
    enum ObservationDim {
        None,
        ActuatingHigh(bool),
        // ConvOpenInference の唯一の本番生成点
        // (`platform_state.rs::report_conv_open_inference`) は必ず Medium
        // confidence（doc に明記）。BUG-63 の忠実な再現のため High ではなく
        // Medium を使う（2026-08-10 Opus レビュー S2）。
        BeliefOnlyMedium(bool),
    }

    #[derive(Debug, Clone, Copy)]
    enum GuardDim {
        Inactive,
        Override,
        HeuristicOnly,
    }

    #[derive(Debug, Clone, Copy)]
    enum PolicyDim {
        // default_feedback = Blind。Step4c(OwnSsot) が発火しうる。
        TsfNative,
        // default_feedback = Read。Step4c は発火しない
        // （`step4c_does_not_fire_for_read_profile` 参照）。旧ゲートの
        // `try_force_on_bootstrap` 呼び出し元はこちら側のプロファイルで
        // 到達する（`ir_poll_and_learn`／`OsPoll` 経由）。
        ImmCross,
    }

    #[test]
    fn differential_old_gate_vs_issue_open_warrant() {
        use super::super::ime_event::{EventTime, ImeEventEnvelope};
        use super::super::ime_model::ImeModel;
        use crate::state::ime_event::UserIntentSource;

        // 2026-08-10 時点の実測値。old/new の食い違いは方向で2種に分けて数える
        // （2026-08-10 Opus レビュー M3: 件数一致だけだと「old が誤って許可する
        // 不一致が1件減り、new が誤って許可する不一致が1件増える」変化を
        // 見逃す）。
        // - old_only（old=true, new=false）: new の方が厳格 = 安全側の差分。
        // - new_only（old=false, new=true）: new の方が緩い = Phase 3 実配線で
        //   新たに force-ON し始める可能性があるため要注意。
        //
        // old_only の内訳（本テストで実測、計8件）:
        // 1. `policy=ImmCross`・観測/意図/guard 一切無し・`desired_open=true`
        //    （`try_force_on_bootstrap` 相当、1件）: 旧は observation 皆無時に
        //    `most_recent_trusted` も外れて `desired_open` にフォールバックし
        //    true になるが、新は Read プロファイルのため Step4c(OwnSsot) が
        //    発火せず None——**Phase 3 実配線で ImmCross の bootstrap force-ON
        //    経路が丸ごと無効化される、今回判明した中で最大の挙動変化**
        //    （2026-08-10 Opus レビュー M2）。
        // 2. `guard=HeuristicOnly`（BrokenAppBootstrap）が実際の Actuating 観測
        //    （ImmGetOpenStatus=false）を無視して force-ON する（旧のみ、
        //    policy={TsfNative,ImmCross}×desired_open={false,true} の4件、
        //    guard の効果はどちらの policy でも変わらないため両方に出現）。
        //    新は Step3 の実観測が Step4b のヒューリスティック推測より優先
        //    されるため force-ON しない——安全側の差分。
        // 3. `observation=BeliefOnlyMedium(true)`（ConvOpenInference）が単独で
        //    force-ON eligibility を作る（旧のみ、3件: policy=TsfNative×
        //    desired_open=false、policy=ImmCross×desired_open={false,true}。
        //    TsfNative×desired_open=true は Step4c(OwnSsot) が同じ true を
        //    返すため偶然 old==new になり不一致に現れない）。**これが BUG-63
        //    （「mise」→「くした」誤入力）の再現そのもの**——新は authority
        //    フィルタでこの観測源を actuation の根拠から除外するため発火しない。
        //
        // new_only の内訳（本テストで実測、計1件）:
        // 4. `observation=BeliefOnlyMedium(false)`・intent/guard 無し・
        //    `policy=TsfNative`・`desired_open=true`: 旧は observation
        //    （ConvOpenInference=false）を採用し false になるが、新は Step3 で
        //    この観測源を除外した結果何も残らず Step4c(OwnSsot) が
        //    `desired_open=true` を採用する——Phase 3 実配線で「今まで
        //    force-ON しなかった状況で新たに force-ON し始める」ケース
        //    （`policy=ImmCross` の同条件は Step4c が発火しないため new=false
        //    のまま old と一致し、不一致に現れない）。
        //
        // Phase 3 実配線前にこの件数が増減したら、この変更が意図したものか
        // （新しい分岐を足した／既存の不一致を解消した）を確認した上で更新すること。
        const EXPECTED_OLD_ONLY_COUNT: usize = 8;
        const EXPECTED_NEW_ONLY_COUNT: usize = 1;

        const EXPLICIT_INTENTS: [ExplicitIntentDim; 3] = [
            ExplicitIntentDim::None,
            ExplicitIntentDim::On,
            ExplicitIntentDim::Off,
        ];
        const OBSERVATIONS: [ObservationDim; 5] = [
            ObservationDim::None,
            ObservationDim::ActuatingHigh(true),
            ObservationDim::ActuatingHigh(false),
            ObservationDim::BeliefOnlyMedium(true),
            ObservationDim::BeliefOnlyMedium(false),
        ];
        const GUARDS: [GuardDim; 3] = [
            GuardDim::Inactive,
            GuardDim::Override,
            GuardDim::HeuristicOnly,
        ];
        const POLICIES: [PolicyDim; 2] = [PolicyDim::TsfNative, PolicyDim::ImmCross];

        let now = Instant::now();
        let mut old_only: Vec<String> = Vec::new();
        let mut new_only: Vec<String> = Vec::new();
        let mut checked = 0usize;

        for &is_japanese_ime in &[false, true] {
            for &intent in &EXPLICIT_INTENTS {
                for &observation in &OBSERVATIONS {
                    for &guard in &GUARDS {
                        for &policy_dim in &POLICIES {
                            for &desired_open in &[false, true] {
                                // intent が Some の場合、本番の reducer
                                // (UserImeSetIntent) は last_intent と desired_open
                                // を必ず同じ値に揃えて書き込む（ime_model.rs::reduce
                                // 参照）。desired_open を独立変数のまま残すと本番では
                                // 起こり得ない組合せ（last_intent=ON なのに
                                // desired_open=false）を作ってしまい、old 側の
                                // `effective_open_at`（has_explicit_intent 時は
                                // base=desired_open を採用）が誤って偽の不一致を生む。
                                // intent!=None のときは desired_open==intent の
                                // 1通りだけ調べる。
                                let intent_bool = match intent {
                                    ExplicitIntentDim::None => None,
                                    ExplicitIntentDim::On => Some(true),
                                    ExplicitIntentDim::Off => Some(false),
                                };
                                if let Some(v) = intent_bool {
                                    if desired_open != v {
                                        continue;
                                    }
                                }
                                checked += 1;

                                let mut model = ImeModel::new();
                                let mut store = IntentStore::default();
                                match intent_bool {
                                    None => model.set_desired_open_for_test(desired_open),
                                    Some(v) => {
                                        // 本番と同じ経路 (UserImeSetIntent) で
                                        // desired_open と last_intent を同時に揃える。
                                        model.reduce(&ImeEventEnvelope {
                                            time: EventTime {
                                                seq: 0,
                                                monotonic: now,
                                                tick_ms: 0,
                                            },
                                            event:
                                                super::super::ime_event::ImeEvent::UserImeSetIntent {
                                                    target: v,
                                                    source: UserIntentSource::PhysicalImeKey,
                                                },
                                        });
                                        store.record(
                                            TARGET,
                                            v,
                                            UserIntentSource::PhysicalImeKey,
                                            TickMs(0),
                                        );
                                    }
                                }

                                let mut obs = ObservationStore::default();
                                match observation {
                                    ObservationDim::None => {}
                                    ObservationDim::ActuatingHigh(open) => {
                                        let o = obs_at(
                                            open,
                                            ObservationSource::ImmGetOpenStatus,
                                            ObservationConfidence::High,
                                            now,
                                        );
                                        rec(&mut model.observations, o);
                                        rec(&mut obs, o);
                                    }
                                    ObservationDim::BeliefOnlyMedium(open) => {
                                        // ConvOpenInference: BUG-63(mise→くした) の
                                        // 直接原因になった BeliefOnly 権限の観測源。
                                        // 旧ゲートはこれを区別せず
                                        // derive_open_filtered(|_| true) に含めるが、
                                        // 新は Step3 で除外する
                                        // （authority()==Actuating のみ）。
                                        let o = obs_at(
                                            open,
                                            ObservationSource::ConvOpenInference,
                                            ObservationConfidence::Medium,
                                            now,
                                        );
                                        rec(&mut model.observations, o);
                                        rec(&mut obs, o);
                                    }
                                }

                                let mut guards_new = ForceGuardSet::default();
                                match guard {
                                    GuardDim::Inactive => {}
                                    GuardDim::Override => {
                                        model.force_guards.add(ForceGuard {
                                            reason: ForceOnReason::PanicReset,
                                            expires_at: None,
                                            generation: 1,
                                        });
                                        guards_new.add(ForceGuard {
                                            reason: ForceOnReason::PanicReset,
                                            expires_at: None,
                                            generation: 1,
                                        });
                                    }
                                    GuardDim::HeuristicOnly => {
                                        model.force_guards.add(ForceGuard {
                                            reason: ForceOnReason::BrokenAppBootstrap,
                                            expires_at: None,
                                            generation: 1,
                                        });
                                        guards_new.add(ForceGuard {
                                            reason: ForceOnReason::BrokenAppBootstrap,
                                            expires_at: None,
                                            generation: 1,
                                        });
                                    }
                                }

                                let policy_profile = match policy_dim {
                                    PolicyDim::TsfNative => ImePolicyProfile::TsfNative,
                                    PolicyDim::ImmCross => ImePolicyProfile::ImmCross,
                                };
                                let policy = AppImePolicy::from_profile(policy_profile);

                                let old =
                                    old_is_eligible_for_ime_force_on(&model, is_japanese_ime, now);
                                let new = issue_open_warrant(
                                    true,
                                    TARGET,
                                    &ctx(
                                        &store,
                                        &obs,
                                        &guards_new,
                                        &policy,
                                        desired_open,
                                        is_japanese_ime,
                                        now,
                                        TickMs(0),
                                    ),
                                )
                                .is_some();

                                if old && !new {
                                    old_only.push(format!(
                                        "is_japanese_ime={is_japanese_ime} intent={intent:?} \
                                         observation={observation:?} guard={guard:?} \
                                         policy={policy_dim:?} desired_open={desired_open} \
                                         → old={old} new={new}"
                                    ));
                                } else if !old && new {
                                    new_only.push(format!(
                                        "is_japanese_ime={is_japanese_ime} intent={intent:?} \
                                         observation={observation:?} guard={guard:?} \
                                         policy={policy_dim:?} desired_open={desired_open} \
                                         → old={old} new={new}"
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        assert_eq!(
            old_only.len(),
            EXPECTED_OLD_ONLY_COUNT,
            "{}/{checked} combinations, old-only 不一致数が想定({EXPECTED_OLD_ONLY_COUNT})と\
             異なります（old=true,new=false — new の方が厳格）。Phase 3 で\
             is_eligible_for_ime_force_on() を issue_open_warrant() に差し替える前に、\
             この差分が既知のものか確認してください:\n{}",
            old_only.len(),
            old_only.join("\n")
        );
        assert_eq!(
            new_only.len(),
            EXPECTED_NEW_ONLY_COUNT,
            "{}/{checked} combinations, new-only 不一致数が想定({EXPECTED_NEW_ONLY_COUNT})と\
             異なります（old=false,new=true — new の方が緩い。Phase 3 実配線で新たに\
             force-ON し始める可能性があるため要注意）:\n{}",
            new_only.len(),
            new_only.join("\n")
        );
    }
}
