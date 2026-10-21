# ADR-275 草案 Opus 敵対的レビュー

対象: `/home/cuzic/rust-nicola-wt-adr275/docs/adr/275-move-platform-state-to-core.md`(作業ツリー HEAD 832e367f)。
行番号はすべてこの作業ツリーのもの。ファイルは編集していない。

## 結論(先に)

- **草案は INV-A97-1 が何をコンパイラに強制させているかを取り違えている。** 「書いてよい呼び出し元」の固定は、今でも
  コンパイラではなく CI(`RECORDERS` の件数)がやっている。`pub(crate)` が強制しているのは「awase-windows の外の crate
  から呼べない」ことだけ。本番の安全性の根拠は、architecture_guard 自身が書くとおり**本番のハブのインスタンスに
  crate の外から届かないこと**(`Runtime.platform_state` が private)で、これは移動しても変わらない。
  したがって D3 の弱まりは草案が言うより**小さい**。ただし別の、草案が数えていない損失がある(下の F1-4・F1-5)。
- **`ime_decision_view.rs` は移せない**(F2-1)。草案の D2 は誤り。
- **D1 は ADR-229 FCIS P2(核は scope を引数で受け取る純粋な形)の反転で、草案はそれを書いていない。** 削減は
  草案の約 -280 より小さく、テスト 90 か所の書き換えと「偽スコープ」の新しい落とし穴が出る(F3)。
- **段階 2 で得られるもののうち、移動しないと得られないものはほぼ無い**(F4)。mutation testing の対象化と
  dylint の UFCS 検出は、移動せずに今すぐできる。推奨は「段階 2 は見送り。D1 も `_in_scope` を残す小さい形に縮めるか
  見送り。代わりに独立の小 PR を 2〜3 本」。

---

## (1) record_* を pub にすると BUG-69 / INV-A97-1 の再発防止はどれだけ弱まるか

### F1-1 [事実の訂正・高] 「コンパイラ強制 → CI 強制」という対比は不正確

- 記録系は今 `pub(crate)`(`crates/awase-windows/src/state/platform_state/shell.rs:196,202,208`)。核の
  `_in_scope` 版は private `fn`(`platform_state.rs:864,879,1488`、子モジュールの shell.rs から見える)。
- `pub(crate)` は awase-windows(45,307 行)の**どのファイルからでも**呼ぶことを許す。呼び出し元を
  `runtime/mod.rs`・`ime_refresh.rs` 等のファイルごとの件数に固定しているのは
  `tests/architecture_guard.rs:2156-2227`(`applied_state_recorders_call_sites_are_accounted_for`)で、**今すでに CI だけ**。
- つまり「ローカルで cargo check だけ通して push すると漏れる」経路は**今すでに存在する**(後述 F1-6)。
  草案 D3 の「コンパイラ強制(違反=ビルドが通らない)から CI 強制へ」は、呼び出し元の固定については事実ではない。
- 失われるコンパイル時の性質は**1 つだけ**: 「awase-windows 以外の crate から record_* を呼べない」。

### F1-2 [根拠・高] 本当の不変条件は「本番のハブに外から届かない」で、移動しても保たれる

- `architecture_guard.rs:7662-7670` のコメントがすでに認めている: `pub fn dispatch_event` があれば
  `ImeApplyRequested → ImeApplySucceeded` で crate の外から `applied` を書ける。安全なのは
  「`Runtime.platform_state` が private、`PlatformState.ime` が `pub(crate)`」だから(`production_hub_is_unreachable_from_outside_the_crate`、
  `:7896-7915`)。
- さらに `ImeModel.applied` は**すでに `pub` フィールド**(`crates/awase-ime-core/src/state/ime_model.rs:269`)、
  `confirm_applied` も `pub`(`:599`)。`&mut ImeModel` を持つ任意の crate は今でも applied を書ける。守っているのは
  `ImeStateHub.shadow_model` が private であること(`platform_state.rs` 36-150 の struct、`shadow_model: ImeModel`)だけ。
- 移動後: `ImeStateHub`/`PlatformState` の**型**は新 crate にあるが、**本番のインスタンス**は awase-windows の
  `Runtime.platform_state`(private)にある。他 crate(awase-settings・awase-keymap-learn-win・e2e-uwp-inputsite-probe・
  統合テスト)は自前で作ったハブにしか record_* を呼べない。本番の applied は汚せない。
