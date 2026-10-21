---
title: DD3 — 「今は動作してよい状況か」を複数の層で繰り返し判定している条件(適用可否の門)
status: 調査のみ(2026-10-21、origin/develop e4766b2a)。cargo 未実行。主張には確かめた git grep を添えた。添えていないものは「未確認」と書いた
created: 2026-10-21
---

# DD3 — 適用可否の門の重なり

## 結論

- **E 候補 3 件、C 候補 6 件、要確認(不具合の疑い)1 件、K 3 件**。
- 一番大きい発見は E・C ではなく DD3-01。**「この窓は IME を読めるか」という同じ事実を、層ごとに別の名前・別の更新契機で持っている**。殻の `FocusTracker.current.app_profile` は窓のクラスが変わるたびと学習による降格で更新される。核の `ImeModel.app_policy`(`default_feedback`=Read/Blind、`focus_settle_ms`)は**プロセスが変わったとき(`FocusChanged`)にしか更新されない**。このため同一プロセス内でクラスが変わる窓と、フォーカス中に降格された窓では、drift correction・warrant の Step 4c・settle の判断が 2 つの値を混ぜて読む。害は**未確認**。
- 述語の重複そのもの(DD3-02・DD3-03)は、到達しうる入力の範囲では値が完全に一致する。Linux の全列挙で等価性を示せるので、統合は安全側に倒せる。

### 上位 5 件(確度の順)

| 順 | ID | 種別 | 内容 | 確度 |
|---|---|---|---|---|
| 1 | DD3-04 | E | `AppImeProfile::should_pass_physical_key` は本番の呼び出し元がゼロ(ADR-119 も「本番デッドコード」と書く) | 高(grep で確定) |
| 2 | DD3-02 | C | `cannot_verify_real_ime_state(class)`・`!can_use_imm32_cross_process()`・`!can_read_imm32_open_status()` の 3 述語は、到達しうる (profile, class) のすべてで同じ値になる | 高(コードで確定、全列挙のテストは未作成) |
| 3 | DD3-01 | 要確認 | 「読めない窓か」の 2 つの写し(`ImeModel.app_policy` と殻の live profile)の更新契機が違い、同一プロセス内でずれる | 機構は高、害は未確認 |
| 4 | DD3-06 | C | 「かな入力の文脈か」(`effective_open && is_japanese_ime && is_user_enabled`)を殻の 3 か所で手書きしている。`Engine::compute_state` の先頭 3 段と同じ式 | 高 |
| 5 | DD3-07 | C/E | drift の ImmCross の送り道は、同じ tick の中で「読めるか」を 3 回、授権を 2 回判定する | 高(呼び出し元が 1 つであることは確定) |

---

## 対象の門の一覧(洗い出し)

「今は動作してよい状況か」を判定している門。行番号は e4766b2a の時点のもの。

