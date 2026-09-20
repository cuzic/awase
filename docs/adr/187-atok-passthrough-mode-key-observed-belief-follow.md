---
id: ADR-187
title: |-
  GJI(ATOK)で無変換/変換をパススルーする設定(`gji_thumb_key_ime_toggle=false`)のとき、生キー通過後に実IMEを読み直し、
  開閉が変わっていればそれをユーザーの明示意図として記録してEngineを追随させる
summary: |-
  CI実機E2E(ADR-186、`atok-passthrough`/`atok-passthrough-henkan`、各3/3)で、ATOKプリセット+パススルー(opt-in無し)では、
  実IMEはGJIが正しく開閉する(生の無変換/変換が届く)のに**Engineが追随しない**ことを確認した(IME OFFでもEngine ONのまま=
  直接入力にNICOLA変換が効く)。原因は3層: (1)生キー通過後に実IMEを読み直す契機が無い(`may_change_ime`は無変換/変換を
  意図的に含まず、親指キーはKeyDownをFSMがConsumeするため通過後の20ms再読み取りも発火しない)。(2)読み直しだけでは足りない
  可能性が高い: 実IMEがOFFなのに`desired_open`と明示ON意図(IntentStore、10秒)が残ると、ドリフト補正がIMEをONへ戻しに行く
  (未検証、下記の実験1で確認する)。(3)無変換/変換のToggleは非冪等で、「!belief」の予測はComposition中(半角英数トグルで開閉
  不変)などで外れる(BUG-115が既定opt-in無しにした理由1)。推奨は**観測型の追随**: 通過後に実IMEを読み、開閉が変化していれば
  その観測値を`PhysicalImeKey`由来の明示意図として既存の単一書き込み口(`write_physical_key`相当)で記録する。予測せず、
  actuateせず、新しい型・フィールドを増やさない。
status: |-
  **ドラフトv1(未実装、opusレビュー前)**。実機/CIでの検証は実験1〜3(下記)。設計はopus-adversarial-consultで収束させてから実装する。
related_adr:
  - "ADR-090"
  - "ADR-115"
  - "ADR-179"
  - "ADR-186"
---

# ADR-187: ATOK+パススルーでの無変換/変換に対するEngine追随(観測型)

## 背景

ユーザー要件(ADR-186): **かな=Engine ON、英数(半角英数・直接入力)=Engine OFF、押下直後から**。

ADR-186は、`gji_thumb_key_ime_toggle=true`(opt-in)なら無変換/変換の開閉トグルにEngineが押下時点で追随することを
実機とCIで確認した。一方、**opt-in無し(既定、パススルー)のATOKユーザーは未解決**として残した。CI(run 35486929410)の
`--walk`(ひらがな/無変換/変換の固定12押下)で、`atok-passthrough`と`atok-passthrough-henkan`は各3/3が次の型で失敗した:

| 押下 | 実IME(+1500ms) | Engine |
|---|---|---|
| 無変換(かなON→) | open=0(GJIが閉じた) | **ON のまま**(期待OFF) |
| 変換(かなON→) | open=0 | **ON のまま** |
| 無変換/変換(OFF→) | open=1 | ON(実IMEと一致) |

実IMEは正しい(生キーがGJIに届き、GJIがATOKのキーマップどおり開閉する)。Engineだけが追随しない。ONのまま直接入力になると
NICOLA変換が効き続け、直接入力にかなが出る。awase起動中のこの状態はユーザーの要件を直接破る。

ログ(CI run 35486929410 `result-atok-passthrough-1`、無変換): 物理KeyDownは`PendingThumb`でConsume、100ms後のタイマーと
KeyUpで単独タップ確定→`send_keys: Key(0x1D)`で**生キーをそのまま送出**→その後4秒間、`IME snapshot`/`stage-observe`/
`notify-refresh`/`drift`/`Engine (de)activated`のいずれも出ない。awaseは何も観測していない。

