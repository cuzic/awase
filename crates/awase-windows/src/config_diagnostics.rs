//! ADR-201 段階2: 設定の読み込みで「無言で握りつぶされていた」ものを診断に流すための、
//! 副作用のない判定関数(Windows 依存なし。Linux でもテストできる)。
//!
//! 出力先(`StartupDiagnostics`、`app/mod.rs`)への配線は `app/bootstrap.rs`・`app/mod.rs` にある。

use awase::config::{AppConfig, PostBypassRule};
use awase::types::VkCode;

use crate::vk::{interpret_combo, parse_key_combo, VkCodeExt};

/// 段階1(`from_name` の寛容化)より前の `from_name` が、`VK_` なしの名前として
/// **完全一致(大文字小文字を区別)で**受けていた名前。`VK_` 付きの名前は別(下記)。
const LEGACY_NO_PREFIX_NAMES: &[&str] = &[
    "Convert",
    "変換",
    "Nonconvert",
    "無変換",
    "Kana",
    "かな",
    "カナ",
    "Kanji",
    "漢字",
    "ImeOn",
    "IMEオン",
    "ImeOff",
    "IMEオフ",
];

/// 段階1で足した別名のうち、`VK_` を付けた形(`VK_ENTER` 等)も旧規則では受理されなかったもの。
const NEW_ONLY_VK_SUFFIXES: &[&str] = &["ENTER", "BACKSPACE", "ESC", "IMEON", "IMEOFF"];

/// 旧規則(表の名前の完全一致・大文字小文字を区別・`VK_` 必須)では解決できなかったが、
/// 寛容な `from_name` では解決できる名前か。これが「以前は無視されていた設定が有効になった」
/// の定義(ADR-201 決定2・未決事項1)。今も解決できない名前は `false`(警告の対象で別扱い)。
#[must_use]
pub fn name_resolved_only_by_lenient_rule(name: &str) -> bool {
    if VkCode::from_name(name).is_none() {
        return false;
    }
    if LEGACY_NO_PREFIX_NAMES.contains(&name) {
        return false;
    }
    let strict = name.trim() == name
        && name.starts_with("VK_")
        && name.is_ascii()
        && name == name.to_ascii_uppercase()
        && !NEW_ONLY_VK_SUFFIXES.contains(&&name[3..]);
    !strict
}

/// 組み合わせ(`"Ctrl+j"`)版。修飾キーの表記が旧規則(`Ctrl`/`Control`/`Shift`/`Alt` の完全一致)
/// と違う、または主キーが上の条件を満たすもの。今も解決できないものは `false`。
#[must_use]
pub fn combo_resolved_only_by_lenient_rule(s: &str) -> bool {
    combo_lenient(s, false)
}

/// `engine_toggle_hotkey` 版。旧 `parse_hotkey`(BUG-167 の修正後)は主キーに `VK_` を補って
/// から表を引いたので、`Ctrl+Shift+F12` は以前から有効だった(`変換` などは `VK_変換` になって無効)。
#[must_use]
pub fn hotkey_resolved_only_by_lenient_rule(s: &str) -> bool {
    combo_lenient(s, true)
}

fn combo_lenient(s: &str, prefix_main: bool) -> bool {
    if parse_key_combo(s).is_none() {
        return false;
    }
    let (mods, _) = awase::key_text::split_combo(s);
    if mods
        .iter()
        .any(|m| !matches!(*m, "Ctrl" | "Control" | "Shift" | "Alt"))
    {
        return true;
    }
    let main = interpret_combo(s).main;
    if prefix_main && !main.starts_with("VK_") {
        return name_resolved_only_by_lenient_rule(&format!("VK_{main}"));
    }
    name_resolved_only_by_lenient_rule(main)
}

