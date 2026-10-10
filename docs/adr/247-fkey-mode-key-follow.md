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
  起草(2026-10-10)。両案を実装済み。CI での評価中。
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

(CI の結果を追記する。)

## 既知の限界

- MS-IME 本体は F キーにモード切替を割り当てられない(ADR-199)。案1は GJI の CUSTOM 表だけ。
- `keys.ime_on` を F17 にした構成で、半角英数から戻れない件(awase が送る IME ON は GJI をひらがなへ戻さない)は別件で、本 ADR の範囲外。
