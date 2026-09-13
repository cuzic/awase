---
id: ADR-171
title: |-
  gji_direct_already_matchesが候補ウィンドウ再表示という既存のdesync証拠(candidate_was_seen)を無視して再送を握り潰す不具合を修正する(BUG-141)
status: |-
  起草・opus-adversarial-consult round1〜round4反映済み。round1・round2で
  当初案（belief/drift correction経由の自動補正、決定1-4）にBlocker合計8件が
  見つかり設計を全面転換、round2が提示した最小案（案Z）を主決定として採用。
  round2は「BUG-141のjournal解釈に事実誤認がある」ことも指摘し（M5）、それを
  自分で裏取りして確定させた。round3は案Z自体に2件のBlockerを検出
  （`#[serde(default)]`必須／送信時にラッチを消費する設計への変更）、
  対応の一部として追加した`candidate_visible`（レベル信号）が
  round4で「BUG-113を決定的に再導入する」Blockerと判明し、実ログでの
  裏取りの末に**撤回**した。round4は本ADR自身の記述（BUG-141タイムライン
  トレース、BUG-051追補の因果帰属）にも実ログとの不一致を複数検出、
  すべて反映済み。round5レビュー待ち。実装未着手。
related_adr:
  - "ADR-034"
  - "ADR-080"
  - "ADR-140"
  - "ADR-163"
---

# ADR-171: `gji_direct_already_matches`が候補ウィンドウ再表示という既存のdesync証拠(`candidate_was_seen`)を無視して再送を握り潰す不具合を修正する(BUG-141)

## 背景

[BUG-141](../known-bugs/BUG-141.md)（report `01M2D8HS5SBWSZ221P240Z4ZXE`）で、
Ctrl+無変換を3回押しても、直後の英字入力でGJIの変換候補ウィンドウ
（`GoogleJapaneseInputCandidateWindow`）が3回連続で実際に再表示される
現象が発生した。

### 事実の訂正（当初案・round1の誤読、round2 M5指摘を自分で裏取りして確定）

当初、journalの`ActuationDecision.outcome`だけを見て「Ctrl+無変換
（`SendVk(26)`＝`VK_IME_OFF`）を3回送ったが3回とも無効だった」と記述していたが、
これは誤り。実際にjournal（`ImeOpenApplied`/`ActuationDecision`）を精読すると:

- **1回目（elapsed `6128805`）**: `outcome:Applied`——実際に`VK_IME_OFF`が
  送信された。
- **2回目（`6131393`）・3回目（`6135183`）**: **`outcome:AlreadyMatched`——
  `send_ime_mode_key`は一度も呼ばれていない**（`gji_direct_already_matches`
  が`shadow_on==Some(false)==open`で早期に`None`を返したため、
  `decide_attempt`のGjiDirectアームで`MechanismCommand`自体が生成されない）。

つまり実際に起きていたのは「GJIが3回とも`VK_IME_OFF`を無視した」ではなく、
**「awase自身が2回目以降の再送をshadowモデル任せで握り潰していた」**。
awaseはこの間、候補ウィンドウの実際の再表示（`GjiFsmTransition
{StartComposition}`）という**desyncの直接証拠**を既に持っていた
（`tsf/observer.rs::candidate_was_seen`、下記参照）にもかかわらず、
その証拠を`gji_direct_already_matches`の判定に一切使っていなかった。

