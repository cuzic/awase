---
id: ADR-258
title: |-
  BUG-200: BUG-074(ADR-227 (i))の追随が実機で効かない — 先に原因を測る
summary: |-
  BUG-200: 外部から実 IME が閉じ belief が ON のままのとき、Imm32Unavailable × GJI の窓で打った文字が raw TSF リテラル回収の Backspace で消える。実機(dragonflyg4)で人工再現(`--form=tsf --mode=drift-on`、本文が空 4/4)。
  これは BUG-074 / ADR-227 と同じ構成で、ADR-227 (i)(give-up を読み直しのきっかけにして追随する、PR #480、CI 検証済み・実機未検証)が実機で働いていない証拠かもしれない(`[external-change]`・`Engine deactivated` が出ない)。
  初版の D1・D2・D3 は取り下げた。D0' で、追随(ADR-227 (i))は GJI × Imm32Unavailable × 明示意図 ON では実機でも働くと分かった(最初の再現で追随しなかったのは、ハーネスの注入が明示意図に昇格しなかったため)。残る害は、追随の前に最初の1打鍵が回収の Backspace で消えること。第一候補は ADR-227 (i) を初回の回収まで広げること。
status: |-
  起草(2026-10-12)。Opus レビュー(重大5件)を受けて改訂し、D0' を実測した(同日): 追随は設計どおり働く。残る害は最初の1打鍵の消失。修正方針は案 D5'(cold の段の開始時に追随を arm し、判定の時点で閉なら回収しない)。Opus 設計レビュー(条件付き可・K1〜K3)を反映済み。段階0(観測だけ + 偽陽性の測定)が先。実装なし。
related_adr:
  - "ADR-227"
  - "ADR-205"
  - "ADR-246"
  - "ADR-212"
  - "ADR-100"
  - "ADR-188"
---

# ADR-258: BUG-200 — BUG-074(ADR-227 (i))の追随が実機で効かない

## 背景

所有者の報告(2026-10-12): 「IME OFF・エンジン ON のとき、半角英数を入れると、その英数が消された」。自然発生は再現しなかったが、
[BUG-200](../known-bugs/BUG-200.md) のとおり実機で人工再現できた。この構成は [BUG-074](../known-bugs/BUG-074.md) / [ADR-227](227-bug074-giveup-as-closed-ime-evidence.md) と同じで、
ADR-227 (i)(PR #480、`900a5f3e`)は「CI 検証済み・実機未検証」のまま。**BUG-200 は、(i) が実機で働いていない(または効かない条件がある)ことの証拠**として扱う。

## 事実(`origin/develop` `7812c9df` の開発ビルド、dragonflyg4、GJI、2026-10-12)

- **再現**(各 n=4、2回走行): `typing_stress --form=tsf --mode=drift-on`。IME を ON にそろえ(ハーネスは物理の `VK_IME_ON` を送る。これで awase に明示意図 ON が付く**はず**だが、設計意図で、確かめていない=Q2)、ハーネスが awase を経由せず実 IME を閉じる(`WM_IME_CONTROL`)。
  2 秒待っても awase は ON のまま(`[external-change]`・`Engine deactivated` が出ない)。NICOLA 単打1回で本文が空(4/4、2走行とも)。同じ手順の `--form=edit`(IMM)は `w`(英字が残る、4/4)。
- **窓の種別**: 再現した窓は `Chrome_RenderWidgetHostHWND`、`profile=Imm32Unavailable`、`himc_null=true`(`[comp-probe]`)。**TsfNative ではない**。
  (初版はこれを TsfNative と書いて混同した。ADR-227 の背景にも、`--form=tsf` の窓は Imm32Unavailable と書いてある。)
- **ログ**(13:28:19Z): `[raw-tsf-literal] ... raw TSF literal suspected → backspace ×1 + re-send "ka"`、続いて `consecutive raw-tsf-literal (count=2..5) → giving up, backs=1 cleanup only (no re-send)`。
  give-up は 4 回続いたが、`ir_follow_after_literal_giveup` の結果は `awase.log` に出ない(`GiveUpFollow` は journal にだけ残る)。
- **開閉の読み**: この窓の種別(`is_tsf_native_window` でない)では、runtime の prefetch が別プロセスの窓へ `IMC_GETOPENSTATUS` を送って開閉を読む(`ime.rs::read_ime_state_full`、ADR-205。GJI では実 Chrome で 1→0 が読めることが測定済み。
  MS-IME × 実 Chrome は常に 0 で信用できないため除外)。回収のアーム(`probe_io.rs`)自体は読まないが、ADR-227 (i) はこの prefetch の読みで追随する設計。
- **TsfNative(Windows Terminal)**: ADR-227 の D0-5(BUG-188、run 37237143414)では、外部クローズ後も give-up が起きない(`giveup=0 suspected=0`)。今回の実機で WT の cold に literal は出なかった(下)。
  22:14(JST)の手動確認の `awase.log` に WT の窓の回収の警告があるが、BUG-188 と食い違い、原因を確かめていない(Q3)。
- **TSF compartment(この調査で測った。BUG-200 の窓の種別と別の話)**(GJI、自前の RichEdit 窓、`compartment_notify_probe`): `WM_IME_CONTROL` の close/open にスレッド compartment `KEYBOARD_OPENCLOSE` が 0〜1ms で追随(awase の外からの閉じ方でも TSF 側に伝わる=閉じた事実の独立な確認)。
  大域 compartment(`GetGlobalCompartment`)は取得できるが一度も変化しない(TIP が大域を鏡のように書かないという推測と合うが、GJI の実装は見ていない。**別プロセスの読み出し口が無い証拠にはならない**)。
  別プロセスの窓への `IMC_GETOPENSTATUS`・言語バー・入力インジケーターの UIA は未測定。

## 初版の D1・D2・D3 の取り下げ

- **D1**(「別プロセスの開閉を読める手段は無い」から「閉を観測したら回収しない」が採れない): 窓の種別の取り違えで誤り。Imm32Unavailable × GJI には読み(ADR-205 の prefetch)がある。
- **D2**(give-up の Backspace を止める): IME が実際に ON の cold-start 失敗で、画面に残ったゴミを掃除できなくなる(所有者の指摘)。
- **D3**(連続 literal が N 回続いたら回収を打ち切り、認識を OFF に直す): ADR-227 の r1 で撤回された案と同じ形。(i) 明示意図があると観測は belief を動かさない(`effective_open` が意図を優先)、
  (ii) 否定的証拠(literal の連続)から閉を推論するのは `GjiIoInference` の一方向方針と `.claude/rules/ime-belief-architecture.md` の事故(観測が無いことを根拠に desired を書き換えた)と同型、
  (iii) TsfNative では閉じた IME でも conv に NATIVE ビットが残り、idle の `ConvOpenInference(true)` が追随を打ち消して振動する(BUG-188 の B1)、(iv) ADR-227 の所有者条件(追随は偽陽性がほぼ 0 と示せること)を満たす測定が無い。
  また、「連続 literal が打鍵のたびに続く」の根拠にした `count=2..5` は、**試行をまたいだ**増加(各試行は1打)で、「N 文字だけ失う」の見積もりは ADR-227 D0-3(`kaka` が画面に残る)と合わない。

## 決定(案)

### D0': 追随が何を出したかを測る(最初。実装の前に)

同じ再現(`--form=tsf --mode=drift-on`)で、journal の `JournalEntry::GiveUpFollow { outcome, baseline }` と `[external-change]` を取り、(i) が働かなかった理由を次のどれかに絞る:

1. `outcome` が `no_explicit_intent`(「ON にそろえる」の操作が awase に明示意図を残さない)または `stale_focus`(プローブ開始時と focus 世代が違う)。
2. 追随を arm したが、読み直しの読みが閉を示さなかった(基準値が既に 0 で `Changed` にならない、など。ADR-205 の基準値の扱い)。
3. 追随したが、次の試行の「ON にそろえる」で戻され、また literal になった。
4. `GiveUpFollow` が**1件も出ない**。コードの読みでは、試行1で1件、世代が変わらなければ試行3でもう1件出るはず(実ログは未確認)。0件なら、give-up の証拠を作る側の配線(`take_giveup_evidence`・`GiveUpTracker`)か、focus 世代の扱いの問題で、(i) の前段が働いていない。

取得方法の候補: (a) journal のダンプ(Alt+変換 → Alt+無変換 → Alt+変換。今回の試行では発動せず、トリガーの条件〈タイムアウト・修飾キーの扱い〉は未確認)、(b) 不具合報告(ADR-095)、(c) `typing_stress` が再現の直後に journal を取る口を足す。

### D0' の結果(2026-10-12、dragonflyg4、`typing_stress --form=tsf --mode=drift-on --trials=4`、GJI、awase 開発ビルドを `AWASE_TEST_INJECTION=1` で起動)

`awase.log` の時刻(4試行とも同じ流れ。数字は1試行目 16:44:16〜17 と、4試行の範囲):

1. 明示意図 ON(`[shadow-toggle] ... step="IntentPromoted" ... action="TurnOn"`)のあと、ハーネスが awase を経由せず実 IME を閉じる。
2. NICOLA 単打1回 → `[key-output] KeyInput(batched): romaji="ka"`。約 340ms 後に `[raw-tsf-literal] ... raw TSF literal suspected → backspace ×1 + re-send "ka"`(1回目の回収)。
3. 再送した `ka` も literal。約 330ms 後に `consecutive raw-tsf-literal (count=2) → giving up, backs=1 cleanup only (no re-send)`。
4. **give-up の 61〜82ms 後に `[external-change] ... open=false へ追随` と `Engine deactivated`(ADR-227 (i) が働いた)**。以後の打鍵は素通しになる。
5. 本文は空(4/4)。**最初の1打鍵(2回の送信ぶん)が、2回の Backspace で消える**。追随より前に消える。

**最初の走行(追随が起きなかった。`awase.log` に `[external-change]` なし)との違い**: その awase は `AWASE_TEST_INJECTION=1` なしで起動しており、ハーネスが注入した `VK_IME_ON` は `step="InjectedNotPromoted"` で、
明示意図に昇格しなかった。ADR-227 (i) の条件「明示意図が ON のまま」を満たさず(`no_explicit_intent`)、追随しなかった。**初回の走行の「追随しない」は、ハーネス側の条件(環境変数なし)による**。
(journal のダンプ〈Alt+変換 → Alt+無変換 → Alt+変換を目印付きで注入〉は、この環境でも発動せず、`[mode-key-follow]` の通常の変換/無変換として扱われた。`GiveUpFollow` の outcome を journal から直接読むことはできなかったが、`awase.log` の `[external-change]` の有無で足りた。)

**結論(D0')**: ADR-227 (i) は、GJI × Imm32Unavailable × 明示意図 ON では、実機でも設計どおり働く。BUG-200 の残る害は、**追随が働く前に、最初の1打鍵が回収の Backspace で消える**こと(利用者が「半角英数が消された」と見た現象と合う: 外から IME が閉じられたあとの最初の1語)。
なお、明示意図が無い場合(IME を物理の ON キーでなく言語バー等で ON にした、など)は (i) の対象外で、追随しない(`no_explicit_intent`)ので、literal の回収は続く。この場合の実害は未測定(Q6)。

### 修正方針(案 D5 → D5' を第一候補に。Opus 設計レビュー〈K1〜K3・M1〜M5〉を反映、2026-10-12)

**目的(K2 で訂正)**: 外から IME が閉じられたあとの最初の1打鍵が、追随より前に回収の Backspace で**消える**のを、「**消さない(痕跡を残す)**」に変える。
**直るのは「消えない」まで**で、利用者が打った英数が正しく出るわけではない: 追随で回収を捨てたとき画面に残るのは、エンジン ON で変換済みの NICOLA のローマ字(`ka` など)であり、押した物理キーの英字ではない。
保留中・probe 中に退避した後続の打鍵も、変換済みの romaji として漏れる(D0' の測定は1打鍵なので見えていない)。退避中の打鍵を元の物理キーとして流し直すかは、別の課題(非目標)。

**案 D5(保留)**: 1回目の `SuspectedLiteral` で追随を arm し、`set_raw_literal`(BS + 再送の予約)を追随の読みが終わるまで保留する。保留を捨てるのは「閉への追随」のときだけ。
**案 D5'(arm の前倒し。M1、第一候補)**: 条件(GJI × Imm32Unavailable × 明示意図 ON)を満たす **cold の段の開始時**に監視窓を arm する。追随の読み(60〜80ms)は literal 判定(キーから約 340ms)より前に終わるので、
判定の時点で既に閉へ追随していれば回収を出さない。**保留も新しい timer も `LITERAL_RECOVERY_HOLD_MS` も要らず、cold-start の回収は遅れない**(`complexity-budget.md` の観点でも定数が増えない)。
代わりに arm の回数が cold の段ごとに増える(読みは prefetch で追加の I/O は無い)。`MODE_KEY_PASS_MARK_WINDOW_MS`(300ms)は判定より短いが、閉の読みは最初の再読みで出るので足りる見込み(**未確認**)。段の途中で Engine が止まったときの probe の扱い(`cancel_probe` が走るか)の確認が要る。

**共通の必須事項(実装前に決める)**

- **K1 保留の置き場所と解き方(D5 を採る場合)**: 現行は、drain handler(`handle_wm_drain_output_queue`)が `flush_raw_tsf_literal_recovery` を無条件に流し、その後でフックが溜めた打鍵(`INPUT_DEFER`)をエンジンへ流す。保留は「予約ありの flush を、追随の結果が出るまで先送りする」ことになり、
  (i) **層**: `Output` は belief を読めないので、判断は `state/raw_recovery_plan.rs` の純粋関数(`plan_raw_recovery` を広げる。入力 `{eligible, recorded_at_ms, now_ms, hold_ms, followed_closed_at_ms: Option<u64>}`、出力 `Send / Hold(remaining) / Discard`)にし、
  runtime が「閉へ追随した」事実を `Output` へ**一方向に入れる**(前例: `platform.set_conv_mode_authority` → `output.set_conv_mutation_allowed`)。
  (ii) **解く契機が2つ**: 追随が閉へ揃えた直後(runtime が `WM_DRAIN_OUTPUT_QUEUE` を投げ直す)と、期限(timer か `TIMER_TSF_PROBE` の延長)。無いと、予約が `RAW_TSF_LITERAL` に残り、`raw_recovery_owns_deferred()` が真のままで `pending_deferred` が次の打鍵まで解放されない(BUG-38・ADR-123 型)。`DRAIN_PENDING`/`DRAIN_RERUN_PENDING` との再入に注意。
  (iii) **保留中の打鍵**: `pending_deferred` を通らない出力(素通しの物理キー、`ReinjectKey`、Enter・Space などの確定キー)が、保留中に BS より先に宛先へ届くかは**未確認**。届くと後の BS が別の文字を消す。安全な形は「保留中は `INPUT_DEFER` も流さない(probe の段を延長し `OUTPUT_GATE` を握る)」=保留を probe の FSM の中の段にし、ProbeIo に「閉への追随が起きたか」の事実を渡す。
  (iv) **解放窓口の表**(再発ファミリー「defer/replay キューの解放条件」): `cancel_probe`・`discard_raw_recovery_if_moved`・`TIMER_TSF_GATE` の `replay_later`・drain-before-send(`drain_pending_deferred_before_send_if_queue_only`)のすべてで、保留中の振る舞いを表にする(**実装前に ADR へ書く**)。
  D5'(arm の前倒し)なら、この保留の機構が不要になる。
- **K3 偽陽性の根拠(訂正)**: 初版が引いた ADR-227 の「偽陽性 0/30」は、外部クローズなしの cold で give-up も suspected も 0 件(= 追随が**一度も arm されなかった**)測定で、「arm したときに読みが誤って閉を返す率」ではない。
  D5/D5' が arm するのは、TSF の文脈が準備できていない最中(cold の段、または literal と判定された時点)で、この時点の GJI × Chrome の `IMC_GETOPENSTATUS` が 1 を返し続けるかは**未測定**(ADR-205 の 1→0 の測定は、閉じる操作の前後の読み)。
  誤って閉を読むと、回収を捨てるだけでなく、**Engine を止めて明示意図を削除**し(`follow_external_change_in_scope`)、以後は物理キーが ON の IME へ素通しになって化ける。ADR-227 の所有者条件(偽陽性がほぼ 0 と示せること)を満たす測定が要る。
  **合格条件(数で)**: `typing_stress --form=tsf` の cold の literal を `--suspend-proc=GoogleIMEJaRenderer`(ADR-227 の再現手段)などで N=30 回以上作り(本当に IME は ON)、**arm 後の閉への追随 0 件・`Engine deactivated` 0 件**。この測定を実装より前(または観測だけの段階)に置く。
  **開く方向の副作用**: 基準値は「同じスコープの直近の読み」(`external_change_watch.rs`)。直近の読みが古い `false`(以前 OFF だった頃)で、その後に物理キーで ON にして一度も読んでいない場合、arm すると `Changed(true)` になり、開く方向へ追随して**明示意図が消える**(`intent_store.remove`)。今まで give-up のときだけだったこの副作用が、arm の頻度とともに広がる。D5/D5' は arm 自体の副作用も評価する。
- **M2 `consecutive_count`**: `mark_cold_raw_tsf` は判定の時点で +1 し、0 に戻るのは `FocusChange`・`SetOpenTrue`(awase 自身の `SetOpen(true)` の適用だけ)・`CompositionConfirmed`。利用者が物理の ON キーで IME を戻すと `SetOpenTrue` にならず、次の cold literal が `consecutive != 0` の分岐に入って BS だけで文字を失う(BUG-27 追補4 と同形。現行の (i) の追随の後にも同じ穴があるはずだが**未確認**)。
  追随で回収を捨てたとき、または閉への追随のとき、`reset_consecutive_count` を呼ぶ(`consecutive` のリセット条件に足す)ことを決める。
- **M3 証拠の入口**: `GiveUpTracker::note_record` は `gave_up && suspected >= 2` でしか証拠を返さない。1回目(D5)の arm には、tracker の外に新しい証拠(`FirstLiteralEvidence { cold_seq, focus_gen }`)を足す(tracker の条件を変えると、`giveup_tracker_tests` と `GiveUpFollow` の意味が変わる)。D5'(段の開始時の arm)は、段の開始の入口が別に要る。
  focus 世代は tracker が `note_vk_sent` で覚える最初の送信時の世代と同じ値を使える。2回目の give-up の arm は、同じスコープで生きている窓なら基準値を保って延長する(`ExternalChangeWatch::arm_with`、延長の上限は最初の arm から `window_ms * 2`)ので二重の追随にならない(コードで確認済み)。
- **M4 journal**: 1回目・段開始の arm を `GiveUpFollow` として記録すると、give-up が起きていないのに「give-up 契機」と読めて、報告の集計(`gave_up` で数えるもの)が混ざる。`trigger: first_literal | stage_start | give_up` を足すか、別の variant にする。`RecoveryDiscarded(FollowedClosed)` は新しい variant なので `journal_policy` の notable 判定と replay の扱いを決める。
- **M5 保留の長さ(D5 のみ)**: 候補 80〜150ms は「give-up から追随まで 61〜82ms」に基づき、1回目の判定の時点の分布は別物(測る前提)。D0' は1試行1打鍵で、利用者は1語続けて打つ(`SkipTyping` 中でも prefetch が走るか、読みの間隔が延びないかを測る条件に入れる。未確認)。
  `tuning-constants.md` のとおり、実測の ms と導出を添える。再読みの間隔は 60ms(`MODE_KEY_PASS_REREAD_MS`、モードキー直後の古い読み対策)だが、外部クローズはずっと前なので 0〜20ms にできるかも検討する(その場合、上限は prefetch の読みのタイムアウト 50ms + offload から導出)。

**検討した別案**
- 回収の前に同期で `IMC_GETOPENSTATUS` を1回読む: 別プロセスへの `SendMessage` が最大 300ms ブロックしうる(`run_with_timeout` が要る)うえ、基準値・古い読みの扱いを再実装することになる。
- 推論で belief を変える(D3): 取り下げ済み。
- D5(保留)と D5'(arm の前倒し): 上のとおり。**D5' を第一候補**にする(保留・timer・新しい定数が要らず、cold-start の回収を遅らせない)。ただし K3 の測定の結果次第で、arm の頻度が増える害が大きければ D5(1回目で arm)に戻る。

**段階0 の実測の結果(2026-10-12、dragonflyg4、GJI、awase 開発ビルドを `AWASE_TEST_INJECTION=1` で起動 = 明示意図 ON)— 実験室では K3 を成立させられない**

読みの形は awase の prefetch と同じ(`hwndFocus` → `ImmGetDefaultIMEWnd` → `SendMessageTimeout(WM_IME_CONTROL, IMC_GETOPENSTATUS, 50ms)`)で、別プロセスのサンプラー(10ms 間隔、PowerShell の `Add-Type`)で取った。

1. **`typing_stress --form=tsf --kinds=single,mixed --trials=6 --suspend-proc=GoogleIMEJaRenderer --suspend-ms=2500`**(12試行、IME は ON のまま Renderer を止める): `tsf` 形(自前の RichEdit 窓、IMM のコンテキストあり)の読みは**約8,000回すべて 1**(閉を返さない)。`[external-change]` 0 件。ただし **literal 判定が 0 件**で、cold の literal は起きなかった(arm の時点を迎えていない)。
2. **`text_store_probe --class=Chrome_RenderWidgetHostHWND --deny-sync --seq=16,4B,41`**(TSF の同期ロックを拒否して literal を作る。30回 × 2 走行): literal 判定は 60〜90 件、`[external-change]` は **0 件**。
   **ただし、この窓の読みは常に 0**(クラス名つきサンプラーで 14,689 回中 14,689 回が 0)。窓に IMM のコンテキストが無く、IMM 経由の `IMC_GETOPENSTATUS` は実際の IME の状態を反映しない。追随は基準値からの変化を見るので、読みが最初から 0 で動かないこの窓では起きないだけで、**「偽陽性が 0」の証拠にならない**。
3. 実 Chrome・Windows Terminal の cold(60 秒アイドル)は、この端末では literal 化しない(先の節)。

**結論**: 実験室の手段では、「実 Chrome の cold の最中の `IMC_GETOPENSTATUS` が、IME が ON のとき閉を返さないか」を、literal が起きた状況で測れない(literal が起きる窓は IMM の読みが動かず、IMM の読みが動く窓では literal が起きない)。
K3 の合格条件(arm 後の閉への追随 0 件、literal 30 回以上)は、**実験室では満たせず、満たせない理由が分かった**。よって、段階0 は実験室の測定をやめ、**実利用の環境で観測だけを出荷して集める**形に改める(下)。

**進め方(段階)**
0. **観測だけを出荷する**(挙動は変えない。PR #622、`JournalEntry::FirstLiteralObserve`): 一連の literal 疑いの1回目(tracker の連鎖の最初)で、判定(`first_literal_would_arm | not_applicable | stale_focus | no_explicit_intent`)、literal の前の直近の読み(`last_read`、時刻なし)、
   `consecutive_before`(`consecutive_count` の数え残り)を記録する。`would_arm`(GJI × Imm32Unavailable × 明示意図 ON)のときは、literal の**直後**に prefetch と同じ読み(`read_ime_state_full_async`)を**1回だけ**取り、`read_after` と所要 ms(`read_after_ms`)も記録する
   (belief・監視窓・refresh のパイプラインは通らず、journal にだけ書く。literal の前の読みだけでは、本当に閉じている場面と開いている cold-start の場面を区別できない=Opus レビュー K1)。
   実利用の不具合報告(ADR-095)から、literal が起きたときに `read_after` が `Some(false)`(閉)だった件数を数える。**本当に ON の窓で `Some(false)` が出れば偽陽性**(D5' は採れない)。IME が実際に ON か OFF かは journal からは分からないので、
   `read_after == Some(false)` の件を、同じ報告の `[external-change]` の追随や、利用者の症状(英数が消えた)と突き合わせて読む。
   **時点のずれ**: この読みは literal の判定(キーから約 340ms)の直後で、D5' が arm する cold の段の開始時(キーの直後)の約 60〜80ms 後の読みとは約 300ms ずれる。cold の準備が進んだ後の読みなので、D5' の偽陽性率を過小に見積もる側に偏りうる。
1. 報告の集計で合格したら D5' を実装(純粋関数への切り出し + 単体テスト、M2 のリセット、M4 の journal)。再発ファミリー(IME belief・warmup・defer/replay キューの解放条件)なので、各ファミリーの (a) を満たす。実装の前に Opus レビューを再度回す。
2. 実機の回帰は `typing_stress --form=tsf --mode=drift-on`(対で書く): 外部クローズ(IME OFF)では**本文 = 送った romaji(例 `ka`)で `backspace ×` が出ない**こと、cold-start の対照(真に ON)では従来どおり `backspace ×` と再送が出ること(片側だけだと「常に捨てる」誤実装も通る)。宛先が変わったとき(ADR-246 `DiscardedStale`)と保留の優先順位(宛先照合が先)も表に入れる。

## 参考(結論に使わない): 不具合報告 47 件の集計

R2 の直近 47 件の `journal.json` で `LiteralDetect` の連続 `SuspectedLiteral` を数えた(列 9 本、3 報告。回復した列の最長 3、未回復 3 と 6)。
**この集計は N の根拠にならない**: `journal_policy.rs::literal_detect_is_notable` が、普通の `CompositionConfirmed` を journal に書かない(`suppressed_confirms` に数だけ入る)ので、比率(約 9%)は意味を持たない。
`consecutive_count` は `FocusChange`/`SetOpenTrue` で 0 に戻り、集計の「列」とは定義が違う。標本は実質 3 報告程度で、N=4 で回復列と未回復列を分けられたのは偶然。IME が実際に OFF だったか否かも journal からは分からない。

## 実機の cold の実測(参考)

実 Chrome(`chromepage`)と Windows Terminal(`--form=ext --ext=windows-terminal`)で、60 秒アイドル後の NICOLA 単打 6 文字 × 4 試行は、ゴミ・欠落なし。`awase.log` は `*-per-vk-confirmed`(cold=2〜6 と 7〜11)のみで、literal 判定は 0 件。
この端末では cold-start の literal を再現できないため(ADR-193 と同じ傾向)、連続 literal の長さは実測できない。

## 検証計画

上の「進め方(段階)」に一本化した(段階0 = 観測だけ + K3 の測定、段階1 = D5' の実装 + 純粋関数の単体テスト + Opus 再レビュー、段階2 = 実機の回帰を対で)。副作用の確認として、cold-start の literal 回収(BUG-002 系)が従来どおり働くことを必ず見る。

## 未決(Q)

- Q1(解消): D0' は `awase.log` の `[external-change]` で足りた。journal のダンプ(目印付きの Alt+変換…)が発動しない理由は未確認(重要ではない)。
- Q2(解消): 物理の ON キーで明示意図が付く(`IntentPromoted`)。環境変数なしの awase ではハーネスの注入は昇格しない(`InjectedNotPromoted`)ので、再現には `AWASE_TEST_INJECTION=1` が要る。
- Q3: 22:14(JST)の WT の警告と BUG-188(WT では give-up が起きない)の食い違い。
- Q4: 所有者が自然発生で見た消え方は、本件と同じか。次に起きたら不具合報告(ADR-095)を取る。
- Q6: 明示意図が無い場合(言語バー等で ON にした)の実害。ADR-227 (i) の対象外。
- Q5: TsfNative(WT)での開閉の読み出し口(別プロセスへの `IMC_GETOPENSTATUS`、言語バー、入力インジケーターの UIA)。BUG-200 の窓(Imm32Unavailable)には不要。

## 出典

- [BUG-200](../known-bugs/BUG-200.md)、[BUG-074](../known-bugs/BUG-074.md)、[ADR-227](227-bug074-giveup-as-closed-ime-evidence.md)、[ADR-205](205-observe-external-ime-close-in-imm32-unavailable-windows.md)。
- `crates/awase-windows/src/output/probe_io.rs`、`runtime/ime_refresh.rs::ir_follow_after_literal_giveup`、`crates/awase-windows-core/src/tsf/literal_facts.rs`、`crates/awase-windows/examples/typing_stress/main.rs`(`--mode=drift-on`)、`examples/compartment_notify_probe.rs`。
