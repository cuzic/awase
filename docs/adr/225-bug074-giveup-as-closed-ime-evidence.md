---
id: ADR-225
title: |-
  RawTsfLiteralRecovery の give-up で文字が痕跡なく消える件(BUG-074)— 先に測り、方向は所有者が決める
summary: |-
  BUG-074: 外部から実 IME が閉じ belief が ON のままのとき、TsfNative×GJI の最初の打鍵は literal になり、回収も literal になって give-up し、文字が消える(CI 10/10)。
  r1 レビュー(Opus、Blocker 2・Must 7)で「give-up を Medium 観測にして drift correction に再オープンさせる」案は、明示意図があると belief が動かず、無いと drift が発火せず、元の報告(Windows Terminal・cold)では NICOLA を止めたまま戻らない、と判明し撤回した。
  r2 レビュー(Must 2・Should 4)で、give-up 後は連続カウントが戻らず以後の打鍵が全部消える見込み(コード確認済み、実測は D0-3)と判明し、「失われるのは 1 文字」を前提にした比較を改めた。
  決定案: D0 で偽陽性率と give-up 後の挙動(BS を打たない案Kとの比較を含む)を測る。方向は 再オープン/案K(BS を打たず痕跡を残す)/通知/追随/受容 から、ADR-205 との整合として所有者が決める。起草者の推奨は D0 後に案K を第一候補、必要なら通知を併用。
status: |-
  起草 r3(2026-10-04)。Opus r1(Blocker 2・Must 7・Should 6)・r2(Must 2・Should 4)を反映。実装なし。D0 の測定と所有者の方向決定が先。
related_adr:
  - "ADR-080"
  - "ADR-100"
  - "ADR-101"
  - "ADR-191"
  - "ADR-200"
  - "ADR-205"
  - "ADR-212"
---

# ADR-225: give-up で文字が消える件(BUG-074)

## 背景と事実

- 再現(CI、2026-10-04、run 37188479610 `sc-driftrecovery-gji-tsf`): 実 IME を awase の外から閉じ(belief は明示意図 ON のまま。`check_drift_recovery.py` は `VK_IME_ON` で `explicit_intent=Some(true)` を前提にする)、`k`,`a` を打つ 10 試行が **10/10 で give-up**、入力先は空。MS-IME×tsf は 0 件(GJI 固有)。
- 経路: `output/probe_io.rs` の `RawTsfLiteralRecovery`(約 582〜627 行)。`consecutive==0` は BS+再送、それ以外は BS のみ。**現状、give-up は belief への観測を一切記録しない**(r1 S4)。reinit は ADR-212 P3 で撤去済み(実 Chrome×GJI 0/10。ただし自前 RichEdit では 30/30 効いた)。
- 既存の決定: ADR-100 決定3(再送の却下・案L)、ADR-205(`follow_external_change`: 外部から閉じられたら**追随して意図を捨て、IME には書かない**。テスト `follow_external_change_closes_belief_even_with_explicit_on_intent`)、ADR-212 P6(drift correction は「明示操作の書き込みが届かなかった」再試行に限る)。
- **give-up の後は連続カウントが戻らない**: リセットするのは `FocusChange`・`SetOpenTrue`・`CompositionConfirmed` だけ(`tsf/probe.rs:368-380`、`probe_io.rs:655`、コメントに「give up→stuck」)。実 IME が閉じたままなら以後の打鍵も literal になり、すべて BS のみの give-up(`probe_io.rs:615-626`)になる**見込み**(コード読解。実測は D0-3)。つまり失われるのは 1 文字ではなく、IME を開け直すまでの全打鍵かもしれない(BUG-27 追補2 の「何も入力できません」と同じ見え方)。
- 元の報告(Windows Terminal・cold)は**フォーカス直後で明示意図が無く、実 IME が ON だったかは推定**(ログは reinit 後の Hiragana を見ただけ)。CI の構成(明示意図あり・外部クローズ)とは別の状況である。

## r1 で撤回した旧案とその理由

