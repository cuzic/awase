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
  起草(2026-10-03)。Opus レビュー待ち。実装なし。
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
- journal(経過 2222779ms の OFF、他 10 回も同経路): 直前 0.3 秒まで親指シフトの打鍵が Consume され romaji が IME に
  送られていた。物理 英数(0xF0)は `Suppress(imm-cross)`、awase は `ImmCross SetOpenThenConvForTarget{open:false}`
  (`AppliedWithoutSendInput`)で閉じている。
- コード全体に、IME を閉じる前に未確定文字を確定する処理は無い(`CPS_COMPLETE` 等 0 件。`CPS_CANCEL` は Ctrl バイパス
  `runtime/mod.rs` のみ)。
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

## 決定

### D1: 修正の前に実測する

`crates/awase-windows/examples/msime_native_composing_probe.rs`(ADR-199 T17。自前 EDIT に SendInput で注入し
`ImmGetCompositionStringW` で実状態を読む)を拡張し、MS-IME 本体(新旧タイプ各 1)で、composing 中(例: `ka` を
未確定)に次の OFF 手段を与えたときの結果(確定文字列/未確定文字列/open/conv)を記録する。

1. 物理 英数(0xF0)をそのまま OS へ(awase なし。素の MS-IME の挙動)
2. 注入 `VK_IME_OFF`(0x1A)(`MsImeDirect` 経路相当)
3. `ImmSetOpenStatus(FALSE)`(`ImmCross` 経路相当。awase の実経路)

併せて「直接入力モードを使用しない」の有無の 2 通りを測る。EDIT に加え、既存の RichEdit(TSF ネイティブ)
プローブ(`richedit_tsf_probe.rs`)でも 1〜3 を測る(Chrome と同じ TSF 経路の代理)。

### D2: 実測結果ごとの修正候補(実測前に選ばない)

| 実測結果 | 修正候補 |
|---|---|
| 3 だけ破棄、1・2 は確定または半角英数で残す | MS-IME 時は 3 の前に `ImmNotifyIME(CPS_COMPLETE)`、または 英数を Suppress せず OS に渡す(BUG-46 の二重 actuation と衝突しないことを確認) |
| 1・2・3 すべて破棄 | 仕様どおり。awase は何もしない(報告は「機能要望」に分類を変え、設定で回避する案内) |
| 1・2・3 すべて残す | awase 側ではなく Chrome 等アプリ側の問題。journal の取り直し |

`composition_active` は信号として使えないため、「composing のときだけ確定する」条件分岐は作らない。
確定を足すなら無条件に近い形(OFF の直前)になる。

### D3: 実測するまでコードを変えない

D1 の結果が出るまで、IME OFF 経路・Suppress 判定・`composition_active` は変更しない。

## 検討して採らなかった案

- **今すぐ OFF 前に CPS_COMPLETE を足す**: MS-IME の素の挙動が未測定のまま、新しい書き込みを actuation 経路に増やす
  ことになる(ADR-117 が戒めた型。complexity-budget.md の趣旨にも反する)。
- **`composition_active` を MS-IME 向けに直す**: 信号の信頼性を上げる大工事で、症状の修正に必須ではない(OFF の
  直前に確定するなら composing の判定自体が要らない)。

## 影響・検証

実測のログを `docs/adr/221-measurements/` に置く。D1 は examples の拡張で、本番コードの変更を含まない。
