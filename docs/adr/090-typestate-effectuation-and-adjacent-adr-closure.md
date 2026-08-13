# ADR-090: ADR-089 の型保護を実効化し、隣接 ADR の後始末を確定する — warrant 実配線 / 読み戻し API / 裏口の可視性 / 非同期 caps / dylint 方針 / ADR-081 Phase 1d 凍結

## ステータス

**ドラフト（計画のみ、実装未着手）。本 ADR 起票時点でプロダクションコードの
変更は 0 行である。**

本 ADR は [ADR-089](089-ime-typestate-and-capability-const-table.md) の
Phase A/B/C を実装した結果 §9 に残った「型は書いたが、まだ効いていない」
残課題を、**実コードを読んで現状を確定させた上で**設計へ落とすものである。
対象は次の 7 件で、内訳は ADR-089 §9 の項番と対応する:

| 項 | 内容 | ADR-089 §9 の対応項 |
|---|---|---|
| **A** | `issue_open_warrant()` の本番呼び出し元がゼロ（`warrant_pending_adr087()` の迂回路） | §9-12 |
| **B** | `ConvergedReceipt` が制御フローに未接続（INV-46 が空回り） | §9-16 |
| **C** | witness の強度が不均一・観測ストアの裏口が残る | §9-11、§9-8 |
| **D** | 非同期チェーンが `caps` 化されていない（`WriteMechanism::ALL` のまま） | §9-20、§6 Phase C 実施記録 C-4 |
| **E** | dylint 2 crate（`ime_event_guard` / `observation_source_guard`）の恒久方針 | §7「撤去するもの — 無し」 |
| **F** | ADR-081 Phase 1d の凍結可否 | §9-4、§6「ADR-081 Phase 1d の凍結（提案）」 |
| **G** | golden の `KEY_DOC` に削除済み関数名が残っている | §7「維持するもの」 |

**本 ADR は新規実測を一切含まない。** §2 の事実確認はすべて
`958e21c2`（`feat/adr-089-ime-axis-typestate`）時点の実コード読解による。
実測が必要になる作業は A-2 / D にのみ現れる。

### 起票時に確定させた事実（ADR-089 §9 の記述より詳しい／一部訂正）

ADR-089 §9 は「まだ効いていない」ことは正確に書いていたが、**閉じるための
コストを決める事実**までは書いていなかった。本 ADR の起票にあたって実コードで
裏取りした結果、次の 4 点が判明した。**うち 1 件は ADR-089 §7 の記述の訂正で
ある。**

1. **`most_recent_trusted_after` を絞らない限り、B（`ConvergedReceipt` の配線）は
   保証を生まない。** 同メソッドは `pub` で、本番呼び出し元は
   `runtime/ime_refresh.rs` の 2 箇所だけである。receipt を返す API を足しても
   **元の `Option<&ImeObservation>` を返す口が公開されたまま**なら、
   「読み戻しの産物は `ImeObservation` として手に入らない」（ADR-080 不変条件6）は
   成立しない。**B の本体は receipt を作ることではなく、`ImeObservation` を返す
   since-fenced 読み口を塞ぐことである**（§2.B）。
2. **C（裏口の可視性）で crate 外に残っている口は 1 つだけで、閉じるコストは
   テスト 2 行の書き換えである。** `ObservationStore::per_source` を crate 外から
   読んでいるのは `tests/golden_scenarios.rs` の 2 箇所
   （`:332` の `per_source.observer_poll` / `:375` の `per_source.gji`）だけで、
   `src/` 配下の `per_source` 直接アクセス（`ime_model.rs:733`/`:783`、
   `platform_state.rs:1519`、`open_warrant.rs:258`）は**すべて `#[cfg(test)]` の
   中**である。ADR-089 §9-11 は「`tests/golden_scenarios.rs` が読んでいるため
   `pub` のまま」と書いたが、読み取り専用アクセサ 1 本で解消する
   （**書き込みの裏口を塞ぐのに読み取りを犠牲にする必要はなかった**）。
3. **A の配線を縛るのは「レイヤ境界」でも「`Copy`」でもなく、
   `WarrantContext` の材料を持つ型（`Runtime`）と warrant を消費する型
   （`ImeController` / `ImeControlView`）が別物であることである。**
   `build_ime_control_view` は `WindowsPlatform` のメソッド（`platform.rs:1005`）で、
   `WarrantContext` の 8 材料のうち `intent_store` / `obs` / `guards` / `policy` /
   `desired_open` は `Runtime.platform_state.ime`（`ImeStateHub`）側にある。
   したがって view の構築点からは warrant を作れず、**warrant は引数として
   運ぶ**しかない。**これは `tests/layer_boundary_guard.rs` の禁止ではない**
   （B-1 の禁則ディレクトリは `observer/` `focus/` `output/` `state/` +
   `ime.rs` の 5 領域で `ime_controller.rs` を含まず、当の `ime_controller.rs`
   は既に `crate::state::{actuation_chain, app_ime_policy::caps,
   ime_decision_view}` を import している）。実際に効いている制約は
   **`with_app` 再入**である（詳細は §2.A.2(1)）。
   さらに実 actuation 入口のうち 2 経路（`ime_refresh.rs:534`/`:752`）は
   `set_ime_open` という**トレイトメソッド**（`platform.rs:710`）を通っており、
   ここにだけは引数を足せない（§2.A の設計案 3 が扱う）。
4. **【ADR-089 §7 の訂正】G（golden の古い関数名）は実機を待つ必要が無い。**
   ADR-089 §7 は「更新には golden の再生成が要るため、次に実機で golden を回す
   ときにまとめて直すこと」と書いていたが、`build_report()`
   （`tests/ime_key_sequence_golden.rs:120`）が golden 文字列を組み立てる過程で
   `KEY_DOC` は**そのまま `push_str` される定数**であり、`UPDATE_GOLDEN=1` での
   再生成は Windows 上で走る通常 CI（`windows-build` ジョブの
   `cargo nextest run -p awase-windows --test ime_key_sequence_golden`）が
   検証する。**手で `.rs` と `.txt` を同時に直せば CI が一致を判定する**ので、
   実機ソークは要らない。**あわせて、golden に残っている stale な名前は
   `set_ime_romaji_mode()` だけではない**——Phase B で撤去した
   `apply_skipping_imm` が golden 本文に **7 箇所**（dispatch 列の値および
   凡例行）残っている。G のスコープはこの 2 つを含む（§2.G）。

### 起票後の訂正（独立レビュー 1 巡目、2026-08-12）

**起票時の草稿は、設計判断の根拠として実コードと食い違う記述を 4 件含んで
いた。** いずれも独立レビューの指摘を受けて実コードで再確認し、本文を
差し替えた。**結論（項 A の「warrant を引数で運ぶ」、項 F の「ADR-081
Phase 1d/1e 凍結」）はどちらも変わっていない**が、根拠が違えば次に検討する
人の判断も変わるため、何をどう間違えたかを残す
（`.claude/rules/experiment-logging.md` の
「なぜ前回それを捨てたのかを辿れるようにする」規約）。

| # | 草稿の記述 | 実コードの事実 | 差し替え先 |
|---|---|---|---|
| 1 | 「`ime_controller` から `ImeStateHub` を読むのはレイヤ境界違反」「`ime_controller.rs` は `state` を読めない windows-gated レイヤ」 | `layer_boundary_guard.rs` の B-1 禁則ディレクトリは `observer/` `focus/` `output/` `state/` + `ime.rs` で `ime_controller.rs` を含まない。当の `ime_controller.rs:37-42` は既に `crate::state::*` を import している。真の制約は **`with_app` 再入**（`open_chain.rs:203` の `fallback_write` が既に `with_app` の中） | §2.A.2(1)、§4.2 |
| 2 | 「ADR-081 の不変条件1 は Phase C が達成済み」 | `ImeActuatorKind` は消えたが、同じ不変条件のもう半分 `AppImeProfile::` へのパターンマッチが非テストコードに **7 サイト（variant 出現 9 個）**残り、要求されたテキスト走査ガードも存在しない。**半分しか達成していない** | §2.F.2 の表、§7-8 |
| 3 | 「ADR-081 Phase 1e のブロッカーは解決済み」 | 非対称そのものは INV-42 で解消したが、Phase 1e の成果物（`platform.rs:879-891` の legacy 同期の撤去）は **ADR-089 INV-43 が明示的に禁止**している。「解決した」ではなく「moot になった」 | §2.F.3 根拠 3 |
| 4 | 「dylint は `let src = ObservationSource::X; .. source: src` の間接構築まで検出できる」 | `observation_source_guard` の `path_expr_ident`（`lib.rs:196-202`）は `ExprKind::Path` の最終セグメントしか見ないので、**まさにこの例を検出しない**。dylint の実際の優位は crate 全体走査 / `typeck` による variant 解決 / `EngineActivationSync` の単独防御の 3 点 | §2.E 決定 E-1、§4.8 |

このほか、`ImeControlView` に warrant を載せない理由（`Copy` ではなく責務）、
`Resolution` の値数（3→4 ではなく **2→4**）、実 actuation 入口の数
（11 は call site 数で、**外部入口は 8**）、§2.D.2 の「§4.9」が指す ADR
（本 ADR ではなく **ADR-089**）、INV-50 の新規性（ADR-089 INV-44 の再掲に
なっていた）、項 F の規模見積り（150〜200 行 → **net 280〜330 行**）を訂正した。

**B・C・D・G の事実記述はレビューで再検証され、いずれも正確だった**
（G の 7 箇所、C の「実依存は 2 行だけ」、B の bit-identical、
D の `ALL` = `[ImmCross, GjiDirect, MsImeDirect, KanjiToggle]` からの等価性）。

### 起票後の訂正（レビュー 2 巡目、2026-08-12）

**設計判断の変更は無い。行番号・件数の事実誤りが 3 件見つかったので直した。**
本 ADR は「実コードを読んで現状を確定させた」ことを自ら規律として掲げており、
その規律に対する違反そのものなので、他の訂正と同じく記録に残す。

| # | 誤り | 実コードの事実 | 直した箇所 |
|---|---|---|---|
| 5 | `AppImeProfile::` の残存を 2 箇所で「**8 箇所**」と書いたが、自らの列挙は 7 サイトだった | 非テストの分岐は **7 サイト・variant 出現 9 個**。どちらの数え方でも 8 にならない（構築点を数えれば増えるが、不変条件が禁じているのは分岐であって構築ではない） | §2.F.2 に 7 サイトの内訳表を新設、§7-8 / F-3 の参照も更新 |
| 6 | `platform.rs` 内部委譲を「`:1016` / `:1028` / `:729`」と書いた | 実在しない行。正しくは **`:1063`（`_with_belief` の中）/ `:1075`（`_with_applied` の中）/ `:735`（トレイト `apply_ime_open` の中）**。外部 8 + 内部 3・期待値 4/3/2/2/0 という**構造と件数は実測と一致していた**ので結論は不変 | §2.A.2(3) の表（委譲の階段も明示）、冒頭事実確定 3 |
| 7 | `try_force_on_bootstrap`（`:892`）、`ime_refresh.rs:727` の位置づけ | `try_force_on_bootstrap` の定義は **`:871`**（`:892` はその中の実 write 呼び出し）。`ime_refresh.rs:727` に呼び出しは無く、近いのは `log::warn!`（`:723-726`）の文字列（`:725`） | §2.A 設計案 2、§2.A.2(3) 脚注 |

あわせて §5 の並び順が INV-53 → INV-52 と逆転していたのを直し、
末尾に「次は INV-54 から」と明記した（末尾だけを見て採番する将来の ADR が
53 を飛ばさないようにするため）。

### 用語（本 ADR を単独で読むための最小定義）

本 ADR は ADR-087/089 の続きだが、次の 5 語の意味が分かれば §2 は単独で読める。
定義の SSOT は実コードの側にある（括弧内）。

| 語 | 意味 | 実体 |
|---|---|---|
| **`caps(p, k)`** | (IME ポリシープロファイル `p`, IME 種別 `k`) → 「どの機構チェーンで書くか / 読み戻せるか（`feedback`）/ フォーカス後どれだけ待つか」を返す **const 表**。capability の宣言点を 1 箇所に集める（ADR-089 INV-44） | `state/app_ime_policy.rs:108` の `pub const fn caps` |
| **`Observed<E>`** | 「証拠型 `E` を伴う観測」。`E` ごとに観測ソースと confidence が**型で固定**され、観測値だけを後から付け替えることができない（ADR-089 Phase A） | `state/evidence.rs` |
| **witness** | ある値を構築できることが「その外部事実が実際に起きた」ことの証拠になる引数。例: `AcceptedObservation`（probe 受理を通った証）を要求すれば、probe を実行せずに観測を記録できない | `state/probe_admission.rs` 等 |
| **warrant（`OpenWarrant`）** | 「この値を実 IME へ書き込んでよい」という**根拠軸**の授権。`issue_open_warrant()` が safety valve / 明示意図 / 実観測 / ヒューリスティック / 自己 SSOT の 5 Step を順に評価して発行する。空間軸（どの HWND へ書くか）は別型 `ActuationTarget` が持つ（ADR-086 INV-14） | `state/open_warrant.rs:136` |
| **receipt** | 「その処理が確かに終わった」ことを表す戻り値型で、**観測へ変換する手段を持たない**。`ActuationReceipt`（GjiFsm 同期義務）と `ConvergedReceipt`（読み戻しの帰結、INV-46）の 2 種がある | `state/gji_direct_mechanism.rs` / `state/ime_actuation.rs:88` |

`Actuation<Requested> → <Warranted> → <Verified>` は型状態チェーンで、
`Requested → Warranted` が warrant（根拠軸）、`Warranted → Verified` が
`ActuationTarget`（空間軸）を要求する。実 write を起こす `run_chain` は
`Verified` にしか生えない（`state/actuation_chain.rs`）。

### 番号空間

- **invariant**: ADR-084 が INV-1〜11、ADR-086 が INV-12〜19、ADR-087 が
  INV-20〜28、ADR-088 が INV-29〜37、ADR-089 が INV-38〜46 を使用済みのため、
  本 ADR は **INV-47 から**採番する。
- **原則（P 番号）**: ADR-089 が P19〜21 まで使用済みのため、本 ADR は
  **P22 から**採番する。
- 本 ADR が採番するのは **INV-47〜53** である（§5）。

### ADR-087 / ADR-089 との役割分担

**項 A の実装は ADR-087 Phase 3 に属する。** ADR-087 §5 item14/15 は
「どの入口が warrant 必須か」を棚卸ししたが、**発行した `OpenWarrant` を
どうやって `Actuation` チェーンまで運ぶか**は書いていない。ADR-089 Phase B が
`Actuation<Requested>::warrant(OpenWarrant)` という受け口を作ったことで、
初めてその配線の形が問える状態になった。本 ADR §2.A はその**運搬経路の設計**で
あり、実装コミットは ADR-087 Phase 3 として記録する（invariant は本 ADR が
採番する）。

---

## 1. コンテキスト

### 1.1 ADR-089 が到達した地点と、到達していない地点

ADR-089 Phase A/B/C は「規律をコンパイラへ移す」ことを 3 箇所で試み、
**構造は全部入った**。しかし §9 が正直に記録しているとおり、
そのうち実際に**規律を強制している**のは一部である:

| 型 | 構造 | 現に効いているか |
|---|---|---|
| `Observed<E>` + `PoolKind` 関連型 | 入った（`state/evidence.rs`） | **一部**。`record`/`record_belief` の本番呼び出し元がゼロで、観測はすべて `record_replayed(AnyObservation)` を通る（§9-10）。効いているのは witness 構築子が `AnyObservation` の唯一の通常経路であることと、evidence 型ごとの `SOURCE`/`CONFIDENCE` 固定の 2 点 |
| `Actuation<Requested/Warranted/Verified>` | 入った（`state/actuation_chain.rs`） | **一部**。段階の順序（`run_chain` は `Verified` にしか生えない）とアフィン性（1 値 = 高々 1 回の成功 write）は効いている。**「warrant 無しに write しない」は効いていない**——`warrant_pending_adr087()` が 2 箇所（§9-12） |
| `ActuationReceipt`（`GjiFsm` 同期義務） | 入った（`state/gji_direct_mechanism.rs`） | **効いている**（`#[must_use]` + `Drop` の `debug_assert`）。ただし保証水準は debug ビルドの実行時検出（§8.1） |
| `ConvergedReceipt`（INV-46） | 入った（`state/ime_actuation.rs`） | **効いていない**。構築されるが `log::debug!` にしか渡らず、収束判定は `most_recent_trusted_after` の返す `ImeObservation` が担う（§9-16） |
| `caps(p, k)` const 表 | 入った（`state/app_ime_policy.rs`） | **同期経路のみ**。非同期経路は `WriteMechanism::ALL` のままで、INV-44 は「`ALL` は全 `caps` チェーンの和集合」という**論証**に依存している（§9-20） |

**この表の「一部」「効いていない」を「効いている」へ変えるのが本 ADR の
項 A〜D である。**

### 1.2 なぜ今まとめて起票するのか

3 つの理由がある。

1. **項 C・F は「今が最も安い」タイミングにある。** 可視性の縮小（C）は、
   `per_source` を読む新しいコードが増えるほど差分が広がる。ADR-081 Phase 1d の
   凍結判断（F）は ADR-089 §6 自身が「配線前の今なら撤去コストがテストの削除
   だけで済む」と書いており、配線が進むほど高くなる。
   **ただし「テストの削除だけ」は正確ではない**——本 ADR が実コードで数え直した
   結果、削除は net 280〜330 行で、`caps` 駆動への差し替えを 2 件伴う
   （§2.F 決定 F-2'、F-R4）。「安い」は相対的な話であって「ただ」ではない。
2. **項 A・D は互いに絡んでいる。** 非同期チェーンで await をまたいで
   フォーカスが動きうる（D）ことと、起案時に発行した warrant が完了時点でも
   有効か（A）は、**同じ「await をまたいだ前提の失効」という一つの問題の
   2 つの面**である（§2.A の設計案 4、§2.D の設計案）。別々の ADR に分けると
   一方だけを直して他方を壊す。
3. **項 E は 4 回間違えた論点である。** ADR-089 の r2〜r5 は 4 ラウンド連続で
   「dylint 2 crate は Phase A の型化で置き換えられる」と書き、Phase A の実装時に
   実コード照合で誤りと判明した（ADR-089 §7 の訂正）。**「置き換えない」までは
   確定したが、「では恒久的にどうするのか」は決まっていない**ため、次に
   この領域を触る人が 5 回目の同じ提案をしうる。`.claude/rules/experiment-logging.md`
   の「なぜ前回それを捨てたのかを辿れるようにする」規約を、ここでも適用する。

---

## 2. 決定

各項について「現状の問題 / 具体的な設計案 / 実装した場合のリスク /
優先度 / 規模感」を記す。優先度と規模の一覧は §3 にまとめる。

---

### A. `issue_open_warrant()` を実配線し、`warrant_pending_adr087()` を消す

#### A.1 現状の問題

`state/open_warrant.rs::issue_open_warrant()` は ADR-087 Phase 0〜2' で
**純粋関数として完成しており、240 通りの差分オラクルテスト
（`differential_old_gate_vs_issue_open_warrant`）まで持っている**が、
**本番の呼び出し元は 1 箇所も無い**（`src/` 全体を grep、`958e21c2` 時点。
ヒットするのは同ファイル内のテストと doc コメントのみ）。

その結果、ADR-089 Phase B が作った `Actuation<Requested>::warrant(OpenWarrant)`
という正規経路は**誰も通らず**、代わりに

```rust
// crates/awase-windows/src/ime_controller.rs:420（同期チェーン）
// crates/awase-windows/src/runtime/open_chain.rs:227（非同期チェーン）
let actuation = Actuation::request(open)
    .warrant_pending_adr087()      // ← 授権を素通しする暫定入口
    .verify(target);
```

という迂回路を 2 箇所が通る。件数は
`tests/architecture_guard.rs::legacy_unwarranted_actuation_sites_are_accounted_for`
（期待値 2）が固定している。

**したがって「warrant 無しに実 IME へ書き込まない」は、現時点で型としては
一切効いていない。** 効いているのは段階の順序とアフィン性だけである
（ADR-089 §9-12）。

#### A.2 配線を難しくしている 3 つの構造的事実

実コードを読んで確定させた、設計を縛る事実:

**(1) warrant を発行できる型と、warrant を消費する型が別である。**

`issue_open_warrant(requested, target, ctx)` が要求する `WarrantContext`
（`state/open_warrant.rs:116`）は 8 フィールド（`intent_store` / `obs` /
`guards` / `policy` / `desired_open` / `is_japanese_ime` / `now` / `now_ms`）で、
先頭 5 つはすべて `Runtime.platform_state.ime`（`state/platform_state.rs` の
`ImeStateHub`）配下にある。`intent_store` は `ImeStateHub` の**private
フィールド**（`platform_state.rs:73`）で、`dispatch_event` が
`UserImeSetIntent{SyncKey|PhysicalImeKey|Command}` を受けたときに `record()`
する write-only 配線が ADR-087 §8.11 item9 で入っている（読み手はまだ無い）。

一方、warrant を消費する `Actuation::request(..).warrant(..)` は
`ime_controller.rs:420`（同期）と `runtime/open_chain.rs:227`（非同期）にある。
`ImeController::apply` が受け取るのは `&ImeControlView` だけで、
**`Runtime` への handle をシグネチャに一切持たない**。

**ここでレイヤ境界を根拠にしてはならない（本 ADR 起票時に一度書いて訂正した）。**
実コードでの事実は次のとおり:

- `tests/layer_boundary_guard.rs::b1_with_app_confined_to_orchestrator_modules`
  の禁則ディレクトリは `src/observer/` `src/focus/` `src/output/` `src/state/`
  の 4 つ + `src/ime.rs` であり、**`ime_controller.rs` は含まれない**。
- `ime_controller.rs:37-42` は既に `crate::state::actuation_chain` /
  `crate::state::app_ime_policy::caps` / `crate::state::ime_decision_view` を
  import している。「`ime_controller.rs` は `state` を読めない」は誤りである
  （ADR-065 が禁じているのは逆向き——`state/` が windows 型に依存すること）。

**実際に効いている制約は `with_app` 再入である。** `ImeController` から
`ImeStateHub` に手を伸ばす唯一の手段は `crate::with_app(..)`
（`lib.rs:203`）だが、`ImeController::apply` の呼び出しフレームは
既に `with_app` の内側にいる: 非同期経路の `fallback_write`
（`open_chain.rs:203-212`）は
`crate::with_app(|app| { let view = app.shadow_ime_control_view(); ..
apply_mechanism(..) })` という形をしており、その中でさらに `with_app` を
呼べば `RUNTIME.try_borrow_mut()` が失敗する。同期経路も
`apply_ime_open_with_view` 経由で `Runtime` のメソッドから呼ばれるため、
実質同じ位置にいる（[[project_in_with_app_removal]] の経緯）。

**しかも `with_app` は再入時に panic せず `log::warn!` して `None` を返す。**
つまりこの経路で warrant を取ろうとすると、**取れなかったことが
「授権が下りなかった」と区別できない形で静かに落ちる**——A-1 の shadow ログ
（`would_have_blocked`）が測ろうとしている当のものが汚染される。
これは「やりにくい」ではなく「やってはいけない」形である。

**したがって結論（warrant は引数で運ぶ）は変わらないが、根拠は
「レイヤ境界違反」ではなく「(a) 材料を持つ型が別、(b) `with_app` 再入」で
ある。** この違いは実装の形に効く——レイヤ境界が理由なら「境界を跨がない
新モジュールを作る」で解けるが、再入が理由なら**呼び出し元が既に持っている
`&ImeStateHub` から値を作って渡す**以外に解が無い（§2.A の設計案 1 が
`ActuationOrder` を「入口で作る値」にしているのはこのため）。

**(2) `ImeControlView` に載せるのは、`Copy` の問題ではなく責務の問題である。**

**`Copy` を根拠にしてはならない（これも起票時の誤り）。** `ImeControlView<'a>`
は既に lifetime 付きの借用構造体で（`FocusFacts<'a>.class_name: &'a str`、
`state/ime_decision_view.rs:112-122`）、`Option<&'a OpenWarrant>` を足しても
`Copy` は失われない。載せない理由は次の 3 点である:

1. **構築点が warrant を作れない。** view を組むのは
   `WindowsPlatform::build_ime_control_view`（`platform.rs:1005`）で、
   `WindowsPlatform` は `ImeStateHub` を持たない（(1) と同じ事実）。
   `Runtime::shadow_ime_control_view`（`runtime/mod.rs:419`）はそれを呼んで
   `belief_input_mode` だけ上書きする薄いラッパである。
2. **view には actuation でない読み手が居る。** `is_applicable` /
   `ImeController::imm_cross_is_first_applicable`（同期 / 非同期の
   dispatch 分岐判定）/ `first_applicable_name` / `characterize_strategy`
   （golden）はいずれも view を読むが**書き込みはしない**。warrant を view の
   フィールドにすると、これらの読み取りのたびに授権を発行することになる
   （`IntentStore::lookup` は TTL 消費こそ無いが、「授権を発行した」という
   journal / ログの意味が壊れる）。
3. **view は機構ごとに作り直される。** `fallback_write` はチェーンの各機構で
   view を作り直す（`open_chain.rs:206`）。warrant を view に載せると
   **チェーンの途中で暗黙に再発行される**——それは §2.A 設計案 4 が
   「明示的にやる」と決めていることであり、暗黙に起きてはならない。

**(3) 実 actuation 入口は外部 8 + 内部委譲 3 で、うち 2 経路はトレイトメソッド越しである。**

`tests/architecture_guard.rs::ime_open_actuation_entry_points_are_accounted_for`
が固定しているのは**呼び出し箇所（call site）数**であり、入口の数ではない。
期待値 4/3/2/2/0 の合計 11 のうち 3 件（`platform.rs:1063` / `:1075` /
`:735`）は `platform.rs` 内部の委譲なので、**外部から叩かれる入口は 8 である**:

| needle | 期待値 | 内訳 | 引数追加の可否 |
|---|---|---|---|
| `.apply_ime_open_with_belief(` | 4 | 外部 3（`ime_refresh.rs:765` / `key_pipeline.rs:742` / `runtime/mod.rs:892`）+ 内部委譲 1（`platform.rs:1075`、`apply_ime_open_with_applied` の中） | 可（`WindowsPlatform` の inherent メソッド、定義は `platform.rs:1056`） |
| `.apply_ime_open_with_view(` | 3 | 外部 2（`executor.rs:854` / `runtime/mod.rs:735`）+ 内部委譲 1（`platform.rs:1063`、`apply_ime_open_with_belief` の中） | 可（同上、定義は `platform.rs:1038`） |
| `.apply_ime_open_with_applied(` | 2 | 外部 1（`ime_refresh.rs:499`）+ 内部委譲 1（`platform.rs:735`、トレイト実装 `apply_ime_open` の中） | 可（同上、定義は `platform.rs:1069`） |
| `.set_ime_open(` | 2 | 外部 2（`ime_refresh.rs:534`/`:752`） | **不可**（`platform.rs:710` の**トレイト実装**。`awase` 側のトレイト定義を触ることになる） |
| `.apply_ime_open(` | 0 | — | 呼び出し元ゼロの死んだ入口（トレイト実装 `platform.rs:734`。`:730-733` のコメントが「誰からも呼ばれない trait オーバーライド」と自認している） |

内部委譲は
`apply_ime_open`(:734) → `_with_applied`(:1069) → `_with_belief`(:1056) →
`_with_view`(:1038) → `ime_controller::CONTROLLER.apply`(:1044)
という 1 本の階段になっており、**外部 8 入口はこの階段のどの段にも合流しうる**
（`_with_applied` に 1・`_with_belief` に 3・`_with_view` に 2、加えて
`set_ime_open` の 2 は階段の外）。A-1 で `ActuationOrder` を通すときは
**外部 8 入口それぞれが起案する**（§2.A 設計案 1）のであって、
階段の下段で受けて上へ配る形にはできない——`WarrantContext` を組めるのは
`Runtime` 側だけで、階段は `WindowsPlatform` のメソッドだからである（(1)）。

`.set_ime_open(` の 2 件は `ime_refresh.rs:534`（focus change の強制 OFF、
IMM32 のみ）と `:752`（drift correction の ImmCross 分岐）で、**どちらも
`Runtime` のメソッドの中**＝`WarrantContext` を組み立てられる場所にある。

> **付随して見つかった小さな stale**: 上記ガードのコメント
> （`tests/architecture_guard.rs:742`）は `.set_ime_open(` の内訳を
> 「`ime_refresh.rs:534/727`」と書いているが、`:727` に呼び出しは無い。
> 近いのは `log::warn!`（`:723-726`）の文字列
> 「`→ set_ime_open({desired})`」（`:725`）で、これは先頭に `.` が無いため
> そもそも `count_real_calls`（`architecture_guard.rs:50`）の needle
> `.set_ime_open(` に一致しない。**実際の 2 件目は `:752`** である。
> A-1 でこのガードを作り替えるときに**コメントも直す**こと。

#### A.3 具体的な設計案

**設計案 1: `ActuationOrder` 値を作り、入口で warrant を発行して運ぶ。**

引数を 5 関数に足すのではなく、1 つの値にまとめて運ぶ:

```rust
// crates/awase-windows/src/state/actuation_chain.rs（ungated、Linux でテスト可）
/// 実 actuation 入口が起案する 1 件の指示。`Actuation<Requested>` の材料。
#[derive(Debug, Clone)]
pub struct ActuationOrder {
    open: bool,
    /// `issue_open_warrant()` の結果。`None` = 授権が下りなかった。
    warrant: Option<OpenWarrant>,
    /// どの入口が起案したか（ADR-082 `EventOrigin` と journal を揃える）。
    origin: EventOrigin,
}

impl ActuationOrder {
    /// 唯一の構築経路。**`issue_open_warrant()` の戻り値をそのまま受ける形**に
    /// して、warrant を「作らない」選択肢を型から消す。
    pub fn issue(open: bool, target: HwndId, ctx: &WarrantContext<'_>, origin: EventOrigin) -> Self {
        Self { open, warrant: issue_open_warrant(open, target, ctx), origin }
    }

    /// 授権が下りていれば `Warranted`、下りていなければ `None`（強制モード）。
    pub fn into_actuation(self) -> Option<Actuation<Warranted>> {
        Actuation::request(self.open).warrant(self.warrant?)
    }
}
```

`ActuationOrder::issue` を**唯一の構築経路**にすることで、
「warrant を発行せずに actuation を起案する」ことが型として書けなくなる
（**INV-47**）。`warrant_pending_adr087()` は削除する。

**設計案 2: 二段階で入れる（shadow → enforce）。**

差分オラクル（`differential_old_gate_vs_issue_open_warrant`、ADR-087 §8.11
item10 / §8.12 M2）は、旧ゲート（`is_eligible_for_ime_force_on()` =
`is_japanese_ime() && effective_open()`）と `issue_open_warrant()` の判定が
**8 通りで旧のみ許可・1 通りで新のみ許可**と食い違うことを既に測っている。
つまり**そのまま強制すると 9 通りの挙動が変わる**。

**内訳を本 ADR にも転記しておく**（`state/open_warrant.rs:1177-1216` の
コメントが SSOT。ADR-087 §8.12 だけを参照していると、A-2 を分割する単位が
読めない）:

| 群 | 件数 | 条件 | 旧 | 新 | 意味 |
|---|---|---|---|---|---|
| **old-1** | 1 | `policy=ImmCross`・観測 / 意図 / guard 一切なし・`desired_open=true` | 許可 | `None` | 旧は観測皆無時に `most_recent_trusted` が外れて `desired_open` にフォールバックする。新は `Read` プロファイルのため Step 4c（`OwnSsot`）が発火しない。**`try_force_on_bootstrap` 相当。判明した中で最大の挙動変化** |
| **old-2** | 4 | `guard=HeuristicOnly`（`BrokenAppBootstrap`）× `policy∈{TsfNative, ImmCross}` × `desired_open∈{false,true}` | 許可 | `None` | 旧は実観測（`ImmGetOpenStatus=false`）を無視して force-ON する。新は Step 3（実観測）が Step 4b（ヒューリスティック）より優先される——**安全側の差分** |
| **old-3** | 3 | `observation=BeliefOnlyMedium(true)`（`ConvOpenInference`）単独 × `(TsfNative, desired=false)` / `(ImmCross, desired∈{false,true})` | 許可 | `None` | **BUG-63（「mise」→「くした」）の再現そのもの**。新は authority フィルタで `ConvOpenInference` を actuation の根拠から除外する——安全側 |
| **new-1** | 1 | `observation=BeliefOnlyMedium(false)`・intent / guard なし・`policy=TsfNative`・`desired_open=true` | 不許可 | 許可 | 旧は `ConvOpenInference=false` を採用して false になるが、新は Step 3 でこの観測源を除外した結果 Step 4c（`OwnSsot`）が `desired_open=true` を採る。**今まで force-ON しなかった状況で新たに force-ON し始める唯一のケース** |

old-2 / old-3 の 7 件は「新のほうが厳格」なので、A-2 で強制しても
**force-ON が減る方向**にしか動かない。実質的にリスクを持つのは
**old-1（bootstrap force-ON の消滅）と new-1（新規 force-ON）の 2 件**である。

old-1 は `runtime/mod.rs::try_force_on_bootstrap`（定義 `:871`、実 write は
その中の `apply_ime_open_with_belief`（`:892`）＝§2.A.2(3) の表で
「外部 3」に数えた 1 件）が
`!can_use_imm32_cross_process()` ガードを持たず Standard（LINE / Qt 等）でも
到達する（ADR-089 §9-21）という事実と表裏である。`ImmCross` は
`default_feedback = Read` なので Step 4c（`OwnSsot`）が発火せず、観測も意図も
guard も無い bootstrap 時には `issue_open_warrant()` が `None` を返す。

したがって:

- **A-1（shadow モード、挙動変更なし、Linux で完結）**:
  `ActuationOrder::issue` を全入口に配線するが、`into_actuation()` が `None`
  でも**書き込みは止めない**。代わりに
  `Authorization::LegacyUnwarranted { would_have_blocked: true, origin }` を
  載せてログと journal（ADR-082 `JournalEntry::ImeActuation`）に残す。
  これで「実機で実際にどの入口が何回 warrant を取れないか」が**測れる**ように
  なる。差分オラクルは 240 通りの**組合せ**を測っているが、実機で**どの
  組合せが実際に起きるか**は測っていない。
- **A-2（強制、入口ごとに 1 つずつ、実機ソーク必須）**:
  A-1 のログで `would_have_blocked` がゼロだった入口から順に、
  `into_actuation()` が `None` なら書き込みを中止する形へ倒す。
  `try_force_on_bootstrap` は**最後**に回す（ADR-089 §9-21 が
  「ここで単独に足してはならない」と明記している経路そのもの）。

**設計案 3: `set_ime_open` トレイト経路の扱い。**

`WindowsPlatform` に inherent メソッド
`set_ime_open_ordered(&mut self, order: ActuationOrder) -> bool` を足し、
`ime_refresh.rs:534`/`:752` をそちらへ移す。トレイトメソッド
`set_ime_open` は呼び出し元ゼロになるので、`.apply_ime_open(` と同じく
**死んだ入口として doc に明記し、`ime_open_actuation_entry_points_are_accounted_for`
の期待値を 2 → 0 に下げる**（ガードは残す。ゼロになったことが可視化される）。

**設計案 4: 非同期経路で await をまたいだ warrant の扱い。**

`run_open_chain_async` は `spawn_local` の中で ImmCross の完了を await してから
フォールバックへ進む。`ActuationOrder` を起案時に作って future へ move すると、
**warrant は起案時点の状態に基づくのに、write は完了時点で起きる**。

ここで「warrant を epoch でフェンスする」方向へ行きたくなるが、**行かない**。
理由は責務の分離である:

- **warrant は「その値を書いてよいか」（根拠軸）** を答える。
- **`ActuationTarget` は「どのウィンドウへ書くか」（空間軸、ADR-086 INV-14）** を
  答える。

ImmCross 本体は既に `ActuationTarget::verify_still_current` でフォーカス移動時に
`Aborted` になる。**守られていないのはフォールバック側**（`fallback_write` の
`VerifiedTarget::FocusImplicit`）である。したがって対策は warrant ではなく
**チェーンの再抽選**（項 D）で行う: 完了時点の view から `caps(p, k)` を引き直す
とき、`focus_gen` が起案時と変わっていたら **warrant を再発行する**
（`with_app` の中なので `WarrantContext` を組み直せる）。再発行が `None` なら
チェーンを打ち切る。**項 A と項 D は同じコミットで入れる**（§3）。

#### A.4 実装した場合のリスク

| # | リスク | 評価と緩和 |
|---|---|---|
| A-R1 | **bootstrap force-ON が Standard で丸ごと止まる。** 差分オラクルが「判明した中で最大の挙動変化」と記録している | A-2 を入口ごとに分割し、`try_force_on_bootstrap` を最後に回す。A-1 の shadow ログで実発火頻度を測ってから判断する。**単独で `!can_use_imm32_cross_process()` を足す代替は禁止**（ADR-089 §9-21） |
| A-R2 | **新だけが許可するケース（new_only 1 件）で、今まで force-ON しなかった状況で force-ON し始める。** `ConvOpenInference(false)` を authority フィルタで除外した結果 Step 4c が `desired_open=true` を採る | ADR-087 §8.12 M3 が方向別に件数固定済み（old_only=8 / new_only=1）。A-1 の shadow モードでは**発火しない**（shadow は書き込みを止めないが増やしもしない）ため、A-2 の対象入口を決める段で個別に判断する |
| A-R3 | **`ActuationOrder` の構築点が増えると `WarrantContext` の組み立てが外部 8 入口に散る。** ADR-087 §7 round4 N-A がまさにこれを避けるため `WarrantContext` を導入した | `ImeStateHub` に `fn warrant_context(&self, now, now_ms) -> WarrantContext<'_>` を 1 本だけ生やし、8 入口はそれを呼ぶ。`intent_store` の private を維持したまま読み手を 1 本に絞れる。`architecture_guard` で `WarrantContext {` のリテラル構築が本番に無いことを固定する（**INV-48**） |
| A-R4 | **`target: HwndId` が取れない入口がある。** `IntentStore::lookup` は対象一致を要求するが、`ImeModel::current_focus()` は `Option<HwndId>` で `None` がありうる | `None` のときは Step 1 が必ず外れるだけで、Step 0/3/4a/4b/4c は評価される（`target` はそれ以外の Step で使われない）。したがって sentinel `HwndId(0)` を渡す設計で**判定は変わらない**。ただし「対象不明」がログで区別できるよう `origin` 側に持たせる |
| A-R5 | **`Authorization::LegacyUnwarranted` に payload を足すと Phase B の compile-fail doctest が壊れうる** | doctest（ケース1/3/4）は `Actuation<Warranted>` から `run_chain` を呼ぶ形などを固定しており `Authorization` の内部形には触れていない。ただし A-1 で variant を変えるときは「通る双子」も同時に確認すること（ADR-089 §9-14 の規約） |
| A-R6 | **`.claude/rules/fix-requires-evidence.md` の「キー選択」ファミリーに該当する。** A-2 は実際に送る VK の有無を変える | A-2 の各ステップに `docs/known-bugs.md` 追記か golden 更新を必ず添える。revert 時は `.claude/rules/experiment-logging.md` の 3 点（アプリ / IME / 再現手順）を書く |

#### A.5 優先度と規模感

- **A-1（shadow 配線）**: 優先度 **中〜高**。規模 **中**（`Phase B` クラスより
  小さいが、外部 8 入口 + 5 関数シグネチャに触るため 1 セッション相当）。
  **Linux で完結し挙動変更ゼロ**。
- **A-2（強制）**: 優先度 **高（価値）／低（着手可能性）**。規模 **大**
  （入口ごとに分割し、それぞれ実機ソーク）。**A-1 のログが取れるまで着手不可**。

---

### B. `ConvergedReceipt` を制御フローへ配線し、`ImeObservation` を返す読み口を塞ぐ

#### B.1 現状の問題

`ConvergedReceipt`（`state/ime_actuation.rs`）は
`runtime/ime_refresh.rs::ir_apply_drift_correction` の 2 箇所——
`Blind` の give-up 到達時（`:656`）と `Read` の収束確認時（`:707`）——で
構築されるが、**値は `log::debug!` にしか渡らない**。実際の判定は:

```rust
// Read 収束（:700）
let confirmed = ...observations
    .most_recent_trusted_after(now, act_sent_at)
    .is_some_and(|o| o.open == desired);

// Blind give-up からの復旧（:678）
let fresh = ...observations
    .most_recent_trusted_after(now, gave_up_at)
    .is_some();
```

が担っている。**receipt を削除しても制御フローは変わらない**（ADR-089 §9-16）。

**より重要な点として、ADR-089 §9-16 の「Phase C で配線する形」の記述だけでは
保証は生まれない。** `most_recent_trusted_after` は `pub` のままなので、
`read_back()` を足しても**元の `Option<&ImeObservation>` を返す口が残る**。
「読み戻しの産物は `ImeObservation` として手に入らない」（ADR-080 不変条件6）は
**その口を塞いで初めて**成立する。

#### B.2 具体的な設計案

**設計案 1: `Resolution` を 2 値から 4 値へ広げ、`ConvergedReceipt` に実際の帰結を持たせる。**

現行 `Resolution`（`state/ime_actuation.rs:35-38`）は
`Confirmed` / `GaveUp` の **2 値**で、`ConvergedReceipt` は
`converged: bool` しか持たない。`resolution()` は `converged` から
`Confirmed`/`GaveUp` を再構成する**非可逆な実装**である。読み戻しには
「give-up 後に外界が動いた」「まだ収束していない（再送する）」という
第 3・第 4 の帰結が必要なので:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// 読み戻しが desired と一致した（Read）。
    Confirmed,
    /// max_attempts 到達で打ち切った（Blind）。
    GaveUp,
    /// give-up 後に「値は不問の」新しい観測が来た（＝外界が動いた）。
    ExternalChange,
    /// まだ収束していない（再送する）。
    Pending,
}

pub struct ConvergedReceipt { resolution: Resolution, attempts: u32 }
```

`converged()` は `matches!(self.resolution, Resolution::Confirmed)` のまま
（既存呼び出し元と互換）。

**設計案 2: 読み戻しの唯一の窓口を `ObservationStore::read_back` にする。**

```rust
/// 読み戻しの問い（ADR-080 不変条件6 / ADR-089 INV-46）。
pub enum ReadBackQuery {
    /// `since` 以降の trusted 観測が `desired` と一致したか（Read の収束確認）。
    Converged { desired: bool },
    /// `since` 以降に trusted 観測が**何であれ**記録されたか
    /// （Blind give-up からの復旧判定。値ではなく鮮度だけを見る）。
    AnyFreshEvidence,
}

impl ObservationStore {
    /// 読み戻しの唯一の公開窓口。**戻り値は `ImeObservation` ではない**——
    /// したがって読み戻しの産物を観測として書き戻すことが型として書けない。
    pub fn read_back(
        &self, now: Instant, since: Instant, query: ReadBackQuery, attempts: u32,
    ) -> ConvergedReceipt { .. }

    /// **`pub` から `fn`（モジュール private）へ縮小する。**
    fn most_recent_trusted_after(&self, ..) -> Option<&ImeObservation> { .. }
}
```

`most_recent_trusted`（`_after` の無いほう）は `pub` のまま残す——
こちらは belief のフォールバック（`ime_model.rs:329`、`platform_state.rs:556`）で
使う**別の用途**であり、「actuation の読み戻し」ではない。**この 2 つを
混同しないことが本設計の要点**である（混同すると、belief の読み取りまで
receipt 越しになって ADR-078 の 3 層分離が壊れる）。

**設計案 3: `ir_apply_drift_correction` を receipt だけ見る形へ書き換える。**

```rust
// Read 収束
let receipt = obs.read_back(now, act_sent_at, ReadBackQuery::Converged { desired }, act_attempts);
if receipt.converged() { self.discard_actuation(); return; }

// Blind give-up 後の復旧
let receipt = obs.read_back(now, gave_up_at, ReadBackQuery::AnyFreshEvidence, act_attempts);
if receipt.resolution() == Resolution::ExternalChange { self.discard_actuation(); }
```

**挙動は bit-identical にできる**（述語をそのまま `read_back` の中へ移すだけ）。
移行前後の同値性は `state/observation_store.rs` の Linux ユニットテストで
全数固定する（`since` の前後 × `desired` の一致/不一致 × confidence 3 値）。

**設計案 4: `ConvergedReceipt::new` の可視性。**

`new(resolution, attempts)` は現在 `pub const fn` で誰でも作れる。
**これは塞がなくてよい**——receipt を偽造しても `AnyObservation` には変換
できない（INV-46 はそこを守っている）ので害が無く、テストが receipt を
組み立てられるほうが有用である。塞ぐべきは「観測を直接手に入れる口」
（設計案 2）のほうであり、**そこを取り違えると作業だけ増えて保証は増えない**。

#### B.3 実装した場合のリスク

| # | リスク | 評価と緩和 |
|---|---|---|
| B-R1 | **BUG-33 / BUG-43 ファミリーの中心（drift correction）に触る。** 述語をずらすと「give-up が即座に無効化される」「無限再送」のどちらかに落ちる | 述語を移すだけで**書き換えない**。ADR-080 が「復旧判定は値ではなく鮮度で行う」理由を `ime_refresh.rs:640` のコメントが説明しており、`ReadBackQuery` の 2 variant はその区別をそのまま型にしたもの。全数ユニットテストで移行前後の同値を固定する |
| B-R2 | **`most_recent_trusted_after` を private にすると、将来の正当な用途まで塞ぐ** | 現在の本番呼び出し元は 2 箇所（どちらも本作業で `read_back` に移る）。新しい用途が出たら `ReadBackQuery` に variant を足すのが正しい形であり、それが「読み戻しの意味を宣言させる」という本設計の狙いである |
| B-R3 | **`drift_correction_giveup_and_confirmed_do_not_write_observations`（テキスト検査）を削除したくなる** | **削除しない。** ADR-089 §9-16 が明記するとおり、型が入っても「型を通る経路に全呼び出し元を移してから削除する」。本作業では 2 箇所とも移るが、`read_back` の外で観測を書く新経路が増えないことはテキスト検査でしか見えない |
| B-R4 | **`Resolution` を 2→4 値にすると `resolution()` の既存呼び出し元が非網羅 match になる** | 現在 `resolution()` の呼び出し元は本番ゼロ（`converged()` / `attempts()` のみ使用）。`#[non_exhaustive]` は付けない（crate 内で網羅させたいため） |

#### B.4 優先度と規模感

優先度 **高**。規模 **小〜中**（1 ファイルの新設なし、
`observation_store.rs` + `ime_actuation.rs` + `ime_refresh.rs` の 3 ファイル）。
**Linux で完結し、挙動を bit-identical に保てる。**
ADR-089 が入れた型のうち「効いていない」ものを最も安く「効く」に変えられる。

---

### C. 観測ストアの裏口を可視性で塞ぎ、閉じられない witness の理由を確定する

#### C.1 現状の問題

