# コントリビューションガイド

## カスタム lint（dylint）

`lints/` にはレイヤー境界を機械的に強制する 2 つの自作 lint がある。

- `no_vk_as_scan` — `VkCode` の数値から `ScanCode` を構築するコード（別名前空間の混同バグ）を検出する。
- `ime_event_guard` — `ImeEvent::PanicReset` / `HwndCacheRestored` を専用関数の外で構築するコードを検出する。

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
