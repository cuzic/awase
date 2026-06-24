//! TSF cold-start 犠牲キーウォームアップ FSM。
//!
//! [`GjiWarmupFsm`] が `needs_literal=true` と判断した場合、
//! 実ローマ字を即送信する代わりに VK_A（犠牲キー）を送信して TSF 暖機を確認する。
//!
//! ## 動作フロー
//!
//! 1. `dispatch_probe_actions` が [`ProbeAction::StartSacrificialWarmup`] を受け取る
//! 2. VK_A を送信（犠牲キー。TSF warm なら 'あ' 形成、cold なら 'a' リテラル）
//! 3. 本 FSM が 10ms ごとに composition 状態を確認
//! 4. 判定完了（composition 確認 or タイムアウト）
//! 5. [`ProbeAction::SacrificialResend`] を emit → dispatcher が BS×1 + 実ローマ字再送
//!
//! ## 利点
//!
//! 実ローマ字が readline バッファにリテラル状態で残らないため、
//! ユーザーが判定待機中に Enter を押しても literal テキストが Submit されない。
//! 「Engine ON / IME OFF（TSF cold）」状態を構造的に排除する。

use crate::tsf::probe::LiteralDetector;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::probe_fsm::{DeferredVk, ProbeAction, SacrificialResend, TransmitTarget};
use crate::tsf::probe_fsm::TsfEnvSnapshot;
use awase::types::VkCode;

/// TSF cold-start 犠牲キー暖機 FSM。
///
/// 構築後は 10ms ごとに [`tick`](SacrificialWarmupFsm::tick) を呼ぶ。
/// `Done` を含む Vec が返ったらタイマーを停止する。
pub(crate) struct SacrificialWarmupFsm {
    cold_seq: u32,
    /// RAII guard — drop で `OUTPUT_GATE.active=false`
    _guard: OutputActiveGuard,
    /// 送信すべき実ローマ字（SacrificialResend ペイロード用）
    romaji: String,
    /// probe 中に蓄積した後続 VK（実ローマ字の後に送信する）
    deferred_vks: Vec<DeferredVk>,
    /// composition 確認 / literal 検出器（VK_A の composition を確認する）
    detector: LiteralDetector,
    /// 暖機判定タイムアウト絶対時刻（ms）
    deadline_ms: u64,
    /// 再送先ターゲット（Chrome / TSF）
    target: TransmitTarget,
}

impl SacrificialWarmupFsm {
    /// `SacrificialWarmupFsm` を生成する。
    ///
    /// VK_A はこのコンストラクタが呼ばれる前に `dispatch_probe_actions` 側で送信済み。
    /// 本 FSM は composition 確認の待機のみを担当する。
    pub(crate) fn new(
        cold_seq: u32,
        romaji: String,
        deferred_vks: Vec<DeferredVk>,
        literal_detect_ms: u64,
        target: TransmitTarget,
        write_bytes_before_vk_a: u64,
    ) -> Self {
        let guard = OutputActiveGuard::begin();
        // Chrome: VK_A 送信前に取得したベースラインを使って cold/warm を区別する。
        //   cold リテラル 'a' → GJI write ≈ +300B < 350B 閾値 → 不検出 → timeout
        //   warm コンポジション 'あ' → GJI write ≈ +400B > 350B 閾値 → confirmed
        // VK_A 送信後にベースラインを取得すると cold の write がベースラインに吸収されてしまう
        // ため、呼び出し元（probe_io.rs）で VK_A 送信前に取得し引数で渡す。
        // TSF/WezTerm: gji_candidate_show（候補ウィンドウ出現）で確認する。
        let detector = match target {
            TransmitTarget::Chrome => {
                LiteralDetector::new_gji_resumed_with_pre_send_baseline(write_bytes_before_vk_a)
            }
            TransmitTarget::Tsf => LiteralDetector::new(),
        };
        let deadline_ms = crate::hook::current_tick_ms() + literal_detect_ms;
        Self {
            cold_seq,
            _guard: guard,
            romaji,
            deferred_vks,
            detector,
            deadline_ms,
            target,
        }
    }

    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。
    ///
    /// VK_A の composition を確認次第（成功・タイムアウトいずれも）[`ProbeAction::SacrificialResend`] を emit する。
    pub(crate) fn tick(&mut self, _env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        let Some(detection) = self.detector.check_now(self.deadline_ms) else {
            return vec![];
        };

        use crate::tsf::probe::DetectionResult;
        let confirmed_warm = matches!(detection, DetectionResult::CompositionConfirmed);
        log::debug!(
            "[sacr-warmup] cold={} VK_A 判定={} → BS×1 + 実ローマ字 {:?} 再送",
            self.cold_seq,
            if confirmed_warm { "composition-confirmed (TSF warm)" } else { "timeout (TSF still cold)" },
            self.romaji,
        );
        crate::ime_diagnostic::log_composition_probe(
            self.cold_seq,
            if confirmed_warm { "sacr-warm" } else { "sacr-timeout" },
        );

        let romaji = std::mem::take(&mut self.romaji);
        let deferred_vks = std::mem::take(&mut self.deferred_vks);
        vec![
            ProbeAction::SacrificialResend(SacrificialResend {
                cold_seq: self.cold_seq,
                romaji,
                deferred_vks,
                target: self.target,
            }),
            ProbeAction::Done,
        ]
    }
}

impl crate::tsf::tickable_fsm::TickableFsm for SacrificialWarmupFsm {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        SacrificialWarmupFsm::tick(self, env)
    }

    fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }

    fn push_deferred(&mut self, vk: VkCode, needs_shift: bool) {
        self.deferred_vks.push(DeferredVk { vk, needs_shift });
    }
}
