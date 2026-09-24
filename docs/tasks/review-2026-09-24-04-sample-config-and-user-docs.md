---
title: 古いサンプル設定・利用者向け文書の撤去済みキー案内と、未知キー警告の不在
status: 未着手
created: 2026-09-24
related_adr: ["ADR-191", "ADR-094", "ADR-125", "ADR-116", "ADR-099"]
source_review: 俯瞰レビュー（受動化・actuation撤去・学習/較正・config棚卸し・v2方針、2026-09-24）の A-3 / A-9
---

# サンプル設定と利用者向け文書の整合（俯瞰レビュー A-3 / A-9）

索引・優先度: [review-2026-09-24-11-low-priority-backlog.md](review-2026-09-24-11-low-priority-backlog.md)。
裏取り基準は worktree の `5877f982`（PR #296 まで）。ここで挙げた設定・文書・コードは `cbae84ff` から変わっていない（`git diff --stat cbae84ff 5877f982` が該当ファイルで空）ので、行番号は両方で有効。着手時は `.claude/rules/worktree-per-session.md` に従い、専用の worktree と branch を切ること。

関連 ADR:
- ADR-191: 撤去キーの大半（`dbe_mode_key_policy`・`gji_thumb_key_ime_toggle`・`apply_calibrated_mode_keys`）を撤去した ADR。`output_mode`/`hook_mode` は 2026-07-06 の撤去（`src/config.rs:8-13` の NOTE）で、ADR-191 ではない。
- ADR-094: `conv_mode_policy` を撤去した ADR（`10f238b5`、`src/config.rs:14-19` の NOTE）。
- ADR-125: BUG-108 の設計元。
- ADR-116: 起動時設定診断。`validate()` の警告の表示先。
- ADR-099: 設定の保存（`AppConfig::save`）と、アップグレード時に設定を保つ方針。

## 現状（裏取り済み）

### A-3: 撤去済みのキーや存在しないセクションを案内している（読み込みでは黙って無視される）

**`config.sample.toml`**（git 管理。MSI・release.yml からは参照されないが、`awase.exe` の起動エラーメッセージが案内している。T1 を参照）
- `output_mode = "unicode"`（`:62`）: 撤去済み。
- 次のキーはフィールドとして存在しない。正しくは `[keys].engine_on` 等。
  - `engine_on_keys`（`:65`）
  - `engine_off_keys`（`:68`）
  - `ime_on_keys`（`:72`）
  - `ime_off_keys`（`:75`）
- `[ime_sync]`（`:110`）: 存在しない。正しくは `[keys.ime_detect]`。
- `# on = ["VK_DBE_HIRAGANA"]`（`:112`）: ADR-191 の「モードキーは書かず追随」と逆の案内。
- `[focus_overrides]`: 存在しない。正しくは `[app_overrides]`。
  - `:119` のセクション本体。
  - `:22` のコメント「下記 [focus_overrides] の force_bypass に…」。
- `speculative` は「廃止。two_phase で speculative_delay_ms=0」と注記済み（`:51-52`）で、問題ない。
- 元レビューが挙げた次の2点は**問題ではなかった**（詳細は末尾「レビュー反映メモ」）。
  - `left_thumb_key = "VK_NONCONVERT"`（`:33`）
  - `layouts_dir = "config"`（`:39`）

**`config.toml.sample`**
- `confirm_mode = "ngram_predictive"`（`:58`）: 実際の値として設定している。
- `speculative`（`:54`）: 確定モードの選択肢として説明している。
- `output_mode = "unicode"`（`:65`）、`hook_mode = "relay"`（`:70`）: どちらも撤去済み。
- `[keys.ime_detect] toggle = ["VK_KANJI"]`（`:103`）: 有効な行として書かれている。これは `src/config.rs:495-507` の `ImeDetectConfig` のコメントが「既定の `keys.ime_toggle=VK_KANJI` と併用すると二重処理で漢字キーが壊れる」と書いている組み合わせそのもの。
- `# [focus_overrides]`（`:108`）。
- 自分自身の `:4` に `cp config.toml.sample config.toml` と書いてある。

