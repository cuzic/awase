# ADR-002: 入力・処理・出力の3層分離

## ステータス
採用

## コンテキスト
物理キー状態（修飾キー、親指キー）の追跡が Engine FSM 内に混在していた。IME チェックでフックコールバックが早期 return すると Engine の `on_event` が呼ばれず、修飾キーの KeyUp を見逃して stuck する問題が繰り返し発生した。

原因は、物理キー状態追跡と FSM ロジックが同じ関数・同じ構造体に同居していたこと。

## 決定
3層アーキテクチャに分離:

1. **入力層 (InputTracker)**: 物理キー状態追跡。全イベントで無条件実行。`PhysicalKeyState` スナップショットを返す。
2. **処理層 (Engine)**: NICOLA FSM。`PhysicalKeyState` を参照するだけ（所有しない、書き換えない）。`TimedStateMachine` トレイトは不使用。
3. **出力層 (dispatch + SendInputExecutor)**: 変更なし。

`InputTracker.process()` はフックコールバックの最初（IME チェックより前）に呼ばれ、修飾キー・親指キーの追跡が漏れることがない。

## 結果
- modifier stuck 問題が構造的に解消
- 親指キー状態の「漏れ」問題が解消（FSM が thumb_down を書き換えない）
- Engine の `toggle_enabled()` での ModifierState リセットが不要に
- テストは `TestHarness` (InputTracker + Engine) で統合

## 関連コミット
`263aa6d`, `776937c`, `3b8ff02`, `8bb511b`, `b630100`
