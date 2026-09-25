//! ユーザーの GJI キー設定から、キーの役割（IME ON/OFF トグルか）を逆算する
//! （ADR-199 決定4・決定11・決定13）。
//!
//! 入力は `config1.db` の生の `session_keymap` と `custom_keymap_table`、出力は
//! [`KeyRole`]（役割が無い＝受動は `None`）。学習表による狭め（決定6-2）・明示 config
//! との重なり（決定8）・IME 種別による分岐は呼び出し側の責務。
//!
//! - プリセット（ATOK/MSIME/KOTOERI/MOBILE）は GUI から中身を変えられないので定数表
//!   （[`Preset::toggle_vk_names`]、テストで Mozc の TSV と突き合わせて固定）。
//! - CUSTOM だけを状態表で評価する。状態の継承（Suggestion→Composition、
//!   Prediction→Conversion、ZeroQuerySuggestion→Precomposition）と「同じ状態・同じキーの
//!   行は後勝ち」は Mozc `session/keymap.cc`（`GetCommandSuggestion` 等・
//!   `KeyMap::AddRule`）に揃える。
//! - 判別できない値（`OVERLAY_*`・`CHROMEOS`・未知の値）は受動（決定6-3「不明なときに
//!   能動側へ倒さない」）。

use crate::command::{classify_command, GjiModeCommand};
use crate::keymap::mozc_key_vk_names;
use crate::tsv::{parse_custom_keymap_table, KeymapRow};
use crate::{
    SESSION_KEYMAP_ATOK, SESSION_KEYMAP_CUSTOM, SESSION_KEYMAP_KOTOERI, SESSION_KEYMAP_MOBILE,
    SESSION_KEYMAP_MSIME, SESSION_KEYMAP_NONE,
};

/// キーの役割（ADR-199 決定2）。役割が無いキー（受動）は `Option::None` で表す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRole {
    /// IME ON/OFF トグル: 閉状態（DirectInput）で押すと開き、全ての開状態で押すと閉じる。
    ImeToggle,
}

/// 役割判定の候補キー（無修飾で評価する、決定4・決定18・決定16）の VK 名:
/// 半角/全角（0xF3/0xF4）・F13〜F24・無変換/変換。これ以外のキーは常に受動。
pub const ROLE_CANDIDATE_VK_NAMES: &[&str] = &[
    "VK_DBE_SBCSCHAR",
    "VK_DBE_DBCSCHAR",
    "VK_F13",
    "VK_F14",
    "VK_F15",
    "VK_F16",
    "VK_F17",
    "VK_F18",
    "VK_F19",
    "VK_F20",
    "VK_F21",
    "VK_F22",
    "VK_F23",
    "VK_F24",
    "VK_NONCONVERT",
    "VK_CONVERT",
];

/// 中身が固定の GJI プリセット。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preset {
    Atok,
    MsIme,
    Kotoeri,
    Mobile,
}

impl Preset {
    /// 候補キーのうち、このプリセットでトグルの役割を持つもの。4種とも半角/全角だけ
    /// （ATOK の変換/無変換は Composition で `Convert`/`ToggleAlphanumericMode` なので
    /// トグルでない、MS-IME/MOBILE の F13 は DirectInput 行しかない）。Mozc の TSV との
    /// 突き合わせはテスト `preset_table_matches_mozc_tsv`。
    const fn toggle_vk_names(self) -> &'static [&'static str] {
        match self {
            Self::Atok | Self::MsIme | Self::Kotoeri | Self::Mobile => {
                &["VK_DBE_SBCSCHAR", "VK_DBE_DBCSCHAR"]
            }
        }
    }
}

/// 役割を何から決めるか（決定4 の判別）。
enum Source<'a> {
    Preset(Preset),
    Custom(&'a str),
    Unknown,
}

