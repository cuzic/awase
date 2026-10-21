# DD4 — 時間窓・凍結・待ち・猶予の重なり(「窓切替直後/キー押下直後は触らない・待つ」)

調査: 2026-10-21、`/home/cuzic/rust-nicola-wt-dd`(origin/develop `e4766b2a`)。読み取りのみ、cargo 未実行。
対象: `crates/awase-windows-base/src/tuning.rs` の定数すべてと、tuning.rs の外にある時間定数・時間窓。

## 結論

- **数**: E 2 件、C 4 件、?(実測・確認が要る)3 件、統治の穴 1 件(DD4-10)。K(独立に必要)と判定したもの 8 組。
- **上位 5 件**
  1. **DD4-01(E、確度 高)** `DRIFT_CORRECTION_THRESHOLD_MS`(400ms)は、閾値としてはもう使われていない。本番の読み手は `FeedbackPolicy::Read.deadline` と `FeedbackPolicy::Blind.backoff` の 2 つの初期化だけで、この 2 つのフィールドは本番では誰も読まない。定数と 2 つのフィールドを消せる。tuning.rs の doc(「400ms 続いたら補正」)は事実と違う。
  2. **DD4-02(E、確度 高)** idle-conv-check の適用時に `EXPLICIT_IME_SUPPRESS_MS` で経過時間をもう一度比べている箇所(`key_pipeline.rs:812-820`)は、常に通る。直前の「値が同じか」の比較(:799)と、spawn 時のガード 4 から論理的に導けるため。
  3. **DD4-03(C、確度 高)** `IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS`(platform_state.rs、8000)と `IME_APPLY_PENDING_TIMEOUT_MS`(tuning.rs、8000)は、根拠(BUG-34、5741ms の実測)も値も同じ。tuning.rs の doc 自身が「同じ根拠・同じ値を採用」と書いている。1 つの定数にまとめる。
  4. **DD4-04(C、確度 中〜高)** TsfGate の warmup の期限が 2 か所で決まっている。FSM の `with_timer(WarmupTimeout, 500ms)` / `with_kill_timer` は戻り値を `let _ =` で捨てていて効いていない。実際のタイマーは殻(`runtime/mod.rs:1832`)が別に張る。どちらか片方にする。
  5. **DD4-05(C、確度 中)** `reschedule_ime_refresh` には「短い窓のあいだ読み直しを予約する」分岐が 3 本並んでいる(外部変化の窓・英数の候補・通過マーク)。トリガーは同じ(モードキー・観測)で、間隔も同じ 60ms。1 つの純関数にまとめれば全列挙で固定できる。外部変化の窓の分岐にだけ、BUG-158 の「読みが失敗したら窓の終わりの 1 回に絞る」が無い(意図的かは未確認)。
- **見立ての確度**: E の 2 件は grep と論理だけで裏付けた(実機は要らない)。C の 2 件(DD4-03・04)は値も挙動も変えない整理。DD4-05・06 と ? の 3 件は、等価性の確認か実機の測定が要る。
- **全体の所見**: tuning.rs の定数を 1 つずつ見たところ、**同じトリガー(モードキー・窓切替)で走る時間窓は多いが、たいていは別の読み手・別の遅れを守っている**(下の K 表)。値の重なりは偶然のものが多い(1500ms の 4 つ:`EXPLICIT_IME_SUPPRESS_MS`・`EISU_CANDIDATE_LIFETIME_MS`・`DRIFT_CORRECTION_OBS_MAX_AGE_MS`・`IME_DETECT_MISS_THRESHOLD`×500ms)。値が同じというだけでまとめてはいけない。消せるのは「もう読まれていない」「論理的に先に止まっている」ものに限られる。

## 0. 数(確かめたコマンド)

```
$ grep -c 'pub const' crates/awase-windows-base/src/tuning.rs            → 41
$ grep -cE 'pub const [A-Z0-9_]+_MS\b' ...tuning.rs                       → 39(ほかに ms の RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE と、回数の IME_DETECT_MISS_THRESHOLD)
$ grep -n 'pending = true' ...tuning.rs | grep '#\[measured' | wc -l      → 39(41 件中、実測が付いているのは
                                                                             RAW_TSF_LITERAL_DETECT_MS_LONG_IDLE と HALF_WIDTH_RESUME_HOLD_MAX_MS の 2 件だけ)
```

