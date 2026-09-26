//! ADR-201 段階0: 設定のキー名解決を検証するテスト(文書・サンプルの例、合成 config)。
//!
//! 実装(段階1〜3)より先にテストを入れ、**現状の失敗を「既知の失敗の一覧」として
//! データで持ち、その一覧どおりに失敗することを期待値にする**。段階1で `from_name`
//! を寛容にすると、一覧に残った項目が「もう失敗しない」ためテストが落ちる。そのとき
//! 一覧から該当項目を消す(消し忘れると落ち続けるので、一覧は必ず空へ向かう)。
//!
//! - 例のブロックは、対象ファイル(`config.toml`/`README.md`/`docs/usage*.html`)に
//!   `example-begin`/`example-end`(HTML は `data-example` 属性)の目印で囲んである。
//!   目印の書式は `extract_*` の doc を参照。ADR は対象外(当時の記録のため)。
//! - 例の読み込みは [`load_config_text`] 1か所にまとめてある。段階2で
//!   `AppConfig::from_toml_str` ができたら、この関数だけ差し替える。
//! - 合成 config は `tests/fixtures/configs/`(実物の不具合報告 config は置かない。決定4-3)。
//! - GUI の候補 × 読み手のテストは `awase-settings` 側(候補表が同 crate の非公開定数のため)。
//!
//! Linux でも走る(`cargo nextest run --workspace --lib`)。`parse_hotkey` だけは
//! `#[cfg(windows)]` なので、Linux では同じ前置き処理の等価コードで確かめ、実物は
//! windows-build ジョブの実行に任せる(`hotkey_readable`)。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use awase::config::{AppConfig, KeysConfig};
use awase::types::VkCode;

use crate::state::alt_impersonation::resolve_thumb_key;
use crate::vk::{parse_key_combo, VkCodeExt};

// ── 既知の失敗の一覧(段階1・2で空にしていく) ─────────────────────────────

/// 文書・サンプルの例の「既知の失敗」。書式は `"<例の名前>|<種別>|<詳細>"`。
///
/// 種別: `unparsable`(TOML/型として読めない)、`unknown_top_level`(未知の最上位キー。
/// 今は無警告で捨てられる)、`unresolved`(読み込めるが、実際の読み手が解決できず
/// 無言で無効になる)、`default_mismatch`(「デフォルト値」と書いた例が
/// `KeysConfig::default()` と違う)。
///
/// 段階1(`from_name` の寛容化)で `unresolved` の行が消え、段階2(`keymap` の合流)で
/// `unknown_top_level` の行が消える。`default_mismatch` は文書の誤りで、別途直す。
const DOC_KNOWN_FAILURES: &[&str] = &[
    // 文書の「デフォルト値」の例が実際の既定値(`VK_INSERT`)と違う(文書が古い。既定は
    // ADR-199 以前に無変換から Insert へ変わった)。文書側を直すときに消す。
    "usage-ja-2|default_mismatch|keys.engine_off_solo_repeat",
    "usage-ja-12|default_mismatch|keys.engine_off_solo_repeat",
    "usage-en-2|default_mismatch|keys.engine_off_solo_repeat",
    "usage-en-10|default_mismatch|keys.engine_off_solo_repeat",
];

/// 合成 config の「既知の失敗」。書式は `"<ファイル名>|<種別>|<詳細>"`(種別は上と同じ)。
const FIXTURE_KNOWN_FAILURES: &[&str] = &[
    // 旧表記 `[[keymap]]`(実際は `keymaps`)。段階2で `keymaps` へ合流する。
    "legacy_keymap_and_post_bypass.toml|unknown_top_level|keymap",
    // ── 未知のキー・存在しないキー名(意図して解決できない値)。
    // 段階2で `validate()` の警告に出るようになったら、警告数の基準を上げて一覧から消す ──
    "unknown_keys.toml|unknown_top_level|calibration",
    "unknown_keys.toml|unknown_top_level|futuresection",
    "unknown_keys.toml|unresolved|general.engine_toggle_hotkey=Ctrl+Shift+NoSuchKey",
    "unknown_keys.toml|unresolved|keys.ime_on=Ctrl+NoSuchKey",
];

