//! `custom_keymap_table`（TSV）へ、専用Fnキー変換（ADR-091 §D3.2）のエントリを
//! 追加するための純粋ロジック。ファイル I/O・バックアップ・原子的置換は
//! 呼び出し側（プラットフォーム層）の責務（[`crate::wire::parse_top_level`]の
//! module doc と同じ分離方針）。

use crate::command::{classify_command, GjiModeCommand};
use crate::tsv::parse_custom_keymap_table;

/// ADR-091 §D3.2 の推奨構成: 専用Fnキーは Composition/Conversion/Prediction/
/// Suggestion 時に `SwitchKanaType` としてバインドする。Precomposition/
/// DirectInput には意図的にバインドしない（未バインド時の挙動は Phase 2 で
/// 実機検証、ADR-091 §4 Phase2-1）。
const TARGET_STATUSES: &[&str] = &["Composition", "Conversion", "Prediction", "Suggestion"];

/// `vk_key`（GJI キー表記、例: `"F21"`）の既存バインドを検査した結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExistingBinding {
    /// 既存バインドなし。安全に追加できる。
    None,
    /// 既存バインドは全て `IMEOn`/`IMEOff`（BUG-64 が記録する旧 awase 実験
    /// （ADR-057世代の config1.db パッチ方式）由来の残骸と一致するパターン）。
    /// awase 自身の残骸として安全に上書きできる。
    KnownAwaseResidual {
        /// 上書き対象になる既存の行（`status\tkey\tcommand`）。
        rows: Vec<String>,
    },
    /// `IMEOn`/`IMEOff` 以外のコマンドを含む既存バインドがある。GJI 側の
    /// 他アプリ由来の設定、またはユーザー自身の意図的な設定の可能性があり、
    /// 安全に上書きできるとは判断できない。書き込みを中止すべき。
    Conflict {
        /// 衝突の原因になった行（`status\tkey\tcommand`）。
        rows: Vec<String>,
    },
}

/// ある1行（`status`/`command`）が「安全に上書きできる」と認識できるか。
///
/// 2パターンを認識する:
/// - `IMEOn`/`IMEOff`（BUG-64 が記録する旧 ADR-057 世代の残骸パターン）。
/// - `SwitchKanaType`（`ToggleKanaType` に分類される）かつ `status` が
///   [`TARGET_STATUSES`] のいずれか。これは [`upsert_dedicated_fn_key_entries`]
///   自身が書き込む内容と同じであり、**既に自分が書き込んだ結果を「衝突」と
///   誤判定して2回目以降の書き込みが失敗する非冪等な挙動を防ぐ**
///   （再実行・別セッションでの再検出いずれでも同じ結果になるべきため）。
fn is_recognized_safe(status: &str, command: &str) -> bool {
    match classify_command(command) {
        GjiModeCommand::ImeOn | GjiModeCommand::ImeOff => true,
        GjiModeCommand::ToggleKanaType => TARGET_STATUSES.contains(&status),
        GjiModeCommand::SetMode(_)
        | GjiModeCommand::ToggleAlphanumericMode
        | GjiModeCommand::Other => false,
    }
}

/// `existing_table`（現在の `custom_keymap_table` TSV）における `vk_key` の
/// 既存バインドを検査する（ADR-091 §4 Phase1-3 の「既存バインドとの衝突検出」）。
#[must_use]
pub fn classify_existing_binding(existing_table: &str, vk_key: &str) -> ExistingBinding {
    let matching_rows: Vec<_> = parse_custom_keymap_table(existing_table)
        .into_iter()
        .filter(|row| row.key == vk_key)
        .collect();
    if matching_rows.is_empty() {
        return ExistingBinding::None;
    }
    let rows: Vec<String> = matching_rows
        .iter()
        .map(|row| format!("{}\t{}\t{}", row.status, row.key, row.command))
        .collect();
    let all_recognized = matching_rows
        .iter()
        .all(|row| is_recognized_safe(&row.status, &row.command));
    if all_recognized {
        ExistingBinding::KnownAwaseResidual { rows }
    } else {
        ExistingBinding::Conflict { rows }
    }
}

/// `existing_table` に、`vk_key` を専用Fnキー変換として追加/更新した新しい
/// TSV 文字列を返す。
///
/// `vk_key` の既存行（あれば）は全て削除してから、[`TARGET_STATUSES`] の
/// 4行（`command = SwitchKanaType`）を追加する。**呼び出し側は事前に
/// [`classify_existing_binding`] で `Conflict` でないことを確認しておくこと**
/// （この関数自体は衝突検出をしない、純粋な TSV 組み立てのみ）。
#[must_use]
pub fn upsert_dedicated_fn_key_entries(existing_table: &str, vk_key: &str) -> String {
    let mut lines: Vec<String> = vec!["status\tkey\tcommand".to_string()];
    for row in parse_custom_keymap_table(existing_table) {
        if row.key != vk_key {
            lines.push(format!("{}\t{}\t{}", row.status, row.key, row.command));
        }
    }
    for status in TARGET_STATUSES {
        lines.push(format!("{status}\t{vk_key}\tSwitchKanaType"));
    }
    let mut result = lines.join("\n");
    result.push('\n');
    result
}

#[cfg(test)]
mod tests {
    use super::{classify_existing_binding, upsert_dedicated_fn_key_entries, ExistingBinding};

