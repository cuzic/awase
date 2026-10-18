# ADR-265 段階 2b 設計(adr265-belief-connection-design.md)への敵対的レビュー

- 対象: `origin/docs/adr265-belief-design`(84dce2f0)の `docs/tasks/adr265-belief-connection-design.md` §0〜§9
- 読んだコード: `/home/cuzic/awase-adr265-stage1` の作業ツリー(84dce2f0)。下に挙げたファイルは `origin/develop`(37edc4bf)と差分なしを確認済み(`git diff --stat HEAD origin/develop -- crates` の差分は ime_kind.rs / tip_detector.rs など 4 ファイルだけ)。
- 行番号はその時点のもの。ビルド・実行はしていない(読んで判定しただけ)。
- 所有者の決定(High で入れる・自動判定・applied の新しい入口)は覆さない。指摘するのは、その決定から出てくる穴と、決定を満たしたまま安全にする代案。

---

## 結論(先に)

1. **High にしても、BUG-200 の場面で belief は直らない。その一方で drift correction は共有メモリを根拠に IME へ書き始める。** 一番危ない組み合わせ(C1+C2)。`resolve_open_at` は明示意図と打鍵予測を `derive_any` より先に見るし、`IntentStore` は 10 秒(ON)/ 30 秒(OFF)のあいだ実効値を上書きする。drift correction のほうは authority も窓の照合も見ないので、`BeliefOnly` の `MozcShm` を根拠に送信する。
2. §3 の「鮮度 3 秒で切れる」は `derive_filtered` の中でしか成り立たない。belief のフォールバック(`most_recent_trusted`)は鮮度も窓の照合も見ない。`expires_at` も必ず `None` なので、古い High が無期限に残る(H2)。
3. §9 の「Confirmed にする入口」は、`Confirmed` と `Optimistic` を区別する本番コードが無い(`is_confirmed()` を呼ぶ本番コードは 0 件)。だから実際の効果は「`Unknown` を `Some(x)` に戻す」ことだけになる。打鍵や自分の書き込みの直後(約 61〜120ms)の古い読みで、BUG-156 を TsfNative 窓で生き返らせる(H1)。
4. 代案: belief の訂正も applied の扱いも、**既存の「外からの変化に追随する」経路**(`follow_external_change_in_scope` / `follow_direct_read_in_scope` と同じ 3 手: 観測を記録 → `intent_store.remove` → `pass_through_observed(tick, true, true)`)に乗せる。applied は**下げる方向だけ**(食い違ったら `Unknown`)にする。因果の判定には共有メモリの `update_tick_ms` を使う。これなら ImeEvent の新しい variant も ADR-214 の再開も要らない。

---

## C1(致命的)drift correction が共有メモリを根拠に IME へ書く。`BeliefOnly` では止まらない

**場所**: `crates/awase-windows-core/src/state/drift_correction.rs:127-190`(`evaluate_drift`)、`observation_store.rs:720-729`(`most_recent_trusted_excluding`)、`ime_model.rs:806-822`(`ObserverReported` の reduce が毎回 `update_drift`)、`drift_plan.rs:385-450`、`ime_actuation.rs:46-57`、`observation_store.rs:666-681`(`AnyFreshEvidence` の除外は 2 ソースだけ)。

**根拠**:
- `evaluate_drift` が根拠を選ぶのは `most_recent_trusted_excluding(now, &[ConvOpenInference])` だけ。**信頼度を優先して並べ、authority も見ず、窓の照合(`is_identity_ok`)もしない**。`MozcShm` が High なら、ほかの全ソースに勝って根拠になる。
- 年齢の上限は `DRIFT_CORRECTION_OBS_MAX_AGE_MS = 1_500`(`tuning.rs:258`)。§3 は毎 tick(約 500ms)報告するので、この条件は常に満たされる。
- `ObserverReported` の reduce は毎回 `update_drift(desired_open, observed.open())` を呼ぶ(`ime_model.rs:812`)。共有メモリが desired と違えば、すぐに drift の追跡が始まる。
- 発火条件は `explicit_intent == Some(desired)` だけ(`drift_correction.rs:139`)。トレイ・Engine コマンド(`write_set_open_request`、`platform_state.rs:1938-1946`)、物理キー・同期キー(`write_physical_key`/`write_sync_key`)のあとは、いつもこれが成り立つ。
- §2 は「`BeliefOnly` だから授権に使わない」と書いている。しかし `BeliefOnly` が効くのは `derive_actuating`(`open_warrant.rs:190`)だけ。drift correction(上記)と読み戻し `read_back`(`most_recent_trusted_after*`)はソースの authority を一切見ない。

