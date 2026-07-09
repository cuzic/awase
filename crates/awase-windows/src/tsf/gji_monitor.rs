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

use super::observer::{ActiveImeKind, TSF_OBS};

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
            read_ops: counters
                .ReadOperationCount
                .saturating_sub(self.last_read_ops),
            write_ops: counters
                .WriteOperationCount
                .saturating_sub(self.last_write_ops),
            other_ops: counters
                .OtherOperationCount
                .saturating_sub(self.last_other_ops),
            read_bytes: counters
                .ReadTransferCount
                .saturating_sub(self.last_read_bytes),
            write_bytes: counters
                .WriteTransferCount
                .saturating_sub(self.last_write_bytes),
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

// ── CLSID ベース IME 種別のデバウンス ──

/// `query_active_kind` の単発フリップを実際の切り替えとして即採用しないためのデバウンス。
///
/// `ITfInputProcessorProfileMgr::GetActiveProfile` は `gji-io-monitor` ワーカースレッド
/// （フォーカスを持たない別 STA スレッド）から 2 秒ごとにポーリングする。フォアグラウンド
/// アプリの実際の TIP 選択と切り離されているため、単発の読み取りが一時的に別の種別を
/// 返すことがある（実測: `send_chrome_gji_reinit_and_poll` が送る実 `VK_IME_OFF→VK_IME_ON`
/// トグル直後に 2146ms 間隔で2回 `[gji-fsm] StartComposition while engine off` が観測され、
/// 2 秒ポーリング周期と一致 — 2026-07-07 ユーザー提供ログ）。
///
/// `set_active_ime_kind`（`output/tsf_warmup_coord.rs`）は種別変化のたびに warmup 戦略
/// （`GjiFsm`/`MsImeStrategy`）を丸ごと新規生成し `OnWarm`/`OnComposing` を破棄するため、
/// 単発フリップをそのまま流すと「Chrome cold-start reinit → 一時的な誤検出 → GjiFsm 再構築
/// → 次の単語も cold → 再度 reinit → …」という自己増幅ループになり、`cold_seq` が単語ごと
/// に発火し続ける（BUG-09 で一度否定された「per-thread GetActiveProfile 固着」仮説とは
/// 別症状・別因果）。
///
/// 同じ新しい種別が 2 回連続（= 前回ポーリングでも候補になっていた）観測されて初めて
/// 確定として扱う。誤検出が単発なら次の tick で元の種別に戻り `candidate` がクリアされる。
struct ImeKindDebounce {
    /// 直近 tick で観測された「まだ確定していない」新種別。
    candidate: Option<ActiveImeKind>,
}

impl ImeKindDebounce {
    const fn new() -> Self {
        Self { candidate: None }
    }

    /// 新しい観測値を投入する。`current` は `TSF_OBS` に確定済みの現在値。
    ///
    /// `observed == current`（変化なし）なら候補をクリアして `None`。
    /// `observed` が前回も候補だった（2 回連続で同じ新種別）なら確定として `Some` を返す。
    /// それ以外（初めて見る新種別）は候補として保持し `None` を返す。
    fn observe(
        &mut self,
        observed: ActiveImeKind,
        current: ActiveImeKind,
    ) -> Option<ActiveImeKind> {
        if observed == current {
            self.candidate = None;
            return None;
        }
        if self.candidate == Some(observed) {
            self.candidate = None;
            Some(observed)
        } else {
            self.candidate = Some(observed);
            None
        }
    }
}

#[cfg(test)]
mod ime_kind_debounce_tests {
    use super::{ActiveImeKind, ImeKindDebounce};

    #[test]
    fn stable_same_kind_never_confirms() {
        let mut d = ImeKindDebounce::new();
        for _ in 0..5 {
            assert_eq!(
                d.observe(
                    ActiveImeKind::GoogleJapaneseInput,
                    ActiveImeKind::GoogleJapaneseInput
                ),
                None
            );
        }
    }

    /// 単発フリップ（1 tick だけ別種別 → 次 tick で元に戻る）は確定させない。
    #[test]
    fn single_tick_flap_is_filtered_out() {
        let mut d = ImeKindDebounce::new();
        // tick 1: 誤検出で MicrosoftIme が混入
        assert_eq!(
            d.observe(
                ActiveImeKind::MicrosoftIme,
                ActiveImeKind::GoogleJapaneseInput
            ),
            None
        );
        // tick 2: 元の GoogleJapaneseInput に戻る → 候補クリア、確定させない
        assert_eq!(
            d.observe(
                ActiveImeKind::GoogleJapaneseInput,
                ActiveImeKind::GoogleJapaneseInput
            ),
            None
        );
    }

