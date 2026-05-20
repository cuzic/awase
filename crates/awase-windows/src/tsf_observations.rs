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
    ///
    /// 内部で `win32_async::block_on` を呼び、メッセージループを動かしながら待機する。
    /// `std::thread::sleep` を使わないため、待機中も WinEvent（OBJ_NAMECHANGE 等）が処理される。
    pub fn wait_until_ready(&self, total_max_ms: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        // ネストしたメッセージループ中にキーフックが再入しないようガード
        crate::PROBE_ACTIVE.store(true, Relaxed);
        win32_async::block_on(self.wait_until_ready_async(total_max_ms));
        crate::PROBE_ACTIVE.store(false, Relaxed);
    }

    /// [`wait_until_ready`] の非同期実装。`sleep_ms` を使って待機し、
    /// メッセージループをブロックしない。
    async fn wait_until_ready_async(&self, total_max_ms: u64) {
        /// warmup 後の GJI I/O がこの ms 静止したら settled
        const GJI_IDLE_MS: u64 = 80;
        /// settled 確認後の追加余裕 (ms)
        const POST_IDLE_MARGIN_MS: u64 = 30;
        /// ポーリング間隔 (ms)
        const POLL_MS: u32 = 10;

        let cold_n = self.cold_n;
        let warmup_ms = self.warmup_sent_ms;
        let call_ms = crate::hook::current_tick_ms();
        let min_deadline = warmup_ms.saturating_add(self.min_ms);
        let max_deadline = warmup_ms.saturating_add(total_max_ms);

        if !OBS_GJI_MONITOR_OK.load(Ordering::Relaxed) {
            // GJI プロセス監視不可: max_deadline まで非ブロッキング sleep
            let remaining = max_deadline.saturating_sub(crate::hook::current_tick_ms());
            log::debug!(
                "[tsf-probe] cold={cold_n} fallback fixed sleep {remaining}ms (GJI monitor unavailable)"
            );
            if remaining > 0 {
                win32_async::sleep_ms(u32::try_from(remaining).unwrap_or(u32::MAX)).await;
            }
            let total = crate::hook::current_tick_ms().saturating_sub(call_ms);
            log::debug!("[tsf-probe] cold={cold_n} done (fallback), waited {total}ms");
            return;
        }

        // Phase 1: min_deadline まで無条件待機（I/O 観測は信頼しない）
        let phase1_wait = min_deadline.saturating_sub(crate::hook::current_tick_ms());
        if phase1_wait > 0 {
            log::debug!("[tsf-probe] cold={cold_n} phase1 min wait {phase1_wait}ms");
            win32_async::sleep_ms(u32::try_from(phase1_wait).unwrap_or(u32::MAX)).await;
        }

        // Phase 2: GJI I/O 静止監視
        let p2_start = crate::hook::current_tick_ms();
        let gji_io_at_p2 = OBS_GJI_LAST_IO_MS.load(Ordering::Relaxed);
        let io_after_warmup_at_start = gji_io_at_p2 >= warmup_ms;
        log::debug!(
            "[tsf-probe] cold={cold_n} phase2 polling \
             (max_remaining={}ms, gji_io_idle={}ms, io_after_warmup={io_after_warmup_at_start})",
            max_deadline.saturating_sub(p2_start),
            p2_start.saturating_sub(gji_io_at_p2),
        );

        let mut found_io_after_warmup = io_after_warmup_at_start;

        loop {
            let now = crate::hook::current_tick_ms();
            let gji_io = OBS_GJI_LAST_IO_MS.load(Ordering::Relaxed);

            if gji_io >= warmup_ms {
                found_io_after_warmup = true;
            }

            if now >= max_deadline {
                log::debug!(
                    "[tsf-probe] cold={cold_n} timeout \
                     (warmup+{}ms, gji_io_idle={}ms, io_after_warmup={found_io_after_warmup})",
                    now.saturating_sub(warmup_ms),
                    now.saturating_sub(gji_io),
                );
                break;
            }

            if found_io_after_warmup {
                let gji_idle = now.saturating_sub(gji_io);
                if gji_idle >= GJI_IDLE_MS {
                    let elapsed_from_warmup = now.saturating_sub(warmup_ms);
                    let margin = max_deadline.saturating_sub(now).min(POST_IDLE_MARGIN_MS);
                    log::debug!(
                        "[tsf-probe] cold={cold_n} GJI settled \
                         (idle={gji_idle}ms) at warmup+{elapsed_from_warmup}ms, +{margin}ms margin"
                    );
                    if margin > 0 {
                        #[allow(clippy::cast_possible_truncation)]
                        win32_async::sleep_ms(margin as u32).await;
                    }
                    break;
                }
            }

            win32_async::sleep_ms(POLL_MS).await;
        }

        let total = crate::hook::current_tick_ms().saturating_sub(call_ms);
        log::debug!("[tsf-probe] cold={cold_n} done, waited {total}ms");
    }
}

