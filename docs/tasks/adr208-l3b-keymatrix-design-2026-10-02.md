# ADR-208 L3b: CI の「ずれの作り方 × キー」行列と E2 の対照(設計メモ、2026-10-02)

対象: ADR-208 決定4(例外の E1/E2)・決定5(b)(CI の drift × キー行列)・決定7 (2)(v2 ブロッカーの受け入れ条件)。
ブランチ `ci/adr208-l3b-keymatrix`(develop `dbfcdf4c` 起点)。awase 本体(`src/`、`crates/awase-windows/src/`)は変更していない。**CI は未実行**(実測前の構成なので全て `expect=observe`)。

## 何を測るか

明示キーを押したとき、awase の内部状態(belief・`applied`)が何であっても、**絶対キーは1押下、トグルは2押下以内**で実 IME がキーの意味に一致するか(INV-L2)。
セル = `<key>=<kind>:<gap>`、各セル内部10試行(1構成=1ジョブ、runs=1)。

- kind: `on`(ON 絶対) / `off`(OFF 絶対) / `tog`(トグル。目標=押す前の状態の反転)
- gap(ずれの作り方):
  - `sync`: awase の明示キー(0x16/0x1A)で状態をそろえる。ずれなし(対照)
  - `close`: awase に ON を書かせた(applied=ON、belief=ON)後、ハーネスが実 IME を `WM_IME_CONTROL(IMC_SETOPENSTATUS,0)` で閉じる。**S-1 の直接再現**(ずれ A と、古い applied を作る手順(c)は同じ操作になる。D3 の `drift-on` と同じ)
  - `open`: awase に OFF を書かせた後、ハーネスが実 IME を外から開く(ずれ B。belief=OFF、実 IME=開)
  - `fresh`: awase を **ハーネスが kill → 起動し直し**、belief・applied が未知の状態で押す(E2)。実 IME は awase が居ない間に IME 自身へ 0x1A/0x16 を処理させてそろえる
- `close` × `off`、`open` × `on` は作らない(押す前から実 IME が意味と一致していて測定にならない)。

## 注入・記録・判定

- 注入: マーカー付き SendInput(`TEST_INJECTION_MARKER`)。`ctrl+` 前置のセルは Ctrl↓(40ms)→キー↓(60ms)→キー↑(30ms)→Ctrl↑(物理 Ctrl 扱い、D3 変種と同じ順序・間隔)。単独タップは 60ms。
- 各押下の後に +500ms/+2000ms の実 IME 状態(自前窓は `ImmGetOpenStatus`)。一致しなければ最大3回まで押す(`--km-max-press`)。1押下目で一致=CONVERGED_1、2押下目=CONVERGED_2、3押下目=LATE、3回押して不一致=STUCK。
- 打鍵(かな単打):自前窓は試行の最後に1回(押下の間には打たない=awase の literal 回収が状態を直す交絡を避ける)。実 Chrome は API(`IMC_GETOPENSTATUS`)が TsfNative で信頼できないので、**押下ごとに k,a を打った結果(かな=開、英字=閉)を主証拠**にする(`chrome_probe` の既存分類)。交絡: Chrome では押下の間にも打つため、2押下目以降は「1押下目の打鍵で awase が状態を直した」可能性がある(1押下目の判定は無傷。STUCK の判定は直す方向にしか交絡しないので保守的)。
- 判定 `check_keymatrix.py`(`typing_stress.log` の `[TS-JSON]` と `chrome_probe.log` の `KM {json}` の両方を読む。awase.log も渡すと ctrl+ セルの物理 Ctrl〈`mods(c=true …) phys_ctrl=true`〉を要求し、無い/false が混じるなら INVALID):

| verdict | 意味 |
|---|---|
| CONVERGED_1 | ずれを作れた全試行が1押下で一致 |
| CONVERGED_2 | 2押下まで(**トグルなら合格**、絶対キーは1押下の保証違反) |
| CONVERGED_LATE | 3押下目で一致する試行がある(保証違反) |
| STUCK | 3回押しても一致しない試行がある(固着) |
| ENV_EXCEPTION | 不合格だが、決定4(a)の例外セル(**MS-IME × 実 Chrome の OFF 方向**=target が OFF のセル。トグルの OFF 方向・単独タップ無変換を含む)で、E2 の対照が成立 |
| GAP_NOT_MADE | ずれを1件も作れなかった(測れなかっただけで不合格ではない) |
| INVALID | 有効試行が半数未満・中断・未完走・物理 Ctrl でない |

