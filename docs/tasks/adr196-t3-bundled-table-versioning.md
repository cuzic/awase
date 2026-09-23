# ADR-196 T3: 内蔵表への版情報埋め込みとCI週次差分検出を実装する

状態: 未着手（2026-09-23起票）。他ADR-196タスクと独立に着手可能（CI側の変更が中心）。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-196](../adr/196-keymap-learn-truth-priority.md)決定1dは、内蔵表
（`state/key_effect_table.rs`）自身が版ずれしうることをCIで直接検出する経路を定める。
内蔵表は現状、測定時のGJIファイル版・OSビルド・キーボード配列を記録していない。

## 実装対象

1. **生成スクリプトの拡張**: `tools/e2e/ime_key_matrix/gen_key_effect_table.py`が、
   格子学習の実行時にGJIのファイル版・OSビルド・キーボード配列を取得し、
   `key_effect_table.rs`の生成ヘッダ（`const`）へ埋め込む。GJIのファイル版取得は
   [ADR196-T5](adr196-t5-revalidation-not-invalidation.md)が実装する`VS_FIXEDFILEINFO`
   共有関数を流用する（Python側は同等のWin32 API呼び出し、またはRust側の小さなCLIを
   経由）。
2. **CI週次差分検出**: `.github/workflows/e2e-ime.yml`の格子ジョブを定期実行
   （例: 週次cron）し、再生成した表とコミット済み`key_effect_table.rs`を比較する。
   **比較対象は両方で決定的なセルの値だけに限る**（MSIMEプリセットは各セル2試行・
   MSIME_NATIVEは大半1試行で非決定セルの出入りがありうる。決定1d自身がヘッダへ版情報を
   埋め込むためGJI更新のたびにファイル差分が出る）。ヘッダやセルの出入りは報告のみ
   （ジョブを失敗させない）。決定的セルの値そのものが変わった場合のみジョブを失敗させ、
   内蔵表の更新（版情報込みで再コミット）を促す。

## 完了条件

- 生成ヘッダに版情報フィールドが追加され、既存の`key_effect_table.rs`パーサ（利用側）
  が新フィールドを無視して動作することの確認。
- CI差分ジョブが、決定的セルの値変更時のみ失敗し、非決定セルの出入り・ヘッダ差分では
  失敗しないことのテスト（合成データでのドライラン）。

## 関連

- [ADR-196](../adr/196-keymap-learn-truth-priority.md) 決定1d
- [ADR-196 opus-review-round2](../adr/196-opus-review-round2.md) S-a（比較方法の指摘）
- [ADR196-T5](adr196-t5-revalidation-not-invalidation.md)（版取得の共有関数）
