# ADR-172 敵対的レビュー round1

対象: `docs/adr/172-tsfnative-blind-rescue-four-system-consolidation.md`（174行、`2a943764` 時点）
レビュー範囲: 読み取りのみ。コード実体は `adr/172-tsf-blind-rescue-consolidation` worktree の HEAD を参照。

結論を先に: **決定2 は「何をするか」の記述は具体的だが、「それで何が達成されるか」の中核前提が
コードと食い違っている**（B1/B2）。この状態で opus-adversarial-consult にかけると、レビュアーが
誤った前提を引き継いだまま議論が進む。B1〜B6 を反映してから次工程へ進めることを推奨する。
決定1 も「挙動不変」という前提が成立しない（B6）。決定3・決定4 は方向性としては妥当だが、
根拠として挙げている理由が弱い、あるいは既存の実装事実と食い違う（S2/S5）。

---

## Blocker

### B1. 決定2は「`HeuristicDefault` 除外を force-on 側にも揃える」を達成しない（前提の事実誤認）

ADR 本文 96〜121行は、決定2 の効能を次のように書いている。

> drift correction が既に持つ `ConvOpenInference`/`HeuristicDefault` 除外（BUG-110修正）と
> 同じ観測ソース信頼基準をforce-on側にも揃える
> …これにより力業の調停機構を新設せずに、force-onとdrift correctionが同じ観測ソース信頼基準を
> 共有するようになる。

**`issue_open_warrant()` は `HeuristicDefault` を除外しない。逆に、専用の Step として明示的に
採用している。**

`crates/awase-windows/src/state/open_warrant.rs:180-190`:

```rust
// Step 4a: HeuristicDefault 観測が実在する（鮮度窓は適用しない、
// ObservationStore::heuristic_default() の doc 参照、§7 round4 S-C）。
if let Some(o) = ctx.obs.heuristic_default(ctx.now) {
    return finalize(
        requested,
        o.open,
        WarrantBasis::HeuristicGuess(HeuristicGuessSource::Observation(
            ObservationSource::HeuristicDefault,
        )),
    );
}
```

対して drift correction 側（`state/platform_state.rs:982-988`）は:

```rust
if matches!(
    trusted.source,
    ObservationSource::ConvOpenInference | ObservationSource::HeuristicDefault
) && explicit_intent.is_none()
{
    return None;
}
```

したがって「明示意図なし ＋ `HeuristicDefault` 観測のみ」という、まさに BUG-110 追補7〜9
（issue #189）が問題にした状況で:

- drift correction: `None`（発火しない）
- `issue_open_warrant(true, ..)`: Step 1 が外れ、Step 3 が外れ、**Step 4a が `HeuristicDefault`
  の値をそのまま採用して warrant を発行する**

つまり決定2 を実施しても、ADR が解消したいと書いている非対称性は解消しない。**移動するだけである。**
しかも Step 4a は doc が明記するとおり **鮮度窓を適用しない**（`:180-181`）ため、drift correction
の `DRIFT_CORRECTION_OBS_MAX_AGE_MS` による古い観測の切り捨て（`platform_state.rs:932-937`）より
むしろ緩い。`HeuristicDefault` の本番記録点は `platform_state.rs:1306`
（`Observed::<evidence::HeuristicDefault>::at_startup`、Imm32Unavailable ウィンドウ入場時）であり、
TsfNative は定義上ここに毎回到達する。

**さらに深刻なのは、TsfNative では Step 3 が構造的に不発になること。** `derive_actuating()` が
見るのは `authority() == Actuating` の観測源だけ（`open_warrant.rs:159-160`、
`observation_store.rs:780-790`）で、その 5 種は
`ImmGetOpenStatus / ImmCrossProbe / ObserverPoll / Gji / Tsf`（`state/ime_event.rs:193-207`）。
このうち本番で実際に記録されるのは以下だけである（`platform_state.rs` の
`Observed::<evidence::*>` 構築点を全数確認）:

