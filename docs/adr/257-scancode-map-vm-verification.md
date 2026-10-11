---
id: ADR-257
title: |-
  Scancode Map の効きを、GitHub ホスト型 Windows ランナー上の入れ子 VM(WinPE)で実機相当に検証する
summary: |-
  ADR-230/248 の「キーの入れ替え」は、レジストリの Scancode Map を書いて再起動すると効く。単体テストで固定できるのはバイト列の組み立てまでで、「OS が実際にそのバイト列で入れ替えるか」は実機でしか確かめられなかった。
  スパイク(ブランチ ci/scancode-vm-spike)で、windows-latest のホスト型ランナーが入れ子 Hyper-V VM を起動でき、WinPE の SYSTEM ハイブへオフラインで Scancode Map を書いて起動し、VM のキーボードへ PressKey でキーを送り、VM 内の Raw Input の `vk` を読めることを確認した(対照・ペア・3巡回の両向きの4シナリオが PASS)。
  本 ADR は、これを本番のワークフローにする方針を決める。判定点(Raw Input の vk は左右を区別しないので LL フックを足す)、「検証不能」と FAIL の区別、シナリオと期待値の出どころ(トートロジーを避ける)、範囲と限界を扱う。
status: |-
  起草 r2(2026-10-11)。Opus round1(Blocker 2・Must 6・Should 7・Nit 4)を反映。スパイクは GitHub ホスト型で検証済み。本番ワークフロー(段階1)は未実装。
related_adr:
  - "ADR-230"
  - "ADR-248"
---

# ADR-257: Scancode Map の効きを入れ子 VM(WinPE)で検証する

## 背景

ADR-230/248 の画面は、`HKLM\SYSTEM\CurrentControlSet\Control\Keyboard Layout\Scancode Map` を書き、**再起動後**に効く。

- 単体テストで固定できるのは「バイト列の組み立て」まで。
- 「OS がそのバイト列でキーを入れ替えるか」——from/to の向き、1回だけの適用、from 重複のときの先勝ち/後勝ち(ADR-230 の未確認)、左右の修飾キー、Caps への追加 Ctrl——は実機でしか確かめられず、ADR-230 段階5 は所有者の実機確認待ちのまま残っている。
- 再起動が要るため、ホスト型ランナーの OS 自体では試せない。

## スパイクで確かめたこと(事実)

2026-10-11、`ci/scancode-vm-spike`(`.github/workflows/scancode-vm-spike.yml`、`tools/scancode-vm/`)。

1. windows-latest(Windows Server 2025)は Hyper-V が有効で、**入れ子の Gen2 VM を起動できる**。VM のキーボードは WMI `Msvm_Keyboard` の `PressKey`/`ReleaseKey`(VK を渡す)で操作できる。当初 `TypeScancodes` は無いと判断したが、クラス定義の列挙で確かめていない(Q3 で確定する)。
2. WinPE は ADK の WinPE アドオン(`choco install windows-adk-winpe`、約150秒)の `boot.wim` と `Media\*` から、**FAT32 の起動用 VHDX**(ESP)に置いて起動できる(oscdimg が付かないので ISO は作れない)。SecureBoot は Off(On で起動できるかは試していない。実機の構成に近づけるなら本番化で試す)。
3. Scancode Map は、`boot.wim` の SYSTEM ハイブを `reg load` してオフラインで書く。WinPE が起動した時点で効いている。
4. WinPE に COM1 は無い。結果は、VM 内で FAT32 ボリュームにドライブ文字を割り当てて `probe.log` を書き、VM 停止後にホストで VHDX をマウントして読む。
5. **観測**: VM 内の probe の Raw Input では、`MakeCode`(スキャンコード)と E0 は **Scancode Map の前の値**のまま、`VKey` が**変換後**の値になる(ペアで `make=0x1E vk=0x42`)。**解釈は未確定**: 写像が Raw Input の採取点より後(win32k の入力処理)で掛かるのか、Raw Input が元の `MakeCode` を保つ仕様なのかは、この観測だけでは決まらない。この観測は、ADR-230 の「Scancode Map はキーボードクラスドライバで効き…」(:84-88)と、ADR-248 の「kbdclass はセーフモードでも読み込まれるので入れ替えは効いたままの可能性が高い」(:171-173)の推測が前提にしている写像点と食い違う可能性がある。→ Q5。スパイクの probe の doc コメントは観測と逆のことを書いているので、本番化で直す。
6. 結果: ホスト型ランナー run 38127199793(headSha `1d2a21bb`)で、対照(A,S→A,S)・ペア(A⇄B)・3巡回(A→S→D→A)・逆向き3巡回(A→D→S→A)の4つが PASS。3巡回は向きどおり、かつ1回だけの適用。SSH で入ったランナー上の再実行(`C:\scancode_vm` と標準出力)でも同じ結果だったが、ログは残していない。
7. **準備完了の検出は一度も成功していない**: 4シナリオとも `probe ready (green screen): False after 150s`。ハーネスは検出を150秒待ち切ったうえで、**20秒の固定待ちのあとにキーを送って**いた(`VM shut down by WinPE: True after 175s`)。1シナリオ約3〜4分という費用の**内訳(WinPE の実際の起動時間)は測れていない**。4/4 PASS は「固定待ちで probe の Raw Input 登録が間に合っていた」上の結果で、起動が遅い回では打鍵が登録前に届いて欠け、FAIL になりうる(段階1の最初に潰す)。

