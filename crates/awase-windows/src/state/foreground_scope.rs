//! 前景窓のスコープ(pid + hwnd)。OS を読む `win32::foreground_scope()` の戻り値型で、型自体は純粋。

/// post-bypass latch のスコープ。武装時と評価時で必ず同じ関数で採る。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForegroundScope {
    pub pid: u32,
    /// `GetForegroundWindow()` の生値（`HWND` は `Send` でないため isize で持つ）。
    pub hwnd: isize,
}

impl ForegroundScope {
    /// 取得失敗（前景窓なし・pid 0）。実在のスコープとは決して等しくならない。
    pub const INVALID: Self = Self { pid: 0, hwnd: 0 };

    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.pid != 0 && self.hwnd != 0
    }
}