- 新 crate 自身(sync_actuation.rs は `&mut ImeStateHub` を受け取る)は本番のハブを書けるが、今も同じ crate 内なので
  同じ。`EXTRA_SRC_CRATES` に足せば RECORDERS が走査する(パス保持なので `src/state/platform_state.rs` のまま)。
- **DAG 上、新 crate より下の core 系 crate(ime-core・hook-core・plans など)は `ImeStateHub` を名指せない**
  (依存が逆向き)。ここからの漏れは構造上起きない。

結論: 本番の BUG-69 型の再発(本番の `applied` を actuation 以外が書く)に対する防御は**実質ほぼ変わらない**。
変わるのは「テスト・ハーネス・他 crate の自前インスタンスで applied を直接書けるようになる」こと。
これは 229 レビュー(`docs/adr/review/229-opus-visibility-policy.md:106`)が恐れた「RECORDERS が見ない crate の外から applied を書ける」
そのものだが、害は**閉ループ(`tests/support/harness.rs`)・リプレイが本番と違う経路で applied を作り、
シナリオの忠実度が黙って落ちる**ことで、本番の不具合ではない。

### F1-3 [dylint の実態・高] actuation_call_guard は RECORDERS の代わりにならない(補完にはなる)

`lints/actuation_call_guard/src/lib.rs` を読んだ結果:

| 性質 | RECORDERS(テキスト) | actuation_call_guard(dylint) |
|---|---|---|
| 許可の単位 | **ファイル**ごとの件数 | 呼び出し元の**関数名**(`check_fn` の `ident.name`、:131-148) |
| 件数 | 固定する | **数えない**(許可関数の中で 2 回目を足しても通る) |
| 呼び出し先の照合 | `.record_confirmed(` の文字列 | 識別子の**名前一致のみ**、DefId で型を見ない(:30-50) |
| UFCS `ImeStateHub::record_confirmed(hub, ..)`・`Self::record_confirmed(self, ..)` | **見逃す**(needle は `.` 始まり) | 拾う(`ExprKind::Call` + `last_segment_ident`、:166-171) |
| クロージャ・async ブロック内 | 行テキストなので拾う | 拾う(:183-196 で本体へ再帰) |
| 関数ポインタ化 `let f = ImeStateHub::record_confirmed; f(..)` | 見逃す | **見逃す**(`ExprKind::Path` は対象外) |
| 走査範囲 | awase-windows + `EXTRA_SRC_CRATES`(本番コードのみ) | `-p` に並べた crate のみ、`--tests` なし(ci.yml:191) |
| 同名の無関係な関数 | 誤検出 | 誤検出(lib.rs:30-38 が `e2e_windows.rs` の `set_ime_open` で実例を記録) |

- 草案 D3-1 の「許可呼び出し元(現在の RECORDERS の表と同じファイル)をリストで固定する」は**この lint では書けない**。
  ファイルではなく関数名で書くことになる。今の本番の呼び出し元関数は
  `kp_shadow_actuate`(key_pipeline.rs:1434)、`on_ime_apply_complete`(runtime/mod.rs:987)、`process_deferred_keys`(mod.rs:1459)、
  `presync_applied_open_on`(focus_tracking.rs:194)、`ir_post_focus_change_snapshot`(ime_refresh.rs:832)、
  `ir_apply_drift_correction`(ime_refresh.rs:1244)、核内部の `record_ime_apply_result(_in_scope)`。
  RECORDERS の doc コメント(`architecture_guard.rs` 2140 付近)は `kp_stage_shadow_ime_toggle` と書いているが実際の囲み関数は
  `kp_shadow_actuate` で、**名前のリストは今すでにずれている**。関数名の許可リストは同じずれ方をする。
- `ir_apply_drift_correction` のような長い関数が許可されると、その中に 2 本目の `record_optimistic` を足しても dylint は通る。
  件数は RECORDERS が見る。**両方残すのが前提**で、草案 D3 の 1・2 はその形になっているので、それ自体は妥当。
- dylint を足す本当の価値は UFCS の穴を塞ぐこと。これは**移動と無関係に今ある穴**なので、段階 2 を待たずに単独で入れる価値がある。

### F1-4 [草案が数えていない損失・高] 公開面の規律(FCIS P5a-1/S20)が崩れる

