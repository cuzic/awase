---
id: ADR-198
title: |-
  永続化先の分類（config.toml / cache.toml / 学習表 JSON）と v2 での calibration の扱い
summary: |-
  v2 方針（2026-09-23 のユーザー決定、2026-09-24 に「変わらない」と再確認）のうち永続化先の分類だけを定める。
  ConfirmMode の2択化は確定エンジンの設定で永続化先とは無関係なので本 ADR に含めず、docs/tasks/review-2026-09-24-04 の
  推奨モード統一後に別 ADR（または既存 ADR への追記）で扱う。
status: |-
  草案（2026-09-24）。opus-adversarial-consult 未実施。
related_adr:
  - "ADR-176"
  - "ADR-191"
  - "ADR-195"
  - "ADR-058"
  - "ADR-125"
---

# 永続化先の分類（v2 方針、俯瞰レビュー A-10 / C-4 / C-5）

## 背景

v2 では calibration を config.toml から cache.toml へ移す、という方針がユーザーにより決定されていた。
しかしその出典はリポジトリ外のメモだけで、リポジトリの読み手には辿れなかった。
またメモの内容は現行の実装・ADR と2点で合わない（下記）。本 ADR は決定内容そのものを本文に書く。

## 決定

### 決定1: 永続化先は3分類とする

| 分類 | 置き場 | 性質 | 例 |
|---|---|---|---|
| ユーザー設定 | `config.toml` | 人間が読み書きする。消えると困る | `[keys]`、`app_overrides`、`confirm_mode` |
| 観測キャッシュ | `cache.toml` | 再学習・再観測で戻る。消えても実害は初回のやり直しだけ | `[imm_capability]`（ADR-125）、`[injection_mode]`（ADR-058） |
| 学習表 | 別 JSON（`<config dir>/keymap-learn-table.json`、`keymap-learn-last-attempt.json`） | 再生成コストが大きい（学習は約17〜20分、ADR-195 成功基準）。表全体を1ファイルで持つ | ADR-195 段階3 |

### 決定2: 学習表は cache.toml へ移さない（メモの `[keymap_learn]` 節案を取り下げる）

ADR-195 段階3 が「`[[calibration]]` とは粒度が違うので別ファイルにする」と決めており、実装もそのとおりである
（`crates/awase-keymap-learn-win/src/main.rs`、`awase::fs_atomic::write_atomic`）。
学習表は「消してよい観測キャッシュ」にも「無人で短時間に作り直せるデータ」にも当てはまらないので、
cache.toml に混ぜて消されうる場所に置く理由がない。

### 決定3: `[[calibration]]` の cache.toml への移設は行わない

手動較正の結果（`[[calibration]]`）は ADR-191 で適用側が撤去済み（`9dc52c89`）で、現在は読む側が本番コードに存在しない。
読まれないデータを移設しても意味がない。手動較正パネル自体の去就は
[docs/tasks/review-2026-09-24-07](../tasks/review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md) のタスク (2) で扱う（判断保留中）。
`AppConfig` は `deny_unknown_fields` を持たないので、フィールドを消しても既存の config.toml は読める。

### 決定4: `app_overrides` は config.toml に維持する

ユーザーが明示的に書く設定であり、決定1のユーザー設定に該当する。

### 決定5: cache.toml の書き込みはアトミックにする

`save_section` は `awase::fs_atomic::write_atomic` を使う（PR #302）。
読込失敗時の扱い（上書きしない／`.bak` 退避／警告のみ）は、v2 で cache.toml のセクションを増やす場合に決める（未決）。

## 範囲外

- ConfirmMode の2択化（確定エンジンの設定。永続化先の話ではない）。
- 手動較正パネルの撤去（07 タスク (2)、03・06 の判断待ち）。
