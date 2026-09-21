# ADR-190 opus 敵対的レビュー round1

対象: `docs/adr/190-msime-immcross-failure-fallback-idempotent-vk-ime-on.md`（worktree
`/home/cuzic/rust-nicola-worktrees/ci-e2e-scenarios`、HEAD `069fc715`）、`docs/known-bugs/BUG-152.md`。

行番号は全て上記 worktree の HEAD 実コードで確認した。CI ログは
`/tmp/claude-1001/-home-cuzic-rust-nicola/bb73299b-cf5f-480f-ae19-18b2a7fd7145/scratchpad/` 配下。

---

## 先に: ADR の主張のうち、独立に検証して **正しかった** もの

後続の指摘と区別できるよう先に列挙する（前ラウンドの数値も自分で引き直した）。

| ADR の主張 | 検証結果 |
|---|---|
| `CHAIN_IMM_CROSS_THEN_KANJI` は `app_ime_policy.rs:63-65`（定数値の行が 65） | 正しい |
| `ms_ime_direct_applicable` は `key_sequence_policy.rs:60`（署名行） | 正しい |
| 他の呼び出し元は `transport.rs:386` だけ | 正しい。`grep -rn ms_ime_direct_applicable crates/awase-windows/src` のヒットは定義（`key_sequence_policy.rs:60`）+ `ime_controller.rs:146` + `transport.rs:386` の3件のみ |
| その transport.rs の呼び出しは「`can_use_imm32_cross_process()` が真の腕を先に処理する else 内」 | 正しい。`transport.rs:378` が `if profile.can_use_imm32_cross_process() { true } else { … ms_ime_direct_applicable(kind, profile) … }`。よって BUG-46/52/116 の物理 Suppress/Allow 判断は不変（ただし M2 参照） |
| `imm_cross_write` は事後読み取りが `None` でも `Failed`、`fallback_write` の doc と食い違う | 正しい。`open_chain.rs:353-374`（`Failed` 腕）が `if actual == Some(open) { AlreadyMatched } else { Failed }`、`fallback_write` の doc は `open_chain.rs:446-449`。**依頼文の「353-367」は `AlreadyMatched` を返す側までで、`Failed` を返す else 側 365-373 が範囲から外れている**ので、ADR に書くなら 353-374 |
| MsImeDirect の VK は `VK_IME_ON`/`VK_IME_OFF`（`ime_key_for`） | 正しい。`key_sequence_policy.rs:146-147`。ADR-063 の `VK_DBE_*` 記述が古いという指摘も正しい（`ime_controller.rs:24` のモジュール doc も「冪等 VK_DBE_*」のまま stale） |
| 「Chrome が VK_IME_ON/OFF を受け付けない」は Chrome × GJI の話で本件と別 | 正しい。`docs/experiments.md:79,94`（`d4d9e27`）。さらに補強材料: Chrome は `Imm32Unavailable` で、その chain は今日すでに `[GjiDirect]` / `[MsImeDirect]`（`app_ime_policy.rs:67-69`）＝ 本番はもう Chrome にも VK_IME_ON/OFF を送っている |
| a9 の sc-kanji 2/3 FAIL は判定窓超過で、実 IME は正しい | 正しい（詳細は「証拠の裏取り」節。ただし「CI のスパイク側タイマー遅延」という**説明**は裏取り不足、S5 参照） |
| ADR line 34「GJI では同じ呼び出しが 12〜25ms」 | **今回渡された res2/res5/res6 では裏が取れなかった**（msime-native 構成しか掘っていない）。出典（どのランのどの行）を ADR に書くこと |

---

## Blocker

### B1. 決定3「`KanjiToggle` は削除しない（到達不能にするだけ）」の**根拠が成立しない** —— 到達不能にした時点で保険は消える

ADR 残る限界（line 102）は「Microsoft IME 以外（ATOK 等）が `ImeKindId::MsIme` と推定された場合に
`VK_IME_ON/OFF` が効くかは未確認（`KanjiToggle` を残す理由）」と書くが、**コードを残すことは
フォールバックを残すことではない**。決定1+2 を入れると `KanjiToggle` は全経路で構造的に到達不能になる:

- `ImeKindId` は `Gji` / `MsIme` の 2 値しかない（`tsf/observer.rs:634-639` の `ActiveImeKind` と
  `644-651` の `From`。`app_ime_policy.rs` の `caps_chains_match_the_adr089_table` が
  `expected.len() == ALL_PROFILES.len() * ImeKindId::ALL.len()` = 10 行で全数を固定しているのが傍証）。
