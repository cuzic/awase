//! TSF warmup VK 送信ヘルパー。
//!
//! TSF cold-start ウォームアップで繰り返し使う「warmup VK の DOWN + UP ペアを
//! SendInput で送信し、送信後の時刻を返す」操作を一本化する。

use super::output::make_tsf_key_input;

/// eager TSF warmup の VK（`VK_IME_ON`）のキーダウン＋キーアップを SendInput で送信する。
///
/// **2026-08-22、ADR-100 決定2（群B 実機検証）により `VK_DBE_HIRAGANA` から
/// `VK_IME_ON` へ変更した。** `VK_DBE_HIRAGANA` は「開く」と「ひらがなに強制する」を
/// 1つの副作用に束ねており（BUG-50 デッドロックの前提）、`VK_IME_ON` は open 軸のみを
/// 触る冪等キーのため、この副作用が構造的に消える。送信形態（scan は
/// `make_tsf_key_input` が都度 `MapVirtualKeyW` で算出、`TSF_MARKER`）は変更していない
/// （`docs/adr/100-gji-warmup-vk-ime-on-reinit.md` F17 で `MapVirtualKeyW(VK_IME_ON,
/// MAPVK_VK_TO_VSC)` が非ゼロ (`0xF2`) であることを実機確認済み）。
///
/// 戻り値: 実際に注入した場合 `Some(送信時刻ms)`（`current_tick_ms` の値）。
/// Win キー押下中でスキップした場合 `None`。
///
/// **呼び出し元は `None` を「送信していない」として扱うこと** — スキップを送信成功
/// 扱いで `eager_warmup_sent_ms` にラッチすると、この warmup が「物理 F2 キーの代替」
/// （`PhysicalKeyDisposition::plan` が物理キーを Suppress した埋め合わせ）である
/// ケースで、GJI に IME-ON 信号が一度も届かないまま belief だけ ON 確定してしまう。
/// `crate::ime::send_ime_mode_key` の BUG-16 追補（2026-07-07）と同型の欠陥
/// （`docs/known-bugs.md` BUG-32 参照）。
#[must_use]
pub(crate) fn send_eager_warmup_vk_pair() -> Option<u64> {
    use crate::vk::VK_IME_ON;

    // Win キー押下中は送信をスキップする。
    // Win を押したまま IME モードキーを注入すると Win+key として届き、
    // Win↑ 時にスタートメニューが開く原因になる。
    if crate::hook::win_key_held() {
        tracing::debug!("[tsf-warmup] skipped VK_IME_ON (Win key held)");
        return None;
    }

    let inputs = [
        make_tsf_key_input(VK_IME_ON, false),
        make_tsf_key_input(VK_IME_ON, true),
    ];
    let _ = crate::win32::send_input_safe(&inputs);
    Some(crate::hook::current_tick_ms())
}
