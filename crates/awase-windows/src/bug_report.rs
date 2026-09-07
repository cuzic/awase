//! ADR-095 bug report payload types.
//!
//! This module intentionally defines a dedicated allowlist payload instead of
//! serializing `journal::JournalEntry` directly. The tray process provides an
//! already dumped journal JSON string; this module only decides what parts go
//! into the report body and how large they may be.

use serde::{Deserialize, Serialize};

pub const ENDPOINT_URL: &str = "https://report.awase.cc/v1/reports";
pub const REPORT_HOST: &str = "report.awase.cc";
// ADR-095 leaves the exact R2 lifecycle rule undecided. The client displays
// 90 days as a practical review window with a clear deletion expectation.
pub const RETENTION_HINT: &str = "約90日間保管後に自動削除";
pub const DESCRIPTION_MAX_CHARS: usize = 4_000;
/// journal/app_log それぞれの添付上限。
///
/// 旧値の 256KiB は journal + app_log の2本をフル添付しただけで
/// 256*2=512KiB = `MAX_BODY_BYTES` に達し、他のフィールド（内部状態
/// スナップショット・設定ファイル・配列ファイル・description等）の
/// ぶんだけ確実に超過する構造だった。実機で「送信のたびに必ず自動切り詰めが
/// 発生する」と報告され、200KiB×2=400KiBを引いた単純計算では他フィールド
/// 用に~112KiBのマージンとなるよう引き下げた（`serde_json::to_string_pretty`
/// のインデント・エスケープ等のオーバーヘッドを含めた実測では、
/// `full_size_journal_and_app_log_fit_within_max_body_bytes_without_shrinking`
/// テストのケースで ~102KiB）。
pub const LOG_EXCERPT_MAX_BYTES: usize = 200 * 1024;
pub const SCHEMA_VERSION: u8 = 3;
/// `services/report-worker/src/index.ts` の `MAX_BODY_BYTES` と同じ値。
/// サーバ側の 413 応答を待たず、送信前にクライアント側で分かりやすく警告する
/// ための閾値としてのみ使う（サーバ側の実際の上限はサーバ側定数がSSOT）。
pub const MAX_BODY_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BugReportImeKind {
    Gji,
    MsIme,
    Unknown,
}

impl BugReportImeKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gji => "Gji",
            Self::MsIme => "MsIme",
            Self::Unknown => "Unknown",
        }
    }
}

impl std::str::FromStr for BugReportImeKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Gji" => Ok(Self::Gji),
            "MsIme" => Ok(Self::MsIme),
            "Unknown" => Ok(Self::Unknown),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymptomCategory {
    WrongCharacterOutput,
    CharacterDropped,
    StuckInRomaji,
    UnexpectedWidthOrKana,
    ImeToggledUnexpectedly,
    ThumbKeyMisbehavior,
    BrokenAfterAppSwitch,
    BrokenAfterIdle,
    NoResponse,
    Other,
}

