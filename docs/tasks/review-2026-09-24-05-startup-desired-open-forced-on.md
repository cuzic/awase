---
title: 起動直後の desired_open=true 初期値による drift correction の強制 IME ON（新規BUG起票が必要）
status: 未着手
created: 2026-09-24
related_adr: ["ADR-191", "ADR-187", "ADR-098"]
source_review: 俯瞰レビュー（2026-09-24）の B-5（起動時の強制ON部分）
---

# 起動時の desired_open=true による強制ON（俯瞰レビュー B-5 前半）

索引: [11](review-2026-09-24-11-low-priority-backlog.md)。残りの B-5（フォーカス変更時の強制OFF、drift correction 本体、conv 軸）は [09](review-2026-09-24-09-remaining-active-writes-inventory.md)。裏取り基準は `5877f982`（origin/develop）。`cbae84ff` 以降の差分（PR #293〜#296）は keymap-learn と `KEY_EFFECT_SETTLE_MS` だけで、本件の関係箇所は変わっていない。

## 現状（裏取り済み）

- 初期値: `crates/awase-windows/src/state/ime_model.rs:309` の `ImeModel::new()` は `desired_open: true`。doc コメントの根拠は「既存 `ImeBelief` の初期値 (`ime_on=true`) に合わせる」。しかし現在の `ImeBelief`（`state/belief.rs:31-46`）には `ime_on` フィールドが無い（残っているのは `is_japanese_ime` と `prev_conversion_mode`）。`belief.rs:28` にも「IME ON/OFF 自体は `ImeModel` の `desired_open` が SSOT」とある。**`true` を初期値にする根拠は、コード上にもう残っていない。**
- 判定: `state/platform_state.rs:1091` の `check_drift_correction` が、`desired_open` と最新の信頼できる観測を比べる。抑止条件は次の2つだけ。
  - 観測源が `ConvOpenInference` または `HeuristicDefault`、かつ `explicit_intent.is_none()`（1162-1168行）。
  - `trusted.open == desired`（1169行）。
  - したがって、起動直後に `ImmCrossProbe` などの実読み取りで「閉」が観測されると、明示意図が無くても初期値 `true` と比べられ、補正が発火する。
- 送信: `runtime/ime_refresh.rs:645` の `ir_apply_drift_correction` が書き込む。ログは `ime_refresh.rs:884` の `[drift] correction: observed=… ≠ desired=…`。
- 記録の欠落: `docs/known-bugs/BUG-157.md:11-13` に次の記述がある。起動直後に awase の想定（desired=true）と実 IME（閉）が乖離し、`[drift] correction ... set_ime_open(true)` が約11秒間に22件出る。これは「develop 版の CI にも同数ある（起動直後の既存の過渡現象、本件ではない）」とされている。この現象は **どの BUG としても追跡されていない**。
- 意味: IME を閉じた状態で awase を起動すると、awase が IME を開けに行く。これは ADR-191 決定1（IME が状態の正で、awase は書かない）に反する。
- 22件の解釈（推測。ログでの確認は未実施）
  - 約11秒に22件なので、約500ms 間隔で再送されている。`DRIFT_CORRECTION_THRESHOLD_MS=400`（`tuning.rs:276`）程度の間隔である。
  - つまり CI では書き込みが効かず、観測が true に変わらなかった可能性が高い。
  - 約11秒で止まったのは、CI ハーネスが起動時に `VK_IME_OFF` を注入して `desired=false` にしたためと読める（BUG-157 の「起動時のVK_IME_OFF(desired=false)の後」）。11秒は現象本来の長さではない。
  - 実機で書き込みが効く窓では、起動するとすぐ「IME が勝手に開く」ことになる。効かない窓では、ユーザーが何か操作するまで再送が続きうる。
  - 元レビューの推奨5は「最大22回」と書いているが、「最大」には根拠が無い（CI の打ち切り条件で決まった数）。BUG を起票するときにこの表現を写さないこと。