**tuning.rs の外にある時間定数**(`#[measured]` も tuning-constants 規約も掛からない。DD4-10): `EXPLICIT_OFF_CACHE_SUPPRESS_MS`(runtime/focus_tracking.rs:16、10s)、`IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS`(state/platform_state.rs:2265、8s)、`WARMUP_TIMEOUT_MS`(tsf/tsf_gate.rs:58、500)、`Caps::focus_settle_ms` 100/500/200(awase-ime-core state/app_ime_policy.rs:114-144)、`RETRY_INTERVAL_MS` 160・`MAX_TRIES` 4(runtime/key_pipeline.rs:2869)、`OBSERVATION_WINDOW_MS` 100(tsf/warmup/unicode_literal_observer.rs:45)、`READ_DELAYS_MS` [900,2500]・`MIN_TRIGGER_INTERVAL_MS` 3000(runtime/indicator_glyph_shadow.rs:39-41)、`MIN_REQUEST_INTERVAL_MS` 300・`PENDING_STALE_MS` 5000(state/indicator_menu.rs:401/407)、`CONFIRM_TSF_WAIT_MS` 10(awase-windows-plans relay_plan.rs:126)、`EPOCH_FENCE_GRACE_MS`(tsf/probe.rs:334)、`KeymapCache::RECHECK_MS` 2000(awase-key-effect key_effect_predictor.rs:872)、`send_health` の 100/2000、`hook_watchdog` の 200 など、`HOOK_JOIN_TIMEOUT_MS` 500、フックの watchdog 3s(runtime/mod.rs の `start_hook_watchdog` に直書き)、設定の `focus_debounce_ms` 50・`ime_poll_interval_ms`・`[auto_detect] max_ms` 2500/`idle_ms` 3000。
(`git grep -nE '\bconst [A-Z0-9_]*(MS|TTL|...)' -- crates/...` の結果から、時間の意味を持つものだけを拾った)

## 1. 候補

### DD4-01(E)`DRIFT_CORRECTION_THRESHOLD_MS` と、書かれるだけのフィールド `Read.deadline`・`Blind.backoff`

- **防御**: `tuning.rs:275 DRIFT_CORRECTION_THRESHOLD_MS = 400`。本番の読み手:
  - `awase-ime-core/src/state/app_ime_policy.rs:70` `FEEDBACK_READ.deadline`
  - `app_ime_policy.rs:75` `FEEDBACK_BLIND.backoff`
  - (ほかはテストだけ。`app_ime_policy.rs:460/486/498`、`platform_state.rs:2883/2973/3664` の「+50」)
- (a) 不変条件(当初の意図): ずれが 400ms 続くまでは drift correction を出さない。
- (b) **単独で要る場面: 無い**。`drift_correction.rs:146-149` が「継続時間の閾値は持たない(即時補正)。かつての閾値は常に 0 だった。`DRIFT_CORRECTION_THRESHOLD_MS` は … 到達できなかった」と書いている(`2cd99845`、2026-10-06 E1 レビュー対応)。
  - `deadline` と `backoff` を本番で読む箇所: `git grep -n 'FeedbackPolicy::Read\|FeedbackPolicy::Blind'` で見ると、本番の match はすべて `{ .. }` か `{ max_attempts, .. }` で、`deadline`/`backoff` を取り出しているのは `app_ime_policy.rs:456-498`(テスト)だけ。tuning.rs:337-342 も「`backoff` は … 一度も読まれていない」と書いている。
