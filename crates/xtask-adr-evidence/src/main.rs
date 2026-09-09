//! ADR-158 TB2: `lints/actuation_call_guard`の`RESTRICTED_CALLS`宣言（ADR-161 D1が
//! 定めるSSOT）から、`.claude/rules/fix-requires-evidence.md`の「IME actuation合流点」行と
//! `crates/awase-windows/tests/architecture_guard.rs`のガード期待値を生成し、
//! 手書きの現状と比較する。
//!
//! ADR-161 D1の生成方針（round4 TJ1 M1で確定）に従い、`note`欄（散文の注記）は生成せず
//! 宣言側の`callee`/`callers`のみを機械的に検証する——`fix-requires-evidence.md`の散文部分
//! （なぜこの合流点が独立に必要か等）は引き続き人手で維持する。
//!
//! 使い方: `cargo run -p xtask-adr-evidence -- <repo_root>`

use std::path::Path;
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

fn main() {
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

    println!("# architecture_guard.rs ガード期待値との照合\n");
    for target in ["apply_ime_open_with_view", "apply_ime_open_with_belief"] {
        if let Some(call) = calls.iter().find(|c| c.callee == target) {
            println!(
                "- `.{}(` の期待値は宣言から数えると **{}** 件",
                target,
                call.callers.len()
            );
        }
    }

    println!("\n# fix-requires-evidence.md「IME actuation合流点」行 生成案\n");
    if let Some(view) = calls
        .iter()
        .find(|c| c.callee == "apply_ime_open_with_view")
    {
        println!(
            "宣言済み呼び出し元（{}件）: {}",
            view.callers.len(),
            view.callers.join(", ")
        );
    }
    if let Some(belief) = calls
        .iter()
        .find(|c| c.callee == "apply_ime_open_with_belief")
    {
        println!(
            "apply_ime_open_with_belief 宣言済み呼び出し元（{}件）: {}",
            belief.callers.len(),
            belief.callers.join(", ")
        );
    }
}
