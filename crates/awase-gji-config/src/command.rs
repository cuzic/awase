//! GJI の `custom_keymap_table` に現れるコマンド名を、awase が意味を理解できる
//! 形に分類する。
//!
//! コマンド名は Mozc (<https://github.com/google/mozc>, Apache-2.0) のキーマップ
//! 定義に由来する非公式知識（`data/keymap/*.tsv` の実データ、および
//! `src/session/keymap.h` の `Commands` 列挙を参照して確認したもの）。
//! GJI 内部の「入力モード変更」コマンドは、Mozc の設計上 2 種類に明確に分かれる:
//!
//! - **絶対設定系**（`CompositionMode*`）: 現在の状態に関係なく特定の
//!   [`GjiCompositionMode`] へ直接遷移する。押されたという事実だけで
//!   遷移後のモードが一意に定まるため、awase 側は「今 GJI は X モードに
//!   なった」と確信を持って追随できる。
//! - **相対トグル系**（`ToggleAlphanumericMode`/`SwitchKanaType`）: 現在の
//!   モードに応じて遷移先が変わる。押されたことだけでは遷移先が一意に
//!   定まらないため、追随するには awase 側で現在の GJI 側モードを把握して
//!   いる必要がある。ADR-078 が懸念する「観測の増幅ループ」と同じ危険が
//!   あるため、本モジュールでは分類のみ行い、追随ロジック（Stage B/C）
//!   には使わない。
//!
//! このモジュールはコマンド名の分類のみを行う純粋関数であり、`status`/`key`
//! 列や TSV のパースには関与しない（[`crate::keymap`] の責務）。

/// Mozc の `commands::CompositionMode` 列挙に対応する、絶対的な入力モード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GjiCompositionMode {
    /// ひらがな。
    Hiragana,
    /// 全角カタカナ。
    FullKatakana,
    /// 半角カタカナ。
    HalfKatakana,
    /// 全角英数。
    FullAlphanumeric,
    /// 半角英数。
    HalfAlphanumeric,
}

/// `custom_keymap_table` の `command` 列を分類した結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GjiModeCommand {
    /// `IMEOn`。
    ImeOn,
    /// `IMEOff`。
    ImeOff,
    /// `CompositionModeHiragana`/`CompositionModeFullKatakana`/
    /// `CompositionModeHalfKatakana`/`CompositionModeFullAlphanumeric`/
    /// `CompositionModeHalfAlphanumeric`。絶対設定系。
    SetMode(GjiCompositionMode),
    /// `ToggleAlphanumericMode`。かな⇔英数トグル。相対トグル系。
    ToggleAlphanumericMode,
    /// `SwitchKanaType`/`CompositionModeSwitchKanaType`。
    /// ひらがな/全角カナ/半角カナを順送りする。相対トグル系。
    ToggleKanaType,
    /// 上記以外（`Backspace`/`Convert`/`Commit` 等、IME モードと無関係な
    /// コマンド、または未知のコマンド名）。
    Other,
}

/// GJI コマンド名の文字列を [`GjiModeCommand`] に分類する。
///
/// 未知の文字列は安全側に倒して [`GjiModeCommand::Other`] を返す
/// （読み取り専用の分類であり、誤って `Other` に分類しても実害はない。
/// 実害があるのは `Other` 以外に誤分類すること）。
#[must_use]
pub fn classify_command(command: &str) -> GjiModeCommand {
    match command {
        "IMEOn" => GjiModeCommand::ImeOn,
        // `CancelAndIMEOff`（BUG-115、ATOKプリセットの`Precomposition`状態）
        // は「進行中の入力をキャンセルしてIMEを閉じる」意味であり、IME開閉の
        // 観点では`IMEOff`と同義として扱う（`keymap.rs::group_ime_rows_by_key`
        // と一貫させる）。
        "IMEOff" | "CancelAndIMEOff" => GjiModeCommand::ImeOff,
        "CompositionModeHiragana" => GjiModeCommand::SetMode(GjiCompositionMode::Hiragana),
        "CompositionModeFullKatakana" => GjiModeCommand::SetMode(GjiCompositionMode::FullKatakana),
        "CompositionModeHalfKatakana" => GjiModeCommand::SetMode(GjiCompositionMode::HalfKatakana),
        "CompositionModeFullAlphanumeric" => {
            GjiModeCommand::SetMode(GjiCompositionMode::FullAlphanumeric)
        }
        "CompositionModeHalfAlphanumeric" => {
            GjiModeCommand::SetMode(GjiCompositionMode::HalfAlphanumeric)
        }
        "ToggleAlphanumericMode" => GjiModeCommand::ToggleAlphanumericMode,
        "SwitchKanaType" | "CompositionModeSwitchKanaType" => GjiModeCommand::ToggleKanaType,
        _ => GjiModeCommand::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_command, GjiCompositionMode, GjiModeCommand};

    #[test]
    fn recognizes_ime_on_off() {
        assert_eq!(classify_command("IMEOn"), GjiModeCommand::ImeOn);
        assert_eq!(classify_command("IMEOff"), GjiModeCommand::ImeOff);
    }

    /// BUG-115: ATOKプリセットの`Precomposition`状態がHenkan/Muhenkanに
    /// 割り当てる`CancelAndIMEOff`も`ImeOff`相当として分類されること。
    #[test]
    fn recognizes_cancel_and_ime_off_as_ime_off() {
        assert_eq!(classify_command("CancelAndIMEOff"), GjiModeCommand::ImeOff);
    }

    #[test]
    fn recognizes_absolute_mode_set_commands() {
        assert_eq!(
            classify_command("CompositionModeHiragana"),
            GjiModeCommand::SetMode(GjiCompositionMode::Hiragana)
        );
        assert_eq!(
            classify_command("CompositionModeFullKatakana"),
            GjiModeCommand::SetMode(GjiCompositionMode::FullKatakana)
        );
        assert_eq!(
            classify_command("CompositionModeHalfKatakana"),
            GjiModeCommand::SetMode(GjiCompositionMode::HalfKatakana)
        );
        assert_eq!(
            classify_command("CompositionModeFullAlphanumeric"),
            GjiModeCommand::SetMode(GjiCompositionMode::FullAlphanumeric)
        );
        assert_eq!(
            classify_command("CompositionModeHalfAlphanumeric"),
            GjiModeCommand::SetMode(GjiCompositionMode::HalfAlphanumeric)
        );
    }

    #[test]
    fn recognizes_relative_toggle_commands() {
        assert_eq!(
            classify_command("ToggleAlphanumericMode"),
            GjiModeCommand::ToggleAlphanumericMode
        );
        assert_eq!(
            classify_command("SwitchKanaType"),
            GjiModeCommand::ToggleKanaType
        );
        assert_eq!(
            classify_command("CompositionModeSwitchKanaType"),
            GjiModeCommand::ToggleKanaType
        );
    }

    #[test]
    fn unrelated_or_unknown_commands_map_to_other() {
        for command in [
            "Backspace",
            "Convert",
            "Commit",
            "ConvertToFullKatakana",
            "",
            "SomeFutureMozcCommand",
        ] {
            assert_eq!(classify_command(command), GjiModeCommand::Other);
        }
    }
}
