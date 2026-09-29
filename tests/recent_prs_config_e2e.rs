//! 直近の config 系PR(ADR-201 段階0〜3: #335〜#338、BUG-167 #330)の通しe2e。
//!
//! 個々の関数のユニットテストは各クレート内にあるので、ここでは
//! 「ディスク上の config.toml → `AppConfig::load` → 診断 → `save_edit` → 再読込」を
//! 公開APIだけで通し、ユーザーが実際に踏む経路が壊れていないことを確認する。
//! Linux で実行できる(`cargo test --test recent_prs_config_e2e`)。

use awase::config::AppConfig;
use awase::config_save::save_edit;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new("tests/fixtures/configs").join(name)
}

fn tmp_copy(fixture_name: &str, tag: &str) -> PathBuf {
    let dst = std::env::temp_dir().join(format!(
        "awase_e2e_{tag}_{}_{fixture_name}",
        std::process::id()
    ));
    std::fs::copy(fixture(fixture_name), &dst).expect("fixture copy");
    dst
}

/// 全合成 fixture は読み込みに失敗しない(表記ゆれで起動不能にならない)。
#[test]
fn every_synthetic_fixture_loads() {
    let mut n = 0;
    for entry in std::fs::read_dir("tests/fixtures/configs").unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "toml") {
            AppConfig::load(&p).unwrap_or_else(|e| panic!("{}: {e:#}", p.display()));
            n += 1;
        }
    }
    assert!(n >= 7, "fixture が少なすぎる: {n}");
}

/// #337: 未知キーは握りつぶさず警告として残り、名前が挙がる。
#[test]
fn unknown_keys_are_reported_not_swallowed() {
    let cfg = AppConfig::load(&fixture("unknown_keys.toml")).unwrap();
    let joined = cfg.load_warnings().join("\n");
    assert!(joined.contains("no_such_option"), "warnings: {joined}");
    assert!(joined.contains("futuresection"), "warnings: {joined}");
}

/// 警告が無い健全な config では警告ゼロ(誤検知しない)。
#[test]
fn clean_config_has_no_warnings() {
    let cfg = AppConfig::load(&fixture("notation_case_and_space.toml")).unwrap();
    let joined = cfg.load_warnings().join("\n");
    assert!(
        !joined.contains("no_such") && !joined.contains("unknown"),
        "warnings: {joined}"
    );
}

/// #338: 別の項目だけ変えて保存しても、コメント・未知キー・外部編集が残る。
#[test]
fn gui_save_preserves_comments_and_unknown_keys() {
    let p = tmp_copy("unknown_keys.toml", "preserve");
    let before = std::fs::read_to_string(&p).unwrap();
    let base = AppConfig::load(&p).unwrap();
    let mut to_save = base.clone();
    to_save.general.simultaneous_threshold_ms = 77;
    save_edit(&to_save, &base, &p).unwrap();

    let after = std::fs::read_to_string(&p).unwrap();
    assert!(
        after.contains("# 未知のキー"),
        "先頭コメントが消えた:\n{after}"
    );
    assert!(
        after.contains("no_such_option = 1"),
        "未知キーが消えた:\n{after}"
    );
    assert!(
        after.contains("[futuresection]"),
        "未知テーブルが消えた:\n{after}"
    );
    assert!(after.contains("77"), "編集が反映されていない:\n{after}");
    assert_ne!(before, after);

    let reloaded = AppConfig::load(&p).unwrap();
    assert_eq!(reloaded.general.simultaneous_threshold_ms, 77);
    assert!(
        !reloaded.load_warnings().is_empty(),
        "未知キー警告は保存後も出続ける"
    );
    let _ = std::fs::remove_file(&p);
}

/// #338: 保存→再読込→再保存でファイルが変わらない(冪等)。
#[test]
fn save_is_idempotent_after_reload() {
    let p = tmp_copy("notation_japanese_names.toml", "idem");
    let base = AppConfig::load(&p).unwrap();
    let mut edited = base.clone();
    edited.general.simultaneous_threshold_ms = 65;
    save_edit(&edited, &base, &p).unwrap();
    let first = std::fs::read_to_string(&p).unwrap();

    let base2 = AppConfig::load(&p).unwrap();
    save_edit(&base2.clone(), &base2, &p).unwrap();
    assert_eq!(first, std::fs::read_to_string(&p).unwrap());
    let _ = std::fs::remove_file(&p);
}

/// #336: 日本語名・大文字小文字・空白ゆれの表記が、読み込み後も欠落せず保持される。
#[test]
fn notation_variants_survive_load() {
    let cfg = AppConfig::load(&fixture("notation_case_and_space.toml")).unwrap();
    assert!(cfg.general.engine_toggle_hotkey.is_some());
    assert_eq!(cfg.keys.engine_on.len(), 1);
    assert_eq!(cfg.keys.ime_on.len(), 1);

    let jp = AppConfig::load(&fixture("notation_japanese_names.toml")).unwrap();
    assert!(jp.general.engine_toggle_hotkey.is_some());
    assert_eq!(jp.keys.ime_toggle.len(), 2);
}

/// #336/#337: 旧表記 `[[keymap]]` は `[[keymaps]]` と合流して落ちない。
#[test]
fn legacy_keymap_merges_with_keymaps() {
    let cfg = AppConfig::load(&fixture("legacy_keymap_and_post_bypass.toml")).unwrap();
    assert_eq!(cfg.keymaps.len(), 3, "keymap 1 + keymaps 2 が合流するはず");
}

/// ファイルが無いときは既定値で起動できる(初回起動)。
#[test]
fn missing_file_falls_back_to_defaults_on_save() {
    let p = std::env::temp_dir().join(format!("awase_e2e_missing_{}.toml", std::process::id()));
    let _ = std::fs::remove_file(&p);
    let base = AppConfig::default_from_empty();
    let mut edited = base.clone();
    edited.general.simultaneous_threshold_ms = 88;
    save_edit(&edited, &base, &p).unwrap();
    assert_eq!(
        AppConfig::load(&p)
            .unwrap()
            .general
            .simultaneous_threshold_ms,
        88
    );
    let _ = std::fs::remove_file(&p);
}

/// 壊れた TOML は保存で上書きせず中止する(データ保護)。
#[test]
fn broken_file_is_not_overwritten() {
    let p = std::env::temp_dir().join(format!("awase_e2e_broken_{}.toml", std::process::id()));
    std::fs::write(&p, "[general\nthis is not toml").unwrap();
    let base = AppConfig::default_from_empty();
    let mut edited = base.clone();
    edited.general.simultaneous_threshold_ms = 55;
    assert!(save_edit(&edited, &base, &p).is_err());
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "[general\nthis is not toml"
    );
    let _ = std::fs::remove_file(&p);
}
