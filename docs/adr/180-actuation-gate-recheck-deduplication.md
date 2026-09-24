---
id: ADR-180
title: |-
  非同期actuation経路のInputRelayゲート再検証を、`with_app`を内包しない
  共有ヘルパー`is_input_relay()`へ統合する（decision1、実装済み）。
  `ActuationDecisionRecord`の3通りの組み立て方統一（decision2）は
  3ラウンドの検証の結果、費用対効果が負と判明し見送る
summary: |-
  ADR-179（旧178）領域B（旧「6箇所のIME actuation合流点」）の設計検討、3ラウンドの
  opus-adversarial-consultを経た。round1: 「領域A撤去で合流点6→4」は誤りで
  `decide_gate`呼び出しは5箇所のまま不変と判明、当初提案の共有gateヘルパー
  （`with_app`内包）は`fallback_write`から呼ぶと再入で恒久的にfail-open化
  する危険が判明。round2: 代替案（`ActuationDecisionRecord`統一）にも同型の
  罠3件を検出。round3: 罠を塞いだ修正版にもさらに実装不能な要件（`caller`が
  `async_record`では構築時引数で事後設定に変更できない）と、実施コストが
  削減効果を上回る（本番−40行に対しテスト+ガード+80〜125行、actuation機構数は
  不変）ことが判明し、decision2は最終的に見送りと結論。一方decision1は、
  ユーザー指示（2026-09-19、リスクを受容し実機検証しながら進める方針への
  転換）を受けて`with_app`を内包しない安全な形（round1 E2）で実装済み
  ——`is_input_relay()`を`state/ime_actuation_decision.rs`に追加し、
  `open_chain.rs`の3箇所（`run_open_chain_async`/`imm_cross_write`/
  `fallback_write`）から呼ぶ形に統合、`tests/architecture_guard.rs`の
  ガードを関数別カウントへ作り替えた。
status: |-
  **decision1: 実装済み・push予定。decision2: 3ラウンド検証の結果見送り
  確定（コスト>効果、`caller`要件が実装不能）。** より野心的な統合
  （ADR-090 §2.A A-2、warrant強制）は本ADRとは別に着手済み
  （ユーザー指示によるリスク受容・実機検証方針への転換、ADR-090側で追跡）。
related_adr:
  - "ADR-086"
  - "ADR-087"
  - "ADR-090"
  - "ADR-098"
  - "ADR-106"
  - "ADR-119"
  - "ADR-159"
  - "ADR-162"
  - "ADR-179"
---

# ADR-180: IME actuation決定レコードの組み立て方を統一する

**2ラウンドのopus-adversarial-consultで、当初案（gate統合）と第一の代替案
（レコード統合、round1時点）の両方に実装すると危険な罠が見つかった。**
経緯自体が将来の再発防止に重要なので、末尾「レビュー経緯」節に残す。
以下はround2反映後の現行案。

## 主目的（誤解しないこと）

**本ADRの主目的は、新しい統一fence型・新しいgate機構・新しい抽象を
導入することではない。** ADR-178（`docs/adr/179-*.md`=ADR-179、領域A撤去
コミット`f83084b3`/`621bf93c`）の続きとして「領域B: IME actuation合流点」
を調べ直した結果、以下が判明した。

1. **`decide_gate`（InputRelay判定）の呼び出し箇所は5箇所で、領域A撤去の
   前後で1つも減っていない。** 撤去した`reassert_explicit_physical_key`/
   `force_on_and_correct_romaji`はどちらも`decide_gate`を自前で呼んでおらず、
   `ImeController::apply`という既存の合流点へ**入る側**の入口だった
   （`.claude/rules/fix-requires-evidence.md`の合流点表と、本ADRが対象と
   する「`decide_gate`を呼ぶボイラープレートの箇所数」は別の数え方）。
2. 5箇所のうち機械的に共通化できるのは**2箇所のみ**
   （`run_open_chain_async`/`imm_cross_write`）。3箇所目`fallback_write`は
   `view`を`decide_gate`の**前に**改変する（BUG-113対策の`shadow_on = None`
   上書き）ため、共有ヘルパーが`with_app`を内包する形にすると
   **`fallback_write`から呼んだ瞬間に`with_app`再入でgateが恒久的に無効化
   される**（issue #136/BUG-90型の回帰）。この統合は**見送る**（決定1）。
