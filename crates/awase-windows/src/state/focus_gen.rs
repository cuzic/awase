//! await をまたぐ書き戻しの失効判定に使う、生の `u32` だった 2 種の世代カウンタの型(ADR-229 W-a)。
//!
//! どちらも「進めて、捕獲した値と等しいか比べる」だけのカウンタで、値の増え方(`wrapping_add(1)`)と
//! 比較の条件(`==` / `!=`)は元の `u32` のときと同じ。型を分けただけで、`FocusGen` を要る所へ
//! `ShiftConvGuardGen` を(あるいは任意の `u32` を)渡すことはコンパイルできない。
//! 順序比較は持たない(折り返しがあるので `<` は意味を持たない)。

/// `Output::ime_mode_focus_gen` の世代。フォーカス変更(`Output::on_ime_mode_focus_changed`、`gji_on_focus_change` から呼ぶ)で進む。
/// `ActuationTarget` が起案時点の値を捕獲し、write 直前に読み直した値と等しいかを見る。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusGen(u32);

impl FocusGen {
    /// 起動直後の値(`Output::new`)。
    pub const INITIAL: Self = Self(0);

    /// テスト用に値を指定して作る(本番では作れない)。本番の世代は `INITIAL` から `next` だけで進める。
    #[cfg(test)]
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// 次の世代(折り返す)。
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }

    /// ログ・journal 用の生の値。
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for FocusGen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// `Output::shift_conv_guard_gen` の世代。`confirm_gate_deadline_override_ms` の所有権を表し、
/// 新しい hold・早期 return・フォーカス変更・`SetOpen(true)` で進む。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShiftConvGuardGen(u32);

impl ShiftConvGuardGen {
    /// 起動直後の値(`Output::new`)。
    pub const INITIAL: Self = Self(0);

    /// 次の世代(折り返す)。
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }

    /// テスト用の生の値。
    #[cfg(test)]
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_gen_next_advances_by_one_and_wraps() {
        assert_eq!(FocusGen::INITIAL.get(), 0);
        assert_eq!(FocusGen::new(4).next(), FocusGen::new(5));
        assert_eq!(FocusGen::new(u32::MAX).next(), FocusGen::INITIAL);
        assert_ne!(FocusGen::new(1), FocusGen::new(2));
    }

    #[test]
    fn shift_conv_guard_gen_next_advances_by_one_and_wraps() {
        let g1 = ShiftConvGuardGen::INITIAL.next();
        assert_eq!(g1.get(), 1);
        assert_ne!(g1, g1.next());
        assert_eq!(
            ShiftConvGuardGen(u32::MAX).next(),
            ShiftConvGuardGen::INITIAL
        );
    }

    #[test]
    fn focus_gen_displays_as_the_raw_number() {
        assert_eq!(FocusGen::new(7).to_string(), "7");
    }
}
