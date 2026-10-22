# 撤去候補の到達プローブ結果(DD1-02・DD2-02・DD5-02)

2026-10-22。`INDEX-dd.md` の撤去候補 3 件を、GitHub の Windows CI(`e2e-ime.yml`)に目印ログを入れた版で回して数えた。
**「0 件」は証明ではなく証拠である**(e2e の構成が触れない経路は、0 件でも到達しないとは言えない)。

## 方法

- プローブ: ブランチ `ci/probe-reach`(`aa9263de`、挙動不変、`tracing::info!` の `[reach-probe]` だけ。ユーザーの入力内容は出さない)。
- 撤去版: `ci/probe-remove-dd1-02`(`ad53597b`)・`ci/probe-remove-dd5-02`(`728af885`)。いずれも PR にしない比較用。
- 集計: 各ジョブの artifact `result-*/dist/awase.log`(ジョブログではなく artifact に全ログがある)を `grep "reach-probe"`。
  ジョブログ(`gh api .../jobs/<id>/logs`)には awase のログが出ない。`awase-filtered.log` は絞り込み済みで目印が無い。

## 回した run

| run ID | ref | 構成(only) | 結果 |
|---|---|---|---|
| 39734755114 | probe-reach(旧版) | sc-focusrestore/follow/driftrecovery/reopen/startup/solotap/adr/late/eisu-km/hz/shift/kanji/dbe + baseline(219 ジョブ) | success。DD5-02 に relevant 情報なし |
| 39740746118 | probe-reach | 同上(219 ジョブ) | success |
| 39737904489 | probe-reach | cal-drift*・atok-*・msime-*・sc-offrca-*・sc-bug*・sc-t1*・sc-p2-*・sc-settle-*・ts-edit-*(208) | success |
| 39743669079 | probe-reach | ts-raw-*-gji-idle/longidle/cold・ts-tsf-*・tsx-chromepage/chromebar(233) | success |
| 39750903017 | probe-reach | sc-dbe/kanji/hz/shift/eisu-km(68 件の artifact) | success |
| 39743410658 | remove-dd5-02 | 39740746118 と同じ(219) | success |
| 39750899100 | remove-dd5-02 | 39750903017 と同じ(71) | success |
| 39743415340 | remove-dd1-02 | 39740746118 と同じ(219) | success |

## 目印の集計(39740746118・39737904489・39743669079・39734755114 の awase.log の合計。再掲は最初の run を除く別 run の数)

| 目印 | 件数 | 読み |
|---|---|---|
| `DD1-02 claim source=Shadow result=fresh` | 約 6500(39740746118: 2028、39737904489: 2088、39743669079: 395 ほか) | shadow 側の claim は全部 `fresh` |
| `DD1-02 claim source=Engine result=fresh` | 40(39740746118)・552(39737904489) | Engine 側も全部 `fresh` |
| `ClaimDeclined`・`duplicate`・`conflict_kept`・`conflict_engine_wins` | **0** | 同じ押下で 2 つ目の経路が来た回は 1 度も無い |
| `DD2-02 observe source=FocusCheck fence_match=true` | 13(39740746118)・12(39734755114) | 不一致 0 |
| `DD2-02 observe source=IdleCheck` | **0**(どの run でも) | **経路に到達していない。判断材料にならない** |
| `DD5-02 A1 set` | 215(39740746118)・39(39737904489)・941(39743669079) | |
| `applied-read-between-A1-A2 via=applied_known` | A1 と同数。すべて `known=true`。`relevant=true` は **0** | 読まれるが、判断を変える条件(打鍵中 かつ 意図あり かつ skip_imm でない)は一度も満たされなかった。`intent=false` が全件 |
| `applied-read-between-A1-A2 via=applied_state`(他の読み手) | 0 | `ImeModel::applied_state()` 経由の読みは A1〜A2 の間に無かった |
| `A2 closed effective_open=false` | 48(39740746118)・73(39743669079) | A1 が `true` を書いた後、A2 が `false` で上書きした回がある(A1 と A2 は別の値になりうる) |

