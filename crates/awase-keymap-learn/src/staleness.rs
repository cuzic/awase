//! 学習済み表の陳腐化検出([ADR-195](../../../docs/adr/195-keymap-learn-productization.md)
//! 「段階8: 陳腐化検出」)。
//!
//! キーマップ構成(GJIの`config1.db`のsession/custom/overlayの3値、Microsoft IME本体の
//! レジストリのキー割り当て3 DWORD。[`crate::fingerprint`]が指紋にする)が学習時点から変わっていたら、
//! または永続化のスキーマ版が現行実装と食い違っていたら、段階4の実行時読込は同梱の
//! 既定表へフォールバックするべきである。本モジュールは「失効しているか」を判定する
//! 純粋関数のみを提供する——実際にファイルを読む・フォールバックする処理は段階4
//! (`awase-windows`側)のスコープであり、本モジュールは呼ばない。
//!
//! [`check`]自身もスキーマ版を検証する（[`persist::from_json`](crate::persist::from_json)が
//! 既に`LoadError::SchemaVersionMismatch`で弾くため、`from_json`経由で得た`PersistedTable`
//! だけを渡す限りこの分岐は通常到達しない）。`PersistedTable`のフィールドは`pub`で
//! 直接構築もできるため、`from_json`を経由しない将来の呼び出し元に対する保険として
//! 冗長に持たせている——段階4の実配線では、`from_json`のエラー（ログ・診断用に原因を
//! 区別）と本関数の失効判定（フォールバックすべきか）の両方を使うことを想定する。

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
    /// 現在のキーマップの指紋を計算できなかった(一時的な読み取り失敗等)。安全側に倒し
    /// 失効扱いにする——「未対応(比較不能)」と「一時的に取得できなかった」を区別せず
    /// どちらも`None`として渡すと、実際にキーマップが変わっていても見逃す(code-review
    /// PR #253指摘: `config1_db_stamp()`はファイル不在と読み取り失敗を区別せず`None`を
    /// 返しうるため、`None`を常にFreshと解釈するのは危険)。
    FingerprintUnavailable,
    /// 表は指紋を持つが、現在のIME/構成には指紋方式が無い(比較不能)。学習表が測った構成と
    /// 今の構成が同じと確認できないので失効扱いにする(以前は`Fresh`だったが、IME切替を
    /// 検出する仕組みが他に無く、GJIの表がATOK等の下で使われうるため安全側に改めた)。
    FingerprintNotSupported,
}

impl Staleness {
    #[must_use]
    pub const fn is_stale(self) -> bool {
        !matches!(self, Self::Fresh)
    }
}

/// 呼び出し側が計算した「現在のキーマップの指紋」の状態。[`check`]は`Option<Fingerprint>`
/// ではなくこの3値を受け取る——「このIMEにはそもそも指紋方式が無い(比較不能、従来どおり
/// キーマップ変化の検出をスキップしてよい)」と「指紋方式はあるが今回は計算できなかった
/// (一時的な読み取り失敗等、安全側に倒して失効扱いにすべき)」を型で区別するため。
///
/// [`crate::revalidation::EnvVersionProbe`]と見た目が似ているが、fail openの規則は
/// 異なる(相互参照は`revalidation`モジュール側のdocを参照)——本型を真似て
/// `revalidation`側の実装を変えないこと。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerprintProbe {
    /// このIME/キーマップ種別には指紋方式が無い(比較対象自体が存在しない)。
    NotSupported,
    /// 指紋方式はあるが、今回は計算できなかった(ファイル読み取り失敗等の一時的な異常)。
    Unavailable,
    /// 計算できた。
    Computed(Fingerprint),
}

