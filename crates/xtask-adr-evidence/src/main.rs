//! ADR-158 TB2: `lints/actuation_call_guard`の`RESTRICTED_CALLS`宣言（ADR-161 D1が
//! 定めるSSOT）から、`crates/awase-windows/tests/architecture_guard.rs`のガード期待値との
//! 一致を検証する。
//!
//! ADR-161 D1の生成方針（round4 TJ1 M1で確定）に従い、`note`欄（散文の注記）は生成せず
//! 宣言側の`callee`/`callers`のみを機械的に検証する——`fix-requires-evidence.md`の散文部分
//! （なぜこの合流点が独立に必要か等）は引き続き人手で維持する。
//!
//! # 実際に照合する（2026-09-09、opus code review M3で追加）
//!
//! 当初のバージョンは`RESTRICTED_CALLS`の内容を`println!`で表示するだけで、
//! `architecture_guard.rs`の内容を一度も読まず、比較も終了コードも無かった——
//! つまり「照合」を名乗りながら実際には何も照合していなかった。本バージョンは
//! `architecture_guard.rs`から`(".apply_ime_open_with_view(", N)`のようなガード
//! タプルを実際に抽出し、宣言の許可呼び出し元件数と数値で突き合わせる。不一致が
//! あれば終了コード1で報告する。
//!
//! 使い方: `cargo run -p xtask-adr-evidence -- <repo_root>`（exit 0 = 一致、
//! exit 1 = 不一致または解析失敗）。

use std::path::Path;
use std::process::ExitCode;
use syn::{Expr, ExprArray, ExprLit, ExprTuple, Item, Lit};

struct RestrictedCall {
    callee: String,
    callers: Vec<String>,
}

fn parse_restricted_calls(src: &str) -> Vec<RestrictedCall> {
    let file = syn::parse_file(src).expect("lints/actuation_call_guard/src/lib.rs must parse");
    let mut result = Vec::new();
    for item in &file.items {
        let Item::Const(item_const) = item else {
            continue;
        };
        if item_const.ident != "RESTRICTED_CALLS" {
            continue;
        }
        let Expr::Reference(reference) = item_const.expr.as_ref() else {
            panic!("RESTRICTED_CALLS must be a `&[...]` reference expression");
        };
        let Expr::Array(ExprArray { elems, .. }) = reference.expr.as_ref() else {
            panic!("RESTRICTED_CALLS must be an array literal");
        };
        for elem in elems {
            let Expr::Tuple(ExprTuple { elems: tuple, .. }) = elem else {
                panic!("each RESTRICTED_CALLS entry must be a tuple");
            };
            let callee = lit_str(&tuple[0]);
            let callers = match &tuple[1] {
                Expr::Reference(r) => match r.expr.as_ref() {
                    Expr::Array(ExprArray { elems, .. }) => {
                        elems.iter().map(lit_str).collect::<Vec<_>>()
                    }
                    other => panic!("expected caller array, got {other:?}"),
                },
                other => panic!("expected &[...] caller list, got {other:?}"),
            };
            result.push(RestrictedCall { callee, callers });
        }
    }
    result
}

fn lit_str(expr: &Expr) -> String {
    let Expr::Lit(ExprLit {
        lit: Lit::Str(s), ..
    }) = expr
    else {
        panic!("expected a string literal, got {expr:?}");
    };
    s.value()
}

/// `crates/awase-windows/tests/architecture_guard.rs`本文から
/// `(".foo(", N)` の形のタプルをすべて抽出し、`foo -> N` の対応表を返す。
///
/// synでの構文解析ではなく単純な文字列走査で行う——このガードファイルは
/// `const ENTRY_POINTS: [(&str, usize); N] = [ ... ]`という配列リテラルを
/// 複数個所に持ち、対象を1つの`const`名で特定できないため、`".foo("`という
/// リテラルパターンと直後の整数を素直に拾う方が頑健。
fn extract_guard_expectations(src: &str) -> std::collections::HashMap<String, usize> {
    let mut result = std::collections::HashMap::new();
    let mut rest = src;
    while let Some(start) = rest.find("(\".") {
        let after_quote = &rest[start + 2..];
        let Some(end_quote) = after_quote.find('"') else {
            break;
        };
        let needle = &after_quote[..end_quote];
        // needle は ".foo(" の形。先頭の '.' を落とし、末尾の '(' も落として関数名にする。
        let name = needle.trim_start_matches('.').trim_end_matches('(');
        let after = &after_quote[end_quote + 1..];
        // 次のカンマの後に続く数値を拾う。
        if let Some(comma) = after.find(',') {
            let after_comma = after[comma + 1..].trim_start();
            let digits: String = after_comma
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !digits.is_empty() {
                if let Ok(n) = digits.parse::<usize>() {
                    result.insert(name.to_string(), n);
                }
            }
        }
        rest = &rest[start + 2..];
    }
    result
}

fn main() -> ExitCode {
    let repo_root = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let lint_path = Path::new(&repo_root).join("lints/actuation_call_guard/src/lib.rs");
    let src = std::fs::read_to_string(&lint_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", lint_path.display()));
    let calls = parse_restricted_calls(&src);

    println!(
        "# RESTRICTED_CALLS宣言から読み取った内容（SSOT: {}）\n",
        lint_path.display()
    );
    for call in &calls {
        println!("## {}", call.callee);
        println!("- 許可呼び出し元件数: {}", call.callers.len());
        for caller in &call.callers {
            println!("  - {caller}");
        }
        println!();
    }

    let guard_path = Path::new(&repo_root).join("crates/awase-windows/tests/architecture_guard.rs");
    let guard_src = std::fs::read_to_string(&guard_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", guard_path.display()));
    let guard_expectations = extract_guard_expectations(&guard_src);

    println!("# architecture_guard.rs ガード期待値との照合\n");
    let mut mismatches = Vec::new();
    for target in ["apply_ime_open_with_view", "apply_ime_open_with_belief"] {
        let Some(call) = calls.iter().find(|c| c.callee == target) else {
            continue;
        };
        let declared = call.callers.len();
        match guard_expectations.get(target) {
            Some(&guard_value) if guard_value == declared => {
                println!("- OK: `.{target}(` 宣言{declared}件 = ガード期待値{guard_value}件");
            }
            Some(&guard_value) => {
                println!(
                    "- MISMATCH: `.{target}(` 宣言{declared}件 != ガード期待値{guard_value}件"
                );
                mismatches.push(format!(
                    "{target}: 宣言={declared}, architecture_guard.rs={guard_value}"
                ));
            }
            None => {
                println!("- MISSING: `.{target}(` はarchitecture_guard.rsに見つからなかった");
                mismatches.push(format!(
                    "{target}: 宣言={declared}, architecture_guard.rsに対応するタプルが無い"
                ));
            }
        }
    }

    if mismatches.is_empty() {
        println!("\n全ての照合対象が一致しました。");
        ExitCode::SUCCESS
    } else {
        eprintln!("\n不一致を検出しました:");
        for m in &mismatches {
            eprintln!("  - {m}");
        }
        eprintln!(
            "\n宣言側（lints/actuation_call_guard/src/lib.rs::RESTRICTED_CALLS）と \
             architecture_guard.rsのガード期待値のどちらかを更新して一致させること。"
        );
        ExitCode::FAILURE
    }
}
