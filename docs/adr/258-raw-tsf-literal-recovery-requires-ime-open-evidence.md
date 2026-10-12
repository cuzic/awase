---
id: ADR-258
title: |-
  BUG-200: BUG-074(ADR-227 (i))の追随が実機で効かない — 先に原因を測る
summary: |-
  BUG-200: 外部から実 IME が閉じ belief が ON のままのとき、Imm32Unavailable × GJI の窓で打った文字が raw TSF リテラル回収の Backspace で消える。実機(dragonflyg4)で人工再現(`--form=tsf --mode=drift-on`、本文が空 4/4)。
  これは BUG-074 / ADR-227 と同じ構成で、ADR-227 (i)(give-up を読み直しのきっかけにして追随する、PR #480、CI 検証済み・実機未検証)が実機で働いていない証拠かもしれない(`[external-change]`・`Engine deactivated` が出ない)。
  初版の D1・D2・D3 は取り下げた。D0' で、追随(ADR-227 (i))は GJI × Imm32Unavailable × 明示意図 ON では実機でも働くと分かった(最初の再現で追随しなかったのは、ハーネスの注入が明示意図に昇格しなかったため)。残る害は、追随の前に最初の1打鍵が回収の Backspace で消えること。第一候補は ADR-227 (i) を初回の回収まで広げること。
status: |-
  起草(2026-10-12)。Opus レビュー(重大5件)を受けて改訂し、D0' を実測した(同日): 追随は設計どおり働く。残る害は最初の1打鍵の消失。修正方針は案 D5(初回の literal 判定でも追随を arm し、回収を追随の読みが終わるまで短く保留する)。実装なし(D5 の Opus レビュー待ち)。
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

### 修正方針(案 D5): 初回の literal 判定でも追随を arm し、回収を追随の読みが終わるまで短く保留する

**目的**: 外から IME が閉じられたあとの最初の1打鍵が、追随より前に回収の Backspace で消えるのを防ぐ(D0' の結果。追随自体は give-up の 61〜82ms 後に完了する)。

**案**: 既存の追随(ADR-227 (i))を、2回目(give-up)ではなく**1回目の `SuspectedLiteral`(`consecutive == 0` の回収)から**使う。新しい読みの仕組みは足さない。

1. **条件**(ADR-227 (i) と同じ): GJI × Imm32Unavailable(`external_change_watch_applies`)、プローブ開始時と同じ focus 世代、明示意図 ON(`giveup_follow_decision` が `Arm`)。条件を満たさなければ従来どおり(回収をそのまま予約)。
2. **arm**: 1回目の `SuspectedLiteral` の verdict で `ir_follow_after_literal_giveup` と同じ手順(`arm_external_change_watch` → 基準値を控える → `schedule_ime_refresh(MODE_KEY_PASS_REREAD_MS)`)を行い、journal に `GiveUpFollow`(`cold_seq`・outcome・基準値)を残す(既存の記録をそのまま使う)。
3. **保留**: `io.set_raw_literal`(Backspace + 再送の予約)を**即時に確定せず**、保留として持つ。保留の期限は追随の読みが終わるまでの上限 `LITERAL_RECOVERY_HOLD_MS`(実測で決める。D0' の追随は give-up から 61〜82ms、再読みの間隔は `MODE_KEY_PASS_REREAD_MS` = 60ms なので、候補は 80〜150ms。`tuning.rs` の規約どおり実測値と導出を添える。**未測定**)。
4. **解決**: 保留の期限までに追随が完了して認識が OFF になった(`Engine deactivated` と同じ効き方。`desired_open`/`effective_open` が閉)なら、保留を**捨てる**(Backspace も再送もしない。literal の文字は画面に残る)。journal に `RecoveryDiscarded(FollowedClosed)` を残す。
   期限が来ても追随が起きなければ、保留を従来どおり確定する(Backspace + 再送)。追随が起きなかった=読みが開(または読めない)なので、cold-start の回収として扱う。
5. **既存との整合**: 回収の宛先照合(ADR-246、`flush_raw_tsf_literal_recovery` の前景窓・世代の照合)は、保留の確定時にも通す。2回目の give-up の追随(従来の (i))は変えない(保留が捨てられなければ、従来の流れ)。

**検討した別案と、採らなかった理由**
- 回収の前に同期で `IMC_GETOPENSTATUS` を1回読む: 別プロセスへの `SendMessage` が最大 300ms ブロックしうる(`run_with_timeout` が要る)うえ、(i) の基準値・古い読みの扱い(最初の再読みは GJI の処理前の古い状態を読むことがある、tuning.rs の `MODE_KEY_PASS_REREAD_MS` の導出)を再実装することになる。既存の追随に乗るほうが、規約(`ime-belief-architecture.md`: 観測 → classify → reduce、belief は `follow_external_change` だけが動かす)に収まる。
- 推論で belief を変える(D3): 取り下げ済み(上)。

**リスクと未決(レビューで見てほしい点)**
- **cold-start の回収への遅延**: 条件(GJI × Imm32Unavailable × 明示意図 ON)を満たす cold-start の literal 回収が、`LITERAL_RECOVERY_HOLD_MS` だけ遅れる。BUG-002 系の「待ちが足りない」を悪化させないか(回収の再送のタイミングは cold の準備待ちと絡む)。
- **保留中の打鍵**: probe 中の打鍵は退避キュー(`OUTPUT_GATE`・`pending_deferred`、ADR-156)に入る。追随で Engine が止まったとき、退避中の打鍵はどう流れるか(literal の romaji が漏れる可能性。`flush_stale_deferred_vks_after_recovery` などの解放側との整合)。
- **世代・カウンタ**: `GiveUpTracker`(`suspected` を2回目で数える)、`consecutive_count`(`FocusChange`/`SetOpenTrue` で 0 に戻る)との相互作用。1回目で arm した追随が、2回目の give-up の追随と二重にならないか(監視窓の再 arm の冪等性)。
- **誤判定**: 本当は ON で cold-start の最初の literal のとき、追随が(基準値の扱いで)誤って閉を読むと、回収を捨てて文字が literal のまま残り、認識も OFF になる。ADR-205 の柵(基準値・focus 世代・明示意図)が守ってくれるか。D0 で偽陽性 0/30(ADR-227)は 2回目の give-up を契機にした読みで、1回目での偽陽性率は未測定。
- **実機で測るもの**: (1) 追随が1回目から arm されたときの、arm から `Engine deactivated` までの ms の分布(`LITERAL_RECOVERY_HOLD_MS` の根拠)。(2) 同じ構成で cold-start が本当に ON の窓(実 Chrome の cold)で、保留が捨てられないこと(偽陽性)。この端末では実 Chrome・WT の cold が literal 化しない(上)ので、(2) は `typing_stress --form=tsf` の cold 系(`--cold`・`--idle`)や、`--suspend-proc=GoogleIMEJaRenderer`(ADR-227 の再現手段)で作る。

**テスト(再発ファミリー: IME belief・warmup。`fix-requires-evidence.md` の (a))**: `giveup_follow_decision` と同様に純粋関数へ切り出す(入力: `consecutive`・条件・追随の結果・保留の経過 ms → 出力: 保留を確定/捨てる/待つ)。単体テストで表にする。実機の回帰は `typing_stress --form=tsf --mode=drift-on`(本文が空にならず、`awase.log` に `backspace ×` が出ないこと)。

## 参考(結論に使わない): 不具合報告 47 件の集計

R2 の直近 47 件の `journal.json` で `LiteralDetect` の連続 `SuspectedLiteral` を数えた(列 9 本、3 報告。回復した列の最長 3、未回復 3 と 6)。
**この集計は N の根拠にならない**: `journal_policy.rs::literal_detect_is_notable` が、普通の `CompositionConfirmed` を journal に書かない(`suppressed_confirms` に数だけ入る)ので、比率(約 9%)は意味を持たない。
`consecutive_count` は `FocusChange`/`SetOpenTrue` で 0 に戻り、集計の「列」とは定義が違う。標本は実質 3 報告程度で、N=4 で回復列と未回復列を分けられたのは偶然。IME が実際に OFF だったか否かも journal からは分からない。

## 実機の cold の実測(参考)

実 Chrome(`chromepage`)と Windows Terminal(`--form=ext --ext=windows-terminal`)で、60 秒アイドル後の NICOLA 単打 6 文字 × 4 試行は、ゴミ・欠落なし。`awase.log` は `*-per-vk-confirmed`(cold=2〜6 と 7〜11)のみで、literal 判定は 0 件。
この端末では cold-start の literal を再現できないため(ADR-193 と同じ傾向)、連続 literal の長さは実測できない。

## 検証計画

1. D0'(上)。結果を本 ADR に追記して修正方針を決める。
2. 回帰の対照: `typing_stress --form=tsf --mode=drift-on` が、修正後に「本文が空でない(英字が残る)か、追随して Engine が止まる」こと。現状(本文が空、4/4)が対照。CI への構成追加は、D0' で再現の作り方が決まってから。
3. 純粋関数の単体テスト(`giveup_follow_decision` など、修正に応じて)。再発ファミリー(IME belief・warmup)なので `fix-requires-evidence.md` の (a) を満たす。
4. 副作用の確認: cold-start の literal 回収(BUG-002 系)が従来どおり働く。

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
