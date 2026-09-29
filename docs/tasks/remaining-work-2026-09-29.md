---
title: 残作業まとめ（2026-09-29時点）— 実機確認待ち・CI観測の次の一手・未起票/未修正の不具合
status: 進行中
created: 2026-09-29
related_adr: ["ADR-178", "ADR-191", "ADR-196", "ADR-200", "ADR-203"]
---

# 残作業まとめ（2026-09-29 時点）

2026-09-29 に「実機確認だけが残っているもの」を GitHub CI で確認できる範囲まで進めた結果の引き継ぎ。
**各項目の状態は、根拠（コミット・run・BUG番号）を付けて書く。根拠が無いものは「未再確認」と明記する。**

## 1. 実機（物理キー・実アプリ）でしか確認できないもの

| 項目 | 状態 | 残っていること |
|---|---|---|
| BUG-163（起動直後の強制ON） | 実装済み（`b6ab8980`・`c7999ce0`・`6f5ef659`）。CI 確認済み（develop `e174c6f6` の e2e-ime run 36506566832 で起動直後 drift 0件、GJI・MS-IME 双方）。`fix_commits` は記入済み（`c448d30a`）。 | GJI/MS-IME × メモ帳/実 Chrome の実打鍵で、最初の打鍵が欠落しないこと。IME を閉じて起動したときの体感。 |
| ADR-203 / BUG-170・171（GjiFsm の OffCold 固着） | PR #354 が develop にマージ済み（`efe66f45`）、CI 全通過。 | ADR-203 の実機シナリオ: OFF 前に1語打って Enter で確定（OnWarm にする）→ 物理 OFF → 1秒以内に物理 ON → 即打鍵。ON 後の最初の語が cold 経路になること。ON キー単独タップ直後の1語の遅延（実測30〜60ms 想定）の再測定。 |
| ADR-178 領域A撤去（reassert・force-on） | develop に撤去済み（`f83084b3`・`621bf93c`）。実機 A/B は未実施。 | 09 の A/B-2（`docs/tasks/review-2026-09-24-09-...` の「実機 A/B 手順」）。物理 Ctrl は SendInput で作れないので実機のみ。 |

## 2. ADR-178 領域A撤去後の ON 回復の観測（2026-09-29 に CI で実施済み、PR #361 `0f47bac5`）

`cal-driftrec-*`（PR #352）に加え、`--refocus`（閉じた直後にフォーカスを外して戻す、run 36530903798）と実 Chrome（`chrome_probe --close-ime=10 --msime`、run 36518453739・36524071258）で測った。詳細と表は `review-2026-09-24-09-...` の「追補 2026-09-29」。

- **結論**: 閉じられた IME を drift correction が ON へ戻す経路は、フォーカス変更を挟んでも成立しない。
  - Chrome 系クラスは FocusChange 後も `profile=Imm32Unavailable`（`Skipping IMM query for known-broken class`）で開閉を観測せず、判断に届かない（observed=0）。
  - ImmCross（edit）は観測が届くが、フォーカス変更で `explicit_intent=None` になり、閉じた状態を新しい belief として採用する（設計どおり、drift=0）。
  - ON へ戻るのは GJI reinit だけ。RichEdit の tsf × GJI は 30/30 で回復（refocus 有りも 10/10）、**実 Chrome × GJI は 0/10**（打鍵1回のみの観測）。
- **反映済み**: 開閉軸の表へ GJI reinit の行、C-2 の但し書き、BUG-172 起票済みの記述。
- **残り**:
  - 実 Chrome でのフォーカス変更: `chrome_probe --refocus` は `SetForegroundWindow`/`SwitchToThisWindow` がタスクバーに拒否され `away=false`（フォーカス変更は起きていない）。別窓を作って前面にする方式に変える。
  - 撤去前ビルド（`f83084b3`・`621bf93c` の前）との対照は未実施。
  - 外部から IME を閉じられるケース（他アプリ・OS による IME OFF）の実運用での頻度の判断。頻度が低ければ対処しない選択もある。
  - edit 構成は Unicode 注入で打鍵結果が開閉の証拠にならない（VK 注入になる入力先が要る）。
