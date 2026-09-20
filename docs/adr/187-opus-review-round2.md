# ADR-187 v2 opusレビュー round2

対象: `docs/adr/187-atok-passthrough-mode-key-observed-belief-follow.md`（worktree `adr187-atok-passthrough-follow`）
裏取り: 同worktreeのコード + CI実機ログ `/tmp/ciout2/result-atok-passthrough-1/dist/awase.log`

**判定: 未収束。Blocker 1件（新規） / Must-fix 5件 / Should-fix 4件。**
round1 の6件（B1/B2/B3/M1/M2/S1〜S4）は正しく反映されている。ただし **round1 自身の見落としにより、
決定2（IntentStoreのみ削除）では Engine が追随しない**ことが判明した。これは v2 の中心的な決定を書き換える。

---

## Blocker

### B4（新規）. 決定2では Engine は追随しない — 明示意図の pin は**2重**で、v2 は片方しか外していない

belief の pin は独立に2箇所ある:

| | 場所 | 根拠 | TTL | スコープ |
|---|---|---|---|---|
| **P1** | `ImeModel::resolve_open_at`（`state/ime_model.rs:390-409`） | `has_user_explicit_intent()` = `last_intent.is_some()`（同 `:341-343`） | **無し** | フォーカス（`FocusChanged` の reduce arm `:735` でのみクリア） |
| **P2** | `ImeStateHub::effective_open_at`（`state/platform_state.rs:674-681`） | `IntentStore::lookup`（`state/intent_store.rs:158-174`） | ON 10s / OFF 30s | hwnd |

呼び出し順は `effective_open_at` → `shadow = shadow_model.effective_open()`（ここで **P1** が効く）
→ `intent_store.resolve_effective_open(focus, shadow, now)`（ここで **P2**）。

```rust
// ime_model.rs:391-394
let has_explicit_intent = self.has_user_explicit_intent();
let (base, decided_by) = if has_explicit_intent {
    (self.desired_open, BaseDecision::ExplicitIntent)   // ← 観測を一切見ない
} else if let Some(outcome) = self.observations.derive_any(now) { ... }
```

決定2は P2 だけを消す。`last_intent`（ひらがな押下時に `record_intent` が設定、`ime_model.rs:527-535`）は
残るので、`shadow_model.effective_open()` は `desired_open = true` を返し続け、
`resolve_effective_open` は intent 無しの経路で **その true をそのまま通す**。
→ `ctx.ime_on` は true のまま、**Engine は OFF にならない**。決定2は目的を達成しない。

round1 は `intent_store.rs:158` の P2 だけを指摘し、ADR v2 はそれに忠実に従った。P1 の見落としは round1 の責任。

**対応（ADRで決めること）**: 決定2に「`last_intent` の無効化」を追加する。`last_intent` は
`state/ime_model.rs` の private フィールドで `reduce()` 経由でしか書けず、production のクリア点は
`FocusChanged` arm の1箇所しか無い（`ime_model.rs:735`、grep で確認）。したがって
**`ImeEvent` に新しい variant が1つ必要**で、これは v2 の決定4（新しい型・variantを足さない）と正面衝突する。
どちらかを改めること（→ M7）。

補足: 既存の前例に倣える。`PanicReset` / `HwndCacheRestored` は「観測でもユーザー意図でもない、
`desired_open` への直接書き込みの正当な例外」として隔離され、**どちらも `last_intent` を設定しない**
（`ime_model.rs:560-575`、テスト `panic_reset_does_not_set_last_intent`:2572 /
`hwnd_cache_restored_does_not_set_last_intent`:2655 が固定）。
さらに `apply_hwnd_cache_restore`（`platform_state.rs:1162-1195`）は
**`HwndCacheRestored` で `desired_open` を書き換え + `IntentStore` エントリを無効化**という、
ADR-187 が欲しい形とほぼ同一の組み合わせを既に production で行っている。ADR-187 の新 variant は
この系列の3つ目として位置づけるのが最も説明コストが低い。

---

## Must-fix

### M7. 決定4（新しい型・variantを足さない）は B4 により成立しない。足す前提で、更新すべきガードを列挙すること

