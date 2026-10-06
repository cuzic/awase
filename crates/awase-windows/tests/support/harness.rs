//! 閉ループのハーネス: 擬似 IME と awase の純粋な状態遷移層をつなぎ、
//! キー・観測・フォーカス・時刻の進行を流し込んで「書き込み命令」を集める。
//!
//! 実際に呼ぶ awase の層（すべて Linux ホストで動く ungated なもの）:
//! - `ImeStateHub`（本物。`with_clock(HubClock::manual(..))` で仮想時計）: `dispatch_event`（= `ImeModel::reduce` と
//!   `event_log`・`journal`）、`effective_open_at`、`record_explicit_intent`、`apply_key_effect_prediction`、
//!   `warrant_context`、`arm/follow_external_change_in_scope`、`IntentStore`・`ExternalChangeWatch` はハブが持つ
//! - `KeyEffectKeymap::predict`（GJI ATOK プリセット、同梱表）
//! - `open_warrant::issue_open_warrant`
//! - `drift_correction::check_drift_correction`（旧 `ImeStateHub::check_drift_correction` の本体）
//! - `awase::engine::Engine`（`EngineCommand::RefreshState`/`FocusChanged` の活性遷移と `SetOpen`）
//!
//! Windows 専用（`#[cfg(windows)]`）で呼べないため、**数行の配線をここで写している**もの
//! （写し元の行は各メソッドの doc に書く。写し元が変わったらここも直すこと）:
//! - `ImeStateHub::issue_actuation_order`/`write_*`（`state/platform_state.rs`。`warrant_context` までは本物）
//! - `ImeStateHub::align_placeholder_desired`（`settle` 内。本物は `pub(crate)`。次の段階で本物にする候補）
//! - `kp_stage_key_effect_track`/`kp_predict_key_effect`（`runtime/key_pipeline.rs`）
//! - `ir_apply_drift_correction` の「検知へ進むか」まで（`runtime/ime_refresh.rs`）: `check_drift_correction` と、
//!   ImmCross で warrant が下りない補正を検知の手前で見送る早期 return（BUG-163 の1段目、`b6ab8980`）。
//!   Blind/Read の再送打ち切り・settle 待ち・conv ラッチは写していない。
//!
//! - ADR-205 の外部変化の監視窓は本物（`arm/follow_external_change_in_scope`、追随の副作用もハブの中）を呼ぶ。
//!   `Setup::with_external_close_watch(true)` のときだけ働く（ADR-205 の有無で結果が分かれるシナリオ用）。
//!
//! 写していない（このハーネスでは起きない）もの: 通過マーク（ADR-187 `ModeKeyPassLatch` →
//! `ModeKeyPassedThrough`）、`ImeApplyRequested`/`applied` の往復、ForceGuard、TSF warmup、
//! Engine の `on_input`（打鍵のかな変換。IME 側には文字キーをそのまま渡す）。

use std::time::Instant;

use awase::config::ConfirmMode;
use awase::engine::{
    Decision, Effect, Engine, EngineCommand, ImeEffect, InputContext, InputModeState, NicolaFsm,
    SpecialKeyCombos,
};
use awase::scanmap::KeyboardModel;
use awase::types::VkCode;
use awase::yab::YabLayout;
use awase_windows::state::conv_classify::ConvSyncReason;
use awase_windows::state::drift_correction::{check_drift_correction, DriftCorrection};
use awase_windows::state::evidence::{ConvOpenInference, ImmCrossProbe, Observed, ObserverPoll};
use awase_windows::state::foreground_scope::ForegroundScope;
use awase_windows::state::hub_clock::HubClock;
use awase_windows::state::ime_event::{
    HwndId, ImeEvent, ImePolicyProfile, ObservationConfidence, ObservationSource, UserIntentSource,
};
use awase_windows::state::ime_model::ImeModel;
use awase_windows::state::key_effect_predictor::{KeyEffectKeymap, PredictInput, Prediction};
use awase_windows::state::open_warrant::{issue_open_warrant, OpenWarrant};
use awase_windows::state::platform_state::ImeStateHub;
use awase_windows::state::probe_admission::{Admission, FocusFence, ImmLikeTicket};
use awase_windows::state::TickMs;

use super::pseudo_ime::{Grid, PseudoIme, TrueState, CONV_ALNUM};

