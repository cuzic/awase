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
//! 2. `SessionIpcMonitor` — session.ipc の LastAccessTime 監視
//!    - `%USERPROFILE%\AppData\LocalLow\Google\Google Japanese Input\session.ipc`
//!    - TSF が GJI に session request を送ると atime が更新される直接シグナル
//!    - GetFileTime(LastAccessTime) を 10ms ごとにポーリング
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

use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetFileTime, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetProcessIoCounters, OpenProcess, IO_COUNTERS, PROCESS_QUERY_INFORMATION,
};
use windows::core::PCWSTR;

// ── グローバル観測値（バックグラウンドスレッド → ロジックスレッド）──

/// GJI の最終 I/O 変化時刻 (GetTickCount64 ms)。0 = 未観測。
///
/// バックグラウンドモニタースレッドが更新する。
/// `send_romaji_as_tsf` や `TsfReadinessProbe` が参照する。
pub static OBS_GJI_LAST_IO_MS: AtomicU64 = AtomicU64::new(0);

/// GJI モニターが利用可能か（プロセス発見・ハンドル取得成功）。
pub static OBS_GJI_MONITOR_OK: AtomicBool = AtomicBool::new(false);

/// session.ipc の LastAccessTime が最後に変化した時刻 (GetTickCount64 ms)。0 = 未観測。
///
/// TSF が GJI Converter に session request を送ると atime が更新される。
/// `TsfReadinessProbe` が `OBS_GJI_LAST_IO_MS` と並行して参照する。
pub static OBS_GJI_SESSION_ATIME_MS: AtomicU64 = AtomicU64::new(0);

/// session.ipc モニターが利用可能か（ファイルオープン成功）。
pub static OBS_GJI_SESSION_MONITOR_OK: AtomicBool = AtomicBool::new(false);

// ── GJI プロセス発見 ──

/// GJI Converter プロセスのプレフィックス候補（大文字小文字無視）。
/// バージョンによりプロセス名が異なるため複数候補を持つ。
/// Converter プロセスのみを対象にする（Renderer/CacheService は除外）。
const GJI_PROCESS_PREFIXES: &[&str] = &[
    "GoogleIMEJaConverter",          // 現行バージョン
    "GoogleJapaneseInputConverter",  // 旧バージョン
];

/// プロセス名が GJI 関連かどうか判定する。
fn is_gji_process(name: &str) -> bool {
    GJI_PROCESS_PREFIXES
        .iter()
        .any(|prefix| name.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()))
}

/// プロセス一覧から GJI converter の PID を探す。
/// マッチしたプロセス名もあわせて返す。
/// 見つからない場合は "Google" を含む全プロセスをデバッグログに出力する。
fn find_gji_pid() -> Option<(u32, String)> {
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;

    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut found = None;
    let mut google_procs: Vec<String> = Vec::new();

    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            let name_end = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..name_end]);
            if is_gji_process(&name) {
                found = Some((entry.th32ProcessID, name));
                break;
            }
            // 診断用: "Google" または "Japanese" を含むプロセスを収集
            let lower = name.to_ascii_lowercase();
            if lower.contains("google") || lower.contains("japanese") {
                google_procs.push(format!("{}({})", name, entry.th32ProcessID));
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }

    let _ = unsafe { CloseHandle(snapshot) };

    match &found {
        Some((pid, name)) => log::debug!("[gji-monitor] found GJI process: {name} pid={pid}"),
        None => {
            log::debug!(
                "[gji-monitor] no GJI process found (searched prefixes: {:?}), google/japanese procs: {:?}",
                GJI_PROCESS_PREFIXES,
                google_procs,
            );
        }
    }

    found
}

// ── GjiMonitor ──

/// GJI プロセスの I/O を監視し、「静止 = TSF 初期化完了」を検出する。
///
/// `GetProcessIoCounters` で累積 I/O を 10ms ごとにサンプリングし、
/// カウントが変化しなくなった時刻を記録する。
struct GjiMonitor {
    handle: HANDLE,
    last_read_ops: u64,
    last_write_ops: u64,
    /// 最後に I/O 変化を検出した時刻 (GetTickCount64 ms)
    last_change_ms: u64,
}

