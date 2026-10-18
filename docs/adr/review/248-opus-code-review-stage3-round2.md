# ADR-248 段階3(置換への一般化)コードレビュー round2(Opus、敵対的)

- 対象: PR #606、HEAD `21e7356e`(round1 の対象 `0c666a66` からの差分 `git diff 0c666a66..HEAD` と、全体の `git diff origin/develop...HEAD`)
- CI(`gh pr view 606`、head `21e7356e4009`、2026-10-11 確認): `mergeStateStatus=UNSTABLE`。**`windows-build` が FAILURE**(下の B2)。
  `report-worker` も FAILURE だが `pnpm test` の時間の閾値(`expected 8.266… to be less than 8.199…`)で、本 PR とは無関係のゆらぎ(推測。再実行で確かめる)。
  `verify`・`shot`・`windows-settings`・smoke e2e は確認時点で実行中。
- 再現: scratchpad `rv4/t.rs`(round1 と同じ `#[path]` 方式。HEAD の5ファイルを並べ `rustc --edition 2021 --test`)。下の q1・q4 は実行して確かめた。

## 結論

round1 の S1〜S7・N1〜N6 は、意図どおり反映されている(各論は後述)。ただし **S1 の事後条件が「削除だけの要求」にも当たるため、
ADR-230 以来の核の不変条件「削除だけの操作は常に通る」(`scancode_pairs.rs` 冒頭の doc :8-10、ADR-230 決定4)が崩れた**(B1)。
1行で直せる。もう1つは CI の clippy が赤(B2、1行)。この2つを直せばマージしてよい水準と判断する。

---

## Blocker

### B1. 事後条件 `Reclassified` が削除だけの要求にも当たり、「削除だけの操作は常に通る」が崩れる(1単位の「戻す」が適用できない)

- 根拠: `scancode_pairs.rs` の事後条件(`compute_swap_write_with_cycles` の末尾、`let all_read = units.iter().all(…)`)が、
  新規の単位だけでなく**残す単位(既存の単位と値で一致するもの)**と、**既に有効な Caps 追加 Ctrl を残す場合**にも当たる。
  削除で他のエントリが露出すると、残す単位が読み替わるのは ADR-230 で `revealed` として認めた現象で、事後条件はそれを書き込み拒否に変えてしまう。
- 失敗シナリオ1(q1、実行して確認): `existing = [(スペース→左Alt),(左Alt→スペース),(無変換→スペース),(スペース→無変換),(無変換→変換),(変換→無変換)]`。
  読み取りは `スペース⇄左Alt`(警告つき)・`無変換⇄変換`(警告つき)+ Unclaimed 2件。図で「スペース」の「戻す」(= `release_position`)→
  `compute_swap_write_with_cycles(existing, [無変換⇄変換], [], false)` は **`Err(Reclassified)`**。消した後、先頭の貪欲な組み取りが
  `無変換⇄スペース` を先に取り、残すはずの `無変換⇄変換` が読めなくなるため。全削除(`[]`)は通る。
- 失敗シナリオ2(q4、実行して確認): `existing = [(Caps→左Ctrl),(左Ctrl→無変換),(無変換→Caps),(無変換→変換),(変換→無変換)]`。
  読み取りは Caps 追加 Ctrl オン + `無変換⇄変換`(警告つき)+ Unclaimed 2件。「Caps を Ctrl としても使う」を残したまま `無変換⇄変換` を外す
  (`compute(existing, [], [], true)`)は **`Err(Reclassified)`**。外すと他ツールの2件と `Caps→左Ctrl` が閉じた3巡回として読めるため。
  恒等(`[無変換⇄変換]`, `true`)は通る。
- 実害: 図の「戻す」が、適用の段で「他のツールの設定と組み合わさって…他のツールの設定を先に外してください」(`swap_error_text`)で止まる。
  他ツールの設定は awase から外せない(ADR 非目標)ので、利用者に残る手段は「すべて解除」だけ。キーボードを壊す書き込みではないが、
  核の不変条件の後退であり、round1 の性質テストは全削除しか見ていないので検出できない(scratchpad の乱数の部分削除 3,024 通りでは
  たまたま出なかった。q1 は 6 エントリが要る)。
- 推奨修正: 事後条件を**新しく足すもの**だけに当てる。`new_units` が読み戻せること、`adds_caps` のとき `caps_extra_ctrl` として読めること。
  新規の単位はキーが他ツールから孤立している(from は `replaced`、to は `CollidesWithForeignTarget`)ので、実質的に効くのは `adds_caps` の検査
  (= round1 S1 の穴)だけで、S1 の防御は落ちない。q4 の「残す Caps が循環に読み替わる」は削除による露出で、`revealed` に `(無変換→Caps)` が出て
  確認ダイアログが出るので、書き込みを止めるより露出の警告で扱う方が ADR-230 と整合する。
  テスト: q1・q4 を固定ケースにし、性質テストに「既存の単位の任意の部分集合(+ caps はそのまま/オフ)を残す要求は常に `Ok`」を足す
  (`exhaustive_small_universe_keeps_the_invariants` の `lists` に対して部分集合を全列挙すれば十分小さい)。
  事後条件の doc と ADR-230 改訂節・ADR-248 実装状況の「書き込みの事後条件」の文も「新しく足したもの」に直す。

