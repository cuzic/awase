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

use crate::state::event_origin::Generation;
use crate::tsf::literal_facts::{
    DetectEvidence, DetectPath, DetectRoute, DetectTarget, LiteralDetectFacts, LiteralVerdict,
};

/// `DeferredVk` の由来。ADR-123 変更B: focus 変更時等に `pending_deferred`
/// を丸ごと破棄する経路（`discard_raw_recovery_if_moved` 等）が、
/// 「awase 自身が再送しようとしていたromaji」（`RecoveryResend`）と
/// 「ユーザーが実際に打鍵したがまだ送信されていない入力」（`UserInput`）を
/// 区別できるようにする。現時点ではログの内訳表示にのみ使う（挙動は変えない、
/// 最小実装(b)）。`RecoveryResend` はADR-123変更A+C（`output/vk_send.rs`の
/// `DeferGate::deferred_origin`、gate免除入口）が唯一の構築箇所 —
/// `tests/architecture_guard.rs` の
/// `deferred_origin_recovery_resend_construction_is_limited_to_gate_bypass`
/// がこれを固定する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeferredOrigin {
    UserInput,
    RecoveryResend,
}

/// probe 進行中に蓄積する後続 VK。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeferredVk {
    pub(crate) vk: VkCode,
    pub(crate) needs_shift: bool,
    pub(crate) order_token: u64,
    pub(crate) origin: DeferredOrigin,
}
use crate::tsf::probe::{LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_coro_state::ProbeCoroState;
use crate::tsf::warmup::tickable_fsm::TickableFsm;
use timed_fsm::coro::{yield_step, Channel, StepCoro};

/// `tick()` 呼び出し時に注入する環境観測値のスナップショット。
/// テストでは任意値を注入できる。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TsfEnvSnapshot {
    pub gji_active: bool,
    /// 前景窓が、per-VK の確認の「2 番目以降の疑い」を信用できないクラス(Zoom のチャット)か(BUG-215)。
    /// 真なら 2 番目以降の `SuspectedLiteral` を確認済みとみなして、残りのキーを送り続ける。
    pub later_vk_literal_check_unreliable: bool,
    /// この tick の時点の宛先の識別（focus 世代 + 前景窓）。段の途中で宛先の窓が変わったかを、
    /// `state::raw_recovery_plan::plan_raw_recovery` で判断するのに使う（ADR-270 の確認の FSM）。`None` = 採っていない。
    pub stage_origin_now: Option<crate::state::raw_recovery_plan::StageOrigin>,
    /// 現在の IME 入力モード belief（Off / Hiragana / Katakana / Unknown）。
    pub ime_mode: crate::tsf::ime_mode_fsm::ImeModeState,
    /// `ime_mode` が `IMC_GETCONVERSIONMODE` で OS から確認済みなら true。
    pub ime_mode_confirmed: bool,
    /// `Output::confirm_gate_deadline_override_ms` の生値。`0` = 上書きなし。
    /// `MsImeReadyCoro` の期限判定に使う（ADR-084、BUG-49 追補2）。
    pub confirm_gate_deadline_override_ms: u64,
    /// `crate::tsf::observer::gji_candidate_visible_now()` の生値。GJI candidate window が
    /// 現時点で可視かどうか（決定分岐から素の `AtomicBool` 読み取りを排除するため、
    /// `env` 経由の明示パラメータとして持つ。`.claude/rules/ime-belief-architecture.md`
    /// 「`ImeModel` 以外の belief 的状態への適用範囲」節参照）。
    pub gji_candidate_visible_now: bool,
    /// `TSF_OBS.literal_session_confirmed_gen` の生値（0 なら `None`）。
    /// `literal_session_confirmed(cold_seq)` は「保存されている世代」と「呼び出し元の
    /// cold_seq」を比較する関数であり、呼び出し元ごとに cold_seq が異なるため、
    /// snapshot にはこの生の世代値だけを埋め込み、比較は FSM 側の純粋なコードで行う。
    pub literal_session_confirmed_gen: Option<Generation>,
}

