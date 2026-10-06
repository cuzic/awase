---
id: ADR-235
title: |-
  実機 CI の各構成で「入力先の profile と、awase がその経路を通った件数」を summary に出す(情報のみ)。INVALID 化と新しいアプリの常設は、そのデータが出てから条件付きで
summary: |-
  e2e-ime.yml の 416 構成(2026-10-06 develop、plan スクリプトを実行して数えた)では、profile を読む判定器が 19 本中 0 本ある。高速打鍵(typing、129 構成)・chrome_probe(54)・consistency(33)
  などの判定器は、awase がどの profile で動き、どの経路を通ったかを見ない。2026-09-29 の cal-driftrec 初版と 2026-10-06 の BUG-114 実 Chrome 測定(16 回すべて drift 補正 0 件)は、
  「経路に乗っていない」を「起きなかった」と読み違えかけた例。Windows Terminal の BUG-114 相 E も 3 回目(run 37429061750)で、9 ジョブすべてが窓を閉じる前の drift 0 件だった
  (fix・nofix-a10 の 6 回は INVALID、nofix-a9 の 3 回は profile=ImmCross のため FAIL)。
  決定: (1) 既存の check_run_validity.py に、入力先の profile(打鍵前の最後の focus transition)と 4 つの経路の件数を足し、summary に出す(情報のみ、合否は変えない)。
  (2) 経路の正規表現の重複のうち、統合しても件数が変わらないものだけを e2e_common.py に寄せる。正味の行数はほぼ ±0 で、ADR 全体では行が増える(撤去先は D1 の表示を「0 件の行だけ」に絞ること)。
  INVALID 化(path=)は、D1 のデータで「expect=pass で、判定器に下限の仕組みが無いのに 0 件の回がある構成」が見つかったときの将来案に下げた。
  Windows Terminal の常設は、閉じる前の drift>0 の run が 1 回出るまで行わない(ADR-233 の BUG-114 の仕上げに合わせる)。
status: |-
  提案(起草中、Opus レビュー round1 反映済み・round2 前)
related_adr:
  - "ADR-134"
  - "ADR-193"
  - "ADR-205"
  - "ADR-225"
  - "ADR-226"
  - "ADR-227"
  - "ADR-233"
---

# ADR-235: 実機 CI の各構成に「入力先の profile と経路の件数」を出す

## 背景(実測した事実)

### 1. 現在の実機 CI の構成(2026-10-06、origin/develop `1630e55e`)

`e2e-ime.yml` の plan ジョブ(Python)をそのまま実行して構成を数えた
(`ONLY=''`、`EXCLUDE_CAL_WHEN_EMPTY=0` で全件。全件では 1018 ジョブで上限 256 を超えて `SystemExit` するので、`except SystemExit` で拾う)。

```sh
python3 -c "import yaml,os;y=yaml.safe_load(open('.github/workflows/e2e-ime.yml'));s=[x for x in y['jobs']['plan']['steps'] if x.get('id')=='p'][0]['run'];os.environ.update(ONLY='',EXCLUDE_CAL_WHEN_EMPTY='0',RUNS='',GITHUB_OUTPUT='/dev/null');g={}
try: exec(s,g)
except SystemExit: pass
print(len(g['configs']))"
```

- 全 416 構成。expect の内訳は observe 346・pass 67・fail 3。observe のうち期限(until)付きは 33 で、
  313 は `tools/e2e/ime_key_matrix/observe_grandfathered.txt`(315 行、コメント・空行を除くと 313 行)に載っていて期限が無い。
- 既定の実行(`only` 空)は 118 構成・252 ジョブで、上限の 256 に近い。そのうち pass は 55 構成。
- 入力先(アプリ)別:

