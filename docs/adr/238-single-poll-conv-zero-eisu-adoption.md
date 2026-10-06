---
id: ADR-238
title: |-
  一過性の conv=0 を 1 回の poll で ObservedEisu と採用して Engine が止まる件(BUG-190)— 英数モードの採用条件
summary: |-
  BUG-190: MS-IME の OS ポーリングが一瞬 `conv=0x00000000`(`romaji=None`)を返すと、`classify_ime_snapshot` が 1 回の観測だけで `InputModeObserved(ObservedEisu)` を belief に書き、`Inactive(NotRomajiInput)` で Engine が 0.08〜3.2s 止まり、その間の打鍵が生のまま IME に渡る。CI 599 本(MS-IME 299・GJI 292)で conv=0 の poll は 5 件、すべて MS-IME・前後の poll が 0x19 の孤立した 1 回・5 件すべて採用されて NotRomajiInput(GJI は 0)。
  決定(案): 英数モードの採用に「確認」を足す。案は A(前回の poll も英数のときだけ採用、純粋・新しい状態なし)・B(ime_on が None の poll では採らない)・C(MS-IME では open 中の conv=0 を英数と見なさない)・D(直近の打鍵中は採らない)。Opus 敵対レビューで決める。
status: |-
  起草中(2026-10-06)。実装は未着手。
related_adr:
  - "ADR-186"
  - "ADR-191"
  - "ADR-233"
---

# ADR-238: 一過性の conv=0 を 1 回の poll で ObservedEisu と採用する件

## 背景(BUG-190、CI で再現)

[BUG-190](../known-bugs/BUG-190.md)。MS-IME で、打鍵の途中(または最初の試行)に出力が全角英字混じりに崩れる低頻度の症状。BUG-189(古い ImmCrossProbe が優先される件、ADR-233)の修正後も残る別の機序。

### 機序(ログで裏取り)

1. OS ポーリング(`ime.rs::read_ime_state_full`)が、一過性に `romaji=None conv=Some("0x00000000")` を返す。直前・直後の poll は `romaji=Some(true) conv=0x00000019`。
2. `observer/ime_observer.rs::classify_ime_snapshot`(171 行付近)が、`ConvMode::is_eisu_evidence(snap.ime_on, snap.conversion_mode)`(`src/engine/conv.rs:75`)が `Some(true)` なら、**その 1 回の観測だけで** `InputModeState::ObservedEisu` を返す(現在値が ObservedEisu でなければ)。`is_eisu_evidence` は `ime_on == Some(false)` のときだけ conv=0 を無視する(BUG-57: IME が閉じた窓の conv=0 は選択の証拠でない)。`ime_on` が `Some(true)` でも `None`(open の読みが 50ms で時間切れ)でも採用される。
3. `input_mode=ObservedEisu` → `Inactive(NotRomajiInput)` で Engine が止まる。回復は次の poll(`conv=0x19` → `input_mode_from_romaji_flag` が ObservedRomaji に戻す、または `ObservedEisu → AssumedRomaji` の stale recovery)。ObservedEisu を作る本番経路は、この poll(`ObserverPoll` と、`classify_fetched_snapshot` 経由の ImmCrossProbe/FocusProbe)だけで、他に物理の英数キーの予測(`KeyEffectPredicted`)がある。

### 発生状況(手元の CI 全 run の `IME snapshot` ログ、run 599 本)

| IME | run 数 | conv=0 の poll | うち孤立(前後が 0x19) | 採用されて NotRomajiInput |
| --- | --- | --- | --- | --- |
| MS-IME | 299 | 5(`ime_on=Some(true)` 4・`None` 1) | 5 | 5 |
| GJI | 292 | 0 | - | - |

- 5 件は Flutter 3(BUG-189 の修正前の run)・wx 1・Java AWT 1。止まった期間は 0.08〜3.2s。wx の 1 件だけ、打鍵中に止まって最初の試行が崩れ FAIL した。Java AWT の 1 件は期間中に打鍵が無く PASS。
- poll の直前の `key-effect-predict` は、Flutter の 3 件は**文字キー**(0.00〜0.02s 前、打鍵と同時)、wx は Enter の 0.81s 後、Java AWT は文字キーの 0.56s 後。**変換/無変換キーとの関係は確認できていない**(BUG-190 の初版の「変換キーの処理中」という推定は撤回する)。
- 本物の半角英数への切替(ユーザーが英数キーを押す)はこれらの run には含まれない。**この変更が本物の切替の検出を壊さないことの測定は、sc-* の IME キー系(特に `check_consistency` の walk、MS-IME 構成)で別に要る。**

## 決めること

`classify_ime_snapshot` が `ObservedEisu` を採る条件に、一過性の読みを弾く確認を足すか。足すなら何で確認するか。

## 案

| 案 | 内容 | 利点 | 懸念 |
| --- | --- | --- | --- |
| A | 前回の poll の conv(`current_prev_conversion_mode`、既に引数にある)も英数のときだけ採用する(連続 2 回)。前回が `None` のとき(フォーカス直後等)の扱いは要決定 | 純粋で新しい状態が要らない。原因を問わず効く | 本物の切替の検出が 1 poll(約 0.5s)遅れる。前回 conv が打鍵中の skip で更新されない(`Skipping observer/SSOT write`)場合、確認が遅れる/古い前回と比べる |
| B | `ime_on` が `None` の poll では採らない | 単純。wx の例を直す | 5 件中 4 件は `ime_on=Some(true)` なので**足りない**。ADR の本命にならない |
| C | MS-IME では `ime_on=Some(true)` の conv=0 を英数と見なさない(MS-IME の半角英数は open=false になる、という前提) | 遅れが無い | **前提が未確認。** GJI/ATOK の半角英数は open=true・conv=0 で、`ActiveImeKind` で分岐する必要がある。MS-IME の英数が open=true・conv=0 のこともあるなら本物の切替を落とす |
| D | 直近 N ms に打鍵(awase の出力注入を含む)があれば、その poll の conv=0 を採らない | 遅れが無い | 5 件の 3 件は打鍵と同時だが、2 件は 0.5〜0.8s 後で、**N を決める根拠が弱い**(tuning-constants.md の実測義務)。打鍵中は poll を skip する既存の仕組みと重複する |
| E | ObservedEisu を採っても、Engine の停止を遅らせる/次の poll まで保留する(belief は書くが engine には反映しない) | 原因を問わない | belief と engine の乖離を作る。`ime-belief-architecture.md` の規律に反する可能性 |

## 評価に必要な測定(実装前)

1. 本物の半角英数への切替で、MS-IME・GJI の `conv` と `ime_on` が何になるか(MS-IME の英数が open=false か、open=true・conv=0 か)。既存の IME キー行列(`tools/e2e/ime_key_matrix`、`check_consistency.py`)の結果・docs/experiments.md・ADR-186 から探す。足りなければ実機/CI の walk で測る。
2. 案 A の遅れ: 本物の切替から Engine が止まるまでの時間が、現状の 1 poll から 2 poll に伸びるときの実害(英数に切替えた直後の数打鍵が変換される)。
3. 診断: `ObservedEisu` を採った件に、その poll が孤立か連続かを 1 行出す診断ログ(`[eisu-adopt] conv=… ime_on=… prev_conv=… next…`)を足して、MS-IME 構成の CI(`tsx-ext-*-msime-*`、約 70 ジョブ)で頻度と形を数える(手元の既存ログでは 5/299 run、孤立 5/5)。

## 状態

起草中。Opus 敵対レビューで案の比較・見落とし(特に本物の切替の検出を壊さないか)を確認してから決める。
