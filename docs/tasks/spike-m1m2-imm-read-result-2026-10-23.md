# スパイク M1・M2: 読めない窓の IMM の読みは実状態と一致するか(2026-10-23)

ADR-279 段階 0 の (a2) の続きで、推奨案 P(明示 OFF の意図への反証に、IMM の「開」の読みだけを使う)が成り立つかを
GitHub の Windows CI で測った。設計メモの「追加で測るもの」の M1・M2 に当たる。

- 測ったのは読み取りだけ。本番コードの挙動は変えていない。
- 測定用の変更は `ci/spike-m1m2` ブランチだけにあり、develop には入れていない(下の「再現」)。
- 窓は CI の自前の窓だけ(実 Chrome の専用プロファイルの検証ページと、Windows Terminal の中の記録器)。dragonflyg4・利用者の本文・不具合報告は読んでいない。

## 結論

| 項目 | 結果 | 判定 |
|---|---|---|
| M1 (a) 実 IME が開のとき、柵の後の読みが `Some(true)` | 開の試行 200/200。柵の後の読み 14,728 件がすべて `Some(true)`。全試行で 3000ms 以上途切れず続いた | 成立 |
| M1 (b) 実 IME が閉の対照で、柵の後の偽の `Some(true)` | 閉の対照 262 試行・柵の後の読み 20,604 件で **0 件** | 成立(偽の「開」なし) |
| M1 (c) MS-IME の OFF が届かない場合の組 | 未確定の文字が残った状態で Ctrl+無変換を押すと、38/40 で `open=1`・`conv=0x10`(半角英数) | 案 P の「英数の軸もそろえる」が要る |
| M2 Windows Terminal の素の IMM の読み | GJI・MS-IME とも 120 試行・1,200 件の読みが実状態と全件一致(不一致 0) | 一致。WT にも P を広げられる |

**推奨案 P は成り立つ**。Chromium 系 × GJI/MS-IME で、開の読みは実状態に追随し、閉の対照で偽の開は 0 件だった。

WT も、クラス名の門前払い(`ime.rs::read_ime_state_full` の `is_tsf_native_window`)を外せば読める見込みが高い。
ただし門前払いのコメント(「TSF ネイティブ窓は imc_open=false を返す」)と食い違う。広げる前に、古い WT・実機で同じ測定をするのが安全(未確認)。

## M1: 実 Chrome(`Chrome_WidgetWin_1`、`Imm32Unavailable`)

### 条件

- run **39928093292**(`9519b82b`)と **39930760444**(`2c7e801b`)。全ジョブ success。
- ただし各 run の 1 ジョブは、Chrome のページが 40 秒で読み込まれず試行 0 件だった(基盤の揺れ。r1 の `gjimspreset-1`、r2 の `msime-1`)。
- ハーネス: `chrome_probe --m1=N`。awase(debug、`AWASE_TEST_INJECTION=1`)を起動した状態で、各試行を次の手順で行う。
  1. ensure でかなにそろえる(Engine=ON を打鍵で確認)。
  2. 目印付きの物理 Ctrl+無変換で、awase が明示 OFF にする。
  3. 試行の種類ごとに、400ms 後に次のことをする。
     - `open`: awase が自己注入として素通しする目印付きの `VK_IME_ON` で、実 IME だけを開け直す。明示 OFF・実 ON(BUG-110 型)になる。
     - `ctl`: 何もしない。
     - `ctlclose`(r2 だけ): `WM_IME_CONTROL`(`IMC_SETOPENSTATUS=0`)で外から閉じる。
  4. 4.4 秒の間、50ms ごとに **awase の prefetch と同じ関数** `awase_windows::ime::read_ime_state_full_with_timeout(300ms)` で読む。前面の窓の hwndFocus → `IMC_GETOPENSTATUS`/`IMC_GETCONVERSIONMODE` を、awase とは別のプロセスから読む。
  5. 最後に k,a を打ち、実状態を確かめる(`か`=開・かな、`ka`=閉)。
- 構成: `spike-m1-gji`(GJI・ATOK プリセット)、`spike-m1-gjimspreset`(GJI・MS-IME プリセット)、`spike-m1-msime`(MS-IME 本体)。それぞれ runs=2。
- 柵: 開け直し(閉じる操作)から 170ms(`KEY_EFFECT_SETTLE_MS`)。`ctl` は Ctrl+無変換の後から 170ms。

### 結果(実状態ごと。2 run の合計)

| 構成 | 種類 | 実状態(打鍵) | 試行 | 柵の後の読み | 読みの内訳 | conv |
|---|---|---|---|---|---|---|
| GJI ATOK | open | 開(か) | 80 | 5,927 | 全件 `Some(true)`、3000ms 継続 80/80 | 0x9 |
| GJI ATOK | ctl / ctlclose | 閉(ka) | 80 / 40 | 6,569 / 2,977 | 全件 `Some(false)` | 0x9 |
| GJI MS プリセット | open | 開 | 60 | 4,392 | 全件 `Some(true)`、継続 60/60 | 0x9 |
| GJI MS プリセット | ctl / ctlclose | 閉 | 60 / 40 | 4,885 / 2,932 | 全件 `Some(false)` | 0x9 |
| MS-IME 本体 | open | 開 | 60 | 4,409 | 全件 `Some(true)`、継続 60/60 | 0x19 |
| MS-IME 本体 | ctl / ctlclose | 閉 | 22 / 20 | 1,781 / 1,460 | 全件 `Some(false)` | 0x19 |
| MS-IME 本体(r1) | ctl | **開・半角英数**(`きうka`、IME が打鍵を処理=`Process`) | 38 | 3,115 | 全件 `Some(true)`・`conv=0x10` | 0x10 |

