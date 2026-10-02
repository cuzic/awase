---
id: ADR-214
title: |-
  IME への書き込みの「送った」と「観測で確認した」を分け、送信を省略してよい根拠を経路ごとに宣言する
summary: |-
  棚卸し(2026-10-02、develop `f4e225e8` 時点)で、`AppliedImeState::Confirmed` が「観測で確認した」ではなく「API が成功を返した」(SendInput 発行成功、SendMessageTimeout の返り)でも書かれ、
  それが `gji_direct_already_matches` 等で「次回は送らなくてよい」の根拠に使われうることを確認した(BUG-141 と ADR-098 の取り違えと同型)。TsfNative と Imm32Unavailable は読み戻せないので、
  これらの窓の `Confirmed` は実質「送ったつもり」である。ADR-208 L1 は明示キーの押下について `applied` を未知化して塞いだが、押下以外の order(`press=None`)には適用されていない。
  本 ADR は、(B) `applied` の「送った(Sent)」と「観測で確認した(Confirmed)」を型で分け、省略の根拠は Confirmed だけに限る、
  (C) 経路(窓プロファイルと書き込み機構の組)ごとに、効果を観測で確認できるか(閉ループ/準閉ループ/開ループ)と、省略してよい根拠を1か所で宣言し、開ループの経路は状態の推測を根拠に省略しない、の2つを決める。
  新しい台帳や gate は足さない。ADR-212 により押下以外の書き込みが減っているため、着手の前提条件(決定0)として、省略の根拠に `applied` を使っている押下以外の経路が現在も残っているかを先に測る。
status: |-
  保留(2026-10-02、所有者判断)。決定0(経路の確認)まで実施し、P1(トレイ起点の Engine コマンドの `SetOpen` が、stale な `applied` で GjiDirect の送信を省かれる)は決定の層では特性テストで確認済み。
  実機での頻度と実害は未確認で、実機の確認はユーザー承認が要るため、決定1(`Sent`/`Confirmed` の分離)・決定2(経路ごとの宣言)は実装しない。再開条件は「保留の理由と再開条件」節。
related_adr:
  - "ADR-098"
  - "ADR-108"
  - "ADR-119"
  - "ADR-208"
  - "ADR-212"
  - "ADR-213"
---

# ADR-214: 「送った」と「確認した」を分け、省略の根拠を経路ごとに宣言する

## 背景

制御は、少し前の観測にもとづいて少し先の書き込みを決める。そのとき、自分が送ったがまだ観測に反映されていないコマンド(in-flight)を、
確認できた事実と混同すると、反映前の古い観測を見て重複送信や握りつぶしが起きる。

2026-10-02 の棚卸し(`docs/tasks/` には残していない。下記の確認箇所から再現できる)で、次を確認した。

- **確認済み(コードを読んだ)**:
  - `AppliedImeState` は `Unknown`/`Optimistic`/`Confirmed{open, at_ms}` の3状態(`state/ime_model.rs:132`)。
  - `ImeApplySucceeded`(`reduce_ime_apply_succeeded`、`state/ime_model.rs:1126`)が受理されると `applied` は `Confirmed` になる。これは書き込み API が成功を返した、
    という意味であって、実 IME を観測した結果ではない。失敗側の reducer(同 `:1183` 付近)も、`UnsafeToToggle` 以外は `Confirmed{open: !target}` を書く(ADR-108 決定3が
    「既存挙動維持」と明記している非対称)。
  - `record_confirmed` の呼び出し元は性質が混在している: 観測値を書くもの(`runtime/ime_refresh.rs:571`、`runtime/mod.rs:1387`)、
    書き込みの**前**に意図を書くもの(`runtime/key_pipeline.rs:1284` の shadow toggle OFF、ADR-098 が「pre-actuation の正当な write」と整理済み)、
    フォーカス変更時の belief ミラー(`runtime/focus_tracking.rs:205`)がある。同じ `Confirmed` でも意味が違う。
  - `gji_direct_already_matches(shadow_on, open, candidate_was_seen)`(`state/ime_actuation_decision.rs:219`)は、`shadow_on`(`applied` 由来)が
    `Some(open)` なら送信を省略する(OFF 方向のみ `candidate_was_seen` で保護、ADR-171)。
  - `ImeModel.pending`(送信済み・未完了)は、完了の突き合わせ以外で次の判断に使われていない。