/// 本番の `GetTickCount64` に相当する tick の起点（0 付近だと TTL 計算が境界に寄るため）。
const TICK_BASE: u64 = 1_000_000;

/// 観測の経路（ハーネスが生成できるもの）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `ImmCrossProbe`（High）。`write_imm_cross_probe` 相当。
    ImmCross,
    /// `ObserverPoll`（Medium）。`apply_ime_update` 相当。
    Poll,
    /// `ConvOpenInference`（Medium、conv ビットからの間接推測。`classify_conv_transition` の
    /// `ReportOpenInference`）。GJI×TsfNative では IME を閉じても conv の NATIVE が残るため誤って「開」と推測する。
    ConvInference,
}

/// 書き込み命令の出所。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOrigin {
    /// drift correction（`ir_apply_drift_correction`）。
    DriftCorrection,
    /// ユーザーの明示操作（IME ON/OFF コンボ等）で awase が送る書き込み。
    ExplicitUserCommand,
}

/// 集めた書き込み命令1件。
#[derive(Debug, Clone)]
pub struct WriteCommand {
    pub step: usize,
    pub at_ms: u64,
    pub origin: WriteOrigin,
    pub open: bool,
    /// `issue_open_warrant()` の結果。`Some` なら本番は実際に書く（A-2）。
    pub warrant: Option<OpenWarrant>,
    /// 起案した時点で明示意図（`last_intent` または `IntentStore` の有効なエントリ）があったか。
    pub explicit_intent: bool,
    /// 起案した時点の擬似 IME の真の開閉。
    pub truth_open: bool,
    /// 擬似 IME に実際に効いたか（warrant が下り、かつ書き込みが塞がれていない）。
    pub applied: bool,
}

/// `check_drift_correction` が補正を要すると判定した1件。
#[derive(Debug, Clone)]
pub struct DriftFire {
    pub step: usize,
    pub at_ms: u64,
    pub drift: DriftCorrection,
    pub explicit_intent: bool,
    /// 補正の書き込みに warrant が下りたか。
    pub warranted: bool,
    /// `ir_apply_drift_correction` が「検知」（journal・`DriftDetected`・送信）まで進むか。
    /// ImmCross で warrant が下りない補正は、その手前で見送る（BUG-163 の1段目、`b6ab8980`）。
    pub detected: bool,
}

/// 打鍵1回の予測の記録。
#[derive(Debug, Clone)]
pub struct PredictionRecord {
    pub step: usize,
    pub vk: u16,
    /// 予測の入力（打鍵前の belief）。
    pub input_open: bool,
    pub input_mode: InputModeState,
    pub prediction: Option<Prediction>,
    pub truth_before: TrueState,
    pub truth_after: TrueState,
}

/// ステップ1回の後の状態（検査用）。
#[derive(Debug, Clone)]
pub struct StepRecord {
    pub step: usize,
    pub at_ms: u64,
    pub label: String,
    pub truth: TrueState,
    pub desired_open: bool,
    pub effective_open: bool,
    pub input_mode: InputModeState,
    pub engine_active: bool,
    pub explicit_intent: bool,
    /// このステップが成功した観測（`Source` から開閉を読めた）だったか。値は観測した開閉。
    pub observed_open: Option<bool>,
}

/// シナリオの設定。
#[derive(Debug, Clone, Copy)]
pub struct Setup {
    pub initial: TrueState,
    /// 起動時のアプリのプロファイル（`InitialAppPolicyEstablished`）。
    pub profile: ImePolicyProfile,
    /// TSF の composition（入力中）を観測できるアプリか。`PredictInput::composing` に効く。
    pub composing_visible: bool,
    /// 擬似 IME の真値にする格子と、awase の予測器が引く同梱表（`session_keymap`）。既定は ATOK。
    pub grid: Grid,
    /// ADR-205（外部クローズの監視窓）が有効か。無効なら Q4（外部注入の閉じが観測されない）のまま。
    pub external_close_watch: bool,
}

impl Setup {
    /// ImmCross（開閉・conv を読めるアプリ）で、指定した真の初期状態から起動する。
    #[must_use]
    pub const fn imm_cross(initial: TrueState) -> Self {
        Self {
            initial,
            profile: ImePolicyProfile::ImmCross,
            composing_visible: true,
            grid: Grid::Atok,
            external_close_watch: false,
        }
    }

