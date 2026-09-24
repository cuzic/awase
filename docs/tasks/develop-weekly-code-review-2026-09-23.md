# develop 過去1週間の fix コードレビュー結果（2026-09-23）

状態: **調査完了。修正済み: A-1 / C-1（PR #272）・B-1配送（PR #274、生存確認は未配線）・B-7 / B-8 / B-9（PR #280）・B-2の副次（PR #278、`mismatch_ratio`は`4af30b0c`）。B-4 / B-5 / B-6は`fix/adr192-t5-learned-table-consistency`で修正（下記）。未修正: A-2 / B-2本体 / B-3 / B-10**（2026-09-24時点、origin/develop `bbd6d133`で再確認）

## 対象と方法

- 範囲: `ce59df44..develop`（2026-09-16 以降、520コミット・Rust 約3.7万行）の fix コミット
- 方法: opus サブエージェントを領域別に3本、**読み取り専用**で並列実行。指摘は現在の HEAD（`2cd8d87a`）のコードで裏取り済み。round1〜3 のレビュー反映で既に直っている項目は除外
- 確信度: CONFIRMED = コードを読んで欠陥を確認 / PLAUSIBLE = 実機タイミング等の前提が必要
- 注意: Windows 専用 cfg のコードは Linux では走らない。修正時は `cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib` と windows-build CI で確認すること

## A. awase-windows（ADR-191 / BUG-154〜160 系）

### A-1. [CONFIRMED・修正済み（PR #272）] `Unwarranted` の完了で `applied` が誤って書き換わる

- 場所: `crates/awase-windows/src/state/ime_model.rs:506-511`（`completion_can_update_applied`）、`:1068-1094`（`reduce_ime_apply_failed`）
- 欠陥: `ImeOpenOutcome::Unwarranted`（`c8bc1adc`、ADR-090 A-2）は他の箇所（`platform_state.rs:1199-1218`、`executor.rs:1076`、`platform.rs:1473`）では「送っていない」扱いに追随済み。だが `completion_can_update_applied` の `NotSent` 条件だけ `UnsafeToToggle | NotOwned` のまま
- 失敗シナリオ: SetOpen(true) が授権なしで `Unwarranted` 完了 → generation 一致で `Accepted` → `applied = Confirmed{open:false}` を書く（実 IME には未送信）→ 後続 SetOpen(false) で GjiDirect の already-matched 判定が `VK_IME_OFF` を省略 → IME ON のまま Engine OFF
- 到達可能性: BUG-148 の CI で委譲 SetOpen が全件 `Unwarranted` になった実例あり（`platform_state.rs:3087`）
- テスト欠落: generation 付き `Unwarranted` のテストなし（`unsafe_to_toggle_…`/`not_owned_…` の同型テストのみ）
- 修正方針案: `NotSent` 条件に `Unwarranted` を追加 + 同型テスト追加

### A-2. [PLAUSIBLE・未修正（`mode_key_pass.rs`は9/23のテスト追加のみ）] 通過マーク窓で「古い観測」に揃えると窓後に揃え直せず BUG-157 が再発しうる

- 場所: `state/mode_key_pass.rs:157-168`（`drop_decision` の `aligned: mark.aligned || (align && !on_expiry)`）、`:258`（`should_align_after_expired_mode_key_pass` の `!mark.aligned`）
- 欠陥: 窓内の最初の成功観測が、IME がキーを処理する前の古い値でも `aligned=true` になる。窓後の救済（`align_after_expired`）は `!aligned` のときしか動かない
- 失敗シナリオ（MS-IME 本体等）: t≈20ms の最初の OsPoll が古い値を読む（IME 反応は最大 62ms）→ `desired_open` が押下前の値に揃い `aligned=true` → t≈80ms の再読み取りが時間切れ → 窓後の成功観測でも揃え直されない → `last_intent` は破棄済みで drift correction がユーザーのモードキー操作を書き戻す
- 修正方針案: `KEY_EFFECT_SETTLE_MS` 以降の観測に揃えたときだけ `aligned` を立てる。要テスト
- 前提: 1回目成功+2回目時間切れという実機タイミング。実機ログでの確認が先

