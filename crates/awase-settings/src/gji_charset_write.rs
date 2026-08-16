//! `config1.db` への専用Fnキー変換バインドの書き込み（ADR-091 §D3.2）。
//!
//! `crates/awase-windows/src/gji_charset_write.rs` と同じ手順（読み込み→
//! `awase_gji_config::write_dedicated_fn_key_binding` で変換→バックアップ→
//! 同ディレクトリへ一時ファイル書き込み→ `rename` で原子的置換）を、
//! awase-settings（設定画面プロセス）から直接呼べるよう独立実装したもの。
//! 変換ロジック本体（`awase_gji_config::write_dedicated_fn_key_binding`）は
//! 共有しているため、書き込み結果は awase.exe 側のポップアップ経由の書き込み
//! と完全に同じ形式になる。
//!
//! Windows API を一切使わない純粋な `std::fs`/`std::env` のみで実装している
//! ため `#[cfg(windows)]` では区切っていない（`awase-settings` は Linux でも
//! ビルド・テストされるため）。非 Windows 環境では `%USERPROFILE%` 相当の
//! 環境変数が無く `config1_db_path()` が `None` を返すため、
//! [`ApplyError::PathNotFound`] で自然に失敗する。

/// 書き込み失敗の理由（呼び出し元がユーザーへ説明する文言を組み立てるための分類）。
#[derive(Debug)]
pub(crate) enum ApplyError {
    /// `config1.db` のパスを解決できない（`%USERPROFILE%` 未設定・非 Windows 等）。
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
            Self::PathNotFound => write!(
                f,
                "config1.db のパスを解決できませんでした（Windows 環境で \
                 Google 日本語入力がインストールされている必要があります）"
            ),
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
            Self::Convert(awase_gji_config::WriteDedicatedFnKeyError::NotCustomKeymap) => write!(
                f,
                "Google 日本語入力の「キー設定」がカスタム以外（ATOK 風/MS-IME 風等の\
                 プリセット）になっているため、設定を追加できませんでした。\n\n\
                 GJI の設定メニューから「キー設定」を「カスタム」に切り替えてから、\
                 もう一度お試しください。"
            ),
            Self::WriteFailed => write!(
                f,
                "config1.db への書き込みに失敗しました（バックアップは作成済みです）"
            ),
        }
    }
}

/// `config1.db` へ専用Fnキー変換のバインドを書き込む。
///
/// `vk_key` は GJI 内部のキー表記（`awase_gji_config`/Mozc `key_parser`
/// 形式、例: `"F21"`）を渡すこと。`"VK_F21"` 形式（awase 側の内部表記）
/// ではない点に注意——呼び出し元（`dedicated_fn_key_combo` の表示ラベル）は
/// 既に GJI 形式（`"F21"`）なのでそのまま渡せる。
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
/// 終了時にメモリ上の内容で上書きすることがあるため）。GJI が起動中のまま
/// 「タスクトレイから終了→起動」を行うと、GJI 終了時に書き込み前の内容で
/// 上書きされ、この変更が消えることがある。確実に反映するには
/// **サインアウトしてからサインインし直す**必要がある
/// （`gji_charset_popup.rs` の案内文と同じ注意点）。
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

#[cfg(test)]
mod tests {
    use super::ApplyError;

    /// `ApplyError`の各バリアントが空文字にならず、UIにそのまま
    /// 表示しても壊れないことを固定する（変換ロジック本体の byte 列
    /// テストは `awase-gji-config` 側で網羅済みのため、ここでは
    /// Display 実装のみを対象にする）。
    #[test]
    fn apply_error_display_is_non_empty_for_every_variant() {
        let variants = [
            ApplyError::PathNotFound,
            ApplyError::ReadFailed,
            ApplyError::BackupFailed,
            ApplyError::Convert(awase_gji_config::WriteDedicatedFnKeyError::UnparsableConfig),
            ApplyError::Convert(awase_gji_config::WriteDedicatedFnKeyError::Conflict {
                rows: vec!["DirectInput\tF21\tIMEOn".to_string()],
            }),
            ApplyError::Convert(awase_gji_config::WriteDedicatedFnKeyError::NotCustomKeymap),
            ApplyError::WriteFailed,
        ];
        for variant in variants {
            let text = variant.to_string();
            assert!(!text.trim().is_empty(), "{variant:?} の Display が空文字");
        }
    }
}
