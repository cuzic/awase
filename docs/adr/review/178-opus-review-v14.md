---
id: ADR-178-companion-178-opus-review-v14
title: |-
  ADR-178（MSIアンインストール時のユーザーデータ保護、v14全面差し替え）Opus敵対的レビュー
type: companion-doc
related_adr:
  - "ADR-178"
---

# ADR-178 v14 敵対的レビュー（実装済み・実機検証済み版）

レビュー対象: `docs/adr/178-msi-uninstall-preserve-userdata.md`（v14）、
コミット `5e6f815a`（ADR差し替え）・`f5cba1ea`（実装）・`3d7a7ece`（実機発見バグ修正）・
`cb99ea24`（実機検証記録）。ブランチ `adr/178-msi-uninstall-preserve-userdata`
（`origin/develop` に対し ahead 17 / behind 5）。

## 検証のために実際に走らせたもの

- `cargo test --lib ensure_` → 4件 pass（新規単体テスト）。
- `cargo test -p awase-windows --test wix_installer_guard` → 7件 pass
  （新規 `config_file_and_nicola_yab_components_have_permanent` 含む）。
- `cargo clippy --target x86_64-pc-windows-msvc -p awase -p awase-windows -p awase-settings -- -A clippy::cargo`
  → ADR-178 由来の新規警告 **0件**（残る4件は develop から持ち越しの ADR-176 段階実装
  dead_code）。
- 埋め込み既定値と MSI 同梱物の一致確認: `.github/workflows/release.yml:66-67` が
  `cp config.toml dist/` / `cp layout/*.yab dist/layout/` であり、`include_str!` が参照する
  リポジトリ実ファイルと**同一ソース**。決定3が「CIでのバイト一致検証は不要」と言うのは
  正しい（同じファイルをコピーしているので構造的に一致する）。ここは v13 より明確に良い。

以下、**Blocker 2件 / Major 7件 / Minor 6件**。

---

## Blocker

### B1. 不可逆な半分（`Permanent`）だけがテストで固定され、それを安全にしている可逆な半分（自己修復の配線）が完全に無防備

v14 の安全性は「MSI がファイルを配置しなくても、アプリが起動時に生成する」という
**決定2の配線が存在し続けること**に全面的に依存している。ADR 148-155 行が自ら
「round1 B1 の実害は決定2の自己修復ロジックが無効化する」と書いているとおり、
自己修復は `Permanent` 化の副作用に対する**唯一の解毒剤**である。

ところが:

- MSI 側（`wix/main.wxs` の `Permanent="yes"`）は
  `crates/awase-windows/tests/wix_installer_guard.rs::config_file_and_nicola_yab_components_have_permanent`
  で固定され、assert メッセージまで丁寧に書かれている。
- 解毒剤側は**ゼロ**。次の4行のどれを消しても、どのテストも落ちない:
  - `crates/awase-windows/src/app/mod.rs:164` `ensure_default_config_exists();`
  - `crates/awase-windows/src/app/bootstrap.rs:237` `ensure_default_layouts_exist(&config.general.layouts_dir);`
  - `crates/awase-settings/src/main.rs:544` `ensure_default_config_exists();`
  - `crates/awase-settings/src/main.rs:556` `ensure_default_layouts_exist(&config.general.layouts_dir);`
- さらに悪いのは、`bootstrap.rs:237-238` は**行の順序そのものが正しさの条件**になっている
  点である。

  ```rust
  ensure_default_layouts_exist(&config.general.layouts_dir);   // 237: 生成（exe_dir固定）
  let layouts_dir = resolve_relative(&config.general.layouts_dir); // 238: 読み取り先の解決
  ```

  この2行を入れ替える、あるいは「引数を揃えよう」というリファクタで 238 の結果を 237 に
  渡し直すと、`3d7a7ece` が実機で踏んだバグがそのまま再発する。再発時の症状は ADR 208-210 行が
  書いているとおり **`awase.log` に何の警告も出ない**（`resolve_relative_to_exe` の CWD
  フォールバック warn は出るが、生成の失敗としては出ない）。
- 対象4箇所はすべて `#[cfg(windows)]` 配下（`crates/awase-windows/src/lib.rs:42-58` のモジュール
  ゲート、および awase-settings の Windows 前提コード）なので、Linux CI では**型チェックすら
  通らない**。CLAUDE.md が警告している「`#[cfg(windows)]` 配下の `#[cfg(test)]` は
  ネイティブ Linux テストバイナリに存在すらしない」という罠の直撃圏内にある。

そして `Permanent` は不可逆なので、解毒剤を失った状態は**次のバージョンで直せない**
（round1 B1 の「Permanent を外した MSI を出しても既に登録済みの extra system client は
残る」）。「守るべきものの重要度」と「守られている度合い」が完全に逆転している。

ADR 214-220 行は今回のバグから「**どんな設計でも『生成・書き込み先は exe 隣に明示的に
固定し、存在依存の解決関数の結果を書き込み先として使わない』という不変条件は必要**」
という教訓を抽出しているが、その不変条件を担保しているのは**doc コメントだけ**である。
v2〜v13 で同型の問題が通算7回再発したのは、まさに doc コメントで担保しようとしたからで、
v14 で8回目を踏んだ事実がそれを証明している。

