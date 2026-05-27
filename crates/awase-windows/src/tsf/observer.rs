//! observation 層 — GJI I/O モニタリングと WinEvent 由来の観測値を一元管理する。
//!
//! ## アクセス制御
//!
//! [`TSF_OBS`] は `pub(in crate::tsf)` のためこのモジュール外から直接アクセス不可（コンパイルエラー）。
//! `tsf/` 外のコードは [`tsf_obs()`] 経由でのみ読み取れる。
//!
//! 判断層（`ime_controller` 等）は [`ObservedState::capture_now()`] 経由のスナップショットを使うこと。
//! 直接 [`tsf_obs()`] を呼んではいけない（tick 境界外での非一貫観測の防止）。
//!
//! ## 書き込み元
//!
//! - `GjiMonitor` バックグラウンドスレッド → `TSF_OBS.gji_last_io_ms`, `TSF_OBS.gji_monitor_ok`
//! - `observation_event_proc` → `TSF_OBS.gji_candidate_visible`,
//!   `TSF_OBS.gji_candidate_show`, `TSF_OBS.focus_namechange`, `TSF_OBS.composition_probe`
//!
//! [`ObservedState::capture_now()`]: crate::state::ime_decision_view::ObservedState::capture_now

use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

// ── ChangeCounter ──────────────────────────────────────────────────────────

/// 単調増加シーケンスカウンタ。変化検出パターンをカプセル化する。
///
/// 書き込み元は `notify()` で +1 し、読み取り元は `baseline()` → `has_changed()` のペアで変化を検出する。
#[derive(Debug)]
pub(in crate::tsf) struct ChangeCounter(AtomicU32);

impl ChangeCounter {
    pub(super) const fn new() -> Self {
        Self(AtomicU32::new(0))
    }

    /// カウンタをインクリメントし、新しいシーケンス番号を返す。
    pub(super) fn notify(&self) -> u32 {
        self.0.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// 現在値をベースラインとして取得する。変化を検出したい時点の直前に呼ぶ。
    pub(super) fn baseline(&self) -> Baseline {
        Baseline(self.0.load(Ordering::Relaxed))
    }

    /// ベースライン取得後にカウンタが変化したかどうかを返す。
    pub(super) fn has_changed(&self, b: &Baseline) -> bool {
        self.0.load(Ordering::Relaxed) != b.0
    }

    /// カウンタを 0 にリセットする（ウォームアップ開始時等）。
    pub(super) fn reset(&self) {
        self.0.store(0, Ordering::Relaxed);
    }

    /// `AtomicWatcher` 等が直接参照できるよう内部 `AtomicU32` を返す。
    pub(super) const fn atomic(&self) -> &AtomicU32 {
        &self.0
    }
}

/// [`ChangeCounter`] のベースライン値。
#[derive(Debug, Clone, Copy)]
pub(in crate::tsf) struct Baseline(u32);

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use crate::win32::HwndExt as _;

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
/// - `observation_event_proc` → `gji_candidate_visible`, `gji_candidate_show`,
///   `focus_namechange`, `composition_probe`
///
/// 読み取りは judgement 層 (`probe.rs`) と action 層 (`output.rs`) から行う。
#[derive(Debug)]
pub struct TsfObservations {
    /// `wait_for_tsf_cold_settle()` が OBJ_NAMECHANGE を early-exit シグナルとして使うカウンタ。
    ///
    /// `send_eager_tsf_warmup()` 呼び出し時にリセットされ、
    /// WezTerm ウィンドウの OBJ_NAMECHANGE が発火するたびに +1 される。
    ///
    /// 書き込み: `observation_event_proc`（NAMECHANGE イベント）, `send_eager_tsf_warmup`（リセット）
    /// 読み取り: `namechange_baseline()` / `NamechangeBaseline::fired()` 経由
    pub(in crate::tsf) focus_namechange: ChangeCounter,

    /// `GoogleJapaneseInputCandidateWindow` が `EVENT_OBJECT_SHOW` で表示されるたびに +1 されるカウンタ。
    ///
    /// raw TSF literal 検出用: cold start ローマ字送信後にこのカウンタが増えれば
    /// GJI candidate window が開いた（composition 成功）、増えなければ literal ASCII の可能性。
    pub(in crate::tsf) gji_candidate_show: ChangeCounter,

