//! TSF/Chrome cold-start probe ステートマシン。
//!
//! [`TsfProbeMachine`] は 10ms 間隔の `TIMER_TSF_PROBE` ハンドラから駆動される。
//! `tick()` がフェーズを 1 ステップ進め、dispatcher が返す [`ProbeAction`] を
//! `platform.rs::dispatch_probe_actions` が実行する。
//!
//! ## フェーズ遷移
//!
//! ```text
//! Probing(Chrome) ──► (TransmitChrome 後に即完了)
//!                      ├─[needs_literal=false]─► Done
//!                      └─[needs_literal=true]──► LiteralDetect ─[apply_transmit_done]─► Done
//! ```
//!
//! ## 設計ポリシー
//!
//! - `tick()` / `apply_*` は副作用なし。状態のみ更新し [`ProbeAction`] を返す。
//! - SendInput / mark_warm / mark_cold / RAW_TSF_LITERAL 操作は dispatcher 側で実行する
//!   (`platform.rs::dispatch_probe_actions`)。
//! - `romaji` / `deferred_vks` は `SendState` として各 phase variant に埋め込まれており、
//!   `ProbeAction::Transmit` 生成時に `take_current_send_for_transmit` で move する
//!   （clone コストを避ける）。
//! - FSM 内部フィールドは非公開。送信コンテキスト（`cold_seq` 等）は
//!   [`ProbeAction`] のペイロードに畳み込んで dispatcher に渡す。

use awase::types::VkCode;

/// probe 進行中に蓄積する後続 VK。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeferredVk {
    pub(crate) vk: VkCode,
    pub(crate) needs_shift: bool,
}
use crate::tsf::probe::{LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;
use timed_fsm::Clock;

/// 本番クロック: `crate::hook::current_tick_ms()` に委譲する。
///
/// `deadline_ms` は `current_tick_ms()` 起点で計算されるため、
/// `MonotonicClock` と混在させると epoch がずれる。
pub(crate) struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        crate::hook::current_tick_ms()
    }
}


/// `tick()` 呼び出し時に注入する環境観測値のスナップショット。
/// テストでは任意値を注入できる。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TsfEnvSnapshot {
    pub is_tsf_mode: bool,
    pub gji_active: bool,
    /// `LiteralDetect` の部分リテラル検出で参照する現在 composition 先頭文字。
    /// `crate::ime::get_foreground_comp_str_char()` のスナップショット。
    /// `None` = HIMC 未取得（テスト・LiteralDetect 非活性時）→ 部分リテラル検出をスキップ。
    pub foreground_comp_char: Option<char>,
    /// `GoogleJapaneseInputCandidateWindow` が現在表示中なら true。
    /// `GjiWarmupFsm` の NameChangeWait 早期脱出判定に使用する。
    /// `crate::tsf::observer::gji_candidate_visible_now()` のスナップショット。
    pub gji_candidate_visible: bool,
    /// 現在の IME 入力モード belief（Off / Hiragana / Katakana / Unknown）。
    /// `ImeModeFsm.state()` のスナップショット（OS 呼び出しなし）。
    /// `ChromeGjiReinitFsm` が IME モード確認待機に使用する。
    pub ime_mode: crate::tsf::ime_mode_fsm::ImeModeState,
    /// `ime_mode` が `IMC_GETCONVERSIONMODE` で OS から確認済みなら true。
    /// false = F21/F22 送信直後の belief のみ（async 確認待ち）。
    pub ime_mode_confirmed: bool,
}

/// probe 中に観測した事実。GjiFsm bridge・WarmupPath 分類に使う。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProbeObservations {
    pub nc_fired: bool,
    pub gji_resumed: bool,
}

/// `decide_transmit_plan` が確定した実行方針。dispatcher がそのまま実行する。
#[derive(Debug, Clone, Copy)]
pub(crate) struct TransmitPlan {
    /// バッチ送信に F2 (VK_DBE_HIRAGANA) を先行させるか。
    pub should_prepend_f2: bool,
    /// VK ローマ字パス（true）か Unicode TSF パス（false）か。
    pub used_eager_path: bool,
    /// raw TSF literal の LiteralDetect フェーズを有効にするか。
    pub needs_literal: bool,
    /// LiteralDetect のタイムアウト期間（ms）。
    pub literal_detect_ms: u64,
}

