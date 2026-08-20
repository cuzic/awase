//! IME 状態モデル (Step 1: Shadow Reducer 段階)
//!
//! 既存の `ImeBelief` / `ImeObservations` と並走する shadow model。
//! 現状 (Step 1) は本番判定には使わず、diff log で検証するのみ。
//!
//! ## 設計原則
//!
//! 1. **UserIntent だけが `desired_open` を即時に変えられる**
//! 2. **Observer は `desired_open` を直接壊せない** (last_observed に記録するのみ)
//! 3. **AppImePolicy / InputBarrier / ForceGuardSet は後続 Step で追加** (Step 1 では placeholder)

use super::app_ime_policy::AppImePolicy;
use super::force_guard::{ForceGuardSet, ForceOnReason, ObserveMissMonitor};
use awase::engine::InputModeState;

use super::ime_event::{
    ChordKind, HwndId, ImeEvent, ImeEventEnvelope, InputModeApplyResult, ObservationConfidence,
    ObservationSource, UserIntentSource,
};
use super::input_barrier::InputBarrier;
use super::observation_store::{DeriveOutcome, ObservationStore};
use super::transition::ImeTransition;
use std::time::Instant;

// ── resolve_open_at 診断API（ADR-087 §5 Phase 0a item2/3） ──────────────────────

/// `resolve_open_at()` の戻り値。`effective_open_at()` が返す `bool` に加えて、
/// 「なぜその値になったか」を保持する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenResolution {
    pub value: bool,
    pub decided_by: DecidedBy,
}

/// `effective_open_at()` の判定内訳。
///
/// `base`（明示意図/観測/フォールバックのどれで決まったか）と
/// `guard_override`（`force_guards` が override したか）を分けて持つ——
/// `ImeModel::effective_open()` の実装が
/// `force_guards.effective_open(base, has_explicit_intent)` という2段構造に
/// なっているため（`ime_model.rs` 本体参照）、診断もそれに合わせる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecidedBy {
    pub base: BaseDecision,
    pub guard_override: Option<ForceOnReason>,
}

/// `base`（`force_guards` 適用前の値）がどの経路で決まったか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseDecision {
    /// `has_user_explicit_intent()==true`、`desired_open` を採用。
    ExplicitIntent,
    /// `derive_any()` / `derive_actuating()` が High confidence 単独ソースで確定。
    DeriveHigh(ObservationSource),
    /// `derive_any()` / `derive_actuating()` が Medium+ の無競合多数決で確定。
    /// `second` が `None` なら単独観測、`Some` なら2ソース以上の合意
    /// （`DeriveOutcome::MediumConsensus` と同じ理由で `Vec` を避ける、
    /// ADR-087 §7 round4 S-A: `effective_open()` は全 `KeyDown` で呼ばれる
    /// ホットパスのため）。
    DeriveMedium {
        first: ObservationSource,
        second: Option<ObservationSource>,
    },
    /// `derive_any()` が `None`（観測なし/矛盾）で `most_recent_trusted()` に
    /// フォールバック。
    MostRecentTrusted(ObservationSource),
    /// 観測が一切なく `desired_open` にフォールバック。
    DesiredFallback,
}

// ── AppliedImeState ──────────────────────────────────────────────────────────

/// IME apply 結果の確信度。
///
/// `Option<(bool, u64)>` + センチネル値 `ts=0` で表現していた3状態を型で明示する。
/// - `Unknown`   : フォーカス直後・起動時。実 IME 状態が不明。
/// - `Optimistic`: ImmCross async の楽観的事前更新。OS 未確認。
/// - `Confirmed` : 実 apply 完了・確認済み。旧 `applied_at_ms > 0` に相当。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppliedImeState {
    #[default]
    Unknown,
    Optimistic(bool),
    Confirmed {
        open: bool,
        at_ms: u64,
    },
}

impl AppliedImeState {
    /// `build_ime_control_view` 互換の `Option<(bool, u64)>` に変換する。
    #[must_use]
    pub const fn to_pair(self) -> Option<(bool, u64)> {
        match self {
            Self::Unknown => None,
            Self::Optimistic(open) => Some((open, 0)),
            Self::Confirmed { open, at_ms } => Some((open, at_ms)),
        }
    }

    /// apply 済みの open 値を返す（Optimistic も含む）。Unknown は None。
    #[must_use]
    pub const fn applied_open(self) -> Option<bool> {
        match self {
            Self::Unknown => None,
            Self::Optimistic(open) | Self::Confirmed { open, .. } => Some(open),
        }
    }

    /// 確認済み (`Confirmed`) かどうか。
    #[must_use]
    pub const fn is_confirmed(self) -> bool {
        matches!(self, Self::Confirmed { .. })
    }

    /// `Confirmed { open, at_ms }` の `at_ms` を返す。それ以外は 0。
    #[must_use]
    pub const fn confirmed_at_ms(self) -> u64 {
        match self {
            Self::Confirmed { at_ms, .. } => at_ms,
            _ => 0,
        }
    }
}

/// Shadow IME モデル。最終形 (Phase 3 完了時) ではこれが SSOT になる予定。
///
/// Step 3 時点: desired_open + last_intent + observations (per-source + drift) + policy。
/// pending transition / barrier / force guard は後続 Step で追加。
#[derive(Debug)]
pub struct ImeModel {
    /// awase が IME をこうしたい状態。UserIntent のみが書き換える。
    ///
    /// private フィールド。`reduce()` 以外からの書き込みを禁止するため、
    /// 外部からは読み取り専用アクセサ `desired_open()` を使うこと
    /// （`input_mode` と同じパターン）。
    desired_open: bool,

    /// 入力モード（ローマ字/かな/英数/不明）の belief。
    ///
    /// H-3-b で追加。H-3-c で `ImeBelief::input_mode` への直接代入が
    /// `InputModeObserved` / `InputModeApplied` / `UserChangedInputMode` イベント経由に
    /// 置換されるまでは shadow として記録するのみで本番判定には使わない。
    /// H-3-d で `ImeBelief::input_mode` が private 化されたのち、このフィールドが SSOT になる。
    ///
    /// private フィールド。外部からは読み取り専用アクセサ `input_mode()` を使うこと。
    input_mode: InputModeState,

    /// 直近のユーザー意図 (intent guard 等の判断材料)
    pub last_intent: Option<RecordedIntent>,

    /// 観測値ストア (Step 3) — per-source + suspicious + drift。
    /// reducer の judge 材料: 鮮度・合意・乖離継続時間。
    pub observations: ObservationStore,

    /// 現フォーカスアプリの IME 制御ポリシー (Step 1.5)。
    /// FocusChanged event で更新される。
    pub app_policy: AppImePolicy,

    /// 入力 chord 等の一時 transaction (Step 4)。
    /// 旧 `ctrl_bypass_hold: bool` の置換。
    pub input_barrier: Option<InputBarrier>,

    /// 発火後の force-on ガード集合 (Step 6)。
    /// 旧 `ImeRecoveryState::force_on_*` 2 つの bool を `ForceGuardSet` に統合。
    pub force_guards: ForceGuardSet,

    /// 発火前の観測失敗カウンタ (Step 6)。
    /// 旧 `ImeRecoveryState::ime_detect_miss_count` の責務分離。
    pub observe_miss_monitor: ObserveMissMonitor,

    /// OS への apply 進行中の transition (Step 7)。
    /// 旧 `ImeEffect::SetOpen` (Layer 3) + 楽観的 latch を統合。
    pub pending: Option<ImeTransition>,

    /// 最後に actuator が成功させた IME 開閉状態の確信度 (Step 7)。
    /// 旧 `applied_open: Option<bool>` + `applied_at_ms: u64` の置換。
    pub applied: AppliedImeState,

