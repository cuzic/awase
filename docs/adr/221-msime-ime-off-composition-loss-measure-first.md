---
id: ADR-221
title: |-
  MS-IME で英数キーによる IME OFF が未確定文字を消す件(issue #138 / BUG-184)は、修正の前に OS 側の挙動を実測する
summary: |-
  report `01M3VCRSS2…`(MS-IME、1.21.0)の journal で、物理 英数(0xF0)が Suppress(imm-cross)され awase が ImmSetOpenStatus(FALSE) で IME を閉じる経路と、
  閉じる前に未確定文字を確定する処理が無いことを確認した。ただし「それで未確定文字が破棄される」こと自体は MS-IME で未測定(ADR-117 が同じ理由で挙動変更を見送った論点)で、
  `composition_active` は MS-IME では常時 false(ADR-117 の懸念どおり信号として使えない)。決定: 修正案を選ぶ前に、既存の msime_native_composing_probe を拡張して
  3 通りの OFF 手段 × composing を実測する(D1)。結果で分岐する修正候補を事前に列挙し(D2)、実測前はコードを変えない(D3)。
status: |-
  起草(2026-10-03)。Opus r1(2026-10-04)で事実認定の誤りを指摘され D0〜D2 を書き直し、再確認待ち。実装なし。
related_adr:
  - "ADR-117"
  - "ADR-186"
  - "ADR-191"
  - "ADR-100"
---

# ADR-221: MS-IME の IME OFF と未確定文字(issue #138 / BUG-184)

## 背景・事実

- 症状(issue #138 と同一。BUG-184、report `01M3VCRSS2BEXGPKH2S8BYCX30`、MS-IME・Chrome): 文字未確定のまま
  英数キーを押すと未確定文字が消える。報告者の要望は「確定してから IME OFF」。
- journal の OFF は経路が混在している(Opus r1 の指摘を journal で確認)。経過 2222779ms の OFF は秀丸
  (`hidemaru.exe`、Win32/ImmCross プロファイル)での出来事で、`ImmSetOpenStatus(FALSE)` 相当。報告直前の Edge
  (`msedge.exe`、TSF/`Imm32Unavailable`)では `MsImeDirect SendVk 26`(VK_IME_OFF)が 2 回ずつ(経過 2146130ms、
  2155092ms)送られており、こちらが再現操作とみられる(Edge で「かんじ+英数」を 2 回)。
- 英数(0xF0)を OS へ届けていないのは、報告者自身が `keys.ime_off = ["VK_DBE_ALPHANUMERIC"]` を設定していて、エンジンが
  Consume しているため(既定は Ctrl+無変換)。秀丸側の Suppress(imm-cross)は別の理由。
- コード全体に、IME を閉じる前に未確定文字を確定する処理は無い(`CPS_COMPLETE` 等 0 件。`CPS_CANCEL` は Ctrl バイパス
  `runtime/mod.rs` のみ)。他プロセスの窓では `ImmGetContext` が NULL を返し、`CPS_COMPLETE` 自体が使えない可能性が高い。
- `composition_active`(ADR-117 が追加した診断値)は OFF 11 回すべてで false。ADR-117 は「MS-IME の TSF インライン
  未確定は IME ウィンドウを作らず IME_SHOW が出ない可能性があり、false は無かったことの証明にならない」と予告していた。
  今回の報告が、**この信号は MS-IME では使えない**ことの最初の実データになる。
- 他 IME の実測(MS-IME ではない): GJI の `VK_IME_OFF` は未確定を commit する(ADR-100、BUG-36)。ATOK の 半角/全角 は
  OFF+未確定破棄(ADR-186 実測 r1:160)。**MS-IME は未測定**。

## 未確認(この ADR の核心)

「ImmSetOpenStatus(FALSE) で MS-IME が未確定文字を破棄する」は仮説である。ADR-117 は同じ仮説を、未検証のまま挙動を
変えると
[conv-mode を gate に使う失敗](../../.claude/rules/ime-belief-architecture.md)と同型になるとして見送った。journal に
残っていないもの: 文字が実際に消えた瞬間、消えた文字数、報告者の MS-IME 設定(「直接入力モードを使用しない」の
有無。#138 の元報告は無効時のみ)。

## 決定(r1 反映後)

### D0: まず v2(develop)で再現するか確認する

報告は v1.21.0 で、v2 は多数の経路を撤去している。既存の CI(`e2e-ime.yml`、`chrome_probe`)で、報告者の設定
(`keys.ime_off=["VK_DBE_ALPHANUMERIC"]`)の awase を動かし、実 Chrome のテキスト欄に `ka` を未確定にして英数を押し、
ページ側の値と未確定文字列を読む(BUG-176 の `sc-bug176-*` と同型、観測のみ)。セル: ① awase なし(素の MS-IME)
② awase あり・報告者の設定 ③ awase あり・設定に英数なし。再現しなければ本 ADR は取り下げ、BUG-184 は「v2 で再現せず」とする。

### D1: 再現する場合は実際の機構で測る

自前 EDIT の代理ではなく、実 Chrome(`chrome_probe --msime`)で、実際に使われている機構(注入 `VK_IME_OFF` ×2、
他プロセスへの `WM_IME_CONTROL` ×2、秀丸の ImmCross)ごとに、未確定文字の行方を記録する。

### D2: 結果ごとの修正候補(実測前に選ばない)

英数は Suppress ではなくエンジンの Consume で消費されているため、「英数を Suppress しない」案は効かない。
候補は ① OFF の機構を変える(`VK_IME_OFF` ×2 をやめる等) ② 確定を足す(他プロセスでは困難な可能性) ③ 仕様として案内する。

### D3: 実測するまでコードを変えない

## 検討して採らなかった案

- **今すぐ OFF 前に CPS_COMPLETE を足す**: MS-IME の素の挙動が未測定のまま、新しい書き込みを actuation 経路に増やす
  ことになる(ADR-117 が戒めた型。complexity-budget.md の趣旨にも反する)。
- **`composition_active` を MS-IME 向けに直す**: 信号の信頼性を上げる大工事で、症状の修正に必須ではない(OFF の
  直前に確定するなら composing の判定自体が要らない)。

## 影響・検証

実測のログを `docs/adr/221-measurements/` に置く。D1 は examples の拡張で、本番コードの変更を含まない。