| 事実 | 門(ファイル:行) | 備考 |
|---|---|---|
| 窓の IME を IMM で読める・書けるか | `class_names.rs:215` `can_use_imm32_cross_process`、`:253` `can_read_imm32_open_status`、`:295` `cannot_verify_real_ime_state`、`runtime/mod.rs:612`(ラッパー)、`key_sequence_policy.rs:43` `imm_cross_applicable`、`sync_actuation.rs:75` `imm_cross_is_first_applicable`、`observation_store.rs:46` `FocusProbeOpenStatus::classify`、`ime.rs:863` `read_ime_state_fast`、`platform.rs:1036` `set_ime_open` | DD3-02・DD3-07 |
| 読み戻せない窓(Blind)か | `app_ime_policy.rs:104` `caps().feedback`(→ `ImeModel.app_policy.default_feedback`)、`key_pipeline.rs:1957` `unreadable`、`explicit_press.rs:232` `PressProfile::is_blind`(モデル専用) | DD3-01・DD3-03 |
| 学習で読めないと分かった窓か | `focus/tracker.rs:67` `is_demoted_by_learning`、`key_pipeline.rs:1902` `imm_capability(..) != Unavailable` | DD3-01 の補足 |
| 実質 TSF ネイティブか | `class_names.rs:277` と、本番の呼び出し 10 か所(`git grep -n "is_effectively_tsf_native(" -- 'crates/awase-windows/src/**' \| grep -v "///" \| wc -l` → 10) | DD3-08 |
| 物理 IME キーを awase が持つか | `class_names.rs:241` `should_pass_physical_key`、`app_ime_policy.rs:167` `owns_physical_kanji`、`physical_disposition.rs::plan_core` | DD3-04・DD3-05 |
| awase が actuation を持たない窓(InputRelay)か | actuation 5 か所(`dispatch_set_open`・`apply_sync`・`run_open_chain_async`・`imm_cross_write`・`fallback_write`)、`physical_disposition.rs:117`、`late_keymap_plan.rs:166`、`runtime/mod.rs:837`(役割の付与)、`key_pipeline.rs:1960`、`lang_detect_shadow.rs:235`、`ime_event.rs:277`(ImmCross へ写す) | DD3-10(K) |
| 日本語 IME か | `open_warrant.rs:165`、`drift_plan`(`DriftIdle::NotJapaneseIme`)、`ime_refresh.rs:902`(`ir_align_placeholder_desired`)、`engine.rs:273` `compute_state` ほか | DD3-06・DD3-11(K) |
| IME の種類 | `ActiveImeKind`(`tsf/observer.rs:701`)、`ImeKindId`(`ime_kind.rs:29`)、`TipIdentity`(`ime_kind.rs:80`)、`table_ime_kind()`(`observer.rs:374`)、`current_tip_identity()`(`observer.rs:389`)、`external_change_watch_applies_for`(`runtime/mod.rs:2604`)、`direct_mode_key_watch_kind_for`(`runtime/mod.rs:2593`) | DD3-09 |
| 授権 | `open_warrant.rs:158` `issue_open_warrant_inner`、`ime_refresh.rs:975` `warrant_would_block`、`platform.rs:1313` `into_actuation`、`sync_actuation.rs:241` | DD3-07 |

担当外として扱ったもの: `DeferGate`・`OUTPUT_GATE`(`state/deferred_gate_plan.rs` に既に集約済み。defer/replay のファミリーなので他の担当)、`FOCUS_RESYNC`・`InputBarrier`・保持バッファ(INDEX の MD-07・MD-16・MD-18 で扱い済み)、`press_ledger` の claim と `applied` の already-matched(二重送信の担当)。

---

## 要確認(不具合の疑い)

### DD3-01 「読めない窓か」の 2 つの写しが、別の契機で更新される

- **門**:
  - 殻: `FocusTracker.current.app_profile`。`focus/current.rs:75` が `from_class_and_process(class_name, ..)` で毎回入れ直し、`focus/tracker.rs:161-177` が学習による降格(Standard→Imm32Unavailable)を載せる。`update_with_process_name` は 500ms のリフレッシュでも毎回呼ばれる(`tracker.rs:139` のコメント、BUG-111)。
  - 核: `ImeModel.app_policy`(`default_feedback`・`focus_settle_ms`)。書き込みは `ime_model.rs:961`(起動時)と `:1078`(`FocusChanged`)だけ(`git grep -n "app_policy = " -- 'crates/**/src/**'` → 本番 2 件+テスト 1 件)。
  - `FocusChanged` を送るのは `focus_tracking.rs:610` の `on_focus_process_changed` 経路だけで、**プロセスが変わったときに限る**(`apply_focus_probe_result` の `if process_changed { self.on_focus_process_changed(..) }`、`focus_tracking.rs:97-99`)。同一プロセス内の窓の変化は `FocusHwndUpdated { hwnd }`(`ime_event.rs:477`)で、profile を運ばない(`ime_model.rs:942-946` は観測の fence しか触らない)。
- **(a) 不変条件**: 「この窓は読み戻せない(Blind)」という判断は、actuation のすべての判断で同じ値を使う。
- **(b) ずれる状況**(コードから導いた。実機では未確認):
  1. 同一プロセス内でクラスの分類が変わる窓。例: `explorer.exe` のエクスプローラー本体(Standard)と `XamlExplorerHostIslandWindow`(Imm32Unavailable、`class_names.rs:29-31`)。最初にフォーカスした窓の分類が `app_policy` に残る。
  2. フォーカス中に学習で降格された窓(`learn_imm_capability_from_miss` は `ime_refresh.rs:747` のリフレッシュ中に呼ばれる)。殻は次の tick で Imm32Unavailable になり、核は Read のまま。
