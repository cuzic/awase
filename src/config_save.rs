//! 設定の保存（ADR-201 決定3）: `toml_edit` で、読み込んだときの値（`base`）から
//! GUI が変えた項目だけを、保存の直前にディスクから読み直した文書（`disk`）へ書く。
//!
//! - **三者比較**: `base`（読み込んだときの生の値）と `to_save`（保存しようとしている値）の
//!   差だけを書く。`disk` は書き込み先としてのみ使い、比較の基準にはしない
//!   （二者比較は外部エディタでの編集を古い値で上書きする＝2026-09-05 の stale
//!   read-modify-write の復活）。
//! - 配列（配列の表を含む）は差があれば配列ごと置き換える。
//! - GUI が変えた結果が既定値（`AppConfig::from_toml_str("")`）と等しければキーを消す。
//! - alias の旧名（[`KEY_ALIASES`]）は、正規名を書く・消すときに文書から消す。
//!   `keymaps` を書くときは `[[keymap]]` も消す（合流した結果を `keymaps` に一本化する）。
//!
//! **`None` を保存できない項目**: キーが無いときの既定値が `Some` の `Option` 項目
//! （`general.ngram_file`、`keys.engine_off_solo_repeat`）は、TOML に null が無いため
//! `None` をキーの削除でしか表せず、削除すると読み込み時に既定値の `Some` に戻る。
//! これは全体再シリアライズだった従来から続く制約で、この保存方式でも変えない
//! （`engine_off_solo_repeat` は空文字 `Some("")` で「無効」を表して回避している）。

use anyhow::{bail, Context, Result};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table};

use crate::config::AppConfig;

/// alias を持つ項目: (親のパス（`""` は最上位）, 正規名, 文書から消す旧名)。
/// `#[serde(alias = ...)]` を項目に足したら、ここにも足す（ガードテストが件数を照合する）。
pub(crate) const KEY_ALIASES: &[(&str, &str, &str)] = &[
    ("keys", "engine_off_solo_repeat", "engine_off_solo_triple"),
    // `[[keymap]]` は alias ではなく `legacy_keymap`（rename）で受けて合流させる（決定5）。
    ("", "keymaps", "keymap"),
];

/// 差分の1件。`value` が `None` ならキーの削除。
#[derive(Debug, PartialEq)]
pub(crate) struct Edit {
    pub path: Vec<String>,
    pub value: Option<toml::Value>,
}

fn to_table(config: &AppConfig) -> Result<toml::Table> {
    toml::Table::try_from(config).context("Failed to serialize config")
}

/// `base` と `to_save` の差（`to_save` が `base` と違う項目）を集める。
/// `default` は「キーが無いときに読まれる値」で、変えた結果がこれに等しければ削除にする。
pub(crate) fn diff(base: &AppConfig, to_save: &AppConfig) -> Result<Vec<Edit>> {
    let default = to_table(&AppConfig::default_from_empty())?;
    let mut edits = Vec::new();
    diff_tables(
        &to_table(base)?,
        &to_table(to_save)?,
        Some(&default),
        &mut Vec::new(),
        &mut edits,
    );
    Ok(edits)
}

fn diff_tables(
    base: &toml::Table,
    to_save: &toml::Table,
    default: Option<&toml::Table>,
    path: &mut Vec<String>,
    out: &mut Vec<Edit>,
) {
    let mut keys: Vec<&String> = base.keys().chain(to_save.keys()).collect();
    keys.sort();
    keys.dedup();
    for k in keys {
        let (b, t) = (base.get(k), to_save.get(k));
        if b == t {
            continue;
        }
        path.push(k.clone());
        match (b, t) {
            (Some(toml::Value::Table(bt)), Some(toml::Value::Table(tt))) => {
                let d = default
                    .and_then(|d| d.get(k))
                    .and_then(toml::Value::as_table);
                diff_tables(bt, tt, d, path, out);
            }
            (_, Some(tv)) => {
                let is_default = default.and_then(|d| d.get(k)) == Some(tv);
                out.push(Edit {
                    path: path.clone(),
                    value: if is_default { None } else { Some(tv.clone()) },
                });
            }
            (_, None) => out.push(Edit {
                path: path.clone(),
                value: None,
            }),
        }
        path.pop();
    }
}

