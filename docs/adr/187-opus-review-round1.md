# ADR-187 ドラフトv1 opusレビュー round1

対象: `/home/cuzic/rust-nicola-worktrees/adr187-atok-passthrough-follow/docs/adr/187-atok-passthrough-mode-key-observed-belief-follow.md`
裏取り: 同worktreeのコード（`crates/awase-windows/src/`, `src/engine/`）+ CI実機ログ `/tmp/ciout2/result-atok-passthrough-1/dist/awase.log`

**判定: Blocker 3件 / Must-fix 6件 / Should-fix 5件。**
設計の方向（観測型・予測しない・actuateしない）は支持できるが、**未決事項1・2の前提がどちらも実コードと食い違っており、
そのまま実装・実験すると誤った結論に到達する**。特にB2は「決定2が要るか」という問い自体を変える。

---

## Blocker

### B1. 決定2の書き込み口（`write_physical_key` + `IntentWitness::from_physical`）は、無変換/変換では型として成立しない

`IntentWitness::from_physical`（`crates/awase-windows/src/state/evidence.rs:369-377`）の受理条件は

```rust
(!e.injected && (e.ime_relevance.shadow_action.is_some()
                 || e.ime_relevance.explicit_ime_action_consumed))
```

`shadow_action` は `hook.rs:289` で `vk.ime_kind()` から導出され、`ImeKeyKind::from_vk`（`vk.rs:122-136`）は
**0x1C/0x1D（変換/無変換）を含まない**（`_ => None`）。`explicit_ime_action_consumed` は ADR-153 の明示config専用。
さらにATOKパススルー構成では自動検出由来の `shadow_action` override も立たない——
`gate_thumb_key_ime_actions`（`gji_charset_autodetect.rs:363-392`）が `opt_in=false` かつ分類が `Toggle` のとき
`None` を返し、`ime_toggle_kind_to_shadow_action`（同:396-406）も `Toggle if !opt_in => None` で二重にゲートするため。

つまりADR-187が前提にしている「`write_physical_key`（`IntentWitness::from_physical` が要る）」は、
**本ADRの対象キーに対して常に `None` を返して黙って空振りする**。しかもこれは無言で失敗する形で、
`evidence.rs:356-368` のdocが「この受理を欠くと `write_physical_key` が一度も呼ばれず、
belief の OFF→ON 書き込みが**毎回黙って失敗する**」という同型の実機回帰（2026-09-08）を記録している。

**対応案（いずれかをADRで決めること）**:
- (a) `from_physical` に第3の受理条件（例: `ime_relevance.is_ime_mode_key && 通過マーカー`）を足す。
  ADR-153 の前例と同型だが、「物理IMEキーである証拠」の定義を広げることになるので、
  `tests/architecture_guard.rs` の `user_intent_source_construction_is_limited_to_typed_writers` と
  `record_explicit_intent_call_sites_are_limited_to_real_user_actions`（同:1022-1058）の両方を更新する。
- (b) そもそも意図として書かない（→ Should-fix S4 の代案）。

### B2. 原因2の手前に、より支配的な原因2'がある: `IntentStore` が `effective_open()` を pin する

ADRは「読み直しだけでは足りない可能性が高い」理由を**ドリフト補正**に置いているが、実コードではそれ以前に、
**Engine の belief そのものが明示意図で上書きされる**:

```
ImeStateHub::effective_open_at()  (state/platform_state.rs:674-681)
  → IntentStore::resolve_effective_open(current_focus, shadow, now)  (state/intent_store.rs:158-174)
     → lookup(target) が Some なら **無条件に intent.open を返す**（shadow_model を完全に無視）
```

CI実機ログで実際にこの状態が作られている:
- `13:13:13.952` ひらがな(0xF2) → `[shadow-toggle] intent 昇格: action=TurnOn kind=PhysicalImeKey false→true`
  → `write_physical_key` → `record_explicit_intent`（`platform_state.rs:1364-1379`）で IntentStore に **ON @ hwnd 0x501b2**。
  TTL は `EXPLICIT_ON_INTENT_TTL_MS = 10_000`（`tuning.rs:409`）。
- `13:13:17.15-17.28` 無変換(0x1D) 単独タップ → `send_keys: Key(0x1D)`、GJI が IME を閉じる。
- この時点から約10秒間、**観測が何を返そうと `effective_open()` は true を返し続ける**。

