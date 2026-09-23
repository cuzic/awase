# ADR-195 T6: ADR-176較正ウィザードとの統合（段階6）を実装する

**【ADR-196で一部置換】実装対象5「同梱表と同じ構成なら学習を勧めない」は、
[ADR-196](../adr/196-keymap-learn-truth-priority.md)決定2（構成に関わらず学習ボタンを
同じ導線で案内し、状態表示1行だけを変える）に置き換わった。実装対象1〜4（子プロセス
起動・進捗表示）はそのまま有効。新しいUI方針は[ADR196-T4](adr196-t4-ui-status-and-adoption.md)
を参照。**

状態: **developマージ済み（2026-09-23、PR #258〈T9〉経由。元PR #255はsupersededで
クローズ済み、詳細は[adr195-remaining-work-2026-09-23.md](adr195-remaining-work-2026-09-23.md)
参照）。実装対象5〈同梱表一致時は学習を勧めない〉は上記の通りADR-196決定2に置換されたため
未着手のまま。**[ADR195-T1](adr195-t1-independent-learning-process.md)
完了後に着手（起動対象となる学習プロセスが必要）。ADR-176は既にdevelopマージ済み。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-195](../adr/195-keymap-learn-productization.md)決定・段階6は、ADR-191が撤去済みの
「較正結果を適用する」という独立動作モードの代わりに、**ADR-176の較正ウィザード
（awase-settings、UI導線）から、T1の独立学習プロセスを子プロセスとして起動できるように
する**ことだけを指す。

## 実装対象

1. **起動**: awase-settingsが学習プロセス（`awase-keymap-learn-win`）を子プロセスとして
   起動する。
2. **対象プロセスの一時停止は不要**: 学習プロセスの実行ファイル名は固定のため、
   awase.exe側はT1で実装したコード内定数照合（`is_keymap_learn_process_name`）で
   恒久的に無効化している。起動・終了のたびにawase.exeへ何かを要求する必要はない
   （動的バイパス要求・keepalive・タイムアウトいずれも無し）。学習中もawase.exe自体は
   動き続け、他の窓で通常どおりNICOLA入力できる。
3. **進捗は子プロセスの標準出力で運ぶ**: awase-settingsは学習プロセスの標準出力
   （進捗〈現在何セル目/推定残り時間〉と成否だけ、表本体は運ばない）を読んでUI表示する。
   IPC（`calibration_ipc.rs`）は使わない（ペイロードが1ワード固定で表本体を運べない）。
4. **反映は次回のfsスタンプ再チェック時**（`KeymapCache`と同じ`RECHECK_MS`相当の遅れを
   許容する）の一本に倒す。「適用」という別のユーザー操作は不要。
5. ~~**同梱表と同じ構成なら学習を勧めない**（[ADR195-T4](adr195-t4-runtime-loading.md)の
   受け入れ基準と対になるUI方針）: 検出したキーマップ構成が同梱の3種
   （ATOK/GJI+MS-IMEプリセット/Microsoft IME本体、いずれもカスタム設定なし）と一致する
   場合、awase-settingsは学習の実行を積極的に案内しない（実行自体は妨げないが、既定の
   導線に出さない）。~~ **【ADR-196で置換】** 実装しないこと。
  [ADR196-T4](adr196-t4-ui-status-and-adoption.md)実装対象1「構成に関わらず同じ位置・
  同じ強さで表示する」を参照。

## 実装対象外（配布関連、[ADR195-T7](adr195-t7-safety-measures.md)参照）

学習プロセスの.exe同梱・署名・アンインストール時の扱いはT7側の担当。

## 完了条件

- 子プロセス起動・標準出力パースのテスト（Windows実機またはモックプロセスでの検証）。
- ~~「同梱表と同じ構成なら勧めない」判定のテスト。~~ **【ADR-196で置換】**
  [ADR196-T4](adr196-t4-ui-status-and-adoption.md)の完了条件へ。

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md) 段階6
- [ADR-176](../adr/176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md)
- [ADR-196](../adr/196-keymap-learn-truth-priority.md) 決定2（実装対象5のみ置換）
- [ADR195-T1](adr195-t1-independent-learning-process.md)
- [ADR195-T4](adr195-t4-runtime-loading.md)
- [ADR196-T4](adr196-t4-ui-status-and-adoption.md)（実装対象5・完了条件の後継先。子プロセス
  起動・標準出力パース機構〈実装対象1〜4〉はこちらからも再利用される）
