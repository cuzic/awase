# ADR-196 T5: 陳腐化検出を「失効」から「要再検証」へ置き換える

状態: 未着手（2026-09-23起票）。**実装前に必須の実測が1件ある**（下記「前提条件」）。
[ADR195-T3](adr195-t3-persistence.md)（永続化）完了後に着手。既存の
[ADR195-T8](adr195-t8-staleness-detection.md)実装（ブランチ`feat/adr195-t8-staleness-detection`、
develop未マージ、コミット`3613707e`/`f5d53047`）は旧設計（陳腐化＝失効）で書かれている
ため、本タスクはゼロから作るのではなく、そのブランチを土台に決定3の差分を当てる形で
進めることを推奨する（該当ブランチの担当者と重複作業にならないよう事前に調整すること）。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 前提条件（実装着手前に確定が必要な実測）

- **Microsoft IMEレガシー互換モードフラグのレジストリ位置**: `msime_legacy_keymap.rs`
  は`keystyle`とStyleListしか読んでおらず、「以前のバージョンのMicrosoft IMEを使う」
  設定そのものを読むコードは存在しない。実機でのレジストリdiff（設定ON/OFF切り替え
  前後の`HKCU\Software\Microsoft\IME\...`の差分）で確定するまで、Microsoft IME本体の
  フィンガープリント・既知構成判定は実装できない。**GJI側の実装はこれを待たずに
  進められる**。
  **2026-09-23追記**: 並行して起票された[ADR-197](../adr/197-msime-legacy-custom-keymap-runtime-warning.md)
  （草案、opus-adversarial-consult未実施、developに未マージ）が、この互換モードの
  レジストリ実体（`NoTsf3Override2`＋`keystyle`＋`StyleList\<style>\key`）を実機調査済み
  と主張している。本タスク着手時は、ADR-197の収束状況を確認し、この実測を重複して
  やり直さずADR-197の成果を参照すること（ADR-197はADR-196を関連ADRとして既に認識して
  いるため、双方の担当が同じ調査を独立に進める事故を避けるためにも要確認）。

## 背景

[ADR-196](../adr/196-keymap-learn-truth-priority.md)決定3は、[ADR-195](../adr/195-keymap-learn-productization.md)
段階8の「陳腐化＝失効」を撤回し、「要再検証」の状態遷移に置き換える。理由は、内蔵表が
より新しいという保証が無いこと、カスタムキーマップ・Microsoft IME本体のユーザーには
内蔵表という戻り先が無いこと、GJI/Windowsの更新頻度から見て即時失効はWindows Update
のたびに再学習を要求しうる設計になることの3点。

## 実装対象

### 3a: 状態遷移

- 「要再検証」は保存しないフラグとする。表ファイルに保存されたフィンガープリント
  （書き手は学習プロセスのみ）と、その都度計算する現在のフィンガープリントの比較結果
  として毎回派生させる。
- **現在のフィンガープリントを計算する必要があるのはawase-settings（状態表示時）と
  学習プロセス（記録・軽量再検証の要否判定）の2者だけ**。要再検証になっても
  `awase.exe`が使う表は変わらないため、`awase.exe`は不具合報告作成時に1回だけ計算
  すれば足りる（周期取得は不要）。
- 学習時・現在いずれかの版が「不明」なら比較しない（対称なfail open）。
- 次に学習プロセスを実行できる機会に、全数再学習ではなく段階2単独の軽量再検証を案内
  する。軽量再検証にも[ADR196-T1](adr196-t1-external-write-observation.md)の観測
  （項目1〜6）を適用する。
- 軽量再検証の正答率が95%（[ADR196-T2](adr196-t2-mismatch-adjudication.md)の閾値）を
  割った場合に初めて失効させる。合格したら学習プロセスがフィンガープリント・正答率・
  採点日時をアトミックに（一時ファイル→置換）書き直す。

### 3b: フィンガープリントの構成

- **GJI**: Converter本体（`GJI_PROCESS_PREFIXES`、`tsf/gji_monitor.rs:25-39`）の
  フルパスを取得する新規関数（`focus/classify.rs::get_process_name`の切り詰め前の値を
  返す派生版）。`VS_FIXEDFILEINFO`（`dwFileVersionMS`/`dwFileVersionLS`の4値）を取得
  する共有関数を1つ用意し、[ADR196-T3](adr196-t3-bundled-table-versioning.md)の内蔵表
  生成スクリプトとも共有する。
  - Converterが見つからない場合は「不明」（fail open）。
  - **更新直後の食い違い対策**: Converter実行ファイルの最終更新時刻が学習プロセス
    自身の起動時刻より新しければ、版を「不明」ではなく**「未確定」**として記録する。
    「未確定」は不明どうしの比較除外の対象外とし、次に取得できたどの版とも常に不一致
    （要再検証）として扱う。
- **Microsoft IME本体**: ファイル版ではなく、OSビルド番号（レジストリ
  `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\CurrentBuildNumber`。UBRは含め
  ない）＋レガシー互換モードフラグ（上記「前提条件」）＋`keystyle`＋
  `msime_key_assignment.rs`の再割り当て検出結果の4値。

## 完了条件

- フィンガープリント比較（一致/不一致/不明どうし/未確定）のユニットテスト。
- 軽量再検証→失効/継続のフローのテスト。
- `awase.exe`がフィンガープリントを周期取得しない（不具合報告作成時のみ計算する）
  ことの確認（既存の`feat/adr195-t8-staleness-detection`実装が周期取得を前提にして
  いれば、そこを削る差分になる）。

## 関連

- [ADR-196](../adr/196-keymap-learn-truth-priority.md) 決定3
- [ADR195-T8](adr195-t8-staleness-detection.md)（supersededマーカー参照、土台となる
  既存実装ブランチあり）
- [ADR196-T1](adr196-t1-external-write-observation.md)・[ADR196-T2](adr196-t2-mismatch-adjudication.md)
- [ADR196-T3](adr196-t3-bundled-table-versioning.md)（版取得の共有関数）
- [ADR196-T4](adr196-t4-ui-status-and-adoption.md)（要再検証の状態表示）
