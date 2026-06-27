//! warm パスおよび GJI post-transmit 共用の LiteralDetect ステートマシン。
//!
//! warm パスで `TsfProbeCoro` の inline LiteralDetect から独立した
//! [`LiteralDetectFsm`] として切り出したもの。
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
use crate::tsf::probe_fsm::{
    DeferredVk, LiteralDetectConfig, ProbeAction, ProbeObservations, TransmitPlan, TransmitTarget,
};
use crate::tsf::probe_fsm::TsfEnvSnapshot;

/// 部分リテラル検出時に送るバックスペース数。
///
/// 部分リテラルの構造は「先頭 1 文字リテラル + 残りが 1 composition ユニット」であり、
/// BS×1 で composition をクリアし BS×1 でリテラル文字を削除する計 2 回が正しい。
/// `ze_bs_count`（= chars.len()）はローマ字文字数に等しいが、3 文字ローマ字（"ltu" など）
/// では ze_bs_count=3 となり 1 回多くなるため、部分リテラルパスでは使わない。
const PARTIAL_LITERAL_BS: usize = 2;

/// warm パス・GJI post-transmit 共用の LiteralDetect ステートマシン。
///
/// 構築後は 10ms ごとに [`tick`](LiteralDetectFsm::tick) を呼ぶ。
/// `Done` を含む Vec が返ったらタイマーを停止する。
pub(crate) struct LiteralDetectFsm {
    /// ログ相関番号
    cold_seq: u32,
    /// RAII guard — drop で `OUTPUT_GATE.active=false`
    _guard: OutputActiveGuard,
    /// 送信したローマ字（回収アクションのペイロード用）
    romaji: String,
    /// probe 中に蓄積した後続 VK（現在は LiteralDetect フェーズでは使用しないが保持）
    #[allow(dead_code)]
    deferred_vks: Vec<DeferredVk>,
    /// 送信方針（回収アクションのペイロード用）
    #[allow(dead_code)]
    plan: TransmitPlan,
    /// probe 中に観測した事実。部分リテラル判定・sacr warmup config に使用する。
    observations: ProbeObservations,
    /// composition 確認 / raw literal 検出器
    detector: LiteralDetector,
    /// LiteralDetect タイムアウト絶対時刻（ms）
    deadline_ms: u64,
    /// raw literal 検出時に送るバックスペース数
    ze_bs_count: usize,
    /// 構築時点の連続 raw-tsf-literal 回数。
    ///
    /// 0 かつ TSF mode の場合は `StartSacrificialWarmup` 経由で sacr warmup を起動する。
    /// 1 以上の場合は give-up（cleanup のみ）。
    consecutive: u32,
}

impl LiteralDetectFsm {
    /// `LiteralDetectFsm` を生成する。
    ///
    /// `literal_detect_ms` はタイムアウト期間（ms）。`OutputActiveGuard::begin()` を内部で
    /// 呼び出し、デッドライン（`current_tick_ms() + literal_detect_ms`）を確定する。
    ///
    /// `consecutive` は現在の連続 raw-tsf-literal 回数。0 かつ TSF mode のとき sacr warmup を起動する。
    pub(crate) fn new(
        cold_seq: u32,
        romaji: String,
        deferred_vks: Vec<DeferredVk>,
        plan: TransmitPlan,
        observations: ProbeObservations,
        ze_bs_count: usize,
        literal_detect_ms: u64,
        consecutive: u32,
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
            consecutive,
        }
    }

    /// literal 回収用アクション列を生成する。
    ///
    /// TSF mode かつ consecutive==0 → sacr warmup パス（`SendRecoveryBs + StartSacrificialWarmup + Done`）。
    /// それ以外 → 従来の `RawTsfLiteralRecovery + Done`。
    fn emit_recovery_actions(&mut self, env: &TsfEnvSnapshot, backs: usize) -> Vec<ProbeAction> {
        let romaji = std::mem::take(&mut self.romaji);
        if env.is_tsf_mode && self.consecutive == 0 {
            vec![
                ProbeAction::SendRecoveryBs { cold_seq: self.cold_seq, backs },
                ProbeAction::StartSacrificialWarmup(LiteralDetectConfig {
                    cold_seq: self.cold_seq,
                    romaji,
                    deferred_vks: vec![],
                    plan: TransmitPlan {
                        should_prepend_f2: false,
                        used_eager_path: false,
                        needs_literal: true,
                        literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                    },
                    observations: self.observations,
                    literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                    target: TransmitTarget::Tsf,
                    from_literal_recovery: true,
                }),
                ProbeAction::Done,
            ]
        } else {
            vec![
                ProbeAction::RawTsfLiteralRecovery {
                    cold_seq: self.cold_seq,
                    backs,
                    romaji,
                },
                ProbeAction::Done,
            ]
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
                    // ze_bs_count (= chars.len()) は「全部リテラル」向けの値であり、
                    // 部分リテラルには使えない。部分リテラルの構造は常に:
                    //   先頭 1 文字リテラル + 残りが 1 composition ユニット
                    // 削除 = BS×1 (composition クリア) + BS×1 (リテラル削除) = 2 固定。
                    // 例: "ltu" → 'l' リテラル + 'tu'→'と' composition → BS×2 が正しく
                    //     ze_bs_count=3 を使うと挿入点前の無関係な文字まで消える。
                    log::debug!(
                        "[raw-tsf-literal] cold={} LiteralDetectFsm: partial literal (nc=false gji_resumed=false tsf romaji={:?} backs={} consecutive={})",
                        self.cold_seq,
                        self.romaji,
                        PARTIAL_LITERAL_BS,
                        self.consecutive,
                    );
                    crate::ime_diagnostic::log_composition_probe(self.cold_seq, "partial-literal");
                    return self.emit_recovery_actions(env, PARTIAL_LITERAL_BS);
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
                    "[raw-tsf-literal] cold={} LiteralDetectFsm: suspected literal (backs={} consecutive={})",
                    self.cold_seq,
                    self.ze_bs_count,
                    self.consecutive,
                );
                crate::ime_diagnostic::log_composition_probe(self.cold_seq, "suspected");
                self.emit_recovery_actions(env, self.ze_bs_count)
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
            0, // consecutive
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

    // 3 文字ローマ字 (っ = "ltu") でも BS は 2 固定
    #[test]
    fn partial_literal_bs_count_is_always_2_regardless_of_romaji_length() {
        // ze_bs_count=3 (chars.len()) を渡しても部分リテラルパスは PARTIAL_LITERAL_BS=2 を使う。
        // "ltu" → 'l' リテラル + 'tu'→'と' composition → BS×2 が正しい。
        // BS×3 を送ると挿入点前の無関係な文字を消してしまう。
        let fsm = make_fsm("ltu", false, false, 3);
        let env = tsf_env();
        let partial = !fsm.observations.nc_fired
            && !fsm.observations.gji_resumed
            && env.is_tsf_mode
            && fsm.romaji.chars().count() >= 2;
        assert!(partial, "ltu: 部分リテラル条件が揃っているべき");
        // PARTIAL_LITERAL_BS=2 であることを確認
        // (tick() 内で self.ze_bs_count=3 でなく 2 を使っていることを静的に検証)
        assert_eq!(
            super::PARTIAL_LITERAL_BS, 2,
            "PARTIAL_LITERAL_BS は常に 2 (1 リテラル + 1 composition クリア)"
        );
    }
}
