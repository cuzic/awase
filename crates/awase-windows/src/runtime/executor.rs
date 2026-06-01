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
use std::collections::{HashSet, VecDeque};

use awase::config::HookMode;
use awase::engine::{Decision, Effect, ImeEffect, InputEffect, TimerEffect, UiEffect};
use awase::platform::{EffectOrigin, PlatformRuntime};
use awase::types::{RawKeyEvent, VkCode};

use crate::hook::CallbackResult;
use crate::platform::WindowsPlatform;
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
    /// Reinject 経由で送った PassThrough KeyDown の VK 集合。
    /// 対応する KeyUp も reinject に揃えて INJECTED_MARKER 対称性を保つ。
    deferred_passthrough_vks: HashSet<VkCode>,
    /// OUTPUT_GUARD で park した ReinjectKey イベント。
    ///
    /// 不変条件: `guard_held.is_some()` ⟺ `TIMER_OUTPUT_GUARD` が登録済み。
    /// drain は「slot を先に試す → 通過したら queue に進む」の 2 段構え。
    /// queue 本体は常に純粋 FIFO で `push_back` / `pop_front` のみ。
    /// `RawKeyEvent` 型にすることで「ReinjectKey 以外が park される」コンパイルエラーになる。
    guard_held: Option<RawKeyEvent>,
    /// 直近の apply 済み IME 状態スナップショット (value, timestamp_ms)。
    ///
    /// decision サイクル開始時に `ImeModel.applied_open/applied_at_ms` から pre-fetch され、
    /// バッチ内の `SetOpen` 処理後に即時更新される（intra-batch ordering 用）。
    /// `ImeModel` が SSOT; これはバッチ内 communication channel 兼 cross-decision cache。
    applied_snapshot: Option<(bool, u64)>,
    /// warm+TSF の confirm キー KeyDown 後に KeyUp で eager warmup を送信するフラグ。
    ///
    /// `on_passthrough_key` が `true` を返したとき KeyDown 側でセットされ、
    /// `try_pending_warmup_on_keyup` の KeyUp タイミングでクリアして warmup を送信する。
    pending_warmup_on_keyup: bool,
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
            deferred_passthrough_vks: HashSet::new(),
            guard_held: None,
            applied_snapshot: None,
            pending_warmup_on_keyup: false,
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
    ) -> BatchResult {
        self.applied_snapshot = ime.applied_pair();
        match self.hook_mode {
            HookMode::Filter => self.execute_filter(platform, decision),
            HookMode::Relay => self.execute_relay(platform, decision, raw_event),
        }
    }

    /// メッセージループから呼ぶ。全 Effects を即座に実行する。
    pub(crate) fn execute_from_loop(
        &mut self,
        platform: &mut WindowsPlatform,
        ime: &ImeStateHub,
        decision: Decision,
    ) -> (CallbackResult, Vec<ImeApplyPair>) {
        self.applied_snapshot = ime.applied_pair();
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
    #[allow(clippy::useless_let_if_seq)]
    pub(crate) fn drain_deferred(
        &mut self,
        platform: &mut WindowsPlatform,
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
    #[allow(clippy::unused_self)]
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
    fn park_in_guard(&mut self, platform: &mut WindowsPlatform, event: RawKeyEvent, remaining: u64) {
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
    ) -> BatchResult {
        let (consumed, effects) = match decision {
            Decision::PassThrough => {
                return BatchResult {
                    callback: CallbackResult::PassThrough,
                    has_pending: self.has_pending(),
                    sync_outcomes: Vec::new(),
                }
            }
            Decision::PassThroughWith { effects } => (false, effects),
            Decision::Consume { effects } => (true, effects),
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
            callback: if consumed {
                CallbackResult::Consumed
            } else {
                CallbackResult::PassThrough
            },
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
    ) -> BatchResult {
        match decision {
            Decision::PassThrough => {
                let callback = self.handle_passthrough(platform, raw_event);
                BatchResult {
                    has_pending: self.has_pending(),
                    callback,
                    sync_outcomes: Vec::new(),
                }
            }
            Decision::PassThroughWith { mut effects } => {
                // flush 出力あり → Consume して flush + キー再注入を FIFO でキュー
                log::debug!(
                    "[relay-flush] PassThroughWith: queue {} effect(s) + reinject(vk={:#04x} {})",
                    effects.len(),
                    raw_event.vk_code,
                    match raw_event.event_type {
                        awase::types::KeyEventType::KeyDown => "down",
                        awase::types::KeyEventType::KeyUp => "up",
                    },
                );
                effects.push(Effect::Input(InputEffect::ReinjectKey(*raw_event)));
                self.queue.extend(effects);
                BatchResult {
                    callback: CallbackResult::Consumed,
                    has_pending: true,
                    sync_outcomes: Vec::new(),
                }
            }
            Decision::Consume { effects } => {
                // Engine が消費 → Effects をキューに入れる
                self.queue.extend(effects);
                BatchResult {
                    callback: CallbackResult::Consumed,
                    has_pending: self.has_pending(),
                    sync_outcomes: Vec::new(),
                }
            }
        }
    }

    // ── PassThrough サブハンドラ ──

    /// `Decision::PassThrough` アーム全体を処理する。
    ///
    /// 各 `try_*` / `handle_*` を早期 return チェーンで呼び出し、
    /// 全チェックを通過した場合は OS に PassThrough を返す。
    fn handle_passthrough(
        &mut self,
        platform: &mut WindowsPlatform,
        raw_event: &RawKeyEvent,
    ) -> CallbackResult {
        // awase の SendInput 出力直後 N ms は、OS キュー → アプリ → IME の pipeline で
        // 出力イベントが処理中。この間に user passthrough キー (Enter / Ctrl /
        // Backspace 等) が割り込むと IME composition が cancel され
        // 「タスク → タスk」のような race が発生する。
        // 本ガードは「直近 N ms 以内の passthrough キーは pending と同様に
        // deferr して reinject 時に wait する」ことで race を構造的に解消する。

        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);

        // 1. KeyUp 対称性: deferred KeyDown の VK は KeyUp も reinject に揃える。
        if let Some(result) = self.try_keyup_symmetry(raw_event) {
            return result;
        }

        // 2. warm+TSF Enter/Space/Esc KeyUp: 保留 eager warmup を送信。
        self.try_pending_warmup_on_keyup(platform, raw_event);

        // 3. Ctrl↑ cold recovery: eager_warmup_sent_ms をリセット。
        self.handle_ctrl_up_recovery(platform, raw_event);

        let in_flight_ms = platform.output_in_flight_ms();
        let output_in_flight = in_flight_ms < crate::tuning::OUTPUT_GUARD_MS;
        let has_pending = self.has_pending();

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

        // 4. output in-flight / pending queue: defer して reinject 経由で順序保証。
        if let Some(result) =
            self.try_output_guard_defer(raw_event, output_in_flight, in_flight_ms, has_pending)
        {
            return result;
        }

        // 5. F2 + TSF mode: 物理 F2 を Consume（double-F2 防止）。
        if let Some(result) = self.try_native_f2_consume(platform, raw_event) {
            return result;
        }

        // 6. Space/Enter/Esc KeyDown: warm+TSF または cold の composition 確定処理。
        self.handle_confirm_key_passthrough(platform, raw_event);

        // 7. F2 + KeyDown + non-TSF: mark_cold（Chrome/Win32 向け）。
        self.handle_f2_non_tsf(platform, raw_event);

        // Effects なし → 直接 OS に通す
        // Passthrough 系の VK (Enter, Esc, Tab 等) は awase 出力との
        // 時系列を見えるようログを残す（char/thumb はノイズになるため除外）。
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

    /// KeyUp: 対応する KeyDown を reinject 経由で送っていた場合、
    /// KeyUp も reinject に揃えて INJECTED_MARKER 対称性を保つ。
    /// （WezTerm が INJECTED↓ + physical↑ のペアを異常扱いする可能性を排除）
    fn try_keyup_symmetry(&mut self, raw_event: &RawKeyEvent) -> Option<CallbackResult> {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if !is_key_down && self.deferred_passthrough_vks.remove(&raw_event.vk_code) {
            log::debug!(
                "[relay-sym] PassThrough KeyUp vk={:#04x}: KeyDown was deferred → force reinject for symmetry",
                raw_event.vk_code,
            );
            self.queue
                .push_back(Effect::Input(InputEffect::ReinjectKey(*raw_event)));
            return Some(CallbackResult::Consumed);
        }
        None
    }

    /// warm+TSF Enter/Space/Escape KeyDown で保留した eager warmup を KeyUp で送信する。
    /// KeyDown 時は SendInput(F2) → CallNextHookEx(Enter↓) の順になり WezTerm が
    /// F2 (新 composition 開始) を受け取った後に Enter で即確定してしまう。
    /// KeyUp タイミングでは Enter↓ が既に処理済みのため F2 との競合なし。
    fn try_pending_warmup_on_keyup(&mut self, platform: &WindowsPlatform, raw_event: &RawKeyEvent) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if !is_key_down
            && raw_event.vk_code.is_composition_confirm_key()
            && self.pending_warmup_on_keyup
        {
            self.pending_warmup_on_keyup = false;
            log::debug!(
                "[composition] vk={:#04x} KeyUp: 保留 eager warmup 送信 (warm+TSF 変換確定後)",
                raw_event.vk_code,
            );
            platform.send_eager_warmup(self.applied_snapshot.map(|(v, _)| v));
        }
    }

    /// Ctrl↑: cold 状態であれば eager_warmup_sent_ms をリセット（この→kおの バグ対策）。
    /// Ctrl が WezTerm に届いている間、GJI TSF 初期化が中断される可能性がある。
    /// Ctrl↑ を起点としてタイマーを再計測し GJI recovery 時間（500ms）を確保する。
    /// 副作用のみで CallbackResult は返さない。
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn handle_ctrl_up_recovery(&mut self, platform: &mut WindowsPlatform, raw_event: &RawKeyEvent) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if !is_key_down && raw_event.vk_code.is_ctrl_variant() && !platform.is_composition_warm() {
            log::debug!(
                "[composition] Ctrl↑ (vk={:#04x}) cold 検出 → eager_warmup_sent_ms リセット (GJI recovery 500ms 再計測)",
                raw_event.vk_code,
            );
            platform.send_eager_warmup(self.applied_snapshot.map(|(v, _)| v));
        }
    }

    /// OUTPUT_GATE.active が true / pending queue がある場合: Consume + reinject で順序保証する。
    ///
    /// 例外: 修飾キー (Ctrl/Alt/Win) の KeyUp を defer すると、reinject まで OS は
    /// 修飾キーが押されたままと認識し、その間に届く次キーが Ctrl+key 等のショートカット
    /// として誤発火する (Ctrl 残留 → Ctrl+H 暴発)。pair 保持の責務は `try_keyup_symmetry`
    /// が `deferred_passthrough_vks` でカバー済みのため、ここに到達した修飾 Up は
    /// 必ず Down が defer されていない (即 passthrough されている) 状態なので、
    /// Up も即 passthrough しても pair は崩れない。
    fn try_output_guard_defer(
        &mut self,
        raw_event: &RawKeyEvent,
        output_in_flight: bool,
        in_flight_ms: u64,
        has_pending: bool,
    ) -> Option<CallbackResult> {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if !is_key_down && raw_event.vk_code.is_non_shift_modifier() {
            // 修飾 Up は defer しない (Ctrl 残留窓を作らない)。
            // Down が defer されたケースは try_keyup_symmetry が先に捕捉している。
            return None;
        }
        if has_pending || output_in_flight {
            // pending effects または output in-flight 中の passthrough は
            // Consume + reinject 経由で順序保証する。
            let reason = if output_in_flight && !has_pending {
                format!("output in-flight ({in_flight_ms}ms ago)")
            } else if has_pending && output_in_flight {
                format!("pending effects + output in-flight ({in_flight_ms}ms)")
            } else {
                "pending effects".to_string()
            };
            log::debug!(
                "[relay-defer] PassThrough deferred: {reason}, reinject(vk={:#04x} {})",
                raw_event.vk_code,
                if is_key_down { "down" } else { "up" },
            );
            self.queue
                .push_back(Effect::Input(InputEffect::ReinjectKey(*raw_event)));
            // KeyDown を defer した場合は VK を記録して KeyUp も reinject に揃える。
            if is_key_down {
                self.deferred_passthrough_vks.insert(raw_event.vk_code);
            }
            return Some(CallbackResult::Consumed);
        }
        None
    }

    /// vk=0xF2 かつ TSF mode のとき物理 F2 を Consume する（double-F2 防止）。
    ///
    /// 物理 F2 が WezTerm に届いた後に warmup F2 を含むバッチを送ると、
    /// WezTerm の TSF ハンドラが F2 を 2 回受け取り "この→koの" になる
    /// （WezTerm 内部で F2 がトグル動作をしている模様）。
    /// 物理 F2 を Consume し、次の NICOLA バッチの warmup F2 で一本化することで解消する。
    /// → output.rs の composition_warm ドキュメントの設計意図と一致。
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn try_native_f2_consume(
        &mut self,
        platform: &mut WindowsPlatform,
        raw_event: &RawKeyEvent,
    ) -> Option<CallbackResult> {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if raw_event.vk_code == crate::vk::VK_DBE_HIRAGANA && platform.is_tsf_mode() {
            if is_key_down {
                // 物理 F2 消費時の composition 状態更新を platform に委譲する。
                // mark_cold(NativeF2Consumed) + eager warmup を platform 内で処理。
                let _ = platform.on_passthrough_key(
                    raw_event.vk_code,
                    true,
                    self.applied_snapshot.map(|(v, _)| v),
                );
            } else {
                log::debug!(
                    "[composition] vk=0xf2 KeyUp TSF mode → consuming (paired KeyDown was consumed)",
                );
            }
            return Some(CallbackResult::Consumed);
        }
        None
    }

    /// Space/Enter/Esc KeyDown の直接 passthrough: warm+TSF または cold の composition 確定処理。
    /// 副作用のみで CallbackResult は返さない。
    fn handle_confirm_key_passthrough(
        &mut self,
        platform: &mut WindowsPlatform,
        raw_event: &RawKeyEvent,
    ) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        // Space/Enter/Escape の直接 passthrough (KeyDown) は composition を
        // 確定・キャンセルしてコンテキストをアイドル状態に戻す。
        // mark_cold / eager warmup は platform に委譲する。戻り値が true なら warmup を KeyUp へ遅延。
        if is_key_down && raw_event.vk_code.is_composition_confirm_key() {
            let deferred = platform.on_passthrough_key(
                raw_event.vk_code,
                true,
                self.applied_snapshot.map(|(v, _)| v),
            );
            self.pending_warmup_on_keyup = deferred;
        }
    }

    /// vk=0xF2 + KeyDown かつ non-TSF mode のとき mark_cold（Chrome/Win32 向け）。
    /// 副作用のみで CallbackResult は返さない。
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn handle_f2_non_tsf(&mut self, platform: &mut WindowsPlatform, raw_event: &RawKeyEvent) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        // F2 non-TSF mode: passthrough + mark_cold（Chrome/Win32 向け）
        // mark_cold(F2NonTsf) を platform に委譲する。
        if raw_event.vk_code == crate::vk::VK_DBE_HIRAGANA && is_key_down {
            let _ = platform.on_passthrough_key(
                raw_event.vk_code,
                true,
                self.applied_snapshot.map(|(v, _)| v),
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
                platform.on_reinject_key(
                    event.vk_code,
                    true,
                    self.applied_snapshot.map(|(v, _)| v),
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
            platform.on_reinject_key(event.vk_code, true, self.applied_snapshot.map(|(v, _)| v));
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
        // send_engine_state_ime_key に渡す applied 値をトレイトオブジェクト取得前に確定する。
        let applied_for_engine_key = self.applied_snapshot.map(|(v, _)| v);
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
                UiEffect::EngineStateChanged { enabled } => {
                    platform_rt.update_tray(enabled);
                    platform_rt.send_engine_state_ime_key(enabled, applied_for_engine_key);
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
        origin: EffectOrigin,
    ) -> Option<(bool, awase::platform::ImeOpenOutcome)> {
        let (imm_first, shadow_on, applied_at_ms) = {
            let view = platform.build_ime_control_view(self.applied_snapshot);
            (
                crate::ime_controller::CONTROLLER.imm_cross_is_first_applicable(&view),
                view.control.shadow_on,
                view.control.applied_at_ms,
            )
        };
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
            self.applied_snapshot = Some((open, 0));
            // IMM が set_ime_open_cross_process(open) 完了後に注入する VK_DBE_DBCSCHAR/
            // VK_DBE_SBCSCHAR KeyUp は key_pipeline の suppress_physical (ImmCross プロファイル
            // の KANJI VK 全 Consume) で構造的に遮断されるため、ここでは applied_snapshot 更新のみ。
            log::debug!(
                "[dispatch-ime] ImmCross async: optimistic applied_snapshot={open} \
                 (suppress send_engine_state_ime_key)"
            );
            let guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
            win32_async::spawn_local(async move {
                let ok = crate::ime::set_ime_open_cross_process_async(open).await;
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
            // EngineIntent (Ctrl+無変換 等、ユーザーの明示的操作) かつ ImmCross が使えない
            // プロファイル (Imm32Unavailable: Chrome/Edge 等、TsfNative: LINE XAML 入力等) の場合、
            // shadow state が desync していても VK_KANJI / F14 を確実に送信するため
            // apply_context を !open に override する。
            //
            // 背景: フォーカス変更直後や awase 起動時に実 IME 状態が unknown になり、
            // applied_snapshot=None または false のまま IME が ON になっていることがある。
            // この状態で KanjiToggle/GjiDirect が shadow=desired と判断してスキップし、
            // Ctrl+無変換 が効かなくなる。
            // ユーザーの明示的操作では shadow desync を無視して必ず送信することで対処する。
            //
            // TsfNative (LINE の Windows.UI.Input.InputSite.WindowClass 等) でも
            // KanjiToggle がフォールバックとして使われるため同様の desync 対策が必要。
            // GJI が起動している場合は GjiDirectStrategy (F13/F14) が選ばれるため override 不要。
            // F14 は shadow に関わらず常に送信される (べき等)。F13 も GjiDirectStrategy が自前で
            // shadow_on チェックを持つため executor 側の override は冗長かつ有害。
            let mut apply_context = self.applied_snapshot;
            if origin == EffectOrigin::EngineIntent {
                let profile = platform.current_app_profile();
                if !profile.can_use_imm32_cross_process()
                    && !crate::tsf::observer::gji_monitor_healthy()
                {
                    let now_ms = crate::hook::current_tick_ms();
                    // SetOpen(false): 「確認済み OFF」なら永続スキップ。
                    //   applied_at_ms > 0 = フォーカス変更後に実 apply が 1 回以上完了 = 信頼できる。
                    //   applied_at_ms == 0 = フォーカス変更直後のプリシンクのみ = 不確定なので override 許可。
                    //   これにより定常状態で Ctrl+無変換 を複数回押しても VK_KANJI が重複送信されず、
                    //   IME が OFF → ON と誤トグルするバグを防ぐ。
                    // SetOpen(true): 300ms ウィンドウを維持。
                    //   KeyDown+KeyUp 二重送信を防ぎつつ、フォーカス変更後 Ctrl+変換 の再試行を許容。
                    let skip_override = if open {
                        // ON 方向: 300ms ウィンドウ (従来動作)
                        shadow_on == open
                            && applied_at_ms > 0
                            && now_ms.saturating_sub(applied_at_ms) < 300
                    } else {
                        // OFF 方向: 実 apply 確認済みなら永続スキップ
                        shadow_on == open && applied_at_ms > 0
                    };
                    if skip_override {
                        log::debug!(
                            "[dispatch-ime] KanjiToggle (profile={:?}): skip override \
                             (confirmed dir={open}, applied {}ms ago)",
                            profile,
                            now_ms.saturating_sub(applied_at_ms)
                        );
                    } else {
                        // override: apply_ime_open_with_applied に現在 state=!open と見せて
                        // KanjiToggle が必ず VK_KANJI を送信するようにする
                        apply_context = Some((!open, 0));
                        log::debug!(
                            "[dispatch-ime] KanjiToggle (profile={:?}): override context={} → force VK_KANJI",
                            profile, !open
                        );
                    }
                }
            }
            let outcome = platform.apply_ime_open_with_applied(open, apply_context);
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
    #[allow(clippy::needless_pass_by_ref_mut)]
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
        self.applied_snapshot = Some((effective, crate::hook::current_tick_ms()));
    }
}
