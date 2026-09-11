# ADR-163: actuation合流点の「決定」と「実I/O」の分離、および決定点ジャーナル再生ハーネス

## ステータス

**設計は収束済み（opus-adversarial-consult round1・round2・round3実施済み・反映済み。
round3で「設計の骨格は収束した」と判定され、round4は不要）。実装はTH1a〜TH1cが
developへマージ済み、TH1d・TH1eが未着手（2026-09-11時点、実装順序は「今後の議論」節参照）。**

- **TH1a（`key_sequence_policy`のImeKindId化等、Step 0）: 完了**
  （`69a9a6b5`、PR#195に統合）。
- **TH1b（Part A、`decide_gate`/`decide_chain`/`decide_attempt`の新設と
  `ime_controller.rs`/`executor.rs`/`open_chain.rs`への配線）: 完了**
  （`e72adfaa`/`cc8624bf`/`3f3bb17c`、PR#195マージ・/code-review指摘対応済み）。
- **TH1c（Part B、`ActuationDecisionRecord`/`AttemptRecord`スキーマ+
  crate内`#[cfg(test)]`再生ハーネス）: 完了**（`6e389a9a`/`f88d019b`、PR#196マージ、
  `state/actuation_decision_record.rs`）。ただしハーネスが読むfixtureは
  現時点ではすべて手組み（`replay_all_actuation_decision_fixtures`が読む
  テストコード内固定値）であり、**`tests/journals/actuation_decision/`
  （実機ダンプからの凍結コーパス）はまだ存在しない**。
- **TH1d（`tests/journals/actuation_decision/`への既知バグ由来fixture投入+
  `assert!(total > 0)`ガード追加）: 未着手**（置き換えではなく維持、下記TH1d'とは
  並行タスク）。
- **TH1d'（新設、Part D）: bug report経由の実機コーパス自動収集: 未着手**。
  opus-adversarial-consult round4を1ラウンド実施済み（Blocker7件・Should-fix9件、
  すべて反映済み）。初版の「別枠リングバッファ+bug_reportへの新規フィールド追加」
  という設計を撤回し、「`journal.rs`のJournalEntry variantとして相乗りする」設計
  （決定D1〜D8）に全面的に書き直した。
- **TH1e（Part C、`AsyncChainWriter::is_applicable`統合+差分ゼロ再生証明、
  ADR-158 TH1発効条件の充足）: 未着手**。

[ADR-159](159-existing-io-boundary-inventory.md)の子ADR。ADR-159段階1（TF1）・段階2の最小実装
（TF2、`shadow_send_trace.rs`）は完了済みだが、これらは「何が起きたか」を記録する側だけであり、
「記録した入力をもう一度、意思決定ロジック（ゲート判定・戦略選択・チェーン走査）に通してWin32を
実際に叩かずに送信列を再現する」という再生ハーネス側が存在しなかった。本ADRはこの欠落を埋める
設計であり、TH1a〜TH1cの実装により再生ハーネス自体（crate内・手組みfixture限定）は存在する
状態になった。**round1レビューでPart Aの前提（既存分離の範囲）が実コードと食い違っていること、
および再生入口が浅すぎて検出したい回帰の中心（ADR-119型のゲート見落とし）を通らないことが判明し
決定節を全面的に書き直した。round2レビューでは、書き直した決定節の内部（決定関数のシグネチャと
attempt単位記録の非両立、TH1eの証明対象選定、windows-gated型の混入）に新たな矛盾が見つかり、
さらに反映した**。round1/round2の指摘は「round1・round2での指摘と反映」節に記録する。
**本ADRはTH1eが未完了のため、
[ADR-158](158-complexity-reduction-north-star.md) TH1の発効条件（実削除・統合1件＋差分ゼロ再生証明）
をまだ満たさない——TH1a〜TH1cはTH1の必要条件の一部を満たすに留まる**（後述）。**

## 背景

### 何と何を混同しないか

`journal_replay.rs`が実際にやっているのは「`classify_conv_transition`という1つの純粋関数に、
記録済みの入力を渡して出力を比較する」というフィクスチャ再生であり、これは`state/conv_classify.rs`
という**末端の純粋判定関数**を対象にしている。同種の手法を他の`classify_*`関数へ横展開することは
可能だが、それは末端の純粋関数の再生対象を増やすだけであり、**ゲート・actuation・SSOTの
「解体・統合の安全性」を担保する再生基盤とは別物**である。

ADR-159本文の該当箇所:

> リファクタ・統合の安全性を「N本の記録トレースを再生して送信列が一致するか」で判定できる
> ようになる。

これが指すのは、`runtime/executor.rs`のゲート判定・`ime_controller.rs::apply`の戦略選択・
`runtime/open_chain.rs`のオーケストレーションという**actuation合流点そのもの**を、記録済みの
入力から再実行し、実際にWin32へ送った内容（`SendInput`/`WM_IME_CONTROL`）と一致するかを検証する
再生である。TF1（`journal.rs`の観測記録）・TF2（`shadow_send_trace.rs`、実送信内容の1行ログ）は
「何が起きたか」を記録する半分が完了しただけで、「記録した入力を決定ロジックにもう一度通す」
再生ハーネス側はまだ存在しない。

### 実コード調査で判明した既存の分離状況（round1で訂正済み）

**当初案は「`state/actuation_chain.rs`・`state/key_sequence_policy.rs`・`state/ime_decision_view.rs`
は既にungatedで、`ime_controller.rs`だけがwindows-gatedなので橋を架けるだけでよい」としていたが、
これは誤りだった**（round1 M1、`cargo test -p awase-windows --lib -- --list`実測で確認）。

正しい状況:

- `state/actuation_chain.rs::Actuation::<Verified>::run_chain` / `run_chain_async` と
  `MechanismWriter` / `AsyncMechanismWriter`トレイトは**確かにungated**。チェーン走査・
  フォールスルー判定（`falls_through`）・アフィン性（1値=高々1回の成功write、INV-41）は
  完全にwindows非依存で、`FakeWriter`（`actuation_chain.rs:733`、`impl`は`:749`）を使った
  テストがLinuxで実行される。ここは当初案の記述通り。
- **`state/key_sequence_policy.rs`と`state/ime_decision_view.rs`はwindows-gated**
  （`state/mod.rs:136-142`の`#[cfg(windows)] pub(crate) mod key_sequence_policy;`等）。
  したがって`imm_cross_applicable`/`gji_direct_applicable`/`ms_ime_direct_applicable`/
  `ime_key_for`もLinuxでは存在せず、`ImeControlView`/`FocusFacts`/`ObservedState`/`ControlLog`
  もLinuxでコンパイル対象外。`characterize_strategy`（`ime_controller.rs:672`）・
  `first_applicable_name`（`:642`）を含む「決定だけ見るシーム」一式は**丸ごとwindows-only**
  であり、`tests/ime_key_sequence_golden.rs`がwindows-onlyなのはこの構造の必然的な帰結。