したがって **決定1（読み直し）だけでは Engine は絶対に追随しない**。
ドリフト補正が実IMEをONへ戻すかどうか以前に、ctx.ime_on が動かない。
（ドリフト補正の方も確かに起きる: `check_drift_correction`（`platform_state.rs:878-...`）は
`desired = shadow_model.desired_open()` = true、`explicit_intent == Some(desired)` かつ `last_intent.is_some()` で
**閾値0＝即時**、trusted観測が `ObserverPoll/ImmGetOpenStatus` なら `ConvOpenInference|HeuristicDefault` の
抑止にも当たらず発火 → `issue_open_warrant` の Step 1 `ExplicitUserIntent(open=true)` が warrant を出す
（`open_warrant.rs:150-158`）→ `set_ime_open_ordered(true)`。起動直後の同ログ `98行目`
`[drift] correction: observed=false ≠ desired=true for 529ms → set_ime_open(true) (source=ObserverPoll confidence=Medium)`
がまさにこの経路の実例。よってADRの原因2自体は**正しい**が、単独では不十分な記述。）

**ADRへの反映**: 原因2を「(2-a) IntentStore が effective_open を pin する（Engine が追随しない直接原因）」と
「(2-b) desired_open + 明示ON意図でドリフト補正が実IMEをONへ戻す（実害の拡大）」に分けること。
決定2の必要性は 2-a から**既に確定**しており、実験1で確かめるまでもない。

### B3. 実験1は設計上、意図した問いに答えられない（誤った「決定2は不要」に着地する）

実験1は「決定1だけ入れた仮ビルドで、実IMEがONへ戻されるかを見る。戻らなければ決定2は簡素化できる」としているが:

1. B2 のとおり、戻る/戻らないに関わらず Engine は追随しない（判定基準が要件と無関係）。
2. そもそも**観測が入るとは限らない**（→ M2）。観測が0件なら `drift_duration` も動かず「戻らなかった」という
   結果になるが、それは「決定2が不要」ではなく「決定1が空振りした」ことの証拠でしかない。

**対応**: 実験1の合否判定を「実IMEがONへ戻るか」ではなく、次の3点の**ログ証拠**に変えること。
- `[stage-observe] strategy=OsPoll`（`SkipTyping` ではない）が20ms後に出たか（`ime_refresh.rs:152-157` の既存debugログ）
- `ObserverReported` の journal エントリが記録されたか
- `[notify-refresh] ctx.ime_on=` が実IMEと一致したか（＝IntentStore pin を受けていないか）

---

## Must-fix

### M1. 未決事項2（「最大の未決事項」）の前提が誤り: `defers_solo_until_release` は既にパススルー親指を含む

ADRは「単独タップ確定がタイマー(100ms)経路のとき物理イベントが無い」「`defers_solo_until_release` を
Passthrough設定の親指にも広げるか」「前者は全パススルー利用者のタップ確定タイミングを変えるため影響が大きい」と書くが、
`src/engine/nicola_fsm.rs:990-999` は既に

```rust
special.dedicated_fn_key.is_none()
    && special.explicit_ime_action.is_none()
    && (special.delegate_to_open_axis.is_some()
        || special.mode_key_config.is_some_and(|cfg|
             matches!(SoloTapAction::from(cfg.for_composing(composing)), SoloTapAction::Passthrough)))
```

で、**delegate の有無に関わらず Passthrough設定の `ModeKeyConfig` を持つ親指を対象にしている**（ADR-182決定1c）。
CI実機ログがそのとおりの挙動を示している:

```
17.153  vk=0x1D KeyDown  state_after="PendingThumb(vk=0x1D,left=true)"
17.256  WM_TIMER logical_id=1  state_before="PendingThumb" state_after="PendingThumb"   ← タイマーで確定していない
17.280  vk=0x1D KeyUp   → send_keys: actions=[Key(VkCode(29)), KeyUp(VkCode(29))]        ← KeyUpで確定
```

したがって **`defers_solo_until_release` の拡張は不要**で、ADRが恐れている「全パススルー利用者のタップ確定
タイミングが変わる」影響も発生しない。この未決事項は解消済みとして削除し、代わりに次の1点だけを残すこと:

> 単独タップは KeyUp のほか「次のキーによる解決」（`step_pending_thumb_char` 決定1b / `decide_pending_thumb`）
> でも確定する。その場合 witness の取得元イベントは**解決のトリガーになったキー**ではなく
> **親指自身の KeyDown/KeyUp** でなければならない（次のキーは IME モードキーではないため）。

そして B1 のとおり、witness の実際の障害は「物理イベントが無いこと」ではなく
「`from_physical` が 0x1C/0x1D を受理しないこと」である。ADR の問題設定をここで差し替えること。

### M2. 決定1の `schedule_ime_refresh(20)` は typing-idle guard で空振りしうる（CIのwalkでは偶然通る）