/// 文書の例の名前と、`validate()` の警告数の基準(増えてはいけない)。
const DOC_BASELINE: &[(&str, usize)] = &[
    ("config-keys-engine", 0),
    ("config-ime-detect", 0),
    ("config-app-overrides", 0),
    ("config-post-bypass", 0),
    ("readme-minimal", 0),
    ("readme-app-overrides", 0),
    ("usage-ja-1", 0),
    ("usage-ja-2", 0),
    ("usage-ja-3", 0),
    ("usage-ja-4", 0),
    ("usage-ja-5", 0),
    ("usage-ja-6", 0),
    ("usage-ja-7", 0),
    ("usage-ja-8", 0),
    ("usage-ja-9", 0),
    ("usage-ja-10", 0),
    ("usage-ja-11", 0),
    ("usage-ja-12", 0),
    ("usage-ja-13", 0),
    ("usage-en-1", 0),
    ("usage-en-2", 0),
    ("usage-en-3", 0),
    ("usage-en-4", 0),
    ("usage-en-5", 0),
    ("usage-en-6", 0),
    ("usage-en-7", 0),
    ("usage-en-8", 0),
    ("usage-en-9", 0),
    ("usage-en-10", 0),
];

/// 合成 config のファイル名と、`validate()` の警告数の基準(増えてはいけない)。
const FIXTURE_BASELINE: &[(&str, usize)] = &[
    ("alt_impersonation_and_legacy_alias.toml", 2),
    ("alt_impersonation_lowercase.toml", 0),
    ("legacy_keymap_and_post_bypass.toml", 0),
    ("notation_case_and_space.toml", 0),
    ("notation_japanese_names.toml", 0),
    ("notation_prefix.toml", 0),
    ("unknown_keys.toml", 0),
];

// ── 読み込みと、実際の読み手の再現 ───────────────────────────────────────────

/// 設定テキストを `AppConfig` として読み込む。**読み込みの入口はここ1か所**。
/// 段階2で `AppConfig::from_toml_str`(`keymap` の合流・`load_warnings` を含む)ができたら
/// 差し替える。
fn load_config_text(text: &str) -> Result<AppConfig, String> {
    toml::from_str(text).map_err(|e| e.to_string())
}

/// ホットキー文字列を、実際の読み手 `parse_hotkey` と同じ経路で解釈できるか。
/// Windows では実物を呼ぶ。Linux では `parse_hotkey` が薄く包んでいる
/// `parse_key_combo`(修飾キー解釈は `interpret_combo` 1関数)で確かめる。
fn hotkey_readable(s: &str) -> bool {
    #[cfg(windows)]
    {
        crate::vk::parse_hotkey(s).is_some()
    }
    #[cfg(not(windows))]
    {
        parse_key_combo(s).is_some()
    }
}

