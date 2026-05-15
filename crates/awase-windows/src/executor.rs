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

#[allow(missing_debug_implementations)]
pub struct DecisionExecutor {
    pub platform: WindowsPlatform,
    /// Effects キュー（FIFO 順序保証）
    queue: VecDeque<Effect>,
    /// フックの動作モード
    hook_mode: HookMode,
    /// Reinject 経由で送った PassThrough KeyDown の VK 集合。
    /// 対応する KeyUp も reinject に揃えて INJECTED_MARKER 対称性を保つ。
    deferred_passthrough_vks: HashSet<u16>,
}

impl DecisionExecutor {
    pub fn new(platform: WindowsPlatform, hook_mode: HookMode) -> Self {
        Self {
            platform,
            queue: VecDeque::new(),
            hook_mode,
            deferred_passthrough_vks: HashSet::new(),
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

    /// `WM_EXECUTE_EFFECTS` ハンドラから呼ぶ。
    pub fn drain_deferred(&mut self) {
        while let Some(effect) = self.queue.pop_front() {
            self.execute_one(effect);
        }
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
        // awase の SendInput 出力直後 N ms は、OS キュー → アプリ → IME の pipeline で
        // 出力イベントが処理中。この間に user passthrough キー (Enter / Ctrl /
        // Backspace 等) が割り込むと IME composition が cancel され
        // 「タスク → タスk」のような race が発生する。
        // 本ガードは「直近 N ms 以内の passthrough キーは pending と同様に
        // deferr して reinject 時に wait する」ことで race を構造的に解消する。
        const OUTPUT_GUARD_MS: u64 = 50;

        match decision {
            Decision::PassThrough => {
                let is_key_down = matches!(raw_event.event_type, awase::types::KeyEventType::KeyDown);

                // KeyUp: 対応する KeyDown を reinject 経由で送っていた場合、
                // KeyUp も reinject に揃えて INJECTED_MARKER 対称性を保つ。
                // （WezTerm が INJECTED↓ + physical↑ のペアを異常扱いする可能性を排除）
                if !is_key_down && self.deferred_passthrough_vks.remove(&raw_event.vk_code.0) {
                    log::debug!(
                        "[relay-sym] PassThrough KeyUp vk={:#04x}: KeyDown was deferred → force reinject for symmetry",
                        raw_event.vk_code.0,
                    );
                    self.queue.push_back(Effect::Input(InputEffect::ReinjectKey(*raw_event)));
                    return HookResult {
                        callback: CallbackResult::Consumed,
                        has_pending: true,
                    };
                }

                let in_flight_ms = self.platform.output.ms_since_last_send();
                let output_in_flight = in_flight_ms < OUTPUT_GUARD_MS;
                let has_pending = self.has_pending();

                log::debug!(
                    "[relay-guard] vk={:#04x} {} in_flight_ms={} has_pending={} output_in_flight={}",
                    raw_event.vk_code.0,
                    if is_key_down { "down" } else { "up" },
                    if in_flight_ms == u64::MAX { "never".to_string() } else { in_flight_ms.to_string() },
                    has_pending,
                    output_in_flight,
                );

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
                    HookResult {
                        callback: CallbackResult::Consumed,
                        has_pending: true,
                    }
                } else {
                    // F2 (VK_DBE_HIRAGANA) in TSF mode: 物理 F2 を Consume してパススルーしない。
                    //
                    // 物理 F2 が WezTerm に届いた後に warmup F2 を含むバッチを送ると、
                    // WezTerm の TSF ハンドラが F2 を 2 回受け取り "この→koの" になる
                    // （WezTerm 内部で F2 がトグル動作をしている模様）。
                    // 物理 F2 を Consume し、次の NICOLA バッチの warmup F2 で一本化することで解消する。
                    // → output.rs の composition_warm ドキュメントの設計意図と一致。
                    if raw_event.vk_code.0 == 0xF2 && self.platform.output.is_tsf_mode() {
                        if is_key_down {
                            log::debug!(
                                "[composition] vk=0xf2 passthrough TSF mode → consuming (prevent double-F2), marking cold",
                            );
                            self.platform.output.mark_composition_cold(crate::output::ColdReason::NativeF2Consumed);
                        } else {
                            log::debug!(
                                "[composition] vk=0xf2 KeyUp TSF mode → consuming (paired KeyDown was consumed)",
                            );
                        }
                        return HookResult {
                            callback: CallbackResult::Consumed,
                            has_pending: self.has_pending(),
                        };
                    }
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
                    // Space/Enter/Escape の直接 passthrough (KeyDown) は composition を
                    // 確定・キャンセルしてコンテキストをアイドル状態に戻す。
                    if is_key_down && matches!(raw_event.vk_code.0, 0x20 | 0x0D | 0x1B) {
                        log::debug!(
                            "[composition] passthrough vk={:#04x} KeyDown → marking cold",
                            raw_event.vk_code.0,
                        );
                        self.platform.output.mark_composition_cold(crate::output::ColdReason::PassthroughConfirmKey);
                    }
                    // F2 non-TSF mode: passthrough + mark_cold（Chrome/Win32 向け）
                    if raw_event.vk_code.0 == 0xF2 && is_key_down {
                        log::debug!(
                            "[composition] vk=0xf2 passthrough direct → marking cold",
                        );
                        self.platform.output.mark_composition_cold(crate::output::ColdReason::F2NonTsf);
                    }
                    HookResult {
                        callback: CallbackResult::PassThrough,
                        has_pending: false,
                    }
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

    // ── 共通 ──

    fn is_input_critical(effect: &Effect) -> bool {
        matches!(effect, Effect::Input(_) | Effect::Timer(_))
    }

    fn execute_one(&mut self, effect: Effect) {
        // ReinjectKey は output guard 期間を消化してから注入する必要があるため、
        // 先にガード時間チェック + sleep を行ってからトレイト経由の処理に渡す。
        if let Effect::Input(InputEffect::ReinjectKey(event)) = effect {
            const OUTPUT_GUARD_MS: u64 = 50;
            let elapsed = self.platform.output.ms_since_last_send();
            if elapsed < OUTPUT_GUARD_MS {
                let remaining = OUTPUT_GUARD_MS - elapsed;
                log::debug!(
                    "[reinject-wait] sleeping {remaining}ms (output {elapsed}ms ago) before reinject(vk={:#04x})",
                    event.vk_code.0,
                );
                // メインスレッドを短時間ブロックする。message loop / hook callback も
                // この間停止するが、最大 50ms なので体感影響は小さい。
                // この sleep を入れないと Ctrl/Enter が IME composition を cancel する
                // race が残る (実測: SendInput 直後 9ms で race 発生)。
                std::thread::sleep(std::time::Duration::from_millis(remaining));
            }
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
                    "[composition] reinject KeyDown vk={:#04x} → marking cold",
                    event.vk_code.0,
                );
                self.platform.output.mark_composition_cold(crate::output::ColdReason::ReinjectConfirmKey);
            }
            return;
        }

        let mut ime_set_open_true = false;

        {
            let platform: &mut dyn PlatformRuntime = &mut self.platform;
            match effect {
                Effect::Input(ie) => match ie {
                    InputEffect::SendKeys(actions) => platform.send_keys(&actions),
                    InputEffect::ReinjectKey(_) => unreachable!("handled above"),
                },
                Effect::Timer(te) => match te {
                    TimerEffect::Set { id, duration } => platform.set_timer(id, duration),
                    TimerEffect::Kill(id) => platform.kill_timer(id),
                },
                Effect::Ime(ie) => match ie {
                    ImeEffect::SetOpen(open) => {
                        let success = platform.set_ime_open(open);
                        if !success {
                            log::warn!(
                                "set_ime_open({open}) failed — requesting IME refresh for resync"
                            );
                        }
                        // 成功/失敗に関わらず refresh をスケジュール（安全ネット + 定期ポーリング復帰）。
                        platform.post_ime_refresh();
                        if open {
                            ime_set_open_true = true;
                        }
                    }
                    ImeEffect::RequestRefresh => platform.post_ime_refresh(),
                },
                Effect::Ui(ue) => match ue {
                    UiEffect::EngineStateChanged { enabled } => platform.update_tray(enabled),
                },
            }
        } // platform の借用をここで解放

        // IME ON 直後の最初の composition が cold start にならないよう cold にマークする。
        if ime_set_open_true {
            log::debug!("[composition] ImeEffect::SetOpen(true) → marking cold");
            self.platform.output.mark_composition_cold(crate::output::ColdReason::SetOpenTrue);
        }
    }
}
