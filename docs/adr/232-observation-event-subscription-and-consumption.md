---
id: ADR-232
title: |-
  Observation/Event の購読と消費
summary: |-
  ADR-229「世界モデルの reduce 化と Effect 列の設計の検討結果」で未起草として残した「観測と Event の購読・消費」(Redux/Elm/Rack/ASGI から借りる案: 単一 Envelope = source/seq/time/scope/confidence、FocusScope、`subscriptions(&Model)`、ミドルウェア列)を、実害の記録(docs/known-bugs・既存 ADR)と W0 の実測(docs/tasks/world-model-write-inventory-2026-10-06/)に当てて選別した。主目的は撤去で、新しい型・trait・機構は足さない。
  採る: (D1) 起動時のフォーカス確立(BUG-081 で作った別経路)で、定常の `FocusChanged` が入れる「スコープの同一性」のフィールドが足されるたびに漏れ、Event が 1 つずつ足された(BUG-102 fence・BUG-114 根本原因1 `app_policy`・BUG-148 `current_focus` の 3 件)。3 つの `Initial*` を 1 つにまとめ(ADR-134 D1c の元の設計に戻す)、`app_policy`・`current_focus` の代入を `reduce_` で始まる共有関数にし、モデル全体の `Debug` 比較の等価性テストで将来の漏れを捕まえる。(D2) 本番に読み手のいない `ImeEventLog` の 512 件のリングを撤去し、`seq` の採番だけを残す。(D3) `InputModeObserved`/`InputModeApplied` の `at: TickMs` は本番の全 7 か所で Envelope の `tick_ms` と同じ値なので撤去する。(S3) `on_focus_process_changed` の死んだ `reset_detect_state` 呼び出しを消す。
  採らない: 全 Event 共通の Envelope、時刻の入口の一本化、admission の集約、フォーカスの入口と 9 種の型の 1 構造体化、フォーカス hwnd の写しの統合、`subscriptions(&Model)`、ミドルウェア列、全書き込みの reduce 化と journal からの完全再生、起動時プローブ失敗の窓(D1 でも残る残余)など。各々に考え直す条件を書く。
status: |-
  起草(2026-10-05)。Opus round1(Blocker なし、Must 3・Should 8・Nit 6)を反映。コードは変えていない。
related_adr:
  - "ADR-229"
  - "ADR-032"
  - "ADR-102"
  - "ADR-104"
  - "ADR-106"
  - "ADR-134"
  - "ADR-170"
  - "ADR-186"
  - "ADR-224"
  - "ADR-225"
  - "ADR-218"
  - "ADR-219"
  - "ADR-220"
  - "ADR-180"
---

# ADR-232: Observation/Event の購読と消費

## 目的(何を撤去するか)

ADR-229 の「世界モデルの reduce 化と Effect 列の設計の検討結果」節は、Redux/Elm/Rack/ASGI から借りる「観測と Event の購読・消費」の案を、W0 の実測で前提が裏づけられたが未起草として残した。本 ADR はその案を、**実害の記録に結びつくものだけ**に絞る。

主目的は新しい仕組みの導入ではなく、**起動時のフォーカス確立のために BUG ごとに 1 つずつ足された Event・関数・ガードの撤去**(D1)と、**読み手のいない記録・二重に持つ時刻・死んだ呼び出しの撤去**(D2・D3・S3)である。成功の基準は、下の「撤去の目録」の項目が実際に消えた数で測る。新しい型・trait・汎用の部品は 1 つも足さない。

**純減の撤去の採用基準**(Opus round1 §1): 実害の記録を要求するのは新しい機構の追加だけとし、実害の記録の無い撤去は次の 3 つを満たすときに採る。(i) 挙動不変を読みで示せる、(ii) テストと docs を含めて行数が減る、(iii) 再発ファミリーに触れるなら、その PR で何を固定するかを書く。

