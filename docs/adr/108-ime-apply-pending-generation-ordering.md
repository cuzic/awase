# ADR-108: IME apply 完了の受理判定を「pending 一致」から3つの独立した問いへ分解する

## ステータス

**提案（未実装、2026-08-28、r5）。** 初版（r1）の決定1（`last_confirmed_generation` による
単調性チェックへの緩和）と決定2（`clear_pending_if_matches` の generation 一致化）は、
Opus によるアドバーサリアルレビューで **Critical 3 件**（うち 1 件は「ADR が前提としていた
コードの事実が誤っていた」）を指摘され、実コード確認で **3 件とも成立することを確認した**
ため **全面的に撤回**した。r2 は同じ実害（stale 完了の取りこぼしによる `applied` の固着）に
対する別設計であり、r1 とはデータ構造も判定軸も共有しない。

r2 の再レビューでは r1 由来の 16 件中 15 件の解消が確認された一方、**r2 の決定3
（`applied` 書き込みの `reduce()` 一本化）が新たに Critical 1 件を生んでいる**ことが判明した
——`reduce()` 冒頭のタイムアウト遅延パージが、正当な完了の `applied` 更新を先回りして
消してしまう。r3 は決定4（パージ順序）を新設してこれを閉じた。

r3 の再レビューでは、r2 で導入し r3 で解放規則を足した `superseded` スロットに
Critical（`ImeApplyFailed` が `superseded` を過剰に破棄し、ADR 自身の主目的シナリオを
打ち消す）が見つかった。**本 r4 は、その 1 行を直すのではなく `superseded` スロットを
丸ごと廃止する。** 理由は「決定」節の冒頭に書いたが、要点は 2 つ:

1. 指摘の 1 行削除では**当のシナリオは直らない**（`pending` が解決済みなら、受理条件が
   比較すべき target がそもそも存在しない）。正しく直すには解放条件を outcome 別に
   場合分けする必要があり、`superseded` の解放セマンティクスへの修正はこれで 3 回目
   （r2 で導入 → N5 で解放規則を追加 → R1 で過剰解放を削除 → さらに `UnsafeToToggle` の
   例外が必要）になる。
2. その過程で、**`superseded` の generation 照合が安全性に何も寄与していない**ことが
   分かった。受理を安全にしているのは epoch 一致と「現在の `pending` と同じ target に
   着地する」ことであって、どの generation の完了かではない。

結果として、r4 の決定は r3 から**削る方向**の変更である（フィールド 1 本、overflow warn、
`FocusChanged` での追加クリア、2 スロット分のパージ、失敗系の解放規則、generation
一意性 INV とその回帰テストが消える）。r3 の骨格のうち決定1（epoch）・決定3（型分離）・
決定4（パージ順序）・決定6（適用範囲）はそのまま維持している。

r4 の再レビューでは**新規 Critical はゼロ**であり、残った指摘（S1〜S6）は設計変更を
伴わない追記・訂正だった。**本 r5 がそれらを反映した最終版である**: `applied` 消費箇所の
母集合を 5 → 7 に訂正し（S3）、決定2 が閉じない範囲（`pending` 解除後に届く旧完了、S1）と
書いた `Optimistic` の寿命（S2）を明示、bootstrap の epoch 説明を実コードに合わせて
訂正した（S6）。決定 1〜6 の内容は r4 から変わっていない。

発端は `/code-review`（medium effort、8 観点並列 Finder + 手動検証）による `develop`
過去 1 週間差分のレビューで、Angle A（correctness line-by-line）と Angle Altitude
（altitude/deferred-fix）が**互いに独立に同一箇所**を指摘した点。対象は
`state/ime_model.rs::reduce()` の `ImeApplyRequested` アームで、著者自身が「拒否ではなく
警告ログのみ…実機で実際の発生頻度を確認してから拒否に倒すかどうかを判断する」という
**未決のまま**保留していた箇所（2026-08-19、BUG-34 横展開 D-prep）。

## コンテキスト

### 現状の実装

`state/ime_model.rs::reduce()` は、進行中の（未タイムアウトの）`pending` がある状態で新しい
`ImeApplyRequested` を受けると、警告ログを出すだけで `pending` を無条件に新しい
`ImeTransition` で上書きする（`crates/awase-windows/src/state/ime_model.rs:576-597`）:

```rust
if let Some(existing) = &self.pending {
    if !existing.is_timed_out(envelope.time.monotonic) {
        log::warn!(
            "[ime-model] ImeApplyRequested(generation={generation}, target={target}) \
             が進行中の pending(generation={}, target={}) を上書きする — \
             その apply の完了が今後 stale 判定される可能性がある",
            existing.generation, existing.target
        );
    }
}
self.pending = Some(ImeTransition { target, generation, timeout_at: .. });
```

上書きされた古い generation の完了は、`ImeStateHub::record_ime_apply_result`
（`crates/awase-windows/src/state/platform_state.rs:831-882`）の入口で例外なく捨てられる:

```rust
if let Some(generation) = generation {
    let pending = self.shadow_model.pending_generation();
    if pending != Some(generation) {
        log::debug!("[ime-apply] stale completion ignored: ...");
        return false;   // ← applied 更新も on_ime_applied も両方まとめて失われる
    }
}
```

完了に付く `generation` は、送信時に `ime.model().pending_generation()` を読んで
タグ付けしたものである（`runtime/executor.rs:225` / `:269` / `:298` / `:455`）。
つまり **generation は「送信時点の pending」を指す参照であり、絶対的な時刻ではない**。
この事実が後述の決定の土台になる。

### 何が壊れているか（r1 から変更なし）

`state/transition.rs` の module doc が挙げる T1/T2/T3 の例（`apply true gen=10` →
`user intent false gen=11` → `apply true succeeded gen=10` は無視）は、「ユーザーが考えを
変えたので古い apply の結果を捨ててよい」場面を正しくモデル化している。しかし `pending`
が上書きされる場面はそれだけではない。同一 target への即時リトライ（force-on の連続
リトライ等）でも `ImeApplyRequested` は発火する。この場合:

1. generation 10 の apply が OS に送信され、まだ完了していない。
2. generation 11 の `ImeApplyRequested`（**同じ target**）が発行され、`pending` が
   generation 11 に上書きされる。
3. generation 10 の実際の OS 側完了が後から届く。generation 一致チェックにより
   `applied` は一切更新されず、generation 10 が要求される**前**の値のまま凍結される。
4. generation 11 自身の完了が届かない経路（呼び出し元のガード、`UnsafeToToggle` 等）に
   入ると、`applied` は `IME_APPLY_PENDING_TIMEOUT_MS`（8000ms）のパージまで古いまま残り、
   `desired_open` との乖離が解消されず drift correction が再送を繰り返す。

`applied`（`AppliedImeState`）は診断用の傍観フィールドではなく、以下の実処理の入力である
（**r1 は 3 箇所しか挙げていなかったが、実際には 7 箇所ある** — 指摘 M1 / S3 を反映）:

| 消費箇所 | 何に使われるか |
| --- | --- |
| `runtime/ime_refresh.rs:479`, `:527` | フォーカス変更時の warmup 戦略選択と「新ウィンドウで IME OFF を強制するか」 |
| `runtime/executor.rs:733` | engine key の gating（`applied_for_engine_key`） |
| `runtime/message_handlers.rs:635` | GJI warmup FSM を belief と同期させる判断 |
| `state/ime_actuation.rs:245-258` | `force_on_attempt_allowed` — force-ON を送ってよいか（ADR-098 決定1-c） |
| `state/platform_state.rs:527-541` | `resolve_warmup_ime_on` / `warmup_ime_on()` — eager warmup の `ime_on` 入力（`applied` 既知なら belief より優先） |
| `runtime/ime_refresh.rs:296-300` | `explicit_verify` — 打鍵中でも OsPoll を先行させるか（`applied != Unknown` を見る） |
| `output/ime_apply_planner.rs:80-91` | `reduce_open_belief` の `confident` 計算。**`Optimistic` と `Confirmed` を区別する唯一の消費箇所**（`is_confirmed()` + `confirmed_at_ms()` で「300ms 以内の Confirmed か」を見る） |

つまり `applied` が古いまま固定されると、warmup 戦略・engine gating・force-ON 予算の
すべてが「数世代前の状態」を見て判断し続ける。`.claude/rules/ime-belief-architecture.md`
が繰り返し警告する belief 破損パターンそのものである。

### r1 のレビューで判明した、より重い前提の誤り

r1 は「これは (a) pending 解決の問いと (b) OS 実状態の問いを 1 つのチェックで答えている
のが原因」と整理したが、実コードを追うと **問いは 3 つある**。r1 が (b) に押し込めていた
ものが、実は互いに独立な 2 つだった。

