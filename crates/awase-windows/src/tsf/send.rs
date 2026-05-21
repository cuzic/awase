//! VK_DBE_HIRAGANA 送信ヘルパー。
//!
//! TSF cold-start ウォームアップで繰り返し使う「VK_DBE_HIRAGANA DOWN + UP ペアを
//! SendInput で送信し、送信後の時刻を返す」操作を一本化する。

use std::mem::size_of;

use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};

use super::output::make_tsf_key_input;

/// VK_DBE_HIRAGANA (F2) のキーダウン＋キーアップを SendInput で送信する。
///
/// 送信後の時刻（`current_tick_ms` の値）を返す。
///
/// # Safety
/// `SendInput` を呼ぶため unsafe。呼び出し元はメッセージループスレッド上から呼ぶこと。
pub(crate) unsafe fn send_vk_dbe_hiragana_pair() -> u64 {
    const VK_DBE_HIRAGANA: u16 = 0xF2;
    let inputs = [
        make_tsf_key_input(VK_DBE_HIRAGANA, false),
        make_tsf_key_input(VK_DBE_HIRAGANA, true),
    ];
    // SAFETY: inputs は呼び出し中に有効な INPUT 配列。
    SendInput(
        &inputs,
        i32::try_from(size_of::<INPUT>()).expect("INPUT size fits in i32"),
    );
    crate::hook::current_tick_ms()
}
