---
id: ADR-174
title: |-
  無変換/変換ソロタップの生キーパススルーを維持したまま、GJIの実結果を
  ソロタップ確定後に再観測してbeliefへ反映し、Engine ON追従を実現する
status: |-
  **起草し直し（2026-09-15、目的を再設定）。opus-adversarial-consult未実施。**
  当初この ADR は BUG-142（IME ON固着）の原因説明として起票されたが、
  round1/round2 で「belief乖離→固着」という因果自体が実機で確定できず
  （B1未解決）、対抗仮説（charset軸デッドロック）も出た末に、BUG-142の
  真因は[ADR-175](175-physical-dbe-key-stuck-direction-recovery.md)が
  独立に特定・解決した（物理半角/全角キーの固定方向マッピングによる
  shadow-toggleのno-op誤判定、`Toggle`解決への変更で実機A/B確定済み）。
  BUG-142がADR-175で解決した以上、**本ADRをBUG-142の対策として位置づける
  根拠は無くなった**。

  一方、BUG-142調査の副産物として「無変換/変換の単独タップでGJIが実際に
  IME ONにしても、awaseのbeliefが追従せずEngineが活性化しない」という
  別の実害（ユーザー指摘、2026-09-15: 「変換単独打鍵で、今IME On / Engine
  Offになっているんですが、IME On / Engine Onにできれば完璧」）が確認
  された。本ADRはこの目的に絞って起票し直す。設計上の統合ポイントも、
  旧版が使っていた`kp_stage_shadow_ime_toggle`の生KeyDown時点ではなく、
  よりチョード誤判定に強い`resolve_pending_thumb_as_single`（ソロタップ
  確定後）に変更した（下記「決定（案）」参照）。
related_adr:
  - "ADR-153"
  - "ADR-173"
  - "ADR-172"
  - "ADR-175"
---

# ADR-174: 無変換/変換ソロタップの生キーパススルーを維持したまま、GJIの実結果をソロタップ確定後に再観測してbeliefへ反映し、Engine ON追従を実現する

## 背景・確定した事実

Windows Terminal + PowerShell + GJI で、半角モード（`ime_on=false`、直接
入力）中に無変換/変換キーを単独タップすると、GJI既定キーマップの
「ひらがな⇔カタカナ⇔半角カナの巡回」動作により実際のIMEがON（ひらがな）
になりうる（ユーザー指摘）。実機ログで確認済み:

```
[engine-input] vk=0x1D KeyDown ... [diag-ctx] ime_on=false ...
key input seq=41 vk_code=29 ... state_before="Idle" state_after="Idle" \
  decision="PassThrough" physical="Allow"
```

`ime_on=false`（NICOLAエンジン非活性）の間、無変換/変換キーは
`kp_stage_shadow_ime_toggle`（`explicit_ime_action_target`が未設定なら
`Inactive`、`shadow_action`自動検出の対象外）を素通りし、コアエンジンの
活性ゲート（`!ctx.ime_on` → `InactiveReason::ImeOff` →
`Decision::PassThrough`）により**生キーがそのままGJIへ渡る**。
`[shadow-toggle]`ログは一切出ない——**この打鍵に対してawaseのbeliefは
何も更新しない**。

### なぜEngine ONにならないか（`compute_state`の性質、2026-09-15確認）

`Engine::compute_state`（`src/engine/engine.rs:308`）は次の純粋関数で、
別途「Engine起動」という手続きは無い:

```rust
pub const fn compute_state(&self, ctx: &InputContext) -> ActivationState {
    if !self.adapter.is_enabled() { return Inactive(UserDisabled); }
    if !ctx.is_japanese_ime { return Inactive(NotJapaneseIme); }
    if !ctx.ime_on { return Inactive(ImeOff); }
    if !ctx.input_mode.is_romaji_capable() { return Inactive(NotRomajiInput); }
    Active
}
```

`ctx.ime_on`はawaseのbelief（`effective_open()`）由来であり、**毎打鍵
ごとに再評価される**。つまり「Engine ONにする」ための特別なactuationは
不要——**belief.effective_open()が正しく`true`になった瞬間、次の打鍵で
Engineは自動的にActiveになる**。したがって本ADRの目標は「Engineを
ONにする処理を足すこと」ではなく、**「無変換/変換単独タップでGJIが実際に
ONになったという事実を、awaseのbeliefへ反映すること」**に単純化できる。

## 却下した代替案: ADR-173（生キー抑止方式）

ADR-153 の `muhenkan_solo_tap_ime_action`/`henkan_solo_tap_ime_action`
（`explicit_ime_action_target`、ケース2/3改）は、無変換/変換の生キーを
**抑止**（GJIへ渡さない）し、awase自身が明示的にactuateすることでこの
乖離を構造的に無くす設計。ADR-173はこれを `app_overrides.solo_tap_
ime_action_apps` でアプリ限定できるようにした（PR #219、実装済み・
opus-adversarial-consult round1-2収束済み）。

