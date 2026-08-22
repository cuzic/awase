//! 一時ファイル＋`rename` によるアトミックなファイル書き込み。
//!
//! `AppConfig::save`（[`crate::config`]）と GJI `config1.db` への書き込み
//! （`awase-windows::gji_charset_write`）の両方が「途中で失敗しても元の
//! ファイルを壊さない」という同じ要件を持つため、この共通処理として集約する。

use anyhow::{Context, Result};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// `path` へ `content` をアトミックに書き込む。
///
/// - `path` が既存ファイルへのシンボリックリンクの場合、`rename` は
///   リンクエントリ自体を張り替えてしまう（リンク先ではなくリンクそのものが
///   通常ファイルに置き換わる）ため、事前に `canonicalize` でリンク先の
///   実体パスへ解決してから書き込む。
/// - 既存ファイルがあればそのパーミッションを新しいファイルへ引き継ぐ
///   （`File::create` は umask 依存のデフォルト権限になるため、明示的に
///   揃えないと `chmod` 済みのファイルが保存のたびに権限を失う）。
/// - 宛先が読み取り専用（Windows の `FILE_ATTRIBUTE_READONLY` 等）の場合、
///   `rename` は何度リトライしても成功しないため、リトライはせず即座に
///   分かりやすいエラーを返す。
/// - それ以外の `rename` 失敗（AV スキャナ・OneDrive 等が一時的にファイルを
///   開いている場合を想定）は 50ms 間隔で最大4回リトライする
///   （初回試行と合わせて最大5回試行・最大200msブロック）。
///
/// # Errors
///
/// 一時ファイルの作成・書き込み・fsync・`rename`（読み取り専用宛先を除く
/// リトライ尽き後を含む）のいずれかに失敗した場合にエラーを返す。
///
/// # Panics
///
/// 発生しない（`last_err` が `Some` であることを確認済みの分岐でのみ
/// 中身を取り出す）。
pub fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    let mut tmp_name = target.as_os_str().to_owned();
    tmp_name.push(format!(".tmp.{}", std::process::id()));
    let tmp_path = PathBuf::from(tmp_name);

    {
        let mut f = std::fs::File::create(&tmp_path)
            .with_context(|| format!("Failed to create {}", tmp_path.display()))?;
        f.write_all(content)
            .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
        f.sync_all()
            .with_context(|| format!("Failed to fsync {}", tmp_path.display()))?;
    }

    if let Ok(metadata) = std::fs::metadata(&target) {
        let _ = std::fs::set_permissions(&tmp_path, metadata.permissions());
    }

    let mut last_err = std::fs::rename(&tmp_path, &target).err();

    if let Some(err) = last_err.as_ref() {
        let is_readonly = std::fs::metadata(&target).is_ok_and(|m| m.permissions().readonly());
        if is_readonly {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(anyhow::anyhow!(
                "{} は読み取り専用のため書き込めません（ファイルの属性を確認してください）: {err}",
                target.display()
            ));
        }
    }

    for _ in 0..4 {
        if last_err.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        last_err = std::fs::rename(&tmp_path, &target).err();
    }
    last_err.map_or_else(
        || Ok(()),
        |e| {
            let _ = std::fs::remove_file(&tmp_path);
            Err(e).with_context(|| format!("Failed to rename into {}", target.display()))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "awase-fs-atomic-test-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn write_atomic_creates_new_file() {
        let path = unique_test_path("new");
        write_atomic(&path, b"hello").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_atomic_replaces_via_rename_not_in_place() {
        let path = unique_test_path("rename");
        std::fs::write(&path, b"old").unwrap();
        #[cfg(unix)]
        let original_ino = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&path).unwrap());

        write_atomic(&path, b"new content").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new content");
        #[cfg(unix)]
        {
            let new_ino = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&path).unwrap());
            assert_ne!(
                original_ino, new_ino,
                "expected rename-based replace, not in-place write"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_preserves_existing_permissions() {
        use std::os::unix::fs::PermissionsExt as _;
        let path = unique_test_path("perms");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        write_atomic(&path, b"new content").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_follows_symlink_to_target() {
        let target = unique_test_path("symlink-target");
        let link = unique_test_path("symlink-link");
        std::fs::write(&target, b"old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        write_atomic(&link, b"new content").unwrap();

        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "symlink itself should survive the write"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"new content");
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_file(&target);
    }

    /// 異常系: `rename` を意図的に失敗させ、(a) 元の宛先が無傷で残ること、
    /// (b) 一時ファイルが残らないこと（リトライ尽き後のクリーンアップ）を確認する。
    /// 宛先パスを既存ディレクトリにすることで `rename` を確実に失敗させる。
    #[test]
    fn write_atomic_leaves_no_tmp_residue_when_rename_fails() {
        let dir_as_dest = unique_test_path("fail-dir");
        std::fs::create_dir(&dir_as_dest).unwrap();

        let result = write_atomic(&dir_as_dest, b"content");
        assert!(
            result.is_err(),
            "write_atomic into a directory path must fail"
        );

        let stray_tmp_files: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&*dir_as_dest.file_name().unwrap().to_string_lossy())
            })
            .filter(|entry| entry.path() != dir_as_dest)
            .collect();
        assert!(
            stray_tmp_files.is_empty(),
            "temp file must be cleaned up after retries are exhausted, found: {stray_tmp_files:?}"
        );
        assert!(
            dir_as_dest.is_dir(),
            "the original destination must be left untouched on failure"
        );
        let _ = std::fs::remove_dir_all(&dir_as_dest);
    }

    /// 事前に同名の `<path>.tmp.<pid>` が残っていても、`write_atomic` が
    /// 同じ内容で上書きして正常に完了すること。
    #[test]
    fn write_atomic_succeeds_when_stale_tmp_file_with_same_name_already_exists() {
        let path = unique_test_path("stale-tmp");
        let mut tmp_name = path.as_os_str().to_owned();
        tmp_name.push(format!(".tmp.{}", std::process::id()));
        let tmp_path = PathBuf::from(tmp_name);
        std::fs::write(&tmp_path, "stale leftover from a previous crashed run").unwrap();

        write_atomic(&path, b"fresh content").unwrap();
        let content = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&tmp_path);

        assert_eq!(content, b"fresh content");
    }

    // NOTE: 読み取り専用宛先での即時失敗（リトライ省略）は Windows の
    // `MOVEFILE_REPLACE_EXISTING` 特有の挙動（宛先の `FILE_ATTRIBUTE_READONLY`
    // で `rename` 自体が失敗する）に対する防御であり、POSIX の `rename` は
    // 宛先ファイルの権限ビットではなくディレクトリの書き込み権限で判定される
    // ため、Linux 上では意図的に再現できない（`cargo xwin build --tests` で
    // コンパイルのみ確認、Windows 実機検証は未実施）。
}
