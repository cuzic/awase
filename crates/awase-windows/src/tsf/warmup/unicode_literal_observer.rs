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
//!
//! ## 観測専用の HIMC 照合ログ（2026-08-03 追加、判定には使わない）
//!
//! `Standard`/`ImmCross` プロファイル（LINE 等）のような IMM32 互換アプリでは、
//! `capture_composition_snapshot`（`ime.rs`）による HIMC 直接照合が、GJI write bytes より
//! 確実な確認手段になりうるという仮説がある。ただし本アプリ種別でこの照合を実機で試した
//! 記録は無く（既存の `log_composition_probe` 呼び出しはすべて `Vk`/`Tsf` モードの
//! per-VK confirm 経路 = `tsf/warmup/probe_fsm.rs` 等に限られ、`Unicode` モードの経路からは
//! 一度も呼ばれていなかった）、`comp_str`/`himc_null` が実際に何を返すか未知数だった。
//!
//! 過去に類似の HIMC ベース composition 検出（`check_tsf_composition_active`,
//! `ime.rs:1088`）を TSF ネイティブアプリ（WezTerm）に試みた実績があるが、
//! `ImmGetCompositionStringW` が常に 0 を返し失敗・撤回された
//! （`558c39f` → `b643bac`、2026-05-15、`git log -S check_tsf_composition_active`
//! で確認可能）。この失敗は HIMC を持たない TSF ネイティブアプリ限定であり、
//! LINE のような有効な HIMC を持つはずの IMM32 互換アプリでは未検証のまま。
//!
//! そこで `tick()` の判定確定点（GJI write の有無を確認した直後）で
//! `log_composition_probe` を1回だけ追加で呼び、`comp_str`/`himc_null` 等を
//! ログに残す。**`ProbeAction` の判定ロジック（GJI write baseline 比較）は一切変更しない**
//! ——この呼び出しは既存の判定結果に影響を与えない、純粋な観測のみ。
//! 実機ログが集まったら、`docs/experiments.md` の事前登録した合格基準と照合し、
//! HIMC 照合を実際の判定ロジックに採用するかを別途検討する。

use crate::state::event_origin::Generation;
use crate::tsf::warmup::probe_fsm::{ProbeAction, TsfEnvSnapshot};
use crate::tsf::warmup::tickable_fsm::TickableFsm;

/// Unicode 送信後の GJI 観測ウィンドウ (ms)。
const OBSERVATION_WINDOW_MS: u64 = 100;

/// Unicode モード文字送信後に GJI write を観測するプローブ FSM。
pub(crate) struct UnicodeLiteralObserverFsm {
    cold_seq: Generation,
    baseline_bytes: u64,
    elapsed_ms: u64,
}

impl UnicodeLiteralObserverFsm {
    /// `baseline_bytes` = 送信直前の `gji_write_bytes()` スナップショット。
    pub(crate) fn new(baseline_bytes: u64, cold_seq: Generation) -> Self {
        Self {
            cold_seq,
            baseline_bytes,
            elapsed_ms: 0,
        }
    }
}

impl TickableFsm for UnicodeLiteralObserverFsm {
    fn tick(&mut self, _env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.elapsed_ms += 10;
        if self.elapsed_ms < OBSERVATION_WINDOW_MS {
            return vec![];
        }
        let current = crate::tsf::observer::gji_write_bytes();
        // 観測専用: 判定（下記 if/else）には一切使わない。HIMC 照合が Standard/ImmCross
        // プロファイルで何を返すかを実機ログで確認するためだけの呼び出し（module doc参照）。
        crate::ime_diagnostic::log_composition_probe(self.cold_seq, "unicode-obs-himc-check");
        if current == self.baseline_bytes {
            tracing::info!(
                "[unicode-obs] cold={} {}ms GJI write なし → injection_mode Tsf 昇格",
                self.cold_seq.value(),
                self.elapsed_ms
            );
            vec![ProbeAction::UpgradeToTsf, ProbeAction::Done]
        } else {
            tracing::debug!(
                "[unicode-obs] cold={} GJI write 確認 (Δ={}) → Unicode 維持",
                self.cold_seq.value(),
                current.wrapping_sub(self.baseline_bytes)
            );
            vec![ProbeAction::Done]
        }
    }

    fn cold_seq_hint(&self) -> Generation {
        self.cold_seq
    }
}