    /// `GoogleJapaneseInputCandidateWindow` が現在表示中かどうかのフラグ。
    ///
    /// `EVENT_OBJECT_SHOW` で `true` に、`EVENT_OBJECT_HIDE` で `false` にセットされる。
    /// raw TSF literal 検出でウィンドウが既に表示中かを判定するために使用する。
    /// ウィンドウが既に表示中の場合は SHOW イベントが来ないため、GJI I/O 変化で composition を検出する。
    pub(super) gji_candidate_visible: AtomicBool,

    /// raw TSF literal 検出の汎用シグナル。
    ///
    /// `gji_candidate_show` が変化したとき（SHOW 発火）と
    /// 検出タイムアウトタスクの両方が `notify()` を呼ぶ。
    /// `output::raw_tsf_literal_show_or_timeout_async` の `AtomicWatcher` がこれを監視し、
    /// SHOW またはタイムアウトのどちらが先に来たかを event-driven に判定する。
    pub(in crate::tsf) composition_probe: ChangeCounter,

    /// GJI の最終 I/O 変化時刻 (GetTickCount64 ms)。0 = 未観測。
    ///
    /// バックグラウンドモニタースレッドが更新する。
    /// `send_romaji_as_tsf` や `TsfReadinessJudge` が参照する。
    pub(super) gji_last_io_ms: AtomicU64,

    /// GJI モニターが利用可能か（プロセス発見・ハンドル取得成功）。
    pub(super) gji_monitor_ok: AtomicBool,

    /// GJI candidate が SHOW になってから次の `set_ime_apply_latch` 呼び出しまでの間に
    /// 「shadow=OFF なのに候補ウィンドウが表示された（desync）」ことがあったかを記録するラッチ。
    ///
    /// `EVENT_OBJECT_SHOW` で `true` に、`reset_candidate_was_seen()` 呼び出し時に `false` にリセット。
    /// `KanjiToggleStrategy` が shadow=false でも desync を検出して VK_KANJI を送れるようにする。
    pub(super) candidate_was_seen: AtomicBool,
}

impl Default for TsfObservations {
    fn default() -> Self {
        Self::new()
    }
}

impl TsfObservations {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            focus_namechange:      ChangeCounter::new(),
            gji_candidate_show:    ChangeCounter::new(),
            gji_candidate_visible: AtomicBool::new(false),
            composition_probe:     ChangeCounter::new(),
            gji_last_io_ms:        AtomicU64::new(0),
            gji_monitor_ok:        AtomicBool::new(false),
            candidate_was_seen:    AtomicBool::new(false),
        }
    }

    /// GJI 最終 I/O 変化時刻 (ms) を読み取る（Relaxed）。
    pub fn gji_last_io_ms(&self) -> u64 {
        self.gji_last_io_ms.load(Ordering::Relaxed)
    }

    /// GJI モニターが利用可能かを読み取る（Acquire）。
    pub fn gji_monitor_ok(&self) -> bool {
        self.gji_monitor_ok.load(Ordering::Acquire)
    }

    /// GJI candidate window が現在表示中かを読み取る（Relaxed）。
    pub fn gji_candidate_visible(&self) -> bool {
        self.gji_candidate_visible.load(Ordering::Relaxed)
    }

    /// raw TSF literal 検出用汎用シグナルへの参照を返す（`AtomicWatcher` 用）。
    ///
    /// `AtomicWatcher::new` に必要な `&AtomicU32` はこのメソッド経由で取得する。
    pub fn composition_probe_atomic(&self) -> &AtomicU32 {
        self.composition_probe.atomic()
    }
}

/// TSF/GJI 観測値グローバル。
///
/// ## アクセス制御（コンパイルガード）
///
/// `pub(in crate::tsf)` により `tsf/` 外からの直接アクセスはコンパイルエラーになる。
/// `tsf/` 外（`output/`, `runtime/`, etc.）は必ず [`tsf_obs()`] 経由で読み取ること。
///
/// ## 書き込み元
///
/// - `GjiMonitor` バックグラウンドスレッド → `gji_last_io_ms`, `gji_monitor_ok`
/// - `observation_event_proc` → `gji_candidate_visible`, `gji_candidate_show`,
///   `focus_namechange`, `composition_probe`
pub(in crate::tsf) static TSF_OBS: TsfObservations = TsfObservations::new();

