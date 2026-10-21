# 機構 ECRS 調査 A: IME の actuation(担当接頭辞 MA)

対象コミット: origin/develop 先端 `f3c2007e`(`/home/cuzic/rust-nicola-wt-mech`)。読み取りのみ。cargo は実行していない。
参照: `docs/tasks/actuation-confluence-inventory.md`、`docs/adr/180-*.md`、`docs/adr/208-*.md`、`.claude/rules/fix-requires-evidence.md`。

## 結論(E/C 候補の数と上位 5 件)

- **E(撤去可)4 件、C(統合可)2 件、? 2 件、K(残す)多数。** 既存の「本物の統合候補ゼロ」(actuation-confluence-inventory.md)は**合流点の数**については正しい。ただし**到達不能な分岐**と**出所の無い授権の残骸**は棚卸しの対象外だったので、そこに E が残っている。
- 「同じ書き込みを 2 経路が行う」型の二重は、(a) 物理 IME キーの shadow 経路と Engine の `SetOpen`(押下 ID の台帳で意図的に解決済み、ADR-208、K)、(b) drift correction の ImmCross 書き込みが `run_open_chain_async` とは別の fire-and-forget(MA-06、?)、(c) `kp_shadow_actuate` と `dispatch_set_open` の async/sync 分岐の並行実装(MA-05、C)の 3 つ。

上位 5 件:

1. **MA-01(E)** 同期チェーンの ImmCross 腕 `SetOpenCrossProcessSync` は本番から構造上到達しない。
2. **MA-02(E)** `DriftEpisode`・`ActuationOrder::into_actuation_shadow`・`Authorization`(授権の記録)は本番の呼び出し元ゼロ。さらに `log_shadow_warrant` のログ文言と `architecture_guard.rs` のコメントが現状(A-2 強制済み)と食い違っている。
3. **MA-03(E)** `AttemptRecord.with_app_available` は本番で常に `true`。
4. **MA-05(C)** `kp_shadow_actuate` が `dispatch_set_open` の判断(claim → imm_first → 順序発行 → async/sync)を並行実装している。
5. **MA-04(C、小)** `run_open_chain_async` 冒頭の gate と `imm_cross_write` の gate は await を挟まない連続した二重検出。

## 調べた範囲と方法

`ime_controller.rs`・`imm.rs`・`ime.rs`・`platform.rs`・`output/conv_actuation.rs`・`runtime/{open_chain,conv_actuation,executor}.rs`・`runtime/key_pipeline.rs` の shadow toggle(`kp_shadow_actuate` 付近)・`state/sync_actuation.rs`・`awase-ime-core/src/state/{actuation_chain,actuation_decision_record,ime_actuation_decision,ime_set_open_plan,app_ime_policy,key_sequence_policy,drift_plan}.rs` を通読した。呼び出し元ゼロの探索は `git grep -n "\bNAME\b"` で定義と非コメント行を数えた。`output/{vk_send,probe_io,mod}.rs` の中身(warmup・defer)は ecrs-output・ecrs-tsf の担当と重なるので、actuation の入口にあたる部分だけ見た(詳細は見ていない)。

前提として確認した事実(以降の根拠):

- プロファイルは 4 値(`awase-windows-base/src/focus/class_names.rs:215-221`): `can_use_imm32_cross_process()` は `Standard` だけが真。
  `git grep -n "Self::Standard => true" -- crates/awase-windows-base/src/focus/class_names.rs` の該当行(215 付近)で確認。
- `caps()`(`awase-ime-core/src/state/app_ime_policy.rs:104-155`)の機構チェーンは、`ImmCross` を含むのは `Standard`(`ImePolicyProfile::ImmCross`)だけ。他の 3 値は `[GjiDirect]` か `[MsImeDirect]` の 1 要素。
- 同期経路は `imm_cross_is_first_applicable` が偽のときだけ使われる(下記 MA-01)。非同期経路(`run_open_chain_async`)は `Standard` のときだけ。

---

## MA-01 同期チェーンの ImmCross 腕(`SetOpenCrossProcessSync`)

