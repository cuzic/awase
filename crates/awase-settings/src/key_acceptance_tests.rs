//! ADR-201 段階0(決定4-1): 設定 GUI の候補表 × その項目の**実際の読み手**の受理テスト。
//!
//! GUI の候補表(`THUMB_KEY_OPTIONS`・`ALT_IMPERSONATION_OPTIONS`・`IME_MODE_KEY_OPTIONS`・
//! `SOLO_REPEAT_EXTRA_OPTIONS`・`KEYMAP_MAIN_KEYS`)の全内部名を、`format_combo` を通した
//! 文字列にして読み手へ渡し、`Some` になることを確かめる。BUG-167(GUI が書く
//! `Ctrl+Shift+VK_F12` を `parse_hotkey` が読めない)は、このテストがあれば CI で見つかっていた。
//!
//! **既知の失敗はデータ(`KNOWN_READER_FAILURES`・`KNOWN_ROUNDTRIP_LOSSES`)として持ち、その
//! 一覧どおりに失敗することを期待値にする。** 段階1(`from_name` と修飾キー解釈の寛容化)で
//! 一覧を空にする。修正が効くと「一覧に残っているのに失敗しない」でテストが落ちるので、
//! 一覧の消し忘れは起きない。
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

/// 読み手ごとの既知の失敗。書式 `"<読み手>|<内部名>"`。段階1で空にする。
///
/// ホットキー(`engine_toggle_hotkey`)の `変換`/`無変換`/`かな`/`漢字`: GUI の候補
/// `KEYMAP_MAIN_KEYS` の内部名は `VK_` 無しの日本語名で、`parse_hotkey` が無条件で `VK_` を
/// 付けて `VK_変換` になり `from_name` が失敗する(背景 #1)。`register_toggle` の失敗は
/// `warn!` のみで、ホットキーが無言で無効になる。
const KNOWN_READER_FAILURES: &[&str] = &[
    "hotkey(parse_hotkey)|かな",
    "hotkey(parse_hotkey)|変換",
    "hotkey(parse_hotkey)|無変換",
    "hotkey(parse_hotkey)|漢字",
];

/// GUI の読み手 `parse_combo_str` が、手書きの表記で修飾キーを落とす入力。
/// `parse_combo_str` は `Ctrl|Control|Shift|Alt` の完全一致で、それ以外は黙って捨てる
/// (決定1: 修飾キーの解釈を1関数にして大文字小文字を `from_name` と揃える)。段階1で空にする。
const KNOWN_ROUNDTRIP_LOSSES: &[&str] =
    &["CTRL+VK_J", "alt+VK_F4", "ctrl+shift+VK_F12", "shift+VK_A"];

/// 修飾キーの全8通り `(ctrl, shift, alt)`。
fn all_mods() -> Vec<(bool, bool, bool)> {
    (0..8)
        .map(|i| (i & 1 != 0, i & 2 != 0, i & 4 != 0))
        .collect()
}

/// `parse_hotkey` と同じ経路でホットキー文字列が読めるか。Windows では実物を呼ぶ。
fn hotkey_readable(s: &str) -> bool {
    #[cfg(windows)]
    {
        awase_windows::vk::parse_hotkey(s).is_some()
    }
    #[cfg(not(windows))]
    {
        use awase_windows::vk::with_vk_prefix;
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let Some((last, mods)) = parts.split_last() else {
            return false;
        };
        mods.iter()
            .all(|m| matches!(*m, "Ctrl" | "Control" | "Shift" | "Alt"))
            && VkCode::from_name(&with_vk_prefix(last)).is_some()
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
            Box::new(|n| {
                VkCode::from_name(n)
                    .or_else(|| VkCode::from_name(&format!("VK_{n}")))
                    .is_some()
            }),
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

    let known: BTreeSet<String> = KNOWN_READER_FAILURES
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let fixed: Vec<_> = known.difference(&failures).collect();
    let new: Vec<_> = failures.difference(&known).collect();
    assert!(
        fixed.is_empty() && new.is_empty(),
        "GUI の候補 × 読み手: 既知の失敗の一覧と現状が違う。\n\
         - 新しい失敗(GUI が書く値を読み手が読めない。回帰): {new:#?}\n\
         - もう失敗しない(修正済み。KNOWN_READER_FAILURES から消す): {fixed:#?}"
    );
}

/// 「読めない候補」が今ある項目でも、それが**候補表そのものの誤り**(`from_name` に無い名前)
/// ではないこと。既知の失敗は「`VK_` を補う読み手側」の問題で、`from_name` 自体は内部名を解決する。
#[test]
fn known_reader_failures_resolve_via_from_name() {
    for entry in KNOWN_READER_FAILURES {
        let (_, internal) = entry.split_once('|').unwrap();
        assert!(
            VkCode::from_name(internal).is_some(),
            "{internal}: from_name でも解決できない(候補表側の誤り)"
        );
    }
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
/// `(手書きの文字列, ctrl, shift, alt)`。読み手 `parse_combo_str` が完全一致のため、
/// 小文字などは今は落ちる(`KNOWN_ROUNDTRIP_LOSSES`)。
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
    let known: BTreeSet<String> = KNOWN_ROUNDTRIP_LOSSES
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    assert_eq!(
        lost, known,
        "GUI の読み手が修飾キーを落とす入力の一覧が、既知の一覧と違う(増えたら回帰、\
         減ったら KNOWN_ROUNDTRIP_LOSSES から消す)"
    );
}
