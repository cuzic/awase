---
id: ADR-187
title: |-
  GJI(ATOK)で無変換/変換をパススルーするとき、生キーを通した直後に実IMEを読み直し、古い明示意図を捨てて
  Engineを観測に追随させる(follow方式。awaseはactuateしない)
summary: |-
  ATOKプリセット+パススルー(`gji_thumb_key_ime_toggle=false`、既定)では、実IMEはGJIが開閉するのにEngineが追随しない
  (IME OFFでもEngine ONのまま直接入力にNICOLA変換が効く。CIの実機E2Eで各3/3再現)。原因は3つ: (1)通過後に読み直す契機が無い、
  (2)直前の明示意図(`IntentStore`と`ImeModel::last_intent`)が`effective_open()`を固定し、観測が入っても`ctx.ime_on`が動かない、
  (3)通過後の20ms再読み取りがtyping-idleガードで観測されない。**ユーザー方針**: 変換前/変換中は原理的に分からない(awaseの
  `composing`は自身の出力履歴からの推定)ので、Toggleをawaseがactuateする案は推定が外れると未確定文字を捨てる。**follow方式**
  (生キーはGJIに通し、GJIが状態依存の動作をする。awaseは結果を観測して追随する)にする。実装: 通過点(FSMのSendKeysと、Engine OFF時の
  PassThrough)で通過マーク+20ms再読み取り(typing-idleバイパス)を予約し、観測が成功した直後に対象hwndのIntentStoreと`last_intent`を
  捨てる(新`ImeEvent::ModeKeyPassedThrough`)。スパイクCIで全12手順追随(各3/3)・cold各3/3・退行なし。
status: |-
  **決定・実装済み(未マージ)**。スパイク(`spike/adr187-follow-observe`)でCI検証済み。本実装は`feat/adr187-follow-mode-key-passthrough`。
  未検証: TsfNative/Imm32Unavailable(メモ帳・Windows Terminal・Chrome/Edge)、Microsoft IME本体(別の既存の問題、ADR-186)。
related_adr:
  - "ADR-090"
  - "ADR-115"
  - "ADR-179"
  - "ADR-186"
---

# ADR-187: ATOKパススルーでの無変換/変換に対するEngine追随(follow方式)

レビュー: [round1](187-opus-review-round1.md)、[round2](187-opus-review-round2.md)(観測型の設計の穴の洗い出し。本ADRの実装はこの指摘を反映)。

## 背景

ユーザー要件(ADR-186): **かな=Engine ON、英数(半角英数・直接入力)=Engine OFF、押下直後から**。

ADR-186は、`gji_thumb_key_ime_toggle=true`(opt-in、awaseがToggleをactuate)なら追随することを実機とCIで確認した。opt-in無し(既定、
生キーをGJIへパススルー)のATOKユーザーは未解決だった。CIの`--walk`(ひらがな/無変換/変換の固定12押下)で、`atok-passthrough`と
`atok-passthrough-henkan`は各3/3が、無変換/変換で実IMEがOFFになる手順でEngineがONのまま残った。

ログ(無変換): KeyDownは`PendingThumb`でConsume→**KeyUpで**単独タップ確定(パススルー設定の親指は既に`defers_solo_until_release`の対象)→
`send_keys: Key(0x1D)`で生キー送出→その後、`IME snapshot`/`stage-observe`/`drift`/`Engine (de)activated`のいずれも出ない。
awaseは何も観測していない。

## 原因

1. **通過後に読み直す契機が無い。** 物理IMEキーの通過後20ms再読み取り(`key_pipeline.rs`、`!decision.is_consumed() && may_change_ime &&
   KeyDown`)は、`may_change_ime`が無変換/変換を意図的に含まず(`vk.rs`の「第3の軸」)、親指キーのKeyDownはFSMがConsumeするため発火しない。
2. **直前の明示意図が`effective_open()`を固定する(支配的)。** 2重の固定: (P1)`ImeModel::resolve_open_at`が`last_intent.is_some()`の間
   `desired_open`を観測より優先する(TTLなし、`FocusChanged`でのみクリア)。(P2)`ImeStateHub::effective_open_at`が`IntentStore`の
   対象hwndの意図(ON 10秒/OFF 30秒)を返す。ひらがなキー等の明示意図が残る間、観測が入っても`ctx.ime_on`は動かない。**読み直しだけでは
   Engineは追随しない**。
