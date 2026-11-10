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
  BUG-192型の回帰)。実装したD1′は、ADR-244 D3(`should_abandon_on_observed_follow`、既存はMS-IME本体
  専用)をmozc-readable利用時のGJI/Mozcへ拡張し、mozc-shmの`effective_mode`観測がかなへの追随を示したら
  `toggle_held`を手放す、観測専用(IMEへは何も書かない)の経路にした。D2(GJI抜け出し直後のMozc確認・回収)
  とD3(MS-IME側の対象確認)は未着手のまま残る。
status: |-
  D1′(observation-basedのtoggle_held解除)を`docs/bug230-adr290-half-width-f2-exit-race`ブランチに
  実装・コミット済み(2026-11-10、developへは未マージ)。Opusレビュー予定。実機確認は未着手。
  D2(GJI抜け出しのMozc確認・回収)・D3は未着手。
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

### D1′. mozc-shmの`effective_mode`観測で`toggle_held`を手放す(実装済み、D1を置き換え)

当初のD1(下記「却下した案」参照)はモードキー受信時にawase側から脱出コミットを能動的に送る案だったが、
Opusレビューとユーザー指摘により却下した。実装したD1′は、IME自身の標準動作(ひらがなキーで実際に
かなへ戻る)には一切介入せず、その結果を**観測するだけ**で`toggle_held`のbookkeepingを追随させる。

- `crates/awase-hook-core/src/state/half_width_alnum.rs::should_abandon_on_observed_follow`
  (ADR-244 D3)に`mozc_shm_sourced: bool`引数を追加。既存のMS-IME本体限定の条件
  (`kind == ImeKindId::MsIme`)を`kind == ImeKindId::MsIme || mozc_shm_sourced`に広げ、
  mozc-readable経由の観測(`mozc_shm_sourced = true`)なら`ImeKindId::Gji`でも手放し判定に
  使えるようにした。判定基準自体(`eisu == Some(false) && belief_left_eisu`)はMS-IMEと同じ。
- `crates/awase-windows/src/state/platform_state.rs::ImeStateHub::report_mozc_shm`に新しい
  `observe_mozc_shm_mode_for_half_width_alnum`ヘルパーを追加。mozc-shmの`effective_mode`
  (Hiragana/Katakana系→かな、HalfAscii/FullAscii→英数、Direct/None→判定しない)から
  `InputModeObserved{source: MozcShm, confidence: High}`をbeliefへdispatchし(`ObservationSource::MozcShm`
  をinput_mode軸に使う初めての経路)、直後の`input_mode()`で実際にObservedEisuから離れたかを確認した
  上で`should_abandon_on_observed_follow`を呼ぶ。
- `ImeStateHub`(belief、`report_mozc_shm`が住む)と`GateStore`(`half_width_alnum`を持つ)は
  `PlatformState`の兄弟フィールドで互いを参照できない(`GateStore`の既存docが明記、ADR-268以来の
  構造)。そのため`report_mozc_shm`はトグルを実際には手放さず、戻り値を`(ShmUse, bool)`にして
  「手放すべきか」の判定結果だけを返す。この`bool`を`mozc_shm_on_tick`/`mozc_shm_on_timer`/
  `mozc_shm_read_once`(`runtime/mozc_shm_shadow.rs`、`gate`に一切触れない設計を保つ)経由で
  gate側へアクセスできる呼び出し元(`ir_execute`・`TIMER_MOZC_SHM`ハンドラ)まで持ち出し、
  `ir_abandon_half_width_alnum_from_mozc_shm_if`が実際に`abandon_on_observed_follow()`を呼ぶ。
- 観測(`InputModeObserved{MozcShm}`のdispatch)自体は、呼び出し元が渡す
  `half_width_alnum_toggle_active: bool`が真のときだけ行う(`&&`の短絡評価)。トグル非アクティブな
  通常時に`ObservationSource::MozcShm`をinput_mode軸へ常時割り込ませると、既存の回帰テスト
  (`report_mozc_shm_repeated_reports_are_journaled_once`)が想定するjournal件数を破る上、通常時の
  beliefへの余計な干渉にもなるため。

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

- **D1′はmozc-readableを使っている利用者だけに効く**。共有メモリが無い窓(mozc-readableを使わない
  GJI利用者、素のMozc)では`report_mozc_shm`自体が`NotFound`のまま観測を一切行わない
  (`MozcShmShadow::absent_for`、ADR-268 D1)ため、`toggle_held`の解除は従来通り2回目Shiftタップ・
  右Shift緊急解除・ADR-245窓復帰の3経路に限られ、BUG-230の症状(ひらがなキーで戻ったつもりでも
  トグルが残る)自体は残る。ユーザー自身の指摘(「GJI/MSIMEでは低コストな観測手段が無いのは
  仕方ない」)を踏まえ、この対象範囲の限定は意図的な設計判断であり、「素のGJI利用者への対応」は
  本ADRの範囲外(将来、字形〈ADR-261/273〉等の別の観測手段が育てば再検討)。
- 観測からbeliefが実際に更新され`should_abandon_on_observed_follow`が真を返しても、`toggle_held`を
  手放すこと自体はADR-244 D3の既存の仕組み(`abandon_on_observed_follow()`、OS書き込み無し)に
  委ねている。これがMS-IME側で想定していない副作用(例: 手放した直後に別の観測が逆方向に動く)を
  GJI/Mozc側でも起こさないかは、実機でのみ確認できる(未確認)。
- D2(GJI抜け出し直後の送信をADR-270の確認・回収機構に乗せる)とD3(MS-IME側の対象確認)は未着手。
  原因2(GJIの抜け出し送信自体にMozc切替完了の確認が無い)はD1′では解決していない——D1′が直すのは
  「`toggle_held`のbookkeeping」(原因1)のみで、「抜け出し送信後、Mozcの実際の切替完了まで約2秒
  romajiがリテラルのまま出力される」症状そのものは別問題として残る。
- D1′は`mozc_shm_sourced`を`true`固定で`observe_mozc_shm_mode_for_half_width_alnum`から呼んでいる
  (呼び出し元が常にmozc-shm経由であるため)。将来、別の低コストな観測源(字形等)を同じ関数に
  繋ぐ場合、`mozc_shm_sourced`という名前のまま使うのか、より一般的なフラグ名に改めるかは
  未検討。

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
  awase自身が追随していないこと」だけであり、D1′(観測ベースの追随)に置き換えた。
- **確認の期限(`MOZC_COMPOSE_CHECK_MS`)を実測の約2秒に合わせて延ばす**: D2(未着手)の検討時点での
  却下理由。対象をGJI脱出直後の1回に限定できないため(他の全てのMozc初回送信の確認にも適用される
  既存の共有定数)、本当にcompositionができないケース(真のリテラル)での利用者の待ち時間が2秒に
  延びてしまう。D2を実装する際は「この1回だけ確認を張る条件を緩める」方針を維持する。
- **半角英数トグルを抜けるまでNICOLAの送信自体を保留する(holdキュー)**: ADR-245の
  `resume_hold`/`defer`機構と同様の「保持→解放」を新設する案。実装コストが高く、D2(既存の
  確認・回収機構への相乗り、未着手)で同じ結果(最終的に正しい文字が残る)が得られる見込みのため
  見送る。
