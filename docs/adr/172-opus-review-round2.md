# ADR-172 敵対的レビュー round2

対象: `docs/adr/172-tsfnative-blind-rescue-four-system-consolidation.md`（294行、`9eb00664`）
前版: `2a943764`（round1 レビュー: `opus-review-adr172-round1.md`）
レビュー範囲: 読み取りのみ。コード実体は worktree HEAD を参照。

## round1 Blocker の解消状況

| round1 | 内容 | 状態 |
|---|---|---|
| B1 | `issue_open_warrant()` は `HeuristicDefault` を除外しない（決定2の効能が成立しない） | **解消**。当初案を撤回し、事実関係を決定2の 149-155 行に正しく記載した |
| B2 | `issue_open_warrant()` は `FocusProbe` を除外する（drift が拒否した方向） | **解消**。156-159 行に記載、再設計案で `FocusProbe` を触らない方針を明記 |
| B3 | ADR-090 A-2 の制約を落としている | **概ね解消**。`related_adr` に ADR-087/090 追加、非スコープ節で A-2 全面配線を明示的に切り離した |
| B4 | 対象入口が2箇所あるのに1箇所しか書いていない | **未解消（形を変えて残存）**。→ 本版 **B1** |
| B5 | ADR-151 Blocker / BUG-63 がチェックリストに無い | **解消**。項目6・7を追加 |
| B6 | 決定1「挙動不変」が成立しない | **解消**。撤回し Gated/Actuated 非対称・`latch_eager_warmup_without_send`・`origin` ガードを明記 |

Should-fix は S1（呼び出し元の実測）が**部分的に誤ったまま**（本版 S5）、S2〜S6 は解消。

**総評: round1 の事実誤認は解消されたが、再設計した決定2 が literal に実装できない
（B1・B2）。** 特に B2 は依頼(a)への直接回答で、**抽出述語のシグネチャは force-on 側の
呼び出しコンテキストと噛み合わず、そのまま実装すると ConvOpenInference ケースで no-op になる。**

---

## Blocker

### B1. 決定2の「1入口に限定」と「`is_eligible_for_ime_force_on()` を変える」が同じ決定の中で両立していない

ADR は同じ決定2の中で次の2つを書いている。

- 171-173行 / 278行: 「…除外判定だけを純粋関数として抽出し…**`check_drift_correction`と
  `is_eligible_for_ime_force_on()`の両方から呼ぶ**」
- 199-200行: 「**対象を`apply_force_on_for_imm_broken`（1入口）に限定する。`try_force_on_
  bootstrap`は…対象外とし**、別途最後に検討する。」

`is_eligible_for_ime_force_on()` は**両入口が共有する1つの関数**である
（`state/platform_state.rs:773-775`、呼び出し元は `runtime/mod.rs:949` と `:1234`）。
関数本体に除外判定を足せば、**`try_force_on_bootstrap` も自動的に対象になる**。
「1入口に限定する」は実現されない。

これは round1 B4 が指摘した問題が、当初案の撤回で消えたと見えて実際には残っているケースである。
ADR-090 A-R1 が `try_force_on_bootstrap` を最後に回せと言っている理由（ImmCross bootstrap
force-ON が丸ごと無効化される可能性）は、当初案（warrant 配線）固有の話ではなく
**「この入口のゲートを厳しくすると bootstrap 経路が死ぬ」という入口固有の性質**なので、
今回の除外追加でも同じリスクがある。

`try_force_on_bootstrap` 側の具体的な失敗シナリオ: Imm32Unavailable と分類された未知アプリで
IME 検出が連続失敗（`detect_miss_count >= IME_DETECT_MISS_THRESHOLD`）し、
`reset_stale_ime_on_for_imm_broken` が `HeuristicDefault(open=true)` を記録した状態
（`platform_state.rs:1300-1314`、`at_startup(profile, true, ..)` で **`open=true` 固定**）。
- 現状: `effective_open()` が `HeuristicDefault(true)` を採用 → `is_eligible_for_ime_force_on()==true`
  → bootstrap force-ON が走る（これがこの機構の主目的そのもの）
- 決定2 実施後: `HeuristicDefault` + 明示意図なし → 除外 → **bootstrap force-ON が発火しなくなる**

