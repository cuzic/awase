//! 閉ループのハーネス: 擬似 IME と awase の純粋な状態遷移層をつなぎ、
//! キー・観測・フォーカス・時刻の進行を流し込んで「書き込み命令」を集める。
//!
//! 実際に呼ぶ awase の層（すべて Linux ホストで動く ungated なもの）:
//! - `ImeModel::reduce`（belief の唯一の書き込み点）と `ImeModel::resolve_open_at`
//! - `IntentStore::{record, lookup, resolve_effective_open, remove}`
//! - `KeyEffectKeymap::predict`（GJI ATOK プリセット、同梱表）
//! - `open_warrant::issue_open_warrant`
//! - `drift_correction::check_drift_correction`（旧 `ImeStateHub::check_drift_correction` の本体）
//! - `awase::engine::Engine`（`EngineCommand::RefreshState`/`FocusChanged` の活性遷移と `SetOpen`）
//!
//! Windows 専用（`#[cfg(windows)]`）で呼べないため、**数行の配線をここで写している**もの
//! （写し元の行は各メソッドの doc に書く。写し元が変わったらここも直すこと）:
//! - `ImeStateHub::apply_key_effect_prediction`（`state/platform_state.rs`）
//! - `ImeStateHub::effective_open_at`（同）
//! - `ImeStateHub::warrant_context`/`issue_actuation_order`（同）
//! - `ImeStateHub::record_explicit_intent`/`write_*`（同）
//! - `kp_stage_key_effect_track`/`kp_predict_key_effect`（`runtime/key_pipeline.rs`）
//! - `ir_apply_drift_correction` の「検知へ進むか」まで（`runtime/ime_refresh.rs`）: `check_drift_correction` と、
//!   ImmCross で warrant が下りない補正を検知の手前で見送る早期 return（BUG-163 の1段目、`b6ab8980`）。
//!   Blind/Read の再送打ち切り・settle 待ち・conv ラッチは写していない。
//!
//! 写していない（このハーネスでは起きない）もの: 通過マーク（ADR-187 `ModeKeyPassLatch` →
//! `ModeKeyPassedThrough`）、`ImeApplyRequested`/`applied` の往復、ForceGuard、TSF warmup、
//! Engine の `on_input`（打鍵のかな変換。IME 側には文字キーをそのまま渡す）。

use std::time::{Duration, Instant};

use awase::config::ConfirmMode;
use awase::engine::{
    Decision, Effect, Engine, EngineCommand, ImeEffect, InputContext, InputModeState, NicolaFsm,
    SetOpenOrigin, SpecialKeyCombos,
};
use awase::scanmap::KeyboardModel;
use awase::types::VkCode;
use awase::yab::YabLayout;
use awase_windows::state::drift_correction::{check_drift_correction, DriftCorrection};
use awase_windows::state::evidence::{ImmCrossProbe, Observed, ObserverPoll};
use awase_windows::state::ime_event::{
    EventTime, HwndId, ImeEvent, ImeEventEnvelope, ImePolicyProfile, ObservationConfidence,
    ObservationSource, UserIntentSource,
};
use awase_windows::state::ime_model::ImeModel;
use awase_windows::state::intent_store::IntentStore;
use awase_windows::state::key_effect_predictor::{KeyEffectKeymap, PredictInput, Prediction};
use awase_windows::state::open_warrant::{issue_open_warrant, OpenWarrant, WarrantContext};
use awase_windows::state::probe_admission::{Admission, FocusFence, ImmLikeTicket};
use awase_windows::state::TickMs;

use super::pseudo_ime::{PseudoIme, TrueState, CONV_ALNUM};

/// 本番の `GetTickCount64` に相当する tick の起点（0 付近だと TTL 計算が境界に寄るため）。
const TICK_BASE: u64 = 1_000_000;

/// 観測の経路（ハーネスが生成できるもの）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `ImmCrossProbe`（High）。`write_imm_cross_probe` 相当。
    ImmCross,
    /// `ObserverPoll`（Medium）。`apply_ime_update` 相当。
    Poll,
}

/// 書き込み命令の出所。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOrigin {
    /// Engine の活性遷移が対称性のために出す `SetOpen`（`SetOpenOrigin::ActivationSync`）。
    EngineActivationSync,
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
}

impl Setup {
    /// ImmCross（開閉・conv を読めるアプリ）で、指定した真の初期状態から起動する。
    #[must_use]
    pub const fn imm_cross(initial: TrueState) -> Self {
        Self {
            initial,
            profile: ImePolicyProfile::ImmCross,
            composing_visible: true,
        }
    }
}

/// 閉ループのハーネス本体。
pub struct Harness {
    pub ime: PseudoIme,
    model: ImeModel,
    intents: IntentStore,
    engine: Engine,
    keymap: KeyEffectKeymap,
    setup: Setup,
    base: Instant,
    now_ms: u64,
    seq: u64,
    focus: HwndId,
    epoch: u64,
    /// 直近に観測した conv の生値（`ImeBelief::prev_conversion_mode` 相当）。
    last_conv_raw: Option<u32>,
    step: usize,
    pub steps: Vec<StepRecord>,
    pub writes: Vec<WriteCommand>,
    pub drift_fires: Vec<DriftFire>,
    pub predictions: Vec<PredictionRecord>,
}

