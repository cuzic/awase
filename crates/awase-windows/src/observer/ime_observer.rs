//! IME 状態の観測 — Win32 API を呼び出してアトミック変数を直接更新する。

use std::sync::atomic::{AtomicU8, Ordering};

use awase::types::ImeReliability;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::WindowsAndMessaging::{
    GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

/// Win32 API を使って IME 状態を観測し、アトミック変数を直接更新する。
///
/// 副作用: `PRECOND_IME_ON`, `PRECOND_IS_JAPANESE`, `IME_IS_KANA_INPUT` を更新する。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn observe(ime_reliability: &AtomicU8) {
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

    // PRECOND_IS_JAPANESE を更新
    crate::PRECOND_IS_JAPANESE.store(is_japanese, Ordering::Relaxed);

    // Step 2: クロスプロセス IME 検出
    let cross_process = crate::ime::detect_ime_open_cross_process();

    // PRECOND_IME_ON を更新（クロスプロセス検出が信頼できる場合）
    // None の場合は shadow 値を維持する（更新しない）
    if let Some(ime_on) = cross_process {
        // 日本語以外は常に OFF
        let effective = ime_on && is_japanese;
        crate::PRECOND_IME_ON.store(effective, Ordering::Release);
    } else if !is_japanese {
        // 日本語 IME でなければ常に OFF
        crate::PRECOND_IME_ON.store(false, Ordering::Release);
    }

    // Step 3: かな入力方式の検出 → グローバルフラグ更新
    if let Some(is_kana) = crate::ime::detect_kana_input_method() {
        let prev = crate::IME_IS_KANA_INPUT.swap(is_kana, Ordering::Relaxed);
        if prev != is_kana {
            log::info!(
                "IME input method changed: {} → {}",
                if prev { "kana" } else { "romaji" },
                if is_kana { "kana" } else { "romaji" },
            );
        }
    }

    // Step 4: ImeReliability を読み取り（ログ用のみ）
    let _reliability = ImeReliability::load(ime_reliability);
}
