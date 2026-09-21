---
id: ADR-181
title: |-
  GJI(ATOKキーマッププリセット)がVK_DBE_HIRAGANAを自己注入マーカー無しで
  周期的に送ってくるため、IME OFF直後にawaseが誤って再actuateしIME ONへ
  戻ってしまう不具合の設計
summary: |-
  ADR-179の無変換/変換Passthrough実験（`muhenkan_solo_tap_always_suppress
  = false`）の実機A/B中に発見。GJI(Google日本語入力、キーマップ設定を
  ATOKプリセットにしている)がVK_DBE_HIRAGANA(0xF2)を`injected=false`
  (LLKHF_INJECTEDなし・awase自身のself-injectedマーカーも無し)で不規則
  (0.4秒〜28秒間隔)に送ってくる。`kp_stage_shadow_ime_toggle`
  (key_pipeline.rs:1485-1495)はGJI/MS-IME自動検出由来の`shadow_action`
  (hook.rs::classify_ime_relevanceがVKコードのみから計算)を`event.injected`
  を一度もチェックせずに「PhysicalImeKey」ユーザー意図として扱う。
  ひらがな/カタカナモードキーはADR-179が意図的に対象外とした
  (delegate_to_open_axisが常に明示actuateする、Suppress/Passthroughの
  概念が無い)ため、この外部由来の疑似TurnOn意図がIME OFF直後に確実に
  VK_IME_ONを再送し、IMEをONへ戻してしまう。既定のSuppress設定では
  無変換/変換単独タップがawaseに握りつぶされIMEが数秒間OFFのまま持続する
  シナリオがそもそも作られないため、この現象は今回のPassthrough実験まで
  一度も可視化されなかったと考えられる。修正候補はBUG-14の教訓
  (foreign-injected IMEモードキーの遮断は別アプリで機能的キー注入を破壊する)
  により慎重な設計が必要。
status: |-
  **ドラフト(起票、opus-adversarial-consult round1前)**。実装未着手。
related_adr:
  - "ADR-179"
  - "ADR-100"
---

# ADR-181: GJI(ATOKキーマップ)のVK_DBE_HIRAGANA自己主張がIME OFFを覆す不具合

## 背景・症状

ADR-179の`muhenkan_solo_tap_always_suppress = false`(Passthrough)実験の
実機A/B中(2026-09-19、dragonflyg4、notepad/Windows Terminal、GJI+
ATOKキーマッププリセット、TsfNative)で発見:

**IMEがOFFになった直後(数百ms〜数秒後)、ユーザー操作なしにIMEがONへ戻る。**

再現条件は「無変換単独タップのPassthrough」に限らず、以下いずれの
OFF契機でも同様に再現する(実機ログで確認済み):

- drift correction由来のOFF(`ir_apply_drift_correction`、
  `BlacklistDriftCorrection`)
- 明示的なCtrl+無変換チョードによるOFF(`origin=ExplicitUserAction`、
  `engine_decision_sync`)

## 機序(実機ログで確認した事実)

1. 何らかの経路でIMEがOFFになる(`dispatch_ime_set_open{open=false}`→
   GjiDirect `VK_IME_OFF`送信→`outcome=Applied`)。
2. 数百ms〜数秒後(観測した間隔は0.4秒〜28秒、typing活動が少ないほど
   長くなる傾向)、`vk=0xF2`(`VK_DBE_HIRAGANA`)のイベントがフックに届く。
   `injected=false`(`LLKHF_INJECTED`無し)かつ`self_injected=false`
   (awase自身のマーカー無し)——**genuinely外部由来**。
3. `hook.rs::classify_ime_relevance(vk)`はVKコードのみから
   `shadow_action = Some(TurnOn)`を計算する(injected状態は見ない)。
4. `kp_stage_shadow_ime_toggle`(`key_pipeline.rs:1485-1495`)が
   `belief.is_japanese_ime()==true`のとき`shadow_action`を
   `IntentKind::PhysicalImeKey`の正当なユーザー意図として採用する。
   **`event.injected`はこの分岐の判定に一度も使われない**
   (ログ出力(`:1558`)にのみ使われる)。
5. ADR-179が対象外とした既存仕様により、ひらがな/カタカナモードキーは
   `delegate_to_open_axis`経由で常に明示actuateする(Suppress/Passthrough
   の概念が無い)。したがってこの疑似TurnOn意図は無条件で
   `VK_IME_ON`実送信を引き起こす。
6. 結果: IME OFF直後に、無関係な外部由来のF2イベントがIMEを再びONへ
   戻してしまう。

## 内部送信元の除外(実コードで確認済み)

VK_DBE_HIRAGANAをawase自身が送信する経路は2つ確認したが、いずれも
この現象の説明にならない:

- `send_eager_tsf_warmup`(warmup機構): ADR-100決定2により
  **VK_IME_ONを送る**(VK_DBE_HIRAGANAではない)。