**失敗シナリオ**:
1. **Standard の窓(Read 方針)**: 利用者がトレイか変換キーで IME を開く(`last_intent=Some(true)`)。そのあと Mozc のツールバーで閉じる。次の tick で `MozcShm(false, High)` → drift の追跡が始まる → `evaluate_drift` が `Ok` → `DriftStep::Send` → awase が IME を開け直す。`FeedbackPolicy::Read` は試行回数で諦めない(`ime_actuation.rs:55`)。結果、**フォーカスを移すか物理モードキーを押すまで、利用者はツールバーで IME を閉じられない**。アプリが自分で閉じる場合(パスワード欄で compartment ごと閉じるアプリ、IME を自動で OFF にする端末など)は、閉じる→開ける の争いになる。
2. **TsfNative / Imm32Unavailable の窓(Blind 方針。Chrome・WT・VS Code)**: いまは開閉の観測が無いので drift は発火しない(BUG-173 追補3 で `ConvOpenInference` を除外して「受動化」した。`drift_correction.rs:171-176` のコメント「TsfNative×GJI では最初の OFF が失われても自動では再送せずユーザーの押し直しに委ねる」)。`MozcShm` はこの受動化を黙って解除する。5 回で GiveUp しても、3 秒後(`DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS`)に `read_back(AnyFreshEvidence)` が `ExternalChange` を返す。`MozcShm` は除外リスト(`ObserverPoll`/`ConvOpenInference`)に無く、毎 tick 新しいからだ。結果は `GiveUpPark::Rearm` → また 5 回 → …… の **BUG-114 と同じ形の無限ループ**。WT × GJI 系(mozc-readable は `is_gji_family_description` で GJI 系に分類される)の OFF 方向では、単発の `VK_IME_OFF` の再送になり、**BUG-124 型の「@」が繰り返し出る**(ADR-208 L3' がまだ A/B を済ませていない領域)。
3. drift が続くあいだ `should_notify_diagnostic` が診断バルーンを出す条件にも入る(`drift_plan.rs:416`)。

**§7 の「未確認」への判定**: **実際に起きる**。コードの道筋は上のとおり全部つながっている。

**推奨対応**:
- `MozcShm` を `evaluate_drift` の除外リストに加える(`[ConvOpenInference, MozcShm]`)。`read_back` の `EXCLUDED_FROM_ANY_FRESH_EVIDENCE` にも加える。後者を忘れると、ほかのソースで始まった drift の GiveUp の後も、毎回 Rearm されてしまう。
- ただし、除外**だけ**だと C2 のとおり belief も直らず、何も効かない設計になる。C2 の対応と組にすること。
- ADR-212 P6 の「明示操作の書き込みが届かなかったときの再試行」に共有メモリを使いたいなら、別の述語で書く。`update_tick_ms` が awase の書き込み時刻より前(=書き込みが届いていない)のときだけ使い、TsfNative×GJI では今の受動化を守る。

---

## C2(致命的)High でも、明示意図・打鍵予測・IntentStore が残っているあいだは belief が直らない

**場所**: `ime_model.rs:466-487`(`resolve_open_at`)、`platform_state.rs:1144-1151`(`effective_open_at` が `intent_store.resolve_effective_open` を shadow より優先)、`tuning.rs:378`/`412`(`EXPLICIT_ON_INTENT_TTL_MS=10_000`、`EXPLICIT_OFF_INTENT_TTL_MS=30_000`)、`ime_model.rs:778-784`(`UserImeSetIntent` が必ず `last_intent` を立てる)。

**根拠**:
- `resolve_open_at` の順序は、明示意図(`last_intent`)→ 打鍵予測(`key_effect.open`)→ `derive_any` → `most_recent_trusted` → `desired_open`。**High の `MozcShm` が勝てるのは 3 番目から**。
- さらに上に `IntentStore` が乗る。物理キー・同期キー・明示コンボのあと 10/30 秒間、hwnd ごとに実効値を上書きし、**フォーカス変更でも消えない**(キーが hwnd)。
- `last_intent` を消すのは、フォーカス変更(`ime_model.rs:1010`)・開閉の打鍵予測(`:922`)・`ModeKeyPassedThrough`(`:942`)・テスト用だけ。トレイや Engine コマンドの `UserImeSetIntent` のあとは、窓を変えるまで残る。

**失敗シナリオ(BUG-200 の系統、設計の目的そのもの)**: awase か利用者が開いた(明示意図 true)→ 外から閉じられる → `MozcShm(false, High)` が毎 tick 届く。それでも `effective_open` は `ExplicitIntent`(と IntentStore)で `true` のまま → エンジンはローマ字を送り続け、閉じた IME に `ka` などがそのまま入る。そこへ C1 の drift が IME を開け直す。**「belief だけを正す、IME へは送らない」(§1・§8 推奨)の正反対**で、送信だけが起きて belief は直らない。C1 の推奨どおり drift から除外すると、送信も belief の訂正も起きず、目的の場面で何も効かない。

**推奨対応(代案、決定 1 の「事実を優先」を満たす最小の形)**:
- 共有メモリの読みが**awase 以外の原因で変わった**とき(因果の判定は下記)は、既存の「外からの変化に追随する」経路と同じ 3 手を実行する。`report_mozc_shm_open`(High)→ `intent_store.remove(hwnd)` → `pass_through_observed(tick, true, true)`。手本は `platform_state.rs:487-516`(`follow_external_change_in_scope`)と `:625-629`(`follow_direct_read_in_scope` の開閉の分岐)。
  - `ModeKeyPassedThrough { align_desired: true, demote_applied: true }` が `last_intent` を消し、`desired_open` を `derive_any`(=直前に記録した High の `MozcShm`)に揃え、食い違う `applied` を `Unknown` に落とす(`ime_model.rs:936-966`)。desired が事実に揃うので、**drift は発火する理由を失う**(C1 の争いが構造的に消える)。
  - `ModeKeyPassedThrough` の構築は dylint `ime_event_guard` が `pass_through_observed` 1 箇所に限っている。新しい呼び出し元を足すだけなら、lint もそのまま通る。`architecture_guard.rs:4851-4903` の「3 手の順序」のガードと同じものを新しい関数にも足す。
- **因果の判定には `update_tick_ms` を使う**(`state/mozc_shm.rs` の `RawBlock::update_tick_ms`、「最後に変化を書いた時の `GetTickCount64()`」)。awase の最後の開閉の書き込み、または物理モードキーの tick を `T_w` とする。値が desired と違い、かつ `update_tick_ms > T_w`(GetTickCount の分解能 約 16ms を見込んで `T_w + 16` など)なら、「書き込みや打鍵のあとに外で変わった」と判定して追随する。`update_tick_ms <= T_w` なら「まだ反映されていない(または書き込みが届いていない)」と判定し、追随しない。`pending_write`(H1 参照、範囲が狭い)より正確で、ImmCross の書き込みと物理キーの通過も同じ式で扱える。コメントの「生存の判定には使わない」とも矛盾しない(ここで使うのは生存の判定ではなく前後関係)。

---

## H1(重大)§9 の「Confirmed にする入口」は BUG-156 を生き返らせる。しかも「Confirmed」という区別に実際の効果は無い

**場所**: `ime_model.rs:153-181`(`AppliedImeState::applied_open` は `Optimistic` と `Confirmed` を同じ `Some` で返す)、`is_confirmed()` を呼ぶ本番コード 0 件(`grep -rn "is_confirmed()"` で出るのはテストの `executor.rs:876-883`・`ime_model.rs:1686` だけ)、`ime_actuation_decision.rs:248-254`(`gji_direct_already_matches` は値しか見ない)、`sync_actuation.rs:313-316` と `ime_actuation_decision.rs:203`(`ENGINE_PRESS_UNKNOWNS_APPLIED_IN_TSF_NATIVE = false`)、`ime_model.rs:924-934`(BUG-156: 開閉の予測が applied と食い違えば `Unknown`)、`platform_state.rs:1479-1484`(`pending_write` は `ImeOpenOutcome::Applied`〈SendInput〉のときだけ控える)。

**根拠**:
- 送信を省く判定は `applied_open() == Some(open)` だけ。`Confirmed` か `Optimistic` かは見ない。だから §9 の本当の効果は、**awase が意図して `Unknown` にした場面**(フォーカス変更 `:1022`、PanicReset `:796`、BUG-156 の予測による降格 `:931`、ADR-205 D6 の降格 `:962`)で、**それを次の tick に `Some(x)` へ戻す**ことになる。
- §5-1・§9-3 は「明示キーの押下は `explicit_press_applied_pair` が未知にする」ことを安全の根拠にしている。しかし TsfNative の窓(`effectively_tsf_native`、WT/WezTerm と、実質 TSF ネイティブの Imm32Unavailable)の Engine 経路は、未知にしない(`unknowns_applied = press.is_some() && engine_press_unknowns_applied(..)`、定数が `false`)。つまり**押下の書き込みも `applied` の値で省かれる**。
- §9-3 の「awase 自身の書き込み直後は `pending_write` で保留」が効くのは SendInput の `Applied` だけ。ImmCross(`AppliedWithoutSendInput`)は控えない(`platform_state.rs:1479-1481` のコメントは「IMM の状態は同期的に変わる」が理由だが、**TIP が共有メモリに書くのは compartment の通知を受けた後で、同期ではない**)。物理キーが IME へ通過した場合(awase の書き込みではない)は、控えの対象外。しかも `pending_write` は 1 枠しかなく、`confirm_awase_write` は別スコープの読みが来ると捨てる(`external_change_watch.rs:90-111`)。

**失敗シナリオ(具体)**: WT × mozc-readable(TsfNative、GJI 系)。
1. 利用者が半角/全角を押す。IME へ通過し、`KeyEffectPredicted(open=false)` で `applied` は `Unknown` に落ちる(BUG-156 の修正)。
2. その 40ms 後に tick が来る。共有メモリはまだ前の値の open(キーから共有メモリが変わるまでの実測は約 61〜120ms)。§9 の入口が `applied = Confirmed(true)` と書く。
3. 約 100ms 後に IME は閉じるが、次の tick まで `applied` は `Confirmed(true)` のまま(次の tick で直るのは「読めた」場合だけ)。
4. その間に利用者が変換(Engine の `SetOpen(true, press=Some)`)を押すと、TsfNative なので押下でも未知にならない → `gji_direct_already_matches(Some(true), true)` → **省かれる → IME が開かない**(BUG-156 と同じ症状)。

tick(500ms)が遅れの窓(約 120ms)に入る確率は、変化 1 回あたり約 2 割。トレイの `SetOpen`(press=None)でも同じことが起きる。ADR-214 の P1 を直すための入口が、**逆向きの P1 を作る**。

**推奨対応**:
- §9 は**下げる方向だけ**にする。共有メモリが `applied` と食い違ったら `Unknown`、一致なら触らない。これは単調で安全(悪くても余分に送るだけで、省きはしない)。P1(古い `applied` が「一致」と言って送らない)は、共有メモリが食い違う場面なので、これだけで直る。
- この降格は、C2 の代案(`pass_through_observed(.., demote_applied=true)`)の中ですでに実装されている。**新しい入口も variant も要らない**。
- どうしても `Some(x)` を書くなら、少なくとも `update_tick_ms > max(最後の awase の書き込み, 最後のモードキー)` を条件にし、`ENGINE_PRESS_UNKNOWNS_APPLIED_IN_TSF_NATIVE=false` の窓では書かない。

---

## H2(重大)「古い値を残さない」(§3)は `derive_filtered` の中でしか成り立たない

**場所**: `observation_store.rs:451-475`(`record_any` は必ず `expires_at: None`)、`:702-713`(`most_recent_trusted`: `is_expired` だけ見て、鮮度 3 秒も窓の照合も見ない)、`:720-729`(`_excluding`: 同じ)、`:827-856`(鮮度と窓の照合は `derive_filtered` だけ)、`:499-503`/`:555-561`(プールを消すのはプロセス変更の `FocusChanged` だけで、同一プロセス内の `FocusHwndUpdated` は fence の hwnd を変えるだけ)。

**根拠と、古い open が採られる実際の経路**:
1. **`active=0` の後(同じ窓で TIP を MS-IME に切り替えた等)**: 報告は止まるが、最後の `MozcShm` はプールに残る(`expires_at: None`)。3 秒後に `derive_any` からは外れる。しかし TsfNative の窓には Medium 以上のほかの観測がほとんど無い。結果、`resolve_open_at` のフォールバック `most_recent_trusted`(`ime_model.rs:480`)が**その古い High を無期限に採り続ける**(プロセスが変わるまで)。
2. **同一プロセス内の窓の切り替え(別スレッドの窓、別スレッドの子窓)**: プールは消えない。移った先のスレッドの共有メモリが `active=0` か開けないと、前の窓の `MozcShm` が残る。§2 の案で `is_identity_ok` に足しても、守られるのは `derive_filtered` だけ。フォールバック(上記)と drift(最大 1.5 秒)は前の窓の値を使う。このとき `last_intent` も `applied` も `FocusHwndUpdated` では消えない(`ime_model.rs:877-881`)。
3. 案 (c)「source ごとの TTL」の可否(§3・§8 の「コードで未確認」): **入れ物はもうある**。`ImeObservation::expires_at` と `is_expired`(`:208-221`)はすべての読み手が見ている(`derive_filtered`・`most_recent_trusted`・`_excluding`・`_after*`・`heuristic_default`)。いまは `record_any` が常に `None` を入れているだけ。

**推奨対応**:
- 案 (c) を、`record_any` で `source == MozcShm` のときだけ `expires_at = Some(at + TTL)` を入れる形で実装する。`at` から決まる値なので、journal の再生でも同じ結果になる。TTL は tuning 定数(`#[measured]`)にし、tick の間隔の実測を添える。こうすれば 3 つの読み手すべてで同時に失効し、1・2 の経路が消える。§3 の (a)「3 秒を受け入れる」は、フォールバックでは無期限になるので不成立。
- 窓の照合は `derive_filtered` だけでなく、`MozcShm` については `most_recent_trusted` と `_excluding` にも入れる。または TTL を 1 tick 余り(約 700ms)まで短くして、実質不要にする。

---

## H3(重大)§9-2 の書き込み点は 2 通りに読め、どちらにも穴がある

**場所**: `platform_state.rs:862-870`(`record_confirmed_in_scope` → `note_awase_write_for_mode_key_pass_in_scope` → `confirm_applied`)、`ime_model.rs:564-571`(`confirm_applied` は reduce の外で、pending を解く)、`ime_model.rs:585-620`(pending が無いと完了は `Stale`)、`architecture_guard.rs:2395-2410`(`applied_direct_assignments_are_accounted_for`)。

**根拠**: §9-2 は「`reduce()` が書く(`record_confirmed_in_scope` と同じ書き込み点)」と書いている。しかし `record_confirmed_in_scope` は reduce を通らない(`ImeStateHub` から `ImeModel::confirm_applied` を直接呼ぶ)。
- **(i) `record_confirmed_in_scope` を流用した場合**: (a) `note_awase_write_for_mode_key_pass_in_scope` が「awase が書いた」と記録し、モードキー通過の判定(`mode_key_pass_mark`)を汚す。(b) `confirm_applied` は `pending.target == open` なら pending を消す。awase の apply が飛行中に、共有メモリがたまたま同じ値を示すと pending が消え、本物の完了は `completion_can_update_applied` で `Stale` になる。結果、**composition/warmup の副作用が走らない**(ADR-108 の「受理」は pending の generation 一致が条件)。
- **(ii) reduce の新しいアームにした場合**: `ImeEvent` は journal に直列化される(ADR-082、`ime_event.rs:65` 付近)ので、journal の形式・再生・ガードの更新が要る(`applied_direct_assignments` の `ime_model.rs` 9→10、`ime_event_guard` の指定の呼び出し点、`architecture_guard` の構築件数)。pending に触らないことも決める必要がある。

**推奨対応**: H1 の下げる方向だけの形にすれば、既存の `ModeKeyPassedThrough` で足りて、この節はまるごと要らない。残すなら (ii) にし、pending と mode-key-pass に触らないことをテストで固定する。

---

## M1(中)打鍵予測の照合の待ち(170ms)と共有メモリの遅れ(61〜120ms)の差が小さく、High の古い読みが正しい予測を消す

**場所**: `ime_model.rs:687-718`(`reconcile_key_effect_open`: Medium 以上の観測が `KEY_EFFECT_SETTLE_MS` を過ぎていれば予測を消す)、`tuning.rs:492`(`KEY_EFFECT_SETTLE_MS = 170`、`pending = true`。実測は IMM の読み直しまでで、共有メモリの遅れではない)。

**失敗シナリオ**: 負荷時に TIP の書き込みが 170ms を超えて遅れると、180ms 時点の tick の読み(打鍵前の値、High)が「観測」として予測と照合され、`[key-effect-miss]` が出て予測が消える。`derive_any` は High の古い値を返すので、belief は**次の tick まで約 500ms、打鍵前の値に戻る**(BUG-156 と同じ形のリテラル化)。遅れの測定(61〜120ms)は実機 1 台・負荷なし。CI の負荷時に測ったのは `read_us` で、TIP の書き込みの遅れではない。

**推奨対応**: 照合の前に `update_tick_ms < pred.at_ms`(打鍵より後に変化していない)なら「まだ処理前」として照合しない。上限は窓(例: 300ms)で切る。C2 の因果の判定と同じ式で書ける。

---

## M2(中)tsf_open と effective_open(§2・§9-3、(d))

**根拠**:
- エンジンの belief が答えるべき問いは「いま送ったローマ字を IME が変換するか」。これは入力スコープで上書きした後の `effective_open` の領分。applied が答えるべき問いは「awase の書き込みで動く compartment の値」で、こちらは `tsf_open` の領分。設計は belief に `tsf_open` を使い、食い違ったときは何もしない。安全側ではある。ただし、§9-1 で名指ししたパスワード欄が、**belief を正す価値が一番大きい場面**でもある(パスワード欄で NICOLA のかなが `ka` として入る)。
- 逆に belief に `effective_open` を入れて、drift を除外しない(C1)と、致命的なループになる。desired=true、観測=false で、awase の書き込みは compartment(`tsf_open`)を変えるだけで `effective_open` は変わらない。Read 方針は収束しないので、**毎 tick 永久に送る**。
- 頻度の測定には新しい仕掛けは要らない。journal の `MozcShmRecord` にはすでに両方が入っている(`journal.rs:194`/`198`)。

**推奨対応**: 段階 1 は設計どおり、食い違ったら belief に入れない。ただし applied は降格する(`Unknown`)。既存の journal で食い違いの頻度と場面を数え、`effective_open` を belief に使うかは、C1 の drift 除外が入った後で決める。drift 除外が無い状態で `effective_open` を入れるのは禁止(上のループ)。

---

## M3(中)毎 tick の報告で、journal の State の枠(2048 件)が約 17 分で埋まる

**場所**: `platform_state.rs:190-240`(`dispatch_event` は毎回 journal に記録する)、`journal.rs:1034-1040`(`ImeEvent` は State の枠)、`journal_policy.rs:23-25`(State は 2048)。

**根拠**: 1 tick に 1 件の `ObserverReported(MozcShm)`、つまり 1 秒に約 2 件で、ほかの ImeEvent がなくても約 17 分で State の枠が入れ替わる。不具合報告に載る IME の状態の履歴が大きく縮む。影の読み(`mozc_shm_shadow.rs`)は、これを避けるために変化したときだけ記録していた。

**推奨対応**: H2 の TTL と組み合わせて、報告は「値が変わったとき」と「TTL の半分ごとの更新」だけにする(例: TTL 1.5 秒なら 700ms ごと)。毎 tick にはしない。

---

## M4(中)自動判定にすると、mozc-readable を使っていない全利用者で、毎 tick 余計な Win32 呼び出しが走る

**場所**: `observer/mozc_shm.rs` の `read_inner`。`Local\` で開けないと、AppContainer かどうかの判定(`OpenProcess` → `OpenProcessToken` → `GetTokenInformation`)を毎回行う。`mozc_shm_shadow.rs:47` は現在オプトインで止めている。

**根拠**: §0「オプトインは要求しない」にすると、GJI・MS-IME の利用者でも、約 500ms ごとに main スレッドで上記を実行する。AppContainer の窓では `NtOpenSection` まで行う。どれもブロックしない API だが、効果が無いのに負担だけ全員にかかる。

**推奨対応**: 有効な TIP が GJI 系(`is_gji_family_description`)のときだけ読む。または (tid, pid) ごとに `NotFound` を覚えて、フォーカスが変わるまで読まない。

---

## M5(中)窓の照合とスレッド ID の再利用(§2・§6(2)、(f))

- §2 の入口 `report_mozc_shm_open(open, hwnd, focus_epoch, tick)` が、`report_conv_open_inference`(`platform_state.rs:2035`)と同じく epoch を `current_fence()` から取るなら、epoch の照合は**記録した瞬間には必ず通る**(同義反復)。照合が効くのは「次の tick 以降に古い観測が残ったとき」だけで、それは H2 のとおり `derive_filtered` の中でしか効かない。
- 共有メモリの読みの本当の同一性は (tid, pid) で、hwnd ではない。同じスレッドの別の窓は同じ状態を共有するので、hwnd の照合は正しい観測を 1 tick 捨てることがある(害は小さい)。
- §6(2)「`pid` の照合が tid の再利用を防ぐ」は、**同じプロセスの中での再利用は防げない**(pid が同じ)。スレッドが終わり、同じプロセスの新しいスレッドが同じ tid を得て、古いマッピングがまだ生きていると(TIP が `active=0` を書く前に落ちた場合など)、別スレッドの古い状態を `active=1` で読む。起きる確率は低い。PR 本文の「未測定」の項に、この形を明記すること。

---

## (e) 設計文書の記述で、コードと食い違うもの

| # | 設計の記述 | コード | 出典 |
| --- | --- | --- | --- |
| e1 | §7「`check_drift_correction` は `ConvOpenInference` と `HeuristicDefault` だけを明示的に除外」 | 除外は `ConvOpenInference` だけ。`HeuristicDefault` の除外は FCIS E1 で撤去済み(明示意図の判定が先に返すので到達できなかった)。本体は core の `drift_correction.rs::evaluate_drift` で、`platform_state.rs` ではない | `drift_correction.rs:8-10, 155-158, 177-178`。設計が引用した `ime_event.rs:126-135` の doc が古い(こちらも直すべき) |
| e2 | §3「観測の鮮度は 3000ms で切れる」 | `derive_filtered` だけ。`most_recent_trusted`(belief のフォールバック)・`_excluding`(drift。年齢の上限は別の 1.5 秒)は 3 秒を見ない。`expires_at` は常に `None` | `observation_store.rs:471, 702-729, 832-835` |
| e3 | §2「`BeliefOnly` なので授権に使わない」= IME へ書く根拠にならない | drift correction と `read_back` は authority を見ない | C1 |
| e4 | §1・§0「High は他の観測を上書き」 | 明示意図・打鍵予測は `derive_any` より先。IntentStore は 10/30 秒上書き | C2 |
| e5 | §5-1・§9-3「押下は `explicit_press_applied_pair` で未知にされるので省かれない」 | TsfNative の Engine 経路は未知にしない | `sync_actuation.rs:313-316`、`ime_actuation_decision.rs:203` |
| e6 | §9-1「`external_change_watch` …… 直す先は belief で `applied` ではない」 | 追随の経路は `pass_through_observed(tick, true, true)` を呼び、食い違う `applied` を `Unknown` に落とす(ADR-205 D6) | `platform_state.rs:509-516, 625-629`、`ime_model.rs:956-963` |
| e7 | §9-2「`reduce()` が書く(`record_confirmed_in_scope` と同じ書き込み点)」 | `record_confirmed_in_scope` は reduce を通らない。mode-key-pass の記録と pending の解除という副作用を持つ | H3 |
| e8 | §9-3「窓が変わったら `applied` はフォーカスで未知に戻る」/ §9-4 未確認 (a) | 未知に戻るのは `FocusChanged`(プロセス変更)と PanicReset だけ。`FocusHwndUpdated`(同一プロセス内の窓)では戻らない。`record_confirmed_in_scope` の scope は mode-key-pass の記録に使うだけで、`applied` の有効範囲ではない | `ime_model.rs:877-881, 1022`、`platform_state.rs:862-870` |
| e9 | §9-3「`pending_write` が awase 自身の書き込みを保留する」/ §9-4 未確認 (b) | SendInput の `Applied` だけで、ImmCross は対象外。枠は 1 つで、別スコープの読みが来ると捨てる。物理キーの通過は対象外 | `platform_state.rs:1479-1484`、`external_change_watch.rs:75-111` |
| e10 | §1「IME へ何も送らない」 | C1 のとおり drift 経由で送る | C1 |
| e11 | §3「`is_expired` が source ごとの TTL を持てるかは未確認」 | 持てる。フィールドはあり、全読み手が見ている。`record_any` が `None` を入れているだけ | H2 |
| e12 | §6(2)「pid 照合で tid 再利用を防ぐ」 | 同一プロセス内の再利用は防げない | M5 |

---

## (f) 測定で覆われていない危険が、High で一番効く場面

1. **キー直後の約 120ms**(M1・H1): 実機 1 台・負荷なしの測定しかない。High はここで正しい予測を消し、§9 なら applied を毒する。物理キーでの複数の窓の測定(実機 Notepad 1 窓、belief の誤り 1/28)は、この遅れの分布を測っていない。必要なのは `update_tick_ms - キーの tick` の分布で、CI の負荷時のもの。
2. **TsfNative の窓**: 共有メモリ以外に Medium 以上の観測がほぼ無い。だから H2 の古い値の残留と C1 の drift の解除が、そのまま挙動に出る。測定は Notepad(Standard)中心。flip の CI がどの方針の窓で測ったかを PR 本文に書くこと。
3. **同一プロセス内の窓の切り替え**(H2-2): flip の測定はフォーカス移動だった。同じプロセスの別スレッドの窓(プールが消えない経路)を含むかが未確認。
4. **スレッド ID の同一プロセス内の再利用**(M5)。

---

## (g) ADR-214 を再開せずに applied を触ることの是非、複雑性予算

- §9 は `applied` に「観測由来の Confirmed」を足す。API の成功でも観測でも同じ `Confirmed` になるので、ADR-214 が分けたい Sent と Confirmed の区別を**さらに混ぜる**。しかも H1 のとおり、`Confirmed` を見る本番コードは無い。ADR-214 を再開しないなら、`applied` に**新しい値を書く**変更はすべきでない。**下げる方向だけ**(`Unknown` にする)なら、ADR-098 決定 1-b(省略は陽性の証拠にだけ基づく)・BUG-156・ADR-205 D6 の既存の原則の範囲に収まり、ADR-214 の再開に当たらない。
- 複雑性予算(`.claude/rules/complexity-budget.md`、未発効)の対象は actuation の合流点と tuning 定数。§9 の新しい variant と新しい入口は対象外の数え方だが、ADR-158 RC4 の趣旨(足すだけで減らさない)に照らすと、次の差がある。
  - 設計案: 新しい `ObservationSource`(9 ソースの網羅 `match` が 3 箇所と `authority`)、新しい `evidence` 型、新しい `ImeEvent` variant(journal の形式、dylint、ガード件数)、ハブの入口 2 本、tuning 定数 1 本。
  - 代案: `ObservationSource::MozcShm` と evidence 型、TTL の定数 1 本だけ。belief の訂正と applied の降格は、既存の `pass_through_observed` を使う(新しい variant 0)。drift と `AnyFreshEvidence` の除外リストに 1 要素ずつ。
- tuning 定数(TTL)は `tuning-constants.md` に従い、tick の間隔と遅れの実測を本文に書く。

---

## 推奨する設計の修正(まとめ)

1. `MozcShm` は High・`BeliefOnly` のまま、`record_any` で `expires_at = at + TTL`(H2)。報告は変化したときと TTL/2 ごと(M3)。
2. drift の根拠と `AnyFreshEvidence` から `MozcShm` を除外(C1)。
3. 共有メモリの値が belief(実効値)と食い違い、かつ `update_tick_ms > max(最後の awase の書き込み, 最後のモードキー)` なら、既存の追随の 3 手(記録 → `intent_store.remove` → `pass_through_observed(tick, true, true)`)を実行する(C2)。desired が揃い、明示意図が外れ、applied は食い違えば `Unknown` に落ちる(H1 の下げる方向だけ)。
4. `tsf_open != effective_open` のときは belief に入れず、applied は降格(M2)。食い違いの頻度は既存の journal で数える。
5. §9 の Confirmed の入口は取り下げる。必要と分かったら(下げる方向だけで P1 が残る事例を journal で見つけたら)ADR-214 を再開して決める(g)。
6. 読むのは GJI 系の TIP のときだけ(M4)。
7. ガード: 新しい追随関数の 3 手の順序のガード(`architecture_guard.rs:4851-4903` と同形)、drift 除外のテスト(`MozcShm` だけでは `evaluate_drift` が `Ok` にならない)、`AnyFreshEvidence` 除外のテスト、TTL の失効が `most_recent_trusted` にも効くテスト、`update_tick_ms <= T_w` で追随しないテスト。