`ir_decide_read_strategy`（`runtime/ime_refresh.rs:294-319`）:

```rust
let is_typing = idle_ms < TYPING_IDLE_MS;              // TYPING_IDLE_MS = 500 (tuning.rs:13)
if is_typing {
    let explicit_verify = !skip_imm_query
        && self.platform_state.ime.explicit_intent().is_some()
        && self.platform_state.ime.model().applied != AppliedImeState::Unknown;
    if !explicit_verify { return ImeReadStrategy::SkipTyping; }
}
```

20ms後の再読み取りは必ず `idle_ms ≈ 20 < 500` なので、`explicit_verify` が真でなければ**何も観測しない**。
`explicit_intent()` は `shadow_model.last_intent`（`platform_state.rs:193-195`）で、`FocusChanged` でクリアされる。

CIの `--walk` は必ず**ひらがな(0xF2)を先に押す**ため `explicit_intent=Some(true)` かつ `applied != Unknown` になり、
実ログでも `Explicit intent: bypassing typing-idle guard for IME verify (idle=94ms)` が出て `OsPoll` に到達している。
つまり **CIがgreenになっても「フォーカス直後にいきなり無変換を押す」通常の使い方を証明しない**。

**対応**: (i) ADRのリスク節にこの条件を明記し、(ii) CIのwalkに「フォーカス変更直後、IMEキーを一度も押さずに
無変換」のケースを追加するか、(iii) 決定1の再読み取りを typing-idle guard の対象外にする根拠を書く
（`explicit_verify` と同型の第2のバイパス条件を足すことになるため、`.claude/rules/fix-requires-evidence.md` の
「IME actuation 合流点」ではないが同じ「1箇所に置いて満足しない」型の配線漏れを作りやすい）。

### M3. 決定2が書く OFF 意図は TTL 30秒で、逆向きの固着を作る

`EXPLICIT_OFF_INTENT_TTL_MS = 30_000`（`tuning.rs:443`、ON の3倍という意図的な非対称）。
決定2が「観測 open=false」を明示意図として記録すると、B2 で示した pin が**今度は OFF 方向に30秒間**効く。
その間に次の無変換で実IMEがONに戻り、かつその回の再読み取りが空振り（M2）または IMM ミスすると、
**実IME ON / Engine OFF が最大30秒続く**——今直そうとしている症状の鏡像で、窓は3倍長い。

**対応**: 観測由来の意図には ON/OFF とも短いTTLを課すか、そもそも `IntentStore` に書かない設計にする（S4）。
少なくともADRのリスク節に「誤記録の最悪は『観測値どおりの意図』（実IMEと一致）で害が小さい」という現在の記述は
**不正確**なので訂正すること——害は「その時点で一致していること」ではなく「以後TTLの間、新しい観測を無効化すること」。

### M4. `ActivationSync` の自動echoをどうするかが決定に無い（OFF→ON方向で二重actuationになる）

ADR-179決定2は、`ModeKeyActuationOwner::PhysicalDelivery` について「awaseは明示actuateも `ActivationSync` の
自動echoも一切発行しない」とし、そのために**専用の strip 関数**を持っている:
`runtime/executor.rs:174-200` `strip_activation_sync_set_open_for_physical_delivery`。

決定2が belief を OFF→ON へ動かすと（無変換をOFF状態で押した回）、`check_active_transition` →
`handle_engine_activation_sync` → `SetOpen{origin: ActivationSync}` が発行され、GJIが既に開けたIMEに対して
awase も VK_IME_ON を送る。CIログ `13.952-14.028` に、ひらがな押下でこの連鎖が実際に走り
`[ime-io] actuation SendInput kind=kanji_marker vk=[1A, 16]` まで出ている実例がある。
ADR-119/BUG-46 型の二重actuationを**このADR自身が新規に作る**形。

**対応**: 決定に「観測由来の belief 追随では `ActivationSync` 由来 SetOpen を strip する（既存の
`strip_activation_sync_set_open_for_physical_delivery` を再利用する）」を明記すること。
既存関数の再利用で済むなら新規コードはほぼゼロ。

### M5. 観測を「明示意図」へ昇格させると、warrant 上で High 観測より強い権限を持つ

`issue_open_warrant`（`state/open_warrant.rs:136-207`）の評価順は Step0 SafetyValve → **Step1 ExplicitUserIntent** →
Step3 観測（High即採用/Medium合意）。つまり明示意図は `ImmGetOpenStatus` の High 観測すら押しのける
（同ファイルのテスト `step1_explicit_intent_blocks_actuating_observation`、:359-389 が固定している）。