    /// 現在フォーカス中のウィンドウ (ADR-087 §5 Phase 3 item15 前提配線)。
    ///
    /// `FocusChanged` の reducer でのみ更新する。`current_focus()` アクセサ経由で
    /// `ImeStateHub::effective_open()`/`record_explicit_intent()`/
    /// `apply_hwnd_cache_restore()`/`reset_stale_ime_on_for_imm_broken()`
    /// （BUG-51 追補 v3、IntentStore の対象キーとして）が本番判定に使用する
    /// （`WarrantContext.target` 用の `issue_open_warrant()` への実配線は
    /// 依然 Phase 3 本体のスコープ）。
    current_focus: Option<HwndId>,
}

#[derive(Debug, Clone)]
pub struct RecordedIntent {
    pub target: bool,
    pub source: UserIntentSource,
    pub at_ms: u64,
}

impl ImeModel {
    /// 既存 `ImeBelief` の初期値 (`ime_on=true`) に合わせる。
    #[must_use]
    pub fn new() -> Self {
        Self {
            desired_open: true,
            input_mode: InputModeState::ObservedRomaji, // ImeBelief 初期値に合わせる
            last_intent: None,
            observations: ObservationStore::default(),
            app_policy: AppImePolicy::standard(),
            input_barrier: None,
            force_guards: ForceGuardSet::default(),
            observe_miss_monitor: ObserveMissMonitor::default(),
            pending: None,
            applied: AppliedImeState::Unknown,
            current_focus: None,
        }
    }

    /// 現在フォーカス中のウィンドウ（読み取り専用アクセサ）。
    ///
    /// `FocusChanged` の reducer でのみ更新される。まだ本番判定には使われない
    /// write-only なフィールドの読み取り口（ADR-087 §5 Phase 3 item15 前提配線）。
    #[must_use]
    pub const fn current_focus(&self) -> Option<HwndId> {
        self.current_focus
    }

    /// awase が IME をこうしたい状態（読み取り専用アクセサ）。
    ///
    /// `desired_open` フィールドは private。外部から書き込まず
    /// `ImeEvent::UserImeSetIntent` / `UserImeToggleIntent` 経由で reducer を通すこと。
    /// 実効値が欲しい場合は `effective_open()` を使うこと（こちらは生の意図のみ）。
    #[must_use]
    pub const fn desired_open(&self) -> bool {
        self.desired_open
    }

    /// 入力モードの belief を返す（読み取り専用アクセサ）。
    ///
    /// `input_mode` フィールドは private。外部から書き込まず
    /// `InputModeObserved` / `InputModeApplied` / `UserChangedInputMode` 経由で
    /// reducer を通すこと。
    #[must_use]
    pub const fn input_mode(&self) -> InputModeState {
        self.input_mode
    }

    /// テスト専用: `desired_open` を直接設定する。
    ///
    /// carry-over シナリオ（focus 変更前の stale な desired_open）をテストで
    /// 模擬するための脱出口。本番コードから呼んではならない。
    #[cfg(test)]
    pub(crate) fn set_desired_open_for_test(&mut self, value: bool) {
        self.desired_open = value;
    }

    /// 現在 CtrlImeChord transaction が active か。
    /// `stage_post_decision` が二次 SetOpen を filter するかどうかの判断材料。
    #[must_use]
    pub const fn is_ctrl_ime_chord_active(&self) -> bool {
        matches!(self.input_barrier, Some(InputBarrier::CtrlImeChord { .. }))
    }

    /// ユーザー/awase の明示的な意図が present かどうか。
    ///
    /// true の場合は `desired_open` を観測より優先する。
    /// false の場合は observation pool の `derive_any()` 結果を採用し、
    /// 観測が空なら `desired_open` にフォールバックする。
    ///
    /// `last_intent` は `UserImeSetIntent` / `UserImeToggleIntent` のみが設定する。
    /// `PanicReset` / `HwndCacheRestored` は設定しないため、ここで除外不要。
    fn has_user_explicit_intent(&self) -> bool {
        self.last_intent.is_some()
    }

    /// 観測プールと `desired_open` を統合した最終 belief (Step 6)。
    ///
    /// **これは belief（間違っていても低リスクな推定）であり、engine の内部挙動
    /// 決定用。実際に OS の IME を操作してよいかという actuation の根拠には
    /// 使わないこと**（ADR-087 §5 Phase 3 item17）。`derive_any()` の
    /// Medium 単一ソース合意がそのまま actuation の根拠として使われたことが
    /// BUG-63（「mise」→「くした」誤入力）の直接原因だった。actuation
    /// warrant が必要な場面（IME を実際に force-ON する等）では
    /// `crate::state::open_warrant::issue_open_warrant()` を使うこと
    /// （Phase 3 で `is_eligible_for_ime_force_on()` 等の既存呼び出し元を
    /// 順次差し替える予定、まだ未配線）。
    ///
    /// - ユーザーの明示意図がある場合: `desired_open` を優先（観測で上書きしない）
    /// - 明示意図なし（フォーカス変化直後等）:
    ///   1. `derive_any()`（Medium+ の合意 / High 即採用）の結果を採用
    ///   2. それが `None` なら `most_recent_trusted()`（confidence 不問、最新優先）
    ///      にフォールバック。cache-miss 等の安全デフォルト推測（Low confidence の
    ///      `HeuristicDefault`）はここでのみ効き、後から届いた実観測（Lowでも）が
    ///      新しければそちらが優先される。
    ///   3. 観測が一切なければ `desired_open` にフォールバック
    /// - 最後に `force_guards` を適用（guard が active なら強制 ON。ただし
    ///   `BrokenAppBootstrap` 等のヒューリスティック由来 guard はユーザーの明示的意図を
    ///   上書きしない。`PanicReset` 等の安全弁は明示的意図があっても override する）
    #[must_use]
    pub fn effective_open(&self) -> bool {
        self.effective_open_at(Instant::now())
    }

    /// `effective_open()` の `Instant` 引数化版。ADR-087 §5 Phase 0a item2
    /// （INV-23: 根拠判定の決定論性）。`effective_open()` はこれの薄い
    /// ラッパーであり、`Instant::now()` を呼ぶのはこの1箇所（`effective_open()`
    /// 自身）に限定される——`effective_open_at` 自体は時刻を内部で確定させない
    /// 純粋関数なので、journal replay やテストで決定論的に呼び出せる。
    #[must_use]
    pub fn effective_open_at(&self, now: Instant) -> bool {
        self.resolve_open_at(now).value
    }

    /// `effective_open_at()` の判定内訳まで返す診断 API（ADR-087 §5 Phase 0a item3）。
    ///
    /// 「なぜこの値になったか」（`DecidedBy`）を journal / テストに残せるようにする。
    /// 本バグ（`mise`→「くした」）は `effective_open()` が単一の bool しか返さず
    /// 判定根拠が失われていたために原因追跡に時間がかかった——この API はその
    /// 反省から追加する。
    #[must_use]
    pub fn resolve_open_at(&self, now: Instant) -> OpenResolution {
        let has_explicit_intent = self.has_user_explicit_intent();
        let (base, decided_by) = if has_explicit_intent {
            (self.desired_open, BaseDecision::ExplicitIntent)
        } else if let Some(outcome) = self.observations.derive_any(now) {
            let decided_by = match outcome {
                DeriveOutcome::HighSingle { source, .. } => BaseDecision::DeriveHigh(source),
                DeriveOutcome::MediumConsensus { first, second, .. } => {
                    BaseDecision::DeriveMedium { first, second }
                }
            };
            (outcome.value(), decided_by)
        } else if let Some(trusted) = self.observations.most_recent_trusted(now) {
            (
                trusted.open,
                BaseDecision::MostRecentTrusted(trusted.source),
            )
        } else {
            (self.desired_open, BaseDecision::DesiredFallback)
        };
        // `ForceGuardSet::resolve()` を唯一の判定点として使う（ADR-087 §7 round4
        // M-C: 述語を手書きで複製すると「guard が active なだけで override して
        // いない」場合にも reason を報告してしまう誤情報バグを生む。resolve() は
        // 実際に値を変えた場合のみ Some を返す）。
        let (value, guard_override) = self.force_guards.resolve(base, has_explicit_intent);
        OpenResolution {
            value,
            decided_by: DecidedBy {
                base: decided_by,
                guard_override,
            },
        }
    }

