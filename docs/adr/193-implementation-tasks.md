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

**改訂履歴**: 初版をopus-adversarial-consult（計画round1）でレビューした結果、Blocker1件・Major4件・Minor11件が
見つかり、全指摘を実コードで裏取りして反映した改訂版（v2）。主な訂正は次の通り。
- **PB1**: ビルドキャッシュの`hashFiles`にworkflow自身が入っておらず、T3aは`chrome_probe.exe`が無いまま走る。
- **PM1**: `ColdKind::classify`は`CHROME_LONG_IDLE_MS`(5s)ではなく7s/10sのみを見る（`tuning.rs`のdocがstale）。
- **PM2**: 測るのはkeyboard idleではなく`gji_idle_ms`。GJI休眠の有無は`idle_at_cold`で直接測れる。
- **PM3**: 45試行がrcのANDに潰れ、flakeと撤去効果を分離できない → 試行単位のTALLYと率で判定。
- **PM4**: `chrome_probe`に既存ワークフローが必要とするbelief合わせ（`--activate-gji`相当）が無い。

## 目的と非目的

- 目的: 実Chrome（GJI・NICOLA ON）で「GJIが長くidleした後の最初の打鍵がリテラル化しない」ことを、
  CI（`e2e-ime.yml`）で自動判定できるようにする。BUG-002型（`という→toいう`）の症状を対象にする。
- 非目的: `bあ`（`9a7e699`）・awaseの分類ロジックの変更・tuning定数の変更・Tauri/Electron・
  クラス名偽装ハーネス（ADR-193「検討した代替案」、別ADR）。

## 設計の要点

### 1. 「idle → 打鍵 → 判定」の1試行

`chrome_probe`の既存`ensure(Setup::Kana)`で「IME ON・かな・Engine ON」を確認済みの状態から、
**キー入力なしで`idle`ミリ秒待ち**、`k`,`a`を目印付き`SendInput`で打ち、ページの`textarea`値を読む。
`ensure()`の最後は必ず`probe()`を通り、`probe()`が末尾でページを`clear`するため、idle前に別途`clear`は要らない。

- **判定は checker（Python）が唯一の判定主体**。`chrome_probe`は生の値をログに書くだけ
  （`classify`相当をRustとPythonに二重化しない。PythonはLinuxで単体テストできる）。
- 既存の`classify()`（`chrome_probe.rs`）は「かなが1文字でもあれば`Nicola`」で`kあ`を`Nicola`扱いするため、
  部分リテラルを見逃す。**既存`classify()`は変更しない**（既存8ケースの判定が変わる）。
- `IDLE`行の書式（案）: `IDLE idle={ms}ms n={i} utc={UTC} text={text:?} process={bool} focus_lost={bool}`。
  `process`は`probe()`が既に計算している`Process(229)`（Chromeがキーを IME に処理させたか、`chrome_probe.rs:343-346`）。
  `utc`は`gji_idle_ms`の突き合わせ（設計2）に使う。
- checkerの判定（案）: `text`が空 → `EMPTY`（FAIL）。**ASCII英字`[A-Za-z]`を含まない** → `PASS`
  （かな範囲U+3040〜30FFを要求しない。`、`・`。`・半角カナになる配列でも誤判定しない）。
  かな等と英字が混在 → `process=true`なら`PARTIAL_LITERAL`（FAIL、BUG-002型）、`process=false`なら
  `PRECONDITION_DRIFT`（INVALID。IMEがキーを見ていない=直接入力に落ちた前提状態のドリフト）。
  英字のみ → 同様に`process`で`LITERAL`（FAIL）/`PRECONDITION_DRIFT`（INVALID）。
  加えて`focus_lost`・`前面化に失敗`・物理キー混入（`check_multi.py::phys_in_awase`をimport再利用）・
  `=== 全ケース完了 ===`が無い（Chrome起動失敗・タイムアウトで途中終了）は`INVALID`。