- **同期経路**: `ImeController::apply` → `caps_chain_for`（`ime_controller.rs:726`）→
  `decide_chain`（`ime_actuation_decision.rs:149-151`）→ `caps(p,k).chain`。決定1 後、どの (p,k) の
  chain にも `KanjiToggle` は現れない（`CHAIN_IMM_CROSS_THEN_KANJI` が唯一の登場箇所）。
- **非同期経路**: `WriteMechanism::ALL` を走査する（`open_chain.rs:35-61` のモジュール doc、
  `all_chain_record()` 173-179。実ログでも `chain_len=4`）。だが次の機構へ進むのは前の機構が
  `Failed` のときだけで（`actuation_chain.rs:211` `falls_through`）、`GjiDirect`/`MsImeDirect` が
  `Failed` を返すのは `mechanism_is_applicable` が偽のときだけ（`open_chain.rs:495-514`）。
  決定2 後は kind==MsIme なら `MsImeDirect` が必ず applicable、kind==Gji なら `GjiDirect` が
  必ず applicable。よって `KanjiToggle` の腕には二度と入らない。

**帰結**: ATOK 等の互換 TSF IME（`observer.rs:637`「GJI 非検出 — MS-IME（または互換 IME）と**推定**」）が
Standard プロファイル（Win32 Edit）で使われ、かつ ImmCross がタイムアウトした場合、
**今日は `VK_KANJI` が届いていたのが、変更後はフォールバックがゼロになる**。決定3 は
この退行を防いでいない。なお ATOK が `Imm32Unavailable`/`TsfNative` のアプリを使う場合は
今日すでに `[MsImeDirect]` = VK_IME_ON/OFF なので、退行面は「ATOK × Win32 Edit × ImmCross 失敗」に
限定される — 小さいが実在する。

**直し方（どれか1つを ADR で選ぶ）**

1. 決定3 の文言を「`KanjiToggle` を到達不能にする＝この組でフォールバックが無くなることを**受容する**」に
   書き換え、`残る限界` の当該行から「`KanjiToggle` を残す理由」を削る。加えて ATOK 実機で
   VK_IME_ON/OFF が効くかを確認する計画を書く（確認できるまでドラフトのままにする）。
2. 同じ PR で `KanjiToggleStrategy` / `WriteMechanism::KanjiToggle` /
   `MechanismCommand::PostKanjiToggle` / `ime::post_kanji_toggle_to_focused` /
   `architecture_guard.rs:1701-1720` のガードまで**削除**する。死んだ分岐を残すより
   `.claude/rules/complexity-budget.md`・「削除量で測る」方針に沿う。
3. ATOK を `MsIme` と推定しない（`ActiveImeKind` に第3値を入れる）—— ただしこれは
   ADR-184 の教訓（敵対的指摘に型/フィールドを積み増さない）に反するので推奨しない。

**併せて stale になる記述**（この ADR の PR で直すべき）: `ime_controller.rs:13-14`、`26-28`、
`153-172`（`KanjiToggleStrategy` の doc「実際に到達する組み合わせは1つだけ」）、
`open_chain.rs:434-437`（「これから送る `KanjiToggleStrategy` にとっては…」）、`465-468`、
`app_ime_policy.rs:60-62`、`focus/class_names.rs:200-204`（`uses_kanji_toggle` の doc）、
`tests/golden/ime_key_sequences.txt` の KanjiToggle 節、
`tests/architecture_guard.rs:1701`（「**生きている** `post_kanji_toggle_to_focused`」）。

### B2. 決定2 は打鍵ホットパスに**新しいブロッキング Win32 往復（ROMAN 補完）を追加する** —— ADR に記載が無く、a9 のログで実際に失敗している

`apply_mechanism` は先頭で `romaji_pre_write` を呼ぶ（`ime_controller.rs:239`）。発火条件は
`decide_needs_romaji_pre_write`（`ime_actuation_decision.rs:209-218`）:

```
open && mechanism ∈ {ImmCross, MsImeDirect} && kind == MsIme && belief != ObservedKana
```

**変更前**の fallback は `KanjiToggle` なので常に偽。**変更後**は `MsImeDirect` なので真になる。
`romaji_pre_write`（`ime_controller.rs:457-478`）は `ActuationTarget::capture_blocking` +
`set_ime_romaji_mode_for_target_blocking` = `SendMessageTimeoutW` ベースの**同期ブロッキング**で、
しかも `fallback_write` が `with_app`（`RUNTIME` の排他 borrow）を握ったまま呼ぶ
（`open_chain.rs:396-426` の doc「BUG-34 横展開 E-prep: 残存する同期ブロッキング（意図的に未解消）」が明記）。

