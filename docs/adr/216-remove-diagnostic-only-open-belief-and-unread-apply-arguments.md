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
  提案(2026-10-02)。Opus 敵対的レビュー待ち。未実装。
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
   - 本番の呼び出し元は `runtime/mod.rs:968`、`runtime/key_pipeline.rs:1271`、`runtime/executor.rs:720`、`platform.rs` の `build_ime_control_view` の4つ。
     いずれも最終的に `build_ime_control_view(applied)` に入り、`platform.rs:1147` の `applied.map(|(open, _applied_at_ms)| open)` で `u64` が捨てられる。
   - `explicit_press_applied_pair`(`state/ime_actuation_decision.rs:209`)は、`explicit_press_shadow_on`(`:195`)と同じ判定をペア版で重複して持つ。
   - `architecture_guard.rs:5867,5885` が `explicit_press_applied_pair(` の存在を文字列で固定している。
3. **`apply_ime_open_with_belief(order, applied, belief)` の `applied` は常に `None`。**
   - 呼び出し元は `runtime/ime_refresh.rs:958` の1箇所だけで、`None` を直書きしている(drift correction の OFF 方向回復。ADR-214 の決定 0 の表にも「`applied` に `None` を直書き」とある)。
   - この関数は `build_ime_control_view(applied)` → `apply_ime_open_with_view` の2行の委譲でしかない。
4. **`AppliedImeState::applied_open()`(`state/ime_model.rs:153-172`)の doc は古い。** すでに存在しない `WarmupImeOn`/`warmup_ime_on()`/`resolve_warmup_ime_on` を参照し、
   production の呼び出し元を「1箇所＋橋渡し」と書くが、現在は4箇所(`ime_model.rs:616`、`:942`、`:970`、`runtime/message_handlers.rs:960`)。

いずれも挙動に影響しない。罠に関係するのは、1(`unwrap_or(false)` の例外が消費者ごと消える)と3(「`None` ハードコードで意図的に bypass」という供給元が1つ減る)。

## 決定

次の4つを、順にコミットを分けて行う。新しい型・gate・ガードは足さない。

- **R1: `OpenBelief`/`OpenBeliefInputs`/`reduce()` と `output/ime_apply_planner.rs` を削除する。**
  - `apply_ime_open_with_view`/`apply_ime_open_with_belief` から `belief` 引数を除く。
  - `executor.rs` の `belief_inputs` の組み立て(`:910-924`)、`[dispatch-ime] belief:` のログ、`ime_refresh.rs` の `OpenBelief` の手組みを削除する。
  - 一緒に、`reduce()` だけを検証していたテスト(`ime_apply_planner.rs` 内、`executor.rs` の `chrome_intent_confident` 系)を削除する。`applied_ime_state_to_pair` のような別の関数のテストは R2 で扱う。
  - `output/mod.rs` の `pub(crate) use` と `architecture_guard.rs:5000` の `DECISION3_FILES` から該当ファイルを外す。
  - `[apply-ime] open={open} eff={} conf={} → outcome=..` は `eff`/`conf` を除いた `[apply-ime] open={open} → outcome=..` にする。`outcome=` の部分は、bug report や E2E の解析が使う
    キー(`outcome=Unwarranted`、`outcome=AlreadyMatched` 等)なので**そのまま残す**。
  - 影響を確認する(リスク節): `eff=`/`conf=` で照合している CI の解析や過去の記録(`docs/experiments.md` の I2 の記述は、過去の結果の引用なので変更しない)。
- **R2: `Option<(bool, u64)>` を `Option<bool>` にする。**
  - `AppliedImeState::to_pair()`/`ImeModel::applied_pair()` の戻り値を、開閉だけを返す形に変える(既存の `applied_open()` と重なる。重なる場合は `applied_pair()` を削除して `applied_open()` に寄せる)。
  - `explicit_press_applied_pair` を削除し、`explicit_press_shadow_on` 1本にする。`build_ime_control_view(applied: Option<bool>)` に変える。
  - `architecture_guard.rs:5867,5885` の文字列ガードを `explicit_press_shadow_on(` に更新する(ガードの意図 = 押下の order で `applied` を未知にすること、は変えない)。
  - `confirmed_at_ms()`(`AppliedImeState`)は別に残る。時刻を使う箇所は影響を受けない。
- **R3: `apply_ime_open_with_belief` の `applied` 引数を削除する。** R1 の後、この関数は「`applied=None` の view を作って `apply_ime_open_with_view` に委譲する」だけになる。
  呼び出し元が `ime_refresh.rs` の1箇所なので、関数ごと呼び出し元へインライン化できるか、名前を保ったまま引数を消すかを、`lints/actuation_call_guard/src/lib.rs` の `RESTRICTED_CALLS` と
  `architecture_guard.rs:1487`(`.apply_ime_open_with_belief(` の件数 1)、`xtask-adr-evidence` への影響を見て決める。**宣言を足す変更にはしない**(インライン化すれば宣言が1行減る)。
- **R4: `applied_open()` の doc を現状に直す。** `WarmupImeOn` への言及を削除し、4つの呼び出し元と、「省略の根拠に使うなら Confirmed かを確認する」という ADR-214 の注意だけを残す。

