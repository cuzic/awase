//! observation 層 — GJI I/O モニタリングと WinEvent 由来の観測値を一元管理する。
//!
//! ここにあるグローバルは書き込み元が限定されている:
//! - `GjiMonitor` バックグラウンドスレッド → `OBS_GJI_LAST_IO_MS`, `OBS_GJI_MONITOR_OK`
//! - `observation_event_proc` (app.rs) → `OBS_GJI_CANDIDATE_VISIBLE`,
//!   `OBS_GJI_CANDIDATE_SHOW_SEQ`, `OBS_FOCUS_NAMECHANGE_SEQ`, `COMPOSITION_PROBE_SEQ`
//!
//! 読み取りは judgement 層 (`probe.rs`) と action 層 (`output.rs`) から行う。
