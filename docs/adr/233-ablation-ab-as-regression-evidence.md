---
id: ADR-233
title: |-
  修正を外したビルド(ablation)との比較を、実機 CI シナリオを回帰の証拠と数える条件にする
summary: |-
  実機 CI のシナリオ(`sc-*`・`wt-probe` 等)を「回帰テスト」と数えるのは、修正ありで PASS し、修正を外したビルド(nofix、`tools/e2e/ime_key_matrix/ablations/*.sh` の mutator で作る)で同じ判定が症状を検出したときに限る、という規約にする。
  背景の実測: (1) known-bugs 183 件のうち状態欄に「CI 検証済み」とあるのは 19 件で、nofix 対照で「修正の有無で結果が変わる」ことまで確かめた記録があるのは BUG-170(ADR-203 (d)、ablation a8)の 1 件だけ。(2) 記録の再生(ADR-163 `replay_record`・ADR-225)は「HEAD で症状が出るか」を判定できず(ADR-225 F1)、journal に触れる BUG 45 件で再生が退行を検知した記録は見つからなかった。(3) 既存の mutator 6 本のうち 2 本(a3・a7)は develop にもう当たらない(腐敗)。a7 は `e2e-ime.yml` の構成から今も呼ばれているが、誰も回さないので 15 日気づかれていない。番号 a8〜a10 は `origin/ci/e2e-ime` と develop・PR #534 で別の意味に使われている。(4) BUG-114 の nofix 比較(PR #534)は、1 回目の「修正あり 3/3 PASS・修正なし 3/3 FAIL」が終了処理の `WM_CLOSE` 後の補正によるものとレビューで分かり、閉じる前だけを数え直した 3 回目の run では 18 回すべて drift 補正 0 件(INVALID)。修正なし a9 で残った差は、mutator が書き換えた値そのもの(profile=ImmCross)だけだった。
  決定: 第 1 段階は撤去と名前の整理だけ(a3・a7 と `atok-resync-nofollow` 構成の撤去、mutator を BUG 番号で命名し直す、`fix-requires-evidence.md` の「将来、再生トレースへ置き換える予定」の文を本規約に置き換える)。判定基準は最初の run の前に書き、observed 件数を出し、mutator が書き換えた値そのものを判別の根拠にしない。腐敗は Linux で mutator が当たるかを数秒で確かめるジョブで検出する(第 2 段階)。nofix の実機 run は必須チェックにせず、リリース前の手順で回す案を推奨する(所有者に聞く)。
status: |-
  提案(起草中、Opus レビュー前)
related_adr:
  - "ADR-163"
  - "ADR-186"
  - "ADR-191"
  - "ADR-203"
  - "ADR-205"
  - "ADR-225"
  - "ADR-226"
  - "ADR-134"
  - "ADR-232"
---

# ADR-233: 修正を外したビルド(ablation)との比較を、実機 CI シナリオを回帰の証拠と数える条件にする

## 用語

- **ablation(撤去実験)/ mutator**: develop のコードを機械的に書き換える小さなスクリプト
  (`tools/e2e/ime_key_matrix/ablations/aN-*.sh`。Python の文字列置換で、置換元が 1 回だけ現れることを `assert` する)。
  `e2e-ime.yml` の `cfg(..., mutator=...)` で指定すると、その構成だけ書き換えたビルドで走る(`e2e-ime.yml:768-774`)。
- **nofix**: バグの修正だけを外したビルド。ADR-186 の撤去実験(「この機構は要るか」)とは目的が違い、
  「このシナリオは、修正が無ければ症状を出すか」を確かめるために使う。
- **observed 件数**: 判定が見ている事象(例: drift 補正の行)が、判定対象の区間に何件あったか。0 件の PASS は「起きなかった」ではなく「通っていない」([ADR-205](205-observe-external-ime-close-in-imm32-unavailable-windows.md):39 と同じ教訓)。

## 背景(実測)

### 1. 「CI 検証済み」の BUG のうち、nofix で差を確かめたのは 1 件

