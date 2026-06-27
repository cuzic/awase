//! TSF/Chrome cold-start probe コルーチン実装。
//!
//! [`TsfProbeCoro`] は 10ms 間隔の `TIMER_TSF_PROBE` ハンドラから駆動される。
//! `tick()` がフェーズを 1 ステップ進め、dispatcher が返す [`ProbeAction`] を
//! `platform.rs::dispatch_probe_actions` が実行する。
//!
//! ## フェーズ遷移
//!
//! ```text
//! ChromeProbe ──► [gji_active]──► StartSacrificialWarmup（SacrificialWarmupCoro に委譲）
//!              └─[!gji_active]──► Transmit(Chrome) ─[needs_literal=false]─► Done
//!                                                   └─[needs_literal=true]──► LiteralDetect ─► Done
//! ```
//!
//! ## 設計ポリシー
//!
//! - `tick()` / `apply_*` は副作用なし。状態のみ更新し [`ProbeAction`] を返す。
//! - SendInput / mark_warm / mark_cold / RAW_TSF_LITERAL 操作は dispatcher 側で実行する
//!   (`platform.rs::dispatch_probe_actions`)。
//! - フェーズ遷移は `StepCoro` async 本体に直線記述し、`ProbePhase` enum は不要。

use std::rc::Rc;

use awase::types::VkCode;

/// probe 進行中に蓄積する後続 VK。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeferredVk {
    pub(crate) vk: VkCode,
    pub(crate) needs_shift: bool,
}
use crate::tsf::probe::{LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::step_coro::{yield_step, Channel, CoroStep, StepCoro};
use crate::tsf::tickable_fsm::TickableFsm;

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
    /// `GjiWarmupCoro` の NameChangeWait 早期脱出判定に使用する。
    pub gji_candidate_visible: bool,
    /// 現在の IME 入力モード belief（Off / Hiragana / Katakana / Unknown）。
    pub ime_mode: crate::tsf::ime_mode_fsm::ImeModeState,
    /// `ime_mode` が `IMC_GETCONVERSIONMODE` で OS から確認済みなら true。
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
/// `GjiWarmupCoro` 内から呼ばれるが、単独でテストも可能。副作用なし。
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

/// [`ProbeAction::Transmit`] の送信先。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransmitTarget {
    Tsf,
    Chrome,
}