- checkerは試行単位の集計行`TALLY pass=N fail=N invalid=N total=N`を`result.txt`に出す（T4の判定に使う、設計4）。

### 2. 測る量は`gji_idle_ms`、掃引点は4帯を踏む

awaseがcold判定に使う量は**keyboard idleではなく`gji_idle_ms`**（`tsf::observer::gji_idle_ms()`、GJIのI/O観測からの
経過時間。`platform.rs:742,824,869,882`）。Chrome(VK)の帯は次の通り（`gji_fsm.rs`）:

| `gji_idle_ms` | 状態 | `forces_prepend_f2` | 根拠 |
|---|---|---|---|
| < 5s | OnWarm（LongIdleタイマー未発火） | — | `CHROME_LONG_IDLE_MS`=5sは`transition_to_warm`のタイマー長（`:470-479`、`long_idle_ms_for`経由） |
| 5〜7s | OnCold / `Short` | false | `ColdKind::classify`（`:127-136`） |
| 7〜10s | OnCold / `Medium` | **true** | `MEDIUM_IDLE_PROBE_MS`=7s |
| ≥ 10s | OnCold / `Long` | **true**（`is_long`も true） | `LONG_IDLE_MS`=10s |

**`ColdKind::classify`は`CHROME_LONG_IDLE_MS`(5s)を参照しない**。`tuning.rs:88-90`のdocコメント（「`ColdKind::classify`の
Short/Medium/Long重症度分岐のcutoffになる」）は実装と食い違っており**stale**（T6で直す）。
掃引点 **3000 / 6000 / 8000 / 11000 / 14000 ms** は、この4帯（<5 / 5–7 / 7–10 / ≥10）を過不足なく踏む。

- **`gji_idle_ms`は`ensure()`直後のGJI I/Oからの経過**であり、掃引点と一致するとは限らない（何かの拍子にGJI I/Oが走ると
  リセットされる）。**試行ごとにawaseログの`[h1-probe] cold=… idle_at_cold=…ms`（`vk_send.rs:279`）を`IDLE`行のUTCで突き合わせて記録する**
  （checkerの出力に`idle_at_cold`を並べる）。GJI休眠（~12s）が実際に起きているかは、これで直接測れる。
- T4候補1（`prepend_f2_warmup`撤去）が効くのは`gji_idle_ms`≥7s（`forces_prepend_f2`）＝掃引点8000/11000/14000。
  3000/6000で差が出ないのは正常。

### 3. belief合わせ（`chrome_probe`に足す）

既存ワークフローは、スパイクの`--activate-gji`で「awase起動後に`VK_IME_OFF`を1回注入し、awaseの起動時belief(ON推定)と
実状態(OFF)をそろえる」（`e2e-ime.yml:294-299`）。`chrome_probe`にはこれが無く、`ensure()`が`VK_IME_ON`から始まるため、
beliefがずれていると前提状態を作れず全試行が`INVALID`になり、G1で「CIでは無理」と**誤診**する。
`chrome_probe`に`--align-belief`（awase起動・前面化の後、最初の`ensure`の前に、目印付きで`VK_IME_OFF`を1回注入して500ms待つ）を足す。
併せて`ensure()`がfalseを返した回数をログ末尾に出す（`PRECOND_FAIL=n`）ことで、G1の判断を
「Chromeが無い／前面化できない／GJIが効かない」と「前提状態を作れない（belief不整合の疑い）」に分けられる。

### 4. 撤去実験の判定は試行単位の率で行う

`summary`ジョブは各ジョブの`rc`（0=PASS/1=FAIL/3=INVALID）の数だけを見る。`ch-idle`は5掃引点×`--idle-repeat=3`=15試行/ジョブ、
`matrix.run:[1,2,3]`で**45試行/構成**あり、rcのANDに潰れると、ベースラインの1回のflakeで`NG`、撤去側は環境flakeだけで`OK`に
なってしまい、「撤去に検出力がある」ことを示せない。

