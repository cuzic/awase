//! FCIS V1: `layer_boundary_guard.rs` の `CORE_MODULES` に PR で足された名前が、同じ PR の後の状態で
//! 次の 3 か所にも載っていることを確かめる。載っていないと、新しい純粋な核が mutants・pre-push の
//! 警告・再発ファミリー表の網から落ちる（F2・F3・F4・F5a で登録漏れが後追いになった）。
//!
//! - mutants: `.cargo/mutants-awase-windows.toml` の `examine_globs` に
//!   `"crates/awase-windows/src/state/<名前>.rs"` の行（コメント行は数えない）。
//! - pre-push: `.githooks/pre-push` の `target` 正規表現の `state/(a|b|...)` の枝のどれかが名前の接頭辞
//!   （`ime` は `ime_kind` 等を、`key_effect_` は `key_effect_table` 等を覆う。既存の書き方に合わせる）。
//! - fix-requires: `.claude/rules/fix-requires-evidence.md` の表の行（`|` 始まり）に `state/<名前>.rs`
//!   （`.rs` まで完全一致。`belief` の登録は `belief_x` を満たさない）。
//!
//! 対象は「base に無く HEAD にある名前」だけ（既存の未登録は対象外、別の棚卸し）。除外リストは持たない。
//! 3 か所とも不要な正当な理由があるときは、`CORE_MODULES` のその行に
//! `// registry-exempt: <理由>`（理由は空でない）を書く。差分に残るのでレビューで見える。

use regex::Regex;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CoreModule {
    pub name: String,
    /// `// registry-exempt: <理由>` の理由。
    pub exempt: Option<String>,
}

/// `const CORE_MODULES: &[&str] = &[ ... ];` の中身を読む。ブロックが無ければ None。
pub fn parse_core_modules(src: &str) -> Option<Vec<CoreModule>> {
    let start = src.find("const CORE_MODULES")?;
    let rest = &src[start..];
    let open = rest.find("&[")? + 2;
    // 要素は文字列と行コメントだけなので、最初の `];` までがブロック。
    let end = rest[open..].find("];")? + open;
    let name_re = Regex::new(r#"^\s*"(\w+)"\s*,\s*(?://\s*(.*))?$"#).unwrap();
    let mut out = Vec::new();
    for line in rest[open..end].lines() {
        if let Some(c) = name_re.captures(line) {
            let exempt = c.get(2).and_then(|m| {
                let t = m.as_str().trim();
                t.strip_prefix("registry-exempt:")
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty())
            });
            out.push(CoreModule {
                name: c[1].to_string(),
                exempt,
            });
        }
    }
    Some(out)
}

/// base に無く head にある名前（並べ替え・削除・コメントだけの変更は含まない）。
pub fn added_modules(base: &[CoreModule], head: &[CoreModule]) -> Vec<CoreModule> {
    head.iter()
        .filter(|h| !base.iter().any(|b| b.name == h.name))
        .cloned()
        .collect()
}

pub fn in_mutants(toml: &str, name: &str) -> bool {
    let want = format!("\"crates/awase-windows/src/state/{name}.rs\"");
    toml.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .any(|l| l.trim().trim_end_matches(',').trim() == want)
}

pub fn in_pre_push(script: &str, name: &str) -> bool {
    let Some(line) = script
        .lines()
        .find(|l| l.trim_start().starts_with("local target="))
    else {
        return false;
    };
    let Some(c) = Regex::new(r"state/\(([^)]*)\)").unwrap().captures(line) else {
        return false;
    };
    c[1].split('|')
        .any(|tok| !tok.is_empty() && name.starts_with(tok))
}

pub fn in_fix_requires(md: &str, name: &str) -> bool {
    let re = Regex::new(&format!(r"(^|[^\w])state/{}\.rs", regex::escape(name))).unwrap();
    md.lines()
        .filter(|l| l.trim_start().starts_with('|'))
        .any(|l| re.is_match(l))
}

pub struct Sources<'a> {
    pub mutants: &'a str,
    pub pre_push: &'a str,
    pub fix_requires: &'a str,
}