弱い観測（`ObserverPoll` Medium、`FocusProbe` Low、`HeuristicDefault` Low）をそのまま意図へ昇格させると、
**観測の信頼度階層を1段階で飛び越える**。これは `.claude/rules/ime-belief-architecture.md` が
「`UserImeSetIntent` を使ってはならない — ユーザー意図を偽装することになり、confidence ガードを
完全にバイパスする」と名指しで禁じているパターンに構造的に近い（本件は物理キー押下という実イベントがある分
完全な偽装ではないが、**値の出所は観測**である）。

**対応**: 決定2に「昇格を許す観測ソースの allowlist」を明記すること。最低限
`ImmGetOpenStatus` / `ImmCrossProbe` の High のみとし、`HeuristicDefault` / `ConvOpenInference` /
`FocusProbe`(Low) は除外する。`check_drift_correction` が既に前2者を名指しで除外している
（`platform_state.rs`、`ConvOpenInference | HeuristicDefault` + `explicit_intent.is_none()` の分岐）ので、
判断基準の書き方はそこに揃えられる。

### M6. `record_explicit_intent` の呼び出し元は CI テストで件数固定、かつ `current_focus()=None` で黙って空振り

- `tests/architecture_guard.rs:1022-1058` `record_explicit_intent_call_sites_are_limited_to_real_user_actions` が
  `("src/state/platform_state.rs", 2)` と `("src/runtime/key_pipeline.rs", 1)` を固定している。
  決定2で新しい呼び出し元を足すならこのテストの更新が必須（忘れるとCIで落ちる＝検知はされる）。
- `record_explicit_intent`（`platform_state.rs:1348-1356`）は `if let Some(hwnd) = self.shadow_model.current_focus()`
  で、`None` のときは**何もせず黙って返る**。記憶にある BUG-148（`current_focus` 未設定で委譲SetOpenが不発）と同型。
  決定2は非同期タイマー経路から呼ばれるため、`current_focus` の設定タイミングを確認すること。

---

## Should-fix

### S1. 未決事項3（決定1の発火点）への回答: executor 側の実送出点1箇所で足りる。エンジン側は触らない

生キーは `execute_one` → `output::send_keys(actions=[Key(vk), KeyUp(vk)])` で出ている（CIログ 17.280）。
ここは既にプラットフォーム層で、`vk.rs::is_ime_mode_key_for_ime` をそのまま呼べる。ADR-019 の
「コアは VK を分岐しない」に抵触しない。**新しい型・variant・engine→platform の通知は不要**。
既存の `OUTPUT_GATE.last_vk_output_ms` を更新している箇所と同じ層なので、通過マーク（M1の witness を含む）も
ここに置くのが自然。
逆に `ImeOpenRequest::FollowOnly` の拡張（`src/engine/fsm_types.rs:706-717`）で解こうとすると、
それは**予測**（`ShadowImeAction` の分類に基づく belief 書き込み）であり、ADR-187 が原因3で却下した案Bに戻る。
FollowOnly は再利用しないこと。

### S2. 「開閉が変わったとき」の定義が曖昧（実際には「beliefと違うとき」＝ドリフトの定義そのもの）

決定2は「通過前の `belief.open` ≠ 観測 open」と書いているが、これは「変化したか」ではなく「beliefが間違っているか」の
判定であり、`check_drift_correction` の乖離判定と同じ述語になる。両者が同じ入力に対して別々に
結論を出す構造は、ADR-132/BUG-110 が「別々の関数が別々の解決経路で独立に計算する」欠陥として
繰り返し記録しているパターン（`resolve_warmup_ime_on` の doc、`platform_state.rs:560-568`）。
「通過**前**に実IMEを読む」のでなければ「変化」は測れない——読むなら読み取りが2回になる（通過前+通過後）。
どちらにするかをADRで明示し、「beliefとの差」で行くなら**その述語がドリフト判定と重複する**ことを
明記した上で、どちらが先に評価されるかの順序を決めること。

### S3. 効く範囲（プロファイル）を明記すること

CI実機の対象は `class="Edit"`、`profile=ImmCross`（ログ44/51行目、`read_ime_state_full: class="Edit"`）で、
IMM のクロスプロセス読み取りが効く環境。決定1が有効なのはここだけで、
TsfNative / Imm32Unavailable（メモ帳・Windows Terminal・Chrome/Edge）は ADR も書くとおり `ime_on=None` で無効。
ユーザー要件「かな=Engine ON、英数=Engine OFF、押下直後から」は**IMM読み取り可能なアプリでのみ満たされる**。
この限定を「決定しないこと」ではなく**受け入れ基準**の側に書くこと（どのアプリで直るのかがユーザーに伝わる形で）。

