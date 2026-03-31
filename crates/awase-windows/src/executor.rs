/// Decision の副作用を実行する。
///
/// # レイヤー分離アーキテクチャ
///
/// フックコールバックと Effect 実行を完全に分離する:
///
/// - **フックコールバック** (`execute_from_hook`):
///   Engine の判断（consume/passthrough）のみ行い、即座に OS に返す。
///   全 Effects はキューに溜め、`WM_EXECUTE_EFFECTS` で委譲する。
///   OS API は一切呼ばない。
///
/// - **メッセージループ** (`execute_from_loop`, `drain_deferred`):
///   キューに溜まった Effects を全て実行する。
///   時間制約がないため、SendInput, IME 操作等の重い処理も安全。
use std::collections::VecDeque;

use awase::engine::{
    Decision, Effect, FocusEffect, ImeCacheEffect, ImeEffect, InputEffect, TimerEffect, UiEffect,
};
use awase::platform::PlatformRuntime;

use crate::hook::CallbackResult;
use crate::platform::WindowsPlatform;

/// `execute_from_hook` の戻り値。
/// フックコールバックに返す `CallbackResult` と、Effect キューの状態を含む。
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
}

impl DecisionExecutor {
    pub const fn new(platform: WindowsPlatform) -> Self {
        Self {
            platform,
            queue: VecDeque::new(),
        }
    }

    /// フックコールバックから呼ぶ。
    ///
    /// consume/passthrough を即座に返す。
    /// 入出力系 Effects（SendKeys, ReinjectKey）は即座実行（キー順序を保証）。
    /// 重い Effects（IME 制御, トレイ更新等）はキューに溜める。
    ///
    /// 戻り値の `has_pending` が true なら、呼び出し元は
    /// `PostMessage(WM_EXECUTE_EFFECTS)` でメッセージループに通知すること。
    pub fn execute_from_hook(&mut self, decision: Decision) -> HookResult {
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
                // キー入出力: 即座実行（キー順序を保証するため遅延不可）
                self.execute_one(effect);
            } else {
                // 重い処理: メッセージループに遅延
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

    /// メッセージループから呼ぶ。
    ///
    /// Decision の Effects を全て即座に実行する。時間制約なし。
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
    ///
    /// フックコールバックが溜めた Effects を全て実行する。
    pub fn drain_deferred(&mut self) {
        while let Some(effect) = self.queue.pop_front() {
            self.execute_one(effect);
        }
    }

    /// キューに Effects が溜まっているか
    pub fn has_pending(&self) -> bool {
        !self.queue.is_empty()
    }

    /// キー入出力に関わる Effect か（フック内で即座実行すべきか）
    ///
    /// SendKeys と ReinjectKey はキー順序に関わるため遅延不可。
    /// Timer も同時打鍵判定のタイミングに影響するため即座実行。
    /// ImeCache は IME 状態判定の整合性に必要。
    fn is_input_critical(effect: &Effect) -> bool {
        matches!(
            effect,
            Effect::Input(_) | Effect::Timer(_) | Effect::ImeCache(_)
        )
    }

    /// Effect を1つ実行する
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
