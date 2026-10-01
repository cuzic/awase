# 明示キーの「固着ゼロ」保証: 省略・取りこぼし経路の監査と仕様案

対象: develop `042b5ee8`(ADR-213 P2a〜P2d-2 マージ済み)。worktree `~/rust-nicola-wt/audit-liveness`(`audit/liveness`、残置)。
読み取りのみ。行番号は develop `042b5ee8` のもの。**推測**は明記する。

## 0. 要約(仕様の言い換え)

所有者の保証は、次の**1押下ごとの配送不変条件**に言い換えられる。

> **INV-L1(配送の過不足なし)**: 明示キーの非リピート物理 KeyDown 1回につき、IME へ届く「開閉の作用」はちょうど1つ。
> - 物理キーそのものが IME に届く(Allow で、IME が自分で処理する)か、
> - awase が書く(Suppress/Consume して、`VK_IME_ON/OFF`・`ImmSetOpenStatus`)か、
>
> のどちらか一方だけ。**両方**は二重 actuation(BUG-46/52/113)、**どちらも無し**は「二重の空振り」で、固着の素になる。
>
> **INV-L2(収束)**: 絶対指定キー(ON 専用/OFF 専用)は、INV-L1 で届いた作用の向きがキーの意味と一致する。したがって1回で一致する。トグルは、作用の向きが belief から決まるので、belief が古いと1回目は逆になりうる。ただし各押下で INV-L1 が成り立ち、作用の後に belief が「書いた向き」になる(=トグルの次の向きが反転する)ので、2回目で必ず一致する。

現状、**INV-L1 が「どちらも無し」になる経路が 6 つ**ある(§2)。そのうち**押下で内部状態が変わらない不動点(=固着)**は、確定 2 つ・条件付き 2 つ(§3)。ADR-208 の stale applied はその1つにすぎない。最小の設計(§4)は次の3点。

1. Engine の明示 SetOpen にも、shadow toggle と同じ「押下の書き込みでは、`applied` を省略の根拠にしない」を適用する。BUG-113 は押下 id で守る。
2. `is_japanese_ime()==false` を、明示キーの書き込みの授権・shadow の昇格の条件から外す。
3. ImmCross の shadow no-op では、物理が Suppress されるので awase が書く。

検証は、`PhysicalKeyDisposition::plan`・shadow の昇格・warrant・`decide_attempt` を合成した純粋関数 `explicit_press_delivery` を `state/` に切り出し、状態を全列挙して INV-L1/L2 を Linux で固定する(§5)。

---

## 1. 明示キーの分類表

凡例:
- **Phys** = 物理キーの配送(`runtime/transport.rs::PhysicalKeyDisposition::plan`、`:208-336`)。Consume = エンジンの `Decision::Consume`。
- **Write** = awase の書き込み経路。
- 窓: **IC** = ImmCross/Plain/Unknown(`can_use_imm32_cross_process`)、**IU** = Imm32Unavailable(Chrome/Edge)、**TN** = TsfNative(WT/WezTerm)、**IR** = InputRelay。
- IME: GJI / MS(MS-IME。ATOK 等の非 GJI も `ImeKindId::MsIme` に推定される、`state/ime_kind.rs:19-24`)。

