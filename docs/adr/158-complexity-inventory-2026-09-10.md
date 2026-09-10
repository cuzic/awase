# ADR-158 複雑性インベントリ（2026-09-10）

[ADR-158](158-complexity-reduction-north-star.md)（北極星）・
[158-implementation-tasks.md](158-implementation-tasks.md)（タスク進捗）の補助資料。

## 位置づけ

`158-implementation-tasks.md`のタスクグループTA〜TJは、2026-09-10時点でほぼ完了している
（[TG1](158-implementation-tasks.md)のC2のみ未着手、TH1/TH4は能力ベース基準待ちでブロック）。
しかし**これらは「今後の増築を防ぐガバナンス機構」の整備であり、既存の増築そのものは
1つも解体・統合されていない**。実際、`docs/known-bugs.md`はADR-158測定時点（2026-09-08、
16,825行）から本調査時点で**17,054行**へ増えており（TH4が新規エントリに30行上限を
課した後もなお増加）、`.claude/rules/complexity-budget.md`（RC4対策の1-in-1-out規約）は
今も「起草済み・未発効」のままである。

本ドキュメントは、「次に何を実際に解体・統合するか」を判断するための材料として、
`crates/awase-windows/src`の実コードを直接調査した棚卸し結果を記録する。**調査は
Codex CLI（`codex exec -s read-only`）に依頼し、結果のうち主要な主張はClaude Code側で
`rg`/`Read`により独立に裏取りした**（裏取り済みの項目には✅を付ける）。判断そのもの
（何を実際に解体するか）はまだ行っていない——本ドキュメントは棚卸しに留める。

## 1. グローバル可変状態

`crates/awase-windows/src`の`static`宣言はCodex実測で**78件**。

### (a) ロックベース（`OnceLock<RwLock/Mutex>`・裸の`RwLock`/`Mutex`）

| 場所 | 対象 | 目的 | 所見 |
|---|---|---|---|
| `focus/classifier.rs:27` | `INPUT_RELAY_APPS: OnceLock<RwLock<Vec<String>>>` | `read_ime_state_fast`（`self`なしの`pub unsafe fn`、worker thread/main thread両方から到達）専用スナップショット | doc commentは✅確認済み。「`RwLock`はこの1箇所に限り正当」と明記 |
| `tsf/tip_detector.rs:33` | `PROFILE_DESCRIPTIONS: RwLock<Vec<ProfileDescription>>`（`OnceLock`無しの裸`RwLock`） | TIP profile description cache | ✅実在確認。**`INPUT_RELAY_APPS`の「この1箇所に限り正当」という主張と矛盾する** |
| `tsf/observer.rs:255` | `TSF_OBS`構造体内`ime_product_name: RwLock<Option<String>>` | IME製品名キャッシュ | ✅実在確認。同上、矛盾の2件目 |
| `hook.rs:29` | `HOOK_IME_MODE_DIAGNOSTICS: Mutex<VecDeque<_>>` | hook側IME mode診断リング | 既存`JournalLane`（`journal.rs:468`）と用途が近く統合候補 |
| `input_defer.rs:23` | `InputDeferQueue`内`Mutex<VecDeque<RawKeyEvent>>` | OUTPUT_GATE/TSF gate中の入力退避 | 実機順序保証に関わるため単純削除は不可、要個別調査 |
| `lib.rs:199` | `RAW_TSF_LITERAL`内`Mutex<String>` | raw TSF literal recoveryの再送文字列 | 統合余地はあるが要調査 |
| `app/logging.rs:178` | `LOG_WRITER_STATE: OnceLock<Arc<Mutex<_>>>` | rotating log writer flush | ログsubsystem固有、必要 |
| `tsf/probe_bridge.rs:75` / `tsf/observer.rs:408` | `*_TEST_LOCK: Mutex<()>` | test並列化防止 | test専用、対象外でよい |

**所見**: `INPUT_RELAY_APPS`のdoc comment「`RwLock`を使う理由...この1箇所に限り正当」は、
現在のコードでは不正確（`PROFILE_DESCRIPTIONS`・`TSF_OBS.ime_product_name`の2件が同種の
`RwLock`を独立に使っている）。RC3（同じ事実の未検証コピー、今回は「唯一の例外」という
主張自体が検証されずに残っていた例）の実例として記録に値する。

### (b) ロックフリー（裸の`AtomicXxx`カウンタ・フラグ）