- **読み手が混ざっている箇所**:
  - drift correction の `ir_observe_drift_facts`(`ime_refresh.rs:943-1000`)は `imm_cross = self.can_use_imm32_cross_process()`(殻)と `default_policy = default_feedback()`(核)を**同じ facts に並べる**。`decide_drift_plan`(`drift_plan.rs:405-440`)は送り道を `imm_cross` で、打ち切りの有無を `policy` で選ぶ。降格した窓では「StrategyChain で送る」かつ「Read なので回数で打ち切らない」という、caps の表に無い組み合わせになる(BUG-43 が型で防ごうとした「同じ VK の再送」の上限が外れる。実際には観測が古くなれば `NoDrift(StaleObservation)` で止まる可能性があり、**害の有無は未確認**)。
  - warrant の Step 4c(`open_warrant.rs:223`)は核の `default_feedback == Blind` だけを見る。降格した窓では Step 4c が働かず、StrategyChain の order が `Unwarranted` になりうる(未確認)。
  - settle の長さ(`focus_settle_ms`、100/200/500ms)も核の値を使う。
- **(c) 根拠**: ADR-087 §8(`docs/adr/087-open-belief-actuation-warrant-separation.md:1744`)が「`app_policy` の同一プロセス内追随」を Phase 3 の未決事項として挙げていたが、決着の記録は見つからない(`git grep -n -i "同一プロセス.*app_policy" -- docs crates` → この 1 件だけ)。
- **(d) 依存**: DD3-02・DD3-03 は、この 2 つの写しのどちらを正とするかが決まらないと統合先が決まらない。
- **判定**: **要確認(要判断)**。E/C の候補ではなく、統合の前に**どちらを正にするか**を決める必要がある。
- **案**: (i) 殻のプロファイルが変わったとき(クラスが変わったとき・降格のとき)に `app_policy` だけを入れ直す専用イベントを足す(`ime-belief-architecture.md` の「専用イベント」の型。belief には触らない)。または (ii) `default_feedback` を `ImeModel` に持たせず、読む側が live profile から `caps()` を引く(`AppImePolicy` は ADR-089 §2.5 が「caps のファサード」と書いており、保持する理由は settle 期限の算出だけ)。(ii) は核に殻の値を渡す引数が増えるため、(i) が小さい。
- **検証**: Linux の純粋テストで `decide_drift_plan` の (imm_cross, policy) の 4 通りを全列挙し、「imm_cross=false × Read」(降格・同一プロセス)の結果を固定する(今は `drift_plan.rs` のテストがこの組み合わせを扱っているか未確認)。実機では journal の `FocusTransition` で `pid` が同じまま `profile` が変わる行を数える(`bug-report-latest` で既存の報告を grep。件数は未確認)。

---

## E 候補

### DD3-04 `AppImeProfile::should_pass_physical_key`(本番の呼び出し元ゼロ)

- **門**: `crates/awase-windows-base/src/focus/class_names.rs:241-246`。
- **(a)**: 物理 IME キーを OS に届けてよいか(Imm32Unavailable だけ false)。
- **(b)**: 単独で止める場面は無い。実際の配送の判断は `state/physical_disposition.rs::plan_core` がする(BUG-46 の修正 `076b8709` で置き換わった)。doc(`class_names.rs:237-239`「`KeyEventPipeline::stage_execute` は `Decision::Consume` に変換する」)は古い。
- **(c)**: `git grep -n "should_pass_physical_key" -- 'crates/**/src/**'` → 定義とテスト(`class_names.rs:513-660`)だけ。ADR-119 の summary(`docs/adr/119-*.md:6`)が「`should_pass_physical_key` が本番デッドコードだった」と書く。
- **(d)**: `plan_core` と同じ事実の古い写し。
- **判定: E**。INDEX の MD-01(`uses_kanji_toggle`)と同じ PR で消す。
- **壊れるもの**: `class_names.rs` の `app_ime_profile_getters_truth_table`・`input_relay_predicates_are_explicit`・`oracle_for`/`OracleResult`/`actual_for`(列を 1 つ減らす)、`architecture_guard.rs::input_relay_profile_wiring_occurrence_counts_are_pinned` の `("src/focus/class_names.rs", 10)`(この関数の match 腕の `InputRelay` が 1 つ減る。MD-01 と合わせて 2 減)、`architecture_guard.rs:5545` の doc。

### DD3-05 `AppImePolicy::owns_physical_kanji`(本番の読み手ゼロ)