実測の基準: `origin/develop` `b3fbe315` で読み、`d703c5d0`(#515 のマージ後)に載せ直した。行番号は `b3fbe315` 時点で、#515 で数行ずれうる。読み取りのみ、ビルドとテストはしていない。件数は `git grep` による。

## 背景(事実)

### W0 の実測(`docs/tasks/world-model-write-inventory-2026-10-06/`)

- `ImeEvent` は W0 時点で 20 variant。本番の発行元の無い `UserImeToggleIntent`・`UserChangedInputMode` は #515 で削除済み。`reduce` を通るのは `ImeStateHub` のうち `shadow_model` だけ、`&mut self` のメソッド 53 個のうち 27 個は Event を出さない。
- Event の入口は `ImeStateHub::dispatch_event`(`state/platform_state.rs:167`)の 1 か所で、手順は「`last_user_explicit_off_ms` の更新 → `event_log.record_at`(`EventTime{seq, monotonic, tick_ms}` の採番)→ `reduce` → `journal.record`」の固定順(依存辺 e9、`docs/tasks/effect-signature-inventory-2026-10-06/dependency-edges.md`)。
- 観測の Envelope に当たるものは**既にある**: `AnyObservation { open, source, hwnd, confidence, focus_epoch }`(`state/evidence.rs:270`)。時刻と seq の Envelope は `EventTime`。`ImeEvent` の型の doc(`ime_event.rs:368`)は「時刻情報は `ImeEventEnvelope::time` に集約する(event 内に重複させない)」と書いている。非同期の観測は、各呼び出し元が `admit_epoch_in_app` で古いフォーカスの結果を捨ててから適用する(ADR-106。`key_pipeline.rs:332/574/2853`、`focus_tracking.rs:837`)。
- 時刻の入口は 3 つ(`EventTime`・`HubClock` の直読み・呼び出し側の `Instant::now()`/`current_tick_ms()`)。journal の `ImeEvent` には W-c(#505・#508・#516)で `event_seq`・`tick_ms` が載った。`Instant` は載らない。
- `ImeEventLog`(512 件のリング、ADR-032 の「Step 0」)は**本番に読み手がいない**。`recent`/`recent_vec`/`iter`/`len`/`is_empty` の呼び出しはテストだけ(`ime_event_log.rs` のテストと `platform_state.rs:1868`)。本番が使うのは `next_seq()`(`:711` の `try_set_focus_transition_barrier`、`:1156` の `apply_panic_reset` の `ForceGuard.generation`)と `record_at` の戻り値の `EventTime` だけ。Shadow Reducer(`shadow_model`)は `dispatch_event` が直接 `reduce` していて、リングを読まない。
- 観測のソースは起動時に固定で据え付けられ、モデルに応じて起動・停止していない: WinEvent のフック 3 本(`tsf/win_event_obs.rs::install_observation_hooks`)とフォーカスのフック(`app/bootstrap.rs:826`)、`gji-io-monitor`(`bootstrap.rs:1318` で無条件に起動)。動的なのはタイマー駆動の読み取り(`TIMER_IME_REFRESH`・idle-conv-check・`start_ms_ime_ready_poll`・warmup)だけで、どれも自己再スケジュールと in-flight の印を既に持つ。

### 実害の記録: 起動時のフォーカス確立で、同一性のフィールドが 3 回漏れた

定常のフォーカス変更は `on_focus_process_changed` → `ImeEvent::FocusChanged { from, to, profile, focus_epoch }` で、reducer(`ime_model.rs::reduce_focus_changed`)が `app_policy`・`current_focus` を入れ、`last_intent`・`key_effect`・`key_track` を消してから `observations.clear_on_focus_change(FocusFence{epoch: focus_epoch, hwnd: to})` で fence を入れ、続けて `applied=Unknown`・`force_guards`・`input_barrier` などを入れる。起動時は BUG-081 の修正で作った別経路 `establish_initial_focus_scope`(`focus_tracking.rs:109`)が走り、ADR-102 決定3-b(最初の IME 観測より前に belief を書かない)のために `FocusChanged` を出さない。その結果、**`FocusChanged` が入れる「スコープの同一性」のフィールドが、起動時の側で 1 つずつ漏れた**。

| BUG | 漏れたもの | 足したもの(D1 の撤去対象) |
|---|---|---|
| BUG-102 | `observations.current_fence`(起動時のアプリの `ImmCrossProbe`〈High〉が導出から外れ、Medium に負ける) | `ImeEvent::InitialFocusFenceEstablished`、`sync_initial_focus_fence`、ガード 1・単体テスト 1 |
| BUG-114 根本原因1(ADR-134 D1c) | `app_policy`(既定の `Read` のまま、TsfNative で `VK_IME_OFF` を再送し続けた。実機確認済み) | `ImeEvent::InitialAppPolicyEstablished`、`sync_initial_app_policy`、ガード 1・単体テスト 1 |
| BUG-148(ADR-186) | `current_focus`(`None` のまま明示意図が記録されず、委譲 `SetOpen` が全て `Unwarranted`。CI で再現) | `ImeEvent::InitialFocusHwndEstablished`、`sync_initial_focus_hwnd`、ガード 1・単体テスト 1 |

BUG-081 は同じ型の 1 件ではなく、別経路そのものを作った起源(`establish_initial_focus_scope`。D1 でも残す)。

3 つの `Initial*` Event は、いずれも同一性のフィールドを 1 つだけ入れ、ガードは「その Event が 1 フィールドしか触らない」を固定する。**「`FocusChanged` が入れる同一性のフィールドを、起動時の側も全部入れる」ことは何も固定していない**ので、4 つ目のフィールドが足されたら同じ型の BUG が 4 件目として出る。なお、ADR-134 D1c の元の設計は、`profile` を `InitialFocusFenceEstablished` に載せる(=1 つの Event にまとめる)案だった(`134-*.md` の D1c)。別 Event への分割は BUG-114 の実装時の選択で、D1 はその元の設計に戻る。

## 決定

### D1: 起動時のフォーカス確立を 1 つの Event にし、同一性の代入を `FocusChanged` と共有する(実害: BUG-102・114・148)

- 3 つの `Initial*` を、`FocusChanged` から `from` を除いたのと同じ形の 1 つの Event にまとめる: `InitialFocusScopeEstablished { to: HwndId, profile: ImePolicyProfile, focus_epoch: FocusEpoch }`(フィールド名・型は `FocusChanged` と同じ。新しい型は作らない)。起動時の 3 つの値が `FocusChanged` と同じ式で取れることは確認済み(`advance_focus_tracking` → `CurrentFocus::update_with_process_name` が `classified.hwnd` をそのまま `current.hwnd` に入れる〈`focus/current.rs:64`〉ので `focus_fence().hwnd == classified.hwnd`。`profile` は `current_app_profile()`、epoch は `platform_state.focus.focus_epoch`)。
- reducer(M3 で確定):
  - 共有する関数は `reduce_focus_changed` の**先頭 2 行**(`app_policy`・`current_focus`)だけを持ち、名前は `reduce_` で始める(例 `reduce_scope_identity`)。`architecture_guard.rs::reduce_helpers_are_called_only_from_reduce_body` が `fn reduce_` の定義を自動で拾い、`reduce()` の本体以外からの呼び出しを禁じる(ADR-170 の規約)ので、この関数も `reduce()` の外から呼べない。
  - **fence は共有関数の外**(各腕)に置き、今の 2 つの口をそのまま呼ぶ: 定常は `clear_on_focus_change`(観測プールと drift を消す)、起動時は `establish_initial_fence`(消さず、1 回だけの `debug_assert!` つき)。fence を共有関数に入れると、起動時に観測プールと drift(belief の根拠と drift correction の入力)を消すことになり ADR-102 決定3-b に触れ、`establish_initial_fence` の 1 回性の検査も消える。
  - `FocusChanged` の腕だけが、続けて belief 側のリセットを行う。
- 呼び出し元: `sync_initial_focus_fence`・`sync_initial_app_policy`・`sync_initial_focus_hwnd` の 3 関数を 1 つにする。統合した関数は、`advance_focus_tracking` の**後**(`current_app_profile()` が確定してから)に呼ぶ、という今の順序の要件を引き継ぐ。`dispatch_event(` を `establish_initial_focus_scope` の本体に直接書かない、という約束(`focus_tracking.rs:229-233` の doc)も保つ。ログは 3 行(`[focus-fence] bootstrap initial fence`・`[app-policy] bootstrap initial app_policy`・`[focus] bootstrap initial current_focus`)を、3 つの値(fence・profile・hwnd)を載せた 1 行にする(BUG-114 本文はこのログ行を解決の証拠に使っている。`tools/e2e/ime_key_matrix/test_log_anchors_in_rust_source.py` の ANCHORS には無いので CI は壊れない)。
- 固定するテスト(すべて `state/ime_model.rs` の単体テストで Linux で回る。3 と 4 は既存の書き換え):
  1. **等価性**(M1): `a`・`b` を `ImeModel::new()` で作り、同じ envelope で `a` に `FocusChanged{from: None, to, profile, focus_epoch}`、`b` に `InitialFocusScopeEstablished{to, profile, focus_epoch}` を通す。**起動時は違ってよいフィールドを手で挙げて** `a` から `b` へ写し(現状は `input_barrier` だけの見込み。`key_track` は `new()` の値と `KeyTrack::default()` が一致するかを実装時に確かめ、違えば一覧に足す)、`format!("{a:?}") == format!("{b:?}")` を要求する。`ImeModel` は `#[derive(Debug)]` なので、`reduce_focus_changed` に既定値以外を入れる新しいフィールドが足されると、このテストが落ち、「同一性か(起動時も入れる)/belief のリセットか(一覧に足す)」の判断を強制する。一覧に `input_barrier` が載ること自体が、「起動時は `FocusTransition` の settle を立てない」という今の意図の明文化になる。
  2. **belief 不変**(M2): 既存の 3 本と同じ方式で 1 本にする。`fully_populated_model` に `app_policy`・`current_focus`・fence(`establish_initial_fence` で同じ値)を目的の値で入れてから `InitialFocusScopeEstablished` を流し、**モデル全体の `Debug` 表現が変わらない**ことを要求する。あわせて、未確立のモデルに流すと 3 つが入ることを確かめる。個別のフィールドを手書きで並べる形にはしない(`desired_is_placeholder` などが漏れるため)。
  3. BUG-102 の回帰テスト `bootstrap_fence_desync_lets_medium_poll_override_high_probe`(`ime_model.rs`)と BUG-148 の回帰テスト `initial_focus_hwnd_lets_explicit_intent_be_recorded_before_first_focus_change`(`platform_state.rs`)は**消さず**、Event 名だけを書き換える(BUG ファイルから参照されている)。
  4. `architecture_guard.rs`: touches-only の 3 本(`initial_focus_fence_event_only_touches_the_fence`・`initial_app_policy_event_only_touches_app_policy`・`initial_focus_hwnd_event_only_touches_current_focus`)を、`InitialFocusScopeEstablished` の dispatch が 1 か所だけであることを固定する 1 本にする。`establish_initial_focus_scope_does_not_write_ime_belief`(`EXEMPT` と対象関数リスト、`dispatch_event(` ちょうど 1 件の assert 3 組、`advance_focus_tracking` の後に呼ぶ順序の assert)は 1 関数分に書き換える。`bootstrap_initial_focus_scope_precedes_ime_cache_initialization`(`run_all` での呼び出しが 1 回)は残す。新しい起動時経路の追加を捕まえるのは、等価性テストではなくこれらのガードの役目。
- docs の書き換え: `ime_model.rs` の `current_focus` の doc(「`FocusChanged` の reducer でのみ更新する」は BUG-148 以降すでに誤り)、`observation_store.rs:506`・`probe_admission.rs:130`・`transition.rs:35` の Event 名。
- 3 つの Event は 1 回の `with_app` の中で同期的に続けて dispatch されており(`bootstrap.rs:1324`)、途中の状態が他から見える窓は無い。`ImeEvent` は `Deserialize` を持たないので、旧 journal の読み込み互換の問題も無い。

### D2: `ImeEventLog` のリングを撤去し、`seq` の採番だけを残す(挙動不変の撤去)

- `ImeEventLog` から `VecDeque` のリング・`capacity`・`DEFAULT_CAPACITY`・`len`/`is_empty`/`recent`/`recent_vec`/`iter`・`Instant::now()` を読む `record()` を削る。残すのは `next_seq`(本番の 2 か所)と、`EventTime` を返す採番。型名 `ImeEventLog` は残し(改名すると docs と ADR に波及する)、doc を「採番器」に直す。`tracing::trace!("[ime-event seq=…]")` は残す。
- `dispatch_event` の `event.clone()` は 2 回とも消せる(`envelope` を作って `reduce(&envelope)` し、その後 `journal.record` に `envelope.event` を move する)。
- 情報は失われない: 再生や不具合報告に使う記録は journal(W-c で `event_seq`・`tick_ms` つき)で、リングは一度もダンプされていない(W0-a §8-1)。`ImeStateHub` 全体を `{:?}` で出す箇所も無い。
- 副次: `ime_event_log.rs` の Tier-2 違反は `Instant::now()` だけ(ADR-229 F-D6 の違反 8 ファイルの 1 つ)なので、撤去後に `CORE_MODULES` に足せる(違反のある ungated ファイル 8 → 7)。
- テスト `manual_hub_clock_drives_event_monotonic`(`platform_state.rs:1850`、`HubClock` の `Instant` が `dispatch_event` の envelope に届くこと)は、journal には `Instant` が無く、`record_at` の戻り値を直接見ると `dispatch_event` の配線が検査から外れるので、どちらでも書き直せない。reduce の結果に `monotonic` が現れる Event で確かめる: 時計を 500ms 進めて `FocusChanged` を 2 回 dispatch し、`input_barrier` の `started_at`(`= envelope.time.monotonic`)の差を見る。
- docs の書き換え: ADR-032(リングの出自)に追記、`docs/layer-boundaries.md` の C-6(「全 `ImeEvent` dispatch は `event_log.record()` 経由で seq」)、`docs/ime-control-overview.md` の「512 エントリリングバッファ」、`layer_boundary_guard.rs` の C-6 のメッセージ文字列、`ime_model.rs:493` の doc、`ImeStateHub.event_log` のフィールド doc(`platform_state.rs:39`)。

### D3: `InputModeObserved`/`InputModeApplied` の `at: TickMs` を撤去する(挙動不変の撤去)

- 本番の構築箇所 7 か所(`InputModeObserved`: `ime_refresh.rs:191`・`key_pipeline.rs:822`・`key_pipeline.rs:2883`・`platform_state.rs:1221`、`InputModeApplied`: `platform_state.rs:1141`・`platform_state.rs:1283`・`runtime/mod.rs:1123`)は、どれも `at` に `dispatch_event` の第 2 引数と同じ値を渡している。journal の JSON から `at` を読む消費者(report-worker・tools・skills)は無く、`tests/journals/` に該当の fixture も無い。
- `ime_event.rs:600-601` の doc は「非同期 probe が完了した時刻を明示したい場合は別値になることがある」と、別値の用途を想定していた。その用途は envelope 側で既に満たされている: 開閉の軸(`ObserverReported`)は既に `envelope.time.tick_ms` で照合し(`ime_model.rs:796-800`)、非同期の観測は呼び出し側が読み取り開始時刻を `dispatch_event` の `tick_ms` として渡す(例 `focus_tracking.rs:833` の `read_started`)。`at` は「時刻を event 内に重複させない」という型の doc(`ime_event.rs:368`)の例外になっていた。
- reducer の読み手 `reconcile_key_effect_mode(mode, at.0)`(`ime_model.rs:842`)などを `envelope.time.tick_ms` に置き換える。
- テストの構築箇所で `at` と envelope の tick が違うもの: `tests/golden_scenarios.rs` の 18 か所(`at: TickMs(0)`、envelope の tick は `seq * 10`)。golden には `KeyEffectPredicted` が無く `key_effect=None` なので、`reconcile_key_effect_mode` は冒頭で `true` を返し(`ime_model.rs:693-696`)、結果は変わらない。閉ループの harness(`tests/support/harness.rs:363-367`)と `ime_model.rs` の単体テストは同じ値。書き換え対象は golden 18・`ime_model.rs` 約 10・harness 1・`platform_state.rs` 1(いずれも 1 行の削除)。

## 撤去の目録(成功の基準)

| # | 撤去するもの | 段階 |
|---|---|---|
| 1 | `ImeEvent::InitialAppPolicyEstablished`・`InitialFocusHwndEstablished`(`InitialFocusFenceEstablished` は `InitialFocusScopeEstablished` に改名して残す) | S2 |
| 2 | `sync_initial_app_policy`・`sync_initial_focus_hwnd`(1 つに統合) | S2 |
| 3 | touches-only の単体テスト 3 本 → belief 不変 1 本(全体比較)+等価性 1 本。touches-only のガード 3 本 → 1 本。`establish_initial_focus_scope_does_not_write_ime_belief` を 1 関数分に縮める。BUG-102/148 の回帰テスト 2 本は Event 名の書き換えのみ | S2 |
| 4 | `ImeEventLog` のリング・`record()`・読み出しの 5 メソッド(`next_seq` と本番の 2 つの呼び出し元は残す) | S1 |
| 5 | `dispatch_event` の clone 2 回 | S1 |
| 6 | `InputModeObserved`/`InputModeApplied` の `at` | S1 |
| 7 | `on_focus_process_changed` の `reset_detect_state()` の条件付き呼び出し(`focus_tracking.rs:858-867`)。死んだコードと確認済み: `FocusChanged` の reduce が同じ 2 つ(`force_guards.clear`・`observe_miss_monitor.record_success`)を行った後、この行までに同期的に走るのは `apply_hwnd_cache_restore`・`reset_stale_ime_on_for_imm_broken`/`assume_closed_for_new_thread`・`presync_applied_open_on` だけで、本番の `force_guards.add`(`apply_panic_reset` のみ)も `record_miss`(`apply_ime_update` のみ)も呼ばれない | S3 |

新しく足すもの: private な `reduce_` 関数 1 つ(D1)と等価性のテスト 1 本だけ。**新しい型・trait・Event の種類は 0**。行数は実装 PR で実測して本文に書く(ガード 3 本〈各約 50〜70 行〉と touches-only 3 本〈各約 40 行〉が 1 本ずつになるので、等価性テスト〈約 30 行〉を足しても純減の見込み)。旧来の Event・関数・ガードを残したまま新しいものを並べたら失敗とする。

## 採らない案と理由

| 案(借りる元) | 採らない理由 | 考え直す条件 |
|---|---|---|
| 全 Event に共通の Envelope(source/seq/time/scope/confidence) | `ImeEvent` は belief の reducer への入力で、意図(`UserImeSetIntent`)や書き込みの結果(`ImeApply*`)には source・confidence の意味が無い。観測の Envelope は `AnyObservation`、時刻と seq の Envelope は `EventTime` として既にある。全 Event に同じ包みを付けるのは toolkit round1 が却下した `Tagged<T>` と同じ(ドメインの区別を消す) | 観測でない Event に source/confidence が要る判断が実際に書かれたとき |
| 時刻の入口の一本化(`dispatch_event` の `tick_ms` 引数をやめ、hub が `HubClock` から読む) | 実害の記録が無い。`on_ime_apply_complete` は完了メッセージの時刻 `ts`、非同期の観測は読み取り開始時刻を渡していて、hub の「いま」とは意味が違う(挙動変更になる)。`Instant` を journal に載せる件も、journal からの完全再生を目標にしない(下記)ので要らない | 時刻の取り違えが原因の BUG が記録されたとき |
| admission(古いフォーカスの結果を捨てる)を Envelope の段で 1 回に集める(Rack/ASGI のミドルウェア的な前処理) | 判定は既に 1 つの関数(`admit_epoch_in_app`)で、呼び出し元ごとに書く理由がある(dispatch だけでなく、`apply_focus_probe` などの適用処理全体の前で捨てる。`key_pipeline.rs:332` ほか)。`ObservationStore::is_identity_ok` は読み出し時の鮮度の判定で、役割が違う。BUG-091 は計測だけで実害は未確認、BUG-102 は fence の写しのずれで admission の問題ではない(D1 で扱う) | 呼び出し元が admission を忘れた BUG が記録されたとき |
| FocusScope を、フォーカスの入口 4〜5 つ(WinEvent・デバウンス後のポーリング・同一プロセス内の hwnd 変化・モーダルポンプ・電源復帰)と 9 種の型にまたがるリセットを 1 つの構造体に集める形にする | W0-b §6.4 のとおり状態の寿命が 3 種類(フォーカスで明示的に消す/`ForegroundScope` で自動失効/世代で失効)あり、1 つにまとめると意味が変わる。WinEvent の入口が `ime_mode_focus_gen` を進めない窓(約 50ms)で古い世代の `spawn_local` が効いた実害の記録は、known-bugs に無い(BUG-037 は世代の問題ではなく ADR-208 で置き換え済み)。focus 遷移は再発ファミリーで、実害の無い大きな組み替えは逆効果 | (a)〜(b) の窓で古い世代の `spawn_local` が誤って効いた BUG が記録されたとき |
| フォーカス hwnd の 4 つの写し(`FocusTracker.current.hwnd`・`current_fence.hwnd`・`ImeModel.current_focus`・`LAST_FOCUS_HWND`)を 1 つにする | 実害の記録は起動時のずれ(BUG-102・148)だけで、それは D1 で扱う。`FocusHwndUpdated` が fence だけを動かし `current_focus` を動かさない(IntentStore の粒度が実質 per-process になる)ことは、BUG-051 が意図的なスコープ外の残存リスクとして開示済みで、実害の記録は無い。D1 の等価性テストは `FocusHwndUpdated` を対象にしないので、この非対称を誤って変えることは無い | 同一プロセスの別ウィンドウ間で明示意図の取り違えが記録されたとき |
| 起動時プローブ失敗の窓を塞ぐ(`classify_focus_probe` が `None` で `establish_initial_focus_scope` が即 return すると、次の定常プローブは `last_pid = None` で `process_changed = false` になり、`FocusChanged` が初めて出るのは 2 回目のプロセス切替。それまで `current_focus=None`・`app_policy` 既定値・fence 既定値のまま) | BUG-081 と同じ型の残余で、D1 でも直らない。ADR-134 D1c は残余を「D1(ライブ再導出)が拾う」としたが、その D1/D1a は未実装。実害の記録は無いので採らない。D1 で関数が 1 つになれば、直すときは定常経路の「`last_pid = None` かつ `focus_epoch == 0`」でその関数を呼ぶだけで済む | 起動時プローブ失敗の後に `current_focus=None`/`app_policy` 既定値の症状が記録されたとき |
| cold マークの 2 回(`Output::on_focus_changed` と `mark_composition_cold_focus_change`、W0-b §11.1-2)を 1 回にする | 目録 #7 と違い同じ中身ではない(後者だけが `WarmEpoch.mark_cold` を行い、`gji_on_focus_change` などほかの処理と並んでいる)。warmup は再発ファミリーで、実害の記録も無い | 2 回のどちらかだけを直して食い違った BUG が記録されたとき |
| `subscriptions(&Model)`(Elm: モデルから「いま観測すべきソース」を導き、差分で起動・停止) | 観測のソースは起動時に固定で据え付けられ、モデルに応じて起動・停止していない(背景参照)。動的なのはタイマーだけで、既に自己再スケジュールと in-flight の印を持つ。ソースの起動・停止の漏れの BUG は記録が無い(Effect Plan round1 の E6 の待つ条件のまま)。UIA のワーカーは BUG-012 以来結果を使っておらず、これは撤去の問題(ADR-229「撤去の候補」)で購読の問題ではない | 観測のソースの起動・停止の漏れが BUG として記録されたとき |
| ミドルウェア列(Rack/ASGI: 取り込みの前後に処理を差し込む列) | 入口は `dispatch_event` の 1 か所で、手順は 4 つの固定順(e9)。列にすると順序が設定に移り、e9 の辺がかえって見えなくなる。キー入口のミドルウェア列は ADR-229 D7 が既に却下、汎用の `Handler` は toolkit round1 が却下 | 取り込みの入口が 3 つ以上に増え、同じ前処理を各入口に書き写した結果の BUG が記録されたとき |
| 全書き込みを Event にして reduce に通し、journal から状態を完全に再生する(Redux の単一 store) | W0 で、Event を出さない hub のメソッドが 27、`Runtime` の 40 フィールドは 0。すべてを Event にするのは大きな追加で、対応する実害の記録が無い。journal からの再生・fixture 化は ADR-225 で見送り済み(`replay_record` は HEAD での再現を判定できない、など) | ADR-225 の見送りの理由を覆す証拠が出たとき |
| `InputModeObserved` に観測のスコープ(`hwnd`・`focus_epoch`)を持たせて reducer で照合する(BUG-057 の残りの非対称) | BUG-057 は `is_eisu_evidence`(`ime_on` も見る)で解決済み。非同期の経路は適用の前に `admit_epoch_in_app` を通る。同期の `ir_stage_observe` は観測時点のスコープが正しい。非対称の実害は「未確認(要追跡)」のまま。フィールドを足すのに見合う記録が無い | 別スコープの `InputModeObserved` が `input_mode` を汚した報告が、BUG-057 の修正後に出たとき |

再提案しないもの(却下済み): Plan/Effect の項(`Try`・`Require`・`Bracket`・`Spawn`)と Stage の全順序(`229-effect-plan-design-draft-r1-rejected.md`・`229-opus-effect-plan-round1.md`)、書き込みの吸収・正規化の法則(BUG-141・ADR-208 と逆向き)、汎用の Handler/Facts/`Tagged<T>`/Clock の部品(`229-opus-fcis-toolkit-round1.md`)。本 ADR の決定は、これらのどれにも当たらない(Event を減らす・既存の reducer の腕を共有する・読み手のない記録と死んだ呼び出しを消す)。

## 段階(実装タスク)

各段階は挙動不変を原則とし、CI だけで成否を判定する(ローカルでビルドしない)。S1・S2・S3 は別の PR にする(S2 は「focus 遷移」「IME belief」の再発ファミリーに触れ、S3 は gated の `runtime/` に触れるため)。round1 で読みによる確認(旧 S0 の③④⑤)は済んだので、各 PR の着手時に、実装する HEAD で次を取り直すだけにする: ①リングの本番の読み手が 0、②`at` と envelope の tick が本番の全構築箇所で同じ値、③`focus_tracking.rs:858-867` の区間に `force_guards.add`・`record_miss` の呼び出しが入っていない。

| 段階 | 内容 | 撤去対象(目録の #) | 検証 | 取りやめ条件 |
|---|---|---|---|---|
| S1 | D2・D3 | 4・5・6 | Linux の `cargo nextest run --workspace --lib`(`ime_event_log`・`platform_state`〈P4 で ungated〉・`ime_model` の単体テスト、書き直した `manual_hub_clock_drives_event_monotonic`)、`golden_scenarios`、`journal.rs` の `event_seq`/`tick_ms` のテスト、`layer_boundary_guard`(`ime_event_log` を `CORE_MODULES` へ移す変更と C-6 のメッセージ)、`windows-cross-check`・`windows-build` | ①②のどちらかが成り立たない。その部分だけ取りやめ、残りは進める |
| S2 | D1 | 1・2・3 | D1 のテスト 1〜4(Linux)、`windows-build`。CI の実機相当: BUG-148 は、当時の再現手順(`ci/e2e-ime` の構成。run 35484080057 が再現、35484314507 が対照)と同じく、awase を起動してフォーカスを一度も別プロセスへ移さずに無変換を押し、手順 5 が PASS し `[apply-ime] outcome=Unwarranted`(`attempts_len=0`)が出ないことを確かめて PR に書く。BUG-114 は CI で同じ症状を数えた前例が無いので、単体テスト(等価性・belief 不変)と統合後のログ行の目視のみとし、「実機未確認」と書く | belief 不変の主張を全体比較の 1 本で表せない、または等価性テストの「起動時は違ってよい」一覧が `input_barrier`(と `key_track`)を超えて増えるとき。その場合は Event を残し、等価性のテストだけを足す |
| S3 | 目録 #7 | 7 | `cargo check --target x86_64-pc-windows-msvc -p awase-windows`(`runtime/` は gated なので Linux のテストは無い)、`windows-build`。固定するもの: 同じ消去は `FocusChanged` の reducer の `force_guards.clear_for_focus_change`・`record_success` が担い、それは既存の reducer のテストが固定している旨を PR に書く | ③で、区間に条件を真にしうる呼び出しが入っていたとき(その場合は撤去しない) |

全段階の共通の後処理は ADR-229「移行のレシピ」に従う(`fix-requires-evidence.md` の表・`.githooks/pre-push`・`examine_globs` の見直し)。

## 既存 ADR との関係

| ADR | 関係 |
|---|---|
| ADR-229 | 「世界モデルの reduce 化と Effect 列の設計の検討結果」節が未起草として残した「観測と Event の購読・消費」の答え。D2 は F-D6 の違反ファイルを 1 つ減らす。Effect Plan round1 の E6(観測の購読)の待つ条件は変えない |
| ADR-032 | `ImeEventLog` のリングの出自(Step 0)。D2 で撤去し、ADR-032 に追記する |
| ADR-102 決定3-b | 起動時は belief を書かない。D1 はこれを保ち、belief 不変の全体比較テストで固定する |
| ADR-104・ADR-106 | fence/epoch による観測の admission。変えない |
| ADR-134 D1c・ADR-186 | BUG-114・BUG-148 の修正。D1 は ADR-134 D1c の元の設計(1 つの Event)に戻すが、起動時に `app_policy`・`current_focus` を入れる効果は変えない |
| ADR-170 | reduce のヘルパーは `reduce_` で始め、`reduce()` の本体からだけ呼ぶ。D1 の共有関数もこれに従う |
| ADR-224 | `ImeStateHub` の ungate。P4(#510)で済んでおり、D1〜D3 の単体テストは Linux で回る |
| ADR-225 | journal からの再生・fixture 化の見送り。本 ADR は journal からの完全再生を目標にしない |
| ADR-218〜220・ADR-180 決定2 | DSL・宣言テーブル・統合しても数が減らない統一の見送り。本 ADR は新しい部品を作らず、Event と関数を減らす |

## Opus レビューの記録

- round1(2026-10-05): Blocker なし。Must 3(等価性テストの比較一覧がトートロジー → 「起動時は違ってよい」一覧+全体の `Debug` 比較、belief 不変テストを全体比較に、fence を共有関数の外に置き名前を `reduce_` で始める)・Should 8・Nit 6 を反映。旧 S0 の③(起動時の hwnd が同じ値)と④(`reset_detect_state` の条件が常に偽)は Opus が読みで確認したので、本文の事実に移した。
