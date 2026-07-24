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
`kana_table.rs`, `ngram.rs`, `scanmap.rs`, `yab/` 等）だけを対象にする。CI には
組み込んでいない（1回のフル実行が数十分かかるため）。ローカルで手動、または
気になる差分だけを絞って実行する。

```sh
# 初回のみ: ツールをインストール
cargo install cargo-mutants --locked

# awase パッケージ全体（デフォルト対象、フル実行は時間がかかる）
mise run mutants   # == cargo mutants

# 変更した差分だけを対象にする（レビュー前のセルフチェックに向いている）
git diff main | cargo mutants --in-diff -

# 特定ファイル・特定パッケージに絞る
cargo mutants -f src/gate.rs
cargo mutants -p awase-windows   # 大半は #[cfg(windows)] のため Linux では
                                  # unviable 判定になるファイルが多い。
                                  # Windows 実機で回すのが本命。
```

`MISSED`（変異を検出できなかった＝その部分のテストが弱い）が出た場合、テストを
補強するかどうかは個別に判断する。既存の [experiment-logging](.claude/rules/experiment-logging.md)
や [fix-requires-evidence](.claude/rules/fix-requires-evidence.md) の対象領域
（warmup / focus / belief / conv / キー選択）で `MISSED` が出た場合は特に優先して
テストを追加すること。