`.claude/rules/fix-requires-evidence.md` の観点でも、`3d7a7ece` は挙動を変える fix で
ありながら (a) 回帰テストも (b) `docs/known-bugs/BUG-NNN.md` も**どちらも添えていない**
（`git show --stat 3d7a7ece` = settings/bootstrap/mod.rs/ADR の4ファイルのみ）。
本ルールが列挙する再発ファミリー表には載っていない領域だが、ADR 本文自身が
「7回再発した問題が8回目を踏んだ」と宣言している以上、実質的に同じ扱いをすべき対象である。

**要求（マージ前）**: ソーススキャン型のガードを1本追加すること。
`crates/awase-windows/tests/architecture_guard.rs` の既存パターン（テキスト検査、Linux 実行可）で足りる:

1. `app/bootstrap.rs` 内で `ensure_default_layouts_exist(&config.general.layouts_dir)` の
   出現位置が `resolve_relative(&config.general.layouts_dir)` より**前**であること。
2. `app/mod.rs::find_config_path` 本体に `ensure_default_config_exists()` が含まれること。
3. `ensure_default_layouts_exist` / `ensure_default_config_exists` の本体に
   `resolve_relative` / `resolve_relative_to_exe` / `resolve_layouts_dir` が
   **出現しない**こと（＝解決関数の結果を書き込み先に使わない、という不変条件そのもの）。
4. `crates/awase-settings/src/main.rs::SettingsApp::new` に両方の呼び出しが含まれること。

いずれも assert メッセージに「これが消えると round1 B1（再インストールしても config が
戻らず起動不能）が**不可逆に**復活する」と書くこと。

---

### B2. `default_config()` の `layouts_dir = "config"` により、config.toml が壊れている環境で awase-settings が `%LOCALAPPDATA%\awase\config\` に `.yab` を6本生成する

`crates/awase-settings/src/main.rs:546-557`:

```rust
let config_path = find_config_path();
let (config, config_load_state) = match awase::config::AppConfig::load(&config_path) {
    Ok(cfg) => (cfg, ConfigLoadState::Loaded),
    Err(e) => { ...; (default_config(), state) }      // ← フォールバック
};
if cli_arg_config_path().is_none() {
    ensure_default_layouts_exist(&config.general.layouts_dir);   // ← その config を書き込み先に使う
}
```

- `default_config()` は `crates/awase-settings/src/main.rs:5543-5545` の
  `toml::from_str("[general]").unwrap()`。`GeneralConfig` は `#[serde(default)]`
  （`src/config.rs:123-124`）なので、`layouts_dir` は serde default
  = **`"config"`**（`src/config.rs:477`）になる。出荷時の `config.toml:4` は `"layout"`。
