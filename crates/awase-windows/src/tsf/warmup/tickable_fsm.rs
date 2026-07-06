//! tick 駆動型 FSM の共通インターフェース。
//!
//! 10ms タイマー (`TIMER_TSF_PROBE`) から `tick()` が呼ばれるパターンを型として表現する。
//!
//! ## 実装一覧（本番実装 8 種 + テスト用 StubMachine）
//!
//! | 実装型 | ファイル | 用途 | 追加でオーバーライドするメソッド |
//! |--------|---------|------|-----------------|
//! | `GjiWarmupCoro` | `gji_warmup_coro.rs` | GJI cold-start warmup probe（StepCoro） | `forces_prepend_f2_for_extra_f2`, `apply_fresh_f2_sent`, `apply_transmit_done` |
//! | `TsfProbeCoro` | `probe_fsm.rs` | Chrome probe + LiteralDetect（StepCoro） | `apply_transmit_done` |
//! | `SacrificialWarmupCoro` | `sacr_warmup_coro.rs` | VK_A 犠牲キー暖機 + Chrome GJI 再初期化（StepCoro） | `notify_start_composition` |
//! | `LiteralDetectFsm` | `literal_detect_fsm.rs` | warm パスの post-transmit composition 確認（`LiteralDetectCore` ラッパー） | なし（Core のみ） |
//! | `ImeOffOnWarmupFsm` | `ime_offon_warmup_fsm.rs` | VK_IME_OFF→ON 暖機（手書きカウンタ FSM） | なし |
//! | `UnicodeColdWarmupFsm` | `unicode_cold_warmup_fsm.rs` | Unicode long-cold の deferred chars 送信（手書き FSM） | `push_deferred_unicode_chars` |
//! | `ChromeProbe` | `chrome_probe.rs` | Chrome cold-start GJI readiness probe（手書き FSM） | なし |
//! | `UnicodeLiteralObserverFsm` | `unicode_literal_observer.rs` | Unicode 送信後の GJI write 観測（事後 Tsf 昇格） | なし |
//!
//! `Core`（`tick` / `cold_seq_hint`）は全実装型が実装する。上表は core 以外に
//! オーバーライドするメソッドのみを列挙する（デフォルト no-op を使うため）。
//!
//! probe 進行中に届いた後続 VK（deferred VK）は `TsfWarmupCoordinator` が一元管理する
//! （`push_deferred` は個々の実装が持たない）。

use crate::tsf::observer::NamechangeBaseline;
use crate::tsf::probe::LiteralDetector;
use crate::tsf::warmup::probe_fsm::{ProbeAction, TsfEnvSnapshot};

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

    // ── Unicode deferred chars 追記（UnicodeColdWarmupFsm のみ）──────────
    //
    // drain 処理中に2文字目以降の long-cold Unicode char が届いたとき、
    // 既存 FSM に追記することで FSM の上書きと文字消失を防ぐ。
    //
    // 対応していない FSM は `false` を返す（デフォルト）。

    fn push_deferred_unicode_chars(&mut self, _chars: &[char]) -> bool {
        false
    }
}
