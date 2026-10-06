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

impl ImeStateHub {
    /// 実 actuation の 1 件を起案する（ADR-090 §2.A A-1、INV-47）。
    ///
    /// `DecisionExecutor` は `Runtime` を持たないため
    /// [`Runtime::issue_actuation_order`](super::Runtime::issue_actuation_order) を
    /// 使えないが、4 つの公開入口（`execute_from_hook` / `execute_from_loop` /
    /// `drain_deferred` / `on_output_guard_timer`）が**既に `ime: &mut ImeStateHub` を
    /// 受け取っている**ので、それを `dispatch_ime_set_open` まで通すだけで
    /// warrant を発行できる。
    ///
    /// **`crate::with_app` で `ImeStateHub` を取りに行ってはならない**——ここは
    /// 既に `with_app` の内側であり、再入すると panic せず `None` が返る。
    /// つまり「取れなかった」ことと「授権が下りなかった」が区別できない形で
    /// 静かに落ち、A-1 の shadow ログが測ろうとしている当のものが汚染される
    /// （ADR-090 §2.A.2(1)・§4.2）。
    ///
    /// # 似た名前のメソッドとの違い（意図的に区別すること）
    ///
    /// - [`Self::issue_actuation_order`]（`state/platform_state.rs`）: 最下層。
    ///   `origin`/`now`/`now_ms` を呼び出し元が組み立てて渡す。本メソッドの
    ///   実装はこれをそのまま呼ぶ。
    /// - [`Runtime::issue_actuation_order`](super::Runtime::issue_actuation_order) /
    ///   [`Runtime::issue_actuation_order_with_origin`](super::Runtime::issue_actuation_order_with_origin)
    ///   （`runtime/mod.rs`）: `Runtime` を持つ呼び出し元向けの同型の便利メソッド。
    ///   本メソッドはそれの `ImeStateHub` 版（`Runtime` を持たない
    ///   `DecisionExecutor` 用）であり、**ロジックは意図的に重複している**
    ///   （統合すると `DecisionExecutor` に `Runtime` 依存を持ち込むことになり、
    ///   上記のとおりそれ自体が本メソッドの存在理由を壊す）。
    ///
    /// 2026-09-10、自由関数`issue_order`からメソッドへ変更した際、`Runtime::
    /// issue_actuation_order`と紛らわしいと指摘を受け`issue_self_actuation_order`
    /// にリネームした（常に`EventSource::SelfActuated`を組み立てることを名前に
    /// 反映）。挙動は変更していない。
    fn issue_self_actuation_order(
        &self,
        open: bool,
        strategy: &'static str,
    ) -> crate::state::actuation_chain::ActuationOrder {
        let origin = crate::state::event_origin::EventOrigin::new(
            crate::state::event_origin::EventSource::SelfActuated { strategy },
            crate::state::event_origin::Generation::INITIAL,
        );
        let now = std::time::Instant::now();
        let now_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        self.issue_actuation_order(open, origin, now, now_ms)
    }
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
            let wait = self.reinject_wait_remaining(platform, &event);
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
        match (plan.action, decision) {
            // physical=Suppress（KANJI 物理キー抑止）の場合は OS に届けず Consume する。
            // handle_passthrough の reinject/warmup 後処理も走らせない。
            (relay_plan::RelayAction::ConsumeSuppressed, _) => BatchResult {
                has_pending: self.has_pending(),
                callback: CallbackResult::Consumed,
                sync_outcomes: Vec::new(),
            },
            (relay_plan::RelayAction::RunPassthroughPipeline, _) => {
                let callback = self.run_passthrough_pipeline(platform, raw_event);
                BatchResult {
                    has_pending: self.has_pending(),
                    callback,
                    sync_outcomes: Vec::new(),
                }
            }
            (
                relay_plan::RelayAction::QueueFlush { reinject },
                Decision::PassThroughWith { mut effects },
            ) => {
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
                    plan.reason,
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
            (relay_plan::RelayAction::ConsumeEffects, Decision::Consume { effects }) => {
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
            _ => unreachable!("plan_relay は Decision の種別と対応する action だけを返す"),
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

    /// `ImeEffect::SetOpen` の専用 dispatch。
    ///
    /// `ImmCrossProcessStrategy` が現在のコンテキストで最初に適用可能な場合は
    /// `win32_async::spawn_local` で非同期実行し `None` を返す（spawn 済み）。
    /// それ以外（GjiDirect / MsImeDirect 経路）はキー注入のみで非ブロッキングなため
    /// 既存の同期 chain を維持し、`Some(..)` を返す。
    ///
    /// `press`（ADR-208 決定2 D1）: この `SetOpen` を起こしたユーザー打鍵（非リピート KeyDown）の押下 ID。
    /// `Some` の書き込みは、(1) 同じ押下で既に同じ向きを予約済みなら書かず（BUG-113 の二重送信防止。向きが逆なら
    /// Engine の明示コンボが優先して書く）、(2) view の `shadow_on` を `applied` が向きと一致していても未知にして
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
        // view は imm_first 判定と sync path の両方で使うため一度だけ構築する。
        // D1: 押下の書き込みは `applied` を省略の根拠にしない（`applied` 自体は書き換えない）。ただし TsfNative の窓は
        // BUG-124 型の「@」の実機 A/B（ADR-208 L3'）が済むまで従来のまま（`engine_press_unknowns_applied`）。
        let unknowns_applied = press.is_some()
            && crate::state::ime_actuation_decision::engine_press_unknowns_applied(
                platform
                    .current_app_profile()
                    .is_effectively_tsf_native(platform.focus.class_name()),
            );
        let mut view = platform.build_ime_control_view(
            crate::state::ime_actuation_decision::explicit_press_applied_pair(
                self.applied_snapshot.applied_open(),
                open,
                unknowns_applied,
            ),
        );
        view.belief_input_mode = self.belief_input_mode;
        let gate_inputs = (&view).into();
        // claim 以降の判断は `state/ime_set_open_plan.rs`（核）。ここは Facts を作る（shell-in）→ 決める → 実行する（shell-out）。
        if matches!(
            crate::state::ime_actuation_decision::decide_gate(gate_inputs),
            crate::state::ime_actuation_decision::GateResult::NotOwned
        ) {
            // /code-review指摘（B-3、PR #201）: ADR-163がDecisionSite::
            // DispatchImeSetOpenを新設した理由は、この早期gate（下のimm_first
            // 判定・sync path双方より前の、executor側だけが持つ独立した
            // 判定点）を「Syncに畳むと回帰が記録上区別できなくなる」ため
            // 区別する必要があったからだが、以前はこのgateがNotOwnedを
            // 返すケースを一切記録していなかった。ここで初めて実際に
            // site=DispatchImeSetOpenのレコードを積む。まだ`ActuationOrder`は
            // 発行されていない（両分岐が自分の理由文字列で個別に発行する）ため、
            // この記録専用に使い捨てのorderを発行する——`ActuationOrder::issue`
            // はA-1（shadow）段階の純粋な読み取りで、発行して`chain`に通さず
            // 破棄しても既存の警告(warrant)会計に副作用は無い
            // （`state/platform_state.rs::issue_actuation_order`のdoc参照）。
            let gate_reject_order =
                ime.issue_self_actuation_order(open, "dispatch_ime_set_open_gate_not_owned");
            let record = crate::state::actuation_decision_record::ActuationDecisionRecord {
                site: crate::state::ime_actuation_decision::DecisionSite::DispatchImeSetOpen,
                gate_inputs,
                order: crate::state::actuation_decision_record::ActuationOrderRecord::from(
                    &gate_reject_order,
                ),
                chain: [None; crate::state::actuation_decision_record::MAX_WRITE_MECHANISMS],
                chain_len: 0,
                attempts: [None; crate::state::actuation_decision_record::MAX_WRITE_MECHANISMS],
                attempts_len: 0,
                caller: None,
            };
            ime.journal
                .record(crate::journal::JournalEntry::ActuationDecision { record });
            return Some((open, awase::platform::ImeOpenOutcome::NotOwned));
        }
        // ADR-208 D1: この押下で既に書いた（同じ向き）なら書かない。order の発行直前に予約する
        // （ImmCross の async は完了が WM 経由で後から届くため、完了時の記録では同じ打鍵の二重送信を防げない）。
        // 非同期（ImmCross 先頭の窓）は完了が後から届くので、書けなくても予約は解かない（次の押下で直る）。
        // 同期は何も送らなかったときだけ下で解く（`release_press_write`）。上の gate（NotOwned）で返済みなので、
        // 書かない窓では予約しない。
        let claim =
            ime.claim_press_write(press, open, crate::state::press_ledger::PressSource::Engine);
        let plan = crate::state::ime_set_open_plan::plan_set_open(
            crate::state::ime_set_open_plan::SetOpenFacts {
                claim,
                imm_first: crate::ime_controller::ImeController::imm_cross_is_first_applicable(
                    &view,
                ),
            },
        );
        if plan == crate::state::ime_set_open_plan::SetOpenPlan::SkipAlreadyClaimed {
            tracing::debug!(
                "[dispatch-ime] 同じ押下で既に書いた（{}）→ 書かない press={press:?} open={open}",
                claim.label()
            );
            // 完了へ流す outcome は「送っていない」もの（`AlreadyMatched` だと書いていない押下が applied=Confirmed になる）。
            return Some((open, crate::state::press_ledger::DUPLICATE_OUTCOME));
        }
        if plan == crate::state::ime_set_open_plan::SetOpenPlan::AsyncImmCross {
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
            tracing::debug!("[dispatch-ime] ImmCross async: optimistic applied_snapshot={open}");
            // ImmCross の set_ime_open_cross_process は IMC_SETOPENSTATUS のみ設定し
            // conv mode は変更しない。IME がかなモード (conv=0x09) のまま ON になると
            // NICOLA エンジンが is_romaji_capable=false で起動できない。
            // MsImeDirectStrategy と同じく ObservedKana 以外なら ROMAN ビットを補完する。
            // ImmCross アプリは ir_poll_and_learn で ObservedKana の観測を抑制するため
            // belief は ObservedKana にならず、ここに到達したときは常に補完対象になる。
            // ADR-090 §2.A A-1（shadow）: 起案は spawn_local の**外**で行う
            // ——future の中では `with_app` 再入で `ImeStateHub` に届かない
            // （ADR-090 §4.2）。
            let order = ime
                .issue_self_actuation_order(open, "engine_decision_async")
                .with_press(press);
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
        } else {
            // ── sync path (Chrome / GJI 経路 / TsfNative 経路) ──
            // ADR-090 §2.A A-1（shadow）。
            let order = ime
                .issue_self_actuation_order(open, "engine_decision_sync")
                .with_press(press);
            let (outcome, mut record) = platform.apply_ime_open_with_view(order, &view);
            // 同期の書き込みが何も送らなかったなら予約を解く（同じ押下の次の経路が書ける。async は解けない）。
            if crate::state::press_ledger::outcome_sent_nothing(outcome) {
                ime.release_press_write(press, open);
            }
            // /code-review指摘（B-2、PR #201）: `site`は上書きしない——
            // `decide_attempt`は常に`Sync`で呼ばれておりrecord.siteもSyncの
            // ままなので、`replay_record`のchain再導出/ImmCross command
            // 再計算検証を維持できる。呼び出し元の識別は独立の`caller`
            // フィールドに記録する。
            record.caller =
                Some(crate::state::ime_actuation_decision::DecisionSite::DispatchImeSetOpen);
            ime.journal
                .record(crate::journal::JournalEntry::ActuationDecision { record });
            if outcome == awase::platform::ImeOpenOutcome::Failed {
                tracing::warn!("apply_ime_open({open}) failed");
            }
            Some((open, outcome))
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
