//! WinEvent 観察フック。
//!
//! `SetWinEventHook` で GJI candidate window の SHOW/HIDE、OBJ_NAMECHANGE、
//! および IME イベントを捕捉し、[`super::observer::TSF_OBS`] を更新する。

use std::sync::atomic::Ordering;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_HIDE, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SHOW, GetClassNameW,
    WINEVENT_OUTOFCONTEXT,
};

use crate::win32::HwndExt as _;

use super::observer::TSF_OBS;

const GJI_CANDIDATE_CLASS: &str = "GoogleJapaneseInputCandidateWindow";
const MSCTFIME_UI_CLASS: &str = "MSCTFIME UI";

// EVENT_OBJECT_IME_SHOW/HIDE/CHANGE (0x8027–0x8029) は windows crate には定義がないため
// 生の値で定義する。GJI TSF モードでは発火しないが Chrome ホスト側から発火するか検証用。
const EVENT_OBJECT_IME_SHOW: u32 = 0x8027;
const EVENT_OBJECT_IME_HIDE: u32 = 0x8028;
const EVENT_OBJECT_IME_CHANGE: u32 = 0x8029;

/// `SetWinEventHook` の RAII ガード。Drop 時に `UnhookWinEvent` を呼ぶ。
pub struct WinEventHookGuard(pub HWINEVENTHOOK);

impl std::fmt::Debug for WinEventHookGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("WinEventHookGuard")
            .field(&self.0 .0)
            .finish()
    }
}

impl Drop for WinEventHookGuard {
    fn drop(&mut self) {
        // SAFETY: `self.0` は `SetWinEventHook` が返した有効なフックハンドル。
        //         `Drop` は一度しか呼ばれないため二重解除にならない。
        unsafe {
            let _ = UnhookWinEvent(self.0);
        }
        log::info!("[obs-hook] uninstalled");
    }
}

