//! 「バージョン相当の情報」不一致を「要再検証」として扱う判定
//! ([ADR-196](../../../docs/adr/196-keymap-learn-truth-priority.md) 決定3a、
//! [docs/tasks/adr196-t5-revalidation-not-invalidation.md](../../../docs/tasks/adr196-t5-revalidation-not-invalidation.md))。
//!
//! [`crate::staleness`] が扱う「失効」(キーマップ設定自体の変更・永続化スキーマ版の不一致、
//! いずれも即時失効のまま)とは**別枠**の判定である。ここで扱う「バージョン相当の情報」は
//! GJI Converter本体のファイルバージョン、または Microsoft IME 本体の OS ビルド番号・
//! レガシー互換モードフラグ・`keystyle`・キー再割り当て検出の4値であり、内蔵表より
//! 新しいという保証が無いため、不一致を検出しても即座に表を捨てず「要再検証」(次回の
//! 学習機会に段階2単独の軽量再検証を促す)にとどめる。
//!
//! 「要再検証」自体は保存しないフラグで、保存されたバージョンと現在のバージョンを毎回
//! 比較して導出する。実際に永続化フィールドへ配線する処理・軽量再検証の合否判定
//! ([ADR196-T2](../../../docs/tasks/adr196-t2-mismatch-adjudication.md)の95%閾値)は
//! 別タスクの担当であり、本モジュールは比較の純粋ロジックのみを提供する。
//!
//! [`crate::staleness::FingerprintProbe`]と見た目が似た「3値・不明ならfail open」
//! パターンだが、**fail openの規則が異なる**——共有関数化はしていない(code-review指摘、
//! 2026-09-23)。`staleness::check`は「保存側にそもそも指紋が無い(`None`)」なら現在側が
//! 何であってもFresh扱いだが、本モジュールの[`needs_revalidation`]は「どちらか一方でも
//! [`EnvVersionProbe::Unconfirmed`]」なら他方の状態に関わらず常に要再検証にする(規則は
//! [`needs_revalidation`]のdoc参照)。`Unconfirmed`は`staleness`側の型には存在しない
//! 状態(「取得できたが信頼できない」)であり、`staleness`の「保存側が無ければ常にFresh」
//! という規則をそのまま流用すると「未確定」を「不明」と取り違えて見逃す。どちらか一方の
//! fail open規則だけを将来変更する際は、もう一方に同じ変更が必要か必ず確認すること。

use serde::{Deserialize, Serialize};

/// GJI/Microsoft IME本体の「バージョン相当の情報」を凝縮した不透明な4値。
/// 値の意味はIME種別ごとに異なる(GJIは`VS_FIXEDFILEINFO`の
/// `dwFileVersionMS`/`dwFileVersionLS`から得る4値、Microsoft IME本体はOSビルド番号・
/// レガシー互換モードフラグ・`keystyle`・再割り当て検出結果の4値)——本モジュールは
/// 比較にしか関心が無いため区別しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnvVersion(pub [u32; 4]);

/// 永続化ファイルへ書き出す側の値。「方式が無い/取得できなかった」は`Option`の`None`
/// (フィールド自体を省略)で表すため、ここには「取得できた」場合の2種類だけを持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoredEnvVersion {
    /// 学習時点でConverter実行ファイルの最終更新時刻が学習プロセス自身の起動時刻より
    /// 新しく、学習中に版が変わっていない保証ができなかった。以後どの版と比較しても
    /// 常に不一致(要再検証)として扱う。
    Unconfirmed,
    /// 取得できた版。
    Known(EnvVersion),
}

/// 呼び出し側がその場で計算した「現在の環境バージョン」の取得状態。
/// [`StoredEnvVersion`]と違い、比較の相手として「取得元が見つからない」
/// (`Unknown`)も表せる必要があるため3値を持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvVersionProbe {
    /// このIME/構成には版取得の方式が無い、またはConverter等の取得元が見つからない
    /// (比較対象自体が存在しない、fail open)。
    Unknown,
    /// 版取得の方式はあるが、今回は信頼できる値を得られなかった。
    Unconfirmed,
    /// 取得できた。
    Known(EnvVersion),
}

impl From<Option<StoredEnvVersion>> for EnvVersionProbe {
    fn from(stored: Option<StoredEnvVersion>) -> Self {
        match stored {
            None => Self::Unknown,
            Some(StoredEnvVersion::Unconfirmed) => Self::Unconfirmed,
            Some(StoredEnvVersion::Known(v)) => Self::Known(v),
        }
    }
}