/// LiteralDetect / SacrificialWarmup フェーズに引き渡す設定。
///
/// [`ProbeAction::StartSacrificialWarmup`] のペイロード。
#[derive(Debug)]
pub(crate) struct LiteralDetectConfig {
    pub cold_seq: u32,
    pub romaji: String,
    pub deferred_vks: Vec<DeferredVk>,
    pub plan: TransmitPlan,
    pub observations: ProbeObservations,
    pub literal_detect_ms: u64,
    /// 送信先ターゲット。SacrificialWarmup の resend フェーズで Chrome/TSF を切り替える。
    pub target: TransmitTarget,
    /// `true` = `LiteralDetectFsm` が partial literal / SuspectedLiteral を検出して
    /// 回収目的で emit した（consecutive インクリメントが必要）。
    /// `false` = `GjiWarmupFsm` の通常 cold-start パスで emit した。
    pub from_literal_recovery: bool,
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
    /// [`TsfProbeCoro::apply_fresh_f2_sent`] を呼ぶ。
    SendFreshF2 { cold_seq: u32, probe_settled: bool },
    /// TSF または Chrome バッチパイプラインで `romaji` を送信する。
    ///
    /// `target == Tsf`: `deferred_vks` をフラッシュし warm マークしたあと
    /// [`TsfProbeCoro::apply_transmit_done`] を呼ぶ。
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
    /// GJI warmup 完了後に犠牲キー（VK_A）暖機フェーズを開始する。
    ///
    /// `GjiWarmupCoro` が `needs_literal=true` かつ `is_long_cold` と判断したときに emit する。
    /// dispatcher が VK_A を送信してから [`crate::tsf::sacr_warmup_fsm::SacrificialWarmupFsm`]
    /// に切り替える。実ローマ字は TSF warm 確認後に [`SacrificialResend`] 経由で送信される。
    StartSacrificialWarmup(LiteralDetectConfig),
    /// [`crate::tsf::sacr_warmup_fsm::SacrificialWarmupFsm`] が composition 確認後に emit する。
    ///
    /// dispatcher が BS×1（犠牲 VK_A 削除）→ 実ローマ字 transmit_tsf → deferred_vks 送信を行う。
    SacrificialResend(SacrificialResend),
    /// Chrome sacr-warmup cold タイムアウト後の GJI 再初期化を dispatcher に委譲する。
    ///
    /// dispatcher が F22→F21 を SendInput でキューイング + `ImeModeFsm` belief 更新 +
    /// async `IMC_GETCONVERSIONMODE` ポーリングを開始する。
    /// FSM 切り替えは不要（[`SacrificialWarmupCoro`] がそのまま IME 確認を待機する）。
    ///
    /// [`crate::tsf::sacr_warmup_coro::SacrificialWarmupCoro`] が emit する。
    SendChromeGjiReinit { cold_seq: u32 },
    /// partial literal / SuspectedLiteral 回収前の terminal cleanup 用 BS 送信。
    ///
    /// [`LiteralDetectFsm`] が TSF mode + consecutive==0 のときに `StartSacrificialWarmup` の直前に
    /// emit する。dispatcher は `ProbeIo::send_literal_recovery_bs` を呼び出す。
    SendRecoveryBs {
        cold_seq: u32,
        backs: usize,
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

// ── TickInput ─────────────────────────────────────────────────────────────────

struct TsfProbeTickInput {
    env: TsfEnvSnapshot,
    new_deferred: Vec<DeferredVk>,
    transmit_done: Option<TsfTransmitDonePayload>,
}

/// `apply_transmit_done(Some(detector))` のペイロード。次 tick で inline LiteralDetect に入る。
struct TsfTransmitDonePayload {
    romaji: String,
    ze_bs_count: usize,
    detector: LiteralDetector,
    /// `apply_transmit_done` 呼び出し時点の `current_tick_ms() + literal_detect_ms`。
    deadline_ms: u64,
    expected_kana: Option<char>,
}

// ── コルーチン本体 ────────────────────────────────────────────────────────────

async fn tsf_probe_coro_body(
    ch: Rc<Channel<TsfProbeTickInput, Vec<ProbeAction>>>,
    romaji: String,
    probe: TsfReadinessProbe,
    total_max_ms: u64,
    cold_seq: u32,
) {
    let mut deferred_vks: Vec<DeferredVk> = vec![];

    // ── Phase 1: ChromeProbe ポーリング ──────────────────────────────────────
    let env = loop {
        let input = yield_step(ch.clone(), vec![]).await;
        deferred_vks.extend(input.new_deferred);
        let Some(outcome) = probe.check_outcome(total_max_ms) else {
            continue;
        };
        log::debug!(
            "[tsf-probe] cold={cold_seq} ChromeProbe 完了 ({}ms)",
            outcome.elapsed_ms
        );
        // F22→F21 を Chrome path で使わない。
        // F22 (IME OFF) が Chrome TSF context を壊し、F21 が間に合わず na がリテラル化する。
        // ChromeProbe の F2 (VK_DBE_HIRAGANA) warmup のみで GJI を活性化する。
        break input.env;
    };

    // ── Phase 2a: gji_active → SacrificialWarmupCoro に委譲 ──────────────────
    if env.gji_active {
        // GJI active: SacrificialWarmup で TSF warm 確認後に実ローマ字を送信する。
        // ChromeProbe 完了直後に TSF context がまだ初期化中で先頭 VK がリテラル化する race を防ぐ。
        yield_step(
            ch,
            vec![ProbeAction::StartSacrificialWarmup(LiteralDetectConfig {
                cold_seq,
                romaji,
                deferred_vks,
                plan: TransmitPlan {
                    should_prepend_f2: false,
                    used_eager_path: false,
                    needs_literal: true,
                    literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                },
                observations: ProbeObservations { nc_fired: true, gji_resumed: false },
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                target: TransmitTarget::Chrome,
                from_literal_recovery: false,
            })],
        )
        .await;
        return; // SacrificialWarmupCoro が引き継ぐ → このコルーチンは破棄
    }

    // ── Phase 2b: !gji_active → GJI モニター不健全のため直接 Chrome 送信 ─────
    let transmit_input = yield_step(
        ch.clone(),
        vec![ProbeAction::Transmit {
            cold_seq,
            plan: TransmitPlan {
                should_prepend_f2: false,
                used_eager_path: false,
                needs_literal: false,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            observations: ProbeObservations { nc_fired: true, gji_resumed: false },
            romaji,
            deferred_vks,
            target: TransmitTarget::Chrome,
        }],
    )
    .await;

    // ── Phase 3: Inline LiteralDetect（dispatcher が detector を渡した場合のみ）─
    let Some(TsfTransmitDonePayload {
        romaji: recovery_romaji,
        ze_bs_count,
        detector,
        deadline_ms,
        expected_kana,
    }) = transmit_input.transmit_done
    else {
        return;
    };

    loop {
        use crate::tsf::probe::DetectionResult;
        let detect_input = yield_step(ch.clone(), vec![]).await;
        let env = detect_input.env;

        let Some(detection) = detector.check_now(deadline_ms) else {
            continue;
        };

        // 部分リテラル判定: SHOW 後に composition 内容が expected_kana と異なるケース。
        // TSF→IMM32 bridge が composition 中に HIMC を更新する環境でのみ有効。
        let partial_literal = matches!(detection, DetectionResult::CompositionConfirmed)
            && expected_kana.map_or(false, |e| {
                env.foreground_comp_char.map_or(false, |c| c != e)
            });

        let final_actions = match detection {
            DetectionResult::CompositionConfirmed if partial_literal => {
                log::warn!(
                    "[raw-tsf-literal] cold={cold_seq} partial literal: comp={:?} ≠ expected='{:?}' → SuspectedLiteral",
                    env.foreground_comp_char, expected_kana
                );
                crate::ime_diagnostic::log_composition_probe(cold_seq, "partial-literal");
                vec![
                    ProbeAction::RawTsfLiteralRecovery {
                        cold_seq,
                        backs: ze_bs_count,
                        romaji: recovery_romaji,
                    },
                    ProbeAction::Done,
                ]
            }
            DetectionResult::SuspectedLiteral => {
                crate::ime_diagnostic::log_composition_probe(cold_seq, "suspected");
                vec![
                    ProbeAction::RawTsfLiteralRecovery {
                        cold_seq,
                        backs: ze_bs_count,
                        romaji: recovery_romaji,
                    },
                    ProbeAction::Done,
                ]
            }
            _ => {
                log::debug!("[raw-tsf-literal] cold={cold_seq} composition confirmed");
                crate::ime_diagnostic::log_composition_probe(cold_seq, "confirmed");
                vec![ProbeAction::Done]
            }
        };

        yield_step(ch, final_actions).await;
        return;
    }
}

// ── TsfProbeCoro ──────────────────────────────────────────────────────────────

/// TSF/Chrome cold-start プローブ コルーチン。`TsfProbeMachine` の後継。
///
/// `pending_tsf` (`RefCell<Option<Box<dyn TickableFsm>>>`) に格納し、
/// `TIMER_TSF_PROBE` ハンドラが `tick()` で 1 ステップ進める。
pub(crate) struct TsfProbeCoro {
    coro: StepCoro<TsfProbeTickInput, Vec<ProbeAction>>,
    pending_deferred: Vec<DeferredVk>,
    pending_transmit_done: Option<TsfTransmitDonePayload>,
    cold_seq: u32,
    /// RAII guard。drop で `OUTPUT_GATE.active=false`。
    _guard: OutputActiveGuard,
}

impl TsfProbeCoro {
    /// Chrome F2 cold warmup (`send_romaji_batched` の cold パス) 用コンストラクタ。
    pub(crate) fn new_chrome(
        romaji: &str,
        cold_seq: u32,
        probe: TsfReadinessProbe,
        total_max_ms: u64,
        guard: OutputActiveGuard,
    ) -> Self {
        let romaji_owned = romaji.to_string();
        let coro = StepCoro::new(|ch| {
            tsf_probe_coro_body(ch, romaji_owned, probe, total_max_ms, cold_seq)
        });
        Self {
            coro,
            pending_deferred: vec![],
            pending_transmit_done: None,
            cold_seq,
            _guard: guard,
        }
    }
}

impl TickableFsm for TsfProbeCoro {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        let input = TsfProbeTickInput {
            env: *env,
            new_deferred: std::mem::take(&mut self.pending_deferred),
            transmit_done: self.pending_transmit_done.take(),
        };
        match self.coro.step(input) {
            CoroStep::Yielded(actions) => actions,
            CoroStep::Complete => vec![ProbeAction::Done],
        }
    }

    fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }

    /// dispatcher が `Transmit(Chrome)` を実行した後に呼ぶ。
    ///
    /// `detector` が `Some` なら `LiteralDetect` フェーズへ進む（`false` を返す）。
    /// `None` なら Done（`true` を返す）。
    fn apply_transmit_done(
        &mut self,
        romaji: String,
        ze_bs_count: usize,
        detector: Option<LiteralDetector>,
        literal_detect_ms: u64,
        expected_kana: Option<char>,
    ) -> bool {
        match detector {
            Some(det) => {
                let deadline_ms = crate::hook::current_tick_ms() + literal_detect_ms;
                self.pending_transmit_done = Some(TsfTransmitDonePayload {
                    romaji,
                    ze_bs_count,
                    detector: det,
                    deadline_ms,
                    expected_kana,
                });
                false
            }
            None => true,
        }
    }

    fn push_deferred(&mut self, vk: VkCode, needs_shift: bool) {
        self.pending_deferred.push(DeferredVk { vk, needs_shift });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::probe_bridge::OutputActiveGuard;

    fn make_chrome_coro() -> TsfProbeCoro {
        let guard = OutputActiveGuard::noop_for_test();
        let probe = TsfReadinessProbe::new(0, 0, 0);
        TsfProbeCoro::new_chrome("ka", 0, probe, 0, guard)
    }

    // ── push_deferred テスト ─────────────────────────────────────────────────────

    #[test]
    fn push_deferred_appends_vk() {
        let mut coro = make_chrome_coro();
        coro.push_deferred(VkCode(0x41), false);
        coro.push_deferred(VkCode(0x42), true);
        // push_deferred が panic しないことを確認する。
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
