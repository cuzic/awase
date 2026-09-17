---
id: ADR-178-companion-178-opus-review-round2
title: |-
  ADR-178（MSIアンインストール時のユーザーデータ保護）Opus敵対的レビュー round2
type: companion-doc
related_adr:
  - "ADR-178"
---

# ADR-178 敵対的レビュー round2（v2: 自己修復方式）

対象: `docs/adr/178-msi-uninstall-preserve-userdata.md`（v2 全面書き直し版、コード未実装）
前回: `opus-review-adr178-round1.md`（v1 `Permanent="yes"` 案をBlocker 2件で却下）

方針転換（`wix/main.wxs`を触らない・アプリ側で自己修復）の**方向性自体は支持する**。
round1のB1/B2は完全に解消しており、MSI側の不可逆リスクも持ち込まない。

ただし現在の決定文のままでは、**本ADRが掲げる主要シナリオ（MSIアンインストール
→再インストール）で復元が一度も発火しない**（B1）。加えて、復元の書き込み先が
未定義で`C:\Windows\system32`等に書く実装になりやすい（B2）、バックアップ対象の
絞り込みが無いためユーザーの配列ファイルを静かに壊しうる（B3）。いずれも決定文の
修正で解決可能。

---

## Blocker

### B1. 復元トリガ「ファイルが存在しなければ」は、主要シナリオでは発火しない（MSI再インストールが既定値でファイルを作り直すため）

ADR 133-136行は「解決されること」の筆頭に
> MSIアンインストール→再インストール後、`config.toml`/`layout/*.yab`がMSI側の
> 挙動によって物理的に削除されても、次回起動時にバックアップから実質的に復元される。

と書いているが、この経路は成立しない。

**理由**: 7コンポーネントは非Permanentのままなので、`msiexec /x`は
`config.toml`/`*.yab`と**同時にKeyPathレジストリ値**（`HKCU\Software\awase\ConfigFile`
等、`wix/main.wxs:116-117` 他）も削除する。したがって次のインストールは
「KeyPathが無い＝真の新規インストール」となり、`NeverOverwrite`は効かず
（`wix/main.wxs:144-148`、`crates/awase-windows/tests/wix_installer_guard.rs:150-156`が
明記している通り「KeyPathが無い真の新規インストールには影響しない」）、
**MSIが`dist\config.toml`と6本の`.yab`を既定値の内容で配置する**。

つまり awase.exe の起動時点で:

```
%LOCALAPPDATA%\awase\config.toml       ← 存在する（ただし中身は出荷時の既定値）
%LOCALAPPDATA%\awase\layout\*.yab      ← 存在する（同上）
%LOCALAPPDATA%\awase-backup\config.toml ← ユーザーの設定が入っている
```

決定2の条件は「`config.toml`が存在しない場合」なので**復元は発火しない**。
ユーザーは既定値のまま使い続け、バックアップは永久に参照されない。
ユーザー体感の結果は現状（v1もv2も無い状態）と完全に同じ＝**本ADRの目的が達成されない**。

決定2が実際に効くのは「MSIを再インストールしない」経路（ユーザーが手で消した、
ZIP版で`awase.exe`だけ差し替えた等）に限られる。ところがADRの動機は
「不具合対応でよくある案内『一度アンインストールして入れ直してください』」
（v1コンテキスト・ADR-177「副次的な発見」節）であり、**まさに再インストールする
経路**である。

**要求**: 復元トリガを「存在しない」から「存在しない **または** 中身が出荷時の
既定値とバイト一致する」に拡張すること。決定3で既定値を`include_str!`で
埋め込むので、比較対象はコード内にあり追加コストはほぼゼロ:

```
if !path.exists()
   || (fs::read(path) == EMBEDDED_DEFAULT && backup_exists && backup != EMBEDDED_DEFAULT)
{ restore_from_backup_or_default() }
```

「既定値と一致する」＝ユーザーが一度も編集していない（またはMSIが今書き戻した）
状態なので、バックアップを優先して実害が出るケースはほぼ無い。唯一の例外は
「ユーザーが意図的に既定値へ戻した直後」で、これはM4（復元したことをユーザーに
知らせる）とセットで扱えば許容できる。この拡張を入れないなら、ADRの
「解決されること」から再インストール経路を削除し、「解決されないこと」へ
移すべき（＝そもそも要望に応えていないことを明示すべき）。

