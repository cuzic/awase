# 機構 ECRS 調査 D(フォーカス・アプリ分類・窓の切替まわりの機構とゲート)

対象ツリー: /home/cuzic/rust-nicola-wt-mech(HEAD f3c2007e、origin/develop 先端)。読み取りのみ・cargo 未実行。
接頭辞 MD。確認コマンドは `git grep -n ... -- crates src` をこのツリーで打ったもの。「未確認」と書いたものは読んだだけで grep/実機で裏取りしていない。

## 結論

- **E(撤去可)の確実な候補は小さい 4 件**(MD-01〜04、合計 約 60 行+テスト)。いずれも呼び出し元ゼロを git grep で確認済み。
- **観測専用・既定 off の実験機構の束**(MD-05: 右クリックメニュー読み取り、MD-06: 字形)は、所有者判断が要る E 候補。メニューは「belief へは使わない」と ADR-273(2026-10-20)で決定済みで、残る価値は journal の記録だけ。
- **C(統合)候補は、どれも「窓切替の直後に複数の機構が似た待ち・読み・同一性判定をしている」型**。ただし順序と遅れの軸が違うため、確実に勧められるものは無い(? が中心)。上位は MD-07(最初の打鍵の保持 3 系統)・MD-08(窓の同一性を表す印が 5 種)・MD-09(窓切替直後の IME 読みが 5 系統)。
- 「減らすと危ない」ものは K として明記した(MD-14 以降)。ADR-156 が「遅延キューの大規模統合は不採用」と決めているので、MD-07 は**保持バッファの内部実装の共通化まで**に絞った。

### 上位 5 件(確度の順)

1. MD-01・MD-02 `AppImeProfile::uses_kanji_toggle`・`is_chromium_widget`: 本番の呼び出し元ゼロ(E、確定)。
2. MD-03 `classify_focus` の捨てられる `ImmGetContext`(E、確定。結果が使われない)。
3. MD-04 `FocusResyncGate::disarm`・`armed_at_ms`(E、確定。doc 自身が未配線と明記)。
4. MD-05 インジケーターのメニュー読み取り(E?、所有者判断。約 1,270 行+テスト、既定 off・belief に使わない決定済み)。
5. MD-07 最初の打鍵の保持(lang_detect・resume_hold・FOCUS_RESYNC)(C?、保持バッファの共通化。ADR-156 の範囲を超えない形で)。

---

## E 候補

### MD-01 `AppImeProfile::uses_kanji_toggle`
- 場所: `crates/awase-windows-base/src/focus/class_names.rs:228`
- 解く問題: かつて `send_engine_state_ime_key` の mode-key 送信スキップ判定(ADR-207 で撤去)。
- 生きているか: **死んでいる**。`git grep -n uses_kanji_toggle -- crates src` → `class_names.rs` の定義(228)とテスト(521〜675 の 8 行)、`tests/architecture_guard.rs:5564` の **doc コメント 1 行**のみ。本番の呼び出し 0 件。doc コメント自身が「現在の本番の呼び出し元は無く、テストの oracle として残している」と書いている。
- 重複・統合先: `should_pass_physical_key`(`Imm32Unavailable` で false)と真偽が同じ(Imm32Unavailable だけ true/false が反転)。
- 撤去した場合の挙動変化: なし(呼び出し元が無い)。
- 擬似 IME: 不要(純粋なテーブル。class_names.rs のテストで足りる)。
- 判断: **E**。撤去で壊れるガード: `architecture_guard.rs:5564` は doc コメントのみで走査ではない(文字列 `uses_kanji_toggle` を探す走査は無い、`git grep` で確認)。ただし同 doc は「InputRelay の消費述語 4 つ」の一覧で、述語が 3 つになる旨の文言直しが要る。
- 再発ファミリー: 当たらない(消費側の無い述語)。

### MD-02 `is_chromium_widget`
- 場所: `class_names.rs:334`
- 生きているか: `git grep -n is_chromium_widget -- crates src` → 定義と同ファイルのテスト 3 行のみ。本番 0 件。`git log -S is_chromium_widget` の最後は `9b18b202`(C-4 で移動した)と `def786eb`(テスト追加)で、現行の呼び出し元は無い。
- 重複: `detect_app_kind`(同ファイル 345)が `chrome_`/`mozillawindowclass` を別に判定している。
- 判断: **E**。挙動変化なし。再発ファミリーに当たらない。

### MD-03 `classify_focus` の `ImmContextGuard::new(hwnd).is_some()`(結果が捨てられる)
- 場所: `crates/awase-windows/src/focus/classify.rs:36-38`(`let _has_imm_context = unsafe { ... .is_some() };`)
- 生きているか: 変数は `_` 始まりで以降どこでも使われない。`git grep -n "_has_imm_context\|has_imm_context" -- crates` → 1 件(定義行のみ)。`git log -S_has_imm_context` → `314e3af9`(VkCode 型への機械的置換)。コメントが「NULL でも NonText 確定にしない」と判断を述べているだけで、呼び出し自体は何も決めていない。
- 撤去した場合: `classify_focus` はワーカースレッド(`resolve_focus_kind` が 300ms の `run_with_timeout` で包む)で走る。`ImmGetContext` 1 回分の呼び出しが無くなるだけで、戻り値は変わらない。**未確認**: クロスプロセスで `ImmGetContext` が引き起こす副作用(スレッド接続)は無いはずだが実機では測っていない。
- 擬似 IME: 不可(Win32 の呼び出し。`#[cfg(windows)]`)。CI の windows-build と e2e で足りる。
- 判断: **E**。再発ファミリー: focus 分類(`focus/classify.rs`)に当たる。コメントは残す(NonText にしない判断の記録)。回帰テストの追加先: `crates/awase-windows-plans/src/state/focus_classify_plan.rs` に判断表が既にある。

