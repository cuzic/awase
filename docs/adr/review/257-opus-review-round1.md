# ADR-257 Opus レビュー round1

対象: `docs/adr/257-scancode-map-vm-verification.md`(worktree adr257、HEAD `c21b8ea8`)。
前提として読んだもの: ADR-230・ADR-248、`origin/ci/scancode-vm-spike`(`1d2a21bb`)の `.github/workflows/scancode-vm-spike.yml`・
`tools/scancode-vm/vm-harness.ps1`・`tools/scancode-vm/probe/src/main.rs`、`crates/awase-windows/src/scancode_{map,pairs,apply}.rs`、
`crates/awase-settings/src/scancode_map_admin.rs`、run 38127199793 のログ(`gh run view 38127199793 --log`、headSha `1d2a21bb`、success)。

集計: Blocker 2・Must 6・Should 7・Nit 4。

---

## Blocker

### B1. 事実7(費用)と Q2 の前提が誤り: 3〜4分の大半は「WinPE の起動待ち」ではなく、準備完了の検出の空振り(150秒のタイムアウト)

run 38127199793 の4シナリオすべてで `probe ready (green screen): False after 150s`〜`152s`。つまり緑画面の検出は**一度も成功しておらず**、
各シナリオは `while ($sw.Elapsed.TotalSeconds -lt 150)` のタイムアウトを満了し、さらに `if (-not $ready) { Start-Sleep -Seconds 20 }`
(vm-harness.ps1 `Invoke-Scenario`)の固定待ちでキーを送っている。`VM shut down by WinPE: True after 175s`。
WinPE の実際の起動時間はこのログからは分からない(150秒より短い可能性が高い)。

影響:
- 事実7「1シナリオ約3〜4分(大半が WinPE の起動待ち)」と Q2「WinPE の起動待ち(約150秒)を短縮できるか」は、測っていないものを測ったと書いている。
  決定5(毎 push では回さない、約15分)の費用判断もこの数字に乗っている。
- 4/4 PASS は「固定20秒待ちで、たまたま probe の Raw Input 登録が間に合っていた」上の結果。起動が遅い回では、テストキーが probe の
  `RegisterRawInputDevices` より前に届いて消え、`vks` が短くなって **FAIL** になる(後述 M2 の「検証不能」に入らない)。
  Q1(失敗率)を測る前に、この偽 FAIL の発生源を潰しておかないと、Q1 の測定値が意味を持たない。

求める修正: 事実6/7 に「準備完了の検出は全回失敗、20秒の固定待ちで送った」と事実として書く。事実7 の内訳は不明と書き直し、
Q2 を「準備完了の検出を guest 側の確証に置き換える」に書き換える(具体案は S1)。

### B2. 「make は変換前・vk は変換後」の事実は ADR-230 の前提(「キーボードクラスドライバで効く」)と食い違うが、ADR-257 はそれを扱っていない

事実5 の観測(ログ: pair で `make=0x001E ... vk=0x42`)は、Raw Input が受け取る `KEYBOARD_INPUT_DATA`(kbdclass がユーザー側へ渡す値)が
**写像前**であることを示す。もし ADR-230 :84-88 のとおり Scancode Map が「キーボードクラスドライバで効き」「vk 変換より前に scancode を書き換える」なら、
Raw Input の MakeCode は写像後になっているはずである。観測と整合する解釈は「写像は kbdclass より上(win32k の入力処理、Raw Input の採取点より後、
vk 変換より前)で適用される」か「Raw Input は元の MakeCode を保持する仕様」のどちらかで、どちらにしても ADR-230 の記述は根拠を失う。

