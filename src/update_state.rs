use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::version::{self, SemVer};

const SCHEMA_VERSION: u32 = 1;
const SUCCESS_INTERVAL_SECS: u64 = 86_400;
const MIN_RETRY_INTERVAL_SECS: u64 = 900;

/// UNIX epoch からの経過秒数。取得に失敗したら 0 を返す。
#[must_use]
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UpdateState {
    pub schema_version: u32,
    pub last_attempted_at: Option<u64>,
    pub last_success_at: Option<u64>,
    pub last_seen_latest: Option<String>,
}

/// `update_check.json` のパス。exe と同じディレクトリに配置する。
#[must_use]
pub fn default_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("update_check.json")))
        .unwrap_or_else(|| PathBuf::from("update_check.json"))
}

/// 読めない/壊れている/存在しない/schema_version不一致は全て default() を返す。
#[must_use]
pub fn load(path: &Path) -> UpdateState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<UpdateState>(&s).ok())
        .filter(|st| st.schema_version == SCHEMA_VERSION)
        .unwrap_or_default()
}

/// 状態をアトミックに保存する。
///
/// # Errors
///
/// ファイル書き込みに失敗した場合（ディスクフル等）。
pub fn save(path: &Path, st: &UpdateState) -> anyhow::Result<()> {
    let mut st = st.clone();
    st.schema_version = SCHEMA_VERSION;
    let json = serde_json::to_vec_pretty(&st)?;
    crate::fs_atomic::write_atomic(path, &json)
}

#[must_use]
pub const fn should_attempt(st: &UpdateState, now: u64) -> bool {
    fresh_enough(st.last_success_at, now, SUCCESS_INTERVAL_SECS)
        && fresh_enough(st.last_attempted_at, now, MIN_RETRY_INTERVAL_SECS)
}

