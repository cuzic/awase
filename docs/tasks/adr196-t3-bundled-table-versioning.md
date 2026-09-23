# ADR-196 T3: 内蔵表への版情報埋め込みとCI週次差分検出を実装する

状態: 一部実装済み（2026-09-23、PR #260 `feat/adr196-t3-bundled-table-versioning`）。
実装対象1のうち「版情報埋め込み」は`measurement-env.json`という**プレースホルダ入力**
（実際のGJIファイル版取得は未実装、値は手動/将来のCI側で書き込む前提）をヘッダへ
埋め込む形でコミット済み。実装対象1のうち「`VS_FIXEDFILEINFO`によるGJIファイル版の
実取得」は依然**未着手**——**【S4対応】この部分は[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)
が所有・実装する共有関数を待つ**（Microsoft IME本体側の実装完了は待たなくてよい）。

実装対象2「CI週次差分検出」の**比較ロジック本体**（`--diff-report`、決定的セルの値変更
とセルの出入りを区別）は実装・単体テスト済み（PR #260）。**ただし`.github/workflows/e2e-ime.yml`
自体の配線（週次cronトリガー追加、格子再学習ジョブの新規追加、`grid_learn.py`実行から
`--diff-report`呼び出しまでの自動化）は未着手**——このファイルは550行超のGJI/MS-IME実機
タイミング調整が凝縮された既存資産で、実機Windows CIでの検証なしに一方的に変更するのは
リスクが高いと判断し、着手前に別途ユーザーへ設計案を提示する運用にした
（2026-09-23、詳細は本ファイル末尾「CI配線の設計メモ」参照）。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-196](../adr/196-keymap-learn-truth-priority.md)決定1dは、内蔵表
（`state/key_effect_table.rs`）自身が版ずれしうることをCIで直接検出する経路を定める。
内蔵表は現状、測定時のGJIファイル版・OSビルド・キーボード配列を記録していない。

## 実装対象

1. **生成スクリプトの拡張**: `tools/e2e/ime_key_matrix/gen_key_effect_table.py`が、
   格子学習の実行時にGJIのファイル版・OSビルド・キーボード配列を取得し、
   `key_effect_table.rs`の生成ヘッダ（`const`）へ埋め込む。GJIのファイル版取得は
   [ADR196-T5](adr196-t5-revalidation-not-invalidation.md)が**所有・実装する**
   `VS_FIXEDFILEINFO`共有関数を**利用するだけ**（本タスクでは再実装しない）。Pythonから
   直接Win32 APIを叩いて同等ロジックを再実装する場合は、Rust版と出力（4つの16ビット
   数値）が一致することをテストで固定すること（表記揺れを避けるのがADR-196の目的の
   一部なので、二重実装で再びずれを持ち込まない）。
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

## CI配線の設計メモ（2026-09-23、未確定・投入保留）

`.github/workflows/e2e-ime.yml`を直接改変する前段階として、既存資産の調査結果と
配線案をここに残す。**この節はまだユーザー確認前の設計案であり、実装には入っていない。**

- 現状トリガーは`push`（`ci/e2e-ime`/`ci/e2e-scenarios`/`ci/e2e-calibration`ブランチのみ）
  と`workflow_dispatch`のみ。`schedule:`（週次cron）は存在しない。
- `cal-*`系ジョブは現状すべて`check='collect'`（ログ収集のみ）。`grid_learn.py`/
  `effect_learning.py`/`cycle.py`による解析は**人がローカルで実行**しており、
  CIから表JSONを自動再生成する経路は存在しない（コード内コメントにも明記）。
- 再利用できそうな既存構成: `cal-notify-atok-s{1..4}`・`cal-notify-msimenative-s{1..4}`
  （`--fast --speed=2 --grid-adaptive --notify`、4シャード、比較的高速）。ただし
  **MS-IMEプリセット用の`--notify`付き高速構成は存在しない**（`cal-fast-msime-s{1..4}`
  は`--notify`無しの旧構成のみ）。週次再学習にはこのMS-IME側の高速構成を新設するか、
  `cal-fast-msime-s{1..4}`をそのまま流用するかの判断が要る。
- 想定する配線案（叩き台）: (1) `schedule:`または`workflow_dispatch`限定の新ジョブ群
  （既存の`cal-notify-*`/`cal-fast-msime-*`と同等の構成を複製し3プリセット×シャードで
  格子再学習）→ (2) 各シャードの生ログを`grid_learn.py --json`でJSON化する新ステップ
  （現状人手の部分を初めて自動化）→ (3) 生成物を`gen_key_effect_table.py --diff-report`
  で現行コミットと比較 → (4) 決定的セルの値が変わっていれば（終了コード1）ジョブを
  fail させ、PRで人が確認・再コミットする運用。
- 未確定点: 週次実行の実機コスト（既存`cal-notify-*`だけで数十分オーダー、3プリセット
  ×4シャード分を毎週回すと相応のWindows実機CI時間を消費する）、失敗時の通知先、
  そもそも週次でよいか（月次でも十分か）。これらはユーザーの温度感を確認してから
  実装する。

## 関連

- [ADR-196](../adr/196-keymap-learn-truth-priority.md) 決定1d
- [ADR-196 opus-review-round2](../adr/196-opus-review-round2.md) S-a（比較方法の指摘）
- [ADR196-T5](adr196-t5-revalidation-not-invalidation.md)（版取得の共有関数）
