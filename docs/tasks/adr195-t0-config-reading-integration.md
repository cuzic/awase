# ADR-195 T0: 設定の読み取りラウンド（段階0）を実装する

状態: 未着手（2026-09-23起票）。ADR-192実装完了を待たずに暫定実装として着手可（下記参照）。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-195](../adr/195-keymap-learn-productization.md)決定・段階0は、develop に既にある
3つの部分的なキーマップ読み取り経路（経路1: `from_config`のプリセット選択、経路2:
`extract_ime_keys`/`extract_mode_keys`〈`crates/awase-gji-config/src/keymap.rs:192`〉、
経路3: `msime_key_assignment.rs`のMS-IME本体レジストリ再割り当て検出）を、キー単位の
1つの構造体（初期仮説表S）へ統合する。新規構築ではなく統合であり、見積もりは薄い集約関数
（100行未満）+`extract_mode_keys`への小拡張（50行未満）。

**ADR-192との依存関係（重要、誤解しやすい点）**: 「キーマップから状態依存キーを機械的に
検出する」ロジック自体は[ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md)
決定1が所有・実装する。ADR-192が本タスクより先に実装されていない場合、**本タスクは
`extract_mode_keys`を直接呼ぶ暫定実装として今すぐ着手してよい**（ADR-195本文が明記する
方針）。ADR-192実装後に、その出力（`state_dependent_key_warning.rs`等が公開する判定結果）
へ差し替える。どちらが先でも検出ロジックの実装箇所は1つに保つ。

## 実装対象

1. 経路1〜3の出力をキー単位の1つの構造体（初期仮説表S）にまとめる集約関数を実装する。
2. `extract_mode_keys`に小拡張: `SetMode`（絶対設定系）コマンドがComposition/Conversion
   （入力中）のstatus行に束縛されていれば、コマンド名だけから初期仮説の予測値を直接
   組み立てる（学習を省略できる）。相対トグル系（`ToggleAlphanumericMode`/
   `ToggleKanaType`）は「不明、要学習」のままにする。`KeymapRow`が既に`status`を持つため
   新しい解析は不要（見積もり50行未満+テスト）。
3. Microsoft IME本体には経路2に相当するものが無いため、初期仮説Sは常に空（全キー要学習）
   になることを確認するテストを書く。
4. charset自動検出や設定への書き戻しは作らない（ADR-191決定5を踏襲、スコープ外）。
5. 段階0の出力は「[ADR195-T1](adr195-t1-independent-learning-process.md)が測定すべき
   キーの集合（初期仮説が不明のまま残ったキー）」と「初期仮説の表S」の2つ。

## 完了条件

- 経路1〜3の統合テスト、および`SetMode`小拡張のテストが通る。
- ADR-192の検出ロジックが未実装の場合、暫定実装であることをコード上のコメント（1行）と
  コミットメッセージに明記する。

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md) 段階0
- [ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md) 決定1
  （検出ロジックの本来の所有者。実装済みならこのタスクの成果を差し替える）