### A-3. 確認して問題なしとしたもの（参考）

`reinject_scan_code` の VK 集合判定 / `TipIdentity` デバウンス / `RuntimeTableCache` のプリセット変更検出 / `poll_counted_no_new_miss` / `encode_outcome`・`decode_outcome`（`Unwarranted=7`）/ `KeyEffectPredicted` reducer アーム / `transport.rs::plan` の Suppress 範囲 0xF3/0xF4 絞り込み / `belief_conflicts_with_applied` の消滅（`4378b061` で revert 済み、`ba6144a6` に置換）

## B. 学習系（awase-keymap-learn / keymap-learn-win / settings）

### B-1. [高・CONFIRMED・配送のみ修正済み(fix/b1-ime-notify-external-write)] IME 通知による外部書き込み検出が動いていない

- 場所: `crates/awase-keymap-learn-win/src/driver.rs:729-738`（`pump_for`）、`ime_notify.rs:50`、`driver.rs:342-356`
- 欠陥: `WM_IME_NOTIFY` は SendMessage で送られ、`PeekMessageW` が MSG として返すことはない。届く先は EDIT 子窓のウィンドウプロシージャ
- 影響: 他プロセスが `ImmSetOpenStatus`/compartment 書き込みで学習窓の IME 状態を変えても `external_count` は 0 のまま。汚染観測が「キーの効果」として表に入る
- 関連: ADR-196 決定1b 項目2 が求める生存確認 `observation_alive()`/`measurement_suspicious()` はどこからも呼ばれていない（grep 確認）→「検出ゼロ = 外部書き込みなし」と合格側に倒れる。物理入力（フック）と `WM_ACTIVATE`（`window_proc`）の検出は正常
- 関連タスク: `adr196-t1-external-write-observation.md`
- 対応（2026-09-23）: EDIT子窓をサブクラス化（`edit_proc`）し、親窓`window_proc`と合わせて`WM_IME_NOTIFY`を到着時刻付きでスレッドローカルのキューへ積み、`pump_for`と`mark_self_injection`の直前に`ImeNotifyMonitor`へ渡す。猶予窓の判定は到着時刻で行う。**未対応**: `observation_alive`/`measurement_suspicious`の配線（実機でEDIT窓にIME通知が実際に届く頻度・遅延を測ってから。届かない環境で配線すると学習が偽陽性で全失敗する）、`NOTIFY_EXPECT_WINDOW_MS`(150ms)より遅い自己注入由来の通知が外部扱いになる恐れ（配送が動いた今、実機で実測が必要）。
- windows-latest実機検証（2026-09-23、`b1-notify-probe-verify`ワークフロー、GJI+ATOK配列、`examples/notify_probe.rs`+`tools/e2e/b1-notify-probe.ps1`）: 配送は動作（全wParam履歴を取得できた）。**第2の欠陥を発見・修正**: `IMN_SETOPENSTATUS`/`IMN_SETCONVERSIONMODE`の定数が誤り（0x2/0x3、正しくは0x8/0x6）で、修正前は`ImmSetOpenStatus`反転(true→false→true)を実際に行っても外部検出0件。修正後はプロセス内反転2/2検出、別プロセスからのIMC_SETOPENSTATUS・物理キー注入も検出(7件)、自己注入6押下での偽陽性0件（猶予窓150ms、GJI warm時）。再測定（自己注入40押下×2）でも偽陽性0件。コールド（起動後120秒idle→40押下）でもidle中の外部通知0件・偽陽性0件（GJI+ATOK）。**MS-IME本体は未測定**: `RealImeDriver::new()`がconv 0x0001未対応で初期化に失敗する（別問題、`normalized_mode`）ため、プローブも走らせられない。

### B-2. [中〜高・本体は未修正／副次は修正済み] 閉状態セルが1つに潰れ、採用セルが実行ごとに変わる

