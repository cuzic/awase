# ADR-196 T2: 採否判定（自己検証正答率・内蔵表突き合わせ・再測定）を実装する

状態: **大部分実装済み（2026-09-23）**。0(内蔵表への参照経路)・1a(自己検証正答率の採否条件、
`judgement::judge_self_verification`)・1c(既知3構成判定、`awase-gji-config::known_keymap`)・
1e前半（判定を実際の学習フロー`run_main`へ配線、C-1〜C-9対応、下記参照）はdevelop統合済み
またはPR起票済み（PR #259: `judgement.rs`・`known_keymap.rs`、PR #263: `diff_against_bundled`、
PR #269: 1e前半の配線 + `known_keymap.rs`の既知構成誤判定バグ修正）。1b-8(判定書き換えモード、
`judgement::adopt_needs_confirmation`+`awase-keymap-learn-win --adopt-pending-judgement`)は
PR #265でdevelop統合（stdoutの`reason=`は空白なしコードのみ、詳細はstderr。採用済みへの再実行は冪等成功）。
PR #275で再測定オーケストレーション（1b項目7〜8、`awase_keymap_learn::remeasure`＋`judgement::combine`配線）、PR #273/#277で1e後半（不具合報告への添付、`BugReportKeymapLearnSummary`）、PR #284で項目9の不一致分布タグ付けの純粋ロジック（`mismatch_tag::tag_mismatches`）、PR #285で再測定の押下数上限・リセット間隔の実測に基づく調整をdevelopへ統合済み（2026-09-24時点）。
**残作業**: 項目9の配線（`tag_mismatches`を学習フロー・不具合報告へ接続、現状は純粋ロジックのみ）・再測定結果とT1外部書き込み観測の永続化（未永続化のため不具合報告にも未添付）・実機(`RealImeDriver`)での再測定の動作確認。MS-IME本体の未解決問題は[adr196-t2-msime-learning-open-issues.md](adr196-t2-msime-learning-open-issues.md)。
[ADR195-T2](adr195-t2-self-verification.md)（自己検証本体。正答率・縮退率の**計算**はT2の担当、
本タスクは**採否判定**の担当——役割を分けること）・[ADR195-T3](adr195-t3-persistence.md)
（永続化、スキーマに本タスクの出力フィールドを追加済みであること）・[ADR196-T3](adr196-t3-bundled-table-versioning.md)
（内蔵表の版情報、1b項目7の分布タグで参照）・[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)
（フィンガープリント、同じく1b項目7で参照）と依存があるため、実装順序をすり合わせること。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

## 背景

[ADR-196](../adr/196-keymap-learn-truth-priority.md)決定1a・1b（項目7〜9）・1c・1eは、
「学習結果を、内蔵表と一致するかどうかではなく、学習結果自身の品質で採否判定する」
仕組みを定める。判定は`awase.exe`の段階4読込時ではなく、**学習セッションの末尾
（学習プロセス自身）**が行い、不採用の場合も理由付きで表ファイルに書き出す。

## 実装対象

### 0: 内蔵表の参照経路（決定1e第1項）【実装済み、PR #263】

学習プロセス（`awase-keymap-learn-win`）が内蔵表（`state/key_effect_table.rs`）を参照
できるようにする。`key_effect_table`は`state/mod.rs`でprivateなモジュールなので、可視性を
広げるか、awase-settings側で比較して子プロセスへ結果を渡す設計にするかを決める
（ADR-195 m-c〈`pub(crate)`で足りるとしていた結論〉を、本タスクの用途に限って更新する）。
本タスクの1b項目7〜9（内蔵表との突き合わせ）はこの経路が無いと着手できない。

**実装内容**: 生の`ATOK`/`MSIME`/`MSIME_NATIVE`定数自体は`pub(super)`のまま広げず、
`awase-windows::state::key_effect_runtime::diff_against_bundled(persisted, preset) -> BundledDiff`
という専用の`pub`関数のみを追加した（決定1e「判定は学習プロセス自身が行う」に従い、
awase-settings側で比較する案は採らなかった）。`BundledDiff`は一致数・不一致セルの
`(status, key)`一覧（再測定対象、下記1b項目7〜9の入力）・片方にしか無いセル数を持つ。
`awase-keymap-learn-win`は既に`awase-windows`に依存しているため、Cargo.tomlの変更は
不要だった。再測定そのもの（実際にIMEへ再送って確認する部分）はスコープ外のまま。

### 1a: 自己検証正答率の採否条件

- 正答率 = 正解数 / 予測したステップ数、縮退率 = 予測しなかったステップ数 / 全ステップ
  数（同じ最終ウォーク上、分母が異なる。**計算自体は[ADR195-T2](adr195-t2-self-verification.md)
  の担当**、本タスクは計算結果を使って採否を判定するだけ）。
- 判定に使うウォークは予測したステップ数が最低300以上になるようにする（この所要時間の
  実測は[ADR195-T2](adr195-t2-self-verification.md)の前提条件とする）。
