---
id: ADR-236
title: |-
  衝突マーカーの残骸を CI で検出する
summary: |-
  マージ解消で残った衝突マーカー(とくに diff3 の基底行 `||||||| <ラベル>`)が develop に 3 回届き、うち 1 回は 284 コミット・6 日残り、1 回は `.githooks/pre-push` を bash の構文エラーにした。どれも Rust のコンパイルに関係しない場所(Markdown・シェル)だったため CI は緑だった。CI の既存 `fmt` ジョブに、`git grep` で行頭の `<<<<<<< `・`>>>>>>> `・`||||||| ` を探すステップと `bash -n .githooks/pre-push` を足す(追加 7 行程度、撤去対象なし)。`=======` は見ない。pre-push への追加はしない。
status: |-
  提案(起草中、Opus レビュー前)
related_adr:
  - "ADR-162"
  - "ADR-158"
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
| 2 | 09-29 | `docs/bug-reports-triage.md` | 4 種そろって | `1096c37d` → `6bbd3d73` | 1 コミットだけ |
| 3 | 09-29 | `docs/known-bugs/BUG-172.md:54` | 基底行のみ(`1a058350`) | `ab368de8`(PR #377 のブランチのマージ)→ `5de4b395`(10-05) | **284 コミット・6 日** |
| 4 | 09-30 | `docs/known-bugs/index.md` | 4 種そろって | `2260fcf5` → `6e7f64fa`(PR #390 のブランチ上) | 届かず |
| 5 | 09-30 | `docs/adr/index.md` | 基底行のみ(`c4d3dd72`) | `d13535ad` → `8f41919c`(PR #394 のブランチ上) | 届かず |
| 6 | 10-05 | `crates/awase-windows/tests/architecture_guard.rs`・`.cargo/mutants-awase-windows.toml` | 基底行のみ(`8c1f7c26`) | PR #517(F3)のブランチ → `dc7b9c35` | 届かず(impl-f3 の自己申告どおり) |
| 7 | 10-06 | `.githooks/pre-push:37`・`.claude/rules/fix-requires-evidence.md:37` | 基底行のみ(`parent of 9c9a1164`) | `e133c730`(PR #535、F5c)→ `1630e55e` | 約 11 分(16:30 → 16:41 JST) |

読み取れること:

- **7 件中 5 件は基底行 `||||||| <ラベル>` だけが残った**。このリポジトリは `merge.conflictStyle=diff3`(`git config --show-origin --get merge.conflictStyle` → `.git/config` に `diff3`)で、解消する人(多くはエージェント)が `<<<<<<<`・`=======`・`>>>>>>>` を消しても、基底行は見た目がマーカーらしくないため見落とす。
- **`=======` だけが残った例は 0 件**(消したコミットの `-=======` 行: `67d918b0`・`8f41919c`・`5de4b395`・`1630e55e` のどれも 0)。4 種そろって残った 2 件は他の 3 種でも捕まる。
- 6 件はブランチ上で気づいて消しているが、それは人(エージェント)が偶然見つけたからで、仕組みで止めたものではない。

### なぜ CI を通ったか

- 3 件(#3・#7・#2)は Markdown とシェルで、Rust のビルド・テストには関係しない。CI の 16 ジョブ(`.github/workflows/ci.yml`)にテキストとしてマーカーを探すものは無い。
- #6 の `architecture_guard.rs` はコンパイルエラーになるので CI で捕まる(ただし CI を 1 周無駄にする)。同じ件の `mutants-awase-windows.toml` は mutants ジョブ(`workflow_dispatch` のときだけ動く)でしか読まれない。

### pre-push は「構文エラーのまま動いていた」のではなく、全 push を拒否していた

`git show e87be9f7:.githooks/pre-push > pp.sh; bash pp.sh </dev/null` を実行すると
`line 37: syntax error near unexpected token '||'` で **exit 2** になる(ファイル冒頭の `set -euo pipefail`
とは関係なく、bash は関数定義の読み込み中の構文エラーでスクリプトを終える)。git は pre-push が 0 以外で終わると
push を中止するので、この 11 分の間は `--no-verify` を付けない push はすべて止まっていた。気づかれなかった理由は、
docs のみの作業では `git push --no-verify` を使う運用(ADR 起草の共通指示など)になっているからと考えられる(未確認。
11 分の間に `--no-verify` なしの push を試みた人がいたかは分からない)。

この事実から、**pre-push に検査を足しても効きが薄い**: (a) `--no-verify` で飛ばされる、(b) 今回のようにフック自身が
壊れると、フックの中に書いた検査も一緒に動かない。

### 既存の誤検出の実測

```sh
git grep -nE '^(<<<<<<< |>>>>>>> |\|\|\|\|\|\|\| )'     # 0 件(1630e55e 時点の develop)
git grep -nE '^=======$'                                 # 0 件
git grep -nE '(<<<<<<<|>>>>>>>|\|\|\|\|\|\|\|)'          # 行の途中も含めて 0 件
git grep -nE '^={7,}'                                    # 2 件(scripts/run-e2e.ps1:95、tools/e2e/ime_key_matrix/device/x5-README.txt:2。どちらも 8 文字以上)
```

今の全域では 4 種とも誤検出は 0 件。ただし `=======` ちょうど 7 文字は Markdown の見出しの下線(setext 形式)として
将来書かれうるうえ、上の通り `=======` だけが残った実例は無いので、**見る価値が無く誤検出の余地だけがある**。

## 決定

### 第 1 段階: CI の `fmt` ジョブに 1 ステップ足す(撤去対象なし、追加 7 行程度)

`.github/workflows/ci.yml` の `fmt` ジョブ(checkout と rustfmt だけの軽いジョブ。push と、develop・main・v1 系への
pull_request の両方で動く)の `cargo fmt` の前に次を足す。新しいジョブは作らない。

```yaml
      - name: 衝突マーカーの残りと pre-push の構文を確認
        run: |
          if git grep -nE '^(<<<<<<< |>>>>>>> |\|\|\|\|\|\|\| )'; then
            echo "::error::衝突マーカーの行が残っています(上の ファイル:行 を直してください)"
            exit 1
          fi
          bash -n .githooks/pre-push
```

- **正規表現**: 行頭の `<<<<<<< `・`>>>>>>> `・`||||||| `(どれも直後に空白。git は必ずラベルを付ける)。`=======` は見ない。
- **範囲**: 差分ではなく作業ツリー全域(`git grep` は追跡下のファイルだけを見る)。develop への直接 push には比較する
  base が無いので、全域を見る方が単純で漏れない。
- **誤検出の扱い**: 除外リストは作らない。docs でマーカーを説明したいときは行頭に置かない(インラインコードにする、
  コードブロック内で字下げする)。この ADR もそうしている。
- **失敗時の表示**: `git grep -n` の出力がそのまま `ファイル:行:内容` で出る(#7 なら
  `.githooks/pre-push:37:||||||| parent of 9c9a1164 ...`)。
- **`bash -n .githooks/pre-push`**: フック自身の構文エラー(#7 の実害)を CI で捕まえる。フックの中には書けない検査なので
  CI に置く。

### pre-push には足さない

背景の通り `--no-verify` とフック自身の破損の両方で効かない。足すと pre-push の行数だけが増える。

### 既存の CI との関係

- `core-registry-consistency`(FCIS V1、#530)は develop への pull_request でだけ動き、`xtask-adr-evidence` を
  cargo でビルドする。マーカー検査は develop への直接 push でも動かしたいので、そこには載せない。
- `adr-evidence-consistency` も cargo のビルドが要る。`fmt` の方が速く、「書式の検査」という意味でも近い。
- `adr-index-consistency`(cargo 不要)も候補だが、ジョブ名が index の検査なので、無関係な検査を入れると読み手を迷わせる。

## 却下した案

- **`git diff --check <base>...HEAD`**: git 2.39 で基底行も `leftover conflict marker` として出る(`git diff --check e87be9f7^1 e87be9f7`
  で 2 件とも出た)。しかし末尾空白・ファイル末尾の空行も同じ終了コードで落ち、直近 100 コミットの範囲で
  `docs/adr/208-...md:150`・`docs/known-bugs/BUG-184.md:43` の 2 件(`new blank line at EOF`)を拾う。
  加えて直接 push では base が決まらない。
- **pre-push に grep と `bash -n` を足す**: 上の理由で効きが薄い。
- **`merge.conflictStyle` を `diff3` から `merge` に戻す**: 基底行は無くなるが、解消の判断材料(共通祖先)を失う。
  設定は `.git/config`(各 clone ごと)にあり、worktree・CI・他の clone に強制できない。
- **Rust のテスト(`architecture_guard.rs` のようなソース走査)として書く**: 1 行の grep で足りるものに crate のビルドを
  挟む理由が無い。

## 段階

| 段階 | 内容 | 撤去対象 | 検証(CI で確認できる形) | 取りやめ条件 |
| --- | --- | --- | --- | --- |
| 1 | `fmt` ジョブにステップ追加 | なし(追加 7 行程度) | (a) 追加した PR で `fmt` が緑(今の全域 0 件)。(b) 同じ PR に一時コミットで `docs/` の適当なファイルの行頭に `||||||| test` を足して `fmt` が赤くなり、`ファイル:行` が表示されることを見てから、そのコミットを落とす。(c) 過去の実例: 上の決定と同じ正規表現で `git grep -nE <正規表現> fe4b5573`(および `e87be9f7`・`1096c37d`)を実行すると、それぞれ #3・#7・#2 の行が出る(起草時に確認済み) | 誤検出が出て、行頭から外す書き換えで避けられないもの(例: マーカーを含むテキストそのものを fixture として持つテストが要る)が現れたら、除外リストは作らずステップを外す |

第 2 段階は置かない。

## リスクと限界

- **ラベルの無いマーカー**(`<<<<<<<` だけの行)は捕まえない。git が出すマーカーは必ずラベル付きなので、手で書かない限り
  起きない。
- **`=======` だけが残る**ケースは捕まえない(実例 0 件)。
- `.githooks/` 以外のシェルスクリプト(`tools/e2e/ime_key_matrix/*.sh` など)の構文エラーは見ない。マーカーが入れば grep が捕まえる。
- 必須チェック(branch protection)の現状は未確認: `gh api repos/cuzic/rust-nicola/branches/develop/protection` と
  `.../rulesets` はどちらも 404(保護が無いのか、権限が無くて見えないのかは区別できない)。
- 11 分の間に pre-push で止められた push があったかは未確認。

## 所有者に聞くこと

1. **`fmt` ジョブを必須チェックにするか**(リポジトリ設定は変えていない)。
   - 推奨: 既に必須なら何もしない(このステップも自動的に必須になる)。必須チェックが無いなら、今回はそのまま(赤くなれば
     PR の画面で見える。#3 のように 6 日残る事態は、マージ後の develop の push でも赤くなるので気づける)。
2. 検査を載せるジョブは `fmt` でよいか(代わりは `adr-index-consistency`。cargo 不要でより速いが、ジョブ名と中身がずれる)。

## 関連する既存文書への追記案

- `docs/adr/index.md`: 1 行追加(team-lead が統合する)。
- `.claude/rules/main-develop-branch-flow.md` などには追記しない(CI が落ちれば分かるため、規約文は要らない)。