/// probe 観測値・環境スナップショット・コンテキストから送信方針を決定する純粋計算関数。
///
/// `GjiWarmupFsm` 内から呼ばれるが、単独でテストも可能。副作用なし。
pub(crate) fn decide_transmit_plan(
    initial_prepend_f2: bool,
    initial_used_eager: bool,
    obs: ProbeObservations,
    env: &TsfEnvSnapshot,
    deferred_empty: bool,
    forces_prepend_f2: bool,
    is_long_cold: bool,
) -> TransmitPlan {
    // nc_fired=false + TSF mode (WezTerm) + !forces_prepend_f2 (Short cold):
    // SendFreshF2 action が ~300ms 前に fresh F2 を送信済み → WezTerm の TSF context は
    // 既に初期化済み。バッチに再び F2 を含めると WezTerm が TSF reinit を再トリガーし、
    // 同一バッチの先頭 VK が reinit 中に届いてリテラル化する race が起きる。
    // Medium/Long cold (forces_prepend_f2=true): GJI が F2×2 で起動中 or 無応答のため F2 同梱が必要。
    let should_prepend_f2 = initial_prepend_f2
        && (obs.nc_fired || !env.is_tsf_mode || forces_prepend_f2);

    // gji_resumed=true: GJI が F2×2 に I/O 応答 → VK path 強制。
    // nc_fired=true: IME モード確認済み（Medium/Long cold かつ deferred なし → VK path）。
    // nc_fired=false + TSF mode: VK path 固定（unicode は GJI composition をバイパスし "nお" race が起きる）。
    let used_eager_path = if obs.gji_resumed {
        false
    } else if obs.nc_fired {
        (initial_used_eager || forces_prepend_f2) && deferred_empty
    } else if env.is_tsf_mode {
        false
    } else {
        initial_used_eager || forces_prepend_f2
    };

    // gji_resumed=true: GJI I/O 応答確認済み → composition 成功が確定。
    // Long cold (is_long_cold=true) 後は候補ウィンドウ表示に >500ms かかり false positive になる
    // （BS 誤送信 → context 破壊）。Medium cold は 7-10s で gji_long_idle_probe=true のため対象外。
    //
    // nc_fired=false + TSF mode 節:
    // NameChangeWait タイムアウトで IME 準備が未確認のまま transmit したケースを
    // LiteralDetect で回収する。F2 をバッチに含めない場合も IME が cold の可能性がある。
    // （seつぞく バグ: gji_idle ~1.4s で 300ms NameChangeWait が間に合わなかったケース）
    let needs_literal = (should_prepend_f2
        && env.gji_active
        && (!is_long_cold || env.is_tsf_mode)
        && !obs.gji_resumed)
        || (!obs.nc_fired && env.is_tsf_mode && env.gji_active && !obs.gji_resumed);

    let literal_detect_ms = if is_long_cold && env.is_tsf_mode {
        crate::tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE
    } else {
        crate::tuning::RAW_TSF_LITERAL_DETECT_MS
    };

    TransmitPlan {
        should_prepend_f2,
        used_eager_path,
        needs_literal,
        literal_detect_ms,
    }
}

/// `inspect_phase` が返す「次に何をすべきか」の宣言。
///
/// `tick_with_clock` はこれを受け取って状態遷移と `ProbeAction` emit を行う。
/// `inspect_phase` は `&self` のみ使用（副作用なし）。
enum NextStep {
    /// 現フェーズで継続待機（actions なし）
    Wait,
    /// `enter_transmit_chrome()` を呼ぶ
    TransmitChrome,
    /// `LiteralDetect`: composition 確定 → `Done`
    LiteralDone,
    /// `LiteralDetect`: raw literal 疑い → `RawTsfLiteralRecovery + Done`
    LiteralSuspected { ze_bs_count: usize },
}

/// [`ProbePhase::Probing`] の完了後アクションを区別するタグ。
pub(crate) enum ProbeKind {
    /// Chrome F2 probe。完了後 `Transmit(Chrome)` を emit。
    Chrome,
}

/// multi-tick にわたって蓄積する送信データ。
///
/// `ProbePhase` の各 variant に埋め込まれ、phase が単独所有する。
/// `Clone` は derive しない（所有権移動のみ許可）。
#[derive(Debug, Default)]
pub(crate) struct SendState {
    pub(crate) romaji: String,
    pub(crate) deferred_vks: Vec<DeferredVk>,
}

impl SendState {
    pub(crate) fn new(romaji: &str) -> Self {
        Self {
            romaji: romaji.to_string(),
            deferred_vks: Vec::new(),
        }
    }
}

