//! tick 駆動型 FSM の共通インターフェース。
//!
//! 10ms タイマー (`TIMER_TSF_PROBE`) から `tick()` が呼ばれるパターンを型として表現する。
//!
//! ## 実装一覧（本番実装 7 種 + テスト用 StubMachine）
//!
//! | 実装型 | ファイル | 用途 | 追加でオーバーライドするメソッド |
//! |--------|---------|------|-----------------|
//! | `GjiWarmupCoro` | `gji_warmup_coro.rs` | GJI cold-start warmup probe（StepCoro） | `apply_transmit_done`, `apply_vk_sent` |
//! | `MsImeReadyCoro` | `ms_ime_ready_coro.rs` | MS-IME IMC 確認待ち confirm-then-transmit（StepCoro, BUG-13） | なし（Core のみ） |
//! | `TsfProbeCoro` | `probe_fsm.rs` | Chrome probe + LiteralDetect（StepCoro） | `apply_transmit_done` |
//! | `LiteralDetectFsm` | `literal_detect_fsm.rs` | warm パスの post-transmit composition 確認（`LiteralDetectCore` ラッパー） | なし（Core のみ） |
//! | `UnicodeColdWarmupFsm` | `unicode_cold_warmup_fsm.rs` | Unicode long-cold の deferred chars 送信（手書き FSM） | `push_deferred_unicode_chars` |
//! | `ChromeProbe` | `chrome_probe.rs` | Chrome cold-start GJI readiness probe（内部 `TsfProbeCoro` ラッパー） | `apply_transmit_done`, `apply_vk_sent`（いずれも内部 `TsfProbeCoro` へ委譲） |
//! | `UnicodeLiteralObserverFsm` | `unicode_literal_observer.rs` | Unicode 送信後の GJI write 観測（事後 Tsf 昇格） | なし |
//!
//! `Core`（`tick` / `cold_seq_hint`）は全実装型が実装する。上表は core 以外に
//! オーバーライドするメソッドのみを列挙する（デフォルト no-op を使うため）。
//!
//! probe 進行中に届いた後続 VK（deferred VK）は `TsfWarmupCoordinator` が一元管理する
//! （`push_deferred` は個々の実装が持たない）。

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

    // ── SetOpenTrue/IMEセッション最初の1文字 per-VK confirm ケイパビリティ
    // （GjiWarmupCoro、TsfProbeCoro/ChromeProbe、BUG-24 追補・BUG-27）───────
    //
    // romaji を VK 単位に分割送信する際、各 VK 送信直後に生成した detector を
    // コルーチンへ渡す。複数 VK にわたって呼ばれる。
    //
    // BUG-27（2026-07-17）: `ChromeProbe`（`TsfProbeCoro` のラッパー）がこの
    // メソッドの委譲を欠いていたため、Chrome 側の呼び出しがここのデフォルト
    // no-op に落ち、内側の `TsfProbeCoro::apply_vk_sent` が一度も呼ばれずに
    // `pending_vk_sent` が常に空のままだった。ラッパー型を追加するときは
    // 実装対象メソッドの委譲漏れが無いか、この表と実装を必ず突き合わせること。

    fn apply_vk_sent(&mut self, _detector: LiteralDetector, _deadline_ms: u64) {}

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