3. **通過後の再読み取りがtyping-idleガードで空振りする。** `ir_decide_read_strategy`は、20ms後は必ず`idle<500ms`なので、
   `explicit_verify`(`explicit_intent().is_some() && applied!=Unknown`)が偽だと`SkipTyping`で観測しない。

## 検討した案

- **Toggleをawaseがactuateする(opt-inを既定`true`)**(PR #227、**closed**)。ATOKの無変換/変換は「入力なしのときだけ」開閉トグルで、
  入力中(変換前=半角英数トグル、変換中=次候補)は別の動作をする。awaseの`composing`は自身の出力履歴からの推定でしかなく、変換前/変換中を
  区別できない(原理的に不可能)。推定が外れると未確定文字を捨てる。
- **Toggleを予測してbeliefを書く**(`FollowOnly(Toggle)`、`!belief`)。同じ理由で外れる。ADR-179/BUG-115の設計判断と衝突する。
- **観測値を明示意図として記録する。** `IntentWitness::from_physical`が0x1C/0x1Dを受理せず黙って空振りする、OFF意図が30秒固着する、
  Medium観測が明示意図としてHigh観測より強い権限を持つ(ime-belief-architectureが禁じる観測の意図への偽装に近い)、ActivationSync二重actuation。
- **採用: follow方式。** 生キーはGJIに通し、GJIが全状態で正しく動作する。awaseは`composing`を知る必要がなく、通過後の**結果(open/conv)を
  読んで追随する**。古い明示意図は「ユーザーがIME操作をした。結果は分からないので古い意図を根拠にしない」として無効化する
  (値は書かない、witness不要)。

## 決定

無変換/変換(`vk.rs::is_ime_mode_key_for_ime`)の**生キーをGJIへ通過させた**とき:

1. **通過マークを立てる**: `ImeStateHub`の`ScopedOneShot<ForegroundScope, ModeKeyPassMark>`(窓 `MODE_KEY_PASS_MARK_WINDOW_MS`=300ms、
   フォアグラウンドが変われば`peek`が自動失効するのでFocusChanged配線は不要)。**一回では消費しない**(下記)。
2. **20ms後にIME再読み取りを予約**する(`TIMER_IME_REFRESH`)。
3. **再読み取りはtyping-idleガードをバイパス**する(通過マークが有効な間。`explicit_verify`の隣の第2条件)。
4. **観測が成功した直後に**、対象hwndの`IntentStore`エントリと`last_intent`を捨てる(`ImeEvent::ModeKeyPassedThrough`、`last_intent`のみ書く。
   dispatch元は`ImeStateHub::invalidate_intents_if_mode_key_pass_live`の1箇所)。観測の**後**に捨てるので、beliefが古いdesired_openへ
   一瞬戻ることがない。
5. **窓が切れるまで`MODE_KEY_PASS_REREAD_MS`(60ms)ごとに読み直す**: 通過から最初の再読み取りまでにGJIがキーを処理し終えているとは限らない
   (CIの`atok-passthrough-henkan-cold`で、通過から11ms後に古い状態を読み、マークを一回で消費したため追随できなかった回があった)。
   マークは消費せず、観測が入るたびに再読み取りを予約し、窓(300ms)が切れたら止まる。

**通過点は2つ必要**: (A)FSM経由の送出(`executor::dispatch_effect`の`SendKeys`、Engine ON時の単独タップ確定)と、(B)Engine OFFのとき
無変換/変換がFSMを通らず`PassThrough`判定でOSへ渡る経路(`key_pipeline`)。(B)は**`shadow_action`が無い(awaseがIMEキーとして扱わない)**
場合に限る。opt-in(Toggle、`shadow_action`あり)は従来のshadow-toggle/delegate経路に任せる: followでbeliefが正しくOFFを追随すると、
opt-inの「直接入力→無変換でON」で`UserImeOnEisuReset`(かなに戻ると仮定してEngineを即ON)+`ActivationSync`のSetOpen(true)が走り、
Engineが約70ms一瞬ONになるため(スパイクv2で確認)。

## 検証(スパイク、CI `e2e-ime`、各3回)

| 構成 | 結果 |
|---|---|
| `atok-passthrough`/`atok-passthrough-henkan`(明示`false`、パススルー) | **全12手順追随**(以前は各3/3不追随) |
| `atok-passthrough-cold`/`-henkan-cold`(先頭のひらがなを除き、明示意図なしでいきなり無変換/変換) | **各3/3追随** |
| `baseline`/`baseline-henkan`(opt-in、ATOK用strict期待表) | 退行なし(3/3 PASS) |
| `atok-optin`、`msime`、`msime-optin`、`msime-stale-table` | 退行なし(各3/3) |

スパイクの版: v1(SendKeys点のみ)は11/12(OFF→ONが追随せず=Engine OFF時はFSMを通らない)。v2(PassThrough経路を追加)で12/12だがopt-inが退行
(上記の一瞬ON)。v3(観測直後に捨てる)。**v4(`shadow_action`なしに限定)で全構成OK**。

副作用(ATOKパススルー12押下): ActivationSync SetOpen 3件(opt-inは5件)、VK_IME_OFF→ONのwarmupバースト2件(opt-inと同じ、ひらがな起因で
無変換/変換連動なし)。`last_intent`を捨てるため周期ポーリング(`reschedule_ime_refresh`の停止)が再開する(OsPoll 58回、opt-inは35回)。

## 受け入れ基準(適用範囲)と未検証

- **IMMのクロスプロセス読み取りが効くアプリ**(`profile=ImmCross`/Win32 Edit系、CIの対象)で要件を満たす。TsfNative/Imm32Unavailable
  (メモ帳・Windows Terminal・Chrome/Edge)は`ime_on=None`で読めず、従来どおりidle-conv-check頼み(未検証。BUG-149参照)。
- **Microsoft IME本体**はCIでawaseのIME ON書き込みが失敗する別の既存の問題(ADR-186)。本変更とは無関係。
- 観測が空振り(IMM miss)した場合は意図を捨てない(観測成功時のみ)。通過マークは窓(300ms)で失効し、それまでは観測のたびに60ms間隔で読み直す。
- 開閉が変わらない場合(入力中の無変換=半角英数トグル等)も、観測が成功すれば意図は捨てられる(beliefは観測に従う。実IMEと一致するので害は小さい)。
- 通過マークの窓`MODE_KEY_PASS_MARK_WINDOW_MS`(300ms)と読み直し間隔`MODE_KEY_PASS_REREAD_MS`(60ms)は暫定・未実測(`pending`)。CIでは押下後20〜70msにGJIの反応が出た。

## リスク

- `last_intent`を捨てると、直前の明示ON意図に守られていた「観測が誤って揺れる」場面(BUG-63型)でEngineが誤OFFになりうる。対象は無変換/変換の
  生キー通過時だけで、IMMで開閉を読めるアプリに限る。
- 追加の読み取り(クロスプロセスIMM、`run_with_timeout`)が、孤立した無変換/変換タップごとに1回増える。

## リセット操作(Ctrl+無変換→Ctrl+変換)の検証(CI `--resync`、各3回)

ずれを完全には防げないアプリ(読めないアプリ、非対応のキーマップ)向けに、「ずれても1操作で必ず戻せる」ことを確認した。
`keys.ime_on = Ctrl+変換`、`keys.ime_off = Ctrl+無変換`(既定、awaseがactuate)。単独のキーは、beliefが既に目標と同じだと書き込みを
省略する(`shadow-toggle no-op`)ため、ずれている最中は効かないことがある。**2連続なら構造的に必ず揃う**: 「Ctrl+無変換→Ctrl+変換」は
1打目のあとbeliefが必ずOFFになり、2打目(ON)は省略されず書き込まれる(逆順は「ON→OFF」で確定)。ずれ4パターン(belief/実IME = ON/OFF、
OFF/ON、ON/ON、OFF/OFF)のどれでも終わりはON。
- 手順: ずれを起こす(無変換/変換)→リセット(Ctrl先押し200ms、1打目と2打目の間隔を40/100/300msで振る)→実IMEとEngineが揃うか。
- follow無効化(`a7-no-follow`)で意図的にずれを起こした構成(各回4件のずれ)でも、**リセット16/16手順が成功**(間隔40/100/300ms、各3回)。
- 限界: IMMで読めるアプリ(Win32 Edit)のみ検証。読めないアプリ(Chrome/TsfNative)とMS-IME本体は未検証(書き込みの成否を確認できない)。
