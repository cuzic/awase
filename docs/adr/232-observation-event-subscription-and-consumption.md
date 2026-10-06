---
id: ADR-232
title: |-
  Observation/Event の購読と消費: Redux/Elm/Rack/ASGI の着想のうち、実害の記録に結びつく「起動時のフォーカススコープの一本化」と、挙動不変の撤去 2 件だけを採る
summary: |-
  ADR-229「世界モデルの reduce 化と Effect 列の設計の検討結果」で未起草として残した「観測と Event の購読・消費」(Redux/Elm/Rack/ASGI から借りる案: 単一 Envelope = source/seq/time/scope/confidence、FocusScope、`subscriptions(&Model)`、ミドルウェア列)を、実害の記録(docs/known-bugs・既存 ADR)と W0 の実測(docs/tasks/world-model-write-inventory-2026-10-06/)に当てて選別した。
  採る(3 件、いずれも新しい型・trait・機構を足さない): (D1) 起動時のフォーカス確立が、定常の `FocusChanged` と別の 3 つの `Initial*` Event(`InitialFocusFenceEstablished`・`InitialAppPolicyEstablished`・`InitialFocusHwndEstablished`)に分かれ、新しいフォーカス由来のフィールドを足すたびに起動時の側を忘れて BUG になった(BUG-081・BUG-102・BUG-114 根本原因1・BUG-148 の 4 件)。これを 1 つの Event にまとめ、reducer の「スコープの同一性を入れる」処理を `FocusChanged` と共有し、等価性のテスト 1 本で固定する。(D2) 本番に読み手のいない `ImeEventLog` の 512 件のリングを撤去し、`seq` の採番だけを残す(W-c で journal が `event_seq`/`tick_ms` を持ったため情報は失われない)。(D3) `InputModeObserved`/`InputModeApplied` の `at: TickMs` は、本番の全 7 か所で Envelope の `tick_ms` と同じ値なので撤去する。
  採らない: 全 Event に共通の Envelope、時刻の入口の一本化、admission の Envelope への集約、フォーカスの入口 4〜5 つと 9 種の型の 1 構造体化、フォーカス hwnd の 4 つの写しの統合、`subscriptions(&Model)`、ミドルウェア列、全書き込みの reduce 化と journal からの完全再生。いずれも実害の記録が無いか、既存の判断(ADR-225・toolkit round1・Effect Plan round1・D7)と衝突する。各々に「何が起きたら考え直すか」を書く。
status: |-
  起草(2026-10-05)。Opus レビュー前。コードは変えていない。
related_adr:
  - "ADR-229"
  - "ADR-102"
  - "ADR-104"
  - "ADR-106"
  - "ADR-134"
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

主目的は新しい仕組みの導入ではなく、**起動時のフォーカス確立のために BUG ごとに 1 つずつ足された Event・関数・ガードの撤去**(D1)と、**読み手のいない記録・二重に持つ時刻の撤去**(D2・D3)である。成功の基準は、下の「撤去の目録」の項目が実際に消えた数で測る。新しい型・trait・汎用の部品は 1 つも足さない。

