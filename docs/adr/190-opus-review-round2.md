# ADR-190 opus 敵対的レビュー round2（v3 = `ad041e1b` 対象）

対象 worktree: `/home/cuzic/rust-nicola-worktrees/ci-e2e-scenarios`（HEAD `ad041e1b`）。
行番号は全て同 HEAD の実コードで引き直した（前ラウンドで自分が書いた数値も再検証した）。

## 総評（依頼の点4への回答を先に）

**v3 は「型/フィールドを積み増して答える」の逆をやっており、方向は正しい。** 決定3 が
round1 B1 への回答として *追加* ではなく *削除* を選んだ結果、この PR の純減は
戦略1本 + `WriteMechanism` 1 variant + `MechanismCommand` 1 variant + `unsafe fn` 1本 +
dylint 許可エントリ1件 + architecture_guard テスト1本になる。さらに MF2（`FallbackSent`）を
採れば core クレートの `ImeOpenOutcome` も 1 variant 減る。
`feedback_dont_pile_complexity_in_response_to_adversarial_review` の轍は踏んでいない。

逆行しているのは 1 点だけ: **決定2 で未使用の `profile` 引数と `#[track_caller]` を温存する**
判断（M2' 参照）。理由（transport.rs に差分を出さない）は理解できるが、削除量では負ける。

一方、**決定3 の撤去範囲は網羅されていない**。ADR が挙げていない実体が最低 6 件あり、
うち 3 件（`lints/actuation_call_guard`、`ImeOpenOutcome::FallbackSent`、
`MAX_WRITE_MECHANISMS` とそのフィクスチャ）は「書き忘れ」ではなく設計判断を要する。
以下を Must-fix とする。

---

## 点3: round1 の指摘への対応が意図を満たしているか

| round1 | v3 の対応 | 判定 |
|---|---|---|
| B1（KanjiToggle は残しても保険にならない） | 決定3 で撤去に変更 | **満たす**。ただし B1' 参照 |
| B2（ROMAN 補完の新設ブロッキング） | 決定5 で実測付き受容 | **満たす**。ただし数値に誤り（B2' 参照） |
| M1（決定1・2 の効き先が逆） | 決定2 の中で訂正 | **満たす** |
| M2（`profile` 引数が未使用化） | `_profile` として温存 | 満たすが逆行（M2' 参照） |
| M3（drift 補正が対象外） | 「影響範囲と対象外」節に明記 | **満たす** |
| M4（a8/a9 の証拠の限界、物理 F2 Allow） | 原因4・検証表の「限界」列・採らなかった案に反映 | **満たす。よく書けている** |
| S1（follow 方式） | 採らなかった案に理由付きで追加 | **満たす** |
| S2（`fallback_write` の doc） | 決定4 に「実装に合わせて直す」 | **満たす** |
| S3（Win キー押下中の差分） | 影響範囲節に追加 | **満たす** |
| S4（conv は壊れない＝既知事実） | 専用節に移動 | **満たす** |
| S5（判定窓） | 決定6 で「別途にしない」 | **満たす** |
| S6（更新するもの全数） | 「更新するもの」節 | 部分的（点1 の MF5/MF9 が欠落） |

### B1'. 撤去を選んだ以上、「ユーザー判断が誤りだった場合の検出口」を1行残すこと

v3 は `残る限界` から ATOK 行を削った。判断としては一貫しているが、
`VK_IME_ON/OFF` がどんな IME でも効くという前提が外れたときの症状は
「Standard プロファイルで ImmCross が失敗したときだけ IME が開かない（Engine だけ ON）」
＝**今回直そうとしているバグと見分けがつかない形**で再来する。
BUG-152 か ADR-190 の「残る限界」に、
「この前提が外れた場合、症状は BUG-152 と同型で、ログ上の差は
`[apply-ime] MS-IME direct: send 0x0016` の後に実 IME が開かないこと」
と 1 行書いておくこと（`.claude/rules/experiment-logging.md` が求める「なぜ前回それを捨てたのか」の逆向き＝
「なぜ今回それを採ったのか」の検証可能な形）。