`try_force_on_bootstrap` は `detect_miss_count` が閾値に達したときの最後の手段であり、
その状況では定義上まともな観測が無い（＝`HeuristicDefault` しか残らない）。つまり
**除外を入れると、この機構が発火する条件とほぼ完全に重なって潰れる。**

**必要な修正**: 以下のいずれかを ADR で決めること。
- (a) 新しい述語を `is_eligible_for_ime_force_on()` に足すのではなく、
  `apply_force_on_for_imm_broken` 側（`runtime/mod.rs:948-952`）に追加条件として書く。
  `is_eligible_for_ime_force_on()` は無改造で残す。
- (b) `is_eligible_for_ime_force_on()` に引数（例: 呼び出し元の種別、または
  「弱い観測を信頼してよいか」の bool）を足し、2入口で別の値を渡す。
- (c) 「1入口に限定」を撤回し、`try_force_on_bootstrap` も対象に含めた上で ADR-090 A-R1 が
  警告する挙動変化を正面から引き受ける（その場合はチェックリストに bootstrap 経路の項目が要る）。

(a) が最も素直で、ADR-157 の教訓（発火源の条件を直す）とも整合する。

### B2. 抽出する述語のシグネチャが force-on 側と噛み合わない — literal に実装すると ConvOpenInference ケースで no-op になる（依頼(a)への直接回答）

**純粋関数として抽出できるか**: できる。対象は `state/platform_state.rs:982-988` の3行で、
入力は `trusted.source: ObservationSource` と `explicit_intent: Option<bool>` のみ。
`self.shadow_model.observations` へのアクセスは**この判定の外側**（`:918` の
`drift_duration`、`:934` の `most_recent_trusted`）にあるので、判定そのものは純粋である。
`state/ime_actuation.rs` への配置も妥当（同ファイルは既に `#[cfg(windows)]` 非依存で、
`platform_state.rs:779` から `force_on_attempt_allowed` を呼ぶ前例がある）。

**問題は呼び出し側が「どの `ObservationSource` を渡すか」である。** 両者は
`effective_open` の値を決める経路が違う。

drift correction 側（`platform_state.rs:932-988`）:

```rust
let trusted = self.shadow_model.observations.most_recent_trusted(now)?;   // ← 単一 source
...
if matches!(trusted.source, ConvOpenInference | HeuristicDefault) && explicit_intent.is_none()
```

force-on 側は `effective_open()` → `ImeModel::resolve_open_at()`（`state/ime_model.rs:398-429`）で、
**判定経路が5通りある**:

```rust
let (base, decided_by) = if has_explicit_intent {
    (self.desired_open, BaseDecision::ExplicitIntent)
} else if let Some(outcome) = self.observations.derive_any(now) {      // ← ここが本命
    ... BaseDecision::DeriveHigh(source) / DeriveMedium { first, second }
} else if let Some(trusted) = self.observations.most_recent_trusted(now) {
    ... BaseDecision::MostRecentTrusted(trusted.source)
} else {
    (self.desired_open, BaseDecision::DesiredFallback)
};
```

`derive_any()` が `most_recent_trusted()` より**先**に評価される。そして
`derive_any()` は BeliefOnly の観測も採用する（`observation_store.rs:1914` の
テストコメント「ConvOpenInference（BeliefOnly）単独でも derive_any は採用する」）。
confidence を見ると:

| 観測源 | confidence | 本番の記録点 | `resolve_open_at` で通る枝 |
|---|---|---|---|
| `ConvOpenInference` | **Medium 固定**（`evidence.rs:243-253` の doc「confidence は `Medium` 固定（BUG-19 対策の上限）」） | `platform_state.rs:1508` | `derive_any` → **`BaseDecision::DeriveMedium { first: ConvOpenInference, second: None }`** |
| `HeuristicDefault` | Low（`ime_model.rs:1362-1380` の回帰テストが Low 前提） | `platform_state.rs:1306` | `derive_any` は Medium+ 専用なので外れ → **`BaseDecision::MostRecentTrusted(HeuristicDefault)`** |