### MD-04 `FocusResyncGate::disarm` と `armed_at_ms`(格納のみで読まれない)
- 場所: `crates/awase-windows/src/focus_resync.rs`(`disarm` 約 60 行目、`armed_at_ms` フィールドとゲッター)
- 解く問題: BUG-77(Alt+Tab 復帰直後の最初のキーが resync 前にリテラルになる)。disarm は「明示的 IME 操作・エンジン無効化で呼ぶ」予定だったが、doc が「未配線」と明記している。
- 生きているか: `git grep -n "FOCUS_RESYNC\.\|\.disarm()" -- crates` → 本番の呼び出しは `arm`(focus_tracking.rs:872)・`is_armed`・`consume_and_close`・`open_if_current`・`current_generation`・`is_gate_active` のみ。`.disarm()` の呼び出しは `post_bypass.disarm()`(別型 ScopedOneShot)だけで、`FocusResyncGate::disarm` は focus_resync.rs のテストのみ。`git grep -n "armed_at_ms()" -- crates | grep -v focus_resync.rs` → 0 件(ゲッターの利用なし)。
- 撤去した場合: 挙動変化なし。doc が警告する「disarm されると仮定した設計を積まない」という落とし穴も消える。
- 判断: **E**(小)。再発ファミリー: focus 遷移に当たる。ただし撤去は純粋な死んだ API の削除なので (a)(b) は要らないと考えるが、`.githooks/pre-push` の警告は出る(`focus_resync.rs` は表の対象)。
- 付記(K): この struct が `static FOCUS_RESYNC`(Atomic)であること自体は、取り込み口 `app/mod.rs::handle_hook_key_event` が `with_app` の外(再入時)で触るためで、撤去対象ではない(未確認: 再入下で Runtime の借用が取れない経路を避けるための設計という読み。ADR-123 系)。

---

## E か C か要判断(所有者判断が要るもの)

### MD-05 インジケーターの右クリックメニュー読み取り(観測専用・既定 off)
- 場所: `observer/indicator_menu.rs`(247 行)、`runtime/indicator_menu_shadow.rs`(173)、`awase-ime-core/src/state/indicator_menu.rs`(851、テスト込み)、`awase-ime-core/src/indicator_evidence.rs` の `IndicatorMenuCounters`、`app/bootstrap.rs:1349-1366`(ワーカースレッドと WinEvent フックの起動)、`lib.rs:313`(`MenuOutcome` を運ぶ WM)、`awase-bug-report` のカウンタ項目、`awase-settings/src/main.rs:4415-4520`(段階の行)、`config.rs` の `[indicator_evidence] menu`。
- 解く問題: ADR-264(観測の無い窓の IME 状態を、メニューの「現在のモード」で読む)。#623(一太郎で belief が実機とずれる)。
- 生きているか: **実行時には既定で動かない**。`config.rs` のテスト `test_indicator_evidence_defaults_to_off` が glyph・menu とも `Off` を固定。`bootstrap.rs:1355` は `effective_evidence.menu >= Observe` のときだけワーカーとフックを起動する(オプトイン)。**belief には入れないと ADR-273 が決定済み**(「メニューの配線を見送る(2026-10-20、所有者判断)」節。実機 dragonflyg4 で GJI のメニューが元の窓の状態と一致せず、12 回中 4〜10 回が「ひらがな」を指した)。ADR-264 の目的①(メニュー読み×直後の字形を字形表の正解ラベルにする)も、`indicator_glyph_shadow.rs` にメニューの結果を参照する行は無い(`grep -n menu` で出るのは popup メニュー窓クラスの判定 1 行のみ。上のコードは `MenuOutcome` の利用先が journal のみ)。ADR-273 の同節は「shadow(journal)は残す」と書いており、**所有者が意図して残している**。
- 重複: 字形(MD-06)と同じ「観測の無い窓を直す」目的の別証拠源。
- 撤去した場合: 既定の利用者には何も変わらない。`[indicator_evidence] menu = "observe"` を設定した人の journal から `IndicatorMenu` が消える。設定キーの後方互換(`config_load_diag.rs` が旧 `indicator_menu_shadow` を既に「撤去済み」と通知している形が前例)。
- 擬似 IME: 対象外。UIA/MSAA・WinEvent・タスクバーの実物が要るので、CI の `ci/indicator-menu-verify` 系の実機測定(ADR-264 の記録)でしか確かめられない。
- 判断: **? → E 寄り**。決めるために必要な事実は「実利用の journal に `IndicatorMenu` が 1 件でも入っているか」(`bug-report-latest` で直近の報告を引く)。入っていなければ、ADR-273 の断念と整合して撤去してよい。**ADR-264 の「shadow は残す」を所有者が覆す判断**になるので、E と断定しない。再発ファミリー: 当たらない(belief に触らない。`architecture_guard.rs:8815` が observe-only を固定しており、撤去時はこのガードも消す)。

