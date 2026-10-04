---
id: ADR-225
title: |-
  RawTsfLiteralRecovery の give-up で文字が痕跡なく消える件(BUG-074)— 先に測り、方向は所有者が決める
summary: |-
  BUG-074: 外部から実 IME が閉じ belief が ON のままのとき、GJI の最初の打鍵は literal になり(再現した窓は `profile=Imm32Unavailable`。TsfNative での close-follow は未測定)、回収も literal になって give-up し、文字が消える(CI 10/10)。
  r1 レビュー(Opus、Blocker 2・Must 7)で「give-up を Medium 観測にして drift correction に再オープンさせる」案は、明示意図があると belief が動かず、無いと drift が発火せず、元の報告(Windows Terminal・cold)では NICOLA を止めたまま戻らない、と判明し撤回した。
  r2 レビュー(Must 2・Should 4)で、give-up 後は連続カウントが戻らず以後の打鍵が全部消える見込み(コード確認済み、実測は D0-3)と判明し、「失われるのは 1 文字」を前提にした比較を改めた。
  決定: 所有者判断で追随(belief だけを実状態へ揃え IME には書かない)。D0 で偽陽性 0/30(RichEdit・実 Chrome・Windows Terminal)。Opus r4・r5 で、追随が TsfNative の conv 推論で打ち消される恐れ(B1)・観測ソースの偽装・取り出し時点の遅れ・再現窓が Imm32Unavailable であること・ADR-205 の柵の欠落等が判明し、設計を r6 に直した(Imm32Unavailable は give-up を読み直しのきっかけにする案、TsfNative は D0-5 の実測待ち)。
status: |-
  起草 r6(2026-10-04): 所有者判断=追随。Opus r4・r5・r6 を反映し、(i) は収束(r6 の Must 1 件を反映済み・再レビュー不要)。測定済みの `Imm32Unavailable`×GJI は「give-up を読み直しのきっかけにする」(i)、TsfNative は B1 を D0-5 で測ってから(ii)。実装なし。Opus r1(Blocker 2・Must 7・Should 6)・r2(Must 2・Should 4)を反映し、r3 で収束(Blocker・Must なし)。実装なし。D0 の測定と所有者の方向決定が先。
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

- 再現(CI、2026-10-04、run 37188479610 `sc-driftrecovery-gji-tsf`。**入力先の `--form=tsf` 窓は `Chrome_RenderWidgetHostHWND` で `profile=Imm32Unavailable` に分類される**=ADR-193 の RichEdit スーパークラス窓。TsfNative(Windows Terminal 等)での close-follow は一度も測っていない): 実 IME を awase の外から閉じ(belief は明示意図 ON のまま。`check_drift_recovery.py` は `VK_IME_ON` で `explicit_intent=Some(true)` を前提にする)、`k`,`a` を打つ 10 試行が **10/10 で give-up**、入力先は空。MS-IME×tsf は 0 件(GJI 固有)。
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

## D0 の結果(2026-10-04、run 37212511286、ブランチ `ci/adr225-d0`、GJI×tsf〈自前 RichEdit〉、各 10 試行)

| 構成 | 内容 | 結果 |
| --- | --- | --- |
| close-follow | 外部クローズ → かな単打 1 回 + 追加 3 回(150ms 間隔)=4 打 | **10/10 で give-up(各 2 回、`count=2,3`)**。画面は確定前・後とも `kaka`(期待 `かかかか`)。実 IME は最後まで閉(`open_after=False`)。明示意図は 10/10 で `Some(true)`、StaleConfirm は 0 |
| close-follow-k | 同じ構成を、give-up で BS を打たない awase で | 10/10 で give-up。画面は `kkakka`(BS を打たないぶんローマ字の断片が残る)。実 IME は閉のまま |
| noclose-idle | 外部クローズなし・25 秒 idle 後にフォーカスを外して戻し cold で 4 打 | **give-up 0/10**。画面は 10/10 で `かかかか`(期待どおり)。明示意図 `Some(true)` |

読み取れること(RichEdit×GJI の範囲):

