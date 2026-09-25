//! GJI のキー表記を awase の VK 名に変換する。
//!
//! Mozc `key_parser` 由来のトークン（例: `"F21"`, `"Hankaku/Zenkaku"`）を、
//! awase の VK 名（`VkCode::from_name` が受理する文字列。例: `"VK_F21"`,
//! `"VK_DBE_SBCSCHAR"`）へ変換し（[`mozc_key_vk_names`]）、GJI の
//! `custom_keymap_table` から IME ON/OFF に使われているキーの集合を抽出する。
//!
//! スコープ（stage 1）: 修飾キー付きの行（`"Ctrl Shift Insert"` 等）は
//! 対象外（ログのみ、無視）。awase 側の `ime_detect.on/off/toggle`
//! （`VkCode::from_name` 直読み、修飾キー非対応）にそのまま乗せられる
//! 単発キーのみを扱う。

use std::collections::{BTreeMap, BTreeSet};

use crate::command::{classify_command, GjiCompositionMode, GjiModeCommand};
use crate::tsv::{parse_custom_keymap_table, KeymapRow};

/// GJI の `IMEOn`/`IMEOff` コマンドが割り当てられている、GJI 内部の入力状態が
/// 「IME は実質 OFF（未起動）」を表すもの。この状態群でのみ `IMEOn` が割り当て
/// られているキーは「ONトリガー」の候補になる。
const STATUSES_WHEN_IME_OFF: &[&str] = &["DirectInput"];

/// 「IME は実質 ON（起動済み、入力前後を問わない）」を表す状態群。
/// この状態群でのみ `IMEOff` が割り当てられているキーは「OFFトリガー」の候補
/// になる。
const STATUSES_WHEN_IME_ON: &[&str] = &[
    "Precomposition",
    "Composition",
    "Conversion",
    "Prediction",
    "Suggestion",
];

/// `F1`..`F24` の VK 名（[`mozc_key_vk_names`] が `'static` のスライスを返すための表）。
const F_KEY_VK_NAMES: [&str; 24] = [
    "VK_F1", "VK_F2", "VK_F3", "VK_F4", "VK_F5", "VK_F6", "VK_F7", "VK_F8", "VK_F9", "VK_F10",
    "VK_F11", "VK_F12", "VK_F13", "VK_F14", "VK_F15", "VK_F16", "VK_F17", "VK_F18", "VK_F19",
    "VK_F20", "VK_F21", "VK_F22", "VK_F23", "VK_F24",
];

/// Mozc のキー名（`custom_keymap_table` の `key` 列）を、Windows でそのキーイベントを
/// 生む VK の名前（`VkCode::from_name` が受理する文字列）へ写す。
///
/// キー名→VK の写像はここ1箇所に置き、GJI 設定の抽出・役割判定（[`crate::role`]）・awase-windows の予測
/// （`key_effect_predictor::custom_table_overrides`）が共有する（ADR-199 T2）。
///
/// Mozc と同じく大文字小文字を区別せず、前後の空白は無視する（`key_parser.cc`
/// `ParseKey`）。空白で区切られた複数トークン（修飾キー付き、例: `"Ctrl Space"`）・
/// 未知のキー名は空スライス（安全側に倒し、未知の VK を使わない）。
///
/// - `Hankaku/Zenkaku`（別名 `Hankaku`・`Zenkaku`）は 0xF3/0xF4 の両方。Mozc は
///   `VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR` を同じ `KeyEvent::HANKAKU` に畳む
///   （`win32/base/keyevent_handler.cc`）。
/// - `Kanji` はどの VK にも写さない。Windows では `VK_KANJI`(0x19) が
///   `KeyEvent::KANJI` にならない（IMM32 では `NO_SPECIALKEY`、TSF では
///   `HANKAKU` に畳まれる。同ファイル L87-93）ので、プリセットの `Kanji` 行は
///   使われない行である（ADR-199 背景3）。
/// - `ON`/`OFF` は `VK_IME_ON`/`VK_IME_OFF`（Mozc は `KeyEvent::ON`/`OFF` に写す）。
/// - `Kana`/`Hiragana` はどちらも `KeyEvent::KANA`（`VK_DBE_HIRAGANA`）。
#[must_use]
pub fn mozc_key_vk_names(key: &str) -> &'static [&'static str] {
    let key = key.trim().to_ascii_lowercase();
    match key.as_str() {
        "hankaku" | "zenkaku" | "hankaku/zenkaku" => &["VK_DBE_SBCSCHAR", "VK_DBE_DBCSCHAR"],
        "on" => &["VK_IME_ON"],
        "off" => &["VK_IME_OFF"],
        "eisu" => &["VK_DBE_ALPHANUMERIC"],
        "henkan" => &["VK_CONVERT"],
        "muhenkan" => &["VK_NONCONVERT"],
        "kana" | "hiragana" => &["VK_DBE_HIRAGANA"],
        "katakana" => &["VK_DBE_KATAKANA"],
        "bs" | "backspace" => &["VK_BACK"],
        "enter" | "return" => &["VK_RETURN"],
        "esc" | "escape" => &["VK_ESCAPE"],
        "space" => &["VK_SPACE"],
        other => other
            .strip_prefix('f')
            .filter(|digits| !digits.starts_with('0'))
            .and_then(|digits| digits.parse::<usize>().ok())
            .filter(|n| (1..=24).contains(n))
            .map_or(&[], |n| std::slice::from_ref(&F_KEY_VK_NAMES[n - 1])),
    }
}

