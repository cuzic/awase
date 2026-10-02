---
id: ADR-216
title: |-
  診断ログ専用の OpenBelief と、読まれない applied の時刻・常に None の引数を撤去する
summary: |-
  IME actuation の入口(`dispatch_ime_set_open`、`apply_ime_open_with_view`/`_with_belief`)に、結果が診断ログにしか使われない計算(`OpenBeliefInputs::reduce` → `OpenBelief`)、
  どこでも捨てられている `u64`(`Option<(bool, u64)>` の applied 時刻)、呼び出し元が1つで常に `None` を渡している引数(`apply_ime_open_with_belief` の `applied`)が残っていることを、
  コードを読んで確認した(2026-10-02、develop `db93ce88`)。いずれも挙動に影響しない。これらを撤去して、`Option<bool>` の「未知を false にする」罠(BUG-113、ADR-098 決定1-b)の読み手を減らす。
  新しい型や gate は足さない(ADR-215 の決定 A の「型で塞ぐ」案は、Opus レビューで消費者の撤去が先と指摘され、取り下げた)。撤去後に残る読み手を数え直してから、型が要るかを別途判断する。
status: |-
  提案(2026-10-02)。Opus 敵対的レビュー round3 で収束(round2 の修正条件と、round3 の2点〈`gji_last_io_ms` は消さない、R2 の回帰テスト主張を弱める〉を本文に反映済み)。未実装。
related_adr:
  - "ADR-087"
  - "ADR-098"
  - "ADR-158"
  - "ADR-208"
  - "ADR-212"
  - "ADR-214"
  - "ADR-215"
---

# ADR-216: 診断ログ専用の OpenBelief と、読まれない applied の時刻・常に None の引数を撤去する

## 背景

ADR-215 の草稿は、`Option<bool>` の罠(未知を `unwrap_or(false)` で確認済み false にしてしまう)を新しい型 `AppliedOpen` で塞ぐ案だった。Opus のレビュー
(round1)は、型を足す前に罠の**消費者そのもの**を消せると指摘した。同じ発想の型 `WarmupImeOn`(ADR-098 決定1-b)は、ADR-212 で消費者(eager warmup)ごと撤去されており、
罠を最終的に消したのは型ではなく撤去だった。この指摘を、コードを読んで確認した(2026-10-02、develop `db93ce88`)。

### 確認した事実

1. **`OpenBeliefInputs`/`OpenBelief`/`reduce()` は診断ログにしか使われない。**
   - 唯一の本番の組み立ては `runtime/executor.rs:910-924`。`reduce(open)` の結果 `belief` は、直後の `tracing::debug!`(`:926`)と
     `platform.apply_ime_open_with_view(order, &view, belief)`(`:936`)に渡るだけ。
   - `platform.rs:1162-1187` の `apply_ime_open_with_view` は `belief` を `tracing::debug!("[apply-ime] open={open} eff={} conf={} → outcome=..")` にしか使わず、
     `ImeController::apply(order, view)` には渡さない。doc 自身が「診断ログ用」と書いている。
   - `output/ime_apply_planner.rs`(148 行)の doc 自身が、`confident` を読む本番コードは存在しないと書いている(2026-08-10 の doc 訂正)。
   - もう1つの呼び出し元 `runtime/ime_refresh.rs:951-954` は `OpenBelief { effective_open: desired, confident: true }` を**手で作って渡している**だけ。
   - 付随して、`executor.rs:915` の `shadow_on: view.control.shadow_on.unwrap_or(false)`(「診断ログ専用の例外」と自分でコメントしている `unwrap_or(false)`)は、この型の入力として作られている。
