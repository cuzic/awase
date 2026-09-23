# ADR-195 T8: 陳腐化検出（段階8）を実装する

**【ADR-196で一部置換、M5対応で範囲を精密化（2026-09-23）】置き換わるのは「不一致時の
動作」だけである。実装対象1（キーマップ設定のフィンガープリント）・実装対象2（スキーマ版
不一致での失効）は、[ADR-196](../adr/196-keymap-learn-truth-priority.md)決定3aへの追記
（`196-...md`3b直前の段落）により**そのまま有効**——「即時失効」のままでよい。置き換わるのは
GJI/Microsoft IME本体の**バージョン相当の情報**が不一致だった場合の扱いのみで、こちらは
「失効」ではなく「要再検証」になる。つまり本タスクの実装対象1・2で作る仕組みに、
[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)が**別枠のフィンガープリント**
（版相当の情報、要再検証の判定）を追加する構成になる。両者を1つの`Staleness`（stale/fresh
の2値）に混ぜないこと——fresh/要再検証/失効の3状態、かつ「要再検証」の原因はバージョン相当の
情報の不一致に限る。**
**既存の実装ブランチ`feat/adr195-t8-staleness-detection`（PR #253、developに未マージ、
コミット`3613707e`/`f5d53047`）の「フィンガープリント不一致→即時失効」というロジック自体
（実装対象1・2）は土台として活かせる。PR #253にコメント済み。[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)は、
これに「バージョン相当の情報」という別枠のフィンガープリントと3状態化を追加する差分になる。**

状態: **実装中（PR #253、独立にテスト可能な判定ロジックのみ先行実装。ADR-196対応は
未反映。2026-09-23時点）**。[ADR195-T3](adr195-t3-persistence.md)/
[ADR195-T4](adr195-t4-runtime-loading.md)完了後に着手。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-195](../adr/195-keymap-learn-productization.md)決定・段階8は、キーマップ
（`config1.db`、レジストリのキー割り当て）が変わったら、学習済みの表を失効させ、
[ADR195-T4](adr195-t4-runtime-loading.md)が既定値へフォールバックするようにする。

## 実装対象

1. **フィンガープリントの粒度**: T3の永続化が「1キー1件」ではなく表全体の1ファイルで
   あるため、ADR-176 176-T12の`relevant_rows_for_vk`（1VKごとの部分文字列）ではなく、
   `config1_db_stamp()`（`config1.db`全体のmtime+len）または`session_keymap`/
   `custom_keymap_table`/`overlay_keymaps`3値のハッシュを、**表ファイル全体の1つの
   フィンガープリント**として使う。
2. **スキーマ版不一致でも失効させる**: キーマップ自体は変わっていなくても、永続化
   ファイルのスキーマバージョン（[ADR195-T3](adr195-t3-persistence.md)が持たせる
   フィールド）が現行の予測器実装と異なる場合も、同じ失効経路（既定値へフォールバック）
   を通す（[ADR195-T5](adr195-t5-mealy-machine-minimization.md)が状態表現を変える
   ため必須）。

## 完了条件

- フィンガープリント計算のテスト（`config1.db`変更検出・3値ハッシュのどちらの方式を
  採るか決定し、期待値を固定する）。
- スキーマ版不一致による失効のテスト。

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md) 段階8
- [ADR-176](../adr/176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md) 176-T12
- [ADR-196](../adr/196-keymap-learn-truth-priority.md) 決定3（バージョン相当の情報の追加分のみ置換）
- [ADR195-T3](adr195-t3-persistence.md)
- [ADR195-T4](adr195-t4-runtime-loading.md)
- [ADR196-T5](adr196-t5-revalidation-not-invalidation.md)（本タスクを土台にする後継タスク）