### MD-06 インジケーターの字形の取り込み(観測専用・既定 off)
- 場所: `observer/indicator_glyph.rs`(348)、`runtime/indicator_glyph_shadow.rs`(292)、`awase-ime-core/src/state/indicator_glyph.rs`(2,164、テスト込み)・`indicator_glyph_store.rs`(357)・`indicator_glyph_seeds.rs`(159)。ADR-273 の核(`TaskbarGlyph` 源・`FencedGlyph`・`resolve_open_at` の分岐)も `awase-ime-core` にあるが**本番に配線されていない**。
- 生きているか: **観測のみ・既定 off**。`ui_max_level()` は glyph も `Observe`(`indicator_evidence.rs:27-31`)で、`unobserved`/`all` は画面にも出ない。ADR-273 の段階 1 は「本番への配線なし」で、`architecture_guard::taskbar_glyph_is_not_reported_from_production_yet` が殻の出現 0 件を固定している(ADR-273「段階 1 の実装状況」)。つまり**字形は、(a) 受け身の収集と表の作成は動く(オプトイン)、(b) belief への入口は核にだけ在って配線が無い**という状態。
- 重複: ADR-263 の打鍵列判定(MD-07 の lang_detect)・ADR-268 の mozc 共有メモリ(belief に High で接続済み)と同じ目的。一太郎の窓の型(ADR-261 D0-2)は実機未確認。
- 判断: **? (K 寄り)**。所有者のメモリ(ADR-261 の位置づけ)は「複数の証拠の 1 つ。取れたら使う」。一太郎の実機確認が出るまで、核(2,000 行超)を残すか畳むかは決まらない。決める事実: 一太郎(または同型の窓)で字形が実状態に追随するか(ADR-261 D0-2)。**私は撤去を勧めない**。
- 再発ファミリー: IME belief に当たる(核の `ObservationSource` 追加)。

---

## C 候補(統合・簡素化)

### MD-07 窓切替直後の「最初の打鍵の保持」が 3 系統ある
- 場所と状態:
  - ① `runtime/lang_detect_shadow.rs` + `src/lang_detect.rs:693 HoldController`(ADR-263)。**既定は `observe` で保持しない**(`src/config.rs:3446` のテストが `AutoDetectMode::Observe` を固定)。保持するのは `mode = ime_on`/`ime_on_eisu` と設定した人だけ。
  - ② `runtime/resume_hold.rs` + `awase-hook-core/src/state/half_width_resume_hold.rs:151 ResumeHold`(ADR-245 追補 2 R2)。MS-IME 本体の戻り待ち復元のとき、最初の打鍵を最大 `RESUME_HOLD_CAP=4` 打鍵・`HALF_WIDTH_RESUME_HOLD_MAX_MS=100`ms 保持。`half_width_alnum_toggle` の既定が `all`(`config.rs:429`)なので既定で有効。
  - ③ `focus_resync.rs`(BUG-77)。TsfNative×日本語 IME で最初の resync 対象キーを `INPUT_DEFER.defer_during_output` で退避(最大 `FOCUS_RESYNC_DEADLINE_MS=100`ms)。
  - 取り込み口 `app/mod.rs:710-735` で ①→②→③(+`OUTPUT_GATE`)の順に直列に判定している。
- 解いている問題: ①#623(観測の無い窓での belief ずれ)、②BUG-193/ADR-245(復元の書き込みの前に打鍵が出てしまう)、③BUG-77。
- 重複の根拠: ①と②は**同じ形の保持バッファ**(スコープ=前面窓を採り、解放時に前面窓が違えば捨て、同じなら `INPUT_DEFER.replay_later` で再生、期限は一発タイマー `TIMER_LANG_HOLD` / `TIMER_RESUME_HOLD`、設定リロード・panic で解放)。実装は別々: ① は `Vec<T>` と `EpisodeTracker`、② は `Active{scope, items}`。「前面窓が違えば捨てる」の判定が 2 か所(`HoldController::on_timer` と `ResumeHold::finish`)。③ は `INPUT_DEFER` 側。
  手書きの排他も見つかる: `lang_detect_window_in_scope`(`lang_detect_shadow.rs:~200`)が「実質 TSF ネイティブの窓は対象外(FOCUS_RESYNC が最初の打鍵を取り込み口で扱うため)」と **③ との重なりを避けるために対象外にしている**。② と ① の排他は無い(② の保持中は `resume_hold_on_key` が先に吸う順序で足りている、という読み。未確認)。
