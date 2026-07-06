//! literal 検出の共有コア ([`LiteralDetectCore`]) と warm パス用の薄いラッパー
//! ([`LiteralDetectFsm`])。
//!
//! ## literal 検出ロジックの単一所在地
//!
//! TSF 送信後に「composition が成立したか / リテラル化したか」を判定するロジックは、
//! 以前は本ファイルの [`LiteralDetectFsm`]（warm パス）と `gji_warmup_coro.rs` の
//! inline Phase 6（cold パス）に**重複**していた（ADR-053 は cold パスへの畳み込みを
//! 記載するが warm パスの実装が別に残り、判定コードが 2 箇所に分岐していた）。
//!
//! この重複を解消するため、判定タイミング・partial literal 判定・回収アクション生成を
//! [`LiteralDetectCore`] に集約した。cold パス（`GjiWarmupCoro` Phase 6）と warm パス
//! （`LiteralDetectFsm`）は同一の `LiteralDetectCore::poll` を呼ぶ。
//!
//! ## 使用場面
//!
//! - warm パス: [`LiteralDetectFsm`] を `pending_tsf` に install（TSF 送信後の composition 確認）
//! - cold パス: `GjiWarmupCoro` が coro 本体内で [`LiteralDetectCore`] を直接駆動
//!
//! ## 動作（`LiteralDetectCore::poll` / `LiteralDetectFsm::tick`）
//!
//! - 10ms 間隔の TIMER_TSF_PROBE ハンドラから駆動する。
//! - composition 確認 → `[ProbeAction::Done]`
//! - raw literal 疑い → `[ProbeAction::RawTsfLiteralRecovery { .. }, ProbeAction::Done]`
//!   （TSF mode + consecutive==0 では `SendRecoveryBs + StartSacrificialWarmup + Done`）
//! - 判定待ち → `None`（`LiteralDetectFsm::tick` では `vec![]`、タイマー継続）

use crate::tsf::probe::LiteralDetector;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{
    LiteralDetectConfig, ProbeAction, ProbeObservations, TransmitPlan, TransmitTarget,
};
use crate::tsf::warmup::probe_fsm::TsfEnvSnapshot;

/// 部分リテラル検出時に送るバックスペース数。
///
/// 部分リテラルの構造は「先頭 1 文字リテラル + 残りが 1 composition ユニット」であり、
/// BS×1 で composition をクリアし BS×1 でリテラル文字を削除する計 2 回が正しい。
/// `ze_bs_count`（= chars.len()）はローマ字文字数に等しいが、3 文字ローマ字（"ltu" など）
/// では ze_bs_count=3 となり 1 回多くなるため、部分リテラルパスでは使わない。
pub(crate) const PARTIAL_LITERAL_BS: usize = 2;

/// `CompositionConfirmed` 時に「先頭文字がリテラル化した partial literal」かどうかを判定する純関数。
///
/// WezTerm (TSF mode) では HIMC=NULL のため foreground_comp_char による文字照合が
/// 不可能。代わりに以下の条件がすべて揃った場合を partial literal と判断する:
///   - `nc_fired=false` : fresh F2 に WezTerm が NAMECHANGE で応答しなかった
///     → TSF context が cold のまま送信した可能性が高い
///   - `gji_resumed=false` : GJI も F2 後に I/O 応答しなかった
///     → composition が全く始まっていない状態で先頭 VK が届いた疑い
///   - `is_tsf_mode` : WezTerm 等の TSF 専用アプリ（HIMC 照合不可）
///   - romaji 2 文字以上 : 1 文字なら partial にならない
pub(crate) fn is_partial_literal(
    observations: ProbeObservations,
    romaji: &str,
    env: &TsfEnvSnapshot,
) -> bool {
    !observations.nc_fired
        && !observations.gji_resumed
        && env.is_tsf_mode
        && romaji.chars().count() >= 2
}