| 問い | 何をゲートするか | 現状の判定 |
| --- | --- | --- |
| (a) この完了は「今追跡中の apply」を解決するか | `pending` の clear（drift correction の再送判断） | generation 厳密一致（`generation=None` 経路は target 一致） |
| (b) この完了は `applied` を更新してよいか | warmup 戦略・engine gating・force-ON 予算・enforce-OFF | 同じ generation 厳密一致 |
| (c) この完了は composition/warmup の**副作用**を駆動してよいか | `on_ime_applied` → `mark_composition_cold` + `send_eager_tsf_warmup` + `ImeModeFsm` | 同じ `bool` 戻り値 |

(c) の存在が r1 の致命的な見落としだった。`record_ime_apply_result` の戻り値は
ログ用ではなく `runtime/mod.rs:482` の

```rust
let accepted = self.platform_state.ime.record_ime_apply_result(open, outcome, generation, ts);
// B: composition warm/cold 更新。stale apply 完了は GJI/Composition に伝播させない。
if accepted {
    self.platform.on_ime_applied(open, outcome);
}
```

というゲートであり、`on_ime_applied`（`platform.rs:1156-1240`）は **`effective` ではなく
要求 target の `open` で分岐して** `feed_composition_event(ImeOn|ImeOff)` →
`mark_composition_cold(SetOpenTrue)` → `send_eager_tsf_warmup()` を走らせる。
r1 の決定1（単調性だけで受理）は、この (c) まで巻き添えで緩めていた。

### `applied` は「履歴」ではなく「現在の OS 状態」として読まれている

r1 は (b) を「OS が実際に何をしたか、という履歴的事実だから単調性で十分」と価値づけたが、
実際の消費側はいずれも `applied` を**今この瞬間の OS 状態**として読む
（`ime_refresh.rs:527` の `if !applied_ime_on && !new_profile_is_tsf_native { set_ime_open(false) }`、
`ime_actuation.rs:250-257` の force-ON スパムガード）。より新しい generation が
**逆 target で in-flight** なら、古い generation の結果は「現在値」としては誤りである。
単調性チェックはこの区別を構造的に持てない。

### `applied` は既にフォーカスを跨いで汚染されうる（本 ADR で判明した既存バグ）

`reduce()` の `FocusChanged` アーム（`ime_model.rs:538`）は `applied = Unknown` に
リセットするが、**同じアームで `pending` はリセットしない**。したがって現行コードでも:

1. ウィンドウ A で generation 10 の apply を送信（`pending = gen10`）。
2. フォーカスが B へ移り `applied = Unknown` にリセットされる。`pending` は gen10 のまま。
3. A 由来の gen10 完了が遅れて到着 → generation 厳密一致を**通過** →
   `record_confirmed(true, ts)` → `applied = Confirmed{open:true}`。
4. `force_on_attempt_allowed`（`ime_actuation.rs:245-258`）が `Confirmed{open:true}` で
   `false` を返し、**B での force-ON が封鎖される**。

ADR-098 決定1-a は「フォーカス入場後 `applied` を `Unknown` のまま残す」ことを
load-bearing な設計にしており（`ime_actuation.rs:241-246` の doc）、これが BUG-16 の修正を
TsfNative で初めて実効させている。上記はその不変条件を旧ウィンドウの完了が破る経路であり、
**r1 の決定1 が作る新規バグではなく、既に存在する欠陥**である（レビュー指摘 C2 は
「決定1 が作る」と帰属していたが、実コード確認の結果、原因帰属を訂正した上で本 ADR の
対象に含める）。

### `generation=None` の完了経路は実在し、target 一致解除に依存している

r1 の決定2 は「`clear_pending_if_matches` は generation 一致チェックを通過した後にしか
呼ばれないため実害はない」と書いたが、**これは事実誤認である**。
`record_ime_apply_result` は `generation: Option<u64>` を取り、`None` のときは一致
チェックを**完全にスキップ**する（`platform_state.rs:832`）。`None` で呼ぶ経路は 5 つ実在する:

| 経路 | 位置 |
| --- | --- |
| idle-conv-check の DirectInput 分岐 | `runtime/key_pipeline.rs:755` |
| shadow toggle OFF（async / sync） | `runtime/key_pipeline.rs:1012` / `:1025` |
| non-ImmCross drift correction | `runtime/ime_refresh.rs:800` |
| force-on（`force_on_and_correct_romaji`） | `runtime/mod.rs:830` |

特に `key_pipeline.rs:733` は `allocate_event_generation()` →
`handle_engine_set_open(target=false, .., generation=N)` で **generation N の pending を
立てた直後**に `:755` で `on_ime_apply_complete(false, outcome, None, ..)` を呼ぶ。
このとき pending N を解放する唯一の手段が `clear_pending_if_matches(false)` の
target 一致である。r1 の決定2（generation 一致化）を入れると pending N は 8000ms 固着する。
**r1 が「実害はない」と切り捨てた経路が、実は現行の正常動作を支えていた。**

### `generation` はグローバル一意ではない

r1 は「`generation` は apply 種別を跨いで単調増加することが既に保証されている」と書いたが、
`allocate_event_generation`（`platform_state.rs:688-690`）は `event_log.next_seq()` を
**返すだけで increment しない**。カウンタを進めるのは `ImeEventLog::record()`
（`state/ime_event_log.rs:46`）だけである。`handle_engine_set_open` は chord フィルタ／
focus-settle フィルタで**何も dispatch せずに `false` を返す**早期 return を 2 つ持つ
（`platform_state.rs:244-277`）ため、その場合は次の allocate が同じ値を返す。
また `ImeApplyRequested` の `generation` は生の `u64` であり、`state/event_origin.rs:60` の
`Generation` 型とも別物である。**generation の大小比較を belief 判定の根拠にしてはならない**
（r1 の決定1 はまさにそれをしていた）。

## 決定

### 決定0（設計原則）: 3 つの問いを 3 つの判定に分解する

`record_ime_apply_result` は 1 つの `bool` で (a)(b)(c) 全部に答えるのをやめる。

- (a) `pending` の解除 → **現状維持**。generation 付きは厳密一致（`reduce()` の
  `ImeApplySucceeded`/`ImeApplyFailed` アーム）、`generation=None` は target 一致
  （`clear_pending_if_matches`）。**r1 の決定2 は撤回する。**
- (b) `applied` の更新 → 決定1（epoch ゲート）+ 決定2（意図と一致する完了の条件付き受理）。
- (c) `on_ime_applied` の駆動 → 決定3（型で (b) と分離し、緩和は一切しない）。

3 つの判定が同一の `pending` スナップショットを見ることは、決定4（タイムアウトパージの
順序）で構造的に保証する。r2 はここを暗黙の前提にしていたため (b) と (c) が食い違う窓を
作っていた。

**新しい状態を一切増やさない。** r1 は `last_confirmed_generation`（グローバル単調
カウンタ）を、r2/r3 は `superseded: Option<ImeTransition>`（退避スロット）を追加していたが、
r4 はどちらも持たない。追加するのは既存 `ImeTransition` へのフィールド 1 本
（`focus_epoch`、決定1）だけである。

この単純化は、r3 のレビューで判明した次の事実に基づく: **上書きされた apply の完了を
受理してよいかは、その完了の generation とは無関係に決まる。** 決めているのは
(1) 同じフォーカスプロセスか、(2) その完了が報告する値が「今 in-flight な apply が
向かっている先」と同じか、の 2 点だけであり、どちらも現在の `pending` だけを見れば
判定できる。したがって「上書きされた transition を覚えておく」という機構そのものが不要
だった（詳細は決定2、経緯は「r3 レビュー指摘との対応」R1 を参照）。

`generation` の**等値**比較は厳密一致経路（(a)(c)）でのみ使う。大小比較はどこでも
使わない——r1 の `last_confirmed_generation` は「generation は一意ではない」と両立しない。

### 決定1: `ImeTransition` に `focus_epoch` を持たせ、フォーカスを跨いだ完了は `applied` を書けなくする

`state/transition.rs` の `ImeTransition` に `focus_epoch: crate::state::probe_admission::FocusEpoch`
を追加する。`reduce()` の `ImeApplyRequested` アームで、
**既に `ImeModel` が保持している** `self.observations.current_focus_epoch`
（`state/observation_store.rs:303`、`FocusChanged` アームの `clear_on_focus_change` で更新）
をスタンプする。**新しい event フィールドも新しいカウンタも増えない。**

```rust
self.pending = Some(ImeTransition {
    target,
    generation,
    timeout_at: envelope.time.monotonic
        + Duration::from_millis(crate::tuning::IME_APPLY_PENDING_TIMEOUT_MS),
    // 決定1: この apply が「どのフォーカスプロセスに対して」送られたかを刻む。
    focus_epoch: self.observations.current_focus_epoch,
});
```