- 統合した場合に変わりうる挙動: 保持バッファ(スコープ照合+再生+期限)を 1 つの純粋な型に共通化するだけなら、観測できる挙動は変わらない。**解放の理由(Observed/Deadline/Cap/FocusChanged/PanicReset/ConfigReload の `ReleaseReason`)と、保持の開始条件・完了の札**は系統ごとに違うので統合しない。③ を ①② の型に寄せると、`FOCUS_RESYNC` の世代照合(ハード期限との競合を `compare_exchange` で 1 回に絞る契約)が崩れる危険がある → ③ は触らない。
- 擬似 IME: **今のハーネスでは検証できない**。`tests/support/harness.rs` は「通過マーク・`ImeApplyRequested` の往復・ForceGuard・TSF warmup・Engine の `on_input`」を写していないと自ら明記しており、打鍵の保持・`INPUT_DEFER`・タイマー・前面窓の識別(`ForegroundScope`)のモデルが無い。保持バッファの純粋部分は既に `awase-hook-core` と root crate の単体テストに在る(`half_width_resume_hold.rs` のテスト、`lang_detect.rs` のテスト)ので、型の共通化はそのテストを移して Linux で等価性を取れる。**足りないもの**: ①と②を同じ打鍵列で流して再生順・捨てる条件が一致することを見る「保持バッファの共通テスト表」。
- 判断: **C?(保持バッファの共通化のみ。2 つの型を 1 つに)**。削減は小さい(推定 -80〜-120 行、未測定)。ADR-156 の「大規模統合は不採用」と矛盾しない範囲(あれは `pending_deferred` 内の 2 窓口・`DeferredExecutionQueue<T>` の統合の話で、この 2 つの保持バッファはその後に入った)。①が既定で動かないので、**先に ① の `ime_on` を残すか(実機の一太郎確認が無い)を所有者が決める方が先**。①を撤去するなら ②③ だけ残って統合の話は消える。
- 再発ファミリー: defer/replay キューの解放条件(ADR-156 の行)・半角英数トグルの復元に当たる。

### MD-08 「窓の同一性」を表す印が 5 種類あり、遅れの軸が違う
- 場所と advance の契機(すべて読んで確認):
  1. `FocusEpoch`(`u64`、`PlatformState.focus.focus_epoch`)。`runtime/focus_tracking.rs:161` の `enter_focus_scope` で +1(**プロセスが変わったときだけ**。`awase-ime-core/src/state/ime_model.rs:468` のコメントが明記)。`FocusFence{epoch, hwnd}`・`ImmLikeTicket`・`AcceptedObservation` が使う(`ImmLikeTicket` 36 件)。
  2. `FocusGen`(`state/focus_gen.rs`、`Output::ime_mode_focus_gen`)。`gji_on_focus_change` → `on_ime_mode_focus_changed`(`output/mod.rs:398`)で +1。つまり `ir_post_focus_change_snapshot` の中=**デバウンス 50ms と非同期 prefetch の後**。`ActuationTarget`・probe_io・ime_refresh・`giveup_follow_decision`・`StageOrigin` が使う(`ime_mode_focus_gen` 54 件・`FocusGen` 70 件)。
  3. `ForegroundScope{pid, hwnd}`(`win32::foreground_scope()` を呼ぶたびに OS を読む。`foreground_scope()` は `platform_state.rs` 78 件・`shell.rs` 18 件ほか)。`post_bypass`・`resume_hold`・`lang_detect`・`StageOrigin`・`mode_key_pass` が使う。
  4. `tsf/gji_fsm.rs:43 FocusEpoch(u32)`(同名の別型。stale な `EndComposition` を弾く)。
  5. `ShiftConvGuardGen`(`focus_gen.rs`。フォーカス変更・SetOpen(true)・新しい hold で +1。`confirm_gate_deadline_override_ms` の所有権)。
  加えて `lang_detect_focus_in_sync`(`lang_detect_shadow.rs`)が「前面窓と awase の追跡が同じか」を独自に判定している。
- 重複していることの根拠: `raw_recovery_plan.rs`(BUG-194/ADR-246)の冒頭が、**`ime_mode_focus_gen` は窓切替直後にデバウンスと prefetch の後でしか進まず、窓切替直後の打鍵が張る段は「旧窓の世代 N・新しい窓 B」で刻まれる**ので前景窓を主な物差しにする、と明記している。つまり FocusGen は「遅れた印」で、同じ消費者が前景窓で補っている。
- 統合した場合: 「`WindowIdentity { epoch, gen, foreground }` を 1 つ捕獲して 1 つの比較関数で照合」に寄せられる形だが、**軸の遅れが違うことが意味を持っている**(上の BUG-194 round 2 N-M1)。1 つに潰すと誤破棄(リテラルが画面に残る側)か誤送信(旧窓の BS が新窓へ)の側に倒れうる。
- 擬似 IME: **足りない**。ハーネスの窓は `HwndId` と `epoch` の 2 つだけ(`harness.rs:391 focus_change`)で、`ForegroundScope`・pid・クラス名・デバウンス・非同期 prefetch の遅れが無い。`tests/warmup_gate_focus_scope.rs` は `ImeModel`+`ObservationStore` の focus-scoping 1 点だけ。
- 判断: **? (K 寄り)**。調べるなら「各消費者が実際にどの軸を必要としているか」の表を先に作る(本調査では 5 軸の消費者の全列挙まではしていない=未確認)。再発ファミリー: focus 遷移・IME belief・defer/replay に当たる。

### MD-09 窓切替直後の IME 状態の読みが 5 系統ある
- 場所と起動契機:
  - ImmCrossProbe/focus(`focus_tracking.rs:~800`。Standard×日本語 IME のとき、FocusChanged 直後に `read_ime_state_full_async` を spawn、High で観測を書く)。
  - FocusProbe(`key_pipeline.rs:430 kp_stage_focus_probe`。`consume_focus_barrier` で**最初の打鍵**に spawn、`read_ime_state_fast_async`、`apply_focus_probe`)。
  - ObserverPoll(`ir_stage_observe`、`TIMER_IME_REFRESH`=デバウンス 50ms/ポーリング 500ms)。
  - FOCUS_RESYNC の conv 読み(`kp_trigger_focus_resync`、TsfNative)と idle_conv_check(同じ `kp_stage_idle_conv_check_inner` の別フラグ)。
  - `gji_on_focus_change` の IMC ヒント読み(`platform.rs:~650`、`update_ime_mode_hint_from_imc`、confirmed を立てない)。
  - (参考)ADR-205 の外部変化の監視窓・mozc 共有メモリ(ADR-268、`mozc_shm_on_tick`)。
