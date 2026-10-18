# ADR-245 追補 2 Opus レビュー round1

対象: `/home/cuzic/rust-nicola-wt-adr245`、ブランチ `feat/adr245-pr2-pr3-rebased`、コミット `c85e668b`(`git log -1 --oneline` で確認)、
`docs/adr/245-half-width-alnum-restore-on-return.md` の 102〜148 行(追補 2)。
読んだコード: `runtime/key_pipeline.rs`(60-260、2443-2867)、`app/mod.rs::handle_hook_key_event`(680-734)、
`runtime/message_handlers.rs`(1-256、525-560、1765-1887)、`input_defer.rs`、`hook.rs`(1600-1931)、
`runtime/lang_detect_shadow.rs`(230-499)、`src/lang_detect.rs::HoldController`(648-806)、`src/engine/engine.rs::compute_state`(269-283)、
`ime.rs`(670-700、1236-1290)、`state/platform_state.rs`(1144-1219、1890-1925)、`lib.rs` の TIMER_*、`.githooks/pre-push`、
`tools/e2e/ime_key_matrix/check_focusrestore.py`、コミット `25857f9e` の差分。

件数: Blocker 2 / Must 6 / Should 6 / Nit 5

---

## Blocker

### B-1 解放条件 (a) が復元に追随するという前提は、追補自身の測定では 35 件中 8 件しか裏付けられていない
- 根拠: 追補 2「事実」3 項目め(ADR 108 行):「`[effective-open-flip] → true` まで 3.4〜5.9ms(n=8。**ほかの試行は flip を 1 秒以内に出さない**=belief が既に真だった等で**未分類**)」。
  一方で同じ節の 2 項目め(107 行)は、全試行で `decision=PassThrough` になった理由を「MS-IME 本体は半角英数トグル中に `effective_open=false`」としている。
- 2 つは両立しない。`kp_run_inner` は段(`key_pipeline.rs:112`)の**後**で ctx を組み(`:132-140`)、復元は末尾で `apply_input_mode_correction(AssumedRomaji)` を**同期**で当てる(`:2860-2866`)。
  もし Resume の時点で belief の open が真だったなら、同じ打鍵で `compute_state` が Active になり(`engine.rs:269-283`)、PassThrough にはならない。
  全試行が `w` で素通しされた以上、残りの 27 件でも段の時点で open は偽だったはずで、その 27 件では **1 秒以内に open が真にならなかった**ことになる
  (`last_effective_open` は打鍵ごとの `effective_open_at` で更新され、反転すれば必ずログを出す。`platform_state.rs:1184-1195`)。KeyUp の `Engine activated ~60ms` とも合わない。
- 失敗シナリオ: 27/35 の側が本来の分布なら、解放はほとんど (b) の 100ms 期限で起き、最初の打鍵は従来どおり `w` のまま、遅れだけが 100ms 増える。
  (a) が働くとしても、それが「復元の書き込みの結果」なのか「別の経路(ポーリングなど)で偶然真になった」のかが分からない(`decided_by` が追補に無い)。
- 提案: 設計を確定する前に、run 39120344451 の awase.log で 1 試行ずつ、(i) Resume の時刻、(ii) その後に `build_ctx()` から `compute_active` が真になる最初の時刻
  (flip ログと `decided_by`、無ければ KeyUp の `[engine-input] [diag-ctx] ime_on=` から読む)、(iii) `[conv-actuate] Written` の時刻を表にする。
  解放条件の根拠に使う数字は「書き込みの時刻」ではなく (ii) にする(M-5 にも関係)。27 件を説明できなければ、保持では直らない可能性が高い。

