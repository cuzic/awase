//! IME 状態の観測 — Win32 API を呼び出して `ImeObservation` を返す。

use std::sync::atomic::{AtomicU8, Ordering};

use awase::engine::ImeObservation;
use awase::types::ImeReliability;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::WindowsAndMessaging::{
    GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

/// Win32 API を使って IME 状態を観測し、OS 非依存の `ImeObservation` を返す。
///
/// 副作用: `IME_IS_KANA_INPUT` フラグも同時に更新する。
///
/// # Safety
/// Win32 API を呼び出す。メインスレッドから呼ぶこと。
pub unsafe fn observe(ime_reliability: &AtomicU8) -> ImeObservation {
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

    // Step 2: クロスプロセス IME 検出
    let cross_process = crate::ime::detect_ime_open_cross_process();

    // Step 3: かな入力方式の検出 → グローバルフラグ更新
    let is_romaji = if let Some(is_kana) = crate::ime::detect_kana_input_method() {
        let prev = crate::IME_IS_KANA_INPUT.swap(is_kana, Ordering::Relaxed);
        if prev != is_kana {
            log::info!(
                "IME input method changed: {} → {}",
                if prev { "kana" } else { "romaji" },
                if is_kana { "kana" } else { "romaji" },
            );
        }
        Some(!is_kana)
    } else {
        None
    };

    // Step 4: ImeReliability を読み取り
    let reliability = ImeReliability::load(ime_reliability);

    ImeObservation {
        cross_process,
        is_japanese,
        reliability,
        is_romaji,
    }
}