- 移動すると `record_*` だけでなく、`ImeStateHub`/`PlatformState` の `pub(crate)` **70 個**(本番部分の
  `pub(crate)` 出現数、`platform_state.rs` 1-2138 行)を `pub` にすることになる。`write_observer_poll`・`write_sync_key`・
  `write_physical_key`・`write_focus_probe`・`report_conv_open_inference`・`apply_ime_update`・`check_drift_correction` など、
  belief を書く口の全部が含まれる。
- `PLATFORM_STATE_PUB_FNS`(`architecture_guard.rs:7672-7687`、14 名)の趣旨は「`pub` はハーネスが呼ぶものだけ」。
  全部 `pub` になればこの集合は約 80 名になり、「pub = 例外」という規律は意味を失う。草案 D3-3 の
  「pub fn の集合の固定に使い回す」は、件数を固定する機能は残るが、**何を守っているか**は変わる(公開の例外の固定 →
  全メソッド名の棚卸し)。
- フィールド: shell は `.ime.belief` を 22 か所、`.ime.journal` を 27 か所、`.ime.event_log` を 2 か所で直接触る。
  移動後はこれらが `pub` フィールドか `&mut` を返すアクセサになる。`platform_state_pub_fns_are_fixed_and_exclude_recorders`
  (`:7837-7863`)の「`ImeStateHub`・`PlatformState` は pub フィールドを持たない」は**大声で落ちる**ので、書き換えが要る。
  `belief` を `pub` にするのは `.claude/rules/ime-belief-architecture.md` の方針に正面から逆らう見た目になる
  (害は F1-2 と同じ理屈で本番には小さいが、レビューでは毎回説明が要る)。
- `production_hub_is_unreachable_from_outside_the_crate` (1) は `pub(crate) ime: ImeStateHub` の文字列を assert している
  (`:7900-7903`)。移動で必ず落ちる(大声)。(2)(`Runtime.platform_state` が private)だけが本当の不変条件として残るので、
  (1) を消して (2) を主に書き直すのが正しい。草案はこのテストに触れていない。

### F1-5 [抜け穴・中] 新 crate の `-p` 足し忘れは黙る

- `tools/core-split/check_wiring.py` は `core-crates.txt` と `EXTRA_SRC_CRATES`・ci.yml の `-p` の一致を見る。
  しかし**新 crate を `core-crates.txt` に足し忘れたこと**はどこも検出しない(check_wiring は workspace の members も、
  awase-windows の path 依存も見ない。`layer_boundary_guard.rs:820-870` も一覧に載った crate しか見ない)。
- 足し忘れたとき: RECORDERS・`mode_key_passed_through` など `platform_state.rs` / `sync_actuation.rs` に**正の期待件数**がある
  ガードは大声で落ちる(良い)。しかし dylint は `-p` に無い crate を lint しない。`dispatch_set_open → apply_sync`(両方
  sync_actuation.rs)の RESTRICTED_CALLS の検査は**黙って外れる**。
- 対策: check_wiring に「awase-windows の `[dependencies]` の path 依存のうち、許可リスト(win32-async 等)以外は
  core-crates.txt に載っていること」を足す。これを段階 2 の前提条件にする。

### F1-6 [ローカル経路・中] 「cargo check だけで push」の漏れは今と同じ

- `.githooks/pre-push:134` は `cargo xwin check --target x86_64-pc-windows-msvc -p awase-windows` だけ。`mise run pre-push`
  も `cargo test --lib`(ルート crate のみ)で、architecture_guard も dylint も走らない。RECORDERS 違反は**今でも**ローカルを通る。
- develop にブランチ保護が無い(`gh api repos/cuzic/rust-nicola/branches/develop/protection` → 404、2026-10-21 確認)。
  草案の「dylint は CI の必須ジョブ」は**誤り**: 必須のチェックは無く、赤い CI のままマージできる。team-lead に
  自動マージを許している運用なら、これが今もこれからも一番弱い所。移動による差ではない。
- 移動で新しく増える漏れ: 他 crate(awase-replay 等)が新 crate に依存して record_* を呼ぶ。RECORDERS も dylint もその crate を
  見ない。害は F1-2 のとおり自前インスタンスに限られる。

### F1-7 より強い代替は?

