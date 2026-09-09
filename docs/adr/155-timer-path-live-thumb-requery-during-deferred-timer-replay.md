# ADR-155: タイマー経路の親指タイムスタンプ問題（ADR-129 が未着手のまま残した部分）— クローズ（未実装、failure scenario 未確立）

## ステータス

**クローズ（2026-09-08、実装せず）。** 「決定」節が引き継ぎ事項として残した
2条件のうち条件1（懸念する失敗シナリオを具体的なイベント列として構成できるか）
は、本ADR自身の round1 レビューが「到達可能な具体的な変種を洗った結果…
1本も構成できなかった」とすでに結論済みであり（下記「中心的な問題」節）、
再調査を要さずクローズ条件を満たしている。`docs/known-bugs.md` BUG-126
として軽い記録を残し、本ADRは実装に進まないままクローズする。次に同種の
症状が実機で報告された場合は、BUG-126の記録から本ADRと [ADR-129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md)
「案A」「案B」（下記「将来の実装案」節）を再検討の出発点にすること。

**実装保留（opus-adversarial-consult round1〜round2 で収束、クローズ前の
経緯）。**

**2026-09-09、ADR-158 TH2で追記**: `docs/adr/index.md`に本ADRとは別に
「ADR-131」という行（同じ`deferred_engine_timers`のreplay/物理状態ライブ再取得を
扱う計装ADR、「採用・実装完了、developマージ済み（診断専用、挙動変更なし）」）が
存在していたが、対応する本文ファイル（`131-deferred-timer-replay-shares-stale-
live-phys-snapshot.md`）は一度もcommitされたことがなかった（ADR-151/152と同型の
「本文なき権威」、新設したCI存在チェックで発見）。内容が本ADRと同一調査の別段階
（診断ログ追加の完了報告）と判断されるため、当該index.md行は削除し、この記録を
本ADRへ統合した——診断専用ログの追加自体はdevelopに実装・マージ済みという事実は
保持する。
起票時点（初版）はコード内の行番号を [ADR-129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md)
「限界」節からそのまま複写していたが、develop 側の変更でその後の複数コミット
により全てずれていた（初版がどの時点の行番号を指していたにせよ、ADR は
「次の担当者が再調査せずに済む」ことが存在意義であり、体裁ではなく機能の
欠落として round1 で Must-fix 扱いにした）。本版はシンボル名ベースの参照に
改め、round1で発見された技術的な誤りを訂正した上で、**実装着手そのものを
保留する**。round2 で「round1 の訂正自体が過剰訂正だった」という新規
Must-fix 1件（`build_ctx()` は12箇所から呼ばれる共有関数であり、
drain/replay 側専用ではない）を追加で検出・反映し収束した。

## 背景（round1 で訂正済みの事実関係）

ADR-129 は `runtime/key_pipeline.rs::kp_run_inner` が呼ぶ
`hook::thumb_down_timestamps()`（`WH_KEYBOARD_LL` フックが実時間で更新する
グローバル `AtomicU64` を、呼ばれた瞬間の値でライブに読む関数）が、
「イベントのライブ配送」と「`OUTPUT_GATE` active 中に `INPUT_DEFER` へ
退避されたイベントの drain replay」の両方から同一コードパスで呼ばれるため、
drain replay 時に「イベント発生時点の値」ではなく「replay を実行している
"今"の値」を読んでしまう、という欠陥を確定させた。この修正（`RawKeyEvent`
に `left_thumb_down_snapshot`/`right_thumb_down_snapshot` を追加し、
`hook.rs::build_raw_key_event` の capture 時点で埋め込む）自体は**未実装**
（`grep -rn thumb_down_snapshot` はコード中に1件もヒットしない、2026-09-08
時点）。

### 訂正1: `thumb_down_timestamps()` の呼び出し箇所は2つではなく3つ

初版は「gate 非 active 時の直接発火は `build_ctx()` を経由する」と書いて
いたが誤り。**round2 レビューで、その訂正自体も過剰訂正だったと判明した
ため本版でさらに訂正する。**

