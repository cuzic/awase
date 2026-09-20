---
id: ADR-187
title: |-
  ATOKの無変換/変換(Toggle)はawaseが唯一の変更主体としてactuateする(`gji_thumb_key_ime_toggle`を既定`true`へ)。
  方向が固定のOn/Offはパススルーしてbeliefだけ追随する。「パススルー+観測で追随」する案は見送る
summary: |-
  CI実機E2E(ADR-186、`atok-passthrough`各3/3)で、ATOK+パススルー(opt-in無し)は実IMEが開閉するのにEngineが追随しない
  (IME OFFでもEngine ON)ことを確認した。追随機構を新設する観測型(生キー通過後に読み直して意図を無効化/記録)をopusレビュー2ラウンドで
  検討した結果、(2-a)`IntentStore`と(P1)`ImeModel::last_intent`の二重の固定を外すには新しい`ImeEvent` variantが要り、さらに
  通過マーク・`ActivationSync`のGJI warmupバースト抑止・ポーリング再開の副作用検証が要ると分かり、ATOKパススルー利用者だけのための
  機構としては重すぎるため見送った。ユーザー方針: **Toggle(非冪等)ならawaseがactuateする、方向が固定ならfollowするだけ**。
  Toggleのactuateは既存のopt-in経路(`gji_thumb_key_ime_toggle=true`、ADR-186でKeyUp解決+warrantにより実機/CI検証済み)そのもの
  なので、**既定を`false`から`true`へ変える**だけで足りる(新しい機構なし)。方向固定のOn/Offは元から冪等でopt-in不要、
  パススルー+`FollowOnly`(ADR-179)で追随済み。明示`false`はopt-outとして残る。
status: |-
  **決定(ユーザー方針)・実装済み(未マージ)・CI検証済み。ただしfollow方式のスパイクが成功したため、この決定は撤回候補(下記「follow方式のスパイク結果」)**。`gji_thumb_key_ime_toggle`の既定を`true`へ。`atok-default`/`atok-default-henkan`
  (キーを書かない構成)で各3/3追随。観測型(下記の分析)は見送りの記録として残す。
related_adr:
  - "ADR-090"
  - "ADR-115"
  - "ADR-179"
  - "ADR-186"
---

# ADR-187: ATOKの無変換/変換(Toggle)は、awaseが唯一の変更主体としてactuateする

## 決定

ユーザー方針(2026-09-20): **「toggleであればawaseがactuateする。ime on/offの方向が一定ならfollowするだけで追随する」**。

| 割当て | 冪等性 | awaseの役割 | 実装 |
|---|---|---|---|
| **Toggle**(ATOKの無変換/変換、状態依存) | 非冪等 | **awaseが唯一の変更主体としてactuate**。生キーはGJIに渡さない | `gji_thumb_key_ime_toggle`(既定`true`)→`delegate_to_open_axis`(親指キー)/`ime_toggle`リスト。`ModeKeyActuationOwner::AwaseExplicit` |
| **On/Off**(方向固定、MS-IMEキーマップのひらがな等) | 冪等 | **生キーをパススルー**し、beliefだけ追随(actuateしない) | 既存(opt-in不要、ADR-179の`FollowOnly`/`PhysicalDelivery`) |

変更は1点: **`GeneralConfig::gji_thumb_key_ime_toggle`の既定を`false`→`true`**(`src/config.rs`、serdeの欠落時とDefaultで同値)。
明示`false`はopt-outとして尊重され、その場合は従来どおり(生キーがGJIに届き実IMEは開閉するがEngineは追随しない、警告ログ)。
BUG-115が既定`false`にした理由(非冪等・露出2倍・全ATOKユーザーへの自動適用・GJIフォーク)は、ADR-186でKeyUp解決と
warrantにより非冪等の誤発火が実機/CIで検証され、残りは受容する(configのdocに5点(Shift+無変換の横取りを追加)として残した)。

## 検証(CI run 35490899254、各3回)

| 構成 | 結果 |
|---|---|
| `baseline`(明示`true`、ATOK用期待表) | 3/3 PASS |
| **`atok-default`**(config.tomlにキーを書かない、`--walk`) | **3/3 追随**(無変換/変換で実IMEが開閉しEngineも追随) |
| **`atok-default-henkan`**(変換キー) | **3/3 追随** |
| `msime-default`(GJIのMS-IMEキーマップ、既定) | 3/3 追随(退行なし) |
| `atok-passthrough`(明示`false`=opt-out) | 3/3 不追随(既知の制約として期待`fail`で固定) |
| `msime-native-default`(Microsoft IME本体、既定) | 3/3 不追随。**本変更とは無関係の既存の問題**(step1〜3で実IMEがOFFのままEngineだけON、`ImmCross`のON書き込みが148msで失敗、ADR-186「CIでの再現」参照。既定`true`でも`false`でも同じ) |

