---
id: ADR-171
title: |-
  gji_direct_already_matchesが候補ウィンドウ再表示という既存のdesync証拠(candidate_was_seen)を無視して再送を握り潰す不具合を修正する(BUG-141)
status: |-
  起草・opus-adversarial-consult round1/round2反映済み。round1・round2で当初案
  （belief/drift correction経由の自動補正、決定1-4）にBlocker合計8件が見つかり
  設計を全面転換、round2が提示した最小案（案Z）を主決定として採用。round2は
  「BUG-141のjournal解釈に事実誤認がある」ことも指摘し（M5）、それを自分で
  裏取りして確定させた（下記「事実の訂正」参照）。round3レビュー待ち。
  実装未着手。
related_adr:
  - "ADR-034"
  - "ADR-080"
  - "ADR-140"
  - "ADR-163"
---

# ADR-171: `gji_direct_already_matches`が候補ウィンドウ再表示という既存のdesync証拠(`candidate_was_seen`)を無視して再送を握り潰す不具合を修正する(BUG-141)

## 背景

[BUG-141](../known-bugs/BUG-141.md)（report `01M2D8HS5SBWSZ221P240Z4ZXE`）で、
Ctrl+無変換を3回押しても、直後の英字入力でGJIの変換候補ウィンドウ
（`GoogleJapaneseInputCandidateWindow`）が3回連続で実際に再表示される
現象が発生した。

### 事実の訂正（当初案・round1の誤読、round2 M5指摘を自分で裏取りして確定）

当初、journalの`ActuationDecision.outcome`だけを見て「Ctrl+無変換
（`SendVk(26)`＝`VK_IME_OFF`）を3回送ったが3回とも無効だった」と記述していたが、
これは誤り。実際にjournal（`ImeOpenApplied`/`ActuationDecision`）を精読すると:

- **1回目（elapsed `6128805`）**: `outcome:Applied`——実際に`VK_IME_OFF`が
  送信された。
- **2回目（`6131393`）・3回目（`6135183`）**: **`outcome:AlreadyMatched`——
  `send_ime_mode_key`は一度も呼ばれていない**（`gji_direct_already_matches`
  が`shadow_on==Some(false)==open`で早期に`None`を返したため、
  `decide_attempt`のGjiDirectアームで`MechanismCommand`自体が生成されない）。

つまり実際に起きていたのは「GJIが3回とも`VK_IME_OFF`を無視した」ではなく、
**「awase自身が2回目以降の再送をshadowモデル任せで握り潰していた」**。
awaseはこの間、候補ウィンドウの実際の再表示（`GjiFsmTransition
{StartComposition}`）という**desyncの直接証拠**を既に持っていた
（`tsf/observer.rs::candidate_was_seen`、下記参照）にもかかわらず、
その証拠を`gji_direct_already_matches`の判定に一切使っていなかった。

さらにjournal/app.logを実測で確認した結果、この区間（elapsed `6128715`〜
`6136419`、absolute `2026-09-13T11:30:03.35`〜`11:30:14.25`）では
**`ir_apply_drift_correction`の周期チェーンが一度も回っていなかった**
ことも確認した（app.log `[stage-observe]`行が`explicit_intent=Some(false)`
の間は疎らにしか出ず、`observer_poll=`行が皆無）。これは
`runtime/mod.rs::reschedule_ime_refresh`が「`explicit_intent().is_some()`の
間は次回tickを張らない」という設計（BUG-51と同型の既知の落とし穴、
`runtime/key_pipeline.rs:1090-1103`に前例コメントあり）によるもので、
本ADRのスコープ外の**別の既知の穴**として記録する（下記「関連する別の穴」）。

### 検討したが見送った設計（belief/drift correction経由の自動補正）