    /// 読めない窓（`Imm32Unavailable`、実 Chrome 相当）で、指定した真の初期状態から起動する。
    #[must_use]
    pub const fn imm32_unavailable(initial: TrueState) -> Self {
        let mut s = Self::imm_cross(initial);
        s.profile = ImePolicyProfile::Imm32Unavailable;
        s
    }

    /// ADR-205 の監視窓を有効にする（無効のままなら修正前の挙動＝Q4）。
    #[must_use]
    pub const fn with_external_close_watch(mut self, on: bool) -> Self {
        self.external_close_watch = on;
        self
    }

    /// 擬似 IME の格子（と予測器の同梱表）を変える。
    #[must_use]
    pub const fn with_grid(mut self, grid: Grid) -> Self {
        self.grid = grid;
        self
    }
}

/// 閉ループのハーネス本体。
pub struct Harness {
    pub ime: PseudoIme,
    /// 本物の `ImeStateHub`（`ImeModel`・`IntentStore`・`ExternalChangeWatch`・仮想時計を持つ）。
    hub: ImeStateHub,
    engine: Engine,
    keymap: KeyEffectKeymap,
    setup: Setup,
    now_ms: u64,
    focus: HwndId,
    epoch: u64,
    /// 直近に観測した conv の生値（`ImeBelief::prev_conversion_mode` 相当）。
    last_conv_raw: Option<u32>,
    step: usize,
    pub steps: Vec<StepRecord>,
    pub writes: Vec<WriteCommand>,
    pub drift_fires: Vec<DriftFire>,
    pub predictions: Vec<PredictionRecord>,
    /// 本番の `is_japanese_ime`（ADR-223 でキー入力時にレイアウト言語から更新される）。既定 true。
    japanese_ime: bool,
}

/// ハーネスは単一のフォアグラウンドなのでスコープは固定。
const WATCH_SCOPE: ForegroundScope = ForegroundScope {
    pid: 1,
    hwnd: 0x1001,
};

impl Harness {
    /// awase を起動した直後の状態を作る（`ImeModel::new()`、観測なし、明示意図なし）。
    ///
    /// 起動時のイベントは本番の bootstrap と同じ3つ（fence・hwnd・policy）だけを流す。
    /// Engine の直前の活性状態は起動時の文脈から計算して合わせる（起動の瞬間に
    /// 活性遷移の `SetOpen` が出ないように。本番の初期化と同じく「遷移」を作らない）。
    #[must_use]
    pub fn start(setup: Setup) -> Self {
        let mut h = Self {
            ime: PseudoIme::from_grid(setup.grid, setup.initial),
            hub: ImeStateHub::with_clock(HubClock::manual(TICK_BASE)),
            engine: make_engine(),
            keymap: match setup.grid {
                Grid::Atok => KeyEffectKeymap::from_config(Some(1), None, &[]),
                Grid::GjiMsime => KeyEffectKeymap::from_config(Some(2), None, &[]),
                // 本番は割り当て設定を読む。ここでは既定（割り当て未設定・互換モード不明）。
                Grid::MsimeNative => {
                    Some(KeyEffectKeymap::for_msime_native(false, None, None, None))
                }
            }
            .expect("同梱表のあるキーマップ"),
            setup,
            now_ms: 0,
            focus: HwndId(0x1001),
            epoch: 1,
            last_conv_raw: None,
            step: 0,
            steps: Vec::new(),
            writes: Vec::new(),
            drift_fires: Vec::new(),
            predictions: Vec::new(),
            japanese_ime: true,
        };
        h.hub.set_is_japanese_ime(true);
        let fence = h.fence();
        h.reduce(ImeEvent::InitialFocusFenceEstablished { fence });
        h.reduce(ImeEvent::InitialFocusHwndEstablished { hwnd: h.focus });
        h.reduce(ImeEvent::InitialAppPolicyEstablished {
            profile: setup.profile,
        });
        let ctx = h.ctx();
        let active = h.engine.compute_active(&ctx);
        h.engine.set_prev_active(active);
        h.record_step("start".into(), None);
        h
    }

    // ── DSL ──────────────────────────────────────────────────────────────

    /// 時刻を進める（その後、drift 判定と Engine の再評価を1回行う）。
    pub fn advance_ms(&mut self, ms: u64) -> &mut Self {
        self.now_ms += ms;
        self.hub.advance_clock_ms(ms);
        self.ime.advance(ms);
        self.settle(format!("advance_ms({ms})"), None);
        self
    }