旧 D1「give-up を Medium 観測として入れ、drift correction に再オープンさせる」は成立しない(r1 B1・B2・M1〜M7)。

- 明示意図があると `effective_open()` は観測を見ず(`ime_model.rs:455-476`)belief は動かない。無いと drift は発火しない(`drift_correction.rs:52-54`)。両方同時には成り立たない。
- 元の報告の状況(明示意図なし)で閉の観測を足すと、NICOLA が OFF になり再オープンもされず、フォーカス変更かモードキーまで固着する(実 IME は ON のまま、以後の打鍵が化ける)。
- TsfNative は refresh tick が止まっており(`runtime/mod.rs:1120-1126`)、BUG-51 同様に記録と同時に `schedule_ime_refresh(20)` が要る。新ソースを `Actuating` にすると授権が下りず送信されない(`open_warrant.rs:180-208`)。
- 「literal だから閉」は否定的証拠からの逆向き推論で、`GjiIoInference` の一方向方針に反する。`consecutive` は StaleConfirm でも増える(ADR-200)。

## 決定案

### D0 先に測る(観測のみ、挙動変更なし)

1. **偽陽性率**: cold・実 IME は ON・外部クローズなしで give-up が何回起きるか(長い idle 後に RichEdit 窓へフォーカスして打つ)。
2. give-up の内訳(`SuspectedLiteral` 2 回か StaleConfirm を含むか、`LiteralDetectRecord.facts`)と、give-up 時点の `explicit_intent`(既存の実機 journal〈BUG-074 の 2 件・BUG-045 等〉と CI の両方)。
   - 先に確認: 実機の不具合報告 journal(JSON)に `explicit_intent` が載っているか。載っていなければ「既存の実機 journal から測る」は不可で、CI の awase.log(`explicit_intent=`)に限る。
   - 偽陽性率は自前 RichEdit だけでは代表できない(ADR-212 P3 で RichEdit 30/30・実 Chrome 0/10 と結果が逆だった)。補助に、実機報告の `LiteralDetectRecord`(`gave_up=true` の `facts`)を集計する。
3. give-up の**後**の追加打鍵 3 回の出力(人間の速さ 100〜200ms 間隔。試行冒頭の `VK_IME_ON` が `consecutive` をリセットする点に注意、r1 S3)と、awase.log の belief 遷移。**以後の全打鍵が消えるか**を確かめる(上記の見込みの検証)。あわせて**案K(give-up で BS を打たない)の変種**で同じ構成を回し、画面に何が残るかを比較する。
4. `VK_IME_ON` 単独を誤検出時(実 IME ON・未確定文字あり)に送ったときの破壊性(awase なしの対照)。

### D1 方向(所有者の判断事項、D0 後に決める)

| 案 | 内容 | 長所 | 短所 |
| --- | --- | --- | --- |
| 1 再オープン(最小に絞る) | 記録条件を **`explicit_intent()==Some(true)` かつ否定的証拠 2 回以上**(ADR-200 決定1 と同じ)に限る。ソースは `BeliefOnly`、TTL≤1500ms か `CompositionConfirmed` で対称に `open:true` を記録、記録と同時に `schedule_ime_refresh(20)`、送信は 1 give-up につき 1 回。belief は動かず効果は drift の再オープンのみ | CI の構成で文字が救われる | ADR-205 と逆方向(ユーザー自身の閉を打ち消す)。効くのは「明示意図 ON・TsfNative・外部クローズ」の 1 構成だけ。Chrome は対象外 |
| 2 通知のみ | give-up で「IME が閉じている疑い」を通知。`show_tray_balloon` だけ流用し、journal は新エントリ(`LiteralGiveUpNotice`)・抑止フラグも `drift_giveup_notified_this_focus` と分ける(既存関数は drift 継続時間が発火条件で、送っていない `VK_IME_ON` を journal に書いてしまう)。案L の romaji 記録と併用 | 「awase は IME に書かない」(ADR-205・212)と矛盾しない | 通知を待つ間の打鍵は消える可能性(D0-3 待ち)。誤検出だと「閉じている疑い」を誤通知 |
| K 痕跡を残す(ADR-100 決定3 の案K) | give-up 分岐で BS の予約(`set_raw_literal(backs, String::new(), …)`)をやめる | IME に書かず、送信も増えず、「痕跡なく消える」を直接解消(`k` 等のローマ字が見え、IME が閉じていると気づける)。誤検出時に正しい文字を BS で消す害も減る。BUG-27 追補2 と逆方向でループの危険が増えない | 部分的なローマ字が画面に残る(BUG-036 の `tみや` 型の汚れ) |
| 3 追随 | 閉と判断したら Engine を OFF にそろえる(ADR-205 と同じ向き) | 方針が一貫 | D0-1 の偽陽性率がほぼ 0 でなければ不可。偽陽性は旧案と同じ害 |
| 4 受容 | 記録のみ(known-bugs) | 変更なし | 外部クローズ後、打鍵が痕跡なく消える(全打鍵かは D0-3 待ち) |