### B2'. 決定5 の「最悪でも `SendMessageTimeout` の上限(~150ms)」は不正確

ROMAN 補完の実際の往復とタイムアウトは `SendMessageTimeout` の 150ms ではない:

- `ActuationTarget::capture_blocking`（`ime.rs:1141-1145`）→ `get_focused_hwnd()`
  （`GetGUIThreadInfo` 30ms + `GetForegroundWindow` フォールバック、`ime_controller.rs:449-452` の doc）
- `set_ime_romaji_mode_for_hwnd`（`ime.rs:786-805`）→ `get_ime_wnd(hwnd)` →
  `modify_conv_mode`（`ime.rs:472-498`）が **`probe_ime_control(GetConversionMode, 50ms)`** と、
  値が変わる場合だけ **`actuate_ime_control(SetConversionMode, 50ms)`** の**2往復**

つまり最悪は **30ms + 50ms + 50ms ≒ 130ms、最大3往復**。a9 の実測
`elapsed_us=62654`（`res6/result-sc-kanji-msime-native-a9-2/dist/awase.log`
`14:27:25.997168`、cmd=0x0001）は **50ms タイムアウトを超えて `None` が返り、
SET 側が走らないまま `ROMAN 補完 Failed` になった**ケースであり、
「62.7ms 遅れた」と「最悪 150ms」は両方とも書き換えが要る。
正: 「実測 62.7ms（GET が 50ms タイムアウト）。最悪は 30ms + 50ms×2 ≒ 130ms、往復3回」。

### M2'. `_profile` + `#[track_caller]` の温存は削除量で負ける

決定2 で `ms_ime_direct_applicable(kind, _profile)` にすると:

- `#[track_caller]`（`key_sequence_policy.rs:57-59`）は**完全に無意味になる**。
  この属性は `AppImeProfile::can_use_imm32_cross_process` が
  `#[actuation_choke_point_macro::actuation_choke_point(...)]` で
  `std::panic::Location::caller()` を記録するために付いていた（`class_names.rs:183-190`）。
  述語が `can_use_imm32_cross_process` を呼ばなくなれば、この属性が伝える先が無い。
  **ADR が「`#[track_caller]` も残す」と明記しているのは誤り**。消すこと。
- 副作用として `[actuation-record] can_use_imm32_cross_process called from …\ime_controller.rs:146`
  という観測ログが消え、`can_use_imm32_cross_process` の `callers = "17+ call sites"` 棚卸しが
  1件減る。これは ADR-158 の棚卸し対象なので、ADR にその旨1行。
- `_profile` 自体の温存理由（transport.rs に差分を出さない）は妥当だが、
  `.githooks/pre-push` は**ブロックせず警告するだけ**（`.claude/rules/fix-requires-evidence.md`
  「自動チェック（pre-push）」節）。引数ごと消して transport.rs:386 を
  `ms_ime_direct_applicable(kind)` にする方が削除量では上。どちらを採るにせよ、
  「pre-push は警告のみなので差分を出すこと自体はコストではない」を判断材料として書くこと。

---

## 点1: 撤去範囲の漏れ（Must-fix）

ADR は「約27ファイル」と書くが、実測は次のとおりで、この数字は内訳を示さないと誤解を招く。

```
KanjiToggle | post_kanji_toggle | kanji_toggle を含むファイル: 75
  crates/ 配下 .rs : 29   lints/ : 1   docs/ + CLAUDE.md + .claude/rules : 45
  うち「コード実体の変更が要る」ファイル: 13 前後、残りは doc コメントのみ
```

### MF1. `lints/actuation_call_guard/src/lib.rs:77` が ADR に無い

`RESTRICTED_CALLS` の `send_input_safe` の許可呼び出し元リストに
`"post_kanji_toggle_to_focused"` が入っている（19件のうちの1件）。関数を消すなら
このエントリも消す。**これは `.claude/rules/complexity-budget.md` が定義する
「actuation 合流点の許可リスト」そのもの＝ 1-in-1-out の *1-out* 実績**になるので、
ADR に明記した方が得（複雑性予算制は未発効だが、削除の方向は望ましいと同ルールが書いている）。