    /// 物理キー1回（awase が消費せず IME へ通した、修飾なしの KeyDown）。
    ///
    /// 本番の順序（`kp_stage_key_effect_track` → `kp_predict_key_effect`）: 打鍵前の belief で予測し、
    /// 生キーが IME に届き、予測を belief へ反映し、Engine を再評価する。
    pub fn key(&mut self, vk: u16) -> &mut Self {
        let truth_before = self.ime.state();
        let input = PredictInput {
            open: self.effective_open(),
            mode: self.hub.model().input_mode(),
            conv_raw: self.last_conv_raw,
            composing: self.setup.composing_visible && self.ime.state().open && {
                !matches!(truth_before.stage, super::pseudo_ime::TrueStage::None)
            },
            track: self.hub.model().key_track(),
            unreadable: false,
            // 物理の無修飾の KeyDown で、エンジンが消費せず IME へ通した打鍵（上のコメント）なので、ゲートは真。
            passive_rule_eligible: true,
        };
        let prediction = self.keymap.predict(vk, &input);
        let press = self.ime.press(vk);
        if let Some(p) = prediction {
            self.apply_key_effect_prediction(p);
        }
        self.predictions.push(PredictionRecord {
            step: self.step + 1,
            vk,
            input_open: input.open,
            input_mode: input.mode,
            prediction,
            truth_before: press.before,
            truth_after: press.after,
        });
        self.settle(format!("key(0x{vk:02X})"), None);
        self
    }

    /// 擬似 IME の真の状態を `source` で観測する（読める窓の probe/poll 相当）。
    pub fn observe(&mut self, source: Source) -> &mut Self {
        let seen = self.ime.read_state();
        self.observe_value(source, seen.open)
    }

    /// 開閉を明示した観測（観測が嘘をつく・古い状態を読む、の模擬に使う）。
    /// conv は擬似 IME の真の値を報告する（開いているときだけ入力モードを観測する）。
    pub fn observe_value(&mut self, source: Source, open: bool) -> &mut Self {
        let fence = self.fence();
        let Admission::Accept(accepted) = (ImmLikeTicket { fence }).admit(fence) else {
            unreachable!("同じ fence なので必ず受理される");
        };
        let (any, obs_source, confidence) = match source {
            Source::ImmCross => (
                Observed::<ImmCrossProbe>::from_cross_probe(&accepted, open).into(),
                ObservationSource::ImmCrossProbe,
                ObservationConfidence::High,
            ),
            Source::Poll => (
                Observed::<ObserverPoll>::from_poll(&accepted, open).into(),
                ObservationSource::ObserverPoll,
                ObservationConfidence::Medium,
            ),
            Source::ConvInference => (
                Observed::<ConvOpenInference>::from_conv(
                    ConvSyncReason::NativeToggleShadowOff,
                    open,
                    self.focus,
                    self.epoch,
                )
                .into(),
                ObservationSource::ConvOpenInference,
                ObservationConfidence::Medium,
            ),
        };
        self.reduce(ImeEvent::ObserverReported(any));
        let conv = self.ime.read_state().conv;
        self.last_conv_raw = Some(conv);
        if open {
            let mode = if conv == CONV_ALNUM {
                InputModeState::ObservedEisu
            } else {
                InputModeState::ObservedRomaji
            };
            self.reduce(ImeEvent::InputModeObserved {
                mode,
                source: obs_source,
                confidence,
                at: TickMs(self.tick()),
            });
        }
        self.settle(format!("observe({source:?}, open={open})"), Some(open));
        self
    }

    /// 前面のアプリが変わる（別プロセスの窓へ）。擬似 IME の状態は共有のまま。
    pub fn focus_change(&mut self) -> &mut Self {
        let from = self.focus;
        self.epoch += 1;
        self.focus = HwndId(self.focus.0 + 1);
        self.reduce(ImeEvent::FocusChanged {
            from: Some(from),
            to: self.focus,
            profile: self.setup.profile,
            focus_epoch: self.epoch,
        });
        let ctx = self.ctx();
        let decision = self.engine.on_command(EngineCommand::FocusChanged, &ctx);
        self.handle_engine_decision(&decision);
        self.settle("focus_change()".into(), None);
        self
    }

