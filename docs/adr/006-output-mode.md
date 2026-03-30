# ADR-006: 出力モード選択 (per_key / batched / unicode)

## ステータス
採用

## コンテキスト
エンジンが生成するローマ字を `SendInput` で注入する際、他のキーボードフック（PowerToys 等）との干渉で文字が取りこぼされる問題が発生した。

- 全文字を1回の `SendInput` にバッチ化 → 一部環境で取りこぼし継続
- 1文字ずつ Sleep で遅延 → メインスレッドをブロック（不可）
- 非同期キュー (WM_TIMER ドリップフィード) → 出力が全く行われない問題発生

原因: `SendInput` のアトミック保証はフックチェーンには適用されない。フックは各イベントを個別に処理する。

## 決定
3つの出力モードを config.toml で選択可能にする:

```toml
[general]
output_mode = "per_key"   # 1文字ずつ個別 SendInput（デフォルト、互換性重視）
output_mode = "batched"   # 全文字を1回の SendInput（高速）
output_mode = "unicode"   # ローマ字→ひらがな変換、KEYEVENTF_UNICODE で直接送信
```

**Unicode モード** は IME を完全にバイパスし、ひらがなを直接テキストフィールドに挿入する。`kana_table.rs` のローマ字→ひらがな変換テーブルを使用。PowerToys Command Palette 等の特殊な入力フィールドでも取りこぼしなし。

## 結果
- Unicode モードで PowerToys Command Palette の取りこぼし問題が完全解決
- ユーザーが環境に応じて最適なモードを選択可能
- 設定は hot-reload 対応

## 却下した代替案
- 非同期出力キュー: WM_TIMER ベースのドリップフィードは出力が行われない問題が発生。原因は NULL HWND タイマーの動作が不安定なためと推定
- Sleep ベースの遅延: メインスレッドブロッキングで LL フックのタイムアウトリスク

## 関連コミット
`50c5b4b`, `7ccc007`, `d1152bf`, `ac2f42c`, `04f8222`
