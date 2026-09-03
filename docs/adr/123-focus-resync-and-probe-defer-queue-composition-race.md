# ADR-123: フォーカス再同期ゲート退避キュー（`INPUT_DEFER`）と TSF プローブ退避キュー（`pending_deferred`）が合成し、モーラ境界のない一括 SendInput が発生して文字脱落・順序入替が起きる

## ステータス

**ドラフト（round 0）。Opus 2体（architect/premortem）敵対的レビュー未実施、これから実施する。**

[GitHub issue #148](https://github.com/cuzic/awase/issues/148) として追跡。

## 背景

Windows Terminal（TsfNative）+ GJI で「たとえば」と入力したところ「ばたと」に
文字脱落・順序入替した（`report_id: 01M1JJD54XQXSEJTHHFKV1WKA1`、app_version
1.18.0、`WrongCharacterOutput`）。journal 解析（`DumpTruncated`、total_entries=2560
中942件のみ、`dropped_key_input=426`）で以下が判明していた:

1. 入力直前、`msedge.exe` ⇔ `explorer.exe` ⇔ `windowsterminal.exe` 間で激しい
   Alt+Tab フォーカス往復が発生（`FocusTransition` の dwell_ms が
   156ms/813ms/140ms と極端に短い）。
2. 「たとえば」の入力開始タイミングで windowsterminal.exe（TsfNative）へ
   フォーカスが移り、GJI が `OnCold(Short)` に落ちた直後だった。
3. 物理キー入力自体は NICOLA 配列上正しい順序だったが、`gate_active=true` の
   ため各キーが 200〜315ms の遅延で「output-drain」キューに溜まってから
   一括 replay された。
4. この間、literal detection（`LiteralDetect`, cold_seq=235/236）が最初の
   「ta」に TSF 側の合成兆候（`write_delta`, `candidate_visible`）が一切見えず
   `SuspectedLiteral` → `gave_up=True(backs=1)` と判定し、backspace+再送の
   リカバリをトリガーした。
5. しかしこの判定が確定する前に、後続の「to」「e」の物理キーは既にキューに
   積まれ処理待ちの状態だった。
6. 結果として GJI 側は単一の合成イベント（`StartComposition` →
   `CompositionConfirmed`, `write_delta=315` という異常に大きい一括書き込み）
   しか記録しておらず、「え」が脱落し「ば」が先頭に来る形で確定した。

半角英数持続トグル機能（BUG-25）自体は正常動作中であり無関係と確認済み。

### 既存の関連バグ（参考、再調査不要と判断していたもの）

- **BUG-38**: `RawTsfLiteralRecovery` の give-up 分岐が `pending_deferred`
  （probe 実行中に届いた後続キーの退避キュー、`TsfWarmupCoordinator` 所有）を
  flush しないため出力順が入れ替わる、という酷似した過去バグ。修正済み
  （`Output::flush_raw_tsf_literal_recovery` の末尾に
  `flush_stale_deferred_vks_after_recovery` を追加）。
- **BUG-89**: gate 中（`OUTPUT_GATE`/`FOCUS_RESYNC`）に defer されたキーは
  `INPUT_DEFER` へ退避され、`handle_wm_drain_output_queue` から
  `KeyOrigin::DeferredReplay` として再生される。
- **BUG-45/BUG-75**: per-VK confirm の literal 判定は「代理指標（候補ウィンドウ
  SHOW / GJI I/O バイト増加）のタイムアウト」に基づく belief であり、実際の
  TSF composition 状態と乖離しても検出も訂正もできない構造的欠陥がある、という
  指摘が過去に何度もされている。

これらは「同じ穴の再発」に見えたため当初は BUG-38 の再発（未修正の残存範囲）を
疑ったが、下記の調査でそれとは別の合流点であると判明した。

## 根本原因

**`INPUT_DEFER`（フォーカス再同期ゲート `FOCUS_RESYNC`/`OUTPUT_GATE` が武装中
に物理キー生イベントを退避する、`app/mod.rs` が唯一の入口の退避キュー）と
`pending_deferred`（TSF プローブ実行中に届いた VK を退避する、
`TsfWarmupCoordinator` 所有のフラット `Vec<DeferredVk>`、BUG-38 が対象とした
キュー）は独立した別のキューである。ところが `WM_DRAIN_OUTPUT_QUEUE` ハンドラ
が `INPUT_DEFER` に溜まった複数モーラ分の生キーを同一メッセージループ・ターン
内で完全に同期的にリプレイするため、そのリプレイ中に新しく起動した TSF probe
が in-flight の間に届いた後続モーラの romaji が次々と `pending_deferred` に
巻き込まれる。`DeferredVk` はモーラ境界を持たない単一フラット Vec のため、
probe 完了時にまとめて flush されると、本来 2 モーラ分（「え」「ば」）である
VK 列が `split_vk_runs`/`send_vk_run_batch` によって「1本の重畳順ラン」
（down 全部 → up 全部）としてバッチ化され、単一 SendInput として GJI に届く。
GJI はこれを単一の合成単位として誤処理し、文字脱落・順序入替を起こす。**

### 因果チェーン（詳細、file:line 付き）

1. **フォーカス変更 → resync arm**: 高速 Alt+Tab 中、windowsterminal.exe
   （TsfNative）にフォーカスが移ると `FocusResyncGate::arm()`
   （`focus_resync.rs:65`）が武装される（`should_arm_focus_resync` は
   TsfNative かつ日本語 IME のときのみ true、
   `state/focus_resync_policy.rs:19-26`）。
2. **最初の物理キー（「た」の1鍵目）が resync をトリガー**:
   `handle_hook_key_event`（`app/mod.rs:469-510`）で
   `event.starts_focus_resync()` かつ `FOCUS_RESYNC.is_armed()` →
   `consume_and_close()`（`focus_resync.rs:103-107`）→
   `INPUT_DEFER.defer_during_output(event)`（`app/mod.rs:496`）でこのキーが
   退避される。同時にハード期限タイマー `TIMER_FOCUS_RESYNC`（100ms、
   `tuning.rs:472` `FOCUS_RESYNC_DEADLINE_MS`）が武装される
   （`app/mod.rs:492`）。
3. **後続の物理キー（と/え/ば の各鍵）も同じキューに巻き込まれる**:
   `app/mod.rs:500-506` の `has_pending_drain` チェック（FIFO 順序保証ロジック）
   により、resync gate 自体は既に `armed=false` でも、キューが空になるまで
   後続キーは同じキューに積まれ続ける。
4. **resync 完了 or 100ms 期限で drain**: `close_focus_resync_gate_if_current`
   （`key_pipeline.rs:519-531`）または `TIMER_FOCUS_RESYNC` ハンドラ
   （`message_handlers.rs:541-559`）が `post_drain_output_queue()` を呼び
   `WM_DRAIN_OUTPUT_QUEUE` を投函 →
   `handle_wm_drain_output_queue`（`message_handlers.rs:1300`）が起動。
5. **drain ハンドラの内部順序**（`message_handlers.rs:1316-1368`）:
   `(a) flush_raw_tsf_literal_recovery()` → `(b) INPUT_DEFER.take_all()`
   （timestamp 昇順ソート済み）→ `(c) for queued_event in &queue { deliver_key_event(...) }`。
   (c) は `with_app` クロージャ内で**完全に同期的なループ**であり、途中で
   `TIMER_TSF_PROBE`（10ms ポーリング）が割り込む余地がない。
   `deliver_key_event`（`message_handlers.rs:136-244`）は最終的に
   `app.process_key_event(event)` を同期呼び出しする。
6. **「た」の romaji 送信が TSF probe を起動し `OUTPUT_GATE` を再ロック**:
   drain ループ1件目（「た」）の romaji "ta" 確定で `Output::send_romaji_as_tsf`
   が cold-start パスで `LiteralDetectFsm`（`tsf/warmup/literal_detect_fsm.rs:476-516`）
   を `install_pending_tsf` する。`LiteralDetectFsm::new()` は
   `OutputActiveGuard::begin()`（`tsf/probe_bridge.rs:112-118`）を保持し、
   probe 完了まで `OUTPUT_GATE.active=true` を維持する。この区間の長さは
   `RAW_TSF_LITERAL_DETECT_MS = 300ms`（`tuning.rs:72`、long-idle 時は 500ms、
   `tuning.rs:80`）— issue 本文の「200〜315ms」の遅延と一致する。
7. **と/え/ば の物理キーが再び `INPUT_DEFER` に積まれる**: `OUTPUT_GATE.active=true`
   になった瞬間から、リアルタイムでタイプされる後続の物理キーは
   `handle_hook_key_event`（`app/mod.rs:487-498`）の `OUTPUT_GATE.is_active()`
   判定に再度捕まり `INPUT_DEFER` へ退避される。
8. **「た」の probe が give-up**: TSF 側に合成兆候が一切見えず
   `LiteralDetectCore::poll`（`literal_detect_fsm.rs:265-415`）が
   `SuspectedLiteral` → `gave_up=True(backs=1)` と判定（journal cold_seq=235）。
   `RawTsfLiteralRecovery` action は `dispatch_probe_actions`
   （`probe_io.rs:710-830`）で `set_raw_literal` により **static へ退避するのみ**
   （実送信は `flush_raw_tsf_literal_recovery` まで遅延）。probe machine が
   drop → `LiteralDetectFsm` の `OutputActiveGuard` が drop →
   `OUTPUT_GATE.active=false` + 新たな `WM_DRAIN_OUTPUT_QUEUE` が投函される
   （`tsf/probe_bridge.rs:121-138`）。
9. **2回目の drain ハンドラ呼び出し**:
   - (a) `flush_raw_tsf_literal_recovery()`（`output/mod.rs:1635-1650`）:
     backspace 送信 → romaji 再送（give-up なので無し）→
     `flush_stale_deferred_vks_after_recovery()`（BUG-38 修正箇所）。この時点で
     「と」の物理キーはまだ `INPUT_DEFER` にいるだけで engine を一度も通って
     いないため `pending_deferred` は空（BUG-38 の懸念とは別条件）。
   - (b)(c) `INPUT_DEFER.take_all()` で と/え/ば の物理キーがまとめて drain され
     **同一の同期ループ内で** 連続処理される。
10. **と の romaji "to" が新しい probe を起動**: ループ1件目（と）の romaji
    確定で `has_pending_tsf()==false`（直前で drop 済み）なので新しい
    `LiteralDetectFsm` が `install_pending_tsf` され、再び
    `OutputActiveGuard::begin()` で `OUTPUT_GATE.active=true` になる。
11. **え と ば が「probe in-flight」経路に落ちる**: 同一ループの2件目（え）・
    3件目（ば）が処理される時点では「と」の probe はまだ一度も tick されて
    いない。`Output::defer_if_probe_in_flight`（`output/mod.rs:1277-1290`）が
    `has_pending_tsf()==true` を見て、romaji "e"・"ba" を VK 列に変換し
    `warmup_coord.pending_deferred`（`output/tsf_warmup_coord.rs:296-305`
    `defer_vks_if_in_flight`）に `.extend()` で積む。この
    `pending_deferred: RefCell<Vec<DeferredVk>>` は**モーラ境界を一切持たない
    単一フラット Vec**（`DeferredVk { vk, needs_shift }` のみ、
    `tsf/warmup/probe_fsm.rs:38-41`）。
12. **「と」probe 完了時に え+ば が無差別に一括 flush される**: 「と」の probe
    完了時、`finish_probe_stage`（`output/mod.rs:1419-1450`）の
    `flush_pending_deferred_vks()`（`output/mod.rs:1753-1766`）、または
    give-up 経路なら `flush_stale_deferred_vks_after_recovery` が
    `pending_deferred` を丸ごと取り出し `send_deferred_vks` →
    `send_deferred_probe_vks_from`（`output/key_injector.rs:302-313`）→
    `split_vk_runs`+`send_vk_run_batch`（`output/key_injector.rs:226-260`）で
    送信する。
13. **重畳順バッチが複数モーラをまたいで適用される＝症状の直接原因**:
    `send_vk_run_batch`（`key_injector.rs:226-243`）は「run 内の全キーの Down
    を先に全部送り、その後 Up を全部送る」設計（本来は同一ローマ字内の同時
    打鍵オーバーラップ表現のため、`key_injector.rs:262-271`）。
    `pending_deferred` はどの VK がどのモーラに属していたかの区切りを保持
    しないため、「え」(VK_E) + 「ば」(VK_B, VK_A) が1本の run として
    `down(E) down(B) down(A) up(E) up(B) up(A)` の順で単一 SendInput にまとめ
    られる。GJI はこれを「1つの重畳打鍵イベント」として解釈し、単一の合成
    イベント（`write_delta=315`）に丸め込む——「え」消失・「ば」先行という
    症状に帰結する。

### 関係ファイル・関数一覧

| 役割 | file:line |
|---|---|
| resync gate 武装/消費/解除 | `crates/awase-windows/src/focus_resync.rs:65,103-107,117-124` |
| resync arm 条件（純関数） | `crates/awase-windows/src/state/focus_resync_policy.rs:19-26` |
| resync ハード期限 100ms | `crates/awase-windows/src/tuning.rs:472` |
| フック直下の gate 判定・`INPUT_DEFER` 投入の唯一の入口 | `crates/awase-windows/src/app/mod.rs:469-510` |
| `INPUT_DEFER` 実体（VecDeque, cap 1024） | `crates/awase-windows/src/input_defer.rs:11-118` |
| `OUTPUT_GATE`/`OutputActiveGuard`（RAII, depth カウント） | `crates/awase-windows/src/tsf/probe_bridge.rs:24-149` |
| `WM_DRAIN_OUTPUT_QUEUE` ハンドラ本体（(a)(b)(c) の順序） | `crates/awase-windows/src/runtime/message_handlers.rs:1300-1420` |
| `deliver_key_event`（同期でエンジン全体を回す） | `crates/awase-windows/src/runtime/message_handlers.rs:136-244` |
| `LiteralDetectFsm`（`OUTPUT_GATE` を probe 完了まで保持） | `crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs:476-516` |
| literal-detect 判定コア・give-up 判定 | `crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs:260-415` |
| `RAW_TSF_LITERAL_DETECT_MS = 300ms` | `crates/awase-windows/src/tuning.rs:72,80` |
| `RawTsfLiteralRecovery` dispatch（backs/romaji を static へ退避のみ） | `crates/awase-windows/src/output/probe_io.rs:710-830` |
| `flush_raw_tsf_literal_recovery`（backspace→romaji再送→reinit→`pending_deferred` flush の順序） | `crates/awase-windows/src/output/mod.rs:1619-1650` |
| `flush_stale_deferred_vks_after_recovery`（BUG-38 修正箇所） | `crates/awase-windows/src/output/mod.rs:1698-1735` |
| `pending_deferred` 実体（`TsfWarmupCoordinator`、モーラ境界なしフラット Vec） | `crates/awase-windows/src/output/tsf_warmup_coord.rs:51-57,296-337` |
| `defer_if_probe_in_flight`（probe 中の romaji を `pending_deferred` へ） | `crates/awase-windows/src/output/mod.rs:1277-1290` |
| `finish_probe_stage`（probe 終了時の `pending_deferred` flush） | `crates/awase-windows/src/output/mod.rs:1415-1450` |
| `flush_pending_deferred_vks` | `crates/awase-windows/src/output/mod.rs:1753-1766` |
| `send_deferred_vks` → `send_deferred_probe_vks_from` | `crates/awase-windows/src/output/probe_io.rs:139-142`, `crates/awase-windows/src/output/vk_send.rs:609-611` |
| `split_vk_runs`/`send_vk_run_batch`（down全部→up全部の重畳順バッチ、モーラ境界非考慮） | `crates/awase-windows/src/output/key_injector.rs:226-260,302-313` |
| `DeferredVk` 構造体（vk/needs_shiftのみ、モーラ情報なし） | `crates/awase-windows/src/tsf/warmup/probe_fsm.rs:38-41` |

### 確度の注記

journal が `DumpTruncated`（2560件中942件のみ、`dropped_key_input=426`）のため:

- **確定（コード読解で直接裏付け）**: 上記1〜7、9〜12（gate の種類・タイミング
  定数・drain ハンドラの実行順序・`pending_deferred` へのフォールバック条件・
  `DeferredVk` にモーラ境界が無いこと・`send_vk_run_batch` の down-then-up
  バッチング）。
- **強い推測**: 8. の cold_seq=235/236 が「た」probe の give-up 検出と対応
  すること（journal 引用と `RAW_TSF_LITERAL_DETECT_MS=300ms` の一致から妥当性
  が高いが、直接ログでの確認ではない）。
- **推測（GJI 内部挙動に依存し awase 側コードからは確認不能）**: 13. の
  「down-down-down-up-up-up の重畳順バッチが GJI 側で `write_delta=315` の
  単一書き込み・え脱落・ば先行という具体的症状に帰結するメカニズム」。SendInput
  の送出順序自体は確定しているが、GJI のローマ字変換ステートマシンがこれを
  どう解釈したかは awase 側の観測からの逆算にとどまる。実機再現時に
  `RUST_LOG=debug` で `key_injector.rs:306-309` 付近のログ（`pending_deferred`
  flush 時の VK 個数とタイミング）を直接確認できれば、確定へ格上げできる。

## BUG-38 との異同

**別の欠陥（BUG-38 の再発ではない）と判断する。**

- BUG-38 が守る不変条件は「同一の give-up イベントの backspace/romaji再送/
  reinit がすべて実送信された後でなければ、その**同じ probe 実行中**に届いた
  `pending_deferred` を flush してはいけない」という**単一 probe サイクル内の
  順序**の話。
- 本件は「`INPUT_DEFER`（gate 側の生キー退避）に溜まった別モーラの生キーが、
  drain の同期リプレイ中に新たな probe を次々に起動しては即座に次のモーラが
  `pending_deferred` に落ちる」という、BUG-38 の修正が触れている
  `flush_stale_deferred_vks_after_recovery` の**さらに手前**、つまり
  `pending_deferred` へ**そもそも複数モーラが混入すること自体**が問題。
  BUG-38 は「1モーラ分の `pending_deferred` を正しいタイミングで flush する」
  ことしか保証しておらず、「複数モーラが無区別に混在した場合の安全な分離」は
  最初から扱っていない（`DeferredVk` にモーラ境界フィールドが無いことからも、
  設計時点で想定されていないことが分かる）。
- BUG-38 のシナリオは「probe 実行中に1回だけ」後続キーが届くケースだが、
  本件は `INPUT_DEFER` 側の gate が先に生キーを塊で堰き止め、それを同期ループ
  で一気に再生することで `pending_deferred` への混入が**複数モーラ分連鎖する**
  という、`INPUT_DEFER` と `pending_deferred` という**2つの独立した退避機構が
  合成されて初めて顕在化する**新しいクラスの競合である。

## 検討する選択肢

### 選択肢A: `pending_deferred` にモーラ境界を持たせ、モーラごとに独立した SendInput で送る

`DeferredVk` のフラットリスト（`Vec<DeferredVk>`）を、モーラ単位でグルーピング
した `Vec<Vec<DeferredVk>>` に変更する。`flush_pending_deferred_vks` は
モーラごとに `send_vk_run_batch` を分けて呼ぶ（モーラ間はバッチを分離、モーラ内
のみ従来通り重畳順を許可）。

- **長所**: 症状の直接原因（手順13）をピンポイントで塞ぐ。GJI 側が「え」「ば」
  を別々の compose イベントとして受け取れるようになり、SendInput レベルの
  誤解釈を構造的に防止。既存の「同一モーラ内は重畳順で送る」という設計意図
  （`key_injector.rs:262-271`）とも整合する。
- **短所**: `pending_deferred` を溜める全経路にモーラ境界の受け渡しを追加する
  必要があり影響範囲がやや広い。モーラ単位に分けても「flush 自体が probe を
  経由しない生 VK 送信である」点は変わらず、GJI がリテラル化するリスク
  （`flush_stale_deferred_vks_after_recovery` の doc が既に認めている既知の
  残課題）は残る。
- **実装コスト**: 中。テストは `tsf_warmup_coord.rs` の既存ユニットテスト
  （`deferred_vks_survive_*` 系）の拡張で足りる範囲。

### 選択肢B: drain ループを probe in-flight 化した時点で中断し、残りは新しい drain に委ねる

`handle_wm_drain_output_queue` の (c) ループで `deliver_key_event` 呼び出し後に
`warmup_coord.has_pending_tsf()` をチェックし、true になった時点で残りのキュー
を `INPUT_DEFER.replay_later()` で書き戻してループを打ち切る。probe 完了時
（`finish_probe_stage`）に改めて drain を post する（既存の
`post_drain_output_queue()` 経路を流用）。

- **長所**: `pending_deferred` への複数モーラ混入という現象そのものが起きなく
  なる（根治に近い）。「1件の probe = 1件の後続キー退避」という BUG-38 の設計
  時の前提に実態を合わせる形で、`pending_deferred` 側のコードは無改造で済む。
- **短所**: 高速タイピング時、各モーラごとに 300〜500ms の literal-detect 待ち
  が直列化されうるため、cold-start 直後の連続入力のレイテンシが悪化するリスク
  がある。ユーザー体感の入力遅延が新たな不具合として顕在化する可能性があり、
  実測での検証が必須（`tuning-constants.md` の規約対象）。
- **実装コスト**: 中〜高。drain ループの中断・再開ロジックと、`INPUT_DEFER` へ
  の戻し順序（FIFO 維持）を慎重に設計する必要がある。

### 選択肢C: cold-start 直後の最初の1語に限り per-VK confirm へ強制フォールバック

`assess_warmth`（`output/mod.rs:1262-1275`）等の warm/cold 判定に「resync 直後」
フラグを追加し、cold-start 直後の最初の probe だけ per-VK 経路（1文字ずつ確認、
`veto_eligible=false`）を強制する。per-VK は1 VK ごとの待ち時間が短く
`OUTPUT_GATE` を長時間ホールドしないため、後続モーラが `INPUT_DEFER` に
溜まりにくくなる。

- **長所**: 300ms の word-level probe 待ちがそもそも発生しないため、選択肢Bの
  レイテンシ懸念を避けられる。cold-start という限定的な条件でのみ挙動を変える
  ため影響範囲が小さい。
- **短所**: 根治ではなく緩和策（per-VK でも複数モーラが同期ループで詰まれば
  同種の競合が理論上は起こりうる、確率を下げるだけ）。per-VK 経路と word-level
  経路の分岐条件がさらに増え、`ime_controller.rs::characterize_strategy` 周辺
  の複雑度が上がる。
- **実装コスト**: 低〜中。

### 暫定の方向性（未確定、レビュー前）

選択肢Aが症状への直接対策として最も確実だが、選択肢Bと組み合わせる
（`INPUT_DEFER` 側の「同期バーストで複数モーラを詰め込まない」構造的対策 +
`pending_deferred` 側の「万一混入しても安全に分離送信する」防御的対策の二段
構え）方が頑健と考えられる。単独ならまず選択肢Aから着手し、実機ソークで残存
頻度を見てから選択肢Bの要否を判断する案を軸に検討中。**この選定は Opus 2体
レビュー未実施の暫定案であり、round 1〜2 で覆る可能性がある。**

## 未決定事項

- 選択肢A/B/Cのどれを decision とするか（複数案の組み合わせ含む）。
- 選択肢Bを採る場合のレイテンシ悪化の許容範囲（実測が必要、
  `tuning-constants.md` 規約対象）。
- 手順13の GJI 側解釈メカニズムは推測にとどまる。実機 `RUST_LOG=debug` での
  検証手順を先行させるべきか（BUG-75/ADR-122 の「観測フェーズ先行」パターンの
  再利用が妥当か）。
- 本件が `FOCUS_RESYNC`/`OUTPUT_GATE` を経由する他のケース（BUG-89 の
  `DeferredReplay` 等）でも同型の合成が起こりうるか（横展開の要否）。

## 関連

BUG-38（`pending_deferred` flush 順序、本件の直接の前例）、BUG-89（`INPUT_DEFER`/
`OUTPUT_GATE`/`FOCUS_RESYNC` の deferred replay 経路、gate 中に defer された
Ctrl+key の別の隙間）、BUG-45/BUG-75（per-VK confirm の代理指標ベース判定の
構造的欠陥）、ADR-122（BUG-75 再発、cold-start per-VK confirm の race recovery、
Opus 2体 round 1〜3 が確立した「観測フェーズ先行」「blocker の切り分け」の
議論パターン）、[docs/bug-reports-triage.md](../bug-reports-triage.md)
（`01M1JJD54XQXSEJTHHFKV1WKA1` 該当行）。