### B2. 復元先パスが未定義。`resolve_relative_to_exe()`は「見つからないとき」CWD相対の裸パスを返すため、素直に実装すると`C:\Windows\system32`に書き込む

決定2は復元ロジックを`find_config_path()`に置くとしている
（`crates/awase-windows/src/app/mod.rs:153-171`、
`crates/awase-settings/src/main.rs:5290-5300`）。両者とも解決の実体は
`awase::paths::resolve_relative_to_exe("config.toml")` であり、この関数は

- exe隣に**存在すれば**そのパス、
- ワークスペースルート相対に**存在すれば**そのパス、
- **どこにも無ければ** `PathBuf::from("config.toml")`（＝CWD相対の裸パス）

を返す（`src/paths.rs:33-65`）。3番目のフォールバックには
「意図しない場所に新規ファイルを作る／別の実行ファイルと異なるファイルを
読み書きする典型的な事故（2026-07-19実機確認、ADR-099 F5）の入口」という
警告コメントが明示的に書かれている。

**復元ロジックが動くのは、まさにこの3番目のケースだけ**（ファイルが無いから
復元するのだから）。したがって「`resolved`へ書き戻す」と実装すると、書き込み先は
プロセスのCWDになる。awase.exeの自動起動は
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` の
`"[INSTALLDIR]awase.exe"`（`wix/main.wxs:83-87`、作業ディレクトリ指定なし）
であり、この経路で起動したプロセスのCWDは`%windir%\system32`等になる。結果:

- 書き込みが`PermissionDenied`で失敗し、復元が常に失敗する（無害だが無意味）、
  または
- 書き込みに成功して`C:\Windows\system32\config.toml`が出来上がり、以後
  CWDが同じ条件で起動したときだけそれを読む、という ADR-099 F5 と同型の
  「どのconfigを読んでいるか分からない」事故になる。

スタートメニューのショートカット（`WorkingDirectory="INSTALLDIR"`、
`wix/main.wxs:206-207`）経由では偶然正しい場所に書かれるため、**起動方法に
よって挙動が変わる**のが最悪で、実機検証でも「ショートカットから起動したら
直った」と誤って合格判定しうる。

**要求**: 決定2に「復元先は`current_exe().parent()`から構成した絶対パスとし、
`resolve_relative_to_exe()`のCWDフォールバック結果には決して書き込まない」ことを
明記する。`src/paths.rs`に「存在チェックをせずexe隣の絶対パスを返す」関数
（例: `resolve_next_to_exe()`）を追加し、復元専用に使うのが素直。
`layouts_dir`側（`resolve_relative(&config.general.layouts_dir)`、
`bootstrap.rs:236`）も同じ関数を通るので同じ罠がある。

### B3. 「`layout_write_to_path()`の成功後にコピー」は、`layouts_dir`外の任意ファイルまでバックアップし、復元時にユーザーの配列を静かに壊す

決定1は「`layout/*.yab`: `layout_write_to_path()`
（`crates/awase-settings/src/main.rs:1598`、配列編集タブの保存処理）の
書き込み成功後にコピー」とだけ書いている。しかし`layout_write_to_path`の
呼び出し元は`layouts_dir`内に限定されていない:

- `main.rs:1823` — `layout_pending_save_as`、すなわち**「名前を付けて保存」
  ダイアログでユーザーが選んだ任意のパス**（`rfd::AsyncFileDialog`、
  `layout_open_dialog_unchecked`と対）。
- `main.rs:758` — `self.layout_file_path`。これは「開く」ダイアログで開いた
  任意のファイルでもよく、コードは`path != default_layout_path`のケースを
  **正常系として明示的に扱っている**（749-753行の
  「この配列ファイルは現在の配列フォルダ／既定の配列と異なるため、awase
  エンジンには反映されません」という注記）。

**具体的な失敗シナリオ**:

```
1. ユーザーが layout/nicola.yab を編集して保存（→ backup/nicola.yab に正しく退避）
2. 実験用に「名前を付けて保存」で D:\experiments\nicola.yab へ保存
   （basename が偶然同じ。あるいは意図的に同名で別案を作る＝自然な操作）
   → 決定1の通りなら backup/nicola.yab が実験版で上書きされる
3. 後日 MSI アンインストール→ layout/nicola.yab 消失
4. 起動時の復元で、layout/nicola.yab に「実験版」が書き戻される
   → ユーザーの本番配列が実験版に静かに置き換わる。元に戻す手段はない。
```

バックアップ機構がユーザーデータを壊すのは、本ADRの目的（データを守る）に対する
自己矛盾であり、しかもユーザーには検知できない。

**要求**: 決定1に「バックアップ対象は、書き込み先が現在の`layouts_dir`配下で
あり、かつ同梱6ファイルのいずれかのファイル名である場合に限る」という絞り込みを
明記する（`layout_write_to_path`の内部で判定するか、呼び出し元で判定するか）。
`config.toml`側も同様に「`find_config_path()`がCLI引数由来でない場合に限る」が要る（→M6）。

---

## Major

### M1. バックアップ契機が「保存の都度」だけだと、**手編集ユーザーが一切保護されない**（しかも今より検知しづらくなる）

`config.toml`はGUIからだけでなく手で編集される前提の設計である
（`docs/index.html`に「`config.toml`の主要設定」表があり、
`src/config.rs:1239`のエラーメッセージも「config.tomlで別のキーに変更してください」と
案内、`crates/awase-settings/src/main.rs:7885`のコメントも「config.toml手編集でのみ
到達しうる値」と書いている）。テキストエディタでの編集は`AppConfig::save()`を
通らないので、**バックアップは一度も作られない**。

この層のユーザーに何が起きるか:
- **現状（v2無し）**: MSIアンインストール後に起動すると
  「Config file not found」でエラーダイアログ（`crates/awase-windows/src/main.rs:49-53`）。
  設定が消えたことに**気づく**。
- **v2導入後**: バックアップが無いので埋め込み既定値から生成され、アプリは
  **正常に起動する**。ユーザーは「なんか設定が戻っている」ことに気づかないまま
  既定値で使い続ける（あるいは原因不明の挙動変化として報告される）。

つまりこの層に対しては、v2は保護にならないどころか**失敗の可視性を下げる**。

**要求**: バックアップ契機に「**`AppConfig::load()`/`.yab`読み込みに成功した
起動時**、内容がバックアップと異なればバックアップを更新する」を追加する。
これで手編集も次回起動時に必ず捕捉でき、契機が「保存時」だけという穴が塞がる
（実装コストも小さい: 起動経路は`bootstrap.rs`に集約されている）。
併せて、既定値から生成した／バックアップから復元した場合は`tracing::info!`だけでなく
ユーザーに見える形（トレイ通知か設定画面のステータス）で知らせること（→m4）。

### M2. バックアップ処理を`AppConfig::save()`（`src/config.rs`）に置くと、ADR-019（コアのOS非依存）違反かつテストが非ヘルメティックになる

決定1は「バックアップ処理は`AppConfig::save()`自身、または両呼び出し元が共通して
通る箇所に実装し、重複実装を避ける」と書いている。前者は採れない:

1. **ADR-019違反**: `%LOCALAPPDATA%`の解決はWindows固有。CLAUDE.mdが明記する
   「コア`awase`クレートはOS非依存でなければならない（`#[cfg(target_os)]`禁止）」に
   正面から反する。`awase-linux`/`awase-macos`スタブもこのクレートを使う。
2. **テストが実ユーザー環境を汚す**: `src/config.rs:2224`のユニットテストが
   `config.save(&path)`を呼んでいる。`save()`に暗黙のバックアップ副作用を入れると、
   `cargo test --lib`が開発者の実`%LOCALAPPDATA%\awase-backup\config.toml`を
   上書きする。CI（Linux）では`%LOCALAPPDATA%`が無いので分岐が増え、
   ローカルWindows開発機では実データが壊れる。

**要求**: バックアップは`save()`の**暗黙の副作用にしない**。
(a) プラットフォーム側（`awase-windows`/`awase-settings`）に共通ヘルパーを置く、
または (b) コアに置くなら`fn backup_to(&self, dir: &Path)`のように**退避先を
引数で受け取る純粋な関数**にし、`%LOCALAPPDATA%`の解決は呼び出し元に残す。
ADR 未解決事項2はこの選択を「設計する」とだけ書いているが、(a)(b)以外は
採ってはいけない制約として決定文に書くべき。

### M3. バックアップ先`%LOCALAPPDATA%\awase-backup`は、ポータブルZIP・複数インストールで破綻する。しかも「MSI管理外にするため兄弟ディレクトリにする」必要が実はない

**必要が無い根拠（一次情報）**: ADR-177「検証の限界」7
（`docs/adr/177-msi-restart-manager-graceful-shutdown.md`、448-452行付近）が
実機で確認している——
> `config.toml`/`layout/`はアンインストール時に正しく削除されており（…）
> **ログファイル等MSI管理外のファイルはディレクトリに残っている**

MSIは File/RemoveFile テーブルに載っているものしか消さず、`RemoveInstallDir`の
`RemoveFolder`は「空のときだけ削除」なので、`%LOCALAPPDATA%\awase\`**の中に**
置いた`backup\`サブディレクトリはアンインストールを生き延びる
（`cache.toml`・`awase.log`・`config.toml.bak`が現に生き延びている）。

**兄弟ディレクトリにした場合の実害**:
- **ポータブルZIP**が壊れる: ZIP版は任意の場所（`D:\tools\awase\`、USBメモリ等）に
  展開できる設計で、リソース解決は全て**exe相対**（`src/paths.rs`）。バックアップ先だけ
  `%LOCALAPPDATA%`固定にすると、ポータブル運用のつもりのユーザーがホストPCの
  ユーザープロファイルに設定を書き残す（意図に反する）。
- **複数インストールの相互汚染**: MSI版とZIP版、あるいは複数バージョンを併用すると
  1つの`awase-backup`を共有し、片方の設定でもう片方が復元される。exe相対なら起きない。
- **後片付けの穴**: `scripts/uninstall.ps1`は`$installDir = "$env:LOCALAPPDATA\awase"`を
  ハードコードしている（`scripts/uninstall.ps1:10`）。兄弟ディレクトリにすると
  `-Purge`に**2つ目のハードコードパス**を足すことになり、しかもポータブル展開先には
  そもそも届かない。

**要求**: バックアップ先を`<exe_dir>\backup\`（＝MSI版なら
`%LOCALAPPDATA%\awase\backup\`）に変更するか、兄弟ディレクトリを維持するなら
上記3点への回答を決定文に書くこと。前者なら決定4（purge案内）も
「`%LOCALAPPDATA%\awase`を削除」の1行で済み、round1 M3で指摘した
「案内が複雑になる」問題も同時に消える。

### M4. ADR-099決定4（`ConfigLoadState`）との統合が「未解決事項」止まりだが、**最もバックアップが要るケース（`Dangerous`）が設計から抜けている**

`classify_load_error`（`src/config.rs:848-858`）は
`io::ErrorKind::NotFound`のみ`NotFound`、それ以外（parse error・`PermissionDenied`・
共有違反）は全て`Dangerous`に倒す。v2の復元ロジックが扱うのは前者だけである。

- 「`config.toml`は存在するがTOMLが壊れている」（`Dangerous`）→ 復元されない。
  ところが**バックアップが最も価値を持つのはこのケース**（直前まで動いていた
  設定が手元にあるのに使わない）。
- 逆に`awase-settings`側には既に`config.toml.bak`という別のバックアップがある
  （`crates/awase-settings/src/main.rs:804-815`）。ただしこれは
  「`Dangerous`のときに**一度だけ**」作られる＝**最初に壊れた時点の内容**であり、
  「最後に正常だった内容」ではない。v2の`awase-backup`とは意味論が違う。
- `NotFound`分岐は復元導入後ほぼ到達不能になる。`awase-settings`側の
  `ConfigLoadState::NotFound`を前提にしたUI分岐（`main.rs:544-560`・1124行付近）が
  死にコード化するので、残すか消すかを決める必要がある。

**要求**: 決定文に以下を書く。
1. `Dangerous`時に復元するか／しないか（推奨: **しない**。ただし
   `config.toml.bak`への退避後にバックアップからの復元を**ユーザーに提案**する。
   自動で上書きすると、ユーザーが手編集中の壊れたファイルを消してしまう）。
2. `config.toml.bak`（1回きり・壊れた版）と`awase-backup\config.toml`
   （毎回更新・正常版）の役割分担、どちらが復旧の第一候補か。
3. `ConfigLoadState::NotFound`の扱い（残すなら到達条件を、消すなら影響範囲を）。

### M5. `awase.exe`と`awase-settings.exe`が同時に復元を走らせる経路が実在する。復元は原子的書き込みにすること

両プロセスは独立に起動し、どちらも`find_config_path()`→復元を通る。しかも
**awase.exe自身が設定画面を起動する経路**がある
（`crates/awase-windows/src/app/bootstrap.rs`の`warn_layout_fallback` →
`super::launch_settings()`、260行付近）。ログオン時の自動起動と手動起動が
重なることもある。

- 復元に`std::fs::write`を素朴に使うと、片方が書いている途中の`config.toml`を
  もう片方が読み、parse error → `Dangerous`判定 → M4の未定義動作へ。
- `AppConfig::save()`は既に`crate::fs_atomic::write_atomic`（`src/fs_atomic.rs`、
  一時ファイル+fsync+rename、ADR-099決定3）を使っている。復元も同じ経路を
  通すべきで、決定文に明記すること。
- `layout_write_to_path`は現状`std::fs::write`直書き（`main.rs:1604`）。
  バックアップ/復元を足すなら、ここも`write_atomic`に寄せるかどうかを決めること
  （本ADRのスコープを広げすぎない判断も可だが、**決めたことを書く**）。

### M6. CLI引数で明示されたconfigパスに対して復元を発火させてはならない

`find_config_path()`はCLI第一引数を**存在チェックせずそのまま返す**
（`crates/awase-windows/src/app/mod.rs:155-162`、`awase-settings`側も同型で
`main.rs:5290-5300`）。ここに復元を足すと:

- `awase.exe D:\tmp\test-config.toml`（存在しない）を実行した瞬間、
  そのパスに**別インストールのバックアップが実体化する**。
- 開発・検証で使う一時configパスに、無関係な実環境の設定が書かれる。
- 「明示的に指定したのに勝手に中身が作られる」のはCLIの期待に反する。

**要求**: 復元は「CLI引数が無く、自動解決（exe隣）に落ちた場合」に限定する、と
決定2に明記する。B2の「復元先はexe隣の絶対パス」と合わせると、
**復元対象は`<exe_dir>\config.toml`と`<exe_dir>\<layouts_dir>\*.yab`のみ**という
単純な不変条件になる。

### M7. 埋め込み既定値の一覧（6本の`.yab`）が3箇所に散る。追加漏れを検出するガードテストが要る

同梱`.yab`は増え続けている（`nicola_kb232.yab` 2026-09-05頃、
`nicola_kakutei.yab` 2026-09-13追加）。追加のたびに更新が必要な場所:

1. `wix/main.wxs`（新しい`<Component>`、`crates/awase-windows/tests/wix_installer_guard.rs:189-208`が
   既にガードしている）
2. 決定3の`include_str!`リスト（**新規**）
3. 決定2の復元対象リスト（**新規**）

（`release.yml`は`cp layout/*.yab dist/layout/`のグロブなので対象外）

2.と3.を忘れると、「MSIには入っているがアンインストール後に復元されない」
あるいは「バックアップもされない」配列が静かに生まれる。これは
`wix_installer_guard.rs`のヘッダコメントが言う「コンパイラは何も教えてくれない」
種類の不変条件そのもの。

**要求**: `layout/`ディレクトリの実ファイル一覧と埋め込み/復元リストの一致を
確認するテスト（既存の`wix_installer_guard.rs`と同型のテキスト/ディレクトリ走査）を
決定文のテスト方針に含めること。

### M8. 復元ロジックを`find_config_path()`の中に置くのは副作用の隠蔽。`find_config_path()`は「パスを引くだけ」の呼び出し元が複数ある

決定2は実装箇所として`find_config_path()`を名指ししているが、この関数は
起動経路以外からも呼ばれる:

- `crates/awase-windows/src/app/mod.rs:185` — `read_bug_report_attachments()`
  （不具合報告の添付作成、ADR-095）。
- `crates/awase-windows/src/tray.rs:1042` — トレイの自動起動トグル。
- `crates/awase-settings/src/update_check.rs:39` — 更新チェック。

「不具合報告ボタンを押したらconfigが復元された」「トレイでauto_startを
切り替えたらファイルが生成された」は、いずれも呼び出し元の期待に反する
（特に不具合報告は**現状を採取する**のが目的なので、採取行為が状態を変えるのは
調査を妨げる）。

**要求**: 復元は起動シーケンス内の**明示的な1ステップ**
（例: `bootstrap.rs`の先頭で`ensure_user_data_present()`を1回呼ぶ）にし、
`find_config_path()`は純粋な解決関数のまま残す。`awase-settings`側も同様。

---

## Minor

### m1. 「Config file not found」のエラーダイアログ文言が不整合になる／参照先が同梱されていない

`crates/awase-windows/src/main.rs:49-53`の
> 「config.toml が見つかりません。awase.exe と同じフォルダに config.toml を
> 置いてください（**同梱の config.sample.toml** をコピーして使えます）。」

は、(a) v2導入後はほぼ到達不能になる、(b) `config.sample.toml`は
**ZIPにもMSIにも同梱されていない**（`.github/workflows/release.yml`の
"Prepare distribution"は`cp config.toml dist/`のみ、`wix/main.wxs`にも
`<File Source="dist\config.sample.toml">`は無い。リポジトリルートには存在する）。
(b)は既存のバグだが、v2でこの分岐を触るなら一緒に直すのが自然。同じ文字列を
参照する`startup_error_hint`の「Failed to parse」分岐も同様。

### m2. `include_str!`のパスと「出荷される既定値」との同一性を決定文に書いておく

決定3は「リポジトリルートの実ファイルを直接参照」とするが、実際に出荷されるのは
`dist/config.toml`＝`release.yml`が`cp config.toml dist/`でコピーしたもの、
`dist/layout/*.yab`＝`cp layout/*.yab dist/layout/`。現時点では同一なので問題ないが、
将来`dist`向けに加工を入れると埋め込み既定値とMSI同梱物が乖離する。
「加工を挟まない」ことを決定文の制約として書くか、B1で提案する
「既定値とのバイト比較」が壊れる旨を注記すること（B1を採る場合、この同一性は
**機能要件**に昇格する）。

### m3. 決定4の案内先がまだ存在しない（round1 m3の積み残し）

`docs/index.html`にアンインストール手順の節は無く、`uninstall.ps1`への言及が
643行目に1箇所あるだけ（しかも「ZIPアップグレード時に先に実行する必要はない」という
別文脈）。`README.md`/`README.en.md`にも無い。`docs/index.en.html`も対象。
M3を採って`<exe_dir>\backup\`にすれば案内は「`%LOCALAPPDATA%\awase`を消すだけ」で
済むので、決定4自体が縮む。

### m4. 「復元した」ことをユーザーに伝える設計が無い

無言でバックアップから書き戻すと、「既定値に戻したつもりが戻っていない」
「設定した覚えのない値になっている」という問い合わせを生む。B1で提案する
「既定値と一致したら復元」を採るならなおさら（MSIが書いた既定値をアプリが
上書きすることになる）。最低限`tracing::info!`＋トレイ通知か、
`awase-settings`のステータス欄に1行出すことを決定文に入れる。

### m5. バックアップ失敗時のログ運用（未解決事項3）は「毎保存ごとに警告」になりうる

`%LOCALAPPDATA%`が書き込み不可（企業ポリシー・ディスクフル）の環境では、
保存のたびに警告が出続ける。ベストエフォートである以上、
「初回失敗時のみ警告し、以降はプロセス内でフラグを立てて抑制する」程度の
方針を決めておくと、`awase.log`がバックアップ失敗で埋まるのを防げる。

### m6. テスト方針が未解決事項5の1行しかない。round1 M1と同じ「false green」を避けること

round1で指摘した通り、テスト手順の設計を誤ると**Blockerが実在しても合格する**。
最低限、以下を明示的な検証項目にすること:

- **B1の直接再現**: MSIインストール → 設定を編集 → `msiexec /x` →
  **同じMSIを再インストール** → 起動 → 編集内容が復元されているか。
  （「アンインストール後、再インストールせずに起動」ではB1を検出できない）
- **B2の直接再現**: スタートメニューのショートカットではなく、
  **ログオン時の自動起動（Runキー）経由**で起動した場合に、復元先が
  `%LOCALAPPDATA%\awase\config.toml`になっているか（CWDに出来ていないか）。
- **B3の直接再現**: 配列編集タブで「名前を付けて保存」を`layouts_dir`外の
  同名ファイルに対して行った後、バックアップが汚染されていないか。
- 手編集した`config.toml`（`AppConfig::save()`を通らない）が保護されるか（M1）。

---

## 総評

v2の方向（MSIを触らず、アプリが自分のデータに責任を持つ）は正しい。round1で
指摘した不可逆リスクは完全に消えており、`wix/main.wxs`側の既存保護
（ADR-099決定0）とも役割分担が明確——**MSIは「アップグレード時に上書きしない」、
アプリは「消えていたら戻す」**——という整理は妥当である。

ただし現在の決定文は、
- 発火条件（B1）、
- 書き込み先（B2）、
- バックアップ対象の範囲（B3）、
- 契機の網羅性（M1）、

という「自己修復機構の4要素」がいずれも曖昧または誤りであり、このまま実装すると
「動いているように見えて主要シナリオで何もしない」機構になる。B1〜B3とM1を
決定文に反映した上で、round3で再確認することを推奨する。
