# ADR-272 段階 B 設計草稿のレビュー(Opus、2026-10-20、origin/develop 8863f062)

対象: `stageb-design-draft.md`。行番号は `git show origin/develop:<path>` の行。

## 0. 結論

1. **B1 はこの形では勧めない。** 利益(フレークと mutants の timeout を消す)は、殻と FSM の API を変えずに、
   **非 Windows 版の時計をテストから進められる形にするだけ(下の「代案 B0」)** でほぼ全部得られる。B1 は ADR-079 の
   epoch fence と BUG-210 の baseline の時点を動かす変更で、再発ファミリー(warmup)の 3 PR になる。この費用に見合わない。
2. B1 をやる場合でも、草稿の取りこぼし防止ガード(リテラル構築の位置の固定)では**主要な失敗(snapshot を yield をまたいで持ち越す)を防げない**。
   持ち越すと、入力が止まったまま戻らない失敗になりうる(§2-a)。
3. F4・F5・F6・F7 に事実誤りか不正確な点がある。F3 は対象の列挙が足りず、終了基準を満たすには草稿の範囲外のファイルにも手を入れる必要がある。

## 1. 事実 F1〜F8 の裏取り

- **F1: 正しい(補足 1 点)。** `output/mod.rs:1039-1079`。`tick_t` と `env` を作ってから、`take_pending_tsf`(1063)とログを挟んで `machine.tick(env)`(1079)を呼ぶ。
  間に `await` も Win32 の呼び出しも無い。補足: env の組み立て自体が `current_stage_origin()` と `gji_is_active_ime()` を呼ぶ。
  `obs` は env リテラルの**最後**で取ること。`tick_t`(1040)は env に入っていないので、`obs.now_ms` と二重になる。
  `tick_t` を `obs.now_ms` から取るように統一すること。
  ただし「判定時点との差は数 µs」が成り立つのは**tick の中の判定**だけ。殻の側の時刻の読み(§2-b)には当てはまらない。
- **F2: 正しい。** `probe_fsm.rs:69-100`。`gji_active`・`gji_evidence_absent`・`gji_candidate_visible_now`・`literal_session_confirmed_gen` がある。
  注意: `TsfEnvSnapshot` は `#[derive(Default, Copy)]`。`obs` を足すとテストの `..Default::default()`(`ms_ime_ready_coro.rs:223-232` ほか多数)が
  **黙って `now_ms=0`** になる(§2-e)。
- **F3: 正しいが不足。** `probe.rs` の読み取りは草稿どおり(585-592, 613-620, 641-668, 697-726, 805-806, 92-94, 122-135)。草稿に無いもの:
  - `evidence_now` が `observer::literal_session_confirmed(cold_seq)`(668)を読む。これは `env.literal_session_confirmed_gen` と同じ値の重複読み。
    `GjiObsSnapshot` のフィールド案にも入っていない。
  - 判定時刻を直接読む箇所が `LiteralDetector`/`TsfReadinessProbe` の外にもある:
    `literal_detect_fsm.rs:451`(`veto_decision`、tick 内)、`:503`(`LiteralDetectFsm::new` の deadline)、
    `probe_coro_state.rs:103`(`apply_transmit_done` の deadline。殻から、送信の**後**に呼ばれる)、
    `ms_ime_ready_coro.rs:90/96/113`(コルーチン内。90 は prime 時、すなわち tick の外)、
    `mozc_compose_check_fsm.rs:65/128`(tick 内。128 の直後に `read_mozc_compose` で共有メモリも読む)、
    `unicode_literal_observer.rs:71`(`gji_write_bytes`、tick 内)、`cold_warmup.rs:115`。
  - 終了基準の `literal_detect_fsm.rs` の sleep(799)は `LiteralDetector` ではなく `veto_decision:451` の時計待ち
    (`GJI_CANDIDATE_VETO_CAP_MS`=300 + 50ms)。B1b の範囲に `veto_decision` を明示しないと、この sleep は消えない。
