# ADR-037: キーマップ再割当設計

## ステータス

採用済み（2026-05-24）

## コンテキスト

アプリケーションによって「同じキーコンビネーション」が全く別の機能を持つ場合がある。例えば Ctrl+I は Excel では斜体、WezTerm では独自ショートカット。従来は特定のキーをハードコードで特例処理していたが、アプリごとの衝突が増えユーザー設定可能なメカニズムが必要になった。

修飾キー（Ctrl/Shift/Alt）を含む再割当では、送信後の修飾キー残留問題が発生する。例えば「Ctrl+I → F7」を実行すると、F7 送信後も Ctrl が物理的に押下中であれば次のキー入力が Ctrl+? として扱われる。

## 決定

`config.toml` の `[[keymap]]` セクションでアプリケーション単位のショートカット再割当を定義する。

```toml
[[keymap]]
app_class = "Chrome_WidgetWin_1"
from = "Ctrl+I"
to = "F7"
```

実行時は以下の手順で再割当を行う：

1. `from` のキーコンビネーションがマッチしたら元キーを Consume
2. 現在押下中の修飾キーを `HeldModifiers` に記録
3. 記録した修飾キーを全て SendInput(KeyUp) で一時解放
4. `to` のキーコードを SendInput(KeyDown + KeyUp) で送信
5. 元々押下中だった修飾キーを SendInput(KeyDown) で復元

手順 2-5 は `HeldModifiers` 構造体がカプセル化する：

```rust
struct HeldModifiers {
    vks: Vec<VkCode>,  // 解放すべき修飾キー一覧
}
impl HeldModifiers {
    fn release_and_send(self, target: VkCode) { ... }
    fn restore(self) { ... }
}
```

## なぜ修飾キーを解放するか

Windows の `SendInput` は修飾キー状態を自動的に無効化しない。物理的に Ctrl が押下中なら、SendInput(F7) の結果は Ctrl+F7 として受け取られる。修飾キーを明示的に解放してから送信することで、意図した単独キー送信が保証される。

## 結果

- アプリ固有のショートカット衝突をユーザーが設定ファイルで解決できる
- 修飾キー残留による誤入力が解消
- `HeldModifiers` パターンは IME 制御コマンド送信でも共通利用

## 関連 ADR

- ADR-025 (TOML カスタマイズ設計)
- ADR-024 (修飾キー passthrough)