ADR-089 §9-11 は witness の偽造難度を 3 段に分類した:

| 段 | witness | 現状 |
|---|---|---|
| 偽造不能 | `AcceptedObservation` | `probe_admission.rs` でしか構築できない。`for_sync(epoch)` は **Phase A で `pub(crate)` へ縮小済み**（`:121`） |
| 偽造容易 | `ImePolicyProfile` / `ConvSyncReason` | 普通の public enum。「起点を宣言させる」効果しかない |
| 裏口 | `ObservationStore::per_source` / `PerSourceObservations` の各フィールド / `ImeObservation` の各フィールド | **`pub` のまま** |

Phase B で `PerSourceObservations::set` は `pub(crate)` へ縮小され、
crate 外からの注入は塞がった。**しかし `ImeObservation` の全フィールドと
`PerSourceObservations` の全フィールド、`ObservationStore::per_source` が
`pub` のままなので、crate 外から**

```rust
store.per_source.observer_poll = Some(ImeObservation { source: ..., .. });
```

**とフィールドへ直接代入すれば、`set` を通らずに観測を注入できる。**
Phase B は `set` だけを塞いだが、`set` は「フィールド代入の便利メソッド」で
あって唯一の入口ではなかった。

#### C.2 実コードで確定させた「閉じるコスト」

ADR-089 §9-11 は「`tests/golden_scenarios.rs` が読んでいるため `pub` のまま」と
書いているが、実際の依存は次の 2 行だけである:

```
tests/golden_scenarios.rs:332  model.observations.per_source.observer_poll.map(|o| o.open)
tests/golden_scenarios.rs:375  model.observations.per_source.gji.is_none()
```

`src/` 配下の `per_source` 直接アクセス 4 箇所
（`ime_model.rs:733`/`:783`、`platform_state.rs:1519`、`open_warrant.rs:258`）は
**すべて `#[cfg(test)]` の中**である。したがって:

- **読み取りは公開アクセサ 1 本で代替でき、**
- **書き込みは crate 外から構造的に不可能にできる。**

#### C.3 具体的な設計案

**設計案 1: `ObservationStore` に読み取り専用アクセサを足し、`per_source` を
`pub(crate)` へ縮小する。**

```rust
impl ObservationStore {
    /// 指定ソースの最新観測（読み取り専用）。
    #[must_use]
    pub const fn observation(&self, source: ObservationSource) -> Option<&ImeObservation> {
        self.per_source.get(source)
    }
}
```

`PerSourceObservations::get` は既に `pub const fn` なので委譲するだけ。
`tests/golden_scenarios.rs` の 2 行を
`model.observations.observation(ObservationSource::ObserverPoll)` 等へ書き換える。
その上で `pub per_source` → `pub(crate) per_source`、
`PerSourceObservations` の 9 フィールド → `pub(crate)`。

**設計案 2: `ImeObservation` に `#[non_exhaustive]` を付ける。**

`#[non_exhaustive]` は **crate 外からの構造体リテラル構築と網羅的分配束縛を
禁止し、フィールドの読み取りは許す**。つまり

- `tests/golden_scenarios.rs` の `o.open` は**そのまま動く**、
- crate 外の `ImeObservation { .. }` は**コンパイルエラーになる**、
- crate 内は影響なし（`record_any` の 1 箇所と `#[cfg(test)]` の
  `open_warrant.rs:267` が構築する）。

**フィールドを private にする必要が無い**のがこの案の要点である
（private 化すると読み取り側まで壊れ、アクセサを 7 本生やす羽目になる）。

**設計案 3: crate 内の残余は件数ガードで固定する。**

crate 内では依然として `per_source` へ書けるが、その入口は
`PerSourceObservations::set`（`pub(crate)`、本番呼び出し元 1 = `record_any`）
だけになる。既存の `per_source_set_is_confined_to_the_store` に加え、
`per_source` の**フィールド直接代入**が本番コードに無いことを固定する
ガードを 1 件足す（**INV-49**）。

**設計案 4: 閉じられない witness の理由を確定させ、テキスト検査の削除条件を
明文化する。**

以下は**塞がない**と決める。それぞれ理由と、塞ぐ場合のコストを記録する:

| witness | 塞がない理由 | 塞ぐ場合の設計（採らない） | 代わりに残す防御 |
|---|---|---|---|
| `ImePolicyProfile`（`Observed::<HeuristicDefault>::at_startup` の witness） | フォーカス分類の結果として `caps` / `AppImePolicy` / journal / 設定 UI まで広く流通する**値**であり、発行権を絞ると影響範囲が型化と無関係に広がる | `AppImeProfile::classify` だけが発行できる `ProfileWitness` newtype で包む。`caps(p, k)` の引数型まで変わるため Phase C 相当の差分になる | `heuristic_default_observation_is_limited_to_designated_methods`（ADR-089 §7、期待値維持） |
| `ConvSyncReason`（`Observed::<ConvOpenInference>::from_conv` の witness） | 同上（conv 分類の結果値であり、journal・ログ・設定へ流れる） | `classify_conv_transition` の戻り値からしか取り出せない形にする。ADR-084/086 の conv 単一所有権と同じ段で判断すべき | `conv_open_inference_source_is_limited_to_report_and_gate`（期待値 1） |
| `AcceptedObservation::for_sync(epoch)` | `pub(crate)` まで縮小済み。さらに絞るには「probe を実際に実行した」ことを表す token が要るが、その発行元 `output/probe_io.rs` は windows-gated で、ungated な `state/` から型で要求できない | `ProbePerformed` token を `output/probe_io.rs` で発行し `for_sync` が要求する。**ADR-065（`state/` は windows 型に依存しない）と衝突する**ため、token 型自体を ungated にする迂回が要る | `focus_probe_observation_is_limited_to_real_probe_path`（ADR-089 §7、維持） |
| `UserIntentSource::Command`（`write_set_open_request`） | engine 内部判断であり、引数の型で起点を限定できる外部事実が無い（ADR-089 §9-8） | (a) `EngineCommandWitness`（`generation` / `ImeApplyRequested` の発行権と束ねる）、(b) `write_set_open_request` を `pub(in crate::state)` へ | `user_intent_source_construction_is_limited_to_typed_writers`（期待値 1、**ゼロにしない**） |

**この表を書き残すこと自体が本項の成果物の半分である。** ADR-089 §9-11 は
「絞れない」と書いたが「なぜ絞れないか」「絞るなら何が要るか」を残して
いなかったため、次のセッションが同じ調査をやり直すことになる。

#### C.4 実装した場合のリスク

| # | リスク | 評価と緩和 |
|---|---|---|
| C-R1 | **`#[non_exhaustive]` が `ImeObservation` の `Copy`/`PartialEq` derive と干渉する** | 干渉しない（`non_exhaustive` は derive を妨げない）。crate 内では網羅的分配束縛も許されるため既存コードは無影響 |
| C-R2 | **`per_source` を `pub(crate)` にすると将来の統合テストが観測を組み立てられない** | それが目的である。統合テストが観測を注入したい場合は `record_replayed(AnyObservation::restored_from_journal(..))` を使う——**それが journal リプレイの正規経路**であり、`any_observation_replay_door_is_not_used_in_production` が本番からの使用だけを禁じている |
| C-R3 | **`ObservationStore::drift` / `current_focus_epoch` も `pub` のままである** | 本項のスコープ外。`drift` は `update_drift` が唯一の書き手で、値としての意味も観測ではない。範囲を広げると「型化と無関係な差分」が増える（ADR-089 §9-11 が Phase A で見送った理由と同じ）ため、必要になったときに別途 |
| C-R4 | **挙動変更ゼロなので実機検証の対象にならず、逆に「入れたつもり」になりやすい** | 新設ガード（INV-49）と `#[non_exhaustive]` の compile_fail doctest（「通る双子」併記、ADR-089 §9-14 の規約）で機械的に固定する |

#### C.5 優先度と規模感

優先度 **高**（最も安く、最も確実に効く）。規模 **小**
（`observation_store.rs` + `tests/golden_scenarios.rs` 2 行 +
ガード 1 件 + ADR の表）。**Linux で完結、挙動変更ゼロ、実機不要。**

---

### D. 非同期チェーンを `caps` へ寄せる（完了時点で引き直す形）

#### D.1 現状の問題

`runtime/open_chain.rs::run_open_chain_async` は chain に
`WriteMechanism::ALL` を渡したままである（ADR-089 §6 Phase C 実施記録 C-4、
§9-20）。理由は「ImmCross の await をまたいでフォーカス（したがって profile と
K）が動きうるため、起案時点の `caps(p, k).chain` を固定すると完了時点で
適用可能な機構を取りこぼす」——`ImeKindId` が推測値である以上（INV-45）、
await をまたいで K を固定するのは P20 が禁じる形のゲートになる。

結果として **INV-44（capability は const 表 1 箇所）は、同期経路では型どおり
成立し、非同期経路では「`ALL` は全 `caps` チェーンの和集合であり
`is_applicable` + `falls_through` で絞れば同値」という論証に依存している**。
その同値は `caps_chain_matches_legacy_all_scan`（windows-gated、CI の
`windows-build` ジョブで実行）が固定するが、**それは「(p, k) が変わらない
場合」の同値**である。

#### D.2 具体的な設計案

ADR-089 §9-20 が挙げた恒久策 (a)（chain を各ステップで引き直す）を採る。
ただし §9-20 は `FnMut() -> &'static [WriteMechanism]` を渡す形を示唆して
いたが、実コードを読むと**もっと単純な形になる**:

`fallback_write`（`open_chain.rs:203`）は既に

```rust
crate::with_app(|app| {
    let view = app.shadow_ime_control_view();   // ← 完了時点の view を作り直している
    if crate::ime_controller::mechanism_is_applicable(mechanism, &view) { .. }
})
```

と**完了時点の view を毎回作り直している**。したがって chain を引き直す材料は
既に手元にある。設計は:

```rust
pub(crate) async fn run_open_chain_async(order: ActuationOrder, imm: ImmCrossOp) -> ImeOpenOutcome {
    // 1) ImmCross を試す（起案時点の caps chain の先頭であることは呼び出し側が保証）
    // 2) Failed なら with_app の中で完了時点の view を取り、
    //    - caps(p_now, k_now).chain を引き直す
    //    - すでに試した機構（attempted: ArrayVec<WriteMechanism, 4>）を除く
    //    - focus_gen が起案時と変わっていたら warrant を再発行（項 A の設計案 4）
    // 3) 残りを順に走らせる
}
```

**この設計が「取りこぼし」を生まないことは、実コードから示せる。**
今日の `ALL` 走査と再抽選が同値になるのは次の理由による:

`WriteMechanism::ALL`（`state/actuation_chain.rs:148`）は
`[ImmCross, GjiDirect, MsImeDirect, KanjiToggle]` の 4 要素で、
`caps` の全チェーンはこの順序の部分列である。以下、profile は
`caps` の引数型に合わせて `ImePolicyProfile`（`ImmCross` /
`Imm32Unavailable` / `TsfNative` / `Plain` / `Unknown`）で書く
（`AppImeProfile::Standard` は `ImePolicyProfile::ImmCross` に写る）:

| 場面 | 今日（`ALL` + 完了時点の `is_applicable`） | 再抽選（完了時点の `caps(p, k)`） |
|---|---|---|
| (p, k) が変わらない | `caps_chain_matches_legacy_all_scan` が同値を固定 | 同じ |
| await 中に `(ImmCross, MsIme)` → `(TsfNative, MsIme)` へ動いた | `ALL` を走査し `MsImeDirect.is_applicable` が真になるので `MsImeDirect` | `caps(TsfNative, MsIme).chain = [MsImeDirect]` → `MsImeDirect` |
| await 中に `(ImmCross, MsIme)` → `(Imm32Unavailable, Gji)` へ動いた | `GjiDirect.is_applicable`（`gji_monitor_ok`）が真なので `GjiDirect` | `caps(Imm32Unavailable, Gji).chain = [GjiDirect]` → `GjiDirect` |

**すなわち再抽選は「起案時に固定する」案とは違い、`ALL` 走査と同じ
『完了時点の状態で残りを選ぶ』意味論を保つ。** さらに P20 の観点では
**再抽選のほうが `ALL` 走査より安全側である**——K が推測値であることを
理由に固定を禁じているのだから、**固定しないのが正解**であり、
`ALL` は「固定しない」を「全部試す」で代用していた。

唯一の差は **ADR-089 §4.9**（`caps` の GJI/MsImeDirect 行の末尾に
`KanjiToggle` を置かない、r3 の誤りの却下記録）に由来する
（**本 ADR の §4.9 ではない**——そちらは `try_force_on_bootstrap` の話）。
`ALL` なら `GjiDirect` が `Failed` を返したとき `KanjiToggle` へ落ちるが、
`caps` chain では落ちない。**`WriteMechanism::may_return_failed()`
（`actuation_chain.rs:182`）が `ImmCross` のみ真である**（Phase C で新設）ため
現状は差が出ない。この前提が崩れたときに壊れることを、再抽選側のテストが
`may_return_failed` を参照する形で明示する（**INV-50**）。

#### D.3 実装した場合のリスク

| # | リスク | 評価と緩和 |
|---|---|---|
| D-R1 | **無限ループ**（再抽選の先頭がまた `ImmCross` になる） | `attempted` 集合を持ち、既に試した機構を除く。chain 長は高々 4 なので `ArrayVec<_, 4>` で足りる。「同じ機構を 2 回 write しない」は Linux ユニットテストで全数固定できる |
| D-R2 | **`GjiDirectStrategy` / `MsImeDirectStrategy` が将来 `Failed` を返すようになると前提が崩れる** | ADR-089 §9-13 が同じ前提に依存している（`fallback_write` が機構ごとに view を作り直しても実害が無い根拠）。`may_return_failed()` を参照する形にして、変更時にコンパイル/テストが落ちるようにする（INV-50） |
| D-R3 | **`with_app` の中で `WarrantContext` を組み立てると再入する** | `with_app` は `RefCell` ベースの再入検出を持つ（[[project_in_with_app_removal]] の経緯）。`fallback_write` は既に `with_app` の中で `shadow_ime_control_view()` を呼んでおり、`WarrantContext` の組み立ても `&self` 読み取りのみ。**ただし `IntentStore` は `ImeStateHub` の private フィールドなので、`&self` アクセサを 1 本足す必要がある**（`ImeStateHub::warrant_context`、項 A の A-R3 と同じもの） |
| D-R4 | **実機ソーク項目 17-b / 17-g に直接効く。** ImmCross → KanjiToggle のフォールスルーと async/sync 分岐の不変性 | ADR-089 §9-17 の 17-b / 17-g をそのままソーク項目として引き継ぐ。**この作業は `.claude/rules/fix-requires-evidence.md` の「キー選択」ファミリー**なので、golden 更新か known-bugs 追記を必ず添える |
| D-R5 | **`ImmCrossOp::Untargeted`（`key_pipeline.rs` の shadow-toggle OFF 経路）は依然 `FocusImplicit` のまま** | §9-19 が「残る `FocusImplicit` の本当の未移行分は 1 件だけ」と特定している。**本項では扱わない**（`Targeted` へ寄せると挙動が変わり、単独で実機ソークが要る）。D の再抽選で `focus_gen` の変化を見るようになれば、その 1 件が抱えるリスク（完了後にフォーカスが動いていても書く）は実質的に緩和される |

#### D.4 優先度と規模感

優先度 **中**。規模 **中**（`open_chain.rs` + `actuation_chain.rs` の
`run_chain_async` シグネチャ変更 + Linux ユニットテスト）。
**実機ソーク必須**（17-b / 17-g）。項 A と同じコミット群で入れる（§2.A 設計案 4）。

---

### E. dylint 2 crate は恒久的に実行時 lint のまま残す

#### E.1 現状の問題

