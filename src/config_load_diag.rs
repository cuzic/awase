//! 設定の読み込み時の診断(ADR-201 決定2): 未知のキーの検出と、近い名前の提案。
//!
//! `serde_ignored` が集めた「無視されたキーのパス」(`general.no_such`、`keymapz` 等)を、
//! ユーザー向けの警告文にする。VK の値・OS 依存は持たない(ADR-019)。

/// 撤去済みで、旧 `config.toml` に残っていても警告しないキー(パス)。
/// 撤去の経緯は `config.rs` 冒頭の NOTE と ADR-191(`apply_calibrated_mode_keys`・
/// `[[calibration]]`・`dbe_mode_key_policy`・`gji_thumb_key_ime_toggle`)。
/// 「わざと無視される」ことは `config.rs` の `test_removed_*` が確かめている。
const REMOVED_KEYS: &[&str] = &[
    "general.output_mode",
    "general.hook_mode",
    "general.conv_mode_policy",
    "general.apply_calibrated_mode_keys",
    "general.dbe_mode_key_policy",
    "general.gji_thumb_key_ime_toggle",
    // ADR-264 のメニュー読み取りの撤去(2026-10-22)。旧キー・新キーとも無警告で無視する
    // (観測だけで挙動を変えない設定だったので、通知しない)。
    "diagnostics.indicator_menu_shadow",
    // 2026-10-22 退役(ADR-276 で既定が全部 VK)。空でないときだけ通知する(`REMOVED_WITH_VALUE_NOTICE`)。
    "app_overrides.force_vk",
    // 2026-10-22 撤去(Tsf 注入経路の撤去、ADR-0004 追記。すべて VK で送る)。空でないときだけ通知する。
    "app_overrides.force_tsf",
    "indicator_evidence.menu",
    "calibration",
];

/// 撤去済みで、旧 `config.toml` に残っていると**効果があった**キー(パス)と、その通知文(ADR-207)。
///
/// `REMOVED_KEYS`(効果の無い死んだ設定。無警告で許容)とは別の表。効果があった設定を黙って
/// 消すと、ユーザーは挙動が変わった理由に気づけないため、読込時に警告して無視する
/// (`AppConfig::validate` が警告として返し、トレイに出る。設定の保存で該当行は消える)。
const REMOVED_WITH_NOTICE: &[(&str, &str)] = &[
    // ADR-206 決定4 の移行期間を SF-09（2026-10-22）で終えた。値は keys.ime_* へ移さず無視する。
    (
        "general.muhenkan_solo_tap_ime_action",
        "general.muhenkan_solo_tap_ime_action は撤去されました。値は無視されます。無変換の単独タップで IME を開閉するには、\
         keys.ime_on / keys.ime_off（または keys.ime_toggle）に「無変換」を単独で書いてください（設定画面の「IME を強制的に ON/OFF にそろえるキー」でも設定できます）。\
         config.toml から削除してください（設定画面で保存しても消えます）",
    ),
    (
        "general.henkan_solo_tap_ime_action",
        "general.henkan_solo_tap_ime_action は撤去されました。値は無視されます。変換の単独タップで IME を開閉するには、\
         keys.ime_on / keys.ime_off（または keys.ime_toggle）に「変換」を単独で書いてください（設定画面の「IME を強制的に ON/OFF にそろえるキー」でも設定できます）。\
         config.toml から削除してください（設定画面で保存しても消えます）",
    ),
    (
        "keys.engine_on_ime_key",
        "keys.engine_on_ime_key は撤去されました。値は無視されます。エンジンの ON/OFF に合わせて \
         IME のモードキーを送る機能は無くなり、代わりの設定はありません。config.toml から削除してください（設定画面で保存しても消えます）",
    ),
    (
        "keys.engine_off_ime_key",
        "keys.engine_off_ime_key は撤去されました。値は無視されます。エンジンの ON/OFF に合わせて \
         IME のモードキーを送る機能は無くなり、代わりの設定はありません。config.toml から削除してください（設定画面で保存しても消えます）",
    ),
];