// プロセスハンドルはスレッド非依存なので Send は安全。
// （バックグラウンドスレッドで所有するため必要）
unsafe impl Send for GjiMonitor {}

impl GjiMonitor {
    /// GJI converter プロセスに接続する。失敗したら None。
    fn try_attach() -> Option<Self> {
        let (pid, _name) = find_gji_pid()?;
        let handle =
            unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, false, pid) }.ok()?;

        let now_ms = crate::hook::current_tick_ms();
        let mut monitor = Self {
            handle,
            last_read_ops: 0,
            last_write_ops: 0,
            last_change_ms: now_ms,
        };
        // ベースライン読み込み（次回 sample との差分比較用）
        let _ = monitor.sample();
        Some(monitor)
    }

    /// I/O カウンタを読んで `last_change_ms` を更新する。
    ///
    /// 返り値: プロセスが生存していれば `true`、死亡 or エラーなら `false`。
    fn sample(&mut self) -> bool {
        let mut counters = IO_COUNTERS::default();
        if unsafe { GetProcessIoCounters(self.handle, &mut counters) }.is_err() {
            return false;
        }
        let changed = counters.ReadOperationCount != self.last_read_ops
            || counters.WriteOperationCount != self.last_write_ops;
        if changed {
            self.last_read_ops = counters.ReadOperationCount;
            self.last_write_ops = counters.WriteOperationCount;
            self.last_change_ms = crate::hook::current_tick_ms();
        }
        true
    }

    fn last_change_ms(&self) -> u64 {
        self.last_change_ms
    }
}

impl Drop for GjiMonitor {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

// ── SessionIpcMonitor ──

/// FILETIME (100ns 単位, 1601年起点) を u64 に変換する。
fn filetime_to_u64(ft: FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

/// Path を null 終端 UTF-16 列に変換する。
fn path_to_wide(path: &std::path::Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    wide
}

/// GJI の session.ipc パスを返す。ファイルが存在しない場合は `None`。
///
/// `%USERPROFILE%\AppData\LocalLow\Google\Google Japanese Input\session.ipc`
fn find_session_ipc_path() -> Option<std::path::PathBuf> {
    let profile = std::env::var("USERPROFILE").ok()?;
    let path = std::path::PathBuf::from(profile)
        .join("AppData")
        .join("LocalLow")
        .join("Google")
        .join("Google Japanese Input")
        .join("session.ipc");
    if path.exists() { Some(path) } else { None }
}

/// GJI の `session.ipc` の LastAccessTime を監視し、TSF session request を検出する。
///
/// atime が更新された = GJI Converter がファイルを読んだ = TSF session を処理した。
struct SessionIpcMonitor {
    handle: HANDLE,
    /// 前回サンプル時の LastAccessTime (FILETIME として u64 化)
    last_atime_ft: u64,
}

// ファイルハンドルはスレッド非依存なので Send は安全。
unsafe impl Send for SessionIpcMonitor {}

impl std::fmt::Debug for SessionIpcMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionIpcMonitor")
            .field("last_atime_ft", &self.last_atime_ft)
            .finish_non_exhaustive()
    }
}

impl SessionIpcMonitor {
    /// session.ipc を開いてモニターを初期化する。失敗したら `None`。
    fn try_open() -> Option<Self> {
        let path = find_session_ipc_path()?;
        let wide = path_to_wide(&path);
        // FILE_READ_ATTRIBUTES (0x80): ファイルデータを読まずに属性だけ取得する最小権限
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                0x0080, // FILE_READ_ATTRIBUTES
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .ok()?;
        log::info!(
            "[session-monitor] opened session.ipc: {}",
            path.display()
        );
        let mut mon = Self { handle, last_atime_ft: 0 };
        let _ = mon.sample(); // ベースライン取得
        Some(mon)
    }

