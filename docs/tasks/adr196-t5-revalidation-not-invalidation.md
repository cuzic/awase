# ADR-196 T5: 陳腐化検出を「失効」から「要再検証」へ置き換える

状態: **主要部分実装済み（2026-09-24、PR #279・#280・#283でdevelop統合）。** 3a(状態遷移)の純粋ロジック・GJI側のConverter版取得と指紋書き込み配線・軽量再検証モード(`--revalidate`)・awase-settingsの起動ボタン(T4)は実装済み（実機での動作確認はGJIのみ・一部）。**残り**: Microsoft IME側の版取得(ADR-197待ち)・採点日時フィールド・互換モード読み取り。下記「進捗」参照。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

**訂正（2026-09-23）**: 起票時点の記載「既存の[ADR195-T8](adr195-t8-staleness-detection.md)
実装(ブランチ`feat/adr195-t8-staleness-detection`)を土台にする」は誤りだった。
T8のコミット(`3613707e`/`f5d53047`)はT9(#258)経由で既にdevelopの祖先に含まれており
(`crates/awase-keymap-learn/src/staleness.rs`として存在)、当該ブランチ自体は既に削除
されている。**「土台にする」は「develop上の`staleness.rs`とは別モジュールとして追加する」
と読み替える**——本タスクが追加する「バージョン相当の情報」の不一致判定は、
`staleness.rs`の`Staleness`(即時失効、キーマップ設定変更・スキーマ版不一致用)とは
意図的に別の型・別モジュールにする(下記「進捗」参照)。

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

## 進捗（2026-09-23）

`diag/adr196-t5-revalidation`ブランチ（未push、developから分岐）で以下を実装・テスト済み:

- `crates/awase-keymap-learn/src/revalidation.rs`（新規）: 3a(状態遷移)の比較ロジックのみ。
  `EnvVersion`(不透明な4値)・`EnvVersionProbe`(Unknown/Unconfirmed/Known)・
  `StoredEnvVersion`(永続化側、Unconfirmed/Known の2値、`Unknown`は`Option::None`で表現)・
  `needs_revalidation(stored, current) -> bool`。前提条件の「未確定」規則
  (`Unconfirmed`は`Unknown`どうしの比較除外＝fail openの対象外、常に要再検証)を含め
  12件のユニットテストで規則を1つずつ確認済み（`cargo test -p awase-keymap-learn --lib
  revalidation`、`cargo clippy -p awase-keymap-learn --lib -- -D warnings`ともgreen）。
- `crate::staleness`（`Staleness`列挙体、キーマップ設定変更・スキーマ版不一致用）には
  一切手を入れていない——完了条件の「本タスクの追加フィンガープリントと混同されていない
  ことの確認」を、型を分けることで構造的に満たす形にした。

**意図的に手を付けていない部分**（別セッションとの衝突回避・前提未確定のため）:

- `persist.rs`/`PersistedTable`への永続化フィールド追加。[ADR196-T2](adr196-t2-mismatch-adjudication.md)
  (2026-09-23時点で別セッション`rust-nicola-0f`が着手中、スキーマ拡張を伴う)と同じ
  ファイルを触るため、T2のスキーマ変更が固まってから合わせて追加する。
  `PersistedTable`に`env_version: Option<StoredEnvVersion>`のような`#[serde(default)]`
  フィールドを足し、`persist.rs`冒頭の既存後方互換コメントに倣うことを想定。
- 「軽量再検証→失効/継続」フロー（[ADR196-T2](adr196-t2-mismatch-adjudication.md)の
  95%閾値に依存、T2未完了のため）。
- 3b: GJI Converterの`VS_FIXEDFILEINFO`4値取得の共有関数(`awase-windows`側、
  `tsf/gji_monitor.rs`の`GJI_PROCESS_PREFIXES`/`find_gji_pid`と`focus/classify.rs::get_process_name`
  を使う)。実機での動作確認ができないままWin32コードを書くことのリスクを踏まえ、
  今回のセッションでは着手を見送った。
- 3b: Microsoft IME本体側の4値取得（前提条件のレジストリ実測が[ADR-197](../adr/197-msime-legacy-custom-keymap-runtime-warning.md)
  側で完了しているかの確認自体が未実施）。
- 軽量再検証モード(CLI引数)・awase-settings側のUI導線・アトミック書き直し。

## 背景

[ADR-196](../adr/196-keymap-learn-truth-priority.md)決定3は、[ADR-195](../adr/195-keymap-learn-productization.md)
段階8の「陳腐化＝失効」を撤回し、「要再検証」の状態遷移に置き換える。理由は、内蔵表が
より新しいという保証が無いこと、カスタムキーマップ・Microsoft IME本体のユーザーには
内蔵表という戻り先が無いこと、GJI/Windowsの更新頻度から見て即時失効はWindows Update
のたびに再学習を要求しうる設計になることの3点。

**この置換は「バージョン相当の情報」に限る（2026-09-23、横断レビューM5対応。ADR-196決定3aへ
追記済み）**: [ADR195-T8](adr195-t8-staleness-detection.md)実装対象1（キーマップ設定自体の
変更＝`config1_db_stamp`または3値ハッシュの不一致）と実装対象2（永続化スキーマ版の不一致）は
撤回しない——本タスクが追加するGJI/Microsoft IME本体のバージョン相当の情報とは**別枠の
フィンガープリント**として、そのまま即時失効の判定に使い続ける。本タスクが実装するのは、
バージョン相当の情報が不一致のときだけ「要再検証」にする部分である。

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
- **【M2対応】軽量再検証モードの実装**: 学習プロセスに、段階1（格子学習）を飛ばして
  段階2（自己検証ウォーク）だけを実行するモード（CLI引数等）を追加する。awase-settings
  側の起動ボタン（状態表示「要再検証」の行に配置、[ADR196-T4](adr196-t4-ui-status-and-adoption.md)
  参照）は[ADR195-T6](adr195-t6-adr176-wizard-integration.md)の子プロセス起動・標準出力
  パース機構を再利用する。

### 3b: フィンガープリントの構成

**進捗(2026-09-24、`diag/adr196-t5-revalidation`)**: GJI側の版取得は実装済み(実機未検証)——
`awase-keymap-learn-win/src/env_version.rs`(`file_version`共有関数・自セッションに絞った
Converterパス探索・`probe_gji_env_version[_with_timeout]`)と、純粋な
`revalidation::classify_converter_version`(不明/未確定/既知の分類、ユニットテスト済み)。
指紋書き込み配線も実装済み: `PersistedTable.env_version: Option<StoredEnvVersion>`(追加のみ、
スキーマ版は上げず`#[serde(default)]`で旧ファイルも読める)を新設し、学習プロセス
(`awase-keymap-learn-win/src/main.rs::probe_env_version`)がGJIのときだけ学習終了時に
Converter版を取得して書く(3秒タイムアウトで超過時は書かない)。
軽量再検証モードも実装済み: `awase-keymap-learn-win --revalidate`が保存済み表を予測として
段階2(自己検証ウォーク)だけを実行し、`revalidate status=passed|invalidated|failure`行を
標準出力へ出す(判定は純粋関数`revalidation::{outcome_of_revalidation,apply_revalidation,
table_from_persisted}`)。合格なら`env_version`・`verification`を書き直し(判定は元のまま)、
`Rejected`相当のときだけ判定を`Rejected`へ落とす。採点日時の記録用フィールドは現スキーマに無く
未実装。
未着手: Microsoft IME側の4値(ADR-197待ち)。awase-settingsでの現在版との比較表示・起動ボタンはADR196-T4(PR #283)で実装済み。

- **GJI**: Converter本体（`GJI_PROCESS_PREFIXES`、`tsf/gji_monitor.rs:25-39`。
  `find_gji_pid`は全セッションから最初の一致を返すため、可能なら`ProcessIdToSessionId`で
  自セッションに絞る）のフルパスを取得する新規関数（`focus/classify.rs::get_process_name`
  の切り詰め前の値を返す派生版）。`VS_FIXEDFILEINFO`（`dwFileVersionMS`/`dwFileVersionLS`
  の4値）を取得する共有関数を1つ用意し、**本タスク（T5）が所有する**（[ADR196-T3](adr196-t3-bundled-table-versioning.md)
  はこの関数を使う側であり、T3側で再実装しない——Python生成スクリプト用にも、この共有
  関数をRust側のCLI等から呼ぶ形にする）。
  - Converterが見つからない場合は「不明」（fail open）。
  - **更新直後の食い違い対策**: Converter実行ファイルの最終更新時刻が学習プロセス
    自身の起動時刻より新しければ、版を「不明」ではなく**「未確定」**として記録する。
    「未確定」は不明どうしの比較除外の対象外とし、次に取得できたどの版とも常に不一致
    （要再検証）として扱う。
  - **取得の頻度とスレッド**: Toolhelpスナップショット・`OpenProcess`・
    `GetFileVersionInfoW`（ファイルI/O）はブロックしうるため、awase-settingsのUIスレッド
    や学習プロセスのメインループから直接呼ばず、別スレッド/`run_with_timeout`等で隔離する。
- **Microsoft IME本体**: ファイル版ではなく、OSビルド番号（レジストリ
  `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\CurrentBuildNumber`。UBRは含め
  ない）＋レガシー互換モードフラグ（上記「前提条件」）＋`keystyle`＋
  `msime_key_assignment.rs`の再割り当て検出結果の4値。

## 完了条件

- フィンガープリント比較（一致/不一致/不明どうし/未確定）のユニットテスト。
- Converterが見つからない場合、失効・要再検証のいずれにもならない（fail open）ことの
  ユニットテスト。
- 軽量再検証→失効/継続のフローのテスト。合格時にフィンガープリント・正答率・採点日時が
  アトミックに書き直され「要再検証」が解消されることのテスト。
- キーマップ設定変更・スキーマ版不一致（[ADR195-T8](adr195-t8-staleness-detection.md)
  実装対象1・2、置換対象外）は即時失効のままであることの回帰テスト（本タスクの追加
  フィンガープリントと混同されていないことの確認）。
- `awase.exe`がフィンガープリントを周期取得しない（不具合報告作成時のみ計算する）
  ことの確認（既存の`feat/adr195-t8-staleness-detection`実装が周期取得を前提にして
  いれば、そこを削る差分になる）。

## 関連

- [ADR-196](../adr/196-keymap-learn-truth-priority.md) 決定3（キーマップ設定・スキーマ版は
  対象外という追記込み）
- [ADR195-T8](adr195-t8-staleness-detection.md)（supersededマーカー参照。実装対象1・2
  〈キーマップ設定・スキーマ版のフィンガープリント〉は有効なまま本タスクへ引き継ぐ。
  土台となる既存実装ブランチあり）
- [ADR195-T6](adr195-t6-adr176-wizard-integration.md)（軽量再検証の起動導線が再利用する
  子プロセス起動機構）
- [ADR196-T1](adr196-t1-external-write-observation.md)・[ADR196-T2](adr196-t2-mismatch-adjudication.md)
- [ADR196-T3](adr196-t3-bundled-table-versioning.md)（版取得の共有関数の利用側。所有は本タスク）
- [ADR196-T4](adr196-t4-ui-status-and-adoption.md)（要再検証の状態表示・軽量再検証の起動ボタン）