- gatedになっている直接の原因は**`ActiveImeKind`（`tsf/observer.rs:633`、`pub(crate) enum`）
  1点**だけである。`AppImeProfile`（`focus/class_names.rs:135`）自体はungated
  （`focus/mod.rs:8`）。ADR-089 §2.8が`ImeKindId`という`ActiveImeKind`のungatedミラー型
  （`state/ime_kind.rs:19`）と`From<ActiveImeKind>`変換（`tsf/observer.rs:643`）を既に
  用意しており、`ime_controller.rs:610-613`の`caps(view.focus.profile.into(), ...)`が
  これを使っている——つまり「windows-gated観測型→ungated決定入力型」変換の前例は既にある。
  `key_sequence_policy`の述語だけがこの前例に揃わず`ActiveImeKind`を直接取っている。

**この訂正が設計に与える影響**: 当初のPart A案（`ImeControlView`をそのまま受け取る
`decide_mechanism_command`をungated `state/`に新設する）は、`ImeControlView`自体を
ungate化する工事を暗黙に要求しており、これは本ADRが「検討した代替案（棄却）」として
明示的に退けている**「`ImeControlView`ごとPure/Impureに分割再設計する」そのもの**に
なってしまう（round1 M1指摘）。この矛盾を解消するため、決定節ではPart Aの入力を
`ImeControlView`全体ではなく**決定に実際に効く値だけを持つ小さな構造体**に絞る
（下記「決定入力の最小化」）。

## 決定

新しい抽象を発明するのではなく、既存の分離線を境界として使う。3部構成
（Part C はround1 M7を受けて新設）。

### Part A: 決定入力の最小化 + ゲート込みの決定関数抽出

#### 決定入力の最小化（`ImeControlView`をungate化しない）

`ImeControlView`の全フィールドのうち、戦略選択・VK選択・already-matched判定に
実際に効くのは次の4値だけである（残りは診断ログ専用——`composition_active`/
`ime_show_seq`/`ime_change_seq`はADR-117診断ログのみ、`candidate_visible`/
`candidate_was_seen`はKanjiToggleのログのみ、`class_name`はwarnログのみ、
`focus_gen`はROMAN補完の対象捕獲専用）:

```rust
// state/ime_actuation_decision.rs（仮称、新規ungatedモジュール）
#[derive(Clone, Copy)]
pub(crate) struct DecisionInputs {
    pub profile: AppImeProfile,       // 既にungated
    pub kind: ImeKindId,              // ActiveImeKind の既存ungatedミラー（ADR-089 §2.8）
    pub shadow_on: Option<bool>,      // ControlLog.shadow_on
    pub belief_input_mode: InputModeState,
}
```

`ImeControlView`をまるごとungate化する必要がなくなるため、代替案として棄却した
「viewごと再設計」との衝突が解消する。副作用として、`DecisionInputs`（view由来の
部分）は`&'a str`のような借用フィールドを持たずに済み、シリアライズ/デシリアライズが
素直になる（round1 M8で指摘された`ImeControlView`の借用問題を設計レベルで回避する。
ただし`order`側の借用問題は別に残る——R2参照）。

**`ImeControlView`自体は消えない**（round2 T5）。`class_name`（warnログ用）・
`focus_gen`（ROMAN補完の対象捕獲用）・`composition_active`等（ADR-117診断ログ用）は
引き続きwindows側の`ImeControlView`が運ぶ。変わるのは「決定への入力経路」だけで、
windows側に`impl From<&ImeControlView<'_>> for DecisionInputs`（1関数）が生え、
`ime_controller.rs`/`open_chain.rs`はこの変換を通してからungatedな決定関数を呼ぶ。

`key_sequence_policy`の3述語のシグネチャを`ActiveImeKind`→`ImeKindId`へ変更するのが
cfg境界の変更の**1つ**（**Part AのStep 0**、TH1aより前に行う）。**もう1つ、
`MechanismCommand::SetOpenThenConvForTarget`が持つ`ConvAfterOpen`（`ime.rs:1404`、
`pub(crate) enum`、`crate::ime`ごと`#[cfg(windows)]`）も同じ理由でStep 0の対象に
加える**（round2 R1）。`ConvAfterOpen`をそのままungatedなenumのフィールドに使うと、
「ungatedモジュールがwindows-gated型を参照する」という、round1 M1で解消したはずの
構造を`MechanismCommand`側で再導入してしまう。`ImeKindId`と同じミラー型パターンに
揃え、`state/`側に`ConvAfterOpenId`（`Skip`/`WriteRomanOnly`/`WriteExact(u32)`相当）を
新設し、`From<ConvAfterOpenId> for ConvAfterOpen`を`ime.rs`に1箇所置く。
これにより`key_sequence_policy`・`MechanismCommand`の両方がungate化でき、
`ime_decision_view.rs`側は`DecisionInputs`への変換関数が生えるだけで済む
（`ImeControlView`自体はungate化しない）。

#### 決定関数の粒度: `run_chain`単体ではなく`apply`/`run_open_chain_async`相当に広げる

round1 M6で指摘された通り、実際に検出したい回帰の中心——ADR-119
（issue #136/BUG-90、新しいgateを1箇所にしか置かず二重actuationが再発した実例）——は
`run_chain`の**外側**（`ImeController::apply`冒頭のInputRelayチェック、
`open_chain.rs`3関数それぞれの独立したInputRelay再検出、`log_shadow_warrant`）で
起きる。`run_chain`だけを再生対象にすると、これらのゲートを1箇所削って回帰させても
検出できない。したがって決定関数の単位を機構1つ分ではなく、**`ImeController::apply`
1呼び出し全体**（および`run_open_chain_async`1呼び出し全体）に広げる:

**round2 R3で訂正**: 当初案は`decide_ime_open(order, inputs, chain) -> DecisionOutcome`
という1回の純粋呼び出しで`attempts: Vec<(WriteMechanism, MechanismCommand)>`を丸ごと
返す形にしていたが、これは成立しない——チェーンが次の機構へ進むかどうかは、前の機構の
**実`ImeOpenOutcome`**（Part Bで「外部入力として記録し再計算しない」と決めたもの）に
依存するため、純粋関数が事前にattemptsの列を確定して返すことは原理的にできない
（Part Aの関数シグネチャとPart Bのattempt単位記録が内部矛盾していた）。

決定関数は「ゲート」「chain選択」「1 attempt分の決定」の3つに分割し、列を組み立てる
のは決定関数ではなく**再生ドライバ側**（記録済みのper-attempt inputsと実outcomeを
順に食わせながら`decide_attempt`を呼ぶ）にする:

```rust
pub(crate) fn decide_gate(inputs: &DecisionInputs) -> GateResult;  // NotOwned(InputRelay) | Proceed

pub(crate) fn decide_chain(inputs: &DecisionInputs) -> &'static [WriteMechanism];  // sync専用

pub(crate) fn decide_attempt(
    inputs: &DecisionInputs,
    site: DecisionSite,  // round3 U1: ImmCross の3系統(sync/async untargeted/async targeted)を
                         // 選ぶには mechanism/open だけでは足りず、どの入口からの呼び出しかが要る
    mechanism: WriteMechanism,
    open: bool,
) -> (bool /* romaji_pre_write するか */, Option<MechanismCommand>);
```

