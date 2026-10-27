---
id: ADR-283
title: |-
  タスクバーの入力インジケーターの字形を、採用した belief(候補 Z、ADR-282)の信頼表の証拠に加える
summary: |-
  ADR-282 で Engine の入力の belief を候補 Z(IME の保持単位〈スコープ〉ごとに最後に受け入れた主張をラッチする)にした(既定、2026-10-27)。
  Z の信頼表は「読めない窓の IMM 系の観測は反証に使わない」だけで、読めない窓(imm32-unavailable・tsf-native・学習で降格した窓)には、
  明示操作・打鍵の予測・O2 の既定(ON)・U2 の事前(新しいスレッドは閉)以外の証拠が無い。外から IME が閉じられると、Z は ON のまま残る(I2 の残り)。
  字形(ADR-261・ADR-273、観測のみで belief に入れていない)は、実機 dragonflyg4 の集計で、読める窓 30/30・tsf-native 72/77 が belief と一致し、
  imm32-unavailable では 126 件中 68 件が「字形 閉・belief 開」、「字形 開・belief 閉」は全 profile で 0 件だった。
  本 ADR は、字形を Z の観測として、読めない窓に限って入れる設計を決める: 字形の読みの証拠の時刻を取り込みから最大の遅れ分さかのぼらせ、
  Z の「新しいほうが勝つ」と明示操作の柵にそのまま載せる(既存の D4 の柵と同じ効果)。開閉と入力モードの両方の軸に入れる(英数の字形を開だけで入れると Engine を誤って ON にする)。
  段階は G1(核とオラクル)→ G2(観測の journal の試算を Z へ付け替え)→ G3(既存の設定 `[indicator_evidence] glyph = "unobserved"` で採用)。
  旧の belief の字形の分岐(ADR-273 段階 1、本番で一度も有効にしていない)は G3 で削除する(複雑性予算の相手)。
status: |-
  起草(2026-10-27)。実装なし。Opus レビュー待ち。
related_adr:
  - "ADR-261"
  - "ADR-271"
  - "ADR-273"
  - "ADR-282"
  - "ADR-233"
  - "ADR-212"
---

# ADR-283: 字形を Z の信頼表の証拠に加える

