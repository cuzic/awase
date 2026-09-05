//! Unicode cold-start GJI 起動待ちウォームアップ FSM。
//!
//! ## 動作フロー
//!
//! 1. `Platform::dispatch_gji_response` が `GjiAction::StartProbe { is_long_cold: true }` を
//!    受信し、Unicode モード + deferred chars がある場合に本 FSM をインストールする。
//! 2. 呼び出し元が VK_IME_ON (0x16) を送信して GJI 起動をポークする。
//! 3. 本 FSM が 10ms ごとに `gji_write_bytes()` を監視する。
//! 4. GJI が write した（`gji_write_bytes` 増加）か `WARMUP_TIMEOUT_MS` 経過したら
//!    [`ProbeAction::FlushDeferredUnicodeChars`] を emit して完了する。
//! 5. dispatcher が各文字を `send_unicode_char_direct()` で送信する。
//!
//! ## BUG-112「あ」混入との関係（未検証の仮説、計装のみ・挙動変更なし）
//!
//! ステップ2の「GJI 起動をポーク」は実際には `send_unicode_cold_warmup_keys` が
//! `VK_IME_ON` の直後に `VK_A` DOWN/UP → `VK_BACK` DOWN/UP を送る（GJI に「あ」の
//! composition を開始させてから即座に BS で取り消す犠牲キー）。この FSM 自身は
//! `gji_write_bytes()` の増加だけを「GJI が反応した」証拠として使うが、その増加は
//! `VK_A` 自身の書き込みでも起こりうるため、BS が composition を確実に取り消せた
//! ことまでは保証しない。GJI が cold で処理が遅れていると、BS が composition
//! 確立前に素通りし、犠牲キーの「あ」が破棄されないまま残る可能性がある。
//!
//! **これは BUG-112 の確認された原因ではない。** `docs/known-bugs.md` BUG-112 の
//! 唯一の確定証拠（cache.toml の `Imm32Unavailable` 誤学習有無と「あ」混入の
//! 対応）は IME 制御戦略（`AppImeProfile`）に関するもので、本 FSM が属する
//! `InjectionMode`（`AppKind` から決まる、`AppImeProfile` とは独立の軸）には
//! 直接関係しない。したがって上記は BUG-112 とは**別の、まだ実機で一度も
//! 確認できていない**独立の仮説にすぎない（opus-adversarial-consult 2026-09-04
//! で「候補ウィンドウ可視性は無関係プロセスにも汚染されうる全システム観測であり
//! HIDE 取りこぼしで恒久固着しうる」等の指摘を受け、挙動を変える対策
//! （VK_ESCAPE 送信）は一旦見送った）。
//!
//! そのため本 FSM は **挙動を一切変えず**、flush 時点の関連観測値
//! （write bytes 増分・候補ウィンドウの可視性/遷移・composition active 有無）を
//! ログに残すだけに留める。次に実機で「あ」混入が再現した際、このログと
//! 突き合わせて仮説の真偽を判定するのが目的（`fix-requires-evidence.md`：
//! 回帰テストが書けない未確認仮説は `docs/known-bugs.md` への記録で補う）。

use crate::state::event_origin::Generation;
use crate::tsf::probe_bridge::OutputActiveGuard;
use crate::tsf::warmup::probe_fsm::{ProbeAction, TsfEnvSnapshot};

/// GJI が write するか、このミリ秒以上経過したら deferred chars を送信する。
const WARMUP_TIMEOUT_MS: u64 = 200;