**粒度について（指摘 N6）。** `focus_epoch` が進むのは
`focus_tracking.rs:368`（`on_focus_process_changed`）と `:127`（bootstrap）の 2 箇所だけで、
**プロセスが変わったとき**にしか増えない。同一プロセス内のウィンドウ跨ぎは保護されない。
ただしこれは欠陥ではなく**要件との一致**である: 保護対象である
`reduce()` の `applied = Unknown` リセットは `ImeEvent::FocusChanged` アームで起き、
その `FocusChanged` を dispatch しているのは**同じ `on_focus_process_changed` だけ**
（`focus_tracking.rs:380-388`）。つまり epoch が進む単位と `applied` がリセットされる
単位は定義上一致しており、「`applied = Unknown` を旧完了に破らせない」という目的に対して
過不足がない。同一プロセス内のウィンドウ跨ぎでは `applied` もリセットされないので、
そもそも守るべき不変条件が存在しない。実装時の doc コメントは「フォーカスプロセス」と
書き、この対応関係を明記すること。

**同名で値の違う 2 つのカウンタに注意（指摘 R3）。** `focus_epoch` を名乗る値は 2 つある:

| 値 | 更新される場所 |
| --- | --- |
| `platform_state.focus.focus_epoch` | `on_focus_process_changed`（`focus_tracking.rs:368`）**と** `establish_initial_focus_scope`（`:127`）。`ImmLikeTicket` が使うのはこちら |
| `observations.current_focus_epoch` | `ImeEvent::FocusChanged` の reducer 経由のみ（dispatch 元は `focus_tracking.rs:380-388` の 1 箇所） |

`on_focus_process_changed` の呼び出し元は `apply_focus_probe_result`（`:98`）の
1 箇所だけで、`establish_initial_focus_scope` は `advance_focus_tracking` を直接呼ぶ
（`:121`）ため**この経路を通らない**。つまり bootstrap は `focus.focus_epoch` を
`:127` で進めるが `FocusChanged` を dispatch しないので、`observations.current_focus_epoch`
は据え置かれ、2 値は最初の実フォーカス変更まで 1 ずれる（そこで再同期する）。決定1 は**スタンプも照合も `observations.current_focus_epoch` という同一の
カウンタで行う**ので、この差は決定の正しさに影響しない。ただし実装時に
`ImmLikeTicket` との相互参照 doc を書く際、**2 つを混ぜて使わない**こと
（`focus.focus_epoch` でスタンプして `current_focus_epoch` で照合する、またはその逆は、
bootstrap 直後に恒久的な不一致を作る）。

完了が `applied` を書けるのは `transition.focus_epoch == self.observations.current_focus_epoch`
のときだけとする。これは既存の `probe_admission::ImmLikeTicket`
（`state/probe_admission.rs:82`、非同期 probe が spawn 時の epoch を捕まえ、完了時に
`admit(current_epoch)` で照合して棄却する）と**同型の考え方を、observation 軸から
actuation 軸へ横展開する**ものである。`ImmLikeTicket` は「仮想デスクトップ切替中の経由
ウィンドウが返す false 観測」を構造的に排除するために導入された。ここで排除したいのは
「旧ウィンドウへの apply の完了が新ウィンドウの `applied` を書く」であり、問題の形が同じ。

`ImmLikeTicket` そのものを再利用しないのは、あちらが非同期クロージャへ move される
値型のチケットなのに対し、こちらは既に `ImeModel` の中に生存している `pending` に
フィールドを 1 本足すだけで済むためである（チケットを `runtime/` から `state/` へ
往復させる配線が不要）。判定ロジックの類似は doc コメントで相互参照する。

**これは r1 の緩和とは独立に、現行の既存バグ（前節）を閉じる。**

### 決定2: generation が一致しない成功完了でも、「今の pending と同じ target・同じ epoch」なら `applied` を `Optimistic` で更新する

**`ImeApplyRequested` アームは r3 から変わって、`pending` の上書きを従来どおり
「捨てる」だけに戻す**（`superseded` への退避はしない）。新規フィールドはゼロ。

`ImeApplySucceeded` が **`pending` の generation と一致しなかった**とき、
以下の **3 条件すべて**を満たす場合に限り `applied` を更新する:

1. 現在 `pending` が存在し、`pending.focus_epoch == observations.current_focus_epoch`（決定1）
2. `pending.target` が、この完了が運ぶ要求 `target` と等しい
3. `self.applied.applied_open() != Some(target)`（**既に同じ値なら何もしない**）

書き込む値は `AppliedImeState::Optimistic(target)` とする（`Confirmed` ではない）。

#### なぜ「どの generation の完了か」を見なくてよいのか

r3 は「上書きされた直前の 1 件」だけを `superseded` に退避して照合していたが、
**その照合は安全性に寄与していなかった**。`ImeApplySucceeded { target }` が届いた
という事実は「**ある** apply が `target` の適用に成功した」を意味し、それが何世代前の
要求だったかは、書き込む値を変えない。安全性を作っているのは条件 1 と 2 である:

- 条件 1 が「その完了は今と同じフォーカスプロセスの話か」を保証する（C2）。
- 条件 2 が「書き込む値は今 in-flight な apply が向かっている先と同じ」を保証する。
  したがって **target が反転した完了は必ず弾かれる**——「ユーザーの ON 意図が
  in-flight なのに古い OFF 完了で `applied` に false を書き、次のフォーカス変更で
  `ime_refresh.rs:527` が `set_ime_open(false)` を撃つ」という `docs/experiments.md` の
  spurious `apply_ime_open(false)` ファミリーは構造的に作れない（指摘 M2）。
- 条件 3 が「情報を増やすときだけ書く」を保証する。既に `Confirmed{open:target}` が
  入っているなら触らないので、**`Confirmed` を `Optimistic` へ降格させて `at_ms` を
  失う**（`to_pair()` が 0 を返すようになり `build_ime_control_view` の判断が変わる）
  事故が起きない。書き込みは純粋に加算的で、`Unknown` か「今の意図と食い違う値」を
  今の意図に寄せるときだけ発火する。

`Optimistic` を選ぶのは ADR-098 決定6-a の語彙に忠実だからである——これは
**in-flight な apply 自身の確認ではない**。「この値の適用に成功した完了を見た。ただし
今まさに走っている apply の帰結としてはまだ未確認」を型で正しく表す。
`Confirmed` を書くと、まだ着地していない `pending` の結果を確定として詐称することになる。

#### この単純化で失うもの（明示）

r3 の `superseded` は「受理してよいのは直前の 1 世代だけ」という上限を持っていた。
r4 にはそれが無いので、理屈の上では**任意に古い完了**が条件 1〜3 を満たしうる。
具体的には「gen10 が true を適用 → gen10.5 が false を適用して完了（`applied` は false）
→ gen11 が true を要求（in-flight）→ ここで gen10 の完了が到着」というケースで、
r3 は棄却し r4 は `Optimistic(true)` を書く。この差を受け入れる理由:

- 書く値は `pending.target`（= 今まさに送っている値）と同じであり、下流の消費箇所
  （前掲 7 箇所）はいずれも「その意図が既に適用された前提で振る舞う」だけになる。
  `force_on_attempt_allowed` は重複 force-ON を抑止（送信中なので正しい）、
  `warmup_ime_on()` は意図と一致、`ime_refresh.rs:527` の enforce-OFF は意図が ON の
  とき撃たない、`ime_refresh.rs:296` の `explicit_verify` は `Unknown` 脱出により
  OsPoll を先行させる（実状態の確認が早まる方向）——**いずれも `Optimistic` の定義
  そのままの振る舞い**である。
- `Optimistic` と `Confirmed` を区別する唯一の消費箇所は
  `output/ime_apply_planner.rs:86`（KanjiToggle 系の `confident` 計算で
  `is_confirmed()` が要求される）である。決定2 の書き込みは条件3 により
  `Confirmed` を上書きしないので `confident` を `true` から落とすことは無く、
  `Unknown`/逆値から `Optimistic` へ動いた場合は `confident=false` のまま変わらない。
  加えて同ファイルの doc（`ime_apply_planner.rs:57-61`）が「`OpenBelief::confident` を
  読む本番コードは現在ログ（`platform.rs`）のみで、already_matched 判定には
  使われていない」と明記しているため、現時点の実害はゼロ。**ただし将来
  `confident` が再配線されたらこの箇所が決定2 の影響を最初に受ける**ので、
  実装時に `ime_apply_planner.rs` の doc から本 ADR を参照させること。
**`pending` が既に解除された後に届く旧完了は救えない（S1、閉じない範囲）。** 決定2 の
条件はすべて「現在 `pending` が存在する」ことを前提にしている。したがって
「gen10 が送信中 → gen11（同一 target）が上書き → gen11 が `UnsafeToToggle` 等で
`pending` を解除 → その後 gen10 の完了が到着」というケースは、比較すべき
`pending.target` がもう無いため受理できない。これはコンテキスト「何が壊れているか」の
項目4 の一部であり、**本 ADR は閉じない**（決定6 と同じ扱いで明示する）。
実害が限定的である理由: `UnsafeToToggle` は `pending` を解放するので、r1 が懸念した
「以後の別 generation の完了が全て stale 判定され続ける固着」の連鎖には入らない。
`applied` は 1 回分の情報を失って古いままだが、`desired_open` との乖離は次の
drift correction が検出して新しい要求を出す。症状は「固着」ではなく
「1 回分の情報の喪失（＝補正が 1 サイクル遅れる）」に留まる。

