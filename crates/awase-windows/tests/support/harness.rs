//! 閉ループのハーネス: 擬似 IME と awase の純粋な状態遷移層をつなぎ、
//! キー・観測・フォーカス・時刻の進行を流し込んで「書き込み命令」を集める。
//!
//! 実際に呼ぶ awase の層（すべて Linux ホストで動く ungated なもの）:
//! - `ImeStateHub`（本物。`with_clock(HubClock::manual(..))` で仮想時計）: `dispatch_event`（= `ImeModel::reduce` と
//!   `event_log`・`journal`）、`effective_open_at`、`record_explicit_intent`、`apply_key_effect_prediction`、
//!   `warrant_context`、`align_placeholder_desired`、`arm/follow_external_change_in_scope`、`IntentStore`・`ExternalChangeWatch` はハブが持つ
//! - `KeyEffectKeymap::predict`（GJI ATOK プリセット、同梱表）と `plan_key_effect_track`（`kp_stage_key_effect_track` の判断）。ただし判断の入力 `KeyTrackFacts`（物理 KeyDown・非修飾キー・修飾なし・非消費）は
//!   ハーネスが決め打ちで組む（`key()` は修飾キー単体の vk を打たない前提。打つシナリオを足すなら `is_modifier_key` を vk から求めること）
//! - `open_warrant::issue_open_warrant`
//! - `drift_correction::check_drift_correction`（旧 `ImeStateHub::check_drift_correction` の本体）
//! - `awase::engine::Engine`（`EngineCommand::RefreshState`/`FocusChanged` の活性遷移と `SetOpen`）
//!
//! Windows 専用（`#[cfg(windows)]`）で呼べないため、**数行の配線をここで写している**もの
//! （写し元の行は各メソッドの doc に書く。写し元が変わったらここも直すこと）:
//! - `ImeStateHub::issue_actuation_order`/`write_*`（`state/platform_state.rs`。`warrant_context` までは本物）
//! - `kp_predict_key_effect`（`runtime/key_pipeline.rs`。観測〈keymap・TSF〉と実行が大半で、純粋な判断は `predict_with_override` と
//!   `plan_key_effect_track` に分離済み。残りは分けない、F5d と同じ理由）
//! - `ir_apply_drift_correction` の「検知へ進むか」まで（`runtime/ime_refresh.rs`）: `check_drift_correction` と、
//!   ImmCross で warrant が下りない補正を検知の手前で見送る早期 return（BUG-163 の1段目、`b6ab8980`）。
//!   Blind/Read の再送打ち切り・settle 待ち・conv ラッチは写していない。
//!
//! - 窓モデル（ADR-277 段階 2、`Window`・`Setup::in_window`・`focus_to`・`demote_by_learning`）: 窓の live の profile は
//!   本物の `AppImeProfile::from_class_and_process` と `learning_store::apply_learned_imm_capability`（学習による降格）で
//!   求め、同じプロセスの中の窓の移動は `FocusHwndUpdated` と本物の `app_profile_follow_event`（`AppProfileChanged`、
//!   DD3-01・#779）、別のプロセスへの移動は `FocusChanged` を流す。殻の `notify_focus_hwnd_updated_if_needed`
//!   （`runtime/focus_tracking.rs`）は `#[cfg(windows)]` の `Runtime` のメソッドなので呼べず、その 2 つの dispatch の
//!   **並びだけ**をここで写す（判断は核の関数）。`on_focus_process_changed` の他の処理（hwnd キャッシュの復元、
//!   新しいスレッドの閉の仮定、settle 期限）は写さない。
//! - ADR-205 の外部変化の監視窓は本物（`arm/follow_external_change_in_scope`、追随の副作用もハブの中）を呼ぶ。
//!   `Setup::with_external_close_watch(true)` のときだけ働く（ADR-205 の有無で結果が分かれるシナリオ用）。
//!
//! 写していない（このハーネスでは起きない）もの: 通過マーク（ADR-187 `ModeKeyPassLatch` →
//! `ModeKeyPassedThrough`）、`ImeApplyRequested`/`applied` の往復、ForceGuard、TSF warmup、
//! Engine の `on_input`（打鍵のかな変換。IME 側には文字キーをそのまま渡す）。

use std::time::Instant;