| 観測源 | 本番の記録点 | TsfNative で到達するか |
|---|---|---|
| `ObserverPoll` | `platform_state.rs:1153`, `:1336` | **しない**（TsfNative は周期ポーリングが無い。`runtime/mod.rs:959-961` のコメントが「TsfNative はそれを上書きする周期ポーリングが無い（`reschedule_ime_refresh` が早期 return する）」と明記） |
| `ImmCrossProbe` | `platform_state.rs:1475` | **しない**（IMM クロスプロセス不可がそもそも TsfNative の定義） |
| `ImmGetOpenStatus` | `state/ime_actuation.rs:524`（classify 経由） | 同上 |
| `Gji` / `Tsf` | **ゼロ** | `state/evidence.rs:126-134` の `declare_evidence!` で型は宣言されているが、`Observed::<evidence::Gji>` / `<evidence::Tsf>` の構築は本番コードに1箇所も無い（テストと `open_warrant.rs` の doc のみ） |

結果、**TsfNative における `issue_open_warrant(true, ..)` は実効的に
「Step 0（override guard）→ Step 1（IntentStore）→ Step 4a（`HeuristicDefault`、鮮度窓なし）→
Step 4b（heuristic guard）→ Step 4c（`desired_open`）」という梯子に縮退する。**
最終段 Step 4c（`open_warrant.rs:201-204`）は `default_feedback == Blind` で `desired_open` を
そのまま採用するので、決定2 の実質は

> TsfNative の force-on ゲートを `effective_open()` から `desired_open`（+ IntentStore +
> 鮮度窓なしの `HeuristicDefault`）へ置き換える

である。これは ADR が書いている「観測ソース信頼基準を揃える」とは別の変更であり、ADR 本文には
一言も書かれていない。**決定2 を残すなら、この実効的な変更内容を本文に明記した上で、
それが本当に望ましいのかを改めて判断すること。**

### B2. 決定2は `FocusProbe` を actuation 根拠から外す変更でもあり、これは drift correction 側が意図的に拒否した方向

`FocusProbe` の authority は `BeliefOnly`（`state/ime_event.rs:200-205`）。したがって
`issue_open_warrant()` の Step 3（`derive_actuating`）は **`FocusProbe` を必ず除外する**。

一方 drift correction 側は `FocusProbe` を意図的に残しており、その理由がコードコメントに
はっきり書かれている（`state/platform_state.rs:978-981`）:

```
// - `FocusProbe` は推測ではなく実 IMC 読み取り（Low confidence なのは
//   hwnd の曖昧性ゆえ、BUG-91 由来）。これを抑止すると BUG-16/BUG-20
//   型の固着（belief と実 IME が乖離したまま補正されない）を再導入する
//   リスクがあり、実機再現なしに含めるべきではない。
```

同コメントの直前（`:958-963`）には、除外の判断基準を `authority() == BeliefOnly` にするのは
**「判断基準として使うには広すぎる（opus-adversarial-consult 指摘）」** と、過去のレビューで
一度否決された旨まで書かれている。

決定2 は、その否決された基準（`authority()` による一括判定）を force-on 側に持ち込む変更である。
ADR は「揃える」と表現しているが、実際には **drift correction が明示的に採らなかった基準へ
force-on だけを動かす**。少なくとも:

- ADR 本文で「両者は揃わない。force-on 側だけ `FocusProbe` を除外することになる」と正直に書く
- その上で、force-on（ON 方向のみ、`requested=true` 固定）では `FocusProbe` 除外が
  BUG-16/BUG-20 型の固着を招かない理由を個別に論証する（drift correction と方向が違うので
  同じ結論にはならない可能性が高いが、**「違うから大丈夫」ではなく論証が要る**）

具体的な失敗シナリオ: TsfNative アプリで `FocusProbe` が `open=true`（Low）を実測しており、
`desired_open=false`、明示意図なし、`HeuristicDefault` なし。
- 旧: `effective_open()` が `FocusProbe(true)` を採用 → `is_eligible_for_ime_force_on()==true`
  → force-on が走り、ON 方向の救済が効く（BUG-16 の想定シナリオ）
- 新: Step 3 が `FocusProbe` を除外 → Step 4a/4b なし → Step 4c が `desired_open=false` を採用
  → `finalize(requested=true, resolved=false)` → `None` → **force-on が止まる**

これは ADR の「落としてはいけない既知シナリオ」1（BUG-69）とは別種の、ON 方向救済が消える経路
である。チェックリストに入っていない。

### B3. 決定2は ADR-090 §2.A の A-2 そのものだが、ADR-090 を本文にも frontmatter にも引いていない