- **門**: `crates/awase-ime-core/src/state/app_ime_policy.rs:167`、`:216`。
- **(a)**: 物理 KANJI を awase が持つか(TsfNative だけ false)。
- **(b)**: 単独で止める場面は無い。doc 自身が「実際に物理キーを配送するかどうかの SSOT ではない…本フィールドを直接参照して suppress 判定を書かないこと」(`:160-166`)と書く。
- **(c)**: `git grep -n "\.owns_physical_kanji\b" -- 'crates/**/src/**' 'crates/**/tests/**'` → `app_ime_policy.rs` のテスト 4 件、`ime_model.rs:1328`(テスト)、`ime_profile_driver.rs:239-240`(パリティテスト)、`tests/golden_scenarios.rs:212,367` のみ。
- **(d)**: DD3-04・`plan_core` と同じ事実の 3 つ目の写し。
- **判定: E(条件付き)**。`ime_profile_driver.rs` の `owns_physical_kanji()` とパリティテストは ADR-090 が凍結した部品(INDEX の所有者判断)なので、**`ime_profile_driver.rs` の扱いが決まってから**消す。消すときは `golden_scenarios.rs` のシナリオ 5(`scenario_5_wezterm_tsf_profile_policy`)の断言を `focus_settle_ms`・`default_feedback` に差し替えるか、シナリオを `plan_core` 側の表(`physical_disposition.rs` の全列挙)へ寄せる。

### DD3-07(E 部分)drift の ImmCross の送り道で、殻の `set_ime_open` が「読めるか」を判定し直す

C 部分と合わせて下の「C 候補」に書く。

---

## C 候補

### DD3-02 「IMM で読める・書ける窓か」の 3 述語が同じ値になる

- **門**: `class_names.rs:215` `can_use_imm32_cross_process`(Standard のみ true)、`:253` `can_read_imm32_open_status`(Standard のみ true。**match の腕が完全に同じ**)、`:295` `cannot_verify_real_ime_state(class)`(Standard は `is_effectively_tsf_native(class)`、他は true)。
- **(a)**: この窓では IMM の open status を信頼して読める(書ける)。
- **(b) 同値の根拠**: `app_profile` を書くのは `focus/current.rs:75` の `from_class_and_process(&class_name, ..)` と `tracker.rs:177` の降格(Standard→Imm32Unavailable)だけ(`git grep -n "app_profile = \|\.app_profile = " -- 'crates/awase-windows/src/**'`)。`from_class_name` は `is_tsf_native_window(class)` が真なら Standard を返さない(`class_names.rs:149-157`)。降格先は Imm32Unavailable。したがって **(Standard, TSF ネイティブのクラス) は本番で作られない**。その結果、到達しうるすべての入力で `cannot_verify_real_ime_state(class) == !can_use_imm32_cross_process() == !can_read_imm32_open_status()`。`class_names.rs` の独立オラクル(`oracle_for`、`cannot_verify = is_input_relay || is_unavailable || effectively_tsf_native`)もこの等式を言い換えている。呼び出し元が自分で解決する 2 か所(`ime.rs:854`、`runtime/mod.rs:1814`)も `AppImeProfile::resolve(&class_name, ..)` で同じクラスから作るので同じ。
- **(c)**: 本番の呼び出し元の数。`can_use_imm32_cross_process` 約 14 か所(platform 1・executor 1・ime_refresh 3・key_pipeline 1・runtime/mod 4〈うち 1 はラッパー本体〉・key_sequence_policy 1・physical_disposition 2・explicit_press 1〈全列挙モデル〉)、`can_read_imm32_open_status` 2 か所(`ime.rs:863`、`observation_store.rs:46`)、`cannot_verify_real_ime_state` 1 か所+`should_reprime_on_lightweight_focus_sync` 経由 1 か所(`key_pipeline.rs:1959`、`runtime/mod.rs:1818`)。
- **(d)**: `cannot_verify_real_ime_state` の Standard の腕(`self.is_effectively_tsf_native(class_name)`)は、上の理由で常に false を返す死んだ分岐。
- **判定: C**。
- **案**:
  1. `cannot_verify_real_ime_state` と `should_reprime_on_lightweight_focus_sync` から `class_name` 引数を外し、本体を `!self.can_use_imm32_cross_process()`(または `!matches!(self, Self::Standard)`)にする。
  2. `can_read_imm32_open_status` を消して `can_use_imm32_cross_process` に寄せるか、逆に 1 つの名前(例 `imm32_readable`)にまとめる。読みと書きを別の名前で残したいなら、片方をもう片方の 1 行の委譲にして match を 1 つにする(将来「読めるが書けない」プロファイルを足すときに分ければよい)。