/// Unicode cold-start warm-up FSM。
///
/// VK_IME_ON 送信後に GJI の起動を確認（`gji_write_bytes` 増加）してから deferred chars を送る。
pub(crate) struct UnicodeColdWarmupFsm {
    cold_seq: Generation,
    /// RAII guard — Drop で `OUTPUT_GATE.active=false`（後続キーを INPUT_DEFER に退避）
    _guard: OutputActiveGuard,
    /// VK_IME_ON 送信前に取得した `gji_write_bytes()` ベースライン
    baseline_bytes: u64,
    /// FSM 開始時点（犠牲キー送信直後）の GJI candidate SHOW カウンタのベースライン。
    /// 診断ログ専用（BUG-112 仮説の検証用）——flush 時点までに SHOW が実際に
    /// 発火したかどうかを、現在値の生読み取りではなく「このウィンドウ内での遷移」
    /// として判定するために使う。ADR-079 のエポック fencing と同じ考え方。
    candidate_show_baseline: crate::tsf::observer::Baseline,
    /// GJI が warm になったら送信する Unicode 文字バッファ
    deferred_chars: Vec<char>,
    /// 累積経過時間 (ms)
    elapsed_ms: u64,
}

impl UnicodeColdWarmupFsm {
    pub(crate) fn new(
        cold_seq: Generation,
        deferred_chars: Vec<char>,
        baseline_bytes: u64,
    ) -> Self {
        log::debug!(
            "[unicode-cold-warmup] cold={cold_seq} FSM 開始: {} chars deferred, baseline_bytes={baseline_bytes}",
            deferred_chars.len(),
            cold_seq = cold_seq.value(),
        );
        Self {
            cold_seq,
            _guard: OutputActiveGuard::begin(),
            baseline_bytes,
            candidate_show_baseline: crate::tsf::observer::TSF_OBS.gji_candidate_show.baseline(),
            deferred_chars,
            elapsed_ms: 0,
        }
    }

    fn tick_inner(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.elapsed_ms += 10;
        let current = crate::tsf::observer::gji_write_bytes();
        let gji_wrote = current > self.baseline_bytes;
        let timed_out = self.elapsed_ms >= WARMUP_TIMEOUT_MS;

        if !gji_wrote && !timed_out {
            return vec![];
        }

        // BUG-112 仮説の診断ログ（挙動には一切影響しない）。
        // - write_delta: 犠牲キー VK_A 自身の書き込みでも増加しうるため、
        //   「GJI が反応した」以上の意味は持たない。
        // - candidate_show_changed: このFSM開始後に GJI candidate の SHOW が
        //   一度でも発火したか（ADR-079 のエポック fencing と同じ「遷移」判定。
        //   `env.gji_candidate_visible_now` の生値だけだと前世代の残骸や無関係
        //   プロセスの候補表示と区別できないため使わない）。
        // - candidate_visible_now / ime_composition_active_now: 生の現在値。
        //   candidate_show_changed と食い違う場合の切り分け用。
        let write_delta = current.saturating_sub(self.baseline_bytes);
        let candidate_show_changed = crate::tsf::observer::TSF_OBS
            .gji_candidate_show
            .has_changed(self.candidate_show_baseline);
        let ime_composition_active_now = crate::tsf::observer::ime_composition_active_now();
        log::info!(
            "[unicode-cold-warmup] cold={} gji_wrote={gji_wrote} timed_out={timed_out} \
             elapsed={}ms write_delta={write_delta} candidate_show_changed={candidate_show_changed} \
             candidate_visible_now={} ime_composition_active_now={ime_composition_active_now} \
             (BUG-112仮説の診断ログ、挙動は変えない)",
            self.cold_seq.value(),
            self.elapsed_ms,
            env.gji_candidate_visible_now,
        );

        let chars = std::mem::take(&mut self.deferred_chars);
        log::debug!(
            "[unicode-cold-warmup] cold={} gji_wrote={gji_wrote} timed_out={timed_out} \
             elapsed={}ms → {} chars 送信",
            self.cold_seq.value(),
            self.elapsed_ms,
            chars.len()
        );
        vec![
            ProbeAction::FlushDeferredUnicodeChars(chars),
            ProbeAction::Done,
        ]
    }
}

