//! TSF warmup VK 送信ヘルパー。
//!
//! TSF cold-start ウォームアップで繰り返し使う「DBE VK DOWN + UP ペアを
//! SendInput で送信し、送信後の時刻を返す」操作を一本化する。

use super::output::make_tsf_key_input;

/// Win キー押下中かどうかを確認するローカルヘルパー。
fn win_key_held() -> bool {
    use crate::vk::{VK_LWIN, VK_RWIN};
    crate::hook::is_physical_key_down(VK_LWIN) || crate::hook::is_physical_key_down(VK_RWIN)
}

/// VK_DBE_HIRAGANA (F2) のキーダウン＋キーアップを SendInput で送信する。
///
/// 送信後の時刻（`current_tick_ms` の値）を返す。
pub(crate) fn send_vk_dbe_hiragana_pair() -> u64 {
    use crate::vk::VK_DBE_HIRAGANA;

    // Win キー押下中は送信をスキップする。
    // Win を押したまま VK_DBE_HIRAGANA を注入すると Win+F2 として届き、
    // Win↑ 時にスタートメニューが開く原因になる。
    if win_key_held() {
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

/// 英数モード用: `charset` に応じた DBE VK ペアを送信する。
///
/// - `ZenkakuAlpha`: `VK_DBE_ALPHANUMERIC` (F0) + `VK_DBE_DBCSCHAR` (F4) DOWN+UP
/// - `HankakuAlpha`: `VK_DBE_ALPHANUMERIC` (F0) DOWN+UP
///
/// 送信後の時刻（`current_tick_ms` の値）を返す。
pub(crate) fn send_vk_dbe_alpha_warmup(charset: awase::engine::Charset) -> u64 {
    use crate::vk::{VK_DBE_ALPHANUMERIC, VK_DBE_DBCSCHAR};

    if win_key_held() {
        log::debug!("[tsf-warmup] skipped alpha warmup (Win key held)");
        return crate::hook::current_tick_ms();
    }

    match charset {
        awase::engine::Charset::ZenkakuAlpha => {
            let inputs = [
                make_tsf_key_input(VK_DBE_ALPHANUMERIC, false),
                make_tsf_key_input(VK_DBE_ALPHANUMERIC, true),
                make_tsf_key_input(VK_DBE_DBCSCHAR, false),
                make_tsf_key_input(VK_DBE_DBCSCHAR, true),
            ];
            let _ = crate::win32::send_input_safe(&inputs);
        }
        _ => {
            let inputs = [
                make_tsf_key_input(VK_DBE_ALPHANUMERIC, false),
                make_tsf_key_input(VK_DBE_ALPHANUMERIC, true),
            ];
            let _ = crate::win32::send_input_safe(&inputs);
        }
    }
    crate::hook::current_tick_ms()
}

/// カタカナモード用: `charset` に応じた DBE VK ペアを送信する。
///
/// - `ZenkakuKatakana`: `VK_DBE_KATAKANA` (F1) DOWN+UP
/// - `HankakuKatakana`: `VK_DBE_KATAKANA` (F1) + `VK_DBE_SBCSCHAR` (F3) DOWN+UP
///
/// TSF composition context を初期化しつつカタカナモードを維持する。
/// `VK_DBE_HIRAGANA` はひらがなに戻してしまうため使ってはいけない。
///
/// 送信後の時刻（`current_tick_ms` の値）を返す。
pub(crate) fn send_vk_dbe_katakana_warmup(charset: awase::engine::Charset) -> u64 {
    use crate::vk::{VK_DBE_KATAKANA, VK_DBE_SBCSCHAR};

    if win_key_held() {
        log::debug!("[tsf-warmup] skipped katakana warmup (Win key held)");
        return crate::hook::current_tick_ms();
    }

    match charset {
        awase::engine::Charset::HankakuKatakana => {
            // F1↓F1↑ でカタカナ、F3↓F3↑ で半角に切り替え。
            let inputs = [
                make_tsf_key_input(VK_DBE_KATAKANA, false),
                make_tsf_key_input(VK_DBE_KATAKANA, true),
                make_tsf_key_input(VK_DBE_SBCSCHAR, false),
                make_tsf_key_input(VK_DBE_SBCSCHAR, true),
            ];
            let _ = crate::win32::send_input_safe(&inputs);
        }
        _ => {
            // ZenkakuKatakana その他: F1↓F1↑ のみ
            let inputs = [
                make_tsf_key_input(VK_DBE_KATAKANA, false),
                make_tsf_key_input(VK_DBE_KATAKANA, true),
            ];
            let _ = crate::win32::send_input_safe(&inputs);
        }
    }
    crate::hook::current_tick_ms()
}
