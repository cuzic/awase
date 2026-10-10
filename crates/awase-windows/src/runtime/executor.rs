#![allow(unsafe_code)] // Win32 API 呼び出しに unsafe が必須(lib.rsのクレート全体allowから個別移管、Task #9)
/// Decision の副作用を実行する。
///
/// # 2モード: Filter / Relay
///
/// - **Filter**: PassThrough キーは OS にそのまま通す。入出力系 Effects は
///   フック内で即座実行（キー順序保証のため）。重い Effects は遅延。
///
/// - **Relay**: 全キーを Consume し、PassThrough キーも ReinjectKey として
///   キューに入れる。全 Effects がメッセージループで FIFO 実行される。
///   フック内で OS API を一切呼ばない。
use std::collections::VecDeque;

use awase::engine::{
    Decision, Effect, ImeEffect, InputEffect, InputModeState, TimerEffect, UiEffect,
};
use awase::platform::{PlatformRuntime, TsfComposition};
use awase::types::RawKeyEvent;

use crate::hook::CallbackResult;
use crate::platform::WindowsPlatform;
use crate::runtime::{PassthroughQueue, PhysicalKeyDisposition};
use crate::state::platform_state::ImeStateHub;
use crate::state::relay_plan;
use crate::state::ConvModeAuthority;
use crate::vk::VkCodeExt;
use crate::RawKeyEventExt as _;

/// IME apply の sync 完了 1 件分。
///
/// `generation` は Engine `SetOpen` 要求時に払い出した generation。完了時に
/// current pending と照合し、古い async/sync 完了が新しい IME 状態を壊すのを防ぐ。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ImeApplyCompletion {
    pub open: bool,
    pub outcome: awase::platform::ImeOpenOutcome,
    pub generation: Option<crate::state::ApplyGeneration>,
    /// ADR-086 §4 INV-18 の provenance。sync path（`execute_one`）は常に
    /// `Decision::SetOpen` エフェクト駆動のため `EngineDecision` 固定。
    pub reason: crate::state::ime_event::OpenApplyReason,
}

pub(crate) type ImeApplyPair = ImeApplyCompletion;

/// `execute_from_hook` の戻り値。
#[derive(Debug)]
pub(crate) struct BatchResult {
    /// OS に返す consume/passthrough 判定
    pub callback: CallbackResult,
    /// true なら `PostMessage(WM_EXECUTE_EFFECTS)` でメッセージループに通知が必要
    pub has_pending: bool,
    /// sync path の SetOpen 完了リスト。
    /// async path は `WM_ASYNC_IME_APPLY_COMPLETE` 経由で `on_ime_apply_complete` に合流するため
    /// ここには含まない（`post_async_ime_apply_complete` を参照）。
    pub sync_outcomes: Vec<ImeApplyPair>,
}

pub(crate) struct DecisionExecutor {
    /// Effects キュー（FIFO 順序保証）
    queue: VecDeque<Effect>,
    /// passthrough キーの Down/Up 対称性と output guard defer を管理する。
    passthrough_queue: PassthroughQueue,
    /// OUTPUT_GUARD で park した ReinjectKey イベント。
    ///
    /// 不変条件: `guard_held.is_some()` ⟺ `TIMER_OUTPUT_GUARD` が登録済み。
    /// drain は「slot を先に試す → 通過したら queue に進む」の 2 段構え。
    /// queue 本体は常に純粋 FIFO で `push_back` / `pop_front` のみ。
    /// `RawKeyEvent` 型にすることで「ReinjectKey 以外が park される」コンパイルエラーになる。
    guard_held: Option<RawKeyEvent>,
    /// 直近の apply 済み IME 状態の確信度スナップショット。
    ///
    /// decision サイクル開始時に `ImeModel.applied_state()` から pre-fetch され、
    /// バッチ内の `SetOpen` 処理後に即時更新される（intra-batch ordering 用）。
    /// `ImeModel` が SSOT; これはバッチ内 communication channel 兼 cross-decision cache。
    applied_snapshot: crate::state::AppliedImeState,
    /// 直近の入力方式 belief（`execute_from_loop` で `ime.input_mode()` から pre-fetch）。
    /// `ImeControlView.belief_input_mode` に転記して apply 戦略に渡す。
    belief_input_mode: InputModeState,
}

impl std::fmt::Debug for DecisionExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecisionExecutor").finish_non_exhaustive()
    }
}