これは ADR-257 の「範囲と限界」の書き方に直結する:
- 限界3「VM のキーボードは仮想デバイスで、実キーボードの i8042prt/HID 経路と同じとは限らない」は、写像点がドライバ層にあるという前提の書き方。
  実際には **hyperkbd→kbdclass という i8042prt/kbdhid を通らない経路でも写像が掛かった**ことが、写像点がポートドライバより上にあることの直接の証拠になる。
  したがって限界として書くべきは「ポートドライバの違い」ではなく、「ポートドライバ/フィルタ(Ctrl2cap 等、kbdclass の UpperFilters)が出す値に対して
  上位で写像が掛かる、という合成順序は VM でも実機でも同じと推定される。ただし実機の HID 経路で同じ点に掛かることは未確認」。
- probe の doc コメント(`probe/src/main.rs` 先頭「Raw Input の `MakeCode` は、キーボードのドライバ層(Scancode Map の変換を含む)を通ったあとのスキャンコード」)は
  観測と逆のまま残っている。段階1で本番化する前に直す(Nit ではなく、この誤りが判定基準の誤りを生んだ実績があるので Blocker の一部とする)。
- ADR-230 :84-88(SAS と押しっぱなしの推論の根拠)と ADR-248 :171-173(「kbdclass はセーフモードでも読み込まれるので入れ替えは効いたままの可能性が高い」)が
  同じ前提に立っている。ADR-257 から「この観測で ADR-230 :84-88 / ADR-248 :171-173 の推論の前提が変わる」と明示的に参照し、両 ADR に追記の要否を Q として立てる。

求める修正: 事実5 に「解釈」と「観測」を分けて書く(観測: Raw Input の MakeCode は写像前、VKey は写像後。解釈は未確定)。
写像点を確定させる追加観測(M1)を段階1の前提にする。

---

## Must

### M1. 判定点を Raw Input の `vk` 1つにすると、決定3が挙げる修飾キー・拡張キーを判定できない。LL フックの観測を probe に足す

`RAWKEYBOARD.VKey` は左右を区別しない(`VK_CONTROL`/`VK_MENU`/`VK_SHIFT` を返す)。左右の区別は `Flags` の E0 で行うが、事実5 によれば
`make`/E0 は写像前の値なので、**「左 Ctrl の位置が右 Ctrl を出すか」「Caps の位置が左 Ctrl を出すか(右ではなく)」は Raw Input では判定できない**。
決定3 は「Caps への追加 Ctrl・拡張キー(右 Ctrl、Alt)」を表に入れると言っているので、決定2「判定は Raw Input の vk」と矛盾する。

さらに awase 本体の判断材料は Raw Input ではなく `WH_KEYBOARD_LL` の `KBDLLHOOKSTRUCT.scanCode`/`vkCode`/`LLKHF_EXTENDED`
(`hook.rs`、親指ラッチは scan_code+拡張ビットで同定する: fix-requires-evidence.md「物理キー押下ラッチ」行)。awase にとって意味がある問いは
「Scancode Map 後に、フックが見る scanCode と拡張ビットは何か」であり、Raw Input の make はその代わりにならない(B2 のとおり写像前)。

求める修正: probe に `SetWindowsHookExW(WH_KEYBOARD_LL)` を足し、1打鍵ごとに Raw Input(make/E0/VKey)と LL フック(scanCode/flags/vkCode)の
両方を記録する。判定は LL フックの `vkCode`(左右区別あり)を主、`scanCode`+`LLKHF_EXTENDED` を副にする。WinPE で LL フックが動くかは
段階1の最初の確認項目(win32k はあるので動く見込み、未確認)。これで B2 の写像点も「LL フックの scanCode が写像後なら win32k 内で写像」と確定できる。

### M2. 「検証不能」と FAIL の区別が、現行ハーネスでは全経路で FAIL に潰れる。判定前の前提条件を列挙して ADR に書く

現行 `vm-harness.ps1` の失敗の流れ:
- `$ErrorActionPreference = 'Stop'` 下で `Start-VM`/`New-VM`/`Mount-VHD`/`dism` が例外 → スクリプト全体が停止し、残りのシナリオは走らず、`SUMMARY` も出ない。
  これは「検証不能」だが exit は非0で FAIL と区別できない。