当初、候補ウィンドウSHOWを新しい`ObservationSource`として belief に流し、
既存のdrift correction機構に自動補正を委譲する設計（決定1-4）を起票した。
opus-adversarial-consultで2ラウンド実施し、合計8件のBlockerが見つかった
（`AnyObservation`構築不能、`platform.rs`にbelief経路が無い、BUG-114型の
無限再武装、観測が焼き付いて訂正されない、`reschedule_ime_refresh`の
explicit-intent停止でdrainが一度も走らない、`OffCold`gateの評価時点ズレに
よる日常操作での誤発火、episode ラッチの配線漏れ、`Imm32Unavailable`の
`ObserverPoll`が構造的に`false`を書けない）。詳細と全指摘は末尾
「検討した代替案（見送り）」参照。**[BUG-033](../known-bugs/BUG-033.md)が
既にこの種の設計を検討し(a)レイテンシ(b)新規observation source追加コストを
理由に見送っていたことも round1 で判明しており、本ADRでも同じ理由に加え
実装難度の高さから見送る。**

## 決定（案Z）: `gji_direct_already_matches`に`candidate_was_seen`を渡し、desync証拠がある場合は再送を短絡させない

### 現状の問題箇所

`state/ime_actuation_decision.rs:142-144`:

```rust
const fn gji_direct_already_matches(shadow_on: Option<bool>, open: bool) -> bool {
    matches!(shadow_on, Some(v) if v == open)
}
```

`shadow_on`（awase自身が最後に送ったコマンドの記録）が`open`（今回の要求）と
一致していれば、実際にOSへ何も送らずに`AlreadyMatched`を返す
（`decide_attempt`、同ファイル`:250`）。この判定は「前回送ったとおりに
GJIが状態を保っているはず」という**awase自身の記録**だけに基づいており、
その後にGJIの実状態が変化したという外部証拠（候補ウィンドウの実際の
再表示）を一切見ない。

一方、awaseは既にこの証拠を`TSF_OBS.candidate_was_seen`
（`tsf/observer.rs:176-181`、doc:「GJI candidateがSHOWになってから次の
`on_ime_applied`呼び出しまでの間に『shadow=OFFなのに候補ウィンドウが
表示された(desync)』ことがあったかを記録するラッチ」）として保持している。
このラッチは`EVENT_OBJECT_SHOW`で`true`に、`on_ime_applied_inner`
（`platform.rs:1518-1527`、`AlreadyMatched`を含む全outcomeで無条件に
リセット、`UnsafeToToggle`/`NotOwned`のみ例外）で`false`にリセットされる
——**次のapply判断が行われる直前まで値を保持し、apply完了後にリセットされる**
ため、「前回のapply〜今回のapply判断の間にSHOWがあったか」を正確に表す。

このラッチは既に`ImeControlView`（`state/ime_decision_view.rs::
ObservedState::candidate_was_seen`、`:54`）へスナップショットされ、
`OpenBelief::reduce`（`output/ime_apply_planner.rs:68`、
`!desired_open && self.candidate_was_seen`という条件式で`KanjiToggleStrategy`
向けのeffective_open計算に既に使われている）にも渡っている。**しかし
`impl From<&ImeControlView<'_>> for DecisionInputs`
（`state/ime_decision_view.rs:152-159`）はこの値をコピーしておらず、
`GjiDirectStrategy`が使う`gji_direct_already_matches`には届いていない。**
これが本バグの直接原因である——新しい観測経路を作る必要は無く、
**既存の値を既存の配線にもう1本つなぐだけ**で足りる。

### 変更内容

1. `DecisionInputs`（`state/ime_actuation_decision.rs:45-51`）に
   `candidate_was_seen: bool`フィールドを追加する。
2. `impl From<&ImeControlView<'_>> for DecisionInputs`
   （`state/ime_decision_view.rs:152-159`）で
   `candidate_was_seen: view.observed.candidate_was_seen`をコピーする。
3. `gji_direct_already_matches`を次のように変更する
   （`output/ime_apply_planner.rs:68`の`!desired_open && candidate_was_seen`
   と同じ形の条件式を、意味の異なる場所へ機械的に複製するのではなく、
   「OFF方向でdesync証拠があるときはshadow一致を信用しない」という
   **同一の判断ルールをこの2箇所に適用する**、という位置づけで書く）:

   ```rust
   const fn gji_direct_already_matches(
       shadow_on: Option<bool>,
       open: bool,
       candidate_was_seen: bool,
   ) -> bool {
       matches!(shadow_on, Some(v) if v == open) && !(!open && candidate_was_seen)
   }
   ```

