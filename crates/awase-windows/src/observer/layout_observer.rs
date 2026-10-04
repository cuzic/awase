#![allow(unsafe_code)]
//! ADR-223: フォーカス窓のスレッドの入力言語(HKL)を読む observer。

use crate::state::ime_event::HwndId;
use crate::state::layout_language::{classify_layout_language, lang_id};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

/// 読み取り結果。`japanese` が `None` のときは「不明」(書き込まない。ADR-223 D0)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadLanguage {
    pub japanese: Option<bool>,
    pub tid: u32,
    pub lang_id: u32,
}

impl ThreadLanguage {
    const UNKNOWN: Self = Self {
        japanese: None,
        tid: 0,
        lang_id: 0,
    };
}

/// `hwnd` を持つスレッドの入力言語を読む。窓が無い(`None`)・破棄済み・自プロセスの窓(トレイ・ダイアログ)・
/// スレッド終了(HKL が 0)のときは「不明」。awase 自身のスレッドの言語は読まない(ADR-223 D0・R4-M2)。
///
/// どちらの API も非ブロッキングの読み取りで、フックを待たせない(`GetGUIThreadInfo` は使わない)。
#[must_use]
pub fn read_thread_language(hwnd: Option<HwndId>) -> ThreadLanguage {
    let Some(hwnd_id) = hwnd else {
        return ThreadLanguage::UNKNOWN;
    };
    let mut pid = 0u32;
    // SAFETY: GetWindowThreadProcessId は非ブロッキングの読み取り API。無効な hwnd では 0 を返す。
    let tid = unsafe { GetWindowThreadProcessId(hwnd_id.to_hwnd(), Some(&raw mut pid)) };
    if tid == 0 || pid == std::process::id() {
        return ThreadLanguage {
            tid,
            ..ThreadLanguage::UNKNOWN
        };
    }
    // SAFETY: GetKeyboardLayout は任意のスレッドから呼べる読み取り専用 API。終了済みスレッドでは 0 を返す。
    let hkl = unsafe { GetKeyboardLayout(tid) };
    let hkl = hkl.0 as u32;
    ThreadLanguage {
        japanese: classify_layout_language(hkl),
        tid,
        lang_id: lang_id(hkl),
    }
}