/// 撤去済みで、**値が既定でないときだけ**効果があった(通知する)キー(パス)。
/// v1 の設定画面が全項目を書き出すため、既定値(`false`・`"suppress"`)はほぼ全員の config.toml に残っており、
/// キーがあるだけで通知すると全員に出てしまう。値が既定でない設定だけを通知する。
const REMOVED_WITH_VALUE_NOTICE: &[&str] = &[
    "general.gji_thumb_key_ime_toggle",
    "general.dbe_mode_key_policy",
    "app_overrides.force_vk",
    "app_overrides.force_tsf",
];

/// `app_overrides.<key>` が空でない配列か。
fn app_override_list_is_nonempty(root: &toml::Table, key: &str) -> bool {
    root.get("app_overrides")
        .and_then(toml::Value::as_table)
        .and_then(|a| a.get(key))
        .and_then(toml::Value::as_array)
        .is_some_and(|v| !v.is_empty())
}

/// 旧版の同梱 `config.toml` の `force_tsf` の値(Windows Terminal・WezTerm の 2 件)。
/// 更新した利用者の `config.toml`(Scoop は persist する)には、自分で書いていないこの値が残っているので、
/// ちょうどこの 2 件のときは通知しない(PR #809 レビュー m4。既定値は通知しない方針、上の表の doc)。
const RETIRED_DEFAULT_FORCE_TSF: &[(&str, &str)] = &[
    ("wezterm-gui.exe", "org.wezfurlong.wezterm"),
    ("WindowsTerminal.exe", "CASCADIA_HOSTING_WINDOW_CLASS"),
];

/// `app_overrides.force_tsf` が旧版の同梱の既定値(順不同、大文字小文字は区別しない)とちょうど同じか。
fn force_tsf_is_retired_default(root: &toml::Table) -> bool {
    let Some(list) = root
        .get("app_overrides")
        .and_then(toml::Value::as_table)
        .and_then(|a| a.get("force_tsf"))
        .and_then(toml::Value::as_array)
    else {
        return false;
    };
    let mut entries: Vec<(String, String)> = Vec::new();
    for v in list {
        let Some(t) = v.as_table() else {
            return false;
        };
        let field = |k: &str| {
            t.get(k)
                .and_then(toml::Value::as_str)
                .map(str::to_ascii_lowercase)
        };
        let (Some(process), Some(class)) = (field("process"), field("class")) else {
            return false;
        };
        if t.len() != 2 {
            return false;
        }
        entries.push((process, class));
    }
    let mut expected: Vec<(String, String)> = RETIRED_DEFAULT_FORCE_TSF
        .iter()
        .map(|(p, c)| (p.to_ascii_lowercase(), c.to_ascii_lowercase()))
        .collect();
    entries.sort();
    expected.sort();
    entries == expected
}

/// 値が既定でない撤去済みキーの通知文を集める。`root` は config.toml 全体の表。
#[must_use]
pub fn removed_value_notices(root: &toml::Table) -> Vec<String> {
    let general = root.get("general").and_then(toml::Value::as_table);
    let mut out = Vec::new();
    if general
        .and_then(|g| g.get("gji_thumb_key_ime_toggle"))
        .and_then(toml::Value::as_bool)
        == Some(true)
    {
        out.push(
            "general.gji_thumb_key_ime_toggle は撤去されました。値は無視されます。GJI の無変換/変換/ひらがな/カタカナキーの\
             状態依存トグルは、IME のキー設定から自動で判定します（代わりの設定はありません）。\
             config.toml から削除してください（設定画面で保存しても消えます）"
                .to_string(),
        );
    }
    if general
        .and_then(|g| g.get("dbe_mode_key_policy"))
        .and_then(toml::Value::as_str)
        .is_some_and(|v| !v.eq_ignore_ascii_case("suppress"))
    {
        out.push(
            "general.dbe_mode_key_policy は撤去されました。値は無視されます。\
             config.toml から削除してください（設定画面で保存しても消えます）"
                .to_string(),
        );
    }
    // 2026-10-22: force_vk は既定(VK)と同じになったので退役。
    if app_override_list_is_nonempty(root, "force_vk") {
        out.push(
            "app_overrides.force_vk は不要になりました(すべて VK で送るため)。値は無視されます。\
             config.toml から削除してください（設定画面で保存しても消えます）"
                .to_string(),
        );
    }
    // 2026-10-22: Tsf 注入経路(VK Sequential)を撤去したので force_tsf も不要(ADR-0004 追記)。
    if app_override_list_is_nonempty(root, "force_tsf") && !force_tsf_is_retired_default(root) {
        out.push(
            "app_overrides.force_tsf は不要になりました(すべて VK で送るため)。値は無視されます。\
             config.toml から削除してください（設定画面で保存しても消えます）"
                .to_string(),
        );
    }
    out
}

