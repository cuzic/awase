//! GJI (Google Japanese Input) 内部状態推測 FSM。
//!
//! GJI の観測可能なイベント（IME ON/OFF、フォーカス変更、composition イベント、
//! warmup probe 結果）から GJI の内部状態を推測する。
//!
//! ## 状態空間（5状態）
//!
//! ```text
//!                ImeOn / FocusChange
//! OffCold ──────────────────────────────────────────────► OnCold(Short)
//!                                                              │
//!                                                   WarmupComplete / WarmupFailed
//!                                                              │
//! OnComposing ◄── StartComposition ── OnWarm ◄───────────────┘
//!     │                                  │
//!     │ EndComposition(epoch ✓)     LongIdle timeout
//!     │                                  │
//!     └──────────────── OnWarm          OnCold(Long, NotStarted)
//! ```
//!
//! ## 設計根拠
//!
//! - `pending` は `OnCold` バリアント内に持つ（型レベルで OffCold/OnWarm/OnComposing の
//!   pending を不正状態として排除）。
//! - `probe_id` により stale な WarmupComplete/WarmupFailed を安全に弾く。
//! - `epoch` により stale な EndComposition を安全に弾く。
//! - `OnCold(Long)` は `LongIdle` タイムアウト直後に入る状態で、最初の `KeyInput` が
//!   来るまで probe を開始しない（`ProbeStatus::NotStarted`）。
//! - LongIdle タイマーは timed-fsm の `on_timeout` で管理し、
//!   `KeyInput` ごとに `with_timer` でリセットする。

use std::time::Duration;

use timed_fsm::{Response, TimedStateMachine};

use crate::output::InjectionMode;
use crate::tsf::probe_fsm::DeferredVk;
use crate::tuning;

// ── プリミティブ型 ────────────────────────────────────────────────────────────

/// フォーカス epoch。stale な `EndComposition` を弾くための単調カウンタ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FocusEpoch(u32);

impl FocusEpoch {
    /// 初期 epoch。
    pub(crate) const ZERO: Self = Self(0);

    /// 次の epoch（単調増加、wrapping）。
    pub(crate) const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// probe ID。stale な `WarmupComplete` / `WarmupFailed` を弾くための識別子。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProbeId(u32);

/// `GjiAction::StartProbe` / `ProbeStatus::Authorized` が持つ probe パラメータ。
///
/// `TsfProbeMachine::new_gji` に渡す3値をまとめる。
/// `ColdKind` から `transition_to_cold` / `on_event(KeyInput NotStarted)` で生成する。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProbeParams {
    pub ncwait_budget_ms: u64,
    pub forces_prepend_f2: bool,
    pub is_long_cold: bool,
}

impl Default for ProbeParams {
    fn default() -> Self {
        Self {
            ncwait_budget_ms: tuning::SETTLE_TIMEOUT_MS,
            forces_prepend_f2: false,
            is_long_cold: false,
        }
    }
}

// ── WarmupResult ─────────────────────────────────────────────────────────────

/// warmup probe の完了経路（4-bool の旧 WarmupResult を enum に昇格）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WarmupPath {
    /// OBJ_NAMECHANGE 発火を確認した通常経路
    NameChangeConfirmed,
    /// eager LiteralDetect 経由で composition を確認した経路
    EagerLiteralDetected,
    /// GJI long-idle 後に I/O 応答を確認した経路
    GjiResumed,
    /// budget 枯渇による保守的フォールバック
    TimedOutFallback,
}

/// warmup probe の結果。送信方法（`SendInput`）の決定に使う。
// Phase 3 で SendInput dispatch が実装されるまでフィールドは書き込みのみ。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct WarmupResult {
    pub path: WarmupPath,
    pub prepend_f2_warmup: bool,
    pub nc_fired: bool,
    pub gji_resumed: bool,
}

impl WarmupResult {
    /// budget 枯渇時の保守的フォールバック値（`WarmupFailed` イベントで使用）。
    pub(crate) const fn conservative_fallback() -> Self {
        Self {
            path: WarmupPath::TimedOutFallback,
            prepend_f2_warmup: true,
            nc_fired: false,
            gji_resumed: false,
        }
    }
}

// ── PendingInput ─────────────────────────────────────────────────────────────

/// `OnCold` 中に蓄積する入力バッファ（旧 `TsfProbeMachine::SendState` に相当）。
///
/// warmup 完了後に `GjiAction::SendInput` に格納して dispatcher に渡す。
// Phase 3 で SendInput/SendInputDirect が実際に dispatch されるまでフィールドは蓄積のみ。
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct PendingInput {
    pub romaji: String,
    pub deferred_vks: Vec<DeferredVk>,
}

impl PendingInput {
    pub(crate) fn new(romaji: impl Into<String>) -> Self {
        Self {
            romaji: romaji.into(),
            deferred_vks: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.romaji.is_empty() && self.deferred_vks.is_empty()
    }
}

// ── 状態型 ───────────────────────────────────────────────────────────────────

/// `OnCold` の種別と warmup budget。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColdKind {
    /// フォーカス変更・IME-ON 直後、GJI 確実に生存（即 Running、budget=100ms）
    Short,
    /// medium idle (7000–9999ms)、GJI 生存不明（NotStarted、ncwait_budget=550ms）
    Medium,
    /// LongIdle タイムアウト後（NotStarted、ncwait_budget=GJI_LONG_IDLE_PROBE_TOTAL_MS=350ms）
    Long,
}

impl ColdKind {
    /// GjiProbe フェーズの probe_budget_ms（Short のみ即プローブ開始で使用）
    pub(crate) const fn budget_ms(self) -> u64 {
        match self {
            Self::Short => 100,
            Self::Medium | Self::Long => tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS,
        }
    }

    /// NameChangeWait フェーズの deadline budget。`apply_fresh_f2_sent` に渡す。
    pub(crate) const fn ncwait_budget_ms(self) -> u64 {
        match self {
            Self::Short => tuning::SETTLE_TIMEOUT_MS,
            Self::Medium => tuning::MEDIUM_IDLE_PROBE_TOTAL_MS,
            Self::Long => tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS,
        }
    }