3. 代わりに`ActuationDecisionRecord`の組み立て方（3ファイルに3通り）の
   統一を検討したが、これにも決定1と同型の罠が3件見つかった
   （`async_record`だけが`chain_len=4`を固定で埋める／凍結コーパス再生は
   実は3実装のどれも呼ばない／`caller`事後設定は「歪み」ではなく意図された
   設計で4箇所ある）。安全に実施するには、純粋部分をungatedモジュールへ
   移し旧3実装との同値性をLinux全数テストで固定する必要がある（決定2）。

成功基準は「新機構を作ったか」ではなく「重複コードをどれだけ**安全に**
削減できたか」である。2ラウンドを経て「深い統一(gate機構の再設計)は
やらない、浅いレコード組み立ての重複だけを、既存3実装の値を1つも変えない
形で統一する」という、当初よりさらに縮小した結論に至った。

## 背景: `decide_gate`の5つの呼び出し箇所（訂正版）

| # | 場所 | `with_app`の使い方 | NotOwned時に作るもの |
|---|---|---|---|
| 1 | `ime_controller.rs::ImeController::apply`（同期経路唯一の合流点） | 呼び出し元が既に`with_app`内、`view`を引数で受け取る | `actuation_decision_record()`（`site: Sync`固定、`chain: &[]`→`chain_len: 0`） |
| 2 | `runtime/executor.rs::dispatch_ime_set_open` | 同上（`platform`/`ime`を引数で受け取り`with_app`不使用） | インライン構造体リテラル（`chain_len: 0`固定）——3つ目の書き方。この時点ではsync/async分岐前で`chain`自体が未導出のため`0`が正しい |
| 3 | `runtime/open_chain.rs::run_open_chain_async`（`spawn_local`直後） | 内部で`with_app`を呼ぶ。再入時fail-open（`is_input_relay=false`で書き込み続行） | `async_record()` → `ActuationDecisionRecord`、`journal.record`まで実行。**`chain`引数を取らず常に`WriteMechanism::ALL`固定＝`chain_len: 4`をNotOwnedでも記録する**（ADR-159が意図した非対称、後述） |
| 4 | `runtime/open_chain.rs::imm_cross_write`（ImmCross機構の`.await`前） | 内部で`with_app`。3と`with_app`クロージャ9行がbyte一致 | `AttemptRecord`（`command`あり、`shadow_on_before_bug113_override: None`） |
| 5 | `runtime/open_chain.rs::fallback_write`（ImmCross以外の機構。**非async関数**、直前の`.await`完了後に呼ばれる） | 関数本体全体が1つの`with_app`クロージャ内。**再入失敗時は`(ImeOpenOutcome::Failed, None)`を返す——3・4の「fail-openで書き込み続行」とは異なり「viewが作れず何もできない」という別の帰結** | `AttemptRecord`（`command: None`固定、`shadow_on_before_bug113_override: Some(..)`——**`decide_gate`に渡す`inputs`自体もこの上書き後の値**） |

1は同期経路（`.await`を挟まない）なので1回で十分——重複は無い。2は
`with_app`を使わない独立実装。3・4・5は非同期経路で、それぞれ別の
`.await`境界の直前・直後にいるため個別に再検証が必要——これがADR-119が
「ゲートを1点に集約できなかった」と結論した理由であり、本ADRもこの結論を
変えない（変えたいのは「タイミング」ではなく「コード」）。

**`chain`/`chain_len`の埋め方は3者で意図的に異なる**（sync=実際に走査した
chain／async=`WriteMechanism::ALL`固定・ADR-159の理由により変更しない／
executor=chain導出前なので0）。統一する場合、これを1つの値へ正規化しては
ならない（決定2のF1参照）。

## 決定1: `decide_gate`のボイラープレート統合は見送る

3・4（`run_open_chain_async`/`imm_cross_write`）の`with_app`クロージャ
9行は、共有ヘルパーに切り出す形では実施しない。理由:

- ヘルパーが`with_app`を内包すると、**同じヘルパーを5（`fallback_write`）
  からは呼べない**——`fallback_write`は既に`with_app`クロージャの中に
  いるため再入し、`try_borrow_mut`失敗→`None`→fail-open→**InputRelay
  ゲートが恒久的に無効化される**（issue #136/BUG-90決定4の回帰そのもの）。
- ヘルパーが`with_app`を内包しない形（`&ImeControlView`を引数で受ける
  純粋関数）にすれば上記の危険は避けられるが、削減できるのは
  実質4〜5行×2箇所に縮小し、費用対効果が薄い。
