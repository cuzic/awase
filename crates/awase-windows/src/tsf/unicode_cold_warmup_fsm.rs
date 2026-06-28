//! Unicode cold-start GJI 起動待ちウォームアップ FSM。
//!
//! ## 動作フロー
//!
//! 1. `Platform::dispatch_gji_response` が `GjiAction::StartProbe { is_long_cold: true }` を
//!    受信し、Unicode モード + deferred chars がある場合に本 FSM をインストールする。
//! 2. 呼び出し元が VK_IME_ON (0x16) を送信して GJI 起動をポークする。
//! 3. 本 FSM が 10ms ごとに `gji_write_bytes()` を監視する。
//! 4. GJI が write した（`gji_write_bytes` 増加）か `WARMUP_TIMEOUT_MS` 経過したら
//!    [`ProbeAction::FlushDeferredUnicodeChars`] を emit して完了する。
//! 5. dispatcher が各文字を `send_unicode_char_direct()` で送信する。

use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::probe_fsm::{ProbeAction, TsfEnvSnapshot};
use awase::types::VkCode;

/// GJI が write するか、このミリ秒以上経過したら deferred chars を送信する。
const WARMUP_TIMEOUT_MS: u64 = 200;

/// Unicode cold-start warm-up FSM。
///
/// VK_IME_ON 送信後に GJI の起動を確認（`gji_write_bytes` 増加）してから deferred chars を送る。
pub(crate) struct UnicodeColdWarmupFsm {
    cold_seq: u32,
    /// RAII guard — Drop で `OUTPUT_GATE.active=false`（後続キーを INPUT_DEFER に退避）
    _guard: OutputActiveGuard,
    /// VK_IME_ON 送信前に取得した `gji_write_bytes()` ベースライン
    baseline_bytes: u64,
    /// GJI が warm になったら送信する Unicode 文字バッファ
    deferred_chars: Vec<char>,
    /// 累積経過時間 (ms)
    elapsed_ms: u64,
}

impl UnicodeColdWarmupFsm {
    pub(crate) fn new(cold_seq: u32, deferred_chars: Vec<char>, baseline_bytes: u64) -> Self {
        log::debug!(
            "[unicode-cold-warmup] cold={cold_seq} FSM 開始: {} chars deferred, baseline_bytes={baseline_bytes}",
            deferred_chars.len()
        );
        Self {
            cold_seq,
            _guard: OutputActiveGuard::begin(),
            baseline_bytes,
            deferred_chars,
            elapsed_ms: 0,
        }
    }

    fn tick_inner(&mut self, _env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.elapsed_ms += 10;
        let current = crate::tsf::observer::gji_write_bytes();
        let gji_wrote = current > self.baseline_bytes;
        let timed_out = self.elapsed_ms >= WARMUP_TIMEOUT_MS;

        if !gji_wrote && !timed_out {
            return vec![];
        }

        let chars = std::mem::take(&mut self.deferred_chars);
        log::debug!(
            "[unicode-cold-warmup] cold={} gji_wrote={gji_wrote} timed_out={timed_out} \
             elapsed={}ms → {} chars 送信",
            self.cold_seq, self.elapsed_ms, chars.len()
        );
        vec![ProbeAction::FlushDeferredUnicodeChars(chars), ProbeAction::Done]
    }
}

impl crate::tsf::tickable_fsm::TickableFsm for UnicodeColdWarmupFsm {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.tick_inner(env)
    }

    fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }

    fn push_deferred(&mut self, _vk: VkCode, _needs_shift: bool) {
        // Unicode cold-start 中に VK が届くことはまれ。
        // 通常は OUTPUT_GATE.active=true により INPUT_DEFER に退避される。
    }
}