- **F4: 判定メソッドについては正しい。構築の記述が誤り。**
  - 判定(`check_now`/`evidence_now`/`visible_fencing_verdict`/`check_outcome`)の本番の呼び出し元は次の 4 か所で、すべて `step_probe` の中の `machine.tick` から到達する:
    `literal_detect_fsm.rs:293/298/374`、`probe_fsm.rs:415/441/446-447/671/733/744`、`gji_warmup_coro.rs:86`、
    `LiteralDetectCore::poll`(`gji_warmup_coro.rs:~226` の `core.poll(env)`)。`machine.tick` の本番の呼び出しは `output/mod.rs:1079` だけ。
  - 誤り: 「構築の呼び出し元は殻と**一部 FSM のコンストラクタ**」。本番で `LiteralDetector::new*` を呼ぶのは殻だけ
    (`probe_io.rs:504/543/605`、`vk_send.rs:629`)。FSM の中の呼び出し(`chrome_probe.rs:116/158`)はテストだけ。
  - 漏れ: 判定ではないが、FSM 側のコードで殻から(tick の外で)時計を読むものがある。
    `ProbeCoroState::apply_transmit_done:103` と `LiteralDetectFsm::new:503`(どちらも送信の後)、prime の中の `ms_ime_ready_coro.rs:90`。
- **F5: 件数が誤り。** 本番の `log_composition_probe` の呼び出しは **13**(probe_fsm 6・literal_detect_fsm 5・gji_warmup_coro 1・unicode_literal_observer 1)。
  そのうち `gji_warmup_coro.rs:212` は**到達不能**(§3 の副次所見)。Windows 版の中身は草稿どおり(`ime_diagnostic.rs:276-`、
  `GetForegroundWindow` と `capture_composition_snapshot`)。ただし `TSF_PROBE_SNAP`(同 33-45 行)は TIMER ハンドラが立てるスレッドローカルで、
  これも「tick の中で呼ばれる」ことに依存している。
- **F6: 誤り。** 構築時に `OutputActiveGuard::begin()` を呼ぶのは `LiteralDetectFsm::new`(`literal_detect_fsm.rs:502`)と
  `MozcComposeCheckFsm::new`(`mozc_compose_check_fsm.rs:67`)の 2 つだけ。残りは次のとおり。
  - 遅延で取る: `GjiWarmupCoro::apply_transmit_done/apply_vk_sent`(`gji_warmup_coro.rs:325/335`)、
    `ms_ime_ready_coro.rs:149`(Phase 2 の直前。BUG-58 のとおり Phase 1 では**持ってはいけない**)。
  - 殻から受け取る: `TsfProbeCoro`/`ChromeProbe`(`vk_send.rs:310`、`cold_warmup`)。
  B2 (ii) の「`wants_output_gate: bool` を値で返す」は、構築時に決まる bool では BUG-58 の遅延取得を表せない。tick ごとの値が要る。
  B2 を見送る理由の 1 つとして書き直すこと。
- **F7: 件数が不正確。** 実際にロックを取っている箇所は probe.rs 14・probe_fsm.rs 6・literal_detect_fsm.rs 4 で、**計 24**
  (「約 32」は use 文・コメントを含む言及数)。`thread::sleep` は 6(probe.rs 841/978/987/1178、probe_fsm.rs 1359、literal_detect_fsm.rs 799)で草稿どおり。
  `TSF_OBS_TEST_LOCK` は observer.rs(自身のテスト)と chrome_probe.rs(1)にも残る。`ms_ime_ready_coro` のテストは別の `OUTPUT_GATE_TEST_LOCK`(5 か所)を使う。
  フレークの原因は sleep(壁時計)で、ロックはフレークの原因ではない。ロックは直列化しているだけ。
- **F8: 未確認。** run 39418696310 は `CI`(develop、11:57Z、failure)で、mutants のシャードが多数 failure。`gh run view --log-failed` は空で返り、
  `tsf/probe.rs` の MISSED/timeout の行は確かめられなかった。補足: `TsfReadinessProbe::check_now` の境界の分岐は、本番では**到達しない**。
  2 つの producer(`vk_send.rs:291` の `(0, 0)`、`cold_warmup.rs:114-118` の `min_ms=0, total_max_ms=0`)はどちらも
  `min_deadline == max_deadline == warmup_sent_ms` を作るので、初回の呼び出しで必ず true を返す(probe.rs:110-118 の doc にも書いてある)。
  したがって MISSED の 2 件は「本番の挙動のテスト漏れ」ではない。本番で効くのは `check_outcome` の `settled`/`monitor_healthy` の算出(92-101)。