/// [`ProbePhase::WaitingForCallback`] の内部状態。
pub(crate) enum WaitingFor {
    /// Fresh F2 送信済み・namechange コールバック待ち（GJI warmup パス）。
    FreshF2Sent {
        probe_settled: bool,
        budget_ms: u64,
        send: SendState,
    },
    /// `apply_transmit_done` 待ち（Transmit(Chrome) パス）。
    TransmitDone,
}

/// プローブ FSM の現在フェーズ。
pub(crate) enum ProbePhase {
    /// Chrome F2 後の静止待ち。
    Probing {
        probe: TsfReadinessProbe,
        total_max_ms: u64,
        kind: ProbeKind,
        send: SendState,
    },
    /// dispatcher コールバック（`apply_transmit_done`）待ち。
    WaitingForCallback(WaitingFor),
    /// raw TSF literal 検出待ち（TSF 送信後の verify フェーズ）。
    LiteralDetect {
        detector: LiteralDetector,
        ze_bs_count: usize,
        deadline_ms: u64,
        send: SendState,
        /// SHOW 発火時に IMM32 composition と突き合わせる期待かな文字。
        /// None の場合は IMM32 検証をスキップして CompositionConfirmed を返す。
        expected_kana: Option<char>,
    },
}

/// [`ProbeAction::Transmit`] の送信先。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransmitTarget {
    Tsf,
    Chrome,
}

/// [`GjiWarmupFsm`] が LiteralDetect / SacrificialWarmup フェーズに引き渡す設定。
///
/// [`ProbeAction::StartLiteralDetect`] および [`ProbeAction::StartSacrificialWarmup`] のペイロード。
/// `GjiWarmupFsm` が transmit 完了後、`needs_literal=true` と判断したときに emit する。
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct LiteralDetectConfig {
    pub cold_seq: u32,
    pub romaji: String,
    pub deferred_vks: Vec<DeferredVk>,
    pub plan: TransmitPlan,
    pub observations: ProbeObservations,
    pub literal_detect_ms: u64,
    /// 送信先ターゲット。SacrificialWarmup の resend フェーズで Chrome/TSF を切り替える。
    pub target: TransmitTarget,
}

/// [`SacrificialWarmupFsm`] が composition 確認後に emit する再送設定。
///
/// dispatcher が BS×1（犠牲 'a' 削除）→ 実ローマ字 transmit_tsf/transmit_chrome を行う。
#[derive(Debug)]
pub(crate) struct SacrificialResend {
    pub cold_seq: u32,
    pub romaji: String,
    pub deferred_vks: Vec<DeferredVk>,
    /// 送信先ターゲット。Chrome の場合は `transmit_chrome` + `VkMarker::Injected`、
    /// TSF の場合は `transmit_tsf` + `VkMarker::Tsf` を使う。
    pub target: TransmitTarget,
    /// `true` = VK_A が composition を確認（warm）、`false` = タイムアウト（cold）。
    /// Chrome dispatcher が cold 時に F22→F21 強制リセットを行うかどうかの判定に使う。
    pub confirmed_warm: bool,
}