- 場所: `crates/awase-windows/src/ime_controller.rs:112-139`(`apply_mechanism` の `Some(MechanismCommand::SetOpenCrossProcessSync(_))` 腕)、`crates/awase-ime-core/src/state/ime_actuation_decision.rs:134`(variant)・`:376`(`decide_attempt` の `(ImmCross, Sync)` 腕)、`decide_needs_romaji_pre_write` の `WriteMechanism::ImmCross |`(同 `:332` 付近)。
- 解いている問題: ADR-089 Phase B 以前の同期 ImmCross 書き込み。`try_force_on_bootstrap` が Standard で同期到達していた(ADR-089 §9-21)。その呼び出し元は `621bf93c`(2026-09-19、領域 A 撤去)で消えた。
- **生きているか: 本番からは構造上到達しない(推論+コード確認。実機の確認は不要)。**
  - `ImeController::apply` と `apply_sync` を呼ぶ本番の入口は 3 つ。`git grep -n "ImeController::apply\b" -- crates/awase-windows/src` の非コメント行は `platform.rs:1275`(`apply_ime_open_with_view`)・`runtime/key_pipeline.rs:1522`(shadow の sync 枝)・`state/sync_actuation.rs:375`(`dispatch_set_open` の sync 枝)。
  - 3 入口とも「ImmCross が先頭で最初に適用可能」(`imm_cross_is_first_applicable`)のときは同期へ来ない。
    - `dispatch_set_open`: `plan_set_open` が `imm_first` で `AsyncImmCross` を選ぶ(`sync_actuation.rs:340-352`)。
    - `kp_shadow_actuate`: `if imm_first { async } else { sync }`(`key_pipeline.rs:1428`)。
    - drift correction: `imm_cross = self.can_use_imm32_cross_process()`(`ime_refresh.rs:956-974`)が真なら `SendPath::ImmCross`(`set_ime_open_ordered` へ。同期チェーンではない)、偽のときだけ `StrategyChain`(`drift_plan.rs:416-420`、`ime_refresh.rs:1247`)。
  - `imm_cross_is_first_applicable(inputs)` は `chain.first()==ImmCross && 最初の適用可能 index==0`(`sync_actuation.rs:75-78`)。`ImmCross` を含むチェーンは `Standard` だけで、そのとき `imm_cross_applicable(Standard)=true` なので常に真。つまり「偽」なら chain に `ImmCross` が無い(`Standard` 以外)。同期チェーンの走査で `ImmCross` が `is_applicable` を通ることは無い。
  - 入力の同一性: 3 入口とも view/inputs は 1 回の `current_app_profile()` から作る。`Runtime::can_use_imm32_cross_process`(`runtime/mod.rs:612-616`)も同じ `current_app_profile()`。await を挟まない。
  - 凍結コーパス `awase-ime-core/tests/journals/actuation_decision/bug-131-report-01m29kdnz.json` の試行 37 件は全て `Sync × GjiDirect`(`python3` で集計: GjiDirect/SendVk22 25 件・SendVk26 10 件・null 2 件)。`ImmCross` の試行は 0 件。`git grep -l "SetOpenCrossProcess"` はコード・docs のみで JSON を含まない。
