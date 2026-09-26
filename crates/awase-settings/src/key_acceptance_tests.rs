//! ADR-201 段階0(決定4-1): 設定 GUI の候補表 × その項目の**実際の読み手**の受理テスト。
//!
//! GUI の候補表(`THUMB_KEY_OPTIONS`・`ALT_IMPERSONATION_OPTIONS`・`IME_MODE_KEY_OPTIONS`・
//! `SOLO_REPEAT_EXTRA_OPTIONS`・`KEYMAP_MAIN_KEYS`)の全内部名を、`format_combo` を通した
//! 文字列にして読み手へ渡し、`Some` になることを確かめる。BUG-167(GUI が書く
//! `Ctrl+Shift+VK_F12` を `parse_hotkey` が読めない)は、このテストがあれば CI で見つかっていた。
//!
//! 段階0では現状の失敗を「既知の失敗の一覧」として期待値にしていた(ホットキーの
//! `変換`/`無変換`/`かな`/`漢字`、GUI の読み手が手書きの小文字の修飾キーを落とす件)。
//! 段階1(`from_name` と修飾キー解釈の寛容化、1関数への集約)で一覧は空になり、
//! 今は「失敗が1件も無いこと」を直接検査する。
//!
//! 走る場所: `windows-settings` ジョブ(`cargo nextest run -p awase-settings`、実 Windows)。
//! ubuntu の `test` ジョブは `awase-settings` を `cargo check` するだけでテストは走らせない
//! (lib target が無く `--workspace --lib` の対象外)。Linux でも `cargo test -p awase-settings`
//! は通る(ローカルで確認済み)。`parse_hotkey` は `#[cfg(windows)]` なので、Linux では前置き処理
//! まで含めた等価コード(`hotkey_readable`)で確かめ、Windows では実物を呼ぶ。

use std::collections::BTreeSet;

use awase::types::VkCode;
use awase_windows::state::alt_impersonation::resolve_thumb_key;
use awase_windows::vk::{VK_CONVERT, VK_NONCONVERT, VkCodeExt, parse_key_combo};

use super::{
    ALT_IMPERSONATION_OPTIONS, IME_MODE_KEY_OPTIONS, KEYMAP_MAIN_KEYS, SOLO_REPEAT_EXTRA_OPTIONS,
    THUMB_KEY_OPTIONS, format_combo, keymap_from_key_options, keymap_to_key_options,
    parse_combo_str, physical_key_options,
};

/// 修飾キーの全8通り `(ctrl, shift, alt)`。
fn all_mods() -> Vec<(bool, bool, bool)> {
    (0..8)
        .map(|i| (i & 1 != 0, i & 2 != 0, i & 4 != 0))
        .collect()
}

/// `parse_hotkey` と同じ経路でホットキー文字列が読めるか。Windows では実物を呼ぶ。
/// `parse_hotkey` は `parse_key_combo` の薄い変換(段階1)なので、Linux では後者で確かめる。
fn hotkey_readable(s: &str) -> bool {
    #[cfg(windows)]
    {
        awase_windows::vk::parse_hotkey(s).is_some()
    }
    #[cfg(not(windows))]
    {
        parse_key_combo(s).is_some()
    }
}

/// `parse_key_combo` が、渡した修飾キーと同じ組を返し、主キーが `from_name(internal)` と
/// 同じ VK になること(全8通りの修飾キーで)。
fn combo_accepts(internal: &str) -> bool {
    let Some(expected) = VkCode::from_name(internal) else {
        return false;
    };
    all_mods().into_iter().all(|(c, s, a)| {
        parse_key_combo(&format_combo(c, s, a, internal))
            .is_some_and(|k| (k.ctrl, k.shift, k.alt) == (c, s, a) && k.vk == expected)
    })
}

