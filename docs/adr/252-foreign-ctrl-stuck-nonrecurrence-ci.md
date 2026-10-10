---
id: ADR-252
title: |-
  「注入 Ctrl↓ だけで Up なし」のあとに物理打鍵が通常どおり変換されることを CI で観測する(ADR-249 の stuck 非再発)
summary: |-
  ADR-249 案 A' の安全性の根拠は「別枠は注入された打鍵にしか効かず、物理打鍵は読まない」こと。単体テストと走査は構造を固定するが、
  実機の hook で注入 Ctrl↓ の KeyUp 欠落のあとに物理相当の打鍵が NICOLA 変換されることは確かめていない。
  ci/e2e-dictation に4ケース(期限内の物理相当・期限内の注入・期限後の注入・0x11)を足し、観測する。
status: |-
  起草(2026-10-10)。未実装。
related_adr:
  - "ADR-249"
  - "ADR-054"
---

# ADR-252: 注入 Ctrl↓ の KeyUp 欠落後の stuck 非再発を CI で観測する

## 背景

[ADR-249](249-foreign-injected-modifier-ttl.md) の案 A' が案 A を退けた理由は、注入 Ctrl↑ が欠けても**物理打鍵**が素通しにならないこと
(ADR-054 問題2: X サーバーが Ctrl の KeyUp を送らず終了する stuck の再発防止)。これは構造(`read_os_modifiers` を変えない、決定5の走査)で守っているが、
**実機の hook で観測した証拠は無い**。ADR-249 の検証節は「追加で観測する」と書いたまま、PR #580 の A/B は paste の 36/36 だけだった。

同じ A/B で、ADR-249 決定1が「未確認」とした **`SendInput` の 0x11(generic Ctrl)が LL フックに 0x11 のまま届くか**も観測する(届き方で `is_ctrl_variant` の分岐が変わる)。

## 決定(案)

`ci/e2e-dictation` の dictation 系に、次の4ケースを足す。各ケースは「Ctrl↓ を注入する(`dwExtraInfo=0`、KeyUp は送らない)」から始める。

| ケース | 注入 Ctrl↓ の後 | 期待 | 守るもの |
|---|---|---|---|
| S1 | TTL 内に**物理相当**の打鍵(`TEST_INJECTION_MARKER`、`hook.rs:1328`)で NICOLA の列を打つ | 通常どおり変換される(`つうほめ…`) | 物理打鍵は別枠を読まない(ADR-249 決定3・5) |
| S2 | TTL 内に**注入**の文字キー(`dwExtraInfo=0`) | 素通しで ctrl 付きの扱い(変換されない) | 案 A' の意図した挙動(貼り付け) |
| S3 | TTL を過ぎてから注入の文字キー | 変換される | 期限が実際に効く(別枠が残り続けない) |
| S4 | S2 を VK 0x11 で行う | 到達した VK を記録し、期待は S2 と同じ | 決定1の未確認の解消 |

- 各ケースの最後に注入 Ctrl↑(`dwExtraInfo=0`)で OS 側を解放する。解放しないと runner の次のケースが汚染される(KeyUp 欠落そのものを再現しているため)。
- 構成は GJI・MS-IME の2つ × edit/rich(tsf は ADR-249 の A/B で貼り付けが別問題のため外す)。awase なしの対照は S2 だけ取る(Ctrl+文字が素通しの基準)。
- 「物理相当」は `TEST_INJECTION_MARKER` でフックが物理扱いにする経路(debug + `AWASE_TEST_INJECTION=1`)。
  **これは実物理キーと同じ経路かは未確認**(`is_injected` が false 扱いになることは `hook.rs:1463` で確認済み)。S1 の結論は「マーカー付き経路で」と限定して書く。
- TTL(`FOREIGN_CTRL_TTL_MS`=1000ms、pending)の実測は ADR-249 の未了項目。S3 の待ち時間は TTL + 余裕で、**TTL の値を決める根拠には使わない**(実測は別)。

## 検証・扱い

- 初回は observe(失敗しても CI を落とさない)。S1 が落ちたら ADR-249 案 A' の前提が崩れているので、即座に原因を調べる(別枠の読み出し漏れ、`HeldModifiers` への混入)。
- 結果は BUG-197 の状態欄と ADR-249 status に run ID つきで書く(「未了」から消す)。
- CI のみ。実機の確認は報告者に任せる(ADR-249 の未了項目、本 ADR の範囲外)。

## 未決

- S1 の「期待」の判定を、画面のテキスト一致で取るか、`[engine-input]` ログの `ctrl` 値で取るか(前者のほうが ADR-249 の A/B と揃う)。