- `tests/architecture_guard.rs::decide_gate_wiring_occurrence_counts_are_pinned`
  はファイル別の`decide_gate(`出現数を固定しており、これがADR-119の事故
  （5経路のうち1つがgateを素通り）を検知する唯一の機械的ガードである。
  統合するならこのガードを関数別カウントへ作り替える必要があり、
  作り替えないなら統合そのものを見送るべき。
- この3・4・5の経路には**実行可能なテストが1件も無い**（`architecture_guard.rs`
  以外の全テストファイルがInputRelayを0件参照、凍結コーパス37件は
  全て`site: Sync`・`profile: TsfNative`）。変更しても「実行した経験の
  無いコードを変更した」ことにしかならず、差分ゼロの検証ができない。

**この判断は撤回可能**——将来、非Sync siteの実機コーパスが増える、
または`with_app`を内包しない形の削減(4〜5行×2箇所)でも価値があると
判断されれば再検討してよい。今回は見送る。

**実装時の付随作業（設計変更ではない）**: 決定1を見送った理由を
`open_chain.rs:583`と`:241`（`with_app`クロージャの直前）に2行コメントで
残すこと。「なぜこの重複を統合しなかったか」がADR文書だけに書いてあると、
`open_chain.rs`を直接開いた将来の実装者には届かず、`.claude/rules/
experiment-logging.md`が警告する「同じアイデアの再導入→同じ失敗の再発」
が起きる。

## 決定2: `ActuationDecisionRecord`の組み立て方を統一する（round3で見送りと結論）

**round3の検証結果、決定2は実施しないと結論した。** 以下の本文（要件a〜d）は
round2時点で罠を塞いだつもりの修正版だが、round3でさらに2つの問題が
見つかった。

1. **要件bが実装不能（round3 H1、Blocker）**: `async_record`は`caller`を
   構築時引数として受け取る設計（`open_chain.rs:188-206`）であり、
   「`caller`はコンストラクタ引数にせず現状維持」という要件bはこの箇所に
   適用できない。統一対象が3→2箇所に縮小する。
2. **費用対効果が負（round3 H4）**: 本番コードは`-40`行程度（ungatedモジュールへの
   移設）に対し、旧3実装との同値性を保証する全数テスト・
   `architecture_guard`のリテラル構築数ガードの追加で`+80〜125`行が必要になる。
   しかも`actuation`機構の数（force-on/reassert等、削除量で測るべき対象）は
   1つも減らない——`ActuationDecisionRecord`は診断用の記録型であり、
   統一してもactuation経路そのものは変わらない。

以上より、決定2は**実施しない**。将来「レコード組み立てを統一したい」という
同じアイデアが再浮上したときにこの2点（`caller`の構築時引数依存・
費用対効果が負）を再確認せずに再実装しようとする事故を防ぐため、以下の
検討過程（round2時点の要件a〜d）は記録として残す。

<details>
<summary>round2時点の修正版（round3でH1/H4により最終的に見送り）</summary>

3通りに散っている記録の組み立て方を統一する。**ただし以下の3つの要件
（round2で発見した罠への対応）を全て満たすこと。要件を満たさない実装は
決定1と同じ理由で見送るべき。**

### 要件a: `chain`/`chain_len`は引数で受け取り、正規化しない

背景表のとおり、sync/async/executorの3者は`chain`の意味論が異なる
（走査結果／`ALL`固定／未導出）。統一コンストラクタは
`new(site, gate_inputs, order, chain: &[WriteMechanism], attempts, attempts_len)`
のように`chain`を引数で受け取り、既存3箇所が渡していた値をそのまま渡す
形にすること。「NotOwnedなら`chain`は空」という単純化は、`async_record`
の`ALL`固定という**意図された非対称**を握り潰す（round2 F1）。

### 要件b: `caller`はコンストラクタ引数にしない、事後設定を維持する

`caller`の事後設定（`record.caller = Some(..)`）は歪みではなく、
PR #201 B-2で確立した意図的な設計である——`site`を「実際にどの経路で
`decide_attempt`が計算されたか」の値として汚さず、`caller`を別フィールド
として「診断上どの呼び出し元由来か」を運ぶ、という役割分担
（`state/actuation_decision_record.rs`の`caller`フィールドdoc参照）。
呼び出し元は4箇所（`executor.rs`/`key_pipeline.rs`×2/`ime_refresh.rs`）
ある。