- checkerの`TALLY`行を`summary`が合算する（`TALLY`が無い既存構成は従来のrc集計にフォールバック）。
- 採否は**率の対比**: ベースラインのfail率≦2%（135試行中2以下の水準）、撤去ありのfail率≧50%（撤去が効くはずの帯=候補1なら
  8/11/14sで）。**これらの数値は初期案**で、T3bの`observe`実行で得られる実際のベースラインflake率を見て較正する。
- 満たせない撤去候補は「E2Eで検出力を示せない」として`docs/experiments.md`に記録し、別候補へ移る（`--idle-repeat`を増やすのは最後の手段）。

### 5. CIへの載せ方

`e2e-ime.yml`の`cfg()`に`driver`フィールド（既定`spike`、新規`chrome`）を足し、`driver=chrome`の構成だけ`chrome_probe.exe`を走らせる。
別ワークフロー(`e2e-chrome.yml`)案は、既存390行を触らず爆発半径が小さい反面、GJI導入・言語設定・`config1.db`書き込みの
約50行を複製する。本計画は`driver`方式を採り、既存構成の回帰は「T3aの前後で既存`baseline`構成の結果が変わらないこと」で確認する。
`config.toml`書き換え部分を別ステップに切り出しても、`${{ }}`は各ステップで展開され、書き換え結果は`dist\config.toml`ファイルとして
次ステップへ渡るので成立する（pwshの変数はステップをまたいで生存しない点に注意）。確認用の`Select-String`行も一緒に移す。

`cache.toml`へのImm capability事前投入が`chrome_probe`に**不要な見込み**な理由: 学習を行う`focus/imm_learning.rs:45-48`は
`new_app_kind != AppKind::Win32`なら早期returnし、ChromeはクラスがChrome_*（`detect_app_kind`が`AppKind::TsfNative`）なので
学習経路に入らない（`IMM32_UNAVAILABLE_CLASSES`は`AppImeProfile`の軸で別）。**要確認**（T3aで実際に誤学習が起きないことを見る）。

## 判定ゲート

- **G0（T1bの後に確定）**: 現行コードで、GJI・Chrome・長idle後に部分リテラルが**出る/出ない**。
  T0の`--settle`は「モードキー押下後の待ち」で、`gji_idle_ms`も伸びるが、モードキー直後にGJIがwarmへ戻っている可能性を
  区別できない。**T0の結論でG0を確定させず、T1b（idle-sweep）の結果で確定する**。
  - 出る → T4（撤去実験）へ。成功基準は「撤去あり=FAIL / なし=PASS」（設計4の率で判定）。
  - 出ない → ADR決定4-0の3条件（ablation必須・陽性対照・INVALID維持）をすべて満たす場合のみCIに載せる。
    ablationで現行機構のいずれかの撤去が症状を戻さないなら、そのassertは何も守っていないのでCIに載せず、
    `docs/experiments.md`に記録して終える。陽性対照（`--no-awase`腕）が取れない回は`INVALID`に落とす。
- **G1（T3aの後）**: CI上でChrome+GJI+awaseの既存8ケースが実行でき、結果が`observe`として集計される。
  失敗した場合は原因を分解する（`chrome_probe.exe`不在＝PB1、Chrome不在、前面化失敗、`PRECOND_FAIL`＝belief不整合）。
  「CIでは無理」と結論するのは、これらを切り分けたうえで、なお解けないときだけとする。

## タスク

### T0（コード変更なし）: 実機で粗い確認をする（G0は確定しない）

- 内容: Windows実機（GJI、awase debugビルド+`AWASE_TEST_INJECTION=1`）で既存`chrome_probe`のケース4
  （`半角英数→ひらがな=かな`、F2→待ち→打鍵でBUG-002の形）を`--settle=500`（対照）と`--settle=3000/6000/8000/11000/14000`で実行する。
  併せて`--no-awase`の対照も1回取る。**ログでは`PROBE action後:`行の`text=`を見る**（`Class`は部分リテラルを`Nicola`等に落とすため。
  `setup:…`・`action後2回目`行も同じ`PROBE `接頭辞なので`action後:`だけを絞る）。
