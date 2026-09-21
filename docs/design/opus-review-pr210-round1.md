# PR #210 敵対的レビュー（round1）

対象: `/home/cuzic/rust-nicola-worktrees/pr210`、ブランチ `fix/hook-diagnostic-repost-and-lock-alloc`
（HEAD `f4f0c2a0` = `79cc1d9e` + origin/develop マージ）。読解のみ。編集は本ファイルの書き出しだけ。

## 結論（先に）

**Blocker なし**（機能経路への影響がゼロであることを確認した。下記「Blocker が無いと判断した根拠」参照）。
ただし **Major 4件**。要点は2つ:

1. コード内コメントと `docs/design/opus-review-hook-mutex-safety.md` §3 が「スピンの実例」として
   挙げているトレイメニュー経路は、**実際には `RUNTIME` を借用していない**（借用を手放してから
   `TrackPopupMenu` に入る設計になっている）。つまり主張1の「スピンしうる」は**現行コードでは
   到達経路が示せていない**。変更自体は無害で筋も通るが、これは *fix* ではなく *hardening* であり、
   根拠の書き方が実態と食い違っている（F-1・E-1）。
2. それと引き換えに導入した「取りこぼし」は**実在のコスト**であり、しかも
   「次の drain が全部拾う」は**ジャーナルダンプ経路には成立しない**（ダンプ側が drain を
   呼んでいない）。取りこぼした診断は、それを読むはずの不具合報告に載らない（B-1）。

## A. `with_app` が借用中に呼ばれたら何をするか

**検証結果（問題なし）**: `crates/awase-windows/src/lib.rs:233-243` → `RUNTIME.try_borrow_mut()` が
失敗したら `tracing::warn!("with_app re-entry detected — returning None ...")` を出して `None` を返す。
`crates/awase-windows/src/single_thread_cell.rs:72-75` の通り `RefCell::try_borrow_mut().ok()` であり
**パニックしない**。さらに `guard.as_mut().map(f)` なので `RUNTIME` が未 `set` / `clear` 済み（`None`）の
場合もクロージャを呼ばず `None`。副作用はログ1行だけ。よって「スピンより悪化」は無い。
`let _ = with_app(...)` は `lib.rs:231-232` の `#[must_use]` 注記が明示的に認めている書き方であり、
clippy（windows ターゲット）も通る（実行して確認）。

- **A-1（Nit）**: この warn はメッセージ本文が「caller should re-post if needed」であり、
  **意図的に repost しない**新しい呼び出し元には的外れ。診断イベントは1キーにつき down/up の2回
  post される（`hook.rs:1191-1199`。self-injected も除外前なので warmup バーストでは更に増える）ので、
  再入が起きる状況では warn が束で出る。それが流れ込む `awase.log` は不具合報告に
  `LOG_EXCERPT_MAX_BYTES` で切り詰めて添付される（`message_handlers.rs:1314-1360` 付近）ため、
  「ログを埋める」という点では本 PR が減らそうとしているノイズと同種。理想は
  `with_app_dropping(reason)` のような別入口か、呼び出し元での warn 抑止。

## B. 「次の drain が取りこぼし分も拾う」は本当か

**検証結果（Major 2件）**: `drain_hook_ime_mode_diagnostics` の呼び出し元は
**`runtime/message_handlers.rs:100`（`handle_wm_hook_ime_mode_diagnostic`）の1箇所だけ**
（HEAD 全体を grep して確認）。そして `WM_HOOK_IME_MODE_DIAGNOSTIC` の post 元も
`hook.rs:1199` の1箇所だけ。つまり **drain の契機は「次の IME モードキー到達」のみ**で、
タイマーもダンプ契機も存在しない。