    /// LastAccessTime を読み取り、変化があれば `OBS_GJI_SESSION_ATIME_MS` を更新する。
    ///
    /// 返り値: ハンドルが有効なら `true`、エラーなら `false`（再オープン要）。
    fn sample(&mut self) -> bool {
        let mut access = FILETIME::default();
        if unsafe { GetFileTime(self.handle, None, Some(&mut access), None) }.is_err() {
            return false;
        }
        let atime_ft = filetime_to_u64(access);
        if self.last_atime_ft != 0 && atime_ft != self.last_atime_ft {
            let now_ms = crate::hook::current_tick_ms();
            OBS_GJI_SESSION_ATIME_MS.store(now_ms, Ordering::Relaxed);
            log::debug!(
                "[session-monitor] session.ipc atime changed (ft={atime_ft:#018x}), recorded at {now_ms}ms"
            );
        }
        self.last_atime_ft = atime_ft;
        true
    }
}

impl Drop for SessionIpcMonitor {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

// ── バックグラウンドモニタースレッド ──

/// GJI I/O モニタースレッドを起動する。
///
/// 常駐し、`OBS_GJI_LAST_IO_MS` と `OBS_GJI_MONITOR_OK` を更新し続ける。
/// GJI が再起動した場合は自動的に再接続する。
/// 起動時に呼ぶこと（1 回のみ）。
pub fn start_monitor_thread() {
    std::thread::Builder::new()
        .name("gji-io-monitor".to_string())
        .spawn(monitor_loop)
        .expect("failed to spawn gji-io-monitor thread");
}

fn monitor_loop() {
    const SAMPLE_INTERVAL_MS: u64 = 10;
    const REATTACH_INTERVAL_MS: u64 = 3_000;

    log::info!("[gji-monitor] thread started");

    let mut monitor: Option<GjiMonitor> = None;
    let mut session_monitor: Option<SessionIpcMonitor> = None;
    let mut next_attach_ms: u64 = 0;
    let mut next_session_attach_ms: u64 = 0;

    loop {
        let now = crate::hook::current_tick_ms();

        // GJI プロセスへのアタッチ試行
        if monitor.is_none() && now >= next_attach_ms {
            match GjiMonitor::try_attach() {
                Some(m) => {
                    log::info!("[gji-monitor] attached to GJI process");
                    OBS_GJI_MONITOR_OK.store(true, Ordering::Relaxed);
                    OBS_GJI_LAST_IO_MS.store(m.last_change_ms(), Ordering::Relaxed);
                    monitor = Some(m);
                }
                None => {
                    OBS_GJI_MONITOR_OK.store(false, Ordering::Relaxed);
                    next_attach_ms = now + REATTACH_INTERVAL_MS;
                }
            }
        }

        // session.ipc のオープン試行
        if session_monitor.is_none() && now >= next_session_attach_ms {
            match SessionIpcMonitor::try_open() {
                Some(m) => {
                    OBS_GJI_SESSION_MONITOR_OK.store(true, Ordering::Relaxed);
                    session_monitor = Some(m);
                }
                None => {
                    OBS_GJI_SESSION_MONITOR_OK.store(false, Ordering::Relaxed);
                    next_session_attach_ms = now + REATTACH_INTERVAL_MS;
                }
            }
        }

        // GJI プロセス I/O サンプリング
        if let Some(ref mut m) = monitor {
            if !m.sample() {
                log::info!("[gji-monitor] GJI process exited, will re-attach");
                OBS_GJI_MONITOR_OK.store(false, Ordering::Relaxed);
                monitor = None;
                next_attach_ms = now + REATTACH_INTERVAL_MS;
            } else {
                OBS_GJI_LAST_IO_MS.store(m.last_change_ms(), Ordering::Relaxed);
            }
        }

        // session.ipc atime サンプリング
        if let Some(ref mut m) = session_monitor {
            if !m.sample() {
                log::info!("[session-monitor] session.ipc read error, will re-open");
                OBS_GJI_SESSION_MONITOR_OK.store(false, Ordering::Relaxed);
                session_monitor = None;
                next_session_attach_ms = now + REATTACH_INTERVAL_MS;
            }
        }

        std::thread::sleep(Duration::from_millis(SAMPLE_INTERVAL_MS));
    }
}

// ── TsfReadinessProbe ──

/// TSF composition readiness を観測し「ready になるまで待つ」プローブ。
///
/// `send_romaji_as_tsf` の固定 sleep を置き換える。
///
/// ## 判断ロジック（2フェーズ）
///
/// ### Phase 1 — 必須最小待機 (`min_ms` from `warmup_sent_ms`)
///
/// VK_IME_ON 送信直後は GJI の I/O がまだ始まっていない可能性がある。
/// `min_ms` 経過するまでは GJI I/O の観測値を信頼せず待機する。
///
/// ### Phase 2 — GJI I/O 静止監視
///
/// - `last_io >= warmup_ms`（warmup 後に GJI I/O が発生した）かつ
///   80ms 静止 → settled → `POST_IDLE_MARGIN_MS` 待機後に送信
/// - `last_io < warmup_ms`（warmup 後に I/O なし）→ GJI は既に正常状態と推定、
///   max_deadline 到達まで待機継続（万が一 I/O が来れば settled 検出に切り替え）
/// - `now >= max_deadline` → タイムアウト（フォールバック）
///
/// ## `min_ms` の目安（ColdReason 別）
///
/// | 状況 | min_ms |
/// |---|---|
/// | FocusChange / SetOpenTrue / NativeF2Consumed | 300ms |
/// | SessionExpired | 200ms |
/// | PassthroughConfirmKey / ReinjectConfirmKey | 50ms |
/// | SymbolVkSent | 30ms |
/// | その他 | 100ms |
#[derive(Debug)]
pub struct TsfReadinessProbe {
    /// VK_IME_ON を送信した時刻 (GetTickCount64 ms)。
    pub warmup_sent_ms: u64,
    /// ログ相関用 cold-start シーケンス番号。
    pub cold_n: u32,
    /// VK_IME_ON 送信から最低この ms が経過するまで I/O 観測を信頼しない。
    pub min_ms: u64,
}

impl TsfReadinessProbe {
    pub const fn new(warmup_sent_ms: u64, cold_n: u32, min_ms: u64) -> Self {
        Self { warmup_sent_ms, cold_n, min_ms }
    }

