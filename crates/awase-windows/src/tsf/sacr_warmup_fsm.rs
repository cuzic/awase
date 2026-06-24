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
//! 5. Chrome パス: candidate window HIDE を待つ（IPC race 回避）
//! 6. [`ProbeAction::SacrificialResend`] を emit → dispatcher が BS×1 + 実ローマ字再送
//!
//! ## Chrome IPC race と HIDE 待機
//!
//! Chrome/GJI の EndComposition はクロスプロセス IPC を経由するため、
//! VK_A+BS の BS キャンセルが TSF スタックを伝播するまでに ~200ms かかる。
//! composition-confirmed（GJI write +400B 検出、~26ms）の直後に実ローマ字を送ると、
//! delayed EndComposition が後続の composition（例：「korede」）をキャンセルする。
//!
//! candidate window HIDE = EndComposition IPC 到達の代理指標として使い、
//! HIDE 確認後に実ローマ字を送ることで race を回避する。
//!
//! ## 利点
//!
//! 実ローマ字が readline バッファにリテラル状態で残らないため、
//! ユーザーが判定待機中に Enter を押しても literal テキストが Submit されない。
//! 「Engine ON / IME OFF（TSF cold）」状態を構造的に排除する。

use crate::tsf::probe::LiteralDetector;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::probe_fsm::{DeferredVk, ProbeAction, SacrificialResend, TransmitTarget};
use crate::tuning::{CHROME_GJI_REINIT_CONFIRM_MS, SACR_WARMUP_CHROME_HIDE_WAIT_MS};
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
    /// Chrome パス: composition-confirmed 後に GJI candidate HIDE を待つフェーズ。
    ///
    /// `None` = まだ HIDE 待機に入っていない。
    /// `Some(deadline_ms)` = HIDE 待機中（deadline を過ぎたら強制送信）。
    ///
    /// VK_A+BS の EndComposition が Chrome の IPC を伝播するまで ~200ms かかるため、
    /// candidate window が非表示になる（HIDE 観測）まで実ローマ字送信を遅延させる。
    hide_wait_deadline_ms: Option<u64>,
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
            hide_wait_deadline_ms: None,
        }
    }

    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。
    ///
    /// VK_A の composition を確認次第（成功・タイムアウトいずれも）[`ProbeAction::SacrificialResend`] を emit する。
    pub(crate) fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        // ── Phase 2: Chrome HIDE 待機中 ────────────────────────────────────────
        // composition-confirmed 後、VK_A+BS の EndComposition IPC が Chrome に
        // 到達するのを candidate window HIDE で確認してから実ローマ字を送る。
        if let Some(hide_deadline) = self.hide_wait_deadline_ms {
            let now = crate::hook::current_tick_ms();
            let candidate_gone = !env.gji_candidate_visible;
            let timed_out = now >= hide_deadline;
            if !candidate_gone && !timed_out {
                return vec![];
            }
            log::debug!(
                "[sacr-warmup] cold={} Chrome HIDE 待機完了: candidate_gone={} timed_out={}",
                self.cold_seq, candidate_gone, timed_out,
            );
            let romaji = std::mem::take(&mut self.romaji);
            let deferred_vks = std::mem::take(&mut self.deferred_vks);
            return vec![
                ProbeAction::SacrificialResend(SacrificialResend {
                    cold_seq: self.cold_seq,
                    romaji,
                    deferred_vks,
                    target: self.target,
                    confirmed_warm: true,
                }),
                ProbeAction::Done,
            ];
        }

        // ── Phase 1: composition 確認待機 ─────────────────────────────────────
        let Some(detection) = self.detector.check_now(self.deadline_ms) else {
            return vec![];
        };

        use crate::tsf::probe::DetectionResult;
        let confirmed_warm = matches!(detection, DetectionResult::CompositionConfirmed);
        log::debug!(
            "[sacr-warmup] cold={} VK_A 判定={} → 実ローマ字 {:?} 再送",
            self.cold_seq,
            if confirmed_warm { "composition-confirmed (TSF warm)" } else { "timeout (TSF still cold)" },
            self.romaji,
        );
        crate::ime_diagnostic::log_composition_probe(
            self.cold_seq,
            if confirmed_warm { "sacr-warm" } else { "sacr-timeout" },
        );

        // Chrome cold: F22→F21 リセット + ImeMode 確認待機フェーズへ移行。
        // ChromeGjiReinitFsm が Hiragana 確認後に SacrificialResend を emit する。
        if !confirmed_warm && self.target == TransmitTarget::Chrome {
            let romaji = std::mem::take(&mut self.romaji);
            let deferred_vks = std::mem::take(&mut self.deferred_vks);
            log::debug!(
                "[sacr-warmup] cold={} Chrome cold → StartChromeGjiReinit (reinit timeout={}ms)",
                self.cold_seq, CHROME_GJI_REINIT_CONFIRM_MS,
            );
            return vec![ProbeAction::StartChromeGjiReinit {
                cold_seq: self.cold_seq,
                romaji,
                deferred_vks,
            }];
        }

        // Chrome warm: VK_A が composition に入った。
        // EndComposition IPC が Chrome に伝播するまで ~200ms かかるため、
        // candidate window HIDE を確認してから実ローマ字を送る（IPC race 回避）。
        if confirmed_warm && self.target == TransmitTarget::Chrome {
            if env.gji_candidate_visible {
                // candidate window がまだ表示中 → HIDE 待機フェーズへ移行
                let hide_deadline = crate::hook::current_tick_ms() + SACR_WARMUP_CHROME_HIDE_WAIT_MS;
                self.hide_wait_deadline_ms = Some(hide_deadline);
                log::debug!(
                    "[sacr-warmup] cold={} Chrome warm → candidate visible, HIDE 待機開始 (timeout={}ms)",
                    self.cold_seq, SACR_WARMUP_CHROME_HIDE_WAIT_MS,
                );
                return vec![];
            }
            // candidate window が最初から非表示（window が出なかった等）→ 即送信
            log::debug!(
                "[sacr-warmup] cold={} Chrome warm → candidate 非表示、即再送",
                self.cold_seq,
            );
        }

        let romaji = std::mem::take(&mut self.romaji);
        let deferred_vks = std::mem::take(&mut self.deferred_vks);
        vec![
            ProbeAction::SacrificialResend(SacrificialResend {
                cold_seq: self.cold_seq,
                romaji,
                deferred_vks,
                target: self.target,
                confirmed_warm,
            }),
            ProbeAction::Done,
        ]
    }
}

impl crate::tsf::tickable_fsm::TickableFsm for SacrificialWarmupFsm {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        Self::tick(self, env)
    }

    fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }

    fn push_deferred(&mut self, vk: VkCode, needs_shift: bool) {
        self.deferred_vks.push(DeferredVk { vk, needs_shift });
    }
}