- 同じ型で解決済みの前例: `docs/adr/191-calibration-experiments.md` の追補3「未解決」と追補4（248-260行付近）。問題の型は「`desired_open` がどの観測にも揃わないまま drift correction が書き戻し続ける」で、本件と同じである。
  - 追補4 では `should_align_after_expired_mode_key_pass`（`state/mode_key_pass.rs:252`、純関数。Linux でテストが走る）と `align_after_expired_mode_key_pass`（`state/platform_state.rs:366`）を導入した。
  - 仕組みは、最初の成功観測で `ImeEvent::ModeKeyPassedThrough { align_desired }`（`ime_model.rs:873`）を1回 dispatch し、`observations.derive_any` の値に `desired_open` を揃えるというもの。
- 初期値の他の用途: `effective_open()`（`ime_model.rs:423`）は、観測が一切無いとき `desired_open` にフォールバックする（doc の3番目の規則）。`resolve_warmup_ime_on`（`platform_state.rs:806`）も `effective_open()` を使う。つまり「読めない窓では Engine を ON で始める」挙動もこの初期値に依存している。
- 規模: `.desired_open()` の呼び出しは `crates/awase-windows/src` に20箇所ある（`grep -rn "\.desired_open()"`。レビューの「31箇所」とは数え方が違う）。

## タスク

- [ ] 新規 BUG を起票する。
  - 番号: `docs/known-bugs/index.md` の末尾は BUG-162（166行）。全 ref の `docs/known-bugs/` に BUG-163 以降のファイルは無い（`for r in $(git for-each-ref --format='%(refname)'); do git ls-tree -r --name-only $r -- docs/known-bugs; done | grep BUG-16[3-9]` が0件）。`git log --all --oneline -S"BUG-163" -- docs/known-bugs` も0件なので、次の連番は **BUG-163** の見込み。
  - ブランチを統合したときに BUG 番号が衝突した過去例があるので、起票の直前に同じコマンドで再確認する。
  - 書式は frontmatter（`id`/`title`/`fix_commits`/`related_adr`）と本文30行以内。
  - 本文書の作成時点では起票していない。BUG-NNN.md と index.md の行を追加した時点で、この項目は完了とする。
- [ ] 22件の中身を CI ログで確認する。対象は BUG-157 の「検証」節にある run 35620809258（`ci/e2e-drift-fix`）の `real-sc-dbe` の awase.log。
  - 起動から最初の `VK_IME_OFF` までに出た `[drift] correction` を数える。
  - 観測元（source/confidence）を確認する。
  - 書き込みが収束（Confirmed）したか、再送が続いたかを区別する。
  - CI の既存シナリオ（`real-*`）でこの現象が見えるなら、実機確認より先にこちらで済ませる。
- [ ] 実機で利用者への影響を確認する。
  - 手順: IME を閉じた状態で awase を起動する。GJI と MS-IME それぞれについて、メモ帳（ImmCross、読める窓）と Chrome（TsfNative）を前面にしておく。30秒放置する。
  - 記録: `[drift] correction … set_ime_open(true)` の件数、Confirmed になったか、再送が上限で打ち切られたか。そのうえで実際に打鍵し、IME が開いているかを見る。
  - TsfNative では API の読み取り値だけで判断せず、実際の打鍵結果で確認する（過去に API の読み取り値を信じて誤判断した例がある）。
- [ ] 修正方針を決める。**原則は代案A**（下記「設計案」）。代案Aでは型も初期値も変えないので、ADR は起票せず、ADR-191 calibration-experiments への追補か BUG 本文への記録で足りる見込み。
- [ ] 代案Aを採る場合、`effective_open()` のフォールバック（観測ゼロのとき）と `resolve_warmup_ime_on` の起動直後の値が変わらないことを確認する。揃えるのは成功観測があったときだけで、読めない窓では `desired_open` に触れない。

## 設計案

