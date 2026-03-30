//! フォーカス検出モジュール
//!
//! ウィンドウのフォーカス変更を監視し、テキスト入力コントロールかどうかを判定する。
//! Phase 1-2（同期）+ Phase 3（UIA 非同期）の多段判定を行い、
//! タイピングパターンによる推定も併用する。

pub mod cache;
pub mod classify;
pub mod msaa;
pub mod pattern;
pub mod uia;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};

use std::sync::mpsc;

use crate::focus::cache::FocusCache;
use crate::focus::pattern::KeyPatternTracker;
use crate::focus::uia::SendableHwnd;

/// フォーカス検出に関するシングルスレッド状態を集約する構造体
///
/// `FOCUS_KIND`（`AtomicU8`）は UIA ワーカースレッドからもアクセスされるため、
/// スレッド安全性のために別の static として保持する。
pub struct FocusDetector {
    pub cache: FocusCache,
    pub overrides: awase::config::FocusOverrides,
    pub last_focus_info: Option<(u32, String)>,
    pub pattern_tracker: KeyPatternTracker,
    pub uia_sender: Option<mpsc::Sender<SendableHwnd>>,
}

impl FocusDetector {
    pub fn new(overrides: awase::config::FocusOverrides) -> Self {
        Self {
            cache: FocusCache::new(),
            overrides,
            last_focus_info: None,
            pattern_tracker: KeyPatternTracker::new(),
            uia_sender: None,
        }
    }

    pub fn set_uia_sender(&mut self, sender: mpsc::Sender<SendableHwnd>) {
        self.uia_sender = Some(sender);
    }

    /// フォーカス変更時などに単一スレッド側の状態をリセットする
    #[allow(dead_code)] // フォーカスリセットの将来拡張用に保持
    pub fn clear(&mut self) {
        self.cache = FocusCache::new();
        self.last_focus_info = None;
        self.pattern_tracker.clear();
    }
}

/// `WINEVENT_OUTOFCONTEXT` (0x0000) — コールバックをメッセージループで実行
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;

/// `EVENT_OBJECT_FOCUS` (0x8005) — フォーカス変更イベント
const EVENT_OBJECT_FOCUS: u32 = 0x8005;

/// フォーカス変更イベントフックを登録する
///
/// `WINEVENT_OUTOFCONTEXT` を使用するため、コールバックはメッセージループ上で実行される。
/// これにより `classify_focus` が非同期（キーイベントとは別タイミング）で呼ばれる。
pub fn install_focus_hook() {
    unsafe {
        let hook = SetWinEventHook(
            EVENT_OBJECT_FOCUS,
            EVENT_OBJECT_FOCUS,
            None,
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        if hook.is_invalid() {
            log::warn!("Failed to install focus event hook");
        } else {
            log::info!("Focus event hook installed");
        }
    }
}

/// フォーカス変更イベントのコールバック
///
/// `WINEVENT_OUTOFCONTEXT` により、メッセージループのコンテキストで呼ばれる。
/// 状態遷移は `AppState::on_focus_changed` に委譲し、ここでは副作用のみ実行する。
unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event != EVENT_OBJECT_FOCUS {
        return;
    }

    let process_id = classify::get_window_process_id(hwnd);
    let class_name = classify::get_class_name_string(hwnd);

    let Some(app) = crate::APP.get_mut() else {
        return;
    };
    let actions = app.on_focus_changed(hwnd, process_id, &class_name);

    for action in actions {
        match action {
            crate::AppAction::InvalidateEngineContext(reason) => {
                app.invalidate_engine_context(reason);
            }
            crate::AppAction::RefreshImeStateCache => {
                app.refresh_ime_state_cache();
            }
            _ => {}
        }
    }
}

