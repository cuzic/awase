---
title: 受動化後も残る能動的な書き込みの棚卸し（フォーカス変更時強制OFF・drift correction・conv軸）と回復手段の喪失
status: 未着手
created: 2026-09-24
related_adr: ["ADR-191", "ADR-090", "ADR-193", "ADR-098", "ADR-121"]
source_review: 俯瞰レビュー（2026-09-24）の B-5（起動時強制ON以外）/ C-2
---

# 残存する能動書き込みの棚卸し（俯瞰レビュー B-5 後半 / C-2）

索引: [11](review-2026-09-24-11-low-priority-backlog.md)。起動時の `desired_open=true` は [05](review-2026-09-24-05-startup-desired-open-forced-on.md)。
裏取り基準は `5877f982`（origin/develop）。`cbae84ff` 以降の差分（PR #293〜#296）で `crates/awase-windows/src` に入った変更は `tuning.rs`（`KEY_EFFECT_SETTLE_MS`）だけで、`lints/` は変わっていない。本文の行番号は `5877f982` で再確認した。
`.claude/rules/ime-belief-architecture.md`・`fix-requires-evidence.md`（IME actuation 合流点・warmup・focus 遷移・conv mode の各ファミリー）の対象領域。

旧称「ADR-178撤去プロジェクト領域A」（force-on / reassert の撤去）の「ADR-178」は、現 ADR-179 の旧番号（`docs/adr/index.md:186`「元178番…179へ採番し直し」）。現在の `docs/adr/178-*.md` は MSI アンインストールの別 ADR。ADR-179 本文には領域A の記述が無く（撤去の決定はどの ADR にも書かれていない）、[10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) の A-5 が ADR-179 に「領域A・C の撤去」節を追記する。本文書では以下「ADR-179（旧178）領域A」と書き、ADR-178 は関連 ADR に含めない。

## 現状（裏取り済み）

### 開閉軸の能動書き込み（ADR-191 決定5 指標3 = `set_ime_open_ordered` の呼び出し2箇所と一致）

| 経路 | 場所 | 条件・由来 |
|---|---|---|
| フォーカス変更時の強制OFF | `runtime/ime_refresh.rs:599-609`（`issue_actuation_order(false, "focus_change_enforce_off")` が `:602`、`set_ime_open_ordered(order)` が `:603`） | `if !applied_ime_on && !new_profile_is_tsf_native`。**非TsfNative だけが対象**。コードコメントの由来は ADR-090 §2.A 設計案3 / A-2 |
| drift correction | `ir_apply_drift_correction`（`ime_refresh.rs:645`）。`set_ime_open_ordered` が `:934`、`apply_ime_open_with_belief(order, None, belief)` が `:947` | 判定は `state/platform_state.rs` の `check_drift_correction`。BUG-020（TsfNative 救済）/ BUG-043（16回連続送信） |

起動時の強制ON（`desired_open=true` 初期値）は drift correction の経路で書かれる。これは [05](review-2026-09-24-05-startup-desired-open-forced-on.md) が扱い、ここでは別の行として数えない。

### フォーカス変更時の強制OFF（ADR-191 P1 の調査結果）

調査ブランチ `feat/adr191-p1-focus-forced-off-investigation`（先端 `49638d07`、2026-09-21）の状態を確認した:
- ローカルにだけあり、origin に push されていない（`git ls-remote origin` に該当なし）。worktree `/home/cuzic/rust-nicola-worktrees/adr191-p1` にチェックアウトされている。
- merge-base は `116eebdc`（2026-09-21、PR #233 のマージ）で、develop（`5877f982`）より471コミット遅れている。先行している5コミットの差分は docs の2ファイルだけ: `docs/teardown-verification-guide.md`（+207）、`docs/ime-passive-model-expected-results.md`（+229）（`git diff --stat 5877f982...<branch>` の三点 diff で確認）。二点 diff（`git diff 5877f982 <branch>`）では 213 ファイル・−37,517 行と出るが、これはブランチが古いだけで、ブランチ側の変更ではない。
- ガイド本文は `116eebdc` 時点のコードを見て書かれている。例えば §7.1 の「`PlatformRuntime::set_ime_open`」は、`5877f982` では `set_ime_open_ordered`（`platform.rs:1683`）が ADR-090 A-2 の授権チェック（`order.into_actuation().is_none()` なら書かずに `false`）を経てから呼ぶ。IMM 専用という結論は変わらない。

