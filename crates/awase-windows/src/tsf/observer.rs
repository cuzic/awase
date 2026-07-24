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
//!   `TSF_OBS.gji_candidate_show`, `TSF_OBS.focus_namechange`, `TSF_OBS.ime_composition_active`
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
///   `focus_namechange`, `ime_composition_active`
///
/// 読み取りは judgement 層 (`probe.rs`) と action 層 (`output.rs`) から行う。
#[derive(Debug)]
pub struct TsfObservations {
    /// OBJ_NAMECHANGE 発火のたびに +1 されるカウンタ。現在は write-only。
    ///
    /// かつて `gji_warmup_coro.rs` の NameChangeWait フェーズ（Phase 3）がこのカウンタの
    /// 変化を読み取って GJI 応答を判定していたが、`DIAG_DISABLE_PROACTIVE_TSF_WARMUP`
    /// （常時 true）下で当該フェーズ自体が到達不能だったため撤去した（`docs/known-bugs.md`
    /// BUG-24 参照）。書き込み側（`observation_event_proc` の NAMECHANGE イベント通知、
    /// `send_eager_tsf_warmup` によるリセット）は WinEventHook 登録・他フィールドと絡む
    /// ため本コミットでは触れず残している。
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

    // 旧 composition_probe（raw TSF literal 検出の event-driven シグナル）は
    // 2026-07-06 の到達不能パス監査で撤去 — 待ち手だった AtomicWatcher 消費者
    // （raw_tsf_literal_show_or_timeout_async）が実装されないままポーリング方式
    // （LiteralDetector の baseline 読み）に置き換わり、write-only になっていた。
    /// GJI の最終 I/O 変化時刻 (GetTickCount64 ms)。0 = 未観測。
    ///
    /// バックグラウンドモニタースレッドが更新する。
    /// `send_romaji_as_tsf` や `TsfReadinessJudge` が参照する。
    pub(super) gji_last_io_ms: AtomicU64,

    /// GJI プロセスの累積 WriteTransferCount（バイト数）。
    ///
    /// バックグラウンドモニタースレッドが 10ms ごとに更新する。
    /// F2（モード切り替え）は WriteTransferCount が増加しない（w_KB=+0.0）のに対し、
    /// 文字変換は +0.2KB 以上増加する。ベースラインとの差分で
    /// 「モード切り替えのみか文字コンポジションが発生したか」を区別できる。
    /// [`LiteralDetector::new`]/[`LiteralDetector::new_with_pre_send_baseline`]
    /// の composition 確認シグナルとして使用する（BUG-30 で TSF/Chrome 共通化）。
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

    /// `LiteralDetectCore` が最後に `CompositionConfirmed`（かつ非 partial-literal）を
    /// 確認できた **`cold_seq`（`WarmEpoch::cold_start_count`）世代**。未確認なら `0`
    /// （`cold_start_count` は 0 始まりで、確認は必ず何らかの cold-start 後にしか
    /// 起こらないため `0` を「未確認」の番人値として使える）。
    ///
    /// 「確認済みかどうか」は真偽値ではなく **この値が現在の `cold_seq` と一致するか**
    /// で判定する（[`literal_session_confirmed()`] 参照）。一致する間は、同一 cold
    /// 世代内の以降の文字は literal-detect 自体をスキップし即送信する（BUG-24:
    /// `is_partial_literal()` の判定材料である `nc_fired` が `SetOpenTrue`/`FocusChange`/
    /// `NativeF2Consumed` 等の cold 直後は構造的に信頼できず、正しく変換されているのに
    /// 不要な ESC+BS 訂正が発生していた）。
    ///
    /// 世代比較そのものが「新しい cold-start が始まれば自動的に stale になる」ことを
    /// 保証するため、`reset_literal_session_confirmed()`（`gji_on_end_composition` =
    /// 候補ウィンドウ HIDE 時）による明示リセットは「次の1語も律儀に再確認させる」
    /// 保守的な最適化オプトアウトに過ぎず、正しさの唯一の拠り所ではない（BUG-39:
    /// 以前は真偽値のみで管理しており、その唯一のリセット経路が `GjiFsm` が
    /// `OnComposing` を抜けた後の HIDE では発火せず、フォーカス変更・長時間 idle・
    /// アプリ切替をまたいで「確認済み」が持ち越され、新しい cold セッションの literal
    /// 漏れが検出されなくなっていた）。
    pub(super) literal_session_confirmed_gen: AtomicU32,