統一コンストラクタに`caller`引数を追加すると、`ImeController::apply`
（`lints/actuation_call_guard`が許可呼び出し元を宣言するchoke point）の
シグネチャ変更を要求し、`.claude/rules/fix-requires-evidence.md`の
「IME actuation合流点」ファミリーに該当する変更（回帰テストか
`docs/known-bugs/`記録が必須）になる。統一コンストラクタは`caller: None`
固定で構築し、4箇所の呼び出し元は従来どおり構築後に`record.caller = ..`
を設定する（**この4箇所のパターン自体は変えない**）。

### 要件c: 同値性の検証は「純粋部分のungated化＋Linux全数テスト」で行う。凍結コーパス再生には依拠しない

凍結コーパス37件の再生テスト（`replay_all_actuation_decision_fixtures`）
は、JSONを`ActuationDecisionRecord`へ**直接デシリアライズ**して
`replay_record`（`decide_gate`/`decide_chain`/`decide_attempt`という3つの
純粋関数だけを呼ぶ）に渡すものであり、**`actuation_decision_record()`/
`async_record()`/executorのインラインリテラルのいずれも一度も実行しない**
（round2 F2で実測確認）。したがって「コーパスで統一後のコンストラクタの
正しさを検証できる」という主張はできない。

代わりに以下の手順（G1）で検証する:

1. 統一コンストラクタ（`ActuationDecisionRecord::new(..)`）を、
   `state/actuation_decision_record.rs`（既にungated、`state/mod.rs`から
   Linuxでも実行可能、本ADR round2レビューで実測: 35テストがLinuxで
   実行済み）に置く。`chain_record()`（現`ime_controller.rs`）・
   `all_chain_record()`（現`open_chain.rs`）も同モジュールへ移す
   （どちらも`WriteMechanism`のみを触る純粋関数）。
2. 旧3実装が計算していた式をテスト内に参照実装として書き下し、
   `site`×`caller`×`chain`パターン×`attempts_len`の代表的な組み合わせで
   `assert_eq!(reference, ActuationDecisionRecord::new(..))`する
   （`ActuationDecisionRecord`は`PartialEq + Eq + Copy`なのでbit比較が
   素直に書ける）。これが「差分ゼロ」の実体であり、コーパス再生では
   ない。
3. コーパス再生は「決定関数(`decide_gate`/`decide_chain`/`decide_attempt`)
   を壊していないこと」の**別の**保証として引き続き実行する（既に
   greenなので追加作業は無い）。ただし決定2の正しさの証拠としては扱わない。
4. `tests/architecture_guard.rs`に、`ActuationDecisionRecord { `の
   リテラル構築箇所数をファイル別に固定するガードを追加する（統一後は
   `state/actuation_decision_record.rs`の1箇所のみになるはず）。現状
   このガードは0件（実測）——コンストラクタを足しても、将来4つ目の
   インラインリテラルが生えるのを止めるものが無いままになる。

### 要件d: 以下は変更しない（不変条件）

- `ActuationOrderRecord`（`Copy`）を受け取る形を維持する。`&ActuationOrder`
  を受け取る形に変えない——`ActuationOrder`はINV-47のアフィン値であり、
  記録のためだけに`.clone()`で複製しないという既存の/code-review結論
  （S-5）を静かに巻き戻すことになる。
- `executor.rs::dispatch_ime_set_open`のNotOwned記録専用に発行している
  使い捨て`ActuationOrder`（`issue_self_actuation_order(open,
  "dispatch_ime_set_open_gate_not_owned")`）の発行自体は消さない
  （ADR-090 A-1の warrant会計への副作用が無いことは既に確認済み）。
- 統一コンストラクタが`chain`を`decide_chain(gate_inputs)`から**導出**
  する形にしない。sync だけが`decide_chain`と一致し、async は`ALL`固定
  （ADR-159の理由で意図的に非対称）——導出に変えるとasync側が壊れる。

### スコープ外: `AttemptRecord`の統一は本ADRに含めない

`AttemptRecord`の構築は`open_chain.rs`に4箇所あり、`command`
（`Some`/`None`）・`shadow_on_before_bug113_override`（`None`/`Some`）が
箇所ごとに異なる。特に`fallback_write`は`inputs.shadow_on`の値そのものが
BUG-113対策の上書き後の値であることが`replay_record`の不変条件
（`shadow_on_before_bug113_override.is_some() ⟹ inputs.shadow_on.is_none()`）
としてハードコードされている（決定1が見送った罠と同型、round1 C3）。
`AttemptRecord`の統一は本ADRのスコープに含めず、将来別ADRで検討する
場合はこの非対称を最初から前提に置くこと。