- **判定の順序（2026-09-23、横断レビューS3で訂正）**: 正答率95%未満なら表全体を**不採用**
  にする（分母の操作によるセル選別での水増しは禁止）。95%**以上**であっても、Microsoft
  IME本体は独立ウォークでの採点実績が無いため、既定では**要確認**（下記1b-8と同じ仕組み
  だが発火条件は異なる、[ADR196-T4](adr196-t4-ui-status-and-adoption.md)で別文言表示）に
  する。「95%基準を適用しない」＝「正答率を見ずに常に要確認」ではない——正答率60%の
  Microsoft IME本体の表は、95%未満の不採用が先に効く。

### 1c: 「既知3構成」の判定条件

- GJI: `session_keymap`がATOK/MSIME相当かつ`overlay_keymaps`が空。ATOKは
  `custom_keymap_table`の有無を問わない（ADR-186決定(c)）。MSIMEプリセットは
  `custom_keymap_table`が空、または無変換/変換等の関連行を含まないこと。
- Microsoft IME本体: `keystyle`が既定値・`msime_key_assignment.rs`検出の再割り当て
  なし・レガシー互換モード無効（[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)
  の互換モード読み取り実装に依存、**未着手のうちは既知構成と判定しない**）。
- 不一致率の分母は学習表と内蔵表の両方に存在するセルの共通部分（非決定・未観測セルは
  除く）。片方にしか存在しないセルは分母に含めず、「表にのみ存在」として不具合報告
  （下記）に別途記録する。

### 1b-8関連: 「学習結果を使う」操作＝判定書き換えモード

[ADR196-T4](adr196-t4-ui-status-and-adoption.md)がawase-settings側の「学習結果を使う」
ボタンを実装するが、実際の書き換えは**学習プロセスを判定書き換えモードで再起動**して行う
（3aの「表ファイルの書き手は学習プロセスのみ」の原則を保つため）。本タスクが、この
モード（CLI引数等での起動、対象の要確認レコードを採用状態へアトミックに書き換える処理、
成否をawase-settingsへ標準出力で返す）を実装する。

### 1b項目7〜9: 再測定・要確認・タグ付け

- 不一致セルは（5%等の起動閾値を置かず）**全件**再測定する。新しい学習プロセス
  （または新しいスレッド、実装時に選択）で、可能なら別のセットアップ経路で。
  再測定にもT1の観測（項目1〜6）を適用する。
- 再測定が元の学習値と一致すれば採用、一致しなければそのセルのみ「予測なし」に落とす。
- 再測定後も共通セルの30%超が不一致のままなら「要確認」状態にする（**既定では不採用**、
  内蔵表または予測なしを使い続ける）。「原則と矛盾しない」とは言わず、系統的バグへの
  安全弁としての例外だと実装コメントにも明記する。
- 不一致の分布（キー列に集中/状態行に集中・分散）をタグとして記録し、[ADR196-T3](adr196-t3-bundled-table-versioning.md)
  が埋め込む内蔵表の版情報と[ADR196-T5](adr196-t5-revalidation-not-invalidation.md)が計算
  するユーザー環境の版を突き合わせた「版一致有無」と合わせて不具合報告の添付に残す
  （単独の採否条件にはしない）。

### 1e: 判定の実行場所と不具合報告への添付（決定1b末尾・成功基準1b(7)、M6対応）

- 判定（1a・1b-8）は学習プロセスが学習セッションの末尾（段階2の後、永続化の前）に行う。
  不採用の場合も理由・スコアを表ファイルに書き出す。
- `awase.exe`の段階4読込は、永続化された判定結果を読むだけで判定をやり直さない。

**1e前半（判定を実際の学習フローへ配線する部分）【実装済み、PR #269】**: `run_main`が
自己検証ウォークの`ScoreReport`を計算するだけで`judge_self_verification`を一度も呼ばず、
`PersistedTable::with_verification`/`with_judgement`も呼んでいなかった（判定ロジック自体は
実装済みでも実際の学習フローでは一度も実行されていなかった）ギャップを埋めた。
着手前のopus-adversarial-consult（2026-09-23）で、当初の配線案（awase-settings側でTSFに
問い合わせてCLI引数で渡す）が事実誤認だったこと（学習プロセスは`RealImeDriver::new`で既に
COM STA初期化・`ITfThreadMgr::Activate`済みのスレッドを持っており、TSFのアクティブプロファイルは
スレッド単位で持つため、awase-settings側で問い合わせると学習窓とは別のIMEを測ってしまう）が
判明し、学習プロセス自身が学習窓のスレッドで同定する設計に変更した。あわせて以下も修正・実装:
- `known_keymap::classify_known_gji_keymap`が`session_keymap`不在（最多構成）を
  「既知構成でない」と誤判定していたバグを修正（B-4、developへ既存の別バグとして
  混入していたものをこのレビューで発見）。
- 段階4読み手（`validate_and_convert`）が`judgement`を一切読んでいなかった欠落を修正
  （C-3、判定を書いても効かない状態だった）。
