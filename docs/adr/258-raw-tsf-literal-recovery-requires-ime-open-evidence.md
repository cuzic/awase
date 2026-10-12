---
id: ADR-258
title: |-
  BUG-200: BUG-074(ADR-227 (i))の追随が実機で効かない — 先に原因を測る
summary: |-
  BUG-200: 外部から実 IME が閉じ belief が ON のままのとき、Imm32Unavailable × GJI の窓で打った文字が raw TSF リテラル回収の Backspace で消える。実機(dragonflyg4)で人工再現(`--form=tsf --mode=drift-on`、本文が空 4/4)。
  これは BUG-074 / ADR-227 と同じ構成で、ADR-227 (i)(give-up を読み直しのきっかけにして追随する、PR #480、CI 検証済み・実機未検証)が実機で働いていない証拠かもしれない(`[external-change]`・`Engine deactivated` が出ない)。
  初版の D1(別プロセスの開閉を読む手段は無い)・D2・D3 は、窓の種別の取り違えと ADR-227 r1 の撤回理由の見落としで取り下げた。先に、追随(`GiveUpFollow`)が何を出したかを journal で測る(D0')。
status: |-
  起草(2026-10-12)。Opus レビュー(重大5件)を受けて改訂: 初版の D1・D2・D3 を取り下げ、D0'(`GiveUpFollow` の outcome を取る)に置き換えた。実装なし。journal の取得は未達(ダンプのトリガーが発動しなかった)。
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

- **再現**(各 n=4、2回走行): `typing_stress --form=tsf --mode=drift-on`。IME を ON にそろえ(明示意図 ON が付く)、ハーネスが awase を経由せず実 IME を閉じる(`WM_IME_CONTROL`)。
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

取得方法の候補: (a) journal のダンプ(Alt+変換 → Alt+無変換 → Alt+変換。今回の試行では発動せず、トリガーの条件〈タイムアウト・修飾キーの扱い〉は未確認)、(b) 不具合報告(ADR-095)、(c) `typing_stress` が再現の直後に journal を取る口を足す。

### 修正方針(D0' の結果で決める。いまは決めない)

- 1 なら、再現側の問題(本物の ON キーで ON にしてから閉じる、など)で、(i) は実利用では働いている可能性がある。BUG-074 の実機未検証を実機で埋める手順になる。
- 2 または 3 なら、(i) の穴を直す(例: 初回の回収の前にも prefetch の読みを使う=ADR-227 (i) を初回まで広げる。基準値の扱いの見直し)。これは ADR-227 の改訂として、`ime-belief-architecture.md` の規約に沿って行う。
- D3 のような「推論で belief を変える」案は採らない。

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

- Q1: D0' の取得方法(上の (a)〜(c))。ダンプのトリガーが今回発動しなかった理由。
- Q2: 再現の「ON にそろえる」が実利用の ON キーと同じ明示意図を残すか。
- Q3: 22:14(JST)の WT の警告と BUG-188(WT では give-up が起きない)の食い違い。
- Q4: 所有者が自然発生で見た消え方は、本件と同じか。次に起きたら不具合報告(ADR-095)を取る。
- Q5: TsfNative(WT)での開閉の読み出し口(別プロセスへの `IMC_GETOPENSTATUS`、言語バー、入力インジケーターの UIA)。BUG-200 の窓(Imm32Unavailable)には不要。

## 出典

- [BUG-200](../known-bugs/BUG-200.md)、[BUG-074](../known-bugs/BUG-074.md)、[ADR-227](227-bug074-giveup-as-closed-ime-evidence.md)、[ADR-205](205-observe-external-ime-close-in-imm32-unavailable-windows.md)。
- `crates/awase-windows/src/output/probe_io.rs`、`runtime/ime_refresh.rs::ir_follow_after_literal_giveup`、`crates/awase-windows-core/src/tsf/literal_facts.rs`、`crates/awase-windows/examples/typing_stress/main.rs`(`--mode=drift-on`)、`examples/compartment_notify_probe.rs`。
