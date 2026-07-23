---
paths:
  - "crates/awase-windows/src/**/*.rs"
---

# IME belief アーキテクチャルール

## Observe → pure decision → belief の三層分離

IME 状態（ON/OFF・input_mode）の belief 更新は必ず以下の流れを守ること。

```
Observe     Win32 API / async probe → ImeSnapshot / raw value
Pure        classify_* 関数 → ImeUpdate / Option<InputModeState>  ← 副作用ゼロ
Apply       dispatch_event → reduce()  ← belief の唯一の書き込み点
```

`ImeModel::desired_open` / `ImeModel::input_mode` フィールドは **private**（`state/ime_model.rs` 以外から書き込み不可、コンパイラが強制）。外部からは `desired_open()` / `input_mode()` の読み取り専用アクセサのみを使うこと。

## なぜこのルールが繰り返し破られるか（背景）

2026-07-05: ウィンドウ切替時に NICOLA エンジンが誤って OFF になり続ける不具合を調査した結果、`reset_to_off_for_tsf_native_cache_miss`（TsfNative の hwnd キャッシュミス時の安全デフォルト処理）が、**観測が何もない**ことを根拠に `UserImeSetIntent { source: IntentSource::Recovery }` を dispatch し、`desired_open` を直接書き換えていた。`Recovery` はユーザー操作のふりをするため、`ObserverReported` の confidence ガード（`derive_open()` の Low 除外）を完全にバイパスしていた。同種の偽装が `InputModeObserved { source: ObservationSource::ImmGetOpenStatus }` でも見つかった（SetOpen 直後の内部訂正が、実際には API を呼んでいないのに「観測した」ことにして dispatch していた）。

**この種の近道が繰り返し発生する理由**: `dispatch_event()` はどんな `ImeEvent` variant でも受け付ける汎用 API であり、「とりあえず動く」修正には、生の観測値をその場で if 分岐して直接 dispatch する／confidence なしの意図イベントを流用する、という近道がタイプ量的に「正しい道」と同じくらい簡単に書けてしまう。規約が散文（このファイル）だけだと、時間的プレッシャーの下ではこの近道に流れやすい。そのため以下の対策を **コンパイラ／CI／専用イベントで構造的に**強制している。

## ON/OFF belief の変更ルール

観測には信頼度 (`ObservationConfidence::Low/Medium/High`) があり、`ImeModel::reduce()` は `ObserverReported` を受けても `desired_open` を直接書き換えない（`observations` に記録するのみ）。`effective_open()` が `derive_open()`（Medium+ の合意 / High 即採用）→ `most_recent_trusted()`（confidence 不問、フォールバック専用）→ `desired_open` の順で解決する。

- **High confidence（ImmCross/FocusProbe 由来の子 hwnd 読み取り）**: `write_imm_cross_probe(open)` / `write_focus_probe(open)` を使う
- **Medium confidence（定期 poll）**: `apply_ime_update(&update)` 経由（`poll_and_classify_ime` の戻り値）
- **観測が何もない場合の安全デフォルト推測**（cache miss 等）: `ObserverReported { source: ObservationSource::HeuristicDefault, confidence: Low, .. }` を使う（`reset_to_off_for_tsf_native_cache_miss` を参照）。**`UserImeSetIntent` を使ってはならない** — ユーザー意図を偽装することになり、confidence ガードを完全にバイパスする。
- `dispatch_event(ImeEvent::ObserverReported { .. })` を直接呼ぶのは上記メソッドの内部に限る

### ユーザー意図 (`UserImeSetIntent` / `UserImeToggleIntent`) の `source`

`UserIntentSource` は `SyncKey` / `PhysicalImeKey` / `Command` の3つのみ。**`Recovery` や `HwndCache` は列挙値として存在しない**（かつて存在し、ヒューリスティックな推測をユーザー意図として偽装する抜け道になっていたため、型ごと削除した）。

真のユーザー操作ではないが `desired_open` を書き換える必要がある場合は、専用イベントを使うこと。

- **パニックリセット（全面復旧）**: `ImeEvent::PanicReset { target }` — `apply_panic_reset` 専用。`last_intent` を設定しない。
- **HWND キャッシュ復元**: `ImeEvent::HwndCacheRestored { target }` — `apply_hwnd_cache_restore` 専用。`last_intent` を設定しない。

これら2つのイベントは「観測ではないが、ユーザー意図でもない、直接書き込みの正当な例外」として明示的に隔離されている。**新しい呼び出し元を追加する前に、本当に「全面復旧」「キャッシュ復元」に該当するか確認すること**。該当しないヒューリスティックな推測は `ObserverReported` + `ObservationConfidence::Low` を使うこと。

