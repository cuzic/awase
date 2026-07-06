//! Layer 4: warmup オーケストレーション。
//!
//! ADR-030 制定時の 3 層（observer / probe / output）+ 統合層に対し、cold-start
//! warmup の「多段フェーズを時系列に駆動する」責務を担う第 4 層。10ms タイマー
//! (`TIMER_TSF_PROBE`) で駆動される [`tickable_fsm::TickableFsm`] 実装群が、
//! probe → FreshF2 → NameChangeWait → transmit → LiteralDetect → recovery の
//! シーケンスを進め、副作用を持たない [`probe_fsm::ProbeAction`] を emit する。
//! dispatcher（`platform.rs`）と `output/probe_io.rs` がそれを Layer 3（output）と
//! timer 呼び出しに変換して実行する。
//!
//! ## メンバー
//!
//! - [`tickable_fsm`]            — TickableFsm トレイト（family 共通 IF、実装 9 種）
//! - [`probe_fsm`]              — ProbeAction 定義 + TsfProbeCoro + decide_transmit_plan
//! - [`gji_warmup_coro`]        — GjiWarmupCoro（GJI cold-start probe, StepCoro）
//! - [`ms_ime_ready_coro`]      — MsImeReadyCoro（MS-IME IMC 確認待ち, StepCoro, BUG-13）
//! - [`sacr_warmup_coro`]       — SacrificialWarmupCoro（VK_A 犠牲キー暖機）
//! - [`ime_offon_warmup_fsm`]   — ImeOffOnWarmupFsm（VK_IME_OFF→ON 暖機, カウンタ FSM）
//! - [`literal_detect_fsm`]     — LiteralDetectCore/Fsm（literal 検出 単一所在地）
//! - [`unicode_cold_warmup_fsm`]— UnicodeColdWarmupFsm（Unicode long-cold deferred）
//! - [`unicode_literal_observer`]— UnicodeLiteralObserverFsm（GJI write 観測 → Tsf 昇格）
//! - [`chrome_probe`]           — ChromeProbe（Chrome cold-start GJI readiness）
//! - [`cold_warmup`]            — ColdWarmupSequence（Preamble/Eager/Non-eager 分解）
//! - [`warmup_strategy`]        — ImeWarmupStrategy トレイト, MsImeStrategy
//!
//! warm/cold の**判定**（`GjiFsm`）と warmup タイミング FSM（`CompositionFsm`）は
//! ProbeAction を emit しない判断寄り状態機械のため Layer 2（`tsf` 直下）に残す。

pub(crate) mod chrome_probe;
pub(crate) mod cold_warmup;
pub(crate) mod gji_warmup_coro;
pub(crate) mod ime_offon_warmup_fsm;
pub(crate) mod literal_detect_fsm;
pub(crate) mod ms_ime_ready_coro;
pub(crate) mod probe_fsm;
pub(crate) mod sacr_warmup_coro;
pub(crate) mod tickable_fsm;
pub(crate) mod unicode_cold_warmup_fsm;
pub(crate) mod unicode_literal_observer;
pub(crate) mod warmup_strategy;