ADR-089 の r2〜r5 は **4 ラウンド連続で**「`lints/ime_event_guard` と
`lints/observation_source_guard` は Phase A の型化で置き換え可能」と書き、
Phase A の実装時に実コード照合で誤りと判明した（ADR-089 §7 の訂正）。
2 crate が見ているものは Phase A の型化範囲（open 軸の観測プール +
`Observed<E>` の witness）と**重ならない**:

| dylint crate | 見ているもの | Phase A との関係 |
|---|---|---|
| `observation_source_guard` | `ImeEvent::InputModeObserved { source: .. }` の source 偽装（`ImmGetOpenStatus` を名乗る／`ConvBitsInference` を designated 関数外で使う）。すなわち **input_mode 軸** | 無関係。Phase A が型化したのは `ObserverReported`（**open 軸**） |
| `ime_event_guard` | `ImeEvent::PanicReset` / `HwndCacheRestored` / `EngineActivationSync` の designated 関数外での構築 | 無関係。この 3 variant は**観測でも意図でもない**（`desired_open` の直接書き込み口）ため `Observed<E>` にも `IntentWitness` にも載らない |

**「置き換えない」までは確定したが、「恒久的にどうするのか」は決まっていない。**
決めないままだと 5 回目の同じ提案が出る。

#### E.2 具体的な設計案（＝決定）

**決定 E-1: `ime_event_guard` は恒久的に dylint のまま残す。型化しない。**

理由は「型化できない」ではなく「**型化しても保証が上がらない**」である:

`PanicReset` / `HwndCacheRestored` / `EngineActivationSync` は
**外部事実に対応しない直接書き込み口**である（それが escape hatch である所以）。
`Observed<E>` の witness が成立するのは「probe を実行した」「物理キーが来た」
といった**引数として渡せる外部事実**があるからで、この 3 variant にはそれが無い。

型化するなら「designated 関数の中でしか作れないトークン」を要求する形になるが、
**そのトークンは crate 内では `pub` にならざるを得ず**（`ImeEvent` の構築点と
reduce 側が別モジュール）、結局「designated 関数の中で作られていること」は
件数ガードでしか担保できない。これは ADR-089 §9-15 が
`ime_controller::apply_mechanism` について記録した「可視性の縮小でも
チェーン経由への書き換えでも解けない」のと**同型の袋小路**である。

**dylint がテキスト検査より強い点を、実装に即して正確に書く。**
起票時の草稿は「`let src = ObservationSource::X; ... source: src` のような
間接構築まで検出できる」と書いていたが、**これは実装で成立しない**——
`lints/observation_source_guard/src/lib.rs:196-202` の `path_expr_ident` は
`ExprKind::Path` の**最終セグメント**しか見ないので、`source: src` は
`"src"` として `allowed_fns_for("src") == None` になり、そのまま素通りする
（変数を経由すると検出できない。variant path を直接書く
`ime_event_guard` 側はこの制約に当たらない）。実際に dylint が
テキスト検査に勝っている点は次の 3 つである:

1. **crate 全体を走査する。** `architecture_guard.rs` の同種検査
   （`panic_reset_event_is_limited_to_apply_panic_reset`（`:260` 付近）/
   `hwnd_cache_restored_event_is_limited_to_apply_hwnd_cache_restore`（`:280` 付近）/
   `input_mode_observed_construction_sites_are_accounted_for`（`:300` 付近））は
   **ファイル名と件数のペアを直書き**しており、新しいファイルで構築されると
   気付けない。dylint は HIR 全体を見るのでファイル一覧の保守が要らない。
2. **型で variant を解決する。** テキスト検査は文字列 `"ImeEvent::PanicReset {"`
   の出現数を数えるだけなので、同名 variant を持つ別型や、コメント／
   マクロ展開の差で誤検出・見逃しが起きうる。dylint は
   `is_ime_event()` で `typeck` 結果の ADT が `ime_event::ImeEvent` である
   ことを確認してから判定する。
3. **`EngineActivationSync` は dylint 単独防御である。**
   `RESTRICTED_VARIANTS`（`lints/ime_event_guard/src/lib.rs:73`）の 3 variant の
   うち `PanicReset` / `HwndCacheRestored` には `architecture_guard` の
   等価なテキスト検査があるが、`EngineActivationSync`（BUG-48）には無い。
   **dylint を降ろすなら、この 1 件はテキスト検査を新設してからでなければ
   防御がゼロになる。**

「型より安い」という評価は変わらない（下記 E-3 の保守義務込みで、なお安い）。

**決定 E-2: `observation_source_guard` も当面 dylint のまま残す。ただし
「降ろせる条件」を明記する。**

input_mode 軸の型化は**ありうる**が、それは本 ADR の成果ではなく
**ADR-088 トラック A（`AxisCapability` + `CharsetOwner`、4 軸への一般化）の
成果**になる。ADR-089 §7 が既に

> 降ろせる dylint があるとすれば、それは `InputModeObserved` / `PanicReset` 系を
> 型化する**別の ADR** の成果になる

と書いており、本 ADR はその「別の ADR」を **ADR-088 トラック A に特定する**。
降ろせる条件は次の 3 つが同時に満たされたときに限る:

1. `InputModeState` の観測に `Observed<E>` 相当（source と confidence が
   evidence 型から決まる形）が入り、
2. `ImeEvent::InputModeObserved` の本番構築点がその witness 経由だけになり、
3. `ConvBitsInference` / `GjiIoInference` の 2 ソースが evidence 型として
   表現される（現在は `PerSourceObservations` にフィールドを持たない、
   ADR-089 §1.3(h)）。

**この 3 条件を満たさずに dylint を降ろしてはならない**（**INV-51**）。

**決定 E-3: 「keep」の判断には保守義務が伴うことを明記する。**

dylint は安くない。`.github/workflows/ci.yml:84` の `dylint` ジョブは
**nightly を `nightly-2026-05-22` にピン留めし**、`cargo-dylint` 6.0.0 を
インストールして走る。ピン留めした nightly はいずれ壊れる。

**CI 時間の内訳は「~17 分」ではない**（起票時の草稿はこの数字を現在値として
引いていたが誤り）。`ci.yml:98-104` のコメントが記録しているとおり、
~17 分という実測（GitHub Actions run 30987463824）は **`~/.cache/cargo-xwin`
を rust-cache 対象に加える前**の値で、そのうち ~15 分は Windows SDK/MSVC CRT
の再ダウンロードだった。キャッシュ導入後にその分は消え、**lint 本体の
型検査は ~1.5 分**である（この数字はコメント由来でキャッシュの有無に
依存しない）。ジョブ全体の現在値は本 ADR では実測していないので引かない。
**保守コストの本体はランタイムではなく、`rustc_private` API 追従である。**

そのときの**取るべき行動を先に決めておく**:

1. まず nightly のピンを上げて追従する（`rustc_private` の API 変更に
   追随するコストは 3 crate 分）。
2. それが現実的でなくなったら、**`architecture_guard.rs` のテキスト検査へ
   降格する**（`lints/` を削除して「守らなくてよい」にはしない）。
   降格時は**検出力が落ちることを ADR に記録する**——具体的には
   E-1 の 3 点（crate 全体走査 / 型による variant 解決 /
   `EngineActivationSync` の単独防御）を失う。特に 3 番目は
   **降格と同時にテキスト検査を新設しなければ防御がゼロになる**ので、
   降格 PR の必須項目とする。
3. **「dylint が壊れたから規律をやめる」は選択肢に入れない。**

#### E.3 実装した場合のリスク

| # | リスク | 評価と緩和 |
|---|---|---|
| E-R1 | **「恒久的に残す」と書いたことで、将来 input_mode 軸を型化する動機が下がる** | E-2 で降ろせる条件を 3 つ明記した。型化の動機は「dylint を消せるから」ではなく「input_mode の source 偽装を型で防ぐため」であり、そちらは ADR-088 が持つ |
| E-R2 | **ピン留め nightly の陳腐化が静かに進む** | E-3 で行動を先に決めた。ピンを上げるコミットは 3 crate 同時になる（`no_vk_as_scan` も同じ toolchain） |
| E-R3 | **本項はコード変更ゼロなので「やった」判定が曖昧になる** | 成果物は (a) 本 ADR の本節、(b) `.claude/rules/ime-belief-architecture.md` の「段2（dylint）」節への追記（降ろせる条件 3 つと降格手順）。この 2 つで完了とする |

#### E.4 優先度と規模感

優先度 **中**（緊急ではないが、決めておかないと 5 回目の再提案が出る）。
規模 **極小**（ドキュメントのみ、プロダクションコード 0 行）。

---

### F. ADR-081 Phase 1d/1e を凍結する

#### F.1 現状の問題

ADR-089 §6 は Phase 1d の凍結を**提案**したが、§9-4 のとおり
「凍結の判断は未決定のまま」である。ADR-081 側のステータスも
「Phase 1d/1e の位置づけ: ADR-089 §6 は凍結を提案しているが、その採否は未決定」
と書かれている。**この宙吊りは、`caps` と `ImeProfileDriver` の
二重定義期間を解消期限なしで延長している。**

#### F.2 `caps(p, k)` と `ImeProfileDriver` の実際の重なりを測る

実コードで両者の担当を突き合わせた結果:

| ADR-081 `ImeProfileDriver` のメソッド | ADR-089 `caps(p, k)` が持つか | 判定 |
|---|---|---|
| `default_feedback() -> FeedbackPolicy` | **持つ**（`Caps.feedback`） | **重複** |
| `focus_settle_ms() -> u64` | **持つ**（`Caps.focus_settle_ms`） | **重複** |
| `ime_open_mechanism(open) -> ImeOpenMechanism` | **持つ**（`Caps.chain`。しかも `caps` は K 軸も含む分だけ細かい） | **重複（`caps` が上位互換）** |
| `probe_budget_ms(is_confirm_key, long_idle) -> u64` | 持たない | **未配線のまま。`is_confirm_key` 軸は実装内で未使用、`ColdReason` 軸の精緻化は `.claude/rules/tuning-constants.md` の実測義務で着手不能** |
| `owns_physical_kanji() -> bool` | 持たない（ADR-089 §2.5 が「`caps` に入れない」と明示的に決定、BUG-46） | **`caps` の対象外・重複なし** |
| `has_ime_on_path() -> bool` | 持たない | **`caps` の対象外・重複なし**（contract test 不変条件1） |
| `stale_eisu_recovery_paired() -> bool` | 持たない | 同上 |

**7 メソッド中 3 つが `caps` と完全に重複し、1 つは着手不能、3 つは対象外**である。
さらに ADR-089 Phase C は `AppImePolicy` を `caps` の**薄いファサード**へ
退化させた（C-2）ため、既存の parity テスト
（`imm32_unavailable_driver_matches_app_ime_policy` 等）は
**推移的に driver ↔ caps の一致を固定している**——重複が 3 本の SSOT に
なっていることが実際に機械検査で見えている状態である（ADR-089 §2.5 の警告どおり）。

**加えて、ADR-081 Phase 1d/1e が達成しようとしていた成果の一部は、
すでに ADR-089 Phase C が別の手段で達成している。ただし「一部」であり、
達成していない部分を先に確定させる:**

- ADR-081 のコンテキスト 4「`ImeOpenStrategy` の固定フォールバックチェーンに
  プロファイルごとの所有権が無い」は、**`caps(p, k).chain` が
  (profile, IME 種別) ごとにチェーンを宣言する**ことで解消した。
- ADR-081「不変条件（Phase 1 着手時にテスト/型で強制する候補）」の 1 つ目
  （`docs/adr/081-...md:182-185`）は「コアループのソースに
  `ImeActuatorKind::` **や** `AppImeProfile::` へのパターンマッチが出現しない
  ことを `architecture_guard.rs` 相当のテキスト走査で固定する」だった。
  **これは半分しか達成していない。**

  | 半分 | 現状 |
  |---|---|
  | `ImeActuatorKind::` | **達成**。型ごと廃止された（Phase C item 11、`state/app_ime_policy.rs` から削除。`crates/` `lints/` の全 `.rs` を grep してヒットゼロ、ADR-081 本文の記述のみが残る） |
  | `AppImeProfile::` | **未達成**。非テストコードに variant へのパターンマッチ／直接比較が **7 サイト**残る（下表）。**構築（`from_class_name` / 文字列→variant / 構造体リテラル）は数えない**——不変条件が禁じているのは分岐であって値の生成ではないため（`ime.rs:921`、`focus/current.rs:25`/`:34`、`runtime/mod.rs:1140`、`ime_controller.rs:525-527` は該当しない） |
  | テキスト走査ガード | **未達成**。`tests/architecture_guard.rs` に `AppImeProfile` を対象にした検査は存在しない |

  **残存 7 サイトの内訳**（`958e21c2` 時点。件数ガードを後から書くときの
  期待値の根拠になるので、サイト数と variant 出現数の両方を明示する）:

  | # | 位置 | 形 | variant 出現 |
  |---|---|---|---|
  | 1 | `runtime/key_pipeline.rs:1981-1984` | `matches!(.., Standard)` | 1 |
  | 2 | `runtime/focus_tracking.rs:255-258` | `matches!(.., Imm32Unavailable)` | 1 |
  | 3 | `runtime/focus_tracking.rs:382-385` | `matches!(.., Standard)` | 1 |
  | 4 | `focus/class_names.rs:76` | `profile == TsfNative` | 1 |
  | 5 | `focus/class_names.rs:87` | `profile == Imm32Unavailable` | 1 |
  | 6 | `focus/class_names.rs:199-201` | `From<AppImeProfile> for ImePolicyProfile` の match | 3 |
  | 7 | `focus/tracker.rs:149-154` | `apply_learned_imm_capability` の match（`:151` の `Imm32Unavailable` は構築側なので数えない） | 1 |
  | | **合計** | **7 サイト** | **9** |

  `state/key_sequence_policy.rs:158-186`、`runtime/transport.rs:277-427`、
  `focus/class_names.rs:246` 以降、`focus/tracker.rs:274` 以降、
  `runtime/key_pipeline.rs:2153` 以降はいずれも `#[cfg(test)]` の中である
  （各ファイルの `#[cfg(test)]` 開始行: `key_sequence_policy.rs:120` /
  `transport.rs:190` / `class_names.rs:239` / `tracker.rs:263` /
  `key_pipeline.rs:2145` / `focus_tracking.rs:465` / `ime_controller.rs:557`）。

  **これを「凍結の根拠」に数えてはならない。** 逆に、凍結を決めることで
  この不変条件は**未達のまま確定する**（ADR-081 側にもそう書く、F-3）。
  なお `focus/class_names.rs:199-201`（#6）は `AppImeProfile`（focus 層）→
  `ImePolicyProfile`（state 層）の変換そのもので、この 1 箇所は残すのが
  正しい（変換点を 1 つに集める設計）。`focus/tracker.rs:149`（#7）も分類の
  内部であって「コアループ」ではない。`class_names.rs:76`/`:87`（#4/#5）は
  「この直接比較を呼び出し元に書かせないための関数」の**中身**なので、
  ここに 1 箇所ずつ残ること自体が設計意図どおりである。
  **実質的にコアループの分岐と言えるのは #1〜#3、すなわち
  `key_pipeline.rs:1981` と `focus_tracking.rs:255`/`:382` の 3 箇所**で、
  #1/#3 は「ImmCross プロファイル（`Standard`）のときだけ IMM32 の追加 probe を
  spawn する」、#2 は逆に「`Imm32Unavailable` なら実状態を読めないとみなす」
  という、いずれも**読み取り（probe）側**の制御であり、
  `caps` にも `ImeProfileDriver` にも対応する軸が
  無い（`caps` は「どう書くか」の表であって「どう読むか」の表ではない）。
  この 3 箇所を型で消す作業は本 ADR のどの項にも属さない**未回収の残作業**で
  ある（§7-8 に記録）。

#### F.3 具体的な設計案（＝決定）

**決定 F-1: ADR-081 Phase 1d / 1e を凍結する（着手しない）。**

