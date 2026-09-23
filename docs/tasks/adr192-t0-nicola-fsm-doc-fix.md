# ADR-192 T0: `nicola_fsm.rs:858-867` のdoc矛盾を解消する（決定3b着手前の前提）

状態: 未着手（2026-09-22起票）
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md)（rev8、
opus-adversarial-consult 7ラウンドで収束済み）の決定3bは、`src/engine/nicola_fsm.rs`に
新しい合流点（単独打鍵確定時の強制ON/OFF、優先順位1.5）を追加する。この着手前に、同ファイル
内の既存docコメントの矛盾を解消しておく必要がある（ADR-192 round1 A-2、決定3b本文の
「前提コードのdoc矛盾」節参照）。

`nicola_fsm.rs:858-864`は「明示config（`*_solo_tap_ime_action`等）を持つキーもKeyUp解決の
対象にする（ADR-186）」と書いているが、`:867`の除外リストと実際のコード（`:879`）は明示config
持ちのキーを**除外**している。ADR-186本文（`docs/adr/186-gji-atok-mode-key-measured-matrix-and-belief-follow.md:117-126`）もdelegate（ADR-191が撤去済み）についてしか述べていない。
このdoc上の矛盾は、ADR-191のdelegate撤去（コミット`983a6bdf`）で陳腐化した記述だと判断できる
（`git log -L 877,886:src/engine/nicola_fsm.rs`で確認済み: `973b0389`→`2b93e185`→`983a6bdf`
の変遷）。

放置すると、次にこのコードを読む実装者（本タスク自身、またはADR-192 T4の実装者）が
「明示config持ちは既にKeyUp解決される」と誤読し、ADR-186が実測した「タイマー解決は
`Unwarranted`で握り潰される」という失敗（実機3/3 FAIL）を再現しうる。

## やること

1. `src/engine/nicola_fsm.rs:858-864`のdocコメントを、実際の挙動に合わせて訂正する:
   「明示config（`*_solo_tap_ime_action`等）を持つキーは`resolve_explicit_ime_action`
   （タイマー解決）のまま。ADR-192の新しい合流点（優先順位1.5、決定3b）だけがKeyUp解決を持つ」
   という趣旨に書き換える。ADR-186のdelegate撤去（ADR-191）の経緯も一言残す。
2. 挙動そのものは変えない（ドキュメントのみの修正）。`cargo test --lib`で既存テストが
   全て通ることを確認する。
3. 純粋なdocs修正なので[fix-requires-evidence.md](../../.claude/rules/fix-requires-evidence.md)
   のテスト/記録要件は適用対象外（挙動が変わらないため）。

## 完了条件

- `src/engine/nicola_fsm.rs:858-864`のコメントが実際のコード（`:867`, `:879`）と矛盾しない。
- `cargo test --lib`が通る。
- develop へマージ済み（このタスクは[ADR192-T4](adr192-t4-thumb-solo-forced-ime-toggle.md)
  の着手前に完了させること）。

## 関連

- [ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md) 決定3b
  「前提コードのdoc矛盾」節
- [ADR-186](../adr/186-gji-atok-mode-key-measured-matrix-and-belief-follow.md)
- [ADR192-T4](adr192-t4-thumb-solo-forced-ime-toggle.md)（このタスクの後続、着手前提）
