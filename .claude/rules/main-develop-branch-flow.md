# main / develop のブランチ運用

## ルール

- 新規の作業（fix / feat を問わない）は **必ず `develop` を経由する**。`develop`
  へ直接コミットするか、feature ブランチを切って `develop` にマージする。
- **`main` への直接コミットは禁止**。`main` を更新してよいのは
  `release-develop-to-main` スキル（`develop` → `main` マージ、
  CHANGELOG.md 更新、バージョン bump、タグ作成、GitHub Release 公開までを
  一貫して行う）による、意図的なリリース操作のときだけ。
- 作業用ブランチ（feature/fix/diag 等）は `develop` の先端から切る。`main` の
  先端から切らない。**例外は hotfix ブランチのみ**（下記「hotfix 例外」参照）。
- 「ちょっとした修正だから」「急ぎだから」を理由に `main` へ直接コミットしない。
  急ぎであっても `develop` にコミット/マージしてから `release-develop-to-main`
  を回す。急ぎの場合は hotfix ブランチ経由にする。

### hotfix 例外

`main`（= 既にリリース済みの最新版）で発生した重大な不具合を、次の
`release-develop-to-main` を待たずに即座に直したい場合に限り、`main` の先端から
`hotfix/*` ブランチを切ってよい。

1. `hotfix/<内容>` を `main` から切る。
2. 修正・検証後、`main` へ直接マージ（このケースのみ `main` への直接コミット/
   マージを許容する）。必要なら即リリース（タグ push、10節参照）。
3. **同じ内容を直後に `develop` へも反映する**（`hotfix/*` ブランチを `develop`
   にもマージするか、`main`→`develop` の同期マージを行う）。ここを省略すると
   次の `release-develop-to-main` 時に `develop` 側がこの修正を持たないまま
   `main` を上書きし、hotfix が消える事故になる。
4. `develop` への反映が完了するまで `hotfix/*` ブランチを削除しない。

## なぜこのルールが必要か（背景）

2026-08-01、`main` に対して直接複数のコミット（about ダイアログ・トレイ表示・
レイアウト反映・BUG-46 修正 等）が積まれる一方で、`develop` 側でも独立に
`awase-gji-config` crate の新設や ADR-081/082 文書のステータス同期が進んでおり、
さらに `diag/bug45-raw-tsf-literal`・`feat/settings-vk-combo-ui` という
トピックブランチが `main` の別の古い時点から切られていた。結果として
`main` と `develop` が二重に乖離し、どちらにも相手にしかないコミットがある
状態を手動で洗い出し、4回のマージ（`develop`→トピック2本→`main`→`develop`）
で強引に一本化する羽目になった。加えてトピックブランチのワークツリーが
`~` 直下や `~/rust-nicola-worktrees/` など不統一な場所に作られており、
後片付けにも追加の作業が発生した（このワークツリー整理は
[[feedback_worktree_per_session]] とは別軸の問題）。

`main` と `develop` のどちらにもコミットしてよいという状態は、書き込み先が
2箇所に分岐すること自体が両者の乖離を生む構造的な原因であり、乖離するたびに
今回のような手動リコンサイルが必要になる。書き込み先を `develop` の1箇所に
固定し、`main` への反映を `release-develop-to-main` スキルという単一の
意図的な操作に絞ることで、この種の乖離を構造的に防ぐ。

## 適用範囲

- ドキュメントのみの変更（例: `docs/known-bugs.md` の追記、ADR のステータス
  同期）であっても、このルールの対象とする。「コードじゃないから直接
  `main` でいい」という例外は設けない（今回の乖離も一部はドキュメントのみの
  差分だった）。
- `main` への直接操作の例外は上記「hotfix 例外」のみ。緊急の revert も
  `hotfix/*` ブランチ経由で行う（`main` へ直接 `git revert`/直接コミットしない）。
