---
id: ADR-212
title: |-
  ユーザー操作を引き金にしない予防的・補正的な IME への書き込み(VK_IME_ON/OFF の先回り送信など)を、段階的に撤去する
summary: |-
  所有者方針(2026-09-30): awase は IME に書かない(actuation をなくす)。ADR-191(IME が真実、観測する・書かない)と ADR-199(能動制御の例外は『IME ON/OFF トグルの役割のキー』と awase 自身の
  `keys.ime_on/off` だけ)が原則だが、確定キー(Enter)ごとの eager `VK_IME_ON`(PR #398 で撤去)のように、ユーザー操作を引き金にしない予防的・補正的な書き込みが残っている。
  棚卸し(`docs/tasks/actuation-inventory-2026-09-30.md`、develop `ccc966b8` 時点)と Opus round1 の裏取りで、`VK_IME_ON` を送る経路6つ(コード上で積む場所は4つ)、eager warmup の呼び出し元(#398 後は実質3つ+デッド1つ)、
  Engine の ON/OFF の遷移が自動で IME の開閉を書く `ActivationSync` 起源の SetOpen(ADR-191 が「EngineDecision」として認識しつつ「分離の是非を調査してから」と先送りしていたもの)を確認した。
  本 ADR は、【許可】(ユーザーが押した/設定したキーへの直接の応答)と【出力】(文字を出すための注入)を対象外に置き、【予防的】【補正的】な経路を、優先順位つきの段階(P0〜P7)で撤去する計画を決める。
  ADR-191 の「warmup は既存の例外として残す」「EngineDecision」は、本 ADR で縮小・改訂する(所有者方針が ADR-191 より新しい)。各段は1PR・revert しやすい単位・実機A/B と CI で退行を確認し、`docs/experiments.md` に判定を残す。
  所有者決定: 左 Shift 単独タップの半角英数トグルは残す(対象外)、ActivationSync は実機で実送信の件数を測ってから止める。CI結果を受けた判断(2026-10-01): 全面停止は却下し、`handle_engine_activation_sync` の gate による縮小は実書き込みを止めないため取り下げた。P2 は方針見直しとして保留する。
status: |-
  ほぼ実装済み(v2.0.0 に含まれる)。P0・P1・P3・P4・P5・P6(b)(c) は develop 実装済み、P2〈ActivationSync〉は ADR-213 の P2a〜P2d-2 で撤去済み(ActivationSync の型・ハンドラは現行コードに無い、2026-10-04 確認)。P6 の (a) drift correction は『許可』として意図的に残す。
  旧(2026-10-04 更新前):
  採用(2026-09-30)。実装状況(2026-10-01): P0(#398)・P1(#399)・P3+P5 の reinit 分(#402)・P4(#401)・P5 の Unicode long-cold warmup(#403)・P6 の (b)(c)(#404)は develop に実装済み。P2(ActivationSync)は [ADR-213](213-shadow-toggle-off-to-on-explicit-actuation-then-remove-activation-sync.md) の段階(P2a〜P2c)で再開(P2a は PR #408)。全面停止は CI で却下、gate による縮小は無効のため取り下げ。新スレッド=閉は ADR-191 の belief 側の改善として実装済み。GJI は CI で検証済み。同定済み MS-IME 本体にも適用されるが、その経路は CI 未検証。
related_adr:
  - "ADR-098"
  - "ADR-100"
  - "ADR-154"
  - "ADR-191"
  - "ADR-199"
  - "ADR-203"
  - "ADR-205"
  - "ADR-206"
  - "ADR-207"
  - "ADR-211"
  - "ADR-213"
---

# ADR-212: 予防的・補正的な IME actuation の段階的撤去

## 背景
**所有者の方針(2026-09-30)**: 「actuation をなくそう、と何度も伝えていた」。awase は、IME の状態を観測・予測して追随し、IME に書かない。書いてよいのは、ユーザーが押した/設定したキーへの直接の応答だけ
(ADR-199 決定1・2、ADR-206)。

しかし、確定キー(Enter)を通すたびに、awase がその場で `VK_IME_ON` を送る eager warmup が残っていた(実機 A/B: Enter 24回中24回。PR #398 で撤去、0回)。
「他にも予防的な `VK_IME_ON` が残っていないか。すべて撤去したい」という要望を受けて全経路を棚卸しした
(`docs/tasks/actuation-inventory-2026-09-30.md`。事実=コードを読んで確認、推測=未確認と明記)。Opus round1 が事実をコードで裏取りした(下記に反映)。

### 棚卸しの要点(develop `ccc966b8`、Opus round1 で裏取り済み)
- F2(0x71)を送る経路は現存しない。`VK_IME_ON`(0x16)を送る**経路**は6つ(A1・A2・A3・A5・A6・A6b。#398 で A4 が消えた後)。**コード上で `VK_IME_ON` を INPUT に積む場所は4つ**(`key_sequence_policy.rs`〈A1〉、`tsf/send.rs`〈A2/A3〉、`output/mod.rs`〈A5〉、`probe_io.rs`〈A6/A6b〉)。
- **予防的・補正的で、ユーザー操作を引き金にしないもの**(棚卸しの ID):
  A2 フォーカス変更時の eager warmup、A3 開く書き込みの後の随伴 eager warmup、A5 Unicode long-cold の `VK_IME_ON`+`VK_A`+`BS`、A6 Unicode long-cold の `VK_IME_OFF`→`VK_IME_ON` reinit(Actuation 起点のみ)、
  A6b Chrome/TSF リテラル2連続 give-up 後の reinit、B1 drift correction、C1 `ActivationSync` 起源の SetOpen(およびその関連: 下記 C2・C3)、D1/D2 ROMAN 補完、D3 cold 時の ROMAN 保護(conv 軸)。
- **デッドコード**: 記号 VK の生フォールバック(`vk_send.rs`)の `send_eager_tsf_warmup(WarmupImeOn::off(), …)`(実送信されない)。**`Platform::set_ime_open` はデッドコードではない**(`set_ime_open_ordered` が完全修飾の構文 `PlatformRuntime::set_ime_open(self, open)` で呼び、B1 drift correction の ImmCross の書き込み経路そのもの。`architecture_guard` の `.set_ime_open(` の0件はメソッド呼び出しの形しか数えていない。round2 M-N1 が round1 の誤りを撤回)。P6 で B1 の形が決まった後の整理(挙動を変えないリファクタ)として扱う。
- **C1(ActivationSync)**: Engine の active/inactive 遷移が、対称性のために自動で `SetOpen` を発行し、executor が origin を見ずに実 VK/`ImmSetOpenStatus` の書き込みへ流す。
  ADR-191 は IME に書く振る舞いとして「EngineDecision」を認識しつつ、「発生元の軸が要るので、分離の是非を調査してから」と先送りしていた(L312・L322)。
  - **C2**(実送信の経路ではなく、**書かないのに pending を立てる経路**): idle-conv-check の `kp_apply_conv_engine_sync` が、`EngineSync::SetOpen(RomajiRecovered)` のとき Engine を経由せず `handle_engine_activation_sync` を**直接**呼ぶ。`SetOpen` の effect は出さず実 IME には書かないが、今すでに、完了の来ない pending transition(タイムアウトあり)と、書いていない抑制窓(`last_explicit_ime_action_ms`)を立てている(コメントの「actuation は同一」は実装と食い違う)。
  - **C3**(再試行): 焦点の遷移中に落とした `SetOpen` を、settle 明けに出し直す仕組み(`strip_ime_set_open_if_settling`・`schedule_settle_retry`。2026-07-08「このせっけい→せっけい」対策=apply 完了通知でしか同期しないサブシステムの固着の防止)。**注(ADR-213 P2d-2、2026-10-01)**: strip と strip した SetOpen の再試行は撤去済み。`schedule_settle_retry` は drift correction の settle 延期用だけ残る(P6 まで)。
- **ActivationSync の actuation が担っていること(M6)**: 2026-08-04(IME OFF 後に Engine が勝手に ON へ戻る)の対策は、echo を `last_intent`/`desired_open` に書かないこと(belief 側)で、**actuation 自体は対策ではない**。actuation が実際に担うのは
  (i) `engine.rs` の「inactive → active: OS IME を強制的に開く(『nonaiyo』問題対策)」= belief が開・実 IME が閉のとき awase が開ける補正、(ii) 予測(ADR-191 の表・ADR-209・ADR-211)が belief を開にしたとき、warrant が下りれば**予測を実 IME に書いて自己成就させる**こと。(ii) は ADR-191「予測は書かない」と矛盾する。
  actuation を止めると、予測の誤りがリテラル出力として表に出るようになる(今は隠れている可能性)。
- **B1 drift correction は3つの性質が混ざっている(M5)**: (a) ユーザーの明示操作(【許可】)の書き込みが実 IME に届かなかったときの再試行(明示意図があれば閾値0。BUG-16/20 型)、(b) belief/desired が古いことによる補正(BUG-157 型の誤作動の源)、(c) 焦点が戻った窓のキャッシュから `desired_open` を復元(`apply_hwnd_cache_restore`)して押し付ける(窓ごとの IME 状態を awase が覚えて書く)。

## 決定

1. **範囲**。本 ADR が撤去の対象にするのは、次の【予防的】【補正的】な、IME への書き込み・IME キーの注入。
   - 【予防的】: ユーザー操作を引き金にせず、あとで起きうる不具合を先に防ぐために送る(warmup、cold 化対策、先回りの再主張)。
   - 【補正的】: belief と実状態のずれを直すために書く(drift correction の (b)(c)、reinit、ROMAN 補完、ActivationSync)。
2. **対象外**(残す)。
   - 【許可】: ユーザーが押した/設定したキーへの直接の応答(`keys.ime_on/off`・ADR-199/206 の役割トグル・ユーザーが押した IME キーの素通し・Ctrl+変換のリセット・トレイ/パニックリセット・`[[keymap]]` 一致時の composition キャンセル)。
     **drift correction (a)**(明示操作の書き込みが届かなかったときの再試行)は、【許可】の書き込みの信頼性の一部として P6 まで残す。P6 で「意図の有効期限内の1回だけの再試行」に縮めるかを決める。
   - 【出力】: 文字を出すための注入(文字・BS・ESC、Unicode 注入、リテラル掃除)。IME の状態を変えない。ただし StaleConfirm 等の**自分の出力の回収**としての ESC+BS(`flush_raw_tsf_literal_backspaces`)は、引き金が awase の判定で未確定文字を消しうる(BUG-170/171)ので、【出力】に残すのは「自分が出した文字の回収」に限る理由で、P3(同じ give-up の reinit)と境界を分ける。
   - **左 Shift 単独タップの半角英数トグル**(A7/A8/D4/D6): IME キーではないが、ユーザーの明示操作による機能で、所有者決定(2026-09-30)で残す。**awase が作った状態の後始末としての強制 Exit(フォーカス変更時・IME ON 時)も残す**(ユーザー操作が引き金ではないが、awase が作った状態を戻すため)。
3. **段階**(優先順位。各段は1PR・revert しやすい単位。前の段の結果を見てから次へ)。
   | 段 | 対象 | 内容 | 進め方 |
   |---|---|---|---|
   | P0 | A4 確定キー reinject の eager warmup | 済み(#398)。実機 24→0、入力の欠落・リテラル化は増えなかった | **測った範囲**: 実機は IME ON・Engine OFF(生ローマ字を通す状態。NICOLA ON では、この経路は通らない〈`[relay-defer]`〉)、WT+GJI の MS-IME プリセット、n=24。**未測定**: NICOLA ON・MS-IME 本体・Chrome の実機 |
   | P1 | デッドコード(記号 VK フォールバックの `send_eager_tsf_warmup(off)`、`WarmupImeOn::off()`・`WarmupOrigin::Off`) | 撤去。挙動は変わらない | PR #399。コンパイル+`architecture_guard`(eager warmup 送信元の件数 3→2)。**`Platform::set_ime_open` は含めない**(上記) |
   | P2 | C1・C2・C3 ActivationSync | **ADR-213 で再開**(P2a=PR #408、P2b/P2b'/P2c 未実装)。下の決定4・5 | 全面停止は却下、gate による縮小は無効。shadow toggle の OFF→ON を明示 actuation にしてから止める(スパイクは CI で退行なし) |
   | P3 | A6b Chrome/TSF give-up 後の reinit | 撤去。BS/ESC の回収だけに縮退 | 実 Chrome×GJI で 0/10 と実測で効かず、BUG-168 で入力中文字を消す副作用も既知。**一方、自前の RichEdit 窓(tsf×GJI、ADR-193)では 30/30 効いた**(review-2026-09-24-09)。CI で、撤去後に RichEdit 窓の自己回復がどう変わるかを見る |
   | P4 | A2 フォーカス変更 eager warmup、A3 随伴 eager warmup | 撤去。InjectionMode::Tsf(WezTerm 等)+GJI だけに効く | **P2 の後に行う**(A3 の引き金の多くは C1 の書き込み結果なので、P2 で発火頻度が変わる)。**2つの PR に分ける**: (1) 環境変数フラグの PR(既定は従来どおり、撤去以外を混ぜない)、(2) 恒久化の PR。ソークの合格条件: フラグなしの期間に `[tsf-eager-warmup]` の送信の目印が N 件以上出ていた窓で、フラグありの期間に cold が 60 件超で無破損(経路が一度も通らないまま「無破損」にしない)。WezTerm+GJI を実際に使っていない機械では判断できない |
   | P5 | A6 Unicode long-cold の reinit(Actuation 起点)、A5 Unicode long-cold の `VK_IME_ON`+`VK_A`+`BS` | **P2 の後の状態を前提に判断する**(P2 で C1 が消えると、Actuation 起点はユーザーの IME キーだけになり、A6 は A3 と同じ「ユーザーの書き込みに付随する warmup」になる)。A5 は高リスク(Unicode 注入は GJI の確認を迂回する) | 実機: Windows Terminal+GJI、10s 以上 idle 後の1文字目(`bあ` 型欠落) |
   | P6 | B1 drift correction の (b)(c) | **(b) 古い desired の補正と (c) HWND キャッシュの復元の押し付けを先に外す**。(a) は決定2 | (b)(c) の持続時間と、(a) の再試行が効いた件数(明示意図のあとに drift correction が送った件数と、その後の観測の一致)を**別に数える**。実 Chrome では観測が乗らない(BUG-172)が、ADR-205 の watch は injected のときに遷移を拾えるので、測定に使えるか検討する。**(c) を外す前に、所有者に「awase が窓ごとの IME 状態を覚えて戻す」ことを機能として期待していないか確認する**(Windows の IME はもともとスレッド/窓ごとに開閉を保持する) |
   | P7 | D1/D2 ROMAN 補完、D3 cold 時の ROMAN 保護 | conv 軸。別に判断する | MS-IME 本体の実機。D3 は ADR-191 決定1が warmup 例外として維持 |
4. **P2 の計測**(ActivationSync)。**今の journal では取れない**: `ActuationDecisionRecord` の `order.origin` は `EventOriginRecord { source: Physical|Injected|SelfActuated, epoch }` で、`SetOpenOrigin`(ExplicitUserAction/ActivationSync)を持たない。
   打鍵の経路には origin を出す info ログがあるが、`RefreshState` から来る遷移(キー入力と無関係。TsfNative × belief 未知が最も気にしている経路)は出ない。
   → **計測専用の最小変更(PR #400)**: `dispatch_effect` で `[set-open] origin=… open=… generation=… outcome=…` を出す(key 経路・refresh 経路の両方が通る。sync は outcome を同じ行に、async〈ImmCross 先の窓〉は `outcome=async` で `generation` を出し、後から届く `on_ime_apply_complete{generation outcome}` の行と突き合わせる)。
   **数え方**: 実機(GJI+Windows Terminal、GJI+Chrome/Edge、MS-IME+メモ帳の通常使用)で、`[set-open] origin=ActivationSync` の件数を、直後の `actuation decision`/`[apply-ime]` の outcome(`Applied`/`AppliedWithoutSendInput`/`AlreadyMatched`/`Unwarranted`/`NotOwned`)別に数える。
   **注意(round3 m9)**: refresh 経路の async では `generation` が `None` や前の値になり、完了ログとの突き合わせに使えない場合がある。その場合は時刻と `open` の一致で対応を取る。
   **範囲外の2つは、別の既存のログで数える**: (C3 の strip は ADR-213 P2d-2 で撤去済みで、数える対象がなくなった)、C2 は `[idle-conv-check] TsfNative: engine ON 同期`(C2 は `dispatch_effect` を通らず、書かないので実送信の数える対象でもない)。
   **棚卸し漏れ(2026-10-01)**: Engine の非キーボード経路(`FocusChanged`/`RefreshState` の `check_active_transition`、`runtime/mod.rs::execute_decision`→`executor.execute_from_loop`)は `handle_engine_activation_sync` を通らない。CI では awase 起動47ms後、観測ゼロのbeliefから`dispatch_ime_set_open{open=true}`→`GJI direct`→`outcome=Applied`を確認した。C1〜C3だけでは入口を網羅していなかった。
   **「実送信」は `outcome=Applied` だけで数えない**(ActuationDecision.outcome:Applied だけでは操作成功と判断できない、BUG-141)。`win32.rs` の SendInput のバッチ分類(`kanji_marker` 等の目印)と突き合わせる。窓の種類・belief の状態(既知/未知)別に。
5. **P2 は保留(方針見直し、2026-10-01)。再開の設計と段階は [ADR-213](213-shadow-toggle-off-to-on-explicit-actuation-then-remove-activation-sync.md)(P2a/P2b/P2b'/P2c)**。
   - **全面停止は却下**: Engine で ActivationSync の `SetOpen` を出さないスパイクでは、`sc-hz-*`・`sc-kanji-*` が2回押した後に反転せず、cold 起動にも退行した。
   - **gate による縮小は無効のため取り下げ**: `handle_engine_activation_sync` 先頭で棄却しても、pending・抑制窓・`EngineActivationSync` の記録を省くだけで、decision の effect は `kp_stage_execute`→executor へ流れた。`[activation-sync] skipped SetOpen(true)` の直後に同じ打鍵の `GJI direct: send 0x0016`・`outcome=Applied` があるため、実書き込みは続いていた。
   - Imm32Unavailable では物理の半角/全角(0x16等)をOSへ届けず(`[imm32-off] key suppress`)、shadow toggleでbeliefをONにする。shadow toggleはON→OFFを書き込むがOFF→ONを書かないため、そのEngine活性化に伴うActivationSyncの`SetOpen(true)`が唯一の実ON書き込みになる。この経路は【予防的・補正的】ではなく、ユーザーが押したキーへの直接の応答(決定2の【許可】)を担う。
   - したがって、ActivationSync のvariantごとの削除やEngineで`SetOpen`を出さない変更には、先にshadow toggleのOFF→ONを明示的なactuation(`ExplicitUserAction`相当)にする設計が必要。それ無しでは全面停止と同じ退行になる。
   - **C3**: settle の strip は ExplicitUserAction の SetOpen も落とすため、P2再開時に `schedule_settle_retry` の他用途を含めて要否を確認する。→ ADR-213 P2d-2 で strip・settle フィルタを撤去、`schedule_settle_retry` は drift correction 用だけ残した。
   **止めたときに戻りうる不具合(検証に入れる)**:
   - **BUG-170 型**: ActivationSync の書き込み結果は `on_ime_applied` → GjiFsm の同期・`feed_composition_event`・A3 の随伴 warmup を駆動している。書かなくなると、apply 完了通知でしか同期しない GjiFsm 等が `OffCold/OnWarm` に取り残され、毎打鍵 per-VK confirm → StaleConfirm → ESC で未確定文字が消える。ADR-203(ii)の `GjiEvent::Reopen` は予測 ON・shadow toggle ON の場合だけ発火する。**「Engine が観測/予測で active になった」ことを GjiFsm に伝える経路が無くなる可能性がある**ので、必要なら `Reopen` の発火元に「Engine の活性遷移」を足す判断を含める。検証: StaleConfirm の件数、ESC での未確定文字の消失。
   - **予測の誤りの表面化**(上の M6): ADR-209/211 の予測の偽 ON の件数(`[key-effect-miss]`)を見る。
   - **新スレッド=閉は独立したbelief側の改善**: `focus/thread_scope.rs`・`state/platform_state.rs::assume_closed_for_new_thread`・`runtime/focus_tracking.rs`で、awase起動後に作られた分類済みhwnd/pidのスレッドを初めてプロセス変更で見たとき、Imm32Unavailableのcache missにLow confidenceのHeuristicDefault「閉」を記録する。同じプロセス内のフォーカス移動は`on_focus_process_changed`を通らないため、「初めて見た」は「初めてプロセス変更で見た」の意味である。適用は`SPI_GETTHREADLOCALINPUTSETTINGS==0`かつGJIまたは同定済みMS-IME本体(`table_ime_kind()`)に限定し、ポーリング確定前の`table_ime_kind()==None`では使わない。GJI は CI で`applied=true`→`ka`を検証済み。同定済みMS-IME本体にも適用されるが、CIのMS-IME構成はTIPを`MicrosoftIme (Other)`（IMM32 HKL）と同定して条件を満たさないため、**同定済みMS-IME本体×新スレッド=閉の経路はCIで一度も通っておらず未検証**。学習で`Standard`から`Imm32Unavailable`へ降格したアプリも対象に含む（Chrome以外は未測定）。
   - **範囲外の前提**: Windows 11実機、Chrome以外、GJIの他のキーマップ、アプリ自身が起動時にIMEを開ける場合（WinFormsの`ImeMode=On`等）は未確認。
   - **残り**: Windows 11実機でMS-IME本体が`table_ime_kind()==Some(MsIme)`と同定され、`[thread-scope] applied=true ime_kind=Some(MsIme)`→`ka`になることを確認する。
   - **revert条件**: その窓の直前に`[thread-scope] … applied=true`があり、IMEが実は開いていて最初の入力が逆向きにずれた場合は、`fix(ADR-212 P2): awase 起動後に作られたスレッドの IME 開閉を「閉」と記録する` のコミット（1本）をrevertする。
6. **各段の共通の規約**。
   - 1段=1PR。撤去の内容だけを含める(実験フラグや CI 構成を混ぜない。#360 の教訓)。ただし P4 は、上のとおりフラグ PR と恒久化 PR の2つ。
   - `fix-requires-evidence.md`(warmup/focus/belief/conv/actuation 合流点の再発ファミリー): 回帰テスト(golden/journal replay)か `docs/known-bugs/` を添える。
   - 検証: 実機の A/B(develop と撤去版、同条件)と、CI(該当する `tsx-*`/`sc-*`)。**測るのは「撤去した送信の数」と「入力の欠落・リテラル化・`@`・StaleConfirm が増えないか」**。感度が低い CI だけで「差なし」と結論しない(実機で経路が通ることを、ログの目印で確認する。#398 の A/B で、最初の条件が経路を通っていなかったことを見つけた)。
   - `docs/experiments.md` に判定を追記する。撤去を revert するときは `experiment-logging.md` に従い、アプリ・IME・症状を本文に書く。
   - `complexity-budget.md`(未発効): `RESTRICTED_CALLS` の関数や tuning 定数(`CHROME_GJI_REINIT_*` など)の削除は、差し引きとして記録する。
   - **ADR-191 の改訂**: ADR-191 は L182 で「TSF cold-start warmup は既存の例外として残す(撤去対象外)」と書き、2026-09-29 の追記で「残る随伴 warmup は FocusChange と IME ON 適用直後のみ」とし、L312・L322 は EngineDecision を IME に書く振る舞いとして数えている。
     本 ADR の P2・P4・P5 はこれらを撤去するので、各段の PR で ADR-191 の該当節を改訂する(または本 ADR が ADR-191 の例外を縮小すると明記し、ADR-191 の status/related にも反映する)。所有者方針が ADR-191 より新しい。
7. **完了条件と固定の方法**。`VK_IME_ON`/`VK_IME_OFF`/F2 を SendInput で送る経路、および `ImmSetOpenStatus`/`WM_IME_CONTROL` で**開閉**を書く経路が、【許可】(決定2)の起点だけになる。
   **固定の方法**: `lints/actuation_call_guard`(`RESTRICTED_CALLS`)は呼び出し元の**関数名**で制限し、【許可】と【補正】が同じ関数のチェーン(`ime_controller::apply` → `send_ime_mode_key`、`open_chain` → `set_ime_open_cross_process_async`、`actuate_ime_control` → `modify_conv_mode`)を共有するので、
   「共有チェーンに【補正】の origin が入らないこと」は固定できない(専用関数を持つ経路〈`send_eager_warmup_vk_pair`・`send_chrome_gji_reinit_and_poll`・`send_unicode_cold_warmup_keys`〉が**消えたこと**までは固定できる)。
   P2で`SetOpenOrigin::ActivationSync`のvariantごと削除する案は、決定5のとおりshadow toggleのOFF→ONを明示的なactuationにする設計まで保留する。それ以外は **`architecture_guard` の件数で固定する**: `dispatch_ime_set_open`/`apply_ime_open_with_belief`/`apply_ime_open_with_view` の呼び出し元の件数。
   件数のガードは「検出の仕掛け」であって証明ではない(P6 の後も (a) のために drift correction からの呼び出しが残るので、『残っているのが (a) だけ』は件数では示せない)。(a) だけを残すことは、`drift_correction.rs` の条件(明示意図があるときだけ発火)を Linux で走る単体テスト(`tests/closed_loop_scenarios.rs` 等)で固定する。
   型で固定する案(`ActuationOrder` の構築子が受け取れる origin を【許可】の列挙に限る)は、P6 の後に、残る origin が確定してから別途検討する。
   **範囲**: P7 の判断が出るまで、完了条件は開閉軸だけ(conv 軸〈`IMC_SETCONVERSIONMODE`・D1〜D3〉、モードキーの注入〈`VK_DBE_*`・A7/A8、残す〉、CapsLock〈A9/A10、残す〉、ESC/BS の回収は含めない)。

## 検証方針(実機 A/B の手順)
#398 の実機 A/B で、clipwire のターゲット(`ab-*`、リポジトリ外の `targets.toml`)を作った。再利用できるよう、要点を `tools/e2e/ime_key_matrix/device/` に置く(実装時に決める)。要点:
- 同じ `config.toml`、`AWASE_TEST_INJECTION=1`(目印つき注入を物理扱い)、`RUST_LOG=debug`。awase は**標準エラーを記録する起動**にする(単純な `Start-Process` だと、途中で止まることがあった。原因は未特定)。
- A(develop)と B(撤去版)の exe を並べて置き、同じ注入手順で比べる。注入は、**前面の窓が意図した窓であること**を、窓のハンドルで確認してから行う(プロセス名だけの確認では、別の窓に入る)。
- ログの目印で、対象の経路が実際に通ったことを確認する(例: `[composition] reinject KeyDown … marking cold + eager warmup`)。通っていなければ、その条件は測定になっていない。

## 非目的
- 半角英数トグル(所有者決定で残す)。ADR-199/206 の役割トグル・`keys.ime_on/off`。文字の出力そのもの。
- 観測(読み取り)の追加・変更。

## 代替案
- **すべて一括で撤去**: 退行の原因の特定が難しくなる。過去に「IME OFF に何を送るか」が5日間で6回反転した(experiments エントリ 01)。段階的に、各段の実測を残す方が、戻すときの根拠になる。
- **測らずに ActivationSync を止める**: 原則(ADR-191)には合うが、上のとおり、(i)「nonaiyo」問題の補正(belief が開・実 IME が閉のときに開ける)と、(ii) 予測が実 IME に書かれて自己成就している分が、止めると表面化する。所有者決定で、先に測る。
  なお、2026-08-04 の再発対策(echo を明示意図にしない)は actuation ではなく belief 側なので、止めても壊れない。

## リスク
1. 予防的な送信を外すと、cold 直後の最初の1文字のリテラル化・欠落(BUG-02 系、BUG-40)が戻りうる。段ごとに実機で見る。
2. 補正的な書き込み(drift correction)を外すと、belief と実 IME のずれが観測で上書きされるまで残る。P6 で持続時間を先に測る。
3. CI(打鍵ストレス)は失敗が0件の環境で感度が低い。実機の A/B と、ログの目印での経路確認を必須にする。
4. ActivationSync を止めると、GjiFsm の取り残し(BUG-170 型)や、予測の誤りの表面化が起きうる(決定5)。