| 入力先 | 中身 | 想定 profile(ログの `ImePolicyProfile` の値) | 構成数 |
| --- | --- | --- | --- |
| スパイク既定窓(`ime_key_matrix_spike`) | 自前の Win32 EDIT | ImmCross(未確認) | 106 |
| `typing_stress --form=edit/multi` | 自前の EDIT | ImmCross(未確認) | 67 |
| `--form=rich` | RICHEDIT50W | 未確認 | 17 |
| `--form=tsf` | RICHEDIT50W を `Chrome_RenderWidgetHostHWND` という名前でスーパークラス化したもの(ADR-193、`examples/typing_stress/main.rs:377`) | TsfNative | 71 |
| `chrome_probe`(`chrome=`) | 実 Chrome(`sc-bug176-edge-*` の 2 構成だけは実 Edge) | Imm32Unavailable | 90 |
| `--form=chromepage/chromebar` | 実 Chrome | Imm32Unavailable | 52 |
| `--form=bugreport` | 不具合報告窓 | 未確認 | 10 |
| `compartment_notify_probe_diag` | 自前窓(awase なし) | — | 3 |

- Windows Terminal は e2e-ime.yml に 0 構成。扱うのは `wt-probe.yml` だけで、動くのは `ci/wt-probe`・`ci/vocab-windows-terminal` への push と手動起動のとき。所要は約 15 分で、冒頭コメントに「判定は付けない」と書いてある。
  ADR-227 D0-5 で `chrome_probe --wt` の構成(`cal-d0-gji-wt-close-*`)を足したコミット `4888adcf` は、どのリモートブランチにも入っていない(`git branch -r --contains 4888adcf` が空)。
- PR ゲート(`e2e-ime-smoke.yml`)が回すのは `only: atok-passthrough-cold,baseline` の 2 構成だけで、どちらもスパイク既定窓。TsfNative・Imm32Unavailable の窓は PR ゲートで一度も通らない。
- e2e-ime.yml が動くのは `ci/*` への push と dispatch のときだけ(`e2e-ime.yml:36-50`)。develop への push や定期実行の run は無い。
- 擬似 IME の閉ループ(`closed_loop_scenarios.rs`)は実機ではないので、本 ADR の対象外。
- IME は 3 通り: GJI の ATOK プリセット(keymap 1)、GJI の MS-IME プリセット(keymap 2)、MS-IME 本体。本物の ATOK は CI に無い。
- 同時進行中の PR #537(`ci/e2e-java-office`、未マージ)が、`tools/e2e/input_forms/forms.toml` で入力先を 24 種足している
  (Java・Office・WinForms/WPF/WebView2/WinUI 3・Qt〈line.exe 名〉・wxWidgets・Electron・Notepad++・Flutter)。構成名は `tsx-ext-*`(observe、`until='2026-12-01'`)。
  run 37425162897 は e2e ジョブ 136 本(plan・build・summary を含めると 139 本)で 132 PASS / 3 FAIL / 1 INVALID、約 45 分。判定は typing なので、下記のとおり awase.log を読まない。

### 2. 判定器は「awase がどの経路を通ったか」をほとんど見ない

`tools/e2e/ime_key_matrix/check_*.py` 19 本のうち、awase.log を読むのは 8 本だけ:
drift_correction・drift_recovery・invariants・keymatrix・reopen・run_validity・startup と、引数で受ける drift_recovery_chrome。
`profile` を読む判定器は 0 本。

```sh
cd tools/e2e/ime_key_matrix
for f in check_*.py; do echo "$f $(grep -c -E 'awase\.log|awase_lines|load_awase' $f) $(grep -c profile $f)"; done
```

`check_typing_stress.py` は冒頭に「awase 本体の挙動は読まない(入力先のテキストだけで判定)」と書いている。
`send_keys: mode=Unicode` の窓では IME が閉じていても文字が入る(`check_drift_recovery.py` の docstring)。
そのため typing の PASS は、IME の開閉を正しく扱えた証拠にならない。PR #537 の 132 PASS も同じ。

「経路に乗らなかった回を PASS にしない」仕組みは、必要な構成には個別に既にある:

- `check_drift_recovery.py:298`: `observed==0 and drift==0` なら `NOT_OBSERVED`(rc=1)。2026-09-29 の教訓から入った。
- `check_drift_recovery_chrome.py`: 同じ形の `NOT_OBSERVED`。通るのは `check='driftrecovery'` で `chrome` が空でない `sc-driftrecovery-*-chrome` の 4 構成(`e2e-ime.yml:1133-1135`)。
  そのうち `ctrlmuhenkan` の 2 構成は `analyze_ctrl` で判定し、`RESULT PASS` と phys_ctrl だけを見て、observed と drift は判定に使わない(`:9-30`)。
