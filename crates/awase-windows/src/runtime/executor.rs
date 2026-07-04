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

use awase::config::HookMode;
use awase::engine::{Decision, DecisionOrigin, Effect, ImeEffect, InputEffect, InputModeState, TimerEffect, UiEffect};
use awase::platform::{EffectOrigin, PlatformRuntime};
use awase::types::RawKeyEvent;

use crate::hook::CallbackResult;
use crate::platform::WindowsPlatform;
use crate::state::ConvModeAuthority;
use crate::runtime::{PassthroughQueue, PhysicalKeyDisposition};
use crate::state::platform_state::ImeStateHub;
use crate::vk::VkCodeExt;
use crate::RawKeyEventExt as _;


/// IME apply の sync 完了 1 件分。`(open: bool, outcome: ImeOpenOutcome)` のエイリアス。
pub(crate) type ImeApplyPair = (bool, awase::platform::ImeOpenOutcome);

/// `execute_from_hook` の戻り値。
#[derive(Debug)]
pub(crate) struct BatchResult {
    /// OS に返す consume/passthrough 判定
    pub callback: CallbackResult,
    /// true なら `PostMessage(WM_EXECUTE_EFFECTS)` でメッセージループに通知が必要
    pub has_pending: bool,
    /// sync path の SetOpen 完了リスト。
    /// async path は spawn_local 内で on_ime_apply_complete を直接呼ぶため含まない。
    pub sync_outcomes: Vec<ImeApplyPair>,
}

pub(crate) struct DecisionExecutor {
    /// Effects キュー（FIFO 順序保証）
    queue: VecDeque<Effect>,
    /// フックの動作モード
    hook_mode: HookMode,
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
    pub(crate) fn new(hook_mode: HookMode) -> Self {
        Self {
            queue: VecDeque::new(),
            hook_mode,
            passthrough_queue: PassthroughQueue::new(),
            guard_held: None,
            applied_snapshot: crate::state::AppliedImeState::Unknown,
            belief_input_mode: InputModeState::Unknown,
        }
    }

    /// フックコールバックから呼ぶ。
    ///
    /// - Filter モード: 入出力系は即座実行、重い処理は遅延。PassThrough を OS に返す。
    /// - Relay モード: 全 Effects をキューに入れ、PassThrough キーも ReinjectKey に変換。
    ///   常に Consumed を返す。
    pub(crate) fn execute_from_hook(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &ImeStateHub,
        decision: Decision,
        raw_event: &RawKeyEvent,
        physical: PhysicalKeyDisposition,
    ) -> BatchResult {
        self.applied_snapshot = ime.model().applied;
        match self.hook_mode {
            HookMode::Filter => self.execute_filter(platform, decision, physical),
            HookMode::Relay => self.execute_relay(platform, decision, raw_event, physical),
        }
    }