/// 読み込んだ設定の各キー項目を、**実際の読み手**で解決してみて、解決できないものを
/// `"<項目>=<値>"` で返す。実行時にはこれらが無言(または `tracing::warn!` のみ)で無効になる。
///
/// 読み手の対応(ADR-201 背景):
/// - `left/right_thumb_key`: `resolve_thumb_key`(`bootstrap.rs`)
/// - `engine_toggle_hotkey`: `parse_hotkey`(`register_toggle`)
/// - `keys.engine_on/off`・`ime_on/off/toggle`: `parse_key_combo`(`parse_key_combos`)
/// - `keys.ime_detect.*`・`engine_off_solo_repeat`・`engine_on/off_ime_key`・
///   `muhenkan_solo_tap_dedicated_fn_key`: `from_name`
/// - `[[post_bypass]] key`: `parse_key_combo` + Ctrl 必須(`bootstrap.rs` の `filter_map`)
/// - `[[keymaps]] from`: `parse_key_combo`、`to`: `from_name`
fn unresolved_keys(c: &AppConfig) -> Vec<String> {
    let mut out = Vec::new();
    for (name, v) in [
        ("general.left_thumb_key", &c.general.left_thumb_key),
        ("general.right_thumb_key", &c.general.right_thumb_key),
    ] {
        if resolve_thumb_key(v).is_none() {
            out.push(format!("{name}={v}"));
        }
    }
    if let Some(h) = &c.general.engine_toggle_hotkey {
        if !hotkey_readable(h) {
            out.push(format!("general.engine_toggle_hotkey={h}"));
        }
    }
    if let Some(k) = &c.general.muhenkan_solo_tap_dedicated_fn_key {
        if VkCode::from_name(k).is_none() {
            out.push(format!("general.muhenkan_solo_tap_dedicated_fn_key={k}"));
        }
    }
    let combos: [(&str, &Vec<String>); 5] = [
        ("keys.engine_on", &c.keys.engine_on),
        ("keys.engine_off", &c.keys.engine_off),
        ("keys.ime_on", &c.keys.ime_on),
        ("keys.ime_off", &c.keys.ime_off),
        ("keys.ime_toggle", &c.keys.ime_toggle),
    ];
    for (name, list) in combos {
        for s in list {
            if parse_key_combo(s).is_none() {
                out.push(format!("{name}={s}"));
            }
        }
    }
    let singles: [(&str, &Vec<String>); 3] = [
        ("keys.ime_detect.toggle", &c.keys.ime_detect.toggle),
        ("keys.ime_detect.on", &c.keys.ime_detect.on),
        ("keys.ime_detect.off", &c.keys.ime_detect.off),
    ];
    for (name, list) in singles {
        for s in list {
            if VkCode::from_name(s).is_none() {
                out.push(format!("{name}={s}"));
            }
        }
    }
    for (name, v) in [
        (
            "keys.engine_off_solo_repeat",
            &c.keys.engine_off_solo_repeat,
        ),
        ("keys.engine_on_ime_key", &c.keys.engine_on_ime_key),
        ("keys.engine_off_ime_key", &c.keys.engine_off_ime_key),
    ] {
        if let Some(s) = v.as_deref().filter(|s| !s.is_empty()) {
            if VkCode::from_name(s).is_none() {
                out.push(format!("{name}={s}"));
            }
        }
    }
    for (i, r) in c.post_bypass.iter().enumerate() {
        if !parse_key_combo(&r.key).is_some_and(|k| k.ctrl) {
            out.push(format!("post_bypass[{i}].key={}", r.key));
        }
    }
    for (i, r) in c.keymaps.iter().enumerate() {
        if parse_key_combo(&r.from).is_none() {
            out.push(format!("keymaps[{i}].from={}", r.from));
        }
        for to in &r.to {
            if VkCode::from_name(to).is_none() {
                out.push(format!("keymaps[{i}].to={to}"));
            }
        }
    }
    out
}

/// 未知の最上位キー(`[[keymap]]` など)。今は `serde` が黙って捨てる。
/// 入れ子の未知キー(`[general] no_such = 1`)は、`serde_ignored`(段階2)が入るまで
/// 検出できないので対象外。
fn unknown_top_level(text: &str) -> Vec<String> {
    let Ok(raw) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let known: BTreeSet<String> = toml::Table::try_from(AppConfig::default())
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default();
    raw.keys()
        .filter(|k| !known.contains(*k))
        .cloned()
        .collect()
}