**書いた `Optimistic` の寿命は in-flight の窓を超えうる（S2）。** 上の受け入れ理由は
「誤差は `pending` が着地するまで」と書いたが、正確には `pending` が
`UnsafeToToggle` 等で解除された場合、この `Optimistic(target)` は**その後も残る**。
値は実際に成功した apply（`ImeApplySucceeded`）に裏付けられているので嘘ではないが、
`applied` を `Unknown` へ戻す経路は TsfNative では `FocusChanged`・drift correction・
ユーザーの明示操作の 3 つに限られる。したがって「本来なら `Unknown` のままで
force-ON が飛んだはずの状況で、`Optimistic(true)` によって
`force_on_attempt_allowed` が `false` を返す」窓が、フォーカスが変わるまで続きうる。
これは ADR-098 決定1-a が TsfNative で守ろうとした不変条件に接する挙動なので、
実機ソークの確認項目に入れた（「未解決の疑問」参照）。

- 一方で r3 の上限を維持するコストは、フィールド 1 本ではなく「退避・overflow・
  `FocusChanged` クリア・パージ・成功時解放・失敗時解放・`UnsafeToToggle` 例外」という
  7 つの規則の集合だった（この解放セマンティクスは r3 の 3 ラウンドで 3 回誤った）。
  上限が守っている実害が「意図と同じ値を数十 ms 早く書く」ことである以上、割に合わない。

なお `ImeApplyFailed`（`Failed` / `UnsafeToToggle`）はこの緩和経路の対象外である
（厳密一致した場合の扱いは決定3 を参照）。`Failed` の `effective = !target` は観測ではなく
推論であり（`platform.rs:1190-1191` は同じ `Failed` を「実状態が不明のため belief を
汚さない」と扱う）、それを in-flight な `pending` との一致判定に使う根拠が無い。

条件 2 は「完了の `effective` 値」ではなく **`ImeApplySucceeded` が運ぶ要求 `target`**
と比較する（指摘 N7）。成功系 outcome（`Applied`/`FallbackSent`/`AlreadyMatched`）では
両者は一致するが、`effective` は `Failed` のとき推論値になるため、将来 `Failed` の除外を
緩めた瞬間に条文と実装が分岐する。`target` 側に統一しておく。

**generation の一意性には依存しない（指摘 N4/R4）。** r3 は「照合先スロットが 2 個に
増えたので誤マッチ確率が上がる」という指摘に対し、「設置済み `pending` の generation は
一意だから起きない」と反証していた（完了のタグは必ず `pending_generation()` 由来であり、
`pending` 設置経路は必ず `ImeApplyRequested` を dispatch して `next_seq` を進めるため）。
この反証自体は今も正しいが、**r4 の緩和経路は generation を一切見ないので、そもそも
一意性に依存しない**。安全性を担保しているのは条件 1〜3 である——この点は将来
条件 2 を緩めようとする変更が入ったときの歯止めとして、実装時の doc に明記すること。

### 決定3: `applied` の書き込みを `reduce()` に一本化し、戻り値を型で (b)/(c) に分離する

現状 `applied` は 2 箇所から書かれている: `record_ime_apply_result` 内の
`record_confirmed(effective, ts)`（`platform_state.rs:877`）と、そこから dispatch された
`ImeApplySucceeded` を受ける `reduce()` のアーム（`ime_model.rs:614-624`）。同じ完了に
対する二重書き込みであり、決定1/2 のゲートを両方に入れると条件が乖離する
（レビュー指摘 m1）。

**generation 付きの完了については `reduce()` を `applied` の唯一の書き込み点にする。**

```rust
pub(crate) fn record_ime_apply_result(
    &mut self, open: bool, outcome: ImeOpenOutcome, generation: Option<u64>, ts: u64,
) -> ImeApplyAcceptance {
    let Some(generation) = generation else {
        // generation を持たない 5 経路（key_pipeline 2 系統 / ime_refresh drift /
        // runtime force-on）は完全に現状維持。target 一致で pending を解放し、
        // record_confirmed で applied を書く。これらは「今まさに自分が送った
        // 同期完了」であり、順序も宛先も曖昧にならない。
        if outcome == ImeOpenOutcome::UnsafeToToggle { return ImeApplyAcceptance::NotSent; }
        self.record_confirmed(effective_of(open, outcome), ts);
        return ImeApplyAcceptance::Accepted;
    };

    // (c) 副作用ゲート: 厳密一致 かつ 同一 epoch かつ 実際に送った場合のみ。緩和しない。
    let acceptance = self.classify_apply_completion(generation, outcome);
    // (a)(b) は reduce() が担当する。generation 不一致でも dispatch する——
    // 決定2 の緩和受理と pending 解除はどちらも reducer の中にあるため。
    self.dispatch_event(ImeEvent::from_apply_outcome(open, outcome, generation), TickMs(ts));
    acceptance
}
```

戻り値は `bool` をやめ、意味を型に持たせる:

```rust
pub(crate) enum ImeApplyAcceptance {
    /// 追跡中の apply の完了。composition/warmup 副作用を駆動してよい。
    Accepted,
    /// 上書きされた古い apply の完了。`applied` は決定2 の条件下で更新されうるが、
    /// composition/warmup 副作用は**駆動しない**。
    Superseded,
    /// 宛先ウィンドウが変わった／どの transition にも属さない完了。
    Stale,
    /// `UnsafeToToggle` — 送っていないので完了ですらない。
    NotSent,
}

impl ImeApplyAcceptance {
    /// `on_ime_applied`（mark_composition_cold + eager warmup + ImeModeFsm）を
    /// 駆動してよいか。`Accepted` のみ。
    pub const fn drives_composition_side_effects(self) -> bool { matches!(self, Self::Accepted) }
}
```

`runtime/mod.rs:482` は `if acceptance.drives_composition_side_effects()` に変える。
これにより、決定2 で `applied` の更新を緩めても **`on_ime_applied` の発火条件は
1mm も緩まない**。r1 の決定1 が持っていた「gen10 target=true の `Failed` 遅延到着が
`ImeOn` → `mark_composition_cold(SetOpenTrue)` → `send_eager_tsf_warmup()` を撃つ」
（BUG-70 / BUG-31 と同族の spurious warmup バースト）は構造的に起こらない。

`reduce()` 側は以下になる。**`ImeApplyFailed` アームも決定の一部である**（指摘 N3）——
決定3 が generation 付き経路から `record_confirmed` を外す以上、現在
`platform_state.rs:877` が担っている `Failed → Confirmed{open: !open}` の書き込みは、
このアームへ移設しなければ**消失する**（挙動の暗黙変更になる）。移設先でも決定1 の
epoch ゲートを掛ける。`UnsafeToToggle` は `ApplyError` で判別して `applied` を書かない
（現状と同じ——`platform_state.rs:858-865` が「何が実際の IME 状態かは依然不明」として
ミラーしていない）:

```rust
ImeEvent::ImeApplySucceeded { target, generation } => {
    let epoch = self.observations.current_focus_epoch;
    if self.pending.as_ref().map(|p| p.generation) == Some(generation) {
        // 厳密一致: 追跡中の apply 自身の確認。pending を解除し Confirmed を書く。
        let p = self.pending.take().expect("checked above");
        if p.focus_epoch == epoch {            // 決定1
            self.applied = AppliedImeState::Confirmed { open: target, at_ms: envelope.time.tick_ms };
        }
    } else if self
        .pending
        .as_ref()
        .is_some_and(|p| p.focus_epoch == epoch && p.target == target)  // 決定2 条件1・2
        && self.applied.applied_open() != Some(target)                  // 決定2 条件3
    {
        // 上書きされた apply の完了。値は今 in-flight な apply の行き先と同じなので
        // 安全だが、in-flight 自身の確認ではないので Optimistic に留める。
        // pending には触らない（解除は厳密一致だけの仕事）。
        self.applied = AppliedImeState::Optimistic(target);
    }
    // どの条件にも当たらない = Stale。何もしない（現状維持）。
}
ImeEvent::ImeApplyFailed { target, generation, error } => {
    let epoch = self.observations.current_focus_epoch;
    if self.pending.as_ref().map(|p| p.generation) == Some(generation) {
        let p = self.pending.take().expect("checked above");
        // platform_state.rs:877 からの移設。UnsafeToToggle は「送っていない」ので
        // 実状態が不明 → 書かない（現状維持）。Failed は effective=!target を書く。
        if p.focus_epoch == epoch && error != ApplyError::UnsafeToToggle {
            self.applied = AppliedImeState::Confirmed { open: !target, at_ms: envelope.time.tick_ms };
        }
    }
    // 失敗系に決定2 の緩和経路は無い（推論値を belief に書かない）。
}
```

