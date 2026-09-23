//! 学習結果(段階1/段階2の出力)の永続化フォーマット([ADR-195](../../../docs/adr/195-keymap-learn-productization.md)
//! 「段階3: 学習結果の永続化」)。
//!
//! `(Status, KeyId)` セル単位の表なので、[ADR-176](../../../docs/adr/176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md)
//! 決定6が定める「1キー1件」粒度の`config.toml`の`[[calibration]]`とは粒度が違い、別ファイル
//! (呼び出し側が`<config dir>/keymap-learn-table.json`のようなパスへ書き出す想定)に持つ。
//! 型はOS非依存の本クレート側で定義する(書き手`awase-keymap-learn-win`→本クレートの依存のみで
//! 成立させ、`awase-windows`→本クレートの依存は段階4側で発生させる)。
//!
//! `schema_version`は、段階5(隠れ状態を最小Mealy機械へ置き換え)がdevelop側の状態表現を
//! 変えるため、段階5より前に永続化した表が段階5実装後にスキーマ不一致になることを検出する
//! ために持つ(段階8の失効条件がこのフィールドを使う予定、本モジュールでは読み書きのみ)。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::model::{KeyId, Outcome, Status};

/// 現行のスキーマバージョン。表現(セルの持ち方、`Status`/`Outcome`の意味)を変えたら上げる。
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// 1セル分の永続化データ。`prediction`が`None`なのは「未測定」または「決定的と言えない
/// (段階2の自己検証で信頼度が低い、非決定と判定された等)ので予測しない」の両方を表す
/// (区別が必要になったら理由フィールドを足す、現時点では段階4が「予測なし」としてしか
/// 使わないため区別しない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedCell {
    pub status: Status,
    pub key: KeyId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prediction: Option<Outcome>,
}

/// 表全体を1ファイルに持つ永続化フォーマット。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedTable {
    pub schema_version: u32,
    pub cells: Vec<PersistedCell>,
}

impl PersistedTable {
    /// 現行スキーマ版で新規作成する。
    #[must_use]
    pub const fn new(cells: Vec<PersistedCell>) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            cells,
        }
    }

    /// JSONへシリアライズする。
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }
}

/// [`from_json`]の失敗理由。パース失敗・スキーマ版不一致・重複セルを区別する
/// (段階4のフォールバック経路がどれも「同梱の既定表へフォールバック」として
/// 扱うが、原因の切り分けはログ・診断のために保つ)。
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("keymap-learn-table のパースに失敗: {0}")]
    Parse(#[source] serde_json::Error),
    #[error("keymap-learn-table のスキーマ版が不一致(見つかった版={found}, 現行版={expected})")]
    SchemaVersionMismatch { found: u32, expected: u32 },
    #[error("keymap-learn-table に(status, key)の重複エントリがある: status={status:?}, key={key:?}")]
    DuplicateCell { status: Status, key: KeyId },
}

/// JSONから読み込み、スキーマ版が現行と一致するか・`(status, key)`の重複が無いかを検証する。
///
/// 段階4の実行時読込がこの`Err`を「同梱の既定表へフォールバック」の判断材料に使う想定
/// (フォールバック自体は段階4のスコープ、本関数は読み込み+検証のみを行う)。
pub fn from_json(s: &str) -> Result<PersistedTable, LoadError> {
    let table: PersistedTable = serde_json::from_str(s).map_err(LoadError::Parse)?;
    if table.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(LoadError::SchemaVersionMismatch {
            found: table.schema_version,
            expected: CURRENT_SCHEMA_VERSION,
        });
    }
    let mut seen: HashSet<(Status, KeyId)> = HashSet::with_capacity(table.cells.len());
    for cell in &table.cells {
        if !seen.insert((cell.status, cell.key)) {
            return Err(LoadError::DuplicateCell {
                status: cell.status,
                key: cell.key,
            });
        }
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Disposition;

    fn cell(open: bool, key: u16, prediction: Option<bool>) -> PersistedCell {
        PersistedCell {
            status: Status {
                open,
                mode: 0,
                composing: false,
            },
            key: KeyId(key),
            prediction: prediction.map(|p| Outcome {
                status: Status {
                    open: p,
                    mode: 0,
                    composing: false,
                },
                disp: Disposition::None,
            }),
        }
    }

    #[test]
    fn round_trips_through_json() {
        let table = PersistedTable::new(vec![cell(true, 0, Some(false)), cell(false, 1, None)]);

        let json = table.to_json().expect("serialize");
        let loaded = from_json(&json).expect("deserialize");

        assert_eq!(loaded, table);
        assert_eq!(loaded.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn new_table_uses_current_schema_version() {
        let table = PersistedTable::new(vec![]);
        assert_eq!(table.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn rejects_newer_schema_version() {
        let mut table = PersistedTable::new(vec![cell(true, 0, None)]);
        table.schema_version = CURRENT_SCHEMA_VERSION + 1;
        let json = table.to_json().expect("serialize");

        let err = from_json(&json).expect_err("must reject future schema version");
        match err {
            LoadError::SchemaVersionMismatch { found, expected } => {
                assert_eq!(found, CURRENT_SCHEMA_VERSION + 1);
                assert_eq!(expected, CURRENT_SCHEMA_VERSION);
            }
            other => panic!("expected schema mismatch, got {other:?}"),
        }
    }

    #[test]
    fn rejects_older_schema_version() {
        // 段階5がdevelop側の状態表現を変えると、段階5より前に永続化した表(古い版)が
        // 現行と食い違う。versionチェックが`!=`ではなく`<`のような片方向比較に
        // 誤って変更される回帰を防ぐため、新しい版だけでなく古い版も拒否することを固定する。
        const { assert!(CURRENT_SCHEMA_VERSION >= 1, "test needs a version below current") };
        let mut table = PersistedTable::new(vec![cell(true, 0, None)]);
        table.schema_version = CURRENT_SCHEMA_VERSION - 1;
        let json = table.to_json().expect("serialize");

        let err = from_json(&json).expect_err("must reject older schema version");
        match err {
            LoadError::SchemaVersionMismatch { found, expected } => {
                assert_eq!(found, CURRENT_SCHEMA_VERSION - 1);
                assert_eq!(expected, CURRENT_SCHEMA_VERSION);
            }
            other => panic!("expected schema mismatch, got {other:?}"),
        }
    }

    #[test]
    fn rejects_invalid_json() {
        let err = from_json("not json").expect_err("must reject invalid json");
        assert!(matches!(err, LoadError::Parse(_)));
    }

    #[test]
    fn rejects_duplicate_status_key_cell() {
        // 同じ(status, key)に対して食い違う2つのPersistedCellが書き込まれた場合、
        // 「どちらが勝つか」を読み込み側の実装に依存する形で黙認しない(重複を拒否する)。
        let table = PersistedTable::new(vec![
            cell(true, 0, Some(false)),
            cell(true, 0, Some(true)),
        ]);
        let json = table.to_json().expect("serialize");

        let err = from_json(&json).expect_err("must reject duplicate (status, key) entries");
        match err {
            LoadError::DuplicateCell { status, key } => {
                assert_eq!(key, KeyId(0));
                assert!(status.open);
            }
            other => panic!("expected duplicate cell error, got {other:?}"),
        }
    }
}
