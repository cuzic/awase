---
title: 設定画面の「使用中: 学習表」表示が awase.exe の実際の採否と食い違う
status: 未着手
priority: 次リリース前（俯瞰レビュー A-2【重大】、同 D節の優先度2位）
created: 2026-09-24
related_adr: ["ADR-196", "ADR-195"]
source_review: 俯瞰レビュー（2026-09-24）の A-2
---

# 設定画面の状態表示の誤り（俯瞰レビュー A-2）

索引・優先度: [11](review-2026-09-24-11-low-priority-backlog.md)。
裏取り基準は worktree の `5877f982`（PR #296 まで）。起票時の基準 `cbae84ff` から本文で引用するファイル
（`crates/awase-settings/`・`crates/awase-windows/src/state/`・`runtime/key_pipeline.rs`・`app/`）に差分が無いことを
`git diff --stat cbae84ff 5877f982` で確認済みなので、行番号はそのまま有効。
着手時は `.claude/rules/worktree-per-session.md` に従い専用 worktree/branch を切ること。

注: `docs/tasks/develop-weekly-code-review-2026-09-23.md` の「A-2」は `state/mode_key_pass.rs` の通過マーク窓の話で、本件とは無関係（ラベルが同じだけ）。

## 背景

[ADR196-T4](adr196-t4-ui-status-and-adoption.md) は設定画面に予測表の状態行（内蔵表/学習表/要再検証/不採用/要確認/予測なし）を出す。
この表示は、awase.exe が実際にどの表を予測に使っているかと独立に決まっている。

## 現状（裏取り済み）

- `crates/awase-settings/src/keymap_learn_status.rs::TableState::from_inputs`（`:70`）は、`judgement==Accepted` だけで
  `Learned`（`:111`、「使用中: 学習表（…）」）か `NeedsRevalidation`（`:104`、「使用中: 学習表（要再検証…）」）を返す。
  カバレッジ・不一致率・opt-out・IME種別は見ていない。
- awase.exe 側は `crates/awase-windows/src/state/key_effect_runtime.rs::validate_and_convert`（`:473`）で採否を決める。
  awase.exe が学習表を使わないのに、画面が「使用中: 学習表」（または要再検証）と出る条件は次の5つ。
  1. 内蔵表との不一致率が5%超で棄却（`key_effect_runtime.rs:488-491`、`MAX_MISMATCH_RATIO`）。
     `check_against_bundled`（＝キーマップが同梱表そのまま）のときだけ効く。
     [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md) がこの判定の見直しを扱う。
  2. カバレッジ80%未満で棄却（`key_effect_runtime.rs:485`、`MIN_COVERAGE_RATIO` は `:48`）。
  3. `general.use_learned_keymap_table=false`（`src/config.rs:403`、既定 true）。awase.exe は
     `runtime/message_handlers.rs:1409-1411` 等で参照しているが、`crates/awase-settings/src/` には参照が0件。
  4. 使用中のIMEが GJI でも Microsoft IME 本体（TIP の CLSID 一致、`ms_ime_native_identified`）でもない場合。
     ATOK・Japanist・未知の TIP・IMM32 HKL が該当し、予測自体が行われない
     （`crates/awase-windows/src/runtime/key_pipeline.rs:1967` の `ActiveImeKind::MicrosoftIme => return`、根拠は直前 `:1950-1953` のコメント）。
     この判定は**フォーカス中アプリの** IME で決まり、フォーカスが変わるたびに変わる。
  5. （優先度低）表ファイルの探索規則の違い。awase.exe の `table_file_path()`（`key_effect_runtime.rs:370-371`）は
     `app::find_config_path()` を使い、コマンドライン引数の config パスを優先する（`app/mod.rs:185-188`）。
     設定画面の `load_keymap_table_state`（`crates/awase-settings/src/main.rs:585`）は `resolve_relative_to_exe("config.toml")` 固定。
     設定画面自身は引数を考慮する `find_config_path()`（`main.rs:6004-6006`）を持っているのに、ここでは使っていない。
     コメント（`:578`）の「awase.exeと同じ探索規則」は誤り。ほかに、4MB（`MAX_TABLE_FILE_BYTES`、`:44`）超過の
     `TooLarge` 棄却も設定画面は見ていない。
  6. （予告、[06](review-2026-09-24-06-keymap-learn-staleness-wiring.md) の配線後）キーマップ変更による学習表の失効。
     06 が入ると awase.exe は内蔵表へ切り戻すが、画面は「使用中: 学習表」のままになる。
