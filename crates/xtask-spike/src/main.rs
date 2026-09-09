//! ADR-161 D1 検証スパイク（恒久化しない）。
//!
//! opus-adversarial-consultレビュー(ADR-161)は、属性マクロ方式(stable Rustでは式に
//! 属性を付けられない、M1)と`inventory`クレート(実行時登録でLinux CIから生成できない、
//! M2)がD1(単一仕様からの生成)には使えないと結論し、代案として「`syn`ベースの
//! source-parsing xtask」を推奨した。本スパイクはこの代案が実際に機能するかを検証する。
//!
//! 検証項目:
//! 1. `crates/awase-windows/src`配下を静的パースし、指定した関数/メソッド呼び出しの
//!    実際の呼び出し箇所(ファイル:行)を数え上げられるか。
//! 2. 数え上げた結果が、既存の`architecture_guard.rs`の`ENTRY_POINTS`定数（人手管理）や
//!    レビューで実測済みの件数と一致するか（正確性の検証）。
//! 3. 正規表現ベースの走査（`architecture_guard.rs`が現在採用）が抱える既知の誤カウント
//!    問題（コメント・文字列リテラル・関数定義行の混入、ADR-161 S1が指摘した
//!    `windows::Win32`161行のうち105行が`use`文だった例）を、AST走査が構造的に
//!    回避できるかを確認する。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, ExprMethodCall};

/// 数え上げたい対象。メソッド呼び出し(`recv.name(...)`)と自由関数呼び出し
/// (`path::to::name(...)`)の両方を、最後のセグメント名だけで照合する
/// （呼び出し元の型解決はしない——source-parsingの限界としてADR本文に明記する）。
const TARGETS: &[&str] = &[
    "apply_ime_open_with_view",
    "apply_ime_open_with_belief",
    "apply_ime_open_with_applied",
    "set_ime_open",
    "set_ime_open_ordered",
    "apply_ime_open",
    "send_input_safe",
    "send_ime_control",
];

#[derive(Debug, Clone)]
struct Hit {
    file: PathBuf,
    line: usize,
    kind: &'static str,
    target: String,
}

struct CallVisitor<'a> {
    file: &'a Path,
    hits: Vec<Hit>,
}

impl<'a> Visit<'a> for CallVisitor<'a> {
    fn visit_expr_method_call(&mut self, node: &'a ExprMethodCall) {
        let name = node.method.to_string();
        if TARGETS.contains(&name.as_str()) {
            let line = node.method.span().start().line;
            self.hits.push(Hit {
                file: self.file.to_path_buf(),
                line,
                kind: "method",
                target: name,
            });
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_call(&mut self, node: &'a ExprCall) {
        if let Expr::Path(p) = &*node.func {
            if let Some(last) = p.path.segments.last() {
                let name = last.ident.to_string();
                if TARGETS.contains(&name.as_str()) {
                    let line = last.ident.span().start().line;
                    self.hits.push(Hit {
                        file: self.file.to_path_buf(),
                        line,
                        kind: "fn",
                        target: name,
                    });
                }
            }
        }
        visit::visit_expr_call(self, node);
    }
}

fn walk_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_rs_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn main() {
    let root = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "crates/awase-windows/src".to_string());
    let root = PathBuf::from(root);

    let mut files = Vec::new();
    walk_rs_files(&root, &mut files).expect("walk failed");
    files.sort();

    let mut all_hits: Vec<Hit> = Vec::new();
    let mut parse_failures: Vec<(PathBuf, String)> = Vec::new();

    for file in &files {
        let src = match fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                parse_failures.push((file.clone(), format!("read error: {e}")));
                continue;
            }
        };
        let parsed = match syn::parse_file(&src) {
            Ok(f) => f,
            Err(e) => {
                parse_failures.push((file.clone(), format!("parse error: {e}")));
                continue;
            }
        };
        let mut visitor = CallVisitor {
            file,
            hits: Vec::new(),
        };
        visitor.visit_file(&parsed);
        all_hits.extend(visitor.hits);
    }

    println!("=== 走査対象: {} ファイル ===", files.len());
    if !parse_failures.is_empty() {
        println!("=== パース失敗: {} 件 ===", parse_failures.len());
        for (f, err) in &parse_failures {
            println!("  {}: {err}", f.display());
        }
    }

    println!("\n=== 呼び出し箇所一覧 ===");
    let mut sorted_hits = all_hits.clone();
    sorted_hits.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    for h in &sorted_hits {
        println!(
            "  {}:{} [{}] {}",
            h.file.display(),
            h.line,
            h.kind,
            h.target
        );
    }

    let mut by_target: BTreeMap<&str, usize> = BTreeMap::new();
    for t in TARGETS {
        by_target.insert(*t, 0);
    }
    for h in &all_hits {
        *by_target.entry(h.target.as_str()).or_insert(0) += 1;
    }

    println!("\n=== 対象別集計 ===");
    for (target, count) in &by_target {
        println!("  {target}: {count}");
    }

    println!("\n=== 合計: {} 件 ===", all_hits.len());
}