約48件。主なグループ（詳細はCodex調査ログ参照、`hook.rs`だけで19件のatomic）:

- **hook hot-path群**（`hook.rs`）: 物理キー状態・親指/Alt状態・設定キャッシュ・hook生存監視。hot-path由来のため削除より`HookSharedState`的な集約が現実的な候補。
- **actuation/probe診断カウンタ群**: `conv_mutation.rs:33`・`win32.rs:246`・`probe_actuation_fence.rs:115,153,155,158,160`・`state/probe_admission.rs:44,50,53`。いずれも「発火/抑止の世代・診断」という同じ役割で、別々のモジュールに分散している。**単一のtelemetry/fence stateへの集約候補として最も分かりやすい**。
- **runtime/UI状態**: `runtime/engine_window.rs`・`runtime/message_handlers.rs`・`tray.rs`・`app/mod.rs`・`lib.rs`。Win32 app shell由来でおおむね必要。
- **小粒のwarning/test抑制フラグ**: `msime_key_assignment.rs:159`・`gji_charset_autodetect.rs:631,640,641`等。統合余地あり。

## 2. キュー・リングバッファ

実コード上で確認できたのは以下（少なくとも11構造体・journal 4 laneを個別に数えると14）。
ADR-158の「12個」という数字は近似としては成立するが、正確な数え方（構造体単位か、
journalの4 laneを1つと数えるか等）自体が曖昧だったことも分かった。

| 場所 | 対象 |
|---|---|
| `hook.rs:29` | `Mutex<VecDeque<HookImeModeDiagnosticRecord>>` |
| `input_defer.rs:11-23` | `InputDeferQueue`（`Mutex<VecDeque<RawKeyEvent>>`） |
| `hook_channel.rs:32-184` | `HookKeyRing`（SPSC ring, `CAP=1024`） |
| `journal.rs:468-518` | `JournalLane` × 4（state/timing/actuation/key_input） |
| `state/ime_event_log.rs:20-30` | `ImeEventLog` |
| `runtime/executor.rs:86,156` | `DecisionExecutor.queue`（`VecDeque<Effect>`） |
| `runtime/executor.rs:88,157` | `PassthroughQueue` |
| `runtime/executor.rs:91` | `guard_held`（OUTPUT_GUARDの1-slot queue） |
| `runtime/outbox.rs:25-44` | `RuntimeOutbox`（`Vec<RuntimeRequest>` FIFO） |
| `output/vk_send.rs:22-84` | `DeferGate` |
| `output/probe_io.rs:674` | local `VecDeque<ProbeAction>` |

**所見**: `RuntimeOutbox`と`DecisionExecutor.queue`はどちらも「runtime境界をまたぐ
deferred request」という同じ概念に見える（型は異なる）。統合可否を検討する価値がある。

## 3. dylintの本数と対象

✅`Cargo.toml:26-31`で実測・確認済み。**4本**。

| クレート | 目的 |
|---|---|
| `no_vk_as_scan` | `VkCode`を`ScanCode`として使う誤用検出 |
| `ime_event_guard` | `ImeEvent::PanicReset`/`HwndCacheRestored`の構築場所制限 |
| `observation_source_guard` | `InputModeObserved`の`ObservationSource`偽装防止 |
| `actuation_call_guard` | actuation choke point呼び出し元allow-list検査（TA2で追加） |

## 4. `tuning.rs`の定数

✅`rg`で独立に再計測・確認済み。`pub const`: **35件**、
`#[measured_macro::measured(...)]`付与: **35件**（付与漏れなし）。TE1（完了）の
成果がそのまま維持されている。

## 5. IME actuation合流点

`.claude/rules/fix-requires-evidence.md`が挙げる6箇所は、実コード上でも全て
`apply_ime_open_with_view`/`apply_ime_open_with_belief`等の合流点APIを実際に呼んでいる
ことを確認した。ただし低レベル境界（`send_input_safe`20箇所・`send_ime_control`10箇所）
まで含めると、`runtime/ime_refresh.rs`・`runtime/key_pipeline.rs`にも
`set_ime_open_ordered`/`apply_ime_open_with_belief`/`run_open_chain_async`の直接呼び出しが
複数あり、「6合流点」という数え方は上位APIレベルの粒度であって、実際の呼び出し経路の
総数はそれより多い（TB0/TB1の宣言はこの粒度で正しく`RESTRICTED_CALLS`に反映済み）。

## 6. IME戦略とconv-mode書き込み経路