単体: `config::tests::gji_thumb_key_ime_toggle_defaults_to_true_and_respects_explicit_false`。

## follow方式のスパイク結果(2026-09-20、`spike/adr187-follow-observe`、CI各3回)

ユーザー指摘(「変換前か変換中かは原理的に分からない」= awaseの`composing`はawase自身の出力履歴からの推定でしかない)により、
Toggleをawaseがactuateする方式(上の決定)はこの推定が外れると未確定文字を捨てるリスクがあると分かり、follow方式を実機(CI)で試した。

**スパイクの実装**(新しい機構は最小、スパイク品質でlint/guardは未更新): 無変換/変換の生キーをGJIへ通過させた点で、
(1)通過マーク(静的なタイムスタンプ、typing-idleガードのバイパス用)+(2)20ms後のIME再読み取りを予約し、
(3)**観測が成功した直後に**対象hwndの`IntentStore`と`last_intent`(新`ImeEvent::ModeKeyPassedThrough`)を捨てる。
通過点は2つ必要: FSM経由の`SendKeys`(Engine ON時の単独タップ確定)と、Engine OFF時に無変換/変換がFSMを通らず`PassThrough`
判定でOSへ渡る経路(`key_pipeline`)。awaseはactuateしない。

| 版 | 内容 | 結果 |
|---|---|---|
| v1 | `SendKeys`点のみ、押下時に意図を捨てる | 11/12(OFF→ONの手順が追随しない。Engine OFF時はFSMを通らないため) |
| v2 | `PassThrough`経路にも配線 | ATOKパススルー12/12だが、**opt-in(baseline)が退行**(下記) |
| v3 | 意図を捨てるのを観測の直後へ移す | 同じ(原因は別) |
| **v4** | フックを`shadow_action`が無い(=awaseがIMEキーとして扱わない)PassThroughだけに限定 | **全構成OK** |

**v4のCI結果**: `atok-passthrough`/`atok-passthrough-henkan`(明示`false`=パススルー)**各3/3で全12手順が追随**(以前は各3/3不追随)。
`atok-passthrough-cold`/`-henkan-cold`(先頭のひらがなを除き、明示意図が無い状態でいきなり無変換/変換)も**各3/3追随**
(round2のM2「typing-idleガードで観測が空振りする」を通過マークで解消)。`baseline`/`baseline-henkan`(opt-in、ATOK用strict期待表)、
`atok-optin`、`msime`、`msime-optin`、`msime-stale-table`は退行なし(各3/3)。

**v2で分かった落とし穴**: followが正しくOFFを追随すると、opt-in(Toggle、`shadow_action`あり)の「直接入力→無変換でON」で、
`UserImeOnEisuReset`(「かなに戻る」と仮定してEngineを即ON)+`ActivationSync`のSetOpen(true)が走り、Engineが約70ms一瞬ONになる
(convが半角英数のまま保存されているため、観測で訂正されOFFに戻る)。以前は、belief自体が古いONのままでこの経路に入らなかった。
ADR-186のE3でeisu reset抑止を「不要=デッドコード」として削除した前提(beliefが正しくOFFなら入る経路)が崩れる場面。
→ followは「awaseが既にIMEキーとして扱う(opt-in)キーには配線しない」ことで分離した。

**副作用の計測(ATOKパススルー、12押下)**: ActivationSync SetOpenは3件(opt-inは5件)、VK_IME_OFF→ONのwarmupバーストは2件(opt-inと同じ、
ひらがな起因)、無変換/変換の押下に連動したバーストは見えない。周期ポーリング(OsPoll)は`last_intent`を捨てることで再開する(58回、opt-inは35回)。

