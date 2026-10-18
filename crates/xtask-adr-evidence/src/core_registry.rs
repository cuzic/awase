//! FCIS V1: `layer_boundary_guard.rs` の `CORE_MODULES` に PR で足された名前が、
//! `.cargo/mutants-awase-windows.toml` の `examine_globs` に
//! `"crates/awase-windows/src/state/<名前>.rs"` の行（`#` のコメント行は数えない、完全一致）として
//! 載っていることを確かめる（F2・F3・F4 で mutants への登録が後追いになった）。
//!
//! pre-push の正規表現は TC3、`fix-requires-evidence.md` の表は人の判断（V2）に任せ、ここでは見ない。
//!
//! 対象は「base に無く HEAD にある名前」。`NOT_CORE_MODULES` から `CORE_MODULES` へ移した名前も
//! 追加として扱う（mutants に載っていないものは載せる、で一貫させる）。既存の未登録は対象外（V1b）。
//! 除外リストは持たない。`CORE_MODULES` を読めない・0 件のときは通さずに失敗する。

use regex::Regex;

/// `const CORE_MODULES: &[&str] = &[ ... ];` の名前を読む。名前行・空行・コメント行のどれでもない行が
/// あるか、0 件なら Err（黙って取りこぼして通すことをしない）。
pub fn parse_core_modules(src: &str) -> Result<Vec<String>, String> {
    let start = src
        .find("const CORE_MODULES")
        .ok_or("CORE_MODULES が無い")?;
    let rest = &src[start..];
    let open = rest.find("= &[").ok_or("CORE_MODULES の `= &[` が無い")? + 4;
    let end = rest[open..]
        .find("];")
        .ok_or("CORE_MODULES の `];` が無い")?
        + open;
    let name_re = Regex::new(r#"^\s*"(\w+)"\s*,\s*(//.*)?$"#).unwrap();
    let mut out = Vec::new();
    for line in rest[open..end].lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") {
            continue;
        }
        match name_re.captures(line) {
            Some(c) => out.push(c[1].to_string()),
            None => return Err(format!("CORE_MODULES の行を読めない: {t}")),
        }
    }
    if out.is_empty() {
        return Err("CORE_MODULES から 1 件も読めない".into());
    }
    Ok(out)
}

/// base に無く head にある名前（並べ替え・削除は含まない。リネームは新名だけ）。
pub fn added_modules(base: &[String], head: &[String]) -> Vec<String> {
    head.iter().filter(|h| !base.contains(h)).cloned().collect()
}

/// `name` の mutants 登録の有無。核 crate（`awase-windows-core`、ADR-229 D4）へ移ったファイルと、
/// 殻の crate（`awase-windows`、`platform_state`・`sync_actuation`）に残ったファイルの両方のパスを受け付ける。
pub fn in_mutants(toml: &str, name: &str) -> bool {
    let wants = [
        format!("\"crates/awase-windows-core/src/state/{name}.rs\""),
        format!("\"crates/awase-windows/src/state/{name}.rs\""),
    ];
    toml.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .any(|l| wants.contains(&l.trim().trim_end_matches(',').trim().to_string()))
}