- PR #534 の `wt_pure.judge_bug114`: 「窓を閉じる前の drift 補正 0 件」で INVALID。
- `check_run_validity.py`: 物理キーの混入と CPU 負荷で汚れた回を rc=3 に書き換える。ただしリポジトリ変数 `E2E_VALIDITY_ENFORCE=1` のときだけ。

経路の正規表現は、判定器ごとに書き直されている:

| 断片 | 定義している判定器 | 統合で件数が変わるか |
| --- | --- | --- |
| `\[stage-observe\] observer_poll=Some\|ObserverReported` | check_drift_recovery.py:64、check_drift_recovery_chrome.py:4 | 変わらない(同じ文字列) |
| `\[drift\] correction:\|Blacklist drift correction: apply_ime_open` | check_drift_recovery.py:65、check_drift_recovery_chrome.py:5 | 変わらない |
| `\[drift\] correction`(`Blacklist …` を含まない) | check_startup.py:18 | **変わる**(上へ寄せると Blacklist の行も数える)ので対象外 |
| `\[drift\] correction: .*?set_ime_open\((true\|false)\)`(捕獲グループ付き) | check_invariants.py:48 | 用途が違うので対象外 |
| `send_keys: mode=Unicode` | check_drift_recovery.py:68、check_reopen.py:53 | 変わらない |
| `mods\(c=true .*phys_ctrl=true` | check_drift_recovery.py:75、check_drift_recovery_chrome.py:7、check_keymatrix.py:56 | 変わらない |

### 3. 「経路に乗っていない」を「起きなかった」と読み違えた実例

- **cal-driftrec 初版(2026-09-29)**: 「MS-IME は drift correction が ON へ戻さない(設計どおり)」と書いたが、Opus レビューで測定の産物と分かった。
  TsfNative ではポーリングが止まっていて observed=0 だった。他の 3 構成も、前提がそれぞれ別の理由で成り立っていなかった
  (明示意図が消えていた・GJI reinit という別の経路が開け直していた・Unicode 注入だった)。
  出典: メモリ `feedback_check_observation_path_before_concluding_2026_09_29.md`、`docs/known-bugs/BUG-172.md`。
- **BUG-172 の実 Chrome(run 36524071258・36537797446)**: 実 Chrome は `profile=Imm32Unavailable` なので開閉の観測を捨て、observed=0 になる。
  症状は 9/9 で再現したが、疑ったゲートは通っていなかった。ゲートの修正は見送られた。
