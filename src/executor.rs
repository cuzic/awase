/// Decision の副作用を実行する。
///
/// Win32 API (SendInput, SetTimer, KillTimer, PostMessageW, ImmSetOpenStatus) の
/// 呼び出しを集約する唯一のポイント。判断ロジックを含まない。
use windows::Win32::Foundation::HWND;

use awase::engine::{
    Decision, Effect, FocusEffect, ImeCacheEffect, ImeEffect, InputEffect, TimerEffect, UiEffect,
};
use awase::types::ImeCacheState;

use crate::focus::cache::DetectionSource;
use crate::focus::uia::SendableHwnd;
use crate::hook::CallbackResult;
use crate::output::Output;
use crate::tray::SystemTray;
use crate::{reinject_key, FOCUS_KIND, IME_RELIABILITY, IME_STATE_CACHE};

use crate::runtime::FocusDetector;

pub struct DecisionExecutor {
    pub output: Output,
    pub tray: SystemTray,
    pub focus: FocusDetector,
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

    /// Effect リストを実行する
    fn execute_effects(&mut self, effects: Vec<Effect>) {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer};

        for effect in effects {
            match effect {
                Effect::Input(ie) => match ie {
                    InputEffect::SendKeys(actions) => {
                        self.output.send_keys(&actions);
                    }
                    InputEffect::ReinjectKey(event) => {
                        // SAFETY: reinject_key は Win32 API (SendInput)。メインスレッドから呼ぶ。
                        unsafe { reinject_key(&event) };
                    }
                },
                Effect::Timer(te) => match te {
                    TimerEffect::Set { id, duration } => {
                        let ms = u32::try_from(duration.as_millis()).unwrap_or(u32::MAX);
                        // SAFETY: SetTimer は Win32 API。メインスレッドから呼ぶ。
                        unsafe {
                            let _ = SetTimer(HWND::default(), id, ms, None);
                        }
                    }
                    TimerEffect::Kill(id) => {
                        // SAFETY: KillTimer は Win32 API。メインスレッドから呼ぶ。
                        unsafe {
                            let _ = KillTimer(HWND::default(), id);
                        }
                    }
                },
                Effect::Ime(ie) => match ie {
                    ImeEffect::SetOpen(open) => {
                        // SAFETY: set_ime_open_cross_process は Win32 API。メインスレッドから呼ぶ。
                        let _ = unsafe { crate::ime::set_ime_open_cross_process(open) };
                    }
                    ImeEffect::RequestCacheRefresh => {
                        // SAFETY: PostMessageW は Win32 API。メインスレッドから呼ぶ。
                        unsafe {
                            let _ = PostMessageW(
                                HWND::default(),
                                crate::WM_IME_KEY_DETECTED,
                                WPARAM(0),
                                LPARAM(0),
                            );
                        }
                    }
                },
                Effect::Ui(ue) => match ue {
                    UiEffect::EngineStateChanged { enabled } => {
                        self.tray.set_enabled(enabled);
                    }
                },
                Effect::Focus(fe) => match fe {
                    FocusEffect::UpdateFocusKind(kind) => {
                        kind.store(&FOCUS_KIND);
                    }
                    FocusEffect::ResetImeReliability => {
                        awase::types::ImeReliability::Unknown.store(&IME_RELIABILITY);
                    }
                    FocusEffect::InsertFocusCache {
                        process_id,
                        class_name,
                        kind,
                    } => {
                        self.focus.cache.insert(
                            process_id,
                            class_name,
                            kind,
                            DetectionSource::Automatic,
                        );
                    }
                    FocusEffect::RequestUiaClassification => {
                        // UIA 非同期判定をリクエスト
                        if let Some(tx) = self.focus.uia_sender.as_ref() {
                            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
                            let fg = unsafe { GetForegroundWindow() };
                            let _ = tx.send(SendableHwnd(fg));
                        }
                    }
                    FocusEffect::UpdateLastFocusInfo {
                        process_id,
                        class_name,
                    } => {
                        self.focus.last_focus_info = Some((process_id, class_name));
                    }
                },
                Effect::ImeCache(ice) => match ice {
                    ImeCacheEffect::UpdateStateCache { ime_on } => {
                        let new_state = ImeCacheState::from(ime_on);
                        let old_state = new_state.swap(&IME_STATE_CACHE);
                        if old_state != new_state {
                            log::debug!(
                                "IME state cache updated: {} → {}",
                                old_state.as_str(),
                                new_state.as_str(),
                            );
                        }
                    }
                },
            }
        }
    }
}
