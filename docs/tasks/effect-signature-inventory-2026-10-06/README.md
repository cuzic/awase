---
title: Effect(命令)の署名の棚卸し E0 — 集計
status: 棚卸し完了(2026-10-06、読み取りのみ、origin/develop 2c9c3eb8 時点)。F の分割の根拠表
created: 2026-10-06
related_adr: ["ADR-229", "ADR-208", "ADR-212", "ADR-156", "ADR-180"]
---

# Effect(命令)の署名の棚卸し E0 — 集計

ADR-229「世界モデルの reduce 化と Effect 列の設計の検討結果」の段階 E0。全文は同ディレクトリの `inventory-e0.md`(各 variant の生成元・実行する所・OS への副作用・結果・順序の制約・冪等性・可換性・吸収の可否・journal での記録)。

## 実測

| 項目 | 結果 |
|---|---|
| 調べた enum | 54 型・variant 244。命令は約 65 variant。ほかに `WM_*` が 13 定数、論理タイマー ID が 12 |
| journal | **Effect 列は journal に載らない**。`KeyInput` に入るのは `effect_count` だけで、実際に出た送信は `SentInput` にだけ残る |
| `Set; Set = Set`(タイマーの上書き) | 論理上は成立するが、**観測込みでは不成立**。`Win32Timer::set` が毎回新しい OS ID を振り、`OUTPUT_GATE` 中に退避した発火を `(logical_id, os_id)` で照合して捨てるため |
| 書き込みの吸収・省略が実害になった例 | 7 件(§7)。事故は `shadow`/`applied` を根拠にした省略に集中(BUG-141、ADR-208 S-1、BUG-113 追補、PR #419 M-1)。**新鮮な読み取り・世代・ID を根拠にした省略は安全だった** |
| 順序の制約がコードの呼び出し順だけで保たれている箇所 | 22 件(§11) |

## F の分割で壊しやすい暗黙の順序(F の担当は必ず読む)

- `execute_relay`(`runtime/executor.rs:430-445`): Consume のとき **Timer だけが即時に実行され、キューを追い越す**。F3(`execute_relay`/`drain_deferred` の計画/ガード状態機械)は、この順序を保つ。
- shadow toggle が `engine.on_input` の**前**に走る(`runtime/key_pipeline.rs`)。
- 詳細は §11。

## 注意

- `GjiReinitRetryCompleted` は本番の生成元が見当たらず、**死んだ variant の疑い**(未確認。W-b の「死んだフィールド」が誤検出だった前例があるので、削除の前に grep で裏取りする)。
- 未確認の点は §12。