- (c) 根拠: ADR-080/089(当初の設計)、ADR-104 決定 6-b(解釈の変更)、`2cd99845`(閾値を撤去)。
- (d) 依存: `DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS` の doc(tuning.rs:322-325)と `IME_ACTUATION_BLIND_MAX_ATTEMPTS` の doc(app_ime_policy.rs:17-24、「5 × backoff ≒ 2s」)が、この定数があることを前提に書かれている。どちらも事実と違うので一緒に直す。
- **統合案**: `FeedbackPolicy::Read { source }`・`FeedbackPolicy::Blind { max_attempts }` にして、`DRIFT_CORRECTION_THRESHOLD_MS` を tuning.rs から消す。platform_state のテストにある `DRIFT_CORRECTION_THRESHOLD_MS + 50` は、「時間が経った」という意味のリテラル(例 450)に置き換える。
- **壊れるもの**: `app_ime_policy.rs` のテスト 3 本(値の固定)。`journal.rs:2601` のテスト(`backoff:` を構築している)。journal の `ImeActuation` の JSON から `deadline`/`backoff` が消える(serde は未知のフィールドを無視し、`ime_actuation.rs` に `deny_unknown_fields` は無いので、`tests/journals/drift_correction/bug-43-*.json` は読める。`drift_correction_replay.rs` は読み込むだけで、書き戻した結果を比べてはいない)。tools/・awase-replay・awase-bug-report には `backoff` の参照が無い(`git grep -ln backoff -- tools crates/awase-replay crates/awase-bug-report` が 0 件)。
- **検証**: 挙動は変わらない(読み手がいないので)。Linux の `cargo test -p awase-ime-core` で足りる。

### DD4-02(E)idle-conv-check の適用時に、経過時間で抑制窓をもう一度判定している

- **防御**(同じ不変条件「明示の IME 操作から 1500ms 以内の conv の読みを belief に入れない」を 3 か所で守っている):
  1. spawn 時のガード 4: `src/engine/idle_check.rs:62`(`should_run_idle_conv_check`、`key_pipeline.rs:566-575` から呼ぶ)
  2. 適用時の値の比較: `key_pipeline.rs:799` `last_explicit_ime_action_ms_raw() != explicit_action_ms_at_spawn` なら棄却
  3. **適用時の経過時間の比較**: `key_pipeline.rs:812-820` `explicit_ime_action_age_ms(now) < EXPLICIT_IME_SUPPRESS_MS` なら棄却
- (b) 3 が単独で止める場面: **無い**。2 を通ったなら、spawn 時と適用時の `last_explicit_ime_action_ms` は同じ値 X。spawn 時には 1 で `now_spawn − X ≥ 1500` を確認済み(ガード 4 は `is_first_key_after_focus` でも外れない。idle_check.rs:16-20)。`GetTickCount64` は単調なので `now_apply − X ≥ now_spawn − X ≥ 1500`。X が 0(未操作)なら、どちらの時点でも `u64::MAX` で通る。
  - 書き込み口は `platform_state.rs:954` と `:1047` の 2 つだけで、どちらも今の tick を代入する(`git grep -n 'last_explicit_ime_action_ms' -- crates`)。`apply_idle_conv_check` の呼び出し元は `key_pipeline.rs:711` の 1 か所だけ。
- (c) 根拠: `a0f63ca5`(1500ms の抑制を導入)、`d4a686b5`(offload 化)、BUG-34 追補(2 を追加した理由:「経過時間ではなく値の一致で判定する」)。3 の直前のコメント(:806-811)は「独立した業務ルール」と書いているが、2 と 1 から導けてしまう。
- **統合案**: 3(:806-820)を消す。2 のコメントに「spawn 時のガード 4 と合わせて、適用時の窓の判定を含む」と 1 行足す。
- **壊れるもの**: 無い(runtime/ は cfg(windows) で、このパスを固定するテストは無い。`git grep -n 'apply 時に explicit IME action' -- crates` は本体のログ文字列だけ)。
- **検証**: 論理の証明のみ。確かめるなら、spawn と適用の 2 段の判定を純関数(例 `state/idle_conv_admission.rs`)へ切り出し、(X, now_spawn, now_apply) を全列挙するテストで「3 が偽を返すのは 1 か 2 が偽のときだけ」を固定してから消す(Linux で走る)。再発ファミリー「IME belief」に当たるので、fix-requires-evidence に従ってその全列挙テストを (a) として添える。

### DD4-03(C)BUG-34 の「ハングで止まった呼び出しを放棄とみなす」期限が 2 つある

- **防御**:
  - `tuning.rs:477 IME_APPLY_PENDING_TIMEOUT_MS = 8000`(`ImeModel.pending` のパージ、ime_model.rs:1160)
  - `platform_state.rs:2265 IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS = 8000`(idle-conv-check の in-flight の自己回復、key_pipeline.rs:622)