**同梱・埋め込みの `config.toml`**（`src/config.rs:1339` の `include_str!("../config.toml")`、`wix/main.wxs:122`）
- コメントで `speculative` を選択肢として残している（`:7`）。
- `# toggle = ["VK_KANJI"]`（`:27`）も上と同じ罠。

**`docs/usage.html`**
- `:543-544`「基本設定タブ」の表に「出力モード」「フックモード」の行がある。設定画面に存在しない UI 項目の説明で、config キーの問題とは別の誤り。
- `:729`: `output_mode = "unicode"`。
- `:757`、`:772`: `[focus_overrides]` の例。
- `:511`: 学習キャッシュのクリア（A-9 を参照）。
- 推奨確定モードが同じページ内で食い違っている（下の「推奨確定モードの食い違い」を参照）。

**英語版と awase.cc の文書**（元レビューの対象外だったが、同じ古い記述がある）
- `docs/usage.en.html`
  - `:543-546`「Basic Settings tab」の表: 日本語版と状態が違う。Confirm mode（`:543`）・Speculative-output wait（`:544`）が表に残っており（日本語版では撤去済みとして表から消え、`:548` の注記に置き換わっている）、Output mode（`:545`）・Hook mode（`:546`）もある。日本語版 `:548` の注記（「ほとんどのユーザーは既定の `wait`」）に当たる英文も無い。
  - `:511`: Clear learning cache。
  - `:560`、`:672-677`、`:945`: ngram_predictive を recommended としている。
  - `:701`: `output_mode`。
  - `:718`、`:733`: `[focus_overrides]`。
- `README.md`
  - `:20`: 「5 つの確定モード」。
  - `:119`: `output_mode` の表。
  - `:128-129`: `speculative` の説明。
- `README.en.md`
  - `:20`: Five confirm modes。
  - `:121`: `output_mode`。
  - `:130-131`: `speculative`。
- `docs/index.html`・`docs/index.en.html` 共通
  - `:468`、`:728`: 「5 種類の確定モード」/ five confirmation modes。
  - `:582`（日本語版の見出し「5 種類の確定モード」）。
  - `:585-597`、`:670`: ngram_predictive を推奨とし（`:586` の「（推奨）」を含む）、speculative も載せている。
  - `:673`、`:677`: `output_mode`/`hook_mode` の表。

**読み込み時の扱い**
- `AppConfig` は `deny_unknown_fields` 無しで、未知のキーは黙って捨てる。
- `validate(self)`（`src/config.rs:1306`）はデシリアライズ後の構造体を受け取る。未知キーはその時点で既に失われているので、`validate()` の中からは検出できない。
- 撤去キーのテストは3本ある。「警告しない」を意図した仕様として明記しているのは3本目だけ。
  - `test_removed_fields_are_tolerated`（`:1585`、`output_mode`/`hook_mode`）: doc は「パースが失敗しない（後方互換）」だけで、警告の有無には触れていない。
  - `test_removed_apply_calibrated_mode_keys_key_is_ignored_on_load`（`:2235`）
  - `test_removed_dbe_mode_key_policy_and_gji_thumb_key_ime_toggle_are_ignored_on_load`（`:2249`）: doc コメント `:2246-2247` が「読み込みエラーにも警告にもならず」と明記している。ADR-191 レビュー指摘 B-M3、`090c13d0`。
  - ADR-191 本文（`docs/adr/191-ime-is-source-of-truth-observe-not-write.md:403`）にも「旧config.tomlにキーが残っていても読める」とある。
- `speculative` はこれらとは性質が違う。
  - まだ有効な値で、`validate_thresholds` が `two_phase`（delay=0）に読み替える（`src/config.rs:958-975` 付近）。
  - 読み替えたときは警告を出す（テスト `:1999-2006`）。
  - したがって「黙って無視される撤去キー」には当たらない。
