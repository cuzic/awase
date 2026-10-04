---
id: ADR-204
title: |-
  `ImeOpenOutcome` の「送っていない outcome」の処遇の重複を1メソッドへ集約し、`legacy_gji_sync_obligation` を網羅化する(最小版)。level 同期の発火を上流漏れの検出器にする
summary: |-
  BUG-170(ADR-203)の再発防止を検討した初稿(D1 網羅 const fn+新 enum 群、D2 証拠型 `OpenEvidence`、D3 決定表の生成、D4 `GjiSyncSkipped` journal)は、
  Opus round1(2026-09-29)で「背景の中心の主張が事実と違う」と指摘され、大幅に縮小した。c8bc1adc の作者は、3か所の網羅 match(2段目、`_` 無し)に
  コンパイラに問われたうえで「Unwarranted=送っていない=同期しない」と答えており、BUG-170 は黙殺ではなく**意味の誤り**(「awase が書かなかった」と
  「実 IME が動かなかった」の混同)だった。よって D1〜D3 の「型で問わせる」仕組みでは BUG-170 は防げなかった。BUG-170 型を実際に塞いでいるのは ADR-203 の
  (i) 送信直前の level 突合と入口の件数ガードである。
  本 ADR は、コード差し引きで減る最小の整理(`ImeOpenOutcome::applied_open` の追加による `matches!`+`unreachable!()` の重複削除、`legacy_gji_sync_obligation` の網羅化、
  誤コメントの修正)と、新しい型を作らない検出(level 同期が B2・起動直後以外で出たら上流漏れ)だけを定める。D2・D3 と旧 D4 の新 journal 型は取り下げた。
status: |-
  採択(縮小版)・未実装のまま(v2.0.0 時点、コード確認: ImeOpenOutcome::ALL は存在せず、legacy_gji_sync_obligation が残る。docs/known-bugs/BUG-170.md の残作業に未完了として記載)。再開条件は ADR-203 の実機検証後。
  旧(2026-10-04 更新前):
  採択(縮小版、2026-09-29)。opus-adversarial-consult round2 で条件付き収束(R2-1/R2-2 を反映済み)。実装未着手(ADR-203 の実機検証後)。
related_adr:
  - "ADR-203"
  - "ADR-089"
  - "ADR-090"
  - "ADR-167"
---

# ADR-204: 「送っていない outcome」の処遇の重複を集約する(最小版)

## 背景(origin/develop `efe66f45` 以降のコードで確認)

[ADR-203](203-gji-fsm-follows-belief-open-transitions.md) は BUG-170 の入口ごとの手当て(level 突合・Reopen・入口件数ガード)である。同種の漏れ(BUG-18/22/170)の再発を
より上流で防げないかを検討したのが本 ADR の初稿だったが、**初稿の前提は誤りだった**:

- 初稿は「`c8bc1adc` が `Unwarranted` を足したとき、`GjiFsm` 同期を落とす決定が誰にも問われなかった(`unreachable!()` は実行時にしか落ちない)」と書いた。
  実際には、重複箇所(`platform.rs::on_ime_applied_inner`、`runtime/executor.rs::update_intra_batch_applied`、`state/platform_state.rs::record_ime_apply_result`)は
  「非網羅の `matches!` で早期 return → その下で variant を**明示列挙した網羅 `match`**(`_` 無し)」の2段になっており、新 variant を足せば2段目で**コンパイルエラー**になる。
  `c8bc1adc` はそれを踏んだうえで、3か所すべてに `Unwarranted` を「送っていない」側として書き足した(`src/platform.rs` の doc も「`UnsafeToToggle`/`NotOwned` と同じく送っていない」と明言)。
  → BUG-170 は**黙殺ではなく意味の誤り**。「awase が書かなかった」ことと「実 IME の開閉が変わらなかった」ことの混同であり、ADR-191 以降は物理キーが awase を経由せず IME を動かす。
- したがって D1(網羅 const fn)・D2(証拠型)・D3(決定表の全数)のような「型で処遇を問わせる」仕組みは、**BUG-170 を防げなかった**(同じ作者は同じ理由で Skip と答える。証拠型は
  「証拠を作る呼び出しを入口に置き忘れる」という同じ漏れ方が残り、決定表の全数は「表に行が無い入口」を捕まえられない)。

