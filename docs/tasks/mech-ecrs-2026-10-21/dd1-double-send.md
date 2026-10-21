---
title: DD1 — 「同じ書き込み・同じキーを二重に送る」を防ぐ防御の重なり
status: 調査のみ(2026-10-21、origin/develop e4766b2a)。読み取りと git grep / git log のみ。cargo は未実行
created: 2026-10-21
related_adr: ["ADR-119", "ADR-171", "ADR-180", "ADR-208", "ADR-212", "ADR-241"]
---

# DD1 — 二重送信の防御の重なり

## 結論

- **E/C 候補は 3 件**(E 2・C 1)。うち確度が高いのは 1 件だけ。残りの防御は、読んだ範囲では**別々の状況を止めている**(K)。
- 上位(確度順):
  1. **DD1-01(E、高)** `open_chain.rs::imm_cross_write` の InputRelay gate(と `with_app` の取り直し)は、`run_open_chain_async` の gate と **同じポーリングの中で `.await` を挟まず**に続けて評価されるので、結果は必ず同じになる。ADR-119・ADR-180 が「別の `.await` 境界にいるので独立に要る」とした前提は、この 1 本については成り立たない。本番から約 45 行減る。
  2. **DD1-02(E、小・中)** `PressLedger` の「shadow が後から来る」場合の分岐(`ConflictKept`、`kp_shadow_actuate` の `ClaimDeclined`)は、読んだ範囲では到達しない。shadow はいつも同じ押下の最初の経路で、同じ `RawKeyEvent` が shadow 段を 2 回通る経路も見つからなかった。台帳そのもの(Engine 側の `Duplicate`/`ConflictEngineWins`)は残す。到達するかどうかは、不具合報告の journal にある `PressWriteClaim` の集計で決める。
  3. **DD1-03(C、低。MC-C1 に依存)** drift correction は「明示意図があるか」を 2 回判定している。`evaluate_drift` が `last_intent` で 1 回(`NotExplicitIntent`)、warrant が `IntentStore` で 1 回(窓ごと・TTL つき)。その差が `SkipWarrantWouldBlock` として表に出ている。MC-C1(2 つの記憶の統合)と同時にやるなら、差の分だけ `SkipWarrantWouldBlock` が縮む。単独ではやらない。
- **見立ての確度**: DD1-01 は構造だけで言える(同じポーリングで `.await` が無いことと、`spawn_local` の初回ポーリングが後回しになることを、コードと依存 crate のソースで確かめた)。DD1-02 は「到達しない」が読みだけの主張なので、実データが要る。DD1-03 は MC-C1 の設計しだい。
- 所有者の見立てとの関係: この失敗の種類では、「同じ不変条件を複数の層が守る」ものは思ったより少なかった。ADR-208 L1 が `applied` による省略を押下 ID の台帳へ移し、ADR-180・ADR-241 が合流点を整理し終えているためである。残っている重なりは主に、**`with_app` の再入と `.await` 境界をどう読むか**から生まれている(DD1-01)。

## 1. 守っている不変条件(この報告の対象)

