# ADR-154: `delegate_owned` ゲートの排他性を OFF→ON 遷移の打鍵でも成立させる（ADR-149「案C」続報）

## ステータス

**実装済み（2026-09-09）。** [ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
「検討し棄却した代替案・案C」からの分離起票。opus-adversarial-consult
architect/critic 各2ラウンド（round1でBlocker 2件を検出、round2で反映し
Blockerゼロで収束）を経て設計確定、そのまま実装した。`cargo test --lib`
（1010件）・`cargo test --test scenarios`（8件）・`cargo nextest run
-p awase-windows --test architecture_guard --test golden_scenarios
--test layer_boundary_guard`（119件）・`cargo clippy --target
x86_64-pc-windows-msvc -p awase -p awase-windows`（`.claude/rules`が
定めるスコープ、`-D clippy::pedantic`/`-D clippy::nursery`込み）全green。
Windows実機ソークは未実施。

round1で見つかった2件のBlocker（詳細は「決定」節）:
- 消費点1でマーカーが立った打鍵を優先順位4（`ModeKeyConfig`の
  Suppress/Passthrough）へフォールスルーさせる初期案は、BUG-123
  （明示config側で確認済みの「かな⇄カタカナ切替と誤認される二重送出」）を
  自動検出delegate側に新規に再現すると判明——`no_op_resolution()`で
  早期returnする形に変更した。
- 新規フィールドが必要な根拠として当初挙げた「`transport.rs::plan`の
  Allow/Suppress判定が効くから流用は壊れる」は、実コード検証の結果
  **成立しないと判明**（delegate armed なキーの物理配送は必ず
  `Decision::Consume`に乗り`plan`の戻り値は参照されない）。正しい根拠は
  「engine非活性時（`Inactive(ImeOff)`/`Inactive(UserDisabled)`）は
  `Decision::PassThrough`に落ち、そこでは`plan`の戻り値が実際に物理配送を
  左右する」ことに差し替えた。

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

### 送信1の発生源の訂正

上で「shadow-toggle経由の実送信（送信1）」と書いたが、これは不正確なので
訂正する。`kp_stage_shadow_ime_toggle`のOFF→ON経路は**actuationを一切
行わない**。実actuationを含む分岐はON→OFF方向専用（`issue_actuation_
order`/`run_open_chain_async`/`ImeController::apply`はすべてこの
ブロックの中）であり、OFF→ONでは書き込み直後に`effective_open()==true`
になるためスキップされる。

実際の`VK_IME_ON`送信は、消費点2の**belief書き込みが誘発する**
`check_active_transition`→`handle_engine_activation_sync`→
`GjiDirectStrategy`経由の**ActivationSync**である（[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
§「送信1・2の発生源」）。したがって本ADRが問題にする二重発火は、正確には

- **送信1**: 消費点2のbelief書き込みが誘発するActivationSync送信
  （消費点2自身はfollow-onlyであり、直接actuateはしない）
- **送信3**: 100ms後の消費点1（`delegate_to_open_axis`）が
  `Effect::Ime(SetOpen)`を積むactuation

の重畳である。「消費点2が直接actuateしている」と読める記述は、
`kp_stage_shadow_ime_toggle`をfollow-onlyではなくactuation経路だと
誤解させ、別の修正でその誤解が再燃する種になる。

### `transport.rs::plan`は本ADRの対象打鍵では参照されない（重要な前提）

`crates/awase-windows/src/runtime/transport.rs`の無変換/変換に対する
コメントは「OS側の実際の切替はGJI自身が物理キー配送を通じて行う」
（＝awaseはfollow-onlyで、`plan`が`Allow`を返して生キーがGJIに届く）と
説明している。**この説明は、無変換/変換が親指キーとして設定されている
場合には成立していない。** 連鎖:

- `delegate_owns_mode_key_shadow_toggle`
  （`crates/awase-windows/src/gji_charset_autodetect.rs`）は先頭で
  `is_configured_thumb_key &&`を要求する。つまりdelegate armed ⟹
  そのVKは必ず設定済み親指キー。
- 消費点2がbeliefをONに書いた直後の同一イベントで`ctx.ime_on = true`
  （`key_pipeline.rs`の`kp_stage_shadow_ime_toggle`→`build_input_
  context`の順）→`compute_state`（`src/engine/engine.rs`）が`Active`
  →`NicolaFsm`は親指KeyDownを`idle_wait`で`PendingThumb`に入れ
  `ParseAction::Shift`を返す。
- `ParseAction::Shift`は`build_response(actions, /*consumed=*/true,
  timers)`になり、`FsmAdapter::response_to_decision`で
  **`Decision::Consume`**。
- `execute_relay`の`Decision::Consume`アーム（`crates/awase-windows/src/runtime/executor.rs`）
  は**`physical`（＝`plan`の戻り値）を一切参照しない**。参照するのは
  `PassThrough`と`PassThroughWith`の2アームのみ。
- 対応するKeyUpも`Engine::on_input`が`take_key_up_duty`で
  `UpDuty::Consume`を得て`decision.force_consume()`するため、
  やはり`Consume`。

**結論: engine活性時の親指キーDown/Upは両方とも`Decision::Consume`に
乗り、`plan`のAllow/Suppressは参照されない。** 「`plan`が効くから
流用不可」という論法はこのケースでは成立しない——成立するのはengine
**非活性**時の`PassThrough`経路だけである（詳細は「決定」節）。

### 対象VKは4種

本ADR本文は無変換/変換を主に論じるが、修正が適用されるのは
`delegate_owns_mode_key_shadow_toggle`が対象とするHiragana / Katakana /
Henkan / Muhenkanの**4VK全体**である。`VK_DBE_HIRAGANA`/
`VK_DBE_KATAKANA`を親指キーに設定しているユーザーにも同じ二重発火が
成立する。

なお`muhenkan_dedicated_fn_key_configured`が trueのときは無変換の
delegate armedがfalseになるため優先順位1（専用Fnキー）が勝つ既存挙動と
整合する（本ADRのマーカーも立たない）——さらに`dedicated_fn_key`自体が
`thumb_solo_special_handling`の無変換分岐でしかSomeにならない（変換/
Hiragana/Katakanaは`None`ハードコード）ため、優先順位1との競合は
Hiragana/Katakana側では原理的に発生しない。

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

## 決定

ADR-153のB13/B14が確立した解法パターン——「one-shot マーカーを独立
チャネルにせず、対象イベントの寿命（`PendingThumb`のライフタイム）に
結びつける」——を、旧来の delegate/shadow-toggle ペアにも適用する。
ただし**マーカーはADR-153の`explicit_ime_action_consumed`を流用せず、
`ImeRelevance`に新規フィールドを設ける**。

### 決定1: `ImeRelevance`に`auto_delegate_open_axis_consumed: bool`を新設する

`src/types.rs`（`explicit_ime_action_consumed`の隣）に追加する。

**`explicit_ime_action_consumed`とは別フィールドである理由**: 流用が
壊すのは「engine**非活性**のまま`Decision::PassThrough`に落ちる打鍵」
である。`kp_stage_shadow_ime_toggle`はengineの有効/無効・活性/非活性に
関係なく走る——`kp_run_inner`は無条件にこれを呼び、活性判定
（`compute_state`）はその後の`engine.on_input`の内部で初めて行われる。
`hook.rs`にも`key_pipeline.rs`にもengine-disabledのバイパスは無い。

具体的な破壊シナリオ:

1. **GJI既定キーマップの最頻ケース**: `muhenkan_delegate_to_open_axis
   = Some(TurnOff)`（GJI既定は無変換=直接入力／変換=ひらがな）、無変換は
   親指キー、IMEは既にOFF。この状態で無変換を単独タップすると、
   `delegate_armed = true`かつ`effective_open() = false`→
   `delegate_owned = false`で消費点2が担当するが、`current=false`→
   `action.resolve(false) = false`なのでbeliefはOFFのまま。
   `ctx.ime_on = false`→`compute_state`が`Inactive(ImeOff)`→
   **`Decision::PassThrough`**→`execute_relay`のPassThroughアームが
   `physical == Suppress`ならOSへ届けず`Consumed`を返す。流用すると
   この打鍵の無変換が**GJIに一切届かなくなる**（現状は`transport.rs`が
   `Allow`を返し`enqueue_reinject`経由で届く）。
2. **engineがユーザー操作で無効の間**（`Inactive(UserDisabled)`）:
   IME OFFから変換キーでIMEを開こうとするとbeliefはONに書かれるが
   engineは非活性なのでPassThrough→流用時はSuppressで握り潰される。
   awaseを一時停止しているのに無変換/変換が死ぬ、という体験の劣化。

（`Inactive(NotRomajiInput)`は OFF→ON 遷移時に`eisu_reset_on_ime_on`が
自己修復するため根拠には数えない。）

なお決定2でマーカーを「beliefが実際にOFF→ONへ動いた」場合に限るため、
シナリオ1ではそもそもマーカーが立たなくなる。それでもフィールドを
分けるのは、シナリオ2が残ることと、「意味の異なる2つのマーカーを1つの
`bool`に潰すと`transport.rs::plan`の判定域が意図せず広がる」構造的
リスクを避けるためである。

**禁止事項**: `transport.rs`のproductionコードはこのフィールドを
読んではならない（`tests/architecture_guard.rs`のgrepガードで機械的に
固定した：`transport_plan_never_reads_auto_delegate_open_axis_consumed`）。

### 決定2: マーカーは「beliefが実際にOFF→ONへ動いた」打鍵にだけ立てる

`key_pipeline.rs`の`delegate_owned`計算をarmed判定とbelief判定に分解し、
armedだけを再利用する:

```rust
let delegate_armed = self.mode_key_delegate_owns_shadow_toggle(event.vk_code);
let delegate_owned = delegate_armed && self.platform_state.ime.effective_open();
```

マーカーを立てるのは`if !delegate_owned { match kind { ... } }`の
**matchの直後・同ブロック内**（両アームがwitnessを得てbeliefを書けた後）:

```rust
if delegate_armed && !current && self.platform_state.ime.effective_open() {
    event.ime_relevance.auto_delegate_open_axis_consumed = true;
}
```

**`!current && effective_open()`と方向つきで書く**（「beliefがONに
なった」場合のみ）理由: マーカーは`PendingThumb`に載って最大100ms
（`simultaneous_threshold_ms`既定値）生き残る。beliefが動かなかった
打鍵（GJI既定の無変換=TurnOff×IME既にOFFが最頻）でマーカーを立てると、
その100msの窓の間に`ir_apply_drift_correction`等の別経路がbeliefを
ONにした場合、タイムアウト時にはengineが活性になっており、本来発火
すべきdelegateを誤って握り潰す。書き込み後の`effective_open()`
（意図ではなく実際にreducerが受理した結果）を見るのは、直後の既存
no-op検出（`if self.platform_state.ime.effective_open() == current`）
と同じidiom。

この位置・この条件である理由:

- **matchの後**: 両アームはwitnessが得られなければ早期returnするため、
  **実際にbeliefを書けた場合にのみ**マーカーが立つ。
- **`kind`で絞らない（`SyncKey`も対象にする）**: `IntentKind::SyncKey`
  アーム（`write_sync_key`）もbeliefをONへ書き換えるため、同一VKに
  `keys.ime_detect`の`sync_direction`と自動検出delegateが同時設定
  されていれば同型の二重発火が成立する。これはADR本文の当初スコープ
  には無い追加スコープである。
- **`if delegate_owned`側には絶対に置かない**: 置くと消費点2も消費点1
  も何もしないBUG-115型の穴になる。

### 決定3: 消費点1はマーカーを見たら`no_op_resolution()`で早期returnする

`resolve_pending_thumb_as_single`に引数を1つ足し、既存の
`explicit_action_consumed`早期returnの直後で同じく打ち切る。
**優先順位4（`ModeKeyConfig`のSuppress/Passthrough）へフォールスルー
させてはならない**（round1で検出したBlocker、下記参照）。

```rust
if explicit_action_consumed {
    return Self::no_op_resolution();
}
if auto_delegate_open_axis_consumed {
    return Self::no_op_resolution();
}
```

**なぜフォールスルーさせてはならないか（round1 Blocker）**: 当初案は
優先順位3（delegate）だけを飛ばし優先順位4へフォールスルーさせる形
だったが、これはBUG-123（`docs/known-bugs.md`）と同型の症状を新規に
再現する。BUG-123の機序は「消費点2のbelief書き込みが誘発する
ActivationSync経由のVK_IME_ON実送信」＋「その100ms後に消費点1の
優先順位4が生キーを合成送出」という2信号の重畳であり、本ADRが対象と
する自動検出delegate経路も前者は完全に同一。優先順位4へ落とすと
`SoloTapAction::Passthrough`分岐が生のVK_NONCONVERT等を合成送出し、
GJIがかな⇄カタカナ切替と誤認する（半角→カタカナに飛ぶ）。上の
`explicit_action_consumed`と同じく`mode_key_config`の設定内容より
優先して打ち切る（BUG-123の「既知のトレードオフ」節と同じ判断）。

**「第3の空振り」は起きない**: 消費点2がbeliefをONにしActivationSync
が実際にVK_IME_ONを送っているため、この打鍵のIME open軸は既に完了
している。さらに、消費点2の`write_physical_key`/`write_sync_key`
自身が既に`UserImeSetIntent`をdispatchし（`last_intent`を設定し）
`record_explicit_intent`を呼んでいるため、このマーカーで打ち切っても
明示意図の記録は失われない（送信3自体は`origin: ExplicitUserAction`で
別途`record_explicit_intent`するが、これは冗長化であって唯一の記録
経路ではない）。

#### 引数追加の形

`resolve_pending_thumb_as_single`は現在`&self`+6引数＝7個。
`clippy.toml`の`too-many-arguments-threshold = 8`に対しclippyは
`args.len() > threshold`で発火するため、8個は許容される（9個で発火）。
**専用構造体化はしない**——非テスト呼び出し7箇所+テスト側の書き換えを
機械的に行うことになり、存在しない制約への対応で差分が膨らむ。素直に
`bool`を1つ足す。

ただし**`clippy::pedantic`（本リポジトリは`deny`）の
`struct_excessive_bools`/`fn_params_excessive_bools`（bool4個以上）は
実際に発火する**——`ClassifiedEvent`・`PendingThumbData`・
`resolve_pending_thumb_as_single`の3箇所に`#[expect(clippy::…)]`と
理由コメントを追加して対応した（opus-adversarial-consultの2ラウンドでは
この2つの pedantic lint は検討されておらず、実装時に発覚。将来同種の
マーカーをさらに追加する場合、この`#[expect]`を専用構造体化への
シグナルとして扱うこと）。

また`timeout_pending_thumb`（`resolve_pending_thumb_as_single`の
timeout経由ラッパー）は元々`&mut self`+7引数＝8個で、`bool`を1つ足すと
9個になり`clippy::too_many_arguments`（全group、`deny`）に**実際に
抵触した**。この関数はすべての引数を`PendingThumbData`のフィールドから
そのまま渡しているだけだったため、シグネチャを
`fn timeout_pending_thumb(&mut self, thumb: PendingThumbData, composing: bool)`
に変更し、3引数に削減して解消した（産物：呼び出し側も簡潔になった）。

### 決定4: `delegate_owned`計算の遅延は却下する

根本原因は「2箇所が異なるタイミングで同じゲートを評価する」ことなので、
消費点2の評価自体を`build_input_context`後へ動かす案が考えられるが、
**3つの独立した理由で不可**:

1. **順序がload-bearing**。`kp_stage_shadow_ime_toggle`が書くbeliefが
   そのまま`build_input_context`の`ime_on`になる。消費点2を後段へ動かす
   と、belief OFFの打鍵で`ctx.ime_on=false`のままengineに入り、
   `compute_state`が`Inactive(ImeOff)`を返して**消費点1にも到達しない**。
   誰もIMEを開けず、観測不能アプリ（UWP等）では恒久固着する
   （BUG-115の元症状そのもの）。
2. **消費点2は同時打鍵チョードにも必要**。belief OFFで親指＋文字を
   同時押しした場合、親指KeyDown時点でbeliefをONにしないと、直後の
   文字キーが`ime_on=false`のctxで処理されpassthroughに落ちる。
   消費点1（単独タップ確定後）に遅延すると最大100msのあいだbeliefが
   staleになる。
3. **消費点1は単独タップ限定**。消費点2はauto-repeatを含む毎KeyDownで
   走る。両者は入力空間そのものが違い、「同じタイミングに揃える」という
   操作自体が定義できない。

したがってマーカー方式（決定1〜3）を採る。ADR-153「未決着#9」の主題は
設定名（`solo_tap`）の再考であり、本却下の根拠はあくまで上記1〜3。

## テスト（実装済み）

`.claude/rules/fix-requires-evidence.md`の「キー選択」「IME belief」
両ファミリーに該当するため、(a)回帰テストと(b)`docs/known-bugs.md`
追記の両方を満たした。

- **(a-1) エンジン側ユニットテスト**（`src/engine/nicola_fsm.rs`、
  `cargo test --lib`でホストターゲットで回る）: 既存の
  `explicit_ime_action_consumed_marker_*`3本の姉妹として
  `auto_delegate_open_axis_consumed_marker_*`3本を追加。マーカーtrue時に
  delegateが発火しないこと、マーカーfalse時は従来どおり発火すること
  （対照テスト）、`ModeKeyConfig=Passthrough`でもフォールスルーしない
  こと（BUG-123と同型の再発防止）を固定。
- **(a-2) `architecture_guard.rs`のgrepガード**（Linuxで回る）:
  `auto_delegate_open_axis_consumed_marker_is_set_only_when_shadow_
  toggle_writes_belief`（`if delegate_owned`側に出現しないこと・
  `if !delegate_owned`側に実際に配線されていることの両方を固定）、
  `transport_plan_never_reads_auto_delegate_open_axis_consumed`
  （follow-only原則を機械的に固定する主要ガード）。
- **(a-3) `transport.rs::plan_tests`**（Windows CI）:
  `henkan_muhenkan_allowed_even_when_auto_delegate_open_axis_consumed`
  ——マーカーだけを立ててもAllowのままであることを固定。
- **(b)** `docs/known-bugs.md` BUG-113節に本ADRの修正を追記。

全テスト（`cargo test --lib`1010件・`cargo test --test scenarios`8件・
`cargo nextest`119件）およびclippy（host/Windows両ターゲット、pedantic/
nursery込み）green。

## 未検証事項の解決（opus-adversarial-consult r1/r2で決着）

1. **ADR-153 M25のマーカーと本ADRのマーカーは同じフィールドで良いか**
   →**別フィールドにする**（決定1）。当初の根拠「`transport.rs::plan`
   のAllow/Suppress判定が効くから流用は物理配送を壊す」は成立しない
   （engine活性時は`Decision::Consume`に乗り`plan`を参照しない）と
   判明したが、正しい根拠（engine非活性経路での破壊シナリオ）に
   差し替えて結論は維持した。
2. **`delegate_owned`の計算自体を遅延できないか**→**却下**（決定4）。
   3つの理由すべてを実コードで裏取り済み。
3. **回帰テスト**→上記「テスト」節に確定版を記載、実装済み。

## 関連

BUG-113、[ADR-141](141-henkan-muhenkan-delegate-inactive-recovery.md)
（排他性の不変条件を明記した当のC2修正）、
[ADR-147](147-thumb-key-delegate-defers-to-user-passthrough.md)、
[ADR-149](149-physical-ime-key-activation-defers-forced-set-open.md)
（「案C」の発見元、必須条件4で`docs/known-bugs.md`への記録を完了済み）、
[ADR-153](153-gji-keymap-aware-safe-vk-substitution-for-mode-keys.md)
（B13/B14が同型の問題を別経路に対して解決、本ADRはその解法パターンの
旧経路への転用）、`.claude/rules/fix-requires-evidence.md`。