### B2. CI `windows-build` の clippy が赤(`clippy::tuple_array_conversions`)

- 根拠: job 114406423431「Clippy (awase-windows)」のログ:
  `error: it looks like you're trying to convert a tuple to an array --> crates\awase-windows\src\scancode_editor.rs:284:41`
  `.flat_map(|&(entry, _)| [entry.0, entry.1])`(`-D clippy::nursery` 由来)。報告の「clippy-driver で指摘なし」は CI と lint の設定が
  そろっていなかったと考えられる(推測)。
- 推奨修正: `.flat_map(|&((from, to), _)| [from, to])` に書き換える(分解してから配列を作れば、この lint は出ない)。
  push 後に `gh pr view 606 --json mergeStateStatus,statusCheckRollup` で `windows-build` の SUCCESS を確かめてから報告する。

---

## Should-fix

なし(round1 の Should-fix はすべて意図どおり反映。B1 は S1 の反映で入った後退)。

---

## Nit

### N1. 注意書きの対象が「足した単位」のまま(禁止の判定は `moved_keys` に変えたので、基準がそろっていない)

- 根拠: `EditorState::preview`(`scancode_editor.rs`)は `added`/`added_cycles`(読み込み時に無かった単位)を `cautions_with_cycles` に渡す。
- 例: round1 r4 の4巡回 `無変換→変換→スペース→左Alt` で、スペースの位置の機能を左Alt へ戻すと、変換(親指キー)の位置も機能も変わらないのに、
  新しい3巡回に変換が入っているので `ThumbKeyMoved{変換}` が出る(禁止の判定 `ime_role_conflict` は正しく止めない)。
- 推奨: 注意も `changes()`(位置 → 前 → 後)から作る(σ が変わった位置だけを対象に `cautions_from_permutation` を当てる)。
  `ThumbKeyRestored` も「読み込み時は σ(t)≠t、いまは σ(t)=t」で判定すれば、added/removed_keys の組み立てが要らなくなる。

### N2. 事後条件は「他ツールのエントリが別の単位として新たに読めるようになる」を、displace 経由では止めない(依頼の論点2)

- 確認(実行): `existing = [(Caps→0x0011),(0x0011→左Ctrl),(左Ctrl→Caps),(変換→Caps)]`(他ツール4件。`Caps` が to に2回あるので循環に読まない)に、
  新しい3巡回 `左Alt→変換→無変換` を要求すると `Ok`、`displaced=[(変換→Caps)]`。書いた結果を読むと、残った他ツールの3件が
  `Caps→0x0011→左Ctrl→Caps` の awase の循環に読み替わる。事後条件(要求した単位・caps が読めるか)は満たすので通る。
- 図からは起きない(変換は他ツールが to/from にしているので `Locked`、ワンクリックは `OtherToolSetting` で止まる)。起きるのは昇格 CLI を
  直接呼んだ場合と、displace の確認を通した場合だけで、確認ダイアログには「他のツールが設定した『変換 → 英数』を消します」としか出ない。
- 推奨(どちらでもよい): (a)事後条件に「Unclaimed のうち、`replaced`・`removed_own` に入らなかったものは、書いた後も Unclaimed のまま」を足す
  (ただし B1 のとおり削除だけの要求には当てない。新規の単位か `adds_caps` があるときだけ)。(b)現状を ADR-230 改訂節に「displace の後は
  残った他ツールのエントリが入れ替えとして読めることがある(確認ダイアログを通った場合のみ)」と書く。実害は小さいので (b) で足りる。

### N3. `Reclassified` の文面が、B1 を直した後の唯一の発生源(Caps 追加 Ctrl で鎖が閉じる)に合わせて具体的にできる

- `swap_error_text(Reclassified)` は「他のツールの設定を先に外してください」だが、awase には他ツールの設定を外す手段が無い(ADR 非目標)。
  B1 修正後は発生源が「Caps を Ctrl としても使う」をオンにする場合だけになるので、「他のツールの設定(『左 Ctrl → …』)と組み合わさって
  英数 / Caps と左 Ctrl が入れ替えに変わってしまうため、『Caps を Ctrl としても使う』はオンにできません」のように、どのスイッチが原因かを書く。
  さらに図の外のチェックボックスでも同じ検査を当て、`caps_extra_unavailable` に理由を出すと、適用の段で初めて止まることが無くなる。

### N4. `settings-screenshot.yml` の push トリガ(round1 N5)

- 予告どおり、マージ前の最後のコミットで `ci/settings-screenshot` だけに戻すこと(HEAD ではまだ `feat/adr248-stage3-permutation`)。

---

## round1 指摘ごとの再確認

