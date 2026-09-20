# Windows 実機検証ハーネス(ime_key_matrix / chrome_probe)

無変換/変換/ひらがな/Shift+無変換などの **IME モードキー**を実機に注入し、**実 IME の状態**と
**awase の Engine 切り替え**を自動で記録・判定する。Windows の IME 周りは実機・アプリごとに挙動が違い、
API が状態を偽ることもあるため、実際にキーを打って結果を見る。

## 構成

| ファイル | 役割 |
|---|---|
| `crates/awase-windows/examples/ime_key_matrix_spike.rs` | 自前ウィンドウ(EDIT/RichEdit)を開き、実 IME の状態(`ImmGet*` / `WM_IME_CONTROL` / TSF compartment)を押下前・+100/+400/+1500ms で記録する。`--auto`(キー自動注入)、`--repeat=N`(1プロセスでN回)、`--fast`(+1500msを省く)、`--speed=K`(手順間をK倍速)、`--shiftmuh`(無変換をShift+無変換で注入)、`--free`/`--script`(手動) |
| `crates/awase-windows/examples/chrome_probe.rs` | 専用プロファイルの **Chrome**(TsfNative)で同じキーを打ち、`k`,`a` の出力(NICOLA文字/`か`/`ka`/`kiu`)で状態を判定する。ローカルHTTP+検証ページ(`keydown`/`composition*`/`beforeinput` を記録)。`--repeat`・`--no-awase`・`--settle=MS`・`--shift-tail=MS`・`--storm=N` |
| `check.py` | スパイクのログと awase のデバッグログを突き合わせ、`EXPECT`(期待表)で PASS/FAIL 判定 |
| `check_multi.py` | `--repeat`/ループ実行のログを実行(RUN)ごとに切り出して判定。人の物理入力の混入(スパイク側+awaseログの `extra=0x0`)を無効(INVALID)にする |
| `run.sh` | 1回実行(約1分)。`run_multi.sh`:1プロセスでN回。`run_loop.sh`:毎回新しいプロセスでN回(Windows側ループ `e2e-loop.ps1`、1回あたり約23秒) |
| `stat_runs.sh` | 有効な回がN回たまるまで `run.sh` を繰り返して集計 |
| `clipwire-targets.example.toml` | Windows 側で実行するコマンド(clipwire の target)。パスは環境に合わせる |

## 仕組み
- キーは `SendInput` で注入し、`dwExtraInfo = 0x5350494B` の目印を付ける。awase は環境変数 `AWASE_TEST_INJECTION=1` の
  ときだけ、この目印付きの注入を物理キーとして扱う(`hook.rs::is_test_injection`)。**本番(環境変数なし)は従来どおり**。
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
