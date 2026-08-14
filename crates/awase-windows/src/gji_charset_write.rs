//! `config1.db` への専用Fnキー変換バインドの書き込み（ADR-091 §D3.2/§4 Phase1-3）。
//!
//! 変換ロジック本体は `awase_gji_config::write_dedicated_fn_key_binding`
//! （メモリ上のバイト列変換、衝突検出込み）。ここではファイル I/O・
//! バックアップ・原子的置換のみを担う。**呼び出しはユーザーの明示的な同意
//! （`gji_charset_popup`）を経てのみ行うこと。無断・自動では書き込まない。**

/// 書き込み失敗の理由（呼び出し元がユーザーへ説明する文言を組み立てるための分類）。
#[derive(Debug)]
pub(crate) enum ApplyError {
    /// `config1.db` のパスを解決できない（`%USERPROFILE%` 未設定等、通常起きない）。
    PathNotFound,
    /// `config1.db` の読み取りに失敗（GJI 未インストール、権限不足等）。
    ReadFailed,
    /// バックアップの作成に失敗。書き込みは行っていない（安全側で中止）。
    BackupFailed,
    /// 変換自体が失敗（protobuf として解釈できない、または既存バインドが
    /// 既知の残骸パターンと一致しない衝突）。
    Convert(awase_gji_config::WriteDedicatedFnKeyError),
    /// 一時ファイルへの書き込み、またはリネームによる原子的置換に失敗。
    /// バックアップは既に作成済みのため、元のファイルは無事なはず。
    WriteFailed,
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PathNotFound => write!(f, "config1.db のパスを解決できませんでした"),
            Self::ReadFailed => write!(
                f,
                "config1.db を読み込めませんでした（GJI が未インストール、\
                 または権限不足の可能性があります）"
            ),
            Self::BackupFailed => write!(f, "config1.db のバックアップ作成に失敗しました"),
            Self::Convert(awase_gji_config::WriteDedicatedFnKeyError::UnparsableConfig) => {
                write!(
                    f,
                    "config1.db の内容を解釈できませんでした（想定と異なる形式の可能性があります）"
                )
            }
            Self::Convert(awase_gji_config::WriteDedicatedFnKeyError::Conflict { rows }) => {
                write!(
                    f,
                    "既存のキー割当てと衝突するため書き込みを中止しました:\n{}",
                    rows.join("\n")
                )
            }
            Self::WriteFailed => write!(
                f,
                "config1.db への書き込みに失敗しました（バックアップは作成済みです）"
            ),
        }
    }
}

#[cfg(windows)]
pub(crate) use windows_impl::apply_dedicated_fn_key_binding;

#[cfg(windows)]
mod windows_impl {
    use super::ApplyError;

    /// `config1.db` へ専用Fnキー変換のバインドを書き込む。
    ///
    /// 手順: (1) 現在の `config1.db` を読む。(2)
    /// `awase_gji_config::write_dedicated_fn_key_binding` で新しいバイト列を
    /// 得る（既存バインドが既知の残骸パターンと一致しなければここで失敗し、
    /// 何も書き込まない）。(3) 元のファイルを `.awase-backup` へコピー
    /// （バックアップ）。(4) 同じディレクトリへ一時ファイルとして書き、
    /// `rename` で原子的に置換する（書き込み途中でクラッシュしても
    /// 元のファイルが壊れた状態で残らない）。
    ///
    /// **GJI プロセスが起動中の場合、この書き込みは GJI 側が再起動される
    /// までは効果を持たない**（config1.db は GJI が起動時に読み込み、
    /// 終了時にメモリ上の内容で上書きすることがあるため）。呼び出し元
    /// （`gji_charset_popup`）は書き込み成功後、ユーザーに GJI の再起動を
    /// 案内すること。
    pub(crate) fn apply_dedicated_fn_key_binding(vk_key: &str) -> Result<(), ApplyError> {
        let path = config1_db_path().ok_or(ApplyError::PathNotFound)?;
        let original = std::fs::read(&path).map_err(|_| ApplyError::ReadFailed)?;
        let new_bytes = awase_gji_config::write_dedicated_fn_key_binding(&original, vk_key)
            .map_err(ApplyError::Convert)?;

        let backup_path = backup_path(&path);
        std::fs::copy(&path, &backup_path).map_err(|_| ApplyError::BackupFailed)?;
        log::info!(
            "[gji-charset-write] config1.db をバックアップしました: {}",
            backup_path.display()
        );

        let tmp_path = tmp_path(&path);
        std::fs::write(&tmp_path, &new_bytes).map_err(|_| ApplyError::WriteFailed)?;
        std::fs::rename(&tmp_path, &path).map_err(|_| ApplyError::WriteFailed)?;
        log::info!(
            "[gji-charset-write] config1.db へ専用Fnキー変換({vk_key})を書き込みました \
             （GJI プロセスの再起動まで反映されません）"
        );
        Ok(())
    }

    fn config1_db_path() -> Option<std::path::PathBuf> {
        let profile = std::env::var_os("USERPROFILE")?;
        let mut path = std::path::PathBuf::from(profile);
        path.push("AppData");
        path.push("LocalLow");
        path.push("Google");
        path.push("Google Japanese Input");
        path.push("config1.db");
        Some(path)
    }

    /// バックアップ先パス。単一のローリングバックアップ（毎回上書き）。
    /// `gji_charset_autodetect::config1_db_path` と同じ実装を意図的に重複させて
    /// いる（このファイルだけで完結させ、モジュール間の暗黙の結合を避けるため）。
    fn backup_path(original: &std::path::Path) -> std::path::PathBuf {
        let mut backup = original.as_os_str().to_owned();
        backup.push(".awase-backup");
        std::path::PathBuf::from(backup)
    }

    /// 原子的置換用の一時ファイルパス。同じディレクトリに置くことで
    /// `rename` がファイルシステムをまたがず原子的になることを保証する。
    fn tmp_path(original: &std::path::Path) -> std::path::PathBuf {
        let mut tmp = original.as_os_str().to_owned();
        tmp.push(".awase-tmp");
        std::path::PathBuf::from(tmp)
    }
}