- ログ回収: `chrome_probe.log`と、`idle_at_cold`突き合わせ用の`awase.log`の**両方**を回収する
  （`clipwire-targets.example.toml`の`Get-Content chrome_probe.log`の系統）。
- 実施: clipwireで実機へ（push→Windows側チェックアウトブランチ確認→ビルド→実行）。
- 成果物: 結果表を`tools/e2e/ime_key_matrix/results/`に置く。ADR-193へ「症状の兆候の有無」を追記（**G0は確定しない**）。
- 受け入れ基準: 掃引点ごとに`text`と`idle_at_cold`が記録される。
- 依存: なし。

### T2: checker（Python）を書く

- 変更: `tools/e2e/ime_key_matrix/check_chrome_probe.py`（新規）、`fixtures/chrome_probe/*.log`（新規）、
  `test_check_chrome_probe.py`（新規、`python -m unittest`でLinux実行可）。
- 内容: 2モード。
  - `cases`: 既存ログの`SUMMARY PASS=n RECOVER=n FAIL=n INVALID=n`と`RESULT`行を読む。`=== 全ケース完了 ===`が無ければ`INVALID`（rc=3）。
    終了コードは既存`check_multi.py`と同じ（有効回すべてPASS=0 / FAILあり=1 / 有効回なし=3）。`TALLY`行も出す。
  - `idle`: 設計1の判定。`idle_at_cold`をawaseログから`utc`で突き合わせて併記する。
- 受け入れ基準: fixture（PASS / PARTIAL_LITERAL / LITERAL / PRECONDITION_DRIFT / INVALID（完了マーカー欠落）各1件、実機ログ由来が理想）で
  単体テストが通る。`chrome_probe`の`utc_stamp()`が`check.to_ms`で読めること（**未確認**）をfixtureで確認する。
- 依存: なし（T0と並行可）。

### T1a: `chrome_probe`にbelief合わせと前提失敗カウントを足す

- 変更: `crates/awase-windows/examples/chrome_probe.rs`（+ `README.md`のフラグ表）。
- 内容: `--align-belief`（設計3）と、ログ末尾の`PRECOND_FAIL=n`。既存の8ケースの挙動は`--align-belief`無しで不変。
- 受け入れ基準: `cargo check --target x86_64-pc-windows-msvc -p awase-windows --examples`が通る（`link.exe`不在のためビルド・実行は不可、CLAUDE.md）。
  実機（clipwire）で`--align-belief`付きの既存8ケースが完走し、`PRECOND_FAIL`が出力される。
- 依存: なし。

### T3a: `driver=chrome`をCIに載せ、既存8ケースのsmokeを回す