凍結の根拠を 3 点に限定して明記する。**「実機が無くて止まっているから」は
根拠にしない**——ADR-089 §6 が指摘するとおり、その論法を認めると
ADR-089 自身の Phase C も同じ理由で捨てられることになる:

1. **表現手段が重複しており、`caps` のほうが細かい。** ADR-081 の trait は
   profile 軸（静的）のみ、`caps` は (profile, IME 種別) の 2 軸。
   ADR-081 が「GJI 横断性の設計」節で design B（profile 軸と IME 軸の分離）を
   採ったのは、trait だと 2 軸を扱えないからであり、`caps` は const 表なので
   その制約が無い。
2. **ADR-089 §4.1 が capability の trait 静的分岐を「再提案禁止」で却下している。**
   Phase 1d は「`AppImePolicy` 参照をドライバ呼び出しへ置換する」作業であり、
   まさにその trait 静的分岐の配線である。**凍結しないことは、却下済みの案の
   実装を続けることを意味する。**
3. **Phase 1e の成果物（legacy 同期の撤去）は、ADR-089 INV-43 によって
   「解決した」のではなく「禁止された」。** ここは起票時の草稿が
   「Phase 1e のブロッカーは解決済み」と書いていたが overclaim なので訂正する。
   経緯を分けて書くと:

   - **非対称そのものは解消した。** `uses_gji_direct()` の撤去（Phase B item 8）
     で ADR-081 の contract test 不変条件 4・5 は ADR-089 INV-42/43 へ移り
     （ADR-081 の 2026-08-12 追記が記録済み）、2026-08-02 に Phase 1e の
     ブロッカーとして発見された「`GjiFsm` 同期義務が profile 軸で非対称」
     （ADR-081:696-726）は、**同期義務の宣言軸から profile を外す**ことで
     消えた（INV-42 = `legacy_gji_sync_obligation` が唯一の導出式）。
   - **しかし Phase 1e が目指していた「legacy（`on_ime_applied` の直接呼び出し）
     の撤去」（ADR-081:505, :708）は、いま明示的に禁止されている。**
     ADR-089 INV-43（`089:1277-1288`）は「**『型で守られている』を根拠に
     `platform.rs:879-891` の legacy 同期を撤去しないこと**」と書いている。
     理由は `ActuationReceipt` の強制力が debug ビルドの実行時検出までしか
     無いこと（§8.1）である。

   したがって Phase 1e は「ブロッカーが取れて着手可能になった」のではなく、
   **成果物そのものが不変条件で塞がれて moot になった**。凍結の根拠としては
   これで十分だが、**「解決した」と書くと次の担当者が「では着手できる」と
   読む**ため、この区別を残す。

**決定 F-2: `ImeProfileDriver` を「`caps` と重複しない軸だけ」に縮小する。**

trait ごと削除はしない。理由は不変条件 1
（`has_ime_on_path()==true` のドライバは stale `ObservedEisu` 救済を対で持つ）に
自然な置き場所が `caps` に無いためである——これは**capability の値**ではなく
**コード構造についての契約**であり、const 表に載る種類の情報ではない。

縮小の内訳:

| メソッド | 処遇 | 移行先 / 理由 |
|---|---|---|
| `default_feedback()` | **削除** | `caps(p, k).feedback` が SSOT。contract test 不変条件3（`Blind` give-up の有界終端）は `caps` 由来の `FeedbackPolicy` で駆動する形へ**書き換える**（テストの主題が SSOT へ寄る分むしろ強くなる） |
| `focus_settle_ms()` | **削除** | `caps(p, k).focus_settle_ms` が SSOT |
| `ime_open_mechanism()` | **削除**（ただし contract test 不変条件2 の差し替えが前提、下記） | `caps(p, k).chain` が SSOT。`ImeOpenMechanism` enum（`ime_profile_driver.rs:52-73`）も未使用になるので同時に削除。`tests/ime_key_sequence_golden.rs:231-287` の `driver_shadow_parity_matches_characterize_strategy_primary_path` とヘルパ 2 本も同時に削除（このテストの唯一の入力が `ime_open_mechanism()` であるため） |
| `probe_budget_ms()` | **削除** | 未配線・未実測・`ColdReason` 軸の精緻化は実測義務で着手不能。**設計のスケッチは git 履歴に残る**ことを ADR-081 に明記する |
| `owns_physical_kanji()` | **残す** | ADR-089 §2.5 が `caps` に入れないと決定済み（BUG-46）。ただし doc の「実効的な disposition の SSOT ではない」注記は必須（`runtime/transport.rs::PhysicalKeyDisposition::plan` が実 SSOT） |
| `has_ime_on_path()` / `stale_eisu_recovery_paired()` | **残す** | contract test 不変条件1（BUG-07/22/37 ファミリー）の宣言点 |
| `driver_for` レジストリ / `ALL_DRIVERS` | **残す** | 上記 3 メソッドの contract test に必要 |

縮小後の `ImeProfileDriver` は「プロファイル別 capability ドライバ」ではなく
**「プロファイル別のコード構造契約の宣言」**になる。**モジュール doc を
その意味へ書き換える**こと（名前を残したまま意味だけ変えると、次の人が
capability 表と読み違える。ADR-089 §9-3 が `caps` と `AxisCapability` について
警告しているのと同じ混同）。

**決定 F-2': 削除で消える contract test を、消す前に差し替える。**

§4.7 は「重複する 4 メソッドだけを削り、契約宣言として残す」と書いているが、
**残す 3 契約のうち不変条件2 は `ime_open_mechanism()` に依存している**ので、
そのまま削ると契約が 1 つ代替なしに消える。実コードを読んだ結果、扱いは
不変条件ごとに違う:

| contract test | 削除の影響 | 差し替え |
|---|---|---|
| 不変条件1（`invariant_1_ime_on_path_drivers_pair_eisu_recovery`、`ime_profile_driver.rs:531`） | 無し（`has_ime_on_path` / `stale_eisu_recovery_paired` は残す） | 不要 |
| 不変条件2（`invariant_2_kanji_owning_drivers_use_non_kanji_mechanism`、`:552`） | **assert の唯一の入力が `ime_open_mechanism()`** なので、削除すると test ごと消える | **そのまま残す価値は無い**——現在の assert は `matches!(mechanism, CrossProcessApi \| SharedImeKeyDispatch)` で、`ImeOpenMechanism` はこの 2 variant しか持たないため **恒真（どう実装を壊しても落ちない）**。したがって「代替なしに消える契約」ではなく「**元から効いていなかった契約**」である。削除と同時に、意図（物理 KANJI を所有するプロファイルは、その物理キーの代わりに送る一次機構として `KanjiToggle` を使わない）を実際に検査する形へ作り直す: `owns_physical_kanji`（`app_ime_policy.rs:220` = `!matches!(profile, TsfNative)`）が真なプロファイルについて `caps(p, k).chain` の**先頭**が `KanjiToggle` でないことを固定する。`(ImmCross, MsIme)` の chain は `CHAIN_IMM_CROSS_THEN_KANJI = [ImmCross, KanjiToggle]`（`app_ime_policy.rs:64`）なので、**先頭**を条件にしないと成立しない（末尾の `KanjiToggle` は `ImmCross` が `Failed` を返したときのフォールバックであり、INV-44 の到達可能性検査が正当化している） |
| 不変条件3（`invariant_3_blind_drivers_terminate_without_writing_observation`、`:575`） | driver の `default_feedback()` で駆動しているので、削除すると駆動元が消える | F-R1 のとおり `caps(p, k).feedback` 駆動へ差し替える（主題が SSOT へ寄るぶん強くなる） |

**この差し替えを F の完了条件に含める**（§6 ステップ 3）。恒真テストを
「あるから安心」と数えないことは、ADR-089 §9 が「型は入ったが効いていない」を
正直に書いたのと同じ規律である（P22）。

**決定 F-3: ADR-081 のステータスを更新する。**

- ステータス節に「Phase 1d/1e 凍結（本 ADR による）。capability の表現は
  ADR-089 `caps(p, k)` に一本化した」を追記。
- F-2 で削除する 4 メソッドそれぞれについて、**廃止理由**を ADR-081 側に
  明記する（ADR-089 §6「凍結する場合、成果物に ADR-081 のステータス更新を
  含めること」の要求）。
- **ADR-081「不変条件（Phase 1 着手時に強制する候補）」の 1 つ目について、
  達成状況を正直に書く**（§2.F.2 の表）: `ImeActuatorKind::` 側は達成、
  `AppImeProfile::` 側とテキスト走査ガードは**未達のまま凍結**。
  「凍結＝達成」と読まれないよう、残存 7 サイトのうちコアループの 3 箇所
  （`key_pipeline.rs:1981` / `focus_tracking.rs:255`/`:382`）と、
  それが `caps` では回収できない理由（読み取り軸が無い）を書き添える。
- **Phase 1e については「ブロッカーが解決した」と書かない。**
  成果物（legacy 同期の撤去）が ADR-089 INV-43 で禁止されて moot になった、
  と書く（F-1 根拠 3）。
- ADR-089 §9-4 の「未決定」を解消済みに更新する。

#### F.4 実装した場合のリスク

| # | リスク | 評価と緩和 |
|---|---|---|
| F-R1 | **contract test 不変条件3 の主題が変わる。** `default_feedback` を消すと、駆動元が driver から `caps` へ移る | 移行の前後で同じ入力に対し同じ判定になることを、`decide_actuation_action` の全数テスト（`FeedbackPolicy` × attempts）で固定してから消す |
| F-R2 | **`probe_budget_ms` を消すと BUG-01/BUG-21 の重症度別予算の設計スケッチが失われる** | ADR-081 の該当節（Phase 1a/1b 実施記録・Phase 1d 申し送り）に**残っている**。コードから消えても ADR とコミット履歴に残るので、実測が取れた段で復元できる。ADR-081 に「復元元はここ」と明記する |
| F-R3 | **「凍結」が「ADR-081 の問題意識まで捨てた」と読まれる** | 捨てない。ADR-081 Phase 0 の定量調査（known-bugs 43 件の分類、cross-profile spillover 11 件 = 26%）は本 ADR も ADR-089 も前提として使っている。凍結するのは**表現手段（trait 静的分岐）**であって**問題意識（プロファイル差分を 1 箇所に閉じる）**ではない。`caps` がその問題意識を引き継いでいる |
| F-R4 | **削除そのもののレビューコストがゼロではない**（ADR-089 §9-4 が指摘） | 起票時の草稿は「約 150〜200 行の純減」と見積もっていたが**過小**（ADR-081 §2 の「3 ドライバの型の骨組み ~156 行」を裏返しただけで、後から積まれた分を数えていなかった）。実コードで数え直すと **gross ≈ 365 行**: `ime_profile_driver.rs`（626 行）から ①`ImeOpenMechanism` enum + doc（`:52-73`、~22）②trait メソッド宣言 4 本 + doc（`:122-157`、~35）③3 impl × 4 メソッド（`:181-208` / `:256-278` / `:308-330`、~74）④`BLIND_MAX_ATTEMPTS` + `blind_feedback()`（`:210-231`、~21）⑤`#[cfg(test)]` 内の parity / 予算テスト群（`focus_settle_matches` / `never_needs_cold_start_probe` / `opens_via_cross_process_api_not_vk` / `assert_policy_parity` の 2 項目 / `imm32_unavailable_driver_matches` / `tsf_native_driver_matches` / `imm_cross_..._default_feedback` / `registry_maps_...` の一部 / `probe_budget_references_existing_tuning_ssot` / 恒真だった不変条件2、~150）、`tests/ime_key_sequence_golden.rs` から ⑥`driver_shadow_parity_...` + `driver_shadow_strategy_name` + `app_profile_to_policy_profile` + import（`:35`, `:231-287`、~60）。差し替えで足し戻す分（不変条件2 の `caps` 版・不変条件3 の駆動元差し替え、~35〜50）を引いて **net ≈ 280〜330 行の純減**。Linux で完結し、未配線コードの削除なので挙動変更はゼロ |
| F-R5 | **「今が最も安い」という前提が、A/D の配線で崩れる** | **崩れない**——A/D は `caps` 側と actuation チェーン側に触るが `ImeProfileDriver` には触らない。ただし F を後回しにするほど、`caps` を触る人が「driver 側も直すべきか」を毎回考えることになる（判断コストは増える） |

#### F.5 優先度と規模感

優先度 **中〜高**（決定そのものは即時、実装は「今が最も安い」）。
規模 **中**（gross ≈ 365 行削除 / net ≈ 280〜330 行の純減、F-R4 の内訳。
触るファイルは `ime_profile_driver.rs` + `tests/ime_key_sequence_golden.rs` +
ADR-081 / ADR-089 の 4 つ）。**起票時の「小〜中」から上方修正した**——
`tests/ime_key_sequence_golden.rs` の driver shadow parity と
contract test 不変条件2・3 の差し替えを数えていなかった。
**Linux で完結、挙動変更ゼロ**（未配線コードの削除であるため）。

---

### G. golden の stale な名前を CI 検証付きで直す

#### G.1 現状の問題

`tests/golden/ime_key_sequences.txt` と、それを生成する
`tests/ime_key_sequence_golden.rs` の定数に、**すでに存在しない名前が 2 種類**
残っている:

| stale な名前 | 消えた時期 | 出現箇所 |
|---|---|---|
| `set_ime_romaji_mode()` | ADR-089 Phase C item 12 で削除（`_async` ともに） | `.rs:70`・`:82`（`KEY_DOC` 内）／ `.txt:30`・`:42` |
| `apply_skipping_imm` | ADR-089 Phase B item 6 で撤去 | `.txt` に **7 箇所**（`build_report()` の dispatch 列の値 6 = 3 profile × 2 IME 種別 + 凡例 1 行（`.txt:8`））／ `.rs` は `HEADER` 内の凡例（`:58`）と `for` 式のラベル（`:124`）の 2 箇所 |

ADR-089 §7 は前者だけを挙げ「更新には golden の再生成が要るため、次に実機で
golden を回すときにまとめて直すこと」と書いていた。

#### G.2 実コードで確定させた訂正

**実機は要らない。** `build_report()`（`:120`）は

```rust
out.push_str(HEADER);
for &(active, active_gji, profile) in COMBOS {
    for (dispatch, skip_imm) in [("apply", false), ("apply_skipping_imm", true)] { .. }
}
out.push_str(KEY_DOC);
out.push_str(WARMUP_DOC);
```

という形で、`HEADER` / `KEY_DOC` / `WARMUP_DOC` は**定数文字列をそのまま
連結している**。テストは `UPDATE_GOLDEN=1` で再生成でき、それ以外では
生成結果とファイルを全文比較する（`:161-162`）。**このテストは
`#![cfg(windows)]` だが、CI の `windows-build` ジョブが
`cargo nextest run -p awase-windows --test ime_key_sequence_golden` を実行する**
（ADR-089 §9-17 冒頭が確定させた事実）。

したがって、`.rs` の定数と `.txt` を**同時に手で直せば、push した時点で CI が
一致を判定する**。Windows 実機は不要である。

#### G.3 具体的な設計案

1. `KEY_DOC` の `set_ime_romaji_mode()` → **`romaji_pre_write()`**
   （`ime_controller.rs` の ROMAN 補完ステップ。実際に write する低レベル関数は
   `set_ime_romaji_mode_for_target_blocking()`）。挙動の記述
   （「ROMAN ビットを先に立てる」）は今も正確なので変えない。
2. dispatch 列の `"apply_skipping_imm"` → **`"async_fallback"`**
   （`runtime/open_chain.rs::run_open_chain_async` の ImmCross `Failed` 後の
   フォールスルー）。`HEADER` の凡例行も同じ言葉に揃える。
   **戦略選択の期待値（`GjiDirect` / `KanjiToggle` 等）は 1 文字も変えない。**
3. `.txt` を手で同じ内容へ直す（または Windows CI で `UPDATE_GOLDEN=1` を
   一度だけ回した結果を取り込む）。
