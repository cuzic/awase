//! journal のファイル書き出し（殻。`#[cfg(windows)]`）。
//!
//! 核の `journal.rs` は ring の中身を JSON にする（`to_json`・`report_json`）ところまでで、
//! OS の時計（`hook::current_tick_ms`）・一時ディレクトリ・ファイル書き込みは持たない
//! （ADR-229 段階 B）。

use crate::journal::{DumpError, UnifiedJournal};

/// `%TEMP%/awase_journal_<tick_ms>.json` に書き出す。
fn temp_path() -> std::path::PathBuf {
    let tick = crate::hook::current_tick_ms();
    std::env::temp_dir().join(format!("awase_journal_{tick}.json"))
}

fn write(path: std::path::PathBuf, json: &str) -> Result<std::path::PathBuf, DumpError> {
    std::fs::write(&path, json).map_err(|source| DumpError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

/// `UnifiedJournal` のファイル書き出し。
pub trait JournalDumpExt {
    /// ring の中身を `%TEMP%/awase_journal_<tick_ms>.json` に書き出す。
    ///
    /// # Errors
    /// JSON 化・書き込みに失敗したとき。
    fn dump_to_file(&self) -> Result<std::path::PathBuf, DumpError>;

    /// 不具合報告用（`UnifiedJournal::report_json`）を書き出す。メインスレッド
    /// （キーボードフックと同じスレッド）で数 MB をシリアライズするため、所要時間を
    /// ログに出す（ADR-222 D2）。
    ///
    /// # Errors
    /// JSON 化・書き込みに失敗したとき。
    fn dump_to_file_for_report(&self) -> Result<std::path::PathBuf, DumpError>;
}

impl JournalDumpExt for UnifiedJournal {
    fn dump_to_file(&self) -> Result<std::path::PathBuf, DumpError> {
        let path = temp_path();
        let json = self.to_json()?;
        write(path, &json)
    }

    fn dump_to_file_for_report(&self) -> Result<std::path::PathBuf, DumpError> {
        let started = std::time::Instant::now();
        let path = temp_path();
        let (json, entries) = self.report_json()?;
        let path = write(path, &json)?;
        tracing::info!(
            "[journal] report dump: {entries} entries, {} bytes, {} ms",
            json.len(),
            started.elapsed().as_millis()
        );
        Ok(path)
    }
}
