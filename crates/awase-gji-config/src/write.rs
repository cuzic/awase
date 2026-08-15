//! `custom_keymap_table`（TSV）へ、専用Fnキー変換（ADR-091 §D3.2）のエントリを
//! 追加するための純粋ロジック。ファイル I/O・バックアップ・原子的置換は
//! 呼び出し側（プラットフォーム層）の責務（[`crate::wire::parse_top_level`]の
//! module doc と同じ分離方針）。

use crate::command::{classify_command, GjiCompositionMode, GjiModeCommand};
use crate::tsv::parse_custom_keymap_table;

/// ADR-091 §D3.2 の推奨構成: 専用Fnキーは Composition/Conversion/Prediction/
/// Suggestion 時に `SwitchKanaType` としてバインドする。Precomposition/
/// DirectInput には意図的にバインドしない（未バインド時の挙動は Phase 2 で
/// 実機検証、ADR-091 §4 Phase2-1）。
const TARGET_STATUSES: &[&str] = &["Composition", "Conversion", "Prediction", "Suggestion"];

/// 専用Fnキー1本ぶんの役割定義。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DedicatedFnKeySpec {
    /// GJI キー表記（例: `"F21"`）。
    pub key: &'static str,
    /// Precomposition 時に追加でバインドする絶対モード（`None` = 未バインド）。
    pub precomposition_mode: Option<GjiCompositionMode>,
}

/// ADR-091 §D3.2 が推奨する専用Fnキー一式（2026-08-15、ユーザー要望で拡張）。
///
/// GJI の `config1.db` への書き込みはユーザーにサインアウト/インを要求する
/// 高コストな操作のため、将来 awase が使う可能性のある構成を一度にまとめて
/// 書き込んでおく。
///
/// - **F21**: Composition 系のみ `SwitchKanaType`（相対トグル）。現状 awase が
///   実際に無変換単独タップで送信する、唯一のアクティブなキー。
/// - **F22/F23/F24**: 同じ `SwitchKanaType` に加え、Precomposition 時に
///   ひらがな/全角カナ/半角カナへの絶対設定（`CompositionModeX`、押すたびに
///   同じ結果になる冪等なコマンド）を追加でバインドする。
///
/// **F22-24 のどのキーを実際にいつ送信するかは awase 側の将来のロジックが
/// 判断する — 本モジュールは GJI 側の「受け皿」を用意するだけで、判断ロジック
/// （awase 自身の内部状態を条件にする分岐）は一切持たない。** GJI の
/// `custom_keymap_table` はキー→固定コマンドの表しか持てず「awase の内部状態を
/// 見て」という条件分岐そのものを表現できないため、これは実装上の制約でもある
/// （ADR-091 の no-new-belief 原則: 判断はあくまで awase 側に留める）。
pub const RECOMMENDED_DEDICATED_FN_KEYS: &[DedicatedFnKeySpec] = &[
    DedicatedFnKeySpec {
        key: "F21",
        precomposition_mode: None,
    },
    DedicatedFnKeySpec {
        key: "F22",
        precomposition_mode: Some(GjiCompositionMode::Hiragana),
    },
    DedicatedFnKeySpec {
        key: "F23",
        precomposition_mode: Some(GjiCompositionMode::FullKatakana),
    },
    DedicatedFnKeySpec {
        key: "F24",
        precomposition_mode: Some(GjiCompositionMode::HalfKatakana),
    },
];

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