- **壊れるもの**: `class_names.rs` のテスト(真理値表・オラクル・`all_imm32_unavailable_classes_are_classified_as_unavailable`)、`input_relay_profile_wiring_occurrence_counts_are_pinned` の件数、`#[actuation_choke_point]`(ADR-158 TE3 の観測フェーズ。`can_use_imm32_cross_process` に付いている。名前を変えるなら `architecture_guard.rs:5048 can_use_imm32_cross_process_wrapper_keeps_track_caller` と `docs/adr/158-implementation-tasks.md:520-545` を合わせて直す)、`architecture_guard.rs:600`(`panic_reset` の本文に `"if !self.can_use_imm32_cross_process()"` という文字列を要求している)。
- **注意**: `ime.rs::read_ime_state_fast` は学習による降格を載せない静的なプロファイルで判定する(`ime.rs:854-857`)。降格された窓でも IMM を読みに行く。これは述語の重複ではなく入力の違いなので、統合しても変えない(変えるなら別の判断)。

### DD3-03 「読み戻せない窓(Blind)か」の 3 つの計算

- **門**:
  - `key_pipeline.rs:1955-1960` `unreadable = cannot_verify_real_ime_state(class) && profile != InputRelay`。DD3-02 により `matches!(profile, Imm32Unavailable | TsfNative)` と同じ。
  - `caps(profile.into(), kind).feedback`(`app_ime_policy.rs:104-147`)。`InputRelay` は `ImmCross` に写る(`ime_event.rs:277`)ので Read。Blind になるのは Imm32Unavailable・TsfNative だけ。上と同じ集合。
  - `PressProfile::is_blind`(`explicit_press.rs:232`、全列挙モデル専用)。同じ集合。
- **(a)**: この窓では実 IME の open を直接読めない(予測・OwnSsot・回数の上限が要る)。
- **(b)**: どれも他の 2 つと同じ集合。ただし `caps` 側の値は DD3-01 のとおり `FocusChanged` 時点の写し。
- **(c)**: ADR-209(`unreadable`)、ADR-080/087(`FeedbackPolicy::Blind`)、ADR-208(`PressProfile`)。
- **判定: C(DD3-01 の決着が先)**。`AppImeProfile::is_blind()` を 1 つ足し、`key_pipeline.rs:1957` はそれを使う。`PressProfile::is_blind` には「`self.app_profile().is_blind()` と一致する」ことを全列挙で固定するテストを足す(`ime_actuation_decision.rs:926` が `is_effectively_tsf_native` で同じことをしている)。`caps` の feedback を `is_blind` から導くかは、`Plain`/`Unknown` の行(INV-44)があるので `caps` 側は表のまま残す。

### DD3-06 「かな入力の文脈か」を殻の 3 か所で手書き

- **門**: `key_pipeline.rs:2121-2124`(`entry_context_ok`、+ `conv_mutation_allowed`)、`key_pipeline.rs:2161-2164`(`entry_ime_ok`)、`lang_detect_shadow.rs:332-334`(`entry_ime_ok`)。どれも `effective_open() && belief.is_japanese_ime() && engine.is_user_enabled()`。
- **(a)**: Engine が「ユーザー無効」「日本語 IME でない」「IME OFF」のどれでもない。
- **(b)**: これは `Engine::compute_state`(`src/engine/engine.rs:267-282`)の先頭 3 段と同じ式で、`build_ctx`(`runtime/mod.rs:519-523`)が `ime_on = effective_open()`・`is_japanese_ime = belief.is_japanese_ime()` を渡す。`engine.rs:286-288` は `ime_off_inactive` について「判断は `compute_state` のままで、呼び出し側が再実装しない」と書いている。同じ方針がこの 3 か所には適用されていない。
- **(c)**: 半角英数トグルのファミリー(BUG-193/ADR-245)と lang_detect(ADR-263)。Down と Up で 2 回判定するのは、その間に状態が変わりうるので**判定の回数は減らさない**(K)。減らすのは「式の写し」だけ。
- **判定: C**。`Engine` に `fn kana_context_ok(&self, ctx: &InputContext) -> bool`(`compute_state` が `Active` か `Inactive(NotRomajiInput)`)を足し、3 か所は `self.engine.kana_context_ok(&self.build_ctx())` を呼ぶ。`build_ctx` が `read_os_modifiers` を呼ぶので、修飾キーを読まない軽い版(3 値だけを渡す)を用意する方がよい。
- **等価性の検証**: Linux で `compute_state` の全列挙(user_enabled × is_japanese × ime_on × input_mode の各 variant)と旧式の比較。`cargo test --lib` で走る。