- セッション失敗判定（外部書き込み・フック断絶）を`RealImeDriver`に追加したが
  呼び出し元がどこからも参照していなかった欠落を配線（C-1）。
- 検証ウォークの最小標本数（予測300歩）チェック追加（C-2）・専用乱数化（C-7）。
- 不採用/要確認の結果は`keymap-learn-table.json`を上書きせず、別ファイル
  `keymap-learn-last-attempt.json`へ退避する設計にした（**ユーザー判断**、C-9:
  以前`Accepted`だった良い表を今回の学習失敗で失わないため）。

**再測定オーケストレーション【実装済み、feat/adr196-t2-remeasure-reconcile】**:
`awase_keymap_learn::remeasure`（純粋ロジック、`SimIme`でテスト）が、`diff_against_bundled`の
不一致セルを全件（閾値なし）、学習の巡回とは別経路（リセット→ランダムキー列で目的statusへ
到達→対象キー押下）で再測定する。再現しなかったセルと**確認できなかった（到達不能・汚染続き・
中止）セル**は`prediction`を`None`へ落とし、`ReconciliationSummary`を`judgement::combine`へ
渡す（`run_main`。既知構成でない/`config1.db`不読のときは`None`のまま）。
**項目9の不一致分布タグ**: 純粋ロジック`awase_keymap_learn::mismatch_tag::tag_mismatches`を追加（キー集中=版ずれ寄り、状態集中・同版で不一致=パイプライン疑い、参考タグのみ）。学習フロー/不具合報告への配線は未実装。
**未実装（残作業）**: 決定1b項目9の配線（PR #284で純粋ロジックのみdevelop統合済み）、実機(RealImeDriver)での動作確認。
`REMEASURE_MAX_SETUP_PRESSES`は実測済み（60→250、windows-latest実GJI+ATOK・1600件: 中央値11・
p95≈100・p99≈150・最大230、60超は10.3%）。`REMEASURE_RESET_EVERY`も実測済み（12→24、3/6/12/24/48
を比較、到達押下数は差が無く所要時間のみ変わる）。

**1e後半（不具合報告への添付）【実装済み（`BugReportKeymapLearnSummary`、`attach_ime_keymap`相乗り・`SCHEMA_VERSION`据え置き）。ただし決定1b項目7〜9の再測定結果とADR196-T1の外部書き込み観測は現状どこにも永続化されていないため未添付——永続化され次第同型へ追加する】**: 別途、**不具合報告への添付は本タスクに一本化する**
（[ADR195-T4](adr195-t4-runtime-loading.md)
  実装対象5が挙げていた「学習表を使用中か」「フィンガープリント」は
  [ADR196-T5](adr196-t5-revalidation-not-invalidation.md)が計算するが、添付項目として
  まとめるのは本タスク）: 不一致セルの一覧・観測結果（[ADR196-T1](adr196-t1-external-write-observation.md)
  項目1〜6）・再測定結果・要確認判定・「表にのみ存在」するセルの一覧・学習表使用中か・
  フィンガープリント、を`bug_report.rs`（ADR-095/148）へ1つの添付項目としてまとめる。
  スキーマ版更新を伴うか確認すること。

## 完了条件

- 正答率・縮退率の分母定義に基づく採否判定のユニットテスト（境界値含む。95%未満→不採用が
  Microsoft IME本体の要確認より優先されることを含む）。
- 既知3構成判定のユニットテスト（ATOKでcustom_keymap_table有無に関わらず判定される
  こと、MSIMEプリセットで関連行があれば除外されることを含む）。
- 再測定→採用/要確認のフロー全体のテスト（30%閾値の境界含む）。
- 不採用時に理由付きで表ファイルへ書き出されることのテスト。
- 判定書き換えモードの結合テスト（要確認レコードが採用状態へアトミックに書き換わること）。
- 不具合報告への添付内容のテスト（一覧・観測結果・再測定結果・要確認判定・表にのみ存在
  するセル・学習表使用中か・フィンガープリントが1項目にまとまること）。

## 関連

- [ADR-196](../adr/196-keymap-learn-truth-priority.md) 決定1a・1b（項目7〜9）・1c・1e
- [ADR196-T1](adr196-t1-external-write-observation.md)
- [ADR196-T3](adr196-t3-bundled-table-versioning.md)（内蔵表の版情報、分布タグで参照）
- [ADR196-T4](adr196-t4-ui-status-and-adoption.md)（「学習結果を使う」ボタンの起動元）
- [ADR196-T5](adr196-t5-revalidation-not-invalidation.md)（フィンガープリント、分布タグで参照）
- [ADR195-T2](adr195-t2-self-verification.md)（正答率・縮退率の計算主体）
- [ADR195-T3](adr195-t3-persistence.md)（正答率・ステップ数・シード・判定結果のスキーマ追加が必要）
- [ADR195-T4](adr195-t4-runtime-loading.md)（実装対象5の不具合報告添付は本タスクへ移管）