- 失敗シナリオ:
  1. 利用者が usage.html やサンプルを写して `[focus_overrides] force_bypass=[...]` を書く。
  2. バイパスは効かない。警告も出ない。

**`validate()` の警告の表示先**（ADR-116）
- `awase.exe`: `app/bootstrap.rs:1014` → `StartupDiagnostics::warn`（`app/mod.rs:90`）の順に流れる。
  - ログに `tracing::warn!` で出す。
  - `report()` がトレイバルーン「N件の警告があります」を出す。
- `awase-settings.exe`: `recompute_diagnostics` が `startup_diagnostics` に合流させ（`crates/awase-settings/src/main.rs:1488`）、`:1589` の折りたたみ見出しに表示する。見出しは、診断が1件でもあれば最初から開いた状態で表示される。

**設定画面で保存すると撤去キーは消える**
- `AppConfig::save` は構造体を `toml::to_string_pretty` で書き出す（`src/config.rs:871-874`）。
- 設定画面は「読み込み → 構造体を編集 → 保存」の流れ（`crates/awase-settings/src/main.rs:998` ほか）。
- そのため、一度保存した時点で未知キーと `[focus_overrides]` は黙って消える。警告が意味を持つのは、設定画面で一度も保存していない利用者だけ。

**推奨確定モードの食い違い**
- wait を推奨・既定としているもの:
  - `src/config.rs` の既定値 `Wait`。
  - `config.sample.toml` の「wait 推奨」。
  - `usage.html:548` の注記「ほとんどのユーザーは既定の `wait`」。これは設定画面から確定モードを撤去した理由として書かれている。
- ngram_predictive を推奨しているもの:
  - `usage.html:563`、`:695`、`:700`、`:1007`。
  - `config.toml.sample:58`。
  - 英語版と index の上記の行。

### A-9: トレイの「学習キャッシュをクリア」は何もしない（BUG-108、未修正）のに、文書が案内している

- コードの流れ:
  1. `tray.rs:666` がメニュー項目「学習キャッシュをクリア」を追加する。
  2. `:729` で `TrayCommand::ClearImmCache` に対応づける。
  3. `runtime/message_handlers.rs:1303` の `Some(tray::TrayCommand::ClearImmCache) | None => {}` に行き着き、何もしない。
- 本来消すべき対象は BUG-108 に書いてある: `ImmCapabilityStore` のメモリ上のキャッシュと、`cache.toml`（`focus/classifier.rs:50`）の `[imm_capability]` セクション。
- `docs/usage.html:511` と `docs/usage.en.html:511` は「IME の学習データキャッシュを削除します」と説明している。
- **新たに見つけた点: `CHANGELOG.md:90` と `docs/changelog.html:230`（1.19.0）が、この不具合を「修正」と書いている。** しかし次の2点から、修正は入っていない。
  - `git log -G"ClearImmCache" -- crates/awase-windows/src` に実装したコミットが無い。
  - BUG-108 の `fix_commits` は `[]`。
  - つまりリリースノートの記述が誤り。
- 学習表（`keymap-learn-table.json`、ADR-195）が加わり、「学習」という語が二重の意味を持つようになった。利用者は学習表が消えると思いうる。実際には何も消えず、BUG-108 の本来の対象も学習表ではなく `[imm_capability]`。
- BUG-108 の記述の古さ:
  - 行番号が古い（`tray.rs:649/697`、`message_handlers.rs:1141`）。
  - 本文は52行あり、`.claude/rules/fix-requires-evidence.md` の目安（30行以内）を超えている。

## タスク

### T1: サンプルを1本化する（設定ファイル・docs と、起動エラーメッセージの文言1箇所）
- [ ] `config.sample.toml` と `config.toml.sample` の両方を削除し、埋め込み `config.toml` のコメントに一本化する。
  - 理由1: どちらも MSI・README・release.yml・wix から同梱も参照もされていない（`.github/workflows/*.yml`・`wix/*.wxs` に該当なし）。唯一の利用者向け参照は下記の `main.rs` の起動エラーメッセージで、これは「同梱の」と書いている時点で既に誤り。
  - 理由2: `config.toml` は MSI 同梱かつ初回生成用で、実質的に公式のサンプルになっている。
  - 理由3: ほぼ全項目が古く、書き直す価値が薄い。
