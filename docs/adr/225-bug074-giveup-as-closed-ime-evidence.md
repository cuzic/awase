---
id: ADR-225
title: |-
  RawTsfLiteralRecovery の give-up で文字が痕跡なく消える件(BUG-074)— 先に測り、方向は所有者が決める
summary: |-
  BUG-074: 外部から実 IME が閉じ belief が ON のままのとき、TsfNative×GJI の最初の打鍵は literal になり、回収も literal になって give-up し、文字が消える(CI 10/10)。
  r1 レビュー(Opus、Blocker 2・Must 7)で「give-up を Medium 観測にして drift correction に再オープンさせる」案は、明示意図があると belief が動かず、無いと drift が発火せず、元の報告(Windows Terminal・cold)では NICOLA を止めたまま戻らない、と判明し撤回した。
  r2 レビュー(Must 2・Should 4)で、give-up 後は連続カウントが戻らず以後の打鍵が全部消える見込み(コード確認済み、実測は D0-3)と判明し、「失われるのは 1 文字」を前提にした比較を改めた。
  決定: 所有者判断で追随(belief だけを実状態へ揃え IME には書かない)。D0 で偽陽性 0/30(RichEdit・実 Chrome・Windows Terminal)。Opus r4 で、追随が TsfNative の conv 推論で打ち消される恐れ(B1)・観測ソースの偽装・取り出し時点の遅れ等が判明し、設計を r5 に直した。
status: |-
  起草 r5(2026-10-04): 所有者判断=追随。Opus r4(Blocker 1・Must 4・Should 6)を反映。Blocker B1(TsfNative で conv 推論が追随を打ち消す)は D0-5 の実測待ち。実装なし。Opus r1(Blocker 2・Must 7・Should 6)・r2(Must 2・Should 4)を反映し、r3 で収束(Blocker・Must なし)。実装なし。D0 の測定と所有者の方向決定が先。
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

## 追随案の設計(r5、Opus r4〈Blocker 1・Must 4・Should 6〉を反映)

**D1(追随)**: give-up を「外部から実 IME が閉じられた」証拠として、**belief だけを実状態へ揃える**。IME には何も書かない・送信も増えない(ADR-205・ADR-212 と同じ向き)。再オープン案・通知案は採らない。

### 前提となる未解決の Blocker(B1): TsfNative では追随が次の打鍵の conv 推論で打ち消されうる

追随で `desired_open=false`・意図なし・`ObserverPoll` 相当(Medium)の状態にしても、Engine が OFF で awase が何も出力しない間に次の KeyDown で `should_run_idle_conv_check`(`src/engine/idle_check.rs:33-63`。Engine の状態を見ず TsfNative かだけが条件)が走り、閉じた IME の conv に **NATIVE ビットが残っていれば** `classify_conv_transition`(`state/conv_classify.rs:139-144`)が `ConvOpenInference(true)`(Medium)を記録し、`most_recent_trusted`(同 confidence なら新しい方)で `effective_open=true` に戻る。明示意図は捨てているので二度と追随せず、元の症状が恒久化する。ADR-205 は対象が `Imm32Unavailable` で idle-conv-check が走らないので同型ではない。

- **確認済みの前半**: D0 の close-follow のログで、外部クローズ後も `[cold-diag] pre-send conv=0x00000009 NATIVE=true`(閉じた IME の conv に NATIVE が残る)。ただしその窓(`--form=tsf`)は `profile=Imm32Unavailable`(`Chrome_RenderWidgetHostHWND`)で idle-conv-check が走らない。
- **未確認の後半(D0-5、先に測る)**: 実 TsfNative(Windows Terminal、`profile=TsfNative`)で、追随後に 600ms 以上空けた打鍵・さらに 3.5 秒(3 秒の鮮度窓)空けた打鍵で belief が開に戻るか。追随の実装を実験ブランチに入れて測る(下記)。
- **対策案(D0-5 の結果で選ぶ)**: (a) 追随の観測を `ConvOpenInference` に負けない形にする(BUG-26 との衝突を確認)、(b) 追随の後、次の明示操作かフォーカス変更まで `NativeToggleShadowOff` の推論を抑止する、(c) TsfNative を対象から外す(D0 で効く構成が無くなる)。

