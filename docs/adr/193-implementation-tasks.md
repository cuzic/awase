---
id: ADR-193-companion-193-implementation-tasks
title: |-
  ADR-193 実装タスク一覧（Chrome idle-sweep E2Eの詳細設計と着手順序）
type: companion-doc
related_adr:
  - "ADR-193"
  - "ADR-186"
---

# ADR-193 実装タスク一覧

[ADR-193](193-extend-existing-e2e-harness-for-chromium-coldstart.md)決定4を、実装可能な単位に分割したタスクリスト。
形式は[163-implementation-tasks.md](163-implementation-tasks.md)を踏襲する（内容・変更ファイル・受け入れ基準・依存）。
各タスクは個別のコミットにすること。**未検証の前提には「(未確認)」を付けた**。

## 目的と非目的

- 目的: 実Chrome（GJI・NICOLA ON）で「長いキーボードidle後の最初の打鍵がリテラル化しない」ことを、
  CI（`e2e-ime.yml`）で自動判定できるようにする。BUG-002型（`という→toいう`）の症状を対象にする。
- 非目的: `bあ`（`9a7e699`）・awaseの分類ロジックの変更・tuning定数の変更・Tauri/Electron・
  クラス名偽装ハーネス（ADR-193「検討した代替案」、別ADR）。

## 設計の要点

### 1. 「idle → 打鍵 → 判定」の1試行

`chrome_probe`の既存`ensure(Setup::Kana)`で「IME ON・かな・Engine ON（`k`,`a`がNICOLA文字）」を確認済みの状態から、
ページを空にし、**キー入力なしで`idle`ミリ秒待ち**、`k`,`a`を目印付き`SendInput`で打ち、ページの`textarea`値を読む。

- 判定は**checker（Python）が唯一の判定主体**にする。`chrome_probe`は生の`text=`と`idle=`をログに書くだけで、
  PASS/FAILの解釈を持たない（`classify`相当をRustとPythonに二重化しない。PythonはLinuxで単体テストできる）。
- 既存の`classify()`（`chrome_probe.rs`）は「かなが1文字でもあれば`Nicola`」で、`kあ`（先頭リテラル+残りがかな）を
  `Nicola`扱いするため、部分リテラルを見逃す。**既存`classify()`は変更しない**（既存8ケースの判定が変わるため）。
  checker側の厳格判定を使う。
- checkerの厳格判定（案）: `text`に`[A-Za-z]`が無くかな（U+3040〜U+30FF）が1文字以上 → `PASS`。
  かな+ASCII英字の混在 → `PARTIAL_LITERAL`（BUG-002型）。ASCII英字のみ → `LITERAL`。空 → `EMPTY`。他 → `OTHER`。
  `PARTIAL_LITERAL`/`LITERAL`/`EMPTY`/`OTHER`は`FAIL`、フォーカス喪失・物理キー混入・前提状態失敗は`INVALID`。

### 2. 掃引点

`CHROME_LONG_IDLE_MS`=5s（Chrome VK）、`MEDIUM_IDLE_PROBE_MS`=7s、`LONG_IDLE_MS`=10s（GJI/TSF）の3閾値（`tuning.rs:85,100,148`）を
またぐ点: **3000 / 6000 / 8000 / 11000 / 14000 ms**。ただし`ensure()`自体がキーを打つのでkeyboard idleはそこから測られる。
GJI休眠（~12s）は時間経過でしか作れないため、14000msの点が「12s超」を担う。**GJI休眠が実際に起きているかは未確認**
（検出手段: awaseのdebugログの`[h1-probe] cold=… idle_at_cold=…ms`を試行ごとに照合し、`idle_at_cold`が掃引点と一致するかを見る）。

### 3. CIへの載せ方

`e2e-ime.yml`の`cfg()`に`driver`フィールド（既定`spike`、新規`chrome`）を足し、`driver=chrome`の構成だけ
`chrome_probe.exe`を走らせる。**代替（別ワークフロー`e2e-chrome.yml`）は、既存390行を触らず爆発半径が小さい反面、
GJI導入・言語設定・`config1.db`書き込みの約50行を複製する**。本計画は`driver`方式を採り、既存構成の回帰は
「T3の前後で既存`baseline`構成の結果が変わらないこと」で確認する。

## 判定ゲート

