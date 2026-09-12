# ADR / known-bugs の frontmatter とファイル分割規約

## ルール

`docs/adr/` と `docs/known-bugs/` は、量が多く1ファイルが巨大化しやすいため、
「索引だけ読めば概要が分かり、必要なときだけ本文を開く」形に構成している。
新規に書く・機械的に処理する際は以下を守ること。

### `docs/known-bugs/`

- 新しい不具合は `docs/known-bugs/BUG-NNN.md`（次の連番、3桁ゼロ埋め）を**1件1ファイル**
  で新規作成する。旧 `docs/known-bugs.md` は索引へのリダイレクトスタブのみで、そこに
  追記しない。
- 各ファイル先頭に以下の frontmatter を付ける:
  ```yaml
  ---
  id: BUG-NNN
  title: |-
    <元の見出し全文>
  fix_commits: ["<hash>", ...]
  related_adr: ["ADR-NNN", ...]
  ---
  ```
- 索引 [docs/known-bugs/index.md](../../docs/known-bugs/index.md) の「概要」列は
  タイトルの機械的な先頭切り出し。新規行を足すときも同様に短く保つ（意味的な要約を
  自分で書き直す必要はない、機械的な切り出しで十分）。
- 本文の長さの目安（1ファイルあたり目安30行以内）は
  [fix-requires-evidence.md](./fix-requires-evidence.md) 参照。

### `docs/adr/`

- 各ADRファイル（`docs/adr/NNN-slug.md`）先頭に以下の frontmatter を付ける:
  ```yaml
  ---
  id: ADR-NNN
  title: |-
    <H1から抽出した短いタイトル>
  summary: |-        # 旧index.mdの記述がH1より大幅に長かった場合のみ（全文をここに保持）
    <...>
  status: |-          # index.mdのステータス列の全文（indexは短縮表示のみ）
    <...>
  related_adr:
    - "ADR-MMM"
  ---
  ```
- [docs/adr/index.md](../../docs/adr/index.md) の「タイトル」「ステータス」列は
  frontmatterと同じ内容を短縮表示したもの。**全文が欲しい場合は index.md を編集せず、
  対象ADRファイルの frontmatter か本文を開く**こと。index.md 側を長文化して差し戻す
  ような編集はしないこと（index.md を再び1000行超級に肥大化させる、この規約が
  修正しようとした問題の再発になる）。
- 新規ADRを起票したら、index.md にも短い1行（タイトル短縮版・ステータス短縮版）を
  追加する。詳細な文脈は ADR ファイル自身の frontmatter `summary` に書く。
- `docs/adr/` 直下には番号付きADR本体ではない補助資料（実装タスクリスト・
  インベントリ・`ADR-001-architecture-history.md` 等）もある。これらは
  `type: companion-doc` / `type: history-doc` の frontmatter を持ち、index.md の
  「補助資料」節に一覧がある。

## なぜこのルールが必要か（背景）

2026-09-11時点で `docs/known-bugs.md` が17,054行・123件、`docs/adr/index.md` が
「タイトル」列に数百〜5,700文字の長大な段落を含む行を多数抱え、それぞれ371行・
135,116バイトまで肥大化していた。個別の不具合や決定を1件確認するためだけに、
無関係な122件・166件ぶんの内容を読み込む（またはgrepで大量にヒットする）
コストが実効的な参照の妨げになっていた。

1ファイル1トピックに分割し、frontmatterに完全な情報を retain したまま index を
機械的に短縮することで、「まず索引を読む→必要な1件だけ本文を開く」という読み方が
可能になった（known-bugsは123ファイルに分割・indexは17,054行→141行、ADR indexは
135,116バイト→48,251バイトに縮小、情報は全てfrontmatterに移設し欠落なし）。

この効果を維持するには、今後の追記が同じ流儀（1ファイル1トピック、frontmatter
に全文、indexは短縮のみ）を守る必要がある。index.md に長文を書き戻す、または
`docs/known-bugs.md` に直接追記する形に戻ると、この整理は数ヶ月で無意味になる。

## 適用範囲

- `docs/known-bugs/` 配下の新規ファイル追加、`docs/adr/` 配下の新規ADR起票・
  frontmatter編集に適用する。
- 個々のADR/BUGファイル本文そのものの書き方（症状・原因・修正履歴の記述粒度）は
  [fix-requires-evidence.md](./fix-requires-evidence.md)・
  [tuning-constants.md](./tuning-constants.md)・
  [experiment-logging.md](./experiment-logging.md) の既存規約に従う。本ルールは
  ファイル配置とfrontmatterの形式のみを規定する。

関連: [fix-requires-evidence](./fix-requires-evidence.md)。