- 重複・統合先: 非同期の `imm_cross_write`(`open_chain.rs:241-`)が ImmCross の実体。同期腕は `ime::set_ime_open_cross_process`(`ime.rs:38`)を呼ぶが、この関数は `ime.rs:644` の `offload_unsafe` 経由の async ラッパーが引き続き使うので関数自体は残す。
- 撤去した場合に変わる挙動: 本番では無し(到達しないため)。将来、`caps` に「ImmCross が先頭だが適用不能な profile」を足すと同期へ漏れる。その保証は純粋な全列挙テストで固定する(下記)。
- 擬似 IME で等価性: **不要(本番の挙動が変わらない)。** 代わりに Linux の純粋な全列挙テスト 1 本で足りる(`DecisionInputs` を (profile 4 × kind 2) で回し、`!imm_cross_is_first_applicable(i)` ⇒ `decide_chain(i)` の全要素について `!(m==ImmCross && mechanism_applicable(m,i))` を assert)。`sync_actuation.rs` のテストは Linux で走る(state/ は ungated)。
- 判断: **E。** 撤去時の作業:
  1. 腕を `debug_assert!(false)`+`Failed` を返す形にするか、他 2 variant と同じ `unreachable!` にする(他 2 variant は既に `unreachable!`、`ime_controller.rs:228-244`)。
  2. `SetOpenCrossProcessSync` variant を削除。`MechanismCommand` は serde 対象だが variant の出現は上記の通りコーパスに無い。
  3. `decide_attempt` の `(ImmCross, Sync)` 腕と `decide_needs_romaji_pre_write` の `ImmCross` を削除(ImmCross は `fallback_write` の機構に入らない。`apply_mechanism` へ渡る機構は GjiDirect/MsImeDirect だけ)。
  4. 壊れるテスト: `sync_actuation.rs:596`(`apply_sync_falls_through_failed_imm_cross_and_records_both_attempts`)、`ime_actuation_decision.rs:805`(`imm_cross_sync_site_sends_set_open_cross_process_sync`)と同 `:826` 付近(`imm_cross_non_sync_sites_return_none_command` の Sync 側)、`sync_actuation.rs:610`。フォールスルー規則そのものは `actuation_chain.rs` の `run_chain` 単体テストが引き続き固定する。
  5. `tests/architecture_guard.rs` の `raw_mechanism_write_sites_are_confined_to_chain_writers` が `set_ime_open_cross_process(`/`apply_mechanism` の件数を固定している可能性がある(未確認、着手前に `git grep -n "set_ime_open_cross_process" crates/awase-windows/tests/architecture_guard.rs` で確認)。
  規模: 本番 約 −30 行、テスト 約 −40 行。
- 副次的な発見(? として別項目): 上記の結果、同期チェーン `run_chain` は常に「適用可能な 1 要素」だけを走査する(MA-07)。
- 再発ファミリー: **該当**(「キー選択」「IME actuation 合流点」)。fix-requires-evidence の (a) として上記の全列挙テストを添える。
- 規模・リスク: 小・低。

## MA-02 授権の記録の残骸(`DriftEpisode`・`into_actuation_shadow`・`Authorization`)

- 場所: `awase-ime-core/src/state/actuation_chain.rs`: `DriftEpisode`(`:640-690`)、`ActuationOrder::into_actuation_shadow`(`:407-422`)、`enum Authorization`(`:249-285`、`LegacyUnwarranted`・`PENDING`)、`Actuation::authorization()`(`:453`)。
- 解いている問題: ADR-089 §2.6(再試行 episode)、ADR-090 §2.A A-1(授権の shadow 測定)。A-2(強制)は `c8bc1adc` で実装済み。
- **生きているか: 本番の呼び出し元ゼロ。**
  - `git grep -n "into_actuation_shadow" -- crates`(コード行): 定義(`actuation_chain.rs:407`)のみ。他はコメント・docs。`git log -S"into_actuation_shadow()"` の最後の変更は `c8bc1adc`(A-2 で全入口を `into_actuation` に差し替えたコミット)。
  - `git grep -n "DriftEpisode" -- crates`: 定義と `actuation_chain.rs:919/936/950`(`#[cfg(test)]`)のみ。`architecture_guard.rs:2925-3000` のコメント自身が「本番未配線の死んだコード」と書いている。
  - `git grep -n "Authorization\b" -- crates`: 本番の読み手は無い。`Actuation::authorization()` を呼ぶのはテスト(`actuation_chain.rs:954`)だけ。`LegacyUnwarranted { would_have_blocked: true }` を作るのは `into_actuation_shadow` だけ(MA-02 の前項で消える)。`PENDING`(`would_have_blocked:false`)は `Actuation::request` の仮置き。
- 重複・統合先: `ActuationOrder::would_have_blocked()`(`warrant.is_none()`、`:398`)が `Authorization` の記録と同じ情報を持つ(drift の `warrant_would_block` は `ime_refresh.rs:974-977` でこちらを読む)。
- **併せて見つけた食い違い(機構ではなく表示・文書):**
  - `state/sync_actuation.rs:194-214` の `log_shadow_warrant` は `would_have_blocked=true` のとき `"(A-1 shadow: 書き込みは止めない。A-2 で強制する際の判断材料)"` と出す。A-2 は強制済みで、`apply_sync`(`:243`)・`run_open_chain_async`(`open_chain.rs:661`)・`set_ime_open_ordered`(`platform.rs:1313`)は warrant が無ければ書かない(`Unwarranted`/`false`)。**不具合報告のログを読む人を誤解させる**。
  - `tests/architecture_guard.rs:2925-3000` のコメントは「`into_actuation` の本番呼び出し元はゼロ」「本番で生きているのは `into_actuation_shadow` 1 本」と書く。実際は逆(`git grep -n "into_actuation()" -- crates` の本番 3 件: `platform.rs:1313`・`open_chain.rs:661`・`sync_actuation.rs:243`)。`actuation_chain.rs:424-428` の「現時点で本番呼び出し元は無い」も同様。