**round3 U1**: 当初のシグネチャ（`inputs`+`mechanism`+`open`のみ）では、`MechanismCommand`の
ImmCross3系統のどれを選ぶかを決定できなかった——sync経路かasync経路か、
`ImmCrossOp::Targeted`/`Untargeted`のどちらかは`DecisionInputs`からは導出できず、
`executor.rs:810`の`imm_cross_is_first_applicable`分岐と`:877-881`の組み立てで決まる
（Part Aの必須スコープに含めた`conv_after_open`判定・`ImmCrossOp`組み立てそのもの）。
`decide_attempt`に`site: DecisionSite`引数を追加することでこれを解決する
（`AttemptRecord`は既に親レコード経由で`site`を持つため、記録側の変更は不要）。

本番経路（`ImeController::apply`/`run_open_chain_async`）側は、この3関数を
「gate判定→chain取得（syncのみ、asyncは`WriteMechanism::ALL`固定）→機構ごとに
attempt決定→実write→実outcomeを次のattemptへ」という既存の`run_chain`/
`run_chain_async`ループの中でそのまま呼ぶだけで、ループ構造自体は変えない。

`runtime/executor.rs::dispatch_ime_set_open`は当初「今後の議論」で先送りしていたが、
**round1 S4を受けて本ADRの必須スコープに格上げする**。理由: この関数は
(a) InputRelayゲート、(b) sync/async分岐（`imm_cross_is_first_applicable`）、
(c) `applied_snapshot`の楽観更新、(d) `focus_gen`捕獲、(e) `needs_romaji_pre_write`
**とは条件式が異なる第3のROMAN判定**（`executor.rs:851-856`、実際に
`send_ime_control(IMC_SETCONVERSIONMODE)`を発行する）、(f) `ImmCrossOp::Targeted`の
組み立て、を一手に持つ——ADR-159が「ゲート判定」と呼ぶものの実体はここであり、
ここを除外したままでは「送信列の再現」を名乗れない。

#### `MechanismCommand`はImmCrossの3系統・ROMAN補完・VK型を区別する

round1 M4指摘: 当初の3バリアント案は次を取りこぼしていた。

- `ImmCrossProcessStrategy`の書き込みは実際には3系統の別APIに分岐する:
  sync `set_ime_open_cross_process`（`ime_controller.rs:92`）／async untargeted
  `set_ime_open_cross_process_async`（`open_chain.rs:203`）／async targeted
  `set_ime_open_then_conv_for_target`（`open_chain.rs:187`、convも同時に書く）。
  1バリアントに潰すとこの3系統の違いが再生で区別できない。
- `apply_mechanism`が機構apply**の前**に呼ぶ`romaji_pre_write`
  （`ime_controller.rs:384`、実体は`send_ime_control(IMC_SETCONVERSIONMODE)`）が
  `MechanismCommand`に一切表現されていなかった。
- `SendVk(u16)`は生のu16ではなく`awase::types::VkCode`にする
  （CLAUDE.mdの「no raw VK-code magic numbers outside `crates/awase-vkmap`」、
  [[feedback_vk_encapsulation]]に抵触するため）。

```rust
pub(crate) enum MechanismCommand {
    SetOpenCrossProcessSync(bool),
    SetOpenCrossProcessAsyncUntargeted(bool),
    SetOpenThenConvForTarget { open: bool, conv_after_open: ConvAfterOpenId },  // R1: ungatedミラー
    SendVk(VkCode),
    PostKanjiToggle,
}
```

**round2 R4で訂正**: 当初`RomajiCommand::SetConversionMode { target_focus_gen: u32 }`
としていたが、決定関数の入力（`DecisionInputs`）に`focus_gen`を含めていないのに出力が
`target_focus_gen`を持つのは矛盾している。さらに`focus_gen`はフォーカス変更のたびに
変わる値であり、コーパスに焼き込むとフィクスチャが脆くなる。**「ROMANを書くか否か」の
判定だけをungated側の決定にし、`focus_gen`の捕獲・対象への適用はI/O側（②）に残す**
——実コードでも`romaji_pre_write`は`view.focus.focus_gen`を読んで
`ActuationTarget::capture_blocking(focus_gen)`に渡しており（`ime_controller.rs:460-469`）、
この読み取りタイミング自体が実I/O側の関心事である。したがって`decide_attempt`が返す
1つ目の要素は`RomajiCommand`という別型ではなく単純な`bool`（`needs_romaji_pre_write`の
戻り値そのもの）でよい。

### Part B: attempt単位の決定点ジャーナルと凍結コーパス

#### スキーマはsite単位ではなくattempt単位

round1 M3指摘: `fallback_write`（`open_chain.rs:299-348`）は機構ごとに
`shadow_ime_control_view()`を作り直す——ADR-159が「`.await`をまたぐ再サンプリングは
1点に畳まない」と決めた3時刻のサンプリングそのもの。`site`単位でview 1件を記録すると
この差が記録から消える。**view（`DecisionInputs`）はattempt単位で記録する。**

また以下はすべて「決定ロジックからは導出不能な外部入力」としてattemptごとに
記録する（round1 S3・S6で一般化）:

- `with_app(...).unwrap_or(false)`のfail-open結果（`open_chain.rs:154-161,300,379-386`）
- 各機構writeの実`ImeOpenOutcome`——ImmCrossのタイムアウト成否だけでなく、
  `send_ime_mode_key`がWinキー押下中に返す`false`（→`UnsafeToToggle`、BUG-16追補、
  GjiDirect/MsImeDirect両方に存在）も同性質の外部入力である
- `ActuationOutcome::Failed`後に`read_ime_state_fast()`を呼んで`AlreadyMatched`/
  `Failed`を分ける追加観測（`open_chain.rs:228-248`）
- `view.control.shadow_on`へのBUG-113追補上書き（`open_chain.rs:329`、決定入力の
  改変であり決定そのものではない——記録は上書き後の値でよいが、上書きが発生した
  事実自体と上書き前の値は別フィールドで残す）
- 使用したchain。**sync/asyncで再生時の扱いを変える**（round2 T2）: asyncは
  `WriteMechanism::ALL`固定（ADR-159の理由により変更しない）なので記録値をそのまま
  使う。**syncは記録値をそのまま信用せず、再生時に`decide_chain(inputs)`で
  再導出し、記録済みchainと一致するかもassertする**——記録値をそのまま流用するだけだと
  `caps()`（`state/app_ime_policy.rs`）の回帰を再生が素通りしてしまうため。