## 2. B1 の弱点・見落とし

### (a) yield をまたいで古い snapshot を使う経路(最重要)

1 つの tick で進むコルーチンの区間は 1 つだけ(`ProbeCoroState::tick` → `coro.step`)。dispatcher は `apply_*` を呼ぶだけで、再 tick しない。
したがって**同じ tick の中**で古くなることは無い。問題は**コルーチンのローカル変数として env を次の tick へ持ち越すこと**で、これは現にコードにある。

- `probe_fsm.rs:381-442` `await_vk_detection(..., env)`: 可視の分岐のループ(414-420)は `yield_step` の戻り値を `let _ = poll_input;` で捨てる。
  そのあと `visible_fencing_verdict` を再び呼ぶ。今は判定の中で時計を読むので正しく動く。
  B1 で `visible_fencing_verdict(&env.obs, ...)` と**引数の `env`** を渡すと、`now` が凍る。すると `grace_hold_verdict`(probe.rs:776-790)は
  `now - hold_since = 0 < EPOCH_FENCE_GRACE_MS` かつ `now < deadline` で、`None` を返し続ける。
  結果はコルーチンが終わらず、`TsfProbeCoro::_guard`/`GjiWarmupCoro::literal_detect_guard` が `OUTPUT_GATE.active` を保持し続ける。
  後続の物理キーは保留されたままになる(probe の段に時間の上限は無い。`watchdog` 等を grep しても見つからなかった)。
  **キー入力が止まったまま戻らない**失敗になる。441 行の `evidence_now(env.gji_candidate_visible_now, ...)` は、すでに引数の `env`(ループ前の値)を使っている。
  これと同じ書き方で `env.obs` を渡すのは自然に起きる。
- `gji_warmup_coro.rs:~204` `let gji_active = env.gji_active;`: Transmit の yield(185)の**後**に、yield の前の env を読む(すでに 1 tick 古い。今は診断のみ)。
- `tsf_probe_coro_body`(`probe_fsm.rs:666-680`)は Phase 1 の env を `break input.env` で外へ持ち出し、のちの判断に使う(同じ区間なので今は安全)。

草稿の「`GjiObsSnapshot { .. }` のリテラル構築は `gji_obs_snapshot()` の中だけ」というガードでは、上のどれも防げない。正当に作られた値を `Copy` で持ち越しているだけだから。
有効な対策の案(どちらか、または両方):
1. `GjiObsSnapshot` に `tick_seq: u64`(`step_probe` で単調増加)を入れる。`LiteralDetector` と `TsfReadinessProbe` に `last_tick_seq: Cell<u64>` を持たせ、
   `check_now`/`visible_fencing_verdict`/`check_outcome` が `None`/`false` を返した**次の**呼び出しで `debug_assert!(snap.tick_seq > last)` を置く。
   凍った値での再判定はテストで即座に落ちる。
2. `GjiObsSnapshot` を `!Copy`・`!Clone` にして `TsfEnvSnapshot` から外し、`ProbeTickInput.obs` として渡す。判定メソッドは `&GjiObsSnapshot` を取る。
   持ち越しは move が要るので見えるようになる。ただし禁止はできない。
   併せて `architecture_guard` で「`await_vk_detection` などループの中の判定は、直前の `yield_step` の戻り値から取った obs を使う」ことを固定する。

### (b) baseline・epoch fence・deadline の意味

