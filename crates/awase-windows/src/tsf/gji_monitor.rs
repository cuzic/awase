//! GJI プロセス I/O モニター。
//!
//! バックグラウンドスレッドが Google 日本語入力 Converter プロセスの
//! I/O カウンタを 10ms ごとにサンプリングし、[`super::observer::TSF_OBS`] を更新する。
//! GJI が再起動した場合は自動的に再接続する。

use std::mem::size_of;
use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetProcessIoCounters, OpenProcess, IO_COUNTERS, PROCESS_QUERY_INFORMATION,
};

use super::observer::TSF_OBS;

// ── GJI プロセス発見 ─────────────────────────────────────────────────────────

/// GJI Converter プロセスのプレフィックス候補（大文字小文字無視）。
/// バージョンによりプロセス名が異なるため複数候補を持つ。
/// Converter プロセスのみを対象にする（Renderer/CacheService は除外）。
const GJI_PROCESS_PREFIXES: &[&str] = &[
    "GoogleIMEJaConverter",         // 現行バージョン
    "GoogleJapaneseInputConverter", // 旧バージョン
];

/// プロセス名が GJI 関連かどうか判定する。
fn is_gji_process(name: &str) -> bool {
    GJI_PROCESS_PREFIXES.iter().any(|prefix| {
        name.to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
    })
}

