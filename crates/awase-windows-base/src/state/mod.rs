// core の `state/mod.rs` と同じ `#![allow(unreachable_pub)]`（再エクスポートの連鎖を支える `pub mod`）。
#![allow(unreachable_pub)]

//! 土台の `state`（ADR-274 段階 2）: 世代・時計・IME 種別・アプリ分類など、他のどれにも依存しない型。
//!
//! `awase-windows-core` の `state/` から、分割前と同じ `src/state/` 内の相対パスのまま移した（`git mv`、類似度 100%）。
//! 上位の crate は `pub use awase_windows_base::state::*;` で再公開するので、`crate::state::TickMs` などの経路は変わらない。

// ── TickMs ─────────────────────────────────────────────────────────────────────

/// `GetTickCount64` 由来のミリ秒タイムスタンプを表すニュータイプ。
///
/// state/ 層が `hook::current_tick_ms()` を直接呼び出す代わりに、
/// 呼び出し元（runtime 層）からタイムスタンプを注入するために使う。
/// これにより state/ が hook 実装に依存しない純粋な型になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, serde::Serialize)]
pub struct TickMs(pub u64);

impl TickMs {
    /// `self - base` を飽和演算で計算して返す。
    #[must_use]
    pub const fn saturating_sub(self, base: u64) -> u64 {
        self.0.saturating_sub(base)
    }
}

pub use generation::{ApplyGeneration, GenerationAllocator};

// ADR-106 決定1: `ApplyGeneration` 専用アロケータ。`ImeEventLog.next_seq` から
// 独立させ、fence 用の識別子が別目的の数を借用する問題（原因A）を解消する。
// ungated（Linux で `allocate()` の単調増加・折り返し・wire エンコード往復を
// 全数テストするため）。`ImeEvent`/`ImeTransition`（ungated）が `ApplyGeneration`
// を保持するため非 Windows でも使用される。
pub mod generation;
// ADR-082「第一歩」: EventOrigin/Generation/EventSource の最小実装。既存コードへの
// 配線はまだ無い（モジュール冒頭のスコープ節参照）。
pub mod event_origin;
pub mod hub_clock;
// FCIS P1: 純粋なデータ型を ungated へ移したもの(元の場所から pub use で再公開)。
pub mod foreground_scope;
// output/types.rs から移設（InjectionHint 依存の From 実装のみ output/ に残す）。
// 唯一の ungated 呼び出し元は tsf::gji_fsm。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod injection_mode;
// ADR-223 段階 0: 入力言語(HKL)の判定(純粋関数。Win32 を呼ばないので Linux でもテストできる)。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod layout_language;
// ADR-089 §2.8「K 軸の型」。`caps(p, k)` の導入（Phase C）に先立ち、Linux で
// 全数テストできる ungated な IME 種別を置く。変換は `tsf/observer.rs` の
// `From<ActiveImeKind>` 1 箇所のみ。
pub mod ime_kind;
// ADR-229 W-a: await をまたぐ失効判定に使う生の `u32` 世代 2 種(`Output::ime_mode_focus_gen`・
// `shift_conv_guard_gen`)の型。ungated(Linux で `next`/比較をテストする)。
pub mod focus_gen;
// hook.rs (#[cfg(windows)]) の唯一の呼び出し元。alt_impersonation と同じ
// 「純粋判定を Linux でテストできるようにする」移設パターン。
#[cfg_attr(not(windows), allow(dead_code))]
pub mod app_suppression;
pub mod scoped_latch;