use awase::config::ConfirmMode;
use awase::engine::{
    ActivationState, ConvMode, Decision, Effect, Engine, EngineCommand, ImeEffect, InputContext,
    InputModeState, NicolaFsm, SpecialKeyCombos,
};
use awase::scanmap::KeyboardModel;
use awase::types::VkCode;
use awase::yab::YabLayout;
use awase_windows::focus::class_names::AppImeProfile;
use awase_windows::focus::learning_store::{apply_learned_imm_capability, ImmCapability};
use awase_windows::state::app_ime_policy::{app_profile_follow_event, AppImePolicy};
use awase_windows::state::conv_classify::ConvSyncReason;
use awase_windows::state::drift_correction::{check_drift_correction, DriftCorrection};
use awase_windows::state::drift_plan::{decide_drift_plan, DriftFacts, DriftPlan, DriftStep};
use awase_windows::state::evidence::{
    ConvOpenInference, FocusProbe, HeuristicDefault, ImmCrossProbe, Observed, ObserverPoll,
};
use awase_windows::state::foreground_scope::ForegroundScope;
use awase_windows::state::hub_clock::HubClock;
use awase_windows::state::ime_event::{
    HwndId, ImeEvent, ImePolicyProfile, ObservationConfidence, ObservationSource, UserIntentSource,
};
use awase_windows::state::ime_model::ImeModel;
use awase_windows::state::key_effect_predictor::{
    plan_key_effect_track, KeyEffectKeymap, KeyTrack, KeyTrackFacts, ModeKeyModifiers,
    PredictInput, Prediction, TableKey,
};
use awase_windows::state::open_warrant::{issue_open_warrant, OpenWarrant};
use awase_windows::state::platform_state::ImeStateHub;
use awase_windows::state::probe_admission::{Admission, FocusFence, ImmLikeTicket};
use awase_windows::state::scope_belief::{ModeClass, ScopeBelief, ScopeEvent, WindowFacts};
use awase_windows::state::TickMs;

use super::pseudo_ime::{Grid, PseudoIme, TrueStage, TrueState, CONV_HIRAGANA};

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
    /// `FocusProbe`（Low、フォーカス変更直後の同期プローブ。`BeliefOnly`）。`belief_oracle`（`belief_engine_invariant.rs`）が使う。
    FocusProbe,
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
    /// 予測の入力の追跡（打鍵前の `key_track`）・入力中の観測・直近に観測した conv（食い違いの仕分けに使う、ADR-277）。
    pub input_track: KeyTrack,
    pub input_composing: bool,
    pub input_conv_raw: Option<u32>,
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
    /// 核の `ImeModel::app_policy`（窓モデルの不変条件 (a)(b) 用、ADR-277 段階 2）。
    pub core_policy: AppImePolicy,
    /// 殻の live の profile（`FocusTracker.current.app_profile` 相当。窓モデルでは窓から求めた値、それ以外は `Setup::profile`）。
    pub live_profile: ImePolicyProfile,
    /// 殻の `can_use_imm32_cross_process()`（drift の書き込み経路）。
    pub live_imm_cross: bool,
    /// 核の focus epoch（プロセスが変わると進む。殻はこのとき進行中の試行を捨てる〈ADR-080 破棄条件2〉）。
    pub focus_epoch: u64,
    /// `effective_open` がどの経路で決まったか（`DecidedBy::label`。`IntentStore` が上書きしていれば `intent-store`）。
    /// 診断専用（`belief_engine_invariant.rs` の違反の分類）。
    pub basis: &'static str,
    /// ADR-282 V0: 候補 Z(`ScopeBelief`)を並走させているときの、Z の開閉・Engine の活性・根拠(`None` は並走していない)。
    pub z_open: Option<bool>,
    pub z_engine_active: Option<bool>,
    pub z_basis: Option<&'static str>,
    /// ADR-282 V0: 候補 Z に U2 の案(新しいスレッドは閉の実測の事前)を足した変種の同じ値。
    pub zp_open: Option<bool>,
    pub zp_engine_active: Option<bool>,
    pub zp_basis: Option<&'static str>,
}

/// 窓 1 つの記述（ADR-277 段階 2）。殻の `FocusTracker.current`（pid・クラス・プロセス・学習による降格）に当たる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// golden・経過に出す短い名前。
    pub name: &'static str,
    pub hwnd: HwndId,
    pub pid: u32,
    /// IME の状態はスレッドごと（同じスレッドの窓は擬似 IME を共有する）。
    pub thread: u32,
    pub class: &'static str,
    pub process: &'static str,
    /// `app_overrides.input_relay_apps` に載っているか（`InputRelay`）。
    pub input_relay: bool,
    /// 実測学習で IMM が使えないと学習されたか（`ImmCapability::Unavailable`、ADR-262 段階 A）。
    pub learned_unavailable: bool,
}

