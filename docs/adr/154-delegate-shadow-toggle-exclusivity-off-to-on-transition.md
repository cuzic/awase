# ADR-154: `delegate_owned` ゲートの排他性を OFF→ON 遷移の打鍵でも成立させる（ADR-149「案C」続報）

## ステータス

**提案中・未実装**（[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
「検討し棄却した代替案・案C」からの分離起票。[ADR-153](153-gji-keymap-aware-safe-vk-substitution-for-mode-keys.md)
決定1の実装着手前提条件（3点のうちの1点）として起票する。実装に進む前に
opus-adversarial-consult での検証を推奨する——本ADRはまだその検証を
経ていない。）

## 背景

`docs/known-bugs.md`（BUG-113節、「独立して発見した2つの未解決事項」1点目）
と[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
「案C」が既に機序を確定させている。要約:

[ADR-141](141-henkan-muhenkan-delegate-inactive-recovery.md)のC2修正
（コミット`246338bc`）は「delegateとshadow-toggleのどちらが処理するかは
`mode_key_delegate_owns_shadow_toggle(vk) && effective_open()`ゲートが
**実行時に排他的に決める**」という不変条件を明記している
（`crates/awase-windows/src/runtime/mod.rs:471-474`）。しかしこの不変条件は
**belief OFF→ON 遷移が起きる打鍵に限り成立していない**——2つの消費点が
異なるタイミングで同じゲートを評価するため、片方がゲートの入力
（`effective_open()`）自体を書き換えてから、もう片方が評価される:

- **消費点2**（`kp_stage_shadow_ime_toggle`、
  `crates/awase-windows/src/runtime/key_pipeline.rs:1150`の
  `delegate_owned`計算、`kp_run_inner`内で`build_input_context`より**前**に
  評価される）: このとき belief は OFF → `delegate_owned = false` →
  shadow-toggle が担当し、`write_physical_key`で **belief を ON へ書き換える**。
- **消費点1**（`resolve_pending_thumb_as_single`の
  `special.delegate_to_open_axis`分岐、`src/engine/nicola_fsm.rs:2020`、
  同時打鍵チョード判定の100ms猶予満了後、`on_timeout`経由で評価される）:
  このとき belief は既に ON（消費点2自身が書き換えた後）→ **delegate も
  発火する**。

結果として1回の物理タップに対し、shadow-toggle 経由の実送信（送信1）と
delegate 経由の実送信（送信3）の**両方**が走る。ADR-149の実機ログでは
送信3が`AlreadyMatched`（実送信なし）に握り潰されたため実害が「@」の
観点では顕在化しなかったが、これは送信1が`Applied`を返し
`applied_snapshot`を先にON確定させていたという**偶然の産物**にすぎない
——ADR-149「案B」の検証が示す通り、送信1側の挙動が変われば送信3は
実送信に転じうる。

`&& effective_open()`という条件式は「同時刻に評価すれば片方だけが
真になる」ことは保証するが、「1つの物理タップの生涯を通じて片方だけが
処理する」ことは保証しない——評価タイミングが2箇所に分かれ、かつ
片方の評価結果が他方が読む状態を書き換えてしまう構造そのものが原因。

### ADR-153 との違い

[ADR-153](153-gji-keymap-aware-safe-vk-substitution-for-mode-keys.md)の
B13/B14は、**同ADRが新設する**「明示config」経路（ケース2）と
「resolve_pending_thumb_as_single内の明示configケース（ケース1）」の
間で同型の二重評価が起きることを発見し、one-shot マーカーを
`PendingThumb`のライフタイムに結びつけることで解決した。しかし
ADR-153のM15対策は「明示config が設定されているキーについては
GJI/MS-IME自動検出由来の`delegate_to_open_axis`と`shadow_override`の
両方を armed にしない」——つまり明示config対象のキーでは
本ADRが扱う旧来の delegate 経路自体が無効化される。

したがって本ADRが扱う「案C」の穴は、**明示config を設定していない
キー**（decision2、既存の自動検出フォールバック経路をそのまま使う
キー）にのみ残る。ADR-153の決定1はこの穴を修正しないままフォール
バック経路として維持することを明示的に選択しており（決定2「add-onで
あり、既存の自動検出パスを置き換えない」）、本ADRはその残置分の
修正を独立に扱う。

## 決定（提案・未検証）

ADR-153のB13/B14が確立した解法パターン——「one-shot マーカーを独立
チャネルにせず、対象イベントの寿命（`PendingThumb`のライフタイム）に
結びつける」——を、旧来の delegate/shadow-toggle ペアにも適用する。

具体的には、消費点2（`kp_stage_shadow_ime_toggle`）が
`delegate_owned == false` と判定して実際に belief を書き換えて
actuation を発行した場合、**その物理タップが後に生成する
`PendingThumb`に「この打鍵の open 軸 actuation は消費点2で済んでいる」
マーカーを持たせる**。消費点1（`resolve_pending_thumb_as_single`の
`delegate_to_open_axis`分岐）はこのマーカーが立っている場合、
`delegate_to_open_axis`の値を見ずにスキップする。

搬送経路は ADR-153 M25 対策と同じ器（`RawKeyEvent.ime_relevance`
= `awase::types::ImeRelevance`）を使う候補が有力——ただし消費点2は
`PendingThumb`が生成される**前**（`kp_run_inner`）に評価されるため、
ADR-153のケース2と同じ「`ImeRelevance`に新フィールドを足し、
`NicolaFsm::on_input`が`PendingThumb`生成時にそれを一緒に格納する」
という配線がそのまま転用できる可能性が高いが、これは検証済みの結論
ではなく**実装時に確認が必要な仮説**である。

### 未検証事項（実装前に詰めること)

1. **ADR-153 M25 のマーカー（ケース2用）と本ADRのマーカー（案C用）は
   同じフィールドで良いか、別フィールドが要るか**——両者は「belief OFF
   →ON遷移をconsumption点2的な場所で処理し、consumption点1側の重複
   発火を止める」という同型の問題を解決するが、ADR-153のケース2は
   明示config対象キー、本ADRの対象は自動検出delegate対象キー
   （両者は排他: M15により同一キーが両方の対象になることは無い）。
   フィールドを共有しコメントで両方の用途を明記する案と、意味の混同を
   避けるため別フィールドにする案のどちらが安全か、実装時に判断する。
2. **`delegate_owned`の計算自体を遅延できないか**——根本原因は「2箇所が
   異なるタイミングで同じゲートを評価する」ことなので、マーカーで
   片方を止める対症ではなく、消費点2の評価自体を`build_input_context`
   後（またはconsumption点1と同じタイミング）に動かせないか、
   opus-adversarial-consultで検討する価値がある。ただし消費点2は
   チョード確定を待たない毎打鍵処理という別の設計上の制約
   （`kp_stage_shadow_ime_toggle`が同時打鍵チョード判定を経由しない
   経路であること自体はADR-153の「未決着#9」が既に指摘している）があり、
   単純な移動では別の壊れ方をする可能性がある。
3. **回帰テスト**: `fix-requires-evidence.md`の「キー選択」「IME belief」
   両ファミリーに該当するため、`crates/awase-windows/tests/`配下
   （`architecture_guard.rs`等）への回帰テスト追加を実装の必須条件とする。
   ADR-149の実機ログ（`docs/known-bugs.md`BUG-113節）が記録した「送信1
   →`Applied`→送信3`AlreadyMatched`」というログパターンを、修正後は
   「送信3自体が発生しない」に変える形の journal replay / golden が
   望ましい。

## 関連

BUG-113、[ADR-141](141-henkan-muhenkan-delegate-inactive-recovery.md)
（排他性の不変条件を明記した当のC2修正）、
[ADR-147](147-thumb-key-delegate-defers-to-user-passthrough.md)、
[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
（「案C」の発見元、必須条件4で`docs/known-bugs.md`への記録を完了済み）、
[ADR-153](153-gji-keymap-aware-safe-vk-substitution-for-mode-keys.md)
（B13/B14が同型の問題を別経路に対して解決、本ADRはその解法パターンの
旧経路への転用）、`.claude/rules/fix-requires-evidence.md`。