- known-bugs は 183 件。frontmatter の `fix_commits` が空でないのは 90 件。
  状態欄に「CI 検証済み」とあるのは 19 件(BUG-025・071・148・150・151・156・157・158・159・162・163・165・166・168・172・176・179・180・183)。
  ```sh
  ls docs/known-bugs/BUG-*.md | wc -l                                            # 183
  grep -l -m1 -E '^\*\*状態[^*]*\*\*.*CI ?検証済み' docs/known-bugs/BUG-*.md | wc -l   # 19
  grep -l -E 'nofix|ablation|撤去したビルド|修正を外' docs/known-bugs/BUG-*.md        # BUG-115・151・175
  ```
  `nofix|ablation|修正を外` を含む 3 件は、BUG-115 が単体テスト(「修正を外すと fail することを確認済み」`BUG-115.md:429`)、
  BUG-151 は測定ビルドが ablation ブランチだった訂正(`BUG-151.md:42`)、BUG-175 は手動の A/B(`BUG-175.md:17`、n は少数)で、
  実機 CI の nofix 対照ではない。
- 実機 CI の nofix 対照は develop に 1 構成だけある: `sc-reopen-tsf-gji-gap600-nofix`(`e2e-ime.yml:426-429`、mutator
  `a8-no-gji-reopen-sync.sh`、BUG-170 / [ADR-203](203-gji-fsm-follows-belief-open-transitions.md) (d)
  「修正前 FAIL・修正後 PASS の両方を実測してからマージ」)。
- CI 構成のコメントが参照する BUG は 17 件(BUG-020・071・103・112・113・147・149・150・152・156・163・170・171・172・176・184・185)。
  そのうち nofix 対照を持つのは BUG-170 の 1 件。
  ```sh
  grep -oE 'BUG-[0-9]+' .github/workflows/{e2e-ime,wt-probe,bug112-probe}.yml .github/workflows/*verify*.yml | cut -d: -f2 | sort -u
  grep -n 'mutator=' .github/workflows/e2e-ime.yml
  ```

### 2. nofix で見えた限界(BUG-170 と BUG-114 の 2 例)

- **BUG-170(a8)**: 修正を外しても、入力先のテキスト・cold 経路・固着は変わらず PASS した(run 36654801007、
  `e2e-ime.yml:427` のコメント)。差が出たのは journal 上の同期を見る `--require-sync` だけで、この判定は nofix の結果を見た後に足された。
  つまり「利用者に見える症状」の回帰網としては、このシナリオは修正の有無を区別していない。