- **代案A（推奨）: 追補4 の揃え機構を起動直後にも適用する**
  - 起動後、awase がまだ IME に書いておらず（`record_optimistic`/`record_confirmed` が無い）、明示意図（`last_intent`）も無い間に最初の成功観測が来たら、`ModeKeyPassedThrough { align_desired: true }` と同じ reducer 経路（`observations.derive_any`）で `desired_open` を観測値に1回だけ揃える。
  - 判定は `should_align_after_expired_mode_key_pass` と同じ形の純関数にし、Linux でテストする。
  - 既存の `ModeKeyPassedThrough` をそのまま流用するか、専用の `ImeEvent` 変種を足すかは、`tests/architecture_guard.rs` の dispatch 元検査・アーム本文検査と合わせて判断する（追補4 は dispatch 元を `pass_through_observed` の1箇所に固定している）。
  - 型と初期値を変えないので、`desired_open()` の20箇所の読み手には影響しない。
- **代案B: `check_drift_correction` の抑止条件を広げる**
  - 「明示意図が一度も無く、awase もまだ書いていない」間は、信頼できる観測でも補正しないようにする。
  - 単独では不十分である。`desired_open=true` が残るので、後で別の判定（warmup の ON 方向など）に使われうる。
- **代案C（原案、非推奨）: `desired_open` を `Option<bool>`（未知）にする**
  - 20箇所の読み手と `effective_open()` のフォールバックのすべてで「未知のときどうするか」を決める必要があり、変更が大きすぎる。
  - 検討する場合は、ADR-098 決定1-b（未知と確認済み false を bool に潰さない。`applied_open()` の doc が警告している罠）を先例として参照する。
  - 型を変えるなら ADR を起票する。

## 受け入れ条件

- ドキュメント: BUG-163（仮）が `docs/known-bugs/` と index.md に追加され、症状・再現手順（CI の run と シナリオ名）・修正時は修正コミットが記録されている。
- 修正時のテスト（どこで走るかを区別する）
  - (a) **Linux で走る**: 揃えるかどうかを決める純関数の単体テスト（起動直後・未書き込み・明示意図なし・成功観測ありなら揃える。awase が書いた後や明示意図の後は揃えない。1回だけ揃える）。加えて、`ImeModel` の reducer 単体で「揃えた後は `desired_open` が観測値と一致する」ことをテストする（`state/ime_model.rs` は Linux でもコンパイルされる）。`cargo test -p awase-windows --lib` で実行する。
  - (b) **windows-build CI で走る**: `platform_state` のテスト「起動直後に IME 閉の成功観測 → `check_drift_correction` が `None` を返す（`set_ime_open(true)` の補正が発行されない）」。`state/mod.rs:164-165` のとおり `platform_state` は `#[cfg(windows)]` なので、Linux のテストバイナリには存在しない。ローカルでは `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib` で型検査だけ行う。
  - `ImeModel` 自体は actuation を発行しない。「actuation が発行されない」ことは (b) で確認する。
- CI: 上記 run と同じ `real-*` シナリオで、起動から最初の `VK_IME_OFF` までの `[drift] correction ... set_ime_open(true)` が0件になる。
- 実機: 「IME 閉で起動 → 30秒後も IME は閉のまま、打鍵が直接入力になる」ことを、GJI/MS-IME × メモ帳/Chrome で確認する。

## 他ファイルとの依存

- [09](review-2026-09-24-09-remaining-active-writes-inventory.md)（05 は 09 に依存しない。09 の A/B 計画は 05 の修正の有無を前提条件として持つ）
  - drift correction 本体（`ir_apply_drift_correction`）は同じ経路なので、09 の A/B 計画と合わせる。
  - フォーカス変更時の強制 OFF（`runtime/ime_refresh.rs:599-609` の `focus_change_enforce_off`。条件は `!applied_ime_on && !new_profile_is_tsf_native`）との相互作用に注意する。現状では、起動時に ON を書き、非 TsfNative の窓へフォーカスが移ると OFF を書く、という往復が起きうる。05 だけを先に直すと中間状態の挙動が変わるので、「起動直後からフォーカス変更まで」と「その後の OFF 強制」を A/B で確認する。
