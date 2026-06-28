# ADR-058: InjectionMode の cache.toml 永続化

## ステータス

採用済み（2026-06-25 実装）

## コンテキスト

### InjectionMode の学習とは

awase は Windows アプリケーションへのキー注入に Unicode/Vk/Tsf の3つのモードを使い分ける。
どのモードを使うかはウィンドウクラス名（`class_name`）ごとに `ForceOverrides`（設定ファイル）
または GJI write 観測による動的学習で決定される。

TSF 経由の書き込みが必要と観測されたクラスは `InjectionHint::ForceTsf` に昇格する。
この「事後昇格」情報はプロセスメモリにのみ存在したため、**再起動のたびにリセット**されていた。

### 問題

- 再起動後の初回フォーカスでは ForceTsf が適用されず、cold-start コストが発生する
- IMM32 能力のキャッシュ（`ImmCapabilityStore`）は既に `imm_cache.toml` で永続化されていたが、
  injection mode は永続化されていなかった
- 2 つの学習キャッシュが別ファイルに分散すると「学習キャッシュをクリア」操作が複雑になる

## 決定

### Step 1: imm_cache.toml → cache.toml に統合（commit 10918c2）

将来のセクション追加に備え、ファイルを `cache.toml` に統合し、
セクション名を `[classes]` から用途を明示した `[imm_capability]` に変更した。

```toml
# cache.toml
[imm_capability]
"TTOTAL_MAIN_WINDOW" = "works"
"Chrome_RenderWidgetHostHWND" = "unavailable"
```

旧ファイル `imm_cache.toml` は `clear()` 時に残骸も削除する。
キャッシュは再学習可能なためデータ移行は行わない。

### Step 2: InjectionModeStore の新設（commit c6fed60）

`[injection_mode]` セクションに `class_name = "tsf"` の形式で保存する
`InjectionModeStore` 構造体を `classifier.rs` に追加した。

```rust
pub struct InjectionModeStore {
    tsf_classes: std::collections::HashSet<String>,
    base_dir: std::path::PathBuf,
}

impl InjectionModeStore {
    pub(crate) fn has_tsf(&self, class_name: &str) -> bool {
        self.tsf_classes.contains(class_name)
    }

    pub(crate) fn learn_tsf(&mut self, class_name: String) {
        if self.tsf_classes.insert(class_name) {
            self.save(); // 冪等：新規追加時のみ書き込む
        }
    }
}
```

セクション単位の merge-write を行う共通ヘルパー `save_section()` を追加し、
`ImmCapabilityStore` と `InjectionModeStore` の両方が `cache.toml` を共有しながら
互いのセクションを上書きしないようにした。

```rust
fn save_section(base_dir: &std::path::Path, section_name: &str, section: toml::Table) {
    let path = base_dir.join(CACHE_FILENAME);
    let mut root: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or_default();
    root.insert(section_name.to_string(), toml::Value::Table(section));
    let content = toml::to_string_pretty(&root).unwrap_or_default();
    // ...
}
```

### 組み込み方

`FocusTracker` に `injection_mode_store` フィールドを追加し、
`injection_hint()` / `injection_hint_for()` の末尾でストアを参照する。

```rust
let hint = self.overrides.injection_hint(pid, class_name);
if hint != InjectionHint::Default {
    return hint;
}
if self.injection_mode_store.has_tsf(class_name) {
    return InjectionHint::ForceTsf;
}
InjectionHint::Default
```

`bootstrap.rs` では `InjectionModeStore::new(base_dir)` を
`FocusTracker::new()` の引数として渡すだけで完結する。

## なぜこの設計か / 検討した代替案

| 案 | 評価 |
|---|---|
| 設定ファイル（awase.toml）に書く | ユーザー編集対象と自動学習を混在させるべきでない |
| injection mode 専用ファイルを新設 | キャッシュ系ファイルが増えると「クリア」操作が煩雑になる |
| SQLite | 依存が重い。TOML で十分なデータ量 |
| **cache.toml にセクションで共存** | 採用。クリアが1ファイル削除で完結し、移行・拡張が容易 |

## 結果

- 再起動後も Tsf 強制が即座に適用され、cold-start での injection 誤判定が減少する
- `cache.toml` の `[imm_capability]` + `[injection_mode]` に学習キャッシュが集約され、
  トレイメニューの「学習キャッシュをクリア」でファイルを1つ削除するだけで一括リセットできる
- `save_section()` ヘルパーにより、将来セクションを追加しても他のセクションを壊さない

## 関連 ADR

- ADR-004: Injection Mode 設計（Unicode/Vk/Tsf の使い分け方針）
- ADR-033: AppImeProfile（アプリごとの IME プロファイルと ForceOverrides）
- ADR-034: GJI Direct Strategy（GJI write 観測による事後昇格のトリガー）
