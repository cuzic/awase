# ADR-155: `deferred_engine_timers` の replay 時にも、親指キー押下タイムスタンプを defer 時点でスナップショットする（ADR-129 が未着手のまま残したタイマー経路）

## ステータス

**起票（未レビュー、opus-adversarial-consult 未実施）。** [ADR-129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md)
「未決定事項2」「限界」節がスコープ外として切り出した残りの半分を引き取る。
ADR-129 のキーイベント経路の修正（`RawKeyEvent` への capture-time スナップ
ショット追加）とは実装が独立しており、本 ADR の着手はそちらの完了を
待たない。

## 背景

ADR-129 は `runtime/key_pipeline.rs:105` の
`hook::thumb_down_timestamps()`（`WH_KEYBOARD_LL` フックが実時間で更新する
グローバル `AtomicU64` を、呼ばれた瞬間の値でライブに読む関数）が、
「イベントのライブ配送」と「`OUTPUT_GATE` active 中に `INPUT_DEFER` へ
退避されたイベントの drain replay」の両方から同一コードパスで呼ばれる
ため、drain replay 時に「イベント発生時点の値」ではなく「replay を実行
している"今"の値」を読んでしまう、という欠陥を確定させた。

この修正（`RawKeyEvent` に `left_thumb_down_snapshot`/
`right_thumb_down_snapshot` を追加し、`hook.rs::build_raw_key_event` の
capture 時点で埋め込む）は **キーイベント経路のみ** を閉じる。ADR-129
「限界」節が明記するとおり、`NicolaFsm::phys`（`nicola_fsm.rs:194`）は
`on_event`（キーイベント経由）と `on_timeout`（タイマー経由）の**両方**で
上書きされる共有フィールドであり、タイマー経由の書き込みは依然として
`hook::thumb_down_timestamps()` のライブクエリ（`runtime/mod.rs:294`
`build_ctx()`）に依存したまま残る。

### タイマー経路が壊れる具体的な条件（ADR-129 から引き継ぐ、未観測だがコードから特定済み）

`message_handlers.rs:611` のタイマーハンドラ本体は、`OUTPUT_GATE.is_active()`
（gate active）中は自身を `deferred_engine_timers`（`message_handlers.rs:
600-601` で push）へ退避して早期 return するため、**gate 非 active 時に
直接発火するケースは delta ≈ 0 で無害**。危険なのは gate 解除後、
`:1403` で `std::mem::take` された `deferred_engine_timers` が `:1407` で
`app.build_ctx()`（`runtime/mod.rs:294`、ライブの `hook::
thumb_down_timestamps()` を含む）を使って一括 replay されるケースのみ。

`resolve_char_and_thumb_as_separate_solos`（`nicola_fsm.rs:2459-2467` の
doc comment が「タイムアウト経由では thumb はまだ物理的に押されたままな
ので明示的に消費済みにする。怠ると `active_thumb_side()` が同じ物理押下を
未消費とみなし二重に使ってしまう」と明言）が、この gate 解除後 replay 時に
`phys` として「タイマーが本来対象としていた押下」ではなく「replay 実行
時点でたまたま押されている別の押下」を受け取ると、`right_thumb_consumed`/
`left_thumb_consumed` に無関係な押下が刻印される。

**症状はキーイベント経路（ADR-129 本編、「う」→「ゔ」のように余計な同時
打鍵が成立する）の鏡像になる**: タイマー経路では逆に、未消費の新しい押下が
「消費済み」と誤って刻印され、**本来成立すべき同時打鍵が失われる**（次に
来る文字キーが親指シフト面ではなく無シフト面で出てしまう）。

### capture 点は既に存在する

タイマー側には capture 点が無いわけではない。`PendingThumbData` は対象と
なる押下の `timestamp` を既に保持しており、`timeout_pending_thumb` が
これを使っている。ADR-129 が却下した代替案(d)（`InputContext` から親指
タイムスタンプを削除し、エンジン自身が親指キーの ↓/↑ から状態を導出する）
は、`deliver_key_event` の5つの早期 return（`keymap_latch`/`Hook(Nested)`/
`FocusKind::NonText`/`consume_keymap_match`/`consume_post_bypass`）のいずれかに
親指キーの ↑ が握り潰されると「エンジンが親指を押されっぱなしと信じ続ける
（無期限のスティッキー親指）」という、現状のバグより遥かに重い regression
を生むため却下されている。この却下理由は本 ADR にもそのまま引き継ぐ——
「エンジン側で導出する」方向は再提案しない。

## 決定

### 採用: `deferred_engine_timers` に defer 時点の親指スナップショットを同梱する

`message_handlers.rs:600-601` が `(timer_id, wparam)` をタイマーキューへ
push する箇所で、その時点の `hook::thumb_down_timestamps()` を1回だけ
読み、`(timer_id, wparam, left_thumb_down_snapshot, right_thumb_down_snapshot)`
として保持する（キーイベント経路が `RawKeyEvent` に埋め込んだのと同じ
「capture 時点で1回読んで運ぶ」パターンを、タイマーキューのエントリ型に
適用するだけであり、新しい設計判断は持ち込まない）。

