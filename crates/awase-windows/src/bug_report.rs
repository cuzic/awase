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
pub const SCHEMA_VERSION: u8 = 1;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BugReportPayload {
    pub schema_version: u8,
    pub app_version: String,
    pub os_version: String,
    pub ime_kind: String,
    pub description: String,
    pub attach_log: bool,
    pub log_excerpt: Option<String>,
    pub reported_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BugReportInput<'a> {
    pub app_version: &'a str,
    pub os_version: &'a str,
    pub ime_kind: BugReportImeKind,
    pub description: &'a str,
    pub attach_log: bool,
    pub journal_json: Option<&'a str>,
    pub reported_at: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum BugReportPayloadError {
    #[error("説明を入力してください")]
    EmptyDescription,
    #[error("JSON シリアライズ失敗: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub fn build_payload(
    input: &BugReportInput<'_>,
) -> Result<BugReportPayload, BugReportPayloadError> {
    let description = truncate_chars(input.description.trim(), DESCRIPTION_MAX_CHARS);
    if description.is_empty() {
        return Err(BugReportPayloadError::EmptyDescription);
    }
    let log_excerpt = if input.attach_log {
        input
            .journal_json
            .map(|log| truncate_utf8_bytes(log, LOG_EXCERPT_MAX_BYTES))
    } else {
        None
    };
    Ok(BugReportPayload {
        schema_version: SCHEMA_VERSION,
        app_version: input.app_version.to_owned(),
        os_version: input.os_version.to_owned(),
        ime_kind: input.ime_kind.as_str().to_owned(),
        description,
        attach_log: input.attach_log,
        log_excerpt,
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

#[must_use]
pub fn truncate_utf8_bytes(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_owned();
    }
    let mut end = max_bytes;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_owned()
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
            description,
            attach_log,
            journal_json,
            reported_at: "2026-08-19T12:34:56Z",
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
        assert_eq!(payload.schema_version, 1);
        assert_eq!(payload.ime_kind, "Gji");
        assert_eq!(payload.log_excerpt.as_deref(), Some(r#"[{"seq":1}]"#));
    }

    #[test]
    fn empty_description_is_rejected_after_trim() {
        let err = build_payload(&input("  \n\t", true, Some("[]"))).unwrap_err();
        assert!(matches!(err, BugReportPayloadError::EmptyDescription));
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
        let log = "あ".repeat((LOG_EXCERPT_MAX_BYTES / 3) + 10);
        let payload = build_payload(&input("説明", true, Some(&log))).unwrap();
        let excerpt = payload.log_excerpt.unwrap();
        assert!(excerpt.len() <= LOG_EXCERPT_MAX_BYTES);
        assert!(excerpt.is_char_boundary(excerpt.len()));

        let detached = build_payload(&input("説明", false, Some(&log))).unwrap();
        assert_eq!(detached.log_excerpt, None);
    }

    #[test]
    fn payload_json_matches_schema_names() {
        let json = build_payload_json(&input("説明", true, Some("[]"))).unwrap();
        assert!(json.contains("\"schema_version\": 1"));
        assert!(json.contains("\"attach_log\": true"));
        assert!(json.contains("\"log_excerpt\": \"[]\""));
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