    /// F2 をバッチに強制同梱するか（GJI が寝ている可能性がある Medium/Long で true）。
    pub(crate) const fn forces_prepend_f2(self) -> bool {
        matches!(self, Self::Medium | Self::Long)
    }

    /// Long cold（≥ LONG_IDLE_MS = 10s）か。`literal_detect_ms` 延長の判定に使う。
    pub(crate) const fn is_long(self) -> bool {
        matches!(self, Self::Long)
    }

    /// 即プローブを開始するか（Short のみ true、Medium/Long は KeyInput まで待機）。
    pub(crate) const fn is_proactive(self) -> bool {
        matches!(self, Self::Short)
    }

    /// gji_idle_ms から cold 種別を分類する。idle 判断の唯一の所在地。
    pub(crate) const fn classify(gji_idle_ms: u64) -> Self {
        if gji_idle_ms >= tuning::LONG_IDLE_MS {
            Self::Long
        } else if gji_idle_ms >= tuning::MEDIUM_IDLE_PROBE_MS {
            Self::Medium
        } else {
            Self::Short
        }
    }
}

/// `OnCold` 内の probe 進行状態。
pub(crate) enum ProbeStatus {
    /// probe 未開始（Medium/Long タイムアウト直後、最初の `KeyInput` を待つ）
    NotStarted,
    /// `StartProbe` を発行済み。`vk_send` が `GjiWarmupFsm::new` を作成して
    /// `install_pending_tsf` を呼ぶと probe が開始される。
    Authorized { probe_id: ProbeId, params: ProbeParams },
}

impl std::fmt::Debug for ProbeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStarted => write!(f, "NotStarted"),
            Self::Authorized { probe_id, params } => {
                f.debug_struct("Authorized").field("probe_id", probe_id).field("params", params).finish()
            }
        }
    }
}

/// GJI FSM の状態。
#[derive(Debug)]
pub(crate) enum GjiState {
    /// IME OFF（初期状態）
    OffCold,
    /// IME ON、TSF cold。`pending` は型安全のためここだけが持つ。
    OnCold {
        kind: ColdKind,
        probe: ProbeStatus,
        pending: Vec<PendingInput>,
        /// `NativeF2Consumed` を Medium/Long cold 中に受信したことを示すフラグ。
        ///
        /// WezTerm が FocusChange 直後に自分で F2 を送る際に立つ。
        /// probe はキャンセルせず継続し、この事実を probe 完了時に参照できる。
        saw_native_f2: bool,
    },
    /// IME ON、TSF warm
    OnWarm { long_idle_ms: u64 },
    /// IME ON、TSF warm、変換中
    OnComposing { epoch: FocusEpoch },
}

// ── イベント・アクション・タイマー ──────────────────────────────────────────

/// GJI FSM に入力するイベント。
#[derive(Debug)]
pub(crate) enum GjiEvent {
    /// IME ON（エンジン起動）。`gji_idle_ms` で ColdKind を分類する（FocusChange と同様）。
    ImeOn { injection_mode: InjectionMode, gji_idle_ms: u64 },
    /// IME OFF（エンジン停止）
    ImeOff,
    /// フォーカス変更。`gji_idle_ms` で ColdKind を分類する。
    FocusChange { injection_mode: InjectionMode, gji_idle_ms: u64 },
    /// キー入力（ローマ字 + deferred VK）
    KeyInput(PendingInput),
    /// warmup probe 完了
    WarmupComplete { probe_id: ProbeId, result: WarmupResult },
    /// warmup probe が budget 内に完了しなかった
    // Phase 3 で接続予定
    #[allow(dead_code)]
    WarmupFailed { probe_id: ProbeId },
    /// `WM_IME_STARTCOMPOSITION`
    // Phase 3 で接続予定
    #[allow(dead_code)]
    StartComposition,
    /// `WM_IME_ENDCOMPOSITION`（epoch チェック付き）
    // Phase 3 で接続予定
    #[allow(dead_code)]
    EndComposition { epoch: FocusEpoch },
    /// WezTerm が FocusChange 直後に内部で F2 を送信し TSF context を初期化した
    /// (reinject-tsf の NativeF2Consumed パス)。
    ///
    /// Medium/Long cold の場合は probe を継続し、`OnCold.saw_native_f2 = true` を立てる。
    /// Short cold / OnWarm / OnComposing の場合は `CompositionReset` 相当で処理する。
    NativeF2Consumed,
    /// IME ON/OFF やフォーカス変化なしに composition context が無効化された
    /// (PassthroughKey, RawTsfLiteralRecovery 等)
    CompositionReset,
}

/// GJI FSM が出力するアクション（ディスパッチャが副作用を実行する）。
#[derive(Debug)]
pub(crate) enum GjiAction {
    /// 新しい warmup probe を開始する
    StartProbe {
        probe_id: ProbeId,
        /// GjiProbe フェーズの最大待機時間 (ms)
        budget_ms: u64,
        /// `TsfProbeMachine::new_gji` に渡す probe パラメータ
        params: ProbeParams,
    },
    /// 実行中の probe をキャンセルする
    CancelProbe { probe_id: ProbeId },
    /// warmup 完了後に蓄積入力を送信する（Phase 3 で dispatch 実装予定）
    #[allow(dead_code)]
    SendInput { result: WarmupResult, pending: Vec<PendingInput> },
    /// warm 状態で即送信する（Phase 3 で dispatch 実装予定）
    #[allow(dead_code)]
    SendInputDirect(PendingInput),
}

/// GJI FSM のタイマー識別子（timed-fsm の `TimerId`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GjiTimer {
    /// `OnWarm` 中のアイドル監視タイマー（発火 → `OnCold(Long)`）
    LongIdle,
}

// ── FSM 本体 ─────────────────────────────────────────────────────────────────

