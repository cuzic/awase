//! journal 再生 fixture のハーネス(dev 専用・非公開、ADR-229「FCIS の汎用部品の判断」)。
//!
//! 規約は「1 ディレクトリ = 1 形式」: ディレクトリ直下の `*.json` は全て同じ型 `T` の配列で、
//! 各要素が 1 件の再生ケース。ハーネスは読み込みと件数・失敗の集計だけを持ち、
//! 「再生して何を比べるか」は呼び出し側の `check` クロージャが決める。

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

/// `replay_dir` の結果。
#[derive(Debug, Default)]
pub struct ReplayReport {
    /// 読んだ `*.json` ファイルの数。
    pub files: usize,
    /// 再生したケース(配列の要素)の数。
    pub cases: usize,
    /// 失敗の全件。各行は「ファイル名/添字: 理由」。
    pub failures: Vec<String>,
}

impl ReplayReport {
    /// 0 件の再生(ディレクトリ・ファイルの消失や空配列で素通りすること)と、失敗が 1 件でもあることを
    /// 拒む。失敗は全件を 1 回で報告する。
    ///
    /// # Panics
    /// ファイルが 0 件、ケースが 0 件、または失敗が 1 件以上のとき。
    pub fn assert_ok(&self) {
        assert!(
            self.files > 0,
            "再生対象のファイル(*.json)が 1 件もない(ディレクトリの消失・移動の可能性)"
        );
        assert!(
            self.cases > 0,
            "{} ファイルを読んだがケースが 1 件もない",
            self.files
        );
        assert!(
            self.failures.is_empty(),
            "{} 件の再生失敗({} ファイル・{} ケース中):\n\n{}",
            self.failures.len(),
            self.files,
            self.cases,
            self.failures.join("\n\n")
        );
    }
}

/// `dir` 直下(非再帰)の `*.json` をファイル名順に読み、各要素を `check` に渡す。
/// 読み込み・パースの失敗も panic せず `failures` に載せる。
pub fn replay_dir<T: DeserializeOwned>(
    dir: &Path,
    mut check: impl FnMut(&T) -> Result<(), String>,
) -> ReplayReport {
    let mut report = ReplayReport::default();
    let paths = match json_paths(dir) {
        Ok(paths) => paths,
        Err(e) => {
            report.failures.push(format!("{}: {e}", dir.display()));
            return report;
        }
    };
    for path in paths {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        report.files += 1;
        match load_file::<T>(&path) {
            Ok(items) => {
                for (i, item) in items.iter().enumerate() {
                    report.cases += 1;
                    if let Err(reason) = check(item) {
                        report.failures.push(format!("{name}/{i}: {reason}"));
                    }
                }
            }
            Err(e) => report.failures.push(format!("{name}: {e}")),
        }
    }
    report
}

/// 1 ファイルの JSON 配列を読む(特定ファイルを名指しする補助テスト向け)。
///
/// # Errors
/// 読み込み・JSON パースに失敗したとき。
pub fn load_file<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("読み込み失敗 {}: {e}", path.display()))?;
    serde_json::from_str(&content).map_err(|e| format!("JSON パース失敗 {}: {e}", path.display()))
}

fn json_paths(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("読めない: {e}"))?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|e| format!("エントリ読み取り失敗: {e}"))?
            .path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("awase-replay-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn counts_files_and_cases_and_collects_every_failure() {
        let dir = temp_dir("count");
        std::fs::write(dir.join("a.json"), "[1, 2, 3]").unwrap();
        std::fs::write(dir.join("b.json"), "[4]").unwrap();
        std::fs::write(dir.join("ignored.txt"), "[9]").unwrap();
        let report = replay_dir::<u32>(&dir, |n| {
            if n % 2 == 0 {
                Err(format!("even {n}"))
            } else {
                Ok(())
            }
        });
        assert_eq!((report.files, report.cases), (2, 4));
        assert_eq!(
            report.failures,
            vec!["a.json/1: even 2", "b.json/0: even 4"]
        );
    }

    #[test]
    fn parse_error_and_missing_dir_are_failures_not_panics() {
        let dir = temp_dir("bad");
        std::fs::write(dir.join("x.json"), "{").unwrap();
        let report = replay_dir::<u32>(&dir, |_| Ok(()));
        assert_eq!(report.files, 1);
        assert_eq!(report.failures.len(), 1);
        let missing = replay_dir::<u32>(&dir.join("nope"), |_| Ok(()));
        assert_eq!(missing.failures.len(), 1);
    }

    #[test]
    #[should_panic(expected = "1 件もない")]
    fn assert_ok_rejects_zero_files() {
        replay_dir::<u32>(&temp_dir("empty"), |_| Ok(())).assert_ok();
    }

    #[test]
    #[should_panic(expected = "ケースが 1 件もない")]
    fn assert_ok_rejects_zero_cases() {
        let dir = temp_dir("nocase");
        std::fs::write(dir.join("a.json"), "[]").unwrap();
        replay_dir::<u32>(&dir, |_| Ok(())).assert_ok();
    }

    #[test]
    #[should_panic(expected = "1 件の再生失敗")]
    fn assert_ok_rejects_failures() {
        let dir = temp_dir("fail");
        std::fs::write(dir.join("a.json"), "[1]").unwrap();
        replay_dir::<u32>(&dir, |_| Err("x".into())).assert_ok();
    }

    #[test]
    fn assert_ok_passes_when_all_cases_pass() {
        let dir = temp_dir("ok");
        std::fs::write(dir.join("a.json"), "[1]").unwrap();
        replay_dir::<u32>(&dir, |_| Ok(())).assert_ok();
    }
}