    /// ユーザーの明示操作（IME ON/OFF コンボ等、`UserIntentSource::Command`）で開閉を指定する。
    /// awase は明示意図を記録し、IME へ書く（`handle_engine_set_open` + `record_explicit_intent`）。
    pub fn user_set_open(&mut self, open: bool) -> &mut Self {
        self.reduce(ImeEvent::UserImeSetIntent {
            target: open,
            source: UserIntentSource::Command,
        });
        let now = TickMs(self.tick());
        self.hub
            .record_explicit_intent(open, UserIntentSource::Command, now);
        self.issue_write(WriteOrigin::ExplicitUserCommand, open);
        self.settle(format!("user_set_open({open})"), None);
        self
    }

    /// 擬似 IME が awase の書き込みを無視するか（書き込みが効かないアプリの模擬）。
    pub fn block_writes(&mut self, blocked: bool) -> &mut Self {
        self.ime.set_writes_blocked(blocked);
        self
    }

    /// awase の見ていない経路で IME の開閉が変わる（言語バーのマウス操作・他アプリの書き込み等）。
    /// awase は打鍵も観測も受けない（belief は次の観測まで古いまま）。
    pub fn external_set_open(&mut self, open: bool) -> &mut Self {
        self.ime.external_set_open(open);
        self.settle(format!("external_set_open({open})"), None);
        self
    }

    /// 他プロセスが目印なしで注入した IME キー（Q4 の外部クローズ）。キーは擬似 IME に届くが、awase は belief を
    /// 動かさない（BUG-14 分岐: 注入キーはユーザー意図に昇格しない）。ADR-205 有効なら監視窓を開く／延ばす
    /// （`ImeStateHub::arm_external_change_watch_in_scope`、本物）。
    pub fn external_injected_key(&mut self, vk: u16) -> &mut Self {
        self.ime.press(vk);
        if self.setup.external_close_watch {
            self.hub
                .arm_external_change_watch_in_scope(self.now_ms, WATCH_SCOPE);
        }
        self.settle(format!("external_injected_key(0x{vk:02X})"), None);
        self
    }

    /// refresh の入口で prefetch 済みの開閉の読み（`IMC_GETOPENSTATUS`）を取り込む。読めない窓では通常の観測
    /// （`ObserverPoll`）は belief へ届かない（Blacklist 分岐が捨てる）ので、ADR-205 の監視窓だけが取り込み口。
    /// `ImeStateHub::follow_external_change_in_scope`（本物）: `Changed(v)` なら `ObserverPoll(v)` 記録 → 明示意図
    /// （`IntentStore`）削除 → `ModeKeyPassedThrough{align_desired, demote_applied}` をハブの中で行う。
    pub fn prefetch_read(&mut self) -> &mut Self {
        assert!(
            matches!(self.setup.profile, ImePolicyProfile::Imm32Unavailable),
            "prefetch_read は読めない窓（Imm32Unavailable）用"
        );
        let read = Some(self.ime.read_state().open);
        if self.setup.external_close_watch {
            let fence = self.fence();
            let Admission::Accept(accepted) = (ImmLikeTicket { fence }).admit(fence) else {
                unreachable!("同じ fence なので必ず受理される");
            };
            let _followed = self.hub.follow_external_change_in_scope(
                read,
                self.now_ms,
                TickMs(self.tick()),
                accepted,
                WATCH_SCOPE,
            );
        }
        self.settle("prefetch_read".into(), None);
        self
    }

    // ── 読み取り ─────────────────────────────────────────────────────────

    #[must_use]
    pub fn desired_open(&self) -> bool {
        self.hub.model().desired_open()
    }

    #[must_use]
    pub fn model(&self) -> &ImeModel {
        self.hub.model()
    }

    /// 経過を人が読める形で（失敗メッセージ用）。
    #[must_use]
    pub fn trace(&self) -> String {
        let mut out = String::new();
        for s in &self.steps {
            out.push_str(&format!(
                "  #{:<2} t={:>5}ms {:<32} truth_after(open={} conv=0x{:02X} {:?}) desired={} eff_open={} mode={:?} engine={} intent={}\n",
                s.step,
                s.at_ms,
                s.label,
                s.truth.open,
                s.truth.conv,
                s.truth.stage,
                s.desired_open,
                s.effective_open,
                s.input_mode,
                if s.engine_active { "active" } else { "inactive" },
                s.explicit_intent,
            ));
            for w in self.writes.iter().filter(|w| w.step == s.step) {
                out.push_str(&format!(
                    "        write {:?} open={} warrant={:?} explicit_intent={} truth_open(書く前)={} applied={}\n",
                    w.origin,
                    w.open,
                    w.warrant.as_ref().map(|w| &w.basis),
                    w.explicit_intent,
                    w.truth_open,
                    w.applied
                ));
            }
            for d in self.drift_fires.iter().filter(|d| d.step == s.step) {
                out.push_str(&format!(
                    "        drift {:?} warranted={} detected={}\n",
                    d.drift, d.warranted, d.detected
                ));
            }
            for p in self.predictions.iter().filter(|p| p.step == s.step) {
                out.push_str(&format!("        predict {:?}\n", p.prediction));
            }
        }
        out
    }

