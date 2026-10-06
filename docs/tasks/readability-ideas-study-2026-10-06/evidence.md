---
title: 可読性の改善案の検討 — 実測の事実
created: 2026-10-06
base: origin/develop 3c8ef62a
related_adr: ["ADR-229", "ADR-218", "ADR-219", "ADR-220", "ADR-090", "ADR-156"]
---

# 実測の事実(可読性の改善案の検討、2026-10-06)

基準: `origin/develop` の `3c8ef62a`。読み取りと grep だけで測った(ビルド・テストはしていない)。結論と案は [README.md](README.md)。

## (a) 長い関数・深いネスト(`crates/awase-windows/src/`)

**方法**: 使い捨ての Python スクリプトで、`fn 名前` の行から最初の `{` を探し、`{`/`}` の数で関数の終わりを見つけた。文字列リテラルと `//` 以降は除いた。ファイル内で最初に `#[cfg(test)] mod … {` が出た位置より後(テストモジュール)は数えていない。粗い測り方で、次の誤差がある: 構造体リテラルの `{` もネストに数える(例: `state/explicit_press.rs:469 PressState::all` の深さ 13 は、12 重の `for` で全状態を列挙するテスト用の関数で、読みにくさの例ではない)。`branches` は `if`・`match`・`while`・`for`・`=>`・`&&`・`||` の出現数。

| 行数 | 深さ | 分岐 | `return` | 関数 |
|---:|---:|---:|---:|---|
| 462 | 6 | 62 | 13 | `hook.rs:1421 hook_callback` |
| 397 | 7 | 63 | 5 | `tsf/gji_fsm.rs:600 on_event` |
| 376 | 2 | 22 | 0 | `journal.rs:806 emit_tracing` |
| 336 | 5 | 22 | 0 | `app/bootstrap.rs:1008 run_all` |
| 308 | 6 | 22 | 0 | `runtime/focus_tracking.rs:587 on_focus_process_changed` |
| 293 | 9 | 28 | 0 | `output/probe_io.rs:390 dispatch_probe_actions` |
| 286 | 6 | 18 | 4 | `runtime/key_pipeline.rs:2162 kp_restore_kana_from_half_width`(`.await` 4・`with_app(` 4) |
| 279 | 3 | 19 | 7 | `runtime/key_pipeline.rs:899 kp_stage_shadow_ime_toggle` |
| 273 | 7 | 30 | 0 | `runtime/key_pipeline.rs:2673 apply_focus_probe` |
| 266 | 7 | 44 | 1 | `runtime/message_handlers.rs:472 handle_wm_timer` |
| 235 | 5 | 23 | 5 | `runtime/ime_refresh.rs:804 ir_apply_drift_correction`(F4 で判断を分けた後の殻) |
| 196 | 3 | 6 | 3 | `runtime/executor.rs:742 dispatch_ime_set_open` |
| 188 | 4 | 17 | 1 | `runtime/open_chain.rs:223 imm_cross_write` |
| 156 | 4 | 13 | 0 | `ime_controller.rs:201 apply_mechanism` |

- 全 2,085 関数のうち、100 行以上が 43、200 行以上が 15、深さ 6 以上が 16(Opus のレビューが別のスクリプトで数え直し、100 行以上 43・200 行以上 15・`runtime/` 19 は一致。全関数数は検出方法の違いで 2,054)。`src/` の中にあるテスト用のファイル(例: `config_key_resolution_tests.rs`)の関数も混ざる(何件かは未確認)。
- ディレクトリ別の 100 行以上: `runtime/` 19(202 関数中)、`tsf/` 6、`state/` 4(616 関数中)、`output/` 3、`focus/` 0。**長い関数は `runtime/` に集中している**。`state/` は短い。

**コメントの割合は関数によって大きく違う**(行頭が `//` の行を数えた)。3〜5 割になるのは `hook_callback` と `runtime/` の IME 操作(actuation)まわりの関数で、長い関数全体には言えない:

| 関数 | コード | コメント | 本文中の `BUG-`/`ADR-`/`INV-`/`issue #`/`PR #` の参照 |
|---|---:|---:|---:|
| `hook_callback` | 270 | 174 | 39 |
| `kp_restore_kana_from_half_width` | 150 | 133 | 22 |
| `kp_stage_shadow_ime_toggle` | 158 | 118 | 21 |
| `ir_apply_drift_correction` | 143 | 86 | 27 |
| `open_chain.rs::imm_cross_write` | 118 | 69 | 16 |
| `dispatch_ime_set_open` | 128 | 68 | 13 |
| `ime_controller.rs::apply_mechanism` | 93 | 62 | — |
| `runtime/message_handlers.rs::handle_wm_timer` | 182 | 84(31%) | — |
| `runtime/focus_tracking.rs::on_focus_process_changed` | 217 | 82(26%) | — |
| `output/probe_io.rs::dispatch_probe_actions` | 231 | 54(18%) | — |
| `app/bootstrap.rs::run_all` | 266 | 40(11%) | — |
| `tsf/gji_fsm.rs::on_event` | 350 | 29(7%) | — |
| `journal.rs::emit_tracing` | 376 | 0(0%) | — |

crate 全体(テスト含む): コード 65,977 行・`//` コメント 7,569 行・doc コメント 17,994 行。`//` コメント中の `BUG-`/`ADR-`/`INV-` の参照は 2,933 件。

例: `ir_apply_drift_correction` の先頭のコメント(`ime_refresh.rs:805-811`)は「関数冒頭に残っていた早期 return を消し忘れていた」という BUG-20 の経緯を書いているが、その早期 return は既に無い。**今のコードを説明していない経緯のコメントが、関数の先頭に残っている**。

## (b) 実装済みの判断関数(`state/*_plan.rs` ほか)の形の比較

| ファイル | 本番の行 | テスト | 全数表(grep の該当行) | 戻り値の形 |
|---|---:|---:|---:|---|
| `drift_plan.rs` | 247 | 11 | 0(事実に `Instant` があるので全数表にしていない。V4) | **1 つの enum**(`DriftPlan::Idle(DriftIdle)`/`DeferToSettle`/`Act(DriftAct{step: DriftStep,..})`)。理由が variant |
| `msaa_role_plan.rs` | 130 | 4 | 1 | **1 つの enum**(`MsaaRoleDecision::TextInput(role)`…)と、そこから導く `.kind()` |
| `deferred_gate_plan.rs` | 118 | 5 | 5 | **1 つの enum**(`DeferPlan::Defer(BlockReason)` など。理由をデータに持つ) |
| `ime_set_open_plan.rs` | 49 | 2 | 2 | **1 つの enum**(`SetOpenPlan` 3 variant) |
| `focus_probe_plan.rs` | 75 | 7 | 2 | 1 つの enum。ただし**入力**が位置引数の bool 3 つ |
| `relay_plan.rs` | 264 | 9 | 5 | **「実行の種類」と「理由」の 2 つの enum を struct に並べる**(`RelayPlan{action, reason}`・`DrainStep{action, reason}`) |
| `ime_read_strategy.rs` | 124 | 1(+ 再生 fixture 11 件) | 1 | **2 つの enum を並べる**(`ReadDecision{strategy, reason, typing_guard_bypassed}`) |

読みにくさが残っている点(具体例):

1. **「実行の種類」と「理由」が別の enum で、組み合わせの大半は起こり得ない**。
   - `relay_plan.rs`: variant の数で数えて `RelayAction`(4)×`RelayReason`(5)= 20 通り(`QueueFlush { reinject: bool }` を 2 通りと数えれば 25 通り)を型が許すが、`plan_relay` が返すのは 5 通り。理由が決まれば実行の種類は 1 つに決まる(`PassThroughPhysicalSuppressed`→`ConsumeSuppressed`、`PassThroughIdle`→`RunPassthroughPipeline`、`FlushWithReinject`→`QueueFlush{reinject:true}`、`FlushPhysicalSuppressedNoReinject`→`QueueFlush{reinject:false}`、`EngineConsumed`→`ConsumeEffects`)。`DrainStep` も同じ(`DrainReason` 6 → `DrainAction` 2)。
   - `ime_read_strategy.rs`: `ReadReason` 4 → `ImeReadStrategy` 3 が一意に決まるのに、両方を struct に持つ。`ReadDecision {..}` の struct リテラルが 4 回繰り返される(`ime_read_strategy.rs:89-123`)。
   - 対照: ルート crate の `src/engine/decision.rs:130-133` の doc は「`consumed: bool` ではなく enum で意味を固定する。`PassThrough` なのに `SendKeys` が入る、といった不整合を型で防ぐ」と書いており、**このリポジトリは既に「起こり得ない組を型で書けなくする」流儀を持つ**。`relay_plan` と `ime_read_strategy` だけがそれと違う形になっている。
