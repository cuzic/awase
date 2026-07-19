//! Chrome IME 向け cold-start warmup probe。
//!
//! [`TsfProbeCoro::new_chrome`] を [`TickableFsm`] トレイト経由で使えるようにラップする。

use crate::tsf::probe::{LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{ProbeAction, TsfEnvSnapshot, TsfProbeCoro};
use crate::tsf::warmup::tickable_fsm::TickableFsm;

pub(crate) struct ChromeProbe(TsfProbeCoro);

impl ChromeProbe {
    pub(crate) fn new(
        romaji: &str,
        cold_seq: u32,
        probe: TsfReadinessProbe,
        total_max_ms: u64,
        guard: OutputActiveGuard,
    ) -> Self {
        Self(TsfProbeCoro::new_chrome(
            romaji,
            cold_seq,
            probe,
            total_max_ms,
            guard,
        ))
    }
}

impl TickableFsm for ChromeProbe {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.0.tick(env)
    }

    fn cold_seq_hint(&self) -> u32 {
        self.0.cold_seq_hint()
    }

    fn apply_transmit_done(
        &mut self,
        romaji: String,
        ze_bs_count: usize,
        detector: Option<LiteralDetector>,
        literal_detect_ms: u64,
        expected_kana: Option<char>,
    ) -> bool {
        self.0.apply_transmit_done(
            romaji,
            ze_bs_count,
            detector,
            literal_detect_ms,
            expected_kana,
        )
    }

    // BUG-27 根本原因（2026-07-17）: この委譲が抜けていたため、Chrome per-VK confirm
    // の `TransmitSingleVk` 処理で
    // `dispatch_probe_actions` が呼ぶ `machine.apply_vk_sent(...)` が
    // `TickableFsm` のデフォルト no-op（`tickable_fsm.rs`）に落ちていた。内側の
    // `TsfProbeCoro::apply_vk_sent` が一度も呼ばれないため `pending_vk_sent` が
    // 常に `None` のままで、次 tick で per-VK ループが「vk_sent 未設定」として
    // 中断していた（毎回・確実に再現、レースではない）。VK自体は
    // `dispatch_probe_actions` 側で物理送信済みのため、romaji の1文字目のVKだけが
    // literal として画面に残り2文字目が送られない、という症状になっていた
    // （docs/known-bugs.md BUG-27 追補3参照）。
    fn apply_vk_sent(&mut self, detector: LiteralDetector, deadline_ms: u64) {
        self.0.apply_vk_sent(detector, deadline_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── BUG-27 根本原因の回帰テスト ──────────────────────────────────────────
    //
    // `ChromeProbe` 経由（本番の `pending_tsf: Box<dyn TickableFsm>` と同じ呼び出し
    // 経路）で `apply_vk_sent` を呼んだとき、内側の `TsfProbeCoro` に実際に
    // 届いているかを確認する。委譲が欠けていた旧実装では、この呼び出しが
    // `TickableFsm` のデフォルト no-op に落ちて `pending_vk_sent` が更新されず、
    // 次 tick で per-VK confirm ループが「vk_sent 未設定」として即 `Done` を
    // 返していた（実機で romaji 2文字目が毎回失われる不具合になった）。
    //
    // `probe_fsm.rs::tests::chrome_per_vk_vk_sent_unset_does_not_backspace` は
    // `TsfProbeCoro` を直接構築するため、この `ChromeProbe` の委譲漏れ自体は
    // 検出できなかった（テストが通っていたのに実機では毎回再現した理由）。
    #[test]
    fn chrome_probe_apply_vk_sent_reaches_inner_coro() {
        crate::tsf::observer::TSF_OBS
            .gji_monitor_ok
            .store(true, std::sync::atomic::Ordering::SeqCst);
        crate::tsf::observer::reset_literal_session_confirmed();
        let guard = OutputActiveGuard::noop_for_test();
        // total_max_ms=0 → 最初の tick で probe.check_outcome が即 ready になる。
        let probe = TsfReadinessProbe::new(0, 0, 0);
        let mut chrome_probe = ChromeProbe::new("ka", 0, probe, 0, guard);

        let first_actions = chrome_probe.tick(&TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });
        assert!(
            matches!(
                first_actions.as_slice(),
                [ProbeAction::TransmitSingleVk { .. }]
            ),
            "per-VK confirm ループの最初の VK 送信要求のはず: {first_actions:?}"
        );

        // 本番の dispatch_probe_actions と同じく、TickableFsm トレイト経由で
        // apply_vk_sent を呼ぶ（ここが ChromeProbe の委譲を経由する）。
        let deadline_ms = crate::hook::current_tick_ms() + 1000;
        chrome_probe.apply_vk_sent(LiteralDetector::new(false), deadline_ms);

        let actions_after_apply = chrome_probe.tick(&TsfEnvSnapshot {
            gji_active: true,
            ..Default::default()
        });

        // 委譲が効いていれば pending_vk_sent が Some として消費され、
        // detection 待ちの polling ループ（空の Vec）に入る。
        // 委譲が欠けていた旧実装では即座に `[ProbeAction::Done]` を返していた
        // （「vk_sent 未設定」の無リカバリ return）。
        assert!(
            actions_after_apply.is_empty(),
            "apply_vk_sent が内側の TsfProbeCoro に届いていれば detection 待ちの \
             polling ループに入り空の Vec を返すはず。'vk_sent 未設定' で即 Done に \
             なっていないか確認: {actions_after_apply:?}"
        );
    }
}