## 原因(3層)

1. **通過後に読み直す契機が無い。** 物理IMEキーの通過後20ms再読み取り(`key_pipeline.rs`、`!decision.is_consumed() &&
   ime_relevance.may_change_ime && KeyDown`)は、(a)`may_change_ime`が無変換/変換を意図的に含まず(`vk.rs`のテストが
   「第3の軸」として固定)、(b)親指キーのKeyDownはFSMがConsumeするため、二重に発火しない。生キーは単独タップ確定時
   (KeyUp/タイマー)に別経路で送出される。
2. **読み直すだけでは足りない(仮説)。** 実IMEがOFFになった後に観測が入ると、`desired_open`は直前の明示ON(ひらがな等、
   IntentStoreに10秒)のままなので`desired≠observed`となり、`check_drift_correction`は強い意図が一致する場合しきい値0で
   即時補正へ進む。warrantのStep 1(IntentStore)が「ONの意図あり」と判定すると、awase自身がIMEをONへ戻す。
   ADR-186決定2の根本原因(明示意図が記録されないとwarrantが`Unwarranted`で外れる)の鏡像。
3. **Toggleは非冪等。** 「!belief」の予測(`FollowOnly(Toggle)`、`ModeKeyActuationOwner::PhysicalDelivery`のToggle版)は、
   ATOKの状態依存(DirectInput→IMEOn、Precomposition→CancelAndIMEOff、**Composition→半角英数トグルで開閉不変**)で外れる。
   beliefが外れるとドリフト補正が実IMEと逆へ書く(BUG-113系)。ADR-179が`FollowOnly`をTurnOn/TurnOffに限りToggleを
   常に`Explicit`にした理由、BUG-115が既定opt-in無しにした理由1と同じ。

## 選択肢

- **A. 通過後の再読み取りだけ足す**(`schedule_ime_refresh(20)`)。最小だが原因2で、実IMEを再びONへ戻す恐れがある
  (実験1で確認)。単独では採らない。
- **B. Toggleを予測してbeliefを書く**(`!belief`)。原因3で却下。ADR-179/BUG-115の設計判断と衝突する。
- **C. opt-inを既定`true`にする**(awaseがactuate)。BUG-115の理由(非冪等、露出2倍、全ATOKユーザーへの自動適用、GJIフォーク)が
  そのまま残る。ユーザーが選んだ「パススルー」設定を尊重しない。
- **D(推奨). 観測型の追随**: 通過した無変換/変換の**直後に実IMEを読み、開閉が変わっていれば、その観測値を明示意図として記録する**。
  予測しない・actuateしない・生キーはGJIにそのまま届く。

## 決定(ドラフト)

**決定1 — 生キー通過後の再読み取りを、単独タップ確定で通過した無変換/変換に対して発火させる。**
`is_ime_mode_key_for_ime`(既存の第3の軸、`vk.rs`)に該当する生キーを、FSMの単独タップPassthroughとして**実際に送出した**直後に、
既存の`schedule_ime_refresh`(20ms)を呼ぶ。`may_change_ime`は広げない(`vk.rs`の「widenして代用してはならない」を守る)。
チョード(親指+文字)・Suppress・delegate経路は対象外(送出が無い/別経路で意図が記録される)。

**決定2 — 再読み取りの結果、開閉が通過前から変わっていれば、観測値を`PhysicalImeKey`由来の明示意図として記録する。**
書き込みは既存の単一口(`write_physical_key`、`IntentWitness::from_physical`が要る)を使い、`reduce()`以外でbeliefを書かない
(`.claude/rules/ime-belief-architecture.md`)。対象は「通過前のbelief.open ≠ 観測open」のときだけ。Composition中の無変換
(半角英数トグル、開閉不変)は差が出ないので記録されない=誤って意図を作らない。窓は短く(通過から数百ms)、他要因による
開閉変化を巻き込まないよう、通過をマークした押下に紐づける。