ADR-172 は `issue_open_warrant()` を「実装済みだが未配線」（ADR 106-110行）と書いているが、
**これは不正確**。`docs/adr/090-*.md` §2.A.6「A-1 実施記録（2026-08-12）— 実施済み」により、
`ActuationOrder::issue()`（= `issue_open_warrant()` を内部で呼ぶ）は**実 actuation 起案点
11 箇所すべてに配線済み**である。force-on の2つの入口も例外ではない:

- `runtime/mod.rs:1035` — `issue_actuation_order(true, "force_on_and_correct_romaji")`
- `runtime/mod.rs:1282` — `issue_actuation_order(true, "try_force_on_bootstrap")`

残っている作業は「配線」ではなく **A-2（`into_actuation_shadow` → `into_actuation` への差し替え、
warrant の強制）** である。ADR-090 はこの A-2 について、ADR-172 が書いていない制約を既に定めている:

| ADR-090 の記述 | 場所 | ADR-172 での扱い |
|---|---|---|
| A-2 は「優先度 高（価値）／低（着手可能性）」「規模 **大**」「**実機ソーク必須**」「挙動変化 **有り（最大9通り）**」 | §3 優先順位表（行1640）/ §A.5（行531-532） | 記載なし。「低リスク」「実装時に詰める」（ADR-172 160-164行） |
| 「A-1 のログで `would_have_blocked` がゼロだった入口から順に」 | 行480 | 記載なし。shadow ログ取得が前提条件であることに触れていない |
| 「A-2 を入口ごとに分割し、`try_force_on_bootstrap` を最後に回す」 | A-R1（行519） | 記載なし（B4 参照） |
| 「A-2 の各ステップに `docs/known-bugs.md` 追記か golden 更新を必ず添える。revert 時は experiment-logging の3点を書く」 | A-R6（行524） | 記載なし |
| 「A-1 の実機ソーク（item 23）は**未実施**」 | 行1638 | 記載なし |

**ADR-172 の決定2 は、ADR-090 が既に「大・ソーク必須・挙動変化9通り」と評価した作業を、
「ADR-087 が元々計画していた配線作業の完了そのもの」と言い換えて軽く見せている。**
frontmatter の `related_adr` にも ADR-087 / ADR-090 が無い。最低限:

- `related_adr` に `ADR-087` と `ADR-090` を追加
- 決定2 の本文を「ADR-090 A-2 を force-on 入口に限定して実施する」という枠組みで書き直し、
  A-2 の既存制約（入口ごと分割・shadow ログ前提・実機ソーク・証跡添付）を継承すると明記

### B4. 決定2の対象が2箇所あるのに1箇所しか書いていない。書かれていない方が「最大の挙動変化」

`is_eligible_for_ime_force_on()` の呼び出し元は2箇所ある（関数自身の doc、
`state/platform_state.rs:769-772` にも明記）:

- `runtime/mod.rs:949` — `apply_force_on_for_imm_broken`
- `runtime/mod.rs:1234` — `try_force_on_bootstrap`

ADR-172 は前者しか挙げていない（109-110行「force-onの呼び出し経路
（`runtime/mod.rs::apply_force_on_for_imm_broken`）にはまだ配線されていない」）。

後者については、**コード側に既に警告コメントが置かれている**（`runtime/mod.rs:1277-1281`）:

```rust
// ADR-090 §2.A A-1（shadow）。**この入口は差分オラクルが
// 「判明した中で最大の挙動変化」と記録している old-1 そのもの**
// （`ImmCross` は `default_feedback = Read` なので Step 4c が
// 発火せず、観測も意図も guard も無い bootstrap では warrant が
// `None` になる）。A-2 で倒すのは**最後**に回すこと（ADR-090 §4.9）。
```

差分テスト側の記述も同じ（`state/open_warrant.rs:1186-1192`）:

> 1. `policy=ImmCross`・観測/意図/guard 一切無し・`desired_open=true`
>    （`try_force_on_bootstrap` 相当、1件）: …**Phase 3 実配線で ImmCross の bootstrap force-ON
>    経路が丸ごと無効化される、今回判明した中で最大の挙動変化**