impl Window {
    /// 殻の live の profile（本物の `from_class_and_process` + `apply_learned_imm_capability`）。
    #[must_use]
    pub fn live_profile(&self) -> AppImeProfile {
        let relay = if self.input_relay {
            vec![self.process.to_string()]
        } else {
            Vec::new()
        };
        apply_learned_imm_capability(
            AppImeProfile::from_class_and_process(self.class, self.process, &relay),
            self.learned_unavailable
                .then_some(ImmCapability::Unavailable),
        )
    }

    /// IME の実状態を IMM で読める窓か（`can_use_imm32_cross_process`）。読めない窓では観測を流さない。
    #[must_use]
    pub fn readable(&self) -> bool {
        self.live_profile().can_use_imm32_cross_process()
    }
}

/// シナリオの設定。
#[derive(Debug, Clone, Copy)]
pub struct Setup {
    pub initial: TrueState,
    /// 起動時のアプリのプロファイル（`InitialFocusScopeEstablished`）。
    pub profile: ImePolicyProfile,
    /// TSF の composition（入力中）を観測できるアプリか。`PredictInput::composing` に効く。
    pub composing_visible: bool,
    /// 擬似 IME の真値にする格子と、awase の予測器が引く同梱表（`session_keymap`）。既定は ATOK。
    pub grid: Grid,
    /// ADR-205（外部クローズの監視窓）が有効か。無効なら Q4（外部注入の閉じが観測されない）のまま。
    pub external_close_watch: bool,
    /// ハーネスが**呼び出し単位で**外せる防御（ablation、ADR-277 決定 3）。既定は全部有効（本番と同じ）。
    pub defenses: Defenses,
    /// 窓モデルで起動するときの最初の窓（`Setup::in_window`）。`None` なら従来どおり窓の表を持たない。
    pub window: Option<Window>,
}

/// ハーネスが呼び出しごと外せる防御のスイッチ（ablation）。
///
/// 外せるのは、ハーネス（=本番の殻の写し）が**別の呼び出しとして**呼んでいる防御だけ。核の関数の内側の条件
/// （例: `issue_open_warrant` の `is_japanese_ime` 棄却）は外せない。そちらは `cargo mutants` の変異で調べる
/// （ADR-277 決定 3 の理由: 本番のコードにテスト用の分岐を足さない）。防御の目録は `support/defenses.rs`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defenses {
    /// BUG-163 代案A: 起動時の初期値のままの `desired_open` を最初の成功観測へ揃える
    /// （`ImeStateHub::align_placeholder_desired`、本番の呼び出し元は `ir_align_placeholder_desired`）。
    pub placeholder_align: bool,
    /// ECRS DD3-01（#779）: 同じフォーカス中に殻の live の profile が変わったら核の `app_policy` を追随させる
    /// （`app_profile_follow_event` → `AppProfileChanged`、本番の呼び出し元は `notify_focus_hwnd_updated_if_needed`）。
    pub app_profile_follow: bool,
    /// BUG-163 の 1 段目（`b6ab8980`）: 授権（warrant）が下りない ImmCross の補正は「検知」の手前で見送る
    /// （`decide_drift_plan` の `DriftStep::SkipWarrantWouldBlock`、本番の呼び出し元は `ir_apply_drift_correction`）。
    /// 外すと `warrant_would_block` を常に偽にして `decide_drift_plan` を呼ぶ（ADR-277 段階 3）。
    pub drift_skip_unwarranted: bool,
}