- [ ] 削除に伴い、参照している箇所を直す（**必須の依存**）。
  - `layout/nicola_keytop.yab:9` のコメント「config.sample.toml参照」を `config.toml` に向け直す。
  - このファイルは `include_str!` で埋め込まれている（`src/config.rs:1346`）。コメント行だけの変更なので、配列定義には影響しない。
  - `crates/awase-windows/src/main.rs:48`・`:51`（`startup_error_hint`、`#[cfg(windows)]`）の「同梱の config.sample.toml と見比べると…」「同梱の config.sample.toml をコピーして使えます」を直す。
    - `:48`（TOML 構文エラー）: 比較先を「初回起動時に生成された既定の `config.toml`」などに変える。
    - `:51`（config.toml が無い）: 通常の起動経路では `ensure_config_exists`（`src/config.rs:1371`）が埋め込み既定値から生成するので、ここに来るのは生成失敗か開発ビルドのとき。「通常は起動時に自動生成される。生成できない場合はフォルダの書き込み権限を確認」といった案内にする。
    - 文言の分類は `app/mod.rs:194` の `bail!` 文言と対応している（`main.rs:40-42` のコメント）。判定用の `contains` 文字列は変えないこと。
  - ADR・known-bugs・bug-reports-triage・CHANGELOG の過去の言及は、当時の記録なので直さない。
- [ ] 埋め込み `config.toml` のコメントを直す。
  - `:7` の選択肢から `speculative` を除く。
  - `:27` の `# toggle = ["VK_KANJI"]` を削除するか、「`keys.ime_toggle` と併用しないこと」という警告付きにする。

### T2: 利用者向け文書を直す（docs のみ）
- [ ] `docs/usage.html`・`docs/usage.en.html`・`README.md`・`README.en.md`・`docs/index.html`・`docs/index.en.html` を直す。
  - `[focus_overrides]` を `[app_overrides]` に直す。
  - `output_mode`・`hook_mode` を削除する。
  - 「5 つの確定モード」と `speculative` を選択肢として載せている箇所を現状に合わせる。
- [ ] 「基本設定タブ」の表を設定画面の現状に合わせる。
  - 日本語版 `usage.html:543-544`: 「出力モード」「フックモード」の2行を削除する。
  - 英語版 `usage.en.html:543-546`: Confirm mode・Speculative-output wait・Output mode・Hook mode の**4行**を削除し、日本語版 `:548` の注記の英訳を表の直後に足す。
- [ ] 推奨確定モードは **`wait` に揃える**。
  - 根拠: 現行の既定値であり、`usage.html:548` が書く設定画面からの撤去理由とも一致する。
  - 上に挙げた ngram_predictive 推奨の全行を直す。
  - v2 で ConfirmMode を2択にする際に推奨を変えるなら、[07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md) の ADR で覆す。ここで決め切るので、07 との循環は起きない。
- [ ] A-9 の文書側の暫定対応: T4 の実装が入るまで、`usage.html:511`・`usage.en.html:511` の説明を「現状は動作しません（BUG-108）」に直す。
  - `CHANGELOG.md:90`・`docs/changelog.html:230`・`docs/changelog.en.html:230`（「Fixed the tray's "Clear learning cache" menu item doing nothing when clicked」）の 1.19.0「修正」記述に、「実際には未修正だった（BUG-108）」と追記する。
  - リリース済みの記述の扱いは `release-develop-to-main` の運用に合わせる。