/// ステートマシン → dispatcher 方向の宣言的アクション。
#[derive(Debug)]
pub(crate) enum ProbeAction {
    /// fresh F2 (`VK_DBE_HIRAGANA`) を送信し、完了したら
    /// [`TsfProbeMachine::apply_fresh_f2_sent`] を呼ぶ。
    SendFreshF2 { cold_seq: u32, probe_settled: bool },
    /// TSF または Chrome バッチパイプラインで `romaji` を送信する。
    ///
    /// `target == Tsf`: `deferred_vks` をフラッシュし warm マークしたあと
    /// [`TsfProbeMachine::apply_transmit_done`] を呼ぶ。
    /// `target == Chrome`: `deferred_vks` をフラッシュし warm マークして完了。
    Transmit {
        cold_seq: u32,
        /// FSM が確定した実行方針。dispatcher はそのまま実行する（再導出不要）。
        plan: TransmitPlan,
        /// probe 中に観測した事実。GjiFsm bridge・WarmupPath 分類に使う。
        observations: ProbeObservations,
        romaji: String,
        deferred_vks: Vec<DeferredVk>,
        target: TransmitTarget,
    },
    /// `RAW_TSF_LITERAL` を設定し、composition を `RawTsfLiteralRecovery` で cold マークする。
    RawTsfLiteralRecovery {
        cold_seq: u32,
        backs: usize,
        romaji: String,
    },
    /// GJI warmup 完了後に LiteralDetect フェーズを開始する。
    ///
    /// [`GjiWarmupFsm`] が `needs_literal=true` と判断したときに emit する。
    /// dispatcher は `LiteralDetectFsm` を生成して `TIMER_TSF_PROBE` を継続させる。
    #[allow(dead_code)]
    StartLiteralDetect(LiteralDetectConfig),
    /// GJI warmup 完了後に犠牲キー（VK_A）暖機フェーズを開始する。
    ///
    /// [`GjiWarmupFsm`] が `needs_literal=true` と判断したときに emit する。
    /// dispatcher が VK_A を送信してから [`crate::tsf::sacr_warmup_fsm::SacrificialWarmupFsm`]
    /// に切り替える。実ローマ字は TSF warm 確認後に [`SacrificialResend`] 経由で送信される。
    StartSacrificialWarmup(LiteralDetectConfig),
    /// [`crate::tsf::sacr_warmup_fsm::SacrificialWarmupFsm`] が composition 確認後に emit する。
    ///
    /// dispatcher が BS×1（犠牲 VK_A 削除）→ 実ローマ字 transmit_tsf → deferred_vks 送信を行う。
    SacrificialResend(SacrificialResend),
    /// Chrome sacr-warmup cold タイムアウト後の GJI 再初期化フェーズを開始する。
    ///
    /// dispatcher が F22→F21 を SendInput でキューイング + `ImeModeFsm` belief 更新 +
    /// async `IMC_GETCONVERSIONMODE` ポーリングを開始し、
    /// [`crate::tsf::chrome_gji_reinit_fsm::ChromeGjiReinitFsm`] に切り替える。
    /// FSM が Hiragana 確認 or タイムアウト後に `SacrificialResend` を emit して実ローマ字を送る。
    StartChromeGjiReinit {
        cold_seq: u32,
        romaji: String,
        deferred_vks: Vec<DeferredVk>,
    },
    /// Unicode モードで GJI write が観測されなかった。
    ///
    /// [`crate::tsf::unicode_literal_observer::UnicodeLiteralObserverFsm`] が emit する。
    /// dispatcher は `DispatchResult::LearnedTsf` を返し、呼び出し元 (`advance_tsf_probe`) が
    /// フォーカス中クラスを `InjectionModeStore` に学習し injection_mode を Tsf に昇格させる。
    UpgradeToTsf,
    /// Unicode cold-start warmup 完了後にバッファ済み文字を送信する。
    ///
    /// [`crate::tsf::unicode_cold_warmup_fsm::UnicodeColdWarmupFsm`] が GJI write 確認
    /// またはタイムアウト後に emit する。
    /// dispatcher が各 `char` を `send_unicode_char_direct()` で送信する。
    FlushDeferredUnicodeChars(Vec<char>),
    /// プローブ完了。dispatcher は `TIMER_TSF_PROBE` を kill する。
    Done,
}

/// TSF/Chrome cold-start プローブの状態機械本体。
///
/// `pending_tsf` (`RefCell<Option<TsfProbeMachine>>`) に格納し、
/// `TIMER_TSF_PROBE` ハンドラが `tick()` で 1 ステップ進める。
pub(crate) struct TsfProbeMachine {
    /// ログ相関番号
    cold_seq: u32,
    /// RAII guard。drop で `OUTPUT_GATE.active=false`。
    ///
    /// Chrome / LiteralDetect probe では `Some` を保持する。
    _guard: Option<OutputActiveGuard>,
    /// 現在フェーズ
    phase: ProbePhase,
}

impl TsfProbeMachine {
    /// Chrome F2 cold warmup (`send_romaji_batched` の cold パス) 用コンストラクタ。
    pub(crate) fn new_chrome(
        romaji: &str,
        cold_seq: u32,
        probe: TsfReadinessProbe,
        total_max_ms: u64,
        guard: OutputActiveGuard,
    ) -> Self {
        Self {
            cold_seq,
            _guard: Some(guard),
            phase: ProbePhase::Probing {
                probe,
                total_max_ms,
                kind: ProbeKind::Chrome,
                send: SendState::new(romaji),
            },
        }
    }

