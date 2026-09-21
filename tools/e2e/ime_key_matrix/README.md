# Windows 実機検証ハーネス(ime_key_matrix / chrome_probe)

無変換/変換/ひらがな/Shift+無変換などの **IME モードキー**を実機に注入し、**実 IME の状態**と
**awase の Engine 切り替え**を自動で記録・判定する。Windows の IME 周りは実機・アプリごとに挙動が違い、
API が状態を偽ることもあるため、実際にキーを打って結果を見る。

## 構成

| ファイル | 役割 |
|---|---|
| `crates/awase-windows/examples/ime_key_matrix_spike.rs` | 自前ウィンドウ(EDIT/RichEdit)を開き、実 IME の状態(`ImmGet*` / `WM_IME_CONTROL` / TSF compartment)を押下前・+100/+400/+1500ms で記録する。`--auto`(キー自動注入)、`--repeat=N`(1プロセスでN回)、`--fast`(+1500msを省く)、`--speed=K`(手順間をK倍速)、`--shiftmuh`(無変換をShift+無変換で注入)、`--free`/`--script`(手動)。ADR-191 の学習/検証用フラグ(`--grid`/`--notify` 等)と全フラグ・ログタグの一覧は同ファイル冒頭doc |
| `crates/awase-windows/examples/chrome_probe.rs` | 専用プロファイルの **Chrome**(TsfNative)で同じキーを打ち、`k`,`a` の出力(NICOLA文字/`か`/`ka`/`kiu`)で状態を判定する。ローカルHTTP+検証ページ(`keydown`/`composition*`/`beforeinput` を記録)。`--repeat`・`--no-awase`・`--settle=MS`・`--shift-tail=MS`・`--storm=N` |
| `check.py` | スパイクのログと awase のデバッグログを突き合わせ、`EXPECT`(期待表)で PASS/FAIL 判定 |
| `check_multi.py` | `--repeat`/ループ実行のログを実行(RUN)ごとに切り出して判定。人の物理入力の混入(スパイク側+awaseログの `extra=0x0`)を無効(INVALID)にする |
| `run.sh` | 1回実行(約1分)。`run_multi.sh`:1プロセスでN回。`run_loop.sh`:毎回新しいプロセスでN回(Windows側ループ `e2e-loop.ps1`、1回あたり約23秒) |
| `stat_runs.sh` | 有効な回がN回たまるまで `run.sh` を繰り返して集計 |
| `clipwire-targets.example.toml` | Windows 側で実行するコマンド(clipwire の target)。パスは環境に合わせる |

## 仕組み
- キーは `SendInput` で注入し、`dwExtraInfo = 0x5350494B` の目印を付ける。awase は環境変数 `AWASE_TEST_INJECTION=1` の
  ときだけ、この目印付きの注入を物理キーとして扱う(`hook.rs::is_test_injection`)。**デバッグビルドでのみ有効**(リリースビルドは環境変数があっても無効)。本番(環境変数なし)は従来どおり。
  目印の定数は `hook::TEST_INJECTION_MARKER` を、スパイク/プローブが参照する(二重定義しない)。
- スパイクの状態機械が「前提の状態」を自動で作る(準備キーを注入)。

## 実行
1. Windows 側で awase を `AWASE_TEST_INJECTION=1`・`RUST_LOG=debug` で起動する。
2. `clipwire-targets.example.toml` の target を登録・承認する。
3. `./run.sh`(または `./run_loop.sh e2e-args-default`)。終了コード 0 = 全 PASS。

## 注意(測定の落とし穴)
- **実行中は Windows 機のキーボード・マウスに触らない/ロックさせない。** 人の入力が混ざると失敗が偽の再現になる
  (実際に、混入した回を含めて「awase のせいで押下が落ちる」と誤った結論を出しかけた)。`check_multi.py` は awase ログの
  物理キー(`extra=0x0`)も見て INVALID にするが、GJI 単体(awase 停止)側は awase ログが無く検査できないので、対照実験は両方の腕を
  同じ条件で取ること。
- 実行中に Windows 側で PowerShell を起動すると前面が奪われる(ログ取得は固定時間待ってから1回だけ)。
- clipwire のスクリプトはバックスラッシュを書かない(`C:/Users/...`)。PowerShell 5 の `.ps1` は BOM なしを ANSI 解釈するので日本語を書かない。
- **`check.py` の `EXPECT` は ADR-186(GJI ATOK の無変換/変換の押下時点 belief 追随)を実装した awase の挙動に対する期待表。**
  そうでない awase では FAIL する項目がある。判定の追加・変更は `EXPECT` を編集する
  (`engine=("none", 1500)` = 押下後1500ms Engine が activated にならない、`("activated", 300)` = 300ms 以内に activated)。