- 読みの失敗(`None`・時間切れ)は全体で 0 件。
- 開け直しから最初の `Some(true)` までは最大 53ms。ポーリングの間隔と、注入の 50ms の押下を含む。
- 柵の前(Ctrl+無変換の直後〜170ms)の読みも、閉の試行では全件 `Some(false)` だった。awase 自身の OFF の直後の古い「開」は見えなかった。
- **(c) MS-IME の OFF が届かない形**: r1 の MS-IME の `ctl` 38/40 は、Ctrl+無変換のあと実 IME が開いたまま半角英数(`conv=0x10`)になった。
  - これは偽の開ではない。打鍵が IME に処理されており、読みは実状態どおり。
  - r1 では ensure の確認打鍵の未確定文字(NICOLA の `きう`)が残ったまま Ctrl+無変換を押していた。r2 で Esc で取り消してからは 20/20 が閉じた。**未確定の文字があると、MS-IME × Chrome の OFF は閉じずに半角英数になる**(BUG-185 の形)。
  - 案 P1 で開の軸だけ採ると、この形で Engine が ON になる。設計メモ §2.2 のとおり、英数の軸(NATIVE ビット)もそろえる必要がある。

### 限界

- 読みは awase と同じ関数だが、ハーネスのプロセスから読んだ。awase の中の prefetch は、明示意図の間 refresh が予約されないので走っていない(段階 0 の S0-1)。案 P0 の予約を入れたあとの実際の読みは、P0 のログで確かめる。
- 実状態は試行の最後の打鍵で確かめた。読みの区間の途中で状態が変わっていないことは、読みが試行の中で一様だったこと(全件同じ値)で裏づけている。
- 実 IME を開け直す手段は、人工の自己注入の `VK_IME_ON` だけ。マウスでのインジケーター操作・他アプリからの変更は通していない。
- 「IME を外から開けない構成」は作っていない。対照は「開け直さない」「外から閉じる」の 2 通り。
- Edge・VS Code・Teams などの他の Chromium 系は測っていない(クラスは同じ `Chrome_WidgetWin_1` 系)。

## M2: Windows Terminal(InputSite)、awase なし

### 条件

- run **39928093294**(`9519b82b`)と **39930760358**(`2c7e801b`)。`wt-probe` の matrix `gji`・`msime`、全ジョブ success。
- ハーネス: `tools/e2e/wt/wt_probe.py` の相 M。awase は起動しない。
  1. WT の中で記録器(`wt_echo.ps1`)を動かす。
  2. `IME_ON`・`IME_OFF`・半角/全角(0x19)を順に押し、0.8 秒待つ。
  3. 100ms ごとに 10 回読む。読み方は awase の `read_ime_state_full` と同じ経路で、クラス名の門前払いだけを外している: WT の GUI スレッドの hwndFocus → `ImmGetDefaultIMEWnd` → `SendMessageTimeout(WM_IME_CONTROL, IMC_GETOPENSTATUS / IMC_GETCONVERSIONMODE, SMTO_ABORTIFHUNG, 50ms)`。トップレベルの hwnd 経由の読みも併記した。
  4. k,a,Enter をシェルへ送り、受け取った文字で実状態を確かめる(かな=開、英字=閉)。
- 1 run あたり各 IME 60 試行(開 40・閉 20)。

### 結果(2 run の合計)

| IME | 試行(開/閉) | 読み(hwndFocus) | 実状態との不一致 | トップレベル経由 | conv |
|---|---|---|---|---|---|
| GJI(ATOK プリセット) | 120(80/40) | 1,200 | **0** | 0 | 0x9(開閉で不変) |
| MS-IME 本体 | 120(80/40) | 1,200 | **0** | 0 | 0x19(開閉で不変) |

- hwndFocus のクラスは全件 `Windows.UI.Input.InputSite.WindowClass`。awase は今、これを `is_tsf_native_window` で `ime_on=None` にしている。
- 前面化の失敗は 0 件。読みの失敗(`None`)も 0 件。

### 限界

- WT の半角英数(conv の英数)は作っていない。conv の読みが英数に追随するかは未確認。
- awase を動かした状態の WT(明示 OFF の意図がある間)は測っていない。読みは IME の状態だけで決まり、awase の有無で変わる理由は無い見込み(未確認)。
- windows-latest の WT の版だけで測った。門前払いの元になった「TSF ネイティブ窓は imc_open=false を返す」が古い版・実機の WezTerm で起きていた可能性は、この測定では否定できない。WezTerm は測っていない。

## 再現

- ブランチ `ci/spike-m1m2`(origin/develop `ff68fe0f` から)。
  - `9519b82b`: `chrome_probe --m1=N`、`spike-m1-*` の 3 構成、`wt_probe.py` の相 M、`wt-probe.yml` の matrix(gji/msime)。
  - `2c7e801b`: `ctlclose` の対照と、Esc で未確定の文字を取り消す処理。
- 実行: ブランチへの push で `e2e-ime`(ONLY はブランチ名で `spike-m1-*` に固定)と `wt-probe` が動く。
- 集計: artifact `result-spike-m1-*` の `dist/chrome_probe.log` の `M1_READ`・`M1_TRIAL` 行と、`wt-probe-out-*` の `results.json` の `m2_trial`。
- develop には測定用の変更を入れていない。[one-off-verification-workflows](../../.claude/rules/one-off-verification-workflows.md) の撤去対象は develop には無い。ブランチ ref の掃除は持ち主が決める。
