---
id: ADR-188
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
  **決定・実装中**。CIの`msime-hz`/`atok-hz`(`check_toggle.py`)で、実装前は各3/3失敗(反転しない)、実装後に各3/3成功を確認する。
related_adr:
  - "ADR-186"
  - "ADR-187"
  - "ADR-179"
---

# ADR-188: GJIの半角/全角キーをbeliefに基づく開閉トグルとして扱う

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

## 検証

- 純関数の単体テスト、`architecture_guard`(shadow_actionの書き込み箇所数は1のまま)。
- CI `msime-hz`/`atok-hz`(`--hz`、`check_toggle.py`): 各押下で実IMEの開閉が反転し、Engineが追随すること。実装前は各3/3失敗、実装後に各3/3成功。
- 退行: `baseline`/`atok-optin`/`atok-passthrough(+cold)`/`msime`/`msime-optin`、リセット(`atok-resync`)。
