# ADR-195 T3: 学習結果の永続化（段階3）を実装する

状態: 未着手（2026-09-23起票）。[ADR195-T2](adr195-t2-self-verification.md)完了後に着手
（着手自体は設計独立なのでT2と並行しても良い）。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-195](../adr/195-keymap-learn-productization.md)決定・段階3は、T1/T2の学習結果を
永続化する。[ADR-176](../adr/176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md)
決定6（176-T11）が定めた`config.toml`の`[[calibration]]`スキーマは「1キー1件」の粒度だが、
本ADRの表は`(状態, キー)`→効果のセル単位なので粒度が違う。`[[calibration]]`は拡張せず、
別ファイル（例: `<config dir>/keymap-learn-table.json`）を新設する。

## 実装対象

1. 表全体を1ファイルに持つ独立フォーマットを新設する。
2. **スキーマの所有クレート**: 型は`awase-windows`側ではなく**OS非依存の
   `awase-keymap-learn`側で定義する**（書き手`awase-keymap-learn-win`→
   `awase-keymap-learn`の依存で成立させ、`awase-windows`→`awase-keymap-learn`の
   依存は段階4側で発生させる。既存の依存の向き〈coreはOS非依存クレートに依存される
   だけ〉と整合させるため）。
3. **スキーマバージョン番号を1フィールド持たせる**: [ADR195-T5](adr195-t5-mealy-machine-minimization.md)
   （隠れ状態を最小Mealy機械へ置き換え）はdevelop側の状態表現を変えるため、T5より前に
   永続化した表はT5実装後にスキーマ不一致になる。[ADR195-T8](adr195-t8-staleness-detection.md)
   の失効条件に「スキーマ版が現行と違う」を追加できるよう、このフィールドを用意する。

## 完了条件

- 永続化フォーマットのシリアライズ/デシリアライズのテスト。
- スキーマバージョン不一致を検出するテスト（T8実装前でもフィールド自体の読み書きは
  ここで確認できる）。

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md) 段階3
- [ADR195-T4](adr195-t4-runtime-loading.md)（このファイルを読み込む側）
- [ADR195-T8](adr195-t8-staleness-detection.md)（スキーマ版不一致による失効）