- `status_line(None)` が固定で呼ばれている（`main.rs:3433`）。ただし渡すべき「内蔵表の測定環境」の値は、まだコードに無い。
  `crates/awase-windows/src/state/key_effect_table.rs:8` の生成コードの**コメント**に
  `// 測定環境: 不明(grid-tables/measurement-env.json が無いか読めない…)` とあるだけで、Rust の定数・関数は無い。
  `measurement-env.json` もリポジトリに無い（`git ls-files` で0件）。
  [adr196-t3-bundled-table-versioning.md](adr196-t3-bundled-table-versioning.md) も実際の版取得は未実装。
  なので現時点では `None` で正しい。
- `keymap_learn_status.rs:3` のモジュール doc が、実在しない `TableState::from_table` を参照している（実体は `from_inputs`）。

## 方針（設計）

- 採否判定は awase.exe と同じ関数を**直接呼ぶ**。`awase-settings` は既に `awase-windows` に依存している
  （`crates/awase-settings/Cargo.toml:15`）。`validate_and_convert` は cfg なしの `pub fn` で、`awase_windows::state::key_effect_runtime` から呼べる
  （`lib.rs:37` の `pub mod state`、`state/mod.rs:128` の `pub mod key_effect_runtime`）。
  Linux ホストでも呼べるので、判定を別クレートへ切り出す必要は無い。
  01 で判定が変わっても、表示は自動で追随する。
- 問題は `validate_and_convert` の引数 `(preset, check_against_bundled)` の出し方。awase.exe はこれを `KeyEffectKeymap` から求める
  （`runtime/key_pipeline.rs:1978` の `keymap.is_unmodified_bundled_config()`。keymap は GJI なら
  `gji_charset_autodetect::read_key_effect_keymap`、MS-IME 本体なら `msime_key_assignment::read_key_effect_keymap_native`）。
  この2関数はどちらも `pub(crate)`（`gji_charset_autodetect.rs:334`、`msime_key_assignment.rs:249`）で、どちらも `#[cfg(windows)] mod windows_impl` の中にある
  （GJI 側 `:278-279`、MS-IME 本体側 `:135-136`〜`:361`）。
  設定画面から呼ぶには、`awase-windows` 側で公開用の薄いラッパーを足す必要がある。
  既存の `probe_custom_keymap_without_prediction`（`awase-keymap-learn-win/src/env_version.rs:158`）が使う
  `bundled_preset_for_adjudication` は判定目的が違う関数（`BundledPresetLookup` を返す）で、awase.exe と同じ値になる保証は無い。流用しない。
- 条件4（IME種別）は、設定画面が awase.exe と同じ値を得る手段が無い。設定画面は `query_tip_identity_on_current_sta`
  （`env_version.rs:168`）で**自分のスレッドの** TIP を見るだけ。
  採るのは次のどちらか: (a) 設定画面起動時に見た IME で近似し、近似であることを文言に出す。(b) スコープ外にする。
  既存の `TableState::NoPrediction`（`:171`、「予測表なし（カスタムキーマップ）— 学習を推奨」）は意味が違うので流用しない。(a) にするなら新しい variant を足す。

## タスク

- [ ] （先にタスク「条件4の扱いを決める」を済ませる。どちらのラッパーを呼ぶかの根拠が条件4の判定と同じなので）
  `awase-windows` に、設定画面から `(preset, check_against_bundled)` を得る `pub` ラッパーを足す（GJI／MS-IME 本体それぞれ、`#[cfg(windows)]`）。
  - 呼び出し場所: config1.db・レジストリの読み込みはブロックしうるので、UI スレッドで呼ばない。
    `refresh_keymap_table_state`（`crates/awase-settings/src/main.rs:1180-1203`）の `#[cfg(windows)]` ワーカースレッドで呼び、
    結果を `keymap_learn_status::EnvSnapshot` に載せて受け取る（既存の `probe_custom_keymap_without_prediction` と同じ形）。
  - どちらを呼ぶか: 同じワーカースレッドで `query_tip_identity_on_current_sta` の結果を見て選ぶ（条件4の (a) の近似と同じ判定）。
  - 取得できないとき（非 Windows の `EnvSnapshot::UNKNOWN`、GJI・本体以外の IME）は採否判定を省き、現状の表示に「近似」の注記を付ける。
- [ ] `StatusInputs` に、`validate_and_convert` の結果（`Result<(), RejectReason>` 相当）と `use_learned_keymap_table` を渡すフィールドを足す。
  呼び出し側（`load_keymap_table_state`）で、`EnvSnapshot` から受け取った `(preset, check_against_bundled)` を使って `validate_and_convert` を呼ぶ。
  棄却時は `Learned`・`NeedsRevalidation` にせず、「内蔵表（学習表は不採用: 理由）」の状態にする。
