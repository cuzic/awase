/// Decision の副作用を実行する。
///
/// `PlatformRuntime` トレイト経由で OS 操作を行う。
/// 判断ロジックを含まない。
use awase::engine::{
    Decision, Effect, FocusEffect, ImeCacheEffect, ImeEffect, InputEffect, TimerEffect, UiEffect,
};
use awase::platform::PlatformRuntime;

use crate::hook::CallbackResult;
use crate::platform_windows::WindowsPlatform;

pub struct DecisionExecutor {
    /// プラットフォーム固有の実装。
    /// `execute_effects` は `PlatformRuntime` トレイト経由でのみアクセスする。
    /// Windows 固有のフィールドへの直接アクセスは `runtime.rs` / `main.rs` から行う。
    pub platform: WindowsPlatform,
}

impl DecisionExecutor {
    /// Decision の副作用を実行する — 唯一の副作用実行ポイント
    pub fn execute(&mut self, decision: Decision) -> CallbackResult {
        let (consumed, effects) = match decision {
            Decision::PassThrough => return CallbackResult::PassThrough,
            Decision::PassThroughWith { effects } => (false, effects),
            Decision::Consume { effects } => (true, effects),
        };
        self.execute_effects(effects);
        if consumed {
            CallbackResult::Consumed
        } else {
            CallbackResult::PassThrough
        }
    }

    /// Effect リストを実行する — `PlatformRuntime` トレイト経由のみ
    fn execute_effects(&mut self, effects: Vec<Effect>) {
        let platform: &mut dyn PlatformRuntime = &mut self.platform;
        for effect in effects {
            match effect {
                Effect::Input(ie) => match ie {
                    InputEffect::SendKeys(actions) => {
                        platform.send_keys(&actions);
                    }
                    InputEffect::ReinjectKey(event) => {
                        platform.reinject_key(&event);
                    }
                },
                Effect::Timer(te) => match te {
                    TimerEffect::Set { id, duration } => {
                        platform.set_timer(id, duration);
                    }
                    TimerEffect::Kill(id) => {
                        platform.kill_timer(id);
                    }
                },
                Effect::Ime(ie) => match ie {
                    ImeEffect::SetOpen(open) => {
                        platform.set_ime_open(open);
                    }
                    ImeEffect::RequestCacheRefresh => {
                        platform.post_ime_refresh();
                    }
                },
                Effect::Ui(ue) => match ue {
                    UiEffect::EngineStateChanged { enabled } => {
                        platform.update_tray(enabled);
                    }
                },
                Effect::Focus(fe) => match fe {
                    FocusEffect::UpdateFocusKind(kind) => {
                        platform.update_focus_kind(kind);
                    }
                    FocusEffect::ResetImeReliability => {
                        platform.reset_ime_reliability();
                    }
                    FocusEffect::InsertFocusCache {
                        process_id,
                        class_name,
                        kind,
                    } => {
                        platform.insert_focus_cache(process_id, class_name, kind);
                    }
                    FocusEffect::RequestUiaClassification => {
                        platform.request_uia_classification();
                    }
                    FocusEffect::UpdateLastFocusInfo {
                        process_id,
                        class_name,
                    } => {
                        platform.update_last_focus_info(process_id, class_name);
                    }
                    FocusEffect::SaveEngineState {
                        process_id,
                        class_name,
                        enabled,
                    } => {
                        platform.save_engine_state(process_id, class_name, enabled);
                    }
                },
                Effect::ImeCache(ice) => match ice {
                    ImeCacheEffect::UpdateStateCache { ime_on } => {
                        platform.update_ime_cache(ime_on);
                    }
                    ImeCacheEffect::Invalidate => {
                        platform.invalidate_ime_cache();
                    }
                },
            }
        }
    }
}