新 `ImeEvent` variant を足すと、最低限:
- `.claude/rules/ime-belief-architecture.md`「新しい呼び出し元を追加する前に、本当に『全面復旧』
  『キャッシュ復元』に該当するか確認すること」の議論をADRに書く（3つ目の escape hatch を作る正当化）。
- `lints/ime_event_guard/src/lib.rs::RESTRICTED_VARIANTS`（現在 `PanicReset` / `HwndCacheRestored` /
  `EngineActivationSync` の3つ）に追加し、designated 関数を1つに固定する。
- `tests/architecture_guard.rs` の構築箇所数ガード（`PanicReset`/`HwndCacheRestored` と同型のもの）を追加。

「追加は通過マーク1個だけ」という現在の決定4の記述は、実装見積もりを実際より小さく見せている。

### M8. `last_intent` を消す副作用の棚卸しがADRに無い（消費者4箇所）

`explicit_intent()` = `shadow_model.last_intent.map(|i| i.target)`（`platform_state.rs:193-195`）の消費者:

1. `ir_decide_read_strategy` の `explicit_verify`（`runtime/ime_refresh.rs:308-311`）
   → false になり `SkipTyping` に落ちる。**決定3の通過マークが必須になる**（ADRの想定と整合。むしろ
   「last_intent が残っているから偶然通っていた」という現状を決定3が置き換える形になる）。
2. `reschedule_ime_refresh`（`runtime/mod.rs:886-889`）の早期 return
   → 消えると **500ms 周期ポーリングが再開する**。ImmCross では毎 tick クロスプロセス IMM 問い合わせが戻る
   （現状は explicit_intent がある間 恒久停止している。CIログでも 13.95 のひらがな押下以降
   `[stage-observe]` の周期出力が止まっている）。挙動変化として明記し、実験3の観点に入れること。
3. `check_drift_correction` の `is_strong_intent`（`platform_state.rs:886-893`）
   → しきい値 0 → `DRIFT_CORRECTION_THRESHOLD_MS`（400ms）。
4. `ImeModel::resolve_open_at` の `force_guards.resolve(base, has_explicit_intent)`（`ime_model.rs:414`）
   → ヒューリスティック guard（`BrokenAppBootstrap` 等）が override できるようになる。
   適用範囲（ImmCross）では通常 armed でないが、ADRに1行残すこと。

### M9 / M4'. `ActivationSync` の echo は「起きるか」ではなく**確実に起きる**。実験送りにせず決定にすること

- `EngineCommand::RefreshState` → `check_active_transition`（`src/engine/engine.rs:357-421`）→
  `transition_activation(new_state, SetOpenOrigin::ActivationSync)`（同 `:434-465`）が
  **active/inactive が反転したら必ず** `Effect::Ime(ImeEffect::SetOpen{open, origin: ActivationSync})` を push する。
  抑止されるのは `Inactive(NotRomajiInput)` のときだけ（`:447-450`）。本件は `Inactive(ImeOff)` なので抑止されない。
- 実行経路は `ir_notify_engine_refresh` → `execute_decision_suppressed` → `execute_from_loop`。
- **既存 strip の production 呼び出し元は `runtime/key_pipeline.rs:458-462` の1箇所だけ**
  （`executor.rs` の 1431/1452/1466 はすべて `#[cfg(test)] mod tests` 内）。条件は
  `shadow_toggled && event.ime_relevance.actuation_owner == ModeKeyActuationOwner::PhysicalDelivery`。
  本経路では `shadow_action` が None なので `kp_stage_shadow_ime_toggle` が早期 return し
  `shadow_toggled = false` / `actuation_owner = NotAModeKey`。**どちらの条件も偽**で、しかも発火場所が
  `execute_from_loop` 側なので、そもそも strip の呼び出しが通らない。
  → ADRの「出るなら `strip_...` を再利用する」は drop-in ではない。**新しい呼び出し点＋新しい条件**が要る
  （ADR-119「新しい gate を1箇所に置いて満足しない」型。`.claude/rules/fix-requires-evidence.md` の
  「IME actuation 合流点」行に載る変更になる）。
- **warrant では止まらない**: echo の要求値は `now_active`＝観測から導いた belief なので、
  `issue_open_warrant` の Step 3（`derive_actuating` = 同じ観測）と必ず一致し、`finalize` が warrant を出す。
  「冗長な送信」と「必要な送信」を warrant は区別できない。