したがって、`check_drift_correction` から抽出した
`fn (source: ObservationSource, explicit_intent: Option<bool>) -> bool` を
force-on 側で「`most_recent_trusted(now)` の source を渡して呼ぶ」形で実装すると:

- `HeuristicDefault` ケース: `most_recent_trusted` が `HeuristicDefault` を返すので**効く**
- **`ConvOpenInference` ケース: `effective_open()` は `DeriveMedium` 枝で値を決めているのに、
  `most_recent_trusted(now)` を別途呼んで得た source を判定材料にすることになり、
  「実際に `effective_open()` を決めた source」と一致する保証がない。**
  かつ ADR の動機の一方（BUG-63 = `ConvOpenInference` 単独での force-on eligibility、
  差分オラクル old_only-3、`open_warrant.rs:1199-1205`）は、まさにこの `DeriveMedium` 経路で
  起きている

結果、**literal に実装すると「HeuristicDefault だけ効いて ConvOpenInference は効かない/
一致しない」という半端な状態になる。** チェックリスト項目6（BUG-63）は達成されない。

さらに悪いのは、`most_recent_trusted(now)` を force-on 側で**新たに直接呼ぶ**設計にすると、
ADR 自身が引用する BUG-110 の構造的欠陥（「別々の関数が別々の解決経路で独立に計算する」、
`platform_state.rs:566-568`）を**新規に作ること**になる点である。`resolve_warmup_ime_on` が
これを避けられたのは、`check_drift_correction` を**そのまま呼んだ**（同じ解決経路を使った）
からで、source を取り直したからではない。

**必要な修正**: 抽出述語の入力を `ObservationSource` ではなく
`ImeModel::resolve_open_at()` が返す `BaseDecision`（`ime_model.rs:79-98`）にすること、
または force-on 側が `resolve_open_at()` の `DecidedBy` を使う設計にすることを ADR に明記する。
`BaseDecision` は5 variant あり、除外対象は最低限:

- `MostRecentTrusted(ConvOpenInference | HeuristicDefault)`
- `DeriveMedium { first: ConvOpenInference | HeuristicDefault, second: None }`（単独合意のみ。
  `second: Some(_)` は2ソース合意なので除外対象外にすべきか要判断）
- `DeriveHigh(_)` は除外しない（High は実 API 読み取り）
- `ExplicitIntent` / `DesiredFallback` は観測由来でないので対象外

そして `resolve_open_at()` は既に「なぜこの値になったか」を返す診断 API として存在し
（`ime_model.rs:391-397` の doc が「本バグ（`mise`→「くした」）は `effective_open()` が単一の
bool しか返さず判定根拠が失われていたために原因追跡に時間がかかった——この API はその反省から
追加する」と書いている）、**まさにこの用途のために用意されている**。ADR がこれに触れていないのは
もったいない。

なお `explicit_intent` の側は噛み合っている: drift 側の `explicit_intent()`
（`platform_state.rs:193-195`、`shadow_model.last_intent.map(|i| i.target)`）と
force-on 側の `has_user_explicit_intent()`（`ime_model.rs:349-351`、`last_intent.is_some()`）は
同じ `last_intent` を見ており、かつ `has_explicit_intent == true` のとき
`resolve_open_at` は `BaseDecision::ExplicitIntent`（観測を一切見ない）になるので、
drift 側の `&& explicit_intent.is_none()` と自然に対応する。ここは追加の配線が不要。

### B3. 新述語を足すと「観測ソースの信頼判定」が同一リポジトリ内で4箇所目になり、ADR 自身の動機（BUG-110 の構造的欠陥）を再生産する

決定2 の正当化は「別々の関数が別々の解決経路で独立に計算する」BUG-110 型欠陥の解消である
（ADR 177-179行が `resolve_warmup_ime_on` の前例を引く）。ところが現状、「どの観測源なら
actuation の根拠にしてよいか」を独立に決めている場所が**既に3つある**:

| # | 場所 | 判定 |
|---|---|---|
| 1 | `state/platform_state.rs:982-988` | `ConvOpenInference \| HeuristicDefault` かつ明示意図なし → drift 不発火 |
| 2 | `state/open_warrant.rs:159-160`（Step 3） | `authority()==Actuating` のみ採用（＝`FocusProbe`/`HwndCache` も除外、`HeuristicDefault` は Step 4a で別枠採用） |
| 3 | **`state/ime_actuation.rs:493-509`** `decide_conv_inference_drift(source, ..)` | `ConvOpenInference` 由来の drift を明示意図エピソードあたり1回に絞る（BUG-113） |

決定2 が作る新述語は4つ目であり、**#3 と同じファイル `state/ime_actuation.rs` に同居する**
（ADR 171行が置き場所として指定している）。#3 は「`ConvOpenInference` は反証不能なので
繰り返し送っても意味が無い」という理由で source を見ており、新述語は「`ConvOpenInference` /
`HeuristicDefault` は awase 自身の推測なので actuation の根拠にしない」という理由で source を
見る。**理由は違うが対象 source が重なっており、片方だけ更新されて乖離する典型的な形**である。

ADR は少なくとも次を書くこと:
- 新述語と #3 `decide_conv_inference_drift` の関係（統合するのか、意図的に別物として残すのか。
  別物なら、なぜ同じ source 集合に対して2つの独立した規則が要るのかの1行）。
- 新述語と #2 `issue_open_warrant()` Step 3 の関係（非スコープ節で A-2 を切り離した以上、
  将来 A-2 が入ったときに #2 と新述語のどちらが勝つのか、あるいは新述語が不要になるのか）。

これを書かないと、本 ADR は「BUG-110 型の非対称を直す」と言いながら**新しい非対称の種を
1つ植えて終わる**。ADR-157 の教訓（抑止機構を重ねるより発火源を直す）にも反する。

---

## Should-fix

### S1. 「`effective_open()` を主たる根拠として使い続ける」という決定が、コード内の不変条件と正面から衝突する

ADR 180-183行:

> `FocusProbe`の扱いを変えない（`is_eligible_for_ime_force_on()`は引き続き`effective_open()`を
> 主たる根拠として使い、その上に`ConvOpenInference`/`HeuristicDefault`単独ケースの除外だけを
> 追加する）

これは round1 B2 への正しい対応だが、次の2つのコードコメントと衝突する。

`state/ime_model.rs:355-362`:

```
/// **これは belief（間違っていても低リスクな推定）であり、engine の内部挙動
/// 決定用。実際に OS の IME を操作してよいかという actuation の根拠には
/// 使わないこと**（ADR-087 §5 Phase 3 item17）。`derive_any()` の
/// Medium 単一ソース合意がそのまま actuation の根拠として使われたことが
/// BUG-63（「mise」→「くした」誤入力）の直接原因だった。actuation
/// warrant が必要な場面（IME を実際に force-ON する等）では
/// `crate::state::open_warrant::issue_open_warrant()` を使うこと
```

`state/platform_state.rs:764-772`（`is_eligible_for_ime_force_on` 自身の doc）:

```
/// **belief 由来の暫定ゲート（ADR-087 §5 Phase 3 item15 で
/// `issue_open_warrant()` に置換予定、まだ未配線）。**
```

ADR は「BUG-63 パターン自体は解消しない…それは ADR-087/090 の別イニシアチブとして残す」
（184-188行）と書いており、意図的な先送りであることは読み取れる。しかし:

- コード側の doc は「`issue_open_warrant()` に置換予定」と書いたままになる。ADR-172 実施後は
  「置換予定だが、ADR-172 が別の除外を先に足した」という中間状態になるので、この doc を
  どう更新するかを「次のアクション」に入れること。更新しないと、次の読み手が
  ADR-172 の除外を「item15 の置換の一部」と誤解する。
- `ime_model.rs` の「actuation の根拠には使わないこと」という不変条件に対し、ADR-172 は
  例外を1つ公式に承認した形になる。ADR 本文で「item17 の例外を1つ意図的に延命する」と
  明記しておくと、将来 A-2 を進める人が本 ADR を根拠に「もう `effective_open()` でよい」と
  読むのを防げる。

### S2. 決定1 が「決定の本体」と宣言しながら、実際には何も決めていない

