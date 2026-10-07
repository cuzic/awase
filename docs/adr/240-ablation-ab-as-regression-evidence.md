---
id: ADR-240
title: |-
  実機 CI シナリオの判定の書き方(observed 件数・前提と症状・INVALID 優先)と、修正を外したビルド(ablation)の整理
summary: |-
  実機 CI のシナリオ(`e2e-ime.yml` の構成・`wt-probe.yml` のジョブ)の判定の書き方と、修正を外したビルド(nofix。`ablations/*.sh` の mutator、または既にある設定での無効化)の扱いを決める。
  決定の中心は撤去: develop にもう当たらない mutator 2 本(a3・a7)と、その参照(構成・grandfathered・ランナー・説明)を消し、a8 を BUG 番号の名前に変える。
  判定の書き方(D1a): 判定基準を「前提」と「症状」に分け、判定対象の区間と症状として数えるフィールドを最初の run の前に書く。observed 件数を出す。observed 0 の回は FAIL より INVALID を優先する。
  nofix との比較(D1b)は任意。回すときは、症状の基準で落ちた回だけを「修正の有無を区別した」と数える。
  背景の実測(develop `8810b812`、2026-10-06): 状態欄に「CI 検証済み」とある BUG は 19 件で、nofix で差を確かめた記録は 0 件。nofix を回した 3 例(BUG-170 の a8、BUG-114 の WT 版 PR #534、BUG-114 の実 Chrome 版 PR #529)はどれも回帰を捕まえていない。nofix が分かったのは BUG-170 の 1 件だけで、入力先の文字の判定が修正の有無を区別しないことだった。BUG-114 では誤った差を 2 回出した(1 回目は `WM_CLOSE` 後の補正、3 回目は a9 が書き換えた profile)。「シナリオが症状に届いていない」を示したのは、修正ありのビルドの INVALID 判定(observed 0)。nofix が無いせいで退行を見逃した記録も 0 件なので、nofix は義務にしない。
  他の ADR との分担(team-lead の決定、ADR-241 決定 7): 同期の判断(打鍵から actuation の決定まで)は ADR-241 の再生が受け、再生が通らない非同期・実機固有の部分を実機 CI の再現シナリオ+D1a が受ける。`fix-requires-evidence.md:22-26` の書き換えは ADR-241 だけが行い、本 ADR はそこに入れる 1 項目の文案を追記案として出す。ADR-241 の mutator(再生の合否用)は再生側、本 ADR の ablation は実機 CI 側の資産として、両方残す。
status: |-
  提案(起草中。Opus round1〈旧 233 として、Blocker 1・Must 5・Should 6・Nit 4〉と 2 人目のレビュー〈Blocker 1・Must 6・Should 7・Nit 6〉を反映、round2 前)
related_adr:
  - "ADR-134"
  - "ADR-163"
  - "ADR-186"
  - "ADR-187"
  - "ADR-191"
  - "ADR-203"
  - "ADR-205"
  - "ADR-209"
  - "ADR-225"
  - "ADR-234"
  - "ADR-235"
  - "ADR-241"
---

# ADR-240: 実機 CI シナリオの判定の書き方と、修正を外したビルド(ablation)の整理

番号の注記: 起草時は ADR-233 だったが、develop の `233-stale-high-observation-beats-newer-in-most-recent-trusted.md` と衝突したため 240 に改番した(ブランチ名 `docs/adr-233-ablation-ab` は 233 のまま)。
件数・行番号は、断りのない限り `origin/develop` の `8810b812`(2026-10-06)時点。行番号は動くので、再確認のコマンドを添える。

## 用語

- **ablation(撤去実験)/ mutator**: develop のコードを機械的に書き換える小さなスクリプト。
  `tools/e2e/ime_key_matrix/ablations/aN-*.sh` にあり、Python の文字列置換で、置換元が 1 回だけ現れることを `assert` する。
  `e2e-ime.yml` の `cfg(..., mutator=...)` で指定すると、その構成だけ書き換えたビルドで走る(`grep -n '撤去(コード変更)を当てる' -A7 .github/workflows/e2e-ime.yml`)。
- **nofix**: バグの修正だけを外したもの。外し方は 2 つある。mutator で書き換えたビルドか、**既にある設定**での無効化
  (例: `sc-adr209-chrome-msime-off`。`general='predict_henkan_open_in_unreadable_windows = false'`、`e2e-ime.yml:343-347`)。
  ADR-186・191 の撤去実験(「この機構は要るか」)とは目的が違う。nofix は「このシナリオは、修正が無ければ症状を出すか」を確かめる。
- **前提(precondition)と症状(symptom)**: 判定基準のうち、測定が成り立ったことを確かめる基準が前提
  (起動時スコープが期待どおり、プロセスの切り替えが 1 回、など)。
  利用者に見える結果、またはその直接の原因になる送信を見る基準が症状(drift 補正の連発・`SendInput` の件数・入力先の文字)。
- **observed 件数**: 症状の基準が見る事象が、判定対象の区間に何件あったか。0 件の PASS は「起きなかった」ではなく「通っていない」
  ([ADR-205](205-observe-external-ime-close-in-imm32-unavailable-windows.md):39 と同じ教訓)。

## 背景(実測)

### 1. 「CI 検証済み」の BUG で nofix の差を確かめた記録は 0 件

- known-bugs は 186 件(起草を始めた 2026-10-06 朝は 183 件)。frontmatter の `fix_commits` が空でないのは 92 件。
  状態欄に「CI 検証済み」とあるのは 19 件
  (BUG-025・071・148・150・151・156・157・158・159・162・163・165・166・168・172・176・179・180・183)。
  この 19 件の本文に、nofix で差を確かめた記録は無い。
  ```sh
  ls docs/known-bugs/BUG-*.md | wc -l                                                # 186
  grep -L -E '^fix_commits:\s*\[\]' docs/known-bugs/BUG-*.md | wc -l                   # 92
  grep -l -m1 -E '^\*\*状態[^*]*\*\*.*CI ?検証済み' docs/known-bugs/BUG-*.md | wc -l      # 19
  grep -l -E 'nofix|ablation|撤去したビルド|修正を外' docs/known-bugs/BUG-*.md            # BUG-115・151・175
  ```
  語が当たる 3 件は、どれも実機 CI の nofix ではない。
  - BUG-115: 単体テスト(`BUG-115.md:429`)。
  - BUG-151: 測定ビルドが ablation ブランチだったという訂正(`BUG-151.md:42`)。
  - BUG-175: 手動の A/B(`BUG-175.md:17`、n は少数)。
- develop にある nofix 対照は 2 構成。
  - `sc-reopen-tsf-gji-gap600-nofix`: mutator `a8-no-gji-reopen-sync.sh`、BUG-170、[ADR-203](203-gji-fsm-follows-belief-open-transitions.md) (d)、expect=fail。run 36655470405 で OK。
    BUG-170 の状態欄は「解決済み(実機確認済み)」で、上の 19 件には入っていない。
  - `sc-adr209-chrome-msime-off`: 設定で ADR-209 の修正を切る。expect=fail、判定は入力先の文字(`か`)。BUG 番号には結びついていない。
  - このほか `a5-no-refresh20`(expect=fail)は「撤去すると壊れる」機構の対照。BUG の修正ではないので、本 ADR の nofix には数えない。
- CI 構成のコメントが参照する BUG は 17 件(BUG-020・071・103・112・113・147・149・150・152・156・163・170・171・172・176・184・185)。
  ```sh
  grep -oE 'BUG-[0-9]+' .github/workflows/{e2e-ime,wt-probe,bug112-probe}.yml .github/workflows/*verify*.yml | cut -d: -f2 | sort -u
  grep -n "mutator=\|'fail'" .github/workflows/e2e-ime.yml
  ```

### 2. nofix を回した 3 例: 回帰を捕まえた例は 0 件、誤った差が 2 回出た

| 例 | 結果 | nofix が果たした役目 |
|---|---|---|
| BUG-170(a8。run 36654801007 → 36655470405) | 修正を外しても、入力先のテキスト・cold 経路・固着は変わらず PASS した(`e2e-ime.yml:428-429` のコメント)。差が出たのは journal 上の同期を見る `--require-sync` だけ。この判定は nofix の run(2026-09-30 01:22Z)を見た後に、`2f3446fc`(同 01:30Z「負の対照で分かった検出力不足を補う」)で足された | **シナリオの弱さを見つけた**: テキストの判定が修正の有無を区別しないと分かった。回帰を捕まえたわけではない |
| BUG-114 WT 版(PR #534、`ci/bug114-wt-startup`、未マージ) | 下表 | **誤った差を 2 回出した**。「症状に届いていない」を示したのは、修正ありのビルドの INVALID 判定 |
| BUG-114 実 Chrome 版(PR #529、run 37422196139、測定器ごと閉じた) | GJI・MS-IME × 2 回 × 打鍵の 16 回すべてで drift 補正 0 件。a9 でも 0 件([ADR-235](235-app-ime-realmachine-matrix.md):110) | 差は出なかった。observed 0 なので、そもそも比べられない |

BUG-114 WT 版の 3 回の run(各 run 9 ジョブ = fix・nofix-a9・nofix-a10 を各 3 回、1 ジョブ 1 判定):

| run | 手順 | fix | nofix-a9 | nofix-a10 | 分かったこと |
|---|---|---|---|---|---|
| 37424011562(1 回目) | 打鍵+終了 | 3/3 PASS | 3/3 FAIL(`drift_correction_read` 57/50/54。PR #534 の本文と ADR-235:112 の値。この run のログは再取得していない) | — | **誤った差の 1 回目**。observed も N/N もそろっていたが、補正 12 本はすべて終了処理の `WM_CLOSE` 後の ObserverPoll だった(PR のレビュー M1)。判定対象の区間を決めていなかったので、条件を満たしたように見えた。基準 5(再武装 0)は、give-up から停止まで約 1.5 秒しかなく、構造上 FAIL になり得なかった(M2) |
| 37426810927(2 回目) | WT のペイン分割→閉じる | 9 ジョブすべて INVALID(PR #534 の本文の記載。この run のログは再取得できず、構成ごとの verdict は未確認) | | | キー操作なので `SkipTyping` で観測が止まり、閉じる前の補正は 0 件 |
| 37429061750(3 回目) | 補助窓への前面移動×2 | 3/3 INVALID | 3/3 FAIL | 3/3 INVALID | 9 判定すべてが「窓を閉じる前の drift 補正 0 件」(FAIL 3・INVALID 6)。fix の INVALID だけで「届いていない」と分かる。**誤った差の 2 回目**: a9 の 3 件にも `invalid` が付いているのに、判定器(`wt_pure.judge_bug114`)が FAIL を INVALID より優先するので verdict が FAIL になった。FAIL の理由は、a9 が固定した profile=ImmCross だけ |

```sh
gh run view 37429061750 --log | grep -c '"type": "bug114_result"'     # 9
gh run view 37429061750 --log | grep '"type": "bug114_result"' | grep -oE '"verdict": "[A-Z]+"|"invalid": \[[^]]*\]|"failures": \[[^]]*\]'
```
(2026-10-06 07:26 UTC 時点。PR #534 は修正対応中で、以後の push の結果はこの表に無い。)

まとめ:
- 3 例のどれも、nofix が回帰を捕まえた例ではない。nofix の役目は、BUG-170 でシナリオの弱さを見つけたことだけ。
- BUG-114 では、nofix が誤った差を 2 回出した。止めたのは PR のレビューと、observed 0 を INVALID にする判定。
- 費用の安い observed 件数の確認は、BUG-114 の 2 版で効いた。
- nofix が無かったせいで退行が develop に入った記録は、見つからなかった(known-bugs の本文と、上の 3 例の経緯で探した)。

### 3. 記録の再生との関係

- **古い再生**(決定の再計算。ADR-163 `replay_record`・RW・凍結コーパス)は「決定関数が記録時から変わったか」しか分からない
  ([ADR-225](225-journal-replay-ci-closed-loop-integration.md) F1。SP0 で直近 10 件の BUG のうち検知できたのは 0 件)。
  所有者の E2・E3 の決定どおり、新しい再生基盤への置き換えと同時に捨てる。
  journal 検討メモ(`docs/tasks/journal-replay-rebuild-study-2026-10-06/inventory.md:114-128`)は「退行を検知した記録は見つからなかった」としつつ、
  「痕跡が残らない可能性があるので需要の根拠にはしない」と注記している。本 ADR もこれを根拠にしない。
- **新しい再生基盤**: 所有者は 2026-10-06 に案 C(打鍵から actuation まで、[ADR-241](241-keystroke-to-actuation-replay.md)、起草中)を選んだ。
  入力の再生(B5)はエンジン側の不具合なら「HEAD で症状が出るか」に答えられる(同メモ `README.md:36-38`、実例 BUG-105・145)。
  ADR-241 は、その合否を「修正を外した mutator で再生だけが落ちる」ことで示すとしている。
  再生の層の合否の決め方は ADR-241 が持ち、本 ADR は実機の層の判定の書き方だけを決める。
- 役割分担(team-lead の決定、ADR-241 決定 7): **同期の判断**(打鍵から actuation の決定まで)は ADR-241 の再生が受ける。**再生が通らない部分**(非同期の経路と、実機固有の IME とのやり取り)を、実機 CI の再現シナリオと本 ADR の D1a が受ける。
- 単体テストの層では、修正を外して落ちるかを**手で確かめる道具がある**(`cargo mutants --in-diff`)。
  PR ごとに自動では回っておらず、回した結果を読む人もいない([ADR-234](234-core-modules-mutants-nightly.md):151・209 の書き分けに合わせる)。

### 4. ablation 機構の現状と腐敗

| ファイル(develop) | 外すもの | 参照元 | develop に当たるか |
|---|---|---|---|
| `a3-no-eisu-suppress.sh` | 無変換/変換単独タップの eisu reset 抑止(ADR-186 決定 2) | `e2e-ime.yml` からは無い。ローカル実験ランナー `run_experiments.sh:34`(E3)から参照 | **当たらない**。対象の抑止自体が `3f9b313e`(2026-09-19、撤去実験 E3 で不要と確認)で撤去済み |
| `a4-no-eisu-reset.sh` | eisu reset の判定関数 2 つ | `a4-no-eisu-reset`(pass) | 当たる |
| `a5-no-refresh20.sh` | 物理 IME キー通過後の 20ms 再読み取り | `a5-no-refresh20`(fail) | 当たる |
| `a6-no-idle-check.sh` | idle-conv-check | `a6-no-idle-check`(pass) | 当たる |
| `a7-no-follow.sh` | follow(ADR-187)の通過マーク | `atok-resync-nofollow`(observe、`e2e-ime.yml:170`)、`observe_grandfathered.txt:7`、`tools/e2e/ime_key_matrix/README.md:62`、`docs/teardown-verification-guide.md:88` | **当たらない**。`executor.rs` の置換元が `f1ef8d76`(2026-09-21)で変わった |
| `a8-no-gji-reopen-sync.sh` | BUG-170 の修正(ADR-203 (i)(ii)) | `sc-reopen-tsf-gji-gap600-nofix`(fail、`e2e-ime.yml:430`。run 36655470405 で OK) | 当たる |
| (PR #534)`a9-bug114-immcross-bootstrap.sh`・`a10-bug114-any-fresh-evidence.sh` | BUG-114 の修正 2 つ | `wt-probe.yml` の `bug114` ジョブ | 当たる |

```sh
T=$(mktemp -d); git archive origin/develop src crates | tar -x -C "$T" && (cd "$T" && bash <mutator>)   # a3・a7 は AssertionError(ビルドはしない)
git log origin/develop -S'ime.arm_mode_key_pass_mark(now);' --oneline -- crates/awase-windows/src/runtime/executor.rs  # f1ef8d76
git grep -n 'a3-no-eisu\|a7-no-follow\|a8-no-gji\|atok-resync-nofollow' origin/develop -- .github tools docs .claude
```

- 腐敗の型は 3 つある。
  - (i) 対象の修正自体が撤去された: a3。a1・a2 は `2f67c380`(2026-09-21)で同じ理由で削除済み。
  - (ii) 周辺のリファクタで置換元の文字列が変わった: a7。
  - (iii) 番号の衝突: `origin/ci/e2e-ime` の `46ae2175`(2026-09-25、develop 未マージ)は、a8〜a10 を ADR-191 の撤去実験(`a8-no-focus-probe-roman.sh` 等)に使っている。
    develop の a8(BUG-170)・PR #534 の a9/a10(BUG-114)とは別物。`ci/e2e-ime` は 2026-09-26(`00ce0c61`)から動いておらず、衝突が実際に起きるのは、そのブランチを develop に入れるとき。
- **腐敗は表に出ても直されなかった**。a7 は `ci/e2e-ime` の run 36209224218(2026-09-26 01:40Z)で、`build (atok-resync-nofollow, …)` が
  `AssertionError: crates/awase-windows/src/runtime/executor.rs` で落ちた。直後の run 36217202077 は `workflow_dispatch` で構成を絞って success にし、a7 は直されないまま残った。
  ```sh
  gh run view 36209224218 --log-failed | grep AssertionError
  ```
  2 人目のレビュアーは 2026-09-21 23:00Z 以降の e2e-ime の run 約 100 件のジョブ名を調べ、`nofollow` を含むジョブを 0 件と数えた(3 件はサーバーエラーで未確認)。
  この run 36209224218 は、その未確認の中にあった可能性がある(未確認)。いずれにせよ、構成が observe(合否に使わない)なので、絞って回避しても誰も困らなかった。
- nofix 構成は `ci/*` ブランチへの push か dispatch でしか走らない(`e2e-ime.yml:38-52`)。PR ゲートの `e2e-ime-smoke.yml` は `atok-passthrough-cold,baseline` だけを回す(`e2e-ime-smoke.yml:58-62`)。

### 5. 運用コスト(実測)

- wt-probe `bug114`: 1 ジョブ約 3 分(デバッグビルド約 80 秒を含む。run 37426810927・37429061750)。9 ジョブで約 27 ランナー分、壁時計 6〜15 分。
- e2e-ime の nofix: キャッシュが当たればビルド約 20 秒、e2e ジョブ 1 回約 2 分(run 36655470405)。キャッシュの外れたビルドの時間は未測定。
- ビルドのキャッシュのキーは `ablations/**` を含む(`e2e-ime.yml:781`)。`ablations/` 配下を変えると、baseline を含む**全構成**のキャッシュが 1 回外れる。
- mutator の保守: 1 本 9〜18 行。(ii) の腐敗は置換元の 1 行を書き換えれば直る。ただし、その修正が今のコードのどこにあるかを読み直す必要がある。

## 決定

### D0. 撤去(第 1 段階の中身)

当たらない mutator と、その参照をすべて消す。a8 を BUG 番号の名前に変える(第 1 段階の表)。本番コードは変えない。

### D1a. 実機 CI シナリオの判定の書き方(規約。置き場所は `teardown-verification-guide.md`)

実機 CI のシナリオ(Windows ランナーで実 IME を使う `e2e-ime.yml` の構成・`wt-probe.yml` のジョブなど)を新しく足すとき、または判定を変えるときは、次のように書く。
単体テスト・golden・閉ループ・再生の層は対象外(同期の判断の再生は ADR-241 が受け、その合否も ADR-241 が決める)。

1. **最初の run の前に、PR 本文かコミット本文に書く**:
   - 判定対象の区間(例: 「窓を閉じる時刻より前の行だけ」。BUG-114 の 1 回目は、これが無かったので終了処理の補正を数えた)。
   - 前提の基準と症状の基準。
   - 症状として数えるフィールドの一覧(observed として数えるもの)。
   run の後に基準を変えたら、それ以前の run は根拠に使わず、変えた理由を本文に残す(PR #534 の「基準 v1 → 3 回目の push の前に追加」の書き方)。
2. **修正や mutator が直接変える量は、症状のフィールドの一覧に入れない**(前提の側にだけ置ける)。例: a9 は起動時の profile を ImmCross に固定するので、profile は前提。
   - 例外: 分類の誤りそのものが症状の BUG(修正が「値を正しく出す」もの)では、その値も症状に入れてよい。ただし、値の先にある利用者に見える結果(`SendInput` の件数・入力先の文字など)を、症状のフィールドに 1 つ以上並べる。
3. **observed 0 の回は、FAIL より INVALID を優先する**。判定器の優先順位は INVALID > FAIL > PASS。observed 件数は結果の表に出す。
   今の `wt_pure.judge_bug114` は FAIL > INVALID > PASS で、3 回目の a9 の誤った差の直接の原因。PR #534 の仕上げで直す。
4. 修正ありのビルドで、有効回が N/N PASS(N ≥ 3)。INVALID は分母から外すが、件数は報告する。

### D1b. nofix との比較(任意)

nofix を回すかは作者が決める。回したときは次のとおり書く。

- nofix(mutator のビルド、または既にある設定での無効化)で、有効回が N/N、**症状の基準**で FAIL したときだけ、「このシナリオは修正の有無を区別する」と書ける。前提の基準だけで落ちた回は数えない。
- nofix で症状が出なければ(PASS、または observed 0)、「CI では修正の有無を区別できなかった(run 番号)」と書く。
- BUG-170 の `--require-sync` のように、nofix で差が出なかった後に足した判定は「journal 上の同期を確かめた」とだけ書く。利用者に見える症状を確かめたとは書かない。
- `e2e-ime.yml` の summary は `expect=fail` を「`n['fail'] > 0` なら OK」と判定する(`grep -n "期待FAILだが全PASS" .github/workflows/e2e-ime.yml`)。
  FAIL の理由を区別しないので、**D1b の根拠には使わない(表示だけ)**。区別したかは、各回の判定器の出力(症状の基準で落ちたか)で確かめる。

任意にする理由: 3 例で回帰を捕まえた例は 0 件、誤った差が 2 回(背景 2)。nofix が無いせいで退行を見逃した記録も 0 件。

### D2. 再生(ADR-241)との分担と、規約の文言

- 同期の判断(打鍵から actuation の決定まで)は ADR-241 の再生が受ける。再生が通らない非同期・実機固有の部分を、実機 CI の再現シナリオ+D1a が受ける。
- `.claude/rules/fix-requires-evidence.md:22-26` の (b) の節(「将来、再生トレースの追加に置き換える予定」)の書き換えは、**ADR-241 だけが行う**(新しい再生基盤を導入するのは ADR-241)。本 ADR は fix-requires を編集しない。
- 本 ADR は、その書き換えに入れる 1 項目の文案を ADR-241 への追記案として出す(関連する既存文書への追記案を参照): 「IME とのやり取りが絡む不具合で、再生が通らない部分(非同期・実機固有)は、実機 CI の再現シナリオで受ける。ADR-240 D1a の条件を満たしたものだけを (a) と数える」。
- D1a の詳しい書き方は、mutator の説明が既にある `docs/teardown-verification-guide.md` に置く。

### D3. mutator の命名と置き場所

- 新しい nofix の mutator は、BUG 番号で名付ける: `ablations/bug<NNN>-<何を外すか>.sh`。
  ファイル先頭のコメントに「**直接変える量**」を 1 行書く(D1a-2 で症状の一覧から外すもの)。
- ADR-241 の mutator(再生の合否用)は再生側の資産で、置き場所は再生のテストの側(ADR-241 決定 7)。`ablations/` には混ぜない。本 ADR の ablation(実機 CI 側の資産)とは分けて、両方残す。
- ADR-186・191 型の「機構は要るか」の撤去実験は、従来どおり aN でよい。`origin/ci/e2e-ime` の a8〜a10 はこちらに入るので改名しない。衝突は develop の a8 を `bug170-no-gji-reopen-sync.sh` に改名すれば解ける。
- 撤去済みの機構に依存する mutator は、機構の撤去と同じコミットで消す(`2f67c380` の前例)。

### D4. 腐敗の検出(第 2 段階。mutator が 5 本を超えたら着手)

`e2e-ime-smoke.yml` の Linux ジョブ(`invariants-unit` と同じ ubuntu ランナー)に、`ablations/*.sh` を 1 本ずつチェックアウト直後のツリーに当て、
「`assert` が通り、差分が出る」ことだけを確かめる手順を足す(ビルドはしない。数秒)。
対象は `ablations/` の mutator(実機 CI 側の資産)だけ。参照元は `.github/workflows/*.yml` のすべてと `run_experiments.sh` とし、どこからも参照されない mutator があれば落とす。ADR-241 の mutator は再生側の資産で `ablations/` の外に置くので、この検査の対象にならない(落とさない)。

- 「当たる」ことは「nofix で症状が出る」ことを保証しない。後者は実機の run でしか分からない。
- 第 1 段階の後、develop の mutator は 4 本(a4・a5・a6・bug170)になる。人が目で見れば足りるので、5 本を超えるまでは着手しない。

### D5. nofix の実機 run の頻度

PR の必須チェックにはしない。nofix 構成を足す・変える PR では、作者が手で `ci/*` ブランチへ push して 1 回回す
(`e2e-ime-smoke.yml` は `only: atok-passthrough-cold,baseline` なので、自動化されない)。
定期実行にする場合、`schedule` は既定ブランチで動く。このリポジトリの既定ブランチは develop(`gh repo view --json defaultBranchRef -q .defaultBranchRef.name`)。

## 段階

### 第 1 段階(撤去・改名・説明の文言のみ、本番コードの変更なし)

| 内容 | 撤去・変更の対象 | 検証(CI) | 取りやめ条件 |
|---|---|---|---|
| a3 を消す | `ablations/a3-no-eisu-suppress.sh`、`run_experiments.sh:34` の E3 行。`results/E3.txt`・`results/SUMMARY.md` は過去の結果なので残す | `git grep -n a3-no-eisu -- .github tools docs .claude ':!tools/e2e/ime_key_matrix/results' ':!docs/adr/240-*'` が 0 件 | なし |
| a7 と構成を消す | `ablations/a7-no-follow.sh`、`e2e-ime.yml:170` の `atok-resync-nofollow`、`observe_grandfathered.txt:7`、`tools/e2e/ime_key_matrix/README.md:62` の a7 の説明、`docs/teardown-verification-guide.md:88` の例(`bug170-*` に差し替え)。`docs/adr/187-*.md:123` は履歴なので残す | `e2e-ime-smoke.yml` の `invariants-unit`(`test_e2e_plan.py` の `test_grandfathered_only_shrinks`〈`:51-56`〉を含む)と、e2e-ime の plan ジョブが通る | 所有者が修理を選んだら(所有者に聞く 2)、消さずに置換元を直す |
| a8 を BUG 番号で改名 | `a8-no-gji-reopen-sync.sh` → `bug170-no-gji-reopen-sync.sh`(先頭に「直接変える量」の 1 行)、`e2e-ime.yml:430` の参照 | `ci/e2e-scenarios` へ push して、`sc-reopen-tsf-gji-gap600-nofix` が従来どおり OK | `ablations/**` を変えるので、baseline を含む全構成のビルドのキャッシュが 1 回外れる(背景 5)。それ以外の影響が出たら改名を戻す |
| `teardown-verification-guide.md` | `:104` の「`a1`〜`a7`」の古い行(a1・a2 は削除済み、a3・a7 はこの段階で削除)。mutator の節(`:81-89`)に D1a の要点(3 行以内)と命名(D3)を足す | docs のみ | なし |
| BUG-114 の仕上げ(判断は本 ADR が持ち、ADR-235 D3 は参照するだけ) | PR #534 の判定器を D1a-3(INVALID > FAIL)に直す。閉じる前の observed が 0 のままなら、a9・a10 と `bug114` ジョブは入れず、BUG-114.md に「WT と実 Chrome の CI では drift 補正に届かなかった(run 37429061750・37422196139)」と書く | PR #534 の run の verdict と observed 件数 | 観測が届く手順が見つかれば、D1a に従って入れる(nofix の a9・a10 を残すなら `bug114-*.sh` に改名し、D1b で書く) |

行数の収支:
- 削除: mutator 2 本(約 20 行)、構成 1 行、grandfathered 1 行、`run_experiments.sh` 1 行、README 1 行、ガイドの古い行 1 行。
- 追加: ガイドの説明 3〜5 行。rules への追加は無い(D2)。
- 撤去のほうが多い。

### 第 2 段階(腐敗の検出、D4。mutator が 5 本を超えたら)

- 追加: Linux ジョブの手順 1 つ(シェル 15 行前後の見込み、未実測)。撤去先は無い(新しい検査)。
- 検証: わざと置換元を変えたブランチで、このジョブが落ちることを 1 回確かめる。
- 取りやめ条件: 5 本を超えない間は着手しない。

既存の BUG に nofix を広げる段階は置かない。所有者が「広げる」を選んだ場合の扱いは、所有者に聞く 1 の (B) に書く。

## 却下した案と理由

- **nofix を実機シナリオの回帰テストの条件(義務)にする(起草時の推奨 C)**: 3 例で回帰を捕まえた例は 0 件、誤った差が 2 回。nofix が無いせいで退行を見逃した記録も 0 件(背景 2)。
- **古い再生(決定の再計算・凍結コーパス)を回帰の証拠にする**: ADR-225 F1・SP0 0 件。所有者の E2・E3 で捨てる。新しい再生基盤は ADR-241 が受け持つ。
- **全 BUG に実機 CI シナリオを義務づける**: 「CI 検証済み」は 19/186 件で、残りは CI で再現する手段が無い。義務にすると、known-bugs への記録で済ませていた BUG の修正が止まる。
- **nofix のためだけに本番コードへ設定・cfg・feature を足す**: 本番に分岐が増え、修正 1 件ごとに残り続ける。**既にある設定**を使うこと(`sc-adr209-chrome-msime-off` の形)は可。ビルドが要らず腐敗もしない。
- **nofix を PR の必須チェックにする**: 1 BUG あたり約 27 ランナー分(背景 5)。V1(#530)を当面必須にしない所有者の決定と同じ扱い。
- **宣言テーブルで「BUG → 構成 → mutator」を生成する**: ADR-218〜220 の却下と同じく、対象が 2 件の段階で表を作る価値は無い。
- **本 ADR が `fix-requires-evidence.md` を書き換える**: ADR-241 と同じ節を別々に書き換えることになる。書き換えは ADR-241 に一本化し、本 ADR は 1 項目の文案を出すだけにする(D2)。

## リスクと限界

- D1a は書き方の規約で、守られているかは PR のレビューでしか捕まらない。D1a-3 だけは判定器のコードで機械的に守れる。
- 前提と症状の分け方、特に D1a-2 の例外(分類の誤りが症状の BUG)に当たるかは、作者の判断になる。
- nofix で差が出ても、そのシナリオが利用者の症状を捕まえるとは限らない。BUG-170 は journal 上の同期でしか差が出なかった。
- mutator は「修正前のコード」を再現しない。nofix は「今のコードから修正の要点だけを外したもの」で、修正前の挙動と一致するかは未確認。
- 実機 CI の揺らぎ(フォーカス喪失で INVALID など)で、N ≥ 3 がそろわない構成がありうる。
- 未確認の点: PR #534 の最終結果、BUG-114 の 1 回目の run のログ(57/50/54 は PR 本文と ADR-235 の記載だけで照合)、キャッシュの外れたビルドの所要時間、a7 の run 36209224218 が 2 人目のレビュアーの数えた範囲に入っていたか。

## 所有者に聞くこと

1. **範囲**
   - (A) D0(撤去)と D1a(判定の書き方)だけを決める。nofix は任意(D1b)。既存の BUG には広げない。
   - (B) A に加え、「CI 検証済み」19 件から 3 件を D1b で試す。最初の 3 件で、症状の基準で差が出たものが 0 件ならやめる。
     候補は、CI 構成が既にあり、修正の hunk が develop に半分以上当たるもの(BUG-156・163・172・176 など。hunk が当たっても nofix が意味を持つとは限らず、個別の確認は未実施)。
   - 推奨: **A**。理由: D1a は費用が小さく、BUG-114 の 2 版で効いた。nofix は 3 例で回帰を捕まえた例が無く、誤った差を 2 回出した。義務や遡及に広げる実害の記録が無い。
2. **a7(nofollow)を消すか直すか**
   - 消す: 5 か所(mutator・構成・grandfathered・README・ガイド)。失うのは、ADR-187 の「follow を無効にしてずれ(各回 4 件)を起こしても、リセット 16/16 成功」(`187-*.md:123`)を再確認する手段。
     `atok-resync` はずれを起こさないので、回復までは試さない。CI の合否は変わらない(observe)。
   - 直す: 置換元の 1 行(`f1ef8d76` で変わった `ime.arm_mode_key_pass_mark(now);` の周辺)を書き換える。
   - 推奨: **消す**。理由: observe で合否に使っておらず、2026-09-26 に壊れたと分かっても、構成を絞って回避されたまま直されなかった。
3. **nofix の実機 run の頻度**: 推奨は D5 のとおり、必須にせず、nofix を足す・変える PR で手で 1 回回す。リリース前にも回すなら、`release-develop-to-main` スキル(リポジトリ外)に 1 手順を足す(リリースが壁時計で 6〜15 分延びる)。

## 関連する既存文書への追記案(この ADR では書き換えない)

- **ADR-241 への追記案(役割分担は ADR-241 決定 7 と同じ)**:
  - `.claude/rules/fix-requires-evidence.md:22-26` の書き換え(ADR-241 だけが行う)に、次の 1 項目を含める(ADR-241 の文案の 2 項目めと同じ趣旨):
    > IME とのやり取りが絡む不具合で、再生が通らない部分(非同期・実機固有)は、実機 CI の再現シナリオで受ける。ADR-240 の D1a の条件(判定対象の区間・前提と症状の基準・症状として数えるフィールドを最初の run の前に書く、observed 件数を出す、observed 0 の回は FAIL より INVALID を優先する)を満たしたものだけを (a) と数える。
  - mutator: ADR-241 の mutator(再生の合否用)は再生のテストの側に置き、`ablations/` に混ぜない。ADR-240 の D4 はそれを検査しない。両方残す。
  - ADR-241 が引く「ADR-240 の条件」は、本 ADR の改訂で「D1a(必須)と D1b(任意)」に分かれた。再生の層の条件は ADR-241 が決める。
- `docs/teardown-verification-guide.md:81-89`: 次の 3 行と命名を足し、`:88` の例を `bug170-*` に差し替える。`:104` の「`a1`〜`a7`」を現状に直す。
  > 実機シナリオの判定は、最初の run の前に、判定対象の区間・前提の基準・症状として数えるフィールドを書く。mutator が直接変える量は症状に入れない。observed 0 の回は FAIL より INVALID を優先する([ADR-240](adr/240-ablation-ab-as-regression-evidence.md) D1a)。nofix との比較は任意で、症状の基準で落ちた回だけを「修正の有無を区別した」と数える(D1b)。
- `docs/adr/235-app-ime-realmachine-matrix.md` D3: 「BUG-114 の判断は ADR-240 第 1 段階に従う」とだけ書く。今の文言は既にこの形に近い。
- `docs/known-bugs/BUG-114.md`: PR #534 の結論(届いたか、届かなかったか)を run 番号つきで 1〜3 行。
- `docs/known-bugs/BUG-170.md`: 「nofix(a8)で差が出たのは、後から足した `--require-sync`(journal 上の同期)だけで、入力先のテキストでは差が出なかった(run 36654801007、`2f3446fc`)」の 1 行。
- `docs/adr/index.md`: ADR-240 の 1 行は、team-lead が統合時に足す(本ブランチでは触らない)。