- **BUG-114 の実 Chrome(2026-10-06、run 37422196139、PR #529。測定器ごと閉じた)**: GJI・MS-IME × 2 回 × 打鍵の 16 回すべてで drift 補正 0 件。修正なし相当(a9)でも 0 件で、差が出ない。
- **BUG-114 の Windows Terminal(PR #534、OPEN・未マージ)**:
  - 1 回目(run 37424011562): fix 3/3 PASS、nofix-a9 3/3 で `drift_correction_read` が 57/50/54 件。ただし補正は 12 本すべて、終了処理の `WM_CLOSE` の後だった(PR 本文のレビュー M1)。
    当時は「閉じる前の drift 0 件なら INVALID」という規約が無かったので、fix の 3 回は PASS と数えられた。規約が足されたのはこのレビューの後。
  - 2 回目(run 37426810927): きっかけをペイン分割に変えたが、打鍵として扱われ(`SkipTyping`)、閉じる前の補正は 0 件で 9 ジョブとも INVALID。
  - 3 回目(run 37429061750): きっかけを補助窓への前面移動に変えた。9 ジョブすべてで閉じる前の drift は 0 件。
    fix と nofix-a10 の 6 回は `profile=TsfNative` で INVALID。nofix-a9 の 3 回は FAIL で、理由は「起動時の profile=ImmCross」だけ(mutator が書き換えた値を基準 1 が読んだだけ)。
    再確認: `gh run view 37429061750 --log | grep '"type": "bug114_result"'`
  - つまり、閉じる前の行に限ると「Windows Terminal なら drift 補正の経路に届いた」はまだ成り立っていない。2 回目以降は、後から足した INVALID の規約が PASS と取り違えるのを止めている。
    この調査は別のエージェント(ADR-233 の BUG-114 の仕上げ)が続けているので、結果は変わる可能性がある。

### 4. アプリ別の実害の件数(docs/known-bugs 全 183 件)

各ファイルの `title` と本文の `**アプリ:**` 行だけを見て数えた。本文中のそれ以外の言及は、別の列に数えた。

| アプリ | 題名・アプリ行 | 該当 BUG | 本文での言及 | CI の現状 |
| --- | --- | --- | --- | --- |
| Chrome | 16 | 002,007,017,021,027,029,036,066,128,149,168,172,179,182,185,186 | 62 | 定常 142 構成。BUG-149 は再現した(`sc-bug149-chrome-*`)。BUG-117・BUG-171 は再現せず |
| Windows Terminal | 12 | 008,059,061,063,113,114,142,143,173,174,175,188 | 62〜63(数え方で差) | e2e-ime は 0 構成。**CI で再現した WT の症状は BUG-188 の 1 件だけ**(未マージの `cal-d0-gji-wt-close-follow` で 8/8、run 37237143414)。BUG-114 は解決済み(実機確認済み)、BUG-113「@」は wt-probe で 0 件(run 37308263578・37313202771)、BUG-142 は WT で再検証していない |
| Edge | 7 | 007,011,022,117,170,176,182 | 20 | `sc-bug176-edge-*` の 2 構成だけ。BUG-176 は 80 試行で 0 件 |
| awase-settings(egui) | 4 | 071,079,107,112 | 20 | BUG-112 は `bug112-probe.yml` の 60/60 で NULL が 0ms(再現せず) |
| UWP | 3 | 018,055,128 | 38 | `e2e-uwp-inputsite-hook-watchdog-probe.yml`(issue #165 用の一回限り) |
| LINE(Qt) | 2 | 059,060 | 8 | 不具合報告は別に 3 件。`--form=qt` で 36 回試して 0 件。PR #483 はマージせず閉じた |
| Teams | 1 | 106 | 8 | なし |
| WezTerm | 1 | 001 | 20 | なし |
| VS Code | 0 | — | 0 | なし |

キーワード一致による機械的な数え方なので、同じ BUG が複数のアプリに数えられることがある。Opus レビュー round1 が別に数え直し、BUG 番号の並びは一致した。
「再現した/しなかった」の出典は、メモリ `project_ci_bug_repro_campaign_2026_10_04.md`・`project_next_session_qt_line_repro_2026_10_04.md` と各 BUG ファイルの追記。

### 5. 実行時間・runner の制約

- 構成の `wait`(上限時間)の合計は約 120 時間。これは runs を掛けない値で、runs を掛けると約 254 時間。直近の run の所要は 2〜45 分。
- windows-latest で起動できたもの: Chrome・Edge(chrome_probe)、Windows Terminal 1.23(BUG-188・wt-probe の記録)、GJI(chocolatey で約 30 秒)、MS-IME 本体(ja-JP を追加)。
  PR #537 は setup スクリプトで Qt・Electron・Flutter・Java・LibreOffice などを入れて起動できている。
- Teams・LINE の実物はサインインが要るので、CI では使えない(未確認。試みた記録も無い)。代わりになるのは PR #537 の Electron・Qt の入力欄。
- runner の同時実行数の上限と、公開リポジトリでの Windows の分数の扱いは未確認。

## 決定

### D1. 入力先の profile と経路の件数を、既存の check_run_validity.py で数えて summary に出す(情報のみ)

新しいスクリプトは作らない。`check_run_validity.py` は awase を起動する全構成で毎回動き(「run の汚れ」ステップ、条件は `awase != 'false' && probe == ''`)、
既に awase.log を読んで `validity.json` を summary に渡している。足すのは次の 2 つだけ。合否と rc は変えない。

- **入力先の profile**: 打鍵前の最後の `focus transition`(`journal.rs:1037-1054`。`profile=` は構造化フィールドで、target は `awase::journal` の debug)の値。
  補助として `[focus-scope] bootstrap initial scope: … profile=…`(`runtime/focus_tracking.rs:248`)も出す。
  bootstrap は「awase を起動したときに前面にあった窓」の profile なので、入力先の profile とは限らない。chrome_probe の 90 構成は awase を先に起動してから Chrome を起動する(`e2e-ime.yml:1062-1068`)ため、bootstrap はランナー側の窓の値になる。
  出すのはログの `ImePolicyProfile` の値。`AppImeProfile` は別物で、`focus/class_names.rs:410` によると Windows Terminal の CASCADIA クラスは `Imm32Unavailable` に丸められるので、取り違えないよう列名に明記する。
- **経路の件数**: 次の 4 つだけ。増やすときは本 ADR を更新する。
  - `observed`: `[stage-observe] observer_poll=Some` / `ObserverReported`(ImeModel へ開閉を観測した回数)
  - `drift`: `[drift] correction`
  - `unicode`: `send_keys: mode=Unicode`(この回の打鍵結果は IME の状態の証拠にならない)
  - `external_change`: ADR-205 の `[external-change]`(Imm32Unavailable の窓で外部変更に追随した回数、`runtime/ime_refresh.rs:267`)
- summary には、既存の「汚れ」の表と同じ形で出す。4 件数がすべて 0 の回の行だけを先頭に並べ、それ以外は件数だけにする(全構成を毎回並べると、既定の run で 252 行増えるため)。
- `test_log_anchors_in_rust_source.py` の `ANCHORS` に、断片を 2 つ足す。
  - `[stage-observe] observer_poll=`: Rust 側は `"[stage-observe] observer_poll={:?}"`(`runtime/ime_refresh.rs:172`)なので、`=Some` まで含めると字面が一致せず、テストが落ちる。
  - `[external-change]`
  - `[drift] correction` と `send_keys: mode=` は既に `ANCHORS` にある(`:28,34,35`)。
  目的は、Rust 側の文言が変わって件数が黙って 0 になる事故(d47645eb の前例)を、PR の smoke で止めること。

### D2. 経路の正規表現の重複のうち、統合しても件数が変わらないものだけを寄せる

- `e2e_common.py` に observed・drift(`Blacklist` を含む版)・unicode・phys_ctrl の 4 パターンを置く。背景 2 の表で「変わらない」とした判定器はそれを import する。D1 も同じパターンを使う。
- 対象外: `check_startup.py:18`(寄せると `Blacklist drift correction` の行も数えて件数が変わる)と `check_invariants.py:48`(捕獲グループ付きで用途が違う)。
- **行数は撤去にならない**。消えるのは 9 行(chrome 版 3、drift_recovery 版 4、keymatrix 1、reopen 1)で、増えるのは定義約 6 行と import 約 4 行。正味は ±0〜−3 行。
  D2 の目的は行を減らすことではなく、D1 と判定器が同じ断片を数えるようにすること(D1 の件数と判定器の observed が食い違ったときに、断片の違いを原因から外せる)。
- ADR 全体(D1・D2)では、profile の抽出・4 件数・json・summary の表・ANCHORS で、数十行の追加になる。撤去先として、D1 の表示は「0 件の行だけ」に絞る。INVALID 化(旧案 D2)は第 1 段階から外し、workflow の分岐と rc の新しい意味を足さない。

### D3. Windows Terminal は、閉じる前の drift>0 の run が 1 回出るまで常設しない

- ADR-233 の第 1 段階「BUG-114 の仕上げ」(`233-ablation-ab-as-regression-evidence.md` の第 1 段階)は、「閉じる前の observed が 0 のままなら、a9・a10 と `bug114` ジョブを入れず、BUG-114.md に『WT の CI では drift 補正に届かなかった』と書く」としている。本 ADR もこれに従う。
- BUG-114 は解決済み(develop の BUG-114.md の状態欄)。第 1 段階の WT の構成として、解決済みで経路にも届いていないバグを常設する理由は弱い。
- WT で実際に再現しているのは BUG-188(外部から閉じた後の `kiu`、8/8)。ADR-227 の実装を待たずに、これを `observe` の再現構成として置けるかを所有者に聞く(下記 2)。測定器のコミット `4888adcf` はどのリモートブランチにも無いので、置くなら作り直しになる。
- 置くときも e2e-ime.yml には入れず、wt-probe.yml(「別のハーネスで、共有ファイルを避けるため」)に置く。

### 将来案(条件付き): 宣言した経路が 0 件の回を INVALID にする(path=)

第 1 段階では入れない。round1 で、当初の宣言先 18 構成(`check=driftrecovery` 16、`driftcorrection` 2)が次の状態だと分かった。

- すべて `expect=observe` で期限が無く、summary はゲートを落とさない(`e2e-ime.yml:1302-1306`)。書き換えても、どの構成の合否も変わらない。
- 判定器が既に同じ下限(`NOT_OBSERVED`)を持っている。
- 入力先の内訳は TSF 系 10・実 Chrome 4・EDIT 4 で、背景 3 のとおり observed が構造的に 0 になりうる構成が大半。

入れる条件は、D1 のデータで次の構成が 1 つ以上見つかること: `expect=pass` で、判定器に下限の仕組みが無く、経路の件数が 0 の回がある。
入れるときは `check_run_validity.py` の出力行と json の verdict を CONTAMINATED と分け(例: `PATH_ABSENT`)、summary の先頭表示にも足す。rc=3 の意味が「汚れ」と「経路 0 件」の 2 つになることを、その時点の ADR 追記に書く。

### やらないこと

- typing(129 構成)と PR #537 の 24 入力先に、経路の件数を合否として要求しない。D1 の表示は自動で出る(#537 の構成も「run の汚れ」ステップに乗る)。
- 新しいアプリを複数同時に足さない。

## 却下した案と理由

- **判定器を 1 本の共通ジャッジ(宣言表 + 汎用チェッカー)に作り直す**: ADR-218(GuardRule 宣言テーブル)・ADR-219(シナリオ DSL)・ADR-220 と同じ理由で却下済み。
  判定器ごとに、数える時間窓と前提が違う(例: drift_recovery は「閉じてから打鍵の直前まで」)。共通化すると件数が変わる。D2 も、件数が変わる `check_startup.py` を対象から外した。
- **path= を第 1 段階で全構成または drift 系に入れる**: 上の「将来案」のとおり、消費者が 0 で、既存の下限と重なる。
- **awase のログを構造化して、経路の件数を journal に集める**(ADR-226 候補 E): 未レビュー・未決定。本 ADR は既存のテキストログの断片を数えるだけ。
- **新しいアプリを一度に広げる(Teams・LINE・VS Code・Edge)**: Teams・LINE の実物は CI で使えない。Edge は Chrome と同じ profile と見込まれる(未確認)。
- **報告 journal・閉ループ・replay との連携**: ADR-225・226 で見送り済み。

## 段階

| 段階 | 内容 | 撤去・統合 | CI での検証 | 取りやめ条件 |
| --- | --- | --- | --- | --- |
| 1 | D1・D2 | 経路の正規表現の重複のうち件数が変わらないもの(背景 2 の表)。D1 の表示は 0 件の行だけ | `test_check_run_validity.py` に profile と 4 件数の単体テスト、`test_log_anchors_in_rust_source.py` が通る。次の 2 つの run の summary に列が出ること: `only='sc-driftrecovery-*,atok-passthrough-cold,baseline'` の run(判定器の observed と D1 の observed が、observed>0 の回で矛盾しない)と、既定(`only` 空、118 構成)の run | D1 の件数が判定器の observed と食い違い、その原因が数え方の違いにあると分かったとき(D1 の該当列を消す) |
| 2 | D1 のデータで、既定の run に入る pass の 55 構成のうち、4 件数がすべて 0 の回がある構成を洗い出す。将来案(path=)の条件に当たる構成があれば、ADR に追記して入れる | 4 件数がすべて 0 で BUG 番号の無い observe 構成(削除するか「対照」と明記するか) | 既定の run を 2 回(`only` 空)。cal/ts/tsx の pass 12 構成は、別の only で 1 回 | 条件に当たる構成が 0 なら、将来案を「不要」として閉じる |
| 3(条件付き) | WT: 閉じる前の drift>0 の run が出たら BUG-114 相 E を wt-probe に置く。または所有者が承認すれば BUG-188 の observe 構成 | WT を常設するなら、wt-probe の相 V(可否表)を外せるか確かめる | wt-probe の run で、閉じる前の drift>0 | ADR-233 の BUG-114 の仕上げで「届かなかった」と記録されたら、BUG-114 相 E はやめる |

PR #537 とは e2e-ime.yml の `def cfg(...)` の引数が重なるので、#537 を先に入れる。D1・D2 は `cfg()` を変えないので競合しない。
なお #537 は `docs/adr/233-stale-high-observation-beats-newer-in-most-recent-trusted.md` を含んでいて、ADR-233(ablation の草案)と番号が衝突している。本 ADR の範囲外だが、233〜239 の草案を入れる前に解決が要る。

## リスクと限界

- **想定 profile の多くは未確認**: スパイク既定窓・`--form=edit/rich/bugreport` の profile は、ログで確かめていない(D1 がそれを出す)。
- **`focus transition` は debug の journal ログ**: CI の awase.log にこの行が出るログレベルかは未確認。出なければ bootstrap だけになり、chrome_probe 系の profile は分からない(第 1 段階の run で確かめる)。
- **全体の件数は時間窓を見ない**: BUG-114 相 E の 1 回目のように、関係ない時刻に経路へ乗った回(終了処理の後の補正)は区別できない。時間窓が要る構成は、今までどおり判定器側で数える。
- **断片の到達性**: `ANCHORS` は Rust のソースに断片があることを確かめるだけで、その行が実際に出力される経路は確かめない。
- **データが溜まらない**: e2e-ime は `ci/*` への push でしか動かないので、D1 は意図して run しないと増えない。段階 2 の run は手で回す。
- **CI 時間**: D1・D2 は Python で行を数える程度で、実行時間はほぼ増えない見込み(未測定)。
- **BUG-114 WT の調査が進行中**: 背景 3 の 3 回目の結果は起草時点のもの。
- アプリ別の件数はキーワード一致で、人の判断で振り分けたものではない。
- `docs/adr/index.md` の 235 の行は、同時に起草している 7 本の衝突を避けるため team-lead が統合する(本ブランチでは足していない)。

## 所有者に聞くこと

1. **次に優先するアプリ**(段階 2 の後):
   - (a) Windows Terminal(推奨。実害が題名で 12 件あり、e2e-ime の構成は 0。ただし CI で再現したのは BUG-188 の 1 件だけ)
   - (b) Edge(7 件。chrome_probe で回せるが、Chrome と同じ profile の見込み)
   - (c) Qt・Electron(PR #537 に乗る。LINE・Teams の実害は計 3 件と、不具合報告 3 件)
2. **BUG-188 の observe 構成を、ADR-227 の実装を待たずに wt-probe に置くか**:
   - 置く(推奨。WT で実際に再現している唯一の症状で、修正の前後の差を測れる)
   - ADR-227 の実装と一緒に置く
3. **D1 で 4 件数がすべて 0 と分かった observe 構成の扱い**(段階 2):
   - 削除(推奨。期限の無い observe 313 件を縮める方向)
   - 名前を「対照」に変えて残す

WT を定期実行するかは、段階 3 の条件(閉じる前の drift>0)が満たされてから聞く。
なお `schedule` と `workflow_dispatch` は既定ブランチ(main)のワークフロー定義で動くので、定期実行は develop の判定の回帰監視にはならない。

## 関連する既存文書への追記案

- `tools/e2e/ime_key_matrix/README.md`: 判定の読み方に「入力先の profile と経路の件数の列(ADR-235)」の段落を 1 つ足す。
- `.claude/rules/fix-requires-evidence.md` の「テストの置き場所」: 実機 CI で「再現しなかった」と書くときは、その構成の経路の件数(D1)を添える、の 1 行。
- `docs/known-bugs/BUG-114.md`: 相 E の 3 回目(run 37429061750、6 INVALID + 3 FAIL、9 ジョブとも閉じる前の drift 0 件)。ADR-233 の BUG-114 の仕上げ側で追記するなら、そちらに任せる。