### S4. より単純で安全な代案: 「意図を作る」のではなく「古い意図を無効化する」

B1（witnessが取れない）・M3（OFF意図30秒固着）・M5（観測の権限昇格）はすべて
「観測値を明示意図として**書く**」ことから派生している。次の代案はそれを回避する:

> **決定2'**: 生キーを通過させた物理IMEモードキーについて、その対象hwndの `IntentStore` エントリと
> `last_intent` を**無効化する**（`IntentStore::remove`（`intent_store.rs:215-217`）は既に存在）。
> 新しい値は書かない。以後 `effective_open()` は `shadow_model.effective_open()` ＝観測の導出結果を素直に返す。

根拠:
- 意味論として正直: 「ユーザーがIME関連の操作をした。結果の状態は awase には分からないので、
  古い意図を根拠にするのをやめ、観測に委ねる」。捏造も権限昇格も起きない。
- witness 不要（B1 が消える）。TTL固着も起きない（M3 が消える）。
  warrant Step1 が外れて Step3（観測）が根拠になるので M5 も消える。
- 観測1件で belief が動くことは実測済み: CIログ65行目 `ObserverPoll +16ms since focus: ime_on true → false(intent=None)`
  （`ObserverPoll` Medium 単独で `effective_open` が反転している）。
- ドリフト補正も止まる: `check_drift_correction` は `desired = desired_open()` と trusted 観測の比較だが、
  `last_intent` が無くなれば閾値が 0→400ms になり、かつ `UserImeSetIntent` を書かない限り `desired_open` は
  ひらがな押下時の true のまま残る点は**未解決**なので、`desired_open` をどう扱うかは別途決める必要がある
  （ここが代案の弱点。`desired_open` を観測に追従させる正規の口が無い）。

**推奨**: 決定2'（無効化）を第一候補として検討し、`desired_open` の扱いで詰まるなら決定2（記録）に戻る、
という順で評価すること。どちらにせよ追加コードは「通過マーク1個 + 既存API呼び出し1箇所」に収まる見込みで、
`.claude/rules/complexity-budget.md` の精神（新しい型・合流点を増やさない）に合う。

### S5. 補助的な観測源として `[gji-io] WRITE` が実測で使える（ただし今回は採らない方がよい）

CIログ 17.295 に、無変換の生キー送出の直後 `[gji-io] WRITE: w_ops=+2 w_KB=+0.0` が出ている
（`tsf/gji_monitor.rs`）。「GJIがこの打鍵に反応した」ことの独立した証拠で、IMM が読めないプロファイルでも
取れる可能性がある。ただしこれは開閉の**方向**を教えないので単独では使えず、新しい観測軸を足すことになる。
今回は採用せず、ADRの「決定しないこと」に「将来TsfNativeを扱うときの候補」として1行残す程度に留めるのが妥当。

---

## 未決事項への直接回答（まとめ）

1. **原因2は起きる。ただし記述が不完全**（B2）。決定1単独では、ドリフト補正以前に `IntentStore` の pin で
   Engine が追随しない。よって**決定2（または代案 S4）は必須**で、実験1で確かめる必要はない。
   起きる条件: 直前（ONは10秒、OFFは30秒以内）に同一hwndへの明示意図がある場合。ひらがなキーの有無で
   挙動が変わるので、ADRの記述もそう限定すること。
2. **witnessは「タイマー確定だから取れない」のではない**（M1）。パススルー設定の親指は既にKeyUp確定
   （`defers_solo_until_release`、実機ログで確認）。真の障害は `from_physical` が 0x1C/0x1D を受理しないこと（B1）。
   `defers_solo_until_release` の拡張は**するな**（不要かつ影響が広い）。witness を持ち回るなら親指自身の
   KeyDown/KeyUp から取ること。
3. **executor の生キー実送出点1箇所**（S1）。`is_ime_mode_key_for_ime` をそのまま使う。engine側・`FollowOnly`の
   拡張はしない（案Bへの逆戻りになる）。
4. **漏れ・誤記録は残る**: Composition中の半角英数トグル（開閉不変）は差が出ないので確かに除ける。
   一方で (a) 観測が入らないケース（M2）、(b) 弱い観測の昇格（M5）、(c) OFF意図30秒固着（M3）、
   (d) `ActivationSync` echo（M4）、(e) `current_focus=None` での空振り（M6）が未処理。
   マウス/トレイ操作の巻き込みは「通過をマークした押下に紐づけ、短い窓で1回だけ消費する」で概ね防げるが、
   窓の長さ・消費の一回性・`FocusChanged` でのクリアをADRに明記すること。