/// WinEvent 観察フックを登録し RAII ガードのリストを返す。
///
/// | フック | イベント範囲 | 目的 |
/// |---|---|---|
/// | NAMECHANGE | 0x800C | WezTerm title 変更 → `wait_for_tsf_cold_settle` early-exit |
/// | OBJECT_SHOW/HIDE | 0x8002-0x8003 | GJI candidate window 表示状態追跡 → raw TSF literal 検出用 |
pub fn install_observation_hooks() -> Vec<WinEventHookGuard> {
    let mut hooks = Vec::new();

    // SAFETY: `observation_event_proc` は `'static` な extern "system" fn ポインタ。
    //         `WINEVENT_OUTOFCONTEXT` によりコールバックはメッセージループスレッドで実行される。
    //         返されたフックは `WinEventHookGuard::drop` で `UnhookWinEvent` される。
    let nc_hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_NAMECHANGE,
            EVENT_OBJECT_NAMECHANGE,
            None,
            Some(observation_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if nc_hook.is_invalid() {
        log::warn!("[obs-hook] failed to install NAMECHANGE hook");
    } else {
        hooks.push(WinEventHookGuard(nc_hook));
    }

    // SAFETY: `observation_event_proc` は `'static` な extern "system" fn ポインタ。
    //         `WINEVENT_OUTOFCONTEXT` によりコールバックはメッセージループスレッドで実行される。
    //         返されたフックは `WinEventHookGuard::drop` で `UnhookWinEvent` される。
    let show_hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_HIDE, // SHOW(0x8002)〜HIDE(0x8003) の両方を捕捉
            None,
            Some(observation_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if show_hook.is_invalid() {
        log::warn!("[obs-hook] failed to install OBJECT_SHOW/HIDE hook");
    } else {
        log::info!(
            "[obs-hook] OBJECT_SHOW/HIDE hook installed (GJI candidate window visibility tracking)"
        );
        hooks.push(WinEventHookGuard(show_hook));
    }

    // SAFETY: `observation_event_proc` は `'static` な extern "system" fn ポインタ。
    //         `WINEVENT_OUTOFCONTEXT` によりコールバックはメッセージループスレッドで実行される。
    //         返されたフックは `WinEventHookGuard::drop` で `UnhookWinEvent` される。
    let ime_hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_IME_SHOW,
            EVENT_OBJECT_IME_CHANGE, // SHOW(0x8027)〜CHANGE(0x8029) の全 IME イベントを捕捉
            None,
            Some(observation_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if ime_hook.is_invalid() {
        log::warn!("[obs-hook] failed to install EVENT_OBJECT_IME_* hook");
    } else {
        log::info!(
            "[obs-hook] EVENT_OBJECT_IME_SHOW/HIDE/CHANGE hook installed (Chrome TSF composition context probe)"
        );
        hooks.push(WinEventHookGuard(ime_hook));
    }

    hooks
}

/// WinEvent 観察コールバック。NAMECHANGE / IME_SHOW / IME_HIDE / IME_CHANGE を処理する。
#[expect(clippy::cognitive_complexity)]
unsafe extern "system" fn observation_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    const OBJID_WINDOW: i32 = 0;
    if id_object != OBJID_WINDOW {
        return;
    }

    match event {
        EVENT_OBJECT_NAMECHANGE => {
            let class = hwnd_class_name(hwnd);
            if class.contains("CASCADIA") {
                let seq = TSF_OBS.focus_namechange.notify();
                log::debug!("[tsf-settle] OBJ_NAMECHANGE #{seq} class={class}");
            }
        }
        EVENT_OBJECT_SHOW => {
            let class = hwnd_class_name(hwnd);
            if class == GJI_CANDIDATE_CLASS {
                TSF_OBS.gji_candidate_visible.store(true, Ordering::Relaxed);
                TSF_OBS.candidate_was_seen.store(true, Ordering::Relaxed);
                TSF_OBS.pending_start_composition.store(true, Ordering::Relaxed);
                let seq = TSF_OBS.gji_candidate_show.notify();
                {
                    let now_ms = crate::hook::current_tick_ms();
                    let last_write_ms = TSF_OBS.gji_last_write_ms.load(Ordering::Relaxed);
                    let write_ago = if last_write_ms == 0 {
                        "never".to_string()
                    } else {
                        format!("{}ms ago", now_ms.saturating_sub(last_write_ms))
                    };
                    log::info!("[gji-obs] candidate SHOW #{seq}: last_gji_write={write_ago}");
                }
            } else if class == MSCTFIME_UI_CLASS {
                log::debug!("[tsf-ime-ui] SHOW hwnd={:?}", hwnd.0);
            }
        }
        EVENT_OBJECT_HIDE => {
            let class = hwnd_class_name(hwnd);
            if class == GJI_CANDIDATE_CLASS {
                TSF_OBS.gji_candidate_visible.store(false, Ordering::Relaxed);
                TSF_OBS.pending_end_composition.store(true, Ordering::Relaxed);
                {
                    let now_ms = crate::hook::current_tick_ms();
                    let last_write_ms = TSF_OBS.gji_last_write_ms.load(Ordering::Relaxed);
                    let write_ago = if last_write_ms == 0 {
                        "never".to_string()
                    } else {
                        format!("{}ms ago", now_ms.saturating_sub(last_write_ms))
                    };
                    log::info!("[gji-obs] candidate HIDE: last_gji_write={write_ago}");
                }
            } else if class == MSCTFIME_UI_CLASS {
                log::debug!("[tsf-ime-ui] HIDE hwnd={:?}", hwnd.0);
            }
        }
        EVENT_OBJECT_IME_SHOW => {
            let class = hwnd_class_name(hwnd);
            TSF_OBS.ime_composition_active.store(true, Ordering::Relaxed);
            let seq = TSF_OBS.ime_show_seq.notify();
            log::info!("[ime-obj] IME_SHOW #{seq} class={class} hwnd={:?}", hwnd.0);
        }
        EVENT_OBJECT_IME_HIDE => {
            let class = hwnd_class_name(hwnd);
            TSF_OBS.ime_composition_active.store(false, Ordering::Relaxed);
            log::info!("[ime-obj] IME_HIDE class={class} hwnd={:?}", hwnd.0);
        }
        EVENT_OBJECT_IME_CHANGE => {
            let class = hwnd_class_name(hwnd);
            let seq = TSF_OBS.ime_change_seq.notify();
            log::info!("[ime-obj] IME_CHANGE #{seq} class={class} hwnd={:?}", hwnd.0);
        }
        _ => {}
    }
}

/// HWND のウィンドウクラス名を取得する。
fn hwnd_class_name(hwnd: HWND) -> String {
    if hwnd.non_null().is_none() {
        return String::new();
    }
    let mut buf = [0u16; 128];
    // SAFETY: `hwnd` は `non_null()` チェックで NULL でないことが確認済み。
    //         `buf` は十分なサイズの有効な UTF-16 バッファ。
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len > 0 {
        #[expect(clippy::cast_sign_loss)]
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        String::new()
    }
}