### MF2. `ImeOpenOutcome::FallbackSent` の**唯一の生成元**が消える（ADR に記載なし）

`FallbackSent` を返すのは `ime_controller.rs:357`（`MechanismCommand::PostKanjiToggle` の腕）だけ。
撤去後は**到達不能な variant**がコア `awase` クレートに残る。これは B1 で指摘した
「到達不能なまま残すと保険に見えて死んでいる」と同型の問題。波及先（全て実コードで確認）:

- コア: `src/platform.rs:159`(定義)、`:163`、`:198`、`:205`(`wrote_open_state`)、`:218`、
  `:252`、`:552`、`:581`(テスト)
- `crates/awase-windows/src/platform.rs:1482, 1500, 1552`
- `runtime/message_handlers.rs:740, 762`（**`ImeOpenOutcome` ↔ u8 のワイヤ符号化**。
  variant を消すと番号が詰まる。プロセス内メッセージなので互換性問題は無いが、
  `2100` のテストと合わせて直すこと）
- `state/ime_event.rs:620`、`state/platform_state.rs:1083`、`runtime/executor.rs:1107`、
  `journal.rs:757`、`state/gji_direct_mechanism.rs:242, 281`、
  `state/actuation_chain.rs:179`(may_return_failed の表), `:667`(ALL_OUTCOMES), `:787`, `:790`

**決めること**: (a) 一緒に消す（削除量で最も望ましいが diff が広がる）／
(b) 残して「到達不能だが `ImeOpenOutcome` の意味論上の区別として残す」と書く。
どちらでもよいが、**ADR が言及しないまま実装に入るのが一番まずい**。

### MF3. `MAX_WRITE_MECHANISMS`（=4）とその境界テストのフィクスチャ

`state/actuation_decision_record.rs:56-57`:
```rust
/// ADR-163 D2: `WriteMechanism::ALL`と同じ最大attempt数。
pub const MAX_WRITE_MECHANISMS: usize = 4;
```
doc が `ALL` との一致を宣言しているので 3 へ。そのとき同ファイルの
`deserialize_rejects_chain_longer_than_max_write_mechanisms`（`:719-727`）のフィクスチャ

```json
"chain": ["ImmCross","GjiDirect","MsImeDirect","KanjiToggle","ImmCross"]
```

は **`KanjiToggle` が未知 variant になるため「長さ超過」ではなく「デシリアライズ不能」で
err になり、`assert!(result.is_err())` は通るが検査したい性質を検査しなくなる**（恒真化）。
`["ImmCross","GjiDirect","MsImeDirect","ImmCross"]`（4件 > 3）に書き換えること。
同ファイル `:752` 付近のコメント「残り3スロットはnull」も 2 へ。

### MF4. `state/ime_profile_driver.rs:315-360` は「doc 言及」ではなく**不変条件テスト**

`invariant_2_kanji_owning_profiles_do_not_lead_with_kanji_toggle` は
`assert_ne!(chain.first(), Some(&WriteMechanism::KanjiToggle), …)` を実行する。
variant を消すと**コンパイル不能**になり、削除するしかない。
このテストの doc（`:319-327`）は「旧版 `invariant_2_kanji_owning_drivers_use_non_kanji_mechanism`
は恒真だったので ADR-090 決定 F-2' で作り直した」と明記しており、**削除は恒真時代へ戻すこと**になる。
ADR に「削除する」と明記し、併せて
「非冪等機構が存在しないことは `WriteMechanism` の variant 集合そのものが固定する
（＝テストではなく型で保証される）」と置き換えの根拠を書くこと。
なお同テストの `:334-336` の doc（「`(ImmCross, MsIme)` の chain は `[ImmCross, KanjiToggle]`」）も
決定1 で嘘になる。

### MF5. `state/actuation_chain.rs` のユニットテスト4本が ADR の更新リストに**1本も入っていない**（Linux で走る）

