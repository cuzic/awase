---
title: v2 チェックリスト B3 — ADR-199 T1 (a)(d)(e) の CI 確認
status: 完了（2026-09-29）。(d)(e) は確認できた、(a) は CI では構造上確認できない（実機の既存記録が根拠）
created: 2026-09-29
related_adr: ["ADR-199", "ADR-186", "ADR-202"]
---

# B3: ADR-199 T1 (a)(d)(e) を GitHub Actions windows-latest で確認する

所有者決定により実機は使わない。develop 先端（`fe4b5573` 時点。以降 `0ef18dc2` までの差分は e2e ワークフロー 2 ファイルのみ）から
検証専用ブランチ `ci/v2-b3-t1`（develop に **マージしない**。`e2e-ime.yml` に T1(d) のレジストリ読み取りステップを 1 つ足しただけ）を切り、
`e2e-ime.yml` を 1 回だけ実行した。

**run**: https://github.com/cuzic/awase/actions/runs/36569131073
（only=`sc-t1e-chrome-f13,sc-t1e-chrome-f13-noawase,sc-t1e-f13-toggle,sc-hz-msime-native-noawase`。
summary ジョブが赤いのは不変条件の上限見直し通知 `i2_unwarranted ... 上限を 0 へ下げてよい` を失敗扱いにする既存の仕組みのため。各構成の e2e ジョブは全て success）

## 結果

| 項目 | 判定 | 根拠 |
| --- | --- | --- |
| (a) カスタム TSV に `Hankaku/Zenkaku` 行が残るか | **CI では確認できない（構造上）。実機の既存記録が根拠** | 下記 |
| (d) 互換モードを触っていない環境に `NoTsf3Override2` が無いか | **確認できた**（3/3 ランナー） | 下記 |
| (e) F13 をトグルにした構成の実 Chrome 実タイピング（develop 先端） | **確認できた**（PASS 4/4、awase が観測した F13 は 4/4） | 下記 |

### (a) 判定不能（CI では原理的に作れない）

CI の GJI 設定手順（`e2e-ime.yml` の「入力言語をja-JP + GJI に設定…config1.db を書く」）は、`tsv` 入力を **ハーネスが protobuf として `config1.db` に直接書く**。
つまり CI で読める TSV は「ハーネスが書いたもの」であり、GJI が CUSTOM 表を書き出したときの行の残り方（Hankaku/Zenkaku 行が残るか）は測れない。
GJI の設定画面を CI で操作する手段も無い。よって CI で再確認しても同語反復になるので実施しなかった。
根拠は ADR-199 T1(a) の既存記録: 実機の GJI が書いた CUSTOM 表 `docs/adr/186-measurements/config1-custom-keymap-table-ignored.tsv` に
`Hankaku/Zenkaku` 行が 4 状態とも残る（2026-09-26 確認済み。ADR-202 でも実機で「4 状態とも残る」）。
**T1(a) は実機の記録で確認済みのまま。CI での追加確認は不要と判断する。**

### (d) 確認できた

`NoTsf3Override2` は「以前のバージョンの Microsoft IME を使う」（互換モード、ADR-197 決定4）のフラグ。何も書かない前に読んだ結果:

| ジョブ（sc-hz-msime-native-noawase） | `Tsf3Override` 親キー | MS-IME の子キー | `NoTsf3Override2` | 10 秒後 | MS-IME TIP |
| --- | --- | --- | --- | --- | --- |
| 109408702430 | 有り（子キー無し） | 無し | (none) | (none) | 登録済み |
| 109408702488 | 有り（子キー無し） | 無し | (none) | (none) | 登録済み |
| 109408702531 | 有り（子キー無し） | 無し | (none) | (none) | 登録済み |

- 試行数 3、いずれも値なし（3/3）。`HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME` キーも無し。
- **観測経路に乗ったか**: この 3 ジョブでは MS-IME 本体が実際に動作していた。対照実験（awase なし）の半角/全角 0xF3/0xF4 の 8 押下が全て期待どおり開閉した（`open=1 conv=0x19` ⇄ `open=0 conv=0x10`。3 ジョブとも result.txt で確認）。
  「IME が無かったから値も無い」ではない。ただし awase を起動していないので **awase 自身が `read_legacy_compat_mode_enabled()` を `None` と読んだこと（observed）は今回測っていない**。
  レジストリ値を直接読んだ結果であり、awase の読み取りが `None` を返す点は ADR-199 T13 のホストテストが担保する。
