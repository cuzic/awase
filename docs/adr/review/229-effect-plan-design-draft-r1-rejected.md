---
id: ADR-229-companion-229-effect-plan-design-draft-r1
title: |-
  Effect 列(Plan)の設計案 r1(起草。Opus round1 で採らないと判断)
type: companion-doc
related_adr:
  - "ADR-229"
---

> **この案は採らない**(2026-10-06、Opus round1。`229-opus-effect-plan-round1.md`)。何を調べ、なぜ採らないかの記録として残す。

# 「Event とワールドモデルから Effect 列を生成する」設計案 r1(根本的な設計変更を前提)

状態: 起草(2026-10-06)。Opus の評価(複数ラウンド)で収束させてから ADR にする。コードは変えない。
前提: ADR-229(FCIS)・Redux/Elm の調査・W0 の棚卸し(`inventory-w0-*.md`、a/b は作業中)。所有者の要望: 「代数的 Effect・圏論・Haskell の monad も含めて調査して、Event とワールドモデルからどのような Effect 列を生成するかを、根本的な設計変更をする前提で良い案を考える」。

## 0. 先に結論(私の主張。反証してほしい)

1. **Effect 列は「平らな Vec<Cmd>」ではなく、小さな「型つきの項(Plan)」にする。** Plan は、独立な効果の束(applicative)、選択(selective: 分岐は静的に見える)、順序制約(段階 Stage)、資源の括弧(bracket)、失効の再検査(Require)を持つ。**任意のクロージャ(monad の `>>=`)は持たない**。長く続くプロトコル(warmup など)は、Plan を時間をかけて出す別の概念「Process」(状態機械、`StepCoro`)にする。
2. **Plan は解釈(interpret)で動かす**。解釈器を差し替える: 実機(Windows)・再生(純粋)・追跡/説明(dry-run、不具合報告に載せる)・偽物(Linux の擬似 IME)・静的解析(gate と capability の検査)。**Plan 自体が journal の記録の単位**になる。
3. **Event の種類で、生成できる Effect の種類を縛る(graded effect / capability)。** 観測(Observation)の Event は `Write*` を生成できない(ADR-208 の「明示キーだけが書く」を、型と網羅テストで保証)。意図(Intent)の Event だけが書き込みの Plan を生成できる(`Warrant` の型状態、ADR-090 の延長)。
4. **level-triggered(状態の差分から効果を作る)は「情報の収集」にだけ使い、OS への書き込みには使わない。** 観測の要求は「いまのモデルが必要とする観測の集合」として、Model から導く(Elm の `subscriptions`)。IME への書き込みは、edge-triggered(明示意図の Event)のまま。ADR-212 が補正的 actuation を撤去した履歴(IME は自分の状態について嘘をつく)に反しないため。
5. **効果の等式(法則)を、純粋な正規化と性質テスト(proptest)で表す。** 最初は 3 つだけ: タイマーの上書き(`Set(id,d1);Set(id,d2) = Set(id,d2)`)、同一対象への書き込みの吸収(間に観測が無い `SetOpen(x);SetOpen(y) = SetOpen(y)`)、bracket の対(acquire は必ず release と対)。

## 1. 調査の要点(何を借りるか)

| 概念 | 要点 | このプロジェクトでの使い道 |
|---|---|---|
| **代数的 Effect とハンドラ**(Plotkin–Pretnar) | 効果は「操作(operation)の署名 + 等式」で定義し、ハンドラは自由代数からの準同型(解釈)。monad ではなく、操作と等式から出発する | Effect の署名 Σ を閉じた enum にし、**等式を法則として書く**(タイマー・書き込みの吸収・bracket)。解釈器=ハンドラ |
| **Free / Freer monad** | 効果のある計算を「操作の木」というデータで表し、解釈で実行・再生・追跡する | Plan をデータにして、解釈器を複数持つ(実機・再生・説明・偽物)。**ただし Rust には HKT も一級の継続も無い**ので、任意の `>>=` は持たない(下記) |
| **Applicative / Selective / Monad の階段** | Applicative=独立な効果(静的に全部見える)、**Selective=条件付き効果だが両方の枝が静的に見える**、Monad=結果が次の効果を動的に決める(枝が隠れる) | **このプロジェクトの連鎖の大半は Selective で足りる**(試す機構の集合は有限で静的。分岐の両側が見える→実行前に gate/capability を静的に検査できる)。Monad が要るのは Process だけ |
| **Mealy 機械 = 余代数**(圏論) | `update(State, Event) -> (State, Output)` は Mealy 機械。機械は積で合成できる | `update` を Mealy 機械とみなす。スライス(belief・intent・focus・engine)は積。Elm の `Cmd.batch` は出力の合成 |
| **State/Writer/Reader monad の積み重ね** | `StateT World (Writer Plan)` と `Reader Scope` が、Elm の `(Model, Cmd)` と ASGI の `scope` に当たる | Rust では `&mut World` + 出力の Plan + 引数の Scope で表す(monad は作らない) |
| **スコープつき Effect / monadic regions / bracket** | 資源の取得と解放を、構造で対にする(解放の漏れを型で防ぐ) | `OutputActiveGuard`・defer/drain の 2 窓口(ADR-123→128→156)の「片側だけ配線」事故を、`Bracket` で構造的に防ぐ |
| **自然変換としての解釈器** | 解釈器を取り替えても、観測できる結果は同じ(自然性) | 「偽物と本物の契約テスト」(ADR-229 D5)を、自然性の四角として定式化できる(再生 ∘ 記録 = 恒等) |
| **level-triggered な reconciliation**(Kubernetes) | 状態の差分から冪等な効果を作る。イベントの取りこぼしに強いが、観測が信用できないと振動する | **情報の収集にだけ採用**。IME の書き込みには採用しない(ADR-212 の履歴) |

