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
  **この目印判定はVKを区別しない**ので、Ctrl/Shift/Alt等のOS修飾キーも目印付きで注入すれば
  `HOOK_STATE.physical_key_state`が正しく更新され(`read_os_modifiers()`のCtrl/Shift判定に反映)、
  修飾キーを保持したままの物理チョード(例: Ctrl+変換の強制ON)を自動テストできる(2026-09-22確認、`--chord`参照)。
- スパイクの状態機械が「前提の状態」を自動で作る(準備キーを注入)。
- **`--chord=VK1,VK2`**: VK1を押しっぱなしにした状態でVK2をタップし、VK1を離す(「本物の同時押し」)。
  `--seq`/`--walk`は各VKを逐次タップするだけで重なりが無く、Ctrl等のOS修飾キーを保持したままの
  物理チョードを再現できない(`send_key`自体は他のモードと同じ目印を使うので、この`--chord`が無くても
  修飾キーの追跡自体は動く。無かったのは「重なりのある注入」を組み立てる手段だけ)。
  実機確認: `Ctrl(0xA2)+変換(0x1C)`で`mods(c=true...) phys_ctrl=true`となり、既定の
  `keys.ime_on=["Ctrl+変換"]`が`origin=ExplicitUserAction`で正しく発火することを確認した。

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
| `ablations/a*.sh` | 撤去実験(ミューテーター)。`a7-no-follow.sh`はfollow(ADR-187)を無効化してずれを起こす。`a8`〜`a10`はconv軸の自動書き込み(焦点プローブ・ROMAN補完)の撤去(docs/tasks/conv-write-paths-inventory.md) |

## ADR-191/193: 学習ラウンド(格子)・検証ラウンド(walk)・通知の計測ツール(ワークフローの `cal-*` 構成 = `check: collect`)
`cal-*` 構成は判定せずログを回収し、`[GRID-ABORT]` による打ち切り(rc=3=INVALID)だけを検出する(解析は下のツールでローカルに行う)。
スパイクの全フラグとログタグは `ime_key_matrix_spike.rs` の冒頭docが一覧(`--grid`/`--grid-setup`/`--grid-adaptive`/`--fast`/`--speed`/`--notify`/`--notify-comp`/`--snap100`、`--walk=N --seed=S` を含む)。
**観測時点(`--at`)に注意**: `grid_learn.py` の既定は、ログにある最も遅い観測時点(通常 +1500ms、`--fast` は +400ms、`--snap100` は +100ms)。`--at=N` を指定してその時点の観測が0件ならエラー終了する(`--fast` のログに `--at=1500` を渡すと空の表になり、`--diff` が「差分0」になる偽陽性を防ぐ)。`--diff`/`effect_learning.py --compare` は共通セルが0件なら警告して非ゼロ終了する。「差分0」を引用するときは共通セル数を併記すること。`effect_learning.py --drift` は `DRIFT_OFF`(100/400/1500)に対応する観測時点(+100/+400/+1500ms)のIME状態と、その時点のEngine状態を比べる(該当観測が無ければエラー)。
**注意: `--walk`(値なし)は ADR-186 の固定キー列、`--walk=N --seed=S` はランダムなキーをN回注入する ADR-191 の walk で、別物。**

| ファイル | 役割 |
|---|---|
| `grid_learn.py` | `--grid` のログ(`ime_key_matrix_spike.log`)を集計し、セル(状態×キー)ごとの結果の分布・決定性・入力中の行方・セットアップ不能を出す。`--json` で表(セル→結果の分布)、`--graph` でキー到達の遷移グラフ、`--diff` で keys 版と imm 版のセル差分 |
| `effect_learning.py` | 名前は「学習」だが実体は次の4つ: (1)walk ログから表を学習し決定性を出す、(2)一段予測・開ループ予測の精度(既定。`--spec` は Mozc 仕様モデルの評価)、(3)awase 起動時の Engine と実 IME のずれ(`--drift <spike.log> <awase.log>`、`DRIFT_OFF=100/400/1500` で判定時刻を選ぶ)、(4)A/B 条件の表の差分(`--compare`) |
| `cycle.py` | 3段階ラウンド(設定の読み取り・学習・検証)のオフライン計測。`all <学習ログ> -- <検証ログ>` で、設定由来の表 S・学習した表 L・合成 M を、学習に使っていない walk の開ループ連鎖で採点する |
| `conv_exp.py` | 隠れ状態「変換中(Conversion)」を打鍵履歴から追跡すると、入力中の Esc・無変換の非決定セルが決定的になるかの検証実験(研究用。製品コードは使わない) |
| `score_walk.py` | 格子から作った予測表(JSON)を、独立した walk(キーで到達した状態)で一段予測として採点し、不一致セルを列挙する |
| `gen_key_effect_table.py` | `grid-tables/{atok,msime,msime-native}.json` から、打鍵時予測の表 `crates/awase-windows/src/state/key_effect_table.rs` を生成する(ADR-191 決定3・4)。`--check` は何も書かずコミット済みの生成物との一致だけを検査する(`architecture_guard` の `key_effect_table_matches_generator` が呼ぶ)。予測器本体は `state/key_effect_predictor.rs` |
| `gen_grid_nondet.py` | 前回の格子の表(`grid-tables/*.json`)から、結果が割れたセルの一覧(`grid-tables/nondet-*.txt`)を作る。`--grid-adaptive` の2パス目(`--grid-retry-file`)が再試行する対象になる |
| `notify_latency.py` | `compartment_notify_probe`(ADR-193)のログから、キー→TSF compartment 変更通知の遅延(P50/P95/最大)、通知が来なかったキーの割合、通知の順序、周期読み取りとの比較を集計する |
| `grid-tables/` | 格子(`--grid`)の学習結果の参照データ(`atok.json`/`msime.json`/`msime-native.json`: セル→結果の分布)と、`--grid-adaptive` の再試行対象(`nondet-*.txt`)。予測表(`key_effect_table.rs`)の生成元でもある(`gen_key_effect_table.py`) |
| `patches/compartment_notify_probe-setfocus.patch` | ADR-193 の `compartment_notify_probe.rs` **本体**への修正案(`WM_SETFOCUS` で入力欄へフォーカスを戻す1アーム。CI で前面化に失敗する原因の対処)。**未適用**(本体は別セッションの成果物) |