/// 撤去済みで効果があったキーなら、通知文を返す。
#[must_use]
pub fn removed_notice(path: &str) -> Option<&'static str> {
    REMOVED_WITH_NOTICE
        .iter()
        .find_map(|(p, msg)| (*p == path).then_some(*msg))
}

/// 撤去済みで効果があったキー(パス)の一覧。設定の保存が、ファイルから消す対象に使う。
pub fn removed_notice_paths() -> impl Iterator<Item = &'static str> {
    REMOVED_WITH_NOTICE
        .iter()
        .map(|(p, _)| *p)
        .chain(REMOVED_WITH_VALUE_NOTICE.iter().copied())
}

/// 撤去済みのキーか。
#[must_use]
pub fn is_removed_key(path: &str) -> bool {
    REMOVED_KEYS.contains(&path)
}

/// 2つの文字列の編集距離(Levenshtein、文字単位)。
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// `name` に近い既知の名前(編集距離2以内、または一方がもう一方の接頭辞)を返す。
#[must_use]
pub fn suggest<'a>(name: &str, known: &'a [String]) -> Option<&'a str> {
    let lower = name.to_ascii_lowercase();
    known
        .iter()
        .filter(|k| {
            let kl = k.to_ascii_lowercase();
            kl != lower
                && (edit_distance(&lower, &kl) <= 2
                    || (lower.len() >= 3 && (kl.starts_with(&lower) || lower.starts_with(&kl))))
        })
        .min_by_key(|k| edit_distance(&lower, &k.to_ascii_lowercase()))
        .map(String::as_str)
}