さらにjournal/app.logを実測で確認した結果、この区間（elapsed `6128715`〜
`6136419`、absolute `2026-09-13T11:30:03.35`〜`11:30:14.25`）で
**`ir_apply_drift_correction`は補正を1回も出していない**ことを確認した
（app.log全体で`[drift]`行が0件）。**round4訂正**: 当初「周期チェーンが
一度も回っていなかった」と記述したが、これは過大な帰属だった。実際には
`[stage-observe]`ログ（`ir_stage_observe`冒頭でstrategyを問わず無条件に出る）
がこの区間に**5回**出ており、`ir_apply_drift_correction`自体は最低4回実行
されている。補正が出なかった近因は2つ: (a) 走ったtickがすべて
`strategy=SkipTyping`（`ir_decide_read_strategy`の打鍵中ガード、
`idle_ms < TYPING_IDLE_MS=500ms`、Blacklist/TsfNativeは`explicit_verify`の
対象外のため打鍵が続く間は観測を一切書かない）、(b) Blacklistで実際に走った
tickでも`observer_poll=None`（`[gji-poll] GJI I/O …ms ago predates focus
change → skipped`、`observer/gji_observer.rs`のpredates-focusガード）。
`runtime/mod.rs::reschedule_ime_refresh`が「`explicit_intent().is_some()`の
間は次回tickを張らない」という設計（BUG-51と同型の既知の落とし穴）自体は
実在し、区間内のtickがすべて apply 直後の20-50msイベントkickであって
500ms周期tickが1つも無いことの説明にはなるが、**「補正が出なかった」ことの
直接の説明にはならない**——上記(a)(b)がその近因である。本ADRのスコープ外の
**別の既知の穴**として記録する（下記「関連する別の穴」、記述はこの訂正を
反映済み）。

### 検討したが見送った設計（belief/drift correction経由の自動補正）

当初、候補ウィンドウSHOWを新しい`ObservationSource`として belief に流し、
既存のdrift correction機構に自動補正を委譲する設計（決定1-4）を起票した。
opus-adversarial-consultで2ラウンド実施し、合計8件のBlockerが見つかった
（`AnyObservation`構築不能、`platform.rs`にbelief経路が無い、BUG-114型の
無限再武装、観測が焼き付いて訂正されない、`reschedule_ime_refresh`の
explicit-intent停止でdrainが一度も走らない、`OffCold`gateの評価時点ズレに
よる日常操作での誤発火、episode ラッチの配線漏れ、`Imm32Unavailable`の
`ObserverPoll`が構造的に`false`を書けない）。詳細と全指摘は末尾
「検討した代替案（見送り）」参照。**[BUG-033](../known-bugs/BUG-033.md)が
既にこの種の設計を検討し(a)レイテンシ(b)新規observation source追加コストを
理由に見送っていたことも round1 で判明しており、本ADRでも同じ理由に加え
実装難度の高さから見送る。**

## 決定（案Z）: `gji_direct_already_matches`に`candidate_was_seen`を渡し、desync証拠がある場合は再送を短絡させない

### 現状の問題箇所

`state/ime_actuation_decision.rs:142-144`:

```rust
const fn gji_direct_already_matches(shadow_on: Option<bool>, open: bool) -> bool {
    matches!(shadow_on, Some(v) if v == open)
}
```

`shadow_on`（awase自身が最後に送ったコマンドの記録）が`open`（今回の要求）と
一致していれば、実際にOSへ何も送らずに`AlreadyMatched`を返す
（`decide_attempt`、同ファイル`:250`）。この判定は「前回送ったとおりに
GJIが状態を保っているはず」という**awase自身の記録**だけに基づいており、
その後にGJIの実状態が変化したという外部証拠（候補ウィンドウの実際の
再表示）を一切見ない。

一方、awaseは既にこの証拠を`TSF_OBS.candidate_was_seen`
（`tsf/observer.rs:176-181`、doc:「GJI candidateがSHOWになってから次の
`on_ime_applied`呼び出しまでの間に『shadow=OFFなのに候補ウィンドウが
表示された(desync)』ことがあったかを記録するラッチ」）として保持している。
このラッチは`EVENT_OBJECT_SHOW`で`true`に、`on_ime_applied_inner`
（`platform.rs:1518-1527`、`AlreadyMatched`を含む全outcomeで無条件に
リセット、`UnsafeToToggle`/`NotOwned`のみ例外）で`false`にリセットされる
——**次のapply判断が行われる直前まで値を保持し、apply完了後にリセットされる**
ため、「前回のapply〜今回のapply判断の間にSHOWがあったか」を正確に表す
（**round3訂正**: このリセットが「無条件」と言えるのは呼び出し経路によって
は成立しない場合がある。詳細と対策は下記「BUG-113再導入にならない理由」
節を参照——本節の記述はあくまで`platform.rs::on_ime_applied_inner`単体の
挙動であり、それが実際に呼ばれるかのゲートは別に存在する）。