- **(C) sealed トークンの変形**: `PlatformState::with_sources(..) -> (PlatformState, RecordKey)` で構築時に 1 回だけ鍵を払い出し、
  record_* に `&RecordKey` を要求、鍵は `Runtime` の private フィールドに置く。これで「鍵を作れるのは新 crate だけ」は書ける
  (草案の (C) 不採用理由「pub な構築子が要る」は、構築子が鍵を**返す**形なら当たらない)。ただし保証は
  「`Runtime` の private に届くコードだけが書ける」で、**今の `Runtime.platform_state` の private と同じ**。足しても何も増えない。勧めない。
- 実際に効く強化は 2 つで、どちらも安い:
  1. RECORDERS の走査を `crates/*/src` 全体(本番コード)に広げ、awase-windows と新 crate 以外での `record_confirmed`/
     `record_optimistic`/`record_ime_apply_result` の出現を 0 に固定する。F1-6 の「他 crate」の穴が塞がる。
  2. ハーネス(`tests/support/harness.rs`)で record_* を名指すことを禁じるテキストガード。閉ループの忠実度を守る
     (229 レビューが本当に恐れていたのはここ)。
- 上の 2 本と dylint 追加(F1-3)があれば、移動後の防御は**今より強い**と言える。

---

## (2) `crate::` 参照の全数と依存の判定

### 参照の内訳(本番コード、コメント中を含む grep の結果)

`platform_state.rs` の `crate::` は合計 161 件。所有 crate ごと:

| 参照 | 所有 crate |
|---|---|
| `state::foreground_scope`(31)、`state::ime_kind`(22)、`state::hub_clock`、`state::scoped_latch`、`state::event_origin`、`state::{ApplyGeneration,TickMs,GenerationAllocator}`、`focus::{AppKind,FocusKind}`、`keymap::KeymapTable`、`tuning::*`(MODE_KEY_PASS_MARK_WINDOW_MS ほか 6 定数)、`state::focus_gen` | awase-windows-base |
| `state::external_change_watch`(26)、`mozc_shm`(10)、`probe_admission`(9)、`eisu_candidate`、`conv_classify`、`intent_store`、`ime_update`、`ime_event`、`eisu_recovery`、`drift_correction`、`belief`、`evidence`、`force_guard`、`ime_event_log`、`ime_model`、`input_barrier`、`mode_key_pass`、`press_ledger`、`actuation_chain`、`open_warrant`、`observation_store`、`explicit_press`、`ime_actuation`、`journal::{JournalEntry,UnifiedJournal,DEFAULT_CAPACITY}` | awase-ime-core |
| `state::key_effect_predictor`(7)、`state::keymap_latch` | awase-key-effect |
| `state::hook_state::SyncKeyGate`(`GateStore` のみ、:2222)、`state::half_width_alnum`(`GateStore` のみ、:2232) | awase-hook-core |
| `focus::hwnd_cache::HwndImeSnapshot`(`apply_hwnd_cache_restore` の引数 1 か所、:1693) | awase-windows-plans |
| `crate::IME_DETECT_MISS_THRESHOLD`(:1636) | awase-windows の lib.rs:129 の再公開。実体は awase-windows-base の tuning.rs:265。新 crate では `crate::tuning::` に書き換えるか lib.rs で再公開が要る |
| `crate::hook::current_tick_ms`(:1133)、`crate::with_app`(:1418)、`crate::vk::is_ctrl_variant`(:986) | **doc コメントだけ**。殻への実依存ではない。intra-doc link(`[...]`)になっていれば新 crate の rustdoc で壊れるので確認が要る |
| `quanta`・`tracing`・(テストで)`serde_json` | 外部 crate。新 crate の Cargo.toml に必要 |

`#[cfg(windows)]` は `mod shell;`(:21-22)だけ。shell.rs の殻依存は `crate::win32::foreground_scope`(19)と
`crate::hook::current_tick_ms`(1)。**platform_state.rs 本体には `crate::win32`・`crate::hook`・`crate::observer`・`crate::tsf` の実コード参照は無い。**

`sync_actuation.rs` の `crate::`: `journal::JournalEntry`(ime-core)、`vk`(base)、`state::{TickMs,hub_clock,ime_kind}`(base)、
`state::{press_ledger,key_sequence_policy,ime_set_open_plan,ime_event,ime_actuation_decision,actuation_decision_record,actuation_chain}`(ime-core)、
`focus::class_names`(base)、`state::platform_state`(新 crate 内)。**殻依存なし。移せる。**ただし `pub(crate) trait CommandSink`(:48)、
`apply_sync`(:216)、`dispatch_set_open`(:301)、`mechanism_applicable` ほかは `pub` にする必要がある(殻の `ime_controller.rs` の
`ShellCommandSink` が `CommandSink` を実装するため)。