    /// `AppliedImeState` を返す。executor の applied_snapshot 同期用。
    #[must_use]
    pub const fn applied_state(&self) -> AppliedImeState {
        self.applied
    }

    /// `build_ime_control_view` 互換の `Option<(bool, u64)>` を返す。
    #[must_use]
    pub const fn applied_pair(&self) -> Option<(bool, u64)> {
        self.applied.to_pair()
    }

    /// `pending` transition の generation を返す。apply 完了 event の照合用。
    #[must_use]
    pub fn pending_generation(&self) -> Option<u64> {
        self.pending.as_ref().map(|p| p.generation)
    }

    /// 現在の `input_barrier` が持つ chord kind を返す。
    #[must_use]
    pub fn active_chord_kind(&self) -> Option<ChordKind> {
        self.input_barrier.and_then(|b| b.chord_kind())
    }

    /// フォーカス切替直後の one-shot barrier が pending かどうか。
    #[must_use]
    pub fn is_focus_transition_pending(&self) -> bool {
        self.input_barrier
            .as_ref()
            .is_some_and(InputBarrier::is_focus_transition)
    }

    /// フォーカス切替直後の settle 期間内（`settle_until` 未経過）かどうか。
    ///
    /// `is_focus_transition_pending` と異なり、barrier がまだ consume されていなくても
    /// `settle_until` を過ぎていれば false を返す。Engine 由来の `SetOpen` 効果適用を
    /// 一時的にフィルタするための判断に使う（`handle_engine_set_open` 参照）。
    #[must_use]
    pub fn is_focus_transition_settling(&self, now: Instant) -> bool {
        self.input_barrier
            .as_ref()
            .is_some_and(|b| b.is_focus_transition_active(now))
    }
}

impl Default for ImeModel {
    fn default() -> Self {
        Self::new()
    }
}