## input_mode の変更ルール

`InputModeObserved` は `confidence: ObservationConfidence` を持ち、`reduce()` は `Medium+` の場合のみ `input_mode` を上書きする（`Low` は記録のみで無視）。ON/OFF の `derive_open()` と同じ考え方。

### ✅ 正しいパターン

観測結果から input_mode を更新するときは `classify_fetched_snapshot()` / `classify_idle()` 等の `classify_*` 純粋関数を経由する。

```rust
// ImmCrossProbe・FocusProbe 等で snap が手に入った場合
let update = crate::observer::ime_observer::classify_fetched_snapshot(
    &snap,
    tick_ms.0,
    app.platform_state.ime.effective_open(),
    app.platform_state.ime.is_force_on_guard_active(),
    app.platform_state.ime.input_mode(),
    app.platform_state.ime.belief.prev_conversion_mode(),
);
if let Some(mode) = update.new_input_mode {
    app.platform_state.ime.dispatch_event(
        ImeEvent::InputModeObserved {
            mode, source, confidence: ObservationConfidence::High, at: tick_ms,
        },
        tick_ms,
    );
}
```

### ❌ 禁止パターン 1: classify_* を経由しないインライン判定

```rust
// NG: classify_* を経由せず直接判定している
if !ConvMode::from_u32(conv).is_eisu()
    && matches!(app.platform_state.ime.input_mode(), InputModeState::ObservedEisu)
{
    app.platform_state.ime.dispatch_event(
        ImeEvent::InputModeObserved { mode: InputModeState::AssumedRomaji { .. }, .. },
        tick_ms,
    );
}
```

`classify_ime_snapshot` / `classify_fetched_snapshot` / `ConvMode::classify_idle` はその判定ロジックを純粋関数として集約するために存在する。同じ判定を外部で再実装しない（`key_pipeline.rs` の focus-conv-check は過去にこれを3箇所で重複していたため `classify_idle` 一本化に統合済み）。

### ❌ 禁止パターン 2: 観測を偽装した内部補正

`InputModeObserved` は「外部を観測した」ことを表す。awase 自身が能動的に input_mode を書き換える場合（過去の SetOpen の帰結を先読みする、IMM-broken 補正、パニックリセット、フォーカスリセット、キャッシュ復元等）は、実際には呼んでいない API を `source` に偽装せず、必ず `InputModeApplied { strategy: InputModeApplyStrategy::.., result, .. }` を使うこと。

```rust
// NG: 実際には ImmGetOpenStatus を呼んでいないのに観測した体で dispatch している
self.dispatch_event(
    ImeEvent::InputModeObserved {
        mode: InputModeState::AssumedRomaji { .. },
        source: ObservationSource::ImmGetOpenStatus, // 嘘
        confidence: ObservationConfidence::High,
        at: tick_ms,
    },
    tick_ms,
);

// OK: awase 自身の能動的な訂正として素直に表現する
self.dispatch_event(
    ImeEvent::InputModeApplied {
        mode: InputModeState::AssumedRomaji { .. },
        strategy: InputModeApplyStrategy::PostSetOpenEisuReset,
        result: InputModeApplyResult::Applied,
        at: tick_ms,
    },
    tick_ms,
);
```

新しい能動的訂正を追加する場合は `InputModeApplyStrategy` に専用の variant を追加すること（既存の `ImmBrokenCorrection` / `PanicReset` / `CacheRestore` / `PostSetOpenEisuReset` / `UserImeOnEisuReset` を参照。かつて記載していた `FocusReset` は実在しない）。

## user IME-ON 経路と ObservedEisu 救済の対称性

IME を ON にする経路を追加したら、stale `ObservedEisu` の救済（`state/eisu_recovery.rs` の `eisu_reset_on_ime_on`）を**必ず対で配線**すること。ObservedEisu は engine activation を塞ぎ、activation 側の救済は Decision 経由に限られるため、救済のない IME-ON 経路は Imm32Unavailable アプリで engine 永久 inactive の循環デッドロックを作る（2026-07-06 MS Edge で実発生）。経路×救済の対応表は `state/eisu_recovery.rs` の module doc を SSOT とし、`tests/architecture_guard.rs` の `user_ime_on_paths_are_paired_with_eisu_reset` が対称性を監視する。

## belief の書き込み点

`ImeModel::reduce()` in `state/ime_model.rs` が唯一の書き込み点。`desired_open` / `input_mode` フィールドは private であり、`reduce()` 以外からの直接代入はコンパイルエラーになる。

## この規約を実際に強制する仕組み（散文だけに頼らない）

規約は「読めば守れる」を前提にしない。以下の3段構えで、規約を破る近道が実際に取れないか、少なくとも自動で検知されるようにしている。

