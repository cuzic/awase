# ADR-192 T3: awase-settingsでの案内・1操作書き込み（決定3）を実装する

状態: 未着手（2026-09-22起票、[ADR192-T2](adr192-t2-warning-detection-and-dispatch.md)完了後に着手）
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md)（rev8）
決定3は、T2が検出した状態依存キーを、新機構を作らず既存のユーザー明示config
（`keys.ime_on`/`ime_off`/`ime_toggle`、`*_solo_tap_ime_action`）へ置き換える案内を
awase-settingsで行う。

## 実装対象（詳細・根拠はADR-192決定3本文参照）

1. **検出結果の表示**: T2の警告結果（該当キー・軸・根拠）をawase-settingsに表示する。
2. **1操作の置き換え**: 「このキーを冪等なIME ON/OFFに置き換える」ボタンで`config.toml`
   （`keys.ime_on`/`keys.ime_off`）を書く。プレビューと元に戻す操作をつける。
3. **`*_solo_tap_always_suppress`の同時整合（ADR-192 round1 F-1、重要）**: 親指キー単体を
   対象にする場合、1操作の書き込みは`*_solo_tap_ime_action`だけでなく
   `*_solo_tap_always_suppress = true`（`ModeKeyConfig`、`awase-settings/src/main.rs`が
   既に露出）も**同時に**揃える。揃えないと、`always_suppress = false`（ADR-153以前からの
   既定・legacy設定のユーザーが大半）のユーザーでは書いた設定がM13で無言で無効化される。
4. **抑止専用の記法は新設しない**: `*_solo_tap_always_suppress = true`
   （`ModeKeyConfig{idle: Suppress, composing: Suppress}`、`src/config.rs`の既定値）が
   既に「キーを無効化するだけ」の記法として使えるため、これ以上の新記法は検討不要
   （ADR-192 round1 F-2で確認済み）。

## 完了条件・テスト

- `cargo test -p awase-settings`（Linuxで走る、bin-onlyクレートだが`cargo test`可能）に
  単体テストを追加: 1操作の書き込みで`config.toml`が期待どおり書かれること（
  `*_solo_tap_always_suppress`の同時書き込みを含む）、元に戻せること。
- 実機での確認（awase-settingsのUI操作）は本タスクのスコープに含める場合は明記し、
  含めない場合は次のフォローアップとして`docs/tasks/`に残すこと。

## 関連

- [ADR-192](../adr/192-state-dependent-mode-key-warning-and-guided-override.md) 決定3
- [ADR192-T2](adr192-t2-warning-detection-and-dispatch.md)（前提）