impl crate::tsf::warmup::tickable_fsm::TickableFsm for UnicodeColdWarmupFsm {
    fn tick(&mut self, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        self.tick_inner(env)
    }

    fn cold_seq_hint(&self) -> Generation {
        self.cold_seq
    }

    fn push_deferred_unicode_chars(&mut self, chars: &[char]) -> bool {
        log::debug!(
            "[unicode-cold-warmup] cold={} in-flight FSM に {} chars 追記 (合計 {} chars)",
            self.cold_seq.value(),
            chars.len(),
            self.deferred_chars.len() + chars.len(),
        );
        self.deferred_chars.extend_from_slice(chars);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsf::warmup::tickable_fsm::TickableFsm;

    // `gji_write_bytes()` はプロセスグローバルで他テストの影響を受けうるため、
    // baseline を `u64::MAX` にして `gji_wrote` を常に false に固定し、
    // `WARMUP_TIMEOUT_MS` のタイムアウト分岐だけで決定論的にテストする。
    // `OutputActiveGuard::begin()` を実際に呼ぶため、`OUTPUT_GATE_TEST_LOCK` で
    // 他の同種テストとの並行実行を排他する（BUG-65 追補2）。
    fn new_fsm(deferred_chars: Vec<char>) -> UnicodeColdWarmupFsm {
        UnicodeColdWarmupFsm::new(Generation::INITIAL, deferred_chars, u64::MAX)
    }

    fn tick_until_timeout(fsm: &mut UnicodeColdWarmupFsm, env: TsfEnvSnapshot) -> Vec<ProbeAction> {
        let mut actions = Vec::new();
        for _ in 0..(WARMUP_TIMEOUT_MS / 10) {
            actions = fsm.tick(env);
            if !actions.is_empty() {
                return actions;
            }
        }
        actions
    }

    #[test]
    fn timeout_flush_carries_all_deferred_chars_regardless_of_candidate_visibility() {
        let _g = crate::tsf::probe_bridge::OUTPUT_GATE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // 診断ログを追加しただけで、flush する action の形・中身は
        // `env.gji_candidate_visible_now` の値に関わらず変化しないことを固定する
        // （挙動を変えていないことの回帰テスト）。
        for candidate_visible in [true, false] {
            let mut fsm = new_fsm(vec!['a', 'b', 'c']);
            let actions = tick_until_timeout(
                &mut fsm,
                TsfEnvSnapshot {
                    gji_candidate_visible_now: candidate_visible,
                    ..Default::default()
                },
            );
            assert!(
                matches!(
                    actions.as_slice(),
                    [ProbeAction::FlushDeferredUnicodeChars(chars), ProbeAction::Done]
                        if chars.as_slice() == ['a', 'b', 'c']
                ),
                "candidate_visible={candidate_visible} でも flush される chars は変わらないはず: {actions:?}"
            );
        }
    }

    #[test]
    fn push_deferred_unicode_chars_appends_to_buffer() {
        let _g = crate::tsf::probe_bridge::OUTPUT_GATE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut fsm = new_fsm(vec!['a']);
        assert!(fsm.push_deferred_unicode_chars(&['b', 'c']));
        let actions = tick_until_timeout(&mut fsm, TsfEnvSnapshot::default());
        let ProbeAction::FlushDeferredUnicodeChars(chars) = &actions[0] else {
            panic!("expected FlushDeferredUnicodeChars, got {actions:?}");
        };
        assert_eq!(chars, &['a', 'b', 'c']);
    }

    #[test]
    fn tick_before_timeout_or_write_returns_no_actions() {
        let _g = crate::tsf::probe_bridge::OUTPUT_GATE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut fsm = new_fsm(vec!['a']);
        let actions = fsm.tick(TsfEnvSnapshot::default());
        assert!(
            actions.is_empty(),
            "baseline=MAX で write は起きず、1 tick(10ms) だけではタイムアウトもしないはず"
        );
    }
}