- 撤去した場合に変わる挙動: 無し(読み手が無い)。`log_shadow_warrant` の文言だけ変わる。
- 擬似 IME: 不要(コンパイル+既存の `actuation_chain` 単体テスト)。`issue_open_warrant` の授権判断は閉ループ(`harness.rs`)が既に本物を呼んでいる。
- 判断: **E。** 作業: `DriftEpisode`(+ そのテスト 3 本)を削除すると `architecture_guard.rs` の `Actuation::request(` 固定が `[("src/state/actuation_chain.rs", 2)]` → 1 に変わる(`:2991-2999` の期待値とコメントを更新)。`into_actuation_shadow`・`Authorization`・`Actuation::authorization()` は一緒に削る(`Actuation` の `authorization` フィールドは `OpenWarrant` を持つかどうかだけなら `Option<OpenWarrant>` へ簡素化できる。ここは任意)。規模: 本番 約 −90 行(うち `DriftEpisode` 約 55)、テスト 約 −45 行。
- 再発ファミリー: **該当**(IME actuation 合流点、warrant)。
- 規模・リスク: 小・低(ただし `architecture_guard` の文字列走査を同時に更新すること)。

## MA-03 `AttemptRecord.with_app_available`

- 場所: `awase-ime-core/src/state/actuation_decision_record.rs:212`(フィールド)。構築側: `open_chain.rs:280/404/509/542`、`state/sync_actuation.rs:132`。読み手: `awase-ime-core/src/journal.rs:1359-1360`(ログ項目 `first_with_app_available`)。
- 解いている問題: ADR-163 round1 S3・S6(`with_app(...).unwrap_or(false)` の fail-open 結果を再生入力に残す)。
- **生きているか: 本番で常に `true`。**
  `git grep -n "with_app_available" -- crates | grep -v "json:" | grep -v "with_app_available: true"` → 定義・journal.rs の読み手・`pub` 宣言のみ(`false` を入れる行は 0)。`with_app` が `None` のときは `AttemptRecord` 自体を作らず `record_actuation_decision_skipped` を数える設計(`open_chain.rs:271-285` の B-1 修正)なので、`false` の試行は構造上記録されない。
- 重複: `ACTUATION_DECISION_RECORD_SKIPPED` カウンタ(`open_chain.rs:60`)が「`with_app` 不可」を別に数えている。
- 判断: **E(小)。** フィールドとログ項目を削除。serde は未知フィールドを無視する(`git grep -n "deny_unknown_fields" -- crates/awase-ime-core/src/state/actuation_decision_record.rs` は 0 件)ので、凍結コーパス JSON の編集は必須ではない。ただし再生側テスト(`actuation_decision_record.rs` の `with_app_available: true` 8 箇所)は直す。規模: 本番 約 −12 行、テスト 約 −10 行。再発ファミリー: 該当しない(診断記録のみ)。

## MA-04 `run_open_chain_async` 冒頭 gate と `imm_cross_write` の gate