### 決定4: `reduce()` のタイムアウト遅延パージを match の**後**へ移す（指摘 N1）

`reduce()` 冒頭（`ime_model.rs:439-453`）の lazy purge は、現状では無害である——
`applied` の書き込み（`record_confirmed`）が `dispatch_event` の**前**に済んでいるため、
パージが `pending` を消しても書き込みは既に終わっている。決定3 がこの書き込みを
`reduce()` のアームへ移すと、この順序関係が**逆転して害になる**:

| | gen10 の pending を T0 に設置し、完了が T0+8100ms に到着した場合 |
| --- | --- |
| 現行 | 入口の `pending_generation()` は未パージなので `Some(10)` で一致 → `record_confirmed` が `applied` を書く → dispatch → 冒頭パージ → アームは no-op。**`applied` は更新される** |
| 決定3 のみ（r2） | dispatch → 冒頭パージで `pending=None` → 厳密一致も決定2 の緩和条件（`pending` の存在が前提）も成立しない → **`applied` が更新されない** |

さらに、`classify_apply_completion` は dispatch の**前**に評価される。この時点の `pending`
は未パージなので `Accepted` が返り `on_ime_applied` が走る。すなわち
**`applied` は更新されないのに composition 副作用だけが走る**——決定0 が分離したはずの
(b) と (c) が矛盾する。これは r2 の設計欠陥であり、8000ms は BUG-34 の
`SendMessageTimeoutW` ハング実測 5741ms を吸収するために選ばれた値
（`tuning.rs:445-457` は「1 秒のままだと正当な in-flight apply が完了するより先に
pending がパージされ、後から届く完了が stale として黙って捨てられる」と明記）である以上、
「境界を跨ぐ完了」は設計上想定内の事象であって無視できない。

**決定**: パージを `match envelope.event { .. }` の**後**へ移す（対象は `pending` のみ。
r4 で `superseded` を廃止したので 2 スロット目は無い）。

```rust
pub fn reduce(&mut self, envelope: &ImeEventEnvelope) {
    match envelope.event { /* ... 全アーム ... */ }
    // 決定4: パージは match の後。期限切れの transition にも「自分自身の完了で
    // 解決される最後の一回」を必ず与える。タイムアウトはスロット寿命の上限で
    // あって、待っていた当の完了を弾くためのフィルタではない。
    if self.pending.as_ref().is_some_and(|p| p.is_timed_out(envelope.time.monotonic)) {
        log::debug!("[ime-model] pending transition timed out — purge");
        self.pending = None;
    }
}
```

この順序なら:

- `classify_apply_completion`（dispatch 前）とアーム（match 内）が**同一のスナップショット**
  を見る。両者の間に `pending` を変更するコードが存在しなくなるため、(b) と (c) の
  食い違いが**構造的に起こりえない**。決定0 の「3 つの判定が同じスナップショットを見る」は
  ここで担保される。
- パージ本来の目的（完了が永遠に来ない `pending` が後続の完了を全部 stale にする固着の
  防止、BUG-34 横展開 D-prep）は保たれる。**差分は `reduce()` 1 回の内部で match の前か
  後かだけ**であり、`reduce()` の外から見た `pending` の可視状態は変わらない
  ——`executor.rs` の `pending_generation()` 読み取りに影響は無い（指摘 R5。r3 の
  「解放が 1 イベント分遅れる」という表現は、外部から観測できる遅延があるかのように
  読めるため撤回する）。
- `ImeApplyRequested` アームは期限切れかどうかに関わらず `pending` を新しい transition で
  置き換えるので、パージが後ろに回っても影響を受けない。

### 決定5: `ImeApplyRequested` アームの未決コメントを解消する

「実機で実際の発生頻度を確認してから拒否に倒すかどうかを判断する」という保留を削除し、
以下の趣旨に置き換える: 「`pending` の上書きは安全である。上書きされた apply の完了は、
(1) 同一フォーカス epoch、(2) 現在の `pending.target` と同じ値への成功、(3) `applied` が
まだその値になっていない、の 3 条件下で `applied` を `Optimistic` として更新できる
（決定2）。composition/warmup 副作用は `ImeApplyAcceptance::Accepted`（generation 厳密
一致 + 同一 epoch）でのみ駆動されるため、上書きによって spurious な cold-mark /
eager warmup は発生しない」。

`log::warn!` は上書きの発生頻度を測る診断として残すが、**文言も更新する**（S5）。
現在の「その apply の完了が今後 stale 判定される可能性がある」は決定2 導入後は
事実に反する——同一 target・同一 epoch なら stale 判定されず `applied` は更新される。
「上書きされた apply の完了は、target と focus epoch が一致すれば `applied` に
反映される（決定2）。一致しなければ破棄される」という趣旨に書き換えること。

### 決定6: 本 ADR が閉じない範囲を明示する（指摘 N2）

決定1 の epoch ゲートが掛かるのは **generation を伴う完了だけ**である。
`generation=None` の 5 経路は決定3 で「完全に現状維持」としたが、r2 はその理由を
「これらは今まさに自分が送った同期完了であり、順序も宛先も曖昧にならない」と書いていた。
**5 経路のうち 1 つでこれは成立しない**:

| 経路 | 同期/非同期 | epoch 露出 |
| --- | --- | --- |
| `key_pipeline.rs:755`（idle-conv-check DirectInput） | 同期 | 無し |
| `key_pipeline.rs:1025`（shadow toggle OFF、sync 分岐） | 同期 | 無し |
| `ime_refresh.rs:800`（non-ImmCross drift correction） | 同期 | 無し |
| `runtime/mod.rs:830`（force-on） | 同期 | 無し |
| **`key_pipeline.rs:1012`（shadow toggle OFF、ImmCross 非同期分岐）** | **`spawn_local` 非同期** | **有り** |

`key_pipeline.rs:998-1021` は `run_open_chain_async(order, ..).await` の完了後に
`with_app(|app| app.on_ime_apply_complete(false, outcome, None, ShadowToggle))` を呼ぶ。
待機中に Alt+Tab でフォーカスプロセスが変われば、完了は新ウィンドウのコンテキストで
到着し、`generation=None` なので epoch ゲートを通らずに `record_confirmed(false, ts)` が
`applied = Confirmed{open:false}` を書き、さらに `Accepted` を返して
`on_ime_applied(false, ..)` が新ウィンドウで `mark_composition_cold(SetOpenFalse)` を
実行する。**C2 が閉じるのは generation 付き経路だけである。**

**本 ADR ではこれを修正しない。** 理由は、この経路に epoch ゲートを掛けると
「棄却された完了が `clear_pending_if_matches` を呼ばなくなる」＝ pending 解放の
意味論まで変わるためで、決定0 が「(a) は現状維持」と決めた範囲を越える。修正の形は
既存の `probe_admission::admit_epoch_in_app`（observation 軸で全く同じ
「spawn 時に epoch を捕まえ、`with_app` の中で照合して早期 return」を提供している）を
actuation 完了へ適用するか、この経路にも generation を払い出して generation 付き経路へ
合流させるかのどちらかになる。**残存ギャップとして `docs/known-bugs.md` に起票し**、
別 ADR で扱う。

## r1 レビュー指摘（C1〜m6）との対応

