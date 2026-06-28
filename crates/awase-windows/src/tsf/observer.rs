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
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};

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
    pub(super) fn has_changed(&self, b: Baseline) -> bool {
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

use crate::win32::HwndExt as _;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
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

    /// GJI プロセスの累積 ReadOperationCount。
    ///
    /// バックグラウンドモニタースレッドが 10ms ごとに更新する。
    /// ベースラインとの差分で「GJI が VK を受け取って辞書 lookup したか」を確認できる
    /// （composition 確認シグナルとして `gji_last_io_ms` より限定的）。
    /// 観測・状態推定用。0 = 未取得。
    pub(super) gji_read_op_count: AtomicU64,

    /// GJI プロセスの累積 ReadTransferCount（バイト数）。
    ///
    /// バックグラウンドモニタースレッドが 10ms ごとに更新する。
    /// スパイク（数 MB）= 辞書ファイル再ロード（コールドスタート）の可能性がある。
    /// 観測・状態推定用。0 = 未取得。
    pub(super) gji_read_bytes: AtomicU64,

    /// GJI プロセスの累積 WriteTransferCount（バイト数）。
    ///
    /// バックグラウンドモニタースレッドが 10ms ごとに更新する。
    /// F2（モード切り替え）は WriteTransferCount が増加しない（w_KB=+0.0）のに対し、
    /// 文字変換は +0.2KB 以上増加する。ベースラインとの差分で
    /// 「モード切り替えのみか文字コンポジションが発生したか」を区別できる。
    /// [`LiteralDetector::new_gji_resumed`] の Chrome 用確認シグナルとして使用する。
    /// 観測・状態推定用。0 = 未取得。
    pub(super) gji_write_bytes: AtomicU64,

    /// GJI プロセスの最終 WriteTransferCount 変化時刻 (GetTickCount64 ms)。0 = 未観測。
    ///
    /// `gji_last_io_ms`（読み書き問わず）とは独立して、WriteOperationCount が増加した
    /// タイミングのみを記録する。historydb 更新タイミングの観測に使う。
    pub(super) gji_last_write_ms: AtomicU64,

    /// GJI モニターが利用可能か（プロセス発見・ハンドル取得成功）。
    pub(super) gji_monitor_ok: AtomicBool,

    /// F21/F22 キーバインドが GJI の config1.db に登録済みか。
    ///
    /// GJI attach 時に config1.db を読み取り、`gji::patch()` が `Ok(None)` を返した場合に `true`。
    /// GJI detach 時に `false` にリセットされる。
    pub(super) gji_keybinds_ok: AtomicBool,

    /// GJI candidate が SHOW になってから次の `on_ime_applied` 呼び出しまでの間に
    /// 「shadow=OFF なのに候補ウィンドウが表示された（desync）」ことがあったかを記録するラッチ。
    ///
    /// `EVENT_OBJECT_SHOW` で `true` に、`reset_candidate_was_seen()` 呼び出し時に `false` にリセット。
    /// `KanjiToggleStrategy` が shadow=false でも desync を検出して VK_KANJI を送れるようにする。
    pub(super) candidate_was_seen: AtomicBool,

    /// `EVENT_OBJECT_SHOW` で GJI candidate が表示されたことを `GjiFsm::StartComposition` に橋渡しする pending フラグ。
    ///
    /// `observation_event_proc` が set → `take_pending_start_composition()` で drain → platform が `StartComposition` を dispatch。
    pub(in crate::tsf) pending_start_composition: AtomicBool,

    /// `EVENT_OBJECT_HIDE` で GJI candidate が消えたことを `GjiFsm::EndComposition` に橋渡しする pending フラグ。
    ///
    /// `observation_event_proc` が set → `take_pending_end_composition()` で drain → platform が `EndComposition` を dispatch。
    pub(in crate::tsf) pending_end_composition: AtomicBool,

    /// `EVENT_OBJECT_IME_SHOW`（0x8027）が発火するたびに +1 するカウンタ（実機検証用）。
    ///
    /// Chrome などのアプリで F21 受信後に GJI がひらがなモードへ移行したとき発火するかを確認する。
    /// 検証で発火が確認されれば `ChromeGjiReinitFsm` の IMC ポーリング代替シグナルとして活用できる。
    pub(in crate::tsf) ime_show_seq: ChangeCounter,

    /// `EVENT_OBJECT_IME_CHANGE`（0x8029）が発火するたびに +1 するカウンタ（実機検証用）。
    ///
    /// IME の入力モード切り替え（ひらがな↔英字など）を捕捉するために使用する。
    /// 発火クラス・タイミングの確認が目的。
    pub(in crate::tsf) ime_change_seq: ChangeCounter,

    /// `ITfInputProcessorProfileMgr::GetActiveProfile` の CLSID ベース IME 種別。
    ///
    /// `gji-io-monitor` スレッドが 2 秒ごとに更新する。
    /// 0 = 未取得（起動直後）、1 = GoogleJapaneseInput、2 = MicrosoftIme。
    ///
    /// `active_ime_kind()` はこの値を優先し、0（未取得）の場合のみ `gji_monitor_ok` から派生する。
    pub(super) tsf_active_kind: AtomicU8,
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
            focus_namechange: ChangeCounter::new(),
            gji_candidate_show: ChangeCounter::new(),
            gji_candidate_visible: AtomicBool::new(false),
            composition_probe: ChangeCounter::new(),
            gji_last_io_ms: AtomicU64::new(0),
            gji_read_op_count: AtomicU64::new(0),
            gji_read_bytes: AtomicU64::new(0),
            gji_write_bytes: AtomicU64::new(0),
            gji_last_write_ms: AtomicU64::new(0),
            gji_monitor_ok: AtomicBool::new(false),
            gji_keybinds_ok: AtomicBool::new(false),
            candidate_was_seen: AtomicBool::new(false),
            pending_start_composition: AtomicBool::new(false),
            pending_end_composition: AtomicBool::new(false),
            ime_show_seq: ChangeCounter::new(),
            ime_change_seq: ChangeCounter::new(),
            tsf_active_kind: AtomicU8::new(0),
        }
    }

    /// GJI 最終 I/O 変化時刻 (ms) を読み取る（Relaxed）。
    #[must_use]
    pub fn gji_last_io_ms(&self) -> u64 {
        self.gji_last_io_ms.load(Ordering::Relaxed)
    }

    /// GJI 最終 Write 変化時刻 (ms) を読み取る（Relaxed）。0 = 未観測。
    ///
    /// `gji_last_io_ms` とは異なり、WriteTransferCount 増加時のみ更新される。
    /// GJI がアクティブ IME かどうかの判定（`GjiDirectStrategy.is_applicable()`）に使用する。
    #[must_use]
    pub fn gji_last_write_ms(&self) -> u64 {
        self.gji_last_write_ms.load(Ordering::Relaxed)
    }

    /// GJI プロセスの累積 ReadOperationCount を読み取る（Relaxed）。
    #[must_use]
    pub fn gji_read_op_count(&self) -> u64 {
        self.gji_read_op_count.load(Ordering::Relaxed)
    }

    /// GJI プロセスの累積 ReadTransferCount（バイト数）を読み取る（Relaxed）。
    #[must_use]
    pub fn gji_read_bytes(&self) -> u64 {
        self.gji_read_bytes.load(Ordering::Relaxed)
    }

    /// GJI プロセスの累積 WriteTransferCount（バイト数）を読み取る（Relaxed）。
    #[must_use]
    pub fn gji_write_bytes(&self) -> u64 {
        self.gji_write_bytes.load(Ordering::Relaxed)
    }

    /// GJI モニターが利用可能かを読み取る（Acquire）。
    #[must_use]
    pub fn gji_monitor_ok(&self) -> bool {
        self.gji_monitor_ok.load(Ordering::Acquire)
    }

    /// F21/F22 キーバインドが config1.db に登録済みかを読み取る（Acquire）。
    #[must_use]
    pub fn gji_keybinds_ok(&self) -> bool {
        self.gji_keybinds_ok.load(Ordering::Acquire)
    }

    /// GJI candidate window が現在表示中かを読み取る（Relaxed）。
    #[must_use]
    pub fn gji_candidate_visible(&self) -> bool {
        self.gji_candidate_visible.load(Ordering::Relaxed)
    }

    /// raw TSF literal 検出用汎用シグナルへの参照を返す（`AtomicWatcher` 用）。
    ///
    /// `AtomicWatcher::new` に必要な `&AtomicU32` はこのメソッド経由で取得する。
    pub const fn composition_probe_atomic(&self) -> &AtomicU32 {
        self.composition_probe.atomic()
    }

    /// 現在使用中の IME 種別を返す。
    ///
    /// `tsf_active_kind`（CLSID ベース）が取得済みならそれを優先する。
    /// 未取得（0）の場合は `gji_monitor_ok` から派生する（起動直後のフォールバック）。
    #[must_use]
    pub(crate) fn active_ime_kind(&self) -> ActiveImeKind {
        match self.tsf_active_kind.load(Ordering::Acquire) {
            1 => ActiveImeKind::GoogleJapaneseInput,
            2 => ActiveImeKind::MicrosoftIme,
            _ => {
                // CLSID 未取得: gji_monitor_ok から派生（後方互換フォールバック）
                if self.gji_monitor_ok() {
                    ActiveImeKind::GoogleJapaneseInput
                } else {
                    ActiveImeKind::MicrosoftIme
                }
            }
        }
    }

    /// CLSID ベース IME 種別を更新する。値が変化した場合 `true` を返す。
    pub(super) fn set_tsf_active_kind(&self, kind: ActiveImeKind) -> bool {
        let val: u8 = match kind {
            ActiveImeKind::GoogleJapaneseInput => 1,
            ActiveImeKind::MicrosoftIme => 2,
        };
        self.tsf_active_kind.swap(val, Ordering::Release) != val
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

/// GJI プロセスの最終 WriteOperationCount 変化時刻 (ms) を返す。0 = 未観測。live 読み取り。
///
/// 読み書き問わず更新される `gji_last_io_ms` と異なり、書き込みのみを追跡する。
/// historydb 更新タイミングの観測ログで使う。
pub(crate) fn gji_last_write_ms() -> u64 {
    TSF_OBS.gji_last_write_ms.load(Ordering::Relaxed)
}

/// GJI プロセスの累積 WriteTransferCount（バイト数）を返す。0 = 未観測。live 読み取り。
///
/// F2 などのモード切り替えキーは WriteTransferCount が増加しない（w_KB=+0.0）のに対し、
/// 文字変換は +0.2KB 以上増加する。`LiteralDetector::new_gji_resumed` の
/// Chrome 用 composition 確認シグナルとして使用する。
pub(crate) fn gji_write_bytes() -> u64 {
    TSF_OBS.gji_write_bytes.load(Ordering::Relaxed)
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

/// `pending_start_composition` フラグを取り出す（set→false swap）。
///
/// `true` が返った場合、platform は `GjiFsm::StartComposition` を dispatch する。
/// `observation_event_proc` の `EVENT_OBJECT_SHOW` が set し、
/// `advance_tsf_probe` / `send_keys` 後に drain する。
pub(crate) fn take_pending_start_composition() -> bool {
    TSF_OBS.pending_start_composition.swap(false, Ordering::Relaxed)
}

/// `pending_end_composition` フラグを取り出す（set→false swap）。
///
/// `true` が返った場合、platform は `GjiFsm::EndComposition` を dispatch する。
/// `observation_event_proc` の `EVENT_OBJECT_HIDE` が set し、
/// `advance_tsf_probe` / `send_keys` 後に drain する。
pub(crate) fn take_pending_end_composition() -> bool {
    TSF_OBS.pending_end_composition.swap(false, Ordering::Relaxed)
}

/// GJI setup 完了後に呼んで `gji_keybinds_ok` を即座に `true` にセットする。
///
/// config1.db パッチが成功した時点で GJI monitor の re-attach を待たずに
/// `GjiDirectStrategy` を有効化するために使う。
pub(crate) fn notify_gji_keybinds_registered() {
    TSF_OBS.gji_keybinds_ok.store(true, Ordering::Release);
}

/// GJI teardown 完了後に呼んで `gji_keybinds_ok` を即座に `false` にセットする。
///
/// config1.db から F21/F22 エントリを削除した時点で GJI monitor の re-attach を待たずに
/// `GjiDirectStrategy` を無効化し `KanjiToggle` にフォールバックさせるために使う。
pub(crate) fn notify_gji_keybinds_removed() {
    TSF_OBS.gji_keybinds_ok.store(false, Ordering::Release);
}

/// OBJ_NAMECHANGE カウンタのベースライン値。
///
/// `namechange_baseline()` で取得し、`fired()` で変化を検出する。
pub(crate) struct NamechangeBaseline(Baseline);

impl NamechangeBaseline {
    /// ベースライン取得後にカウンタが変化したかどうかを返す。
    pub(crate) fn fired(&self) -> bool {
        TSF_OBS.focus_namechange.has_changed(self.0)
    }
}

// ── GJI プロセス発見 ──

/// フォアグラウンドで使用中の IME の種別。
///
/// `gji_monitor_ok` の状態から派生する（新たなアトミック不要）。
/// GJI が検出されていなければ MS-IME（または互換 IME）とみなす。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ActiveImeKind {
    /// Google 日本語入力が起動・検出済み。
    GoogleJapaneseInput,
    /// GJI 非検出 — MS-IME（または互換 IME）と推定。
    MicrosoftIme,
}

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

fn check_keybinds_in_db() -> bool {
    crate::gji::default_config_path()
        .and_then(|p| std::fs::read(&p).ok())
        .is_some_and(|data| matches!(crate::gji::patch(&data), Ok(None)))
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
    let mut next_config_recheck_ms: u64 = 0;
    // GJI 検出状態の直前通知値。変化したときのみ WM_IME_KIND_CHANGED を post する。
    // None = まだ通知未送信（起動直後）。
    let mut last_notified_ok: Option<bool> = None;
    // CLSID ベース IME 種別ポーリング次回時刻
    let mut next_clsid_check_ms: u64 = 0;

    loop {
        let now = crate::hook::current_tick_ms();

        // CLSID ベース IME 種別を 2 秒ごとにポーリングして更新する
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
                log::info!("[gji-monitor] attached to GJI process");
                let keybinds_ok = check_keybinds_in_db();
                if keybinds_ok {
                    log::info!("[gji-monitor] F21/F22 keybinds registered in config1.db");
                } else {
                    log::warn!(
                        "[gji-monitor] F21/F22 keybinds not registered in config1.db \
                         — GjiDirect strategy unavailable, falling back to KanjiToggle"
                    );
                }
                TSF_OBS.gji_keybinds_ok.store(keybinds_ok, Ordering::Release);
                TSF_OBS
                    .gji_last_io_ms
                    .store(m.last_change_ms(), Ordering::Relaxed);
                TSF_OBS.gji_monitor_ok.store(true, Ordering::Release);
                next_config_recheck_ms = now + crate::tuning::GJI_CONFIG_RECHECK_INTERVAL_MS;
                monitor = Some(m);
                // GJI 検出: GoogleJapaneseInput に変化
                if last_notified_ok != Some(true) {
                    last_notified_ok = Some(true);
                    log::info!("[gji-monitor] IME kind → GoogleJapaneseInput");
                    crate::win32::post_to_main_thread(crate::WM_IME_KIND_CHANGED);
                }
            } else {
                TSF_OBS.gji_monitor_ok.store(false, Ordering::Relaxed);
                next_attach_ms = now + crate::tuning::GJI_REATTACH_INTERVAL_MS;
                // GJI 非検出: MicrosoftIme に変化（起動初回 or GJI 消失後）
                if last_notified_ok != Some(false) {
                    last_notified_ok = Some(false);
                    log::info!("[gji-monitor] IME kind → MicrosoftIme (GJI not found)");
                    crate::win32::post_to_main_thread(crate::WM_IME_KIND_CHANGED);
                }
            }
        }

        if let Some(ref mut m) = monitor {
            match m.sample() {
                None => {
                    log::info!("[gji-monitor] GJI process exited, will re-attach");
                    TSF_OBS.gji_monitor_ok.store(false, Ordering::Relaxed);
                    TSF_OBS.gji_keybinds_ok.store(false, Ordering::Relaxed);
                    monitor = None;
                    next_attach_ms = now + crate::tuning::GJI_REATTACH_INTERVAL_MS;
                    // GJI 消失: MicrosoftIme に変化
                    if last_notified_ok != Some(false) {
                        last_notified_ok = Some(false);
                        log::info!("[gji-monitor] IME kind → MicrosoftIme (GJI exited)");
                        crate::win32::post_to_main_thread(crate::WM_IME_KIND_CHANGED);
                    }
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

        if monitor.is_some() && now >= next_config_recheck_ms {
            let keybinds_ok = check_keybinds_in_db();
            let prev = TSF_OBS.gji_keybinds_ok.load(Ordering::Acquire);
            if keybinds_ok != prev {
                TSF_OBS.gji_keybinds_ok.store(keybinds_ok, Ordering::Release);
                if keybinds_ok {
                    log::info!(
                        "[gji-monitor] config1.db 変化検出: F21/F22 keybinds が復元されました"
                    );
                } else {
                    log::warn!(
                        "[gji-monitor] config1.db 変化検出: F21/F22 keybinds が消去されました \
                         — GjiDirect strategy 無効化"
                    );
                }
            }
            next_config_recheck_ms = now + crate::tuning::GJI_CONFIG_RECHECK_INTERVAL_MS;
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

// ── WinEvent 観察フック ──

use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_HIDE, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SHOW, WINEVENT_OUTOFCONTEXT,
};

const GJI_CANDIDATE_CLASS: &str = "GoogleJapaneseInputCandidateWindow";
const MSCTFIME_UI_CLASS: &str = "MSCTFIME UI";

// EVENT_OBJECT_IME_SHOW/HIDE/CHANGE (0x8027–0x8029) は windows crate には定義がないため
// 生の値で定義する。GJI TSF モードでは発火しないが Chrome ホスト側から発火するか検証用。
const EVENT_OBJECT_IME_SHOW: u32 = 0x8027;
const EVENT_OBJECT_IME_HIDE: u32 = 0x8028;
const EVENT_OBJECT_IME_CHANGE: u32 = 0x8029;

/// `SetWinEventHook` の RAII ガード。Drop 時に `UnhookWinEvent` を呼ぶ。
pub struct WinEventHookGuard(pub HWINEVENTHOOK);

impl std::fmt::Debug for WinEventHookGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("WinEventHookGuard")
            .field(&self.0 .0)
            .finish()
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
            0,
            0,
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
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if show_hook.is_invalid() {
        log::warn!("[obs-hook] failed to install OBJECT_SHOW/HIDE hook");
    } else {
        log::info!(
            "[obs-hook] OBJECT_SHOW/HIDE hook installed (GJI candidate window visibility tracking)"
        );
        hooks.push(WinEventHookGuard(show_hook));
    }

    // SAFETY: `observation_event_proc` は `'static` な extern "system" fn ポインタ。
    //         `WINEVENT_OUTOFCONTEXT` によりコールバックはメッセージループスレッドで実行される。
    //         返されたフックは `WinEventHookGuard::drop` で `UnhookWinEvent` される。
    let ime_hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_IME_SHOW,
            EVENT_OBJECT_IME_CHANGE, // SHOW(0x8027)〜CHANGE(0x8029) の全 IME イベントを捕捉
            None,
            Some(observation_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if ime_hook.is_invalid() {
        log::warn!("[obs-hook] failed to install EVENT_OBJECT_IME_* hook");
    } else {
        log::info!(
            "[obs-hook] EVENT_OBJECT_IME_SHOW/HIDE/CHANGE hook installed (Chrome TSF composition context probe)"
        );
        hooks.push(WinEventHookGuard(ime_hook));
    }

    hooks
}

/// WinEvent 観察コールバック。NAMECHANGE / IME_SHOW / IME_HIDE / IME_CHANGE を処理する。
#[expect(clippy::cognitive_complexity)]
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
                TSF_OBS.pending_start_composition.store(true, Ordering::Relaxed);
                let seq = TSF_OBS.gji_candidate_show.notify();
                // raw TSF literal 検出用の汎用シグナルも +1（SHOW と timeout の両方が
                // AtomicWatcher で event-driven に待機する設計）
                TSF_OBS.composition_probe.notify();
                {
                    let now_ms = crate::hook::current_tick_ms();
                    let last_write_ms = TSF_OBS.gji_last_write_ms.load(Ordering::Relaxed);
                    let write_ago = if last_write_ms == 0 {
                        "never".to_string()
                    } else {
                        format!("{}ms ago", now_ms.saturating_sub(last_write_ms))
                    };
                    log::info!("[gji-obs] candidate SHOW #{seq}: last_gji_write={write_ago}");
                }
                win32_async::notify_all();
            } else if class == MSCTFIME_UI_CLASS {
                log::debug!("[tsf-ime-ui] SHOW hwnd={:?}", hwnd.0);
            }
        }
        EVENT_OBJECT_HIDE => {
            let class = hwnd_class_name(hwnd);
            if class == GJI_CANDIDATE_CLASS {
                TSF_OBS.gji_candidate_visible.store(false, Ordering::Relaxed);
                TSF_OBS.pending_end_composition.store(true, Ordering::Relaxed);
                {
                    let now_ms = crate::hook::current_tick_ms();
                    let last_write_ms = TSF_OBS.gji_last_write_ms.load(Ordering::Relaxed);
                    let write_ago = if last_write_ms == 0 {
                        "never".to_string()
                    } else {
                        format!("{}ms ago", now_ms.saturating_sub(last_write_ms))
                    };
                    log::info!("[gji-obs] candidate HIDE: last_gji_write={write_ago}");
                }
            } else if class == MSCTFIME_UI_CLASS {
                log::debug!("[tsf-ime-ui] HIDE hwnd={:?}", hwnd.0);
            }
        }
        EVENT_OBJECT_IME_SHOW => {
            let class = hwnd_class_name(hwnd);
            let seq = TSF_OBS.ime_show_seq.notify();
            log::info!("[ime-obj] IME_SHOW #{seq} class={class} hwnd={:?}", hwnd.0);
            win32_async::notify_all();
        }
        EVENT_OBJECT_IME_HIDE => {
            let class = hwnd_class_name(hwnd);
            log::info!("[ime-obj] IME_HIDE class={class} hwnd={:?}", hwnd.0);
        }
        EVENT_OBJECT_IME_CHANGE => {
            let class = hwnd_class_name(hwnd);
            let seq = TSF_OBS.ime_change_seq.notify();
            log::info!("[ime-obj] IME_CHANGE #{seq} class={class} hwnd={:?}", hwnd.0);
            win32_async::notify_all();
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
        #[expect(clippy::cast_sign_loss)]
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        String::new()
    }
}
