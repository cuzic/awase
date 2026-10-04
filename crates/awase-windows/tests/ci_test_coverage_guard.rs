//! `crates/awase-windows/tests/*.rs` の統合テストが、すべて `.github/workflows/ci.yml` の
//! `--test <name>` に列挙されていることを固定する。
//!
//! 統合テストは `--test` で明示しないと `cargo nextest run --lib` にも含まれず、
//! エラーも警告も出さずに一度も実行されない。2026-10-04、`closed_loop_scenarios` を
//! 含む6本がこの形で CI から漏れていたことが発覚した（ADR-212 は「closed_loop_scenarios
//! で固定する」としていたが、実際には回帰ガードとして機能していなかった）。
//! 新しい統合テストを足したら ci.yml にも列挙すること。

use std::fs;
use std::path::Path;

#[test]
fn every_integration_test_is_listed_in_ci() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ci = fs::read_to_string(manifest_dir.join("../../.github/workflows/ci.yml"))
        .expect("ci.yml が読めること");

    let mut missing = Vec::new();
    for entry in fs::read_dir(manifest_dir.join("tests")).expect("tests/ が読めること") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("file stem");
        // `--test NAME` の後ろが空白・行末のときだけ一致とする(接頭辞の偶然一致を避ける)。
        let listed = ci.lines().any(|l| {
            !l.trim_start().starts_with('#')
                && l.split_whitespace()
                    .zip(l.split_whitespace().skip(1))
                    .any(|(flag, n)| flag == "--test" && n == name)
        });
        if !listed {
            missing.push(name.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "ci.yml の `--test` に無い統合テスト(CI で一度も走らない): {missing:?}"
    );
}