- `Read-ProbeLog` が `(no probe.log)` → `$vks` が空 → 期待と不一致 → `FAIL`。
- `Send-Key` は `PressKey` の例外を握りつぶし(F12 用の catch が全キーに掛かっている)、`ReturnValue` が非0でも続行 → 送れなかったキーは「出なかった」として `FAIL`。
- probe が準備前に来たキー(B1)→ `FAIL`。

求める修正(決定7 に表で書く): 判定の前提条件をすべて満たしたときだけ PASS/FAIL を出し、1つでも欠ければ「検証不能」。最低限:
1. VM が起動し、`probe.log` に `START` と `READY` がある。
2. 送ったすべてのキーで `PressKey`/`ReleaseKey` の `ReturnValue` が 0(F12 以外)。
3. 送ったキーの数と、log の break=0 行の数が一致する(多い=オートリピート等、少ない=取りこぼし。どちらも検証不能)。
4. **各打鍵の写像前の値(Raw Input の make、または M1 の観測で写像前と分かった値)が、送ったつもりの from スキャンコードと一致する**
   (現行は make を一切見ていない。Hyper-V の VK→スキャンコード変換が想定と違えば、そのシナリオは別のキーを試験していることになる)。
5. 同じ run の対照(control)シナリオが PASS。対照が FAIL なら、その run の全シナリオを検証不能にする(番兵)。

前提条件がそろった上で `vk` が期待と違うときだけ FAIL。

### M3. 「検証不能」を GitHub Actions のどの結果に写すかが決まっていない。緑に写すと、検証が黙って消える

Actions のジョブ結果は success/failure/cancelled しかない(neutral は Check Run API を直接使う場合だけ)。決定7 は区別すると言うが、
- 検証不能を success+警告注釈にすると、ホスト型ランナーの入れ子仮想化が将来使えなくなったとき(決定7 自身が「保証された機能ではない」と言う)、
  ワークフローは永久に緑のまま何も検証しなくなる。パスフィルタで稀にしか走らないので、気づくのはさらに遅れる。
- failure にすると FAIL と見分けがつかない(決定7 の目的に反する)。

求める修正: 写像を決める。案: 検証不能は1回だけ VM 再作成で再試行(FAIL は再試行しない。再試行で FAIL を PASS に変えない)、
再試行後も検証不能なら failure にし、ジョブサマリの1行目で `INCONCLUSIVE`(理由: 前提条件の番号)と書く。
加えて S3 の定期実行で「検証不能が N 回連続」を検出する。

### M4. 決定3 の生成元は `build_bytes` ではなく `decide`(→ `compute_swap_write_with_cycles`)にする。`build_bytes` 単体は Linux の単体テストで足りている

本番で書かれるバイト列は `scancode_map_admin.rs::run_elevated_pairs_worker_windows` の
`decide(original, &request)` → `Decision::Write { entries }` → `sm::build_bytes(&entries)` で作られる。`build_bytes` 自体は 15 行の直列化で、
`parse_entries` との往復テストで固定済み。VM が付加価値を出せるのは、
- `compute_swap_write_with_cycles` の**出力順**(既存保持分→新規ペア→Caps 追加 Ctrl)と OS の解釈の組み合わせ、
- 他ツールのエントリを保持したまま書いたとき(`expected` に既存エントリを入れる)の実効の写像、
- ADR-230 :207-208「同じ from のエントリが複数あるとき OS がどちらを効かせるか(先勝ちか後勝ちか)」——`detect_swap_pairs` の
  「入力順で最初のものを採る」(`scancode_pairs.rs:266`)と `revealed` の警告文(`scancode_pairs.rs:963` のコメント「OS の先勝ち/後勝ちは未確認」)が
  この未確認に依存している。**ADR-230 の未確認リストで VM が答えられる最も価値の高い項目**なのに、ADR-257 の決定3・範囲のどちらにも無い。