/// literal 回収用アクション列を生成する（cold/warm 共通）。
///
/// TSF mode かつ consecutive==0 → sacr warmup パス（`SendRecoveryBs + StartSacrificialWarmup + Done`）。
/// それ以外 → 従来の `RawTsfLiteralRecovery + Done`。
pub(crate) fn emit_recovery_actions(
    cold_seq: u32,
    romaji: String,
    backs: usize,
    observations: ProbeObservations,
    consecutive: u32,
    env: &TsfEnvSnapshot,
) -> Vec<ProbeAction> {
    if env.is_tsf_mode && consecutive == 0 {
        vec![
            ProbeAction::SendRecoveryBs { cold_seq, backs },
            ProbeAction::StartSacrificialWarmup(LiteralDetectConfig {
                cold_seq,
                romaji,
                plan: TransmitPlan {
                    should_prepend_f2: false,
                    used_eager_path: false,
                    needs_literal: true,
                    literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                },
                observations,
                literal_detect_ms: crate::tuning::RAW_TSF_LITERAL_DETECT_MS,
                target: TransmitTarget::Tsf,
                from_literal_recovery: true,
            }),
            ProbeAction::Done,
        ]
    } else {
        vec![
            ProbeAction::RawTsfLiteralRecovery {
                cold_seq,
                backs,
                romaji,
            },
            ProbeAction::Done,
        ]
    }
}

/// warm パス（`LiteralDetectFsm`）と cold パス（`GjiWarmupCoro` Phase 6）が共有する
/// literal 検出コア。
///
/// 検出タイミング（`LiteralDetector::check_now`）・partial literal 判定・回収アクション生成を
/// ここ 1 箇所に集約する。両パスは `poll` を 10ms ごとに呼ぶだけで、判定ロジックを重複させない。
pub(crate) struct LiteralDetectCore {
    /// ログ相関番号
    cold_seq: u32,
    /// 送信したローマ字（回収アクションのペイロード用）
    romaji: String,
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

impl LiteralDetectCore {
    /// `LiteralDetectCore` を生成する。`detector` と `deadline_ms` は呼び出し側が用意する
    /// （cold パスは transmit 完了時、warm パスは送信直後に確定させる）。
    pub(crate) const fn new(
        cold_seq: u32,
        romaji: String,
        observations: ProbeObservations,
        detector: LiteralDetector,
        deadline_ms: u64,
        ze_bs_count: usize,
        consecutive: u32,
    ) -> Self {
        Self {
            cold_seq,
            romaji,
            observations,
            detector,
            deadline_ms,
            ze_bs_count,
            consecutive,
        }
    }

    /// TIMER_TSF_PROBE ハンドラから 10ms ごとに呼ぶ。
    ///
    /// - `None` → まだ待機中（タイマー継続）
    /// - `Some([Done])` → composition 確認（タイマー停止）
    /// - `Some([RawTsfLiteralRecovery { .. }, Done])` → raw literal 検出（タイマー停止）
    pub(crate) fn poll(&mut self, env: &TsfEnvSnapshot) -> Option<Vec<ProbeAction>> {
        use crate::tsf::probe::DetectionResult;

        let detection = self.detector.check_now(self.deadline_ms)?;

        match detection {
            DetectionResult::CompositionConfirmed => {
                if is_partial_literal(self.observations, &self.romaji, env) {
                    // ze_bs_count (= chars.len()) は「全部リテラル」向けの値であり、
                    // 部分リテラルには使えない。部分リテラルの構造は常に:
                    //   先頭 1 文字リテラル + 残りが 1 composition ユニット
                    // 削除 = BS×1 (composition クリア) + BS×1 (リテラル削除) = 2 固定。
                    // 例: "ltu" → 'l' リテラル + 'tu'→'と' composition → BS×2 が正しく
                    //     ze_bs_count=3 を使うと挿入点前の無関係な文字まで消える。
                    log::debug!(
                        "[literal-detect] cold={} partial literal (nc=false gji_resumed=false tsf romaji={:?} backs={} consecutive={})",
                        self.cold_seq,
                        self.romaji,
                        PARTIAL_LITERAL_BS,
                        self.consecutive,
                    );
                    crate::ime_diagnostic::log_composition_probe(self.cold_seq, "partial-literal");
                    return Some(self.recovery(env, PARTIAL_LITERAL_BS));
                }

                log::debug!(
                    "[literal-detect] cold={} composition confirmed",
                    self.cold_seq
                );
                crate::ime_diagnostic::log_composition_probe(self.cold_seq, "confirmed");
                Some(vec![ProbeAction::Done])
            }
            DetectionResult::SuspectedLiteral => {
                log::debug!(
                    "[literal-detect] cold={} suspected literal (backs={} consecutive={})",
                    self.cold_seq,
                    self.ze_bs_count,
                    self.consecutive,
                );
                crate::ime_diagnostic::log_composition_probe(self.cold_seq, "suspected");
                Some(self.recovery(env, self.ze_bs_count))
            }
        }
    }

