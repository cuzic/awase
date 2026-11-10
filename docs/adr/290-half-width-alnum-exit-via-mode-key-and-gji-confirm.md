---
id: ADR-290
title: |-
  半角英数トグルをモードキー(ひらがなキー等)で抜けたときもtoggle_heldを正しく解除し、GJI側の抜け出し送信にもMozcの実際の切替完了の確認を足す(BUG-230)
summary: |-
  `half_width_alnum.rs`の`toggle_held`は、2回目の左Shiftタップ・右Shift緊急解除・ADR-245の窓復帰時の
  復元の3経路でしか`false`に戻らない。ひらがなキー(F2等)は`kp_stage_mode_key_follow`の汎用PassThrough
  経路(ADR-187/191)を通り、この経路は`half_width_alnum`の状態を一切見ない・更新しない。結果、ひらがなキー
  で半角英数から戻ったつもりでも`toggle_held`は`true`のまま残る。
  さらに、GJIの抜け出し(`kp_send_gji_restore_exit`)自体に、Mozcが実際にモード切替を完了したかの確認が
  無い(MS-IMEのような640ms IMC writeリトライループが無い。この関数自身のdocコメントが「SendInputが
  見送られた場合」の実害をすでに警告しているが、**SendInputが成功した場合の遅延**は未対応)。実機では
  F2送信からMozc側の実際の切替完了まで約2秒かかり、この間もawaseのbeliefは即座に「成功」として
  romajiを送り続け、まだ半角英数のMozcがそれをリテラルな英字のまま出力する(BUG-230、2026-11-10実機)。
  D1でtoggle_heldのbookkeepingを直し、D2でこの抜け出し後の最初の送信をADR-270の既存の確認・回収
  機構(`MozcComposeCheckFsm`)に乗せる(`should_check_compose`の`is_composing_mode`前提を、この送信
  1回だけ迂回する)。
status: |-
  起草(2026-11-10)。Opusレビュー前。実装・実機確認とも未着手。
related_adr:
  - "ADR-270"
  - "ADR-289"
  - "ADR-245"
  - "ADR-244"
  - "ADR-107"
---

# ADR-290: 半角英数トグルのモードキー脱出とGJI確認の欠落(BUG-230)

## 背景

[BUG-230](../known-bugs/BUG-230.md)の実機報告(dragonflyg4、2026-11-10T08:09:56-08:10:02 UTC):
左Shift単独タップで半角英数トグルに入り「Excel」を入力後、ひらがなキー(F2)を押して日本語入力に
戻したが、続けて入力した日本語がローマ字のまま(NICOLA変換されず)出力された。

### 原因1: `toggle_held`がひらがなキーで解除されない

`crates/awase-hook-core/src/state/half_width_alnum.rs`で`toggle_held`を`false`に戻す箇所は
2か所だけ(`:430`・`:452`)で、どちらも「2回目の左Shiftタップ」「右Shift緊急解除」
(`on_shift_up`のExit分岐)、またはADR-245の窓復帰時の復元(`kp_restore_kana_from_half_width`、
`ExitOwnership::FromResume`)からしか呼ばれない。ひらがなキー(vk=0xF2)は`shadow_action`を持たない
ため`kp_stage_mode_key_follow`(ADR-187/191、モードキーをFSMに通さずPassThrough判定でOSへ渡し、
20ms後に`KeyEffectPrediction`でbeliefを進める汎用経路)を通るが、この経路は`half_width_alnum`の
状態を一切参照・更新しない(`crates/awase-windows/src/runtime/key_pipeline.rs:1738-1790`
`kp_stage_mode_key_follow`を参照)。

結果、ひらがなキーで半角英数から戻ったつもりでも`toggle_held`は`true`のまま残り、
`freezes_conv_observation()`(`is_guard_pending() || is_toggle_active()`)も効き続け、
観測(idle-conv-check等)によるbeliefの訂正が凍結される。

### 原因2: GJIの抜け出し送信にMozc切替完了の確認が無い

`kp_send_gji_restore_exit`(`key_pipeline.rs:2461`)は`send_gji_half_width_alnum_toggle`
(`output/mod.rs:504`、`HalfWidthAlnumAction::Exit`→`VK_DBE_HIRAGANA`のSendInput)を1回送るだけで、
MS-IME側(`kp_restore_kana_from_half_width`のMS-IME分岐、`key_pipeline.rs:2840`以降)が持つ
160ms×4回のIMC write確認リトライループに相当する仕組みを持たない。この関数自身のdocコメント
(`key_pipeline.rs:2443-2447`)は「SendInputが**見送られた**場合、実GJIは半角英数のままなのに
engineがpass-throughを抜けて生ローマ字を送る——同型の実害になる」と明記しているが、**SendInputが
成功した場合でもMozc側の実際の切替には時間がかかる**ことへの対応は無い。

実機ログ(2026-11-10T08:09:59.859〜08:10:01.984)では、F2送信から`[mozc-shm]`が`"tsf_mode":
"hiragana"`へ切り替わるまで**約2秒**かかっている。この間、`mozc_compose_precheck`
(`output/probe_io.rs:92-123`)は送信直前の`effective_mode`を読むが、まだ`half-ascii`のため
`should_check_compose`の`is_composing_mode(b.effective_mode)`(`state/mozc_shm.rs:675`)が
偽になり、**確認(`MozcComposeCheckFsm`)が一切張られない**。ADR-270の「Mozc coldの最初の送信の
確認・回収」は、Mozc自身のアイドル後cold startを対象にしており、この「awase自身がGJI半角英数
トグルを抜けた直後」という既知の遷移中であることは認識していない。

## 決定

### D1. モードキーによる脱出も`toggle_held`を解除する