- [10](review-2026-09-24-10-adr-status-and-stale-docs-sync.md) B-8（pre-push の対象正規表現）
  - 代案A/B で触る `state/platform_state.rs` は、`fix-requires-evidence.md` の IME belief 行にも、pre-push の正規表現 `state/(ime|conv_mode|observation_store)` にも該当しない。この正規表現は、実行される `.git/hooks/pre-push:28` と追跡下の `.githooks/pre-push:36` の両方に共通する。
  - そのため `platform_state.rs` だけを変更しても pre-push は警告を出さない。テストか BUG 記録を必ず同じコミットに含める。対象への追加を 10 で扱うかどうかは 10 側で判断する。
- `fix-requires-evidence`（IME belief 再発ファミリー）の対象になる。`state/ime_model.rs` を触る場合は pre-push で検知される。

## 未確認点

- 起動時の drift correction が、読める窓（ImmCross）の実機で実際に IME を開けてしまうか。BUG-157 が記録しているのは CI ログの件数だけ。
- 22件が「書き込みが効かず再送が続いた」結果なのかどうか（上記「22件の解釈」は推測）。ImmCross の Read ポリシーが無制限に再送するのか、上限に当たるのかも未確認。
- 代案Aで `ModeKeyPassedThrough` を流用したとき、`architecture_guard` の dispatch 元の件数ガードに抵触するかどうか。

## レビュー反映メモ（2026-09-24、Opus 批判的レビューへの対応）

- 反映した指摘
  - 1-1: `ImeBelief::ime_on` の消失。
  - 1-2: 判定経路と送信経路の2関数、抑止条件の抜け。
  - 1-3: 22件の解釈。推測として明記し、確認タスクを追加した。
  - 1-4: ADR-098 を代案Cの先例として位置づけた。
  - 2-1: 追補3・追補4。related_adr に、`ModeKeyPassedThrough` の出所である ADR-187 を追加した。
  - 2-2: 「最大22回」を写さない注記。
  - 2-3: `focus_change_enforce_off` との往復。条件 `!new_profile_is_tsf_native` を実コードで確認して併記した。
  - 3-1: テストを Linux / windows-build CI の2段に分けた。
  - 3-2: 実機手順の具体化。
  - 3-3: 起票タスクの矛盾を解消した。
  - 3-4: 再現条件を run 35620809258 に置き換えた。
  - 4-1: 代案A を推奨にした。
  - 4-2: `effective_open`/warmup への依存を確認するタスクを追加した。
  - 4-3: pre-push の検知漏れ。
  - 5: 記憶ファイル名の参照を外し、コマンドを完全な形にした。
- 訂正・一部不採用
  - 呼び出し箇所数: レビューの「`desired_open()` 呼び出し元31箇所」は再現できなかった。`grep -rn "\.desired_open()" crates/awase-windows/src` では20件だった。20件と書き、数え方の違いとして扱った。
  - pre-push の正規表現の行: レビューは `.githooks/pre-push` だけを前提にしていた。実行される `.git/hooks/pre-push`（メイン作業ツリー側、28行）も同じ `state/(ime|conv_mode|observation_store)` であることを確認して併記した。なお `runtime/ime_refresh.rs` は `.githooks` 側（36行）にだけ含まれ、実行される側には含まれない。
  - BUG-151 型の回帰: レビューは「起動直後の Engine の固まり」と表現していた。BUG-151 は「cold 状態で Engine が OFF にならない」件なので、本文では BUG 番号を挙げず、「`effective_open` のフォールバック値を変えないこと」の確認に留めた。
- 解消済みの有無: `cbae84ff`→`5877f982`（PR #293〜#296）に本件の修正は含まれない。解消済みの項目は無い。
