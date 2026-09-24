---
id: ADR-189
title: |-
  GJIの半角/全角キー(VK_DBE_SBCSCHAR 0xF3 / VK_DBE_DBCSCHAR 0xF4)を、VKで方向を決め打たず「beliefに基づく開閉トグル」としてawaseがactuateする
summary: |-
  GJIでは半角/全角の3つのVK(0x19/0xF3/0xF4)がどれも「開なら閉、閉なら開」のトグルで、入力中・変換中でも開閉の効果は同じ
  (開いていれば必ずOFF、未確定は破棄。実測はADR-186の表とCIの`--vkprobe`)。入力中かどうかで挙動が変わらないため、awaseが
  belief(`!belief`)で目標を決めてidempotentなVK_IME_ON/OFFでactuateすれば、観測なしで全アプリでモードずれが起きない。
  ところが0xF3/0xF4は静的に「0xF3=TurnOff、0xF4=TurnOn」の方向固定としてモデル化されており(`vk.rs::ImeKeyKind`)、同じVKの
  連続やF3/F4の順序で「押しても反転しない」(CIの`--hz`: 8手順中4手順が反転せず)。GJIがアクティブなときだけ、`shadow_action`を
  Toggleへ上書きする(`runtime/mod.rs`の既存の上書き点に1つ追加、既存のHiragana/Katakana/Henkan/Muhenkan上書きと同じ様式)。
status: |-
  **[ADR-191で範囲を拡張]** 本文の「GJIがアクティブなときだけ」「GJI以外・GJI判定が不明なときは静的モデルのまま」は、ADR-191（実装の現状9・11）でGJIと、CLSIDで同定できたMicrosoft IME本体の両方に適用する形へ変わった（未検出・第三者IMEは静的に決めず、生キーを通して観測に追随）。以下は当初（2026-09-20）の決定。
  **決定・実装済み(developマージ済み: `651cab8d`)・CI検証済み**。`msime-hz`/`atok-hz`(`check_toggle.py`)は、実装前は8手順中4手順が反転せず、実装後は各3/3で全8手順が反転しEngineも追随。
related_adr:
  - "ADR-186"
  - "ADR-187"
  - "ADR-179"
---

# ADR-189: GJIの半角/全角キーをbeliefに基づく開閉トグルとして扱う

## 背景

ユーザー方針(2026-09-20): 「MS-IMEキーマップの半角/全角のように、IMEがOFFならON、ONならOFFにするキーは、awaseがbeliefに基づいて
actuateすることでモードずれを起こさない」。入力中/変換中で挙動が変わるキーマップ(ATOKの無変換/変換)は、開閉トグルとして扱えない
(ADR-187)が、半角/全角は入力中でも開閉の効果が変わらない(ADR-186の表: 変換前/変換中でも「IME OFF、未確定破棄」)ので、
awaseがbeliefで決めて代行できる。

## 現状(CI `--hz`、実装前)

awase起動中、GJI(ATOK/MS-IMEキーマップとも)で、半角/全角をVK 0xF3/0xF4で8回押した結果(各回の実IME):
同じVKの連続(F3,F3 / F4,F4)で**開閉が反転しない**(8手順中4手順)。awaseが物理キーをSuppressし(`transport.rs`)、`vk.rs::ImeKeyKind`の
静的モデル(0xF3=TurnOff、0xF4=TurnOn)で方向を決め打つため。実IMEとEngineは一致している(ずれではない)が、GJIのネイティブな
挙動(どちらのVKもトグル)とは異なる。物理の半角/全角が押すたびにF3/F4を交互に出す保証はなく(ADR-186決定5)、OSのレイアウト層の
状態とIMEの状態がずれると、押しても反転しないキーになる。

## 決定

