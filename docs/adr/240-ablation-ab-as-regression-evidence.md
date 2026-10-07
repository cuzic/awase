---
id: ADR-240
title: |-
  実機 CI シナリオを回帰の証拠と数える条件(observed 件数は必須、修正を外したビルドとの比較は任意)と、ablation の整理
summary: |-
  実機 CI のシナリオ(`e2e-ime.yml` の構成・`wt-probe.yml` のジョブ)を fix-requires-evidence の (a)(回帰テスト)と数える条件を決める。
  (D1a・必須)判定基準を「前提」と「症状」に分けて最初の run の前に書き、症状の基準が見る事象の observed 件数を出し、observed 0 の回は理由が何であれ INVALID にする(FAIL より INVALID を優先)。
  (D1b・任意)修正を外したビルド(nofix。`ablations/*.sh` の mutator、または既にある設定での無効化)で症状の基準が FAIL したときだけ「修正の有無を区別するシナリオ」と書ける。
  背景の実測(develop `8810b812`、2026-10-06): known-bugs 186 件のうち状態欄に「CI 検証済み」とあるのは 19 件で、そのうち nofix で差を確かめた記録は 0 件。nofix を回した 3 例(BUG-170 の a8、BUG-114 の WT 版 PR #534、BUG-114 の実 Chrome 版 PR #529)は、どれも D1 を満たさない。nofix に固有の効果は BUG-170 の 1 件で、入力先の文字の判定が修正の有無を区別しないと分かったこと。observed 件数の数え直しが効いたのは BUG-114(閉じる前に限ると 18 回すべて drift 補正 0 件)。
  記録の再生について: 決定の再計算(ADR-163 `replay_record`)は「HEAD で症状が出るか」に答えない(ADR-225 F1)。入力の再生(B5)はエンジン側の不具合なら答えられる(journal 検討メモ)。IME とのやり取りが絡む不具合はどちらでも答えられない。そこで fix-requires の (b) にある「将来、再生トレースへ置き換える」は削らず、エンジン側は B5、IME 側は本 ADR の D1、と書き分ける(D2)。
  ablation 機構の整理: mutator 6 本のうち a3・a7 は develop にもう当たらない。a7 は 2026-09-26 の CI で実際にビルドが落ちたが、構成を絞った dispatch で回避されたまま直されていない。第 1 段階は撤去と改名だけ(a3 と `run_experiments.sh` の E3 行、a7 と `atok-resync-nofollow` 構成・grandfathered の行・ガイド 2 か所、a8 を `bug170-*` に改名、fix-requires の文言)。第 2 段階で、mutator が当たるかを Linux で数秒で確かめるジョブを足す。nofix の実機 run は必須チェックにしない。
status: |-
  提案(起草中、Opus round1〈Blocker 1・Must 5・Should 6・Nit 4〉反映済み、round2 前)
related_adr:
  - "ADR-163"
  - "ADR-186"
  - "ADR-187"
  - "ADR-191"
  - "ADR-203"
  - "ADR-205"
  - "ADR-209"
  - "ADR-225"
  - "ADR-226"
  - "ADR-134"
  - "ADR-232"
  - "ADR-234"
  - "ADR-235"
---

# ADR-240: 実機 CI シナリオを回帰の証拠と数える条件と、ablation の整理

番号の注記: 起草時は ADR-233 だったが、develop の `233-stale-high-observation-beats-newer-in-most-recent-trusted.md` と衝突したため 240 に改番した。
件数・行番号は、断りのない限り `origin/develop` の `8810b812`(2026-10-06)時点。行番号は動くので、再確認のコマンドを併記する。

## 用語

- **ablation(撤去実験)/ mutator**: develop のコードを機械的に書き換える小さなスクリプト
  (`tools/e2e/ime_key_matrix/ablations/aN-*.sh`。Python の文字列置換で、置換元が 1 回だけ現れることを `assert` する)。
  `e2e-ime.yml` の `cfg(..., mutator=...)` で指定すると、その構成だけ書き換えたビルドで走る(`grep -n '撤去(コード変更)を当てる' -A7 .github/workflows/e2e-ime.yml`)。
- **nofix**: バグの修正だけを外したもの。外し方は 2 つある: mutator で書き換えたビルド、または**既にある設定**での無効化
  (例: `sc-adr209-chrome-msime-off`、`general='predict_henkan_open_in_unreadable_windows = false'`、`e2e-ime.yml:343-347`)。
  ADR-186 の撤去実験(「この機構は要るか」)とは目的が違い、「このシナリオは修正が無ければ症状を出すか」を確かめる。
- **前提(precondition)と症状(symptom)**: 判定基準のうち、「測定が成り立った」ことを確かめる基準(起動時スコープが期待どおり、プロセスの切り替えが 1 回、など)が前提。
  利用者に見える症状、またはその直接の原因になる送信(drift 補正の連発・`SendInput` の件数・入力先の文字)を見る基準が症状。
- **observed 件数**: 症状の基準が見る事象が、判定対象の区間に何件あったか。0 件の PASS は「起きなかった」ではなく「通っていない」
  ([ADR-205](205-observe-external-ime-close-in-imm32-unavailable-windows.md):39 と同じ教訓)。

## 背景(実測)

### 1. 「CI 検証済み」の BUG で nofix の差を確かめた記録は 0 件

- known-bugs は 186 件。frontmatter の `fix_commits` が空でないのは 92 件。状態欄に「CI 検証済み」とあるのは 19 件
  (BUG-025・071・148・150・151・156・157・158・159・162・163・165・166・168・172・176・179・180・183)。
  この 19 件の本文に、nofix で差を確かめた記録は無い。
  ```sh
  ls docs/known-bugs/BUG-*.md | wc -l                                                # 186
  grep -L -E '^fix_commits:\s*\[\]' docs/known-bugs/BUG-*.md | wc -l                   # 92
  grep -l -m1 -E '^\*\*状態[^*]*\*\*.*CI ?検証済み' docs/known-bugs/BUG-*.md | wc -l      # 19
  grep -l -E 'nofix|ablation|撤去したビルド|修正を外' docs/known-bugs/BUG-*.md            # BUG-115・151・175(下記)
  ```
  語が当たる 3 件はどれも実機 CI の nofix ではない。BUG-115 は単体テスト(`BUG-115.md:429`)、BUG-151 は測定ビルドが ablation ブランチだったという訂正(`BUG-151.md:42`)、
  BUG-175 は手動の A/B(`BUG-175.md:17`、n は少数)。
- develop にある nofix 対照は 2 構成。
  - `sc-reopen-tsf-gji-gap600-nofix`(mutator `a8-no-gji-reopen-sync.sh`、BUG-170、[ADR-203](203-gji-fsm-follows-belief-open-transitions.md) (d))。BUG-170 の状態欄は「解決済み(実機確認済み)」で、上の 19 件には入っていない。
  - `sc-adr209-chrome-msime-off`(設定で ADR-209 の修正を切る。expect=fail、判定は入力先の文字 `か`)。BUG 番号には結びついていない。
  - このほか `a5-no-refresh20`(expect=fail)は「撤去すると壊れる」機構の対照で、BUG の修正ではないので D1 の対象外。
- CI 構成のコメントが参照する BUG は 17 件(BUG-020・071・103・112・113・147・149・150・152・156・163・170・171・172・176・184・185)。
  ```sh
  grep -oE 'BUG-[0-9]+' .github/workflows/{e2e-ime,wt-probe,bug112-probe}.yml .github/workflows/*verify*.yml | cut -d: -f2 | sort -u
  grep -n "mutator=\|'fail'" .github/workflows/e2e-ime.yml
  ```

### 2. nofix を回した 3 例は、どれも本 ADR の条件(D1)を満たさない

| 例 | 結果 | D1 に照らすと | 何が効いたか |
|---|---|---|---|
| BUG-170(a8、run 36654801007 → 36655470405) | 修正を外しても、入力先のテキスト・cold 経路・固着は変わらず PASS(`e2e-ime.yml:428-429` のコメント)。差が出たのは journal 上の同期を見る `--require-sync` だけで、これは nofix の結果を見た後に `2f3446fc`(2026-09-29「負の対照で分かった検出力不足を補う」)で足された | 基準を run の後に足したので、回帰テストには数えない(D1a-1) | **nofix に固有の効果**: テキストの判定が修正の有無を区別しないと分かった |
| BUG-114 WT 版(PR #534、`ci/bug114-wt-startup`、未マージ) | 下表 | 3 回目は observed 0 で INVALID(D1a-3)。a9 の FAIL は mutator が書き換えた値(profile)を読んだだけ(D1a-2) | **observed 件数の数え直し**(1 回目の誤りはレビュー M1 が見つけた) |
| BUG-114 実 Chrome 版(PR #529、run 37422196139、測定器ごと閉じた) | GJI・MS-IME × 2 回 × 打鍵の 16 回すべてで drift 補正 0 件。a9 でも 0 件([ADR-235](235-app-ime-realmachine-matrix.md):110) | observed 0 | 実 Chrome では drift 補正に届かないと分かった |

BUG-114 WT 版の 3 回の run:

| run | 手順 | fix | nofix-a9 | nofix-a10 | 分かったこと |
|---|---|---|---|---|---|
| 37424011562(1 回目) | 打鍵+終了 | 3/3 PASS | 3/3 FAIL(`drift_correction_read` 57/50/54) | — | 補正 12 本すべてが終了処理の `WM_CLOSE` 後の ObserverPoll(レビュー M1)。基準 5(再武装 0)は give-up から停止まで約 1.5 秒しかなく構造上 FAIL になり得なかった(M2) |
| 37426810927(2 回目) | WT のペイン分割→閉じる | 9 ジョブ INVALID | | | キー操作なので `SkipTyping` で観測が止まり、閉じる前の補正 0 件 |
| 37429061750(3 回目) | 補助窓への前面移動×2 | 3/3 INVALID | 3/3 FAIL | 3/3 INVALID | 18 回すべて「窓を閉じる前の drift 補正 0 件」。a9 の 3 件も `invalid` が付いているのに、判定器が **FAIL を INVALID より優先する**ため verdict が FAIL になった。FAIL の理由は profile=ImmCross だけ |

```sh
gh run view 37429061750 --log | grep '"type": "bug114_result"' | grep -oE '"verdict": "[A-Z]+"|"invalid": \[[^]]*\]|"failures": \[[^]]*\]'
```
(2026-10-06 07:26 UTC 時点。PR #534 は修正対応中で、以後の push の結果はこの表に無い。)

まとめ: nofix は回帰網を作ったのではなく、**回帰網ができていないことを表に出した**。費用の安い observed 件数の確認は 2 例(BUG-114 の 2 版)で効いた。nofix に固有の寄与は BUG-170 の 1 件。

### 3. 記録の再生: 決定の再計算は答えず、入力の再生はエンジン側だけ答える

- **決定の再計算**(記録した Facts から `decide` を再計算する。ADR-163 `replay_record`)は、「決定関数が記録時から変わったか」しか分からない
  ([ADR-225](225-journal-replay-ci-closed-loop-integration.md) F1。SP0 で直近 10 件の BUG のうち検知できたのは 0 件)。
  journal に触れる BUG 45 件で、replay fixture が退行を検知した記録は見つからなかった(`docs/tasks/journal-replay-rebuild-study-2026-10-06/inventory.md:114-128`。痕跡が残らない可能性があるので需要の根拠にはしない、と同メモが注記)。
- **入力の再生**(B5。記録した `KeyInput` 列を HEAD の分類器とエンジンに流す)は、**エンジン側の不具合なら「HEAD で症状が出るか」に答えられる**
  (同メモ `README.md:36-38`、実例 BUG-105・145)。所有者は B5 を BUG-105 の 1 件で試作すると回答済み(同 `README.md:20` の E1、E7)。
- **IME とのやり取りが絡む不具合**は、決定の再計算でも入力の再生でも答えられない(同 `README.md:65`。閉ループ B6 でクセを写した範囲だけ)。
  ここが実機 CI シナリオの受け持ちで、本 ADR の対象。
- 単体テストの層では、修正を外して落ちるかを確かめる**手で回せる道具がある**(`cargo mutants --in-diff`。PR ごとの自動実行はしていない。[ADR-234](234-core-modules-mutants-nightly.md):151 と同じ書き方)。

### 4. ablation 機構の現状と腐敗

| ファイル(develop) | 外すもの | 参照元 | develop に当たるか |
|---|---|---|---|
| `a3-no-eisu-suppress.sh` | 無変換/変換単独タップの eisu reset 抑止(ADR-186 決定 2) | `e2e-ime.yml` からは無し。ローカル実験ランナー `run_experiments.sh:34`(E3)から参照 | **当たらない**。対象の抑止自体が `3f9b313e`(2026-09-19、撤去実験 E3 で不要と確認)で撤去済み |
| `a4-no-eisu-reset.sh` | eisu reset の判定関数 2 つ | `a4-no-eisu-reset`(pass) | 当たる |
| `a5-no-refresh20.sh` | 物理 IME キー通過後の 20ms 再読み取り | `a5-no-refresh20`(fail) | 当たる |
| `a6-no-idle-check.sh` | idle-conv-check | `a6-no-idle-check`(pass) | 当たる |
| `a7-no-follow.sh` | follow(ADR-187)の通過マーク | `atok-resync-nofollow`(observe、`e2e-ime.yml:170`)、`observe_grandfathered.txt:7` | **当たらない**。`executor.rs` の置換元が `f1ef8d76`(2026-09-21)で変わった |
| `a8-no-gji-reopen-sync.sh` | BUG-170 の修正(ADR-203 (i)(ii)) | `sc-reopen-tsf-gji-gap600-nofix`(fail、`e2e-ime.yml:430`) | 当たる |
| (PR #534)`a9-bug114-immcross-bootstrap.sh`・`a10-bug114-any-fresh-evidence.sh` | BUG-114 の修正 2 つ | `wt-probe.yml` の `bug114` ジョブ | 当たる |

```sh
T=$(mktemp -d); git archive origin/develop src crates | tar -x -C "$T" && (cd "$T" && bash <mutator>)   # a3・a7 は AssertionError(ビルドはしない)
git log origin/develop -S'ime.arm_mode_key_pass_mark(now);' --oneline -- crates/awase-windows/src/runtime/executor.rs  # f1ef8d76
grep -rn 'a3-no-eisu\|a7-no-follow\|atok-resync-nofollow' .github tools docs .claude
```

- 腐敗の型は 3 つ:
  - (i) 対象の修正自体が撤去された: a3。a1・a2 は `2f67c380`(2026-09-21)で同じ理由で削除済み。
  - (ii) 周辺のリファクタで置換元の文字列が変わった: a7。
  - (iii) 番号の衝突: `origin/ci/e2e-ime` の `46ae2175`(2026-09-25、develop 未マージ)は、a8〜a10 を ADR-191 の撤去実験(`a8-no-focus-probe-roman.sh` 等、「機構は要るか」)に使っている。develop の a8(BUG-170)と PR #534 の a9/a10(BUG-114)とは別物。
- **腐敗は表に出ても直されなかった**: a7 は `ci/e2e-ime` の run 36209224218(2026-09-26 01:40Z)で、`build (atok-resync-nofollow, …)` が
  `AssertionError: crates/awase-windows/src/runtime/executor.rs` で落ちた。直後の run 36217202077 は `workflow_dispatch` で構成を絞って success にしており、a7 は直されないまま残った
  (`gh run view 36209224218 --log-failed | grep AssertionError`)。構成が observe(合否に使わない)なので、絞って回避しても誰も困らなかった。
- nofix 構成は `ci/*` ブランチへの push か dispatch でしか走らない(`e2e-ime.yml:38-52`)。PR ゲートの `e2e-ime-smoke.yml` は `atok-passthrough-cold,baseline` だけを回す(`e2e-ime-smoke.yml:58-62`)。
- 修正コミットの「外しやすさ」の粗い目安(`fix_commits` のある 90 件〈起草時 `1630e55e`〉。本番の `.rs` の hunk を develop に逆向きに当てる `patch -R --dry-run -F0`):
  全 hunk が当たるもの 5 件、半分以上 61 件、半分未満 12 件、0 件 7 件、本番コードを含まないもの 5 件。
  hunk が当たっても、その修正だけを外したビルドが意味を持つとは限らない(未確認)。当たらない修正は mutator を新しく書く必要がある。

### 5. 運用コスト(実測)

- wt-probe `bug114`: 1 ジョブ約 3 分(デバッグビルド約 80 秒を含む。run 37426810927・37429061750)。fix/nofix-a9/nofix-a10 × 3 回 = 9 ジョブ、約 27 ランナー分、壁時計 6〜15 分。
- e2e-ime の nofix: キャッシュが当たればビルド約 20 秒、e2e ジョブ 1 回約 2 分(run 36655470405)。キャッシュの外れたビルドの時間は未測定。
- mutator の保守: 1 本 9〜18 行。(ii) の腐敗は置換元の 1 行を書き換えれば直る。ただし直すには、その修正が今のコードのどこにあるかを読み直す必要がある。

## 決定

### D1a(必須). 実機 CI シナリオを回帰テストと数える条件

`fix-requires-evidence.md` の (a)(回帰テスト)のうち**実機 CI のシナリオ**(Windows ランナーで実 IME を使う `e2e-ime.yml` の構成・`wt-probe.yml` のジョブなど)は、次をすべて満たすときだけ (a) と数える。
単体テスト・golden・閉ループは対象外(従来どおり)。

1. **判定基準を最初の run の前に書く**(PR 本文かコミット本文)。基準は「前提」と「症状」に分け、症状の基準が見る事象(observed として数えるもの)を名指しする。
   run の後に基準を変えたら、それ以前の run は根拠に使わず、変えた理由を本文に残す(PR #534 の「基準 v1 → 3 回目の push の前に追加」の書き方)。
2. **修正や mutator が直接変える量は、前提の側にしか置けない**(例: a9 は profile を ImmCross に固定するので、「profile=TsfNative」は前提であって症状ではない)。
3. **observed 0 の回は理由が何であれ INVALID にし、FAIL にしない**。判定器の優先順位は INVALID > FAIL > PASS。observed 件数は結果の表に出す。
   (今の `wt_pure.judge_bug114` は FAIL > INVALID > PASS で、3 回目の a9 の誤りの直接の原因。PR #534 の仕上げで直す。)
4. 修正ありで、有効回が N/N PASS(N ≥ 3。INVALID は分母から外すが件数は報告する)。

### D1b(任意). 修正の有無を区別するシナリオと書ける条件

nofix(mutator のビルド、または**既にある設定**での無効化)で、有効回が N/N、**症状の基準**で FAIL したときだけ、known-bugs や PR に「このシナリオは修正の有無を区別する」と書ける。
前提の基準だけで落ちた回は数えない。nofix で症状が出なければ(PASS、または observed 0)そう書かず、「CI では修正の有無を区別できなかった(run 番号)」と書く。

- BUG-170 の `--require-sync` のように、nofix で差が出なかった後に足した判定は「journal 上の同期を確かめた」とだけ書き、利用者に見える症状を確かめたとは書かない。
- `e2e-ime.yml` の summary は `expect=fail` を「`n['fail'] > 0` なら OK」と判定する(`grep -n "期待FAILだが全PASS" .github/workflows/e2e-ime.yml`)。この判定は FAIL の理由を区別しないので、**D1b の根拠には使わない(表示だけ)**。
  D1b を満たしたかは、各回の判定器の出力(症状の基準で落ちたか)で確かめる。

D1b を任意にする理由: nofix を回した 3 例で D1 を満たしたものは 0(背景 2)。実績の無い手続きを義務にしない。D1a は費用が小さく、2 例で効いている。

### D2. `fix-requires-evidence.md` の「将来」の文を書き分ける

`fix-requires-evidence.md:22-26` の「将来的に ADR-159 の記録・再生基盤が育てば、(b) は『再生トレースの追加』へ置き換える予定」は削らず、次のように書き分ける。

- **エンジン側の不具合**: (b) は将来、入力の再生(B5、journal 検討メモの段階 4)による再現テストで置き換える。今は未実装。
- **IME とのやり取りが絡む不具合**: 再生では答えられない(同メモ `README.md:65`)。実機 CI シナリオを (a) と数える条件は ADR-240 の D1a。
- (b)(known-bugs への記録)自体は残す。
- ADR-162 E1/E4 への参照(能力ベースの前提条件)は、complexity-budget.md の TH1e が所有者の E2 で書き直し予定(同メモ `README.md:29-30`)なので、本 ADR では変えない。書き直しのときに合わせて見直す。

### D3. mutator の命名

- 新しい nofix の mutator は BUG 番号で名付ける: `ablations/bug<NNN>-<何を外すか>.sh`。ファイル先頭のコメントに「**直接変える量**」を 1 行書く(D1a-2 で前提の側に置くもの)。
- ADR-186・191 型の「機構は要るか」の撤去実験は従来どおり aN でよい。`origin/ci/e2e-ime` の a8〜a10 はこちらに入るので改名しない。
  衝突は develop の a8 を `bug170-no-gji-reopen-sync.sh` に改名すれば解ける。
- 撤去済みの機構に依存する mutator は、機構の撤去と同じコミットで消す(`2f67c380` の前例)。

### D4. 腐敗の検出(第 2 段階)

`e2e-ime-smoke.yml` の Linux ジョブ(`invariants-unit` と同じ ubuntu ランナー)に、`ablations/*.sh` を 1 本ずつチェックアウト直後のツリーに当て、
「`assert` が通り、差分が出る」ことだけを確かめる手順を足す(ビルドはしない。数秒)。
参照元は `e2e-ime.yml`・`wt-probe.yml`・`run_experiments.sh` の 3 つとし、どこからも参照されない mutator があれば落とす。

「当たる」ことは「nofix で症状が出る」ことを保証しない。後者は実機の run でしか分からない(D5)。
a7 の例(背景 4)では、実機で落ちても絞り込みで回避された。PR ごとの当たり確認なら、mutator を壊したコードの PR がその場で赤くなる。

### D5. nofix の実機 run の頻度

PR の必須チェックにはしない(所有者に聞く 2 の推奨)。nofix 構成を足す・変える PR では、作者が手で `ci/*` ブランチへ push して 1 回回す。
`e2e-ime-smoke.yml` は `only: atok-passthrough-cold,baseline` なので、これは自動化されない。
リリース前にも回すかは所有者に聞く 2 の (A') とする。採るなら `release-develop-to-main` スキル(リポジトリ外)に 1 手順を足す。リリースは壁時計で 6〜15 分延び、INVALID が出れば再実行が要る。
定期実行にする場合は `schedule` が既定ブランチで動き、このリポジトリの既定ブランチは develop(`gh repo view --json defaultBranchRef -q .defaultBranchRef.name`)。

## 段階

### 第 1 段階(撤去・改名・文言のみ、本番コードの変更なし)

| 内容 | 撤去・変更の対象 | 検証(CI) | 取りやめ条件 |
|---|---|---|---|
| a3 を消す | `ablations/a3-no-eisu-suppress.sh`、`run_experiments.sh:34` の E3 行。`results/E3.txt`・`results/SUMMARY.md` は過去の結果なので残す | `git grep -n a3-no-eisu -- .github tools docs .claude ':!tools/e2e/ime_key_matrix/results' ':!docs/adr/240-*'` が 0 件 | なし |
| a7 と構成を消す | `ablations/a7-no-follow.sh`、`e2e-ime.yml:170` の `atok-resync-nofollow`、`observe_grandfathered.txt:7`、`tools/e2e/ime_key_matrix/README.md:62` の a7 の説明、`docs/teardown-verification-guide.md:88` の例(a8 改名後の `bug170-*` に差し替え)。`docs/adr/187-*.md:123` は履歴なので残す | `test_e2e_plan.py`(`test_grandfathered_only_shrinks` を含む)が通る。e2e-ime の plan ジョブが通る | 所有者が修理を選んだら(所有者に聞く 3)、消さずに置換元を直す |
| a8 を BUG 番号で改名 | `a8-no-gji-reopen-sync.sh` → `bug170-no-gji-reopen-sync.sh`(先頭に「直接変える量」の 1 行)、`e2e-ime.yml:430` の参照 | `ci/e2e-scenarios` へ push して `sc-reopen-tsf-gji-gap600-nofix` が従来どおり OK | ビルドキャッシュのキーに `ablations/**` が入る(`e2e-ime.yml:781`)ので、全構成のキャッシュが 1 回外れる。それ以外の影響が出たら改名を戻す |
| `fix-requires-evidence.md` の文言 | D2 の書き分け | docs のみ。`adr-evidence-consistency` が通る | なし |
| `teardown-verification-guide.md:104` | 「`a1`〜`a7`」の古い行(a1・a2 は削除済み、a3・a7 はこの段階で削除) | docs のみ | なし |
| BUG-114 の仕上げ(判断は本 ADR が持ち、ADR-235 D3 は参照するだけ) | PR #534 の判定器を D1a-3(INVALID > FAIL)に直す。閉じる前の observed が 0 のままなら、a9・a10 と `bug114` ジョブは入れず、BUG-114.md に「WT と実 Chrome の CI では drift 補正に届かなかった(run 37429061750・37422196139)」と書く | PR #534 の run の verdict と observed 件数 | 観測が届く手順が見つかれば、D1a を満たす形で入れる(a9・a10 は `bug114-*.sh` に改名) |

行数: 削除は mutator 2 本(約 20 行)、構成 1 行、grandfathered 1 行、`run_experiments.sh` 1 行、README 1 行。追加は `fix-requires-evidence.md` の約 6 行。撤去のほうが多い。

### 第 2 段階(腐敗の検出、D4)

- 追加: Linux ジョブの手順 1 つ(シェル 15 行前後の見込み、未実測)。撤去先は無い(新しい検査)。第 1 段階で孤立した mutator を消し、今後の孤立を構造的に防ぐことと引き換えにする。
- 検証: わざと置換元を変えたブランチで、このジョブが落ちることを 1 回確かめる。
- 取りやめ条件: mutator が 3 本以下のまま増えない(人が目で見れば足りる)。

### 第 3 段階(既存の BUG に広げる。所有者が聞く 1 で B を選んだ場合だけ)

- 候補: 状態欄が「CI 検証済み」の 19 件のうち、CI 構成が既にあり、修正の hunk が develop に半分以上当たるもの(BUG-156・163・172・176 など。個別の確認は未実施)。
- 1 件ずつ D1b の手順で nofix を作る。差が出なければ known-bugs に「CI では修正の有無を区別できなかった」と書いて終える(mutator は残さない)。
- 取りやめ条件: 最初の 3 件で、症状の基準で差が出たものが 0 件ならやめる。既存の実績も 0/3(背景 2)なので、その場合は nofix を増やすより、シナリオの観測を直すほうが先。これは本 ADR の範囲外。

## 却下した案と理由

- **記録の再生だけで実機の不具合の回帰の証拠にする**: 決定の再計算は ADR-225 F1・SP0 0 件。入力の再生(B5)はエンジン側に限る(背景 3)。B5 は D2 で受け皿として残すが、IME 側の代わりにはならない。
- **nofix 対照を義務にする(旧 D1)**: 実績が 0/3(背景 2)。D1b の任意に留める。
- **全 BUG に実機 CI シナリオを義務づける**: 「CI 検証済み」は 19/186 件。残りは CI で再現する手段が無い。義務にすると、(b) で済ませていた BUG の修正が止まる(complexity-budget.md の例外条項と同じ理由)。
- **nofix のためだけに本番コードへ設定・cfg・feature を足す**: 本番に分岐が増え、修正 1 件ごとに残り続ける。**既にある設定**を使うこと(`sc-adr209-chrome-msime-off` の形)は可。こちらはビルドが要らず腐敗もしない。
- **nofix を PR の必須チェックにする**: 1 BUG あたり約 27 ランナー分(背景 5)。V1(#530)を当面必須にしない所有者の決定と同じ扱い。
- **宣言テーブルで「BUG → 構成 → mutator」を生成する**: ADR-218〜220 の却下と同じく、対象が 2 件の段階で表を作る価値が無い。

## リスクと限界

- nofix で差が出ても、そのシナリオが利用者の症状を捕まえるとは限らない。BUG-170 は journal 上の同期でしか差が出なかった。D1b で書き分けるだけで、解決はしない。
- mutator は「修正前のコード」を再現しない。修正後に周辺が変わっているので、nofix は「今のコードから修正の要点だけを外したもの」。修正前の挙動と一致するかは未確認。
- 修正の hunk が当たる割合(背景 4)は粗い目安。hunk が当たる BUG で実際に nofix が意味を持つかは、1 件も確かめていない。
- 実機 CI の揺らぎ(フォーカス喪失で INVALID など)で、N ≥ 3 がそろわない構成がありうる。
- 「前提」と「症状」の分け方は、PR ごとに作者が書く。分け方の誤りはレビューでしか捕まらない。
- PR #534 の最終結果は、この ADR の起草時点で未確定。キャッシュの外れたビルドの所要時間は未確認。

## 所有者に聞くこと

1. **対象の範囲**
   - (A) D1a を規約にし、既存は BUG-170 と BUG-114 だけ(第 1 段階のみ)。
   - (B) A に加え、「CI 検証済み」19 件から 3 件を D1b で試す(第 3 段階、取りやめ条件つき)。
   - (C) A に加え、D1b(nofix)も新しいシナリオには義務にする。
   - 推奨: **A**。理由: D1a(observed 件数・前提と症状・INVALID 優先)は費用が小さく、BUG-114 の 2 版で効いた。D1b は実績が 0/3 で、義務にする根拠が無い。nofix に固有の効果は BUG-170 の 1 件だけ。
2. **nofix の実機 run を CI 必須にするか**
   - (A) 必須にしない。nofix を足す・変える PR で作者が手で 1 回回す。
   - (A') A に加え、リリース前に 1 回回す(`release-develop-to-main` スキルに 1 手順。リリースが 6〜15 分延びる)。
   - (B) 週 1 回の定期実行(`schedule`、既定ブランチ develop で動く)。
   - (C) PR の必須チェック。
   - 推奨: **A**(第 2 段階の当たり確認は全 PR)。理由: 背景 4 の腐敗 (i)(ii) は数秒の当たり確認で捕まる。「実機で症状が出なくなった」型の腐敗は今のところ実例が無い。
3. **a7(nofollow)を消すか直すか**
   - 消す: 5 か所(mutator・構成・grandfathered・README・ガイド)。失うのは、ADR-187 の「follow を無効にしてずれ(各回 4 件)を起こしても、リセット 16/16 成功」(`187-*.md:123`)を再確認する手段。`atok-resync` はずれを起こさないので、回復までは試さない。CI の合否は変わらない(observe)。
   - 直す: 置換元の 1 行(`f1ef8d76` で変わった `ime.arm_mode_key_pass_mark(now);` の周辺)を書き換える。
   - 推奨: **消す**。理由: observe で合否に使っておらず、2026-09-26 に壊れたと分かっても絞り込みで回避されたまま直されなかった。

## 関連する既存文書への追記案(この ADR では書き換えない)

- `.claude/rules/fix-requires-evidence.md:22-26`: 「将来的に……置き換える予定(未実装……)。」を次に置き換える。
  > (b) のうち**エンジン側の不具合**は、将来、入力の再生(journal 検討メモ `docs/tasks/journal-replay-rebuild-study-2026-10-06/` の B5)による再現テストで置き換える(未実装)。**IME とのやり取りが絡む不具合**は再生では答えられないので、実機 CI のシナリオで (a) を満たす。実機 CI のシナリオを (a) と数えるのは、判定基準を前提と症状に分けて最初の run の前に書き、observed 件数を出し、observed 0 の回を INVALID にした場合に限る。修正を外したビルドで症状が出たときだけ「修正の有無を区別する」と書ける([ADR-240](../../docs/adr/240-ablation-ab-as-regression-evidence.md))。
- `docs/teardown-verification-guide.md:81-89`: `mutator` の説明に「nofix(BUG の修正を外す)は `bug<NNN>-*.sh`、機構の撤去実験は `aN-*.sh`」の 1 行を足し、`:88` の例を `bug170-*` に差し替える。`:104` の「`a1`〜`a7`」を現状に直す。
- `docs/adr/235-app-ime-realmachine-matrix.md` D3: 「BUG-114 の判断は ADR-240 第 1 段階に従う」とだけ書く(判断を 240 に寄せる)。今の文言は既にこの形に近い。
- `docs/known-bugs/BUG-114.md`: PR #534 の結論(差が出たか、届かなかったか)を run 番号つきで 1〜3 行。
- `docs/known-bugs/BUG-170.md`: 「nofix(a8)で差が出たのは後から足した `--require-sync`(journal 上の同期)だけで、入力先のテキストでは差が出なかった(run 36654801007、`2f3446fc`)」の 1 行。