- (a) どちらも「`SendMessageTimeoutW` が `HungAppTimeout`(実測 5741ms)まで止まりうる呼び出しを、それより先に放棄とみなさない」。
- (b) 別々の経路(apply と conv の読み)を守っているので、**機構としてはどちらも要る(K)**。定数を 2 つ持つ必要は無い。
- (c) tuning.rs:473-475「`IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS` … と同じ根拠・同じ値を採用する」。
- **統合案**: tuning.rs に 1 つだけ残す(例 `HUNG_BLOCKING_CALL_STALE_MS = 8000`、doc に BUG-34 の 5741ms)。`IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS` を消して、key_pipeline.rs:622・platform_state.rs:2245 をそれに向ける。片方だけ値が上がる、という将来のずれを防ぐ。
- **壊れるもの**: 無い(テストからの参照は無い。`git grep` で確認)。tuning.rs の定数の数は変わらない(片方は元から tuning.rs の外)。

### DD4-04(C)TsfGate の warmup の期限を FSM と殻が二重に持っている

- **防御**:
  - FSM: `tsf/tsf_gate.rs:146-150` `FocusChange` → `with_timer(GateTimer::WarmupTimeout, WARMUP_TIMEOUT_MS)`、`:157/162` `with_kill_timer`
  - 殻: `runtime/mod.rs:1832-1835` `timer.set(TIMER_TSF_GATE, WARMUP_TIMEOUT_MS)`。`output/mod.rs:594/612/620` の doc で「呼び出し後に TIMER_TSF_GATE をセット/kill すること」と取り決めている
- (b) **FSM 側は効いていない**。`tsf_gate.rs:224/236/252/264` は `self.inner.on_event(...)` の `Response` のタイマーを捨てている(`let _ =` か `(_, drained)`)。タイマーを実際に動かしているのは殻だけ。FSM のタイマー宣言を読むのは、テストの `resp.timers.iter().any(..)`(:336/:348)だけ。
- (d) 殻は、PendingWarmup 中に別の focus イベントが来ると FSM を再初期化しない(output/mod.rs:600-606)のに、タイマーは張り直す(期限を延ばす)。FSM のタイマー宣言は、この「延長」を表していない。2 つの記述がずれている。
- **統合案(推奨)**: FSM から `with_timer`/`with_kill_timer` を外し、期限の所有者を殻だけにする(今の実際の挙動のまま)。逆に FSM の Response で動かすなら、`on_focus_change_tsf` の「PendingWarmup 中は FSM を呼ばないが期限は延ばす」を FSM の遷移(`(PendingWarmup, FocusChange)`)として持たせる必要があり、変更が大きい。
- **壊れるもの**: `tsf_gate.rs` のテスト 2 本(タイマーの宣言を検査している)。挙動は変わらない。

### DD4-05(C)「短い窓のあいだ読み直しを予約する」分岐が 3 本並ぶ

- **防御**(`runtime/mod.rs:1162-1250 reschedule_ime_refresh`):
  1. 外部変化の監視窓(ADR-205/188): `external_change_watch_remaining_ms` → `MODE_KEY_PASS_REREAD_MS.min(remaining+1)`(:1182-1192)
  2. 英数の候補(ADR-238): `eisu_candidate_remaining_ms` → `mode_key_pass_next_read_ms(last_ime_read_ok, ..)`(:1199-1212)
  3. 通過マーク(ADR-187): `mode_key_pass_window_remaining_ms` → 同じ `mode_key_pass_next_read_ms`(:1236-1245)。読めない窓では `mode_key_pass_expiry_wait_ms`(:1226-1229)
