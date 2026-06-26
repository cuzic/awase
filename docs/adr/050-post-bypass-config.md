# ADR-050: post_bypass — バイパス後キーの NICOLA スキップ設定

## ステータス

採用済み（2026-06-25 実装）

## コンテキスト

### 問題: Ctrl+J 後の Enter が NICOLA エンジンを通過してしまう

tmux でペインを縦分割する操作は `Ctrl+B → J` だが、
`Ctrl+J` は Enter キーとして機能する（tmux のデフォルトバインド）。

awase の Ctrl+key バイパス機構は `Ctrl+J` を NICOLA エンジンを迂回して
そのまま送信するが、直後の次キー入力が NICOLA エンジンを通過する問題があった。

また `Ctrl+J` を送った後 GJI が IME 入力待ちになるため、
J↑（キーリリース）を受けた NICOLA エンジンが誤った判定をするケースがあった。

### 追加で発覚した問題

- **IME ON + Ctrl+J**: GJI が J のキーイベントを横取りし、tmux に届かない
- **Ctrl バイパス後 modifier 先行リリース**: J↑ の前に Ctrl↑ が来ると
  NICOLA エンジンが J↑ を Suppress すべきか判定できず誤動作

これらを修正する中で「バイパス直後の次キーも NICOLA をスキップする」という
一般的な設定が必要なことが明確になった。

### 要求

- 特定の Ctrl+key バイパス後の次キーを NICOLA スキップする
- アプリ別（プロセス名 / クラス名）でフィルタリングできる
- 設定がない場合は従来通り（post_bypass 無効）

## 決定

### [[post_bypass]] TOML 設定の設計（commit a67ebb5）

```toml
[[post_bypass]]
key = "Ctrl+J"
process = "WindowsTerminal"   # wt.exe のプロセス名

[[post_bypass]]
key = "Ctrl+J"
class = "tmux"                # ウィンドウクラス名（将来用）
```

### 型設計の分離

設定ファイルの生 TOML 表現と、実行時に使うコンパイル済み表現を分離した。

```rust
// config.rs — 設定ファイル側（serde deserialize）
pub struct PostBypassRule {
    pub key: String,                // "Ctrl+J" 形式の文字列
    pub process: Option<String>,
    pub class: Option<String>,
}

// runtime/mod.rs — 実行時側（コンパイル済み）
pub struct PostBypassEntry {
    pub vk: u16,                    // VkCode（パース済み）
    pub modifiers: ModifierMask,    // 修飾キーマスク（パース済み）
    pub process: Option<String>,
    pub class: Option<String>,
}
```

**分離の理由**: 文字列 "Ctrl+J" を毎回パースするのは非効率。
また `PostBypassRule` は validation 前のデータなので、`PostBypassEntry` が
持つ VkCode は必ず有効な値であることを型で保証できる。

### コンパイルと格納

```rust
// app/bootstrap.rs
fn compile_post_bypass_rules(rules: &[PostBypassRule]) -> Vec<PostBypassEntry> {
    rules.iter()
         .filter_map(|r| PostBypassEntry::compile(r))
         .collect()
}

// Runtime に格納
runtime.post_bypass_entries = compile_post_bypass_rules(&config.post_bypass);
```

### 照合と適用

```rust
// runtime/message_handlers.rs — バイパスイベント受信時
fn on_ctrl_bypass(vk: u16, modifiers: ModifierMask, hwnd: HWND) {
    let process = get_process_name(hwnd);
    let class   = get_class_name(hwnd);
    
    let matched = runtime.post_bypass_entries.iter().any(|e| {
        e.vk == vk && e.modifiers == modifiers
        && e.process.as_deref().map_or(true, |p| process.contains(p))
        && e.class.as_deref().map_or(true,   |c| class.contains(c))
    });
    
    if matched {
        runtime.post_bypass_active = true;   // 次の1キーをスキップ
    }
}
```

`post_bypass_active` フラグが立っている間、次の1キーは NICOLA エンジンを
スキップして直接 passthrough する。

### 関連修正: Ctrl+J の GJI 横取り問題（commit 32a037d, ee0b1fd）

IME ON 状態で Ctrl+J を送ると GJI が J のキーイベントを消費する問題は、
`handle_bypass` で IME を一時的に OFF にしてから J を送ることで解決した。

この修正は `post_bypass` 設定とは独立しており、Ctrl+key バイパス機構全体に適用される。

## 結果

- tmux での `Ctrl+J` 後の Enter が正しく届くようになった
- プロセス名フィルタリングにより、他のアプリへの誤適用を防止
- `PostBypassRule` / `PostBypassEntry` の分離で型安全なコンパイル処理を実現

## 関連 ADR

- ADR-026: 事前条件とキールーティング（バイパス機構の設計）
- ADR-024: 修飾キーパススルーとping watchdog
- ADR-041: フック再入時の修飾キー整合性保証