- 実害の実体（M9 として分離）: `execute_decision_suppressed` は `UiEffect::EngineStateChanged` 由来の
  engine-state IME キー送信を抑止する（`runtime/mod.rs:599-605` の `suppress_engine_state_key_guard`）ので、
  残るのは `ImeEffect::SetOpen` 1本。ImmCross ではこれが `IMC_SETOPENSTATUS` の IMM write になり、
  その完了通知（`on_ime_apply_complete`）から GJI warmup が走る——CIログ 13.984 の
  `[gji-fsm] Unicode long-cold StartProbe: VK_IME_OFF→VK_IME_ON reinit` +
  `[ime-io] actuation SendInput kind=kanji_marker vk=[1A, 16]` がその実例。
  つまり「VK_IME_ON を二重に送る」ではなく「**VK_IME_OFF→VK_IME_ON の warmup バーストが走る**」が正しい記述。
  ユーザーが今まさに無変換で開けた GJI に VK_IME_OFF を送る形になり、BUG-113 系の機序に触れる。

### M10. 「Step 3(High観測=OFF)」は不正確、かつ Step 3 が空のケースが抜けている

- CI実機の観測源は `ObserverPoll` の **Medium**（ログ98行目 `source=ObserverPoll confidence=Medium`）。
  warrant の basis は `DirectRead` ではなく `SingleIndirect`（`open_warrant.rs:173-176`）。
  結論（`finalize` が requested と不一致で `None`）は変わらないが、根拠の記述を直すこと。
- `derive_actuating` が何も返さないケース（`OBSERVATION_FRESH_WINDOW_MS` 超過、または BeliefOnly
  ソースしか無い）では Step 4a/4b を素通りし、**Step 4c `OwnSsot(desired_open)`**（`open_warrant.rs:201-204`）
  に落ちる。`policy.default_feedback == Blind` のプロファイル（TsfNative / Imm32Unavailable）では
  `desired_open = true` で warrant が出て、awase が IME を ON へ戻す。
  適用範囲（ImmCross = `Read`）では Step4c は発火しない（`step4c_does_not_fire_for_read_profile`、
  `open_warrant.rs:511-533`）が、「適用範囲の外では戻りうる」ことを受け入れ基準に併記すること。

---

## Should-fix

### S6. 決定3の通過マークは既存プリミティブに載る（質問2・3への回答）

`GateStore`（`state/platform_state.rs:1557-1631`）は既に
`post_bypass: ScopedOneShot<crate::win32::ForegroundScope, PostBypassArm>` を持っている。
`ScopedOneShot`（`state/scoped_latch.rs:1-47`）は `arm(scope, payload)` / `peek(now_scope)` / `disarm()` を持ち、
**`peek` がスコープ不一致を自動失効させる**ので「`FocusChanged` でクリアする配線」を自分で書く必要がない
（この型の存在意義そのもの、doc 1-4行目）。

推奨: `GateStore` に `mode_key_passthrough: ScopedOneShot<ForegroundScope, PassMark>` を1フィールド追加。
`PassMark { armed_at_ms: u64 }` で窓を表し、消費側で `now - armed_at_ms <= WINDOW_MS` を見て `disarm()`。
新しい型は `PassMark` 1個（`Copy` な struct）で済む。

配線上の注意: `ir_decide_read_strategy` は `&self`、`ir_stage_strategy` も `&self`
（`runtime/ime_refresh.rs:135-137, 294`）。消費（`disarm`）には `&mut self` が要る。
呼び出し元 `ir_execute` は既に `&mut self`（同 `:63`）で両者とも private なので、
シグネチャを `&mut self` に変えるだけ（2行）。`explicit_verify` の隣に `|| pass_mark_live` を足す形になる。

### S7. 実験1の期待値表を B4 反映後に書き直すこと

現在の「2-aにより一致しないと予測する」は正しいが、**`last_intent` を消さない限り決定1+2+3を全部入れても
一致しない**ので、実験2の期待値も同時に破綻する。B4 の対応（`last_intent` 無効化）を決定に入れてから、
実験1（決定1のみ）/ 実験2（決定1+2+3）の期待値を引き直すこと。

### S8. belief と actuation で観測への要求水準が非対称であることを1行残す

