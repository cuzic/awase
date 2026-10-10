---
id: ADR-255-companion-255-opus-review-round7
title: |-
  ADR-255（IME OFF のときだけ無変換/変換を Space にする）Opus敵対的レビュー round7
type: companion-doc
related_adr:
  - "ADR-255"
---


---

## 追加評価: 所有者の提案「`keymap_latch` の代わりにエンジンの `KeyLifecycle` に記録する」

提案の内容: 遅いルールが Down を消費したら、`Engine::record_shell_consumed(&event)` を呼ぶ。この関数は `lifecycle.on_key_down_consumed(&event)` を行い、bare の親指なら `phase1_held = Some(vk)` も立てる。KeyUp とリピートはエンジン自身に回収させる。

### (1) 実コードで成り立つか → 成り立つ

- **リピート**: `on_input_body` 冒頭のガード(`src/engine/engine.rs:493-495`)は「`is_key_down && event.was_down && phase1_held == Some(vk)` なら `Decision::consumed()`」。活性・非活性を問わず Phase 1 より前で効く。遅いルールの照合(`kp_run_inner` の `on_input` の後)には届かない。
- **KeyUp**: `on_input`(:460-476)は Up で `phase1_held` を外し、`take_key_up_duty(vk)`(`key_lifecycle.rs:78-85`)で `UpDuty::Consume` を取り、出口で `force_consume` する。
  - 非活性のままなら Phase 2 で `adapter.release_only(&event)`(:513-516、`fsm_adapter.rs:50-53`)を通る。`output_history` に無変換のエントリは無いので、何も送らずに Consume になる。
  - Space は shell の effects で Down+Up を完結しているので、Up の義務は無い。
- **前提**: 記録は、遅いルールが当たり `force_consume` した後、`kp_stage_execute` より前に、同じ打鍵の中で行うこと。エンジンの判断を書き換えるのではなく、シェルの判断を事後に登録する口なので、ADR-112 の「Engine::on_input の唯一の出口」の不変条件とも衝突しない(Up 側は従来どおり唯一の出口を通る)。

### (2) 活性化後の Up、flush 時の KeyUp の再注入 → 害は既存の役割経路と同じ範囲

- **押している間に活性化した場合**: Up は `UpDuty::Consume` で Phase 3 の FSM に渡る(:524-)。FSM には Down 無しの親指 Up が届く。これは ADR-206 の役割経路(Phase 1 で Consume した変換の Up)と**同じ形**で、R2-S4 の「既存の役割経路と同じにする」にそのまま乗る。
  - `keymap_latch` 方式では Up がエンジンの前で消えるので、FSM は Up を見ない。どちらでも、押している間の文字キーはフックのスナップショットで親指シフト扱いになりうる(M3 の窓)。差は「FSM が Up を見るか」だけで、役割経路と揃う提案の方が検証しやすい。
- **フォーカス変更・非活性化での flush**: `release_pending_and_reinject`(:304-336)は `flush_pending_key_ups` の各キーについて `ReinjectKey(KeyUp)` を積む。`handle_focus_changed`(:733-736)は無条件に呼ぶ。
  - 押している間にフォーカスが移ると、OS/IME に**無変換の KeyUp が 1 つ**(Down 無しで)届く。その後の物理 Up は `UpDuty::None` で素通しされ、もう 1 つ届く。
  - Down の無い KeyUp は、修飾キーと違って固着を作らない。GJI は無変換の Down で動くので、実害は無い見込み。これも Phase 1 で Consume した役割経路と同じ挙動で、新しい種類の害ではない。
  - 決定4 に「既存の役割経路と同じ」と書き、単体テスト(`src/engine/tests.rs`)で、flush 時の `ReinjectKey(KeyUp)` を固定すること。
- **`phase1_held` が flush で消えた後のリピート**: `phase1_held = None`(:329)の後のリピート Down は、冒頭のガードを抜ける。
  - 非活性なら、R7-M2(`!was_down`)で遅いルールに当たらず素通しになる。OS には「リピート Down → Up」が届き、対になる。
  - 活性なら、リピートが FSM に PendingThumb として入りうる。これも役割経路と同じ既存の窓(ADR-206 のコメントが述べる範囲)。
- **古い記録(stale)**: overflow で Up を取り逃すと、`active_keys` に無変換が残る。次の押下で Down が素通しされ(例: composition 中で当たらない)、Up が `UpDuty::Consume` で飲まれると、OS 側に無変換の Down だけが残る。
  - ただし `active_keys` は、フォーカス変更と活性→非活性のたびに flush される。`keymap_latch`(アプリ無効化・watchdog・アンロック・panic でしか解放されない)より、古い記録の寿命がずっと短い。エンジンの全 Consume キーが既に負っている性質(ADR-112)に合流するだけで、遅いルール固有の新しい穴ではない。

### (3) 二重の機構にならないか → 層で分かれており、重ならない

- 早いルールはエンジンの**前**で消費する(エンジンは打鍵を見ない)ので、`keymap_latch` が Up を回収するのが正しい。遅いルールはエンジンが打鍵を**見た後**に消費するので、エンジンの `KeyLifecycle` が持ち主になるのが自然。機構は 2 つあるが、持ち主は打鍵ごとに 1 つに決まる。
- **対象のキーも重ならない**: 早いルールは `from` に親指キーを禁じたまま(決定5)。R7-M3(a) で遅いルールを無修飾の無変換/変換に限れば、両者の vk の集合は交わらない。同じ vk が `keymap_latch` と `active_keys` の両方に載ることは構造的に起きない。決定5 にこの交わらない性質を書き、`KeymapTable::new` のテストで固定する。

### (4) 推奨 → 提案を採る(`keymap_latch` には積まない)

理由:
- (a) r1 M4 の stale latch の問題が、遅いルールについては消える。決定4 の `was_down` で latch を捨てる手当、`KeymapLatch` に早い/遅いの種類を持たせる変更(S1)、`message_handlers.rs` ステップ1 の変更が不要になる。影響範囲から `runtime/message_handlers.rs` が外れる。
- (b) Down/Up とリピートの扱いが、既存の役割経路(ADR-206、Phase 1 Consume)と同じ機構・同じ窓になる。R2-S4 の「役割経路と同じにする」が実装上もそのまま成り立つ。
- (c) 古い記録の寿命が短い(flush のたびに消える)。

条件:
- R7-M1(送信を effects に載せる)・R7-M2(`!was_down`)・R7-M3(無修飾の無変換/変換に限る)はこの提案の下でも必要。特に R7-M2 は、リピート Down に当たって記録すると、最初の Down が素通しなのに Up を飲む同じ穴になる。
- 記録の API は「消費した Down の登録」だけに限り、エンジンの判断(活性/非活性・FSM の状態)を変えないこと。`phase1_held` を立てるのは bare の親指のときだけ(R7-M3 で常に満たされる)。
- 決定8 の引き返し条件の (1)(latch と親指ラッチの整合)は、この提案で「`KeyLifecycle` への登録が役割経路と同じ挙動になることを単体テストで固定できない」に読み替える。