#[cfg(test)]
#[cfg(windows)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering::SeqCst;
    use std::time::Instant;

    /// テスト間でグローバルな観測 atomic が競合しないようにシリアライズするロック
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// GJI モニター不可のとき、total_max_ms ぶん待機して返る（フォールバックパス）
    #[test]
    fn probe_fallback_waits_total_max_ms() {
        let _g = TEST_LOCK.lock().unwrap();
        OBS_GJI_MONITOR_OK.store(false, SeqCst);

        let start = Instant::now();
        let now_ms = crate::hook::current_tick_ms();
        let probe = TsfReadinessProbe::new(now_ms, 0, 0);
        probe.wait_until_ready(100);

        let elapsed = start.elapsed().as_millis();
        // フォールバック: warmup_ms=now, remaining=100ms → sleep_ms(100)
        assert!(elapsed >= 60, "fallback too short: {elapsed}ms");
        assert!(elapsed < 400, "fallback too long: {elapsed}ms");
    }

    /// GJI モニター有効・warmup 後にすでに 80ms+ 静止していれば即 settled
    #[test]
    fn probe_phase2_detects_already_settled() {
        let _g = TEST_LOCK.lock().unwrap();
        let now_ms = crate::hook::current_tick_ms();

        // warmup 200ms 前、GJI 最終 I/O は warmup の 50ms 後（= 150ms 前）
        // → idle = 150ms > 80ms → settled 検出済み
        let warmup_ms = now_ms.saturating_sub(200);
        let io_ms = warmup_ms + 50;

        OBS_GJI_MONITOR_OK.store(true, SeqCst);
        OBS_GJI_LAST_IO_MS.store(io_ms, SeqCst);

        let start = Instant::now();
        let probe = TsfReadinessProbe::new(warmup_ms, 0, 0); // min_ms=0
        probe.wait_until_ready(1_000);

        let elapsed = start.elapsed().as_millis();
        // 即 settled（margin = POST_IDLE_MARGIN_MS = 30ms 以内）
        assert!(elapsed < 150, "should settle quickly (idle>80ms), got {elapsed}ms");
    }

    /// phase1: min_ms が経過するまで probe は I/O 観測を信頼しない
    #[test]
    fn probe_phase1_min_wait_respected() {
        let _g = TEST_LOCK.lock().unwrap();
        let now_ms = crate::hook::current_tick_ms();

        // GJI は settled 状態だが min_ms=80 のため phase1 で 80ms 待機する
        OBS_GJI_MONITOR_OK.store(true, SeqCst);
        OBS_GJI_LAST_IO_MS.store(now_ms.saturating_sub(200), SeqCst); // 200ms 前に I/O（warmup 前）

        let start = Instant::now();
        let probe = TsfReadinessProbe::new(now_ms, 0, 80); // min_ms=80
        probe.wait_until_ready(300);

        let elapsed = start.elapsed().as_millis();
        // min_ms=80 の phase1 wait + phase2 timeout(no io after warmup)=300ms
        // → 最低 60ms 以上はかかる
        assert!(elapsed >= 60, "phase1 min_wait not respected: {elapsed}ms");
    }

    /// warmup 後に GJI I/O が発生しない場合 max_deadline でタイムアウト
    #[test]
    fn probe_phase2_times_out_when_no_io_after_warmup() {
        let _g = TEST_LOCK.lock().unwrap();
        let now_ms = crate::hook::current_tick_ms();

        // GJI I/O は warmup より前 → warmup 後に I/O なし → タイムアウト
        OBS_GJI_MONITOR_OK.store(true, SeqCst);
        OBS_GJI_LAST_IO_MS.store(now_ms.saturating_sub(5_000), SeqCst);

        let start = Instant::now();
        let probe = TsfReadinessProbe::new(now_ms, 0, 0); // min_ms=0
        probe.wait_until_ready(120);

        let elapsed = start.elapsed().as_millis();
        assert!(elapsed >= 80, "should timeout at ~120ms, got {elapsed}ms");
        assert!(elapsed < 500, "exceeded max by too much: {elapsed}ms");
    }
}