**E2 の判定**: 例外セルの不合格に対し、同じ job の `fresh` セル(同じキー・種別)の失敗率 ff と、そのセルの失敗率 fc を比べる。`ff > 0 かつ ff >= 0.5 * fc` なら ENV_EXCEPTION(「新鮮でも同じ程度に閉じない=環境」)。ff が小さければ STUCK のまま(`e2=fresh_ok`、新鮮だと閉じる=**内部状態による固着=バグ**)。fresh セルが無ければ STUCK のまま(`e2=no_fresh_cell`)。fresh セル自身が例外セルで失敗した場合も、内部状態に由来しえないので ENV_EXCEPTION。**例外の列挙の外**(GJI の OFF、ON 方向、自前窓)では fresh が失敗しても ENV_EXCEPTION にしない(例外は閉じた列挙で、増やすには ADR の改訂が要る)。比率 0.5 は暫定(`E2_RATIO`)。実測の分布を見て決める。
E1(決定表で書き込みが出ている)は L0 の全列挙で既に示せるので、ここでは扱わない。

## 構成一覧(14 構成・14 ジョブ)

窓 = `edit`(ImmCross)・`tsf`(`typing_stress --form=tsf`、TsfNative 相当)・`chrome`(実 Chrome、Imm32Unavailable、`chrome_probe`)。IME = `gji`(ATOK プリセット、keymap=1)・`msime`(MS-IME 本体)。

| 構成 | 窓 × IME | config の `[keys]` | セル |
|---|---|---|---|
| `sc-keymatrix-abs-{edit,tsf,chrome}-{gji,msime}`(6) | 3窓 × 2 IME | `ime_on = ["Ctrl+変換", "VK_CONVERT"]`、`ime_off = ["Ctrl+無変換", "VK_NONCONVERT"]`(単独タップを bare の役割にする=ADR-206 の役割由来。GJI の CUSTOM 表や MS-IME の設定に依らず作れる) | `ctrl+1c=on:close`・`ctrl+1d=off:open`・`ctrl+1c=on:sync`・`ctrl+1d=off:sync`・`1c=on:close`・`1d=off:open`・`16=on:close`・`1a=off:open`、MS-IME のみ `f2=on:close`(9 セル×10) |
| `sc-keymatrix-tog-{edit,tsf,chrome}-{gji,msime}`(6) | 同上 | `ime_toggle = ["VK_NONCONVERT"]`(無変換の単独タップをトグルに) | `1d=tog:{sync,close,open}`・`f3=tog:{close,open}`・`19=tog:{close,open}`(7 セル×10) |
| `sc-keymatrix-e2-{msime,gji}-chrome`(2) | 実 Chrome × MS-IME / GJI | abs と同じ | `ctrl+1d=off:fresh`・`ctrl+1d=off:open`・`ctrl+1d=off:sync`・`1d=off:fresh`・`1d=off:open`・`ctrl+1c=on:fresh`(**E2**。msime が認定対象、gji が対照) |

行列のセル数: abs 8×3×2 + MS-IME の F2 3 = 51、tog 7×6 = 42、e2 6×2 = 12。計 **105 セル**(各 n=10 = 1,050 試行)。S-1 の再現構成(Imm32Unavailable × GJI/MS-IME × Ctrl+変換/Ctrl+無変換/単独タップ × 外からの反転)は `abs-chrome-{gji,msime}` と `e2-*-chrome` の `ctrl+1c=on:close`・`ctrl+1d=off:open`・`1c=on:close`・`1d=off:open` が該当する。
`sc-keymatrix-abs-tsf-gji` の Engine 経路(Ctrl+変換/無変換)は、**L3'(TsfNative×GJI)が未実装なので STUCK が出うる=既知の制限**(ADR-208 決定6。`ENGINE_PRESS_UNKNOWNS_APPLIED_IN_TSF_NATIVE=false`)。v2 の範囲は Chrome(Imm32Unavailable)と全窓の S-2 まで。