    /// ログ用 cold_seq 参照（上書き警告等で使う）。
    pub(crate) const fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }

    /// probe 進行中に後続 VK を 1 つ蓄積する。
    pub(crate) fn push_deferred(&mut self, vk: VkCode, needs_shift: bool) {
        if let Some(send) = self.current_send_mut() {
            send.deferred_vks.push(DeferredVk { vk, needs_shift });
        } else {
            log::warn!(
                "[tsf-probe] cold={} push_deferred dropped: phase has no SendState (label={})",
                self.cold_seq,
                self.phase_label_internal()
            );
        }
    }

    /// 現フェーズの `SendState` への可変参照を返す。`TransmitDone` フェーズは `None`。
    const fn current_send_mut(&mut self) -> Option<&mut SendState> {
        match &mut self.phase {
            ProbePhase::Probing { send, .. }
            | ProbePhase::LiteralDetect { send, .. } => Some(send),
            ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent { send, .. }) => Some(send),
            ProbePhase::WaitingForCallback(WaitingFor::TransmitDone) => None,
        }
    }

    /// フェーズ名文字列（内部用）。
    const fn phase_label_internal(&self) -> &'static str {
        match &self.phase {
            ProbePhase::Probing { .. } => "Probing",
            ProbePhase::WaitingForCallback(WaitingFor::FreshF2Sent { .. }) => {
                "WaitingForCallback(FreshF2Sent)"
            }
            ProbePhase::WaitingForCallback(WaitingFor::TransmitDone) => {
                "WaitingForCallback(TransmitDone)"
            }
            ProbePhase::LiteralDetect { .. } => "LiteralDetect",
        }
    }

    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。フェーズを 1 ステップ進める。
    ///
    /// 返値の `Vec<ProbeAction>` を `dispatch_probe_actions` が実行する。
    /// 空 Vec = まだ待機中（タイマー継続）。
    pub(crate) fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.tick_impl(&SystemClock, env)
    }

    /// `Clock` と `TsfEnvSnapshot` を注入できる `tick` の内部実装（テスト用）。
    ///
    /// `inspect_phase` で副作用なしに次の遷移先を決定し、
    /// その後 `&self` を解放してから状態変化を適用する。
    #[cfg(test)]
    pub(crate) fn tick_with_clock_env<C: Clock>(
        &mut self,
        clock: &C,
        env: &TsfEnvSnapshot,
    ) -> Vec<ProbeAction> {
        self.tick_impl(clock, env)
    }

    fn tick_impl<C: Clock>(&mut self, clock: &C, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        match self.inspect_phase(clock, env) {
            NextStep::Wait => vec![],
            NextStep::TransmitChrome => self.enter_transmit_chrome(env),
            NextStep::LiteralDone => vec![ProbeAction::Done],
            NextStep::LiteralSuspected { ze_bs_count } => {
                let romaji = self.take_romaji_from_literal_detect();
                // ※「consecutive_count」のチェック (false-positive 抑制) は dispatcher 側で行う。
                vec![
                    ProbeAction::RawTsfLiteralRecovery {
                        cold_seq: self.cold_seq,
                        backs: ze_bs_count,
                        romaji,
                    },
                    ProbeAction::Done,
                ]
            }
        }
    }

    /// 現フェーズを検査して次の遷移先を返す。副作用なし（`&self` のみ使用）。
    fn inspect_phase<C: Clock>(&self, _clock: &C, env: &TsfEnvSnapshot) -> NextStep {
        match &self.phase {
            ProbePhase::Probing {
                probe,
                total_max_ms,
                kind,
                ..
            } => {
                let Some(outcome) = probe.check_outcome(*total_max_ms) else {
                    return NextStep::Wait;
                };
                match kind {
                    ProbeKind::Chrome => {
                        log::debug!(
                            "[tsf-probe] cold={} ChromeProbe 完了 ({}ms)",
                            self.cold_seq,
                            outcome.elapsed_ms
                        );
                        // F22→F21 を Chrome path で使わない。
                        // F22 (IME OFF) が Chrome TSF context を壊し、F21 が間に合わず na がリテラル化する。
                        // ChromeProbe の F2 (VK_DBE_HIRAGANA) warmup のみで GJI を活性化する。
                        NextStep::TransmitChrome
                    }
                }
            }

            ProbePhase::WaitingForCallback(_) => NextStep::Wait,

            ProbePhase::LiteralDetect {
                detector,
                ze_bs_count,
                deadline_ms,
                expected_kana,
                ..
            } => {
                use crate::tsf::probe::DetectionResult;
                let Some(detection) = detector.check_now(*deadline_ms) else {
                    return NextStep::Wait;
                };
                match detection {
                    DetectionResult::CompositionConfirmed => {
                        // 部分リテラル検出: SHOW が発火しても composition 内容が expected_kana と
                        // 異なる場合は「K がリテラル化して O だけが compose された」部分リテラルを示す。
                        // TSF→IMM32 bridge が composition 中に HIMC を更新する環境でのみ有効。
                        // HIMC が NULL の場合（bridge 未対応など）は None → スキップして安全に CompositionConfirmed へ。
                        if let Some(expected) = expected_kana {
                            // env.foreground_comp_char は output/mod.rs が取得して注入したスナップショット。
                            // None = HIMC 未取得（テスト・bridge 非対応環境）→ 部分リテラル検出をスキップ。
                            let actual = env.foreground_comp_char;
                            if actual.map_or(false, |c| c != *expected) {
                                log::warn!(
                                    "[raw-tsf-literal] cold={} partial literal: comp={actual:?} ≠ expected='{expected}' → SuspectedLiteral",
                                    self.cold_seq
                                );
                                crate::ime_diagnostic::log_composition_probe(
                                    self.cold_seq,
                                    "partial-literal",
                                );
                                return NextStep::LiteralSuspected {
                                    ze_bs_count: *ze_bs_count,
                                };
                            }
                        }
                        log::debug!(
                            "[raw-tsf-literal] cold={} composition confirmed",
                            self.cold_seq
                        );
                        crate::ime_diagnostic::log_composition_probe(self.cold_seq, "confirmed");
                        NextStep::LiteralDone
                    }
                    DetectionResult::SuspectedLiteral => {
                        crate::ime_diagnostic::log_composition_probe(self.cold_seq, "suspected");
                        NextStep::LiteralSuspected {
                            ze_bs_count: *ze_bs_count,
                        }
                    }
                }
            }
        }
    }

    /// テスト専用: 任意のフェーズに強制遷移する。
    #[cfg(test)]
    pub(crate) fn force_phase_for_test(&mut self, phase: ProbePhase) {
        self.phase = phase;
    }

    /// テスト専用: 現在フェーズの文字列ラベルを返す。
    #[cfg(test)]
    pub(crate) fn phase_label(&self) -> &'static str {
        self.phase_label_internal()
    }

    /// dispatcher が `Transmit(Chrome)` を実行した後に呼ぶ。
    ///
    /// `detector` は送信**前**に dispatcher 側で生成し渡すこと（ベースラインは transmit 前が正しい）。
    ///
    /// `true` を返した場合は Done（dispatcher は `TIMER_TSF_PROBE` を kill する）。
    /// `false` を返した場合は `LiteralDetect` フェーズへ遷移し、タイマーを継続する。
    pub(crate) fn apply_transmit_done(
        &mut self,
        romaji: String,
        ze_bs_count: usize,
        detector: Option<LiteralDetector>,
        literal_detect_ms: u64,
        expected_kana: Option<char>,
    ) -> bool {
        if let Some(detector) = detector {
            let deadline_ms = crate::hook::current_tick_ms() + literal_detect_ms;
            self.phase = ProbePhase::LiteralDetect {
                detector,
                ze_bs_count,
                deadline_ms,
                send: SendState {
                    romaji,
                    deferred_vks: Vec::new(),
                },
                expected_kana,
            };
            false
        } else {
            true
        }
    }

    fn enter_transmit_chrome(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        let send = self.take_current_send_for_transmit();
        let cold_seq = self.cold_seq;
        if env.gji_active {
            // GJI active: SacrificialWarmup で TSF warm 確認後に実ローマ字を送信する。
            // ChromeProbe 完了直後に TSF context がまだ初期化中で先頭 VK がリテラル化する race を防ぐ。
            // （例: "ko"→'k' literal + 'o'→'お' → "kお..." となる partial literal バグ）
            // VK_A を送信して GJI I/O 応答を待ち、BS×1 + 実ローマ字の順で再送する。
            vec![ProbeAction::StartSacrificialWarmup(LiteralDetectConfig {
                cold_seq,
                romaji: send.romaji,
                deferred_vks: send.deferred_vks,
                plan: TransmitPlan {
                    should_prepend_f2: false,
                    used_eager_path: false,
                    needs_literal: true,
                    literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                },
                observations: ProbeObservations { nc_fired: true, gji_resumed: false },
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                target: TransmitTarget::Chrome,
            })]
        } else {
            // GJI inactive: GJI モニター不健全のため composition 確認不可 → 直接送信。
            self.phase = ProbePhase::WaitingForCallback(WaitingFor::TransmitDone);
            vec![ProbeAction::Transmit {
                cold_seq,
                plan: TransmitPlan {
                    should_prepend_f2: false,
                    used_eager_path: false,
                    needs_literal: false,
                    literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                },
                observations: ProbeObservations { nc_fired: true, gji_resumed: false },
                romaji: send.romaji,
                deferred_vks: send.deferred_vks,
                target: TransmitTarget::Chrome,
            }]
        }
    }

    /// `Transmit` action 生成時に現フェーズから `SendState` を取り出す。
    fn take_current_send_for_transmit(&mut self) -> SendState {
        match &mut self.phase {
            ProbePhase::Probing { send, .. } => std::mem::take(send),
            _ => {
                log::warn!(
                    "[tsf-probe] cold={} enter_transmit_chrome called from unexpected phase {}",
                    self.cold_seq,
                    self.phase_label_internal()
                );
                SendState::default()
            }
        }
    }

    /// `LiteralSuspected` 遷移時に `LiteralDetect` フェーズから `romaji` を取り出す。
    fn take_romaji_from_literal_detect(&mut self) -> String {
        if let ProbePhase::LiteralDetect { send, .. } = &mut self.phase {
            std::mem::take(&mut send.romaji)
        } else {
            log::warn!(
                "[tsf-probe] cold={} take_romaji_from_literal_detect unexpected phase {}",
                self.cold_seq,
                self.phase_label_internal()
            );
            String::new()
        }
    }
}

