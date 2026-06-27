//! VK_DBE_HIRAGANA 送信ヘルパー。
//!
//! TSF cold-start ウォームアップで繰り返し使う「VK_DBE_HIRAGANA DOWN + UP ペアを
//! SendInput で送信し、送信後の時刻を返す」操作を一本化する。

use super::output::make_tsf_key_input;

/// VK_DBE_HIRAGANA (F2) のキーダウン＋キーアップを SendInput で送信する。
///
/// 送信後の時刻（`current_tick_ms` の値）を返す。
pub(crate) fn send_vk_dbe_hiragana_pair() -> u64 {
    use crate::vk::{VK_DBE_HIRAGANA, VK_LWIN, VK_RWIN};

    // Win キー押下中は送信をスキップする。
    // Win を押したまま VK_DBE_HIRAGANA を注入すると Win+F2 として届き、
    // Win↑ 時にスタートメニューが開く原因になる。
    if crate::hook::is_physical_key_down(VK_LWIN) || crate::hook::is_physical_key_down(VK_RWIN) {
        log::debug!("[tsf-warmup] skipped VK_DBE_HIRAGANA (Win key held)");
        return crate::hook::current_tick_ms();
    }

    let inputs = [
        make_tsf_key_input(VK_DBE_HIRAGANA, false),
        make_tsf_key_input(VK_DBE_HIRAGANA, true),
    ];
    let _ = crate::win32::send_input_safe(&inputs);
    crate::hook::current_tick_ms()
}