    #[test]
    fn no_existing_binding_is_none() {
        assert_eq!(
            classify_existing_binding("status\tkey\tcommand\n", "F21"),
            ExistingBinding::None
        );
        assert_eq!(classify_existing_binding("", "F21"), ExistingBinding::None);
    }

    /// BUG-64 が記録する実際の残骸パターン（ADR-057 の ENTRIES 由来）。
    #[test]
    fn bug64_residual_pattern_is_known_awase_residual() {
        let table = "status\tkey\tcommand
DirectInput\tF21\tIMEOn
Precomposition\tF21\tIMEOn
Composition\tF21\tIMEOn
Conversion\tF21\tIMEOn
";
        let classification = classify_existing_binding(table, "F21");
        assert!(matches!(
            classification,
            ExistingBinding::KnownAwaseResidual { .. }
        ));
    }

    #[test]
    fn f22_ime_off_residual_is_known_awase_residual() {
        let table = "status\tkey\tcommand
Precomposition\tF22\tIMEOff
Composition\tF22\tIMEOff
Conversion\tF22\tIMEOff
";
        let classification = classify_existing_binding(table, "F22");
        assert!(matches!(
            classification,
            ExistingBinding::KnownAwaseResidual { .. }
        ));
    }

    /// 既存行に `IMEOn`/`IMEOff`/`SwitchKanaType`（対象 status）のいずれでもない
    /// コマンドが混ざっていれば、既知の残骸パターンとは一致しないため衝突扱い。
    #[test]
    fn unrelated_command_is_conflict() {
        let table = "status\tkey\tcommand\nComposition\tF21\tBackspace\n";
        let classification = classify_existing_binding(table, "F21");
        assert!(matches!(classification, ExistingBinding::Conflict { .. }));
    }

    /// `SwitchKanaType` でも、[`TARGET_STATUSES`] に含まれない `status`
    /// （例: `Precomposition`、D3.2 で意図的に未バインドとする状態）に
    /// バインドされていれば、`upsert_dedicated_fn_key_entries` が書く内容とは
    /// 異なるため衝突扱いのまま。
    #[test]
    fn switch_kana_type_at_precomposition_is_still_conflict() {
        let table = "status\tkey\tcommand\nPrecomposition\tF21\tSwitchKanaType\n";
        let classification = classify_existing_binding(table, "F21");
        assert!(matches!(classification, ExistingBinding::Conflict { .. }));
    }

    /// `upsert_dedicated_fn_key_entries` 自身が書き込んだ内容
    /// （Composition/Conversion/Prediction/Suggestion への `SwitchKanaType`）は
    /// 「既に自分が書き込んだ結果」として安全に上書きできる（冪等性の根拠）。
    #[test]
    fn own_prior_output_is_known_safe_not_conflict() {
        let table = upsert_dedicated_fn_key_entries("", "F21");
        let classification = classify_existing_binding(&table, "F21");
        assert!(
            matches!(classification, ExistingBinding::KnownAwaseResidual { .. }),
            "自分が直前に書き込んだ内容は衝突扱いにならないはず: {classification:?}"
        );
    }

    #[test]
    fn unrelated_key_does_not_affect_target_key_classification() {
        let table = "status\tkey\tcommand\nComposition\tF15\tBackspace\n";
        assert_eq!(
            classify_existing_binding(table, "F21"),
            ExistingBinding::None
        );
    }

    #[test]
    fn upsert_adds_recommended_entries_to_empty_table() {
        let result = upsert_dedicated_fn_key_entries("", "F21");
        assert_eq!(
            result,
            "status\tkey\tcommand
Composition\tF21\tSwitchKanaType
Conversion\tF21\tSwitchKanaType
Prediction\tF21\tSwitchKanaType
Suggestion\tF21\tSwitchKanaType
"
        );
    }

    /// Precomposition/DirectInput には意図的にバインドしない（D3.2）。
    #[test]
    fn upsert_never_binds_precomposition_or_directinput() {
        let result = upsert_dedicated_fn_key_entries("", "F21");
        assert!(!result.contains("Precomposition\tF21"));
        assert!(!result.contains("DirectInput\tF21"));
    }

    #[test]
    fn upsert_removes_existing_rows_for_target_key_before_adding() {
        let table = "status\tkey\tcommand
DirectInput\tF21\tIMEOn
Precomposition\tF21\tIMEOn
";
        let result = upsert_dedicated_fn_key_entries(table, "F21");
        assert!(!result.contains("IMEOn"), "旧残骸行が残ってはならない");
        assert!(result.contains("Composition\tF21\tSwitchKanaType"));
    }

    #[test]
    fn upsert_preserves_unrelated_keys() {
        let table = "status\tkey\tcommand\nComposition\tHankaku/Zenkaku\tIMEOff\n";
        let result = upsert_dedicated_fn_key_entries(table, "F21");
        assert!(result.contains("Composition\tHankaku/Zenkaku\tIMEOff"));
        assert!(result.contains("Composition\tF21\tSwitchKanaType"));
    }

    #[test]
    fn upsert_is_idempotent() {
        let once = upsert_dedicated_fn_key_entries("", "F21");
        let twice = upsert_dedicated_fn_key_entries(&once, "F21");
        assert_eq!(once, twice);
    }
}