- **G0（T0の後）**: 現行コードで、GJI・Chrome・長idle後に部分リテラルが**出る/出ない**。
  - 出る → T4（撤去実験）へ進む。成功基準は「撤去あり=FAIL / なし=PASS」。
  - 出ない → T4は「per-VK confirmを撤去しても壊れるか」の**確認のみ**に縮小する。壊れないなら
    per-VK confirmの有効性はE2Eで示せないと記録し（`docs/experiments.md`）、本E2Eは
    「idle後にリテラル化しないこと自体の回帰検知」（`expect=pass`）として残す。成功基準はそれのみとなる。
- **G1（T3aの後）**: CI上でChrome+GJI+awaseの既存8ケースが実行でき、結果が`observe`として集計される。
  ここで失敗する場合（Chromeが無い/前面化できない/GJIが効かない）、以降のCI関連タスクは止めてT0の実機結果だけで
  ADRを閉じる選択肢を検討する。

## タスク

### T0（コード変更なし）: 実機でステップ0を実行してG0を判定する

- 内容: Windows実機（GJI、awase debugビルド+`AWASE_TEST_INJECTION=1`）で既存`chrome_probe`を
  `--repeat=3 --settle=500`（対照）と`--settle=3000/6000/8000/11000/14000`で実行し、ログの`PROBE 行動後: … text=…`
  の`text`を目で確認する（**既存`classify()`は部分リテラルを`Nicola`扱いするため`RESULT`行ではなく`text=`を見る**）。
  併せて`--no-awase`の対照も1回取る。
  なお`--settle`は「モードキー押下→打鍵」の間隔であり、「フォーカス・キー無入力のidle」とは厳密には別物
  （モードキー後のIME状態遷移が絡む）。T0は「症状が今も出る可能性があるか」の粗い確認で、確定はT1で行う。
- 実施: clipwireで実機へ（push→Windows側のチェックアウトブランチ確認→ビルド→実行）。
- 成果物: 結果表を`tools/e2e/ime_key_matrix/results/`に置く（既存の置き場）。ADR-193へ結果を追記。
- 受け入れ基準: 掃引点ごとに`text`が記録され、G0の出る/出ないが判断できる。
- 依存: なし。

### T2: checker（Python）を書く

- 変更: `tools/e2e/ime_key_matrix/check_chrome_probe.py`（新規）、`tools/e2e/ime_key_matrix/fixtures/chrome_probe/*.log`（新規）、
  `tools/e2e/ime_key_matrix/test_check_chrome_probe.py`（新規、`python -m unittest`でLinux実行可）。
- 内容: 2モード。
  - `cases`: 既存ログの`SUMMARY PASS=n RECOVER=n FAIL=n INVALID=n`行と`RESULT`行を読む。既存`check_multi.py`と同じ終了コード
    （有効回すべてPASS=0 / FAILあり=1 / 有効回なし=3）。
  - `idle`: `IDLE idle=…ms n=… text="…"`行（T1で出力）を厳格判定（設計1）。INVALID条件は
    `check_multi.py`の`phys_in_awase`（`extra=0x0`の物理キー混入）を**import再利用**し、加えて`focus_lost`・`前面化に失敗`行。
- 受け入れ基準: fixture（PASS/PARTIAL_LITERAL/LITERAL/INVALID各1件、実機ログ由来が理想）で単体テストが通る。
  `chrome_probe`のタイムスタンプ形式（`utc_stamp()`）が`check.to_ms`で読めること（**未確認**）をfixtureで確認する。
- 依存: なし（T0と並行可）。

### T3a: `driver=chrome`をCIに載せ、既存8ケースのsmokeを回す

- 変更: `.github/workflows/e2e-ime.yml`のみ。
  1. `cfg()`に`driver='spike'`を追加。新規構成`ch-smoke`（`driver='chrome'`, `expect='observe'`, `check='chrome-cases'`）。
  2. `build`ジョブで`--example chrome_probe`も`dist/`へコピー（全構成、コンパイル時間の増分は**未確認**）。
  3. **既存の「awase を起動し、スパイク --auto を実行して判定する」ステップのうち、`config.toml`書き換え部分
     （約12行）を共通ステップに切り出す**（`driver`に依存しない。既存構成の挙動を変えない機械的な抽出）。
  4. `driver=chrome`用のステップ: Chromeの有無確認（`Test-Path 'C:\Program Files\Google\Chrome\Application\chrome.exe'`、
     無ければ`choco install googlechrome -y`）→ awase起動（debug、`AWASE_TEST_INJECTION=1`、`RUST_LOG=debug`）→
     数秒待機して`awase.log`に起動完了が出るまで確認 → `chrome_probe.exe --repeat=1 --log=chrome_probe.log`を
     時間制限付きで実行 → `check_chrome_probe.py`（T2）→ `rc.txt`。artifactに`chrome_probe.log`を追加。
  5. `plan`の`ONLY`に、`ci/e2e-chrome`ブランチのとき`ch-*`を選ぶ分岐を追加し、`on.push.branches`にも`ci/e2e-chrome`を足す。
