# ADR-248 段階3(置換への一般化)コードレビュー round3(Opus、敵対的)

- 対象: PR #606、HEAD `1f62e7e5`(round2 の対象 `21e7356e` からの差分 `git diff 21e7356e..HEAD`: `9dc76791`・`1f62e7e5`)
- CI(`gh pr view 606`、head `1f62e7e59c5c`、2026-10-11 確認): 確認時点で `windows-build`・`windows-settings`・`test` などが実行中のため、
  **B2(clippy)が直ったかは未確認**。`9dc76791` のチェックも `windows-build` は実行中、Linux の `clippy` は SUCCESS。
  確かめる手段: `gh pr view 606 --json statusCheckRollup` の `windows-build`(Clippy (awase-windows) の段)。
- 再現: scratchpad `rv5/t.rs`(HEAD の5ファイルを `#[path]` で並べ `rustc --edition 2021 --test`)。p1〜p3 は実行して確かめた。

## 結論

B1 は意図どおり直った。「削除だけの操作は常に通る」が戻り、S1 の防御(新たにオンにする Caps 追加 Ctrl が他ツールの鎖を閉じる)も残っている。
ただし **N1 の反映(注意の対象を `changes()` にした)で、親指キーとスペースの「機能が別の位置へ移る」編集に注意が出なくなった**(S1)。
書き込みの安全性には関係しない(注意は確認ダイアログの文面と、ダイアログを出すかの判定だけ)。Blocker は無い。
S1 を直し、CI の `windows-build` が緑であることを確かめれば、マージしてよい。

---

## Blocker

なし(CI の `windows-build` の結果は未確認。赤ならそれが Blocker)。

---

## Should-fix

### S1. 注意が「位置の出力が変わったか」だけを見るので、親指キー・スペースの**機能**が別の位置へ移る編集で `ThumbKeyMoved`・`SpaceMoved` が出ない(round2 N1 の反映で入った後退)

- 根拠: `EditorState::preview`(`scancode_editor.rs`)は `changes()` から「出力が変わり、恒等に戻らない位置」だけの σ(`changed_sig`)を作り、
  `cautions_from_permutation` に当てる。`cautions_from_permutation` は `image(t) != t`(親指キー t の**位置**が別の機能を出す)で
  `ThumbKeyMoved` を、`image(スペース) != スペース` で `SpaceMoved` を出す。置換の全体なら σ(t)≠t ⇔ σ⁻¹(t)≠t なので同じだが、
  「変わった位置だけ」の σ では、t の位置の出力は変わらず、t の**機能を出す位置**(σ⁻¹(t))だけが変わる編集を取りこぼす。
  親指シフトの物理位置(`ThumbKeyMoved` の文面「親指シフトを押す物理的な位置が変わります」)も、スペースでの変換操作の位置(`SpaceMoved`)も、
  機能側の σ⁻¹ の話である。
- 失敗シナリオ(p1、実行して確認): 読み込み時 `左Alt→変換→スペース→左Alt`(左Alt の位置が変換=親指キーを出す)。編集で左Alt を元に戻し
  `変換⇄スペース` のペアにする(図では、変換の機能をスペースの位置へ落とすか、左Alt の機能を左Alt へ戻す)。
  `changes()=[(左Alt, 変換→左Alt), (スペース, 左Alt→変換)]`。親指シフトの物理位置は左Alt → スペースへ移るのに、`preview(…, [変換]).cautions = [SpaceMoved]`
  (`ThumbKeyMoved{変換}` が無い。`ThumbKeyRestored` も無い)。round2 の実装(足した単位が対象)では `ThumbKeyMoved{変換}` が出ていた。
- 失敗シナリオ(p2、実行して確認): 読み込み時 `左Alt→スペース→変換→左Alt`(左Alt の位置がスペースを出す)。編集で `スペース⇄変換` のペアにする。
  スペースの機能は左Alt の位置 → 変換の位置へ移るのに、`preview(…, []).cautions = []`。注意が空なので、displace・revealed が無ければ確認ダイアログも出ない。
- 推奨修正: 判定を、S4 で禁止の判定に使った `touched_keys`/`moved_keys`(位置の出力が変わった位置と、その前後の機能を両方集める)にそろえる。
  - `ThumbKeyMoved{t}`: t ∈ `moved_keys(読み込み時, いま)`(位置か機能が変わり、まだ入れ替えに残る)。うち いまの σ(t) = Caps なら `CapsWithThumbKey{t}`。
  - `ThumbKeyRestored{t}`: t ∈ `touched_keys` かつ t ∉ `moved_keys`(恒等へ戻った)。
  - `ThumbKeysSwapped{t1,t2}`: いまの σ(t1)=t2 で、t1 か t2 が `moved_keys` にある。
  - `SpaceMoved`: スペース ∈ `moved_keys`。
  p1・p2 を固定テストにし、round2 N1 の4巡回の例(親指キーの位置も機能も変わらない → 出さない)が引き続き通ることを確かめる。

