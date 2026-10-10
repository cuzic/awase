//! ガード（`architecture_guard.rs`・`layer_boundary_guard.rs`）が走査する本番コードの `src/` の根。
//! crate の物理分割（ADR-229 D4）で核 crate を足すときは `EXTRA_SRC_CRATES` に 1 行足すだけにする。

use std::path::Path;

/// crate の物理分割（ADR-229 D4）で、ガードが走査する本番コードを持つ**追加の crate**
/// （ワークスペース相対のディレクトリ。例: `"crates/awase-windows-core"`）。
/// 核 crate は `src/` 以下で元の配置（`src/state/...` など）をそのまま保つ。これで
/// ガードが持つ `"src/..."` のパス文字列も、`strip_prefix` した相対パスも変わらない。
/// 分割前は空。
pub(crate) const EXTRA_SRC_CRATES: &[&str] = &[];

pub(crate) fn workspace_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("awase-windows must be under <workspace>/crates")
        .to_path_buf()
}

/// ガードの対象にする crate のディレクトリ（このクレート + `EXTRA_SRC_CRATES`）。
pub(crate) fn src_crate_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = vec![Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()];
    dirs.extend(EXTRA_SRC_CRATES.iter().map(|c| workspace_dir().join(c)));
    dirs
}