- (a) どれも「短い窓の中の観測を取りこぼさないよう、通常のポーリング(500ms)や明示意図による停止より優先して読み直す」。
- (d) 1 と 3 は同じ打鍵で同時に立つ(`key_pipeline.rs:1783-1802`、`executor.rs:639-647`)。窓の長さは同じ `MODE_KEY_PASS_MARK_WINDOW_MS`(platform_state.rs:377-599 の 11 か所)。**1 にだけ BUG-158 の失敗時の間引き(`mode_key_pass_next_read_ms`)が無い**。読めない窓の prefetch が失敗し続けたとき、60ms ごとに読み直すかどうかは未確認。
- **統合案**: 判断を純関数 `state/reread_schedule.rs::plan_reread(Facts{ tsf_native, watch_remaining, eisu_remaining, readable, mark_remaining, mark_expiry_wait, last_read_ok, explicit_intent, poll_ms }) -> Option<u64>` に移す。殻は事実を集めて結果を `schedule_ime_refresh` に渡すだけにする。分岐の優先順位を全列挙(ME-3 の方式)で固定してから、1 にも失敗時の間引きを掛けるかは別の PR で判断する(挙動が変わるので)。
- **前回との関係**: INDEX の MC-C2(「外部変化に追随する 3 経路が同じ後半をコピー」)は追随の**後半**の重なりの話。DD4-05 は**読み直しの予約**の重なりで、MC-C2 より前にできる(擬似 IME は要らない)。
- **検証**: `explicit_press_exhaustive.rs` の方式で、切り出し前の分岐をそのまま写した参照実装と比べる。

### DD4-06(C?)窓切替直後の最初の打鍵を保持する仕組みが 4 つ重なる

- **防御**(フォーカス変化を起点に、最初の打鍵を保持する。`app/mod.rs:705-733` → `runtime/key_pipeline.rs:62-120` の順):
  1. `lang_detect` の保持(ADR-263): `[auto_detect] max_ms` 2500(config.rs:604)、`TIMER_LANG_HOLD`
  2. 半角英数の戻りの保持(ADR-245 R2): `HALF_WIDTH_RESUME_HOLD_MAX_MS` 100、`TIMER_RESUME_HOLD`
  3. focus resync の保持(BUG-77): `FOCUS_RESYNC_DEADLINE_MS` 100、`TIMER_FOCUS_RESYNC`(TsfNative のみ)
  4. TsfGate の PendingWarmup(DD4-04): `WARMUP_TIMEOUT_MS` 500、`TIMER_TSF_GATE`
  - 加えて、2 の起案と同時に `confirm_gate_deadline_override_ms = now + SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS(800)`(key_pipeline.rs:2563-2567)と `note_explicit_ime_action`(→ idle-conv-check を 1500ms 止める)が立つ。
- (b) 2 と 3 はどちらも「利用者が体感する遅れの上限」として 100ms を選んだ**方針値**で、実測から導いた値ではない(tuning.rs:485-488・544-547 の本文)。**3 で保持されて再生された打鍵が、次に 2 の保持に入る**と、合わせて最大 200ms 待たせうる(3 は `deliver_key_event` の手前、2 は `kp_run_inner` の中)。TsfNative × MS-IME 本体 × 半角英数の戻り待ち、が重なる構成。この経路を実際に通るかは**未確認**。
- 2 と confirm-gate の上書き: 2 が R1 の観測(conv の読み)で解放された場合、再生された打鍵が出力するときには conv が既に読めているので、confirm-gate は最初の読みで通る(=重複)。2 が期限で解放された場合(観測が来ない)は、confirm-gate だけが守る(=独立に要る、K)。
- **統合案**: 2 と 3 の 100ms を 1 つの方針値(例 `FIRST_KEY_HOLD_BUDGET_MS`)にまとめ、「最初の打鍵を捕まえた時刻から数えた残りの予算」で後段の保持の期限を決める(合計が予算を超えないようにする)。1 と 4 は目的と長さが違う(1 は判定のための打鍵列、4 は TSF モードの確定待ち)ので含めない。
- **前回との関係**: INDEX の MD-07〜10(窓切替直後の保持・読み・先同期の重なり、ME-5 待ち)と同じ領域。本件はその時間軸の見方で、**MD-07〜10 と一緒に扱う**。
- **実機要**: e2e-ime の `sc-focusrestore-*`(e2e-ime.yml:81)を TsfNative の窓 × MS-IME 本体で走らせ、ログ `[focus-resync]` と `[resume-hold]`(`ResumeDoneToken`)が同じ打鍵に続けて出るかを数える。

### DD4-07(C?、所有者判断)字形の柵が殻と reducer の 2 か所にある(まだ潜在)