- 場所: `awase-windows/src/state/key_effect_runtime.rs:107-117`、`keymap-learn-win/src/main.rs:59-68`、`key_effect_predictor.rs:236`（`find_in`）
- 欠陥: 閉状態は `conv: None`（ワイルドカード）に変換され、mode 0x09 と 0x00 の閉セルが同じ検索キーになる。開く遷移の `after_conv` は `Some(保持モード)` で値が異なる（atok_like の閉状態は m=0/1 の2つ）。`build_persisted_cells` は `HashMap` 反復順で書くため `find_in` の先勝ちが学習ごとに変わる（潰れる点は CONFIRMED）
- 影響: 閉→漢字キーで開くとき、予測変換モードがひらがな/英数で入れ替わる
- 副次（PLAUSIBLE）: 同梱表の閉セルは `after_conv: None` のため `mismatch_ratio`（`:167`）で開く系5セルが常に不一致。比較対象が100セル未満なら `MAX_MISMATCH_RATIO`（5%）超過で、正しい学習表が `MismatchesBundledTooMuch` で不採用になりやすい
- 関連タスク: `adr196-t2-mismatch-adjudication.md`、`adr196-t3-bundled-table-versioning.md`
- 対応（2026-09-23〜24）: 副次の`mismatch_ratio`/`diff_against_bundled`は同梱表側の`after_conv=None`を「主張なし」として扱うよう修正済み（PR #278、`4af30b0c`）。本体（`convert_cell`が閉セルを`conv: None`へ潰す点、`key_effect_runtime.rs`）は未修正のまま

### B-3. [中・PLAUSIBLE・未修正] `classify_robust` の頑健性が既定 k=2 ではほぼ効かない

- 場所: `crates/awase-keymap-learn/src/verify.rs:46-81`
- (a) 同文脈で 1対1 に割れても少数派1件は閾値2未満 → `Det(先着)`。`Req::default().k=2` では2回観測したセルは割れても非決定と宣言されない
- (b) 1件しかない文脈グループは多数派補正を受けず、別文脈の迷い観測1件でセル全体が `HistoryDep`/`Conflict`（予測なし）になる
- 影響: 入力中 BS（75/25）が同文脈で1対1に割れると25%側が確定予測として書き出される（`declared_not_det` が偽でやり直しも起きない）。逆に単発の誤観測で不要な全体再巡回
- 注: PR #282/#286は多数決ロジックの共通化（`table::majority_of`）と再カウント解消のリファクタで、(a)(b)の挙動は変えていない（同数タイのテストのみ追加）

### B-4. [中・CONFIRMED・修正済み(2026-09-24)] 進捗の分母が実セル数の約2倍

- 場所: `keymap-learn-win/src/main.rs:299`、`:261-265`
- 欠陥: `total_cells = states.len() × 14 = 12×14 = 168`。Table は `Status` 単位で入力中4段階が同 Status にまとまるため区別できる Status は6個、`covered1` 上限は約84
- 対応: `Machine::distinct_status_count()`を追加し、分母を`distinct Status数 × キー数`にした（`states.len()`→重複除去、`awase-keymap-learn/src/model.rs`、`main.rs`の2箇所）。回帰テスト`distinct_status_count_merges_hidden_states_sharing_a_status`
- 影響: 進捗バーが約50%で止まる。ETA は完了時にも経過時間相当の「残り」を表示。result 行の `cells=/total=` も同様にずれる

### B-5. [中・修正済み(2026-09-24)] 変換不能キー（0x41）が縮退率の分母に常に入る

- 場所: `keymap-learn-win/src/main.rs:21`（`KEYS`）、`key_effect_runtime.rs:143`、`key_effect_predictor.rs:119-135`
- 欠陥: `TableKey::from_vk(0x41)` は `None` だが、書き出し側は訪問セルを全部書く（M5 方針）ため全セルの 1/14（約7.1%）が「変換できないセル」として `coverage_ratio` の分母に入る（事実は CONFIRMED）
- 対応: 書き手(`build_persisted_cells`)が、読み手の語彙外のキー（`TableKey::from_vk`が`None`＝0x41）のセルを書かないようにした。読み手の`coverage_ratio`は変更していない（既存テスト`coverage_too_low_is_rejected`の「表に無いVKは変換不能」の意図を保つため）。テスト`build_persisted_cells_omits_keys_the_reader_cannot_represent`はWindows専用cfgのため、windows-build CIで初めて実行される
- 影響（PLAUSIBLE）: `MIN_COVERAGE_RATIO=0.80` の実余裕は約13%。入力中 BS/Esc/文字キーの履歴依存・非決定セルが両モード分で6件前後出ると `CoverageTooLow`