    /// メッセージループから呼ぶ。全 Effects を即座に実行する。
    pub(crate) fn execute_from_loop(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &ImeStateHub,
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
            if let Some(o) = self.execute_one(platform, effect) {
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
    #[expect(clippy::useless_let_if_seq)]
    pub(crate) fn drain_deferred(&mut self, platform: &mut WindowsPlatform) -> Vec<ImeApplyPair> {
        // 同一 drain 呼び出し内で最初の ReinjectKey だけ OUTPUT_GUARD を適用する。
        // 連続する reinject (例: Win_DOWN→X_DOWN→X_UP→Win_UP) を個別にガードすると
        // Win が 150ms 以上 OS 側でスタックし、後続のショートカットが Win+key と
        // 誤解釈されるため、先頭の reinject が guard を通過したら残りはまとめて送出する。
        let mut sync_outcomes = Vec::new();
        let mut reinject_guard_passed = false;

        // 1) 前回 park した ReinjectKey があれば最初に試す。
        //    guard 解除済みなら execute_one してから queue に進む (batching を継続)。
        if let Some(event) = self.guard_held.take() {
            if let Some(remaining) = self.output_guard_remaining(platform) {
                log::debug!(
                    "[reinject-guard] held event, output {}ms ago, suspending for {remaining}ms",
                    crate::tuning::OUTPUT_GUARD_MS - remaining,
                );
                self.park_in_guard(platform, event, remaining);
                return sync_outcomes;
            }
            let effect = Effect::Input(InputEffect::ReinjectKey(event));
            if let Some(o) = self.execute_one(platform, effect) {
                sync_outcomes.push(o);
            }
            reinject_guard_passed = true;
        }

        // 2) queue を FIFO で drain。
        while let Some(effect) = self.queue.pop_front() {
            let is_reinject = matches!(effect, Effect::Input(InputEffect::ReinjectKey(_)));
            if is_reinject && !reinject_guard_passed {
                if let Some(remaining) = self.output_guard_remaining(platform) {
                    log::debug!(
                        "[reinject-guard] output {}ms ago, suspending drain for {remaining}ms",
                        crate::tuning::OUTPUT_GUARD_MS - remaining,
                    );
                    let Effect::Input(InputEffect::ReinjectKey(event)) = effect else {
                        unreachable!("is_reinject was true")
                    };
                    self.park_in_guard(platform, event, remaining);
                    return sync_outcomes;
                }
                reinject_guard_passed = true;
            } else if !is_reinject {
                // NICOLA 出力など reinject 以外の effect は mark_send を呼ぶので
                // 次の reinject には再びガードを適用する。
                reinject_guard_passed = false;
            }
            if let Some(o) = self.execute_one(platform, effect) {
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
    ) -> Vec<ImeApplyPair> {
        platform.timer.kill(crate::TIMER_OUTPUT_GUARD);
        self.drain_deferred(platform)
    }

    /// queue または guard slot に Effect が残っているか
    pub(crate) fn has_pending(&self) -> bool {
        !self.queue.is_empty() || self.guard_held.is_some()
    }

    /// output guard 期間中なら残り ms を返す。期間外なら None。
    #[expect(clippy::unused_self)]
    fn output_guard_remaining(&self, platform: &WindowsPlatform) -> Option<u64> {
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
    /// 通常 hook 経路では PassThrough は `CallNextHookEx` で OS に直接届く。
    /// しかし OUTPUT_GATE active 期間や with_app 再入セーフネットで `INPUT_DEFER` へ
    /// Consumed として退避されたキーは drain で engine に replay されたあと
    /// `CallbackResult::PassThrough` が返っても hook 経路に戻らないため、
    /// 明示的に SendInput で送出する必要がある。
    pub(crate) fn enqueue_reinject(&mut self, event: RawKeyEvent) {
        self.queue
            .push_back(Effect::Input(InputEffect::ReinjectKey(event)));
    }

    // ── Filter モード ──

    fn execute_filter(
        &mut self,
        platform: &mut WindowsPlatform,
        decision: Decision,
        physical: PhysicalKeyDisposition,
    ) -> BatchResult {
        let (callback, effects) = match decision {
            Decision::PassThrough => {
                return BatchResult {
                    callback: physical.to_callback(false),
                    has_pending: self.has_pending(),
                    sync_outcomes: Vec::new(),
                }
            }
            Decision::PassThroughWith { effects } => (physical.to_callback(false), effects),
            Decision::Consume { effects } => (physical.to_callback(true), effects),
        };

        let mut sync_outcomes = Vec::new();
        for effect in effects {
            if Self::is_input_critical(&effect) {
                // Ime/Ui effects are not critical → they go to queue, never reach execute_one here.
                if let Some(o) = self.execute_one(platform, effect) {
                    sync_outcomes.push(o);
                }
            } else {
                self.queue.push_back(effect);
            }
        }

        BatchResult {
            callback,
            has_pending: self.has_pending(),
            sync_outcomes,
        }
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
                let callback = self.run_passthrough_pipeline(platform, raw_event);
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
                    if reinject { " + reinject" } else { " (no reinject, suppressed)" },
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
                        if let Some(o) = self.execute_one(platform, effect) {
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
        raw_event: &RawKeyEvent,
    ) -> CallbackResult {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);

        // A. [transport] KeyUp 対称性
        if let Some(event) = self.passthrough_queue.check_keyup_symmetry(raw_event) {
            self.enqueue_reinject(event);
            return CallbackResult::Consumed;
        }

        // B. [platform] 副作用（defer されても FSM は進める）
        self.try_pending_warmup_on_keyup(platform, raw_event);
        self.handle_ctrl_up_recovery(platform, raw_event);

        // C. [transport] output guard defer
        let in_flight_ms = platform.output_in_flight_ms();
        let output_in_flight = in_flight_ms < crate::tuning::OUTPUT_GUARD_MS;
        let has_pending = self.has_pending();
        log::debug!(
            "[relay-guard] vk={:#04x} {} in_flight_ms={} has_pending={} output_in_flight={}",
            raw_event.vk_code,
            if is_key_down { "down" } else { "up" },
            if in_flight_ms == u64::MAX { "never".to_string() } else { in_flight_ms.to_string() },
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
        self.handle_confirm_key_passthrough(platform, raw_event);

        if matches!(raw_event.key_classification, awase::types::KeyClassification::Passthrough) {
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
    fn try_pending_warmup_on_keyup(&self, platform: &mut WindowsPlatform, raw_event: &RawKeyEvent) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if !is_key_down && raw_event.vk_code.is_composition_confirm_key() {
            platform
                .composition_confirm_key_up(raw_event.vk_code, self.applied_snapshot.applied_open());
        }
    }

    /// Ctrl↑: cold 状態であれば eager_warmup_sent_ms をリセット（この→kおの バグ対策）。
    /// Ctrl が WezTerm に届いている間、GJI TSF 初期化が中断される可能性がある。
    /// Ctrl↑ を起点としてタイマーを再計測し GJI recovery 時間（500ms）を確保する。
    /// cold 判定・warmup 送信は `CompositionFsm`（CtrlUp）に委譲する。副作用のみ。
    fn handle_ctrl_up_recovery(&self, platform: &mut WindowsPlatform, raw_event: &RawKeyEvent) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if !is_key_down && raw_event.vk_code.is_ctrl_variant() {
            platform.composition_ctrl_up(self.applied_snapshot.applied_open());
        }
    }

    /// Space/Enter/Esc KeyDown の直接 passthrough: warm+TSF または cold の composition 確定処理。
    /// 副作用のみで CallbackResult は返さない。
    fn handle_confirm_key_passthrough(
        &self,
        platform: &mut WindowsPlatform,
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
                self.applied_snapshot.applied_open(),
            );
        }
    }

    // ── 共通 ──

    const fn is_input_critical(effect: &Effect) -> bool {
        matches!(effect, Effect::Input(_) | Effect::Timer(_))
    }

    fn execute_one(
        &mut self,
        platform: &mut WindowsPlatform,
        effect: Effect,
    ) -> Option<(bool, awase::platform::ImeOpenOutcome)> {
        if let Effect::Input(InputEffect::ReinjectKey(event)) = effect {
            self.handle_reinject(platform, event);
            return None;
        }
        self.dispatch_effect(platform, effect)
            .map(|(open, outcome)| {
                self.update_intra_batch_applied(open, outcome);
                (open, outcome)
            })
    }

    /// F2-TSF 特殊扱い + 通常 reinject + confirm キー後処理。
    fn handle_reinject(&self, platform: &mut WindowsPlatform, event: RawKeyEvent) {
        let is_key_down = matches!(event.event_type, awase::types::KeyEventType::KeyDown);
        let dir = if is_key_down { "down" } else { "up" };

        // F2 (VK_DBE_HIRAGANA) in TSF mode: deferred F2 も reinject しない。
        // pending 中に F2 が来た場合も ReinjectKey としてキューに入るが、
        // TSF モードでは物理 F2 を WezTerm に届けないことで double-F2 を防ぐ。
        if event.vk_code == crate::vk::VK_DBE_HIRAGANA && platform.is_tsf_mode() {
            if is_key_down {
                // mark_cold(NativeF2Consumed) + eager warmup を platform に委譲する。
                platform.on_reinject_key(event.vk_code, true, self.applied_snapshot.applied_open());
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
            platform.on_reinject_key(event.vk_code, true, self.applied_snapshot.applied_open());
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
        effect: Effect,
    ) -> Option<(bool, awase::platform::ImeOpenOutcome)> {
        // ImeEffect::SetOpen は ImmCross-first か否かで async / sync を分岐するため
        // 先に処理する（後段の `let platform_rt = platform` が `platform`
        // を独占する前に `build_ime_control_view` を呼ぶ必要がある）。
        if let Effect::Ime(ImeEffect::SetOpen { open, origin }) = effect {
            return self.dispatch_ime_set_open(platform, open, origin);
        }
        // EngineStateChanged: エンジン ON/OFF に連動して ConvModeAuthority を更新する。
        // platform_rt (&mut dyn PlatformRuntime) 変換前に行う必要がある。
        // pending_conv_mode_authority に格納し、runtime が take して ImeStateHub に dispatch する。
        if let Effect::Ui(UiEffect::EngineStateChanged { enabled, .. }) = &effect {
            let authority = if *enabled {
                ConvModeAuthority::AwaseOwned
            } else {
                ConvModeAuthority::UserOwned
            };
            platform.set_conv_mode_authority(authority);
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
                ImeEffect::RequestRefresh => {
                    platform_rt.post_ime_refresh();
                    None
                }
            },
            Effect::Ui(ue) => match ue {
                UiEffect::EngineStateChanged { enabled, send_ime_key } => {
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
        open: bool,
        origin: DecisionOrigin,
    ) -> Option<(bool, awase::platform::ImeOpenOutcome)> {
        // view は imm_first 判定と sync path の両方で使うため一度だけ構築する。
        let mut view = platform.build_ime_control_view(self.applied_snapshot.to_pair());
        view.belief_input_mode = self.belief_input_mode;
        let imm_first = crate::ime_controller::CONTROLLER.imm_cross_is_first_applicable(&view);
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
            let belief_input_mode = self.belief_input_mode;
            let guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
            win32_async::spawn_local(async move {
                let ok = crate::ime::set_ime_open_cross_process_async(open).await;
                if ok && open && !matches!(belief_input_mode, InputModeState::ObservedKana) {
                    let _ = crate::ime::set_ime_romaji_mode_with_target_async(None).await;
                }
                let outcome = if ok {
                    awase::platform::ImeOpenOutcome::Applied
                } else {
                    // SAFETY: `read_ime_state_fast` は Win32 IMM API を呼ぶ。
                    //         spawn_local はメインスレッドのメッセージループで実行される。
                    let actual = unsafe { crate::ime::read_ime_state_fast() }.ime_on;
                    if actual == Some(open) {
                        log::debug!(
                            "[apply-ime] ImmCross failed but actual ime_on={actual:?} \
                             already matches desired={open}, skip fallback"
                        );
                        awase::platform::ImeOpenOutcome::AlreadyMatched
                    } else {
                        log::debug!(
                            "[apply-ime] ImmCross failed (async), trying fallback \
                             (actual ime_on={actual:?})"
                        );
                        crate::with_app(|app| {
                            crate::ime_controller::CONTROLLER
                                .apply_skipping_imm(open, &app.shadow_ime_control_view())
                        })
                        .unwrap_or(awase::platform::ImeOpenOutcome::Failed)
                    }
                };
                let _ = crate::with_app(|app| {
                    if outcome == awase::platform::ImeOpenOutcome::Failed {
                        log::warn!("apply_ime_open({open}) failed (async)");
                    }
                    app.on_async_ime_apply_complete(open, outcome);
                });
                drop(guard);
            });
            None
        } else {
            // ── sync path (Chrome / GJI 経路 / TsfNative 経路) ──
            //
            // 観測値は冒頭で構築済みの view から読む（tsf_obs() の二重呼び出し回避）。
            // EngineIntent かつ ImmCross/GJI で確認できない環境では
            // `confident=false` → `already_matched=false` → 必ず apply する（desync 対策）。
            let is_engine_intent = EffectOrigin::from(origin) == EffectOrigin::EngineIntent;
            let now_ms = crate::hook::current_tick_ms();

            // MS-IME + TsfNative の場合のみ conv_mode を直接読む（ground-truth）。
            // ImmCross 対応アプリはこの branch に来ない。GJI 環境は conv_mode 不要。
            let conv_mode = if view.observed.active_ime_kind
                == crate::tsf::observer::ActiveImeKind::MicrosoftIme
                && !view.focus.profile.can_use_imm32_cross_process()
            {
                // SAFETY: Win32 IMM API。メインスレッド前提。
                unsafe { crate::ime::get_ime_conversion_mode_raw_timeout(5) }
            } else {
                None
            };

            let belief_inputs = crate::output::OpenBeliefInputs {
                shadow_on: view.control.shadow_on,
                applied: self.applied_snapshot,
                candidate_visible: view.observed.candidate_visible,
                candidate_was_seen: view.observed.candidate_was_seen,
                gji_monitor_ok: view.observed.gji_monitor_ok,
                conv_mode,
                can_imm32_cross_process: view.focus.profile.can_use_imm32_cross_process(),
                is_engine_intent,
                now_ms,
            };
            let belief = crate::output::reduce_open_belief(&belief_inputs, open);
            log::debug!(
                "[dispatch-ime] belief: effective={} confident={} conv={:?} \
                 (engine_intent={is_engine_intent} profile={:?})",
                belief.effective_open, belief.confident, conv_mode, view.focus.profile
            );
            let outcome = platform.apply_ime_open_with_view(open, &view, belief);
            if outcome == awase::platform::ImeOpenOutcome::Failed {
                log::warn!("apply_ime_open({open}) failed");
            }
            Some((open, outcome))
        }
    }

    /// intra-batch の applied_snapshot のみを更新する。
    ///
    /// B（`on_ime_applied`）は `Runtime::on_ime_apply_complete` に委譲済み。
    /// UnsafeToToggle は送信していないので更新しない。
    #[expect(clippy::needless_pass_by_ref_mut)]
    pub(crate) fn update_intra_batch_applied(
        &mut self,
        open: bool,
        outcome: awase::platform::ImeOpenOutcome,
    ) {
        use awase::platform::ImeOpenOutcome;
        if outcome == ImeOpenOutcome::UnsafeToToggle {
            return;
        }
        let effective = match outcome {
            ImeOpenOutcome::Applied
            | ImeOpenOutcome::FallbackSent
            | ImeOpenOutcome::AlreadyMatched => open,
            ImeOpenOutcome::Failed => !open,
            ImeOpenOutcome::UnsafeToToggle => unreachable!(),
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
    use crate::output::{OpenBeliefInputs, reduce_open_belief};
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
            is_engine_intent: true,
            now_ms,
        };
        reduce_open_belief(&inputs, desired).confident
    }

    // 6-C ケース 1: フォーカス直後 (Unknown) → confident=false（必ず apply）
    #[test]
    fn not_confident_when_unknown() {
        assert!(!chrome_intent_confident(false, AppliedImeState::Unknown, false, 1000));
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
            AppliedImeState::Confirmed { open: false, at_ms: 900 },
            false,
            1000
        ));
    }

    // ケース 3b: Confirmed OFF + 目標 OFF + 300ms 超過 → not confident（desync 修正のため再送）
    #[test]
    fn not_confident_when_confirmed_off_over_300ms() {
        assert!(!chrome_intent_confident(
            false,
            AppliedImeState::Confirmed { open: false, at_ms: 500 },
            false,
            1000
        ));
    }

    // ケース 4: Confirmed + 目標 ON + 300ms 以内 → confident（二重送信防止）
    #[test]
    fn confident_when_confirmed_within_300ms() {
        assert!(chrome_intent_confident(
            true,
            AppliedImeState::Confirmed { open: false, at_ms: 800 },
            true,
            1000
        ));
    }

    // ケース 5: Confirmed + 300ms 超過 → not confident（再試行許容）
    #[test]
    fn not_confident_when_confirmed_over_300ms() {
        assert!(!chrome_intent_confident(
            true,
            AppliedImeState::Confirmed { open: false, at_ms: 500 },
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
            is_engine_intent: true,
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
            is_engine_intent: true,
            now_ms: 1000,
        };
        assert!(reduce_open_belief(&inputs, false).confident);
    }

    // ケース 8: EngineIntent でない → confident（override 不要）
    #[test]
    fn confident_when_not_engine_intent() {
        let inputs = OpenBeliefInputs {
            shadow_on: false,
            applied: AppliedImeState::Unknown,
            candidate_visible: false,
            candidate_was_seen: false,
            gji_monitor_ok: false,
            conv_mode: None,
            can_imm32_cross_process: false,
            is_engine_intent: false,
            now_ms: 1000,
        };
        assert!(reduce_open_belief(&inputs, false).confident);
    }

    // ケース 9: Confirmed ON + 目標 ON → confident（永続スキップ）
    #[test]
    fn confident_when_confirmed_on_desired_on() {
        assert!(chrome_intent_confident(
            true,
            AppliedImeState::Confirmed {
                open: true,
                at_ms: 500
            },
            true,
            100_000
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
}