| キー | 意味 | 経路 | Phys | awase が書く条件 |
|---|---|---|---|---|
| **VK_IME_ON/OFF(0x16/0x1A)** | 絶対 | shadow toggle(`key_pipeline.rs` の `kp_stage_shadow_ime_toggle`、ADR-207 で `is_japanese_ime` を問わない、`:949-956`) | IC: Down/Up とも常に Suppress(`transport.rs:290-294`)。IU/TN: belief が変わる押下(`shadow_toggled`)は Down を Suppress。**no-op(belief が既に一致)は Down を Allow**(`:326-329`)、Up は常に Suppress。IR: Allow | belief が `!target` → `target` に倒れたとき、`kp_shadow_actuate(target)`(`:1149`)。no-op では書かない(`:1033` の分岐) |
| **半角/全角 0xF3/0xF4(役割 Toggle)** | トグル | shadow toggle(`is_japanese_ime` 必須、ただし 0xF0-F4 の物理受信で即 true に上げる、`:919-921`) | IC: Suppress。IU/TN: **Down は shadow に関わらず常に Suppress**(`is_role_toggle_hz_key_down`、`:341`)。IR: Allow | 常に(トグルは no-op にならない)。warrant が通れば |
| **漢字 0x19(VK_KANJI)** | トグル | shadow toggle(`is_japanese_ime` 必須。**0x19 は上げ対象外**、`vk.rs:328`) | IC: Suppress。IU/TN: shadow なら Down Suppress、shadow が無ければ Allow(役割 hz ではない) | `is_japanese_ime` が true のとき |
| かな/ひらがな **F2(VK_DBE_HIRAGANA)** | 絶対(ON+ひらがな) | 物理のみ(BUG-173) | 全窓 Allow(`:235-237`) | 書かない(IME 自身が処理) |
| 英数 0xF0・カタカナ 0xF1 | (モード) | 物理のみ(ADR-191) | Allow | 書かない |
| **Ctrl+変換 / Ctrl+無変換**(`keys.ime_on/off` の既定) | 絶対 | Engine 特殊キー → `ime_set_open_effects`(`src/engine/engine.rs:856` 付近)→ `SetOpen(ExplicitUserAction)` → `kp_stage_post_decision` → `handle_engine_set_open`(chord フィルタ)→ executor `dispatch_ime_set_open`(`executor.rs:694`) | Consume | 常に SetOpen が出る(belief と一致していても `ime_set_open_effects` が無条件で足す)。実送信は、gate・warrant・already-matched を通れば |
| **F13 等の `keys.ime_on/off`** | 絶対 | 同上(Engine 特殊キー) | Consume | 同上 |
| **sync_direction キー**(`keys.ime_detect` 等) | 絶対(設定どおり) | shadow toggle(SyncKey) | IU/TN: 書く押下は Suppress、no-op は Allow(IME がそのキーをどう扱うかは IME の設定次第)。IC: is_kanji_event なら Suppress | 0x16/0x1A と同じ |
| **無変換/変換の単独タップ**(ADR-206、`bare_ime_action`/`forced_open_action`) | 絶対(OFF は常に `SetOpen(false)`) | Engine FSM の単独確定 → SetOpen(ExplicitUserAction)。KeyUp で確定、またはタイムアウト → `execute_from_loop` | Consume(開閉を書く打鍵) | Ctrl+変換 と同じ |
| 無変換/変換(役割なし・GJI が処理) | (IME 次第) | 物理のみ(BUG-115) | Allow | 書かない |
| `[[keymap]]` で IME キーへ写像 | 写像先次第 | awase 自身の注入出力 → hook は self-injected を無視、`plan` は injected を Allow(`:257-264`) | 注入された VK が IME に届く | 書かない(IME が処理) |

**InputRelay**: すべて Phys=Allow(`:216-218`)だが、Engine のコンボは Consume のまま、SetOpen は `NotOwned`(`ime_controller.rs:599`)。→ §2 L-5。

---

## 2. 書き込みが省略・棄却・取りこぼされる全箇所

INV-L1 の「どちらも無し」は ✗、「片方だけ」は ○。「固着」= 同じ内部状態のまま、何度押しても ✗(§3)。