/// ある1行（`vk_key`/`status`/`command`）が「安全に上書きできる」と認識できるか。
///
/// 3パターンを認識する:
/// - `IMEOn`/`IMEOff`（BUG-64 が記録する旧 ADR-057 世代の残骸パターン）。
/// - `SwitchKanaType`（`ToggleKanaType` に分類される）かつ `status` が
///   [`TARGET_STATUSES`] のいずれか。これは [`upsert_dedicated_fn_key_entries`]
///   自身が書き込む内容と同じであり、**既に自分が書き込んだ結果を「衝突」と
///   誤判定して2回目以降の書き込みが失敗する非冪等な挙動を防ぐ**
///   （再実行・別セッションでの再検出いずれでも同じ結果になるべきため）。
/// - `CompositionModeHiragana`/`FullKatakana`/`HalfKatakana`（`SetMode` に
///   分類される）かつ `status` が `Precomposition` かつ、その `vk_key` に
///   対応する [`RECOMMENDED_DEDICATED_FN_KEYS`] のエントリが同じモードを
///   期待している場合。F22-24 の Precomposition バインド分（同上の理由で
///   冪等性を確保する）。
fn is_recognized_safe(vk_key: &str, status: &str, command: &str) -> bool {
    match classify_command(command) {
        GjiModeCommand::ImeOn | GjiModeCommand::ImeOff => true,
        GjiModeCommand::ToggleKanaType => TARGET_STATUSES.contains(&status),
        GjiModeCommand::SetMode(mode) => {
            status == "Precomposition"
                && RECOMMENDED_DEDICATED_FN_KEYS
                    .iter()
                    .any(|spec| spec.key == vk_key && spec.precomposition_mode == Some(mode))
        }
        GjiModeCommand::ToggleAlphanumericMode | GjiModeCommand::Other => false,
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
        .all(|row| is_recognized_safe(vk_key, &row.status, &row.command));
    if all_recognized {
        ExistingBinding::KnownAwaseResidual { rows }
    } else {
        ExistingBinding::Conflict { rows }
    }
}

/// [`RECOMMENDED_DEDICATED_FN_KEYS`] 全キー（F21-F24）ぶんの既存バインドを
/// まとめて検査する。1つでも [`ExistingBinding::Conflict`] があれば全体を
/// `Conflict`（該当行を全キーぶん集約）として返す——4キーを1回の書き込みで
/// 提供する以上、一部だけ書いて一部を諦める中途半端な状態を避けるため。
#[must_use]
pub fn classify_existing_binding_set(existing_table: &str) -> ExistingBinding {
    let mut rows = Vec::new();
    let mut any_conflict = false;
    for spec in RECOMMENDED_DEDICATED_FN_KEYS {
        match classify_existing_binding(existing_table, spec.key) {
            ExistingBinding::None => {}
            ExistingBinding::KnownAwaseResidual { rows: r } => rows.extend(r),
            ExistingBinding::Conflict { rows: r } => {
                any_conflict = true;
                rows.extend(r);
            }
        }
    }
    if any_conflict {
        ExistingBinding::Conflict { rows }
    } else if rows.is_empty() {
        ExistingBinding::None
    } else {
        ExistingBinding::KnownAwaseResidual { rows }
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

/// `existing_table` に、[`RECOMMENDED_DEDICATED_FN_KEYS`] 全キー（F21-F24）を
/// 一度に追加/更新した新しい TSV 文字列を返す。
///
/// 各キーの既存行はすべて削除してから、[`TARGET_STATUSES`] への
/// `SwitchKanaType` 4行と、`precomposition_mode` が `Some` なら
/// `Precomposition` への絶対設定コマンド1行を追加する。**呼び出し側は事前に
/// [`classify_existing_binding_set`] で `Conflict` でないことを確認しておく
/// こと**（この関数自体は衝突検出をしない、純粋な TSV 組み立てのみ）。
#[must_use]
pub fn upsert_dedicated_fn_key_set(existing_table: &str) -> String {
    let target_keys: Vec<&str> = RECOMMENDED_DEDICATED_FN_KEYS
        .iter()
        .map(|spec| spec.key)
        .collect();
    let mut lines: Vec<String> = vec!["status\tkey\tcommand".to_string()];
    for row in parse_custom_keymap_table(existing_table) {
        if !target_keys.contains(&row.key.as_str()) {
            lines.push(format!("{}\t{}\t{}", row.status, row.key, row.command));
        }
    }
    for spec in RECOMMENDED_DEDICATED_FN_KEYS {
        if let Some(mode) = spec.precomposition_mode {
            lines.push(format!(
                "Precomposition\t{}\t{}",
                spec.key,
                mode.command_name()
            ));
        }
        for status in TARGET_STATUSES {
            lines.push(format!("{status}\t{}\tSwitchKanaType", spec.key));
        }
    }
    let mut result = lines.join("\n");
    result.push('\n');
    result
}

#[cfg(test)]
mod tests {
    use super::{
        classify_existing_binding, classify_existing_binding_set, upsert_dedicated_fn_key_entries,
        upsert_dedicated_fn_key_set, ExistingBinding,
    };

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

    // ── upsert_dedicated_fn_key_set / classify_existing_binding_set ──────────

    #[test]
    fn set_upsert_writes_all_four_keys_on_empty_table() {
        let result = upsert_dedicated_fn_key_set("");
        for key in ["F21", "F22", "F23", "F24"] {
            for status in ["Composition", "Conversion", "Prediction", "Suggestion"] {
                assert!(
                    result.contains(&format!("{status}\t{key}\tSwitchKanaType")),
                    "{status}\\t{key}\\tSwitchKanaType が無い: {result}"
                );
            }
        }
    }

    /// F21 は Precomposition 未バインドのまま（D3.2 の既存方針を維持）。
    #[test]
    fn set_upsert_leaves_f21_precomposition_unbound() {
        let result = upsert_dedicated_fn_key_set("");
        assert!(!result.contains("Precomposition\tF21"));
    }

    /// F22/F23/F24 は Precomposition にそれぞれ絶対設定コマンドを持つ。
    #[test]
    fn set_upsert_binds_f22_24_precomposition_to_absolute_modes() {
        let result = upsert_dedicated_fn_key_set("");
        assert!(result.contains("Precomposition\tF22\tCompositionModeHiragana"));
        assert!(result.contains("Precomposition\tF23\tCompositionModeFullKatakana"));
        assert!(result.contains("Precomposition\tF24\tCompositionModeHalfKatakana"));
    }

    #[test]
    fn set_upsert_removes_existing_rows_for_all_target_keys_before_adding() {
        let table = "status\tkey\tcommand
DirectInput\tF21\tIMEOn
Precomposition\tF22\tIMEOff
Composition\tF23\tBackspace
";
        let result = upsert_dedicated_fn_key_set(table);
        assert!(!result.contains("IMEOn"));
        assert!(!result.contains("IMEOff"));
        assert!(!result.contains("Backspace"));
    }

    #[test]
    fn set_upsert_preserves_unrelated_keys() {
        let table = "status\tkey\tcommand\nComposition\tHankaku/Zenkaku\tIMEOff\n";
        let result = upsert_dedicated_fn_key_set(table);
        assert!(result.contains("Composition\tHankaku/Zenkaku\tIMEOff"));
    }

    #[test]
    fn set_upsert_is_idempotent() {
        let once = upsert_dedicated_fn_key_set("");
        let twice = upsert_dedicated_fn_key_set(&once);
        assert_eq!(once, twice);
    }

    /// F21-24 全てが未バインドなら `None`。
    #[test]
    fn set_classify_no_existing_binding_is_none() {
        assert_eq!(
            classify_existing_binding_set("status\tkey\tcommand\n"),
            ExistingBinding::None
        );
    }

    /// 自分が直前に書き込んだ内容（F21-24 全部）は衝突扱いにならない
    /// （冪等性、`write_dedicated_fn_key_set` を複数回呼んでも失敗しないための根拠）。
    #[test]
    fn set_classify_own_prior_output_is_known_safe() {
        let table = upsert_dedicated_fn_key_set("");
        assert!(matches!(
            classify_existing_binding_set(&table),
            ExistingBinding::KnownAwaseResidual { .. }
        ));
    }

    /// F22 の Precomposition が「別のキー用の絶対モード」（例: F23 用の
    /// `CompositionModeFullKatakana`）になっていれば、そのキー自身の期待
    /// （`CompositionModeHiragana`）とは一致しないため衝突扱いのまま
    /// （他者・ユーザー自身が意図的に設定した組み合わせを誤って安全と
    /// 判定しないための固定）。
    #[test]
    fn set_classify_mismatched_precomposition_mode_is_conflict() {
        let table = "status\tkey\tcommand\nPrecomposition\tF22\tCompositionModeFullKatakana\n";
        assert!(matches!(
            classify_existing_binding_set(table),
            ExistingBinding::Conflict { .. }
        ));
    }

    /// 4キーのうち1つでも衝突があれば、全体を `Conflict` として扱う
    /// （一部だけ書いて一部を諦める中途半端な状態を避ける）。
    #[test]
    fn set_classify_any_single_conflict_makes_whole_set_conflict() {
        let table = "status\tkey\tcommand\nComposition\tF24\tBackspace\n";
        assert!(matches!(
            classify_existing_binding_set(table),
            ExistingBinding::Conflict { .. }
        ));
    }
}