impl DecisionExecutor {
    pub(crate) fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            passthrough_queue: PassthroughQueue::new(),
            guard_held: None,
            applied_snapshot: crate::state::AppliedImeState::Unknown,
            belief_input_mode: InputModeState::Unknown,
        }
    }

    /// フックコールバックから呼ぶ。
    ///
    /// Relay モード（唯一のモード）: 全 Effects をキューに入れ、PassThrough キーも
    /// ReinjectKey に変換。常に Consumed を返す。
    /// （旧 Filter モードは 2026-07-06 撤去 — relay-defer/INPUT_DEFER 対称性/
    /// NonText パススルー等がすべて Relay 前提で設計・実機検証されており、
    /// Filter は長期間テストされていないレガシー経路だったため。）
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn execute_from_hook(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &mut ImeStateHub,
        decision: Decision,
        raw_event: &RawKeyEvent,
        physical: PhysicalKeyDisposition,
    ) -> BatchResult {
        self.applied_snapshot = ime.model().applied;
        self.execute_relay(platform, ime, decision, raw_event, physical)
    }

    /// メッセージループから呼ぶ。全 Effects を即座に実行する。
    ///
    /// `EngineCommand::FocusChanged` / `RefreshState` 等、キーボードフックを経由しない
    /// 全ての `Decision` 実行経路（フォーカス変更通知・IME リフレッシュポーリング・
    /// ホットキー・タイマー由来の deferred key 再処理等）がここに合流する。
    /// この関数はキーボード経路（`execute_from_hook`）と違い `kp_stage_focus_probe` による
    /// barrier 消費を経ないため、ここで `is_focus_transition_settling` を素直に評価してよい。
    ///
    /// ADR-213 P2d-2: かつてここと `kp_run_inner` にあった settle 中の `SetOpen` 除去
    /// （2026-07-05 の Alt+Tab 中間窓対策）は撤去した。`SetOpen` を出すのは明示操作だけで、
    /// settle 直後の書き込みも Chrome×GJI・Chrome×MS-IME に受け付けられると実測したため
    /// （`docs/experiments.md` エントリ 30）。
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn execute_from_loop(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &mut ImeStateHub,
        decision: Decision,
    ) -> (CallbackResult, Vec<ImeApplyPair>) {
        self.applied_snapshot = ime.model().applied;
        self.belief_input_mode = ime.input_mode();
        let (consumed, effects) = match decision {
            Decision::PassThrough => return (CallbackResult::PassThrough, Vec::new()),
            Decision::PassThroughWith { effects } => (false, effects),
            Decision::Consume { effects } => (true, effects),
        };

        let mut sync_outcomes = Vec::new();
        for effect in effects {
            let generation = ime.model().pending_generation();
            if let Some(o) = self.execute_one(platform, ime, effect, generation) {
                sync_outcomes.push(o);
            }
        }

        let callback = if consumed {
            CallbackResult::Consumed
        } else {
            CallbackResult::PassThrough
        };
        (callback, sync_outcomes)
    }

    /// `WM_EXECUTE_EFFECTS` ハンドラ、および `TIMER_OUTPUT_GUARD` タイマーから呼ぶ。
    ///
    /// `guard_held` に park 済みの Effect があれば最初にそれを試し、
    /// output guard 期間中なら `TIMER_OUTPUT_GUARD` を設定して即座に返る（block_on しない）。
    /// タイマー発火後に再び呼ばれ、guard 解除済みなら reinject を実行する。
    pub(crate) fn drain_deferred(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &mut ImeStateHub,
    ) -> Vec<ImeApplyPair> {
        // 同一 drain 呼び出し内で最初の ReinjectKey だけ OUTPUT_GUARD を適用する。
        // 連続する reinject (例: Win_DOWN→X_DOWN→X_UP→Win_UP) を個別にガードすると
        // Win が 150ms 以上 OS 側でスタックし、後続のショートカットが Win+key と
        // 誤解釈されるため、先頭の reinject が guard を通過したら残りはまとめて送出する。
        let mut sync_outcomes = Vec::new();
        let mut reinject_guard_passed = false;

        // 1) 前回 park した ReinjectKey があれば最初に試す。
        //    guard 解除済みなら execute_one してから queue に進む (batching を継続)。
        //    判断（park するか・guard 通過の持ち越し）は `relay_plan::plan_drain_step`。
        if let Some(event) = self.guard_held.take() {
            let wait = if relay_plan::drain_needs_wait_check(relay_plan::DrainItem::Held, false) {
                self.reinject_wait_remaining(platform, &event)
            } else {
                None
            };
            let step = relay_plan::plan_drain_step(relay_plan::DrainItem::Held, false, wait);
            match step.action {
                relay_plan::DrainAction::Park { remaining } => {
                    tracing::debug!(
                        "[reinject-guard] held event, suspending for {remaining}ms (vk={:#04x}) reason={:?}",
                        event.vk_code,
                        step.reason,
                    );
                    self.park_in_guard(platform, event, remaining);
                    return sync_outcomes;
                }
                relay_plan::DrainAction::Execute { guard_passed_after } => {
                    let effect = Effect::Input(InputEffect::ReinjectKey(event));
                    let generation = ime.model().pending_generation();
                    if let Some(o) = self.execute_one(platform, ime, effect, generation) {
                        sync_outcomes.push(o);
                    }
                    reinject_guard_passed = guard_passed_after;
                }
            }
        }

        // 2) queue を FIFO で drain。
        while let Some(effect) = self.queue.pop_front() {
            let item = if matches!(effect, Effect::Input(InputEffect::ReinjectKey(_))) {
                relay_plan::DrainItem::QueuedReinject
            } else {
                relay_plan::DrainItem::QueuedOther
            };
            // guard の判断が要るときだけ OS を読む（元の読む回数を変えない）。
            let wait = match (
                &effect,
                relay_plan::drain_needs_wait_check(item, reinject_guard_passed),
            ) {
                (Effect::Input(InputEffect::ReinjectKey(event)), true) => {
                    self.reinject_wait_remaining(platform, event)
                }
                _ => None,
            };
            let step = relay_plan::plan_drain_step(item, reinject_guard_passed, wait);
            match step.action {
                relay_plan::DrainAction::Park { remaining } => {
                    let Effect::Input(InputEffect::ReinjectKey(event)) = effect else {
                        unreachable!("Park は QueuedReinject のときだけ返る")
                    };
                    tracing::debug!(
                        "[reinject-guard] suspending drain for {remaining}ms (vk={:#04x}) reason={:?}",
                        event.vk_code,
                        step.reason,
                    );
                    self.park_in_guard(platform, event, remaining);
                    return sync_outcomes;
                }
                relay_plan::DrainAction::Execute { guard_passed_after } => {
                    reinject_guard_passed = guard_passed_after;
                }
            }
            let generation = ime.model().pending_generation();
            if let Some(o) = self.execute_one(platform, ime, effect, generation) {
                sync_outcomes.push(o);
            }
        }

        // 全 Effect を消化: lingering な timer を kill (no-op if not registered)。
        if self.guard_held.is_none() {
            platform.timer.kill(crate::TIMER_OUTPUT_GUARD);
        }

        sync_outcomes
    }

    /// `TIMER_OUTPUT_GUARD` 発火時に呼ぶ。timer を kill して drain を再試行する。
    pub(crate) fn on_output_guard_timer(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &mut ImeStateHub,
    ) -> Vec<ImeApplyPair> {
        platform.timer.kill(crate::TIMER_OUTPUT_GUARD);
        self.drain_deferred(platform, ime)
    }

    /// queue または guard slot に Effect が残っているか
    pub(crate) fn has_pending(&self) -> bool {
        !self.queue.is_empty() || self.guard_held.is_some()
    }

    /// ReinjectKey をまだ流してはいけない場合、再試行までの待ち時間を返す。
    ///
    /// Enter/Space/Escape は IME composition を確定するため、直前の flush 出力が
    /// TSF/GJI probe に残っている間は通さない。
    #[expect(clippy::unused_self)]
    fn reinject_wait_remaining(
        &self,
        platform: &WindowsPlatform,
        event: &RawKeyEvent,
    ) -> Option<u64> {
        // `has_pending_tsf_work` / `output_in_flight_ms` は元と同じく必要なときだけ読む。
        let confirm_held_by_tsf = matches!(event.event_type, awase::types::KeyEventType::KeyDown)
            && event.vk_code.is_composition_confirm_key()
            && platform.has_pending_tsf_work();
        let output_elapsed_ms = if confirm_held_by_tsf {
            0
        } else {
            platform.output_in_flight_ms()
        };
        relay_plan::reinject_wait_remaining(relay_plan::ReinjectWaitFacts {
            confirm_held_by_tsf,
            output_elapsed_ms,
            guard_ms: crate::tuning::OUTPUT_GUARD_MS,
        })
    }

    /// ReinjectKey イベントを guard slot に park し、TIMER_OUTPUT_GUARD を再設定する。
    /// 再設定は idempotent (remaining は last_send からの相対時刻基準で計算される)。
    fn park_in_guard(
        &mut self,
        platform: &mut WindowsPlatform,
        event: RawKeyEvent,
        remaining: u64,
    ) {
        self.guard_held = Some(event);
        platform.timer.set(
            crate::TIMER_OUTPUT_GUARD,
            std::time::Duration::from_millis(remaining),
        );
    }

    /// drain 経路 (`WM_DRAIN_OUTPUT_QUEUE`) 専用: PassThrough を OS に届けるための
    /// `ReinjectKey` を末尾にキューイングする。
    ///
    /// **訂正（2026-09-05、BUG-116 調査で発覚）**: 以前このコメントは「通常
    /// hook 経路では PassThrough は `CallNextHookEx` で OS に直接届く」と
    /// 書いていたが、これは誤り。フック (`hook.rs::hook_proc`) は
    /// `produce_result` が通常時（`ProduceResult::Accepted`）は常に
    /// `LRESULT(1)` を返して元イベントを消費する（`hook.rs:1195-1199`）ため、
    /// **通常 hook 経路でも PassThrough は必ず `enqueue_reinject`（
    /// `runtime/message_handlers.rs:244-250`）経由で SendInput により
    /// 再送出される**。`CallNextHookEx` で OS に直接届く経路は存在しない。
    /// この誤った mental model が、`docs/adr/137-...md`（BUG-116）で
    /// 「`PhysicalKeyDisposition::plan` が Allow を返せば OS に届く」という
    /// 前提の一因になっていた（実際には reinject が `wScan: 0` を使うため、
    /// scan 依存のモードキー処理に影響しうる）。
    ///
    /// drain 経路 (`WM_DRAIN_OUTPUT_QUEUE`) がこの関数を明示的に呼ぶ理由は
    /// 元々の記述どおり: OUTPUT_GATE active 期間や with_app 再入セーフネットで
    /// `INPUT_DEFER` へ Consumed として退避されたキーは drain で engine に
    /// replay されたあと `CallbackResult::PassThrough` が返っても hook 経路
    /// （そもそも通常 hook 経路も `enqueue_reinject` を通る）には戻らないため、
    /// ここから明示的に呼び出す必要がある。
    pub(crate) fn enqueue_reinject(&mut self, event: RawKeyEvent) {
        self.queue
            .push_back(Effect::Input(InputEffect::ReinjectKey(event)));
    }

    // ── Relay モード（スマートリレー）──
    //
    // PassThrough（Effects なし）: 直接 OS に通す（修飾キー、スペース等）
    // PassThroughWith（flush あり）: Consume → flush 出力 + キー再注入を FIFO
    // Consume: Effects をキューに入れる
    //
    // NICOLA 変換と無関係なキーは OS に直接通すことで、
    // Win キー等のシステム動作を壊さず、INJECTED フラグ問題も回避する。
    // flush を伴う PassThrough のみ Consume して順序を保証する。

    fn execute_relay(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &mut ImeStateHub,
        decision: Decision,
        raw_event: &RawKeyEvent,
        physical: PhysicalKeyDisposition,
    ) -> BatchResult {
        let kind = match &decision {
            Decision::PassThrough => relay_plan::RelayDecisionKind::PassThrough,
            Decision::PassThroughWith { .. } => relay_plan::RelayDecisionKind::PassThroughWith,
            Decision::Consume { .. } => relay_plan::RelayDecisionKind::Consume,
        };
        let plan = relay_plan::plan_relay(relay_plan::RelayFacts { kind, physical });
        // フックのコールバック上なので panic しうる `unreachable!` は使わず、Decision の種別ごとに
        // plan を読む（種別と plan の対応は `plan_relay_exhaustive` が固定）。
        match decision {
            Decision::PassThrough => {
                // physical=Suppress（KANJI 物理キー抑止）の場合は OS に届けず Consume する。
                // handle_passthrough の reinject/warmup 後処理も走らせない。
                if matches!(plan, relay_plan::RelayPlan::PassThroughPhysicalSuppressed) {
                    return BatchResult {
                        has_pending: self.has_pending(),
                        callback: CallbackResult::Consumed,
                        sync_outcomes: Vec::new(),
                    };
                }
                let callback = self.run_passthrough_pipeline(platform, raw_event);
                BatchResult {
                    has_pending: self.has_pending(),
                    callback,
                    sync_outcomes: Vec::new(),
                }
            }
            Decision::PassThroughWith { mut effects } => {
                let reinject = matches!(plan, relay_plan::RelayPlan::FlushWithReinject);
                // flush 出力あり → Consume して flush + キー再注入を FIFO でキュー。
                // physical=Suppress（KANJI 物理キー抑止）の場合は reinject を積まない。
                tracing::debug!(
                    "[relay-flush] PassThroughWith: queue {} effect(s){} (vk={:#04x} {}) reason={:?}",
                    effects.len(),
                    if reinject {
                        " + reinject"
                    } else {
                        " (no reinject, suppressed)"
                    },
                    raw_event.vk_code,
                    match raw_event.event_type {
                        awase::types::KeyEventType::KeyDown => "down",
                        awase::types::KeyEventType::KeyUp => "up",
                    },
                    plan,
                );
                if reinject {
                    effects.push(Effect::Input(InputEffect::ReinjectKey(*raw_event)));
                }
                self.queue.extend(effects);
                BatchResult {
                    callback: CallbackResult::Consumed,
                    has_pending: true,
                    sync_outcomes: Vec::new(),
                }
            }
            Decision::Consume { effects } => {
                // Engine が消費 → Timer は即時実行（platform timer state を常に最新に保つ）、
                // それ以外はキューに入れる（e12、判断は `relay_plan::plan_consume_effect`）。
                //
                // Timer を即時実行しない場合、drain 中に Kill/Set がキューに積まれたまま
                // platform の current_os_id が更新されず、deferred_engine_timers の
                // os_id 照合が stale なタイマーを有効と誤判定して早期発火する
                // （例: PendingChar(S)→PendingChar(D) 遷移後に古い S のタイマーが発火）。
                let mut sync_outcomes = Vec::new();
                for effect in effects {
                    match relay_plan::plan_consume_effect(matches!(effect, Effect::Timer(_))) {
                        relay_plan::EffectRoute::Immediate => {
                            let generation = ime.model().pending_generation();
                            if let Some(o) = self.execute_one(platform, ime, effect, generation) {
                                sync_outcomes.push(o);
                            }
                        }
                        relay_plan::EffectRoute::Queue => self.queue.push_back(effect),
                    }
                }
                BatchResult {
                    callback: CallbackResult::Consumed,
                    has_pending: self.has_pending(),
                    sync_outcomes,
                }
            }
        }
    }

    // ── PassThrough サブハンドラ ──

    /// PassThrough パイプラインの統合エントリポイント。
    ///
    /// 段階:
    ///   A. [transport] KeyUp 対称性 — deferred Down に対応する Up も reinject に揃える
    ///   B. [transport] output guard defer — 出力 in-flight 中は reinject 経由で順序保証
    ///   → PassThrough（確認キー KeyDown の cold 化・warmup は reinject 段 `handle_reinject` だけが担う）
    fn run_passthrough_pipeline(
        &mut self,
        platform: &WindowsPlatform,
        raw_event: &RawKeyEvent,
    ) -> CallbackResult {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);

        // A. [transport] KeyUp 対称性
        if let Some(event) = self.passthrough_queue.check_keyup_symmetry(raw_event) {
            self.enqueue_reinject(event);
            return CallbackResult::Consumed;
        }

        // B. [transport] output guard defer
        let in_flight_ms = platform.output_in_flight_ms();
        let output_in_flight =
            relay_plan::output_in_flight(in_flight_ms, crate::tuning::OUTPUT_GUARD_MS);
        // BUG-58: `self.has_pending()` は executor 自身の effect queue しか見ない。
        // `MsImeReadyCoro` の Phase 1（NATIVE 確認待ち、無出力）は
        // `OutputActiveGuard` を持たなくなった（`ms_ime_ready_coro.rs` 参照）ため、
        // `has_pending_tsf_work()` を OR することで、この待機中の PassThrough キーも
        // `check_output_guard_defer` により ReinjectKey 化されるようにする。
        //
        // ただし実効的に保護されるのは Enter/Space/Escape の KeyDown（composition
        // 確定キー）のみである点に注意: `drain_deferred` 側の
        // `reinject_wait_remaining`（本ファイル下部）が `is_composition_confirm_key()`
        // の場合に限り `has_pending_tsf_work()` が下りるまで park する。この1行が
        // ORで加えたことで、その既存保護が Phase 1 待機中に初めて実効化する
        // （従来は `OutputActiveGuard` がフック分配自体を止めていたため、この
        // reinject 経路にすら到達しなかった）。矢印キー・Tab・Ctrl+C 等それ以外の
        // PassThrough は `OUTPUT_GUARD_MS` 窓を過ぎていれば即 reinject されるため、
        // Phase 1 待機（実測 ~180ms）中にまだ送信されていない romaji を追い越しうる
        // ケースは残存する既知の限界（BUG-58 のフリーズ解消と比べて実害は小さいと
        // 判断、将来 PassThrough 全般を defer する場合は改めて検討）。
        let has_pending = self.has_pending() || platform.has_pending_tsf_work();
        tracing::debug!(
            "[relay-guard] vk={:#04x} {} in_flight_ms={} has_pending={} output_in_flight={}",
            raw_event.vk_code,
            if is_key_down { "down" } else { "up" },
            if in_flight_ms == u64::MAX {
                "never".to_string()
            } else {
                in_flight_ms.to_string()
            },
            has_pending,
            output_in_flight,
        );
        if let Some(event) = self.passthrough_queue.check_output_guard_defer(
            raw_event,
            output_in_flight,
            in_flight_ms,
            has_pending,
        ) {
            self.enqueue_reinject(event);
            return CallbackResult::Consumed;
        }

        if matches!(
            raw_event.key_classification,
            awase::types::KeyClassification::Passthrough
        ) {
            tracing::debug!(
                "[relay-passthrough] PassThrough idle: direct OS pass-through (vk={:#04x} {})",
                raw_event.vk_code,
                if is_key_down { "down" } else { "up" },
            );
        }
        CallbackResult::PassThrough
    }

    // ── 共通 ──

    #[tracing::instrument(level = "debug", skip_all, fields(?generation))]
    fn execute_one(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &mut ImeStateHub,
        effect: Effect,
        generation: Option<crate::state::ApplyGeneration>,
    ) -> Option<ImeApplyCompletion> {
        if let Effect::Input(InputEffect::ReinjectKey(event)) = effect {
            Self::handle_reinject(platform, event);
            return None;
        }
        self.dispatch_effect(platform, ime, effect, generation)
            .map(|(open, outcome)| {
                self.update_intra_batch_applied(open, outcome);
                ImeApplyCompletion {
                    open,
                    outcome,
                    generation,
                    reason: crate::state::ime_event::OpenApplyReason::EngineDecision,
                }
            })
    }

    /// 通常 reinject + confirm キー後処理。
    fn handle_reinject(platform: &mut WindowsPlatform, event: RawKeyEvent) {
        let is_key_down = matches!(event.event_type, awase::types::KeyEventType::KeyDown);
        let dir = if is_key_down { "down" } else { "up" };

        // BUG-173: 以前はここで TSF mode の deferred F2 を reinject せず握りつぶしていた
        // （「warmup が F2 を代わりに再送する」double-F2 防止）。ADR-100 決定2 で warmup が
        // `VK_IME_ON` 単発になり代替 F2 が無くなったため、物理 F2 は通常キーと同様に
        // reinject する。cold 化は `kp_stage_execute` の `composition_native_f2_down` が
        // KeyDown ごとに既に実行している（`VK_IME_ON` は送らない）。

        tracing::debug!(
            "[reinject] vk={:#04x} {dir} (queued passthrough now firing)",
            event.vk_code,
        );

        // 案 2a: Space/Enter/Escape (confirm key) KeyDown の composition 後処理を spawn 前に実行する。
        // OUTPUT_GATE.active=true 中は新たなキーが INPUT_DEFER に退避されるため、
        // on_reinject_key を reinject() の前後どちらで呼んでも観測可能な差がない。
        // これにより spawn_local 内の with_app 呼び出しを除去できる。
        if is_key_down && event.vk_code.is_composition_confirm_key() {
            platform.on_reinject_key(event.vk_code, true);
        }

        // OutputActiveGuard を先に取得してから spawn_local で SendInput を RUNTIME 借用外に移す。
        // RUNTIME 借用中に SendInput を呼ぶと WH_KEYBOARD_LL フックが再入し、ユーザーキーが
        // NICOLA 処理をスキップして素通しになる（「いが l になった」バグの原因）。
        // spawn_local 実行中にユーザーキーが届いても OUTPUT_GATE.active=true で INPUT_DEFER
        // に退避され、guard drop 後に drain されて正しく NICOLA 処理される。
        let guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
        win32_async::spawn_local(async move {
            // SAFETY: spawn_local はメインスレッドのメッセージループで実行される。
            unsafe { event.reinject() };
            drop(guard);
        });
    }

    /// Effect::* の match dispatch。
    /// ImeEffect::SetOpen の sync 経路は `Some(..)`、async 経路は `None`（spawn 済み）。
    fn dispatch_effect(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &mut ImeStateHub,
        effect: Effect,
        generation: Option<crate::state::ApplyGeneration>,
    ) -> Option<(bool, awase::platform::ImeOpenOutcome)> {
        // ImeEffect::SetOpen は ImmCross-first か否かで async / sync を分岐するため
        // 先に処理する（後段の `let platform_rt = platform` が `platform`
        // を独占する前に `build_ime_control_view` を呼ぶ必要がある）。
        if let Effect::Ime(ImeEffect::SetOpen { open, press }) = effect {
            // ADR-212 P2: 実 actuation を起こした SetOpen をログで数える（outcome も同じ行に出す。
            // 以前の `origin=`（ActivationSync/ExplicitUserAction）は ADR-213 P2c で SetOpenOrigin ごと撤去し、
            // 全て明示操作になった）。async（ImmCross 先の窓）は `generation` で、後から届く
            // `on_ime_apply_complete{generation outcome}` の行と突き合わせる。
            let result = self.dispatch_ime_set_open(platform, ime, open, press, generation);
            let outcome = result.as_ref().map_or_else(
                || "async".to_string(),
                |(_, outcome)| format!("{outcome:?}"),
            );
            tracing::info!(
                "[set-open] open={open} press={press:?} generation={generation:?} outcome={outcome}"
            );
            return result;
        }
        // EngineStateChanged: エンジン ON/OFF に連動して conv mutation ゲートを更新する。
        // platform_rt (&mut dyn PlatformRuntime) 変換前に行う必要がある。
        // set_conv_mode_authority が Output::conv_mutation_allowed（唯一の実体）へ push する。
        if let Effect::Ui(UiEffect::EngineStateChanged { enabled, .. }) = &effect {
            let authority = if *enabled {
                ConvModeAuthority::AwaseOwned
            } else {
                ConvModeAuthority::UserOwned
            };
            platform.set_conv_mode_authority(authority);
            // Alt なりすまし（left/right_thumb_key == "Left Alt"/"Right Alt"）の
            // 発動条件。フックスレッドから同期的に読めるようキャッシュを更新する。
            crate::hook::set_engine_enabled(*enabled);
        }
        if let Effect::Input(InputEffect::SendKeys(actions)) = &effect {
            let passes_mode_key = actions.iter().any(|action| {
                matches!(
                    action,
                    awase::types::KeyAction::Key(vk) if crate::vk::is_followed_mode_key(*vk)
                )
            });
            if passes_mode_key {
                let now = crate::hook::current_tick_ms();
                // ADR-188・ADR-244: FSM 再送出でも直接観測の窓を開く（GJI／同定済み MS-IME 本体 × Imm32Unavailable、変換中は除く）。
                if super::direct_mode_key_watch_kind_for(platform.current_app_profile()).is_some()
                    && !crate::tsf::observer::ime_composition_active_now()
                {
                    ime.arm_direct_external_change_watch(now);
                }
                ime.arm_mode_key_pass_mark(
                    now,
                    platform.current_app_profile().can_use_imm32_cross_process(),
                );
                platform.timer.set(
                    crate::TIMER_IME_REFRESH,
                    std::time::Duration::from_millis(20),
                );
                tracing::info!(
                    "[mode-key-follow] mode key sent through FSM: IME refresh scheduled (20ms)"
                );
            }
        }
        let platform_rt: &mut dyn PlatformRuntime = platform;
        match effect {
            Effect::Input(ie) => match ie {
                InputEffect::SendKeys(actions) => {
                    platform_rt.send_keys(&actions);
                    None
                }
                InputEffect::ReinjectKey(_) => unreachable!("handled in execute_one"),
            },
            Effect::Timer(te) => match te {
                TimerEffect::Set { id, duration } => {
                    platform_rt.set_timer(id, duration);
                    None
                }
                TimerEffect::Kill(id) => {
                    platform_rt.kill_timer(id);
                    None
                }
            },
            Effect::Ime(ie) => match ie {
                ImeEffect::SetOpen { .. } => unreachable!("handled above"),
            },
            Effect::Ui(ue) => match ue {
                UiEffect::EngineStateChanged { enabled } => {
                    platform_rt.update_tray(enabled);
                    None
                }
            },
        }
    }

    /// `ImeEffect::SetOpen` の専用 dispatch（殻）。
    ///
    /// 判断（D1 の未知化 → gate〈issue #136 / BUG-90 決定4〉→ 押下の予約 → `plan_set_open` → 同期チェーン → 予約の解除・記録）は
    /// 核の `state::sync_actuation::dispatch_set_open`（ADR-241 決定2）。ここは Facts を集め（shell-in）、
    /// 核を呼び、ImmCross が先頭の窓では返ってきた order で `spawn_local` の非同期チェーンを走らせる（shell-out）。
    /// 非同期は `None`（spawn 済み）、それ以外（GjiDirect / MsImeDirect 経路、キー注入のみで非ブロッキング）は
    /// `Some(..)` を返す。
    ///
    /// `press`（ADR-208 決定2 D1）: この `SetOpen` を起こしたユーザー打鍵（非リピート KeyDown）の押下 ID。
    /// `Some` の書き込みは、(1) 同じ押下で既に同じ向きを予約済みなら書かず（BUG-113 の二重送信防止。向きが逆なら
    /// Engine の明示コンボが優先して書く）、(2) 決定入力の `shadow_on` を `applied` が向きと一致していても未知にして
    /// GjiDirect の already-matched 省略を外す（S-1: Blind 窓で stale な `applied` により絶対キーが握りつぶされ続ける
    /// 固着の解消）。`None`（自動リピート等）は従来どおり `applied_snapshot` のまま。
    #[tracing::instrument(level = "debug", skip_all, fields(open = open, ?press, ?generation))]
    fn dispatch_ime_set_open(
        &mut self,
        platform: &WindowsPlatform,
        ime: &mut ImeStateHub,
        open: bool,
        press: Option<awase::types::PressId>,
        generation: Option<crate::state::ApplyGeneration>,
    ) -> Option<(bool, awase::platform::ImeOpenOutcome)> {
        use crate::state::sync_actuation::{dispatch_set_open, SetOpenDispatch, SetOpenRequest};
        // view は imm_first 判定と sync path の両方で使うため一度だけ構築する。`shadow_on` は `applied_snapshot` の
        // 素の値で、D1 の未知化は核が決定入力に対して行う（view の `shadow_on` は送信口〈`apply_mechanism`〉が読まない）。
        let effectively_tsf_native = platform
            .current_app_profile()
            .is_effectively_tsf_native(platform.focus.class_name());
        let mut view = platform.build_ime_control_view(self.applied_snapshot.applied_open());
        view.belief_input_mode = self.belief_input_mode;
        let request = SetOpenRequest {
            open,
            press,
            inputs: (&view).into(),
            effectively_tsf_native,
        };
        let mut sink = crate::ime_controller::ShellCommandSink::new(&view);
        match dispatch_set_open(ime, request, view.focus.class_name, &mut sink) {
            SetOpenDispatch::NotOwned => Some((open, awase::platform::ImeOpenOutcome::NotOwned)),
            // 完了へ流す outcome は「送っていない」もの（`AlreadyMatched` だと書いていない押下が applied=Confirmed になる）。
            SetOpenDispatch::SkipAlreadyClaimed => {
                Some((open, crate::state::press_ledger::DUPLICATE_OUTCOME))
            }
            SetOpenDispatch::SyncChain { outcome } => Some((open, outcome)),
            SetOpenDispatch::AsyncImmCross { order } => {
                // ── async path (ImmCross が選ばれるアプリ) ──
                // OutputActiveGuard を先に取得しておくことで、await 中に走るフックコールバックは
                // INPUT_DEFER へ退避され、SetOpen 進行中に新キーが engine に届かない。
                //
                // async 完了前は applied_snapshot が旧値のままなので、同一バッチ内の後続 effect や
                // 次の判定（`build_ime_control_view` の `shadow_on` 供給、`resolve_warmup_ime_on`）が
                // 「まだ揃っていない」と誤判断しないよう、楽観的に更新する。
                // （かつては `send_engine_state_ime_key` のモードキー送信を止める役目もあった。
                // ADR-207 で撤去したが、上記の消費者があるためこの更新は残す。）
                self.applied_snapshot = crate::state::AppliedImeState::Optimistic(open);
                // IMM が set_ime_open_cross_process(open) 完了後に注入する VK_DBE_DBCSCHAR/
                // VK_DBE_SBCSCHAR KeyUp は key_pipeline の suppress_physical (ImmCross プロファイル
                // の KANJI VK 全 Consume) で構造的に遮断されるため、ここでは applied_snapshot 更新のみ。
                tracing::debug!(
                    "[dispatch-ime] ImmCross async: optimistic applied_snapshot={open}"
                );
                // ImmCross の set_ime_open_cross_process は IMC_SETOPENSTATUS のみ設定し
                // conv mode は変更しない。IME がかなモード (conv=0x09) のまま ON になると
                // NICOLA エンジンが is_romaji_capable=false で起動できない。
                // MsImeDirectStrategy と同じく ObservedKana 以外なら ROMAN ビットを補完する。
                // ImmCross アプリは ir_poll_and_learn で ObservedKana の観測を抑制するため
                // belief は ObservedKana にならず、ここに到達したときは常に補完対象になる。
                let guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
                // ADR-086 §1.2 欠陥1 是正（opus レビュー指摘 2026-08-08）: 「open と
                // 同じウィンドウへ ROMAN ビットを補完する」という意図を、open/conv を
                // 別々に検証していたのでは保証できない（open 完了を待つ間にフォーカスが
                // 動いても ime_mode_focus_gen の更新が遅れるため、conv 側の再検証だけでは
                // 検知できず無関係な別ウィンドウへ ROMAN が着弾しうる）。起案時点の
                // focus_gen を捕獲し、実際の verify → open → conv はすべて
                // set_ime_open_then_conv_for_target 1回に閉じ込めて同一 hwnd を使い回す。
                let focus_gen = platform.output.ime_mode_focus_gen.get();
                let conv_after_open: crate::ime::ConvAfterOpen =
                    crate::state::ime_actuation_decision::decide_dispatch_conv_after_open(
                        (&view).into(),
                        open,
                    )
                    .into();
                win32_async::spawn_local(async move {
                    let Some(target) = crate::ime::ActuationTarget::capture(focus_gen).await else {
                        tracing::debug!(
                            "[dispatch-ime] capture 失敗（フォーカス無し） → UnsafeToToggle"
                        );
                        crate::runtime::message_handlers::post_async_ime_apply_complete(
                            open,
                            awase::platform::ImeOpenOutcome::UnsafeToToggle,
                            generation,
                            crate::state::ime_event::OpenApplyReason::EngineDecision,
                        );
                        drop(guard);
                        return;
                    };
                    // ADR-089 §2.3 Phase B: ImmCross を機構チェーンの**要素**として
                    // 実行する。`Failed` のときのフォールスルー（旧
                    // `apply_skipping_imm`）は `run_chain_async` が行うため、ここに
                    // 分岐は書かない（走査規則の SSOT は `state/actuation_chain.rs`）。
                    let outcome = crate::runtime::open_chain::run_open_chain_async(
                        order,
                        crate::runtime::open_chain::ImmCrossOp::Targeted {
                            target,
                            conv_after_open,
                            focus_gen,
                        },
                        crate::state::ime_actuation_decision::DecisionSite::DispatchImeSetOpen,
                        // ADR-163 Part D S-8対応: `site`自体が`DispatchImeSetOpen`
                        // として一意に識別できるため、追加のラベルは不要。
                        None,
                    )
                    .await;
                    // sync path（sync_outcomes → dispatch_outcomes → on_ime_apply_complete）と
                    // 対称に、完了 outcome を WM 経由で Runtime の単一入口へ委譲する。
                    // spawn_local の future 内で with_app を直接握らないことで再入面を減らし、
                    // generation 照合を含む B+C+D+E を on_ime_apply_complete に一元化する。
                    crate::runtime::message_handlers::post_async_ime_apply_complete(
                        open,
                        outcome,
                        generation,
                        crate::state::ime_event::OpenApplyReason::EngineDecision,
                    );
                    drop(guard);
                });
                None
            }
        }
    }

    /// intra-batch の applied_snapshot のみを更新する。
    ///
    /// sync SetOpen 直後に同一バッチ内の後続 effect が
    /// 参照するキャッシュを更新するためだけに使う（`execute_one` からのみ呼ばれる）。
    /// B（`on_ime_applied`）と C（ImeModel write-back）は `Runtime::on_ime_apply_complete`
    /// に委譲済み。UnsafeToToggle は送信していないので更新しない。
    ///
    /// async path は完了時にバッチが既に終わっており、次バッチ開始時に
    /// `applied_snapshot = ime.model().applied` で SSOT から再取得されるため、
    /// 完了時の intra-batch 更新は不要（`on_ime_apply_complete` が SSOT を更新する）。
    fn update_intra_batch_applied(&mut self, open: bool, outcome: awase::platform::ImeOpenOutcome) {
        use awase::platform::ImeOpenOutcome;
        if matches!(
            outcome,
            ImeOpenOutcome::UnsafeToToggle | ImeOpenOutcome::NotOwned | ImeOpenOutcome::Unwarranted
        ) {
            return;
        }
        let effective = match outcome {
            ImeOpenOutcome::Applied
            | ImeOpenOutcome::AppliedWithoutSendInput
            | ImeOpenOutcome::AlreadyMatched => open,
            ImeOpenOutcome::Failed => !open,
            ImeOpenOutcome::UnsafeToToggle
            | ImeOpenOutcome::NotOwned
            | ImeOpenOutcome::Unwarranted => unreachable!(),
        };
        self.applied_snapshot = crate::state::AppliedImeState::Confirmed {
            open: effective,
            at_ms: crate::hook::current_tick_ms(),
        };
    }
}

/// `AppliedImeState` の unit tests。
///
/// `awase-windows` クレートは `#![cfg(windows)]` で囲まれているため
/// Windows 実機でのみ実行される。
#[cfg(test)]
mod tests {
    use crate::state::AppliedImeState;

    // AppliedImeState ヘルパーメソッドのテスト
    #[test]
    fn applied_ime_state_applied_open() {
        assert_eq!(AppliedImeState::Unknown.applied_open(), None);
        assert_eq!(AppliedImeState::Optimistic(true).applied_open(), Some(true));
        assert_eq!(
            AppliedImeState::Confirmed {
                open: false,
                at_ms: 1
            }
            .applied_open(),
            Some(false)
        );
    }

    #[test]
    fn applied_ime_state_is_confirmed() {
        assert!(!AppliedImeState::Unknown.is_confirmed());
        assert!(!AppliedImeState::Optimistic(true).is_confirmed());
        assert!(AppliedImeState::Confirmed {
            open: true,
            at_ms: 1
        }
        .is_confirmed());
    }
}