ADR-172 は決定2 の適用範囲を **`apply_force_on_for_imm_broken` 1入口に限定する**と明記し、
`try_force_on_bootstrap` は対象外（ADR-090 A-R1 / §4.9 に従い最後）と書くこと。
現状の書きぶり（「`is_eligible_for_ime_force_on()` を `issue_open_warrant()` へ配線する」）は、
実装者が関数の両方の呼び出し元を機械的に差し替える読み方を許してしまう。

### B5. 決定2と ADR-151 の Blocker（force-on 永久停止）の関係が検討されていない

ADR-151 が案Dを不採用にした Blocker は観測手段の欠如ではなく、**「TsfNative における唯一の
ON 方向救済 `apply_force_on_for_imm_broken` が構造的に永久停止する」**である
（`docs/adr/151-*.md` 「決定（保留のまま）」節）。

決定2 はまさにその救済機構のゲートを差し替える変更であり、B1/B2 で示したとおり
**発火しなくなるケース（old_only 8件のうち force-on 側に該当するもの、および B2 のシナリオ）と
新たに発火するケース（new_only 1件）の両方**を生む。ADR-090 A-R2（行520）が後者を明示している:

> **新だけが許可するケース（new_only 1 件）で、今まで force-ON しなかった状況で force-ON し始める。**
> `ConvOpenInference(false)` を authority フィルタで除外した結果 Step 4c が `desired_open=true` を採る

ADR-172 の「落としてはいけない既知シナリオ」（137-149行）には、この2種類のどちらも入っていない。
BUG-63 の再現条件（差分オラクル old_only-3、`ConvOpenInference` 単独での force-on eligibility）も
入っていない。決定2 が触るのは BUG-63 の原因パターンそのものなのに、チェックリストが BUG-63 を
挙げていないのは穴である。

チェックリストに最低限追加すべき項目:

6. 差分オラクル old_only 8件 / new_only 1件（`open_warrant.rs::differential_old_gate_vs_issue_open_warrant`）
   の内訳のうち、force-on 入口に該当するものを個別に実機挙動へ翻訳し、意図した変化か確認する。
7. BUG-63（「mise」→「くした」）: `ConvOpenInference` 単独での force-on を止めるのは意図した改善だが、
   止まった結果 ON 方向救済が別のシナリオで消えないこと。
8. ADR-151 Blocker: force-on の発火頻度が実機ソークで実質ゼロに落ちていないこと
   （落ちれば ADR-151 が拒否した状態と同じになる）。

### B6. 決定1「挙動不変の構造リファクタ」は成立しない（6箇所は渡す値が違う）

ADR-172 87-92行:

> まずこれを `send_eager_tsf_warmup` 一本の呼び出しに集約し、各呼び出し元は「呼ぶかどうかの条件」
> だけを渡す形にする。挙動を変えない純粋なリファクタであり…

6箇所が渡す `warmup_ime_on` は **同じ値ではない**。2系統に分かれる:

1. **Gated 系**（`platform.rs:305`, `:1427`, `:1451`、および `:642` が受け取る値の多く）:
   呼び出し元が `resolve_warmup_ime_on()` 経由の値を渡す。この関数は
   `check_drift_correction` と同一述語の `off_drift_active` ゲートを適用する
   （`state/platform_state.rs:593-657`、INV-B1'）。
2. **Actuated 系**（`platform.rs:1610`）: `WarmupImeOn::from_actuated(effective)`。
   **このゲートを通らない。** `platform.rs:1563-1570` のコメントが明示している:

   ```
   // この `warmup_ime_on` は `from_actuated`（実 actuation 直後の確定値）由来であり、
   // `resolve_warmup_ime_on` が課す `off_drift_active` ゲートを通らない——force-ON
   // （`apply_force_on_for_imm_broken`）が `SetOpen(true)` を適用した直後にも
   // ここを通るため、drift correction が OFF 方向へ送り続けている最中でも
   // 随伴 warmup（`VK_IME_ON`）が飛びうる。INV-B1' は**この経路には及ばない**、
   // 既知の限界（ADR-132「Phase 2」節参照）。
   ```

「単一の合流点へ集約し、呼び出し元は呼ぶかどうかの条件だけを渡す」形にすると、この
**Gated / Actuated の値の作り分けをどちらかに倒すことになり、挙動が変わる**。倒さずに
`warmup_ime_on` を引数で受け続けるなら、それは今の `send_eager_tsf_warmup`
（既に単一関数）そのものであって、集約すべきものが残らない。

