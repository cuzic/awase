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
  無い(MS-IMEのような640ms IMC writeリトライループが無い)。実機ではF2送信からMozc側の実際の切替完了
  まで約2秒かかり、この間もawaseのbeliefは即座に「成功」としてromajiを送り続け、まだ半角英数のMozcが
  それをリテラルな英字のまま出力する(BUG-230、2026-11-10実機)。
  当初案(D1: モードキー受信時に`toggle_held`を即座に解除コミットする)はOpusレビューで却下——ユーザーが
  「IME ONの半角英数入力でひらがなキーを押せば、そもそも規定動作としてかなへ戻るはず」と指摘した通り、
  モード切替自体はIME側が標準動作として行う。真の欠落は「awase自身のbookkeeping(`toggle_held`)がその
  標準動作の結果に追随していないこと」だけであり、awase側からの能動的なキー送信は不要・有害(二重送信・
  BUG-192型の回帰)。第1版の実装(D1′、observation-based)もOpus敵対的レビューround1で2件の欠陥が
  見つかり(F1致命的: mozcの`Hiragana`をawaseの`ObservedKana`〈JISかな直接入力〉として書くとEngineが
  `Inactive(NotRomajiInput)`のまま戻らない。F2高: エントリ以前の古い共有メモリの読みで入った直後に
  手放してしまう)、belief を一切書かないedge-triggeredの純粋判定(D1″)へ置き換えた。D1″もround2で
  F2が「半分しか直っていない」(R2-1)と指摘された——`update_tick_ms`は「共有メモリに何か書いた最後の
  時刻」であって「モードが変わった最後の時刻」ではないため、エントリ直後のキー入力自身が
  `update_tick_ms`だけ進めてしまう経路があった。2段ラッチ(エントリ後に英数の読みを実際に観測してから
  のかなの読みだけを追随と認める)で修正した版が現在の実装。D2(GJI抜け出し直後のMozc確認・回収)と
  D3(MS-IME側の対象確認)は未着手のまま残る。
