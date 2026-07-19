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
//! - [`probe_fsm`]              — ProbeAction 定義 + TsfProbeCoro + decide_transmit_plan +
//!   run_per_vk_confirm（Chrome/TSF 共通 per-VK confirm ループ）
//! - [`gji_warmup_coro`]        — GjiWarmupCoro（GJI cold-start probe, StepCoro）
//! - [`ms_ime_ready_coro`]      — MsImeReadyCoro（MS-IME IMC 確認待ち, StepCoro, BUG-13）
//! - [`literal_detect_fsm`]     — LiteralDetectCore/Fsm（literal 検出 単一所在地）
//! - [`unicode_cold_warmup_fsm`]— UnicodeColdWarmupFsm（Unicode long-cold deferred）
//! - [`unicode_literal_observer`]— UnicodeLiteralObserverFsm（GJI write 観測 → Tsf 昇格）
//! - [`chrome_probe`]           — ChromeProbe（Chrome cold-start GJI readiness）
//! - [`cold_warmup`]            — ColdWarmupSequence（`run_start` 単一経路、事前待機なし）
//! - [`warmup_strategy`]        — ImeWarmupStrategy トレイト, MsImeStrategy
//!
//! warm/cold の**判定**（`GjiFsm`）と warmup タイミング FSM（`CompositionFsm`）は
//! ProbeAction を emit しない判断寄り状態機械のため Layer 2（`tsf` 直下）に残す。

pub(crate) mod chrome_probe;
pub(crate) mod cold_warmup;
pub(crate) mod gji_warmup_coro;
pub(crate) mod literal_detect_fsm;
pub(crate) mod ms_ime_ready_coro;
pub(crate) mod probe_fsm;
pub(crate) mod tickable_fsm;
pub(crate) mod unicode_cold_warmup_fsm;
pub(crate) mod unicode_literal_observer;
pub(crate) mod warmup_strategy;