- 重複の根拠: Standard の窓では ImmCrossProbe/focus と FocusProbe が**同じ API で同じ窓の開閉を読み**、どちらも High の観測として書く(送信元だけが `ImmCrossProbe` と `FocusProbe` で違う)。FocusProbe は「窓切替直後は IME 状態が落ち着いていない」ため最初の打鍵時に読み直す意味があるが、その必要性を測った記録は見つけていない(未確認)。
- 統合した場合: ImmCrossProbe/focus を撤去すると、Standard 窓で最初の打鍵までの間の belief が ObserverPoll 頼みになる(ADR-212/188 の記述では「最初のキーから正しい belief」が目的)。FocusProbe を撤去すると最初の打鍵の時点の読み直しが無くなる。
- 擬似 IME: ハーネスに `prefetch_read`(`harness.rs:500`)・`observe(Source::ImmCross|Poll)` があり、**読み値の流入と belief の結果の比較は Linux でできる**。足りないもの: 読み戻しの遅延と同時に走る複数の読みの順序(`set_readback_lag_ms` は 1 つの遅延値だけ)・FocusProbe の「最初の打鍵で 1 回だけ」のトリガ(`consume_focus_barrier`)・`FocusGen`/`ForegroundScope` による棄却。
- 判断: **?**。決める事実: 実機の journal で、同じ窓切替の中で ImmCrossProbe/focus と FocusProbe が**異なる値**を返した件数(返さないなら冗長)。journal は `ObservationSource` ごとに記録されているので `bug-report-latest` で集計できる(実測は未実施)。再発ファミリー: IME belief に当たる。

### MD-10 同じ窓切替の「適用済み(applied)の先同期」が 2 か所
- 場所: `runtime/focus_tracking.rs:~840 presync_applied_open_on`(`on_focus_process_changed` の中。`record_confirmed(true)` と GJI なら `gji_on_ime_on`)と、`runtime/ime_refresh.rs:~828 ir_post_focus_change_snapshot`(`record_confirmed(effective_open)`)。**どちらも `may_mirror_applied_on_focus`(`awase-ime-core/src/state/applied_mirror_plan.rs:20`)を通す**。同 doc が「写しを行う殻の到達可能な 3 箇所(これらと `ir_align_placeholder_desired`)」と列挙し、`architecture_guard.rs` が固定している(既知)。
- 確認した差: presync は `!desired_is_placeholder()` を要求する(BUG-163。「閉で起動した IME を awase が開けない」ため)。`ir_post_focus_change_snapshot` の写しにはその条件が**見当たらない**(読んだ範囲。`record_confirmed` → `confirm_applied(open)` に placeholder の分岐が無いことまでは確認していない=**未確認**)。両方とも同じ窓切替で実行されうるので、`ir_post` が placeholder の ON を `applied=Confirmed` に写している可能性がある。**これは E/C ではなく、要確認の不整合**(BUG-163/ADR-262 の趣旨との食い違いの疑い)。
- 判断: **?(統合の前に、`ir_post` 側に placeholder ガードが要るかの確認。要実機または journal)**。再発ファミリー: IME belief・warmup に当たる。擬似 IME: `align_placeholder_desired` はハーネスが呼べるが(`harness.rs` 冒頭の列挙)、`record_confirmed` と `ir_post` の写しは「ImeApplyRequested/applied の往復」に含まれ**ハーネスは写していない**。

### MD-11 `ir_execute` の同期モード(`IoMode::Sync`)が 3 か所だけ残っている
- 場所: `runtime/ime_refresh.rs:15-60`(`IoMode::{Sync, Prefetched}`)、`runtime/mod.rs:1079 refresh_ime_state_cache` → `run_ime_refresh`。呼び出し元: `app/bootstrap.rs:841`(起動)、`message_handlers.rs:1085`(`WM_INPUTLANGCHANGE`)、`runtime/mod.rs:2403`(`panic_reset`)。それ以外は `spawn_ime_refresh`(Prefetched)。`git grep -n refresh_ime_state_cache -- crates src` で確認。
- 重複の根拠: 同じパイプライン `ir_execute` を、同期(`detect_and_update_focus`→`read_focus_snapshot` が `run_with_timeout` 300ms でメッセージループ上でブロック)と非同期(`run_focus_probe_async`)の 2 通りで実行する。`focus/probe.rs` の `read_focus_snapshot` と `run_focus_probe_async` は本体がほぼ同じ(25 行ずつ)、`ime_refresh.rs::poll_and_classify_ime`(同期)と `classify_fetched_snapshot`(prefetched)も対。
- 統合した場合: `WM_INPUTLANGCHANGE` と `panic_reset` も `spawn_ime_refresh` に寄せれば同期モードは起動時 1 回だけになる。変わりうる挙動: **panic_reset の直後に refresh が走る時機が非同期になる**。`panic_reset` は「`apply_panic_reset` が書いた ime_on=true を stale な `observe()` が直後に上書きしないよう force_on_guard で 1 サイクル保護」と書いており、同期実行を前提に順序を組んでいる(`mod.rs:2378`)。非同期にするとこの保護の時間窓が変わるので BUG-182 系の確認が要る。`WM_INPUTLANGCHANGE` は `invalidate_engine_context` の直後に同期で読む順序。
- 擬似 IME: 不可(同期/非同期の差は Win32 のブロック時間)。
- 判断: **?(C 候補)**。削減は `probe.rs` の約 25 行と `IoMode::Sync` の分岐。測るもの: panic_reset 後の最初の打鍵で ime_on が再び false に落ちないこと(`e2e-ime` の panic ターゲットがあるかは未確認)。再発ファミリー: IME belief・focus 遷移に当たる。