### T3: 撤去キーの警告（コード。ADR-191 の判断を覆す）
- [ ] 実装方式は次のどちらか。**第一案は (A)**。
  - (A) ダミーフィールド方式: 撤去キーを `#[serde(default, skip_serializing)] Option<toml::Value>` のダミーフィールドとして受ける。`validate()` で `is_some()` を見て警告する。
    - `validate()` の中で完結し、依存も増えない。
    - `skip_serializing` なので、設定画面で保存すると消える。
  - (B) `AppConfig::load`（`:847-852`）で生の `toml::Value` を別にパースし、既知の表と照合する。結果を `validate()` の warnings に合流させる。
    - 汎用の未知キー警告にするなら (B) か `serde_ignored`（現状どの Cargo.toml にも無く、依存の追加になる）が必要。
- [ ] 警告の対象は次の2グループ。
  - 撤去キー: `output_mode`、`hook_mode`、`conv_mode_policy`、`dbe_mode_key_policy`、`gji_thumb_key_ime_toggle`、`apply_calibrated_mode_keys`、`[focus_overrides]`。
  - サンプル由来の架空キー: `engine_on_keys`/`engine_off_keys`/`ime_on_keys`/`ime_off_keys`、`[ime_sync]`。
  - `speculative` は既に正規化と警告があるので対象外。
  - 各キーについて、撤去前にどのセクション配下だったかを実装前に確認すること。`conv_mode_policy` は `GeneralConfig` のフィールドだったので `[general]` 配下（`git show 10f238b5 -- src/config.rs` の `-    pub conv_mode_policy: ConvModePolicy,`、ADR-094）。
- [ ] 表示先は既存の ADR-116 の経路を使う（`awase.exe` のログとトレイバルーン、設定画面の起動時診断）。新しい表示経路は作らない。
- [ ] 設定画面で保存すると撤去キーは消える。その前に次のどちらかをする。
  - 起動時診断に「保存するとこの N 件は削除されます」と出す（第一案。文言を追加するだけで済む）。
  - 保存時に知らせる。
- [ ] ADR-191 が「警告しない」とした判断を覆すため、次を更新する。
  - テストの doc と名前: `…_are_ignored_on_load`（`:2235`・`:2249`）を「無視はするが警告は出す」に改める。特に `:2246-2247` の「警告にもならず」は仕様変更になる。3本とも「起動を続け、他の設定は読める」という性質は保つ。
  - `docs/adr/191-ime-is-source-of-truth-observe-not-write.md:403` に追記する（`docs/adr/191-*` は3ファイルあるので取り違えないこと）。

### T4: A-9 の実装
- [ ] 次の3案から選ぶ。
  - (a) メニュー項目を削除する。
  - (b) BUG-108 の本来の対象である `[imm_capability]` だけを消す（メモリ上のキャッシュと `cache.toml` の該当セクション）。
  - (c) (b) に加えて `cache.toml` のほかの学習キャッシュ（`[injection_mode]` 等）も消す。
- [ ] **学習表（`keymap-learn-table.json`）は、どの案でも消さない。**
  - 学習表は約20分かけて専有学習した成果物で、元レビュー C-4/C-5 が「消してよいデータではない」としている。
  - 学習表を消す操作が必要なら、設定画面の学習 UI で別に扱う（[02](review-2026-09-24-02-settings-status-display.md)・[07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md) の管轄）。
- [ ] 残すならメニュー名を「IME 制御の学習キャッシュをクリア」などにして、学習表と区別する。
- [ ] (b)/(c) の方針は、[07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md) の C-4（`save_section` の弱さ）とタスク(7)（`save_section` が読込に失敗したときの扱い）と整合させる。07 C-4 に「cache.toml を消す人はめったにいないという前提は、04 A-9 のクリアメニューを機能させた時点で崩れる」とあり、07 `:115` も「04 A-9 のクリアメニューは 07(7) の判断を参照してよい（07 → 04）」としているので、07(7) の判断が先。
- [ ] BUG-108 を更新する。
  - `fix_commits` を追記する。
  - 行番号を `tray.rs:666/729`・`message_handlers.rs:1303` に直す。
  - 1.19.0 のリリースノートが誤っていたことを1行記録する。
  - 本文を30行の目安に近づける。

## 受け入れ条件