- 変更: `.github/workflows/e2e-ime.yml`のみ。
  1. **ビルドキャッシュ（PB1）**: `hashFiles(...)`に`'.github/workflows/e2e-ime.yml'`を追加する（無いと、`ch-smoke`は`bin='baseline'`で
     キャッシュが必ずヒットしてビルドがスキップされ、`dist\chrome_probe.exe`が無いまま走る）。
     受け入れ基準に`Test-Path dist\chrome_probe.exe`の確認を入れる（無ければ即`INVALID`）。
  2. `cfg()`に`driver='spike'`を追加。新規構成`ch-smoke`（`driver='chrome'`, `expect='observe'`, `check='chrome-cases'`, `args='--align-belief'`）。
  3. `build`ジョブで`--example chrome_probe`も`dist/`へコピー（全構成、コンパイル時間の増分は**未確認**）。
  4. 既存runステップの`config.toml`書き換え部分（約12行と確認用`Select-String`）を共通ステップに切り出す（機械的な抽出。既存構成の挙動を変えない）。
  5. `driver=chrome`用のステップ: Chromeの有無確認（`Test-Path 'C:\Program Files\Google\Chrome\Application\chrome.exe'`、無ければ
     `choco install googlechrome -y`）→ awase起動（debug、`AWASE_TEST_INJECTION=1`、`RUST_LOG=debug`）→ `awase.log`に起動完了が出るまで確認 →
     `chrome_probe.exe --align-belief … --log=chrome_probe.log`を時間制限付きで実行（ログに`=== 全ケース完了 ===`が無ければ checker が`INVALID`）→
     `check_chrome_probe.py`（T2）→ `rc.txt`。
  6. upload artifactの`path:`に`dist/chrome_probe.log`・`dist/awase.log`を追加し、存在しない側（`driver`違い）があっても落ちないことを確認する。
  7. `plan`の`ONLY`に、`ci/e2e-chrome`ブランチのとき`ch-*`を選ぶ分岐を追加し、`on.push.branches`にも`ci/e2e-chrome`を足す。
- 受け入れ基準: `ci/e2e-chrome`へのpushで`ch-smoke`が3回実行され、artifactに`chrome_probe.log`が残る（結果はFAILでもよい=G1、ただし
  G1の原因分解ができるだけの情報＝`PRECOND_FAIL`・`Test-Path`結果・awase起動ログが残る）。既存`baseline`構成の結果がT3a前後で変わらない。
- 依存: T2, T1a。

### T1b: `chrome_probe`にidle-sweepを足す

- 変更: `crates/awase-windows/examples/chrome_probe.rs`のみ（+ `README.md`）。
- 内容: `--idle-sweep=<ms,ms,…>`と`--idle-repeat=N`（既定3）。指定時は既存の8ケースを回さず、各`(idle, 反復)`で
  `bring_to_front()` → `ensure(Setup::Kana)` → `sleep(idle)` → `k`,`a`（既存`probe()`と同じ押下: 30ms保持・30ms間隔・350ms待ち）→ `snap`で`text`取得 →
  `IDLE`行（設計1の書式）。`ensure`失敗時は`RESULT INVALID: 前提状態にできなかった`。`probe()`を内部関数と既存の分類付きラッパーに分ける
  （既存呼び出しの挙動は不変）。**判定（PASS/FAIL）はログに書かない**。
  「keyboard short idle かつ GJI long idle」（旧BUG-002表の物理F2+GJI休眠）を作る`--pre-settle=<ms>`（モードキー押下の**前**に待つ）も
  ここで足す（ADR決定4-1）。
- ページの30ms周期`fetch('/cmd')`ポーリング（`chrome_probe.rs:57-64`）が、idle中もレンダラを起こし続ける。`gji_idle_ms`には影響しないが、
  Chrome側のidle挙動（TSF composition contextの破棄・再初期化）を歪めうる。**まず現状（30ms）で測り、結果が不安定なら間引き版
  （idle中のみ1000ms、停止はしない=`snap`/`clear`が届かなくなる）と比較する**。
- 受け入れ基準: `cargo check … --examples`が通る。実機で`--idle-sweep=3000,11000 --idle-repeat=2`が完走し`IDLE`行が出力される。
  T2の`idle`モードがその実機ログをパースでき、`idle_at_cold`が掃引点と突き合わせられる。
- 依存: T2（入力形式の固定）、T3a（CI smokeで土台が動くことを確認してから）。

### T3b: `ch-idle`構成を追加

- 変更: `.github/workflows/e2e-ime.yml`のみ。`cfg('ch-idle', 'observe', driver='chrome', check='chrome-idle',
  args='--align-belief --idle-sweep=3000,6000,8000,11000,14000 --idle-repeat=3')`。あわせて`summary`が`TALLY`を合算するよう拡張
  （設計4、`TALLY`が無ければ従来のrc集計）。