/// mutants に未登録の追加名ごとの報告（空なら合格）。
pub fn check(added: &[String], mutants: &str) -> Vec<String> {
    added
        .iter()
        .filter(|n| !in_mutants(mutants, n))
        .map(|n| {
            format!(
                "CORE_MODULES に追加された `{n}` が .cargo/mutants-awase-windows{{,-core}}.toml の examine_globs に未登録\
                 （核 crate のファイルなら \"crates/awase-windows-core/src/state/{n}.rs\"〔-core.toml〕、\
                 殻の crate のファイルなら \"crates/awase-windows/src/state/{n}.rs\" を足す）"
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core(lines: &[&str]) -> String {
        format!(
            "const A: &[&str] = &[\n    \"zzz\",\n];\n/// doc\nconst CORE_MODULES: &[&str] = &[\n{}\n];\nconst NOT_CORE_MODULES: &[(&str, &str)] = &[\n    (\n        \"hub_clock\",\n        \"理由\",\n    ),\n];\n",
            lines.iter().map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n")
        )
    }
    fn parse(lines: &[&str]) -> Vec<String> {
        parse_core_modules(&core(lines)).unwrap()
    }
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    // 実物: 各コミット直前の examine_globs の state/ 行（git show <hash>^:.cargo/mutants-awase-windows.toml）
    const MUT_BEFORE_F2: &str = "examine_globs = [\n    \"crates/awase-windows/src/state/physical_disposition.rs\",\n    \"crates/awase-windows/src/state/transition.rs\",\n]\n"; // d04c3843^
    const MUT_BEFORE_F3: &str = "examine_globs = [\n    \"crates/awase-windows/src/state/physical_disposition.rs\",\n    \"crates/awase-windows/src/state/transition.rs\",\n    \"crates/awase-windows/src/state/drift_plan.rs\",\n    \"crates/awase-windows/src/state/ime_set_open_plan.rs\",\n]\n"; // bfdceb08^
    const MUT_BEFORE_F4: &str = MUT_BEFORE_F2; // fbd1bea6^
    const MUT_BEFORE_PHYS: &str =
        "examine_globs = [\n    \"crates/awase-windows/src/state/transition.rs\",\n]\n"; // 71b7bbff^

    #[test]
    fn past_misses_fail() {
        for (name, mutants) in [
            ("ime_set_open_plan", MUT_BEFORE_F2),
            ("relay_plan", MUT_BEFORE_F3),
            ("drift_plan", MUT_BEFORE_F4),
            ("physical_disposition", MUT_BEFORE_PHYS),
        ] {
            assert_eq!(check(&s(&[name]), mutants).len(), 1, "{name}");
        }
    }

    #[test]
    fn same_commit_registration_passes() {
        // F5a(4fb4d836)は CORE と mutants を同じコミットで足していた
        let m =
            format!("{MUT_BEFORE_F2}    \"crates/awase-windows/src/state/msaa_role_plan.rs\",\n");
        assert!(check(&s(&["msaa_role_plan"]), &m).is_empty());
    }

    #[test]
    fn mutants_accepts_core_crate_and_shell_crate_paths() {
        // 核 crate（awase-windows-core）へ移ったファイルと、殻に残ったファイルの両方を受け付ける（ADR-229 D4）。
        assert!(in_mutants(
            "examine_globs = [\n    \"crates/awase-windows-core/src/state/foo.rs\",\n]\n",
            "foo"
        ));
        assert!(in_mutants(
            "examine_globs = [\n    \"crates/awase-windows/src/state/foo.rs\",\n]\n",
            "foo"
        ));
        assert!(!in_mutants(
            "examine_globs = [\n    \"crates/awase-windows-core/src/state/bar.rs\",\n]\n",
            "foo"
        ));
    }

    #[test]
    fn mutants_requires_exact_uncommented_line() {
        assert!(!in_mutants(
            "    \"crates/awase-windows/src/state/belief_x.rs\",\n",
            "belief"
        ));
        assert!(!in_mutants(
            "    # \"crates/awase-windows/src/state/belief.rs\",\n",
            "belief"
        ));
        assert!(!in_mutants(
            "    \"crates/awase-windows/src/focus/belief.rs\",\n",
            "belief"
        ));
        assert!(in_mutants(
            "    \"crates/awase-windows/src/state/belief.rs\",\n",
            "belief"
        ));
    }

    #[test]
    fn parses_names_ignoring_trailing_and_full_line_comments() {
        let v = parse(&[
            "\"belief\",",
            "// \"commented\",",
            "\"drift_plan\", // 備考",
        ]);
        assert_eq!(v, ["belief", "drift_plan"]);
    }

    #[test]
    fn unreadable_block_fails_instead_of_passing() {
        // 1 行に 2 名、識別子でない名前、閉じ忘れ、0 件、ブロック無し
        assert!(parse_core_modules(&core(&["\"a\", \"b\","])).is_err());
        assert!(parse_core_modules(&core(&["\"a-b\","])).is_err());
        assert!(parse_core_modules(&core(&["\"a\""])).is_err());
        assert!(parse_core_modules(&core(&[])).is_err());
        assert!(parse_core_modules("const CORE_MODULES: &[&str] = &[\n \"a\",\n").is_err());
        assert!(parse_core_modules("fn main() {}").is_err());
    }

    #[test]
    fn added_ignores_reorder_and_deletion_and_rename_counts_new_name() {
        let base = parse(&["\"a\",", "\"b\",", "\"c\","]);
        assert!(added_modules(&base, &parse(&["\"c\",", "\"a\","])).is_empty());
        assert_eq!(
            added_modules(&parse(&["\"old_plan\","]), &parse(&["\"new_plan\","])),
            ["new_plan"]
        );
    }

    #[test]
    fn moved_from_not_core_counts_as_added() {
        // NOT_CORE_MODULES の名前は CORE_MODULES の外なので、移すと追加になる
        let base = parse(&["\"a\","]);
        assert_eq!(
            added_modules(&base, &parse(&["\"a\",", "\"hub_clock\","])),
            ["hub_clock"]
        );
    }
}