`kp_stage_mode_key_follow`が処理するモードキーのうち、効果が「IME ON・かな入力」側になるもの
(ひらがなキー等、`crate::state::mode_key_pass`で判定可能な役割)が`half_width_alnum.is_toggle_active()`
のときに来たら、`KeyEffectPrediction`による楽観的なbelief更新だけで済ませず、既存の脱出コミット
(`kp_send_gji_restore_exit`/`kp_restore_kana_from_half_width`相当、`ExitOwnership`を新設の
`FromModeKey`のような値で明示)を呼び、`toggle_held`を正しく`false`に戻す。既存の2経路
(2回目Shiftタップ・ADR-245復元)と同じ`exit_owns_write`の冪等性判定(INV-B「1回」)に従うことで、
二重のSendInputを防ぐ。

### D2. GJI脱出直後の最初の送信を、ADR-270の確認・回収機構に乗せる

`kp_send_gji_restore_exit`が`sent == true`を返した(SendInputが実際に送られた)ときに、
「次の1回のNICOLA送信は確認対象にする」という1回限りのマーカーを立てる(ADR-270の`cold_seq`と
同様の、段をまたいで消費される仕組みを転用。具体的な保持場所——`Output`の既存フィールドに
`Cell<bool>`を1つ追加するか、`cold_seq`自体をこの遷移でも進めるか——は実装時に決める)。

`mozc_compose_precheck`(`output/probe_io.rs:92`)は、このマーカーが立っているときは
`should_check_compose`の`is_composing_mode(b.effective_mode)`の判定を迂回し(送信直前の
`half-ascii`という読みは「移行中で当然古い」と分かっているため)、他の条件(`active`・
`effective_open`・`phase == 0`・`scope_forced_mode == 0`・ローマ字が英小文字のみ)はそのまま適用して
確認(`MozcComposeCheckFsm`)を張る。マーカーは使った時点(次の送信が来た時点)で消費して`false`に戻す
(この1回だけの迂回、恒常的な迂回にしない)。

これにより、確認が「composition未確立」と判定した場合は既存のリテラル回収(Backspace+再送、
`RawTsfLiteralRecovery`)がそのまま働く。Mozcの実際の切替完了(実測~2秒)より確認の期限
(`MOZC_COMPOSE_CHECK_MS`=300ms)が短いため、初回の確認は「Literal」寄りに倒れて1回分の
回収(Backspace+再送)を消費する可能性が高いが、連続失敗の打ち切り(2回目は消すだけで再送しない、
BUG-227関連)を含む既存の挙動の範囲で収束する。**新しい固定の待ち時間は作らない**
(`tuning-constants.md`の実測義務。約2秒という値自体も1回の実機観測に基づく参考値で、
確認の期限そのものを2秒へ延ばすことは提案しない——その場合の副作用〈本当にcompositionが
できない場合に2秒待つ〉の方が大きいと判断。既存の300ms期限+既存の回収ループに委ねる)。

### D3. MS-IME側は対象外

MS-IME分岐(`kp_restore_kana_from_half_width`のMicrosoftIme側)は既に160ms×4回のIMC write
確認リトライループを持つため、D2の対象に含めない。D1(モードキーでの`toggle_held`解除)は
IME種別に関わらず適用する(bookkeepingの問題はGJI/MS-IME共通)。

## 限界・未確認

- D2の「1回限りのマーカー」の正確な保持場所・命名・既存の`cold_seq`機構との関係は未確定。実装時に
  `tsf/warmup/mozc_compose_check_fsm.rs`・`output/probe_io.rs`の既存の段(stage)管理を読み直して
  決める。
- 実機で観測した「約2秒」は1回の観測であり、分布(最小・最大・典型)は未測定。D2は固定待ち時間を
  作らない設計のため直接の影響は小さいはずだが、確認の期限(300ms)に対して実際の切替がどれだけ
  バラつくか(2秒より短い場合・長い場合)によって、回収が何回繰り返されるか(=目に見えるBackspace
  フリッカーの頻度)が変わる。実機での再現率・繰り返し回数は未確認。
- D1の「モードキーの効果がIME ON・かな入力側になるもの」の判定(`mode_key_pass`のどの関数/値を
  使うか)は未確定。ひらがなキー以外のモードキー(無変換・変換等)がこの経路を通る場合の扱いも
  実装時に洗い出す。
- `kp_stage_mode_key_follow`は`shadow_action`を持たないキー専用の経路なので、D1の変更が
  「`shadow_action`を持つキー用の従来の経路」(ADR-191の表現)に意図せず影響しないことを確認する
  必要がある。
- MS-IME側で同様の「モードキーでの脱出がtoggle_heldを解除しない」問題が起きるか(MS-IMEは既に
  IMC writeリトライを持つため実害が小さい可能性があるが、`toggle_held`自体のbookkeepingの欠落は
  IME種別に関わらず共通のはず)は未検証。

## 却下した案

- **確認の期限(`MOZC_COMPOSE_CHECK_MS`)を実測の約2秒に合わせて延ばす**: 対象をGJI脱出直後の
  1回に限定できないため(他の全てのMozc初回送信の確認にも適用される既存の共有定数)、本当に
  compositionができないケース(真のリテラル)での利用者の待ち時間が2秒に延びてしまう。D2のように
  「この1回だけ確認を張る条件を緩める」方が、既存の期限・既存の回収ロジックを変えずに済む。
- **半角英数トグルを抜けるまでNICOLAの送信自体を保留する(holdキュー)**: ADR-245の
  `resume_hold`/`defer`機構と同様の「保持→解放」を新設する案。実装コストが高く、D2(既存の
  確認・回収機構への相乗り)で同じ結果(最終的に正しい文字が残る)が得られるため見送る。
