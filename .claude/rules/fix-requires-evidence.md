# fix には「テスト」か「記録」を添える

## ルール

以下の **再発ファミリー** に触れる `fix` コミットは、次の (a)(b) の少なくとも一方を
同じコミット（または直後の追随コミット）に含めること。

- **(a) 回帰テスト**を追加する — golden / ジャーナルリプレイ / characterization の
  いずれか（下記「テストの置き場所」）。
- **(b) [docs/known-bugs.md](../../docs/known-bugs.md)** に、症状・再現手順・修正履歴
  （コミットハッシュ）を追記する。

### 再発ファミリー（このルールが効く領域）

これまで同種のバグが何度も再燃してきた領域。ファイルの目安:

| ファミリー | 主なファイル |
| --- | --- |
| warmup / cold-start | `output/tsf_warmup_coord.rs`, `output/probe_io.rs`, `tsf/`, `output/ime_apply_planner.rs`, `tuning.rs` |
| focus 遷移 | `focus/`, `runtime/focus_tracking.rs` |
| IME belief | `state/ime_model.rs`, `state/observation_store.rs`, `runtime/ime_coordinator.rs`, `focus/uia.rs`, `focus/msaa.rs` |
| conv mode | `state/conv_mode.rs`, `focus/classify.rs` |
| キー選択（IME ON/OFF に送る VK） | `ime_controller.rs`, `output/vk_send.rs` |

## テストの置き場所（このリポジトリの既存資産）

- `crates/awase-windows/tests/ime_key_sequence_golden.rs` — 戦略選択（ImmCross →
  GjiDirect → MsImeDirect → KanjiToggle）と送信キー列の golden。キー選択を変える
  fix はここに期待値を足す。`ime_controller.rs::characterize_strategy` が SSOT。
- `crates/awase-windows/tests/golden_scenarios.rs` と `crates/awase-windows/tests/golden/` —
  シナリオ golden。
- `crates/awase-windows/tests/e2e_windows.rs` — Windows 実機経路の e2e。
- ジャーナルリプレイ基盤（`journal.rs` 起点、整備中）— `classify_*` 純粋関数への
  入力列を記録・再生して belief/conv 遷移を回帰させる。純粋判定を変える fix はここが最適。

Linux で `cargo test -p awase-windows` から実行できるもの（golden / architecture_guard /
layer_boundary_guard 等）を優先する。実機依存で自動化できない場合は (b) の known-bugs.md
追記で代替する。

## なぜこのルールが必要か（背景）

warmup・focus・belief・conv・キー選択の 5 領域は、実機の組み合わせ依存が強く、
「直したつもり」が別の環境で再発する。実例:

- IME OFF キー選択は 5 日間で 6 回反転した（[docs/experiments.md](../../docs/experiments.md)、
  `534051a`〜`489cdf1`）。golden（`ime_key_sequence_golden.rs`）があれば、キーを変えた
  瞬間に「Chrome では受け付けない `VK_IME_OFF` に変えた」等の退行を CI で検知できる。
- Chrome cold-start のリテラル化（`b101153` / `79134f5` / `3c275a7` …）は known-bugs.md
  の BUG-02 に修正履歴が積まれており、次の担当者が「probe 起点のズレが真因で、値を
  上げるのは対症」という過去の知見にすぐ辿り着ける。

「fix コミット単体」は、それが**何を再発させないためのものか**を残さない。テストは
機械可読な再発防止、known-bugs.md は人間可読な再発防止であり、どちらか一方は必ず要る。

## 自動チェック（pre-push）

`.git/hooks/pre-push` に軽量チェックを入れてある。上表の対象ファイルが変更されている
push で、`crates/awase-windows/tests/` にも `docs/known-bugs.md` にも差分が無い場合、
**警告を出す（ブロックはしない）**。golden の期待値更新や known-bugs 追記を忘れていないか
の気づきを与えるのが目的。意図的にテスト/記録が不要な変更（純粋なリファクタ等）は
そのまま push してよい。

関連: [experiment-logging](./experiment-logging.md)、[tuning-constants](./tuning-constants.md)、
[ime-belief-architecture](./ime-belief-architecture.md)。
