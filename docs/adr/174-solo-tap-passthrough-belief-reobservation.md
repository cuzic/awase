---
id: ADR-174
title: |-
  無変換/変換ソロタップの生キーパススルーを維持したまま、GJIの実結果をbeliefへ再観測で追従させる
status: |-
  **保留（B1未解決）。** opus-adversarial-consult round1でBlocker6件検出
  （B1「belief乖離→固着」の因果が未実証、B2既存ConvOpenInference経路への
  合流案は巡回を潰すactuationを増やすだけ、B3 observed側だけでは目的未達、
  B4-B6既存機構の転用が目的と噛み合わない）。round1の提案どおりB1を実機で
  確定させようとしたが、develop・GJI再起動後とも複数回「無変換単独タップ
  でIME ON」自体が再現せず、コード差分でブランチ起因の可能性も排除済み。
  [BUG-142](../known-bugs/BUG-142.md)に詳細記録、因果関係が未確定のため
  設計を先に進められない。round2は未実施のまま保留。
related_adr:
  - "ADR-153"
  - "ADR-173"
  - "ADR-172"
---

# ADR-174: 無変換/変換ソロタップの生キーパススルーを維持したまま、GJIの実結果をbeliefへ再観測で追従させる

## 背景・確定した事実（2026-09-15、実機ログで裏取り済み）

Windows Terminal + PowerShell + GJI で、半角モード（`ime_on=false`、直接
入力）中に無変換キーを単独タップすると、直後から物理半角/全角キーで IME
を OFF に戻せなくなる「IME ON固着」（[BUG-142](../known-bugs/BUG-142.md)）
が発生していた。実機デバッグログで根本原因を特定した:

```
[engine-input] vk=0x1D KeyDown ... [diag-ctx] ime_on=false ...
key input seq=41 vk_code=29 ... state_before="Idle" state_after="Idle" \
  decision="PassThrough" physical="Allow"
```

`ime_on=false`（NICOLAエンジン非活性）の間、無変換/変換キーは
`kp_stage_shadow_ime_toggle`（`explicit_ime_action_target`が未設定なら
`Inactive`、`shadow_action`自動検出の対象外）を素通りし、コアエンジンの
活性ゲート（`!ctx.ime_on` → `InactiveReason::ImeOff` →
`Decision::PassThrough`）により**生キーがそのままGJIへ渡る**。`[shadow-
toggle]`ログは一切出ない——**この打鍵に対してawaseのbeliefは何も更新
しない**。

GJI既定キーマップでは無変換/変換は「ひらがな⇔カタカナ⇔半角カナの巡回
キー」であり（ユーザー指摘）、直接入力（半角英数）状態から押すと巡回の
副作用として IME が ON（ひらがな）になりうる。この「実際にはONになった
のにbeliefはOFFのまま」という乖離が、後続の drift correction 等が誤った
前提で actuate を試み、GJI の TSF composition を乱す（固着として観測
される）という連鎖を生んでいたと考えられる。

## 却下した代替案: ADR-173（生キー抑止方式）

ADR-153 の `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`
（`explicit_ime_action_target`、ケース2/3改）は、無変換/変換の生キーを
**抑止**（GJIへ渡さない）し、awase自身が明示的にactuateすることでこの
乖離を構造的に無くす設計。ADR-173はこれを `app_overrides.solo_tap_
ime_action_apps` でアプリ限定できるようにした（PR #219、実装済み・
opus-adversarial-consult round1-2収束済み）。

実機検証（2026-09-15、dragonflyg4）: `muhenkan_solo_tap_ime_action =
"off"` + `solo_tap_ime_action_apps = ["WindowsTerminal.exe"]` で「IME
ON固着」は解消した。しかし:

1. **`henkan_solo_tap_ime_action = "on"`（変換キーを明示的にIME ON
   actuationへ置き換える）はユーザーにより却下された。** 無変換/変換の
   本来の役割は「ひらがな⇔カタカナ⇔半角カナの巡回」であり、「IME ON」は
   直接入力状態から巡回を始めたときの**副作用に過ぎない**。これを`"on"`
   という固定的な意味へ置き換えるのは設計として乱暴——GJI側のキー
   マップ変更（例: ユーザーがGJI設定で無変換の役割を変えた場合）に
   追従できず、本来の巡回機能も失われる。
2. muhenkan側の`"off"`（抑止のみ）も同じ理由で理想的ではない——巡回
   機能（ONの間の状態遷移としての無変換）は維持されるが、**半角状態
   からの巡回開始そのものが握り潰される**（生キーが届かないため）。

**ユーザーの結論**: 生キーは常にパススルーし、GJIの実際の結果を
awaseが**観測してbeliefに反映する**のが正しい設計。ADR-173の抑止方式は
実機configから撤回済み（`muhenkan_solo_tap_ime_action`/`solo_tap_ime_
action_apps`とも設定解除）。**ADR-173自体（`solo_tap_ime_action_apps`
という汎用のプロセス名限定機構）はコードとして残す**——将来他の目的で
再利用しうる汎用インフラであり、今回の方針転換はこの機構を使わない
という運用判断であって、機構自体の誤りではない。

## 決定（案、opus-adversarial-consult未実施）

半角モード（belief OFF）中に無変換/変換キーが `Decision::PassThrough`
で処理された直後、GJIの実際のconv-mode/open状態を再観測し、乖離があれば
beliefを訂正する。

### 設計の骨子（検討中、確定ではない）

