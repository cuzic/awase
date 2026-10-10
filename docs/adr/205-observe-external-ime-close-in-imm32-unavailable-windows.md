---
id: ADR-205
title: |-
  Imm32Unavailable(Chrome 等)の窓で、外部注入の IME キーで閉じられた IME を ADR-187 の通過マーク機構で実状態へ追随する(BUG-172)
summary: |-
  実 Chrome × GJI で他プロセスが注入した 0xF3/VK_IME_OFF により IME が閉じても、awase の belief は ON のまま(observed=0、10/10)。原因は Blacklist 分岐が毎 refresh の
  prefetch 済み snapshot を捨てていることと、注入キー後の refresh が SkipTyping+明示意図でのポーリング停止で Blacklist 分岐に届かないこと。
  本 ADR は、目印なしの外部注入 IME キーで専用の短い監視窓(300ms)を立て、prefetch 済み snapshot で窓内に 1→0 の遷移を観測したときだけ
  実状態へ追随する(意図を捨て desired を揃える。awase は開け直さない)。新 I/O・新 actuation 合流点・新イベント種別なし。
status: |-
  採択・実装済み(PR #377 fd41bf88、v2.0.0 に含まれる)。CI 検証済み(GJI×実 Chrome で追随 10/10)。実機でも X5 で効果を確認(2026-09-30、dragonflyg4、docs/tasks/v2-device-verification-results-2026-09-30.md)。ただし偽 OFF の疑い1件が未解消(BUG-176)。モードキー押下後の期待状態の実機検証は未了。
  旧(2026-10-04 更新前):
  採択・実装済み(2026-09-29、`fd41bf88`)。opus-adversarial-consult round5 で観測部は収束(D7 は ADR-208 へ切り出し)。CI 検証: GJI × 実 Chrome 注入で追随 10/10。実機・モードキー押下の検証は未了。
related_adr:
  - "ADR-029"
  - "ADR-089"
  - "ADR-178"
  - "ADR-191"
  - "ADR-206"
  - "ADR-208"
---

# ADR-205: 外部から閉じられた IME を Imm32Unavailable の窓で観測する

## 背景(BUG-172、実 Chrome の測定)

[docs/known-bugs/BUG-172.md](../known-bugs/BUG-172.md) の実 Chrome 測定(2026-09-29、GitHub windows CI、各10試行、`cal-driftrec-chrome-real-*`):

- **GJI × 実 Chrome は、他プロセスが注入した 半角/全角(0xF3)・VK_IME_OFF で 10/10 再現**。IME は `IMC_GETOPENSTATUS` で 1→0 に閉じ、3秒後も閉じたまま(`open_at_probe=Some(0)`)、
  打鍵は `kiu`(ローマ字のまま。Engine は ON のまま=直接入力に落ちる)。
- awase の Chrome 開閉の**観測件数 = 0**(`ObserverPoll`=0、`Imm32Unavailable` 39件)。注入キーは hook で見えている(`injected=true`)が、
  `key_pipeline.rs` の BUG-14 分岐(`event.injected` → 「ユーザー意図に昇格させない — belief 追従は may_change_ime refresh 観測に委譲」)で昇格せず、
  委譲先の refresh が Chrome では `ir_stage_observe` の `Blacklist` 分岐(`Skipping IMM query for known-broken class`)で読まないため、belief は古い ON のまま。
- 物理キー(awase 経由)では起きない(awase が shadow-toggle で Engine も OFF にする)。メモ帳経由(アプリ間)は Chrome に影響しない(IME 開閉は窓/スレッド単位)。
- MS-IME × 実 Chrome は注入の 0xF3/0x1A を効かせず再現できていない。対象はまず GJI。
- 「ゲートへ開閉を要求する」案は実 Chrome の症状を直さないため却下済み(BUG-172)。
- 撤去済みの ADR-178 領域A(reassert・force-on)を戻しても差は出ない(対照 `ci/bug172-pre-teardown`)。回復手段の欠如ではなく**観測の欠如**が原因。

**観測経路に乗ったかの確認**(教訓): 上の 0/10 は「起きなかった」ではなく `observed=0`(判断に届いていない)。本 ADR の効果判定は verdict でなく observed 件数で行う。

## 前提となる既存コードの事実(round1 で独立再検証済み)

- **開閉は毎 refresh で既に読まれ、Blacklist 分岐が捨てている**(round1 B0、本セッションでコード確認): `runtime/mod.rs::spawn_ime_refresh` は全プロファイルで
  `read_ime_state_full_async()` を prefetch し、`ime.rs::read_ime_state_full` は `is_tsf_native_window` 以外(Chrome 含む)で `detect_ime_open_for_hwnd`(`IMC_GETOPENSTATUS`、50ms)まで読む。
  `ir_stage_observe` の `Blacklist` 分岐は `ime_snap` を参照しない。よって新しいクロスプロセス読み取り関数は要らない(初稿の `read_open_status_ungated_async` は取り下げ)。
- `read_ime_state_fast`(`ime.rs:873-885`)の「Chrome は常に 0」ゲートはこの prefetch 経路に無い。コメントの出典は `36593bcd`→`d6442c3b`(2026-04-10)で、元コミット本文の根拠は「unreliable or outright blocking」「leaked threads」であり
  「常に 0」は書かれていない。ブロック懸念は現在の offload+`run_with_timeout` で受け入れ済み。値の信頼性は下の D3 で局所的に担保する。
- BUG-14 分岐(`key_pipeline.rs:1092-1101`)は `event.injected` で `return false` するが、`kp_stage_post_decision`(`:1610-1617`)は `may_change_ime && KeyDown && !consumed` で `schedule_ime_refresh(20)` を呼ぶ。
  その 20ms 後の refresh は `ir_decide_read_strategy` で `idle_ms≈20 < TYPING_IDLE_MS(500)` かつ `explicit_verify=false`(`!skip_imm_query` が必要)なので **`SkipTyping`**(round1 B1)。
  さらに `reschedule_ime_refresh`(`runtime/mod.rs:1128-1133`)は、読めない窓で明示意図があればポーリングを止める。ハーネスの `ensure()` は目印付き VK_IME_ON で明示意図を立てるため、
  **注入後に Blacklist 分岐へ届く refresh は再現条件の中で一度も無い**(= BUG-172 の `observed=0` の本当の内訳:「Blacklist が読まない」ではなく「SkipTyping で届かない」+「明示意図でポーリング停止」+「届いても snapshot を捨てる」)。
- 効果の判定は明示意図が支配する: `effective_open`(`state/ime_model.rs:458-479`)は明示意図があれば `desired_open` を返し、観測は無視される。`desired_open` は `HwndCacheRestored`(`:727-734`)で ON に復元されうる。
  `check_drift_correction`(`state/drift_correction.rs:46-133`)は明示意図が無くても `desired` と観測の乖離で発火し(閾値 400ms〈`DRIFT_CORRECTION_THRESHOLD_MS`〉)、Blacklist では `ir_apply_drift_correction`(`:966-995`)が
  `BlacklistDriftCorrection` として実送信する(round1 B3)。**単に `ObserverPoll(false)` を書くだけでは belief は OFF に追従せず、awase が IME を開け直す**。
- `ObserverPoll` は 1 ソース 1 スロット(`observation_store.rs:451-475`)で、Blacklist 分岐内の GJI I/O 観測(`observe_gji_after_focus`)も同じスロットに `true` を書く(round1 M2)。
- TSF `ITfCompartmentEventSink` は ADR-029 で削除済み。ADR-191 の 2〜5ms 実測は自プロセス・自スレッドの compartment。BUG-172 のメモ帳測定(メモ帳で閉じても Chrome は `open=Some(1)`)が、開閉が窓/スレッド単位であることの一次根拠。

## 所有者方針(2026-09-29)と受け入れ基準

**所有者方針(決定の根拠)**: 外部から IME が閉じられたとき、awase は能動書き込み(開け直し)を一切しない。ユーザーが Ctrl+変換 / 半角全角 / 漢字 / かな などのモードキーを押したとき、
awase が belief に従って正しいキーを送ることでモードずれを解消する。**「固着する不具合は絶対に起こさない」**。本 ADR の「実状態に追随、開け直さない」(D4)はこの方針の承認を受けたものである。

**固着の定義(所有者、2026-09-29)**: **固着 = 何度モードキーを押しても状態が変わらないこと**。belief が古くて 1 回目が逆方向に効き、2 回押せば期待した状態になるのは**許容**（固着ではない）。
**受け入れ基準**: 固着（N 回押しても変化しない状態）が起きないこと = 押すたびに状態が変化するか、絶対指定キーで確実に収束すること。
検証では「2 回目で期待状態になる」を PASS、「N 回押しても変化しない」を FAIL とする。GJI と MS-IME は分けて集計する。
（トグル系で belief が古い場合の「2 回押し」は本 ADR では許容と明記し、追加の緩和は置かない。絶対指定キーが `AlreadyMatched` で握り潰され続けるケースだけが固着に当たり、ADR-208 で扱う。）

### トグル系と belief 依存の検討(所有者の懸念への回答)

awase が IME に送るのは冪等な絶対キー(GJI・MS-IME とも `VK_IME_ON`/`VK_IME_OFF`。`ime_key_sequence_golden.rs` の記述。`VK_KANJI` は使わない)なので、トグル系でも「実 IME を誤って反対へ倒し続ける」ことは無い。
方向は `!belief` で決まる。belief が古い(Chrome の開閉が観測できない、BUG-172)場合:

| 状況(belief ↔ 実 IME) | 押下 | awase が送る | 結果 |
|---|---|---|---|
| belief ON / 実 OFF(外部 close 後) | トグル(ON にしたい) | `VK_IME_OFF`(belief ON の反転) | 見た目は変化なし、belief は OFF に。 |
| 同上 | トグル(2 回目) | `VK_IME_ON` | 実 ON。**2 回押しで直る。固着ではない**。 |
| belief OFF / 実 ON(外部 open 後) | トグル | `VK_IME_ON` | 見た目変化なし、belief ON(エンジン活性)。2 回目で OFF。同上。 |
| belief ON / 実 OFF | 絶対 ON(Ctrl+変換) | **`applied` が ON なら `gji_direct_already_matches` により送信を省略** | **実 OFF のまま。何度押しても直らない = 固着**(BUG-156 と同型の「古い記録を根拠に送信を省く」) |
| belief OFF / 実 ON | 絶対 OFF（GJI: applied が Some(false) なら `gji_direct_already_matches` で省略。なお ADR-206 の決定3(b)〈Consume のみ〉は ADR-206 側で撤回済み〈2026-09-29〉） | 何も送らない | 実 ON のまま。何度押しても直らない = 固着 |

したがって **GJI では、トグル系は 2 回押しが最悪で固着に当たらない**（注: 「AlreadyMatched で省略」は `GjiDirect` にだけある。MsImeDirect は常に送る。ただし **MS-IME × 実 Chrome では awase 自身の `VK_IME_OFF` が効かなかった測定がある**〈BUG-172 の対照、目印付き 0xF3 で 10/10 閉じず〉ため、MS-IME のトグル系は 2 回押しでも直らない可能性があり、ADR-205 とは独立の既存の制限。実機確認を第0段に加える）。**固着に当たるのは絶対指定キーが「belief/applied を根拠に送信を省く」場合**であり、所有者の「絶対指定は belief が古くても確実」という前提は、
現行コードでは `applied` の AlreadyMatched 省略(GJI)に対しては成り立たない。緩和を D6（検出できた場合）と ADR-208（検出できない場合）に置く。なお表の Ctrl+無変換（GJI、belief OFF/実 ON）も同じ握り潰しに当たる（ADR-206 (b) に限らない）。

## 案の比較

| 案 | 内容 | 実 Chrome の症状に効くか | 副作用・リスク | 判定 |
|---|---|---|---|---|
| A | `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` の通知購読(WH_CALLWNDPROC による他プロセスへの注入も含む) | 効かない見込み。thread/global compartment に Chrome のスレッドの変化は載らないと考えるのが自然(メモ帳/Chrome 独立の測定、ADR-029) | COM sink・寿命管理、注入は侵襲的すぎる | 不採用(再検討防止のため記録) |
| **B'** | **prefetch 済み snapshot の開閉を、外部注入 IME キー後の限定窓で ADR-187 の通過マーク機構に載せて追随する**(D1〜D4) | 効く見込み(実 Chrome で 1→0 が読めることは測定済み、awase 自身の prefetch も同じ読み) | 偽の OFF(D3 で局所化) | **採用** |
| C | フォーカス復帰時に HwndCache の ime_on を鮮度切れ扱い | 効かない(再現条件はフォーカス復帰を伴わない) | 復帰のたびに belief が揺れる | 不採用 |
| D | 何もしない・既知の制限として記録 | 効かない | なし | B' が却下された場合の代替 |
| E | `SetWinEventHook`(`EVENT_OBJECT_IME_*`) | 効かない見込み(IME UI/変換対象のイベントで開閉ではない、GJI TSF では発火しない) | — | 不採用 |
| F | 常時ポーリング拡大・打鍵ごとの読み取り | 効くが、`TYPING_IDLE_MS` の設計と refresh 頻度を悪化させる(読み自体はワーカー offload 済みでフックはブロックしない) | 高 | 不採用 |
| G | Blacklist 分岐で毎回 snapshot を belief に反映 | 効きうるが偽 OFF が常時露出(入力欄/本文で HIMC の付け外しが起きる仮説、M1) | 高 | 不採用(B' は外部注入キー後の窓内に限定) |

## 決定

方針: **「IME の実状態が真実」**(ADR-191)。外部が閉じたなら awase の belief と desired を実状態へ揃え、awase が開け直して外部の操作に逆らうことはしない(BUG-14: 注入キーはユーザー意図に昇格させない、の延長)。
**追随するのは「窓の中で 1→0（GJI のときは 0→1 も）の遷移を実際に観測したとき」だけ**(round2 で、arm 時点の値による readable 判定を、窓内の遷移観測へ置き換えた)。「常に 0」の環境では遷移が起きないので何も変わらない。
既存の ADR-187 通過マーク自体は使わない(round2 M4: `readable_at_arm` の副作用〈観測が全部失敗したら窓の終了で意図を捨てる、「常に 0」でも観測成功で desired を 0 に揃える〉と、既存の物理モードキー通過の挙動変更を避けるため)。
追随の最後の一手だけ既存の `ImeEvent::ModeKeyPassedThrough{align_desired:true}`(唯一の構築点は `ImeStateHub::pass_through_observed`)を再利用する。

### D1. 外部注入 IME キーで「外部クローズ監視」(`ExternalCloseWatch`)を立てる

`ImeStateHub` に `Option<ExternalCloseWatch{armed_at_ms, scope, saw_open: bool}>` を1つ足す。`kp_stage_post_decision` の `may_change_ime && KeyDown && !consumed` 経路で、次の**両方**を満たすとき立てる:
1. `event.injected`(hook の `is_injected` = LLKHF_INJECTED かつテスト目印なし)。awase 自身の注入は含まない。
2. Blacklist 窓(`!can_use_imm32_cross_process()`)。

除外(明示操作直後は arm しない)は**置かない**(round3 R3-2): 追随は 1→0 の一方向だけで、MS-IME/CTF の注入がユーザーの明示 OFF の直後に来ても、観測される 1→0 は awase 自身の OFF が効いた瞬間で追随先は OFF(ユーザー意図と一致)。
明示 ON の直後なら観測は 1→1 か 0→1 で追随しない。BUG-14 型の上書き(OFF 意図を ON で潰す)は構造的に起きない。ハーネスは VK_IME_ON から注入まで約 1.1〜1.3 秒で、除外(1500ms)を置くと再現条件そのものを弾いてしまう。
窓の中で注入キーが連続する場合(0xF3 の2連、CTF の 0xF0 up + 0xF2 down)は、同じ scope なら `saw_open` を保持したまま `armed_at_ms` だけ延ばす(round3 m2)。
`VK_IME_ON/OFF`(0x16/0x1A)も対象(方向を awase が決めない注入ではこれが再現キー)。既存の 20ms 後の refresh(`schedule_ime_refresh(20)`)がそのまま最初の読みになる。

窓は既存の `MODE_KEY_PASS_MARK_WINDOW_MS`(300ms)を流用し、`foreground_scope` が変わったら失効(通過マークと同じ)。**GJI が注入キーで実際に閉じるまでの時間が 300ms に収まるかは未測定**(ハーネスは KeyDown の 360ms 後に閉状態を確認しただけ)。第0段の trace で prefetch の読みの時刻と値の列を測り、
収まらなければ窓の定数を実測(ms)を根拠にして別途決める(tuning-constants)。収まる場合は新しい定数は作らない。

### D2. 監視は prefetch 済み snapshot の消費だけ(追加 I/O なし)

`ir_stage_observe` の strategy `match` の**外**に `ir_watch_external_close(ime_snap)` を1段足す(SkipTyping でも走る。窓は 300ms と短く、`observe_gji_after_focus` など Blacklist 分岐の他の書き込みは打鍵中に走らせない。round2 m3)。
- 純粋関数 `observer/ime_observer.rs::classify_external_close(watch_saw_open: bool, snap_open: Option<bool>) -> ExternalCloseVerdict`
  (`NoEvidence`〈`None`〉/ `SawOpen`〈`Some(true)`〉/ `Closed`〈`saw_open` かつ `Some(false)`〉/ `IgnoreZero`〈`!saw_open` かつ `Some(false)`〉)。
- `SawOpen` は watch の `saw_open=true` を立てるだけ(belief に書かない)。`IgnoreZero` は捨ててログ(`[external-close] ignored 0 without prior 1`)。
- `Closed` のとき: `ImeStateHub` に新設する 1 メソッド `adopt_external_close(tick, accepted)` を呼ぶ(round3 R3-1)。中身は `write_observer_poll(false, ..)` → **`intent_store.remove(current_focus)`**(`effective_open_at` は IntentStore の意図を shadow_model より優先し TTL は約30秒。物理キー経由の VK_IME_ON の意図が残ると belief が ON のままになる。既存の通過マークの経路も `drop_intents_for_mode_key_pass_in_scope` で同じ除去をしている)→ private の `pass_through_observed(tick, true)`(`shadow_model.last_intent` を捨て、`derive_any` から `desired_open` を実状態へ揃える)。最後に watch を解除。`pass_through_observed` は private なので `runtime/` からはこのメソッド越しにだけ呼べる。回帰テスト: IntentStore に ON の意図がある状態から Closed 後に `effective_open()` が false。
  （round5 で変更）閉じる方向(1→0)だけでなく**開く方向(0→1)も同じ規則で追随する（ただし 0→1 の追随は GJI が有効な間に限って始める。MS-IME/CTF 自身の注入が IME を開いた場合に Engine が ON になり、MS-IME では次の絶対 OFF が効かない可能性があるため。round5）**。理由: 実状態が真実、かつ ADR-206 (b) が「belief OFF の OFF キーは何も送らない」ため、外部 open で belief OFF/実 ON のまま残ると固着する（上の表）。BUG-14 型の上書きは、追随先が常に「窓の中で実際に読んだ値」であり stale な値へ揃える経路が無いので、双方向でも起きない。`classify_external_close` は `ExternalStateVerdict::{NoEvidence, Baseline(bool), Changed(bool), IgnoreFirst(bool)}` の形に一般化する（`Baseline`＝窓内の最初の `Some` の読み〈代案を採る場合のみ、同じ scope で直近の値〉を保持、`Changed(v)`＝ベースラインと逆の値を読んだ）。
- `reschedule_ime_refresh`: watch が生きている間は、明示意図の停止(`runtime/mod.rs:1128-1133`)より前で `MODE_KEY_PASS_REREAD_MS`(60ms)の読み直しを予約する。窓が切れたら watch を破棄して従来どおり(意図は捨てない)。
- **第0段の実測（run 36545236017、`cal-driftrec-chrome-real-hz-ext-gji`、GJI × 実 Chrome、trace ログ）**: 注入 0xF3 の KeyDown は 08:53:20.826、`may_change_ime key passed through → IME refresh scheduled (20ms)` が出て Engine は消費しない。
  最初の prefetch 読みは KeyDown の **32ms 後（20.858）で既に `open=0`**、strategy は `SkipTyping`（idle=32ms）、`explicit_intent=Some(true)`。つまり **GJI は 32ms 以内に閉じ、窓内の読みは最初から 0** で、`saw_open` を窓内の読みだけで立てる方式は効かない。
  注入前の直近の読みは 19.670 の `open=1`（1.16 秒前、目印付き VK_IME_ON の 20ms 後の refresh）。よって **R3-3 の代案を採用する**: 全 refresh の入口で `foreground_scope` 付きの直近 prefetch 値を1つ記録し、arm 時にそれが `Some(true)` なら `saw_open` の初期値を true にする。
  採用条件は「注入 IME キー直後の窓の中で実際に 0 を読んだ」ことのままなので、古い 1 を検証と誤認する危険（round2 M1）は、HIMC の付け外しと注入キーが同時に起きない限り増えない。プローブ側の `open_before=Some(1) → open_after=Some(0)` と prefetch の 1→0 は一致（(d) 確認）。
  窓 300ms は閉じるまでの実測 32ms 以内に収まる（新しい定数なし、tuning-constants の対象外）。
- Closed の後の `observe_gji_after_focus` の `ObserverPoll(true)` 上書き(round3 m1): その第1引数を `max(last_focus_change_ms, last_external_close_ms)` にして、閉じる前の GJI I/O を無視する(実装時。GJI が閉じるときに I/O を出すかは第0段の `[gji-poll]` で確認)。

### D3. 「常に 0」説と「1→0 が読める」説の両立

遷移(同じ窓の中で 1 の後に 0)を観測したときだけ追随するので、どちらが正しくても安全側:「常に 0」なら遷移が起きず従来どおり。読めるなら本物の閉じを拾う。偽 OFF の露出は
「外部注入 IME キー直後の監視窓（300ms。連続注入で延長されると最大 900ms＝最初の arm から 2W まで延長 + W）の間に、値が 1→0 と動いた」場合に限られる。入力欄/本文の HIMC 付け外し(round1 M1 仮説)が同じ窓の中で起きる確率は低いが、ゼロではないので e2e で確認する(下記)。

### D4. 書き込み・合流点・定数

新しい actuation 合流点なし。`ImeEvent` の新 variant なし(`ModeKeyPassedThrough` の構築点は `pass_through_observed` のまま)。`ObserverPoll` は既存の `write_observer_poll` 経由。新しい `_MS` 定数なし(D1 の窓が実測で収まる場合)。
awase は IME を開け直さない。ADR-178 領域A撤去・ADR-191 の方針(能動書き込みを足さず観測に従う)に沿う。

### D5. 対象範囲（PR #377 Opus レビュー 1・2 で訂正）

**`AppImeProfile::Imm32Unavailable` かつ有効な IME が GJI の窓だけ**（arm・追随の両方、述語 `Runtime::external_change_watch_applies`、architecture_guard で固定）。
- MS-IME を除く理由: CI 実測で MS-IME × 実 Chrome の開閉の読みは常に 0 で信用できず、GJI で ON の読みを記録した後に Win+Space で MS-IME へ切り替え、注入キー（AHK や CTF の 0xF0/0xF2）で窓が開くと、古い基準値との差で偽の Changed(false) になり得る。
  閉じる方向も GJI に限る（レビュー 1(a)）。(b) スコープに IME 種別を含める／(c) 読みに時刻上限を付ける案は、(a) で足りるため採らない（GJI 以外へ切り替えた後に GJI へ戻った場合の基準値は、戻った後の最初の窓の中の読みか、GJI 有効中の直近の読みで決まり、実状態と一致する）。
- **InputRelay は対象外**（awase が actuation を所有しない、BUG-90 決定4 条件(c)）。以前の記述「InputRelay は `ime_on=None`」は事前読み取り経路（`read_ime_state_full` は TsfNative だけ `None`）では事実と違ったため、プロファイル比較で明示的に除く。TsfNative は読みが `None` で影響を受けない。
- ADR-193 の CI 入力先（RichEdit を `Chrome_RenderWidgetHostHWND` 名でスーパークラス化したもの）は Imm32Unavailable なので、GJI のとき対象に入る。

### D6. 外部 close の追随で `applied`（awase 自身の書き込み記録）も実状態に合わせる

`adopt_external_close`（0→1 も含めて `adopt_external_change`）は、`ModeKeyPassedThrough` の reducer 腕（または同等の 1 か所）で、観測値と `applied` が食い違うとき `applied = Unknown` に落とす
（`KeyEffectPredicted` 腕が BUG-156 で既にやっている規則の再利用。「送信を省略してよいか」は陽性の確認済み証拠にだけ基づく〈ADR-098 決定1-b〉）。これで、検出できた外部変化の後は Ctrl+変換 が `AlreadyMatched` で握り潰されない。

### D7. 検出できない stale に対する絶対指定キーの保証は ADR-208 に切り出す

検出できなかった外部変化（アイコン操作など注入キーを伴わない変化）では `applied` が古いまま残り、GJI の絶対指定キーが `AlreadyMatched` で握り潰される。これは actuation の判断を変える変更で、本 ADR の主題（観測だけを足す）の外なので、
**[ADR-208](208-absolute-ime-keys-must-not-be-elided-on-stale-applied-in-blind-windows.md)（起草）へ切り出す**（round5 の推奨。置き場所〈ADR-206 の要請: 対象に「bare_ime_action または forced_open_action を持つ親指の非リピート Down」を含める〉・書き込み口・TsfNative の「@」・MS-IME の実測を含め ADR-208 で検討。ADR-206 は決定3(b) を撤回し、OFF 方向を常に絶対指定 SetOpen(false) で書く。代償の単発 VK_IME_OFF の「@」は ADR-206 側で実機 A/B をマージ条件にしている。出荷順は ADR-208 と同時か後）。
本 ADR 単体で保証するのは「検出できた外部変化の後は絶対指定キーが握り潰されない」（D6）まで。

### 受け入れ基準の対象外（明記）

InputRelay（awase は actuation を持たない）。ATOK/未同定 IME × Blind 窓（GJI/MS-IME の actuation を持たず、物理キーは Allow で awase は書かない）。TsfNative の絶対 OFF は ADR-208 で扱う。
Blind 窓で学習表が「開閉トグルではない」とする半角/全角や 0xF0/0xF1（常に Allow）は IME が自分で処理し belief は打鍵予測（ADR-191）に従う。予測が外れても観測では訂正されない（watch は注入キーでしか立たない）が、
次の絶対キー（ADR-208 の範囲）で直るので固着ではない。watch を物理キーの Allow 通過にも立てる拡張は将来課題。

## リスクと検証計画

| リスク | 対策・確認 |
|---|---|
| 偽の OFF で Engine が誤って OFF になる(最大のリスク) | D3: 外部注入 IME キー直後 300ms の窓の中で 1→0 の遷移を観測したときだけ。通常打鍵の e2e で「追随」ログが 0 件であること、加えて idle 500ms 超を挟み「ページ本文→入力欄→即打鍵」「omnibox 往復」のシナリオで偽 OFF が 0 件であること(未実施、要追加)。 |
| 読み取りがブロックする | 既存 prefetch(50ms + offload)のまま。新しい I/O は無い。 |
| AutoHotkey 等で意図して閉じた IME を awase が開け直す | 開け直さない(D4)。desired を観測へ揃える。 |
| 通過マークの副作用(明示意図の破棄・既存の物理モードキー通過の挙動変更) | 通過マークを使わず専用の watch にした。遷移を観測したときだけ意図を捨てる(観測なしでの破棄なし)。 |
| 物理 IME キー(目印付き)の経路への影響 | D1 は「目印なしの注入」のみ。目印付きは従来の shadow-toggle(awase が Engine も OFF)。 |
| MS-IME・edit・他アプリ | Standard(OsPoll)は無変更。MS-IME は再現できていない(注入の 0xF3/0x1A が効かない)が、CTF の注入で watch は立ちうる——追随は 1→0 の観測時のみ。 |
| 既存の複数窓口(fix-requires-evidence の表) | watch の arm/consume/expire/reschedule の各窓口(`kp_stage_post_decision`、`ir_stage_observe`、`reschedule_ime_refresh`、フォーカス変更での失効)すべてに配線したか、architecture_guard の件数で固定する。 |

**実装の第0段(コード変更なし、推奨)**: 既存ハーネス(`cal-driftrec-chrome-real-hz-ext-gji`)を trace レベル(`ime.rs::detect_ime_open_for_hwnd` の `CrossProcess(hwndFocus)`)で1回走らせ、
注入前後の prefetch 値(1→0)、注入後の refresh が `SkipTyping` であること、明示意図が `Some(true)` であることを確認する。前提(B0/B1/B3)の実測での裏取りで、「観測経路に乗ったか」の確認を兼ねる。

**回帰テスト**(host で走るもの): `classify_external_close` の表(1→0 で Closed、0 のみは IgnoreZero、None は NoEvidence)、「明示 OFF 直後の注入 0xF2 で 1→0 → OFF のまま(整合)」「明示 ON 直後の注入は追随しない」の2本、watch の連続 arm(saw_open 保持)、窓切れ・scope 変化での失効、
`state/drift_correction.rs` の closed_loop で「Closed 追随後は drift が発火しない」を固定。architecture_guard: `ModeKeyPassedThrough` の構築点が `pass_through_observed` のみであること、watch の arm 呼び出し元が1か所であること。

**実機・CI**: 測定用ブランチから `gh workflow run e2e-ime.yml --ref <branch> -f only='cal-driftrec-chrome-real-*'`(乱発しない)。
ハーネスの PASS は「開け直して NICOLA が出た」を意味し(`got == Class::Nicola`)、本 ADR の期待(追随して Engine も OFF、`ka` で一貫。物理キー対照と同じ)とは逆なので、**判定を書き換える**:
`kiu`(不整合)0/10 を合格、`ka`(一貫した OFF)を許容、`Nicola`(開け直し)は想定外として別計上。効果指標は新経路専用ログタグ(追随〈Closed〉した件数・`IgnoreZero` で見送った件数)で数え、`ObserverPoll` の総数は使わない(GJI I/O の `true` が混ざるため)。
対照: 目印付き 0xF3 と MS-IME が退行しないこと、通常の Chrome 打鍵(`ts-chrome`)で追随ログ 0 件。

## 未決事項(所有者判断の候補)

1. 方針「IME の実状態が真実、awase は開け直さない」(D4)の確認。従来の drift correction 型の「開け直し」(ハーネスの旧 PASS 定義)を望む場合は設計が変わる。
2. 取りこぼし(窓の最初の読みが既に 0 のとき追随しない)の許容。第0段の結果次第で D2 の代案(arm 前の直近値)を採る。
3. 専用 watch による追随(BUG-14 の「注入はユーザー意図にしない」との整合。意図は昇格させず、実状態への追随だけを行う)。

## 敵対レビューの記録

### round1(Opus、2026-09-29): blocker 4・major 3・minor 3

- B0 `read_ime_state_full` の prefetch が Chrome でも開閉を読み、Blacklist 分岐が捨てている → 反映(新 I/O 関数を取り下げ、snapshot 再利用)。
- B1 注入後の refresh は `SkipTyping`+明示意図でポーリング停止 → 反映(D1/D2: 通過マーク、`explicit_verify` の拡張、読み直し予約)。
- B2 フォーカス確定後の読みではハーネスで検証されない → 反映(D3: 全 refresh で直近値を記録)。
- B3 単に `ObserverPoll(false)` を書くと awase が開け直す → 反映(方針決定と D2 の align)。
- B4 ハーネスの PASS 定義が逆 → 反映(合格条件の書き換え)。
- M1 クラス単位 latch の穴 → 反映(フォーカス世代の直近値に変更)。M2 `ObserverPoll` スロット競合 → 反映(書き込み順を固定)。M3 fence → 反映。
- m1〜m3(参照の正確さ、案 G/H の見落とし、ガバナンス)→ 反映。

### round2(Opus、2026-09-29): blocker 0・major 4・minor 3(未収束)

- 方向(実状態が真実、開け直さない)は妥当。ObserverPoll(false) 単独で desired=false になり drift が起きないことを reducer 側で独立に確認された。
- M1 arm 時点の直近値による readable は古い値/競合に依存 → 反映(窓内の 1→0 遷移観測へ変更。世代状態は廃止)。
- M2 最初の読みが閉じる前の 1 だと以後の 0 を採用しない → 反映(遷移観測は 1 で監視を続ける。60ms 読み直しを窓の間予約。窓 300ms に GJI の閉じが収まるかは第0段で測定)。
- M3 CTF 自身の注入で BUG-14 型の上書き → 反映(EXPLICIT_IME_SUPPRESS_MS の間は arm しない。かつ追随は 0 方向のみで 1 に揃える経路が無い)。
- M4 通過マークの `readable_at_arm` 副作用・物理モードキー通過の挙動変更 → 反映(通過マークを使わず専用 watch、`ModeKeyPassedThrough` の再利用のみ)。
- m1 D5 の誤り → 訂正。m2 世代の定義 → 廃止で解消。m3 打鍵中の GJI I/O 書き込み → strategy の外で消費して回避。

### round3(Opus、2026-09-29): major 3・minor 3(未収束、修正は局所的)

- R3-1 `pass_through_observed` だけでは IntentStore の意図が残り belief が ON のまま → 反映(`adopt_external_close` 新設、IntentStore 除去、回帰テスト)。
- R3-2 明示操作 1500ms の除外がハーネスの再現条件を弾く(VK_IME_ON から注入まで約 1.1〜1.3 秒)、しかも 1→0 限定の設計では不要 → 反映(除外を撤去)。
- R3-3 最初の読みが既に 0 の取りこぼしの競合 → 第0段で確定、代案(scope 付き直近値で `saw_open` 初期化)を D2 に記載。
- m1(GJI I/O の上書き)・m2(連続 arm は `saw_open` 保持)・m3(文書の残骸)→ 反映。

### round4(Opus、2026-09-29): **収束**(blocker 0・major 0・minor 5、実装時に対応)

- adopt_external_close の順序(`write_observer_poll` → `intent_store.remove` → `pass_through_observed`)、IntentStore の除去対象(`current_focus`)、`last_external_close_ms` の柵の意味、連続 arm の規則を独立に確認。順序はテストで固定する。
- n1 awase 自身のトグル actuation による 1→0 で、ユーザーの明示 ON を捨てうる → watch に `awase_wrote`(ADR-187 の `note_awase_write` と同型)を持たせ、窓の中で awase が書いていたら ObserverPoll(false) の記録だけにして意図除去・desired 揃えはしない(送信がトグルかは `characterize_strategy` で実装前に確認)。
- n2 D5 の残骸 → 修正済み。n3 注入が続くと窓が延び続ける → 延長は `saw_open` が未成立の間だけ、かつ最初の arm から窓の2倍を上限とする。
- n4 KeyEffectPrediction(170ms 柵)が Closed を隠しうる → `adopt_external_close` で key_effect の open 予測も消すか、起きないことをテストで固定する。
- n5 第0段の確認項目: (a) 注入キーが Engine に消費されず 20ms の refresh が予約される (b) GJI が閉じるまでの時間と窓の最初の読みの値 (c) 閉じるときの GJI I/O (d) prefetch が chrome_probe と同じ 1→0 を読む (e) 目印付き VK_IME_ON が IntentStore に意図を記録する (f) MS-IME × 実 Chrome で awase 自身の `VK_IME_OFF`(目印付き 0xF3 の Suppress→MsImeDirect)が実際に IME を閉じるか。

### round5(Opus、2026-09-29): 観測部は健全、D7 が未収束（major 3）→ D7 を ADR-208 へ切り出し

- 観測部（双方向化を含む watch、BUG-14 型の不発）は健全と確認。0→1 の追随は GJI 限定で始める（反映済み）。
- M5-1 D7 の置き場所は `kp_stage_shadow_ime_toggle` の入口では Ctrl+変換（エンジンのコンボ）に効かない、`applied` の書き込み口は reducer 経由で新 event が要る（D4 と両立しない）。M5-2 D7 を TsfNative に適用すると BUG-124 の「@」の構成を作り直す。
  M5-3 MS-IME × 実 Chrome では awase の `VK_IME_OFF` が効かない測定があり、受け入れ基準が belief と無関係に満たせない可能性。→ いずれも ADR-208 に移し、本 ADR は観測+D6 で収束扱い（D6 の影響範囲は `adopt_external_change` 専用の経路に限る）。
- n1〜n4（D5 の残骸、表の脚注、件数ガード〈ime_model.rs の applied 直接代入 6→7〉、GJI/MS-IME の分離集計）を反映。

### 第0段の実測結果（2026-09-29、run 36545236017、GJI × 実 Chrome、1試行目）

(a) 注入 0xF3 は Engine に消費されず 20ms の refresh が予約される: 確認。(b) GJI が閉じるまで <32ms（最初の読みで既に 0）→ arm 前の直近値を `saw_open` の初期値にする（D2）。
(d) prefetch（フォーカス HWND）は chrome_probe と同じ 1→0 を読む: 確認。(e) 目印付き VK_IME_ON が明示意図を記録する（`explicit_intent=Some(true)`）: 確認。
(c) 閉じるときの GJI I/O: 該当ログなし（`[gji-poll]` は SkipTyping で走らない）。(f) MS-IME × 実 Chrome の awase `VK_IME_OFF` は本 run の対象外（ADR-208 の前提として別 run が要る）。
補足: 3秒後の打鍵の後、GJI の `Reopen(BeliefSync:shadow-noop)` の reinit が走り、次の読みは `open=1` に戻った（既存の GJI 経路による再オープン。watch の窓の外なので追随の対象外）。

### 実装後の CI 検証（2026-09-29、`ci/bug172-step0-trace` = 実装 `fd41bf88` を測定ハーネスへ merge、各10試行）

| 構成 | 追随（`[external-change]` 件数） | 3秒後の打鍵 | 従来 |
|---|---|---|---|
| hz-ext-gji（注入 0xF3、run 36548371652） | **10/10** | `ka`（IME OFF と一致）10/10 | `kiu` 10/10 |
| imeoff-ext-gji（注入 0x1A、run 36547215222・36548761653） | **10/10** | `ka` 10/10 | `kiu` 10/10 |
| hz-phys-gji（目印付き=物理キー相当） | 0（awase 経由で Engine も OFF、従来どおり） | `ka` 10/10 | 同じ |
| np-hz/np-wm × GJI/MS-IME（メモ帳側で閉じる） | 0 | NICOLA 継続 10/10 | 同じ |
| hz-ext/imeoff-ext/hz-phys × MS-IME | 1/0/0 | 9〜10 回は IME が閉じず NICOLA 継続、閉じた回は `ka` か従来の `kiu`（各1件） | 概ね同じ |

- observed（追随）件数は GJI × 注入で 0 → 10/10。ハーネスの PASS 判定は「開け直して NICOLA」なので、追随後の `ka` は FAIL 表示のまま（期待どおり。判定の書き換えは未実施）。
- **「常に 0」は MS-IME × 実 Chrome で実在した**: MS-IME 構成の prefetch は IME が開いているセットアップ中も `CrossProcess(hwndFocus) open=0` を返し続ける（`imeoff-ext-msime-native` のログ）。基準値 0 のままなので遷移が起きず追随しない＝偽の OFF を採用していない（D3 が実環境で効いた）。GJI の prefetch は 1→0 を正しく読む。
- 未検証: (1) 追随後にモードキーを押して期待状態になるか（受け入れ基準の CI 検証。ハーネス未実装）。(2) MS-IME × 実 Chrome の awase 自身の `VK_IME_OFF`（ADR-208 の前提）。(3) 実機。(4) 通常打鍵・入力欄/本文移動での偽追随 0 件の長時間確認（今回の対照 10 構成では偽追随 0）。

### PR #377 の Opus コードレビュー（HEAD 1bc16b37）への対応

1. 偽 OFF（基準値が古い＋MS-IME）→ 閉じる方向も GJI × Imm32Unavailable に限定（D5、(a) を採用。(b)(c) は不要と判断した根拠は D5）。
2. InputRelay の混入 → 述語で Imm32Unavailable に絞る（D5 訂正、architecture_guard で固定）。
3. D6 が共有 reducer の全経路に効いていた → `ModeKeyPassedThrough` に `demote_applied: bool` を足し、追随経路（`follow_external_change`）だけ `true`。ADR-187 の通過マーク・BUG-163 の揃えは `false` で従来どおり（variant は増やしていない、既存テストの期待値も元に戻した）。
4. テストの穴 → `follow_external_change` の単体テスト3件（IntentStore に ON の意図があっても Changed(false) 後に `effective_open()==false`／窓の外は追随しない／0→1）、適用窓の限定を固定する architecture_guard を追加。
5. 追随の不発 → 既知の制限として記載: `ImeSnapshot` は読み取り時刻を持たないため、窓が開く前に読み始めた読みが反映時点の時刻で窓の中として扱われ、基準値と違うと（注入前の値で）Changed になって窓を閉じ、注入による本当の変化を取りこぼしうる（誤った方向へは書かない）。
   窓の寿命の記述は「最大 3W」に訂正（doc と D3）。

### round6（Opus、PR #377 の対応差分 92fe08b2）: M6-1 を直せば収束

- M6-1 述語を `can_use_imm32_cross_process` の `#[must_use]`/`#[track_caller]`/doc の間に挿入して属性が剥がれた（ADR-158 TE3 の呼び出し元記録が壊れる）→ 述語をラッパの後ろへ移し、`can_use_imm32_cross_process_wrapper_keeps_track_caller` ガードを追加。
- belief 更新範囲（`demote_applied=true` は追随経路だけ、構築点は `pass_through_observed` の1か所）、GJI × Imm32Unavailable への限定は意図どおりと確認された。
- 追加テスト: ハブ経路で追随後に `applied` が未確認へ落ちること。GJI→MS-IME→GJI の往復で古い基準値が残る件は、採用されるのが常に窓内で読んだ現在値であり実状態への追随になるため害は小さい（IME 種別変更で基準値を捨てる案は採らない）。
- MS-IME の読みの根拠: run 36548761653 `imeoff-ext-msime-native` の trace で、IME が開いているセットアップ中も `CrossProcess(hwndFocus) open=0` が続いた。BUG-172 の「chrome_probe は MS-IME も 1→0 を読めた」とは測定方法（トップレベル窓と awase の hwndFocus 経路）が異なる可能性があり、MS-IME を外す判断は M5-3（awase 自身の VK_IME_OFF が効かない測定）だけでも正当化できる。

## 2026-10-08 追記(ADR-244)

直接観測(ADR-188)は ADR-244 で `table_ime_kind()`（GJI／同定済み MS-IME 本体）の範囲へ広げた（MS-IME 本体は英数の軸だけを採る）。本 ADR の外部変化の監視（開閉の基準値を持つ）は GJI 限定のまま——上記の「MS-IME × 実 Chrome の開閉の読みが 0」の懸念は、基準値を持たない直接観測が英数の軸だけを見ることで踏まない。この GJI 限定の理由は撤回していない。