## 実行

```sh
# 全部(14 ジョブ)
gh workflow run e2e-ime.yml --ref ci/adr208-l3b-keymatrix -f only='sc-keymatrix-*'
# S-1 の本体(絶対キー)だけ / E2 だけ / 1構成
gh workflow run e2e-ime.yml --ref ci/adr208-l3b-keymatrix -f only='sc-keymatrix-abs-*'
gh workflow run e2e-ime.yml --ref ci/adr208-l3b-keymatrix -f only='sc-keymatrix-e2-*'
gh workflow run e2e-ime.yml --ref ci/adr208-l3b-keymatrix -f only='sc-keymatrix-abs-chrome-msime'
```
`only='sc-*'` や `only` 空の既定には含めない(plan の除外。`only='sc-*,sc-keymatrix-abs-tsf-gji'` のように `sc-keymatrix-` で始まる指定に合致した構成だけ入る)。plan を実コードで展開した結果: `only='sc-keymatrix-*'` = 14 ジョブ、`only='sc-*'` = 208 ジョブ・既定(only 空)= 248 ジョブ(いずれも新構成を含まず、この変更の前後で同じ)。matrix 上限 256 に対し余裕あり。
所要は1ジョブあたり abs 約15〜25分(自前窓)/約25〜35分(Chrome)、e2 は fresh セルが1試行約25秒なので約40分。`wait` は 1800 秒(e2 は 2400 秒)、ジョブの `timeout-minutes` は 80。結果は step summary の「ADR-208 L3b」表(セルごとの判定・1/2/3押下・固着・E2)と、各ジョブの `result.txt` の `KEYMATRIX_CELL:` 行。

## 受け入れ条件(決定7 (2))との対応

- 「S-1 の再現構成が各 n≥10 で、絶対キーは1押下、トグルは2押下で一致する」: セルごとに `n=10`(`--km-n`)。`meets_n`(ずれを作れた試行 ≥ 10)を出す。ずれを作れない試行が混じると made<10 になるので、足りないセルは `--km-n` を増やして再実行する(runs=1 の job を再実行するか `RUNS` ではなく `--km-n=` を構成側で上げる)。合否は verdict の `pass`(絶対=CONVERGED_1、トグル=CONVERGED_1/2)。
- 「MS-IME×Chrome の OFF は例外の対照(E2)付き」: `sc-keymatrix-e2-msime-chrome` の `fresh` セルと、`abs-chrome-msime`/`e2` の `open` セルの照合(上記)。既存の `sc-driftrecovery-ctrlmuhenkan-msime-chrome`(9/10)に対する対照を、同じ job・同じ窓・同じ時間帯で取る設計にした(時間帯依存〈C4〉と切り分けられる。別 job の fresh では揺れと区別できないため)。
- 昇格: 実測で全セルが pass(または ENV_EXCEPTION/GAP_NOT_MADE)になったら `expect=pass` へ上げる(現状は未実測のため observe。決定7 の「v2 ブロッカー」判定は実測後)。

## CI で作れない状態次元(Linux の全列挙だけが根拠、決定5(b))

- **S-2**(`is_japanese_ime=false`): 英語レイアウトへ切り替える操作を CI で安定して作れない。L2 の全列挙(0 件)が根拠。
- **S-3**(IC の no-op で Suppress なのに誰も書かない、L4): ImmCross の「読みが嘘」の窓を CI で作れない。D4 の全列挙のみ。
- **S-4**(`current_focus=None` 等の窓なし状態): 作れない。全列挙のみ。
- InputRelay の窓(L4/L5): 作れない。
- 物理キーボードそのもの(HKL・scan=0 の実物・常駐ソフトのフック): SendInput は物理 Ctrl 扱いの目印を付けるだけで、実キー経路は再現しない(D3 と同じ限界)。
- `applied` が「awase が書いた」のでなく「観測で得た」状態(ずれ(a)の純粋形): `close`/`open` は awase が書いた後に外から反転させる(applied=書いた値)ので、書いていない applied を作るセルはない(未実施の追加候補)。

## 既知のフレーク要因