```rust
pub(crate) struct ActuationDecisionRecord {
    // round3 U2: DispatchImeSetOpen を追加。executor.rs::dispatch_ime_set_open の
    // InputRelay ゲートは ImeController::apply（Sync）とも run_open_chain_async とも
    // 別の独立した5つ目の入口（両者が独立に同じ判定を持つのがADR-119の経緯そのもの）。
    // Sync に畳むと、executor側のゲートだけを削る回帰が記録上区別できなくなる。
    pub site: DecisionSite,  // Sync | ImmCrossWrite | FallbackWrite | RunOpenChainAsync |
                             // DispatchImeSetOpen | ReassertExplicitPhysicalKey |
                             // ForceOnRomajiCorrection
    pub gate_inputs: DecisionInputs,
    pub order: ActuationOrderRecord,
    pub chain: [Option<WriteMechanism>; 4],
    pub chain_len: usize,     // syncは decide_chain 再導出との一致もassertする
    pub attempts: [Option<AttemptRecord>; 4],
    pub attempts_len: usize,
}
pub(crate) struct AttemptRecord {
    pub inputs: DecisionInputs,         // この attempt 時点の再サンプリング値
    pub with_app_available: bool,
    pub mechanism: WriteMechanism,
    pub command: Option<MechanismCommand>,  // None = already-matched で送信せず
    pub outcome: ImeOpenOutcome,             // 外部入力として記録、再計算しない
    // BUG-113追補上書き前の`view.control.shadow_on`。外側Noneは「上書きなし」、
    // Some(None)は「上書き前の値が未知」。
    pub shadow_on_before_bug113_override: Option<Option<bool>>,
    // round2 T3: 「未取得」と「取得してfalse」を区別する（BUG-113と同型の罠を
    // ここで再現しないため）。実体は read_ime_state_fast().ime_on の Option<bool> を
    // そのまま運ぶ。Option<bool> に潰さないこと。
    pub post_failed_reobservation: Option<Option<bool>>,
}
```

**`ActuationOrderRecord`の借用問題（round2 R2）**: `order`側は`DecisionInputs`とは
別の壁に当たる。`ActuationOrder.origin: EventOrigin` → `EventOrigin.source: EventSource`
の`Injected { reason: &'static str }` / `SelfActuated { strategy: &'static str }`
（`event_origin.rs:125,129`）は、`event_origin.rs`自身が「`&'static str`のため、
任意入力から借用を復元するDeserializeは型として表現できない」と明記している壁で、
`DecisionInputs`側の借用回避（R1で解消済み）とは独立に残る。**`ime_actuation.rs:283-286`
の`ActuationRecord`が採る回避策——`strategy`文字列そのものは保存せず、`policy`等の
判別子情報から必要なら再構築する——と同型の扱いに揃える**: `ActuationOrderRecord`は
`EventSource`の`&'static str`を含む文字列フィールドを一切保存せず、`EventSource`の
**判別子（`&'static str`を含まない別enum）**・`epoch`・`open`・
`would_have_blocked`（A-1 shadow authorization、`log_shadow_warrant`が使う値）だけを
保存する。

#### コーパスの置き場所と運用は既存前例（`drift_correction_replay.rs`）を踏襲する

round1 M8指摘: `JournalEntry`自体は`Serialize`専用で`Deserialize`不可
（`journal.rs`の`origin: &'static str`等が理由）。**`tests/drift_correction_replay.rs`
が既にこの壁を回避する前例を持つ**——gatedな`journal`モジュールを迂回し、payload型
（`ActuationRecord`相当）をungatedな`state/`側に定義してLinuxで再生する。本ADRの
Part Bはこのパターンをそのまま踏襲する（当初案がこの前例に言及していなかった点を
是正）。

- `ActuationDecisionRecord`等は`state/`側のungatedなfixture専用型として定義し、
  `journal.rs::JournalEntry`とは別に持つ（実機ダンプ→手動転記、という既存運用も踏襲）。
- フィクスチャは`tests/journals/actuation_decision/`という**専用サブディレクトリ**に
  置く。`journal_replay.rs`の`read_dir`が`tests/journals/`直下の全`*.json`を
  `ConvClassifyFixture`としてパースする非再帰実装であり（`drift_correction_replay.rs`
  も同じ衝突を記録済み）、直下に置くと既存テストのパースに巻き込まれる。
- `journal_replay.rs`の運用規約（転記直後の値は「実際に起きたバグの出力」であり、
  必ず手で「あるべき出力」に書き換えてからコミットする）を踏襲する。
- **記録と再生が同一バイナリ・同一関数だと`run_chain`部分は恒真テストになる
  （round1 M8最重要指摘）**。価値が出るのは「現行コードで記録→コーパスとして凍結→
  リファクタ後のコードで再生」という使い方であり、これを`docs/journal-replay-guide.md`
  相当のガイドに明記する。

#### 可視性: crate内`#[cfg(test)]`として置く（外部integration testにしない）

round1 M9指摘: `ImeControlView`/`ImeController`/`ActiveImeKind`等はいずれも
`pub(crate)`で、外部crate扱いの`tests/*.rs`integration testからは参照できない
（`characterize_strategy`が生のプリミティブ引数を取る形になっているのも、この
可視性の壁が実際の理由——ADR当初案は「副作用なしで観測するため」とだけ理解しており
誤りだった）。再生ハーネスは`state::actuation_chain::tests`と同じパターンで
**crate内の`#[cfg(test)] mod`**として置く。

### Part C（round1 M7で新設）: TH1発効に向けた実削除タスク（TH1e）

[complexity-budget.md](../../.claude/rules/complexity-budget.md)「発効条件」
（ADR-162 round4 TJ4 M5訂正）は能力ベースであり、**「実際の削除・統合を1件、N本の
記録トレースの再生で送信列差分ゼロと検証できたこと」**を要求する。本ADRのPart A/Bは
ハーネスを用意するだけでこの条件を満たさない。

**round2 R5でTH1eの対象を差し替えた**: 当初案は`characterize_strategy`
（`ime_controller.rs:672`）・`first_applicable_name`（`:642`）・
`first_applicable_name_skipping_imm`（`:651`）の3シーム統合をTH1eの証明対象に
挙げていたが、この3つは`is_applicable`しか評価せず**実writeを一切起こさない
読み取り専用シーム**であり、`ime_controller.rs:622-629`自身が「本番経路
（`ImeController::apply`/`runtime/open_chain.rs`）からは参照されない」と明記している。
これを統合しても`attempts`列は定義上変化しないため「差分ゼロ」は自明に成立してしまい、
**TH1発効条件が要求する「ハーネスが実際に削除の安全性を判定した」証拠にならない**
（空証明）。

**TH1eの対象は`AsyncChainWriter::is_applicable`の定数`true`実装の統合とする**
（`open_chain.rs:103-113`）。現状、async側の`is_applicable`は常に`true`を返し、
実際の適用可否判定は`write`内部で行って不適用なら`Failed`に写して`run_chain_async`の
フォールスルーへ委ねている（`:338-347`）。これを`SyncChainWriter`と同じ
「`is_applicable`で絞る」形に統合する変更は、(i) actuation経路上にあり、
(ii) 削除で送信列が変わりうるが実際には変わらないと示せる、という2条件を満たす
——`open_chain.rs:31-33`のモジュールdoc自身が既に同値性の根拠を明文で持っている
（「本実装は『適用不能ならFailedを返す』形にしているが、Failedは必ずフォールスルー
するため走査結果は同一である」）。実機（MWB環境）が必要なInputRelay 3箇所の統合
（ADR-159「実機スパイク結果」節）より着手コストが低く、本ADRのPart A/Bだけで
検証を完結できる。