## 2. 提案する構造

### 2.1 Event の分類と、生成できる Effect(capability)

| Event の種類 | 例 | 生成してよい Effect |
|---|---|---|
| Observation | プローブ結果、WinEvent、GJI の I/O、フォーカス変更の観測 | Probe(追加の観測要求)、Timer、Ui、Log。**Write は不可** |
| Intent | 物理 IME キー、ユーザー操作、設定変更、`UserImeSetIntent` | Write(`Warrant` を通ったものだけ)、Output(キー送信)、Probe、Timer、Ui、Log |
| Outcome | Cmd の結果(`ApplyComplete`、`ProbeResult::TimedOut` など) | 続きの Plan の一部(Process の次の step)、Probe、Log |
| Time | `WM_TIMER`、tick | Process の進行、Timer、Probe |
| Control | 設定の再読み込み、パニックリセット | 限定された Write(例外イベント。現状の 4 つ=PanicReset・HwndCacheRestored・ModeKeyPassedThrough・KeyEffectPredicted) |

- **性質テスト**: 「任意の Observation の Event に対して、`plan(world, event)` は `Write*` を含まない」を、Event の全 variant の網羅で検査する。

### 2.2 Plan の型(スケッチ)

```rust
// 効果の署名 Σ(閉じた enum。serde できる=journal に載る)
enum Effect { Out(OutEffect), Ime(ImeWrite), Probe(ProbeReq), Timer(TimerEffect), Gate(GateEffect), Ui(UiEffect), Log(LogEffect) }

struct Plan { steps: Vec<Step>, scope: ScopeId }   // scope = ASGI の scope(フォーカス世代など)
enum Step {
    Do(Stage, Effect),                          // 独立な効果。Stage で順序の粗い制約
    Try { first_of: Vec<ImeWrite>, stop_on: StopOn, reobserve: Option<ProbeReq> },  // selective: 有限で静的
    Require(Epoch),                             // 失効の再検査(INV-45 を Plan の中の 1 つの step に)
    Bracket { acquire: GateEffect, body: Vec<Step>, release: GateEffect },          // 資源。release は構造で保証
    Spawn(ProcessId),                           // 長いプロトコル(warmup)は別の Process に委譲
}
enum Stage { Observe, Gate, Write, Output, Ui, Log }   // 全順序の粗い順序。段階内は発行順
```

- **monad の `>>=`(任意の継続)は持たない**。`Try` と `reobserve` で、現在の「ImmCross が `Failed` のあとに再読み取りして `AlreadyMatched` かを決める」(`imm_cross_write`)が表せるかを、最大の検証点とする(下記 Q2)。
- **Process**: 複数 turn にまたがる状態機械(`StepCoro`、`ProbeCoroState`)。Plan の `Spawn` で起動し、Outcome/Time の Event で進み、各 turn で自分の Plan を出す。timed-fsm の `Response<A, T>` と `StepCoro` が既にこの形。

### 2.3 解釈器(shell と、それ以外)

