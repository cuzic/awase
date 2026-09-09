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
//! # ADR-158 TC3: `.githooks/pre-push`の対象ファイル正規表現との照合
//!
//! 着手前に調べたところ、`.githooks/pre-push`の対象ファイル正規表現の実際の
//! ソースオブトゥルースは`lints/actuation_call_guard`の`RESTRICTED_CALLS`（TB0/TB1の
//! 呼び出し元関数名の宣言）ではなく、`.claude/rules/fix-requires-evidence.md`の
//! 「再発ファミリー」表（warmup/focus/belief/conv/キー選択等、より広い概念の
//! ファイル一覧）だと判明した——同表は「本表にも`.git/hooks/pre-push`の正規表現にも
//! 含まれていなかった」という文言を2箇所に持ち、明示的にこの同期関係を意図している。
//! RESTRICTED_CALLSの呼び出し元一覧はこの表のごく一部（IME actuation合流点・キー選択の
//! 2ファミリー相当）に過ぎず、正規表現全体をRESTRICTED_CALLSから生成するのは無理がある。
//!
//! そのため、正規表現を「生成」する（TC3の当初の字義）のではなく、
//! `fix-requires-evidence.md`表のバッククォート付きファイルパスを抽出し、
//! `.githooks/pre-push`の対象ファイル正規表現がそれらすべてを実際にカバーしているかを
//! **検証**する（表に新しいファイルが追加されたのに正規表現の更新を忘れる、という
//! ADR-156/BUG-116が実際に踏んだ退行パターンを機械的に検出する）。
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

/// `.claude/rules/fix-requires-evidence.md`の「再発ファミリー」表から、バッククォート付き
/// ファイルパスをすべて抽出する。表の各セルは自由記述の日本語プロースとバッククォート付き
/// コードスパンが混在しているため、synのような構文解析ではなく単純な走査で行う。
///
/// 抽出ロジック: バッククォートで囲まれた各スパンについて、
/// 1. `::`以降（関数名サフィックス、例: `runtime/mod.rs::reassert_explicit_physical_key`の
///    `::reassert_explicit_physical_key`）を切り捨てる。
/// 2. 残った文字列が「パスらしい」（`.rs`で終わる、または`/`で終わるディレクトリ参照、かつ
///    ASCII文字のみで構成される）ものだけを採用する。
///
/// 制約: `runtime/open_chain.rs::run_open_chain_async`\`/\`fallback_write\`/\`imm_cross_write`
/// のような「1つのパスに複数の関数名がバッククォート区切りで続く」パターンでは、2つ目以降の
/// 関数名（`fallback_write`等、`/`を含まずパスらしくない）は正しく除外されるが、それらが
/// 実際には1つ目と同じファイルに属するという情報は失われる——今回の照合目的（ファイルパスが
/// 正規表現でカバーされているか）には影響しない。
fn extract_fix_requires_evidence_paths(src: &str) -> Vec<String> {
    // 表本体（`| ファミリー | 主なファイル |`見出しから、次の`##`見出しの直前まで）に走査範囲を
    // 限定する。全文を走査すると「テストの置き場所」節（`tests/`配下のテストファイル一覧、
    // 本チェックの対象＝再発ファミリーの「変更元」ファイルとは別の概念）まで拾ってしまう。
    let Some(table_start) = src.find("| ファミリー | 主なファイル |") else {
        panic!("fix-requires-evidence.md: 再発ファミリー表の見出し行が見つからない");
    };
    let after_start = &src[table_start..];
    let table_end = after_start.find("\n## ").unwrap_or(after_start.len());
    let table_src = &after_start[..table_end];

    // 既知の誤検出（実測して確認済み、いずれも「対象ファイル」ではなくメタな言及）:
    // - `crates/awase-windows/`: キー選択行の「過去の見落とし」を説明する文脈でのみ登場し、
    //   実際にこのディレクトリ全体を対象にする意図ではない。
    // - `tests/architecture_guard.rs`: IME actuation合流点行の中で、本ガードファイル自身
    //   （照合対象ではなく照合する側）への言及として登場する。
    const KNOWN_META_REFERENCES: &[&str] =
        &["crates/awase-windows/", "tests/architecture_guard.rs"];

    let mut paths = Vec::new();
    let mut rest = table_src;
    while let Some(start) = rest.find('`') {
        let after_open = &rest[start + 1..];
        let Some(end) = after_open.find('`') else {
            break;
        };
        let span = &after_open[..end];
        let path_part = span.split("::").next().unwrap_or(span);
        let looks_like_path = path_part.is_ascii()
            && !path_part.contains(' ')
            && (path_part.ends_with(".rs") || path_part.ends_with('/'))
            && !path_part.starts_with('.') // `.claude/rules/...`等の自己参照を除外
            && !KNOWN_META_REFERENCES.contains(&path_part);
        if looks_like_path {
            paths.push(path_part.to_string());
        }
        rest = &after_open[end + 1..];
    }
    paths.sort();
    paths.dedup();
    paths
}

