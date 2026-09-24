# ADR-196 T4: 較正パネルの状態表示・採用導線を実装する

状態: **主要部分実装済み（2026-09-23、`feat/adr196-t4-ui-status`）**。下記「進捗」参照。起票時: 2026-09-23。**【S5対応】前提タスクを列挙**: [ADR195-T6](adr195-t6-adr176-wizard-integration.md)
（子プロセス起動・標準出力パース機構、本タスクが利用する）、[ADR196-T2](adr196-t2-mismatch-adjudication.md)
（採否判定・要確認状態・判定書き換えモード）、[ADR196-T3](adr196-t3-bundled-table-versioning.md)
（「測定環境」表示に使う内蔵表の版情報）、[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)
（要再検証の判定・軽量再検証モード）。[ADR195-T6](adr195-t6-adr176-wizard-integration.md)
（旧・段階6のADR-176ウィザード統合、supersededマーカー参照）の後継。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-196](../adr/196-keymap-learn-truth-priority.md)決定2は、[ADR-195](../adr/195-keymap-learn-productization.md)
段階6の「同梱表と同じ構成なら学習を積極的に案内しない」というUI方針を撤回し、構成に
関わらず学習を同じ導線で案内する。学習プロセスの起動・進捗表示そのもの
（[ADR195-T6](adr195-t6-adr176-wizard-integration.md)実装対象1〜4）は変更しない。

## 実装対象

0. **【S5対応】現在のフィンガープリントの計算（awase-settings側）**: 状態表示（下記2）の
   「要再検証」判定に使う「現在のフィンガープリント」は、[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)
   決定3aにより**awase-settingsが計算する**（`awase.exe`は不具合報告作成時のみ）。
   計算はUIスレッドと分離すること（ブロックしうるWin32 API呼び出しのため）。この配線
   （較正パネル表示時に計算をキックし、結果を状態表示へ反映する）は本タスクの担当。
1. **学習の実行ボタン**: 構成に関わらず較正パネルの同じ位置・同じ強さで表示する
   （既定の導線から外さない）。
2. **状態表示（1行のみ、事実ベース）**:
   - 内蔵表使用中: 「使用中の予測表: 内蔵表（測定環境: GJI x.y.z, Windows Build
     NNNNN）」（[ADR196-T3](adr196-t3-bundled-table-versioning.md)が埋め込む版情報を表示。
     「Windows Build」ではなく「測定環境」と表記——CIのrunner版はユーザーには意味が
     通らないため）。
   - 学習表使用中: 「使用中: 学習表（YYYY-MM-DD学習、自己検証 NN%）」
   - 要再検証: 「使用中: 学習表（要再検証: GJI x.y.z → x.y.w）」
     （[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)と連動）
   - 学習したが採用されなかった: 「学習結果を採用しませんでした（理由: 自己検証 NN%
     / 外部からの書き込みを検出 / 予測できないキーが多い）」
   - 要確認・系統的な不一致: 「学習結果が内蔵表と大きく異なるため保留中（NN%のセルが
     不一致）— 学習結果を使う」
   - 要確認・Microsoft IME本体: 「Microsoft IME本体は実機での精度検証待ちのため既定
     では使用しません — 学習結果を使う」（不一致率の数字は出さない。系統的な不一致
     〈上記〉と発火条件が異なるため文言を分ける）
   - 未学習・カスタムキーマップで予測なし: 「予測表なし（カスタムキーマップ）— 学習を
     推奨」
3. **採用操作**: 「学習結果を使う」ボタンから、要確認状態の表を明示的に採用できる。
   採用は学習プロセスを判定書き換えモードで再起動して行う（表ファイルの書き手は学習
   プロセスのみ、という原則を保つ）。
4. **学習を勧める文言**: 症状ベースにする（例:「IME状態の表示やNICOLA入力の開閉が
   実際の入力とずれることがある場合、学習を実行してください」）。「より高精度な予測が
   必要な場合」のような、ユーザーが自分では判断できない表現は使わない。
5. **【M2対応】軽量再検証の起動ボタン**: 「要再検証」状態の表示行に、[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)
   が実装する軽量再検証モードを起動するボタンを置く（[ADR195-T6](adr195-t6-adr176-wizard-integration.md)
   の子プロセス起動機構を再利用、進捗表示は学習ボタンと共通のUIコンポーネントを流用
   できる）。

## 完了条件

- 各状態（内蔵表/学習表/要再検証/不採用/要確認×2種/予測なし）で正しい表示行になる
  ことのテスト。
- 「学習結果を使う」操作が学習プロセスの判定書き換えモード起動につながることの結合
  テスト（Windows実機またはモックプロセス）。
- 「要再検証」表示から軽量再検証を起動できることの結合テスト。
- 現在のフィンガープリント計算がUIスレッドをブロックしないことの確認。

## 関連

- [ADR-196](../adr/196-keymap-learn-truth-priority.md) 決定2
- [ADR195-T6](adr195-t6-adr176-wizard-integration.md)（実装対象1〜4はそのまま有効、
  実装対象5のみ本タスクが置き換える）
- [ADR196-T2](adr196-t2-mismatch-adjudication.md)
- [ADR196-T5](adr196-t5-revalidation-not-invalidation.md)

## 進捗（2026-09-23）

実装済み: `crates/awase-settings/src/keymap_learn_status.rs`（状態表示の純粋関数、7状態
のテスト、症状ベース文言）、`keymap_learn_launcher.rs`（`LearnMode`で`--adopt-pending-judgement`/
`--revalidate`起動、`adopt`/`revalidate`行のパース）、`main.rs`（状態行・「学習結果を使う」
「軽量再検証を実行」ボタン、現在のGJI版取得を別スレッドで実行〈対象0〉）。
旧`should_recommend_learning`（構成一致時に案内しない方針）は決定2に従い削除。

**残作業**:
- 内蔵表の測定環境（T3の版情報）は`status_line(bundled_env)`の引数に渡せるが、呼び出しは
  `None`固定（内蔵表側の版情報を実行時に読む経路が未整備）。
- 「学習したが不採用: 外部からの書き込みを検出」は表ファイルに理由が残らないため未対応
  （`RejectedReason`に相当が無い）。
- 「予測表なし（カスタムキーマップ）」は判定入力`custom_keymap_without_prediction`を
  常に`false`で渡している（ADR195-T0の構成検出との配線が未実装）。
- 現在の版取得はGJIのみ（Microsoft IME本体はT5の共有関数待ち）。
- 結合テスト（モックプロセスでの採用/再検証起動）は起動フラグ・パースのユニットテストまで。
- Windows実機でのUI確認は未実施。