impl ImeModel {
    /// Event を反映する。
    ///
    /// **UserIntent だけが `desired_open` を即時に変えられる**。
    /// Observer は `observations` に記録するだけで desired を壊さない。
    pub fn reduce(&mut self, envelope: &ImeEventEnvelope) {
        // BUG-34 横展開 D-prep: pending transition の期限切れを毎 dispatch で
        // 遅延パージする。`ImeTransition.timeout_at` は Step 7 導入時から存在したが
        // 呼び出し元がゼロで、実際には一度も評価されていなかった。パージが無いと、
        // 完了イベント（generation 照合）が届かないまま pending が残留した場合
        // （典型例: `ImeOpenOutcome::UnsafeToToggle` を早期 return で捨てていた旧経路）
        // 以後の別 generation の完了が全て stale 判定され続ける固着になる。
        if let Some(pending) = &self.pending {
            if pending.is_timed_out(envelope.time.monotonic) {
                log::debug!(
                    "[ime-model] pending transition timed out (generation={}, target={}) — purge",
                    pending.generation,
                    pending.target
                );
                self.pending = None;
            }
        }
        match envelope.event {
            ImeEvent::UserImeToggleIntent { source } => {
                let target = !self.desired_open;
                self.desired_open = target;
                self.last_intent = Some(RecordedIntent {
                    target,
                    source,
                    at_ms: envelope.time.tick_ms,
                });
            }
            ImeEvent::UserImeSetIntent { target, source } => {
                self.desired_open = target;
                self.last_intent = Some(RecordedIntent {
                    target,
                    source,
                    at_ms: envelope.time.tick_ms,
                });
            }
            ImeEvent::PanicReset { target } => {
                // 復旧操作: desired_open を安全デフォルト値に戻す。
                // UserImeSetIntent と異なり last_intent を設定しない。
                // ForceGuard::PanicReset が IME ON を保証するため、
                // has_user_explicit_intent() を汚染しない。
                self.desired_open = target;
            }
            ImeEvent::HwndCacheRestored { target } => {
                // HWND キャッシュ復元: 前回フォーカス時の desired_open を回復する。
                // ユーザーの能動的操作ではないため last_intent を設定しない。
                // has_user_explicit_intent() が false のまま維持され、
                // 後続の実観測が effective_open() を上書きできる。
                self.desired_open = target;
            }
            ImeEvent::EngineActivationSync { target: _target } => {
                // Engine の active/inactive 遷移が対称性のために自動発行した echo。
                // `last_intent` はもちろん `desired_open` も一切書き換えない
                // （`event_log`/`journal` への記録は `dispatch_event` が無条件に行うため、
                // 「何が起きたか」の記録自体は失われない）。
                //
                // `desired_open` を書かない理由（Opus レビュー 2026-08-04 で指摘、
                // 当初は `has_user_explicit_intent()==false` の間だけ書いていた）:
                // `target` は `ctx.ime_on`（≒その時点の `effective_open()`）由来であり、
                // explicit intent が無い状況では `effective_open()` 自体が観測プール
                // (`derive_any()`) から計算されている。そこへ `desired_open := target`
                // を書くと `desired_open := effective_open()` という循環 echo になり、
                // 元になった観測が期限切れで消えた後も `effective_open()` の
                // フォールバック (`unwrap_or(self.desired_open)`) がこの値を恒久化して
                // しまう（一度もユーザー操作が無いのに、ノイズ観測 1 発が焼き付く）。
                // `desired_open` は `UserImeSetIntent`/`PanicReset`/`HwndCacheRestored`
                // という「値を確定させる」ための専用イベントにのみ任せ、この echo
                // イベントは純粋に「Engine 側で何が起きたか」の記録に徹する。
            }
            ImeEvent::ObserverReported(observed) => {
                // 絶対ルール: Observer は desired_open を直接書き換えない。
                // 値としての観測を記録する唯一の口（ADR-089 §2.1）。
                self.observations
                    .record_replayed(observed, envelope.time.monotonic);
                // drift 追跡 (desired と observed の乖離)
                self.observations.update_drift(
                    self.desired_open,
                    observed.open(),
                    envelope.time.monotonic,
                );
            }
            ImeEvent::FocusChanged {
                profile,
                to,
                focus_epoch,
                ..
            } => {
                // Step 1.5/5: policy 確定 → observation 評価の順序ルール。
                // FocusChanged を受けた時点で policy を更新し、以降の observation は
                // 新しい policy で評価される。
                self.app_policy = AppImePolicy::from_profile(profile);
                // current_focus: write-only（ADR-087 §5 Phase 3 item15 前提配線、
                // read 側は Phase 3 本体のスコープでまだ無い）。
                self.current_focus = Some(to);
                // フォーカス変更で intent / observation / applied / force_guard / drift は clear する
                // (旧アプリの観測値が新アプリで有効と勘違いされないため)
                self.last_intent = None;
                // 新しい epoch を store に伝える。derive_any() はこれ以降、
                // 古い epoch の ImmCrossProbe / FocusProbe を無視する。
                self.observations.clear_on_focus_change(focus_epoch);
                log::debug!("[explicit-intent] cleared (focus change)");
                self.applied = AppliedImeState::Unknown;
                // force_guard: 旧アプリ文脈の guard を新アプリに引き継がない
                self.force_guards.clear_for_focus_change();
                // observe_miss_monitor: 旧アプリの miss_count が新アプリで閾値を誤超えしないようリセット
                self.observe_miss_monitor.record_success();
                // Step 5: FocusTransition barrier を立てる (旧 focus_transition_pending 相当)。
                // settle_until は AppImePolicy.focus_settle_ms 由来。
                let settle_until = envelope.time.monotonic
                    + std::time::Duration::from_millis(self.app_policy.focus_settle_ms);
                self.input_barrier = Some(InputBarrier::FocusTransition {
                    to_hwnd: to,
                    started_seq: envelope.time.seq,
                    started_at: envelope.time.monotonic,
                    settle_until,
                });
            }
            ImeEvent::ChordEnded { .. } => {
                // Step 4: chord transaction を終了。barrier を解除。
                self.input_barrier = None;
            }
            ImeEvent::ImeApplyRequested {
                target,
                generation,
                ctrl_held,
            } => {
                // BUG-34 横展開 D-prep: 進行中の未期限切れ pending を無条件で
                // 上書きしない。上書きすると、進行中の別 apply（例: 打鍵駆動の
                // apply）の完了が届いたときに generation 不一致で stale 判定され、
                // その apply の結果が黙って捨てられる（`applied_open` が古いまま
                // 固定され drift correction が再送を繰り返す事故につながる）。
                // 【現時点では拒否ではなく警告ログのみ】正当な高頻度連続要求
                // （例: force-on の即時リトライ）を誤って落とさないため、実機で
                // 実際の発生頻度を確認してから拒否に倒すかどうかを判断する。
                if let Some(existing) = &self.pending {
                    if !existing.is_timed_out(envelope.time.monotonic) {
                        log::warn!(
                            "[ime-model] ImeApplyRequested(generation={generation}, target={target}) \
                             が進行中の pending(generation={}, target={}) を上書きする — \
                             その apply の完了が今後 stale 判定される可能性がある",
                            existing.generation,
                            existing.target
                        );
                    }
                }
                // Step 7: pending transition を立てる。
                // 実際の timeout / actuator 詳細は呼び出し元 (Phase 3 cleanup) が
                // 個別 dispatch で渡す想定。Step 7 では最低限の placeholder。
                self.pending = Some(ImeTransition {
                    target,
                    generation,
                    timeout_at: envelope.time.monotonic
                        + std::time::Duration::from_millis(
                            crate::tuning::IME_APPLY_PENDING_TIMEOUT_MS,
                        ),
                });
                // Chord 開始判断: IME OFF 要求 + Ctrl 押下中 → CtrlImeChord barrier を立てる。
                // KANJI（Ctrl なし）では立てない: ChordEnded のトリガが Ctrl KeyUp なので
                // ペアにならず永続する事故を防ぐ。
                if !target && ctrl_held {
                    self.input_barrier = Some(InputBarrier::CtrlImeChord {
                        target: false,
                        kind: ChordKind::CtrlMuhenkanImeOff,
                        started_seq: envelope.time.seq,
                        started_at: envelope.time.monotonic,
                    });
                }
                // Chord 中に IME ON 要求が来た場合 → chord を即時終了する。
                if target && self.is_ctrl_ime_chord_active() {
                    self.input_barrier = None;
                }
            }
            ImeEvent::ImeApplySucceeded { target, generation } => {
                // Step 7: **必須** generation 照合で stale apply を排除。
                // pending の generation と一致しなければ無視する。
                if self.pending.as_ref().map(|p| p.generation) == Some(generation) {
                    self.applied = AppliedImeState::Confirmed {
                        open: target,
                        at_ms: envelope.time.tick_ms,
                    };
                    self.pending = None;
                }
                // 一致しない場合は何もしない (stale → 無視)
            }
            ImeEvent::ImeApplyFailed { generation, .. } => {
                // 同じく generation 照合
                if self.pending.as_ref().map(|p| p.generation) == Some(generation) {
                    self.pending = None;
                }
            }
            ImeEvent::DriftDetected { desired, .. } => {
                // skip_override を無効化する: Optimistic にリセットすることで
                // 次の SetOpen(desired) が「確認済み apply がない」扱いになり skip されなくなる。
                // applied は desired に合わせて楽観的にセット（ImmCross async 送信と同じ扱い）。
                self.applied = AppliedImeState::Optimistic(desired);
            }
            ImeEvent::InputModeObserved {
                mode, confidence, ..
            } => {
                // ON/OFF の derive_any() と同じ考え方: Low confidence 単独では
                // belief を動かさない（記録のみ）。Medium+ のみ input_mode を上書きする。
                if confidence >= ObservationConfidence::Medium {
                    self.input_mode = mode;
                } else {
                    log::debug!(
                        "[input-mode] Low confidence observation 無視: {mode:?} (confidence={confidence:?})"
                    );
                }
            }
            ImeEvent::InputModeApplied { mode, result, .. } => {
                // Skipped の場合はモード変更が起きていないため更新しない。
                if result == InputModeApplyResult::Applied {
                    self.input_mode = mode;
                }
            }
            ImeEvent::UserChangedInputMode { mode, .. } => {
                // ユーザーの明示操作 → 観測と同等の信頼度で即時反映する。
                self.input_mode = mode;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ime_event::{
        ApplyError, ChordKind, EventTime, HwndId, ImePolicyProfile, ObservationConfidence,
        ObservationSource,
    };
    use super::*;
    use crate::state::evidence::AnyObservation;
    use crate::state::force_guard::{ForceGuard, ForceOnReason};
    use std::time::Instant;

    fn envelope(seq: u64, event: ImeEvent) -> ImeEventEnvelope {
        ImeEventEnvelope {
            time: EventTime {
                seq,
                monotonic: Instant::now(),
                tick_ms: 0,
            },
            event,
        }
    }

    // ── AppliedImeState / ImeModel::applied_pair 系 getter ──────────────────
    //
    // これらは `runtime/executor.rs` で間接的に使われテストもあるが、そちらは
    // crate 全体が `#![cfg(windows)]` のため Linux 上の `cargo mutants -p
    // awase-windows` では一切ビルドされず、mutants の実行対象にならない。
    // ここ(`state/ime_model.rs` 自身の `#[cfg(test)]`)はプラットフォーム非依存で
    // Linux でも実行されるため、バリアント別の直接テストをここに置く。

    #[test]
    fn applied_ime_state_to_pair_and_related_getters() {
        assert_eq!(AppliedImeState::Unknown.to_pair(), None);
        assert!(!AppliedImeState::Unknown.is_confirmed());
        assert_eq!(AppliedImeState::Unknown.confirmed_at_ms(), 0);

        assert_eq!(AppliedImeState::Optimistic(true).to_pair(), Some((true, 0)));
        assert!(!AppliedImeState::Optimistic(true).is_confirmed());
        assert_eq!(AppliedImeState::Optimistic(true).confirmed_at_ms(), 0);

        let confirmed = AppliedImeState::Confirmed {
            open: false,
            at_ms: 42,
        };
        assert_eq!(confirmed.to_pair(), Some((false, 42)));
        assert!(confirmed.is_confirmed());
        assert_eq!(confirmed.confirmed_at_ms(), 42);
    }

    #[test]
    fn applied_pair_reflects_applied_state() {
        let mut model = ImeModel::new();
        assert_eq!(model.applied_pair(), None, "初期状態は Unknown");

        model.applied = AppliedImeState::Confirmed {
            open: true,
            at_ms: 7,
        };
        assert_eq!(model.applied_pair(), Some((true, 7)));
    }

    #[test]
    fn is_focus_transition_pending_reflects_input_barrier() {
        let mut model = ImeModel::new();
        assert!(
            !model.is_focus_transition_pending(),
            "barrier なしなら false"
        );

        model.input_barrier = Some(InputBarrier::FocusTransition {
            to_hwnd: HwndId::NULL,
            started_seq: 1,
            started_at: Instant::now(),
            settle_until: Instant::now() + std::time::Duration::from_millis(100),
        });
        assert!(
            model.is_focus_transition_pending(),
            "FocusTransition barrier があれば true"
        );

        model.input_barrier = Some(InputBarrier::CtrlImeChord {
            target: false,
            kind: ChordKind::CtrlMuhenkanImeOff,
            started_seq: 1,
            started_at: Instant::now(),
        });
        assert!(
            !model.is_focus_transition_pending(),
            "CtrlImeChord は FocusTransition ではない"
        );
    }

    #[test]
    fn user_intent_sets_desired() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: false,
                source: UserIntentSource::PhysicalImeKey,
            },
        ));
        assert!(!model.desired_open);
        assert_eq!(model.last_intent.as_ref().unwrap().target, false);
    }

    #[test]
    fn toggle_intent_flips_desired() {
        let mut model = ImeModel::new(); // desired_open = true (default)
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeToggleIntent {
                source: UserIntentSource::PhysicalImeKey,
            },
        ));
        assert!(!model.desired_open);
        model.reduce(&envelope(
            2,
            ImeEvent::UserImeToggleIntent {
                source: UserIntentSource::PhysicalImeKey,
            },
        ));
        assert!(model.desired_open);
    }

    #[test]
    fn observer_does_not_change_desired() {
        let mut model = ImeModel::new(); // desired_open = true
        model.reduce(&envelope(
            1,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                false,
                ObservationSource::ObserverPoll,
                HwndId::NULL,
                ObservationConfidence::Medium,
                0,
            )),
        ));
        assert!(model.desired_open, "observer は desired を壊さない");
        assert_eq!(
            model
                .observations
                .per_source
                .observer_poll
                .as_ref()
                .unwrap()
                .open,
            false
        );
    }

    /// BUG-19 再発対策: `KatakanaShadowOff`/`NativeToggleShadowOff` が
    /// `PlatformState::report_conv_open_inference()` 経由で `ObserverReported
    /// { source: ConvOpenInference }` を dispatch しても、`desired_open` と
    /// `last_intent`（ユーザーの明示 OFF 意図）は一切変更されないことを固定する。
    /// `ConvOpenInference` は `PerSourceObservations` に正式に記録される点が
    /// `ConvBitsInference`（input_mode 専用、常に記録されない）と異なる。
    #[test]
    fn conv_open_inference_observer_does_not_change_desired_or_last_intent() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: false,
                source: UserIntentSource::PhysicalImeKey,
            },
        ));
        assert!(!model.desired_open);
        assert!(model.last_intent.is_some());

        model.reduce(&envelope(
            2,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                true,
                ObservationSource::ConvOpenInference,
                HwndId::NULL,
                ObservationConfidence::Medium,
                0,
            )),
        ));

        assert!(
            !model.desired_open,
            "conv 由来の open 推論は desired_open を書き換えない (BUG-19 再発対策)"
        );
        assert!(
            model.last_intent.is_some(),
            "conv 由来の open 推論は last_intent (ユーザー明示意図) を消さない"
        );
        assert_eq!(
            model
                .observations
                .per_source
                .conv_open_inference
                .as_ref()
                .unwrap()
                .open,
            true,
            "ConvOpenInference は ConvBitsInference と異なり正式な open 観測として記録される"
        );
        assert!(
            model.observations.drift.is_some(),
            "desired=false と observed=true の乖離が drift として追跡される"
        );
    }

    #[test]
    fn effective_open_falls_back_to_most_recent_trusted_when_derive_open_is_none() {
        let mut model = ImeModel::new(); // desired_open = true, 明示 intent なし
                                         // Low confidence 単独 → derive_any() は None（Medium+ 専用のため）。
        model.reduce(&envelope(
            1,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                false,
                ObservationSource::HeuristicDefault,
                HwndId::NULL,
                ObservationConfidence::Low,
                0,
            )),
        ));
        assert!(
            !model.effective_open(),
            "derive_open()=None でも most_recent_trusted() の Low observation が \
             desired_open より優先される"
        );
    }

    #[test]
    fn effective_open_medium_observation_overrides_low_fallback() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                false,
                ObservationSource::HeuristicDefault,
                HwndId::NULL,
                ObservationConfidence::Low,
                0,
            )),
        ));
        model.reduce(&envelope(
            2,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                true,
                ObservationSource::ObserverPoll,
                HwndId::NULL,
                ObservationConfidence::Medium,
                0,
            )),
        ));
        assert!(
            model.effective_open(),
            "Medium confidence の derive_any() 結果が Low fallback より常に優先される"
        );
    }

    // ── resolve_open_at / DecidedBy（ADR-087 §5 Phase 0a item2/3） ──────────────

    #[test]
    fn resolve_open_at_decided_by_explicit_intent() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: false,
                source: UserIntentSource::PhysicalImeKey,
            },
        ));
        let now = Instant::now();
        let res = model.resolve_open_at(now);
        assert!(!res.value);
        assert_eq!(res.decided_by.base, BaseDecision::ExplicitIntent);
        assert_eq!(res.decided_by.guard_override, None);
    }

    #[test]
    fn resolve_open_at_decided_by_derive_medium_mise_bug_scenario() {
        // ADR-087 発端バグ（mise→くした）の belief 側の再現: 明示意図なし、
        // ConvOpenInference 1件（Medium, open:true）だけがある状態。
        // belief は ON に復帰する（P13: derive_any() の Medium 単独多数決は
        // 弱めない、BUG-26 が依拠する挙動）。
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                true,
                ObservationSource::ConvOpenInference,
                HwndId::NULL,
                ObservationConfidence::Medium,
                0,
            )),
        ));
        let now = Instant::now();
        let res = model.resolve_open_at(now);
        assert!(
            res.value,
            "conv 推論1件で belief は ON に復帰する（BUG-26 の依拠先）"
        );
        assert_eq!(
            res.decided_by.base,
            BaseDecision::DeriveMedium {
                first: ObservationSource::ConvOpenInference,
                second: None,
            }
        );
    }

    #[test]
    fn resolve_open_at_decided_by_desired_fallback() {
        let model = ImeModel::new(); // desired_open=true, 観測なし, 意図なし
        let now = Instant::now();
        let res = model.resolve_open_at(now);
        assert!(res.value);
        assert_eq!(res.decided_by.base, BaseDecision::DesiredFallback);
    }

    #[test]
    fn resolve_open_at_reports_guard_override() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: false,
                source: UserIntentSource::PhysicalImeKey,
            },
        ));
        model.force_guards.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: 1,
        });
        let now = Instant::now();
        let res = model.resolve_open_at(now);
        assert!(res.value, "PanicReset は明示 OFF 意図を override する");
        assert_eq!(
            res.decided_by.base,
            BaseDecision::ExplicitIntent,
            "base 自体は明示意図のまま false 側で決まる"
        );
        assert_eq!(
            res.decided_by.guard_override,
            Some(ForceOnReason::PanicReset),
            "guard_override に override した reason が残る"
        );
    }

    #[test]
    fn resolve_open_at_guard_override_none_when_guard_active_but_did_not_flip_value() {
        // ADR-087 §7 round4 M-C の直接の回帰テスト: guard が active でも
        // base が既に true なら override は起きていないので guard_override は
        // None であるべき（旧実装は誤って Some を返していた）。
        let mut model = ImeModel::new(); // desired_open=true, 明示意図なし
        model.force_guards.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 1,
        });
        let now = Instant::now();
        let res = model.resolve_open_at(now);
        assert!(res.value);
        assert_eq!(
            res.decided_by.guard_override, None,
            "base が既に true（DesiredFallback）なので guard は何も override していない"
        );
    }

    #[test]
    fn resolve_open_at_now_argument_actually_affects_result() {
        // ADR-087 §7 round4 M-B: 注入した `now` が本当に使われていることの
        // 直接検証。derive_any() の FRESH ウィンドウ（3秒、observation_store.rs）
        // を跨ぐ前後で decided_by が変わることを確認する——これが変わらなければ
        // resolve_open_at が引数を無視している可能性がある。
        let mut model = ImeModel::new();
        let t0 = Instant::now();
        model.reduce(&envelope(
            1,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                true,
                ObservationSource::ObserverPoll,
                HwndId::NULL,
                ObservationConfidence::Medium,
                0,
            )),
        ));
        let fresh = model.resolve_open_at(t0);
        assert_eq!(
            fresh.decided_by.base,
            BaseDecision::DeriveMedium {
                first: ObservationSource::ObserverPoll,
                second: None,
            },
            "FRESH ウィンドウ内では derive_open が観測を採用する"
        );

        let stale = model.resolve_open_at(t0 + std::time::Duration::from_secs(4));
        assert_eq!(
            stale.decided_by.base,
            BaseDecision::MostRecentTrusted(ObservationSource::ObserverPoll),
            "FRESH(3s) を超えると derive_open は None になるが、\
             most_recent_trusted() は expires_at のみを見る（FRESH 窓を見ない）\
             ため同じ観測を拾い、フォールバック先が DeriveMedium から \
             MostRecentTrusted に切り替わる——now が本当に効いている証拠"
        );
    }

    #[test]
    fn effective_open_at_matches_effective_open() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                true,
                ObservationSource::ObserverPoll,
                HwndId::NULL,
                ObservationConfidence::Medium,
                0,
            )),
        ));
        // effective_open() は effective_open_at(Instant::now()) の薄いラッパーで
        // あるべき。テスト実行中に Instant が動くのは無視できる程度なので、
        // 両者が同じ bool を返すことだけ確認する。
        assert_eq!(
            model.effective_open(),
            model.effective_open_at(Instant::now())
        );
    }

    #[test]
    fn input_mode_observed_low_confidence_is_ignored() {
        let mut model = ImeModel::new(); // input_mode = ObservedRomaji (初期値)
        model.reduce(&envelope(
            1,
            ImeEvent::InputModeObserved {
                mode: InputModeState::ObservedEisu,
                source: ObservationSource::FocusProbe,
                confidence: ObservationConfidence::Low,
                at: crate::state::TickMs(0),
            },
        ));
        assert_eq!(
            model.input_mode(),
            InputModeState::ObservedRomaji,
            "Low confidence の観測は input_mode を上書きしない"
        );
    }

    #[test]
    fn input_mode_observed_medium_confidence_updates() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::InputModeObserved {
                mode: InputModeState::ObservedEisu,
                source: ObservationSource::ObserverPoll,
                confidence: ObservationConfidence::Medium,
                at: crate::state::TickMs(0),
            },
        ));
        assert_eq!(
            model.input_mode(),
            InputModeState::ObservedEisu,
            "Medium+ confidence の観測は input_mode を更新する"
        );
    }

    fn focus_changed_event(seq: u64) -> ImeEventEnvelope {
        envelope(
            seq,
            ImeEvent::FocusChanged {
                from: None,
                to: HwndId::NULL,
                profile: ImePolicyProfile::ImmCross,
                focus_epoch: seq,
            },
        )
    }

    #[test]
    fn focus_change_clears_force_guards() {
        let mut model = ImeModel::new();
        model.force_guards.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 1,
        });
        assert!(model.force_guards.requires_on());

        model.reduce(&focus_changed_event(2));

        assert!(
            !model.force_guards.requires_on(),
            "focus change で force guard が解除される"
        );
    }

    // 回帰テスト: BrokenAppBootstrap は observation-miss カウンタというヒューリスティック
    // にすぎないため、ユーザーが明示的に IME を OFF にした場合はそちらを尊重する
    // (force_guard.rs の overrides_explicit_intent() を参照)。
    #[test]
    fn broken_app_bootstrap_guard_does_not_override_explicit_off_intent() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: false,
                source: UserIntentSource::SyncKey,
            },
        ));
        model.force_guards.add(ForceGuard {
            reason: ForceOnReason::BrokenAppBootstrap,
            expires_at: None,
            generation: 1,
        });
        assert!(
            !model.effective_open(),
            "ユーザーの明示的な IME OFF は BrokenAppBootstrap guard より優先される"
        );
    }

    // 対比: PanicReset は安全弁のため、明示的意図があっても引き続き override する。
    #[test]
    fn panic_reset_guard_overrides_explicit_off_intent() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: false,
                source: UserIntentSource::SyncKey,
            },
        ));
        model.force_guards.add(ForceGuard {
            reason: ForceOnReason::PanicReset,
            expires_at: None,
            generation: 1,
        });
        assert!(
            model.effective_open(),
            "PanicReset は明示的意図があっても IME ON を保証する安全弁として override する"
        );
    }

    #[test]
    fn focus_change_resets_observe_miss_monitor() {
        let mut model = ImeModel::new();
        let t = Instant::now();
        model.observe_miss_monitor.record_miss(t);
        model.observe_miss_monitor.record_miss(t);
        model.observe_miss_monitor.record_miss(t);
        assert!(model.observe_miss_monitor.exceeds(3));

        model.reduce(&focus_changed_event(2));

        assert!(
            !model.observe_miss_monitor.exceeds(1),
            "focus change で observe_miss_monitor がリセットされる"
        );
    }

    #[test]
    fn focus_change_does_not_clear_desired_open() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: true,
                source: UserIntentSource::PhysicalImeKey,
            },
        ));
        assert!(model.desired_open);

        model.reduce(&focus_changed_event(2));

        assert!(
            model.desired_open,
            "focus change は desired_open を変えない"
        );
    }

    // ── ImeApplyRequested による chord barrier 制御 (Phase 2) ──

    #[test]
    fn ime_off_with_ctrl_held_starts_chord() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ImeApplyRequested {
                target: false,
                generation: 1,
                ctrl_held: true,
            },
        ));
        assert!(
            model.is_ctrl_ime_chord_active(),
            "IME OFF 要求 + Ctrl 押下中 → chord 開始"
        );
        assert_eq!(
            model.active_chord_kind(),
            Some(ChordKind::CtrlMuhenkanImeOff)
        );
    }

    #[test]
    fn ime_off_without_ctrl_does_not_start_chord() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ImeApplyRequested {
                target: false,
                generation: 1,
                ctrl_held: false,
            },
        ));
        assert!(
            !model.is_ctrl_ime_chord_active(),
            "KANJI（Ctrl なし）IME OFF では chord を開始しない"
        );
    }

    #[test]
    fn ime_on_during_chord_ends_it() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ImeApplyRequested {
                target: false,
                generation: 1,
                ctrl_held: true,
            },
        ));
        assert!(model.is_ctrl_ime_chord_active());

        model.reduce(&envelope(
            2,
            ImeEvent::ImeApplyRequested {
                target: true,
                generation: 2,
                ctrl_held: true,
            },
        ));
        assert!(
            !model.is_ctrl_ime_chord_active(),
            "chord 中の IME ON 要求は chord を即時終了する"
        );
    }

    #[test]
    fn stale_ime_apply_success_does_not_consume_pending() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ImeApplyRequested {
                target: false,
                generation: 10,
                ctrl_held: false,
            },
        ));

        model.reduce(&envelope(
            2,
            ImeEvent::ImeApplySucceeded {
                target: false,
                generation: 9,
            },
        ));

        assert_eq!(
            model.pending_generation(),
            Some(10),
            "古い generation の完了で current pending を消費しない"
        );
    }

    #[test]
    fn matching_ime_apply_success_consumes_pending() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ImeApplyRequested {
                target: false,
                generation: 10,
                ctrl_held: false,
            },
        ));

        model.reduce(&envelope(
            2,
            ImeEvent::ImeApplySucceeded {
                target: false,
                generation: 10,
            },
        ));

        assert!(
            model.pending_generation().is_none(),
            "一致する generation の完了で pending を消費する"
        );
        assert!(
            model.applied.applied_open() == Some(false),
            "一致する generation の完了で applied state を更新する"
        );
    }

    /// `stale_ime_apply_success_does_not_consume_pending` の `ImeApplyFailed` 版。
    /// `reduce()` の `ImeApplyFailed` ハンドラは generation 照合 (`==`) で stale な
    /// 失敗完了を無視するはずだが、`ImeApplySucceeded` 側と異なりこの経路には
    /// 対称なテストが無く、`==`→`!=` の反転が mutants で検知されなかった。
    #[test]
    fn stale_ime_apply_failure_does_not_consume_pending() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ImeApplyRequested {
                target: false,
                generation: 10,
                ctrl_held: false,
            },
        ));

        model.reduce(&envelope(
            2,
            ImeEvent::ImeApplyFailed {
                target: false,
                generation: 9,
                error: ApplyError::Timeout,
            },
        ));

        assert_eq!(
            model.pending_generation(),
            Some(10),
            "古い generation の失敗完了で current pending を消費しない"
        );
    }

    #[test]
    fn matching_ime_apply_failure_consumes_pending() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(
            1,
            ImeEvent::ImeApplyRequested {
                target: false,
                generation: 10,
                ctrl_held: false,
            },
        ));

        model.reduce(&envelope(
            2,
            ImeEvent::ImeApplyFailed {
                target: false,
                generation: 10,
                error: ApplyError::Timeout,
            },
        ));

        assert!(
            model.pending_generation().is_none(),
            "一致する generation の失敗完了で pending を消費する"
        );
    }

    // ── BUG-34 横展開 D-prep: pending purge / UnsafeToToggle 解放 ──────────────

    /// `ImeTransition.timeout_at` は元々存在したが呼び出し元がゼロで、期限切れの
    /// pending が生存し続けていた。`reduce()` の先頭で毎 dispatch パージすることで、
    /// 期限切れ後に届く無関係なイベントが pending を自然に解放することを確認する。
    #[test]
    fn pending_purges_lazily_after_timeout_on_next_event() {
        let mut model = ImeModel::new();
        let t0 = Instant::now();
        model.reduce(&ImeEventEnvelope {
            time: EventTime {
                seq: 1,
                monotonic: t0,
                tick_ms: 0,
            },
            event: ImeEvent::ImeApplyRequested {
                target: true,
                generation: 10,
                ctrl_held: false,
            },
        });
        assert_eq!(model.pending_generation(), Some(10));

        // timeout_at は ImeApplyRequested から IME_APPLY_PENDING_TIMEOUT_MS 後
        // （tuning.rs 参照、BUG-34 実測の HungAppTimeout ~5741ms に安全マージンを
        // 載せた 8000ms）。期限をわずかに超えた時刻で、pending と無関係な
        // イベントを送る。
        let timeout_ms = crate::tuning::IME_APPLY_PENDING_TIMEOUT_MS;
        model.reduce(&ImeEventEnvelope {
            time: EventTime {
                seq: 2,
                monotonic: t0 + std::time::Duration::from_millis(timeout_ms + 1),
                tick_ms: timeout_ms + 1,
            },
            event: ImeEvent::ChordEnded {
                kind: ChordKind::CtrlMuhenkanImeOff,
            },
        });

        assert!(
            model.pending_generation().is_none(),
            "期限を過ぎたら、無関係な後続イベントの処理時に pending が自然にパージされる"
        );
    }

    /// 期限内であれば無関係なイベントが来ても pending はパージされないことを確認する
    /// （`pending_purges_lazily_after_timeout_on_next_event` の対称テスト）。
    #[test]
    fn pending_survives_unrelated_event_within_timeout() {
        let mut model = ImeModel::new();
        let t0 = Instant::now();
        model.reduce(&ImeEventEnvelope {
            time: EventTime {
                seq: 1,
                monotonic: t0,
                tick_ms: 0,
            },
            event: ImeEvent::ImeApplyRequested {
                target: true,
                generation: 10,
                ctrl_held: false,
            },
        });

        model.reduce(&ImeEventEnvelope {
            time: EventTime {
                seq: 2,
                monotonic: t0 + std::time::Duration::from_millis(500),
                tick_ms: 500,
            },
            event: ImeEvent::ChordEnded {
                kind: ChordKind::CtrlMuhenkanImeOff,
            },
        });

        assert_eq!(
            model.pending_generation(),
            Some(10),
            "期限(1秒)内なら無関係なイベントで pending を失わない"
        );
    }

    /// 進行中の未期限切れ pending を別の `ImeApplyRequested` が上書きしても、
    /// クラッシュせず新しい generation の pending に置き換わることを確認する
    /// （拒否はしない設計、警告ログのみ。ログ出力自体はここでは検証しない）。
    #[test]
    fn overwriting_live_pending_replaces_generation() {
        let mut model = ImeModel::new();
        let t0 = Instant::now();
        model.reduce(&ImeEventEnvelope {
            time: EventTime {
                seq: 1,
                monotonic: t0,
                tick_ms: 0,
            },
            event: ImeEvent::ImeApplyRequested {
                target: true,
                generation: 10,
                ctrl_held: false,
            },
        });
        assert_eq!(model.pending_generation(), Some(10));

        model.reduce(&ImeEventEnvelope {
            time: EventTime {
                seq: 2,
                monotonic: t0 + std::time::Duration::from_millis(50),
                tick_ms: 50,
            },
            event: ImeEvent::ImeApplyRequested {
                target: false,
                generation: 11,
                ctrl_held: false,
            },
        });

        assert_eq!(
            model.pending_generation(),
            Some(11),
            "上書きは拒否しない(警告ログのみ) — 新しい generation が pending になる"
        );
    }

    // ── PanicReset ────────────────────────────────────────────────────────────

    #[test]
    fn panic_reset_sets_desired_open() {
        let mut model = ImeModel::new(); // desired_open = true
        model.reduce(&envelope(1, ImeEvent::PanicReset { target: true }));
        assert!(
            model.desired_open,
            "PanicReset は desired_open を target に設定する"
        );
    }

    // 最重要: PanicReset は last_intent を設定しない。
    // これが UserImeSetIntent との本質的な差異。
    // last_intent が None のままなので has_user_explicit_intent() = false となり、
    // 後続の実観測が effective_open() を上書きできる。
    #[test]
    fn panic_reset_does_not_set_last_intent() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(1, ImeEvent::PanicReset { target: true }));
        assert!(
            model.last_intent.is_none(),
            "PanicReset は last_intent を設定しない（ForceGuard に委ねる）"
        );
    }

    // PanicReset 後は has_user_explicit_intent() が false のため、
    // Medium+ の実観測が effective_open() を上書きできることを確認。
    #[test]
    fn panic_reset_allows_observation_to_override_effective_open() {
        let mut model = ImeModel::new();
        // PanicReset で desired_open=true に戻す
        model.reduce(&envelope(1, ImeEvent::PanicReset { target: true }));
        assert!(model.desired_open);
        // Medium 観測が false を報告
        model.reduce(&envelope(
            2,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                false,
                ObservationSource::ObserverPoll,
                HwndId::NULL,
                ObservationConfidence::Medium,
                0,
            )),
        ));
        assert!(
            !model.effective_open(),
            "PanicReset 後は explicit intent がないため、Medium 観測が effective_open を上書きする"
        );
        assert!(
            model.desired_open,
            "desired_open は PanicReset の値 (true) のまま変わらない"
        );
    }

    // PanicReset ≠ UserImeSetIntent の対比：UserImeSetIntent は観測で上書きされない。
    #[test]
    fn user_intent_blocks_observation_unlike_panic_reset() {
        let mut model = ImeModel::new();
        // ユーザーが明示的に IME ON に設定した
        model.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: true,
                source: UserIntentSource::PhysicalImeKey,
            },
        ));
        // Medium 観測が false を報告（PanicReset とは違い上書きされない）
        model.reduce(&envelope(
            2,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                false,
                ObservationSource::ObserverPoll,
                HwndId::NULL,
                ObservationConfidence::Medium,
                0,
            )),
        ));
        assert!(
            model.effective_open(),
            "UserImeSetIntent 後は explicit intent があるため、観測は effective_open を上書きしない"
        );
    }

    // ── HwndCacheRestored ─────────────────────────────────────────────────────

    #[test]
    fn hwnd_cache_restored_sets_desired_open() {
        let mut model = ImeModel::new(); // desired_open = true
        model.reduce(&envelope(1, ImeEvent::HwndCacheRestored { target: false }));
        assert!(
            !model.desired_open,
            "HwndCacheRestored は desired_open を target に設定する"
        );
    }

    // 最重要: HwndCacheRestored は last_intent を設定しない。
    // キャッシュ復元はユーザーの能動的操作ではないため、
    // has_user_explicit_intent() を true にしてはならない。
    #[test]
    fn hwnd_cache_restored_does_not_set_last_intent() {
        let mut model = ImeModel::new();
        model.reduce(&envelope(1, ImeEvent::HwndCacheRestored { target: false }));
        assert!(
            model.last_intent.is_none(),
            "HwndCacheRestored は last_intent を設定しない（後続の実観測で上書き可能）"
        );
    }

    // HwndCacheRestored 後は has_user_explicit_intent() が false のため、
    // Medium+ の実観測が effective_open() を上書きできることを確認。
    // これが PanicReset と同じ「非意図 desired 書き換え」の設計。
    #[test]
    fn hwnd_cache_restored_allows_observation_to_override_effective_open() {
        let mut model = ImeModel::new();
        // キャッシュから desired_open=false を復元
        model.reduce(&envelope(1, ImeEvent::HwndCacheRestored { target: false }));
        assert!(!model.desired_open);
        // 実際の API 観測が true を返す（実 IME 状態は ON）
        model.reduce(&envelope(
            2,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                true,
                ObservationSource::ImmGetOpenStatus,
                HwndId::NULL,
                ObservationConfidence::High,
                0,
            )),
        ));
        assert!(
            model.effective_open(),
            "HwndCacheRestored 後は explicit intent がないため、High 観測が effective_open を上書きする"
        );
        assert!(
            !model.desired_open,
            "desired_open はキャッシュの復元値 (false) のまま変わらない"
        );
    }

    // HwndCacheRestored ≠ UserImeSetIntent の対比：
    // UserImeSetIntent は観測で effective_open が変わらないが、
    // HwndCacheRestored はキャッシュ起源なので観測で上書きされる。
    #[test]
    fn user_intent_blocks_observation_but_hwnd_cache_does_not() {
        // UserImeSetIntent の場合
        let mut model_intent = ImeModel::new();
        model_intent.reduce(&envelope(
            1,
            ImeEvent::UserImeSetIntent {
                target: false,
                source: UserIntentSource::SyncKey,
            },
        ));
        model_intent.reduce(&envelope(
            2,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                true,
                ObservationSource::ImmGetOpenStatus,
                HwndId::NULL,
                ObservationConfidence::High,
                0,
            )),
        ));
        assert!(
            !model_intent.effective_open(),
            "UserImeSetIntent 後は explicit intent が High 観測を遮断する"
        );

        // HwndCacheRestored の場合（同じ操作）
        let mut model_cache = ImeModel::new();
        model_cache.reduce(&envelope(1, ImeEvent::HwndCacheRestored { target: false }));
        model_cache.reduce(&envelope(
            2,
            ImeEvent::ObserverReported(AnyObservation::restored_from_journal(
                true,
                ObservationSource::ImmGetOpenStatus,
                HwndId::NULL,
                ObservationConfidence::High,
                0,
            )),
        ));
        assert!(
            model_cache.effective_open(),
            "HwndCacheRestored 後は explicit intent がなく、High 観測が通過する"
        );
    }

    // InputModeApplied のテスト

    #[test]
    fn input_mode_applied_updates_input_mode() {
        let mut model = ImeModel::new();
        // 初期状態は ObservedRomaji
        assert_eq!(model.input_mode(), InputModeState::ObservedRomaji);

        model.reduce(&envelope(
            1,
            ImeEvent::InputModeApplied {
                mode: InputModeState::ObservedEisu,
                strategy: crate::state::ime_event::InputModeApplyStrategy::ImmBrokenCorrection,
                result: InputModeApplyResult::Applied,
                at: crate::state::TickMs(0),
            },
        ));
        assert_eq!(
            model.input_mode(),
            InputModeState::ObservedEisu,
            "InputModeApplied(Applied) は input_mode を更新する"
        );
    }

    #[test]
    fn input_mode_applied_skipped_does_not_update_input_mode() {
        let mut model = ImeModel::new();
        // 初期状態は ObservedRomaji
        assert_eq!(model.input_mode(), InputModeState::ObservedRomaji);

        model.reduce(&envelope(
            1,
            ImeEvent::InputModeApplied {
                mode: InputModeState::ObservedEisu,
                strategy: crate::state::ime_event::InputModeApplyStrategy::ImmBrokenCorrection,
                result: InputModeApplyResult::Skipped,
                at: crate::state::TickMs(0),
            },
        ));
        assert_eq!(
            model.input_mode(),
            InputModeState::ObservedRomaji,
            "InputModeApplied(Skipped) は input_mode を変更しない"
        );
    }

    // is_focus_transition_settling: settle_until 前後での判定。

    #[test]
    fn is_focus_transition_settling_true_before_settle_until() {
        let mut model = ImeModel::new();
        let now = Instant::now();
        model.input_barrier = Some(InputBarrier::FocusTransition {
            to_hwnd: HwndId(1),
            started_seq: 1,
            started_at: now,
            settle_until: now + std::time::Duration::from_millis(100),
        });
        assert!(model.is_focus_transition_settling(now));
        assert!(
            model.is_focus_transition_pending(),
            "barrier はまだ consume されていない"
        );
    }

    #[test]
    fn is_focus_transition_settling_false_after_settle_until() {
        let mut model = ImeModel::new();
        let now = Instant::now();
        model.input_barrier = Some(InputBarrier::FocusTransition {
            to_hwnd: HwndId(1),
            started_seq: 1,
            started_at: now,
            settle_until: now + std::time::Duration::from_millis(100),
        });
        let later = now + std::time::Duration::from_millis(200);
        assert!(
            !model.is_focus_transition_settling(later),
            "settle_until 経過後は settling ではない"
        );
        assert!(
            model.is_focus_transition_pending(),
            "settle_until 経過だけでは barrier は consume されない（別途 consume_focus_barrier が必要）"
        );
    }

    #[test]
    fn is_focus_transition_settling_false_when_no_barrier() {
        let model = ImeModel::new();
        assert!(!model.is_focus_transition_settling(Instant::now()));
    }
}
