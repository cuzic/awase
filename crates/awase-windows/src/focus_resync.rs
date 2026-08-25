//! フォーカス復帰後 resync（report `01M0VGJ2M5KQHD1D9V7HAMBHNT`）の armed/gate 管理。
//!
//! Alt+Tab 等でフォーカスが一瞬離れて復帰した直後、最初の物理キー入力が
//! resync（IME 状態の再確認）完了前に PassThrough でリテラル出力されるバグの修正。
//! `state/focus_resync_policy.rs` の純粋判定を、実際の状態遷移として持ち回る。
//!
//! ## 状態遷移
//!
//! ```text
//! [フォーカス変更]        arm()                → armed=true
//! [最初の resync 対象キー] consume_and_close()  → armed=false, gate_active=true, generation+=1
//! [resync 完了 or 期限]   open_if_current(gen) → 最初に呼んだ方だけ gate_active=false で true を返す
//! ```
//!
//! `open_if_current` は世代照合 + `compare_exchange` により、resync 完了と
//! ハード期限タイマーが競合しても「最初に到達した方だけが drain を post し、
//! 遅れて届いた方の結果は破棄する」ことを保証する（BUG-31/BUG-70 系の
//! 「タイピング中に遅れて belief が書き換わる」事故を防ぐ）。
//!
//! disarm（`disarm()`）は設計上、以下の契機で呼ぶべきものとして用意してある
//! （有効期限は付けない——理由は `state/focus_resync_policy.rs` および
//! `docs/known-bugs.md` BUG-77 参照）:
//! - 次のフォーカス変更（`arm()` が armed を再度 true にするので自然に上書きされる。
//!   これは実際に配線済み）
//! - 明示的な IME 操作（変換/無変換/F2 等）— **未配線**
//! - エンジン無効化 — **未配線**
//!
//! 後2者は `docs/known-bugs.md` BUG-77 が明記する既知の限界として意図的に
//! 未配線のまま残している（正しい統合ポイントの実機検証待ち）。呼ばれなくても
//! 安全側に働く——ガード4（`EXPLICIT_IME_SUPPRESS_MS`）が同じ状況で resync の
//! conv 読み取り自体を棄却するため、disarm が無いと「最大
//! `FOCUS_RESYNC_DEADLINE_MS` だけ無駄に待つ」だけで、誤った belief が
//! 適用されることはない。**この doc を読んで「明示的 IME 操作で armed が
//! クリアされている」と仮定した設計を上に積まないこと。**

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// フォーカス復帰後 resync の armed/gate 状態。
#[derive(Debug)]
pub struct FocusResyncGate {
    armed: AtomicBool,
    gate_active: AtomicBool,
    generation: AtomicU64,
    armed_at_ms: AtomicU64,
}

impl Default for FocusResyncGate {
    fn default() -> Self {
        Self::new()
    }
}