### DD3-07 drift の ImmCross の送り道で、同じ tick に「読めるか」を 3 回、授権を 2 回判定する

- **門**(すべて `ir_apply_drift_correction` の同じ同期呼び出しの中):
  1. `ime_refresh.rs:972` `facts.imm_cross = self.can_use_imm32_cross_process()` → `decide_drift_plan` が `SendPath::ImmCross` を選ぶ(`drift_plan.rs:416-420`)。
  2. `ime_refresh.rs:974-977` `warrant_would_block = imm_cross && issue_actuation_order_with_origin(..).would_have_blocked()`(授権 1 回目、order は捨てる)。
  3. `ime_refresh.rs:1233` `issue_actuation_order_with_origin(desired, act_origin)`(授権 2 回目)。
  4. `platform.rs:1313` `set_ime_open_ordered` の `order.into_actuation().is_none()`(2 回目の授権の結果を見る)。
  5. `platform.rs:1036` `PlatformRuntime::set_ime_open` の `if !self.current_app_profile().can_use_imm32_cross_process() { return false; }`(「読めるか」の 2 回目)。
- **(b)**: `set_ime_open_ordered` の本番の呼び出し元は drift の ImmCross の 1 か所だけ(`git grep -n "set_ime_open_ordered" -- 'crates/**/src/**'` → 呼び出しは `ime_refresh.rs:1243` のみ。focus change の強制 OFF は 2026-09-25 に撤去済み、`ime_refresh.rs:877`)。トレイトの `set_ime_open` の呼び出しは `platform.rs:1316` だけ(`architecture_guard.rs::ime_open_actuation_entry_points_are_accounted_for` が `.set_ime_open(` = 0 を固定)。1 と 5 の間に profile を変える処理は無い(同期、`with_app` の中)ので、**5 は常に 1 と同じ値**。3 は 2 と同じ状態に対する同じ問い合わせ。間にある `DriftDetected` の dispatch は warrant の入力(intent_store・observations・force_guards・app_policy・desired_open・is_japanese_ime)を変えない……**と読んだが、`reduce` の `DriftDetected` 腕は精読していない(未確認)**。
- **(c)**: BUG-163(warrant の事前判定、ImmCross のみ)、ADR-090 §2.A A-2。
- **(d)**: 「読めるか」はさらに、`dispatch_set_open`・`kp_shadow_actuate` では `imm_cross_is_first_applicable(inputs)` という別の式で判定している(`sync_actuation.rs:75`)。`caps` の chain の先頭が ImmCross なのは Standard と InputRelay、ImmCross が適用可能なのは Standard だけなので、`imm_cross_is_first_applicable ⇔ profile == Standard ⇔ can_use_imm32_cross_process()`。drift だけが別の式を使う。
- **判定**:
  - **E**: `platform.rs:1036-1038` の profile の再判定(5)。`debug_assert!` にするか消す。トレイトの `set_ime_open` は「死んだ入口が復活したら気づく」ために残す方針(`architecture_guard.rs:2020` のコメント)なので、メソッドは残して中の判定だけ外す。
  - **C**: 授権は 1 回にする。`ir_observe_drift_facts` で発行した order を facts の外(`Option<ActuationOrder>`)に持って `ir_apply_drift_correction` まで運ぶ。`ActuationOrder` はアフィン値なので facts(`Copy`)には入れない。
  - **C**: drift の `facts.imm_cross` を `imm_cross_is_first_applicable(DecisionInputs)` で求め、「ImmCross で送るか」の式を 1 つにする(`sync_chain_never_applies_imm_cross_for_any_profile_and_kind` がすでに profile × kind を全列挙している)。
- **関連(挙動が変わるので要判断)**: 事前の授権判定(2)は ImmCross の送り道だけにある。StrategyChain の送り道(Blind の窓)では、授権が下りない order でも `DriftDetected`・journal・試行回数の加算まで進んでから `apply_sync` が `Unwarranted` を返す。Blind では Step 4c(OwnSsot)がほぼ必ず授権するので、下りないのは force guard が動いていて `desired=false` のときくらい(未確認)。統合して両方の送り道で事前判定するなら挙動が変わる。