実機検証（2026-09-15、dragonflyg4）: `muhenkan_solo_tap_ime_action =
"off"` + `solo_tap_ime_action_apps = ["WindowsTerminal.exe"]` で「IME
ON固着」（当時の主目的）は解消した。しかし:

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

## 却下した代替案: `keys.ime_detect`の拡張

`keys.ime_detect.{on,off,toggle}`（`ADR-175`がVK_DBE_SBCSCHAR/DBCSCHAR
向けに使った、実機A/Bで確定済みの機構）に無変換/変換のVKを追加すれば
同じ効果が得られないか検討した。**却下**——`init_ime_sync_keys`
（`crates/awase-windows/src/app/mod.rs:253-283`）が、設定されたVKが
`left_thumb_vk`/`right_thumb_vk`（NICOLA同時打鍵チョード用の親指キー）
と一致する場合、sync key登録を自動的に除外する（BUG-140対策）。無変換/
変換をNICOLA親指キーとして使っている（本プロジェクトの主要ユースケース）
限り、この経路は構造的に塞がれる。

除外の理由も本質的: `ime_detect`のsync-key機構は「このVKのKeyDownが来た
瞬間、無条件にIME状態が変わったとみなす」という**事前の思い込み**方式
であり、チョード入力の1打鍵目（まだチョードかどうか未確定）を早まって
「IME操作」と解釈するとチョード判定自体を壊す（BUG-140の実例）。これは
上記「`henkan_solo_tap_ime_action = "on"`却下」と同型の問題——固定的な
意味へ置き換える設計は、チョードキーとしての本来の柔軟性を犠牲にする。

## 決定（案、opus-adversarial-consult未実施）

無変換/変換キーの押下が`src/engine/nicola_fsm.rs::
resolve_pending_thumb_as_single`で**ソロタップとして確定**し、かつ
専用Fnキー・ユーザー明示config（`*_solo_tap_ime_action`）・
delegate_to_open_axisのいずれにも該当しない（＝現状「何もしない」）場合、
GJIの実際のconv-mode/open状態を**遅延して**再観測し、乖離があればbelief
を訂正する。

### なぜ`resolve_pending_thumb_as_single`が正しい統合ポイントか

旧版のこのADRは`kp_stage_shadow_ime_toggle`（生のKeyDown到達時点）を
起点にする設計だったが、この時点では「このキー押下が単独タップか
チョードの1打鍵目か」がまだ確定していない。`resolve_pending_thumb_as_
single`は、NICOLAのチョード判定タイムアウト（`simultaneous_threshold_ms`
既定100ms）を経て**ソロタップと確定した後**にのみ呼ばれる（`ADR-153`の
優先順位1〜4の解決点そのもの）——したがって、ここを起点にすれば
「`keys.ime_detect`却下」節が指摘したBUG-140型のチョード誤判定リスクを
構造的に踏まない。既に`Option<ShadowImeAction>`（`ime_open_requested`
経由で`Engine::apply_ime_open_request`が消費する）という「ソロタップ
確定後にIME操作意図を返す」ための戻り値が存在する（専用Fnキー・明示
config・delegateの各分岐が使用中）——ただし、これらは全て**同期的に
確定する**アクション（今すぐ何をするか分かっている）を返す設計であり、
本ADRが必要とする「GJIの巡回結果を後から非同期に観測する」用途にはその
まま使えない（下記「設計の骨子」参照）。

### 設計の骨子（検討中、確定ではない）

1. **新しいトリガー**: `resolve_pending_thumb_as_single`が上記4分岐
   （専用Fnキー・明示config・delegate・その他ModeKeyConfig）のいずれにも
   該当せず「生キーをそのままパススルーする」結果になった場合（＝現状
   awaseが何もしない場合）、`kp_trigger_focus_resync`（`key_pipeline.rs:
   618`）と同型の「特定イベント起因の即座トリガー」として、遅延IME状態
   再観測を1回スケジュールする（`schedule_ime_refresh(ms)`、
   `runtime/mod.rs:779`の既存機構）。
2. **settle時間**: 即座に読むとGJIのTSF遷移が未完了で中間値を拾う
   （`should_run_idle_conv_check`の`is_ime_mode_key`スキップと同じ理由、
   `vk.rs::is_ime_mode_key_for_ime`のdoc参照）。GJIが遷移を終えるのに
   十分な時間をおいてから再観測する。具体的な値は実測が必要
   （`tuning-constants.md`対象、BUG-002の類似ケースでChromeのTSF
   再初期化に実測~326ms等の前例がある）。