### F2-1 [草案の誤り・高] `ime_decision_view.rs` は移せない

- `use crate::tsf::observer::TsfObservations;`(:17)、`crate::tsf::observer::ActiveImeKind`(:52,68)、
  `crate::tsf::observer::candidate_was_seen()`(:87、グローバルを読む関数)。
- `tsf/observer.rs` は殻の `#[cfg(windows)]` モジュールで、`TsfObservations` は Atomic を持つグローバル観測、
  `ActiveImeKind` は `pub(crate) enum`(observer.rs:712)、`candidate_was_seen` は `pub(crate) fn`(:585)。
- `from_snapshot`(:85-94)は OS 観測のスナップショット化そのもの = 殻の仕事。
- しかも `platform_state.rs` も `sync_actuation.rs` も `ime_decision_view` を使っていない(sync_actuation は `DecisionInputs` を受ける)。
  **移す理由も無い。** D2・summary・背景表から外すべき。移したければ `ActiveImeKind` を base へ下ろし `from_snapshot` を殻に残す
  分割が要るが、153 行のためにやる価値は無い。

### F2-2 [依存の判定] 循環は起きない。awase-ime-core に入れると循環する(草案の「恐れ」は確定)

core 系の依存辺(各 Cargo.toml の `[dependencies]`):

```
awase-windows-base ← awase-key-effect ← awase-ime-core ← { awase-hook-core, awase-windows-plans }
```

- `platform_state.rs` は hook-core(GateStore)と plans(hwnd_cache)を使う。hook-core・plans はどちらも ime-core に依存する。
  ime-core に入れると ime-core → hook-core → ime-core で**循環が確定**する(deps.py で測るまでもない)。
- 新 crate `awase-ime-state` を最上段に置けば循環しない。必要な依存:
  `awase`、`awase-windows-base`、`awase-key-effect`、`awase-ime-core`、`awase-hook-core`、`awase-windows-plans`、`tracing`、`quanta`
  (dev: `serde_json`、ime-core の `testing` feature)。草案の列挙に `tracing`・`quanta` が抜けている。
- 依存を減らす選択肢: hook-core・plans への依存は `GateStore`(SyncKeyGate・HalfWidthAlnumState)と引数 1 つ(HwndImeSnapshot)だけ。
  `ImeStateHub` だけを移し `PlatformState`・`FocusStore`・`GateStore`・`KeymapStore`(2156-2335 行)を殻に残せば、依存は
  base・key-effect・ime-core・plans の 4 つになる。ただし 2,693 行のテストは `PlatformState::for_test` を使うので分割が要る。

### F2-3 [見落とし・高] 孤児規則で構築口が書けなくなる

- shell.rs:22-48 は `impl ImeStateHub { fn new() }`・`impl PlatformState { pub fn new() }`・`impl Default for PlatformState` を
  **殻の crate で**書いている。型が新 crate に移ると、inherent impl も `Default`(外部トレイト × 外部型)も**孤児規則で書けない**。
- 構築は新 crate の `PlatformState::with_sources(clock, scope_source)` にし、殻は自由関数(または bootstrap.rs:747 で直接)で
  実時計と `win32::foreground_scope` を渡す形になる。`Default` 経由の構築が他にあれば全部書き換え。
- これに連動して、時計のガード群(`with_clock_call_violations` :1662-1720、`REAL_CLOCK_CALL` :1650、`effective_open` の
  `HubClock::wall(crate::hook::current_tick_ms, Instant::now)` 件数固定 :1450-1460)は SHELL のパスを前提にしているので書き直しになる。
  多くは `read_crate_file` の panic か「許可リスト外」で大声で落ちるので、黙る危険は小さいが、量は草案の想定外。

### F2-4 [テスト用の口・中] `#[cfg(test)]` の口が crate 境界で消える