### DD3-08 `is_effectively_tsf_native(class)` を (profile, class) から 10 か所で計算し直す

- **門**: 本番 10 か所(executor・focus_tracking 3・ime_refresh 2・key_pipeline 3・lang_detect・runtime/mod)。`focus_tracking.rs:760-768` のコメントは「同一引数での重複計算の指摘」で 1 関数内の 2 回を 1 回にした経緯を書いている。
- **(b)**: DD3-02 の理由で、`class` が結果を変えるのは profile が Imm32Unavailable のときだけ(`CASCADIA_HOSTING_WINDOW_CLASS` 等、`class_names.rs` のバケット)。
- **判定: C(小)**。`FocusTracker::update_with_process_name` で 1 回計算して `CurrentFocus` に `effectively_tsf_native: bool` として持ち、`focus.effectively_tsf_native()` で読む。クラス名の文字列を渡し間違える余地(`focus_tracking.rs:631` は `classified.class_name` を、他は `focus.class_name()` を渡す)が消える。値の差は無い(`advance_focus_tracking` の後で両者は同じ文字列、未確認)。

### DD3-09 IME の種類の同じ関数が、型を変えて 2 つずつある

- **門**:
  - `ActiveImeKind`(2 値、`tsf/observer.rs:701`)と `ImeKindId`(2 値、`ime_kind.rs:29`)は同型で、`From` で 1 対 1 に写る(`observer.rs:711-718`)。
  - `table_ime_kind() -> Option<ImeKindId>`(`observer.rs:374`)と `current_tip_identity() -> TipIdentity`(`observer.rs:389`)は同じ 3 値の関数(Gji / 同定済みの MS-IME 本体 / それ以外)。
  - `external_change_watch_applies_for(profile)`(`runtime/mod.rs:2604`: `profile == Imm32Unavailable && active_ime_kind() == GoogleJapaneseInput`)と `direct_mode_key_watch_kind_for(profile) == Some(Gji)`(`runtime/mod.rs:2593` → `direct_watch_kind`)は同じ値。`kp_observable_toggle_guard`(`key_pipeline.rs:1901`)は後者の形を使う。
- **(c)**: ADR-205(外部変化の監視、GJI 限定)、ADR-188/244(直接観測)。両 ADR の doc が「別の述語」と明記しているのは**適用範囲の違い(GJI だけか、MS-IME 本体も含むか)**で、GJI の判定そのものは同じ。
- **判定: C(小)**。`external_change_watch_applies_for` を `direct_mode_key_watch_kind_for(profile) == Some(ImeKindId::Gji)` と書き、`current_tip_identity` を `table_ime_kind()` から導く。`ActiveImeKind` の撤去は 22 ファイル 129 行(`git grep -n 'ActiveImeKind' -- 'crates/**/src/**' | wc -l`)の置き換えで得るものが少ないので勧めない。
- **Mozc の判別**: `is_gji_family_description`(Mozc を GJI に含める)と `TipIdentity::Gji`(Google 版だけ)は、memory と `ime_kind.rs:40-46` のとおり意図して範囲が違う。重複ではない(K)。

---

## K(独立に必要)

### DD3-10 InputRelay の門(約 11 か所)

- actuation の 5 か所は ADR-180 が K と決めたもので、今回もそのまま(`dispatch_set_open` の早い門は `DecisionSite::DispatchImeSetOpen` の記録と「書かない窓では予約しない」ため、`apply_sync` の門は drift・shadow toggle の入口のため、非同期の 3 か所は await 後の窓の変化のため)。
- 残り(`plan_core`・`late_keymap_plan`・役割の付与・`unreadable`・`lang_detect`・`ImePolicyProfile` の写像・`FocusProbeOpenStatus`)は、**同じ事実を見ているが止めている副作用がそれぞれ違う**(物理キーの配送、遅延キーマップ、役割、予測、言語の自動判定、caps の行、観測の取り込み)。1 か所にまとめると「InputRelay なら全部やめる」という単一の門になるが、InputRelay の窓でも NICOLA の変換は動く(エンジンは止めない)ので、まとめる単位が無い。
- 気付いた非対称: `kp_shadow_actuate`(`key_pipeline.rs:1391`)は門の**前に** `claim_press_write` を呼び、`dispatch_set_open`(`sync_actuation.rs:317-348`)は門の**後に**呼ぶ。InputRelay ではどちらも何も送らないので害は無い。統合は INDEX の MA-05(`kp_shadow_actuate` と `dispatch_set_open` の並行実装)で一緒に揃える。