- 見積もり式: Σ(idle)×repeat + 試行数 ×（`ensure`1.2〜2.6s + 打鍵0.6〜1.1s + 前面化）。Σidle=42s、repeat=3、15試行 → 126s + 15×(3〜5s) ≒ **3.5〜4分/ジョブ**
  （Chrome起動は最大40s待ち+1.5s+0.8sでプロセス当たり1回）。matrixの`run:[1,2,3]`で3ジョブ。`timeout-minutes: 25`内の見込み
  （GJI導入約30秒+`ctfmon`再起動等は別、実測で確認）。`--idle-repeat`を増やす場合は式に戻して見積もり直す。
  **25分を超える場合の対処順**: 掃引点を減らす → 構成を分ける → `run:[1,2,3]`を減らす（別ワークフロー化は最後）。
- 受け入れ基準: `ci/e2e-chrome`で`ch-idle`が3回完走し、`observe`として集計される。`TALLY`から実際のベースラインflake率が得られる。
- 依存: T1b, T3a。

### T4（G0が「出る」、または「出ない」でもablationが効くと示したいとき）: 撤去スクリプトを追加

- 変更: `tools/e2e/ime_key_matrix/ablations/a8-*.sh`（新規、a5と同形式=Pythonで`assert a in s`して文字列置換）、
  `e2e-ime.yml`の`plan`に`cfg('ch-a8-…', 'fail', mutator='a8-….sh', driver='chrome', check='chrome-idle', …)`。
- 撤去対象の候補（**どれが症状を防いでいるかはT1b/T3bの結果で決める。現時点では未特定**）:
  1. `output/vk_send.rs`のcold-start分岐（`prepend_f2_warmup`、`:256`。定義`output/mod.rs:405`、条件`:1435`）を無効化する。
     **有効域は`gji_idle_ms`≥7s**（`forces_prepend_f2`）＝掃引点8000/11000/14000のみ。3000/6000で差が出ないのは正常。
  2. `tsf/warmup/probe_fsm.rs::run_per_vk_confirm`（**`:454`**）が各VKを即confirm扱いにする。
  3. `RawTsfLiteralRecovery`の回収分岐: **所在を確定してから**候補にする（`literal_detect_fsm.rs`には無い。出現は
     `output/tsf_warmup_coord.rs:411,607,633`、`output/mod.rs:309,314,1827,1982,1985`）。どのファイルのどの分岐を撤去するかを先に特定する。
- 採否基準: 設計4（試行単位のTALLY、ベースラインfail率≦2%・撤去ありfail率≧50%、撤去が効くはずの帯で差が出ること。数値はT3bで較正）。
  満たせない候補は`docs/experiments.md`に記録して別候補へ。
- 受け入れ基準: `summary`で撤去構成が`OK`（期待FAILで実際にFAIL）、ベースライン構成が`OK`（期待PASS）を、**別々の2回のCI実行で再現**する。
- 依存: T3b、G0。

### T5: 記録を更新する

- 変更: `docs/known-bugs/BUG-002.md`（T4の結果に基づき「現在の対策」を per-VK confirmに書き直す）、ADR-193のstatus・決定4の結果欄、
  （新規バグ発見なら）`docs/known-bugs/BUG-NNN.md`を次の連番で新規作成（frontmatter規約に従う）、（撤去しても壊れない場合）`docs/experiments.md`に1行。
- 受け入れ基準: known-bugsの「現在の対策」が現行コードに存在する定数・関数だけを指す（`grep`で存在を裏取り）。
- 依存: T4（または「出ない」判定）。

### T6（独立・いつでも）: `tuning.rs`のstale docを直す

- 変更: `crates/awase-windows/src/tuning.rs:88-90`のdocコメントのみ。`CHROME_LONG_IDLE_MS`(5s)は`transition_to_warm`のOnWarm→OnColdタイマー長
  （`gji_fsm.rs:470-479`）であり、`ColdKind::classify`のcutoffは`MEDIUM_IDLE_PROBE_MS`(7s)/`LONG_IDLE_MS`(10s)であることを書く。