/// コルーチンが確定した送信方針。dispatcher がそのまま実行する。
#[derive(Debug, Clone, Copy)]
pub(crate) struct TransmitPlan {
    /// raw TSF literal の LiteralDetect フェーズを有効にするか。
    pub needs_literal: bool,
    /// LiteralDetect のタイムアウト期間（ms）。
    pub literal_detect_ms: u64,
}

/// ステートマシン → dispatcher 方向の宣言的アクション。
#[derive(Debug)]
pub(crate) enum ProbeAction {
    /// Chrome バッチパイプライン（Batched VK）で `romaji` を送信する。
    ///
    /// dispatcher は romaji 送信直後に `TsfWarmupCoordinator` の deferred キューを
    /// フラッシュしてから warm マークする。
    Transmit {
        cold_seq: Generation,
        /// FSM が確定した実行方針。dispatcher はそのまま実行する（再導出不要）。
        plan: TransmitPlan,
        romaji: String,
    },
    /// IME セッション最初の1文字専用（BUG-24 追補）: romaji の VK を1つだけ送信する。
    ///
    /// 旧 `is_partial_literal()`（ECRS MB-02 で撤去）が送信前の無関係な代理指標（`nc_fired`）に
    /// 頼っているため、cold 直後の最初の1文字は VK を1個ずつ送って「送ったVK自身」への
    /// `CompositionConfirmed`/`SuspectedLiteral` を確認してから次の VK を送る。
    /// dispatcher は送信直前に `LiteralDetector::new_with_pre_send_baseline()` でベースラインを取り、1 VK だけ
    /// `KeyInjector::send_vk_pair` で送信する。`is_last=true` のときのみ、通常の
    /// `Transmit` と同じ deferred VK フラッシュ・GjiFsm warmup 結果保存を行う。
    TransmitSingleVk {
        cold_seq: Generation,
        vk: VkCode,
        needs_shift: bool,
        /// この VK の confirm 待ちタイムアウト（ms）。`plan.literal_detect_ms` を渡す。
        timeout_ms: u64,
        is_last: bool,
        idx: u16,
        last_idx: u16,
    },
    /// ADR-285: Mozc の cold の最初の 1 キーの確認ができたあとの、残りのローマ字の送信。
    ///
    /// `Transmit` と違い、Mozc の確認を張り直さない（最初の 1 キーで入力中の文字ができたので、残りは確認の対象外）。
    /// 確認が結論を出せずに終わった場合（判断しない）にも、残りは必ず送る。
    TransmitRest { romaji: String },
    /// `RAW_TSF_LITERAL` を設定し、composition を `RawTsfLiteralRecovery` で cold マークする。
    RawTsfLiteralRecovery {
        cold_seq: Generation,
        backs: usize,
        romaji: String,
        /// `true` = partial literal（candidate 表示中に一部だけ literal 化）回収。
        /// バックスペース前に `VK_ESCAPE` を送って composition を確実に破棄する。
        escape_composition: bool,
        facts: LiteralDetectFacts,
    },
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
    ///
    /// `resets_consecutive=true` の場合、`mark_literal_session` とは別に連続失敗の数を 0 に戻す(BUG-227)。
    /// Mozc は最初の 1 キーだけを確かめるので `mark_literal_session` を使わない(語の確認済み印を付けると次の語の
    /// 確認が飛ぶ)が、確認できたなら以前のリテラルとは連続しない。戻さないと、13 分前のリテラルの数え残りで
    /// 次のリテラルが「連続 2 回目」になり、`s` を消さずに `a` だけ送って `sあ` になった。
    CompositionConfirmed {
        cold_seq: Generation,
        mark_literal_session: bool,
        resets_consecutive: bool,
        facts: LiteralDetectFacts,
    },
    /// プローブ完了。dispatcher は `TIMER_TSF_PROBE` を kill する。
    Done,
}

// ── TickInput ─────────────────────────────────────────────────────────────────

/// probe コルーチン (`TsfProbeCoro`) の tick 入力。
///
/// 統合前（2026-07-17 以前）は `TsfProbeTickInput`（Chrome）/ `TickInput`（TSF）として
/// 別々に定義されていたが、フィールド構成が完全に同一だったため共有した。
pub(crate) struct ProbeTickInput {
    pub(crate) env: TsfEnvSnapshot,
    pub(crate) vk_sent: Option<VkSentPayload>,
}