### DD3-11 `is_japanese_ime` の門

- drift では `decide_drift_plan` の `DriftIdle::NotJapaneseIme` が先に止めるので、warrant の `!ctx.is_japanese_ime`(`open_warrant.rs:165`)は drift の経路では**常に先に止められている**(INDEX の ME-1 が「2 重の関門」と書いたもの)。ただし warrant の判定は押下 ID の無い Engine の `SetOpen`(自動リピート等)にも効き、そこでは drift の門を通らない。drift の門を消すと、日本語 IME でない窓で `DriftDetected`・試行回数の加算・バルーンまで進んでから `Unwarranted` になる。**どちらも消せない(K)**。
- `ir_align_placeholder_desired`(`ime_refresh.rs:902`)と `decide_drift_plan` は同じ tick に同じ 2 条件(engine 有効・日本語 IME)を判定するが、前者は belief を書く別の処理なので K。

### DD3-12 `is_demoted_by_learning` と `imm_capability(..) == Unavailable`

- `may_mirror_applied_on_focus`・`lang_detect_window_in_scope` は前者(静的な分類が Standard で学習が Unavailable)を、`kp_observable_toggle_guard`(`key_pipeline.rs:1902-1906`)は後者(学習の生の値。静的な分類が Imm32Unavailable の窓も含む)を見る。集合が違い、ADR-262 と ADR-260 がそれぞれ意図して選んでいる。K。

---

## 検証に要るもの

| 候補 | Linux で取れる等価性 | 足りないもの |
|---|---|---|
| DD3-02 | `class_names.rs` に「`IMM32_UNAVAILABLE_CLASSES` ∪ `is_tsf_native_window` の全クラス ∪ 代表の非該当クラス」×「relay 一致の有無」×「降格の有無」を全列挙し、3 述語の一致を固定するテストを先に足す(今のオラクルは 5 バケットの代表だけ)。`cargo nextest run -p awase-windows-base` | なし |
| DD3-03 | `PressProfile::ALL` で `is_blind() == app_profile().is_blind()`、`AppImeProfile` 4 値で `is_blind() == matches!(caps(p.into(), k).feedback, Blind)` を全列挙 | なし(DD3-01 の決着が先) |
| DD3-06 | `compute_state` の全列挙と旧式の比較(`cargo test --lib`) | なし |
| DD3-07 | `decide_drift_plan` の (imm_cross, policy, warrant_would_block) の全列挙は純粋で取れる。order を 1 回にする変更は殻(`runtime/`、`#[cfg(windows)]`)なので Linux では走らない。`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib` と windows-build の CI | ME-2(ハーネスに `dispatch_set_open`・drift の殻を通す)が入れば擬似 IME で送信列(`MechanismCommand`)を比べられる |
| DD3-01 | `ImeStateHub` を直接動かし、`FocusChanged(ImmCross)` の後に殻側だけ Imm32Unavailable にした状態で `decide_drift_plan` に渡る facts を組む純粋テストで、組み合わせの結果は固定できる | **害の有無は実機要**。同一プロセス内の分類の変化(`explorer.exe` の本体と XAML 島)と、フォーカス中の降格(一太郎等)で、journal の `FocusTransition`(pid 同じ・profile 違い)と `[drift]`・`[warrant-shadow]` の行を数える。擬似 IME には窓モデル(INDEX の ME-5、同一プロセス内でクラスが変わる窓の構築子)が要る。e2e-ime に該当ターゲットがあるかは未確認 |
| DD3-04・05・09 | grep で十分(呼び出し元ゼロ・同値の式) | DD3-05 は `ime_profile_driver.rs` の所有者判断 |

この調査で見ていないもの: `reduce` の `DriftDetected` 腕が warrant の入力を変えないこと(DD3-07)、`advance_focus_tracking` の後で `classified.class_name == focus.class_name()` であること(DD3-08)、`drift_plan.rs` の既存テストが「imm_cross=false × Read」を扱っているか(DD3-01)。どれも着手前に読むこと。
