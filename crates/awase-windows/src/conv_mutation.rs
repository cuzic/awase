//! conv-mode ワード（NATIVE/KATAKANA/FULLSHAPE/ROMAN、`imm.rs::IME_CMODE_*`）を
//! 変えうる書き込みだけを数える単調カウンタ（BUG-34 横展開 Step0-a）。
//!
//! # なぜ要るのか
//!
//! BUG-34 の完了 fence（`runtime/key_pipeline.rs` の idle-conv-check 等）は元々
//! `Output::send_keys` が冒頭・末尾で呼ぶ `mark_send()`（`output/mod.rs`）由来の
//! `output_in_flight_ms()`/`last_send` を「自己出力なし」の根拠に使っていたが、
//! これは二重に誤っていた:
//!
//! 1. **過剰**: `mark_send()` は NICOLA の通常の文字出力（conv を一切変えない）
//!    でも呼ばれるため、打鍵のたびに fence が誤って落ちる。
//! 2. **不足**: `send_eager_tsf_warmup` が呼ぶ `send_eager_warmup_vk_pair`
//!    （`tsf/send.rs`）は `win32::send_input_safe` を直接叩き `mark_send` を
//!    一切通らないため、fence が本来捕捉すべき「awase 自身の warmup 書き込み」
//!    を1つも検出できていなかった。
//!
//! このモジュールは `mark_send`/`output_in_flight_ms` を置き換える専用カウンタで、
//! **conv ワードを実際に変えうる VK を送るときだけ**増分する（`vk::vk_may_mutate_conv`
//! が判定、`win32::send_input_safe` が唯一のゲート）。open 専用の VK（VK_IME_ON/
//! OFF/KANJI）や、conv を一切変えない通常の文字出力では増分しない。
//!
//! # スレッド安全性
//!
//! `send_input_safe` はエンジンスレッドから直接呼ばれる場合と、
//! `win32_async::offload` 経由でワーカースレッドから呼ばれる場合の両方が
//! あるため、`send_health` と同じ理由で `Atomic` を使う。

use std::sync::atomic::{AtomicU64, Ordering};

/// 0 は「まだ一度も観測していない」ことを表すセンチネルとして予約する
/// （`send_health` 等、他の fence 実装と同じ規約）ため 1 から始める。
static CONV_MUTATION_SEQ: AtomicU64 = AtomicU64::new(1);

/// conv ワードを変えうる書き込みが発生したことを記録する。
///
/// ゲートは2箇所ある(BUG-34 横展開レビュー指摘、当初は send_input_safe のみで
/// IMC write 経路の捕捉漏れがあった):
/// - `win32::send_input_safe`（SendInput 経由、VK_DBE_*/VK_KANA/VK_CONVERT）
/// - `imm::send_ime_control`（IMC_SETCONVERSIONMODE、`set_ime_romaji_mode_for_hwnd`
///   等が使う IMC write 経路）
///
/// 個々の呼び出しごとではなく、1回のバッチ/書き込みにつき高々1回増分すれば
/// 十分（fence は「変わったか」を見るだけで、正確な変異回数を数える必要はない）。
pub(crate) fn bump() {
    CONV_MUTATION_SEQ.fetch_add(1, Ordering::Relaxed);
}

/// 現在の値を読む。fence の spawn 時スナップショットと apply 時の再読み取りを
/// 比較するために使う（ビット一致で判定すること、経過時間で判定しない）。
pub(crate) fn current() -> u64 {
    CONV_MUTATION_SEQ.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_advances_current_monotonically() {
        let before = current();
        bump();
        let after = current();
        assert!(after > before, "bump() は current() を単調に進める");
    }
}