/// 「デフォルト値」と書いた `[keys]` の例が `KeysConfig::default()` と一致しない項目。
/// 例に書かれた項目(別名を含む)だけを比べる。
fn default_mismatches(text: &str, c: &AppConfig) -> Vec<String> {
    let Ok(raw) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let Some(keys) = raw.get("keys").and_then(toml::Value::as_table) else {
        return Vec::new();
    };
    let d = KeysConfig::default();
    let has = |names: &[&str]| names.iter().any(|n| keys.contains_key(*n));
    let mut out = Vec::new();
    let checks: [(&str, &[&str], String, String); 6] = [
        (
            "engine_on",
            &["engine_on"],
            format!("{:?}", c.keys.engine_on),
            format!("{:?}", d.engine_on),
        ),
        (
            "engine_off",
            &["engine_off"],
            format!("{:?}", c.keys.engine_off),
            format!("{:?}", d.engine_off),
        ),
        (
            "ime_on",
            &["ime_on"],
            format!("{:?}", c.keys.ime_on),
            format!("{:?}", d.ime_on),
        ),
        (
            "ime_off",
            &["ime_off"],
            format!("{:?}", c.keys.ime_off),
            format!("{:?}", d.ime_off),
        ),
        (
            "ime_toggle",
            &["ime_toggle"],
            format!("{:?}", c.keys.ime_toggle),
            format!("{:?}", d.ime_toggle),
        ),
        (
            "engine_off_solo_repeat",
            &["engine_off_solo_repeat", "engine_off_solo_triple"],
            format!("{:?}", c.keys.engine_off_solo_repeat),
            format!("{:?}", d.engine_off_solo_repeat),
        ),
    ];
    for (field, names, actual, default) in checks {
        if has(names) && actual != default {
            out.push(format!("keys.{field}"));
        }
    }
    out
}

/// 1件の結果: `validate()` の警告数と、失敗の一覧(`"<種別>|<詳細>"`)。
struct Analysis {
    warnings: usize,
    failures: Vec<String>,
}

fn analyze(text: &str, check_defaults: bool) -> Analysis {
    let config = match load_config_text(text) {
        Ok(c) => c,
        Err(e) => {
            return Analysis {
                warnings: 0,
                failures: vec![format!("unparsable|{}", e.lines().next().unwrap_or(""))],
            };
        }
    };
    let (_, warnings) = config.clone().validate();
    let mut failures = Vec::new();
    failures.extend(
        unknown_top_level(text)
            .into_iter()
            .map(|k| format!("unknown_top_level|{k}")),
    );
    failures.extend(
        unresolved_keys(&config)
            .into_iter()
            .map(|k| format!("unresolved|{k}")),
    );
    if check_defaults {
        failures.extend(
            default_mismatches(text, &config)
                .into_iter()
                .map(|k| format!("default_mismatch|{k}")),
        );
    }
    Analysis {
        warnings: warnings.len(),
        failures,
    }
}