2. **「bool と、その bool が真のときだけ意味を持つ値」が別々の引数・フィールド**になっている。doc に「〜のときだけ意味を持つ」「読まないので偽を渡す」と書いて補っている箇所が、本番コードに 12(`deferred_gate_plan` 5、`ime_read_strategy` 3、`relay_plan` 2、`drift_plan` 1、`msaa_role_plan` 1。`意味を持つ|読まない|読んでいない|既定値のまま|使われない|使わない` の grep)。例: `plan_focus_probe(status, is_japanese_ime, grace_any: bool, grace_primary_reason: &str, shadow_on: bool)` は `grace_any` が偽なら `grace_primary_reason` を見ない。テストの呼び出しは `plan_focus_probe(read(true), true, false, "gji-io", false)` で、位置の bool が何かを読み手が覚えておく必要がある。
3. 早期 `return` の列そのものは読みにくくない。`decide_drift_plan` は「稼働条件 → ずれの有無 → settle → 試行の解決 → 授権 → 診断 → 方針ごと」の順に上から下へ読め、`return` は 4。`decide_read_strategy` は 2、`plan_drain_before_send` は 2。**早期 return は既に「途中で決まったら抜ける」を上から読める形で書けている**。
4. 事実を 1 つの引数にまとめる形(`DriftFacts`・`ReadStrategyFacts`・`RelayFacts`・`SetOpenFacts`・`BlockingFacts`)は、7 ファイル中 5 で既にできている。残りは `plan_drain_step(item, guard_passed, wait_remaining)`・`plan_defer(blocking, would_exceed_cap)`・`plan_drain_before_send(gate_enforced, blocking, queue_len)`・`plan_focus_probe(…5 引数)`。

## (c) 殻(実行側)で「判断 → 実行」の結びつきが読みにくい箇所

- **`runtime/executor.rs::execute_relay`(`:399-492`)は、同じことを 2 回場合分けする**。① `Decision` を `RelayDecisionKind` に写す match(`:407-411`)→ ② `plan_relay`(`:412`)→ ③ もう一度 `Decision` で match し(`:415`)、④ 各腕の中で `matches!(plan.action, …)` を見直す(`:419`、`:434-437`)。コメント(`:413-414`)は「フックのコールバック上なので `unreachable!` を使わず、Decision の種別ごとに plan の action を読む(対応は `plan_relay_exhaustive` が固定)」。つまり**判断の結果が「どの効果の列を運ぶか」を持たないため、殻が元の入力を握ったまま判断の結果と突き合わせている**。
- `ir_apply_drift_correction`(F4 後)は、`match decide_drift_plan(..)` → `match act.step` の 2 段で、判断と実行の対応は読める。読みにくさの大半は**腕の中の長いコメント**(上の表: コード 143 行に対しコメント 86 行・参照 27)で、構造ではない。
- `kp_restore_kana_from_half_width`(`.await` 4・`with_app(` 4)、`kp_stage_shadow_ime_toggle`、`hook_callback` は F の分割の対象外か未着手で、判断と OS 呼び出しが混ざっている。**これは書き方(DSL)ではなく F の分割(ADR-229 のサンドイッチ)で扱う種類**。

## (d) Rust に既にある「途中で抜ける・値を変換する」書き方の使用量(`crates/awase-windows/src/`、テスト含む)

| 書き方 | 件数 |
|---|---:|
| `?`(Option/Result の早期脱出) | 129 |
| `let Some(..) = .. else` / `let Ok(..) = .. else` | 104 / 13 |
| `.and_then(` | 38 |
| `.map_or(` / `.map_or_else(` | 34 / 34 |
| `.unwrap_or(` / `.unwrap_or_else(` | 74 / 73 |
| `.is_some_and(` / `.filter(` | 45 / 37 |
| `matches!(` | 410 |
| `return` | 733 |
| `#[track_caller]` | 5 |

- 「途中で決まったら抜ける」は、Rust の `return`・`let … else`・`?` が言語の機能として既に持っている(Haskell の `Maybe`/`Either` の do 記法で書くことの大半)。自前の型に `?` を使えるようにする `Try` trait は stable Rust では実装できない(使えるのは `Option`・`Result`・`std::ops::ControlFlow` など標準の型だけ)。
- `Option<bool>` は 107 か所、`unwrap_or(false)` は 10 か所。`Option<bool>` を `unwrap_or(false)` で潰して「未知」と「確認済みの偽」を取り違えるのは、BUG-113・ADR-098 で実害が出た既知の型(`fix-requires-evidence.md` の表の `shadow_on` の行)。
- 型状態(typestate)は既にある: `state/actuation_chain.rs:437` の `Actuation<S>`(`Requested` → `Warranted` → `Verified`、ADR-090)。`OutputActiveGuard` の順序を型にするのはタスク表 V5 として計画済み。

