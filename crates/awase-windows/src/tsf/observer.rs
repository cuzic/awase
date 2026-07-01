//! observation 層 — TSF/GJI 観測値の集約データ構造と名前付きアクセサ API。
//!
//! ## アクセス制御
//!
//! [`TSF_OBS`] は `pub(in crate::tsf)` のためこのモジュール外から直接アクセス不可（コンパイルエラー）。
//! `tsf/` 外のコードは [`tsf_obs()`] 経由でのみ読み取れる。
//!
//! 判断層（`ime_controller` 等）は [`ObservedState::from_snapshot()`] 経由のスナップショットを使うこと。
//! 直接 [`tsf_obs()`] を呼んではいけない（tick 境界外での非一貫観測の防止）。
//!
//! ## 書き込み元
//!
//! - [`gji_monitor`] バックグラウンドスレッド → `TSF_OBS.gji_last_io_ms`, `TSF_OBS.gji_monitor_ok`
//! - [`win_event_obs`] `observation_event_proc` → `TSF_OBS.gji_candidate_visible`,
//!   `TSF_OBS.gji_candidate_show`, `TSF_OBS.focus_namechange`, `TSF_OBS.composition_probe`
//!
//! [`ObservedState::from_snapshot()`]: crate::state::ime_decision_view::ObservedState::from_snapshot
//! [`gji_monitor`]: super::gji_monitor
//! [`win_event_obs`]: super::win_event_obs

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
    /// Chrome などのアプリで VK_IME_ON 受信後に GJI がひらがなモードへ移行したとき発火するかを確認する。
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
    /// 未取得（0）の場合は `MicrosoftIme` をデフォルトとする。
    /// `VK_DBE_ALPHANUMERIC/HIRAGANA` は GJI でも機能するため未検出時は MsIme 扱いが安全。
    ///
    /// なお、一度 GJI と確定した後は `set_tsf_active_kind` により GJI 固定になるため、
    /// 定期ポーリングで MS-IME に戻ることはない。
    #[must_use]
    pub(crate) fn active_ime_kind(&self) -> ActiveImeKind {
        match self.tsf_active_kind.load(Ordering::Acquire) {
            1 => ActiveImeKind::GoogleJapaneseInput,
            2 => ActiveImeKind::MicrosoftIme,
            _ => ActiveImeKind::MicrosoftIme, // 未検出時は安全なデフォルト
        }
    }

    /// CLSID ベース IME 種別を更新する。値が変化した場合 `true` を返す。
    ///
    /// # GJI 固定ポリシー
    ///
    /// 一度 `GoogleJapaneseInput` と確定したら、以後他の種別への変更を拒否する。
    /// GJI ↔ MS-IME の動的切り替えは通常行われないため、プロセス中は GJI 固定とする。
    /// デバッグ目的の強制切り替えはトレイメニュー経由で別途実装予定。
    pub(super) fn set_tsf_active_kind(&self, kind: ActiveImeKind) -> bool {
        let val: u8 = match kind {
            ActiveImeKind::GoogleJapaneseInput => 1,
            ActiveImeKind::MicrosoftIme => 2,
        };
        // GJI(1) 確定後は他種別への降格を禁止する。
        if self.tsf_active_kind.load(Ordering::Acquire) == 1 && val != 1 {
            return false;
        }
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
/// - `state::ime_decision_view` — `ObservedState::from_snapshot()` の実装元
/// - `app::key_pipeline` — フォーカスプローブ結果の構築
///
/// ## 呼び出し禁止レイヤー
///
/// 判断層（`ime_controller` 等）は `ObservedState::from_snapshot()` 経由のスナップショットを使うこと。
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

/// GJI プロセスが起動済みかつアクティブ IME として CLSID ベースで選択されているかどうか。
///
/// `gji_monitor_ok`（プロセス稼働）だけでは、GJI Converter が起動中でも
/// MS-IME がアクティブな場合に GJI と誤判定してしまう。
/// `tsf_active_kind == GoogleJapaneseInput`（CLSID 判定）を合わせることで
/// MS-IME 使用中の LiteralDetect 誤発火（BS 連射）を防ぐ。
pub(crate) fn gji_is_active_ime() -> bool {
    TSF_OBS.gji_monitor_ok.load(Ordering::Acquire)
        && TSF_OBS.tsf_active_kind.load(Ordering::Acquire) == 1
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

// ── IME 種別 ──

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

// ── 下位モジュールへの委譲 ──
//
// GJI I/O モニターと WinEvent 観察フックは専用モジュールに分離している。
// 外部からは引き続き `crate::tsf::observer::*` として参照できるよう re-export する。

pub use super::gji_monitor::start_monitor_thread;
pub use super::win_event_obs::{WinEventHookGuard, install_observation_hooks};

