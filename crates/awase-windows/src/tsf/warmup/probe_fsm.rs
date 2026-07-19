//! TSF/Chrome cold-start probe コルーチン実装。
//!
//! [`TsfProbeCoro`] は 10ms 間隔の `TIMER_TSF_PROBE` ハンドラから駆動される。
//! `tick()` がフェーズを 1 ステップ進め、dispatcher が返す [`ProbeAction`] を
//! `platform.rs::dispatch_probe_actions` が実行する。
//!
//! ## フェーズ遷移
//!
//! ```text
//! ChromeProbe ──► Transmit(Chrome) ─[needs_literal=false]─► Done
//!                                   └─[needs_literal=true]──► LiteralDetect / per-VK confirm ─► Done
//! ```
//!
//! cold-start の予防的犠牲キー（VK_A probe + Chrome reinit のフルコース、
//! `StartSacrificialWarmup`）は 2026-07-18 に撤去した。per-VK confirm
//! （[`run_per_vk_confirm`]）が送信後の confirm/recovery を担うため、送信前に
//! GJI の準備を待つ予防は二重の保険だった（実機ソーク数日、無破損。
//! `docs/known-bugs.md` 参照）。
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
use crate::tsf::warmup::tickable_fsm::TickableFsm;
use timed_fsm::coro::{yield_step, Channel, CoroStep, StepCoro};

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
    /// 現在の IME 入力モード belief（Off / Hiragana / Katakana / Unknown）。
    pub ime_mode: crate::tsf::ime_mode_fsm::ImeModeState,
    /// `ime_mode` が `IMC_GETCONVERSIONMODE` で OS から確認済みなら true。
    pub ime_mode_confirmed: bool,
    /// `TsfWarmupCoordinator` の deferred キューに現在何か積まれているか（覗き見、消費しない）。
    /// `GjiWarmupCoro` の `decide_transmit_plan` eager path 判定に使う。
    pub deferred_pending: bool,
}

/// probe 中に観測した事実。`decide_transmit_plan` の入力に使う。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProbeObservations {
    pub nc_fired: bool,
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

/// BUG-29: 候補ウィンドウが既に表示中なら、Chrome per-VK confirm の
/// literal-detect polling をスキップしてよいかを判定する（純粋関数、テスト用に分離）。
///
/// 候補ウィンドウが表示中であること自体が「warm な composition が継続している」
/// 直接証拠であるため、SHOW イベント（エッジトリガで VK1 以降は再発火しない）や
/// WriteTransferCount 閾値（子音単体 VK では原理的に越えない）に頼らず即
/// confirmed とみなせる。`crates/awase-windows/src/tsf/probe.rs:676-680` に
/// 記載された既知の限界を構造的に解消する。
pub(crate) const fn should_skip_literal_wait(candidate_visible: bool) -> bool {
    candidate_visible
}