## 決定

1. **入れ子 VM(WinPE)で Scancode Map の効きを検証する専用ワークフローを作る**(段階1)。IME 入りの本物の Windows を使う既存の E2E とは別にし、範囲は「入力スタックの `vk` と、LL フックが見る値」に絞る。
2. **判定点は2つ記録する**: probe は Raw Input(`MakeCode`・E0・`VKey`)に加えて **`WH_KEYBOARD_LL` フック**(`vkCode`・`scanCode`・`LLKHF_EXTENDED`)を1打鍵ごとに記録する。Raw Input の `VKey` は左右を区別しない(`VK_CONTROL` など)うえ E0 は写像前なので、左右の Ctrl・Alt や Caps→左 Ctrl は Raw Input だけでは判定できない。**判定は LL フックの `vkCode`(左右の区別あり)を主、`scanCode`+拡張ビットを副**にする。awase 本体の判断材料もフックの値なので、問いに合う。LL フックが WinPE で動くかは段階1の最初の確認項目(win32k はあるので動く見込み、未確認)。Raw Input と LL フックの `scanCode` を並べれば、Q5(写像点)の答えも出る。
3. **シナリオの書き込みは本番経路で作る**: 本番のバイト列は `decide(original, &request)` → `Decision::Write { entries }` → `build_bytes` で作られる(`scancode_map_admin.rs`)。シナリオは `ApplyRequest` と既存値から `decide` に通した結果を `build_bytes` で書く(`build_bytes` 単体は単体テストで足りている)。
4. **期待値はトートロジーにしない**:
   - 期待は、シナリオ表に**人が読める形で手書きする**(例:「循環 [A,S,D] を作った → A の位置を押すと S が出る」)。`entries()`・`decide` の出力から導かない(導くと、entries の向きが UI の約束と逆に実装されていても書き込みと期待が一緒に逆になり PASS する)。
   - 期待の `vk` は表で持たず、**同じ run の対照シナリオで実測する**: 対照で各キー(from と to の両方)を押して「送ったキー → (写像前の値, vk)」を記録し、写像シナリオの期待 `vk` は、対照で「写像前の値 = to」だったキーの `vk`。`awase-vkmap` の定数の誤りが書き込み側と期待側で相殺されず、WinPE のキーボードレイアウトへの依存も減る。