1. **既存の再観測インフラの再利用**: `kp_stage_idle_conv_check_inner`
   （`key_pipeline.rs:633`）は既に「conv値を読み belief を訂正する」
   経路そのものであり、`kp_trigger_focus_resync`（`:618`）が「通常の
   周期実行ではなく特定イベント起因で即座にトリガーする」前例として
   既にある（フォーカス復帰直後のresync専用、`FocusResyncGate`による
   one-shot消費・世代フェンシング付き）。無変換/変換パススルー後の
   再観測も同型の「イベント駆動トリガー」として設計する。
2. **BUG-113 decision4（ガード5）との整合**: `should_run_idle_conv_
   check`（`src/engine/idle_check.rs:33`）は`is_ime_mode_key`引数で
   無変換/変換キー自体が起点になったidle-conv-check probeを**スキップ
   する**（このキー押下直後にconvを読むとGJIのTSF遷移がまだ完了して
   おらず中間値を拾う、`vk.rs::is_ime_mode_key_for_ime`のdoc参照）。
   本ADRが提案する「後から再観測する」トリガーは、この即時スキップと
   矛盾しない——**即時にではなく、GJIが遷移を終えるのに十分な settle
   時間をおいてから**再観測する設計にする（`schedule_ime_refresh(ms)`
   `runtime/mod.rs:779`が既存の遅延スケジューリング機構）。settle時間の
   具体的な値は実測が必要（`tuning-constants.md`対象、BUG-002の類似
   ケースでChromeのTSF再初期化に実測~326ms等の前例がある）。
3. **観測結果のbeliefへの反映経路**: 既存の`ObservationSource::
   ConvOpenInference`（`state/observation_store.rs`）が「conv値からIME
   open状態を推論してbeliefへ記録する」経路として既にある
   （`classify_conv_transition`、BUG-26対策）。新しい`ObservationSource`
   variantを追加するのではなく、この既存経路にこのイベントも合流
   させられないかをまず検討する（BUG-63の「弱い観測を信じすぎない」
   教訓、ADR-172の「観測ソースの信頼判定が4箇所目の独立判定になる」
   という同種の罠を繰り返さないこと）。

### 未解決・opus-adversarial-consultで詰めるべき点

1. **半角モード中のidle-conv-checkは、TsfNativeプロファイルでのみ意味を
   持つ**（`is_effectively_tsf_native`が`is_tsf_native`引数として
   `should_run_idle_conv_check`に渡る）。Windows Terminal（今回の対象
   アプリ）はこの分類に含まれることを確認済みだが、他プロファイル
   （Imm32Unavailable等）でも同じ乖離が起きうるか、対象を広げるべきか
   は未検討。
2. **settle時間の実測**: 無変換/変換パススルー後、GJIのconv遷移が
   実際に何msで完了するかの実測データが無い。`tuning-constants.md`が
   要求する「何ms必要かの実測」なしに値を決めてはならない。
3. **「巡回」の多段階性**: ひらがな→カタカナ→半角カナの3段階巡回を
   awaseのbeliefがどこまで追従する必要があるか（IME open/closeの
   二値だけで十分か、conv-modeの詳細〈カタカナ/半角カナの区別〉まで
   追従すべきか）は今回のBUG-142（IME ON固着）の解消に必要な範囲を
   見極めてスコープを決める。
4. **BUG-113「@」への影響評価**: 本ADRは生キーパススルーを維持する
   （ADR-173の抑止方式とは逆方向）ため、「@」（GJI自身のTSFキー横取り
   由来、awase停止でも再現）には対処しない——これは想定どおりで、
   BUG-113自体は既に「awase側で修正不能」と結論済み。本ADRの受け入れ
   基準に「@」の解消は含めない。
5. **再観測プローブ自体が新しい「@」トリガーにならないか**: 本ADRが
   追加する再観測は`WM_IME_CONTROL/IMC_GETCONVERSIONMODE`の読み取り
   専用プローブであり、`SendInput`によるactuationではない。BUG-113が
   確立した「@」の機構（actuationのSendInput自体が引き金）とは異なる
   経路のため直接のリスクは低いと考えられるが、ADR-140が扱った
   「probe/actuation競合」（別の物理キー操作由来のactuationとこの
   プローブが時間的に近接するケース）は理論上ありうる——
   `probe_actuation_fence`（ADR-140）の既存フェンシング機構が
   このプローブにも自動的に適用されるか確認する。

## 非スコープ

- 「@」の解消（BUG-113、awase側で修正不能と結論済み、別問題として
  切り離す）。
- ひらがな⇔カタカナ⇔半角カナの巡回状態そのものをawaseのbeliefが完全
  追従すること（IME open/closeの二値追従に必要な範囲を超える場合は
  別ADRで扱う）。
- ADR-173（`solo_tap_ime_action_apps`）のコード自体の削除——汎用インフラ
  として残す。

## 次のアクション

1. 本ADRをopus-adversarial-consultにかけ、上記未解決点（特にsettle時間
   の実測要否、既存`ConvOpenInference`経路への合流可否）を詰める。
2. 実機（dragonflyg4）でsettle時間の実測を行う（`tuning-constants.md`
   準拠）。
3. 実装後、BUG-142の再現手順（半角モードで無変換単独タップ）で固着が
   解消することを確認する。

## 関連

ADR-153（`explicit_ime_action_target`、却下した代替案の元設計）、
ADR-173（`solo_tap_ime_action_apps`、プロセス名限定機構・今回不使用だが
インフラとして残す）、ADR-172（TsfNative ON方向救済4系統の整理、
`ObservationSource`信頼判定の既存の罠）、BUG-142（本ADRが解消を目指す
「IME ON固着」）、BUG-113（「@」、本ADRの非スコープ）。