| テスト | 行 | 撤去の影響 |
|---|---|---|
| `unsafe_to_toggle_stops_the_chain_before_kanji_toggle` | `:753-762` | 名前・chain が `KanjiToggle` 前提。MF6 と合わせて意味ごと再定義が必要 |
| `inapplicable_mechanisms_are_skipped` | `:783-791` | `KanjiToggle` + `FallbackSent` を使う |
| `all_failed_yields_failed` | `:806-814` | `[ImeOpenOutcome::Failed; 4]` と `writer.calls.len() == 4` → 3 |
| `mechanism_names_match_strategy_names` | `:930-937` | `["ImmCrossProcess","GjiDirect","MsImeDirect","KanjiToggle"]` → 3件 |

加えて `ALL_OUTCOMES`（`:665-672`）から `FallbackSent` を外すかは MF2 次第。

### MF6. `UnsafeToToggle` を `falls_through` に含めない**理由**が消える（設計上いちばん危ない置き忘れ）

3 箇所が同じ理由を書いている:
- `state/actuation_chain.rs:205-210`（`falls_through` の doc）
  「**`UnsafeToToggle` を含めてはならない。** … ここでフォールスルーさせると
  **Win キー押下中に非冪等な `VK_KANJI` を送る新経路**が生まれる」
- `state/actuation_chain.rs:173-196`（`may_return_failed` の表と INV-44 の説明）
- `state/app_ime_policy.rs:44-50`（`Caps::chain` の doc「`GjiDirect`/`MsImeDirect` の後ろに
  `KanjiToggle` を置かないこと」）

撤去後は**守るべき対象（非冪等キー）が存在しなくなる**ので、理由をそのまま残すと
将来「非冪等キーはもう無いから `UnsafeToToggle` もフォールスルーさせてよい」と読まれる。
規則自体は維持すべき（`UnsafeToToggle` = そもそも送れていない、で二重送信の話ではない）なので、
**理由を「`UnsafeToToggle` は `applied_snapshot` をラッチさせないための未適用シグナルであり、
次の機構へ進む根拠にならない」へ書き換える**こと。BUG-16 追補（`ime_controller.rs:330-341`）が
その本来の根拠。

なお INV-44 のガード `caps_chains_have_no_unreachable_trailing_element`
（`app_ime_policy.rs:368-392`）自体は撤去後も成立する（全 chain の非末尾要素は `ImmCross` のみ、
`may_return_failed` は撤去後も `ImmCross` だけが真）。

### MF7. `architecture_guard.rs` の該当箇所は「件数ガード」ではない

ADR は「`raw_mechanism_write_sites_are_confined_to_chain_writers` 等の**件数**」と書くが、
実際に壊れるのは件数ではなく**宣言の存在検査**:

- `architecture_guard.rs:2438-2447`: `for decl in [… "struct KanjiToggleStrategy"]` が
  `unwrap_or_else(|| panic!("`{decl}` の宣言が `ime_controller.rs` に見つかりません"))` するので、
  struct を消すと**このテストが panic で落ちる**。配列から 1 要素を除く修正。
  `apply_mechanism(` の件数（`:2391-2400` の 2 箇所）と `fallback_write(` の件数は**不変**。
- `kanji_toggle_fallback_sends_expected_vk_codes`（`:1702-1721`）は**テストごと撤去**
  （`post_kanji_toggle_to_focused` の本体を `extract_fn_body` で探すので、消すと落ちる）。

### MF8. 「残すもの」に `tests/e2e_windows.rs` を明記すること

`e2e_gji_vk_kanji_toggle_hazard_interactive`（`:1812`）/
`e2e_msime_vk_kanji_toggle_hazard_interactive`（`:1923`）は `WriteMechanism` を使わず、
Edit コントロールへ生の VK_KANJI (0x19) を送って「VK_KANJI は冪等でない」ことを実機で示す。
**撤去後、これが `VK_KANJI` を非冪等と結論した唯一の実行可能な証拠になる。**
掃除で巻き込まれないよう「残すもの」に書くこと（現在は ADR に一切出てこない）。

### MF9. doc は「言及の差し替え」で済まないものが 2 本ある