impl FocusResyncGate {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            gate_active: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            armed_at_ms: AtomicU64::new(0),
        }
    }

    /// フォーカス変更時に呼ぶ。`now_ms` は arm した時刻（診断ログ用）。
    pub fn arm(&self, now_ms: u64) {
        self.armed_at_ms.store(now_ms, Ordering::Relaxed);
        self.armed.store(true, Ordering::Relaxed);
    }

    /// 明示的 IME 操作 / エンジン無効化時に呼ぶ。
    pub fn disarm(&self) {
        self.armed.store(false, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_armed(&self) -> bool {
        self.armed.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn is_gate_active(&self) -> bool {
        self.gate_active.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn armed_at_ms(&self) -> u64 {
        self.armed_at_ms.load(Ordering::Relaxed)
    }

    /// 現在の resync 世代番号。`open_if_current` に渡す値をタイマー側が
    /// 再取得する用（consume 側が返した値を保持できない Windows タイマー
    /// コールバックのため）。
    #[must_use]
    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// arm を消費し gate を閉じ（=defer 中）、この resync 世代番号を返す。
    ///
    /// 呼び出し元（`app/mod.rs` の trigger 分岐）はこの1回だけ呼ぶこと
    /// （`RawKeyEvent::starts_focus_resync()` が true の最初のキーでのみ到達する）。
    #[must_use]
    pub fn consume_and_close(&self) -> u64 {
        self.armed.store(false, Ordering::Relaxed);
        self.gate_active.store(true, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// resync 完了（conv read 完了）またはハード期限到達時に呼ぶ。
    ///
    /// `generation` が現在の世代と一致し、かつ gate がまだ active（=自分より先に
    /// 誰も閉じていない）ときだけ gate を閉じて `true` を返す。すでに閉じられて
    /// いるか世代が古い（新しい resync サイクルに上書きされた）場合は `false` を
    /// 返す——呼び出し元はこの結果を「自分の resync 結果を適用してよいか」の
    /// 判定にも使うこと（`false` なら belief 適用も drain post も行わない）。
    #[must_use]
    pub fn open_if_current(&self, generation: u64) -> bool {
        if self.generation.load(Ordering::Relaxed) != generation {
            return false;
        }
        self.gate_active
            .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }
}

pub static FOCUS_RESYNC: FocusResyncGate = FocusResyncGate::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_sets_armed_and_timestamp() {
        let g = FocusResyncGate::new();
        assert!(!g.is_armed());
        g.arm(1_000);
        assert!(g.is_armed());
        assert_eq!(g.armed_at_ms(), 1_000);
    }

    #[test]
    fn disarm_clears_armed() {
        let g = FocusResyncGate::new();
        g.arm(1_000);
        g.disarm();
        assert!(!g.is_armed());
    }

    #[test]
    fn consume_and_close_is_one_shot() {
        let g = FocusResyncGate::new();
        g.arm(1_000);
        assert!(g.is_armed());
        let gen1 = g.consume_and_close();
        assert!(!g.is_armed(), "consume した後は armed=false（2回目の消費が起きない）");
        assert!(g.is_gate_active());
        assert_eq!(g.current_generation(), gen1);
    }

    #[test]
    fn consume_and_close_advances_generation_each_time() {
        let g = FocusResyncGate::new();
        g.arm(1_000);
        let gen1 = g.consume_and_close();
        g.arm(2_000);
        let gen2 = g.consume_and_close();
        assert_ne!(gen1, gen2);
    }

    #[test]
    fn open_if_current_rejects_stale_generation() {
        // 期限先行後、遅れて届いた古い世代の resync 結果は破棄されること。
        let g = FocusResyncGate::new();
        g.arm(1_000);
        let gen1 = g.consume_and_close();
        // 期限タイマーが先に到達して gate を閉じる。
        assert!(g.open_if_current(gen1));
        assert!(!g.is_gate_active());
        // 次のフォーカス変更で新しい resync サイクルが始まる。
        g.arm(2_000);
        let gen2 = g.consume_and_close();
        assert_ne!(gen1, gen2);
        // gen1 の resync 完了がここで遅れて届いても、世代不一致で棄却される。
        assert!(!g.open_if_current(gen1));
        // gen2 は現行世代なので通る。
        assert!(g.open_if_current(gen2));
    }

    #[test]
    fn open_if_current_only_first_caller_wins_for_same_generation() {
        // resync 完了とハード期限タイマーが同じ世代に対して競合した場合、
        // 最初に呼んだ方だけが true を得る（二重 drain post の防止）。
        let g = FocusResyncGate::new();
        g.arm(1_000);
        let gen = g.consume_and_close();
        assert!(g.open_if_current(gen), "1件目は成功する");
        assert!(!g.open_if_current(gen), "2件目（同一世代）は失敗する");
    }

    #[test]
    fn open_if_current_without_prior_consume_fails() {
        // consume_and_close を一度も呼んでいない（generation=0, gate 非active）
        // 状態で open_if_current(0) を呼んでも、gate が active でないため失敗する。
        let g = FocusResyncGate::new();
        assert!(!g.open_if_current(0));
    }
}