同ガイド §7.1 の要点（コードと突き合わせて確認済み）:
- 書き込みは `set_ime_open_ordered` → `PlatformRuntime::set_ime_open`（IMM32 専用）で行う。授権が下りなければ書かない。**効くのは ImmCross のアプリだけ**で、TsfNative には効かない（上表の条件とも一致）。
- 条件の `applied_ime_on` は、同じ関数の直前で非TsfNative のとき `record_confirmed(effective_open)` として書いた**前の窓の belief** で決まる。新しい窓の実IMEは読んでいない。観測していない値で書いているので、ADR-191 決定1に反する。
- 専用のテストも known-bugs も無い。裏付けは ADR-090 のインベントリ表の1行だけで、導入の意図は `git log -S` でも辿れない。
- 撤去は約10行で試作できるが、確認手段が無い（同ガイド §6 の表で L1/L2・L3 とも「無し」）。撤去すると、belief=OFF のまま新しい窓の実IMEが ON の場合に「Engine OFF なのに IME ON」のずれが観測で上書きされるまで残る。その持続時間は**未検証**。

### drift correction（ADR-191 P2 の前提）

同ガイド §7.2 の要点:
- **判定側**は既存テストで守られている: `check_drift_correction` の単体テストと `crates/awase-windows/tests/drift_correction_replay.rs`（BUG-043 の16回連続送信を有界化）。どちらも Linux で走る。
- **書く側**（TsfNative で VK を実送信して回復するか）は純粋テストでは測れない。実機か、ADR-193 の RichEdit スーパークラス（`RICHEDIT50W` を `Chrome_RenderWidgetHostHWND` の名前で登録、awase は TsfNative として扱う）を使った CI E2E でしか測れない。ADR-193 の入力先の `e2e-ime.yml` への配線は未着手（同ガイド §8-2）。
- 明示意図の回復シナリオ（ユーザーが OFF にした直後に IME が ON へ戻る）を ATOK/GJI で作れるかは未確認。作れなければ、drift correction は撤去せず残す判断の根拠になる（同ガイド §8-3）。

### フォーカス変更時の eager warmup

強制OFFの直前、`ime_refresh.rs:594` の `self.platform.send_eager_warmup(warmup_ime_on)` がフォーカス変更ごとに ON 方向の warmup を送る。コメントに「トレイで半角英数へ切り替えた直後のフォーカス復帰で、一度だけひらがなへ戻る（既知の制限）」とあり、conv 軸にも作用する。ADR-191 決定1は warmup を既存の例外として撤去対象外にしているので、棚卸し表には「例外として維持」の行として載せる。

### conv 軸（変換モード）の書き込み

ADR-191 の線引き「awase が書いてよいのは開閉だけに作用するキー」（frontmatter summary の (2)。本文では決定1の線引き。本文の「決定2」は BUG-151 の最小修正で別物）に照らして棚卸しする対象。以下は**現時点で見つかった経路で、下限**（件数は棚卸しの成果物として確定させる）。

IMM 経由の書き込みは、最終的に `ime.rs:371` の `modify_conv_mode`（`ime.rs:393` で `actuate_ime_control(…, ActuateCmd::SetConversionMode(…))`）に集まる。`modify_conv_mode` を直接呼ぶのは `ime.rs:709`（`set_ime_romaji_mode_for_hwnd`）、`:750`（`set_ime_hiragana_mode_cross_process`）、`:1771`（`set_ime_mode_for_target`）の3箇所。そこから上へ辿った呼び出し元:

| 呼び出し | 場所 | 備考 |
|---|---|---|
| `set_ime_conv_for_target` | `runtime/key_pipeline.rs:1847`、`:2574`、`:3170` | |
| 同上 | `output/conv_actuation.rs:176` | |
| 同上 | `tsf/warmup/cold_warmup.rs:94` | warmup の一部。指標5の「warmup」に含まれると読める |
| `set_ime_open_then_conv_for_target` | `runtime/open_chain.rs:312` | 開の直後に conv を書く（`ConvAfterOpen::Write`）。`runtime/executor.rs:926` の `decide_dispatch_conv_after_open` が決める |
| `set_ime_romaji_mode_for_target_blocking` | `ime_controller.rs:439` | 同期経路の ROMAN ビット補完。呼び出し元は `architecture_guard.rs` の `sync_romaji_write_goes_through_a_captured_target` が固定済み |
| `set_ime_mode_for_target` | `runtime/message_handlers.rs:1291` | トレイからのリセット |
| `set_ime_hiragana_mode_cross_process_async` | `runtime/mod.rs:1794` | パニック時のリセット |

- トレイリセットの `set_ime_mode_for_target(hwnd, true, …)`（`message_handlers.rs:1291`）は、conv だけでなく先に `set_ime_open_for_target` で開閉も書く（`ime.rs:1757-1763`）。パニックリセットも `runtime/mod.rs:1791-1792` で `set_ime_open_cross_process_async(false/true)` を書く。どちらも `set_ime_open_ordered` の外なので、開閉軸の表（指標3）には入らない。T5 で「開閉軸・正当な例外」の行として載せる。
- `ime.rs:1742` の `set_ime_mode_for_target(` は独立した経路ではない。`pub unsafe fn set_ime_mode`（`ime.rs:1733`）の本体が委譲しているだけで、`set_ime_mode` の呼び出し元は `crates/` と `src/` にゼロ（`grep -rn "set_ime_mode(" crates src` で定義行のみ）。棚卸し表からは外し、デッドコードとして [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) の B-6（撤去後に使われなくなったコード）へ回す。
- **モードキー注入による conv 変更**も別系統としてある: `kp_restore_kana_from_half_width`（`key_pipeline.rs:1463`、`:1506`、`:1761`、`:2301`、`ime_refresh.rs:302`）や shift-conv-guard の `VK_DBE_HIRAGANA` 注入など。決定1の線引きは「キー」について述べているので、IMM 経由の書き込みに加えてこれを対象に含めるかを、棚卸しの最初に決める。

ADR-191 決定5の指標5（IME へ書く振る舞いの数）の列挙は「固定の例外・表駆動の追加・opt-in の単独タップ・`keys.ime_on/off/toggle`・EngineDecision・warmup」。conv 軸の書き込みは、`cold_warmup.rs:94` を除いて対応する項目が無い。

lint（`lints/actuation_call_guard/src/lib.rs:98-101`）は `actuate_ime_control` の許可呼び出し元として `set_ime_open_for_target` と `modify_conv_mode` を持つだけで、`set_ime_conv_for_target` / `set_ime_mode_for_target` / `set_ime_romaji_mode_for_hwnd` を呼ぶ側は数えていない（確認済み）。

## C-2: 回復手段の喪失と非対称

- force-on と reassert は撤去済み（`f83084b3` / `621bf93c`、`5877f982` に含まれる）。TsfNative の ON 方向の救済は drift correction だけ。記憶メモによれば、撤去時点で「drift 単独で代替できるか」の実機 A/B は未実施で、その後の実施記録もリポジトリ内で見つからない（**未確認**）。
- 一方で、状態を押し付ける書き込みは残る: 起動時の強制ON（[05](review-2026-09-24-05-startup-desired-open-forced-on.md)）と、ImmCross へのフォーカス変更時の強制OFF。「救済のための書き込みは消したのに、押し付ける書き込みは残っている」形。
- 注意: 強制OFFは TsfNative では発火しないので、TsfNative の「回復手段の喪失」は drift correction だけの問題。強制OFFの撤去で確かめるべきなのは ImmCross 側のずれの持続時間。

## タスク