type Options = Vec<&'static (&'static str, &'static str)>;
type Reader = (&'static str, Options, Box<dyn Fn(&str) -> bool>);

/// (読み手の名前, 候補, 読み手が受理するか) の全組。
fn readers() -> Vec<Reader> {
    let thumb: Options = THUMB_KEY_OPTIONS
        .iter()
        .chain(ALT_IMPERSONATION_OPTIONS)
        .collect();
    let engine: Options = THUMB_KEY_OPTIONS
        .iter()
        .chain(IME_MODE_KEY_OPTIONS)
        .collect();
    let solo: Options = SOLO_REPEAT_EXTRA_OPTIONS
        .iter()
        .chain(THUMB_KEY_OPTIONS.iter())
        .collect();
    vec![
        (
            // left_thumb_key / right_thumb_key(`thumb_key_combo`)。値は内部名そのもの。
            "thumb_key(resolve_thumb_key)",
            thumb,
            Box::new(|n| resolve_thumb_key(n).is_some()),
        ),
        (
            // keys.engine_on/off・ime_on/off/toggle(`combo_key_list_ui`+`engine_key_combo`)。
            "combo_list(parse_key_combo)",
            engine,
            Box::new(combo_accepts),
        ),
        (
            // keys.engine_off_solo_repeat(`solo_repeat_combo`)。値は内部名そのもの。
            "solo_repeat(from_name)",
            solo,
            Box::new(|n| VkCode::from_name(n).is_some()),
        ),
        (
            // general.engine_toggle_hotkey(`hotkey_combo_ui`+`physical_key_options`)。
            "hotkey(parse_hotkey)",
            physical_key_options().collect(),
            Box::new(|n| {
                all_mods()
                    .into_iter()
                    .all(|(c, s, a)| hotkey_readable(&format_combo(c, s, a, n)))
            }),
        ),
        (
            // [[post_bypass]] key(`main_key_combo`+`physical_key_options`、Ctrl 固定)。
            "post_bypass(parse_key_combo)",
            physical_key_options().collect(),
            Box::new(|n| {
                parse_key_combo(&format_combo(true, false, false, n))
                    .is_some_and(|k| k.ctrl && Some(k.vk) == VkCode::from_name(n))
            }),
        ),
        (
            // [[keymaps]] from(`keymap_from_key_options`)。
            "keymap_from(parse_key_combo)",
            keymap_from_key_options(VK_NONCONVERT, VK_CONVERT).collect(),
            Box::new(combo_accepts),
        ),
        (
            // [[keymaps]] to(`keymap_to_key_options`)。`KeymapTable::new` の解決と同じ。
            "keymap_to(from_name)",
            keymap_to_key_options(VK_NONCONVERT, VK_CONVERT).collect(),
            Box::new(|n| VkCode::from_name(n).is_some()),
        ),
        (
            // 候補表全体(どの項目に出す予定であれ、`parse_key_combo` で読めること)。
            "keymap_main_keys_all(parse_key_combo)",
            KEYMAP_MAIN_KEYS.iter().collect(),
            Box::new(combo_accepts),
        ),
    ]
}

#[test]
fn gui_candidates_are_accepted_by_their_readers() {
    let mut failures = BTreeSet::new();
    let mut checked = 0usize;
    for (reader, options, accepts) in readers() {
        assert!(!options.is_empty(), "{reader}: 候補が空(候補表の名前変更?)");
        for (_, internal) in options {
            checked += 1;
            if !accepts(internal) {
                failures.insert(format!("{reader}|{internal}"));
            }
        }
    }
    assert!(checked > 300, "検査件数が少なすぎる: {checked}");

    assert!(
        failures.is_empty(),
        "GUI の候補 × 読み手: GUI が書く値を読み手が読めない(回帰): {failures:#?}"
    );
}

/// GUI の書き手 `format_combo` → GUI の読み手 `parse_combo_str` の往復で、修飾キーと主キーが
/// 落ちないこと(全候補 × 全8通り)。
#[test]
fn format_then_parse_combo_roundtrips_for_all_candidates() {
    for (_, internal) in KEYMAP_MAIN_KEYS
        .iter()
        .chain(THUMB_KEY_OPTIONS)
        .chain(IME_MODE_KEY_OPTIONS)
        .chain(SOLO_REPEAT_EXTRA_OPTIONS)
    {
        for (c, s, a) in all_mods() {
            let text = format_combo(c, s, a, internal);
            assert_eq!(
                parse_combo_str(&text),
                (c, s, a, (*internal).to_string()),
                "往復で変わった: {text:?}"
            );
        }
    }
}

/// 手書きの表記を GUI が開いて保存し直したとき、修飾キーが落ちないこと。
/// `(手書きの文字列, ctrl, shift, alt)`。`parse_combo_str` は `vk::interpret_combo` を使い、
/// 大文字小文字を `from_name` と同じ規則で扱う。
#[test]
fn hand_written_modifiers_survive_gui_parse() {
    let cases: &[(&str, (bool, bool, bool))] = &[
        ("Ctrl+Shift+VK_F12", (true, true, false)),
        ("Control+VK_J", (true, false, false)),
        ("Alt+VK_F4", (false, false, true)),
        (" Ctrl + Shift + VK_A ", (true, true, false)),
        ("CTRL+VK_J", (true, false, false)),
        ("alt+VK_F4", (false, false, true)),
        ("ctrl+shift+VK_F12", (true, true, false)),
        ("shift+VK_A", (false, true, false)),
    ];
    let lost: BTreeSet<String> = cases
        .iter()
        .filter(|(text, want)| {
            let (c, s, a, _) = parse_combo_str(text);
            (c, s, a) != *want
        })
        .map(|(text, _)| (*text).to_string())
        .collect();
    assert!(
        lost.is_empty(),
        "GUI の読み手が修飾キーを落とす入力がある: {lost:#?}"
    );
}
