use std::cmp::Ordering;

/// リリースページ URL の接頭辞。タグ命名規則 `vX.Y.Z` に依存する。
/// URL は必ず「この定数 + 検証済み SemVer の Display 出力」だけで組む。
const RELEASE_TAG_URL_PREFIX: &str = "https://github.com/cuzic/awase/releases/tag/v";

/// 検証済み `SemVer` からリリースページ URL を組み立てる、唯一の場所。
///
/// URL の材料は①コンパイル時定数の prefix と②`parse()` を通った `SemVer` の `Display`
/// 出力だけであり、サーバ由来の生文字列がここに混ざる経路は無い（決定5）。呼び出し側
/// （`tray.rs` の about ダイアログ／更新ページ遷移）はこの関数だけを使い、URL 組み立てを
/// 個別に再実装しない。
#[must_use]
pub fn release_url(version: &SemVer) -> String {
    format!("{RELEASE_TAG_URL_PREFIX}{version}")
}

/// パース済み・検証済みのバージョン。
///
/// フィールドは意図的に非公開（★M-2対応）: `parse()` を経由しない構築を型で禁じることで、
/// 「URL に流れるのは検証済み SemVer の `Display` 出力だけ」という決定5の主張を、規律ではなく
/// コンパイラに担保させる。`prerelease` は `parse()` 内の `valid_prerelease` を必ず通るため、
/// `[0-9A-Za-z-]` と区切りの `.` 以外の文字を含む `SemVer` は構築できない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<String>,
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| {
                compare_prerelease(self.prerelease.as_deref(), other.prerelease.as_deref())
            })
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(prerelease) = &self.prerelease {
            write!(f, "-{prerelease}")?;
        }
        Ok(())
    }
}

fn compare_prerelease(left: Option<&str>, right: Option<&str>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => compare_prerelease_identifiers(left, right),
    }
}

