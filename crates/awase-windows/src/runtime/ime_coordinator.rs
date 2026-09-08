//! IME apply・パニック回復の調停を担う `ImeCoordinator`。
//!
//! `on_ime_apply_complete`（sync/async 両経路の単一入口）/ `panic_reset` など
//! IME 適用結果の後処理と緊急回復に関連する状態をここに集約する。

use awase::types::RawKeyEvent;

/// IME 適用・パニック回復に関する状態の集約点。
///
/// # フィールド
/// - `pending_ime_off_rescue` — Ctrl+無変換 IME OFF 救済窓中に保留している event。
///   `TIMER_IME_OFF_RESCUE` 満了で IME OFF 発火、Ctrl↑ 到達で ctrl=false に書き換えて発火。
///   `Some` 中に他のキーが到着したら救済中止して原 event を engine に渡す。
/// - `deferred_engine_timers` — OUTPUT_GATE active 中に発火したエンジンタイマー
///   （TIMER_PENDING / TIMER_SPECULATIVE）の (logical_id, os_id) リスト。
///   drain 完了後に `handle_wm_drain_output_queue` が replay する。
///   os_id を一緒に保存することで、drain 中に元のタイマーが kill → 別の新規タイマーが
///   セットされた場合に誤って新タイマーを発火させないよう照合できる。
/// - `pending_explicit_reassert` — ADR-121 D1: 物理IMEキーの no-op 冪等再送
///   （`Runtime::reassert_explicit_physical_key`）が focus-settle 中で見送られた
///   とき、settle 明けの次回 `TIMER_IME_REFRESH` tick で1回だけ消費する単発
///   フラグ（`Some(open)`）。`schedule_settle_retry` が使うのと同じタイマーに
///   相乗りする（新規タイマーは増やさない）。
#[derive(Debug, Default)]
pub(crate) struct ImeCoordinator {
    pub(crate) pending_ime_off_rescue: Option<RawKeyEvent>,
    pub(crate) deferred_engine_timers: Vec<(usize, usize)>,
    pub(crate) pending_explicit_reassert: Option<bool>,
}

impl ImeCoordinator {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}