### B-6. [中・PLAUSIBLE・修正済み(2026-09-24、実機未検証)] `CoUninitialize` の後に COM インターフェースを Release している

- 場所: `keymap-learn-win/src/driver.rs:508-514`（`Drop::drop`）
- 欠陥: `drop` 本体で `CoUninitialize()` を呼ぶが、フィールド `thread_mgr`/`thread_compartments` は `drop` の後に Release される
- 影響: 正常終了時や quiet window 失敗で `new()` 内の `driver` が drop されたとき、アンロード済み COM への Release でアクセス違反の恐れ（result 行出力後なので主な影響は非0終了コードとクラッシュダイアログ）
- 対応: `ComApartment`ガード型を追加し、`RealImeDriver`の最後のフィールド（`_com`）に置いて、COMインターフェース解放後に`CoUninitialize`が走るようにした。`new()`内の早期returnでもローカル変数の解放順で同じ順序になる。`cargo check --target x86_64-pc-windows-msvc`のみ確認、実機未検証
- 修正方針案（採用しなかった案）: COM フィールドを `Option` にして `drop` 内で先に take するか、`CoUninitialize` を専用ガード型の Drop に分離

### B-7. [低〜中・PLAUSIBLE・修正済み（PR #280）] 読み取りスレッドが `child` のロックを握ったまま `wait()` し UI が固まる

- 場所: `crates/awase-settings/src/main.rs:1088-1094`（読み取りスレッド）、`:1118-1130`（`kill_keymap_learn_child`）
- 欠陥: `drain_learning_output` 復帰後、ロック保持のまま `guard.wait()` でブロック
- 失敗シナリオ: stdout 読み取りで InvalidData 以外の I/O エラー、子は生存 → UI が `kill_keymap_learn_child()` の `lock()` で停止 → 子の終了まで設定画面がフリーズし kill も効かない

### B-8. [低・CONFIRMED・修正済み（PR #280）] 書き込み失敗時に失敗理由ではなく警告文が表示される

- 場所: `keymap-learn-win/src/main.rs:375-379`、`awase-settings/src/keymap_learn_launcher.rs:147-155`
- 欠陥: 失敗理由を eprintln → `result status=failure` → その後 `decode_errors>0` の警告を stderr に出す。設定画面は stderr の最後の非空行を失敗理由として表示
- 影響: config.toml 未検出 + `decode_errors>0` のとき、真の原因が警告文に隠れる

### B-9. [低・CONFIRMED・修正済み（PR #280）] UI 説明が実挙動と矛盾する

- 場所: `awase-settings/src/main.rs:3279-3280`、`keymap-learn-win/src/hook_monitor.rs:48-50`、`driver.rs:454-467`
- 欠陥: 画面に「他の窓では通常どおり入力できます」とあるが、フックは全システムの物理キー入力を汚染として数え、フォーカス喪失で `send_gated` が即 `session_failed` を立てる
- 影響: 説明どおり別窓で入力するとセッション確実に失敗

### B-10. [潜在・未修正・見送り中] `staleness.rs` の `NotSupported` が `Fresh` と判定される

- 場所: `crates/awase-keymap-learn/src/staleness.rs:75-81`
- 「表に指紋あり」かつ「現在の IME が `NotSupported`」の組が `Fresh` になる。GJI → 指紋方式のない IME への切替が陳腐化として検出されない
- 見送り理由（2026-09-24）: `staleness::check`は現在も呼び出し元が無く、既存テスト`fresh_when_current_fingerprint_not_supported`が`NotSupported`→`Fresh`を意図した挙動として固定している。ADR196-T5の再検証（`revalidation.rs`、`--revalidate`）が別モデルで陳腐化判定を担う方向なので、`check`の意味を変える前にT5側の扱いを決める必要がある
- 現状は呼び出し元が未配線のため未発現。**ADR196-T5 の配線時に見直すこと**（`adr196-t5-revalidation-not-invalidation.md`）