**GJIがアクティブなとき、0xF3/0xF4の`shadow_action`を`Toggle`にする**(修飾キー(Ctrl/Alt/Shift/Win)を押している場合は上書きしない)。
以後は既存のshadow-toggle経路で、beliefから目標(`!belief`)を決め、既存のactuation(GjiDirectのidempotentなVK_IME_ON/OFF)で開閉する。
物理キーは従来どおりSuppressのまま(`transport.rs`は変更しない)。0x19(VK_KANJI)は元からToggleで、`keys.ime_toggle`の既定。

- 上書きは`runtime/mod.rs`の既存の`shadow_action`上書き点(architecture_guardが書き込み箇所を1に固定)に`or_else`で1つ足す。判定本体は
  `gji_charset_autodetect.rs`の純関数(`resolve_hankaku_zenkaku_shadow_override_for_event`)。
- GJI以外(Microsoft IME本体など)は変更しない(静的モデルのまま)。GJIのアクティブ判定が不明なときも静的モデルにフォールバックする(安全側)。
- 観測(IMM読み取り)に依存しないので、TsfNative/Chrome等の読めないアプリでも効く。belief(`effective_open`)が実IMEとずれている場合は、
  リセット操作(Ctrl+無変換→Ctrl+変換、ADR-187)で直せる。

## 検証(CI `e2e-ime`、各3回)

| 構成 | 実装前 | 実装後 |
|---|---|---|
| `msime-hz`(GJIのMS-IMEキーマップ、`--hz`: 0xF3/0xF4の連続・交互8押下) | 8手順中4手順が反転せず(F3,F3 / F4,F4) | **3/3で全8手順が反転、Engineも追随** |
| `atok-hz`(ATOKプリセット) | 同上 | **3/3で全手順が反転、Engineも追随** |
| 退行: `baseline`、`atok-optin`、`atok-passthrough`(+cold)、`msime`、`msime-optin` | - | 各3/3 OK |

単体: `resolve_hankaku_zenkaku_shadow_override_for_event`(GJI×0xF3/0xF4のみToggle)、`architecture_guard`(shadow_action書き込み箇所は1のまま)。
実装は`resolve_hankaku_zenkaku_shadow_override_for_event`(`gji_charset_autodetect.rs`)と、`runtime/mod.rs`の既存の上書き点への`or_else`1つ。

未検証: 実機のキーボードで物理の半角/全角がどのVKを出すか(F3/F4の交互か、KANJIか)。どのVKでもbelief基づくトグルになるので、
出るVKに依存せず動く。Microsoft IME本体は対象外(静的モデルのまま)。

## 今回の範囲外(将来: ユーザー設定の学習、ユーザー方針 2026-09-20)

TsfNative/Imm32Unavailable(Chrome/Edge/メモ帳/Windows Terminal)での検証、およびawaseが自動では知らないキー(GJIのキーマップで任意のキーに
IMEOn/IMEOffを割り当てた場合など)への対応は、**較正機能(ADR-176、一度お蔵入り)と別セッションのスパイク実装を組み合わせ、ユーザーの設定を
学習する仕組みとして別途行う**。本ADR・ADR-187が用意した接続点:
- **`shadow_action`の上書き点**(`runtime/mod.rs`の`override_action`連鎖、書き込み1箇所): 学習した「このキーは方向固定On/Off/開閉トグル」を
  ここへ流せる(`resolve_*_shadow_override_for_event`と同じ様式)。
- **`keys.ime_on`/`ime_off`/`ime_toggle`**(awaseが消費して代行する既存の設定): 任意のキーを登録できる。
- **リセット操作**(Ctrl+無変換→Ctrl+変換、ADR-187): 学習が外れた・beliefがずれたときの必ず直せる手順。
- **CIハーネス**(`--walk`/`--resync`/`--hz`、`check_*.py`): 学習結果を検証する手順に流用できる。読めないアプリの検証にはBUG-149の`chrome_probe`が使える。
- 学習の判定材料(入力中に依存しないか)は、状態ごとに押して実IMEの前後を比べる測定(較正ウィザードの押下→観測)で得られる。