### MD-12 `focus::tracker::FocusTracker` と `runtime::focus_tracker::FocusTracker` が同名で別物
- 場所: `focus/tracker.rs:19`(470 行。`CurrentFocus`・`FocusCache`・`ForceOverrides`・`ImmCapabilityStore`・`InjectionModeStore`・`HwndImeCache` を束ねる)と `runtime/focus_tracker.rs:22`(145 行。**中身は IME sync キーの 3 つの `Vec<VkCode>` と、`enrich_ime_relevance`、純粋な `decide_imm_capability` だけで、フォーカスの状態を持たない**)。さらにフォーカスの状態は `platform_state.focus: FocusStore`(`platform_state.rs:2158`)にも在る。
- 判断: **S(命名・置き場所の整理)**。`runtime/focus_tracker.rs` は `sync_keys`(または key 設定)と `decide_imm_capability` に分け、`decide_imm_capability` は `focus/learning_store.rs` へ(両方とも純粋)。`mod.rs:317` の doc と 35 か所の参照が変わる(同ファイルの doc が数えている)。`git grep -n "focus_tracker\b" -- crates/awase-windows/tests` → 0 件なので文字列ガードは無い。削減行数はほぼ無い。混乱の除去が目的。再発ファミリーに当たらない。

### MD-13 `injection_hint` と `injection_hint_for`、フォーカスでの `update_injection_mode` が 3 経路
- 場所: `focus/tracker.rs:95-125`(`injection_hint` は `self.current` を引き、`injection_hint_for` は引数の pid/class を引く。本体は同じ 2 段 [override → 学習した class])。`update_injection_mode` の呼び出し: `runtime/mod.rs:1791`(`on_window_focus_event`、WinEvent の hwnd で**デバウンス前**に先行)・`focus_tracking.rs:95`(`apply_focus_probe_result`、確定した hwndFocus)・`focus_tracking.rs:138`(`establish_initial_focus_scope`、起動時)。`git grep -n update_injection_mode -- crates/awase-windows/src` で確認。
- 重複の根拠: 先行(mod.rs:1791)は「デバウンスが済む前にキーが来ても injection_mode を stale にしない」ため(コメント明記)。確定側(95)はプローブした実フォーカス窓で補正する(lang_check のコメントが「WinEvent の hwnd は実際のフォーカスと限らない」と書く)。
- 判断: `injection_hint` は `injection_hint_for(self.current.pid, &self.current.class_name)` に寄せて 1 つにできる(**C、約 -10 行、確実**。`is_focused()` の早期 return は呼び出し側で足す)。3 経路の先行/確定は **K**(上の理由で意味がある)。再発ファミリー: warmup/focus 遷移。

---

## K(残す、または今は触らない)

### MD-14 `FocusCache`(`focus/cache.rs`)の Automatic/UserOverride と TTL
- `kind_classifier::resolve_focus_kind` の順序(config override → cache → engine 活性 → `classify_focus`)の判断は `focus_resolve_plan.rs::decide_resolution` に純粋で在る。cache は `(pid, class)` で 5 分(Automatic)/24 時間(UserOverride)。cache は**メモリのみ**(`cache.toml` に載るのは `[imm_capability]` と `[injection_mode]`。`learning_store.rs:3`)で、`UserOverride`(トレイ/ホットキーの `toggle_app_override`、`mod.rs:1387`)の 24h TTL は再起動で消えるので実質プロセス寿命。
- 観察(E ではない): 現状 `DetectionSource` は 2 値で、`UIA 非同期` 撤去後の名残の doc(「UIA 非同期判定を省略できる」)が古い。cache の目的は classify_focus(ワーカー 300ms)の省略。cache を外すと、同じ窓クラスでフォーカスのたびに `classify_focus`(MSAA 含む)がワーカーで走る(**未確認**: 実機の頻度)。K。doc だけ直せる(S)。