このラッチは既に`ImeControlView`（`state/ime_decision_view.rs::
ObservedState::candidate_was_seen`、`:54`）へスナップショットされ、
`state/ime_decision_view.rs:52-54`/`tsf/observer.rs:179-181`のdocは
現在「`KanjiToggleStrategy`が消費する」とだけ名指ししている。**round3
指摘（m4）: この名指しは古い**——`output/ime_apply_planner.rs`の
`OpenBelief::reduce`（`:68`、`!desired_open && self.candidate_was_seen`）が
計算する`effective_open`の本番消費者は、2026-08-10のdoc訂正（ADR-087 §5
Phase 3 item14）により**現在`platform.rs::apply_ime_open_with_view`の
`tracing::debug!`だけ**であり、`already_matched`判定には使われていない
（診断専用に降格している）。つまり本ADRの案Zは、**この信号を初めて
実際のactuation判断（`gji_direct_already_matches`）へ配線する**ものである
——「既にある配線をもう1本つなぐだけ」という表現は実態より楽観的だったため
訂正する。既存のdoc（`tsf/observer.rs:179-181`、
`state/ime_decision_view.rs:52-54`）の「`KanjiToggleStrategy`が唯一の消費者」
という記述も、GjiDirectを2人目の消費者として追加する実装時に更新すること
（`candidate_was_seen`を複数の判断サイトが読むこと自体は
`ObservedState::from_snapshot`のdocが想定する使い方であり、問題ない）。

`impl From<&ImeControlView<'_>> for DecisionInputs`
（`state/ime_decision_view.rs:152-159`）は現状この値をコピーしておらず、
`GjiDirectStrategy`が使う`gji_direct_already_matches`には届いていない。
これが本バグの直接原因である。

### 変更内容

1. `DecisionInputs`（`state/ime_actuation_decision.rs:45-51`）に
   `#[serde(default)] candidate_was_seen: bool`フィールドを追加する
   （**round3 B1: `#[serde(default)]`は必須**——同型は`serde::Deserialize`を
   導出しており、ADR-163の凍結リプレイコーパス
   （`crates/awase-windows/tests/journals/actuation_decision/
   bug-131-report-01m29kdnz.json`等、37レコード）が`DecisionInputs`をJSONから
   復元する。`#[serde(default)]`が無いとフィールド追加だけで
   `replay_all_actuation_decision_fixtures`が既存fixtureのパース失敗で
   panicする。同型の前例は`bug_report.rs:456-472`
   （「旧バージョンが生成した診断JSONにはこのフィールドが存在しない、
   `#[serde(default)]`必須」というdoc付き）。付ければ既存37レコードは
   `candidate_was_seen`を持たないため`default=false`で復元され、
   `!(!open && false)`は常に`true`＝旧実装とビット同値のまま**差分ゼロで
   再生される**——ADR-163 TH1eの複雑性予算制発効条件（決定・統合の
   差分ゼロ再生証明）にも抵触しない。
2. `state/ime_actuation_decision.rs:30-43`の「この型のフィールドを増やす前に
   読むこと（ADR-163決定D8）」docに明示的に応答する（**round3 M1**）:
   `candidate_was_seen`はbool 1個で、アプリ名・打鍵内容・class_name等の
   PIIを一切含まない。bug report（ADR-095）の`journal_json`スキーマが
   1フィールド増えるが、D8が警告する「除外という防壁を素通りする」ケースには
   当たらない。副次的な利点として、この追加により`ActuationDecision`レコード
   に`candidate_was_seen`が載るため、**override（後述）が効いた瞬間が
   bug reportからそのまま読める**（実機A/Bの判定材料が自動で手に入る）。