ADR 133-134行: 「**決定: 集約するかどうか自体を決定の本体とし、以下を明示する:**」
その後に続く3点は (1)「倒すなら〜の方向にする」（条件付き）、(2) latch を保存する（制約）、
(3)「維持するか…置き換えるかを実装前に決める」（先送り）。
次のアクション 271-274行も「どちらへ倒すか…を確定した上で着手する」。

つまり **「集約するかどうか」は決まっていない。** ADR は決定文書なので、
- 集約する / しない のどちらかを決める（しないなら決定1 を削除して背景の記述に降格する）
- 集約するなら「Actuated 系に INV-B1' ゲートを適用する」を推奨ではなく決定として書く

のいずれかにすること。現状は round1 B6 を受けて「挙動不変ではない」と正しく認めた結果、
決定が空洞化している。

なお Actuated 系にゲートを適用する場合の具体的な影響を1つ挙げておく:
`platform.rs:1563-1570` のコメントが書くとおり、この経路は
**force-on が `SetOpen(true)` を適用した直後にも通る**。ここに `off_drift_active` ゲートを
かけると、force-on 直後の随伴 warmup が drift correction の OFF 方向継続中に抑止される。
これは意図した効果だが、同時に `eager_warmup_sent_ms` の更新機会が減るため
`compute_focus_probe_grace` の grace が短くなる（ADR-149 M3 が `latch_eager_warmup_without_send`
を作った理由そのもの）。**ゲートで抑止する場合も latch は行う**、と決定に書くこと。
ADR 138-140行は latch の「保存」に言及しているが、それは `should_send_accompanying_warmup`
分岐の話で、新設するゲートの分岐については触れていない。

### S3. 「観測手段の探索は閉じたのか」が ADR 内の3箇所で食い違う

- 背景節 104-108行: 「4・5は未検討のまま残っている。**「観測手段の探索は尽くした」とは
  言えない。**」
- 決定4（223-235行）: 「今回検討した3方向…に限って閉じたことを記録する。ただし以下は
  未検討のまま残し」
- 非スコープ節 261行: 「mozc内部IPCの解析・傍受を実装の結合先にすること（**前節の調査で
  閉じたと判断**）。」
- 決定の前文 112行: 「観測手段探しに**全面的には**賭けず」

3番目は 1・2 の経路に限れば正しいが、「前節の調査で閉じた」という表現は背景節の訂正
（「尽くしたとは言えない」）と読み手には矛盾して見える。ADR-151 の再検討条件1が
「閉じた」のか「一部未確認」なのかは、この ADR を将来参照する人が最初に知りたい1点なので、
1文で統一すること。提案: 非スコープ節を「mozc 内部 IPC の解析・傍受（経路1・2、調査済みで
不成立）」と限定し、4・5 は決定4 の別issue化に一本化する。

### S4. チェックリスト項目6・7 が「確認する」止まりで、確認手段もベースラインも無い

項目6（254-255行）:「ADR-151のBlocker（force-onの構造的永久停止）を実機ソークの
**発火頻度で確認する**」

現在の force-on 発火頻度のベースラインが無いと「落ち込んでいない」の判定ができない。
`ActuationDecision` は journal に記録されている（`runtime/mod.rs:1041-1044`、
`DecisionSite::ForceOnRomajiCorrection`）ので、**変更前に同じシナリオで
`caller == ForceOnRomajiCorrection` のレコード数を数えておく**手順を (iv) に足すこと。

加えて、`ActuationDecision` の `outcome` だけで「force-on が効いた」と判定してはいけない
（`outcome: Applied` は送信の成功であって IME 状態が変わった証拠ではない）。
発火「回数」を数えるのが目的なので今回は問題になりにくいが、項目6 が
「救済が機能しているか」まで見るつもりなら、observation 側のイベントと突き合わせる必要がある。

項目7（256-257行）「`Instant::now()`ズレを、決定2の変更が新たに悪化させないこと」も
測り方が書かれていない。決定2 が共有述語の追加のみに留まるなら、この項目は
「決定2 は新しい `Instant::now()` 取得点を追加しない」というレビュー時の確認事項に
書き換えたほうが実行可能（ソークで時刻ズレを測るのは現実的でない）。

### S5. 背景表の warmup 行「呼び出し元7箇所」が二重計上（round1 S1 の修正が行き過ぎた）

