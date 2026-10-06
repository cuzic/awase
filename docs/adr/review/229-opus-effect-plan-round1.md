---
id: ADR-229-companion-229-opus-effect-plan-round1
title: |-
  Effect 列(Plan)の設計案 opus-adversarial-consult round1(結論: 採らない。E0・E1・BUG-098 のみ)
type: companion-doc
related_adr:
  - "ADR-229"
  - "ADR-212"
  - "ADR-180"
---

# 「Event とワールドモデルから Effect 列を生成する」設計案 r1 の敵対的レビュー(round1)

- 実測の基準: `origin/develop`(#500 マージ後)。読み取りのみ。
- 対象: `effect-plan-design-draft-r1.md`。

## 0. 結論(先に)

- **調査の整理(§1)は質が高い。ただし、主張の 5 本柱のうち 3 本が、今のコードの事実と食い違っている**。このまま「根本的な設計変更」に進むと、既存の仕組みと二重になるか、過去に BUG を出した方向へ戻る。
  1. **「Observation の Event は Write を生成できない」は、今のコードで既に破れていて、それは意図されたものである**。drift correction は観測(`observed ≠ desired`)を引き金に IME へ書く(`ir_apply_drift_correction` → `apply_ime_open_with_view`)。ADR-212 は P6(a) で、これを「許可」として**意図的に残した**(ADR-212 の status)。つまりこれは、**level-triggered の書き込みそのもの**で、柱 3 と柱 4 の両方の反例になる。(B1)
  2. **「書き込みの吸収」という法則と、正規化を 1 か所に集める案は、BUG の多い方向**。BUG-141(`gji_direct_already_matches` が 2・3 回目の Ctrl+無変換 を無送信で握りつぶした)と ADR-208(古い `applied` を根拠に、絶対指定のキーを省略してはならない)は、どちらも「書き込みを省略・合体したこと」が原因の実害。省略の規則を汎用の正規化に一般化すると、この再発ファミリーを広げる。(M1)
  3. **「monad の `>>=` を持たず、Selective で足りる」は、半分だけ正しい**。どの機構をどの順で試すか(枝の集合)は静的で、Selective で表せる。しかし**効果の中身(どの VK を送るか、ROMAN を補うか、ローマ字モードを先に書くか)は、await の後に読み直した値から決まる**(`fallback_write` が機構ごとに view を作り直して `decide_attempt` する)。必要なのは「任意のクロージャ」ではないが、「名前のついた純粋な決定関数への継続(defunctionalized bind)」は要る。(M2)
- **さらに、Plan を新しい汎用の型として作ると、`Actuation<Verified>::run_chain(_async)` と二重の表現になる**。これは型状態(ADR-090)と再生(#499)を既に支えていて、ADR-229 F-D1 が例外として明文化したもの。toolkit round1 が「既にある(`Response`)ので作らない」とした候補 2 と同じ判定になる。(M3)
- **判定**: E0(Effect の署名の棚卸し、読み取りのみ)は今すぐやってよい。**E1 は「新しい Trace 解釈器」としてではなく、「決定関数が省略の理由を enum で返し、journal に載せる」の個別の改善として、実害が出た決定から 1 つずつ**行う。E2〜E6 は、今は着手しない(待つ条件は §4)。
- 加えて、案の中で**本当に実害に効く部分は、Plan とは独立に今すぐできる**: 未修正の BUG-098(世代の無い非同期の shadow toggle OFF の完了が、focus epoch の gate を通らない)を、F-D5-2(既存の世代で由来を示す)で直すこと。
- 所有者の「根本的な設計変更を前提に」への答えとしては、**「根本的に変えるべきなのは Effect 列の表現ではなく、判断の入口(F の分割)である。Effect 列の表現は、F の分割が 3 本出て、同じ形の Cmd 列が実際に現れてから決める」**が、私の推奨。「E1 の個別の改善だけで終わるのが最善」という結論は**ありうる**。

---

## 1. Q1〜Q11 への回答

### Q1: 連鎖は Selective で足りるか

**成り立つ箇所**

- **試す機構の集合と順序**: 同期は `caps(p, k).chain`、非同期は `WriteMechanism::ALL` の固定(`open_chain.rs:35-59`、`:692`)。集合は有限で、静的に列挙できる。
- **打ち切りの規則**: `falls_through` / `Actuation::classify` の 1 か所(`state/actuation_chain.rs:212`、`run_chain` は `:549`)。結果が `Failed` なら次、それ以外なら終わり。Selective の「条件付きの効果。両方の枝が見える」に当てはまる。
- **focus の段階的な分類**: 同期の分類 → MSAA →(UIA は結果を使わない。BUG-12)。段階の集合は静的。

**成り立たない箇所(枝の集合ではなく、効果の中身が実行時の値で決まる)**

1. **`fallback_write`(`open_chain.rs:459-555`)**: await の後に `shadow_ime_control_view()` で view を作り直し、`shadow_on = None` で上書きし(BUG-113)、`mechanism_is_applicable(mechanism, &view)` と `decide_attempt(inputs, …)` で、**送るかどうかと送る命令(`MechanismCommand`)を、その時点の観測から決める**。plan の時点ではまだ値が無い。
2. **`set_ime_open_then_conv_for_target`(`open_chain.rs:300-313` から呼ぶ)**: open を書いた結果が `Written` なら、フォーカスの世代を読み直し(`with_app(|r| r.platform.output.ime_mode_focus_gen.get())`)、そのうえで ROMAN 補完の conv を書く。2 つ目の効果の有無が、1 つ目の結果と読み直した値で決まる。
3. **`romaji_pre_write`(`ime_controller.rs::apply_mechanism` の先頭)**: 実行時のグローバル `SendHealth::blocking_allowed`(サーキットブレーカ)と、`belief_input_mode` を見て、ブロックしうる書き込みを行うかを決める。
4. **長いプロトコル**: warmup の per-VK confirm(`probe_fsm.rs:470-503`。VK の数だけ、結果を見て次へ)、drift correction の再送(`FeedbackPolicy`)。これらは案のとおり Process 側。

**結論**: 「任意のクロージャは不要」は正しい。だが「Selective で足りる」は誤りで、**閉じた決定関数の集合への継続**(`Step::Then { probe: ProbeReq, decide: DecideId }`。`DecideId` は列挙で、中身は `decide_attempt` などの既存の純関数)が要る。これは今の `AsyncChainWriter::write`(handler)が実質的にしていること。

### Q2: `imm_cross_write` の `post_failed_reobservation` は `Try { reobserve }` で表せるか

- **表せる**。`open_chain.rs` の `ActuationOutcome::Failed` の腕は、次のことしかしていない。
  1. `read_ime_state_fast().ime_on` を読む。
  2. 純関数 `imm_cross_reobservation_already_matches(actual, open)`(`state/ime_actuation_decision.rs`)で、`AlreadyMatched`(終わり)か `Failed`(次へ)かを決める。
- 判断は既に純粋で、枝は 2 つで静的。`reobserve: Some(ReadOpen)` と `stop_on: ReobservedMatches` で表せ、executor は太らない。
- ただし、**表せない部分が同じ関数の中にある**。
  - `ImmCrossOp::Targeted` の検証済みの宛先(INV-14)
  - `Aborted(reason)` → `UnsafeToToggle`
  - タイムアウトの扱い(`open_timed_out`)
  - Q1-2 の conv の連鎖

  `Try` は `imm_cross_write` の「Failed の後」だけを表せ、前半は handler(F-D1 の例外)に残る。再読み取りだけを Plan に出しても、handler が無くなるわけではない。

### Q3: gate を plan 時の静的解析に、失効を `Require(epoch)` に分けられるか

- **ADR-180 の理由は満たさず、意味が変わる**。3 つの関数が独立に gate を再検出するのは、await の間に**フォーカスが別の窓に移り、その窓が InputRelay かもしれない**から(ADR-180 決定1、`open_chain.rs:578-591` のコメント)。
- plan の時点で gate を 1 回だけ見るなら、await の後のフォーカスの変化は `Require(epoch)` に任せることになる。だが今の挙動は「**世代が変わっても止めず、新しい view で gate を判定し直し、InputRelay でなければ新しいフォーカスに書く**」(`fallback_write` が `is_input_relay(inputs)` を毎回判定。ADR-090 項 D の「チェーンの再抽選」の未実装部分)。
- `Require(epoch)` で止めると、**「フォーカスが変わったら書かない」という別の挙動**になる。それ自体は INV-14(検証済みの宛先)の精神には合う。しかし BUG の記録に照らした実機の確認が要る挙動変更で、Plan の導入の副産物として入れてはならない。
- **B-1 の fail-open**: `with_app` が `None`(再入)のとき、gate は fail-open で書き込みを続ける(`open_chain.rs:584-597`)。`Require` が世界を読めないときにどうするか(fail-open か fail-closed か)を、案は定めていない。
- **結論**: gate の判定(`decide_gate`)は既に純粋で、1 か所にある(ADR-180 決定1)。複製されているのは「view を作る殻」の部分で、それは await の後に作り直す必要があるから(F-D5-4)。plan の時点の 1 回にまとめられるものは、もう残っていない。

### Q4: `Bracket` で `OutputActiveGuard` と defer/drain の 2 窓口を対にできるか

- **前提が違う**。`OutputActiveGuard` は RAII(`tsf/probe_bridge.rs:101-125`、`Drop` で release)なので、**「acquire に対する release の漏れ」は既に型で防がれている**。
- ADR-156(ADR-123 → 128 の回帰)の事故は release の漏れではない。**同じキューを読む defer 側と drain 側の 2 つの窓口の片方にだけ、新しい解放の条件を配線した**(ADR-156 の summary: 「正味は `pending_deferred` 内の 2 窓口間 1 件」)。`Bracket` の構造はこの型の事故を防がない(窓口は 2 つのまま)。
- さらに、ガードの寿命は 1 つの Plan の中に収まらない。production の取得は 12 か所: `output/mod.rs` 4(`:1553`〜`:1641`)、`sender.rs:74`、`tsf_warmup_coord.rs:159`(`RefCell<Option<OutputActiveGuard>>` に保持し、複数の turn にまたがる)、`vk_send.rs:296, 811`、`executor.rs:587, 805`、`key_pipeline.rs:1311`、`runtime/mod.rs:2454`、`gji_warmup_coro.rs:325`(コルーチンの状態に保持)。ほかに、`chrome_probe.rs` の型がガードを所有する(move で受け取る)。
- body を持つ `Bracket { acquire, body, release }` は、Process の状態やタスクの寿命にぶら下がるガードを表せない。表そうとすると、acquire と release を別の turn の Plan に分ける必要があり、構造で対にするという利点が消える。
- **結論**: `Bracket` は、ADR-156 の実害にも、今のガードの寿命にも合わない。ADR-156 の型の事故への対策は、既存の「2 窓口を必ず対で見直す」規約(fix-requires-evidence.md の表)と、窓口が 2 つあることを示すガードのほうが的確。

### Q5: Event の分類と capability の網羅テスト

- **数字の誤り**: `ImeEvent` の variant は **20**(約 65 ではない)。`UserImeToggleIntent`・`UserImeSetIntent`・`PanicReset`・`HwndCacheRestored`・`ImeApplyRequested`・`ImeApplySucceeded`・`ImeApplyFailed`・`ObserverReported`・`FocusChanged`・`FocusHwndUpdated`・`InitialFocusFenceEstablished`・`InitialAppPolicyEstablished`・`ModeKeyPassedThrough`・`KeyEffectPredicted`・`InitialFocusHwndEstablished`・`ChordEnded`・`DriftDetected`・`InputModeObserved`・`InputModeApplied`・`UserChangedInputMode`。
- **分類の対象を取り違えている**: `ImeEvent` は**信念(belief)の reducer への入力**で、Plan を生む引き金ではない。`ImeApplyRequested`/`Succeeded`/`Failed` は書き込みの**結果の記録**。Plan を生むのは runtime の入口で、`RawKeyEvent`(key_pipeline)、`WM_TIMER`(ime_refresh・warmup)、WinEvent のフォーカス変更、`WM_ASYNC_IME_APPLY_COMPLETE` などの完了メッセージ。capability の網羅は、`ImeEvent` ではなく**これらの入口 × 呼び出してよい actuation の関門**で表すべきもの。
- そしてそれは**既にある**。`lints/actuation_call_guard`(`RESTRICTED_CALLS`。`apply_ime_open_with_view` の呼び出し元は `dispatch_ime_set_open` と `ir_apply_drift_correction` の 2 つだけ)と、ADR-208 の押下 ID の 3 点の配線のガード(`press_id_is_claimed_and_carried_at_every_order_issuing_entry`)が、「どの入口が書いてよいか」を固定している。E5 は、これらの重複になる。
- **境界の曖昧な variant**: `DriftDetected`(観測だが、drift correction の書き込みを引き起こす)、`KeyEffectPredicted`(物理キーという意図に由来する予測で、`input_mode` を書く)、`ModeKeyPassedThrough`(観測の結果を受けて `desired_open` を揃える)。どれも「Observation か Intent か」で割り切れない。

### Q6: 「level-triggered は情報の収集だけ」の線引き

- **整合しない**(B1)。drift correction(P6(a)、意図的に残した「許可」)は、`check_drift_correction` が観測と `desired_open` の差を見て書き込みを決める、典型的な level-triggered の書き込み。
- 整理し直すと、次のようになる。
  - **収集側**: 観測の要求(idle-conv-check、focus probe、GJI モニター)、`observe_miss_monitor`(観測の失敗を数える。書かない)。
  - **書き込み側の level-triggered**: drift correction(有界の再送。ADR-080 の `FeedbackPolicy`、BUG-43 の無限再送の対策)。
  - **Process**: warmup の再試行(edge の後の有限の手順)。
- ADR-212 が撤去したのは、「ユーザー操作を引き金にしない、**予防的・補正的**な書き込み」。drift correction は、その中で唯一、所有者が許可として残したもの。線は「level か edge か」ではなく、**「許可された補正(drift correction、有界の再送つき)だけが、観測から書いてよい」**と引くのが事実に合う。

### Q7: 効果の法則

| 法則 | 判定 | 理由 |
|---|---|---|
| タイマーの上書き(`Set(id,d1);Set(id,d2)=Set(id,d2)`、`Kill;Kill=Kill`) | **成り立つ** | Win32 の `SetTimer` は同じ id で再設定すると置き換わる。`timed_fsm::TimerRuntime` の doc の不変条件とも一致。proptest にする価値はある(小さく、実害の記録は無い) |
| 書き込みの吸収(`SetOpen(x);SetOpen(y)=SetOpen(y)`、間に Probe が無ければ) | **成り立たない/危険** | (i) 各書き込みには観測できる副作用がある。`ImmSetOpenStatus` の通知、`applied` の更新、journal の `ActuationDecision`、`PressLedger` の予約(ADR-208 L1。同じ押下の二重送信の防止は意図的)。(ii) 間に Output(打鍵の送信)があれば順序が意味を持つ。「Probe が無ければ」では足りない。(iii) **省略・合体は BUG-141・ADR-208 の原因そのもの**(古い `applied` を根拠にした `already_matches` の省略が押下を握りつぶした)。(M1) |
| 独立(Ui・Log は何とも可換) | おおむね成り立つ | ただし Log(journal)の順序は、不具合報告の時系列の再構成に使う(`seq`)。「可換」にすると、記録の順序の意味が消える。Log は順序を保つ前提で |
| bracket の対 | 既に RAII | Q4 |
| 正規化の健全性 | 対象が無い | 吸収を採らなければ、正規化するものはタイマーだけ |

### Q8: 信念を「証拠の view」にし、観測を join-semilattice にする案

- **部分的には既にそうなっている**。`effective_open_at(now)` は、観測(`ObservationStore`)・`IntentStore`(TTL つき)・`desired_open` からの導出。観測の古さの拒否は、`probe_admission`(FocusFence/epoch、ADR-104)が既に担っている。
- **semilattice にならない部分**
  - `FocusChanged` の reduce で drift の追跡を消す(warmup_gate_focus_scope.rs が固定)。単調でないリセット。
  - `desired_open` を直接書く 4 つの例外イベント(PanicReset・HwndCacheRestored・ModeKeyPassedThrough の揃え・KeyEffectPredicted は `input_mode`)。
  - `last_intent` の破棄。
  - `IntentStore` の TTL による失効。

  これらは「後から来た証拠と合わせる」ではなく、「状態を捨てる」操作。
- 「観測は source ごとに最新を残す」(LWW レジスタ)は semilattice として扱えるが、それは既にある形で、新しく得るものは無い。**「信念全体を semilattice に」は成り立たず、成り立つ部分は既にある**。

### Q9: ADR-229 と toolkit round1 との関係

- **F-D1 とは衝突する**。F-D1 は「core は Cmd を返すか、閉じた列挙の handler の例外(`run_chain(_async)`)を使う」。Plan の `Try` は `run_chain` の外側の第 2 の表現になり、同じ連鎖を 2 通りに書く(M3)。
- toolkit round1 は、候補 2(Plan/Cmd)を「既にある(`timed_fsm::Response`、ドメインの `ProbeAction`・`MechanismCommand`)。新設しない」とした。Plan を作るなら、**その判定を覆すだけの使い手(Plan を総称として解釈する解釈器が 3 つ)**が要る。今ある解釈器は、Replay(`ReplayWriter`、#499。`MechanismWriter` の実装)と Fake(擬似 IME。別の抽象)の 2 つで、Trace と Static は 0、Real は handler。**共通の Plan を解釈しているものは 0**。

### Q10: 過去の轍と費用

- **同じ轍の部分**: Plan の項(`Try`・`Require`・`Bracket`・`Spawn`)は、ADR-219(シナリオの DSL)と同じ性質を持つ。「既存の型を 1 つの項に束ねる、小さな言語」になる。さらに ADR-180 決定2(3 実装の統一。統合しても機構の数は不変で、テストとガードが増えて費用対効果が負)と同じ構造を持つ。Plan を足しても、Windows の機構(ImmCross・GjiDirect・MsImeDirect)の数と、await の後の作り直しは減らない。
- **実害の記録との対応**: 案が挙げる 6 件(ADR-180、ADR-156、BUG-34、BUG-98、BUG-141、ADR-212)のうち、**Plan が直接効くのは BUG-098 だけ**で、それも `Require` ではなく「完了に世代を持たせる」(F-D5-2)で直る。
  - ADR-180 は Q3、ADR-156 は Q4、BUG-141 は Q7 で逆方向、ADR-212 は Q6 で反例。
  - BUG-34 は、本来は `SendMessageTimeoutW` の同期ブロック(BUG-034 の題)で、観測と実行の窓の話は「BUG-34 型」と呼ばれた別の派生。Plan の表現は、どちらも変えない。
- **E1 だけで終わるのが最善か**: ありうる(§4)。

### Q11: 欠けている論点と代案

1. **代案(推奨)**: Plan を作らず、**現状の延長**。
   - (a) F の分割で `decide → Cmd` を増やす。
   - (b) 連鎖は handler の例外(`run_chain`)のまま。
   - (c) 再生は `awase-replay`(toolkit)+ ReplayWriter。
   - (d) 「なぜ省略したか」は、各決定関数の戻り値の理由の enum(既に `GateResult`、`suppress_reason`、`Delivery`、`FeedbackPolicy` の action がある)を journal に載せる。

   見通し・replayability・複雑性の改善は、Plan の導入と同等で、二重の表現を作らない。
2. **欠けている論点: Effect 列の生成の主体は誰か**。今は key_pipeline・ime_refresh・executor・focus_tracking・warmup が別々に Cmd を出し、順序は呼び出しの順。案の §2.6(スライスごとの planner を `Cmd.batch` で合成し、Stage の全順序で並べる)は、**今の暗黙の順序(例: baseline を `SendInput` の前に取る、BUG-027/029/030/033。`kp_latch_keyup_to_keydown_disposition` を `plan()` の直後・journal の記録の前に呼ぶ、bug173 のガード)を壊す危険**が最も高い部分。Stage の 6 段階で足りるかを先に確かめると案は書いているが、反例は既にガードとして存在する。
3. **欠けている論点: フックスレッド**。Plan と turn はエンジンスレッドだけで、フックの通す/消すの同期判定は Plan に載らない(ADR-229 F-D5 と同じ)。ADR-129 のスナップショットの埋め込みも同様。
4. **欠けている論点: 失敗の意味論**。`Try` の各段の `Aborted`/`UnsafeToToggle`/`NotOwned`/`Unwarranted`(`ImeOpenOutcome` の variant)を、Plan のどこで扱うか。今は `classify` が担っている。

---

## 2. 前提の誤り・重複・見落とし(指摘の一覧)

| ID | 重大度 | 指摘 | 直し方・代案 |
|---|---|---|---|
| B1 | **Blocker** | 柱 3(Observation は Write を生成しない)と柱 4(level-triggered は収集だけ)は、drift correction(ADR-212 P6(a) で意図的に許可として残した、観測を引き金とする有界の補正の書き込み)で既に破れている | 柱を「**許可された補正(drift correction、`FeedbackPolicy` で有界)だけが観測から書いてよい。その他の書き込みは明示の意図(物理キー・コマンド)から**」に書き直す。これは既に `RESTRICTED_CALLS`(`apply_ime_open_with_view` の呼び出し元 = `dispatch_ime_set_open`・`ir_apply_drift_correction`)で固定されているので、E5 は新設せず、既存のガードの意味を ADR に書くだけでよい |
| M1 | **Must** | 「書き込みの吸収」の法則と、正規化を 1 か所に集める案は、BUG-141・ADR-208(省略が押下を握りつぶした)の再発ファミリーを広げる | 書き込みの法則は採らない。正規化はタイマーだけに限る。省略は、今の個別の判定(`already_matches`。陽性の確認済みの証拠に限る、fix-requires-evidence.md の `shadow_on` の行)のままにする |
| M2 | **Must** | 「`>>=` を持たず Selective で足りる」は、効果の中身が await の後の観測で決まる箇所(`fallback_write`、conv の補完、`romaji_pre_write`)で成り立たない | 「任意のクロージャは持たないが、閉じた決定関数の集合への継続(`Then { probe, decide: DecideId }`)は要る」と書き直す。そのうえで、それは今の handler(`AsyncChainWriter::write` が `decide_attempt` を呼ぶ形)と同じだと認め、Plan で置き換える利点を再評価する |
| M3 | **Must** | Plan の `Try` は `Actuation<Verified>::run_chain(_async)` と二重の表現になる(ADR-229 F-D1・toolkit round1 の判定と衝突)。共通の Plan を解釈する解釈器は 0 | Plan を新設しない(Q11-1 の代案)。新設するなら、「同じ Plan を解釈する解釈器が 3 つ(Real・Replay・Trace)」が実際に揃い、`run_chain` を Plan の解釈器の 1 つとして**置き換える**(並存させない)ことを条件にする |
| M4 | **Must** | `Bracket` の根拠(ADR-156 の「片側だけ配線」を防ぐ)が事実と違う。release の漏れは RAII で既に無く、ADR-156 の事故は 2 窓口の条件の配線漏れ。ガードは複数の turn・Process・move にまたがる(12 か所) | `Bracket` を案から外す。ADR-156 の対策は既存の規約のまま |
| M5 | **Must** | gate を plan の時点の 1 回にし、失効を `Require(epoch)` にする案は、「世代が変わっても新しい view で判定し直して書く」今の挙動を「止める」に変える(ADR-090 項 D)。B-1 の fail-open の扱いも未定義 | 挙動変更として別の ADR にし、実機の確認とセットにする。Plan の導入と切り離す |
| S1 | Should | `ImeEvent` の variant の数(約 65 → 実際は 20)と、capability の分類の対象(`ImeEvent` は reducer の入力で、Plan の引き金ではない) | 分類の対象を runtime の入口(`RawKeyEvent`・`WM_TIMER`・WinEvent・完了メッセージ)にし、境界の曖昧な variant(`DriftDetected`・`KeyEffectPredicted`・`ModeKeyPassedThrough`)を明記する |
| S2 | Should | 信念全体の join-semilattice 化は成り立たない(フォーカスでのリセット、例外イベント、TTL)。成り立つ部分(source ごとの最新、epoch による古さの拒否)は既にある | §2.4 を「既存の形の説明」に縮める |
| S3 | Should | §2.6 の Stage の全順序は、既存の順序の制約(baseline を `SendInput` の前、KeyUp のラッチの位置。ガードあり)を壊す危険が最も高い | 合成の順序を変えない前提を明記する。Stage は導入しない(F の分割では、呼び出し元の順序をそのまま保つ) |
| S4 | Should | BUG-034 の要約(「観測と実行の間の窓」)が不正確。BUG-034 の題は、`SendMessageTimeoutW` の同期ブロック | 「BUG-34 型(view1/view2 の窓、`open_chain.rs:426-436`)」と「BUG-034 本体」を書き分ける |
| N1 | Nit | §1 の「自然変換としての解釈器(再生 ∘ 記録 = 恒等)」は、ReplayWriter(#499)が実際に示した性質と同じ | 既存の実例として #499 を挙げる |

---

## 3. 代案の評価(過去の轍との比較つき)

| 代案 | 実害の記録 | 見通し・replayability・複雑性 | 過去の轍 |
|---|---|---|---|
| **A(推奨): 現状の延長**。F の分割で `decide → Cmd`、連鎖は handler のまま、再生は `awase-replay` + ReplayWriter、決定関数の理由の enum を journal に | BUG-098(世代)を F-D5-2 で直す。「なぜ省略したか」が分からず調査が長引いた件(memory の BUG-141 の調査の経緯)は、理由の enum で効く | 二重の表現を作らない。決定点ごとに再生できる範囲が広がる | 新しい言語も、汎用の型も作らない。ADR-218〜220・ADR-180 決定2・toolkit round1 と整合 |
| B: 案 r1 の Plan(`Try`・`Require`・`Bracket`・`Spawn`) | 直接効くのは BUG-098 だけ(それも A で直る) | 解釈器を差し替えられる利点はある。ただし `run_chain` と二重になり、Stage の順序の再構成で既存の順序を壊す危険がある | ADR-219(小さな言語)と ADR-180 決定2(統合しても機構の数は不変)と同型 |
| C: Plan を「記録の形式」としてだけ使う(実行は今のまま。決定の結果を Plan 風の構造で journal に残す) | 調査のしやすさ(BUG-141 の経緯) | replayability は上がる。実行経路は不変 | `ActuationDecisionRecord`(ADR-163)が既にこれ。拡張で足りる |

---

## 4. 段階 E0〜E6 の判定表

| 段階 | 実害の記録 | rule of three | 判定 | 待つ条件(着手しない場合) |
|---|---|---|---|---|
| E0 Effect の署名の棚卸し | (読み取り) | — | **今すぐ可**(W0 と並行、コード変更なし)。各 variant の順序の制約・冪等性・副作用を一覧にし、M1 の「吸収が危険な理由」の根拠表にする | — |
| E1 Trace 解釈器 | 調査の長期化(BUG-141 の経緯)。ただし「新しい解釈器が要る」という記録は無い | 理由を返す決定関数は既に 4 つ以上(`decide_gate`、`plan_core` + `suppress_reason`、`explicit_press_delivery`、`FeedbackPolicy`) | **形を変えて可**: 新しい解釈器としてではなく、「journal に、決定関数の理由の enum を載せる」を、実際の調査で理由が足りなかった決定から 1 つずつ | 解釈器として作るのは、Plan(B)を採ったときだけ |
| E2 `Try` と plan の時点の静的解析 | ADR-180 は、gate の判定が既に 1 か所(Q3) | 使い手 4 と書かれているが、実体は handler 1 種 | **着手しない**(M2・M3・M5) | F の分割が 3 本進み、`run_chain` 以外にも「有限の機構を順に試す」連鎖が 2 つ以上現れたら |
| E3 正規化と法則 | タイマー: 実害の記録なし。書き込み: 逆方向(BUG-141) | タイマー: 多数 | **タイマーだけなら任意**(proptest 1 本。優先度は低い)。**書き込みは採らない** | — |
| E4 `Bracket` | ADR-156 の事故とは型が違う(Q4) | 12 か所 | **着手しない**(M4) | release の漏れが BUG として記録されたら(今は RAII で起きない) |
| E5 capability の網羅テスト | ADR-208 は既に固定済み | `RESTRICTED_CALLS` と押下 ID のガードが既にある | **着手しない**(重複)。B1 の書き直しを ADR に書くだけ | 既存のガードで捕まらない「観測からの書き込み」の新しい入口が BUG として記録されたら |
| E6 観測の購読(`needs`) | 記録なし | 観測の切り替えの箇所は数えていない(**未確認**) | **着手しない**(後段) | 観測のソースの起動・停止の漏れが BUG として記録されたら |

**結論**: E0 は今すぐやる。E1 は「理由の enum を journal へ」の個別の改善として、必要が出た決定から行う。それ以外は待つ。**「E0 と E1(個別の改善)で終わり、Effect 列の表現は F の分割の結果を見て決め直す」が、今の証拠で最善**。並行して、BUG-098 を F-D5-2(完了に既存の世代を持たせる)で直すタスクを独立に起こす。これは Plan の有無に関わらず効く唯一の実害対応。

---

## 5. 次のラウンドで直すべき点

1. B1: 柱 3・4 を「許可された補正(drift correction)だけが観測から書く」に書き直し、既存の `RESTRICTED_CALLS` との対応を示す。
2. M1: 書き込みの吸収の法則を外す。正規化の範囲をタイマーに限る。
3. M2: 「Selective で足りる」を、「閉じた決定関数への継続が要る」に直す。そのうえで、今の handler との差(何が良くなるか)を、見通し・replayability・複雑性で具体的に示す。示せなければ代案 A を採る。
4. M3: Plan と `run_chain` の関係(置き換えるのか、並存するのか)を決める。並存なら Plan を採らない。
5. M4: `Bracket` を外すか、ADR-156 の実際の事故の型(2 窓口の条件の配線漏れ)に効く別の形を示す。
6. M5: gate・失効の分離は、別の ADR の挙動変更として切り出す(ADR-090 項 D、B-1 の fail-open)。
7. S1〜S4 の訂正(`ImeEvent` の 20 variant、分類の対象、semilattice、Stage、BUG-034)。
8. 所有者への説明として、「根本的に変えるべきは判断の入口(F の分割)で、Effect 列の表現は F の分割の結果を見て決める」を代案 A として並べ、E0・E1・BUG-098 の 3 つを次の具体的なタスクとして提示する。
