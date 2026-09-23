# ADR-195 T3: 学習結果の永続化（段階3）を実装する

**【ADR-196で一部拡張、2026-09-23追記・Blocker】opus-adversarial-consultのdocs/tasks横断レビュー
（B1）で、PR #251（本タスクの実装）が[ADR-196](../adr/196-keymap-learn-truth-priority.md)を
一切反映しておらず（スキーマは`schema_version`と`cells`のみ）、拡張の担当タスクがどこにも
無いことが判明した。実装対象4として以下のフィールド追加を明記する。**

状態: **developマージ済み（2026-09-23、PR #258〈T9〉経由。元PR #251はsupersededで
クローズ済み、詳細は[adr195-remaining-work-2026-09-23.md](adr195-remaining-work-2026-09-23.md)
参照）。ADR-196拡張フィールドはまだ未反映のまま。**
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

4. **【ADR-196追加】表ファイル全体に次のフィールドを持たせる**（計算・書き込みの主体は
   [ADR196-T2](adr196-t2-mismatch-adjudication.md)/[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)
   だが、フィールド自体の定義・シリアライズはT3の責務）:
   - 自己検証の**正答率・予測したステップ数・全ステップ数・乱数シード**（決定1a、
     `196-...md:92`）。
   - **判定結果**（採用／要確認／不採用）と、その理由・スコア（決定1e、`196-...md:133`）。
   - **フィンガープリント**（決定3a、`196-...md:172,176`）。値は「一致する版」「不一致」
     に加えて「不明」（版取得失敗、比較除外）「未確定」（更新直後、常に不一致扱い）を
     区別できる形にする（真偽値2値ではなく3〜4値の列挙型にする）。
   - **採点日時**（軽量再検証のアトミック書き直し対象、決定3a）。
   - 不具合報告への添付用に、不一致セルの一覧・観測結果・再測定結果を保持できる構造
     （決定1b末尾、`196-...md:123`。[ADR196-T2](adr196-t2-mismatch-adjudication.md)参照）。

## 完了条件

- 永続化フォーマットのシリアライズ/デシリアライズのテスト。
- スキーマバージョン不一致を検出するテスト（T8実装前でもフィールド自体の読み書きは
  ここで確認できる）。
- 上記4のフィールド（特にフィンガープリントの「不明」「未確定」の3〜4値）のシリアライズ/
  デシリアライズのテスト。

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md) 段階3
- [ADR-196](../adr/196-keymap-learn-truth-priority.md) 決定1a・1e・3a
- [ADR195-T4](adr195-t4-runtime-loading.md)（このファイルを読み込む側）
- [ADR195-T8](adr195-t8-staleness-detection.md)（スキーマ版不一致による失効。ADR-196決定3で
  「即時失効」から「要再検証」に置換）
- [ADR196-T2](adr196-t2-mismatch-adjudication.md)・[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)
  （上記4フィールドの計算・書き込み主体）