- 場所: `runtime/open_chain.rs:581-621`(冒頭 gate)と `:241-301`(`imm_cross_write` 冒頭の gate+`NotOwned` の `AttemptRecord`)。
- 解いている問題: issue #136・BUG-90 決定4(InputRelay は actuation を所有しない)、ADR-180 決定1(`is_input_relay` に集約済み)。
- **生きているか・重複:** `run_chain_async` は `WriteMechanism::ALL`(ImmCross が先頭)を走査し、`is_applicable(ImmCross)` は `self.imm.is_some()`(最初は必ず `Some`)。冒頭 gate から最初の write 呼び出しまでの間に `await` は無い(`actuation_chain.rs:565-600` の `run_chain_async`、冒頭〜`writer.write(...).await` の間は同期処理のみ)。したがって `imm_cross_write` の gate は冒頭 gate の**直後の連続した再検出**で、違うのは「冒頭の `with_app` が `None`(fail-open)だったときに 2 回目の機会がある」点だけ。`fallback_write` の gate は ImmCross の `await` の後なので別物(これは K)。
- 撤去/統合した場合に変わる挙動: 冒頭の `with_app` が再入で `None`、かつ直後の `with_app` が成功する、という稀なケースでだけ変わる(ImmCross を書くか否か)。`spawn_local` の中では `RUNTIME` の borrow が競合しない前提(`open_chain.rs` の既存コメントは「再入時」を想定しているが、実際に起きるかは未確認)。実機での頻度は `ACTUATION_DECISION_RECORD_SKIPPED`(`open_chain.rs:60`)が既に数えている。**不具合報告の journal・ログからこのカウンタの値を集計すれば、fail-open の実頻度が分かる**(リポジトリ内にデータは無い。実機/報告要)。
- 判断: **C(小、ただし ADR-180 が見送った決定1と隣接)。** 統合の形: 冒頭 gate の結果 `(inputs, is_input_relay)` を `AsyncChainWriter` に持たせて `imm_cross_write` へ渡す。ADR-180 決定1 の罠(`with_app` を内包する共有ヘルパーを `fallback_write` から呼ぶと再入で恒久 fail-open)は、**ヘルパーを作らず値を渡す**形なので踏まない。規模: 本番 約 −25 行。優先度は低い(効果が小さく、fail-open の頻度が 0 と確認できてから)。
- 再発ファミリー: **該当**(IME actuation 合流点。`architecture_guard.rs` の関数別 `is_input_relay` 件数ガード、ADR-180 で作り替えたもの、を更新する)。

## MA-05 `kp_shadow_actuate` と `dispatch_set_open` の並行実装