| 指摘 | 対応 |
| --- | --- |
| **C1** `accepted` が `on_ime_applied` のゲートであり、緩和すると stale 完了が cold-mark + eager warmup を撃つ | **決定3**。戻り値を `ImeApplyAcceptance` にし、副作用は `Accepted`（厳密一致 + 同一 epoch）のみ。緩和は (b) `applied` 軸に限定した。指摘のとおり「実装時確認」ではなく決定事項として扱う |
| **C2** フォーカス跨ぎで旧ウィンドウの完了が新ウィンドウの `applied` を書く | **決定1**（`ImeTransition.focus_epoch`）。ただし**原因帰属を訂正**した: これは r1 の決定1 が作る新規バグではなく、`FocusChanged` が `applied` だけリセットして `pending` を残す**既存の欠陥**であり、現行コードで既に成立する。本 ADR の対象に格上げした。**ただし閉じるのは generation を伴う完了だけ**であり、非同期の `generation=None` 経路は決定6 で残存ギャップとして明示した（指摘 N2） |
| **C3** `clear_pending_if_matches` の target 一致は `generation=None` 経路の唯一の pending 解放手段 | **r1 の決定2 を全面撤回**。`clear_pending_if_matches` / `record_confirmed` / `record_optimistic` は一切変更しない。指摘は正しく、r1 の「実害はない」は事実誤認だった（5 経路を本文に列挙した） |
| **M1** `applied` の消費箇所の列挙が不完全（3 箇所ではなく 5 箇所） | コンテキストの表を 5 箇所に修正。`force_on_attempt_allowed` と `resolve_warmup_ime_on` を追加し、それぞれが決定1/決定2 の条件3 で守られることを本文に書いた |
| **M2** `applied` は「履歴」ではなく「現在の OS 状態」として読まれており、r1 の価値づけが誤り | 指摘を全面的に採用し、r1 の表(b)の理屈を削除。**決定2 の条件2**（`pending.target` との一致）が指摘の対案(i)そのもの |
| **M3** `last_confirmed_generation` は非-generation writer 5 箇所を見ないため古い完了が新しいミラーを上書きしうる | `last_confirmed_generation` **ごと廃止**したため消滅。決定1 の epoch ゲートが、指摘の失敗シナリオ（フォーカス変更後の `ir_post_focus_change_snapshot` を古い完了が上書き）を直接閉じる |
| **M4** generation はグローバル一意ではない（`allocate_event_generation` は increment しない） | 指摘は正しい。実コードで確認し、r1 の「単調増加が保証されている」を誤りとして本文に明記。**大小比較を使う設計をやめ、等値比較のみ**にした（等値が安全な理由も明記） |
| **M5** `architecture_guard.rs` の呼び出し箇所数ピン留めが壊れ、新 API がガードの穴になる | 新しい recorder API を**作らない**設計にしたため、`.record_confirmed(`=5 / `.record_optimistic(`=1 は変わらない。代わりに `applied` の書き込みが `ime_model.rs` に集約されることを利用し、**新しいピン留めテスト**（`ime_model.rs` 内の `self.applied = ` 出現数）を証拠義務に必須項目として入れた |
| **m1** `applied` の writer が 2 つになりゲート条件が非対称 | **決定3** で generation 付き完了の `applied` 書き込みを `reduce()` に一本化。二重書き込みは新設ではなく既存であり、本 ADR で解消する |
| **m2** 案A 却下理由の「8000ms 近くブロック」は誇張 | 却下理由を書き直した。8000ms は BUG-34 の `SendMessageTimeoutW` ハング実測 5741ms に由来する**最悪ケース上限**（`tuning.rs:439-457`）であって典型待ち時間ではない、と明記した上で、別の（実測に依存しない）根拠で却下している |
| **m3** reuse 根拠に挙げた `output/mod.rs` の自前カウンタ 3 つは無関係で逆向きの論拠 | 指摘のとおり。**新カウンタを足す設計そのものをやめた**ので、この段落は削除した |
| **m4** 「ADR-098 決定5 が許容した 5 箇所」は不正確（許容は 3 箇所） | `record_confirmed`/`record_optimistic` を変更対象から外したため監査自体が不要になった。関連節の記述も 3 箇所に訂正 |
| **m5** `Failed` で単調カウンタを進める妥当性が未定義 | カウンタ廃止で消滅。加えて**決定2 の緩和経路は `Failed`/`UnsafeToToggle` を対象外**とし、`platform.rs:1190-1191` の「Failed は belief を汚さない」と整合させた。厳密一致経路の `Failed → Confirmed{!open}` は**意図的に現状維持**（除去は独立した挙動変更になるため。r2 が書いていた「ADR-098 決定1-c が依存している」という根拠は誤りで、r3 で訂正済み——下記「未解決の疑問」参照） |

## r2 レビュー指摘（N1〜N9）との対応

| 指摘 | 対応 |
| --- | --- |
| **N1**（Critical）決定3 が `applied` 書き込みを `reduce()` へ移すと、冒頭のタイムアウトパージが正当な完了の更新を消す。しかも `classify_apply_completion` は未パージの `pending` を見るため、`applied` は更新されないのに副作用だけ走る | **決定4 を新設**。パージを `match` の**後**へ移す。提示された 3 案のうち (1) を採用した——(2)（完了イベントだけパージ対象外）は「どのイベントが対象か」という表を増やし、(3)（スナップショット共有）は照合のたびにコピーを持ち回る必要がある。(1) は「期限切れの transition にも自分自身の完了で解決される最後の一回を与える」という**単一の意味論**で言い切れ、副次的に classify とアームが同一スナップショットを見ることも保証される（決定0 の前提が構造的に成立する） |
| **N2**（Major）epoch ゲートが `generation=None` に掛からず C2 が半分しか閉じない。`key_pipeline.rs:1012` は `spawn_local` 非同期 | 指摘は正しい（5 経路のうち 4 つは同期、1 つだけ非同期であることを表で確認した）。**決定6 で適用範囲を明示**し、r2 の「5 経路とも同期完了だから曖昧にならない」という誤った理由づけを撤回。修正は pending 解放の意味論に触れるため本 ADR の範囲外とし、`docs/known-bugs.md` への起票と、`probe_admission::admit_epoch_in_app` を使う follow-up の形を書いた |
| **N3**（Major）`ImeApplyFailed` アームが擬似コードに無く、epoch ゲートの有無が未定義。ADR-098 決定1-c との因果の記述も逆 | **決定3 の擬似コードに `ImeApplyFailed` アームを追加**（epoch ゲート付き、`UnsafeToToggle` は `ApplyError` で判別して書かない）。因果の誤りは「未解決の疑問」で明示的に訂正し、**留保の根拠を差し替えた**（クールダウンへの依存ではなく、除去が独立した挙動変更になるため） |
| **N4**（Major）2 スロット化で generation 非一意性への露出が広がる | r3 で**反証した**（完了の generation は必ず `pending_generation()` 由来であり、`pending` 設置経路は必ず 1 件以上 `record()` するため、設置済み pending の generation は互いに一意）。**r4 では論点自体が消滅**——緩和経路が generation を一切見なくなったため（決定2 末尾に経緯を残した） |
| **N5**（Major）`superseded` が `Failed`/`UnsafeToToggle` で解放されず、決定5 の warn が診断信号として機能しない | r3 では解放を明記して対応したが、**r4 では `superseded` ごと廃止したので論点が消滅**した（解放規則が存在しない）。この解放セマンティクスが 3 ラウンドで 3 回誤ったことが、廃止判断の直接の根拠になっている |
| **N6**（Minor）`FocusEpoch` はプロセス変更でしか進まない | 文言を「どのフォーカスプロセスに対して」へ修正。加えて**粒度が要件と一致している理由**を追記した——`applied = Unknown` をリセットする `ImeEvent::FocusChanged` の dispatch 元も同じ `on_focus_process_changed` なので、epoch が進む単位と守るべき不変条件の単位が定義上一致する |
| **N7**（Minor）条件3 の条文（effective 値）と擬似コード（要求 target）の食い違い | 採用。**`target` に統一**し、`effective` が `Failed` で推論値になるため将来分岐する、という理由も明記 |
| **N8**（Minor）「到着順序が結果に影響しない」は成立範囲が狭い | 採用。「受理を `pending.target` との一致に限ったので、受理された書き込みは常に現在の意図と同じ値になる」に書き換えた |
| **N9**（Minor）(a-2) のピン留めは `applied` が `pub` である限り穴が残る | 採用。ピン留め対象を `ime_model.rs` 内から **crate 全域の `.applied = ` / `applied:` 直接代入**へ広げた。private 化が本筋である旨も証拠義務に書き添えた |
| **m6** 証拠義務が C1/C2 をカバーしない | 証拠義務を書き直し、(a) unit test **と** (b) `docs/known-bugs.md` エントリを**両方必須**にした。Linux で検証できない `runtime/` 層の副作用（決定3）とフォーカス跨ぎ（決定1）については journal replay シナリオと実機ソークを個別項目として立てた |

## r3 レビュー指摘（R1〜R5）との対応