/// GJI 内部状態推測 FSM。
///
/// 副作用はなし。遷移ごとに `Response<GjiAction, GjiTimer>` を返し、
/// ディスパッチャが `GjiAction` を実行し timed-fsm ランタイムがタイマーを管理する。
///
/// ## CompositionFsm との関係
///
/// [`crate::tsf::composition_fsm::CompositionFsm`] も warm/cold を追跡するが意味が異なる。
/// `CompositionFsm` は warmup 送信タイミングの制御、`GjiFsm` は GJI readiness の事実推測。
/// dispatcher が両方に個別にイベントを送り、直接の依存関係はない。
pub(crate) struct GjiFsm {
    state: GjiState,
    /// `EndComposition` の stale 判定用 epoch
    epoch: FocusEpoch,
    /// `ProbeId` の連番カウンタ
    next_probe_id: u32,
    /// 現在フォーカス中アプリの injection_mode（`long_idle_ms` 計算用）
    injection_mode: InjectionMode,
}

impl GjiFsm {
    pub(crate) fn new() -> Self {
        Self {
            state: GjiState::OffCold,
            epoch: FocusEpoch(0),
            next_probe_id: 0,
            injection_mode: InjectionMode::Unicode,
        }
    }

    pub(crate) const fn state(&self) -> &GjiState {
        &self.state
    }

    fn alloc_probe_id(&mut self) -> ProbeId {
        let id = ProbeId(self.next_probe_id);
        self.next_probe_id = self.next_probe_id.wrapping_add(1);
        id
    }

    fn bump_epoch(&mut self) -> FocusEpoch {
        self.epoch = self.epoch.next();
        self.epoch
    }

    fn long_idle_ms(&self) -> u64 {
        long_idle_ms_for(self.injection_mode)
    }

    /// `OnCold(Authorized)` なら probe_id を返す。
    fn running_probe_id(&self) -> Option<ProbeId> {
        match &self.state {
            GjiState::OnCold { probe: ProbeStatus::Authorized { probe_id, .. }, .. } => Some(*probe_id),
            _ => None,
        }
    }

    /// `OnCold(Authorized)` なら `ProbeParams` を返す。
    ///
    /// `vk_send` が `TsfProbeMachine::new_gji` に渡すパラメータを読み出すために使う。
    pub(crate) fn current_probe_params(&self) -> Option<ProbeParams> {
        match &self.state {
            GjiState::OnCold {
                probe: ProbeStatus::Authorized { params, .. },
                ..
            } => Some(*params),
            _ => None,
        }
    }

    /// OnCold 入場（probe を強制即開始）。`ImeOn` 専用。
    ///
    /// `FocusChange` と異なり、ユーザーが F2 で IME ON した場合は Long/Medium でも
    /// 即プローブを開始する（入力意図が確実なため）。
    fn transition_to_cold_proactive(
        &mut self,
        kind: ColdKind,
        initial_pending: Vec<PendingInput>,
        old_probe: Option<ProbeId>,
    ) -> Response<GjiAction, GjiTimer> {
        let probe_id = self.alloc_probe_id();
        let params = ProbeParams {
            ncwait_budget_ms: kind.ncwait_budget_ms(),
            forces_prepend_f2: kind.forces_prepend_f2(),
            is_long_cold: kind.is_long(),
        };
        self.state = GjiState::OnCold {
            kind,
            probe: ProbeStatus::Authorized { probe_id, params },
            pending: initial_pending,
            saw_native_f2: false,
        };
        let mut actions = Vec::new();
        if let Some(id) = old_probe {
            actions.push(GjiAction::CancelProbe { probe_id: id });
        }
        actions.push(GjiAction::StartProbe { probe_id, budget_ms: kind.budget_ms(), params });
        Response::emit(actions).with_kill_timer(GjiTimer::LongIdle)
    }

    /// OnCold 入場（既存 probe のキャンセルと新 probe の開始を含む）。
    ///
    /// `Short` → 即 probe 開始（is_proactive）、`Medium`/`Long` → `NotStarted`（最初の `KeyInput` まで待機）。
    fn transition_to_cold(
        &mut self,
        kind: ColdKind,
        initial_pending: Vec<PendingInput>,
        old_probe: Option<ProbeId>,
    ) -> Response<GjiAction, GjiTimer> {
        let (probe_status, start_action) = if kind.is_proactive() {
            let probe_id = self.alloc_probe_id();
            let params = ProbeParams {
                ncwait_budget_ms: kind.ncwait_budget_ms(),
                forces_prepend_f2: kind.forces_prepend_f2(),
                is_long_cold: kind.is_long(),
            };
            (
                ProbeStatus::Authorized { probe_id, params },
                Some(GjiAction::StartProbe { probe_id, budget_ms: kind.budget_ms(), params }),
            )
        } else {
            (ProbeStatus::NotStarted, None)
        };

        self.state = GjiState::OnCold {
            kind,
            probe: probe_status,
            pending: initial_pending,
            saw_native_f2: false,
        };

        let mut actions = Vec::new();
        if let Some(id) = old_probe {
            actions.push(GjiAction::CancelProbe { probe_id: id });
        }
        if let Some(a) = start_action {
            actions.push(a);
        }
        Response::emit(actions).with_kill_timer(GjiTimer::LongIdle)
    }

    /// OnWarm 入場（LongIdle タイマーを開始する）。
    fn transition_to_warm(
        &mut self,
        extra_actions: Vec<GjiAction>,
    ) -> Response<GjiAction, GjiTimer> {
        let long_idle_ms = self.long_idle_ms();
        self.state = GjiState::OnWarm { long_idle_ms };
        Response::emit(extra_actions)
            .with_timer(GjiTimer::LongIdle, Duration::from_millis(long_idle_ms))
    }

    /// OnComposing 入場（LongIdle タイマーを kill する）。
    fn transition_to_composing(
        &mut self,
        extra_actions: Vec<GjiAction>,
    ) -> Response<GjiAction, GjiTimer> {
        let epoch = self.bump_epoch();
        self.state = GjiState::OnComposing { epoch };
        Response::emit(extra_actions).with_kill_timer(GjiTimer::LongIdle)
    }