### B-2 後続の打鍵を「同じ保持へ足す」場所が書かれておらず、`kp_run_inner` の中に置くと順序が入れ替わる経路がいくつもある
- 根拠: 追補 2 決定 2(114 行)は「保持が終わるまでの後続の打鍵も同じ保持へ」とだけ書いている。起案は `kp_stage_half_width_return`(`key_pipeline.rs:112`)の中で起きる。
  打鍵がそこに届くまでに、以下の早期 return を通る:
  - `deliver_key_event` の `[[keymap]]` 照合(`message_handlers.rs:217-221` → `consume_keymap_match` が `send_keymap_target` で**その場で SendInput**、`:299-306`)
  - `PumpContext::Nested` の即時 reinject(`:185-188`)、NonText の即時 reinject(`:198-203`)、`[[post_bypass]]`(`:235-237`)
  - `kp_run_inner` 冒頭の `try_hold_key`(TsfGate、`key_pipeline.rs:69-76`。別キュー)、ime-off-rescue の Phase A(`:83-108`)
  - 取り込み口の `FOCUS_RESYNC` の defer(`app/mod.rs:709-722`)
- 失敗シナリオ: `w`(Resume、保持)の 10ms 後に `[[keymap]]` の Ctrl+H を打つと、BS が先に OS へ出て `w` が後から再生される。TsfGate が PendingWarmup に入ると、2 打鍵目が TsfGate、1 打鍵目が ResumeHold に分かれ、解放の順序が保証されない。
- 対照: ADR-263 の保持は**取り込み口**(`handle_hook_key_event`、`app/mod.rs:703-708`)で全打鍵を取るので、上の経路をすべて通る前に順序が固定される。
  再生は `handle_wm_drain_output_queue` → `deliver_key_event(DeferredReplay)`(`message_handlers.rs:1825`)で、取り込み口を通らないので二重に保持されない。
- 提案: 「起案(どの打鍵で保持を始めるか)」は `kp_stage_half_width_return` に残し、「保持中の後続の打鍵を足す」判定は `handle_hook_key_event` の `lang_detect_on_key` の直後(OUTPUT_GATE・FOCUS_RESYNC の判定より前、パニック検出より後)に置くと明記する。
  起案した打鍵自身は、`kp_run_inner` から `Consumed` を返して保持へ入れる。architecture_guard でこの配置(取り込み口の呼び出しが OUTPUT_GATE の判定より前にあること)を固定する。

---

## Must

### M-1 解放 (c)(上限で先に解放してから新しい打鍵を通常どおり処理)は、新しい打鍵が保持した打鍵を追い越す
- 根拠: 追補 2 決定 3(c)(118 行)。`INPUT_DEFER.replay_later` は積んで `WM_DRAIN_OUTPUT_QUEUE` を post するだけ(`input_defer.rs:56-69`)で、再生は後のメッセージで起きる。新しい打鍵をその場で「通常どおり処理」すれば、こちらが先に Engine と reinject を通る。
- 失敗シナリオ: 保持が 4 件になった直後の 5 件目(KeyDown)が先に OS へ出て、保持していた `w` がその後に出る。
- 提案: 契機の打鍵も解放列の末尾に入れて、まとめて `replay_later` する(`HoldController::on_key` の `release` は「保持していた打鍵と、解放の契機になった打鍵」を同じ列で返している。`src/lang_detect.rs:655,743,764`)。
  (a)(b) をタイマー以外(打鍵が来たとき)にも判定するなら、同じ規則を適用する。

### M-2 MS-IME 本体の Resume は VK_DBE_HIRAGANA を送らず IMC だけで戻すので、`compute_active` が真になっても実際のモードがかなとは限らない。msime-lshift を「別経路」とするのも誤り
- 根拠: `kp_restore_kana_from_half_width` は `effective_open()` が偽なら scan 付き VK_DBE_HIRAGANA の注入を飛ばし、IMC への書き込みだけにする(`key_pipeline.rs:2665,2695-2700`)。
  MS-IME 本体の Resume は、`25857f9e` によって**常に** `effective_open=false` のまま来る(ADR 61 行・107 行)。同じ関数のコメントは「IMC の conv 書き込みだけでは新 MS-IME(TSF)の実モードは英数から戻らない(2026-07-07 実機)」と書いている(`:2610-2614`。ADR 決定の事実 3 項目め・23 行も同じ)。
  msime-lshift も Shift の KeyUp から同じ `kp_restore_kana_from_half_width(.., FromResume)` に入る(決定 6)。違うのは契機と `prepend_synthetic_shift_up` だけで、`effective_open` が偽なので後者は効かない。`25857f9e` の本文が記録している `ｗ`(全角)は「IME は開いたが NATIVE ではない」ことを示唆しており、復元の本体が効いていない証拠かもしれない。
