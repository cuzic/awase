//! GJI (Google Japanese Input) config1.db パッチツール
//!
//! GJI の config1.db に F13/F14 キーバインドを追加する。
//!
//! 使用法:
//!   awase-gji-setup              # デフォルトパス自動検出
//!   awase-gji-setup <path>       # パス指定

#[cfg(windows)]
use awase_windows::gji::{default_config_path, patch};

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use std::{fs, path::PathBuf, process::ExitCode};

    fn resolve_path() -> Result<PathBuf, String> {
        let mut args = std::env::args().skip(1);
        if let Some(arg) = args.next() {
            return Ok(PathBuf::from(arg));
        }
        default_config_path().ok_or_else(|| {
            "cannot determine default path; LOCALAPPDATA not set. \
             Specify path explicitly: awase-gji-setup <path>"
                .to_string()
        })
    }

    let path = match resolve_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("target: {}", path.display());

    let data = match fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot read '{}': {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    match patch(&data) {
        Ok(None) => {
            println!("already up-to-date: all F13/F14 entries are present");
            ExitCode::SUCCESS
        }
        Ok(Some((patched, added))) => {
            let backup = path.with_extension("db.bak");
            if let Err(e) = fs::write(&backup, &data) {
                eprintln!("warning: backup failed ({}): {e}", backup.display());
            } else {
                println!("backup: {}", backup.display());
            }
            match fs::write(&path, patched.as_slice()) {
                Ok(()) => {
                    println!("patched: {} bytes -> {} bytes", data.len(), patched.len());
                    for entry in &added {
                        println!("  added: {}", entry.trim_end());
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: cannot write '{}': {e}", path.display());
                    ExitCode::FAILURE
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("awase-gji-setup: Windows only");
    std::process::exit(1);
}
