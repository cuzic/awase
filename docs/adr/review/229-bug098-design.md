---
id: ADR-229-companion-bug098-design
title: |-
  BUG-098 修正の設計(調査のみ。Opus は案 D を推奨)
type: companion-doc
related_adr:
  - "ADR-229"
  - "ADR-108"
---

# BUG-098 修正の設計(調査のみ・コード未変更)

基準: `origin/develop`(efe5248e 時点で fetch 済み)。行番号は `git show origin/develop:<path>` のもの。

## 0. 要点(先に読む)

1. **BUG-098 の本文(「generation=None のまま epoch ゲートを通らない」)は、v2.0.0 時点の記述として部分的に古い。**
   `cf48bc67`(ADR-213 P2a、2026-10-01、PR #408 の Opus M-3)で、shadow toggle の ImmCross 非同期経路には
   **future の中の「書き込み後の focus 世代照合」**が既に入っている(`key_pipeline.rs:1351-1359`)。
   known-bugs の「2026-10-04 現行コードに残存を確認」は `None` を渡している事実は正しいが、この緩和に触れていない。
2. それでも**穴は 3 つ残る**(§2): (G1) 照合から WM 到着までの窓、(G2) `with_app` 再入時の「一致扱い」、
   (G3) 世代が進む時点が「フォーカス検出後」でしかなく、検出前の窓は世代では原理的に見えない(これは直せない)。
   実害は G1・G2 を閉じることで減る。G3 は残余リスクとして明記する。
3. **推奨案(A)**: 照合を**完了ハンドラ側**(`handle_wm_async_ime_apply_complete`、`&mut Runtime` を持つ唯一の合流点)へ移す。
   spawn 時の `focus_gen`(既存の `Output::ime_mode_focus_gen`、F-D5-2 の表の `focus_gen`)を WM に載せ、
   ハンドラで現在値と比べて、不一致なら `UnsafeToToggle` に落とす。**新しい世代・id は作らない。**
4. 再現は **Linux では不可能**(`runtime/` は丸ごと `#[cfg(windows)]`、閉ループの harness は async 完了を写していない)。
   確実に書けるのは「純関数に切り出した判定」の host 単体テストと、windows-build の lib テスト、source-scan ガード。
   実機での再現は、現状は G1 の窓が数 ms なので**期待しない**(推測で直さない: §1)。
5. 所有者に仰ぎたい最大の点: **「実害が実機で観測されたか」が記録に無い**(報告・journal 無し、BUG-098 は ADR レビュー由来の理論上の残存)。
   M-3 で既に主窓は閉じているので、本修正は「G1・G2 の小さな窓を閉じ、ガードで固定する」整理としての価値が主。優先度の確認が要る。

## 1. 症状と再現

**症状(平易に)**: IME を明示的に閉じる(または開く)ための非同期の書き込み(ImmCross の `spawn_local`)を始めたあと、
その完了が届くまでにユーザーが Alt+Tab で別アプリへ移ると、旧ウィンドウ宛ての完了が新ウィンドウの文脈で処理される。
その結果 (a) 新ウィンドウの `applied` が `Confirmed{open:false}` になる(実際は新窓の IME は触っていない)、
(b) 新窓で `on_ime_applied` が composition の cold-mark を行う。以後、「閉じたはず」という誤った記録を根拠に省略・補正が誤判断しうる
(BUG-113 の `shadow_on` の罠・BUG-141 型の「記録だけ先に進む」と同型)。

**経緯**: ADR-108 決定6(2026-09 初頭)が「generation 付き経路だけ epoch ゲート」と決め、`generation=None` の 5 経路のうち
非同期の 1 つ(shadow toggle OFF の ImmCross)を残存ギャップとして BUG-098 に起票。ADR-213 P2a で shadow toggle ON も同じ async 経路になり
(`key_pipeline.rs:1287-1366`)、対象が ON/OFF の 2 つに増えると同時に `cf48bc67` が M-3 照合を足した。

**再現できるか**:

| 層 | 可否 | 理由 |
|---|---|---|
| 実機(Alt+Tab のタイミング) | 期待しない | await の長さは `run_open_chain_async` の IMM 呼び出し(SendMessageTimeout 系、通常は数 ms〜数十 ms)。G1 の窓は「照合 → WM 取り出し」の同一スレッド内の数 ms で、フォーカス検出(50ms デバウンス、`ime_refresh.rs`)が割り込む確率は低い。journal に該当症状の報告が無い |
| Linux 閉ループ `tests/closed_loop_scenarios.rs` | 不可 | `ImeStateHub`・`runtime/` が見えない層(同ファイル冒頭の表)。async 完了(WM)は写されていない |
| `tests/journal_replay.rs` | 不可 | `classify_*` の純関数への入力列。WM の到着順(照合 → focus 世代の bump → ハンドラ)は journal に載らない |
| `tests/warmup_gate_focus_scope.rs` | 不可 | warmup のフォーカス scope 用 |
| `ImeModel` の host 単体テスト(`state/ime_model.rs`) | **generation 付きの epoch 照合だけ可** | `ImeApplyRequested → FocusChanged → ImeApplySucceeded` で Stale になる既存テスト群がある(2669 行〜)。generation=None の経路は `ImeStateHub::record_ime_apply_result_in_scope`(gated)が `confirm_applied` を直接呼ぶ |
| windows-build の lib テスト(`runtime/`・gated) | 可(CI のみ) | ハンドラへ直接 `(wparam,lparam)` を与えて `Runtime` の状態を検査する形。ただし `Runtime` の構築コストが要る(既存の `runtime/` テストの流儀を要確認) |

→ **「再現しない」を前提に、純関数 + ガードで固定する**。実機再現は要求しない(`fix-requires-evidence.md` は (a) 回帰テストか (b) known-bugs 記録のどちらかでよい)。

## 2. 原因(コードで裏取り)

### 2-1. 経路の全体

`kp_shadow_actuate`(`runtime/key_pipeline.rs`、imm_first 分岐 1287-1366):
```
issue_actuation_order(...)                         // spawn の外(ADR-090 §4.2)
let focus_gen = output.ime_mode_focus_gen.get();   // :1304  spawn 時点の世代を捕獲
spawn_local(async {
   ON : ActuationTarget::capture(focus_gen).await  // 書き込み側は target.verify_still_current が live hwnd + gen を照合
   OFF: ImmCrossOp::Untargeted                      // :1337  宛先を捕獲しない(Phase C 未移行) ← 書き込み側の照合が無い
   run_open_chain_async(...).await                  // :1344
   // PR #408 M-3: with_app で gen を読み直し、不一致なら UnsafeToToggle  (:1354-1359)
   post_async_ime_apply_complete(open, outcome, None, ShadowToggle)   // :1362  generation=None
})
```
→ `message_handlers.rs:841 post_async_ime_apply_complete` → WM → `message_handlers.rs:908 handle_wm_async_ime_apply_complete`
→ `runtime/mod.rs:920 on_ime_apply_complete` → `ImeStateHub::record_ime_apply_result`(`platform_state.rs:1056 record_ime_apply_result_in_scope`)。

### 2-2. どの完了がどのゲートを通らないか

- `generation=None` のとき `record_ime_apply_result_in_scope` は**先頭で分岐**して `apply_result_effective_open` → `record_confirmed_in_scope` →
  `ImeModel::confirm_applied`(`ime_model.rs:536`)を呼び、`Accepted` を返す(`platform_state.rs:1057-1066`)。
  `ImeModel::classify_apply_completion` → `completion_can_update_applied`(`ime_model.rs:555-593`)の **epoch 照合(`pending.focus_epoch == current_epoch`)は generation 付きの完了にしか掛からない**。
- shadow toggle は `ImeApplyRequested` を dispatch しない(`pending` を立てない)ので、そもそも epoch 付きの `ImeTransition` が存在しない。
  ここへ epoch を載せるには、(i) `ImeApplyRequested` を足す(案 B)か、(ii) 別の世代で守る(案 A: `ime_mode_focus_gen`)しかない。
- `Accepted` を受けると `Runtime::on_ime_apply_complete`(`mod.rs:~960`)の `drives_composition_side_effects` が真になり、
  `Platform::on_ime_applied` が新窓で cold-mark を行う。

### 2-3. 既存ゲートと、なぜ迂回されるか

| 既存ゲート | 場所 | shadow toggle(None)で効くか |
|---|---|---|
| `ime_mode_focus_gen`(`Output`、`on_ime_mode_focus_changed` が +1。`platform.rs:632`、呼び元は `ir_post_focus_change_snapshot`=`ime_refresh.rs:253`) | 書き込み前後 | ON: `capture`/`verify_still_current`(`ime.rs:1067`)が書き込み直前に照合(**書き込み**は守る)。OFF: `Untargeted` なので書き込み前は無し。完了側は M-3(`:1354`)のみ |
| `ActuationTarget::verify_still_current`(`ime.rs:1067`) | 書き込み直前 | ON のみ。**完了の記録**は対象外 |
| ADR-104 `FocusFence`/epoch(`ObservationStore::current_fence().epoch`) | `ImeTransition.focus_epoch`・観測の `AcceptedObservation` | `pending` が無いので不参加 |
| ADR-108 epoch ゲート(`completion_can_update_applied`) | generation 付き完了 | `None` 経路は先頭の分岐で素通り |

### 2-4. M-3 でも残る穴

- **G1(照合 → WM 取り出しの窓)**: 照合は future 内(`:1354`)、ハンドラは `post_to_main_thread_with` した WM をメインループが後で取り出す。
  この間にフォーカス処理(`run_ime_refresh_with_prefetched` → `ir_post_focus_change_snapshot` → gen +1)が先に走れば、照合を通った完了が新窓の文脈で処理される。
  同じスレッド(`spawn_local` もメインスレッド)なので窓は「キューに積まれた順」で決まる。通常は数 ms だが 0 ではない。
- **G2(再入時は一致扱い)**: `with_app(...).is_some_and(|g| g != focus_gen)` は再入で `None` のとき `false`(=一致扱い)(`:1354-1359`、コメントにも明記)。
  `with_app` を握っている最中(フォーカス処理・タイマーハンドラ中の `spawn_local` の poll 等)にこの future が進むと照合が素通りする。
  ハンドラ側は `&mut Runtime` を引数で受けるので**再入がそもそも起きない**(これが案 A の最大の利点)。
- **G3(検出前の窓: 直せない)**: gen は「awase がフォーカス変更を検出した後」にしか進まない(`ir_post_focus_change_snapshot`)。
  Alt+Tab で OS のフォーカスが動いてから検出(デバウンス)までの間に始まった/完了した書き込みは、どの世代でも見えない。
  OFF は `Untargeted` なので書き込み先自体も「その時点のフォーカス」で決まる(`ImmCrossOp::Untargeted`)。
  これは Phase C(OFF の宛先捕獲、ADR-086 INV-14)の課題で、本修正の範囲外(§7 で所有者に確認)。
- (**要確認 U1**) `ime_mode_focus_gen` の bump(`ir_post_focus_change_snapshot`、refresh の最後)と、`ImeModel` の epoch(`FocusChanged` dispatch、
  `focus_tracking.rs:on_focus_process_changed`)・`FocusStore::focus_epoch`(`enter_focus_scope`:176)は**別カウンタ**で、bump の時点も違う可能性がある。
  ADR-108 は「`FocusStore` の epoch と混ぜるな」と明記。案 A は `ime_mode_focus_gen` だけを使うので混ぜない。
  なお `ime_mode_focus_gen` は **同一プロセス内の hwnd 変更でも**進む(プロセス変更でのみ進む epoch より厳しい)→ §4 のリスクに反映。

## 3. 修正案

### 案 A(推奨): 完了ハンドラで照合する(既存の `focus_gen` を WM に載せる)

**変更箇所**:

1. `message_handlers.rs`
   - `post_async_ime_apply_complete` に `focus_gen: Option<u32>`(照合しない呼び元は `None`)を足す。
     または別関数 `post_async_ime_apply_complete_with_focus_gen` を足して既存 3 呼び元(`executor.rs:825/855`、`key_pipeline.rs:1322`)は不変にする(**差分を小さくするなら後者**)。
   - 載せる場所: `lparam`(`isize`、64bit)。現状は `encode_outcome` の 0〜7 だけ。`lparam = outcome | (1 << 8)[有効フラグ] | ((focus_gen as isize) << 9)`。
     `wparam` の generation 欄は shadow toggle では未使用(0=None)だが、**意味を混ぜない**ために使わない(F-D5-2: 世代は由来ごとに別)。
     `decode_outcome` は下位 8 bit だけを見る形に変える(`encode_outcome`/`decode_outcome` は網羅 match なので、テストを足して往復を固定)。
   - `handle_wm_async_ime_apply_complete`(:908)で、有効フラグが立っていれば
     `app.platform.output.ime_mode_focus_gen.get() != wire_gen` のとき `outcome = UnsafeToToggle` に落としてから `on_ime_apply_complete` を呼ぶ。
     (判定は純関数 `state/` 側に `shadow_completion_outcome(outcome, spawn_gen, now_gen) -> ImeOpenOutcome` として切り出す。host で単体テスト可能にする。)
2. `key_pipeline.rs:1351-1359` の future 内照合(M-3)を**削除**(ハンドラが唯一の照合点)。再入で素通りする側を残すと「2 箇所に同じ判定」になる。
   - ただし**削除するかは Opus に確認**: 残すと二重判定(安全側だが、再入時の挙動が不一致)、削除すると architecture_guard の source-scan(`with_app` の文字列を探すガードがあれば)に影響しうる(U2: guard の該当を grep で確認)。
3. `key_pipeline.rs:1362` と ON の capture 失敗(`:1322`、`UnsafeToToggle` 固定なので照合不要)のうち、`:1362` だけ新関数に切り替える。

**なぜこれが「既存の世代を使う」か**: `focus_gen` は ADR-229 FCIS 改訂の対応表にある既存の世代(`ime_mode_focus_gen`・`ActuationTarget::verify_still_current` と同じカウンタ)。
新しい世代は発明せず、「実行結果は既存の世代を再利用して由来を示す Event として戻し、古い結果は core が捨てる」(F-D5-2)をそのまま実装する。

**副次効果**: 「`last_explicit_ime_action_ms` を `await` をまたいで手書きで比較する」(inventory-w0-c、`key_pipeline.rs:425/664`)とは別物(あちらはタイムスタンプ)。
本案はあの 2 箇所の置換ではない(それらは別途、世代化を検討するが BUG-098 の範囲外)。

### 案 B: shadow toggle にも `ApplyGeneration` を払い出し、generation 付き経路へ合流(BUG-098 本文の follow-up 方針の 2 つ目)

- `allocate_event_generation()` → `ImeApplyRequested` を dispatch → `generation` を `post_async_ime_apply_complete` に渡す。
  `completion_can_update_applied`(epoch 照合)がそのまま効く(プロセス変更で Stale)。`ime_mode_focus_gen` は使わない。
- **却下理由**: (1) ADR-108 決定6 が懸念した pending 解放の意味論変更を**そのまま持ち込む**(棄却された完了が `clear_pending_if_matches` を呼ばない)。
  (2) `reduce_ime_apply_requested`(`ime_model.rs:1060-1100`)は pending 上書きログ・`last_seen_generation`(watermark)・**`!target && ctrl_held` で chord barrier を立てる副作用**を持つ。
  shadow toggle OFF は物理 IME キー押下由来で `ctrl_held` の扱いが Engine 経路と異なり、BUG-46 型の二重 barrier を作る恐れ。
  (3) 既に `record_confirmed(false, ...)` を pre-actuation で書いている(`:1288`、ADR-098 決定5/6-a)ので、`pending` を足すと「pre-record と pending の二重管理」。
  (4) epoch は「プロセス変更でのみ進む」ので、同一プロセス内の別 hwnd への移動を検出できず、M-3 の `focus_gen` より**緩い**(後退)。
  → 変更量が大きく、利益が案 A より小さい。

### 案 C(却下): 世代を持たせず、受け取り側で「現在のフォーカス」と照合する

- ハンドラで現在の hwnd/プロセスを読んでも、「この完了がどの窓向けだったか」を持たないので、A→B に移ったあとの A 向け完了と、B 向けの完了を区別できない。
  `applied` を書く対象が「現在の窓」かを問う限り、書き込み先の記録(spawn 時の世代)が要る → 案 A に帰着。却下。

### 案 D(最小): コードは変えず、BUG-098 の記録を M-3 反映に更新し、G1/G2 を明記して「許容」とする

- 実害の報告が無い(§1)ので、修正を見送る選択肢。リスク: G2 は再入頻度が不明(未測定、U3)。
  所有者が「実害ありの根拠が無いなら不要」と判断するなら採れる。ただし所有者の決定(F-D5-2 で直す)は案 A/B を前提にしている。

**比較**: A は変更箇所が 1 つのハンドラ + 純関数 + WM の載せ方で、既存の世代を再利用、再入を構造的に避け、M-3 の緩和を維持/強化する。
B は意味論変更が大きく、C は成立しない。→ **A を推奨**。

## 4. 挙動の変化と回帰の恐れ

**捨てられるようになる(=記録を動かさなくなる)完了**:
- 照合(`:1354`)を通ったが、ハンドラの取り出しまでに focus 世代が進んだ完了(G1)。
- 照合時に `with_app` が再入で `None` だったために素通りしていた完了(G2、A で必ず照合される)。
→ どちらも `UnsafeToToggle`(= `NotSent`: `applied` を書かない・composition 副作用を駆動しない・`post_ime_refresh` は必ず走る(`mod.rs`、E))。

**捨てられていたものが適用されるようになる**: 無い(A は照合を強めるだけ、緩める箇所は無い)。

**「正しく適用されるべき完了を誤って捨てる」リスク(BUG-141 型)**:

- R1: **同一プロセス内の hwnd 変更**(例: Chrome 内の別ウィジェット、IME 入力欄へのフォーカス移動)でも `ime_mode_focus_gen` は進むので、
  待機中にそれが起きると完了が捨てられる(M-3 でも同じ。ハンドラ化で G1 の分だけ増える)。
  影響: 旧窓へ実際に書いた IME 状態が `applied` に反映されない。`UnsafeToToggle` なので **belief は pre-actuation の `record_confirmed(false)`(:1288)のまま**
  (OFF のとき)で、フォーカス変更後の refresh(`post_ime_refresh`)が実状態を観測して補正する。ON は pre-record 無し→ 観測待ち。
  → **「押下が握りつぶされる」ことは無い**(書き込み自体は既に済んでおり、完了の記録だけの話)。BUG-141 の型(省略が押下を握りつぶす)とは向きが逆。
- R2: **press 台帳(ADR-208 L1)**: `claim_press_write` は async の完了で解かない(`:1378` の `release_press_write` は sync 分岐のみ)ので、完了を捨てても台帳の状態は変わらない。確認要(U4: `kp_shadow_actuate` の press 予約の解放条件)。
- R3: **`OutputActiveGuard`(`drop(guard)`)**: 照合・投函の位置を変えても `drop(guard)` は WM 投函の後のまま(ハンドラ側の判定に移すだけ)。ガードの寿命は変えない。
- R4: **defer/replay キュー(ADR-156)**: 完了ハンドラは `post_ime_refresh` を呼ぶだけで defer キューに触れない。窓口の増減なし。
- R5: **actuation 合流点(ADR-119)**: 新しい gate を足す形だが、これは「実行」の gate ではなく「完了の記録」の gate。`run_open_chain_async`・`ImeController::apply` の呼び出し元は増えない。
  `architecture_guard.rs` の `.apply_ime_open_with_view(` 件数(=2)・`run_open_chain_async` の呼び出し箇所(1 つ)は不変。
- R6: **lparam の幅**: 32bit ビルドでは `isize` が 32bit。awase は x86_64 のみ(`x86_64-pc-windows-msvc`)だが、`focus_gen: u32` を `<< 9` で詰めると 41bit 要る。→ 64bit 前提を `const_assert!(size_of::<isize>() == 8)` で固定するか、`focus_gen` を下位 16bit に畳む(wrapping_add なので 65536 回の bump で衝突。実用上は十分だが、**衝突=誤って一致扱い**なので 64bit 前提+assert を推奨)。U5。

## 5. 回帰テスト

fix-requires-evidence の再発ファミリー: focus 遷移 / IME actuation 合流点 / IME belief に該当。(a) テストで満たす(実機再現不可のため (b) だけでは弱い。(b) の更新も併せて行う)。

| # | テスト | 層 | 実行場所 |
|---|---|---|---|
| T1 | `shadow_completion_outcome(outcome, spawn_gen, now_gen)` の表: 一致→不変、不一致→`UnsafeToToggle`、`None`(照合しない)→不変、wrapping(`u32::MAX`→0)で不一致 | `state/`(ungated の純関数。置き場は `state/ime_actuation_decision.rs` か `state/press_ledger.rs` の隣、**要確認**)の `#[cfg(test)]` | host(`cargo test --lib`系。CI の `nextest --workspace --lib`) |
| T2 | `encode_outcome`+focus_gen の往復(全 outcome × gen の代表値 × 有効/無効フラグ、`decode` が元に戻る)。`decode_outcome` の「下位 8 bit だけを見る」ことを固定 | `runtime/message_handlers.rs` の `#[cfg(test)]`(**gated**) | windows-build のみ(`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib` でコンパイル確認、実行は CI の windows-build) |
| T3 | ハンドラ統合: `Runtime` を作り、`ime_mode_focus_gen` を +1 してから `handle_wm_async_ime_apply_complete` に `(open=false, Applied, gen=古い)` を与え、`applied` が `Confirmed{false}` にならないこと・`on_ime_applied` が呼ばれないことを検査 | `runtime/` の lib テスト(gated) | windows-build のみ。`Runtime` 構築の既存フィクスチャ有無は未確認(U6)。無ければ T1+T2+T4 で代替し、T3 は見送る |
| T4 | source-scan ガード: `post_async_ime_apply_complete(`(ShadowToggle の呼び元)が `focus_gen` を渡していること、`handle_wm_async_ime_apply_complete` が `shadow_completion_outcome` を経由すること。`architecture_guard.rs` の既存形式(文字列走査) | `tests/architecture_guard.rs` | Linux(CI の `nextest -p awase-windows --test architecture_guard`) |
| T5 | `ImeModel` レベル: 既存の「generation 付き epoch 不一致 → Stale」(`ime_model.rs:2669〜`)は不変であることの確認(変更なしで通ること)。新規は不要 | 既存 | host |

**再現 fixture**: 実機の journal は無い。T1/T3 の「世代が進んだ」は合成で作る(journal 由来ではない)。`tests/journals/` への新規 journal 追加は、WM の到着順が journal に載らないので不可。

**(b) の記録**: `docs/known-bugs/BUG-098.md` を更新する。状態を「M-3(`cf48bc67`)で主窓は閉じ、本修正で G1/G2 を閉じた。G3(OFF の宛先未捕獲)は残る」に。
`fix_commits` に追加。30 行以内のルールを守る(現在の本文は冗長なので、更新時に短くする)。

## 6. PR の分け方・成否判定・取りやめ条件・CI

**PR 分け**(ローカルでビルド・テストしない方針。すべて develop 向け、worktree で作業):

- **PR-1(docs のみ、先行・単独で良い)**: BUG-098 の記録を現状(M-3 の緩和・G1〜G3)に訂正。known-bugs/index.md の概要列も更新。**挙動が変わらないので直接 commit 可**(feedback の「docs のみは直接 commit 可」)。
- **PR-2(コード)**: 純関数 `shadow_completion_outcome` + T1 / WM の載せ方 + ハンドラ照合 + T2 + T4 + (できれば T3) / M-3 の future 内照合の削除(Opus の判断次第)。
  1 PR に収める(純関数とハンドラの結線を分けると、使われない関数が dead_code になる・結線を忘れる)。コードを含み挙動を変えるので **PR 経由**。

**成否の判定(CI のどのジョブか)**:
- `check`/`clippy`(`-p awase -p awase-windows`、Windows ターゲット): `runtime/` のコンパイルと lint。
- `nextest --workspace --lib`(Linux): T1。
- `nextest -p awase-windows --test architecture_guard --test golden_scenarios --test layer_boundary_guard`: T4 と既存ガードが通る。
- `windows-build` ジョブ(`windows-latest`): T2/T3 が実際に実行される。**ここで T2/T3 が走ったことをログで確認する**(件数に含まれるか。gated テストが「存在しない」ことの罠は CLAUDE.md の説明どおり。`cargo test --list` で見えない)。
- dylint(`DYLINT_RUSTFLAGS="-D warnings" cargo dylint --all -p awase-windows`): 新規の `&mut`/belief 書き込みを足さないので影響なしの見込み(要確認)。
- ci-vocab・実機 CI(e2e)は本修正では**回さない/効果を測れない**(再現しないため)。「CI で実機再現して直ったことを確認する」ことは**できない**ので、そう PR に明記する。
- **効果の確認(`feedback_verify_gate_by_effect_log`)**: 照合が効いたことは「skip ログの件数」でなく下流の効果で見る。
  ハンドラが落とした完了を `tracing::info!("[async-apply-stale] ...")` で 1 行出し、journal の `ImeOpenApplied{outcome}` に `UnsafeToToggle` として残ることを windows-build の T3 で確認。実機ではこの行が **0 件でも異常ではない**(窓が狭いため)。

**取りやめ条件**:
- `Runtime` の構築が重く T3 が書けず、かつ T1/T2/T4 だけではハンドラの結線を固定できないと Opus が判断した場合 → 案 D(記録更新のみ)へ。
- 実機で、フォーカス変更を伴わない通常の shadow toggle の後に `[async-apply-stale]` が出る(= 誤って捨てている)、または `applied` が OFF 後も `Optimistic/Confirmed(true)` のまま固着した場合は **revert**。
  revert 時は experiment-logging.md に従いコミット本文に アプリ(例: Chrome/WezTerm)・IME(GJI/MS-IME、ON/OFF)・再現手順/症状を書き、`docs/experiments.md` に 1 行追記。
- `ime_mode_focus_gen` が同一プロセス内の hwnd 変更で頻繁に進み、完了が高頻度で捨てられる(§4 R1)と判明した場合: 照合を「プロセス変更の epoch」に切り替える変形(案 B の epoch 照合部分だけを使う)を検討。

## 7. 未確認の点・判断を仰ぎたい点

未確認(コードを読んだだけで実行・実機確認はしていない):
- U1: `ime_mode_focus_gen` の bump 時点(`ir_post_focus_change_snapshot`)と ImeModel の epoch(`FocusChanged` dispatch)の**前後関係**。同一の refresh 呼び出し内かどうか未確認。G1 の窓の大きさに関わる。
- U2: `architecture_guard.rs` に、M-3 の future 内照合の文字列(`ime_mode_focus_gen.get()` の `with_app`)や `post_async_ime_apply_complete(` の呼び元を固定しているものがあるか(grep のヒット無しだが、複数行チェーンの盲点あり=`feedback_multiline_grep_blind_spot`)。実装前に再確認。
- U3: `with_app` が再入で `None` になる頻度(G2)。ログに該当行は無い(M-3 は `None` を黙って一致扱い)。**実害の有無は未測定**。
- U4: `kp_shadow_actuate` の press 予約(`claim_press_write`)が async 完了を捨てたとき解けなくなることの影響(R2)。
- U5: `lparam` に `focus_gen` を詰める幅(R6)。64bit 前提でよいか。
- U6: `Runtime` を作る既存の lib テスト用フィクスチャの有無(T3 の可否)。
- U7: `state/` ungated の純関数の置き場(T1)。

所有者・Opus に判断を仰ぎたい点:
1. **優先度**: 実害の観測(報告・journal)が無い。M-3 で主窓は閉じている。それでも案 A(G1/G2 を閉じる)を進めるか、案 D(記録の訂正だけ)で止めるか。
   所有者の決定は「既存の世代で直す」だが、前提(M-3 の存在)が記録に反映されていなかった事実を踏まえた再確認。
2. **M-3 の future 内照合を削除するか残すか**(ハンドラ照合が唯一の照合点になる方が単純だが、再入時の挙動の差を Opus に確認)。
3. **OFF の `Untargeted`(G3)**: 書き込み先の捕獲(Phase C)を別タスクで進めるか。世代では原理的に守れない窓なので、BUG-098 の記録に「残る」と明記して閉じてよいか。
4. **載せ方**: `lparam` 詰め込み(案 A の推奨)か、`RuntimeOutbox` 等で構造体のまま渡す(WM の wire を触らず済むが、既存 3 呼び元との一貫性が崩れる)か。
5. **R1(同一プロセス内の hwnd 変更でも捨てる)**を許容するか。M-3 と同じ挙動なので後退ではないが、G1 の分だけ捨てる範囲が増える。