1. **コンパイラ（最強）**: `desired_open` / `input_mode` フィールドの private 化。`UserIntentSource` から `Recovery` / `HwndCache` を削除し `PanicReset` / `HwndCacheRestored` 専用イベントに分離。`InputModeObserved` への `confidence` フィールド必須化。
2. **dylint lint（HIR レベルの意味解析）**: `lints/ime_event_guard` — `ImeEvent::PanicReset` / `HwndCacheRestored` が designated 関数（`apply_panic_reset` / `apply_hwnd_cache_restore`）以外で構築されると warning。`lints/observation_source_guard` — 禁止パターン2（観測偽装）を直接検出する: `InputModeObserved { source: ObservationSource::ImmGetOpenStatus, .. }` はどこで構築しても warning（この組合せは常に偽装）、`ConvBitsInference` は `apply_idle_conv_check` 以外で構築すると warning。`cargo dylint --all -p awase-windows -- --target x86_64-pc-windows-gnu` で両方まとめて実行。
3. **CI テスト（軽量な第二の防衛線）**: `crates/awase-windows/tests/architecture_guard.rs` — `PanicReset` / `HwndCacheRestored` / `InputModeObserved` の構築箇所数をテキスト走査で固定し、想定外の増加を検知する。`cargo test -p awase-windows --test architecture_guard`（Linux でも実行可能、CI に組み込み済み）。

新しい「観測が乏しい状況での安全デフォルト」や「awase 自身の能動的訂正」を追加するときは、上記のどの仕組みにも引っかからないからといって「近道が許されている」わけではない。まず本当に `ObserverReported`（confidence 付き）/ `InputModeApplied`（strategy 付き）で表現できないか検討すること。

## `ImeModel` 以外の belief 的状態への適用範囲（2026-07-23 追記）

`GjiFsm`（TSF composition の warm/cold、`tsf/gji_fsm.rs`）にはこの3段防御が
一切無く、`GjiEvent::CompositionReset`/`NativeF2Consumed` が弱い代理指標
（`gji_candidate_visible_now()` の素の `AtomicBool` 読み取り）だけで無条件に
belief を書き換えていたことが、実機バグ2件（確定済み文字が VK_BACK で消える、
`docs/known-bugs.md` BUG-33 追補3・4）の根本原因だった。修正は dylint 新設や
private 化ではなく、`gji_idle_ms`（実観測値）をイベントの必須パラメータ化する
という軽量な手法で行った。

この教訓を受けて `ImeModel` 以外の belief 的トラッカー全体を監査した結果、
`ImeModeFsm`/conv mode/`TsfGate`/injection_mode/focus 分類は実観測でゲート
済みだったが、以下2箇所は「規約・コメントのみで守られている」構造的な弱さが
見つかり是正した:

- **`state/force_guard.rs::ForceGuardSet.guards`**: 全 `pub` フィールドで、
  正規の `clear_for_focus_change()` を迂回した直接フィールド操作が現存して
  いた → `guards` を private 化し `clear()` を唯一の公開クリア口にした。
- **UIA 非同期結果ハンドラ（`runtime/message_handlers.rs::handle_wm_focus_kind_update`、
  BUG-12 対策）**: 結果を意図的に破棄する no-op が単一のコメントだけで
  守られており、コンパイラ強制が無かった → `tests/architecture_guard.rs::
  uia_async_focus_kind_handler_does_not_write_belief` を追加し、この関数の
  本体に belief 書き込みパターンが一切出現しないことを固定した。

**新しい belief 的状態（FSM の内部状態、キャッシュされた分類結果、force guard
のような override 機構等）を追加・変更する際の判断基準**: 新規 dylint crate は
「型シグネチャの変更や private 化では防げない、意味論的な偽装」（例:
`ObserverReported` の `source` を偽って別の観測に見せかける）にのみ投資する。
それ以外は次の軽い順に検討する:

1. その値が**蓄積する**（一度書き込まれると以後の判断に効き続ける）か、
   それとも**毎回純粋関数で再計算される**か。後者なら無条件上書きでも実害
   なし（`AppKind`/`FocusCache` 等）。
2. 蓄積する値なら、書き込み経路を**1箇所の関数に集約**し、フィールドを
   private 化できないか（`ForceGuardSet` の例）。
3. private 化できない・関数を集約しても「その関数の中身が観測を無視している」
   ことまでは防げない場合は、`tests/architecture_guard.rs` に出現数固定
   テストを足す（UIA ハンドラの例、または `GjiEvent` のように必須パラメータ化）。
4. 「意味論的な偽装」（型は正しいが呼び出し元が嘘をついている）でなければ、
   dylint の新設は基本的に過剰投資。
