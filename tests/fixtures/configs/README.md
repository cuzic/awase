# 合成 config の集まり(ADR-201 決定4-3)

ここにあるファイルは、実際の不具合報告(ADR-095)の config を**元にせず**、
実際のユーザーが書きそうな表記のゆれ(`F12`/`VK_F12`、日本語名、大文字小文字、
空白、Alt なりすまし、旧名 alias、`[[keymap]]`、`[[post_bypass]]` など)を
網羅するように**作った**ものである。実物の config はユーザーの同意がないため
リポジトリに置かない。プロセス名・クラス名・パスも架空の値にしてある。

テスト: `crates/awase-windows/src/config_key_resolution_tests.rs`
(`cargo nextest run --workspace --lib` で走る)。新しいファイルを足したら、
テスト側の `FIXTURE_BASELINE` にも登録すること(未登録だとテストが落ちる)。
