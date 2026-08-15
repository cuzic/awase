//! Chrome IME 向け cold-start warmup probe。
//!
//! [`TsfProbeCoro::new_chrome`] を [`TickableFsm`] トレイト経由で使えるようにラップする。

use crate::state::event_origin::Generation;
use crate::tsf::probe::{LiteralDetector, TsfReadinessProbe};
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{ProbeAction, TsfEnvSnapshot, TsfProbeCoro};
use crate::tsf::warmup::tickable_fsm::TickableFsm;

pub(crate) struct ChromeProbe(TsfProbeCoro);

impl ChromeProbe {
    pub(crate) fn new(
        romaji: &str,
        cold_seq: Generation,
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
    fn tick(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.0.tick(env)
    }

    fn cold_seq_hint(&self) -> Generation {
        self.0.cold_seq_hint()
    }

    fn apply_transmit_done(
        &mut self,
        romaji: String,
        ze_bs_count: usize,
        detector: Option<LiteralDetector>,
        literal_detect_ms: u64,
    ) -> bool {
        self.0
            .apply_transmit_done(romaji, ze_bs_count, detector, literal_detect_ms)
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
        // `TSF_OBS.gji_monitor_ok` はプロセス全体で共有される static のため、
        // `probe.rs`/`observer.rs`/`literal_detect_fsm.rs`/`probe_fsm.rs` と
        // 共有するロックで直列化する（BUG-65 追補4: この関数だけロックを持たず
        // 無施錠で書き換えており、`tsf::probe::tests::probe_fallback_waits_total_max_ms`
        // を実機で決定論的に誤検知させていた）。
        let _g = crate::tsf::observer::TSF_OBS_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::tsf::observer::TSF_OBS
            .gji_monitor_ok
            .store(true, std::sync::atomic::Ordering::SeqCst);
        crate::tsf::observer::reset_literal_session_confirmed();
        let guard = OutputActiveGuard::noop_for_test();
        // total_max_ms=0 → 最初の tick で probe.check_outcome が即 ready になる。
        let probe = TsfReadinessProbe::new(0, Generation::INITIAL, 0);
        let mut chrome_probe = ChromeProbe::new("ka", Generation::INITIAL, probe, 0, guard);

        let first_actions = chrome_probe.tick(TsfEnvSnapshot {
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

        let actions_after_apply = chrome_probe.tick(TsfEnvSnapshot {
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

    // ── apply_transmit_done 委譲の回帰テスト ────────────────────────────────
    //
    // `apply_vk_sent` と対称の委譲漏れリスクが `apply_transmit_done` にもある
    // （どちらも `ChromeProbe` が `TsfProbeCoro` へ委譲するオーバーライドで、
    // BUG-27 は前者でのみ実際に発生したが、後者の委譲が抜けても症状は同種になる:
    // `TickableFsm` のデフォルト no-op は常に `true`（＝この machine は Done 扱い）
    // を返すため、委譲が欠けると inline LiteralDetect フェーズへ絶対に継続しない）。
    // `TickableFsm::tickable_fsm.rs` のモジュール doc が要求する「ラップ型の
    // オーバーライドは対で回帰テストを持つ」規約に従い追加する。
    #[test]
    fn chrome_probe_apply_transmit_done_reaches_inner_coro() {
        let guard = OutputActiveGuard::noop_for_test();
        let probe = TsfReadinessProbe::new(0, Generation::INITIAL, 0);
        let mut chrome_probe = ChromeProbe::new("ka", Generation::INITIAL, probe, 0, guard);

        // `detector: Some(..)` を渡した場合、内側の `TsfProbeCoro::apply_transmit_done`
        // は `pending_transmit_done` に積んで `false`（＝まだ Done ではない、次 tick で
        // inline LiteralDetect に続く）を返す。`TickableFsm` のデフォルト no-op は
        // 引数に関わらず常に `true` を返すため、委譲が効いているかどうかは戻り値だけで
        // 判別できる。
        let is_done = chrome_probe.apply_transmit_done(
            "ka".to_string(),
            2,
            Some(LiteralDetector::new(false)),
            1000,
        );

        assert!(
            !is_done,
            "apply_transmit_done(detector=Some(..)) が内側の TsfProbeCoro に届いて \
             いれば pending_transmit_done に積んで false を返すはず。デフォルト \
             no-op（常に true）に落ちていないか確認: is_done={is_done}"
        );
    }
}