実測（`res6/result-sc-kanji-msime-native-a9-2/dist/awase.log`）:

```
14:27:25.933979 fallback_write{open=true mechanism=MsImeDirect}: shadow_on=Some(true) → None で bypass
14:27:25.997168 …apply_mechanism{mechanism=MsImeDirect…}: [ime-io] cross_process cmd=0x0001 kind=probe … elapsed_us=62654
14:27:25.997269 …[imm-romaji] ROMAN 補完 Failed (mechanism=MsImeDirect)
14:27:25.997288 …[apply-ime] MS-IME direct: send 0x0016 (IME ON)
```

**VK_IME_ON の送信が 63ms 遅れ、しかも ROMAN 補完自体は失敗している。** 同じログの直前で
ImmCross は 150ms / 149ms でタイムアウトし `send_health` が
`slow IMM call: 156ms (連続1回目、2回連続でブレーカ作動)` を出している。つまり
**「IMM32 の往復が信頼できないと今まさに確認した直後に、IMM32 往復ベースの ROMAN 補完を
同期ブロッキングで追加する」**という自己矛盾した経路を新設することになる。最悪ケースは
`SendMessageTimeout` の上限ぶん（この環境で ~150ms）。

**直し方**: ADR に (a) この挙動差分を明記し、(b) どちらかを決める:
- `decide_needs_romaji_pre_write` の `MsImeDirect` 腕を「ImmCross 失敗後のフォールバック経路では
  行わない」に限定する（`DecisionSite` を見る、あるいは `fallback_write` 側で抑止する）、または
- そのまま受容し、`.claude/rules/tuning-constants.md` に倣って上記実測値（62.7ms、最悪 ~150ms）を
  コミット本文と ADR に残す。

（`GjiDirect` が同じ経路を通っても `decide_needs_romaji_pre_write` は偽なので、この負担は
MS-IME 側だけに新規に乗る。）

---

## Must-fix

### M1. 「同期チェーンと非同期チェーンの両方に効くか」の説明が実装と逆 —— 非同期はチェーン定数を**見ていない**

ADR line 67 は「非同期チェーン（`run_open_chain_async`→`fallback_write`）は機構ごとに
`is_applicable` を再評価するので、述語側の変更が必要」と書く。再評価は事実だが、非対称の本質は
**非同期経路は `caps(p,k).chain` を一切使わず `WriteMechanism::ALL` を渡し続ける**ことにある
（`open_chain.rs:35-61` のモジュール doc「なぜ Phase C でも chain が `WriteMechanism::ALL` の
ままなのか」、`all_chain_record()` 173-179、実ログの `chain_len=4`）。したがって:

- **決定1（チェーン定数）は同期経路にしか効かない**（`ImeController::apply` → `caps_chain_for` →
  `run_chain`、`ime_controller.rs:681-687`）。
- **決定2（述語）は両方に効く**。a9 が述語だけで直ったのはこのため。

さらに重要なのは、**2つはセットでしか入れられない**こと: 決定2 だけだと
`caps_chain_matches_legacy_all_scan`（`ime_controller.rs:1006-1023`）が落ちる（ALL 走査は
`[ImmCross, MsImeDirect]`、caps は `[ImmCross, KanjiToggle]` を返すため）。決定1 だけだと
非同期経路の挙動が変わらない。

→ 依頼文の問い「片方だけ直って片方に非冪等キー経路が残らないか」への答え: **残らない。ただし
それは経路が対称だからではなく、片側だけの変更が既存テストで落ちるから**。ADR の「影響範囲」節を
この形に書き直すこと（「両方が同じ `is_applicable`/チェーン定義を見る」という現行の記述は誤り）。

### M2. 決定2 は `ms_ime_direct_applicable` の `profile` 引数を完全に未使用にする

述語が `matches!(kind, ImeKindId::MsIme)` だけになると `profile` が未使用になり、`#[track_caller]`
（`key_sequence_policy.rs:57-59`、観測ログに真の呼び出し元を残すためのもの）の存在意義も消える。
引数を削ると `transport.rs:386` にも diff が出る＝ `.claude/rules/fix-requires-evidence.md` の
「物理 IME キーの Suppress/Allow 配送判断（BUG-46/52/116）」ファミリーのファイルに触ることになり、
`.githooks/pre-push` の警告対象になる。**ADR で「`_profile` として残すか、引数を削るか」を先に決め、
削るなら transport.rs への波及を影響範囲に書く**こと。