加えて、ADR は **7つ目の入口 `latch_eager_warmup_without_send`**（`output/mod.rs:1150-1156`）に
触れていない。これは `platform.rs:1610` の else 分岐（`should_send_accompanying_warmup(outcome)`
が false のとき）で呼ばれ、`send_eager_tsf_warmup` と同じ `eager_tsf_warmup_inner` に
`send_vk=false` で入る。ADR-149 M3 の doc（`output/mod.rs:1144-1149`）が、この latch は
`compute_focus_probe_grace` の唯一の入力 `eager_warmup_sent_ms` の供給元であり、丸ごと
スキップすると focus probe の grace が短縮される、と記録している。集約設計はこの分岐を
保存しなければならない。

またガードテスト `crates/awase-windows/tests/architecture_guard.rs:4192` が
`send_eager_tsf_warmup` へ渡す `origin`（`"gated"`/`"actuated"`）の区別を固定している。
集約で origin を単一化すると、このガードも同時に緩めることになる（＝診断可能性の後退）。

**決定1 を残すなら、「挙動不変」という主張を撤回し、「Gated/Actuated の作り分けをどう扱うか」を
決定の本体として書くこと。** ADR の「他系統との関係を論じる前提条件として先に片付ける」という
位置づけは、この作り分けこそが force-on × drift correction × warmup の相互作用の核心
（INV-B1' が及ばない経路）である以上、逆である。

---

## Should-fix

### S1. 決定1の呼び出し元6箇所の所在が実測と違う

ADR 37行の表: 「呼び出し元6箇所: `platform.rs`4箇所、`output/vk_send.rs`、`runtime/ime_refresh.rs`」

実測（`grep -rn "\.send_eager_tsf_warmup(" crates/awase-windows/src/`）:

```
crates/awase-windows/src/platform.rs:305
crates/awase-windows/src/platform.rs:642
crates/awase-windows/src/platform.rs:1427
crates/awase-windows/src/platform.rs:1451
crates/awase-windows/src/platform.rs:1610
crates/awase-windows/src/output/vk_send.rs:692
```

= `platform.rs` **5箇所** + `output/vk_send.rs` 1箇所。`runtime/ime_refresh.rs` からの直接呼び出しは
**ゼロ**（`platform.rs:299-306` の `send_eager_warmup` ラッパーを経由する。そのラッパーの doc が
「唯一の呼び出し元（`ime_refresh.rs` の FocusChange 処理）」と書いている）。総数6は偶然合っている。
表を実測に直すこと。

### S2. 決定3の論拠が弱く、かつ reassert が「既に A-2 済みの唯一の入口」である事実を落としている

ADR 123-129行は reassert を「周期tickではなくイベント駆動、根本原因も未解明」だから
別カテゴリにする、としている。しかし `runtime/mod.rs:1189-1196`:

```rust
// ADR-090 §2.A A-1（shadow）+ D3: `force_on_and_correct_romaji` と
// 同じく `ActuationOrder` を起案し、`would_have_blocked()` なら送信
// しない（A-2 の最初の限定的インスタンス）。
let order = self.issue_actuation_order(open, "explicit_key_reassert");
if order.would_have_blocked() {
    tracing::debug!("[explicit-reassert] would_have_blocked のため見送り (open={open})");
    return;
}
```

**reassert は4系統の中で唯一、既に warrant を強制している入口である。** つまり:

- 「発火の性質が違うから別カテゴリ」は、warrant ゲートの観点では逆の結論を支持する
  （reassert は既に決定2 が force-on に導入しようとしているものを持っている）
- 決定2 のロールアウト手順は、reassert という**既存の前例**を参照して書くべきである
  （どういう順序で入れたか、実機で何を観測したか、`would_have_blocked` の発火頻度はどうだったか）

決定3 を「別カテゴリとして記述し直す」だけで終わらせるのはもったいない。少なくとも
「reassert は A-2 済み、force-on/drift correction は A-1 shadow のまま」という**現在地の差**を
表に追記すること。ADR 33-38行の表に「warrant の強制状態（A-1 shadow / A-2 強制）」列を足すのが
最小の修正。