    /// `EVENT_OBJECT_SHOW` で GJI candidate が表示されたことを `GjiFsm::StartComposition` に橋渡しする pending フラグ。
    ///
    /// `observation_event_proc` が set → `take_pending_start_composition()` で drain → platform が `StartComposition` を dispatch。
    pub(in crate::tsf) pending_start_composition: AtomicBool,

    /// `EVENT_OBJECT_HIDE` で GJI candidate が消えたことを `GjiFsm::EndComposition` に橋渡しする pending フラグ。
    ///
    /// `observation_event_proc` が set → `take_pending_end_composition()` で drain → platform が `EndComposition` を dispatch。
    pub(in crate::tsf) pending_end_composition: AtomicBool,

    /// `EVENT_OBJECT_IME_SHOW`/`EVENT_OBJECT_IME_HIDE` で更新する、IME composition window
    /// （IME固有の合成/候補 UI）が現在表示中かどうかのフラグ。GJI 専用の `gji_candidate_visible`
    /// と異なり、MS-IME を含む任意の IME の composition window を対象にする近似シグナル。
    ///
    /// `NicolaFsm::timeout_pending_thumb`（無変換/変換キー単独タップの生VK送出）が
    /// composition 中に MS-IME の既定機能（かな/カタカナ切替・再変換）を誤発火させるのを
    /// 防ぐために `InputContext::composing` 経由で参照する。
    ///
    /// この WinEvent が実際にどの範囲の composition 状態と相関するか（インライン合成のみの
    /// アプリで発火するか等）は実機検証が必要。
    pub(super) ime_composition_active: AtomicBool,

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
            gji_last_io_ms: AtomicU64::new(0),
            gji_write_bytes: AtomicU64::new(0),
            gji_last_write_ms: AtomicU64::new(0),
            gji_monitor_ok: AtomicBool::new(false),
            candidate_was_seen: AtomicBool::new(false),
            literal_session_confirmed_gen: AtomicU32::new(0),
            pending_start_composition: AtomicBool::new(false),
            pending_end_composition: AtomicBool::new(false),
            ime_composition_active: AtomicBool::new(false),
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

    /// 現在使用中の IME 種別を返す。
    ///
    /// `tsf_active_kind`（CLSID ベース）が取得済みならそれを優先する。
    /// 未取得（0）の場合は `MicrosoftIme` をデフォルトとする。
    /// `VK_DBE_ALPHANUMERIC/HIRAGANA` は GJI でも機能するため未検出時は MsIme 扱いが安全。
    #[must_use]
    pub(crate) fn active_ime_kind(&self) -> ActiveImeKind {
        match self.tsf_active_kind.load(Ordering::Acquire) {
            1 => ActiveImeKind::GoogleJapaneseInput,
            // 2 (MicrosoftIme 明示検出) と 0 (未検出) はどちらも安全デフォルト MicrosoftIme。
            _ => ActiveImeKind::MicrosoftIme,
        }
    }

    /// CLSID ベース IME 種別が一度でも検出済みか。
    ///
    /// `false` の間、[`Self::active_ime_kind`] は安全デフォルト（`MicrosoftIme`）を
    /// 返している。「実際に MS-IME と検出されたか」を区別したい呼び出し元
    /// （MS-IME キー割当てチェック等）はこれを併用すること。
    pub(crate) fn ime_kind_detected(&self) -> bool {
        self.tsf_active_kind.load(Ordering::Acquire) != 0
    }

