//! 外部から IME の開閉が変えられたことを、読めない窓（`Imm32Unavailable`）で観測する監視窓の状態機械
//! （ADR-205、BUG-172）。
//!
//! 外部プロセスが注入した IME キー（目印なし）の直後だけ短い窓を開き、その窓の中で prefetch 済みの開閉の読み
//! （`IMC_GETOPENSTATUS`）が「基準値」から変わったことを観測したときだけ、実状態への追随を求める。
//! 「Chrome は常に 0」のような環境では読みが変わらないので何も起きない（偽の OFF を採用しない）。
//! Win32 に依存しない（スコープ `S` を外から渡す）ので `#[cfg(windows)]` 外でユニットテストできる。

/// 窓の延長の上限（最初の arm から数えて、窓の何倍まで延ばすか）。注入が続く環境で窓が切れなくなるのを防ぐ。
const MAX_EXTENSION_FACTOR: u64 = 2;

/// 監視窓の判定結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeVerdict {
    /// 窓が無い・切れた・スコープ違い・読めなかった・基準値と同じ。何もしない。
    NoEvidence,
    /// 窓の中で基準値と違う値を読んだ。実状態がこの値へ変わったので追随する。
    Changed(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Armed<S> {
    scope: S,
    first_arm_ms: u64,
    last_arm_ms: u64,
    /// 基準値。arm 前の直近の読み（同じスコープ）か、窓の中の最初の読み。
    baseline: Option<bool>,
}

/// 外部変化の監視窓と、直近の読みの記録。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExternalChangeWatch<S: Copy + PartialEq> {
    armed: Option<Armed<S>>,
    /// 全 refresh の入口で記録する直近の読み（スコープ付き）。基準値の初期値になる。
    last_read: Option<(S, bool)>,
}

impl<S: Copy + PartialEq> ExternalChangeWatch<S> {
    pub(crate) const fn new() -> Self {
        Self {
            armed: None,
            last_read: None,
        }
    }

    /// 直近の読みを記録する（`None`〈読めなかった〉は記録しない）。`observe` の後に呼ぶ。
    pub(crate) fn record_read(&mut self, scope: S, read: Option<bool>) {
        if let Some(v) = read {
            self.last_read = Some((scope, v));
        }
    }

    /// 外部注入の IME キーを見たら呼ぶ。同じスコープの窓が生きていれば基準値を保ったまま延ばす
    /// （延長は最初の arm から `window_ms * 2` までで、窓の寿命は最大でその時点から `window_ms` 後＝`window_ms * 3`）。そうでなければ新しく開き、基準値は直近の読み（同じスコープ）。
    pub(crate) fn arm(&mut self, scope: S, now_ms: u64, window_ms: u64) {
        if let Some(a) = self.armed.as_mut() {
            let alive = a.scope == scope && now_ms.saturating_sub(a.last_arm_ms) <= window_ms;
            if alive {
                let cap = a
                    .first_arm_ms
                    .saturating_add(window_ms.saturating_mul(MAX_EXTENSION_FACTOR));
                a.last_arm_ms = now_ms.min(cap);
                return;
            }
        }
        let baseline = self.last_read.filter(|(s, _)| *s == scope).map(|(_, v)| v);
        self.armed = Some(Armed {
            scope,
            first_arm_ms: now_ms,
            last_arm_ms: now_ms,
            baseline,
        });
    }

    /// 窓が生きているか（消費しない）。スコープが変わった・窓が切れたなら破棄して `false`。
    pub(crate) fn live(&mut self, scope: S, now_ms: u64, window_ms: u64) -> bool {
        self.remaining_ms(scope, now_ms, window_ms).is_some()
    }

    /// 窓の残り時間（ms）。無い・切れた・スコープ違いなら `None`（破棄する）。
    pub(crate) fn remaining_ms(&mut self, scope: S, now_ms: u64, window_ms: u64) -> Option<u64> {
        let a = self.armed?;
        let age = now_ms.saturating_sub(a.last_arm_ms);
        if a.scope != scope || age > window_ms {
            self.armed = None;
            return None;
        }
        Some(window_ms - age)
    }

    /// prefetch 済みの読みを判定する。窓の中で基準値と違う値を読んだら `Changed`（窓を閉じる）。
    /// 基準値が無ければ最初の読みを基準値にする（変化とは扱わない）。
    pub(crate) fn observe(
        &mut self,
        scope: S,
        now_ms: u64,
        window_ms: u64,
        read: Option<bool>,
    ) -> ChangeVerdict {
        if !self.live(scope, now_ms, window_ms) {
            return ChangeVerdict::NoEvidence;
        }
        let Some(v) = read else {
            return ChangeVerdict::NoEvidence;
        };
        let Some(a) = self.armed.as_mut() else {
            return ChangeVerdict::NoEvidence;
        };
        match a.baseline {
            None => {
                a.baseline = Some(v);
                ChangeVerdict::NoEvidence
            }
            Some(b) if b == v => ChangeVerdict::NoEvidence,
            Some(_) => {
                self.armed = None;
                ChangeVerdict::Changed(v)
            }
        }
    }
}