### S3. レビュー観点3（同tick衝突）が本文で検討されていない。実際に同tickで走る経路がある

reassert が settle で見送られた分は latch され、**`TIMER_IME_REFRESH` の tick 上で消費される**
（`runtime/message_handlers.rs:508-526` の `peek_pending_explicit_reassert` →
`disarm_pending_explicit_reassert`）。同じ tick で `ir_stage_notify` が走り、
force-on → drift correction を連続実行する（`runtime/ime_refresh.rs:226-235`）:

```rust
fn ir_stage_notify(&mut self) {
    self.apply_force_on_for_imm_broken();   // Phase 4a
    self.ir_notify_engine_refresh();        // Phase 4
    self.ir_apply_drift_correction();       // Phase 4b
    self.reschedule_ime_refresh();          // Phase 5
}
```

したがって「reassert と force-on が同一 hwnd・同一 tick で発火する」は仮定ではなく実在の配置である。
決定3 が「相互作用を無視してよい」と言うなら、この配置を明示した上で無視してよい理由を書くこと。

無視してよくない具体的な理由が1つある: **3者は同じ `issue_actuation_order` を呼ぶが、それぞれ
独立に `Instant::now()` を取る**。`ImeStateHub::warrant_context()`（`platform_state.rs:859-874`）は
`now` / `now_ms` を引数で受け取り、呼び出し元がその都度確定させる設計であり、tick 内で共有される
スナップショットは無い。同型の限界は既に `resolve_warmup_ime_on` の doc に「`now` はバッチ内で
一貫しない（既知の限界、敵対的コードレビュー指摘）」（`platform_state.rs:583-592`）として
記録されている。決定2 で force-on 側も warrant 判定に依存するようになると、
**同一 tick 内で reassert が warrant あり・force-on が warrant なし（または逆）** という
評価の食い違いが `DRIFT_CORRECTION_THRESHOLD_MS` 等の境界跨ぎで起こりうる。
ADR で「許容する既知の限界」と書くか、`batch_now` の導入を決定に含めるかを決めること。

### S4. 決定2の配線先（ゲートかチェーンか）が未確定のまま「実装時に詰める」に丸投げされている

ADR 161-164行:

> 2. 決定2（`is_eligible_for_ime_force_on()` を `issue_open_warrant()` へ配線）は対象箇所を
>    特定済み。…`force_on_attempt_allowed` のクールダウン/`applied` 判定とどう組み合わせるか
>    （`issue_open_warrant()` が全面置換なのか、追加の必要条件として併用するのか）は実装時に詰める。

選択肢は実際には2つあり、**どちらを選ぶかで ADR-090 との整合性が変わる**ので、実装時ではなく
ADR で決めるべきである。

- **(a) ゲート差し替え**: `runtime/mod.rs:949` の `is_eligible_for_ime_force_on()` を
  `issue_open_warrant(true, ..).is_some()` に置き換える。
  → このとき `force_on_and_correct_romaji` は `:1035` で**もう一度** `issue_actuation_order`
  を呼ぶ。同一の force-on 1回に対して warrant を2回、**別々の `Instant::now()`** で評価することになる
  （`:967` の `now_ms` と `:1035` 内部の now は別々に取られる）。2回目だけが `would_have_blocked`
  を journal に載せるので、**ゲートで止めた分は journal に一切残らない**——実機ソークで
  「止まった回数」を数える手段が消える。A-2 の前提（shadow ログで発火頻度を測る）と噛み合わない。
- **(b) チェーン側で強制（ADR-090 の設計）**: `into_actuation_shadow` → `into_actuation`
  へ差し替える（ADR-090 §A.6 A-2' 行603 が「差し替えが型として見える」と書いている形）。
  → ゲート `is_eligible_for_ime_force_on()` はそのまま残り、`effective_open()` 依存も残る。
  ADR-172 が問題視している「BUG-63 の原因パターンが実 actuation ゲートとして今も本番で使われている」
  という自己申告コメント（`platform_state.rs:764-772`）は**解消しない**。

つまり (a) は ADR-090 の手順と衝突し、(b) は ADR-172 の動機を満たさない。この緊張関係こそが
決定2 で解くべき問題であり、ADR 本文に書かれていない。