- **B-1（Major）: ジャーナルダンプがリングを flush しない。**
  - 不具合報告（トレイ「不具合を報告...」）: `message_handlers.rs:1314-1345` の `with_app` 内で
    `drain_journal_entries` → `ClockAnchor` → `DumpTriggered` → `dump_to_file_capped` を実行するが、
    **`hook::drain_hook_ime_mode_diagnostics()` を呼んでいない**。
  - Alt 連打トリガの `WM_DUMP_JOURNAL`（`message_handlers.rs:2151` `handle_wm_dump_journal`）も同様に
    呼んでいない。
  - 帰結: 取りこぼした診断レコードはリングに滞留したまま、**次の IME モードキーが押されるまで**
    journal に入らない。ユーザーが「症状が出た直後に不具合を報告」した場合（＝この診断の主用途）、
    その分のレコードは報告に**載らない**。PR 本文の「次の drain が取りこぼし分も拾うのでロスレス配送は
    不要」は、**唯一の消費者であるダンプ経路について偽**。
  - 対策（2行、本 PR のスコープ内に収まる）: `handle_wm_dump_journal` と BugReport 分岐の先頭で
    `handle_wm_hook_ime_mode_diagnostic(app)` を呼ぶ。これを入れれば「次の drain が全部拾う」が
    ダンプ時点で真になり、lossy 化の言い分が初めて成立する。
- **B-2（Major）: 遅延 drain はタイムスタンプを捏造する。**
  `journal.rs:1363-1368` `record()` → `1297-1305` `stamp()` は **`seq` と `elapsed_ms` を record 時点で**
  打つ。`HookImeModeDiagnosticRecord`（`journal.rs:103-110`）は絶対時刻を持たず、時間情報は
  `since_prev_ime_mode_ms`（前回の IME モードキーとの差分、push 時に算出）だけ。
  従来（repost）の遅延は「`RUNTIME` が空くまで」＝数ms オーダーで、journal 上の位置は実質正しかった。
  lossy 化後は「次の IME モードキーまで」＝分オーダーになりうるので、レコードは**無関係な
  エントリの後ろに、実際より遅い `elapsed_ms` で**並ぶ。`since_actuation_us`（`hook.rs:1183-1186` で
  debug ログにだけ出している値）や他レーンとの前後関係を journal から読む作業が壊れる。
  このリポジトリには `docs/design/journal-diagnostic-fidelity-fixes.md` があるほど
  「journal の時系列忠実性」を重視しているのに、本 PR はその軸のトレードオフを明記していない。
  対策: レコードに push 時の `tick_ms` を1フィールド足す（`hook.rs:1175` で既に `now_ms` を算出済み。
  drain 時刻との差が見えるようになる）か、少なくともコメントに「journal 上の位置は drain 時点である」
  と書く。
- **B-3（Minor）: 押し出されるのは古い側。** `push_hook_ime_mode_diagnostic`（`hook.rs:1106-1114`）は
  `len >= 64` で `pop_front`＝**最古を捨てる**。64件を超えて drain が失敗し続ける状況では、
  「事象の起点に最も近いレコード」から失われる（診断としては欲しくない側）。
  また post は per-push で coalescing が無いため、回復後は `N` 個の `WM_HOOK_IME_MODE_DIAGNOSTIC` が
  並び、1個目が全部拾って残り `N-1` 個は空 drain になる（各回 C-2 のアロケーションを払う）。
  設計記録 §3 の補足が示す `WAKE_PENDING` 型の coalescing は入れていない（今回はスコープ外で可）。

## C. `mem::replace` と push 側の容量・上限ロジックの整合