調べた版: `origin/develop` `29e23c2d3`(ADR-282 V2-3 #893 `58517793c` と docs #894 のマージ後)。cargo はローカルで実行していない。
実機の数値は team-lead の報告の転記(起草者はログを読んでいない)。

## 1. 背景

### 1.1 今の Z の信頼表(ADR-282 §4.4、`crates/awase-ime-core/src/state/scope_belief.rs`)

- 観測は `scope_event_of` で Z の事象へ写す。`is_imm_family`(IMM の読み・conv のビット・GJI の I/O からの推測)の源は、読めない窓
  (`WindowFacts::imm_trusted` が偽)では捨てる。それ以外の源(`Tsf`・`MozcShm`・`TaskbarGlyph`)は窓の種類に関わらず使う。
- 受け入れ(`accepts`): Low は主張なし・既定・実測の事前だけを覆す。Medium 以上は明示操作の柵(`KEY_EFFECT_SETTLE_MS` 170ms)より後に
  読み始めたものだけが覆し、観測どうしは新しいほうが勝つ(古い High は新しい Medium に勝たない、ADR-233 A')。
- 観測の時刻は、`ImeEvent` が読み取りの開始時刻を持たないので、事象を受けた時刻で代えている(V1 の近似)。
- **注意(既存の穴)**: `TaskbarGlyph` は `is_imm_family` に入っていないので、仮に今 `ObserverReported(TaskbarGlyph)` が dispatch されると、
  Z は読める窓でも、事象を受けた時刻(取り込みから約 0.9〜1.3 秒遅い)で受け入れる。今は字形を報告する経路が本番で動かない
  (`EvidenceSource::Glyph.ui_max_level()` が `Observe`、`report_indicator_glyph` の呼び出しは `ui_max_level` を上げないと走らない)ので実害は無い。
  本 ADR の G1 でこの穴を閉じる(字形専用の写像)。

### 1.2 字形の今(ADR-261・ADR-271・ADR-273)

- 取り込み・照合・柵は実装済み。照合は粗いラベル(`BeliefLabel`: 閉・かな系・英数系)で、見本は読める窓で受け身に集める(ADR-261)。
- 柵(`fence_glyph`): 取り込みの間にフォーカスが動いた・前面と追跡がずれた・最後のモード系の打鍵から `GLYPH_KEY_FENCE_MS`(1300ms、
  実測の最大の遅れ 0.885 秒 + 余裕)経っていない字形は通さない。
- 旧の belief への配線(ADR-273): `resolve_open_at` の独立した分岐(`derive_any` の後・`most_recent_trusted` の前)、Medium・BeliefOnly、
  窓の範囲(`glyph_window_in_scope`): `unobserved` は学習で降格した窓(NULL 型)だけ、静的に `Imm32Unavailable`・tsf-native・入力中継・
  mozc の共有メモリがある窓は除外。**本番では一度も有効にしていない**(`ui_max_level` が `Observe`)。
- 観測のみの journal(`preview_indicator_glyph`): `would_report`・`would_scope`・`would_decide`・`would_flip` を残す。試算は旧の belief に対して行う。

### 1.3 実機の集計(dragonflyg4、`awase.log*` 10 ファイル、2026-10-20〜27、重複あり・上限の目安。team-lead の報告)

| profile | 字形と belief の一致 | 不一致の向き |
|---|---|---|
| standard(読める窓) | 30/30 | — |
| tsf-native | 72/77 | 5 件(向きは未報告) |
| imm32-unavailable(観測の無い窓) | 58/126 | 68 件すべて「字形 閉・belief 開」 |

- 「字形 開・belief 閉」は全 profile で 0 件。見本に無い字形(Miss)は imm32-unavailable で 28 件、`miss-shape` の行は 2 件。
- 字形が `effective-open-flip` の決め手になった行は 0 件(配線していないので当然)。`would_decide` は journal のメモリ上のリングにしか無く、集計できていない。
- 前面窓の変化から取り込むまでの待ち(`delay_ms`)は約 900ms で固定。
- この期間の belief は大半が旧の belief(Z の既定化は 2026-10-27)。

### 1.4 所有者の方針と既存の決定

- belief と実 IME のずれが直らない窓では、ユーザーがやりたいことを belief より優先する(2026-10-23)。
- 字形は複数の証拠の 1 つ。取れたら使い、取れなければ諦める。誤判定が無いこと(Miss に倒れること)の確認が主目的(ADR-261)。
- ADR-271 の入口条件: Medium が High を追い越さない、「観測の無い窓」の述語を統一する。
- ADR-282: U3(InputRelay は O2 の ON 寄りに入れない)・D8(書き込みの根拠に入れない)・O6(未確定の組は独立)・I1〜I7。

## 2. 決定(案)

### D1. 字形の重み(信頼表のセル)

| 窓(Z に渡す `WindowFacts`) | 字形を使うか | 理由 |
|---|---|---|
| 読める窓(`imm_trusted`) | **使わない**(既定)。`all` でも使わない(D6) | 直接の読み(High)がある。30/30 一致で、入れても便益が無い。遅れ(約 0.9 秒)で新しい読みと競う危険だけが増える |
| imm32-unavailable(静的、Chrome 等) | **使う**(`unobserved`) | Z に外部の証拠が無い。実機で「字形 開・belief 閉」0 件、不一致はすべて Z(旧)が ON 寄りに倒れた側 |
| 学習で降格した窓(NULL 型) | **使う**(`unobserved`) | ADR-273 D3 のまま |
| tsf-native | **G2 の集計の後に決める**(U-G1) | 72/77 一致だが、不一致 5 件の向きが未報告。awase が書いた値を予測で持つ窓で、字形の遅れが予測と競う |
| 入力中継(InputRelay) | 使わない | U3。IME の状態の所有者が別 |
| mozc の共有メモリがある窓 | 使わない | `MozcShm`(High)がある(ADR-273 R1) |

- **述語を 1 つにする**(ADR-271 の入口条件): 「字形を使う窓」= `!imm_trusted && !input_relay && !mozc_block`(tsf-native は U-G1 の結果で含めるか決める)。
  `imm_trusted` は Z が既に持つ事実(`can_use_imm32_cross_process() && !is_demoted_by_learning()`、`focus_tracking.rs`)。旧の
  `glyph_window_in_scope`(降格した窓だけ)は、静的な `Imm32Unavailable` を含めるように変わる(ADR-273 D3 の改訂)。
- **確度は Medium**(旧の配線と同じ。`Observed<TaskbarGlyph>` の witness は変えない)。

### D2. 証拠の時刻を取り込みからさかのぼらせる(遅れの扱い)

字形は「取り込んだ時点より最大 `GLYPH_KEY_FENCE_MS` 前の IME の状態」を映しうる(実測の最大の遅れ 0.885 秒)。Z へ写すとき、
**読み取りの開始時刻 = 報告の時刻 − `GLYPH_KEY_FENCE_MS`** とする。これで Z の既存の規則だけで次が成り立つ:

- 明示操作・表のキーの予測(時刻 `t`、柵 `t + 170`)を、`t + 170 + 1300` より前に報告された字形は覆さない(旧の D4 の柵
  「最後のモード系の打鍵から 1300ms」と同じ効果。旧の柵は打鍵の時刻、Z は Z 自身の主張の時刻で測るので、他のスコープの打鍵では止まらない)。
- 字形より新しい観測(読めない窓では `Tsf`・`MozcShm` など信頼する源)が字形に勝つ。古い観測には字形が勝つ(ADR-233 A' と同じ)。
  「Medium が High を追い越さない」(ADR-271)は、D1 で High のある窓(読める窓・mozc)を除くことと、時刻のさかのぼりで、
  **字形より後に読み始めた High には必ず負ける**という形で守る。字形より古い High に勝つのは Z の既存の規則(新しい Medium が勝つ)で、
  旧の配線(新鮮な Medium 以上が無いときだけ採る)より強い。U-G2 で所有者に確かめる。
- 既存の柵(`fence_glyph` の窓の同一性・前面の一致)は残す。モード系の打鍵の柵(`TooSoonAfterModeKey`)は、Z の時刻のさかのぼりと重なるが、
  旧の belief の打鍵(他のスコープを含む)で止める点が違う。G1 では残し(二重でも安全側)、G3 で要否を決める。

### D3. 軸: 開閉と入力モードの両方

`BeliefLabel` から Z の事象へ: `Closed` → 開閉=閉、`Kana` → 開閉=開・モード=かな、`Eisu` → 開閉=開・モード=英数。
**英数の字形を開閉だけで入れると、Z の入力モードの主張が無い窓では O2 の既定(かな)と組んで Engine が ON になる**(I2 の新しい違反)。
旧の ADR-273 が開閉だけにしたのは、旧の belief の入力モードの経路が別だったから。Z では 2 軸とも同じ規則で受け入れる。
GJI の「半角英数」(IME は開)と MS-IME の「半角英数」(IME は閉)は字形が違い、どちらも Engine は OFF(O1)なので、誤って取り違えても I1・I2 には効かない。

### D4. 取り消し(Miss)は Z の主張を消さない

字形が表に無い状態に変わった(`TooFar`・`Ambiguous`)とき、旧の配線は字形の枠を消した(`IndicatorGlyphMissed`)。Z は寿命で後退しない
(I6)ので、**Miss は何もしない**(前の字形の主張は、次の証拠が来るまで残る)。「取れなければ諦める」(ADR-261)はこの意味で守る。
Miss が「状態が変わった」ことを示す場合(見本の無いモードに切り替わった)に古い主張が残る危険は、U-G3 で集計して判断する。

### D5. 書き込み・授権には入れない(D8)

字形は Z の belief(Engine の入力)にだけ入る。drift correction・授権・`read_back`・`evaluate_drift` には入れない(`BeliefOnly` のまま。
旧の除外リストの扱いは変えない)。

### D6. 設定と戻し方

- 既存の `[indicator_evidence] glyph`(ADR-271、`off`/`observe`/`unobserved`/`all`)をそのまま使う。新しい設定は足さない。
  - `observe`: 取り込み・照合・journal だけ(今と同じ)。
  - `unobserved`: D1 の窓で Z に入れる(G3 で `ui_max_level` を `Unobserved` に上げる)。
  - `all`: 本 ADR では `unobserved` と同じ扱い(読める窓には入れない。ADR-271 D2b の「便益を journal で示せたときだけ」に従い、示せていない)。
- 戻し方: `glyph = "observe"`(または `"off"`)。既定は今の `off` のまま(オプトイン、ADR-271)。
- `belief_v2 = false`(旧の belief)のときは、字形は旧の belief に入れない(旧の分岐は G3 で削除する、§4)。

## 3. 検証

### 3.1 オラクル(I1〜I7、G1)

`crates/awase-windows/tests/belief_engine_invariant.rs` の字母に字形の事象を足す:

- `Obs::Glyph`: 擬似 IME の真の状態の**遅れた写し**(`GLYPH_LAG_MS` 前の真。ハーネスが真の履歴を持つ)を `report_indicator_glyph` と同じ口
  (`Observed<TaskbarGlyph>` の witness を作る試験用の口。`restored_from_journal` と同じ扱いで、本番では使わない)で報告する。
- `Obs::GlyphStale`(嘘の一種): 直前の窓の真を写す(前面の取り違え。柵を抜けた場合の最悪)。
- 窓モデルの読めない窓(chrome・wezterm)でだけ報告する(D1)。
- 測るもの: 採用した Z(既定)と「Z + 字形」の I1・I2(shared・per-thread・混ぜた窓モデル)、I3・I4・I6(0 のままか)、
  §1.1.1・BUG-223 の列。**合格条件**: どの設定でも I1・I2 が Z 以下、I3・I4・I6 が 0、`GlyphStale` を含む列の増分を分けて数える。
- 追加の型: 外部から閉じられた読めない窓(今の Z の I2 の残り)を字形が直すか。

### 3.2 実 IME との照合

- CI(ADR-261 の測定の写し): 観測の無い窓の近似(自前の TSF text store の窓、IMM コンテキストなし)で、字形と実 IME の状態の一致は 6 構成すべてで確認済み。
  本 ADR では新しい CI 測定を足さない(U-G4 で、必要なら imm32-unavailable の実 Chrome の構成を足す)。
- 実機: §1.3 の 68 件の読み(下の §3.3)を、G2 の journal で確かめる。

### 3.3 「字形 閉・belief 開」68 件の読み

- 字形が正しいと考える根拠: (i) 逆向き(字形 開・belief 閉)が 0 件で、字形が一方向にだけ外れる誤り方は取り込みの不具合として不自然。
  (ii) 旧の belief は観測の無い窓で `desired-fallback`・既定の ON に倒れる(BUG-223 の型、ADR-282 §1)。(iii) U2(新しいスレッドは閉で始まる)の実測と整合する。
- 字形が誤りうる経路と、それぞれの柵:
  - 取り込みの遅れ(約 900ms): 窓切替の直後の字形が前の窓を映す → 窓の同一性・前面の一致(`fence_glyph`)と D2 のさかのぼり。
  - 前面の取り違え(通知・ポップアップ・Chrome の非テキスト部): 前面の一致の柵。Chrome の非テキスト部は `FocusKind::NonText` で Engine が OFF(U1)なので I1・I2 に効かない。
  - IME ごとの字形の違い(GJI・MS-IME・Mozc): 見本は環境ごとに集める(内蔵表なし)。Mozc 系は共有メモリがあれば除外(D1)。
- 実機の追加データ(G2 の後、deploy-g4 に頼む項目。状態の語と件数だけ): 下の §5。

### 3.4 shadow の試算(G2)

`preview_indicator_glyph` の `would_decide`・`would_flip` を、旧の belief ではなく **Z の写しに字形を入れたら Z の読み出しが変わるか**で計算する
(Z は小さい表なので、`ScopeBelief` を複製して `evolve` するだけ)。journal に `would_decide_z`・`would_flip_z` と、変わる向き
(閉→開・開→閉)を残す。

## 4. コストと複雑性予算

- **入れない場合の損失**: 読めない窓で外から IME が閉じられたとき(別アプリで閉じた・IME のショートカット・既定の ON の外れ)、Z は
  次の明示操作か表のキーまで ON のまま(I2)。旧の集計の 68 件はこの型の上限の目安。
- **入れるコスト(見当)**:
  - 核: 字形専用の写像(`scope_event_of` の `TaskbarGlyph` の分岐: 時刻のさかのぼり・2 軸・窓の述語)と `WindowFacts` の事実 1 つ
    (`glyph_ok` または述語の入力)。数十行 + 単体テスト。
  - 殻: 新しい経路は作らない。既存の `report_indicator_glyph` の呼び出し(`indicator_glyph_shadow.rs`)が、`unobserved` のとき既に走る形
    (ADR-273 段階 2a)。試算の付け替え(G2)で数十行。
  - オラクル: 字母 2 つ・真の履歴(百行前後)。golden の更新。
- **複雑性予算の相手(1-in-1-out)**: 旧の belief の字形の分岐を削除する(G3)。対象: `ImeModel::resolve_open_at` の `IndicatorGlyph` の分岐、
  `ObservationStore` の `taskbar_glyph` の枠と `retract_indicator`、`ImeEvent::IndicatorGlyphMissed`(核の reducer・journal・architecture_guard の固定)、
  `would_glyph_decide` の旧の試算、`belief_is_decided_by_glyph`(R2)。本番で一度も有効にしていない経路なので、削除で挙動は変わらない。
  `ObservationSource::TaskbarGlyph` と witness は残す(Z の入力)。

## 5. 段階と戻し方

| 段階 | 中身 | 挙動 | 合格・次へ進む条件 |
|---|---|---|---|
| G1 | 核の写像(D1〜D4)・オラクルの字母・golden | 変わらない(`ui_max_level` は `Observe` のまま) | §3.1 の合格条件 |
| G2 | 試算を Z へ付け替え(§3.4)・journal の項目 | 変わらない | 実機の journal(下)を集計し、所有者が判断 |
| G3 | `ui_max_level` を `Unobserved` に上げる・旧の分岐の削除 | `glyph = "unobserved"` の利用者だけ変わる(既定 `off`) | CI と所有者の判断 |

G2 の後に実機で集める項目(状態の語と件数だけ。打鍵の内容は不要):
1. `would_decide_z` が真の件数と、そのうち `would_flip_z` の向き(閉→開・開→閉)の件数、profile ごと。
2. `would_flip_z` が真のとき、その後 10 秒以内に利用者が IME のモードキー(ON/OFF・ひらがな・英数)を押した件数(字形が正しければ、利用者が直す操作は減るはず)。
3. tsf-native の不一致の向きの件数(U-G1)。
4. Miss(`TooFar`・`Ambiguous`)の直前の字形のラベルの件数(U-G3)。

戻し方: 設定 `glyph = "observe"`。コードの戻しは G3 の revert(`experiment-logging.md` に従い、失敗の条件を本文に書く)。

## 6. 未決(所有者に確かめる)

| # | 問い | 提案 |
|---|---|---|
| U-G1 | tsf-native の窓に字形を入れるか | G2 の集計(不一致の向き)の後に決める。それまでは入れない |
| U-G2 | 字形より古い High(読めない窓の `Tsf`・`MozcShm`)に字形が勝つ(Z の既存の規則)のでよいか | よい(ADR-233 A' と同じ。D1 で High のある窓は除くので、実際に起きるのは High の源が途切れた後だけ) |
| U-G3 | Miss で前の字形の主張を消さない(D4)でよいか | よい。G2 の集計で「Miss の前の字形」が開に偏るなら見直す |
| U-G4 | 新しい CI 測定(実 Chrome の imm32-unavailable で字形と実状態)を足すか | 足さない(ADR-261 の 6 構成で足りる)。G2 の実機の集計で代える |
| U-G5 | `all` を読める窓に広げるか | 広げない(便益が示せていない、ADR-271 D2b) |

## 7. 却下した案

- **旧の belief の配線(ADR-273)をそのまま有効にする**: Engine の入力は Z になった(ADR-282 V2-3)ので、旧の belief に入れても Engine には効かない。
- **字形を Low にする**: Z の Low は既定・実測の事前しか覆さないので、Z が明示操作・予測で ON を主張している窓(68 件の型の一部)を直せない。
- **字形の時刻をさかのぼらせず、旧の打鍵の柵だけで守る**: 柵は旧の belief の最後の打鍵で測るので、Z のスコープの主張との前後が保証されない。