    fn recovery(&mut self, env: &TsfEnvSnapshot, backs: usize) -> Vec<ProbeAction> {
        emit_recovery_actions(
            self.cold_seq,
            std::mem::take(&mut self.romaji),
            backs,
            self.observations,
            self.consecutive,
            env,
        )
    }
}

/// warm パスの post-transmit composition 確認 FSM。[`LiteralDetectCore`] の薄いラッパー。
///
/// 構築後は 10ms ごとに [`tick`](LiteralDetectFsm::tick) を呼ぶ。
/// `Done` を含む Vec が返ったらタイマーを停止する。
pub(crate) struct LiteralDetectFsm {
    core: LiteralDetectCore,
    /// RAII guard — drop で `OUTPUT_GATE.active=false`
    _guard: OutputActiveGuard,
}

impl LiteralDetectFsm {
    /// `LiteralDetectFsm` を生成する。
    ///
    /// `literal_detect_ms` はタイムアウト期間（ms）。`OutputActiveGuard::begin()` を内部で
    /// 呼び出し、`LiteralDetector::new()` と deadline（`current_tick_ms() + literal_detect_ms`）を
    /// 確定して `LiteralDetectCore` を組み立てる。
    ///
    /// `consecutive` は現在の連続 raw-tsf-literal 回数。0 かつ TSF mode のとき sacr warmup を起動する。
    pub(crate) fn new(
        cold_seq: u32,
        romaji: String,
        observations: ProbeObservations,
        ze_bs_count: usize,
        literal_detect_ms: u64,
        consecutive: u32,
    ) -> Self {
        let guard = OutputActiveGuard::begin();
        let detector = LiteralDetector::new();
        let deadline_ms = crate::hook::current_tick_ms() + literal_detect_ms;
        Self {
            core: LiteralDetectCore::new(
                cold_seq,
                romaji,
                observations,
                detector,
                deadline_ms,
                ze_bs_count,
                consecutive,
            ),
            _guard: guard,
        }
    }
}

impl crate::tsf::warmup::tickable_fsm::TickableFsm for LiteralDetectFsm {
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.core.poll(env).unwrap_or_default()
    }

    fn cold_seq_hint(&self) -> u32 {
        self.core.cold_seq
    }
}

// ── テスト ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(nc_fired: bool, gji_resumed: bool) -> ProbeObservations {
        ProbeObservations { nc_fired, gji_resumed }
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
        // 条件充足: nc=false, gji_resumed=false, is_tsf_mode=true, romaji.chars()=2
        assert!(
            is_partial_literal(obs(false, false), "ni", &tsf_env()),
            "部分リテラル条件がすべて揃っているべき"
        );
    }

    // nc_fired=true の場合は強制 recovery しない
    #[test]
    fn composition_confirmed_nc_fired_does_not_force_recovery() {
        assert!(
            !is_partial_literal(obs(true, false), "ni", &tsf_env()),
            "nc_fired=true → 強制 recovery 不要"
        );
    }

    // gji_resumed=true の場合は強制 recovery しない
    #[test]
    fn composition_confirmed_gji_resumed_does_not_force_recovery() {
        assert!(
            !is_partial_literal(obs(false, true), "ni", &tsf_env()),
            "gji_resumed=true → 強制 recovery 不要"
        );
    }

    // 1 文字ローマ字は部分リテラルにならない
    #[test]
    fn composition_confirmed_single_char_romaji_no_recovery() {
        assert!(
            !is_partial_literal(obs(false, false), "n", &tsf_env()),
            "1 文字ローマ字 → 部分リテラルにならない"
        );
    }

    // TSF モードでない場合は強制 recovery しない
    #[test]
    fn composition_confirmed_non_tsf_no_recovery() {
        let env = TsfEnvSnapshot {
            is_tsf_mode: false,
            ..Default::default()
        };
        assert!(
            !is_partial_literal(obs(false, false), "ni", &env),
            "non-TSF mode → 強制 recovery 不要"
        );
    }

    // 3 文字ローマ字 (っ = "ltu") でも BS は 2 固定
    #[test]
    fn partial_literal_bs_count_is_always_2_regardless_of_romaji_length() {
        // "ltu" → 'l' リテラル + 'tu'→'と' composition → BS×2 が正しい。
        // BS×3 (= chars.len()) を送ると挿入点前の無関係な文字を消してしまう。
        assert!(
            is_partial_literal(obs(false, false), "ltu", &tsf_env()),
            "ltu: 部分リテラル条件が揃っているべき"
        );
        assert_eq!(
            PARTIAL_LITERAL_BS, 2,
            "PARTIAL_LITERAL_BS は常に 2 (1 リテラル + 1 composition クリア)"
        );
    }
}