（判定結果自体が変わらないという ADR の主張は上の表のとおり検証済みで正しい。）

### M3. 実機の因果に、ADR が触れていない**第3のアクチュエーション**（drift correction）が並走している

`res6/result-sc-kanji-msime-native-a9-2/dist/awase.log`:

```
14:27:25.847454 WARN ir_apply_drift_correction: [drift] correction: observed=false ≠ desired=true for 117ms → set_ime_open(true)
14:27:25.847554 …[actuation-record] can_use_imm32_cross_process called from …\platform.rs:1250
14:27:25.997263 set_ime_open_cross_process{open=true}: set_ime_open_for_target: … open=true success=false send_elapsed=149ms
```

`ir_apply_drift_correction` → `Platform::set_ime_open`（`platform.rs:1246-1261`）は**チェーンを一切
通らない**。`can_use_imm32_cross_process()` が偽なら即 `false` を返すだけでフォールバックを持たず、
ImmCross が失敗しても代替機構へ落ちない。つまり ADR-190 は「ImmCross 失敗時の代替」をチェーン1本に
ついてだけ直し、同じ症状を生みうる `set_ime_open` 直呼びの経路は手つかずで残る。

`.claude/rules/fix-requires-evidence.md` の「IME actuation 合流点」行が要求する洗い出しそのものなので、
ADR の「影響範囲」に `runtime/ime_refresh.rs::ir_apply_drift_correction` → `platform.rs::set_ime_open`
を列挙し、**対象外なら対象外と明記**すること。

### M4. a8 の ALL PASS が意味するのは「awase が何も送らなくても物理 F2 が IME を開けた」—— a8 の却下理由も a9 の証拠力も、この事実を踏まえて書き直す必要がある

`PhysicalKeyDisposition::plan` の F2 分岐（`transport.rs:279-286`）は
`is_tsf_mode && f2_warmup_owned` のときだけ Suppress で、CI では `is_tsf_mode=false`
（`plan{profile=Standard shadow_toggled=false is_tsf_mode=false f2_warmup_owned=false …}` が
awase.log に出ている）なので **Allow**。実際 `14:27:25.714471 [reinject] vk=0xf2 down
(queued passthrough now firing)`、journal も `vk_code=242 … physical="Allow"`。

つまり ImmCross プロファイルでも**物理ひらがなキーは OS に届き、MS-IME 自身が IME を開く**。
（`plan` の doc `transport.rs:203-206`「KANJI 関連キー / ImmCross プロファイル: Down/Up 共に
Suppress（spurious 連鎖を構造的に遮断）」は、F2 の早期 return のせいで **F2 については成立して
いない**。BUG-116 で問題になった「表と実装の乖離」と同型なので、この doc も直すこと。）

帰結は2つ:

1. **ADR が a8 を却下した理由（「ImmCross が失敗したとき開く/閉じる手段が何も残らない」）は、
   少なくとも本シナリオでは成立しない。** 物理 F2 が既にその役を果たしており、だからこそ a8 は
   3/3 ALL PASS した。却下理由は「物理キーが無い経路（engine 起点の open、shadow-toggle OFF）では
   手段が無くなる」に限定して書き直すこと。
2. **a9 は「ImmCross プロファイルで VK_IME_ON が*閉じた* IME を開けること」を証明していない。**
   step1 で VK_IME_ON を送った時点で、物理 F2 により IME は既に開いていた可能性が高い
   （スパイクの +100ms 観測が `A(open=1 conv=0x19) … comp="き"`）。ADR の検証表の a9 行に
   この限界を書き、実機検証計画に「物理キーを伴わない open（engine 起点）で ImmCross を失敗させ、
   VK_IME_ON だけで開くか」を追加すること。

なお本質的には、この組は **物理 F2 も awase も両方 actuate する BUG-46 型の二重 actuation** であり、
ADR-190 は 2 本目を「同じ方向の冪等キー」にして衝突を見えなくする対症である（方向が一致するので
実害は消える、という論理）。それ自体は妥当だが、ADR にそう書いておかないと次の担当者が
同じ発見を繰り返す。

---

## Should-fix

### S1. より小さい/より根本的な案を「検討して採らなかった案」に追加する