    /// GJI が settled になるまで待機する。
    ///
    /// - `total_max_ms`: `warmup_sent_ms` からの最大許容待機時間（タイムアウト）。
    ///   呼び出し時点での残り時間ではなく、VK_IME_ON 送信からの合計予算。
    pub fn wait_until_ready(&self, total_max_ms: u64) {
        /// warmup 後の GJI I/O がこの ms 静止したら settled
        const GJI_IDLE_MS: u64 = 80;
        /// settled 確認後の追加余裕 (ms)
        const POST_IDLE_MARGIN_MS: u64 = 30;
        /// ポーリング間隔 (ms)
        const POLL_MS: u64 = 10;

        let cold_n = self.cold_n;
        let warmup_ms = self.warmup_sent_ms;
        let min_ms = self.min_ms;
        let call_ms = crate::hook::current_tick_ms();
        let min_deadline = warmup_ms.saturating_add(min_ms);
        let max_deadline = warmup_ms.saturating_add(total_max_ms);

        let gji_ok = OBS_GJI_MONITOR_OK.load(Ordering::Relaxed);
        let session_ok = OBS_GJI_SESSION_MONITOR_OK.load(Ordering::Relaxed);

        if !gji_ok && !session_ok {
            // 両モニター利用不可: max_deadline まで固定 sleep
            let remaining = max_deadline.saturating_sub(crate::hook::current_tick_ms());
            log::debug!(
                "[tsf-probe] cold={cold_n} fallback fixed sleep {remaining}ms (both monitors unavailable)"
            );
            if remaining > 0 {
                std::thread::sleep(Duration::from_millis(remaining));
            }
            let total = crate::hook::current_tick_ms().saturating_sub(call_ms);
            log::debug!("[tsf-probe] cold={cold_n} done (fallback), waited {total}ms");
            return;
        }

        // Phase 1: min_deadline まで無条件待機（I/O 観測は信頼しない）
        let phase1_wait = min_deadline.saturating_sub(crate::hook::current_tick_ms());
        if phase1_wait > 0 {
            log::debug!(
                "[tsf-probe] cold={cold_n} phase1 min wait {phase1_wait}ms (warmup+{min_ms}ms not yet elapsed)"
            );
            std::thread::sleep(Duration::from_millis(phase1_wait));
        }

        // Phase 2: GJI 活動監視（I/O カウンタ + session.ipc atime の両シグナル）
        let p2_start = crate::hook::current_tick_ms();
        let gji_io_at_p2 = OBS_GJI_LAST_IO_MS.load(Ordering::Relaxed);
        let session_atime_at_p2 = OBS_GJI_SESSION_ATIME_MS.load(Ordering::Relaxed);
        // どちらか新しい方をベースとして採用
        let last_io_at_p2 = gji_io_at_p2.max(session_atime_at_p2);
        let io_after_warmup_at_start = last_io_at_p2 >= warmup_ms;
        log::debug!(
            "[tsf-probe] cold={cold_n} phase2 polling \
             (max_remaining={}ms, gji_io_idle={}ms, session_atime_idle={}ms, io_after_warmup={io_after_warmup_at_start})",
            max_deadline.saturating_sub(p2_start),
            p2_start.saturating_sub(gji_io_at_p2),
            p2_start.saturating_sub(session_atime_at_p2),
        );

        // warmup 後に GJI 活動が発生したかをトラッキング
        let mut found_io_after_warmup = io_after_warmup_at_start;

        loop {
            let now = crate::hook::current_tick_ms();
            let gji_io = OBS_GJI_LAST_IO_MS.load(Ordering::Relaxed);
            let session_atime = OBS_GJI_SESSION_ATIME_MS.load(Ordering::Relaxed);
            // 両シグナルのうち新しい方を使う（どちらかが warmup 後に動けば settled 対象）
            let last_io = gji_io.max(session_atime);

            if last_io >= warmup_ms {
                found_io_after_warmup = true;
            }

            if now >= max_deadline {
                log::debug!(
                    "[tsf-probe] cold={cold_n} timeout \
                     (warmup+{}ms, gji_io_idle={}ms, session_atime_idle={}ms, io_after_warmup={found_io_after_warmup})",
                    now.saturating_sub(warmup_ms),
                    now.saturating_sub(gji_io),
                    now.saturating_sub(session_atime),
                );
                break;
            }

            if found_io_after_warmup {
                // warmup 後に活動確認済み → 静止を待つ
                let gji_idle = now.saturating_sub(last_io);
                if gji_idle >= GJI_IDLE_MS {
                    let elapsed_from_warmup = now.saturating_sub(warmup_ms);
                    let margin = max_deadline.saturating_sub(now).min(POST_IDLE_MARGIN_MS);
                    log::debug!(
                        "[tsf-probe] cold={cold_n} GJI settled \
                         (idle={gji_idle}ms, gji_io={gji_io}ms, session_atime={session_atime}ms) \
                         at warmup+{elapsed_from_warmup}ms, +{margin}ms margin"
                    );
                    if margin > 0 {
                        std::thread::sleep(Duration::from_millis(margin));
                    }
                    break;
                }
            }
            // found_io_after_warmup=false: GJI が既に正常状態か未応答。
            // max_deadline まで待機継続（活動が来れば上記 settled 検出に切り替わる）。

            std::thread::sleep(Duration::from_millis(POLL_MS));
        }

        let total = crate::hook::current_tick_ms().saturating_sub(call_ms);
        log::debug!("[tsf-probe] cold={cold_n} done, waited {total}ms");
    }
}