3. `impl From<&ImeControlView<'_>> for DecisionInputs`
   （`state/ime_decision_view.rs:152-159`）で
   `candidate_was_seen: view.observed.candidate_was_seen`をコピーする。
4. `gji_direct_already_matches`を次のように変更する:

   ```rust
   const fn gji_direct_already_matches(
       shadow_on: Option<bool>,
       open: bool,
       candidate_was_seen: bool,
   ) -> bool {
       matches!(shadow_on, Some(v) if v == open) && !(!open && candidate_was_seen)
   }
   ```

   **round4訂正（重要）**: round3では、`output/ime_apply_planner.rs:68`の
   `self.shadow_on || self.candidate_visible || (!desired_open &&
   self.candidate_was_seen)`という前例に倣い、レベル信号`candidate_visible`
   も同じ形でORに加える案を検討・一度採用した（「候補ウィンドウが開いたまま
   Backspace無しで再度押す」ケースを救うため）。しかし round4 で
   `candidate_visible`を**落とすことに決定した**——実ログで裏取りした結果、
   `candidate_visible`をORに含めるとBUG-113を**決定的に再導入する**ことが
   判明したため（詳細は下記「BUG-113再導入にならない理由」節）。
   `candidate_was_seen`のみを使う。
5. `#[cfg(test)]`の`inputs()`ヘルパー（同ファイル`:273-286`）と、
   直接`DecisionInputs { .. }`を書いている他の全構築サイトに
   `candidate_was_seen: false`（既定値、既存挙動を変えない）を追加する。
   **round3 m1で判明した漏れ**: `state/actuation_decision_record.rs:614`の
   `inputs()`テストヘルパー（`ime_actuation_decision.rs`の同名ヘルパーとは
   別物）も対象に含める。本番構築サイトは`state/ime_decision_view.rs:152`の
   `From`実装1箇所のみ（round3で確認済み）。

### 実際のBUG-141タイムラインでの動作確認（トレース済み、round4で実ログと突き合わせて修正）

report `01M2D8HS5SBWSZ221P240Z4ZXE`の生app.logで実測した各押下時点の値
（round4指摘M1）:

| 押下 | 実ログ時刻 | outcome | `candidate_was_seen` |
|---|---|---|---|
| 1回目 | `11:30:03.446` | `Applied` | `false`（初回、SHOWはまだ無い） |
| 2回目 | `11:30:06.034` | 現状`AlreadyMatched` | `true`（直前のSHOW`11:30:05.050`で立った） |
| 3回目 | `11:30:09.824` | 現状`AlreadyMatched` | `true`（直前のSHOW`11:30:07.904`で立った） |

**この3回の押下は`candidate_was_seen`だけで全て救われる**
（round3で検討した`candidate_visible`は、この特定のincidentでは
一度も必要としていなかった——round4 M1）。

1. Ctrl+無変換1回目（`candidate_was_seen=false`、初回のため）:
   `shadow_on`が`open(false)`と不一致 → 通常どおり送信、`Applied`。
2. 候補SHOW（`11:30:05.050`）→ `candidate_was_seen=true`。
3. Ctrl+無変換2回目: `shadow_on==Some(false)==open`だが
   `candidate_was_seen=true` → already-matchedと判定されず**実際に再送する**
   （旧実装ではここで無送信だった）。送信直後に`candidate_was_seen`を消費
   （後述「BUG-113再導入にならない理由」）。
4. 候補SHOW（`11:30:07.904`）→ 再び`candidate_was_seen=true`。
5. Ctrl+無変換3回目: 同様に再送する。
6. 4回目の`C`,`H`,`A`入力で候補は表示されず（実際のjournal通り）。