## 不変条件(異常検出器)と PR ゲート(`check_invariants.py` / `invariant_limits.json` / `e2e-ime-smoke`)

期待表(`check.py` の `EXPECT` 等)は設計が変わるたびに古くなり、BUG-162・BUG-163 はログに出ていたのに数日見逃された。
そこで、設計によらず「起きてはいけないこと」を awase.log から数え、件数だけで合否を決める検出器を置いた。

| 不変条件 | 数えるもの | 上限の根拠 |
|---|---|---|
| I1 `i1_startup_drift_no_intent` | 起動(ログ先頭行)から `window_s` 秒以内の `[drift] correction: … set_ime_open(…)` のうち、同じ観測サイクル(直前 100ms 以内の `explicit_intent=` 行)が `explicit_intent=None` のもの。直前に `explicit_intent=` 行が無いものも「意図の証拠なし」として数える(書式変更で黙って0件にならないように) | BUG-163 |
| I1 `i1_drift_no_intent_total` | 同じ条件でログ全体 | BUG-163 |
| I2 `i2_unwarranted` | journal の `ime open applied seq=… outcome="Unwarranted"` 行(同じ seq は1件)。同じ span(`on_ime_apply_complete{… outcome=Unwarranted …}`)の別の行は数えない(`check.py` は2行を2件と数える) | BUG-162 |
| I3(情報のみ) | 自己注入の IME モードキー(`[hook] IME-mode vk=… down self_injected=true`、vk 別)と `[warrant-shadow] … would_have_blocked=true`(chain/strategy 別) | 上限なし |

- 使い方: `python3 check_invariants.py [--config 構成名] [--window 秒] [--json out.json] awase.log`。
  終了コード 0=上限以内 / 1=超過 / 3=ログが無い・起動行が無い(INVALID)。最終行が1行サマリ `INVARIANTS: verdict=…`。
- 単体テスト: `python3 -m unittest discover -s tools/e2e/ime_key_matrix -p 'test_*.py'`(フィクスチャは `testdata/` の CI 実ログ抜粋)。
- `e2e-ime.yml`: awase を起動する全構成で数え、summary に**期待表とは別の表**で出す(情報のみ。期待表の合否は変えない)。
- `e2e-ime-smoke.yml`(PR ゲート、develop 向け PR と develop への push): `e2e-ime.yml` を workflow_call で呼び、
  `atok-passthrough-cold` を1回だけ回して**不変条件の件数だけ**で合否を決める(`gate_mode=invariants`)。
  期待表の結果は表示のみ(フォーカス喪失で INVALID になる等の CI の揺らぎで赤くしないため)。

### 上限(ratchet)の運用

- 上限は `invariant_limits.json` の `limits`(全構成の既定)と `config_overrides`(構成別)。各上限に
  `max`(超えたら FAIL)、`observed_min`(実測の揺れ幅の下限)、`bug`(許容している既知バグの番号)、`measured`(実測値と出典)を書く。
- 値が `max` 以下なら OK。`observed_min` も下回ったら「上限を下げてよい」と表示する。**上限は下げる方向にだけ動かす**:
  バグが直ったら `max` を実測値(理想は 0)まで下げ、`observed_min` もそろえる。
- 上限を上げる・新しい上限を足すときは、**必ず対応する BUG 番号(`docs/known-bugs/BUG-NNN.md`)と実測値(どの run の何本で何件)**を
  書く。「赤いので上げた」は禁止(`.claude/rules/tuning-constants.md` と同じ考え方)。
- CI の揺らぎで件数が揺れる不変条件は、揺れ幅の最大を `max`、最小を `observed_min` にする。
- 現在の上限の実測(windows-latest・GJI+ATOK、run 36103384502 の baseline×3・atok-passthrough-cold×3 と
  run 36104015748 の baseline×3、計9本): I1 起動10秒 2〜3件、I1 全体 3〜4件、I2 全て1件。
  旧 run 35620809258(ci/e2e-drift-fix)の3本は I1 起動10秒 18〜19件・全体 19〜22件で、今の上限では FAIL になる。