求める修正: 決定3 を「`ApplyRequest` と既存値(`existing_raw`)を `decide` に通した `Decision::Write` の entries を `build_bytes` で書く」に直し、
シナリオ表に (a) 既存の他ツールエントリを保持したまま書く、(b) from 重複(手書きバイト列。`decide` は作らない)で先勝ち/後勝ちを確かめる、を足す。
(b) の結果は ADR-230 の未確認リストと `scancode_pairs.rs:963` のコメントへ反映する(段階3)。

### M5. 期待値の導出がトートロジーになる経路を具体的に塞ぐ

トートロジーの危険は `build_bytes` ではなく、その上の**意味論の層**にある:
- `Cycle::entries` が `(keys[i], keys[i+1])` を返すのは「`keys[i]` の位置が `keys[i+1]` の機能を出す」(`scancode_pairs.rs:93` の doc)という UI の約束の実装。
  期待値を `permutation_entries`/`Cycle::entries`/`decide` の出力から「(from,to) なら from の位置で to が出る」と導くと、entries の向きが UI の約束と
  逆に実装されていても、書き込みも期待も一緒に逆になり PASS する。(from,to) の OS 上の意味はスパイクで一度確定したので、以後 VM で毎回検証する価値は
  「UI の約束 → entries」の向きのほうにある。
- 期待の vk を「to スキャンコード → vk」の表で出すとき、その表を `awase-vkmap` から取ると、`awase-vkmap` のスキャンコード定数の誤りが
  書き込み側(UI が選んだキーのスキャンコード)と期待側で相殺される。

求める修正(決定3 に書く):
1. 期待は、シナリオ表に**人が読める形で手書きする**(例: 「循環 [A,S,D] を作った → A の位置を押すと S が出る」)。`entries()`/`decide` を通さない。
   生成器は `ApplyRequest` → `decide` → `build_bytes` の側だけに使う。
2. 期待の vk は表で持たず、**同じ run の対照シナリオで実測する**: 対照で各キー(from と to の両方)を押して `送った VK → (写像前の値, vk)` を記録し、
   写像シナリオの期待 vk = 対照で「写像前の値 = to」だったキーの vk とする。WinPE のレイアウト(US/JIS)にも依存しなくなる(限界4 の影響も縮む)。
   M2 の前提条件4(写像前の値の一致)もこの対照表で照合する。

### M6. 決定5 のパスフィルタが、実際に書く値を決めるファイルを外している

決定5: `scancode_map*`、`scancode_pairs*`、`tools/scancode-vm/**`、本ワークフロー。外れているもの:
- `crates/awase-windows/src/scancode_apply.rs`(`decide`。M4 で生成元にするなら必須)。
- `crates/awase-settings/src/scancode_map_admin.rs`(昇格ワーカー。M4 の経路の殻)。
- 生成器とシナリオ表(段階2で置く場所。S4 で golden にするならそのファイル)。
- `crates/awase-windows/src/scancode_editor.rs`(UI が `ApplyRequest` を組む側の判断。`QUICK_PAIRS` 等)——M5 の「UI の約束」を試すなら対象。

求める修正: パターンをディレクトリやモジュール名でなく、上記ファイルを列挙して書く。`scancode_*` に広げるなら `scancode_diagram.rs` も入ることを明記する。

---

## Should

### S1. 準備完了を guest 側の確証で取る(B1 の解決案)

現行: probe は `CreateWindowExW(... WS_VISIBLE ...)` で窓を出した後に `RegisterRawInputDevices` を呼ぶ(`main.rs` の `main`)。仮に緑の検出が
成功しても、緑が出た時点で Raw Input は未登録でありうる(競合)。
案: (1) probe は Raw Input(と M1 の LL フック)の登録が終わってから窓を表示する。(2) ハンドシェイク: ホストは番兵キー(写像の対象外、例 F11)を
数秒おきに送り、probe はそれを受け取ったら窓の色を変える(例: 緑→青)。ホストは色の変化を見てからテストキーを送る。log では番兵の打鍵を判定から除く。
これで「キーボード経路が生きている」ことが判定前に確定し、M2 の前提条件1・2の大半が事前に分かる。
緑の検出が全回失敗した原因(サムネイルの RGB565 のビット配置の仮定 `vm-harness.ps1` `Save-Screen` のコメント「青が上位ビット」、
160×120 への縮小で窓が何 px になるか、`$count -ge 50` の閾値)は、`$ShotDir\*-ready.png` の artifact で1回確かめてから ADR に書く。