    /// composition context が無効化されたときの共通処理。
    ///
    /// 既存 probe をキャンセルして `OnCold(Short, NotStarted)` に戻る。
    /// `NativeF2Consumed` と `CompositionReset` 双方から呼ばれる。
    fn handle_composition_reset(&mut self) -> Response<GjiAction, GjiTimer> {
        match &self.state {
            GjiState::OffCold => Response::consume(),

            GjiState::OnCold { .. } => {
                // 既存 probe をキャンセルして Short で再開（pending も破棄）
                let old = self.running_probe_id();
                self.state = GjiState::OnCold {
                    kind: ColdKind::Short,
                    probe: ProbeStatus::NotStarted,
                    pending: vec![],
                    saw_native_f2: false,
                };
                let mut actions = Vec::new();
                if let Some(id) = old {
                    actions.push(GjiAction::CancelProbe { probe_id: id });
                }
                Response::emit(actions).with_kill_timer(GjiTimer::LongIdle)
            }

            GjiState::OnWarm { .. } | GjiState::OnComposing { .. } => {
                self.state = GjiState::OnCold {
                    kind: ColdKind::Short,
                    probe: ProbeStatus::NotStarted,
                    pending: vec![],
                    saw_native_f2: false,
                };
                Response::consume().with_kill_timer(GjiTimer::LongIdle)
            }
        }
    }
}

impl Default for GjiFsm {
    fn default() -> Self {
        Self::new()
    }
}

impl TimedStateMachine for GjiFsm {
    type Event = GjiEvent;
    type Action = GjiAction;
    type TimerId = GjiTimer;

    #[expect(clippy::too_many_lines)]
    fn on_event(&mut self, event: GjiEvent) -> Response<GjiAction, GjiTimer> {
        match event {
            // ── ImeOn ──────────────────────────────────────────────────────
            GjiEvent::ImeOn { injection_mode, gji_idle_ms } => {
                self.injection_mode = injection_mode;
                match &self.state {
                    GjiState::OffCold => {
                        let kind = ColdKind::classify(gji_idle_ms);
                        log::debug!("[gji-fsm] ImeOn gji_idle={gji_idle_ms}ms → {kind:?}");
                        // ImeOn（ユーザーが F2 を押した）は FocusChange と異なり
                        // 即入力する意図があるため、Long/Medium でも proactive に probe を開始する。
                        self.transition_to_cold_proactive(kind, vec![], None)
                    }
                    _ => {
                        log::debug!(
                            "[gji-fsm] ImeOn: already on ({}), ignored",
                            state_label(&self.state)
                        );
                        Response::consume()
                    }
                }
            }

            // ── ImeOff ─────────────────────────────────────────────────────
            GjiEvent::ImeOff => {
                let old_probe = self.running_probe_id();
                let pending_count = match &self.state {
                    GjiState::OnCold { pending, .. } => pending.len(),
                    _ => 0,
                };
                match &self.state {
                    GjiState::OffCold => return Response::consume(),
                    _ => {}
                }
                if pending_count > 0 {
                    log::warn!(
                        "[gji-fsm] ImeOff with {pending_count} pending input(s) — discarding"
                    );
                }
                let mut actions = Vec::new();
                if let Some(id) = old_probe {
                    actions.push(GjiAction::CancelProbe { probe_id: id });
                }
                self.state = GjiState::OffCold;
                Response::emit(actions).with_kill_timer(GjiTimer::LongIdle)
            }

            // ── FocusChange ────────────────────────────────────────────────
            GjiEvent::FocusChange { injection_mode, gji_idle_ms } => {
                self.injection_mode = injection_mode;
                let old_probe = self.running_probe_id();
                let pending_count = match &self.state {
                    GjiState::OnCold { pending, .. } => pending.len(),
                    _ => 0,
                };
                let engine_on = !matches!(self.state, GjiState::OffCold);

                if !engine_on {
                    // エンジン OFF のままフォーカスが動いても状態変化なし
                    return Response::consume();
                }
                if pending_count > 0 {
                    log::warn!(
                        "[gji-fsm] FocusChange with {pending_count} pending input(s) — discarding"
                    );
                }
                let kind = ColdKind::classify(gji_idle_ms);
                log::debug!(
                    "[gji-fsm] FocusChange gji_idle={gji_idle_ms}ms → {kind:?}"
                );
                self.transition_to_cold(kind, vec![], old_probe)
            }

            // ── KeyInput ───────────────────────────────────────────────────
            GjiEvent::KeyInput(input) => {
                // NotStarted の場合のみ probe_id を事前確保する（&mut self.state との二重借用を回避）
                let maybe_new_probe_id = if matches!(
                    &self.state,
                    GjiState::OnCold { probe: ProbeStatus::NotStarted, .. }
                ) {
                    Some(self.alloc_probe_id())
                } else {
                    None
                };

                match &mut self.state {
                    GjiState::OffCold => Response::pass_through(),

                    GjiState::OnCold { probe, pending, kind, .. } => {
                        let kind = *kind;
                        match probe {
                            ProbeStatus::NotStarted => {
                                // Medium/Long の最初の KeyInput で probe を開始する
                                let probe_id = maybe_new_probe_id.unwrap();
                                let params = ProbeParams {
                                    ncwait_budget_ms: kind.ncwait_budget_ms(),
                                    forces_prepend_f2: kind.forces_prepend_f2(),
                                    is_long_cold: kind.is_long(),
                                };
                                *probe = ProbeStatus::Authorized { probe_id, params };
                                pending.push(input);
                                Response::emit(vec![GjiAction::StartProbe {
                                    probe_id,
                                    budget_ms: kind.budget_ms(),
                                    params,
                                }])
                            }
                            ProbeStatus::Authorized { .. } => {
                                pending.push(input);
                                Response::consume()
                            }
                        }
                    }

                    GjiState::OnWarm { long_idle_ms } => {
                        let ms = *long_idle_ms;
                        Response::emit_one(GjiAction::SendInputDirect(input))
                            .with_timer(GjiTimer::LongIdle, Duration::from_millis(ms))
                    }

                    GjiState::OnComposing { .. } => {
                        Response::emit_one(GjiAction::SendInputDirect(input))
                    }
                }
            },

            // ── WarmupComplete ─────────────────────────────────────────────
            GjiEvent::WarmupComplete { probe_id, result } => {
                // 現在 Authorized/Executing の probe_id と照合（stale 判定）
                let current_id = self.running_probe_id();
                if current_id != Some(probe_id) {
                    log::debug!(
                        "[gji-fsm] WarmupComplete {probe_id:?}: stale (current={current_id:?}), ignored"
                    );
                    return Response::consume();
                }
                // pending を take する（immutable borrow を解放してから）
                let pending = match &mut self.state {
                    GjiState::OnCold { pending, .. } => std::mem::take(pending),
                    _ => vec![],
                };
                let mut extra_actions = Vec::new();
                if !pending.is_empty() {
                    extra_actions.push(GjiAction::SendInput { result, pending });
                }
                self.transition_to_warm(extra_actions)
            }

            // ── WarmupFailed ───────────────────────────────────────────────
            GjiEvent::WarmupFailed { probe_id } => {
                let current_id = self.running_probe_id();
                if current_id != Some(probe_id) {
                    log::debug!(
                        "[gji-fsm] WarmupFailed {probe_id:?}: stale (current={current_id:?}), ignored"
                    );
                    return Response::consume();
                }
                log::warn!(
                    "[gji-fsm] WarmupFailed {probe_id:?}: budget exhausted, using conservative fallback"
                );
                let pending = match &mut self.state {
                    GjiState::OnCold { pending, .. } => std::mem::take(pending),
                    _ => vec![],
                };
                let result = WarmupResult::conservative_fallback();
                let mut extra_actions = Vec::new();
                if !pending.is_empty() {
                    extra_actions.push(GjiAction::SendInput { result, pending });
                }
                self.transition_to_warm(extra_actions)
            }

            // ── StartComposition ───────────────────────────────────────────
            GjiEvent::StartComposition => match &self.state {
                GjiState::OnWarm { .. } => self.transition_to_composing(vec![]),

                GjiState::OnComposing { .. } => {
                    log::debug!("[gji-fsm] StartComposition: already composing, ignored");
                    Response::consume()
                }

                GjiState::OnCold { .. } => {
                    // probe 実行中の StartComposition は GJI が既に warm である証拠。
                    // CancelProbe を出すと pending_tsf（GjiWarmupFsm）が破棄され、
                    // バッファされたロマ字が失われる（例：「こ」→「れ」バグ）。
                    // probe はキャンセルせず継続させ、次の TIMER_TSF_PROBE tick で
                    // ロマ字を送信させる。
                    if self.running_probe_id().is_some() {
                        log::debug!(
                            "[gji-fsm] StartComposition while cold (probe running) — probe continues"
                        );
                    } else {
                        log::debug!("[gji-fsm] StartComposition while cold (no probe)");
                    }
                    self.transition_to_composing(vec![])
                }

                GjiState::OffCold => {
                    log::warn!("[gji-fsm] StartComposition while engine off — ignored");
                    Response::consume()
                }
            },

            // ── EndComposition ─────────────────────────────────────────────
            GjiEvent::EndComposition { epoch } => match &self.state {
                GjiState::OnComposing {
                    epoch: current_epoch,
                } => {
                    if epoch == *current_epoch {
                        self.transition_to_warm(vec![])
                    } else {
                        log::debug!(
                            "[gji-fsm] EndComposition: stale epoch {epoch:?} ≠ {current_epoch:?}, ignored"
                        );
                        Response::consume()
                    }
                }
                _ => {
                    log::debug!(
                        "[gji-fsm] EndComposition: not composing ({}), ignored",
                        state_label(&self.state)
                    );
                    Response::consume()
                }
            },

            // ── NativeF2Consumed ───────────────────────────────────────────
            GjiEvent::NativeF2Consumed => {
                // Medium/Long cold 中は probe を継続し、saw_native_f2 フラグを立てる。
                // WezTerm が FocusChange 直後に自分で F2 を送る動作は probe に役立てられる。
                // Short cold / OnWarm / OnComposing は文脈破壊として CompositionReset 相当で処理する。
                let is_medium_or_long_cold = matches!(
                    &self.state,
                    GjiState::OnCold { kind, .. } if !kind.is_proactive()
                );
                if is_medium_or_long_cold {
                    if let GjiState::OnCold { saw_native_f2, kind, .. } = &mut self.state {
                        log::debug!(
                            "[gji-fsm] NativeF2Consumed: {kind:?} cold, probe continues (saw_native_f2=true)"
                        );
                        *saw_native_f2 = true;
                    }
                    Response::consume()
                } else {
                    log::debug!("[gji-fsm] NativeF2Consumed → CompositionReset (short/warm/composing)");
                    self.handle_composition_reset()
                }
            }

            // ── CompositionReset ───────────────────────────────────────────
            GjiEvent::CompositionReset => self.handle_composition_reset(),
        }
    }

