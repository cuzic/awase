---
id: ADR-283
title: |-
  タスクバーの入力インジケーターの字形を、採用した belief(候補 Z、ADR-282)の信頼表の証拠に加える
summary: |-
  ADR-282 で Engine の入力の belief を候補 Z(IME の保持単位〈スコープ〉ごとに最後に受け入れた主張をラッチする)にした(既定、2026-10-27)。
  Z の信頼表は「読めない窓の IMM 系の観測は反証に使わない」だけで、読めない窓(imm32-unavailable・tsf-native・学習で降格した窓)には、
  明示操作・打鍵の予測・O2 の既定(ON)・U2 の事前(新しいスレッドは閉)以外の証拠が無い。
  字形(ADR-261・ADR-273、観測のみで belief に入れていない)の実機の集計は、読める窓 30/30・tsf-native 72/77 が belief と一致、
  imm32-unavailable では 126 件中 68 件が「字形 閉・belief 開」だった。ただし旧の belief は観測の無い窓でほぼ常に開に倒れるので、この非対称は字形の正しさの証拠にならない(§3.3)。
  本 ADR は、字形を Z の観測として入れる設計を決める。取り込みは前面窓の変化の時だけ(900ms・2500ms 後の 2 回)なので、便益は「窓に入った時の状態の訂正」に限る。
  字形の証拠の時刻を `GLYPH_KEY_FENCE_MS` さかのぼらせて Z の規則に載せ(D2)、非テキストの欄の字形は入れない(D7、Z はラッチするので誤った閉が残る)。
  写像は、閉 → 閉、かな → 開・かな、英数 → 入力モードだけ英数(開閉は変えない、D3)。
  初期の対象は ADR-273 と同じ学習で降格した窓(NULL 型)だけで、静的な imm32-unavailable と tsf-native は G2 の集計の後に所有者が決める(D1)。
  段階は G1(核とオラクル)→ G2(観測の journal の試算を Z へ付け替え、内訳つき)→ G3(既存の設定 `[indicator_evidence] glyph = "unobserved"` で採用)。
  旧の belief の字形の分岐(本番で一度も有効にしていない)は G3 で削除する(複雑性予算の相手)。
status: |-
  起草(2026-10-27)。実装なし。Opus レビュー round 1(B1・M4・m6)を反映。
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
実機の数値は team-lead の報告の転記(起草者はログを読んでいない)。レビューの指摘への対応は §8。

## 1. 背景

### 1.1 今の Z の信頼表(ADR-282 §4.4、`crates/awase-ime-core/src/state/scope_belief.rs`)

- 観測は `scope_event_of` で Z の事象へ写す。`is_imm_family`(IMM の読み・conv のビット・GJI の I/O からの推測)の源は、読めない窓
  (`WindowFacts::imm_trusted` が偽)では捨てる。それ以外の源(`Tsf`・`MozcShm`・`TaskbarGlyph`)は窓の種類に関わらず使う。
