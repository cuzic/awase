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
pub const LOG_EXCERPT_MAX_BYTES: usize = 256 * 1024;
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
    pub reported_at: String,
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
    pub reported_at: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum BugReportPayloadError {
    #[error("症状カテゴリがその他の場合は説明を入力してください")]
    DescriptionRequiredForOther,
    #[error("JSON シリアライズ失敗: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub fn build_payload(
    input: &BugReportInput<'_>,
) -> Result<BugReportPayload, BugReportPayloadError> {
    let description = truncate_chars(input.description.trim(), DESCRIPTION_MAX_CHARS);
    if input.symptom_category == SymptomCategory::Other && description.is_empty() {
        return Err(BugReportPayloadError::DescriptionRequiredForOther);
    }
    let log_excerpt = if input.attach_log {
        input
            .journal_json
            .map(|log| truncate_journal_json_tail(log, LOG_EXCERPT_MAX_BYTES))
    } else {
        None
    };
    let app_log_excerpt = if input.attach_log {
        input
            .app_log
            .map(|log| truncate_text_tail(log, LOG_EXCERPT_MAX_BYTES))
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
        reported_at: input.reported_at.to_owned(),
    })
}

pub fn build_payload_json(input: &BugReportInput<'_>) -> Result<String, BugReportPayloadError> {
    Ok(serde_json::to_string_pretty(&build_payload(input)?)?)
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
            reported_at: "2026-08-19T12:34:56Z",
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

        input.attach_state_snapshot = false;
        input.attach_config = false;
        input.attach_layout = false;
        let detached = build_payload(&input).unwrap();
        assert!(!detached.attach_state_snapshot);
        assert_eq!(detached.state_snapshot, None);
        assert!(!detached.attach_config);
        assert_eq!(detached.config_toml, None);
        assert!(!detached.attach_layout);
        assert_eq!(detached.layout_yab, None);
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
        assert!(json.contains(
            "\"app_log_excerpt\": \"[2026-08-20T00:00:00Z INFO awase] started\""
        ));
        assert!(json.contains("\"attach_state_snapshot\": true"));
        assert!(json.contains("\"state_snapshot\": {"));
        assert!(json.contains("\"send_health_last_elapsed_ms\": 12"));
        assert!(json.contains("\"send_health_breaker_tripped\": false"));
        assert!(json.contains("\"idle_conv_check_in_flight_ms\": null"));
        assert!(json.contains("\"attach_config\": true"));
        assert!(json.contains("\"config_toml\": \"general.default_layout = \\\"nicola\\\"\""));
        assert!(json.contains("\"attach_layout\": true"));
        assert!(json.contains("\"layout_yab\": \"あ\\tい\""));
        assert!(!json.contains("JournalEntry"));
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
