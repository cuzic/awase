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
/// 戻り値: 実際に注入した場合 `Some(送信時刻ms)`（`current_tick_ms` の値）。
/// Win キー押下中でスキップした場合 `None`。
///
/// **呼び出し元は `None` を「送信していない」として扱うこと** — スキップを送信成功
/// 扱いで `eager_warmup_sent_ms` にラッチすると、この F2 が「物理 F2 キーの代替」
/// （`PhysicalKeyDisposition::plan` が物理キーを Suppress した埋め合わせ）である
/// ケースで、GJI に IME-ON 信号が一度も届かないまま belief だけ ON 確定してしまう。
/// `crate::ime::send_ime_mode_key` の BUG-16 追補（2026-07-07）と同型の欠陥
/// （`docs/known-bugs.md` BUG-32 参照）。
#[must_use]
pub(crate) fn send_vk_dbe_hiragana_pair() -> Option<u64> {
    use crate::vk::VK_DBE_HIRAGANA;

    // Win キー押下中は送信をスキップする。
    // Win を押したまま VK_DBE_HIRAGANA を注入すると Win+F2 として届き、
    // Win↑ 時にスタートメニューが開く原因になる。
    if win_key_held() {
        log::debug!("[tsf-warmup] skipped VK_DBE_HIRAGANA (Win key held)");
        return None;
    }

    let inputs = [
        make_tsf_key_input(VK_DBE_HIRAGANA, false),
        make_tsf_key_input(VK_DBE_HIRAGANA, true),
    ];
    let _ = crate::win32::send_input_safe(&inputs);
    Some(crate::hook::current_tick_ms())
}