- awase の WM_TIMER は約64ms刻み。注入ずれ(`inject.late_*`)と、単独タップ(KeyUp 確定またはタイムアウト)・Ctrl 付き注入の間隔(40/60/30ms)が判定に効く。
- Chrome の前面化失敗・アクセシビリティツリー再構築・TIP 初期化。`bring_to_front` に失敗すると `focus_lost` で INVALID。
- **MS-IME × 実 Chrome の環境揺れ**(時間帯依存、`v2-release-checklist` C4)。E2 は同じ job 内で取るので相殺されるが、job をまたぐ比較はしない。
- `fresh` はハーネスが `taskkill /F /IM awase.exe` して `awase.exe`(自分と同じフォルダ)を起動し直す。起動直後の startup-align や TIP 検出の揺れが入る(落ち着き待ち既定 8 秒〈自前窓〉/10 秒〈Chrome〉、`--km-fresh-settle`)。awase.log は追記なので、複数回起動の行が混じる(判定は awase.log の ctrl 行の `phys_ctrl` だけに使う)。
- 外から開く(`IMC_SETOPENSTATUS,1`)ことが MS-IME/TSF で効くかは未確認。効かなければ `open` のセルは GAP_NOT_MADE になる(自前窓は API で、Chrome は押す前の API で確認。Chrome の API が嘘をつくと誤って gap 成立と扱う恐れがある)。
- 構成が追加する awase の書き込み(再起動直後の startup-align、`close`/`open` 直後の drift correction、literal 回収)は押下の前に起きうる。`--km-wait`(既定1000ms)後の押す前の実 IME が r0 でなければ GAP_NOT_MADE にする。
- 既存の不変条件(`check_invariants.py`)は同じ awase.log を数えるので、再起動や外からの反転で i1/i2 等の件数が増えうる(合否ではなく別表。構成は observe)。

## 見送った/未実装の組み合わせ(追加候補)

- **F13 等の `keys.ime_on/off`**(分類表の行): 設定名の書式と、GJI/MS-IME が F13 を捨てるかの確認が要るため見送り。`ime_on` に `"VK_F13"` を足し `7c=on:close` を加えるだけで追加できる想定(scan 0x64 は `scan_of` に入れてある)。
- **MS-IME のかな/ひらがな以外の DBE キー**(F0 英数・F1 カタカナ): 開閉ではなくモード切替で意味が一致する/しないの定義が絶対/トグルに載らない。`f2=on:close` のみ。
- **tsf/edit 窓の E2**(fresh): 例外は実 Chrome だけなので省略。必要なら `km('sc-keymatrix-e2-msime-tsf', 'tsf', …)` を足すだけ。
- **GJI の MS-IME プリセット(keymap=2)× Chrome**: 指定の IME は「GJI(ATOK)と MS-IME 本体」なので省略。
- **ずれ C 「古い applied」の別手順**(awase に ON を書かせた後、実 IME を反転し、さらに時間を置く/フォーカスを挟む): `close` に `--km-wait` を長くする、`--refocus` 相当を足す拡張(D3 の `--refocus`)で作れる。
- **TN×GJI の `@` 件数**(BUG-124、L3' の A/B): `check_typing_stress.py` の `@` 検出を使う別構成。L3' の実機 A/B が条件で本件の範囲外。
- 自前窓の押下間の打鍵(Chrome と同じ観測): 交絡を避けるため入れていない。

## ハーネスの追加(awase 本体は無変更)

- `crates/awase-windows/examples/typing_stress/keymatrix.rs`(新規)と `main.rs`: `--mode=keymatrix --km-cells=… [--km-n --km-wait --km-max-press --km-fresh-settle]`。`edit`/`multi`/`rich`/`tsf` のみ(自プロセスの HIMC が要る)。
- `crates/awase-windows/examples/chrome_probe.rs`: `--keymatrix=…`(同じ書式)。`KM_CONFIG`/`KM` 行を出す。`scan_for` に 0x19 を追加。
- `tools/e2e/ime_key_matrix/check_keymatrix.py` + `test_check_keymatrix.py`(Linux の単体テスト、28 件)。
- `.github/workflows/e2e-ime.yml`: 構成の定義(`km` ヘルパー)、plan の除外、判定ステップ(`check=keymatrix`)、summary の表、artifact に `keymatrix.json`。