M4 の裏返しとして、**Standard × MsIme では awase が actuate せず観測に追随する（ADR-186/187 の
follow 方式）** という案がある。機構を1つ**減らす**方向で `.claude/rules/complexity-budget.md`・
「削除量で測る」方針に最も合い、a8 の 3/3 ALL PASS がその実現可能性の直接証拠でもある。
採らないなら「engine 起点の open には対応する物理キーが無いので follow できない」等、
1行で理由を残すこと。

### S2. 決定4 は妥当だが、その根拠になっている `fallback_write` の doc を同じ PR で直す

ADR の指摘どおり `open_chain.rs:446-449` の「`imm_cross_write` の `Failed` は … 実際に確認した
場合だけ返る」は実装（`open_chain.rs:353-374`）と食い違う。この doc は本 PR 以降
**MsImeDirect が実際に通る場所の doc** になるので、放置すると誤読の害が増える。

同時に、依頼文の問い（BUG-113 / shadow_on bypass）への答えを ADR 本文に書いてよい:
`MsImeDirect` は `decide_attempt` で `shadow_on` を参照しない（`ime_actuation_decision.rs:277-282`、
無条件に `SendVk`）ので、`fallback_write` の `view.control.shadow_on = None` 上書き
（`open_chain.rs:473`）は MsImeDirect に対して無害。`architecture_guard.rs:2020` のコメントが
既にそう明言している。**AlreadyMatched / UnsafeToToggle / decide_gate も含め、MsImeDirect 側に
新しい危険は無い**（`decide_gate` の InputRelay 判定は機構に依らず `fallback_write` 冒頭
`open_chain.rs:480-494` で効く）。

### S3. Win キー押下中の挙動差分を1行書く

`MsImeDirect` は `send_ime_mode_key` が失敗（Win キー押下中）したとき `UnsafeToToggle` を返し
（`ime_controller.rs:330-341`）、`falls_through` が偽なのでチェーンはそこで止まる。変更前は
`KanjiToggle` が無条件に `post_kanji_toggle_to_focused` を送っていた（`apply_mechanism` の
`PostKanjiToggle` 腕 `ime_controller.rs:342-358`）。**「Win キー押下中は今まで VK_KANJI が飛んで
いたが、今後は何も飛ばない」**という差分が出る。安全側だが挙動差分なので ADR に記録。

### S4. 「既に開いている IME の conv（カタカナ等）が変わらないか」はコードから答えが出る

- `VK_IME_ON` 自体は open 軸のみ（`ime_controller.rs:124-140` の doc）。
- ROMAN 補完は `set_ime_romaji_mode_for_hwnd`（`ime.rs:786-805`）が `conv | IME_CMODE_ROMAN` の
  read-modify-write で、`IME_CMODE_KATAKANA` ビットを落とさない。
- かつ `belief_input_mode == ObservedKana` のときはそもそも発火しない
  （`ime_actuation_decision.rs:217`、かな入力ユーザー保護）。

→ 実機確認項目ではなく**既知事実**として ADR に書ける。実機で確認すべきは B2 のレイテンシの方。

### S5. a9 の FAIL を「CI のスパイク側タイマー遅延」と片付ける説明は裏取り不足（結論自体は保てる）

判定窓は `check_consistency.py:61-69` の `press < ms <= press + 2500`。step1 の押下から
最初の `k`（`vk_code=75 is_down=true`）までの実測:

| 構成 | step1 の k 遅延 | decision | step2 以降 |
|---|---|---|---|
| a9 sc-kanji run1 | +1.60s | Consume（ON） | ~+0.68s |
| a9 sc-kanji run2 | **+3.72s**（14:27:25.712 → 14:27:29.431） | **Consume（＝Engine ON）** | ~+0.68s |
| a9 sc-kanji run3 | **+4.63s**（14:28:36.483 → 14:28:41.111） | **Consume（＝Engine ON）** | ~+0.68s |
| a9 sc-dbe run1/2/3 | +1.48s / +1.14s / +1.64s | Consume | — |
| a8 sc-kanji run1 | +1.23s | Consume | ~+0.67s |

→ **ADR の結論（実 IME は正しく、窓を広げれば PASS になる）は正しい**。run2/run3 の k は
`Consume` = Engine ON であり、スパイク側も step1 の +100ms で `A(open=1 conv=0x19) … comp="き"` を
記録している（`res6/result-sc-kanji-msime-native-a9-2/dist/ime_key_matrix_spike.log` の
`[14:27:30.931Z] KEY [SCRIPT 1/10 …]`）。a9 が a8 より系統的に遅いという証拠も無い（a9 run1 は 1.60s）。

