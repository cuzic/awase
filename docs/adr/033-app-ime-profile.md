# ADR-033: AppImeProfile — アプリ別 IME API 互換性分類

## ステータス

採用済み（2026-05-24）

## コンテキスト

IME 制御 API（ImmSetOpenStatus, VK_IME_ON/OFF, VK_KANJI, TSF 経由）のアプリケーション対応状況がまちまちで、制御ロジックが `ime.rs`, `platform.rs`, `ime_controller.rs` に 8 箇所以上散在していた。新しいアプリを追加するたびに条件分岐の追加漏れが発生し、バグの温床になっていた。

具体的な問題：

- Chrome/Edge は `ImmSetOpenStatus` が届かない（`Chrome_WidgetWin_1` は IMM-broken クラス）
- LINE/Qt は `ImmSetOpenStatus` が別プロセスに届くが `VK_KANJI` が spurious VK_F3/F4 を生成する
- WezTerm / Windows Terminal は IMM32 で読み取れず TSF 経由が必要
- メモ帳 / VSCode は IMM32 で完全制御可能

## 決定

フォーカス変更時にアプリを `AppImeProfile` enum に分類し、フォーカスキャッシュに保存する。以降の IME 制御はプロファイルに基づいて戦略を選択する。

```
AppImeProfile::Imm32Available   → ImmSetOpenStatus（同プロセス）
AppImeProfile::ImmCross         → ImmSetOpenStatus（クロスプロセス）+ KANJI Consume
AppImeProfile::TsfOnly          → VK_DBE_HIRAGANA + probe FSM
AppImeProfile::KanjiToggle      → VK_KANJI（Chrome/Edge 向け）
AppImeProfile::Unknown          → passthrough
```

プロファイルはフォーカス変更イベント時に `focus/classify.rs` が決定し、`HwndImeCache` にウィンドウ単位でキャッシュする。

## 結果

- IME 制御戦略の選択が 1 箇所（`DecisionExecutor::build_ime_control_view`）に集約
- 新アプリ追加時は `classify.rs` にクラス名マッチを追加するだけでよい
- `is_imm_bridge_broken()` のような ad-hoc 判定関数群を削除できた

## 関連 ADR

- [ADR-005](005-focus-classification.md) — フォーカス分類
- [ADR-027](027-ime-state-refresh-and-control.md) — IME 状態 refresh と制御
- [ADR-032](032-ime-state-reducer-4-layer-model.md) — IME 状態 reducer 4 階層