`Runtime::build_ctx()`（`runtime/mod.rs`）は **12箇所**（`runtime/mod.rs`
8箇所、`message_handlers.rs` 2箇所——`begin_key_batch` と
`handle_wm_drain_output_queue` の deferred timer replay、
`runtime/ime_refresh.rs` 2箇所）から呼ばれる**共有関数**である。この
うち `deferred_engine_timers` の replay に使われているのは
`handle_wm_drain_output_queue`（replay ループの**前に1回だけ** `let
ctx = app.build_ctx();` を呼び、全エントリで使い回す）の1箇所のみ。
**gate 非 active 時にタイマーが直接発火する経路**（`handle_wm_timer` の
`Some(timer_id) =>` 分岐、gate 判定を通過した場合）は `build_ctx()` を
呼ばず、`read_os_modifiers()` → Alt なりすまし補正 → `hook::
thumb_down_timestamps()` → `super::build_input_context(...)` という
**同じ処理を手書きで複製**している。したがって `thumb_down_timestamps()`
の呼び出し箇所は現に3つ: `runtime/mod.rs::build_ctx`、
`message_handlers.rs`（タイマー直接発火経路の複製コード）、
`key_pipeline.rs::kp_run_inner`（キーイベント経路）。

これは「未決着論点3」（`architecture_guard.rs` の許可箇所を `build_ctx`
1箇所に縮小できるか）の前提を崩す——**現状のコード構造のままでは縮小
できない**。縮小するには、まずタイマー直接発火経路の手書き複製を
`build_ctx()` の呼び出しに置き換える独立したリファクタが要る。

**`build_ctx()` が12箇所から呼ばれる共有関数であるという事実は、下記
「未決着・要レビュー論点」1（シグネチャ変更の是非）に直結する**——
呼び出し元が多数ある以上、`build_ctx()` 自身のシグネチャを変えて
スナップショットを渡せるようにする案は、本 ADR のスコープ（タイマー
replay のみ）を超えて影響範囲が広がる。案Aを採る場合、`build_ctx()`
本体は変えず、タイマーキューのエントリ側にスナップショットを持たせて
replay 側で戻り値を部分的に上書きする方が変更を局所化できる。

### 訂正2: gate 条件は `OUTPUT_GATE` だけではない

`handle_wm_timer` がエンジンタイマーを `deferred_engine_timers` へ退避する
条件は `crate::OUTPUT_GATE.is_active() || crate::focus_resync::
FOCUS_RESYNC.is_gate_active()`（BUG-77 コードレビュー追補で `FOCUS_RESYNC`
が追加された）。初版は `OUTPUT_GATE` のみを前提に書いていた。

### 訂正3: replay 側の `InputContext` はループの外で1回だけ構築される

`handle_wm_drain_output_queue` の replay ループは `let ctx =
app.build_ctx();` をループの**外**で1回だけ呼び、`deferred_engine_timers`
の全エントリに同じ `ctx` を使い回す（`for (timer_id, os_id) in deferred {
... app.engine.on_timeout(timer_id, &ctx) ... }`）。「エントリごとに defer
時点のスナップショットを使う」という決定を実装するには、この `ctx` 単一
構築という既存構造自体を変える必要がある——本 ADR の決定節はこの点を
反映していなかった。

## 中心的な問題: 本 ADR が想定する失敗シナリオは、到達可能性が実証されていない

`handle_wm_drain_output_queue` の実行順序は次のとおりである。

1. `crate::INPUT_DEFER.take_all()` → 退避されていたキーイベントを**先に**
   FSM へ流す（`deliver_key_event` 経由）。
2. その**後**で `deferred_engine_timers` を `std::mem::take` → 上記の
   共有 `ctx` で replay する。

replay 直前には `current_os_id(timer_id) == os_id` という照合ガードがあり
（コード中のコメントが「drain 中に『古いタイマー kill → 新タイマー set』が
起きると `logical_id` は `is_active=true` のままだが別の文字に属する新規
タイマーになる。新規タイマーを早期発火させると文字順が狂うのを防ぐため」
と明記している）、本 ADR が懸念する「replay 時点でたまたま押されている
**別の**押下」が `phys` に混入するには、その別の親指押下の KeyDown が
手順1で先に FSM へ流れてもなお、対象タイマーの `os_id` が変化せず
（= FSM が kill/re-set していない）、かつ FSM が `PendingCharThumb` 相当の
状態のまま生き残っている、という2条件を**両方**すり抜ける必要がある。

到達可能な具体的な変種を洗った結果:

