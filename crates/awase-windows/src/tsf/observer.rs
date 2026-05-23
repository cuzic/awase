//! observation 層 — GJI I/O モニタリングと WinEvent 由来の観測値を一元管理する。
//!
//! ここにあるグローバルは書き込み元が限定されている:
//! - `GjiMonitor` バックグラウンドスレッド → `TSF_OBS.gji_last_io_ms`, `TSF_OBS.gji_monitor_ok`
//! - `observation_event_proc` → `TSF_OBS.gji_candidate_visible`,
//!   `TSF_OBS.gji_candidate_show_seq`, `TSF_OBS.focus_namechange_seq`, `TSF_OBS.composition_probe_seq`
//!
//! 読み取りは judgement 層 (`probe.rs`) と action 層 (`output.rs`) から行う。

use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetProcessIoCounters, OpenProcess, IO_COUNTERS, PROCESS_QUERY_INFORMATION,
};

// ── TSF 観測値の集約構造体 ──

/// TSF / GJI 観測値をまとめた構造体。
///
/// 書き込み元:
/// - `GjiMonitor` バックグラウンドスレッド → `gji_last_io_ms`, `gji_monitor_ok`
/// - `observation_event_proc` → `gji_candidate_visible`, `gji_candidate_show_seq`,
///   `focus_namechange_seq`, `composition_probe_seq`
///
/// 読み取りは judgement 層 (`probe.rs`) と action 層 (`output.rs`) から行う。
pub struct TsfObservations {
    /// `wait_for_tsf_cold_settle()` が OBJ_NAMECHANGE を early-exit シグナルとして使うカウンタ。
    ///
    /// `send_eager_tsf_warmup()` 呼び出し時に 0 にリセットされ、
    /// WezTerm ウィンドウの OBJ_NAMECHANGE が発火するたびに +1 される。
    pub focus_namechange_seq: AtomicU32,

    /// `GoogleJapaneseInputCandidateWindow` が `EVENT_OBJECT_SHOW` で表示されるたびに +1 されるカウンタ。
    ///
    /// raw TSF literal 検出用: cold start ローマ字送信後にこのカウンタが増えれば
    /// GJI candidate window が開いた（composition 成功）、増えなければ literal ASCII の可能性。
    pub gji_candidate_show_seq: AtomicU32,

    /// `GoogleJapaneseInputCandidateWindow` が現在表示中かどうかのフラグ。
    ///
    /// `EVENT_OBJECT_SHOW` で `true` に、`EVENT_OBJECT_HIDE` で `false` にセットされる。
    /// raw TSF literal 検出でウィンドウが既に表示中かを判定するために使用する。
    /// ウィンドウが既に表示中の場合は SHOW イベントが来ないため、GJI I/O 変化で composition を検出する。
    pub gji_candidate_visible: AtomicBool,

    /// raw TSF literal 検出の汎用シグナル AtomicU32。
    ///
    /// `gji_candidate_show_seq` が変化したとき（SHOW 発火）と
    /// 検出タイムアウトタスクの両方が +1 してから `notify_all()` を呼ぶ。
    /// `output::raw_tsf_literal_show_or_timeout_async` の `AtomicWatcher` がこれを監視し、
    /// SHOW またはタイムアウトのどちらが先に来たかを event-driven に判定する。
    pub composition_probe_seq: AtomicU32,

    /// GJI の最終 I/O 変化時刻 (GetTickCount64 ms)。0 = 未観測。
    ///
    /// バックグラウンドモニタースレッドが更新する。
    /// `send_romaji_as_tsf` や `TsfReadinessProbe` が参照する。
    pub gji_last_io_ms: AtomicU64,

    /// GJI モニターが利用可能か（プロセス発見・ハンドル取得成功）。
    pub gji_monitor_ok: AtomicBool,
}

impl TsfObservations {
    pub const fn new() -> Self {
        Self {
            focus_namechange_seq:   AtomicU32::new(0),
            gji_candidate_show_seq: AtomicU32::new(0),
            gji_candidate_visible:  AtomicBool::new(false),
            composition_probe_seq:  AtomicU32::new(0),
            gji_last_io_ms:         AtomicU64::new(0),
            gji_monitor_ok:         AtomicBool::new(false),
        }
    }
}

pub static TSF_OBS: TsfObservations = TsfObservations::new();


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
/// 常駐し、`TSF_OBS.gji_last_io_ms` と `TSF_OBS.gji_monitor_ok` を更新し続ける。
/// GJI が再起動した場合は自動的に再接続する。
/// 起動時に呼ぶこと（1 回のみ）。戻り値の [`win32_worker::WorkerThread`] を
/// アプリ終了まで保持すること（drop 時にスレッドが停止・join される）。
pub fn start_monitor_thread() -> win32_worker::WorkerThread {
    win32_worker::WorkerThread::spawn("gji-io-monitor", |token| {
        monitor_loop(token);
    })
}