**round3 U3（注記）**: `AsyncChainWriter::is_applicable`が定数`true`なのは怠慢ではなく、
`open_chain.rs:107-111`が「実行時の`ImeControlView`を見ないと判断できない（viewの構築に
`with_app`が要る）」と理由を明記している。`SyncChainWriter`と同じ「`is_applicable`で
絞る」形に統合するには、`is_applicable`内で`with_app`を呼ぶ必要があり、**`with_app`が
呼ばれるタイミングが`.await`に対して変わる**（再入時に`None`を返すfail-open分岐が
1つ増えうる）。上記の同値性の根拠は「走査結果は同一」までしか保証しておらず、この
タイミング変化までは対象外——TH1eの「差分ゼロ」検証対象は走査結果（`attempts`列）と
送信列（`MechanismCommand`列）であり、`with_app`再入の頻度変化そのものは
`AttemptRecord.with_app_available`フィールドで別途観測する（差分ゼロの必須条件には
含めない）。

`needs_romaji_pre_write`と`executor.rs:851-856`の第3のROMAN判定の統合は、次点候補
として記録するに留める——条件式が実際に異なる（後者は mechanism 条件も
`kind==MsIme` 条件も持たない）ため差分ゼロにはならない公算が高く、TH1eの証明対象
としては不適切。統合を試みて実際に差分が出れば、それは「本ADRのハーネスが最初に
検出した実回帰」として基盤の有効性そのものの証拠になる（TH1eとは別のタスクとして
「今後の議論」に記録）。

3シーム統合（`characterize_strategy`等）はTH1eの証明対象からは外すが、Part Cの
**副次タスク**（ハーネスのスモークテスト+複雑性予算の返済、round1 S7が本来
意図していた位置づけ）としてPart Cに残す。この統合には**golden移行**が伴う
（round2 R6）: `tests/ime_key_sequence_golden.rs`は`characterize_strategy`
（`:34`使用、表生成ループ`:135`と個別assert計8箇所が依存）を通じて
`tests/golden/ime_key_sequences.txt`（戦略選択テーブル12行、GJI/MS-IME×3profile×
apply/async_fallback）というfix-requires-evidence.mdが名指しする再発防止資産の
SSOTになっている。この資産を落とさないため、**3シーム統合と同時に、`decide_chain`/
`decide_attempt`ベースで同じ表を再生成し、crate内`#[cfg(test)]`（Part BのM9方針）へ
移すこと**を明記する。これにより golden 自体が「Linux で走るようになる」という
副次効果も得られる（背景節が問題視した「決定だけ見るシームがwindows-gatedファイルに
閉じ込められている」ことの直接的な解消）。

### Part D（round4実施済み・反映済み、TH1d'として新設）: journal相乗りによるbug report経由の実機コーパス収集

**発端**: ユーザーから「bug report機能にmetrics/tracing情報を含め、操作のリプレイや
不要ロジックの調査に役立てたい」という要望。評価基準はユーザー自身が明示した2点——
**①撤去すべき処理の特定に役立つこと、②不具合の根本原因特定に役立つこと**——であり、
bug reportに含める価値は「母集団N（実機からの記録本数）を増やせること」にある
（ユーザー指示、2026-09-11：「母集団nの数を増やせるからbug reportに含めることは
やりたい」）。

**round4での全面書き直し**: 初版（2026-09-11、Blocker7件・Should-fix9件、
opus-adversarial-consult 1ラウンド）は「別枠の新規リングバッファ＋bug_reportへの
新規`attach_X`フィールド」という設計だったが、以下が判明し決定を全面的に書き直した:

- **B7**: Part Bが前例として引用した`drift_correction_replay.rs`が実際に踏襲している
  構成は「ungatedな`pub`型を`state/`に置き`JournalEntry`のvariantとして積む」
  （`journal.rs:262-264`の`JournalEntry::ImeActuation { record: ActuationRecord }`、
  `state/ime_actuation.rs`）であり、初版が提案した「別枠リングバッファ＋新規bug_report
  フィールド」とは逆方向だった。
- **B1**: `ActuationDecisionRecord`等を`pub(crate)`のままにすると`BugReportPayload`
  （`pub`型）のフィールドに置けずE0446でコンパイルが通らない。`#[cfg(windows)]`を
  足すと`mod tests`ごとLinuxビルドから消える。
- **B2**: bug reportは`awase.exe`と`awase-settings.exe`の別プロセス間をファイル経由で
  受け渡す構成（`message_handlers.rs`→JSON書き出し→`crates/awase-settings/src/bug_report.rs`
  が読み込み）であり、`bug_report.rs`への型追加だけでは配線されない。
- **B3**: サーバ側`services/report-worker/src/index.ts::validatePayload`は未知
  フィールドを無言で捨てる設計であり、Worker先行デプロイ無しでは新フィールドが
  400にすらならず欠落する。
- **B4**: `ActuationDecisionRecord`が持つ`chain: Vec<WriteMechanism>`/
  `attempts: Vec<AttemptRecord>`は、ADR自身がTF2再開条件として明記した
  「actuationのホットパスにロック・ヒープ確保・キュー操作を足さないことを示す」
  （後述「TF2との突合せ」節参照）に抵触する。
- **B5**: `journal_replay.rs`の凍結運用（「あるべき出力」に手で書き換えてから
  コミット）を転用できない——`ActuationDecisionRecord.command`の「あるべき値」を
  人間が独立に導く情報源が存在しない（`decide_attempt`の実装自体が唯一の情報源）。
- **B6**: `decide_chain`/`decide_attempt`の実配線は`ime_controller.rs:602`/`:241`の
  各1箇所のみで、後者は`DecisionSite::Sync`固定。非同期ImmCross attemptは
  `decide_attempt`が構造的に`None`を返し再生で`continue`（skip）されるため、
  初版が主張した「母数を増やせば検証範囲が広がる」は成立しない。

（レビュー全文はセッション記録参照。ここでは反映結果のみ記す。）

#### 決定D1: 型はjournal.rsの前例に揃える——ungatedなpub型+`JournalEntry` variant（B7対応）

`ActuationDecisionRecord`/`AttemptRecord`等（現行`state/mod.rs:98`で
`#[cfg(test)] pub(crate) mod actuation_decision_record;`）を、
`state/ime_actuation.rs::ActuationRecord`と同じ構成に揃える:

- 型定義をungatedな`pub`型として`state/actuation_decision_record.rs`に置く
  （`#[cfg(windows)]`も`#[cfg(test)]`も付けない）。
- `journal.rs::JournalEntry`に新variant`ActuationDecision { record: ActuationDecisionRecord }`
  を追加する。
- 再生ハーネス（`mod tests`、TH1c実装済み）はcrate内`#[cfg(test)]`のまま維持する——
  Part BのM9決定が守ろうとしたのは「外部crate扱いの`tests/*.rs`から`pub(crate)`型が
  見えない」問題であり、`ActuationRecord`の前例どおり型をungated `pub`にした上で
  再生テストだけを`#[cfg(test)]`に残せば両立する（M9への抵触なし、B1解消）。
