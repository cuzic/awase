# ADR-196 T3: 内蔵表への版情報埋め込みとCI週次差分検出を実装する

状態: 一部実装済み（2026-09-23、PR #260 `feat/adr196-t3-bundled-table-versioning`）。
実装対象1のうち「版情報埋め込み」は`measurement-env.json`という**プレースホルダ入力**
（実際のGJIファイル版取得は未実装、値は手動/将来のCI側で書き込む前提）をヘッダへ
埋め込む形でコミット済み。実装対象1のうち「`VS_FIXEDFILEINFO`によるGJIファイル版の
実取得」は依然**未着手**——**【S4対応】この部分は[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)
が所有・実装する共有関数を待つ**（Microsoft IME本体側の実装完了は待たなくてよい）。
**【2026-09-24追記】待っていた共有関数はPR #279（`awase-keymap-learn-win/src/env_version.rs`の`file_version`、GJI側のみ）でdevelop統合済み。`measurement-env.json`への実値投入・CI配線は引き続き未着手。**

実装対象2「CI差分検出」の**比較ロジック本体**（`--diff-report`、決定的セルの値変更と
セルの出入りを区別）は実装・単体テスト済み（PR #260）。**`.github/workflows/e2e-ime.yml`
自体の配線（格子再学習ジョブの新規追加、`grid_learn.py`実行から`--diff-report`呼び出し
までの自動化）は未着手のまま今回は投入しないことにした**（2026-09-23、ユーザー判断）。
**週次cron等の定期実行は不要**——既存の`windows-latest`実機CIで検証できること自体は
分かっている（`cal-notify-*`等が既にそれをやっている）が、「定期実行するか・どう配線
するか」は今判断すべきことではないという整理。将来この自動化に着手する場合は
`workflow_dispatch`手動実行止まりで十分か、以下の設計メモから再検討すること。
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
2. **CI差分検出**: `.github/workflows/e2e-ime.yml`の格子ジョブを実行
   （定期実行〈cron〉は不要、`workflow_dispatch`手動実行で足りる——2026-09-23ユーザー判断）
   し、再生成した表とコミット済み`key_effect_table.rs`を比較する。
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

## CI配線の設計メモ（2026-09-23、参考情報・着手時期は未定）

`.github/workflows/e2e-ime.yml`を直接改変する前段階として調べた既存資産の情報と
配線の叩き台をここに残す。**定期実行(cron)は不要と判断済み**。自動化自体に
着手するかどうかも今回は判断せず先送りにした——将来このタスクに戻ってくる
セッションのための参考情報。

- 現状トリガーは`push`（`ci/e2e-ime`/`ci/e2e-scenarios`/`ci/e2e-calibration`ブランチのみ）
  と`workflow_dispatch`のみ。着手するとしても`workflow_dispatch`手動実行で足り、
  `schedule:`は追加不要。
- `cal-*`系ジョブは現状すべて`check='collect'`（ログ収集のみ）。`grid_learn.py`/
  `effect_learning.py`/`cycle.py`による解析は**人がローカルで実行**しており、
  CIから表JSONを自動再生成する経路は存在しない（コード内コメントにも明記）。
- 再利用できそうな既存構成: `cal-notify-atok-s{1..4}`・`cal-notify-msimenative-s{1..4}`
  （`--fast --speed=2 --grid-adaptive --notify`、4シャード、比較的高速）。ただし
  **MS-IMEプリセット用の`--notify`付き高速構成は存在しない**（`cal-fast-msime-s{1..4}`
  は`--notify`無しの旧構成のみ）。再学習にはこのMS-IME側の高速構成を新設するか、
  `cal-fast-msime-s{1..4}`をそのまま流用するかの判断が要る。
- 想定する配線案（叩き台）: (1) `workflow_dispatch`限定の新ジョブ群（既存の
  `cal-notify-*`/`cal-fast-msime-*`と同等の構成を複製し3プリセット×シャードで
  格子再学習）→ (2) 各シャードの生ログを`grid_learn.py --json`でJSON化する新ステップ
  （現状人手の部分を初めて自動化）→ (3) 生成物を`gen_key_effect_table.py --diff-report`
  で現行コミットと比較 → (4) 決定的セルの値が変わっていれば（終了コード1）ジョブを
  fail させ、PRで人が確認・再コミットする運用。

## 関連

- [ADR-196](../adr/196-keymap-learn-truth-priority.md) 決定1d
- [ADR-196 opus-review-round2](../adr/196-opus-review-round2.md) S-a（比較方法の指摘）
- [ADR196-T5](adr196-t5-revalidation-not-invalidation.md)（版取得の共有関数）