### MD-15 学習の 3 ストア(`ImmCapabilityStore`・`InjectionModeStore`・`FocusCache`)と静的表(`class_names.rs` 808 行)・設定 override(`force_tsf`/`force_vk`/`input_relay_apps`/`force_text`/`force_bypass`/`disable_apps`)
- 「この窓は IMM が読めない/TSF だ」を言う経路が静的+学習+設定で 3 層あるが、それぞれ入力が違う: 静的=クラス名、学習=実測(`ImmGetDefaultIMEWnd` NULL の連続・miss 閾値超え、BUG-56/107/108、ADR-262 の `demoted_by_learning` が ADR-263/271 の対象条件)、設定=利用者の明示。ADR-262/263 が `demoted_by_learning` を足場にしているので **K**。
- 別の軸の混乱(S): `AppKind`(Win32/TsfNative/Uwp。`detect_app_kind`)の `TsfNative` は **Chrome 系**、`AppImeProfile::TsfNative` は **WezTerm 系**(Chrome は `Imm32Unavailable`)。名前が逆向きに重なる。`app_kind` の読み出しは `git grep -n app_kind -- crates/awase-windows/src` で 32 行。統合は ADR-089/098 の領分(未確認)。

### MD-16 `InputBarrier::FocusTransition`(`input_barrier.rs`)と `focus_settle_ms`(100/200/500、`app_ime_policy.rs`)
- 現存の消費者(`git grep -n "is_focus_transition_settling\|consume_focus_barrier"` で確認): drift correction の settle 延期(`ime_refresh.rs:947 ir_observe_drift_facts`)と `kp_stage_focus_probe` の「最初の打鍵で 1 回」トリガ(`key_pipeline.rs:435`)。`SetOpen` の settle 中の除去は ADR-213 P2d-2 で撤去済み(`executor.rs:124` のコメント)。
- 観察: `focus_settle_ms` は今や drift correction の延期にしか効かない(`ime_apply_should_defer` の唯一の呼び出しが `ime_refresh.rs:947`)。AppImePolicy の 3 値(100/200/500)がこの 1 経路のためだけにあるなら、定数 1 つへ畳める余地がある(**未確認**: `AppImePolicy` の他フィールドとの結び付き、`ime_profile_driver.rs` の ADR-090 決定 F-2 が `focus_settle_ms` を K 非依存と固定している)。**?**(tuning-constants の実測義務に当たる。測るもの: drift correction が settle 延期で実際に待った回数と時間の journal)。

### MD-17 hook.rs のラッチ群(物理押下・親指・Alt なりすまし・foreign ctrl)と `clear_hook_latches_for_app_disable`/`..._for_watchdog_reinstall`/`reset_physical_key_state` の 3 変種
- 既に ECRS(コード単位)の F-13 で「別々に持つことを doc とガードが意図して固定」と見送り済み(`architecture_guard.rs::app_disable_leave_edge_clears_only_ctrl_and_shift_not_alt_or_win` が関数本体を走査)。再報告しない。**K**。再発ファミリー: 物理キー押下ラッチ。擬似 IME: hook スレッドの `AtomicX` 群はハーネスに無い(フックコールバック全体が cfg(windows))。

### MD-18 `FOCUS_RESYNC` の gate 自体、`PassthroughQueue`(`runtime/transport.rs`)、`OUTPUT_GATE`
- 窓切替の待ちと出力中の待ちは別の問題(③は belief の確認待ち、`PassthroughQueue` は出力ガードと KeyUp 対称性、`OUTPUT_GATE` は自分の出力中)。ADR-156 の結論(大規模統合は不採用)を超える根拠は見つからなかった。**K**。
- 小さい S: `PlanPhysicalKey` トレイトは `ActiveImeKind → ImeKindId` の変換だけの薄い殻(`transport.rs:93-113`)。呼び出しは `key_pipeline.rs:244,1336` の 2 か所だけで、`plan_core(..., active_ime_kind.into())` を直接呼べば 25 行ほど減る(`transport.rs::plan_shell_tests` のテストも消える)。`git grep -n PlanPhysicalKey -- crates` → 3 ファイル。architecture_guard が `PhysicalKeyDisposition::plan` という名前を走査するかは**未確認**(doc コメントに出る箇所が複数ある。走査があれば名前を残して委譲にする)。再発ファミリー: 物理 IME キーの Suppress/Allow 配送(`physical_disposition.rs` ではなく殻の薄い層なので、判断は変わらない)。

### MD-19 `lang_check`(ADR-223)・`layout_observer`・`kana_lock`・`mozc_shm_shadow`
- `lang_check` は打鍵ごとに入力言語を読み(`ime_relevance.layout_japanese`)、`is_japanese_ime` を更新して読み直しを 1 回予約する本番機構(BUG-183)。生きている。**K**。
- `mozc_shm`(ADR-265/268)は belief に High で接続済み(メモリの記述)で、本調査の対象外(mech-c の領分)。

### MD-20 `focus_hwnd_observed`(`win32.rs:544`、ADR-246 Q3 の観測版)
- 本番の呼び出しは `output/mod.rs:1372` の 1 か所(literal 検出と予約あり flush の 2 回だけ、30ms)で、結果は journal の `focus_hwnd_changed` にだけ出る(判断に使わない)。`get_gui_thread_info_with_timeout` と 25 行ほどが重複するが、BUG-194 の ADR が「代用値で変化頻度を過大に測るため別関数」と理由を明記している(B1)。**K(実害の報告待ち)**。決める事実: 実利用の `DeferredRecoveryFlush` で `focus_hwnd_changed=Some(true)` が出る頻度。

---

## 窓切替で働く待ち・凍結・ラッチの一覧(「同じ窓切替で複数の機構が似た待ちをしている」の整理)