/// 読み込んだ表と、現在の環境から計算した指紋を照合し、失効しているかを判定する。
///
/// - 表自体が`fingerprint: None`で永続化されていた場合(学習時点でフィンガープリント方式が
///   無かった)は、比較対象が無いためキーマップ変化による失効は検出しない(スキーマ版の
///   検証だけ行う)。
/// - `current_fingerprint`が[`FingerprintProbe::NotSupported`]
///   (このIME/構成に指紋方式が無い)で表が指紋を持つ場合は、同じ構成と確認できないので
///   失効扱いにする([`Staleness::FingerprintNotSupported`])。表が指紋を持たなければ
///   比較対象が無いのでスキップする。
/// - `current_fingerprint`が[`FingerprintProbe::Unavailable`]
///   (指紋方式はあるが今回は計算できなかった)の場合は、「変化していない」ことを
///   確認できていないので安全側に倒し失効扱いにする。
#[must_use]
pub fn check(table: &PersistedTable, current_fingerprint: FingerprintProbe) -> Staleness {
    if table.schema_version != CURRENT_SCHEMA_VERSION {
        return Staleness::SchemaVersionMismatch {
            found: table.schema_version,
            expected: CURRENT_SCHEMA_VERSION,
        };
    }
    match (table.fingerprint, current_fingerprint) {
        (Some(_), FingerprintProbe::Unavailable) => Staleness::FingerprintUnavailable,
        (Some(_), FingerprintProbe::NotSupported) => Staleness::FingerprintNotSupported,
        (Some(stored), FingerprintProbe::Computed(current)) if stored != current => {
            Staleness::FingerprintMismatch
        }
        _ => Staleness::Fresh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_with(schema_version: u32, fingerprint: Option<Fingerprint>) -> PersistedTable {
        let mut t = PersistedTable::new(vec![]);
        t.fingerprint = fingerprint;
        t.schema_version = schema_version;
        t
    }

    #[test]
    fn fresh_when_schema_and_fingerprint_match() {
        let fp = Fingerprint(10, 20);
        let table = table_with(CURRENT_SCHEMA_VERSION, Some(fp));
        assert_eq!(
            check(&table, FingerprintProbe::Computed(fp)),
            Staleness::Fresh
        );
        assert!(!check(&table, FingerprintProbe::Computed(fp)).is_stale());
    }

    #[test]
    fn stale_when_schema_version_differs() {
        let table = table_with(CURRENT_SCHEMA_VERSION + 1, None);
        let got = check(&table, FingerprintProbe::NotSupported);
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
        let got = check(&table, FingerprintProbe::Computed(fp));
        assert!(matches!(got, Staleness::SchemaVersionMismatch { .. }));
    }

    #[test]
    fn stale_when_fingerprint_differs() {
        let table = table_with(CURRENT_SCHEMA_VERSION, Some(Fingerprint(1, 1)));
        let got = check(&table, FingerprintProbe::Computed(Fingerprint(1, 2)));
        assert_eq!(got, Staleness::FingerprintMismatch);
        assert!(got.is_stale());
    }

    #[test]
    fn stale_when_stored_fingerprint_meets_not_supported() {
        // 表が指紋を持つのに現在の構成が指紋方式を持たない(別IMEへ切替等)場合は、
        // 同じ構成と確認できないので失効扱い(旧: Fresh。B-10の見送り理由は
        // IME切替を検出する仕組みが他に無いため成り立たない)。
        let table = table_with(CURRENT_SCHEMA_VERSION, Some(Fingerprint(1, 1)));
        let got = check(&table, FingerprintProbe::NotSupported);
        assert_eq!(got, Staleness::FingerprintNotSupported);
        assert!(got.is_stale());
    }

    #[test]
    fn fresh_when_neither_side_has_a_fingerprint() {
        let table = table_with(CURRENT_SCHEMA_VERSION, None);
        assert_eq!(
            check(&table, FingerprintProbe::NotSupported),
            Staleness::Fresh
        );
    }

    #[test]
    fn stale_when_fingerprint_unavailable() {
        // 指紋方式はあるが今回は計算できなかった(config1_db_stamp()の一時的な読み取り
        // 失敗等)場合、「変化していない」ことを確認できていないので安全側に倒し
        // 失効扱いにする(code-review PR #253指摘、以前はNoneとして常にFresh扱いだった)。
        let table = table_with(CURRENT_SCHEMA_VERSION, Some(Fingerprint(1, 1)));
        let got = check(&table, FingerprintProbe::Unavailable);
        assert_eq!(got, Staleness::FingerprintUnavailable);
        assert!(got.is_stale());
    }

    #[test]
    fn fresh_when_table_has_no_stored_fingerprint() {
        // 表自体が指紋無しで永続化されていた場合も、比較対象が無いため Fresh 扱い。
        let table = table_with(CURRENT_SCHEMA_VERSION, None);
        assert_eq!(
            check(&table, FingerprintProbe::Computed(Fingerprint(1, 1))),
            Staleness::Fresh
        );
    }

    #[test]
    fn fresh_when_table_has_no_stored_fingerprint_even_if_current_is_unavailable() {
        // 表自体が指紋無しなら、Unavailableでも比較対象が無いのでFresh扱いのまま
        // (Unavailableを一律失効扱いにするのは「表に指紋がある」ときだけ)。
        let table = table_with(CURRENT_SCHEMA_VERSION, None);
        assert_eq!(
            check(&table, FingerprintProbe::Unavailable),
            Staleness::Fresh
        );
    }
}
