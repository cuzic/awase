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
use awase::platform::{EffectOrigin, PlatformRuntime};
use awase::types::{RawKeyEvent, VkCode};

use crate::hook::CallbackResult;
use crate::platform::WindowsPlatform;
use crate::vk::VkCodeExt;
use crate::RawKeyEventExt as _;

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
    deferred_passthrough_vks: HashSet<VkCode>,
    /// warm+TSF Enter/Space/Escape KeyDown 後に KeyUp で eager warmup を送信するフラグ。
    ///
    /// hook callback 内では `SendInput(F2)` → `CallNextHookEx(Enter↓)` の順になり、
    /// WezTerm が F2 (新 composition 開始) を受け取った後に Enter で即確定してしまう。
    /// KeyUp タイミングで F2 を送れば、Enter↓ は処理済みのため競合しない。
    pending_warmup_on_keyup: bool,
    /// OUTPUT_GUARD で park した 1 個分の Effect スロット。
    ///
    /// 不変条件: `guard_held.is_some()` ⟺ `TIMER_OUTPUT_GUARD` が登録済み。
    /// drain は「slot を先に試す → 通過したら queue に進む」の 2 段構え。
    /// queue 本体は常に純粋 FIFO で `push_back` / `pop_front` のみ。
    guard_held: Option<Effect>,
    /// sync path の apply outcome を `Runtime::flush_pending_apply_events` で
    /// dispatch するための pending record。詳細は `PendingApplyEvent` 参照。
    pending_apply_events: Vec<crate::state::ime_event::PendingApplyEvent>,
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
            guard_held: None,
            pending_apply_events: Vec::new(),
        }
    }

    /// sync path の pending apply events をすべて取り出す。
    /// `Runtime` 側で event dispatch (generation 照合付き) するために呼ぶ。
    pub fn drain_pending_apply_events(
        &mut self,
    ) -> Vec<crate::state::ime_event::PendingApplyEvent> {
        std::mem::take(&mut self.pending_apply_events)
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
    /// `guard_held` に park 済みの Effect があれば最初にそれを試し、
    /// output guard 期間中なら `TIMER_OUTPUT_GUARD` を設定して即座に返る（block_on しない）。
    /// タイマー発火後に再び呼ばれ、guard 解除済みなら reinject を実行する。
    pub fn drain_deferred(&mut self) {
        // 同一 drain 呼び出し内で最初の ReinjectKey だけ OUTPUT_GUARD を適用する。
        // 連続する reinject (例: Win_DOWN→X_DOWN→X_UP→Win_UP) を個別にガードすると
        // Win が 150ms 以上 OS 側でスタックし、後続のショートカットが Win+key と
        // 誤解釈されるため、先頭の reinject が guard を通過したら残りはまとめて送出する。
        let mut reinject_guard_passed = false;

        // 1) 前回 park した Effect があれば最初に試す。
        //    guard 解除済みなら execute_one してから queue に進む (batching を継続)。
        if let Some(effect) = self.guard_held.take() {
            if let Some(remaining) = self.output_guard_remaining() {
                log::debug!(
                    "[reinject-guard] held effect, output {}ms ago, suspending for {remaining}ms",
                    crate::tuning::OUTPUT_GUARD_MS - remaining,
                );
                self.park_in_guard(effect, remaining);
                return;
            }
            self.execute_one(effect);
            reinject_guard_passed = true;
        }

        // 2) queue を FIFO で drain。
        while let Some(effect) = self.queue.pop_front() {
            let is_reinject = matches!(effect, Effect::Input(InputEffect::ReinjectKey(_)));
            if is_reinject && !reinject_guard_passed {
                if let Some(remaining) = self.output_guard_remaining() {
                    log::debug!(
                        "[reinject-guard] output {}ms ago, suspending drain for {remaining}ms",
                        crate::tuning::OUTPUT_GUARD_MS - remaining,
                    );
                    self.park_in_guard(effect, remaining);
                    return;
                }
                reinject_guard_passed = true;
            } else if !is_reinject {
                // NICOLA 出力など reinject 以外の effect は mark_send を呼ぶので
                // 次の reinject には再びガードを適用する。
                reinject_guard_passed = false;
            }
            self.execute_one(effect);
        }

        // 全 Effect を消化: lingering な timer を kill (no-op if not registered)。
        if self.guard_held.is_none() {
            self.platform.timer.kill(crate::TIMER_OUTPUT_GUARD);
        }
    }

    /// `TIMER_OUTPUT_GUARD` 発火時に呼ぶ。timer を kill して drain を再試行する。
    pub fn on_output_guard_timer(&mut self) {
        self.platform.timer.kill(crate::TIMER_OUTPUT_GUARD);
        self.drain_deferred();
    }

    /// queue または guard slot に Effect が残っているか
    pub fn has_pending(&self) -> bool {
        !self.queue.is_empty() || self.guard_held.is_some()
    }

    /// output guard 期間中なら残り ms を返す。期間外なら None。
    fn output_guard_remaining(&self) -> Option<u64> {
        let elapsed = self.platform.output.ms_since_last_send();
        if elapsed < crate::tuning::OUTPUT_GUARD_MS {
            Some(crate::tuning::OUTPUT_GUARD_MS - elapsed)
        } else {
            None
        }
    }

    /// Effect を guard slot に park し、TIMER_OUTPUT_GUARD を再設定する。
    /// 再設定は idempotent (remaining は last_send からの相対時刻基準で計算される)。
    fn park_in_guard(&mut self, effect: Effect, remaining: u64) {
        self.guard_held = Some(effect);
        self.platform.timer.set(
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
    pub fn enqueue_reinject(&mut self, event: RawKeyEvent) {
        self.queue
            .push_back(Effect::Input(InputEffect::ReinjectKey(event)));
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
                    raw_event.vk_code,
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
            raw_event.vk_code,
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
            && raw_event.vk_code.is_composition_confirm_key()
            && self.pending_warmup_on_keyup
        {
            self.pending_warmup_on_keyup = false;
            log::debug!(
                "[composition] vk={:#04x} KeyUp: 保留 eager warmup 送信 (warm+TSF 変換確定後)",
                raw_event.vk_code,
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
            && raw_event.vk_code.is_ctrl_variant()
            && !self.platform.output.is_composition_warm()
        {
            log::debug!(
                "[composition] Ctrl↑ (vk={:#04x}) cold 検出 → eager_warmup_sent_ms リセット (GJI recovery 500ms 再計測)",
                raw_event.vk_code,
            );
            self.platform.output.send_eager_tsf_warmup();
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
            self.queue.push_back(Effect::Input(InputEffect::ReinjectKey(*raw_event)));
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
    fn try_native_f2_consume(&mut self, raw_event: &RawKeyEvent) -> Option<CallbackResult> {
        let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);
        if raw_event.vk_code == crate::vk::VK_DBE_HIRAGANA && self.platform.output.is_tsf_mode() {
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
        if is_key_down && raw_event.vk_code.is_composition_confirm_key() {
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
                    raw_event.vk_code,
                );
                self.platform.output.mark_composition_cold(crate::output::ColdReason::PassthroughConfirmKey);
                self.pending_warmup_on_keyup = true;
            } else {
                // cold または non-TSF: mark cold + eager F2 warmup
                // 直前の warm+TSF フラグがあれば解除（別キーが確定を引き継いだ）
                self.pending_warmup_on_keyup = false;
                log::debug!(
                    "[composition] passthrough vk={:#04x} KeyDown → marking cold + eager warmup",
                    raw_event.vk_code,
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
        if raw_event.vk_code == crate::vk::VK_DBE_HIRAGANA && is_key_down {
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
            // Phase 3b: sync path で ImeApplySucceeded/Failed event を dispatch するため、
            // outcome を caller (Runtime::flush_pending_apply_events) に伝達する。
            // async path (ImmCross) は spawn_local 内で直接 dispatch するためここを経由しない。
            self.pending_apply_events.push(
                crate::state::ime_event::PendingApplyEvent { target: open, outcome },
            );
        }
    }

    /// F2-TSF 特殊扱い + 通常 reinject + confirm キー後処理。
    fn handle_reinject(&mut self, event: RawKeyEvent) {
        let is_key_down = matches!(event.event_type, awase::types::KeyEventType::KeyDown);
        let dir = if is_key_down { "down" } else { "up" };

        // F2 (VK_DBE_HIRAGANA) in TSF mode: deferred F2 も reinject しない。
        // pending 中に F2 が来た場合も ReinjectKey としてキューに入るが、
        // TSF モードでは物理 F2 を WezTerm に届けないことで double-F2 を防ぐ。
        if event.vk_code == crate::vk::VK_DBE_HIRAGANA && self.platform.output.is_tsf_mode() {
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
            event.vk_code,
        );
        // OutputActiveGuard を先に取得してから spawn_local で SendInput を RUNTIME 借用外に移す。
        // RUNTIME 借用中に SendInput を呼ぶと WH_KEYBOARD_LL フックが再入し、ユーザーキーが
        // NICOLA 処理をスキップして素通しになる（「いが l になった」バグの原因）。
        // spawn_local 実行中にユーザーキーが届いても OUTPUT_GATE.active=true で INPUT_DEFER
        // に退避され、guard drop 後に drain されて正しく NICOLA 処理される。
        let guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
        let vk_code = event.vk_code;
        win32_async::spawn_local(async move {
            // SAFETY: spawn_local はメインスレッドのメッセージループで実行される。
            unsafe { event.reinject() };
            // Space/Enter/Escape の reinject (KeyDown) は composition を確定・キャンセルする。
            // Backspace 等は composition を維持するためここでは対象外。
            if is_key_down && vk_code.is_composition_confirm_key() {
                let _ = crate::with_app(|app| {
                    log::debug!(
                        "[composition] reinject KeyDown vk={:#04x} → marking cold + eager warmup",
                        vk_code,
                    );
                    app.executor.platform.output.mark_composition_cold(crate::output::ColdReason::ReinjectConfirmKey);
                    app.executor.platform.output.send_eager_tsf_warmup();
                });
            }
            drop(guard);
        });
    }

    /// Effect::* の match dispatch。ImeEffect::SetOpen の結果のみ Some で返す。
    fn dispatch_effect(&mut self, effect: Effect) -> Option<(bool, awase::platform::ImeOpenOutcome)> {
        // ImeEffect::SetOpen は ImmCross-first か否かで async / sync を分岐するため
        // 先に処理する（後段の `let platform = &mut self.platform` が `self.platform`
        // を独占する前に `build_ime_control_view` を呼ぶ必要がある）。
        if let Effect::Ime(ImeEffect::SetOpen { open, origin }) = effect {
            return self.dispatch_ime_set_open(open, origin);
        }
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
                ImeEffect::SetOpen { .. } => unreachable!("handled above"),
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

    /// `ImeEffect::SetOpen` の専用 dispatch。
    ///
    /// `ImmCrossProcessStrategy` が現在のコンテキストで最初に適用可能な場合は
    /// `set_ime_open_cross_process_async` を `win32_async::spawn_local` で async 実行する
    /// (`SendMessageTimeoutW` 由来の `with_app` 再入を回避するため)。
    /// それ以外（GjiDirect / KanjiToggle 経路）はキー注入のみで非ブロッキングなため
    /// 既存の同期 chain を維持する。
    ///
    /// async 経路では同期 outcome を返せないため `None` を返し、latch 更新
    /// (`post_apply_ime_open`) と `post_ime_refresh` を spawn_local 内で完了させる。
    fn dispatch_ime_set_open(
        &mut self,
        open: bool,
        origin: EffectOrigin,
    ) -> Option<(bool, awase::platform::ImeOpenOutcome)> {
        let imm_first = {
            let view = self.platform.build_ime_control_view();
            crate::ime_controller::CONTROLLER.imm_cross_is_first_applicable(&view)
        };
        if imm_first {
            // ── async path (ImmCross が選ばれるアプリ) ──
            // OutputActiveGuard を先に取得しておくことで、await 中に走るフックコールバックは
            // INPUT_DEFER へ退避され、SetOpen 進行中に新キーが engine に届かない。
            // guard は spawn_local 内の async move 末尾で drop され、その時点で OUTPUT_GATE
            // が解除されて drain がキックされる。
            //
            // 同一エフェクトバッチ内で直後に処理される UiEffect::EngineStateChanged →
            // send_engine_state_ime_key が last_applied_ime_on を見て VK_F4/VK_F3 を
            // 送信するかを決める。async 完了前は last_applied が旧値のままなので
            // 「不整合あり→モードキー送信」と判断されてしまう。
            // LINE/Qt 等の ImmCross アプリはこの VK_F4 Up に対して VK_F3 Down を
            // 生成し（extra=0x0、マーカーなし）、shadow toggle が ON→OFF に反転する。
            // → 楽観的に last_applied を更新して send_engine_state_ime_key をスキップさせる。
            self.platform.output.set_ime_apply_latch(open);
            // IMM が set_ime_open_cross_process(open) 完了後に注入する VK_DBE_DBCSCHAR/
            // VK_DBE_SBCSCHAR KeyUp は key_pipeline の suppress_physical (ImmCross プロファイル
            // の KANJI VK 全 Consume) で構造的に遮断されるため、ここでは latch 更新のみ。
            log::debug!(
                "[dispatch-ime] ImmCross async: optimistic last_applied={open} \
                 (suppress send_engine_state_ime_key)"
            );
            let guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
            win32_async::spawn_local(async move {
                let ok = crate::ime::set_ime_open_cross_process_async(open).await;
                let outcome = if ok {
                    awase::platform::ImeOpenOutcome::Applied
                } else {
                    log::debug!("[apply-ime] ImmCross failed (async), trying fallback");
                    crate::with_app(|app| {
                        let view = app.executor.platform.build_ime_control_view();
                        crate::ime_controller::CONTROLLER.apply_skipping_imm(open, &view)
                    })
                    .unwrap_or(awase::platform::ImeOpenOutcome::Failed)
                };
                let _ = crate::with_app(|app| {
                    if outcome == awase::platform::ImeOpenOutcome::Failed {
                        log::warn!("apply_ime_open({open}) failed (async)");
                    }
                    // Phase 3b: ImeApplySucceeded/Failed event を dispatch して
                    // shadow_model.applied_open / pending を generation 照合で更新する。
                    let pending_gen = app
                        .platform_state
                        .ime
                        .shadow_model
                        .pending
                        .as_ref()
                        .map(|p| p.generation);
                    if let Some(generation) = pending_gen {
                        let event = crate::state::ime_event::ImeEvent::from_apply_outcome(
                            open, outcome, generation,
                        );
                        app.platform_state.ime.dispatch_event(event);
                    }
                    app.executor.post_apply_ime_open(open, outcome);
                    let platform: &mut dyn PlatformRuntime = &mut app.executor.platform;
                    platform.post_ime_refresh();
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
            // latch を !open に強制する。
            //
            // 背景: フォーカス変更直後や awase 起動時に実 IME 状態が unknown になり、
            // last_applied_ime_on=false のまま IME が ON になっていることがある。
            // この状態で KanjiToggle/GjiDirect が shadow=desired と判断してスキップし、
            // Ctrl+無変換 が効かなくなる。
            // ユーザーの明示的操作では shadow desync を無視して必ず送信することで対処する。
            //
            // TsfNative (LINE の Windows.UI.Input.InputSite.WindowClass 等) でも
            // KanjiToggle がフォールバックとして使われるため同様の desync 対策が必要。
            // GJI が起動している場合は GjiDirectStrategy (F13/F14) が選ばれるため latch override 不要。
            // F14 は shadow に関わらず常に送信される (べき等)。F13 も GjiDirectStrategy が自前で
            // shadow_on チェックを持つため executor 側の override は冗長かつ有害。
            if origin == EffectOrigin::EngineIntent {
                let profile = self.platform.focus.current_app_profile();
                if !profile.can_use_imm32_cross_process()
                    && !crate::tsf::observer::gji_monitor_healthy()
                {
                    let last_ms = self.platform.output.last_applied_ime_on_ms();
                    let now_ms = crate::hook::current_tick_ms();
                    // SetOpen(false): 「確認済み OFF」なら永続スキップ。
                    //   applied_ms > 0 = フォーカス変更後に実 apply が 1 回以上完了 = 信頼できる。
                    //   applied_ms == 0 = フォーカス変更直後のプリシンクのみ = 不確定なので override 許可。
                    //   これにより定常状態で Ctrl+無変換 を複数回押しても VK_KANJI が重複送信されず、
                    //   IME が OFF → ON と誤トグルするバグを防ぐ。
                    // SetOpen(true): 300ms ウィンドウを維持。
                    //   KeyDown+KeyUp 二重送信を防ぎつつ、フォーカス変更後 Ctrl+変換 の再試行を許容。
                    let skip_override = if !open {
                        // OFF 方向: 実 apply 確認済みなら永続スキップ
                        self.platform.output.last_applied_ime_on() == open
                            && last_ms > 0
                    } else {
                        // ON 方向: 300ms ウィンドウ (従来動作)
                        self.platform.output.last_applied_ime_on() == open
                            && last_ms > 0
                            && now_ms.saturating_sub(last_ms) < 300
                    };
                    if skip_override {
                        log::debug!(
                            "[dispatch-ime] KanjiToggle (profile={:?}): skip latch force \
                             (confirmed dir={open}, applied {}ms ago)",
                            profile,
                            now_ms.saturating_sub(last_ms)
                        );
                    } else {
                        self.platform.output.set_ime_apply_latch(!open);
                        log::debug!(
                            "[dispatch-ime] KanjiToggle (profile={:?}): override latch={} → force VK_KANJI",
                            profile, !open
                        );
                    }
                }
            }
            let platform: &mut dyn PlatformRuntime = &mut self.platform;
            let outcome = platform.apply_ime_open(open);
            if outcome == awase::platform::ImeOpenOutcome::Failed {
                log::warn!("apply_ime_open({open}) failed");
            }
            platform.post_ime_refresh();
            Some((open, outcome))
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
            ImeOpenOutcome::Failed => {
                // ImmCross async パスで楽観的に set_ime_apply_latch(open) した場合のロールバック。
                // Failed なら実際の IME 状態は !open のまま。
                self.platform.output.set_ime_apply_latch(!open);
            }
        }
        // IME ON 直後の最初の composition が cold start にならないよう cold にマークする。
        // IME OFF 時も cold にマークする: warm のまま放置すると Enter/Space/Escape が
        // warm+TSF パスに流れ、GJI に composition がない状態でアプリへ漏れる
        // （LINE 等でメッセージ送信につながる）。
        if open {
            log::debug!("[composition] ImeEffect::SetOpen(true) → marking cold");
            self.platform.output.mark_composition_cold(crate::output::ColdReason::SetOpenTrue);
            self.platform.output.send_eager_tsf_warmup();
        } else {
            log::debug!("[composition] ImeEffect::SetOpen(false) → marking cold (prevent warm+TSF Enter leak)");
            self.platform.output.mark_composition_cold(crate::output::ColdReason::SetOpenFalse);
        }
    }
}