// ── 文書・サンプルの例の抽出 ─────────────────────────────────────────────────

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_repo_file(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Mode {
    /// 中身をそのまま TOML として読む。
    AsIs,
    /// `#` で始まる行のうち、TOML の書き出しに見えるもの(`[表]`・`キー =`・`{`・`]`)は
    /// `#` を外し、説明文のコメントは捨てる(コメントアウトされた設定例を検証するため)。
    Uncomment,
}

struct Example {
    name: String,
    mode: Mode,
    /// `data-default="keys"` など。「デフォルト値」と書いた例。
    default_of: Option<String>,
    body: String,
}

fn parse_mode(s: &str) -> Mode {
    match s.trim() {
        "asis" => Mode::AsIs,
        "uncomment" => Mode::Uncomment,
        other => panic!("未知の example mode: {other:?}"),
    }
}

/// `config.toml`・`README.md` 用: 行ベースの目印を抽出する。
/// 書式: `# example-begin: <名前> <asis|uncomment>` … `# example-end`(TOML のコメント)、
/// または `<!-- example-begin: <名前> <mode> -->` … `<!-- example-end -->`(Markdown。
/// 間にある ``` の囲み行は捨てる)。
fn extract_line_examples(text: &str) -> Vec<Example> {
    let mut out = Vec::new();
    let mut cur: Option<(String, Mode, String)> = None;
    for line in text.lines() {
        if let Some(pos) = line.find("example-begin:") {
            let spec = line[pos + "example-begin:".len()..]
                .trim_end_matches("-->")
                .trim();
            let (name, mode) = spec.split_once(' ').expect("example-begin: <名前> <mode>");
            assert!(cur.is_none(), "example-begin が入れ子になっている: {name}");
            cur = Some((name.to_string(), parse_mode(mode), String::new()));
        } else if line.contains("example-end") {
            let (name, mode, body) = cur.take().expect("example-begin の無い example-end");
            out.push(Example {
                name,
                mode,
                default_of: None,
                body,
            });
        } else if let Some((_, _, body)) = cur.as_mut() {
            if !line.trim_start().starts_with("```") {
                body.push_str(line);
                body.push('\n');
            }
        }
    }
    assert!(cur.is_none(), "example-end が無い例がある");
    out
}

/// `docs/usage*.html` 用: `<div class="code-block" data-example="<名前>" data-mode="<mode>"
/// [data-default="keys"]>…</div>` を抽出する(目印の無い `code-block` は対象外)。
fn extract_html_examples(html: &str) -> Vec<Example> {
    const TAG: &str = "<div class=\"code-block\" data-example=\"";
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find(TAG) {
        let after = &rest[i..];
        let gt = after.find('>').expect("タグが閉じていない");
        let tag = &after[..gt];
        let attr = |key: &str| -> Option<String> {
            let k = format!("{key}=\"");
            let s = tag.find(&k)? + k.len();
            Some(tag[s..s + tag[s..].find('"')?].to_string())
        };
        let end = after.find("</div>").expect("</div> が無い");
        out.push(Example {
            name: attr("data-example").expect("data-example"),
            mode: parse_mode(&attr("data-mode").expect("data-mode")),
            default_of: attr("data-default"),
            body: after[gt + 1..end].to_string(),
        });
        rest = &after[end..];
    }
    out
}

/// `#` 以降(先頭の空白を除く)が TOML の書き出しに見えるか。
fn looks_like_toml(s: &str) -> bool {
    let s = s.trim_start();
    if s.starts_with(['[', '{', ']']) {
        return true;
    }
    let ident: String = s
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
        .collect();
    !ident.is_empty() && s[ident.len()..].trim_start().starts_with('=')
}

fn to_toml(e: &Example) -> String {
    match e.mode {
        Mode::AsIs => e.body.clone(),
        Mode::Uncomment => e
            .body
            .lines()
            .filter_map(|l| {
                l.trim_start().strip_prefix('#').map_or_else(
                    || Some(l.to_string()),
                    |rest| {
                        looks_like_toml(rest)
                            .then(|| rest.strip_prefix(' ').unwrap_or(rest).to_string())
                    },
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// 対象ファイルすべての例(`<ファイル>::<名前>` → 例)。
fn all_doc_examples() -> Vec<(String, Example)> {
    let mut out = Vec::new();
    for (file, is_html) in [
        ("config.toml", false),
        ("README.md", false),
        ("docs/usage.html", true),
        ("docs/usage.en.html", true),
    ] {
        let text = read_repo_file(file);
        let examples = if is_html {
            extract_html_examples(&text)
        } else {
            extract_line_examples(&text)
        };
        assert!(
            !examples.is_empty(),
            "{file}: 例の目印が1つも見つからない(目印が消えた?)"
        );
        out.extend(examples.into_iter().map(|e| (file.to_string(), e)));
    }
    out
}

// ── テスト ──────────────────────────────────────────────────────────────────

fn assert_same_set(what: &str, actual: &BTreeSet<String>, known: &[&str]) {
    let known: BTreeSet<String> = known.iter().map(|s| (*s).to_string()).collect();
    let fixed: Vec<_> = known.difference(actual).collect();
    let new: Vec<_> = actual.difference(&known).collect();
    assert!(
        fixed.is_empty() && new.is_empty(),
        "{what}: 既知の失敗の一覧と現状が違う。\n\
         - 新しい失敗(一覧に無い。回帰か、一覧への追加が必要): {new:#?}\n\
         - もう失敗しない(一覧に残っている。修正済みなら一覧から消す): {fixed:#?}"
    );
}

#[test]
fn doc_examples_match_baseline() {
    let mut failures = BTreeSet::new();
    let mut warn_counts = Vec::new();
    for (file, ex) in all_doc_examples() {
        let a = analyze(&to_toml(&ex), ex.default_of.as_deref() == Some("keys"));
        warn_counts.push((ex.name.clone(), a.warnings));
        for f in a.failures {
            failures.insert(format!("{}|{f}", ex.name));
        }
        let _ = file;
    }
    assert_same_set("文書・サンプルの例", &failures, DOC_KNOWN_FAILURES);

    let expected: BTreeSet<&str> = DOC_BASELINE.iter().map(|(n, _)| *n).collect();
    let actual: BTreeSet<&str> = warn_counts.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        expected, actual,
        "DOC_BASELINE と抽出した例の名前が一致しない(例を足した・消したら基準表も更新する)"
    );
    for (name, n) in &warn_counts {
        let base = DOC_BASELINE.iter().find(|(b, _)| b == name).unwrap().1;
        assert!(
            *n <= base,
            "{name}: validate() の警告が増えた ({base} -> {n})"
        );
    }
}

#[test]
fn doc_examples_extraction_is_stable() {
    // 目印の書式の単体確認(抽出器そのものの回帰)。
    let ex = extract_line_examples(
        "a\n# example-begin: x uncomment\n# [keys]\n# 説明 = ではない\n# ime_on = [\"F13\"]\n#     { a = 1 },\n# example-end\n",
    );
    assert_eq!(ex.len(), 1);
    assert_eq!(
        to_toml(&ex[0]),
        "[keys]\nime_on = [\"F13\"]\n    { a = 1 },"
    );
    let html = "<div class=\"code-block\">skip</div><div class=\"code-block\" data-example=\"n\" data-mode=\"asis\" data-default=\"keys\">[keys]\na = 1</div>";
    let ex = extract_html_examples(html);
    assert_eq!(
        (ex.len(), ex[0].name.as_str(), ex[0].default_of.as_deref()),
        (1, "n", Some("keys"))
    );
}

/// 合成 config の一覧。ディレクトリ内の `*.toml` を全部読み、基準表に無いものは落とす。
fn fixture_files() -> Vec<(String, String)> {
    let dir = repo_root().join("tests/fixtures/configs");
    let mut v: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(e.path()).unwrap(),
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn synthetic_configs_match_baseline() {
    let files = fixture_files();
    let mut failures = BTreeSet::new();
    let mut counts = Vec::new();
    for (name, text) in &files {
        let a = analyze(text, false);
        counts.push((name.as_str(), a.warnings));
        for f in a.failures {
            failures.insert(format!("{name}|{f}"));
        }
    }
    assert_same_set("合成 config", &failures, FIXTURE_KNOWN_FAILURES);

    let expected: BTreeSet<&str> = FIXTURE_BASELINE.iter().map(|(n, _)| *n).collect();
    let actual: BTreeSet<&str> = counts.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        expected, actual,
        "FIXTURE_BASELINE と tests/fixtures/configs/*.toml が一致しない"
    );
    for (name, n) in &counts {
        let base = FIXTURE_BASELINE.iter().find(|(b, _)| b == name).unwrap().1;
        assert!(
            *n <= base,
            "{name}: validate() の警告が増えた ({base} -> {n})"
        );
    }
}

/// 実物の不具合報告の config は同意が無いためリポジトリに置かない(ADR-201 決定4-3)。
/// 合成 config が実物らしい印(実在しそうなパス・プロセス名)を含まないことの最低限の確認。
#[test]
fn synthetic_configs_contain_no_environment_specific_values() {
    for (name, text) in fixture_files() {
        for banned in ["C:\\", "C:/", "/home/", "/Users/", "layouts_dir"] {
            assert!(
                !text.contains(banned),
                "{name}: {banned:?} を含む(実物由来の値を置かない)"
            );
        }
    }
}