**決定3 — 新しい型・フィールド・variantは足さない(目標)。** 通過マーカーは既存の`ime_relevance`/`ModeKeyActuationOwner`の
再利用、または通過した押下のwitnessと通過前beliefを1つ持つ最小のローカル状態で表す。予測(`FollowOnly(Toggle)`)は追加しない。

## 決定しないこと(意図的)

- TsfNative/Imm32Unavailable(メモ帳・Windows Terminal・Chrome/Edge)。IMMで開閉を読めない(`ime_on=None`)ため、決定1の
  再読み取りは効かず、`idle-conv-check`(次の打鍵後、TsfNative限定)が担う現状のまま。未検証。
- MS-IMEキーマップ/MS-IME本体。無変換/変換は開閉を変えないので、再読み取りは変化なしで終わる(害は追加の読み取りのみ)。
  IME種別で分岐する新しい軸は足さない。
- 半角/全角(0xF3/0xF4)のモデル誤り(ADR-186決定5)。

## 検証計画(実験)

CIの`--walk`+`check_consistency.py`(既存)をそのまま使う。`atok-passthrough`/`-henkan`の期待を`fail`→`pass`へ変える。

1. **実験1(原因2の確認)**: 決定1だけ入れた仮ビルド(記録なし)で、`atok-passthrough`の各押下+1500msで実IMEが**ONへ戻される**か
   (ドリフト補正/warrantの挙動)を見る。戻れば決定2が必須と確定、戻らなければ決定2は簡素化できる。
2. **実験2**: 決定1+2で`atok-passthrough(-henkan)`が3/3追随、かつ`atok-optin`/`baseline`(opt-in)が退行しないこと。
3. **実験3(押下保持の影響)**: `--hold`を80/180/400msで振り、再読み取り(20ms)が間に合うかを見る(通過は単独タップ確定後で、
   ひらがなキーの30〜70msより遅れる可能性)。間に合わなければ再読み取りを2段(20ms+100ms)にするが、根拠(実測ms)を
   コミットに残す(`tuning-constants.md`)。

回帰テスト: 純関数(通過前belief・観測open→記録するか)の単体テスト。`platform_state`側の記録テストはWindows専用。

## リスク

- 開閉が変わる別要因(ユーザーが同時にマウスでIMEを切り替える等)を意図と誤記録する。窓を通過直後に限り、対象を「変化した」
  場合だけにして影響を局所化する。誤記録の最悪は「観測値どおりの意図」(実IMEと一致)で、ドリフト補正が実IMEと逆へ書くことはない。
- 単独タップ確定がタイマー(100ms)経路のとき、物理イベントが無くwitnessが取れない(ADR-186決定2bと同型)。
  KeyUp確定へ揃える(`defers_solo_until_release`をPassthrough設定の親指にも広げる)か、KeyDownのwitnessを持ち回るかを決める
  (前者は全パススルー利用者のタップ確定タイミングを変えるため影響が大きい)。**最大の未決事項**。
- 追加の読み取り(クロスプロセスIMM、`run_with_timeout`)が、孤立した無変換/変換タップごとに1回増える。
- ADR-186のE1〜E7bと同様、必須/不要の判断は実機/CIのN回で行い、少数回の結果で決めない。

## 未決事項(opusレビューで確認したい)

1. 原因2(ドリフト補正が実IMEをONへ戻す)は本当に起きるか。起きないなら決定2は不要にならないか。
2. witnessの取り方(KeyUp確定へ寄せる/KeyDownから持ち回る)。
3. 決定1の発火点は、実際の生キー送出の直後(executor)か、FSMの解決結果(engine→platformの通知)か。ADR-019(コアはVKを分岐しない)との整合。
4. 「開閉が変わったときだけ意図を記録する」で、Composition中の無変換(半角英数トグル)と、GJIが変換を受けてもIMEを閉じないケースを漏れなく除けるか。