### 設計(B1 が解けた前提で)

1. **証拠の条件(すべて満たすときだけ)**:
   - give-up(`RawTsfLiteralRecovery` で `consecutive>=1`)であること。
   - **最後の CompositionConfirmed 以降の否定的証拠が 2 回以上、すべて `SuspectedLiteral`**(StaleConfirm でない)。`consume_literal_detect_trace`/`note_literal_detect_record` は両方の記録を受け取るので 2 回分を数える(既存の `negative_evidence_count` を配線し直す。Opus r4 M4)。
   - **取り出した時点で `explicit_intent()==Some(true)`**(give-up から取り出しまでに物理 IME キーが押されていれば条件不成立で何もしない。r4 S1)。
   - 入力先の開閉を読み戻せない構成(TsfNative/Imm32Unavailable)。Chrome の close-follow は未測定なので、測るまでは **TsfNative に絞る**(r4 S6)。
2. **観測の型(r4 M1)**: `write_observer_poll` の流用は観測ソースの偽装(`evidence.rs:218`「周期ポーリング専用」、`ObserverPoll` は `Actuating`)で、`open_warrant` Step 3 が偽の Actuating 観測から「閉」を導いてしまう。**専用の evidence 型 `Observed<LiteralGiveUp>`**(confidence Medium、`authority()=BeliefOnly`、構築子は `gave_up && SuspectedLiteral` の `LiteralDetectRecord` からしか作れない witness)を新設する。`PerSourceObservations`・journal シリアライズ・`architecture_guard` の件数ガードも更新する。
3. **動作**: `ImeStateHub::follow_literal_giveup(tick, accepted)`: `LiteralGiveUp` を記録 → 現在窓の `IntentStore` の明示意図を削除 → `pass_through_observed(align_desired=true, demote_applied=true)`。`align_desired` は `derive_any` が `Some` のときだけ効く(`ime_model.rs:935`)ので、新鮮な Medium の開の観測と衝突している場合に `desired_open` が true のまま意図だけ消える挙動を単体テストで固定し、扱いを決める(r4 S4)。追随の直後に Engine へ `RefreshState` を明示的に出す(refresh tick の外なので。r4 S3)。
4. **取り出し口(r4 M2)**: give-up の確定経路は `Output::step_probe`→`WindowsPlatform::advance_tsf_probe`(`consume_literal_detect_trace`)→ runtime の `TIMER_TSF_PROBE` ハンドラ(`runtime/message_handlers.rs:502-513`)の 1 本。**`advance_tsf_probe()` の直後**に `app.platform.take_giveup_evidence()`(`drain_journal_entries` と同じ位置・同じ型)を置き、give-up と同じ tick で追随する(`drain_output_post_send_effects` は送信の**後**にしか呼ばれず、2 打鍵目も Engine ON のまま処理されて消えるので使わない)。予約済みの BS と INPUT_DEFER の再生は後の `handle_wm_drain_output_queue`(`flush_raw_tsf_literal_recovery` の後)が行うので、保留していた打鍵は Engine OFF の状態で再生される。この順序を実装で固定する。
5. **focus 世代(r4 S2)**: 出力層の `output.ime_mode_focus_gen`(u32)と belief の `FocusFence { epoch, hwnd }` は別系統。**プローブ開始時に `ime_mode_focus_gen` を捕獲**し、取り出し時に一致を確かめてから `AcceptedObservation::for_sync(app.focus_fence())` を作る(`for_sync` 自体は照合しない。誤った世代の観測は後から除外されない)。
6. **利用者に見える入力(r4 M3、r4 の説明を訂正)**: 追随後は Engine が OFF で awase は何も送らないので、物理キーがそのまま通り、出るのは**生ローマ字ではなく物理キーの QWERTY 文字**(NICOLA 配列の「か」の位置のキーの英字)。IME が閉じていて awase が無いときと同じ出力で、方針としては正しい。失われるのは give-up した最初の 1 モーラ(BS は現状のまま残す=案K にしない)。**親指キー(無変換・変換)も IME へそのまま届く**。GJI の閉状態で変換キーが「IME を有効化」に割り当たる構成では、親指シフトを押しただけで IME が開き、`KeyEffectPredicted` が開を予測しないと Engine OFF のまま IME だけ ON になる(以後の打鍵が GJI のローマ字入力になる)。CI で追随後に親指キーを含む打鍵を確認する。
7. **効く範囲(限界、r4 S1)**: 明示意図が残るのは shadow-toggle の意図への昇格(`key_pipeline.rs:1019-1034`)か awase のコマンドで開いた後に限る。ADR-191 の予測経路(`KeyEffectPredicted`)で IME を開いた利用者は意図が常に `None` なので、**この追随は効かず元の症状が残る**。実機 journal に `explicit_intent` が載るかは未確認。
8. **戻り**: 利用者が IME キーで開け直せば通常の明示意図で ON に戻る。外部から再び開かれた場合は、TsfNative には読み戻しが無いので、次のフォーカス変更かモードキーまで OFF のまま(限界として受容)。
9. **ADR-212 との関係**: actuation は増やさない。belief の書き込み元が 1 つ増えるだけ(新 `ObservationSource` と `follow_*` 入口が各 1)。