- **C-1（Major）: push 側のアロケーションが残っており、新しい doc コメントはそれを否定している。**
  `HookState::new()` は `const fn`（`hook.rs:158-160`）なので `ime_mode_diagnostics: Mutex::new(VecDeque::new())`
  ＝**capacity 0** で始まる（`VecDeque::with_capacity` は const fn ではないため、ここで直せない）。
  したがって**プロセス起動から最初の成功した drain までの間**、`push_back`（`hook.rs:1113`）は
  **hook スレッド上・ロック保持中に**バッファを grow＝`HeapAlloc` する。これは設計記録 §4 が
  「hook スレッド → Mutex →（メインスレッド保持中）→ ヒープロック」として問題視した2段ネストの、
  **より直接的な版**（`LowLevelHooksTimeout` が実際に適用される側のスレッドがヒープロックを取る）。
  それにも関わらず本 PR の doc コメント（`hook.rs:39-46`）は
  「`pop_front`/`push_back`/`mem::replace` ... のみの O(1) 構造体操作で、**アロケーションも**
  ブロッキング処理も**含まない**」と無条件に書き、設計記録 §4 は
  「push 側も capacity 64 の deque を受け取るため ... **hook スレッド側もアロケーションフリーが保証される**」
  と書き、ADR-164 追記は「緩和条項自体を撤廃した」と書いている。**3箇所すべてが、同じファイル内に
  反例があるのに無条件の主張になっている。**
  選択肢: (a) doc を「初回 drain 以降は」に限定する（1行、最小）、(b) 初期化を遅延（`OnceLock` 等）
  または初回 push で `reserve(CAP)` して「1回だけ・有界」と明記する、(c) 主張を弱める。
  いずれにせよ**「文字どおり真になった」という現在の書き方は維持できない**。
- **C-2（Minor）: 保持区間外に出した代わりにアロケーション量は増えている。**
  `size_of::<HookImeModeDiagnosticRecord>()` は `Option<u64>` による align 8 で **32バイト**
  （フィールド合計25バイト）。よって `VecDeque::with_capacity(64)` は毎回 **2KB** を確保する
  ——**キューが空でも、レコード1件でも**。さらに `taken.into_iter().collect()` が
  **2個目**の `Vec`（len×32）を確保する。旧実装はロック内で `len×32` の1個だけだった。
  「保持区間から出す」目的は達成しているが、`4行で不変条件が真になる`という説明は
  「アロケーションを外に出し、かつ増やした」を含んでいない。
  安価な改善: `Vec::from(taken)`（std の `From<VecDeque<T>> for Vec<T>` は**再確保しない**、最悪
  O(n) の回転のみ）にすれば2個目の確保が消える。より簡単には戻り値を `VecDeque` にして
  呼び出し元（`message_handlers.rs:100` は `for record in ...` するだけ）にそのまま渡す。
- **C-3（検証結果、問題なし）: Poisoned lock の扱いは従来と同一。** push（`hook.rs:1107`）も
  drain（`hook.rs:1125`）も `let Ok(..) else` で早期 return、drain は `Vec::new()` を返す——変更なし。
  唯一の差は、poisoned 時に先に確保した `fresh` を確保→即 drop する無駄（実害なし）。
- **C-4（検証結果、問題なし）: 上限ロジックとの整合。** push は `len >= 64` で `pop_front` してから
  `push_back` するので `len <= 64` が不変。`with_capacity(64)` は capacity >= 64 を保証するので、
  初回 drain 後は push が再確保しない（C-1 の起動直後を除く）。`HOOK_IME_MODE_DIAGNOSTIC_CAP` を
  両者が共有しているので定数ズレも無い。

## D. `taken.into_iter().collect()` の順序

**検証結果（問題なし）**: `VecDeque::into_iter()` は front → back（FIFO、push した順）で yield し、
`ExactSizeIterator` なので `collect::<Vec<_>>()` は len ぶん確保して順序どおり move する。
旧実装の `drain(..)` も front → back。したがって**呼び出し元が見る順序は完全に同一**で、
`message_handlers.rs:100-106` が journal に記録する順＝到達順という性質は保たれる。
（C-2 で挙げた `Vec::from(taken)` も順序は同じ。）

## E. 他に同種の repost スピン候補が残っていないか

**スコープ内の欠陥**:

- **E-1（Major）: スピンの到達経路が現行コードに見つからない（＝この PR は fix ではなく hardening）。**
  `WM_HOOK_IME_MODE_DIAGNOSTIC` は post 専用（`hook.rs:1199` → `win32.rs:106-150` は
  エンジン HWND 宛 `PostMessageW`）。post されたメッセージが**借用中に**ディスパッチされるには
  「`RUNTIME` 借用中にネストしたポストメッセージポンプへ入る」区間が必要。
  本番で `ModalPumpGuard::enter()`（`runtime/engine_window.rs:56`）を使うのは
  **`tray.rs:696` の `TrackPopupMenu` 1箇所だけ**で、その経路は借用していない（F-1）。
  他に nested pump は見つからなかった（`PeekMessage`/`GetMessage` の自前ループは
  `hook.rs` のフックスレッドと `examples/` のみ。`SendMessageTimeoutW` ブロック中に配送されるのは
  *送信*メッセージだけで post は配送されない。`MessageBoxW` 系は
  `msime_key_assignment::spawn_yes_open_ime_settings_dialog` が別スレッド、
  `tray::show_about_dialog` は `with_app` 外）。
  `RUNTIME == None`（未 set / clear 後）でも `with_app_or_repost` は repost するので同じスピンに
  なりうるが、起動順（`bootstrap.rs:630` `RUNTIME.set` → `:1178` エンジン窓作成 → `:1180` フック導入）と
  終了順（`run_message_loop` 終了 → `:698` `RUNTIME.clear`、以後ポンプが回らない）から、
  ここも現行では発火しない。
  → 変更を入れること自体は妥当（将来 nested-pump-under-borrow を作られても損害が「診断の欠落」で
  止まる）。**ただしコメントを「観測された不具合の修正」と読める書き方にしないこと。**
  実損が確定しているのは B-1/B-2 の側だけなので、少なくとも B-1 の flush を同じ PR に入れるべき。

**スコープ外の観察**:

- **E-2（Minor、スコープ外）**: 残る `with_app_or_repost*` は6箇所——`app/mod.rs:476`
  (`WM_ASYNC_IME_APPLY_COMPLETE`)、`:482`(`WM_GJI_REINIT_RETRY_COMPLETE`)、
  `:487`(`WM_KANA_LOCK_WARNING_CHANGED`)、`:501`(`WM_CALIBRATION_KEY_DETECTED`)、
  `:506`(`WM_PANIC_RESET`)、`:532`(`WM_FOCUS_KIND_UPDATE`)。コメントが主張する機構が本物なら
  **6箇所すべてが同じようにスピンする**。1箇所だけ直して「repost 経路が1本減る」で終わっているのは
  論理として非対称（＝E-1 の裏返し。スピンが仮説なら今回の書き方が過剰、実害なら残り6箇所が未処置）。
  なお payload 付きの2件は落とせず、`WM_KANA_LOCK_WARNING_CHANGED` は `lib.rs:344-350` の doc が
  「repost で順序が入れ替わっても冪等」と明記済みなので、対処方針は一律ではない。
- **E-3（参考）**: 同じ問題に対する3つ目の既存イディオムがある——`app/mod.rs:640-643`
  `handle_hook_key_event` は `with_app` が `None` を返したら `INPUT_DEFER.replay_later(..)` に回す
  （捨てず、repost せず、専用キューへ退避）。診断でここまでやる必要は無いが、
  「repost か破棄か」の二択ではないことは記録しておくと将来の判断材料になる。

## F. ADR-164 追記・設計記録とコードの食い違い

