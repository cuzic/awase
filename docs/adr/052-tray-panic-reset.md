# ADR-052: トレイメニューからのパニックリセット

## ステータス

採用済み（2026-06-27 実装）

## コンテキスト

### 既存のパニックリセット機構

awase には内部状態の完全初期化（パニックリセット）が実装されている。

- **トリガー**: IME OFF → ON → OFF の3操作を 2000ms 以内に連打
- **実装**: `panic_detect.rs` の `RapidPressTracker` が検出 → `WM_PANIC_RESET` を post
- **処理**: `runtime/mod.rs` の `panic_reset()` が Engine / FSM / IME モデルを初期化

```
IME OFF → ON → OFF (2s以内)
    ↓
RapidPressTracker::push() が true を返す
    ↓
post_to_main_thread(WM_PANIC_RESET)
    ↓
Runtime::panic_reset() — 全内部状態を初期化
```

### 問題: キーシーケンスが使えない状況がある

パニックリセットが必要になる場面（stuck modifier、IME 状態不整合、
TSF コンテキスト破損等）は、同時にキー入力が正常に機能しない状況でもある。

- Ctrl がスタックした状態では修飾キー付きの操作が誤動作する
- IME が壊れている場合、IME ON/OFF キー自体が期待通り動かないことがある
- キーボードフックが異常状態にある場合、シーケンス検出に届かない可能性がある

つまり「パニックリセットが最も必要なとき」に「トリガーシーケンスが最も使いにくい」
という矛盾がある。

### トレイアイコンは常に到達可能

トレイアイコンのメニューはマウス操作で開くため、キーボード状態に依存しない。
Win32 の `WM_RBUTTONUP` → `TrackPopupMenu` → `WM_COMMAND` の経路は
awase のキーボードフックを経由しない。

## 決定

トレイメニューに「内部状態をリセット」項目を追加し、
選択時に直接 `WM_PANIC_RESET` を post する（commit 1591593）。

```
[トレイアイコン 右クリック]
    → 内部状態をリセット
        ↓
post_to_main_thread(WM_PANIC_RESET)   ← キーシーケンスと同じ経路
        ↓
Runtime::panic_reset()
```

### 実装の最小性

既存の `WM_PANIC_RESET` 処理を再利用するため、追加コードは最小：

```rust
// tray.rs
const IDM_PANIC_RESET: u16 = 56;

pub enum TrayCommand {
    // ...
    PanicReset,
}

// handle_tray_message: メニュー項目追加
append_menu_item(hmenu, IDM_PANIC_RESET, "内部状態をリセット");

// WM_COMMAND ハンドラ
Some(TrayCommand::PanicReset) => {
    log::warn!("Panic reset requested from tray menu");
    crate::win32::post_to_main_thread(crate::WM_PANIC_RESET);
}
```

### メニュー内の位置

「学習キャッシュをクリア」の直下に配置した。どちらも「状態を消去する」
操作であり、ユーザーが「何か変なことが起きたときの修復手段」として
探す場所として自然。

## 検討した代替案

**確認ダイアログを挟む**  
→ 採用しなかった。パニックリセットは副作用が小さく（進行中の変換が消える程度）、
緊急時に素早く実行できることを優先した。誤操作のリスクより利便性が上回る。

**メニュー項目名を「パニックリセット」にする**  
→ 採用しなかった。「内部状態をリセット」のほうが、技術用語を知らないユーザーにも
意味が伝わる。

## 結果

- キーボード操作が困難な状態でもトレイメニューから確実にパニックリセットを実行可能
- 既存の `WM_PANIC_RESET` 処理を完全に再利用（8行の追加のみ）
- キーシーケンストリガーとトレイメニュートリガーが同一の処理経路を通る

## 関連 ADR

- ADR-041: フック再入時の修飾キー整合性保証（stuck modifier の背景）
