//! P1: ジャーナル・リプレイ回帰基盤。
//!
//! # 目的
//! 過去の再発バグ（fc18cc7 / 109b4c9 / 1544d3f / ea3da7f 等）は、実機でしか観測できない
//! conv ビットの組合せが原因だった。`tests/journals/*.json` に「実際に観測された
//! `classify_conv_transition` の入力＋期待出力」をフィクスチャとして蓄積し、この
//! テストが毎回再実行して一致を確認する。**実機でしか観測できない事象を、観測した
//! 瞬間に固定化する**のが狙い。
//!
//! `state::conv_classify` モジュールは `#[cfg(windows)]` でゲートされていない
//! （唯一の呼び出し元 `runtime/key_pipeline.rs` が Windows 専用なだけ）ため、
//! このテストは Linux ホストでもそのまま実行できる。
//!
//! # フィクスチャの追加手順
//! 詳細は `docs/journal-replay-guide.md` を参照。要点:
//! 1. 実機でバグに気づいたら、**修正する前に** ホットキー（Alt+変換→Alt+無変換 を
//!    2 回連続）でジャーナルをダンプする（`journal.rs`）。
//! 2. ダンプ JSON から該当する `ConvClassifyCall` エントリを見つけ、このディレクトリに
//!    `ConvClassifyFixture` 形式（`name`/`note`/`conv`/`current`/`is_cold`/
//!    `effective_open`/`conv_mode_changed`/`is_roman_reliable`/`expected`）で転記する。
//! 3. 転記した直後の `expected` は「実際に起きたバグの出力」なので、**必ず** 手で
//!    「あるべき出力」に書き換えてからコミットする（そうしないとこのテストはバグを
//!    固定化してしまう）。
//! 4. 修正を実装し、このテストが通ることを確認する。

use awase_windows::state::conv_classify::{classify_conv_transition, ConvClassifyFixture};

fn load_fixtures(path: &std::path::Path) -> Vec<ConvClassifyFixture> {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("フィクスチャ読み込み失敗 {}: {e}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("フィクスチャのJSONパース失敗 {}: {e}", path.display()))
}

#[test]
fn replay_all_journal_fixtures() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/journals");
    let mut failures = Vec::new();
    let mut total = 0usize;

    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} が読めない: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry read failed").path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    paths.sort();

    for path in &paths {
        for fixture in load_fixtures(path) {
            total += 1;
            let actual = classify_conv_transition(
                fixture.conv,
                fixture.current,
                fixture.is_cold,
                fixture.effective_open,
                fixture.conv_mode_changed,
                fixture.is_roman_reliable,
            );
            if actual != fixture.expected {
                failures.push(format!(
                    "[{}] {} ({}):\n  expected: {:?}\n  actual:   {:?}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    fixture.name,
                    fixture.note,
                    fixture.expected,
                    actual,
                ));
            }
        }
    }

    assert!(total > 0, "tests/journals/ にフィクスチャが1件もない");
    assert!(
        failures.is_empty(),
        "{} 件のジャーナル・リプレイ不一致（実機で観測済みの入力に対する退行）:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
