# ADR-245 追補 2(改訂 r3)Opus レビュー round 3

対象: `/home/cuzic/rust-nicola-wt-adr245`、`feat/adr245-pr2-pr3-rebased`、`426e0c58`(`git log -2 --oneline` で確認)、`docs/adr/245-half-width-alnum-restore-on-return.md` の追補 2(r3)。
追加で読んだコード: `ime.rs::modify_conv_mode`(364-384)・`detect_ime_open_for_hwnd`(404-413)・`set_ime_conv_for_target`(1236-1290)、`key_pipeline.rs::kp_stage_focus_probe`(421-470)・`kp_run_inner` の Phase A(83-108)、`runtime/lang_detect_shadow.rs::lang_detect_focus_in_sync`(220-227)、`message_handlers.rs` の `TIMER_IME_OFF_RESCUE`(539-557)と `deliver_key_event`(148-256)。

件数: Blocker 0 / Must 4 / Should 3 / Nit 2

round 2 の対応の確認: B-1'(attempt==0 の直後)・M-2・M-3・M-4・S-1・S-2・S-4・N-1〜N-4 は、方針として反映されている。
M-1 は棄却の規則として入ったが、基準の時刻の取り方では (ii) の経路を棄却できない(下の M-1')。

---

## Must

### M-1' floor を「Resume の起案時刻(ms)」で取ると、(ii) の FocusProbe 起点の読みは棄却できない。R1 を予約しないときにも floor を立てると、PR 2 より悪くなる
- 根拠: (ii) の経路は次のとおり。`kp_stage_focus_probe`(`key_pipeline.rs:421-470`)は同じ打鍵の中で `spawn_local` するだけで、まず `read_ime_state_fast_async` を await し、その完了後に `apply_focus_probe` が ImmCrossProbe を `spawn_local` する(`:3263`)。
  その ImmCrossProbe の `read_started` は、タスクが最初に poll された時点(`:3267`)に取られる。つまり **Resume の段(同じ打鍵の同期部分)より必ず後**になり、`read_started < floor` は偽で通ってしまう。
  一方、読み自体は起案から約 1ms 後に走り、書き込みの完了(2.8〜6.0ms)より前なので、書き込み前の「閉・0x10」を読む。round 2 の M-1 が一番危ないとした経路(`InputModeObserved{ObservedEisu, High}` も出す側)が、この規則をすり抜ける。
  逆に (i)(フォーカス時、`focus_tracking.rs:806-833`)は、打鍵より前に spawn されるので必ず棄却される。決定 2 の「予約の前提」が偽で R1 を予約しない場合も floor を立てると、A の唯一の High の観測を捨てて R1 も来ない。PR 2 より belief の追随が遅くなる。
- 失敗シナリオ: 応答の遅い窓で (ii) の読みが 10ms かかり、R1(書き込み後、0.5ms)より後に着く。(ii) は floor を通り、閉・ObservedEisu で上書きする。R2 は R1 の通知で既に解放されているので、再生した `w` が素通しになり、2 打鍵目以降も止まる。
- 提案:
  - (1) floor は「R1 の書き込みが `Written` を返した時刻」にし、R1 が `attempt == 0` の `Written` の直後に書く。基準を「起案」ではなく「書き込みの完了」にすれば、完了前に始まった読み(=書き込み前の状態を見うる読み)だけを捨てられる。
  - (2) 比較は `<` ではなく µs(`hook::now_timestamp_us`)で行うか、ms のまま `<=` にする。後者なら R1 自身の記録は floor の比較を通さない、と明記する(R1 の `read_started` は同じ ms になりうる)。
  - (3) floor を書くのは R1 を予約したときだけにする(予約しなければ (i)(ii) は今までどおり)。
  - (4) floor は解除しなくてよい。後から始まる読みは必ず floor より後なので、古い floor が後の正当な読みを棄却することは無い(team-lead の問い 2 への答え)。その理由を 1 行書いておく。
  - 単体の表に「(ii) の読みが floor の後に始まったが、書き込みの完了より前」を足す。基準を (1) にすれば、この行は棄却になる。

### M-2' R1 の読みが「閉」を返すことがありうる。そのとき何を記録し、どう解放するかが決まっていない
- 根拠(問い 1 の確認): `Written` は、`modify_conv_mode` が IMC_GETCONVERSIONMODE で読み、IMC_SETCONVERSIONMODE の `SendMessageTimeout`(50ms)が戻ったことを意味するだけで、読み戻しは含まない(`ime.rs:364-384`)。`new_conv == conv` なら、書かずに `Written` になる(`:374-375`)。
  R1 の `read_ime_state_full` は、同じ IME 窓へ IMC_GETOPENSTATUS(`ime.rs:404-413`)と conv の probe を送る。同じ窓のスレッドで SET の後に処理されるので、**IMC の値としては**書き込み後の conv を返す。
  しかし半角英数トグル中の A は `imm=C`(閉)と読まれる。開閉は conv の書き込みで直接変わる値ではなく、MS-IME(TSF)が後から開くのか、SET の処理の中で開くのかは、コードからは分からない。typed2 の flip +4ms(8 件)は、読みがたまたま書き込みの後に来た例にすぎず、「直後の読みが常に開を返す」ことの保証にはならない。
- 失敗シナリオ: R1 が「閉・0x19」を読み、ImmCrossProbe/High で閉を記録して、`Observed` で R2 を解放する。最初の打鍵は `w` で、2 打鍵目以降も止まる(PR 2 と同じ)。R1 の観測の時刻は (i)(ii) より新しいので、その後の正しい読みを上書きはしないが、R1 は空振りになる。
- 提案: (a) まず CI で R1 の読んだ値(`ime_on`・conv)をログに出し、分布を見てから決める(`[ImmCrossProbe/resume] IME=` の件数をログ断片検査で数える)。
  (b) 閉が出るなら、`Observed` を 2 つに分ける。`Observed(open=true)` は解放する。`Observed(open=false)` は 1 回だけ読み直す(期限内。間隔は測ってから決め、`tuning.rs` に置く)。それでも閉なら記録して解放する。
  (c) 書かずに `Written` になった場合(`changed == false`。A の conv が既に 0x19)も R1 は読む、と 1 行書く。

### M-3' 完了の札は、`Drop` で `with_app` を呼ばずに post だけを行い、保持の世代を持つ
- 根拠(問い 4): 札の `Drop` は、executor の終了、タスク内の panic の巻き戻し、`with_app` の中でのタスクの破棄でも走る。`Drop` の中で `with_app` を呼ぶと、借用中なら `None`(通知が消える)、巻き戻しの途中に panic すれば abort になる。
  また、札が保持より長く残ることがある。たとえば Deadline で解放した後に、遅れて札が通知する場合である。そのときに次の Resume の保持が始まっていると、古い札が新しい保持を早く解放してしまう。
- 失敗シナリオ: 窓 A1 の Resume が Deadline で解放される → すぐに窓 A2 の Resume で新しい保持が始まる → A1 の復元タスクが遅れて終わり、札が通知 → A2 の保持が、A2 の R1 を待たずに解放される(A2 の最初の打鍵が `w`)。
- 提案: 札の `Drop` は `post_to_main_thread_with(WM_RESUME_HOLD_DONE, hold_id, reason)` だけを行い、panic しない(`with_app` を呼ばない)。受け側は `with_app_or_repost_with` で処理し(`lib.rs:255-258` の既存の流儀で足りる)、`hold_id` が今の保持と一致するときだけ解放する。
  通知の順序は「R1 の記録 → 札の drop」で固定する(drop を記録より先に走らせない。記録を閉じたスコープの中で札を明示的に `drop` する)。終了時は post が捨てられるだけで、保持はプロセスとともに消える(問題ない)。

### M-4' `deliver_key_event` の先頭で足すと、ime-off-rescue の再生の origin が失われ、Phase A の入れ子の中で始まった保持は今の打鍵に追い越される
- 根拠(問い 3):
  - (1) `TIMER_IME_OFF_RESCUE` は `deliver_key_event(.., KeyOrigin::ImeOffRescueReplay)` を直接呼ぶ(`message_handlers.rs:539-557`)。先頭で保持へ足すと、再生は `INPUT_DEFER` → `DeferredReplay` になり、`skip_rescue_defer=true`(`replay_ime_off_rescue_event`)と NonText の免除(`:198-199`)が消える。Phase B がもう一度 50ms 保留しうるので、IME OFF の意図が遅れたり化けたりする。
  - (2) `kp_run_inner` の Phase A は、保留した打鍵を入れ子の `kp_run_inner(pending, true)` で処理する(`key_pipeline.rs:100`)。この入れ子の打鍵が Resume の契機になると、保持は入れ子の中で始まる。しかし今の打鍵は、`deliver_key_event` の先頭を既に通過しているので足されず、そのまま Engine と reinject に進んで、保持した打鍵を追い越す。
  - lang_detect の再生・TsfGate の再生は `INPUT_DEFER` 経由(`DeferredReplay`)なので、先頭で足しても順序は保たれる(時刻順に取り出されるため)。衝突は無い。
- 提案: 保持には `(event, origin)` を積み、再生では origin を保つ(`ImeOffRescueReplay` は再生のときも `replay_ime_off_rescue_event` を通す)。もしくは、`ImeOffRescueReplay` は保持へ足さずに先に処理し、保持の先頭に置いて扱う。
  (2) は、`kp_run_inner` の中で保持が始まったかを Phase A の直後に確かめ、始まっていれば今の打鍵も保持の末尾へ入れて `Consumed` を返す、と決定 3 に 1 行書く。

---

## Should

### S-1' 1 回目の書き込みが失敗したら R1 を一度も行わないので、「R1 単独で 2 打鍵目から効く」が失われる
- 決定 3 の札は、1 回目が `Failed`/`Aborted` なら `FirstWriteFailed` で通知する。R2 の解放としてはこれで正しい。しかし R1 は 1 回目の直後にしか行わないので、160ms 後のリトライで `Written` になっても、belief は追随しない(現状の 27/35 と同じ)。
- 提案: R1 の「読む」は、最初に `Written` になった試行の直後に 1 回行う(R2 の札はその前に `FirstWriteFailed` で通知済みでよい)。こうすれば、2 打鍵目以降の効果は失敗の後でも残る。「1 回だけ」の意味を「試行番号 0」から「最初の `Written`」に変える。

### S-2' 予約の前提の `lang_detect_focus_in_sync()` は、追跡側の root が 0 のときも真を返す
- `tracked_root == 0 || tracked_root == foreground.hwnd`(`lang_detect_shadow.rs:226`)なので、pid が同じで root が未確定なら一致とみなす。同一プロセスの別の窓(Q4)では、B の hwnd の fence を採りうる。
- R1 の宛先の誤りは `clear_on_focus_change`/`update_focus_window` で消えるので実害は小さい。それでも、「同じ式を使う」と書く以上は、この緩さを引き継ぐことを未確認の節に 1 行書く。あるいは、R1 では `tracked_root != 0` も要求する。

### S-3' 決定 6 の読みの所要時間の根拠がずれている
- `[ime-io] cross_process cmd=0x0001` は IMC_GETCONVERSIONMODE で、`modify_conv_mode` の書き込み前の読み(`ime.rs:371`)にも同じログが出る。R1 の `read_ime_state_full` は、GETOPENSTATUS(0x0005)と conv の読みに加え、場合によっては `detect_kana_for_hwnd`(`ime.rs:605-612`)も送る。所要時間は 1 回の probe の合計の 2〜3 倍になりうる。
- 「典型 ~7ms」の数字を、R1 を入れた後の CI で `[ImmCrossProbe/resume]` の起案からの時刻として測り直す、と書いておく。

---

## Nit

### N-1' 契機の表の行 4 と行 6 の整合
- 行 4(窓の変化。保持の中は全部捨て、後から来た Up は通す)と行 6(保持の外の Up は足さない)で、round 2 の S-1 の矛盾は解けた。「後から来た Up」の判定には、捨てた Down の同一性を覚えておく必要がない(通すだけなので)ことも 1 行書くと、実装が簡単になる。

### N-2' `deliver_key_event` の先頭の位置
- 先頭で足すなら、`[[keymap]]` のラッチの判定(`message_handlers.rs:165-183`)より前になる。行 6(保持の外の Up は通す)の判定は、ラッチより前でも結果が同じ(ラッチ済みの Up は保持の外の Down の Up なので通る)。それでも、順序を architecture_guard で固定するなら「行 6 の判定 → 保持へ足す → ラッチ」の順にする、と書いておく。