impl Harness {
    /// awase を起動した直後の状態を作る（`ImeModel::new()`、観測なし、明示意図なし）。
    ///
    /// 起動時のイベントは本番の bootstrap と同じ3つ（fence・hwnd・policy）だけを流す。
    /// Engine の直前の活性状態は起動時の文脈から計算して合わせる（起動の瞬間に
    /// 活性遷移の `SetOpen` が出ないように。本番の初期化と同じく「遷移」を作らない）。
    #[must_use]
    pub fn start(setup: Setup) -> Self {
        let mut h = Self {
            ime: PseudoIme::atok(setup.initial),
            model: ImeModel::new(),
            intents: IntentStore::default(),
            engine: make_engine(),
            keymap: KeyEffectKeymap::from_config(Some(1), None, &[])
                .expect("ATOK プリセット（session_keymap=1）"),
            setup,
            base: Instant::now(),
            now_ms: 0,
            seq: 0,
            focus: HwndId(0x1001),
            epoch: 1,
            last_conv_raw: None,
            step: 0,
            steps: Vec::new(),
            writes: Vec::new(),
            drift_fires: Vec::new(),
            predictions: Vec::new(),
        };
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
            mode: self.model.input_mode(),
            conv_raw: self.last_conv_raw,
            composing: self.setup.composing_visible && self.ime.state().open && {
                !matches!(truth_before.stage, super::pseudo_ime::TrueStage::None)
            },
            track: self.model.key_track(),
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
        let truth = self.ime.state();
        self.observe_value(source, truth.open)
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
        };
        self.reduce(ImeEvent::ObserverReported(any));
        let conv = self.ime.state().conv;
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
        if let Some(hwnd) = self.model.current_focus() {
            self.intents
                .record(hwnd, open, UserIntentSource::Command, now);
        }
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

    // ── 読み取り ─────────────────────────────────────────────────────────

    #[must_use]
    pub fn desired_open(&self) -> bool {
        self.model.desired_open()
    }

    #[must_use]
    pub const fn model(&self) -> &ImeModel {
        &self.model
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

    const fn tick(&self) -> u64 {
        TICK_BASE + self.now_ms
    }

    fn now(&self) -> Instant {
        self.base + Duration::from_millis(self.now_ms)
    }

    fn reduce(&mut self, event: ImeEvent) {
        self.seq += 1;
        let envelope = ImeEventEnvelope {
            time: EventTime {
                seq: self.seq,
                monotonic: self.now(),
                tick_ms: self.tick(),
            },
            event,
        };
        self.model.reduce(&envelope);
    }

    /// `ImeStateHub::effective_open_at` の写し（`state/platform_state.rs`）:
    /// `IntentStore` の有効な明示意図が `ImeModel` の belief より優先する。
    fn effective_open(&self) -> bool {
        let shadow = self.model.effective_open_at(self.now());
        self.intents
            .resolve_effective_open(self.model.current_focus(), shadow, TickMs(self.tick()))
            .value
    }

    fn has_explicit_intent(&self) -> bool {
        self.model.last_intent.is_some()
            || self
                .model
                .current_focus()
                .and_then(|h| self.intents.lookup(h, TickMs(self.tick())))
                .is_some()
    }

    fn ctx(&self) -> InputContext {
        InputContext {
            ime_on: self.effective_open(),
            input_mode: self.model.input_mode(),
            is_japanese_ime: true,
            composing: false,
            modifiers: awase::engine::ModifierState::default(),
            left_thumb_down: None,
            right_thumb_down: None,
        }
    }

    /// `ImeStateHub::apply_key_effect_prediction` の写し（`state/platform_state.rs`）。
    fn apply_key_effect_prediction(&mut self, p: Prediction) {
        if p.effect.is_noop() && p.track == self.model.key_track() {
            return;
        }
        self.reduce(ImeEvent::KeyEffectPredicted {
            open: p.effect.open,
            mode: p.effect.mode,
            track: p.track,
        });
        if p.effect.open.is_some() {
            if let Some(hwnd) = self.model.current_focus() {
                self.intents.remove(hwnd);
            }
        }
    }

    /// `ImeStateHub::warrant_context` + `issue_open_warrant` の写し（`state/platform_state.rs`）。
    fn warrant_for(&self, open: bool) -> Option<OpenWarrant> {
        let ctx = WarrantContext {
            intent_store: &self.intents,
            obs: &self.model.observations,
            guards: &self.model.force_guards,
            policy: &self.model.app_policy,
            desired_open: self.model.desired_open(),
            is_japanese_ime: true,
            now: self.now(),
            now_ms: TickMs(self.tick()),
        };
        let target = self.model.current_focus().unwrap_or(HwndId::NULL);
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

    fn handle_engine_decision(&mut self, decision: &Decision) {
        let effects = match decision {
            Decision::Consume { effects } | Decision::PassThroughWith { effects } => {
                effects.iter().cloned().collect::<Vec<_>>()
            }
            Decision::PassThrough => Vec::new(),
        };
        for e in effects {
            if let Effect::Ime(ImeEffect::SetOpen { open, origin }) = e {
                assert_eq!(
                    origin,
                    SetOpenOrigin::ActivationSync,
                    "このハーネスでは明示操作の SetOpen は user_set_open だけが出す"
                );
                // `handle_engine_activation_sync`: echo を記録するだけで desired を書かない。
                self.reduce(ImeEvent::EngineActivationSync { target: open });
                self.issue_write(WriteOrigin::EngineActivationSync, open);
            }
        }
    }

    /// 各ステップの後: Engine の再評価（`notify_engine_refresh`）と drift 判定。
    fn settle(&mut self, label: String, observed_open: Option<bool>) {
        let ctx = self.ctx();
        let decision = self.engine.on_command(EngineCommand::RefreshState, &ctx);
        self.handle_engine_decision(&decision);

        let explicit = self.model.last_intent.as_ref().map(|i| i.target);
        if let Some(drift) = check_drift_correction(&self.model, self.now(), explicit) {
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
            desired_open: self.model.desired_open(),
            effective_open: ctx.ime_on,
            input_mode: self.model.input_mode(),
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
