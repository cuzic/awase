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
    /// 現在の IME 入力モード belief（Off / Hiragana / Katakana / Unknown）。
    pub ime_mode: crate::tsf::ime_mode_fsm::ImeModeState,
    /// `ime_mode` が `IMC_GETCONVERSIONMODE` で OS から確認済みなら true。
    pub ime_mode_confirmed: bool,
    /// `TsfWarmupCoordinator` の deferred キューに現在何か積まれているか（覗き見、消費しない）。
    /// `GjiWarmupCoro` の `decide_transmit_plan` eager path 判定に使う。
    pub deferred_pending: bool,
}

/// probe 中に観測した事実。`decide_transmit_plan` の入力に使う。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ProbeObservations {
    /// TSF の NameChange イベントが実際に発火したか（生の観測値、呼び出し元での上書き禁止）。
    pub nc_fired: bool,
    /// GJI probe が実際に新規 I/O を確認できたか（`GjiProbeOutcome.settled`）。
    /// `confirm_key_tsf_hint` の救済を効かせてよいかどうかの実測ゲートに使う（BUG-40）。
    pub gji_settled: bool,
    /// `cold_reason` が confirm-key 系（ReinjectConfirmKey/PassthroughConfirmKey）かつ
    /// TSF mode か。WezTerm 等で Enter/Space 後に NameChange が発火しないが GJI は
    /// 正常に合成中というケースの救済ヒント（`3ffbe66` 参照）。`gji_settled` が
    /// true（GJI が実際に I/O を返している）の場合に限り `nc_fired=false` を
    /// 救済してよい。`gji_settled=false`（例: 88s idle 後に probe がわずか16msで
    /// 完了した genuinely cold なセッション）まで無条件に救済すると、reactive
    /// LiteralDetect が丸ごとスキップされ、漏れた romaji が無補正で出力される
    /// （BUG-40、`docs/known-bugs.md` 参照）。
    pub confirm_key_tsf_hint: bool,
}

