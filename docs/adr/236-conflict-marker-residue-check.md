---
id: ADR-236
title: |-
  衝突マーカーの残骸を CI で検出する
summary: |-
  マージ解消で残った衝突マーカー(とくに diff3 の基底行 `||||||| <ラベル>`)が develop に 3 回届いた。うち 1 回は 284 コミット・6 日残って main(v2.0.0)にも入り、1 回は `.githooks/pre-push` を bash の構文エラーにした。どれも Rust のコンパイルに関係しない場所(Markdown・シェル)だったため CI は緑だった。CI の既存 `fmt` ジョブの checkout 直後に、`git grep` で行頭の `<<<<<<< `・`>>>>>>> `・`||||||| ` を探すステップを足す(追加 6 行程度、撤去対象なし)。`=======` は見ない。pre-push への追加はしない。
status: |-
  提案(起草中、Opus round1 反映済み: Must 2・Should 6・Nit 5)
related_adr:
  - "ADR-162"
  - "ADR-158"
  - "ADR-237"
---

# ADR-236: 衝突マーカーの残骸を CI で検出する

## 背景(実測)

### 実例: 履歴に入った衝突マーカーは 7 件、develop の先端に届いたのは 3 件

再確認のコマンド(下の表はこの出力から作った):

```sh
# マーカー行を足した/消したコミット(マージを含む)
git log origin/develop -m --format='%h %cd %s' --date=short -G'^(<<<<<<< |>>>>>>> |\|\|\|\|\|\|\| )'
# 全 ref(未マージのブランチを含む)の非マージコミット
git log --all --no-merges --format='%h %cd %s' --date=short -G'^(<<<<<<< |>>>>>>> |\|\|\|\|\|\|\| )'
# develop の first-parent の各先端に、マーカー行を含むファイルがあったか(2026-09-15 以降の 649 コミット)
for c in $(git rev-list --first-parent --since=2026-09-15 origin/develop); do
  git grep -lE '^(<<<<<<< |>>>>>>> |\|\|\|\|\|\|\| )' $c; done | sed 's/^[0-9a-f]*://' | sort | uniq -c
```