## ADR-186/187/189 で追加した判定・モード(GitHub Actions `e2e-ime` ワークフローでも使う)
| ファイル / 引数 | 役割 |
|---|---|
| `--walk` / `--cold` | ひらがな/無変換/変換の固定キー列(前提状態なし)を押す。`--cold`は先頭のひらがなを除く(明示意図のない状態でいきなり押す) |
| `--resync` / `--resync-gap=N` | ずれを起こしたあと、Ctrl+無変換→Ctrl+変換(または逆)を素早く押してリセットできるかを見る |
| `--hz` | 半角/全角(0xF3/0xF4、GJIではどちらも開閉トグル)の連続・交互押下 |
| `--vkprobe` | 各VK候補を注入して、実IMEの変化とOSが配送するVKを記録する(awaseなしの対照実験向け) |
| `--msime` / `--activate-gji` | 使うIMEのTSFプロファイルをアクティブ化(Microsoft IME / GJI) |
| `check_consistency.py` | プリセット非依存: 実IMEのかな=Engine ON、英数/直接入力=Engine OFF に追随するか(Engineは各押下の700ms後の`k`の扱いで読む) |
| `check_resync.py` | リセット操作の後に実IMEとEngineが揃うか |
| `check_toggle.py` | 開閉トグルキーが押すたびに反転し、Engineが追随するか |
| `ablations/a*.sh` | 撤去実験(ミューテーター)。`a7-no-follow.sh`はfollow(ADR-187)を無効化してずれを起こす |

## ADR-191/193: 学習ラウンド(格子)・検証ラウンド(walk)・通知の計測ツール(ワークフローの `cal-*` 構成 = `check: collect`)
`cal-*` 構成は判定せずログを回収し、`[GRID-ABORT]` による打ち切り(rc=3=INVALID)だけを検出する(解析は下のツールでローカルに行う)。
スパイクの全フラグとログタグは `ime_key_matrix_spike.rs` の冒頭docが一覧(`--grid`/`--grid-setup`/`--grid-adaptive`/`--fast`/`--speed`/`--notify`/`--notify-comp`/`--snap100`、`--walk=N --seed=S` を含む)。
**注意: `--walk`(値なし)は ADR-186 の固定キー列、`--walk=N --seed=S` はランダムなキーをN回注入する ADR-191 の walk で、別物。**

| ファイル | 役割 |
|---|---|
| `grid_learn.py` | `--grid` のログ(`ime_key_matrix_spike.log`)を集計し、セル(状態×キー)ごとの結果の分布・決定性・入力中の行方・セットアップ不能を出す。`--json` で表(セル→結果の分布)、`--graph` でキー到達の遷移グラフ、`--diff` で keys 版と imm 版のセル差分 |
| `effect_learning.py` | 名前は「学習」だが実体は次の4つ: (1)walk ログから表を学習し決定性を出す、(2)一段予測・開ループ予測の精度(既定。`--spec` は Mozc 仕様モデルの評価)、(3)awase 起動時の Engine と実 IME のずれ(`--drift <spike.log> <awase.log>`、`DRIFT_OFF=100/400/1500` で判定時刻を選ぶ)、(4)A/B 条件の表の差分(`--compare`) |
| `cycle.py` | 3段階ラウンド(設定の読み取り・学習・検証)のオフライン計測。`all <学習ログ> -- <検証ログ>` で、設定由来の表 S・学習した表 L・合成 M を、学習に使っていない walk の開ループ連鎖で採点する |
| `conv_exp.py` | 隠れ状態「変換中(Conversion)」を打鍵履歴から追跡すると、入力中の Esc・無変換の非決定セルが決定的になるかの検証実験(研究用。製品コードは使わない) |
| `score_walk.py` | 格子から作った予測表(JSON)を、独立した walk(キーで到達した状態)で一段予測として採点し、不一致セルを列挙する |
| `gen_grid_nondet.py` | 前回の格子の表(`grid-tables/*.json`)から、結果が割れたセルの一覧(`grid-tables/nondet-*.txt`)を作る。`--grid-adaptive` の2パス目(`--grid-retry-file`)が再試行する対象になる |
| `notify_latency.py` | `compartment_notify_probe`(ADR-193)のログから、キー→TSF compartment 変更通知の遅延(P50/P95/最大)、通知が来なかったキーの割合、通知の順序、周期読み取りとの比較を集計する |
| `grid-tables/` | 格子(`--grid`)の学習結果の参照データ(`atok.json`/`msime.json`: セル→結果の分布)と、`--grid-adaptive` の再試行対象(`nondet-*.txt`)。予測表の生成元は撤去ブランチ側 |
| `patches/compartment_notify_probe-setfocus.patch` | ADR-193 の `compartment_notify_probe.rs` **本体**への修正案(`WM_SETFOCUS` で入力欄へフォーカスを戻す1アーム。CI で前面化に失敗する原因の対処)。**未適用**(本体は別セッションの成果物) |