/// 生の `session_keymap` から判別する。`preset` の流用は不可（ATOK/MSIME 以外を1つに
/// まとめるので、KOTOERI/MOBILE で使われていない古い `custom_keymap_table` を評価してしまう）。
const fn source(session_keymap: Option<i64>, custom_keymap_table: Option<&str>) -> Source<'_> {
    match (session_keymap, custom_keymap_table) {
        (Some(SESSION_KEYMAP_CUSTOM), Some(table)) if !table.is_empty() => Source::Custom(table),
        // フィールド無し・NONE・表が空または無い CUSTOM は、Mozc が既定（Windows では
        // MSIME）の TSV を読む（`keymap.cc` `ApplyPrimarySessionKeymap`）。
        (None | Some(SESSION_KEYMAP_NONE | SESSION_KEYMAP_CUSTOM | SESSION_KEYMAP_MSIME), _) => {
            Source::Preset(Preset::MsIme)
        }
        (Some(SESSION_KEYMAP_ATOK), _) => Source::Preset(Preset::Atok),
        (Some(SESSION_KEYMAP_KOTOERI), _) => Source::Preset(Preset::Kotoeri),
        (Some(SESSION_KEYMAP_MOBILE), _) => Source::Preset(Preset::Mobile),
        (Some(_), _) => Source::Unknown,
    }
}

/// `vk_name`（無修飾の打鍵）が、この GJI 設定で持つ役割（ADR-199 決定4）。
/// 候補外のキー（[`ROLE_CANDIDATE_VK_NAMES`] に無い）は常に `None`。
#[must_use]
pub fn key_role(
    session_keymap: Option<i64>,
    custom_keymap_table: Option<&str>,
    vk_name: &str,
) -> Option<KeyRole> {
    if !ROLE_CANDIDATE_VK_NAMES.contains(&vk_name) {
        return None;
    }
    let toggle = match source(session_keymap, custom_keymap_table) {
        Source::Preset(preset) => preset.toggle_vk_names().contains(&vk_name),
        Source::Custom(table) => custom_table_has_toggle(table, vk_name),
        Source::Unknown => false,
    };
    toggle.then_some(KeyRole::ImeToggle)
}

/// CUSTOM の表で `vk_name` がトグルか。awase の書き込み手段（`VK_IME_ON`/`VK_IME_OFF`、
/// Mozc の `ON`/`OFF` 行）が効くことも要求する（決定4）: 欠けた表でトグルと判定すると、
/// Suppress したうえで送る冪等キーを GJI が無視し、誰も開閉しない。
fn custom_table_has_toggle(table: &str, vk_name: &str) -> bool {
    let rows = parse_custom_keymap_table(table);
    let key = KeyStates::of(&rows, vk_name);
    let on = KeyStates::of(&rows, "VK_IME_ON");
    let off = KeyStates::of(&rows, "VK_IME_OFF");
    key.opens_when_closed()
        && key.closes_in_all_open_states()
        && on.opens_when_closed()
        && off.closes_in_all_open_states()
}

/// Mozc の入力状態（`custom_keymap_table` の `status` 列）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    DirectInput,
    Precomposition,
    Composition,
    Conversion,
    ZeroQuerySuggestion,
    Suggestion,
    Prediction,
}

impl Status {
    /// 開状態（決定11: 全ての開状態で閉じるキーだけがトグル）。
    const OPEN: [Self; 6] = [
        Self::Precomposition,
        Self::Composition,
        Self::Conversion,
        Self::ZeroQuerySuggestion,
        Self::Suggestion,
        Self::Prediction,
    ];

    fn parse(status: &str) -> Option<Self> {
        match status {
            // `Direct` は Mozc `AddCommand` が受け付ける別名。
            "DirectInput" | "Direct" => Some(Self::DirectInput),
            "Precomposition" => Some(Self::Precomposition),
            "Composition" => Some(Self::Composition),
            "Conversion" => Some(Self::Conversion),
            "ZeroQuerySuggestion" => Some(Self::ZeroQuerySuggestion),
            "Suggestion" => Some(Self::Suggestion),
            "Prediction" => Some(Self::Prediction),
            _ => None,
        }
    }

