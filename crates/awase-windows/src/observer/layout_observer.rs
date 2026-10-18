#![allow(unsafe_code)]
//! ADR-223: フォーカス窓のスレッドの入力言語(HKL)を読む observer。

use crate::state::ime_event::HwndId;
use crate::state::layout_language::{classify_layout_language, lang_id};
use crate::win32::HwndIdExt;
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
/// `hwnd` は、既存の非同期のフォーカス解決(`GetGUIThreadInfo`)が確定した実際のフォーカス窓であること。
/// UWP でもフォーカス窓のスレッドが実際の入力先なので、フレーム窓(`ApplicationFrameWindow`)を子の `CoreWindow` に読み替えない
/// (WinEvent の最後の hwnd を使っていた間は、フレームの古い言語を読む問題があったが、読む窓を変えて不要になった)。
///
/// どちらの API も非ブロッキングの読み取りで、フックを待たせない(`GetGUIThreadInfo` は打鍵ごとには呼ばない)。
#[must_use]
pub fn read_thread_language(hwnd: Option<HwndId>) -> ThreadLanguage {
    let Some(hwnd_id) = hwnd else {
        return ThreadLanguage::UNKNOWN;
    };
    let hwnd = hwnd_id.to_hwnd();
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