- **`bug_report.rs`側の変更は不要**: `journal_json`は既に`attach_log`経由で
  `JournalEntry`群をシリアライズして送っており、新variantを追加するだけで既存経路に
  自動的に乗る。初版決定3（`attach_actuation_decisions`+`BugReportActuationDecisions`
  の新設）・決定4（MAX_BODY_BYTES新規予算設計）は撤回する（B1・B2・B3・S1・S7・S8
  が同時に解消する）。ADR-095決定3（B-5、allowlist原則）が求める4層ミラー型の新設も
  不要になる。サーバ側`report-worker`の変更も不要（journal_jsonは既存の文字列
  フィールドとしてそのままR2へ保存される）。
- lane配置は既存`LaneKind::Actuation`（capacity=512、`ImeActuation`等と共有）への
  相乗りか、専用の新規`LaneKind`を切るかを実装時に決める。共有する場合は既存エントリの
  枠を食う実測影響を、専用lane を切る場合は容量の実測根拠
  （[tuning-constants](../../.claude/rules/tuning-constants.md)の精神）を、
  それぞれコミット本文に残すこと——**ここでの数値の決め打ちはしない**。

#### 決定D2: `chain`/`attempts`はVecではなく固定長配列にする（B4対応、TF2再開条件を満たす）

`WriteMechanism::ALL`は`[Self; 4]`（`state/actuation_chain.rs:162`）で最大4機構固定
のため、`chain`/`attempts`を`Vec`ではなく`[Option<_>; 4]`+使用数のような固定長表現に
変更しヒープ確保を排除する。`AttemptRecord`自体も可能な限り`Copy`にできる構成を
優先する。これにより「TF2との突合せ」節（後述）が定める再開条件（ホットパスに
ロック・ヒープ確保・キュー操作を足さないことを示す）を満たす。

#### 決定D3: このコーパスはcharacterization corpus——「あるべき出力」への書き換えは行わない（B5対応）

初版決定5（`journal_replay.rs`と同じ「あるべき出力に書き換えてから凍結」）は撤回する。
bug report経由で集まったレコードは「**凍結時点の実際の出力をそのまま記録する**」
characterization corpusとして扱う。用途は2つ、いずれもユーザーが示した評価基準に
直接対応する:

1. **不具合の根本原因特定**（評価基準②）: 人間が個別のbug reportを見て「このケースで
   どのmechanism/commandが選ばれ、どんな`ImeOpenOutcome`が返ったか」を読む診断材料
   として、TH1e/自動再生の完成を待たずに**今すぐ**使える。
2. **撤去すべき処理の特定**（評価基準①、TH1e）: 「現行コードでの記録」と
   「リファクタ後コードでの再生」を比較し送信列（attempts/command列）の差分ゼロを
   機械的に確認する。この用途では「あるべき出力」の正しさは不要——比較対象は常に
   「変更前のコード自身が出した値」であり、それがそもそも正しいかは問わない。

#### 決定D4: 収集はTH1eを待たず今から始める。非同期ImmCross経路は当面「死蔵」であることを明記し、バイナリバージョンをスタンプする（B6対応）

`decide_chain`/`decide_attempt`の配線がSync経路1箇所に限られる現状では、非同期
ImmCrossのattemptは再生で`continue`（skip）される（TH1eのスコープ）。ユーザー指示
（「母集団Nを増やせるから含めたい」）を優先し、**収集はTH1eの完了を待たずに今から
始める**：

- `ActuationDecisionRecord`ごとの`app_version: String`は持たせない。バージョンは
  journal dumpの`DumpTruncated`ヘッダに1回だけ持たせる（B4対応）。TH1eが着地した
  時点で、それ以前に集まった非同期ImmCrossレコードを遡って再生できるようにするため。
- 「非同期ImmCross系のattemptはTH1e完了まで決定D3用途2（自動差分証明）の対象外だが、
  用途1（人間による根本原因特定）では収集開始時点から有効」という限定を本ADRと
  `docs/journal-replay-guide.md`相当のガイドに明記する。

#### 決定D5: `DecisionSite`に2バリアントを追加し、スコープ外2経路の混入を避ける（S5対応）

`fix-requires-evidence.md`表が5番目・6番目の独立入口として挙げる
`runtime/mod.rs::reassert_explicit_physical_key`/`force_on_and_correct_romaji`
（本ADR「検出できる回帰・できない回帰」節が「本ハーネスでは検出できない」と明記した
経路）は、現状`ImeController::apply`経由で`site: Sync`として記録され他のSyncレコードと
区別がつかない。`DecisionSite`に`ReassertExplicitPhysicalKey`/
`ForceOnRomajiCorrection`を追加し、TH1eの差分ゼロ検証の母数にスコープ外経路が無自覚に
混入しないようにする。

#### 決定D6: `with_app`再入時の記録漏れは別カウンタで可視化する（解消はしない、S4対応）

`UnifiedJournal`は`PlatformState`（`with_app`経由でのみ到達可能）の中にあるため、
journal相乗り方式でも`run_open_chain_async`/`imm_cross_write`のfail-open再入時
（`with_app`が`None`）は記録できない——これは記録したい`with_app_available: false`の
ケースそのものが記録から漏れるという構造的な限界。再入自体を解決しようとはせず、
既存の`dropped_by_lane`と同じパターンで「再入により記録をスキップした回数」を
カウンタとして残す。

#### 決定D7: `AttemptRecord`にBUG-113上書き前の値を追加する（S2対応、Part Dの前提条件）

Part B「attempt単位の決定点ジャーナルと凍結コーパス」節が「上書きが発生した事実自体は
別フィールドで残す」と約束していたのに、現行`AttemptRecord`に該当フィールドが無い
（TH1cの実装漏れ）。Part D着手前に、`fallback_write`が`view.control.shadow_on`を
上書きする直前の値を`AttemptRecord`に追加する。

#### 決定D8: `DecisionInputs`のdoc commentにドリフト防止の警告を追加する（S6対応）

プライバシー境界についてはround4レビューで実コード（`DecisionInputs`/`AttemptRecord`/
`MechanismCommand`/`EventSourceKind`の全フィールド）を確認した上で「打鍵の生の文字・
ローマ字・かなは含まれず、新しい境界を動かす話にはならない」と確認済み（ユーザー
指摘どおり）。ただし将来「診断のため`class_name`も入れよう」のような1行が入ると
アプリ名（ユーザーが何のソフトを使っているか）の送信チャネルに変質するリスクがある。
ADR-148が`BugReportGjiKeymapSummary`のdocに残した同種の警告（allowlist原則を素通り
する変更への注意）を、`DecisionInputs`のdoc comment（`state/ime_actuation_decision.rs`、
既に`class_name`除外理由が書かれている箇所）に追記する。

#### タスク名: TH1dは維持、本Part Dは並行タスク「TH1d'」とする（S9対応）