2. **`Option<(bool, u64)>`(`ImeModel::applied_pair()`/`AppliedImeState::to_pair()`)の `u64` は、どこでも捨てられる。**
   - `applied_pair()`/`to_pair()` の本番の読み手は `runtime/mod.rs:968`、`runtime/key_pipeline.rs:1271`、`runtime/executor.rs:720`、`platform.rs:1202`(`apply_ime_open_with_belief` → `build_ime_control_view(applied)`)の4つ。
     いずれも最終的に `build_ime_control_view(applied)` に入り、`platform.rs:1147` の `applied.map(|(open, _applied_at_ms)| open)` で `u64` が捨てられる(例外は `#[tracing::instrument(fields(?applied))]` の span の Debug 出力だけ。`tools/`・`.github/` に照合するものはない)。
   - `explicit_press_applied_pair(pair, open, has_press)`(`state/ime_actuation_decision.rs:209`)は `explicit_press_shadow_on`(`:195`)の重複**ではない**。`has_press` が false なら降格しない、という条件を持ち、
     `executor.rs:713-724` はここに `unknowns_applied`(押下あり **かつ** `engine_press_unknowns_applied(is_effectively_tsf_native)`)を渡して、TsfNative の窓では押下があっても降格しない例外(ADR-208 L3'、BUG-124 型の「@」の実機 A/B まで)を運ぶ。`key_pipeline.rs:1271` は `press.is_some()` を渡す。
   - `architecture_guard.rs:5867,5885` が `explicit_press_applied_pair(` の存在を文字列で固定している。
3. **`apply_ime_open_with_belief(order, applied, belief)` の `applied` は常に `None`。**
   - 呼び出し元は `runtime/ime_refresh.rs:958` の1箇所だけで、`None` を直書きしている(drift correction の OFF 方向回復。ADR-214 の決定 0 の表にも「`applied` に `None` を直書き」とある)。
   - この関数は `build_ime_control_view(applied)` → `apply_ime_open_with_view` の2行の委譲でしかない。
4. **`confirmed_at_ms()`(`state/ime_model.rs:189`)の本番の読み手は `output/ime_apply_planner.rs:87` だけ。** R1 の後は読み手がゼロになる(テスト `ime_model.rs:1730-1742` のみ)。`Confirmed { at_ms }` は R1 の後、実効的には書かれるだけの値になる(`to_pair()` が組に含めるが、その時刻成分は全呼び出し元で捨てられ、`AppliedImeState` は serialize されない)。rustc の dead_code も `cargo machete` もこれは検出しない。
5. **`ObservedState` の `candidate_visible`・`gji_monitor_ok` の読み手は `executor.rs:917,919` だけ**で、R1 の後はフィールドとして書かれるだけになる(`ime_decision_view.rs:45,51,72,74,94,96`。Opus round3 が `obs.`/`view.observed.`/分割代入のいずれの形でも他に読み手がないことを確認)。**`gji_last_io_ms`(`:48`)は読み手がいる**(`runtime/key_pipeline.rs:310-311` が `obs.gji_last_io_ms` を読み、`compute_focus_probe_grace` に渡す)ので消さない(round2 でこれを「読み手ゼロ」としたのは誤りで、round3 で訂正した)。`gji_monitor_ok` の doc(「`GjiDirectStrategy` の `is_applicable` ゲートに使用」)は既に古い(実際の判定は `observed.active_ime_kind`、`ime_controller.rs:108,141`)。`candidate_was_seen` は `DecisionInputs`(ADR-171)でも読まれるので残る。`TSF_OBS` 側のアクセサは他の読み手がいるので残す。
6. **`AppliedImeState::applied_open()`(`state/ime_model.rs:153-172`)の doc は古い。** すでに存在しない `WarmupImeOn`/`warmup_ime_on()`/`resolve_warmup_ime_on` を参照し、
   production の呼び出し元を「1箇所＋橋渡し」と書くが、現在は4箇所(`ime_model.rs:616`、`:942`、`:970`、`runtime/message_handlers.rs:960`)。

いずれも挙動に影響しない。罠に関係するのは1(`unwrap_or(false)` の例外が消費者ごと消える)だけで、3は間接層の撤去であり、`None` を直書きして already-matched を意図的に迂回する供給元(drift correction の OFF 方向回復、BUG-113)の数は変わらない(`ime_refresh.rs` に `build_ime_control_view(None)` 相当が残る)。

## 決定

次の4つを、**R1 → R3 → R2 → R4 の順**に、コミットを分けて行う(R3 を R2 より先にすると、R2 が `build_ime_control_view` の引数の型を変えるときの呼び出し元が1つ減る)。新しい型・gate・ガードは足さない。

- **R1: `OpenBelief`/`OpenBeliefInputs`/`reduce()` と `output/ime_apply_planner.rs` を削除する。**
  - `apply_ime_open_with_view`/`apply_ime_open_with_belief` から `belief` 引数を除く。`executor.rs` の `belief_inputs` の組み立て(`:910-924`)、`[dispatch-ime] belief:` のログ、`:860-905` の `OpenBeliefInputs`/`belief.confident` の経緯を説明する
    コメント、`ime_refresh.rs` の `OpenBelief` の手組みを削除する。
  - `reduce()` だけを検証していたテスト(`ime_apply_planner.rs` 内、`executor.rs` の `chrome_intent_confident` 系)を削除する。`output/mod.rs` の `pub(crate) use` と `architecture_guard.rs:5000` の `DECISION3_FILES` から該当ファイルを外す。
  - **R1 で読み手がゼロになるものも同じ R1 で削除する**: `ObservedState` の `candidate_visible`・`gji_monitor_ok`(`gji_last_io_ms` は読み手がいるので残す)、`confirmed_at_ms()` とそのテスト。
    `at_ms` フィールド自体(`Confirmed { open, at_ms }`)を消すかは、`PartialEq` の意味(同じ `open` で時刻だけ違う `Confirmed` 同士が等しくなる)と、ADR-214 の `Sent`/`Confirmed` 分離で時刻を使う可能性があるため、
    **R1 では消さず、「`at_ms` は R1 の後、書かれるだけの値になる」と本 ADR に記録して、ADR-214 の再開判断に渡す**。
  - `[apply-ime] open={open} eff={} conf={} → outcome=..` は `eff`/`conf` を除いた `[apply-ime] open={open} → outcome=..` にする。`outcome=` の部分は bug report の引用や解析が使うので**そのまま残す**。
    E2E の検査(`check.py:62`、`check_invariants.py:47`)は journal の `ime open applied … outcome="Unwarranted"` 行で照合し、`[apply-ime]` の行は見ない(Opus round2 が確認)。`docs/known-bugs/`・`docs/experiments.md` 内の `eff=`/`conf=` は過去ログの引用なので変更しない。
- **R3: `apply_ime_open_with_belief` を呼び出し元へインライン化する(引数削除ではなく)。**
  - R1 の後、この関数は「`applied=None` の view を作って `apply_ime_open_with_view` に委譲する」だけで、`belief` も `applied` も無くなった名前が嘘になる。引数削除で改名すると、`RESTRICTED_CALLS`・
    `xtask-adr-evidence` の対象名・件数ガード・規約が全て追従を要するので、インライン化のほうが安全。
  - `lints/actuation_call_guard/src/lib.rs` の `RESTRICTED_CALLS`: `apply_ime_open_with_belief` の項目を削除し、`apply_ime_open_with_view` の許可リストを `["dispatch_ime_set_open", "ir_apply_drift_correction"]` にする
    (**1-in-1-out**: 許可リストへの追加と、項目の削除が対。コミット本文に書く)。
  - `architecture_guard.rs:1487` の `.apply_ime_open_with_belief(` は 1 → **0 で残す**(`.apply_ime_open_with_applied(` の 0 と同じ「復活したら気づく」ガード)。`.apply_ime_open_with_view(` は 2 のまま。
  - `crates/xtask-adr-evidence/src/main.rs:227` の対象名のハードコードから `apply_ime_open_with_belief` を消す(宣言から消えた名前は `continue` で飛ばされるので CI は落ちないが、古くなる)。
  - `platform.rs` の `apply_ime_open_with_view` の `#[allow(clippy::unused_self)]` の理由コメント(「兄弟メソッド `apply_ime_open_with_belief` から `self.` 記法で呼ばれるため」)は、兄弟メソッドが消えるので「`PlatformRuntime` 委譲メソッド群との API 配置の一貫性」だけに直す(`&self` は未使用のままなので allow は要る)。
  - `RESTRICTED_CALLS` の変更と `architecture_guard.rs:1487` の件数の変更は**同じコミット**で行う(dylint ジョブが、`ir_apply_drift_correction` が許可リストにない状態を落とす)。
  - この R3 は間接層の撤去であり、`None` を直書きする供給元の数は変えない。
- **R2: `Option<(bool, u64)>` を `Option<bool>` にする。ただし `explicit_press_applied_pair` の3引数の形は保つ。**
  - `AppliedImeState::to_pair()`/`ImeModel::applied_pair()` を、開閉だけを返す形にする(既存の `applied_open()` と重なるので、重なる場合は `applied_pair()` を削除して `applied_open()` に寄せる)。
  - `explicit_press_applied_pair(pair, open, has_press)` は `(applied: Option<bool>, open, has_press) -> Option<bool>` にするだけにする(名前を `explicit_press_applied_open` 等に変えるのは可。変えたら `.claude/rules/fix-requires-evidence.md` と
    ADR-214 `:67` の表も追従させる)。**`explicit_press_shadow_on` との一本化はしない。** `has_press` は呼び出し元ごとに違う値(`executor.rs` は TsfNative の例外込みの `unknowns_applied`、`key_pipeline.rs` は `press.is_some()`)で、
    条件を呼び出し元の `if` に移すと、書き違いが L3' の実機 A/B の前に、TsfNative × GJI で押下ごとに単発の `VK_IME_OFF`(「@」)を出す。この書き違いは `explicit_press_exhaustive` では検出できない
    (モデルが `state/explicit_press.rs:873-879` で条件を自前で再実装していて、`cfg(windows)` の `runtime/` の呼び出しを通らないため)。
  - **モデルを本番と同じヘルパーに寄せる**: `explicit_press.rs:873-879` と `:1012-1017` の自前の `if` を、このヘルパーの呼び出しに置き換える。`explicit_press_exhaustive` が降格の判定の中身について本番と同じヘルパーを通るので、
    モデルと本番の判定が乖離しなくなる。ただし**共有されるのは降格の判定だけ**で、`has_press` に渡す引数の計算は別々に書かれたまま残る。
    - Engine 経路のモデルは `has_press && engine_press_unknowns_applied(state.profile.is_effectively_tsf_native())`(`has_press` は `mode.has_press_id() && !state.was_down`)、`executor.rs:713-718` は `press.is_some() && engine_press_unknowns_applied(..)`、`key_pipeline.rs` は `press.is_some()`。
    - shadow 経路のモデル(`explicit_press.rs:1012-1017`)の条件は `has_press` ではなく、降格するのが `!(mode.has_press_id() && state.was_down)` のとき(押下 ID の無いモードでは PR #408 の「無条件に降格」を再現する)。ヘルパーへ寄せるときの第3引数はこの式で、`has_press` と書くと非 press-id モードの期待値が変わる(`explicit_press_exhaustive` の期待値が変わるので検出はされる)。
    - **この R2 は `refactor` であり、(a)(回帰テスト)の主張はしない。** executor の引数の書き違いは `cfg(windows)` の `runtime/` 配下にあり、Linux のテストでは検出できない。
      部分的に押さえるのは `architecture_guard.rs:5867,5885` の文字列ガードと `engine_press_unknowns_applied_except_tsf_native_until_l3_prime`(`ime_actuation_decision.rs:876`)だけで、書き違いの実害は L3' の実機 A/B で定数ごと消える
      例外(`ENGINE_PRESS_UNKNOWNS_APPLIED_IN_TSF_NATIVE`)の範囲に限られる。引数の計算も共有する案(`engine_press_demotes_applied(has_press, effectively_tsf_native)` を純粋関数にして executor とモデルから呼ぶ)は、
      ロジックは増えないが関数が1つ増えるので、**今回は採らない**(実装時に書き違いの懸念が強いと判断したら追加する)。
  - `build_ime_control_view(applied: Option<bool>)` に変える。`architecture_guard.rs:5867,5885` の文字列ガードは、改名した場合だけ更新する(意図=押下の order で `applied` を未知にすること、は変えない)。
- **R4: `applied_open()` の doc を現状に直す。** `WarmupImeOn` への言及を削除し、4つの呼び出し元と、「省略の根拠に使うなら Confirmed かを確認する」という ADR-214 の注意だけを残す。

### 撤去に追従させる規約・ツール・doc(各コミットの一部として扱う)

`xtask-adr-evidence` は宣言とガードの整合を CI で見ているので、これらは「ついでの docs」ではない。

- `.claude/rules/fix-requires-evidence.md`: `:34` の `output/ime_apply_planner.rs`、`:41` の「現存する `apply_ime_open_with_view` 直接呼び出し元は…`apply_ime_open_with_belief`」と `explicit_press_applied_pair` の名指し、
  `:42` の `ImeModel.applied_pair()`・`apply_ime_open_with_belief(order, None, ..)`。(`:42` の「`key_pipeline.rs` の idle-conv-check DirectInput 回復」は ADR-185 で撤去済みで、本撤去とは無関係に既に古い。ついでに直してよい。)
- `.claude/rules/complexity-budget.md:15`(チョークポイント一覧の `apply_ime_open_with_belief`)、`.claude/rules/experiment-logging.md:51`(対象ファイルの目安の `ime_apply_planner`)。
- `crates/xtask-adr-evidence/src/main.rs:227`、`architecture_guard.rs:97`(存在しないテスト名への言及)と `:1406-1415`(「実 IME actuation 入口 6 種」の doc)。
- `docs/ime-control-overview.md` などの参照資料のうち、「現在の構造」として `OpenBelief`/`apply_ime_open_with_belief` を説明している文(過去の記録の引用は変更不要)。

## 決定しないこと

- 新しい型(`AppliedOpen` など)の導入。R1〜R4 の後に残る `Option<bool>` の読み手を数え直し(見込みは `gji_direct_already_matches`、`ime_model.rs` の3箇所、`message_handlers.rs:960`、`journal.rs:854` の約5箇所)、
  型が要るかを ADR-214 の再開判断と一緒に決める。ADR-214 は `Sent`/`Confirmed` を型で分ける ADR で、`applied` から開閉への射影が2種類(省略の根拠用と最後に書いた値)になるため、
  値の型だけを先に入れると後で意味が食い違う(ADR-215 の草稿への Opus round1 M-2)。
- `Confirmed { open, at_ms }` の `at_ms` の削除(R1 の注記のとおり、ADR-214 の判断に渡す)。
- `journal.rs:854` の `gate_shadow_on` の出力形式の変更。`gate_shadow_known`(`is_some()`)と対で出ており、情報は失われていない。
- belief の reducer(`ImeModel::reduce()`)のロジック変更。R2 は `applied_open()`/`to_pair()` の戻り値の型の追従だけ。

## 検証

- **コンパイルが通ること自体が、本 ADR の主張(消費者がいない)の証明。** 各コミットで `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows --tests --lib`、`cargo test --lib`、
  `architecture_guard`・`layer_boundary_guard`、`mise run pre-push`、`xtask-adr-evidence`(CI の `adr-evidence-consistency`)。
- **R2: モデルと本番が降格の判定について同じヘルパーを通ること(引数の計算は対象外)。** `explicit_press_exhaustive` の期待値を**変えずに**通ること(押下の有無 × `applied` の3状態 × open の2値 × TsfNative かどうか)。
- **`windows-build` の E2E `sc-*` シナリオ**で、journal の `ActuationDecision`(`gate_shadow_known`/`gate_shadow_on`/`first_command`)と I2 `Unwarranted` 件数が develop と同じ分布になること。
  R2 は view の `shadow_on` の供給を変えるので、ここが挙動の本当の確認になる。実機は確認しない(挙動を変えない削除のため)。
- `ime_key_sequence_golden.rs` と ADR-163 のコーパス再生(`bug-131`)は、R1〜R3 が触る「view の組み立て」と「ログ専用の値」を通らないので、変更前後で必ず同じ結果になる。**回帰していないことの一般的な確認に過ぎない**
  (`windows-build` で実行されること、コーパスの再生が差分ゼロであること)。
- 撤去で消えた行数は、R1 実装後の `git diff --stat` で **438 行**（12 files changed, 10 insertions(+), 438 deletions(-)）。
- R2 の後の実測は、`git diff --stat` で **85 行削除**（11 files changed, 69 insertions(+), 85 deletions(-)）。

## 未確定・リスク

- **ログの `eff=`/`conf=`**: リポジトリ内に照合するスクリプトは無い(Opus round2 が `tools/`・`.github/` と E2E の検査を確認)。CI の解析手順がリポジトリの外にある場合は未確認。R1 の後は `outcome=` と `origin=` で同じ分類ができる。
- **`RESTRICTED_CALLS`(dylint)と `architecture_guard` の文字列ガードは、関数名や呼び出し形に依存する。** R3 のインライン化後の期待値は上のとおり(`.apply_ime_open_with_belief(` = 0、`.apply_ime_open_with_view(` = 2)。
- **見落とした消費者**: コンパイルが通れば消費者はいない、という前提で進める。各コミットで `windows-build` CI を確認する。
- **ADR-214 の本文**(`to_pair()` で `Sent` を未知として扱う記述)は、R2 の後は `applied_open()` の話に読み替える必要がある。ADR-214 は保留中なので、本文への追記は R2 の実装時に行う。
- **IME actuation の合流点・belief の領域に触れる**(`fix-requires-evidence.md` の再発ファミリー)。R1・R3 は `refactor`(挙動を変えない削除)で、コミット本文に「挙動を変えない削除であり、既存のテスト/ガードとコンパイルで確認した」旨を書く。
  R2 は view の `shadow_on` の供給経路(BUG-113)を書き換えるが、`refactor` として扱い (a) の主張はしない(上の R2 のとおり、引数の計算は共有されず、`windows-build` の E2E の journal 分布が実質的な確認になる)。

## 参考

- ADR-215(`Option<bool>` の型化案。決定 A・B は取り下げ、決定 C だけ残す)
- `docs/adr/098-*.md` 決定1-b(`WarmupImeOn`、ADR-212 で撤去済み)
- `docs/adr/214-split-sent-from-confirmed-and-declare-skip-eligibility-per-path.md`(保留中)
- `crates/awase-windows/src/output/ime_apply_planner.rs`、`runtime/executor.rs:860-940`、`platform.rs:1130-1200`、`runtime/ime_refresh.rs:940-960`
- ADR-215 の草稿に対する Opus round1 の指摘(B-1: 消費者の撤去が先、M-1: `WarmupImeOn` の前例、M-2: ADR-214 との意味の食い違い、M-3: `From`/`Into` と `Option<(bool,u64)>` の穴)
