//! Chrome GJI 再初期化 FSM。
//!
//! F22→F21 送信後、`IMC_GETCONVERSIONMODE` が Hiragana を返すまで（最大 300ms）待機し、
//! 確認後に実ローマ字を再送する。
//!
//! ## 動作フロー
//!
//! 1. `dispatch_probe_actions` が `StartChromeGjiReinit` を受け取る
//! 2. F22→F21 を SendInput でキューイング + `ImeModeFsm` belief 更新 + async IMC ポーリング開始
//! 3. 本 FSM が 10ms ごとに `env.ime_mode == Hiragana && env.ime_mode_confirmed` を確認
//! 4. Hiragana 確認 or タイムアウト → `SacrificialResend` emit → 実ローマ字送信

use crate::tsf::ime_mode_fsm::ImeModeState;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::probe_fsm::{DeferredVk, ProbeAction, SacrificialResend, TransmitTarget, TsfEnvSnapshot};
use awase::types::VkCode;

/// Chrome GJI 再初期化後の実ローマ字再送 FSM。
pub(crate) struct ChromeGjiReinitFsm {
    cold_seq: u32,
    _guard: OutputActiveGuard,
    romaji: String,
    deferred_vks: Vec<DeferredVk>,
    /// タイムアウト絶対時刻（ms）
    deadline_ms: u64,
}

impl ChromeGjiReinitFsm {
    pub(crate) fn new(
        cold_seq: u32,
        romaji: String,
        deferred_vks: Vec<DeferredVk>,
        deadline_ms: u64,
    ) -> Self {
        Self {
            cold_seq,
            _guard: OutputActiveGuard::begin(),
            romaji,
            deferred_vks,
            deadline_ms,
        }
    }

    fn tick_inner(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        let now = crate::hook::current_tick_ms();
        let ime_ready = env.ime_mode == ImeModeState::Hiragana && env.ime_mode_confirmed;
        let timed_out = now >= self.deadline_ms;

        if !ime_ready && !timed_out {
            return vec![]; // 待機中
        }

        if ime_ready {
            log::info!(
                "[chrome-reinit] cold={} IME Hiragana 確認 → 実ローマ字 {:?} 再送",
                self.cold_seq, self.romaji,
            );
        } else {
            log::warn!(
                "[chrome-reinit] cold={} タイムアウト → 強制再送 {:?}",
                self.cold_seq, self.romaji,
            );
        }

        let romaji = std::mem::take(&mut self.romaji);
        let deferred_vks = std::mem::take(&mut self.deferred_vks);
        vec![
            ProbeAction::SacrificialResend(SacrificialResend {
                cold_seq: self.cold_seq,
                romaji,
                deferred_vks,
                target: TransmitTarget::Chrome,
                // VK_A は cold だった（reinit 完了後の再送）。
                // SacrificialResend Chrome ハンドラは confirmed_warm の値によらず
                // transmit_chrome を直接呼ぶ（StartChromeGjiReinit で F22→F21 は送信済み）。
                confirmed_warm: false,
            }),
            ProbeAction::Done,
        ]
    }
}

impl crate::tsf::tickable_fsm::TickableFsm for ChromeGjiReinitFsm {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.tick_inner(env)
    }

    fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }

    fn push_deferred(&mut self, vk: VkCode, needs_shift: bool) {
        self.deferred_vks.push(DeferredVk { vk, needs_shift });
    }
}
