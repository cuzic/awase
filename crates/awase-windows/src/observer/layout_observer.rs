#![allow(unsafe_code)]
//! ADR-223: フォーカス窓のスレッドの入力言語(HKL)を読む observer。

use crate::state::ime_event::HwndId;
use crate::state::layout_language::{classify_layout_language, lang_id};
use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetClassNameW, GetWindowThreadProcessId,
};

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

/// UWP のフレーム窓(`ApplicationFrameWindow`、`ApplicationFrameHost` のスレッド)か。
fn is_uwp_frame(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    // SAFETY: GetClassNameW は非ブロッキングで、バッファ長を上限に書き込む。
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    usize::try_from(n)
        .ok()
        .filter(|n| *n > 0)
        .is_some_and(|n| String::from_utf16_lossy(&buf[..n]) == "ApplicationFrameWindow")
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
    let mut hwnd = hwnd_id.to_hwnd();
    if is_uwp_frame(hwnd) {
        // UWP のフレームのスレッドの言語は、実際に入力を受けるアプリ側の `CoreWindow` のスレッドと食い違う
        // (ADR-223 段階 0 の実測: フレーム=ru のまま、CoreWindow=ja で、日本語のままの打鍵を非日本語と誤読した)。
        // 子の `CoreWindow` を引いて読み、見つからなければ「不明」にする。
        // SAFETY: FindWindowExW は非ブロッキングの列挙 API。見つからなければ Err。
        match unsafe { FindWindowExW(Some(hwnd), None, w!("Windows.UI.Core.CoreWindow"), None) } {
            Ok(core) if !core.0.is_null() => hwnd = core,
            _ => return ThreadLanguage::UNKNOWN,
        }
    }
    let mut pid = 0u32;
    // SAFETY: GetWindowThreadProcessId は非ブロッキングの読み取り API。無効な hwnd では 0 を返す。
    let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
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