---

## Nit

### N1. `caps_extra_would_reclassify` は `Reclassified` だけを事前に判定し、Caps 追加 Ctrl をオンにしたときの他の拒否(`CollidesWithForeignTarget`)は適用の段まで分からない

- 根拠: `scancode_editor.rs` の `caps_extra_would_reclassify` は `matches!(…, Err(SwapError::Reclassified))`。他ツールの `X→左Ctrl` があると、
  オンにした `Caps→左Ctrl` は to の衝突で `CollidesWithForeignTarget` になる(段階2以前からある拒否)が、チェックボックスはオンにできる。
- 推奨: 「オンにした要求が `compute_*` で `Err` になるか」で判定し、理由は `swap_error_text` を出す(N3 の意図「適用の段で初めて止まらない」を一般化)。
  ただし編集中の他の単位の誤りで `Err` になる場合と区別するため、`caps=false` で `Ok`・`caps=true` で `Err` のときだけ無効にする。

### N2. `DropRefusal::Blocked` の文面に原因のキー名が無い(round2 で挙げた点、未対応と明記済み)

- ドロップした2つの位置とは別の位置(循環の中の IME 兼用キー・Caps)が理由になるので、キー名があると迷わない。影響は小さい。

### N3. `CAPS_WOULD_RECLASSIFY_TEXT` の「他のツールの設定を先に外してください」

- awase からは外せない(ADR 非目標)ので「そのツールで外してください」の方が誤解が無い(`LockReason::OtherToolSetting` の文面と同じ言い回し)。

---

## 再確認の結果(依頼の論点ごと)

1. **B1**: 事後条件は `new_units` の読み戻しと `adds_caps` のときの `caps_extra_ctrl` だけになった。削除だけ・恒等の要求では `new_units` が空で
   `adds_caps` が偽なので、事後条件は常に満たされ、「削除だけの操作は常に通る」が戻った(q1・q4 の固定ケースと、乱数 3 万件の各反復での部分集合の全列挙)。
   S1 の防御は `caps_extra_ctrl_that_would_close_a_foreign_chain_is_rejected_not_reclassified` で残っている。
   別の読み替わりの型の確認(p3、実行): 乱数のレジストリ(許可リスト外・0・恒等・重複・Caps を含む 0〜6 件)に、既存の単位の乱数の部分集合 +
   乱数の新しいペアか3巡回 1つ + 乱数の caps を要求して 40 万件。`Reclassified` は 284 件で、**すべて Caps 追加 Ctrl を新たにオンにする要求**だった。
   新しい単位はキーが他ツールから孤立する(from は `replaced`、to は `CollidesWithForeignTarget`)ので読み戻せなくなることが無い、という round2 の推論と合う。
   よって `Reclassified` の文面を Caps 専用にしたこと(N3 の反映)も実態と一致する。
2. **N1 の注意の対象**: 過剰(位置も機能も変わらない親指キーへの注意)は直った。漏れは上の S1。`ThumbKeyRestored` は「読み込み時 σ(t)≠t、いま σ(t)=t」で、
   t が恒等へ戻る場合は正しく出る(全体の σ で見て、位置が戻れば機能も戻るため)。`CapsWithThumbKey`・`ThumbKeysSwapped` は位置側の条件なので
   「変わった位置だけ」の σ で正しい(t の位置の出力が変わらなければ、Caps や他方の親指の機能が t の位置へ来る変化も無い)。
3. **N3 の事前判定**: `caps_extra_would_reclassify` は、適用と同じ `loaded.entries`・`pairs()`・`cycles` で `compute_*(…, true)` を呼ぶので、
   `Reclassified` については適用の段と一致する。読み込み時からオン(`adds_caps` が偽)なら適用でも事後条件に当たらず、事前判定も `!self.caps_extra` で聞かないので一致。
   一致しないのは N1(他の拒否理由)だけ。
4. **新しい矛盾・漏れ**: S1 のほかに見つからなかった。`foreign_keys` は `push` で集める形になり、`tuple_array_conversions` を避ける書き方として妥当
   (CI の結果は未確認)。`settings-screenshot.yml` の push トリガは `ci/settings-screenshot` だけに戻った。ADR-230 改訂節・ADR-248 実装状況の事後条件の文は
   「新しく足すものだけ」に直っている。
5. **収束**: Blocker なし。S1(注意の判定を `moved_keys`/`touched_keys` にそろえる、テスト2本)を直し、`windows-build` の SUCCESS を
   `gh pr view 606 --json statusCheckRollup` で確かめればマージしてよい。S1 は書き込みの安全性に関係しないので、所有者が急ぐなら
   既知の不足として ADR-248 実装状況に書いて後続の PR に回す判断もありうる(その場合は p1・p2 を記録しておくこと)。