    fn on_timeout(&mut self, timer_id: GjiTimer) -> Response<GjiAction, GjiTimer> {
        match timer_id {
            GjiTimer::LongIdle => match &self.state {
                GjiState::OnWarm { .. } => {
                    log::debug!("[gji-fsm] LongIdle timeout → OnCold(Long, NotStarted)");
                    self.state = GjiState::OnCold {
                        kind: ColdKind::Long,
                        probe: ProbeStatus::NotStarted,
                        pending: vec![],
                        saw_native_f2: false,
                    };
                    Response::consume()
                }
                _ => {
                    log::warn!(
                        "[gji-fsm] LongIdle timeout in unexpected state ({})",
                        state_label(&self.state)
                    );
                    Response::consume()
                }
            },
        }
    }
}

// ── ヘルパー関数 ──────────────────────────────────────────────────────────────

/// `InjectionMode` から `LongIdle` タイムアウト時間 (ms) を計算する。
///
/// - `Tsf`（WezTerm 等）: `LONG_IDLE_MS`（10 s）
/// - `Vk`（Chrome/Edge 等）: `CHROME_LONG_IDLE_MS`（5 s）
/// - `Unicode`（Win32 等）: `LONG_IDLE_MS`（保守的に長めに設定）
pub(crate) fn long_idle_ms_for(mode: InjectionMode) -> u64 {
    match mode {
        InjectionMode::Tsf => tuning::LONG_IDLE_MS,
        InjectionMode::Vk => tuning::CHROME_LONG_IDLE_MS,
        InjectionMode::Unicode => tuning::LONG_IDLE_MS,
    }
}