- **epoch(`epoch_send_ms`)と baseline は送信の直前に取ること。** 今の時点は送信の直前で、per-VK は `probe_io.rs:605-614`、一括送信は 504/543、warm は `vk_send.rs:629`。
  B1 で `new(&snap)` にしたとき、殻が**tick 入口の `env.obs` を使い回す**と、epoch が「tick 入口」まで前にずれる。
  その間に GJI モニタのサンプル(`GJI_SAMPLE_INTERVAL_MS`=10)が入ると、前の世代の write が `last_write_ms >= epoch` を満たす。
  すると StaleConfirm が CompositionConfirmed に化け、literal を回収しない(ADR-079 の退行)。
  write_bytes の baseline も同じで、前の送信の分を吸収する。今は dispatcher に env が渡っていないので起きない。
  しかし「tick ですでに snapshot を持っているから渡そう」は自然な改修。
  対策: 送信前の取得は**別の型**にする(例 `PreSendBaseline`。`!Copy`、`observer::pre_send_baseline()` だけが作り、`LiteralDetector::new(PreSendBaseline, veto)` が値で消費する)。
  per-tick の `GjiObsSnapshot` からは作れないようにする。
- **deadline は送信の後に取る。** 今は送信の後に時計を読んでいる: `probe_io.rs:617`(`hook::current_tick_ms() + timeout_ms`)、
  `probe_coro_state.rs:103`、`literal_detect_fsm.rs:503`。B1 で「時刻は snapshot から」と統一して pre-send の snapshot の `now_ms` で deadline を作ると、
  deadline が送信にかかった時間だけ早まる(今は同期の SendInput だけなので小さいが、0 ではない)。
  「epoch=送信前、deadline=送信後」という非対称は意図された設計で、草稿はこれに触れていない。
  殻が送信後に `now` を渡す形(`apply_transmit_done(now_ms, ..)`、`LiteralDetectFsm::new(now_ms, ..)`)を設計に明記すること。
- **#704 のガードは文字列で位置を固定している。** `architecture_guard.rs:9470/9485/9503` は `"let literal_detector = crate::tsf::probe::LiteralDetector::new(true);"` などの文字列で順序を固定する。
  B1 でシグネチャが変わるとこれらは落ちる(気づけるので良い)。直すときは、固定の基準を「`gji_obs_snapshot()`/`pre_send_baseline()` を呼ぶ行が送信より前」に**付け替える**こと。
  `new(` の位置だけを固定し直すと、snapshot を送信後に取って `new` にだけ渡す形を見逃す。
- **tick の中での「古さ」は今と同じ。** EPOCH_FENCE_GRACE_MS(20ms)と GJI のサンプル周期(10ms)の意味は変わらない。ADR-272 (iv) の「最大 10ms」は、草稿のとおりこの構造には当てはまらない。
  ただし当てはまるのは、(a) の持ち越しと (b) の流用を構造で禁止した場合だけ。
- **既存の競合で、B1 では直らないもの。** `gji_monitor.rs:502-524` は `gji_write_bytes` を先に、`gji_last_write_ms` を後に、どちらも Relaxed で store する。
  `check_now` が bytes(新)と last_write_ms(旧)を読むと、偽の StaleConfirm(romaji の再送)になりうる。snapshot の取得関数が 1 か所になるのはこれを直す好機だが、
  load の順序だけでは直らない(store の順序を入れ替えるか seqlock が要る)。少なくとも snapshot の doc に「整合した組だ」とは書かないこと。

### (c) tick の外から判定を呼ぶ経路

判定メソッドを tick の外から呼ぶ本番の経路は無い(§1 F4)。ただし次の 3 つは tick の外で時刻を読む。B1 の範囲に入れるか、意図して残すと明記すること。
- prime(`ProbeCoroState::new` → `coro.prime()`。構築時、殻から)の中: `ms_ime_ready_coro.rs:90` の `start_ms`(ログ用)。prime の時点では env が無い。
- dispatcher からの `apply_transmit_done`(`probe_coro_state.rs:103`)。
- コンストラクタ: `LiteralDetectFsm::new:503`、`MozcComposeCheckFsm::new:65`。

### (d) 1 回の tick で同じ値を読むと挙動が変わる箇所

- `check_outcome`(probe.rs:88-101)は `check_now` で `now` を読み、そのあともう一度 `now` と `last_io` を読む。snapshot にすると一致する(`elapsed_ms` がわずかに短くなるだけ)。無害。
- `LiteralDetectCore::poll` は、`check_now` の verdict と、`evidence_now` の `write_delta`/`show_changed`(journal の証拠)が今は別々に読まれ、食い違いうる。
  一致するようになるのは改善だが、journal の証拠の値が変わる。`journal_replay` やゴールデンに evidence の数値を固定しているものがあれば更新が要る(未確認)。