- [ ] `use_learned_keymap_table=false` のときは新 variant（例: `LearnedDisabled`、「学習表は設定で無効化されています」）を返す。
  設定画面のアプリは `self.config` を持っている（`main.rs:682`・`:838` のコメント参照）ので、値を渡すのは容易。
  この設定を設定画面から切り替えられるようにするかはスコープ判断。`src/config.rs:400-401` の doc コメントに
  「awase-settingsのUIチェックボックスは未実装、フォローアップが必要」とあり、追加する予定自体はコードに書かれている。
- [ ] `load_keymap_table_state` の config パス解決を、設定画面の `find_config_path()`（引数考慮）に揃え、`:578` のコメントを直す（条件5）。
  4MB 超過（TooLarge）の扱いも揃える。公開作業は要らない: `key_effect_runtime::load_runtime_table(path, preset, check)`（`:442`）は
  cfg なしの `pub fn` で、TooLarge・Parse・スキーマ不一致を含めて awase.exe と同じ棄却理由を返す。`judgement` 等の表示用に
  `PersistedTable` も要るなら、`read_persisted_table`（`:456`、これも `pub`）→ `validate_and_convert` の順に呼ぶ。
  これで設定画面独自の `std::fs::read_to_string` + `persist::from_json`（`main.rs:590-592`）も置き換えられる。
- [ ] 条件4の扱いを (a)/(b) から決め、決定をこのファイルに書く。
- [ ] 06 の配線後: 06 の失効判定を状態表示にも通す（条件6）。判定が `validate_and_convert` の中に入るなら自動で追随する。
  外に出るなら `StatusInputs` にフィールドを足す。
- [ ] `keymap_learn_status.rs:3` の doc の `from_table` を `from_inputs` に直す。
- [ ] 内蔵表の測定環境の表示（`status_line(None)` の置き換え）は、ADR196-T3 が preset ごと（ATOK・MSIME・MSIME_NATIVE の3表）の測定環境を
  `pub const` か生成関数として出力するまで**着手しない**。それまでは `None` のままで正しい。

## 受け入れ条件

- 自動テスト（Linux ホストで実行可。`cargo test -p awase-settings --bin awase-settings keymap_learn_status`。
  CI の `nextest run --workspace --lib` は bin の awase-settings を含まず、`windows-build` も `--tests` のビルドまでなので、ローカル実行で確認する。
  このコマンドが Linux で通ることは本改訂時点では**未実行・未確認**）:
  `TableState::from_inputs` のテストに、`judgement==Accepted` の表で次の各ケースが `Learned` にも `NeedsRevalidation` にもならないことを追加する。
  - 条件1（不一致棄却）
  - 条件2（カバレッジ不足）
  - 条件3（`use_learned_keymap_table=false`）
  - 条件4は (a) を採る場合だけ、`StatusInputs` の IME種別フィールドを使って追加する。
  - ラッパーと IME 判定は `#[cfg(windows)]` なので、Linux の単体テストは `from_inputs` に判定結果を渡す形で書く。
- 実機確認（Windows 実機、GJI 既定構成）:
  - `keymap-learn-table.json` のセルを手で書き換えて不一致率を5%超にする。awase.exe ログの `[key-effect-runtime] 学習済み表を不採用` と、設定画面の状態行（不採用＋理由）が一致することを見る。
  - 01 で5%判定が撤去された場合は、代わりにセルを削除してカバレッジを80%未満にした構成で同じ確認をする。
  - 条件3は単体テストで十分なので、実機確認はしない。

## 他ファイルとの依存

- [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md) とは**独立に着手可**。`validate_and_convert` を直接呼ぶので、01 で判定が変わっても表示は追随する。
  01 が採用フラグを `validate_and_convert` の外に分離した場合だけ、本タスクの呼び出しを追随させる（向き: 02 が 01 の結果に追随する。01 は 02 を待たない）。
- 01 の暫定策（「学習結果を使う」ボタンを隠す／注意文を出す）は同じ画面なので、実装時に衝突しないよう順序を合わせる。
- 次リリース前に [01](review-2026-09-24-01-adr196-adoption-and-mismatch-check.md)・[03](review-2026-09-24-03-release-bundle-keymap-learn-win.md) とセットで片付ける（索引 11 の推奨順）。
- [06](review-2026-09-24-06-keymap-learn-staleness-wiring.md) → 02: 06 の失効判定ができたら、同じ判定を状態表示にも通す（条件6、索引 11 の「06 → 02」）。
  それ以外の部分は 06 を待たずに着手してよい。