| 指摘 | 対応 |
| --- | --- |
| **R1**（Critical）`ImeApplyFailed` の pending 一致分岐が無条件に `superseded = None` し、ADR 自身の主目的シナリオ（gen11 が `UnsafeToToggle` → gen10 の完了が捨てられる）を打ち消す | 指摘は正しい。**ただし提案された 1 行削除では当のシナリオは直らない**——`ImeApplyFailed` が `pending` を解除した後は、受理条件が比較すべき `pending.target` 自体が存在しないため、`superseded` を残しても gen10 の完了は受理されない。正しく直すには解放条件を outcome 別に場合分け（成功と真の失敗では解放、`UnsafeToToggle` では保持）する必要がある。これは同じスロットの解放規則に対する 3 度目の訂正であり、**`superseded` を廃止する**方を選んだ（決定2）。廃止後は解放規則が存在しないので R1 は構造的に発生しない |
| **R2**（Major）決定2 が受理条件からタイムアウトを外した根拠（「パージ済みならスロットが空」）が、決定4 のパージ後置化で無効化されている | 指摘は正しい（r3 は自分が同じ ADR 内で変えた順序の帰結を、削除の根拠に使っていた）。**`superseded` の廃止で論点ごと消滅**した——r4 の緩和経路が見るのは現在の `pending` だけであり、期限切れの `pending` に対しては決定4 が「自分自身の完了で解決される最後の一回」を与えるという、pending 側と**同一の**意味論が働く |
| **R3**（Minor）`focus_tracking.rs:127`（bootstrap）は `focus.focus_epoch` を進めるが `FocusChanged` を dispatch しないため、2 つの `focus_epoch` がずれる | 採用。決定1 に**表を追加**し、`platform_state.focus.focus_epoch`（`ImmLikeTicket` 側）と `observations.current_focus_epoch`（決定1 側）が別の値であること、bootstrap 直後に不一致になり次の `FocusChanged` で再同期すること、**2 つを混ぜて使うと恒久的な不一致を作る**ことを明記した。決定1 はスタンプも照合も後者だけを使うので正しさは保たれる |
| **R4**（Minor）N4 の反証は collision には正しいが mis-attribution チャネルを見ていない。安全性を担保しているのは条件2 だと明記すべき | 採用。**r4 では緩和経路が generation を一切見ない**のでチャネル自体が消えた。その上で決定2 末尾に「安全性を担保しているのは条件 1〜3 であって generation の一意性ではない」を明記し、将来条件2 を緩める変更への歯止めとして doc 化を義務づけた |
| **R5**（Minor）決定4 の「解放が 1 イベント分遅れる」は不正確。証拠義務の項番の並び順も乱れている | 採用。「差分は `reduce()` 1 回の内部で match の前か後かだけで、`reduce()` の外から見た `pending` の可視状態は変わらない（`executor.rs` の `pending_generation()` に影響しない）」に書き換えた。項番は (a-4) を削除（generation 一意性テストが不要になったため）して (a-1)〜(a-3) の連番に整理した |


## r4 レビュー指摘（S1〜S6）との対応

| 指摘 | 対応 |
| --- | --- |
| **S1**（Major）「失うもの」節に、`superseded` 廃止で実際に失った最大のもの（`pending` 解除後に届く旧完了は救えない）が無い | 採用。決定2「この単純化で失うもの」に段落を追加し、決定6 と同じ「閉じない範囲の明示」として扱った。`UnsafeToToggle` は `pending` を解放するので固着連鎖にはならず、症状は「1 回分の情報の喪失（補正が 1 サイクル遅れる）」に留まることも書いた |
| **S2**（Major）緩和経路が書く `Optimistic(true)` の寿命が「in-flight の窓」を超えるケースが受け入れ理由の射程外 | 採用。「誤差は `pending` が着地するまで」という記述を訂正し、`pending` が `UnsafeToToggle` で解除された後も `Optimistic` が残ること、`applied` を `Unknown` へ戻す経路が TsfNative では 3 つに限られることを明記。未解決の疑問に「TsfNative で force-ON が想定より発火しなくなっていないか」を追加し、Win キー保持で `UnsafeToToggle` を意図的に起こす具体的ソーク手順を書いた |
| **S3**（Major）`applied` 消費箇所は 5 箇所ではなく 7 箇所（`output/ime_apply_planner.rs:80-91` と `runtime/ime_refresh.rs:296-300` が欠落） | 採用。表を 7 行に拡張。特に `ime_apply_planner.rs:86` が **`Optimistic`/`Confirmed` を区別する唯一の消費箇所**であることを明記し、決定2 の受け入れ理由をこの母集合に対して書き直した。条件3 により `Confirmed` は上書きされないので `confident` が `true` から落ちることは無く、かつ同ファイルの doc（`:57-61`）が「`confident` を読む本番コードはログのみ」と明記しているため現時点の実害はゼロ。将来の再配線に備え、実装時に同ファイルの doc から本 ADR を参照させる |
| **S4**（Minor）(a-1) に緩和経路の 2 つの入口ケースが無い | 採用。`applied == Unknown` からの書き込みと `Confirmed{open: !target}` からの変化を項目 5 として追加（条件3 が「同じ値のときだけ何もしない」であって「常に何もしない」ではないことの固定） |
| **S5**（Minor）決定5 が残す `log::warn!` の文言が決定2 導入後は事実に反する | 採用。決定5 に文言更新の指示を追記した（「stale 判定される可能性がある」→「target と focus epoch が一致すれば `applied` に反映される」） |
| **S6**（Minor）R3 対応で追加した bootstrap の説明が実コードの呼び出し構造と食い違う | 採用。実コードで確認（`on_focus_process_changed` の呼び出し元は `apply_focus_probe_result:98` の 1 箇所のみ、`establish_initial_focus_scope` は `advance_focus_tracking` を直接呼ぶ `:121`）。「bootstrap は epoch を進めるが `FocusChanged` を dispatch しないので 2 値がずれる」と簡潔に書き直した |


## 却下した代替案

- **案A: 新規要求を拒否し、1 件だけ deferred queue で再試行する**（reject-and-requeue）。
  却下理由: 遅延の長さではなく、**新しい queue が独自の順序・タイムアウト・フォーカス
  無効化の意味論を必要とする**点。`pending` は既にタイムアウトとパージを持っており、
  その外側にもう 1 段の待ち行列を作ると「queue に載ったまま焦点が変わった要求」の扱いを
  再発明することになる（決定2 は退避も再送もしないので、この問題を持たない）。
  なお r1 は却下理由を「`IME_APPLY_PENDING_TIMEOUT_MS`（8000ms）近くブロックしうる」と
  書いていたが、この 8000ms は BUG-34 の `SendMessageTimeoutW` ハング実測 5741ms に
  マージンを載せた最悪ケース上限（`tuning.rs:439-457` に導出あり）であって典型的な
  待ち時間ではない。実測に基づかない誇張だったので取り下げる。
- **案B: `pending` を `Vec<ImeTransition>` にして複数 generation を同時追跡する**。
  却下理由: `pending.is_some()` を「今 apply が進行中か」として読む箇所
  （`executor.rs` の generation タグ付け 4 箇所を含む）の意味論を全部精査し直す必要がある。
  r3 は `superseded` 1 スロットという有界版を採ったが、r4 はそれも廃止した——
  「どの世代の完了か」を覚えておく必要が無いことが分かったため（決定2）。
  複数 generation の同時追跡は、本 ADR が解く問題に対して二重に過剰である。
- **案C（r1 の決定1）: `last_confirmed_generation` による単調性チェックへの緩和**。
  却下理由: (1) generation が一意でないため大小比較が成立しない（M4）、(2) `applied` を
  「現在の OS 状態」として読む消費側と矛盾する（M2）、(3) フォーカス軸を区別できない
  （C2）、(4) 副作用ゲート (c) まで巻き添えで緩む（C1）、(5) 非-generation writer を
  見ないため新しい情報を古い情報で上書きしうる（M3）。5 点とも実コードで確認した。
- **案D: `FocusChanged` で `pending` を `None` にする**（決定1 の代わり）。
  却下理由: 一見単純だが、`pending = None` は `executor.rs` の
  `ime.model().pending_generation()` が `None` を返すことを意味し、フォーカス変更後の
  actuation が**generation なし**でタグ付けされるようになる。generation を持たない完了は
  target 一致でしか pending を解けない経路に合流するため、追跡性が下がる方向の変更になる。
  epoch スタンプなら `pending` を生かしたまま「解除はできるが belief は書けない」という
  必要な区別だけを表現できる。
- **案E: `generation` を `state/event_origin.rs::Generation` 型／真に一意なカウンタにする**。
  却下理由: M4 は正しい指摘だが、本 ADR の決定はいずれも**等値比較しか使わない**（かつ
  r4 では緩和経路が generation を見ない）ため一意性を必要としない。型の統一自体は望ましいが、belief 修正と同じコミットに混ぜると
  実機ソークの原因切り分けができなくなる。別 ADR / 別コミットに切り出す。

## 未解決の疑問（実装時に確認すること）

- **決定1 が副作用も止めることの実機影響。** フォーカス変更後に旧ウィンドウ宛ての完了が
  届いても `on_ime_applied` が走らなくなる。設計上は問題ないはず（新ウィンドウ側では
  `ir_post_focus_change_snapshot` が `mark_composition_cold_focus_change()` と
  `send_eager_warmup()` を独立に実行済み）だが、eager warmup の欠落は BUG-02 の
  リテラル化ファミリーに直結するため、Chrome / Windows Terminal + GJI で
  Alt+Tab 直後の初打鍵を重点的にソークすること。