## ADR-162 TH1eとの関係（位置づけを弱める）

[ADR-162](162-governance-reversal.md)（複雑性予算制）のTH1e発効条件は
「実際の削除・統合を1件、**N本の決定レコード（attempts列・
`MechanismCommand`列）**の再生で差分ゼロと検証できたこと」である。
決定2は`ActuationDecisionRecord`の**外枠**（`site`/`chain`/`caller`等）を
統一するものであり、`attempts`/`MechanismCommand`列自体は一切変更しない。
したがって**決定2はTH1eが求める証明にはならない可能性が高い**。

本ADRはTH1eの「証明ケース」と断定して位置づけることを避け、「構築側の
統一という部分的な前進」とのみ位置づける。TH1eの発効条件を満たす証明
そのものを求めるなら、ADR-162の起票者判断を別途仰ぐこと。

## `ModeKeyActuationOwner`との関係（ADR-179、round1で発見・スコープ外に保留）

round1で、`ModeKeyActuationOwner::PhysicalDelivery`（ADR-179、「awase側の
送信をゼロにする」ことが設計の核心）のenforceが、`key_pipeline.rs`内の
2つの`if`（`owner_permits_explicit_off_actuate`と
`strip_activation_sync_set_open_for_physical_delivery`）だけに留まって
おり、上記5つのchoke point自体は`ModeKeyActuationOwner`を一切知らない、
という非対称が見つかった。`kp_stage_shadow_ime_toggle`を経由しない別経路
（drift correction、idle-conv-checkのDirectInput回復等）から choke point
に到達した場合、`PhysicalDelivery`な打鍵に対してもawaseが送信してしまう
可能性がある——ADR-119/issue #136が名指しした「新しいgateを1箇所に置いて
満足しない、実際の呼び出し経路を全て洗い出す」という教訓が再現している
可能性がある。

**この非対称は実在するが、本ADRのスコープには含めない。** 理由:

1. 修正の形は「`ModeKeyActuationOwner`を`DecisionInputs`に混ぜて1つの
   型に統合する」ではなく（別軸の値を1つの機構に共有することになり、
   ADR-106決定5が戒める形に近づく）、「`PhysicalDelivery`の抑止を
   `key_pipeline`の2つの`if`からchoke point側へ移す」という**配置**の
   問題である。
2. ADR-179自身が実験段階（`c0814776`/`f0e36b0e`は`experiment`コミット、
   実機A/B未実施）であり、まだ安定していない機構をchoke point側へ
   前倒しで配線すると、ADR-179側の実験結果次第で二度手間になる。

ADR-179が実機A/Bを終え安定した後、別ADRとしてこの配置の問題に着手する
ことを推奨する。本ADRはこの発見を記録するに留める。

## 明示的にスコープ外にすること

- **決定1で見送った`decide_gate`のボイラープレート統合**（上記）。
- **`AttemptRecord`の組み立て方統一**（上記、決定2のスコープ外節）。
- `focus_epoch`/`ime_mode_focus_gen`/`ApplyGeneration`の統合・共有ストレージ化
  （[ADR-106](106-fence-ownership-and-observation-provenance.md)決定5が
  値の共有として明示的に禁じている。決定5はfence**値**の共有を禁じたので
  あり、状態を持たない`decide_gate`という述語の共有はそもそも決定5の
  対象外——round1で「決定5が本ADRを却下している」という誤った位置づけを
  訂正済み）。
- [ADR-090](090-typestate-effectuation-and-adjacent-adr-closure.md) A-2
  （`decide_gate`のhard blockと`ActuationOrder`/`log_shadow_warrant`の
  shadow記録を1つの授権機構へ統合する計画）——実機ソーク必須のため既に
  「着手不可」と分類されている既存計画であり、本ADRはこれを代替しない・
  対象にしない。`decide_gate`は[ADR-087](087-open-belief-actuation-warrant-separation.md)
  §1.5の**根拠軸**（プロファイルcapabilityからwarrant発行可否を引く、
  ADR-081との関係として同節が明記）に属する既存の軸の一部であり、
  「第5の軸」のような新しい軸を立てる必要はない。