- [04](review-2026-09-24-04-sample-config-and-user-docs.md) は「学習表を消す操作」を 02・07 の管轄としている（04 `:191`・`:219`）。
  本ファイルのタスクには含めていない（未確認点参照）。
- 測定環境の表示は ADR196-T3（[adr196-t3-bundled-table-versioning.md](adr196-t3-bundled-table-versioning.md)）に依存する（向き: 02 が T3 を待つ）。

## 未確認点

- 学習表を消す操作を設定画面に設けるか（04 から 02・07 の管轄とされている）。07 の C-4（cache.toml の分類）の判断待ち。
- 設定画面起動時の TIP 判定が、awase.exe のフォーカス先 IME と実運用でどの程度食い違うか（条件4の (a) を採る場合の近似の精度）。

## レビュー反映メモ（2026-09-24、Opus 批判的レビュー R1〜R12）

- R1（週次レビュー A-2 と同名の別視点という記述は誤り）: 反映。週次 A-2 は `mode_key_pass.rs` の話であることを確認し、注記に置き換えた。
- R2（判定は直接呼べる、切り出し不要）: 反映。ただし「`probe_custom_keymap_without_prediction` と同じ経路で preset を得る」という修正案は採らなかった。
  awase.exe は `KeyEffectKeymap::is_unmodified_bundled_config()` から `(preset, check_against_bundled)` を得ている。一方 `bundled_preset_for_adjudication` は別目的の関数で、値が一致する保証が無い。
  awase.exe と同じ値を得る `read_key_effect_keymap`・`read_key_effect_keymap_native` は `pub(crate)` なので、公開ラッパーが要る。これをタスクに加えた。
- R3（測定環境の埋め込み値は存在しない）: 反映。`key_effect_table.rs:8` がコメントだけであること、`measurement-env.json` が未追跡であることを確認した。
- R4（`from_table` は誤り、正しくは `from_inputs`）: 反映。doc の修正もタスク化した。
- R5（条件4の範囲・設定画面では同値を得られない・NoPrediction は別の意味）: 反映。(a)/(b) の選択タスクにした。
- R6（01 への依存が強すぎる）: 反映。
- R7（探索規則の違い）: 反映。設定画面が引数考慮の `find_config_path()` を既に持っていることを追加で確認し、それに揃える形にした。
- R8（`NeedsRevalidation` も対象）: 反映。awase.exe は env 版を見ずに `Accepted` なら採用するので、採用されている場合の「要再検証」表示自体は正しい。棄却時だけを対象にした。
- R9（実機手順）・R10（フルパス）・R11（worktree 文言・優先度）・R12（タスク2の重複・具体化）: 反映。
- R11 の書式の件（docs/tasks の既存書式は frontmatter なし）: シリーズ内で揃っているので frontmatter は維持した。索引側の注記は 11 の担当範囲なので、本ファイルでは扱わない。
- 追加の確認: PR #293（`d00ac8dd`）は `awase-keymap-learn-win/src/main.rs` だけの変更で、本件の条件は解消していない。

## レビュー反映メモ（2026-09-24、再確認レビュー N1〜N6）

- N1（06 への依存が抜けている）: 反映。06 `:75`・11 `:24` の「06 → 02」を確認し、条件6（予告）・タスク・依存節に追加した。
- N2（ラッパーの呼び出しスレッドと IME の選び方が未定）: 反映。`refresh_keymap_table_state`（`main.rs:1180-1203`）がワーカースレッドで
  `EnvSnapshot` を返していることを確認し、ラッパーをそこで呼ぶ・条件4の判定で選ぶ・取得できないときは注記、の3点と、条件4の決定を先に行う順序を書いた。
- N3（cfg(windows) は両方）: 反映。`msime_key_assignment.rs:135` の `#[cfg(windows)]`、`mod windows_impl` が `:361` まで続き `:249` を含むことを確認した。
- N4（`read_persisted_table`・`load_runtime_table` は既に `pub`）: 反映。`key_effect_runtime.rs:442`・`:456` を確認した。
- N5（未確認点1はコードで解消）: 反映。`src/config.rs:400-401` の doc コメントを確認し、未確認点1を削除してタスク3に移した。
- N6（`main.rs:630` の引用が弱い）: 反映。`:630` 付近は `config: &mut AppConfig` を引数に取る別関数だったので、`self.config`（`:682`・`:838`）に差し替えた。
  01 側の矢印表現の補足は 01 の担当なので本ファイルでは変えない。04 の「学習表を消す操作」の管轄は依存節と未確認点に一行ずつ足した。