/// 学習時に記録した環境バージョンと現在の環境バージョンを比較し、「要再検証」かどうかを
/// 判定する。
///
/// 規則([docs/tasks/adr196-t5-revalidation-not-invalidation.md](../../../docs/tasks/adr196-t5-revalidation-not-invalidation.md)
/// 「3a: 状態遷移」に対応):
/// - どちらか一方でも[`EnvVersionProbe::Unconfirmed`]なら、常に要再検証にする
///   (`Unknown`どうしの比較除外の対象外——`Unconfirmed`は`Unknown`より弱い保証しか
///   持たないため、`Unknown`と組み合わさっても「比較不能だから見逃す」扱いにはしない)。
/// - (上記に当てはまらない場合)どちらか一方でも[`EnvVersionProbe::Unknown`]なら、
///   比較不能として要再検証にしない(対称なfail open)。
/// - 両方[`EnvVersionProbe::Known`]で値が異なれば要再検証にする。
#[must_use]
pub fn needs_revalidation(stored: EnvVersionProbe, current: EnvVersionProbe) -> bool {
    match (stored, current) {
        (EnvVersionProbe::Unconfirmed, _) | (_, EnvVersionProbe::Unconfirmed) => true,
        (EnvVersionProbe::Unknown, _) | (_, EnvVersionProbe::Unknown) => false,
        (EnvVersionProbe::Known(a), EnvVersionProbe::Known(b)) => a != b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: EnvVersion = EnvVersion([1, 2, 3, 4]);
    const V2: EnvVersion = EnvVersion([1, 2, 3, 5]);

    #[test]
    fn up_to_date_when_known_versions_match() {
        assert!(!needs_revalidation(
            EnvVersionProbe::Known(V1),
            EnvVersionProbe::Known(V1)
        ));
    }

    #[test]
    fn needs_revalidation_when_known_versions_differ() {
        assert!(needs_revalidation(
            EnvVersionProbe::Known(V1),
            EnvVersionProbe::Known(V2)
        ));
    }

    #[test]
    fn fail_open_when_stored_unknown() {
        assert!(!needs_revalidation(
            EnvVersionProbe::Unknown,
            EnvVersionProbe::Known(V1)
        ));
    }

    #[test]
    fn fail_open_when_current_unknown() {
        assert!(!needs_revalidation(
            EnvVersionProbe::Known(V1),
            EnvVersionProbe::Unknown
        ));
    }

    #[test]
    fn fail_open_when_both_unknown() {
        assert!(!needs_revalidation(
            EnvVersionProbe::Unknown,
            EnvVersionProbe::Unknown
        ));
    }

    #[test]
    fn stored_unconfirmed_always_needs_revalidation_even_against_known() {
        assert!(needs_revalidation(
            EnvVersionProbe::Unconfirmed,
            EnvVersionProbe::Known(V1)
        ));
    }

    #[test]
    fn stored_unconfirmed_needs_revalidation_even_against_unknown() {
        // 「不明」どうしの比較除外(fail open)の対象外——Unconfirmedは弱い保証しか
        // 持たないため、現在側が取得元不明でも「見逃さない」側に倒す。
        assert!(needs_revalidation(
            EnvVersionProbe::Unconfirmed,
            EnvVersionProbe::Unknown
        ));
    }

    #[test]
    fn current_unconfirmed_always_needs_revalidation() {
        assert!(needs_revalidation(
            EnvVersionProbe::Known(V1),
            EnvVersionProbe::Unconfirmed
        ));
    }

    #[test]
    fn both_unconfirmed_needs_revalidation() {
        assert!(needs_revalidation(
            EnvVersionProbe::Unconfirmed,
            EnvVersionProbe::Unconfirmed
        ));
    }

    #[test]
    fn stored_none_converts_to_unknown_probe() {
        assert_eq!(EnvVersionProbe::from(None), EnvVersionProbe::Unknown);
    }

    #[test]
    fn stored_known_round_trips_through_probe_conversion() {
        assert_eq!(
            EnvVersionProbe::from(Some(StoredEnvVersion::Known(V1))),
            EnvVersionProbe::Known(V1)
        );
    }

    #[test]
    fn stored_unconfirmed_converts_to_unconfirmed_probe() {
        assert_eq!(
            EnvVersionProbe::from(Some(StoredEnvVersion::Unconfirmed)),
            EnvVersionProbe::Unconfirmed
        );
    }
}
