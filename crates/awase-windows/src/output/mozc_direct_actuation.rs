#![allow(unsafe_code)] // Win32 の PostMessageW/RegisterWindowMessageW 呼び出しに unsafe が必須
#![allow(dead_code)]
// EXPERIMENTAL（awase exp/mozc-direct-actuation、未マージ）: まだ runtime/ から配線していない
//! EXPERIMENTAL（awase `exp/mozc-direct-actuation`、未マージ）: Mozc Readable の直接actuationコマンド
//! （version 6、未リリース。`readable/docs/shared-memory.md` 6-5 節）を送る殻。
//!
//! 送った結果の確認（ack の待ち受け・再送・諦めの判断）は純粋関数
//! [`crate::state::direct_actuation_plan`] が担う。ここは `PostMessage` を 1 回呼ぶだけ。
//!
//! まだ `runtime/` のどこからも呼ばれていない（実験の次段）。呼び出し元を配線するときは、既存の
//! actuation 合流点（`fix-requires-evidence.md` の「IME actuation 合流点」表）を監査し、
//! press-identity（ADR-208 L1、`state/press_ledger.rs`）を他の経路と同じように配線すること。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, RegisterWindowMessageW};

/// 登録ウィンドウメッセージの名前（mozc-readable `src/base/const.h` の
/// `kDirectActuationMessageName` と同じ文字列。どちらの `GOOGLE_JAPANESE_INPUT_BUILD` 分岐でも
/// `mozc-readable` 側にしか効かないので、この実験ではビルド分岐を 1 つだけ想定する）。
const DIRECT_ACTUATION_MESSAGE_NAME: &str = "mozcreadable.direct_actuation.message";

/// `nonce` は呼び出し元が管理する（[`crate::state::direct_actuation_plan::next_nonce`]）。0 は
/// 「要求なし」の意味なので渡してはならない（呼び出し元の責任。ここでは検査しない —— 殻は薄く保つ）。
///
/// 戻り値は `PostMessage` 自体が成功したか（**要求が実際に処理されたかではない**。それは
/// 共有メモリの `last_cmd_nonce`/`last_cmd_result` を読んで確認する、閉ループの本体）。
#[must_use]
pub(crate) fn post_command(cmd_window_handle: u64, nonce: u32, desired_open: bool) -> bool {
    debug_assert_ne!(
        nonce, 0,
        "nonce 0 は「要求なし」の意味なので送ってはならない"
    );
    if cmd_window_handle == 0 {
        return false;
    }
    let name = windows::core::HSTRING::from(DIRECT_ACTUATION_MESSAGE_NAME);
    // SAFETY: 文字列リテラルから作った有効な UTF-16 文字列を渡すだけ。失敗時は 0 を返す（登録自体は失敗しない想定）。
    let message = unsafe { RegisterWindowMessageW(&name) };
    if message == 0 {
        return false;
    }
    // bit0 だけが意味を持つ（bit1〜31 は予約、常に 0 のまま送る。`readable/docs/shared-memory.md` 6-5 節）。
    let lparam: u32 = u32::from(desired_open);
    let hwnd = HWND(cmd_window_handle as *mut core::ffi::c_void);
    // SAFETY: `hwnd` は呼び出し元が共有メモリから読んだ値。無効な HWND でも `PostMessageW` は
    //         `Err` を返すだけで安全（クラッシュしない）。
    unsafe {
        PostMessageW(
            Some(hwnd),
            message,
            WPARAM(nonce as usize),
            LPARAM(lparam as isize),
        )
    }
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_command_window_handle_is_rejected_without_calling_into_win32() {
        // cmd_window_handle == 0 は「まだ窓が無い」(readable/docs/shared-memory.md 6-5 節)なので、
        // Win32 を呼ばずに false を返す。
        assert!(!post_command(0, 1, true));
    }
}