/// `TsfObservations` グローバルへの参照を返す。
///
/// `tsf/` 外から TSF/GJI 観測値を読む唯一の正規ルート。
///
/// ## 呼び出し可能なレイヤー
///
/// - `output/` — action 層: live シーケンスカウンタ読み取り（スナップショット不可のため直読）
/// - `runtime/` — observe/poll 層: IME リフレッシュ中の GJI I/O ガード判定
/// - `state::ime_decision_view` — `ObservedState::capture_now()` の実装元
/// - `app::key_pipeline` — フォーカスプローブ結果の構築
///
/// ## 呼び出し禁止レイヤー
///
/// 判断層（`ime_controller` 等）は `ObservedState::capture_now()` 経由のスナップショットを使うこと。
/// `tsf_obs()` を直接呼ぶと tick 境界外での非一貫観測が混入する恐れがある。
pub(crate) fn tsf_obs() -> &'static TsfObservations {
    &TSF_OBS
}

// ── output / observer 層向け名前付き API ──
//
// output/ は tsf_obs() を直接呼ばずこれらの関数を使うこと。
// 各関数の名前が「何を読んでいるか」を呼び出し元で自明にする。

/// GJI プロセスの最終 I/O 変化時刻 (ms) を返す。0 = 未観測。live 読み取り。
pub(crate) fn gji_last_io_ms() -> u64 {
    TSF_OBS.gji_last_io_ms.load(Ordering::Relaxed)
}

/// GJI モニターが利用可能かどうか。live 読み取り。
pub(crate) fn gji_monitor_healthy() -> bool {
    TSF_OBS.gji_monitor_ok.load(Ordering::Acquire)
}

/// OBJ_NAMECHANGE カウンタのベースラインを取得する。
///
/// `SendInput` 等を呼ぶ前に取得し、完了後に `NamechangeBaseline::fired()` で
/// 変化があったかを確認する。
pub(crate) fn namechange_baseline() -> NamechangeBaseline {
    NamechangeBaseline(TSF_OBS.focus_namechange.baseline())
}

/// OBJ_NAMECHANGE カウンタをリセットする（`send_eager_tsf_warmup` 用）。
pub(crate) fn reset_namechange_seq() {
    TSF_OBS.focus_namechange.reset();
}

/// GJI candidate が SHOW になってから次の `reset_candidate_was_seen()` まで `true`。
///
/// `KanjiToggleStrategy` が shadow=false でも desync を検出するために使う。
pub(crate) fn candidate_was_seen() -> bool {
    TSF_OBS.candidate_was_seen.load(Ordering::Relaxed)
}

/// 現時点で GJI candidate window が可視かどうか。診断ログ用 live 読み取り。
pub(crate) fn gji_candidate_visible_now() -> bool {
    TSF_OBS.gji_candidate_visible.load(Ordering::Relaxed)
}

/// `apply_ime_open` 後に `candidate_was_seen` フラグをリセットする。
pub(crate) fn reset_candidate_was_seen() {
    TSF_OBS.candidate_was_seen.store(false, Ordering::Relaxed);
}

/// OBJ_NAMECHANGE カウンタのベースライン値。
///
/// `namechange_baseline()` で取得し、`fired()` で変化を検出する。
pub(crate) struct NamechangeBaseline(Baseline);