`last_intent` を消すと `resolve_open_at` は `derive_any`（BeliefOnly プール込み、Medium 単独合意も採用）で
belief を決めるようになる。これは BUG-63（`mise`→「くした」）が問題にした導出そのものだが、
ADR-087 はこれを **belief 側では許容し、actuation 側でのみ禁じる**と明文化している
（`state/open_warrant.rs:5-14` の module doc、`ime_model.rs:345-356` の `effective_open` doc）。
「なぜ belief だけ緩いのか」を後から再発見しないよう、ADRにこの非対称を1行書いておくこと。

### S9. 「決定しないこと」の MS-IME 節に、ポーリング再開の副作用を足す

現在は「IntentStore を消して困る場面が無いかを実験3で確認する」だが、`last_intent` も消すなら
M8-2（`reschedule_ime_refresh` の停止解除）で **全プロファイルの IMM ポーリング頻度が変わる**。
実験3の観点に「追加ポーリングの副作用（BUG-34 系のクロスプロセス読み取り増加）」を明記すること。

---

## 未決事項への直接回答

**1. 決定2で ドリフト補正が実IMEを戻さない、という warrant の読みは正しいか**
→ **半分正しい。** IntentStore を消した後、ImmCross（`FeedbackPolicy::Read`）では戻らない:
`check_drift_correction` は `DriftCorrection{desired: true, observed: false}` を返すが、
`ir_apply_drift_correction`（`runtime/ime_refresh.rs:862-876`）→ `issue_actuation_order_with_origin`
→ `ActuationOrder::issue`（`state/actuation_chain.rs:341-352`）→ `issue_open_warrant(true, hwnd, ctx)`:
Step 1 は IntentStore 空で外れ、Step 3 は `derive_actuating` が観測値 false を返し
`finalize(requested=true, resolved=false, ..)` が **`None`**（`open_warrant.rs:211-220`）。
`set_ime_open_ordered` は `order.into_actuation().is_none()` で `false` を返して書き込まない
（`platform.rs:1694-1697`。A-2 強制は `platform.rs` / `ime_controller.rs:663` / `open_chain.rs:639` の3点で実在。
`actuation_chain.rs:393` の「現時点で本番呼び出し元は無い」という doc は stale）。
`record_optimistic` も呼ばれない（`ime_refresh.rs:874-876`）。
**ただし2つの穴**: (a) Step 3 が空のとき Blind プロファイルで Step 4c `OwnSsot` が warrant を出す（→ M10）。
(b) そもそも **Engine が追随しない**（→ B4）ので「戻さない」だけでは要件を満たさない。

**2. 決定3の通過マークを `ir_decide_read_strategy` にどう1条件で渡すか**
→ `GateStore` の新フィールド `ScopedOneShot<ForegroundScope, PassMark>` を `explicit_verify` の隣で
`|| pass_mark_live` として評価。`ir_decide_read_strategy` / `ir_stage_strategy` を `&mut self` に変える（→ S6）。

**3. 通過マークの窓・消費・`FocusChanged` クリアの置き場所**
→ `ScopedOneShot` がスコープ失効を `peek` の中で自動で行うため、`FocusChanged` 側の配線は不要。
窓は payload の `armed_at_ms`、消費は `disarm()`。既存の `post_bypass` と同じ使い方（→ S6）。

**4. `ActivationSync` の echo（VK_IME_ON の二重送信）は起きるか**
→ **起きる。** 抑止条件は `Inactive(NotRomajiInput)` のみで本件は該当せず、既存 strip の production 呼び出し元は
key_pipeline の1箇所で本経路を通らない。warrant も止めない。実体は VK 1本ではなく
IMM write + その完了から走る GJI warmup バースト（`VK_IME_OFF→VK_IME_ON`）。
engine-state IME キーだけは `execute_decision_suppressed` が抑止する（→ M9/M4'）。

**5. 収束と言えるか**
→ **言えない。Blocker 1件（B4）。** 決定2が目的を達成しないため、決定2と決定4を書き換えたうえで round3 が要る。
B4 に対応すれば、残りの Must-fix（M7〜M10）はすべて「ADR本文への追記・訂正」で閉じられる見込みで、
実装量は「新 `ImeEvent` variant 1個 + `PassMark` 1個 + 既存API呼び出し」に収まる。
