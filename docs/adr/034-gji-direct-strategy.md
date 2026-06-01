# ADR-034: GJI Direct Strategy — Google 日本語入力との協調設計

## ステータス

採用済み（2026-05-24）

## コンテキスト

Chrome/Edge では `ImmSetOpenStatus` が無効で `VK_KANJI` によるトグルしか機能しない。しかし VK_KANJI は **トグル** キーのため：

- 現在の IME 状態を正確に把握していないと二重トグルが発生する
- shadow_model との乖離が起きると逆効果になる（OFF にしたいのに ON になる）

加えて、Ctrl+Shift+Delete（旧 GJI ショートカット）はブラウザの DevTools ショートカットと衝突し、Ctrl+Shift+M, F14 等の代替を探す試行錯誤が必要だった。

## 決定

Google 日本語入力（GJI）インストール済み環境では、`awase-gji-setup` ユーティリティで GJI の設定ファイル（`config1.db`）に「F14 → IME オフ」「F13 → IME オン」エントリを冪等パッチし、awase はこの既知ショートカットで IME を制御する。

```
awase-gji-setup（初回セットアップ）
  → config1.db に F13/F14 エントリを追加（既存エントリがあればスキップ）

awase 実行時（Chrome/Edge フォーカス + GJI 環境）
  → IME ON: SendInput(F13)
  → IME OFF: SendInput(F14)
```

GJI 未導入環境では `KanjiToggle` 戦略（VK_KANJI + shadow チェック）にフォールバックする。

### なぜ config1.db パッチか

- GJI は独自ショートカット定義を SQLite ベースの `config1.db` で管理する
- F13/F14 は標準キーボードに存在せず他ショートカットと衝突しない
- IMM32 シム経由（SendMessageTimeout）は Chrome で失敗・タイムアウトが多く不安定

## 結果

- Chrome/Edge での IME ON/OFF が安定（shadow desync による逆トグルが解消）
- `awase-gji-setup` binary により初期セットアップが 1 コマンドで完結
- GJI 未導入ユーザーも KanjiToggle フォールバックで動作

## 関連 ADR

- [ADR-0003](0003-chrome-vk-injection.md) — Chrome VK injection
- [ADR-033](033-app-ime-profile.md) — AppImeProfile
