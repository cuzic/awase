//! 実行ファイルからの相対パスでリソース（`config.toml`, `layout/*.yab` 等）を
//! 解決する共通ロジック。
//!
//! 各バイナリクレート（awase-windows / awase-linux / awase-macos / awase-settings /
//! awase-yab-editor）が同じ問題を別々に解決しようとして、`current_exe().parent()`
//! だけを見る／`.exists()` チェックを忘れる、といった非対称なロジックが個別実装
//! に紛れ込んでいた。ここに一本化する。
//!
//! 2つの実行形態をサポートする:
//! - インストール後の配置: `layout/` や `config.toml` が exe と同じディレクトリにある
//! - `cargo run` / `cargo build` による開発時の実行: exe は
//!   `<workspace root>/target/{debug,release,<triple>/debug,...}/foo.exe` にあり、
//!   リソースはワークスペースルート直下（`target/` の外）にある

use std::path::{Path, PathBuf};

/// 相対パスを解決する。
///
/// 1. 絶対パスならそのまま返す。
/// 2. 実行ファイルと同じディレクトリに存在すればそれを返す。
/// 3. 実行ファイルのパスに `target` という名前のディレクトリが含まれる場合、その
///    親（ワークスペースルート）からの相対パスに存在すればそれを返す。
/// 4. どれも見つからなければ、相対パスをそのまま返す（カレントディレクトリ基準の
///    解決を呼び出し側 `std::fs` に委ねる）。
#[must_use]
pub fn resolve_relative_to_exe(path: &str) -> PathBuf {
    std::env::current_exe().map_or_else(
        |_| PathBuf::from(path),
        |exe| resolve_relative_to(&exe, path),
    )
}

fn resolve_relative_to(exe: &Path, path: &str) -> PathBuf {
    let raw = Path::new(path);
    if raw.is_absolute() {
        return raw.to_path_buf();
    }
    if let Some(dir) = exe.parent() {
        let candidate = dir.join(path);
        if candidate.exists() {
            return candidate;
        }
    }
    if let Some(workspace_root) = exe
        .ancestors()
        .find(|a| a.file_name().is_some_and(|n| n == "target"))
        .and_then(Path::parent)
    {
        let candidate = workspace_root.join(path);
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::{resolve_relative_to, Path, PathBuf};
    use std::fs;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("awase_paths_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn absolute_path_is_returned_unchanged() {
        let abs = if cfg!(windows) {
            r"C:\foo\bar"
        } else {
            "/foo/bar"
        };
        assert_eq!(
            resolve_relative_to(Path::new("/anything/exe"), abs),
            PathBuf::from(abs)
        );
    }

    #[test]
    fn prefers_directory_next_to_exe_when_present() {
        let root = unique_temp_dir("exe_sibling");
        let exe_dir = root.join("installed");
        fs::create_dir_all(exe_dir.join("layout")).unwrap();
        fs::write(exe_dir.join("layout").join("nicola.yab"), "").unwrap();
        let exe_path = exe_dir.join("awase-settings.exe");

        let resolved = resolve_relative_to(&exe_path, "layout/nicola.yab");
        assert_eq!(resolved, exe_dir.join("layout").join("nicola.yab"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn falls_back_to_workspace_root_when_run_from_cargo_target_dir() {
        // cargo run/build 実行時: exe は <root>/target/debug/awase-settings.exe に
        // あり、layout/ はワークスペースルート直下（target/ の外）にある。これが
        // 実際に踏んだ回帰（exe 隣の target/debug/layout を探しに行って見つからず、
        // ワークスペースルート直下の layout/ にフォールバックできていなかった）。
        let root = unique_temp_dir("cargo_target");
        fs::create_dir_all(root.join("layout")).unwrap();
        fs::write(root.join("layout").join("nicola.yab"), "").unwrap();
        let exe_path = root.join("target").join("debug").join("awase-settings.exe");
        fs::create_dir_all(exe_path.parent().unwrap()).unwrap();

        let resolved = resolve_relative_to(&exe_path, "layout/nicola.yab");
        assert_eq!(resolved, root.join("layout").join("nicola.yab"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn falls_back_to_relative_path_when_nothing_found() {
        let root = unique_temp_dir("nothing_found");
        let exe_path = root.join("target").join("debug").join("awase-settings.exe");
        fs::create_dir_all(exe_path.parent().unwrap()).unwrap();

        let resolved = resolve_relative_to(&exe_path, "layout/does_not_exist.yab");
        assert_eq!(resolved, PathBuf::from("layout/does_not_exist.yab"));

        let _ = fs::remove_dir_all(&root);
    }
}