- 失敗シナリオ: 解放 (a) は belief(IMC の読み)で真になり、Engine は `k`,`a` を送る。実モードが英数のままなら、`w` が `ka`/`ｋａ` に変わるだけで直らない。CI の `typed` の本文の判定(`check_focusrestore.py:137-141`)では落ちるが、原因は保持ではなく復元の本体にある。
- 提案: (i) 範囲外の節から「msime-lshift は別経路」を外し、同じ復元の本体を通ることを書く。(ii) 保持を実装する前に、basic の CI で「typed の後にもう 1 文字打ってかなが出る」ことを確かめるか、lshift の `ｗ` の原因を先に切る。
  (iii) Resume では A が前面にあることが確かなので、MS-IME 本体の Resume に限って `effective_open` のガード(BUG-15 追補7)を外すかを判断点として明記する(所有者判断が要る)。

### M-3 解放条件 (a) の `Engine::compute_active` は、エンジンの ON/OFF と言語も含むので、決定 9(エンジン OFF でも復元)の構成では毎回 100ms 待つ
- 根拠: `compute_state` は `!adapter.is_enabled()` → `UserDisabled`、`!is_japanese_ime` → `NotJapaneseIme` を、IME の状態より先に返す(`engine.rs:270-275`)。
  決定 9(65 行)では、エンジン OFF でも Resume が走る。
- 失敗シナリオ: エンジン OFF(利用者は IME へ直接ローマ字を打つ)で A に戻ると、(a) は決して真にならず、最初の打鍵が毎回 100ms 遅れて (b) で解放される。
  実は IME 側はこの 100ms の間に戻っていることが多いので、素通しでも結果は正しく、遅れだけが余計にかかる。
- 提案: 解放条件は「IME 側の条件が揃った」(belief の open が真かつ `input_mode.is_romaji_capable()`。`compute_state` が `ImeOff`/`NotRomajiInput` 以外を返す)にし、エンジン OFF でも同じ条件で解放する。
  `compute_active` と同じ式にしたいなら、エンジン OFF のときは保持しないと決め、その旨を書く。単体の表に「エンジン OFF + Resume」の行を足す。

### M-4 前面窓が変わったときに保持をまとめて捨てると、保持の前に Down が OS へ届いていたキーの Up まで捨てて、キーが押されたまま残る(Down/Up 非対称)
- 根拠: 追補 2 決定 4(119 行)は保持した打鍵をすべて捨てる。フックは全打鍵を `LRESULT(1)` で握り、OS へは engine thread の reinject だけが届く(`hook.rs:1919-1923`)。
  保持の**前**に reinject された Down(B で押したまま A に戻った文字キー、親指キー、保持より前に段を `Nothing` で抜けた修飾キー〈`key_pipeline.rs:2452`〉)の Up が保持に入ると、その Up は OS へ一度も届かない。
  フック側のラッチ(`physical_key_state`、`*_thumb_down_scan`、`hook.rs:1792-1844`)は Consumed と関係なく更新されるので安全だが、OS の `GetAsyncKeyState`・アプリから見たキー状態は Down のまま残る。
- 失敗シナリオ: 修飾キーは押されている間は行 5 で Deferred になるので、この穴に当たるのは文字キーと親指キー。
  例: B で `a` を押しながら A をクリックし、A で `w` を打つ → `w` Down を保持 → `a` Up が保持に入る → 保持中にマウスで C へ → 全部捨てる → `a` が押されたまま。
