# ADR-195 T7: 安全対策（段階7）を実装する

状態: **一部実装済み（2026-09-23、ブランチ`feat/adr195-t7-safety-measures`）。
項目2（ユーザー入力混入検出）を実装・テスト済み。項目1・4は既存実装で既に満たして
いることを確認した（新規実装は不要）。項目3・5は未着手のまま残る（下記参照）。**
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-195](../adr/195-keymap-learn-productization.md)決定・段階7は、学習プロセスの
安全対策一式を定める。

## 実装対象

1. **専用窓への注入に限る**（他アプリへは送らない。T1のA'確保策と同じ窓を使う）。
   **【2026-09-23確認】新規実装は不要**——`RealImeDriver::new()`が
   `SetForegroundWindow`/`SetFocus`で専用EDIT窓へフォーカスを強制した上で
   `send_key_press`が`SendInput`するため、既存設計により最初から満たされている。
   本タスクの項目2で追加した`focus_intact()`チェックが、この前提が崩れた場合
   （フォーカスが他窓へ移った場合）を試行無効化として検出する形で補強している。
2. **ユーザー入力の混入検出**: 学習プロセスが自分の注入以外のキー・フォーカス変更を
   検出したら当該試行を無効化する。**【S7対応、2026-09-23】`WH_KEYBOARD_LL`フックと
   注入イベントの分類器（`LLKHF_INJECTED`の有無・自分の目印の有無での3分類）は
   [ADR196-T1](adr196-t1-external-write-observation.md)が実装・所有する。本タスクは、
   T1の分類器が「ユーザーの物理入力」と分類したイベントを受け取った後の**無効化処理**
   だけを担当し、独自のフックを作らない（T1着手前に本タスクが先行して着手する場合は
   特に注意——T1と同じフックを2つ登録しないこと）。**
   **【実装済み、2026-09-23】** `ADR196-T1`が`HookMonitor::physical_event_count()`
   として既に観測基盤を用意していた（未消費のまま残っていた）ため、これを
   `RealImeDriver::check_session_interference()`（`crates/awase-keymap-learn-win/src/driver.rs`）
   から消費する形で結線した。フォーカス変更の検出は本タスクで新規に追加
   （`GetFocus()`と専用EDIT窓の比較、`focus_intact()`）。両者と既存の外部書き込み
   検出は、純粋な判定ロジック`trial_contaminated()`
   （`crates/awase-keymap-learn/src/external_write.rs`、ホストでユニットテスト済み）
   に集約し、`RealImeDriver::new()`のquiet window判定・`check_session_interference()`
   の両方から同じ判定を使う（決定1b項目3・項目5と同じ枠組みへ統合）。
3. **他アプリへの副作用を作らない**: TsfNativeアプリへ生キーが届く経路が無いため
   BUG-113/124の「@」の機序は成立しないことを確認する。管理者権限の窓は対象外とする。
   短時間の大量注入がセキュリティソフトに検知されない範囲に押下数を抑える（S6+イベント
   待ちで既に大幅な削減がある前提を踏襲）。**未着手——実機での確認（セキュリティ
   ソフト検知の有無、TsfNativeアプリへの副作用の有無）が前提のため、Windows実機
   セッションで対応すること。**
4. **永続化ファイルの検証**（[ADR195-T4](adr195-t4-runtime-loading.md)の破損ファイル
   縮退と連動）。**【2026-09-23確認】新規実装は不要**——[ADR195-T4](adr195-t4-runtime-loading.md)が
   develop統合済み（`d7e0df17`）で、`crates/awase-windows/src/state/key_effect_runtime.rs`が
   サイズ上限超過・パース失敗・スキーマ版不一致のいずれも安全に縮退させる実装と
   回帰テスト（`schema_version_mismatch_is_rejected`等）を既に持つ。
5. **配布**: 学習プロセスはユーザーの実機で走る新しい.exeになるため、MSIへの同梱・署名・
   アンインストール時の扱い（ADR-177/178）を実装時に検討する。
   [ADR195-T6](adr195-t6-adr176-wizard-integration.md)でウィザードから起動する以上、
   同梱は必須。**未着手——MSI同梱・署名はWindows実機でのインストーラビルド・検証が
   前提のため、Windows実機セッションで対応すること。**

## 完了条件

- ユーザー入力混入検出のテスト（学習窓以外へのフォーカス変更・非注入キーの検出）。
  **【完了】** `crates/awase-keymap-learn/src/external_write.rs`の
  `trial_contaminated_by_physical_input_alone`/`trial_contaminated_by_focus_loss_alone`他
  （`cargo test -p awase-keymap-learn --lib external_write`で確認、host targetで実行可）。
  `RealImeDriver`側の結線は`cargo check --target x86_64-pc-windows-msvc -p
  awase-keymap-learn-win --tests --lib`でコンパイル確認済み（Win32実機での動作確認は
  未実施——CLAUDE.mdの既存注意通り、このサンドボックスではlink.exe不在のため実行不可）。
- MSI同梱・署名の実装確認（Windows実機でのインストーラ検証）。**未完了（項目5参照）。**

## 関連

- [ADR-195](../adr/195-keymap-learn-productization.md) 段階7
- [ADR195-T1](adr195-t1-independent-learning-process.md)
- [ADR195-T6](adr195-t6-adr176-wizard-integration.md)
