# ADR-282: IME の開閉・変換モードは窓(スレッド)ごとか、全体で共有か(CI 実測、2026-10-26)

ADR-282 D1(belief のスコープの単位)の材料。結果は状態の集計だけで、打鍵した文字列・ログの本文・利用者のデータは含まない
(打つのは固定の `ka` の 2 打だけ。結果は「かな(か)」「英字のまま(ka)」の 2 値に分類)。

## 方法

窓 A(別プロセス)で IME の状態を作って保持し、**別プロセスの新しい窓 B(新しいスレッド)**で `ka` を打って Enter で確定し、結果を読む。
awase は起動しない(IME 単体の性質)。状態はキーだけで作る(IMM 書き込みで作らない)。

| セル | 内容 | 読み方 |
|---|---|---|
| ctl_direct | A なし。B で何もせず `ka`(各セルの前に「IME OFF にして終了する EDIT 窓」で状態をそろえる) | 期待は `ka`(B は閉で始まる) |
| ctl_on | A なし。B で ON キー(GJI=VK_IME_ON、MS-IME=ひらがなキー)を 1 回押して `ka` | 期待は か(B で IME が効く) |
| s1 | A=開×ひらがな(OFF→ON)。B で何もせず `ka` | か なら B は A の「開」を引き継いだ(共有) |
| s3 | A=開×半角英数(ON のあと英数キー)。B で ON キー→`ka` | `ka` なら B は A の「半角英数」を引き継いだ(共有)。か なら独立 |

A は自分でも `ka` を打って状態を自己検証する(s1 は か、s3 は `ka` のはず)。合わない試行は集計から除く。
判定(`tools/e2e/ime_scope/scope_pure.py`): `open` = ctl_direct が `ka` で s1 が か → shared-open、s1 も `ka` → independent。
`conv` = ctl_on が か で s3 が `ka` → shared-conv、か → independent。ctl が期待と違えば inconclusive。

A/B の種類: **edit**(`typing_stress` の自前 Win32 EDIT。IMM で読める窓)、**chrome**(実 Chrome のテキスト欄。Imm32Unavailable)、
**wt**(Windows Terminal。tsf-native。入力は `wt_echo.ps1` の ReadKey の文字コードで読む)。

## 環境

- GitHub Actions `windows-latest` = **Windows Server 2025 Datacenter、build 26100**。`SPI_GETTHREADLOCALINPUTSETTINGS = 0`(入力設定はユーザー全体で共有する既定)。
- IME: GJI(chocolatey の最新、ATOK プリセット)と Microsoft IME 本体(ja-JP を追加。TSF)。それぞれ別ジョブ。
- 試行: 第 1 回は各セル 2 回、WT を含むセルの再測定は各セル 6 回(待ち時間を延ばし ON キーを 2 回押す)。

## 結果

### 第 1 回(run 40327981441、head ef6e3d4e0。ジョブ `scope (gji)` 121589557165・`scope (msime)` 121589556404。各セル 2 回)

向きは **A→B**(A で状態を作り、B で読む)。

| A→B | GJI open | GJI conv | MS-IME open | MS-IME conv |
|---|---|---|---|---|
| edit→edit(別プロセス) | independent(2/2) | independent(2/2) | independent(2/2) | independent(2/2) |
| edit→chrome | independent(2/2) | independent(2/2) | independent(2/2) | independent(2/2) |
| chrome→edit | independent(2/2) | independent(2/2) | independent(2/2) | independent(2/2) |
| edit→wt | 不定(mixed) | 不定(ctl_on=mixed) | shared-open(2/2) | independent(2/2) |
| wt→edit | independent(A の自己検証 OK の 1/1) | independent(1/1) | independent(2/2) | independent(2/2) |

edit/chrome の組では ctl_direct=`ka`(2/2)・ctl_on=か(2/2)で、判定の前提(B は閉で始まり、ON キーが効く)が成立している。
「independent」は、A を開(ひらがな/半角英数)にしても B は閉で始まり、ON キーを押すとひらがな(半角英数は引き継がない)という意味。

### 再測定(WT を含むセルだけ。run 40397245969、head 8f80eec2e。ジョブ `scope (gji)` 121833875179・`scope (msime)` 121833875821。各セル 6 回)