/// 抽出したパスを`.githooks/pre-push`の対象ファイル正規表現でテストするための、
/// 実際のリポジトリ相対パス文字列を組み立てる。`fix-requires-evidence.md`のパスは
/// `crates/awase-windows/src/`基準（例外: `src/engine/nicola_fsm.rs`はルート`awase`クレート
/// 基準ですでにフルパス）で書かれている。
fn to_repo_relative_candidate(path: &str) -> String {
    if path.starts_with("src/") {
        path.to_string()
    } else {
        format!("crates/awase-windows/src/{path}")
    }
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

    // ADR-158 TC3: fix-requires-evidence.md表のファイルパスが
    // .githooks/pre-pushの対象ファイル正規表現でカバーされているかを検証する。
    println!(
        "\n# .githooks/pre-push 対象ファイル正規表現との照合（fix-requires-evidence.md基準）\n"
    );
    let evidence_path = Path::new(&repo_root).join(".claude/rules/fix-requires-evidence.md");
    let evidence_src = std::fs::read_to_string(&evidence_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", evidence_path.display()));
    let evidence_paths = extract_fix_requires_evidence_paths(&evidence_src);

    let hook_path = Path::new(&repo_root).join(".githooks/pre-push");
    let hook_src = std::fs::read_to_string(&hook_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", hook_path.display()));
    let Some(target_line) = hook_src
        .lines()
        .find(|l| l.trim_start().starts_with("local target="))
    else {
        eprintln!("MISSING: .githooks/pre-push に `local target=` 行が見つからなかった");
        mismatches.push(".githooks/pre-push: target正規表現の行が見つからない".to_string());
        return finish(mismatches);
    };
    // `local target='REGEX'` からシングルクォートで囲まれた本体を取り出す。
    let Some(quote_start) = target_line.find('\'') else {
        eprintln!("MISSING: target行がシングルクォートで囲まれていない");
        mismatches.push(".githooks/pre-push: target正規表現の解析に失敗".to_string());
        return finish(mismatches);
    };
    let after_quote = &target_line[quote_start + 1..];
    let quote_end = after_quote.rfind('\'').unwrap_or(after_quote.len());
    let target_regex_src = &after_quote[..quote_end];

    match regex::Regex::new(target_regex_src) {
        Ok(target_regex) => {
            let mut uncovered = Vec::new();
            for path in &evidence_paths {
                let candidate = to_repo_relative_candidate(path);
                // ディレクトリ参照（末尾/）は、そのディレクトリ配下の代表的なファイル名で
                // マッチを試す（正規表現自体はディレクトリ末尾では素の文字列一致にしかならず、
                // 実際のpre-pushはgit diffのファイルパス全体に対してマッチさせるため）。
                let test_candidate = if candidate.ends_with('/') {
                    format!("{candidate}__probe__.rs")
                } else {
                    candidate.clone()
                };
                if !target_regex.is_match(&test_candidate) {
                    uncovered.push(path.clone());
                }
            }
            if uncovered.is_empty() {
                println!(
                    "- OK: fix-requires-evidence.mdの{}件のパスがすべて対象正規表現でカバーされています。",
                    evidence_paths.len()
                );
            } else {
                println!("- MISMATCH: 以下のパスが対象正規表現でカバーされていません:");
                for p in &uncovered {
                    println!("  - {p}");
                    mismatches.push(format!(
                        "fix-requires-evidence.mdのパス`{p}`が.githooks/pre-pushの対象正規表現でカバーされていない"
                    ));
                }
            }
        }
        Err(e) => {
            eprintln!("MISSING: .githooks/pre-pushのtarget正規表現がRustのregexクレートで解析できなかった: {e}");
            mismatches.push(format!(
                ".githooks/pre-push: target正規表現の解析エラー: {e}"
            ));
        }
    }

    finish(mismatches)
}

fn finish(mismatches: Vec<String>) -> ExitCode {
    if mismatches.is_empty() {
        println!("\n全ての照合対象が一致しました。");
        ExitCode::SUCCESS
    } else {
        eprintln!("\n不一致を検出しました:");
        for m in &mismatches {
            eprintln!("  - {m}");
        }
        eprintln!(
            "\n宣言側（lints/actuation_call_guard/src/lib.rs::RESTRICTED_CALLS、または \
             .claude/rules/fix-requires-evidence.md）と、architecture_guard.rsのガード期待値・\
             .githooks/pre-pushの対象正規表現のどちらかを更新して一致させること。"
        );
        ExitCode::FAILURE
    }
}