- **経路ごとに確認手段が違う**(`state/app_ime_policy.rs` の `feedback`): ImmCross 系は読み戻せる(Read)、TsfNative と Imm32Unavailable は読み戻せない(Blind)。
  Blind の窓では、`Confirmed` は事実上「送ったつもり」でしかない。
- **既に塞がれている範囲**: ADR-208 L1 は、押下(`press.is_some()`)の order に限り `applied` を未知にして省略を無効にした(`explicit_press_shadow_on`)。
  `PressLedger` が同一押下の二重書き込みを防ぐ。ADR-212/213 は、押下を引き金にしない予防的・補正的な書き込みを撤去している。

したがって残る問題は、**押下以外の order(`press=None`)が、`Confirmed` を根拠に省略されうる**ことである。ADR-098 が警告した、
belief と actuation の記録の取り違えと同じ型であり、BUG-141(GJI の already-matched が再送を握りつぶす)の構造でもある。

## 決定

0. **前提条件(実装前に確認し、結果をこの節に書き戻す)**。次を測る。
   - `decide_attempt` が `press=None` で呼ばれ、かつ `shadow_on` が `Some` のまま渡る経路が、ADR-212/213 の撤去後も残っているか(grep と journal リプレイで列挙する)。
   - 残る経路がなければ、本 ADR の決定 1・2 は実装せず、決定 2 の「宣言」だけを ADR-208 の保証範囲の注記として残して終える(不要な機構を足さない)。
   - 残る経路がある場合、その経路が実際に BUG-141 型の握りつぶしを起こしうるか(ADR-208 の全列挙テストの状態空間で `press=None` の行を追加して数える)。
   - **結果(2026-10-02)**: 経路は残っている(P1 トレイ起点の Engine コマンド、P3 TsfNative の Engine 経路)。ただし P1 の実害は未実測、P3 は ADR-208 L3' と重なる。下の「決定 0 の結果」節を参照。
   - **P1 の確認(2026-10-02)**: 決定の層では確認済み(特性テスト2本)。実機の頻度と実害の大きさは未確認(下の P1 の節)。
### 決定 0 の結果(2026-10-02、origin/develop `f4e225e8` のコードを読んで確認)

`decide_attempt`(`ime_controller.rs:218,462`、`runtime/open_chain.rs:518`、`state/explicit_press.rs:802`)に至る actuation の入口は3つで、それぞれ `applied` 由来の `shadow_on` の扱いが違う。

| 入口 | `applied` を省略の根拠にするか | 根拠 |
| --- | --- | --- |
| `executor::dispatch_ime_set_open`(Engine の `SetOpen`) | **する場合がある**(下の P1・P2) | `runtime/executor.rs:723-735`: `press.is_some()` かつ `engine_press_unknowns_applied` のときだけ未知にする |
| `key_pipeline::kp_shadow_actuate`(shadow toggle) | `press=None`(自動リピート)のときだけ | `runtime/key_pipeline.rs:1277-1281`: `explicit_press_applied_pair(.., press.is_some())` |
| drift correction(`ime_refresh.rs:958`) | **しない**。`apply_ime_open_with_belief(order, None, ..)` で `applied` に `None` を直書き(`shadow_on` は未知) | 読んで確認 |
| `open_chain.rs` の `fallback_write` / `imm_cross_write` | しない。`fallback_write` は `shadow_on=None` に強制、`imm_cross_write` は直後の再観測(`imm_cross_reobservation_already_matches`) | ADR 本文の既存記述どおり(今回は再読していない) |