3. **観測結果のbeliefへの反映経路**: 既存の`ObservationSource::
   ConvOpenInference`（`state/observation_store.rs`）が「conv値からIME
   open状態を推論してbeliefへ記録する」経路として既にある
   （`classify_conv_transition`、BUG-26対策）。新しい`ObservationSource`
   variantを追加するのではなく、この既存経路にこのイベントも合流
   させられないかをまず検討する（BUG-63の「弱い観測を信じすぎない」
   教訓、ADR-172の「観測ソースの信頼判定が4箇所目の独立判定になる」
   という同種の罠を繰り返さないこと）。
4. **目的の再確認**: 目標は「beliefを正しくすること」自体であり
   （旧版が主張していた「stale beliefに基づく誤ったactuationの発生を
   防ぐこと」というBUG-142向けの正当化はもはや不要——BUG-142は
   ADR-175で別に解決済み）、belief.effective_open()が正しく`true`に
   なれば、`Engine::compute_state`が次の打鍵で自動的にActiveへ遷移する
   （上記「なぜEngine ONにならないか」参照）。

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
   追従すべきか）はスコープを決める必要がある。
4. **確認済み**: `resolve_pending_thumb_as_single`は`fn(&self, ...)`
   （`nicola_fsm.rs:2130`）——`&mut self`ではなく`const fn`でもないが、
   自身では状態を書き換えない純粋関数として設計されている。戻り値
   `(ResolvedAction, Option<ShadowImeAction>)`の消費（`ime_open_requested`
   への代入）は呼び出し元（`on_input`等、7箇所）が`&mut self`で行う。
   つまり本ADRが必要とする「遅延観測をスケジュールする」という新しい
   意図は、既存の`Option<ShadowImeAction>`（同期的に確定するアクション
   専用）では表現できない——**新しい戻り値のバリアント（例:
   `enum PendingImeAction { Immediate(ShadowImeAction), ScheduleReobserve
   }`への拡張、または3つ目の戻り値）を追加し、呼び出し元7箇所全てで
   consumeする配線が必要**になる。これは`resolve_pending_thumb_as_single`
   の呼び出し元7箇所（`nicola_fsm.rs`内、テスト除く）全てに影響する
   変更であり、実装コストの見積もりに含めること。
5. **再観測プローブ自体が新しい「@」トリガーにならないか**: 本ADRが
   追加する再観測は`WM_IME_CONTROL/IMC_GETCONVERSIONMODE`の読み取り
   専用プローブであり、`SendInput`によるactuationではない。BUG-113が
   確立した「@」の機構（actuationのSendInput自体が引き金）とは異なる
   経路のため直接のリスクは低いと考えられるが、ADR-140が扱った
   「probe/actuation競合」（別の物理キー操作由来のactuationとこの
   プローブが時間的に近接するケース）は理論上ありうる——
   `probe_actuation_fence`（ADR-140）の既存フェンシング機構が
   このプローブにも自動的に適用されるか確認する。
6. **チョードの1打鍵目でこの再観測トリガーが誤発火しないことの確認**:
   `resolve_pending_thumb_as_single`が「ソロタップ確定後」にのみ呼ばれる
   という前提（本ADRの中心的な安全主張）を、実際のコードパス（チョード
   成立時は別の分岐を通るか）で裏取りすること。

## 非スコープ

- 「@」の解消（BUG-113、awase側で修正不能と結論済み、別問題として
  切り離す）。
- BUG-142（「IME ON固着」）の解消——[ADR-175](175-physical-dbe-key-stuck-direction-recovery.md)
  が独立に解決した。本ADRはEngine ON追従のみを目的とする。
- ひらがな⇔カタカナ⇔半角カナの巡回状態そのものをawaseのbeliefが完全
  追従すること（IME open/closeの二値追従に必要な範囲を超える場合は
  別ADRで扱う）。
- ADR-173（`solo_tap_ime_action_apps`）のコード自体の削除——汎用インフラ
  として残す。

## 次のアクション

1. `resolve_pending_thumb_as_single`のシグネチャ（pure関数か否か）を
   確認し、新トリガーの配線方法を確定する。
2. 本ADRをopus-adversarial-consultにかけ、上記未解決点（特に安全主張
   〈チョード1打鍵目での誤発火なし〉の裏取り、settle時間の実測要否）を
   詰める。
3. 実機（dragonflyg4）でsettle時間の実測を行う（`tuning-constants.md`
   準拠）。
4. 実装後、無変換/変換単独タップ直後にEngineが自動的にActiveへ遷移する
   ことを実機で確認する。

## 関連

ADR-153（`explicit_ime_action_target`、却下した代替案の元設計）、
ADR-173（`solo_tap_ime_action_apps`、プロセス名限定機構・今回不使用だが
インフラとして残す）、ADR-172（TsfNative ON方向救済4系統の整理、
`ObservationSource`信頼判定の既存の罠）、ADR-175（BUG-142「IME ON固着」を
独立に解決、本ADRの前提から切り離された経緯）、BUG-113（「@」、本ADRの
非スコープ）。