- 注意: `ime_key_matrix_spike`用の`cache.toml`事前投入（`ime_key_matrix_spike.exe`のImm capability）は`chrome_probe`に不要な見込み
  （ChromeクラスはIMM32分類で`IMM32_UNAVAILABLE_CLASSES`に入りIMMプローブされない、`class_names.rs:19-35`）。**要確認**。
- 受け入れ基準: `ci/e2e-chrome`へのpushで`ch-smoke`が3回実行され、artifactに`chrome_probe.log`が残る（結果はFAILでもよい=G1）。
  併せて既存`baseline`構成の結果がT3a前後で変わらない（回帰確認）。
- 依存: T2。

### T1: `chrome_probe`にidle-sweepを足す

- 変更: `crates/awase-windows/examples/chrome_probe.rs`のみ（+ `tools/e2e/ime_key_matrix/README.md`のフラグ表）。
- 内容:
  - フラグ`--idle-sweep=<ms,ms,…>`（既定なし）と`--idle-repeat=N`（既定3）。指定時は既存の8ケースを回さず、
    各`(idle, 反復)`で: `bring_to_front()` → `ensure(Setup::Kana)` → `command("clear")` → `sleep(idle)` →
    `k`,`a`（既存`probe()`と同じ押下: 30ms保持・30ms間隔・350ms待ち）→ `snap`で`text`取得 →
    ログに`IDLE idle={idle}ms n={i} text={text:?} focus_lost={bool}`と、`ensure`失敗時は
    `RESULT INVALID: 前提状態にできなかった`。最後に`=== 全ケース完了 ===`。
  - `probe()`を、テキスト取得までの内部関数と既存の分類付きラッパーに分ける（既存呼び出しの挙動は不変）。
  - **判定（PASS/FAIL）はログに書かない**（設計1）。
- 受け入れ基準:
  - `cargo check --target x86_64-pc-windows-msvc -p awase-windows --examples`が通る（`link.exe`不在のためビルドは不可、CLAUDE.md）。
  - 実機（clipwire）で`--idle-sweep=3000,11000 --idle-repeat=2`が完走し、`IDLE`行が出力される。
  - T2の`idle`モードがその実機ログをパースできる。
- 依存: T2（checkerのidleモードの入力形式を先に固定するため）、T3a（CI smokeでハーネスの土台が動くことを確認してから）。

### T3b: `ch-idle`構成を追加

- 変更: `.github/workflows/e2e-ime.yml`のみ。`cfg('ch-idle', 'observe', driver='chrome', check='chrome-idle',
  args='--idle-sweep=3000,6000,8000,11000,14000 --idle-repeat=3')`。`driver=chrome`のrunステップで`args`を渡す。
- 見積もり: 1ジョブ ≒ Σidle(42s)×repeat(3)=126s + 15試行×(`ensure`≈2s+打鍵1s)≈45s ≈ 3〜4分。matrixの`run:[1,2,3]`で3ジョブ。
  `timeout-minutes: 25`内に収まる見込み（**GJI導入約30秒+`ctfmon`再起動等は別、実測で確認**）。
- 受け入れ基準: `ci/e2e-chrome`で`ch-idle`が3回完走し、`observe`として集計される。
- 依存: T1, T3a。

### T4（G0が「出る」のときのみ）: 撤去スクリプトを追加

- 変更: `tools/e2e/ime_key_matrix/ablations/a8-*.sh`（新規、a5と同形式=Pythonで`assert a in s`して文字列置換）、
  `e2e-ime.yml`の`plan`に`cfg('ch-a8-…', 'fail', mutator='a8-….sh', driver='chrome', check='chrome-idle', …)`。