**この変更により、ユーザーが実際に押した3回のCtrl+無変換が3回とも実際に
GJIへ送信されるようになる**（旧実装では1回のみ）。GJI側が本当に受理する
かどうか（Mozc/Chromiumの`OnSetFocus`無条件上書き機序、BUG-141背景参照）は
依然awaseの管理外だが、少なくとも**awase自身がユーザーの意思を握り潰す**
という、この変更で確実に解消できる部分が直る。

### BUG-113再導入にならない理由（round3 B2/round4 B1で訂正、送信時にラッチを消費する。`candidate_visible`は不採用）

BUG-113は「同一の物理キー押下に対し`shadow_toggle_off_sync`/
`engine_decision_sync`の2経路から連続で2回`apply`が呼ばれ、2回目も
実送信していた」ことが原因（`ime_controller.rs:88-100`のdoc）。

**round3訂正**: 当初「`candidate_was_seen`は`on_ime_applied_inner`が全
outcomeで無条件にリセットするから安全」と説明したが、これは一般命題として
不成立と判明した。実際には (a) リセットは`acceptance == Accepted`
（`state/ime_model.rs:49-51`）のときにしか走らない
（`runtime/mod.rs:664-671`、`Stale`/`Superseded`/`NotSent`は素通り）、
(b) executor経路（`runtime/executor.rs::dispatch_ime_set_open`）の完了は
バッチ内の全effectを実行し終えた後にまとめて処理される
（`runtime/executor.rs:277-332`→`runtime/mod.rs:596`）一方、view はeffectご
とに新しく構築される（`runtime/executor.rs:826`）。したがって同一バッチ
内に2つの`SetOpen`effectがある場合、1個目の送信後もリセットがまだ走らず、
2個目のviewも同じ`candidate_was_seen=true`を見て再送しうる。

**決定（round3推奨案を採用）**: リセットのタイミングに依存せず、
**ラッチを「override送信に使った時点で即座に消費する」**。
`ime_controller.rs::apply_mechanism`のGjiDirectアーム
（`:273`で始まり`:276`の`send_ime_mode_key(vk)`の戻り値`true`が唯一の
成功条件、round4で範囲確認済み）で、`open==false`かつ実際に送信が成立した
場合、その場で`crate::tsf::observer::reset_candidate_was_seen()`
（既存のpub(crate)関数）を呼ぶ。

**round4 M3への対応（配置の選定）**: `ime_controller.rs:30-33`のモジュール
doc「## アーキテクチャ制約」は「このモジュールは観測値を自ら読んではならず、
すべて`ImeControlView`経由で受け取ること」と定めており、
`reset_candidate_was_seen()`は読み取りではなく観測グローバルへの書き込み
だが、同じ規律が適用されるべき対象である。検討した配置案:

- (a) `platform.rs::apply_ime_open_with_view`（既存の唯一の呼び出し元
  `platform.rs:1527`と同じファイル）で行う。ただしasync経路
  （`runtime/open_chain.rs::fallback_write`→`apply_mechanism`）はこの関数を
  通らないため、GjiDirectがasync側に来る場合（ImmCrossが先に失敗した場合の
  み到達）で消費が漏れる。
- (b) `apply_mechanism`のGjiDirectアームに置き、モジュールdocに明示的な
  例外を1行追記する。同期・非同期の両writerから必ず通るため漏れが無い。
- (c) `apply_mechanism`の戻り値/`AttemptRecord`に「overrideを消費した」
  事実を載せ、2つのwriter実装側で消費する。漏れは無いが変更点が増える。

**(b)を採用する**——`Imm32Unavailable`プロファイル（本ADRが対象とする
GJI環境）ではImmCrossが`is_applicable`で到達しないため(a)の漏れは
発生しないと考えられるが、(b)はその前提に依存せず全経路を機械的に
カバーできる。実装時に`ime_controller.rs:30-33`のモジュールdocへ
「`reset_candidate_was_seen()`の呼び出しはこの制約の例外（読み取りでは
なく書き込みであり、GjiDirectのOFF方向override消費専用）」という1行を
追記すること。