impl SymptomCategory {
    pub const ALL: [Self; 10] = [
        Self::WrongCharacterOutput,
        Self::CharacterDropped,
        Self::StuckInRomaji,
        Self::UnexpectedWidthOrKana,
        Self::ImeToggledUnexpectedly,
        Self::ThumbKeyMisbehavior,
        Self::BrokenAfterAppSwitch,
        Self::BrokenAfterIdle,
        Self::NoResponse,
        Self::Other,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::WrongCharacterOutput => "入力した文字と違う文字が出た（変換ミス）",
            Self::CharacterDropped => "一部の文字が消えた／出力されなかった",
            Self::StuckInRomaji => "ローマ字のまま出る／ひらがなに戻らない",
            Self::UnexpectedWidthOrKana => "全角・半角やカタカナが意図せず切り替わった",
            Self::ImeToggledUnexpectedly => "日本語入力（IME）が勝手にON/OFFになった",
            Self::ThumbKeyMisbehavior => "親指キー（無変換・変換など）が効かない、誤動作する",
            Self::BrokenAfterAppSwitch => "別のアプリに切り替えた直後におかしくなった",
            Self::BrokenAfterIdle => "しばらく操作しなかった後、最初の入力がおかしい",
            Self::NoResponse => "キーを押しても反応しない",
            Self::Other => "その他",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BugReportStateSnapshot {
    pub desired_open: bool,
    pub effective_open: bool,
    pub input_mode: String,
    pub applied: String,
    pub app_kind: String,
    pub focus_kind: String,
    pub gji_state: String,
    /// BUG-34 横展開の切り分け用（docs/known-bugs.md BUG-34 参照）:
    /// 直近の `SendMessageTimeoutW` 呼び出しの実測ms。
    pub send_health_last_elapsed_ms: u64,
    /// `send_health` の連続 slow 判定回数（ブレーカ作動の予兆、閾値未満でも記録）。
    pub send_health_consecutive_slow: u32,
    /// 報告時点で SendHealth サーキットブレーカが作動中（同期サイトの発行を
    /// 見送っている）かどうか。
    pub send_health_breaker_tripped: bool,
    /// `kp_stage_idle_conv_check` の offload 読み取りが in-flight のままの経過ms。
    /// `None` なら in-flight なし。長時間 `Some` が続く場合は完了取りこぼし
    /// （旧: 永久ラッチのバグ、レビューで修正済みだが再発検知用に残す）を疑う。
    pub idle_conv_check_in_flight_ms: Option<u64>,
    /// ADR-140 Step1 決定I: idle-conv-check probe が GJI actuation との交錯を
    /// 検知して abandon した累計回数（resync 経路、`FocusResyncGate` 経由）。
    /// resync 経路の abandon は defer 中のキーが `FOCUS_RESYNC_DEADLINE_MS`
    /// まで出てこない体感遅延に直結するため、通常経路とは別に数える
    /// （`crate::probe_actuation_fence` module doc 参照）。増え続ける場合は
    /// probe の starvation（本来の目的であるタスクバーからのモード変更検知が
    /// 機能不全に陥っている）を疑う。
    pub idle_conv_check_abandoned_resync_count: u32,
    /// 同上、通常経路（`kp_stage_idle_conv_check`）の累計回数。
    pub idle_conv_check_abandoned_normal_count: u32,
    /// ADR-140 Step1 実装レビュー指摘M1: 上記abandonカウンタの分母
    /// （resync経路でprobeを実際にspawnした累計回数）。分母が無いと
    /// abandonカウンタ単体では「頻発しているか」を判定できないため追加した。
    pub idle_conv_check_spawned_resync_count: u32,
    /// 同上、通常経路の累計回数。
    pub idle_conv_check_spawned_normal_count: u32,
    /// 「長時間使うと重くなる」報告の切り分け用に追加したプロセスリソース
    /// スナップショット。単発の報告だけでは判断できないが、複数の報告を
    /// `process_uptime_secs` でソートして並べれば、稼働時間とともに
    /// `working_set_bytes`/`handle_count`/`gdi_object_count`/`user_object_count`
    /// のどれが増加傾向にあるか（メモリリークかハンドル/GDIオブジェクトの
    /// リークか、あるいはどれも増えていないか）を後から確認できる。
    /// プロセス起動からの経過秒数（`GetProcessTimes` の creation time 基準）。
    pub process_uptime_secs: u64,
    /// ワーキングセットサイズ（`GetProcessMemoryInfo` の `WorkingSetSize`、バイト）。
    pub working_set_bytes: u64,
    /// プロセスが保持しているカーネルオブジェクトハンドル数
    /// （`GetProcessHandleCount`）。
    pub handle_count: u32,
    /// プロセスが保持している GDI オブジェクト数（`GetGuiResources(GR_GDIOBJECTS)`）。
    pub gdi_object_count: u32,
    /// プロセスが保持している USER オブジェクト数（`GetGuiResources(GR_USEROBJECTS)`）。
    pub user_object_count: u32,
    /// `HKCU\Control Panel\Desktop\LowLevelHooksTimeout` の実値（ms）。未設定なら
    /// `None`（Windows既定の5000msとみなしてよい）。issue #165「キーフックのフック
    /// 落ち」仮説の切り分け用（`docs/bug-reports-triage.md` 01M1NET4A7D8Z9EYN3JM4WVETP
    /// 行）。値そのものは診断専用で、awaseの挙動判定には使わない。
    pub low_level_hooks_timeout_ms: Option<u32>,
    /// `request_engine_wake` の `PostMessageW` が失敗した累計回数（プロセス生存期間
    /// 中）。既存の `[hook-ring] request_engine_wake の PostMessageW が失敗した形跡が
    /// あります` ログ（`hook_channel.rs::recover_stuck_wake_if_needed`）と同じ検出を
    /// 不具合報告に持たせたもの。0 でなければエンジンスレッド側のメッセージキューが
    /// 詰まった形跡がある（issue #165 H1: エンジンスレッド詰まり仮説）。
    pub wake_post_failed_lifetime_count: u32,
    /// `HOOK_KEYS`（フックスレッド→エンジンスレッド転送用リングバッファ）の
    /// プロセス生存期間中の最大占有数。容量（1024）に近いほどエンジンスレッド側の
    /// 処理が詰まっていた形跡が強い（issue #165 H1）。
    pub hook_ring_max_occupancy: u32,
}

/// GJI（`config1.db`）から抽出した、無変換/変換キーのIME意味論・
/// キーマップ設定の要約（ADR-148）。
///
/// フィールドは大きく2種類に分かれる:
/// - **生値・分類系**（`session_keymap`/`custom_keymap_table_present`/
///   `custom_keymap_table_is_effective`/`ime_*_keys`/`mode_*_keys`/
///   `henkan_classified_kind`/`muhenkan_classified_kind`）:
///   `config1.db`の内容を解釈するだけの計算で、現在のアクティブIME
///   （`ime_kind`）に関わらず常に計算する。
/// - **採用系**（`henkan_adopted_kind`/`muhenkan_adopted_kind`/
///   `henkan_adopted_route`/`muhenkan_adopted_route`/
///   `thumb_key_ime_warning`）: GJIが実際にアクティブ（`ime_kind ==
///   Gji`）なときのみ計算する。GJIから離脱すると
///   `sync_gji_charset_autodetect`がこれらの値を全部解除するため、
///   非アクティブ時に計算すると「既に解除済みの設定」を「現在の設定」
///   であるかのように報告してしまう（Opus敵対的レビューG1で検出）。
///
/// # この型の安全性が依存している前提（レビューF7・S-2）
///
/// `ime_*_keys`/`mode_*_keys`に含まれるVK名は
/// `awase_gji_config::keymap::mozc_key_to_vk_name`のallowlist
/// （固定エイリアス表と`F1`-`F24`のみ）を通過したものだけであり、
/// `config1.db`由来の任意文字列が混入する経路はない。**将来「未対応の
/// キートークンも診断のため載せよう」という変更を加えると、この
/// allowlistという唯一の防壁を素通りして`config1.db`由来の任意文字列を
/// 送信するチャネルに変質する**ため、そのような変更は行わないこと。
///
/// # `ime_*_keys`はawaseが実際に採用したキー集合ではない（レビューS-3）
///
/// `ime_on_keys`/`ime_off_keys`/`ime_toggle_keys`は
/// `awase_gji_config::keymap::extract_ime_keys`の抽出結果をそのまま
/// 反映したものであり、awase本体が実際にIME ON/OFF自動検出へ採用する
/// 際にさらに適用する安全範囲フィルタ（`gji_charset_autodetect.rs::
/// is_in_safe_autodetect_range`、F15-F24限定、`VK_KANJI`等はBUG-14
/// 対策で除外）は通していない。したがって、ここに`"VK_KANJI"`等
/// フィルタで除外されるはずのVK名が現れても、それは「awaseがその
/// キーを誤って自動検出に採用した」ことを意味しない——`config1.db`側の
/// 生の宣言をそのまま見せているだけである。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BugReportGjiKeymapSummary {
    /// `"NotFound"` / `"ParseFailed"` / `"Ok"`。
    pub config1_db_status: String,
    /// `SESSION_KEYMAP_CUSTOM`等の生値。
    pub session_keymap: Option<i64>,
    /// `overlay_keymaps`に`SESSION_KEYMAP_OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF`
    /// を含むか。
    pub has_henkan_muhenkan_overlay: bool,
    /// `custom_keymap_table`（field 42）そのものが存在するか
    /// （`session_keymap`の値は問わない）。
    pub custom_keymap_table_present: bool,
    /// `session_keymap == CUSTOM`のときのみ`true`。GJI本体が
    /// `custom_keymap_table`を実際に参照するかどうかのガード
    /// （`gji_charset_autodetect.rs`のガードを再現）。
    pub custom_keymap_table_is_effective: bool,
    /// `custom_keymap_table_is_effective`が`true`のときのみ`Some`。
    pub ime_on_keys: Option<Vec<String>>,
    pub ime_off_keys: Option<Vec<String>>,
    pub ime_toggle_keys: Option<Vec<String>>,
    /// VK名と`GjiCompositionMode`の文字列表現のペア。
    pub mode_set_keys: Option<Vec<(String, String)>>,
    pub mode_toggle_alphanumeric_keys: Option<Vec<String>>,
    pub mode_toggle_kana_type_keys: Option<Vec<String>>,
    /// `classify_thumb_key_ime_actions`（gate前）の結果。`"On"`/`"Off"`/
    /// `"Toggle"`。
    pub henkan_classified_kind: Option<String>,
    pub muhenkan_classified_kind: Option<String>,
    /// `gate_thumb_key_ime_actions`（gate後）の結果。`ime_kind == Gji`
    /// のときのみ`Some`。
    pub henkan_adopted_kind: Option<String>,
    pub muhenkan_adopted_kind: Option<String>,
    /// `"Delegate"` / `"ActuationAuto"`。`ime_kind == Gji`のときのみ`Some`。
    pub henkan_adopted_route: Option<String>,
    pub muhenkan_adopted_route: Option<String>,
    /// `"ToggleDeclined"` / `"ToggleHonored"`。`ime_kind == Gji`のときのみ
    /// `Some`（警告不要なら`None`）。
    pub thumb_key_ime_warning: Option<String>,
    /// `muhenkan_solo_tap_dedicated_fn_key`が設定済みか。`true`の場合、
    /// `muhenkan_adopted_route == Some("Delegate")`であっても実際には
    /// 発火しない（優先順位で専用Fnキーが勝つ）。GJI/MS-IME共通の
    /// 意味を持つため両summary型に同じフィールドを持たせる。
    pub muhenkan_dedicated_fn_key_configured: bool,
}

/// MS-IME「キーとタッチのカスタマイズ」（シンプルキー割当て）のレジストリ
/// 値の要約（ADR-148）。
///
/// フィールドの生値/採用系の区別は[`BugReportGjiKeymapSummary`]と同じ
/// 考え方: 生のDWORD5個は`ime_kind`に関わらず常に読む。`adopted_*`は
/// `ime_kind == MsIme`のときのみ`Some`（MS-IMEが非アクティブなら、その
/// レジストリ値をawaseは採用していない）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BugReportMsImeKeyAssignmentSummary {
    pub is_key_assignment_enabled: Option<u32>,
    pub key_assignment_muhenkan: Option<u32>,
    pub key_assignment_henkan: Option<u32>,
    pub key_assignment_ctrl_space: Option<u32>,
    pub key_assignment_shift_space: Option<u32>,
    /// `"Ctrl+Space"`/`"Shift+Space"`のような表現。`ime_kind == MsIme`の
    /// ときのみ`Some`。
    pub adopted_ime_toggle_combos: Option<Vec<String>>,
    /// `ShadowImeAction`の文字列表現。`ime_kind == MsIme`かつ対象キーが
    /// 親指キーとして設定されているときのみ`Some`。
    pub adopted_muhenkan_delegate: Option<String>,
    pub adopted_henkan_delegate: Option<String>,
    /// [`BugReportGjiKeymapSummary::muhenkan_dedicated_fn_key_configured`]
    /// と同じ意味。
    pub muhenkan_dedicated_fn_key_configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BugReportPayload {
    pub schema_version: u8,
    pub app_version: String,
    pub os_version: String,
    pub ime_kind: String,
    pub ime_product_name: Option<String>,
    pub keyboard_model: String,
    pub windows_keyboard_layout: String,
    pub competing_software: Vec<String>,
    pub symptom_category: SymptomCategory,
    pub description: String,
    pub attach_state_snapshot: bool,
    pub state_snapshot: Option<BugReportStateSnapshot>,
    pub attach_config: bool,
    pub config_toml: Option<String>,
    pub attach_layout: bool,
    pub layout_yab: Option<String>,
    pub attach_log: bool,
    pub log_excerpt: Option<String>,
    /// 実際の `log::` 出力（`awase.log`）の末尾。`log_excerpt`（構造化 journal）
    /// には無い send_health/degrade 系の警告等を拾うための別系統の添付
    /// （BUG-34 横展開）。`attach_log` チェックボックスで両方まとめて制御する。
    pub app_log_excerpt: Option<String>,
    /// ADR-120 決定0a-report: 3キー仲裁の判定過程・訂正発生の観測カウンタ。
    /// 打鍵内容・かな1文字も含まない、起動からの累積カウンタのみ。
    pub attach_retro_eval_stats: bool,
    pub retro_eval_stats: Option<BugReportRetroEvalStats>,
    /// ADR-148: GJI/MS-IMEのキーマップ・キー割当て設定。
    pub attach_ime_keymap: bool,
    pub gji_keymap: Option<BugReportGjiKeymapSummary>,
    pub msime_key_assignment: Option<BugReportMsImeKeyAssignmentSummary>,
    pub reported_at: String,
}

/// ADR-120 決定0a-report: 3キー仲裁の判定過程・訂正発生を観測する累積カウンタ
/// （`awase::engine::RetroEvalStats` 相当）を bug report ペイロードへ写す型。
/// 打鍵内容・かな1文字も含まない、起動からの累積カウンタのみで構成する。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BugReportRetroEvalStats {
    pub three_key_total: u64,
    pub phase2_reached: u64,
    pub phase1_reached: u64,
    pub no_ngram_count: u64,
    pub score_a_neg_infinity_count: u64,
    pub score_a_zero_count: u64,
    pub score_a_finite_count: u64,
    pub score_b_neg_infinity_count: u64,
    pub score_b_zero_count: u64,
    pub score_b_finite_count: u64,
    pub char2_normal_hiragana_count: u64,
    pub no_thumb_followup_count: u64,
    pub thumb_watch_window_thumb_arrived_count: u64,
    pub thumb_watch_window_abandoned_count: u64,
    pub followup_elapsed_ms_histogram: [u64; 7],
    pub followup_overwritten_count: u64,
    pub followup_dropped_imprecise_count: u64,
    pub phase2_decisions_total: u64,
    pub phase2_correction_histogram: [u64; 7],
    pub phase1_decisions_total: u64,
    pub phase1_correction_histogram: [u64; 7],
    pub baseline_decisions_total: u64,
    pub baseline_correction_histogram: [u64; 7],
    pub escape_output_count: u64,
}