- 撤去対象の候補（**どれが症状を防いでいるかはT0/T3bの結果で決める。現時点では未特定**）:
  1. `output/vk_send.rs`のcold-start分岐（`prepend_f2_warmup`）を無効化する。
  2. `tsf/warmup/probe_fsm.rs::run_per_vk_confirm`（`:307`付近）が各VKを即confirm扱いにする。
  3. `tsf/warmup/literal_detect_fsm.rs`の`RawTsfLiteralRecovery`（部分リテラル回収）を無効化する。
- 採否基準（撤去1本ごと）: ベースライン（撤去なし）が**有効回すべてPASS**、撤去ありが**有効回のうち少なくとも1回FAIL**。
  3回×掃引点の中で再現が不安定なら、`expect='fail'`の判定（「全PASSではない」）は成立しても実用に足りないため、
  `--idle-repeat`を増やすか候補を替える。
- 受け入れ基準: `summary`ジョブで撤去構成が`OK`（期待FAILで実際にFAIL）、ベースライン構成が`OK`（期待PASS）となる、を
  **別々の2回のCI実行で再現**する。
- 依存: T3b、G0。

### T5: 記録を更新する

- 変更: `docs/known-bugs/BUG-002.md`（T0/T4の結果に基づき「現在の対策」を per-VK confirmに書き直す）、ADR-193のstatus・
  決定4の結果欄、（G0が「出る」かつ新規バグ発見なら）`docs/known-bugs/BUG-NNN.md`を次の連番で新規作成（frontmatter規約に従う）、
  （撤去しても壊れない場合）`docs/experiments.md`に1行。
- 受け入れ基準: known-bugsの「現在の対策」が現行コードに存在する定数・関数だけを指す（`grep`で存在を裏取り）。
- 依存: T0、T4（または「出ない」判定）。

## 着手順序

```
T0 ─────────────────────────────────────┐(G0)
T2 ──► T3a ──(G1)──► T1 ──► T3b ──► [T4 は G0「出る」時のみ] ──► T5
```

T0とT2は並行可。T3aはT1より先（ハーネスがCI上で動くかの確認を、新機能の実装より先に行う）。
実装ブランチは`develop`から専用worktreeで切る（`worktree-per-session`）。ADR・本計画のdevelopへの反映を先に行う。
CI実行は`ci/e2e-chrome`ブランチへのpush（`ci/e2e-scenarios`と同じ運用）。

## 規約との関係

- 変更対象は`examples/`・`tools/`・`.github/workflows/`・docsのみで、`fix-requires-evidence.md`の再発ファミリー対象ファイル
  （`src/`配下のwarmup/focus/belief等）は**変更しない**。撤去スクリプトはCI実行時に作業ツリーへ差分を当てるだけ。
- `tuning.rs`の定数・`RESTRICTED_CALLS`許可リストは触らない（`complexity-budget`対象外）。
- checkerはfixtureを持つPython単体テストで担保する（`.claude/rules/fix-requires-evidence.md`の「回帰テスト」の趣旨。
  E2E自体は実機依存でLinuxでは走らない）。

## リスク

| # | リスク | 対策 |
|---|---|---|
| R1 | `chrome_probe`はCIで一度も実行されておらず、Chromeの前面化・初回起動ダイアログ・ループバックHTTPがランナーで動くか不明 | T3a（既存8ケースのsmoke）を先行し、G1で判断する |
| R2 | GJI休眠（~12s）が実際に起きているか不明 | `idle_at_cold`をawaseログで照合（設計2）。取れなければ掃引点を延ばす |
| R3 | 実IMEのflake（1回の失敗で判定が揺れる） | 撤去実験の採否基準を「ベースライン全PASS／撤去あり少なくとも1回FAIL」とし、`observe`から始めて安定を確認してから`pass`に昇格 |
| R4 | e2e-ime.ymlの共通ステップ切り出しで既存構成が壊れる | 機械的抽出に限定し、T3a前後で`baseline`の結果を比較 |
| R5 | ステップ0（`--settle`）は「モードキー後の間隔」でありキー無入力idleとは別物 | T0は粗い確認に留め、確定はT1のidle-sweepで行う |
| R6 | `k`,`a`のNICOLA出力が`ensure()`のプローブと同じ文字になる保証（配列・設定依存） | checkerは特定のかなを期待せず「英字混入の有無」で判定する |