/// `decide_transmit_plan` が確定した実行方針。dispatcher がそのまま実行する。
#[derive(Debug, Clone, Copy)]
pub(crate) struct TransmitPlan {
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
    initial_used_eager: bool,
    obs: ProbeObservations,
    env: TsfEnvSnapshot,
    deferred_empty: bool,
    forces_prepend_f2: bool,
    is_long_cold: bool,
) -> TransmitPlan {
    // romaji バッチへの F2 直接同梱（第3の防御層）は DIAG_DISABLE_PROACTIVE_TSF_WARMUP を
    // 2026-07-19 に恒久化したことで撤去した。reactive な LiteralDetect のみに委ねる
    // （`docs/known-bugs.md` BUG-24 参照）。

    // nc_fired=false でも、confirm_key_tsf_hint（WezTerm 等で NameChange が発火しない
    // 既知ケース）かつ gji_settled（GJI が実際に I/O を返しており genuinely warm）の
    // 場合に限り、NameChange 発火とみなして救済する（`3ffbe66`）。gji_settled=false の
    // ままこの救済を効かせると、genuinely cold なセッションまで誤救済されて reactive
    // LiteralDetect が丸ごとスキップされる（BUG-40）。nc_fired 自体は書き換えない
    // （呼び出し元・`is_partial_literal` が生値として参照するため）。
    let nc_confirmed = obs.nc_fired || (obs.confirm_key_tsf_hint && obs.gji_settled);

    // nc_confirmed=true: IME モード確認済み（Medium/Long cold かつ deferred なし → VK path）。
    // nc_confirmed=false + TSF mode: VK path 固定（unicode は GJI composition をバイパスし "nお" race が起きる）。
    let used_eager_path = if nc_confirmed {
        (initial_used_eager || forces_prepend_f2) && deferred_empty
    } else if env.is_tsf_mode {
        false
    } else {
        initial_used_eager || forces_prepend_f2
    };

    // NameChangeWait タイムアウトで IME 準備が未確認のまま transmit したケースを
    // LiteralDetect で回収する。F2 をバッチに含めない場合も IME が cold の可能性がある。
    // （既知バグ: gji_idle ~1.4s で 300ms NameChangeWait が間に合わなかったケース）
    let needs_literal = !nc_confirmed && env.is_tsf_mode && env.gji_active;

    let literal_detect_ms = if is_long_cold && env.is_tsf_mode {
        crate::tuning::RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE
    } else {
        crate::tuning::RAW_TSF_LITERAL_DETECT_MS
    };

    TransmitPlan {
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
    /// dispatcher は送信直前に `LiteralDetector::new_with_pre_send_baseline()` でベースラインを取り、1 VK だけ
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
    /// `mark_literal_session=true` の場合、`cold_seq` 世代の literal-detect 自体を
    /// スキップ対象としてマークする（`tsf::observer::mark_literal_session_confirmed`、
    /// BUG-39 で世代付きに変更）。per-VK confirm では各 VK の confirm で
    /// `mark_literal_session=false`、全 VK 確認済みの最終確認でのみ `true` を使う。
    CompositionConfirmed {
        cold_seq: u32,
        mark_literal_session: bool,
    },
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
/// Chrome (`TsfProbeCoro`) / TSF (`GjiWarmupCoro`) で共有。
pub(crate) struct TransmitDonePayload {
    pub(crate) romaji: String,
    pub(crate) ze_bs_count: usize,
    pub(crate) detector: LiteralDetector,
    /// `apply_transmit_done` 呼び出し時点の `current_tick_ms() + literal_detect_ms`。
    pub(crate) deadline_ms: u64,
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
/// 差分はログ/診断タグの接頭辞のみで、`target` から導出できるためここに統合できた
/// （BUG-29 の候補ウィンドウ可視時 polling スキップは元々 Chrome 限定だったが、
/// BUG-30 で検出ロジック自体が統一されたため両ターゲット共通になった）。
///
/// 全 VK 確認できたらセッション確認 (`mark_literal_session=true`) + `Done` を、
/// `SuspectedLiteral` を検出したら `RawTsfLiteralRecovery` を、dispatcher が
/// `apply_vk_sent` を呼ばなかった場合（BUG-27）は無リカバリで中断する。いずれの場合も
/// 必要なアクションは内部で yield 済み（または `CoroStep::Complete` の `Done` フォール
/// バックに委ねる）で、呼び出し元は `.await` 後にそのまま `return` すればよい。
/// [`run_per_vk_confirm`] のログ・診断タグ（Chrome/TSF で従来通り書き分ける。
/// docs/known-bugs.md のデバッグキーワード表・実機ログ grep パターンとの互換性を保つため）。
fn per_vk_confirm_tags(target: TransmitTarget) -> (&'static str, &'static str, &'static str) {
    match target {
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
    }
}

/// 1 VK 分の [`DetectionResult`] を確定させる（[`run_per_vk_confirm`] から抽出）。
///
/// BUG-29/BUG-30: 候補ウィンドウが既に表示中なら literal-detect の polling をスキップする。
/// SHOW はエッジトリガ（hidden→visible 遷移でのみ増分）のため VK1 以降では再発火せず、
/// 子音単体 VK は WriteTransferCount 閾値も原理的に越えない（probe.rs の
/// `LiteralDetector::check_now` に既知の限界として記載済み）。検出ロジック自体が
/// TSF/Chrome で統一された（BUG-30、`docs/known-bugs.md` BUG-30 追補1）ため、
/// この早期脱出も両ターゲットに適用する（旧: Chrome 限定）。
#[expect(clippy::future_not_send)]
async fn await_vk_detection(
    ch: &Rc<Channel<ProbeTickInput, Vec<ProbeAction>>>,
    sent: &VkSentPayload,
    log_tag: &str,
    cold_seq: u32,
    idx: usize,
    last_idx: usize,
    vk: VkCode,
) -> crate::tsf::probe::DetectionResult {
    use crate::tsf::probe::DetectionResult;

    if crate::tsf::observer::gji_candidate_visible_now() {
        // ADR-079 epoch fencing: 「既に可視」だけを根拠に無条件で confirmed とすると、
        // 前の（見捨てた）世代が開いたまま残っている候補ウィンドウを現世代の証拠として
        // 誤って採用してしまう（実機トレースで確認済み、本 VK1 以降のショートカットが
        // まさにその発火箇所だった）。直近の GJI I/O が本当にこの VK の送信時刻より
        // 後かを `LiteralDetector::visible_fencing_verdict` で確認する。
        //
        // 猶予なしの一発判定は禁止: `gji_last_write_ms` は GJI I/O monitor の
        // ポーリングサンプル（`GJI_SAMPLE_INTERVAL_MS` 周期）でしか更新されない
        // ため、この VK 自身の合成が実際には成功していても、直後の tick では
        // まだ反映されていないことが多い。これを猶予なしで stale と断定すると、
        // 候補ウィンドウが開きっぱなしになる通常の高速タイピングで毎回のように
        // false positive の StaleConfirm が発生し、正しく合成できていた文字が
        // backspace で失われる regression になる（実機で確認済み、2026-07-23）。
        // `check_now` の SHOW-only 分岐と同じ `EPOCH_FENCE_GRACE_MS` の猶予を
        // 共有するため、`Some` が返るまで tick ごとに再確認する。
        let verdict = loop {
            if let Some(verdict) = sent.detector.visible_fencing_verdict(sent.deadline_ms) {
                break verdict;
            }
            let poll_input = yield_step(ch.clone(), vec![]).await;
            let _ = poll_input;
        };
        match verdict {
            DetectionResult::CompositionConfirmed => log::debug!(
                "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] \
                 candidate window already visible → skip literal-detect wait (vk=0x{:02X})",
                vk.0,
            ),
            DetectionResult::StaleConfirm => log::warn!(
                "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] candidate window already \
                 visible だが直近の GJI I/O が猶予期間内に送信時刻へ追いつかず \
                 → stale confirm として扱う (vk=0x{:02X})",
                vk.0,
            ),
            DetectionResult::SuspectedLiteral => unreachable!(
                "visible_fencing_verdict は CompositionConfirmed/StaleConfirm のみ返す"
            ),
        }
        return verdict;
    }
    loop {
        let poll_input = yield_step(ch.clone(), vec![]).await;
        if let Some(d) = sent.detector.check_now(sent.deadline_ms) {
            break d;
        }
        let _ = poll_input;
    }
}

#[expect(clippy::future_not_send)]
pub(crate) async fn run_per_vk_confirm(
    ch: Rc<Channel<ProbeTickInput, Vec<ProbeAction>>>,
    cold_seq: u32,
    romaji: &str,
    plan: TransmitPlan,
    target: TransmitTarget,
) {
    use crate::tsf::probe::DetectionResult;

    let (log_tag, literal_tag, confirmed_tag) = per_vk_confirm_tags(target);

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

        let detection = await_vk_detection(&ch, &sent, log_tag, cold_seq, idx, last_idx, vk).await;

        match detection {
            DetectionResult::CompositionConfirmed => {
                log::debug!(
                    "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] confirmed (vk=0x{:02X})",
                    vk.0,
                );
                // BUG-27 追補4: この VK 自身の confirm で consecutive_count を
                // リセットする（セッション確認はまだ、全 VK 確認後にまとめて行う）。
                pending_confirm = Some(ProbeAction::CompositionConfirmed {
                    cold_seq,
                    mark_literal_session: false,
                });
            }
            DetectionResult::SuspectedLiteral => {
                let (backs, escape_composition) =
                    crate::tsf::warmup::literal_detect_fsm::per_vk_recovery_params(false, idx);
                log::debug!(
                    "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] suspected literal \
                     (vk=0x{:02X} backs={backs} escape={escape_composition})",
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
            DetectionResult::StaleConfirm => {
                // ADR-079 Stage 1 追補（実機で発見した回帰の修正）: 「候補ウィンドウ
                // 既に可視」ショートカットは per-VK confirm の**1文字目**でも発火し
                // うる（前世代の後始末ではなく、フォーカス変更前からの残留 GJI UI
                // 状態等が原因）。この場合に「検出のみ・recovery なし」で単に Done
                // にすると、まだ送信していない後続 VK が一切送られないまま処理が
                // 終了し、既に送信済みの VK（このVKの生文字）だけが取り残されて
                // 消失・文字化けする実害が生じた（実機報告 2026-07-22:
                // 「これでできる」→「kれでできる」）。romaji の再送自体は必要
                // なので Done では終わらせない。
                //
                // BUG-33 追補4（2026-07-23、実機で2件確認）: 一方で `StaleConfirm`
                // は「confirm 根拠が古い」ことの検出であって「literal である」
                // 証拠ではないため、backspace は送らない（`per_vk_recovery_params`
                // のドキュメント参照）。以前は SuspectedLiteral と全く同じ
                // backspace+再送に倒しており、これが「リーク」の「ク」・
                // 「cold 」の末尾スペースといった別スコープの確定済み文字を
                // 誤って消す実害・および stale confirm が連鎖して consecutive
                // カウントが積み上がり複数文字が連続して消える実害を引き起こして
                // いた。
                let (backs, escape_composition) =
                    crate::tsf::warmup::literal_detect_fsm::per_vk_recovery_params(true, idx);
                log::warn!(
                    "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] stale confirm 検出 \
                     → backspace は送らず romaji 再送のみ行う (vk=0x{:02X} backs={backs} \
                     escape={escape_composition})",
                    vk.0,
                );
                crate::ime_diagnostic::log_composition_probe(cold_seq, "epoch-fence-stale");
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
                cold_seq,
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
    if needs_literal && !crate::tsf::observer::literal_session_confirmed(cold_seq) {
        let plan = TransmitPlan {
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
    }) = transmit_input.transmit_done
    else {
        return;
    };

    loop {
        use crate::tsf::probe::DetectionResult;
        yield_step(ch.clone(), vec![]).await;

        let Some(detection) = detector.check_now(deadline_ms) else {
            continue;
        };

        let final_actions = match detection {
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
            DetectionResult::StaleConfirm => {
                // ADR-079 Stage 1 追補: 「検出のみ・recovery なし」は実機で
                // 未送信 VK が欠落する回帰を引き起こした（run_per_vk_confirm 側の
                // 同種修正のコメント参照）。romaji 再送自体は必要なので Done では
                // 終わらせない。
                //
                // BUG-33 追補4: 一方 backspace は「literal である」証拠がない
                // 限り送らない（`per_vk_recovery_params` のドキュメント参照）。
                // StaleConfirm は confirm 根拠が古いことの検出であって literal の
                // 証拠ではないため backs=0 とする。
                log::warn!(
                    "[raw-tsf-literal] cold={cold_seq} stale confirm 検出 → \
                     backspace は送らず romaji 再送のみ行う"
                );
                crate::ime_diagnostic::log_composition_probe(cold_seq, "epoch-fence-stale");
                vec![
                    ProbeAction::RawTsfLiteralRecovery {
                        cold_seq,
                        backs: 0,
                        romaji: recovery_romaji,
                        escape_composition: false,
                    },
                    ProbeAction::Done,
                ]
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
        let mut coro = StepCoro::new(async move |ch| {
            tsf_probe_coro_body(ch, romaji, probe, total_max_ms, cold_seq).await;
        });
        // pending_tsf に格納して外部から本物の tick を受け取り始める前に prime() で
        // 消費しておく（詳細は `GjiWarmupCoro::new` のコメント参照）。
        let primed = coro.prime();
        debug_assert!(
            matches!(&primed, CoroStep::Yielded(actions) if actions.is_empty()),
            "TsfProbeCoro prime() は空の ProbeAction を yield するはず: {primed:?}"
        );
        Self {
            coro,
            pending_transmit_done: None,
            pending_vk_sent: None,
            cold_seq,
            _guard: guard,
        }
    }
}

impl TickableFsm for TsfProbeCoro {
    fn tick(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
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
            env,
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
    ) -> bool {
        match detector {
            Some(det) => {
                let deadline_ms = crate::hook::current_tick_ms() + literal_detect_ms;
                self.pending_transmit_done = Some(TransmitDonePayload {
                    romaji,
                    ze_bs_count,
                    detector: det,
                    deadline_ms,
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

    // ── decide_transmit_plan 回帰テスト ──────────────────────────────────────

    #[test]
    fn decide_plan_nc_not_fired_tsf_not_long_idle_keeps_literal() {
        // nc_fired=false + TSF mode + Short cold (forces_prepend_f2=false):
        // IME 準備完了が未確認のため LiteralDetect は有効にして回収する。
        let obs = ProbeObservations {
            nc_fired: false,
            ..Default::default()
        };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(false, obs, env, true, false, false);

        assert!(
            plan.needs_literal,
            "nc_fired=false + tsf: IME 準備未確認 → LiteralDetect で保護"
        );
    }

    #[test]
    fn decide_plan_nc_not_fired_tsf_long_idle_keeps_literal() {
        // Long cold (forces_prepend_f2=true): LiteralDetect が有効（GJI 応答未確認）。
        let obs = ProbeObservations {
            nc_fired: false,
            ..Default::default()
        };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(false, obs, env, true, true, true);

        assert!(plan.needs_literal, "tsf mode: LiteralDetect 有効");
    }

    #[test]
    fn decide_plan_nc_fired_suppresses_literal_even_when_gji_active() {
        // BUG-40（docs/known-bugs.md）で `nc_confirmed = nc_fired || (confirm_key_tsf_hint
        // && gji_settled)` に整理されて以降、nc_fired=true（NameChange が実発火 = 合成が
        // 実際に確認された）は常に needs_literal=false になる。この関数は
        // `426a7f2`（2026-07-19）で追加された旧テスト（旧実装では
        // `should_prepend_f2` 由来の第1節があり、nc_fired=true でも
        // gji_active+!is_long_cold なら literal 有効、という別の分岐が生きていた）の
        // 期待値が、その後の削除・BUG-40整理で古くなったまま残っていたもの。
        // BUG-40の副次的発見として記録済み（本テスト自体は未修正のまま放置されて
        // いた）で、2026-07-25にWindows実機CIで実際にassertion failureとして
        // 顕在化したのを機に、現在の意図された挙動に合わせて更新した。
        let obs = ProbeObservations {
            nc_fired: true,
            ..Default::default()
        };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(false, obs, env, true, false, false);

        assert!(
            !plan.needs_literal,
            "nc_fired=true(NameChange確認済み)ならgji_active/is_long_cold問わずLiteralDetect不要のはず(BUG-40)"
        );
    }

    #[test]
    fn decide_plan_nc_fired_with_deferred_vks_disables_eager_path() {
        // nc_fired=true でも deferred_vks が存在する場合は unicode に戻さない（nお race 防止）。
        let obs = ProbeObservations {
            nc_fired: true,
            ..Default::default()
        };
        let env = TsfEnvSnapshot {
            is_tsf_mode: false,
            gji_active: false,
            ..Default::default()
        };

        let plan = decide_transmit_plan(true, obs, env, false, false, false); // deferred_empty=false

        assert!(
            !plan.used_eager_path,
            "deferred_vks あり: unicode TSF パスを使わない"
        );
    }

    #[test]
    fn decide_plan_medium_cold_keeps_literal_on_ncwait_timeout() {
        // Medium cold (forces_prepend_f2=true) + nc_fired=false → GJI 応答未確認のため
        // LiteralDetect は有効なまま。
        let obs = ProbeObservations {
            nc_fired: false,
            ..Default::default()
        };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(false, obs, env, true, true, false); // Medium cold

        assert!(plan.needs_literal, "GJI 応答未確認: LiteralDetect 有効");
    }

    // ── BUG-40 回帰テスト: confirm_key_tsf_hint は gji_settled 必須 ────────────
    //
    // `nc_for_plan = nc_fired || (cold_reason.is_confirm_key() && env.is_tsf_mode)`
    // という旧実装（gji_settled を一切見ない）は、WezTerm の Enter/Space 後に
    // NameChange が発火しないが GJI は実際に合成中というケース（`3ffbe66`）を
    // 救済するために書かれた。しかし gji_settled を見ないため、88s idle 後に
    // probe がわずか16msで完了した genuinely cold なセッション（GJI は未確定、
    // gji_settled=false）まで同じ条件で誤救済し、reactive LiteralDetect が
    // 丸ごとスキップされて漏れた romaji ("ke" 等) が無補正で出力された。

    #[test]
    fn decide_plan_confirm_key_hint_without_settled_keeps_literal() {
        // confirm_key_tsf_hint=true だが gji_settled=false（genuinely cold, BUG-40
        // の実トレース相当）: 救済せず LiteralDetect を有効なままにする。
        let obs = ProbeObservations {
            nc_fired: false,
            gji_settled: false,
            confirm_key_tsf_hint: true,
        };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(false, obs, env, true, false, false);

        assert!(
            plan.needs_literal,
            "BUG-40: gji_settled=false のまま confirm_key_tsf_hint だけで救済してはならない"
        );
    }

    #[test]
    fn decide_plan_confirm_key_hint_with_settled_suppresses_literal() {
        // confirm_key_tsf_hint=true かつ gji_settled=true（WezTerm が実際に I/O を
        // 返している、3ffbe66 の対象シナリオ）: NameChange 欠落を救済し、誤検出の
        // backspace で確定済み文字を消さないよう LiteralDetect を抑制する。
        let obs = ProbeObservations {
            nc_fired: false,
            gji_settled: true,
            confirm_key_tsf_hint: true,
        };
        let env = TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(false, obs, env, true, false, false);

        assert!(
            !plan.needs_literal,
            "3ffbe66: gji_settled=true なら NameChange 欠落を救済し LiteralDetect を抑制する"
        );
    }

    #[test]
    fn decide_plan_confirm_key_hint_without_tsf_mode_has_no_effect() {
        // confirm_key_tsf_hint は is_tsf_mode 前提の救済ヒントであり、それ単体
        // （gji_settled=true でも）non-TSF では needs_literal に影響しない
        // （non-TSF は元々 needs_literal の第3項 `env.is_tsf_mode` で false 固定）。
        let obs = ProbeObservations {
            nc_fired: false,
            gji_settled: true,
            confirm_key_tsf_hint: true,
        };
        let env = TsfEnvSnapshot {
            is_tsf_mode: false,
            gji_active: true,
            ..Default::default()
        };

        let plan = decide_transmit_plan(false, obs, env, true, false, false);

        assert!(!plan.needs_literal, "non-TSF: needs_literal は常に false");
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
        let actions = machine.tick(TsfEnvSnapshot {
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
        let actions = machine.tick(TsfEnvSnapshot {
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
        let first_actions = machine.tick(TsfEnvSnapshot {
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
        let actions_after_unset = machine.tick(TsfEnvSnapshot {
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

    // ── ADR-079: epoch fencing（「既に可視」ショートカット）の回帰テスト ──────

    /// 候補ウィンドウが「既に可視」でも、直近の GJI I/O が今回の VK 送信より前
    /// （前世代の残存合成、あるいはフォーカス変更前からの残留 GJI UI 状態）にしか
    /// 裏付けられていなければ `StaleConfirm` として扱われ、romaji 再送は
    /// スケジュールするが backspace は送らないことを確認する。
    ///
    /// 当初は「検出のみ・recovery なし（ただの Done）」だったが、これは per-VK
    /// confirm の**1文字目**でこの経路が発火した場合（前世代の後始末ではなく、
    /// フォーカス変更直後の残留 UI 状態等が原因）に、まだ送信していない後続 VK
    /// が一切送られないまま処理が終了し、既に送信済みの VK の生文字だけが
    /// 取り残される実害を実機で引き起こした（2026-07-22 実機報告:
    /// 「これでできる」→「kれでできる」、docs/known-bugs.md BUG-33 参照）。
    /// そのため romaji 再送自体は必要（Done で終わらせない）。
    ///
    /// BUG-33 追補4: 一方 backspace は「literal である」証拠がない限り送らない。
    /// `StaleConfirm` は confirm 根拠が古いことの検出であって literal の証拠では
    /// ないため、以前の「SuspectedLiteral と同じ backspace+再送」は誤りだった
    /// （実機で「リーク」の「ク」・「cold 」の末尾スペースを誤って消す実害を
    /// 確認済み）。
    #[test]
    fn chrome_per_vk_stale_confirm_from_leftover_candidate_window_recovers_like_suspected_literal()
    {
        use crate::tsf::observer::{TSF_OBS, TSF_OBS_TEST_LOCK};
        use std::sync::atomic::Ordering::SeqCst;

        // TSF_OBS(プロセス全体のグローバル状態)への並行アクセスをobserver.rs/
        // probe.rs/literal_detect_fsm.rsと共通のロックで直列化する。このファイルの
        // テストは元々ロックを一切持たず、他ファイルのテストと無防備にTSF_OBSを
        // 奪い合っていた(2026-07-25、Windows実機での初回cargo test実行で
        // クロスファイルの汚染が判明)。
        let _g = TSF_OBS_TEST_LOCK.lock().unwrap();

        crate::tsf::observer::reset_literal_session_confirmed();
        TSF_OBS.gji_candidate_visible.store(false, SeqCst);
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);

        let mut machine = ready_chrome_probe();
        let first_actions = machine.tick(TsfEnvSnapshot {
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

        let detector = LiteralDetector::new_with_pre_send_baseline(
            crate::tsf::observer::gji_write_bytes(),
            false,
        );
        let deadline_ms = crate::hook::current_tick_ms() + 10_000;

        // 前世代の残存 GJI I/O（今回の VK 送信より前）だけがある状態を模擬する。
        // detector構築時刻(epoch_send_ms)より確実に前のwrite根拠を作るため、実時間
        // sleepではなくsaturating_subで明示的な過去時刻を使う。GetTickCount64の
        // 粗い解像度(既定~15.6ms)下ではsleep(5ms)後にtickが進んでいる保証がなく、
        // 同一tickに丸まるとevidence_is_freshのtie判定(`>=`、probe.rs:656/735)が
        // trueになり、猶予に入らず即座にactionが出て「猶予期間中は空のはず」の
        // アサートがflakyに失敗する(2026-07-25、Windows実機で初めてこのテストを
        // 実行した際に顕在化)。
        let stale_write_ms = crate::hook::current_tick_ms().saturating_sub(50);
        TSF_OBS.gji_last_write_ms.store(stale_write_ms, SeqCst);

        // 候補ウィンドウが「既に可視」= 前世代の合成が残っている状態を模擬する。
        TSF_OBS.gji_candidate_visible.store(true, SeqCst);

        machine.apply_vk_sent(detector, deadline_ms);
        let actions_immediately_after_send = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });
        assert!(
            actions_immediately_after_send.is_empty(),
            "ADR-079 猶予期間中は即断せず、まだ何も action を出さないはず \
             （猶予なしの即断は false positive の stale confirm を量産する \
             regression になった）: {actions_immediately_after_send:?}"
        );

        // 猶予期間（EPOCH_FENCE_GRACE_MS）が過ぎても gji_last_write_ms は追いつかない
        // （前世代の残存 GJI I/O のまま）ため、次 tick で StaleConfirm に確定する。
        std::thread::sleep(std::time::Duration::from_millis(
            LiteralDetector::EPOCH_FENCE_GRACE_MS + 10,
        ));
        let actions_after_stale = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });

        assert!(
            actions_after_stale
                .iter()
                .any(|a| matches!(a, ProbeAction::RawTsfLiteralRecovery { backs: 0, .. })),
            "stale confirm 検出時は romaji 再送はスケジュールする（未送信 VK を \
             残したまま無回収で終わると生文字が残存するため）が、backspace は \
             送らない（backs=0）はず: {actions_after_stale:?}"
        );

        TSF_OBS.gji_candidate_visible.store(false, SeqCst);
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);
    }

    /// ADR-079 の猶予期間（`EPOCH_FENCE_GRACE_MS`）内に `gji_last_write_ms` が
    /// 追いつけば、「候補ウィンドウ既に可視」ショートカットは stale confirm と
    /// 誤判定せず `CompositionConfirmed` として確定し、backspace 回収を一切
    /// 発行しないことを確認する。
    ///
    /// これが実機で欠けていた回帰そのもの: 高速タイピング中は候補ウィンドウが
    /// 開きっぱなしのままこのショートカットに毎回入るため、猶予なしの一発判定だと
    /// GJI I/O monitor のポーリングサンプルが追いつく前に stale と誤判定され、
    /// 正しく合成できていた文字まで backspace で失われていた
    /// （2026-07-23 実機ログ、romaji "de" が3世代連続で stale 誤判定され
    /// 再送なしで消失）。
    #[test]
    fn chrome_per_vk_visible_shortcut_confirms_when_write_catches_up_within_grace() {
        use crate::tsf::observer::{TSF_OBS, TSF_OBS_TEST_LOCK};
        use std::sync::atomic::Ordering::SeqCst;

        let _g = TSF_OBS_TEST_LOCK.lock().unwrap();
        crate::tsf::observer::reset_literal_session_confirmed();
        TSF_OBS.gji_candidate_visible.store(false, SeqCst);
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);

        let mut machine = ready_chrome_probe();
        machine.tick(TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });

        // 送信直前時点では GJI I/O がまだ観測されていない（同一世代内の
        // ポーリングラグを模擬）。
        let epoch_ms = crate::hook::current_tick_ms();
        TSF_OBS
            .gji_last_write_ms
            .store(epoch_ms.saturating_sub(50), SeqCst); // stale（epoch より前）

        let detector = LiteralDetector::new_with_pre_send_baseline(
            crate::tsf::observer::gji_write_bytes(),
            false,
        );
        let deadline_ms = crate::hook::current_tick_ms() + 10_000;
        TSF_OBS.gji_candidate_visible.store(true, SeqCst);

        machine.apply_vk_sent(detector, deadline_ms);
        let actions_immediately_after_send = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });
        assert!(
            actions_immediately_after_send.is_empty(),
            "猶予中はまだ確定しないはず: {actions_immediately_after_send:?}"
        );

        // 猶予期間内に、このVK自身の送信によるGJI I/Oが観測されたことにする。
        TSF_OBS
            .gji_last_write_ms
            .store(crate::hook::current_tick_ms(), SeqCst);
        let actions_after_catchup = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });

        assert!(
            !actions_after_catchup
                .iter()
                .any(|a| matches!(a, ProbeAction::RawTsfLiteralRecovery { .. })),
            "猶予内に write が追いつけば CompositionConfirmed のはずで、\
             backspace 回収を発行してはいけない: {actions_after_catchup:?}"
        );

        TSF_OBS.gji_candidate_visible.store(false, SeqCst);
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);
    }

    /// BUG-33 追補4 の直接回帰テスト（実機報告「cold が」→「coldが」、末尾スペース
    /// 消失）: 2文字目以降の VK が「本物の」`SuspectedLiteral`（deadline 到達、
    /// 「既に可視」ショートカットではない）になっても、直前の VK が fresh に
    /// confirmed 済みである以上 backspace は送らず ESC のみで回収する。
    #[test]
    fn chrome_per_vk_suspected_literal_after_confirmed_prior_vk_escapes_without_backspace() {
        use crate::tsf::observer::{TSF_OBS, TSF_OBS_TEST_LOCK};
        use std::sync::atomic::Ordering::SeqCst;

        let _g = TSF_OBS_TEST_LOCK.lock().unwrap();
        crate::tsf::observer::reset_literal_session_confirmed();
        TSF_OBS.gji_candidate_visible.store(false, SeqCst);
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);

        let mut machine = ready_chrome_probe(); // romaji "ka" (2 VK)
        let first_actions = machine.tick(TsfEnvSnapshot {
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

        // VK0 ('k') を write-bytes 閾値超過（genuinely fresh）で confirm する。
        let baseline = crate::tsf::observer::gji_write_bytes();
        let detector0 = LiteralDetector::new_with_pre_send_baseline(baseline, false);
        let deadline0 = crate::hook::current_tick_ms() + 10_000;
        machine.apply_vk_sent(detector0, deadline0);
        let after_send0 = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });
        assert!(
            after_send0.is_empty(),
            "confirm 待ちの1 tick 目は空のはず: {after_send0:?}"
        );

        TSF_OBS.gji_write_bytes.store(baseline + 400, SeqCst); // 閾値超過、fresh write
        TSF_OBS
            .gji_last_write_ms
            .store(crate::hook::current_tick_ms(), SeqCst);
        let actions_vk0_confirmed = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });
        assert!(
            actions_vk0_confirmed
                .iter()
                .any(|a| matches!(a, ProbeAction::TransmitSingleVk { .. })),
            "VK0 confirm 後は VK1 の送信要求が来るはず: {actions_vk0_confirmed:?}"
        );

        // VK1 ('a') は候補ウィンドウ非可視のまま、deadline を既に過ぎた状態にして
        // 本物の SuspectedLiteral（「既に可視」ショートカットではない）を起こす。
        let detector1 = LiteralDetector::new_with_pre_send_baseline(
            crate::tsf::observer::gji_write_bytes(),
            false,
        );
        let deadline1 = crate::hook::current_tick_ms(); // 既に期限切れ
        machine.apply_vk_sent(detector1, deadline1);
        let after_send1 = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });
        assert!(
            after_send1.is_empty(),
            "confirm 待ちの1 tick 目は空のはず: {after_send1:?}"
        );

        let actions_vk1 = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });
        assert!(
            actions_vk1.iter().any(|a| matches!(
                a,
                ProbeAction::RawTsfLiteralRecovery {
                    backs: 0,
                    escape_composition: true,
                    ..
                }
            )),
            "直前の VK が confirmed 済みの SuspectedLiteral は ESC のみ・backspace \
             なしのはず（実機で確定済みの直前文字を誤って消す実害が確認された）: \
             {actions_vk1:?}"
        );

        TSF_OBS.gji_candidate_visible.store(false, SeqCst);
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);
    }
}
