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
use awase::engine::{
    Decision, Effect, FocusEffect, ImeCacheEffect, ImeEffect, InputEffect, TimerEffect, UiEffect,
};
use awase::platform::PlatformRuntime;
use awase::types::RawKeyEvent;

use crate::hook::CallbackResult;
use crate::platform::WindowsPlatform;

/// `execute_from_hook` の戻り値。
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
}

impl DecisionExecutor {
    pub fn new(platform: WindowsPlatform, hook_mode: HookMode) -> Self {
        Self {
            platform,
            queue: VecDeque::new(),
            hook_mode,
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

    // ── Relay モード ──

    fn execute_relay(&mut self, decision: Decision, raw_event: &RawKeyEvent) -> HookResult {
        let effects = match decision {
            Decision::PassThrough => {
                // PassThrough キーも Consume して ReinjectKey でキューに入れる
                self.queue
                    .push_back(Effect::Input(InputEffect::ReinjectKey(*raw_event)));
                return HookResult {
                    callback: CallbackResult::Consumed,
                    has_pending: true,
                };
            }
            Decision::PassThroughWith { effects } => {
                // PassThrough + Effects → 全て Consume、キーも ReinjectKey
                let mut all = effects;
                all.push(Effect::Input(InputEffect::ReinjectKey(*raw_event)));
                all
            }
            Decision::Consume { effects } => effects,
        };

        self.queue.extend(effects);

        HookResult {
            callback: CallbackResult::Consumed,
            has_pending: self.has_pending(),
        }
    }

    // ── 共通 ──

    fn is_input_critical(effect: &Effect) -> bool {
        matches!(
            effect,
            Effect::Input(_) | Effect::Timer(_) | Effect::ImeCache(_)
        )
    }

    fn execute_one(&mut self, effect: Effect) {
        let platform: &mut dyn PlatformRuntime = &mut self.platform;
        match effect {
            Effect::Input(ie) => match ie {
                InputEffect::SendKeys(actions) => platform.send_keys(&actions),
                InputEffect::ReinjectKey(event) => platform.reinject_key(&event),
            },
            Effect::Timer(te) => match te {
                TimerEffect::Set { id, duration } => platform.set_timer(id, duration),
                TimerEffect::Kill(id) => platform.kill_timer(id),
            },
            Effect::Ime(ie) => match ie {
                ImeEffect::SetOpen(open) => {
                    platform.set_ime_open(open);
                }
                ImeEffect::RequestCacheRefresh => platform.post_ime_refresh(),
            },
            Effect::Ui(ue) => match ue {
                UiEffect::EngineStateChanged { enabled } => platform.update_tray(enabled),
            },
            Effect::Focus(fe) => match fe {
                FocusEffect::UpdateFocusKind(kind) => platform.update_focus_kind(kind),
                FocusEffect::ResetImeReliability => platform.reset_ime_reliability(),
                FocusEffect::InsertFocusCache {
                    process_id,
                    class_name,
                    kind,
                } => platform.insert_focus_cache(process_id, class_name, kind),
                FocusEffect::RequestUiaClassification => platform.request_uia_classification(),
                FocusEffect::UpdateLastFocusInfo {
                    process_id,
                    class_name,
                } => platform.update_last_focus_info(process_id, class_name),
                FocusEffect::SaveEngineState {
                    process_id,
                    class_name,
                    enabled,
                } => platform.save_engine_state(process_id, class_name, enabled),
            },
            Effect::ImeCache(ice) => match ice {
                ImeCacheEffect::UpdateStateCache { ime_on } => platform.update_ime_cache(ime_on),
                ImeCacheEffect::Invalidate => platform.invalidate_ime_cache(),
            },
        }
    }
}