4. 呼び出し元（`decide_attempt`、同ファイル`:250`）を
   `gji_direct_already_matches(inputs.shadow_on, open, inputs.candidate_was_seen)`
   に変更する。
5. `#[cfg(test)]`の`inputs()`ヘルパー（同ファイル`:269-284`）と、
   直接`DecisionInputs { .. }`を書いている他のテスト（`runtime/open_chain.rs`
   の`async_record_carries_caller_label_distinct_from_site`等）に
   `candidate_was_seen: false`（既定値、既存挙動を変えない）を追加する。

### 実際のBUG-141タイムラインでの動作確認（トレース済み）

1. Ctrl+無変換1回目（`candidate_was_seen=false`、初回のため）:
   `shadow_on`が`open(false)`と不一致 → 通常どおり送信、`Applied`。
   送信完了後`candidate_was_seen`はリセット（既に`false`）。
2. 候補SHOW（`6130462`）→ `candidate_was_seen=true`。
3. Ctrl+無変換2回目: `shadow_on==Some(false)==open`だが
   `candidate_was_seen==true` → **`!(!false && true)`は成立せず
   全体が`false`になり、already-matchedと判定されない → 実際に再送する**
   （旧実装ではここで無送信だった）。送信完了後リセット。
4. 候補SHOW（`6131714`）→ `candidate_was_seen=true`。
5. Ctrl+無変換3回目: 同様に再送する。
6. 4回目の`C`,`H`,`A`入力で候補は表示されず（実際のjournal通り）。

**この変更により、ユーザーが実際に押した3回のCtrl+無変換が3回とも実際に
GJIへ送信されるようになる**（旧実装では1回のみ）。GJI側が本当に受理する
かどうか（Mozc/Chromiumの`OnSetFocus`無条件上書き機序、BUG-141背景参照）は
依然awaseの管理外だが、少なくとも**awase自身がユーザーの意思を握り潰す**
という、この変更で確実に解消できる部分が直る。

### BUG-113再導入にならない理由（構造的、round2で確認済み）

BUG-113は「同一の物理キー押下に対し`shadow_toggle_off_sync`/
`engine_decision_sync`の2経路から連続で2回`apply`が呼ばれ、2回目も
実送信していた」ことが原因（`ime_controller.rs:88-100`のdoc）。
`candidate_was_seen`は`on_ime_applied_inner`が**`AlreadyMatched`を含む
全outcomeで毎回リセットする**ため、1回目のapply完了時点で`false`に落ちる。
同一物理キー押下の2回目の呼び出しは（実際のGJI側composition変化が
起きる時間が無いため）`candidate_was_seen`はまだ`false`のままで、
**従来どおり`AlreadyMatched`のまま**になる——本変更が拾うのは「新しい
候補SHOWが実際に挟まった後の再送」だけであり、BUG-113型の同一キー押下内
の重複送信を再導入しない。

### テスト（`.claude/rules/fix-requires-evidence.md`「キー選択」ファミリー）

- `state/ime_actuation_decision.rs`の既存テスト
  `gji_direct_skips_when_shadow_already_matches_close_direction`
  （`:558-567`、`candidate_was_seen`無しの既存挙動、暗黙に`false`）は
  変更後もそのまま緑であることを確認する（デフォルト`false`なら旧動作と
  ビット同値）。
- 新規テスト`gji_direct_resends_when_candidate_was_seen_despite_shadow_
  match`（同ファイル）: `shadow_on=Some(false)`, `open=false`,
  `candidate_was_seen=true`で`decide_attempt`が`Some(MechanismCommand::
  SendVk(..))`を返すことを固定する。
- `open=true`方向（ON時）は`candidate_was_seen`を条件に含めない
  （`!(!open && ..)`の`!open`ガードにより`open=true`のときは常に`false`
  側に落ち、既存の挙動と変わらないことを対称テストで固定する）。
- 実機A/B（developマージ前、`.claude/rules/tuning-constants.md`は無関係だが
  実機確認は必須）: BUG-113の症状（Windows Terminal × GJI × PSReadLineで
  余分な「@」）が再発しないことを確認する。BUG-141の再現条件（GJI長時間
  idle→Ctrl+無変換→候補SHOW）と同じセッションで一緒に測定できる。