`SetOpen` が `press=None` になる発行元は、`src/engine/engine.rs` の `transition_activation`(`:427`)、`apply_engine_on_with_ime_recovery`(`:824`)、`ime_set_open_effects`(`:864`)の3つ。打鍵起点のものは入口で `stamp_set_open_press`(`engine.rs:1113`)が `press_id` を載せるので `Some` になる。`None` のまま残る起点は次のとおり。

- **P1: 打鍵ではない Engine コマンド**(`runtime/mod.rs:982` の `toggle_engine`〈トレイ〉、`:1001` の `force_engine_on`〈トレイの「状態をリセット」等〉)。
  belief が OFF で active になれないときだけ `apply_engine_on_with_ime_recovery` が `SetOpen{true, press: None}` を出す(belief が ON で既に active なら `SetOpen` は出ない)。
  `applied` がそのまま GjiDirect の `gji_direct_already_matches` に渡り、`applied=Confirmed(true)` なら省略される。
  **確認済み(2026-10-02、Linux の特性テスト2本)**:
  - `engine::tests::…::on_command_force_engine_on_emits_set_open_without_press_only_when_belief_is_off`: belief OFF なら `SetOpen{true}` が `press=None` で出る。belief ON で active なら出ない。
  - `state::ime_model::tests::adr214_p1_press_none_set_open_is_elided_by_stale_applied_in_gji_blind_window`: 実際の `ImeModel` で「`applied=Confirmed(true)`、belief は物理 IME キーの明示意図で OFF」を作る
    (belief は `applied` と無関係に決まるので到達できる)と、Imm32Unavailable × GJI(`CHAIN_GJI_ONLY`、Blind)で `decide_attempt` が送信を省く(`None`)。同じ状態でも押下付き(`has_press=true`)なら送る。
  - 影響を受けるのは GJI だけ(MS-IME の `MsImeDirect` は省略判定を持たない)。窓は Imm32Unavailable と TsfNative(いずれも `CHAIN_GJI_ONLY`)、および ImmCross 先頭の窓で ImmCross が失敗して GjiDirect に落ちた場合。
  - **未確認**: 実機で起きるか(belief OFF × `applied=Confirmed(true)` の食い違いが実運用で出るか)、省略された後に何かが回復するか(Blind の drift correction は `applied=None` で送るが、その発火条件は読んでいない)。
    つまり P1 は「決定の層では**確認済み**、実機の頻度と実害の大きさは**未確認**」。