- `send_gji_half_width_alnum_toggle`(BUG-25半角英数トグル、Exit時に
  VK_DBE_HIRAGANAを送る): `IME_KANJI_MARKER`付きSendInputを使うため、
  `hook.rs::is_self_injected`が真になり`shadow-toggle`より前で
  `CallNextHookEx`に素通しされる——`kp_stage_shadow_ime_toggle`まで
  到達しない。

残る説明は、GJI自身(現在のキーマッププリセット=ATOK)が内部的に
このキーを模擬送信している、というもの。`composition_fsm.rs:290-305`
のBUG-31コメントが「無関係な物理IMEキー(VK_DBE_HIRAGANA、自己注入
ではない=外部/OS由来)が届く」現象を既に記録しており、本ADRの現象と
整合する(BUG-31はcomposition_fsm側の対処のみで、`shadow_action`/
意図判定側は未対応だった)。

## なぜ今まで顕在化しなかったか

既定のSuppress設定(`muhenkan_solo_tap_always_suppress = true`)では
無変換/変換単独タップがawaseに握りつぶされ、GJI/MS-IME側にIME OFFの
raw keyが渡ることも、awase自身がOFFを明示actuateしてOFF状態が数秒間
持続することも、ほぼ起こらない。今回初めてPassthrough設定を実機で
試したことで、IMEがOFFのまま数秒間持続するシナリオが作られ、上記の
周期的な外部F2イベントと衝突する機会が生まれ、初めて可視化された。
**現象自体はPassthrough実験固有ではなく、GJI(ATOKキーマップ)+
TsfNativeという組み合わせで以前から存在していた可能性が高い。**

## 制約: BUG-14の教訓

`hook.rs`のBUG-14修正コメント(1171-1178行目)が明記する通り、
foreign-injected(自己注入でない外部由来)のIMEモードキーをフック
レベルで遮断すると、MS-IME自身の機能的なキー注入(例: MS-IME×
Windows Terminalで導入直後から入力不能になった実例)を破壊する。
`event.injected`を条件に**単純に無視する**修正は、この既知の
リグレッションパターンを再現しうる。

## 検討中の設計方向(未確定、opus-adversarial-consultで検証してほしい)

1. **`event.injected`による除外を`kp_stage_shadow_ime_toggle`の
   ひらがな/カタカナ系VK(`ImeKeyKind`のうちDBE_HIRAGANA/KATAKANA等)
   に限定して追加する。** BUG-14が問題にしたのはVK_KANA(かなロック)
   系であり、DBE_HIRAGANA/KATAKANAとは別のVK。同じ轍を踏まないよう、
   影響範囲をVK単位で厳密に区切る必要がある。
2. **`injected`単独ではなく「直前に本物の物理キー活動があったか」を
   要求する(コロボレーション方式)。** 単発の孤立したF2イベントを
   疑い、直前・直後に他の物理キー入力(ThumbKey/Char等)が伴う場合のみ
   信頼する。ただし「疑わしいから無視する」設計は、逆に本物の単発
   Hiragana/Katakanaキー押下(ユーザーが意図して押した場合)を機能
   不全にするリスクがある。
3. **ADR-179の「ひらがな/カタカナは対象外」という決定を再検討する。**
   もしこの現象がGJI(ATOKキーマップ)固有で、かつ`delegate_to_open_axis`
   の「常に明示actuate」という設計自体が引き起こしているなら、
   ADR-179で見送った「ひらがな/カタカナにもPassthrough/Suppress概念を
   広げる」という案(当時「ボツ」とした)を、この不具合の文脈で
   再評価する価値があるかもしれない。

## 未解決の疑問(opus-adversarial-consultで検証してほしい点)

1. 上記3案(またはその他の案)のうち、BUG-14型の退行を再現しない
   設計はどれか。具体的な失敗シナリオを検証してほしい。
2. GJIの他のキーマッププリセット(既定/MS-IME風等)でも同じ現象が
   起きるか——ATOKプリセット固有の可能性を実コード上の根拠(あれば)
   で検証できるか。awase側にGJIのキーマッププリセットを検出する手段は
   無いため、これは実機検証でしか確認できない可能性が高い。
3. `hook.rs::classify_ime_relevance(vk)`が`injected`を受け取らない
   設計(VKのみから計算)は他の呼び出し元にも影響するため、この関数
   自体を変えず`kp_stage_shadow_ime_toggle`側だけで対処するのが安全か。
4. この修正は`.claude/rules/fix-requires-evidence.md`の「キー選択」
   ファミリーに該当するため、goldenテストか`docs/known-bugs/BUG-NNN.md`
   のいずれかが必要。実機再現のみで自動テスト化が難しい場合、後者を
   選ぶことになるが、これで十分な回帰防止になるか。