/// GJI の custom keymap から抽出した、IME ON/OFF 検出に使える VK 名の集合。
/// いずれも `awase::config::ImeDetectConfig.toggle/on/off` へそのまま
/// 反映できる形（`VkCode::from_name` が受理する文字列）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GjiImeKeys {
    /// このキー単独で IME を ON にする（`ImeDetectConfig.on` 相当）。
    pub on: Vec<String>,
    /// このキー単独で IME を OFF にする（`ImeDetectConfig.off` 相当）。
    pub off: Vec<String>,
    /// このキーで ON/OFF がトグルする（`ImeDetectConfig.toggle` 相当）。
    pub toggle: Vec<String>,
}

/// GJI の custom keymap から抽出した、入力モード変更に使われている VK 名の分類
/// （[`crate::command::classify_command`] 参照）。`GjiImeKeys` と異なり、
/// awase 側の belief 追随に使う想定のためモードの種類ごとに分けてある。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GjiModeKeys {
    /// このキーで特定の [`GjiCompositionMode`] へ絶対設定される
    /// （現在のモードに依らず遷移先が一意）。
    pub set_mode: Vec<(String, GjiCompositionMode)>,
    /// このキーでかな⇔英数がトグルする（遷移先は現在モード依存）。
    pub toggle_alphanumeric: Vec<String>,
    /// このキーでカナ種別（ひらがな/全角カナ/半角カナ）が順送りされる
    /// （遷移先は現在モード依存）。
    pub toggle_kana_type: Vec<String>,
}

/// キーごとに集計した「IMEOn が割り当てられている状態の集合」と
/// 「IMEOff が割り当てられている状態の集合」。
type StatusSetsByKey = BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)>;

/// `custom_keymap_table` の TSV 文字列から [`GjiImeKeys`] を構築する。
///
/// 手順:
/// 1. `command` が `IMEOn`/`IMEOff` の行だけを残す。
/// 2. `key` に空白を含む行（修飾キー付き）は stage 1 のスコープ外として除外
///    （`tracing::debug!` のみ、エラーにはしない）。
/// 3. 残った行をキー単位で集約し、[`STATUSES_WHEN_IME_OFF`]/
///    [`STATUSES_WHEN_IME_ON`] に基づいて toggle/on/off/矛盾（除外）に分類する。
#[must_use]
pub fn extract_ime_keys(custom_keymap_table: &str) -> GjiImeKeys {
    let rows = parse_custom_keymap_table(custom_keymap_table);
    let grouped = group_ime_rows_by_key(&rows);

    let mut result = GjiImeKeys::default();
    for (key, (on_statuses, off_statuses)) in grouped {
        let vk_names = mozc_key_vk_names(&key);
        if vk_names.is_empty() {
            tracing::debug!("gji-config: VK に写らないキートークンをスキップしました: key={key}");
        }
        for vk_name in vk_names {
            classify_and_push(&key, vk_name, &on_statuses, &off_statuses, &mut result);
        }
    }

    result.on.sort_unstable();
    result.on.dedup();
    result.off.sort_unstable();
    result.off.dedup();
    result.toggle.sort_unstable();
    result.toggle.dedup();
    result
}

