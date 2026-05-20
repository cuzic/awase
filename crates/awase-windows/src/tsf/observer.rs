//! observation 層 — GJI I/O モニタリングと WinEvent 由来の観測値を一元管理する。
//!
//! ここにあるグローバルは書き込み元が限定されている:
//! - `GjiMonitor` バックグラウンドスレッド → `OBS_GJI_LAST_IO_MS`, `OBS_GJI_MONITOR_OK`
//! - `observation_event_proc` (app.rs) → `OBS_GJI_CANDIDATE_VISIBLE`,
//!   `OBS_GJI_CANDIDATE_SHOW_SEQ`, `OBS_FOCUS_NAMECHANGE_SEQ`, `COMPOSITION_PROBE_SEQ`
//!
//! 読み取りは judgement 層 (`probe.rs`) と action 層 (`output.rs`) から行う。

use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetProcessIoCounters, OpenProcess, IO_COUNTERS, PROCESS_QUERY_INFORMATION,
};

// ── WinEvent 由来の観測値（app.rs observation_event_proc → ロジックスレッド）──

/// `wait_for_tsf_cold_settle()` が OBJ_NAMECHANGE を early-exit シグナルとして使うカウンタ。
///
/// `send_eager_tsf_warmup()` 呼び出し時に 0 にリセットされ、
/// WezTerm ウィンドウの OBJ_NAMECHANGE が発火するたびに +1 される。
pub static OBS_FOCUS_NAMECHANGE_SEQ: AtomicU32 = AtomicU32::new(0);

/// `GoogleJapaneseInputCandidateWindow` が `EVENT_OBJECT_SHOW` で表示されるたびに +1 されるカウンタ。
///
/// raw TSF literal 検出用: cold start ローマ字送信後にこのカウンタが増えれば
/// GJI candidate window が開いた（composition 成功）、増えなければ literal ASCII の可能性。
pub static OBS_GJI_CANDIDATE_SHOW_SEQ: AtomicU32 = AtomicU32::new(0);

/// `GoogleJapaneseInputCandidateWindow` が現在表示中かどうかのフラグ。
///
/// `EVENT_OBJECT_SHOW` で `true` に、`EVENT_OBJECT_HIDE` で `false` にセットされる。
/// raw TSF literal 検出でウィンドウが既に表示中かを判定するために使用する。
/// ウィンドウが既に表示中の場合は SHOW イベントが来ないため、GJI I/O 変化で composition を検出する。
pub static OBS_GJI_CANDIDATE_VISIBLE: AtomicBool = AtomicBool::new(false);

/// raw TSF literal 検出の汎用シグナル AtomicU32。
///
/// `OBS_GJI_CANDIDATE_SHOW_SEQ` が変化したとき（SHOW 発火）と
/// 検出タイムアウトタスクの両方が +1 してから `notify_all()` を呼ぶ。
/// `output::raw_tsf_literal_show_or_timeout_async` の `AtomicWatcher` がこれを監視し、
/// SHOW またはタイムアウトのどちらが先に来たかを event-driven に判定する。
pub static COMPOSITION_PROBE_SEQ: AtomicU32 = AtomicU32::new(0);

// ── GJI I/O 観測値（バックグラウンドスレッド → ロジックスレッド）──

/// GJI の最終 I/O 変化時刻 (GetTickCount64 ms)。0 = 未観測。
///
/// バックグラウンドモニタースレッドが更新する。
/// `send_romaji_as_tsf` や `TsfReadinessProbe` が参照する。
pub static OBS_GJI_LAST_IO_MS: AtomicU64 = AtomicU64::new(0);

/// GJI モニターが利用可能か（プロセス発見・ハンドル取得成功）。
pub static OBS_GJI_MONITOR_OK: AtomicBool = AtomicBool::new(false);


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
    /// パイプ・セクション経由 IPC などが OtherOperationCount に計上される
    last_other_ops: u64,
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
            last_other_ops: 0,
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
            || counters.WriteOperationCount != self.last_write_ops
            || counters.OtherOperationCount != self.last_other_ops;
        if changed {
            self.last_read_ops = counters.ReadOperationCount;
            self.last_write_ops = counters.WriteOperationCount;
            self.last_other_ops = counters.OtherOperationCount;
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
    let mut next_attach_ms: u64 = 0;

    loop {
        let now = crate::hook::current_tick_ms();

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

        std::thread::sleep(Duration::from_millis(SAMPLE_INTERVAL_MS));
    }
}