/// `toml::Value` を `toml_edit` の項目にする。表は `position` を持たない新しい表として組む
/// （別の文書から取った表を差し込むと、出力上の並びが崩れうる）。
fn build_item(v: &toml::Value) -> Item {
    match v {
        toml::Value::Table(t) => {
            let mut table = Table::new();
            for (k, c) in t {
                table.insert(k, build_item(c));
            }
            Item::Table(table)
        }
        toml::Value::Array(a) if !a.is_empty() && a.iter().all(toml::Value::is_table) => {
            let mut aot = ArrayOfTables::new();
            for e in a {
                if let Item::Table(t) = build_item(e) {
                    aot.push(t);
                }
            }
            Item::ArrayOfTables(aot)
        }
        leaf => format!("v = {leaf}")
            .parse::<DocumentMut>()
            .ok()
            .and_then(|mut d| d.remove("v"))
            .unwrap_or_default(),
    }
}

/// `path` の親の表（ドット付きキー・インライン表・`[keys]` のどれでも）を引く。
/// `create` なら無い表を作る。戻り値の `bool` は、インライン表の中か。
fn parent_mut<'a>(
    doc: &'a mut DocumentMut,
    parents: &[String],
    create: bool,
) -> Option<(&'a mut dyn toml_edit::TableLike, bool)> {
    let mut cur: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    let mut inline = false;
    for seg in parents {
        if !cur.contains_key(seg) {
            if !create {
                return None;
            }
            cur.insert(seg, Item::Table(Table::new()));
        }
        let item = cur.get_mut(seg)?;
        inline |= item.is_value();
        cur = item.as_table_like_mut()?;
    }
    Some((cur, inline))
}

fn remove_key(doc: &mut DocumentMut, parents: &[String], key: &str) {
    if let Some((t, _)) = parent_mut(doc, parents, false) {
        t.remove(key);
    }
}

/// 差分を文書へ適用する。
pub(crate) fn apply_edits(doc: &mut DocumentMut, edits: &[Edit]) {
    for e in edits {
        let Some((key, parents)) = e.path.split_last() else {
            continue;
        };
        let parent_path = parents.join(".");
        // alias の旧名を消す（残すと次の読み込みで serde が重複として失敗する）。
        for (p, canonical, old) in KEY_ALIASES {
            if *p == parent_path && canonical == key {
                remove_key(doc, parents, old);
            }
        }
        let Some(value) = &e.value else {
            remove_key(doc, parents, key);
            continue;
        };
        let Some((table, inline)) = parent_mut(doc, parents, true) else {
            continue; // 途中の項目が表でない（読めない形）。書かずに残す。
        };
        let mut item = build_item(value);
        if inline {
            item = match item.into_value() {
                Ok(v) => Item::Value(v),
                Err(i) => i,
            };
        }
        // 変えた値の行末コメント（`x = 1 # 説明`）は残す。
        if let (Some(Item::Value(old)), Item::Value(new)) = (table.get_mut(key), &mut item) {
            *new.decor_mut() = old.decor().clone();
            *old = new.clone();
            continue;
        }
        table.insert(key, item);
    }
}