/// 追加された各名前について、足りない登録先の報告を返す（空なら合格）。
pub fn check(added: &[CoreModule], s: &Sources) -> Vec<String> {
    let mut out = Vec::new();
    for m in added {
        if m.exempt.is_some() {
            continue;
        }
        let mut missing = Vec::new();
        if !in_mutants(s.mutants, &m.name) {
            missing.push(".cargo/mutants-awase-windows.toml の examine_globs");
        }
        if !in_pre_push(s.pre_push, &m.name) {
            missing.push(".githooks/pre-push の target 正規表現の state/(...)");
        }
        if !in_fix_requires(s.fix_requires, &m.name) {
            missing.push(".claude/rules/fix-requires-evidence.md の再発ファミリー表");
        }
        if !missing.is_empty() {
            out.push(format!(
                "CORE_MODULES に追加された `{}` が未登録: {}（不要なら CORE_MODULES のその行に `// registry-exempt: <理由>`）",
                m.name,
                missing.join("、")
            ));
        }
    }
    out
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
    fn names(src: &str) -> Vec<String> {
        parse_core_modules(src)
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect()
    }

    // 実物（bfdceb08 の直前の pre-push 行。relay_plan は未登録）
    const PP_BEFORE_RELAY: &str = "    local target='(crates/awase-windows/src/(hook\\.rs|output/|tsf/|focus/|runtime/(ime_coordinator|mod)\\.rs|state/(ime|conv_mode|observation_store|platform_state|mode_key_pass|key_effect_|press_ledger|explicit_press|physical_disposition|drift_plan)|ime_controller\\.rs|ime\\.rs)|src/engine/(nicola_fsm|engine)\\.rs)'\n";
    const PP_AFTER_RELAY: &str = "    local target='(crates/awase-windows/src/(hook\\.rs|runtime/(ime_coordinator|mod)\\.rs|state/(ime|conv_mode|observation_store|platform_state|mode_key_pass|key_effect_|press_ledger|explicit_press|physical_disposition|drift_plan|relay_plan)|ime_controller\\.rs)|src/engine/(nicola_fsm|engine)\\.rs)'\n";
    const MUT_BEFORE: &str = "examine_globs = [\n    \"crates/awase-windows/src/state/drift_plan.rs\",\n    \"crates/awase-windows/src/state/ime_set_open_plan.rs\",\n]\n";
    const MUT_AFTER: &str = "examine_globs = [\n    \"crates/awase-windows/src/state/drift_plan.rs\",\n    \"crates/awase-windows/src/state/relay_plan.rs\",\n]\n";
    const FR_BEFORE: &str = "| IME belief | `state/ime_model.rs`, `state/drift_plan.rs`（FCIS F4）, `runtime/ime_coordinator.rs` |\n";
    const FR_AFTER: &str = "| IME belief | `state/ime_model.rs`, `state/drift_plan.rs`（FCIS F4）, `state/relay_plan.rs`, `runtime/ime_coordinator.rs` |\n";

    fn relay() -> Vec<CoreModule> {
        vec![CoreModule {
            name: "relay_plan".into(),
            exempt: None,
        }]
    }

    #[test]
    fn parses_only_core_modules_block() {
        let src = core(&[
            "\"belief\",",
            "\"drift_plan\", // 備考",
            "\"x_y\",   // registry-exempt: 理由あり",
        ]);
        assert_eq!(names(&src), ["belief", "drift_plan", "x_y"]);
        let m = parse_core_modules(&src).unwrap();
        assert_eq!(m[1].exempt, None);
        assert_eq!(m[2].exempt.as_deref(), Some("理由あり"));
    }

    #[test]
    fn exempt_requires_nonempty_reason() {
        let src = core(&[
            "\"a\", // registry-exempt:",
            "\"b\", // registry-exempt:   ",
        ]);
        assert!(parse_core_modules(&src)
            .unwrap()
            .iter()
            .all(|m| m.exempt.is_none()));
    }

    #[test]
    fn comment_line_and_other_consts_are_not_names() {
        let src = core(&["// \"commented\",", "\"real\","]);
        assert_eq!(names(&src), ["real"]);
    }

    #[test]
    fn added_ignores_reorder_removal_and_comment_change() {
        let base = parse_core_modules(&core(&["\"a\",", "\"b\",", "\"c\","])).unwrap();
        let head = parse_core_modules(&core(&["\"c\", // 新しいコメント", "\"a\","])).unwrap();
        assert!(added_modules(&base, &head).is_empty());
        let head2 = parse_core_modules(&core(&["\"a\",", "\"d\","])).unwrap();
        assert_eq!(added_modules(&base, &head2)[0].name, "d");
    }

    #[test]
    fn rename_counts_as_added_new_name_only() {
        let base = parse_core_modules(&core(&["\"old_plan\","])).unwrap();
        let head = parse_core_modules(&core(&["\"new_plan\","])).unwrap();
        let a = added_modules(&base, &head);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].name, "new_plan");
    }

    #[test]
    fn no_core_block_is_none() {
        assert!(parse_core_modules("fn main() {}").is_none());
    }

    #[test]
    fn mutants_requires_exact_uncommented_line() {
        assert!(in_mutants(MUT_AFTER, "relay_plan"));
        assert!(!in_mutants(MUT_BEFORE, "relay_plan"));
        // 部分一致・コメントアウトでは満たさない
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
    }

    #[test]
    fn pre_push_uses_prefix_branches_like_existing_regex() {
        assert!(in_pre_push(PP_AFTER_RELAY, "relay_plan"));
        assert!(!in_pre_push(PP_BEFORE_RELAY, "relay_plan"));
        // 既存の枝 `ime` / `key_effect_` が覆う名前
        assert!(in_pre_push(PP_BEFORE_RELAY, "ime_set_open_plan"));
        assert!(in_pre_push(PP_BEFORE_RELAY, "key_effect_table"));
        // `conv_mode` は `conv_classify` を覆わない
        assert!(!in_pre_push(PP_BEFORE_RELAY, "conv_classify"));
        // コメント内の名前は数えない（target 行以外）
        assert!(!in_pre_push("# state/(relay_plan)\n", "relay_plan"));
    }

    #[test]
    fn fix_requires_exact_name_in_table_rows_only() {
        assert!(in_fix_requires(FR_AFTER, "relay_plan"));
        assert!(!in_fix_requires(FR_BEFORE, "relay_plan"));
        assert!(!in_fix_requires("| x | `state/belief_x.rs` |\n", "belief"));
        assert!(in_fix_requires("| x | `state/belief.rs` |\n", "belief"));
        // 表の行でない本文（散文）は数えない
        assert!(!in_fix_requires(
            "本文 `state/belief.rs` に言及\n",
            "belief"
        ));
        // 別ディレクトリの同名は数えない
        assert!(!in_fix_requires("| x | `xstate/belief.rs` |\n", "belief"));
    }

    #[test]
    fn past_miss_relay_plan_f3_fails_on_all_three() {
        // PR #517: CORE_MODULES だけ追加し、3 か所は後追い（bfdceb08）だった状態
        let v = check(
            &relay(),
            &Sources {
                mutants: MUT_BEFORE,
                pre_push: PP_BEFORE_RELAY,
                fix_requires: FR_BEFORE,
            },
        );
        assert_eq!(v.len(), 1);
        assert!(
            v[0].contains("relay_plan")
                && v[0].contains("mutants")
                && v[0].contains("pre-push")
                && v[0].contains("fix-requires")
        );
    }

    #[test]
    fn past_miss_msaa_role_plan_f5a_fails_on_two() {
        // PR #512: mutants だけ登録し、pre-push・表は未登録のまま
        let mutants = "    \"crates/awase-windows/src/state/msaa_role_plan.rs\",\n";
        let added = vec![CoreModule {
            name: "msaa_role_plan".into(),
            exempt: None,
        }];
        let v = check(
            &added,
            &Sources {
                mutants,
                pre_push: PP_BEFORE_RELAY,
                fix_requires: FR_BEFORE,
            },
        );
        assert_eq!(v.len(), 1);
        assert!(
            !v[0].contains("mutants-awase")
                && v[0].contains("pre-push")
                && v[0].contains("fix-requires")
        );
    }

    #[test]
    fn partial_registration_one_missing_fails() {
        let v = check(
            &relay(),
            &Sources {
                mutants: MUT_AFTER,
                pre_push: PP_AFTER_RELAY,
                fix_requires: FR_BEFORE,
            },
        );
        assert_eq!(v.len(), 1);
        assert!(
            v[0].contains("fix-requires")
                && !v[0].contains("mutants")
                && !v[0].contains("pre-push")
        );
    }

    #[test]
    fn fully_registered_and_exempt_pass() {
        let ok = Sources {
            mutants: MUT_AFTER,
            pre_push: PP_AFTER_RELAY,
            fix_requires: FR_AFTER,
        };
        assert!(check(&relay(), &ok).is_empty());
        let ex = vec![CoreModule {
            name: "zzz".into(),
            exempt: Some("再発ファミリーに属さない".into()),
        }];
        assert!(check(
            &ex,
            &Sources {
                mutants: "",
                pre_push: "",
                fix_requires: ""
            }
        )
        .is_empty());
    }

    #[test]
    fn deletion_only_diff_checks_nothing() {
        let base = parse_core_modules(&core(&["\"a\",", "\"b\","])).unwrap();
        let head = parse_core_modules(&core(&["\"a\","])).unwrap();
        assert!(added_modules(&base, &head).is_empty());
    }
}