impl<S: Copy + PartialEq> Default for ExternalChangeWatch<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u64 = 300;

    fn armed_after_open_read() -> ExternalChangeWatch<u32> {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.record_read(1, Some(true));
        w.arm(1, 1000, W);
        w
    }

    /// 第0段の実測（run 36545236017）: 注入の 32ms 後の最初の読みが既に 0。arm 前の直近の 1 が基準になり Changed(false)。
    #[test]
    fn first_read_already_closed_after_prior_open_read_is_a_change() {
        let mut w = armed_after_open_read();
        assert_eq!(
            w.observe(1, 1032, W, Some(false)),
            ChangeVerdict::Changed(false)
        );
    }

    /// 「常に 0」の環境: 直近の読みも 0 なので変化ではない。
    #[test]
    fn constant_zero_environment_never_changes() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.record_read(1, Some(false));
        w.arm(1, 1000, W);
        assert_eq!(
            w.observe(1, 1032, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
        assert_eq!(
            w.observe(1, 1092, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
    }

    /// 直近の読みが無いとき、窓内の最初の読みが基準値。その後の変化を拾う。
    #[test]
    fn without_prior_read_first_in_window_read_is_baseline() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.arm(1, 1000, W);
        assert_eq!(w.observe(1, 1020, W, Some(true)), ChangeVerdict::NoEvidence);
        assert_eq!(
            w.observe(1, 1080, W, Some(false)),
            ChangeVerdict::Changed(false)
        );
    }

    /// 開く方向（0→1）も同じ規則で拾う。
    #[test]
    fn opening_direction_is_also_a_change() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.record_read(1, Some(false));
        w.arm(1, 1000, W);
        assert_eq!(
            w.observe(1, 1040, W, Some(true)),
            ChangeVerdict::Changed(true)
        );
    }

    #[test]
    fn expired_window_and_scope_change_yield_no_evidence() {
        let mut w = armed_after_open_read();
        assert_eq!(
            w.observe(1, 1000 + W + 1, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
        let mut w = armed_after_open_read();
        assert_eq!(
            w.observe(2, 1010, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
        // スコープ違いで窓は破棄済み
        assert_eq!(
            w.observe(1, 1020, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
    }

    /// 別スコープの直近の読みは基準値にしない（別窓の値を持ち込まない）。
    #[test]
    fn last_read_from_another_scope_is_not_a_baseline() {
        let mut w = ExternalChangeWatch::<u32>::new();
        w.record_read(9, Some(true));
        w.arm(1, 1000, W);
        assert_eq!(
            w.observe(1, 1030, W, Some(false)),
            ChangeVerdict::NoEvidence
        );
    }

    #[test]
    fn unreadable_read_yields_no_evidence_and_keeps_window() {
        let mut w = armed_after_open_read();
        assert_eq!(w.observe(1, 1030, W, None), ChangeVerdict::NoEvidence);
        assert_eq!(
            w.observe(1, 1090, W, Some(false)),
            ChangeVerdict::Changed(false)
        );
    }

    /// 連続 arm: 同じスコープなら基準値を保ったまま延ばす。延長には上限がある。
    #[test]
    fn re_arm_keeps_baseline_and_extension_is_capped() {
        let mut w = armed_after_open_read();
        w.arm(1, 1200, W); // 0xF0 up + 0xF2 down のような連続注入
        assert_eq!(
            w.observe(1, 1400, W, Some(false)),
            ChangeVerdict::Changed(false),
            "1000 に arm、1200 に再 arm → 窓は 1500 まで。基準値の 1 は保たれる"
        );
        // 上限: 最初の arm(1000)から 2W(=600) を超えて延ばせない
        let mut w = armed_after_open_read();
        for t in [1250, 1500, 1750] {
            w.arm(1, t, W);
        }
        assert_eq!(
            w.remaining_ms(1, 1950, W),
            None,
            "延長は最初の arm から 2W(=1600)まで。窓は 1900 で切れる"
        );
    }

    #[test]
    fn same_value_reads_do_not_close_the_window() {
        let mut w = armed_after_open_read();
        assert_eq!(w.observe(1, 1020, W, Some(true)), ChangeVerdict::NoEvidence);
        assert_eq!(
            w.observe(1, 1100, W, Some(false)),
            ChangeVerdict::Changed(false)
        );
        // Changed で窓は閉じる
        assert_eq!(w.observe(1, 1120, W, Some(true)), ChangeVerdict::NoEvidence);
    }
}