条件: WT を前面化してから 3 秒待つ、ON キー後 2.5 秒待つ、ON キーを 2 回押す。**測ったのは edit→wt と wt→edit の 2 つの向きだけ**
(wt が B のときの対照 ctl_direct・ctl_on は edit の A を使う。wt→edit の B=edit の対照はこの run では回していないので第 1 回〈2/2〉を使う)。

| A→B | IME | open | conv | ctl_direct | ctl_on | s1 | s3 |
|---|---|---|---|---|---|---|---|
| edit→wt | MS-IME | **shared-open** | independent | `ka` 6/6 | か 6/6 | **か 6/6** | か 6/6 |
| edit→wt | GJI | **不定** | 不定 | `ka` 4/6、か 2/6 | か 3/6、`ka` 3/6 | か 3/6、`ka` 3/6 | か 3/6、`ka` 3/6 |
| wt→edit | MS-IME | independent | independent | (第 1 回: `ka` 2/2) | (第 1 回: か 2/2) | `ka` 6/6 | か 6/6 |
| wt→edit | GJI | independent(A の自己検証 OK の 3/3) | independent(3/3) | (第 1 回: `ka` 2/2) | (第 1 回: か 2/2) | `ka` 3/3(他の 3 回は A の自己検証が合わず除外) | か 3/3 |

- **MS-IME × WT(edit→wt)**: ctl_direct は 6/6 で `ka` なのに、A=edit を「開」にすると B=WT は何も押さなくても か(6/6)。**WT の B は、別プロセスの A の「開」を引き継ぐ**。
  一方で変換モードは引き継がない(A=半角英数でも、B で ON キーを押すと か 6/6)。逆向き wt→edit(B=edit)では開も引き継がない(`ka` 6/6)。**向きで非対称**。原因は未解明(仮説: WT は TSF ネイティブで自分のスレッドの状態を持たず、直前に変更された状態を読んでいる。未検証)。
- **GJI × WT は不定**: セルの結果が回ごとに入れ替わる。ctl_direct は round 2・4 で、何も押していない B が最初から開(か)、ctl_on は round 1・3・5 で ON キーを 2 回押しても閉のまま(`ka`)。
  s1・s3 も同じ回で変わる(round 0・2・4 は か、1・3・5 は `ka`)。偶数/奇数の回で入れ替わるので、直前のセルの状態の持ち越し(リセットの効き方)か WT 内での ON キーの受け方のどちらかと思われるが、切り分けられていない。
  wt→edit の A=WT は、round 1・3・5 で A の自己検証が合わず(hira 状態で `ka`、eisu 状態で か)、同じ入れ替わりが A 側にも出ている。
- WT の B が前面化・フォーカスできていたかは、`wt_echo` が READY を出し、キーが 1 打以上記録されていること(全 cell で確認)まで。IME の状態の取りこぼしと区別はしていない。

## 実機(dragonflyg4、所有者の測定の報告)

条件: Windows 11 build 22631、GJI、**実機に `wt.exe` と Chrome は無い**(ブラウザは Edge)。**edit→edit、edit→chrome(Edge)、chrome(Edge)→edit は independent**
(ブランチ `exp/ime-scope-realmachine`。私は実機を測っていないので、結果は所有者側の報告のとおり記す。試行数・一致率はこの文書に転記していない)。WT・WezTerm・UWP は実機で未測定。
CI の edit/chrome の組(GJI、独立)と同じ結論で、Windows Server と Windows 11 の差は、この 3 組では出ていない。

## 限界