    // ── 内部 ─────────────────────────────────────────────────────────────

    const fn fence(&self) -> FocusFence {
        FocusFence {
            epoch: self.epoch,
            hwnd: self.focus,
        }
    }

    fn tick(&self) -> u64 {
        debug_assert_eq!(
            self.hub.clock().now_tick(),
            TICK_BASE + self.now_ms,
            "ハブの仮想時計とハーネスの経過時間がずれた"
        );
        TICK_BASE + self.now_ms
    }

    /// ハブの仮想時計の現在の `Instant`（`advance_ms` で進める）。
    fn now(&self) -> Instant {
        self.hub.clock().now_instant()
    }

    /// 本物の `ImeStateHub::dispatch_event`（`event_log` 記録 + `ImeModel::reduce` + journal）。
    fn reduce(&mut self, event: ImeEvent) {
        self.hub.dispatch_event(event, TickMs(self.tick()));
    }

    /// 本物の `ImeStateHub::effective_open_at`（`IntentStore` の有効な明示意図が `ImeModel` の belief より優先する）。
    fn effective_open(&self) -> bool {
        self.hub.effective_open_at(TickMs(self.tick()))
    }

    /// `IntentStore` はハブの private なので、`warrant_context` の `intent_store`（読み取り専用の参照）経由で読む。
    fn has_explicit_intent(&self) -> bool {
        let now_ms = TickMs(self.tick());
        self.hub.model().last_intent.is_some()
            || self
                .hub
                .model()
                .current_focus()
                .and_then(|h| {
                    self.hub
                        .warrant_context(self.now(), now_ms)
                        .intent_store
                        .lookup(h, now_ms)
                })
                .is_some()
    }

    fn ctx(&self) -> InputContext {
        InputContext {
            ime_on: self.effective_open(),
            input_mode: self.hub.model().input_mode(),
            is_japanese_ime: self.japanese_ime,
            composing: false,
            modifiers: awase::engine::ModifierState::default(),
            left_thumb_down: None,
            right_thumb_down: None,
        }
    }

    /// 本番の `is_japanese_ime`(ADR-223: キー入力時にフォーカス窓のレイアウト言語から更新)を切り替える。既定は true。
    /// false のとき Engine は非活性になり、推測に基づく書き込み(`issue_open_warrant`)は下りない(明示キー押下の order は影響を受けない)。
    pub fn set_japanese_ime(&mut self, on: bool) -> &mut Self {
        self.japanese_ime = on;
        self.hub.set_is_japanese_ime(on);
        self
    }

    /// 本物の `ImeStateHub::apply_key_effect_prediction`。
    fn apply_key_effect_prediction(&mut self, p: Prediction) {
        self.hub.apply_key_effect_prediction(p, TickMs(self.tick()));
    }

    /// `ImeStateHub::warrant_context`（本物）+ `issue_open_warrant`（`issue_actuation_order` の写し）。
    fn warrant_for(&self, open: bool) -> Option<OpenWarrant> {
        let ctx = self.hub.warrant_context(self.now(), TickMs(self.tick()));
        let target = self.hub.model().current_focus().unwrap_or(HwndId::NULL);
        issue_open_warrant(open, target, &ctx)
    }

    /// 書き込み命令を起案する（`ImeStateHub::issue_actuation_order` の写し）。
    /// warrant が下りたら擬似 IME へ書く（A-2: warrant 無しの書き込みは止まる）。
    fn issue_write(&mut self, origin: WriteOrigin, open: bool) {
        let warrant = self.warrant_for(open);
        let truth_open = self.ime.state().open;
        let explicit_intent = self.has_explicit_intent();
        let applied = warrant.is_some() && self.ime.write_open(open);
        self.writes.push(WriteCommand {
            step: self.step + 1,
            at_ms: self.now_ms,
            origin,
            open,
            warrant,
            explicit_intent,
            truth_open,
            applied,
        });
    }