/// プロセス一覧から GJI converter の PID を探す。
/// マッチしたプロセス名もあわせて返す。
/// 見つからない場合は "Google" を含む全プロセスをデバッグログに出力する。
fn find_gji_pid() -> Option<(u32, String)> {
    // SAFETY: `TH32CS_SNAPPROCESS` フラグと PID=0（全プロセス対象）は有効な引数。
    //         返されるハンドルは使用後に `CloseHandle` で必ず閉じる。
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;

    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut found = None;
    let mut google_procs: Vec<String> = Vec::new();

    // SAFETY: `snapshot` は直上の `CreateToolhelp32Snapshot` が返した有効なハンドル。
    //         `entry` は `dwSize` を設定した有効な `PROCESSENTRY32W` 構造体。
    if unsafe { Process32FirstW(snapshot, &raw mut entry) }.is_ok() {
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
            // SAFETY: `snapshot` は有効なスナップショットハンドル。
            //         `entry` は `Process32FirstW` で初期化済みの有効な構造体。
            if unsafe { Process32NextW(snapshot, &raw mut entry) }.is_err() {
                break;
            }
        }
    }

    // SAFETY: `snapshot` は `CreateToolhelp32Snapshot` が返した有効なハンドル。
    //         ループ終了後は二度使用しないため二重クローズにならない。
    let _ = unsafe { CloseHandle(snapshot) };

    match &found {
        Some((pid, name)) => log::debug!("[gji-monitor] found GJI process: {name} pid={pid}"),
        None => {
            log::debug!(
                "[gji-monitor] no GJI process found (searched prefixes: {GJI_PROCESS_PREFIXES:?}), google/japanese procs: {google_procs:?}",
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
/// [`GjiMonitor::sample`] が返す I/O カウンタ差分。
struct GjiIoDelta {
    /// 前回ポーリングからの ReadOperationCount 差分
    read_ops: u64,
    /// 前回ポーリングからの WriteOperationCount 差分
    write_ops: u64,
    /// 前回ポーリングからの OtherOperationCount 差分（IPC・パイプ等）
    other_ops: u64,
    /// 前回ポーリングからの ReadTransferCount 差分（バイト数）
    read_bytes: u64,
    /// 前回ポーリングからの WriteTransferCount 差分（バイト数）
    write_bytes: u64,
}

impl GjiIoDelta {
    const fn any(&self) -> bool {
        self.read_ops > 0 || self.write_ops > 0 || self.other_ops > 0
    }
}

struct GjiMonitor {
    handle: HANDLE,
    last_read_ops: u64,
    last_write_ops: u64,
    /// パイプ・セクション経由 IPC などが OtherOperationCount に計上される
    last_other_ops: u64,
    /// GJI プロセスの累積 ReadTransferCount（バイト数）
    last_read_bytes: u64,
    /// GJI プロセスの累積 WriteTransferCount（バイト数）
    last_write_bytes: u64,
    /// 最後に I/O 変化を検出した時刻 (GetTickCount64 ms)
    last_change_ms: u64,
    /// 最後に WriteOperationCount が変化した時刻 (GetTickCount64 ms)。0 = 未観測。
    last_write_change_ms: u64,
}

// プロセスハンドルはスレッド非依存なので Send は安全。
// （バックグラウンドスレッドで所有するため必要）
unsafe impl Send for GjiMonitor {}

impl GjiMonitor {
    /// GJI converter プロセスに接続する。失敗したら None。
    fn try_attach() -> Option<Self> {
        let (pid, _name) = find_gji_pid()?;
        // SAFETY: `pid` は `find_gji_pid` で発見した有効なプロセス ID。
        //         `PROCESS_QUERY_INFORMATION` は I/O カウンタ取得に必要な最小権限。
        //         返されるハンドルは `Drop` で `CloseHandle` される。
        let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, false, pid) }.ok()?;

        let now_ms = crate::hook::current_tick_ms();
        let mut monitor = Self {
            handle,
            last_read_ops: 0,
            last_write_ops: 0,
            last_other_ops: 0,
            last_read_bytes: 0,
            last_write_bytes: 0,
            last_change_ms: now_ms,
            last_write_change_ms: 0,
        };
        // ベースライン読み込み（次回 sample との差分比較用）
        let _ = monitor.sample();
        Some(monitor)
    }

    /// I/O カウンタを読んで差分を返す。
    ///
    /// 返り値: `Some(delta)` = プロセス生存（delta.any() が false なら変化なし）、
    ///        `None` = プロセス死亡またはエラー。
    fn sample(&mut self) -> Option<GjiIoDelta> {
        let mut counters = IO_COUNTERS::default();
        // SAFETY: `self.handle` は `try_attach` で `OpenProcess` が返した有効なハンドル。
        //         `counters` は `IO_COUNTERS::default()` で初期化された有効なバッファ。
        if unsafe { GetProcessIoCounters(self.handle, &raw mut counters) }.is_err() {
            return None;
        }
        let delta = GjiIoDelta {
            read_ops: counters.ReadOperationCount.saturating_sub(self.last_read_ops),
            write_ops: counters.WriteOperationCount.saturating_sub(self.last_write_ops),
            other_ops: counters.OtherOperationCount.saturating_sub(self.last_other_ops),
            read_bytes: counters.ReadTransferCount.saturating_sub(self.last_read_bytes),
            write_bytes: counters.WriteTransferCount.saturating_sub(self.last_write_bytes),
        };
        if delta.any() {
            let now_ms = crate::hook::current_tick_ms();
            self.last_read_ops = counters.ReadOperationCount;
            self.last_write_ops = counters.WriteOperationCount;
            self.last_other_ops = counters.OtherOperationCount;
            self.last_read_bytes = counters.ReadTransferCount;
            self.last_write_bytes = counters.WriteTransferCount;
            self.last_change_ms = now_ms;
            if delta.write_ops > 0 {
                self.last_write_change_ms = now_ms;
            }
        }
        Some(delta)
    }

    const fn last_change_ms(&self) -> u64 {
        self.last_change_ms
    }

    const fn last_write_change_ms(&self) -> u64 {
        self.last_write_change_ms
    }

    const fn last_read_ops(&self) -> u64 {
        self.last_read_ops
    }

    const fn last_read_bytes(&self) -> u64 {
        self.last_read_bytes
    }

    const fn last_write_bytes(&self) -> u64 {
        self.last_write_bytes
    }
}

impl Drop for GjiMonitor {
    fn drop(&mut self) {
        // SAFETY: `self.handle` は `try_attach` で `OpenProcess` が返した有効なハンドル。
        //         `Drop` は一度しか呼ばれないため二重クローズにならない。
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

// ── バックグラウンドモニタースレッド ──

/// GJI I/O モニタースレッドを起動する。
///
/// 常駐し、`TSF_OBS.gji_last_io_ms` と `TSF_OBS.gji_monitor_ok` を更新し続ける。
/// GJI が再起動した場合は自動的に再接続する。
/// 起動時に呼ぶこと（1 回のみ）。戻り値の [`win32_worker::WorkerThread`] を
/// アプリ終了まで保持すること（drop 時にスレッドが停止・join される）。
#[must_use]
pub fn start_monitor_thread(base_dir: std::path::PathBuf) -> win32_worker::WorkerThread {
    win32_worker::WorkerThread::spawn("gji-io-monitor", move |token| {
        super::tip_detector::set_base_dir(base_dir);
        monitor_loop(&token);
    })
}


#[expect(clippy::cognitive_complexity)]
fn monitor_loop(token: &win32_worker::ShutdownToken) {
    log::info!("[gji-monitor] thread started");

    // COM STA 初期化 (TSF プロファイル API に必要)
    // S_FALSE (既に初期化済み) も含めて無視する。
    let _ = unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        )
    };

    // TSF プロファイル COM オブジェクトを生成（失敗しても GJI モニタリングは継続）
    let tsf_ctx = super::tip_detector::create_profile_ctx();
    if let Some((ref mgr, ref profiles)) = tsf_ctx {
        super::tip_detector::dump_profiles(mgr, profiles);
        super::tip_detector::discover_and_cache_gji_clsid(mgr, profiles);
        // 起動時点の IME 種別を即時取得
        if let Some(kind) = super::tip_detector::query_active_kind(mgr) {
            TSF_OBS.set_tsf_active_kind(kind);
            log::info!("[tip-detect] initial IME kind: {kind:?}");
        }
    } else {
        log::warn!("[tip-detect] TSF COM 初期化失敗 — CLSID ベース IME 判定を無効化");
    }

    let mut monitor: Option<GjiMonitor> = None;
    let mut next_attach_ms: u64 = 0;
    // CLSID ベース IME 種別ポーリング次回時刻
    let mut next_clsid_check_ms: u64 = 0;

    loop {
        let now = crate::hook::current_tick_ms();

        // CLSID ベース IME 種別を 2 秒ごとにポーリングして更新する。
        // WM_IME_KIND_CHANGED はここだけから発行する（プロセス存在ではなく API で判定）。
        if let Some((ref mgr, _)) = tsf_ctx {
            if now >= next_clsid_check_ms {
                next_clsid_check_ms = now + 2_000;
                if let Some(kind) = super::tip_detector::query_active_kind(mgr) {
                    if TSF_OBS.set_tsf_active_kind(kind) {
                        log::info!("[tip-detect] IME kind → {kind:?}");
                        crate::win32::post_to_main_thread(crate::WM_IME_KIND_CHANGED);
                    }
                }
            }
        }

        if monitor.is_none() && now >= next_attach_ms {
            if let Some(m) = GjiMonitor::try_attach() {
                log::info!("[gji-monitor] attached to GJI process (I/O monitoring enabled)");
                TSF_OBS
                    .gji_last_io_ms
                    .store(m.last_change_ms(), Ordering::Relaxed);
                TSF_OBS.gji_monitor_ok.store(true, Ordering::Release);
                monitor = Some(m);
                // GJI プロセスにアタッチした時点で CLSID を即時確認する。
                // プロセス存在だけでは「GJI がアクティブ IME」とは限らないため、
                // WM_IME_KIND_CHANGED はプロセス存在ではなく CLSID 結果の変化時のみ発行する。
                if let Some((ref mgr, _)) = tsf_ctx {
                    if let Some(kind) = super::tip_detector::query_active_kind(mgr) {
                        if TSF_OBS.set_tsf_active_kind(kind) {
                            log::info!("[tip-detect] IME kind → {kind:?} (on GJI attach)");
                            crate::win32::post_to_main_thread(crate::WM_IME_KIND_CHANGED);
                        }
                    }
                    next_clsid_check_ms = now + 2_000;
                }
            } else {
                TSF_OBS.gji_monitor_ok.store(false, Ordering::Relaxed);
                next_attach_ms = now + crate::tuning::GJI_REATTACH_INTERVAL_MS;
                log::debug!("[gji-monitor] GJI process not found (I/O monitoring unavailable)");
                // プロセス非検出時は WM_IME_KIND_CHANGED を発行しない。
                // CLSID ポーリングが IME 種別を管理する。
            }
        }

        if let Some(ref mut m) = monitor {
            match m.sample() {
                None => {
                    log::info!("[gji-monitor] GJI process exited, will re-attach");
                    TSF_OBS.gji_monitor_ok.store(false, Ordering::Relaxed);
                    monitor = None;
                    next_attach_ms = now + crate::tuning::GJI_REATTACH_INTERVAL_MS;
                    // プロセス消失時も WM_IME_KIND_CHANGED は発行しない。
                    // CLSID ポーリングが次のサイクルで MicrosoftIme を検出して発行する。
                }
                Some(delta) => {
                    TSF_OBS
                        .gji_last_io_ms
                        .store(m.last_change_ms(), Ordering::Relaxed);
                    TSF_OBS
                        .gji_read_op_count
                        .store(m.last_read_ops(), Ordering::Relaxed);
                    TSF_OBS
                        .gji_read_bytes
                        .store(m.last_read_bytes(), Ordering::Relaxed);
                    TSF_OBS
                        .gji_write_bytes
                        .store(m.last_write_bytes(), Ordering::Relaxed);
                    if delta.write_ops > 0 {
                        TSF_OBS
                            .gji_last_write_ms
                            .store(m.last_write_change_ms(), Ordering::Relaxed);
                        log::info!(
                            "[gji-io] WRITE: w_ops=+{} w_KB=+{:.1} \
                             (r_ops=+{} x_ops=+{})",
                            delta.write_ops,
                            delta.write_bytes as f64 / 1024.0,
                            delta.read_ops,
                            delta.other_ops,
                        );
                    } else if delta.any() {
                        log::debug!(
                            "[gji-io] r_ops=+{} w_ops=+{} x_ops=+{} read_KB=+{:.1}",
                            delta.read_ops,
                            delta.write_ops,
                            delta.other_ops,
                            delta.read_bytes as f64 / 1024.0,
                        );
                    }
                    if delta.read_bytes >= 512 * 1024 {
                        log::info!(
                            "[gji-io] HEAVY read: +{:.1}KB \
                             (possible cold-start dictionary reload)",
                            delta.read_bytes as f64 / 1024.0,
                        );
                    }
                }
            }
        }

        if token
            .sleep_ms(crate::tuning::GJI_SAMPLE_INTERVAL_MS)
            .is_break()
        {
            log::info!("[gji-monitor] shutdown signal received, exiting");
            break;
        }
    }
}
