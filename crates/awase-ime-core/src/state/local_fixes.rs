//! 実験用（`exp/belief-local-fixes`、本番にはマージしない）: 局所修正の切り替え。スレッドごとに持つので、
//! 1 つのテストプロセスの中で複数の変種を並列に測れる。
use std::cell::Cell;

/// L1: 開閉の予測（true/false とも）で `desired_open` も書く（`last_intent` は作らない）。
pub const L1: u8 = 1;
/// L2: 読めない窓（TsfNative/Imm32Unavailable）への窓切替で `desired_open` を true にする（無条件。明示意図の行は hub の IntentStore が上書きする）。
pub const L2: u8 = 2;
/// L3: `IntentStore` の鍵をスレッド（IME の持ち主）にし、開閉の予測も入れる。
pub const L3: u8 = 4;
/// L2 の拡張: 同じフォーカス中の学習による降格（`AppProfileChanged`）で読めない窓になったときも `desired_open` を true にする。
pub const L2B: u8 = 8;

/// 既定（実験ブランチごとに変える）。
pub const DEFAULT: u8 = 0;

thread_local! {
    static FIXES: Cell<Option<u8>> = const { Cell::new(None) };
    static SCOPE: Cell<usize> = const { Cell::new(0) };
}

#[must_use]
pub fn get() -> u8 {
    FIXES.with(|c| c.get().unwrap_or(DEFAULT))
}

pub fn set(fixes: u8) {
    FIXES.with(|c| c.set(Some(fixes)));
}

#[must_use]
pub fn has(bit: u8) -> bool {
    get() & bit != 0
}

/// L3 の鍵（現在のフォーカス窓のスレッド。0 = 未設定）。ハーネスが窓切替ごとに設定する。
pub fn set_scope(scope: usize) {
    SCOPE.with(|c| c.set(scope));
}

#[must_use]
pub fn scope() -> usize {
    SCOPE.with(Cell::get)
}