impl From<&awase::engine::RetroEvalStats> for BugReportRetroEvalStats {
    fn from(stats: &awase::engine::RetroEvalStats) -> Self {
        // `RetroEvalStats` を `..` を使わずフィールド名で丸ごと分解する
        // （`/code-review` 指摘対応）: `..` を使った部分アクセスや
        // `stats.field` の個別参照だと、将来 `RetroEvalStats` に新しい
        // フィールドを追加してもこの変換をコンパイラが強制してくれない
        // （送信元に未使用フィールドがあってもエラーにならない）ため、
        // 新カウンタが bug report に反映されないまま気づかれずに
        // 出荷されるリスクがある。分解パターンを網羅的にすることで、
        // フィールド追加時に必ずここがコンパイルエラーになるようにする。
        let awase::engine::RetroEvalStats {
            three_key_total,
            phase2_reached,
            phase1_reached,
            no_ngram_count,
            score_a_neg_infinity_count,
            score_a_zero_count,
            score_a_finite_count,
            score_b_neg_infinity_count,
            score_b_zero_count,
            score_b_finite_count,
            char2_normal_hiragana_count,
            no_thumb_followup_count,
            thumb_watch_window_thumb_arrived_count,
            thumb_watch_window_abandoned_count,
            followup_elapsed_ms_histogram,
            followup_overwritten_count,
            followup_dropped_imprecise_count,
            phase2_decisions_total,
            phase2_correction_histogram,
            phase1_decisions_total,
            phase1_correction_histogram,
            baseline_decisions_total,
            baseline_correction_histogram,
            escape_output_count,
        } = *stats;
        Self {
            three_key_total,
            phase2_reached,
            phase1_reached,
            no_ngram_count,
            score_a_neg_infinity_count,
            score_a_zero_count,
            score_a_finite_count,
            score_b_neg_infinity_count,
            score_b_zero_count,
            score_b_finite_count,
            char2_normal_hiragana_count,
            no_thumb_followup_count,
            thumb_watch_window_thumb_arrived_count,
            thumb_watch_window_abandoned_count,
            followup_elapsed_ms_histogram,
            followup_overwritten_count,
            followup_dropped_imprecise_count,
            phase2_decisions_total,
            phase2_correction_histogram,
            phase1_decisions_total,
            phase1_correction_histogram,
            baseline_decisions_total,
            baseline_correction_histogram,
            escape_output_count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BugReportDiagnostics {
    pub ime_product_name: Option<String>,
    pub keyboard_model: String,
    pub windows_keyboard_layout: String,
    pub competing_software: Vec<String>,
    pub state_snapshot: Option<BugReportStateSnapshot>,
    pub config_toml: Option<String>,
    pub layout_yab: Option<String>,
    /// ADR-120 決定0a-report。`SCHEMA_VERSION` は上げていないため、旧クライアント
    /// が生成した診断JSONにはこのフィールドが存在しない。`#[serde(default)]`
    /// を外すと、その旧データを読み込んだ際に `state_snapshot` 等
    /// **既存の診断情報も含めて全部**が `load_diagnostics` の `.ok()` で静かに
    /// 消える（`crates/awase-settings/src/bug_report.rs` 参照）ため必須。
    #[serde(default)]
    pub retro_eval_stats: Option<BugReportRetroEvalStats>,
    /// ADR-148。上記`retro_eval_stats`と同じ理由で`#[serde(default)]`必須
    /// （`SCHEMA_VERSION`は上げていないため、旧クライアントが生成した
    /// 診断JSONにはこの2フィールドが存在しない）。
    #[serde(default)]
    pub gji_keymap: Option<BugReportGjiKeymapSummary>,
    #[serde(default)]
    pub msime_key_assignment: Option<BugReportMsImeKeyAssignmentSummary>,
}

impl Default for BugReportDiagnostics {
    fn default() -> Self {
        Self {
            ime_product_name: None,
            keyboard_model: "Jis".to_owned(),
            windows_keyboard_layout: "unavailable".to_owned(),
            competing_software: Vec::new(),
            state_snapshot: None,
            config_toml: None,
            layout_yab: None,
            retro_eval_stats: None,
            gji_keymap: None,
            msime_key_assignment: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BugReportInput<'a> {
    pub app_version: &'a str,
    pub os_version: &'a str,
    pub ime_kind: BugReportImeKind,
    pub ime_product_name: Option<&'a str>,
    pub keyboard_model: &'a str,
    pub windows_keyboard_layout: &'a str,
    pub competing_software: Vec<String>,
    pub symptom_category: SymptomCategory,
    pub description: &'a str,
    pub attach_log: bool,
    pub journal_json: Option<&'a str>,
    /// 実際の `log::` 出力（`awase.log`）の生テキスト。`attach_log` で
    /// `journal_json` と一緒に添付するかどうかを制御する（BUG-34 横展開）。
    pub app_log: Option<&'a str>,
    pub state_snapshot: Option<BugReportStateSnapshot>,
    pub attach_state_snapshot: bool,
    pub config_toml: Option<&'a str>,
    pub attach_config: bool,
    pub layout_yab: Option<&'a str>,
    pub attach_layout: bool,
    /// ADR-120 決定0a-report。呼び出し側（`crates/awase-windows/src/runtime/message_handlers.rs`
    /// の `current_bug_report_diagnostics`）が `Engine::retro_eval_stats()` から
    /// 変換して渡す。
    pub attach_retro_eval_stats: bool,
    pub retro_eval_stats: Option<BugReportRetroEvalStats>,
    /// ADR-148。呼び出し側（`current_bug_report_diagnostics`）が
    /// `ime_kind`に応じたゲート済みの値を構築して渡す。
    pub attach_ime_keymap: bool,
    pub gji_keymap: Option<BugReportGjiKeymapSummary>,
    pub msime_key_assignment: Option<BugReportMsImeKeyAssignmentSummary>,
    pub reported_at: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum BugReportPayloadError {
    #[error("症状カテゴリがその他の場合は説明を入力してください")]
    DescriptionRequiredForOther,
    #[error("JSON シリアライズ失敗: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// `build_payload` と同じだが、journal/app_log 添付の切り詰め上限
/// （既定は `LOG_EXCERPT_MAX_BYTES`）を呼び出し側で指定できる。
/// `build_payload_json_fitting` が `MAX_BODY_BYTES` に収まるまで
/// この上限を段階的に縮小しながら再構築するために使う。
pub fn build_payload_with_log_budget(
    input: &BugReportInput<'_>,
    log_excerpt_max_bytes: usize,
) -> Result<BugReportPayload, BugReportPayloadError> {
    let description = truncate_chars(input.description.trim(), DESCRIPTION_MAX_CHARS);
    if input.symptom_category == SymptomCategory::Other && description.is_empty() {
        return Err(BugReportPayloadError::DescriptionRequiredForOther);
    }
    let log_excerpt = if input.attach_log {
        input
            .journal_json
            .map(|log| truncate_journal_json_tail(log, log_excerpt_max_bytes))
    } else {
        None
    };
    let app_log_excerpt = if input.attach_log {
        input
            .app_log
            .map(|log| truncate_text_tail(log, log_excerpt_max_bytes))
    } else {
        None
    };
    let state_snapshot = if input.attach_state_snapshot {
        input.state_snapshot.clone()
    } else {
        None
    };
    let config_toml = if input.attach_config {
        input.config_toml.map(str::to_owned)
    } else {
        None
    };
    let layout_yab = if input.attach_layout {
        input.layout_yab.map(str::to_owned)
    } else {
        None
    };
    let retro_eval_stats = if input.attach_retro_eval_stats {
        input.retro_eval_stats
    } else {
        None
    };
    let gji_keymap = if input.attach_ime_keymap {
        input.gji_keymap.clone()
    } else {
        None
    };
    let msime_key_assignment = if input.attach_ime_keymap {
        input.msime_key_assignment.clone()
    } else {
        None
    };
    Ok(BugReportPayload {
        schema_version: SCHEMA_VERSION,
        app_version: input.app_version.to_owned(),
        os_version: input.os_version.to_owned(),
        ime_kind: input.ime_kind.as_str().to_owned(),
        ime_product_name: input.ime_product_name.map(str::to_owned),
        keyboard_model: input.keyboard_model.to_owned(),
        windows_keyboard_layout: input.windows_keyboard_layout.to_owned(),
        competing_software: input.competing_software.clone(),
        symptom_category: input.symptom_category,
        description,
        attach_state_snapshot: input.attach_state_snapshot,
        state_snapshot,
        attach_config: input.attach_config,
        config_toml,
        attach_layout: input.attach_layout,
        layout_yab,
        attach_log: input.attach_log,
        log_excerpt,
        app_log_excerpt,
        attach_retro_eval_stats: input.attach_retro_eval_stats,
        retro_eval_stats,
        attach_ime_keymap: input.attach_ime_keymap,
        gji_keymap,
        msime_key_assignment,
        reported_at: input.reported_at.to_owned(),
    })
}

pub fn build_payload(
    input: &BugReportInput<'_>,
) -> Result<BugReportPayload, BugReportPayloadError> {
    build_payload_with_log_budget(input, LOG_EXCERPT_MAX_BYTES)
}

pub fn build_payload_json(input: &BugReportInput<'_>) -> Result<String, BugReportPayloadError> {
    Ok(serde_json::to_string_pretty(&build_payload(input)?)?)
}

/// `max_body_bytes` に収まるまで journal/app_log の添付を自動的に切り詰める。
///
/// `build_payload_json` が生成した JSON が上限を超える場合、切り詰め上限
/// （既定 `LOG_EXCERPT_MAX_BYTES`）を半分ずつ縮小しながら収まるまで
/// 再構築する。他の添付（内部状態スナップショット・設定ファイル・配列
/// ファイル）は縮小の対象にしない — これらは journal/app_log と違って
/// 個々のユーザー環境で急に肥大化するものではなく、診断上も基本情報として
/// 全量が必要なため。
///
/// 戻り値は `(生成された JSON, 実際に使った log_excerpt 上限バイト数)`。
/// 予算が 0 になっても収まらない場合はそこで打ち切り、その JSON をそのまま
/// 返す（呼び出し側の `MAX_BODY_BYTES` チェックがフォールバックとして働く）。
///
/// 半減を毎回底(0)まで繰り返すと最大 log2(LOG_EXCERPT_MAX_BYTES) ≈ 18 回
/// ペイロード全体（最大数百KB）を再シリアライズすることになり、これは
/// UI スレッドから同期呼び出しされる場合に無視できないコストになる
/// （journal/app_log 以外のフィールドだけで既に上限超過している場合、
/// 半減を繰り返しても収まらず 18 回すべて無駄になる）。`MAX_HALVINGS` 回で
/// 打ち切り、それでも収まらなければ最後に一度だけ budget=0（ログ完全除去）
/// を試して終える。
const MAX_HALVINGS: u32 = 8;

pub fn build_payload_json_fitting(
    input: &BugReportInput<'_>,
    max_body_bytes: usize,
) -> Result<(String, usize), BugReportPayloadError> {
    let mut budget = LOG_EXCERPT_MAX_BYTES;
    for _ in 0..MAX_HALVINGS {
        let json = serde_json::to_string_pretty(&build_payload_with_log_budget(input, budget)?)?;
        if json.len() <= max_body_bytes || budget == 0 {
            return Ok((json, budget));
        }
        budget /= 2;
    }
    let json = serde_json::to_string_pretty(&build_payload_with_log_budget(input, 0)?)?;
    Ok((json, 0))
}

#[must_use]
pub fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

/// プレーンテキストログ（`awase.log`）の末尾を `max_bytes` 以内に切り詰める。
///
/// `truncate_journal_json_tail` と異なり JSON 構造を意識しない単純なバイト末尾
/// 切り出しで、UTF-8 文字境界のみ尊重する（境界がずれる場合は見つかるまで
/// 1 バイトずつ後方へ寄せる）。バグ報告は診断目的であり、先頭が途中の行から
/// 始まっても実害はない——直近の出来事（BUG-34 の切り分けに必要な
/// `[send-health]`/`[idle-conv-check]` 等の警告）を優先して残すことが重要。
#[must_use]
pub fn truncate_text_tail(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_owned();
    }
    let mut start = input.len() - max_bytes;
    while start < input.len() && !input.is_char_boundary(start) {
        start += 1;
    }
    input[start..].to_owned()
}

#[must_use]
pub fn truncate_journal_json_tail(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_owned();
    }
    if let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(input) {
        return truncate_json_values_tail(&values, max_bytes);
    }
    truncate_pretty_json_array_tail(input, max_bytes)
}

fn truncate_json_values_tail(values: &[serde_json::Value], max_bytes: usize) -> String {
    if max_bytes < 2 {
        return "[]".to_owned();
    }
    let mut selected = Vec::new();
    let mut used = 2usize;
    for value in values.iter().rev() {
        let Ok(item) = serde_json::to_string(value) else {
            continue;
        };
        let cost = item.len() + usize::from(!selected.is_empty());
        if used + cost <= max_bytes {
            used += cost;
            selected.push(item);
        }
    }
    selected.reverse();
    let mut json = String::from("[");
    for (index, item) in selected.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str(item);
    }
    json.push(']');
    json
}

fn truncate_pretty_json_array_tail(input: &str, max_bytes: usize) -> String {
    if max_bytes < 2 {
        return "[]".to_owned();
    }
    let lower = input.len().saturating_sub(max_bytes.saturating_sub(2));
    let Some(relative_start) = input.get(lower..).and_then(|tail| tail.find("\n  {")) else {
        return "[]".to_owned();
    };
    let start = lower + relative_start;
    let tail = input.get(start..).unwrap_or("");
    let mut json = String::with_capacity(tail.len() + 2);
    json.push('[');
    json.push_str(tail.trim_end());
    if !json.ends_with(']') {
        json.push('\n');
        json.push(']');
    }
    while json.len() > max_bytes {
        let Some(remove_start) = json.get(1..).and_then(|tail| tail.find("\n  {")) else {
            return "[]".to_owned();
        };
        let remove_start = remove_start + 1;
        let Some(next_start) = json
            .get(remove_start + 1..)
            .and_then(|tail| tail.find("\n  {"))
        else {
            return "[]".to_owned();
        };
        let next_start = remove_start + 1 + next_start;
        json.replace_range(1..next_start, "");
    }
    json
}

#[must_use]
pub fn unix_seconds_to_rfc3339(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hour = rem / 3_600;
    let minute = (rem % 3_600) / 60;
    let second = rem % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days_since_epoch: u64) -> (i32, u32, u32) {
    let z = i64::try_from(days_since_epoch).unwrap_or(i64::MAX) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + i64::from(m <= 2);
    (
        i32::try_from(year).unwrap_or(i32::MAX),
        u32::try_from(m).unwrap_or(12),
        u32::try_from(d).unwrap_or(31),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(
        description: &'a str,
        attach_log: bool,
        journal_json: Option<&'a str>,
    ) -> BugReportInput<'a> {
        BugReportInput {
            app_version: "1.14.0",
            os_version: "Windows 11 Build 22631",
            ime_kind: BugReportImeKind::Gji,
            ime_product_name: Some("Google 日本語入力"),
            keyboard_model: "Jis",
            windows_keyboard_layout: "LANGID=0x0411 (Japanese=true)",
            competing_software: vec!["やまぶき".to_owned()],
            symptom_category: SymptomCategory::WrongCharacterOutput,
            description,
            attach_log,
            journal_json,
            app_log: Some("[2026-08-20T00:00:00Z INFO awase] started"),
            state_snapshot: Some(test_state_snapshot()),
            attach_state_snapshot: true,
            config_toml: Some("general.default_layout = \"nicola\""),
            attach_config: true,
            layout_yab: Some("あ\tい"),
            attach_layout: true,
            attach_retro_eval_stats: true,
            retro_eval_stats: Some(BugReportRetroEvalStats {
                three_key_total: 42,
                ..BugReportRetroEvalStats::default()
            }),
            attach_ime_keymap: true,
            gji_keymap: Some(test_gji_keymap_summary()),
            msime_key_assignment: Some(test_msime_key_assignment_summary()),
            reported_at: "2026-08-19T12:34:56Z",
        }
    }

    fn test_gji_keymap_summary() -> BugReportGjiKeymapSummary {
        BugReportGjiKeymapSummary {
            config1_db_status: "Ok".to_owned(),
            session_keymap: Some(0),
            has_henkan_muhenkan_overlay: false,
            custom_keymap_table_present: true,
            custom_keymap_table_is_effective: true,
            // Opus敵対的コードレビューS-1: henkan/muhenkan_classified_kindが
            // "On"/"Off"になるのは、実コードでは無変換/変換キー自身
            // （VK_CONVERT/VK_NONCONVERT）がcustom_keymap_tableでIMEOn/Off
            // に割り当てられている場合のみ（`classify_thumb_key_ime_actions`
            // 参照）。VK_F21/F22だけではこの組み合わせは到達不能だったため、
            // フィクスチャに含める。
            ime_on_keys: Some(vec!["VK_F21".to_owned(), "VK_CONVERT".to_owned()]),
            ime_off_keys: Some(vec!["VK_F22".to_owned(), "VK_NONCONVERT".to_owned()]),
            ime_toggle_keys: Some(vec![]),
            mode_set_keys: Some(vec![("VK_F6".to_owned(), "Hiragana".to_owned())]),
            mode_toggle_alphanumeric_keys: Some(vec![]),
            mode_toggle_kana_type_keys: Some(vec![]),
            henkan_classified_kind: Some("On".to_owned()),
            muhenkan_classified_kind: Some("Off".to_owned()),
            henkan_adopted_kind: Some("On".to_owned()),
            muhenkan_adopted_kind: Some("Off".to_owned()),
            henkan_adopted_route: Some("ActuationAuto".to_owned()),
            muhenkan_adopted_route: Some("ActuationAuto".to_owned()),
            thumb_key_ime_warning: None,
            muhenkan_dedicated_fn_key_configured: false,
        }
    }

    fn test_msime_key_assignment_summary() -> BugReportMsImeKeyAssignmentSummary {
        BugReportMsImeKeyAssignmentSummary {
            is_key_assignment_enabled: Some(1),
            key_assignment_muhenkan: Some(1),
            key_assignment_henkan: Some(1),
            key_assignment_ctrl_space: Some(0),
            key_assignment_shift_space: Some(0),
            adopted_ime_toggle_combos: None,
            adopted_muhenkan_delegate: None,
            adopted_henkan_delegate: None,
            muhenkan_dedicated_fn_key_configured: false,
        }
    }

    fn test_state_snapshot() -> BugReportStateSnapshot {
        BugReportStateSnapshot {
            desired_open: true,
            effective_open: false,
            input_mode: "ObservedRomaji".to_owned(),
            applied: "Unknown".to_owned(),
            app_kind: "Win32".to_owned(),
            focus_kind: "Text".to_owned(),
            gji_state: "ready".to_owned(),
            send_health_last_elapsed_ms: 12,
            send_health_consecutive_slow: 0,
            send_health_breaker_tripped: false,
            idle_conv_check_in_flight_ms: None,
            idle_conv_check_abandoned_resync_count: 0,
            idle_conv_check_abandoned_normal_count: 0,
            idle_conv_check_spawned_resync_count: 0,
            idle_conv_check_spawned_normal_count: 0,
            process_uptime_secs: 3_600,
            working_set_bytes: 42_000_000,
            handle_count: 321,
            gdi_object_count: 45,
            user_object_count: 67,
            low_level_hooks_timeout_ms: Some(5000),
            wake_post_failed_lifetime_count: 0,
            hook_ring_max_occupancy: 3,
        }
    }

    #[test]
    fn build_payload_sets_schema_and_allowlisted_fields() {
        let payload = build_payload(&input(
            "変換後に取りこぼします",
            true,
            Some(r#"[{"seq":1}]"#),
        ))
        .unwrap();
        assert_eq!(payload.schema_version, 3);
        assert_eq!(payload.ime_kind, "Gji");
        assert_eq!(
            payload.ime_product_name.as_deref(),
            Some("Google 日本語入力")
        );
        assert_eq!(payload.keyboard_model, "Jis");
        assert_eq!(
            payload.windows_keyboard_layout,
            "LANGID=0x0411 (Japanese=true)"
        );
        assert_eq!(payload.competing_software, vec!["やまぶき"]);
        assert_eq!(
            payload.symptom_category,
            SymptomCategory::WrongCharacterOutput
        );
        assert_eq!(payload.log_excerpt.as_deref(), Some(r#"[{"seq":1}]"#));
        assert_eq!(
            payload.app_log_excerpt.as_deref(),
            Some("[2026-08-20T00:00:00Z INFO awase] started")
        );
    }

    #[test]
    fn attachments_are_included_only_when_requested() {
        let mut input = input("説明", true, Some("[]"));
        let payload = build_payload(&input).unwrap();
        assert!(payload.attach_state_snapshot);
        assert_eq!(payload.state_snapshot, Some(test_state_snapshot()));
        assert!(payload.attach_config);
        assert_eq!(
            payload.config_toml.as_deref(),
            Some("general.default_layout = \"nicola\"")
        );
        assert!(payload.attach_layout);
        assert_eq!(payload.layout_yab.as_deref(), Some("あ\tい"));
        assert!(payload.attach_retro_eval_stats);
        assert_eq!(
            payload.retro_eval_stats,
            Some(BugReportRetroEvalStats {
                three_key_total: 42,
                ..BugReportRetroEvalStats::default()
            })
        );
        assert!(payload.attach_ime_keymap);
        assert_eq!(payload.gji_keymap, Some(test_gji_keymap_summary()));
        assert_eq!(
            payload.msime_key_assignment,
            Some(test_msime_key_assignment_summary())
        );

        input.attach_state_snapshot = false;
        input.attach_config = false;
        input.attach_layout = false;
        input.attach_retro_eval_stats = false;
        input.attach_ime_keymap = false;
        let detached = build_payload(&input).unwrap();
        assert!(!detached.attach_state_snapshot);
        assert_eq!(detached.state_snapshot, None);
        assert!(!detached.attach_config);
        assert_eq!(detached.config_toml, None);
        assert!(!detached.attach_layout);
        assert_eq!(detached.layout_yab, None);
        assert!(!detached.attach_retro_eval_stats);
        assert_eq!(detached.retro_eval_stats, None);
        assert!(!detached.attach_ime_keymap);
        assert_eq!(detached.gji_keymap, None);
        assert_eq!(detached.msime_key_assignment, None);
    }

    #[test]
    fn empty_description_is_allowed_for_specific_category() {
        let payload = build_payload(&input("  \n\t", true, Some("[]"))).unwrap();
        assert_eq!(payload.description, "");
    }

    #[test]
    fn empty_description_is_rejected_for_other_category_after_trim() {
        let mut input = input("  \n\t", true, Some("[]"));
        input.symptom_category = SymptomCategory::Other;
        let err = build_payload(&input).unwrap_err();
        assert!(matches!(
            err,
            BugReportPayloadError::DescriptionRequiredForOther
        ));
    }

    #[test]
    fn description_is_truncated_by_char_count() {
        let desc = "あ".repeat(DESCRIPTION_MAX_CHARS + 3);
        let payload = build_payload(&input(&desc, false, None)).unwrap();
        assert_eq!(payload.description.chars().count(), DESCRIPTION_MAX_CHARS);
        assert_eq!(payload.log_excerpt, None);
    }

    #[test]
    fn log_is_attached_only_when_requested_and_truncated_by_utf8_boundary() {
        let log =
            serde_json::to_string(&vec!["あ".repeat((LOG_EXCERPT_MAX_BYTES / 3) + 10)]).unwrap();
        let payload = build_payload(&input("説明", true, Some(&log))).unwrap();
        let excerpt = payload.log_excerpt.unwrap();
        assert!(excerpt.len() <= LOG_EXCERPT_MAX_BYTES);
        assert!(excerpt.is_char_boundary(excerpt.len()));

        let detached = build_payload(&input("説明", false, Some(&log))).unwrap();
        assert_eq!(detached.log_excerpt, None);
    }

    #[test]
    fn app_log_is_attached_only_when_requested_and_truncated_by_utf8_boundary() {
        let long_log = "あ".repeat((LOG_EXCERPT_MAX_BYTES / 3) + 10);
        let mut base = input("説明", true, Some("[]"));
        base.app_log = Some(&long_log);
        let payload = build_payload(&base).unwrap();
        let excerpt = payload.app_log_excerpt.unwrap();
        assert!(excerpt.len() <= LOG_EXCERPT_MAX_BYTES);
        assert!(excerpt.is_char_boundary(excerpt.len()));
        // 末尾優先: 切り詰め後は元テキストの末尾がそのまま残っている。
        assert!(long_log.ends_with(&excerpt));

        base.attach_log = false;
        let detached = build_payload(&base).unwrap();
        assert_eq!(detached.app_log_excerpt, None);
    }

    #[test]
    fn truncate_text_tail_keeps_short_input_unchanged() {
        assert_eq!(truncate_text_tail("hello", 100), "hello");
    }

    #[test]
    fn truncate_text_tail_truncates_at_utf8_boundary_keeping_the_tail() {
        // "あ" は UTF-8 で3バイト。max_bytes=4 の素朴なバイト末尾切り出しは
        // "あ"(5..8) の途中(byte 7)を指すため、境界(byte 8)まで前方へ
        // 寄せる必要がある。結果は max_bytes 以下（境界調整は常に切り詰め側、
        // 超過方向には動かない）。
        let input_text = "ab".to_owned() + &"あ".repeat(3); // "ab" + 9バイト = 11バイト
        let truncated = truncate_text_tail(&input_text, 4);
        assert!(truncated.is_char_boundary(0));
        assert!(input_text.ends_with(&truncated));
        assert!(truncated.len() <= 4);
        assert_eq!(truncated, "あ"); // byte 8..11 の最後の1文字のみ残る
    }

    #[test]
    fn journal_log_truncation_keeps_newer_tail_and_valid_json() {
        let log = serde_json::to_string_pretty(&vec![
            serde_json::json!({"seq": 0, "entry": {"type": "Old"}}),
            serde_json::json!({"seq": 1, "entry": {"type": "Middle"}}),
            serde_json::json!({"seq": 2, "entry": {"type": "Newest"}}),
        ])
        .unwrap();
        let excerpt = truncate_journal_json_tail(&log, 95);
        let values: Vec<serde_json::Value> = serde_json::from_str(&excerpt).unwrap();
        let seqs: Vec<u64> = values.iter().map(|v| v["seq"].as_u64().unwrap()).collect();
        assert!(seqs.contains(&2));
        assert!(!seqs.contains(&0));
    }

    #[test]
    fn broken_pretty_journal_fallback_keeps_top_level_tail_as_array() {
        let log = "[\n  {\"seq\":0,\"payload\":\"old\"},\n  {\"seq\":1,\"payload\":\"new\"}\n";
        let excerpt = truncate_journal_json_tail(log, 40);
        let values: Vec<serde_json::Value> = serde_json::from_str(&excerpt).unwrap();
        let seqs: Vec<u64> = values.iter().map(|v| v["seq"].as_u64().unwrap()).collect();
        assert_eq!(seqs, vec![1]);
    }

    #[test]
    fn payload_json_matches_schema_names() {
        let json = build_payload_json(&input("説明", true, Some("[]"))).unwrap();
        assert!(json.contains("\"schema_version\": 3"));
        assert!(json.contains("\"ime_product_name\": \"Google 日本語入力\""));
        assert!(json.contains("\"keyboard_model\": \"Jis\""));
        assert!(json.contains("\"windows_keyboard_layout\": \"LANGID=0x0411 (Japanese=true)\""));
        assert!(json.contains("\"competing_software\": ["));
        assert!(json.contains("\"symptom_category\": \"WrongCharacterOutput\""));
        assert!(json.contains("\"attach_log\": true"));
        assert!(json.contains("\"log_excerpt\": \"[]\""));
        assert!(json.contains("\"app_log_excerpt\": \"[2026-08-20T00:00:00Z INFO awase] started\""));
        assert!(json.contains("\"attach_state_snapshot\": true"));
        assert!(json.contains("\"state_snapshot\": {"));
        assert!(json.contains("\"send_health_last_elapsed_ms\": 12"));
        assert!(json.contains("\"send_health_breaker_tripped\": false"));
        assert!(json.contains("\"idle_conv_check_in_flight_ms\": null"));
        assert!(json.contains("\"process_uptime_secs\": 3600"));
        assert!(json.contains("\"working_set_bytes\": 42000000"));
        assert!(json.contains("\"handle_count\": 321"));
        assert!(json.contains("\"gdi_object_count\": 45"));
        assert!(json.contains("\"user_object_count\": 67"));
        assert!(json.contains("\"low_level_hooks_timeout_ms\": 5000"));
        assert!(json.contains("\"wake_post_failed_lifetime_count\": 0"));
        assert!(json.contains("\"hook_ring_max_occupancy\": 3"));
        assert!(json.contains("\"attach_config\": true"));
        assert!(json.contains("\"config_toml\": \"general.default_layout = \\\"nicola\\\"\""));
        assert!(json.contains("\"attach_layout\": true"));
        assert!(json.contains("\"layout_yab\": \"あ\\tい\""));
        assert!(json.contains("\"attach_retro_eval_stats\": true"));
        assert!(json.contains("\"retro_eval_stats\": {"));
        assert!(json.contains("\"three_key_total\": 42"));
        assert!(json.contains("\"attach_ime_keymap\": true"));
        assert!(json.contains("\"gji_keymap\": {"));
        assert!(json.contains("\"config1_db_status\": \"Ok\""));
        assert!(json.contains("\"custom_keymap_table_is_effective\": true"));
        assert!(json.contains("\"msime_key_assignment\": {"));
        assert!(json.contains("\"key_assignment_muhenkan\": 1"));
        assert!(!json.contains("JournalEntry"));
    }

    #[test]
    fn build_payload_json_fitting_keeps_full_budget_when_already_within_limit() {
        let (json, used_budget) =
            build_payload_json_fitting(&input("説明", true, Some("[]")), MAX_BODY_BYTES).unwrap();
        assert_eq!(used_budget, LOG_EXCERPT_MAX_BYTES);
        assert!(json.len() <= MAX_BODY_BYTES);
    }

    #[test]
    fn build_payload_json_fitting_shrinks_log_budget_to_stay_under_max_body_bytes() {
        // journal と app_log を両方フルサイズ(LOG_EXCERPT_MAX_BYTES each)で
        // 添付した場合でも、上限に対して十分小さい max_body_bytes を渡せば
        // 縮小ロジックが機能することを確認する回帰テスト。journal は小さい
        // 要素を大量に並べる（1要素が LOG_EXCERPT_MAX_BYTES を超えると
        // truncate_journal_json_tail が丸ごと弾いて空配列になり、意図せず
        // 予算を使い切らないため）。
        //
        // MAX_BODY_BYTES をそのまま使わない理由: LOG_EXCERPT_MAX_BYTES は
        // 200KiB に調整済みで、journal+app_log の2本をフル添付しても
        // 400KiB(< 512KiB=MAX_BODY_BYTES)に収まり、縮小自体が発生しなく
        // なった（これは「送信のたびに必ず自動切り詰めが発生する」という
        // 実機報告を受けた意図的な改善）。縮小ロジック自体の回帰を検知する
        // ため、テストでは意図的に小さい上限を渡す。
        let journal_items: Vec<_> = (0..4_000)
            .map(|i| serde_json::json!({"seq": i, "payload": "x".repeat(80)}))
            .collect();
        let journal = serde_json::to_string(&journal_items).unwrap();
        let app_log = "a".repeat(LOG_EXCERPT_MAX_BYTES);
        let mut base = input("説明", true, Some(&journal));
        base.app_log = Some(&app_log);
        let small_max_body_bytes = 200 * 1024;
        let (json, used_budget) = build_payload_json_fitting(&base, small_max_body_bytes).unwrap();
        assert!(
            json.len() <= small_max_body_bytes,
            "fittingを試みても指定した上限を超えている: {} > {small_max_body_bytes}",
            json.len()
        );
        assert!(
            used_budget < LOG_EXCERPT_MAX_BYTES,
            "予算が縮小されていない: {used_budget}"
        );
    }

    #[test]
    fn full_size_journal_and_app_log_fit_within_max_body_bytes_without_shrinking() {
        // LOG_EXCERPT_MAX_BYTES を 256KiB から 200KiB に引き下げた理由そのもの
        // の回帰テスト。旧値では journal(256KiB) + app_log(256KiB) だけで
        // MAX_BODY_BYTES(512KiB) に達し、他のフィールドのぶんだけ確実に
        // 超過して「送信のたびに必ず自動切り詰めが発生する」実機報告が
        // あった。journal/app_log を両方フルサイズで添付しても、通常サイズの
        // 他フィールドと合わせて MAX_BODY_BYTES に収まり、追加の予算縮小が
        // 発生しないことを確認する。
        let journal_items: Vec<_> = (0..4_000)
            .map(|i| serde_json::json!({"seq": i, "payload": "x".repeat(80)}))
            .collect();
        let journal = serde_json::to_string(&journal_items).unwrap();
        let app_log = "a".repeat(LOG_EXCERPT_MAX_BYTES);
        let mut base = input("説明", true, Some(&journal));
        base.app_log = Some(&app_log);
        let (json, used_budget) = build_payload_json_fitting(&base, MAX_BODY_BYTES).unwrap();
        assert_eq!(
            used_budget, LOG_EXCERPT_MAX_BYTES,
            "journal/app_logフル添付だけで通常ケースの縮小が発生した: {used_budget}"
        );
        assert!(json.len() <= MAX_BODY_BYTES);
    }

    #[test]
    fn build_payload_json_fitting_gives_up_at_zero_budget_without_looping_forever() {
        // 上限そのものが極端に小さい（ログ以外のフィールドだけで既に超過する）
        // 異常系でも、budget=0 まで縮小して打ち切ることを確認する
        // （無限ループしない・panicしないことの回帰）。
        let (json, used_budget) =
            build_payload_json_fitting(&input("説明", true, Some("[]")), 1).unwrap();
        assert_eq!(used_budget, 0);
        assert!(!json.is_empty());
    }

    #[test]
    fn unix_seconds_format_as_rfc3339_utc() {
        assert_eq!(unix_seconds_to_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            unix_seconds_to_rfc3339(1_787_142_896),
            "2026-08-19T12:34:56Z"
        );
    }
}