### S5. 決定4の「観測手段の探索は閉じた」は、検討範囲が mozc IPC × TSF in-proc API に閉じている

ADR 52-71行が検討したのは (1) mozc IPC 能動問い合わせ、(2) 同 受動傍受、(3) `ITfUIElementMgr`
/ UIA `ITextEditProvider` の3方向。**次の2つが検討対象に入っていない。**

1. **`ImmGetDefaultIMEWnd` + `WM_IME_CONTROL(IMC_GETOPENSTATUS)`**。これはクロスプロセスで
   IME open 状態を読む古典的経路で、**このリポジトリには既にスパイクが存在する**:
   `crates/awase-windows/examples/spike_egui_ime_control_probe.rs`（`:13-36` の doc が
   `ImmGetDefaultIMEWnd(hwnd)` → `SendMessageTimeoutW(ime_wnd, WM_IME_CONTROL, IMC_GETOPENSTATUS, ..)`
   の検証手順を書いている）。「TsfNative では不成立」なら、そのスパイクの実測結果を ADR に
   引用して閉じること。実行していないなら、決定4 の「調査は閉じた」は成立しない。
   （関連: `spike_bug112_ime_wnd_race_probe.rs` も `ImmGetDefaultIMEWnd` を使っている）
2. **awase 自身が既に持っている I/O を Actuating 観測として記録する**（レビュー観点4 の
   「awase 自身の観測タイミングの改善」に相当）。B1 で示したとおり、`ObservationSource::Gji` /
   `Tsf` は `authority() == Actuating` として型が用意されているのに、**本番の記録点がゼロ**
   （`state/evidence.rs:126-134` で宣言のみ）。GJI 戦略は実際には GJI との I/O を観測しており
   （`GjiIoInference` という別ソースが存在する）、その一部を `Gji`（Actuating）として
   record できるなら、TsfNative の Actuating プールが構造的に空という現状（B1）が変わる。
   これは**新しい外部 API を一切足さない観測改善**であり、「観測手段の探索は閉じた」という
   決定4 の結論を直接揺るがす。

決定4 を「打ち切り」として確定させるなら、上記2つを「検討して不成立だった」または
「スコープ外として別issue化する」と明記すること。現状の書きぶりだと、次にこの領域へ戻る人が
「外部 API 経路は全部調べ尽くされた」と読んでしまう。

### S6. ADR-151 の引用元の記述が一部陳腐化している

決定4 が依拠する ADR-151 の限界効用評価には次の一文がある:

> さらに、この設計を採用しても随伴 eager warmup（`platform.rs` の `send_eager_tsf_warmup` 呼び出し）は
> `outcome` を見ないため、送信2回が別途残ってしまう

これは現在のコードでは正しくない。`platform.rs:1605-1615` は
`awase::platform::should_send_accompanying_warmup(outcome)` で分岐し、送らない場合は
`latch_eager_warmup_without_send` を呼ぶ（ADR-149 の決定 + ADR-167）。ADR-172 が ADR-151 の
判断を引き継ぐなら、この前提が既に実装済みであることを注記すること
（ADR-151 自身も「それを先に入れた時点で残り送信は1回になり」と条件付きで書いているので、
条件が満たされた今、限界効用の評価が変わっていないかを1行で確認するだけでよい）。

---

## Nice-to-have

- **N1.** frontmatter `related_adr` に `ADR-087`（`issue_open_warrant` の設計）と
  `ADR-090`（A-1/A-2 のロールアウト計画）が無い。決定2 の直接の親なので追加する。
  `ADR-132`（INV-B1'、warmup ゲート）も決定1 に関係する。
- **N2.** 「次のアクション」（158-167行）の粒度が実装可能水準に達していない。決定2 について
  最低限必要な項目: (i) 対象入口を `apply_force_on_for_imm_broken` 1本に限定すると明記、
  (ii) A-1 shadow ログ（`would_have_blocked` の実発火頻度）を先に取る、
  (iii) 差分テスト `EXPECTED_OLD_ONLY_COUNT=8` / `EXPECTED_NEW_ONLY_COUNT=1` を動かすのか
  据え置くのか、(iv) 実機ソークの対象（アプリ × IME × idle 条件）、
  (v) `.claude/rules/fix-requires-evidence.md` の「キー選択」ファミリー該当なので
  golden 更新か `docs/known-bugs/BUG-NNN.md` のどちらを添えるか。
  (i)〜(v) は ADR-090 A-R1 / A-R6 が既に要求している内容なので、新規に考える必要はない。