- [ ] **T1 P1 調査結果の取り込み**:
  - (a) worktree `adr191-p1` を使っているセッションを確認する（`worktree-per-session`。他セッションの作業中ブランチを勝手にマージしない）。
  - (b) `docs/teardown-verification-guide.md` と `docs/ime-passive-model-expected-results.md` を develop に入れる（docs のみ、`main-develop-branch-flow` に従い develop へ直接マージ可）。手段は先行5コミットの cherry-pick か2ファイルのチェックアウト（ブランチは471コミット遅れているので、ブランチごとのマージや二点 diff での確認はしない）。取り込むとき、ガイド中のコード参照（関数名・行番号）を `5877f982` 以降の develop で再確認して直す（例: §7.1 の `set_ime_open` → `set_ime_open_ordered`）。
  - (c) ADR-191 決定5の P1 行から `teardown-verification-guide.md` §7.1 へリンクする。
- [ ] **T2 強制OFFの確認手段を先に作る**（同ガイド §8-1）: `ime_key_matrix_spike` に2窓のフォーカス切替モードを足し、片方を IME ON にしてから belief OFF のままもう片方へ移り、移動後 +100/+400/+1500ms で実IMEと Engine の一致を記録する。撤去前のビルドでは、`focus_change_enforce_off` が実際に書いたか（`set_ime_open_ordered` の戻り値 `sent`、`ime_refresh.rs:603` 以降のログ）を各試行で記録する。授権が下りずに書いていない試行は、撤去前後の差がゼロでも「撤去しても影響なし」の証拠にならないため分けて数える。対象は ImmCross（CI の GJI 構成は Win32 `Edit` が入力先なのでそのまま測れる）。
- [ ] **T3 強制OFFの撤去要否を ADR-191 で決める**: T2 で撤去前後の「不一致が続く時間」を比べる。撤去するなら撤去コミットに T2 のモードを CI の構成として含める。
- [ ] **T4 drift correction（P2 の前提）**:
  - 判定側: 既存テスト（`check_drift_correction` の単体テスト、`drift_correction_replay.rs`）で足りる。追加は不要。
  - 書く側: ADR-193 の入力先を `e2e-ime.yml` に配線し（同ガイド §8-2、GJI 有効化と `awase=true/false` の対照が要る）、明示意図の回復シナリオ（§8-3）を作る。
  - **終了条件**: シナリオが ATOK/GJI で作れなければ、drift correction は撤去せず残すと ADR-191 に記録して終える。作れたら、TsfNative の ON 回復を drift 単独で代替できるかを A/B する。
  - A/B は [05](review-2026-09-24-05-startup-desired-open-forced-on.md) の修正前か後か、どちらのビルドで行うかを固定する（起動時の強制ONを直すと、起動直後の drift 発火が減るため）。
- [x] **T5 conv 軸の棚卸し**（実施済み: [conv-write-paths-inventory.md](conv-write-paths-inventory.md)。conv 軸11経路: 撤去候補A 4・例外B 6・warmup C 1）: 起点を「3関数の grep」ではなく「`modify_conv_mode` と `ActuateCmd::SetConversionMode` に到達する全経路（呼び出し元を逆に辿る）＋ conv を変えるモードキー注入」にする。上表は下限。各経路を次の3つに分類する: 決定1の線引き違反（撤去候補）／正当な例外（パニック・トレイのリセット）／撤去対象外の warmup。フォーカス変更時の eager warmup（`ime_refresh.rs:594`）も「例外として維持」の行で載せる。パニック・トレイのリセットが書く開閉（`runtime/mod.rs:1791-1792`、`message_handlers.rs:1291` 経由の `set_ime_open_for_target`）も「開閉軸・正当な例外」の行で載せる。[08](review-2026-09-24-08-open-close-fixed-set-vs-custom-keymap.md) の固定セットの結論を分類に反映する。表は `docs/tasks/actuation-confluence-inventory.md` と同じ粒度（1経路1行、根拠の行番号付き）にする。ファイル書式や分類軸（あちらは統合候補/構造上必要/ロジック共有候補）は合わせない。結果を ADR-191 決定5の指標5に加算する。
- [ ] **T6 呼び出し元の固定**: 棚卸しで経路が確定するまで lint（`RESTRICTED_CALLS`）には追加しない。ADR-191 決定5は `RESTRICTED_CALLS` の行数を「補助に留める」としており、撤去を成功基準とする ADR で許可リストのエントリを増やすのは逆向き（`complexity-budget.md` は未発効だが方向は同じ）。既存ガードで既に固定されているもの: `set_ime_open_then_conv_for_target(` は `async_imm_cross_actuation_goes_through_the_single_chain_entry`（`architecture_guard.rs:2128`）が `open_chain.rs` の1件に固定、`set_ime_conv_for_target(` は `force_write_is_not_triggered_by_raw_focus_change`（`:1874`）が `gji_on_focus_change` からの呼び出しを禁止、`set_ime_romaji_mode_for_target_blocking` は `sync_romaji_write_goes_through_a_captured_target`（`:2557`）。新たに足すのは `set_ime_conv_for_target` の呼び出し元件数ガード（同じ方式）だけに絞る。lint 化は撤去が頭打ちになってから判断する。

