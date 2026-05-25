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
use awase::engine::{
    Decision, Effect, ImeEffect, InputEffect, TimerEffect, UiEffect,
};
use awase::platform::PlatformRuntime;
use awase::types::RawKeyEvent;

use crate::hook::CallbackResult;
use crate::platform::WindowsPlatform;

/// `execute_from_hook` の戻り値。
#[derive(Debug)]
pub struct HookResult {
    /// OS に返す consume/passthrough 判定
    pub callback: CallbackResult,
    /// true なら `PostMessage(WM_EXECUTE_EFFECTS)` でメッセージループに通知が必要
    pub has_pending: bool,
}

pub struct DecisionExecutor {
    pub platform: WindowsPlatform,
    /// Effects キュー（FIFO 順序保証）
    queue: VecDeque<Effect>,
    /// フックの動作モード
    hook_mode: HookMode,
    /// Reinject 経由で送った PassThrough KeyDown の VK 集合。
    /// 対応する KeyUp も reinject に揃えて INJECTED_MARKER 対称性を保つ。
    deferred_passthrough_vks: HashSet<u16>,
    /// warm+TSF Enter/Space/Escape KeyDown 後に KeyUp で eager warmup を送信するフラグ。
    ///
    /// hook callback 内では `SendInput(F2)` → `CallNextHookEx(Enter↓)` の順になり、
    /// WezTerm が F2 (新 composition 開始) を受け取った後に Enter で即確定してしまう。
    /// KeyUp タイミングで F2 を送れば、Enter↓ は処理済みのため競合しない。
    pending_warmup_on_keyup: bool,
    /// TIMER_OUTPUT_GUARD が発火待ちの間 true。
    /// この間は drain_deferred が再入しても新規タイマーを二重登録しない。
    guard_timer_active: bool,
}

impl std::fmt::Debug for DecisionExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecisionExecutor").finish_non_exhaustive()
    }
}

impl DecisionExecutor {
    pub fn new(platform: WindowsPlatform, hook_mode: HookMode) -> Self {
        Self {
            platform,
            queue: VecDeque::new(),
            hook_mode,
            deferred_passthrough_vks: HashSet::new(),
            pending_warmup_on_keyup: false,
            guard_timer_active: false,
        }
    }

    /// フックコールバックから呼ぶ。
    ///
    /// - Filter モード: 入出力系は即座実行、重い処理は遅延。PassThrough を OS に返す。
    /// - Relay モード: 全 Effects をキューに入れ、PassThrough キーも ReinjectKey に変換。
    ///   常に Consumed を返す。
    pub fn execute_from_hook(&mut self, decision: Decision, raw_event: &RawKeyEvent) -> HookResult {
        match self.hook_mode {
            HookMode::Filter => self.execute_filter(decision),
            HookMode::Relay => self.execute_relay(decision, raw_event),
        }
    }

    /// メッセージループから呼ぶ。全 Effects を即座に実行する。
    pub fn execute_from_loop(&mut self, decision: Decision) -> CallbackResult {
        let (consumed, effects) = match decision {
            Decision::PassThrough => return CallbackResult::PassThrough,
            Decision::PassThroughWith { effects } => (false, effects),
            Decision::Consume { effects } => (true, effects),
        };

        for effect in effects {
            self.execute_one(effect);
        }

        if consumed {
            CallbackResult::Consumed
        } else {
            CallbackResult::PassThrough
        }
    }

    /// `WM_EXECUTE_EFFECTS` ハンドラ、および `TIMER_OUTPUT_GUARD` タイマーから呼ぶ。
    ///
    /// キューの先頭が `ReinjectKey` かつ output guard 期間中の場合、
    /// `TIMER_OUTPUT_GUARD` を設定して即座に返る（block_on しない）。
    /// タイマー発火後に再び呼ばれ、guard 解除済みなら reinject を実行する。
    pub fn drain_deferred(&mut self) {
        while let Some(effect) = self.queue.pop_front() {
            if matches!(effect, Effect::Input(InputEffect::ReinjectKey(_))) {
                let elapsed = self.platform.output.ms_since_last_send();
                if elapsed < crate::tuning::OUTPUT_GUARD_MS {
                    let remaining = crate::tuning::OUTPUT_GUARD_MS - elapsed;
                    log::debug!(
                        "[reinject-guard] output {elapsed}ms ago, suspending drain for {remaining}ms"
                    );
                    self.queue.push_front(effect);
                    if !self.guard_timer_active {
                        self.platform.timer.set(
                            crate::TIMER_OUTPUT_GUARD,
                            std::time::Duration::from_millis(remaining),
                        );
                        self.guard_timer_active = true;
                    }
                    return;
                }
            }
            self.execute_one(effect);
        }
        // 全 Effect を消化: 先に WM_EXECUTE_EFFECTS が処理を完了した場合に guard timer を解除
        if self.guard_timer_active {
            self.platform.timer.kill(crate::TIMER_OUTPUT_GUARD);
            self.guard_timer_active = false;
        }
    }

