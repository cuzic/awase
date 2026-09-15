---
id: ADR-174
title: |-
  無変換/変換ソロタップの生キーパススルーを維持したまま、GJIの実結果を
  ソロタップ確定後に再観測してbeliefへ反映し、Engine ON追従を実現する
status: |-
  **2026-09-15: opus-adversarial-consult 3ラウンドいずれもBlocker検出で
  破綻。能動actuation代替案も実機履歴から却下（ユーザー判断）。次
  セッションは下記「次に検討すべき方向」から再設計すること。**

  - **round1**（旧設計、`resolve_pending_thumb_as_single`統合）:
    Blocker5件。最重要（B1）: この関数は対象シナリオ（`ime_on=false`）
    では**一度も呼ばれない**——`Engine::on_input_body`のPhase 2が
    `compute_active(ctx)=false`で早期returnし、Phase 3（NicolaFsm）
    自体に入らない。B2（`last_intent`がSomeの間`effective_open()`は
    `desired_open`に固定され観測を無視する）、B3（提案した再観測が
    drift correction経由でSendInputに到達し、GJIが開けたIMEをawaseが
    閉じに行く逆方向actuationになる——BUG-113「@」対策のガード5の穴を
    再び開ける）も検出。
  - **round2**（M2案、`explicit_ime_action_target`統合＋
    `should_run_idle_conv_check`ガード3緩和）: Blocker3件・Major7件。
    round1のB2・B3が形を変えて生き残っていることが判明——
    `report_conv_open_inference()`の呼び出し元が直後に
    `schedule_ime_refresh(20)`を自ら叩いてdrift correctionを誘発して
    おり「observe-onlyで安全」という前提が誤りだった。統合ポイント
    （`explicit_ime_action_target`）も副作用禁止契約・KeyDown/KeyUp
    二重呼び出しの点で機構的に不適と判明。一方でレビュアーから
    「`check_drift_correction`のConvOpenInferenceガードは
    `explicit_intent.is_none()`のときだけ効く→**素の変換/無変換単独
    タップで`last_intent`を無効化する1つの変更が、belief不動(P1)と
    逆方向actuation(P2)を同時に解く可能性がある**」という方針転換案
    （M3の元になった提案）が出た。
  - **round3**（M3案、新イベント`UserIntentAbandoned`で`last_intent`
    のみクリア）: Blocker2件。**決定的な見落とし**——
    `Engine::compute_state`が読む`ctx.ime_on`の供給元は
    `ImeModel::effective_open()`単体ではなく、その上に`IntentStore`
    （明示OFF意図を**30秒間**保持、BUG-51追補対策）が重なった
    `ImeStateHub::effective_open()`である。`last_intent`と
    `IntentStore`は同じ3箇所（`write_physical_key`/`write_sync_key`/
    `kp_stage_post_decision`のExplicitUserAction）で**同時に**書かれる
    ため、`last_intent`だけをクリアしてもIntentStoreが30秒間古い値を
    返し続け、P1（belief不動）は解けない。逆に`IntentStore`まで消すと
    それが導入された理由（BUG-51追補、2026-08-11実機再発）が直撃し、
    しかもdrift correctionも同時に無効化するため訂正経路の無い恒久
    固着（現状より悪い劣化）になる。round1〜round3の3ラウンド連続で
    この`IntentStore`層が設計から漏れていた。
  - **能動actuation代替案**（変換単独タップをVK_IME_ON＋元のVK_CONVERT
    転送に置き換える、ユーザー提案・2026-09-15）: ADR-153ケース3の
    実機履歴（`docs/known-bugs/BUG-113.md`・`BUG-124.md`、
    `key_pipeline.rs:1170-1189`）を精査した結果、**「生キーがGJIへ届く
    こと」「awase自身が明示actuationすること」のどちらか片方だけでも
    「@」を誘発するのに十分**と2回の実機A/Bで確定済み。この案は両方を
    同時に含むため高確率で「@」を再現すると予想され、ユーザー判断で
    却下・受動観測方針を維持することにした（2026-09-15）。

  レビュアーから目的そのものへの根本的指摘も出ている:
  `ConvOpenInference`（GJIのconv巡回からの間接推測）はBUG-19・BUG-51
  追補・BUG-55の3件で「単独で信じてはいけない」と繰り返し確認されて
  きたソースであり、本ADRの目的（この1件だけを根拠にEngineをActiveに
  する）はこれらの結論と正面衝突している。

  **次セッションで検討すべき方向（round3レビュアー提案、優先順）**:
  1. `IntentStore`を含むbelief解決の全階層（`ImeStateHub::
     effective_open()`→`IntentStore`→`ImeModel::resolve_open_at`→
     `last_intent`/`derive_any`/`most_recent_trusted`/`force_guards`）
     をADRに図示してから設計し直す。
  2. 「クリア」ではなく「素タップから有界なNms間だけ、`IntentStore`
     より新しいMedium+観測を優先する」という有界窓方式を検討する
     （BUG-19の1.6秒シナリオとタップ起点で区別可能、`force_guards`
     の判定〈`BrokenAppBootstrap`への無条件force-ON権限付与、round3
     Major M5〉には触れない）。Nは`tuning-constants.md`の実測義務対象。
  3. 弱い間接観測1件だけでEngineを活性化してよいかを先に決める。
     他ソースとの裏付け（corroboration）を要求する設計に倒せないか
     検討する。
  4. GJI既定キーマップで直接入力中の無変換/変換が実際に毎回IMEを
     開くのか、開かない構成が存在するのか、実機で確認する
     （現状これは無検証の仮定——round3 Major M4）。

  下記「決定（案）」節は**round1で破綻が確定した旧設計の記述であり
  失効している**（統合ポイント・未解決点ともに現在の設計方針とは
  異なる）。実装対象としては使わず、経緯の参考記録としてのみ残す。

  以下は上記redesignが必要になる前の起票内容（参考として残す）。
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