## 受け入れ条件

- **ドキュメント（T1・T5）**: ADR-191 に棚卸し表が追記されている。中身は開閉軸2箇所（強制OFF `ime_refresh.rs:603`、drift 補正 `:934`）、eager warmup、conv 軸の全経路で、各行に分類と撤去/維持の判断がある。docs のみの段階ではコードを変えない（`git diff --stat` が `docs/` だけ）。
- **A/B 結果の記録先**: ADR-191 の補助資料 `docs/adr/191-calibration-experiments.md`（または取り込んだ `teardown-verification-guide.md` §7.1/§7.2）。撤去を取り下げた（revert した）ときだけ、`experiment-logging` 規約に従い `docs/experiments.md` にも1行足す（同ファイルは取り下げたアプローチの記録なので）。A/B の結果は [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) A-5 で作る領域A撤去の記録にも反映する。
- **強制OFFの撤去（T2・T3）**: windows-build / e2e-ime CI で、T2 のフォーカス切替モードの撤去前後の不一致時間が記録されている（ImmCross、Linux では走らない）。撤去コミットは `fix-requires-evidence` を満たす（T2 のモードを回帰の確認として残す、または known-bugs を足す）。
- **drift correction の撤去（T4）**: 判定側の既存テスト（Linux: `cargo nextest run -p awase-windows --test drift_correction_replay` と `check_drift_correction` の単体テスト）が通る。書く側は ADR-193 の入力先を使う CI E2E、または実機（Chrome / VS Code / WezTerm 等の TsfNative 環境で実タイピング）で、明示意図の回復が撤去前と同じく働くことを確認する。シナリオが作れない場合は「撤去しない」の記録で完了とする。

## 他ファイルとの依存

- [05](review-2026-09-24-05-startup-desired-open-forced-on.md): 05 は 09 に依存しない。09 の A/B（T4）は 05 の修正の有無を前提条件として持つ（どちらのビルドで測るかを固定する）。起動時に ON を書き、ImmCross の窓へフォーカスが移ると強制OFFが OFF を書く往復は、05 と T3 の両方に関わる。
- [08](review-2026-09-24-08-open-close-fixed-set-vs-custom-keymap.md) → 09: 08 の開閉書き込み固定セットの結論を、T5 の棚卸し表の分類に反映する。
- [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md): A-5（領域A撤去の記録が ADR-179 に無い）が C-2 の「未確認」の原因。09 は 10 A-5 が ADR-179 に書く「領域A・C の撤去」節を参照し、09 の A/B 結果はその節へ戻す（双方向）。`set_ime_mode`（`ime.rs:1733`）のデッドコードは 10 の B-6 へ渡す。**要追随（09 の担当外）**: 10 B-6 には現在 `set_ime_mode` の行が無いので1行加える。10 `:155` の「09 の `related_adr: ADR-178`」は 09 側で既に外しているので 10 側で削る。
- [11](review-2026-09-24-11-low-priority-backlog.md): **要追随（09 の担当外）**: 11 `:57` の「conv 軸の書き込みは `ime.rs:1742` の `set_ime_mode_for_target` 呼び出しも含む（09）」は 09 の結論と逆。「`ime.rs:1742` は呼び出し元ゼロの `set_ime_mode` 内の委譲で独立経路ではない（09、デッドコードとして 10 B-6 へ）」に直す。

## 未確認点