fn state_label(state: &GjiState) -> &'static str {
    match state {
        GjiState::OffCold => "OffCold",
        GjiState::OnCold {
            kind: ColdKind::Short,
            ..
        } => "OnCold(Short)",
        GjiState::OnCold {
            kind: ColdKind::Medium,
            ..
        } => "OnCold(Medium)",
        GjiState::OnCold {
            kind: ColdKind::Long,
            ..
        } => "OnCold(Long)",
        GjiState::OnWarm { .. } => "OnWarm",
        GjiState::OnComposing { .. } => "OnComposing",
    }
}

// ── ユニットテスト ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ime_on() -> GjiEvent {
        GjiEvent::ImeOn {
            injection_mode: InjectionMode::Vk,
            gji_idle_ms: 0,
        }
    }

    fn ime_on_with_idle(gji_idle_ms: u64) -> GjiEvent {
        GjiEvent::ImeOn {
            injection_mode: InjectionMode::Vk,
            gji_idle_ms,
        }
    }

    fn focus_change() -> GjiEvent {
        focus_change_with_idle(0)
    }

    fn focus_change_with_idle(gji_idle_ms: u64) -> GjiEvent {
        GjiEvent::FocusChange {
            injection_mode: InjectionMode::Vk,
            gji_idle_ms,
        }
    }

    fn complete(fsm: &GjiFsm) -> GjiEvent {
        let probe_id = match fsm.state() {
            GjiState::OnCold {
                probe: ProbeStatus::Authorized { probe_id, .. },
                ..
            } => *probe_id,
            s => panic!("expected OnCold(Authorized), got {}", state_label(s)),
        };
        GjiEvent::WarmupComplete {
            probe_id,
            result: WarmupResult {
                path: WarmupPath::NameChangeConfirmed,
                prepend_f2_warmup: false,
                nc_fired: true,
                gji_resumed: false,
            },
        }
    }

    fn failed(fsm: &GjiFsm) -> GjiEvent {
        let probe_id = match fsm.state() {
            GjiState::OnCold {
                probe: ProbeStatus::Authorized { probe_id, .. },
                ..
            } => *probe_id,
            s => panic!("expected OnCold(Authorized), got {}", state_label(s)),
        };
        GjiEvent::WarmupFailed { probe_id }
    }

    // ── ImeOn → OnCold(Short) ────────────────────────────────────────────

    #[test]
    fn ime_on_from_off_cold_starts_short_probe() {
        let mut fsm = GjiFsm::new();
        let r = fsm.on_event(ime_on());
        r.assert_consumed();
        r.assert_action_count(1);
        assert!(matches!(r.actions[0], GjiAction::StartProbe { .. }));
        assert!(matches!(fsm.state(), GjiState::OnCold { kind: ColdKind::Short, .. }));
    }

    #[test]
    fn ime_on_while_on_warm_is_ignored() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        // now OnWarm
        let r = fsm.on_event(ime_on());
        r.assert_consumed();
        r.assert_action_count(0); // ignored
        assert!(matches!(fsm.state(), GjiState::OnWarm { .. }));
    }

    // ── WarmupComplete → OnWarm ──────────────────────────────────────────

    #[test]
    fn warmup_complete_transitions_to_warm_with_timer() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        let r = fsm.on_event(ev);
        r.assert_consumed();
        r.assert_timer_set(GjiTimer::LongIdle);
        assert!(matches!(fsm.state(), GjiState::OnWarm { .. }));
    }

    #[test]
    fn warmup_complete_flushes_pending_input() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        fsm.on_event(GjiEvent::KeyInput(PendingInput::new("ka")));
        fsm.on_event(GjiEvent::KeyInput(PendingInput::new("na")));
        let ev = complete(&fsm);
        let r = fsm.on_event(ev);
        // SendInput action が存在するはず
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::SendInput { pending, .. } if pending.len() == 2)),
            "expected SendInput with 2 pending items, got {:?}", r.actions
        );
    }

    // ── WarmupFailed → OnWarm（conservative fallback）───────────────────

    #[test]
    fn warmup_failed_transitions_to_warm_with_fallback() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = failed(&fsm);
        let r = fsm.on_event(ev);
        r.assert_consumed();
        r.assert_timer_set(GjiTimer::LongIdle);
        assert!(matches!(fsm.state(), GjiState::OnWarm { .. }));
    }

    // ── probe_id stale 防止 ──────────────────────────────────────────────

    #[test]
    fn stale_warmup_complete_is_ignored() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        // 古い probe_id でイベントを作成
        let stale_probe_id = match fsm.state() {
            GjiState::OnCold { probe: ProbeStatus::Authorized { probe_id, .. }, .. } => *probe_id,
            _ => panic!(),
        };
        // FocusChange で probe を再起動
        fsm.on_event(focus_change());
        // 古い probe_id の Complete → 無視されるはず
        let r = fsm.on_event(GjiEvent::WarmupComplete {
            probe_id: stale_probe_id,
            result: WarmupResult::conservative_fallback(),
        });
        r.assert_consumed();
        r.assert_action_count(0);
        // まだ OnCold のまま
        assert!(matches!(fsm.state(), GjiState::OnCold { .. }));
    }

    // ── FocusChange ─────────────────────────────────────────────────────

    #[test]
    fn focus_change_while_warm_enters_cold_short() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        let r = fsm.on_event(focus_change());
        r.assert_consumed();
        r.assert_timer_kill(GjiTimer::LongIdle);
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::StartProbe { .. })),
            "expected StartProbe action"
        );
        assert!(matches!(fsm.state(), GjiState::OnCold { kind: ColdKind::Short, .. }));
    }

    #[test]
    fn focus_change_while_cold_cancels_old_probe_and_starts_new() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let r = fsm.on_event(focus_change());
        // CancelProbe(old) + StartProbe(new) の2アクション
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::CancelProbe { .. })),
            "expected CancelProbe"
        );
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::StartProbe { .. })),
            "expected StartProbe"
        );
    }

    #[test]
    fn focus_change_while_off_cold_is_noop() {
        let mut fsm = GjiFsm::new();
        let r = fsm.on_event(focus_change());
        r.assert_consumed();
        r.assert_action_count(0);
        assert!(matches!(fsm.state(), GjiState::OffCold));
    }

    // ── LongIdle タイムアウト ────────────────────────────────────────────

    #[test]
    fn long_idle_timeout_from_warm_enters_cold_long_not_started() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        let r = fsm.on_timeout(GjiTimer::LongIdle);
        r.assert_consumed();
        assert!(matches!(
            fsm.state(),
            GjiState::OnCold { kind: ColdKind::Long, probe: ProbeStatus::NotStarted, .. }
        ));
    }

    #[test]
    fn key_input_in_cold_long_not_started_starts_probe() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        fsm.on_timeout(GjiTimer::LongIdle);
        let r = fsm.on_event(GjiEvent::KeyInput(PendingInput::new("a")));
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::StartProbe { .. })),
            "expected StartProbe on first KeyInput in Long cold"
        );
        assert!(matches!(
            fsm.state(),
            GjiState::OnCold { probe: ProbeStatus::Authorized { .. }, .. }
        ));
    }

    // ── ColdKind::Medium + NativeF2Consumed ─────────────────────────────

    #[test]
    fn focus_change_medium_idle_enters_cold_medium_not_started() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        // medium idle: 7000ms ≤ gji_idle < 10000ms → ColdKind::Medium, NotStarted
        let r = fsm.on_event(focus_change_with_idle(8_000));
        // NotStarted なので StartProbe アクションなし（pending_tsf のみ設定）
        assert!(
            !r.actions.iter().any(|a| matches!(a, GjiAction::StartProbe { .. })),
            "Medium cold は即 probe を開始しない（KeyInput まで NotStarted）"
        );
        assert!(matches!(
            fsm.state(),
            GjiState::OnCold { kind: ColdKind::Medium, probe: ProbeStatus::NotStarted, .. }
        ));
    }

    #[test]
    fn focus_change_long_idle_enters_cold_long_not_started() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        let r = fsm.on_event(focus_change_with_idle(12_000));
        assert!(
            !r.actions.iter().any(|a| matches!(a, GjiAction::StartProbe { .. })),
            "Long cold は即 probe を開始しない"
        );
        assert!(matches!(
            fsm.state(),
            GjiState::OnCold { kind: ColdKind::Long, probe: ProbeStatus::NotStarted, .. }
        ));
    }

    #[test]
    fn medium_cold_key_input_starts_probe_with_ncwait_budget() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        fsm.on_event(focus_change_with_idle(8_000));
        let r = fsm.on_event(GjiEvent::KeyInput(PendingInput::new("ka")));
        let probe_action = r.actions.iter().find(|a| matches!(a, GjiAction::StartProbe { .. }));
        assert!(probe_action.is_some(), "Medium cold: KeyInput で StartProbe が必要");
        if let Some(GjiAction::StartProbe { params, .. }) = probe_action {
            assert_eq!(
                params.ncwait_budget_ms, crate::tuning::MEDIUM_IDLE_PROBE_TOTAL_MS,
                "Medium cold: ncwait_budget_ms = MEDIUM_IDLE_PROBE_TOTAL_MS"
            );
            assert!(params.forces_prepend_f2, "Medium cold: forces_prepend_f2=true");
        }
    }

    #[test]
    fn long_cold_key_input_starts_probe_with_long_ncwait_budget() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        // LongIdle タイムアウトから Long cold に入る
        fsm.on_timeout(GjiTimer::LongIdle);
        let r = fsm.on_event(GjiEvent::KeyInput(PendingInput::new("a")));
        if let Some(GjiAction::StartProbe { params, .. }) =
            r.actions.iter().find(|a| matches!(a, GjiAction::StartProbe { .. }))
        {
            assert_eq!(
                params.ncwait_budget_ms, crate::tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS,
                "Long cold: ncwait_budget_ms = GJI_LONG_IDLE_PROBE_TOTAL_MS"
            );
            assert!(params.forces_prepend_f2, "Long cold: forces_prepend_f2=true");
        } else {
            panic!("Long cold KeyInput: StartProbe が必要");
        }
    }

    #[test]
    fn short_cold_starts_probe_with_short_ncwait_budget() {
        let mut fsm = GjiFsm::new();
        // ImeOn → OnCold(Short) で即 StartProbe
        let r = fsm.on_event(ime_on());
        if let Some(GjiAction::StartProbe { params, .. }) =
            r.actions.iter().find(|a| matches!(a, GjiAction::StartProbe { .. }))
        {
            assert_eq!(
                params.ncwait_budget_ms, crate::tuning::SETTLE_TIMEOUT_MS,
                "Short cold: ncwait_budget_ms = SETTLE_TIMEOUT_MS"
            );
            assert!(!params.forces_prepend_f2, "Short cold: forces_prepend_f2=false");
        } else {
            panic!("ImeOn: StartProbe が必要");
        }
    }

    #[test]
    fn native_f2_consumed_while_medium_cold_continues_probe() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        fsm.on_event(focus_change_with_idle(8_000));
        // NativeF2Consumed → probe 継続、saw_native_f2=true
        let r = fsm.on_event(GjiEvent::NativeF2Consumed);
        r.assert_consumed();
        r.assert_action_count(0);
        // まだ OnCold(Medium, NotStarted) のまま
        assert!(matches!(
            fsm.state(),
            GjiState::OnCold { kind: ColdKind::Medium, probe: ProbeStatus::NotStarted, saw_native_f2: true, .. }
        ));
    }

    #[test]
    fn native_f2_consumed_while_short_cold_resets_probe() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on()); // → OnCold(Short, Authorized)
        // NativeF2Consumed → CompositionReset 相当（CancelProbe + NotStarted）
        let r = fsm.on_event(GjiEvent::NativeF2Consumed);
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::CancelProbe { .. })),
            "Short cold: NativeF2Consumed → CancelProbe が必要"
        );
        assert!(matches!(
            fsm.state(),
            GjiState::OnCold { kind: ColdKind::Short, probe: ProbeStatus::NotStarted, .. }
        ));
    }

    // ── StartComposition / EndComposition ────────────────────────────────

    /// 回帰テスト: OnCold(Authorized) 中に StartComposition が来ても probe をキャンセルしない。
    ///
    /// WezTerm で「こ」→「れでいいか」と化けるバグの再現シナリオ:
    /// 1. ImeOn → OnCold(Short, Authorized)
    /// 2. キー入力 → GjiWarmupFsm が pending_tsf にセットされる（このテストでは FSM 外部）
    /// 3. eager F2 から来た StartComposition がキューから drain される
    ///    → CancelProbe を出して GjiWarmupFsm を破壊してはいけない
    #[test]
    fn start_composition_while_cold_does_not_cancel_probe() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on()); // → OnCold(Short, Authorized, probe_id=0)

        let probe_id_before = match fsm.state() {
            GjiState::OnCold { probe: ProbeStatus::Authorized { probe_id, .. }, .. } => *probe_id,
            _ => panic!("expected OnCold(Authorized)"),
        };

        let r = fsm.on_event(GjiEvent::StartComposition);

        // CancelProbe を出してはいけない
        assert!(
            !r.actions.iter().any(|a| matches!(a, GjiAction::CancelProbe { .. })),
            "StartComposition while cold must NOT emit CancelProbe (would destroy GjiWarmupFsm)"
        );
        // OnComposing に遷移しているはず
        assert!(
            matches!(fsm.state(), GjiState::OnComposing { .. }),
            "expected OnComposing after StartComposition while cold"
        );
        // probe_id は外部の current_gji_probe_id に記録されているので FSM では追跡しない
        let _ = probe_id_before;
    }

    #[test]
    fn start_then_end_composition_returns_to_warm() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        fsm.on_event(GjiEvent::StartComposition);
        assert!(matches!(fsm.state(), GjiState::OnComposing { .. }));

        let epoch = match fsm.state() {
            GjiState::OnComposing { epoch } => *epoch,
            _ => panic!(),
        };
        let r = fsm.on_event(GjiEvent::EndComposition { epoch });
        r.assert_consumed();
        r.assert_timer_set(GjiTimer::LongIdle);
        assert!(matches!(fsm.state(), GjiState::OnWarm { .. }));
    }

    #[test]
    fn stale_end_composition_is_ignored() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        fsm.on_event(GjiEvent::StartComposition);
        // FocusChange で epoch が進む
        fsm.on_event(focus_change());
        let ev2 = complete(&fsm);
        fsm.on_event(ev2);
        fsm.on_event(GjiEvent::StartComposition);

        // 古い epoch で EndComposition → 無視
        let r = fsm.on_event(GjiEvent::EndComposition {
            epoch: FocusEpoch(0),
        });
        r.assert_consumed();
        r.assert_action_count(0);
        assert!(matches!(fsm.state(), GjiState::OnComposing { .. }));
    }

    // ── ImeOff ──────────────────────────────────────────────────────────

    #[test]
    fn ime_off_from_warm_enters_off_cold() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        let r = fsm.on_event(GjiEvent::ImeOff);
        r.assert_consumed();
        r.assert_timer_kill(GjiTimer::LongIdle);
        assert!(matches!(fsm.state(), GjiState::OffCold));
    }

    #[test]
    fn ime_off_cancels_running_probe() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let r = fsm.on_event(GjiEvent::ImeOff);
        assert!(
            r.actions.iter().any(|a| matches!(a, GjiAction::CancelProbe { .. })),
            "expected CancelProbe on ImeOff while cold"
        );
        assert!(matches!(fsm.state(), GjiState::OffCold));
    }

    // ── ImeOn with long idle → proactive Long probe (regression: IME Off Engine ON) ──

    #[test]
    fn ime_on_with_long_idle_uses_long_probe_proactively() {
        // WezTerm で F2 を押した際に gji_idle > LONG_IDLE_MS なら
        // forces_prepend_f2=true の probe を即開始する（FocusChange とは異なり NotStarted にしない）。
        let mut fsm = GjiFsm::new();
        let r = fsm.on_event(ime_on_with_idle(crate::tuning::LONG_IDLE_MS + 1000));
        let probe_action = r.actions.iter().find(|a| matches!(a, GjiAction::StartProbe { .. }));
        assert!(probe_action.is_some(), "long idle ImeOn: StartProbe を即開始すべき");
        if let Some(GjiAction::StartProbe { params, .. }) = probe_action {
            assert!(params.forces_prepend_f2, "long idle ImeOn: forces_prepend_f2=true が必要");
            assert!(params.is_long_cold, "long idle ImeOn: is_long_cold=true が必要");
        }
        assert!(
            matches!(fsm.state(), GjiState::OnCold { kind: ColdKind::Long, probe: ProbeStatus::Authorized { .. }, .. }),
            "long idle ImeOn → OnCold(Long, Authorized)（NotStarted ではない）"
        );
    }

    #[test]
    fn ime_on_with_medium_idle_uses_medium_probe_proactively() {
        let mut fsm = GjiFsm::new();
        let r = fsm.on_event(ime_on_with_idle(crate::tuning::MEDIUM_IDLE_PROBE_MS + 500));
        let probe_action = r.actions.iter().find(|a| matches!(a, GjiAction::StartProbe { .. }));
        assert!(probe_action.is_some(), "medium idle ImeOn: StartProbe を即開始すべき");
        if let Some(GjiAction::StartProbe { params, .. }) = probe_action {
            assert!(params.forces_prepend_f2, "medium idle ImeOn: forces_prepend_f2=true が必要");
        }
        assert!(
            matches!(fsm.state(), GjiState::OnCold { kind: ColdKind::Medium, probe: ProbeStatus::Authorized { .. }, .. }),
            "medium idle ImeOn → OnCold(Medium, Authorized)"
        );
    }

    // ── KeyInput in OnWarm → timer reset ─────────────────────────────────

    #[test]
    fn key_input_in_warm_sends_direct_and_resets_timer() {
        let mut fsm = GjiFsm::new();
        fsm.on_event(ime_on());
        let ev = complete(&fsm);
        fsm.on_event(ev);
        let r = fsm.on_event(GjiEvent::KeyInput(PendingInput::new("a")));
        assert!(matches!(r.actions[0], GjiAction::SendInputDirect(_)));
        r.assert_timer_set(GjiTimer::LongIdle);
    }
}