- **BUG-114(PR #534、`ci/bug114-wt-startup`、未マージ)**: Windows Terminal で起動時スコープのまま drift 補正を起こし、
  修正あり(fix)と nofix-a9(起動時の profile を ImmCross に固定=ADR-134 D1c 以前)・nofix-a10(`AnyFreshEvidence` の除外を外す=Finding 5 以前)を各 3 回比べる。
  | run | 手順 | fix | nofix-a9 | nofix-a10 | 分かったこと |
  |---|---|---|---|---|---|
  | 37424011562(1 回目) | 打鍵+終了 | 3/3 PASS | 3/3 FAIL(`drift_correction_read` 57/50/54) | — | 補正 12 本すべてが終了処理の `WM_CLOSE` 後の ObserverPoll で起きていた(レビュー M1)。基準 5(再武装 0)は give-up から停止まで約 1.5 秒しかなく構造上 FAIL になり得なかった(M2) |
  | 37426810927(2 回目) | WT のペイン分割→閉じる | 9 ジョブ INVALID | | | キー操作なので `SkipTyping` で観測が止まり、閉じる前の補正 0 件 |
  | 37429061750(3 回目) | 補助窓への前面移動×2 | 3/3 INVALID | 3/3 FAIL(理由は profile=ImmCross だけ) | 3/3 INVALID | 18 回すべて「窓を閉じる前の drift 補正 0 件」。a9 の FAIL は mutator が書き換えた値(profile)を基準 1 が読んだだけで、症状(補正の連発)は出ていない |
  ```sh
  gh run view 37429061750 --log | grep '"type": "bug114_result"' | grep -oE '"verdict": "[A-Z]+"|"invalid": \[[^]]*\]'
  ```
  (2026-10-06 07:26 UTC 時点。PR #534 は修正対応中で、以後の push の結果はこの表に無い。)
- 2 例とも、nofix を回さなければ「修正あり PASS」だけで回帰網ができたと数えていた。逆に nofix を回したことで、
  (a) シナリオが症状に届いていない(observed 0)、(b) 判別しているのが症状でなく書き換えた値そのもの、の 2 つが表に出た。

### 3. 記録の再生は「HEAD で症状が出るか」に答えない

- [ADR-225](225-journal-replay-ci-closed-loop-integration.md) F1: `replay_record` はレコード 1 件ごとに決定を再計算して記録値と比べるだけ。
  SP0 で、直近 10 件の BUG のうち安全レーンの fixture があれば検知できたのは 0 件(同 status)。
- `docs/tasks/journal-replay-rebuild-study-2026-10-06/inventory.md:114-128`: journal に触れる BUG 45 件(journal かログに触れるのは 96 件)のうち、
  replay fixture が作られたのは 5 件(BUG-008・019・097・131・146)、**退行を検知した記録は見つからなかった**(痕跡が残らない可能性があるので需要の根拠にはしない、と同メモ自身が注記)。
- `.claude/rules/fix-requires-evidence.md:22-26` は、(b)(known-bugs への記録)を「将来、再生トレースの追加へ置き換える予定」と書いている。
  上の 2 点から、この予定の前提(再生で再発を捕まえる)は当面成り立たない。
- 単体テストの層では、修正を外して落ちることの確認は既にある(`cargo mutants --in-diff`、BUG-115 の手作業の例)。
  欠けているのは**実機 CI シナリオの層**だけ。

### 4. ablation 機構の現状と腐敗

| ファイル(develop) | 外すもの | 使う構成 | develop に当たるか |
|---|---|---|---|
| `a3-no-eisu-suppress.sh` | 無変換/変換単独タップの eisu reset 抑止(ADR-186 決定 2) | なし(孤立) | **当たらない**。対象の抑止自体が `3f9b313e`(2026-09-19、撤去実験 E3 で不要と確認)で撤去済み |
| `a4-no-eisu-reset.sh` | eisu reset の判定関数 2 つ | `a4-no-eisu-reset`(pass) | 当たる |
| `a5-no-refresh20.sh` | 物理 IME キー通過後の 20ms 再読み取り | `a5-no-refresh20`(fail) | 当たる |
| `a6-no-idle-check.sh` | idle-conv-check | `a6-no-idle-check`(pass) | 当たる |
| `a7-no-follow.sh` | follow(ADR-187)の通過マーク | `atok-resync-nofollow`(observe、`e2e-ime.yml:168`) | **当たらない**。`executor.rs` の置換元が `f1ef8d76`(2026-09-21)で変わった。構成は残ったまま |
| `a8-no-gji-reopen-sync.sh` | BUG-170 の修正(ADR-203 (i)(ii)) | `sc-reopen-tsf-gji-gap600-nofix`(fail) | 当たる |
| (PR #534)`a9-bug114-immcross-bootstrap.sh`・`a10-bug114-any-fresh-evidence.sh` | BUG-114 の修正 2 つ | `wt-probe.yml` の `bug114` ジョブ | 当たる |

再確認(develop のソースを scratch に展開して各 mutator を当てる。ビルドはしない):
```sh
git archive origin/develop src crates | tar -x -C "$T" && (cd "$T" && bash <mutator>)   # a3・a7 は AssertionError
git log origin/develop -S'ime.arm_mode_key_pass_mark(now);' --oneline -- crates/awase-windows/src/runtime/executor.rs   # f1ef8d76
```

- 腐敗の型は 3 つ見つかった: (i) 対象の修正自体が撤去された(a3、a1・a2 は `2f67c380` で同じ理由で削除済み)、
  (ii) 周辺のリファクタで置換元の文字列が変わった(a7)、(iii) 番号の衝突: `origin/ci/e2e-ime` の `46ae2175`(2026-09-25、develop 未マージ)は
  a8〜a10 を conv 軸の撤去実験(`a8-no-focus-probe-roman.sh` 等)に使っており、develop の a8(BUG-170)・PR #534 の a9/a10(BUG-114)と別物。
- 腐敗の検出は「走らせたとき」だけ: mutator の `assert` が落ちる、または差分 0 でワークフローが落ちる(`e2e-ime.yml:772-774`)。
  nofix 構成は `ci/*` ブランチへの push か dispatch でしか走らず(`e2e-ime.yml:37-50`)、PR ゲートの `e2e-ime-smoke.yml` は
  `atok-passthrough-cold,baseline` だけを回す(`e2e-ime-smoke.yml:58-62`)。a7 が 15 日気づかれていないのはこのため。
- 修正コミットの「外しやすさ」の目安(fix_commits のある 90 件、本番の `.rs` の hunk を develop に逆向きに当てる `patch -R --dry-run -F0`):
  全 hunk が当たる 5 件・半分以上 61 件・半分未満 12 件・0 件 7 件・本番コード無し 5 件。粗い目安で、hunk が当たることは
  「その修正だけを外したビルドが意味を持つ」ことを保証しない(未確認)。逆に、当たらない修正は mutator を新しく書く必要がある。

### 5. 運用コスト(実測)

- wt-probe `bug114`: 1 ジョブ約 3 分(デバッグビルド約 80 秒を含む、run 37426810927・37429061750)。fix/nofix-a9/nofix-a10 × 3 回 = 9 ジョブ、約 27 ランナー分、壁時計 6〜15 分。
- e2e-ime nofix: ビルドはキャッシュが当たれば約 20 秒、1 回の e2e ジョブ約 2 分(run 36655470405)。キャッシュの外れたビルドの時間は未測定。
- mutator の保守: 1 本 9〜18 行。上の (ii) の腐敗は置換元の 1 行の書き換えで直るが、直すには「その修正が今のコードのどこにあるか」を読み直す必要がある。

## 決定

### D1. 実機 CI シナリオを回帰テストと数える条件

`fix-requires-evidence.md` の (a)(回帰テスト)のうち、**実機 CI のシナリオ**(`e2e-ime.yml` の構成・`wt-probe.yml` のジョブなど、Windows ランナーで実 IME を使うもの)を
回帰テストとして数えるのは、次をすべて満たす場合に限る。単体テスト・golden・閉ループは対象外(従来どおり)。

1. **判定基準を最初の run の前に書く**(PR 本文かコミット本文)。observed として数える事象と、0 件なら INVALID にする条件を含める。
2. 修正ありのビルドで N/N PASS、nofix(その修正だけを外したビルド)で N/N「症状が出た」。N ≥ 3。INVALID は分母から外すが件数は報告する。
3. **判別の根拠が、mutator の書き換えた値そのものでない**こと。例: a9 は profile を ImmCross に固定するので「profile=ImmCross」で FAIL するのは当然で、症状(補正の連発・`SendInput` の件数・入力先の文字)が出たことにはならない。
4. 基準を run の後に変えたら、それ以前の run の結果は差の根拠に使わず、変えた理由を本文に残す(PR #534 の「基準 v1 → 3 回目の push の前に追加」の書き方を標準とする)。
   BUG-170 の `--require-sync` のように、nofix で差が出なかった後に足した判定は、「journal 上の同期を確かめた」とだけ書き、利用者に見える症状を確かめたとは書かない。
5. nofix で差が出なければ(nofix も PASS、または observed 0)、そのシナリオは回帰テストに数えず、known-bugs の (b) に「CI では確かめられなかった(理由)」を書く。

### D2. `fix-requires-evidence.md` の (b) の「将来」の文を置き換える

`fix-requires-evidence.md:22-26` の「将来的に ADR-159 の記録・再生基盤が育てば、(b) は『再生トレースの追加』へ置き換える予定」を削り、
D1 の要点(実機 CI シナリオを (a) と数える条件、5 行以内)と本 ADR へのリンクに置き換える。(b)(known-bugs への記録)自体は残す
(CI で確かめられる BUG は「CI 検証済み」19 件に限られ、残りの記録先が要るため)。

### D3. mutator の命名と置き場所

- 新しい nofix の mutator は BUG 番号で名付ける: `ablations/bug<NNN>-<何を外すか>.sh`(例: `bug170-no-gji-reopen-sync.sh`、`bug114-immcross-bootstrap.sh`)。連番(aN)はブランチ間で衝突する(背景 4 (iii))。
- ADR-186 型の「機構は要るか」の撤去実験は従来どおり aN でよいが、撤去済みの機構に依存するものは機構の撤去と同じコミットで消す(`2f67c380` の前例)。

### D4. 腐敗の検出(第 2 段階)

既存の `ci.yml` か `e2e-ime-smoke.yml` の Linux ジョブ(`invariants-unit` と同じ ubuntu ランナー)に、`ablations/*.sh` を 1 本ずつチェックアウト直後のツリーに当てて
「`assert` が通り、差分が出る」ことだけを確かめる手順を足す(ビルドはしない。数秒)。対象は `e2e-ime.yml`・`wt-probe.yml` の構成から参照される mutator だけにし、
参照されない mutator があれば落とす(a3 のような孤立を残さない)。

「当たる」ことは「nofix で症状が出る」ことを保証しない。後者は実機 run でしか分からないので、D5 の頻度で確かめる。

### D5. nofix の実機 run の頻度

PR の必須チェックにはしない(推奨、所有者に聞く 2)。nofix 構成の追加・変更を含む PR と、`release-develop-to-main` の前に 1 回回す。
定期実行にする場合、`schedule` トリガーは既定ブランチで動き、このリポジトリの既定ブランチは develop(`gh repo view --json defaultBranchRef -q .defaultBranchRef.name`)なので、追加の手順は要らない。

## 段階

### 第 1 段階(撤去と文言のみ、コード変更なし)

| 内容 | 撤去対象 | 検証(CI) | 取りやめ条件 |
|---|---|---|---|
| a3 を消す | `ablations/a3-no-eisu-suppress.sh`(孤立・対象撤去済み) | `grep -rn a3-no-eisu .github tools docs` が 0 件 | なし |
| a7 と構成を消す | `ablations/a7-no-follow.sh`、`e2e-ime.yml:168` の `atok-resync-nofollow`(expect=observe で合否が無い) | e2e-ime の plan ジョブが通る(構成一覧の生成) | 所有者が resync の nofollow 対照を残したいと言えば、消さずに置換元を直す |
| a8 を BUG 番号で改名 | `a8-no-gji-reopen-sync.sh` → `bug170-no-gji-reopen-sync.sh`(`e2e-ime.yml:428` の参照も) | `ci/e2e-scenarios` へ push して `sc-reopen-tsf-gji-gap600-nofix` が従来どおり expect=fail で OK | ビルドキャッシュのキー(`e2e-ime.yml:759`)が変わるので 1 回はキャッシュ外れ。それ以上の影響が出たら改名を戻す |
| `fix-requires-evidence.md` の文言 | 「将来、再生トレースへ置き換える予定」の文(D2) | docs のみ。`adr-evidence-consistency` が通る | なし |
| `teardown-verification-guide.md:104` | 「`a1`〜`a7`」の古い行(a1・a2 は削除済み) | docs のみ | なし |
| BUG-114 の仕上げ | PR #534 の a9 を判別に使わない(D1-3)。閉じる前の observed が 0 のままなら a9・a10 と `bug114` ジョブを入れず、BUG-114.md に「WT の CI では drift 補正に届かなかった(run 番号)」と書く | PR #534 の run の verdict と observed 件数 | 観測が届く手順が見つかれば D1 を満たす形で入れる(その場合 a9・a10 は `bug114-*.sh` に改名) |

行数: 削除が mutator 2 本(約 20 行)と構成 1 行、追加は `fix-requires-evidence.md` の約 5 行。

### 第 2 段階(腐敗の検出、D4)

- 追加: Linux ジョブの手順 1 つ(シェル 15 行前後の見込み、未実測)。撤去先: なし(新しい検査)。代わりに、第 1 段階で孤立 mutator を消し、今後の孤立を構造的に防ぐことで相殺する。
- 検証: わざと置換元を変えたブランチで、このジョブが落ちることを 1 回確かめる。
- 取りやめ条件: mutator が 3 本以下のまま増えない(人が目で見れば足りる)。

### 第 3 段階(対象を広げる、所有者の判断後)

- 候補: 状態欄「CI 検証済み」19 件のうち、CI 構成が既にあり修正の hunk が develop に半分以上当たるもの(BUG-156・163・172・176 など。個別の確認は未実施)。
- 1 件ごとに D1 の手順で nofix を作り、差が出なければ「CI では確かめられなかった」と known-bugs に書いて終える(mutator は残さない)。
- 取りやめ条件: 最初の 3 件で、nofix で症状まで差が出たものが 0 件なら、第 3 段階はやめる(BUG-170・114 と同じく「シナリオが症状に届いていない」が多数なら、nofix を増やすより先にシナリオの観測を直す問題で、本 ADR の範囲外)。

## 却下した案と理由

- **記録の再生(journal・`replay_record`)を回帰の証拠にする**: ADR-225 F1・SP0 0 件、背景 3。新しい再生基盤(B5)はエンジン側の不具合に限る別の検討(journal 検討メモ)で、本 ADR はそれに依存しない。
- **全 BUG に nofix を義務づける**: 「CI 検証済み」は 19/183 件で、残りは CI で再現する手段が無い。義務にすると (b) で済ませていた BUG の修正が止まる(complexity-budget.md の例外条項と同じ理由)。
- **mutator の代わりに `cfg` フラグや feature で修正を切り替えられるように本番コードを書く**: 本番に分岐が増え、修正 1 件ごとに残り続ける。mutator は本番コードを変えない。
- **nofix を PR の必須チェックにする**: 1 BUG あたり約 27 ランナー分(背景 5)。V1(#530)も当面必須にしない所有者の決定と同じ扱いにする。
- **宣言テーブルで「BUG → 構成 → mutator」を生成する**: ADR-218〜220 の却下と同じく、対象が 2 件の段階で表を作る価値が無い。

## リスクと限界

- **nofix で差が出ることは、そのシナリオが利用者の症状を捕まえる保証ではない**。BUG-170 は journal 上の同期でしか差が出なかった。D1-4 で書き分けるだけで、解決はしない。
- mutator は「修正前のコード」を再現しない。修正後に周辺が変わっているので、nofix は「今のコードから修正の要点だけを外したもの」。修正前の挙動との一致は未確認のまま使う。
- 修正の hunk が当たる割合(背景 4 の 5/61/12/7/5)は粗い目安。hunk が当たる BUG で実際に nofix が意味を持つかは 1 件も確かめていない。
- 実機 CI の揺らぎ(フォーカス喪失で INVALID など)で N ≥ 3 がそろわない構成がありうる。INVALID が続く構成は D1-5 で「確かめられなかった」に倒す。
- PR #534 の最終結果は、この ADR の起草時点で未確定。
- キャッシュの外れたビルドの所要時間は未確認。

## 所有者に聞くこと

1. **対象の範囲**
   - (A) BUG-170 と BUG-114 の 2 件だけ(第 1 段階のみ)。
   - (B) A に加え、「CI 検証済み」19 件から 3 件を試す(第 3 段階の入口、取りやめ条件つき)。
   - (C) 今後の再発ファミリーの fix で、実機 CI シナリオを足すものすべて(D1 を規約として適用)。
   - 推奨: **C を規約にし、既存分は A だけ**。理由: 既存 19 件をさかのぼるのは費用が読めない一方、新しくシナリオを足すときに nofix を 1 回回す費用は約 27 ランナー分で、BUG-114 のような「届いていないシナリオ」をマージ前に見つけられる。
2. **nofix の実機 run を CI 必須にするか**
   - (A) 必須にしない。nofix の追加・変更を含む PR と、リリース前に 1 回。
   - (B) 週 1 回の定期実行(`schedule`、既定ブランチ develop で動く)。
   - (C) PR の必須チェック。
   - 推奨: **A**(+第 2 段階の Linux での当たり確認は全 PR)。理由: 腐敗の主な型(背景 4 の (i)(ii))は数秒の当たり確認で捕まり、実機で症状が出なくなる型は今のところ実例が無い。
3. **a7(nofollow)を消してよいか**: 推奨は消す(expect=observe で合否に使っておらず、15 日壊れたまま気づかれていない)。

## 関連する既存文書への追記案(この ADR では書き換えない)

- `.claude/rules/fix-requires-evidence.md:22-26`: 「将来的に……置き換える予定(未実装……)。」を削り、次に置き換える。
  > 実機 CI のシナリオ(`e2e-ime.yml`・`wt-probe.yml` 等)を (a) と数えるのは、修正を外したビルド(`ablations/bug<NNN>-*.sh`)で同じ判定が症状を検出し、修正ありで PASS した場合に限る。判定基準は最初の run の前に書き、observed 件数を出し、mutator が書き換えた値そのものを判別の根拠にしない([ADR-233](../../docs/adr/233-ablation-ab-as-regression-evidence.md))。差が出なければ (b) に「CI では確かめられなかった」と書く。
- `docs/teardown-verification-guide.md:81-89`: `mutator` の説明に「nofix(BUG の修正を外す)は `bug<NNN>-*.sh`、機構の撤去実験は `aN-*.sh`」の 1 行。`:104` の「`a1`〜`a7`」を現状に直す。
- `docs/known-bugs/BUG-114.md`: PR #534 の結論(差が出たか、届かなかったか)を run 番号つきで 1〜3 行。
- `docs/known-bugs/BUG-170.md`: 「nofix(a8)で差が出たのは `--require-sync`(journal 上の同期)だけで、入力先のテキストでは差が出なかった(run 36654801007)」の 1 行。