## 候補ごとの判断

### DD1-02(shadow が後から来る分岐)
- 6500 件の shadow の claim と約 600 件の Engine の claim のうち、`Declined/Duplicate/Conflict*` は 0。
  同じ押下で Engine と shadow の両方が claim した回自体が無い(P2 `engine_owns_open_key` が先に止めている推定と一致)。
- 撤去版(`kp_shadow_actuate` の `ClaimDeclined` 分岐を外す)で同じ 219 構成を回した(39743415340)。判定の差は、同じコードを 2 回回しただけでも出る不安定な構成
  (`sc-startup-msime-chrome-on-norefocus2-*`・`precheck-*`・`gated-*`・`sc-driftrecovery-ctrlmuhenkan-gji-tsf-1` の invariants)に限られた。
- **撤去の判断**: e2e では撤去前後で差が無い。ただし台帳の衝突分岐は `explicit_press_exhaustive.rs` の `p5_ledger`(P1 単独でも違反 0)の前提になっているので、
  分岐を撤去するなら**テストの性質(P2 無しでも P1 が守る)を捨てる**判断が要る。e2e の 0 件では押せない。実利用の journal の `PressWriteClaim` 集計(調査の元の提案)を待つ。PR は作らない。

### DD2-02(`ConvModeMgr::observe` の fence 不一致)
- `FocusCheck` は 13 回呼ばれ、すべて一致。`IdleCheck` は**どの構成でも 1 回も呼ばれなかった**(`idle-conv-check` の spawn が、TsfNative + 打鍵停止 500ms 超 + 明示操作から 1500ms 超の条件に当たる e2e 構成が見つからなかった。`ts-tsf-*`・Chrome 系・sc-follow-* を含めて 0)。
- コードの読みは調査と同じ(`admit_epoch_in_app` が同じ `with_app` の同期フレーム内で `accepted.fence` を作り、`apply_idle_conv_check` に渡す)。ただしそれは到達の証拠ではなく読み。
- **判断: 未確認。撤去しない。** IdleCheck に到達する構成(実機の WezTerm 等、または新しい構成)が要る。

### DD5-02(フォーカス時の `presync_applied_open_on`)
- A1〜A2 の間に `applied` は読まれる(`ReadStrategyFacts.applied_known`、A1 のたびに 1 回、`known=true`)。**調査の「読まれるなら先同期は要る」の基準に従うと、読まれたので要る**。
  ただし、その読みが判断を変える条件(`explicit_verify` に効く: 打鍵中 かつ 意図あり)は 1195 回中 0 回で、`intent=false`(フォーカス変更で意図が消えている)が全件。
- 撤去版(A1 の呼び出しを外す)で 219 構成(39743410658)と 71 構成(39750899100)を回した。
  1 回目に `sc-dbe-gji-msime-drv-3` で step 1(最初のひらがな)が「Engine が追随しない」で FAIL(同構成は撤去前 6 回・撤去後の 2 回目の 3 回で再現せず)。他の差は不安定な構成のみ。
  単発で、再現しない。原因は未確認。
- **判断: 撤去を勧めない(未確認が残る)。** 撤去するなら、`architecture_guard.rs` の `applied_mirror_on_focus_goes_through_the_plan_at_all_three_sites`・`startup_placeholder_desired_is_not_treated_as_intent`(`presync_applied_open_on` の呼び出しが 1 箇所と数える)も書き換える必要があり、効果(約 8 行)に比べて大きい。PR は作らない。

## 未確認

- IdleCheck の観測(DD2-02)。
- DD5-02 の `sc-dbe-gji-msime-drv-3` step 1 FAIL が A1 撤去に由来するか(1 件、再現せず)。
- 本番の実利用での DD1-02 の到達(e2e の構成外。journal の集計が要る)。
- 確かめたコマンド: `gh run download <run> -p 'result-*'` → `grep -rah reach-probe --include=awase.log`。PASS/FAIL は各 `result-*/rc.txt`・`inv_rc.txt`。