- 提案: 捨てるのは「保持の中に対応する Down がある打鍵」だけにし、Down が保持の外にある Up は再生する(Up だけの再生は、別窓に届いても文字を出さない)。
  同じ穴は ADR-263 の `HoldController::on_timer` の破棄(`src/lang_detect.rs:794-796`)にもあるので、そちらも直すか known-bugs に起票する。単体の表に「Down が保持外の Up だけが保持中に来て、窓が変わる」を足す。

### M-5 `HALF_WIDTH_RESUME_HOLD_MAX_MS = 100` と 10ms の周期は、`tuning-constants.md` の「実測 → 導出」を満たしていない
- 根拠: 追補 2 決定 6(121 行)の導出は「実測最大 6.0ms に余裕を見込む。上限は体感と釣り合わせる」で、6ms から 100ms(約 16 倍)をどう決めたかの数字が無い。測っているのは書き込みの完了時刻で、解放が実際に待つ「`compute_active` が真になる時刻」は n=8 しかない(B-1)。
  決定 5(120 行)の「10ms は Windows のタイマー分解能の下限(約 15.6ms)に当たる」は事実として誤り。10ms は `USER_TIMER_MINIMUM` で、既定の分解能(15.625ms)より短いので、実際の周期は約 15.6ms に丸められる(`SetTimer` は `timer.rs:31-44` で ms をそのまま渡している)。
  さらに WM_TIMER はキューに他のメッセージが無いときにしか作られないので、打鍵が続いて drain や `spawn_local` の起床が post されている間は遅れる。
  10ms も `_MS` 系のタイミング定数なので、`tuning.rs` の `#[measured]` の対象になる。
- 提案: (i) 100ms は B-1 の表の (ii) の最大値に、根拠のあるマージンを足して導く(例: 「(ii) の実測最大 X ms + 負荷時の IMC cross-process の測定値 Y ms」)。測れないなら、CI の値だけで決めたことと、dragonflyg4 で測り直す条件を本文に書く。
  (ii) 周期の定数も `tuning.rs` に置き、「実効 15.6ms」と書き直す。(iii) S-1 の事象駆動の解放を入れれば、周期は期限の監視だけに使えるので、10ms に縛られなくなる。

### M-6 検証計画は 1 打鍵しか打たないので、保持の追加・順序・破棄の経路を CI でまったく通らない
- 根拠: `check_focusrestore.py` の判定は、戻って最初の 1 打鍵の本文と conv だけを見る(`:112-141`)。追補 2 の検証節(130-134 行)の CI は、strict の PASS(最初の打鍵が `か`)だけを期待している。
  未確認の節(147-148 行)が挙げている順序の問題は、どの検証にも割り当てられていない。
- 失敗シナリオ: B-2・M-1・M-4 の退行は、単体テスト(純粋な状態機械)では捕まえられない(取り込み口・`deliver_key_event` の早期 return は殻の側にある)。CI でも 1 打鍵の構成は全部 PASS のまま通る。
- 提案: (i) 戻って**続けて 2〜3 文字**を短い間隔(5ms 以下、`SendInput` 1 回にまとめる)で打つ構成を足し、本文が期待のかな列になることを見る(例: `wk` → `かき`)。
  (ii) ログ断片検査で解放の理由(`release=EngineActive`/`Timeout`/`Cap`/`WindowChanged`)を出させ、strict の MS-IME 構成で `Timeout` が 0 件であることを見る(B-1 の前提が CI で崩れたらすぐ分かる)。
  (iii) エンジン OFF の構成(M-3)を observe で 1 本。(iv) architecture_guard に B-2 の配置の固定を足す。

---

## Should

### S-1 「事象駆動の解放」の却下理由は belief の経路しか検討しておらず、復元タスク自身の完了を契機にする案が抜けている
- 根拠: 決定 5(120 行)・却下表 2 行目(141 行)は、事象駆動を「`effective-open-flip` の発火点に差し込む」ことと同一視している。
  しかし書き込みは `kp_restore_kana_from_half_width` の `spawn_local`(`key_pipeline.rs:2729-2844`)の中で起き、`Written` が分かる場所(`:2785-2786`)は `with_app` で Runtime に触れられる。ここで「保持を解放してよいか見て」と 1 回呼ぶ(または `post_to_main_thread`)のは belief でも `ImeEvent` でもない。
