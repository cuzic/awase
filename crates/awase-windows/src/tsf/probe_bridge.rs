//! メッセージループ統合 — PROBE_ACTIVE / PROBE_KEY_QUEUE と Win32 メッセージループを橋渡し。
//!
//! `TsfReadinessProbe` の `block_on` 待機中にキーフックが再入しないよう制御し、
//! プローブ完了後に退避キーを `WM_DRAIN_PROBE_QUEUE` 経由で順序保証付きで再配送する。