BUG-170 型を実際に塞いでいるのは、ADR-203 で導入済みの次の2つである:
- (i) `needs_belief_sync_on`(`state/gji_direct_mechanism.rs`、`platform.rs::send_keys` 冒頭): 入口の種類を問わず「エンジンが送る時点で GjiFsm が OffCold」を捕まえる。
- `architecture_guard::gji_fsm_sync_entry_points_are_accounted_for`・`user_ime_on_paths_are_paired_with_gji_reopen`: 入口の増減と、user IME-ON 経路と Reopen の対を固定する。

### 「送っていない outcome」の処遇の現状(初稿の表の訂正)

| 箇所 | 形 | 新 variant を足すと |
|---|---|---|
| `platform.rs::on_ime_applied_inner`、`executor.rs::update_intra_batch_applied`、`platform_state.rs::record_ime_apply_result` | 1段目 非網羅 `matches!`+2段目 網羅 `match`(`unreachable!()` アーム) | 2段目でコンパイルエラー(既に問われる) |
| `state/ime_model.rs::completion_can_update_applied`(reduce ではない) | 非網羅 `matches!` | 「送った」側へフォールスルー。ただし同じ outcome を通る2段目で問われる |
| `state/ime_event.rs::from_apply_outcome`(→`ImeApplyFailed{error: Unwarranted}`)と逆写像 `ime_model.rs` | 網羅 match | コンパイルエラー |
| `runtime/message_handlers.rs` の u8 符号化(encode)、`journal.rs` の文字列化 | 網羅 match | コンパイルエラー(decode は `other => UnsafeToToggle` の安全側) |
| `src/platform.rs::ImeOpenOutcome::wrote_open_state()`(ADR-167) | 網羅 const メソッド | コンパイルエラー。**既に「非網羅な `matches!` の書き直しを1箇所に集約する」前例** |
| `state/gji_direct_mechanism.rs` のテスト `ALL_OUTCOMES`(INV-42「全数テスト」`settle_matches_legacy_obligation_for_every_outcome_and_open` が回す) | 手書きの配列で **7 variant 中 5つ**(`AppliedWithoutSendInput`(ADR-167)と `Unwarranted`(c8bc1adc)が欠落) | **コンパイルエラーにならない**(variant を足しても誰も足さなかった。「処遇が黙って落ちる」のテスト側の実例) |
| **`state/gji_direct_mechanism.rs::legacy_gji_sync_obligation`** | `matches!(UnsafeToToggle \| NotOwned)` 以外は `Some`(**真に非網羅、2段目の match 無し**) | **黙って「同期する」になる**(BUG-170 とは逆向き) |

`platform.rs::on_ime_applied_inner` のコメント「同期義務は無い(`legacy_gji_sync_obligation` が `None`)」は、ADR-203 の議論で誤りと確定した(実際は Unwarranted に `Some`)まま develop に残っている。
`drives_composition_side_effects() -> bool`(`ime_model.rs`、呼び出し元は `runtime/mod.rs` の1か所で `#[cfg(windows)]` のため host では dead に見える)は、理由を潰す bool である点は初稿の指摘どおり。

## 決定(最小版)

**D1(最小): `ImeOpenOutcome` に網羅 const メソッドを1つ足し、重複を削除する(コードは差し引きで減る)。**
- `src/platform.rs` の `ImeOpenOutcome` に、`wrote_open_state()` と同形式で `applied_open(self, want: bool) -> Option<bool>` を足す
  (`Applied`/`AppliedWithoutSendInput`/`AlreadyMatched` → `Some(want)`、`Failed` → `Some(!want)`、`UnsafeToToggle`/`NotOwned`/`Unwarranted` → `None`)。網羅 match で `_` は書かない。
- `on_ime_applied_inner`・`update_intra_batch_applied`・`record_ime_apply_result`・`completion_can_update_applied` の「`matches!` + `unreachable!()`」を
  `let Some(effective) = outcome.applied_open(open) else { … }` に置き換える(`unreachable!()` 3つと `matches!` 4つを削除)。