- 提案: 解放の判定を「(1) 復元タスクの `Written`/`Failed`/`Aborted` の直後、(2) 保持中に新しい打鍵が取り込み口に来たとき、(3) 期限のタイマー」の 3 か所にし、判定の式は 1 つ(M-3 の式)にする。
  周期ポーリングが要らなくなり、最初の打鍵の遅れが「書き込み + belief の追随」まで縮む(15.6ms の下限が消える)。belief の追随が書き込みより遅い場合だけ (3) まで待つ。

### S-2 Engine の出力側には既に確認ゲートがあり、(a) の偽陽性の一部はそこで止まる。その関係を書いておく
- 根拠: 復元は `ime_mode_fsm.unconfirm("shift-conv-guard release")`(`key_pipeline.rs:2594-2598`)と `confirm_gate_deadline_override_ms`(`:2546-2549`、リトライ中は延長 `:2752-2758`)を張る。次のかな送信は msime-ready ゲートが IMC の NATIVE を確認するまで待つ(`output/vk_send.rs:549`、`output/probe_io.rs:190,318`)。
- 意味: (a) が書き込みより先に真になっても、IMC が NATIVE になるまで romaji は出ない。逆に M-2 の「IMC は NATIVE だが TSF の実モードは英数」はこのゲートを素通りする。
- 提案: 追補 2 の決定 3 か「既存の保持との関係」に、「(a) が保証するのは Engine の判断だけで、出力のタイミングは既存の msime-ready ゲートが保証する。IMC と実モードの食い違い(M-2)はどちらも保証しない」と 1 段落足す。
  この関係があるので、解放条件は IME 側の belief だけにでき(M-3)、書き込み完了を待つ必要は必ずしも無い(S-1 の (1) は遅れを縮めるためのもの)。

### S-3 新しいファイルが、再発ファミリーの表と pre-push の正規表現のどちらにも入らない
- 根拠: `.githooks/pre-push:38` の正規表現は `state/(...|half_width_alnum)` で、`state/half_width_resume_hold.rs` には当たらない。`runtime/lang_detect_shadow.rs` もどちらにも入っていない。
  `fix-requires-evidence.md` の「半角英数トグルの復元」行と「defer/replay キューの解放条件」行も同じ。`crates/xtask-adr-evidence` が表と正規表現の一致を CI で見ている。
- 提案: 表の「半角英数トグルの復元」行と「defer/replay」行に `state/half_width_resume_hold.rs` と殻の配線ファイルを足し、正規表現に `half_width_` を足す(同じ PR で)。

### S-4 保持を即座に片付けるべき契機が、窓の変化(次のタイマー tick で検出)しか無い
- 根拠: 決定 4・5 は、窓の変化をタイマー tick の `foreground_scope()` で見るだけ。ADR-263 は設定の再読み込みで保持を解放する(`lang_detect_shadow.rs:492-496`)。
- 失敗シナリオ: 保持中に `WM_PANIC_RESET`、エンジンのトグル(ホットキー)、`WM_WTSSESSION_CHANGE`(ロック)、設定の再読み込み、`ir_notify_focus_changed` が来ても保持が残り、次の tick まで(タイマーが止まれば無期限に)打鍵が出ない。
  保持中に A を離れると、復元タスクは世代の bump で中断し(`key_pipeline.rs:2770-2776`)、エントリは取り出し済みなので、A は英数のまま戻り待ちからも消える(PR 2 からある狭い穴だが、保持の間は窓が広がる)。
- 提案: 契機ごとに「解放(再生)」か「破棄」かを表にし、殻でタイマーを kill する箇所を architecture_guard で固定する(検証節の「kill される」を契機ごとにする)。