- 前回（run 36281035712 の `sc-t12-baseline`、1 ランナー）と合わせて 4/4 ランナー。
- 決定17 の `None`→トグル扱いは「既定環境では値が無い」前提に立っており、windows-latest の既定ではその前提が成り立つ。**決定17 は現行の `None`＝トグルのままでよい。**
- 限界: 値が無いことの確認は「windows-latest の ja-JP を追加した直後のランナー」に限る。実利用者が過去に互換モードを ON→OFF した場合に値が `0` で残るかは CI では見られない（`Some(false)`＝トグルなので結論は同じ）。

### (e) 確認できた（develop 先端、#367・#373・#376 の後）

構成 `sc-t1e-chrome-f13`（GJI の CUSTOM 表で F13 をトグル、awase 起動、実 Chrome＝TsfNative）。
表: `DirectInput,F13,IMEOn` / `Precomposition,Composition,Conversion` の F13 は `IMEOff`（`ON`/`OFF` の書き込み手段の行つき）。
`ime_toggle`・`ime_detect` の既定は空（#367・#373 後の既定 config）。

| ケース | 回 | 結果 |
| --- | --- | --- |
| かな→F13 = IME OFF（`ka` が出る） | 1, 2 | PASS, PASS |
| 直接入力→F13 = かな ON（NICOLA 文字 `きう`） | 1, 2 | PASS, PASS |

- 試行 4、`PASS=4 RECOVER=0 FAIL=0 INVALID=0`。不変条件も OK（`i1`=0、`i2_unwarranted`=0、`i4_gji_fsm_off`=0）。
- **観測経路に乗ったか（awase.log）**: F13（VK 0x7C）の KeyDown を awase が 4 回観測し、4/4 で `[shadow-toggle] intent 昇格 ... action=Toggle kind=PhysicalImeKey`
  （`true→false` / `false→true` を交互）と `[role-fkey] key suppress vk=0x7c KeyDown` が出た。決定18 の配送規則（最初の Down で実際に書いた打鍵だけ Suppress）の経路を 4/4 通っている。
  `Process(229)` の有無も想定どおり（ON で `true`、直接入力で `false`）。
- 対照 `sc-t1e-chrome-f13-noawase`（awase なし）も PASS 4/4（かな状態は `か`）。F13 は GJI 単体でも Chrome に効く。
- 限界（変わらず）: SendInput の F13（スキャンコード無し）で代替しており、`Scancode Map` で出す実 F13 はレジストリ反映に再起動が要り CI では確認できない。
  `sc-t1e-f13-toggle`（GJI 単体の API 読み取り、Chrome 無し）は今回 INVALID（実行中にフォーカスが外れた 1 回）で**判定不能**。
  ただしこれは (e) の Chrome 構成と重複する部分検証で、run 36243203670 では確認済み。再実行はしていない。

## ADR-199 T1 行への追記案

T1 行の (e) の「実 Chrome…確認済み（2026-09-26…）」の直後に次の一文を足す（本 PR で ADR-199 に反映済み）:

> **develop 先端での再確認（2026-09-29、ADR-205 後・#367/#373/#376 後、run 36569131073）**: `sc-t1e-chrome-f13` は 4/4 PASS（awase が F13 の KeyDown を 4/4 観測し `role-fkey` の Suppress 経路を通った）。(d) は windows-latest のランナー 3 台（MS-IME 本体が動作している）で `NoTsf3Override2` 無し（前回の 1 台と合わせて 4/4）。(a) は CI ではハーネスが `config1.db` を直接書くため確認できず、実機記録（ADR-186 サンプル）が根拠。

## 残ること

- 実機での `Scancode Map` の実 F13（(e) の限界）は B6 と同様に CI では見られない。所有者が必要と判断した場合のみ実機で。
- 検証用ステップ（T1(d) のレジストリ読み取り）は `ci/v2-b3-t1` にだけある。常設するかは所有者判断（常設しても構成数は増えない）。