ADR 46行:

> 呼び出し元7箇所: `platform.rs`5箇所〔`:305,642,1427,1451,1610`〕、`output/vk_send.rs:692`、
> および`platform.rs:299`の`send_eager_warmup`ラッパー経由で`runtime/ime_refresh.rs`の
> FocusChange処理から間接的に1箇所

`platform.rs:299` は**doc コメントの行**であり、呼び出し箇所ではない（実測: `:297` が
セクションコメント、`:299-302` が doc、`:303` が `pub(crate) fn send_eager_warmup`、
`:305` がその中の `.send_eager_tsf_warmup(...)`）。つまり**このラッパー経由の1箇所は
既に列挙済みの `:305` そのもの**で、7箇所目として別に数えると二重計上になる。

正確な数え方は次のどちらか:
- `.send_eager_tsf_warmup(` の呼び出し箇所 = **6箇所**（`platform.rs` 5 + `vk_send.rs` 1）。
  うち `:305` は `ime_refresh.rs` の FocusChange から `send_eager_warmup` ラッパー経由で
  到達する。
- `eager_tsf_warmup_inner`（`output/mod.rs:1158`）への到達経路 = **7**
  （`send_eager_tsf_warmup` 6 + `latch_eager_warmup_without_send` 1）。

決定1 が扱うのは後者（latch を含む）なので、後者の数え方で書くのが自然。ADR 46行の
現在の内訳はどちらでもない。

### S6. 決定2 の再設計が、当初の動機「BUG-110 追補7〜9 の非対称」を本当に解消するかの検証が無い

決定2 の動機は「drift correction 側だけに除外があり force-on 側に無い」という非対称
（チェックリスト項目4）。B2 で示したとおり、除外が効くのは `HeuristicDefault` ケースである。
そのシナリオを最後まで辿ると:

1. Imm32Unavailable ウィンドウ入場 → `reset_stale_ime_on_for_imm_broken` が
   `HeuristicDefault(open=**true**)` を Low confidence で記録（`platform_state.rs:1300-1314`）
2. `FocusChanged` で `last_intent` はクリア済み（明示意図なし）
3. force-on 側: `effective_open()` = `MostRecentTrusted(HeuristicDefault)` → `true`
   → force-on が ON を書く
4. drift correction 側: `desired_open` が前ウィンドウの残留で `false` の場合、
   `trusted.open(true) != desired(false)` だが除外ガードで `None` → **既に発火しない**

つまり **BUG-110 追補9 の修正時点で、drift correction 側は既に沈黙している。**
「互いに逆方向へ書き込みを取り合う」状態は片側が既に止まっているので解消済みであり、
残っているのは「force-on だけが `HeuristicDefault(true)` を信じて ON を書く」という
**片側のみの動作**である。

これを止めるのが決定2 だとすると、ADR は「双方向の衝突を解消する」ではなく
**「force-on が弱い観測だけで ON を書くのをやめる」**という単独の判断であり、その是非は
「では誰が TsfNative で ON へ戻すのか」という ADR-151 Blocker と直結する
（B1 の `try_force_on_bootstrap` シナリオと同型）。ADR 247-248行のチェックリスト項目4 は
「二重SSOTとして衝突しない」と書いているが、**衝突は既に片側修正で止まっている**ので、
確認すべきは衝突の有無ではなく「force-on を止めた後に ON へ戻す経路があるか」である。
チェックリスト項目4 の文言を実態に合わせること。

---

## Nice-to-have

- **N1.** ADR が参照する `opus-review-adr172-round1.md`（28行）はリポジトリ未追跡
  （`git status` で `??`）。本 ADR がマージ後もこの参照を保つなら追跡対象にするか、
  参照を消して要点を ADR 本文に取り込むか決めること。
- **N2.** タイトル・`title` frontmatter が「4系統の整理方針」のままだが、本版の実質は
  「決定1=warmup ゲートの方向決め」「決定2=観測ソースフィルタの共有」「決定3=位置づけ整理」
  「決定4=探索の中間報告」で、4系統の統合はもう目標ではない（非スコープ節 262-263行が
  明示的に否定している）。`docs/adr/index.md:181` の1行も現状の内容とややズレる。