- ADR-179（旧178）領域A撤去後に drift correction だけで TsfNative の ON 回復を代替できるかの実機 A/B の結果（リポジトリ内に記録なし）。
- 強制OFFを撤去したとき、ImmCross の新しい窓で「Engine OFF なのに IME ON」が観測で上書きされるまでの時間（P1 調査も未検証と記載）。
- 明示意図の回復シナリオを ATOK/GJI で作れるか。
- worktree `adr191-p1` を現在どのセッションが所有しているか。

## レビュー反映メモ（2026-09-24、Opus 批判的レビューへの対応）

指摘はすべて `5877f982` のコード・git で裏取りしてから反映した。反映しなかった指摘は無い。
- 1（強制OFFは TsfNative で発火しない）: `ime_refresh.rs:599` の条件で確認。T2/T3 と受け入れ条件を ImmCross 向けに直し、TsfNative の実機確認は T4 へ移した。
- 2（conv 7箇所の数え方）: `set_ime_mode` の呼び出し元ゼロ、`open_chain.rs:312`・`ime_controller.rs:439`・`modify_conv_mode` の3呼び出し元・`kp_restore_kana_from_half_width` の注入を確認。件数を確定値として扱うのをやめ、下限の表にした。
- 3（ADR-178 は別件）: `docs/adr/178-msi-uninstall-preserve-userdata.md` を確認。`related_adr` から外し、ADR-090（`ime_refresh.rs:600` のコメント）と ADR-193 を加えた。BUG-020/BUG-043 は本文の表で参照した（シリーズの frontmatter に BUG 用のキーが無いため）。
- 4〜6: ブランチ状態（`49638d07`、origin に無い、docs 2ファイル）とガイド §7.1/§7.2/§8 の内容を `git show` で確認して取り込んだ。
- 7〜9: ADR-191 決定5の指標3（2箇所）・指標5の列挙を本文で確認し、「3箇所」を2箇所の列挙に、「指標5に入っていない」を「warmup の1件を除き」に直した。eager warmup（`:594`）を追加した。
- 10: `lints/actuation_call_guard/src/lib.rs:98-101` で確認し、未確認点から外した。
- 11: 代案（`architecture_guard.rs` の件数ガード）を採用した。同方式の既存ガード `sync_romaji_write_goes_through_a_captured_target` があることも確認した。
- 12・13: 記録先を ADR-191 補助資料に変え、「cargo check が通る」を「`git diff --stat` が `docs/` だけ」に置き換えた。
- 14〜16: 05 の依存節（「09 の A/B 計画は 05 の修正の有無を前提条件として持つ」）と向きを合わせ、10 との双方向の依存を書いた。行番号を `:602-603` と併記し、分類表は書式ではなく粒度を合わせると書き分けた。

## レビュー反映メモ（2026-09-24、再確認レビューへの対応）

指摘 A〜G はすべて `5877f982` のコード・git で裏取りし、反映した。反映しなかった指摘は無い。
- A: merge-base `116eebdc`、develop より471遅れ・先行5、三点 diff は docs 2ファイル +436、二点 diff は 213 ファイル −37,517 行を確認。`platform.rs:1683` の `set_ime_open_ordered` と授権チェックも確認。
- B: 11 `:57` と 10 B-6 の記載を確認。どちらも担当外なので依存節に要追随として書いた。
- C: `docs/adr/index.md:186` と 10 A-5（`:57`・`:64`・`:155`）を確認。冒頭と依存節を直し、表記を「ADR-179（旧178）」にそろえた。
- D: ADR-191 本文の「### 決定2」が `:182` の BUG-151 最小修正であることを確認。
- E: `set_ime_open_ordered` が授権なしで `false` を返すことを確認し、T2 に記録項目を足した。
- F: `architecture_guard.rs:1874`・`:2128` の既存ガードを確認し、T6 の範囲を絞った。
- G: `runtime/mod.rs:1791-1792` に加え、トレイリセットの `set_ime_mode_for_target(hwnd, true, …)` も `ime.rs:1763` で開閉を書くことを確認した（レビューが挙げていない点）。08 → 09 の向きと中身も依存節に書いた。