- `ModeKeyActuationOwner`(ADR-179)のchoke point側enforce配線（上記節、
  ADR-179安定後に別ADR）。

## 未解決の疑問（opus-adversarial-consult round3で検証してほしい点）

1. 決定2の要件a〜dは、round2が見つけた3つの罠（F1/F2/F3）を実際に
   塞いでいるか。特に要件c（G1手順）が、決定1で見送った「テストが
   変更対象コードを1度も実行しない」問題を本当に解消しているか
   （`state/actuation_decision_record.rs`が本当にLinuxで実行可能で
   ungatedかを再確認してほしい）。
2. 決定2の実施コスト（ungated化・全数テスト・architecture_guardガード
   追加）に対して、削減できる重複行数が本当に見合っているか。
   見合わないなら決定2自体も「見送り」が正しい結論かもしれない。
3. `caller`を事後設定のまま残す（要件b）ことで、コンストラクタ統一の
   実質的な価値（何が「統一」されたと言えるのか）がどれだけ残るのか。

## レビュー経緯（記録）

このADRは2ラウンドのopus-adversarial-consultを経て、当初案から大きく
縮小した。将来「gate機構やレコード組み立てを統合したい」という同じ
アイデアが再浮上したときに、同じ調査を繰り返さず本節を参照できるように
するため記録する。

### round1で判明したこと

- **前提の数え方の誤り**: 「領域A撤去で合流点6→4」と書いたが、
  `decide_gate`の実際の呼び出し箇所は撤去前後とも5箇所で不変。
  [ADR-090](090-typestate-effectuation-and-adjacent-adr-closure.md)
  A-1'が既に自己訂正した「needleの数と起案点の数を取り違える」誤りの
  再演だった。
- **箇所の見落とし**: `executor.rs::dispatch_ime_set_open`
  （ADR-163が専用`DecisionSite::DispatchImeSetOpen`を新設した独立gate）
  を背景表から完全に落としていた。
- **危険な実装提案**: `with_app`を内包する共有ヘルパーを提案したが、
  `fallback_write`（関数本体全体が`with_app`クロージャ内）から呼ぶと
  再入により恒久的にfail-open化する。加えて`fallback_write`の
  `shadow_on = None`上書き（BUG-113対策）を握り潰し、BUG-113の実害
  （`GjiDirectStrategy`が`AlreadyMatched`を誤返却）と
  `actuation_decision_record.rs`の再生不変条件違反を同時に引き起こす。
- **既存計画の見落とし**: [ADR-090](090-typestate-effectuation-and-adjacent-adr-closure.md)
  A-2（decide_gateとwarrant shadow記録の統合計画）を検討対象から外して
  いた。
- **`ModeKeyActuationOwner`統合の動機を誤って「無い」と結論**: 実際には
  `PhysicalDelivery`のenforceがchoke point側に存在しないという実在の
  ギャップがあった。ただし修正の形は「型統合」ではなく「配置の移動」で
  あり、ADR-179の実験段階が終わるまで着手しないという判断自体は妥当と
  判定された。
- **ADR-106決定5の引用が的外れ**: 決定5は「fence値の共有」を禁じたもの
  で、状態を持たない述語（`decide_gate`）の共有はそもそも対象外。

### round2で判明したこと

- **`chain_len`の非対称を握り潰す危険**: `async_record`はNotOwnedでも
  `WriteMechanism::ALL`固定（`chain_len=4`）を記録するが、他2箇所は`0`。
  素直な統一コンストラクタはこの差を正規化してしまい、bug report JSONの
  内容が黙って変わる。既存のどのテストにも引っかからない。
- **凍結コーパス再生は統一対象のコードを1度も実行しない**: 37件の
  フィクスチャはJSONから直接デシリアライズされ、`decide_gate`/
  `decide_chain`/`decide_attempt`という3つの純粋関数にのみ渡される。
  `actuation_decision_record()`/`async_record()`/インラインリテラルは
  一度も呼ばれない。決定1を見送った理由（テストが変更対象を踏まない）が
  決定2にもそのまま当てはまっていた。
- **`caller`の事後設定は歪みではない**: PR #201 B-2で確立した意図的な
  設計（`site`を汚さずprovenanceを残す）であり、4箇所ある（1箇所ではない）。
  コンストラクタ引数化はchoke pointのシグネチャ変更を要求し、
  fix-requires-evidence.mdの対象になる、当初想定より1段大きい変更。