- CI は **Windows Server 2025**(build 26100)。所有者の実機は Windows 11(build 22631)の Home/Pro。IME のバージョン差(GJI の版、Microsoft IME の版)は見ていない。
- 「読める窓/読めない窓」は、A/B の種類(edit=IMM で読める、chrome=Imm32Unavailable、wt=tsf-native)で区別しただけで、awase の判定の通りかは確認していない。
- 人間の操作(マウス、タスクバーの入力インジケーターからの切替、Alt+Shift など OS のホットキー)による切替は含まない。キー(SendInput)だけ。
- B は毎回**新しいプロセス**(新しいスレッド)。同じスレッドの別 hwnd・同じプロセスの別スレッドは測っていない(ADR-191 §2 の `edit2`/`top2`/`thread2`〈自前の EDIT のみ〉を参照)。**chrome→chrome は測っていない**(同じプロファイルの Chrome は 1 プロセスに集まるため)。
- WT は edit との 2 つの向きだけ(wt→chrome・chrome→wt・wt→wt は未測定)。GJI × WT は不定のまま。WezTerm・UWP/XAML は測っていない(CI に用意していない)。
- 各セル 2〜6 回(実機は報告の範囲)。回ごとの入れ替わりがある組(GJI × WT)の一致率は、標本が小さく、結論に使えない。
- ctl 以外の構成要因(ActivateProfile を B のプロセスでも呼ぶ。edit/chrome の B は `typing_stress --activate-gji/--msime`)が B の初期状態に影響しうる。ctl_direct が `ka`、ctl_on が か なので edit/chrome の B では影響は出ていないが、WT の B(プロファイルの有効化は WT 側に任せる)とは条件が違う。

## ADR-282 D1(スコープの単位)への含意(1 行ずつ)

1. 別プロセス・新しいスレッドの edit/chrome(読める窓・Imm32Unavailable)は、GJI・MS-IME とも開閉・変換モードを**引き継がず独立**(新しい窓は閉で始まる)。スコープの単位は窓ではなくスレッド(プロセス)で足りる。
2. 変換モード(半角英数)は、測れた全構成(WT × MS-IME を含む)で**引き継がれない**。「半角英数のまま別の窓へ」は belief の根拠にならない。
3. **MS-IME 本体 × WT の B は A の開を引き継ぐ**(edit→wt のみ、6/6)。向き(wt→edit)は独立。tsf-native の窓の開閉を「窓ごとに閉で始まる」と置く前提は、この組には当てはまらない可能性があり、D1 は「窓の種類によりスコープが違う」を許す形にする(仮説は未検証)。
4. GJI × WT は結果が定まらない。WT の開閉は、belief ではなく実状態の観測(または L1 のような局所の根拠)に頼る側に倒すのが安全(測定からは共有も独立も言えない)。
5. 実機(Windows 11、GJI)でも edit/Edge の 3 組は独立。Windows Server との差は出ていない。ただし WT・Chrome は実機で未測定。

## 再現

一回限りの測定なので、workflow・スクリプトはこの PR には含めない(`.claude/rules/one-off-verification-workflows.md`)。ブランチ `ci/adr282-ime-scope`(ref は残す)に次のコミットで入っている:

- `b96e29b9d`(追加)→ `ef6e3d4e0`(taskkill を自プロセスだけに)→ `ab59a4f27`・`8f80eec2e`(WT 再測定用の一時の引数)→ `cc62195d6`(引数を既定に戻す)。
  ファイル: `.github/workflows/ime-scope.yml`、`tools/e2e/ime_scope/{scope_probe.py,scope_pure.py,test_scope_pure.py}`、`crates/awase-windows/examples/typing_stress/scope.rs`(+ `main.rs` の `mod scope;` と `--mode=scope` の分岐)。
- 再実行: `git checkout ci/adr282-ime-scope`(または上のコミットを develop に cherry-pick)して `ci/adr282-ime-scope` ブランチへ push すると `ime-scope.yml` が動く(GJI・MS-IME の 2 ジョブ、各 約 30〜50 分)。
  既定は `--pairs "edit>edit,edit>chrome,chrome>edit,edit>wt,wt>edit" --rounds 2`。WT の再測定は
  `--pairs "edit>wt,wt>edit" --rounds 0 --wt-rounds 6 --wt-settle 3 --on-wait 2.5 --on-times 2`(pwsh では `>` をリダイレクトにしないよう `--pairs` の値を引用符で囲む)。
- 結果の artifact(`ime-scope-gji`・`ime-scope-msime`、保持 14 日 = 2026-11-09 まで): `out/results.json`(全試行の記録)・`out/summary.md`・`out-console.log`(`SCOPE:` の判定行)。
- 各 `SCOPE:` 行の判定の生データはこの文書の表に転記済み。