/// `custom_keymap_table` の TSV 文字列から [`GjiModeKeys`] を構築する。
///
/// 手順:
/// 1. 各行の `command` を [`classify_command`] で分類し、`SetMode`/
///    `ToggleAlphanumericMode`/`ToggleKanaType` のいずれかの行だけを残す
///    （`IMEOn`/`IMEOff`/`Other` は無視。前者2つは [`extract_ime_keys`] の
///    担当）。
/// 2. `key` に空白を含む行（修飾キー付き）は stage 1 と同様スコープ外
///    として除外する。
/// 3. 同じキーに複数の異なる分類（例: 状態によって `SetMode(Hiragana)` と
///    `ToggleAlphanumericMode` の両方）が付いている場合は、遷移先が
///    一意に定まらないため警告ログのみで取り込まない。
#[must_use]
pub fn extract_mode_keys(custom_keymap_table: &str) -> GjiModeKeys {
    let rows = parse_custom_keymap_table(custom_keymap_table);
    let mut by_key: BTreeMap<String, BTreeSet<GjiModeCommand>> = BTreeMap::new();
    for row in &rows {
        let classified = classify_command(&row.command);
        if matches!(
            classified,
            GjiModeCommand::ImeOn | GjiModeCommand::ImeOff | GjiModeCommand::Other
        ) {
            continue;
        }
        if row.key.contains(char::is_whitespace) {
            tracing::debug!(
                "gji-config: 修飾キー付き行は stage 1 のスコープ外のため無視: key={}",
                row.key
            );
            continue;
        }
        by_key
            .entry(row.key.clone())
            .or_default()
            .insert(classified);
    }

    let mut result = GjiModeKeys::default();
    for (key, commands) in by_key {
        let vk_names = mozc_key_vk_names(&key);
        if vk_names.is_empty() {
            tracing::debug!("gji-config: VK に写らないキートークンをスキップしました: key={key}");
            continue;
        }
        let mut distinct = commands.into_iter();
        let Some(only) = distinct.next() else {
            continue;
        };
        if distinct.next().is_some() {
            tracing::warn!(
                "gji-config: 状態間で遷移先が一意に定まらないためスキップしました: key={key}"
            );
            continue;
        }
        for vk_name in vk_names.iter().map(|name| (*name).to_string()) {
            match only {
                GjiModeCommand::SetMode(mode) => result.set_mode.push((vk_name, mode)),
                GjiModeCommand::ToggleAlphanumericMode => result.toggle_alphanumeric.push(vk_name),
                GjiModeCommand::ToggleKanaType => result.toggle_kana_type.push(vk_name),
                GjiModeCommand::ImeOn | GjiModeCommand::ImeOff | GjiModeCommand::Other => {
                    unreachable!("filtered out above")
                }
            }
        }
    }

    result.set_mode.sort_unstable();
    result.set_mode.dedup();
    result.toggle_alphanumeric.sort_unstable();
    result.toggle_alphanumeric.dedup();
    result.toggle_kana_type.sort_unstable();
    result.toggle_kana_type.dedup();
    result
}