/// `apply_vk_sent` のペイロード。次 tick の [`ProbeTickInput`] に載り、コルーチン本体が
/// そのVK専用の detector をポーリングする（`TsfProbeCoro` の per-VK confirm）。
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
/// かつては Tsf 注入モード用の `GjiWarmupCoro`（TSF Phase 5b）と共有していたが、Tsf 注入経路の
/// 撤去（2026-10-22）で使い手は `tsf_probe_coro_body`（Chrome Phase 2c）だけになった。
///
/// 全 VK 確認できたらセッション確認 (`mark_literal_session=true`) + `Done` を、
/// `SuspectedLiteral` を検出したら `RawTsfLiteralRecovery` を、dispatcher が
/// `apply_vk_sent` を呼ばなかった場合（BUG-27）は無リカバリで中断する。いずれの場合も
/// 必要なアクションは内部で yield 済み（または `CoroStep::Complete` の `Done` フォール
/// バックに委ねる）で、呼び出し元は `.await` 後にそのまま `return` すればよい。
/// [`run_per_vk_confirm`] のログ・診断タグ（docs/known-bugs.md のデバッグキーワード表・
/// 実機ログ grep パターンとの互換性を保つため、従来の Chrome のタグのまま）。
const PER_VK_LOG_TAG: &str = "tsf-probe";
const PER_VK_LITERAL_TAG: &str = "chrome-per-vk-literal";
const PER_VK_CONFIRMED_TAG: &str = "chrome-per-vk-confirmed";

fn idx_u16(idx: usize) -> u16 {
    u16::try_from(idx).unwrap_or(u16::MAX)
}

fn literal_facts(
    detection: &crate::tsf::probe::DetectionResult,
    route: DetectRoute,
    path: DetectPath,
    vk: Option<VkCode>,
    idx: usize,
    last_idx: usize,
    evidence: DetectEvidence,
) -> LiteralDetectFacts {
    LiteralDetectFacts {
        verdict: LiteralVerdict::from(detection),
        route,
        path,
        target: DetectTarget::Chrome,
        vk: vk.map(|vk| vk.0),
        idx: idx_u16(idx),
        last_idx: idx_u16(last_idx),
        evidence,
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
    cold_seq: Generation,
    idx: usize,
    last_idx: usize,
    vk: VkCode,
    env: TsfEnvSnapshot,
) -> (
    crate::tsf::probe::DetectionResult,
    DetectRoute,
    DetectEvidence,
) {
    use crate::tsf::probe::DetectionResult;

    if env.gji_candidate_visible_now {
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
            DetectionResult::CompositionConfirmed => tracing::debug!(
                "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] \
                 candidate window already visible → skip literal-detect wait (vk=0x{:02X})",
                vk.0,
                cold_seq = cold_seq.value(),
            ),
            DetectionResult::StaleConfirm => tracing::warn!(
                "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] candidate window already \
                 visible だが直近の GJI I/O が猶予期間内に送信時刻へ追いつかず \
                 → stale confirm として扱う (vk=0x{:02X})",
                vk.0,
                cold_seq = cold_seq.value(),
            ),
            DetectionResult::SuspectedLiteral => unreachable!(
                "visible_fencing_verdict は CompositionConfirmed/StaleConfirm のみ返す"
            ),
        }
        let evidence =
            sent.detector
                .evidence_now(env.gji_candidate_visible_now, sent.deadline_ms, cold_seq);
        return (verdict, DetectRoute::VisibleFencing, evidence);
    }
    loop {
        let poll_input = yield_step(ch.clone(), vec![]).await;
        if let Some(d) = sent.detector.check_now(sent.deadline_ms) {
            let evidence = sent.detector.evidence_now(
                poll_input.env.gji_candidate_visible_now,
                sent.deadline_ms,
                cold_seq,
            );
            break (d, DetectRoute::CheckNow, evidence);
        }
        let _ = poll_input;
    }
}