fn monitor_loop(token: win32_worker::ShutdownToken) {
    const SAMPLE_INTERVAL_MS: u32 = 10;
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
                    TSF_OBS.gji_monitor_ok.store(true, Ordering::Relaxed);
                    TSF_OBS.gji_last_io_ms.store(m.last_change_ms(), Ordering::Relaxed);
                    monitor = Some(m);
                }
                None => {
                    TSF_OBS.gji_monitor_ok.store(false, Ordering::Relaxed);
                    next_attach_ms = now + REATTACH_INTERVAL_MS;
                }
            }
        }

        if let Some(ref mut m) = monitor {
            if !m.sample() {
                log::info!("[gji-monitor] GJI process exited, will re-attach");
                TSF_OBS.gji_monitor_ok.store(false, Ordering::Relaxed);
                monitor = None;
                next_attach_ms = now + REATTACH_INTERVAL_MS;
            } else {
                TSF_OBS.gji_last_io_ms.store(m.last_change_ms(), Ordering::Relaxed);
            }
        }

        if token.sleep_ms(SAMPLE_INTERVAL_MS).is_break() {
            log::info!("[gji-monitor] shutdown signal received, exiting");
            break;
        }
    }
}

// ── WinEvent 観察フック ──

/// `WINEVENT_OUTOFCONTEXT` (0x0000) — コールバックをメッセージループで実行
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;

const EVENT_OBJECT_SHOW: u32 = 0x8002;
const EVENT_OBJECT_HIDE: u32 = 0x8003;
const EVENT_OBJECT_NAMECHANGE: u32 = 0x800C;

const GJI_CANDIDATE_CLASS: &str = "GoogleJapaneseInputCandidateWindow";

/// `SetWinEventHook` の RAII ガード。Drop 時に `UnhookWinEvent` を呼ぶ。
pub struct WinEventHookGuard(pub HWINEVENTHOOK);

impl std::fmt::Debug for WinEventHookGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("WinEventHookGuard").field(&self.0 .0).finish()
    }
}

impl Drop for WinEventHookGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::UI::Accessibility::UnhookWinEvent(self.0);
        }
        log::info!("[obs-hook] uninstalled");
    }
}

/// WinEvent 観察フックを登録し RAII ガードのリストを返す。
///
/// | フック | イベント範囲 | 目的 |
/// |---|---|---|
/// | NAMECHANGE | 0x800C | WezTerm title 変更 → `wait_for_tsf_cold_settle` early-exit |
/// | OBJECT_SHOW/HIDE | 0x8002-0x8003 | GJI candidate window 表示状態追跡 → raw TSF literal 検出用 |
pub fn install_observation_hooks() -> Vec<WinEventHookGuard> {
    use windows::Win32::UI::Accessibility::SetWinEventHook;
    let mut hooks = Vec::new();

    let nc_hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_NAMECHANGE,
            EVENT_OBJECT_NAMECHANGE,
            None,
            Some(observation_event_proc),
            0, 0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if nc_hook.is_invalid() {
        log::warn!("[obs-hook] failed to install NAMECHANGE hook");
    } else {
        hooks.push(WinEventHookGuard(nc_hook));
    }

    let show_hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_HIDE, // SHOW(0x8002)〜HIDE(0x8003) の両方を捕捉
            None,
            Some(observation_event_proc),
            0, 0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if show_hook.is_invalid() {
        log::warn!("[obs-hook] failed to install OBJECT_SHOW/HIDE hook");
    } else {
        log::info!("[obs-hook] OBJECT_SHOW/HIDE hook installed (GJI candidate window visibility tracking)");
        hooks.push(WinEventHookGuard(show_hook));
    }

    hooks
}

/// WinEvent 観察コールバック。NAMECHANGE / IME_SHOW / IME_HIDE / IME_CHANGE を処理する。
unsafe extern "system" fn observation_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    const OBJID_WINDOW: i32 = 0;
    if id_object != OBJID_WINDOW {
        return;
    }

    match event {
        EVENT_OBJECT_NAMECHANGE => {
            let class = hwnd_class_name(hwnd);
            if class.contains("CASCADIA") {
                let seq = TSF_OBS.focus_namechange_seq.fetch_add(1, Ordering::Relaxed) + 1;
                log::debug!("[tsf-settle] OBJ_NAMECHANGE #{seq} class={class}");
                win32_async::notify_all();
            }
        }
        EVENT_OBJECT_SHOW => {
            let class = hwnd_class_name(hwnd);
            if class == GJI_CANDIDATE_CLASS {
                TSF_OBS.gji_candidate_visible.store(true, Ordering::Relaxed);
                let seq = TSF_OBS.gji_candidate_show_seq.fetch_add(1, Ordering::Relaxed) + 1;
                // raw TSF literal 検出用の汎用シグナルも +1（SHOW と timeout の両方で同じ atomic を +1 し
                // AtomicWatcher で event-driven に待機する設計）
                TSF_OBS.composition_probe_seq.fetch_add(1, Ordering::Relaxed);
                log::debug!("[gji-candidate] SHOW #{seq}");
                win32_async::notify_all();
            }
        }
        EVENT_OBJECT_HIDE => {
            let class = hwnd_class_name(hwnd);
            if class == GJI_CANDIDATE_CLASS {
                TSF_OBS.gji_candidate_visible.store(false, Ordering::Relaxed);
                log::debug!("[gji-candidate] HIDE");
            }
        }
        _ => {}
    }
}

/// HWND のウィンドウクラス名を取得する。
fn hwnd_class_name(hwnd: HWND) -> String {
    if crate::win32::non_null_hwnd(hwnd).is_none() {
        return String::new();
    }
    let mut buf = [0u16; 128];
    let len = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(hwnd, &mut buf) };
    if len > 0 {
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        String::new()
    }
}