    /// `TIMER_OUTPUT_GUARD` 発火時に呼ぶ。guard フラグをクリアして drain を再試行する。
    pub fn on_output_guard_timer(&mut self) {
        self.platform.timer.kill(crate::TIMER_OUTPUT_GUARD);
        self.guard_timer_active = false;
        self.drain_deferred();
    }

    /// キューに Effects が溜まっているか
    pub fn has_pending(&self) -> bool {
        !self.queue.is_empty()
    }

    // ── Filter モード ──

    fn execute_filter(&mut self, decision: Decision) -> HookResult {
        let (consumed, effects) = match decision {
            Decision::PassThrough => {
                return HookResult {
                    callback: CallbackResult::PassThrough,
                    has_pending: self.has_pending(),
                }
            }
            Decision::PassThroughWith { effects } => (false, effects),
            Decision::Consume { effects } => (true, effects),
        };

        for effect in effects {
            if Self::is_input_critical(&effect) {
                self.execute_one(effect);
            } else {
                self.queue.push_back(effect);
            }
        }

        HookResult {
            callback: if consumed {
                CallbackResult::Consumed
            } else {
                CallbackResult::PassThrough
            },
            has_pending: self.has_pending(),
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

    fn execute_relay(&mut self, decision: Decision, raw_event: &RawKeyEvent) -> HookResult {
        match decision {
            Decision::PassThrough => {
                let callback = self.handle_passthrough(raw_event);
                HookResult {
                    has_pending: self.has_pending(),
                    callback,
                }
            }
            Decision::PassThroughWith { mut effects } => {
                // flush 出力あり → Consume して flush + キー再注入を FIFO でキュー
                log::debug!(
                    "[relay-flush] PassThroughWith: queue {} effect(s) + reinject(vk={:#04x} {})",
                    effects.len(),
                    raw_event.vk_code.0,
                    match raw_event.event_type {
                            awase::types::KeyEventType::KeyDown => "down",
                            awase::types::KeyEventType::KeyUp => "up",
                        },
                );
                effects.push(Effect::Input(InputEffect::ReinjectKey(*raw_event)));
                self.queue.extend(effects);
                HookResult {
                    callback: CallbackResult::Consumed,
                    has_pending: true,
                }
            }
            Decision::Consume { effects } => {
                // Engine が消費 → Effects をキューに入れる
                self.queue.extend(effects);
                HookResult {
                    callback: CallbackResult::Consumed,
                    has_pending: self.has_pending(),
                }
            }
        }
    }

    // ── PassThrough サブハンドラ ──

    /// `Decision::PassThrough` アーム全体を処理する。
    ///
    /// 各 `try_*` / `handle_*` を早期 return チェーンで呼び出し、
    /// 全チェックを通過した場合は OS に PassThrough を返す。
    fn handle_passthrough(&mut self, raw_event: &RawKeyEvent) -> CallbackResult {
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
        self.try_pending_warmup_on_keyup(raw_event);

        // 3. Ctrl↑ cold recovery: eager_warmup_sent_ms をリセット。
        self.handle_ctrl_up_recovery(raw_event);

        let in_flight_ms = self.platform.output.ms_since_last_send();
        let output_in_flight = in_flight_ms < crate::tuning::OUTPUT_GUARD_MS;
        let has_pending = self.has_pending();

        log::debug!(
            "[relay-guard] vk={:#04x} {} in_flight_ms={} has_pending={} output_in_flight={}",
            raw_event.vk_code.0,
            if is_key_down { "down" } else { "up" },
            if in_flight_ms == u64::MAX { "never".to_string() } else { in_flight_ms.to_string() },
            has_pending,
            output_in_flight,
        );

        // 4. output in-flight / pending queue: defer して reinject 経由で順序保証。
        if let Some(result) = self.try_output_guard_defer(raw_event, output_in_flight, in_flight_ms, has_pending) {
            return result;
        }

        // 5. F2 + TSF mode: 物理 F2 を Consume（double-F2 防止）。
        if let Some(result) = self.try_native_f2_consume(raw_event) {
            return result;
        }

        // 6. Space/Enter/Esc KeyDown: warm+TSF または cold の composition 確定処理。
        self.handle_confirm_key_passthrough(raw_event);

        // 7. F2 + KeyDown + non-TSF: mark_cold（Chrome/Win32 向け）。
        self.handle_f2_non_tsf(raw_event);

        // Effects なし → 直接 OS に通す
        // Passthrough 系の VK (Enter, Esc, Tab 等) は awase 出力との
        // 時系列を見えるようログを残す（char/thumb はノイズになるため除外）。
        if matches!(
            raw_event.key_classification,
            awase::types::KeyClassification::Passthrough
        ) {
            log::debug!(
                "[relay-passthrough] PassThrough idle: direct OS pass-through (vk={:#04x} {})",
                raw_event.vk_code.0,
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
        if !is_key_down && self.deferred_passthrough_vks.remove(&raw_event.vk_code.0) {
            log::debug!(
                "[relay-sym] PassThrough KeyUp vk={:#04x}: KeyDown was deferred → force reinject for symmetry",
                raw_event.vk_code.0,
            );
            self.queue.push_back(Effect::Input(InputEffect::ReinjectKey(*raw_event)));
            return Some(CallbackResult::Consumed);
        }
        None
    }

    /// warm+TSF Enter/Space/Escape KeyDown で保留した eager warmup を KeyUp で送信する。
    /// KeyDown 時は SendInput(F2) → CallNextHookEx(Enter↓) の順になり WezTerm が
    /// F2 (新 composition 開始) を受け取った後に Enter で即確定してしまう。
    /// KeyUp タイミングでは Enter↓ が既に処理済みのため F2 との競合なし。
    fn try_pending_warmup_on_keyup(&mut self, raw_event: &RawKeyEvent) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if !is_key_down
            && matches!(raw_event.vk_code.0, 0x20 | 0x0D | 0x1B)
            && self.pending_warmup_on_keyup
        {
            self.pending_warmup_on_keyup = false;
            log::debug!(
                "[composition] vk={:#04x} KeyUp: 保留 eager warmup 送信 (warm+TSF 変換確定後)",
                raw_event.vk_code.0,
            );
            self.platform.output.send_eager_tsf_warmup();
        }
    }

    /// Ctrl↑: cold 状態であれば eager_warmup_sent_ms をリセット（この→kおの バグ対策）。
    /// Ctrl が WezTerm に届いている間、GJI TSF 初期化が中断される可能性がある。
    /// Ctrl↑ を起点としてタイマーを再計測し GJI recovery 時間（500ms）を確保する。
    /// 副作用のみで CallbackResult は返さない。
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn handle_ctrl_up_recovery(&mut self, raw_event: &RawKeyEvent) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if !is_key_down
            && matches!(raw_event.vk_code.0, 0x11 | 0xA2 | 0xA3)
            && !self.platform.output.is_composition_warm()
        {
            log::debug!(
                "[composition] Ctrl↑ (vk={:#04x}) cold 検出 → eager_warmup_sent_ms リセット (GJI recovery 500ms 再計測)",
                raw_event.vk_code.0,
            );
            self.platform.output.send_eager_tsf_warmup();
        }
    }

    /// OUTPUT_GATE.active が true / pending queue がある場合: Consume + reinject で順序保証する。
    fn try_output_guard_defer(
        &mut self,
        raw_event: &RawKeyEvent,
        output_in_flight: bool,
        in_flight_ms: u64,
        has_pending: bool,
    ) -> Option<CallbackResult> {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
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
                raw_event.vk_code.0,
                if is_key_down { "down" } else { "up" },
            );
            self.queue.push_back(Effect::Input(InputEffect::ReinjectKey(*raw_event)));
            // KeyDown を defer した場合は VK を記録して KeyUp も reinject に揃える。
            if is_key_down {
                self.deferred_passthrough_vks.insert(raw_event.vk_code.0);
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
    fn try_native_f2_consume(&mut self, raw_event: &RawKeyEvent) -> Option<CallbackResult> {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if raw_event.vk_code.0 == 0xF2 && self.platform.output.is_tsf_mode() {
            if is_key_down {
                log::debug!(
                    "[composition] vk=0xf2 passthrough TSF mode → consuming (prevent double-F2), marking cold",
                );
                self.platform.output.mark_composition_cold(crate::output::ColdReason::NativeF2Consumed);
                // 物理 F2 消費直後に warmup F2 を即送信。WezTerm の TSF context
                // 初期化がユーザーの次キーストロークまでに完了するよう先行させる。
                self.platform.output.send_eager_tsf_warmup();
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
    fn handle_confirm_key_passthrough(&mut self, raw_event: &RawKeyEvent) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        // Space/Enter/Escape の直接 passthrough (KeyDown) は composition を
        // 確定・キャンセルしてコンテキストをアイドル状態に戻す。
        if is_key_down && matches!(raw_event.vk_code.0, 0x20 | 0x0D | 0x1B) {
            let was_warm = self.platform.output.is_composition_warm();
            let is_tsf = self.platform.output.is_tsf_mode();
            if was_warm && is_tsf {
                // 変換確定/取消 (TSF composition active 中の Enter/Space/Escape):
                // cold にするが eager F2 は KeyDown では送らない。
                // hook callback 内で SendInput(F2) すると CallNextHookEx(Enter↓) より
                // 先に F2 が WezTerm に届き、IME 確定前に composition を壊して
                // Enter が PTY に素通りする。
                // 代わりに対応 KeyUp タイミングで eager F2 を送信する
                // (pending_warmup_on_keyup フラグで追跡)。
                // Enter↓ が WezTerm で処理済みの後に F2 が届くため競合しない。
                log::debug!(
                    "[composition] passthrough vk={:#04x} KeyDown (warm+TSF) → 変換確定, cold markのみ (eager F2はKeyUpで送信)",
                    raw_event.vk_code.0,
                );
                self.platform.output.mark_composition_cold(crate::output::ColdReason::PassthroughConfirmKey);
                self.pending_warmup_on_keyup = true;
            } else {
                // cold または non-TSF: mark cold + eager F2 warmup
                // 直前の warm+TSF フラグがあれば解除（別キーが確定を引き継いだ）
                self.pending_warmup_on_keyup = false;
                log::debug!(
                    "[composition] passthrough vk={:#04x} KeyDown → marking cold + eager warmup",
                    raw_event.vk_code.0,
                );
                self.platform.output.mark_composition_cold(crate::output::ColdReason::PassthroughConfirmKey);
                // 次打鍵が 305ms 以内でも文字化けしないよう即 F2 warmup を先行送信する。
                // IME OFF の場合は send_eager_tsf_warmup が内部でガードする。
                self.platform.output.send_eager_tsf_warmup();
            }
        }
    }

    /// vk=0xF2 + KeyDown かつ non-TSF mode のとき mark_cold（Chrome/Win32 向け）。
    /// 副作用のみで CallbackResult は返さない。
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn handle_f2_non_tsf(&mut self, raw_event: &RawKeyEvent) {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        // F2 non-TSF mode: passthrough + mark_cold（Chrome/Win32 向け）
        if raw_event.vk_code.0 == 0xF2 && is_key_down {
            log::debug!(
                "[composition] vk=0xf2 passthrough direct → marking cold",
            );
            self.platform.output.mark_composition_cold(crate::output::ColdReason::F2NonTsf);
        }
    }

    // ── 共通 ──

    const fn is_input_critical(effect: &Effect) -> bool {
        matches!(effect, Effect::Input(_) | Effect::Timer(_))
    }

    fn execute_one(&mut self, effect: Effect) {
        if let Effect::Input(InputEffect::ReinjectKey(event)) = effect {
            self.handle_reinject(event);
            return;
        }
        if let Some((open, outcome)) = self.dispatch_effect(effect) {
            self.post_apply_ime_open(open, outcome);
        }
    }

    /// F2-TSF 特殊扱い + 通常 reinject + confirm キー後処理。
    fn handle_reinject(&mut self, event: RawKeyEvent) {
        let is_key_down = matches!(event.event_type, awase::types::KeyEventType::KeyDown);
        let dir = if is_key_down { "down" } else { "up" };

        // F2 (VK_DBE_HIRAGANA) in TSF mode: deferred F2 も reinject しない。
        // pending 中に F2 が来た場合も ReinjectKey としてキューに入るが、
        // TSF モードでは物理 F2 を WezTerm に届けないことで double-F2 を防ぐ。
        if event.vk_code.0 == 0xF2 && self.platform.output.is_tsf_mode() {
            if is_key_down {
                log::debug!(
                    "[reinject-tsf] vk=0xf2 KeyDown TSF mode → consuming deferred F2 (no reinject), marking cold",
                );
                self.platform.output.mark_composition_cold(crate::output::ColdReason::NativeF2Consumed);
                // deferred F2 も即 eager warmup を送信する（passthrough 経路と同様）。
                self.platform.output.send_eager_tsf_warmup();
            } else {
                log::debug!(
                    "[reinject-tsf] vk=0xf2 KeyUp TSF mode → consuming (paired KeyDown was consumed)",
                );
            }
            return;
        }

        log::debug!(
            "[reinject] vk={:#04x} {dir} (queued passthrough now firing)",
            event.vk_code.0,
        );
        {
            let platform: &mut dyn PlatformRuntime = &mut self.platform;
            platform.reinject_key(&event);
        }
        // Space/Enter/Escape の reinject (KeyDown) は composition を確定・キャンセルする。
        // Backspace 等は composition を維持するためここでは対象外。
        if is_key_down && matches!(event.vk_code.0, 0x20 | 0x0D | 0x1B) {
            log::debug!(
                "[composition] reinject KeyDown vk={:#04x} → marking cold + eager warmup",
                event.vk_code.0,
            );
            self.platform.output.mark_composition_cold(crate::output::ColdReason::ReinjectConfirmKey);
            self.platform.output.send_eager_tsf_warmup();
        }
    }

    /// Effect::* の match dispatch。ImeEffect::SetOpen の結果のみ Some で返す。
    fn dispatch_effect(&mut self, effect: Effect) -> Option<(bool, awase::platform::ImeOpenOutcome)> {
        let platform: &mut dyn PlatformRuntime = &mut self.platform;
        match effect {
            Effect::Input(ie) => match ie {
                InputEffect::SendKeys(actions) => {
                    platform.send_keys(&actions);
                    None
                }
                InputEffect::ReinjectKey(_) => unreachable!("handled in execute_one"),
            },
            Effect::Timer(te) => match te {
                TimerEffect::Set { id, duration } => {
                    platform.set_timer(id, duration);
                    None
                }
                TimerEffect::Kill(id) => {
                    platform.kill_timer(id);
                    None
                }
            },
            Effect::Ime(ie) => match ie {
                ImeEffect::SetOpen { open, origin: _ } => {
                    let outcome = platform.apply_ime_open(open);
                    if outcome == awase::platform::ImeOpenOutcome::Failed {
                        log::warn!("apply_ime_open({open}) failed");
                    }
                    // 成功/失敗に関わらず refresh をスケジュール（安全ネット + 定期ポーリング復帰）。
                    platform.post_ime_refresh();
                    Some((open, outcome))
                }
                ImeEffect::RequestRefresh => {
                    platform.post_ime_refresh();
                    None
                }
            },
            Effect::Ui(ue) => match ue {
                UiEffect::EngineStateChanged { enabled } => {
                    platform.update_tray(enabled);
                    platform.send_engine_state_ime_key(enabled);
                    None
                }
            },
        }
    }

    /// latch 更新 + open==true の cold/warmup 処理。
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn post_apply_ime_open(&mut self, open: bool, outcome: awase::platform::ImeOpenOutcome) {
        use awase::platform::ImeOpenOutcome;
        // 成功した場合は last_applied を更新する（last_applied_ime_on → send_eager_tsf_warmup ガードに使用）。
        // Applied 後に last_applied を更新しないと shadow_on が旧値 true のままになり、
        // IME OFF 直後の Ctrl↑ で VK_DBE_HIRAGANA が送信されて IME が ON に戻るバグが発生する。
        match outcome {
            ImeOpenOutcome::Applied | ImeOpenOutcome::FallbackSent | ImeOpenOutcome::AlreadyMatched => {
                self.platform.output.set_ime_apply_latch(open);
            }
            ImeOpenOutcome::Failed => {}
        }
        // IME ON 直後の最初の composition が cold start にならないよう cold にマークする。
        if open {
            log::debug!("[composition] ImeEffect::SetOpen(true) → marking cold");
            self.platform.output.mark_composition_cold(crate::output::ColdReason::SetOpenTrue);
            self.platform.output.send_eager_tsf_warmup();
        }
    }
}