- `platform_state.rs:2139-2150`(`set_desired_open_for_test`・`clear_last_intent_for_test`)と `:2324-2335`(`PlatformState::for_test`)は
  `#[cfg(test)]`。新 crate に移ると、**awase-windows のテストをビルドするときには cfg(test) が立たない**。今は殻の外から
  呼ばれていない(grep で 0 件)が、`runtime/` 配下の `#[cfg(windows)]` のユニットテストから将来呼ぶと、Linux では見えず
  Windows CI でだけコンパイルエラーになる(CLAUDE.md が警告する型の穴)。ime-core と同じく `feature = "testing"` で出すこと。

---

## (3) scope 注入(`fn() -> ForegroundScope`)の落とし穴

### F3-1 [設計の反転・高] D1 は ADR-229 FCIS P2 を反転する。草案に書かれていない

- shell.rs:1-8 の doc と `shell_shape_violation`(architecture_guard.rs:2230-2262)は「核は scope を引数で受け取る純粋な関数、
  OS を読むのは殻で 1 回だけ」を固定している。D1 は核の中から関数ポインタ経由で OS を読む形に戻す。
  `HubClock::wall` は前例だが、時計は「読むたびに違ってよい」ものなのに対し、scope は**1 操作の中で一定でなければならない**値。
- 具体例: `record_ime_apply_result_in_scope`(:1488-1531)は同じ `scope` を `note_awase_open_write` と `record_confirmed_in_scope`
  の両方へ渡す。`expire_mode_key_pass_mark_in_scope` → `drop_intents_for_mode_key_pass_in_scope` も同じ。
  草案の「`_in_scope` を同名メソッドに統合し内部で `(self.scope_source)()` を読む」を素直にやると、外側と内側で**2 回読む**。
  同じスレッドの数 µs の間でも前景は変わりうる(GetForegroundWindow は別プロセスの状態)ので、「1 呼び出し 1 回読み」(草案 D1 第 1 項)は
  自動では守られない。内部用の `_with_scope(scope)` を残す必要があり、15 個がそのまま消えるわけではない。
- その「1 回読み」を今は `shell_shape_violation` が機械的に固定している。D1 でこのガードを消すと、**核の任意のメソッドが
  何回でも OS を読める**状態になり、それを固定するガードが無くなる。代わりのガード(例:「`scope_source)()` の出現は
  公開メソッドの先頭の 1 か所ずつ」)を書かないと、ここが「黙って外れる検査」になる。

### F3-2 [テストの偽スコープ・高] 関数ポインタはキャプチャできない

- `fn() -> ForegroundScope` は値を閉じ込められない。ハーネス(`tests/support/harness.rs:218` の定数 `WATCH_SCOPE`)は
  定数を返す関数で済む。しかし `platform_state.rs` のテストには**1 テストの中で scope を切り替える**ものがある:
  `conv_read_ok_is_per_scope_and_expires_on_focus_change`(:5013-5029、`a`・`b`・`INVALID`)、:2440 の `other`、:2679-2680。
  関数ポインタでやるには `thread_local!` の可変セルが要る。`static` にすると `cargo test` の並列スレッドで競合する。
- 推奨: `HubClock` と同じく enum にする。`ScopeSource::Os(fn() -> ForegroundScope)` / `ScopeSource::Fixed(ForegroundScope)` と、
  `#[cfg(any(test, feature = "testing"))] fn set_scope_for_test`。ただしこれは「scope を書き換える口」を新設することになり、
  時計と同じく「本番で書き換えない」ガード(`count_clock_writes` 相当)が要る。
- `Send`/`Clone`/`Debug`(草案の未決 2): `fn` ポインタは `Debug`・`Copy`・`Send`・`Sync`。`ImeStateHub` は `Debug` だけ derive
  (:35)で、`Cell` フィールドで既に `!Sync`。ここは困らない。困るのは上の「キャプチャできない」と次の F3-3。

### F3-3 [本番で黙って壊れる・高] 既定値の偽スコープが本番に漏れる

- 草案は構築口の形を決めていない。テストの便宜で `with_clock(clock)` が scope を `|| ForegroundScope::INVALID` 既定で
  埋める形にすると、本番が誤ってその構築口を通ったときに:
  `note_conv_read_ok_in_scope(INVALID)` は記録しない(:5022-5026 のテストが示す)→ `conv_read_ok_now` が常に false →
  ADR-260 決定 1 が黙って無効、通過マーク・外部変化の監視窓も INVALID スコープに紐づく。**どのテストも落ちない。**