## 却下した代替案: 能動actuation（VK_IME_ON注入＋元のVK_CONVERT転送）

（ユーザー提案、2026-09-15）観測に頼る受動的な設計（M2/M3、下記参照）が
いずれも破綻したことを受け、「無変換/変換の単独タップをVK_IME_ON等の
IME制御actuationとオリジナルのVK_CONVERT/VK_NONCONVERTの転送を組み合わせた
打鍵列に置き換え、awase自身が能動的にbeliefを書き換える」という方向性を
検討した。ADR-153が却下した`henkan_solo_tap_ime_action = "on"`（オリジナル
キーを完全に置き換え、巡回機能が失われる）とは異なり、こちらは**GJIへの
生キー転送を維持したまま**IME ON actuationを追加する点が新しい。

**却下**。ADR-153決定1「ケース3」の実機履歴
（`docs/known-bugs/BUG-113.md`・`docs/known-bugs/BUG-124.md`、
`crates/awase-windows/src/runtime/key_pipeline.rs:1170-1189`）を精査した
結果、このアプリ/GJIの組み合わせでは次の2事実が2回の独立した実機A/Bで
確定している:

1. **旧ケース3**（生キーを抑止 + beliefが変化しなくても毎回強制actuate）
   → 「@」再現。
2. **旧ケース3の全面撤回版**（BUG-124、生キー抑止なし + actuateなし、
   GJI自身が無変換/変換を生で受け取る）→ 「@」再現
   （GJI自身のTSFキー横取り`ITfKeyEventSink`が原因）。
3. **ケース3改**（現行、生キーを抑止 + actuateなし）のみ「@」消滅を確認。

つまり「生キーがGJIへ届くこと」「awase自身が明示IME制御actuationを行う
こと」は、**どちらか片方だけでも「@」を誘発するのに十分**という機序が
確定している。今回提案した能動actuation案は、この2つのリスク要因を
**同時に**含む（生キー転送を維持しつつ、明示actuationも追加する）ため、
未検証ではあるが高確率で「@」を再現すると予想される。実装手段として
`.yab`の打鍵列機能（`ADR-115`、1キーに複数`KeyAction`を定義できる汎用
機構）を流用するかどうかは実装上の選択肢に過ぎず、この根本的な相性
問題を回避できるものではない。

ユーザー判断によりこの方向性は却下し、受動観測方針（下記M2/M3、
round2/round3で破綻したが方針自体は維持）を継続することにした
（2026-09-15）。

## 決定（案、opus-adversarial-consult未実施、**round1で破綻・失効。
参考記録として保持**）

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