- 4戦略（`ime_controller.rs`）の実装規模: `ImmCrossProcessStrategy`32行 <
  `KanjiToggleStrategy`29行 < `GjiDirectStrategy`38行 < `MsImeDirectStrategy`71行。
  `MsImeDirectStrategy`が突出して複雑——[ADR-160](160-explicit-non-scope-declaration.md)
  のC1（GJI一本化）を検討する際の具体的な削減量の目安になる。
- conv-mode書き込み経路: `set_ime_conv_for_target`経由5箇所・`modify_conv_mode`経由4系統、
  いずれも[ADR-160](160-explicit-non-scope-declaration.md)「TG1判断材料収集結果」の記述と
  一致することを実コードで再確認した。

## 所見サマリ：解体・統合候補（優先度順）と実施結果（2026-09-10追記）

1. **actuation/probe診断カウンタ群の統合** — **部分完了**。累積カウンタ8箇所
   （`state/probe_admission.rs`の3つ・`probe_actuation_fence.rs`のabandoned/spawned
   4つ・`hook_channel.rs`の1つ）を`LifetimeCounter`型に統合した
   （`refactor/adr158-shared-counters`ブランチ、コミット`ee66073f`）。一方
   `probe_actuation_fence::PROBE_ACTUATION_FENCE`・`conv_mutation::CONV_MUTATION_SEQ`は
   「単調に増え続けるフェンス（bump/current、staleness検知用）」であり累積カウンタとは
   意味論が異なると実装前調査で判明したため、意図的に統合対象から除外した
   （見落としではなく検討済みの判断）。`send_health::SendHealth`（サーキットブレーカ）・
   `focus_resync::FocusResyncGate`（世代付きゲート）も同じ理由で対象外。
2. **`INPUT_RELAY_APPS`の「唯一の例外」ドキュメントの訂正** — **完了**（コミット
   `8401daca`）。docコメントを実態（3箇所）に訂正し、`architecture_guard.rs`に
   クロススレッド共有ロック宣言の件数固定テストを追加した。新規dylintは
   `.claude/rules/ime-belief-architecture.md`の「意味論的偽装以外への新規dylintは
   過剰投資」という方針に従い見送った。
3. **`HOOK_IME_MODE_DIAGNOSTICS`と既存`JournalLane`の統合** — **調査の上、見送り**。
   `ImeEventLog`も含めた3者は「固定長・満杯時に最古を捨てる履歴リング」という
   ストレージ形だけは共通していたが、周辺の振る舞い（`Mutex`の有無・lane分類・
   seq付与）が異なり、共通化できる部分は「if full, pop_front」という数行に限られる。
   費用対効果が薄いと判断し実施しないことにした（ユーザー判断、2026-09-10）。
4. **`RuntimeOutbox`と`DecisionExecutor.queue`の統合検討** — **調査の上、見送り**。
   上記3と同じ調査で、両者とも実行順序制御・drainのタイミング管理が主目的で
   単なるストレージではないと判明し、統合候補から除外した。ADR-156が「defer/replay」
   5キューの統合を却下した理由（構造的な性質の違い）と同型。
5. **`MsImeDirectStrategy`（71行、他戦略の約2〜2.4倍）の複雑さ** — **未着手・判断待ち**。
   [ADR-160](160-explicit-non-scope-declaration.md)のC1（GJI一本化）を実施するか
   どうかの製品判断そのものであり、実装作業ではない。C1判断材料の一部としての
   数値提供に留める。

**除外（RC1寄りのため解体候補から外す）**: `HOOK_KEYS`（SPSCリング）・`OUTPUT_GATE`・
`INPUT_DEFER`・`TSF_OBS`の主要観測atomic群は、Win32 hook/SendInput/TSFの順序保証・
観測という、このリポジトリの存在理由そのもの（ADR-158 RC1: TsfNativeでIME open状態の
真値が読めない構造）に直結しており、単純な削除・統合の対象にはならない。

## 次のアクション（未決定・ユーザー判断待ち）

本ドキュメントは棚卸しのみ。上記5候補のうちどれを実際に着手するかは、
[complexity-budget.md](../../.claude/rules/complexity-budget.md)の発効条件
（実際の削除・統合1件を記録再生で送信列差分ゼロと検証できたこと）とも関係する——
**候補1・3・4はいずれもTH1の発効条件を満たす「最初の1件」の具体的な実施先になりうる**。