TH1d（`tests/journals/actuation_decision/`へ既知バグ由来のfixtureを最低1本手で投入し、
`replay_all_actuation_decision_fixtures`の「ディレクトリが存在しない間は黙って通す」
ガードを`assert!(total > 0)`相当に強化するタスク）は**そのまま残す**——bug report由来の
レコードは決定D3の通りcharacterization専用であり、「意図した不一致を実際に検出できる」
ことを示す最初の1本（fixtureファイルとして、ユニットテスト内の手組みレコード
`replay_detects_a_tampered_command`とは別に）は依然として手組みが要る。本Part Dは
**TH1d'**として、TH1d/TH1eと並行に進める自動コーパス成長タスクと位置づける。

**TH1e・複雑性予算制との関係**: 決定D4のとおり、このコーパスは収集開始時点から
評価基準②（根本原因特定）に使え、TH1e完了後は評価基準①（撤去すべき処理の特定）にも
使える。[complexity-budget.md](../../.claude/rules/complexity-budget.md)の発効条件
（「実際の削除・統合を1件、N本の記録トレースの再生で送信列差分ゼロと検証できたこと」）
が要求する「N本の記録トレース」の供給源になりうるが、発効条件を満たすのはあくまで
「実削除+差分ゼロ証明を1件やり切ったとき」であり、コーパスを集めただけでは満たさない。

## 検出できる回帰・できない回帰（round1 M6を受けて明記）

- **検出できる**: `is_applicable`判定の変更によるチェーンの早期終了・別機構への
  分岐（`decide_gate`/`decide_chain`/`decide_attempt`が本物の述語を呼ぶため）。
  InputRelayゲート・ROMAN補完条件・`ImmCrossOp`選択の変更（Part Aのスコープに
  `executor.rs::dispatch_ime_set_open`を含めたため）。**VK値**そのものの変更
  （`MechanismCommand::SendVk(VkCode)`の比較）。**round2 T1で限定**: `cmd`値
  （`IMC_SETOPENSTATUS`/`IMC_SETCONVERSIONMODE`の実際のバイト値）はTF2突合せが
  スコープ外（M5）のため検出対象外——検出できるのは`MechanismCommand`の
  variant種別（open操作かconv操作か）レベルまでで、`imm::send_ime_control`内部が
  組み立てる実際のcmd/lparamバイト値の一致は保証しない。
- **検出できない**: `fix-requires-evidence.md`「IME actuation合流点」表が挙げる
  残り2箇所——`runtime/mod.rs::reassert_explicit_physical_key`・
  `runtime/mod.rs::force_on_and_correct_romaji`——は本ADRのスコープ外であり、
  これらにだけ新しいgateが追加される／既存gateが欠落する形の回帰は、本ハーネスでは
  検出できない。ADR-119型の回帰は「本ADRが対象とする4箇所（`ImeController::apply`・
  `open_chain.rs`3関数・`dispatch_ime_set_open`）の間」でのみ再現・検出できる。

## TF2（`[shadow-send]`ログ）との突合せは将来課題に降格する（round1 M5）

当初案の「Part Bの再生結果とTF2の`[shadow-send]`ログを突き合わせる」は、TF2の
現在の実装（`shadow_send_trace.rs:32`のdocが明記する通り`tracing::debug!`1行のみで
蓄積・保持を一切行わない、`/code-review`指摘で意図的に撤回済み）では実行不能。
本ADRでは**この突合せをスコープから外し、将来の拡張として明記するに留める**。
再開する場合の条件: `journal.rs`の`JournalLane`/`LaneKind::Actuation`（既存の
bounded-buffer機構）へ合流させる設計を別途詰め、TF2が一度撤回した「actuationの
ホットパスにロック・ヒープ確保・キュー操作を足す」問題を再燃させないことを示すこと。

## この設計が対象にしないもの

- **belief/observation層の再生**（`ImeModel::reduce()`・focus tracking・
  `ObservationSource`11バリアント）は対象外。既存の`journal_replay.rs`
  （`replay_ime_apply_focus_epoch_fixtures`）が別レイヤーとして担う。
- **`runtime/mod.rs::reassert_explicit_physical_key`・`force_on_and_correct_romaji`**
  （fix-requires-evidence.md表の残り2箇所）は未調査のまま。「検出できない回帰」節参照。
- **classify_key/classify_ime_snapshotのungate化**（ユーザーが別途提起した案）は
  本ADRと並行して進めてよいが前提条件ではない——journal_replay.rsと同型の末端
  フィクスチャ再生の横展開であり、本ADRが対象とするactuation合流点の再生とは独立。

## 検討した代替案

### 代替案（棄却）: `ImeControlView`ごとPure/Impureに分割再設計する

「windows-gated観測型を全部ungated化する」野心的な再設計は、ADR-159が当初案A0で
辿った失敗（「新しい境界を設計する」→実コードと合わずround1で反証）を繰り返す
リスクが高い。本ADRは「決定入力の最小化」（`DecisionInputs`4値）でこの代替案を
避けつつ、実質的に同じ効果（ungatedな決定関数）を得る。

### 代替案（棄却）: `ImeOpenOutcome`も含めて全部を決定ロジック側で再計算する

`ImmCrossProcessStrategy`の成否や`send_ime_mode_key`のWinキー競合結果は実時間・
実OS状態依存であり、決定ロジックからは原理的に導出不能な外部入力である。これを
無理に「再現可能な決定」として扱うと、再生ハーネスが「実際には起きていない成功/失敗」
を捏造することになり、`.claude/rules/ime-belief-architecture.md`の禁止パターン2
（観測を偽装する）と同型の誤りを再生基盤側で犯すことになる。記録された結果を外部入力
として与える設計はこれを避ける。

## round1・round2での指摘と反映（要約）

**round1**（Must-fix 9件・Should-fix 8件）:

- M1（`key_sequence_policy`/`ime_decision_view`は実際はwindows-gated）→「既存の
  分離状況」節を訂正、Part Aを「決定入力の最小化」へ再設計
- M2（`ActiveImeKind`のみgated、`ImeKindId`ミラー型が既存）→ Part A Step 0として明記
- M3（記録スキーマがsite単位でasyncの3点再サンプリングを潰す）→ attempt単位に変更
- M4（`MechanismCommand`がROMAN補完・ImmCross3系統・生VKを取りこぼす）→ enum再設計
- M5（TF2は現状ログを蓄積せず突合せ不能）→ 将来課題に降格
- M6（再生入口が`run_chain`ではゲート回帰を検出できない）→ `apply`/
  `run_open_chain_async`粒度に拡大、`executor.rs::dispatch_ime_set_open`を必須化
- M7（TH1発効条件を満たさない）→ Part C（TH1e）を新設、ステータス節の主張を修正
- M8（コーパス凍結設計が無く恒真テスト化するリスク）→ `drift_correction_replay.rs`
  前例の踏襲、サブディレクトリ運用、手動curationの明記
- M9（`pub(crate)`型ばかりでintegration testから見えない）→ crate内`#[cfg(test)]`化
- S1〜S8 → 決定入力4値案（S2）採用、`open_chain.rs`のPart A適用範囲の限定（S3）、
  `executor.rs`必須化（S4）、chain非対称の明記（S5）、outcome外部入力の一般化（S6）、
  TH1e対象選定（S7）、行番号等の軽微な修正（S8）