/// 設定の各キー項目のうち、寛容化によって初めて解決できるようになったもの(`"項目=値"`)。
/// 旧規則で無視されていた(ホットキーが無効・警告付きで捨てられる・無言で消える)設定が、
/// 更新しただけで効き始めることをユーザーに知らせるための材料。
#[must_use]
pub fn newly_effective_keys(c: &AppConfig) -> Vec<String> {
    let mut out = Vec::new();
    let mut name = |label: &str, v: &str| {
        if name_resolved_only_by_lenient_rule(v) {
            out.push(format!("{label}={v}"));
        }
    };
    name("general.left_thumb_key", &c.general.left_thumb_key);
    name("general.right_thumb_key", &c.general.right_thumb_key);
    if let Some(k) = &c.general.muhenkan_solo_tap_dedicated_fn_key {
        name("general.muhenkan_solo_tap_dedicated_fn_key", k);
    }
    for (label, list) in [
        ("keys.ime_detect.toggle", &c.keys.ime_detect.toggle),
        ("keys.ime_detect.on", &c.keys.ime_detect.on),
        ("keys.ime_detect.off", &c.keys.ime_detect.off),
    ] {
        for s in list {
            name(label, s);
        }
    }
    for (label, v) in [
        (
            "keys.engine_off_solo_repeat",
            &c.keys.engine_off_solo_repeat,
        ),
        ("keys.engine_on_ime_key", &c.keys.engine_on_ime_key),
        ("keys.engine_off_ime_key", &c.keys.engine_off_ime_key),
    ] {
        if let Some(s) = v.as_deref() {
            name(label, s);
        }
    }
    for r in &c.keymaps {
        for to in &r.to {
            name("keymaps.to", to);
        }
    }
    if let Some(h) = &c.general.engine_toggle_hotkey {
        if hotkey_resolved_only_by_lenient_rule(h) {
            out.push(format!("general.engine_toggle_hotkey={h}"));
        }
    }
    let mut combo = |label: &str, v: &str| {
        if combo_resolved_only_by_lenient_rule(v) {
            out.push(format!("{label}={v}"));
        }
    };
    for (label, list) in [
        ("keys.engine_on", &c.keys.engine_on),
        ("keys.engine_off", &c.keys.engine_off),
        ("keys.ime_on", &c.keys.ime_on),
        ("keys.ime_off", &c.keys.ime_off),
        ("keys.ime_toggle", &c.keys.ime_toggle),
    ] {
        for s in list {
            combo(label, s);
        }
    }
    for r in &c.post_bypass {
        combo("post_bypass.key", &r.key);
    }
    for r in &c.keymaps {
        combo("keymaps.from", &r.from);
    }
    out
}

/// 「以前は無視されていた設定 N 件が有効になりました」のログ用の文言。該当が無ければ `None`。
#[must_use]
pub fn newly_effective_note(c: &AppConfig) -> Option<String> {
    let items = newly_effective_keys(c);
    if items.is_empty() {
        return None;
    }
    Some(format!(
        "以前は無視されていた設定 {} 件が有効になりました: {}",
        items.len(),
        items.join(", ")
    ))
}

/// `[[post_bypass]]` の1件を解釈する。解決できれば `(VK, )`、できなければ理由の文言。
///
/// # Errors
///
/// キーが解決できない、または `Ctrl+` 形式でないときに、診断に流す文言を返す。
pub fn resolve_post_bypass_key(rule: &PostBypassRule) -> Result<VkCode, String> {
    let combo = parse_key_combo(&rule.key)
        .ok_or_else(|| format!("[[post_bypass]] の key を解決できません: \"{}\"", rule.key))?;
    if !combo.ctrl {
        return Err(format!(
            "[[post_bypass]] の key \"{}\" は Ctrl+key 形式であること（例: \"Ctrl+J\"）",
            rule.key
        ));
    }
    Ok(combo.vk)
}

