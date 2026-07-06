//! Unicode モード送信後の GJI write 観測 FSM。
//!
//! Unicode (KEYEVENTF_UNICODE) で文字を送った後、GJI が write を行ったかどうかを監視する。
//! 標準 IMM32 アプリなら IME が composition 経由で GJI に書き込む。
//! TSF text store 専用アプリ（Windows Terminal 等）は Unicode 送信を TSF 経由で受け取らないため
//! GJI write が起きない → injection_mode を Tsf に昇格すべきと判断する。
//!
//! ## 使い方
//!
//! 1. Unicode 送信直後に `UnicodeLiteralObserverFsm::new(baseline_bytes, cold_seq)` を生成し
//!    `pending_tsf` にインストールする（`Output::request_unicode_observation()` 経由）。
//! 2. `TIMER_TSF_PROBE` が tick するたびに `elapsed_ms` が増える。
//! 3. `OBSERVATION_WINDOW_MS` に達したとき:
//!    - GJI write あり → `ProbeAction::Done`（Unicode 維持）
//!    - GJI write なし → `ProbeAction::UpgradeToTsf` + `ProbeAction::Done`

use crate::tsf::warmup::probe_fsm::{ProbeAction, TsfEnvSnapshot};
use crate::tsf::warmup::tickable_fsm::TickableFsm;

/// Unicode 送信後の GJI 観測ウィンドウ (ms)。
const OBSERVATION_WINDOW_MS: u64 = 100;

/// Unicode モード文字送信後に GJI write を観測するプローブ FSM。
pub(crate) struct UnicodeLiteralObserverFsm {
    cold_seq: u32,
    baseline_bytes: u64,
    elapsed_ms: u64,
}

impl UnicodeLiteralObserverFsm {
    /// `baseline_bytes` = 送信直前の `gji_write_bytes()` スナップショット。
    pub(crate) fn new(baseline_bytes: u64, cold_seq: u32) -> Self {
        Self { cold_seq, baseline_bytes, elapsed_ms: 0 }
    }
}

impl TickableFsm for UnicodeLiteralObserverFsm {
    fn tick(&mut self, _env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.elapsed_ms += 10;
        if self.elapsed_ms < OBSERVATION_WINDOW_MS {
            return vec![];
        }
        let current = crate::tsf::observer::gji_write_bytes();
        if current == self.baseline_bytes {
            log::info!(
                "[unicode-obs] cold={} {}ms GJI write なし → injection_mode Tsf 昇格",
                self.cold_seq, self.elapsed_ms
            );
            vec![ProbeAction::UpgradeToTsf, ProbeAction::Done]
        } else {
            log::debug!(
                "[unicode-obs] cold={} GJI write 確認 (Δ={}) → Unicode 維持",
                self.cold_seq,
                current.wrapping_sub(self.baseline_bytes)
            );
            vec![ProbeAction::Done]
        }
    }

    fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }
}