4. **`characterize_strategy(active_gji, profile, skip_imm)` のシグネチャは
   変えない**（`skip_imm: bool` はチェーンの 2 番目以降を走る意味であり、
   その意味自体は残っている）。

#### G.4 実装した場合のリスク

| # | リスク | 評価と緩和 |
|---|---|---|
| G-R1 | **golden ファイルに触ること自体が、キー選択の回帰検知点を動かす** | 変えるのは**列の値のラベルと doc 文字列だけ**で、`characterize_strategy` の戻り値（= 実際に選ばれる戦略名）は 1 文字も変わらない。差分を見れば「ラベルのみ」であることが一目で分かる形にすること（1 コミットで他の変更を混ぜない） |
| G-R2 | **`.rs` と `.txt` の手編集がずれると CI が赤くなる** | それが正しい挙動である（ずれを検出するためのテスト）。赤くなったら `UPDATE_GOLDEN=1` の結果を取り込む |
| G-R3 | **`.claude/rules/fix-requires-evidence.md` の「キー選択」ファミリーに形式上該当する** | 該当するので golden 更新を伴う（本項そのものが golden 更新である）。挙動変更はゼロなので known-bugs 追記は不要 |

#### G.5 優先度と規模感

優先度 **低**（実害なし。ただし「stale な名前が残っている」こと自体が、
次に読む人に「`apply_skipping_imm` はまだあるのか」と誤解させる）。
規模 **極小**（2 ファイル、定数文字列のみ）。**実機不要・CI で検証可能**
（ADR-089 §7 の記述はこの点で誤っていた）。

---

## 3. 優先順位と規模の一覧

| 順 | 項 | 内容 | 優先度 | 規模 | 検証環境 | 挙動変更 |
|---|---|---|---|---|---|---|
| 1 | **C** | 観測ストアの裏口を可視性で塞ぐ + 閉じられない witness の理由確定 | 高 | 小 | Linux | 無し |
| 2 | **B** | `ConvergedReceipt` 配線 + `most_recent_trusted_after` を private 化 | 高 | 小〜中 | Linux | 無し（bit-identical） |
| 3 | **F** | ADR-081 Phase 1d/1e 凍結 + `ImeProfileDriver` の縮小 | 中〜高 | 中（net −280〜330 行） | Linux | 無し（未配線コードの削除） |
| 4 | **E** | dylint 2 crate の恒久方針を確定（ドキュメントのみ） | 中 | 極小 | — | 無し |
| 5 | **A-1** | `ActuationOrder` を全入口へ配線（shadow モード） | 中〜高 | 中 | Linux | 無し（ログ/journal のみ増える） |
| 6 | **D** | 非同期チェーンの `caps` 再抽選化 | 中 | 中 | Linux + **実機ソーク** | 有り（等価のはず） |
| 7 | **A-2** | warrant の強制（入口ごとに 1 つずつ） | 高（価値）／低（着手可能性） | 大 | **実機ソーク必須** | **有り（最大 9 通り）** |
| 8 | **G** | golden の stale な名前を直す | 低 | 極小 | CI（`windows-build`） | 無し |

### 順序の根拠

- **1〜4 は「今やらないと高くなる／今なら無料」に並べた。**
  C は `per_source` を読む新しいコードが増えるほど差分が広がる。
  F は ADR-089 §6 自身が「配線前の今が低コスト」と書いている。
  E は決めないと 5 回目の再提案が出る（実際に 4 回出た）。
  B は費用対効果が最も良い——ADR-089 が入れた型のうち「効いていない」ものを
  **Linux だけで、挙動を変えずに**「効く」へ変えられる唯一の項である。
- **5〜7 は「実機ソークの必要度」で並べた。** A-1 は挙動を変えないので
  先に入れて**測定手段を作る**。D と A-2 はどちらもソークが要るが、
  D は等価性を実コードで論証できる（§2.D.2 の表）のに対し、
  A-2 は差分オラクルが**明示的に 9 通りの挙動変化を予告している**。
- **8 は独立**。いつやってもよいが、他の作業で golden に触るタイミングが
  あればそこで混ぜず、単独コミットで入れる（G-R1）。

### 「Phase B クラスの大きな変更」に相当するのはどれか

ADR-089 Phase B（新設 2 ファイル + 改修 8 ファイル）を基準にすると:

- **A-2 だけが Phase B より大きい**（挙動変更 + 入口ごとのソーク分割）。
- **A-1 と D は Phase B より小さく Phase C（改修 9 ファイル・新設なし）と同程度。**
- **B・C・F は Phase C より小さい**（3 ファイル以内）。
- **E・G はドキュメント / 定数のみ。**

---

## 4. 検討して採らなかった案

### 4.1 却下: `ImeControlView` に `OpenWarrant` を載せる（項 A）

**却下の根拠は `Copy` ではない**（起票時の草稿は「view が `Copy` を失う」と
書いていたが、`ImeControlView<'a>` は既に `FocusFacts<'a>` を借用で保持する
lifetime 付き構造体なので、`Option<&'a OpenWarrant>` なら `Copy` は失われない。
§2.A.2(2) で訂正済み）。却下の理由は次の 3 点である:

1. view の構築点 `WindowsPlatform::build_ime_control_view`（`platform.rs:1005`）
   は `ImeStateHub` を持たず、`WarrantContext` を組めない。
2. view には actuation でない読み手（`is_applicable` /
   `imm_cross_is_first_applicable` / `characterize_strategy`）が居り、
   warrant をフィールドにすると読み取りのたびに授権を発行することになる。
3. view は `fallback_write` がチェーンの機構ごとに作り直す
   （`open_chain.rs:206`）ため、warrant がチェーン途中で**暗黙に再発行**される。
   再発行は §2.A 設計案 4 が明示的に行うと決めた操作であり、暗黙に起きてはならない。

**`ActuationOrder` を別の値として運ぶ**（§2.A 設計案 1）。

### 4.2 却下: `ImeController` / `open_chain` から `ImeStateHub` を直接読んで
warrant を発行する（項 A）

**却下の根拠はレイヤ境界ではない**（起票時の草稿は
`tests/layer_boundary_guard.rs` の違反と書いていたが誤り。B-1 の禁則
ディレクトリに `ime_controller.rs` は含まれず、同ファイルは既に
`crate::state::*` を import している。ADR-065 が禁じるのは逆向き——
`state/` が windows 型に依存すること。§2.A.2(1) で訂正済み）。

却下の理由は **`with_app` 再入**である。`ImeController::apply` /
`fallback_write` の呼び出しフレームは既に `crate::with_app` の内側にあり
（`open_chain.rs:203-212` はまさに `with_app(|app| ..)` の中で
`shadow_ime_control_view()` と `apply_mechanism` を呼んでいる）、そこから
`ImeStateHub` に届く唯一の手段である `with_app` を再度呼ぶと `RefCell` の
再入検出に当たる（[[project_in_with_app_removal]]）。加えて `intent_store` は
`ImeStateHub` の private フィールドで読み手が存在しない。
**warrant は、既に `&ImeStateHub` を持っている入口側で作って引数で運ぶ。**

### 4.3 却下: `OpenWarrant` に `focus_epoch` / `focus_gen` を持たせて
await をまたいだ失効を検出する（項 A / D）

warrant は**根拠軸**（その値を書いてよいか）、`ActuationTarget` は**空間軸**
（どのウィンドウへ書くか、ADR-086 INV-14）という ADR-087/086 の役割分担を
崩す。await をまたいだ失効は**チェーンの再抽選**（項 D）で扱う。
なお ADR-087 §4 INV-23 は「`WarrantContext` は 1 回の呼び出しの間は不変」と
しており、warrant 自体に時間軸の意味を持たせない設計を既に採っている。

### 4.4 却下: `ConvergedReceipt::new` を private にする（項 B）

receipt を偽造しても `AnyObservation` へは変換できない（INV-46 がそこを
守っている）ので害が無い。塞ぐべきは「観測を直接手に入れる口」
（`most_recent_trusted_after`）であり、**そこを取り違えると作業だけ増えて
保証は増えない**。

### 4.5 却下: `ImeObservation` の各フィールドを private にする（項 C）

読み取り側（`tests/golden_scenarios.rs`、`derive_*`、drift 判定）まで壊れ、
アクセサを 7 本生やすことになる。**`#[non_exhaustive]` は
「crate 外からの構築だけ」を禁じ、読み取りは通す**——欲しい保証と過不足なく
一致する。

### 4.6 却下: `run_chain_async` に `FnMut() -> &'static [WriteMechanism]` を
渡す（項 D）

ADR-089 §9-20 の恒久策候補 (a) の素朴な形。実コードでは `fallback_write` が
既に `with_app` の中で完了時点の view を作り直しており、**クロージャを渡さずとも
その場で `caps` を引き直せる**。クロージャにすると呼び出し側が
「いつ引き直されるか」を追えなくなる。

### 4.7 却下: `ImeProfileDriver` を trait ごと削除する（項 F）

contract test 不変条件1（`has_ime_on_path()==true` のドライバは stale
`ObservedEisu` 救済を対で持つ、BUG-07/22/37 ファミリー）に自然な置き場所が
`caps` に無い——これは**capability の値**ではなく**コード構造についての契約**で
あり、const 表に載る種類の情報ではない。**重複する 4 メソッドだけを削り、
契約宣言として残す。**

**ただし「4 メソッドを削れば契約が 3 つ無傷で残る」わけではない。**
不変条件2 は `ime_open_mechanism()` を唯一の入力にしており（しかも現在の
assert は恒真）、不変条件3 は `default_feedback()` で駆動している。
削除と同時に両方を `caps` 駆動へ差し替えることが本案の前提条件である
（§2.F 決定 F-2'）。この差し替えを省くと、trait を残す目的（契約宣言）が
3 件中 1 件しか果たされない。

### 4.8 却下: dylint 3 crate を `architecture_guard` のテキスト検査へ今すぐ降格する（項 E）

検出力が落ちる。失うのは §2.E 決定 E-1 の 3 点——(a) crate 全体走査
（テキスト検査はファイル名と件数を直書きしており、新しいファイルでの構築に
気付けない）、(b) `typeck` による variant 解決、(c) `EngineActivationSync` の
防御（`architecture_guard` に等価な検査が無く、dylint 単独）。降格は
「ピン留め nightly の追従が現実的でなくなったとき」の**退避策**であって、
平時の選択肢ではない（§2.E 決定 E-3）。

### 4.9 却下: `try_force_on_bootstrap` に `!can_use_imm32_cross_process()` を
単独で足す（項 A）

ADR-089 §9-21 が「**ここで単独に足してはならない**」と明記している。
Standard での bootstrap force-ON が丸ごと止まる挙動変更であり、
ADR-087 Phase 3 の差分テストが「判明した中で最大の挙動変化」と記録している
論点そのもの。**A-2 の一部として、他の入口の実測が揃ってから判断する。**

---

## 5. 不変条件（invariant）

- **INV-47（項 A）**: 実 actuation は `ActuationOrder::issue()` を通ってのみ
  起案される。`ActuationOrder` の他の構築経路を作らない。
  `Actuation::warrant_pending_adr087()` は A-1 完了時点で削除する。
- **INV-48（項 A）**: `WarrantContext` は `ImeStateHub::warrant_context()` の
  1 箇所でのみ組み立てる。本番コードに `WarrantContext {` のリテラル構築が
  出現しない（`architecture_guard` が固定）。
- **INV-49（項 C）**: `ObservationStore` への観測の書き込みは
  `record` / `record_belief` / `record_replayed` の 3 口のみ。
  `per_source` は crate 外から到達不能（`pub(crate)`）であり、crate 内でも
  フィールドへの直接代入が本番コードに存在しない。
  `ImeObservation` は crate 外から構築できない（`#[non_exhaustive]`）。
- **INV-50（項 D）**: **非同期チェーンは起案時の chain を固定して保持しない。**
  ImmCross の完了後は、完了時点の view から `caps(p, k).chain` を引き直し、
  `attempted` 集合に含まれる機構を除いた残りを走る。同じ機構を 2 回
  write しない。
  「`caps(p, k).chain` の末尾に到達不能な要素を置かない」（起票時の草稿が
  INV-50 に書いていた内容）は**新規性が無いので採らない**——それは
  ADR-089 INV-44 そのもので、`app_ime_policy.rs` の
  `caps_chains_have_no_unreachable_trailing_element`（`:373` 付近）が
  既に機械検査している。本 ADR が新たに固定するのは**再抽選の側**であり、
  その正当性が依存する前提「`Failed` を返しうる機構は `ImmCross` だけ」は
  `WriteMechanism::may_return_failed()`（`actuation_chain.rs:182`）が SSOT で
  ある。再抽選のテストは**この関数を参照する形で書く**——`may_return_failed`
  が `ImmCross` 以外にも真を返すようになった瞬間に、`caps` 表と再抽選の
  両方を見直す必要があることがテストの失敗として現れるようにする。
- **INV-51（項 E）**: `lints/observation_source_guard` を降ろしてよいのは、
  §2.E 決定 E-2 の 3 条件（input_mode 観測の witness 化 / 本番構築点の witness
  経由への統一 / `ConvBitsInference`・`GjiIoInference` の evidence 型化）が
  **すべて**満たされたときに限る。`lints/ime_event_guard` は降ろさない。
- **INV-52（項 B）**: actuation の読み戻し（since フェンス付きの観測参照）は
  `ObservationStore::read_back()` を通ってのみ行う。
  `most_recent_trusted_after` は `ObservationStore` の外から呼べない。
  `most_recent_trusted`（`_after` 無し）は belief のフォールバック専用であり、
  actuation の読み戻しには使わない。
- **INV-53（項 F）**: `ImeProfileDriver` に残す 3 メソッド
  （`owns_physical_kanji` / `has_ime_on_path` / `stale_eisu_recovery_paired`）は
  **コード構造の契約宣言**であり、capability の値ではない。
  capability（チェーン / feedback / settle）の宣言点は `caps(p, k)` 1 箇所
  （ADR-089 INV-44）であり、trait 側に**戻してはならない**。
  contract test は 3 件とも「driver が返す値どうし」または
  「driver × `caps`」で閉じる形にし、**恒真な assert を置かない**
  （不変条件2 は 2026-08-12 時点で恒真だった、§2.F 決定 F-2'）。

**次の ADR は INV-54 から採番する。**

### 原則

- **P22: 型を書いた後に残る「効いていない」を、ADR に明示的な項として
  残し、閉じる条件を書く。**
  ADR-089 §9 の 10・11・12・16・20 は、いずれも「型は入ったが本番経路に
  効いていない」ことを正直に書いた。**その正直さは、閉じる条件と規模が
  書かれて初めて次の作業につながる。** 「効いていない」だけを書き残すと、
  次のセッションは (a) それを見落として型を信用するか、(b) 同じ調査を
  やり直すかのどちらかになる。本 ADR §2 の各項が持つ
  「現状の問題 / 設計案 / リスク / 優先度 / 規模」の 5 点セットは、この原則の
  実装である。

---

## 6. 移行計画

各項は独立してリリース可能で、後の項が中止されても前の項は残る。
§3 の順序で進める。

### ステップ 1（Linux で完結、挙動変更なし）— 項 C

1. `ObservationStore::observation(source)` を新設し、
   `tests/golden_scenarios.rs` の 2 行を書き換える。
2. `ObservationStore::per_source` と `PerSourceObservations` の 9 フィールドを
   `pub(crate)` へ縮小する。
3. `ImeObservation` に `#[non_exhaustive]` を付ける。
   compile_fail doctest（「通る双子」併記）を 1 組追加する。
4. `architecture_guard` に INV-49 のガードを 1 件足す。
5. **§2.C 設計案 4 の表を ADR に残す**（閉じられない witness の理由と、
   閉じる場合の設計・コスト）。

### ステップ 2（Linux で完結、bit-identical）— 項 B

6. `Resolution` を 2 値から 4 値へ広げ、`ConvergedReceipt` に `resolution` を
   持たせる。