**未対応(本実装で要る)**: 新`ImeEvent`のlint(`ime_event_guard`)/`architecture_guard`の更新、通過マークを静的変数でなく`GateStore`の
`ScopedOneShot`に、`ActivationSync`のバースト対策(観測由来の活性化でGJI warmupが走る経路。今回のCIでは無変換/変換連動では見えなかった)、
単体テスト、TsfNative/Imm32Unavailable(Chrome/Edge/メモ帳/Windows Terminal)は`ime_on=None`で読めず未検証(従来どおりidle-conv-check頼み)、
Microsoft IME本体は別の既存の問題(ADR-186)。**この結果を受け、上の「決定」(Toggleをawaseがactuate、既定`true`)は撤回候補**(PR #227)。

## 見送った案: パススルーのまま観測で追随する(観測型)

以下は、生キーをGJIに渡したまま(awaseはactuateしない)Engineを追随させる案の分析と、opusレビュー2ラウンドの結果。
**実装しない**ことにした根拠として残す(同じ案を再検討する前に読むこと)。

## 背景

ユーザー要件(ADR-186): **かな=Engine ON、英数(半角英数・直接入力)=Engine OFF、押下直後から**。

ADR-186は、`gji_thumb_key_ime_toggle=true`(opt-in)なら無変換/変換の開閉トグルにEngineが押下時点で追随することを実機とCIで
確認した。**opt-in無し(既定、パススルー)のATOKユーザーは未解決**として残した。CI(run 35486929410)の`--walk`
(ひらがな/無変換/変換の固定12押下)で、`atok-passthrough`と`atok-passthrough-henkan`は各3/3が次の型で失敗した:

| 押下 | 実IME(+1500ms) | Engine |
|---|---|---|
| 無変換(かなON→) | open=0(GJIが閉じた) | **ON のまま**(期待OFF) |
| 変換(かなON→) | open=0 | **ON のまま** |
| 無変換/変換(OFF→) | open=1 | ON(実IMEと一致) |

実IMEは正しい(生キーがGJIに届く)。Engineだけが追随せず、ONのまま直接入力になるとNICOLA変換が効き続ける。

ログ(`result-atok-passthrough-1`、無変換): KeyDownは`PendingThumb`でConsume→**KeyUpで**単独タップ確定(タイマーではない。
パススルー設定の親指は既に`defers_solo_until_release`の対象)→`send_keys: Key(0x1D)`で生キー送出→その後4秒間、`IME snapshot`/
`stage-observe`/`drift`/`Engine (de)activated`のいずれも出ない。awaseは何も観測していない。

## 原因(4層)

1. **通過後に読み直す契機が無い。** 物理IMEキーの通過後20ms再読み取り(`key_pipeline.rs`、`!decision.is_consumed() &&
   may_change_ime && KeyDown`)は、`may_change_ime`が無変換/変換を意図的に含まず(`vk.rs`の「第3の軸」テスト)、親指キーの
   KeyDownはFSMがConsumeするため発火しない。生キーはKeyUp確定時に別経路(`execute_one`→`output::send_keys`)で出る。
2. **(2-a)直前の明示意図がEngineの見る値を固定する(支配的)。** `effective_open_at`(`platform_state.rs:674`)は
   `IntentStore::resolve_effective_open`(`intent_store.rs:158`)を通り、対象hwndに意図が**有れば観測を無視してその値を返す**。
   ひらがなキー(`shadow-toggle`→`write_physical_key`→`record_explicit_intent`)は10秒のON意図をhwndに残す
   (`EXPLICIT_ON_INTENT_TTL_MS`)。この間、実IMEがOFFになって観測が入っても`ctx.ime_on`は動かない。**読み直しだけでは
   Engineは原理的に追随しない**。OFF意図は30秒(`EXPLICIT_OFF_INTENT_TTL_MS`)。
   **(2-b)ドリフト補正。** `desired_open`(ひらがなで`true`)と明示意図が一致する場合、`check_drift_correction`はしきい値0で
   即補正に進み、warrant Step 1(意図)が出て、awase自身がIMEをONへ戻す。起動直後のCIログ98行目に同経路の実例がある。
3. **Toggleは非冪等。** 「!belief」の予測は、ATOKの状態依存(DirectInput→IMEOn、Precomposition→CancelAndIMEOff、
   **Composition→半角英数トグルで開閉不変**)で外れる。ADR-179が`FollowOnly`をTurnOn/TurnOffに限った理由、
   BUG-115の理由1と同じ。
4. **観測を意図として書く案は、`from_physical`が受理しない。** `IntentWitness::from_physical`(`evidence.rs:369`)は
   `shadow_action`が有る物理キーだけを受理し、0x1C/0x1D(変換/無変換)は`ImeKeyKind::from_vk`に含まれない。ATOKパススルーでは
   自動検出の`shadow_action`もopt-in無しで`None`(`gate_thumb_key_ime_actions`)。よって`write_physical_key`は黙って空振りする
   (`evidence.rs:356`のdocが記録する2026-09-08の実機回帰と同型)。

## 選択肢

- **A. 通過後の再読み取りだけ足す。** 原因2-aで効かない。却下。
- **B. Toggleを予測してbeliefを書く**(`FollowOnly(Toggle)`)。原因3で却下(ADR-179/BUG-115)。`FollowOnly`は再利用しない。
- **C. opt-inを既定`true`にする**(awaseがactuate)。BUG-115の理由がそのまま残り、ユーザーが選んだパススルーを尊重しない。
- **D. 観測値を明示意図として記録する**(v1の決定2)。原因4の`from_physical`拡張+`architecture_guard`2本の更新が要り、
  OFF意図が30秒固着し(M3)、Medium観測が明示意図としてHigh観測より強い権限を持つ(M5、ime-belief-architectureが禁じる
  「観測をユーザー意図に偽装する」に構造的に近い)、`ActivationSync`のechoで二重actuationにもなる(M4)。
- **E(推奨). 古い意図を無効化する。** 「ユーザーがIME操作をした。結果は分からないので、古い意図を根拠にせず観測に委ねる」。
  値を書かない。witness不要、TTL固着なし、権限昇格なし、warrant Step 1が外れてStep 3(観測)が根拠になる。

## 決定(ドラフトv2)

**決定1 — 通過した無変換/変換の実送出点で、再読み取りを発火させる。**
`execute_one`→`output::send_keys`が生キー(`is_ime_mode_key_for_ime`、`vk.rs`の既存の第3の軸)を実際に送出した直後(プラット
フォーム層、ADR-019に抵触しない。engine→platformの通知・新variantは不要)。`may_change_ime`は広げない。チョード・Suppress・
delegate経路は送出が無い/別経路のため対象外。`FollowOnly`は拡張しない(選択肢B)。

**決定2 — 同じ点で、対象hwndの`IntentStore`エントリを消す**(`IntentStore::remove`、`intent_store.rs:215`、既存API)。
以後`effective_open()`は観測の導出結果になり、warrant Step 1が外れて、ドリフト補正が古いON意図でIMEを戻すことも止まる。
消す対象は通過をマークした押下のhwndだけ(`current_focus`)。値は書かない。

**決定3 — 再読み取りはtyping-idleガードをバイパスする。** `ir_decide_read_strategy`(`ime_refresh.rs:294`)は、20ms後は
必ず`idle<500ms`なので、`explicit_verify`(`explicit_intent().is_some() && applied!=Unknown`)が偽だと`SkipTyping`で
**観測しない**。CIのwalkは先にひらがなを押すため偶然通るが、フォーカス直後にいきなり無変換を押す通常の使い方では空振りする。
通過マークを第2のバイパス条件にする(有効期間は短い窓で1回だけ消費、`FocusChanged`でクリア)。

**決定4 — 新しい型・フィールド・variantは足さない(目標)。** 追加は「通過マーク1個(窓・一回消費)」と、決定1〜3の既存API呼び出しだけ。

**受け入れ基準(適用範囲)。** IMMのクロスプロセス読み取りが効くアプリ(`profile=ImmCross`/Win32 Edit系)でのみ要件を満たす。
TsfNative/Imm32Unavailable(メモ帳・Windows Terminal・Chrome/Edge)は`ime_on=None`で読めず、`idle-conv-check`(次の打鍵後、
TsfNative限定)が担う現状のまま(未検証)。将来の候補として`[gji-io] WRITE`(GJIが打鍵に反応した独立証拠、方向は不明)を残す。

## round2の結果(v2の決定2・4は成立しない)

- **B4(Blocker)**: 明示意図の固定は**2重**。P1=`ImeModel::resolve_open_at`(`ime_model.rs:390`)が`last_intent.is_some()`(TTL無し、
  `FocusChanged`でのみクリア)だけで`desired_open`を観測より優先する。P2=`IntentStore`(v2が消す対象)。**P2だけ消してもEngineは
  追随しない**。`last_intent`は`reduce()`経由でしか書けないため、消すには**新しい`ImeEvent` variantが1つ要る**(決定4「新variantを
  足さない」と衝突)。前例: `PanicReset`/`HwndCacheRestored`は`last_intent`を設定しない`desired_open`直接書き込みの隔離された例外で、
  `apply_hwnd_cache_restore`は「desired_open書き換え+IntentStore無効化」を既にproductionで行っている。3つ目の系列として足す形になる。
  更新が要るもの: `lints/ime_event_guard`の`RESTRICTED_VARIANTS`、`tests/architecture_guard.rs`の構築箇所数ガード、
  ime-belief-architectureの「3つ目のescape hatch」の正当化。
- **M8**: `last_intent`を消す副作用(消費者4箇所): `explicit_verify`が偽になる(決定3の通過マークが必須になる)/
  `reschedule_ime_refresh`の停止が解除され**500ms周期のIMMポーリングが全プロファイルで再開**/ドリフト補正のしきい値0→400ms/
  `force_guards.resolve`のヒューリスティックguardがoverrideできるようになる。
- **M9(ActivationSyncは確実に起きる)**: 観測でEngineが活性化すると`check_active_transition`が必ず`SetOpen{ActivationSync}`を
  発行する(抑止は`Inactive(NotRomajiInput)`のみ)。既存の`strip_...`のproduction呼び出し元は`key_pipeline.rs:458`の1箇所で、
  この経路(`execute_from_loop`)を通らず、条件も両方偽。warrantも止めない(観測から導いた値なのでStep 3と一致)。実体は
  「VK_IME_ONの二重送信」ではなく**IMM write+完了通知から走るGJI warmupバースト(VK_IME_OFF→VK_IME_ON)**で、開けたばかりの
  GJIにVK_IME_OFFを送る形になりBUG-113系に触れる。**新しい呼び出し点と条件が要る**(ADR-119型の合流点追加)。
- **M10**: 観測源は`ObserverPoll`のMedium(basisは`SingleIndirect`、Highではない)。`derive_actuating`が空のとき、
  `FeedbackPolicy::Blind`(TsfNative/Imm32Unavailable)ではStep 4c `OwnSsot(desired_open)`でwarrantが出て、awaseがIMEをONへ戻す。
  適用範囲(ImmCross=Read)ではStep 4cは発火しない(テストで固定済み)が、範囲外では戻りうる。
- **S6(通過マークの最小配線)**: `GateStore`に`ScopedOneShot<ForegroundScope, PassMark>`を1フィールド(`post_bypass`と同型、`peek`が
  スコープ失効を自動処理するので`FocusChanged`配線は不要)。`ir_decide_read_strategy`/`ir_stage_strategy`を`&mut self`に。
- S8: `last_intent`を消すとbelief側は`derive_any`(Medium単独合意も採用)で決まる。ADR-087は「belief側は許容、actuation側だけ禁じる」と
  明文化している(`open_warrant.rs`のmodule doc)。

**結論**: 観測型(選択肢E)を実装するなら、新`ImeEvent` variant 1個+`PassMark`1個+`ActivationSync`の新しい抑止点(合流点追加)+
ポーリング再開の副作用検証、が要る。当初の「通過マーク+既存API呼び出し」より大きく、ATOKパススルーの利用者だけのための機構としては重い。

## 方針の分岐(ユーザー判断)

- **F1. 観測型(選択肢E)を実装する。** 上記の追加物と、CIでの実験(ポーリング再開・warmupバースト・フォーカス直後)が要る。
  パススルー設定は保たれ、awaseはactuateしない。
- **F2. ATOKプリセットのopt-inを既定にする**(選択肢C)。新しい機構は不要で、CI(`baseline`/`atok-optin`、3/3)で追随が確認済み。
  BUG-115が既定opt-in無しにした理由1〜4のうち、1(非冪等)はKeyUp解決+warrantで実機/CI検証済み(ADR-186)、2(露出2倍)・
  3(全ATOKユーザーへの自動適用)・4(GJIフォーク)は残る。ユーザーが明示的に選んだパススルー設定は変わる(awaseがactuateする)。
- **F3. 現状維持+文書化。** ATOK+パススルーは追随しない既知の制約として、設定画面/ドキュメントで案内し、opt-inを推奨する。

## 決定しないこと(意図的)

- MS-IMEキーマップ/MS-IME本体。無変換/変換は開閉を変えないので、再読み取りは変化なしで終わる(害は追加の読み取りとIntent無効化のみ。
  MS-IME側でIntentを消して困る場面が無いかを実験3で確認する)。IME種別で分岐する軸は足さない。
- 半角/全角(0xF3/0xF4)のモデル誤り(ADR-186決定5)。

## 未解決(実験で確定)

- **`desired_open`が古いまま残る**(決定2の弱点、レビューS4)。決定2はIntentStoreだけを消し、`shadow_model.desired_open`と
  `last_intent`は触らない。`desired_open`(true)と観測(false)の乖離はドリフト補正へ進むが、`last_intent`を残す限りしきい値は
  0で、warrantはStep 1が外れStep 3(High観測=OFF)と要求ON不一致で`None`(Unwarranted、A-2強制済み、`c8bc1adc`)となり
  書き込まれない、というのが期待。これが成り立たず補正が実IMEをONへ戻す、またはEngineが追随しない場合に限り、決定2(観測を
  意図として記録、選択肢D)へ戻る。その場合はM1〜M6(`from_physical`拡張、TTL短縮、観測ソースallowlist=High
  (`ImmGetOpenStatus`/`ImmCrossProbe`)のみ、`ActivationSync`のstrip=`strip_activation_sync_set_open_for_physical_delivery`再利用、
  `record_explicit_intent`の呼び出し元件数ガード更新、`current_focus`がNoneのときの空振り)を全て設計に含める。
- `ActivationSync`のechoが、観測由来のEngine活性化(実IME OFF→ONの回)でVK_IME_ONを二重に送らないか(ひらがな押下では
  `[ime-io] actuation SendInput kind=kanji_marker vk=[1A,16]`が実際に出ている)。出るなら`strip_...`を再利用する。

## 検証計画(実験)

CIの`--walk`+`check_consistency.py`を使う。`atok-passthrough`/`-henkan`の期待を`fail`→`pass`へ変える。判定はログ証拠3点で見る
(実IMEがONへ戻るかだけでは、Engineが追随しない場合も「戻らなかった」となり判定できない)。

1. **実験1(決定1のみ)**: 再読み取りだけ入れた仮ビルドで、`[stage-observe] strategy=OsPoll`が20ms後に出るか(`SkipTyping`でないか)、
   `ObserverReported`が記録されるか、`[notify-refresh] ctx.ime_on`が実IMEと一致するか(IntentStoreのpinを受けていないか)。
   2-aにより一致しないと予測する(予測が外れたら原因2-aの前提を見直す)。
2. **実験2(決定1+2+3)**: `atok-passthrough(-henkan)`が3/3追随。`atok-optin`/`baseline`(opt-in)が退行しない。
   上記の`desired_open`の扱い(補正が実IMEを戻さないか)と`ActivationSync`のechoを、ログ(`[drift]`、`origin=ActivationSync`)で確認。
3. **実験3(MS-IMEへの影響)**: MS-IMEキーマップ3構成・MS-IME本体で、退行しないこと。
4. **実験4(フォーカス直後)**: `--walk`の先頭をひらがなでなく無変換にした構成(`--walk-cold`相当、意図が無い状態)を追加し、
   決定3のバイパスが効くこと。`--hold`を80/180/400msで振り、20msの再読み取りが間に合うかも見る(間に合わなければ2段にするが、
   実測msをコミットに残す、`tuning-constants.md`)。

回帰テスト: 純関数(通過マークの窓・一回消費・FocusChangedでのクリア、`ir_decide_read_strategy`のバイパス条件)の単体テスト。
`platform_state`側のIntentStore無効化テストはWindows専用(`windows-build` CI)。

## リスク

- 通過マークの窓が長いと、無関係な観測をバイパスさせる。窓は短く、1回で消費する。
- IntentStoreを消すと、直前の明示ON意図に守られていた「観測が誤って揺れる」場面(BUG-63型、ConvOpenInference)で、
  Engineが誤OFFになりうる。無効化するのは無変換/変換の生キー通過時だけで、対象はIMMで開閉を読めるアプリに限る。
- 追加の読み取り(クロスプロセスIMM、`run_with_timeout`)が、孤立した無変換/変換タップごとに1回増える。
- ADR-186のE1〜E7bと同様、必須/不要はCIのN回で判断し、少数回で決めない。

## 未決事項(opus round2で確認したい)

1. 決定2(IntentStoreのみ無効化)で、`desired_open`/`last_intent`が残ってもドリフト補正が実IMEを戻さない、というwarrant Step 3の読みは正しいか。
2. 決定3の通過マークを、`ir_decide_read_strategy`にどう1条件で渡すか(既存の`explicit_verify`の隣)。
3. 通過マークの窓・消費・`FocusChanged`クリアの置き場所(既存の状態に載せられるか)。