status: |-
  D1″(edge-triggeredの2段ラッチによるtoggle_held解除、Opusレビューround1〈F1/F2〉・round2〈R2-1〉で
  指摘された欠陥を修正した版)はround3レビューで「コードはマージ可能」と判定され、developへマージ済み
  (2026-11-10、PR #933、commit `e2f438871`)。dragonflyg4へ投入済み。D1′はOpus敵対的レビューround1で
  指摘され却下済み(下記却下した案参照)。マージ後の実機(2026-11-11)で、D1″の観測ベースの手放し自体が
  最大約5.3秒の新しい遅延([BUG-231](../known-bugs/BUG-231.md))を持つことが分かり、
  [ADR-291](291-mozc-shm-push-notify-and-engine-recheck.md)で対処を提案中。D2(GJI抜け出しのMozc確認・
  回収)・D3は未着手。
related_adr:
  - "ADR-270"
  - "ADR-289"
  - "ADR-245"
  - "ADR-244"
  - "ADR-107"
  - "ADR-291"
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
"hiragana"`へ切り替わるまで**約2秒**かかっている(**未確認**: ログが示すのは共有メモリの書き込みが
2秒後に反映されたことのみで、Mozc自身の切替が遅かったのか、切替は早かったが共有メモリへの反映が
次の打鍵まで遅れただけなのかは分からない。D2の前提・下記F2のedge-triggerの窓の両方に関わる)。この間、
`mozc_compose_precheck`(`output/probe_io.rs:92-123`)は送信直前の`effective_mode`を読むが、まだ
`half-ascii`のため`should_check_compose`の`is_composing_mode(b.effective_mode)`
(`state/mozc_shm.rs:675`)が偽になり、**確認(`MozcComposeCheckFsm`)が一切張られない**。ADR-270の
「Mozc coldの最初の送信の確認・回収」は、Mozc自身のアイドル後cold startを対象にしており、この
「awase自身がGJI半角英数トグルを抜けた直後」という既知の遷移中であることは認識していない。

## 決定

### D1″. mozc-shmの`effective_mode`観測(edge-triggered・2段ラッチ)で`toggle_held`を手放す(実装済み、D1・D1′を置き換え)

当初のD1(モードキー受信時にawase側から脱出コミットを能動的に送る案)はOpusレビューとユーザー指摘で
却下した(下記「却下した案」参照)。第1版の実装D1′(`InputModeObserved`をbeliefへ書く案)もOpus敵対的
レビューround1で2件の欠陥(F1致命的・F2高)が見つかり却下した(同)。D1′を置き換えた最初のD1″
(edge-triggeredのみ、ラッチ無し)もround2でF2が「半分しか直っていない」(R2-1)と指摘された——
`update_tick_ms`は「共有メモリに何か書いた最後の時刻」であって「モードが変わった最後の時刻」ではない
(mozc-readableの書き手`tip_state_publisher.cc`は、モード以外のフィールド`key_seq`等の変化でも
`update_tick_ms`を進める、`readable/docs/shared-memory.md` §3-1)。そのため「エントリより後」だけでは、
エントリ直後の`VK_DBE_ALPHANUMERIC`自身がまだ`Hiragana`のまま`key_seq`だけ進めて書いた読みを
「かなへの追随」と誤認しうる。現在の実装は2段ラッチでこれを修正した版。IME自身の標準動作(ひらがな
キーで実際にかなへ戻る)には一切介入せず、**belief を一切書かない純粋な判定**で`toggle_held`の
bookkeepingだけを追随させる。

- `crates/awase-ime-core/src/state/mozc_shm.rs::eisu_of_effective_mode`: `effective_mode`
  (Hiragana/FullKatakana/HalfKatakana→かな、HalfAscii/FullAscii→英数、Direct/None→判定不能)
  から英数/かなを判定する純粋関数(`classify_*`系、`is_composing_mode`と同じ「かな系」の定義を再利用)。
  belief には一切触れない。
- `crates/awase-hook-core/src/state/half_width_alnum.rs::plan_mozc_shm_follow`: トグルのエントリtick
  (`entered_at_tick_ms`)・2段ラッチの状態(`eisu_seen_since_entry`)・判定した`eisu`・読みの
  `update_tick_ms`の4つから、`MozcShmFollowPlan::{None, MarkEisuSeen, Abandon}`を決める純粋関数。
  - `update_tick_ms`がエントリtick以下(≦)なら常に`None`(エントリ**以前**の古い値を、入った直後に
    「追随した」と誤認しない、round1 F2)。
  - エントリより後の読みが英数側(`eisu == Some(true)`)で、まだラッチが立っていなければ
    `MarkEisuSeen`(エントリ後にMozcが実際に英数のままであることを一度確認した、という事実だけを記録)。
  - ラッチが立った**後**に来た、エントリより後のかな側(`eisu == Some(false)`)の読みだけが`Abandon`
    (round2 R2-1の修正: エントリ直後のkey_seq書き込みだけでは`MarkEisuSeen`すら立たない——
    `eisu == Some(true)`を要求するため——ので、そのタイミングのかな読みの誤認自体は発生しない。
    ラッチが一度立った後のかな読みは、共有メモリが最新値しか持たないことから、エントリ後に実際に
    Mozcがかなへ切り替わった証拠になる)。
- `HalfWidthAlnumState`に`entered_at_tick_ms: Option<u64>`と`eisu_seen_since_entry: bool`(2段ラッチ)を
  追加。`commit_enter_imc`/`commit_enter_gji`自体の引数は変えず(既存の呼び出し元・テストのシグネチャを
  保つ)、直後に呼び出し元(`key_pipeline.rs`の2箇所)が新設の`mark_entered_tick(tick_ms)`を呼んでセット
  する(ラッチも同時に未観測へ戻す)。クリアは`entered_in`と対で`begin_restore_kana`・
  `abandon_on_observed_follow`で行う。ADR-245の`rebuild_toggle_from_entry`(戻り待ちからの立て直し)と
  `rearm_after_failed_gji_exit`(GJI exit失敗時の巻き戻し)は、呼び出し元が渡す現在の tick を新しい
  エントリ点として`mark_entered_tick`を呼ぶ(round2 R2-3の修正。以前は`entered_at_tick_ms`を`None`の
  ままにしていたため、これらの経路で立て直した/巻き戻したトグルはmozc-shm観測ベースの手放しの対象に
  永久にならなかった)。
- `runtime/mozc_shm_shadow.rs::mozc_shm_on_tick`/`mozc_shm_on_timer`/`mozc_shm_read_once`の
  戻り値を`Option<(CompositionMode, u64)>`(受け入れた読みの`effective_mode`・`update_tick_ms`、
  belief を経由しない生の値)に変更。`report_mozc_shm`自体はADR-268時点の形(`ShmUse`のみを返す、
  半角英数トグル関連の引数・戻り値は一切持たない)のまま変えていない——`ImeStateHub`
  (belief、`report_mozc_shm`が住む)は`GateStore`(`half_width_alnum`を持つ)を参照できない
  (`GateStore`の既存docが明記、ADR-268以来の構造)という制約は、D1′の設計時点では「`report_mozc_shm`
  の戻り値をboolにして判定結果だけ持ち出す」形で対処していたが、D1″では**そもそも`report_mozc_shm`に
  半角英数トグルの関心を持ち込まない**(観測した生の値をそのまま呼び出し元まで持ち出すだけで十分、と
  設計を単純化した)。
- `gate.half_width_alnum`へアクセスできる呼び出し元(`ir_execute`・`TIMER_MOZC_SHM`ハンドラ)が、
  `mode_read`(受け入れた読み)・`entered_at_tick_ms`・`eisu_seen_since_entry`(いずれもgateから読む)を
  `ir_abandon_half_width_alnum_from_mozc_shm_if`へ渡し、`eisu_of_effective_mode` → `plan_mozc_shm_follow`
  の判定結果に従う: `MarkEisuSeen`なら`mark_eisu_seen_since_entry()`を呼ぶだけ、`Abandon`なら実際に
  `abandon_on_observed_follow()`を呼んで手放す。
- 手放した後、BUG-222(`kp_exit_lang_detect_toggle_on_hiragana`)と同じ後始末
  (`confirm_gate_deadline_override_ms`のクリア・`bump_shift_conv_guard_gen`)に加えて、
  `eisu_recovery::eisu_reset_on_turn_on_while_open(true, input_mode())` → `InputModeApplied
  {strategy: UserTurnOnEisuReset}`でbeliefをromaji-capableへ戻す(round1 F1の修正。D1′は`ObservedKana`を
  belief へ直接書いていたため、JISかな直接入力と誤認されEngineが`Inactive(NotRomajiInput)`のまま
  戻らなかった)。

旧D1が前提にしていた「モードキーの効果がIME ON・かな入力側になるものを`mode_key_pass`で判定する」
手順は不要になった——mozc-shmの`effective_mode`という実際の切替結果を直接読むため、どのキーが
モードキーとして機能したかをawase側で先読みする必要が無い。

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
`RawTsfLiteralRecovery`)がそのまま働く。Mozcの実際の切替完了(実測~2秒、ただし上記「原因2」の
注記のとおり未確認)より確認の期限(`MOZC_COMPOSE_CHECK_MS`=300ms)が短いため、初回の確認は
「Literal」寄りに倒れて1回分の回収(Backspace+再送)を消費する可能性が高いが、連続失敗の打ち切り
(2回目は消すだけで再送しない、BUG-227関連)を含む既存の挙動の範囲で収束する。**新しい固定の待ち
時間は作らない**(`tuning-constants.md`の実測義務。約2秒という値自体も1回の実機観測に基づく参考値で、
確認の期限そのものを2秒へ延ばすことは提案しない——その場合の副作用〈本当にcompositionが
できない場合に2秒待つ〉の方が大きいと判断。既存の300ms期限+既存の回収ループに委ねる)。

### D3. MS-IME側は対象外

MS-IME分岐(`kp_restore_kana_from_half_width`のMicrosoftIme側)は既に160ms×4回のIMC write
確認リトライループを持つため、D2の対象に含めない。`toggle_held`のbookkeeping不備(原因1)自体は
IME種別に関わらず共通の問題だが、MS-IME本体は既にADR-244 D3(`should_abandon_on_observed_follow`、
`ir_follow_direct_mode_key_read`からのIMM/TSF直接観測)でモードキー直後の観測ベースの手放しを
持っている(D1″はこの既存経路に一切変更を加えていない)。D1″が新規に対象にしたのは、ADR-244 D3の
対象外だったmozc-readable経由のGJI/Mozcだけ。

## 限界・未確認

- **D1″はmozc-readableを使っている利用者だけに効く**。共有メモリが無い窓(mozc-readableを使わない
  GJI利用者、素のMozc)では`report_mozc_shm`自体が`NotFound`のまま観測を一切行わない
  (`MozcShmShadow::absent_for`、ADR-268 D1)ため、`toggle_held`の解除は従来通り2回目Shiftタップ・
  右Shift緊急解除・ADR-245窓復帰の3経路に限られ、BUG-230の症状(ひらがなキーで戻ったつもりでも
  トグルが残る)自体は残る。ユーザー自身の指摘(「GJI/MSIMEでは低コストな観測手段が無いのは
  仕方ない」)を踏まえ、この対象範囲の限定は意図的な設計判断であり、「素のGJI利用者への対応」は
  本ADRの範囲外(将来、字形〈ADR-261/273〉等の別の観測手段が育てば再検討)。
- `plan_mozc_shm_follow`が`Abandon`を返しても、`toggle_held`を手放すこと自体はADR-244 D3の既存の
  仕組み(`abandon_on_observed_follow()`、OS書き込み無し)に委ねている。これがMS-IME側で想定していない
  副作用(例: 手放した直後に別の観測が逆方向に動く)をGJI/Mozc側でも起こさないかは、実機でのみ確認
  できる(未確認)。
- D2(GJI抜け出し直後の送信をADR-270の確認・回収機構に乗せる)とD3は未着手。原因2(GJIの抜け出し
  送信自体にMozc切替完了の確認が無い)はD1″では解決していない——D1″が直すのは「`toggle_held`の
  bookkeeping」(原因1)のみで、「抜け出し送信後、Mozcの実際の切替完了まで約2秒romajiがリテラルの
  まま出力される」症状そのものは別問題として残る。
- 「約2秒」というMozc側の切替所要時間は未確認(上記「原因2」注記参照)。D2の実装時にこの前提を
  再検証すること。
- **(round2 R2-2、round3で訂正)** 手放した後のbeliefの訂正に`InputModeApplyStrategy::
  UserTurnOnEisuReset`を使っている(BUG-222の既存の呼び出し元`kp_exit_lang_detect_toggle_on_hiragana`
  と同じラベル付け)。当初「strategyのラベルを変える」対応を想定したが、round3レビューでその対応は
  **効果が無い**と訂正された: 通常のBUG-230経路ではF2の`KeyEffectPredicted`が既にbeliefをromaji
  (`AssumedRomaji`)にしているため、`eisu_reset_on_turn_on_while_open`は`None`を返し
  `apply_input_mode_correction`自体が呼ばれない(ラベルを変えても何も起きない)。マージ時に本当に
  決めるべきことは、**トグルを手放した際にADR-289のクリア(`track.conv`・`key_track.conv`・
  `prev_conversion_mode`)を、`InputModeApplyStrategy`のラベルに関わらず無条件で呼ぶべきかどうか**
  (手放し自体が「実convを確定させる」行為である点はBUG-222・UserHalfWidthAlnumToggle・PanicResetと
  同じ性質を持つため)。この問題はBUG-222・ADR-244 D3の既存の呼び出し元にも**既に存在する**
  (本ADRが新規に持ち込んだものではない)。
- **(round2 R2-3の残課題)** `rearm_after_failed_gji_exit`/`rebuild_toggle_from_entry`は現在の tick を
  新しいエントリ点として扱う(上記D1″参照)。これは「巻き戻し/立て直しの時点でMozcが実際にどちらの
  モードだったか」を見ていない、保守的な近似(エントリ以降に英数の読みを1回観測してからのかな読みしか
  手放しの根拠にしない2段ラッチが、この近似の誤りを吸収する設計)。実機での頻度・挙動は未確認。
- **(round2 R2-4、round3で訂正)** `entered_at_tick_ms`・`update_tick_ms`は双方`GetTickCount64`
  (分解度 約15.6ms)。エントリと同じ分解度単位内に書かれた読みは`update_tick_ms <= entered_at`で
  「エントリ以前」と判定されうる(安全側の誤り)。当初「2段ラッチにより次の読みで追随できる」と書いたが
  楽観的すぎると訂正された: もしユーザーがエントリ直後、`HalfAscii`の読みが一度も観測される前に
  すぐF2を押してしまうと、ラッチが一度も立たないため**その回のトグルは手放されず**、旧来の挙動
  (ひらがなキーではtoggle_heldが解除されない)のまま残る。これはフェイルセーフ(分からないときは
  手放さない)として意図どおりであり、コード変更は不要——「次の読みで追随できる」という楽観的な表現を
  修正しただけ。
- **(round3 R3-3、低・未確認)** ラッチ(`eisu_seen_since_entry`)はどのスレッドの読みで立ったかを記録
  しない。同一プロセス内でフォーカスが別スレッドへ移った場合、その別スレッドの読みが(本来無関係な)
  ラッチ/手放し判定に使われる可能性がある。実機での発生可否・頻度は未確認。

## 却下した案

- **D1(旧、モードキー受信時にawase側から脱出コミットを能動的に送る)**: `kp_stage_mode_key_follow`が
  処理するモードキーのうち、効果が「IME ON・かな入力」側になるもの(ひらがなキー等)が
  `half_width_alnum.is_toggle_active()`のときに来たら、既存の脱出コミット
  (`kp_send_gji_restore_exit`/`kp_restore_kana_from_half_width`相当)を能動的に呼ぶ案。
  Opus敵対的レビューで、(a)ひらがなキー自体がIME側の標準動作として既にモード切替を行うため、
  awase側からの追加送信は`VK_DBE_HIRAGANA`の二重送信になりうる、(b)この二重送信パターンは
  BUG-192(2026-10-04、`024ca336`→`6b1e91b8`)で一度実装され、GJIのMS-IMEプリセットで`ro`という
  リテラル化を引き起こしrevertされた経緯と同型である、と指摘され却下。ユーザー自身も
  「IME ONの半角英数入力のときのかなキーって、そもそも規定の動作としてひらがなになりますよね」と
  同じ結論に達した。真に欠けていたのは「awaseの能動的な介入」ではなく「IME側の標準動作の結果を
  awase自身が追随していないこと」だけであり、D1′(その後D1″)に置き換えた。
- **D1′(第1版、`InputModeObserved{source: MozcShm}`をbeliefへdispatchし、
  `should_abandon_on_observed_follow`に`mozc_shm_sourced: bool`を追加する案)**: Opus敵対的レビューで
  2件の欠陥が見つかり却下した。
  - **F1(致命的)**: Mozcの`CompositionMode::Hiragana`を、awaseの`InputModeState::ObservedKana`
    (「JISかな直接入力と確認できた」という意味、ADR-091決定2によりサポート対象外)として書いていた。
    `Hiragana`はローマ字テーブルでの入力(NICOLAの前提そのもの)とJISかな入力を区別できないため、
    手放しに成功するたびにbeliefが`ObservedKana`になり、`engine.rs::compute_state`が
    `Inactive(NotRomajiInput)`を返し続けてEngineが戻らなかった(BUG-230の症状を直すはずの変更が、
    むしろ悪化させる方向だった)。
  - **F2(高)**: `report_mozc_shm`内の観測が`decide_use`の`Held`/`Split`判定を素通りしていたため、
    トグルに入った直後、共有メモリにまだ残っているエントリ**前**の古い`hiragana`値を「かなへの
    追随」と誤認し、入った直後にトグルを即座に手放してしまう経路があった(ADR-244 D3が本来防ぐ
    はずの「belief は英数・トグルは無し」を作る反転)。
  - 両方とも、belief を書く・`decide_use`の判定に相乗りする、という設計そのものに起因していたため、
    D1″では「belief を一切書かない・`decide_use`を経由しない独立したedge-triggered判定」に全面的に
    置き換えた。
- **D1″初版(round2レビュー前、ラッチ無しのedge-triggered判定のみ)**: `update_tick_ms`がエントリtick
  より後であることだけを条件にしていた。Opus敵対的レビューround2(R2-1)で、`update_tick_ms`が
  「共有メモリに何か書いた最後の時刻」であって「モードが変わった最後の時刻」ではないと指摘され
  (mozc-readableの書き手はモード以外のフィールドの変化でも`update_tick_ms`を進める、
  `shared-memory.md` §3-1)、エントリ直後の`VK_DBE_ALPHANUMERIC`自身の`key_seq`書き込みが
  「かなへの追随」と誤認されうる反例(`entered=1000`、`(かな, 1001)`、英数の読みを一度も観測して
  いない)が示され、現在の2段ラッチ版に置き換えた。
- **確認の期限(`MOZC_COMPOSE_CHECK_MS`)を実測の約2秒に合わせて延ばす**: D2(未着手)の検討時点での
  却下理由。対象をGJI脱出直後の1回に限定できないため(他の全てのMozc初回送信の確認にも適用される
  既存の共有定数)、本当にcompositionができないケース(真のリテラル)での利用者の待ち時間が2秒に
  延びてしまう。D2を実装する際は「この1回だけ確認を張る条件を緩める」方針を維持する。
- **半角英数トグルを抜けるまでNICOLAの送信自体を保留する(holdキュー)**: ADR-245の
  `resume_hold`/`defer`機構と同様の「保持→解放」を新設する案。実装コストが高く、D2(既存の
  確認・回収機構への相乗り、未着手)で同じ結果(最終的に正しい文字が残る)が得られる見込みのため
  見送る。