impl Defenses {
    /// 全部有効（本番と同じ）。
    pub const ALL_ON: Self = Self {
        placeholder_align: true,
        app_profile_follow: true,
        drift_skip_unwarranted: true,
    };
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
            defenses: Defenses::ALL_ON,
            window: None,
        }
    }

    /// 窓モデル（ADR-277 段階 2）で、`window` にフォーカスがある状態から起動する。profile は窓の live の値。
    #[must_use]
    pub fn in_window(initial: TrueState, window: Window) -> Self {
        let mut s = Self::imm_cross(initial);
        s.profile = ImePolicyProfile::from(window.live_profile());
        s.window = Some(window);
        s
    }

    /// 防御のスイッチを変える（ablation）。
    #[must_use]
    pub const fn with_defenses(mut self, defenses: Defenses) -> Self {
        self.defenses = defenses;
        self
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
    /// 窓モデル: いまフォーカスのある窓（`Setup::in_window` のときだけ `Some`）。
    window: Option<Window>,
    /// 窓モデル: いまフォーカスの無いスレッドの擬似 IME（スレッドごとに状態を持つ）。
    parked: Vec<(u32, PseudoIme)>,
    /// 入場したときに、直前のスレッドの擬似 IME が「開」なら開を引き継ぐスレッド(ADR-282 §1.4・O5: MS-IME 本体の Win32 EDIT →
    /// Windows Terminal の向きで、CI で 6/6 測った振る舞いの写し。閉の向き・変換モードは引き継がない)。既定は空(引き継がない)。
    inherit_open_threads: Vec<u32>,
    /// ADR-282 V0: 候補 Z を本物の hub と並走させる(`with_scope_belief`)。本物の belief・Engine には効かせない。
    z: Option<ScopeBelief>,
    /// 候補 Z に U2 の案(新しいスレッドは閉)を足した変種(`with_scope_belief` が `z` と一緒に作る)。
    zp: Option<ScopeBelief>,
    /// 窓モデル: 実測学習の表（`ImmCapabilityStore` 相当、(プロセス, クラス) ごと）。戻ってきた窓にも効く。
    learned_unavailable: Vec<(&'static str, &'static str)>,
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
            hub: ImeStateHub::with_clock(
                HubClock::manual(TICK_BASE, Instant::now()),
                quanta::Clock::new(),
            ),
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
            focus: setup.window.map_or(HwndId(0x1001), |w| w.hwnd),
            epoch: 1,
            last_conv_raw: None,
            step: 0,
            steps: Vec::new(),
            writes: Vec::new(),
            drift_fires: Vec::new(),
            predictions: Vec::new(),
            japanese_ime: true,
            window: setup.window,
            parked: Vec::new(),
            inherit_open_threads: Vec::new(),
            z: None,
            zp: None,
            learned_unavailable: Vec::new(),
        };
        h.hub.set_is_japanese_ime(true);
        h.reduce(ImeEvent::InitialFocusScopeEstablished {
            to: h.focus,
            profile: setup.profile,
            focus_epoch: h.epoch,
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
        self.key_mods(vk, false)
    }

    /// Ctrl を押したままの物理キー1回。実 IME はキーを受けて普通に動く（擬似 IME は修飾を知らない。実機の
    /// GJI は Ctrl 押下中のひらがなキー〈0xF2〉でも ON になった、2026-10-26 dragonflyg4 の awase.log。Mozc のクライアントは
    /// 0xF0〜0xF4 の修飾を捨てる〈`keyevent_handler.cc:330-352`〉ので、素のキーと同じ）。awase の追跡・予測は、修飾付きを抑止する
    /// （`plan_key_effect_track` が `None`）が、ADR-281 で GJI 系の 0xF0〜0xF4 の Ctrl・Shift 付きだけは素のキーとして予測する。
    pub fn key_ctrl(&mut self, vk: u16) -> &mut Self {
        self.key_mods(vk, true)
    }

    fn key_mods(&mut self, vk: u16, ctrl: bool) -> &mut Self {
        let truth_before = self.ime.state();
        // 本物の `kp_stage_key_effect_track` の判断（`plan_key_effect_track`）。物理の無修飾の KeyDown で、エンジンが消費せず
        // IME へ通した打鍵なので、`None`（追跡しない）にはならない。
        let passive_rule_eligible = plan_key_effect_track(&KeyTrackFacts {
            vk,
            is_physical_key_down: true,
            is_modifier_key: false,
            ctrl,
            alt: false,
            shift: false,
            win: false,
            was_down: false,
            consumed: false,
            has_shadow_action: false,
            has_sync_direction: false,
            // ADR-281: GJI 系の格子（ATOK・MS-IME プリセット）は 0xF0〜0xF4 の修飾を捨てる IME。Microsoft IME 本体は 0xF2 の Ctrl だけ。
            mode_key_modifiers: match self.setup.grid {
                Grid::MsimeNative => ModeKeyModifiers::MsImeNativeCtrlOnHiragana,
                Grid::Atok | Grid::GjiMsime => ModeKeyModifiers::DroppedByGjiMozc,
            },
        });
        // 本番は追跡しない打鍵（`None`）では予測も呼ばない。無修飾の物理 KeyDown は必ず `Some`。
        assert!(
            ctrl || passive_rule_eligible.is_some(),
            "修飾キーでない無修飾の物理 KeyDown は追跡する"
        );
        let Some(passive_rule_eligible) = passive_rule_eligible else {
            let press = self.ime.press(vk);
            self.predictions.push(PredictionRecord {
                step: self.step + 1,
                vk,
                input_open: self.effective_open(),
                input_mode: self.hub.model().input_mode(),
                input_track: self.hub.model().key_track(),
                input_composing: false,
                input_conv_raw: self.last_conv_raw,
                prediction: None,
                truth_before: press.before,
                truth_after: press.after,
            });
            self.settle(format!("key_ctrl(0x{vk:02X})"), None);
            return self;
        };
        let input = PredictInput {
            open: self.effective_open(),
            mode: self.hub.model().input_mode(),
            conv_raw: self.last_conv_raw,
            composing: self.setup.composing_visible && self.ime.state().open && {
                !matches!(truth_before.stage, super::pseudo_ime::TrueStage::None)
            },
            track: self.hub.model().key_track(),
            unreadable: false,
            passive_rule_eligible,
            observable_toggle_guard: false,
        };
        let prediction = self.keymap.predict(vk, &input);
        let press = self.ime.press(vk);
        if let Some(p) = prediction {
            self.apply_key_effect_prediction(p);
        }
        self.z_key(vk, passive_rule_eligible, input.composing);
        self.predictions.push(PredictionRecord {
            step: self.step + 1,
            vk,
            input_open: input.open,
            input_mode: input.mode,
            input_track: input.track,
            input_composing: input.composing,
            input_conv_raw: input.conv_raw,
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
            Source::FocusProbe => (
                Observed::<FocusProbe>::from_probe(&accepted, open).into(),
                ObservationSource::FocusProbe,
                ObservationConfidence::Low,
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
            // 英数かは本番と同じく NATIVE ビットで決める（`ConvMode::from_u32`、`is_eisu_evidence`）。以前は `conv == 0x10`
            // だけを英数としていた。格子に出る conv（0x10・0x13・0x19・0x1B）では結果は同じで、これが原因の食い違いは
            // 無かった（ADR-277 の仕分け）が、0x18（全角英数）・0x00 等で本番とずれるので揃えた。
            let mode = if ConvMode::from_u32(conv).is_eisu() {
                InputModeState::ObservedEisu
            } else {
                InputModeState::ObservedRomaji
            };
            self.reduce(ImeEvent::InputModeObserved {
                mode,
                source: obs_source,
                confidence,
            });
        }
        let z_mode = open.then(|| {
            if ConvMode::from_u32(conv).is_eisu() {
                ModeClass::Eisu
            } else {
                ModeClass::Kana
            }
        });
        let read_started_ms = self.now_ms;
        self.z_evolve(ScopeEvent::Observed {
            open: Some(open),
            mode: z_mode,
            source: obs_source,
            confidence,
            // 4 つの源(ImmCross・Poll・FocusProbe・ConvInference)はどれも IMM の読み・conv のビットからの推測で、
            // 読めない窓では反証に使わない(ADR-282 §4.4 の信頼表)。
            imm_family: true,
            read_started_ms,
        });
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

    /// 起動後に作られた新しいスレッドの、読めない窓(`Imm32Unavailable`)へ移る(BUG-204 の関連、フォーカス移動の再現用)。
    ///
    /// - 擬似 IME は、その新しいスレッドの IME の状態に置き換える(`new_thread_initial`。ADR-191 §3.2: 新しいスレッドは
    ///   閉×ひらがな)。`focus_change()` は擬似 IME を共有のままにするが、実際の IME はスレッドごとに状態を持つ。
    /// - 本番の `focus_tracking`(`Imm32Unavailable` のキャッシュ無し・新スレッド)と同じく、`FocusChanged` の後に
    ///   「新しいスレッドは閉」という観測(`assume_closed_for_new_thread` 相当。低信頼の `HeuristicDefault`)を記録する。
    ///   **入力モードには触れない**(本番も触れない)。本番の関数は `pub(crate)` なので、同じ事象を流す。
    /// - 本番の `key_pipeline` の他の段(物理 IME キーの `ObservedEisu` 救済など)は写さない。
    pub fn focus_change_to_new_thread_unreadable(
        &mut self,
        new_thread_initial: super::pseudo_ime::TrueState,
    ) -> &mut Self {
        let from = self.focus;
        self.epoch += 1;
        self.focus = HwndId(self.focus.0 + 1);
        self.setup.profile = ImePolicyProfile::Imm32Unavailable;
        self.ime = PseudoIme::from_grid(self.setup.grid, new_thread_initial);
        self.last_conv_raw = None;
        self.reduce(ImeEvent::FocusChanged {
            from: Some(from),
            to: self.focus,
            profile: self.setup.profile,
            focus_epoch: self.epoch,
        });
        self.reduce(ImeEvent::ObserverReported(
            Observed::<HeuristicDefault>::at_startup(
                self.setup.profile,
                false,
                HwndId::NULL,
                self.epoch,
            )
            .into(),
        ));
        let ctx = self.ctx();
        let decision = self.engine.on_command(EngineCommand::FocusChanged, &ctx);
        self.handle_engine_decision(&decision);
        self.settle("focus_change_to_new_thread_unreadable()".into(), None);
        self
    }

    /// 窓モデル: 別の窓へフォーカスを移す（ADR-277 段階 2）。
    ///
    /// - 殻の live の profile（`Setup::profile` に持つ）は窓から求め直す。
    /// - プロセスが変わったら `FocusChanged`（核の `app_policy`・epoch・観測プールを入れ直す）と Engine の `FocusChanged`。
    /// - 同じプロセスなら `notify_focus_hwnd_updated_if_needed` の並び: hwnd が変われば `FocusHwndUpdated`、続けて
    ///   `app_profile_follow_event`（防御 `app_profile_follow`、外すと #779 の前の挙動）。
    /// - スレッドが変われば擬似 IME を入れ替える。初めてのスレッドは閉×ひらがな（ADR-191 §3.2）。
    /// - 殻と同じく、直近に観測した conv（`prev_conversion_mode`）は毎回捨てる。
    pub fn focus_to(&mut self, mut to: Window) -> &mut Self {
        let from = self
            .window
            .expect("focus_to は窓モデル（Setup::in_window）で使う");
        to.learned_unavailable |= self.learned_unavailable.contains(&(to.process, to.class));
        let new_thread =
            to.thread != from.thread && !self.parked.iter().any(|(t, _)| *t == to.thread);
        self.switch_thread(from.thread, to.thread);
        self.window = Some(to);
        self.z_focus(to, new_thread);
        self.setup.profile = ImePolicyProfile::from(to.live_profile());
        self.last_conv_raw = None;
        if from.pid == to.pid {
            self.notify_same_process(to.hwnd);
        } else {
            let prev = self.focus;
            self.epoch += 1;
            self.focus = to.hwnd;
            self.reduce(ImeEvent::FocusChanged {
                from: Some(prev),
                to: self.focus,
                profile: self.setup.profile,
                focus_epoch: self.epoch,
            });
            let ctx = self.ctx();
            let decision = self.engine.on_command(EngineCommand::FocusChanged, &ctx);
            self.handle_engine_decision(&decision);
        }
        self.settle(format!("focus_to({})", to.name), None);
        self
    }

    /// 窓モデル: いまの窓が実測学習で降格される（`ImmCapability::Unavailable` を学習した。同じ窓のまま、次の
    /// フォーカスの更新で殻の live の profile だけが変わる。`FocusTracker::update_with_process_name` の 500ms 周期）。
    pub fn demote_by_learning(&mut self) -> &mut Self {
        let mut w = self
            .window
            .expect("demote_by_learning は窓モデル（Setup::in_window）で使う");
        w.learned_unavailable = true;
        self.learned_unavailable.push((w.process, w.class));
        self.window = Some(w);
        self.setup.profile = ImePolicyProfile::from(w.live_profile());
        self.notify_same_process(w.hwnd);
        self.z_focus(w, false);
        self.settle(format!("demote_by_learning({})", w.name), None);
        self
    }

    /// 窓モデル: いまフォーカスのある窓。
    #[must_use]
    pub const fn window(&self) -> Option<Window> {
        self.window
    }

    /// `notify_focus_hwnd_updated_if_needed`（bootstrap でもプロセス変更でもないとき）の dispatch の並び。
    fn notify_same_process(&mut self, new_hwnd: HwndId) {
        if new_hwnd != self.focus {
            self.focus = new_hwnd;
            self.reduce(ImeEvent::FocusHwndUpdated { hwnd: new_hwnd });
        }
        if self.setup.defenses.app_profile_follow {
            if let Some(event) =
                app_profile_follow_event(&self.hub.model().app_policy, self.setup.profile)
            {
                self.reduce(event);
            }
        }
    }

    /// スレッドが変わったら擬似 IME を入れ替える（前のスレッドの擬似 IME は控える）。
    fn switch_thread(&mut self, from: u32, to: u32) {
        if from == to {
            return;
        }
        let next = match self.parked.iter().position(|(t, _)| *t == to) {
            Some(i) => self.parked.swap_remove(i).1,
            None => PseudoIme::from_grid(
                self.setup.grid,
                TrueState {
                    open: false,
                    conv: CONV_HIRAGANA,
                    stage: TrueStage::None,
                },
            ),
        };
        let prev = std::mem::replace(&mut self.ime, next);
        let prev_open = prev.state().open;
        self.parked.push((from, prev));
        if prev_open && !self.ime.state().open && self.inherit_open_threads.contains(&to) {
            self.ime.external_set_open(true);
        }
    }

    /// 窓モデル: スレッド `thread` を「入場時に直前のスレッドの開を引き継ぐ」スレッドにする(ADR-282 O5 の写し)。
    pub fn inherit_open_on_entry(&mut self, thread: u32) -> &mut Self {
        if !self.inherit_open_threads.contains(&thread) {
            self.inherit_open_threads.push(thread);
        }
        self
    }

    /// 現在の belief で、Engine が有効か(`Engine::compute_state`。IME ON・日本語 IME・ローマ字入力と判断できるか)。
    #[must_use]
    pub fn activation(&self) -> ActivationState {
        self.engine.compute_state(&self.ctx())
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
        let at_ms = self.now_ms;
        self.z_evolve(ScopeEvent::Explicit {
            open: Some(open),
            mode: None,
            at_ms,
        });
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

        // `ir_align_placeholder_desired`（`runtime/ime_refresh.rs`）が呼ぶ本物（BUG-163、代案A）: 起動時の初期値のままの
        // `desired_open` を、明示意図が無く、観測から導ける開閉があるとき、最初の成功観測へ 1 回だけ揃える。
        if self.setup.defenses.placeholder_align {
            let _ = self
                .hub
                .align_placeholder_desired(self.now(), TickMs(self.tick()));
        }

        if let Some(drift) = check_drift_correction(self.hub.model(), self.now()) {
            // `ir_apply_drift_correction`（`runtime/ime_refresh.rs`）が呼ぶ本物の `decide_drift_plan`（ADR-277 段階 3）。
            // 以前は「ImmCross で warrant が下りないなら見送る」の 1 条件だけをここに写していた。facts は殻の
            // `ir_observe_drift_facts` と同じ式で、ずれ・方針・書き込み経路・授権の有無を載せる（進行中の試行・settle 中・
            // 収束・診断済みは、このハーネスが模していないので既定の値）。
            let warranted = self.warrant_for(drift.desired).is_some();
            let imm_cross = matches!(self.setup.profile, ImePolicyProfile::ImmCross);
            let facts = DriftFacts {
                now: Instant::now(),
                engine_enabled: true,
                japanese_ime: true,
                drift: Ok(drift),
                settling: false,
                active: None,
                default_policy: self.hub.model().app_policy.default_feedback,
                imm_cross,
                // 防御 `drift_skip_unwarranted` を外すと、授権が下りなくても見送らない（#BUG-163 の 1 段目の前の挙動）。
                warrant_would_block: self.setup.defenses.drift_skip_unwarranted && !warranted,
                diag_already_notified: false,
                converged: false,
                fresh_evidence_after_giveup: false,
            };
            let detected = !matches!(
                decide_drift_plan(&facts),
                DriftPlan::Act(a) if a.step == DriftStep::SkipWarrantWouldBlock
            );
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
            core_policy: self.hub.model().app_policy,
            live_profile: self.setup.profile,
            live_imm_cross: self.window.map_or(
                matches!(self.setup.profile, ImePolicyProfile::ImmCross),
                |w| w.live_profile().can_use_imm32_cross_process(),
            ),
            focus_epoch: self.epoch,
            basis: self.effective_basis(),
            z_open: self.z.as_ref().map(|z| z.view().open),
            z_engine_active: self
                .z_ctx(self.z.as_ref())
                .map(|c| self.engine.compute_active(&c)),
            z_basis: self.z.as_ref().map(|z| z.view().basis),
            zp_open: self.zp.as_ref().map(|z| z.view().open),
            zp_engine_active: self
                .z_ctx(self.zp.as_ref())
                .map(|c| self.engine.compute_active(&c)),
            zp_basis: self.zp.as_ref().map(|z| z.view().basis),
        });
    }

    /// ADR-282 V0: 候補 Z の belief から作った Engine の文脈(`ctx` の `ime_on`・`input_mode` だけを Z に替えたもの)。
    fn z_ctx(&self, z: Option<&ScopeBelief>) -> Option<InputContext> {
        let v = z?.view();
        Some(InputContext {
            ime_on: v.open,
            input_mode: v.input_mode,
            ..self.ctx()
        })
    }

    /// ADR-282 V0: 候補 Z を並走させる。いまの窓へのフォーカスを Z に流してから使う。窓モデル(`Setup::in_window`)専用。
    pub fn with_scope_belief(&mut self) -> &mut Self {
        let w = self.window.expect("with_scope_belief は窓モデルで使う");
        let settle = awase_windows::tuning::KEY_EFFECT_SETTLE_MS;
        self.z = Some(ScopeBelief::new(settle));
        self.zp = Some(ScopeBelief::with_new_thread_prior(settle));
        self.z_focus(w, false);
        self
    }

    fn z_focus(&mut self, w: Window, new_thread: bool) {
        let facts = WindowFacts {
            imm_trusted: w.readable(),
            inherit_open_on_entry: self.inherit_open_threads.contains(&w.thread),
            new_thread,
        };
        let at_ms = self.now_ms;
        self.z_evolve(ScopeEvent::Focus {
            scope: w.thread,
            facts,
            at_ms,
        });
    }

    fn z_evolve(&mut self, e: ScopeEvent) {
        for z in [self.z.as_mut(), self.zp.as_mut()].into_iter().flatten() {
            z.evolve(e);
        }
    }

    /// ADR-282 V0: 候補 Z の明示操作(表のセルで決まるモードキー)。Z 自身の belief を入力に予測し、押した後の状態(L1′)で記録する。
    /// 追跡(`key_track`)は本物の hub のものを借りる(Z のスコープごとの追跡は持たない近似)。
    fn z_key(&mut self, vk: u16, passive_rule_eligible: bool, composing: bool) {
        if TableKey::from_vk(vk).is_none() {
            return;
        }
        let track = self.hub.model().key_track();
        let conv_raw = self.last_conv_raw;
        let at_ms = self.now_ms;
        let keymap = &self.keymap;
        for z in [self.z.as_mut(), self.zp.as_mut()].into_iter().flatten() {
            let v = z.view();
            let input = PredictInput {
                open: v.open,
                mode: v.input_mode,
                conv_raw,
                composing,
                track,
                unreadable: false,
                passive_rule_eligible,
                observable_toggle_guard: false,
            };
            let Some(p) = keymap.predict(vk, &input) else {
                continue;
            };
            let mode = p.effect.mode.map(|m| match m {
                InputModeState::ObservedEisu => ModeClass::Eisu,
                InputModeState::ObservedKana => ModeClass::KanaInputMethod,
                _ => ModeClass::Kana,
            });
            z.evolve(ScopeEvent::Explicit {
                open: Some(p.effect.open.unwrap_or(v.open)),
                mode,
                at_ms,
            });
        }
    }

    /// `effective_open` の根拠の分類名（殻の `ImeStateHub::effective_open_basis` と同じ式。あちらは `pub(crate)`）。
    fn effective_basis(&self) -> &'static str {
        let resolution = self.hub.model().resolve_open_at(self.now());
        if self.effective_open() == resolution.value {
            resolution.decided_by.label()
        } else {
            "intent-store"
        }
    }
}

fn make_engine() -> Engine {
    // 全列挙のテストが何十万回も起動するので、パース結果は 1 回だけ作って複製する。
    static LAYOUT: std::sync::OnceLock<YabLayout> = std::sync::OnceLock::new();
    let layout = LAYOUT
        .get_or_init(|| {
            let path =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../layout/nicola.yab");
            let content = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{} が読めない: {e}", path.display()));
            YabLayout::parse(&content, KeyboardModel::Jis).expect("nicola.yab")
        })
        .clone();
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