起草者の推奨: **D0(D0-3 に案Kの比較を含む)→ 案K を第一候補、必要なら案2 を併用**。案2 を出すなら N2(journal・フラグの分離)と、否定的証拠 2 回以上・配線・focus 世代照合が前提。D0 で偽陽性がほぼ 0 と示せ、かつ所有者が「外部クローズは打ち消してよい」と判断した場合に限り案1。案3 は偽陽性が測れてから。reinit の復活(C')・Unicode 直接送信(案J)は不採用(後者は偽陽性で二重出力)。

### 全案に共通する事項(案1・案2 とも。案K・案4 は不要)

- 配線: 通知も記録も、give-up が確定する output 層(`dispatch_probe_actions`)から runtime へ渡す必要がある。
- 照合: 記録・通知の focus 世代は**プローブ開始時**のもの(ADR-101 追補2)。
- 条件: 発火は**否定的証拠 2 回以上**(ADR-200 決定1)に限る。`consecutive` は StaleConfirm でも増えるので使わない。誤検出率(D0-1)を採用条件にする。

### 案1 だけの実装上の必須事項(r1 より)

- 配線: `dispatch_probe_actions` は `ImeStateHub` に触れないので `ProbeIo` へのメソッド追加か runtime の outbox 経由の新経路を決める(M6)。観測のフェンスは**記録時でなくプローブ開始時の focus 世代**(ADR-101 追補2)。
- 新 `ObservationSource` の影響範囲: `PerSourceObservations::get/set`、`authority()=BeliefOnly`(`ime_event.rs`)、journal シリアライズ、`architecture_guard` の件数ガード(S4)。
- 否定的証拠カウンタ(`tsf/probe.rs` の `negative_evidence_count`)は ADR-212 P3 以降本番の呼び出し元が無いので配線し直す(M5)。
- 送信の上限: Blind の `backoff` は未使用で、`VK_IME_ON` が 100〜200ms に最大 5 回出うる。1 give-up 1 回に制限する(M4)。
- ADR-212 との関係: P6 の「許可」の範囲を広げることになるので、複雑性予算(`complexity-budget.md`、未発効)の観点で超過を明記する(S6)。

## 守る規約

- 再発ファミリー(warmup/IME belief/actuation 合流点)。回帰テストは `tests/closed_loop_scenarios.rs` 等(Linux)で固定: ①明示意図 ON+記録 ⇒ drift `Some`、②明示意図なし ⇒ `None` かつ `effective_open` 不変、③`CompositionConfirmed` 後 ⇒ `None`(S5)。`docs/known-bugs/BUG-074.md` も同じ PR で更新。
- belief 書き込みは `ObserverReported` 経由のみ(`UserImeSetIntent`・`HeuristicDefault` の流用は禁止)。
- 設計の前に測る(D0 が先)。実機 Chrome の同一性が確認できるまで「修正済み」とは書かない。

## 限界

本 ADR が根拠にできるのは CI の自前 RichEdit 窓(tsf×GJI)の 10/10 だけで、元の報告(Windows Terminal)・Chrome との同一性は未確認。