- **N3.** 背景表の force-on 行（44行）は2入口を1行にまとめているが、warrant 強制状態も
  今後の扱い（`try_force_on_bootstrap` は A-2 最後）も入口ごとに違う。B1 の修正とあわせて
  2行に分けると、決定2 の適用範囲が表からも読める。
- **N4.** 決定2 の「当初案を否定する5点」（149-166行）は round1 レビューの要約であり、
  ADR 本体としてはやや長い。`opus-review-adr172-round1.md` を追跡下に置くなら
  「詳細は round1 レビュー参照」に圧縮でき、`.claude/rules/fix-requires-evidence.md` の
  「1ファイル30行以内」の精神（ADR には直接適用されないが）にも沿う。
- **N5.** 決定2 の実装を Linux で検証できるテストの置き場所が書かれていない。抽出した
  述語は `state/ime_actuation.rs`（`#[cfg(windows)]` 非依存）に置くので、
  `cargo test -p awase-windows --lib` で走るユニットテストを足せる。
  `open_warrant.rs::differential_old_gate_vs_issue_open_warrant` と同じ形で
  「除外導入前/後」の差分を全数列挙して固定するテストが書けるはずで、実機ソーク前に
  挙動変化の全体像を押さえられる。次のアクション (ii)(iii) に足すと良い。

---

## 依頼事項への直接回答

**(a) B1〜B6 は解消されたか**

round1 B1/B2/B3/B5/B6 は解消。**B4 は形を変えて残存**（本版 B1: 共有関数
`is_eligible_for_ime_force_on()` を変えながら「1入口に限定」と書いている矛盾）。

再設計案（共有述語抽出）が新しい問題を生んでいるか → **生んでいる、3点**:
1. 述語の抽出自体は可能（`platform_state.rs:982-988` の3行は純粋で、
   `state/ime_actuation.rs` は `#[cfg(windows)]` 非依存かつ `platform_state.rs:779` からの
   呼び出し前例あり）。**しかしシグネチャが噛み合わない**——drift 側は
   `most_recent_trusted()` の単一 `ObservationSource`、force-on 側は
   `resolve_open_at()` の `BaseDecision`（5 variant）。`ConvOpenInference` は Medium なので
   `DeriveMedium` 枝、`HeuristicDefault` は Low なので `MostRecentTrusted` 枝と、
   **2つの対象 source が別々の枝を通る**。単一 source 前提で実装すると
   `ConvOpenInference`（＝BUG-63 の当該ケース）に当たらない（本版 B2）。
2. force-on 側で `most_recent_trusted()` を新たに直接呼ぶ実装にすると、ADR 自身が引用する
   BUG-110 の構造的欠陥（別々の関数が別々の解決経路で独立に計算する）を新規に作る（本版 B2 後半）。
   正しくは `resolve_open_at()` の `DecidedBy` を使う——この診断 API は
   `ime_model.rs:391-397` の doc がまさに BUG-63 の反省で追加したと書いており、用途が一致する。
3. 「観測ソースの信頼判定」が4箇所目になり、うち1つ（`decide_conv_inference_drift`、
   `ime_actuation.rs:493-509`）は**同じファイルに同居する**（本版 B3）。

**(b) 新たに見落としている論点**

- 決定2 の動機である「双方向の衝突」は、drift correction 側の BUG-110 追補9 修正で
  **既に片側が沈黙している**。残るのは force-on の片側動作であり、決定2 は
  「衝突の解消」ではなく「TsfNative の ON 方向救済を弱める」判断である（本版 S6）。
  この読み替えをしないと、チェックリスト項目4 と項目6 が同じことを別方向から
  書いている状態になる。
- 決定1 が実質何も決めていない（本版 S2）。
- `effective_open()` を actuation 根拠として延命する決定と、コード内 doc
  （`ime_model.rs:355-362` / `platform_state.rs:764-772`）の衝突（本版 S1）。

**次工程の判断**: B1・B2 は実装着手前に必ず解決が要る（literal に実装すると動かない／
意図しない入口を巻き込む）。B3・S1〜S6 を反映すれば round3 で収束できる見込み。