- **P2: 自動リピート**(`press=None`)。意図した省略(同じキーの押下で既に書いた)なので問題ではない。
- **P3(`press=None` ではないが同じ構造): TsfNative の窓の Engine 経路**。`ENGINE_PRESS_UNKNOWNS_APPLIED_IN_TSF_NATIVE=false`(`ime_actuation_decision.rs`)のため、押下でも `applied` を未知にしない。
  ADR-208 の決定6が、BUG-124 型の「@」(WT × GJI × PSReadLine で OFF キーごとに単発の `VK_IME_OFF` が出る)の実機 A/B(L3')をマージ条件にしているため。S-1 は意図的に残っている。
- **撤去済みで該当なし**: `check_active_transition` 由来の `SetOpen`(ActivationSync)は `transition_activation(new_state, false)`(`engine.rs:396`)で止まっている(ADR-213 P2b)。

**含意**: 範囲は「ゼロ」ではないが、想定より狭い。(1) P1 は実在するが、トレイ操作という低頻度の経路で、実害は仮説の段階。(2) P3 は ADR-208 L3' と**同じ変更**(TsfNative の Engine 経路で `applied` を省略の根拠から外す)になるため、
本 ADR の決定 1 を単独で入れると、L3' の実機 A/B 抜きで BUG-124 型の「@」のリスクを持ち込む。決定 1 の適用範囲は、ADR-208 L3' の判断(`ENGINE_PRESS_UNKNOWNS_APPLIED_IN_TSF_NATIVE`)と**一本化する**(TsfNative の窓は L3' が解禁するまで従来どおり)。

**`applied_open()` の他の消費者**(決定 1 で `Sent` を未知として扱うと挙動が変わるので、実装前に洗う): `state/ime_model.rs:616`(完了通知の受理判定。`Superseded` の条件に `applied != Some(open)` を使う)、`:942`、`:970`、`runtime/message_handlers.rs:960`(後3つは未読)。

1. **B: `applied` の「送った」と「確認した」を型で分ける**。
   - `AppliedImeState` に `Sent{open, at_ms}` を足す(API が成功を返しただけ。実 IME の観測は未確認)。`Confirmed` は**観測で確認できた**場合だけに限る
     (読み戻して一致した、完了通知に対応する観測が届いた、等)。
   - 書き込みの完了が `Applied`/`AppliedWithoutSendInput`/`AlreadyMatched` で届いたとき、読み戻せない経路(Blind)は `Sent`、読み戻せて一致を確認した経路は `Confirmed` にする。
   - **送信の省略の根拠は `Confirmed` だけ**とする(`applied_open()` の証拠用アクセサの契約を、型で強制する)。`Sent` は未知(`None`)として扱う。
     ADR-216 R2 で時刻を捨てるだけだった `to_pair()` は撤去され、現在この記述は `applied_open()` が `Sent` を `None` に射影する話として読み替える。
     `Optimistic` との関係: `Optimistic` は ImmCross async の事前更新で、意味は `Sent` に近い。統合できるかは実装時に確認し、できなければ並置する。
   - 影響: Blind の窓では、`press=None` の書き込みが省略されなくなり、送信が増える方向に変わる。レイテンシと副作用(BUG-46 型の二重作用)は、決定 0 の測定とリプレイの差分で確認する。
2. **C: 経路ごとに、確認手段と省略の根拠を1か所で宣言する**。
   - 窓プロファイルと書き込み機構の組ごとに、次を `app_ime_policy.rs` の既存の `feedback` の近くに宣言する。
     - 確認手段: 閉ループ(読み戻して確認できる)/ 準閉ループ(観測はあるが遅い・嘘をつく)/ 開ループ(観測できない)。
     - 省略してよい根拠: 閉ループは `Confirmed` のみ、準閉ループは `Confirmed` かつ鮮度内、開ループは**省略しない**(毎回、冪等な絶対指定を送る)。
   - 同じ chain の中で性質が変わる経路(ImmCross が失敗して GjiDirect/MsImeDirect に落ちる)は、**実際に書く機構の側**の宣言に従う。`feedback` が chain 全体で Read のままになっている現状を改める。
   - トグル型の書き込み(半角英数、物理モードキー)は、省略の根拠が問題にならない別枠(既存のラッチと予測で守る)として宣言に含め、絶対指定と混ぜない。
   - 宣言と実態のずれは、`architecture_guard` に「省略の根拠に使える状態は `Confirmed` のみ」を検査するテストを足して検出する(新しい gate は足さない)。
3. **対象外**(本 ADR では決めない)。
   - in-flight の台帳(`pending` の格上げ)。決定 1・2 の後でも二重送信が観測された場合に別 ADR で扱う。ADR-212 以降の撤去方針に逆行するため、先には入れない。
   - 待ち時間の定数(リテラル化検出の期限、grace 窓)。別の棚卸しで扱う。
   - 明示キーの押下(ADR-208 で扱い済み)。

## 検証

- 決定 0 の列挙結果と、`press=None` の行を足した全列挙テスト(ADR-208 の L0 のモデルを拡張)で、決定 1 の前後の件数差を出す。
- journal リプレイ(ADR-163)で、決定 1 の前後に送信が増える経路と、その種類(`MechanismCommand`)を差分として確認する。
- 実機(TsfNative × GJI、Imm32Unavailable)で、押下以外の書き込みが増えたことによる退行(spurious な書き込み、BUG-46 型)がないかを確認する。`docs/experiments.md` に判定を残す。
- revert 条件: 実機で上記の退行が出たら、決定 1 を revert する。revert コミットには、アプリ・IME・症状を本文に残す。

## 未確定・リスク

- 決定 0 の結果、範囲は P1(トレイ起点の Engine コマンド)と P3(TsfNative、L3' と一本化)に縮んだ。P1 の実害の有無が、決定 1 を実装する価値を左右する。ADR-212 の撤去が進むほど、対象は減る。
- `Optimistic` と `Sent` の統合可否は未確認。
- 開ループの経路で省略をやめることは、冪等な絶対指定が二重に効かない前提に立つ。BUG-46 型の「awase の送信と物理キーの二重作用」では、この前提が崩れうる。
  物理キーの配送判断(`PhysicalKeyDisposition::plan`)との組み合わせを、決定 0 の列挙に含める。
- 本 ADR の事実は、コードを読んだ範囲に限る。hook watchdog の canary、chrome GJI reinit、eager warmup の経路、ADR-108 と ADR-080 の本文全体は読んでいない。

## 保留の理由と再開条件(2026-10-02、所有者判断)

- **保留の理由**: 決定0の結果、対象はトレイ起点の Engine コマンド(P1)と TsfNative の Engine 経路(P3、ADR-208 L3' と一本化)に縮んだ。P1 は決定の層では確認できたが、実機で起きる頻度と、省略後に自然回復するかが未確認。
  実機の確認には承認が要る。実害が小さい可能性が残るため、案 B・C の全体を実装する根拠はまだない。
- **この時点で残したもの**: 経路の棚卸し(決定0の結果の節)、P1 の特性テスト2本(`engine::tests::…::on_command_force_engine_on_emits_set_open_without_press_only_when_belief_is_off`、
  `state::ime_model::tests::adr214_p1_press_none_set_open_is_elided_by_stale_applied_in_gji_blind_window`)。テストは現状の挙動を固定するだけで、直すべき挙動の宣言ではない。
  P1 を直す場合は、このテストの期待値を更新する。
- **再開条件**(いずれか):
  1. 実機・不具合報告・journal で、belief OFF × `applied=Confirmed(true)` の食い違いのあとにトレイの「状態をリセット」等が効かなかった、という報告か記録が出た。
  2. ADR-208 L3'(TsfNative の Engine 経路で `applied` を省略の根拠から外す)の実機 A/B に進む。そのときは本 ADR の決定1と一本化できるかを再検討する。
  3. 押下以外の書き込みを根拠に、BUG-141 型の握りつぶしが新たに報告された。
- **再開時の最小の選択肢**: P1 だけなら、`press=None` かつユーザー操作起点(トレイ)の `SetOpen` に限って `applied` を未知にする小さな変更で足りる可能性がある(案 B 全体は不要)。実機の確認で食い違いが出ないと分かれば、本 ADR は却下してよい。
- **未読のまま**: hook watchdog の canary、chrome GJI reinit、eager warmup の経路、`open_chain.rs` の2関数と `set_ime_open_ordered` の ImmCross 経路の再読、ADR-108・ADR-080 の本文全体、`applied_open()` の消費者のうち `ime_model.rs:942`・`:970`、`message_handlers.rs:960`。

## 参考

- ADR-098(applied の偽装確定の撤去)、ADR-108(pending と generation)、ADR-119(actuation 合流点)、ADR-208(明示キーの固着ゼロ)、ADR-212/213(予防的・補正的書き込みの撤去)。
- BUG-141(GJI の already-matched が再送を握りつぶす)、ADR-171。