- **親指 KeyUp のみが drain 中に処理された場合**: グローバル
  （`LEFT/RIGHT_THUMB_DOWN_AT_US`）が `None` 相当になる →
  `NicolaFsm::is_thumb_consumed`（`phys_down.is_some() && consumed ==
  phys_down` という判定）は `phys_down` が `None` の時点で不成立 →
  **実害なし**。
- **`hook.rs` のグローバルクリア系関数**（`clear_hook_latches_for_app_
  disable`/`set_thumb_vk_codes`/`reset_physical_key_state`）による
  ゼロクリア: 対応する FSM イベントを伴わずに到達しうるが、同じく
  `phys_down = None` になるだけで **実害なし**。
- **実害がある変種**（無関係な新しい押下が誤って「消費済み」と刻印される）:
  上記2条件（`os_id` 一致・FSM 状態維持）を両方満たす具体的なイベント列を
  round1 レビューで探したが、1本も構成できなかった。

**「未観測・コードからの理論的特定のみ」という初版の位置づけは過大評価
だった。正確には「理論的にも未確立」である。** `.claude/rules/
tuning-constants.md` が禁じる「効かないので増やした」型の対症変更と
同じ構造的リスクがある——実証されていない失敗シナリオへ実装コストを
払うべきではない。

## 決定

### クローズ: 実装に進まない。次の担当者への引き継ぎ事項として以下を残す

1. **✅ 確認済み（本ADR自身のround1レビューで実施、2026-09-08クローズ
   判断の根拠）**: `os_id` 照合とドレイン順序（`INPUT_DEFER` を先に
   flush → その後で `deferred_engine_timers` を replay）を両方すり抜ける、
   具体的なイベント列（VK・タイミング・FSM 状態遷移込み）を構成できるかを
   確認した。「中心的な問題」節が記録するとおり、到達可能な変種をすべて
   洗った結果、実害のある変種を1本も構成できなかった。**対象の失敗クラス
   自体が到達不能である可能性が高いと判断し、`docs/known-bugs.md`
   BUG-126 への軽い記録に留めて本 ADR をクローズする。**
2. **今後、構成できた場合、あるいは実機で再現した場合**は、下記「将来の実装
   案（未採用のまま記録）」を出発点に設計し直すこと——ただし後述の
   「決定1未満の優先論点」を先に解決すること。BUG-126 に追記の上、本ADRを
   再オープンする。

### 将来の実装案（未採用のまま記録、round1 で発見された代替案を含む）

**案A（初版の案、当初決定）**: `deferred_engine_timers` の push 時点で
`hook::thumb_down_timestamps()` を1回読み、`(timer_id, os_id,
left_thumb_down_snapshot, right_thumb_down_snapshot)` として保持する。
replay 側はこのスナップショットで `InputContext` の該当2フィールドを
上書きする。この案を採る場合、上記「訂正3」により `ctx` の単一構築
構造をエントリ単位の構築へ変える実装コストが伴う。

**案B（round1 で新規発見、より根本的。round2 で発火順序を訂正）**:
`hook.rs` 側で「同一キーイベントに対して `now_timestamp()` を複数回
呼ばない」よう改める。フックコールバック内の実行順は次のとおり
（round2 で訂正——初版は逆順に書いていた）: まず `update_thumb`
クロージャ（`vk == config.left/right_thumb_vk` かつ非 injected の
場合のみ）が `slot.store(now_timestamp(), ..)` で **グローバルを先に
更新**し、その**後**に `build_raw_key_event(...)` 呼び出しが
`timestamp: now_timestamp()` で `RawKeyEvent.timestamp` を決める
（**イベント自身の timestamp の方が後**）。**同一の物理押下に対して
`now_timestamp()` が2回呼ばれ、数 µs ずれた別の値になる。** この2値を
どちらも「その押下の時刻」として比較に使おうとすると（`is_thumb_
consumed` のような等値比較）、原理的に一致しない。

案Bは、親指キーの KeyDown を処理する箇所で `now_timestamp()` を1回だけ
呼び、その値を `RawKeyEvent.timestamp` とグローバル両方へ同じ値として
書き込む——ただし、これを実装可能な規定にするには最低限次の2点を
先に決める必要がある（round2 で発見、未解決のまま記録）。