**round4 B1（Blocker、実ログで確定）: `candidate_visible`は同じ理屈で
安全化できないため不採用に変更した。** round3で一時追加した
`candidate_visible`は、awase側で消費するタイミングを持たない**レベル信号**
であり、GJI側の実composition状態が変化する（`EVENT_OBJECT_HIDE`が配送
される）まで`true`のまま推移する。実ログで確認した候補ウィンドウの可視
時間は**0.6〜3.2秒**（例: `11:30:07.904`SHOW→`11:30:11.066`HIDE、3162ms）。
一方BUG-113の2経路dispatchは**同一バッチである必要がなく**、
「物理IMEキーを`effective_open()==true`の状態で押す」だけで成立する
（`shadow_toggle_off_sync`が同期即時で1回目のapplyを行い
`candidate_was_seen`はここで消費されるが、続く`engine_decision_sync`側の
`SetOpen`effectが2回目のapplyを行う時点で、両者の間隔は実測**約15ms**
——`11:30:03.446378`→`03.461231`——であり、この間にHIDE（0.6秒以上先）が
入る余地は事実上無い。したがって`candidate_visible`をORに含めると、
「物理IMEキーを変換中（候補可視）に押す」という**BUG-113の実際の再現条件
そのもの**でBUG-113を決定的に再導入する。`candidate_was_seen`の送信時消費
はこの経路を防げない——`candidate_visible`という別の項がその場で`true`の
ままだからである。**この理由により`candidate_visible`のOR追加を撤回した**
（決定4参照）。同一バッチ内の重複`SetOpen`effectをデデュープする既存機構も
無い（`dispatch_ime_set_open`の早期exitはInputRelayゲート1つのみ、
`runtime/executor.rs:829-862`）。

**残る既知の限界（round4 M4、受容する）**: `candidate_was_seen`のみに
絞った結果、「候補ウィンドウが開いたままBackspace無しで再度Ctrl+無変換を
押す」（新しいSHOWイベントが発火しない）操作は救われない——次の押下は
`AlreadyMatched`に戻る。これはBUG-113の決定的な再導入という代償と比べて
受容すべきトレードオフである。恒久的な解（候補が開いたままであることを
継続的に検知して自動収束させる）は、本ADRが見送ったbelief/drift
correction経由の設計の再検討にあたる（BUG-033がADR-171に予約したのと
同じ形で、次のADRへ予約する）。

### テスト（`.claude/rules/fix-requires-evidence.md`「キー選択」ファミリー）

- `state/ime_actuation_decision.rs`の既存テスト
  `gji_direct_skips_when_shadow_already_matches_close_direction`
  （`:559-568`、round3で行番号訂正、`candidate_was_seen`無しの既存挙動、
  暗黙に`false`）は変更後もそのまま緑であることを確認する
  （デフォルト`false`なら旧動作とビット同値）。
- 新規テスト`gji_direct_resends_when_candidate_was_seen_despite_shadow_
  match`（同ファイル）: `shadow_on=Some(false)`, `open=false`,
  `candidate_was_seen=true`で`decide_attempt`が`Some(MechanismCommand::
  SendVk(..))`を返すことを固定する。
- `open=true`方向（ON時）は`candidate_was_seen`を条件に含めない
  （`!(!open && ..)`の`!open`ガードにより`open=true`のときは常に`false`
  側に落ち、既存の挙動と変わらないことを対称テストで固定する）。
- **round4 m1への対応**: 消費（`reset_candidate_was_seen()`）は
  `open==false`のときだけ行う——ON方向の送信で消費すると、ON→OFF切替の
  直後に立ったSHOWの証拠をON側のapplyが誤って食べてしまう。
  「`open=true`の送信は`candidate_was_seen`を消費しない」ことを固定する
  テストを1本追加する。