- **D0-3**: r2 の見込み「以後の打鍵が全部消える」は**外れ**。2 回目以降の打鍵は生ローマ字(`ka`)として画面に出る(4 打で `kaka`)。消えるのは各 give-up の BS で消される分で、IME は閉じたまま・誰も開け直さない。ユーザーには「半分ローマ字、半分欠落」に見える。
- **D0-1**: 外部クローズなしの cold(25 秒 idle+フォーカス移動)では give-up が **0/10**。偽陽性率は RichEdit では 0/10 だが、Windows Terminal・Chrome は未測定(代表性なし。案3 の採用条件は満たせない)。
- **D0-2**: 明示意図は全試行 `Some(true)`、StaleConfirm は 0(この構成では否定的証拠は SuspectedLiteral 由来)。実機 journal に `explicit_intent` が載っているかは未確認。
- **案K**: 痕跡は残るが `kkakka` のように汚れる。実 IME が閉じていることには気づけるが、見た目は良くならない。
- **実 Chrome の偽陽性率(run 37213745139、`cal-d0-gji-chrome-noclose-idle`)**: 外部クローズなし・IME ON・25 秒 idle 後にフォーカスを外して戻して `k`,`a` を打つ 10 試行で、**give-up 0・suspected 0(literal 疑いも 0)**、10/10 で NICOLA 文字が出た。RichEdit・Windows Terminal と合わせて、この条件(単発の cold・短い打鍵)では偽陽性は出ていない。ただし長い連続入力・高速打鍵(ADR-200/BUG-168 の StaleConfirm 型)は測っていない。
- **Windows Terminal の偽陽性率(run 37227022397、`cal-d0-gji-wt-noclose-idle`、`wt_probe.exe`)**: windows-latest の Windows Terminal 1.23(`CASCADIA_HOSTING_WINDOW_CLASS`、PowerShell で標準入力を 1 行ずつ読んでファイルへ書く)に、外部クローズなし・IME ON・25 秒 idle 後にフォーカスを外して戻して `k`,`a` を打つ 10 試行で、**give-up 0・suspected 0**、10/10 で NICOLA 文字(`きう`)が出た。元の報告と同じ入力先でも、この条件では偽陽性は出ていない。
  - 測定器の作り込みで分かった罠(再利用する人向け): ① `taskkill /im WindowsTerminal.exe` は runner 自身のコンソールホストを巻き込み、ジョブが「shutdown signal」で落ちる(窓を WM_CLOSE で閉じる)。② `wt.exe` は引数中の `;` をサブコマンド区切りと解釈する(`-EncodedCommand` で渡す)。③ IME の未確定文字は 1 回目の Enter では確定されるだけ(2 回押す)。④ .NET の `Console.In` は既定のコードページでかなを `?` に化けさせる(`InputEncoding` を UTF-8 に)。
- **未測定**: D0-4(`VK_IME_ON` 単独の破壊性)、Windows Terminal・Chrome。

## 所有者の判断(2026-10-04)

- 外部から IME を閉じられた場合は**追随する(IME には書かない)**。ADR-205 と同じ向き。再オープン案(案1)は採らない。
- ただし追随(案3)は**偽陽性がほぼ 0 と示せることが条件**(誤って閉と判断すると NICOLA が止まったままになる)。そのため方向の確定の前に、Windows Terminal と実 Chrome の偽陽性率を測る(RichEdit は 0/10 で済み)。
- 測定: 実 Chrome は `cal-d0-gji-chrome-noclose-idle`(`GIVEUP_D0_CHROME` 行)。Windows Terminal は CI で使えるかを `d0-wt-check.yml` で先に調べる(使えなければ代替を決める)。

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

