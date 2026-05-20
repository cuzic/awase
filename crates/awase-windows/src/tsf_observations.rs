//! TSF composition context readiness の観測システム。
//!
//! ## 設計
//!
//! `ImeObservations` が「IME ON/OFF」を複数ソースから観測するのと同様に、
//! `TsfObservations` は「TSF composition context が ready か」を複数ソースで観測する。
//!
//! ## 観測ソース
//!
//! 1. `GjiMonitor` — GetProcessIoCounters ベースの I/O 静止検出
//!    - GJI Converter プロセスの累積 I/O カウンタを 10ms ごとに監視
//!    - 静止（80ms 変化なし）で初期化完了と推定
//!
//! 2. `SessionIpcMonitor` — GJI ディレクトリ内 .ipc ファイルの atime/mtime 監視
//!    - `%LocalAppDataLow%\Google\Google Japanese Input\*.ipc`
//!      (session.ipc, renderer.*.ipc など)
//!    - TSF が GJI に session request を送ると atime が更新される直接シグナル
//!    - GetFileTime(LastAccessTime + LastWriteTime) を 10ms ごとにポーリング
//!    - GJI ディレクトリは SHGetKnownFolderPath(FOLDERID_LocalAppDataLow) で特定
//!
//! 3. 時間ベース（フォールバック）
//!    - 両モニターが利用不可の場合の固定 sleep
//!
//! ## 使い方
//!
//! ```text
//! // 起動時
//! tsf_observations::start_monitor_thread();
//!
//! // send_romaji_as_tsf 内
//! let probe = TsfReadinessProbe::new(warmup_sent_ms, cold_reason);
//! probe.wait_until_ready(timeout_ms); // GJI 静止を待つ or フォールバック
//! // → romaji 送信
//! ```

// GjiMonitor・start_monitor_thread および OBS_GJI_* グローバルは tsf/observer.rs に移動。
// 後方互換のため re-export する。
pub use crate::tsf::observer::{OBS_GJI_LAST_IO_MS, OBS_GJI_MONITOR_OK, start_monitor_thread};

// TsfReadinessProbe は tsf/probe.rs に移動。後方互換のため re-export する。
pub use crate::tsf::probe::TsfReadinessProbe;