### S2. `PressKey` で送れるキーの範囲が、awase の主用途(JIS キー)に届くかを Q3 より広く問う。`TypeScancodes` の不在は再確認する

ADR-230 の許可リストとワンクリック(「変換 ⇄ スペース」、親指キー=無変換/変換、かな、半角/全角)の中心は JIS 専用キー(0x7B・0x79・0x70・0x29・0x73・0x7D)。
`Msvm_Keyboard.PressKey` は VK を受け取り、Hyper-V ホスト側の VK→スキャンコード変換で送る。この変換が JIS キーの VK(`VK_NONCONVERT` 等)を
どのスキャンコードにするか(そもそも送れるか)は未確認で、送れなければ決定3 のシナリオの半分が作れない。Q3 は拡張キーしか挙げていない。
また事実1「`TypeScancodes` は無い」は、公開されている `Msvm_Keyboard` クラスの定義には `TypeScancodes(uint8[] Scancodes)` があるはずなので、
`Get-CimClass -Namespace root/virtualization/v2 -ClassName Msvm_Keyboard | Select-Object -ExpandProperty CimClassMethods` の出力を ADR に貼って確定させる。
あれば E0 付きや JIS キーを生のスキャンコードで送れ、Q3 と M2 前提条件4 がまとめて解ける。
求める修正: Q3 を「JIS 専用キー・E0 付きキーを、どの方法(PressKey の VK/TypeScancodes)で、どのスキャンコードとして送れるか」に広げ、段階1の最初に測る。

### S3. 環境の劣化を検出する定期実行を足す(パスフィルタだけでは足りない)

windows-latest のイメージ更新(Server 2025 の累積更新・Hyper-V の変更・choco の `windows-adk-winpe` パッケージの版変更)は、awase のコードが
変わらなくても壊す。パスフィルタだけだと、数か月後に scancode 系を触った PR が初めて壊れたインフラを踏み、自分の変更のせいだと誤認する。
求める修正: 決定5 に週1〜月1 の `schedule` を足す(費用は 4 シナリオで月数回)。M3 の「検証不能 N 回連続」の検出もここで行う。

### S4. 段階2の生成器を「Linux の golden + VM はチェックイン済みの表を読む」に分けると、ワークフローで awase-windows をビルドしなくて済む

ADR は生成方法を書いていない。VM ワークフローの中で `awase-windows`(windows クレート依存、Win32 全体)をビルドすると、キャッシュ無しで数分、
キャッシュ有りでも 1 分以上かかり、事実7 の費用に入っていない。また PowerShell ハーネスから Rust の `decide` を呼ぶ接着が要る。
案: `crates/awase-windows` に `#[test]` で「シナリオ表 → `decide` → `build_bytes` の16進」を `tools/scancode-vm/scenarios.json` と比較する golden を置く
(Linux の `cargo test` で走る。`scancode_*` は `#[cfg(windows)]` でない純粋部分)。VM ワークフローはその JSON を読むだけにする。
ハーネスの改変(手書き → 生成)と生成物の正しさの検証が分離され、M6 のパスフィルタにも JSON を入れれば済む。

### S5. 1シナリオ1起動の費用の線形増加への手当てを決めておく