- 場所: `runtime/key_pipeline.rs:1370-1540`(`kp_shadow_actuate`)と `state/sync_actuation.rs:308-` `dispatch_set_open`+`runtime/executor.rs:710-815`(`dispatch_ime_set_open` の async 枝)。
- 解いている問題: 物理 IME キーの応答(shadow)と Engine の `SetOpen` の 2 経路が同一の押下で同じ書き込みをしうる(BUG-113 型の二重送信)。ADR-208 L1(押下 ID の台帳 `claim_press_write`)で**二重送信自体は解決済み**。
- **重複(挙動の二重の構造):** 次の骨格が 2 回書かれている。
  1. `claim_press_write(press, open, source)`(`key_pipeline.rs:1381`/`sync_actuation.rs:320`)
  2. `imm_first` で async/sync を分岐(shadow は `if imm_first`、`dispatch_set_open` は `plan_set_open(SetOpenFacts{claim, imm_first})`。後者は純粋関数 `ime_set_open_plan.rs` に抜けているが、shadow は呼ばず `claim.writes()`・`imm_first` を直接読む)
  3. `issue_actuation_order(...).with_press(press)` の発行
  4. async: `OutputActiveGuard::begin()` → `spawn_local` → `ActuationTarget::capture(focus_gen).await` → `run_open_chain_async` → `post_async_ime_apply_complete`
  5. sync: `apply_sync`/`ImeController::apply` → `outcome_sent_nothing` なら `release_press_write` → `record.caller = Some(..)` → `journal.record(ActuationDecision{..})`
  差分(統合時に保つ必要がある): shadow は OFF で `record_confirmed(false)` を書き込み前に行う(ADR-098 決定 5/6-a)、OFF は `Untargeted`/ON は `Targeted`、完了後に `focus_gen` が動いていたら `UnsafeToToggle` へ落とす(PR #408 Opus M-3。executor 側にはこの降格が無い、**これ自体が差**)、同期の枝で `on_ime_apply_complete` を直接呼ぶ、`PressSource::Shadow` と `Engine`、`caller` ラベル。
- 撤去/統合した場合に変わる挙動: shadow と Engine の async 枝を共通関数にすると、**executor 側にも `focus_gen` 降格が入る**(または shadow 側が失う)。どちらにするかが挙動差。Standard×(GJI/MS-IME)×フォーカス移動中の SetOpen 完了で効く。
- 擬似 IME で等価性: **今のハーネスでは検証できない**(下記ハーネス節)。`dispatch_set_open` は ungated で `CommandSink` を取るので、同期枝は Linux で本物を回せる。非同期枝(`run_open_chain_async`・`ActuationTarget`)は `cfg(windows)` で、フォーカス世代の競合・await 中の移動を模擬する窓モデルが無い。
- 判断: **C(中)。** 段階案: (1) shadow を `plan_set_open` に載せ替える(純粋、挙動不変、Linux で全列挙)、(2) sync 枝の「予約解除・caller・journal」を核へ(`dispatch_set_open` を `source`・`caller` を引数に取る形にして shadow からも呼ぶ)、(3) async の `spawn_local` 本体の共有は最後(`focus_gen` 降格の差を決めてから)。規模: 本番 約 −60 行。**architecture_guard `press_id_is_claimed_and_carried_at_every_order_issuing_entry` が現在 2 入口を固定**しているので、入口が 1 に減るなら更新が要る。
- 再発ファミリー: **該当**(IME actuation 合流点、物理 IME キーの Suppress/Allow、defer/replay)。ADR-208 の範囲。

## MA-06 drift correction の ImmCross 書き込み(`set_ime_open_ordered` → trait `set_ime_open`)

- 場所: `runtime/ime_refresh.rs:1235-1246`、`platform.rs:1032-1050`(trait `set_ime_open`、fire-and-forget)・`:1303-1320`(`set_ime_open_ordered`)。
- 解いている問題: ADR-080/BUG-43(drift の再送、Blind/Read のフィードバック)。ImmCross で書き、読み戻しで収束確認(`FeedbackPolicy::Read`)。
- **重複:** ImmCross の untargeted 書き込みが 3 経路ある: `imm_cross_write`(`Untargeted`、shadow OFF)・`platform.rs:1045`(この経路)・`runtime/mod.rs:2359-2360`(`panic_reset`、OFF→ON を直列化する SafetyValve、K)。`async_imm_cross_actuation_goes_through_the_single_chain_entry`(`architecture_guard.rs:3054-`)が `set_ime_open_cross_process_async(` の呼び出し元を 4 箇所(open_chain 1・platform 1・mod.rs 2)に固定していて、`platform.rs` の 1 件は「チェーンの対象ではない」と明示されている。
- 差(`run_open_chain_async` と比べて): outcome を返さない(`true` は「dispatch した」の意味)、ImmCross が失敗しても GjiDirect/MsImeDirect の VK へフォールスルーしない、warrant 無しなら書かない(`into_actuation().is_none()` → `false`)、`record_optimistic(desired)` は書き込み成否と無関係(dispatch できれば記録)。
- 撤去/統合した場合に変わる挙動: Standard×GJI/MS-IME で drift 補正の ImmCross が時間切れ/失敗したとき、チェーンに載せると VK(`VK_IME_ON/OFF`)が追い送りされる(drift は 500ms ごとの Read ループなので現状は次の周回で再送)。**Standard 窓での余分な VK の副作用**(IME OFF キーの履歴は `docs/experiments.md` エントリ 01)が出うる。
- 擬似 IME: drift の検知までは閉ループが既に回す(`harness.rs`、`ir_apply_drift_correction` の「検知へ進むか」まで)。実書き込みと読み戻しの遅延は `PseudoIme::set_readback_lag_ms`(Q3)があるが、ImmCross の失敗・時間切れ(Q-imm-probe-bimodal 系)の注入が無い。
- 判断: **?(実機要)。** 実機で測るもの: Standard×(GJI・MS-IME 本体)の ImmCross 書き込み失敗率(`[apply-ime] ImmCross failed ... falling through` のログ頻度、MS-IME 本体 CI の ImmCross ON 失敗〈ADR-186〉)。失敗がほぼ 0 なら統合しても挙動は変わらず C、非ゼロなら K。e2e-ime の `cal-driftrec` 系構成が使える。
- 再発ファミリー: 該当(IME belief / actuation 合流点)。

## MA-07 同期チェーンの走査が実質 1 要素(MA-01 の後の副産物)

- MA-01 により、同期の `run_chain` が走査する適用可能な機構は常に 1 つ(`[GjiDirect]` か `[MsImeDirect]`)。`run_chain`・`MechanismWriter`・`CoreSyncWriter.attempts[]`(最大 `MAX_WRITE_MECHANISMS`)の「複数機構を順に試す」実体は同期側では動かない(非同期側 `run_chain_async` だけが ImmCross→フォールバックを使う)。
- 判断: **? (C の可能性)。** 実際に同期を 1 機構の単発にすると `ActuationDecisionRecord`(`chain`・`attempts[]`)の形、凍結コーパスの `replay_chain_scan`、`actuation_chain.rs` の `MechanismWriter` 公開面が連動して変わり、MA-01 を済ませてから別に費用対効果を見る(`docs/adr/180` が「レコード統一は費用対効果が負」と結論した前例あり)。今は触らない。

## K(残すもの。理由つき)

- **gate の 4 箇所(`dispatch_set_open`・`apply_sync`・`run_open_chain_async` 冒頭・`fallback_write`)の再検出**(`fallback_write` は ImmCross の await の後なので有効、`apply_sync` は drift/shadow sync からの入口なので有効、`dispatch_set_open` の gate は claim を書かないための前置き)。ADR-119・ADR-180 の結論どおり。MA-04 の連続二重だけが C。
- **非同期チェーンが `WriteMechanism::ALL`、同期が `caps` チェーン**(`open_chain.rs` モジュール doc、ADR-089 §9-20): await をまたぐ K の推測値を固定しないため。`ime_controller.rs::caps_chain_matches_legacy_all_scan` が同値性を固定。
- **GjiDirect の `AlreadyMatched`(applied の確認済み)省略**、`explicit_press_applied_pair`(D1 の未知化)、`PressLedger`: 自動リピートの Down は `press=None` で GjiDirect の省略に頼る(`explicit_press_exhaustive.rs` の P6)。ADR-208 の全列挙が固定。
- **`romaji_pre_write`(同期ブロッキング)と `conv_after_open`(非同期の ROMAN 補完)の二重**: 前者は MsImeDirect のフォールバック先、後者は ImmCross の `Targeted`。BUG-34 横展開 E-prep が「実機ソーク無しには分割しない」と明記(`open_chain.rs` の `fallback_write` doc)。実機要。
- **`panic_reset`・トレイ ResetState の復旧書き込み**(`runtime/mod.rs:2359`、`message_handlers.rs:1247`): 緊急用の別経路で warrant を通さない設計(ADR-087 の SafetyValve)。
- **`ImmCrossOp::Untargeted`(shadow OFF)**: ADR-086 INV-14 の未移行(宛先を捕獲していない)。捕獲するなら実機ソーク。
- **`characterize_strategy`/`first_applicable_name*`(`ime_controller.rs:471-` 付近)と `tests/ime_key_sequence_golden.rs`**: Windows 専用(`#![cfg(windows)]`)で、戦略選択の表は純粋関数 `sync_actuation::mechanism_applicable`・`app_ime_policy::caps` の Linux テストと重なる。ただし golden はキー選択ファミリーの回帰点(experiments.md エントリ 01)でもあるので、**撤去しない**。表の全列挙を Linux へ足すだけなら XS。

## 見つからなかったもの(確認済み)

- 公開関数で呼び出し元ゼロ: `ime.rs::check_tsf_composition_active`(`git grep -n "check_tsf_composition_active" -- crates` → 定義 `ime.rs:1476` とコメントのみ。約 33 行。warmup の probe 用だった。`ImmContextGuard` は他に `ime.rs:416,1521`・`focus/classify.rs:38` が使うので残る)。これは前回の ECRS(コード単位)の領分だが、確認できたので **MA-08(E、小): `check_tsf_composition_active` の削除** として挙げる。規模: −33 行。再発ファミリー: 該当しない。
- `ime.rs`・`imm.rs`・`ime_controller.rs`・`platform.rs`・`runtime/executor.rs` の `pub`/`pub(crate)` 関数を「非コメント行の出現数」で走査し、出現 ≤2 の候補を全部開いた。残りは trait 実装(定義+trait 宣言で 2 件)か、1 箇所だけ呼ぶ補助関数。他に呼び出し元ゼロは無かった。
- `ActuationOrder::with_press`・`PressLedger`・`ImeStateHub::claim_press_write`・`explicit_press_*` は 2 入口で使われており生きている。

## 擬似 IME に足すと検証できる範囲が広がるもの(優先順、規模の見積り)

現状のハーネス(`tests/support/{pseudo_ime,harness,invariants}.rs`)は、belief・warrant・drift 検知・キー予測までを本物で回し、**書き込みは抽象的な `WriteCommand{origin, open, warrant, applied}` として集めて擬似 IME へ適用する**。機構(戦略チェーン・ゲート・押下台帳・ImmCross の失敗/フォールスルー)の層は通らない(`harness.rs` の冒頭 doc、「`ImeApplyRequested`/`applied` の往復」は写していないと明記)。

1. **擬似 IME を裏に持つ `CommandSink` で、本物の `apply_sync`/`dispatch_set_open` をハーネスから回す(S、約 150 行)。** どちらも ungated で、`ImeStateHub` と `CommandSink` を引数に取る(`sync_actuation.rs` のテストが既に `FakeSink` で同じことをしている)。`SendVk(vk)` を `PseudoIme::press_key(vk)` に、`None`(already-matched)は送らず、として写せば、claim → plan → 順序発行 → 同期チェーン → 予約解除・記録が Linux で通る。これで MA-01・MA-05 の同期枝・ADR-208 の P5(同一押下の二重送信なし)を閉ループで検証できる。`CommandSink` は現在 `pub(crate)` なので `pub` 化するか、テストを crate 内に置く。
2. **MA-01 の前提を固定する純粋な全列挙テスト(XS、約 20 行)。** `(profile 4 × kind 2)` で「同期へ来る入力に適用可能な ImmCross が無い」を assert。ハーネス不要。
3. **非同期チェーンの判断を核へ移し、`futures::executor::block_on` で駆動する試験台(M、約 300〜400 行)。** `imm_cross_write`・`fallback_write` の判断(gate・`imm_cross_reobservation_already_matches`・shadow_on の未知化〈BUG-113〉・`is_applicable`)は既に半分が核の純粋関数(`ime_actuation_decision.rs`)。`AsyncMechanismWriter` を擬似 IME で実装し、ImmCross の結果(`Written`/`Failed`/`Aborted(GenStale)`/時間切れ)と読み戻し値を注入できれば、MA-04・MA-05(async 枝)・MA-06 の等価性を Linux で全列挙できる。足りないもの: ImmCross の失敗・時間切れの注入、await の間にフォーカス(`focus_gen`・profile)が動く窓モデル、`with_app` 再入(None)の注入。
4. **窓・フォーカスのモデル(M)。** `ActuationTarget`(捕獲した hwnd と `focus_gen` の照合)、`InputRelay` への遷移、await 中のフォーカス移動を模擬する。MA-05 の `focus_gen` 降格の差と MA-04 の競合の検証に必要。擬似 IME 側ではなく、ハーネス側の `focus_change` に世代を持たせる拡張(`harness.rs:391` の `focus_change` が起点)。
5. **出力層の SendInput 列の記録(M)。** `send_ime_mode_key` が実際に送るキー列(修飾キーの一時解放・マーカー)を記録する差し替え口。`MechanismCommand::SendVk` の段階までは (1) で検証できるが、物理 Shift/Ctrl 押下中の前置 KeyUp 列などはここでしか見えない。キー選択ファミリーの golden(Windows 専用)の Linux 化にもなる。
6. **MS-IME 本体の格子との組合せ(既存)**: `Grid::MsimeNative` は既にある。ImmCross 書き込みが MS-IME 本体で失敗する癖(ADR-186)は注入項目として未実装(MA-06 の判断材料)。

実機でしか決まらないもの(CI の構成名): MA-04 の fail-open 頻度(不具合報告 journal の `ACTUATION_DECISION_RECORD_SKIPPED`)、MA-06 の ImmCross 失敗率(`cal-driftrec` 系、MS-IME 本体 CI)、MA-05 の `focus_gen` 降格の差(`sc-keymatrix-*`・focus 移動中の SetOpen を含む e2e-ime)。