/// `Option<&str>` のキー名を解決する。空文字は「未設定」として扱う(`engine_off_solo_repeat` と同じ)。
/// 解決できない名前は `Err`(診断に流す文言)。
///
/// # Errors
///
/// 名前が VK として解決できないとき。
pub fn resolve_optional_key_name(
    label: &str,
    name: Option<&str>,
) -> Result<Option<VkCode>, String> {
    name.filter(|s| !s.is_empty()).map_or(Ok(None), |s| {
        VkCode::from_name(s)
            .map(Some)
            .ok_or_else(|| format!("{label} のキー名を解決できません: \"{s}\""))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_names_are_not_reported_as_newly_effective() {
        for n in [
            "VK_F12",
            "VK_A",
            "無変換",
            "Nonconvert",
            "IMEオン",
            "VK_OEM_1",
            "VK_INSERT",
            "Kana",
        ] {
            assert!(!name_resolved_only_by_lenient_rule(n), "{n}");
        }
    }

    #[test]
    fn lenient_only_names_are_reported() {
        for n in [
            "F13",
            "f13",
            "vk_f13",
            " VK_F13",
            "Enter",
            "VK_ESC",
            "VK_ENTER",
            "VK_変換",
            "convert",
            "Space",
        ] {
            assert!(name_resolved_only_by_lenient_rule(n), "{n}");
        }
        assert!(
            !name_resolved_only_by_lenient_rule("NoSuchKey"),
            "解決できない名前は対象外(警告の側)"
        );
    }

    #[test]
    fn combos_are_judged_by_modifier_spelling_and_main_key() {
        assert!(!combo_resolved_only_by_lenient_rule("Ctrl+Shift+無変換"));
        assert!(!combo_resolved_only_by_lenient_rule("Ctrl+VK_F12"));
        assert!(combo_resolved_only_by_lenient_rule("Ctrl+J"));
        assert!(combo_resolved_only_by_lenient_rule("ctrl+VK_J"));
        assert!(!combo_resolved_only_by_lenient_rule("Ctrl+NoSuchKey"));
        // ホットキーは旧 parse_hotkey が `VK_` を補っていたので F12 は以前から有効、`変換` は新しく有効
        assert!(!hotkey_resolved_only_by_lenient_rule("Ctrl+Shift+F12"));
        assert!(!hotkey_resolved_only_by_lenient_rule("Ctrl+Shift+VK_F12"));
        assert!(hotkey_resolved_only_by_lenient_rule("Ctrl+Shift+変換"));
        assert!(hotkey_resolved_only_by_lenient_rule("Ctrl+Shift+f12"));
    }

    #[test]
    fn default_and_bundled_config_have_nothing_newly_effective() {
        assert!(newly_effective_keys(&AppConfig::default()).is_empty());
        let bundled = AppConfig::from_toml_str(include_str!("../../../config.toml")).unwrap();
        assert!(
            newly_effective_keys(&bundled).is_empty(),
            "{:?}",
            newly_effective_keys(&bundled)
        );
    }

    #[test]
    fn newly_effective_note_lists_items() {
        let c = AppConfig::from_toml_str(
            "[general]\nmuhenkan_solo_tap_dedicated_fn_key = \"F18\"\n\
             [keys]\nime_detect = { on = [\"F13\"] }\n\
             [[post_bypass]]\nkey = \"Ctrl+J\"\n",
        )
        .unwrap();
        let note = newly_effective_note(&c).expect("3件");
        assert!(note.contains("3 件"), "{note}");
        assert!(note.contains("F18") && note.contains("F13") && note.contains("Ctrl+J"));
    }

    #[test]
    fn post_bypass_and_optional_key_errors_are_reported() {
        let ok = PostBypassRule {
            key: "Ctrl+J".into(),
            ..Default::default()
        };
        assert!(resolve_post_bypass_key(&ok).is_ok());
        let bad = PostBypassRule {
            key: "Ctrl+NoSuch".into(),
            ..Default::default()
        };
        assert!(resolve_post_bypass_key(&bad)
            .unwrap_err()
            .contains("NoSuch"));
        let no_ctrl = PostBypassRule {
            key: "J".into(),
            ..Default::default()
        };
        assert!(resolve_post_bypass_key(&no_ctrl)
            .unwrap_err()
            .contains("Ctrl+"));
        assert_eq!(resolve_optional_key_name("x", None), Ok(None));
        assert_eq!(resolve_optional_key_name("x", Some("")), Ok(None));
        assert!(resolve_optional_key_name("x", Some("NoSuch")).is_err());
        assert!(resolve_optional_key_name("x", Some("F13"))
            .unwrap()
            .is_some());
    }
}