- **定数値は変えない**（`tuning-constants.md`の実測義務は発生しない）。pre-pushフックが再発ファミリー対象ファイルの変更として
  警告を出しうるが、コメントのみの変更でありテスト・known-bugsの追加は不要（警告は無視してよい旨をコミット本文に書く）。
- 受け入れ基準: `cargo check --target x86_64-pc-windows-msvc -p awase-windows`が通る。
- 依存: なし。

## 着手順序

```
T0 ─────────────────────────────────────────────────┐(粗い確認のみ)
T2 ─┐
T1a ┴► T3a ─(G1)► T1b ─► T3b ─► (G0確定) ─► [T4] ─► T5
T6（独立）
```

T0・T2・T1a・T6は並行可。T3aはT1bより先（ハーネスがCI上で動くかの確認を、新機能の実装より先に行う）。
**G0はT1b（idle-sweep）の結果で確定する**（T0では確定しない）。実装ブランチは`develop`から専用worktreeで切る（`worktree-per-session`）。
ADR・本計画のdevelopへの反映を先に行う。CI実行は`ci/e2e-chrome`ブランチへのpush（`ci/e2e-scenarios`と同じ運用）。

## 規約との関係

- 変更対象は`examples/`・`tools/`・`.github/workflows/`・docsと、T6の`tuning.rs`のコメントのみ。`fix-requires-evidence.md`の再発ファミリー
  対象ファイル（`src/`配下のwarmup/focus/belief等）のロジックは**変更しない**。撤去スクリプトはCI実行時に作業ツリーへ差分を当てるだけ。
- `tuning.rs`の定数を新設・変更しない、`RESTRICTED_CALLS`許可リストも触らない（`complexity-budget`対象外）。
- checkerはfixtureを持つPython単体テストで担保する（E2E自体は実機依存でLinuxでは走らない）。
- 本ADRが自ら掲げる規律: **known-bugsの「現在の対策」だけでなく、`tuning.rs`等のdocコメントも、現行実装で裏取りしてから前提にする**
  （PM1で、docコメントの記述を鵜呑みにした誤りが計画・ADRとレビュアー自身の指摘にまで伝播した）。

## リスク

| # | リスク | 対策 |
|---|---|---|
| R1 | `chrome_probe`はCIで一度も実行されておらず、Chromeの前面化・初回起動ダイアログ・ループバックHTTPがランナーで動くか不明 | T3aのsmokeを先行し、G1で原因を分解して判断する |
| R2 | GJI休眠（~12s）が実際に起きているか | `idle_at_cold`を試行ごとに突き合わせて記録する（設計2）。未知のリスクではなく必ず記録する観測量 |
| R3 | 実IMEのflake（1回の失敗で判定が揺れる） | 試行単位のTALLYと率で判定（設計4）。`observe`から始め、ベースラインflake率を測って数値を較正 |
| R4 | e2e-ime.ymlの共通ステップ切り出しで既存構成が壊れる | 機械的抽出に限定し、T3a前後で`baseline`の結果を比較 |
| R5 | T0の`--settle`は「モードキー後の間隔」で、キー無入力idleとは別物 | T0はG0を確定させず、確定はT1b |
| R6 | NICOLA出力の文字が配列・設定に依存する | 判定を「ASCII英字を含まない」にし、特定のかな範囲を要求しない |
| R7 | beliefのずれで全試行が`INVALID`になり、G1で誤診する | `--align-belief`（T1a）と`PRECOND_FAIL`カウント |
| R8 | ページの30msポーリングがChromeのidle挙動を歪める | まず現状で測り、不安定なら間引き版と比較（T1b） |
| R9 | ビルドキャッシュが古いままT3aを走らせる | `hashFiles`にworkflowを追加、`Test-Path dist\chrome_probe.exe`を受け入れ基準に（PB1） |