/// ADR-195 段階0 決定3: `custom_keymap_table` の中から、`SetMode`（絶対設定系）
/// コマンドが入力中（`Composition`/`Conversion`）の status 行に束縛されているキー
/// （[`mozc_key_vk_names`] 形式の VK 名）の集合を返す。
///
/// [`extract_mode_keys`] は `status` 列を分類の一意性判定にしか使わず、結果からは
/// 捨てている。しかし `SetMode` が入力中の status に束縛されていれば、それは
/// 「押した結果の変換モードそのもの」であり、コマンド名だけから初期仮説の予測値を
/// 直接組み立てられる（学習を省略できる）。相対トグル系
/// （`ToggleAlphanumericMode`/`ToggleKanaType`）はこの集合に入らない——現在のモード
/// 依存で遷移先が一意に定まらないため、呼び出し側は「不明、要学習」のままにする。
///
/// キーが確定と認められるのは、(a) `Composition`/`Conversion`のいずれかの行で
/// `SetMode`が観測され、かつ(b)そのキーの`SetMode`行が(status問わず)全て同じ
/// 遷移先モードで一致しているときだけ（[`extract_mode_keys`]の一意性判定と同じ規則、
/// 判定ロジックの二重実装を避けるためここでも同じ「distinctが1件だけ」の基準を使う）。
/// 単一のComposition/Conversion行だけを見て確定と判定すると、他のstatusで食い違う
/// `SetMode`行を持つキーまで誤って確定扱いにしうる。
#[must_use]
pub fn set_mode_keys_confirmed_by_input_progress_status(
    custom_keymap_table: &str,
) -> BTreeSet<String> {
    const INPUT_IN_PROGRESS_STATUSES: &[&str] = &["Composition", "Conversion"];
    let rows = parse_custom_keymap_table(custom_keymap_table);
    // code-review指摘: 一意性判定は`SetMode`行だけでなく、`extract_mode_keys`と同じ
    // 集合(`ToggleAlphanumericMode`/`ToggleKanaType`込み)で行わないと、状態間で
    // `SetMode`と`Toggle*`が食い違うキーまで「一意」と誤判定する(`extract_mode_keys`
    // なら遷移先不定として弾く食い違いを見逃す)。分類の集計自体を`SetMode`のみに
    // 絞っていたのが本関数のドキュメントと実装の食い違いだった。
    let mut commands_by_key: BTreeMap<String, BTreeSet<GjiModeCommand>> = BTreeMap::new();
    let mut has_input_progress_set_mode: BTreeSet<String> = BTreeSet::new();
    for row in &rows {
        let classified = classify_command(&row.command);
        if matches!(
            classified,
            GjiModeCommand::ImeOn | GjiModeCommand::ImeOff | GjiModeCommand::Other
        ) {
            continue;
        }
        commands_by_key
            .entry(row.key.clone())
            .or_default()
            .insert(classified);
        if matches!(classified, GjiModeCommand::SetMode(_))
            && INPUT_IN_PROGRESS_STATUSES.contains(&row.status.as_str())
        {
            has_input_progress_set_mode.insert(row.key.clone());
        }
    }
    let mut result = BTreeSet::new();
    for key in has_input_progress_set_mode {
        let Some(commands) = commands_by_key.get(&key) else {
            continue;
        };
        if commands.len() != 1 {
            // 状態間で遷移先が一意に定まらない(extract_mode_keysと同じ判定)。
            continue;
        }
        // ここに来た時点で`commands`の唯一の要素は`SetMode(_)`だと分かる:
        // `has_input_progress_set_mode`への挿入は`SetMode`行の観測が条件であり、
        // そのキーの`commands_by_key`に他の値が無ければ(len() == 1)その1件が
        // 観測された`SetMode`そのものになるため。
        result.extend(
            mozc_key_vk_names(&key)
                .iter()
                .map(|name| (*name).to_string()),
        );
    }
    result
}

/// `IMEOn`/`IMEOff` 以外のコマンドの行・修飾キー付きの行を除外しつつ、
/// キーごとに「IMEOn が割り当てられている状態」「IMEOff が割り当てられている
/// 状態」を集計する。
fn group_ime_rows_by_key(rows: &[KeymapRow]) -> StatusSetsByKey {
    let mut grouped: StatusSetsByKey = BTreeMap::new();
    for row in rows {
        // command でまず絞る: IMEOn/IMEOff 以外の行（Backspace 等)は空白キー
        // 判定より先に捨てる。修飾キー付き行のログを IME 無関係コマンドで
        // 発火させないため。`classify_command`（`CancelAndIMEOff`を
        // `IMEOff`と同義に扱うエイリアス処理込み）を使い、
        // `extract_mode_keys`と分類ロジックを二重管理しない
        // （/code-review指摘、文字列比較の再実装を避ける）。
        let is_ime_on = matches!(classify_command(&row.command), GjiModeCommand::ImeOn);
        let is_ime_off = matches!(classify_command(&row.command), GjiModeCommand::ImeOff);
        if !is_ime_on && !is_ime_off {
            continue;
        }
        if row.key.contains(char::is_whitespace) {
            tracing::debug!(
                "gji-config: 修飾キー付き行は stage 1 のスコープ外のため無視: key={}",
                row.key
            );
            continue;
        }
        let entry = if is_ime_on {
            &mut grouped.entry(row.key.clone()).or_default().0
        } else {
            &mut grouped.entry(row.key.clone()).or_default().1
        };
        entry.insert(row.status.clone());
    }
    grouped
}

