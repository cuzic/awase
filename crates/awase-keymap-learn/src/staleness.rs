//! 学習済み表の陳腐化検出([ADR-195](../../../docs/adr/195-keymap-learn-productization.md)
//! 「段階8: 陳腐化検出」)。
//!
//! キーマップ構成(`config1.db`、レジストリのキー割り当て)が学習時点から変わっていたら、
//! または永続化のスキーマ版が現行実装と食い違っていたら、段階4の実行時読込は同梱の
//! 既定表へフォールバックするべきである。本モジュールは「失効しているか」を判定する
//! 純粋関数のみを提供する——実際にファイルを読む・フォールバックする処理は段階4
//! (`awase-windows`側)のスコープであり、本モジュールは呼ばない。

use crate::persist::{Fingerprint, PersistedTable, CURRENT_SCHEMA_VERSION};

/// 失効の理由。`Fresh`以外はすべて「段階4は同梱の既定表へフォールバックすべき」を意味する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Staleness {
    /// 失効していない。学習済み表を採用してよい。
    Fresh,
    /// 永続化ファイルのスキーマ版が現行実装と異なる(段階5等、状態表現を変える変更の後)。
    SchemaVersionMismatch { found: u32, expected: u32 },
    /// キーマップの指紋が学習時点と異なる(`config1.db`更新・レジストリ再割り当て等)。
    FingerprintMismatch,
}

impl Staleness {
    #[must_use]
    pub const fn is_stale(self) -> bool {
        !matches!(self, Self::Fresh)
    }
}

/// 読み込んだ表と、現在の環境から計算した指紋を照合し、失効しているかを判定する。
///
/// `current_fingerprint`が`None`(呼び出し側が指紋を計算できなかった、フィンガープリント
/// 方式が無いIME等)の場合、または表自体が`fingerprint: None`で永続化されていた場合は、
/// キーマップ変化による失効は検出しない(比較対象が無いため)——スキーマ版の検証だけ行う。
#[must_use]
pub fn check(table: &PersistedTable, current_fingerprint: Option<Fingerprint>) -> Staleness {
    if table.schema_version != CURRENT_SCHEMA_VERSION {
        return Staleness::SchemaVersionMismatch {
            found: table.schema_version,
            expected: CURRENT_SCHEMA_VERSION,
        };
    }
    match (table.fingerprint, current_fingerprint) {
        (Some(stored), Some(current)) if stored != current => Staleness::FingerprintMismatch,
        _ => Staleness::Fresh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_with(schema_version: u32, fingerprint: Option<Fingerprint>) -> PersistedTable {
        let mut t = PersistedTable::new(vec![], fingerprint);
        t.schema_version = schema_version;
        t
    }

    #[test]
    fn fresh_when_schema_and_fingerprint_match() {
        let fp = Fingerprint(10, 20);
        let table = table_with(CURRENT_SCHEMA_VERSION, Some(fp));
        assert_eq!(check(&table, Some(fp)), Staleness::Fresh);
        assert!(!check(&table, Some(fp)).is_stale());
    }

    #[test]
    fn stale_when_schema_version_differs() {
        let table = table_with(CURRENT_SCHEMA_VERSION + 1, None);
        let got = check(&table, None);
        assert_eq!(
            got,
            Staleness::SchemaVersionMismatch {
                found: CURRENT_SCHEMA_VERSION + 1,
                expected: CURRENT_SCHEMA_VERSION,
            }
        );
        assert!(got.is_stale());
    }

    #[test]
    fn schema_mismatch_takes_priority_over_fingerprint_check() {
        let fp = Fingerprint(1, 1);
        let table = table_with(CURRENT_SCHEMA_VERSION + 1, Some(fp));
        // 指紋は一致しているが、スキーマ不一致が優先して報告される。
        let got = check(&table, Some(fp));
        assert!(matches!(got, Staleness::SchemaVersionMismatch { .. }));
    }

    #[test]
    fn stale_when_fingerprint_differs() {
        let table = table_with(CURRENT_SCHEMA_VERSION, Some(Fingerprint(1, 1)));
        let got = check(&table, Some(Fingerprint(1, 2)));
        assert_eq!(got, Staleness::FingerprintMismatch);
        assert!(got.is_stale());
    }

    #[test]
    fn fresh_when_current_fingerprint_unavailable() {
        // フィンガープリント方式が無いIME(MS-IME本体等)や、指紋計算に失敗した場合、
        // キーマップ変化による失効は検出できないため Fresh 扱いにする。
        let table = table_with(CURRENT_SCHEMA_VERSION, Some(Fingerprint(1, 1)));
        assert_eq!(check(&table, None), Staleness::Fresh);
    }

    #[test]
    fn fresh_when_table_has_no_stored_fingerprint() {
        // 表自体が指紋無しで永続化されていた場合も、比較対象が無いため Fresh 扱い。
        let table = table_with(CURRENT_SCHEMA_VERSION, None);
        assert_eq!(check(&table, Some(Fingerprint(1, 1))), Staleness::Fresh);
    }
}