- **N3.** 決定1 の効能が書かれていない。「他系統との関係を論じる前提条件」とあるが、
  具体的に今どの議論がブロックされているのかが読み取れない。B6 を踏まえると、
  集約の真の目的は「Actuated 経路に INV-B1' が及んでいない」ことの解消であるはずで、
  それなら決定1 は構造リファクタではなく挙動変更の決定として書き直すべき。
- **N4.** ADR 43-45行「`architecture_guard.rs` が `.apply_ime_open_with_view(` の呼び出し件数を
  固定値でガードしており」— 現在の固定値は 4
  （`crates/awase-windows/tests/architecture_guard.rs:1256`）。決定2/決定1 がこの件数を
  動かすかどうかを明記しておくと、実装時の手戻りが減る。
- **N5.** ADR 141-142行「BUG-113: …（現状3回→2回、残り1回はwarmup由来として既知）」は
  ADR-149/ADR-167 の適用後の値だと思われるが、B6 で触れた `should_send_accompanying_warmup`
  分岐が入った後の実測なのか、入る前の見積もりなのかが読み取れない。決定1 がこの分岐を
  触る以上、どちらかを明記すること。

---

## レビュー観点への直接回答

1. **決定2 は本当に安全か** → いいえ。差し替え自体が技術的に可能かという以前に、
   **ADR が書いている効能（`HeuristicDefault` 除外の共有）が達成されない**（B1）。
   `force_on_attempt_allowed` との衝突は起きない（クールダウンは `applied` ベースで warrant と
   直交する）が、配線先をゲートにすると warrant 評価が同一呼び出し内で2回・別時刻になる（S4）。
   差分テスト `differential_old_gate_vs_issue_open_warrant` は**このユースケースを正しく
   カバーしている**（`old_is_eligible_for_ime_force_on` が本番と同じ
   `is_japanese_ime && effective_open_at(now)` を再現し、old_only 8 / new_only 1 を方向別に固定）。
   ただしカバーしているのは「判定の一致/不一致」だけで、`force_on_attempt_allowed` の
   クールダウンや `note_force_on_attempt` の副作用は次元に入っていない。
2. **意図しない退行を生まないか** → 生む。`FocusProbe` が actuation 根拠から外れる（B2）。
   これは drift correction 側で「BUG-16/BUG-20 型の固着を再導入するリスク」として明示的に
   拒否された変更であり、force-on 側で同じ判断が成り立つ保証はない。加えて new_only 1件で
   **今まで force-on しなかった状況で force-on し始める**（ADR-090 A-R2）。
3. **決定3 は本当に「別カテゴリ」で済むか** → 結論（統合対象に含めない）は妥当だが、
   理由が間違っている。reassert は既に warrant を強制している唯一の入口であり（S2）、
   settle 見送り分は force-on/drift correction と**同じ tick で消費される**（S3）。
   「相互作用を無視してよい」ではなく「reassert は既に A-2 済みなので、決定2 は reassert に
   追いつく変更である」と位置づけ直すのが正確。
4. **決定4 は打ち切ってよい水準か** → 現状では不十分。`WM_IME_CONTROL(IMC_GETOPENSTATUS)`
   経路（リポジトリ内に既存スパイクあり）と、`ObservationSource::Gji`/`Tsf` という
   Actuating 観測源が型だけあって本番 writer ゼロという事実（＝新 API なしの観測改善余地）の
   2つが検討されていない（S5）。
5. **opus-adversarial-consult 前の完成度として十分か** → **不十分。** B1/B2 は「レビュアーの
   判断材料が間違っている」類の誤りで、このまま次工程へ渡すとレビュアーが誤った前提を
   引き継ぐ（過去に round1 の誤った数値が round2 にそのまま転記された前例がある）。
   B1〜B6 を反映し、特に決定2 を「ADR-090 A-2 の force-on 入口限定実施」という正しい枠に
   置き直してから opus-adversarial-consult へ進めること。決定3・決定4 は S2/S5 の修正で足りる。