- 「読んでから待つ」判断: `TsfReadinessProbe::check_now` は `settled_at_ms` を初回は `now` で設定して false を返し、次の tick で差を見る。
  `grace_hold_verdict` と `veto_decision` も同じ形。tick ごとに新しい snapshot なら挙動は同じ。凍ると (a) の無限待ちになる。挙動が変わる別の箇所は見つからなかった。

### (e) 取りこぼし防止ガードは足りるか

足りない。理由は 3 つ。
1. (a) の持ち越しと (b) の流用は、どちらも正当に構築された値を使っているので、リテラル構築の位置のガードでは止まらない。
2. テストは snapshot を自由に作る必要がある(注入が目的)。「構築は 1 か所だけ」はテストを例外にするしかなく、文字列走査のガードが複雑になる。
   フィールドを private にして `#[cfg(test)] fn for_test(..)` を置く方が単純。
3. `TsfEnvSnapshot: Default` のせいで、`obs` 追加後の既存テストは `now_ms=0` で黙って走る。
   例: `ms_ime_ready_coro` を `env.obs.now_ms` に移すと、`deadline` との比較で期限に一生届かない。そのため期限切れを確かめるテストが**期待どおり落ちる**か、
   **期限切れが起きない側の主張だけが空振りで通る**。`GjiObsSnapshot` には `Default` を付けないこと。`TsfEnvSnapshot` の `Default` も外すか、`obs` だけ明示を強制すること。

## 3. B2 見送り・PR 分割・終了基準

- **B2 を見送るのは妥当。** 理由の書き方を 2 点直す。
  - `log_composition_probe` を action にする件。草稿の「backspace より前に読む順序」は難しさを過大に見ている。
    `RawTsfLiteralRecovery` の処理(`probe_io.rs:~640-700`)は `io.set_raw_literal` で**予約するだけ**で、backspace はあとで送る。
    同じ Vec の前の位置に action を置けば順序は自明に守られる。難しさは順序ではなく、`TSF_PROBE_SNAP`(`ime_diagnostic.rs:28-45`)が tick の中にしか無いことの扱いにある。
  - `OutputActiveGuard` の件。F6 の訂正のとおり、遅延で取る(BUG-58)ものがある。構築時の bool では足りない。
- **副次所見(段階 C を検討するなら効く): `gji_warmup_coro.rs:196-215` の `!plan.needs_literal` の分岐は到達不能。**
  `needs_literal=false` だと dispatcher は `apply_transmit_done(None)` → true → `break 'stage Some(ProbeDone)`(`probe_io.rs:~520-530`)と進み、
  `output/mod.rs:1100-1102` で `drop(machine)` する。コルーチンは Transmit の yield から再開されない。
  この分岐の `win32_async::spawn_local`/`sleep_ms`(205-206)が、`gji_warmup_coro` を `#[cfg(windows)]` で個別 gate している唯一の Windows 依存に見える
  (`git grep 'win32_async\|crate::ime::\|crate::imm::\|with_app'` の該当はこの 2 行だけ。`cargo check` での確認は未実施)。
  これを消せば `gji_warmup_coro` を ungate でき、Linux で走るテストが増える可能性がある。段階 B よりも安く、効果がはっきりしている。
- **PR 分割。**
  - **B1a(型と生成点だけ、未使用)は単独では CI を通らない可能性が高い。** `ci.yml:138` は `cargo clippy --lib -- ... -D warnings`(Linux ホスト)。
    `TsfEnvSnapshot.obs` を作るだけで読まなければ、`dead_code`(field never read)になる。`allow(dead_code)` を足すと ADR-229 §0 の 5 の上限にかかる。B1a は B1b に吸収すること。
  - **代わりの分割(B1 をやる場合)。** PR1: `LiteralDetector`/`TsfReadinessProbe`/`veto_decision` に、`&GjiObsSnapshot` を取る純粋な `*_at` 版を足す。
    既存の `check_now(d)` は `check_at(&observer::gji_obs_snapshot(), d)` を呼ぶだけの薄い包みにする。**挙動は同一**(読む時点も同じ)で、テストは `*_at` に移して sleep とロックを消す。
    PR2: FSM 側の呼び出しを `input.obs` に切り替え、包みを消す。読む時点が変わるのは PR2 だけなので、(a)(b) のレビューの焦点が 1 つの PR に集まる。
