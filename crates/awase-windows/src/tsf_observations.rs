//! TSF composition context readiness の観測システム。
//!
//! ## 設計
//!
//! `ImeObservations` が「IME ON/OFF」を複数ソースから観測するのと同様に、
//! `TsfObservations` は「TSF composition context が ready か」を複数ソースで観測する。
//!
//! ## 観測ソース（今後拡張予定）
//!
//! 1. `GjiMonitor` — GetProcessIoCounters ベースの I/O 静止検出
//!    - GJI が VK_IME_ON 後に I/O を行い、静止したら初期化完了と推定
//!    - ETW 不要・管理者権限不要
//!
//! 2. 時間ベース（フォールバック）
//!    - GJI プロセスが見つからない場合の従来ロジック
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

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetProcessIoCounters, OpenProcess, IO_COUNTERS, PROCESS_QUERY_INFORMATION,
};

// ── グローバル観測値（バックグラウンドスレッド → ロジックスレッド）──

/// GJI の最終 I/O 変化時刻 (GetTickCount64 ms)。0 = 未観測。
///
/// バックグラウンドモニタースレッドが更新する。
/// `send_romaji_as_tsf` や `TsfReadinessProbe` が参照する。
pub static OBS_GJI_LAST_IO_MS: AtomicU64 = AtomicU64::new(0);

/// GJI モニターが利用可能か（プロセス発見・ハンドル取得成功）。
pub static OBS_GJI_MONITOR_OK: AtomicBool = AtomicBool::new(false);

// ── GJI プロセス発見 ──

const GJI_CONVERTER_EXE: &str = "GoogleJapaneseInputConverter.exe";

/// プロセス一覧から GJI converter の PID を探す。
fn find_gji_pid() -> Option<u32> {
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;

    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut found = None;

    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            let name_end = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..name_end]);
            if name.eq_ignore_ascii_case(GJI_CONVERTER_EXE) {
                found = Some(entry.th32ProcessID);
                break;
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }

    let _ = unsafe { CloseHandle(snapshot) };

    if let Some(pid) = found {
        log::debug!("[gji-monitor] found {GJI_CONVERTER_EXE} pid={pid}");
    } else {
        log::debug!("[gji-monitor] {GJI_CONVERTER_EXE} not found in process list");
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
        let pid = find_gji_pid()?;
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

        // アタッチ試行（未接続 or 再接続待ち）
        if monitor.is_none() && now >= next_attach_ms {
            match GjiMonitor::try_attach() {
                Some(m) => {
                    log::info!("[gji-monitor] attached to {GJI_CONVERTER_EXE}");
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

        // サンプリング
        if let Some(ref mut m) = monitor {
            if !m.sample() {
                // GJI プロセス死亡
                log::info!("[gji-monitor] {GJI_CONVERTER_EXE} exited, will re-attach");
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

// ── TsfReadinessProbe ──

/// TSF composition readiness を観測し「ready になるまで待つ」プローブ。
///
/// `send_romaji_as_tsf` の固定 sleep を置き換える。
///
/// ## 判断ロジック（優先順位）
///
/// 1. GJI モニター利用可能 → I/O が `GJI_IDLE_MS` 静止したら settled
/// 2. GJI モニター利用不可 → 時間ベース（`timeout_ms` まで固定 sleep）
#[derive(Debug)]
pub struct TsfReadinessProbe {
    /// VK_IME_ON を送信した時刻 (ms)。0 = 未送信。
    pub warmup_sent_ms: u64,
    /// ログ相関用 cold-start シーケンス番号。
    pub cold_n: u32,
}

impl TsfReadinessProbe {
    pub const fn new(warmup_sent_ms: u64, cold_n: u32) -> Self {
        Self { warmup_sent_ms, cold_n }
    }

    /// GJI が settled になるまで、最大 `timeout_ms` 待機する。
    ///
    /// - `timeout_ms`: 許容する最大待機時間。GJI settled の場合はそれより早く終わる。
    pub fn wait_until_ready(&self, timeout_ms: u64) {
        /// GJI I/O がこの ms 静止したら settled とみなす
        const GJI_IDLE_MS: u64 = 80;
        /// settled 確認後の追加余裕 (ms)
        const POST_IDLE_MARGIN_MS: u64 = 30;
        /// ポーリング間隔 (ms)
        const POLL_MS: u64 = 10;

        let cold_n = self.cold_n;
        let start_ms = crate::hook::current_tick_ms();
        let deadline_ms = start_ms + timeout_ms;

        if !OBS_GJI_MONITOR_OK.load(Ordering::Relaxed) {
            // フォールバック: 固定 sleep
            let remaining = deadline_ms.saturating_sub(crate::hook::current_tick_ms());
            log::debug!(
                "[tsf-probe] cold={cold_n} fallback fixed sleep {remaining}ms (GJI monitor unavailable)"
            );
            if remaining > 0 {
                std::thread::sleep(Duration::from_millis(remaining));
            }
            let total = crate::hook::current_tick_ms() - start_ms;
            log::debug!("[tsf-probe] cold={cold_n} done (fallback), waited {total}ms");
            return;
        }

        // GJI モニター利用可能: ポーリングループ
        let last_io_at_start = OBS_GJI_LAST_IO_MS.load(Ordering::Relaxed);
        let gji_idle_at_start = start_ms.saturating_sub(last_io_at_start);
        log::debug!(
            "[tsf-probe] cold={cold_n} polling (GJI I/O monitor, timeout={timeout_ms}ms, gji_idle_at_start={gji_idle_at_start}ms)"
        );

        loop {
            let now = crate::hook::current_tick_ms();

            if now >= deadline_ms {
                let last_io_ms = OBS_GJI_LAST_IO_MS.load(Ordering::Relaxed);
                log::debug!(
                    "[tsf-probe] cold={cold_n} timeout after {}ms (gji_idle={}ms)",
                    now.saturating_sub(start_ms),
                    now.saturating_sub(last_io_ms),
                );
                break;
            }

            let last_io_ms = OBS_GJI_LAST_IO_MS.load(Ordering::Relaxed);
            let gji_idle_ms = now.saturating_sub(last_io_ms);

            if gji_idle_ms >= GJI_IDLE_MS {
                // settled → 余裕分だけ追加待機してから送信
                let elapsed = now.saturating_sub(start_ms);
                let margin = deadline_ms.saturating_sub(now).min(POST_IDLE_MARGIN_MS);
                log::debug!(
                    "[tsf-probe] cold={cold_n} GJI settled (idle={gji_idle_ms}ms) after {elapsed}ms, +{margin}ms margin"
                );
                if margin > 0 {
                    std::thread::sleep(Duration::from_millis(margin));
                }
                break;
            }

            std::thread::sleep(Duration::from_millis(POLL_MS));
        }

        let total = crate::hook::current_tick_ms().saturating_sub(start_ms);
        log::debug!("[tsf-probe] cold={cold_n} done, waited {total}ms");
    }
}