- `docs/ime-control-overview.md`: 図（`:35`）と戦略リスト（`:188-190`）に加え、
  **独立した節「`KanjiToggleStrategy` の confidence gate と 300ms ウィンドウ」（`:244`〜）**。
  これは `output/ime_apply_planner.rs:53, 84` の `safely_confirmed` ロジックの説明で、
  ロジック自体は残る（しかも「KanjiToggle 系（Chrome/TsfNative 等）」という呼称は
  今日すでに誤り＝ Chrome/TsfNative は GjiDirect/MsImeDirect）。**節ごと書き直す**対象。
- `docs/workarounds.md`: **独立項目「4-1. `post_kanji_toggle_to_focused`（VK_KANJI フォールバック）」
  （`:116`〜）** に加え `:194`、`:218`。

また、`docs/known-bugs/BUG-110.md` / `BUG-113.md` と過去 ADR 群
（033/034/044/063/070/081/087/088/089/090/095/097/098/108/114/117/121/130/133/135/138/139/153/
158/159/160/163/167/168/171）は**履歴なので触らない**ことを、`docs/experiments.md` と同じ扱いで
明記すること（現在の ADR は experiments.md だけを例外にしている）。

### MF10. `ImeKeyKind::KanjiToggle` との取り違え防止（ADR の「残すもの」を具体化）

実体は 3 箇所:
- `vk.rs:96`（variant 定義）、`:127`（`0x19 => Some(Self::KanjiToggle)`）、
  `:149`（`Self::KanjiToggle => ShadowImeEffect::Toggle`）
- `crates/awase-settings/src/main.rs:4846`（設定 GUI の doc。**この PR では無変更**）
- `win32.rs:222-223`: 「`send_ime_mode_key`（VK_IME_ON/OFF）と
  `post_kanji_toggle_to_focused`（KanjiToggle の VK_KANJI=0x19）の両方が同じマーカーを使うため
  **区別できない**」——撤去後は送信元が 1 つになるので、この制約自体が解消する。
  「直す doc」ではなく「**制約が消える**」として書けるので、削除量の実績に数えられる。

---

## 点2: 撤去前後の挙動（全組、同期/非同期）

前提（実コードで確認）: `ImePolicyProfile::from(AppImeProfile::InputRelay) == ImmCross`
（`class_names.rs:540-543` のテスト）、`decide_gate` は `InputRelay` のみ `NotOwned`
（`ime_actuation_decision.rs:125-131`）、`run_chain` は適用可能機構ゼロ/全 `Failed` で
`ImeOpenOutcome::Failed`（`actuation_chain.rs:519-540`、`empty_chain_yields_failed`）。

### 同期チェーン（`ImeController::apply` → `caps_chain_for` → `run_chain`）

| profile × kind | 撤去前 chain | v3 後 chain | ImmCross 失敗後に走る機構 | 差分 |
|---|---|---|---|---|
| Standard(ImmCross) × MsIme | `[ImmCross, KanjiToggle]` | `[ImmCross, MsImeDirect]` | VK_KANJI → **VK_IME_ON/OFF** | **意図どおり変わる**（本 ADR の目的） |
| Standard × Gji | `[ImmCross, GjiDirect]` | 同左 | GjiDirect | 不変 |
| Imm32Unavailable × MsIme | `[MsImeDirect]` | 同左 | — | 不変 |
| Imm32Unavailable × Gji | `[GjiDirect]` | 同左 | — | 不変 |
| TsfNative × MsIme | `[MsImeDirect]` | 同左 | — | 不変 |
| TsfNative × Gji | `[GjiDirect]` | 同左 | — | 不変 |
| InputRelay（→ ImmCross 行） | `[ImmCross, KanjiToggle]` | `[ImmCross, MsImeDirect]` | — | **到達しない**（`apply` 冒頭の `decide_gate` が `NotOwned` を返し `caps_chain_for` の前に return、`ime_controller.rs:628-646`） |
| Plain / Unknown | ImmCross 行と同一 | 同一 | — | 構造的に到達不能（ADR-089 §1.3(e)）。`plain_and_unknown_caps_are_identical_to_imm_cross` が維持される |

### 非同期チェーン（`run_open_chain_async` → `WriteMechanism::ALL` 走査 → `fallback_write`）

`ALL` は `[ImmCross, GjiDirect, MsImeDirect]`（4→3）になる。