    /// Engine の観測・RefreshState・FocusChanged 由来の decision は `SetOpen` を出さない
    /// （ADR-213 P2b/P2c。かつての ActivationSync の写しはここで書き込みを起案していた）。
    /// 出たら退行なので panic する。明示操作の書き込みは `user_set_open` だけが起案する。
    fn handle_engine_decision(&mut self, decision: &Decision) {
        let effects = match decision {
            Decision::Consume { effects } | Decision::PassThroughWith { effects } => {
                effects.iter().cloned().collect::<Vec<_>>()
            }
            Decision::PassThrough => Vec::new(),
        };
        for e in effects {
            assert!(
                !matches!(e, Effect::Ime(ImeEffect::SetOpen { .. })),
                "このハーネスでは明示操作の SetOpen は user_set_open だけが出す: {e:?}"
            );
        }
    }

    /// 各ステップの後: Engine の再評価（`notify_engine_refresh`）と drift 判定。
    fn settle(&mut self, label: String, observed_open: Option<bool>) {
        let ctx = self.ctx();
        let decision = self.engine.on_command(EngineCommand::RefreshState, &ctx);
        self.handle_engine_decision(&decision);

        // `ir_align_placeholder_desired`（`runtime/ime_refresh.rs`）の写し（BUG-163、代案A）: 起動時の初期値のままの
        // `desired_open` を、明示意図が無く、観測から導ける開閉があるとき、最初の成功観測へ 1 回だけ揃える
        // （`ImeStateHub::align_placeholder_desired`、reducer は `ModeKeyPassedThrough { align_desired: true, demote_applied: false, }`）。
        if self.hub.model().desired_is_placeholder()
            && self.hub.model().last_intent.is_none()
            && self
                .hub
                .model()
                .observations
                .derive_any(self.now())
                .is_some()
        {
            self.reduce(ImeEvent::ModeKeyPassedThrough {
                align_desired: true,
                demote_applied: false,
            });
        }

        let explicit = self.hub.model().last_intent.as_ref().map(|i| i.target);
        if let Some(drift) = check_drift_correction(self.hub.model(), self.now(), explicit) {
            // `ir_apply_drift_correction`（`runtime/ime_refresh.rs`）: ImmCross（書き込み経路が
            // `set_ime_open_ordered`）で warrant が下りない補正は、「検知」の手前で見送る（`b6ab8980`）。
            let warranted = self.warrant_for(drift.desired).is_some();
            let imm_cross = matches!(self.setup.profile, ImePolicyProfile::ImmCross);
            let detected = !(imm_cross && !warranted);
            self.drift_fires.push(DriftFire {
                step: self.step + 1,
                at_ms: self.now_ms,
                drift,
                explicit_intent: self.has_explicit_intent(),
                warranted,
                detected,
            });
            if detected {
                self.issue_write(WriteOrigin::DriftCorrection, drift.desired);
            }
        }
        self.record_step(label, observed_open);
    }

    fn record_step(&mut self, label: String, observed_open: Option<bool>) {
        let ctx = self.ctx();
        self.step += 1;
        self.steps.push(StepRecord {
            step: self.step,
            at_ms: self.now_ms,
            label,
            truth: self.ime.state(),
            desired_open: self.hub.model().desired_open(),
            effective_open: ctx.ime_on,
            input_mode: self.hub.model().input_mode(),
            engine_active: self.engine.compute_active(&ctx),
            explicit_intent: self.has_explicit_intent(),
            observed_open,
        });
    }
}

fn make_engine() -> Engine {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../layout/nicola.yab");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} が読めない: {e}", path.display()));
    let layout = YabLayout::parse(&content, KeyboardModel::Jis).expect("nicola.yab");
    let fsm = NicolaFsm::new(
        layout,
        VkCode(0x1D),
        VkCode(0x1C),
        100,
        ConfirmMode::Wait,
        0,
    );
    Engine::new(
        fsm,
        SpecialKeyCombos {
            engine_on: vec![],
            engine_off: vec![],
            ime_on: vec![],
            ime_off: vec![],
            ime_toggle: vec![],
        },
    )
}