**round2**（Must-fix 6件・Should-fix 5件、round1反映後の決定節を再読して発見）:

- R1（`ConvAfterOpen`がwindows-gatedでM1の構造をPart Aが再導入していた）→
  `ConvAfterOpenId`ミラー型を新設、Step 0の対象に追加
- R2（`ActuationOrderRecord`が`EventSource`の`&'static str`の壁に当たる）→
  `ime_actuation.rs::ActuationRecord`の回避策（文字列を保存せず判別子のみ保存）に揃えた
- R3（`decide_ime_open`のシグネチャとattempt単位記録が内部矛盾、純粋関数が事前に
  outcome依存のattempts列を返せない）→ `decide_gate`/`decide_chain`/`decide_attempt`
  への分割、列の組み立ては再生ドライバ側に移した
- R4（`RomajiCommand`の`target_focus_gen`が決定入力に無い値を出力する矛盾）→
  ペイロードなしの`bool`に単純化、`focus_gen`はI/O側に残す
- R5（TH1eの対象=3シーム統合は実writeを起こさず差分ゼロが自明で空証明）→
  `AsyncChainWriter::is_applicable`統合に差し替え、3シーム統合は副次タスクへ降格
- R6（Part Cが`characterize_strategy`を消すとgoldenが壊れる、移行先が未定義）→
  `decide_chain`/`decide_attempt`ベースでの表再生成+crate内`#[cfg(test)]`化を明記
- T1（VK/cmd値一致の主張がTF2降格と矛盾）→ 検出範囲をVK値のみ・cmd種別レベルに限定
- T2（sync chainを記録値のみで再生するとcaps回帰を素通り）→ `decide_chain`再導出+
  記録値との一致assertを追加
- T3（`post_failed_reobservation: Option<bool>`がBUG-113型の「未知」潰しを再現）→
  `Option<Option<bool>>`に訂正
- T4（`architecture_guard.rs`の件数ガード更新が計画に無い）→ 今後の議論に追記
- T5（`ImeControlView`が消える誤読）→「`ImeControlView`は残り、決定への入力経路だけ
  `DecisionInputs`に置き換わる」と明記

**round3**（U1〜U3、round2反映後の再読で発見。round3で「収束」と判定、round4は不要）:

- U1（`decide_attempt`のシグネチャではImmCrossの3系統を選べない）→ `site`引数を追加
- U2（`DecisionSite`に`executor.rs::dispatch_ime_set_open`用の入口が無い）→
  `DispatchImeSetOpen`バリアントを追加
- U3（`AsyncChainWriter::is_applicable`統合には`with_app`再入タイミングという
  別論点がある、Should-fix）→ TH1eの「差分ゼロ」対象を走査結果/送信列に限定し、
  `with_app`再入頻度は`with_app_available`で別途観測する旨を注記

## 今後の議論

1. `runtime/mod.rs::reassert_explicit_physical_key`・`force_on_and_correct_romaji`
   （fix-requires-evidence.md表の残り2箇所）が同型の分離を適用できるかを別途調査する
   （本ADRのスコープ外、「検出できない回帰」節参照）。
2. TF2の`[shadow-send]`ログを`journal.rs`の`JournalLane`へ合流させる設計
   （「TF2との突合せは将来課題」節の再開条件）。
3. 実装順序案（TF系タスクの命名規約に揃える）: TH1a（Step 0、`key_sequence_policy`の
   `ImeKindId`化+`ConvAfterOpen`の`ConvAfterOpenId`化）→ TH1b（Part A、
   `decide_gate`/`decide_chain`/`decide_attempt`+`DecisionInputs`+`MechanismCommand`の
   新設、`ime_controller.rs`/`executor.rs`/`open_chain.rs`への配線。**round2 T4:
   このタスクに`tests/architecture_guard.rs`の件数ガード更新を含める**——
   `raw_mechanism_write_sites_are_confined_to_chain_writers`（`apply_mechanism(`呼び出し元
   件数）・`.apply_ime_open_with_view(`件数（4）は配線変更で動きうるため、期待値更新と
   想定外の増加が無いことの確認を行う）→ TH1c（Part B、`ActuationDecisionRecord`
   スキーマ+crate内再生ハーネス）→ TH1d（既知バグ由来fixture1本の手動投入、
   `assert!(total > 0)`ガード）と**TH1d'（Part D、journal相乗りによるbug report経由の
   実機コーパス自動収集、決定D1〜D8。round4のopus-adversarial-consultで全面書き直し
   済み）は並行**→ TH1e（Part C、`AsyncChainWriter::is_applicable`統合+差分ゼロ再生証明、
   TH1発効条件の充足。TH1d'決定D4により非同期ImmCross系レコードはTH1e完了時点で
   遡って再生対象になる）→ Part C副次タスク（3シーム統合+golden移行、複雑性予算の返済）。
   TH1a〜TH1cは2026-09-10にPR#195/#196でdevelopマージ済み（詳細はステータス節）。
4. **round2 R5の次点候補**: `needs_romaji_pre_write`と`executor.rs:851-856`の
   第3のROMAN判定をSSOTへ統合する変更を、TH1e完了後に試す。条件式が実際に異なるため
   差分が出る可能性が高く、出た場合は「本ハーネスが最初に検出した実回帰」として基盤の
   有効性そのものの証拠になる（TH1eの代替候補ではなく別タスク）。
5. opus-adversarial-consult round3（U1〜U3反映済み）で「設計の骨格（Part A〜C）は
   収束した」と判定済み。**Part D（TH1d'）はround4を1ラウンド実施済み（Blocker7件・
   Should-fix9件、すべて反映済み。ユーザー指示により多段ループはせず1ラウンドで
   打ち切り）**。実装着手時に決定D1〜D8の記述と実コードが乖離していないか
   （特にD1の`journal.rs::JournalEntry`variant追加、D2の固定長配列化）を再確認する
   こと。TH1aの実装着手時に本ADRの記述と実コードが乖離していないか（特にPart AのStep 0
   対象・`DecisionSite`の5バリアント）を再確認すること、という記述自体はTH1a完了に
   より役目を終えた（実施済み・乖離なし）。

## 関連

[ADR-158](158-complexity-reduction-north-star.md)（北極星、TH1/TH4がこのADRの成果に
依存）、
[ADR-159](159-existing-io-boundary-inventory.md)（親ADR、段階1/2の記録側は完了済み、
本ADRが再生側を埋める）、
[ADR-089](089-decision-effect-typestate-and-strategy-consolidation.md)
（`ImeKindId`ミラー型・`caps()`・`Actuation`型状態チェーンの原設計、本ADRのPart Aが
踏襲する分離パターンの前例）、
[ADR-151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md)/
[ADR-152](152-keystroke-step-source-sink-pipeline.md)（同じ領域での過去のBlocker）、
[ADR-095](095-tray-bug-report-cloudflare-intake.md)（Part Dが実機コーパスの収集元と
して相乗りする既存のbug report機能。`attach_X`フラグ追加の既存パターンのSSOT）、
`.claude/rules/complexity-budget.md`（TH1e/Part CがADR-162 E1の1-in-1-out原則の
実例になる）。