/// `path` の `disk` を読み、`base` から `to_save` への差だけを書いて保存する。
///
/// - ファイルが存在しない: 空の文書から始め、`to_save` のうち既定値と違う項目をすべて書く。
/// - TOML として読めない・読み取りに失敗: 保存を中止する（外部の書きかけを上書きしない）。
///
/// # Errors
///
/// 上記の中止、シリアライズ、書き込みの失敗。
pub fn save_edit(to_save: &AppConfig, base: &AppConfig, path: &std::path::Path) -> Result<()> {
    let (mut doc, base) = match std::fs::read_to_string(path) {
        Ok(text) => match text.parse::<DocumentMut>() {
            Ok(doc) => (doc, base.clone()),
            Err(e) => bail!("ファイルが外部で編集されていて読めません（{e}）"),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (DocumentMut::new(), AppConfig::default_from_empty())
        }
        Err(e) => bail!("ファイルが外部で編集されていて読めません（{e}）"),
    };
    apply_edits(&mut doc, &diff(&base, to_save)?);
    crate::fs_atomic::write_atomic(path, doc.to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "awase_cfgsave_{name}_{}_{}.toml",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn write(name: &str, text: &str) -> PathBuf {
        let p = tmp(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    fn read(p: &std::path::Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    const RICH: &str = "# 先頭コメント\n\
        [general]\n\
        simultaneous_threshold_ms = 80 # 行末コメント\n\
        removed_or_unknown = 1\n\
        \n\
        [keys]\n\
        engine_on = [\"Ctrl+A\", \"Ctrl+B\"]\n\
        engine_off_solo_triple = \"VK_F13\"\n\
        \n\
        [keys.ime_detect]\n\
        on = [\"VK_F16\"]\n\
        \n\
        [[keymap]]\n\
        from = \"Ctrl+VK_I\"\n\
        to = [\"F7\"]\n";

    // (1) 何も編集せずに保存するとバイト単位で変わらない。
    #[test]
    fn unchanged_save_is_byte_identical() {
        let p = write("identical", RICH);
        let base = AppConfig::load(&p).unwrap();
        save_edit(&base.clone(), &base, &p).unwrap();
        assert_eq!(read(&p), RICH);
        let _ = std::fs::remove_file(&p);
    }

    // (2) 外部エディタでの画面に無い項目の編集が、別の項目だけ変えた保存で消えない。
    #[test]
    fn external_edit_of_untouched_item_survives_save() {
        let p = write("stale", RICH);
        let base = AppConfig::load(&p).unwrap();
        // GUI を開いた後に外部エディタで ime_detect を書き換える。
        std::fs::write(&p, RICH.replace("VK_F16", "VK_F17")).unwrap();
        let mut to_save = base.clone();
        to_save.general.simultaneous_threshold_ms = 90;
        save_edit(&to_save, &base, &p).unwrap();
        let saved = AppConfig::load(&p).unwrap();
        assert_eq!(saved.keys.ime_detect.on, vec!["VK_F17".to_string()]);
        assert_eq!(saved.general.simultaneous_threshold_ms, 90);
        let text = read(&p);
        assert!(text.contains("# 先頭コメント") && text.contains("removed_or_unknown = 1"));
        assert!(text.contains("90 # 行末コメント"), "{text}");
        let _ = std::fs::remove_file(&p);
    }

    // 同じ項目を両方が変えたら GUI が勝つ。
    #[test]
    fn gui_wins_when_both_change_same_item() {
        let p = write("both", RICH);
        let base = AppConfig::load(&p).unwrap();
        std::fs::write(&p, RICH.replace("= 80", "= 70")).unwrap();
        let mut to_save = base.clone();
        to_save.general.simultaneous_threshold_ms = 90;
        save_edit(&to_save, &base, &p).unwrap();
        assert_eq!(
            AppConfig::load(&p)
                .unwrap()
                .general
                .simultaneous_threshold_ms,
            90
        );
        let _ = std::fs::remove_file(&p);
    }

    // (3) トレイの save_auto_start が間に書いた auto_start が GUI の保存で消えない。
    #[test]
    fn auto_start_written_in_between_survives_gui_save() {
        let p = write("autostart", RICH);
        let base = AppConfig::load(&p).unwrap();
        assert!(AppConfig::save_auto_start(&p, "disabled").is_some());
        let mid = read(&p);
        assert!(mid.contains("auto_start = \"disabled\""), "{mid}");
        assert!(mid.contains("# 先頭コメント") && mid.contains("removed_or_unknown = 1"));
        let mut to_save = base.clone();
        to_save.general.simultaneous_threshold_ms = 90;
        save_edit(&to_save, &base, &p).unwrap();
        assert_eq!(AppConfig::load(&p).unwrap().general.auto_start, "disabled");
        let _ = std::fs::remove_file(&p);
    }

    // (4) alias の旧名だけのファイルでその項目を編集して保存 → 再読み込みできる。
    #[test]
    fn alias_old_name_is_removed_when_canonical_is_written() {
        let p = write("alias", RICH);
        let base = AppConfig::load(&p).unwrap();
        assert_eq!(base.keys.engine_off_solo_repeat.as_deref(), Some("VK_F13"));
        let mut to_save = base.clone();
        to_save.keys.engine_off_solo_repeat = Some("VK_F14".into());
        save_edit(&to_save, &base, &p).unwrap();
        let text = read(&p);
        assert!(!text.contains("engine_off_solo_triple"), "{text}");
        let saved = AppConfig::load(&p).expect("Dangerous(重複)にならない");
        assert_eq!(saved.keys.engine_off_solo_repeat.as_deref(), Some("VK_F14"));
        let _ = std::fs::remove_file(&p);
    }

    // (5) [[keymap]] だけのファイルで keymaps を1つ編集して保存 → 規則の数が変わらない。
    #[test]
    fn legacy_keymap_is_consolidated_when_keymaps_written() {
        let p = write("keymap", RICH);
        let base = AppConfig::load(&p).unwrap();
        assert_eq!(base.keymaps.len(), 1);
        let mut to_save = base.clone();
        to_save.keymaps[0].to = vec!["F8".into()];
        save_edit(&to_save, &base, &p).unwrap();
        let text = read(&p);
        assert!(!text.contains("[[keymap]]"), "{text}");
        let saved = AppConfig::load(&p).unwrap();
        assert_eq!(saved.keymaps.len(), 1);
        assert_eq!(saved.keymaps[0].to, vec!["F8".to_string()]);
        // もう一度保存しても倍にならない。
        save_edit(&saved.clone(), &saved, &p).unwrap();
        assert_eq!(AppConfig::load(&p).unwrap().keymaps.len(), 1);
        let _ = std::fs::remove_file(&p);
    }

    // keymaps を書かない保存では [[keymap]] はそのまま残る。
    #[test]
    fn legacy_keymap_kept_when_keymaps_untouched() {
        let p = write("keymap_kept", RICH);
        let base = AppConfig::load(&p).unwrap();
        let mut to_save = base.clone();
        to_save.general.simultaneous_threshold_ms = 90;
        save_edit(&to_save, &base, &p).unwrap();
        assert!(read(&p).contains("[[keymap]]"));
        let _ = std::fs::remove_file(&p);
    }

    // (6) 保存直前にファイルが無い: 既定値と違う項目をすべて書く。
    #[test]
    fn missing_file_writes_all_non_default_items() {
        let p = tmp("missing");
        let base = AppConfig::default_from_empty();
        let mut to_save = base.clone();
        to_save.general.simultaneous_threshold_ms = 55;
        save_edit(
            &to_save,
            &AppConfig::load(&write("other", RICH)).unwrap(),
            &p,
        )
        .unwrap();
        // base（GUI が知っている RICH の値）でなく既定値との差を書く。
        let saved = AppConfig::load(&p).unwrap();
        assert_eq!(saved.general.simultaneous_threshold_ms, 55);
        assert_eq!(
            saved.keys.engine_off_solo_repeat,
            base.keys.engine_off_solo_repeat
        );
        assert!(!read(&p).contains("engine_on"), "既定値は書かない");
        let _ = std::fs::remove_file(&p);
    }

    // (6) TOML として壊れている / 読み取りに失敗: 保存を中止し、ファイルには触れない。
    #[test]
    fn broken_or_unreadable_file_aborts_without_touching() {
        let p = write("broken", "[general\nsimultaneous_threshold_ms = ");
        let base = AppConfig::default_from_empty();
        let mut to_save = base.clone();
        to_save.general.simultaneous_threshold_ms = 55;
        let err = save_edit(&to_save, &base, &p).unwrap_err().to_string();
        assert!(err.contains("外部で編集されていて読めません"), "{err}");
        assert_eq!(read(&p), "[general\nsimultaneous_threshold_ms = ");
        let _ = std::fs::remove_file(&p);
        // 読み取りの失敗（NotFound 以外）: ディレクトリを指す。
        let dir = std::env::temp_dir();
        let err = save_edit(&to_save, &base, &dir).unwrap_err().to_string();
        assert!(err.contains("外部で編集されていて読めません"), "{err}");
    }

    // (9) 既定値に戻した項目のキーが消える。コメントと未知キーは残る。
    #[test]
    fn reverting_to_default_removes_key_but_keeps_comments_and_unknown() {
        let p = write("revert", RICH);
        let base = AppConfig::load(&p).unwrap();
        let mut to_save = base.clone();
        to_save.general.simultaneous_threshold_ms = AppConfig::default_from_empty()
            .general
            .simultaneous_threshold_ms;
        save_edit(&to_save, &base, &p).unwrap();
        let text = read(&p);
        assert!(!text.contains("simultaneous_threshold_ms"), "{text}");
        assert!(text.contains("# 先頭コメント") && text.contains("removed_or_unknown = 1"));
        let _ = std::fs::remove_file(&p);
    }

    // GUI が変えていない項目は、既定値と同じ明示値でも触らない。
    #[test]
    fn untouched_explicit_default_is_left_alone() {
        let text = "[general]\nsimultaneous_threshold_ms = 100\nauto_start = \"enabled\"\n";
        let p = write("explicit_default", text);
        let base = AppConfig::load(&p).unwrap();
        let mut to_save = base.clone();
        to_save.general.auto_start = "disabled".into();
        save_edit(&to_save, &base, &p).unwrap();
        assert!(read(&p).contains("simultaneous_threshold_ms = 100"));
        let _ = std::fs::remove_file(&p);
    }

    // (10) 配列の一部の要素だけ変えても配列ごと置換される。
    #[test]
    fn array_is_replaced_as_a_whole() {
        let p = write("array", RICH);
        let base = AppConfig::load(&p).unwrap();
        let mut to_save = base.clone();
        to_save.keys.engine_on[1] = "Ctrl+C".into();
        save_edit(&to_save, &base, &p).unwrap();
        let saved = AppConfig::load(&p).unwrap();
        assert_eq!(saved.keys.engine_on, vec!["Ctrl+A", "Ctrl+C"]);
        let _ = std::fs::remove_file(&p);
    }

    // ドット付きキー・インライン表・表が無い場合のどれでも同じパスを引く。
    #[test]
    fn dotted_inline_and_missing_tables_are_resolved() {
        let text = "general.simultaneous_threshold_ms = 80\n\
                    keys = { engine_on = [\"Ctrl+A\"] }\n";
        let p = write("shapes", text);
        let base = AppConfig::load(&p).unwrap();
        let mut to_save = base.clone();
        to_save.general.simultaneous_threshold_ms = 90;
        to_save.keys.engine_on = vec!["Ctrl+Z".into()];
        to_save.app_overrides.input_relay_apps = vec!["x.exe".into()]; // 表が無い
        save_edit(&to_save, &base, &p).unwrap();
        let saved = AppConfig::load(&p).unwrap();
        assert_eq!(saved.general.simultaneous_threshold_ms, 90);
        assert_eq!(saved.keys.engine_on, vec!["Ctrl+Z"]);
        assert_eq!(saved.app_overrides.input_relay_apps, vec!["x.exe"]);
        assert!(
            saved.load_warnings().is_empty(),
            "{:?}",
            saved.load_warnings()
        );
        let _ = std::fs::remove_file(&p);
    }

    // `None` を保存できない項目（既定値が Some）: None はキーの削除になり既定値に戻る。
    // 従来（全体再シリアライズ）からの制約で、この保存方式でも変えない。
    #[test]
    fn none_cannot_be_saved_for_items_whose_default_is_some() {
        let default = AppConfig::default_from_empty();
        assert!(default.general.ngram_file.is_some(), "前提: 既定値は Some");
        let p = write("none", "[general]\nngram_file = \"custom.tsv\"\n");
        let base = AppConfig::load(&p).unwrap();
        let mut to_save = base.clone();
        to_save.general.ngram_file = None;
        save_edit(&to_save, &base, &p).unwrap();
        assert!(!read(&p).contains("ngram_file"));
        assert_eq!(
            AppConfig::load(&p).unwrap().general.ngram_file,
            default.general.ngram_file,
            "None は既定の Some に戻る（既知の制約）"
        );
        let _ = std::fs::remove_file(&p);
    }

    // 撤去済みキー・未知キーは保存後も残る（再読み込みで未知キーの警告が出る）。
    #[test]
    fn unknown_keys_survive_and_are_still_reported() {
        let p = write("unknown", "[general]\nno_such_key = 1\n");
        let base = AppConfig::load(&p).unwrap();
        let mut to_save = base.clone();
        to_save.general.simultaneous_threshold_ms = 90;
        save_edit(&to_save, &base, &p).unwrap();
        assert!(!AppConfig::load(&p).unwrap().load_warnings().is_empty());
        let _ = std::fs::remove_file(&p);
    }

    // alias の一覧が `#[serde(alias)]` の実数と食い違わない（足し忘れの検出）。
    #[test]
    fn key_aliases_table_matches_serde_aliases() {
        let n = include_str!("config.rs")
            .lines()
            .filter(|l| l.trim_start().starts_with("#[serde(alias"))
            .count();
        let non_keymap = KEY_ALIASES.iter().filter(|a| a.2 != "keymap").count();
        assert_eq!(
            n, non_keymap,
            "`#[serde(alias)]` を足したら KEY_ALIASES にも足す"
        );
    }
}