## C. エンジン / gji-config / CI / lints

### C-1. [中・CONFIRMED・修正済み（PR #272）] 強制IME操作（bare `keys.ime_*`）を設定すると Shift+無変換/変換 でも IME が切り替わる

- 場所: `src/engine/nicola_fsm.rs:1471-1480`（`is_mode_key_thumb_shift_passthrough`）、`:2177-2189`（`resolve_pending_thumb_as_single` の強制操作分岐）
- 欠陥: Shift 押下中の素通しガードは `*_solo_tap_ime_action` と `ModeKeyConfig::Passthrough` しか見ない。`cd77e455` で追加された `forced_open_action`（`keys.ime_on/off/toggle` に修飾なしの無変換/変換を書いた場合）は対象外。強制操作分岐もキー自体が修飾キーかしか見ず、Shift 押下を判定していない
- 失敗シナリオ: 親指キー=無変換、`keys.ime_toggle=["VK_NONCONVERT"]`、単独タップは既定 Suppress。Shift+無変換（GJI ATOK では「かな⇔半角英数」）→ 押下はチョード待ち保留 → 離した時に強制トグル発火 → `SetOpen(false)` と `EngineStateChanged{enabled:false}`。IME が OFF になり Shift+無変換は OS に届かない
- ADR-186 残る問題2 と同じ症状が新経路で再発。ADR-192 決定3b は「修飾なしで設定した場合のみ」前提で、特殊キー照合 `matches_key_combo`（Shift 一致まで要求）とも食い違う
- 再現: develop のコピーで使い捨てテスト（`make_test_engine()` + `set_thumb_forced_open_actions(Some(Toggle), None)`、shift=true で無変換の押下→解放）。押下時 consumed=true、解放時に `Ime(SetOpen{open:false, origin:ExplicitUserAction})`。既存6本（`2036228e`）に Shift 併用ケースなし
- 修正方針案: `is_mode_key_thumb_shift_passthrough` に `special.forced_open_action.is_some()` を加える、または強制操作分岐で `self.phys.modifiers.shift` を見て除外。`src/engine/tests.rs` に回帰テスト（fix-requires-evidence の キー選択ファミリー）

### C-2. 確認して問題なしとしたもの（参考）

BUG-160（`cbedf857`、既定 Suppress では挙動不変。Shift 素通し後に Shift を離して無変換を押したまま文字キーを打っても親指面にならず通常面 `う`、離した時の余計な出力もなし）/ `34678cdc`（`detect()` 振り分けと `WarningDialogTracker`）/ `41633fe1`（`extract_mode_keys` と集計単位・除外コマンド・分類1種類の基準が一致）/ `202d32de`（`Option` は `#[serde(default)]` 無しでも欠落が `None`、`read_dword` 簡略化も挙動不変）/ `111dfac4`（stderr は backslashreplace、`architecture_guard` は `PYTHONUTF8=1`）/ CI（ci.yml の windows-build/settings/package 分割・成果物パス・mutants 系2本・e2e-ime.yml の行列 41構成/119ジョブ ≤256・重複なし）/ Python（pyflakes で未定義名なし、`score_walk.py` に例外経路なし）/ `cargo test --lib`（1022件）・`--test scenarios`・`-p awase-gji-config` 全通過（コピー上）

## 着手優先度（案）

1. A-1（belief 破損・修正が1行+テスト、到達実績あり）
2. C-1（既定設定では起きないが、修正は小さく再現テストもある）
3. B-1（学習表の汚染検出が無効。ADR-196 T1 と合わせて）
4. B-2 / B-5 / B-4（学習表が不採用になる・進捗表示が狂う。ADR-196 T2〜T4 と合わせて）
5. B-6 / B-7（クラッシュ・フリーズ）
6. A-2（実機ログで発生条件の確認が先）
7. B-3 / B-8 / B-9 / B-10

修正時は各コミットで `.claude/rules/fix-requires-evidence.md`（回帰テストまたは `docs/known-bugs/BUG-NNN.md`）に従うこと。A-1/A-2 は IME belief ファミリー、C-1 はキー選択ファミリー、B-1 は学習系のため対象。