- **防御**: `awase-ime-core/src/state/indicator_glyph.rs:707-720 fence_glyph`(`TooSoonAfterModeKey`、ほかにフォーカスの世代・前面の一致)と、`ime_model.rs:853-866`(reducer の `GLYPH_KEY_FENCE_MS`)。コメントが自ら「二重の柵」と書いている。
- (b) 今は**どちらも本番では発火しない**。`fence_glyph(` の本番の呼び出しは 0 件で、`architecture_guard.rs:8949-8992` がそれを固定している(ADR-273 段階 1 は配線していない)。
- (d) reducer の柵は journal の再生でも決定的。殻の時間の柵は、同じ事実(`since_last_mode_key_ms` は reducer の `last_mode_key_tick_ms` から作る)を別の層でもう一度判定している。
- **統合案**: 配線するときに、`fence_glyph` の**時間の柵だけ**を外す(フォーカスの世代・前面の一致は reducer が持たない事実なので残す)。字形を belief に入れない判断は reducer の 1 か所に寄せる。今は消さない(配線の設計は ADR-273 の所有者の判断)。

### DD4-08(?)Chrome(Vk モード)では「GJI が冷えた」を 2 つの時計が判定している

- **防御**: `ColdKind::classify`(gji_fsm.rs:124-131、`MEDIUM_IDLE_PROBE_MS` 7000 / `LONG_IDLE_MS` 10000)と、`GjiTimer::LongIdle`(`long_idle_ms_for`、gji_fsm.rs:1031-1036。Vk モードは `CHROME_LONG_IDLE_MS` 5000)。OnWarm のまま 5 秒経つと OnCold(Long) に落ちる。
- (b) Vk モードでは、OnWarm から 7〜10 秒の「Medium」に落ちる道は 5 秒のタイマーが先に取るので通らない。Vk モードで Long が効くのは `forces_prepend_f2` だけ(`is_long_cold` が効くのは literal の窓の延長で、`is_tsf_mode` のときだけ。probe_fsm.rs:173)。
- (c) tuning.rs:111-116 によると、5 秒の根拠(6312ms の idle 後に再初期化に約 145ms)は撤去された機構(BUG-024、Chrome の probe の最小待ちの延長)のもので、今の値には実測の裏付けが無い。
- **候補**: `CHROME_LONG_IDLE_MS` を消し、全モードで `LONG_IDLE_MS` を使う。**挙動が変わる**: Chrome × GJI で idle 5〜10 秒の後の最初の打鍵に F2 を前置しなくなる。tuning-constants.md の「Chrome probe の 20→100→200→350」と同じ系統(こ→ko 型のリテラル化)の再発に直結する恐れがあるので、測らずに動かさない。
- **実機要**: e2e-ime の `ts-raw-*-gji-idle-*` / `ts-*-gji-idle-*`(e2e-ime.yml:81 にある名前の型)を idle 6 秒・8 秒で、実 Chrome(chromepage)に対して走らせる。「F2 前置あり/なし」でリテラル化の件数を比べる。0 件のままなら撤去する。

### DD4-09(?)`focus_settle_ms` が守る場面は、ほぼ残っていない

- **防御**: `Caps::focus_settle_ms`(app_ime_policy.rs:114-144、ImmCross 100 / Imm32Unavailable 500 / TsfNative 200、**tuning.rs の外**)→ `InputBarrier::FocusTransition.settle_until` → `ime_apply_should_defer`(runtime/mod.rs:1313)。本番の読み手は drift correction の延期だけ(`ime_refresh.rs:947`、`DriftPlan::DeferToSettle` → `schedule_settle_retry`)。
- (b) drift correction には `last_intent == desired` が要る(drift_correction.rs:137-139)。プロセスをまたぐフォーカス変化では `FocusChanged` が `last_intent` を消す(ime_model.rs:1095)ので、settle がなくても drift は出ない(=他が先に止める)。barrier は最初の打鍵で消える(key_pipeline.rs:435 `consume_focus_barrier`)。したがって settle だけが止めるのは、「**同じプロセスの中で hwnd が変わり**(`update_focus_window`、last_intent が残る)、最初の打鍵より前に、明示意図と食い違う新しい観測が来た」場合に限られる。
- **実機要**: 実機ログで `[focus-settle] drift correction skipped (settling)` の件数を数える。0 件なら、`focus_settle_ms`・`InputBarrier` の時間の条件・`schedule_settle_retry` を撤去する候補になる(barrier 自体は focus probe の 1 回だけの起動に使っているので残す)。件数がある場合でも、値を tuning.rs に移して `#[measured]` を付ける。