/// probe 観測値・環境スナップショット・コンテキストから送信方針を決定する純粋計算関数。
///
/// `GjiWarmupCoro` 内から呼ばれるが、単独でテストも可能。副作用なし。
pub(crate) fn decide_transmit_plan(
    initial_used_eager: bool,
    obs: ProbeObservations,
    env: &TsfEnvSnapshot,
    deferred_empty: bool,
    forces_prepend_f2: bool,
    is_long_cold: bool,
) -> TransmitPlan {
    // romaji バッチへの F2 直接同梱（第3の防御層）は DIAG_DISABLE_PROACTIVE_TSF_WARMUP を
    // 2026-07-19 に恒久化したことで撤去し、常に false になった。reactive な LiteralDetect
    // のみに委ねる（`docs/known-bugs.md` BUG-24 参照。再度有効化する場合はこのコミットの
    // revert が必要）。
    let should_prepend_f2 = false;

    // nc_fired=true: IME モード確認済み（Medium/Long cold かつ deferred なし → VK path）。
    // nc_fired=false + TSF mode: VK path 固定（unicode は GJI composition をバイパスし "nお" race が起きる）。
    let used_eager_path = if obs.nc_fired {
        (initial_used_eager || forces_prepend_f2) && deferred_empty
    } else if env.is_tsf_mode {
        false
    } else {
        initial_used_eager || forces_prepend_f2
    };

    // NameChangeWait タイムアウトで IME 準備が未確認のまま transmit したケースを
    // LiteralDetect で回収する。F2 をバッチに含めない場合も IME が cold の可能性がある。
    // （既知バグ: gji_idle ~1.4s で 300ms NameChangeWait が間に合わなかったケース）
    let needs_literal = !obs.nc_fired && env.is_tsf_mode && env.gji_active;

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

/// ステートマシン → dispatcher 方向の宣言的アクション。
#[derive(Debug)]
pub(crate) enum ProbeAction {
    /// TSF または Chrome バッチパイプラインで `romaji` を送信する。
    ///
    /// dispatcher は romaji 送信直後に `TsfWarmupCoordinator` の deferred キューを
    /// フラッシュしてから warm マークし、`target == Tsf` なら
    /// [`TsfProbeCoro::apply_transmit_done`] を呼ぶ。
    Transmit {
        cold_seq: u32,
        /// FSM が確定した実行方針。dispatcher はそのまま実行する（再導出不要）。
        plan: TransmitPlan,
        romaji: String,
        target: TransmitTarget,
    },
    /// IME セッション最初の1文字専用（BUG-24 追補）: romaji の VK を1つだけ送信する。
    ///
    /// `is_partial_literal()` が送信前の無関係な代理指標（`nc_fired`）に
    /// 頼っているため、cold 直後の最初の1文字は VK を1個ずつ送って「送ったVK自身」への
    /// `CompositionConfirmed`/`SuspectedLiteral` を確認してから次の VK を送る。
    /// dispatcher は送信直前に `LiteralDetector::new()` でベースラインを取り、1 VK だけ
    /// `KeyInjector::send_vk_pair` で送信する。`is_last=true` のときのみ、通常の
    /// `Transmit` と同じ deferred VK フラッシュ・GjiFsm warmup 結果保存を行う。
    TransmitSingleVk {
        cold_seq: u32,
        vk: VkCode,
        needs_shift: bool,
        /// この VK の confirm 待ちタイムアウト（ms）。`plan.literal_detect_ms` を渡す。
        timeout_ms: u64,
        is_last: bool,
        /// 送信先（`Tsf`: WezTerm 等 TSF-native、`Chrome`: Chrome/Edge）。
        /// dispatcher が VK 送信関数（`send_single_tsf_vk`/`send_single_chrome_vk`）と
        /// detector 構築方式（SHOW ベース/write-bytes ベース）を切り替えるために使う。
        target: TransmitTarget,
    },
    /// `RAW_TSF_LITERAL` を設定し、composition を `RawTsfLiteralRecovery` で cold マークする。
    RawTsfLiteralRecovery {
        cold_seq: u32,
        backs: usize,
        romaji: String,
        /// `true` = partial literal（candidate 表示中に一部だけ literal 化）回収。
        /// バックスペース前に `VK_ESCAPE` を送って composition を確実に破棄する。
        escape_composition: bool,
    },
    /// Unicode モードで GJI write が観測されなかった。
    ///
    /// [`crate::tsf::warmup::unicode_literal_observer::UnicodeLiteralObserverFsm`] が emit する。
    /// dispatcher は `DispatchResult::LearnedTsf` を返し、呼び出し元 (`advance_tsf_probe`) が
    /// フォーカス中クラスを `InjectionModeStore` に学習し injection_mode を Tsf に昇格させる。
    UpgradeToTsf,
    /// Unicode cold-start warmup 完了後にバッファ済み文字を送信する。
    ///
    /// [`crate::tsf::warmup::unicode_cold_warmup_fsm::UnicodeColdWarmupFsm`] が GJI write 確認
    /// またはタイムアウト後に emit する。
    /// dispatcher が各 `char` を `send_unicode_char_direct()` で送信する。
    FlushDeferredUnicodeChars(Vec<char>),
    /// `DetectionResult::CompositionConfirmed`（非 partial）を確認した。
    ///
    /// dispatcher は必ず `consecutive_count`（`RawTsfLiteralRecovery` 連続発火数）を
    /// リセットする。`consecutive_count` は「連続失敗」の抑止用カウンタであり、
    /// 途中で本物の confirm が挟まれば連続ではなくなる（BUG-27 追補4:
    /// 従来は `CompositionConfirmed` では一度もリセットされず、セッション中に
    /// 一度でも literal 化すると以後ずっと give-up＝backspace のみに固定される
    /// regression があった）。
    ///
    /// `mark_literal_session=true` の場合、このセッションの literal-detect 自体を
    /// スキップ対象としてマークする（`tsf::observer::mark_literal_session_confirmed`）。
    /// per-VK confirm では各 VK の confirm で `mark_literal_session=false`、
    /// 全 VK 確認済みの最終確認でのみ `true` を使う。
    CompositionConfirmed { mark_literal_session: bool },
    /// プローブ完了。dispatcher は `TIMER_TSF_PROBE` を kill する。
    Done,
}

// ── TickInput ─────────────────────────────────────────────────────────────────

/// probe コルーチン (`TsfProbeCoro`/`GjiWarmupCoro`) 共有の tick 入力。
///
/// 統合前（2026-07-17 以前）は `TsfProbeTickInput`（Chrome）/ `TickInput`（TSF）として
/// 別々に定義されていたが、フィールド構成が完全に同一だったため共有した。
pub(crate) struct ProbeTickInput {
    pub(crate) env: TsfEnvSnapshot,
    pub(crate) transmit_done: Option<TransmitDonePayload>,
    pub(crate) vk_sent: Option<VkSentPayload>,
}

/// `apply_transmit_done(Some(detector))` のペイロード。次 tick で inline LiteralDetect に入る。
/// Chrome (`TsfProbeCoro`) / TSF (`GjiWarmupCoro`) で共有。`expected_kana` は Chrome の
/// inline LiteralDetect（Phase 3、partial literal 判定）のみが使用し、TSF 側
/// （`LiteralDetectCore` に委譲）は常に `None` を渡す。
pub(crate) struct TransmitDonePayload {
    pub(crate) romaji: String,
    pub(crate) ze_bs_count: usize,
    pub(crate) detector: LiteralDetector,
    /// `apply_transmit_done` 呼び出し時点の `current_tick_ms() + literal_detect_ms`。
    pub(crate) deadline_ms: u64,
    pub(crate) expected_kana: Option<char>,
}

/// `apply_vk_sent` のペイロード。次 tick の [`ProbeTickInput`] に載り、コルーチン本体が
/// そのVK専用の detector をポーリングする。Chrome per-VK confirm (`TsfProbeCoro`) と
/// TSF per-VK confirm (`GjiWarmupCoro`) で共有する（統合前は別々に定義されていた）。
pub(crate) struct VkSentPayload {
    pub(crate) detector: LiteralDetector,
    pub(crate) deadline_ms: u64,
}

// ── per-VK confirm 共有実装（Chrome Phase 2c・TSF Phase 5b、2026-07-17 統合）───

/// per-VK confirm ループの共通実装。romaji を1文字ずつ [`ProbeAction::TransmitSingleVk`]
/// で送信し、その VK 自身への `CompositionConfirmed`/`SuspectedLiteral` を都度確認してから
/// 次の VK へ進む。バッチ送信特有の「どの文字が化けたか区別できない」曖昧さ（BUG-03 が
/// Chrome で解決できなかった原因）を、そもそもバッチにしないことで回避する。
///
/// 統合前は `probe_fsm.rs::tsf_probe_coro_body`（Chrome Phase 2c）と
/// `gji_warmup_coro.rs::gji_coro_body`（TSF Phase 5b）にほぼ同一のループが重複していた。
/// 差分は BUG-29（候補ウィンドウ可視時の polling スキップ、Chrome のみ）とログ/診断タグの
/// 接頭辞のみで、いずれも `target` から導出できるためここに統合できた。
///
/// 全 VK 確認できたらセッション確認 (`mark_literal_session=true`) + `Done` を、
/// `SuspectedLiteral` を検出したら `RawTsfLiteralRecovery` を、dispatcher が
/// `apply_vk_sent` を呼ばなかった場合（BUG-27）は無リカバリで中断する。いずれの場合も
/// 必要なアクションは内部で yield 済み（または `CoroStep::Complete` の `Done` フォール
/// バックに委ねる）で、呼び出し元は `.await` 後にそのまま `return` すればよい。
#[expect(clippy::future_not_send)]
pub(crate) async fn run_per_vk_confirm(
    ch: Rc<Channel<ProbeTickInput, Vec<ProbeAction>>>,
    cold_seq: u32,
    romaji: &str,
    plan: TransmitPlan,
    target: TransmitTarget,
) {
    use crate::tsf::probe::DetectionResult;

    // ログ・診断タグは Chrome/TSF で従来通り書き分ける（docs/known-bugs.md のデバッグ
    // キーワード表・実機ログ grep パターンとの互換性を保つため）。
    let (log_tag, literal_tag, confirmed_tag) = match target {
        TransmitTarget::Chrome => (
            "tsf-probe",
            "chrome-per-vk-literal",
            "chrome-per-vk-confirmed",
        ),
        TransmitTarget::Tsf => (
            "gji-coro",
            "setopen-per-vk-literal",
            "setopen-per-vk-confirmed",
        ),
    };

    let vk_chars: Vec<(VkCode, bool)> = romaji
        .chars()
        .filter_map(crate::output::resolve_ascii_to_vk)
        .collect();
    if vk_chars.is_empty() {
        yield_step(ch.clone(), vec![ProbeAction::Done]).await;
        return;
    }

    let last_idx = vk_chars.len() - 1;
    // BUG-27 追補4: 前の VK の CompositionConfirmed で emit する consecutive_count
    // リセット action を、次の TransmitSingleVk の yield に相乗りさせる（コルーチンの
    // 構造上、confirm 直後に単独で yield するポイントが無いため）。
    let mut pending_confirm: Option<ProbeAction> = None;
    for (idx, &(vk, needs_shift)) in vk_chars.iter().enumerate() {
        let is_last = idx == last_idx;
        let mut actions = Vec::with_capacity(2);
        if let Some(confirm) = pending_confirm.take() {
            actions.push(confirm);
        }
        actions.push(ProbeAction::TransmitSingleVk {
            cold_seq,
            vk,
            needs_shift,
            timeout_ms: plan.literal_detect_ms,
            is_last,
            target,
        });
        let vk_input = yield_step(ch.clone(), actions).await;
        let Some(sent) = vk_input.vk_sent else {
            // BUG-27 追補2（2026-07-17、revert）: 一度は SuspectedLiteral と同じ
            // backspace+romaji 再送リカバリに倒したが、実機で msedge の
            // Chrome_WidgetWin_1 において `vk_sent 未設定` が**打鍵のたびに毎回**
            // 発火することが判明した。consecutive raw-tsf-literal count が単調増加して
            // 二度と 0 に戻らず、常に「give up, backspace ×1 のみ（再送なし）」分岐に
            // 落ちるため、正しく入力できていた文字まで毎回 backspace で消え、実質何も
            // 入力できなくなった。無リカバリの `return` に戻す
            // （docs/known-bugs.md BUG-27 参照）。
            log::warn!(
                "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] vk_sent 未設定 → 中断"
            );
            return;
        };

        // BUG-29: Chrome のみ、候補ウィンドウが既に表示中なら literal-detect の
        // polling をスキップする。SHOW はエッジトリガ（hidden→visible 遷移でのみ増分）
        // のため VK1 以降では再発火せず、子音単体 VK は WriteTransferCount 閾値も
        // 原理的に越えない（probe.rs:676-680, 658-667 に既知の限界として記載済み）。
        // TSF 側はこの早期脱出を経験的に必要としていない（従来から常時 polling）ため
        // 据え置く。
        let detection = if target == TransmitTarget::Chrome
            && should_skip_literal_wait(crate::tsf::observer::gji_candidate_visible_now())
        {
            log::debug!(
                "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] \
                 candidate window already visible → skip literal-detect wait (vk=0x{:02X})",
                vk.0,
            );
            DetectionResult::CompositionConfirmed
        } else {
            loop {
                let poll_input = yield_step(ch.clone(), vec![]).await;
                if let Some(d) = sent.detector.check_now(sent.deadline_ms) {
                    break d;
                }
                let _ = poll_input;
            }
        };

        match detection {
            DetectionResult::CompositionConfirmed => {
                log::debug!(
                    "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] confirmed (vk=0x{:02X})",
                    vk.0,
                );
                // BUG-27 追補4: この VK 自身の confirm で consecutive_count を
                // リセットする（セッション確認はまだ、全 VK 確認後にまとめて行う）。
                pending_confirm = Some(ProbeAction::CompositionConfirmed {
                    mark_literal_session: false,
                });
            }
            DetectionResult::SuspectedLiteral => {
                let (backs, escape_composition) =
                    crate::tsf::warmup::literal_detect_fsm::per_vk_recovery_params(idx);
                log::debug!(
                    "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] suspected literal \
                     (vk=0x{:02X} escape={escape_composition})",
                    vk.0,
                );
                crate::ime_diagnostic::log_composition_probe(cold_seq, literal_tag);
                // emit_recovery_actions は常に RawTsfLiteralRecovery（backspace のみ、
                // 捨て駒キーには倒れない）を返す。consecutive==0 なら dispatcher が
                // romaji の再送を自然に次の cold パス（per-VK confirm）へ委ねる。
                let actions = crate::tsf::warmup::literal_detect_fsm::emit_recovery_actions(
                    cold_seq,
                    romaji.to_string(),
                    backs,
                    escape_composition,
                );
                yield_step(ch.clone(), actions).await;
                return;
            }
        }
    }

    log::debug!(
        "[{log_tag}] cold={cold_seq} per-VK: 全 {} VK 確認済み → セッション確認",
        vk_chars.len(),
    );
    crate::ime_diagnostic::log_composition_probe(cold_seq, confirmed_tag);
    // BUG-27 追補4: 最後の VK の pending_confirm は不要（mark_literal_session=true
    // 版のリセットで包含されるため）、破棄してよい。
    let _ = pending_confirm.take();
    yield_step(
        ch.clone(),
        vec![
            ProbeAction::CompositionConfirmed {
                mark_literal_session: true,
            },
            ProbeAction::Done,
        ],
    )
    .await;
}

// ── コルーチン本体 ────────────────────────────────────────────────────────────

// `Rc` を使うため生成される future は `!Send`。これはタイマー駆動の単一スレッド設計
// による意図的な制約（crates/timed-fsm/src/coro.rs::yield_step 参照）。
#[expect(clippy::future_not_send)]
async fn tsf_probe_coro_body(
    ch: Rc<Channel<ProbeTickInput, Vec<ProbeAction>>>,
    romaji: String,
    probe: TsfReadinessProbe,
    total_max_ms: u64,
    cold_seq: u32,
) {
    // ── Phase 1: ChromeProbe ポーリング ──────────────────────────────────────
    let env = loop {
        let input = yield_step(ch.clone(), vec![]).await;
        let Some(outcome) = probe.check_outcome(total_max_ms) else {
            continue;
        };
        log::debug!(
            "[tsf-probe] cold={cold_seq} ChromeProbe 完了 ({}ms)",
            outcome.elapsed_ms
        );
        // VK_IME_OFF→VK_IME_ON を Chrome path で使わない（タイムアウト後のリセット専用）。
        // VK_IME_OFF が Chrome TSF context を壊し、VK_IME_ON が間に合わず na がリテラル化する。
        // ChromeProbe の F2 (VK_DBE_HIRAGANA) warmup のみで GJI を活性化する。
        break input.env;
    };

    // ── Phase 2b: 直接 Chrome 送信 ──
    // gji_active なら inline LiteralDetect（Phase 3）/ per-VK confirm（Phase 2c）を
    // 安全網として有効化する。VK_A probe + Chrome reinit のフルコース
    // （StartSacrificialWarmup、2026-07-18 撤去）は踏まない。
    let needs_literal = env.gji_active;

    // ── Phase 2c（per-VK confirm）──
    // TSF 側 gji_coro_body の Phase 5b と共通実装（`run_per_vk_confirm`、2026-07-17 統合）。
    if needs_literal && !crate::tsf::observer::literal_session_confirmed() {
        let plan = TransmitPlan {
            should_prepend_f2: false,
            used_eager_path: false,
            needs_literal: true,
            literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
        };
        run_per_vk_confirm(ch.clone(), cold_seq, &romaji, plan, TransmitTarget::Chrome).await;
        return;
    }

    let transmit_input = yield_step(
        ch.clone(),
        vec![ProbeAction::Transmit {
            cold_seq,
            plan: TransmitPlan {
                should_prepend_f2: false,
                used_eager_path: false,
                needs_literal,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            romaji,
            target: TransmitTarget::Chrome,
        }],
    )
    .await;

    // ── Phase 3: Inline LiteralDetect（dispatcher が detector を渡した場合のみ）─
    let Some(TransmitDonePayload {
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
            && expected_kana.is_some_and(|e| env.foreground_comp_char.is_some_and(|c| c != e));

        let final_actions = match detection {
            DetectionResult::CompositionConfirmed if partial_literal => {
                log::warn!(
                    "[raw-tsf-literal] cold={cold_seq} partial literal: comp={:?} ≠ expected='{:?}' → SuspectedLiteral",
                    env.foreground_comp_char, expected_kana
                );
                crate::ime_diagnostic::log_composition_probe(cold_seq, "partial-literal");
                // この経路は TSF→IMM32 bridge による実文字列突合せ (expected_kana との比較)
                // に基づく別系統の partial-literal 検出であり、is_partial_literal() の
                // ヒューリスティック（nc_fired/is_tsf_mode）とは異なる。
                // ESC-based 回収は candidate 表示中の partial literal 専用に限定するため、
                // ここでは既存どおり backs=ze_bs_count のみで対応する（escape_composition=false）。
                vec![
                    ProbeAction::RawTsfLiteralRecovery {
                        cold_seq,
                        backs: ze_bs_count,
                        romaji: recovery_romaji,
                        escape_composition: false,
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
                        escape_composition: false,
                    },
                    ProbeAction::Done,
                ]
            }
            DetectionResult::CompositionConfirmed => {
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
    coro: StepCoro<ProbeTickInput, Vec<ProbeAction>>,
    pending_transmit_done: Option<TransmitDonePayload>,
    pending_vk_sent: Option<VkSentPayload>,
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
        let romaji = romaji.to_string();
        let coro = StepCoro::new(async move |ch| {
            tsf_probe_coro_body(ch, romaji, probe, total_max_ms, cold_seq).await;
        });
        let mut this = Self {
            coro,
            pending_transmit_done: None,
            pending_vk_sent: None,
            cold_seq,
            _guard: guard,
        };
        // Self-priming: StepCoro の最初の step() は input を消費しない。construction 直後・
        // pending_tsf に格納される前にこの「捨てられる1回」を消費しておく
        // （詳細は `GjiWarmupCoro::new` のコメント参照）。
        let primed = this.tick(&TsfEnvSnapshot::default());
        debug_assert!(
            primed.is_empty(),
            "TsfProbeCoro self-priming tick は空の ProbeAction を返すはず: {primed:?}"
        );
        this
    }
}

impl TickableFsm for TsfProbeCoro {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        // BUG-27 調査用ログ: 通常 tick（Phase 1 の 10ms ポーリング等）は毎回
        // vk_sent/transmit_done とも None のため、どちらかが Some の場合のみ出す
        // （毎 tick 出すと Phase 1 のポーリングだけでログが埋まる）。
        if self.pending_vk_sent.is_some() || self.pending_transmit_done.is_some() {
            log::debug!(
                "[tsf-probe-vk-sent-trace] cold={} tick consuming pending_vk_sent={} \
                 pending_transmit_done={} t={}ms",
                self.cold_seq,
                self.pending_vk_sent.is_some(),
                self.pending_transmit_done.is_some(),
                crate::hook::current_tick_ms(),
            );
        }
        let input = ProbeTickInput {
            env: *env,
            transmit_done: self.pending_transmit_done.take(),
            vk_sent: self.pending_vk_sent.take(),
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
                self.pending_transmit_done = Some(TransmitDonePayload {
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

    /// Chrome per-VK confirm が1 VK 送信するたびに呼ぶ。
    /// `GjiWarmupCoro::apply_vk_sent` の Chrome 版。`_guard` がコルーチン生存中ずっと
    /// active を保持しているため、追加のガード確保は不要。
    fn apply_vk_sent(&mut self, detector: LiteralDetector, deadline_ms: u64) {
        // BUG-27 調査用ログ: overwritten=true なら、前回の apply_vk_sent が
        // まだ tick() に消費されないまま次の apply_vk_sent が来ている
        // （＝1 tick 内で TransmitSingleVk が2回ディスパッチされた等の異常）。
        let overwritten = self.pending_vk_sent.is_some();
        log::debug!(
            "[tsf-probe-vk-sent-trace] cold={} apply_vk_sent SET deadline_ms={deadline_ms} \
             overwritten_unconsumed={overwritten} t={}ms",
            self.cold_seq,
            crate::hook::current_tick_ms(),
        );
        self.pending_vk_sent = Some(VkSentPayload {
            detector,
            deadline_ms,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── should_skip_literal_wait 回帰テスト（BUG-29）────────────────────────

    #[test]
    fn should_skip_literal_wait_when_candidate_already_visible() {
        assert!(
            should_skip_literal_wait(true),
            "候補ウィンドウ表示中は literal-detect の polling をスキップする"
        );
    }

    #[test]
    fn should_skip_literal_wait_false_when_candidate_hidden() {
        assert!(
            !should_skip_literal_wait(false),
            "非表示中は従来通り literal-detect の polling を行う"
        );
    }

    // ── decide_transmit_plan 回帰テスト ──────────────────────────────────────

    #[test]
    fn decide_plan_should_prepend_f2_is_always_false() {
        // DIAG_DISABLE_PROACTIVE_TSF_WARMUP を 2026-07-19 に恒久化したことで、romaji
        // バッチへの F2 直接同梱（第3の防御層）は撤去され、should_prepend_f2 は
        // 入力に関わらず常に false になった。この不変条件が将来壊れないことを
        // 固定する回帰テスト（再度有効化する場合はこのコミットの revert が必要）。
        let obs = ProbeObservations { nc_fired: true };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };
        let plan = decide_transmit_plan(false, obs, &env, true, true, true); // Long cold, nc_fired
        assert!(!plan.should_prepend_f2);
    }

    #[test]
    fn decide_plan_nc_not_fired_tsf_not_long_idle_keeps_literal() {
        // nc_fired=false + TSF mode + Short cold (forces_prepend_f2=false):
        // IME 準備完了が未確認のため LiteralDetect は有効にして回収する。
        let obs = ProbeObservations { nc_fired: false };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(false, obs, &env, true, false, false);

        assert!(
            plan.needs_literal,
            "nc_fired=false + tsf: IME 準備未確認 → LiteralDetect で保護"
        );
    }

    #[test]
    fn decide_plan_nc_not_fired_tsf_long_idle_keeps_literal() {
        // Long cold (forces_prepend_f2=true): LiteralDetect が有効（GJI 応答未確認）。
        let obs = ProbeObservations { nc_fired: false };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(false, obs, &env, true, true, true);

        assert!(plan.needs_literal, "tsf mode: LiteralDetect 有効");
    }

    #[test]
    fn decide_plan_nc_fired_enables_literal_when_gji_active() {
        // nc_fired=true でも gji_active + !is_long_cold なら LiteralDetect は有効なまま。
        let obs = ProbeObservations { nc_fired: true };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(false, obs, &env, true, false, false);

        assert!(
            plan.needs_literal,
            "gji_active + !is_long_cold: LiteralDetect 有効"
        );
    }

    #[test]
    fn decide_plan_nc_fired_with_deferred_vks_disables_eager_path() {
        // nc_fired=true でも deferred_vks が存在する場合は unicode に戻さない（nお race 防止）。
        let obs = ProbeObservations { nc_fired: true };
        let env = TsfEnvSnapshot {
            is_tsf_mode: false,
            gji_active: false,
            ..Default::default()
        };

        let plan = decide_transmit_plan(true, obs, &env, false, false, false); // deferred_empty=false

        assert!(
            !plan.used_eager_path,
            "deferred_vks あり: unicode TSF パスを使わない"
        );
    }

    #[test]
    fn decide_plan_medium_cold_keeps_literal_on_ncwait_timeout() {
        // Medium cold (forces_prepend_f2=true) + nc_fired=false → GJI 応答未確認のため
        // LiteralDetect は有効なまま。
        let obs = ProbeObservations { nc_fired: false };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(false, obs, &env, true, true, false); // Medium cold

        assert!(plan.needs_literal, "GJI 応答未確認: LiteralDetect 有効");
    }

    // ── TsfProbeCoro (Chrome) — 犠牲キー撤去後の直接送信回帰テスト ────────────
    //
    // 2026-07-18: is_long_cold 重症度分岐・StartSacrificialWarmup（VK_A probe +
    // Chrome reinit のフルコース）を撤去した。gji_active なら inline LiteralDetect /
    // per-VK confirm を安全網として残し、直接 Chrome へ送信する。

    fn ready_chrome_probe() -> TsfProbeCoro {
        // total_max_ms=0 → check_now が最初の tick で即 ready になる。
        crate::tsf::observer::TSF_OBS
            .gji_monitor_ok
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let guard = OutputActiveGuard::noop_for_test();
        let probe = TsfReadinessProbe::new(0, 0, 0);
        TsfProbeCoro::new_chrome("ka", 0, probe, 0, guard)
    }

    #[test]
    fn chrome_gji_active_enters_per_vk_confirm_as_safety_net() {
        // literal_session_confirmed はプロセスグローバルなので、他テストの実行順序に
        // 依存しないよう明示的にリセットする。
        crate::tsf::observer::reset_literal_session_confirmed();
        let mut machine = ready_chrome_probe();
        let actions = machine.tick(&TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });

        assert!(
            matches!(
                actions.as_slice(),
                [ProbeAction::TransmitSingleVk {
                    target: TransmitTarget::Chrome,
                    ..
                }]
            ),
            "gji_active: per-VK confirm（TransmitSingleVk）を安全網として使う: {actions:?}"
        );
    }

    #[test]
    fn chrome_without_gji_active_skips_literal_detect() {
        // !gji_active（GJI モニター不健全）: needs_literal=false（安全網なしの直接送信）。
        let mut machine = ready_chrome_probe();
        let actions = machine.tick(&TsfEnvSnapshot {
            gji_active: false,
            ..Default::default()
        });

        assert!(
            actions.iter().any(|a| matches!(
                a,
                ProbeAction::Transmit {
                    plan,
                    target: TransmitTarget::Chrome,
                    ..
                } if !plan.needs_literal
            )),
            "!gji_active: LiteralDetect 不要のまま直接送信する: {actions:?}"
        );
    }

    // ── BUG-27 追補2: per-VK confirm の vk_sent 未設定は無リカバリ return に戻す ──

    /// `vk_sent 未設定`（dispatcher が `apply_vk_sent` を呼ばなかった状態）を、あえて
    /// `apply_vk_sent` を呼ばずに次の `tick()` を実行することで再現する。
    ///
    /// 一度は `SuspectedLiteral` と同じ backspace+romaji 再送リカバリに倒したが
    /// （BUG-27 初版）、msedge 実機で `vk_sent 未設定` が毎打鍵で発火し、
    /// `consecutive` が単調増加して二度と 0 に戻らないため常に「give up,
    /// backspace ×1 のみ（再送なし）」に落ち、正しく入力できていた文字まで
    /// 毎回消える regression になった（docs/known-bugs.md BUG-27 追補2）。
    /// 無リカバリの `return`（`ProbeAction::Done` のみを返す）に戻したことを
    /// 固定する回帰テスト。
    #[test]
    fn chrome_per_vk_vk_sent_unset_does_not_backspace() {
        crate::tsf::observer::reset_literal_session_confirmed();
        let mut machine = ready_chrome_probe();
        let first_actions = machine.tick(&TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });
        assert!(
            matches!(
                first_actions.as_slice(),
                [ProbeAction::TransmitSingleVk { .. }]
            ),
            "per-VK confirm ループの最初の VK 送信要求のはず: {first_actions:?}"
        );

        // apply_vk_sent を呼ばずに次の tick を実行 → pending_vk_sent が None のまま渡る。
        let actions_after_unset = machine.tick(&TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });

        assert!(
            !actions_after_unset
                .iter()
                .any(|a| matches!(a, ProbeAction::RawTsfLiteralRecovery { .. })),
            "vk_sent 未設定で backspace を発行してはいけない（msedge で正しく入力できていた \
             文字まで消える regression になった）: {actions_after_unset:?}"
        );
        assert!(
            matches!(actions_after_unset.as_slice(), [ProbeAction::Done]),
            "無リカバリで Done のみを返すはず: {actions_after_unset:?}"
        );
    }
}