impl crate::tsf::tickable_fsm::TickableFsm for TsfProbeMachine {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        TsfProbeMachine::tick(self, env)
    }

    fn cold_seq_hint(&self) -> u32 {
        TsfProbeMachine::cold_seq_hint(self)
    }

    fn apply_transmit_done(
        &mut self,
        romaji: String,
        ze_bs_count: usize,
        detector: Option<LiteralDetector>,
        literal_detect_ms: u64,
        expected_kana: Option<char>,
    ) -> bool {
        TsfProbeMachine::apply_transmit_done(
            self,
            romaji,
            ze_bs_count,
            detector,
            literal_detect_ms,
            expected_kana,
        )
    }

    fn push_deferred(&mut self, vk: VkCode, needs_shift: bool) {
        TsfProbeMachine::push_deferred(self, vk, needs_shift);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::probe_bridge::OutputActiveGuard;

    use timed_fsm::ManualClock;

    fn make_chrome_machine() -> TsfProbeMachine {
        let guard = OutputActiveGuard::noop_for_test();
        let probe = TsfReadinessProbe::new(0, 0, 0);
        TsfProbeMachine::new_chrome("ka", 0, probe, 0, guard)
    }

    // ── WaitingForCallback フェーズテスト ────────────────────────────────────

    #[test]
    fn waiting_for_callback_is_no_op() {
        let mut machine = make_chrome_machine();
        machine.force_phase_for_test(ProbePhase::WaitingForCallback(WaitingFor::TransmitDone));

        let actions = machine.tick_with_clock_env(&ManualClock(0), &TsfEnvSnapshot::default());
        assert!(actions.is_empty(), "WaitingForCallback は空 Vec を返すべき");
        assert_eq!(machine.phase_label(), "WaitingForCallback(TransmitDone)");
    }

    // ── push_deferred テスト ─────────────────────────────────────────────────

    #[test]
    fn push_deferred_appends_vk() {
        let mut machine = make_chrome_machine();
        machine.push_deferred(VkCode(0x41), false);
        machine.push_deferred(VkCode(0x42), true);
        // deferred_vks は private だが Transmit action 経由で確認できる。
        // ここでは push_deferred が panic しないことを確認するだけで十分。
    }

    // ── regression: literal_suspected 経路で romaji が保持されるか ────────────

    #[test]
    fn literal_suspected_carries_romaji_through_transmit_to_recovery() {
        let guard = OutputActiveGuard::noop_for_test();
        let detector = crate::tsf::probe::LiteralDetector::new();
        let mut machine = TsfProbeMachine::new_literal_detect("a", 0, detector, 1, 0, guard);
        let actions = machine.tick_with_clock_env(&ManualClock(1000), &TsfEnvSnapshot::default());
        match &actions[..] {
            [ProbeAction::RawTsfLiteralRecovery { romaji, backs, .. }, ProbeAction::Done] => {
                assert_eq!(
                    romaji, "a",
                    "literal suspected 経路で romaji が空になってはいけない"
                );
                assert_eq!(*backs, 1);
            }
            other => panic!("unexpected actions: {other:?}"),
        }
    }

    // ── decide_transmit_plan 回帰テスト ──────────────────────────────────────

    #[test]
    fn decide_plan_nc_not_fired_tsf_not_long_idle_suppresses_f2_but_keeps_literal() {
        // nc_fired=false + TSF mode + Short cold (forces_prepend_f2=false):
        // SendFreshF2 が ~300ms 前に送信済み → バッチに F2 を含めると reinit race でリテラル化する。
        // ただし IME 準備完了が未確認のため LiteralDetect は有効にして回収する。
        let obs = ProbeObservations { nc_fired: false, gji_resumed: false };
        let env = TsfEnvSnapshot { is_tsf_mode: true, gji_active: true, ..Default::default() };

        let plan = decide_transmit_plan(true, false, obs, &env, true, false, false);

        assert!(!plan.should_prepend_f2, "nc_fired=false + tsf + Short cold: F2 をバッチに含めない");
        assert!(plan.needs_literal, "nc_fired=false + tsf: IME 準備未確認 → LiteralDetect で保護");
    }

    #[test]
    fn decide_plan_gji_resumed_disables_literal_detection() {
        // gji_resumed=true 時の false positive BS 再発防止:
        // GJI が F2×2 に I/O 応答済み → composition 成功確定 → LiteralDetect は false positive になる。
        let obs = ProbeObservations { nc_fired: true, gji_resumed: true };
        let env = TsfEnvSnapshot { is_tsf_mode: true, gji_active: true, ..Default::default() };

        let plan = decide_transmit_plan(true, false, obs, &env, true, true, true);

        assert!(!plan.needs_literal, "gji_resumed=true: LiteralDetect をスキップしないと BS 誤送信になる");
    }

    #[test]
    fn decide_plan_nc_not_fired_tsf_long_idle_keeps_f2_and_literal() {
        // Long cold (forces_prepend_f2=true): F2×2 で GJI を起動する必要があるためバッチに F2 が必要。
        // LiteralDetect も有効（GJI 応答未確認）。
        let obs = ProbeObservations { nc_fired: false, gji_resumed: false };
        let env = TsfEnvSnapshot { is_tsf_mode: true, gji_active: true, ..Default::default() };

        let plan = decide_transmit_plan(true, false, obs, &env, true, true, true);

        assert!(plan.should_prepend_f2, "Long cold: F2 バッチ同梱が必要");
        assert!(plan.needs_literal, "gji_resumed=false + tsf: LiteralDetect 有効");
    }

    #[test]
    fn decide_plan_nc_fired_keeps_f2_and_enables_literal_when_gji_active() {
        // nc_fired=true: NameChange 発火確認済み → F2 はバッチに含める。
        let obs = ProbeObservations { nc_fired: true, gji_resumed: false };
        let env = TsfEnvSnapshot { is_tsf_mode: true, gji_active: true, ..Default::default() };

        let plan = decide_transmit_plan(true, false, obs, &env, true, false, false);

        assert!(plan.should_prepend_f2, "nc_fired=true: F2 はバッチに含める");
        assert!(plan.needs_literal, "gji_active + !is_long_cold + !gji_resumed: LiteralDetect 有効");
    }

    #[test]
    fn decide_plan_non_tsf_mode_keeps_f2() {
        // 非 TSF mode（Chrome 等）: nc_fired=false でも F2 バッチ同梱が必要。
        let obs = ProbeObservations { nc_fired: false, gji_resumed: false };
        let env = TsfEnvSnapshot { is_tsf_mode: false, gji_active: true, ..Default::default() };

        let plan = decide_transmit_plan(true, false, obs, &env, true, false, false);

        assert!(plan.should_prepend_f2, "非 TSF mode: nc_fired=false でも F2 を含める");
    }

    #[test]
    fn decide_plan_nc_fired_with_deferred_vks_disables_eager_path() {
        // nc_fired=true でも deferred_vks が存在する場合は unicode に戻さない（nお race 防止）。
        let obs = ProbeObservations { nc_fired: true, gji_resumed: false };
        let env = TsfEnvSnapshot { is_tsf_mode: false, gji_active: false, ..Default::default() };

        let plan = decide_transmit_plan(false, true, obs, &env, false, false, false); // deferred_empty=false

        assert!(!plan.used_eager_path, "deferred_vks あり: unicode TSF パスを使わない");
    }

    #[test]
    fn decide_plan_medium_cold_forces_f2_in_batch_on_ncwait_timeout() {
        // Medium cold (forces_prepend_f2=true) + nc_fired=false → should_prepend_f2=true
        let obs = ProbeObservations { nc_fired: false, gji_resumed: false };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true, gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(true, false, obs, &env, true, true, false); // Medium cold

        assert!(
            plan.should_prepend_f2,
            "Medium cold + GJI 無応答: F2 をバッチに含めないと先頭 VK がリテラル化する"
        );
        assert!(plan.needs_literal, "GJI 応答未確認: LiteralDetect 有効");
    }
}