| # | 日付 | ファイル | 残った行 | 入れた → 消した | develop の先端に |
| --- | --- | --- | --- | --- | --- |
| 1 | 09-20 | `docs/adr/index.md` | 基底行のみ | `80bfb208`(マージ)→ `67d918b0`(1 分後、同じブランチ) | 届かず |
| 2 | 09-29 | `docs/bug-reports-triage.md` | 4 種そろって | `1096c37d`(develop への直接 push)→ `6bbd3d73` | 1 コミットだけ |
| 3 | 09-29 | `docs/known-bugs/BUG-172.md`(持ち込み時 `:50`、消す直前 `:54`) | 基底行のみ(`1a058350`) | `ab368de8`(PR #377 のブランチのマージ)→ `5de4b395`(10-05) | **284 コミット・6 日**。main(v2.0.0、`d28f58eb`)にも入り、main には今も残る |
| 4 | 09-30 | `docs/known-bugs/index.md` | 4 種そろって | `2260fcf5` → `6e7f64fa`(PR #390 のブランチ上) | 届かず |
| 5 | 09-30 | `docs/adr/index.md` | 基底行のみ(`c4d3dd72`) | `d13535ad` → `8f41919c`(PR #394 のブランチ上) | 届かず |
| 6 | 10-05 | `crates/awase-windows/tests/architecture_guard.rs`・`.cargo/mutants-awase-windows.toml` | 基底行のみ(`8c1f7c26`) | `05fd77ba`(PR #517〈F3〉のブランチのマージ)→ `dc7b9c35`(39 秒後) | 届かず |
| 7 | 10-06 | `.githooks/pre-push:37`・`.claude/rules/fix-requires-evidence.md:37` | 基底行のみ(`parent of 9c9a1164`) | `e133c730`(PR #535〈F5c〉の head、マージではないコミット)→ `1630e55e` | 約 11 分(16:30 → 16:41 JST) |

読み取れること:

- **7 件中 5 件は基底行 `||||||| <ラベル>` だけが残った**。このリポジトリは `merge.conflictStyle=diff3`(`git config --show-origin --get merge.conflictStyle` → `.git/config` に `diff3`)で、解消する人(多くはエージェント)が `<<<<<<<`・`=======`・`>>>>>>>` を消しても、基底行は見た目がマーカーらしくないため見落とす。
- **`=======` だけが残った例は 0 件**。`git log --all -m -G'^=======$'` が出すのは `1096c37d`・`2260fcf5`・`6bbd3d73`・`6e7f64fa` だけで、どれも 4 種そろって残った件(他の 3 種でも捕まる)に含まれる。
- 7 件のうち 4 件(#1・#4・#5・#6)はブランチ上で気づいて消し、3 件(#2・#3・#7)は develop の上で消した。ブランチ上の 4 件も人(エージェント)が偶然見つけたもので、仕組みで止めたものではない。

### なぜ CI を通ったか

- 先端に届いた 3 件(#2・#3・#7)は Markdown とシェルで、Rust のビルド・テストには関係しない。CI の 16 ジョブ(`.github/workflows/ci.yml`)にテキストとしてマーカーを探すものは無い。#7 の PR #535 は checks が SUCCESS 20・SKIPPED 2 のままマージされた。
- #6 の `architecture_guard.rs` は、push されていればコンパイルエラーで CI に捕まる(39 秒後に直されており、push されたかは未確認)。同じ件の `mutants-awase-windows.toml` は mutants ジョブ(`workflow_dispatch` のときだけ動く)でしか読まれない。

### 壊れた pre-push の実害は、このマシンでは 0 件だった

- `git show e87be9f7:.githooks/pre-push > pp.sh; bash pp.sh </dev/null` は `line 37: syntax error near unexpected token '||'` で exit 2 になる。develop の先端の pre-push は約 11 分、構文エラーだった。
- しかし `git config --show-origin --get core.hooksPath` は `.git/config` の `/home/cuzic/rust-nicola/.githooks`(**メインの作業ツリーを指す絶対パス**)で、65 個の worktree はどれもこれを共有する。push のときに動くのは push するブランチの版ではなく、メインの作業ツリーにその時点でチェックアウトされている版である。メインの作業ツリーの reflog(`e5567ddb` → `c8f841ef` → rebase で `1630e55e` 以降)に壊れた `e87be9f7` は一度も現れず、その間の版はどれも `bash -n` を通る。
- このため止まった push はこのマシンでは無い。止まりうるのは、壊れた develop を pull した別の clone(Windows 側のチェックアウトなど)だけで、その実例は確認できていない。
- **本当の害は旧い行の残留**: `1630e55e` はマーカー行と一緒に、`.githooks/pre-push` の旧い `local target=` 行を 1 行消している(`fix-requires-evidence.md` も同じ形)。diff3 の基底行の下には基底の内容(旧い行)が残るので、マーカーだけを消す「解消」は旧い内容を黙って残しうる。

### 既存の誤検出の実測

```sh
git grep -nE '^(<<<<<<< |>>>>>>> |\|\|\|\|\|\|\| )'     # develop(1630e55e)・v1-develop・v1-main で 0 件。main は #3 の 1 件
git grep -nE '^=======$'                                 # 0 件
git grep -nE '(<<<<<<<|>>>>>>>|\|\|\|\|\|\|\|)'          # 行の途中も含めて 0 件
git grep -nE '^={7,}'                                    # 2 件(scripts/run-e2e.ps1:95、tools/e2e/ime_key_matrix/device/x5-README.txt:2。どちらも 8 文字以上)
```

今の develop の全域では 4 種とも誤検出は 0 件。ただし `=======` ちょうど 7 文字は Markdown の見出しの下線(setext 形式)として
将来書かれうるうえ、上の通り `=======` だけが残った実例は無いので、**見る価値が無く誤検出の余地だけがある**。

## 決定

### 第 1 段階: CI の `fmt` ジョブに 1 ステップ足す(撤去対象なし、追加 6 行程度)

`.github/workflows/ci.yml` の `fmt` ジョブ(checkout と rustfmt だけの軽いジョブ。push と、develop・main・v1 系への
pull_request の両方で動き、`paths` の絞り込みは無い)の **checkout の直後**(`dtolnay/rust-toolchain` の前。マーカーが
あるときに toolchain を入れる時間を省く)に次を足す。新しいジョブは作らない。

```yaml
      - name: 衝突マーカーの残りを確認
        run: |
          if git grep -nE '^(<<<<<<< |>>>>>>> |\|\|\|\|\|\|\| )'; then
            echo "::error::衝突マーカーの行が残っています(上の ファイル:行 を直してください)"
            exit 1
          fi
```

- **正規表現**: 行頭の `<<<<<<< `・`>>>>>>> `・`||||||| `(どれも直後に空白。git は必ずラベルを付ける)。`=======` は見ない。
- **範囲**: 差分ではなく作業ツリー全域(`git grep` は追跡下のファイルだけを見る)。develop への直接 push には比較する
  base が無いので、全域を見る方が単純で漏れない。
- **経路ごとの効き方**: PR 経由の 2 件(#3・#7)は、マージ前の PR の CI が赤くなるので事前に止まる。直接 push の 1 件(#2)は、
  push した後に develop の CI が赤くなって事後に気づく。
- **誤検出の扱い**: 除外リストは作らない。docs でマーカーを説明したいときは行頭に置かない(インラインコードにする、
  コードブロック内で字下げする)。この ADR もそうしている。
- **失敗時の表示**: `git grep -n` の出力がそのまま `ファイル:行:内容` で出る(#7 なら
  `.githooks/pre-push:37:||||||| parent of 9c9a1164 ...`)。
- **`bash -n .githooks/pre-push` は足さない**: #7 の構文エラーの原因はマーカー行そのもので、上の grep が先に捕まえる。
  マーカーによらない pre-push の構文エラーは履歴上 0 件で、壊れた pre-push の実害もこのマシンでは 0 件だった。

### pre-push には足さない

- (a) `git push --no-verify` で飛ばされる(docs のみの作業ではこの運用が使われている)。
- (b) 動くのはメインの作業ツリーにチェックアウトされている版で、push するブランチの版ではない(背景参照)。
  ブランチ側で検査を足したり直したりしても、メインの作業ツリーが追いつくまで効かない。

### 既存の CI との関係

- `core-registry-consistency`(FCIS V1、#530)は develop への pull_request でだけ動き(`if: pull_request && base_ref == develop`)、
  `xtask-adr-evidence` を cargo でビルドする。マーカー検査は develop への直接 push でも動かしたいので、そこには載せない。
- `adr-evidence-consistency` も cargo のビルドが要る。`fmt` の方が速く、「書式の検査」という意味でも近い。
- `adr-index-consistency`(cargo 不要)も候補だが、ジョブ名が index の検査なので、無関係な検査を入れると読み手を迷わせる。
- main は #3 の 1 件が残っているので、新しいステップが入った後に main へ直接 push すると赤くなる。main は release
  (develop → main のマージ)でしか更新せず、そのマージで消えるので実害は無い。

## 却下した案

- **`git diff --check <base>...HEAD`**: git 2.39 で基底行も `leftover conflict marker` として出る(`git diff --check e87be9f7^1 e87be9f7`
  で 2 件とも出た)。しかし末尾空白・ファイル末尾の空行も同じ終了コードで落ち、直近 100 コミットの範囲で
  `docs/adr/208-...md:150`・`docs/known-bugs/BUG-184.md:43` の 2 件(`new blank line at EOF`)を拾う。
  加えて直接 push では base が決まらない。
- **pre-push に grep を足す**: 上の (a)(b) の理由で効きが薄い。
- **CI に `bash -n .githooks/pre-push` を足す**: 上の通り、実害の記録に結びつかない。
- **`merge.conflictStyle` を `diff3` から `merge` に戻す**: 基底行は無くなるが、解消の判断材料(共通祖先)を失う。
  設定は `.git/config`(各 clone ごと)にあり、worktree・CI・他の clone に強制できない。
- **Rust のテスト(`architecture_guard.rs` のようなソース走査)として書く**: 1 行の grep で足りるものに crate のビルドを
  挟む理由が無い。

## 段階

| 段階 | 内容 | 撤去対象 | 検証(CI で確認できる形) | 取りやめ条件 |
| --- | --- | --- | --- | --- |
| 1 | `fmt` ジョブにステップ追加 | なし(追加 6 行程度) | (a) 追加した PR で `fmt` が緑(develop の全域 0 件)。(b) 同じ PR に一時コミットで `docs/` の適当なファイルの行頭に `||||||| test` を足して `fmt` が赤くなり、`ファイル:行` が表示されることを見てから、そのコミットを落とす。(c) 過去の実例: 上の決定と同じ正規表現で `git grep -nE <正規表現> fe4b5573`(および `e87be9f7`・`1096c37d`)を実行すると、それぞれ #3(`BUG-172.md:50`)・#7・#2 の行が出る(起草時に確認済み) | 誤検出が出て、行頭から外す書き換えで避けられないもの(例: マーカーを含むテキストそのものを fixture として持つテストが要る)が現れたら、除外リストは作らずステップを外す |

第 2 段階は置かない。

## リスクと限界

- **grep が捕まえるのは目印だけ**。マーカーを消しても基底の旧い行が残る(#7 の `local target=`)ことは CI では検出できない。
  対策は解消の手順側(ADR-237)の担当とする。
- **直接 push の経路は事後にしか気づけない**(#2 の型)。
- **ラベルの無いマーカー**(`<<<<<<<` だけの行)は捕まえない。git が出すマーカーは必ずラベル付きなので、手で書かない限り起きない。
- **`=======` だけが残る**ケースは捕まえない(実例 0 件)。
- `git grep` がエラー(exit 2)で終わったときも `if` は偽になりステップは緑で通る。checkout の後なので起きにくく、
  行数を増やしてまで区別しない。
- `.githooks/` やその他のシェルスクリプトの構文エラーは見ない。マーカーが入れば grep が捕まえる。
- 必須チェックは確認済みで、develop には無い(`gh api repos/cuzic/awase/branches/develop/protection` → `Branch not protected`、
  `gh api repos/cuzic/awase/rulesets` → `[]`、`gh api repos/cuzic/awase/rules/branches/develop` → `[]`)。

## 所有者に聞くこと

1. **`fmt` ジョブを必須チェックにするか**(リポジトリ設定は変えていない)。
   - 推奨: develop に保護が無いので、今回は変えない。理由は 2 つ。(i) PR 経由の 2 件は必須でなくても PR の画面で赤が見えて
     止まる。(ii) develop への push の CI は直近 80 回で失敗 5 回(どれも `report-worker`)、どれも 1 時間ほどで緑に戻っており、
     赤が放置される運用ではない。V1 を必須にしない決定とも揃う。
2. 検査を載せるジョブは `fmt` でよいか(代わりは `adr-index-consistency`。cargo 不要でより速いが、ジョブ名と中身がずれる)。

## 関連する既存文書への追記案

- `docs/adr/index.md`: 1 行追加(team-lead が統合する)。
- ADR-237(`docs/adr/237-serialized-develop-merge.md` の草案、113 行目)の手元の確認手順の正規表現
  (`=======` とラベル無しも見る)を、この ADR の正規表現に揃える(team-lead が統合するときに)。
  手元と CI で結果が食い違わないようにする。
- `.claude/rules/main-develop-branch-flow.md` などには追記しない(CI が落ちれば分かるため、規約文は要らない)。