`:1403` の `std::mem::take` → `:1407` の `app.build_ctx()` 一括 replay を、
エントリごとに保持しているスナップショットを使う形に置き換える。
`build_ctx()`（`runtime/mod.rs:294`）はライブの `hook::
thumb_down_timestamps()` を呼ぶ既存の実装のままでよい（gate 非 active 時の
直接発火や、他のフィールド（modifiers 等）の解決には引き続き使われる）——
本 ADR が変えるのは「タイマー replay 時に限り、親指タイムスタンプ2フィールド
だけは defer 時点のスナップショットで上書きする」経路のみ。

### 却下: `build_ctx()` 自体を「呼び出し元がスナップショットを渡せる」形にシグネチャ変更する

`build_ctx()` は他の呼び出し元（gate 非 active 時の直接発火経路含む）でも
使われる共有関数であり、シグネチャを変えると影響範囲が本 ADR のスコープ
（タイマー replay のみ）を超える。タイマーキューのエントリ側にスナップ
ショットを持たせ、replay 側で `build_ctx()` の戻り値を**部分的に上書き**
する方が、変更を局所化できる。

## 未決着・要レビュー論点（opus-adversarial-consult で詰めること）

1. **上書きの実装形態**: `build_ctx()` の戻り値を丸ごと使うか、`InputContext`
   の該当2フィールドだけをタイマー側で差し替えるか。後者の場合、
   `InputContext` の構築責務が2箇所に分散することの是非。
2. **`deferred_engine_timers` のエントリ型変更の影響範囲**: push 側
   （`message_handlers.rs:600-601`）と consume 側（`:1403-1407`）以外に、
   このキューを参照する箇所が無いか実装時に洗い出す。
3. **回帰テストの置き場所**: ADR-129 が採用した
   `tests/architecture_guard.rs` のテキスト走査ガード（「`hook::
   thumb_down_timestamps()` の呼び出し許可箇所は `runtime/mod.rs::
   build_ctx` と `message_handlers.rs` のタイマー経路のみ」）を、本 ADR
   実装後は「タイマー経路」の許可自体を外す（= 呼び出し許可箇所を
   `build_ctx` 1箇所のみへ縮小する）方向で更新できるか確認する。
4. **実機観測**: ADR-129 のキーイベント経路は実際の不具合報告（`report
   01M1N36MGDDJ5HN8FWRE4ZHS3J`）から起票されたが、本 ADR のタイマー経路は
   **未観測・コードからの理論的特定のみ**。実装前提条件として `fix-
   requires-evidence.md` の (b)（`docs/known-bugs.md` への記録）だけで
   実装に進んでよいか、実機再現を待つべきかは opus-adversarial-consult で
   判断する。
5. **[[keymap]] 等、他のタイマー系（`deferred_engine_timers` 以外）に同型の
   ライブクエリが残っていないかの棚卸し。** 本 ADR は `hook::
   thumb_down_timestamps()` のタイマー経路に限定するが、同じ「defer
   キューが実行時点のライブグローバル状態を読む」構造は他にもある
   可能性がある（[ADR-156](156-unify-deferred-execution-queues.md) 参照）。

## テスト

`fix-requires-evidence.md` のキー選択/warmup ファミリーに該当するため、
(a) 回帰テストまたは (b) known-bugs.md 記録の少なくとも一方が必須。

- 第一候補: `tests/architecture_guard.rs` への `hook::
  thumb_down_timestamps()` 呼び出し許可箇所の縮小（上記論点3）。
- 副次: `deferred_engine_timers` の replay がスナップショット値を使う
  ことを検証する Windows 専用テスト（`#[cfg(windows)]`、実行は
  `windows-build` CI に委ねる）。
- 単体テストで `NicolaFsm::on_timeout(event, phys)` に新旧2つの `phys` を
  渡す形は、ADR-129 が同型のケースで却下した理由（「バグの実体は
  `phys` を作る側にあり、`on_timeout` 自体の比較ロジックは健全」）と
  同じ理由で不採用とする。

## 関連

[ADR-129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md)
（本 ADR が引き継ぐ「限界」節・未決定事項2の出所、キーイベント経路の
先行修正）、[ADR-010](010-thumb-consumption-timestamp.md)（`Option<Timestamp>`
による親指消費追跡、比較ロジック自体は健全と確認済み）、
[ADR-008](008-physical-thumb-state-separation.md)（物理親指キー状態と
FSM 解決ロジックの分離）、[ADR-156](156-unify-deferred-execution-queues.md)
（`deferred_engine_timers`/`INPUT_DEFER`/`pending_deferred` の構造的な
共通パターンを扱う将来構想、本 ADR はその1インスタンスの局所修正）。
