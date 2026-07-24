# コントリビューションガイド

## カスタム lint（dylint）

`lints/` にはレイヤー境界を機械的に強制する 3 つの自作 lint がある。

- `no_vk_as_scan` — `VkCode` の数値から `ScanCode` を構築するコード（別名前空間の混同バグ）を検出する。
- `ime_event_guard` — `ImeEvent::PanicReset` / `HwndCacheRestored` を専用関数の外で構築するコードを検出する。
- `observation_source_guard` — `ImeEvent::InputModeObserved` を、実際には呼んでいない API を偽装した `source`（`ImmGetOpenStatus` は常に禁止、`ConvBitsInference` は `kp_stage_idle_conv_check` 限定）で構築するコードを検出する。

これらは `Cargo.toml` の `[workspace.metadata.dylint]` に登録されており、CI の `dylint` ジョブで自動実行される。

### ローカルでの実行

`rustc_private` を使うため、`lints/*/rust-toolchain` に固定した nightly と `rustc-dev` / `llvm-tools-preview` が必要。

```sh
# 初回のみ: ツールをインストール
cargo install cargo-dylint dylint-link --version 6.0.0 --locked

# 対象は awase-windows。journal.rs 等の #[cfg(windows)] コードも lint 対象に含めるため
# Windows ターゲットへ cross-compile して check する。
# DYLINT_RUSTFLAGS=-D warnings で Warn レベルの lint 違反を失敗として扱う（CI と同一）。
DYLINT_RUSTFLAGS="-D warnings" \
  cargo dylint --all -p awase-windows -- --target x86_64-pc-windows-gnu
```

lint の toolchain バージョン（`lints/*/rust-toolchain` の `channel`）を変更する場合は、
`.github/workflows/ci.yml` の `dylint` ジョブの `toolchain:` も同じ値に揃えること。

## ミューテーションテスト（cargo-mutants）

[cargo-mutants](https://mutants.rs/) はソースコードに小さな変異（比較演算子の反転、
関数本体を `Default::default()` に置換、等）を注入し、既存テストがそれを検出できるか
（=落ちるか）を検証するツール。テストの「網羅率」ではなく「有効性」を計測できる。

設定は `.cargo/mutants.toml` にあり、引数なしで実行するとワークスペースルートの
`awase` パッケージ（プラットフォーム非依存の中核ロジック: `engine/`, `gate.rs`,
`kana_table.rs`, `ngram.rs`, `scanmap.rs`, `yab/` 等）だけを対象にする。CI では
`workflow_dispatch` で手動起動する `mutants` ジョブから実行できる
（`push`/`pull_request` には組み込んでいない。1回のフル実行が数十分かかるため）。

`awase-windows` パッケージは大半が `#[cfg(windows)]` で、Linux 上では
コンパイル自体されないため無指定で回すと unviable/false MISSED だらけになる。
そのうちプラットフォーム非依存な約550 mutants だけを対象にした専用設定が
`.cargo/mutants-awase-windows.toml`（明示的なファイル許可リスト）にあり、
CI では `mutants-awase-windows` ジョブ（同じく `workflow_dispatch` 手動起動）
から実行できる。このリストはファイルの `#[cfg(windows)]` 状況が変わったら
追随して更新が必要（詳細は同 toml 内のコメント参照）。Windows 実機依存の
残りのロジックは対象外（windows-latest ランナーでの実行は将来検討）。

```sh
# 初回のみ: ツールをインストール
cargo install cargo-mutants --locked

# awase パッケージ全体（デフォルト対象、フル実行は時間がかかる）
mise run mutants   # == cargo mutants

# 変更した差分だけを対象にする（レビュー前のセルフチェックに向いている）
git diff main | cargo mutants --in-diff -

# 特定ファイル・特定パッケージに絞る
cargo mutants -f src/gate.rs

# awase-windows のプラットフォーム非依存な部分だけに絞る
cargo mutants -p awase-windows --config .cargo/mutants-awase-windows.toml
```

`MISSED`（変異を検出できなかった＝その部分のテストが弱い）が出た場合、テストを
補強するかどうかは個別に判断する。既存の [experiment-logging](.claude/rules/experiment-logging.md)
や [fix-requires-evidence](.claude/rules/fix-requires-evidence.md) の対象領域
（warmup / focus / belief / conv / キー選択）で `MISSED` が出た場合は特に優先して
テストを追加すること。