impl NamechangeBaseline {
    /// ベースライン取得後にカウンタが変化したかどうかを返す。
    pub(crate) fn fired(&self) -> bool {
        TSF_OBS.focus_namechange.has_changed(&self.0)
    }
}


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
    // SAFETY: `TH32CS_SNAPPROCESS` フラグと PID=0（全プロセス対象）は有効な引数。
    //         返されるハンドルは使用後に `CloseHandle` で必ず閉じる。
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;

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
        // SAFETY: `pid` は `find_gji_pid` で発見した有効なプロセス ID。
        //         `PROCESS_QUERY_INFORMATION` は I/O カウンタ取得に必要な最小権限。
        //         返されるハンドルは `Drop` で `CloseHandle` される。
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
        // SAFETY: `self.handle` は `try_attach` で `OpenProcess` が返した有効なハンドル。
        //         `counters` は `IO_COUNTERS::default()` で初期化された有効なバッファ。
        if unsafe { GetProcessIoCounters(self.handle, &raw mut counters) }.is_err() {
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

    const fn last_change_ms(&self) -> u64 {
        self.last_change_ms
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
pub fn start_monitor_thread() -> win32_worker::WorkerThread {
    win32_worker::WorkerThread::spawn("gji-io-monitor", |token| {
        monitor_loop(&token);
    })
}

fn monitor_loop(token: &win32_worker::ShutdownToken) {
    log::info!("[gji-monitor] thread started");

    let mut monitor: Option<GjiMonitor> = None;
    let mut next_attach_ms: u64 = 0;

    loop {
        let now = crate::hook::current_tick_ms();

        if monitor.is_none() && now >= next_attach_ms {
            if let Some(m) = GjiMonitor::try_attach() {
                log::info!("[gji-monitor] attached to GJI process");
                TSF_OBS.gji_last_io_ms.store(m.last_change_ms(), Ordering::Relaxed);
                TSF_OBS.gji_monitor_ok.store(true, Ordering::Release);
                monitor = Some(m);
            } else {
                TSF_OBS.gji_monitor_ok.store(false, Ordering::Relaxed);
                next_attach_ms = now + crate::tuning::GJI_REATTACH_INTERVAL_MS;
            }
        }

        if let Some(ref mut m) = monitor {
            if m.sample() {
                TSF_OBS.gji_last_io_ms.store(m.last_change_ms(), Ordering::Relaxed);
            } else {
                log::info!("[gji-monitor] GJI process exited, will re-attach");
                TSF_OBS.gji_monitor_ok.store(false, Ordering::Relaxed);
                monitor = None;
                next_attach_ms = now + crate::tuning::GJI_REATTACH_INTERVAL_MS;
            }
        }

        if token.sleep_ms(crate::tuning::GJI_SAMPLE_INTERVAL_MS).is_break() {
            log::info!("[gji-monitor] shutdown signal received, exiting");
            break;
        }
    }
}

// ── WinEvent 観察フック ──

use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_HIDE, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SHOW, WINEVENT_OUTOFCONTEXT,
};

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
        // SAFETY: `self.0` は `SetWinEventHook` が返した有効なフックハンドル。
        //         `Drop` は一度しか呼ばれないため二重解除にならない。
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

    // SAFETY: `observation_event_proc` は `'static` な extern "system" fn ポインタ。
    //         `WINEVENT_OUTOFCONTEXT` によりコールバックはメッセージループスレッドで実行される。
    //         返されたフックは `WinEventHookGuard::drop` で `UnhookWinEvent` される。
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

    // SAFETY: `observation_event_proc` は `'static` な extern "system" fn ポインタ。
    //         `WINEVENT_OUTOFCONTEXT` によりコールバックはメッセージループスレッドで実行される。
    //         返されたフックは `WinEventHookGuard::drop` で `UnhookWinEvent` される。
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
                let seq = TSF_OBS.focus_namechange.notify();
                log::debug!("[tsf-settle] OBJ_NAMECHANGE #{seq} class={class}");
                win32_async::notify_all();
            }
        }
        EVENT_OBJECT_SHOW => {
            let class = hwnd_class_name(hwnd);
            if class == GJI_CANDIDATE_CLASS {
                TSF_OBS.gji_candidate_visible.store(true, Ordering::Relaxed);
                TSF_OBS.candidate_was_seen.store(true, Ordering::Relaxed);
                let seq = TSF_OBS.gji_candidate_show.notify();
                // raw TSF literal 検出用の汎用シグナルも +1（SHOW と timeout の両方が
                // AtomicWatcher で event-driven に待機する設計）
                TSF_OBS.composition_probe.notify();
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
    if hwnd.non_null().is_none() {
        return String::new();
    }
    let mut buf = [0u16; 128];
    // SAFETY: `hwnd` は `non_null()` チェックで NULL でないことが確認済み。
    //         `buf` は十分なサイズの有効な UTF-16 バッファ。
    let len = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(hwnd, &mut buf) };
    if len > 0 {
        #[allow(clippy::cast_sign_loss)]
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        String::new()
    }
}