- `legacy_gji_sync_obligation` を網羅 `match` にし、`Unwarranted` を明示的に `None` にする(`generation=None` の `Unwarranted` は `drives_composition_side_effects` のゲートで `on_ime_applied` に
  到達せず receipt が作られないので、実際には呼ばれない。到達しない理由と「receipt が作られない outcome の一覧」を関数の doc に書く)。`platform.rs` の誤コメントを直す。
- **`ImeOpenOutcome::ALL: [Self; 7]` を core に置く**(R2-2)。隣に網羅 `match` の const fn(各 variant を index へ写し最大値+1 を `COUNT` とする)を置き、
  `const _: () = assert!(ImeOpenOutcome::ALL.len() == ImeOpenOutcome::COUNT)` で配列の長さと variant 数のずれをコンパイル時に検出する。`legacy_gji_sync_obligation`・
  `applied_open`・`wrote_open_state` のテストはこの `ALL` で回し、手書きの `ALL_OUTCOMES` は削除する。新しい型は増えない(core に const を1つ足すだけ)。
- 置き換え時の注意: `on_ime_applied_inner` の `else` では `receipt.settle(self)` を保って return する(落とすと INV-43 の `Drop` の `debug_assert` が出る)。`executor.rs`・`platform_state.rs`
  (`NotSent`)・`completion_can_update_applied`(`applied_open(open).is_none()` で `NotSent`)は従来どおりの early return。
- 実装後の姿: 上の表の「2段目でコンパイルエラー」の各行は `applied_open` の1か所に集約される。
- INV-42 は保たれる(導出式は `legacy_gji_sync_obligation` 1か所のまま)。`legacy_gji_sync_obligation` の doc(「`UnsafeToToggle` / `NotOwned` の場合のみ同期しない」)も実態に合わせて更新する。ADR-089 の INV-42 の記述に「outcome 由来の同期に限る。belief 由来の同期(ADR-203)は別軸」と1行注記する。
- 効果の正直な評価: **この整理は BUG-170 を防げなかった**(保守が楽になり `legacy_gji_sync_obligation` の非網羅が消えるだけで、検出力は既存のコンパイルエラーと同じ)。
  「意味の誤り」を型で防ぐ手段は無く、防いでいるのは ADR-203 の (i) と件数ガードである。

**D2(旧 D4 の新型なし版): level 同期の発火を、ON 系キーとして見ていない ON 経路の検出器にする。**
- (i) の `ImeOn(BeliefSync:level)`(`GjiFsmTransition.trigger`)は、ADR-203 が OFF を同期しない設計なので、GjiFsm が OffCold になる経路(awase が actuate した OFF、または GjiFsm の作り直し)の後に
  実 IME が ON のまま送る文脈で出る。**正当な発火文脈**(漏れではないもの):
  1. B2: GJI 種別の検出し直しで GjiFsm を作り直した(起動直後の最初の種別確定を含む)。
  2. フォーカス移動で IME が ON の窓に入った(窓 A で awase が OFF を actuate→OffCold のまま、IME が ON の窓 B へ移る。`FocusChange` は OffCold を保ち、`presync_applied_open_on` の対象外の窓で最初の送信に level が出る。日常的に起きる)。
  3. awase が ON 系キーとして見ていない経路での ON(言語バー・他アプリ・PowerToys Keyboard Manager〈報告の `competing_software`〉、予測表にセルが無いキー〈非決定セル・キーマップ未取得・修飾付き〉)。
  4. エンジンの活性の更新遅れ(belief が変わってから refresh までの窓、頻度は低い)。
- **不変条件は時間窓ではなくシナリオ単位**で決める: IME の開閉を on-key で行い、フォーカスを移さず、種別の切替もしない制御された e2e(現行の baseline・ts-*・sc-* の大半)では level は **0件**。
  B2 を狙う e2e (b) や、文脈 2・3 を再現する構成では **1件以上**を期待値にする。許容窓の長さの実測は不要(tuning 定数も増えない)。
  実測(step 0、修正後の baseline×3・atok-passthrough-cold×3 の `dist/awase.log`): 6 run すべて level は 0 件(`Reopen(BeliefSync:on-key)` は各3件、`IME kind sync` は各2件、最初の送信の前に Reopen が OffCold を ON にしているため B2 は発火していない)。
  この実測は B2 の許容窓の材料にはならない。check_invariants.py が読む `dist/awase.log` に揃えること(`awase-stderr.log`/`awase-filtered.log` には該当行が無い)。
