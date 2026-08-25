# 不具合報告（タスクトレイ「不具合を報告」機能）トリアージ台帳

タスクトレイの「不具合を報告」機能（[ADR-095](adr/095-tray-bug-report-cloudflare-intake.md)）が
Cloudflare R2 (`awase-report-bucket`) に保存した個々の報告について、確認済みかどうか・
どの既知バグに該当するか・対応状況を記録する台帳。

`.claude/skills/bug-report-fetch` で report_id を調査する際は、**まずこの台帳に
既存の行があるか確認し**、無ければ調査後にここへ追記する（同じ報告を毎回ゼロから
再調査しないため）。

## 列の意味

- **report_id**: R2オブジェクトキー末尾のULID。
- **reported_at**: 報告送信時刻（UTC、`payload.reported_at`）。
- **症状**: `payload.description` の要約 + symptom_category。
- **原因/該当バグ**: journal・app_log から特定した原因。既存の `known-bugs.md` の
  BUG-N に対応する場合はその番号を書く。
- **対応状況**:
  - `未トリアージ` — まだ調査していない
  - `対応不要` — 実際の不具合ではない（テスト送信・情報不足で再現不可 等）
  - `対応済み(未リリース)` — 修正コミット/PRは存在するが、報告時点の最新リリースには未反映
  - `対応済み(vX.Y.Z〜)` — 該当バージョン以降のリリースに修正が含まれる
  - `未対応` — 原因は特定済みだが修正がまだ存在しない、または修正がdevelop/mainに未マージ
- **メモ**: 修正コミットハッシュ・PR番号・ブランチ名など。

## 台帳

| report_id | reported_at (UTC) | app_version | 症状 | 原因/該当バグ | 対応状況 | メモ |
|---|---|---|---|---|---|---|
| `01M0QPJVRZJTTG7ZZ8S0ZFXHJQ` | 2026-08-23T16:16:54Z | 1.14.0 | description="テスト"（具体症状記載なし）、symptom_category=WrongCharacterOutput | journal上もgive_up/StaleConfirm等の異常パターンなし | 対応不要 | 機能テスト目的の送信とみられる |
| `01M0RE56S6EQ4MTQGJ2EB2W4N0` | 2026-08-23T23:08:34Z | 1.14.0 | 「こういう」と入力→「ういう」（先頭「こ」欠落） | BUG-74: `RawTsfLiteralRecovery` の2連続give-upで文字が痕跡なく消える。app_logに `re-send "ko" scheduled` 直後 `giving up...no re-send` | 対応済み(未リリース) | 修正はPR #97（`fix/bug74-raw-tsf-literal-giveup-drops-char`）でdevelopにマージ済み(2026-08-24 01:37 UTC)。マージがv1.16.0タグ(08-24 00:29 UTC)より後のため v1.16.0 には未反映。次リリースで解消見込み |
| `01M0RN7RR031SFSP8H0RCM9QAH` | 2026-08-24T01:12:22Z | 1.14.0 | 「りよう」と入力→「よう」（先頭「り」欠落） | BUG-74（上記と同一原因）。app_logに `re-send "ri" scheduled` → `giving up` | 対応済み(未リリース) | 上記と同じくPR #97で対応済み・未リリース |
| `01M0S4S6R4C1YJ581YJ9ZGAXXD` | 2026-08-24T05:43:45Z | 1.14.0 | 「つかって」と入力→「っつかって」（先頭が二重化） | BUG-75: `StaleConfirm` 回収が「先頭VKは着弾していない」と無条件仮定して romaji 全体を再送し、着弾済みの子音が二重化する。journalで `verdict: StaleConfirm`、app_logに `backspace ×0 + re-send "tu"` を確認 | 未対応 | 修正コミット `45f833d3`（`fix(literal-detect): StaleConfirm回収で着弾済み先頭VKを再送しないようにする(BUG-75)`）はローカルの `fix/bug75-stale-confirm-duplicate-resend` ブランチにあるのみ。origin未push・PR未作成・develop未マージ |

## 既知バグとの対応関係（この台帳から見つかったもの）

- BUG-74 → report `01M0RE56S6EQ4MTQGJ2EB2W4N0`, `01M0RN7RR031SFSP8H0RCM9QAH`
- BUG-75 → report `01M0S4S6R4C1YJ581YJ9ZGAXXD`