### S-5 TsfNative の A(Chrome・WezTerm など)+ MS-IME 本体では、belief を真にする観測があるかが分からない
- 根拠: `reschedule_ime_refresh` は TsfNative でポーリングを止める(`runtime/mod.rs:1165-1171`)。CI の msime 構成は typing_stress の Win32 窓なので、TsfNative は通っていない。ADR の決定 7(63 行)は TsfNative の順序を「見込み」と書いている。
- 失敗シナリオ: TsfNative の A では (a) が真にならず、最初の打鍵が毎回 100ms 遅れて `w` のまま。
- 提案: B-1 の表に `decided_by` を入れて belief を真にした観測の出どころを特定し、Chrome の MS-IME 構成を observe で 1 本足す(検証節の「Chrome(TsfNative)・UWP を observe」に、解放の理由のログ断片を足す)。

### S-6 保持した最初の打鍵では、段の前の副作用が 2 回走る
- 根拠: 起案した打鍵は、`lang_check_apply`・`enrich_ime_relevance`・`enrich_key_role`・`enrich_thumb_key_role`・`try_hold_key`・ime-off-rescue の Phase A・`kp_stage_focus_probe`(`key_pipeline.rs:62-110`)を通ってから保持され、再生でもう一度通る。
  `enrich_key_role` のラッチは 1 スロットで、再生の Down で上書きされる(`key_effect_runtime.rs:563-586`)。保持の間に別の Down が来ると、保持がない場合と同じ順序で上書きされるので結果は同じだが、`kp_stage_focus_probe` は probe を 2 回起こしうる。
- 提案: 段の前の各処理が再実行しても同じ結果になる(冪等)ことを 1 行ずつ確かめて書くか、起案した打鍵だけは段より前の処理を済ませた印を付けて、再生で飛ばす。

---

## Nit

### N-1 書き込みの出どころの記述が違う
- 追補 2 107 行「復元の書き込みは `actuate_conv_mode` の `spawn_local`」。実際は `kp_restore_kana_from_half_width` の `spawn_local`(`key_pipeline.rs:2729`)→ `ime::set_ime_conv_for_target`(`ime.rs:1236-1278`、`[conv-actuate] Written` はここ)→ `set_ime_romaji_mode_for_hwnd`(`[imm-romaji]`、`ime.rs:670-693`)。`actuate_conv_mode`(`runtime/conv_actuation.rs:19`)は通らない。

### N-2 n の単位が合っていない
- 「6 構成 × 5 試行」は 30 試行だが、書き込みは n=35。リトライ(最大 4 回、`key_pipeline.rs:2734-2735`)か `-twice` の 2 往復で増えたと思われる。表は試行単位で書き、書き込みは「試行あたり最初の 1 回」に揃える。

### N-3 「一発ポーリングタイマー(10ms 周期)」は言葉が矛盾している
- 決定 5(120 行)。tick ごとに `set` し直す一発タイマーか、保持中は周期で鳴らし保持が空になったら kill する周期タイマーかを決めて書く。ID は `TIMER_LANG_HOLD = 111` の次の 112 で、エンジンの `TIMER_PENDING = 1`/`TIMER_SPECULATIVE = 2`(`src/engine/fsm_types.rs:11,14`)とは衝突しない。

### N-4 却下表 2 行目は理由を具体的に書く
- 「belief 更新に触る」だけでなく、awase 側の起案は `record_explicit_intent` の呼び出し元のガード(`tests/architecture_guard.rs:1291` `record_explicit_intent_call_sites_are_limited_to_real_user_actions`)で本物のユーザー操作に限られていて、Resume はその条件を満たさない、と書くと後で蒸し返されにくい。却下の結論は妥当。

### N-5 frontmatter と節の名前
- `status` は round4 までで、追補 2(とその前の行 3 の変更)が入っていない。追補 2 の本文が「追補 1 相当、決定 5 の行 3 の注記」と書くが、「追補 1」という節は無い。`status` に「追補 2 起草(2026-10-18)・Opus round1 待ち」を足し、呼び方を「決定 5 の行 3 の注記(`25857f9e`)」に揃える。
