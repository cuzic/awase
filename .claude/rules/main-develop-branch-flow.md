# main / develop のブランチ運用

## ルール

- 新規の作業（fix / feat を問わない）は **必ず `develop` を経由する**。`develop`
  へ直接コミットするか、feature ブランチを切って `develop` にマージする。
- **`main` への直接コミットは禁止**。`main` を更新してよいのは
  `release-develop-to-main` スキル（`develop` → `main` マージ、
  CHANGELOG.md 更新、バージョン bump、タグ作成、GitHub Release 公開までを
  一貫して行う）による、意図的なリリース操作のときだけ。
- 作業用ブランチ（feature/fix/diag 等）は `develop` の先端から切る。`main` の
  先端から切らない。
- 「ちょっとした修正だから」「急ぎだから」を理由に `main` へ直接コミットしない。
  急ぎであっても `develop` にコミット/マージしてから `release-develop-to-main`
  を回す。

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
- 緊急の revert（`main` に出た重大な退行を即座に戻す等）は例外として `main`
  への直接操作を許容するが、その場合は直後に同じ revert を `develop` にも
  反映し、乖離を放置しない。