- **INV-DS1(1 押下 1 書き込み)**: 物理キー 1 押下(`PressId`)につき、awase が OS IME へ出す開閉の書き込みは同じ向きで高々 1 回。BUG-113(WT×GJI×PSReadLine で `VK_IME_OFF` が 2 回出ると「@」が入る)。
- **INV-L1(配送か書き込みのちょうど一方)**: 物理キーを OS に届けるなら awase は書かない。書くなら物理は届けない(ADR-208 決定1、BUG-46)。
- **INV-DS2(書き込みの後に古い観測で同じ向きを重ねない)**: 押下の書き込みの直後に、drift correction が同じ向きを再送しない(BUG-113 型、journal 01M3NJ784NKMH120HM6QGKF7W7 の 106ms 後の再送)。
- **INV-DS3(awase が actuation を持たない窓では書かない)**: InputRelay(issue #136/BUG-90 決定4、ADR-119)。物理を Allow するので、書くと INV-L1 が破れる。
- **INV-DS4(1 つの証拠で再送するのは 1 回)**: GJI の候補窓の再表示(`candidate_was_seen`)を根拠にした OFF の上書きは、1 つの証拠につき 1 回(ADR-171)。

## 2. 防御の目録

(a) 不変条件 (b) それだけが止める状況 (c) 根拠・固定しているもの (d) ほかの防御との関係

| ID | 防御(ファイル:行) | (a) | (b) 単独で要る場面 | (c) | (d) |
|---|---|---|---|---|---|
| P1 | `PressLedger::claim`/`release`(`crates/awase-ime-core/src/state/press_ledger.rs:155`/`:141`)。殻は `platform_state.rs:267`(claim)・`:287`(release) | DS1 | Engine の `SetOpen` が、同じ押下で先に書いた shadow と同じ向きになる場合。ただし P2 があるので本番で起きるかは未確認(DD1-02) | ADR-208 L1、`explicit_press_exhaustive.rs::p5_same_press_sends_once`(`p5_ledger` は事前問い合わせ無しでも違反 0)、`architecture_guard.rs:7616 press_id_is_claimed_and_carried_at_every_order_issuing_entry` | P2 と同じ不変条件を別の層で守る |
| P2 | `engine_owns_open_key`(`crates/awase-windows/src/runtime/key_pipeline.rs:2408`)→ `kp_stage_shadow_ime_toggle` の `if engine_owns_open_key`(`:1124`) | DS1(と belief の二重更新) | Engine が同じ打鍵で `SetOpen` を出すキーで、shadow が **belief・意図・書き込みのどれにも**触らないこと。P1 は書き込みしか止めず、belief は止めない | PR #419 Opus M-4(`8ba1fc2f`、本文「ledger の衝突分岐は防御線として残す」)、`architecture_guard.rs:7678/7686/9413` | P1 と重なる。P2 は書く前に静的に止め、P1 は書く時点で止める |
| P3 | `key_shadow_action` の `explicit_overlap`(`crates/awase-key-effect/src/state/key_effect_runtime.rs:535`、呼び出しは `runtime/mod.rs:877`) | DS1・L1 | 役割で付けた `shadow_action` が、config の素の `keys.ime_*` と重なる場合。`shadow_action` は物理配送(P8)・ラッチ・`is_japanese_ime` の格上げにも効くので、P2 では代わりにならない | ADR-199 決定8、`99b64c3b` | P2 と一部重なる(書き込みの抑止)が、P3 は配送にも効く |
| P4 | Engine が同期キーを照合しない(`src/engine/tests.rs:9260 matches_ime_set_open_ignores_keys_that_are_also_sync_keys`) | DS1 | `keys.ime_detect` と `keys.ime_*` が同じキーのとき、shadow 側だけが動く | M-4 | P2 の前提 |
| P5 | `explicit_press_demotes_applied`/`explicit_press_applied_pair`(`ime_actuation_decision.rs:191/233`)、Engine 側の段階制御 `ENGINE_PRESS_UNKNOWNS_APPLIED_IN_TSF_NATIVE=false`(`:203`) | DS1 の**逆**(古い `applied` で押下を省かない、S-1) | 二重送信の防御ではない。二重送信の防御を `applied` から P1 へ移した結果、省略を外すために要る | ADR-208 D1、`ime_actuation_decision.rs:904-982` のテスト | P1 があるから外せる |
| P6 | `gji_direct_already_matches`(`ime_actuation_decision.rs:248`) | DS1(`press=None`)・DS4 | 自動リピート(`press=None`)で `VK_IME_ON` を毎秒約 30 回送らない。TsfNative の Engine 経路(L3' 待ち)では、OFF の押下ごとに単発の `VK_IME_OFF` が出ないようにする(BUG-124 型の「@」) | BUG-113、ADR-208 決定1(自動リピートは対象外)、`explicit_press_exhaustive.rs::p6_repeat_never_rewrites_gji_direct` | 押下 ID を持つ書き込みは P1、持たない書き込みは P6 が受け持つ(重ならない) |
| P7 | `consumes_candidate_evidence` → `reset_candidate_was_seen`(`sync_actuation.rs:88`、`ime_controller.rs:140`) | DS4 | 同じ証拠を `press=None` の書き込みや別の押下が読み直さないようにする | ADR-171、`tsf/observer.rs::reset_candidate_was_seen_takes_effect_synchronously` | 同じ押下の 2 回目は P1 も止めるが、P7 は押下をまたいで効く |
| P8 | `PhysicalKeyDisposition::plan_core`(`crates/awase-ime-core/src/state/physical_disposition.rs:109`)。InputRelay は Allow(`:117`)、注入は Allow(`:158`) | L1 | 配送側の決定。shadow で倒れたら Suppress、ImmCross 窓の IME 系キーも Suppress | BUG-46/BUG-116/ADR-119、`explicit_press_exhaustive.rs::physical_delivery_matches_the_audit_table` | 書き込み側(P9〜P13)と対になる |
| P9 | D4 の固定点 `kp_shadow_noop_write` の `plan(false)`(`key_pipeline.rs:1336`) | L1 | shadow が no-op の窓で、物理を Allow するなら書かない | ADR-208 L3a、`l3a_changes_only_the_suppressed_shadow_noop_write` | P8 を 2 回評価する(設計上の固定点) |
| P10 | `PhysicalKeyDisposition` の Down/Up のそろえ: `kp_latch_keyup_to_keydown_disposition`(`key_pipeline.rs:399`)、`key_role_latch`、hook の親指ラッチ | L1(KeyUp 単独の配送を防ぐ) | 3 つは受け持つキーを分けている(`excluded` に無変換/変換・F13〜F24・注入)。重なりは無い | BUG-131/132、BUG-173 追補(`bb6e9440`) | 互いに排他 |
| P11 | `apply_sync` の gate(`crates/awase-windows/src/state/sync_actuation.rs:227`) | DS3 | 同期経路の唯一の合流点。shadow の同期書き込み(`kp_shadow_actuate` は gate より前に台帳を予約し、gate を持たない)と drift の StrategyChain は、ここ以外に gate が無い | ADR-119、ADR-241、`architecture_guard.rs:5907` | executor 経路では P12 と同じ入力で 2 回評価する |
| P12 | `dispatch_set_open` の gate(`sync_actuation.rs:317`) | DS3 | 台帳の予約(`:348`)より前に止める(書かない窓では予約しない)。`DecisionSite::DispatchImeSetOpen` の記録 | ADR-163 B-3、ADR-241 | P11 と同じ入力を同じ同期呼び出しの中で見る |
| P13 | `run_open_chain_async` の gate(`crates/awase-windows/src/runtime/open_chain.rs:599`) | DS3 | 起案(Standard の窓で `imm_first`)から `spawn_local` の初回ポーリングまでの間に、フォーカスが InputRelay へ移る場合。初回ポーリングは PostMessage 経由で後回しになる(winmsg-executor 0.3.2 `lib.rs:67-85`、`runnable.schedule()` → `PostMessageA`) | ADR-119 #2 | P14 と同じポーリングで続く(DD1-01) |
| P14 | `imm_cross_write` の gate(`open_chain.rs:245`) | DS3 | **無い**(DD1-01) | ADR-119 #4(`/code-review` で追加)、ADR-180 表の 4 | P13 と同じポーリング・同じ `with_app` の状態で、`.await` を挟まずに評価される |
| P15 | `fallback_write` の gate(`open_chain.rs:503`) | DS3 | ImmCross の `.await` の後に、フォーカスが InputRelay へ移った場合 | ADR-119 #3 | P13 との間に `.await` があるので独立(K) |
| P16 | `fallback_write` の `shadow_on = None`(`open_chain.rs:496`) | DS1 の逆(フォールバックを省かない) | ImmCross が `Failed` で不一致(または不明)を返した後に、shadow の OFF 経路が事前に書いた `record_confirmed(false)`(`key_pipeline.rs:1434`)を根拠に GjiDirect を省かないこと | BUG-113 追補 | P5 とは別の理由(押下の有無を問わない) |
| P17 | `imm_cross_reobservation_already_matches`(`ime_actuation_decision.rs:275`、呼び出しは `open_chain.rs:371`) | DS1(ImmCross と VK の重ね送り) | ImmCross が `Failed` でも実 IME が既にその向きなら、VK を追い送りしない | `docs/tasks/actuation-confluence-already-matched-gap.md`、同ファイルの単体テスト | 時間切れの場合は追い送りを止めない(`open_chain.rs:381-387`、冪等なので無害と明記) |
| P18 | 書き込み前の `TIMER_IME_REFRESH` の kill。shadow は `key_pipeline.rs:1409`、Engine は `:1562` | DS2 | 押下の前に予約された refresh が、書き込みの直後に古い観測で drift を発火させること | PR #408 Opus M-1、ADR-208 D1 | 入口 2 つに同じ防御を置いている(入口ごとに要る。MA-05 の統合に入る) |
| P19 | drift の `ConvOpenInference`・`MozcShm`・`TaskbarGlyph` を根拠にしない(`drift_correction.rs`、`most_recent_trusted_excluding`) | DS2 | 反証できない conv の推測を根拠に、押下の 106ms 後に同じ `VK_IME_OFF` を重ねること | BUG-173 追補3、journal 01M3NJ784NKMH120HM6QGKF7W7 | P18 とは別の窓(書き込みの後の refresh) |
| P20 | drift の再送の件数と打ち切り(`FeedbackPolicy::Blind` の `max_attempts`、BUG-68 のクールダウン、`Read` の `Confirmed`)(`drift_plan.rs`、`ime_refresh.rs:1091-1281`) | DS2(同じ VK の無限再送) | BUG-43 のタイトループ | ADR-080、`drift_plan.rs` の単体テスト | 順に効く(重ならない) |
| P21 | drift の `SkipWarrantWouldBlock`(`drift_plan.rs:405`)と `evaluate_drift` の `NotExplicitIntent`(`drift_correction.rs:132-137`) | 書けない補正の空振りと、推測の書き込み | DD1-03 | BUG-163(`b6ab8980`、2026-09-25)、ADR-212 P6(`6570c324`、2026-09-30) | 明示意図を 2 つの記憶で 2 回判定している |
| P22 | executor の `applied_snapshot` のバッチ内更新(async では `Optimistic`、`executor.rs:742`/`:828`) | DS1(`press=None` の同じバッチ内) | 同じバッチに `press=None` の `SetOpen` が 2 つある場合。1 つのバッチに 2 つ入るかは未確認(`ime_set_open_effects` は 1 件しか出さない、`src/engine/engine.rs:898-914`)。もう 1 つの消費者 `resolve_warmup_ime_on` は warmup の領域 | ADR-207 | P1 とは押下 ID の有無で分かれる |
| P23 | Engine の `prev_activation`(「二重 enqueue 防止」、`src/engine/engine.rs:888`) | 次の打鍵で同じ `SetOpen` を再発行しない | 押下をまたぐ再発行 | `src/engine/tests.rs:8395` | P1 は同じ押下の中だけなので重ならない |

確かめたこと(コマンドと結果):

- `git grep -n -E '念のため|多重防御|belt|防御線|二重送信|二重に送|冗長' -- 'crates/*.rs' 'src/*.rs' | grep -v /tests/` → 48 行。この失敗の種類に関わるのは press_ledger・ime_set_open_plan・sync_actuation・open_chain.rs:582(「ここでも念のため」)・key_pipeline.rs:1390・ime_controller.rs:128・platform.rs:1171 だけ。
- `awk 'NR>=567 && NR<=694 && /\.await/' crates/awase-windows/src/runtime/open_chain.rs` → `.await` は 694 行の `run_chain_async(..).await` の 1 つだけ(gate のある 599 行より後で、チェーンに入る前の `.await` は無い)。`NR>=223 && NR<=262`(`imm_cross_write` の gate まで)にも `.await` は無い。
- `WriteMechanism::ALL = [ImmCross, GjiDirect, MsImeDirect]`(`crates/awase-ime-core/src/state/actuation_chain.rs:168`)。`AsyncChainWriter::is_applicable(ImmCross) = self.imm.is_some()`(`open_chain.rs:113`)なので、`run_chain_async` の最初の繰り返しでそのまま `imm_cross_write` に入る(`actuation_chain.rs:562-567`)。

## 3. 候補

### DD1-01(E、確度: 高)`imm_cross_write` の InputRelay gate を消し、`run_open_chain_async` の gate の結果を渡す

**現状**: 非同期チェーンの gate は 3 か所ある(`run_open_chain_async`・`imm_cross_write`・`fallback_write`。どれも `is_input_relay(`、`architecture_guard.rs:5944-5970` が関数ごとに 1 件・ファイル全体で 3 件に固定している)。

**なぜ 2 か所目が要らないか**:
1. `run_open_chain_async` の gate(`:599`)から `imm_cross_write` の gate(`:245`)までの間に `.await` は無い(上のコマンド)。チェーンの最初の機構はいつも ImmCross で、適用できるかの判定も `self.imm.is_some()` だけなので、最初の `write` がそのまま `imm_cross_write` になる。シングルスレッドの winmsg-executor では、`.await` で手放さない限り、同じポーリングの中で別のメッセージ(フォーカス変更)は処理されない。そのため 2 つの gate が作る view は、同じ状態から作られる。
2. `with_app` が `None`(RUNTIME の再入)を返す場合も同じになる。`:599` で `None` だったなら、借用はポーリングの外側で握られている。その借用は `:245` までに手放されない(間で `with_app` を呼ぶのは NotOwned と Unwarranted の早期 return の分岐だけで、どちらもチェーンへ進まない)。どちらの gate も `None` なら fail-open する(`:251-260`・`:605-619`。PR #201 B-1 で両方を fail-open にそろえた)。
3. したがって、ADR-119 が `imm_cross_write` に gate を足した理由(「2 の `run_open_chain_async` 冒頭 gate が `with_app` 再入失敗で fail-open した場合、修正前はこの関数まで無条件で ImmCross write が到達していた」、`docs/adr/119-…md:171-174`)は、PR #201 B-1 で `imm_cross_write` 側も fail-open に変えた時点でなくなっている。同じ状況では `imm_cross_write` の gate も素通りするからである。
4. ADR-180 は表(`docs/adr/180-…md:87-88`)で 3 と 4 を「別の `.await` 境界の直前・直後」としてまとめて扱った。だが `.await` は 4 の gate の**後**にあり、3 と 4 の**間**には無い。5(`fallback_write`)は確かに `imm_cross_write` の `.await` の後なので独立に要る(K のまま)。

**前回の判断のどこを覆すか**: ADR-180 決定1 は「3・4 の `with_app` クロージャ 9 行の共通化」を見送った(共通化しても 4〜5 行×2 にしかならない)。今回の提案は共通化ではなく **4 の撤去**で、ADR-180 が見送った理由(`fallback_write` からの再入)には当たらない。fix-requires-evidence.md の合流点の表にある「3関数**すべて**が独立に再検出する設計、1箇所だけでは足りない」も、`imm_cross_write` については誤りになる。

**統合の具体案**:
- `run_open_chain_async`: gate の結果 `gate_inputs: Option<DecisionInputs>` を `AsyncChainWriter` に持たせる(フィールド `gate_inputs` を足す)。
- `imm_cross_write(op, open, inputs: Option<DecisionInputs>)`: `:224-260` の `with_app` と `is_input_relay` を消す。`:272-288` の NotOwned の分岐も消す(到達しない)。`AttemptRecord.inputs` には渡された `inputs` を使う。`record_actuation_decision_skipped(DecisionSite::ImmCrossWrite)` も消す。`with_app` が `None` のとき、今は `run_open_chain_async` の末尾とこの関数で同じ 1 回の失敗を 2 回数えているので、それも直る。
- `crate::hook::note_imm_cross_write()`・コマンドの組み立て・実書き込み・`Failed` の読み直しはそのまま残す。
- 減る量: 本番で約 45 行(コメント込み約 65 行を消し、約 20 行を足す)。

**壊れるもの(同じ PR で直す)**:
- `crates/awase-windows/tests/architecture_guard.rs:5944-5970`(`("fn imm_cross_write", 1)` を 0 に、`total_in_open_chain, 3` を 2 に)。ガードのコメント「3つの`.await`境界」(`:5940-5942`、`:5782-5795`、`:5625`)。
- `.claude/rules/fix-requires-evidence.md` の「IME actuation 合流点」の行(「3関数**すべて**が独立に再検出」)。ADR-119 の 4 番、ADR-180 の表の 4 には追記で訂正する。
- 代わりの固定: `architecture_guard` に「`run_open_chain_async` の本体で、`is_input_relay(` から `run_chain_async(` までの間に `.await` が無いこと」と「`WriteMechanism::ALL[0] == ImmCross`」(後者は ungated なので単体テストにできる)を足す。前提が崩れたら(チェーンの前に `.await` を足したら)gate を戻す、と分かるようにする。

**検証**: `open_chain.rs` は `runtime/`(`#[cfg(windows)]`)の中なので、擬似 IME からも Linux のテストからも届かない(INDEX §3「`runtime/` は 1 本も通らない」)。等価性は構造の論証(同じポーリング・同じ借用の状態)と、上のテキストガードで固定する。`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib` で型を確かめる。InputRelay を実際に通るテストは無い(ADR-180 も同じことを書いている)。
**危ないか**: 前提(`.await` が無いこと)が保たれる限り、挙動は 1 ビットも変わらない。前提が崩れる変更(チェーンの前に `capture().await` を足す等)をしたときに gate が無い、という将来の危険は、上のガードが落ちて知らせる。

### DD1-02(E 小+実データ待ち、確度: 中)台帳の「shadow が後から来る」場合の分岐を消す

**現状**: 同じ押下で shadow と Engine の両方が書くことを、P2(`engine_owns_open_key`、書く前に静的に止める)と P1(台帳、書く時点で止める)の 2 層で防いでいる。全列挙テストでは、**それぞれ単独で**違反が 0 になっている(`explicit_press_exhaustive.rs:526-571`。`p5` は P2+P1、`p5_ledger` は P1 だけ)。`8ba1fc2f`(PR #419 M-4)の本文は「ledger の衝突分岐は防御線として残す」。

**読んだ結果**:
- shadow 段(`kp_run_inner` の `:130`)は、同じ打鍵では Engine(`:227 on_input`)・executor より前に走る。このため Engine の予約の**後に** shadow が来ることは、同じ打鍵の 1 回の処理の中では起きない。`press_ledger.rs:24` も「shadow が後に来る順序は現状ありえない」と書いている。
- `kp_shadow_actuate` のコメント(`key_pipeline.rs:1388-1390`)は、台帳が要る場面として「reinject・drain replay・Ctrl 救済の 50ms 保留の再処理」を挙げている。しかし読んだ範囲では、どれも同じ `RawKeyEvent` に shadow 段で 2 回書かせない。
  - Ctrl 救済: 保留(`:214-224`)は shadow 段より**後**にあり、対象は `engine.matches_ime_off` が真の打鍵だけ。その打鍵では `engine_owns_open_key` も真なので、1 回目も再処理(`replay_ime_off_rescue_event` → `kp_run_inner(event, true)`)も shadow は `EngineOwnsOpenKey` で戻り、書かない。
  - TsfGate の保留: `try_hold_key`(`:71`)は shadow 段より**前**で戻るので、shadow 段を通るのは後で 1 回だけ。
  - reinject: awase 自身の再注入は `event.injected` になり、shadow 段の BUG-14 ガード(`:1086-1097`)で戻る。
- そのため、shadow 側の `claim` が `writes()==false` になる分岐(`key_pipeline.rs:1396-1403`、`ShadowToggleStep::ClaimDeclined`)と `PressClaim::ConflictKept` は、読んだ範囲では到達しない。
- Engine 側の `Duplicate`/`ConflictEngineWins` に到達するのは、P2 の予測(`matches_ime_set_open`)が外れたときだけである。外れうる候補は 1 つ見つけたが、確かめていない(**未確認**)。エンジン活性中の親指キーの単独タップは、`SetOpen` を KeyUp/タイムアウトで KeyDown の押下 ID つきで出す(`src/engine/engine.rs:871-…`)。KeyDown 時点の `match_special_keys` は `suppress_ime_combos` で `None` を返す。その親指キーに `sync_direction`(`keys.ime_detect`)が付いていると、shadow が KeyDown で書く。FSM の単独タップが同期キーを除くかどうかは読んでいない。

**統合の具体案**:
- 消す: `PressClaim::ConflictKept`(`press_ledger.rs:86-90` と `claim` の分岐 `:167`)。shadow の claim は予約するだけにする(`PressLedger::reserve_shadow(press, open)` に変え、戻り値で書くかどうかを分けない)。`ShadowToggleStep::ClaimDeclined` と journal のラベル `conflict_kept` も消す。全列挙モデルのうち「shadow が Engine の後に来る」テスト(`explicit_press_exhaustive.rs:1175 shadow_never_overrides_an_earlier_engine_reservation`)は、「起きない」を固定するガード(shadow の claim は `kp_run_inner` で `on_input` より前)に置き換える。減る量は本番で約 20 行と、テスト少し。
- 残す: Engine 側の `Duplicate`・`ConflictEngineWins`・`release`(P2 の予測が外れたときの唯一の防御で、純粋関数で安く、全列挙で固定されている)。
- **やらない**: 台帳ごと消して P2 だけにすること。P2 が外れた場合に起きるのは WT×GJI×PSReadLine の「@」(BUG-113 の症状そのもの)で、外れる候補(上の単独タップ)を潰しきれていない。

**決め方(実データ)**: `claim_press_write` は journal に `PressWriteClaim { verdict }` を残す(`crates/awase-ime-core/src/journal.rs:749-758`、`platform_state.rs:267-283`)。不具合報告(R2、`bug-report-fetch`)の journal で `verdict ∈ {duplicate, conflict_engine_wins, conflict_kept}` を数える。
- `conflict_kept` と、`source=shadow` の `duplicate` が 0 なら、上の E を実施する。
- `source=engine` の `duplicate`/`conflict_engine_wins` が出たら、P2 の予測が外れる実例なので、その構成(どのキーか)を `matches_ime_set_open` の側で直すかを決める。
- リポジトリ内には集計できる journal が無かった(`git grep -l PressWriteClaim -- '*.json'` → 0 件)。

### DD1-03(C、確度: 低。MC-C1 と同時にしかやらない)drift correction の明示意図の 2 重判定

**現状**:
- `evaluate_drift` は、`desired_open` が `last_intent.target` と一致しなければ補正しない(`NotExplicitIntent`、`drift_correction.rs:132-137`。ADR-212 P6、`6570c324`、2026-09-30)。
- ImmCross 窓では、さらに `warrant_would_block`(`drift_plan.rs:405`)を見る。BUG-163(`b6ab8980`、2026-09-25)で入った判定で、**P6 より前**に足されたもの。warrant は `IntentStore::lookup(target, now)`(窓ごと・TTL つき、`intent_store.rs:132-138`)で明示意図を引く。見つからなければ観測(`derive_actuating`)に落ちる。drift 中は観測が `desired` と逆なので、そこで拒まれる。
- したがって P6 の後の `SkipWarrantWouldBlock` は、主に「`last_intent` はあるが `IntentStore` は TTL 切れ/別の窓」という**2 つの記憶の食い違い**を拾っている。ほかに残る経路は PanicReset ガード(`SafetyValve`)と向きの衝突だけ。
- StrategyChain の窓(Imm32Unavailable/TsfNative)にはこの事前判定が無い。同じ食い違いのとき、`DriftDetected`(`applied` を `Optimistic` へ)・試行回数の加算・`Unwarranted` の書き込み試行まで進む(`ime_refresh.rs:1210-1271`)。BUG-163 が避けたかった空振りが、Blind の窓では残っている。

**統合の具体案(MC-C1 で明示意図を 1 つの記憶にするなら)**: `evaluate_drift` の `NotExplicitIntent` を、`IntentStore::lookup(現在の窓, now)` の向きで判定する。そうすると `SkipWarrantWouldBlock` は `SafetyValve` の衝突だけに縮み、StrategyChain の窓の空振りも同じ判定で止まる。送るかどうかの結果は ImmCross 窓では変わらない(今も Skip で送らない)。StrategyChain の窓では「送らない」はそのままで、`DriftDetected` と試行回数の加算が無くなる(診断と `applied` が変わる)。
**単独ではやらない理由**: `last_intent` を読む消費者は drift 以外にもある(MC-C1 の範囲)。drift だけを `IntentStore` に替えると、記憶が 2 つあるまま読み手だけが割れる。
**検証**: `evaluate_drift`・`decide_drift_plan`・`IntentStore` はすべて ungated の純粋関数なので、ME-3(決定表の全列挙 golden)を `decide_drift_plan` に足せば Linux で差分を固定できる。擬似 IME(`tests/support/harness.rs`)の `settle()` は drift を写しで持っているので(INDEX ME-1)、先に ME-1 で本物の `decide_drift_plan` に置き換える必要がある。

## 4. 重なっているが K(残す)とした束

| 束 | 防御 | 判定 | 理由(どの状況で何が壊れるか) |
|---|---|---|---|
| 同期の gate を 2 回 | P11・P12 | K | executor 経路では同じ `inputs` を同じ同期呼び出しの中で 2 回見ており、結果は同じ。しかし P12 を消すと、InputRelay で台帳を予約し(`release` で解けるのは `outcome_sent_nothing` のときだけ。NotOwned は含まれるので実害は無い)、`DispatchImeSetOpen` の記録も消える。P11 を消すと、shadow の同期書き込みと drift の StrategyChain が InputRelay で書く(INV-L1 違反。物理は P8 で Allow 済み)。削れるのは条件 1 行ぶんで、ADR-163 B-3 の記録の区別を失う |
| 非同期の gate | P13・P15 | K | P13 と起案の間には PostMessage による初回ポーリングの遅れがあり、P15 と P13 の間には ImmCross の `.await` がある。どちらの間でもフォーカスが InputRelay へ移りうる |
| 古い `applied` による省略 | P5・P6・P16 | K | P5 は押下つき、P6 は押下なし(リピート・TsfNative の Engine 経路)、P16 は ImmCross の `Failed` 後(押下の有無を問わず、`record_confirmed(false)` の事前書き込みの打ち消し)。それぞれ別の書き込みを受け持つ |
| 候補窓の証拠 | P7・P1 | K | 同じ押下の 2 回目は両方が止める。押下をまたぐ場合と `press=None` の場合は P7 だけが止める |
| drift の重ね送り | P18・P19・P20 | K | P18 は書き込みの**前に**予約された refresh を止める。P19 は書き込みの**後の** refresh が反証できない推測で重ねることを止める。P20 は収束しないときの再送の回数を抑える。時間の窓が別々 |
| 入口ごとの kill | P18 の 2 か所(`:1409`・`:1562`) | K(MA-05 で一緒に) | 入口が 2 つある限り両方に要る。MA-05(`kp_shadow_actuate` と `dispatch_set_open` の並行実装)を統合するとき、「予約 → `note_explicit_ime_action` → refresh の kill」を 1 つの前処理にまとめる。Engine 側の `last_explicit_ime_action_ms` は `on_engine_set_open_request` が立てている(`platform_state.rs:954`)ので、今は対称 |
| 配送と書き込みの排他 | P8・P9・P10 | K | P9 は P8 の固定点の評価(設計)。P10 の 3 つのラッチは受け持つキーが排他(`kp_latch_keyup_to_keydown_disposition` の `excluded`) |
| 静的な重なりの除去 | P2・P3・P4 | K | P3 は配送(P8)とラッチにも効く。P4 は Engine 側の前提。P2 は belief の二重更新も止める |
| バッチ内の `applied` | P22・P1 | K(未確認) | 押下つきの書き込みは P1 が止める。押下なしで 1 つのバッチに `SetOpen` が 2 つ入る経路は見つからなかった。ただし P22 は `resolve_warmup_ime_on`(warmup の領域)にも効くので、二重送信の観点だけでは消せない |

## 5. 再報告しないもの(関係だけ記す)

- **MA-05**(`kp_shadow_actuate` が `dispatch_set_open` の判断を並行して実装している): DD1 の P1・P5・P18 が入口ごとに 2 回書かれているのはこのため。DD1-02 の shadow の claim の簡略化は、MA-05 より前にできる。
- **MC-C1**(`last_intent` と `IntentStore`): DD1-03 はその一部。
- **MB-01**(cold コルーチンの inline LiteralDetect と per-VK confirm の重なり。打鍵の二重出力の防御): DD1 の「per-VK confirm と事後検出」はこれと同じ。
- **MB-06**(`RuntimeOutbox::StartTsfProbe` が belt-and-suspenders、`vk_send.rs:308`): 見送り済み。
- defer キューの二重 flush: `take_pending_deferred_*` が取り出しと同時に空にする構造で、重ねた防御は見当たらなかった(`tsf_warmup_coord.rs:700-721` のテストが「flush 後は空」を固定)。
- ForceGuard: belief を上書きする機構で、二重送信の防御ではない(`ProfilePolicy` は MC-E2 で報告済み)。

## 6. 付随して見つけた古い記述(DD1-01・02 の PR で直す)

- `key_pipeline.rs:1388-1390`: 台帳の理由として「reinject・drain replay・Ctrl 救済の 50ms 保留の再処理」を挙げているが、読んだ範囲ではどれも shadow に 2 回書かせない(DD1-02)。
- `open_chain.rs:224-230`・`:579-592` と ADR-119 の 4 番、ADR-180 の表の 3・4: 「`.await` 境界ごとに独立」「fail-open した場合の防御」(DD1-01)。
- `physical_disposition.rs:167-179`: 「C2対策で無変換/変換にも `shadow_action` が付く」とある。だが今は `enrich_key_role` が無変換/変換を扱わない(`runtime/mod.rs:686-687`「無変換/変換(決定16)は `shadow_action` でなく単独タップ確定点なのでここでは扱わない(T10)」)。分岐は `sync_direction` の昇格とも関わるので残すが、理由の文は古い。

## 検証に要るもの

1. **DD1-01**: `architecture_guard` の新しいテキストガード(`run_open_chain_async` で gate からチェーンまでの間に `.await` が無いこと)と、`WriteMechanism::ALL[0] == ImmCross` の単体テスト。`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib`。InputRelay を実際に通す CI ターゲットは無い。実機で確かめるなら PowerToys Mouse Without Borders(issue #136 の構成)で、IME キーが awase に書かれず素通しされることを確かめる。挙動は変わらない想定なので、必須ではない。
2. **DD1-02**: 不具合報告の journal の `PressWriteClaim` の `verdict` の集計(`bug-report-latest` で直近 N 件。`source` と `verdict` の組で数える)。`source=engine` の `duplicate` が出たら、その押下のキーと Engine の状態(単独タップか)を見る。FSM の単独タップが同期キーを除くかどうか(`src/engine/nicola_fsm.rs::resolve_pending_thumb_as_single` を読む。ルート crate なので `cargo test --lib` で確かめられる)。
3. **DD1-03**: ME-1(ハーネスの drift の写しを本物の `decide_drift_plan` にする)→ ME-3 の `decide_drift_plan` の全列挙 golden(`last_intent` と `IntentStore` の有無・TTL・窓の一致 × 観測 × `imm_cross` × ガード)。MC-C1 の ADR が先。
4. 擬似 IME で確かめられる範囲: DD1-02・03 の判断は純粋関数(`PressLedger`・`explicit_press`・`drift_plan`・`evaluate_drift`)なので Linux の全列挙で固定できる。DD1-01 は `runtime/` の殻で、擬似 IME の能力(ME-2)が整っても `with_app` と `spawn_local` のポーリングの順序は模擬しない。だから構造のガードで固定するしかない。