7. `ObservationStore::read_back(now, since, query, attempts)` を新設し、
   `most_recent_trusted_after` を module private へ縮小する。
8. `ir_apply_drift_correction` の 2 箇所を `read_back` 経由へ書き換える。
   **移行前後の同値を Linux 全数テストで固定してから**書き換えること。
9. `drift_correction_giveup_and_confirmed_do_not_write_observations` は
   **削除しない**（ADR-089 §9-16）。

### ステップ 3（Linux で完結、未配線コードの削除）— 項 F

10. contract test 不変条件3 の駆動元を `caps(p, k).feedback` へ差し替える
    （F-R1: 差し替え前後で `decide_actuation_action` の全数テストが同判定に
    なることを固定してから）。
11. contract test 不変条件2 を `caps` 駆動へ作り直す（決定 F-2'）。
    現行は恒真なので、**先に新しい検査を書いて赤→緑を 1 度確認してから**
    旧テストを消すこと。
12. `ImeProfileDriver` から `default_feedback` / `focus_settle_ms` /
    `ime_open_mechanism` / `probe_budget_ms`、`ImeOpenMechanism` enum、
    `BLIND_MAX_ATTEMPTS` / `blind_feedback()`、および対応する parity テストを
    削除する。
13. `tests/ime_key_sequence_golden.rs` の
    `driver_shadow_parity_matches_characterize_strategy_primary_path` と
    ヘルパ（`driver_shadow_strategy_name` / `app_profile_to_policy_profile`）、
    `ime_profile_driver` の import を削除する（唯一の入力が
    `ime_open_mechanism()` のため）。**このファイルは windows-gated なので、
    削除の検証は CI の `windows-build` ジョブで行う。**
14. モジュール doc を「プロファイル別のコード構造契約の宣言」へ書き換える。
15. **ADR-081 のステータス節を更新する**（Phase 1d/1e 凍結、4 メソッドの
    廃止理由、`probe_budget_ms` の設計スケッチの復元元、**および
    「不変条件1 のうち `AppImeProfile::` 側とテキスト走査ガードは未達のまま
    確定する」**——§2.F.2 の表）。
16. ADR-089 §9-4 を「解消」へ更新する。

### ステップ 4（ドキュメントのみ）— 項 E

17. `.claude/rules/ime-belief-architecture.md` の「段2（dylint）」節に、
    降ろせる条件 3 つ（INV-51）と、ピン留め nightly が壊れたときの降格手順
    （特に `EngineActivationSync` のテキスト検査新設が降格の必須項目である
    こと、§2.E 決定 E-1(3)）を追記する。

### ステップ 5（Linux で完結、挙動変更なし）— 項 A-1

18. `ImeStateHub::warrant_context(now, now_ms)` を新設する（INV-48）。
    `intent_store` は private フィールドのままにし、この 1 本だけが読む。
19. `state/actuation_chain.rs` に `ActuationOrder` を新設する（INV-47）。
20. 実 actuation 入口（外部 8 経路）を `ActuationOrder::issue()` 経由へ移す。
    `set_ime_open` トレイト経路の 2 件は
    `WindowsPlatform::set_ime_open_ordered` へ移し、トレイトメソッドを
    死んだ入口として doc に明記する（期待値 2 → 0）。**同時に
    `ime_open_actuation_entry_points_are_accounted_for` のコメントの
    stale な行番号（`ime_refresh.rs:727` → `:752`）も直す**（§2.A.2(3) 脚注）。
21. `Authorization::LegacyUnwarranted` に `would_have_blocked` / `origin` を
    載せ、ログと journal（ADR-082 `JournalEntry::ImeActuation`）へ出す。
22. `warrant_pending_adr087()` を削除し、
    `legacy_unwarranted_actuation_sites_are_accounted_for` を
    「`would_have_blocked=true` が観測された入口の一覧」を固定する形へ
    作り替える。
23. **この時点で実機ソークを開始し、どの入口が何回 warrant を取れないかを
    測る。** ソーク項目は ADR-089 §9-17 に本項の観測項目を追加する形で書く。

### ステップ 6（実機ソーク必須）— 項 D

24. `run_chain_async` を「完了時点の view から `caps` を引き直し、
    `attempted` を除いて続ける」形へ書き換える。
25. `focus_gen` が起案時と変わっていたら warrant を再発行し、`None` なら
    チェーンを打ち切る（項 A 設計案 4）。
26. 再抽選が `may_return_failed()` を参照する形になっていることを固定する
    テストを足す（INV-50）。
27. ADR-089 §9-17 の 17-b / 17-g をソーク項目として引き継ぐ。

### ステップ 7（実機ソーク必須、入口ごとに分割）— 項 A-2

28. ステップ 5 のログで `would_have_blocked` がゼロだった入口から順に、
    `into_actuation()` が `None` のとき書き込みを中止する形へ倒す。
    §2.A.2 設計案 2 の差分表のうち **old-2 / old-3（計 7 件）は
    force-ON が減る方向**なので先に、**new-1（新規 force-ON）と
    old-1（bootstrap force-ON の消滅）は後に**回す。
29. `try_force_on_bootstrap` は**最後**に回す。倒す際は
    `!can_use_imm32_cross_process()` を単独で足さない（§4.9）。

### ステップ 8（CI で検証、いつでも可）— 項 G

30. `KEY_DOC` / `HEADER` / dispatch 列ラベルの stale な名前を直し、
    `.txt` を同期させる。**単独コミットにする。**

### revert する場合の義務

`.claude/rules/experiment-logging.md` に従い、本 ADR 由来の変更を revert する
コミットは本文に **アプリ / IME（種別と状態）/ 再現手順と症状** を必ず記載する。
特に **項 A-2 と項 D は `ime_controller.rs` / `runtime/open_chain.rs`
（キー選択・IME 制御）に触れる**ため、
`.claude/rules/fix-requires-evidence.md` の「キー選択（IME ON/OFF に送る VK）」
ファミリーに該当する。`tests/ime_key_sequence_golden.rs` の期待値更新か
`docs/known-bugs.md` の追記のどちらかを必ず添えること。

---

## 7. 未解決の論点

1. **項 A-1 の shadow ログをどれだけの期間集めれば A-2 の判断材料になるか。**
   `docs/experiments.md` エントリ01 が示すとおり、IME 系の不具合は
   「特定アプリ × 特定 idle 時間」でしか出ないことがある。
   `try_force_on_bootstrap` の発火条件（`IME_DETECT_MISS_THRESHOLD` 回連続の
   検出失敗）は稀であり、**1 日の通常利用では一度も踏まない可能性が高い**。
   「ログにゼロだったから安全」と「そもそも発火していないから測れていない」を
   区別する手段（発火カウンタを別に取る等）を A-1 の実装時に決めること。

2. **項 B の `ReadBackQuery::AnyFreshEvidence` は本当に `ConvergedReceipt` に
   載せるべきか。** give-up 後の復旧判定は「収束したか」ではなく
   「外界が動いたか」を問うており、`ConvergedReceipt` という名前と合わない
   （ADR-089 §9-16 も「receipt に載せる情報は `converged`/`attempts` の
   2 つでは足りない可能性」と書いている）。別型 `RecoveryReceipt` に分ける案も
   ある。**分けると INV-46 の「観測へ変換できない」を 2 型で守ることになる**
   ——それ自体は問題ないが、型が増える。API の形は配線時に決める。

3. **項 C の `#[non_exhaustive]` は `ImeObservation` だけで十分か。**
   `PerSourceObservations` と `ObservationStore` 自体も crate 外から
   `Default::default()` + フィールド代入で組み立てられる。
   `ObservationStore` は `Debug, Default, Clone` を derive しており、
   統合テストが `ObservationStore::default()` を作れることには価値がある。
   **どこまで `#[non_exhaustive]` を広げるかは、`tests/` の実際の依存を
   数えてから決める**（本 ADR では `per_source` の 2 行しか数えていない）。

4. **項 D の再抽選と `ImmCrossOp::Untargeted` の関係。** ADR-089 §9-19 が
   「残る `FocusImplicit` の本当の未移行分は `Untargeted` 1 件だけ」と特定して
   いる。D で `focus_gen` の変化を見るようになれば、その 1 件が抱えるリスクは
   実質的に緩和されるが、**`Targeted` へ寄せる作業そのものは残る**。
   §9-20 の恒久策候補 (b) を D と同時に入れるか、別に切るかは未決定。

5. **項 F で `ImeProfileDriver` を縮小した後、`driver_for` レジストリを
   残す意味があるか。** 残る 3 メソッドはいずれも
   `ImePolicyProfile` → bool の写像であり、`caps` と同じ const 表として
   書ける。**trait として残す価値は「contract test が impl 単位で回る」ことに
   尽きる**が、const 表 + 全数テストでも同じ検査ができる。縮小後に改めて
   「trait を残すか const 表へ寄せるか」を判断する余地がある。
   **ただし ADR-089 §4.1 の「trait 静的分岐は再提案禁止」は capability の
   分岐についての決定であって、契約宣言の表現手段については何も決めていない**
   ——この区別を混同しないこと。

6. **本 ADR は ADR-088 トラック A（`AxisCapability` + `CharsetOwner`）の
   実装計画と統合されていない。** ADR-089 §9-7 が残した「どちらを先に実装するか
   決めていない」がそのまま残っている。本 ADR の項 B・C は
   `state/observation_store.rs` に触るため、ADR-088 トラック A が同じファイルに
   触るなら競合する。**項 E（INV-51）は ADR-088 トラック A の完了を降格条件に
   しているので、両者の順序は「本 ADR → ADR-088」が自然**だが、確定していない。

7. **`ObservationStore::drift` / `current_focus_epoch` の可視性**（§2.C の
   C-R3）。本 ADR のスコープ外としたが、`drift` は
   `ir_check_drift_correction` の判定に直結する状態であり、
   `update_drift` 以外の書き手が増えると BUG-20/33/43 ファミリーの
   再発条件になりうる。**次に `observation_store.rs` を触るときに数えること。**

8. **【本 ADR のどの項にも属さない残作業】コアループに残る
   `AppImeProfile::` の直接分岐 3 箇所。** §2.F.2 の表（残存 7 サイトの
   #1〜#3）のとおり、`runtime/key_pipeline.rs:1981-1984` と
   `runtime/focus_tracking.rs:255-258`/`:382-385` は
   「ImmCross プロファイルのときだけ IMM32 の追加 probe を spawn する」
   （`:255-258` は逆に Imm32Unavailable のときの分岐）という**読み取り側**の
   プロファイル分岐である。ADR-081 の不変条件1 が消したかった対象だが、
   `caps(p, k)` は「どう書くか」の表であって「どう読むか（probe するか）」の
   軸を持たないため、F の凍結では回収されない。回収するなら
   `Caps` に読み取り軸（probe 要否 / probe 予算）を足すか、ADR-088
   トラック A の 4 軸一般化で扱うことになる。**本 ADR では起票しない**——
   `probe_budget_ms` と同じく `.claude/rules/tuning-constants.md` の実測義務が
   かかるため、実機の測定なしに設計だけ進めても値が決まらない。
   **「ADR-081 の不変条件1 は達成済み」と書かないこと**が本項の目的である。

---

## 8. 関連

- [ADR-080](080-ime-actuation-lifecycle-and-epoch-fenced-drift-correction.md):
  **不変条件6（`ReadBack` の産物を観測として記録しない）は、項 B の
  `read_back()` + `most_recent_trusted_after` の private 化で初めて
  コンパイラ強制になる**（INV-52）
- [ADR-081](081-per-profile-capability-driver-decomposition.md):
  **項 F が Phase 1d/1e の凍結を決定する。** `ImeProfileDriver` は
  `caps` と重複する 4 メソッドを削り、契約宣言（`owns_physical_kanji` /
  `has_ime_on_path` / `stale_eisu_recovery_paired`）として残す（INV-53）。
  **凍結にあたって、ADR-081 の不変条件1 のうち `AppImeProfile::` パターン
  マッチの排除とテキスト走査ガードは「未達のまま確定」する**（§2.F.2 の表・
  §7-8。`ImeActuatorKind::` 側のみ達成）。**Phase 1e の成果物である
  legacy 同期の撤去は、ADR-089 INV-43 が明示的に禁止している**——
  「ブロッカーが解決した」ではなく「成果物が禁止されて moot になった」
- [ADR-082](082-journal-structured-replay-and-event-origin.md):
  項 A-1 の `would_have_blocked` は `JournalEntry::ImeActuation` の
  `EventOrigin` と組で記録する
- [ADR-084](084-conv-mode-single-ownership-and-width-ssot.md):
  P1/INV-1（conv 単一 actuator）。項 C の `ConvSyncReason` witness を
  絞る場合は ADR-084/086 の conv 単一所有権と同じ段で判断する
- [ADR-086](086-force-write-trigger-and-target-identity.md):
  **INV-14（ターゲット同一性、空間軸）と項 A の warrant（根拠軸）を
  混同しない**（§4.3）。項 D の再抽選は `FocusImplicit` のフォールバックが
  INV-14 の保護を持たない穴を緩和する
- [ADR-087](087-open-belief-actuation-warrant-separation.md):
  **項 A の実装は本 ADR ではなく ADR-087 Phase 3（§5 item14〜17）として
  記録する。** 本 ADR §2.A は「発行した warrant をどう運ぶか」という、
  ADR-087 が書いていなかった運搬経路の設計を補う。差分オラクル
  （§8.11 item10 / §8.12）が予告する 9 通りの挙動変化が A-2 のリスクの中心
- [ADR-088](088-ime-axis-capability-and-charset-owner.md):
  **項 E の INV-51 は、`lints/observation_source_guard` を降ろせる条件を
  ADR-088 トラック A（input_mode を含む 4 軸の型化）の完了に紐付ける**
- [ADR-089](089-ime-typestate-and-capability-const-table.md):
  **本 ADR の直接の親。** §9-8 / §9-11 / §9-12 / §9-16 / §9-20 と
  §6「ADR-081 Phase 1d の凍結（提案）」/ §7「維持するもの」が
  本 ADR の 7 項の出発点。**§7 の「G は実機での golden 再生成が要る」は
  本 ADR §2.G.2 で訂正した**（CI の `windows-build` ジョブで検証できる）
- `docs/known-bugs.md`: **BUG-33**（give-up 後の観測書き込みによる収束偽装。
  項 B が型で閉じる）、**BUG-43**（`Blind` give-up の無限再送。項 B が触る
  制御フローそのもの）、**BUG-19**（conv 由来の間接推測が `desired_open` を
  書き換えた。項 C の witness 強度の根拠）、**BUG-07 / BUG-22 / BUG-37**
  （stale `ObservedEisu`。項 F が `ImeProfileDriver` に残す不変条件1 の根拠）、
  **BUG-46**（物理キー抑止。項 F が `owns_physical_kanji` を残す根拠）、
  **BUG-63**（`ConvOpenInference` 単独での force-ON。項 A の差分オラクルが
  old-only として捕まえているケース）
- `docs/experiments.md`: **エントリ01**（IME OFF キーが 5 日間で 6 回反転。
  項 A-2・項 D のソーク期間を「1 日の通常利用では足りない」と見積もる根拠、
  §7-1）
- `.claude/rules/ime-belief-architecture.md`: 3 段構えの強制。
  **項 E は段2（dylint）の恒久方針を確定し、降ろせる条件と降格手順を
  同ルールへ追記する**
- `.claude/rules/fix-requires-evidence.md`: 項 A-2 / 項 D は「キー選択」
  ファミリーに該当する（§6「revert する場合の義務」）
- `.claude/rules/experiment-logging.md`: §4 の却下記録はこの規約の
  ADR レベルでの適用（ADR-089 §4 と同じ）
- `.claude/rules/tuning-constants.md`: 項 F で `probe_budget_ms` を削除する
  根拠（`ColdReason` 軸の精緻化は実測義務により着手不能）