5. **シナリオ表に必ず入れるもの**: ペア、3つ以上の循環(両向き)、Caps への追加 Ctrl、右 Ctrl・Alt などの拡張キー、**既存の他ツールのエントリを保持したままの書き込み**、**from 重複の先勝ち/後勝ち**(`decide` は重複を作らないので手書きのバイト列。ADR-230 の未確認リストで VM が答えられる価値の最も高い項目。結果は `scancode_pairs.rs` の `revealed` の警告文の根拠に反映する)、JIS 専用キー(Q3)。
6. **判定の前提条件(「検証不能」の定義)**: 次をすべて満たしたときだけ PASS/FAIL を出す。1つでも欠ければ**検証不能**(理由に条件の番号を付ける)。
   1. VM が起動し、`probe.log` に `START` と `READY` がある。
   2. 送ったすべてのキーで `PressKey`/`ReleaseKey` の `ReturnValue` が 0(終了キー以外)。
   3. 送ったキーの数と、記録の押下(break=0)の数が一致する(オートリピートで増えても、取りこぼしで減っても検証不能。同じ `make` が連続する break=0 は1つに畳むかは段階1で押下時間を実測して決める)。
   4. 各打鍵の**写像前の値**が、送ったつもりの from スキャンコードと一致する(Hyper-V の VK→スキャンコード変換が想定と違うシナリオを検出する)。
   5. 同じ run の対照シナリオが PASS(対照が FAIL なら run 全体を検証不能にする)。
7. **Actions の結果への写し方**: 検証不能は VM を作り直して**1回だけ再試行**する(FAIL は再試行しない。再試行で FAIL を PASS にしない)。再試行後も検証不能ならジョブは **failure** にし、ジョブサマリの1行目に `INCONCLUSIVE`(条件の番号)と書く。緑のまま何も検証しなくなる事態を避けるためである。
8. **準備完了のハンドシェイク**: probe は Raw Input と LL フックの登録が終わってから窓を表示する。ホストは写像の対象外の番兵キー(例: F11)を数秒おきに送り、probe が受け取ったら窓の色を変える(緑→青)。ホストは色の変化を確かめてからテストキーを送る。番兵の打鍵は判定から除く。緑検出が全回失敗した原因(サムネイルのビット配置の仮定、160×120 への縮小、`$count -ge 50` の閾値)は `$ShotDir\*-ready.png` で1回確かめる。
9. **起動条件**: 次のファイルの push/PR と、**週1回の定期実行**と手動。毎 push では回さない。
   - `crates/awase-windows/src/scancode_map.rs`・`scancode_pairs.rs`・`scancode_apply.rs`・`scancode_editor.rs`(`scancode_diagram.rs` も含めたいなら明示)、`crates/awase-settings/src/scancode_map_admin.rs`
   - `tools/scancode-vm/**`(シナリオ表の JSON と生成物の golden を含む)と本ワークフロー
   定期実行は、windows-latest のイメージ更新や ADK パッケージの版変更で、awase のコードが変わらなくても壊れることの検出用。「検証不能が連続した」ことも、定期実行で検出する。
10. **生成と実行の分離**: VM ワークフローで `awase-windows` をビルドしない。`crates/awase-windows` に、シナリオ表(`ApplyRequest`)→ `decide` → `build_bytes` の16進を `tools/scancode-vm/scenarios.json` と比較する golden テストを置き(Linux の `cargo test` で走る)、VM ワークフローはその JSON を読むだけにする。
11. **費用**: 1シナリオ1起動のままだと10〜15シナリオで35〜50分になりうる(未測定)。互いに素なキー集合の写像は1つのマップにまとめて1回の起動にする(from 重複のように他と干渉するシナリオは単独)。上限は1ジョブ45分とし、超える場合は VM を2〜3台並列(各 1GB)にする。
12. **ライセンス・公開**: WinPE は ADK 由来で、評価版 Windows イメージは使わない。ISO や VHDX は成果物にしない。artifact は、スクリーンショット・`probe.log`(打鍵の記録のみ)・シナリオ JSON。ADK を `actions/cache` に入れる場合、公開リポジトリのキャッシュは fork の PR のワークフローからも読めることを前提に、イメージ由来のファイルは入れない。
13. **probe の修正**: doc コメント(Raw Input の `MakeCode` は変換後、という記述)を観測に合わせて直す。終了キー F12 は写像前の `make` に依存して終了でき、シナリオで F12 の位置を写像しても終了できる、という性質を意図として書く。