- **ホスト Linux（`cargo test --lib`、ルート `awase` クレート）**
  - T3: 撤去キーと架空キーの各1件について、`validate()` が警告を返すテスト。
  - T3: 既存3本の撤去キーテスト（`:1585`、`:2235`、`:2249`）が、「読み込みは成功し、他の設定は読める」を保ったまま通ること。
  - T1: 埋め込み `config.toml`（`EMBEDDED_CONFIG_TOML`）でテストする。
    - コメントを外した全ての例示行を `AppConfig` として読む。
    - 入力の全キーのパスが再シリアライズ後にも残っていること（未知キー0件）を確かめる。
    - 例示行だけのパースでは確かめられないので、これとは別に source-scan のガードを置く。`config.toml` と T2 の対象文書に `focus_overrides`・`output_mode`・`hook_mode`・`speculative`（`config.toml` のみ）・`ime_sync`・`engine_on_keys` が現れないことを文字列走査で確認する（`architecture_guard.rs` と同じ流儀。置き場所はルートクレートの `tests/`）。
- **Windows target の compile check と CI**: T1 の `main.rs` の文言変更と T4 のトレイ変更は Windows 専用コード（`#[cfg(windows)]`）。
  - `cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows -p awase-settings` が通ること。
  - `windows-build` CI が通ること。
- **目視**: T2 の6文書の該当箇所が現行の設定名と一致すること。推奨確定モードの表記が wait に揃っていること。
- **実機**: T4 で (b)/(c) を選んだ場合、`cache.toml` の該当セクションが消え、再学習されることを確認する。T3 の設定画面の警告表示も、実機で1回目視する。

## 他ファイルとの依存

- 本タスク → [07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md): 07 の ConfirmMode 2択化の前提は、T2 の推奨モード統一（wait）。本タスクが先に行う。
- [07](review-2026-09-24-07-adr176-fate-and-v2-cache-toml.md) → 本タスク T4: A-9 の (b)/(c) は、07 のタスク(7)（C-4）の判断の後に行う。(a) と T2 の文書の暫定対応は、07 を待たずに進めてよい。
- [02](review-2026-09-24-02-settings-status-display.md): 学習表を消す操作を設けるなら、02 の設定画面側の管轄。
- T1・T2 は他のタスクと独立に先に進められる（[11](review-2026-09-24-11-low-priority-backlog.md) の着手順どおり）。ただし T1 は docs と設定ファイルに加えて `main.rs` の起動エラーメッセージの文言1箇所（Windows 専用コード）を含む。11 `:47` の「04 の T1・T2（docs と設定ファイル）」はこれに合わせて直す必要がある（11 側の担当で対応）。

## 未確認点

- なし（前回の2件は再確認レビュー R-3 で解消し、本文 T2・T3 に移した）。

## レビュー反映メモ（Opus タスク文書レビューへの対応）

反映した指摘:
- 1-a（`output_mode` の行番号 `:62`）。
- 1-b（`engine_off_keys`/`ime_on_keys`/`ime_off_keys` の列挙漏れ）。
- 1-c（撤去キーテストの残り2本と「警告しない」を意図した仕様であること）。
- 1-d（推奨モードの食い違いの全行）。
- 1-e（`speculative` の性質の違い）。
  - 裏取りで補足: `speculative` は無視されるのではなく警告付きで正規化される（テスト `:1999-2006`）。
- 2-a（英語版と index の追加）。
- 2-b（基本設定タブの UI 説明の誤り）。
- 2-c（related_adr）。
  - ADR-158 は A-3/A-9 との関係を本文で示せないので外した。
  - ADR-125（BUG-108）を加えた。
  - 保存・診断経路の根拠として ADR-099・ADR-116 を加えた。