## 決定しないこと

- 新しい型(`AppliedOpen` など)の導入。R1〜R4 の後に残る `Option<bool>` の読み手を数え直し(見込みは `gji_direct_already_matches`、`ime_model.rs` の3箇所、`message_handlers.rs:960`、`journal.rs:854` の約5箇所)、
  型が要るかを ADR-214 の再開判断と一緒に決める。ADR-214 は「`Sent`/`Confirmed` を型で分ける」ADR で、`applied` から開閉への射影が2種類(省略の根拠用と最後に書いた値)になるため、
  値の型だけを先に入れると、後で意味が食い違う(Opus round1 M-2)。
- `journal.rs:854` の `gate_shadow_on` の出力形式の変更。`gate_shadow_known`(`is_some()`)と対で出ており、情報は失われていない。ログ形式を変えない。
- belief の reducer(`ImeModel::reduce()`)の変更。R2 は `applied_open()`/`to_pair()` の戻り値に触れるが、reducer のロジックは変えない(戻り値の型の追従だけ)。

## 検証

- 各コミットで: `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows --tests --lib`、`cargo test --lib`、
  `architecture_guard`・`layer_boundary_guard`・`golden_scenarios`・`explicit_press_exhaustive`、`mise run pre-push`。
- R1・R3(挙動を変えない削除)の確認として、`apply_ime_open_with_view` に渡る `order`/`view` が変わらないことを、`ime_key_sequence_golden.rs`(`cfg(windows)`、Linux では実行されない。
  コンパイルは `cargo check --tests` で確認し、実行は `windows-build` CI に任せる)と ADR-163 のコーパス再生(`bug-131` の 37 レコード、差分ゼロ)で確かめる。
- R2: `explicit_press_exhaustive` の全列挙テスト(押下の有無 × `applied` の3状態 × open の2値)の期待値を**変えずに**通ること。
- `cargo machete`/`cargo clippy` で、撤去で未使用になったもの(`ObservedState` の `candidate_visible` など、`OpenBeliefInputs` だけが読んでいたフィールドがあれば)を洗い出し、
  同じ PR で削除するか、残す理由を書く。
- 撤去で消えた行数(約 250〜300 行の見込み、実装後に実測してこの節へ書き戻す)を、複雑性予算の観点で記録する。

## 未確定・リスク

- **ログの `eff=`/`conf=` に頼る解析**: `docs/experiments.md`(2026-10-01、ADR-213 の CI 解析)は、I2 `Unwarranted` の特徴として `eff=false conf=true` を使った。
  リポジトリ内の `tools/`・`.github/` には、この文字列で照合するスクリプトは見つからなかった(2026-10-02 の grep)が、実行環境側(CI の解析手順)で使われていないかは未確認。
  R1 の後は `outcome=Unwarranted` と `origin=` で同じ分類ができる、という前提でよいかを確認する。
- **`ObservedState` の他のフィールドが死ぬ可能性**: `candidate_visible` 等は他の読み手もある(grep で 12〜29 件)が、`OpenBeliefInputs` だけが読んでいたフィールドが無いかは R1 の実装時に確認する。
- **`RESTRICTED_CALLS`(dylint)と `architecture_guard` の文字列ガードは、関数名や呼び出し形に依存する。** R3 でインライン化すると、宣言と件数ガードの更新が要る
  (宣言は減る方向なので `complexity-budget.md` の趣旨には沿う)。
- **見落とした消費者**: コードを読んだ範囲で「診断ログ専用」と判断したが、コンパイルが通れば消費者はいない、という前提で進め、R1 のコミットごとに CI(`windows-build`)で確認する。
- **ADR-214 の本文**(`to_pair()` で `Sent` を未知として扱う、という記述)は、R2 の後は `applied_open()` の話に読み替える必要がある。ADR-214 は保留中なので、本文への追記は R2 の実装時に行う。
- **IME actuation の合流点・belief の領域に触れる**。挙動を変えない削除でも、`fix-requires-evidence.md` の再発ファミリー(IME actuation 合流点、`shadow_on` の供給元)に該当する。
  回帰テストを足す変更ではないので、R1〜R3 のコミット本文に「挙動を変えない削除であり、既存のテスト/ガードで確認した」旨を書く。

## 参考

- ADR-215(`Option<bool>` の型化案。決定 A・B は取り下げ、決定 C だけ残す)
- `docs/adr/098-*.md` 決定1-b(`WarmupImeOn`、ADR-212 で撤去済み)
- `docs/adr/214-split-sent-from-confirmed-and-declare-skip-eligibility-per-path.md`(保留中)
- `crates/awase-windows/src/output/ime_apply_planner.rs`、`runtime/executor.rs:860-940`、`platform.rs:1130-1200`、`runtime/ime_refresh.rs:940-960`
- ADR-215 の草稿に対する Opus round1 の指摘(B-1: 消費者の撤去が先、M-1: `WarmupImeOn` の前例、M-2: ADR-214 との意味の食い違い、M-3: `From`/`Into` と `Option<(bool,u64)>` の穴)