- **F-1（Major）: トレイメニューは `RUNTIME` を借用していない。**
  `app/mod.rs:493-497` のコメントと設計記録 §3 手順2 は「トレイメニュー（`TrackPopupMenu` ネスト
  モーダルループ）... のように、`RUNTIME` を借用したままネストしたメッセージポンプに入る区間がある」と
  書くが、実コードは逆:
  - `app/mod.rs:565-567` `WM_APP` は `with_app` を経由せず `handle_wm_app_tray` を呼ぶ。
  - `message_handlers.rs:1244-1263` は `with_app_ref(|app| (…snapshot…))` で**値をコピーして借用を
    手放してから** `tray::handle_tray_message(..)` を呼ぶ（借用は `SingleThreadCell::with` の
    戻りで終了、`single_thread_cell.rs:55-57`）。
  - `tray.rs:696-702` の `ModalPumpGuard` + `TrackPopupMenu` はその外側。メニュー選択後の
    `WM_COMMAND`（`app/mod.rs:578-580`）も `with_app` の外。
  つまり**メニュー表示中は `RUNTIME` が空いており、届いた診断メッセージは普通に成功する**。
  この一文は削除または「将来そういう区間を作った場合に備えて」と書き換えるべき。
  ADR-164 追記側にはこの誤りは持ち込まれていない（追記は §1 の反証の話だけ）ので、
  修正対象は `app/mod.rs` のコメントと設計記録 §3。
- **F-2（Major）: ADR-164 が自己矛盾している。** 追記（`docs/adr/164-...md:313-331`）は
  「緩和条項『アロケーションは上限64件の`Vec`収集のみに留める』を**撤廃した**」と書くが、
  直前の 訂正3 本文（同ファイル 305-311 行付近）は
  「`drain(..).collect()`——アロケーションは発生するが上限64件の有界サイズ」
  「（ロック下でのアロケーションは上限64件の`Vec`収集のみに留める）」を**そのまま残している**。
  `.claude/rules/docs-frontmatter-convention.md` の方針では ADR 本文が全文の SSOT なので、
  将来 `不変条件` を grep した人は**撤廃したはずの許容条項に先に当たる**。
  追記するだけでなく、当該文を打ち消し線／書き換えで直すこと（C-1 の限定つきで）。
- **F-3（Nit）: 数字が2つ合っていない。** (a)「構造体3ワードの入れ替え」（設計記録 §4、
  ADR 追記の両方）——`VecDeque` は `ptr`/`cap`/`head`/`len` の**4ワード**。
  (b)「64要素×24バイト＝約1.5KB」（設計記録 §1 反証B）——`Option<u64>` があるため
  `size_of` は **32バイト**、64件で **2KB**。結論に影響しないが、C-2 の「毎回2KB」を見落とす原因に
  なっている。
- **F-4（Nit）: 行番号の陳腐化。** 設計記録は「読んだ版: develop `3e802984`」と明記しているので
  記録としては許容だが、`hook.rs`/`app/mod.rs` の**恒久コメントが §3/§4 を参照**しており、
  そこに書かれた `hook.rs:975-981`（現 1116-1130）・`app/mod.rs:408-412`（現 491-499）・
  `hook.rs:995`（現 1145）・`hook.rs:1038-1046`（現 1191-1199）・`hook.rs:57`（現 56）・
  `message_handlers.rs:648-679`（現 570-610）・`runtime/mod.rs:1888-1892`（現 1591-1596）は
  いずれも解決しない。恒久側から参照するなら関数名基準に直すのが安全。
- **F-5（検証結果、問題なし）: 主張3（この Mutex は issue #165 の原因ではない）は HEAD で成立。**
  ① watchdog 警告は3秒周期 `WM_TIMER` ハンドラ（`message_handlers.rs:570-610`、周期は
  `runtime/mod.rs:1591-1596` で `Duration::from_secs(3)`）＝メインスレッド上で出るので、
  警告が出続けている間そのスレッドが同じロックを5000ms 保持しているという状態は両立しない。
  ② `tick_hook_alive()` は `hook_callback`（`hook.rs:1143`）の**最初の文**（`:1145`）。
  ③ §5 の「既定ログレベルは info なので debug 行は報告に入らない＝journal が唯一の経路」も
  `bootstrap.rs:138/152`（`--debug` 時のみ `debug`、既定 `info`）で確認。
  よって ADR への「否定的証拠」追記は内容として妥当（残る問題は F-1/F-2 の書き方だけ）。

