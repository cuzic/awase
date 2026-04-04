//! IME 状態の観測 — Win32 API を呼び出して Preconditions を直接更新する。

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::WindowsAndMessaging::{
    GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

use crate::Preconditions;

/// Win32 API を使って IME 状態を観測し、`Preconditions` を直接更新する。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn observe(preconditions: &mut Preconditions) {
    // Step 1: 対象スレッドの HKL を取得（日本語チェック）
    let lang_id = {
        let mut gui_info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let thread_id = if GetGUIThreadInfo(0, &raw mut gui_info).is_ok() {
            let fg_hwnd = if gui_info.hwndFocus == HWND::default() {
                gui_info.hwndActive
            } else {
                gui_info.hwndFocus
            };
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg_hwnd, Some(&raw mut pid))
        } else {
            0
        };

        let hkl = GetKeyboardLayout(thread_id);
        (hkl.0 as u32) & 0xFFFF
    };
    let is_japanese = lang_id == crate::vk::LANGID_JAPANESE;

    // is_japanese_ime を更新
    preconditions.is_japanese_ime = is_japanese;

    // Step 2: クロスプロセス IME 検出
    let cross_process = crate::ime::detect_ime_open_cross_process();

    // ime_on を更新（クロスプロセス検出が信頼できる場合）
    // None の場合は shadow 値を維持する（更新しない）
    if let Some(ime_on) = cross_process {
        // 日本語以外は常に OFF
        let effective = ime_on && is_japanese;
        preconditions.ime_on = effective;
    } else if !is_japanese {
        // 日本語 IME でなければ常に OFF
        preconditions.ime_on = false;
    }

    // Step 3: かな入力方式の検出 → is_romaji 更新
    if let Some(is_kana) = crate::ime::detect_kana_input_method() {
        let prev_romaji = preconditions.is_romaji;
        preconditions.is_romaji = !is_kana;
        if prev_romaji != preconditions.is_romaji {
            log::info!(
                "IME input method changed: {} → {}",
                if !prev_romaji { "kana" } else { "romaji" },
                if is_kana { "kana" } else { "romaji" },
            );
        }
    }
}
