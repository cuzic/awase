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
    /// probe 中に観測した事実。部分リテラル判定に使用する。
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
                // 部分リテラル検出: WezTerm (TSF mode) では HIMC=NULL のため
                // foreground_comp_char による文字照合が不可能。代わりに以下の条件が
                // すべて揃った場合を「先頭文字がリテラル化した partial literal」と判断する:
                //   - nc_fired=false : fresh F2 に WezTerm が NAMECHANGE で応答しなかった
                //     → TSF context が cold のまま送信した可能性が高い
                //   - gji_resumed=false : GJI も F2 後に I/O 応答しなかった
                //     → composition が全く始まっていない状態で先頭 VK が届いた疑い
                //   - is_tsf_mode : WezTerm 等の TSF 専用アプリ（HIMC 照合不可）
                //   - romaji 2 文字以上 : 1 文字なら partial にならない
                let partial_literal_suspected = !self.observations.nc_fired
                    && !self.observations.gji_resumed
                    && env.is_tsf_mode
                    && self.romaji.chars().count() >= 2;

                if partial_literal_suspected {
                    log::debug!(
                        "[raw-tsf-literal] cold={} LiteralDetectFsm: partial literal (nc=false gji_resumed=false tsf romaji={:?} backs={})",
                        self.cold_seq,
                        self.romaji,
                        self.ze_bs_count,
                    );
                    crate::ime_diagnostic::log_composition_probe(self.cold_seq, "partial-literal");
                    let romaji = std::mem::take(&mut self.romaji);
                    return vec![
                        ProbeAction::RawTsfLiteralRecovery {
                            cold_seq: self.cold_seq,
                            backs: self.ze_bs_count,
                            romaji,
                        },
                        ProbeAction::Done,
                    ];
                }

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

// ── テスト ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::probe_fsm::TransmitPlan;

    fn make_fsm(
        romaji: &str,
        nc_fired: bool,
        gji_resumed: bool,
        ze_bs_count: usize,
    ) -> LiteralDetectFsm {
        LiteralDetectFsm::new(
            0,
            romaji.to_string(),
            vec![],
            TransmitPlan {
                should_prepend_f2: false,
                used_eager_path: false,
                needs_literal: true,
                literal_detect_ms: 500,
            },
            ProbeObservations { nc_fired, gji_resumed },
            ze_bs_count,
            500,
        )
    }

    fn tsf_env() -> TsfEnvSnapshot {
        TsfEnvSnapshot {
            is_tsf_mode: true,
            gji_active: true,
            ..Default::default()
        }
    }

    // CompositionConfirmed が partial literal 条件を満たす場合 → RawTsfLiteralRecovery
    #[test]
    fn composition_confirmed_tsf_nc_false_gji_not_resumed_multi_char_forces_recovery() {
        let mut fsm = make_fsm("ni", false, false, 2);
        // detector.check_now はグローバル状態に依存するため直接テスト不可。
        // 代わりに partial_literal_suspected フラグの条件をロジックレベルで検証する。
        let env = tsf_env();
        // 条件充足: nc=false, gji_resumed=false, is_tsf_mode=true, romaji.chars()=2
        let partial = !fsm.observations.nc_fired
            && !fsm.observations.gji_resumed
            && env.is_tsf_mode
            && fsm.romaji.chars().count() >= 2;
        assert!(partial, "部分リテラル条件がすべて揃っているべき");
    }

    // nc_fired=true の場合は強制 recovery しない
    #[test]
    fn composition_confirmed_nc_fired_does_not_force_recovery() {
        let fsm = make_fsm("ni", true, false, 2);
        let env = tsf_env();
        let partial = !fsm.observations.nc_fired
            && !fsm.observations.gji_resumed
            && env.is_tsf_mode
            && fsm.romaji.chars().count() >= 2;
        assert!(!partial, "nc_fired=true → 強制 recovery 不要");
    }

    // gji_resumed=true の場合は強制 recovery しない
    #[test]
    fn composition_confirmed_gji_resumed_does_not_force_recovery() {
        let fsm = make_fsm("ni", false, true, 2);
        let env = tsf_env();
        let partial = !fsm.observations.nc_fired
            && !fsm.observations.gji_resumed
            && env.is_tsf_mode
            && fsm.romaji.chars().count() >= 2;
        assert!(!partial, "gji_resumed=true → 強制 recovery 不要");
    }

    // 1 文字ローマ字は部分リテラルにならない
    #[test]
    fn composition_confirmed_single_char_romaji_no_recovery() {
        let fsm = make_fsm("n", false, false, 1);
        let env = tsf_env();
        let partial = !fsm.observations.nc_fired
            && !fsm.observations.gji_resumed
            && env.is_tsf_mode
            && fsm.romaji.chars().count() >= 2;
        assert!(!partial, "1 文字ローマ字 → 部分リテラルにならない");
    }

    // TSF モードでない場合は強制 recovery しない
    #[test]
    fn composition_confirmed_non_tsf_no_recovery() {
        let fsm = make_fsm("ni", false, false, 2);
        let env = TsfEnvSnapshot {
            is_tsf_mode: false,
            ..Default::default()
        };
        let partial = !fsm.observations.nc_fired
            && !fsm.observations.gji_resumed
            && env.is_tsf_mode
            && fsm.romaji.chars().count() >= 2;
        assert!(!partial, "non-TSF mode → 強制 recovery 不要");
    }
}