**所有者の判断(2026-10-04)により案3(追随)を採る**。案K・案2・案1 は採らない(上の「所有者の判断」節と「追随案の設計」節)。reinit の復活(C')・Unicode 直接送信(案J)は不採用(後者は偽陽性で二重出力)。

### 案1・案2 に共通する事項(案K・案3・案4 は対象外)

- 配線: 通知も記録も、give-up が確定する output 層(`dispatch_probe_actions`)から runtime へ渡す必要がある。
- 照合: 記録・通知の focus 世代は**プローブ開始時**のもの(ADR-101 追補2)。
- 条件: 発火は**否定的証拠 2 回以上**(ADR-200 決定1)に限る。`consecutive` は StaleConfirm でも増えるので使わない。誤検出率(D0-1)を採用条件にする。

### 案1 だけの実装上の必須事項(r1 より)

- 配線: `dispatch_probe_actions` は `ImeStateHub` に触れないので `ProbeIo` へのメソッド追加か runtime の outbox 経由の新経路を決める(M6)。観測のフェンスは**記録時でなくプローブ開始時の focus 世代**(ADR-101 追補2)。
- 新 `ObservationSource` の影響範囲: `PerSourceObservations::get/set`、`authority()=BeliefOnly`(`ime_event.rs`)、journal シリアライズ、`architecture_guard` の件数ガード(S4)。
- 否定的証拠カウンタ(`tsf/probe.rs` の `negative_evidence_count`)は ADR-212 P3 以降本番の呼び出し元が無いので配線し直す(M5)。
- 送信の上限: Blind の `backoff` は未使用で、`VK_IME_ON` が 100〜200ms に最大 5 回出うる。1 give-up 1 回に制限する(M4)。
- ADR-212 との関係: P6 の「許可」の範囲を広げることになるので、複雑性予算(`complexity-budget.md`、未発効)の観点で超過を明記する(S6)。

### 案K を採る場合の注意(r3 Should)

- BS の予約をやめるとき、`escape_composition` の ESC を残すかを決める(一緒にやめると未確定文字が残るおそれ)。
- D0-3 では、各打鍵の先頭 1 文字(`k` 等)だけが残る可能性があるので、残る文字列をそのまま記録する。
- 既存テスト `raw_tsf_literal_recovery_tsf_mode_consecutive_gives_up_with_cold_mark` の期待値更新と、`BUG-074.md` の更新を同じ PR で行う。

## 追随案の設計(r6、Opus r5〈Must 3〉を反映)

**D1(追随)**: give-up を契機に、belief を実状態へ揃える。**IME には何も書かない**(ADR-205・ADR-212 と同じ向き)。再オープン案・通知案は採らない。

### 対象の整理(r5 M1): 測った構成と、設計の対象を分ける

D0 で再現できたのは **`Imm32Unavailable`×GJI**(自前 RichEdit 窓、`--form=tsf`)だけ。実 Chrome・Windows Terminal は偽陽性 0 しか測っておらず、**外部クローズ後の give-up は未測定**。TsfNative(Windows Terminal)の close-follow も未測定。そこで 2 つに分ける。

- **(i) `Imm32Unavailable`×GJI(測定済み・先に直す対象)**: 実際の読み(prefetch の `snap.ime_on`、ADR-205 が使うもの)が使える。give-up を**「閉の証拠」ではなく「読み直しのきっかけ」**にする(r5 S1)。読みが閉なら ADR-205 と同じ追随(`follow_external_change` 相当、実在の観測を記録、意図を削除、`pass_through_observed`)。読みが開なら何もしない=偽陽性は起きない。新しい evidence 型も推論も要らない。**ADR-205 の柵を必ず入れる**(下の M2)。
- **(ii) TsfNative(Windows Terminal 等、開閉を読めない)**: 推論(give-up を閉の証拠として `Observed<LiteralGiveUp>` を記録)が唯一の手段だが、B1(次の打鍵の idle-conv-check が `ConvOpenInference(true)` で打ち消す)が未解決。**D0-5 で TsfNative の close-follow と B1 を測ってから決める**。測るまで実装しない。

### 共通: 追随の柵(r5 M2、ADR-205 と同型にするために必須)

追随で明示意図を捨てると通常のポーリングが再開し(`reschedule_ime_refresh`、`runtime/mod.rs:1149-1178`)、`Blacklist` の `observe_gji_after_focus`(`observer/gji_observer.rs:28-58`)が「フォーカス変更後の GJI I/O が `GJI_CONFIRM_WINDOW_MS` 以内」なら `ObserverPoll(true)`(Medium・Actuating)を書き、belief を開へ戻しうる。閉じる前の composition の I/O や literal 回収中の I/O が窓に残っていれば起きる。ADR-205 は `follow_external_change` で `last_external_change_ms = now_ms` を進め、`ime_refresh.rs:170-171` の柵(`last_focus_change_ms.max(last_external_change_ms)`)で防いでいる(BUG-176 系)。**追随の手順に `last_external_change_ms`(または専用の柵)を追随時刻へ進める手順を入れる**。単体テストに「追随後、柵より前の GJI I/O では開に戻らない」を足す。

### (i) の設計: give-up を読み直しのきっかけにする

1. **条件**: give-up(`RawTsfLiteralRecovery` で `consecutive>=1`)、**最後の CompositionConfirmed 以降の否定的証拠が 2 回以上すべて `SuspectedLiteral`**(r4 M4。`consume_literal_detect_trace`/`note_literal_detect_record` で数え、`negative_evidence_count` を配線し直す)、取り出した時点で `explicit_intent()==Some(true)`(r4 S1)、`profile=Imm32Unavailable`×GJI。
2. **取り出し口(r4 M2)**: `advance_tsf_probe()` の直後(`runtime/message_handlers.rs:502-513`、`drain_journal_entries` と同じ位置)で `app.platform.take_giveup_evidence()`。`drain_output_post_send_effects` は送信の後にしか呼ばれないので使わない。予約済みの BS・INPUT_DEFER の再生は後段(`handle_wm_drain_output_queue`)なので、保留した打鍵は追随後の状態で再生される。
3. **動作(r6 Opus 確認済み: 監視窓の拡張は不要)**: give-up の取り出し口で、(1) `arm_external_change_watch(now)`(`platform_state.rs:416`、既存の `kp_arm_external_change_watch` と同じ呼び方)を呼び、(2) 続けて `schedule_ime_refresh(MODE_KEY_PASS_REREAD_MS)` で最初の読み直しを予約する(2 回目以降は監視窓〈300ms〉が生きている間、`reschedule_ime_refresh` が 60ms ごとに予約する)。読みの結果は既存の `ImeStateHub::follow_external_change(read, …)` に渡る(柵・意図の削除・desired の揃えはそのまま引き継がれる)。**give-up を閉の観測として直接書かない**。`arm()` は同じスコープの直近の読み(`last_read`)を基準値に採る(`external_change_watch.rs:54-71`)。
   - **基準値は `last_read` に限る**(belief・`desired_open` を基準値に入れない。入れると、読みが常に閉を返す環境〈ADR-205 が防いだもの〉で give-up のたびに偽の追随が起き、実 IME は ON なのに Engine が OFF になる=r1 B2 と同じ害)。
   - **`last_read` が無いか閉なら何もしない**。
   - 単体テスト 3 件: `last_read` が無い ⇒ 不変 / `last_read` が開で読みも開 ⇒ 不変 / スコープ違いの `last_read` ⇒ 不変。
   - D0-5 では arm した時点の基準値(`last_read`)を awase.log に出す(追随が起きなかった理由を区別するため)。
4. **focus 世代(r4 S2)**: プローブ開始時に `ime_mode_focus_gen` を捕獲し、取り出し時に一致を確かめてから `AcceptedObservation::for_sync(app.focus_fence())` を作る(`for_sync` は照合しない)。
5. **Engine への通知(r4 S3)**: 追随の直後に `RefreshState` を出す。
6. **利用者に見える入力(r4 M3)**: 追随後は Engine OFF で、出るのは**物理キーの QWERTY 文字**。失われるのは give-up した最初の 1 モーラ(BS は現状のまま)。親指キー(無変換・変換)が IME にそのまま届き、構成によっては IME が開く点を CI で確認する。
7. **効く範囲(限界)**: 明示意図が残る利用者だけ(読み直しは実際の読みに基づくので、この条件は偽陽性の防止ではなく**効く範囲を狭めるだけ**。残す理由は、意図が無い状況〈フォーカス直後等〉では ADR-205 の監視窓の前提〈外部注入キー直後〉も成り立たず、挙動を変える根拠が無いため)。ADR-205 から引き継ぐ BUG-176(偽 OFF の疑い 1 件)がこの追随にも当てはまりうる。読みは GJI×実 Chrome で 1→0 を返す(ADR-205 の測定、追随 10/10)が、開いていても 0 を読む MS-IME×Chrome は対象外(GJI に限る)。ADR-191 の予測経路(`KeyEffectPredicted`)で IME を開いた利用者は意図が常に `None` で、効かない。実機 journal に `explicit_intent` が載るかは未確認。外部から再び開かれた場合の戻りは既存のポーリングに任せる。

### (ii) の設計候補(D0-5 の結果次第、旧 r5 の内容)

専用の evidence 型 `Observed<LiteralGiveUp>`(Medium・`BeliefOnly`・`gave_up && SuspectedLiteral` の witness)を新設し、`follow_literal_giveup` が記録→意図削除→`pass_through_observed(align_desired=true)`。B1 の対策案: (a) `ConvOpenInference` に負けない形(BUG-26 との衝突を確認)、(b) 追随後〜次の明示操作/フォーカス変更まで `NativeToggleShadowOff` を抑止、(c) TsfNative を外す。`align_desired` が新鮮な開の観測と衝突したときの挙動を単体テストで固定する(r4 S4)。

### 検証計画

- **D0-5(先に測る、実装前)**: 次を `ci/adr225-d0` で測る。**成立条件(r5 M3)**: (a) **実 IME が閉じたことを awase に依存しない手段で確認**する(awase を起動しない対照で同じ方法で閉じ、`wt_probe` の出力が `ka`〈閉〉か `か`〈開〉かを見る。閉じていない試行は invalid)。`WM_IME_CONTROL` が TSF の開閉に反映される保証は無い(awase は WT で IMM32 の読みが常に None)。閉じない場合の代替は、マーカーなし注入の `VK_IME_OFF`(0x1A)。(b) 試行ごとに awase.log から、閉じた後〜give-up の最後の `explicit_intent=` が `Some(true)` であること(満たさない試行は invalid)。(c) 打鍵ごとに、`idle-conv-check` の実行有無・conv 値・`NativeToggleShadowOff` の件数を記録する(0 件は「起きない」か「観測経路に乗らなかった」かを区別できるようにする)。測るのは、(1) **TsfNative(Windows Terminal)の close-follow 自体**(give-up が出るか、先頭の打鍵の出力)、(2) `Imm32Unavailable` で give-up→読み直し→追随の効果(追随の後、**1 秒以上空けた打鍵**〈ポーリング周期〉で開に戻らないか)。
- 単体(`state/platform_state.rs`、Linux): 明示意図 ON+証拠 2 回 ⇒ 追随 / 明示意図なし ⇒ 不変 / focus 世代違い ⇒ 破棄 / 否定的証拠 1 回・StaleConfirm 混在 ⇒ 不変 / 読みが開 ⇒ 不変 / **追随後、柵より前の GJI I/O では開に戻らない** / 新鮮な開の観測と衝突 ⇒ 決めた扱い。(ii) を採る場合は B1 の対策を選んだ案ごとに具体化したテスト。
- CI: 追随後の期待値は**物理キーの文字**。親指キーを含む打鍵。IME キーで開け直す戻り(shadow-toggle の意図経路と予測経路の両方)。偽陽性ガード(`cal-d0-*-noclose-idle` の 3 つ)を 0 件で通す。長い連続入力・高速打鍵は未測定(`ts-chrome` 系に give-up 件数の列を足す)。
- 実機の確認(ユーザー環境、Chrome・Windows Terminal)は CI では置き換えられないので、修正済みとは書かない。

## 守る規約

- 再発ファミリー(warmup/IME belief/actuation 合流点)。回帰テストは上の「検証計画」の単体・CI。`docs/known-bugs/BUG-074.md` も同じ PR で更新。
- belief 書き込みは `ObserverReported` 経由のみ(`UserImeSetIntent`・`HeuristicDefault` の流用は禁止)。
- 設計の前に測る(D0 が先)。実機 Chrome の同一性が確認できるまで「修正済み」とは書かない。

## 限界

本 ADR が根拠にできるのは CI の自前 RichEdit 窓(`--form=tsf`、`profile=Imm32Unavailable`)の 10/10 だけで、元の報告(Windows Terminal)・Chrome との同一性は未確認。