const fn fresh_enough(t: Option<u64>, now: u64, interval: u64) -> bool {
    match t {
        None => true,
        Some(t) if t > now => true,
        Some(t) => now.saturating_sub(t) >= interval,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Display {
    Disabled,
    NeverSucceeded {
        last_attempt_ago: Option<u64>,
    },
    NoUpdate {
        last_success_ago: u64,
    },
    Available {
        version: SemVer,
        last_success_ago: u64,
    },
}

#[must_use]
pub fn display(st: &UpdateState, enabled: bool, current: &str, now: u64) -> Display {
    if !enabled {
        return Display::Disabled;
    }
    let Some(since) = st.last_success_at.map(|t| now.saturating_sub(t)) else {
        return Display::NeverSucceeded {
            last_attempt_ago: st.last_attempted_at.map(|t| now.saturating_sub(t)),
        };
    };
    let cur = version::parse(current);
    match (st.last_seen_latest.as_deref().and_then(version::parse), cur) {
        (Some(v), Some(c)) if v > c => Display::Available {
            version: v,
            last_success_ago: since,
        },
        _ => Display::NoUpdate {
            last_success_ago: since,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "awase-update-state-{name}-{}-{}",
            std::process::id(),
            now_unix()
        ))
    }

    #[test]
    fn load_accepts_valid_json_and_ignores_unknown_fields() {
        let path = temp_path("valid");
        std::fs::write(
            &path,
            r#"{"schema_version":1,"last_attempted_at":10,"last_success_at":20,"last_seen_latest":"1.19.0","extra":true}"#,
        )
        .unwrap();

        assert_eq!(
            load(&path),
            UpdateState {
                schema_version: 1,
                last_attempted_at: Some(10),
                last_success_at: Some(20),
                last_seen_latest: Some("1.19.0".to_owned()),
            }
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_returns_default_for_invalid_input() {
        for (name, json) in [
            ("missing", r#"{"last_attempted_at":10}"#),
            (
                "wrong-type",
                r#"{"schema_version":1,"last_attempted_at":"10"}"#,
            ),
            ("wrong-schema", r#"{"schema_version":2}"#),
            ("broken", "{"),
        ] {
            let path = temp_path(name);
            std::fs::write(&path, json).unwrap();
            assert_eq!(load(&path), UpdateState::default(), "{name}");
            let _ = std::fs::remove_file(path);
        }
        assert_eq!(load(&temp_path("absent")), UpdateState::default());
    }

    #[test]
    fn should_attempt_uses_success_and_attempt_gates() {
        let now = 100_000;
        let h25 = 25 * 60 * 60;
        let h1 = 60 * 60;
        let m5 = 5 * 60;
        let m20 = 20 * 60;

        for (last_success_at, last_attempted_at, expected) in [
            (None, None, true),
            (Some(now - h25), Some(now - h25), true),
            (Some(now - h1), Some(now - h1), false),
            (Some(now - h25), Some(now - m5), false),
            (Some(now - h25), Some(now - m20), true),
            (None, Some(now - m5), false),
            (None, Some(now - m20), true),
            (Some(now + h1), Some(now - h25), true),
            (Some(now - h25), Some(now + h1), true),
        ] {
            let st = UpdateState {
                last_success_at,
                last_attempted_at,
                ..UpdateState::default()
            };
            assert_eq!(should_attempt(&st, now), expected, "{st:?}");
        }
    }

    #[test]
    fn fresh_enough_accepts_none_future_and_boundary() {
        let now = 1_000;
        assert!(fresh_enough(None, now, 100));
        assert!(fresh_enough(Some(now + 1), now, 100));
        assert!(!fresh_enough(Some(now - 99), now, 100));
        assert!(fresh_enough(Some(now - 100), now, 100));
    }

    #[test]
    fn display_variants_are_derived_from_inputs() {
        let now = 100_000;
        let successful = UpdateState {
            last_success_at: Some(now - 100),
            last_seen_latest: Some("1.19.0".to_owned()),
            ..UpdateState::default()
        };
        assert_eq!(
            display(&successful, false, "1.18.0", now),
            Display::Disabled
        );
        assert_eq!(
            display(
                &UpdateState {
                    last_attempted_at: Some(now - 50),
                    ..UpdateState::default()
                },
                true,
                "1.18.0",
                now
            ),
            Display::NeverSucceeded {
                last_attempt_ago: Some(50)
            }
        );
        // インストール直後、update_check.json がまだ存在せず一度も試行していない状態
        // （全新規ユーザーが最初に見る表示）。last_attempted_at も last_success_at も None。
        assert_eq!(
            display(&UpdateState::default(), true, "1.18.0", now),
            Display::NeverSucceeded {
                last_attempt_ago: None
            }
        );
        assert_eq!(
            display(&successful, true, "1.18.0", now),
            Display::Available {
                version: version::parse("1.19.0").unwrap(),
                last_success_ago: 100,
            }
        );

        for latest in ["1.18.0", "1.17.9", "bad"] {
            let st = UpdateState {
                last_success_at: Some(now - 100),
                last_seen_latest: Some(latest.to_owned()),
                ..UpdateState::default()
            };
            assert_eq!(
                display(&st, true, "1.18.0", now),
                Display::NoUpdate {
                    last_success_ago: 100
                }
            );
        }

        let future = UpdateState {
            last_success_at: Some(now + 100),
            last_seen_latest: Some("1.17.0".to_owned()),
            ..UpdateState::default()
        };
        assert_eq!(
            display(&future, true, "1.18.0", now),
            Display::NoUpdate {
                last_success_ago: 0
            }
        );
    }

    #[test]
    fn display_is_stable_for_same_input() {
        let st = UpdateState {
            last_success_at: Some(100),
            last_seen_latest: Some("1.19.0".to_owned()),
            ..UpdateState::default()
        };
        assert_eq!(
            display(&st, true, "1.18.0", 200),
            display(&st, true, "1.18.0", 200)
        );
    }

    #[test]
    fn failed_attempt_does_not_clear_last_seen_latest() {
        let path = temp_path("keep-latest");
        let mut st = UpdateState {
            last_success_at: Some(100),
            last_seen_latest: Some("1.19.0".to_owned()),
            ..UpdateState::default()
        };
        st.last_attempted_at = Some(200);
        save(&path, &st).unwrap();

        assert_eq!(load(&path).last_seen_latest.as_deref(), Some("1.19.0"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn save_round_trips_with_schema_version_one() {
        let path = temp_path("roundtrip");
        let st = UpdateState {
            schema_version: 0,
            last_attempted_at: Some(10),
            last_success_at: Some(20),
            last_seen_latest: Some("1.19.0".to_owned()),
        };
        save(&path, &st).unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.schema_version, 1);
        assert_eq!(loaded.last_seen_latest.as_deref(), Some("1.19.0"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn retry_interval_is_shorter_than_success_interval() {
        assert!(MIN_RETRY_INTERVAL_SECS < SUCCESS_INTERVAL_SECS);
    }
}