- したがって `ensure_default_layouts_exist("config")` →
  `exe_dir.join("config")` → **`%LOCALAPPDATA%\awase\config\` に同梱6本を新規生成**する。
- `scan_layout_names`（`main.rs:5524-5539`）も `resolve_layouts_dir("config")` 経由で
  そこを見るため、設定画面の配列一覧は「正常に見える」。ユーザーが異常に気づく手掛かりがない。
- `AppConfig::save`（`src/config.rs:890-893`）は `toml::to_string_pretty(self)` で
  **全フィールドを serialize** する（`#[serde(default)]` は deserialize 側の指定であり
  serialize は抑制しない）。したがってこの状態でユーザーが保存すると
  `layouts_dir = "config"` が config.toml に焼き込まれ、**`Permanent` で永久に残るはずの
  MSI 管理下 `layout\` の6本が恒久的に孤児になる**。

  （緩和材料: `ConfigLoadState::Dangerous` の場合は `show_dangerous_save_confirm` モーダル
  を挟むため、焼き込みにはユーザーの明示確認が1回必要。ただしモーダルの文面は
  「config.toml が読めなかったが上書きするか」であって、`layouts_dir` が書き換わることは
  一切伝えない。）
- 生成された `%LOCALAPPDATA%\awase\config\` は MSI の Component にも `RemoveFolder` にも
  含まれないので、**アンインストールしても永久に残る**。本ADRが「MSI管理外のファイルは
  アンインストールで消えない」（ADR 117-122 行の `backup\` 実験）と自ら検証した性質が、
  今度は意図しないゴミとして働く。

**最も重い点**: 決定3（ADR 236-242 行）は `GeneralConfig::default()` の `layouts_dir` が
`"config"` であることを**認識していて、だからこそ serialize を既定値の出典にしなかった**。
にもかかわらず決定2の実装は、その同じ壊れた既定値を**書き込み先**として消費している。
「壊れていると知っている値を読み取りに使わない」対策は入れたが、書き込みに使う経路が
新設されたことに気づいていない。

**発火条件**: config.toml が存在するが TOML として壊れている（手編集ミス、
中断された書き込み、エンコーディング事故）。`classify_load_error` /
`ConfigLoadState::Dangerous` / 危険保存確認モーダルという専用機構が存在することが、
この状態が現実に起きるとリポジトリ自身が認めている証拠である。
なお awase.exe 側は `load_config()` が Err を返すと起動自体が失敗して
`init_engine_validated` に到達しないため、この経路を踏むのは awase-settings のみ。

**要求（マージ前、いずれか1つ）**:
- (i) `src/config.rs:477` の `layouts_dir: "config".to_string()` を `"layout"` に直す
  （出荷 `config.toml` と一致させる。他の参照への影響確認が必要 —
  `src/config.rs:1461` のテストが `"config"` を期待している）。**根治だが影響範囲が広い**。
- (ii) `main.rs:555-557` のゲートを `config_load_state == Loaded` の場合のみに絞る
  （読めなかった config の値を書き込み先として使わない）。**最小差分、推奨**。
- (iii) `ensure_default_layouts_exist` に渡す前に `validate()` を通す
  （`src/config.rs:1039-1044` の `..` 正規化は通るが `"config"` は直らないので不十分）。

---

## Major

### M1. `find_config_path()`（awase-windows）に副作用を埋め込んだのは、awase-settings で意図的に避けた設計の**ちょうど逆**。不具合報告の観測経路がユーザーの実状態を書き換える

`f5cba1ea` のコミット本文は「awase-settings 側は `update_check.rs` 等の他の呼び出し元を
誤って発火させないよう、`ensure_default_config_exists` を `find_config_path` から独立させ
`SettingsApp::new` 側だけで呼ぶ設計にした」と明記している。その判断は正しく、実際に
awase-settings では `--bug-report` / `--check-update` / `--scancode-map` が
`main()`（`main.rs:344-366`）で早期 return するため `SettingsApp::new` に到達せず、
自己修復は発火しない。決定5の4番目「非GUIサブコマンド経路では呼ばれないことを確認する」は
**settings 側では成立している**。

ところが awase-windows 側は逆に、`find_config_path()` の中に直接副作用を置いた
（`app/mod.rs:164`）。その結果、次の2つの呼び出し元が自己修復を発火させる:

- `crates/awase-windows/src/tray.rs:1042` `save_auto_start_config()`
- `crates/awase-windows/src/app/mod.rs:246` `read_bug_report_attachments()`

後者が問題である。これは**不具合報告に添付する config.toml を読むための観測経路**であり、
ここで自己修復が走ると次が起きる:

> ユーザーが「設定が消えた」という不具合に遭遇 → トレイの「不具合を報告」を実行 →
> `read_bug_report_attachments` が `find_config_path()` を呼ぶ →
> `ensure_default_config_exists()` が**工場出荷値の config.toml を新規生成** →
> その工場出荷値が「ユーザーの config.toml」として報告に添付される。

報告を受け取った側には「config.toml は正常（工場出荷値）」としか見えず、
**実際には存在しなかったという最重要の事実が消える**。しかも副作用によって現場が
書き換えられているので、後から確認もできない。

実運用では awase.exe 起動時に既に生成済みなので発火確率は低いが、これは
「観測経路に副作用を置いてはいけない」という設計原則の違反そのものであり、
settings 側では同じ原則を守っている以上、非対称を残す理由がない。

**要求**: awase-settings と同形にする。`find_config_path()` から
`ensure_default_config_exists()` を外し、`load_config()`（`app/mod.rs:140-150`）または
bootstrap の起動経路1箇所だけで呼ぶ。`cli_arg_config_path()` 相当の関数を切り出せば
settings 側とロジックも揃う。

### M2. `bootstrap.rs:237` の `.yab` 自己修復が CLI 引数ゲートを通っていない（settings 側は通っている）

決定2（ADR 222-224 行）は「CLI 引数で config パスが明示されている場合は
`ensure_config_exists` は呼ばない——ユーザーが明示的に指定したパスにアプリが勝手に
ファイルを生成するのは意図しない副作用になる」と定めている。

- `awase-settings`: `main.rs:543` と `main.rs:555` の**両方**が
  `if cli_arg_config_path().is_none()` でゲートされている。原則どおり。
- `awase-windows`: config.toml 側は `find_config_path()` のループが CLI 引数を見つけたら
  `return` するので実質ゲートされているが、`.yab` 側（`bootstrap.rs:237`）は**無条件**。

したがって `awase.exe C:\somewhere\my.toml` で起動すると:

- config.toml の自己修復はスキップされる（原則どおり）。
- `.yab` は `exe_dir.join(my.toml の layouts_dir)` に6本生成される（原則違反）。
- `my.toml` の `layouts_dir` が絶対パス（例 `D:\mylayouts`）なら
  `exe_dir.join("D:\\mylayouts")` = `D:\mylayouts` になるため（`Path::join` の絶対パス
  置換セマンティクス、`app/mod.rs:211-212` の doc も「絶対パスならそのまま使われる」と
  認めている）、**ユーザーの任意のディレクトリに6本書き込む**。

原則が半分しか適用されていない。`bootstrap.rs` 側にも同じゲートを入れるか、
`find_config_path` が CLI 由来かどうかを `AppConfig` と一緒に持ち回ること。

### M3. ADR 本文の「**有効な** `.yab`」という表現が実装と食い違い、「解決されること」の記述が成立していない

ADR 本文は3箇所で「有効な `.yab`」と書いている:

- 185-188 行「`layouts_dir` に**1本も**有効な `.yab` が存在しない場合にのみ」
- 326-328 行「**解決されること**: `layouts_dir` に有効な `.yab` が1本も無い状態で
  アプリが起動不能になる事態を、埋め込み既定値からの復旧で防ぐ」
- 348-351 行「同梱6ファイル名のいずれかが1本もそのディレクトリに存在しない場合のみ発動」
  （これは条件がさらに別物 — 実装は「同梱6ファイル名」ではなく「拡張子 `.yab` の何か」を見る）

実装（`src/config.rs:1392-1403`）は**拡張子が `yab` かどうかしか見ない**。
`config.rs:1382-1385` の doc コメントは「中身の妥当性（パース可能かどうか）は判定しない」
と正しく書いているので、食い違っているのは ADR 本文のほうである。

実害シナリオ: `layouts_dir` に 0 バイトの `broken.yab` が1本だけある（AV による隔離、
中断された書き込み、ユーザーの実験の残骸）。

1. `ensure_layouts_exist` → `has_any_yab = true` → **何もしない**。
2. `LayoutEntry::scan_all` → 有効なレイアウト0件。
3. `NonEmptyLayouts::new` が `None` → `show_no_layouts_dialog`（`bootstrap.rs:246-252`）→
   **起動失敗**。

つまり 326-328 行の「防ぐ」は成立しない。これは v13 決定5 が
`KeyboardModel` 全バリアント試行で扱おうとしていた問題そのもので、v14 が意図的に
捨てた複雑さである（捨てる判断自体は妥当）。**ADR 本文から「有効な」を落とし、
348-351 行の条件記述も実装（拡張子判定）に合わせること。** 「解決されないこと」節に
「壊れた `.yab` が1本でもあると自己修復は発動しない」を1行足すのが正確。

### M4. `EMBEDDED_LAYOUTS`（6）・`layout/*.yab`（6）・`main.wxs` の Permanent コンポーネント（6）が同期している保証がない

現状は3者とも6本で一致している（`ls layout/` = 6ファイル、`src/config.rs:1344-1360` = 6エントリ、
`wix/main.wxs` の `NicolaYab`〜`NicolaKakuteiYab` = 6コンポーネント）。しかし7本目の
`.yab` を追加するとき、3箇所すべてを更新する必要があることを機械的に強制するものがない。

忘れた場合の帰結が**非対称に重い**:

- `main.wxs` に Permanent 付きコンポーネントを足し忘れる → その `.yab` はアンインストールで
  消える。後から `Permanent="yes"` を足しても**既存環境には永久に効かない**（不可逆）。
- `EMBEDDED_LAYOUTS` に足し忘れる → 自己修復が5本/7本しか作らない不完全な集合を生む。
  しかも「1本でもあれば何もしない」判定のため、**次回以降も永久に補完されない**（M5 参照）。

`wix_installer_guard.rs` の既存テスト（`nicola_us_f_kb232_yab_are_bundled_in_msi` 等）は
既知の名前をハードコードしているだけで、`layout/` の実体との突き合わせをしていない。

**要求**: `CARGO_MANIFEST_DIR` から `layout/` を `read_dir` して
「実ファイル名集合 == `EMBEDDED_LAYOUTS` の名前集合 == `main.wxs` の
`<File Source="dist\layout\*.yab" />` 集合」を突き合わせるテストを1本。Linux 実行可。

### M5. `ensure_layouts_exist` の部分失敗が「1本でもあれば何もしない」判定と噛み合って**恒久的に**中途半端な状態を固定する

`src/config.rs:1406-1408`:

```rust
for (name, content) in EMBEDDED_LAYOUTS {
    crate::fs_atomic::write_atomic(&layouts_dir.join(name), content.as_bytes())?;   // ← 途中で return Err
}
```

3本目で失敗（ディスクフル、AV がディレクトリを掴んでいる、`write_atomic` の rename
リトライ4回が尽きる）すると、`layouts_dir` には2本だけが存在する状態で抜ける。
呼び出し元は warn ログを出すだけ（`app/mod.rs:229-231`）。

次回起動時: `has_any_yab = true`（2本ある）→ **何もしない**。
**残り4本は永久に生成されない。** `config.toml:5` の
`default_layout = "nicola_keytop.yab"` が生成されていなければ
`warn_layout_fallback`（`bootstrap.rs:258-264`）が毎回モーダルを出し、設定画面が毎回
勝手に起動する。起動不能ではないが、自力では絶対に回復しない状態である。

自己修復ロジックが「自分自身の部分失敗を回復できない」のは設計上の穴である。
少なくとも ADR の「解決されないこと」に明記すること。実装で直すなら:
6本を書き終えてから判定するか、失敗時にそこまで書いたファイルを片付けて
「0本」に戻す（次回の再試行を可能にする）。前者のほうが単純。

### M6. `is_dev_build()` は2クレートではなく**3箇所目**の "target" ヒューリスティックであり、「別課題」で片付けられない

ADR / コード doc は「2クレートに分かれている既知の重複、共通化は別課題」
（`awase-settings/src/main.rs:5319-5320`）としているが、実際には同じ判定が3箇所にある:

1. `crates/awase-windows/src/app/mod.rs:179-184` `is_dev_build()`
2. `crates/awase-settings/src/main.rs:5321-5326` `is_dev_build()`
3. `src/paths.rs::resolve_relative_to` — `exe.ancestors().find(|a| a.file_name() == "target")`
   でワークスペースルートを求める（**読み取り先の解決**）

3 が「読み取り先」、1/2 が「書き込みを抑止するか」を同じヒューリスティックで決めている。
現状 1/2 が true なら ensure が no-op になるので偶然無害だが、**この無害さは3つの独立
実装が常に同じ答えを返すことに依存している**。片方だけ条件を変えた瞬間、
「resolve はワークスペースルートを見るのに ensure は exe_dir に書く」という
**新しい読み取り先/書き込み先の非対称**が生まれる。これは ADR 214-220 行が
「どんな設計でも必要」と結論づけた不変条件の、まさに次の破れ方である。

重複しているのは 2 クレートではなく「判定の意味を共有する 3 箇所」なので、
`awase::paths` に `is_dev_build()`（あるいは `exe_dir_for_generation() -> Option<PathBuf>`）
を1本置いて3者から呼ぶのが妥当。コストは小さい。

なお副次的に、`is_dev_build()` は「祖先のどこかに `target` という名前のディレクトリがある」
だけを見るので、インストール先の途中に `target` を含むパス（`D:\target\awase` 等）では
自己修復が**黙って無効化**される。確率は低いが、無効化されても何のログも出ない
（`if is_dev_build() { return; }` に warn がない）ので、実機で踏んだら原因究明が難しい。
`tracing::debug!` を1行入れるだけでも違う。

### M7. 決定4 の「ZIP版 `scripts/uninstall.ps1 -Purge` は変更不要（元々MSI管理外の操作のため）」は事実として誤り

ADR 257-258 行の理由付けが成立していない:

- `scripts/uninstall.ps1:10` `$installDir = "$env:LOCALAPPDATA\awase"` は
  MSI の `INSTALLDIR`（`wix/main.wxs:60-61` の `LocalAppDataFolder` → `awase`）と**同一**である。
- `-Purge`（`uninstall.ps1:25-30`）はそのディレクトリを `Remove-Item -Recurse -Force` する。
- MSI でインストールした環境でこれを実行すると、**ファイルは消えるが
  `HKCU\Software\awase` の7つの KeyPath レジストリ値は残る** = round1 B1 のシナリオ3
  （「フォルダだけ消す」）をそのまま作る。

自己修復があるので致命化はしない（B1 の要求(b)が効く）。しかし「MSI 管理外だから
無関係」という理由付けは誤りなので、ADR の記述を訂正すべき。
また `uninstall.ps1` は既に `HKCU:\Software\Microsoft\Windows\CurrentVersion\Run` を
削っている（`:17-18`）ので、`-Purge` に `Remove-Item HKCU:\Software\awase -Recurse` を
足すのは自然であり、決定4 が「任意」としている `purge.ps1` 新規同梱よりずっと低コストで
同じ効果が得られる（→ Q3 の回答参照）。

### M8. ADR がコンテキストとして挙げているサポート定型句「一度アンインストールして入れ直してください」が、この変更で config に対して**無効になる**ことが本文にも docs にも書かれていない

ADR 79-82 行はまさにこの定型句を動機として引いている。v14 適用後:

- アンインストール→再インストールしても、`config.toml` と `layout/*.yab` は
  `Permanent` で残り、`NeverOverwrite` で上書きもされない。
  **「初期状態で試してください」という切り分けが一切できなくなる。**
- 正しい新しい手順は「`%LOCALAPPDATA%\awase\config.toml` と `layout\` を削除して
  awase を再起動する」であり、決定2の自己修復がまさにこれを可能にした
  （実機検証4番がこの手順そのものを検証している）。

決定4 は「**完全削除**したい場合の案内」しか扱っておらず、頻度で言えば圧倒的に多い
「**初期化**したい場合の案内」が抜けている。ADR にこの手順を1節足すこと
（docs 側への反映は M-優先度としては決定4の完全削除案内より上）。

---

## Minor

### m1. `--flag value` パーサが boolean フラグの直後の位置引数を食う。誤判定の帰結が「読む場所が違う」から「勝手に書く」に格上げされている

`app/mod.rs:156-160` と `awase-settings/src/main.rs:5307-5315` のループは
`--` で始まる引数を見ると**無条件に次の1個を値として捨てる**。`awase.exe --debug` は
boolean フラグ（`bootstrap.rs:899`）なので、`awase.exe --debug C:\my\config.toml` では
`C:\my\config.toml` が捨てられ、「CLI 指定なし」と判定される。

このミスパース自体は v14 以前からの既存挙動（`f5cba1ea^` の settings 側 `find_config_path`
も同じ）。ただし従来の帰結は「exe 隣の既存 config.toml を読む」だけだったのに対し、
v14 では `ensure_default_config_exists()` が走って**ファイルを新規生成する**ようになった。
決定2 の CLI ゲートはこのパーサの正しさに乗っているので、ADR に既知の限界として
1行書いておくこと（修正までは求めない）。

### m2. `ensure_layouts_exist` のテストに「ディレクトリは存在するが空」ケースがない

`src/config.rs:2530` は「ディレクトリが丸ごと無い」、`:2546` は「`.yab` が1本ある」を
カバーしているが、**「ディレクトリはあるが空」がない**。v14 適用後はむしろこれが
標準ケースになる: `Permanent` 化により `RemoveFolder Id="RemoveLayoutDir"` が
実行されなくなった（`wix/main.wxs:142-145` が自ら認めている）ため、
アンインストール後に `layout\` ディレクトリだけが残る形が増える。追加コストは数行。

### m3. `ensure_layouts_exist` が `read_dir` の失敗を「`.yab` が0本」と同一視している

`src/config.rs:1393` の `std::fs::read_dir(layouts_dir).is_ok_and(...)` は、
「存在しない」と「存在するが権限で開けない」を区別しない。後者では
`create_dir_all` が成功（既にある）し `write_atomic` が失敗して `Err` を返すので
ログには残るが、doc コメントにこの意図（区別しない）を1行書いておくべき。

### m4. frontmatter / index.md のステータスが「起草中・レビュー未実施」のまま

`docs/adr/178-msi-uninstall-preserve-userdata.md:5-11` および
`docs/adr/index.md:185` が「起草中v14・opus-adversarial-consult レビュー待ち」。
マージ時に `.claude/rules/docs-frontmatter-convention.md` に従って確定ステータスへ更新し、
index.md 側は短縮表示のまま保つこと（長文を書き戻さない）。
「未解決事項」節（353-368 行）も、1〜4 が既に完了済みなので残タスク（5・6）だけが
読み取れる形に整理したほうがよい。

### m5. round1 m5（`related_adr` に複雑性予算系への言及）が未反映

frontmatter の `related_adr` は `ADR-099` / `ADR-177` のみ。
`.claude/rules/complexity-budget.md` は未発効なので必須ではないが、v1〜v13 で
20 Blocker を積み上げた末に「複雑さを捨てる」判断をした ADR として、
ADR-158（複雑さ削減の北極星）への参照は記録価値が高い。

### m6. `src/config.rs:2486-2495` の `unique_temp_dir` が `src/paths.rs` の同名ヘルパーと重複

pid のみでユニーク化しているが、各テストが異なる `name` を渡すので現状は衝突しない。
指摘としては軽微。

---

## 依頼された質問への回答

### Q1. round1 の B1・B2・M1〜M7 は構造的に解消されているか

| 項目 | 判定 |
| --- | --- |
| **B1**（KeyPath 残留 × NeverOverwrite で再配置されず起動不能） | **実質解消。ただし解毒剤が無防備（→ Blocker B1）。** 実機検証4番が「config.toml + layout を手動削除 → 起動 → 両方生成」を確認しており、round1 が示した最悪シナリオ（起動不能）は消えた。だが解消しているのは MSI 側ではなくアプリ側の配線であり、その配線を守るテストがゼロ。`Permanent` が不可逆である以上、非対称が致命的。 |
| **B2**（既存ユーザーへの遡及効果が未検証） | **完全解消。** 実機検証1（1.20.6 → 1.20.7 アップグレード → アンインストール → 編集内容 918 が保持）で「効く側の根拠」が正しいと確定。round1 が懸念した catch-22 は存在しなかった。**v14 で最も価値の高い成果。** |
| **M1**（テスト方針が false green） | **部分解消。** B2 は実機で潰れ、B1 は自己修復で潰れた。しかし「false green を出す設計」という指摘の本質は残っている: 現在の自動テストは `main.wxs` の文字列一致だけで、B1 の解毒剤も M4 の3者同期も検出できない。→ Blocker B1 / Major M4。 |
| **M2**（perUser のレジストリパス） | 実機検証を PowerShell の `Test-Path` ベースで実施しており、round1 が懸念した false negative は発生していない。**実質解消**（ただし決定4 の案内が `HKCU\Software\awase` を指すのは KeyPath としては正しい。`HKCU\Software\Microsoft\Installer\Components\<packed GUID>` の permanent client 登録は実務上削除できず、それが不可逆性の実体であることは「解決されないこと」で言及済み）。 |
| **M3**（完全削除手順が不完全） | **設計としては解消、実装は未着手。** 決定4 が「フォルダ＋レジストリキー両方」と正しく定めているが、`docs/index.html` / `index.en.html` への反映は未実施。ただし現状これらのファイルにアンインストール手順の記述が**一切ない**（grep で0件）ため、誤った案内が残るわけではない。→ Q3 参照。 |
| **M4**（選択肢B の棄却理由） | v14 は選択肢B を採らないので争点が消滅。**論点消滅**。 |
| **M5**（選択肢D） | **採用（ハイブリッド形）。** v14 の中核。round1 が最初に出していた方向に12ラウンド遅れて戻った経緯も本文に記録されており、記録として良い。 |
| **M6**（将来のインストール先変更が詰む） | ADR 341-343 行で明記済み。**解消**。 |
| **M7**（`msiexec /f` は復旧経路にならない） | ADR 344-347 行で明記済み。「ただし決定2の自己修復は config.toml 自体が消えた場合には機能する」という補足も正確。**解消**。 |
| **m1〜m5** | m2（`RemoveLayoutDir` の挙動）は `wix/main.wxs:142-145` で明記済み。m3（docs への追記先）は未実施。m4（assert メッセージ）は `wix_installer_guard.rs:108-113` で対応済み、文面も要求水準を満たす。m5（related_adr）未反映（→ m5）。 |

**「形を変えて残っているもの」の総括**: v2〜v13 を支配した「読み取り先/書き込み先」問題は、
ADR 214-220 行が正直に認めているとおり **v14 でも1回再発した**（`3d7a7ece`）。
バックアップ機構を捨てても消えなかったということは、この問題の原因はバックアップ機構
ではなく「`resolve_relative_to_exe` が存在依存フォールバックを持つ」という
`src/paths.rs` の設計そのものにある。v14 はそれを doc コメント2箇所で防いでいるが、
**同じ関数を書き込み先に使えてしまう状態は変わっていない**。Blocker B1 の要求3
（`ensure_*` 本体に解決関数が出現しないことをテストで固定）は、この根を機械的に
塞ぐ最小の手段である。

### Q2. 実機検証で見つからなかった実装上のバグ・見落とし

- **Blocker B2**: `default_config()` の `layouts_dir = "config"` による
  `%LOCALAPPDATA%\awase\config\` への `.yab` 6本生成（実機検証は config.toml が
  正常な状態でしか行われていないので検出されなかった）。
- **Major M2**: `.yab` 自己修復が CLI 引数ゲートを通っていない
  （実機検証は常に引数なし起動だったので検出されなかった）。
- **Major M5**: 部分失敗が「1本でもあれば何もしない」で恒久固定される
  （実機ではディスクフル等が起きなかったので検出されなかった）。
- **Major M1**: `read_bug_report_attachments` 経由の自己修復発火
  （観測経路の汚染。実機では config が常に存在したので検出されなかった）。

依頼で名指しされた4点への回答:

1. **`is_dev_build()` の2クレート重複**: 「2箇所の重複」ではなく「意味を共有する3箇所」
   （`src/paths.rs::resolve_relative_to` を含む）。→ M6。別課題に送るのは妥当だが、
   ADR の書き方は訂正すべき。
2. **awase-settings の `ensure_default_config_exists` を `find_config_path` から独立させた設計**:
   **正しく機能している。** `update_check.rs` が呼ぶ `find_config_path()`（`main.rs:5297-5299`）は
   `cli_arg_config_path()` + `resolve_relative_to_exe` だけで副作用がない。
   `--bug-report` / `--check-update` / `--scancode-map` はいずれも `main()`
   （`main.rs:344-366`）で早期 return するので `SettingsApp::new` に到達しない。
   テストコードからの呼び出しも安全。**問題は逆側**で、この正しい設計が awase-windows に
   適用されていない（→ M1）。
3. **ICE91 以外の新しいビルド時警告**: このサンドボックスでは `candle.exe`/`light.exe` を
   実行できないため断定できないが、静的解析としては新規警告は出ないはずである。
   根拠: ICE64（「ユーザープロファイル配下のディレクトリが RemoveFile テーブルに
   ない」）は**テーブルに行があるかどうか**の静的チェックであり、`RemoveFolder
   Id="RemoveLayoutDir"`（`wix/main.wxs:150`）と `RemoveInstallDir`（`:104`）の行は
   そのまま残っている。`Permanent` が変えるのは**実行時のアクション状態**だけなので
   ICE64 は引き続き通る。なお副作用として **ICE64 のチェックが実質的に無意味になる**
   （行はあるが決して実行されない）点は記録しておく価値がある。
   `Permanent` + `NeverOverwrite` の同時指定を咎める標準 ICE は存在しない。
   `.github/workflows/ci.yml:394-400` の MSI ビルドは `-wx` を付けていないので、
   仮に新規警告が出ても CI は落ちない = **CI は検出器として機能しない**ことに注意。
   実機で 1.20.7 / 1.20.8 のビルドが通っている事実が最も強い証拠である。
4. **「解決されないこと」節の正確性**: 3項目とも正確。
   - 不可逆性（338-340 行）: 正確。round1 が引用した MS Q&A 1602667 の通り。
   - 将来のインストール先変更（341-343 行）: 正確。
   - `msiexec /f` が復旧経路にならない（344-347 行）: 正確。「Permanent 以前からの
     既存挙動」という但し書きも正しい（`NeverOverwrite` 由来なので `Permanent` は無関係）。
   - **不足**: M5（部分失敗の恒久固定）と M3（壊れた `.yab` が1本あると自己修復しない）が
     この節に無い。M8（アンインストール→再インストールによる初期化ができなくなる）も
     「解決されないこと」というより「新たに失われること」として明記すべき。

### Q3. 決定4・決定5 の未実施項目は受け入れ基準として必須か

**`docs/index.html` / `docs/index.en.html` へのアンインストール手順追記: 必須ではない（後回し可）。**
理由:
- 現状これらのファイルにアンインストール手順の記述が**一切存在しない**
  （`README.md` / `README.en.md` / `docs/index*.html` を grep して0件）。つまり
  「既存の誤った案内が残る」問題は発生しない。
- round1 B1 の致命的帰結（起動不能）は決定2 の自己修復で消えており、実機検証4で
  確認済み。案内が無いことによる最悪ケースは「レジストリにゴミが残る」であって
  「アプリが起動しない」ではない。

**`purge.ps1` 同梱: 不要。** 代わりに M7（`scripts/uninstall.ps1 -Purge` に
`HKCU\Software\awase` の削除を追加）を採ること。既に Run キーを削っているスクリプトに
1行足すだけで、新規ファイル・新規 MSI コンポーネント（＝新しい GUID と新しい永続的
制約）を増やさずに同じ効果が得られる。

**ただし優先順位の入れ替えを推奨**: 未実施項目のうち最も実害頻度が高いのは
決定4 の「完全削除」案内ではなく、**M8 の「初期化」手順**（サポート定型句の差し替え）
である。docs 更新を1回行うなら、完全削除より先にこちらを入れるべき。

### Q4. 総合判定

**実装をやり直す必要はない。設計の骨格は正しい。ただし Blocker 2件の解消を
マージの条件とすべき。**

支持する根拠:
- `Permanent="yes"` + 「無ければ作るだけ」の自己修復という組み合わせは、
  round1 の要求(b) そのものであり、v2〜v13 が積み上げた
  `EnsureOutcome`/`UserDataGuard`/`FileRestoreState` を一切必要としない。
  「バックアップと実ファイルの整合」という問題クラスが構造的に消えているという
  ADR 64-69 行の主張は正しい。
- 実機検証が round1 の2つの Blocker のうち**片方（B2）を完全に潰し、もう片方（B1）の
  最悪シナリオを実際に踏んで解毒剤が効くことまで確認している**。さらにその過程で
  実バグを1件発見・修正している。v1〜v13 の13回の机上検討より、この4項目の実機検証
  1回のほうが情報量が多い。
- 埋め込み既定値と MSI 同梱物が `release.yml` のコピー元として同一であり、
  v13 が必要としていたバイト一致 CI 検証が構造的に不要になっている。
- 差分は7ファイル・約324行と小さく、clippy 新規警告 0 件、新規テスト11件すべて green。

マージ前に必須:
1. **Blocker B1** — 自己修復配線のソーススキャンガード追加（`architecture_guard.rs` に
   4アサート程度）。`Permanent` が不可逆である以上、これは「あとで」にしてよい種類の
   宿題ではない。
2. **Blocker B2** — `awase-settings/src/main.rs:555-557` を
   `config_load_state == Loaded` でゲート（1行）。

マージ前に推奨（いずれも小差分）:
3. **M1** — `find_config_path()` から副作用を外す。
4. **M2** — `bootstrap.rs:237` に CLI 引数ゲートを追加。
5. **M3** — ADR 本文の「有効な `.yab`」3箇所を実装に合わせて訂正。
6. **M5 / M8** — 「解決されないこと」節に2行追記。

フォローアップで可: M4（3者同期テスト）、M6（`is_dev_build` の一本化）、
M7（`uninstall.ps1 -Purge` のレジストリ削除）、Minor 全件、決定4 の docs 更新。

**マージ手続き上の注意**: 本ブランチは `origin/develop` に対し behind 5 で、
develop 側の ADR-176 T8/T9b（`c9dbfedd` / `d179ad6c`）が
`crates/awase-settings/src/main.rs` と `crates/awase-windows/src/app/` を触っている。
ただし develop 側の settings 変更は `main()` 冒頭にメッセージ専用ウィンドウ生成を
足す8行のみで、**新しい CLI 引数も `SettingsApp::new` の変更も含まない**ため、
`cli_arg_config_path()` / 自己修復ゲートとの意味的な衝突はない。
コンフリクトは機械的なもので済む見込み。マージ後に
`cargo clippy --target x86_64-pc-windows-msvc`（3クレート）と
`cargo test -p awase-windows --test wix_installer_guard` を再実行すれば足りる。