### DD4-10(統治の穴)tuning.rs の外にある時間定数

- 0 節の約 20 個は、`tuning-constants.md`(対象 `paths: tuning.rs`)の実測の義務も、`architecture_guard.rs:6951` の `#[measured]` の強制も受けない。このうち**フォーカス・belief・warmup の再発ファミリーの待ち**に当たるのは、`EXPLICIT_OFF_CACHE_SUPPRESS_MS`・`IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS`(DD4-03)・`WARMUP_TIMEOUT_MS`・`focus_settle_ms`(DD4-09)・`RETRY_INTERVAL_MS`/`MAX_TRIES`(shift-conv-guard の復元。tuning.rs:217-220 の `SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS` が根拠として引いている)。
- `EXPLICIT_OFF_CACHE_SUPPRESS_MS`(10s、全窓共通)・`EXPLICIT_OFF_INTENT_TTL_MS`(30s、窓ごと)・`EXPLICIT_ON_INTENT_TTL_MS`(10s): tuning.rs:394-398 と ADR-087 INV-24(a) が「将来の統合」を求めている。調べた結果、範囲が違う(前者は**別の窓**に移ったときに Imm32Unavailable のキャッシュ・安全デフォルトを抑える。focus_tracking.rs:731・939。後者は同じ窓に戻ったときの上書き)ので **K**。ただし定数は tuning.rs に移すべき(値は変えない)。
- complexity-budget.md(未発効)の数え方では、移した分だけ tuning.rs の `pub const` が増える。DD4-01・03 で 2 つ減るので、相殺できる。

## 2. 見たうえで K(独立に必要)としたもの

| 組 | 同じ起点 | 独立に要る理由(単独で止める場面) |
|---|---|---|
| `KEY_EFFECT_SETTLE_MS` 170 / `MODE_KEY_PASS_MARK_WINDOW_MS` 300 / `GLYPH_KEY_FENCE_MS` 1300 | モードキーの押下 | 読み手と遅れが違う: IMM の読み(実測最大 131ms)・追随の窓(再読み取りが 2 回収まる)・字形(実測最大 0.885s)。値の引き上げは 100→170(`aac67a4c`)・1000→1300(`8b011058`)で、どちらも実測付き。**ただし「settle + 再読み取り間隔 < 窓」という関係(tuning.rs:511-512)は、どのテストも固定していない**。`const _: () = assert!(KEY_EFFECT_SETTLE_MS + MODE_KEY_PASS_REREAD_MS < MODE_KEY_PASS_MARK_WINDOW_MS)` の 1 行で固定するのを勧める(GLYPH > SETTLE はテストで固定済み。indicator_glyph.rs:2122-2123) |
| `EXPLICIT_IME_SUPPRESS_MS`(ガード 4)/ ガード 5(`is_ime_mode_key`)/ key-effect の fence | 明示操作・モードキー | 明示操作(`UserImeSetIntent`)は `key_effect` を消す(ime_model.rs:834)ので、fence は明示操作の後を守らない。ガード 5 はこの打鍵自身だけを見る。ガード 4 は Shift の復元(`note_explicit_ime_action`)の後も守る |
| `follow_direct_read_in_scope` の「予測があれば採らない」/ reducer の settle fence | モードキー | 前者は直接観測の窓の経路だけ、後者はすべての観測の経路(通常の ObserverPoll を含む)。GJI では前者が窓全体(300ms)で予測のある軸を隠すので、その経路に限れば後者を含む。ただし後者は他の経路のために要る |
| `GJI_CONFIRM_WINDOW_MS` 500 / `GJI_SETTLE_GRACE_MS` 300 | フォーカス後の GJI I/O | 守る性質が違う: 開いている(observer_poll=true、gji_observer.rs:46)と、`is_japanese_ime` を false に落とさない(key_pipeline.rs:3301-3307)。比較も `>` と `>=` で違う |
| 戻りの保持(DD4-06 の 2)/ confirm-gate の上書き 800ms | 半角英数の戻り | 保持が期限で解放されたとき(R1 の観測が来ない)、confirm-gate だけが送信を止める |
| `SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS` 5000 / `..._RELEASE_CONFIRM_MS` 800 | Shift | 入ったときと離したときで、順番に置き換わる(同時には効かない) |
| `OBSERVATION_FRESH_WINDOW_MS` 3000 / `DRIFT_CORRECTION_OBS_MAX_AGE_MS` 1500 | 観測の鮮度 | 後者は drift の判断だけに使う、前者より厳しい条件。前者は belief の導出の全体に効く |
| `EXPLICIT_OFF_CACHE_SUPPRESS_MS` / IntentStore の OFF TTL | 明示 OFF | DD4-10 のとおり範囲が違う(全窓 / 窓ごと) |