- **厳密一致経路の `Failed → Confirmed{open: !open}` を残すか。** `platform.rs:1190-1191`
  は同じ `Failed` を「実状態が不明」として belief を汚さない扱いにしており、非対称が残る。
  **r2 はこれを「ADR-098 決定1-c の force-ON クールダウンがこの書き込みを前提にしている
  から消せない」と説明していたが、因果が逆である**（指摘 N3）——`ime_actuation.rs:236-244`
  は「chain が `Failed` を返した場合に生成される `Confirmed{open:false}` を素通ししてしまい
  …このクールダウンがその歯止めになる」と書いており、クールダウンはこの書き込みの
  **害を補償するために**入っている。書き込みを消してもクールダウンは壊れない。
  それでも本 ADR で**変えない**理由は別にあり、この書き込みは `warmup_ime_on()` /
  `ime_refresh.rs:527` の enforce-OFF / engine gating が「apply に失敗した直後」に何を
  見るかを決めているため、除去は belief 修正とは独立した挙動変更になり、同一コミットに
  混ぜると実機ソークの原因切り分けができなくなるからである。決定3 で
  `state/platform_state.rs` から `state/ime_model.rs` へ移設する際、この非対称と
  「消してもクールダウンは壊れない」ことを doc に明記し、除去は別 ADR で扱う。
- **TsfNative で force-ON が想定より発火しなくなっていないか（S2）。** 決定2 が書く
  `Optimistic(true)` は `force_on_attempt_allowed` の (1) 番ガードに引っかかるため、
  `pending` が `UnsafeToToggle` で解除された後もフォーカスが変わるまで force-ON を
  抑止しうる。ADR-098 決定1-a が「フォーカス入場後 `applied` は `Unknown`」を
  load-bearing にしている以上、ここは BUG-16 / BUG-69 の再燃と同じ形になりうる。
  Chrome / Windows Terminal + GJI で、Win キーを押しながらの IME 操作（`UnsafeToToggle`
  を意図的に起こす）→ そのまま打鍵、という手順をソーク項目に含めること。
- **`pending` 上書きの実頻度。** 決定5 で残す `log::warn!` を実機ログで数える。
  常態的に発生するようなら、決定2 の緩和で症状を吸収するより「なぜ in-flight 中に
  次の要求が出るのか」を先に調べること（本 ADR は上書きを安全にするだけで、
  上書きが起きること自体は正常化していない）。
- **決定4 のパージ順序変更が他アームに与える影響。** `reduce()` 内で `self.pending` を
  読むのは `ImeApplyRequested` / `ImeApplySucceeded` / `ImeApplyFailed` の 3 アームだけで
  あることをコード確認済み（他アームは触らない）。`ImeApplyRequested` は期限切れかを
  問わず上書きするので順序変更の影響を受けない。実装時、この 3 アーム以外に
  `self.pending` の読み取りが増えていないかを再確認すること。
- **`ImeModel` の既存 unit test 群**（`ime_model.rs` の `ImeApplyRequested` 関連、
  `platform_state.rs:1642` 以降の `record_ime_apply_result` テスト）は `ImeTransition` の
  フィールド追加と戻り値型変更でコンパイルが通らなくなる。`unsafe_to_toggle_*` の 2 本は
  `ImeApplyAcceptance::NotSent` を期待する形に書き換える（意味は変わらない）。
- Windows 実機ソークによる検証。このリポジトリの IME belief 変更は実機依存が強く、
  単体テストだけでは不十分（[[ime-belief-architecture]] 参照）。

## 証拠義務

[.claude/rules/fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md) の
再発ファミリーのうち **IME belief**（`state/ime_model.rs`）と
**warmup / cold-start**（決定3 が `on_ime_applied` → `send_eager_tsf_warmup` の
ゲートを変えるため）の**両方**にまたがる。(a) か (b) の片方ではなく、以下を**すべて**
実装コミットに添えること。

- **(a-1) `state/ime_model.rs` の unit test**（Linux 実行可）。最低限:
  1. gen10 pending → gen11 `ImeApplyRequested`（同一 target）で上書き → gen10 の
     `ImeApplySucceeded` 到着 → `applied` が `Optimistic(target)` に更新され、
     `pending` は gen11 のまま残ること（本 ADR の主目的シナリオ）。
  2. 同上で gen11 が**逆 target** の場合 → `applied` が更新**されない**こと（M2）。
  3. gen10 pending → `FocusChanged` → gen10 の `ImeApplySucceeded` 到着 →
     `applied` が `Unknown` のままであること（C2、既存バグの回帰テスト）。
  4. generation 不一致の `ImeApplyFailed`（`Failed` / `UnsafeToToggle`）が `applied` を
     書かないこと（m5）。また `applied` が既に `Confirmed{open:target}` のとき、
     generation 不一致の成功完了が**それを `Optimistic` へ降格させない**こと
     （決定2 条件3、`at_ms` の消失防止）。
  5. 緩和経路の 2 つの入口: (i) `applied == Unknown` の状態で generation 不一致の
     成功完了が届き `Optimistic(target)` が書かれること、(ii) `applied ==
     Confirmed{open: !target}` の状態で同じ完了が届き `Optimistic(target)` へ
     変わること（決定2 条件3 が「同じ値のときだけ何もしない」であって
     「常に何もしない」ではないことの固定、S4）。
  6. **タイムアウト境界を跨ぐ完了**（gen10 の pending を設置 → `IME_APPLY_PENDING_TIMEOUT_MS`
     経過後に gen10 の `ImeApplySucceeded` が到着）で `applied` が更新されること（N1）。
     決定4 が入っていないとここで落ちる。**他のシナリオはいずれも境界を跨がないため、
     このテストが無いと N1 の回帰を検出できない。**
  7. `ImeApplyFailed`（`error != UnsafeToToggle`）の厳密一致完了が
     `Confirmed{open: !target}` を書くこと（N3、`platform_state.rs:877` からの移設が
     欠落していないことの回帰テスト）。
- **(a-2) `crates/awase-windows/tests/architecture_guard.rs` に新規ピン留めテスト**:
  **crate 全域**の `applied` 直接代入（`.applied = ` / 構造体リテラルの `applied:`）の
  出現数を固定する。`ImeModel.applied` は `pub`（`ime_model.rs:198`）で
  `platform_state.rs:188/198` からも代入されているため、`ime_model.rs` 内だけを数える
  テストでは穴が残る（N9）。本筋は `applied` の private 化 + アクセサ化だが、
  それは `to_pair()`/`applied_open()` の消費側 5 箇所と `record_*` の再配置を伴うため
  別コミットに切り出し、本 ADR ではピン留めで代替する。既存の
  `applied_state_recorders_call_sites_are_accounted_for`（`.record_confirmed(`=5 /
  `.record_optimistic(`=1）は**期待値が変わらない**ことも確認する（変わるなら
  決定0「(a) は現状維持」が破れている）。
- **(a-3) journal replay シナリオ**（`tests/journal_replay.rs`）。決定1 の
  フォーカス跨ぎは `runtime/` 層のタイミングが絡み unit test では再現しきれないため、
  `FocusChanged` → 遅延 `ImeApplySucceeded` の順序を記録済みジャーナルで再生する。
- **(b) `docs/known-bugs.md` へのエントリ起票（必須、2 件）。**
  1. 本 ADR が閉じる症状（`applied` の固着による drift correction の再送ループ／
     フォーカス跨ぎでの force-ON 恒久封鎖）、再現条件、本 ADR と実装コミットのハッシュ。
     Linux で検証できない決定3 の副作用ゲートと決定1 の実機影響は、known-bugs 側の
     人間可読な記録でしか残らないため、(a) があっても省略しない。
  2. **決定6 の残存ギャップ**（`key_pipeline.rs:1012` の非同期 shadow toggle OFF が
     `generation=None` のため epoch ゲートを通らない）。本 ADR で意図的に閉じないので、
     「ADR-108 で対象外と判断した既知の穴」として再現条件（shadow toggle OFF 直後の
     Alt+Tab）と follow-up の方針まで書き残す。ここを書かないと、後日
     「C2 は ADR-108 で閉じたはず」と誤読される。

## 関連

- [.claude/rules/ime-belief-architecture.md](../../.claude/rules/ime-belief-architecture.md)
  — Observe → 純粋 `classify_*` → `reduce()` の三層分離。決定3 の「`applied` の
  書き込みを `reduce()` に一本化する」はこの原則の `applied` 軸への適用である。
- [ADR-098](098-tsfnative-applied-confirmed-laundering-and-force-on-removal.md)
  — 決定1-a（TsfNative でフォーカス入場後 `applied` を `Unknown` に保つ）、
  決定1-c（force-ON クールダウン）、決定5（`record_confirmed` の非-actuation 例外
  **3 箇所**）、決定6-a（`Optimistic`/`Confirmed` の構築子分離）。本 ADR は
  `record_confirmed`/`record_optimistic`/`clear_pending_if_matches` を**変更しない**。
- `crates/awase-windows/src/state/probe_admission.rs` — `ImmLikeTicket` /
  `AcceptedObservation` による observation 軸の focus epoch 照合。決定1 は同じ考え方の
  actuation 軸への横展開であり、実装時に相互参照 doc を張ること。
- `crates/awase-windows/src/tuning.rs:439-457` — `IME_APPLY_PENDING_TIMEOUT_MS` の
  導出（BUG-34 実測 5741ms + マージン）。案A の却下理由の訂正根拠。