### 検証計画(r4 S6 を反映)

- **D0-5(先に測る)**: 実験ブランチ(`ci/adr225-d0`)に追随を入れ、**Windows Terminal(`wt_probe`)で外部クローズ→1 打(give-up)→700ms→1 打→3.5 秒→1 打→Enter**。awase.log の `ConvOpenInference`/`NativeToggleShadowOff`・belief の戻りと、画面の文字を記録する。打鍵間隔は 150ms ではなく B1 を検出できる値にする。
- 単体(`state/platform_state.rs`、Linux): 明示意図 ON+証拠 2 回 ⇒ `desired_open=false`・意図が消える・`applied` が未確認へ落ちる / 明示意図なし ⇒ 不変 / focus 世代違い ⇒ 破棄 / 否定的証拠 1 回・StaleConfirm 混在 ⇒ 不変 / 新鮮な開の観測と衝突 ⇒ 決めた扱い / 追随後に conv 推論が来ても開に戻らない(B1 の対策)。
- CI: 追随後の期待値は**物理キーの文字**。親指キーを含む打鍵。IME キーで開け直す戻り(shadow-toggle の意図経路と予測経路の両方)。偽陽性ガードは `cal-d0-gji-noclose-idle`・`cal-d0-gji-chrome-noclose-idle`・`cal-d0-gji-wt-noclose-idle` を 0 件で通す。長い連続入力・高速打鍵(`ts-chrome` 系に give-up 件数の列を足す)は未測定。
- 実機の確認(ユーザー環境、Chrome・Windows Terminal)は CI では置き換えられないので、修正済みとは書かない。

## 守る規約

- 再発ファミリー(warmup/IME belief/actuation 合流点)。回帰テストは上の「検証計画」の単体・CI。`docs/known-bugs/BUG-074.md` も同じ PR で更新。
- belief 書き込みは `ObserverReported` 経由のみ(`UserImeSetIntent`・`HeuristicDefault` の流用は禁止)。
- 設計の前に測る(D0 が先)。実機 Chrome の同一性が確認できるまで「修正済み」とは書かない。

## 限界

本 ADR が根拠にできるのは CI の自前 RichEdit 窓(tsf×GJI)の 10/10 だけで、元の報告(Windows Terminal)・Chrome との同一性は未確認。