- 3-a（参照元の調査結果と、`layout/nicola_keytop.yab:9` の依存）。
- 3-b（`validate()` の中では検出できないこと）。
- 3-c（保存すると消えること）。
- 3-d（表示先。ADR-116 の経路としてコードで確認し、「未確認」から確定に変えた）。
- 3-e（テストをパースと source-scan の2つに分けた）。
- 3-f（wait に決め切った）。
- 3-g（消す対象を BUG-108 の `[imm_capability]` と明記し、学習表は消さない）。
- 3-h（Windows の compile check と CI の条件）。
- 4-a（ADR-191 の判断を覆す手順）。
- 4-b（ダミーフィールド方式を第一案として採用し、架空キーも警告対象に加えた）。
- 4-c（両サンプルを削除して一本化）。
- 5-a、5-b（書式を兄弟文書に揃えた）。
- 5-c（BUG-108 は52行で目安超過。T4 に含めた）。

反映しなかった点・訂正した点:
- 元の文書（とレビューがそのまま「一致」とした）「`left_thumb_key = "VK_NONCONVERT"`（`:33`）、`layouts_dir = "config"`（`:39`、既定と違う）」は**誤り**なので削除した。
  - `GeneralConfig::default()` の `layouts_dir` は `"config"` で、既定と同じ（`src/config.rs:417`）。
  - `"VK_NONCONVERT"` は既定値 `"無変換"` の別名として受け付けられる（`THUMB_KEY_ALIASES` `src/config.rs:1096-1097`、`crates/awase-windows/src/vk.rs:587`）。
  - これに伴い、旧「未確認点」2件目（`left_thumb_key` の許容値）は解消した。
- 裏取り基準は、レビューの `cbae84ff` から現 HEAD `5877f982` に更新した。
  - 間に入った PR #293〜#296 は、keymap-learn・tuning と ADR-196 の文書だけの変更で、本タスクの対象ファイルに差分は無い。
  - そのため「解消済み」に変わった項目は無い。
- 裏取りで新しく見つけた点: 1.19.0 のリリースノート（`CHANGELOG.md:90`、`docs/changelog.html:230`）が BUG-108 を「修正」と書いているが、実装は無い。T2・T4 に加えた。

再確認レビュー（R-1〜R-7）への対応（すべて `5877f982` で裏取りして反映）:
- R-1: `crates/awase-windows/src/main.rs:48/:51` が `config.sample.toml` を案内しているのを確認（`grep -rn config.sample.toml` で ADR・known-bugs・CHANGELOG・タスク文書以外の該当はこの2行のみ）。`main.rs` の修正は T1 に含め、見出し・理由1・受け入れ条件の Windows compile check・依存節を直した。`:51` の代替文言は、`ensure_config_exists`（`src/config.rs:1371`）が通常は既定値を自動生成することを踏まえた案にした。11 `:47` の追随は 11 の担当に委ねる（本作業は担当ファイル以外を編集しない）。
- R-2: `usage.html:546` は表の「レイアウト」行、注記は `:548` と確認し2箇所訂正。英語版 `usage.en.html:543-546` の4行と注記の欠落を確認し、T2 に明記した。
- R-3: `src/config.rs:14-19` と `10f238b5` の diff、`docs/changelog.en.html:230` を確認し、未確認点を解消して本文へ移した。
- R-4: `src/config.rs:8-13`（2026-07-06 撤去）と `:1582-1583` の doc（警告の有無に触れない）を確認。関連 ADR の説明を書き分け、ADR-094 を related_adr に加えた。「警告しない」を明記しているのは `:2249` の1本だけと書き分けた。
- R-5: `docs/adr/191-*` が3ファイルあり、`:403` が `191-ime-is-source-of-truth-observe-not-write.md` の該当行であることを確認して2箇所にファイル名を書いた。
- R-6: `docs/index.html:468/582/586/670/728` を確認し、index の列挙を ja/en 共通の1まとめに直した。
- R-7: 反映したが、指摘の前提を一部訂正した。07 の C-4 は「`save_section` の弱さ」で、本文にあった「cache.toml を『消してよいデータ』と分類するか」は C-4 の説明として不正確だった（07 `:47`）。04 A-9 の依存先は 07 のタスク(7)（読込失敗時の扱い、07 `:86`・`:115`）なので、「07 の C-4（`save_section` の弱さ）とタスク(7)」と両方を書いた。