    /// その状態の行が無いときに使われる継承元（Mozc `keymap.cc` `GetCommandSuggestion` 等）。
    const fn inherits_from(self) -> Option<Self> {
        match self {
            Self::Suggestion => Some(Self::Composition),
            Self::Prediction => Some(Self::Conversion),
            Self::ZeroQuerySuggestion => Some(Self::Precomposition),
            _ => None,
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// コマンドの3類（決定4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Effect {
    /// 閉状態から開く（DirectInput の `IMEOn`）。DirectInput の `CompositionMode*` は
    /// 実機で開くと確認できるまで数えない（T1(c)、決定13）。
    Open,
    /// 開状態から閉じる（`IMEOff`・`CancelAndIMEOff`）。
    Close,
    /// それ以外（未知のコマンドを含む）。
    Other,
}

impl Effect {
    fn of(status: Status, command: &str) -> Self {
        match (status, classify_command(command)) {
            (Status::DirectInput, GjiModeCommand::ImeOn) => Self::Open,
            (Status::DirectInput, _) => Self::Other,
            (_, GjiModeCommand::ImeOff) => Self::Close,
            _ => Self::Other,
        }
    }
}

/// 1キー分の状態表（状態ごとの、そのキーの行のコマンドの類。行が無ければ `None`）。
struct KeyStates([Option<Effect>; 7]);

impl KeyStates {
    /// `vk_name` を生むキー名の無修飾の行だけを、表の順に読む（後勝ち）。キー名の別名
    /// （`Hankaku`/`Zenkaku`/`Hankaku/Zenkaku` 等）は Mozc と同じく同じキーになる。
    fn of(rows: &[KeymapRow], vk_name: &str) -> Self {
        let mut states = [None; 7];
        for row in rows {
            if !mozc_key_vk_names(&row.key).contains(&vk_name) {
                continue;
            }
            if let Some(status) = Status::parse(&row.status) {
                states[status.index()] = Some(Effect::of(status, &row.command));
            }
        }
        Self(states)
    }

    /// 継承規則を適用した実効のコマンドの類。
    fn effective(&self, status: Status) -> Option<Effect> {
        self.0[status.index()].or_else(|| {
            status
                .inherits_from()
                .and_then(|parent| self.0[parent.index()])
        })
    }

    fn opens_when_closed(&self) -> bool {
        self.effective(Status::DirectInput) == Some(Effect::Open)
    }

    fn closes_in_all_open_states(&self) -> bool {
        Status::OPEN
            .iter()
            .all(|&status| self.effective(status) == Some(Effect::Close))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 書き込み手段（`ON`/`OFF` 行）を揃えた CUSTOM 表の雛形に、キーの行を足す。
    fn custom(rows: &str) -> String {
        format!(
            "status\tkey\tcommand\n\
             DirectInput\tON\tIMEOn\n\
             Precomposition\tOFF\tIMEOff\n\
             Composition\tOFF\tIMEOff\n\
             Conversion\tOFF\tIMEOff\n\
             {rows}"
        )
    }

    /// 開閉トグル形の4行（DirectInput=IMEOn、Precomposition/Composition/Conversion=IMEOff）。
    fn toggle_rows(key: &str) -> String {
        format!(
            "DirectInput\t{key}\tIMEOn\n\
             Precomposition\t{key}\tIMEOff\n\
             Composition\t{key}\tIMEOff\n\
             Conversion\t{key}\tIMEOff\n"
        )
    }

    fn role_in_custom(table: &str, vk_name: &str) -> Option<KeyRole> {
        key_role(Some(SESSION_KEYMAP_CUSTOM), Some(table), vk_name)
    }

    const HZ: [&str; 2] = ["VK_DBE_SBCSCHAR", "VK_DBE_DBCSCHAR"];

    // ---- プリセット定数表と Mozc TSV の突き合わせ ----
    //
    // 以下は google/mozc master の `src/data/keymap/{ms-ime,atok,kotoeri,mobile}.tsv`
    // （2026-09-25 取得）から、候補キー（Hankaku/Zenkaku・F13〜F24・Henkan・Muhenkan）と
    // `Kanji`・`ON`・`OFF` の行だけを抜き出したもの（該当キーの Suggestion/Prediction/
    // ZeroQuerySuggestion 行は4種とも無い）。TSV 本体は同梱しない（決定4）。

    const MS_IME_TSV: &str = "status\tkey\tcommand
Composition\tHankaku/Zenkaku\tIMEOff
Composition\tHenkan\tConvert
Composition\tKanji\tIMEOff
Composition\tMuhenkan\tSwitchKanaType
Composition\tOFF\tIMEOff
Composition\tON\tIMEOn
Conversion\tHankaku/Zenkaku\tIMEOff
Conversion\tHenkan\tConvertNext
Conversion\tKanji\tIMEOff
Conversion\tMuhenkan\tSwitchKanaType
Conversion\tOFF\tIMEOff
Conversion\tON\tIMEOn
DirectInput\tF13\tIMEOn
DirectInput\tHankaku/Zenkaku\tIMEOn
DirectInput\tHenkan\tReconvert
DirectInput\tKanji\tIMEOn
DirectInput\tON\tIMEOn
Precomposition\tHankaku/Zenkaku\tIMEOff
Precomposition\tHenkan\tReconvert
Precomposition\tKanji\tIMEOff
Precomposition\tMuhenkan\tCompositionModeSwitchKanaType
Precomposition\tOFF\tIMEOff
Precomposition\tON\tIMEOn
";

    const ATOK_TSV: &str = "status\tkey\tcommand
Composition\tHankaku/Zenkaku\tCancelAndIMEOff
Composition\tHenkan\tConvert
Composition\tKanji\tCancelAndIMEOff
Composition\tMuhenkan\tToggleAlphanumericMode
Composition\tOFF\tCancelAndIMEOff
Composition\tON\tIMEOn
Conversion\tHankaku/Zenkaku\tCancelAndIMEOff
Conversion\tHenkan\tConvertNextPage
Conversion\tKanji\tCancelAndIMEOff
Conversion\tOFF\tCancelAndIMEOff
Conversion\tON\tIMEOn
DirectInput\tHankaku/Zenkaku\tIMEOn
DirectInput\tHenkan\tIMEOn
DirectInput\tKanji\tIMEOn
DirectInput\tMuhenkan\tIMEOn
DirectInput\tON\tIMEOn
Precomposition\tHankaku/Zenkaku\tCancelAndIMEOff
Precomposition\tHenkan\tCancelAndIMEOff
Precomposition\tKanji\tCancelAndIMEOff
Precomposition\tMuhenkan\tCancelAndIMEOff
Precomposition\tOFF\tCancelAndIMEOff
Precomposition\tON\tIMEOn
";

    const KOTOERI_TSV: &str = "status\tkey\tcommand
Composition\tHankaku/Zenkaku\tIMEOff
Composition\tKanji\tIMEOff
Composition\tOFF\tIMEOff
Composition\tON\tIMEOn
Conversion\tHankaku/Zenkaku\tIMEOff
Conversion\tKanji\tIMEOff
Conversion\tOFF\tIMEOff
Conversion\tON\tIMEOn
DirectInput\tHankaku/Zenkaku\tIMEOn
DirectInput\tKanji\tIMEOn
DirectInput\tON\tIMEOn
Precomposition\tHankaku/Zenkaku\tIMEOff
Precomposition\tKanji\tIMEOff
Precomposition\tOFF\tIMEOff
Precomposition\tON\tIMEOn
";

    /// `mobile.tsv` の該当行は `ms-ime.tsv` と同一（取得時に diff で確認）。
    const MOBILE_TSV: &str = MS_IME_TSV;

    #[test]
    fn preset_table_matches_mozc_tsv() {
        for (preset, tsv) in [
            (Preset::MsIme, MS_IME_TSV),
            (Preset::Atok, ATOK_TSV),
            (Preset::Kotoeri, KOTOERI_TSV),
            (Preset::Mobile, MOBILE_TSV),
        ] {
            for vk_name in ROLE_CANDIDATE_VK_NAMES {
                assert_eq!(
                    custom_table_has_toggle(tsv, vk_name),
                    preset.toggle_vk_names().contains(vk_name),
                    "{preset:?} {vk_name}"
                );
            }
        }
    }

    #[test]
    fn presets_are_dispatched_by_raw_session_keymap() {
        for session in [
            SESSION_KEYMAP_ATOK,
            SESSION_KEYMAP_MSIME,
            SESSION_KEYMAP_KOTOERI,
            SESSION_KEYMAP_MOBILE,
        ] {
            for vk_name in HZ {
                assert_eq!(
                    key_role(Some(session), None, vk_name),
                    Some(KeyRole::ImeToggle)
                );
            }
            // ATOK の変換/無変換（Composition で Convert/ToggleAlphanumericMode）・
            // MS-IME の F13（DirectInput 行のみ）は受動。
            for vk_name in ["VK_CONVERT", "VK_NONCONVERT", "VK_F13"] {
                assert_eq!(key_role(Some(session), None, vk_name), None);
            }
        }
    }

    #[test]
    fn kotoeri_or_mobile_ignores_a_stale_custom_table() {
        // 半角/全角を別機能にした古い custom 表が残っていても、プリセットの定数表を使う。
        let stale = custom("DirectInput\tHankaku/Zenkaku\tCompositionModeHiragana\n");
        assert_eq!(role_in_custom(&stale, HZ[0]), None);
        for session in [SESSION_KEYMAP_KOTOERI, SESSION_KEYMAP_MOBILE] {
            assert_eq!(
                key_role(Some(session), Some(&stale), HZ[0]),
                Some(KeyRole::ImeToggle)
            );
        }
    }

    #[test]
    fn default_keymap_cases_use_the_msime_table() {
        let empty = "";
        for (session, table) in [
            (None, None),
            (Some(SESSION_KEYMAP_NONE), None),
            (Some(SESSION_KEYMAP_CUSTOM), None),
            (Some(SESSION_KEYMAP_CUSTOM), Some(empty)),
        ] {
            assert_eq!(
                key_role(session, table, HZ[1]),
                Some(KeyRole::ImeToggle),
                "{session:?} {table:?}"
            );
        }
    }

    #[test]
    fn unknown_session_keymap_is_passive() {
        // CHROMEOS(5)・OVERLAY_HENKAN_MUHENKAN_TO_IME_ON_OFF(100)・未知の値。
        for session in [5, 100, 999] {
            assert_eq!(key_role(Some(session), None, HZ[0]), None);
        }
    }

    // ---- CUSTOM の状態表 ----

    #[test]
    fn custom_toggle_on_hankaku_zenkaku_and_its_aliases() {
        let table = custom(&toggle_rows("Hankaku/Zenkaku"));
        for vk_name in HZ {
            assert_eq!(role_in_custom(&table, vk_name), Some(KeyRole::ImeToggle));
        }
        // 別名・大文字小文字の違いも同じキー（Mozc key_parser）。
        let alias = custom(&toggle_rows("zenkaku"));
        assert_eq!(role_in_custom(&alias, HZ[0]), Some(KeyRole::ImeToggle));
    }

    #[test]
    fn custom_hankaku_zenkaku_opening_with_a_mode_is_passive_until_t1c() {
        // 決定13: DirectInput の CompositionMode* は実機確認（T1(c)）まで Open に数えない。
        let table = custom(
            "DirectInput\tHankaku/Zenkaku\tCompositionModeHiragana\n\
             Precomposition\tHankaku/Zenkaku\tIMEOff\n\
             Composition\tHankaku/Zenkaku\tIMEOff\n\
             Conversion\tHankaku/Zenkaku\tIMEOff\n",
        );
        assert_eq!(role_in_custom(&table, HZ[0]), None);
    }

    #[test]
    fn suggestion_inherits_composition_but_its_own_row_wins() {
        // Suggestion 行が無い → Composition の IMEOff を継承してトグル（上のテスト）。
        // Suggestion にだけ別コマンド → トグルでない（決定11）。
        let table = custom(&format!(
            "{}Suggestion\tHankaku/Zenkaku\tCommit\n",
            toggle_rows("Hankaku/Zenkaku")
        ));
        assert_eq!(role_in_custom(&table, HZ[0]), None);
        // ZeroQuerySuggestion（Precomposition を継承）・Prediction（Conversion を継承）も同じ。
        for state in ["ZeroQuerySuggestion", "Prediction"] {
            let table = custom(&format!("{}{state}\tF13\tCommit\n", toggle_rows("F13")));
            assert_eq!(role_in_custom(&table, "VK_F13"), None, "{state}");
        }
    }

    #[test]
    fn missing_open_state_row_is_passive() {
        let table = custom(
            "DirectInput\tHankaku/Zenkaku\tIMEOn\n\
             Precomposition\tHankaku/Zenkaku\tIMEOff\n\
             Conversion\tHankaku/Zenkaku\tIMEOff\n",
        );
        assert_eq!(role_in_custom(&table, HZ[0]), None);
    }

    #[test]
    fn later_row_for_the_same_state_wins() {
        let table = custom(&format!(
            "{}DirectInput\tHankaku\tReconvert\n",
            toggle_rows("Hankaku/Zenkaku")
        ));
        assert_eq!(role_in_custom(&table, HZ[0]), None);
        // `Direct` は DirectInput の別名。
        let table = custom(&format!(
            "DirectInput\tF14\tReconvert\n{}",
            toggle_rows("F14").replace("DirectInput", "Direct")
        ));
        assert_eq!(role_in_custom(&table, "VK_F14"), Some(KeyRole::ImeToggle));
    }

    #[test]
    fn custom_without_on_off_rows_is_passive() {
        // awase が送る VK_IME_ON/OFF を GJI が無視する表では、トグル形でも受動（決定4）。
        let bare = format!("status\tkey\tcommand\n{}", toggle_rows("Hankaku/Zenkaku"));
        assert_eq!(role_in_custom(&bare, HZ[0]), None);
        let off_missing_in_conversion = format!(
            "status\tkey\tcommand\nDirectInput\tON\tIMEOn\nPrecomposition\tOFF\tIMEOff\n\
             Composition\tOFF\tIMEOff\n{}",
            toggle_rows("Hankaku/Zenkaku")
        );
        assert_eq!(role_in_custom(&off_missing_in_conversion, HZ[0]), None);
    }

    #[test]
    fn kanji_rows_are_not_mapped_to_any_candidate() {
        // Hankaku/Zenkaku を別機能にし Kanji 行だけトグル形 → 半角/全角は受動。
        let table = custom(&format!(
            "{}DirectInput\tHankaku/Zenkaku\tReconvert\n",
            toggle_rows("Kanji")
        ));
        assert_eq!(role_in_custom(&table, HZ[0]), None);
        assert_eq!(role_in_custom(&table, "VK_KANJI"), None);
    }

    #[test]
    fn modified_rows_are_out_of_scope() {
        let table = custom(&toggle_rows("Ctrl F13"));
        assert_eq!(role_in_custom(&table, "VK_F13"), None);
        let table = custom(&toggle_rows("Shift Hankaku/Zenkaku"));
        assert_eq!(role_in_custom(&table, HZ[0]), None);
    }

    #[test]
    fn non_candidate_keys_are_passive_even_when_toggle_shaped() {
        for (key, vk_name) in [
            ("Eisu", "VK_DBE_ALPHANUMERIC"),
            ("Katakana", "VK_DBE_KATAKANA"),
            ("Hiragana", "VK_DBE_HIRAGANA"),
            ("F1", "VK_F1"),
            ("F12", "VK_F12"),
            ("Kanji", "VK_KANJI"),
        ] {
            let table = custom(&toggle_rows(key));
            assert_eq!(role_in_custom(&table, vk_name), None, "{key}");
        }
    }

    #[test]
    fn f13_to_f24_in_custom() {
        for n in 13..=24 {
            let key = format!("F{n}");
            let table = custom(&toggle_rows(&key));
            assert_eq!(
                role_in_custom(&table, &format!("VK_{key}")),
                Some(KeyRole::ImeToggle)
            );
        }
        // BUG-64 型の残骸（F21=IMEOn だけ・F22=IMEOff だけ、方向固定の別キー）はどちらも受動。
        let leftover = custom(
            "DirectInput\tF21\tIMEOn\nPrecomposition\tF21\tIMEOn\n\
             Precomposition\tF22\tIMEOff\nComposition\tF22\tIMEOff\nConversion\tF22\tIMEOff\n",
        );
        assert_eq!(role_in_custom(&leftover, "VK_F21"), None);
        assert_eq!(role_in_custom(&leftover, "VK_F22"), None);
        // MS-IME プリセットをコピーした表の F13（DirectInput 行のみ）も受動。
        assert_eq!(role_in_custom(MS_IME_TSV, "VK_F13"), None);
    }

    #[test]
    fn henkan_muhenkan_in_custom() {
        let table = custom(&format!(
            "{}{}",
            toggle_rows("Henkan"),
            toggle_rows("Muhenkan")
        ));
        assert_eq!(
            role_in_custom(&table, "VK_CONVERT"),
            Some(KeyRole::ImeToggle)
        );
        assert_eq!(
            role_in_custom(&table, "VK_NONCONVERT"),
            Some(KeyRole::ImeToggle)
        );
        // ATOK プリセットをコピーした表（Composition で Convert）は受動。
        assert_eq!(role_in_custom(ATOK_TSV, "VK_CONVERT"), None);
        assert_eq!(role_in_custom(ATOK_TSV, "VK_NONCONVERT"), None);
        // Precomposition だけ IMEOff（入力中は閉じない）→ 受動（決定11）。
        let precomp_only =
            custom("DirectInput\tMuhenkan\tIMEOn\nPrecomposition\tMuhenkan\tCancelAndIMEOff\n");
        assert_eq!(role_in_custom(&precomp_only, "VK_NONCONVERT"), None);
    }
}