値が偶然同じもの(1500ms の 4 つ、100ms の `MIN_FOCUS_DURATION_MS`・`FOCUS_RESYNC_DEADLINE_MS`・`HALF_WIDTH_RESUME_HOLD_MAX_MS`、3000ms の `GJI_REATTACH_INTERVAL_MS`・`DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS`・`OBSERVATION_FRESH_WINDOW_MS`)は、DD4-06 の 2 つを除いて目的が違う。統合しない。

## 3. 着手の順序(案)

1. DD4-01・DD4-03(Linux で完結し、挙動は変わらない。1 PR にまとめてよい)
2. DD4-04(挙動は変わらない。tsf_gate のテスト 2 本を更新)
3. DD4-02(先に純関数へ切り出し、全列挙テストを付けて、それから消す)
4. DD4-05(純関数への切り出しと全列挙。外部変化の窓に失敗時の間引きを掛けるかは別に判断)
5. DD4-09・DD4-08 は実機ログ・e2e で測ってから。DD4-06・07 は MD-07〜10・ADR-273 の作業と一緒に。

## 4. 検証に要るもの

- **純関数の全列挙(ME-3 の方式、Linux)で足りるもの**: DD4-02(spawn/適用の 2 段の判定を `state/` に切り出す)、DD4-05(`plan_reread`)。どちらも今は `runtime/`(cfg(windows))の中にあり、Linux のテストバイナリには入らない。切り出しが前提。
- **grep と型だけで足りるもの**: DD4-01(本番の読み手が 0 件)、DD4-03(定数の付け替え)、DD4-04(戻り値を捨てていること)。
- **擬似 IME で確かめられるもの**: DD4-09 の「同じプロセスの中で hwnd が変わったときの drift」は、ハーネスに `update_focus_window` 相当の入力と `FocusTransition` barrier が要る。今のハーネス(INDEX §3)は `runtime/` を通らないので、`ime_apply_should_defer` は届かない。ME-2 の後になる。
- **実機・e2e が要るもの**:
  - DD4-08: e2e-ime の `ts-*-gji-idle-*` を idle 6s/8s、実 Chrome で。F2 前置あり/なしのリテラル化の件数。
  - DD4-09: 実機ログの `[focus-settle] drift correction skipped (settling)` の件数(dragonflyg4 の過去ログ・不具合報告の journal)。
  - DD4-06: `sc-focusrestore-*` を TsfNative × MS-IME 本体で。`[focus-resync]` の保持と `[resume-hold]` の保持が、同じ打鍵に続けて出るか。
- **壊れるものの一覧**: `app_ime_policy.rs` のテスト 3 本と `journal.rs:2601` のテスト(DD4-01)、`tsf_gate.rs:336/348` のテスト(DD4-04)、journal の `ImeActuation` の JSON の形(DD4-01、読み込みは serde が未知のフィールドを無視するので互換)。ガードが固定している名前・件数のうち、DD4-01〜05 で変わるものは無い(`architecture_guard.rs` を `DRIFT_CORRECTION_THRESHOLD|FOCUS_RESYNC_DEADLINE|EXPLICIT_IME_SUPPRESS` で grep して 0 件)。
