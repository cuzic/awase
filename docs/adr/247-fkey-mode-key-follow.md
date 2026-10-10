---
id: ADR-247
title: |-
  F13〜F24 に割り当てたモードキー(英数=F16 等)の効果に Engine を追随させる(BUG-196、直接観測と CUSTOM 表からの予測)
summary: |-
  BUG-196: ANSI 配列の利用者がキーボード側で英数などを F13〜F24 に割り当てると、GJI は半角英数になるが awase の Engine は ON のまま追随しない
  (CI の実 Chrome × GJI で 3/3 再現)。原因は F13〜F24 が(1)直接観測の対象(`is_followed_mode_key`)に入っていないこと、(2)打鍵時予測の表にセルが無く CUSTOM 表の行も読まないこと。
  案2 = `is_followed_mode_key` に F13〜F24 を足して ADR-188 の直接観測を使う(読める窓向け)。案1 = GJI の CUSTOM 表の行から開閉と入力モードを予測する
  (`awase_gji_config::role::custom_key_mode_effect`、読めない窓向け)。両方を実装して CI で A/B する。
status: |-
  起草(2026-10-10)。両案を実装済み。CI の A/B で両案とも単独で効くことを確認(実機未確認)。案1の単独の価値は未確認。
related_adr:
  - "ADR-188"
  - "ADR-191"
  - "ADR-199"
  - "ADR-211"
---

# ADR-247: F13〜F24 のモードキーへの追随(BUG-196)

## 問題

[BUG-196](../known-bugs/BUG-196.md)。ANSI 配列の利用者は、物理キーをキーボードのドライバで F14〜F17 などに割り当てる(外部記事)。
英数を F16 にして GJI の `config1.db` で F16 を半角英数へ SET すると、GJI は半角英数になるが awase の Engine は ON のまま `kiu` を出す。
CI(`sc-f16-alnum-chrome-*`、実 Chrome × GJI の CUSTOM 表)で 3/3 再現し、awase なしの対照は全回 PASS だった。

## 原因

1. `vk::is_followed_mode_key` は `is_ime_mode_key_for_ime`(0xF0〜0xF6 等)だけで、F13〜F24 を含まない。
   このため `kp_stage_mode_key_follow` / `executor` の FSM 再送出の経路が、直接観測の窓(ADR-188)も通過マーク(ADR-187/191)も開かない。
2. 打鍵時予測の表(`key_effect_table`、13 キー)に F13〜F24 のセルが無い。ADR-199 決定18 は F13〜F24 を開閉トグルの役割の候補にしたが、
   `KeyRole` は `ImeToggle` だけで、モードを SET するキーは受動のまま。CUSTOM 表の F キーの行も、`custom_table_overrides` が予測を打ち切るだけで読まれない。

## 決定

- **案2(直接観測)**: `is_followed_mode_key` に `is_role_fkey`(F13〜F24)を足す。`shadow_action` を持つ F キー(開閉トグルの役割)は従来どおり呼び出し側が除く。
  GJI × `Imm32Unavailable` の窓では ADR-188 の直接観測が、そのほかの窓では通過マーク + 20ms 後の IME の読み直しが働く。
- **案1(CUSTOM 表からの予測)**: `awase_gji_config::role::custom_key_mode_effect` が、CUSTOM 表の F キー(無修飾)の行から、状態(閉=DirectInput、開=Precomposition、入力中=Composition)に応じた
  開閉と入力モードの効果を返す(`IMEOn`/`IMEOff`/`CompositionMode*`/`InputMode*`。相対トグル・未知は `None`)。`KeyEffectKeymap::custom_f_key_prediction` がそれを `Prediction` に変える。
  変換中・修飾付き・自動リピート・overlay あり・プリセットは予測しない。観測が読めない窓(TSF)でも打鍵の時点で追随できる。
- 両案を同時に入れ、CI で個別に外して A/B する(`ablations/a9`〜`a11`、`sc-f16-alnum-chrome-gjif17-{obsonly,predonly,none}`)。評価で効かない案は撤去する(複雑性の予算)。

## 実装

- `crates/awase-gji-config/src/role.rs`: `KeyModeEffect`、`custom_key_mode_effect` と単体テスト。
- `crates/awase-windows/src/state/key_effect_predictor.rs`: `custom_f_key_prediction`(`predict_with_override` の `passive_open_key_prediction` の次)と単体テスト。
- `crates/awase-windows/src/vk.rs`: `is_followed_mode_key` に F13〜F24。
- `crates/awase-windows/examples/chrome_probe.rs`: `--f16-alnum`。`.github/workflows/e2e-ime.yml`: `sc-f16-alnum-chrome-*`。

## 評価(CI、実 Chrome × GJI)

