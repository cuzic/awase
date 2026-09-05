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

/// 実 actuation の 1 件を起案する（ADR-090 §2.A A-1、INV-47）。
///
/// `DecisionExecutor` は `Runtime` を持たないため
/// `Runtime::issue_actuation_order` を使えないが、4 つの公開入口
/// （`execute_from_hook` / `execute_from_loop` / `drain_deferred` /
/// `on_output_guard_timer`）が**既に `ime: &ImeStateHub` を受け取っている**ので、
/// それを `dispatch_ime_set_open` まで通すだけで warrant を発行できる。
///
/// **`crate::with_app` で `ImeStateHub` を取りに行ってはならない**——ここは
/// 既に `with_app` の内側であり、再入すると panic せず `None` が返る。
/// つまり「取れなかった」ことと「授権が下りなかった」が区別できない形で
/// 静かに落ち、A-1 の shadow ログが測ろうとしている当のものが汚染される
/// （ADR-090 §2.A.2(1)・§4.2）。
fn issue_order(
    ime: &ImeStateHub,
    open: bool,
    strategy: &'static str,
) -> crate::state::actuation_chain::ActuationOrder {
    let origin = crate::state::event_origin::EventOrigin::new(
        crate::state::event_origin::EventSource::SelfActuated { strategy },
        crate::state::event_origin::Generation::INITIAL,
    );
    let now = std::time::Instant::now();
    let now_ms = crate::state::TickMs(crate::hook::current_tick_ms());
    ime.issue_actuation_order(open, origin, now, now_ms)
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

/// FocusTransition settle 期間中に Engine が発行した `ImeEffect::SetOpen` を decision から取り除く。
///
/// 「settle 中は SetOpen を実行させない」という不変条件の**一次フィルタ（decision 除去）の
/// 単一実装**。SetOpen が実行に到達する Decision 経路は 2 つだけ（`435e2d3` の調査）:
/// - キーボード経路: `key_pipeline::kp_run_inner`（`focus_transition_was_pending` スナップショット）
/// - 非キーボード経路: `execute_from_loop`（`is_focus_transition_settling` のライブ評価）
///
/// どちらの settle 判定を使うかは呼び出し元が持ち（barrier consume タイミングが異なるため）、
/// ここでは `settling` を受け取るだけで判定条件自体は変えない。belief（`desired_open` 等）を
/// 汚染させない最終防衛線は `ImeStateHub::handle_engine_set_open` にある（意図が異なるため別に残す）。
///
/// settle 中に落とした事実は必ずログに残す（無音で消すと focus 遷移バグの調査コストが跳ね上がるため）。
///
/// 戻り値 `Some(target)` は「本来 apply されるはずだった SetOpen(target) を握りつぶした」ことを
/// 呼び出し元に伝える。`Engine::check_active_transition` は該当する Active/Inactive 遷移を
/// この呼び出しより前に確定させており（`prev_activation` 更新はログ出力と同時、effect 実行より
/// 前）、以後 belief が変わらない限り同じ遷移は二度と検知されない＝この SetOpen は自然には
/// 再発行されない。呼び出し元は `Some` を受けたら settle 明けの再試行
/// （`focus_settle_ms() + 50`ms 後、`apply_force_on_for_imm_broken` 等と同じ確立済みパターン）を
/// 必ずスケジュールすること（さもないと GjiFsm 等 apply 完了通知でしか同期しないサブシステムが
/// 実 IME 状態と乖離したまま固着する。2026-07-08 実機: 「このせっけい」が「せっけい」に文字欠落）。
#[must_use]
pub(crate) fn strip_ime_set_open_if_settling(
    decision: &mut Decision,
    settling: bool,
) -> Option<bool> {
    if !settling {
        return None;
    }
    let target = decision.find_ime_set_open()?;
    decision
        .effects_mut()
        .retain(|e| !matches!(e, Effect::Ime(ImeEffect::SetOpen { .. })));
    log::debug!(
        "[focus-settle] SetOpen({target}) effect stripped from decision \
         (focus transition barrier still settling)"
    );
    Some(target)
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
    pub(crate) fn execute_from_hook(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &ImeStateHub,
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
    /// 2026-07-05: Alt+Tab 中間ウィンドウへの一瞬のフォーカスで `Engine::on_command`
    /// (`FocusChanged`/`RefreshState`) が Active/Inactive 遷移を検知し `ImeEffect::SetOpen`
    /// を発行 → ここで無条件に実行されて実際に SendInput してしまうバグを修正。
    /// settle 期間中は `SetOpen` effect を取り除いてから実行する
    /// （`key_pipeline.rs` の `kp_run_inner` と同じパターン）。
    ///
    /// 戻り値の第3要素は `strip_ime_set_open_if_settling` が握りつぶした SetOpen の目標値。
    /// `Some` の場合、呼び出し元は settle 明けの再試行を必ずスケジュールすること
    /// （`Engine::prev_activation` は該当遷移を確定済みで、同じ SetOpen は自然には
    /// 再発行されないため。2026-07-08: GjiFsm が resync できず「このせっけい」の
    /// 文字欠落に至った実機ログから判明）。
    pub(crate) fn execute_from_loop(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &ImeStateHub,
        mut decision: Decision,
    ) -> (CallbackResult, Vec<ImeApplyPair>, Option<bool>) {
        self.applied_snapshot = ime.model().applied;
        self.belief_input_mode = ime.input_mode();
        // 非キーボード経路の一次フィルタ。ここは barrier consume を経ないので settling をライブ評価する。
        let stripped_set_open = strip_ime_set_open_if_settling(
            &mut decision,
            ime.is_focus_transition_settling(std::time::Instant::now()),
        );
        let (consumed, effects) = match decision {
            Decision::PassThrough => {
                return (CallbackResult::PassThrough, Vec::new(), stripped_set_open)
            }
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
        (callback, sync_outcomes, stripped_set_open)
    }

    /// `WM_EXECUTE_EFFECTS` ハンドラ、および `TIMER_OUTPUT_GUARD` タイマーから呼ぶ。
    ///
    /// `guard_held` に park 済みの Effect があれば最初にそれを試し、
    /// output guard 期間中なら `TIMER_OUTPUT_GUARD` を設定して即座に返る（block_on しない）。
    /// タイマー発火後に再び呼ばれ、guard 解除済みなら reinject を実行する。
    #[expect(clippy::useless_let_if_seq)]
    pub(crate) fn drain_deferred(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &ImeStateHub,
    ) -> Vec<ImeApplyPair> {
        // 同一 drain 呼び出し内で最初の ReinjectKey だけ OUTPUT_GUARD を適用する。
        // 連続する reinject (例: Win_DOWN→X_DOWN→X_UP→Win_UP) を個別にガードすると
        // Win が 150ms 以上 OS 側でスタックし、後続のショートカットが Win+key と
        // 誤解釈されるため、先頭の reinject が guard を通過したら残りはまとめて送出する。
        let mut sync_outcomes = Vec::new();
        let mut reinject_guard_passed = false;

        // 1) 前回 park した ReinjectKey があれば最初に試す。
        //    guard 解除済みなら execute_one してから queue に進む (batching を継続)。
        if let Some(event) = self.guard_held.take() {
            if let Some(remaining) = self.reinject_wait_remaining(platform, &event) {
                log::debug!(
                    "[reinject-guard] held event, suspending for {remaining}ms (vk={:#04x})",
                    event.vk_code,
                );
                self.park_in_guard(platform, event, remaining);
                return sync_outcomes;
            }
            let effect = Effect::Input(InputEffect::ReinjectKey(event));
            let generation = ime.model().pending_generation();
            if let Some(o) = self.execute_one(platform, ime, effect, generation) {
                sync_outcomes.push(o);
            }
            reinject_guard_passed = true;
        }

        // 2) queue を FIFO で drain。
        while let Some(mut effect) = self.queue.pop_front() {
            let is_reinject = matches!(effect, Effect::Input(InputEffect::ReinjectKey(_)));
            if is_reinject && !reinject_guard_passed {
                let Effect::Input(InputEffect::ReinjectKey(event)) = effect else {
                    unreachable!("is_reinject was true")
                };
                if let Some(remaining) = self.reinject_wait_remaining(platform, &event) {
                    log::debug!(
                        "[reinject-guard] suspending drain for {remaining}ms (vk={:#04x})",
                        event.vk_code,
                    );
                    self.park_in_guard(platform, event, remaining);
                    return sync_outcomes;
                }
                effect = Effect::Input(InputEffect::ReinjectKey(event));
                reinject_guard_passed = true;
            } else if !is_reinject {
                // NICOLA 出力など reinject 以外の effect は mark_send を呼ぶので
                // 次の reinject には再びガードを適用する。
                reinject_guard_passed = false;
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
        ime: &ImeStateHub,
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
        if matches!(event.event_type, awase::types::KeyEventType::KeyDown)
            && event.vk_code.is_composition_confirm_key()
            && platform.has_pending_tsf_work()
        {
            return Some(10);
        }

        let elapsed = platform.output_in_flight_ms();
        if elapsed < crate::tuning::OUTPUT_GUARD_MS {
            Some(crate::tuning::OUTPUT_GUARD_MS - elapsed)
        } else {
            None
        }
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
        ime: &ImeStateHub,
        decision: Decision,
        raw_event: &RawKeyEvent,
        physical: PhysicalKeyDisposition,
    ) -> BatchResult {
        match decision {
            Decision::PassThrough => {
                // physical=Suppress（KANJI 物理キー抑止）の場合は OS に届けず Consume する。
                // handle_passthrough の reinject/warmup 後処理も走らせない。
                if physical == PhysicalKeyDisposition::Suppress {
                    return BatchResult {
                        has_pending: self.has_pending(),
                        callback: CallbackResult::Consumed,
                        sync_outcomes: Vec::new(),
                    };
                }
                let callback = self.run_passthrough_pipeline(platform, ime, raw_event);
                BatchResult {
                    has_pending: self.has_pending(),
                    callback,
                    sync_outcomes: Vec::new(),
                }
            }
            Decision::PassThroughWith { mut effects } => {
                // flush 出力あり → Consume して flush + キー再注入を FIFO でキュー。
                // physical=Suppress（KANJI 物理キー抑止）の場合は reinject を積まない。
                let reinject = physical == PhysicalKeyDisposition::Allow;
                log::debug!(
                    "[relay-flush] PassThroughWith: queue {} effect(s){} (vk={:#04x} {})",
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
                // それ以外はキューに入れる。
                //
                // Timer を即時実行しない場合、drain 中に Kill/Set がキューに積まれたまま
                // platform の current_os_id が更新されず、deferred_engine_timers の
                // os_id 照合が stale なタイマーを有効と誤判定して早期発火する
                // （例: PendingChar(S)→PendingChar(D) 遷移後に古い S のタイマーが発火）。
                let mut sync_outcomes = Vec::new();
                for effect in effects {
                    if matches!(effect, Effect::Timer(_)) {
                        let generation = ime.model().pending_generation();
                        if let Some(o) = self.execute_one(platform, ime, effect, generation) {
                            sync_outcomes.push(o);
                        }
                    } else {
                        self.queue.push_back(effect);
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
    ///   B. [platform]  確認キー KeyUp warmup / Ctrl↑ cold recovery（副作用のみ）
    ///   C. [transport] output guard defer — 出力 in-flight 中は reinject 経由で順序保証
    ///   D. [platform]  確認キー KeyDown passthrough 後処理（副作用のみ）
    ///   → PassThrough
    fn run_passthrough_pipeline(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &ImeStateHub,
        raw_event: &RawKeyEvent,
    ) -> CallbackResult {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);

        // A. [transport] KeyUp 対称性
        if let Some(event) = self.passthrough_queue.check_keyup_symmetry(raw_event) {
            self.enqueue_reinject(event);
            return CallbackResult::Consumed;
        }

        // B. [platform] 副作用（defer されても FSM は進める）
        self.try_pending_warmup_on_keyup(platform, ime, raw_event);
        self.handle_ctrl_up_recovery(platform, ime, raw_event);

        // C. [transport] output guard defer
        let in_flight_ms = platform.output_in_flight_ms();
        let output_in_flight = in_flight_ms < crate::tuning::OUTPUT_GUARD_MS;
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
        log::debug!(
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

        // D. [platform] 確認キー後処理
        self.handle_confirm_key_passthrough(platform, ime, raw_event);

        if matches!(
            raw_event.key_classification,
            awase::types::KeyClassification::Passthrough
        ) {
            log::debug!(
                "[relay-passthrough] PassThrough idle: direct OS pass-through (vk={:#04x} {})",
                raw_event.vk_code,
                if is_key_down { "down" } else { "up" },
            );
        }
        CallbackResult::PassThrough
    }

    /// warm+TSF Enter/Space/Escape KeyDown で保留した eager warmup を KeyUp で送信する。
    /// KeyDown 時は SendInput(F2) → CallNextHookEx(Enter↓) の順になり WezTerm が
    /// F2 (新 composition 開始) を受け取った後に Enter で即確定してしまう。
    /// KeyUp タイミングでは Enter↓ が既に処理済みのため F2 との競合なし。
    ///
    /// 保留状態は `CompositionFsm` が `PendingWarmupOnKeyUp` として持つ。
    /// KeyUp を FSM に feed し、保留があれば dispatcher が warmup を送信する。
    fn try_pending_warmup_on_keyup(
        &self,
        platform: &mut WindowsPlatform,
        ime: &ImeStateHub,
        raw_event: &RawKeyEvent,
    ) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if !is_key_down && raw_event.vk_code.is_composition_confirm_key() {
            platform.composition_confirm_key_up(
                raw_event.vk_code,
                ime.resolve_warmup_ime_on(self.applied_snapshot, std::time::Instant::now()),
            );
        }
    }

    /// Ctrl↑: cold 状態であれば eager_warmup_sent_ms をリセット（この→kおの バグ対策）。
    /// Ctrl が WezTerm に届いている間、GJI TSF 初期化が中断される可能性がある。
    /// Ctrl↑ を起点としてタイマーを再計測し GJI recovery 時間（500ms）を確保する。
    /// cold 判定・warmup 送信は `CompositionFsm`（CtrlUp）に委譲する。副作用のみ。
    fn handle_ctrl_up_recovery(
        &self,
        platform: &mut WindowsPlatform,
        ime: &ImeStateHub,
        raw_event: &RawKeyEvent,
    ) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if !is_key_down && raw_event.vk_code.is_ctrl_variant() {
            platform.composition_ctrl_up(
                ime.resolve_warmup_ime_on(self.applied_snapshot, std::time::Instant::now()),
            );
        }
    }

    /// Space/Enter/Esc KeyDown の直接 passthrough: warm+TSF または cold の composition 確定処理。
    /// 副作用のみで CallbackResult は返さない。
    fn handle_confirm_key_passthrough(
        &self,
        platform: &mut WindowsPlatform,
        ime: &ImeStateHub,
        raw_event: &RawKeyEvent,
    ) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        // Space/Enter/Escape の直接 passthrough (KeyDown) は composition を
        // 確定・キャンセルしてコンテキストをアイドル状態に戻す。
        // mark_cold / eager warmup / warmup の KeyUp 遅延は CompositionFsm（on_passthrough_key
        // 経由）に委譲する。保留状態は FSM が PendingWarmupOnKeyUp として持つ。
        if is_key_down && raw_event.vk_code.is_composition_confirm_key() {
            platform.on_passthrough_key(
                raw_event.vk_code,
                true,
                ime.resolve_warmup_ime_on(self.applied_snapshot, std::time::Instant::now()),
            );
        }
    }

    // ── 共通 ──

    fn execute_one(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &ImeStateHub,
        effect: Effect,
        generation: Option<crate::state::ApplyGeneration>,
    ) -> Option<ImeApplyCompletion> {
        if let Effect::Input(InputEffect::ReinjectKey(event)) = effect {
            self.handle_reinject(platform, ime, event);
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

    /// F2-TSF 特殊扱い + 通常 reinject + confirm キー後処理。
    fn handle_reinject(
        &self,
        platform: &mut WindowsPlatform,
        ime: &ImeStateHub,
        event: RawKeyEvent,
    ) {
        let is_key_down = matches!(event.event_type, awase::types::KeyEventType::KeyDown);
        let dir = if is_key_down { "down" } else { "up" };

        // F2 (VK_DBE_HIRAGANA) in TSF mode: deferred F2 も reinject しない。
        // pending 中に F2 が来た場合も ReinjectKey としてキューに入るが、
        // TSF モードでは物理 F2 を WezTerm に届けないことで double-F2 を防ぐ。
        if event.vk_code == crate::vk::VK_DBE_HIRAGANA && platform.is_tsf_mode() {
            if is_key_down {
                // mark_cold(NativeF2Consumed) + eager warmup を platform に委譲する。
                platform.on_reinject_key(
                    event.vk_code,
                    true,
                    ime.resolve_warmup_ime_on(self.applied_snapshot, std::time::Instant::now()),
                );
            } else {
                log::debug!(
                    "[reinject-tsf] vk=0xf2 KeyUp TSF mode → consuming (paired KeyDown was consumed)",
                );
            }
            return;
        }

        log::debug!(
            "[reinject] vk={:#04x} {dir} (queued passthrough now firing)",
            event.vk_code,
        );

        // 案 2a: Space/Enter/Escape (confirm key) KeyDown の composition 後処理を spawn 前に実行する。
        // OUTPUT_GATE.active=true 中は新たなキーが INPUT_DEFER に退避されるため、
        // on_reinject_key を reinject() の前後どちらで呼んでも観測可能な差がない。
        // これにより spawn_local 内の with_app 呼び出しを除去できる。
        if is_key_down && event.vk_code.is_composition_confirm_key() {
            platform.on_reinject_key(
                event.vk_code,
                true,
                ime.resolve_warmup_ime_on(self.applied_snapshot, std::time::Instant::now()),
            );
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
        ime: &ImeStateHub,
        effect: Effect,
        generation: Option<crate::state::ApplyGeneration>,
    ) -> Option<(bool, awase::platform::ImeOpenOutcome)> {
        // ImeEffect::SetOpen は ImmCross-first か否かで async / sync を分岐するため
        // 先に処理する（後段の `let platform_rt = platform` が `platform`
        // を独占する前に `build_ime_control_view` を呼ぶ必要がある）。
        if let Effect::Ime(ImeEffect::SetOpen { open, .. }) = effect {
            return self.dispatch_ime_set_open(platform, ime, open, generation);
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
        // send_engine_state_ime_key に渡す applied 値をトレイトオブジェクト取得前に確定する。
        let applied_for_engine_key = self.applied_snapshot.applied_open();
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
                UiEffect::EngineStateChanged {
                    enabled,
                    send_ime_key,
                } => {
                    platform_rt.update_tray(enabled);
                    if send_ime_key {
                        platform_rt.send_engine_state_ime_key(enabled, applied_for_engine_key);
                    }
                    None
                }
            },
        }
    }

    /// `ImeEffect::SetOpen` の専用 dispatch。
    ///
    /// `ImmCrossProcessStrategy` が現在のコンテキストで最初に適用可能な場合は
    /// `win32_async::spawn_local` で非同期実行し `None` を返す（spawn 済み）。
    /// それ以外（GjiDirect / KanjiToggle 経路）はキー注入のみで非ブロッキングなため
    /// 既存の同期 chain を維持し、`Some(..)` を返す。
    fn dispatch_ime_set_open(
        &mut self,
        platform: &WindowsPlatform,
        ime: &ImeStateHub,
        open: bool,
        generation: Option<crate::state::ApplyGeneration>,
    ) -> Option<(bool, awase::platform::ImeOpenOutcome)> {
        // view は imm_first 判定と sync path の両方で使うため一度だけ構築する。
        let mut view = platform.build_ime_control_view(self.applied_snapshot.to_pair());
        view.belief_input_mode = self.belief_input_mode;
        if view.focus.profile == crate::focus::class_names::AppImeProfile::InputRelay {
            return Some((open, awase::platform::ImeOpenOutcome::NotOwned));
        }
        let imm_first = crate::ime_controller::ImeController::imm_cross_is_first_applicable(&view);
        if imm_first {
            // ── async path (ImmCross が選ばれるアプリ) ──
            // OutputActiveGuard を先に取得しておくことで、await 中に走るフックコールバックは
            // INPUT_DEFER へ退避され、SetOpen 進行中に新キーが engine に届かない。
            //
            // 同一エフェクトバッチ内で直後に処理される UiEffect::EngineStateChanged →
            // send_engine_state_ime_key が applied_snapshot を見て VK_F4/VK_F3 を
            // 送信するかを決める。async 完了前は applied_snapshot が旧値のままなので
            // 「不整合あり→モードキー送信」と判断されてしまう。
            // LINE/Qt 等の ImmCross アプリはこの VK_F4 Up に対して VK_F3 Down を
            // 生成し（extra=0x0、マーカーなし）、shadow toggle が ON→OFF に反転する。
            // → 楽観的に applied_snapshot を更新して send_engine_state_ime_key をスキップさせる。
            self.applied_snapshot = crate::state::AppliedImeState::Optimistic(open);
            // IMM が set_ime_open_cross_process(open) 完了後に注入する VK_DBE_DBCSCHAR/
            // VK_DBE_SBCSCHAR KeyUp は key_pipeline の suppress_physical (ImmCross プロファイル
            // の KANJI VK 全 Consume) で構造的に遮断されるため、ここでは applied_snapshot 更新のみ。
            log::debug!(
                "[dispatch-ime] ImmCross async: optimistic applied_snapshot={open} \
                 (suppress send_engine_state_ime_key)"
            );
            // ImmCross の set_ime_open_cross_process は IMC_SETOPENSTATUS のみ設定し
            // conv mode は変更しない。IME がかなモード (conv=0x09) のまま ON になると
            // NICOLA エンジンが is_romaji_capable=false で起動できない。
            // MsImeDirectStrategy と同じく ObservedKana 以外なら ROMAN ビットを補完する。
            // ImmCross アプリは ir_poll_and_learn で ObservedKana の観測を抑制するため
            // belief は ObservedKana にならず、ここに到達したときは常に補完対象になる。
            let belief_input_mode = self.belief_input_mode;
            // ADR-090 §2.A A-1（shadow）: 起案は spawn_local の**外**で行う
            // ——future の中では `with_app` 再入で `ImeStateHub` に届かない
            // （ADR-090 §4.2）。
            let order = issue_order(ime, open, "engine_decision_async");
            let guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
            // ADR-086 §1.2 欠陥1 是正（opus レビュー指摘 2026-08-08）: 「open と
            // 同じウィンドウへ ROMAN ビットを補完する」という意図を、open/conv を
            // 別々に検証していたのでは保証できない（open 完了を待つ間にフォーカスが
            // 動いても ime_mode_focus_gen の更新が遅れるため、conv 側の再検証だけでは
            // 検知できず無関係な別ウィンドウへ ROMAN が着弾しうる）。起案時点の
            // focus_gen を捕獲し、実際の verify → open → conv はすべて
            // set_ime_open_then_conv_for_target 1回に閉じ込めて同一 hwnd を使い回す。
            let focus_gen = platform.output.ime_mode_focus_gen.get();
            let conv_after_open =
                if open && !matches!(belief_input_mode, InputModeState::ObservedKana) {
                    crate::ime::ConvAfterOpen::Write(None)
                } else {
                    crate::ime::ConvAfterOpen::Skip
                };
            win32_async::spawn_local(async move {
                let Some(target) = crate::ime::ActuationTarget::capture(focus_gen).await else {
                    log::debug!("[dispatch-ime] capture 失敗（フォーカス無し） → UnsafeToToggle");
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
            //
            // 観測値は冒頭で構築済みの view から読む（tsf_obs() の二重呼び出し回避）。
            //
            // 【doc 訂正、2026-08-10、ADR-087 §5 Phase 3 item14 棚卸しで判明】
            // 元々このコメントは「EngineIntent かつ ImmCross/GJI で確認できない環境では
            // confident=false → already_matched=false → 必ず apply する」という設計意図
            // だったが、`belief.confident` を読んで already_matched 判定に使う本番コードは
            // 現在存在しない（`already_matched`/`AlreadyMatched` は
            // `ime_controller::ImeController::apply` が `view.control.shadow_on` から独立に
            // 判定する）。`belief.confident` の本番消費者は診断ログ
            // （`platform.rs::apply_ime_open_with_view` の `log::debug!`）のみ。
            let now_ms = crate::hook::current_tick_ms();

            // BUG-34 横展開 B: 以前はここで MS-IME + TsfNative の場合のみ
            // get_ime_conversion_mode_raw_timeout(5) を同期的に呼んでいた
            // （SendMessageTimeoutW ベース、SMTO_ABORTIFHUNG は呼び出し中に
            // ハングし始めた相手には効かず最大 HungAppTimeout ~5s ブロックしうる、
            // known-bugs.md BUG-34）。この打鍵経路は毎打鍵で走るため、ブロックすると
            // その打鍵の IME open/close 判定そのものが Win32 往復の後ろに回る。
            //
            // この read の唯一の消費先は belief_inputs.conv_mode →
            // reduce_open_belief → belief.effective_open/confident だが、
            // apply_ime_open_with_view (platform.rs) は belief を log::debug! に
            // 渡すだけで、実行本体 ImeController::apply(order, view) は belief
            // 引数を受け取っていない（読んだ値は最終的にログ2行にしか影響しない）。
            // そのため fence や degrade 方針を設計する必要はなく、単純に conv_mode
            // を常に None にして同期 read を削除するだけで安全に打鍵経路の
            // ブロックを解消できる。
            //
            // BUG-34 横展開レビュー指摘: 当初はここに fire-and-forget の診断読み取り
            // （spawn_local + offload、結果は log のみ）を残していたが、この経路は
            // 「毎打鍵で走る」（本関数冒頭のコメント参照）ため、キー入力のたびに
            // OS スレッドを spawn する（`win32_async::offload` は呼び出しごとに
            // `std::thread::spawn`）・宣言タイムアウトを 5ms→50ms に引き上げた
            // クロスプロセス `SendMessageTimeoutW` を送る・その結果が
            // `send_health::record` に給餌されグローバルなサーキットブレーカを
            // 誤って作動させうる、という副作用があった。BUG-34 の第1修正
            // （`kp_stage_idle_conv_check`）がまさにこの積み上がりを防ぐために
            // in-flight ガードを持つのに対し、この診断はその保護を持たない
            // 一回性イベント向けの idiom（shift-conv-guard entry verify）を
            // 毎打鍵経路へ転用したものだった。唯一の消費先がログだけである以上、
            // 削除するのが最も一貫した選択（fence/ガードを新設する価値がない）。
            let conv_mode = None;

            let belief_inputs = crate::output::OpenBeliefInputs {
                // `OpenBeliefInputs.shadow_on` は診断ログ専用の消費先（上記コメント
                // 参照）で、BUG-113 修正の対象外。`ControlLog.shadow_on` が
                // `Option<bool>` 化された後もこちらの既存挙動（未知は false
                // 扱い）を変えない。
                shadow_on: view.control.shadow_on.unwrap_or(false),
                applied: self.applied_snapshot,
                candidate_visible: view.observed.candidate_visible,
                candidate_was_seen: view.observed.candidate_was_seen,
                gji_monitor_ok: view.observed.gji_monitor_ok,
                conv_mode,
                can_imm32_cross_process: view.focus.profile.can_use_imm32_cross_process(),
                now_ms,
            };
            let belief = crate::output::reduce_open_belief(&belief_inputs, open);
            log::debug!(
                "[dispatch-ime] belief: effective={} confident={} conv={:?} (profile={:?})",
                belief.effective_open,
                belief.confident,
                conv_mode,
                view.focus.profile
            );
            // ADR-090 §2.A A-1（shadow）。
            let order = issue_order(ime, open, "engine_decision_sync");
            let outcome = platform.apply_ime_open_with_view(order, &view, belief);
            if outcome == awase::platform::ImeOpenOutcome::Failed {
                log::warn!("apply_ime_open({open}) failed");
            }
            Some((open, outcome))
        }
    }

    /// intra-batch の applied_snapshot のみを更新する。
    ///
    /// sync SetOpen 直後に同一バッチ内の後続 effect（`send_engine_state_ime_key` 等）が
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
            ImeOpenOutcome::UnsafeToToggle | ImeOpenOutcome::NotOwned
        ) {
            return;
        }
        let effective = match outcome {
            ImeOpenOutcome::Applied
            | ImeOpenOutcome::FallbackSent
            | ImeOpenOutcome::AlreadyMatched => open,
            ImeOpenOutcome::Failed => !open,
            ImeOpenOutcome::UnsafeToToggle | ImeOpenOutcome::NotOwned => unreachable!(),
        };
        self.applied_snapshot = crate::state::AppliedImeState::Confirmed {
            open: effective,
            at_ms: crate::hook::current_tick_ms(),
        };
    }
}

/// `reduce_open_belief` および `AppliedImeState` の unit tests。
///
/// `awase-windows` クレートは `#![cfg(windows)]` で囲まれているため
/// Windows 実機でのみ実行される。
#[cfg(test)]
mod tests {
    use crate::output::{reduce_open_belief, OpenBeliefInputs};
    use crate::state::AppliedImeState;

    /// Chrome 相当の設定（can_imm32=false, gji=false, EngineIntent）で confident を返すヘルパー。
    /// `kanji_needs_context_override(...)` == `!chrome_intent(...).confident`
    fn chrome_intent_confident(
        desired: bool,
        applied: AppliedImeState,
        shadow_on: bool,
        now_ms: u64,
    ) -> bool {
        let inputs = OpenBeliefInputs {
            shadow_on,
            applied,
            candidate_visible: false,
            candidate_was_seen: false,
            gji_monitor_ok: false,
            conv_mode: None,
            can_imm32_cross_process: false,
            now_ms,
        };
        reduce_open_belief(&inputs, desired).confident
    }

    // 6-C ケース 1: フォーカス直後 (Unknown) → confident=false（必ず apply）
    #[test]
    fn not_confident_when_unknown() {
        assert!(!chrome_intent_confident(
            false,
            AppliedImeState::Unknown,
            false,
            1000
        ));
    }

    // 6-C ケース 2: Optimistic のみ → confident=false
    #[test]
    fn not_confident_when_optimistic_only() {
        assert!(!chrome_intent_confident(
            false,
            AppliedImeState::Optimistic(false),
            false,
            1000
        ));
    }

    // ケース 3a: Confirmed OFF + 目標 OFF + 300ms 以内 → confident（二重送信防止）
    #[test]
    fn confident_when_confirmed_off_within_300ms() {
        assert!(chrome_intent_confident(
            false,
            AppliedImeState::Confirmed {
                open: false,
                at_ms: 900
            },
            false,
            1000
        ));
    }

    // ケース 3b: Confirmed OFF + 目標 OFF + 300ms 超過 → not confident（desync 修正のため再送）
    #[test]
    fn not_confident_when_confirmed_off_over_300ms() {
        assert!(!chrome_intent_confident(
            false,
            AppliedImeState::Confirmed {
                open: false,
                at_ms: 500
            },
            false,
            1000
        ));
    }

    // ケース 4: Confirmed + 目標 ON + 300ms 以内 → confident（二重送信防止）
    #[test]
    fn confident_when_confirmed_within_300ms() {
        assert!(chrome_intent_confident(
            true,
            AppliedImeState::Confirmed {
                open: false,
                at_ms: 800
            },
            true,
            1000
        ));
    }

    // ケース 5: Confirmed + 300ms 超過 → not confident（再試行許容）
    #[test]
    fn not_confident_when_confirmed_over_300ms() {
        assert!(!chrome_intent_confident(
            true,
            AppliedImeState::Confirmed {
                open: false,
                at_ms: 500
            },
            true,
            1000
        ));
    }

    // ケース 6: IMM32 使用可 → confident（ImmCross が先行するのでここには来ないが念のため）
    #[test]
    fn confident_when_imm32_available() {
        let inputs = OpenBeliefInputs {
            shadow_on: false,
            applied: AppliedImeState::Unknown,
            candidate_visible: false,
            candidate_was_seen: false,
            gji_monitor_ok: false,
            conv_mode: None,
            can_imm32_cross_process: true,
            now_ms: 1000,
        };
        assert!(reduce_open_belief(&inputs, false).confident);
    }

    // ケース 7: GJI 健全 → confident
    #[test]
    fn confident_when_gji_healthy() {
        let inputs = OpenBeliefInputs {
            shadow_on: false,
            applied: AppliedImeState::Unknown,
            candidate_visible: false,
            candidate_was_seen: false,
            gji_monitor_ok: true,
            conv_mode: None,
            can_imm32_cross_process: false,
            now_ms: 1000,
        };
        assert!(reduce_open_belief(&inputs, false).confident);
    }

    // （旧ケース 8「EngineIntent でない → confident」は 2026-07-06 到達不能パス監査
    // B6 で撤去 — SetOpen は常に Engine の意図であり is_engine_intent 区別ごと畳んだ。）

    // ケース 9: Confirmed ON + 目標 ON + 300ms 以内 → confident。
    // 300ms超過後は他ケース同様not confidentになる(7a24442でOFF方向の「永続
    // スキップ」を廃止した設計と一貫させるため、confidentは時間無制限の
    // 「永続」ではなく300msの再検証ウィンドウという設計)。旧now_ms=100_000/
    // at_ms=500(elapsed=99,500ms)はテスト新設(f7f09bc, 2026-06-04)時点で
    // 既に300ms窓の外にあり、Windows実機で初めてこのテストを実行するまで
    // (2026-07-25)発見されなかった。
    #[test]
    fn confident_when_confirmed_on_desired_on() {
        assert!(chrome_intent_confident(
            true,
            AppliedImeState::Confirmed {
                open: true,
                at_ms: 900
            },
            true,
            1000
        ));
    }

    // AppliedImeState ヘルパーメソッドのテスト
    #[test]
    fn applied_ime_state_to_pair() {
        assert_eq!(AppliedImeState::Unknown.to_pair(), None);
        assert_eq!(AppliedImeState::Optimistic(true).to_pair(), Some((true, 0)));
        assert_eq!(
            AppliedImeState::Confirmed {
                open: false,
                at_ms: 42
            }
            .to_pair(),
            Some((false, 42))
        );
    }

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

    // ── strip_ime_set_open_if_settling (P3-1: focus-settle SetOpen 一次フィルタ) ──

    use awase::engine::{Decision, Effect, ImeEffect, SetOpenOrigin, TimerEffect};

    fn set_open_effect(open: bool) -> Effect {
        Effect::Ime(ImeEffect::SetOpen {
            open,
            origin: SetOpenOrigin::ExplicitUserAction,
        })
    }

    // settling=true: SetOpen effect は decision から除去され、除去された目標値が返る。
    #[test]
    fn strip_removes_set_open_when_settling() {
        let mut decision = Decision::consumed_with(vec![set_open_effect(true)].into());
        let stripped = super::strip_ime_set_open_if_settling(&mut decision, true);
        assert!(
            decision.find_ime_set_open().is_none(),
            "settle 中は SetOpen effect が除去される"
        );
        assert_eq!(
            stripped,
            Some(true),
            "呼び出し元が settle 明けの再試行をスケジュールできるよう、\
             除去した目標値を返す必要がある"
        );
    }

    // settling=false: SetOpen effect はそのまま保持され、何も除去していないので None が返る。
    #[test]
    fn strip_keeps_set_open_when_not_settling() {
        let mut decision = Decision::consumed_with(vec![set_open_effect(true)].into());
        let stripped = super::strip_ime_set_open_if_settling(&mut decision, false);
        assert_eq!(
            decision.find_ime_set_open(),
            Some(true),
            "settle 外では SetOpen effect は保持される"
        );
        assert_eq!(stripped, None, "何も除去していないので None");
    }

    // settling=true だが SetOpen effect が無い場合: 除去対象が無いので None
    // （retry のスケジュールも不要 — 呼び出し元は Some のときだけ再試行すればよい）。
    #[test]
    fn strip_returns_none_when_settling_but_no_set_open_effect() {
        let mut decision =
            Decision::consumed_with(vec![Effect::Timer(TimerEffect::Kill(0))].into());
        let stripped = super::strip_ime_set_open_if_settling(&mut decision, true);
        assert_eq!(
            stripped, None,
            "SetOpen effect が無ければ settling=true でも None"
        );
        let remaining = match &decision {
            Decision::Consume { effects } => effects.len(),
            _ => unreachable!("Consume のまま"),
        };
        assert_eq!(remaining, 1, "SetOpen 以外の effect は手つかずのまま残る");
    }

    // settling=true でも SetOpen 以外の effect（Timer 等）は保持される（対象集合を SetOpen に限定）。
    #[test]
    fn strip_preserves_non_set_open_effects_when_settling() {
        let mut decision = Decision::consumed_with(
            vec![set_open_effect(false), Effect::Timer(TimerEffect::Kill(0))].into(),
        );
        let stripped = super::strip_ime_set_open_if_settling(&mut decision, true);
        assert!(
            decision.find_ime_set_open().is_none(),
            "SetOpen は除去される"
        );
        assert_eq!(stripped, Some(false), "除去した目標値 false が返る");
        let remaining = match &decision {
            Decision::Consume { effects } => effects.len(),
            _ => unreachable!("Consume のまま"),
        };
        assert_eq!(remaining, 1, "SetOpen 以外の effect（Timer）は残る");
    }

    // 2026-07-08: 実機で GjiFsm が resync できず「このせっけい」の文字欠落に至った
    // シナリオの再発防止。settle 中に握りつぶした SetOpen(true) の戻り値を呼び出し元
    // （execute_decision / kp_run_inner）が無視すると、Engine::prev_activation は既に
    // 遷移確定済みのため、同じ SetOpen は二度と自然発行されない。
    // この関数の契約（Some を返したら呼び出し元は必ず settle 明け再試行をスケジュールする）
    // を型レベルで思い出させるため #[must_use] を付けている。ここでは戻り値の意味そのもの
    // （「再試行が必要かどうか」の判定に使えること）を固定する。
    #[test]
    fn strip_stripped_value_signals_retry_is_owed() {
        let mut decision = Decision::consumed_with(vec![set_open_effect(true)].into());
        let stripped = super::strip_ime_set_open_if_settling(&mut decision, true);
        // 呼び出し元の実装（execute_decision / kp_run_inner）はこの `is_some()` で
        // schedule_ime_refresh(focus_settle_ms + 50) を呼ぶかどうかを判断する。
        assert!(
            stripped.is_some(),
            "SetOpen を握りつぶしたら再試行が必要という事実を呼び出し元へ伝える"
        );
    }
}