- 今は `with_clock_call_violations`(:1662-)が「本番の `with_clock(` は shell の実時計 1 件だけ」を固定し、`REAL_CLOCK_CALL` で
  実時計の式まで照合している。scope にも**同じ強さのガード**(本番の構築は `win32::foreground_scope` を渡す 1 か所だけ)が必須。
  草案の「-100(guard)行」はこの追加を数えていない。

### F3-4 [移行量・中] 実数

- `_in_scope(` の呼び出し: `platform_state.rs` のテスト部分に **90 件**、ハーネスに 2 件(:490,511)。
- ガード: `PLATFORM_STATE_PUB_FNS` に `arm_external_change_watch_in_scope`・`follow_external_change_in_scope`(:7676,7680、大声で落ちる)。
  `split("fn follow_direct_read_in_scope").expect(..)` 系(:4845,4901,4914,9097、`expect` で大声)。shell の形の 3 テスト(:2230-2320、削除)。
  時計の 4〜5 テスト(SHELL パス前提、大声)。
- **黙って外れるもの**: `external_change_watch_has_single_arm_and_follow_sites`(:4657-4705)の `_in_scope` の 4 エントリ。
  許可ファイルが `state/platform_state/shell.rs` で、shell.rs が消えて名前も消えると、全ファイルで件数 0・期待 0 になり**常に通る死んだ検査**に
  なる。ADR-274 の教訓そのもの。D1 の PR でエントリを消すか、期待を「核のどこで読むか」に置き換えること。
- RECORDERS の `.record_*_in_scope(` の needle も、名前が消えると死ぬ(SHELL の期待 1 が消えるので一度は大声で落ちるが、
  期待を直した後は needle が死んだまま残る)。

### F3-5 [より安い D1] `_in_scope` を残す形

- 核の `_in_scope`(純粋・引数で scope)を API として残し、`scope_source` を読む薄い公開メソッドを**同じファイル**に置く形なら、
  テスト 90 件もハーネスも無変更、内部合成の「1 回読み」も今のまま。shell.rs の中身が platform_state.rs に移るだけで、
  削減は shell.rs の重複 doc とファイル 1 枚(数十行)程度。行数は減らないが、段階 2 の孤児規則問題(F2-3)の半分(scope 側)は解ける。
- 草案の「統合して約 -180」を取るなら F3-1〜F3-4 の費用を払う。取らないなら上の形。どちらにするかを ADR に書くべき。

---

## (4) 段階 1 で止める案の妥当性と、段階 2 の費用対効果

### 段階 2 で得られるもの(行数以外)の棚卸し

| 得られるもの | 移動しないと得られないか |
|---|---|
| core 系の行数比率(40.9% → 約 48%、platform_state 5,030 + sync 757 = 5,787 行) | はい。ただし指標であって性質ではない |
| platform_state を Linux でテストできる | **いいえ。今すでにできる**(`state/mod.rs:16-18` は `cfg_attr(not(windows), allow(dead_code))` で、gate は shell だけ。sync_actuation も `cfg(any(windows, test))`) |
| mutation testing の対象にできる | **いいえ。** `.cargo/mutants-awase-windows.toml:18` のコメントは「platform_state.rs は cfg(windows) なので対象外」と書くが**古い**(2,337 行の本番が Linux でコンパイルされる)。`examine_globs` に 1 行足せば今日から対象。IME belief ファミリーの中心が mutation の対象外のまま、というのが本当の穴 |
| 「OS に触らない」を Cargo で保証 | 半分。今は `layer_boundary_guard.rs` の `CORE_MODULES`(:587 `platform_state`、:596 `sync_actuation`)がテキストで保証。Cargo 保証は強いが、D1 後は関数ポインタ経由で OS を読むので「OS に触らない」の意味自体が弱まる |
| awase-windows の増分ビルドの短縮 | はい(runtime/ を変えても platform_state を再コンパイルしない)。ただしローカルでビルドしない運用(メモリ: 検証は GitHub Actions)では体感の利得は小さい |
| 依存の向きの強制(platform_state が runtime/ を呼べない) | はい。ただし今 platform_state.rs に `crate::runtime` 参照は 0 件で、違反の実績が無い |

### 費用(草案が数えていないもの)