fn compare_prerelease_identifiers(left: &str, right: &str) -> Ordering {
    for (left_id, right_id) in left.split('.').zip(right.split('.')) {
        let ordering = compare_identifier(left_id, right_id);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.split('.').count().cmp(&right.split('.').count())
}

fn compare_identifier(left: &str, right: &str) -> Ordering {
    let left_numeric = left.chars().all(|c| c.is_ascii_digit());
    let right_numeric = right.chars().all(|c| c.is_ascii_digit());
    match (left_numeric, right_numeric) {
        (true, true) => parse_numeric_identifier(left).cmp(&parse_numeric_identifier(right)),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => left.cmp(right),
    }
}

fn parse_numeric_identifier(identifier: &str) -> u64 {
    identifier.parse::<u64>().unwrap_or(u64::MAX)
}

/// awase のタグ文字列を SemVer として解釈する。
#[must_use]
pub fn parse(s: &str) -> Option<SemVer> {
    if s.is_empty() || s.len() > 32 {
        return None;
    }

    let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
    let without_build = s.split_once('+').map_or(s, |(version, _)| version);
    let (core, prerelease) = without_build
        .split_once('-')
        .map_or((without_build, None), |(core, prerelease)| {
            (core, Some(prerelease))
        });

    if let Some(prerelease) = prerelease {
        if !valid_prerelease(prerelease) {
            return None;
        }
    }

    let mut parts = core.split('.');
    let major = parse_numeric_core(parts.next()?)?;
    let minor = parse_numeric_core(parts.next()?)?;
    let patch = parse_numeric_core(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }

    Some(SemVer {
        major,
        minor,
        patch,
        prerelease: prerelease.map(ToOwned::to_owned),
    })
}

/// `major`/`minor`/`patch` を数値として解釈する。semver 2.0 は数値コアフィールドの
/// 先行ゼロを禁じている（`"01"` は無効、`"0"` は有効）ため、`str::parse` だけでは
/// 弾けない（Rust の整数パーサは先行ゼロを許容してしまう）。
fn parse_numeric_core(s: &str) -> Option<u64> {
    if s.len() > 1 && s.starts_with('0') {
        return None;
    }
    s.parse::<u64>().ok()
}

fn valid_prerelease(prerelease: &str) -> bool {
    !prerelease.is_empty() && prerelease.split('.').all(is_valid_prerelease_identifier)
}

fn is_valid_prerelease_identifier(identifier: &str) -> bool {
    if identifier.is_empty()
        || !identifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return false;
    }
    // semver 2.0: 数字のみの識別子は先行ゼロを禁じる（英数字混在なら "-" 等が
    // 含まれ得るので対象外。"0" 自体は許可）。
    let is_numeric_only = identifier.chars().all(|c| c.is_ascii_digit());
    !is_numeric_only || identifier == "0" || !identifier.starts_with('0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_core_numbers_numerically() {
        // 数値比較。辞書順だと "1.18.0" < "1.9.0" になってしまうため必須。
        assert!(parse("1.18.0") > parse("1.9.0"));
        assert!(parse("1.17.9") < parse("1.18.0"));
        assert_eq!(parse("1.18.0"), parse("1.18.0"));
    }

    #[test]
    fn compares_prerelease_versions() {
        assert!(parse("1.18.0") > parse("1.18.0-rc1"));
        assert!(parse("1.18.0-rc2") > parse("1.18.0-rc10"));
        assert!(parse("1.18.0-rc.10") > parse("1.18.0-rc.2"));
        assert!(parse("1.0.0-alpha") > parse("1.0.0-1"));
        assert!(parse("1.0.0-alpha.1") > parse("1.0.0-alpha"));
    }

    #[test]
    fn parses_tag_prefix_and_ignores_build_metadata() {
        assert_eq!(parse("v1.18.0+build.1"), parse("1.18.0"));
        assert_eq!(
            parse("V1.18.0-rc.1+build.1").unwrap().to_string(),
            "1.18.0-rc.1"
        );
    }

    #[test]
    fn rejects_invalid_versions() {
        for input in [
            "",
            "1.18",
            "1.18.0.1",
            "abc",
            "1.x.0",
            "123456789012345678901234567890123",
        ] {
            assert_eq!(parse(input), None, "{input}");
        }
    }

    #[test]
    fn rejects_leading_zeros_in_numeric_fields() {
        // semver 2.0 は数値コアフィールド・数値のみのprerelease識別子の先行ゼロを禁じる。
        // Rustのstr::parse::<u64>はこれを許容してしまうため専用のガードが要る。
        for input in ["01.2.3", "1.02.3", "1.2.03", "1.2.3-01", "1.2.3-alpha.02"] {
            assert_eq!(parse(input), None, "{input}");
        }
        // "0" 自体は先行ゼロ扱いにしない。
        assert!(parse("0.0.0").is_some());
        assert!(parse("1.2.3-0").is_some());
    }

    #[test]
    fn validates_prerelease_character_set() {
        for input in [
            "1.19.0-rc 1",
            "1.19.0-rc\"x",
            "1.19.0-a/../../evil",
            "1.19.0-a\u{0}b",
            "1.19.0-",
            "1.19.0-a..b",
        ] {
            assert_eq!(parse(input), None, "{input:?}");
        }
        assert!(parse("1.19.0-rc.1").is_some());
        assert!(parse("1.19.0-alpha-2").is_some());
    }

    #[test]
    fn display_output_is_url_safe_for_valid_inputs() {
        for input in ["1.19.0", "v1.19.0-rc.1", "1.19.0-alpha-2+build"] {
            let formatted = parse(input).unwrap().to_string();
            assert!(
                formatted
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'),
                "{formatted}"
            );
        }
    }

    #[test]
    fn package_versions_stay_in_sync() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root_version = package_version(&root.join("Cargo.toml"));
        let windows_version = package_version(&root.join("crates/awase-windows/Cargo.toml"));
        let settings_version = package_version(&root.join("crates/awase-settings/Cargo.toml"));

        assert_eq!(root_version, windows_version);
        assert_eq!(root_version, settings_version);
    }

    #[test]
    fn release_workflow_tag_rule_matches_release_url_prefix() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let release_yml =
            std::fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();

        assert!(release_yml.contains("- 'v*'"));
        assert!(release_yml.contains("${TAG#v}"));
        assert!(RELEASE_TAG_URL_PREFIX.ends_with("/tag/v"));
    }

    fn package_version(path: &std::path::Path) -> String {
        let text = std::fs::read_to_string(path).unwrap();
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        parsed["package"]["version"].as_str().unwrap().to_owned()
    }
}