/// 2 番目以降の疑いを信用しない窓(Zoom のチャット)で、疑いを無視して「確認済み」として進めるための
/// アクションを返す(BUG-215)。`run_per_vk_confirm` の複雑度を抑えるために切り出した。
fn ignored_later_suspicion(
    log_tag: &str,
    cold_seq: Generation,
    idx: usize,
    last_idx: usize,
    vk: VkCode,
    facts: LiteralDetectFacts,
) -> ProbeAction {
    tracing::debug!(
        "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] 2 番目以降の疑いは信用しない窓 \
         → 確認済みとして続行 (vk=0x{:02X})",
        vk.0,
        cold_seq = cold_seq.value(),
    );
    ProbeAction::CompositionConfirmed {
        cold_seq,
        mark_literal_session: false,
        resets_consecutive: false,
        facts,
    }
}

#[expect(clippy::future_not_send)]
pub(crate) async fn run_per_vk_confirm(
    ch: Rc<Channel<ProbeTickInput, Vec<ProbeAction>>>,
    cold_seq: Generation,
    romaji: &str,
    plan: TransmitPlan,
) {
    use crate::tsf::probe::DetectionResult;

    let (log_tag, literal_tag, confirmed_tag) =
        (PER_VK_LOG_TAG, PER_VK_LITERAL_TAG, PER_VK_CONFIRMED_TAG);

    let vk_chars: Vec<(VkCode, bool)> = romaji.chars().filter_map(crate::vk::ascii_to_vk).collect();
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
            idx: idx_u16(idx),
            last_idx: idx_u16(last_idx),
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
            tracing::warn!(
                "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] vk_sent 未設定 → 中断",
                cold_seq = cold_seq.value(),
            );
            return;
        };

        // `vk_input.env`（このVKの送信が確定した、まさにこの tick のスナップショット）を
        // 渡す。旧実装は `gji_candidate_visible_now()` を `await_vk_detection` 実行時点で
        // 都度ライブ読みしており、VK ごとに変化しうる候補ウィンドウ可視性を反映していた。
        // ループ開始時に一度だけ取った `env`（このコルーチンの呼び出し元から渡された
        // スナップショット）を全 VK で使い回すと、後続 VK の可視性変化を無視してしまい
        // 挙動が変わる。`ProbeTickInput.env` は tick ごとに更新されるため、これが
        // 旧実装のライブ読みに最も近い等価物。
        let later_unreliable = vk_input.env.later_vk_literal_check_unreliable;
        let (detection, route, evidence) = await_vk_detection(
            &ch,
            &sent,
            log_tag,
            cold_seq,
            idx,
            last_idx,
            vk,
            vk_input.env,
        )
        .await;
        let facts = literal_facts(
            &detection,
            route,
            DetectPath::PerVk,
            Some(vk),
            idx,
            last_idx,
            evidence,
        );

        match detection {
            DetectionResult::CompositionConfirmed => {
                tracing::debug!(
                    "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] confirmed (vk=0x{:02X})",
                    vk.0,
                    cold_seq = cold_seq.value(),
                );
                // BUG-27 追補4: この VK 自身の confirm で consecutive_count を
                // リセットする（セッション確認はまだ、全 VK 確認後にまとめて行う）。
                pending_confirm = Some(ProbeAction::CompositionConfirmed {
                    cold_seq,
                    mark_literal_session: false,
                    resets_consecutive: false,
                    facts,
                });
            }
            // BUG-215: Zoom のチャットでは 2 番目以降の疑いを信用しない(直前のキーは確認済み)。ESC + 全体の再送の連鎖を
            // 起こすだけなので、確認済みとして次のキーへ進む。1 番目のキーの疑いは下の回収に任せる。
            DetectionResult::SuspectedLiteral if idx > 0 && later_unreliable => {
                pending_confirm = Some(ignored_later_suspicion(
                    log_tag, cold_seq, idx, last_idx, vk, facts,
                ));
            }
            DetectionResult::SuspectedLiteral => {
                let (backs, escape_composition) =
                    crate::tsf::warmup::literal_detect_fsm::per_vk_recovery_params(false, idx);
                tracing::debug!(
                    "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] suspected literal \
                     (vk=0x{:02X} backs={backs} escape={escape_composition})",
                    vk.0,
                    cold_seq = cold_seq.value(),
                );
                crate::tsf::log_composition_probe(cold_seq, literal_tag);
                // emit_recovery_actions は常に RawTsfLiteralRecovery（backspace のみ、
                // 捨て駒キーには倒れない）を返す。consecutive==0 なら dispatcher が
                // romaji の再送を自然に次の cold パス（per-VK confirm）へ委ねる。
                let actions = crate::tsf::warmup::literal_detect_fsm::emit_recovery_actions(
                    cold_seq,
                    romaji.to_string(),
                    backs,
                    escape_composition,
                    facts,
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
                tracing::warn!(
                    "[{log_tag}] cold={cold_seq} per-VK[{idx}/{last_idx}] stale confirm 検出 \
                     → backspace は送らず romaji 再送のみ行う (vk=0x{:02X} backs={backs} \
                     escape={escape_composition})",
                    vk.0,
                    cold_seq = cold_seq.value(),
                );
                crate::tsf::log_composition_probe(cold_seq, "epoch-fence-stale");
                let actions = crate::tsf::warmup::literal_detect_fsm::emit_recovery_actions(
                    cold_seq,
                    romaji.to_string(),
                    backs,
                    escape_composition,
                    facts,
                );
                yield_step(ch.clone(), actions).await;
                return;
            }
        }
    }

    tracing::debug!(
        "[{log_tag}] cold={cold_seq} per-VK: 全 {} VK 確認済み → セッション確認",
        vk_chars.len(),
        cold_seq = cold_seq.value(),
    );
    crate::tsf::log_composition_probe(cold_seq, confirmed_tag);
    // BUG-27 追補4: 最後の VK の pending_confirm は不要（mark_literal_session=true
    // 版のリセットで包含されるため）、破棄してよい。
    let _ = pending_confirm.take();
    yield_step(
        ch.clone(),
        vec![
            ProbeAction::CompositionConfirmed {
                cold_seq,
                mark_literal_session: true,
                resets_consecutive: false,
                facts: LiteralDetectFacts {
                    verdict: LiteralVerdict::CompositionConfirmed,
                    route: DetectRoute::CheckNow,
                    path: DetectPath::PerVk,
                    target: DetectTarget::Chrome,
                    vk: None,
                    idx: idx_u16(last_idx),
                    last_idx: idx_u16(last_idx),
                    evidence: DetectEvidence::default(),
                },
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
    cold_seq: Generation,
) {
    // ── Phase 1: ChromeProbe ポーリング ──────────────────────────────────────
    let env = {
        let input = yield_step(ch.clone(), vec![]).await;
        let outcome = probe.outcome();
        tracing::debug!(
            "[tsf-probe] cold={cold_seq} ChromeProbe 完了 ({}ms)",
            outcome.elapsed_ms,
            cold_seq = cold_seq.value(),
        );
        // VK_IME_OFF→VK_IME_ON を Chrome path で使わない（タイムアウト後のリセット専用）。
        // VK_IME_OFF が Chrome TSF context を壊し、VK_IME_ON が間に合わず na がリテラル化する。
        // ChromeProbe の F2 (VK_DBE_HIRAGANA) warmup のみで GJI を活性化する。
        input.env
    };

    // ── Phase 2b: 直接 Chrome 送信 ──
    // gji_active なら per-VK confirm（Phase 2c）を
    // 安全網として有効化する。VK_A probe + Chrome reinit のフルコース
    // （StartSacrificialWarmup、2026-07-18 撤去）は踏まない。
    let needs_literal = env.gji_active;

    // ── Phase 2c（per-VK confirm）──
    // `run_per_vk_confirm`（2026-07-17 統合。Tsf 側の利用者は 2026-10-22 に撤去）。
    if needs_literal && env.literal_session_confirmed_gen != Some(cold_seq) {
        let plan = TransmitPlan {
            needs_literal: true,
            literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
        };
        run_per_vk_confirm(ch.clone(), cold_seq, &romaji, plan).await;
        return;
    }

    // ここへ来るのは `needs_literal=false`(= `gji_active` でない)のときだけ。dispatcher が `apply_transmit_done(None)` → true で
    // machine を drop するので、コルーチンは再開されない。`needs_literal=true` は上の per-VK confirm が必ず先に取る
    // (ECRS MB-01、2026-10-21。`cold_seq` は cold start のたびに新規採番され、`literal_session_confirmed_gen` に
    // 生成直後のコルーチンの `cold_seq` が入ることは無い)。
    // 旧 Phase 3(inline LiteralDetect。StaleConfirm→romaji 再送を含む)は撤去した。
    yield_step(
        ch.clone(),
        vec![ProbeAction::Transmit {
            cold_seq,
            plan: TransmitPlan {
                needs_literal,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
            },
            romaji,
        }],
    )
    .await;
}

// ── TsfProbeCoro ──────────────────────────────────────────────────────────────

/// TSF/Chrome cold-start プローブ コルーチン。`TsfProbeMachine` の後継。
///
/// `pending_tsf` (`RefCell<Option<Box<dyn TickableFsm>>>`) に格納し、
/// `TIMER_TSF_PROBE` ハンドラが `tick()` で 1 ステップ進める。
pub(crate) struct TsfProbeCoro {
    /// `tick`/`apply_vk_sent`/prime のラッパー配線（`probe_coro_state.rs` 参照）。
    state: ProbeCoroState,
    /// RAII guard。drop で `OUTPUT_GATE.active=false`。コルーチン生存中ずっと
    /// active を保持する（construction 時点から確保する）。
    _guard: OutputActiveGuard,
}

impl TsfProbeCoro {
    /// Chrome F2 cold warmup (`send_romaji_batched` の cold パス) 用コンストラクタ。
    pub(crate) fn new_chrome(
        romaji: &str,
        cold_seq: Generation,
        probe: TsfReadinessProbe,
        guard: OutputActiveGuard,
    ) -> Self {
        let romaji = romaji.to_string();
        let coro = StepCoro::new(async move |ch| {
            tsf_probe_coro_body(ch, romaji, probe, cold_seq).await;
        });
        Self {
            state: ProbeCoroState::new(coro, cold_seq, "TsfProbeCoro"),
            _guard: guard,
        }
    }
}

impl TickableFsm for TsfProbeCoro {
    fn tick(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.state.tick(env, "tsf-probe-vk-sent-trace")
    }

    fn cold_seq_hint(&self) -> Generation {
        self.state.cold_seq()
    }

    /// Chrome per-VK confirm が1 VK 送信するたびに呼ぶ。`_guard` がコルーチン生存中ずっと
    /// active を保持しているため、追加のガード確保は不要。
    fn apply_vk_sent(&mut self, detector: LiteralDetector, deadline_ms: u64) {
        self.state
            .apply_vk_sent(detector, deadline_ms, "tsf-probe-vk-sent-trace");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── TsfProbeCoro (Chrome) — 犠牲キー撤去後の直接送信回帰テスト ────────────
    //
    // 2026-07-18: is_long_cold 重症度分岐・StartSacrificialWarmup（VK_A probe +
    // Chrome reinit のフルコース）を撤去した。gji_active なら inline LiteralDetect /
    // per-VK confirm を安全網として残し、直接 Chrome へ送信する。

    /// `TSF_OBS.gji_monitor_ok` を書き換えるため、呼び出し元は必ず
    /// `TSF_OBS_TEST_LOCK` を保持していること（BUG-65 追補4参照。この関数自体は
    /// ロックを取らない — 呼び出し元ごとに他の TSF_OBS フィールドの初期化と
    /// まとめてロックを取りたいケースがあるため）。
    fn ready_chrome_probe() -> TsfProbeCoro {
        // 最初の tick で probe.outcome() を読んで進む。
        crate::tsf::observer::TSF_OBS
            .gji_monitor_ok
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let guard = OutputActiveGuard::noop_for_test();
        let probe = TsfReadinessProbe::new(0, Generation::INITIAL);
        TsfProbeCoro::new_chrome("ka", Generation::INITIAL, probe, guard)
    }

    // ── TickableFsm 経由の capability 到達(旧 ChromeProbe ラッパーの BUG-27 回帰を引き継ぐ) ──
    //
    // 本番の `pending_tsf: Box<dyn TickableFsm>` と同じ経路で `apply_vk_sent` を
    // 呼んでも、デフォルト no-op に落ちず実装に届くこと。

    #[test]
    fn chrome_apply_vk_sent_via_trait_reaches_coro() {
        use crate::tsf::warmup::tickable_fsm::TickableFsm;
        let _g = crate::tsf::observer::TSF_OBS_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::tsf::observer::reset_literal_session_confirmed();
        let mut machine = ready_chrome_probe();
        let env = || TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        };
        let first = TickableFsm::tick(&mut machine, env());
        assert!(
            matches!(first.as_slice(), [ProbeAction::TransmitSingleVk { .. }]),
            "{first:?}"
        );
        let deadline_ms = crate::clock::current_tick_ms() + 1000;
        TickableFsm::apply_vk_sent(
            &mut machine,
            LiteralDetector::new_with_pre_send_baseline(crate::tsf::observer::gji_write_bytes()),
            deadline_ms,
        );
        // 届いていれば detection 待ちの polling(空の Vec)。no-op に落ちると即 Done。
        let after = TickableFsm::tick(&mut machine, env());
        assert!(after.is_empty(), "{after:?}");
    }

    #[test]
    fn chrome_gji_active_enters_per_vk_confirm_as_safety_net() {
        let _g = crate::tsf::observer::TSF_OBS_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // literal_session_confirmed はプロセスグローバルなので、他テストの実行順序に
        // 依存しないよう明示的にリセットする。
        crate::tsf::observer::reset_literal_session_confirmed();
        let mut machine = ready_chrome_probe();
        let actions = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });

        assert!(
            matches!(actions.as_slice(), [ProbeAction::TransmitSingleVk { .. }]),
            "gji_active: per-VK confirm（TransmitSingleVk）を安全網として使う: {actions:?}"
        );
    }

    #[test]
    fn chrome_without_gji_active_skips_literal_detect() {
        let _g = crate::tsf::observer::TSF_OBS_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // !gji_active（GJI モニター不健全）: needs_literal=false（安全網なしの直接送信）。
        let mut machine = ready_chrome_probe();
        let actions = machine.tick(TsfEnvSnapshot {
            gji_active: false,
            ..Default::default()
        });

        assert!(
            actions.iter().any(|a| matches!(
                a,
                ProbeAction::Transmit { plan, .. } if !plan.needs_literal
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
        let _g = crate::tsf::observer::TSF_OBS_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _g = TSF_OBS_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _clock = crate::clock::freeze();

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

        let detector =
            LiteralDetector::new_with_pre_send_baseline(crate::tsf::observer::gji_write_bytes());
        let deadline_ms = crate::clock::current_tick_ms() + 10_000;

        // 前世代の残存 GJI I/O（今回の VK 送信より前）だけがある状態を模擬する。
        // detector構築時刻(epoch_send_ms)より確実に前のwrite根拠を作るため、実時間
        // sleepではなくsaturating_subで明示的な過去時刻を使う。GetTickCount64の
        // 粗い解像度(既定~15.6ms)下ではsleep(5ms)後にtickが進んでいる保証がなく、
        // 同一tickに丸まるとevidence_is_freshのtie判定(`>=`、probe.rs:656/735)が
        // trueになり、猶予に入らず即座にactionが出て「猶予期間中は空のはず」の
        // アサートがflakyに失敗する(2026-07-25、Windows実機で初めてこのテストを
        // 実行した際に顕在化)。
        let stale_write_ms = crate::clock::current_tick_ms().saturating_sub(50);
        TSF_OBS.gji_last_write_ms.store(stale_write_ms, SeqCst);

        // 候補ウィンドウが「既に可視」= 前世代の合成が残っている状態を模擬する。
        TSF_OBS.gji_candidate_visible.store(true, SeqCst);

        machine.apply_vk_sent(detector, deadline_ms);
        // `gji_candidate_visible_now` は `await_vk_detection` が `env` 経由で読む
        // ようになった（グローバル直読み廃止）ため、上で `TSF_OBS.gji_candidate_visible`
        // に立てた「既に可視」を、この tick に渡す `env` にも明示的に反映する。
        let actions_immediately_after_send = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            gji_candidate_visible_now: true,
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
        // 実時間の sleep は BUG-130 と同型で GetTickCount64 の量子化(~15.6ms)と CI の負荷でフレークした。仮想時計を進める(ADR-272 段階 B0)。
        crate::clock::advance(LiteralDetector::EPOCH_FENCE_GRACE_MS + 50);
        let actions_after_stale = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            gji_candidate_visible_now: true,
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

        let _g = TSF_OBS_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let epoch_ms = crate::clock::current_tick_ms();
        TSF_OBS
            .gji_last_write_ms
            .store(epoch_ms.saturating_sub(50), SeqCst); // stale（epoch より前）

        let detector =
            LiteralDetector::new_with_pre_send_baseline(crate::tsf::observer::gji_write_bytes());
        let deadline_ms = crate::clock::current_tick_ms() + 10_000;
        TSF_OBS.gji_candidate_visible.store(true, SeqCst);

        machine.apply_vk_sent(detector, deadline_ms);
        // `gji_candidate_visible_now` は `await_vk_detection` が `env` 経由で読む
        // ようになった（グローバル直読み廃止）ため、上で `TSF_OBS.gji_candidate_visible`
        // に立てた「既に可視」を、この tick に渡す `env` にも明示的に反映する。
        let actions_immediately_after_send = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            gji_candidate_visible_now: true,
            ..Default::default()
        });
        assert!(
            actions_immediately_after_send.is_empty(),
            "猶予中はまだ確定しないはず: {actions_immediately_after_send:?}"
        );

        // 猶予期間内に、このVK自身の送信によるGJI I/Oが観測されたことにする。
        TSF_OBS
            .gji_last_write_ms
            .store(crate::clock::current_tick_ms(), SeqCst);
        let actions_after_catchup = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            gji_candidate_visible_now: true,
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

        let _g = TSF_OBS_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let detector0 = LiteralDetector::new_with_pre_send_baseline(baseline);
        let deadline0 = crate::clock::current_tick_ms() + 10_000;
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
            .store(crate::clock::current_tick_ms(), SeqCst);
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
        let detector1 =
            LiteralDetector::new_with_pre_send_baseline(crate::tsf::observer::gji_write_bytes());
        let deadline1 = crate::clock::current_tick_ms(); // 既に期限切れ
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

    /// BUG-215 の回帰テスト: Zoom のチャット（`later_vk_literal_check_unreliable`）では、2 番目以降の VK が
    /// `SuspectedLiteral` になっても回収（ESC + 全体の再送）に入らず、確認済みとして最後まで進む。
    /// 実機では 1 打鍵で `ki` が 6 回再送された。
    #[test]
    fn chrome_per_vk_later_vk_suspicion_is_ignored_in_unreliable_window() {
        use crate::tsf::observer::{TSF_OBS, TSF_OBS_TEST_LOCK};
        use std::sync::atomic::Ordering::SeqCst;

        let _g = TSF_OBS_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let detector0 = LiteralDetector::new_with_pre_send_baseline(baseline);
        let deadline0 = crate::clock::current_tick_ms() + 10_000;
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
            .store(crate::clock::current_tick_ms(), SeqCst);
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
        let detector1 =
            LiteralDetector::new_with_pre_send_baseline(crate::tsf::observer::gji_write_bytes());
        let deadline1 = crate::clock::current_tick_ms(); // 既に期限切れ
        machine.apply_vk_sent(detector1, deadline1);
        let after_send1 = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            later_vk_literal_check_unreliable: true,
            ..Default::default()
        });
        assert!(
            after_send1.is_empty(),
            "confirm 待ちの1 tick 目は空のはず: {after_send1:?}"
        );

        let actions_vk1 = machine.tick(TsfEnvSnapshot {
            gji_active: true,
            later_vk_literal_check_unreliable: true,
            ..Default::default()
        });
        assert!(
            !actions_vk1
                .iter()
                .any(|a| matches!(a, ProbeAction::RawTsfLiteralRecovery { .. })),
            "信用できない窓の 2 番目の VK の疑いは回収しない: {actions_vk1:?}"
        );
        assert!(
            actions_vk1
                .iter()
                .any(|a| matches!(a, ProbeAction::CompositionConfirmed { .. })),
            "確認済みとして最後まで進み、セッション確認で終わる: {actions_vk1:?}"
        );

        TSF_OBS.gji_candidate_visible.store(false, SeqCst);
        TSF_OBS.gji_last_write_ms.store(0, SeqCst);
    }
}