- bug report の triage での意味は、「上流の漏れの直接の証拠」ではなく「**awase が ON 系のキーとして見ていない経路で IME が ON になっていた**」の証拠にとどめる。文脈 2・3 と区別するには、
  直前の journal(`FocusTransition`、GjiFsm の作り直し、Applied の OFF、`[key-effect-predict] no prediction`)を併読する。**新しい `JournalEntry` も ring の集約も要らない**(既存の trigger 文字列を使う)。

## 取り下げたもの(初稿)

- **D2 証拠型 `OpenEvidence`(7 variant+関数)**: 「証拠を作る呼び出しを入口に置き忘れる」同じ漏れ方が残る。忘れを検出するのは level 突合と入口件数ガードで、どちらも ADR-203 で入っている。
- **D3 決定表の単一データ化・文書生成**: 全数が保証するのは「表にある証拠の処遇が決まっていること」だけで、BUG-170 は「表に行が無い入口」だった。ADR-161 の生成基盤の流用も未確定。
  ADR-203 の入口件数ガードと「証拠の種類=入口」の表が同じことの二重記述になる。
- **D4 `JournalEntry::GjiSyncSkipped`(理由つき)**: 新型が要らない上記の代替で足りる。
- **`ImeOpenOutcome` の2 enum 分割**: 「実 IME の状態変化の証拠」は outcome からは得られない(物理キーは awase を通らない)ので、分割しても片方は常に「不明」になる。
- **receipt を outcome の生成箇所で作る案(INV-43)**: 変更が大きく、`Unwarranted` の receipt を作っても Skip と判定すれば同じ。ADR-203 は outcome 由来の同期を復活させていない。
- 理由: 宣言・型・ファイルを増やす方向は ADR-158 の北極星(減算に報酬)と逆で、`complexity-budget.md`(未発効)の 1-in-1-out の精神にも反する(D2/D3 には削除相手が無い)。
  起きていない失敗モード(網羅漏れ)を型で防ごうとして複雑化した点は、ADR-184 の教訓(round1〜6 で複雑化し最小限まで押し戻した)と同型だった。

## 決着した未決事項

- **`Unwarranted` で outcome 由来の同期を復活させるか → 復活させない(Skip のまま)。** ADR-203 の (i)+(ii) が同じ状況をより確かな根拠(送る時点の不一致、ON キーのイベント)で覆っている。
  復活させると同じ ON 遷移に2つの経路(receipt と Reopen/level)から同期が来る。
- ADR-065(state 層は `GjiFsm` に依存できない): 本案では state 層に新しい依存が生じない。
- presync・kind 同期の点パッチの撤去: ADR-203 決定4 のとおり、e2e (b) の結果次第(本 ADR の範囲外)。

## 検証方針

- D1: `ImeOpenOutcome::applied_open` の全 variant の単体テスト(`wrote_open_state` の既存テストと同形式、`ImeOpenOutcome::ALL` で回す)。`legacy_gji_sync_obligation` の全 outcome × open の網羅テスト(既存の
  `settle_matches_legacy_obligation_for_every_outcome_and_open` を `ALL` で回して `Unwarranted`・`AppliedWithoutSendInput` 込みにする。テスト `legacy_obligation_is_none_only_for_unsafe_to_toggle` は
  実態に合わなくなるので `legacy_obligation_is_none_for_not_written_outcomes` へ改名し、`Unwarranted → None`・`AppliedWithoutSendInput → Some` を足す)。`architecture_guard` に「`ImeOpenOutcome::Unwarranted` を `matches!` で列挙する箇所が0件」を足してもよいが、
  置き換え後はコンパイラが守るので任意。
- D2: `check_invariants.py` に、制御された構成では `ImeOn(BeliefSync:level)` が 0 件であることの検査を足す(構成ごとに期待値を宣言。B2/文脈 2・3 を狙う構成は 1 件以上)。

## 未決事項

- なし(D2 の許容窓の実測は、シナリオ単位の不変条件にしたため不要)。
- 独立 ADR のまま残す(「初稿の前提が誤りだった=BUG-170 は黙殺ではなく意味の誤りで型では防げない」の記録が、次に同じ提案が出たときの歯止めになる)。