/// 1キー分の on/off 状態集合を、toggle/on/off のいずれかに分類して
/// `result` に積む。どちらにも当てはまらない（状態間で矛盾する）場合は
/// 警告ログのみで取り込まない。
fn classify_and_push(
    key: &str,
    vk_name: &str,
    on_statuses: &BTreeSet<String>,
    off_statuses: &BTreeSet<String>,
    result: &mut GjiImeKeys,
) {
    let on_only_in_off_states = !on_statuses.is_empty()
        && on_statuses
            .iter()
            .all(|s| STATUSES_WHEN_IME_OFF.contains(&s.as_str()));
    let off_only_in_on_states = !off_statuses.is_empty()
        && off_statuses
            .iter()
            .all(|s| STATUSES_WHEN_IME_ON.contains(&s.as_str()));

    if on_only_in_off_states && off_only_in_on_states {
        result.toggle.push(vk_name.to_string());
    } else if !on_statuses.is_empty() && off_statuses.is_empty() {
        result.on.push(vk_name.to_string());
    } else if !off_statuses.is_empty() && on_statuses.is_empty() {
        result.off.push(vk_name.to_string());
    } else {
        tracing::warn!(
            "gji-config: 状態間で矛盾する割当のためスキップしました: key={key} on={on_statuses:?} off={off_statuses:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_ime_keys, extract_mode_keys, mozc_key_vk_names,
        set_mode_keys_confirmed_by_input_progress_status, GjiImeKeys, GjiModeKeys,
    };
    use crate::command::GjiCompositionMode;

    #[test]
    fn f_key_tokens_map_to_vk_names() {
        assert_eq!(mozc_key_vk_names("F1"), ["VK_F1"]);
        assert_eq!(mozc_key_vk_names("F21"), ["VK_F21"]);
        assert_eq!(mozc_key_vk_names("F24"), ["VK_F24"]);
        assert!(mozc_key_vk_names("F25").is_empty());
        assert!(mozc_key_vk_names("F0").is_empty());
        assert!(mozc_key_vk_names("F013").is_empty());
    }

    #[test]
    fn alias_tokens_map_to_vk_names() {
        // ADR-199 T2: 半角/全角は 0xF3/0xF4 の両方（Mozc は同じ KeyEvent::HANKAKU に畳む）。
        // 旧実装は VK_KANJI に写しており、予測側（0xF3/0xF4）と逆向きに食い違っていた。
        for token in ["Hankaku/Zenkaku", "Hankaku", "Zenkaku"] {
            assert_eq!(
                mozc_key_vk_names(token),
                ["VK_DBE_SBCSCHAR", "VK_DBE_DBCSCHAR"]
            );
        }
        // Windows では 0x19 が KeyEvent::KANJI にならないので、Kanji 行は使われない。
        assert!(mozc_key_vk_names("Kanji").is_empty());
        assert_eq!(mozc_key_vk_names("ON"), ["VK_IME_ON"]);
        assert_eq!(mozc_key_vk_names("OFF"), ["VK_IME_OFF"]);
        assert_eq!(mozc_key_vk_names("Eisu"), ["VK_DBE_ALPHANUMERIC"]);
        // BUG-115: Henkan/Muhenkanは、config1.dbのcustom_keymap_tableに
        // literalに含まれうる（例: ユーザーがATOKベースからカスタムを
        // 作った場合）。
        assert_eq!(mozc_key_vk_names("Henkan"), ["VK_CONVERT"]);
        assert_eq!(mozc_key_vk_names("Muhenkan"), ["VK_NONCONVERT"]);
        assert_eq!(mozc_key_vk_names("Hiragana"), ["VK_DBE_HIRAGANA"]);
        assert_eq!(mozc_key_vk_names("Kana"), ["VK_DBE_HIRAGANA"]);
        assert_eq!(mozc_key_vk_names("Katakana"), ["VK_DBE_KATAKANA"]);
        // プリセット TSV の綴り（`ESC`）と Mozc の別名。
        assert_eq!(mozc_key_vk_names("ESC"), ["VK_ESCAPE"]);
        assert_eq!(mozc_key_vk_names("Escape"), ["VK_ESCAPE"]);
        assert_eq!(mozc_key_vk_names("BS"), ["VK_BACK"]);
        assert_eq!(mozc_key_vk_names("Backspace"), ["VK_BACK"]);
        assert_eq!(mozc_key_vk_names("Return"), ["VK_RETURN"]);
        assert_eq!(mozc_key_vk_names("Enter"), ["VK_RETURN"]);
        assert_eq!(mozc_key_vk_names("Space"), ["VK_SPACE"]);
    }

    #[test]
    fn tokens_are_case_insensitive_and_trimmed_like_mozc() {
        assert_eq!(mozc_key_vk_names("hankaku/zenkaku").len(), 2);
        assert_eq!(mozc_key_vk_names(" f13 "), ["VK_F13"]);
    }

    #[test]
    fn unknown_or_modified_token_maps_to_nothing() {
        assert!(mozc_key_vk_names("Insert").is_empty());
        assert!(mozc_key_vk_names("").is_empty());
        assert!(mozc_key_vk_names("Ctrl Space").is_empty());
        assert!(mozc_key_vk_names("Shift Hankaku/Zenkaku").is_empty());
    }

    /// BUG-115: ユーザーがATOKベースからカスタムキーマップを作った場合を
    /// 想定したフィクスチャ（`atok.tsv`実データを、custom_keymap_table
    /// として書き写した体、と同じ構造）。DirectInputでIMEOn・
    /// Precompositionで`CancelAndIMEOff`という状態依存の割当てが、
    /// `toggle`として正しく分類されることを確認する
    /// （`CancelAndIMEOff`をIMEOff相当として扱うロジックの検証）。
    #[test]
    fn henkan_muhenkan_atok_style_custom_table_classifies_as_toggle() {
        let text = "status\tkey\tcommand
DirectInput\tHenkan\tIMEOn
DirectInput\tMuhenkan\tIMEOn
Precomposition\tHenkan\tCancelAndIMEOff
Precomposition\tMuhenkan\tCancelAndIMEOff
";
        let keys = extract_ime_keys(text);
        assert_eq!(
            keys,
            GjiImeKeys {
                on: vec![],
                off: vec![],
                toggle: vec!["VK_CONVERT".to_string(), "VK_NONCONVERT".to_string()],
            }
        );
    }

    /// BUG-115: overlay相当（Henkan→IMEOnのみ、Muhenkan→IMEOffのみ、状態間
    /// で矛盾しない）をcustom_keymap_tableに直接書いた場合は`on`/`off`に
    /// 分類され、`Toggle`にはならない（冪等なので警告不要）ことを確認する。
    #[test]
    fn henkan_muhenkan_consistent_custom_table_classifies_as_on_off() {
        let text = "status\tkey\tcommand
DirectInput\tHenkan\tIMEOn
Precomposition\tHenkan\tIMEOn
Composition\tHenkan\tIMEOn
Conversion\tHenkan\tIMEOn
Composition\tMuhenkan\tIMEOff
Conversion\tMuhenkan\tIMEOff
Precomposition\tMuhenkan\tIMEOff
";
        let keys = extract_ime_keys(text);
        assert_eq!(
            keys,
            GjiImeKeys {
                on: vec!["VK_CONVERT".to_string()],
                off: vec!["VK_NONCONVERT".to_string()],
                toggle: vec![],
            }
        );
    }

    /// 実機で取得した GJI カスタムキーマップの実データに現れたパターンを
    /// 反映したフィクスチャ（このセッションで実機確認済みの値そのものではなく、
    /// 同じ構造を持つ代表例として再構成したもの。個人設定の丸ごとコミットは
    /// 避ける）。
    const FIXTURE_TSV: &str = "status\tkey\tcommand
Composition\tHankaku/Zenkaku\tIMEOff
Conversion\tHankaku/Zenkaku\tIMEOff
DirectInput\tHankaku/Zenkaku\tIMEOn
Precomposition\tHankaku/Zenkaku\tIMEOff
Composition\tKanji\tIMEOff
Conversion\tKanji\tIMEOff
DirectInput\tKanji\tIMEOn
Precomposition\tKanji\tIMEOff
DirectInput\tF13\tIMEOn
DirectInput\tF21\tIMEOn
Precomposition\tF21\tIMEOn
Composition\tF21\tIMEOn
Conversion\tF21\tIMEOn
Precomposition\tF22\tIMEOff
Composition\tF22\tIMEOff
Conversion\tF22\tIMEOff
Composition\tON\tIMEOn
Composition\tOFF\tIMEOff
Conversion\tON\tIMEOn
Conversion\tOFF\tIMEOff
DirectInput\tON\tIMEOn
Precomposition\tON\tIMEOn
Precomposition\tOFF\tIMEOff
DirectInput\tEisu\tIMEOn
Composition\tEisu\tToggleAlphanumericMode
Conversion\tEisu\tToggleAlphanumericMode
Precomposition\tEisu\tToggleAlphanumericMode
Precomposition\tCtrl Shift Insert\tIMEOn
Precomposition\tCtrl Shift Delete\tIMEOff
Composition\tBackspace\tBackspace
Composition\tSpace\tConvert
";

    #[test]
    fn extracts_toggle_on_off_from_fixture() {
        let keys = extract_ime_keys(FIXTURE_TSV);
        assert_eq!(
            keys,
            GjiImeKeys {
                on: vec![
                    "VK_DBE_ALPHANUMERIC".to_string(), // Eisu (DirectInputのみ)
                    "VK_F13".to_string(),
                    "VK_F21".to_string(),
                    "VK_IME_ON".to_string(), // ON
                ],
                off: vec!["VK_F22".to_string(), "VK_IME_OFF".to_string()], // OFF
                // Hankaku/Zenkaku は 0xF3/0xF4 の両方。Kanji 行は VK に写らない。
                toggle: vec!["VK_DBE_DBCSCHAR".to_string(), "VK_DBE_SBCSCHAR".to_string()],
            }
        );
    }

    #[test]
    fn empty_table_yields_empty_keys() {
        assert_eq!(extract_ime_keys(""), GjiImeKeys::default());
    }

    #[test]
    fn conflicting_key_is_dropped_not_panicking() {
        // 同じ状態グループ内で on/off が両方立つ、解釈不能な矛盾行。
        let text = "status\tkey\tcommand\nComposition\tF15\tIMEOn\nComposition\tF15\tIMEOff\n";
        let keys = extract_ime_keys(text);
        assert_eq!(keys, GjiImeKeys::default());
    }

    #[test]
    fn prediction_and_suggestion_are_recognized_as_ime_on_states() {
        // STATUSES_WHEN_IME_ON の5状態のうちメインfixtureがカバーしないのは
        // Prediction/Suggestion。この2つが正しく toggle 判定に効くことを
        // 個別に固定する（この定数を削っても検知できるように）。
        let text = "status\tkey\tcommand
DirectInput\tF20\tIMEOn
Prediction\tF20\tIMEOff
DirectInput\tF19\tIMEOn
Suggestion\tF19\tIMEOff
";
        let keys = extract_ime_keys(text);
        assert_eq!(
            keys,
            GjiImeKeys {
                on: vec![],
                off: vec![],
                toggle: vec!["VK_F19".to_string(), "VK_F20".to_string()],
            }
        );
    }

    #[test]
    fn extracts_absolute_and_toggle_mode_keys_from_fixture() {
        // Mozc の data/keymap/*.tsv 実データではこれらのコマンドは通常 Ctrl 付き
        // キーに割り当てられているが（stage 1 のスコープ外）、分類ロジック自体は
        // どのキーに割り当てられても同じなので、単発キーで代表させて検証する。
        let text = "status\tkey\tcommand
DirectInput\tF7\tCompositionModeFullKatakana
Precomposition\tF7\tCompositionModeFullKatakana
Composition\tF7\tCompositionModeFullKatakana
Conversion\tF7\tCompositionModeFullKatakana
DirectInput\tF6\tCompositionModeHiragana
Precomposition\tF6\tCompositionModeHiragana
DirectInput\tF10\tCompositionModeHalfAlphanumeric
DirectInput\tEisu\tIMEOn
Precomposition\tEisu\tToggleAlphanumericMode
Composition\tEisu\tToggleAlphanumericMode
Conversion\tEisu\tToggleAlphanumericMode
Precomposition\tF8\tSwitchKanaType
Composition\tBackspace\tBackspace
";
        let keys = extract_mode_keys(text);
        assert_eq!(
            keys,
            GjiModeKeys {
                set_mode: vec![
                    ("VK_F10".to_string(), GjiCompositionMode::HalfAlphanumeric),
                    ("VK_F6".to_string(), GjiCompositionMode::Hiragana),
                    ("VK_F7".to_string(), GjiCompositionMode::FullKatakana),
                ],
                // Eisu の IMEOn(DirectInput) は classify_command で ImeOn に
                // 分類され extract_mode_keys では無視されるため、残る
                // ToggleAlphanumericMode だけが一意に定まり採用される。
                toggle_alphanumeric: vec!["VK_DBE_ALPHANUMERIC".to_string()],
                toggle_kana_type: vec!["VK_F8".to_string()],
            }
        );
    }

    #[test]
    fn empty_table_yields_empty_mode_keys() {
        assert_eq!(extract_mode_keys(""), GjiModeKeys::default());
    }

    #[test]
    fn conflicting_mode_key_is_dropped_not_panicking() {
        // 同じキーが状態によって異なる絶対モードへ設定される、解釈不能な矛盾行。
        let text = "status\tkey\tcommand
DirectInput\tF9\tCompositionModeHiragana
Precomposition\tF9\tCompositionModeFullKatakana
";
        let keys = extract_mode_keys(text);
        assert_eq!(keys, GjiModeKeys::default());
    }

    // ── ADR-195 段階0: set_mode_keys_confirmed_by_input_progress_status ──

    #[test]
    fn set_mode_confirmed_when_bound_to_composition_or_conversion() {
        let text = "status\tkey\tcommand
DirectInput\tF6\tCompositionModeHiragana
Precomposition\tF6\tCompositionModeHiragana
Composition\tF6\tCompositionModeHiragana
Conversion\tF7\tCompositionModeFullKatakana
";
        let confirmed = set_mode_keys_confirmed_by_input_progress_status(text);
        assert_eq!(
            confirmed,
            ["VK_F6", "VK_F7"].into_iter().map(String::from).collect()
        );
    }

    #[test]
    fn set_mode_not_confirmed_when_bound_only_to_direct_input_or_precomposition() {
        // 実際にはSetModeはF6のようにDirectInput/Precompositionにも束縛されるが、
        // Composition/Conversionのstatus行が無い限り未確定のまま。
        let text = "status\tkey\tcommand
DirectInput\tF6\tCompositionModeHiragana
Precomposition\tF6\tCompositionModeHiragana
";
        assert!(set_mode_keys_confirmed_by_input_progress_status(text).is_empty());
    }

    #[test]
    fn set_mode_confirmed_ignores_relative_toggle_commands() {
        // 相対トグル系はComposition/Conversionに束縛されていても対象外
        // （遷移先が現在モード依存で一意に定まらないため）。
        let text = "status\tkey\tcommand
Composition\tEisu\tToggleAlphanumericMode
Conversion\tF8\tSwitchKanaType
";
        assert!(set_mode_keys_confirmed_by_input_progress_status(text).is_empty());
    }

    #[test]
    fn set_mode_confirmed_empty_table_yields_empty_set() {
        assert!(set_mode_keys_confirmed_by_input_progress_status("").is_empty());
    }

    #[test]
    fn set_mode_not_confirmed_when_key_also_has_a_toggle_row_in_another_status() {
        // code-review指摘: 同じキーがComposition中はSetModeに束縛されている一方、
        // 別のstatus(ここではDirectInput)ではToggleAlphanumericModeに束縛されて
        // いる場合、extract_mode_keysなら「遷移先が状態依存で一意に定まらない」
        // として除外する(SetModeとToggleが2種の異なる分類として同じキーに
        // 集まるため)。本関数もextract_mode_keysと同じ一意性判定を謳っている
        // 以上、SetMode行だけを見て「一意」と誤確定してはならない。
        let text = "status\tkey\tcommand
DirectInput\tEisu\tToggleAlphanumericMode
Composition\tEisu\tCompositionModeHiragana
";
        assert!(set_mode_keys_confirmed_by_input_progress_status(text).is_empty());
    }
}