- 受け入れ(`accepts`): Low は主張なし・既定・実測の事前だけを覆す。Medium 以上は明示操作の柵(`KEY_EFFECT_SETTLE_MS` 170ms)より後に
  読み始めたものだけが覆し、観測どうしは新しいほうが勝つ(古い High は新しい Medium に勝たない、ADR-233 A')。
- 観測の時刻は、`ImeEvent` が読み取りの開始時刻を持たないので、事象を受けた時刻で代えている(V1 の近似)。
- 主張はスコープ(pid, フォーカス先のスレッド)ごとにラッチされ、時間では後退しない(I6)。Chrome のページ本体・アドレスバー・テキスト欄は
  同じ UI スレッドで、同じスコープになる。
- **既存の穴**: `TaskbarGlyph` は `is_imm_family` に入っていないので、仮に今 `ObserverReported(TaskbarGlyph)` が dispatch されると、
  Z は読める窓でも、事象を受けた時刻(取り込みから約 0.9〜1.3 秒遅い)で受け入れる。今は字形を報告する経路が本番で動かない
  (`EvidenceSource::Glyph.ui_max_level()` が `Observe` で、`indicator_glyph_shadow.rs` の `glyph_belief_in_scope` が常に偽)ので実害は無い。
  G1 でこの穴を閉じる(字形専用の写像)。

### 1.2 字形の今(ADR-261・ADR-271・ADR-273)

- 取り込み: **前面窓の変化(`EVENT_SYSTEM_FOREGROUND`)のときだけ**、900ms と 2500ms 後の 2 回(`indicator_glyph_shadow.rs` の `READ_DELAYS_MS`)。
  前回から 3000ms 未満(`MIN_TRIGGER_INTERVAL_MS`)なら取り込まない。2 回目は 1 回目の基準(`glyph_read_fence` の baseline)が無いと報告しない。
  タスクバー自身は前面の候補から除く(`is_excluded_foreground_class`)。同じ窓の中のフォーカスの移動・同じ窓の中の外部変更では取り込まない。
- 照合は粗いラベル(`BeliefLabel`: 閉・かな系・英数系)。見本は読める窓で、旧の belief(`effective_open`・`input_mode`)から付けて受け身に集める(ADR-261)。
- 柵(`fence_glyph`): 取り込みの間にフォーカスが動いた・前面と追跡がずれた・最後のモード系の打鍵(全スコープ)から `GLYPH_KEY_FENCE_MS`(1300ms、
  実測の最大の遅れ 0.885 秒 + 余裕)経っていない字形は通さない。`FocusKind`(テキストか)は見ない。
- 旧の belief への配線(ADR-273): `resolve_open_at` の独立した分岐、Medium・BeliefOnly、窓の範囲(`glyph_window_in_scope`)は
  `unobserved` で学習で降格した窓(NULL 型)だけ。静的な `Imm32Unavailable` は「打鍵量の大半を占める窓で害の面積が大きい。Chrome 系で便益を示せるまで除外」
  (ADR-273)。tsf-native・入力中継・mozc の共有メモリがある窓も除外。**本番では一度も有効にしていない**。
- 観測のみの journal(`preview_indicator_glyph`): `would_report`・`would_scope`・`would_decide`・`would_flip`。試算は旧の belief に対して行う。

### 1.3 実機の集計(dragonflyg4、`awase.log*` 10 ファイル、2026-10-20〜27、重複あり・上限の目安。team-lead の報告)

| profile | 字形と belief の一致 | 不一致の向き |
|---|---|---|
| standard(読める窓) | 30/30 | — |
| tsf-native | 72/77 | 5 件(向きは未報告) |
| imm32-unavailable(観測の無い窓) | 58/126 | 68 件すべて「字形 閉・belief 開」 |

- 「字形 開・belief 閉」は全 profile で 0 件。見本に無い字形(Miss)は imm32-unavailable で 28 件、`miss-shape` の行は 2 件。
- 字形が `effective-open-flip` の決め手になった行は 0 件(配線していないので当然)。`would_decide` は journal のメモリ上のリングにしか無く、集計できていない。
- `delay_ms` は約 900ms で固定(1 回目の待ちの値)。2 回目(2500ms)の行の件数と、窓のクラス・`FocusKind`・1 回目か 2 回目かの内訳は未報告。
- この期間の belief は大半が旧の belief(Z の既定化は 2026-10-27)。

### 1.4 所有者の方針と既存の決定

- belief と実 IME のずれが直らない窓では、ユーザーがやりたいことを belief より優先する(2026-10-23)。
- 字形は複数の証拠の 1 つ。取れたら使い、取れなければ諦める。誤判定が無いこと(Miss に倒れること)の確認が主目的(ADR-261)。
- ADR-271 の入口条件: Medium が High を追い越さない、「観測の無い窓」の述語を統一する。
- ADR-282: U1(NonText は例外)・U3(InputRelay は O2 の ON 寄りに入れない)・D8(書き込みの根拠に入れない)・O6(未確定の組は独立)・I1〜I7。

## 2. 決定(案)

### D1. 字形を使う窓(信頼表のセル)

| 窓 | G1〜G3 の初期 | 理由 |
|---|---|---|
| 読める窓(`imm_trusted`) | **使わない**(`all` でも使わない、D6) | 直接の読み(High)がある。30/30 一致で便益が無く、遅れで新しい読みと競う危険だけが増える |
| 学習で降格した窓(NULL 型) | **使う**(`unobserved`) | ADR-273 D3 のまま |
| 静的な imm32-unavailable(Chrome・Edge・VS Code・Electron 等) | **使わない。G2 の集計の後に所有者が決める**(U-G1) | ADR-273 の除外の理由(打鍵量の大半・害の面積)は Z でも同じ。68 件は字形の正しさを示さない(§3.3) |
| tsf-native | **使わない。G2 の集計の後に所有者が決める**(U-G1) | 72/77 一致だが不一致 5 件の向きが未報告。予測で状態を持つ窓で、遅れた字形が予測と競う |
| 入力中継(InputRelay) | 使わない | U3 |
| mozc の共有メモリがある窓 | 使わない | `MozcShm`(High)がある(ADR-273 R1) |

- **述語を 1 つにする**(ADR-271 の入口条件)。字形を使う窓は、次の 4 つをすべて満たす窓:
  - `!imm_trusted`
  - `!input_relay`
  - `!mozc_block`
  - `!tsf_native`(U-G1 の結果まで)

  加えて、G1〜G3 の初期は `demoted_by_learning` に限る(静的な imm32-unavailable を含めるかは U-G1)。
  `imm_trusted` は Z が既に持つ事実(`focus_tracking.rs` の `scope_shadow_window_facts` で `can_use_imm32_cross_process() && !is_demoted_by_learning()`)。
  tsf-native も `imm_trusted` が偽になるので、`!tsf_native` を明記しないと述語どおりの実装で入ってしまう。
- **確度は Medium**(旧の配線と同じ。`Observed<TaskbarGlyph>` の witness は変えない)。

### D2. 証拠の時刻を取り込みからさかのぼらせる(遅れの扱い)

字形は「取り込んだ時点より最大 `GLYPH_KEY_FENCE_MS` 前の IME の状態」を映しうる(実測の最大の遅れ 0.885 秒)。Z へ写すとき、
**読み取りの開始時刻 = 報告の時刻 − `GLYPH_KEY_FENCE_MS`** とする。

- 取り込みは窓に入った時の 2 回だけ(§1.2)なので、実際の型は「入場 `f` → 1 回目 `f+900`(さかのぼって `f−400`)・2 回目 `f+2500`(さかのぼって `f+1200`)」。帰結:
  - 入ってすぐ(`f+1030` より後)に表のキーを打つと、2 回目の字形も Z の柵(`t + 170`)で退けられる。その窓では字形は効かない(打った利用者の操作が勝つ。O3 と整合)。
  - U2 の事前(新しいスレッドは閉、`fence_ms = f`)は、1 回目(`f−400 < f`)を必ず退ける。U2 の事前を覆せるのは 2 回目だけ。
  - 便益は「窓に入った時の状態の訂正」に限る。同じ窓の中で外から閉じられた場合は、窓に入り直すまで直らない(取り込みの契機を足すかは U-G6)。
- 他の源(`Tsf` など)の時刻は事象を受けた時刻のまま。字形だけ古く扱われるので、後から届いた古い読みに負けやすい(安全側)。
- 「Medium が High を追い越さない」(ADR-271): 読めない窓で High になりうる源は `ImmCrossProbe`(IMM 系なので Z が捨てる)と `MozcShm`(D1 で窓ごと除外)だけで、
  `Tsf` は Medium。D1 の窓では字形が High と競うことは起きない。読める窓は D1 で除く。

**旧の柵(`fence_glyph` の `TooSoonAfterModeKey`)との差**(G3 で旧の柵を外すかの判断の材料):

| 打鍵 | 旧の柵 | Z のさかのぼり |
|---|---|---|
| (a) Z の主張にならない打鍵(表に無いモードキー〈`ScopeEvent::Track` だけ〉・ADR-187 の通過・awase 自身の書き込み) | 止める | **止めない**(押す前の状態を映した古い字形を受け入れうる) |
| (b) 同じスコープの表のキー | 1300ms | `170 + 1300` ms(Z のほうが強い) |
| (c) 他のスコープの打鍵 | 止める | 止めない(意図どおり) |

G1〜G3 では旧の柵を残す(二重でも安全側)。外すのは、(a) の打鍵も Z に柵の時刻として渡すようにした後に限る。

### D3. 写像: 閉 → 閉、かな → 開・かな、英数 → 入力モードだけ

| 字形のラベル | 開閉の軸 | 入力モードの軸 |
|---|---|---|
| `Closed` | 閉 | 変えない |
| `Kana` | 開 | かな |
| `Eisu` | **変えない** | 英数 |

- Engine の活性は「開 ∧ かな」(`kana_on`)なので、入力モードが英数なら開閉の主張によらず Engine は OFF になる。英数の字形で開を主張する必要は無い。
- 英数の字形で開を主張すると害がある。見本のラベルは旧の belief の開閉と入力モードから付ける(`belief_label`)。MS-IME では直接入力(閉)と半角英数の表示が同じ「A」になりうる。
  そうなると、同じ形に `Closed` と `Eisu` の見本が混ざる。`Eisu` と読まれて開を主張した後に、入力モードだけの明示操作(半角英数トグルを出る)が来ると、開・かなになり Engine が ON になる(I2)。
- G1 のオラクルで、「英数 → 開・英数」と「英数 → 入力モードだけ」の両方の I2 を並べる。

### D4. 取り消し(Miss)は Z の主張を消さない

字形が表に無い状態に変わった(`TooFar`・`Ambiguous`)とき、旧の配線は字形の枠を消した(`IndicatorGlyphMissed`)。Z は寿命で後退しない(I6)ので、
**Miss は何もしない**(前の字形の主張は、次の証拠が来るまで残る)。「取れなければ諦める」(ADR-261)はこの意味で守る。
Miss が「状態が変わった」ことを示す場合に古い主張が残る危険は、G2 の集計(Miss の直前の字形のラベル)で判断する(U-G3)。

### D5. 書き込み・授権には入れない(D8)

字形は Z の belief(Engine の入力)にだけ入る。drift correction・授権・`read_back`・`evaluate_drift` には入れない(`BeliefOnly` のまま)。

### D6. 設定と戻し方

- 既存の `[indicator_evidence] glyph`(ADR-271、`off`/`observe`/`unobserved`/`all`)をそのまま使う。新しい設定は足さない。
  - `observe`: 取り込み・照合・journal だけ(今と同じ)。
  - `unobserved`: D1 の窓で Z に入れる(G3 で `ui_max_level` を `Unobserved` に上げる)。
  - `all`: 本 ADR では `unobserved` と同じ扱い(ADR-271 D2b の「便益を journal で示せたときだけ」に従い、示せていない)。
- 戻し方: `glyph = "observe"`(または `"off"`)。既定は今の `off` のまま(オプトイン、ADR-271)。
- `belief_v2 = false`(旧の belief)のときは、字形は belief に入れない(旧の分岐は G3 で削除する、§4)。

### D7. 非テキストの欄の字形は入れない(Z のラッチと U1)

Chrome の非テキスト部・入力の無いページに入った直後の字形は「閉」と読まれうる(ADR-273 R7 の曖昧な 6 回はこの型)。旧の belief は読み出しのたびに解くので、
NonText の欄では Engine が OFF(U1)で害が無かった。**Z は主張をスコープにラッチする**ので、非テキスト部で読んだ閉が残り、同じ窓の中でテキスト欄へ移っても
前面窓は変わらず取り込み直さない。明示操作か表のキーが来るまで、実 IME は ON なのに Engine が OFF のまま残る(I1、BUG-223 と同じ症状)。

- 柵の 4 つ目として、**取り込みの時点と報告の時点の両方で `FocusKind` がテキストのときだけ**字形を Z に入れる(NonText・不明は入れない)。
- G2 の集計に、報告時の `FocusKind` ごとの件数を足す(柵を足しても非テキストの欄の字形が閉に分類されうるかを見る)。

## 3. 検証

### 3.1 オラクル(I1〜I7、G1)

`crates/awase-windows/tests/belief_engine_invariant.rs` の字母に字形の事象を足す:

- `Obs::Glyph`: 擬似 IME の真の状態の**遅れた写し**(`GLYPH_LAG_MS` 前の真。ハーネスが真の履歴を持つ)を、`report_indicator_glyph` と同じ口
  (`Observed<TaskbarGlyph>` の witness を作る試験用の口。`restored_from_journal` と同じ扱いで、本番では使わない)で報告する。
  取り込みの型(窓に入った時の 2 回)に合わせ、窓切替の後の `+900`・`+2500` でだけ起こす。
- `Obs::GlyphStale`(嘘の一種): 直前の窓の真を写す(前面の取り違え。柵を抜けた場合の最悪)。
- `Obs::GlyphNonTextClosed`(B1 の型): 同じスコープで、入場直後だけ真と違う「閉」の字形。D7 の柵があれば報告されないことを確かめる。
- 窓モデルの D1 の窓でだけ報告する。
- 測るもの: 採用した Z(既定)と「Z + 字形」の I1・I2(shared・per-thread・混ぜた窓モデル)、I3・I4・I6(0 のままか)、§1.1.1・BUG-223 の列。
  D3 の 2 つの写像を並べる。
- **合格条件**: どの設定でも I1・I2 が Z 以下、I3・I4・I6 が 0。嘘の字形(`GlyphStale`)を含む列の増分は分けて数える。
- 追加の型: 外から閉じられた D1 の窓に入り直したとき、字形が Z を直すか。

### 3.2 実 IME との照合

- CI(ADR-261 の測定の写し): 観測の無い窓の近似(自前の TSF text store の窓、IMM コンテキストなし)で、字形と実 IME の状態の一致は 6 構成すべてで確認済み。
  本 ADR では新しい CI 測定を足さない(U-G4)。
- 実機: G2 の journal で確かめる(§5)。

### 3.3 「字形 閉・belief 開」68 件の読み

- **68 件は字形の正しさの証拠にならない**。観測の無い窓の旧の belief は、ほぼ常に開に倒れる(既定 ON・`DesiredFallback`、ADR-282 §1)。
  belief がほぼ常に開なら、字形の誤りは「閉と誤る」ときだけ不一致として見え、「開と誤る」ときは一致に数えられる。「字形 開・belief 閉」0 件の非対称は、belief の偏りで説明できる。
- 一方向に閉と出る自然な仕組みの候補もある: 非テキスト部・入力の無いページへの入場で、インジケーターが閉(または無効)を示す型(D7)。
- 字形が誤りうる経路と、それぞれの柵:
  - 取り込みの遅れ(1 回目 900ms): 窓切替の直後の字形が前の窓を映す → 窓の同一性・前面の一致(`fence_glyph`)と D2 のさかのぼり。
  - 前面の取り違え(通知・ポップアップ): 前面の一致の柵。
  - 非テキストの欄: D7 の柵。
  - IME ごとの字形の違い(GJI・MS-IME・Mozc): 見本は環境ごとに集める(内蔵表なし)。Mozc 系は共有メモリがあれば除外(D1)。
- 真の状態の代わりは、G2 の journal の「字形が Z を変えた後に、利用者が直す操作をしたか」で取る(§5)。

### 3.4 shadow の試算(G2)

`preview_indicator_glyph` の `would_decide`・`would_flip` を、旧の belief ではなく **Z の写しに字形を入れたら Z の読み出しが変わるか**で計算する
(Z は小さい表なので、`ScopeBelief` を複製して `evolve` するだけ)。D1 の窓の判定は、初期の対象(降格した窓)と、U-G1 の候補(静的な imm32-unavailable・tsf-native)を
分けて試算する(候補の窓では、入れたら変わるかを数えるだけで入れない)。

## 4. コストと複雑性予算

- **入れない場合の損失**: D1 の窓に、外から IME が閉じられた状態で入ったとき(別アプリ・別の窓で閉じた、既定の ON の外れ)、Z は次の明示操作か表のキーまで ON のまま(I2)。
  初期の対象(降格した窓、NULL 型)はまれで、損失は小さい。静的な imm32-unavailable に広げると対象は増えるが、害の面積も増える(U-G1)。
- **入れるコスト(見当)**:
  - 核: 字形専用の写像(`scope_event_of` の `TaskbarGlyph` の分岐: 時刻のさかのぼり・D3 の写像・D1 の述語)と、`WindowFacts` の事実 1〜2 つ(字形を使う窓か・テキストか)。
    数十行 + 単体テスト。
  - 殻: 新しい経路は作らない。既存の `report_indicator_glyph`(`indicator_glyph_shadow.rs` から、`unobserved` のとき既に走る形、ADR-273 段階 2a)に、
    `FocusKind` の柵(D7)を足す。試算の付け替え(G2)で数十行。
  - オラクル: 字母 3 つ・真の履歴(百行前後)。golden の更新。
- **複雑性予算の相手(1-in-1-out)**: G3 で、旧の belief の字形の分岐を削除する。
  - 対象: `ImeModel::resolve_open_at` の `IndicatorGlyph` の分岐、`ObservationStore` の `taskbar_glyph` の枠と `retract_indicator`、旧の `would_glyph_decide` の試算、
    `belief_is_decided_by_glyph`(R2)。本番で一度も有効にしていない経路なので、削除で挙動は変わらない(round 1 で確認)。
  - `ImeEvent::IndicatorGlyphMissed` は、journal の再生のため列挙値を残し(ADR-082・ADR-282 の方針)、reducer を no-op にする。
    golden・`journal_replay` のテストには出ていない(`git grep IndicatorGlyphMissed -- crates/*/tests` は architecture_guard の固定だけ)。
  - `ObservationSource::TaskbarGlyph` と witness は残す(Z の入力)。
  - R2(字形が決めた belief で見本にラベルを付けない)は、旧の分岐を消せば不要になる(ラベルは旧の belief から付けるので)。
    **不変条件として残す**: ラベルを Z から付ける、または D1 を読める窓・tsf-native に広げるときは、字形が決めた Z の値でラベルを付けない(自己強化のループを作らない)。

## 5. 段階と戻し方

| 段階 | 中身 | 挙動 | 次へ進む条件 |
|---|---|---|---|
| G1 | 核の写像(D1〜D4・D7)・オラクルの字母・golden | 変わらない(`ui_max_level` は `Observe` のまま) | §3.1 の合格条件 |
| G2 | 試算を Z へ付け替え(§3.4)・D7 の柵・journal の項目 | 変わらない | 実機の journal(下)を集計し、所有者が判断 |
| G3 | `ui_max_level` を `Unobserved` に上げる・旧の分岐の削除 | `glyph = "unobserved"` の利用者の、降格した窓だけ変わる(既定 `off`) | CI と所有者の判断 |

G2 の後に実機で集める項目(状態の語と件数だけ。打鍵の内容は不要)。どれも profile・窓のクラス・`FocusKind`・読みの回(1 回目/2 回目)ごとに数える:
1. `would_decide_z` が真の件数と、そのうち `would_flip_z` の向き(閉→開・開→閉)の件数。
2. `would_flip_z` が真のとき、その後 10 秒以内に利用者が IME のモードキー(ON/OFF・ひらがな・英数)を押した件数(真の状態の代わり。字形が正しければ、利用者が直す操作は減るはず)。
3. tsf-native・静的な imm32-unavailable(U-G1 の候補)の、`would_flip_z` の向きの件数。
4. Miss(`TooFar`・`Ambiguous`)の直前の字形のラベルの件数(U-G3)。

戻し方: 設定 `glyph = "observe"`。コードの戻しは G3 の revert(`experiment-logging.md` に従い、失敗の条件を本文に書く)。

## 6. 未決(所有者に確かめる)

| # | 問い | 提案 |
|---|---|---|
| U-G1 | 静的な imm32-unavailable(Chrome 等)と tsf-native に字形を入れるか | G2 の集計(窓のクラス・`FocusKind` ごとの向きと、利用者の訂正の件数)の後に決める。それまでは入れない |
| U-G3 | Miss で前の字形の主張を消さない(D4)でよいか | よい。G2 の集計で「Miss の前の字形」が閉・開のどちらかに偏るなら見直す |
| U-G4 | 新しい CI 測定(実 Chrome の imm32-unavailable で字形と実状態)を足すか | 足さない(ADR-261 の 6 構成で足りる)。G2 の実機の集計で代える |
| U-G5 | `all` を読める窓に広げるか | 広げない(便益が示せていない、ADR-271 D2b) |
| U-G6 | 同じ窓の中の外部変更を拾うため、取り込みの契機(フォーカスの移動・一定の間隔)を足すか | 足さない(G3 の後の別の決定。取り込みの費用と、遅れた字形が打鍵と競う危険が増える) |

(round 1 の U-G2「字形より古い High に勝つか」は、D1 の窓では起きないので外した。D2 の最後の項目。)

## 7. 却下した案

- **旧の belief の配線(ADR-273)をそのまま有効にする**: Engine の入力は Z になった(ADR-282 V2-3)ので、旧の belief に入れても Engine には効かない。
- **字形を Low にする**: Z の Low は既定・実測の事前しか覆さないので、Z が明示操作・予測で ON を主張している窓を直せない。
- **字形の時刻をさかのぼらせず、旧の打鍵の柵だけで守る**: 柵は全スコープの最後の打鍵で測るので、Z のスコープの主張との前後が保証されない。
- **英数の字形で開も主張する**: D3。閉と取り違えたときに I2 を作る。

## 8. Opus レビュー round 1 への対応(2026-10-27)

| ID | 対応 |
|---|---|
| B1 | 反映: D7(取り込みと報告の両方で `FocusKind` がテキストのときだけ)、§3.3 の文を Z の性質に合わせて直した、G2 の集計に `FocusKind` ごとの件数、オラクルに `GlyphNonTextClosed` |
| M1 | 反映: §1.2 に取り込みの型、D2 に入場時の 2 回とその帰結(入ってすぐの打鍵・U2 の事前)、summary と §4 の便益を「窓に入った時の状態の訂正」に限定、U-G6 |
| M2 | 反映: D1 の静的な imm32-unavailable は G2 の後に決める(U-G1)、初期の対象は降格した窓だけ、§3.3 の読みを「証拠にならない」に直した、G2 の集計に窓のクラス・`FocusKind`・読みの回の内訳 |
| M3 | 反映: D3 を「英数 → 入力モードだけ」に変更、オラクルで両方の写像を並べる |
| M4 | 反映: D2 に旧の柵との差の表、G3 で旧の柵を外す条件 |
| m1 | 反映: U-G2 を外し、起きない理由を D2 に書いた |
| m2 | 反映: §4 に R2 を消してよい理由と、残す不変条件 |
| m3 | 反映: `IndicatorGlyphMissed` は列挙値を残し reducer を no-op、テストに出ていないことを確認 |
| m4 | 反映: §1.2・§1.3 に 2 回目と基準の条件、内訳が未報告であること |
| m5 | 反映: D1 の述語に `!tsf_native` を明記 |
| m6 | 反映: §3.1 に `GlyphNonTextClosed` |