/// 無視されたキーの警告文を作る。`known_siblings` は同じ階層の既知のキー名(提案用、空でもよい)。
#[must_use]
pub fn unknown_key_message(path: &str, known_siblings: &[String]) -> String {
    let leaf = path.rsplit('.').next().unwrap_or(path);
    suggest(leaf, known_siblings).map_or_else(
        || format!("config.toml の \"{path}\" は未知のキーのため無視されます"),
        |s| {
            format!(
                "config.toml の \"{path}\" は未知のキーのため無視されます(\"{s}\" の間違いではありませんか)"
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn edit_distance_basic() {
        assert_eq!(edit_distance("keymap", "keymaps"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("same", "same"), 0);
    }

    #[test]
    fn suggests_close_names_only() {
        let known = names(&["general", "keys", "keymaps", "post_bypass"]);
        assert_eq!(suggest("keymapz", &known), Some("keymaps"));
        assert_eq!(suggest("post_bypas", &known), Some("post_bypass"));
        assert_eq!(suggest("generl", &known), Some("general"));
        assert_eq!(suggest("futuresection", &known), None);
        // 大文字小文字だけの違いも(完全一致でなければ)提案する
        assert_eq!(
            suggest("KEYS", &known),
            None,
            "同一の名前(大小無視)は提案しない"
        );
    }

    #[test]
    fn removed_keys_are_recognized() {
        assert!(is_removed_key("general.apply_calibrated_mode_keys"));
        assert!(is_removed_key("calibration"));
        assert!(!is_removed_key("general.no_such"));
    }

    #[test]
    fn removed_with_notice_keys_have_a_notice_and_are_not_silent() {
        for p in [
            "keys.engine_on_ime_key",
            "keys.engine_off_ime_key",
            "general.muhenkan_solo_tap_ime_action",
            "general.henkan_solo_tap_ime_action",
        ] {
            let m = removed_notice(p).expect("通知文がある");
            assert!(m.contains(p) && m.contains("撤去"), "{m}");
            assert!(!is_removed_key(p), "無警告の表に重複登録しない: {p}");
        }
        assert!(removed_notice("keys.engine_on").is_none());
        assert_eq!(removed_notice_paths().count(), 8);
    }

    #[test]
    fn removed_value_notices_only_for_non_default_values() {
        let t = |s: &str| s.parse::<toml::Table>().unwrap();
        // v1 の設定画面が書き出す既定値だけなら通知しない。
        assert!(removed_value_notices(&t(
            "[general]\ngji_thumb_key_ime_toggle = false\ndbe_mode_key_policy = \"suppress\"\n"
        ))
        .is_empty());
        assert!(removed_value_notices(&t("[general]\n")).is_empty());
        // 既定でない値は通知する。
        let m = removed_value_notices(&t("[general]\ngji_thumb_key_ime_toggle = true\n"));
        assert_eq!(m.len(), 1);
        assert!(
            m[0].contains("gji_thumb_key_ime_toggle") && m[0].contains("撤去"),
            "{m:?}"
        );
        let m = removed_value_notices(&t("[general]\ndbe_mode_key_policy = \"passthrough\"\n"));
        assert_eq!(m.len(), 1);
        assert!(m[0].contains("dbe_mode_key_policy"), "{m:?}");
        assert!(
            removed_value_notices(&t("[diagnostics]\nindicator_menu_shadow = true\n")).is_empty()
        );
        let m = removed_value_notices(&t(
            "[app_overrides]\nforce_vk = [{ process = \"a.exe\", class = \"A\" }]\n",
        ));
        assert_eq!(m.len(), 1);
        assert!(m[0].contains("force_vk") && m[0].contains("不要"), "{m:?}");
        assert!(removed_value_notices(&t("[app_overrides]\nforce_vk = []\n")).is_empty());
        let m = removed_value_notices(&t(
            "[app_overrides]\nforce_tsf = [{ process = \"a.exe\", class = \"A\" }]\n",
        ));
        assert_eq!(m.len(), 1);
        assert!(m[0].contains("force_tsf") && m[0].contains("不要"), "{m:?}");
        assert!(removed_value_notices(&t("[app_overrides]\nforce_tsf = []\n")).is_empty());
        // 旧版の同梱の既定値(2 件、順不同)だけなら通知しない(m4)。1 件でも違えば通知する。
        assert!(removed_value_notices(&t(
            "[app_overrides]\nforce_tsf = [\n  { process = \"WindowsTerminal.exe\", class = \"CASCADIA_HOSTING_WINDOW_CLASS\" },\n  { process = \"wezterm-gui.exe\", class = \"org.wezfurlong.wezterm\" },\n]\n"
        ))
        .is_empty());
        assert_eq!(
            removed_value_notices(&t(
                "[app_overrides]\nforce_tsf = [\n  { process = \"wezterm-gui.exe\", class = \"org.wezfurlong.wezterm\" },\n]\n"
            ))
            .len(),
            1
        );
        for p in REMOVED_WITH_VALUE_NOTICE {
            assert!(is_removed_key(p), "ログ警告は出さない表にも載せる: {p}");
        }
    }

    #[test]
    fn message_includes_suggestion_when_close() {
        let m = unknown_key_message("keymapz", &names(&["keymaps"]));
        assert!(m.contains("keymapz") && m.contains("keymaps"), "{m}");
        let m = unknown_key_message("general.zzz", &names(&["threshold"]));
        assert!(!m.contains("間違い"), "{m}");
    }
}