| 解釈器 | 役割 | 既存の部品 |
|---|---|---|
| Real(Windows) | Step を順に実行し、Outcome を Event で戻す。各 Step の前後で `Require(epoch)` を検査 | `ImeController::apply`・`run_chain`・`open_chain.rs` の 3 関数(今は gate・失効の再検査が各所に複製) |
| Replay(純粋) | 記録済みの Outcome 列を返して、Plan の生成が同じかを検査 | `ReplayWriter`(PR #499) |
| Trace / Explain | 「この Event でこの Plan が出た理由・出なかった理由」を文字列で返す(dry-run)。不具合報告に載せる | 無い |
| Fake(Linux) | 擬似 IME(QUIRKS) | `tests/support/pseudo_ime.rs` |
| Static(解析) | 実行せずに、Plan が使う capability・gate・順序制約を検査する | `decide_gate` の判定(今は実行の途中で呼ぶ) |

- **gate は plan 時に 1 回、失効は `Require` の step で一様に**: ADR-180 が見送った「3 関数が独立に gate を再検出する」設計(await をまたぐと状態が変わるため)を、gate は Static 解析(plan 時)、失効は `Require(epoch)`(実行時、解釈器が一様に検査)に分けることで、1 つにできるか(Q3)。

### 2.4 ワールドモデル

- `World = fold(Event)`(Mealy 機械)。ただし**信念(belief)は、蓄積された証拠(観測・意図)に対する純粋な「view」**にする(`effective_open_at(now)` が既にこの形)。可変の `desired_open` に書く経路を、証拠の追加だけにする。
- 観測の保存は、**(source, scope) ごとに、(epoch, seq) の大きいものを残す**ような、順序に依らず収束する(join-semilattice)形にする。遅れて届いた古い観測や重複が、状態を壊さない。
- 時間(TTL など)に依存する view は、`now` を引数に取る(純粋)。

### 2.5 観測の購読(Elm の `subscriptions`)

`fn needs(&World) -> ObservationNeeds`(いま観測すべきソースの集合。例: GJI がアクティブな間だけ GJI モニター、TsfNative なら idle-conv-check)。shell が差分で WinEvent hook・モニタースレッド・タイマーを切り替える。**これが level-triggered の唯一の適用先**。

### 2.6 順序と衝突

- スライスごとの planner(エンジン、IME、focus)が出した Plan を、`Cmd.batch` 的に合成する。**Stage の全順序**(Observe < Gate < Write < Output < Ui < Log)で粗く並べ、同じ Stage の中は発行順。
- 順序の制約が Stage で足りないもの(例: baseline は SendInput の前、BUG-027/029/030/033)は、`Step` の依存辺として明示する(DAG)。**最初は Stage だけで足りるかを確かめる**。

## 3. 効果の法則(代数的な部分)

| 法則 | 内容 | 検査 |
|---|---|---|
| タイマー | `Set(id,d1); Set(id,d2) = Set(id,d2)`、`Kill(id); Kill(id) = Kill(id)`(`TimerRuntime` の doc に既にある不変条件) | proptest |
| 書き込みの吸収 | 同一対象への `SetOpen(x); SetOpen(y)` は、間に Probe が無ければ `SetOpen(y)` | proptest |
| 独立 | Ui・Log の効果は、他のどの効果とも可換 | proptest |
| bracket | acquire と release が対(release の漏れは構造的に無い) | 型 + 網羅 |
| 正規化の健全性 | `interpret(normalize(p)) ≈ interpret(p)`(Trace 解釈器の出力で比べる) | proptest |

- **正規化(`normalize(plan)`)は純粋で、1 か所**。今は `already_matches` による省略、defer キューの合体、重複する gate などが散っている。

## 4. このプロジェクトに当てはめたときの利点と、過去の轍との比較

- **利点(行数ではなく)**: (a) 解釈器を差し替えて再生・説明・偽物を同じ Plan で動かせる(replayability)、(b) gate・capability・失効の検査が Plan に対して 1 回(複雑性: 今は 3 関数 + ime_controller の 4 か所)、(c) `Bracket` で「片側だけ配線」の事故を構造的に防ぐ(ADR-123→128→156、5 か所)、(d) Observation が書き込みを生成できないことを網羅テストで保証(ADR-208)、(e) dry-run の説明を不具合報告に載せられる。
- **轍との比較**: ADR-218〜220(宣言テーブル・DSL)は見送り。本案は新しい DSL を作らず、既存の `MechanismCommand`・`decide_*`・`ActuationOrder`・`ProbeAction`・`Response` を **Plan という 1 つの項に束ねる**。ADR-180 決定2(レコード組み立ての統一)は費用対効果が負だった。本案も、実害の記録(下記)と、使い手が 3 か所以上あること(rule of three)を、段階ごとに確かめてから進める。
- **実害の記録の候補**: ADR-180(gate の 3 重)、ADR-156(defer と drain の 2 窓口)、BUG-34(観測と実行の間の窓)、BUG-98(世代の無い非同期の完了)、BUG-141(`outcome: Applied` だけでは成功と言えない)、ADR-212(補正的 actuation の撤去。level-triggered を書き込みに使わない根拠)。

## 5. 段階(いずれも着手しやすい所から。各段階で結果を見て考え直す)

| 段階 | 内容 | 前提・使い手の数 |
|---|---|---|
| E0 | Effect の署名 Σ の棚卸し(今の `MechanismCommand`・`ProbeAction`・`UiEffect`・`OutEffect`・`TimerCommand` などの全 variant の一覧と、それぞれの順序制約・冪等性・可換性) | 読み取りのみ。W0 と並行 |
| E1 | **Trace 解釈器(dry-run の説明)**: 既存の `decide_*` の出力から、「この Event で何が決まったか・なぜ省略したか」の説明を作る。journal に記録 | 新しい抽象を足さずに、説明だけ。価値が測れる |
| E2 | `Try { first_of, stop_on, reobserve }` を、`run_chain`(`MechanismWriter`)の外側の表現として導入し、Static 解析(gate・capability)を plan 時に 1 回にする | `open_chain.rs` の 3 関数 + `ImeController::apply`(使い手 4)。ADR-180 の領域 |
| E3 | 正規化と法則(タイマー・書き込みの吸収)を proptest で | タイマー(多数)、書き込み(複数) |
| E4 | `Bracket`(gate の対)。`OutputActiveGuard` の使い手 5 か所 | ADR-156 の領域 |
| E5 | Event の分類と capability の網羅テスト(Observation は Write を生成しない) | `ImeEvent` の全 variant |
| E6 | 観測の購読(`needs`) | 後段 |

## 6. Opus に判断を仰ぎたい論点

- **Q1**: 「Plan は任意の継続を持たず、Selective(`Try`)で足りる」は、このコードの連鎖(open_chain の 3 関数、`imm_cross_write` の `Failed` 後の再読み取り、`romaji_pre_write`、focus の段階的な分類)で本当に成り立つか。成り立たない箇所(結果が次の効果の*種類*を動的に決める)は何か。
- **Q2**: `Try { first_of, stop_on, reobserve }` で、現在の `imm_cross_write` の `post_failed_reobservation`(`Failed` のあとの `read_ime_state_fast` で `AlreadyMatched` を判定)を、executor を太らせず(F にせず)表せるか。表せないなら、Plan にどんな最小の拡張が要るか。
- **Q3**: gate を plan 時の Static 解析に、失効を `Require(epoch)` に分ける案は、ADR-180 の「3 関数が独立に gate を再検出する(await をまたぐと状態が変わる)」の理由を満たすか。INV-45・BUG-34・B-1 の fail-open との整合。
- **Q4**: `Bracket` は、`OutputActiveGuard` と defer/drain の 2 窓口(ADR-156)を、実際に構造で対にできるか。RAII がタスクの寿命にぶら下がる現在の形(`ChromeProbe` への move など)と矛盾しないか。
- **Q5**: Event の分類(Observation/Intent/Outcome/Time/Control)と capability の網羅テストは、既存の `ImeEvent` の約 65 variant と例外イベント 4 つで成り立つか(境界の曖昧な variant)。
- **Q6**: 「level-triggered は情報の収集だけ」の線引きは、ADR-212・ADR-208・BUG-141 の履歴と整合するか。`observe_miss_monitor`・drift correction・warmup の再試行は、どちら側か。
- **Q7**: 効果の法則(タイマー・書き込みの吸収・独立・bracket)は、実際のコードで成り立つか。成り立たない例(順序が意味を持つ、`SetOpen(x); SetOpen(y)` の間に副作用が挟まる等)。正規化を入れると、再発ファミリーを悪化させる箇所は。
- **Q8**: 信念を「証拠の view」にし、観測を join-semilattice にする案は、`ImeModel::reduce` の既存の規約(`ObserverReported` は `desired_open` を直接書かない、信頼度、`last_intent`)と整合するか。破綻する箇所。
- **Q9**: この案は、ADR-229(FCIS)の F-D1〜F-D6 と矛盾するか、置き換えるか、延長か。toolkit round1 の「作らない」判断(handler trait の汎用化、Facts の trait など)との関係(Plan は新しい汎用の型だが、ドメイン固有で rule of three を満たすか)。
- **Q10**: 過去に見送った案(ADR-218〜220、ADR-180 決定2、ADR-053)と同じ轍か。実害の記録に対応するか。「根本的な設計変更」の費用に見合うか。段階 E1(Trace)だけで終わるのが最善、という結論もありうるか。
- **Q11**: 欠けている論点、より良い代案(例: Plan を作らず、解釈器の差し替えだけを `MechanismWriter` 型の trait で続ける現状の延長)。
