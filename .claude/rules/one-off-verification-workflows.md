---
paths:
  - ".github/workflows/*.yml"
  - "tools/e2e/**"
  - "docs/known-bugs/**"
  - "docs/adr/**"
---

# 1 回限りの検証 workflow は、結果を書いたら撤去する

## ルール

1 回限りの検証・調査のために作った workflow・`ci/*` ブランチ・専用スクリプト(例: `bug103-verify.yml`、
`msi-upgrade-verify.yml`、`env-events-probe.yml`)は、**結果を docs/ADR/BUG に書いたら、同じ PR で撤去する**。

1. **撤去する**: 結果(run ID・数値・結論)を該当の `docs/known-bugs/BUG-NNN.md`・ADR・`tools/e2e/**/README.md` に
   書いたら、その PR の中で workflow ファイル・専用スクリプト(他から使われないもの)を削除する。常設の workflow
   (`config-verify`・`tray-cache-clear` など)が使う部品は残す。
2. **撤去コミットのハッシュを書く**: 該当の BUG/ADR の本文に、撤去したコミットのハッシュを書く
   (例: `撤去済み: git show <hash>^:.github/workflows/xxx.yml で戻せる`)。squash マージの場合は、マージ後のハッシュに
   差し替える(マージ前のハッシュは履歴に残らない)。
3. **再実行が要りそうなときも撤去してよい**: 再実行の手順(戻す元のコミット、置くブランチ、`gh workflow run` の引数)を
   docs に書き残せば、撤去してよい。手順が無いまま「あとで使うかも」で残さない。
4. **結論が出ていないものは残す**: BUG が「再現待ち」「未解決」のまま、あるいは ADR が今後の観測の置き場に
   指定している workflow は残す。残す理由は PR の本文に書く(残すのは例外)。
5. **`e2e-ime.yml` の構成**: `expect='observe'` の構成には `until='YYYY-MM-DD'` を付ける(既存の規約。
   `tools/e2e/ime_key_matrix/observe_audit.py`・`test_e2e_plan.py`)。期限までに pass・fail(再現固定)・削除の
   どれかに決める。1 回限りの調査用の構成は、結果を docs に書いたら期限を待たずに削除する。
   `observe_grandfathered.txt` は縮める方向にだけ動かす。
6. **他のエージェントが回している ref には触らない**: 撤去の対象は workflow ファイルとスクリプトで、`ci/*`
   ブランチの ref そのものは消さない(他の作業が使っているかもしれない。ブランチの掃除は持ち主が決める)。

## なぜこのルールが必要か(背景)

2026-10-22 の棚卸し(`docs/tasks/mech-ecrs-2026-10-21/simplify-crates.md` S2・S7・S8)で、結果が BUG/ADR に入った
あとも、1 回限りの workflow 約 20 本、過去の実測結果 474 ファイル(10.3 MB)、一時の mutants workflow が残っていた。
`git grep` や読み手の注意がこれらのノイズに取られ、どれが常設でどれが使い捨てかを毎回確かめる必要があった。
結果が docs に入っていれば、workflow はコミットから戻せる。残しておく価値は低く、読む費用は毎回かかる。

関連: [fix-requires-evidence](./fix-requires-evidence.md)(再現の記録は BUG に書く)、
[experiment-logging](./experiment-logging.md)(revert の失敗条件)。
