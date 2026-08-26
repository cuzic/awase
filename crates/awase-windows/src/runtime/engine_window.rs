#![allow(unsafe_code)]
//! エンジンスレッド専用の message-only window。

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, PostQuitMessage, RegisterClassW, CREATESTRUCTW,
    HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_NCCREATE, WNDCLASSW,
};

static ENGINE_HWND: AtomicIsize = AtomicIsize::new(0);
static MODAL_DEPTH: AtomicU32 = AtomicU32::new(0);
static NEEDS_ENGINE_RESYNC: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PumpContext {
    Main,
    Nested,
}

#[must_use]
pub(crate) fn engine_hwnd() -> Option<HWND> {
    let raw = ENGINE_HWND.load(Ordering::Acquire);
    (raw != 0).then_some(HWND(raw as *mut _))
}

#[must_use]
pub(crate) fn current_pump_context() -> PumpContext {
    if MODAL_DEPTH.load(Ordering::Acquire) == 0 {
        PumpContext::Main
    } else {
        PumpContext::Nested
    }
}

pub(crate) fn take_needs_engine_resync() -> bool {
    NEEDS_ENGINE_RESYNC.swap(false, Ordering::AcqRel)
}

#[must_use]
pub(crate) fn is_in_modal_pump() -> bool {
    MODAL_DEPTH.load(Ordering::Acquire) != 0
}

pub(crate) fn mark_needs_engine_resync() {
    NEEDS_ENGINE_RESYNC.store(true, Ordering::Release);
}

pub(crate) struct ModalPumpGuard;

impl ModalPumpGuard {
    #[must_use]
    pub(crate) fn enter() -> Self {
        MODAL_DEPTH.fetch_add(1, Ordering::AcqRel);
        mark_needs_engine_resync();
        Self
    }
}

impl Drop for ModalPumpGuard {
    fn drop(&mut self) {
        MODAL_DEPTH.fetch_sub(1, Ordering::AcqRel);
        mark_needs_engine_resync();
        if MODAL_DEPTH.load(Ordering::Acquire) == 0 && crate::is_quit_requested() {
            unsafe {
                PostQuitMessage(0);
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct EngineWindowGuard(HWND);

impl Drop for EngineWindowGuard {
    fn drop(&mut self) {
        ENGINE_HWND.store(0, Ordering::Release);
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

pub(crate) fn create_engine_window() -> windows::core::Result<EngineWindowGuard> {
    let hinstance = unsafe { GetModuleHandleW(None)? };
    let class_name = w!("awase_engine_message_window");
    let wc = WNDCLASSW {
        hInstance: hinstance.into(),
        lpszClassName: class_name,
        lpfnWndProc: Some(engine_wnd_proc),
        ..Default::default()
    };
    unsafe {
        let _ = RegisterClassW(&raw const wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            PCWSTR::null(),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinstance.into()),
            Some(std::ptr::null::<CREATESTRUCTW>().cast()),
        )?;
        ENGINE_HWND.store(hwnd.0 as isize, Ordering::Release);
        Ok(EngineWindowGuard(hwnd))
    }
}

unsafe extern "system" fn engine_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg != WM_NCCREATE && crate::app::dispatch_engine_message(hwnd, msg, wparam, lparam) {
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_pump_guard_marks_resync_on_enter_and_drop() {
        NEEDS_ENGINE_RESYNC.store(false, Ordering::Release);
        {
            let _guard = ModalPumpGuard::enter();
            assert!(take_needs_engine_resync());
        }
        assert!(take_needs_engine_resync());
    }
}