| # | 箇所 | 発火条件(内部状態) | 対象キー | 押下の結果 | 固着か | 既知/未知 |
|---|---|---|---|---|---|---|
| **L-1** | GjiDirect の already-matched: `ime_controller.rs:334-353`(判定 `ime_actuation_decision.rs:167-173`) | GJI × 窓の chain が GjiDirect 先頭(IU/TN、`state/app_ime_policy.rs::caps`)× `applied == Some(target)` × (open か `!candidate_was_seen`) | **Engine 経由の絶対キー(Ctrl+変換/無変換・F13・単独タップ)**。executor が `applied_snapshot` をそのまま渡す(`executor.rs:702`) | Consume + AlreadyMatched → ✗ | **固着**(§3 S-1) | 既知(ADR-208)。shadow 経路は #408 の `shadow_toggle_demotes_applied`(`:160`)で解消済み |
| L-2 | 同上、shadow 経路 | `applied==Some(open)` | shadow のトグル/絶対 | 降格して書く → ○ | 解消済み | #408 |
| **L-3** | `issue_open_warrant` の `!ctx.is_japanese_ime → None`(`state/open_warrant.rs:136`)→ Unwarranted(`ime_controller.rs:624`、async は `open_chain.rs:673`) | `belief.is_japanese_ime()==false`(probe の誤答、HKL がワーカースレッド由来。ADR-207 のコメント、`key_pipeline.rs:952-954`) | **全ての明示書き込み**: Engine のコンボ(Consume)と、0x16/0x1A(ADR-207 で shadow は昇格するが、書き込みが Unwarranted。物理は `shadow_toggled` で Suppress) | ✗ | **固着**(コンボ、§3 S-2)。0x16/0x1A は1回目が ✗、2回目は no-op で Allow になり IME が処理 → 「絶対指定は1回」の違反 | **未知**(推測: ADR-207 は shadow の昇格だけを直し、warrant 側の同じ条件を残した) |
| L-4 | shadow の昇格に `is_japanese_ime` が必要(`key_pipeline.rs:957-964`) | `is_japanese_ime==false` × キー 0x19(0xF0-F4 は物理受信で上がる、`:919-921`) | 0x19 | IC: Suppress(`transport.rs:290-294`、`is_kanji_event`)+ 書かない → ✗。IU/TN: Allow → ○ | **IC で固着**(S-2 と同じ根) | 未知 |
| **L-5** | InputRelay: `decide_gate` NotOwned(`ime_actuation_decision.rs:125-131`、`ime_controller.rs:599`) | プロファイル IR | Engine のコンボ/単独タップ(Consume) | ✗ | 固着(この窓にいる限り) | 既知(issue #136/ADR-119 の設計)。所有者の「全窓で必ず書く」と衝突 → **決定が要る** |
| L-6 | shadow no-op(`key_pipeline.rs:1026-1079`): belief が既に target | IC × 絶対キー(0x16/0x1A・sync)× belief==target × 実 IME≠target | IC は物理 Suppress、書かない → ✗ | **drift correction (a) が観測で救うときだけ直る**(明示意図 = target、ImmCross の読みが実状態を返す)。観測が読めない・誤る IC 窓では固着(§3 S-3、条件付き) | 未知(推測) |
| L-7 | warrant Step 1 を外したとき(`record_explicit_intent` は `current_focus==None` で no-op、`platform_state.rs:1474-1482`)の Step 3 の観測との食い違い | IC × `current_focus==None` × 鮮度 3s 以内の Actuating 観測 ≠ target | shadow(Blind は Step 4c の `desired_open` = 直前の書き込みで一致するので通る) | Unwarranted → ✗。トグルなら belief だけ反転し、次の押下で逆向きが通る(書く向き=観測と同じ → 実質変わらず)→ 交互に ✗/空書き | **条件付き固着**(S-4、推測。`current_focus==None` は起動直後(BUG-148 で対処済み)や、デスクトップ等の稀な状態) | 未知 |
| L-8 | `UnsafeToToggle`(Win キー押下中、`ime_controller.rs:274/309`。ImmCross の Aborted/capture 失敗、`open_chain.rs:355`・`executor.rs` の capture 失敗) | Win 押下中・フォーカス世代の不一致 | 全て | ✗(その押下だけ) | 固着しない(状態が変わる) | 既知(BUG-16 追補) |
| L-9 | chord フィルタ(`platform_state.rs:570-584`) | Ctrl 押下中の2回目の Ctrl+無変換 | Ctrl+無変換 | belief は書かないが、**effect は executor へ流れる**(strip 撤去済み、P2d-2)→ L-1 次第 | L-1 に帰着 | 既知 |
| L-10 | Ctrl 救済の破棄(`key_pipeline.rs:179-191`、Ctrl↑ が 50ms 以内なら無変換を破棄) | 「Ctrl+他キー → 無変換 → Ctrl↑」が 50ms 以内 | Ctrl+無変換 | ✗(意図された誤打の破棄) | 固着しない | 既知(設計) |
| L-11 | ImmCross の async で Failed → 再読みが一致して AlreadyMatched(`open_chain.rs:364-374`) | 実際の読みで判定 | IC | 実状態が既に一致 → 問題なし | — | 既知 |
| L-12 | outcome=Failed(全機構が失敗)で再試行が無い | 機構の失敗(ハング等) | 全て | ✗(その押下) | 失敗が恒常的なら固着だが、awase の外の要因 | 既知 |
| L-13 | settle | P2d-2 で strip と belief フィルタは撤去済み(`key_pipeline.rs:194-195`)。残るのは drift correction の延期(`runtime/ime_refresh.rs` の `ime_apply_should_defer`)だけで、明示キーには掛からない | — | — | — | — |
| L-14 | 物理 Allow なのに IME が意味どおりに処理しない | MS-IME × 実 Chrome の `VK_IME_OFF`(BUG-172 対照)、sync キーが IME の設定に無い VK | 物理経路 | 届くが効かない | awase の外。ただし awase が書いても同じ VK なので同じ | 既知(BUG-172) |
| L-15 | 「二重の空振り」になりうる Suppress の分岐: IU/TN の役割 hz キー(0xF3/F4)は `shadow_toggled` に関わらず Down を Suppress(`transport.rs:325`) | 役割 Toggle × shadow が昇格しない(`is_japanese_ime` false。ただし 0xF3/F4 は物理受信で上がる)・F13 役割のリピート Down | 0xF3/F4 | 通常は昇格するので ○ | 固着しない(推測: 上げが先に効く) | — |

**MsImeDirect と ImmCross には already-matched の省略が無い**(`ime_controller.rs:277-310`、async ImmCross)。L-1 は GJI × Blind だけ。

---

## 3. 固着(不動点)の状態遷移

押下 P が内部状態 σ を σ' に変え、σ' でも同じ ✗ になる(σ' ≅ σ)とき、固着。

### S-1(確定、ADR-208): GJI × Blind × Engine 経由の絶対キー × stale applied

- σ: `kind=Gji`、`profile∈{IU,TN}`、`applied=Confirmed/Optimistic(target)`、実 IME=¬target(awase を経由せず変わり、予測・ModeKeyPass・観測による降格が起きなかった。BUG-156 の降格は予測の不一致時だけ、`ime_model.rs:879-918`)。
- P(Ctrl+変換 等): `handle_engine_set_open` が desired/last_intent/IntentStore を target にする(既に target でも同じ)。executor → GjiDirect → `AlreadyMatched`。
- 完了: `on_ime_apply_complete(AlreadyMatched, generation)` → `applied` は不変(AlreadyMatched は「確認済み」を降格しない、`platform.rs` の `on_ime_applied`)。
- σ' = σ。Blind なので、drift correction が使える観測は来ない。**固着。**
- 抜けるのは、フォーカス変更(`applied=Unknown`、`ime_model.rs:986`)・shadow toggle のキー(半角/全角)・予測の不一致のときだけ。

### S-2(確定、未知): `is_japanese_ime==false` × Engine 経由の絶対キー(全窓・全 IME)

- σ: `belief.is_japanese_ime=false`(実際は日本語 IME)。P: SetOpen → warrant `None` → Unwarranted。Consume なので IME にも届かない。
- `is_japanese_ime` を true に上げるのは、0xF0-F4 の物理受信(`vk.rs:328`)と probe だけ。コンボの押下では上がらない。**σ'=σ、固着。**
- 0x19 × IC(L-4)も同じ根。0x16/0x1A は2回目に Allow へ落ちて直る(1回保証の違反)。
- 推測: `is_japanese_ime=false` のとき Engine は `NotJapaneseIme` で inactive。ユーザーは「NICOLA が効かない」状態で Ctrl+変換 を押すので、まさに救済が要る場面で効かない。

### S-3(条件付き): IC × 絶対キー × belief==target × 実≠target × 観測で drift (a) が発火しない

- shadow no-op → 書かず、物理は Suppress。救済は drift (a)(明示意図 target、観測 ¬target、閾値0)。
- IC で読み取りが成功していれば直る。読みが失敗/嘘(ハング窓・IME の偽報告)なら固着。
- ADR-212 P6 で drift (a) を「意図の有効期限内の1回」に縮めると、救済が1回に限られる。

### S-4(条件付き、推測): IC × `current_focus==None` × 観測と食い違う向き

- §2 L-7。トグルは「Unwarranted と、実状態と同じ向きの空書き」を交互に繰り返し、実 IME は変わらない。

**不動点を作る次元のまとめ**:

| 次元 | S-1 | S-2 | S-3 | S-4 |
|---|---|---|---|---|
| applied | Some(target) | — | — | — |
| is_japanese_ime | true | **false** | true | true |
| 窓 | IU/TN | 全 | IC | IC |
| IME | GJI | 全 | 全 | 全 |
| belief | 任意 | 任意 | ==target | 反転 |
| 観測 | 無し(Blind) | — | 読めない/嘘 | 鮮度内で ¬target |
| current_focus | — | — | — | None |
| 経路 | Engine(Consume) | Engine/0x16 | shadow no-op | shadow |

pending・last_intent・desired・chord・settle・conv・Engine active は、どの不動点の必要条件にもならない(chord は S-1 に帰着、settle は撤去済み)。

---

## 4. 仕様と最小の設計

### 4.1 仕様(ADR-208 を昇格させるときの決定文)

1. **対象押下**: 非注入(またはテスト目印つき)の非リピート KeyDown で、(a) shadow toggle を昇格させた、または昇格しうるキー(`shadow_action`/`sync_direction`)、または (b) Engine が `SetOpen(ExplicitUserAction)` を出した打鍵(コンボ・単独タップ・`keys.ime_*`)。トレイ等の UI 操作は別に扱う。
2. **INV-L1**: 対象押下ごとに、物理の配送と awase の書き込みのちょうど一方が起きる。awase が書く場合、**belief・applied・is_japanese_ime を理由に省略・棄却しない**。省略してよいのは「同じ押下で既に送った」場合だけ(BUG-113)。
3. **INV-L2**: 絶対指定は1押下で、トグルは2押下以内で、実 IME がキーの意味に一致する。前提: 環境の故障(機構の恒常的失敗、BUG-172)を除く。
4. **例外の明示**: InputRelay(§4.4 の決定待ち)、Win キー押下中(L-8)、Ctrl 救済の破棄(L-10)。

### 4.2 設計(最小、3か所)

**D1: Engine の明示 SetOpen でも、`applied` を省略の根拠にしない(S-1)。**

- 置き場所は、合流点の表(`.claude/rules/fix-requires-evidence.md` の「IME actuation 合流点」)のうち、Engine の明示 SetOpen が通る `executor.rs::dispatch_ime_set_open`(`:694`)。今は SetOpen を出すのが明示操作だけ(ADR-213 P2b/P2c)なので、**この関数に来る SetOpen はすべて対象押下**とみなせる。
- view の `shadow_on` を、shadow 経路と同じ規則(`shadow_toggle_demotes_applied` を一般化した `explicit_press_shadow_on(applied, open) -> Option<bool>`)で未知にする。
- **BUG-113 の両立**: 同じ押下で shadow の書き込み → Engine の SetOpen と2回来る組み合わせ(例: sync キーが `keys.ime_on` でもある、0x16 が `keys.ime_on` に設定されている)がある。そこで、hook が非リピート KeyDown に**押下 id**(単調増加の u64、`RawKeyEvent` に `press_id`)を振る。`ActuationOrder` に `press: Option<PressId>` を載せ、`ImeStateHub` に `last_written_press: Option<(PressId, bool)>` を持たせる。`decide_attempt` の前に「同じ press_id・同じ向きで既に `wrote_open_state()` した」なら `AlreadyMatched` で省く。
  - これは「applied が一致」ではなく「この押下で送った」に基づく省略(ADR-208 論点1)。
  - drift correction 等の押下に由来しない書き込みは `press=None` で、従来の applied 判定のまま。
- `applied` の降格を reducer へ流すかについて。D1 は view だけを未知にし、`applied` そのものは書き換えない。完了時の `record_ime_apply_result` が正しい値を書く。新しい ImeEvent は不要で、`ime_event_guard`・belief の規律に触れない。ADR-208 論点2 の「専用 event」は、view の差し替えで代替できる(#408 と同じ形)。

**D2: `is_japanese_ime` を、対象押下の授権と shadow の昇格から外す(S-2、L-3/L-4)。**

- `issue_open_warrant` に `ActuationOrder` の押下の有無を渡し、`press.is_some()` なら `!is_japanese_ime` の早期 None を飛ばす。
  - Step 1(IntentStore)/4c で、押下が記録した意図と一致して授権される。Step 1 は `current_focus==None` だと外れるので、D3 と合わせる。
- shadow の昇格(`key_pipeline.rs:957`)の `is_japanese_ime()` の条件を、0x19 については外す。または `should_upgrade_is_japanese_ime` に 0x19 の物理受信を加える。ただし 0x19 は US 配列の Alt+` でも出るので、ADR-093 の基準(IME の証拠)に合わない。外すほうを推奨。
- リスク: 本当に英語 IME の窓で、Ctrl+変換 が `VK_IME_ON` を送る。英語 IME では `VK_IME_ON` は無害(冪等で効かない)。推測なので、CI の非日本語の構成で1本確かめる。

**D3: 対象押下では、warrant を押下由来の授権として扱う(L-7/S-4)。**

- 押下で `record_explicit_intent` が `current_focus==None` のため外れる場合に備え、`press.is_some()` の order は Step 1 の代わりに「この押下の意図」(`order.open()` そのもの)で授権する(`WarrantBasis::ExplicitPress`)。
- 押下はユーザーの明示操作で、ADR-090 の warrant が防ぎたい「推測による書き込み」ではないので、授権の根拠になる。

**D4: IC の shadow no-op で書く(S-3、L-6)。**

- no-op 分岐(`key_pipeline.rs:1026`)で、**物理が Suppress される場合**(`plan` が Suppress を返す条件、IC か役割 hz キー)は、`kp_shadow_actuate(target)` を呼ぶ。Allow の場合(IU/TN の 0x16/0x1A/sync)は、物理が届くので書かない(INV-L1 の「ちょうど一方」)。
- これには「plan の判断」を shadow の判断より前に知る必要がある。§5 で `explicit_press_delivery` に一本化し、shadow 側はその結果を参照する形に直す。今は `plan` が `shadow_toggled` を入力に取る循環になっている(`kp_run_inner` で shadow → plan の順)。

**D5(トグルの2回収束)**: D1-D4 で各押下が書く。書いた後は `on_ime_apply_complete` で `applied=Confirmed(書いた向き)`、belief=書いた向き。2回目の押下は belief を反転させて逆向きに書くので、実 IME は「1回目の向き → 2回目の向き」になり、どちらかがキーの意味(=実状態の反転)と一致する。追加の仕組みは不要。§5(a) の全列挙で固定する。

**BUG-124(TN×GJI の単発 `VK_IME_OFF` で「@」)との衝突**:

- D1 で、TN×GJI の Ctrl+無変換・単独タップの OFF は、belief/applied に関わらず毎回 `VK_IME_OFF` を送る。ADR-206 の決定3(b) の撤回と同じ代償で、BUG-124 型の構成を作り直す。
- **D1 の TN×GJI への適用は、WT×GJI×PSReadLine の実機 A/B(押下ごとの `@` の発生率、develop と D1 版、各 n≥30)をマージ条件にする。** A/B で `@` が出る場合の代替は、ADR-206 にある案「同じ OFF キーを2回続けて押したときだけ送る」。これは「2回で一致」に収まるので、所有者の (2) を満たす。

**合流点の配線チェック**(1か所に足して満足しない):

| 合流点 | D1 | D2 | D3 | D4 |
|---|---|---|---|---|
| `ime_controller.rs::apply`(同期) | 押下 id による省略の判定をここ(`decide_attempt` の前)に置く | warrant(`into_actuation`)経由 | 同 | — |
| `open_chain.rs::run_open_chain_async`・`imm_cross_write`・`fallback_write`(非同期) | ImmCross は元々省略しない。`fallback_write` の GjiDirect への落ちで、押下 id の判定を共有する | `open_chain.rs:673` の Unwarranted も同じ warrant | 同 | — |
| `executor.rs::dispatch_ime_set_open` | view の `shadow_on` を未知にする | order に press を載せる | 同 | — |
| `key_pipeline.rs::kp_shadow_actuate` | 既に降格済み。press を載せる | 同 | 同 | no-op 分岐から呼ぶ |
| drift correction(`ir_apply_drift_correction`) | press=None のまま(変更しない) | 変更しない | 変更しない | — |

### 4.3 新しい gate を足さない理由

D1-D3 は、既存の gate(already-matched・warrant)を**緩める**変更で、新しい gate は足さない。押下 id による省略だけが新しい判定で、`decide_attempt` の1か所(同期・非同期とも `decide_attempt` を通る、ADR-163)に置く。

### 4.4 所有者の決定が要る点

- **InputRelay(L-5)**: issue #136/ADR-119 は「この窓は入力面ではなく、awase は actuation を所有しない」と決めている。「全窓で必ず書く」とは衝突する。選択肢は次の2つ。
  - (a) InputRelay ではコンボを Consume しない(PassThrough にして、リレー先の IME に任せる。INV-L1 の「物理が届く」側)。
  - (b) 例外として明記する。

  推奨は (a)。`Decision` を profile で変えるのは layer の規則(Engine は profile を知らない)に触れるので、Platform 側で「NotOwned になった SetOpen の元の打鍵を reinject する」形にする。推測。実装の可否は要検討。

---

## 5. 検証設計

### (a) Linux の網羅テスト(純粋な決定関数)

**切り出す関数**(新設 `state/explicit_press.rs`、ungated):

```text
fn explicit_press_delivery(s: &PressState, key: ExplicitKey) -> Delivery
  Delivery { physical: Allow|Suppress|Consume, write: Option<bool>, reason: Elision }
```

これは、今ある次の部品の合成にする。

- `transport.rs::plan` の判断の核。`RawKeyEvent`・`AppImeProfile` は windows 側の型なので、`ImePolicyProfile`・`ImeKindId`・キー種別・`shadow_toggled` の ungated な値で受ける純粋関数へ移す。`plan` はそれを呼ぶ薄い殻にする。`thumb_or_role_fkey_disposition` も同様。
- shadow の昇格の判断(`intent_kind` の選択、`key_pipeline.rs:949-972`)と、no-op の判定。`ShadowImeAction::resolve` は既に ungated。
- `issue_open_warrant`(`state/open_warrant.rs`、ungated 済み)。
- `decide_gate`・`decide_chain`・`decide_attempt`・`shadow_toggle_demotes_applied`(`state/ime_actuation_decision.rs`、ungated 済み)。

**状態空間**(全列挙。proptest は awase-windows の dev-deps にある〈Cargo.toml:57-60〉が、全列挙で足りる):

- belief{T,F}
- applied{Unknown, Opt T/F, Conf T/F}=5
- is_japanese{T,F}
- profile{ImmCross, Plain, Unknown, IU, TN, IR}=6
- kind{Gji, MsIme}
- current_focus{Some, None}
- 鮮度内の Actuating 観測{None, T, F}
- IntentStore{None, T, F}
- candidate_was_seen{T,F}
- chord{T,F}
- Win 押下{T,F}
- キー種別 12

合計 約 2·5·2·6·2·2·3·3·2·2·2·12 ≈ 83万通り。1ケース μs 単位なので、Linux の `cargo test` で数秒。

**性質**:

1. **INV-L1**: Win 押下と InputRelay の例外を除く全ケースで、`(physical==Allow) XOR write.is_some()`。
2. **絶対キーの1回収束**: 実 IME の初期値 R∈{T,F} も列挙する。IME モデル: Allow なら絶対キーは target、トグルは ¬R。write(o) なら o。1押下後に R'=キーの意味。
3. **トグルの2回収束**: 1押下後の状態遷移モデルを適用する。belief=書いた向き。applied=完了後の値(`record_ime_apply_result` の純粋部分。`ime_model.rs::completion_can_update_applied` は ungated)。その後の2押下目で、R''==キーの意味、または R'==キーの意味。
4. **不動点が無い**: 全状態で「押下 → 遷移」を最大3回回し、同じ ✗ を2回繰り返さない。
5. **BUG-113**: 同じ押下 id で shadow 書き込み + Engine の SetOpen が来る組み合わせで、送信は1回。

**期待**: D1-D4 の前にこのテストを入れると、§2 の L-1/L-3/L-4/L-6/L-7 が具体的な反例として出る。第1段では、現状の穴を「既知の反例リスト」として `#[should_panic]` ではなく、反例の集合を golden ファイル(`tests/golden/explicit_press_counterexamples.txt`)に固定する。D の各段でこのファイルから行が減ることを確認する。

### (b) CI の drift × キー行列(実打鍵)

**ずれの作り方**(既存の `typing_stress` を拡張):

- 実 IME を直接閉じる・開く: `--mode=drift-on` は `WM_IME_CONTROL(IMC_SETOPENSTATUS)`。`cal-driftrec-*`(`e2e-ime.yml:469-476`)にある。OFF 方向も足す。
- 古い applied を作る: awase に ON を書かせた(Ctrl+変換)後、ハーネスが閉じる。S-1 そのもの。
- フォーカス変更を挟む: `--refocus`、既存。
- `is_japanese_ime=false`(S-2): 作るのが難しい。推測: キーボードレイアウトを英語へ一瞬切り替えて戻す(`ActivateKeyboardLayout`)。CI では「観測のみ」にする。

**キー**: §1 の表の行。0x16/0x1A・0xF3・0x19・F2・Ctrl+変換・Ctrl+無変換・F13(`keys.ime_on`)・単独タップ無変換/変換。

物理の Ctrl は `TEST_INJECTION_MARKER`(`hook.rs:1298/1348`、debug + `AWASE_TEST_INJECTION=1`)で物理扱いにする。

**窓**: 自前 EDIT(IC)、RichEdit(ADR-193)/tsf 窓(TN 相当)、実 Chrome(IU)。**IME**: GJI(atok/msime プリセット)、MS-IME。

**判定**: 押下後 +0.5s/+2s に、かな(または英字)を実打鍵する。確定テキストで、絶対キーは1回目、トグルは2回目までにキーの意味と一致すること。

`check_drift_recovery.py` の分類(recovered/typed_blind/…)を流用した `check_explicit_liveness.py` を新設する。Unicode 注入の窓は `typed_blind` として除外するという規則も踏襲する。

**再利用**:

- `sc-driftrecovery-*`/`cal-driftrec-*`: ずれの作り方と実打鍵の判定。
- `sc-kanji-*`: 0x19/0xF3 のキー送出。
- `sc-solotap-*`(`e2e-ime.yml:260-277`): 単独タップ。`sc-solotap-stale-toggle` は S-1 に近い。

**追加**: 「`applied` を古くする手順(awase 書き込み → 外部で反転)× Engine のコンボ」の構成(S-1 の直接再現)と、TN×GJI の OFF 系で `@` を数える列(BUG-124、`check_typing_stress.py` の `@` 検出を流用)。

---

## 6. 段階案とリスク

| 段 | 内容 | 検証 | revert 条件 |
|---|---|---|---|
| L0 | `explicit_press_delivery` の切り出し(挙動不変のリファクタ。`plan` は新関数を呼ぶ殻に)+ §5(a) の全列挙テスト + 反例 golden(現状の穴の可視化) | Linux。golden に L-1/L-3/L-4/L-6/L-7 の反例が出ること | 挙動差(既存の golden/architecture_guard の失敗) |
| L1 | D2+D3(warrant を押下由来にする、0x19 の昇格)。S-2/S-4 を消す。BUG-124 に触れない | 反例 golden から S-2/S-4 が消える。CI の既存 `sc-*` | 非日本語 IME 窓で、awase の `VK_IME_ON` が副作用を起こす(実機 A/B で) |
| L2 | 押下 id(hook → `RawKeyEvent` → `ActuationOrder`)+ `decide_attempt` 前の同一押下の省略。**まだ applied の省略は残す**(BUG-113 の守りを先に入れる) | Linux: 性質5。CI: `@` の件数が develop と同じ | 同一押下の二重送信 |
| L3 | D1 を IU のみ(TN を除く)に適用。S-1 の IU 部分を消す | 反例 golden。CI: 新設の S-1 再現構成(IU × GJI)が1押下で一致 | IU × GJI で入力の欠落・`@` |
| L3' | D1 を TN へ広げる。**WT×GJI×PSReadLine の実機 A/B(n≥30)をマージ条件**。だめなら「同じ OFF キー2連打時だけ送る」へ | 実機 A/B + CI の TN×GJI の `@` 列 | `@` の発生率が develop より有意に増える |
| L4 | D4(IC の no-op で、Suppress なら書く)。S-3 を消す | 反例 golden が空。CI: IC × 読みが嘘の構成は作れないので、Linux の性質だけで固定 | IC で二重 actuation(Allow と書き込みの両方) |
| L5 | CI の drift × キー行列を常設(`expect=pass`、まず observe で2週) | 全行が1回(絶対)/2回(トグル)で一致 | — |
| (決定) | InputRelay(§4.4)は所有者の決定の後に別 PR | — | — |

**ADR-208 の昇格で書くべき決定**: §4.1 の仕様(INV-L1/L2)、§2 の穴の一覧(既知の例外の明記)、D1-D4、押下 id による BUG-113 の守り、L3' の A/B の条件、全列挙テストを「固着ゼロ」の機械的な定義にすること。

**BUG-124 の実機 A/B のタイミング**: L3' の直前(L3 で IU の効果を確かめてから)。実機の構成が要るので、L0-L2 と並行して所有者に依頼する。

**v2 のブロッカーにするか**:

- 所有者の方針で保証に格上げされたので、**L0-L3(IU と、全窓の is_japanese・warrant の穴)はブロッカー**にすることを推奨。
- L3'(TN の BUG-124 依存)と L4(IC の条件付き)は、A/B の結果次第で「既知の制限」に戻せる余地を残す。
- `docs/tasks/v2-release-checklist-2026-09-29.md:29,59` の「ブロッカーにしない」の行は、決定の日付を付けて書き換える。

**リスク**:

1. 押下 id を `RawKeyEvent` に足す変更は、hook → drain replay → journal の構造に及ぶ(ADR-129 の capture-time snapshot と同じ扱いにする)。
2. D2 で英語 IME 窓に `VK_IME_ON` が飛ぶ。
3. D4 の no-op の書き込みは、IC で ImmCross の async を増やす(`OutputActiveGuard` による打鍵の退避が増える)。
4. 全列挙テストの IME モデルが単純化しすぎると、偽の安心を生む。CI 行列(L5)を必須にするのはこのため。