- 孤児規則による構築口の作り直し(F2-3)と時計ガードの書き直し。
- `pub(crate)` 70 個の `pub` 化、S20 の公開面の規律の消失、`belief`・`journal`・`event_log` の公開(F1-4)。
- `production_hub_is_unreachable_from_outside_the_crate` (1) の書き直し(F1-4)。
- `CommandSink` など sync_actuation の `pub` 化。
- `#[cfg(test)]` の口の `testing` feature 化(F2-4)。
- check_wiring の新ルール(F1-5)を先に入れないと dylint が黙る。
- 草案の「`src/state/platform_state.rs` 直読み約 20 箇所のパス更新」は、ADR-274 のパス保持方式(`EXTRA_SRC_CRATES` 経由で
  `src/...` のまま解決、`resolve_crate_paths` :52-72)なら**ほぼ 0 件**のはず。逆に「パスを書き換えなくてよい」ことを確認する
  作業(112 件の参照の点検)が要る。草案の見積もりの根拠が逆。

### 判断

- **段階 2 は見送りを推奨する。** 移動しないと得られない性質は「行数比率」「増分ビルド」「依存の向きの Cargo 保証」で、
  前 2 つは指標・体感、最後は違反実績が 0。費用は INV-A97-1 の再設計ではなく、**公開面の規律の崩壊と構築口の作り直し**が主で、
  草案はそこを見ていない。
- **段階 1(D1)も草案の形(統合)なら見送り寄り。** 得られるのは約 -150 行(shell.rs 223 行 − 残る構築口 − 新しい scope ガード)で、
  FCIS P2 の反転・偽スコープの落とし穴・死んだ検査 4 件のリスクを負う。やるなら F3-5 の「`_in_scope` を残す」形にし、
  scope の構築口ガードを同じ PR で入れる。
- **代わりに独立の小 PR を勧める**(どれも移動と無関係に価値があり、段階 2 を将来やるときの前提にもなる):
  1. `.cargo/mutants-awase-windows.toml` に `state/platform_state.rs` を足し、古いコメントを直す(mutants の 1 回の実測を添える)。
  2. `lints/actuation_call_guard` に `record_optimistic`・`record_confirmed`・`record_ime_apply_result` を足す(UFCS の穴を塞ぐ)。
     許可は関数名で: `kp_shadow_actuate`・`on_ime_apply_complete`・`process_deferred_keys`・`presync_applied_open_on`・
     `ir_post_focus_change_snapshot`・`ir_apply_drift_correction`・`record_ime_apply_result(_in_scope)`(と shell の各ラッパー名)。
     同時に RECORDERS の doc の古い関数名(`kp_stage_shadow_ime_toggle`)を直す。
  3. RECORDERS の走査を workspace 全体の本番コードに広げる + ハーネスで record_* を名指さないガード(F1-7)。
- もし所有者が行数比率を目標として段階 2 を選ぶなら、前提条件は: F1-5 の check_wiring 拡張、F2-1(ime_decision_view を外す)、
  F2-3 の構築口の設計、F1-4 の `production_hub_is_unreachable` の書き直し方針、F2-4 の `testing` feature、を ADR に書いてから。

## 草案本文の個別の誤り(直すべき箇所)

1. summary・背景表・D2: `ime_decision_view.rs` を移動対象に含めている → 移せない(F2-1)。
2. D3 最終段落: 「コンパイラ強制(違反=ビルドが通らない)」→ 呼び出し元の固定は今も CI(F1-1)。
3. D3-1: 「現在の RECORDERS の表と同じファイル」→ dylint はファイル単位で許可できない、関数名(F1-3)。
4. D3-1: 「dylint は CI の必須ジョブ」→ develop にブランチ保護が無い(F1-6)。
5. D2 の依存列挙に `tracing`・`quanta` が無い。「循環する恐れ」は「循環する」と確定してよい(F2-2)。
6. D1: FCIS P2(ADR-229)の反転であることが書かれていない。関連 ADR にも、`docs/adr/review/229-opus-visibility-policy.md` への参照も無い。
7. D4: 「約 20 箇所のパス更新」はパス保持方式なら不要のはず(根拠の確認が要る)。
8. 影響・リスク: 孤児規則(F2-3)、公開面 70 個(F1-4)、`#[cfg(test)]` の口(F2-4)が抜けている。
9. 検討した代案 (C): 「pub な構築子が要る」は、構築時に鍵を返す形なら当たらない。不採用の本当の理由は
   「`Runtime` の private と同じ保証しか増えないから」(F1-7)。
