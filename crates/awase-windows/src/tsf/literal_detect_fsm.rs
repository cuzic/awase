//! warm パスおよび GJI post-transmit 共用の LiteralDetect ステートマシン。
//!
//! [`TsfProbeMachine::new_literal_detect`] が担う LiteralDetect フェーズを
//! 独立した [`LiteralDetectFsm`] として切り出したもの。
//!
//! ## 使用場面
//!
//! - warm パスからの直接呼び出し（TSF 送信後の composition 確認）
//! - GJI post-transmit（probe_fsm から独立して LiteralDetect だけを動かしたい場合）
//!
//! ## 動作
//!
//! - 10ms 間隔の TIMER_TSF_PROBE ハンドラから [`LiteralDetectFsm::tick`] を呼ぶ。
//! - composition 確認 → `vec![ProbeAction::Done]`
//! - raw literal 疑い → `vec![ProbeAction::RawTsfLiteralRecovery { .. }, ProbeAction::Done]`
//! - 判定待ち → `vec![]`（タイマー継続）

use crate::tsf::probe::LiteralDetector;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::probe_fsm::{DeferredVk, ProbeAction, ProbeObservations, TransmitPlan};
use crate::tsf::probe_fsm::TsfEnvSnapshot;

/// warm パス・GJI post-transmit 共用の LiteralDetect ステートマシン。
///
/// 構築後は 10ms ごとに [`tick`](LiteralDetectFsm::tick) を呼ぶ。
/// `Done` を含む Vec が返ったらタイマーを停止する。
pub(crate) struct LiteralDetectFsm {
    /// ログ相関番号
    cold_seq: u32,
    /// RAII guard — drop で `OUTPUT_GATE.active=false`
    _guard: OutputActiveGuard,
    /// 送信したローマ字（RawTsfLiteralRecovery ペイロード用）
    romaji: String,
    /// probe 中に蓄積した後続 VK（現在は LiteralDetect フェーズでは使用しないが保持）
    #[allow(dead_code)]
    deferred_vks: Vec<DeferredVk>,
    /// 送信方針（RawTsfLiteralRecovery ペイロード用）
    #[allow(dead_code)]
    plan: TransmitPlan,
    /// probe 中に観測した事実（GjiFsm bridge・WarmupPath 分類用）
    #[allow(dead_code)]
    observations: ProbeObservations,
    /// composition 確認 / raw literal 検出器
    detector: LiteralDetector,
    /// LiteralDetect タイムアウト絶対時刻（ms）
    deadline_ms: u64,
    /// raw literal 検出時に送るバックスペース数
    ze_bs_count: usize,
}

impl LiteralDetectFsm {
    /// `LiteralDetectFsm` を生成する。
    ///
    /// `literal_detect_ms` はタイムアウト期間（ms）。`OutputActiveGuard::begin()` を内部で
    /// 呼び出し、デッドライン（`current_tick_ms() + literal_detect_ms`）を確定する。
    pub(crate) fn new(
        cold_seq: u32,
        romaji: String,
        deferred_vks: Vec<DeferredVk>,
        plan: TransmitPlan,
        observations: ProbeObservations,
        ze_bs_count: usize,
        literal_detect_ms: u64,
    ) -> Self {
        let guard = OutputActiveGuard::begin();
        let detector = LiteralDetector::new();
        let deadline_ms = crate::hook::current_tick_ms() + literal_detect_ms;
        Self {
            cold_seq,
            _guard: guard,
            romaji,
            deferred_vks,
            plan,
            observations,
            detector,
            deadline_ms,
            ze_bs_count,
        }
    }

    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。
    ///
    /// 返値の `Vec<ProbeAction>` を `dispatch_probe_actions` が実行する。
    /// - 空 Vec → まだ待機中（タイマー継続）
    /// - `[Done]` → composition 確認（タイマー停止）
    /// - `[RawTsfLiteralRecovery { .. }, Done]` → raw literal 検出（タイマー停止）
    pub(crate) fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        use crate::tsf::probe::DetectionResult;

        let Some(detection) = self.detector.check_now(self.deadline_ms) else {
            return vec![];
        };

        match detection {
            DetectionResult::CompositionConfirmed => {
                // 部分リテラル検出: SHOW が発火しても composition 内容が expected_kana と
                // 異なる場合は「K がリテラル化して O だけが compose された」部分リテラルを示す。
                // LiteralDetectFsm では expected_kana = None（新規パスでは使用しない）。
                // env.foreground_comp_char は利用可能だが、None ケースはスキップして
                // 安全に CompositionConfirmed へ。
                let _ = env; // 将来の partial literal 検出用に env を受け取る
                log::debug!(
                    "[raw-tsf-literal] cold={} LiteralDetectFsm: composition confirmed",
                    self.cold_seq
                );
                crate::ime_diagnostic::log_composition_probe(self.cold_seq, "confirmed");
                vec![ProbeAction::Done]
            }
            DetectionResult::SuspectedLiteral => {
                log::debug!(
                    "[raw-tsf-literal] cold={} LiteralDetectFsm: suspected literal (backs={})",
                    self.cold_seq,
                    self.ze_bs_count
                );
                crate::ime_diagnostic::log_composition_probe(self.cold_seq, "suspected");
                let romaji = std::mem::take(&mut self.romaji);
                vec![
                    ProbeAction::RawTsfLiteralRecovery {
                        cold_seq: self.cold_seq,
                        backs: self.ze_bs_count,
                        romaji,
                    },
                    ProbeAction::Done,
                ]
            }
        }
    }

}

impl crate::tsf::tickable_fsm::TickableFsm for LiteralDetectFsm {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        LiteralDetectFsm::tick(self, env)
    }

    fn cold_seq_hint(&self) -> u32 {
        self.cold_seq
    }
}
