# ADR-195 前提: `feat/awase-calibration`ブランチをdevelopにrebaseする

状態: **実質的に完了済み（2026-09-23、横断レビューM7）。ただし判断3（下記）は実際には
「後続タスクのPRに含める」が7回繰り返され、単体PR化されなかった**——PR #250〜#257の
うちT0を除く6本が、それぞれ独立に`awase-keymap-learn`/`-win`クレート一式（約4,300行、
17〜25コミット）を含んでいる。さらにT6（PR #255）だけ最初のコミットハッシュが
（`37034be9`、他は`f6101e49`）異なり、同じクレートが2系統に分かれてrebaseされている。
**今後この状況を解消する場合**（マージ順序の整理、または土台の1本化）は、このタスク文書を
その作業記録として更新すること。新規に一からrebaseし直す必要はない。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-195](../adr/195-keymap-learn-productization.md)（rev7、opus-adversarial-consult
round6で「収束、実装可、Blockerゼロ」判定済み）が定める`awase-keymap-learn`/
`awase-keymap-learn-win`クレートは、`feat/awase-calibration`ブランチ
（worktree: `rust-nicola-worktrees/adr191-calibration`）に既に存在する（巡回プランナ・
シミュレータ・`RealImeDriver`によるWin32/TSF実機観測・注入）。ただしこのブランチは
2026-09-23時点でdevelopから約23コミット遅れており（ADR-191撤去〈PR #240〉やADR-192の
コミット群を含まない）、[ADR195-T1](adr195-t1-independent-learning-process.md)以降の
どのタスクもこのブランチの上で作業することになるため、最初にrebase/マージしてdevelop
最新に追随させる必要がある。

## 実装対象

1. `feat/awase-calibration`を`develop`最新へrebase（またはdevelopからの新規worktreeへ
   該当クレート一式をcherry-pick）する。どちらの方式でも、ADR-191撤去後のAPI変化
   （`key_effect_table.rs`/`key_effect_predictor.rs`の現行シグネチャ）に実装が追随して
   いることを確認する。
2. rebase後、`cargo check --target x86_64-pc-windows-msvc -p awase-keymap-learn
   -p awase-keymap-learn-win`（および`awase-keymap-learn`は`cargo test`がLinuxで
   走ることを確認、OS非依存が前提のため）が通ることを確認する。
3. rebase自体はコード変更を伴わない整理作業なので、単体でPR化して先にdevelopへ載せる
   か、後続タスク（T1等）のPRに含めるかは着手時に判断してよい。

## 完了条件

- `feat/awase-calibration`（またはその後継ブランチ）がdevelop最新をベースにしている。
- 上記のビルド確認が通る。

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md)
- [ADR195-T1](adr195-t1-independent-learning-process.md)（このタスクの直後に着手）