→ ただし**説明は書き直すべき**: 「step1 だけ常に +1.1〜1.6s かかる（step2 以降は +0.68s）＝
2500ms のマージンは元々薄く、時々 +3.7〜4.6s に跳ねる」というのが実態。したがって
**判定窓の拡大は「別途」ではなく本 PR の必須項目**（ADR 検証計画の該当行を「別途広げる」→
「本変更と同時に広げる」に）。

また `res6/result-sc-kanji-msime-native-a9-3/result.txt` の
`注意: outcome=Unwarranted が 2 件(判定には使わない)` は、起動直後
（`14:28:16.844799 on_ime_apply_complete{outcome=Unwarranted} seq=6 elapsed_ms=197`）の 2 行で
スクリプト手順と無関係。ADR が a9 を引くときに 1 行添えておくと後続の混乱を防げる。

### S6. 回帰テスト/ガードの更新漏れ（ADR「検証計画」に追加すべき全数）

ADR は `characterize_strategy` / `ms_ime_direct_applicable` / チェーン表の3つを挙げているが、
実際に落ちる/直すべきものは以下:

1. `crates/awase-windows/tests/golden/ime_key_sequences.txt`:
   `MS-IME	Standard	async_fallback	KanjiToggle` → `MsImeDirect`。加えて本文の
   「MsImeDirect (is_applicable: active_ime_kind==MicrosoftIme && !can_use_imm32_cross_process()):」
   の説明行と KanjiToggle 節の「稀にしか到達しない」。
2. `crates/awase-windows/tests/ime_key_sequence_golden.rs:211-215`
   （`assert_eq!(characterize_strategy(false, "Standard", true), "KanjiToggle")`）。
   **このファイルは `#![cfg(windows)]`（30 行目）で、`.github/workflows/ci.yml:324,335` のとおり
   Linux の test ジョブでは 0 tests。更新漏れは windows-build CI まで気付けない**ことを
   ADR に明記すること。
3. `crates/awase-windows/src/state/key_sequence_policy.rs:205-222` の 4 アサーション
   （Standard で偽になる前提）。
4. `crates/awase-windows/src/state/app_ime_policy.rs` の `caps_chains_match_the_adr089_table`、
   および定数名 `CHAIN_IMM_CROSS_THEN_KANJI` → `CHAIN_IMM_CROSS_THEN_MS_IME`。
5. `crates/awase-windows/src/ime_controller.rs:1006` `caps_chain_matches_legacy_all_scan`
   （M1 のとおり決定1・2 を同時に入れれば通る。片方だけだと落ちる＝安全網として機能している）。
6. `crates/awase-windows/tests/architecture_guard.rs:1701-1720` の doc（B1 参照）。
7. **ADR-089 §2.8 の表と、本 ADR が追記した「`ImmCross × MsIme` に `MsImeDirect` を入れない理由」
   節は今回*反転*する**。削除ではなく「2026-09-20、BUG-152 により覆した。当時の理由は実測ではなく
   実装の書き写しだった」と経緯を残すこと（`.claude/rules/experiment-logging.md` の趣旨。
   `WriteMechanism::may_return_failed` を使った INV-44 のガードは今回も生きるので、
   ADR-089 §4.9 の r3 の議論自体は無効にならない）。
8. ADR frontmatter の `status`「opus-adversarial-consult 未実施」の更新。

---

## 参考: 検証に使った主なログ行（再掲用）

ベースライン（`res2/result-sc-kanji-msime-native-1/dist/awase.log`、症状の因果）:

```
13:40:14.352984 [apply-ime] ImmCross failed (async, actual ime_on=None), falling through to next mechanism
13:40:14.353068 fallback_write: mechanism=GjiDirect not applicable → Failed
13:40:14.353124 fallback_write: mechanism=MsImeDirect not applicable → Failed
13:40:14.353192 [apply-ime] shadow=None … profile=Standard → desired=true: SendInput VK_KANJI
13:40:15.254958 [ime-fallback] SendInput VK_KANJI done: send_elapsed=901ms
```

→ result.txt の step1 は `open=0 … 期待OFF 実ON FAIL`。ADR の因果は正しい。
（なお `send_elapsed=901ms` は ADR が触れていない別の観測点。VK_KANJI の SendInput 自体が
このランナーで ~0.9s かかっている。）
