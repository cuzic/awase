# ADR-192 T5: 学習表採用後の状態依存キー警告の整合性を取る

状態: 未着手（2026-09-23起票、ADR-196非目的S2からの後続課題として記録）。
[ADR196-T2](adr196-t2-mismatch-adjudication.md)（学習表の採用）着手後に着手。
**【2026-09-24追記】前提のT2は「内蔵表への参照経路」(PR #263)・採否判定の配線(PR #269)・再測定(PR #275)までdevelop統合済みのため、着手可能。**
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-196](../adr/196-keymap-learn-truth-priority.md)非目的S2（`196-...md:80-82`）が指摘した
ギャップ: [ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md)の
警告判定`classify_state_dependent_mode_key`（`key_effect_table.rs:505`、`predict()`を経由
せず内蔵表のセルを直接横断する）は、学習表が採用された後も**内蔵表を見続ける**。このため、
予測器（学習表）が「冪等」と見ているキーを、ADR-192の警告が「状態依存」と表示する（または
その逆）という食い違いが起こりうる。ADR-196本文はこれを範囲外とし、本タスクとして
ADR-192側に記録することを推奨している。

## 実装対象

1. `classify_state_dependent_mode_key`の入力を、内蔵表の直接参照から、学習表が採用されて
   いる場合はそちらを優先する形に変える（`KeyEffectKeymap::predict`経由、または同等の
   抽象化）。
2. 学習表とADR-192の警告判定の間で使う「表の出所」を揃える（[ADR196-T2](adr196-t2-mismatch-adjudication.md)
   の実装対象0「内蔵表の参照経路」と同じ仕組みを再利用できないか検討する）。

## 完了条件

- 学習表採用時、ADR-192の警告が学習表の予測結果と一致することのテスト。
- 学習表未採用（内蔵表のまま）のときの既存挙動が変わらないことの回帰テスト。

## 関連

- [ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md)
- [ADR-196](../adr/196-keymap-learn-truth-priority.md) 非目的S2
- [ADR196-T2](adr196-t2-mismatch-adjudication.md)
