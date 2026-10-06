---
id: ADR-235
title: |-
  アプリ × IME の実機 CI マトリクスを広げる前に、各構成の判定に「awase がその経路を通った件数」を必須で出し、0 件なら PASS でなく INVALID にする
summary: |-
  e2e-ime.yml の 416 構成(2026-10-06 develop、plan スクリプトを実行して数えた)のうち、判定器が awase.log を読むのは一部だけで、高速打鍵(typing、129 構成)・chrome_probe(54)・consistency(33)などの判定器は
  awase がどの profile で動き、どの経路を通ったかを見ない。2026-09-29 の cal-driftrec 初版と 2026-10-06 の BUG-114 実 Chrome 測定(16 回すべて drift 補正 0 件)は、
  「経路に乗っていない」を「起きなかった」と読み違えかけた例。さらに Windows Terminal の BUG-114 相 E も、補正を閉じる前に限ると 9/9 が drift 0 件で INVALID になった(run 37429061750)。
  決定: (1) 全構成の awase.log から、起動時 profile とあらかじめ決めた少数の経路の件数を、既存の check_run_validity.py に足して数え summary に出す(情報のみ)。
  (2) cfg() に path= を足し、宣言した構成だけ件数 0 の回を rc=3(INVALID)に書き換える。(3) 判定器ごとに重複している経路の正規表現(observed・drift・unicode・phys_ctrl)を e2e_common.py の 1 か所へ移す(撤去)。
  (4) 新しいアプリは、実害の記録が多く CI の定常構成が 0 の Windows Terminal を 1 つだけ。既存の wt-probe.yml(PR #534 の bug114 ジョブ)を使い、e2e-ime.yml には入れない。
status: |-
  提案(起草中、Opus レビュー前)
related_adr:
  - "ADR-134"
  - "ADR-193"
  - "ADR-205"
  - "ADR-225"
  - "ADR-226"
  - "ADR-227"
---

# ADR-235: アプリ × IME の実機 CI マトリクスと「経路に乗った件数」の必須化

## 背景(実測した事実)

### 1. 現在の実機 CI の構成(2026-10-06、origin/develop `1630e55e`)

`e2e-ime.yml` の plan ジョブ(Python)をそのまま実行して構成を数えた
(`ONLY=''`、`EXCLUDE_CAL_WHEN_EMPTY=0` で全件)。

```sh
# 再確認: plan の run: を取り出して exec し、configs を数える
python3 -c "import yaml,os,io,contextlib;y=yaml.safe_load(open('.github/workflows/e2e-ime.yml'));s=[x for x in y['jobs']['plan']['steps'] if x.get('id')=='p'][0]['run'];os.environ.update(ONLY='',EXCLUDE_CAL_WHEN_EMPTY='0',RUNS='',GITHUB_OUTPUT='/dev/null');g={};exec(s,g);print(len(g['configs']))"
```

- 全 416 構成。expect の内訳は observe 346・pass 67・fail 3。observe のうち期限(until)付きは 33 で、
  313 は `tools/e2e/ime_key_matrix/observe_grandfathered.txt`(313 行)で期限なしのまま。
- 入力先(アプリ)別:

| 入力先 | 中身 | 想定 profile | 構成数 |
| --- | --- | --- | --- |
| スパイク既定窓(`ime_key_matrix_spike`) | 自前の Win32 EDIT | ImmCross(未確認、下記) | 106 |
| `typing_stress --form=edit/multi` | 自前の EDIT | ImmCross(未確認) | 67 |
| `--form=rich` | RICHEDIT50W | 未確認 | 17 |
| `--form=tsf` | RICHEDIT50W を `Chrome_RenderWidgetHostHWND` 名でスーパークラス化(ADR-193、`examples/typing_stress/main.rs:377`) | TsfNative | 71 |
| `chrome_probe`(`chrome=`) | 実 Chrome(`sc-bug176-edge-*` の 2 構成だけ実 Edge) | Imm32Unavailable | 90 |
| `--form=chromepage/chromebar` | 実 Chrome | Imm32Unavailable | 52 |
| `--form=bugreport` | 不具合報告窓 | 未確認 | 10 |
| `compartment_notify_probe_diag` | 自前窓(awase なし) | — | 3 |

- Windows Terminal は e2e-ime.yml に 0 構成。`wt-probe.yml`(`ci/wt-probe`・`ci/vocab-windows-terminal` への push と手動起動だけ、
  約 15 分、「判定は付けない」と冒頭コメントに明記)だけが扱う。ADR-227 D0-5 で `chrome_probe --wt` の構成
  (`cal-d0-gji-wt-close-*`)を足したコミット `4888adcf` は、どのリモートブランチにも入っていない(`git branch -r --contains 4888adcf` が空)。
- PR ゲート(`e2e-ime-smoke.yml`)は `only: atok-passthrough-cold,baseline` の 2 構成だけで、どちらもスパイク既定窓。TsfNative・Imm32Unavailable の窓は PR ゲートで一度も通らない。
- 擬似 IME の閉ループ(`crates/awase-windows/tests/closed_loop_scenarios.rs`、`closed-loop-ignored.yml`)は実機ではないので本 ADR の対象外。
- IME は GJI(ATOK プリセット=keymap 1、MS-IME プリセット=keymap 2)と MS-IME 本体の 3 通り。本物の ATOK は CI に無い。
- 他のワークフロー(`bug103-verify`・`preedit-verify`・`vocab-*-verify`・`env-events-probe`・`bug112-probe`・`e2e-uwp-inputsite-hook-watchdog-probe` ほか)は、
  どれも `ci/…` ブランチへの push でだけ動く一回限りの検証で、構成の一覧を持たない。
- 同時進行中の PR #537(`ci/e2e-java-office`、未マージ)が、`tools/e2e/input_forms/forms.toml` で入力先を 24 種
  (Java Swing/AWT/JavaFX/JBR、LibreOffice/OpenOffice、WinForms/WPF/WebView2/WinUI 3、Qt〈line.exe 名〉、wxWidgets、Electron、Notepad++、Flutter)足している。
  run 37425162897 は 136 ジョブで 132 PASS / 3 FAIL / 1 INVALID、約 45 分。判定は typing(下記のとおり awase.log を読まない)。

### 2. 判定器が「awase がどの経路を通ったか」を見ていない

`tools/e2e/ime_key_matrix/check_*.py` 19 本のうち、awase.log を読むのは 8 本だけ
(drift_correction・drift_recovery・invariants・keymatrix・reopen・run_validity・startup と、引数で受ける drift_recovery_chrome)。
`profile` を読む判定器は 0 本。

```sh
cd tools/e2e/ime_key_matrix
for f in check_*.py; do echo "$f $(grep -c -E 'awase\.log|awase_lines|load_awase' $f) $(grep -c profile $f)"; done
```

awase を起動する構成の判定方式の内訳(上の plan 実行で数えた): typing 105・chromeprobe 48・commitkm 28・consistency 23・driftrecovery 16・preedit 14・
keymatrix 20 ほか。`check_typing_stress.py` は冒頭で「awase 本体の挙動は読まない(入力先のテキストだけで判定)」と書いている。
`send_keys: mode=Unicode` の窓では IME が閉じていても文字が入る(`check_drift_recovery.py` の docstring)ので、
typing の PASS は「IME の開閉の扱いが正しかった」証拠にならない。PR #537 の 132 PASS も同じ。

一方、「経路に乗らなかった回を INVALID にする」仕組みは個別には既にある:

- `check_drift_recovery.py`: `NOT_OBSERVED`(observed=0 かつ drift=0)を `NOT_RECOVERED` と分ける(2026-09-29 の教訓から)。
- `check_drift_recovery_chrome.py:31`: `observed==0 and drift==0` で `NOT_OBSERVED`。
- PR #534 の `wt_pure.judge_bug114`: 「窓を閉じる前の drift 補正 0 件」を INVALID。
- `check_run_validity.py`: 物理キー混入・CPU 負荷で汚れた回を rc=3 に書き換える(リポジトリ変数 `E2E_VALIDITY_ENFORCE=1` のときだけ。`e2e-ime.yml` の「run の汚れ」ステップ)。

経路の正規表現は判定器ごとに書き直されている(重複):

| 断片 | 定義している判定器 |
| --- | --- |
| `\[stage-observe\] observer_poll=Some\|ObserverReported` | check_drift_recovery.py:64、check_drift_recovery_chrome.py:3 |
| `\[drift\] correction`(書式違い 3 種) | check_drift_recovery.py:65、check_drift_recovery_chrome.py:4、check_startup.py:18、check_invariants.py:48 |
| `send_keys: mode=Unicode` | check_drift_recovery.py:68、check_reopen.py:53 |
| `mods\(c=true .*phys_ctrl=true` | check_drift_recovery.py:75、check_drift_recovery_chrome.py:6、check_keymatrix.py:56 |

```sh
grep -n -E 're\.compile' tools/e2e/ime_key_matrix/check_*.py | grep -E 'observer_poll|drift\\\] correction|mode=Unicode|phys_ctrl'
```

### 3. 「経路に乗っていない」を「起きなかった」と読み違えた実例

- **cal-driftrec 初版(2026-09-29)**: 「MS-IME は drift correction が ON へ戻さない(設計どおり)」と書いたが、Opus レビューで測定の産物と判明。
  TsfNative ではポーリングが止まり observed=0 だった。他の 3 構成も、明示意図が消えていた・別経路(GJI reinit)が開け直していた・Unicode 注入だった、
  とそれぞれ別の理由で前提が成り立っていなかった(メモリ `feedback_check_observation_path_before_concluding_2026_09_29.md`、`docs/known-bugs/BUG-172.md`)。
- **BUG-172 実 Chrome(run 36524071258・36537797446)**: 実 Chrome は `profile=Imm32Unavailable` で開閉の観測を捨て、observed=0。
  「症状は再現(9/9)したが、疑ったゲートは通っていない」(`docs/known-bugs/BUG-172.md`)。ゲート修正は見送りになった。
- **BUG-114 実 Chrome(2026-10-06、run 37422196139、PR #529)**: GJI・MS-IME × 2 回 × 打鍵で 16 回すべて drift 補正 0 件。修正なし相当(a9)でも 0 件で差が出ない
  (PR #534 ブランチの `docs/known-bugs/BUG-114.md` 222 行目)。
- **BUG-114 Windows Terminal(PR #534、未マージ)**:
  - 1 回目(run 37424011562): fix 3/3 PASS(drift 5 → gave up 1)、nofix-a9 3/3 で `drift_correction_read` 57/50/54 件。差は出たが、補正は 12 本すべて終了処理の `WM_CLOSE` の後だった(PR 本文のレビュー M1)。
  - 2 回目(run 37426810927): ペイン分割をきっかけにしたが打鍵扱い(`SkipTyping`)で、閉じる前の補正 0 件=INVALID。
  - 3 回目(run 37429061750、本 ADR 起草時に確認): 補助窓への前面移動に変えたが、fix・nofix-a10 は `profile=TsfNative`・drift 0・INVALID、
    nofix-a9 は `profile=ImmCross`・drift 0(9 ジョブすべて「窓を閉じる前の drift 補正 0 件(観測経路に乗っていない)」)。
  - つまり **「Windows Terminal なら drift 補正の経路に届いた」は、閉じる前の行に限ると現時点では成り立っていない**。届いた証拠は終了処理の後の 1 回目だけ。
    INVALID の規約がこれを PASS と取り違えずに止めた、という意味で本 ADR の決定の実例でもある(この調査は別のエージェントが継続中で、結果が変わる可能性がある)。

### 4. アプリ別の実害の件数(docs/known-bugs 全 183 件)

各ファイルの `title` と本文の `**アプリ:**` 行だけを見て数えた(本文中の言及は「ほかに言及あり」として別に数えた)。

```sh
cd docs/known-bugs && ls BUG-*.md | wc -l    # 183
grep -l -E 'Windows Terminal|WindowsTerminal' BUG-*.md | wc -l
```

| アプリ | 題名・アプリ行 | 該当 BUG | 本文での言及 | CI の現状 |
| --- | --- | --- | --- | --- |
| Chrome | 16 | 002,007,017,021,027,029,036,066,128,149,168,172,179,182,185,186 | 62 | 定常 142 構成。BUG-149 は再現(`sc-bug149-chrome-*`)。BUG-117・BUG-171 は再現せず |
| Windows Terminal | 12 | 008,059,061,063,113,114,142,143,173,174,175,188 | 63 | e2e-ime は 0 構成。wt-probe で BUG-113「@」は 0 件(run 37308263578・37313202771)、BUG-188 は未マージの `cal-d0-gji-wt-close-follow` で 8/8 再現(run 37237143414)、BUG-142 は WT での再検証が未実施 |
| Edge | 7 | 007,011,022,117,170,176,182 | 20 | `sc-bug176-edge-*` 2 構成のみ。BUG-176 は 80 試行で 0 |
| awase-settings(egui) | 4 | 071,079,107,112 | 20 | BUG-112 は `bug112-probe.yml` で 60/60 NULL 0ms(再現せず) |
| UWP | 3 | 018,055,128 | 38 | `e2e-uwp-inputsite-hook-watchdog-probe.yml`(issue #165 用、一回限り) |
| LINE(Qt) | 2 | 059,060 | 8 | 不具合報告は別に 3 件(01M43NK5…ほか)。`--form=qt` で 36 回 0 件、PR #483 は未マージで閉じた |
| Teams | 1 | 106 | 8 | なし |
| WezTerm | 1 | 001 | 20 | なし |
| VS Code | 0 | — | 0 | なし |

「再現した/しなかった」はメモリ `project_ci_bug_repro_campaign_2026_10_04.md`・`project_next_session_qt_line_repro_2026_10_04.md` と各 BUG ファイルの追記から。
件数はキーワード一致の機械的な数え方で、同じ BUG が複数アプリに数えられることがある。

### 5. 実行時間・runner の制約

- `e2e-ime.yml` の構成の上限時間(`wait` の合計)は 416 構成で約 120 時間。既定(`only` 空)は cal-*・ts-*・tsx-* を除く。直近の run は 2〜45 分。
- windows-latest に入っていて CI で起動できたもの: Chrome・Edge(chrome_probe)、Windows Terminal 1.23(BUG-188・wt-probe の記録)、GJI(chocolatey、約 30 秒)、MS-IME 本体(ja-JP を追加)。
  PR #537 が setup スクリプトで Qt・Electron・Flutter・Java・LibreOffice などを入れて起動できている(run 37425162897)。
- Teams・LINE の実物は、サインインが要るので CI では使えない(未確認だが、試みた記録は無い)。代わりは PR #537 の Electron・Qt の入力欄。
- runner の同時実行数の上限・公開リポジトリの Windows 分の扱いは未確認。

## 決定

### D1. 全構成で、起動時 profile と「経路の件数」を数えて summary に出す(情報のみ)

既存の `check_run_validity.py` に足す(新しいスクリプト・新しい基盤は作らない)。このスクリプトは awase を起動する全構成で毎回動き、
awase.log を既に読み、`validity.json` を summary に渡している。足すもの:

- `profile`: `[focus-scope] bootstrap initial scope: … profile=…`(`runtime/focus_tracking.rs:248`)の値と、以後の `focus transition` で入った profile の件数。
- `paths`: 次の 4 つの件数だけ(増やすときは本 ADR を更新する)。
  - `observed`: `[stage-observe] observer_poll=Some` / `ObserverReported`(ImeModel へ開閉を観測した)
  - `drift`: `[drift] correction`
  - `unicode`: `send_keys: mode=Unicode`(この回の打鍵結果は IME 状態の証拠にならない)
  - `external_change`: ADR-205 の `[external-change]`(Imm32Unavailable での外部変更の追随)
- summary の「run の汚れ」表の隣に、構成ごとの profile と 4 件数の列を出す。この段階では合否を変えない。
- 4 つの断片を `test_log_anchors_in_rust_source.py` の `ANCHORS` に足す。Rust 側の文言が変わって黙って 0 件になる(d47645eb の前例)のを PR の smoke で止める。

### D2. 構成が宣言した経路が 0 件の回は INVALID(rc=3)にする

- `cfg()` に `path=''` を 1 つ足す(値は D1 の 4 つのどれか、または空)。宣言表・DSL は作らない。
- 「run の汚れ」ステップで `check_run_validity.py --require-path <path>` を渡し、件数 0 なら rc.txt を `rc=3` に書き換える。
  `E2E_VALIDITY_ENFORCE` とは独立に、`path` を宣言した構成では常に行う(宣言した構成にだけ効くので、既存構成の結果は変わらない)。
- summary の集計は既に `rc=3` を INVALID に数え、`expect=pass` で有効回 0 なら「判定不能」としてゲートを落とす。新しい集計は要らない。
- 第 1 段階で `path` を宣言するのは、D4 の Windows Terminal の構成と、判定器が既に observed/drift を数えている 18 構成
  (`check=driftrecovery` 16、`driftcorrection` 2)だけ。後者は判定器自身の `NOT_OBSERVED` を残す
  (判定器は時間窓で数えていて、D1 の全体件数より細かいため)。chromeprobe で `check_drift_recovery_chrome.py` を通る `cal-driftrec-chrome-*` 4 構成は、
  実 Chrome が Imm32Unavailable で observed が構造的に 0 になりうる(BUG-172)ので、`external_change` を宣言するか第 2 段階で決める。

### D3. 経路の正規表現を 1 か所に集める(撤去)

`e2e_common.py` に `PATH_PATTERNS`(observed・drift・unicode・phys_ctrl)を置き、背景 2 の表の判定器はそれを import する。
`check_drift_recovery_chrome.py` の `OBS`・`DRIFT`・`PHYS` と、`check_drift_recovery.py`・`check_keymatrix.py`・`check_reopen.py` の同じ定義を消す。
D1 で増える行より消える行が多いことを PR で示す(数えて本文に書く)。`check_invariants.py:48` の `DRIFT_RE` は捕獲グループ付きで用途が違うので残す。

### D4. 新しいアプリは Windows Terminal だけ(既存の wt-probe.yml を使う)

- 理由: 題名ベースの実害が Chrome(16)に次いで多い 12 件で、Chrome は既に定常 142 構成あるのに Windows Terminal は 0。profile も TsfNative の実アプリで、
  ADR-193 のスーパークラス窓(自前の RichEdit)とは違う(BUG-114 の起動時 profile の焼き付きは実 WT でしか出ない)。
- 入れ先: e2e-ime.yml には入れない(wt-probe.yml 冒頭の「別のハーネスで、共有ファイルを避けるため」を維持)。PR #534 の `bug114` ジョブとその判定
  (`judge_bug114`、drift 0 件で INVALID を既に持つ)を足場にする。wt-probe の判定も D1 と同じ 4 件数を出す(`wt_pure.py` の数え方を `e2e_common.PATH_PATTERNS` に寄せられるかは第 1 段階で確かめる。
  ディレクトリが違うので import できなければ寄せない)。
- 第 1 段階の WT の構成は、BUG-114 相 E(PR #534)1 つ。BUG-188(外部クローズ後の `kiu`)は ADR-227 の実装待ちなので第 2 段階。
- 撤去: wt-probe の `probe` ジョブの相 V(wt.exe・pwsh の有無などの可否表)は、可否が分かった今は毎回要らない。相 E を常設するときに V を外す
  (V の中身を確かめてから。外せなければそう書く)。

### やらないこと

- typing(129 構成)と PR #537 の 24 入力先に `path` を一律で要求しない。typing の目的は「文字が崩れないか」で、経路は構成ごとに違う。D1 の件数を見て、第 2 段階で個別に決める。
- 新しいアプリを複数同時に足さない。

## 却下した案と理由

- **判定器を 1 本の共通ジャッジ(宣言表 + 汎用チェッカー)に作り直す**: ADR-218(GuardRule 宣言テーブル)・ADR-219(シナリオ DSL)・ADR-220 と同じ理由で却下済み。
  判定器ごとに数える時間窓・前提が違い(例: drift_recovery は「閉じてから打鍵直前まで」)、共通化すると件数が変わる。本 ADR は件数の数え方を足すだけで、判定ロジックは触らない。
- **awase のログを構造化して経路の件数を journal に集める**(ADR-226 候補 E): 未レビュー・未決定。本 ADR は既存のテキストログの断片を数えるだけで、構造化を前提にしない。
- **新しいアプリを一度に広げる(Teams・LINE・VS Code・Edge を全部)**: 実物は Teams・LINE が CI で使えず、Edge は Chrome と同じ profile と見込まれる(未確認)。
  件数 0 の構成を増やしても INVALID が増えるだけで情報にならない。
- **全構成で path を必須にする**: 既存 346 の observe 構成の多くは目的の経路が記録されておらず、一律にすると何を数えるかを 416 件ぶん後付けで決めることになる。D1 の実データを見てから決める。
- **報告 journal・閉ループ・replay との連携**: ADR-225・226 で見送り済み。

## 段階

| 段階 | 内容 | 撤去・統合 | CI での検証 | 取りやめ条件 |
| --- | --- | --- | --- | --- |
| 1 | D1・D2・D3、WT は PR #534 の相 E を合否付きで回す | 経路の正規表現の重複(背景 2 の表)。WT 常設時に相 V | `test_e2e_plan.py`・`test_check_run_validity.py` に path 0 件→rc=3 の境界テスト。`ci/e2e-ime` の run で summary に profile と 4 件数の列が出る。driftrecovery 系 16 構成の既存の NOT_OBSERVED と D1 の observed が矛盾しない(判定器が observed>0 の回で全体件数も >0) | D1 の件数が既存の判定器の observed と食い違い、原因が断片の取り違えでなく数え方の違いと分かったとき(D2 を止め D1 の表示だけ残す)。WT 相 E が 3 run 連続で全 INVALID のまま、経路に乗せる手順が見つからないとき(WT の常設をやめ、件数 0 を BUG-114 に記録して閉じる) |
| 2 | D1 の 2 run 分のデータで、`expect=pass` の 67 構成それぞれに `path` を付けるか決める。全回 0 件の構成は「経路外の対照」と名前・コメントを直すか削除する。WT に BUG-188 の構成(ADR-227 実装後) | 全回 0 件で BUG 番号の無い observe 構成(件数は D1 の結果次第。目安として typing の重複入力先) | 付けた `path` の構成が 2 run で INVALID にならない | `path` を付けられる pass 構成が 5 未満なら、規約を「drift/observe 系だけ」に縮めて終える |
| 3 | 次のアプリ: UWP(題名 3 件、issue #165)か、PR #537 の Qt・Electron の入力欄に D1 の件数を出す | 同じ入力先を重ねる構成 | 追加した構成の profile が期待どおり(Qt は Imm32Unavailable、line.exe 名) | 実害の新規報告が無く、件数 0 が続くとき |

## リスクと限界

- **想定 profile の多くは未確認**: スパイク既定窓・`--form=edit/rich/bugreport` の profile はログで確かめていない(D1 がまさにそれを出す)。
- **全体件数は時間窓を見ない**: 終了処理の後の補正(BUG-114 相 E 1 回目の M1)のような「関係ない時刻に乗った」回は D1 の件数では区別できない。
  時間窓が要る構成は、今までどおり判定器側で数える(D2 は「0 件なら INVALID」の下限だけを保証する)。
- **断片の文言変更**: `ANCHORS` で Rust ソースに断片があることは確かめるが、その行が実際に出る経路(到達性)は確かめない(`test_log_anchors_in_rust_source.py` の docstring の限界と同じ)。
- **CI 時間**: D1・D2・D3 は Python の行数え程度で、実行時間はほぼ増えない(未測定)。WT 相 E は 9 ジョブで約 6 分(run 37429061750 の 07:20〜07:26)。
- **BUG-114 WT の調査が進行中**: 背景 3 の 3 回目の結果は起草時点のもの。経路に乗せる手順が見つかれば D4 の前提は強まり、見つからなければ D4 の取りやめ条件に当たる。
- アプリ別の件数はキーワード一致で、人の判断で振り分けたものではない。

## 所有者に聞くこと

1. **優先するアプリ**: (a) Windows Terminal(推奨。実害 12 件で定常構成 0、TsfNative の実アプリ) / (b) Edge(7 件、chrome_probe で回せるが Chrome と profile が同じ見込み) /
   (c) Qt・Electron(PR #537 に乗る。LINE・Teams の実害は計 3 件+報告 3 件)。
2. **CI 時間の許容**: WT 相 E(約 6 分・9 ジョブ)を (a) develop への push ごと / (b) 週 1 回の定期実行(推奨。BUG-114 は修正済みで回帰監視が目的のため) / (c) `ci/…` push のときだけ(現状のまま)。
3. **D2 の書き換えを常に行うか**: `path` を宣言した構成だけ常に INVALID にする(推奨。宣言した構成にしか効かない) / `E2E_VALIDITY_ENFORCE` と同じくリポジトリ変数で有効にする。
4. **全回 0 件の observe 構成の扱い**(第 2 段階): 削除(推奨。313 件の期限なし observe を縮める方向) / 名前を「対照」に変えて残す。

## 関連する既存文書への追記案

- `.github/workflows/e2e-ime.yml` の `cfg()` のコメント(`# expect: …` の並び): `# path: この構成が通るはずの awase の経路(ADR-235)。宣言すると件数 0 の回は INVALID`。
- `tools/e2e/ime_key_matrix/README.md`: 判定の読み方に「profile と経路の件数の列(ADR-235)」の 1 段落。
- `.claude/rules/fix-requires-evidence.md` の「テストの置き場所」: 実機 CI で「再現しなかった」と書くときは、その構成の経路の件数(D1)を添える、の 1 行。
- `docs/known-bugs/BUG-114.md`: 相 E 3 回目(run 37429061750)の結果(9/9 で閉じる前の drift 0 件)。PR #534 側で追記されるならそちらに任せる。