- 実行: `gh workflow run e2e-ime.yml --ref <branch> -f only='cal-driftrec-*'`（`cal-driftrec-refocus-*`・`cal-driftrec-chrome-*` も同様。cal-* は only 指定時だけ走る。観測のみで合否には含めない）。

## 3. 未修正の不具合

- **BUG-172**（`docs/known-bugs/BUG-172.md`）: CI の RichEdit 入力先（TsfNative 相当）では、msime-ready ゲートが conv の NATIVE を「ON 確認」と扱い `ka` が生ローマ字で入る（30/30）。**実 Chrome では別経路と確認**（2026-09-29、run 36524071258）: 症状は出る（9/9）が msime-ready ゲートは経由せず、原因は Imm32Unavailable による観測不能。ゲートへ開閉を要求する修正は実 Chrome の症状を直さず、TsfNative では開閉が信頼できないため**見送り**。実 Chrome の症状は上の「2.」の残りとして扱う。
- **ts-chrome 高速打鍵（BUG-168 / ADR-200）**: 修正は develop にマージ済み（PR #334、CI 実 Chrome 2ms 1,440試行で失敗0）。BUG-168 の frontmatter は「修正済み・実機/CI 確認待ち」。残り（記録: 2026-09-26 のメモ、**未再確認**）: 候補窓が残ったまま GJI が OFF のときの回復低下（ADR-200 のリスク）、StaleConfirm の romaji 再送重複（BUG-075 系）、Escape 経路、他の reinit 呼び出し元、起動直後の IME モード不整合と awase 主スレッド7秒停止（未解明）。

## 4. MS-IME 本体の学習（ADR-196 T2）— 記録は 2026-09-24 時点、未再確認

- 当初の「conv 0x0001 未対応で失敗」は**古い情報**: 原因は CI の言語設定（ja-JP のみ＋MS-IME TIP に直すと学習は完走）。PR #293/#294/#296 が develop にマージ済み。
- 残り: 半角カタカナ（conv 0x0013）が学習モデルの Conv に無く復号失敗（decode_errors あり）。MS-IME の正答率のばらつきは隠れ状態（入力中 BS/Enter、英数の巡回、閉状態からの再オープンで直前モードへ復帰）が原因と判明（`docs/tasks/adr196-t2-msime-hidden-state-hypothesis.md`）。**要確認→`--adopt-pending-judgement` の採用経路は、精度≥0.95でないと通らず未検証**。

## 5. 後片付け・運用メモ

- 残っている作業ブランチ/ワークツリー: リモート `ci/adr178-tsfnative-on-recovery`（#352 マージ済み）、ワークツリー `/home/cuzic/rust-nicola-wt/ci-verify-remaining`。BUG-172 用の `fix/bug172-msime-ready-open-check`・`ci/bug172-chrome-close-ime` は #361 マージ後に削除済み。削除は使用者の確認後（`worktree-per-session.md`）。
- v1 ラインへの backport 要否は未確認（`main-develop-branch-flow.md`: 修正は develop で先に直し `v1-develop` へ backport。BUG-168・170・171 等が対象になるかは未判断）。
- BUG-170/171 は別ブランチで採番されていたため、今回の BUG-172 は衝突を避けて 172。新規採番前に他ブランチの `docs/known-bugs/` を確認する。
- ワークフロー実行の落とし穴: `workflow_dispatch` は develop 以外のブランチでも `--ref` で指定して使える。cal-* と ts-* は `only` 指定時だけ走る。`ImmSetOpenStatus` は別スレッドから呼ぶと失敗するので、外部からの開閉操作は既定 IME ウィンドウへの `WM_IME_CONTROL` にする。