## G. develop マージ後の整合性

**検証結果（問題なし）**:

- `git diff origin/develop HEAD -- crates/awase-windows/src/{hook.rs,app/mod.rs}` は
  意図した2ハンク以外を含まない（マージによる混入・コンフリクト残骸なし）。
- ブランチ後に develop 側が `hook.rs` に入れた変更（`43a400cc`/`7f13c4e5`:
  `AWASE_TEST_INJECTION` / `TEST_INJECTION_MARKER`、`hook.rs:1078-1097` 付近）は HEAD に存在し、
  診断リングとは独立（push 判定 `ime_key_kind.is_some()` に影響しない）。
- マージ後も `drain_hook_ime_mode_diagnostics` の呼び出し元は `message_handlers.rs:100` の1つだけ
  （B-1 の前提が develop 側の変更で覆っていないことの確認）。
- `cargo check --target x86_64-pc-windows-msvc -p awase-windows` 成功、
  `cargo clippy --target x86_64-pc-windows-msvc -p awase-windows` 警告なし。
- `cargo test -p awase-windows --test architecture_guard`（106 passed）/
  `--test layer_boundary_guard`（8 passed）。特に `architecture_guard.rs:4126-4140` の
  「`hook.rs` の `Mutex<` 出現数 == 1」ガードは、doc コメント書き換えで `Mutex<` を増やして
  いないため影響なし（実測でも `grep -c "Mutex<" hook.rs` == 1）。

## Blocker が無いと判断した根拠

`handle_wm_hook_ime_mode_diagnostic`（`message_handlers.rs:99-106`）は
`journal.record(JournalEntry::HookImeModeDiagnostic{..})` **だけ**を行い、engine / belief / conv /
actuation いずれの状態も触らない。よって取りこぼしの影響は**診断情報の欠落に限定**され、
入力挙動・IME 制御の正しさには波及しない（`.claude/rules/ime-belief-architecture.md` の
Observe→classify→reduce 経路には一切入らない）。`mem::replace` 版も順序・poison 挙動・
上限ロジックが従来と一致（C-3/C-4/D）。よってマージ不可の欠陥は無い。

## マージ前に入れることを推奨する最小セット

1. **B-1**: `handle_wm_dump_journal` と BugReport 分岐の先頭で診断リングを drain（2行）。
   これが無いと「次の drain が拾う」という lossy 化の前提が、唯一の消費者について偽のまま。
2. **F-1**: `app/mod.rs` のコメントから「トレイメニュー等で `RUNTIME` 借用中」という
   誤った実例を外す（設計記録 §3 も同様に訂正か注記）。
3. **C-1 + F-2**: 「アロケーションを含まない」の無条件主張を「初回 drain 以降」に限定するか、
   push 側の初回 grow を潰す。合わせて ADR-164 訂正3 本文の旧緩和条項を実際に書き換える
   （追記だけでは自己矛盾が残る）。
4. 任意（C-2）: `taken.into_iter().collect()` → `Vec::from(taken)`、または戻り値を `VecDeque` に。

## 参考: 規約面の観察（Minor、ブロックしない）

`.claude/rules/fix-requires-evidence.md` の再発ファミリー表に `hook.rs` は無く、
`.git/hooks/pre-push` の警告対象にも入らないため自動チェックは沈黙する。一方で設計記録 §7 自身が
「§3 の変更は挙動変更を含む」と書いており、本 PR には回帰テストも `docs/known-bugs/BUG-NNN.md` も無い
（ADR-164 への追記が (b) 相当を部分的に満たしているとは言える）。Windows 限定コードなので
`hook.rs` 内の `#[cfg(test)]` は Linux CI に出ない（CLAUDE.md 記載の性質）ため、テストを足すなら
`drain` の順序・空化を検証する小さな windows-only ユニットテストか、そもそも (b) に寄せる判断が現実的。