- **終了基準。** 「3 ファイルのテストから `TSF_OBS_TEST_LOCK` と `thread::sleep` が消える」は、`veto_decision`(literal_detect_fsm.rs:451)を範囲に入れないと満たせない。
  また段階 B の目的(warmup が `TSF_OBS` と時計を直接読まない)に照らすと、`ms_ime_ready_coro`・`mozc_compose_check_fsm`・`unicode_literal_observer` が残り、目的は達成されない。
  終了基準は「目的を達成した」ではなく「フレークの元(sleep 6 か所)と mutants の timeout が消えた」と書き直すこと。そうすると次の代案 B0 で足りる。
- **代案 B0(勧める)。** `clock.rs` の非 Windows 版(10-27 行)はもともと `cfg(not(windows))` で、awase-windows の Linux ビルドはテスト専用。
  ここに `#[cfg(test)]` のスレッドローカルの偽の時計(`clock::test_set(ms)`/`test_advance(ms)`)を足す。6 か所の `thread::sleep` を `test_advance` に置き換える。
  - 本番(Windows)のコードは 1 行も変わらない。ADR-079 と BUG-210 の時点は動かない。再発ファミリーに触れない。
  - フレーク(sleep 60ms → 354ms)は原理的に消える。probe.rs:841 の待ちループも時計を進めるループにし、回数に上限を付ければ、mutants の timeout は「回数の上限で落ちる」に変わる。
  - `TSF_OBS_TEST_LOCK` は残る。ただしこれはフレークの原因ではない(直列化のコストだけ)。
  - 注意: nextest はテストごとにプロセスを分けるが、`cargo test` はスレッドを分ける。スレッドローカルなら両方で安全。テストが別スレッドを起こす箇所が無いかは確かめること。

## 4. 段階 B をやる価値

見立て(「残る利益はフレークの除去と、任意の段階 C の準備だけ」)は**正しい**。付け加えるなら、利益はそれよりさらに小さい。
- フレークは load average 25 のローカルでしか出ていない。GitHub のランナーでは通っている(ADR-272「段階 A の結果」)。
  さらに方針として、ローカルではビルド・テストをしない。実害は mutants の timeout と、ローカルで回したときの雑音にほぼ限られる。
- mutants の MISSED のうち `TsfReadinessProbe::check_now` の境界は、本番で到達しない分岐(§1 F8)。B1c で直しても、本番の誤りを検出する力は増えない。
- 段階 C は「誰も求めていない」(草稿 B3)。段階 C の準備としての B1 は、段階 C を決めるまで寝かせても損が無い。B1 の設計は段階 C の要件(core の公開 API の最小化)と同時に決めた方が二度手間にならない。
- 一方で B1 は、ADR-079(epoch fence)・BUG-210(baseline の時点)・BUG-58(gate の寿命)が絡む箇所の読み取りの時点を動かす。
  §2-a の失敗は「キー入力が止まったまま戻らない」で、CI では実機の GJI の I/O タイミングを再現できない(memory: CI ではリテラルを再現できない)。

**勧める順:**
1. B0(偽の時計、1 PR、本番の差分 0)。
2. 余力があれば `gji_warmup_coro.rs:196-215` の到達不能な分岐を消して ungate する(別 PR。到達不能の根拠として `probe_io.rs` の `break 'stage` と `output/mod.rs:1102` の `drop` を本文に引く)。
3. B1 は段階 C をやると決めたときに、§2 の (a)(b) の構造的な対策(`tick_seq` の assert、`PreSendBaseline` 型、`Default` の撤去)を前提に組み直す。
   ADR-272 の「未決」の (iv) は「B0 で目的を満たしたので保留」と書いて閉じてよい。