    /// CLSID ベース IME 種別を更新する。値が変化した場合 `true` を返す。
    ///
    /// GJI ↔ MS-IME の動的切り替えに対応するため、値は常に上書きされる。
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

/// `TSF_OBS` への並行テストアクセスを直列化する唯一のロック。
///
/// `TSF_OBS` はプロセス全体で共有される単一の`static`であり、`cargo test`は
/// デフォルトで複数スレッド並行実行する。過去は`observer.rs`/`probe.rs`/
/// `warmup/literal_detect_fsm.rs`の各テストモジュールがそれぞれ**別々**の
/// `Mutex`(`TEST_LOCK`/`TEST_LOCK`/`VETO_TEST_LOCK`)でこのstaticを
/// 「保護しているつもり」だったが、異なる`Mutex`インスタンスは互いに排他
/// しないため実質ノーガードだった。2026-07-25、Windows実機での初回
/// `cargo test --lib -p awase-windows`実行でこのレースが顕在化し、
/// `literal_detect_fsm::poll_recovers_like_suspected_literal_when_stale_confirm_detected`
/// が`gji_last_write_ms`を他モジュールのテストに書き換えられて
/// `StaleConfirm`の代わりに`CompositionConfirmed`を観測し失敗した。
#[cfg(test)]
pub(in crate::tsf) static TSF_OBS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// 現在時刻と最終 GJI I/O 時刻の差（アイドル時間）を ms で返す。
pub(crate) fn gji_idle_ms() -> u64 {
    crate::hook::current_tick_ms().saturating_sub(gji_last_io_ms())
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
/// 文字変換は +0.2KB 以上増加する。`LiteralDetector`（`new`/`new_with_pre_send_baseline`）の
/// composition 確認シグナルとして使用する（BUG-30 で TSF/Chrome 共通化）。
pub(crate) fn gji_write_bytes() -> u64 {
    TSF_OBS.gji_write_bytes.load(Ordering::Relaxed)
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

/// 現時点で（GJI/MS-IME 問わず）IME composition window が可視かどうか。
///
/// `InputContext::composing` の供給元。`EVENT_OBJECT_IME_SHOW`/`HIDE` により更新される。
pub(crate) fn ime_composition_active_now() -> bool {
    TSF_OBS.ime_composition_active.load(Ordering::Relaxed)
}

/// `apply_ime_open` 後に `candidate_was_seen` フラグをリセットする。
pub(crate) fn reset_candidate_was_seen() {
    TSF_OBS.candidate_was_seen.store(false, Ordering::Relaxed);
}

/// `current_cold_seq` 世代において literal-detect が一度でも確認済みかどうか
/// （BUG-24 追補、BUG-39 で真偽値から世代比較に変更）。
///
/// 記録されている確認済み世代が `0`（未確認）だったり `current_cold_seq` と異なる
/// （＝その後 `FocusChange`/`NativeF2Consumed` 等で新しい cold-start が実際に走り、
/// `cold_seq` が進んでいた）場合は `false` を返す。これにより、フォーカス変更や
/// 長時間 idle をまたいで「前の cold 世代で確認済み」がそのまま信頼され続けることは
/// 構造的に起こらない。`true` の間、`LiteralDetectCore::poll` は検出処理自体を
/// スキップして即 `Done` を返す。
pub(crate) fn literal_session_confirmed(current_cold_seq: u32) -> bool {
    let confirmed_gen = TSF_OBS
        .literal_session_confirmed_gen
        .load(Ordering::Relaxed);
    confirmed_gen != 0 && confirmed_gen == current_cold_seq
}

/// literal-detect が `cold_seq` 世代で初めて `CompositionConfirmed`（非 partial-literal）を
/// 確認したときに呼ぶ。`cold_seq` が進む（＝新しい cold-start が走る）まで、または
/// `reset_literal_session_confirmed()`（候補ウィンドウ HIDE）が呼ばれるまで、以降の
/// 同世代内の文字の literal-detect をスキップさせる。
///
/// `cold_seq` は呼び出し元（`run_per_vk_confirm`/`LiteralDetectCore`）が確認した VK を
/// 送信した時点の `WarmEpoch::cold_start_count()` であること（`0` は「未確認」の番人値
/// のため渡さない）。
pub(crate) fn mark_literal_session_confirmed(cold_seq: u32) {
    debug_assert_ne!(
        cold_seq, 0,
        "cold_seq=0 は「未確認」の番人値のため mark に使ってはならない"
    );
    TSF_OBS
        .literal_session_confirmed_gen
        .store(cold_seq, Ordering::Relaxed);
}

/// 候補ウィンドウ HIDE（`gji_on_end_composition`）で呼ぶ。保守的な最適化オプトアウト
/// （次の1語も律儀に再確認させる）であり、正しさはこれに依存しない — `cold_seq` が
/// 進めば `literal_session_confirmed()` は自動的に `false` を返すため（BUG-39）。
pub(crate) fn reset_literal_session_confirmed() {
    TSF_OBS
        .literal_session_confirmed_gen
        .store(0, Ordering::Relaxed);
}

/// `pending_start_composition` フラグを取り出す（set→false swap）。
///
/// `true` が返った場合、platform は `GjiFsm::StartComposition` を dispatch する。
/// `observation_event_proc` の `EVENT_OBJECT_SHOW` が set し、
/// `advance_tsf_probe` / `send_keys` 後に drain する。
pub(crate) fn take_pending_start_composition() -> bool {
    TSF_OBS
        .pending_start_composition
        .swap(false, Ordering::Relaxed)
}

/// `pending_end_composition` フラグを取り出す（set→false swap）。
///
/// `true` が返った場合、platform は `GjiFsm::EndComposition` を dispatch する。
/// `observation_event_proc` の `EVENT_OBJECT_HIDE` が set し、
/// `advance_tsf_probe` / `send_keys` 後に drain する。
pub(crate) fn take_pending_end_composition() -> bool {
    TSF_OBS
        .pending_end_composition
        .swap(false, Ordering::Relaxed)
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
pub use super::win_event_obs::{install_observation_hooks, WinEventHookGuard};

#[cfg(test)]
#[cfg(windows)]
mod tests {
    use super::*;

    /// `TSF_OBS` はプロセス全体のグローバル状態のため、テスト間の競合を防ぐロック
    /// (`probe.rs`/`literal_detect_fsm.rs`と共有、詳細は`TSF_OBS_TEST_LOCK`のdoc参照)。
    use super::TSF_OBS_TEST_LOCK as TEST_LOCK;

    // ── BUG-39: literal_session_confirmed の世代付け回帰テスト ─────────────

    /// 確認していない状態（`cold_seq=0` 番人値）では、どの世代を問い合わせても
    /// 確認済みにならない。
    #[test]
    fn unconfirmed_state_is_never_confirmed() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_literal_session_confirmed();

        assert!(!literal_session_confirmed(1));
        assert!(!literal_session_confirmed(301));
    }

    /// `mark_literal_session_confirmed(cold_seq)` で記録した世代と同じ `cold_seq` を
    /// 問い合わせれば確認済みになる。
    #[test]
    fn same_generation_query_is_confirmed() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_literal_session_confirmed();

        mark_literal_session_confirmed(301);

        assert!(literal_session_confirmed(301));
    }

    /// BUG-39 の核心: `mark_literal_session_confirmed(301)` 後、
    /// `reset_literal_session_confirmed()`（候補ウィンドウ HIDE、`GjiFsm` の epoch 欠如で
    /// 握り潰されうる）が一切呼ばれなくても、新しい cold-start で `cold_seq` が進めば
    /// （FocusChange・NativeF2Consumed 等を経て実際に新しい probe/warmup が走った結果）
    /// 古い世代の確認は自動的に無効になる。フォーカス変更・長時間 idle・アプリ切替を
    /// またいで「前セッションで確認済み」が持ち越され、新しい cold セッションの literal
    /// 漏れが reactive literal-detect に検出されなくなる実機バグ（Windows Terminal で
    /// "こっか"→"koっか"）の回帰防止。
    #[test]
    fn new_cold_generation_invalidates_prior_confirmation_without_explicit_reset() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_literal_session_confirmed();

        mark_literal_session_confirmed(301);
        assert!(literal_session_confirmed(301));

        // reset_literal_session_confirmed() を挟まずに次の cold-start が
        // cold_seq=302 として走った場合を模擬する。
        assert!(
            !literal_session_confirmed(302),
            "古い世代(301)の確認は新しい世代(302)の問い合わせには適用されないべき"
        );
    }

    /// `reset_literal_session_confirmed()`（候補ウィンドウ HIDE）は同一世代内でも
    /// 明示的に「未確認」へ戻す（BUG-24 の「次の1語は再確認」という保守的な挙動を
    /// 引き続き提供する、世代比較はこれを代替するのではなく補完する）。
    #[test]
    fn explicit_reset_invalidates_same_generation_confirmation() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_literal_session_confirmed();

        mark_literal_session_confirmed(301);
        assert!(literal_session_confirmed(301));

        reset_literal_session_confirmed();

        assert!(!literal_session_confirmed(301));
    }
}