決定3 の表(ペア・循環両向き・Caps 追加 Ctrl・拡張キー・M4 の保持と from 重複・S2 の JIS キー)を素直に並べると 10〜15 シナリオで、
現行の 1 シナリオ約 3.3 分だと 35〜50 分(B1 を直せば短くなる見込みだが未測定)。案: (a) 互いに素なキー集合の写像は1回の起動にまとめる
(例: ペア A⇄B と循環 [C,D,E] を同じマップに入れ、全キーを押す)。ただし from 重複の検証のように他と干渉するシナリオは単独にする。
(b) ランナー(4 vCPU/16GB)で VM を2〜3台並列に起動する(各 1GB)。どちらを採るかと、上限の分数を決定5 に書く。

### S6. オートリピートと打鍵間隔の扱いを決める

`Send-Key` は `PressKey` → 150ms → `ReleaseKey` だが、間に `Get-CimInstance`/`Get-CimAssociatedInstance` の WMI 往復が挟まり(キーごとに再取得)、
入れ子環境の負荷次第で実際の押下時間は 150ms より長くなりうる。仮想キーボードがタイプマティックを出す場合、break=0 の行が増えて現行の比較は FAIL になる。
M2 の前提条件3で検証不能に落とすか、比較を「同じ make の連続した break=0 を1回に畳む」にするかを決める。押下時間の実測(log にタイムスタンプを足す)も入れる。

### S7. 「範囲と限界」に、ADR-230/248 の未確認項目との対応表を足す

読み手が「どの未確認がこの ADR で片付き、どれが実機に残るか」を引けるように、ADR-230 :205-210 と ADR-248 の Q2/Q3 を1行ずつ並べる:
- 3巡回の向き・1回だけの適用: 済(スパイク)。
- from 重複の先勝ち/後勝ち: VM で可(M4)。
- `0x003A` を X と入れ替えたときの Shift 分岐(`VK_CAPITAL`): JIS レイアウト(kbd106)を WinPE に入れられれば VM で可。入れられなければ実機。
  なお Raw Input の VKey が Shift 依存の変換後の値を返すかは未確認なので、M1 の LL フックの vkCode で見る。
- SAS(Ctrl+Alt+Del): WinPE に Winlogon の SAS 経路が無いので不可、実機。
- ADR-248 Q2(再起動と「シャットダウン→電源オン」で効きが違うか、高速スタートアップ): WinPE では不可、実機。
- ADR-248 Q3(セーフモード・画面キーボード・WinRE でのオフライン削除の手順): WinRE のオフライン削除の手順(`ControlSet00N` の N)だけは、
  このハーネスの `reg load` と同じ操作なので VM で手順の確認ができる。残りは実機。
- Ctrl2cap との重なり: Ctrl2cap は kbdclass の UpperFilters ドライバで、WinPE にもドライバを入れて試せなくはないが、費用に見合わないので実機(B2 の合成順序の推定を書く)。

---

## Nit

- N1. 事実6「SSH で入ったランナー上の再実行で同一」は、検証の手段(run ID か、ログの保存場所)が書かれていない。agent-handoff.md 3 の書式に合わせ、
  run 38127199793 には headSha `1d2a21bb` を添える(`gh run view 38127199793 --json headSha` で確認済み)。SSH 再実行は「未記録」と書くか、ログを artifact に残す。
- N2. 事実2「SecureBoot は Off」は理由(WinPE の boot manager は Microsoft 署名なので On でも起動するはず、On で失敗したのか試していないのか)を1語添える。
  本番化で On に戻せるなら、その方が実機の構成に近い。
- N3. 決定6「スクリーンショットのみ artifact」: S4 の JSON と probe.log(打鍵の記録だけで、イメージ由来の内容を含まない)も artifact に付けると、
  失敗の再解析が run のログ文字列からの grep に頼らずに済む。ADK のファイルを `actions/cache` に入れる場合、公開リポジトリのキャッシュは fork の PR の
  ワークフローからも読めることを決定6 の文脈で一言書く(公開しないの範囲)。
- N4. probe の終了条件 `k.MakeCode == 0x58`(F12)は make が写像前であることに依存している(事実5)。シナリオで F12 の位置を写像しても終了できる
  という性質を、偶然ではなく意図として probe のコメントに書く(B2 の doc コメント修正と同時に)。
