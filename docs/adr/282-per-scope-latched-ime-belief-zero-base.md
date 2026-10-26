---
id: ADR-282
title: |-
  IME の開閉と Engine の活性の belief をゼロベースで作り直す: IME の保持単位(スレッド)ごとに「最後に受け入れた証拠」を保持し、明示操作は ON・OFF とも同じ重さで、窓切替・観測の寿命切れで古い値へ戻さない
summary: |-
  BUG-223(GJI の tsf-native / imm32-unavailable 窓で、IME は ON なのに Engine が OFF のまま数秒〜数十秒続き、「こうしき」が「るしき」になった)と、
  所有者の方針(2026-10-26)「IME が ON でかなモード(ひらがな・カタカナ)なら必ず Engine ON。逆は OFF」を受けて、belief をゼロベースで設計し直す。
  今の belief は、明示意図の保管場所が 5 つ(`desired_open`・`last_intent`・`IntentStore`・`persistent_explicit_off`・`key_effect.open`)あり、
  寿命がそれぞれ違い、ひらがなキー(F2)の ON は予測、IME-OFF キーの OFF は意図、と経路で非対称で、窓切替は予測・観測を捨ててグローバルな `desired_open` を持ち越す。
  基準の測定(PR #857、擬似 IME・擬似窓のオラクル)では、msime shared・全字母・長さ 3 で I1 違反 405 本、主因は `desired-fallback` 297 本、最小列は `key:hiragana > focus:wezterm`。
  新設計は、(1) belief を **IME が状態を持つ単位(GJI・MS-IME とも CI 実測でスレッド)ごと**に持ち、(2) 各単位は「最後に受け入れた証拠」(明示操作・信頼できる観測・その単位の既定)を
  **ラッチ**し、寿命切れで別の値へ後退しない、(3) 明示操作は ON・OFF・モードを同じ型で記録し(F2 の予測も明示操作の値として持つ)、新しい信頼できる観測だけが覆す、
  (4) 観測を信頼するかは「窓の種類 × IME × 証拠源」の表で決め、読めない窓の IMM の読みは反証に使わない、(5) 不確実なとき(単位に何も無い)は、読めない窓は ON 寄り、読める窓は観測、
  (6) 明示意図があってもポーリングを止めない(BUG-151 原因③の再発を防ぐのは、ポーリングを意図から切り離すことで行う)、(7) 授権(OpenWarrant)には既定値を入れない、とする。
  IME への書き込み(actuation)の方針は変えない(ADR-191・212 のまま)。純粋な核をまず擬似 IME の上で実装して基準の測定と比べ、shadow → 切替 → 旧の撤去の順で移る。
status: |-
  起草(2026-10-26)。所有者の判断 4 件(かなの範囲・不確実なときの既定・最後の明示操作が勝つ・窓ごとに独立)を決定として反映。
  未レビュー(Opus の批判的レビュー待ち)。未決 U1〜U8 は所有者の確認待ち。実装はまだ(コード・テストは不変)。
related_adr:
  - "ADR-087"
  - "ADR-089"
  - "ADR-098"
  - "ADR-106"
  - "ADR-165"
  - "ADR-179"
  - "ADR-187"
  - "ADR-188"
  - "ADR-191"
  - "ADR-199"
  - "ADR-205"
  - "ADR-208"
  - "ADR-212"
  - "ADR-213"
  - "ADR-214"
  - "ADR-233"
  - "ADR-245"
  - "ADR-255"
  - "ADR-260"
  - "ADR-262"
  - "ADR-263"
  - "ADR-268"
  - "ADR-273"
  - "ADR-277"
  - "ADR-279"
  - "ADR-280"
---

# ADR-282: IME の開閉と Engine の活性の belief をゼロベースで作り直す

調べた版: `origin/develop` `ebd744b6d`(`git rev-parse --short HEAD`)。行番号はこの版のもの。
棚卸し(PR #858、`origin/docs/belief-zero-base-inventory` の `docs/tasks/belief-zero-base-inventory-2026-10-26/`、以下「棚卸し」)は `ff75d5d58` で書かれており、
ここで引いたコードの行は、本 ADR の起草時に `ebd744b6d` で読み直したもの(違う行は本文で直した。例: `compute_state` は `src/engine/engine.rs:269`)。
基準の測定は PR #857(`origin/test/belief-engine-invariant`、`docs/tasks/belief-engine-invariant-baseline-2026-10-26.md`、golden
`crates/awase-windows/tests/golden/belief_engine_invariant_violations.txt`。以下「基準の測定」)。cargo はローカルで実行していない。実機のログは読んでいない。

## 0. 決定の要約

| # | 決定 | 今との違い |
|---|---|---|
| D1 | belief を **IME の保持単位(スコープ)ごと**に持つ。スコープの鍵は(プロセス, スレッド)、取れなければ hwnd | 今はグローバルな `ImeModel` 1 つ + hwnd ごとの `IntentStore` + hwnd キャッシュ |
| D2 | 各スコープは開閉軸・モード軸それぞれに **「最後に受け入れた主張(claim)」を 1 つだけ**持つ。主張は寿命切れで消えない(ラッチ) | 今は 6 段の優先順位 + 鮮度 3 秒 + `DesiredFallback` で、寿命が切れると別の値へ後退する |
| D3 | **明示操作**(IME-ON/OFF キー・トグル・ひらがな/カタカナ/英数キー・`keys.ime_*`・Engine の `SetOpen`)は、方向・キーの種類によらず同じ型 `ExplicitOp` で主張になる。値の出所(awase が書いた/表の予測)は `certainty` に持つ | 今は `UserImeSetIntent`(OFF キーなど)と `KeyEffectPredicted`(F2 など)に分かれ、F2 の ON だけ窓切替で消える(A1・A9、BUG-223) |
| D4 | 主張を覆せるのは、**より新しい明示操作**か、**受け入れ基準を満たす新しい観測**だけ。観測の受け入れは「窓の種類 × IME × 証拠源」の信頼表と、柵(settle 170ms・字形 1300ms・フォーカス epoch+hwnd)で決める | 今は Medium 合意・High 単独なら窓の種類を問わず `derive_any` が勝つ(BUG-219・223(a)) |
| D5 | **不確実なとき**(スコープに主張が無い)の既定: 読めない窓は **ON(かな)寄り**、読める窓は最初の観測。測定済みの事実(新しいスレッドは閉で始まる、ADR-191 付属の CI 実測)は既定より強い「実測の事前」として使う | 今は前窓の `desired_open` を持ち越す / ON の HeuristicDefault / OFF の HeuristicDefault が窓の種類ごとに混在(A5・A9) |
| D6 | **ポーリングを明示意図から切り離す**。読める窓では、明示操作の直後も settle 後の読み直しと周期の poll を続ける(打鍵中の読み取りの抑止〈typing-idle〉は残す) | 今は明示意図があると `reschedule_ime_refresh` が止まる(`crates/awase-windows/src/runtime/mod.rs:1196,1217`) |
| D7 | Engine の活性 = `user_enabled ∧ 日本語 IME ∧ 開 ∧ かな(ひらがな・カタカナ、ローマ字入力)`。式は今の `compute_state` の 4 条件のまま、入力が新しいスコープの主張になる | `Unknown` を「不確実」として D5 の方針で読む点が違う |
| D8 | **授権(OpenWarrant)には D5 の既定を入れない**。IME への書き込みを許す根拠は、今と同じく明示操作・観測だけ | 今の `OwnSsot` は `desired_open`(既定・持ち越しを含む)を読む(`crates/awase-ime-core/src/state/open_warrant.rs:224`) |
| D9 | IME への書き込みの方針(何をいつ書くか)は変えない。ADR-191・199・212・213 のまま | 変更なし |

## 1. 背景

### 1.1 実機の事故(BUG-223)

`origin/fix/focus-desired-fallback-off:docs/known-bugs/BUG-223.md`(PR #854、未マージ)より。dragonflyg4(GJI、awase develop 43359222)で「こうしき」が「るしき」になった。
journal の `effective-open-flip → false` のうち、所有者の IME-OFF キー(`ExplicitIntent`)でない区間が 5 つ(03:48:28・03:49:31・04:33:48・04:34:27・04:37:54Z)。

- (a) 区間 1・4: 観測が false を出した。`DeriveMedium{ObserverPoll}` の単独、`DeriveHigh(ImmCrossProbe)`。GJI の tsf-native / imm32-unavailable 窓の IMM の読みが偽の閉を返した(BUG-219 と同型)。
- (b) 区間 2・3・5: 観測が無く `desired_open`(前窓の false)へ後退した(`DesiredFallback`)。区間 3 は直前の操作が F2(ON)だった。`KeyEffectPredicted` は `desired_open` を書かず、
  窓切替(`reduce_focus_changed`、`crates/awase-ime-core/src/state/ime_model.rs:1157`)が予測 `key_effect` を捨てると、前の OFF キーが書いた `desired_open=false` に戻る。
- BUG-223 自身が未確認としている点: (b) の各区間の直前に明示 OFF があったか。

### 1.2 基準の測定(PR #857)が示したこと

擬似 IME・擬似窓(ADR-277)の上で、各事象の直後に「真の状態が 開×かな なのに Engine が OFF」(I1)を検査した。以下は基準の測定の文書と golden の値の引用。

- msime shared・全字母(19 種)・長さ 3: I1 違反 405 本。2000ms 事象なしでは 1 本も治らない。読める窓の poll が 1 回届いても 375 本残る。**読めない窓で終わる違反は 100% 残る**。
- 分類: `belief-off/desired-fallback` 297、`derive-high` 30、`explicit-intent` 23、`most-recent-trusted` 21、`key-effect` 18、`belief-on-but-engine-off(ObservedEisu)` 10、`derive-medium` 6。
- 小さい字母・長さ 4 の最小列は 4 本だけで、`key:hiragana > focus:wezterm` のように **OFF キーが要らない**。「F2 が `desired_open` を書かない」非対称そのものが原因。
- 列の性質: 「awase の操作・窓切替・正しい観測だけ」228 本、嘘の観測を含む 56、外部変更を含む 121(msime shared)。
- per-thread(窓ごとに別スレッド、初めての窓は閉)では違反は 9 本に減る。
- 基準の測定が自分で書いている限界: 窓切替の本物の処理(hwnd キャッシュの復元・TSF 窓の ON キャッシュの復元)を写していない(TSF 窓へ戻る列は偽陽性の疑い)、読めない窓の IMM の嘘は字母に無い、周期の poll は事象に無い、`Engine::on_input` は呼ばない。
- 本 ADR の起草時に `crates/awase-windows/tests/support/harness.rs`(develop)の `key_mods` を読んで確かめたこと: ハーネスの物理キーは全部 `has_shadow_action: false` で予測の経路に流す。
  本番の 0x16(IME-ON)・トグル解決キーは `UserImeSetIntent` の経路(棚卸し 01-map §4.1)なので、**`key:ime-on`・`key:hankaku-zenkaku` で始まる最小列は本番と経路が違う**(本番で同じ違反になるかは未確認)。
  F2(0xF2)は本番でも予測の経路なので、`key:hiragana` の最小列は本番と同じ経路。

### 1.3 今の構造の問題(棚卸し 04-complexity の X1〜X11 のうち、本 ADR が解くもの)

| # | 問題 | 本 ADR での扱い |
|---|---|---|
| X1 | 明示意図の保管場所が 5 つで寿命がばらばら | `ExplicitOp` 1 つ(スコープごと)に統合(D3) |
| X2 | `desired_open` の意味が 8 つ混在(意図の値・観測ゼロの既定・drift の比較・持ち越し・placeholder・授権の入力・TSF 復元の入力・共有メモリの判断) | `desired_open` を廃止し、意味ごとに置き場所を分ける(§4.8 の対応表) |
| X3 | ON と OFF の経路・寿命の非対称(A1〜A9) | 方向で分けない(D3)。寿命は「次の明示操作か受け入れた観測まで」で同じ |
| X4 | 窓切替の後処理が窓の種類ごとに別の書き方 | 窓切替はスコープの切り替えだけ(§4.5)。窓の種類の差は既定(D5)と信頼表(D4)に集める |
| X6 | 優先順位が 2 層(`resolve_open_at` の上に `IntentStore` の上書き) | 1 層(スコープの主張)にする |
| X8 | 半角英数トグルの状態が 2 か所 | 表現はモード軸の主張に寄せる。出入りの条件は今の規則のまま(§4.10。ここは慎重に、取り下げ #6 を繰り返さない) |
| X10 | 初期値 `desired_open=true` の placeholder と特例 | スコープの「主張なし」で表す(D5) |

解かないもの: X5(外からの変化への追随の窓 4 つの統合)は観測の受け入れの入口が 1 つになるので尾部の重複は消えるが、窓の機構そのものは残す。X7(Engine を通さない入口)は U6 に挙げる。X9・X11 は本 ADR の範囲外。

### 1.4 所有者の方針と判断(2026-10-26、決定として扱う)

方針: IME が ON でかなモード(ひらがな・カタカナ)なら、必ず Engine ON。例外は `user_enabled` が偽のとき(ほかの例外候補は U1〜U3)。逆に IME OFF・半角英数・直接入力なら Engine OFF。
擬似窓・擬似 IME をそろえたので、設計をゼロベースで考え直す。

| # | 所有者の判断 | 本 ADR で対応する決定 |
|---|---|---|
| O1 | 「かな」の範囲 = ひらがな・カタカナ。半角英数・直接入力は含まない | D7・§4.7 |
| O2 | 不確実なとき(観測が読めない窓で IME の状態が分からない)は窓の種類で分ける。読めない窓は ON 寄り、読める窓は観測を信じる | D5・§4.6 |
| O3 | 最後の明示操作が勝つ。ON も OFF も対称。窓切替や観測の寿命切れで古い値に戻らない。観測が反証する場合の扱いは設計する | D2・D3・D4・§4.4 |
| O4 | 窓切替では IME の状態を窓ごとに独立させ、持ち越さない。新しい窓は、その窓の直前の記録、無ければ不確実の方針 | D1・§4.5 |

O4 の「窓ごと」は、本 ADR では **「IME が実際に状態を持つ単位ごと」**と読む。根拠: `docs/adr/191-gji-state-scope-spec.md` §2.1 の CI 実測(GJI の ATOK/MS-IME プリセット・Microsoft IME 本体、
`SPI_GETTHREADLOCALINPUTSETTINGS = 0`)で、**保持の単位はスレッド**(同じスレッドの別入力欄・別最上位窓は状態を共有、別スレッド・別プロセスは独立)、HWND 単位の記憶は観測されなかった。
同 §3.1 は「同じスレッドの別 hwnd で状態が変わった後に古い hwnd のキャッシュを戻すと実状態と食い違う」と書いている。所有者の言う「窓ごと」と食い違う可能性があるので、U4 で確認する。

## 2. 不変条件(設計の目標)

検査の語彙は `crates/awase-windows/tests/support/harness.rs` と基準の測定のもの(`key(vk)`・`observe`/`observe_value`・`focus_to`・`user_set_open`・`external_set_open`・`advance_ms`・`activation()`、
擬似 IME の `TrueState`)。「真」は擬似 IME の、いまフォーカスのあるスレッドの状態。

| # | 不変条件 | 検査の形(どんな列を与えて何を見るか) | 目標 |
|---|---|---|---|
| I1 | 真が 開 ∧ かな(conv が NATIVE。ひらがな・カタカナ)∧ `user_enabled` ∧ 日本語 IME なら、Engine は Active | 基準の測定のオラクルそのもの(全列挙・長さ 3〜4・ランダム探索)。各事象の直後に `activation()` を見る | 「awase の操作・窓切替・正しい観測だけ」の列で **違反 0**。外部変更・嘘の観測を含む列は §2.1 |
| I2 | 真が 閉、または 開 ∧ 半角英数(NATIVE なし)なら、Engine は Inactive | 同じオラクルの I2 | 「awase の操作・窓切替・正しい観測だけ」の列で、§2.1 の「既定の区分」を除き違反 0。既定の区分の件数は別に数えて報告する |
| I3 | 最後の明示操作が勝つ(対称): スコープ S で明示操作 v(ON/OFF/モード)をした後、S に対する新しい明示操作・受け入れた観測が無い限り、S にフォーカスがある間の belief は v | 列 `op(v) @S`、その後に {他スコープへの `focus_to`・`advance_ms`(3 秒・60 秒)・受け入れ基準を満たさない観測(読めない窓の IMM、柵の内側の観測)}を任意に挟み、`focus_to(S)` の後の belief と `activation()` を見る。v を ON・OFF の両方で回す | 違反 0 |
| I4 | スコープの独立: スコープ T の事象(キー・観測・外部変更)は、S の主張を変えない | 変成関係(metamorphic): ランダムな列 L について、S にフォーカスがある時点の S の主張が、L から「S の事象と全体の事象(`user_enabled`・日本語 IME・パニックリセット)だけ」を残した列で計算した主張と一致する | 違反 0 |
| I5 | 古い観測で戻らない: 明示操作の時刻 + settle(`KEY_EFFECT_SETTLE_MS`=170)より前に読み始めた観測、字形の柵(`GLYPH_KEY_FENCE_MS`=1300)の内側の字形、旧 epoch/hwnd の観測は主張を変えない | `op(v)` の直後に、読み取り開始時刻を op より前にした観測(遅れて届く)を流し、主張が変わらないこと。柵の境界の前後(169/171ms)で回す | 違反 0 |
| I6 | 寿命で後退しない: 主張は時間の経過だけでは変わらない | 任意の状態で `advance_ms(N)`(N = 3,500・60,000)だけを流し、主張・`activation()` が変わらないこと | 違反 0 |
| I7 | 既定は書き込みを授権しない: スコープの主張の根拠が D5 の既定のとき、`issue_open_warrant` に渡す awase 自身の意図は `None` | 既定だけのスコープで、授権の入力(`warrant_context` 相当)を検査する。書き込み命令(ハーネスの `WriteCommand`)の件数が今より増えないこと | 違反 0、書き込み件数は今以下 |
| I8 | 例外: `user_enabled` が偽なら、どの列でも Inactive(`UserDisabled`) | 列の途中で `user_enabled` を切り替える事象をハーネスに足す(今は常に真。§5.2 の拡張) | 違反 0 |
| I9 | Engine が非活性になった瞬間に保留中のキーを flush する(今の `check_active_transition`) | ハーネスは `on_input` を呼ばないので、今は Engine の単体テストで守る(§5.2) | 今のテストを維持 |

### 2.1 0 にできない違反(弱点として明記する)

- **外部変更 × 読めない窓**: 言語バー・他アプリが読めない窓の IME を変えたとき(`external-on`/`external-off`)、awase には証拠が届かない。新設計では主張が明示操作か既定のまま残るので、
  `external-off` は I2 違反、`external-on`(主張が OFF のとき)は I1 違反のまま残る。直るのは、次の明示操作、信頼できる証拠(MozcShm・ADR-205 の監視窓・字形を belief に入れる設定)、
  または ADR-279 型の打鍵列による反証(未実装)が来たとき。今の設計でも直らない(基準の測定: 読めない窓は 100% 残る)ので、悪化ではないが、改善もしない。
- **嘘の観測 × 読める窓**: 信頼表で「信頼する」にした源が嘘をつけば、主張は嘘に従う(BUG-189 の窓では新しい Medium を信じるのが正しかった。どちらを信じても片方の事故は残る)。
- **既定の区分**: 読めない窓・スコープに主張が無い・実測の事前も無い、のとき D5 で ON に倒す。真が閉なら I2 違反。これは O2 の判断の帰結で、件数を数えて報告する。
- **表の予測の外れ**: F2 などの値は表の予測(ADR-191 決定 3。1 手で約 98.5%)。読めない窓では観測で訂正できないので、外れは次の明示操作まで残る(今も同じ。今は窓切替で消えるので、
  「外れが窓切替で偶然直る」ことが新設計では無くなる)。BUG-211・212 の予測器の誤りは、新設計では窓切替をまたいで残る分だけ影響が長くなる。

## 3. 新設計の全体像

```
            物理キー・Engine の要求・観測・フォーカス・タイマー(殻が集める事実)
                                │
                                ▼
   classify_*(今のまま。生の値 → 型のある証拠)   kp_predict_key_effect(今のまま。表引き)
                                │
                                ▼
            BeliefEvent(値)  ──►  evolve(&mut BeliefStore, &BeliefEvent)   ← belief の唯一の書き込み点
                                │         (スコープ選択 → 受け入れ判定 → 主張の更新)
                                ▼
            decide(&BeliefStore, &BeliefEvent) -> Vec<BeliefCommand>        ← 命令は値で返す
              ・ScheduleRead { scope, at, reason }(settle 後の読み直し・周期 poll)
              ・ActivationChanged { from, to }(殻が Engine の flush を回す)
              ・Journal(BeliefRecord)(根拠・却下の理由)
                                │
                                ▼
            derive(&BeliefStore, globals) -> BeliefView { open, mode, basis, activation }
              (Engine の ctx・drift correction・授権・lang-detect が読む。読み取り専用)
```

- `evolve` は今の `ImeModel::reduce` の位置に入る(`.claude/rules/ime-belief-architecture.md` の「書き込み点は 1 か所」を引き継ぐ)。`decide` と `derive` は I/O を持たない純粋な関数で、
  `docs/fcis-vocabulary.md` の Decider の形(`decide`/`evolve`、命令は値で返す)に合わせる。置き場所は OS 非依存の `crates/awase-ime-core`(今の `ime_model.rs` と同じ crate)。
- IME への書き込み(actuation)は、今の `kp_shadow_actuate`・`dispatch_ime_set_open`・drift correction の経路のまま。`decide` は IME への書き込みを返さない(D9)。

## 4. 設計

### 4.1 スコープ(D1)

- **鍵**: `ScopeKey = (pid, tid)`。`tid` はフォーカス先 hwnd の `GetWindowThreadProcessId`(殻が取り、事実として渡す)。取れないときは `ScopeKey::Hwnd(hwnd)` に落とす。
- **根拠**: GJI・MS-IME 本体とも保持の単位はスレッド(`191-gji-state-scope-spec.md` §2.1、CI 実測。上の 1.4)。同じスレッドの別 hwnd は状態を共有するので、hwnd を鍵にすると
  「同じスレッドの別 hwnd で変えた後に古い hwnd の記録が戻る」食い違いが起きる(同 §3.1)。
- **未確認(U4)**: Chrome・VS Code・WezTerm など内部で複数スレッド・プロセスを使う TSF ネイティブのアプリで、フォーカス hwnd のスレッドが IME の状態を持つスレッドと一致するか
  (同 §4 が未確認と書いている)。一致しない場合は hwnd 単位より粗く(または細かく)なる。ハーネスと実機 CI で確かめる(§5)。
- **フォーカスの柵**: hwnd と focus epoch は、観測の受け入れの柵(ADR-106 の `FocusFence`)として今のまま使う。スコープの鍵とは別。
- **全体の状態**(スコープに入れないもの): `user_enabled`、日本語 IME か(`is_japanese_ime`)、IME の種類(TIP 同定)、パニックリセットの guard。
- **スコープ表の寿命**: 主張の値は寿命で後退しない(I6)が、表の行は無限には持たない。プロセス終了・スレッド消滅は殻が知れば消す。知れない場合は、
  最後にフォーカスがあってから `HWND_CACHE_MAX_AGE_MS`(3,600,000、今の hwnd キャッシュと同じ値。`crates/awase-windows-base/src/tuning.rs:223`)で行を捨てる。
  捨てた行に戻ったときは「主張なし」(D5)。tid の再利用で別スレッドの行を読むおそれは、pid と組にすることで下げるが 0 ではない(弱点)。

### 4.2 状態(D2)

```rust
// 擬似コード。名前は案。
struct BeliefStore {
    scopes: Map<ScopeKey, ScopeBelief>,
    focused: Option<(ScopeKey, FocusFence, WindowClass)>,
    observations: ObservationStore,   // 今の観測プール。受け入れ判定の材料。フォーカス切替で今どおり柵を進める
    globals: Globals,                 // user_enabled・is_japanese_ime・ime_kind・panic guard
}

struct ScopeBelief {
    open: Option<Claim<bool>>,        // None = 主張なし(不確実)
    mode: Option<Claim<ModeClass>>,
    last_op: Option<ExplicitOp>,      // 最後の明示操作(主張に勝っていても負けていても記録。drift・授権・診断が読む)
    half_width: HalfWidthLink,        // 半角英数トグル・戻り待ちとの結び付き(§4.10)
    last_focused_at: Tick,
}

struct Claim<T> {
    value: T,
    basis: ClaimBasis,                // どの証拠で決まったか(journal の `effective_open_basis` の後継)
    at: Tick,                         // 証拠の時刻(観測は読み取り開始時刻)
    fence: ClaimFence,                // この主張を覆せる観測の下限時刻(op の at + settle、字形は + 1300ms)
}

enum ClaimBasis {
    Explicit { origin: OpOrigin, certainty: Certainty, press: Option<PressId> },
    Observed { source: ObservationSource, confidence: ObservationConfidence },
    MeasuredPrior(PriorRule),         // 実測の事前(新しいスレッドは閉で始まる、など)
    UncertainDefault(WindowClass),    // D5 の既定
}

enum Certainty { Written, Predicted }   // Written = awase が書いた(0x16/0x1A・トグルの shadow 書き込み・keys.ime_*・SetOpen)
                                        // Predicted = 生キーを IME へ通し、効果を表から予測した(F2・F1・F0・無変換/変換 など)
enum OpOrigin { UserKey, UserCommand, Engine, LangDetect, HalfWidthToggle, PanicReset }
enum ModeClass { Kana, Eisu, KanaInputMethod, Unknown }   // Kana = ひらがな・カタカナ(ローマ字入力)。KanaInputMethod = JIS かな入力方式
```

- `ModeClass::Kana` が O1 の「かな」。今の `InputModeState` との対応: `ObservedRomaji`・`AssumedRomaji{..}` → `Kana`、`ObservedEisu` → `Eisu`、`ObservedKana`(JIS かな入力方式、
  `src/engine/mode_state.rs:14` の doc)→ `KanaInputMethod`、`Unknown` → `Unknown`。`ObservedKana` を Engine ON にするかは U5(今は OFF)。
- 今の `InputModeState` の確度の区別(Observed/Assumed)は `ClaimBasis` が持つ。Engine の `InputContext` へは今の `InputModeState` に写して渡す(Engine 側は変えない。ADR-019 の境界)。

### 4.3 更新: `evolve` の手順

`evolve` は事象ごとに次の順で処理する。

1. **スコープの決定**: 事象が属するスコープを決める。
   - 観測: 観測の hwnd/epoch が今のフォーカスの柵と一致するときだけ、今のスコープへ(ADR-106 のまま)。一致しない観測は捨てる。
   - 明示操作: **押下時点の前面**(`win32::foreground_scope()`、`crates/awase-windows/src/win32.rs:82`。殻が事実として渡す)のスコープへ。フォーカスの追跡が遅れた(BUG-218)とき、
     追跡上の窓ではなく実際に押した窓に記録するため。前面が取れなければ追跡上のスコープ。
   - フォーカス変化: §4.5。
2. **受け入れ判定**(§4.4): 事象が、そのスコープの今の主張を覆せるかを決める。覆せなければ主張は変えず、却下の理由を journal に残す(`decide` の `Journal`)。
3. **主張の更新**: 受け入れたら、その軸の主張を事象の値・根拠・時刻で置き換える。明示操作は `last_op` も更新する。
4. **観測プール**: 観測は受け入れの可否によらず今どおり `ObservationStore` に記録する(Medium 合意の材料・診断のため)。主張の更新は 2 の判定だけで行い、プールの寿命切れは主張に影響しない(I6)。

### 4.4 優先順位と反証(D3・D4、O3)

主張の強さは「種類の順位」ではなく、**時刻の新しさ + 受け入れ基準**で決める。今の主張を新しい事象が覆す条件:

| 新しい事象 | 今の主張の根拠 | 覆す条件 |
|---|---|---|
| 明示操作(ON・OFF・モード、`Written`/`Predicted` とも) | どれでも | **常に覆す**(最後の明示操作が勝つ。O3) |
| 受け入れ可能な観測(下の信頼表で「反証可」) | 明示操作 | 観測の読み取り開始時刻が主張の `fence`(op の時刻 + 170ms、字形は + 1300ms)より後、かつ観測の強さが基準を満たす(High 単独、または Medium 以上が 2 つ矛盾なく。今の `derive_any` と同じ基準) |
| 同上 | 観測 | 新しいほうが勝つ(同時刻なら信頼度の高いほう。ADR-233 の A' と同じ順位キー)。ただし既存の主張が High で新しい事象が Medium 単独のときは、Medium が 2 つ矛盾なくそろうまで待つ(BUG-219 の型。U7) |
| 同上 | 実測の事前・既定 | 1 つで覆す(基準: Medium 以上なら単独でよい) |
| 信頼表で「反証不可」の観測(例: 読めない窓の IMM の読み) | どれでも | **覆さない**。記録だけ(BUG-223(a)・BUG-219) |
| 時間の経過・観測の寿命切れ | どれでも | **覆さない**(I6) |
| 実測の事前・既定 | どれでも | 主張なしのときだけ入る(§4.6) |

**信頼表**(`TrustTable`、データで持つ。初期値は今の挙動から作り、ハーネスと実機 CI で確かめる):

| 窓の種類 × IME | IMM 系(`ObserverPoll`・`ImmCrossProbe`・`FocusProbe`) | `MozcShm` | ADR-205 の外部変化の監視窓(窓内の 1→0 の遷移) | ADR-188/244 の直接観測(窓内の遅延 conv 読み取り) | `TaskbarGlyph` |
|---|---|---|---|---|---|
| 読める窓(Standard・学習で降格していない)× 任意 | 反証可 | 反証可 | — | — | 設定が belief 配線のときだけ反証可(今は `ui_max_level` が Observe なので不可) |
| tsf-native・imm32-unavailable × GJI 系(Mozc 含む) | **反証不可**(BUG-223(a)・BUG-219) | 反証可(ADR-268) | 反証可(開閉の遷移のみ) | モード軸のみ反証可(開閉は不可。要確認) | 同上 |
| tsf-native・imm32-unavailable × MS-IME 本体 | 未確定(U7。今は TsfNative で poll が走らない) | — | 反証可 | モード軸のみ反証可 | 同上 |
| InputRelay | 反証不可 | — | — | — | — |

「反証可」の源でも、Medium 単独と High 単独の扱いは上の表の条件に従う。表の各セルの根拠は §6 の台帳の行に対応させる(未確定のセルは、実装前にハーネスの全列挙と実機 CI で埋める)。

**`Written` と `Predicted` の違い**は覆す条件には無い(対称)。違いが出るのは次の 2 点だけ:
- drift correction(ADR-212 P6 (a)、`crates/awase-ime-core/src/state/drift_correction.rs`)の対象は `Written` の明示操作だけ(awase が書いたものの再試行)。`Predicted` は観測に覆されたら従うだけで、書き直さない(ADR-191 決定 1・3 のまま)。
- 授権(D8・§4.9)で「awase 自身の意図」として渡すのは `Written` だけ。

### 4.5 窓切替(D1、O4)

フォーカス変化の事象(今の `FocusChanged`)で `evolve` がすることは **スコープの切り替えだけ**:

1. 前のスコープの行は**そのまま残す**(主張・`last_op` を消さない)。前のスコープへ戻れば、その主張がそのまま効く(I3)。
2. 新しいスコープの行を引く。有れば、その主張を使う(「その窓の直前の記録」)。無ければ「主張なし」で、§4.6 の既定へ。
3. 観測プールの柵を進める(旧 epoch/hwnd の観測を新しい窓で使わない。ADR-106・C-c7 のまま)。
4. 予測の追跡状態(`key_track`)は、スコープの行に持たせ、戻ったときに使う(今は窓切替で捨てる。`191-gji-state-scope-spec.md` §3.4 の「GJI は同じスレッド内で閉→開でモードを保持」を予測の入力に残すため)。
5. `InputBarrier::FocusTransition`(settle)は今のまま。

これで消える処理(§4.8): グローバルな `desired_open` の持ち越し、`plan_tsf_cache_restore`(ON のキャッシュだけ復元)、`apply_hwnd_cache_restore`、`HwndCacheRestored`、
`reset_stale_ime_on_for_imm_broken`(ON の HeuristicDefault)、退場時のキャッシュ保存と `MIN_FOCUS_DURATION_MS`(100ms 未満の滞在で保存しない、BUG-57 型)の特例。
スコープの行は常に最新なので、「保存し忘れた古いキャッシュが新しい意図に勝つ」(`IntentStore::invalidate_for_cache_restore` の doc、BUG-51 追補 v3)型は起きない。

**BUG-128・BUG-218 を再発させない理由**:
- BUG-128(explorer の別 UWP 入力面へ移ったら、無関係な過去の ON が復元された): 新設計は `(pid, class_name)` のような粗い鍵で記録を引かない。鍵は(pid, tid)で、
  記録が無ければ「主張なし」。ただし、別 UWP 入力面が同じスレッドなら同じスコープになる。それは IME の状態が実際に共有される場合なので正しい(と CI 実測からは読める)が、
  explorer の UWP 入力面のスレッド構成は未確認(U4)。
- BUG-218(Ctrl+無変換で OFF にした直後、フォーカスの追跡の遅れで 28 秒前の ON が復元された): 新設計では OFF の明示操作を押下時点の前面のスコープに記録する(§4.3-1)。
  追跡が遅れて後から入場しても、そのスコープの主張は OFF(新しいほう)。今の `plan_tsf_cache_restore` の「同じプロセスで押された、記録より新しい明示 OFF があれば復元しない」
  (`crates/awase-windows-plans/src/focus/tsf_cache_restore.rs:80`)を、「主張は時刻の新しいほうが勝つ」の一般則に置き換える。**前面の取得が押下時点で正しいこと**が前提で、
  ここが崩れると BUG-218 が戻る(弱点。§5 の拡張で列を作る)。

### 4.6 不確実なときの既定(D5、O2)

スコープの開閉の主張が無いとき(初めての窓、行を捨てた窓、パニックリセットの後)、次の順に決める:

1. **実測の事前**(`ClaimBasis::MeasuredPrior`): 測定で裏付けた規則に当たれば、それを主張にする。今ある規則は 1 つ: 「awase の起動後に作られたスレッドの IME は 閉 × ひらがな で始まる」
   (`crates/awase-windows/src/focus/thread_scope.rs` の doc、`191-gji-state-scope-spec.md` §2.1・§3.2。条件は今の `assume_closed_for_new_thread` と同じ:
   `SPI_GETTHREADLOCALINPUTSETTINGS == 0` かつ GJI または同定済み MS-IME 本体。同定済み MS-IME 本体 × 新スレッドの経路は CI で未検証と同 §3.2 が書いている)。
   これは「分からない」ではなく測った事実なので、O2 の ON 寄りより先に使う(**所有者の確認が要る: U2**)。
2. **読める窓**: フォーカス直後の同期の読み(今の `FocusProbe`、Low・BeliefOnly)と非同期の `ImmCrossProbe` が届けば、それを主張にする(主張なしのスコープには Low でも入る)。
   読みが失敗したとき(読める窓でも空振りはある)は 3 へ。
3. **読めない窓、または 2 で読めなかった窓**: 開閉 = ON、モード = かな(`ClaimBasis::UncertainDefault`)。O2 の「ON 寄り」。

モードの主張が無いときは、開閉の主張が ON なら「かな」(今の `eisu_reset_on_ime_on` と同じ考え。IME を開くと、新しいスレッドはひらがな、MS-IME 本体は閉→開でひらがなに戻る。
`191-gji-state-scope-spec.md` §3.3)。GJI の同じスレッド内の閉→開は直前のモードを保持するので、スコープの行に追跡中のモード(`key_track.conv`)があればそれを使う(同 §3.4)。

**既定で倒した ON の失敗のモード(隠さない)**:
- 読めない窓で、実 IME は OFF(利用者が英字を打つつもり)なのに ON に倒すと、生の英字が NICOLA の仮名のローマ字になる(I2 違反。BUG-110 の向き)。
  利用者が OFF キーを 1 回押せば、そのスコープに OFF の明示操作が残り、以後そのスレッドでは戻らない(I3)。今は前窓の `desired_open` を持ち越すので、
  「前の窓で OFF にしていれば新しい窓も OFF」だったのが、新設計では「スレッドごとに 1 回」になる。英字を主に打つ利用者には今より手間が増えうる。
- ON に倒した窓で Engine が活性になるので、ADR-262(観測の無い窓で最初の同時打鍵を契機に IME ON を 1 回送る)と ADR-263(エピソード先頭の保持と言語判定)が今より多く働く。
  ADR-262 は実 IME が OFF だったときの救済になる一方、ADR-263/280 の誤判定(BUG-220・222、ADR-280 の事故)の機会も増える。
- 既定は授権に入れない(D8)ので、既定の ON を根拠に IME へ書き込むことは無い。

### 4.7 Engine の活性の導出(D7、O1)

`derive` は今の `Engine::compute_state(ctx)`(`src/engine/engine.rs:269`)の 4 条件をそのまま使い、`ctx` の入力を新しい主張から作る:

| `compute_state` の条件 | 今の入力 | 新しい入力 |
|---|---|---|
| `adapter.is_enabled()`(`UserDisabled`) | 利用者の設定 | 同じ |
| `ctx.is_japanese_ime`(`NotJapaneseIme`) | `ImeBelief::is_japanese_ime()` | 同じ(全体の状態) |
| `ctx.ime_on`(`ImeOff`) | `ImeStateHub::effective_open()`(`IntentStore` → `resolve_open_at` の 6 段) | フォーカス中のスコープの `open` の主張(無ければ §4.6 の結果) |
| `ctx.input_mode.is_romaji_capable()`(`NotRomajiInput`) | `ImeStateHub::input_mode()`(グローバル) | フォーカス中のスコープの `mode` の主張を `InputModeState` に写したもの。`Kana` → `AssumedRomaji`/`ObservedRomaji`(根拠で選ぶ)、`Unknown` は §4.6 の方針でかなとして写す(読めない窓)、または `Unknown` のまま(読める窓で、読みが来るまで) |

I1 の「かな」は O1 のとおりひらがな・カタカナで、`ModeClass::Kana`。Engine 側(`src/engine/`)は変えない。

### 4.8 今の構造との対応表

| 今の仕組み | 新設計での行き先 | 消える/残る |
|---|---|---|
| `ImeModel::desired_open`(グローバル・永続・初期値 true) | 意味ごとに分解: 明示意図の値 → `ScopeBelief::last_op`、観測ゼロの既定 → §4.6、drift の比較 → `last_op`(`Written` のみ)、窓切替の持ち越し → **無し**、placeholder → 主張なし、授権の入力 → `last_op`(`Written` のみ、D8)、TSF 復元の入力 → 無し(復元そのものが無くなる)、共有メモリの判断(`decide_use`)→ スコープの主張(**要確認**: `platform_state.rs:1429,2148` の読み方を実装前に読む) | **消える** |
| `desired_is_placeholder`・`align_placeholder_desired`(BUG-163) | 主張なしのスコープに最初の観測が入ること(§4.6-2)で同じ効果 | 消える |
| `ImeModel::last_intent` | `ScopeBelief::last_op`(スコープごと、窓切替で消えない) | 統合 |
| `ImeStateHub::intent_store`(hwnd ごと、ON 10 秒/OFF 30 秒) | `ScopeBelief::last_op` と主張。**TTL は無くなる**(`EXPLICIT_ON/OFF_INTENT_TTL_MS` は `tuning.rs:363,397` のコメントで「未実測・暫定値」) | 消える |
| `last_user_explicit_off_{ms,pid}`(`persistent_explicit_off`、OFF でだけ立つ) | 明示操作の `at` と押下時点の前面の pid。ON・OFF 対称 | 消える(一般則に吸収) |
| `ImeModel::key_effect.open`(予測、観測の照合・意図・窓切替で消える) | `ExplicitOp{certainty: Predicted}` の値。照合(settle 後の観測で確認・訂正)は §4.4 の受け入れ判定 | 統合。窓切替では消えない |
| `ImeModel::key_effect.mode`・`input_mode` | スコープの `mode` の主張 | 統合 |
| `key_track`(追跡状態、窓切替で既定へ) | スコープの行に持つ | 残る(置き場所が変わる) |
| `ObservationStore`(`derive_any`・`most_recent_trusted`・`indicator_reading`) | 観測プールは残る(受け入れ判定の材料)。`most_recent_trusted`・`DesiredFallback` の後退の段は不要(主張がラッチなので) | プールは残る。フォールバックの段は消える |
| `resolve_open_at` の 6 段 + `ForceGuard` | 主張の参照 1 つ。`ForceGuard::PanicReset` は、パニックリセットを `OpOrigin::PanicReset` の明示操作(ON)として現スコープに入れ、他のスコープを主張なしにする形へ | 段は消える。パニックリセットは残る |
| `resolve_effective`(`IntentStore` が丸ごと上書き、`platform_state.rs:2186`) | 不要(1 層) | 消える |
| `on_focus_process_changed` の窓種ごとの後処理(`focus_tracking.rs:537`〜) | スコープの切り替え(§4.5)+ 新しいスコープの既定(§4.6) | 大幅に縮む |
| `plan_tsf_cache_restore`(BUG-128・218)・`apply_hwnd_cache_restore`・`HwndImeSnapshot` | スコープ表 | 消える(BUG-128・218 の規則は §4.5 の一般則で守る) |
| `HeuristicDefault` ON(`reset_stale_ime_on_for_imm_broken`)/ OFF(`assume_closed_for_new_thread`) | ON → `UncertainDefault`、OFF → `MeasuredPrior` | 観測として書くのをやめる(観測でないので。`.claude/rules/ime-belief-architecture.md` の「観測を偽装しない」とも合う) |
| `ModeKeyPassedThrough{align_desired}`(ADR-187、BUG-157) | `desired_open` が無いので揃える必要が無い。通過マーク(300ms)は「settle 後の読み直しを予約する」理由として残す。通過後の観測は §4.4 で `Predicted` の主張を覆す | イベントの役割が縮む |
| `reschedule_ime_refresh` の「明示意図があれば止める」(`runtime/mod.rs:1196,1217`) | D6: 止めない。typing-idle のガード(`ir_decide_read_strategy`)は残す | 消える |
| drift correction(`evaluate_drift`) | 入力を `last_op`(`Written`)と受け入れ可能な観測に差し替える。方針(OFF の再試行だけ残す等)は ADR-212 P6 のまま | 残る |
| `OpenWarrant`(`issue_open_warrant`、`OwnSsot` が `desired_open` を読む) | `OwnSsot` の入力を `last_op`(`Written`)だけにし、既定・観測は入れない(D8) | 残る(入力が変わる) |
| `ImeEvent` の列挙値(`HwndCacheRestored`・`KeyEffectPredicted`・`UserImeSetIntent` など) | 古い journal・不具合報告の再生のため、列挙値は残す(棚卸し 03-constraints C-e6、ADR-082)。新しい事象は追加する | 残る(本番では作らなくなる値がある) |
| `PressLedger`・押下 ID(ADR-208) | `ExplicitOp::press` に持つ。二重書き込みの防止は今の actuation 側のまま | 残る |

### 4.9 授権との関係(D8、ADR-087 P11)

ADR-087 の原則「belief(誤りが可逆)と書き込みの授権(誤りが不可逆)は同じ bool を共有しない」を守る。D5 の既定(ON 寄り)は belief を緩くするための値で、
授権に入れると「読めない窓の既定の ON を根拠に IME を開く」書き込みが生まれる(BUG-063 型)。よって `issue_open_warrant` に渡す awase 自身の意図(今の `desired_open`、
`open_warrant.rs:121-122` の `WarrantContext::desired_open`)は、`last_op` が `Written` のときの値だけにし、それ以外は「意図なし」にする。
今の `OwnSsot` が `desired_open` の既定(初期値 true・持ち越し)を採用している場面(`open_warrant.rs:224`、BUG-16 非退行のテスト `:551`)では、新設計で授権の結果が変わりうる。
**この差は実装前に `open_warrant.rs` のテストを全部読み、どの書き込みが増減するかを列挙する**(未確認)。

### 4.10 半角英数トグル・戻り待ち(ADR-107・244・245)

- **表現**: トグルに入ることは、モード軸の明示操作(`OpOrigin::HalfWidthToggle` または `LangDetect`、値 `Eisu`、`Written`)。出ることは、値 `Kana` の明示操作。
  これで「Engine の ctx の `input_mode`」と「gate の `toggle_held`」の二重(X8)のうち、belief 側はモード軸の主張 1 つになる。
- **出入りの条件は今の規則のまま変えない**。理由: 取り下げ #6(`024ca336` の「キー押下を根拠にトグルを手放す」は、実 Chrome × MS-IME 本体で `か` のまま、
  GJI の MS-IME プリセットの Shift タップ後の無変換で `ro`、revert `6b1e91b8`)。新設計が F2 などを明示操作として扱っても、**トグル中の打鍵の予測は今どおり捨てる**(ADR-244 M-2)。
  トグルを手放す条件は、`half_width_alnum.rs` の判断(`plan_leave`・`plan_key_stage`・`exit_owns_write`)と、BUG-222 の修正(未マージ。lang-detect が入れたトグルに限り
  tsf-native × GJI 系で素の F2 の KeyDown で手放す)の範囲に閉じる。新設計はその結果をモード軸の主張として受け取るだけ。
- **窓切替**: ADR-245 は「離脱時に IME へ送らず戻り待ちに積み、belief は離脱時に従来どおり戻す」。新設計では belief はスコープごとなので、離脱したスコープの行は `Eisu` と戻り待ちを持ったまま残り、
  新しいスコープは自分の主張を使う(「belief を戻す」操作が要らなくなる)。戻った窓の最初の打鍵での復元(ADR-245)は今のまま書き、`Kana` の明示操作として記録する。
  BUG-220 の取り合い(戻り待ちと lang-detect の入り)は `plan_rebuild_from_return_for_language_detect` のまま。
- **C-b3**(キャッシュの `ObservedEisu` が Engine を塞ぎ続けない、今の `cache_restore_eisu_guard`): スコープの行の `Eisu` は、戻り待ちが持っているか、明示操作か観測で決まったときだけ残す。
  根拠が古い観測だけの `Eisu` は、戻ったときに `Kana`(仮定)へ落とす(今の規則と同じ効果)。ハーネスで列を作って確かめる。

### 4.11 lang-detect(ADR-263・280)・ADR-262・ADR-279 との接続

- lang-detect は今どおり「belief を書き換えるのではなく、IME に書いて belief を追随させる」(ひらがなキーを 1 回送る)。新設計では、送ったひらがなキーを
  `ExplicitOp{origin: LangDetect, certainty: Written}`(開閉 ON・モード かな)として記録する。英数の判定で半角英数トグルに入るのも `origin: LangDetect` のモード軸の明示操作。
  **「最後の明示操作が勝つ」が lang-detect の誤判定にも効いてしまう**(誤った入りが利用者の操作と同じ重さで残る)。誤判定の後は利用者の次の明示操作で覆るが、
  利用者の操作より弱くする案(`origin` で重さを分ける)は U8 で所有者に確認する。
- lang-detect の保持の発火条件「観測の無い窓で belief が ON のとき」の belief は、フォーカス中のスコープの主張になる。§4.6 の既定で ON になる窓が増えると、発火も増える(§4.6 の弱点)。
- ADR-262(観測の無い窓で最初の同時打鍵を契機に IME ON を 1 回送る)は「belief・IntentStore は動かさない」(ADR-262 の summary)。新設計では IntentStore が無いので、
  「主張を動かさない」と読み替える。送った ON を明示操作として記録するかは、ADR-262 の「belief を動かさない」と食い違うので、今は記録しない(今の挙動を保つ)。
- ADR-279(ずれが続く窓で打鍵列から日本語と判定したら残った明示 OFF を取り下げる、未実装)は、新設計の受け入れ判定に「打鍵列による反証」という源を 1 つ足す形で載る。
  本 ADR では実装しない(ADR-279 の所有者判断を待つ)。

### 4.12 ポーリング(D6)と ADR-191 決定 3 の関係

ADR-191 決定 3 は、打鍵時の予測を `UserImeSetIntent` にしなかった理由を 2 つ書いている(`docs/adr/191-ime-is-source-of-truth-observe-not-write.md` の「実装での単純化(round4 QB1・QB2)」):
(1) `UserImeSetIntent` だと明示意図でポーリングが止まる問題(BUG-151 原因③)を全モードキーへ広げる、(2) `desired_open` への新イベントだと `resolve_open_at` で観測に負ける。

新設計は F2 などを明示操作として扱う(O3)が、2 つとも次の仕組みで避ける:

- (1) への対処: **ポーリングの予約を明示意図から切り離す**。`decide` が返す `ScheduleRead` は「読める窓か」「打鍵中でないか(typing-idle のガード、今の `ir_decide_read_strategy`)」
  「直前の明示操作・通過マークの settle 後か」だけで決まり、`last_op` の有無を見ない。BUG-151 の型(明示意図が残る → `reschedule_ime_refresh` が早期 return → 観測が来ない)は、
  早期 return そのものが無くなるので起きない。ADR-279 段階 0 の「ずれの間の観測 0 件」も、読める窓では解消する見込み(読めない窓は観測の手段が無いので変わらない)。
  **未確認の副作用**: 今の早期 return が、打鍵中のクロスプロセス読み取りによる「@」(ADR-191 の「やらないこと」が `explicit_verify` の `applied != Unknown` の削除について書いている、
  BUG-113 系)を防ぐ役割を兼ねているか。typing-idle のガードだけで足りるかは、実装前に `ir_decide_read_strategy` と BUG-113 の記録を読み、実機 CI(`sc-*` の GJI × Windows Terminal)で確かめる。
- (2) への対処: 明示操作は主張としてラッチされ、覆すのは settle 後の受け入れ可能な観測だけ(§4.4)。観測と同じ土俵の「段」にいないので、古い観測・鮮度の窓の内側の観測に負けない。
  逆に settle 後の信頼できる観測には負ける(ADR-191 決定 3「観測は確認と訂正」と同じ)。
- 予測を観測なしで連鎖させる(開ループ)のは読めない窓だけ、という ADR-191 決定 3 の注意はそのまま当てはまる(読める窓では毎回照合する)。

## 5. 検証計画

### 5.1 基準の測定(PR #857)との関係

- 基準の測定のオラクル(`belief_engine_invariant.rs`)を、**新設計の純粋な核にも当てられるようにする**: ハーネスの事象列を、旧(本物の `ImeStateHub`)と新(`BeliefStore` の `evolve`/`derive`)の
  両方へ流し、同じ真の状態に対する I1・I2 の違反を並べて数える。golden に新の列を足す。
- 目標: 新の I1 違反が「awase の操作・窓切替・正しい観測だけ」の列で 0(msime shared・全字母・長さ 3 の 36 本の最小列、小さい字母の 4 本を含む)。BUG-223 の 4 本
  (`bug223_sequences`)が新では違反でないこと。I2 は「既定の区分」以外で今以下。外部変更・嘘の観測を含む列は件数を並べて報告する(0 は目標にしない。§2.1)。
- ハーネスのスレッドの扱いが結果を左右する: 新設計の鍵はスレッドなので、shared(全窓が同じスレッド)では全窓が 1 つのスコープになり、per-thread では窓ごとに別のスコープになる。
  **基準の測定の主の設定(shared)は、CI 実測(保持単位はスレッド)と合わない可能性がある**(基準の測定は BUG-223 の記述から shared を主にした)。新設計の評価は per-thread を主にし、
  shared は感度の確認として並べる。

### 5.2 ハーネスの拡張(擬似窓が写していないもの)

| 写していないもの | 新設計での必要性 | 扱い |
|---|---|---|
| `on_focus_process_changed` の hwnd キャッシュ復元・TSF の ON キャッシュ復元・新スレッドの閉の仮定 | 新設計では消える処理。旧との比較で、旧が「戻る列」で救われる分(基準の測定の偽陽性の疑い)を正しく数えるために写す | **ハーネスの拡張が要る**(旧の比較用)。新は写す必要が無い |
| per-thread で「awase の起動前からあるスレッド」(真の初期状態が開/閉どちらもありうる) | §4.6 の既定の区分の件数を数えるのに要る | **ハーネスの拡張が要る**(窓の初期状態を開・閉の両方で列挙) |
| 押下時点の前面とフォーカスの追跡のずれ(BUG-218) | §4.3-1 の帰属の規則を検査するのに要る | **ハーネスの拡張が要る**(`focus_to` の遅延) |
| 通過マーク(ADR-187)・`ImeApplyRequested`/`applied` の往復・ForceGuard | 通過後の観測が `Predicted` を覆す列、パニックリセット | ハーネスの拡張(通過マーク・パニックリセット)。`applied` の往復は actuation 側なので実機 CI |
| lang-detect の保持・半角英数トグル・戻り待ち | §4.10・§4.11。BUG-186・193・220・222 の列 | ハーネスの拡張が要る。取り下げ #6 の失敗の列(実 Chrome × MS-IME 本体、GJI の MS-IME プリセットの Shift タップ後の無変換)を必ず入れる |
| 周期の poll・settle 後の読み直し(D6) | 読める窓で I1 が治るまでの時間 | 事象として足す(基準の測定の「優先 (3)」) |
| 読めない窓の IMM の嘘(BUG-223(a)) | 信頼表の「反証不可」を検査する | 字母に足す(基準の測定の「優先 (2)」) |
| `user_enabled` の切り替え | I8 | 事象として足す |
| `Engine::on_input`(打鍵の出力)・flush | I9 | Engine の単体テストで守る(ハーネスには入れない) |
| TSF の composition・cold/warm・実 IME の遅れ | 新設計は扱わない | 実機 CI(`sc-*`)と所有者の実機 |

### 5.3 その他の検査

- **参照モデルとの差分**: 新の核の仕様を、§4.4 の表だけから書いた小さな参照モデル(テスト内、数十行)と、ランダム列で突き合わせる(実装が表と食い違う箇所を見つける)。
- **I4 の変成関係**(§2): ランダム探索で回す。
- **突然変異テスト**: `cargo mutants` を新の核(`awase-ime-core` の新モジュール)に当て、受け入れ判定の各条件(柵の不等号・信頼表のセル・`Written`/`Predicted` の分岐)が
  オラクルか単体テストで殺されることを確かめる(`git diff develop | cargo mutants --in-diff -`)。
- **台帳の回帰列**: §6 の台帳で「引き継ぐ」「代替で守る」にした各規則について、その事故の最小列をハーネスに足し、新で違反しないこと。

### 5.4 段階 V0: 純粋な核を擬似 IME の上でだけ動かす(本番は触らない)

1. `crates/awase-ime-core` に新の核(`BeliefStore`・`evolve`・`decide`・`derive`・`TrustTable`)を足す。本番からは呼ばない。
2. ハーネスに新の核のアダプタを足し、§5.1 の旧新比較を CI の `test` ジョブで回す。
3. **終わりの判定**: I1(核の列)0、BUG-223 の 4 本が新で違反なし、I3〜I7 の検査が全部通る、台帳の回帰列が全部通る、§5.2 の拡張のうち「必要」の行がそろっている。
   所有者が §2.1 の残る違反(既定の区分・外部変更)の件数を見て受け入れる。
4. 戻し方: 本番に配線していないので、モジュールを消すだけ。

## 6. 台帳との照合(棚卸し 02-ledger の各規則)

「引き継ぐ」= 今の規則・実装をそのまま使う。「代替で守る」= 規則の目的を新設計の別の仕組みで守る。「不要」= 新設計では起きる経路が無い(理由つき)。

| 出典 | 守っている規則 | 新設計 | 理由・どこで |
|---|---|---|---|
| BUG-46・052・116、ADR-191 | 物理 IME キーの配送(awase が書くトグルの KeyDown だけ Suppress、0xF0〜0xF2 は Allow) | 引き継ぐ | 配送(`physical_disposition.rs::plan_core`)は belief の外 |
| BUG-110 | 明示意図が無い間は HeuristicDefault だけを根拠に drift correction を出さない | 代替で守る | drift の対象は `Written` の明示操作だけ(§4.4)。既定・実測の事前は観測でも意図でもないので drift の根拠にならない |
| BUG-110 追補1(ADR-279) | (未実装)残った明示 OFF を打鍵列で取り下げる | 接続のみ | §4.11。本 ADR では足さない |
| BUG-113 | `shadow_on` を `Option<bool>` のまま扱う | 引き継ぐ | actuation 側。`applied` は変えない |
| BUG-128 | TSF 窓への ON 復元は hwnd 一致を要求 | 代替で守る | 粗い鍵で記録を引かない。鍵は(pid, tid)(§4.5)。explorer の UWP 入力面のスレッド構成は未確認(U4) |
| BUG-151 | 通過したモードキー後は `applied` に依らず読み直す / 明示意図でポーリングが止まらない | 引き継ぐ+代替 | `explicit_verify`・`mode_key_pass_live` は残す。ポーリングの停止は D6 で無くす |
| BUG-157 | 通過キーの後、観測から `desired_open` を揃える | 不要 | `desired_open` が無い。通過後の観測は §4.4 で `Predicted` を覆す。drift の対象は `Written` だけなので、F2 の通過を drift が閉じ直す経路が無い |
| BUG-163 | placeholder の `desired_open` を最初の観測に揃える | 代替で守る | 主張なしのスコープに最初の観測が入る(§4.6-2) |
| BUG-186 | 直接観測を GJI/MS-IME 本体へ広げ、NATIVE の読みに追随したらトグルを書き込みなしで手放す | 引き継ぐ | §4.10。手放しの条件は変えない |
| BUG-189・ADR-233 | 新しい Medium が古い High に勝つ(信頼度優先にしない) | 引き継ぐ+条件つき | 読める窓では新しいほうが勝つ。BUG-219 の型(High の後の Medium 単独)は U7 |
| BUG-193・ADR-245 | 離脱時に書かず、戻った窓の最初の打鍵で復元 | 引き継ぐ | §4.10。belief を離脱時に戻す操作は不要になる |
| BUG-194・200・207 | raw TSF リテラル回収の宛先照合・Mozc 時の監視外し | 引き継ぐ | 出力・回収の層。BUG-200(belief ON × 実 IME OFF で最初の打鍵が消える)は §4.6 の既定 ON で機会が増えうる(弱点) |
| BUG-205・211・212、ADR-281 | 予測器の段階規則・表 | 引き継ぐ | 表の予測は明示操作の値になる。外れが窓切替をまたいで残る分、影響が長くなる(§2.1) |
| BUG-218 | 入場先と同じプロセスで押された、記録より新しい明示 OFF があれば復元しない | 代替で守る | 押下時点の前面のスコープに記録し、新しいほうが勝つ(§4.3-1・§4.5)。対称(ON にも効く) |
| BUG-219 | Mozc 系を `[direct-follow]` から外す | 代替で守る | 信頼表で GJI 系の読めない窓の IMM を反証不可(§4.4) |
| BUG-220 | 戻り待ちのある窓で英数と判定したら戻り待ちから立て直す | 引き継ぐ | §4.10 |
| BUG-222(未マージ) | lang-detect が入れたトグルは、tsf-native × GJI 系で素の F2 で手放す | 引き継ぐ(マージされた形で) | §4.10。範囲を広げない |
| BUG-223 | (修正なし) | 新設計の目標 | F2 の ON は明示操作として窓切替で消えない(D3)、グローバルな持ち越しが無い(D1)、読めない窓の IMM は反証不可(D4) |
| C-b3(2026-07-09 MS Edge) | キャッシュの `ObservedEisu` が Engine を塞ぎ続けない | 代替で守る | §4.10 の最後 |
| C-b4(`architecture_guard::user_ime_on_paths_are_paired_with_eisu_reset`) | IME-ON の経路に `ObservedEisu` の救済を対で置く | 代替で守る | 開閉 ON の主張でモードが無い・古いときは かな(§4.6)。ガードテストは新の経路に合わせて書き直す |
| ADR-087・089 | belief と授権の分離、観測プールの Actuating/BeliefOnly | 引き継ぐ | D8・§4.9。プールは残す |
| ADR-098 | `applied` は actuation を試みた経路だけが書く | 引き継ぐ | `applied` は belief の主張と別。新設計は `applied` を書かない |
| ADR-106 | フォーカスの epoch+hwnd の柵 | 引き継ぐ | §4.1・§4.5 |
| ADR-165 | 時間だけで復元を抑制しない(案 A 却下) | 引き継ぐ | 新設計に「時間だけの抑制」は無い(取り下げ #5) |
| ADR-179・212・213 | 予防的・補正的な書き込みをしない(force-on・reassert・ActivationSync 撤去、`kp_shadow_actuate` に一本化) | 引き継ぐ | D9。`decide` は書き込みを返さない |
| ADR-187 | 通過後に読み直し、古い明示意図を捨てて観測に追随 | 代替で守る | 通過マークは読み直しの予約に残り、通過後の観測が主張を覆す(§4.4)。「意図を捨てる」操作は不要(新しい観測が勝つ) |
| ADR-188・244・205・268 | 外から変わったことへの追随(直接観測・監視窓・共有メモリ) | 代替で守る | 信頼表の「反証可」の源として受け入れ判定の 1 つの入口から入る(X5 の尾部の重複が消える) |
| ADR-191 決定 1・3 | IME が状態の正。表は予測。予測は打鍵時に反映し、観測で確認・訂正 | 引き継ぐ | §4.12 |
| ADR-199 | 能動に書くのはトグルの役割のキーと `keys.ime_*` だけ | 引き継ぐ | D9。`Written`/`Predicted` の区別はこの線引きと同じ |
| ADR-208 | 押下 ID、明示キーは最大 2 回で一致 | 引き継ぐ | actuation 側。`ExplicitOp::press` に ID を持つ |
| ADR-214 | (保留)Sent/Confirmed の分離 | 関係なし | `applied` を変えない |
| ADR-255 | IME OFF のときだけ無変換/変換を Space に。判定は Engine 自身の belief | 引き継ぐ | belief の出所がスコープの主張になるだけ |
| ADR-260 | 読める窓では開状態の英数トグルの入力モードを予測しない(B1o) | 引き継ぐ | 予測器側。予測しなかった打鍵はモード軸の明示操作にならない |
| ADR-262・263・280 | 観測の無い窓の補正・保持・言語判定 | 引き継ぐ | §4.11。既定 ON で発火が増える(弱点) |
| ADR-273 | 字形の柵 1300ms、Medium が High を追い越さない、読み手の除外 | 引き継ぐ | 柵は §4.4、字形は信頼表の設定どおり |
| 定数(棚卸し 02-ledger §D) | `OBSERVATION_FRESH_WINDOW_MS`=3000・`KEY_EFFECT_SETTLE_MS`=170・`GLYPH_KEY_FENCE_MS`=1300・`MODE_KEY_PASS_MARK_WINDOW_MS`=300・`DRIFT_CORRECTION_OBS_MAX_AGE_MS`=1500・`MOZC_SHM_OBSERVATION_TTL_MS`=1000 | 引き継ぐ | 値は変えない。3000 は「Medium 合意の材料の鮮度」として残り、主張の寿命には使わない。`EXPLICIT_ON/OFF_INTENT_TTL_MS`(10000/30000、未実測・暫定値)と `EXPLICIT_OFF_CACHE_SUPPRESS_MS`(10000)は不要になる。`MIN_FOCUS_DURATION_MS`(100)は belief では不要(他の読み手は未確認) |

## 7. 取り下げた方式との照合(棚卸し 03-constraints (d) の 9 件)

| # | 取り下げた方式 | 新設計は繰り返していないか |
|---|---|---|
| 1 | awase が観測・belief に合わせて IME へ書き直す補正(force-on・reassert・ActivationSync・focus 時の強制 OFF・drift の拡張) | 繰り返さない。`decide` は IME への書き込みを返さない(D9)。既定の ON は授権に入らない(D8)ので、既定を根拠に IME を開く書き込みは生まれない。drift correction は ADR-212 P6 (a) の範囲のまま |
| 2 | 観測が無いことを根拠に意図を偽装する(`Recovery`/`HwndCache` の `UserIntentSource`、`InputModeObserved{ImmGetOpenStatus}`) | 繰り返さない。既定は `ClaimBasis::UncertainDefault`、実測の事前は `MeasuredPrior` という**自分の名前の根拠**で持ち、明示操作とも観測とも名乗らない。明示操作で覆り、受け入れ可能な観測 1 つで覆る(§4.4)。今の `HeuristicDefault` を「観測」として書く形もやめる |
| 3 | 「この窓種には日本語 IME が無い」という推測を観測として書く(experiments エントリ 02) | 繰り返さない。窓の種類から作るのは既定(ON 寄り)だけで、観測としては書かない。向きも逆(OFF を作らない) |
| 4 | 信頼度優先で古い High を残す(BUG-189) | 繰り返さない。読める窓では新しいほうが勝つ(ADR-233 A')。U7 の「High の後の Medium 単独は 2 つそろうまで待つ」は、信頼度優先ではなく「単独の Medium で反証しない」規則で、BUG-189 の窓(Medium が続けて届く)では 2 つ目で覆る。BUG-189 の列をハーネスに入れて確かめる |
| 5 | 時間だけで「OFF の後の復元」を抑制する(ADR-165 の案 A) | 繰り返さない。時間の窓で抑制する規則は無い。勝ち負けは「同じスコープの中の証拠の時刻の新しさ」で決める |
| 6 | キー押下を根拠にした半角英数トグルの手放し(`024ca336`) | 繰り返さない。トグル中の打鍵の予測は今どおり捨て、手放しの条件は `half_width_alnum.rs` と BUG-222 の範囲のまま(§4.10)。**F2 を明示操作として扱うことと、トグルを手放すことを結び付けない** |
| 7 | IME OFF に送るキーを動かす(experiments エントリ 01) | 繰り返さない。送るキーの選択(`ime_controller.rs`・`vk_send.rs`)は触らない |
| 8 | 実測なしの暫定定数を増やす | 繰り返さない。新しい時間の定数は足さない。未実測の `EXPLICIT_*_INTENT_TTL_MS` は消える。スコープ表の行の寿命は今の `HWND_CACHE_MAX_AGE_MS` を流用する(新しい値を作らない) |
| 9 | 窓切替で「ON へ戻す」方向の修正は BUG-128・218 を再発させうる | 新設計は「ON へ戻す」のではなく「スコープごとの主張を使う」。BUG-128 は粗い鍵を使わないこと、BUG-218 は押下時点の前面への帰属と時刻の比較で守る(§4.5)。**ただし既定の ON(§4.6)は、記録の無い読めない窓では実際に ON へ倒す**。これは O2 の判断で、既定は授権に入らない(BUG-128 の「force-ON まで発火」は起きない)が、Engine だけ ON になる向きの誤り(BUG-218 の「zoom → zotumo」と同じ向き)は既定の区分として残る(§2.1) |

## 8. 移行

| 段階 | 内容 | 終わりの判定 | 戻し方 |
|---|---|---|---|
| V0 | §5.4。純粋な核を擬似 IME の上でだけ動かす | §5.4-3 | 新モジュールを消す |
| V1(shadow) | 本番で旧と新を並べて計算する。新は Engine にも授権にも効かせない。旧と新の Engine の活性・開閉・モードが食い違ったら journal に `[belief-v2]` として根拠(`ClaimBasis`)と旧の `decided_by` を残す。殻は新の `evolve` に同じ事実を渡す(スコープの鍵の取得、押下時点の前面の取得を足す) | 所有者の dragonflyg4(GJI)と CI の `sc-*` で食い違いを集め、食い違いを分類する(旧が誤り/新が誤り/真が分からない)。分類は後続の観測・利用者の操作で判断する。「新が誤り」の分類が既定の区分(§2.1)以外で 0、かつ BUG-223 型の区間で新が正しい側にいる。件数の基準は所有者が決める | shadow をやめる(新の呼び出しを外す)。本番の挙動は変わっていない |
| V2(切替) | Engine の ctx・drift correction・授権・lang-detect の入力を新へ切り替える。切り替えは隠し設定(例: `belief_engine = "v2"`)で、既定は所有者の判断で切り替える | 所有者の実機で一定期間(所有者が決める)、BUG-223 型(IME ON で Engine OFF)の報告が無く、§2.1 の既定の区分による誤 ON の報告が許容範囲。CI の `sc-*`・e2e が今と同じか良い | 設定を旧へ戻す(旧のコードは残っている) |
| V3(撤去) | 旧の仕組み(§4.8 で「消える」の行)を撤去する。`ImeEvent` の列挙値は journal の再生のため残す。`architecture_guard`・dylint(`ime_event_guard` の対象 variant)を新の構造に合わせる | 撤去後に CI 全部・基準の測定の golden・台帳の回帰列が通る | revert(このとき `.claude/rules/experiment-logging.md` に従い、失敗の条件を本文に書く) |

- 実機の確認は所有者の GJI 環境(dragonflyg4)で行う。本 ADR の起草では実機・clipwire は使っていない。
- 再発ファミリー(IME belief・focus 遷移・conv mode・半角英数トグル)に触れるので、各段階の PR は `.claude/rules/fix-requires-evidence.md` の (a)(回帰テスト)を満たす(基準の測定の golden と台帳の回帰列)。

### 8.1 実装の見積もり(粗い)

| 場所 | 変更 | 規模の見当(根拠のない目安) |
|---|---|---|
| `crates/awase-ime-core/src/state/`(新モジュール、例 `scope_belief.rs`・`trust_table.rs`) | 核(V0) | 新規 600〜1,000 行 + 単体テスト |
| `crates/awase-windows/tests/support/harness.rs`・`belief_engine_invariant.rs`・golden | 新の核のアダプタ、§5.2 の拡張、旧新比較 | 数百行 |
| `crates/awase-windows/src/state/platform_state.rs`(`ImeStateHub`) | V1 で新を並走、V2 で読み口を切替、V3 で `intent_store`・`resolve_effective`・`persistent_explicit_off`・`reset_stale`・`assume_closed`・`apply_hwnd_cache_restore` を撤去 | 大(このファイルは ADR-275 で殻に残すと決めたもの) |
| `crates/awase-windows/src/runtime/focus_tracking.rs`(`on_focus_process_changed`) | スコープの鍵の取得、後処理の縮小 | 中〜大 |
| `crates/awase-windows-plans/src/focus/tsf_cache_restore.rs`・`focus/hwnd_cache.rs` | V3 で撤去 | 撤去 |
| `crates/awase-windows/src/runtime/mod.rs`(`reschedule_ime_refresh`) | D6(意図による停止の撤去) | 小(ただし実機 CI の確認が要る) |
| `crates/awase-windows/src/runtime/key_pipeline.rs` | 予測・shadow toggle・`keys.ime_*` を明示操作として核へ渡す、押下時点の前面を渡す | 中 |
| `crates/awase-ime-core/src/state/ime_model.rs`・`ime_event.rs`・`intent_store.rs`・`open_warrant.rs`・`drift_correction.rs` | 新事象の追加、入力の差し替え、V3 で撤去 | 中〜大 |
| `crates/awase-windows/tests/architecture_guard.rs`、`lints/ime_event_guard` | 書き込み点の固定を新の構造に合わせる | 中 |
| `src/engine/`(ルート crate) | 変えない(`InputContext` の作り方だけが変わる) | 0 |

## 9. 却下した案

再提案しないもの(§7 の 9 件)に加えて、本 ADR で検討して捨てた案:

| 案 | 捨てた理由 |
|---|---|
| A. BUG-223 だけを小さく直す(F2 の予測で `desired_open` を書く、または窓切替で `desired_open` を捨てる) | 前者は「予測を意図扱い」で BUG-151 原因③(意図でポーリングが止まる)を全モードキーへ広げる(ADR-191 決定 3 が避けた形)。後者は持ち越しを消すだけで、消した後の値(既定)を決めないと別の窓で同じ型が出る。どちらも保管場所 5 つ(X1)を残す。所有者の指示がゼロベースの設計なので、局所修正は V0 の比較対象としてだけ使う |
| B. hwnd を鍵にする(今の `IntentStore` を広げる) | CI 実測で IME の保持単位はスレッド。同じスレッドの別 hwnd で変えた後に古い hwnd の記録が戻る食い違い(`191-gji-state-scope-spec.md` §3.1) |
| C. グローバル 1 つの belief のまま、窓切替で既定へリセットする | O4(窓ごとに独立、その窓の直前の記録を使う)に合わない。戻った窓で利用者の明示 OFF を失う(I3 違反) |
| D. 明示操作に TTL を付ける(今の 10/30 秒を実測して決め直す) | O3(寿命切れで古い値に戻らない)に合わない。今の TTL の doc は「フォーカス断絶(奪取→復帰)のギャップを何秒までカバーするか」と問いを定義し直している(`tuning.rs:378-380`)が、スコープごとに持てば断絶で意図が消えないので、その問い自体が無くなる。TTL が守っていた「誤記録の最悪持続時間を有界にする」効果は失う(§10 リスク 9) |
| E. 不確実なときは常に ON(実測の事前も使わない) | 新しいスレッドは閉で始まる(CI 実測)ので、測った事実を捨てて I2 違反を自分で作る。U2 で所有者に確認する |
| F. 不確実なときは lang-detect の判定を待つ | 判定まで最大 7 打鍵・2.5 秒(ADR-263 の summary)保持する。誤判定の事故(ADR-280)があり、既定の代わりにするには重い。lang-detect は既定の後に働く補正として残す |
| G. 観測を窓ごとに保存して戻ったときに観測として使う | 旧窓の観測を新しい文脈で観測扱いする(C-c7、ADR-106 に反する)。戻ったときに使うのは「主張」(その時点の最終結果)で、観測そのものではない |
| H. `Written` と `Predicted` で覆す条件を変える(`Predicted` だけ観測 1 つで覆る等) | O3(ON・OFF・キーの種類によらず同じ重さ)に合わない。違いは drift と授権の 2 点に限る(§4.4) |

## 10. リスク

1. **スコープの鍵の誤り**(U4): TSF ネイティブのアプリでフォーカス hwnd のスレッドが IME の状態を持つスレッドと違うと、スコープが実際より粗い/細かい。粗いと別の窓の主張が効き(持ち越しの再来)、
   細かいと同じ状態の窓で既定に倒れる。実機 CI で確かめるまで、Chrome・VS Code・WezTerm・Windows Terminal は未確認。
2. **既定 ON の誤り**(§4.6): 読めない窓・記録なし・実測の事前なしで、実 IME が OFF のとき、英字が NICOLA の仮名のローマ字になる。BUG-200(belief ON × 実 IME OFF で TSF 窓の最初の打鍵が
   raw TSF リテラル回収の Backspace で消える)の機会も増えうる。
3. **IME の状態がスレッドをまたいで共有される構成**: CI 実測は `SPI_GETTHREADLOCALINPUTSETTINGS = 0` の windows-latest。利用者の設定・IME で状態が全体共有なら、スコープごとの独立は誤りになる
   (A で OFF にしても B の既定は ON)。実機で確かめる。
4. **押下時点の前面の取得**(BUG-218 の代替): 前面の取得が押下時点からずれると、明示操作が別のスコープに付く。
5. **D6(ポーリングの停止の撤去)の副作用**: 打鍵中のクロスプロセス読み取りの増加と「@」(BUG-113 系)の関係が未確認(§4.12)。
6. **明示操作の重さが lang-detect の誤判定にも乗る**(U8)。
7. **予測の外れが長く残る**(§2.1): 窓切替で偶然直っていた外れが残る。BUG-211・212 は未修正。
8. **移行の規模**: `platform_state.rs`・`focus_tracking.rs` は大きく、ADR-275 で core へ移さないと決めた殻にある。V1 の shadow で殻に事実の取得(tid・前面)を足すと、殻が一時的に太る。
9. **寿命の無い主張は、誤った記録の最悪の持続時間になる**: `EXPLICIT_OFF_INTENT_TTL_MS` の doc(`crates/awase-windows-base/src/tuning.rs:365-397`)は、
   ADR-087 round4 の Opus レビュー(M-A)で「対象ごとに永続する `IntentStore` で OFF を無期限にすると、drift correction が永久に再同期できない固着を作る」と指摘され、
   当初の 1 時間から 30 秒へ縮めた経緯を書いている(「`effective_open()` から読まれると、誤記録・stale 化などのあらゆる失敗モードの最悪持続時間そのものになる」)。
   本 ADR はこの指摘を受けたうえで、O3 の判断に従い主張に寿命を付けない。区別する点は 2 つ:
   (a) **読める窓**では、受け入れ可能な観測が settle 後に主張を覆す(§4.4)。drift correction は `last_op`(`Written`)と観測を比べて ADR-212 P6 (a) の範囲で再送し、
   今の打ち切り(`crates/awase-ime-core/src/state/drift_plan.rs`)の後は観測の主張が残る。Engine は再送の成否を待たず観測に従う(今は明示意図が勝つので、BUG-110 追補1 のように Engine が止まったまま再送が続いた)。
   (b) **読めない窓**では覆す証拠が無いので、誤った帰属(リスク 4)・予測の外れ(リスク 7)・外部変更は、次の明示操作かスコープ表の行の寿命(1 時間)まで残る。**ここは今の
   30 秒より悪くなりうる**(今は 30 秒後に `resolve_open_at` へ落ちる。ただし落ちた先の `DesiredFallback` が正しい保証も無い〈BUG-223〉)。V0 で、読めない窓の外部変更の列で
   「誤りが続いた時間」を旧新で比べ、所有者に示す。
10. **ハーネスの仮定への過適合**: 新の核を擬似 IME に合わせて作ると、擬似 IME が表せないクセ(ADR-277 の QUIRKS のうち未モデル化の 3 件、TSF の composition、cold/warm)で外れる。V1 の shadow の実機データで確かめる。

## 11. 未決(所有者の確認が要ること)

| # | 問い | 本 ADR の提案(決定ではない) |
|---|---|---|
| U1 | 方針の例外に `config.app_overrides.disable_apps`(BUG-78)・`FocusKind::NonText`(experiments エントリ 02)・日本語でない IME(`NotJapaneseIme`)を含めるか | 3 つとも「Engine OFF」の例外に含める(今の挙動と同じ。`disable_apps`・`NonText` は `compute_state` の外にあるので、I1 の検査の対象外として扱う) |
| U2 | 不確実なときの既定で、実測の事前(「awase の起動後に作られたスレッドは閉で始まる」)を O2 の ON 寄りより先に使ってよいか | 使う(測った事実なので)。使わない場合は案 E になり、新しい窓で I2 違反が増える |
| U3 | InputRelay(リモートデスクトップ等)を「読めない窓」として ON 寄りにするか | ON 寄りの対象から外し、今の挙動を保つ(今の挙動の詳細は未確認。実装前に読む) |
| U4 | O4 の「窓ごと」を「IME が状態を持つ単位(スレッド)ごと」と読んでよいか。同じスレッドの別の窓は状態を共有する | スレッドごと(CI 実測)。TSF ネイティブのアプリは実機 CI で確かめてから |
| U5 | JIS かな入力方式(`ObservedKana`)のときの Engine | OFF のまま(NICOLA はローマ字を出すので、かな入力方式では文字が化ける)。O1 の「かな入力の全般」がかな入力方式を含む意味なら、出力方式の変更が要り本 ADR の範囲外 |
| U6 | `compute_state` の外にある入口(`app_disabled`・lang-detect の保持・戻り待ちの保持・`FocusTransition` の settle)を本 ADR で 1 か所にそろえるか(X7) | そろえない(本 ADR は belief の入力だけを変える)。理由の journal の統一は別 ADR |
| U7 | 信頼表の未確定のセル(tsf-native/imm32-unavailable × MS-IME 本体の IMM、直接観測の開閉軸)と、「High の主張の後の Medium 単独」を何で覆すか | V0 の全列挙と実機 CI で決める。Medium 単独は 2 つそろうまで待つ案(BUG-219 の型)を初期値にし、BUG-189 の列で悪化しないことを確かめる |
| U8 | lang-detect・半角英数トグル・パニックリセットなど awase が起こした明示操作を、利用者の操作と同じ重さにするか | 同じ重さ(O3 の対称を優先)。lang-detect の誤判定の害は ADR-280 の候補(C0 等)で小さくする方向。重さを分けると「どちらが勝つか」の規則が 1 つ増える |

## 12. 所有者の判断(2026-10-26、決定)

1. 「かな」の範囲 = ひらがな・カタカナ(かな入力の全般)。半角英数・直接入力は含まない。→ D7・§4.7(JIS かな入力方式の扱いは U5)。
2. 不確実なときは窓の種類で分ける。観測が読めない窓(GJI の tsf-native など)は ON 寄り、読める窓は観測を信じる。→ D5・§4.6(実測の事前を先に使うかは U2)。
3. 最後の明示操作が勝つ(ON も OFF も対称)。利用者の IME-OFF キーも ON キーも同じ重さの明示操作として扱い、窓切替や観測の寿命切れでその効果が消えて古い値に戻らない。
   観測が反証する場合の扱いは設計する。→ D2・D3・D4・§4.4(反証は信頼表と柵で決める)。
4. 窓切替では IME の状態(開閉)を窓ごとに独立させ、持ち越さない。新しい窓で観測が来るまでは、その窓の直前の記録、無ければ不確実の方針。→ D1・§4.5(「窓ごと」の読み方は U4)。

## 13. 基準の測定の結果で補う節

基準の測定(PR #857)は起草時点で OPEN。本 ADR は PR #857 の `docs/tasks/belief-engine-invariant-baseline-2026-10-26.md` と golden(CI run 40275882156 で一致と同文書が記録)の値を引いた。
PR #857 の字母・規模が変わったら、§1.2・§5.1 の数値をその版で引き直す。V0 で新の核を同じオラクルに当てた結果(旧新の違反の表)は、この節に追記する。