構成: 実 Chrome × GJI の CUSTOM 表(F16=半角英数、F17=ひらがな/DirectInput では IME ON)、`chrome_probe --f16-alnum`(かな→F16=半角英数、F16 の半角英数→F17=かな)を 3 周。
run 38016104665(commit d98c291d)。`ablations/a9`(観測を外す)・`a10`(予測を外す)・`a11`(両方)で案を個別に外した。

| 構成 | かな→F16(`ka` が期待) | F16 の半角英数→F17(かなが期待) |
|---|---|---|
| awase なし(対照、GJI 単体) | 3/3 PASS | 3/3 PASS |
| 修正前相当(`-none`、a11) | **3/3 FAIL**(`kiu`、Engine ON のまま、400ms 後も同じ) | 3/3 PASS |
| 案2のみ(`-obsonly`、a10) | 3/3 PASS | 3/3 PASS |
| 案1のみ(`-predonly`、a9) | 3/3 PASS | 3/3 PASS |
| 両案(`-gjif17`) | 3/3 PASS | 3/3 PASS |

- **どちらの案も単独で効く**。実 Chrome(`Imm32Unavailable`)は直接観測が読める窓なので案2が効き、観測を外しても案1の予測だけで追随できる。
- 案1が効く理由は、観測が読めない窓でも打鍵の時点で効果が分かること。ただし、**この CI には「観測が読めず、案1だけが効く」窓が無い**。案1の価値を示せたのは「観測を外しても通る」までで、読めない窓での実測は未了(限界)。
- 退行確認: 全 `sc-*`(run 38016465418)で期待表と食い違う構成は 0 件(`NG` なし、OK 227 行)。F13〜F24 を通したモードキーに加えたことによる既存シナリオの悪化は見えない。
- `keys.ime_on = VK_F17`(awase が IME ON を送る、`-awimeon`)では、F16 は 3/3 PASS(修正後は Engine が OFF に追随)。F17 のあと GJI が半角英数のままで `ka`(Engine は OFF、IME の実状態と一致)となり 3/3 FAIL。
  **GJI は開いたままの IME ON で半角英数をひらがなへ戻さない**ため。別件(下記)。

### 読めない窓の確認(Windows Terminal、TsfNative)

案1が必要になる窓(直接観測が届かない窓)を探すため、実 Windows Terminal(`wt_probe.py` 相 F、`.github/workflows/f16-wt-verify.yml`、run 38018835598)で同じ A/B をした。
GJI の CUSTOM 表(F16=半角英数、Hiragana=ひらがな、Enter=Commit)。かな(Engine ON)で `kata`→かな、F16 のあと `kata` が ASCII のまま届くか(各変種 6 回)。

| 変種 | PASS / FAIL / INVALID |
|---|---|
| awase なし(対照) | 6 / 0 / 0 |
| 修正前相当(両案を外す、a11) | **6 / 0 / 0** |
| 案2のみ(a10) | 6 / 0 / 0 |
| 案1のみ(a9) | 6 / 0 / 0 |
| 両案 | 6 / 0 / 0 |

- **Windows Terminal では修正前でも再現しない**。TsfNative の窓は別の観測(TSF の conv 観測、`conv_obs`)が F16 の効果を拾い、1 秒余りで Engine が追随するため。
  Chrome では修正前に 3/3 FAIL だったのと対照的。
- したがって、**案1が必要になる窓は、この CI の範囲(Chrome・Windows Terminal)では見つからなかった**。案1が救うのは「どの観測も届かない窓」だけだが、そのような窓を CI で作れていない
  (InputRelay は設計上 awase が観測を採らない窓で、本件の対象としては不適)。
- 検証の注意: 初回の run は全回 INVALID だった。CUSTOM 表に Enter/Escape の行が無く、未確定文字列を確定できなかったため(表に行を足して解消)。

### 判断

暫定: 案2は読める窓全般(GJI 以外の IME を含む)へ効く最小の変更(1 行)、案1は GJI の CUSTOM 表で観測が読めない窓に効く見込みだが、現時点の CI では案2と区別できない。
案1は約 70 行と単体テスト 6 本の追加で、CI では案2との差を示せなかった(Chrome は案2が読める窓、Windows Terminal は修正前でも追随する)。**案1の撤去を推奨**する(読めない窓で F キーの追随が欠ける報告が出たら、本 ADR の設計で入れ直す)。

## 既知の限界

- MS-IME 本体は F キーにモード切替を割り当てられない(ADR-199)。案1は GJI の CUSTOM 表だけ。
- `keys.ime_on` を F17 にした構成で、半角英数から戻れない件(awase が送る IME ON は GJI をひらがなへ戻さない)は別件で、本 ADR の範囲外。
  筆者の記事の症状(英数入力から親指シフトへ戻らない)はこちらに近い可能性があり、筆者の設定を確認するまで断定しない。次の候補: awase の IME ON が半角英数の belief を見て、ひらがな(0xF2 等)を足して送る。
