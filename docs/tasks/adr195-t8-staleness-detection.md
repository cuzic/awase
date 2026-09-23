# ADR-195 T8: 陳腐化検出（段階8）を実装する

**【ADR-196で置換】本タスクが定める「フィンガープリント不一致→即時失効」は、
[ADR-196](../adr/196-keymap-learn-truth-priority.md)決定3（失効ではなく「要再検証」。
GJI/Microsoft IME本体のバージョン相当の情報をフィンガープリントに追加）に置き換わった。
新しい実装対象は[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)を参照。**
**既存の実装ブランチ`feat/adr195-t8-staleness-detection`（developに未マージ、コミット
`3613707e`/`f5d53047`）は旧設計（即時失効）で書かれているため、そのブランチを土台に
ADR-196決定3の差分を当てる形での作り直しを推奨する（ADR196-T5参照。ゼロから作り
直さない）。**

状態: 未着手（2026-09-23起票）。[ADR195-T3](adr195-t3-persistence.md)/
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
- [ADR195-T3](adr195-t3-persistence.md)
- [ADR195-T4](adr195-t4-runtime-loading.md)