    /// 2 回連続で同じ新種別が観測されたら確定として返す。
    #[test]
    fn two_consecutive_same_new_kind_confirms() {
        let mut d = ImeKindDebounce::new();
        assert_eq!(
            d.observe(
                ActiveImeKind::MicrosoftIme,
                ActiveImeKind::GoogleJapaneseInput
            ),
            None
        );
        assert_eq!(
            d.observe(
                ActiveImeKind::MicrosoftIme,
                ActiveImeKind::GoogleJapaneseInput
            ),
            Some(ActiveImeKind::MicrosoftIme)
        );
    }

    /// 確定後、次の観測が current 側の更新を反映して安定すれば再度クリアされる
    /// （呼び出し元が確定値で `current` を更新した後の挙動）。
    #[test]
    fn confirms_then_settles() {
        let mut d = ImeKindDebounce::new();
        d.observe(
            ActiveImeKind::MicrosoftIme,
            ActiveImeKind::GoogleJapaneseInput,
        );
        let confirmed = d.observe(
            ActiveImeKind::MicrosoftIme,
            ActiveImeKind::GoogleJapaneseInput,
        );
        assert_eq!(confirmed, Some(ActiveImeKind::MicrosoftIme));
        // 呼び出し元が TSF_OBS を MicrosoftIme に更新した後の次 tick
        assert_eq!(
            d.observe(ActiveImeKind::MicrosoftIme, ActiveImeKind::MicrosoftIme),
            None
        );
    }

    /// フリップ後、別の値が来ても連続2回条件を満たさない限り確定しない。
    #[test]
    fn differing_candidates_do_not_accumulate_across_kinds() {
        let mut d = ImeKindDebounce::new();
        d.observe(
            ActiveImeKind::MicrosoftIme,
            ActiveImeKind::GoogleJapaneseInput,
        );
        assert_eq!(
            d.observe(
                ActiveImeKind::GoogleJapaneseInput,
                ActiveImeKind::GoogleJapaneseInput
            ),
            None
        );
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
pub fn start_monitor_thread() -> win32_worker::WorkerThread {
    win32_worker::WorkerThread::spawn("gji-io-monitor", move |token| {
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
        // 起動時点の IME 種別を即時取得し、WM_IME_KIND_CHANGED を必ず発行して
        // warmup 戦略（GjiFsm vs MsImeStrategy）を初期化する。
        // ポーリングループは「変化時のみ」発行するため、MS-IME 環境では起動後に
        // WM_IME_KIND_CHANGED が届かず GjiFsm が残り続けるバグを防ぐ。
        if let Some(kind) = super::tip_detector::query_active_kind(mgr) {
            TSF_OBS.set_tsf_active_kind(kind);
            log::info!("[tip-detect] initial IME kind: {kind:?}");
            crate::win32::post_to_main_thread(crate::WM_IME_KIND_CHANGED);
        }
    } else {
        log::warn!("[tip-detect] TSF COM 初期化失敗 — CLSID ベース IME 判定を無効化");
    }

    let mut monitor: Option<GjiMonitor> = None;
    let mut next_attach_ms: u64 = 0;
    // CLSID ベース IME 種別ポーリング次回時刻
    let mut next_clsid_check_ms: u64 = 0;
    // 単発フリップで warmup 戦略を破棄しないためのデバウンス（`ImeKindDebounce` 参照）。
    let mut kind_debounce = ImeKindDebounce::new();

    loop {
        let now = crate::hook::current_tick_ms();

        // CLSID ベース IME 種別を 2 秒ごとにポーリングして更新する。
        // WM_IME_KIND_CHANGED はここだけから発行する（プロセス存在ではなく API で判定）。
        // 同じ新種別が 2 tick 連続で観測されるまでは `TSF_OBS` を更新せず
        // 通知も発行しない（`ImeKindDebounce` — 単発の誤検出で warmup 戦略
        // (`GjiFsm`/`MsImeStrategy`) が丸ごと再構築され、確立済みの
        // OnWarm/OnComposing が失われるのを防ぐ）。
        if let Some((ref mgr, _)) = tsf_ctx {
            if now >= next_clsid_check_ms {
                next_clsid_check_ms = now + 2_000;
                if let Some(kind) = super::tip_detector::query_active_kind(mgr) {
                    let current = TSF_OBS.active_ime_kind();
                    if let Some(confirmed) = kind_debounce.observe(kind, current) {
                        if TSF_OBS.set_tsf_active_kind(confirmed) {
                            log::info!("[tip-detect] IME kind → {confirmed:?}");
                            crate::win32::post_to_main_thread(crate::WM_IME_KIND_CHANGED);
                        }
                    } else if kind != current {
                        log::debug!(
                            "[tip-detect] IME kind candidate {kind:?} (current={current:?}), \
                             awaiting confirmation next tick"
                        );
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
                        log::debug!(
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