| profile × kind | 撤去前の走査 | v3 後の走査 | 差分 |
|---|---|---|---|
| Standard × MsIme | ImmCross(Failed) → GjiDirect(不適用=Failed) → MsImeDirect(**不適用=Failed**) → KanjiToggle(送信) | ImmCross(Failed) → GjiDirect(不適用=Failed) → **MsImeDirect(送信)** | **意図どおり**。attempts 3件（実ログの `attempts_len=3` と一致） |
| Standard × Gji | ImmCross(Failed) → GjiDirect(送信/AlreadyMatched) | 同左 | 不変（KanjiToggle には元から到達しない） |
| Imm32Unavailable × MsIme | ImmCross(不適用) → GjiDirect(不適用) → MsImeDirect(送信) | 同左 | 不変 |
| Imm32Unavailable × Gji | ImmCross(不適用) → GjiDirect(送信) | 同左 | 不変 |
| TsfNative × 両 | 同上 | 同左 | 不変 |
| InputRelay × 両 | 最初の機構の `fallback_write` 冒頭で `NotOwned`（`falls_through` 偽 → 停止、`open_chain.rs:480-494`）。`run_open_chain_async` 冒頭の gate も同じ | 同左 | 不変 |

**終端の扱い**: 撤去後も全 (p,k) で少なくとも 1 機構が applicable になる
（`GjiDirect ⟺ kind==Gji`、`MsImeDirect ⟺ kind==MsIme`、`ImeKindId` は 2 値）。
したがって `Failed` で終端するのは `with_app` が全機構で `None` を返す場合だけで
（`fallback_write` の `unwrap_or_else` → `(Failed, None)`、`open_chain.rs:528-533`）、
撤去前は 4 回、撤去後は 3 回試して `Failed` を返す——**帰結は同じ**（`run_chain` が
使い切って `Failed`）。`attempts` の上限も 3 で足りる（MF3 と整合）。

**`may_return_failed` の前提**: 撤去後も真なのは `ImmCross` だけで、INV-44 は成立する。
ただし**撤去前から存在する緩み**として、`fallback_write` は「適用不能」「`with_app` が None」で
`ImeOpenOutcome::Failed` を返すため、非同期経路では `GjiDirect`/`MsImeDirect` も実際には
`Failed` を返す（`open_chain.rs:31-33` の doc が「適用不能なら `Failed` を返す形にしているが、
`Failed` は必ずフォールスルーするため走査結果は同一」と説明済み）。撤去後は
**その `Failed` の落ちる先が無くなる**（末尾要素になるため）ので、
「MsImeDirect が適用不能 かつ GjiDirect も適用不能」が起きれば無音で `Failed` になる。
`ImeKindId` が 2 値である限り起きないが、**この結論は `ImeKindId` の variant 数に依存している**ので、
ADR にその依存を1行明記すること（将来 `ImeKindId` に 3 値目を足す人への tripwire）。

**`imm_cross_is_first_applicable`（`ime_controller.rs:713-717`）は全組で不変**:
`chain.first() == Some(&ImmCross)` の短絡があるため、Imm32Unavailable/TsfNative は撤去前後とも
`false`。Standard×MsIme は撤去前後とも `Some(0)` → `true`。InputRelay は
ImmCross が不適用で `position == Some(1)` → 撤去前後とも `false`。

---

## 点3 の続き: v3 が新たに入れた数値・行番号の裏取り