- **round3 M2への対応**: BUG-113の不変条件（OFF方向の同一キー押下で
  二重送信しない）を守る機械可読な検査は、`candidate_was_seen==false`の
  場合しか存在しなくなる（`decide_attempt`は純関数のため「2回目は送らない」
  という時間依存の性質はここでは表現できない）。「送信時にラッチを消費する」
  実装については、`architecture_guard.rs`のテキスト走査で
  「`reset_candidate_was_seen(`の呼び出し箇所数＝2
  （`platform.rs:1527`と`ime_controller.rs`のGjiDirectアーム）」を固定する
  （Linux上で走る）。
- **実機A/B（developマージ前、必須。round4 m5で最小再現手順を訂正）**:
  BUG-113の症状（Windows Terminal × GJI × PSReadLineで余分な「@」）が
  再発しないことを確認する。実ログ解析の結果、本reportではBUG-113型の
  2経路dispatchは一度も起きておらず（1回目のCtrl+無変換は
  `dispatch_ime_set_open`単独、`11:30:07.402`の物理`vk=0xF3`は
  `effective_open()`が既に`false`のためno-opで`apply`に至らない）、
  2経路dispatchが起きるのは**「物理IMEキーを`effective_open()==true`
  （変換中＝候補ウィンドウ可視）の状態で押す」**ときである。最小再現手順は
  「変換中（候補ウィンドウ可視）に半角/全角キー（またはF3等の物理IMEキー）
  を1回押す」——Ctrl+無変換ではなく**物理IMEキー**であることが要点。
  BUG-141の再現条件（GJI長時間idle→Ctrl+無変換→候補SHOW）とは別のシナリオ
  なので、実機A/Bはこの2つを両方カバーすること。

## 残存リスク: この状況ではdrift correctionが構造的に空振りする（BUG-51の別プロファイル再発を含む2層構造、記録済み・修正は別ADR）

**round4で因果帰属を訂正**: `ir_apply_drift_correction`は本reportの区間内で
最低4回実行されているが、補正を1回も出していない（`[drift]`行0件）。
近因は2層構造:

1. **打鍵中は観測を一切書かない**（`ir_decide_read_strategy`の
   `strategy=SkipTyping`ガード、`idle_ms < TYPING_IDLE_MS`=500ms。
   `explicit_verify`による迂回は`!skip_imm_query`を要求するため
   Blacklist/TsfNativeは構造的に対象外）。
2. **Blacklistで実際に走ったtickでも`observer_poll=None`**
   （`observer/gji_observer.rs`のpredates-focusガード、`[gji-poll] GJI I/O
   …ms ago predates focus change → skipped`）。

加えて、`runtime/mod.rs::reschedule_ime_refresh`が「`explicit_intent().
is_some()`の間は次回tickを張らない」という設計（`runtime/mod.rs:834-855`）
——**新しい穴ではなく[BUG-051](../known-bugs/BUG-051.md)
（`fix_commits: ["21ca84d1"]`で「修正済み」と記録されている既知バグ）の
未修理な別プロファイルでの再発**（round3 M4、round4で因果帰属を訂正）——
により、500ms周期のtickが1つも無く、区間内のtickはすべてapply直後の
20-50msイベントkickだった。**この停止自体は実在するが、上記1・2が「補正が
出なかった」ことの直接の説明である**——タイマーを蹴るだけ（BUG-051の対策）
では1・2は解消しない。BUG-051に追補として実測を追記済み
（因果帰属も訂正済み）。**`fix_commits`が付いているからといって
「解決済み」と結論づけないこと**という教訓自体は変わらない。

これを**本ADRの残存リスクとして明示的に格上げする**理由: 案Zは「awase自身の
握り潰しをやめる」だけで、GJI側が実際に受理したかの自動確認・自動収束は
提供しない。「送ったOFFが効いたかを確認して再送する」唯一の安全網である
drift correctionが、まさにBUG-141が起きた状況（打鍵が続く間、
Imm32Unavailable×GJIでは観測が構造的に1件も書かれない）で常に空振りする
ことが実測で判明した以上、**案Zの効果は「ユーザーが物理キーを押した回数
だけ、確実に送信されるようになる」ことに限られ、それ以上の自動回復力は
無い**、という事実として読者に伝わるようにする。修正自体は本ADRのスコープ
に含めない（案Zの正しさに依存しない別軸の修正のため）が、必要になった
場合は上記1・2の解消（BUG-051への追補実装）、または本ADRが検討した
belief/drift correction 経由の設計の再検討として、別ADRを起票すること。