## (e) 読みにくさが実害に結びついた記録

`docs/known-bugs/BUG-*.md`(183 件)・`docs/adr/*.md`・`docs/adr/review/*.md` を語で grep した件数(ファイル数):

| 語 | known-bugs | ADR | review |
|---|---:|---:|---:|
| 見落と | 17 | 52 | 14 |
| 取り違え | 5 | 34 | 12 |
| 消し忘れ | 1 | 1 | 2 |
| 読み違え | 0 | 2 | 1 |
| 読みにく | 0 | 1 | 0 |
| 可読性 | 1 | 3 | 0 |

known-bugs の該当を読んで分けた結果(語が出るファイルのうち、原因の説明に関わるもの):

| 種類 | 例 | 書き方(DSL・combinator)で防げたか |
|---|---|---|
| 結果を見ずに「反映済み」にした + 長い手続きの中のガードの消し忘れ | **BUG-020**。本来の原因は、`set_ime_open` の戻り値(non-ImmCross では常に no-op で false)を見ずに `mirror_applied_open_with_ts` で反映済みにしたこと(`BUG-020.md` の原因節)。冒頭の早期 return の消し忘れで追加した分岐が dead code になったのは追補(同 :85-)の原因 | 前者は「結果を型で返し、無視できなくする」(`#[must_use]`・結果の enum)で防げる種類で、**案 A(結果を型にする)を支える**。後者は関数を短くする(F4 で分割済み)ほうが効く。DSL ではない |
| **抽象の層そのものが原因**: 委譲するラッパーが 1 メソッドだけ委譲し忘れ、trait の既定実装(何もしない)が黙って使われた | **BUG-027**(`ChromeProbe` が `TickableFsm::apply_vk_sent` を内側の `TsfProbeCoro` へ委譲していなかった。`BUG-027.md` の冒頭。旧コメントの「なし」は見落としを誘った副因) | **汎用の Handler・combinator を勧めない理由として最も強い実例**。既定メソッドを持つ trait と委譲の層を増やすほど、この種類が増える |
| 実態より広い名前が見落としを誘った | **BUG-129**(型名 `ComposingHint` が実態より広い意味を名乗っていた) | 名前を実態に合わせる。DSL ではない |
| 複数の窓口の片方だけに条件を配線 | ADR-156(ADR-123→128)、BUG-090/issue #136(gate を 5 経路中 1 つにだけ) | **代数的 effect の handler(1 か所で割り込む)が狙う問題そのもの**。このリポジトリは合流点の許可リスト(`lints/actuation_call_guard` の `RESTRICTED_CALLS`)・`decide_gate`/`apply_mechanism` への集約(ADR-180 決定1)・`architecture_guard` の件数固定で答えている。`with_app` を内包する共有 gate helper は再入で gate が無効になる(issue #136/BUG-90 型)ため ADR-180 で見送り済み(README §2) |
| 「未知」を `false` に潰した | BUG-113・ADR-098(`shadow_on`) | `Option<bool>` を保つ(型)。DSL ではない |
| 取り違え(キャッシュキー・宛先 hwnd など) | BUG-011、BUG-061、BUG-035 | ドメインの値の取り違えで、書き方の問題ではない |

- **「action と reason の組が食い違った」実害の記録は 0 件**。ただし `relay_plan` は F3(`76f96c76`・`ce8de690`、2026-10-05 マージ)で作られて 1 日なので、0 件なのは当然で、動機としては弱い。
- **monad 風の合成や DSL が無いことが原因になった記録は、語の grep と目視では見つからなかった**。ただし「合成の書き方が無いことが原因」は、grep の語(見落と・取り違え…)では原理的に拾いにくい分類で、「見つからない」は「無い」と同じではない。系統(上の表の「複数の窓口」の行)で見直すと、handler が狙う問題には記録があり、既存の仕組みで答えが出ている。

## 確認できなかったこと

- 行数・深さはスクリプトの粗い数え方。構造体リテラル・クロージャの `{` も数える。
- README の「撤去できる行数」は書いてみた数ではなく、現在のコードの行番号からの見積もり(コンパイルしていない)。
- `std::ops::ControlFlow` に `?` を使えることは Rust の仕様の知識で、このリポジトリで試していない。