実測の基準: `origin/develop` `b3fbe315`(#516 のマージ後)。読み取りのみ、ビルドとテストはしていない。件数は `git grep` による。

## 背景(事実)

### W0 の実測(`docs/tasks/world-model-write-inventory-2026-10-06/`)

- `ImeEvent` は 20 variant(うち `UserImeToggleIntent`・`UserChangedInputMode` は本番の発行元が無く、#515 で削除中)。`reduce` を通るのは `ImeStateHub` のうち `shadow_model` だけ、`&mut self` のメソッド 53 個のうち 27 個は Event を出さない。
- Event の入口は `ImeStateHub::dispatch_event`(`state/platform_state.rs:167`)の 1 か所で、手順は「`last_user_explicit_off_ms` の更新 → `event_log.record_at`(`EventTime{seq, monotonic, tick_ms}` の採番)→ `reduce` → `journal.record`」の固定順(依存辺 e9、`docs/tasks/effect-signature-inventory-2026-10-06/dependency-edges.md`)。
- 観測の Envelope に当たるものは**既にある**: `AnyObservation { open, source, hwnd, confidence, focus_epoch }`(`state/evidence.rs:270`)。非同期の観測は、各呼び出し元が `admit_epoch_in_app` で古いフォーカスの結果を捨ててから dispatch する(ADR-106。`key_pipeline.rs:332/574/2853`、`focus_tracking.rs:837`)。
- 時刻の入口は 3 つ(`EventTime`・`HubClock` の直読み・呼び出し側の `Instant::now()`/`current_tick_ms()`)。journal の `ImeEvent` には W-c(#505・#508・#516)で `event_seq`・`tick_ms` が載った。`Instant` は載らない。
- `ImeEventLog`(512 件のリング)は**本番に読み手がいない**。`recent`/`recent_vec`/`iter` の呼び出しはテストだけ(`platform_state.rs:1868` ほか)。本番が使うのは `next_seq()`(`:711`・`:1156`)と `record_at` の戻り値の `EventTime` だけ。モジュールの doc は「Step 0 では本番判定には使わず、将来の Shadow Reducer の入力源として保持」とあるが、Shadow Reducer(`shadow_model`)は `dispatch_event` が直接 `reduce` していて、リングを読まない。
- 観測のソースは起動時に固定で据え付けられ、モデルに応じて起動・停止していない: WinEvent のフック 3 本(`tsf/win_event_obs.rs::install_observation_hooks`)とフォーカスのフック(`app/bootstrap.rs:826`)、`gji-io-monitor`(`bootstrap.rs:1318` で無条件に起動)。動的なのはタイマー駆動の読み取り(`TIMER_IME_REFRESH`・idle-conv-check・`start_ms_ime_ready_poll`・warmup)だけで、どれも自己再スケジュールと in-flight の印を既に持つ。

### 実害の記録: 起動時のフォーカス確立が定常の経路と別だったことによる BUG が 4 件

定常のフォーカス変更は `on_focus_process_changed` → `ImeEvent::FocusChanged { from, to, profile, focus_epoch }` で、reducer(`ime_model.rs::reduce_focus_changed`)が `app_policy`・`current_focus`・`observations.current_fence`(=`FocusFence{epoch: focus_epoch, hwnd: to}`)を入れ、続けて belief 側のリセット(`last_intent`・`applied=Unknown`・`force_guards`・`input_barrier` ほか)を行う。起動時は `establish_initial_focus_scope`(`focus_tracking.rs:109`)が別に走り、ADR-102 決定3-b(最初の IME 観測より前に belief を書かない)のために `FocusChanged` を出さない。その結果、**`FocusChanged` が入れる「スコープの同一性」のフィールドが、足されるたびに起動時の側で漏れた**。

| BUG | 漏れたもの | 足した撤去対象 |
|---|---|---|
| BUG-081 | 起動直後の初回フォーカスが `process_changed` 判定を通らない | `establish_initial_focus_scope` |
| BUG-102 | `observations.current_fence`(起動時のアプリの `ImmCrossProbe`〈High〉が導出から外れ、Medium に負ける) | `ImeEvent::InitialFocusFenceEstablished`、`sync_initial_focus_fence`、ガード `initial_focus_fence_event_only_touches_the_fence`、単体テスト 1 |
| BUG-114 根本原因1(ADR-134 D1c) | `app_policy`(既定の `Read` のまま、TsfNative で `VK_IME_OFF` を再送し続けた。実機確認済み) | `ImeEvent::InitialAppPolicyEstablished`、`sync_initial_app_policy`、ガード `initial_app_policy_event_only_touches_app_policy`、単体テスト 1 |
| BUG-148(ADR-186) | `current_focus`(`None` のまま明示意図が記録されず、委譲 `SetOpen` が全て `Unwarranted`。CI で再現) | `ImeEvent::InitialFocusHwndEstablished`、`sync_initial_focus_hwnd`、ガード `initial_focus_hwnd_event_only_touches_current_focus`、単体テスト 1 |

3 つの `Initial*` Event は、いずれも `FocusChanged` の reducer が入れる 3 つの同一性フィールドのうち 1 つだけを入れる。ガードはそれぞれ「その Event が 1 フィールドしか触らない」ことを固定するが、**「`FocusChanged` が入れる同一性のフィールドを、起動時の側も全部入れる」ことは何も固定していない**。4 つ目のフィールドが足されたら、同じ型の BUG が 5 件目として出る構造のまま。

## 決定

### D1: 起動時のフォーカス確立を 1 つの Event にし、同一性の代入を `FocusChanged` と共有する(実害: BUG-081・102・114・148)

- 3 つの `Initial*` を、`FocusChanged` から `from` を除いたのと同じ形の 1 つの Event にまとめる: `InitialFocusScopeEstablished { to: HwndId, profile: ImePolicyProfile, focus_epoch: FocusEpoch }`(フィールド名・型は `FocusChanged` と同じ。新しい型は作らない)。
- reducer: `reduce_focus_changed` の先頭の 3 行(`app_policy`・`current_focus`・fence の確立)を、private な関数 1 つ(例: `enter_scope_identity(to, profile, focus_epoch)`)に出し、`FocusChanged` の腕と `InitialFocusScopeEstablished` の腕の両方から呼ぶ。`FocusChanged` の腕だけが、続けて belief 側のリセットを行う。ADR-102 決定3-b(起動時は belief を書かない)は、起動時の腕が同一性の関数しか呼ばないことで保つ。
  - fence の確立は、定常では `observations.clear_on_focus_change(fence)`、起動時では `observations.establish_initial_fence(fence)`(観測プールを持ったまま差し替えない、の `debug_assert!` つき)で、意味が違う。共有する関数には `app_policy` と `current_focus` だけを入れ、fence は各腕に残すか、`ObservationStore` 側の 2 つの口をそのまま使う。どちらにするかは実装 PR で決め、本 ADR の改訂は要らない。
- 呼び出し元: `sync_initial_focus_fence`・`sync_initial_app_policy`・`sync_initial_focus_hwnd` の 3 関数を 1 つにする。`dispatch_event(` を本体に直接書かない、という `establish_initial_focus_scope_does_not_write_ime_belief` との約束(`focus_tracking.rs:229-233` の doc)は保つ。
- 固定するテスト(3 本のガードと 3 本の単体テストを置き換える):
  1. **等価性**(`state/ime_model.rs` の単体テスト、Linux で回る): 同じ `(to, profile, focus_epoch)` で、新しいモデルに `FocusChanged` を通したものと `InitialFocusScopeEstablished` を通したものの、`app_policy`・`current_focus`・`current_fence` が等しい。**`FocusChanged` に同一性のフィールドが足されて起動時の側が漏れたら、このテストが落ちる**ようにするため、比べるフィールドの一覧は `enter_scope_identity` が書くものと同じ 1 か所から取る(テストに一覧を手書きしない)。
  2. **belief 不変**: `InitialFocusScopeEstablished` が `desired_open`・`input_mode`・`applied`・`last_intent`・`force_guards`・`input_barrier`・`key_effect`・`key_track` を変えない(ADR-102 決定3-b。既存の 3 本の単体テストの主張を 1 本に集める)。
  3. **発行元が 1 か所**: `architecture_guard.rs` の 3 本(`:3856`・`:3923`・`:4093`)を、`InitialFocusScopeEstablished` の dispatch が `establish_initial_focus_scope` の配下の 1 か所だけであることを固定する 1 本にする。
- 撤去の量(概数、実装 PR で実測して本文に書く): `ImeEvent` の variant 2、`focus_tracking.rs` の関数 2(doc を含め約 45 行)、`ime_model.rs` の腕 2、ガード 3 → 1(約 150 行 → 約 60 行)、単体テスト 3 → 2。

### D2: `ImeEventLog` のリングを撤去し、`seq` の採番だけを残す(挙動不変の撤去)

- `ImeEventLog` から `VecDeque` のリング・`capacity`・`DEFAULT_CAPACITY`・`len`/`is_empty`/`recent`/`recent_vec`/`iter`・`Instant::now()` を読む `record()` を削る。残すのは `next_seq` と、`EventTime` を返す採番(名前は `record_at` のままでも、`stamp` などに変えてもよい)。
- `dispatch_event` の `event.clone()` が 1 回減る(リングへの move が無くなるので、journal 用と reduce 用の 2 回のうち 1 回は元の値を使える)。
- 情報は失われない: 再生や不具合報告に使う記録は journal(W-c で `event_seq`・`tick_ms` つき)で、リングは一度もダンプされていない(W0-a §8-1)。
- 副次: `ime_event_log.rs` の Tier-2 違反は `Instant::now()` だけ(ADR-229 F-D6 の違反 8 ファイルの 1 つ)なので、撤去後に `CORE_MODULES` に足せる(違反のある ungated ファイル 8 → 7)。
- テスト `platform_state.rs:1868`(`HubClock` の値が `EventTime` に届くこと)は、`record_at` の戻り値か journal の `tick_ms` で同じことを確かめる形に直す。

### D3: `InputModeObserved`/`InputModeApplied` の `at: TickMs` を撤去する(挙動不変の撤去)

- 本番の構築箇所 7 か所(`InputModeObserved`: `ime_refresh.rs:191`・`key_pipeline.rs:822`・`key_pipeline.rs:2883`・`platform_state.rs:1221`、`InputModeApplied`: `platform_state.rs:1141`・`platform_state.rs:1283`・`runtime/mod.rs:1123`)は、どれも `at` に `dispatch_event` の第 2 引数と同じ値を渡している(`git grep` で確認、テスト `platform_state.rs:3337` は `TickMs(0)`)。
- reducer の読み手は `reconcile_key_effect_mode(mode, at.0)`(`ime_model.rs:842`)などで、`envelope.time.tick_ms` に置き換える。時刻は Envelope(`EventTime`)の 1 か所にだけ持つ。
- journal の JSON から `at` が消えるが、同じ値は W-c の `tick_ms` にある。`tests/journals/` に `InputModeObserved`/`InputModeApplied` を含む fixture は無い(`git grep`)。
- `UserChangedInputMode` の `at` は #515 の variant 削除で消える(本 ADR では扱わない)。

## 撤去の目録(成功の基準)

| # | 撤去するもの | 段階 |
|---|---|---|
| 1 | `ImeEvent::InitialAppPolicyEstablished`・`InitialFocusHwndEstablished`(`InitialFocusFenceEstablished` は改名して残す) | S2 |
| 2 | `sync_initial_app_policy`・`sync_initial_focus_hwnd`(1 つに統合) | S2 |
| 3 | 起動時専用のガード 3 本 → 1 本、単体テスト 3 本 → 2 本 | S2 |
| 4 | `ImeEventLog` のリング・`record()`・読み出しの 5 メソッド | S1 |
| 5 | `dispatch_event` の clone 1 回 | S1 |
| 6 | `InputModeObserved`/`InputModeApplied` の `at` | S1 |
| 7 | `on_focus_process_changed` の末尾近くの `reset_detect_state()` の条件付き呼び出し(`focus_tracking.rs:858-867`)。`FocusChanged` の reducer が同じ 2 つ(`force_guards` の消去・`observe_miss_monitor.record_success`)を直前に行うので、条件が真になる経路が無ければ死んだコード | S3(S0 の確認しだい) |

新しく足すもの: private な関数 1 つ(D1 の `enter_scope_identity`)と等価性のテスト 1 本だけ。**新しい型・trait・Event の種類は 0**。旧来の Event・関数・ガードを残したまま新しいものを並べたら失敗とする。

## 採らない案と理由

| 案(借りる元) | 採らない理由 | 考え直す条件 |
|---|---|---|
| 全 Event に共通の Envelope(source/seq/time/scope/confidence) | `ImeEvent` は belief の reducer への入力で、意図(`UserImeSetIntent`)や書き込みの結果(`ImeApply*`)には source・confidence の意味が無い。観測の Envelope は `AnyObservation` として既にあり、時刻と seq の Envelope は `EventTime` として既にある。全 Event に同じ包みを付けるのは toolkit round1 が却下した `Tagged<T>` と同じ(ドメインの区別を消す) | 観測でない Event に source/confidence が要る判断が実際に書かれたとき |
| 時刻の入口の一本化(`dispatch_event` の `tick_ms` 引数をやめ、hub が `HubClock` から読む) | 実害の記録が無い。`on_ime_apply_complete` は完了メッセージの時刻 `ts` を渡していて、hub の「いま」とは意味が違う(挙動変更になる)。`Instant` を journal に載せる件も、journal からの完全再生を目標にしない(下記)ので要らない | 時刻の取り違えが原因の BUG が記録されたとき |
| admission(古いフォーカスの結果を捨てる)を Envelope の段で 1 回に集める(Rack/ASGI のミドルウェア的な前処理) | 判定は既に 1 つの関数(`admit_epoch_in_app`)で、呼び出し元ごとに書く理由がある(dispatch だけでなく、`apply_focus_probe` などの適用処理全体の前で捨てる。`key_pipeline.rs:332` ほか)。`ObservationStore::is_identity_ok` は読み出し時の鮮度の判定で、役割が違う。BUG-091 は計測だけで実害は未確認、BUG-102 は fence の写しのずれで admission の問題ではない(D1 で扱う) | 呼び出し元が admission を忘れた BUG が記録されたとき |
| FocusScope を、フォーカスの入口 4〜5 つ(WinEvent・デバウンス後のポーリング・同一プロセス内の hwnd 変化・モーダルポンプ・電源復帰)と 9 種の型にまたがるリセットを 1 つの構造体に集める形にする | W0-b §6.4 のとおり状態の寿命が 3 種類(フォーカスで明示的に消す/`ForegroundScope` で自動失効/世代で失効)あり、1 つにまとめると意味が変わる。WinEvent の入口が `ime_mode_focus_gen` を進めない窓(約 50ms)の実害は未確認(W0-b §11.1-1)。focus 遷移は再発ファミリーで、実害の無い大きな組み替えは逆効果 | (a)〜(b) の窓で古い世代の `spawn_local` が誤って効いた BUG が記録されたとき |
| フォーカス hwnd の 4 つの写し(`FocusTracker.current.hwnd`・`current_fence.hwnd`・`ImeModel.current_focus`・`LAST_FOCUS_HWND`)を 1 つにする | 実害の記録は起動時のずれ(BUG-102・148)だけで、それは D1 で扱う。`FocusHwndUpdated` が fence だけを動かし `current_focus` を動かさない理由は未確認で、意図的かもしれない | 定常の経路で写しのずれが原因の BUG が記録されたとき |
| `subscriptions(&Model)`(Elm: モデルから「いま観測すべきソース」を導き、差分で起動・停止) | 観測のソースは起動時に固定で据え付けられ、モデルに応じて起動・停止していない(背景参照)。動的なのはタイマーだけで、既に自己再スケジュールと in-flight の印を持つ。ソースの起動・停止の漏れの BUG は記録が無い(Effect Plan round1 の E6 の待つ条件のまま)。UIA のワーカーは BUG-012 以来結果を使っておらず、これは撤去の問題(ADR-229「撤去の候補」)で購読の問題ではない | 観測のソースの起動・停止の漏れが BUG として記録されたとき |
| ミドルウェア列(Rack/ASGI: 取り込みの前後に処理を差し込む列) | 入口は `dispatch_event` の 1 か所で、手順は 4 つの固定順(e9)。列にすると順序が設定に移り、e9 の辺がかえって見えなくなる。キー入口のミドルウェア列は ADR-229 D7 が既に却下、汎用の `Handler` は toolkit round1 が却下 | 取り込みの入口が 3 つ以上に増え、同じ前処理を各入口に書き写した結果の BUG が記録されたとき |
| 全書き込みを Event にして reduce に通し、journal から状態を完全に再生する(Redux の単一 store) | W0 で、Event を出さない hub のメソッドが 27、`Runtime` の 40 フィールドは 0。すべてを Event にするのは大きな追加で、対応する実害の記録が無い。journal からの再生・fixture 化は ADR-225 で見送り済み(`replay_record` は HEAD での再現を判定できない、など) | ADR-225 の見送りの理由を覆す証拠が出たとき |
| `InputModeObserved` に観測のスコープ(`hwnd`・`focus_epoch`)を持たせて reducer で照合する(BUG-057 の残りの非対称) | BUG-057 は `is_eisu_evidence`(`ime_on` も見る)で解決済み。非同期の経路は dispatch の前に `admit_epoch_in_app` を通る。同期の `ir_stage_observe` は観測時点のスコープが正しい。非対称の実害は「未確認(要追跡)」のまま。フィールドを足すのに見合う記録が無い | 別スコープの `InputModeObserved` が `input_mode` を汚した報告が、BUG-057 の修正後に出たとき |

再提案しないもの(却下済み): Plan/Effect の項(`Try`・`Require`・`Bracket`・`Spawn`)と Stage の全順序(`229-effect-plan-design-draft-r1-rejected.md`・`229-opus-effect-plan-round1.md`)、書き込みの吸収・正規化の法則(BUG-141・ADR-208 と逆向き)、汎用の Handler/Facts/`Tagged<T>`/Clock の部品(`229-opus-fcis-toolkit-round1.md`)。本 ADR の D1〜D3 は、これらのどれにも当たらない(Event を減らす・既存の reducer の腕を共有する・読み手のない記録を消す)。

## 段階(実装タスク)

各段階は挙動不変を原則とし、CI だけで成否を判定する(ローカルでビルドしない)。S1 と S2 は別の PR にする(S2 だけが「focus 遷移」の再発ファミリーに触れるため)。

| 段階 | 内容 | 撤去対象(目録の #) | 検証 | 取りやめ条件 |
|---|---|---|---|---|
| S0 | 読み取りだけの確認(実装する HEAD で取り直す): ①`ImeEventLog` のリングの本番の読み手が 0、②`at` と `tick_ms` が本番の全構築箇所で同じ値、③起動時に `classified.hwnd` と `focus_fence().hwnd`(=`FocusTracker.current.hwnd`)が同じ値になること(`advance_focus_tracking` が `classified` から `current` を入れる経路を読む)、④`focus_tracking.rs:858-867` の条件(`is_force_on_guard_active() \|\| detect_miss_count() > 0`)が、直前の `FocusChanged` の reduce の後に真になる経路(`force_guards.add`・`record_miss` の呼び出し)があるか、⑤#515(死んだ variant の削除)のマージ | — | 結果を本 ADR に追記 | ③で値が違う経路が見つかったら、D1 は「同じ値を入れる」形にできないので、3 つの Event を残し、等価性のテストだけを足す形に縮める |
| S1 | D2・D3 | 4・5・6 | Linux の `cargo nextest run --workspace --lib`(`ime_event_log`・`platform_state`〈P4 で ungated〉・`ime_model` の単体テスト)、`journal.rs` の `event_seq`/`tick_ms` のテスト、`layer_boundary_guard` に `ime_event_log` を `CORE_MODULES` へ移す変更が通ること、`windows-cross-check`・`windows-build` | ①②のどちらかが成り立たない(本番の読み手がある、`at` が違う値の箇所がある)。その部分だけ取りやめ、残りは進める |
| S2 | D1 | 1・2・3 | 等価性と belief 不変の単体テスト(Linux)、`architecture_guard` の書き換え、`windows-build`。実機相当は、BUG-148 を再現した CI の手順(awase を起動し、フォーカスを一度も別プロセスへ移さずに無変換を押す)と BUG-114 の確認(起動直後の TsfNative で `VK_IME_OFF` の再送が出ない)を、既存の e2e の構成で 1 回流して結果を PR に書く | S0-③で値が違う、または ADR-102 決定3-b を 1 本のテストで表せない(belief 不変の主張が 3 本の和より弱くなる)とき。その場合は Event を残し、等価性のテストだけを足す |
| S3 | 目録 #7 | 7 | `cargo check --target x86_64-pc-windows-msvc -p awase-windows`(`runtime/` は gated なので Linux のテストは無い)、`windows-build` | S0-④で、条件が真になる経路が 1 つでも見つかったとき(その場合は撤去しない) |

全段階の共通の後処理は ADR-229「移行のレシピ」に従う(`fix-requires-evidence.md` の表・`.githooks/pre-push`・`examine_globs` の見直し)。S2 は「focus 遷移」「IME belief」の再発ファミリーに触れるので、回帰テスト(上記 1〜3)を同じ PR に含める。

## 既存 ADR との関係

| ADR | 関係 |
|---|---|
| ADR-229 | 「世界モデルの reduce 化と Effect 列の設計の検討結果」節が未起草として残した「観測と Event の購読・消費」の答え。D2 は F-D6 の違反ファイルを 1 つ減らす。Effect Plan round1 の E6(観測の購読)の待つ条件は変えない |
| ADR-102 決定3-b | 起動時は belief を書かない。D1 はこれを保ち、belief 不変のテストで固定する |
| ADR-104・ADR-106 | fence/epoch による観測の admission。変えない |
| ADR-134 D1c・ADR-186 | BUG-114・BUG-148 の修正。D1 はその Event を 1 つにまとめるが、起動時に `app_policy`・`current_focus` を入れる効果は変えない |
| ADR-224 | `ImeStateHub` の ungate。P4(#510)で済んでおり、D1〜D3 の単体テストは Linux で回る |
| ADR-225 | journal からの再生・fixture 化の見送り。本 ADR は journal からの完全再生を目標にしない |
| ADR-218〜220・ADR-180 決定2 | DSL・宣言テーブル・統合しても数が減らない統一の見送り。本 ADR は新しい部品を作らず、Event と関数を減らす |

## Opus に判断を仰ぎたい論点

1. D2・D3 は実害の記録の無い「挙動不変の撤去」。採用の基準(実害に結びつくものだけ)を新機構の追加にだけ当て、純減の撤去には当てない、という線引きは妥当か。
2. D1: 3 つの `Initial*` が 1 フィールドずつに分かれていたのは、ガードを単純にするための意図的な分割(各 doc)。1 つにまとめても ADR-102 決定3-b の保証が弱くならないか。等価性のテストは「`FocusChanged` に同一性のフィールドを足したら起動時の側も要る」を本当に捕まえるか(比べる一覧を 1 か所から取る方法を含めて)。
3. fence の確立(定常は `clear_on_focus_change`、起動時は `establish_initial_fence`)の意味の違いを、共有する関数の内と外のどちらに置くべきか。
4. S0-③(起動時の `classified.hwnd` と `FocusTracker.current.hwnd`)と S0-④(`reset_detect_state` の条件)の読みで、見落としている経路はないか。
5. 採らない案のうち、実害の記録を見落としているもの(とくに、フォーカスの入口 (a)〜(b) の窓、`FocusHwndUpdated` と `current_focus` の非対称)はないか。
