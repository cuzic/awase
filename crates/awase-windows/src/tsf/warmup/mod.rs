//! Layer 4: warmup オーケストレーション。
//!
//! ADR-030 制定時の 3 層（observer / probe / output）+ 統合層に対し、cold-start
//! warmup の「多段フェーズを時系列に駆動する」責務を担う第 4 層。10ms タイマー
//! (`TIMER_TSF_PROBE`) で駆動される [`tickable_fsm::TickableFsm`] 実装群が、
//! 送信前の事前 F2/probe 待機（2026-07-18 に全廃、`docs/known-bugs.md` BUG-24
//! 追補8 参照）を経ずに romaji を送信し、送信後の per-VK confirm /
//! LiteralDetect（`literal_detect_fsm`）で確認・失敗時は backspace + 再送
//! でリカバリする方式を [`probe_fsm::ProbeAction`] として emit する。
//! dispatcher（`platform.rs`）と `output/probe_io.rs` がそれを Layer 3（output）と
//! timer 呼び出しに変換して実行する。
//!
//! ## メンバー
//!
//! - [`tickable_fsm`]            — TickableFsm トレイト（family 共通 IF）
//! - [`probe_fsm`]              — ProbeAction 定義 + TsfProbeCoro + run_per_vk_confirm（per-VK confirm ループ）
//! - [`probe_coro_state`]       — ProbeCoroState（TsfProbeCoro の TickableFsm ラッパー状態: tick/apply_vk_sent/prime）
//! - [`ms_ime_ready_coro`]      — MsImeReadyCoro（MS-IME IMC 確認待ち, StepCoro, BUG-13）
//! - [`literal_detect_fsm`]     — per-VK confirm の literal 回収パラメータと回収アクション（語単位の検出コアは ECRS MB-02 で撤去）
//! - [`mozc_compose_check_fsm`] — MozcComposeCheckFsm（Mozc の最初の送信の後、入力中の文字ができたかを共有メモリで確認。ADR-270）
//! - [`warmup_strategy`]        — ImeWarmupStrategy トレイト, MsImeStrategy
//!
//! Tsf 注入モード（`force_tsf`）用の `GjiWarmupCoro`・`ColdWarmupSequence` は、Tsf 注入経路の撤去
//! （2026-10-22、ADR-0004 追記）とともに削除した。cold 開始は `TsfProbeCoro::new_chrome` だけが担う。
//!
//! warm/cold の**判定**（`GjiFsm`）は ProbeAction を emit しない判断寄り状態機械のため
//! Layer 2（`tsf` 直下）に残す。

pub(crate) mod literal_detect_fsm;
pub(crate) mod mozc_compose_check_fsm;
pub(crate) mod ms_ime_ready_coro;
pub(crate) mod probe_coro_state;
pub(crate) mod probe_fsm;
// `output::probe_io` が Windows 専用のため、擬似窓のテストも Windows でだけ動く(実行は CI の windows-test)。
#[cfg(all(test, windows))]
mod pseudo_window;
pub(crate) mod tickable_fsm;
pub(crate) mod warmup_strategy;
