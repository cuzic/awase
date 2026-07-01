//! tick 駆動型 FSM の共通インターフェース。
//!
//! 10ms タイマー (`TIMER_TSF_PROBE`) から `tick()` が呼ばれるパターンを型として表現する。
//!
//! ## 実装一覧
//!
//! | 実装型 | 用途 | 使用するメソッド |
//! |--------|------|-----------------|
//! | `GjiWarmupCoro` | GJI cold-start warmup probe | `tick`, `cold_seq_hint`, `forces_prepend_f2_for_extra_f2`, `apply_fresh_f2_sent`, `apply_transmit_done` |
//! | `TsfProbeCoro` | Chrome probe + LiteralDetect | `tick`, `cold_seq_hint`, `apply_transmit_done` |
//! | `SacrificialWarmupCoro` | VK_A 犠牲キー暖機 + Chrome GJI 再初期化 | `tick`, `cold_seq_hint`, `notify_start_composition` |
//! | `LiteralDetectFsm` | warm パスの post-transmit composition 確認 | `tick`, `cold_seq_hint` のみ |
//!
//! デフォルト実装（no-op）が多いのは各 implementor が必要なメソッドだけをオーバーライドするため。
//!
//! probe 進行中に届いた後続 VK（deferred VK）は `TsfWarmupCoordinator` が一元管理する
//! （`push_deferred` は個々の実装が持たない）。

use crate::tsf::observer::NamechangeBaseline;
use crate::tsf::probe::LiteralDetector;
use crate::tsf::probe_fsm::{ProbeAction, TsfEnvSnapshot};

/// tick 駆動型 FSM の共通インターフェース。
///
/// `Box<dyn TickableFsm>` として `pending_tsf` に格納される。
/// 実装型ごとに使うメソッドが異なるため、未使用メソッドにはデフォルト no-op が付いている。
pub(crate) trait TickableFsm {
    // ── Core（全実装型）────────────────────────────────────────────────────

    /// 1 ステップ進める。[`ProbeAction::Done`] が含まれたら完了。
    fn tick(&mut self, env: &TsfEnvSnapshot) -> Vec<ProbeAction>;
    /// ログ相関用の cold_seq を返す。
    fn cold_seq_hint(&self) -> u32;

    // ── FreshF2 ケイパビリティ（GjiWarmupCoro のみ）───────────────────────
    //
    // Medium/Long cold start で F2×2 連続送信が必要かどうか。
    // GjiWarmupCoro のみが `true` または実装を返す。他は no-op デフォルト。

    fn forces_prepend_f2_for_extra_f2(&self) -> bool {
        false
    }
    fn apply_fresh_f2_sent(&mut self, _nc_baseline: NamechangeBaseline, _fresh_f2_ms: u64) {}

    // ── TransmitDone ケイパビリティ（GjiWarmupCoro / TsfProbeCoro）───────────
    //
    // TSF/Chrome 経由の送信完了後、inline LiteralDetect フェーズへの継続を制御する。
    // `true` = この machine は完了扱い（Done）、`false` = 次 tick で LiteralDetect に続く（Continue）。

    fn apply_transmit_done(
        &mut self,
        _romaji: String,
        _ze_bs_count: usize,
        _detector: Option<LiteralDetector>,
        _literal_detect_ms: u64,
        _expected_kana: Option<char>,
    ) -> bool {
        true
    }

    // ── StartComposition 通知（SacrificialWarmupFsm のみ）────────────────
    //
    // drain_pending_composition_events が StartComposition を取り出したとき、
    // 現在の sacr-warmup probe に対して composition が観測されたことを通知する。
    // VK_A+BS atomic batch で SHOW+HIDE が最初の tick より前に完了する場合の
    // IPC race 検出（Phase 3 IPC settle 待機）に使う。

    fn notify_start_composition(&mut self) {}
}
