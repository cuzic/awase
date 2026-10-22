#![allow(unsafe_code)]
//! 診断: Raw Input で物理キーボードの打鍵を、`WH_KEYBOARD_LL` フックとは別経路で観測する（issue #165）。
//!
//! 他のプロセスのフックが物理キーを握りつぶしても、Raw Input（`WM_INPUT`）には物理キーが届く。フックの
//! 物理キー（`hook::hook_physical_key_tick_ms`）と並べると、「フックが物理キーを取りこぼしているか」が分かる
//! （`state::hook_watchdog::raw_vs_hook_verdict`）。**観測だけで、フックの再インストールなどの判断には使わない。**
//!
//! `RIDEV_INPUTSINK` で、フォーカスが他の窓にあるときも `WM_INPUT` を受ける。物理デバイスからのキーは
//! `header.hDevice` が非 NULL で、`SendInput` などの注入は NULL なので区別できる。

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RIDEV_INPUTSINK, RIDEV_REMOVE, RID_INPUT, RIM_TYPEKEYBOARD,
};
use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, WM_INPUT};

/// Raw Input が最後に見た物理キーのキーダウンの時刻（`GetTickCount64`）。0 = まだ無い。
static RAW_PHYSICAL_KEY_TICK_MS: AtomicU64 = AtomicU64::new(0);
/// Raw Input が見た物理キーのキーダウンの通算（診断用）。
static RAW_PHYSICAL_KEY_COUNT: AtomicU32 = AtomicU32::new(0);

/// キーボードの `RAWKEYBOARD::Flags` のブレイク（キーアップ）ビット。
const RI_KEY_BREAK: u16 = 0x0001;
/// 無効な仮想キー（拡張シーケンスの前置など。実際の打鍵ではない）。
const VK_FAKE: u16 = 0xFF;

/// Raw Input の最後の物理キーのキーダウンの時刻（ms）。0 = まだ無い。
#[must_use]
pub fn raw_physical_key_tick_ms() -> u64 {
    RAW_PHYSICAL_KEY_TICK_MS.load(Ordering::Relaxed)
}

/// Raw Input が見た物理キーのキーダウンの通算。
#[must_use]
pub fn raw_physical_key_count() -> u32 {
    RAW_PHYSICAL_KEY_COUNT.load(Ordering::Relaxed)
}

/// 登録の RAII ガード。Drop で解除する。
#[derive(Debug)]
pub struct RawKeyInputGuard(HWND);

impl Drop for RawKeyInputGuard {
    fn drop(&mut self) {
        let device = RAWINPUTDEVICE {
            usUsagePage: 0x01, // Generic Desktop
            usUsage: 0x06,     // Keyboard
            dwFlags: RIDEV_REMOVE,
            hwndTarget: HWND::default(), // RIDEV_REMOVE では NULL にする
        };
        // SAFETY: `device` は 1 要素の有効な配列。サイズは構造体のサイズ。
        let _ = unsafe { RegisterRawInputDevices(&[device], size_of::<RAWINPUTDEVICE>() as u32) };
        tracing::info!(
            "[raw-key] Raw Input keyboard observer unregistered (hwnd {:?})",
            self.0
        );
    }
}

/// キーボードの Raw Input を `hwnd` 宛てに登録する（背景でも受ける）。失敗しても本体の動作に影響しない。
pub fn register(hwnd: HWND) -> Option<RawKeyInputGuard> {
    let device = RAWINPUTDEVICE {
        usUsagePage: 0x01, // Generic Desktop
        usUsage: 0x06,     // Keyboard
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: hwnd,
    };
    // SAFETY: `device` は 1 要素の有効な配列。サイズは構造体のサイズ。
    let result = unsafe { RegisterRawInputDevices(&[device], size_of::<RAWINPUTDEVICE>() as u32) };
    match result {
        Ok(()) => {
            tracing::info!("[raw-key] Raw Input keyboard observer registered (hwnd {hwnd:?})");
            Some(RawKeyInputGuard(hwnd))
        }
        Err(e) => {
            tracing::warn!("[raw-key] RegisterRawInputDevices failed: {e}");
            None
        }
    }
}

/// `WM_INPUT` を受けたときに呼ぶ。物理キーのキーダウンなら時刻と通算を更新する。最後に `DefWindowProcW` を呼ぶ
/// （`WM_INPUT` の後始末のため、システムが要求する）。
pub fn on_wm_input(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    // SAFETY: `RAWINPUT` はゼロで初期化してよい POD。`GetRawInputData` は `size` バイトまで書く。
    unsafe {
        let mut raw: RAWINPUT = std::mem::zeroed();
        let mut size = size_of::<RAWINPUT>() as u32;
        let copied = GetRawInputData(
            HRAWINPUT(lparam.0 as *mut _),
            RID_INPUT,
            Some(std::ptr::from_mut(&mut raw).cast()),
            &raw mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        );
        if copied != u32::MAX
            && raw.header.dwType == RIM_TYPEKEYBOARD.0
            && !raw.header.hDevice.is_invalid()
        {
            let kb = raw.data.keyboard;
            if (kb.Flags & RI_KEY_BREAK) == 0 && kb.VKey != VK_FAKE {
                RAW_PHYSICAL_KEY_TICK_MS.store(crate::hook::current_tick_ms(), Ordering::Relaxed);
                RAW_PHYSICAL_KEY_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }
        let _ = DefWindowProcW(hwnd, WM_INPUT, wparam, lparam);
    }
}