| 時刻 | 機構 | 待つ/凍結する対象 | 期間 | 既定で有効か |
|---|---|---|---|---|
| WinEvent 直後 | `InputBarrier::FocusTransition`(`on_window_focus_event`) | 外部観測の信頼度・drift 延期・最初の打鍵で FocusProbe | `focus_settle_ms` 100/200/500 | 有効 |
| WinEvent 直後 | `injection_mode` の先行更新 + `TIMER_TSF_GATE` | TSF 出力の準備待ち | `WARMUP_TIMEOUT_MS` | 有効 |
| +50ms | `TIMER_IME_REFRESH`(`focus_debounce_ms`)→ 分類 → `on_focus_process_changed` → `ir_notify_focus_changed` → `ir_post_focus_change_snapshot` | 分類・belief の先同期・GjiFsm の cold 化 | デバウンス 50ms | 有効 |
| 最初の打鍵 | `FOCUS_RESYNC`(TsfNative) | 打鍵を defer して conv を読む | ≤ 100ms | 有効 |
| 最初の打鍵 | `resume_hold`(MS-IME 本体の戻り復元) | 打鍵を保持して R1 観測を待つ | ≤ 100ms・4 打鍵 | 有効(`half_width_alnum_toggle=all`) |
| 最初の打鍵 | `lang_detect` の保持(観測の無い窓) | 打鍵を保持して日本語/英数を判定 | ≤ `max_ms`・`max_keys` | **既定は observe(保持しない)** |
| 最初の打鍵 | `FocusProbe`(`consume_focus_barrier`) | 非同期 IME 読み | — | 有効 |
| 窓切替後 300ms | `GJI_SETTLE_GRACE_MS` / warmup grace(`FocusProbeGraceFlags`) | probe の false 観測を抑制 | 300ms | 有効 |
| 窓切替の都度 | `hwnd_ime_cache` の復元(`HwndCacheRestored`)・TsfNative の SSOT 持ち越し・`reset_stale_ime_on_for_imm_broken`(HeuristicDefault(true))・`assume_closed_for_new_thread`(ADR-212 P2) | 初期 belief の選び方 | — | 有効 |

→ 「最初の入力までに belief を整える」目的の機構が 8 本あり、窓の型(Standard/TsfNative/Imm32Unavailable/demoted)でほぼ**排他的に使い分けられている**(手書きの排他が `lang_detect_window_in_scope` のコメントに見える)。型ごとの表(`window kind × 機構`)が 1 か所に無いのが、統合を判断しにくくしている最大の理由。**まず表を `docs/` に起こすのが次の一手**(撤去ではなく、この表の作成)。

## 擬似 IME に足すと検証できる範囲が広がるもの(優先順・規模の見積り)

読んだ `tests/support/{harness,pseudo_ime,invariants}.rs`・`closed_loop_scenarios.rs`・`warmup_gate_focus_scope.rs` の範囲では、今のハーネスは **`ImeStateHub`/`ImeModel` の belief と書き込み命令の層**(焦点は `HwndId`+`epoch` の 2 値、時計は仮想、読み戻しの遅延は 1 値)で、**窓切替の機構(MD-07〜13)の入出力は写していない**。

1. **窓モデル(中、約 200〜300 行)**: `ForegroundScope{pid,hwnd}`・クラス名・`AppImeProfile`(静的+学習降格)・`FocusGen`/`FocusEpoch` を別々の遅れで進める(デバウンス 50ms+prefetch 遅延を仮想時計で)。MD-08(同一性の印)・MD-09(読み 5 系統)・MD-10(applied の先同期)の等価性の前提。`focus_change_to_new_thread_unreadable` が既にあるので、`focus_change_to(pid, class, profile)` を一般化する形。
2. **入力側の保持・defer のモデル(中、約 150〜250 行)**: `INPUT_DEFER`・`TIMER_*` の仮想タイマーと、保持バッファ(MD-07 の ①②)・`FOCUS_RESYNC` の世代付き gate を、**取り込み口の順序**(`app/mod.rs:710-735`)ごとにハーネスから打鍵列を流して「出力される打鍵の順序・捨てられる数」を比べる。これができれば MD-07 の共通化の等価性が Linux で取れる。
3. **出力層の送信列の記録(大、約 400 行〜)**: `output/*` の `SendInput` 列。MD-13 の injection_mode 先行更新・`tsf_warmup_coord` を含む機構の等価性はこれが要るが、他の担当(mech-b)の領分と重なる。
4. **同時に走る複数の読みと読み戻し遅延の順序(小〜中、約 100 行)**: `set_readback_lag_ms` を読みの種類(ImmCrossProbe/FocusProbe/ObserverPoll)別にし、`prefetch_read` を非同期の完了順で並べ替えられるようにする。MD-09 の「同じ窓切替で異なる値を返した」状況を人工的に作れる。
5. **`applied` の往復(中)**: `ImeApplyRequested`/`applied` の `Unknown`→`Confirmed` を写す(ハーネス冒頭が「写していない」と明記)。MD-10 の placeholder ガードの要否を Linux で確かめるのに必須。

実機でしか決まらないもの(CI の既存ターゲットで測れるかを括弧で): MD-03 の副作用なし(windows-build の e2e)、MD-05/06 の journal 件数(`bug-report-latest`)、MD-09 の ImmCrossProbe と FocusProbe の食い違い件数(実利用の journal)、MD-11 の panic_reset 後の最初の打鍵(e2e-ime の panic 系ターゲットの有無は**未確認**)。