1. **キーリピート**: `update_thumb` は `prev == 0` のとき（＝新規押下）
   のみ `store` する（押しっぱなしで届く2回目以降の KeyDown では
   グローバルを更新しない）。一方 `RawKeyEvent.timestamp` は
   auto-repeat の KeyDown ごとに毎回新しい値になる。「同じ値を両方へ
   書く」は auto-repeat 中の KeyDown には文字どおり適用できない——
   案Bはこの場合「グローバルへ新規に書かず、格納済みの値を読み戻して
   `RawKeyEvent.timestamp` 側に採用する」という非対称な規定にする
   必要がある。
2. **injected イベントの扱い**: `update_thumb` は `!is_injected` の
   条件下でのみ呼ばれるが、`build_raw_key_event` は injected な
   イベントに対しても呼ばれる。injected な親指 KeyDown の
   `timestamp` が何と比較されるべきか（グローバルは更新されないため
   比較対象自体が無い）を、案Bの実装前に定義する必要がある。

これらが解決すれば、`PendingThumbData.timestamp`（対象押下の
`RawKeyEvent.timestamp` をそのまま保持）を直接 `Some(thumb.timestamp)`
として使え、グローバル `AtomicU64` 経由のライブクエリ機構自体（案A・
ADR-129 決定・本 ADR が扱ってきた仕組み全体）を代替できる可能性がある。

**優先順位に関する注記**: 案Bは [ADR-129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md)
自身の実装（`left/right_thumb_down_snapshot` フィールド追加、まだ未着手）
にも影響する——ADR-129 の決定がそのまま実装されると、案Bが解消しうる
「二重 `now_timestamp()` 呼び出し」という根本問題を型で覆い隠したまま
新しいフィールドだけが増える。**ADR-129 の実装に着手する前に、案A/案Bの
どちらを土台にするかを判断すべき順序依存がある。** 本 ADR 単独では
どちらを推奨するかを決定しない（実機再現/具体的失敗シナリオが無い以上、
実装判断自体を保留しているため）。

## 未決着・要レビュー論点

1. **上書きの実装形態**（案A採用時）: `build_ctx()` の戻り値を丸ごと
   使うか、`InputContext` の該当2フィールドだけをタイマー側で差し替える
   か。
2. **タイマー直接発火経路の手書き複製の解消**: 「訂正1」で確認したとおり、
   `message_handlers.rs` のタイマー直接発火経路が `build_ctx()` を呼ばず
   ロジックを複製している。本 ADR のどの案を採るにしても、まずこの複製
   を `build_ctx()` 呼び出しへ統合すべきかを判断すること。
3. **回帰テストの置き場所**: 案Aを採る場合、`tests/architecture_guard.rs`
   のテキスト走査ガードで `thumb_down_timestamps()` の呼び出し許可箇所を
   `build_ctx` 1箇所（論点2 が先に解決していれば）に縮小できるか確認する。
4. **[[keymap]] 等、他のタイマー系（`deferred_engine_timers` 以外）に同型の
   ライブクエリが残っていないかの棚卸し。** [ADR-156](156-unify-deferred-execution-queues.md)
   参照。

## テスト

実装に進んでいないため未着手。実装に進む場合は `fix-requires-evidence.md`
のキー選択/warmup ファミリーに該当するため、(a) 回帰テストまたは
(b) known-bugs.md 記録の少なくとも一方が必須。単体テストで
`NicolaFsm::on_timeout(event, phys)` に新旧2つの `phys` を渡す形は、
ADR-129 が同型のケースで却下した理由（「バグの実体は `phys` を作る側に
あり、`on_timeout` 自体の比較ロジックは健全」）と同じ理由で不採用とする。

## 関連

[ADR-129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md)
（本 ADR が引き継ぐ「限界」節・未決定事項2の出所、キーイベント経路の
決定は未実装。案Bとの順序依存あり）、
[ADR-010](010-thumb-consumption-timestamp.md)（`Option<Timestamp>`
による親指消費追跡、比較ロジック自体は健全と確認済み）、
[ADR-008](008-physical-thumb-state-separation.md)（物理親指キー状態と
FSM 解決ロジックの分離）、[ADR-156](156-unify-deferred-execution-queues.md)
（`deferred_engine_timers`/`INPUT_DEFER`/`pending_deferred` の構造的な
共通パターンを扱う将来構想）。