| ID | 判定 | 確認内容 |
| --- | --- | --- |
| S1 | 反映済み(ただし B1 の後退) | `caps_extra_ctrl_that_would_close_a_foreign_chain_is_rejected_not_reclassified`・`decide_refuses_caps_extra_that_would_close_a_foreign_chain` で r1 と許可リスト外経由の鎖を固定。性質テストの「循環に読み替わった」許容は2箇所とも削除済み。 |
| S2 | 反映済み | `to_spec` は常に `cycles=` を出し、`from_spec` は欠けると `MissingOrDuplicateKey("cycles")`。古い UI → 新しいワーカーは書かずに `BadArguments`、新しい UI → 古いワーカーは `UnknownKey` で書かない。両方向とも安全側。 |
| S3 | 反映済み | `cautions_from_permutation` は σ(t)=Caps で `CapsWithThumbKey`、σ(t)=他方の親指で `ThumbKeysSwapped`、σ(スペース)≠スペースで `SpaceMoved`。ペアでは従来の結果と同じ(ペアは σ=σ⁻¹ なので向きが効かない)。向きの2ケースと逆向きの左右逆のケースを固定。意味は正しい(論点4)。N1 は対象の選び方の問題で、向きの判定ではない。 |
| S4 | 反映済み | `moved_keys` は「σ が変わる位置 p と、その前後の機能」から「変更後も入れ替えに残るキー」を残す。恒等へ戻ったキーは変更後の置換に現れないので外れ、`release_all`・丸ごとの「解除」・自分の位置へ戻すドロップは止めない(論点3)。逆に、止めるべきキーが漏れる経路(位置も機能も変わるのに含まれない)は、変わる位置・前の機能・後の機能を全部集めているので無い。`drop_function` も同じ関数で `Blocked` を返し、適用の段の判定と一致する。 |
| S5 | 反映済み | `foreign_keys`(読み込み時の Unclaimed の from/to、0 を除く)で `quick_unavailable` が `OtherToolSetting`。図では他ツールのキーは従来どおり `Locked`、ワンクリック経由で自分の単位に他ツールのキーが入る経路が無くなったので、`position_state` を変えなくてよい判断は妥当。 |
| S6 | 反映済み | ADR-230 改訂節(分類表の5行・事後条件・`cycles=` 必須)、ADR-248 実装状況(`perm=`→`cycles=`、`NotPermutation` 不要の理由、ドロップの意味、禁止の判定、注意書き、差分表示、実機の未確認事項)、index の状態更新。バイト列の固定テスト(ヘッダ・count+1・`to,from` の LE・終端、`parse_entries_strict` → 同じ向きの `Cycle`)。`changes()` と適用ボタン直上の常時表示。事後条件の記述は B1 に合わせて直す必要がある。 |
| S7 | 反映済み | 長さ3以上で「解除(N個)」+ hover。長さ2は「戻す」のまま。 |
| N1〜N3 | 反映済み | `drop_hint` の腕の順、「ほかの位置は変わりません」、`set_pairs` 非公開(呼び出しは `set_units` のみ)。 |
| N4 | 反映済み | 24×24 の遷移、固定シードの乱数 3 万件、3巡回→ペアの中身、`decide` の恒等。乱数テストに部分削除が無いのが B1 を見逃した原因なので、B1 の推奨テストを足すこと。 |
| N5 | 未反映(予告あり) | 本 round の N4。 |
| N6 | 反映済み | ADR に記録。 |

## 新しい矛盾・漏れの確認(論点5)

- `changes()` は位置ごとに「読み込み時の機能 → いまの機能」を出し、解除した循環は「元へ戻る位置」として出る(テストあり)。Caps 追加 Ctrl の変更は別の行。
  他ツールのエントリ由来の変化(displace)は出ない。displace は確認ダイアログで出るので重複しない。問題なし。
- `DropRefusal::Blocked` の文面は `LockReason::text()` を流用している。`ThumbImeRole` の文面「親指キーが IME の ON/OFF キーも兼ねているため、入れ替えられません」は
  ドロップした位置ではなく循環の中の別の位置が理由なので、どのキーが原因かが分からない(Nit 相当。キー名を足すとよい)。
- `foreign_keys` は読み込み時のスナップショットで、タブを開くたびに読み直す設計(段階2)と整合する。
- 図の `position_state` の自分の単位の位置は、他ツールのキーを見ないまま。ワンクリックで塞いだので現状は到達しないが、今後ほかの「単位を足す入口」を作るときは
  `foreign_keys` を同じく当てること(入口ごとに判定を足す型なので、`EditorState` に「単位を足す」唯一の関数を置く方が漏れにくい)。

## 収束の判断(論点6)

B1(事後条件を新規分に限る、1行+テスト)と B2(clippy、1行)を直して CI が緑になれば、マージしてよい。round3 は B1 の修正とテストの確認だけでよい。
実機(向き・UAC・再起動・循環の解除)は ADR-248 実装状況に未確認として記録済みで、マージの条件にはしない(ADR-230 段階5 と同じ機会に確かめる)。