| ADR の記述 | 検証 |
|---|---|
| `open_chain.rs:353-374`（`Failed` 腕） | 正（round1 の指摘を正しく反映） |
| `fallback_write` の doc `:446-449` | 正 |
| `app_ime_policy.rs:65` の `CHAIN_IMM_CROSS_THEN_KANJI` | 正 |
| `key_sequence_policy.rs:60` / `:205-222` | 正 |
| `transport.rs:279`（F2 分岐） | **ずれ**。`:279` は `plan` の doc コメント中。実際の分岐は **`transport.rs:292-298`**（`if event.vk_code == crate::vk::VK_DBE_HIRAGANA {`） |
| `transport.rs:386` | 正 |
| `ime_controller.rs:1006`（`caps_chain_matches_legacy_all_scan`） | 正 |
| `ime_key_sequence_golden.rs:211-215` / `#![cfg(windows)]` | 正（`#![cfg(windows)]` は 30 行目、`ci.yml:324,335` が Linux で 0 tests と明記） |
| `platform.rs:1302`（`uses_kanji_toggle`） | 正 |
| `focus/tracker.rs:204` / `key_pipeline.rs:1682-1689` / `focus_tracking.rs:1043` | 正。ただし `focus_tracking.rs:1043` は**単なる言及ではなく根拠**（「chain が `KanjiToggle` を含むため、この pre-sync は Standard でも引き続き必要」）。撤去後、pre-sync がまだ必要かを**再導出**してから文言を直すこと（文言だけ直すと根拠を失った処理が残る） |
| `chain_len=4`（実ログ） | 正（`a9-2` `14:27:26.614171`） |
| 実測 62.7ms / `ROMAN 補完 Failed` | 正（ただし B2' の解釈訂正が要る） |
| step1 の k 遅延 +1.1〜1.6s、+3.72s/+4.63s、`decision=Consume` | 正 |
| `Unwarranted` 2件は起動直後（`elapsed_ms=197`） | 正（`a9-3` `14:28:16.844799`） |
| 「2回目以降は成功する（`sc-hz-msime-native-suppress` step5: `success=true send_elapsed=12ms`）」 | 値は実在（`res2/result-sc-hz-msime-native-suppress-1/dist/awase.log:872`、`open=false` の set-open）。ただし**その行が「step5」かは確認できていない**。ADR にはログのパス+行番号で書く方が安全 |
| `tests/journals/` に `KanjiToggle` が含まれていないことを「実装前に確認する」 | **今回確認済み**。`grep -rl KanjiToggle --include=*.json`（target 除く）は 0 件。`WriteMechanism` が載るのは `crates/awase-windows/tests/journals/ime_apply/adr108-focus-crossing-success.json` のみで `ImmCross`/`MsImeDirect` だけ。ADR は TODO ではなく**確認済み**と書いてよい |

### 追加（S6 の補足）: 実ユーザーの bug report は再生できなくなる

`WriteMechanism` は `serde::Serialize/Deserialize` を derive している
（`actuation_chain.rs:130`）。`ActuationDecisionRecord` は ADR-095 の bug report
`journal_json` に相乗りする（`ime_actuation_decision.rs:32-45` の警告）。
**撤去後、撤去前に収集された report で `"KanjiToggle"` を含むものは
デシリアライズできなくなる**（現在のコーパス
`tests/journals/actuation_decision/bug-131-report-*.json` には無いので実害は無い）。
ADR-163 の再生ハーネスの前提に関わるので、「旧 report の再生は諦める」か
「`#[serde(other)]` 相当の受け口を置く（＝型を足すので今回は採らない）」かを1行書くこと。

---

## まとめ（実装前に ADR へ反映すべき最小セット）

1. 決定3 の撤去範囲に **MF1（dylint 許可エントリ）・MF2（`FallbackSent`）・MF3（`MAX_WRITE_MECHANISMS` と
   境界テストのフィクスチャ）・MF4（`ime_profile_driver` の不変条件テストは削除になる）・
   MF5（`actuation_chain.rs` のユニットテスト4本）** を追加する。
2. **MF6**（`UnsafeToToggle` を `falls_through` に含めない理由の書き換え）を決定として明記する。
   ここだけは「消し忘れ」ではなく「消したあとに残る規則の根拠の付け替え」で、
   放置すると次の変更が規則ごと外しにくる。
3. **MF7/MF8/MF9/MF10** で「壊れ方」「残すもの」「書き直す節」を正確にする。
4. **B2'**（130ms・3往復への訂正）、**M2'**（`#[track_caller]` は残さない）、
   **`transport.rs:292`**（行番号）、**`sc-hz` の 12ms はログのパス+行で引用**、
   **`tests/journals` は確認済み**、を反映する。
5. 点2 の表を ADR に入れ、**「KanjiToggle が不要である結論は `ImeKindId` が 2 値であることに
   依存する」**を tripwire として1行残す。