## 範囲と限界

検証できるのは「OS の入力スタックが、そのバイト列で `vk`(と LL フックの値)を変換するか」まで。

- IME・TSF・awase 本体は動いていない(WinPE)。awase のフックを通ったあとは既存の E2E の担当。
- 書き込み経路(UAC、`HKLM` への書き込み、実 OS の再起動)。VM では SYSTEM ハイブへ直接書くので、ADR-230 段階5 の実機確認は残る。
- 実機の HID 経路で同じ点に写像が掛かるかは未確認。Hyper-V の仮想キーボード(i8042prt/kbdhid を通らない経路)でも写像が掛かったので、写像点はポートドライバより上にあると推定できるが、実機の確認は別。Ctrl2cap 等の kbdclass の UpperFilters ドライバが出す値に対して上位で写像が掛かる、という合成順序も推定のまま。
- JIS/US の違い。WinPE の既定レイアウトを使う。JIS レイアウトを入れられるかは Q3。

ADR-230/248 の未確認項目との対応:

| 未確認 | この ADR で |
| --- | --- |
| 3巡回の向き・1回だけの適用 | 済(スパイク) |
| from 重複の先勝ち/後勝ち | VM で可(決定5) |
| `0x003A` を別キーと入れ替えたときの Shift 分岐(`VK_CAPITAL`) | JIS レイアウトを WinPE に入れられれば VM で可。LL フックの `vkCode` で見る。入れられなければ実機 |
| SAS(Ctrl+Alt+Del) | WinPE に経路が無く不可。実機 |
| ADR-248 Q2(再起動と電源オンで効きが違うか・高速スタートアップ) | 不可。実機 |
| ADR-248 Q3(WinRE でのオフライン削除の手順) | `ControlSet00N` の N の確認は `reg load` と同じ操作なので VM で可。セーフモード・画面キーボードは実機 |
| Ctrl2cap との重なり | 費用に見合わないので実機 |

## 未確認(Q)

- Q1: ホスト型ランナーの入れ子仮想化の安定性(失敗率)。段階1の連続実行で測る。測定の前に、決定8 のハンドシェイクで偽 FAIL の発生源を潰す。
- Q2: WinPE の実際の起動時間(ハンドシェイクでの準備完了までの時間)。短縮できるか。
- Q3: `Msvm_Keyboard` の `TypeScancodes` の有無(`Get-CimClass … CimClassMethods` の出力で確定)。JIS 専用キー(無変換・変換・かな・半角/全角)と E0 付きキーを、`PressKey` の VK と生のスキャンコードのどちらで、どのスキャンコードとして送れるか。送れなければ、シナリオの半分(ADR-230 のワンクリックの中心)が作れない。
- Q4: LL フックが WinPE で動くか(決定2)。
- Q5: 写像点。Raw Input と LL フックの `scanCode` を比べて確定し、ADR-230 :84-88 と ADR-248 :171-173 の推測を訂正する要否を決める(両 ADR への追記)。

## 段階

- 段階0(済): スパイク。4シナリオ PASS。
- 段階1: ハンドシェイクと LL フック(Q3・Q4・Q5 の測定を含む)、検証不能の定義と再試行(決定6・7)、パスフィルタと定期実行。
- 段階2: シナリオ表(JSON)と golden、シナリオの追加(決定3〜5)。
- 段階3: 結果を ADR-230 段階5 と `revealed` の警告文に反映する。

## 関連

- ADR-230(Scancode Map の双射編集)、ADR-248(画面の分かりやすさ・循環への一般化)。
- `docs/ci/debug-session.md`(ランナーへ SSH で入る手順。スパイクの再現・Q の測定に使う)。
- レビュー: `docs/adr/review/257-opus-review-round1.md`。