## 関連する別の穴（本ADRのスコープ外、記録のみ）

「事実の訂正」節で見つけた`reschedule_ime_refresh`のexplicit-intent停止
（`runtime/mod.rs:834-855`、BUG-51と同型）により、本ADRの変更後も
「awaseが送ったVK_IME_OFFが本当に効いたか」を確認する周期的なdrift
correctionは、Ctrl+無変換直後の数秒〜十数秒間は動かないままである。
今回の案Zは「awase自身の握り潰しをやめる」だけで、GJI側が実際に受理した
かの自動確認・自動収束は依然無い。将来この方向の改善（BUG-51型の穴自体の
修正、または本ADRが検討した belief/drift correction 経由の設計の再検討）が
必要になった場合は、別ADRとして起票すること。

## 検討した代替案（見送り）: belief/drift correction経由の自動補正

以下は当初案の要約。実装しないが、将来同種の検討をする際に同じ轍を
踏まないよう記録する。

**方針**: 候補ウィンドウSHOWを新しい`ObservationSource`（evidence型5点セット:
`ObservationSource`variant追加・`declare_evidence!`・witness構築子・
`PerSourceObservations`フィールド・全数テスト更新）として`ObserverReported`
経由でbeliefへ流し、既存の`check_drift_correction`/`ir_apply_drift_correction`
に自動補正を委譲する。

**round1で見つかったBlocker（4件）**:
1. `AnyObservation`は`Observed<E>`のwitness経由専用で、ADRが書いていた
   コード片（`at`フィールド等）は実在せず構築不能。
2. ディスパッチ先として想定した`platform.rs`は`platform_state`（belief）を
   一切持たず、そこからbeliefへ書き込めない。
3. 新しい観測は`AnyFreshEvidence`除外リストに入らず、BUG-114で実機確認済みの
   「Blindバーストの無限再武装」を再現する（3秒クールダウンごとに再武装、
   BUG-141相当のセッションで最低5回・境界を跨げば10回の自動送信）。
4. confidence=Highを選ぶと、`Imm32Unavailable`にこの観測を上書きできる
   同等以上の観測源が構造的に存在せず、一度発火すると belief が
   「IME ON」に永久に焼き付く。

**round1の指摘を反映してdrain経路をGjiFsm外・`TIMER_IME_REFRESH`ベースに
再設計し、confidenceをMediumに変更した round2 でも、新たに4件のBlocker
が見つかった**:
1. `TIMER_IME_REFRESH`の周期チェーンは`explicit_intent().is_some()`の間
   停止する（前述、BUG-51と同型）ため、本ADRが対象とする状況で新しいdrainが
   一度も走らない。
2. `OffCold`gateをdrain時点で評価するため、「日本語を打ってからIMEを切る」
   という日常操作のたびに5連射を誘発する偽陽性がある。
3. episode ラッチ（`decide_conv_inference_drift`の流用）は書き込み側
   （`runtime/ime_refresh.rs:834`）の配線漏れでno-opになり、かつ
   明示意図なしのケースでは「プロセス起動中ずっと1回だけ」という
   恒久抑止になり、決定4後半（明示意図なしでも補正する）と正面衝突する。
4. confidenceをMediumにしても、`Imm32Unavailable`唯一の`ObserverPoll`観測源
   （`observer/gji_observer.rs:28-62`）は構造的に`Some(false)`を返す分岐を
   持たず、値としての訂正力がゼロ——「後続の観測が自然に上書きする」という
   前提が成立しない。

加えてround2は、この設計を導入する前にBUG-141の journal で「既存のdrift
correctionは既に発火していたのか」を確認すべきだと指摘した（本ADRの
「事実の訂正」節で確認済み: 発火していなかった。理由は上記「関連する別の穴」）。

**結論**: 8件のBlockerを全て解消するコストは、案Zの1関数1パラメータ追加という
コストと比べて見合わない。将来「awase自身の再送だけでは不十分（GJI側が
本当に受理したかを確認して自動収束させたい）」という実害が実機で確認された
場合に、この設計を再検討すること（BUG-033がADR-171に予約したのと同じ形で、
ADR-171が次のADRに予約する）。