## 検討した代替案（見送り）: belief/drift correction経由の自動補正

以下は当初案の要約。実装しないが、将来同種の検討をする際に同じ轍を
踏まないよう記録する。

**方針**: 候補ウィンドウSHOWを新しい`ObservationSource`（evidence型5点セット:
`ObservationSource`variant追加・`declare_evidence!`・witness構築子・
`PerSourceObservations`フィールド・全数テスト更新）として`ObserverReported`
経由でbeliefへ流し、既存の`check_drift_correction`/`ir_apply_drift_correction`
に自動補正を委譲する。

**round1で見つかったBlocker（4件）**:
1. `AnyObservation`は`Observed<E>`のwitness経由専用で、ADRが書いていた
   コード片（`at`フィールド等）は実在せず構築不能。
2. ディスパッチ先として想定した`platform.rs`は`platform_state`（belief）を
   一切持たず、そこからbeliefへ書き込めない。
3. 新しい観測は`AnyFreshEvidence`除外リストに入らず、BUG-114で実機確認済みの
   「Blindバーストの無限再武装」を再現する（3秒クールダウンごとに再武装、
   BUG-141相当のセッションで最低5回・境界を跨げば10回の自動送信）。
4. confidence=Highを選ぶと、`Imm32Unavailable`にこの観測を上書きできる
   同等以上の観測源が構造的に存在せず、一度発火すると belief が
   「IME ON」に永久に焼き付く。

**round1の指摘を反映してdrain経路をGjiFsm外・`TIMER_IME_REFRESH`ベースに
再設計し、confidenceをMediumに変更した round2 でも、新たに4件のBlocker
が見つかった**:
1. `TIMER_IME_REFRESH`の周期チェーンは`explicit_intent().is_some()`の間
   停止する（前述、BUG-51と同型）ため、本ADRが対象とする状況で新しいdrainが
   一度も走らない。
2. `OffCold`gateをdrain時点で評価するため、「日本語を打ってからIMEを切る」
   という日常操作のたびに5連射を誘発する偽陽性がある。
3. episode ラッチ（`decide_conv_inference_drift`の流用）は書き込み側
   （`runtime/ime_refresh.rs:834`）の配線漏れでno-opになり、かつ
   明示意図なしのケースでは「プロセス起動中ずっと1回だけ」という
   恒久抑止になり、決定4後半（明示意図なしでも補正する）と正面衝突する。
4. confidenceをMediumにしても、`Imm32Unavailable`唯一の`ObserverPoll`観測源
   （`observer/gji_observer.rs:28-62`）は構造的に`Some(false)`を返す分岐を
   持たず、値としての訂正力がゼロ——「後続の観測が自然に上書きする」という
   前提が成立しない。

加えてround2は、この設計を導入する前にBUG-141の journal で「既存のdrift
correctionは既に発火していたのか」を確認すべきだと指摘した（本ADRの
「事実の訂正」節で確認済み: `ir_apply_drift_correction`自体は最低4回実行
されていたが、補正を1度も出していなかった——round4で因果帰属を訂正済み、
理由は上記「残存リスク」節）。

**結論**: 8件のBlockerを全て解消するコストは、案Zの1関数1パラメータ追加という
コストと比べて見合わない。将来「awase自身の再送だけでは不十分（GJI側が
本当に受理したかを確認して自動収束させたい）」という実害が実機で確認された
場合に、この設計を再検討すること（BUG-033がADR-171に予約したのと同じ形で、
ADR-171が次のADRに予約する）。
