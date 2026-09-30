# ADR-212 草案 Opus レビュー round1(2026-09-30、対象 `a08480a0`)

裏取りに使ったもの: docs360(develop 先端、#398 マージ後)のコード、ADR-191・ADR-203・BUG-157/170 の本文、`docs/experiments.md`。行番号は docs360 の HEAD。

## 総評

方針(【許可】【出力】を残し、【予防的】【補正的】を段階的に外す)と「1段=1PR、各段で実測」の進め方は妥当。棚卸しの主要な事実も、おおむねコードと合っている。
ただし次の4点は、そのまま実装に進むと誤りになる。
- **P2(ActivationSync)**: 計測方法が今の journal では取れない(B1)。「残す」とした belief 側を残すと別の副作用が出る(B2)。止めたときに戻りうる不具合(BUG-170 型)が抜けている(B3)。
- **ADR-191 との関係**: ADR-191 は warmup と EngineDecision を「既存の例外」として明文で残している。本 ADR はそれを覆すのに、改訂として書いていない(M1)。
- **B1 drift correction の分類**: 「明示意図の書き込みの再試行」と「HWND キャッシュの復元の押し付け」という、性質の違う2つが混ざっている(M5)。
- **決定6(完了条件)**: 関数名ベースの lint では固定できない(M8)。

重大度: **B**=このままでは段が成り立たない/実害 / **M**=範囲・分類・検証の穴 / **m**=記述。

---

## (1) 棚卸しの事実の裏取り

| 主張 | 確認結果 |
|---|---|
| F2(0x71)を送る経路は無い | **正しい**。`0x71`/`VK_F2` は `vk.rs:621,1630` の名前表だけ。キー入力の組み立て(`make_key_input_ex`/`make_tsf_key_input`)の呼び出し元にも F2 は無い |
| `VK_IME_ON` の送信点 6 箇所 | **数え方を直すべき**。コード上で `VK_IME_ON` を INPUT に積む場所は4つ: `state/key_sequence_policy.rs:132/138`(A1、`ime_controller.rs:250/297` の `send_ime_mode_key` 経由)、`tsf/send.rs:39`(A2/A3 共通)、`output/mod.rs:756`(A5)、`output/probe_io.rs:216`(A6/A6b 共通)。「6」は経路(ID)の数で、#398 で A4 が消えた今は A1・A2・A3・A5・A6・A6b の6経路。本文は「送信点」でなく「経路」と書くこと |
| eager warmup の呼び出し元4つ | **#398 後の今は実質3つ+デッド1つ**: `ime_refresh.rs:618`(A2)、`platform.rs:287`(A2 の薄いラッパー)、`platform.rs:1409`(A3、随伴)、`output/vk_send.rs:692`(`WarmupImeOn::off()` を渡すデッドコード)。背景の要約と合う |
| ActivationSync が実 actuation へ流れる | **正しい**。`engine.rs:393-402`(`check_active_transition` → `transition_activation(.., ActivationSync)`)が `SetOpen` を出す。`executor.rs:661-662` は origin を見ずに `dispatch_ime_set_open` に渡す。ただし「新しく発見」は言い過ぎ: ADR-191 L312 は IME に書く振る舞いとして「EngineDecision」を数え、L322 は「発生元の軸が要るので、分離の是非を調査してから」と先送りしている(M1)。棚卸しにあるとおり ADR-154 にも記述がある |
| 記号 VK フォールバックはデッドコード | **正しい**(`vk_send.rs:692`、`WarmupImeOn::off()`) |

### 漏れている経路・事実
- **m1. もう1つのデッドコード(P1 に足せる)**: `Platform::set_ime_open`(`src/platform.rs:424` のトレイト定義と、`crates/awase-windows/src/platform.rs:1182-1197` の実装)。
  本番の呼び出し元はゼロで、`tests/architecture_guard.rs:1458` が `.set_ime_open(` の件数を0件と固定している。実装の本体は `set_ime_open_cross_process_async` を投げるだけ。
  P1 で消せば、`ImmSetOpenStatus` の書き込み口が1つ減る。
- **M2. 第2の ActivationSync 相当の経路**: `key_pipeline.rs:856-914` の `kp_apply_conv_engine_sync`(idle-conv-check)は、`EngineSync::SetOpen(RomajiRecovered)`(conv の観測から Engine を ON に同期する)のとき、Engine を経由せず `handle_engine_activation_sync` を**直接**呼ぶ。
  コメントは「BUG-48 の ActivationSync 経路(… actuation は同一)を使う」と書く。棚卸しの C1 は Engine の `check_active_transition` しか見ていない。
  この経路が実際に書くのか(`ImeApplyRequested` の pending を立てるだけで終わるのか)を確認し、C1 と一緒に扱うこと。
- **M3. 焦点確定後の再試行**: `executor.rs:140-170`(`strip_ime_set_open_if_settling`)と `runtime/mod.rs:866-880`(`schedule_settle_retry`)。
  焦点の遷移中に落とした `SetOpen` を、settle 明けに refresh を起こして**出し直す**、補正的な仕組み。落とした `SetOpen` の多くは ActivationSync のはず。
  コメント(`executor.rs:151-155`)によれば、目的は「GjiFsm 等、apply 完了通知でしか同期しないサブシステムが実状態と乖離したまま固着する」ことの防止(2026-07-08「このせっけい→せっけい」)。P2 の対象として明記し、B3 と一緒に扱うこと。
- **M4. 焦点変更による desired の復元(B1 の隠れた役割)**: `platform_state.rs:1380-1395` の `apply_hwnd_cache_restore` は、焦点が戻った窓のキャッシュから `desired_open` を復元する(`HwndCacheRestored`)。
  その後、実 IME の観測がキャッシュと違えば、B1 の drift correction が閾値を過ぎてから書く。つまり B1 は「belief のずれの補正」だけでなく、**窓ごとの IME 状態を awase が覚えておいて押し付ける**機能も担っている。P6 の分類と検証に入れること(M5)。
- **m2. GjiFsm の StaleConfirm → ESC/BS の回収**: `probe_io.rs` の give-up → `output/mod.rs:1921` → `flush_raw_tsf_literal_backspaces`(`tsf/output.rs:175-181`、ESC+BS)。
  本文は【出力】(リテラル掃除)に入れているが、引き金は awase の判定(StaleConfirm 等)で、ユーザーの未確定文字列を消しうる(BUG-170・BUG-171)。IME の状態(composition)も変える。
  【出力】に残すなら「自分の出力の回収だから」という理由を決定2に1行書き、P3(同じ give-up の reinit)との境界を明確にすること。
- **確認済みで漏れていないもの**: `ImmNotifyIME` は A11 だけ(`runtime/mod.rs:2457`)。`WM_IME_CONTROL` の書き込みは `imm.rs::actuate_ime_control` の2種(SetOpenStatus・SetConversionMode)に集約され、呼び出し元は `ime.rs:86/380` だけ(棚卸しの B/D 表と一致)。
  CapsLock の呼び出し元は A9・A10 の3か所。`conv_mode_policy=force` の焦点変更時の書き込みは撤去済み(設定名は `config_load_diag.rs` にだけ残る)。TSF の compartment への書き込みは無い。

---

## (2) 分類の誤り

- **M5. B1 drift correction を一括で【補正】にするのは粗い**。`drift_correction.rs` の閾値は、`explicit_intent == Some(desired)` かつ `last_intent` があるときに **0**(即時)になる。つまり B1 には次の3つが混ざっている。
  - (a) **ユーザーの明示操作(【許可】)の書き込みが実 IME に届かなかったときの再試行**(BUG-16/20 型)。
  - (b) belief/desired が古いことによる補正(BUG-157 型の誤作動の源)。
  - (c) M4 のキャッシュの押し付け。

  (a) は【許可】の書き込みの信頼性の一部で、外すと「IME キーが効かないことがある」(ADR-189 が直した症状)が戻りうる。
  P6 は B1 を (a) と (b)(c) に分け、(b)(c) を先に外し、(a) は【許可】として残すか、「意図の有効期限内の1回だけの再試行」に縮めるかを決めるべき。
- **m3. A7/A8(半角英数トグル)の「焦点変更時・IME ON 時の強制 Exit」は、ユーザー操作が引き金ではない**(`ime_refresh.rs:334`、`key_pipeline.rs:1099/1147/1411`)。
  所有者決定の「トグルを残す」は左 Shift の押下による Enter/Exit を指すはず。awase が自分で作った状態の後始末として強制 Exit も残すなら、決定2にそう書くこと(妥当な判断だと思う)。
- **m4. P5 の A6(Unicode long-cold の reinit、Actuation 起点)の分類は、P2 の後に変わる**。Actuation 起点=A1/C1。P2 で C1 が消えると、残る起点は ExplicitUserAction(ユーザーの IME キー)だけになる。
  そうなると A6 は「ユーザーの書き込みに付随する warmup」で、A3 と同じ性質になる。P5 の説明を、P2 の後の状態を前提に書き直すこと。
- **A3 の実送信の条件**(棚卸しの「GJI+InjectionMode::Tsf かつ `AlreadyMatched`/ImmCross 成功」)は、コード(`platform.rs:1409` 付近、`should_send_accompanying_warmup`)と合っている。A3 の引き金の多くは C1 の書き込み結果なので、**P2 の後に A3 の発火頻度が変わる**(P4 の測定の前提が P2 で変わる)。

---

## (3) 段の順序と検証

### B1. P2 の計測方法は今の journal では取れない
決定4は「journal の `ActuationDecision`(`caller=DispatchImeSetOpen`)から `origin=ActivationSync` の件数」と書く。しかし `ActuationDecisionRecord` の `order.origin` は `EventOriginRecord { source: Physical|Injected|SelfActuated, epoch }`(`state/actuation_decision_record.rs:140-181`)で、**`SetOpenOrigin`(ExplicitUserAction/ActivationSync)を持たない**。
- **key 経路と refresh 経路で拾えるものが違う**: 打鍵の経路には `key_pipeline.rs:1336-1341` の info ログ「`IME control: … (SetOpenRequest, origin=ActivationSync)`」がある。
  しかし **`RefreshState` から来る遷移**(`ime_refresh.rs:1082`、`runtime/mod.rs:1391` → `execute_decision` → `executor.execute_from_loop` → `dispatch_ime_set_open`)は、`key_pipeline` の origin の分岐も `handle_engine_activation_sync` も通らない。したがって、このログにも `EngineActivationSync` イベントにも出ない。
  キー入力と無関係に起きる遷移(本文が一番気にしている TsfNative × belief 未知)が、まさにこちらに当たる。
- **決めること**: P2 の前に**計測専用の最小変更**を1PR入れる。例: `dispatch_ime_set_open` の入口で origin・経路(key/refresh)・結果(`Applied`/`AppliedWithoutSendInput`/`AlreadyMatched`/`Unwarranted`/`NotOwned`)・warrant の根拠を1行ログに出す。または `ActuationOrderRecord` に `set_open_origin` を足す。
- **「実送信」の数え方**: `outcome=Applied` だけで数えないこと。memory にある教訓「Applied だけでは操作成功と判断できない」のとおり、`win32.rs:205-220` の送信バッチの分類(`kanji_marker`/`tsf_marker_warmup`)のような SendInput 側の目印と突き合わせて数える。

### B2. 「belief 側の `handle_engine_activation_sync` は残す」は、そのまま残すと危ない
`handle_engine_activation_sync`(`platform_state.rs:631-690`)は、`EngineActivationSync` の記録に加えて、次のことをする。
- `on_set_open_requested()`(検出状態のリセット)
- `ImeApplyRequested { target, generation }`(`ime_model.rs:1013-` で **pending transition** を立てる)
- `last_explicit_ime_action_ms` の更新(idle-conv-check の抑制窓)

effect の発行元で `SetOpen` を落として、これを残すと、完了の来ない pending がタイムアウトまで残る。さらに「awase が書いた」扱いの抑制窓が、書いていないのに開く。
P2 の決定は「`EngineActivationSync` の記録だけを残し、`ImeApplyRequested`・`on_set_open_requested`・`last_explicit_ime_action_ms` の更新も落とす」と書くこと(key 経路の `key_pipeline.rs:1325-1333` と idle-conv-check の `:913` の両方)。

### B3. P2 で戻りうる不具合: apply 完了を受け取れないサブシステムの取り残し(BUG-170 型)
ActivationSync の書き込み結果は `on_ime_applied` → `feed_composition_event`・GjiFsm の同期・A3 の随伴 warmup を駆動している。
settle の再試行(M3)が存在する理由そのものが「apply 完了通知でしか同期しないサブシステムの固着」だった。BUG-170 は、書かない経路で GjiFsmSync の receipt が届かず、`OffCold/OnWarm` に取り残され、毎打鍵 per-VK confirm → StaleConfirm → ESC で未確定文字が消える、というもの。
ADR-203 (ii) は、予測 ON・shadow toggle ON のときだけ `GjiEvent::Reopen` を出す(`key_pipeline.rs:1606-1620`)。
P2 の後は、「観測/予測で Engine が active になった」ことを GjiFsm に伝える経路が無くなる可能性がある。P2 の検証に BUG-170 の症状(StaleConfirm の件数、ESC での未確定文字の消失)を入れ、必要なら Reopen の発火元に「Engine の活性遷移」を足す判断を、決定に含めること。

### M6. P2 の判断基準の誤読(2026-08-04)
決定4・代替案は、ActivationSync の SetOpen を止めると「IME OFF 後に Engine が勝手に ON へ戻る(2026-08-04)の再発対策」を壊す恐れがある、と書く。これは逆。
- `decision.rs:53-66` と `platform_state.rs:631-642` によれば、2026-08-04 の不具合の原因は、ActivationSync の echo を明示意図(`last_intent`)として記録していたこと。
- 対策は「echo を `last_intent`/`desired_open` に書かない」(belief 側)で、actuation 自体は対策ではない。
- actuation を止めても、この対策は壊れない。むしろ echo が IME に書かれる経路が減る。

ActivationSync の actuation が実際に担っている役割は、次の2つ。
- `engine.rs:406-407` の「inactive → active: OS IME を強制的に開く("nonaiyo" 問題対策)」。belief が開なのに実 IME が閉のとき、awase が実 IME を開けてしまう補正。
- 予測(ADR-191 の表、ADR-209、ADR-211)が belief を開にしたとき、warrant が下りれば予測を実 IME に**書いて自己成就させる**こと。これは ADR-191「予測は書かない」と矛盾する。

**P2 の後は、予測の誤りがリテラル出力として表に出るようになる**(今は隠れている可能性)。P2 の検証に、ADR-209/211 の予測の偽 ON の件数(`[key-effect-miss]`)を入れること。

### M7. P4(フラグ→ソーク)の運用
- **決定5との矛盾**: 決定5の「1段=1PR、実験フラグを混ぜない(#360 の教訓)」と、P4 の「環境変数フラグで無効化→ソーク→恒久化」(experiments エントリ10 の手順)は矛盾して読める。
  「P4 はフラグの PR(既定は従来どおり)と恒久化の PR の2つにする。フラグの PR には撤去以外を混ぜない」と明記すること。
- **ソークの前提**: A2/A3 が通るのは InjectionMode::Tsf(WezTerm 等)× GJI × Engine ON だけ。ソークする機械で WezTerm+GJI を実際に使っていなければ、経路が一度も通らないまま「無破損」になる。
  experiments エントリ10 のソークは WezTerm で行われた(L364)。今の所有者の常用がそうかを確認し、ソークの合格条件に「`[tsf-eager-warmup] VK_IME_ON 送信` の目印が、フラグなしの期間に N 件以上出ていた窓で、フラグありの期間に cold が60件超」を入れること。
- **順序**: A3 の引き金は P2 で変わる(m4)。P4 のソークは P2 の後に行うこと。

### P6
M5 のとおり B1 を3つに分けること。「ずれの持続時間」を測るときは、(b)(c) の持続時間と、(a) の再試行が効いた件数(明示意図のあとに drift correction が送った件数と、その後の観測の一致)を別に数えること。
実 Chrome では観測が乗らない(BUG-172)が、ADR-205 の watch は injected のときに遷移を拾える。P6 の測定にもこの watch を使えるかを検討する価値がある。

### P3
「実 Chrome × GJI で 0/10」と「RichEdit の入力先 tsf × GJI で 30/30 効いた」(棚卸しの A6b)の両方を本文に書くこと。今の本文は前者しか書いていない。30/30 の側(自前の RichEdit 窓、ADR-193)で撤去後の自己回復がどう変わるかを、CI で見る必要がある。

### m5. P0 の「退行なし」の範囲
棚卸し(A4)の記述では、#398 の実機 A/B は「IME ON・Engine OFF(生ローマ字を通す状態)」だけが経路を通り、NICOLA ON・MS-IME・Chrome は未測定。P0 の行に「測った範囲」を書くこと。

---

## (4) 完了条件と固定の方法

### M8. 決定6は `RESTRICTED_CALLS` と件数ガードでは固定できない
`lints/actuation_call_guard` は、**呼び出し元の関数名**で制限する(`lib.rs:58-116`、しかも `Warn`)。【許可】と【補正】は同じ関数を共有する。
- A1: `send_ime_mode_key` を `ime_controller::apply` から呼ぶ経路を、ExplicitUserAction・ActivationSync・B1 の3つが通る。
- B2: `set_ime_open_cross_process_async` を `open_chain` から呼ぶ経路も同じ。
- conv 側: `actuate_ime_control` → `modify_conv_mode` を、D1〜D8 の【許可】と【補正】が共有する。

関数名の許可リストで固定できるのは、専用関数を持つ経路(`send_eager_warmup_vk_pair`・`send_chrome_gji_reinit_and_poll`・`send_unicode_cold_warmup_keys`)が**消えたこと**までで、「共有チェーンに【補正】の origin が入らないこと」は固定できない。

固定する方法の案:
- **型で固定**: `ActuationOrder` の構築子が受け取れる origin を、【許可】の列挙(`ExplicitUserAction`・パニック・トレイ等)に限る。`SetOpenOrigin::ActivationSync` と drift の origin は、構築できないか別型にする。
- **architecture_guard で固定**: `dispatch_ime_set_open`/`apply_ime_open_with_belief`/`apply_ime_open_with_view` の呼び出し元の件数と、`SetOpenOrigin::ActivationSync` の構築箇所の件数を固定する。

どちらにするかを決定6に書くこと。

### m6. 完了条件の範囲
決定6は `VK_IME_ON/OFF`/F2 と開閉(`ImmSetOpenStatus`/`WM_IME_CONTROL`)だけを挙げる。次の扱いが書かれていない。
- conv 軸(`IMC_SETCONVERSIONMODE`、D1〜D3。P7 は「別に判断」)
- モードキーの注入(`VK_DBE_*`、A7/A8。残す)
- CapsLock(A9/A10。残す)
- ESC/BS の回収(m2)

「P7 の判断が出るまで、完了条件は開閉軸だけ」と明記すること。

---

## (5) 所有者決定との整合

- **半角英数トグルを残す**: 整合している。強制 Exit の扱いを明記すること(m3)。
- **ActivationSync は測ってから止める**: 方針は整合しているが、計測方法が今は成り立たない(B1)。判断の基準にも誤読がある(M6)。
  所有者への報告では、「測る前に計測用の小さな PR が要る」ことと「止めても 2026-08-04 の対策は壊れない」ことを伝えるべき。
- **ADR-191 の改訂が必要(M1)**: ADR-191 は L182 で「TSF cold-start warmup は既存の例外として残す(撤去対象外)」と書き、2026-09-29 の追記で「残る随伴 warmup は FocusChange と IME ON 適用直後のみ」としている。L312・L322 は EngineDecision を IME に書く振る舞いとして数えている。
  本 ADR の P2・P4・P5 はこれらを撤去するので、ADR-191 の該当節を改訂する(または本 ADR が ADR-191 の例外を縮小すると明記し、ADR-191 の status/related にも反映する)必要がある。
  所有者方針「actuation をなくす」は ADR-191 より新しいので、方向は問題ない。ただ文書の上で矛盾が残る。

---

## まとめ(実装前に決めること)

1. P2 の前に、計測専用の PR を入れる。origin・経路・結果・warrant の根拠を出し、SendInput の目印と突き合わせる(B1)。
2. P2 で落とす範囲。`SetOpen` の effect、`ImeApplyRequested`・pending、抑制窓の更新をまとめて落とすこと。idle-conv-check の直接呼び出し(M2)と settle の再試行(M3)も含めること(B2)。
3. P2 の検証に、BUG-170 型の取り残し(B3)と予測の偽 ON の表面化(M6)を入れる。2026-08-04 の記述を訂正する(M6)。
4. B1 を (a) 明示意図の再試行、(b) 古い desired の補正、(c) キャッシュの押し付け、に分けて P6 を組み直す(M4・M5)。
5. P4 のフラグ PR と恒久化 PR の分け方。ソークの前提(経路の目印の件数)。P2 の後に行うこと(M7)。
6. 決定6の固定方法(型か、architecture_guard か)と、完了条件の範囲(M8・m6)。
7. ADR-191 の例外節を改訂すること(M1)。P1 に `Platform::set_ime_open` を足すこと(m1)。
