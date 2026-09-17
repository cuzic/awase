---
id: ADR-178-companion-178-opus-review-round3
title: |-
  ADR-178（MSIアンインストール時のユーザーデータ保護）Opus敵対的レビュー round3
type: companion-doc
related_adr:
  - "ADR-178"
---

# ADR-178 敵対的レビュー round3（v3: 自己修復方式・Blocker対応版）

対象: `docs/adr/178-msi-uninstall-preserve-userdata.md`（v3、コード未実装）
既往: `opus-review-adr178-round1.md`（v1 `Permanent="yes"` 却下）、
`opus-review-adr178-round2.md`（v2 Blocker 3件）

## round2 Blocker 3件の解消確認

| round2 | v3の対応 | 判定 |
| --- | --- | --- |
| B1 復元が主要シナリオで発火しない | 決定2「存在しない **または** 内容が埋め込み既定値とバイト一致し、かつバックアップが存在し、かつバックアップ≠既定値」 | **解消**。下記の補強あり: `AppConfig::save()`は`toml::to_string_pretty`で書き戻す（`src/config.rs:892-895`）ためコメントが全て落ちる。したがってGUIで一度でも保存したユーザーの`config.toml`は出荷ファイルと**絶対にバイト一致しない**。「既定値と一致＝工場出荷版」という判定は、当初思うより誤検出しにくい。設定画面の「既定値に戻す」相当機能も同じ理由で誤爆しない |
| B2 復元先がCWDになりうる | 決定2「`current_exe().parent()`から構成した絶対パスにのみ書く／`resolve_next_to_exe()`新設／CWDフォールバックには決して書かない」 | **config.toml側は解消**。ただし同じ決定文の`layouts_dir`への適用が新たな回帰を生む（→B5） |
| B3 バックアップ対象が絞られていない | 決定1「`find_config_path()`の自動解決結果と一致する場合のみ」「`layouts_dir`配下かつ同梱6ファイル名のみ」 | **解消** |

round1・round2で挙げたMajor（M1〜M8）も、v3の決定1・2・3・5・6・7に
それぞれ反映されていることを確認した。方針としては実装に進んでよい段階に近い。

ただし**v3で新たに持ち込まれたBlockerが2件**ある。いずれも決定文の数行で解消できる。

---

## Blocker（v3で新規）

### B4. 決定1の契機3（ロード成功時のバックアップ更新）と決定2（復元）の**実行順序が不変条件として書かれていない**。順序を誤ると、MSI再インストール直後にバックアップが工場出荷値で上書きされ、ユーザーデータが恒久的に失われる

決定1の契機3:
> 起動時、`AppConfig::load()`/`.yab`読み込みが成功した時点でも、内容がバックアップと異なればバックアップを更新する

決定2の復元:
> 起動シーケンス（`bootstrap.rs`）の先頭で1回だけ呼ぶ明示的な関数（例: `ensure_user_data_present()`）

**この2つの相対順序が決定文のどこにも書かれていない。** 順序を誤ると:

```
1. MSI アンインストール → 再インストール
   → config.toml / layout/*.yab が工場出荷値で再配置される
   → backup\config.toml にはユーザーの設定が入っている（唯一の原本）
2. 起動シーケンスで「ロード成功 → バックアップ更新」が先に走る
   → 工場出荷値 ≠ backup\config.toml なので「異なる」と判定される
   → backup\config.toml が **工場出荷値で上書きされる**
3. その後 ensure_user_data_present() が走る
   → 復元条件「backup の内容が既定値と異なる」が false になっている
   → 復元されない
4. ユーザーの設定は本体からもバックアップからも消え、**復旧不能**
```

契機3は`.yab`にも効くので、同じ順序ミスで**同梱6ファイル全てのバックアップが
同時に失われる**（`LayoutEntry::scan_all`は`layouts_dir`内の全`.yab`を読む、
`crates/awase-windows/src/app/bootstrap.rs:236-242`）。

これは「実装者が気をつければよい」で済ませられない。理由:

- 決定1（バックアップ）と決定2（復元）は**別々の決定として独立に記述**されており、
  決定1を読んだだけで実装できてしまう。順序依存の存在が文面から読み取れない。
- `awase-settings.exe`側には`bootstrap.rs`に相当する明示的な起動シーケンスが無い。
  ロードは`SettingsApp::new`が直接行う（`crates/awase-settings/src/main.rs:542-556`、
  `find_config_path()` → `AppConfig::load` → `classify_load_error`）。
  「`new`の中でロードの直前に復元を入れる」か「ロードの直後にバックアップを入れる」かの
  判断が実装者に委ねられ、**片方だけ入れる**（＝復元なしでバックアップだけする）ことが
  容易に起こる。その`awase-settings.exe`を MSI 再インストール直後に一度でも起動すれば
  上記の破壊が確定する。
- 失敗が**静か**（エラーも警告も出ない）で、**不可逆**（原本が2箇所とも消える）。
  round2 B3と同じく、データを守るための機構がデータを壊す形。

**要求**: 決定文に不変条件として明記すること。最低限:

> **不変条件**: 同一プロセスの起動シーケンスにおいて、バックアップ更新（決定1契機3）は
> 復元ステップ（決定2 `ensure_user_data_present()`）が**完了した後**にのみ行う。
> 復元ステップを実装しない／通らない経路では、バックアップ更新も行ってはならない。

加えて、順序ミスを構造的に防ぐなら「復元とバックアップ更新を1つの関数
（`ensure_user_data_present()`）の中で順に行い、外部からバックアップ更新だけを
呼べるAPIを公開しない」設計にするのが確実。決定7のテストにも
「工場出荷値で上書きされた状態＋ユーザー設定入りバックアップ」から起動して
**バックアップ側が壊れていないこと**を確認する項目を足すこと
（現在の決定7は「復元されていること」しか見ておらず、先にバックアップが壊れた場合
復元もされないので検出はできるが、原因の切り分けができない）。

### B5. 決定2「`layouts_dir`側も同じ関数（`resolve_next_to_exe()`）を通す」は、既存の解決ロジックを壊す回帰になる

決定2:
> `layouts_dir`側（`bootstrap.rs`の`resolve_relative(&config.general.layouts_dir)`）も
> 同じ関数を通すことで同じ罠を避ける。

`resolve_next_to_exe()`はB2対策として「**存在チェックをせず**exe隣の絶対パスを
構成するだけ」と定義されている。これを`layouts_dir`の**解決（読み取り）**にも使うと、
`resolve_relative_to_exe()`が持っている2つの分岐が失われる:

1. **絶対パスの`layouts_dir`が壊れる**。`src/paths.rs:34-37`は絶対パスを
   そのまま返す。`config.general.layouts_dir`は文字列で、`validate()`が弾くのは
   `".."`を含む場合だけ（`src/config.rs:1039-1044`で`"layout"`へ差し替え）であり、
   **絶対パスは通る**。`layouts_dir = "D:\\my\\yabs"`と設定している既存ユーザーの
   配列ディレクトリが参照されなくなり、`show_no_layouts_dialog`で起動不能になる。
2. **`cargo run`での開発が壊れる**。`src/paths.rs:44-53`のワークスペースルート
   フォールバック（exeが`target/debug/`配下にあるときリポジトリ直下の`layout/`を見る）が
   失われ、`target/debug/layout`を見に行って何も見つからなくなる。この挙動は
   `src/paths.rs`のテスト`falls_back_to_workspace_root_when_run_from_cargo_target_dir`が
   「実際に踏んだ回帰」として明示的に固定しているもの。

B2が問題にしていたのは**書き込み先**であって読み取り経路ではない。

**要求**: 決定2を次のように分けて書くこと。

- **読み取り（既存パスの解決）**: 従来どおり`resolve_relative_to_exe()`。変更しない。
- **書き込み（復元先の決定）**: `resolve_next_to_exe()`。`config.toml`は常にこれ。
  `layouts_dir`については、**解決結果がexe隣の既定ディレクトリである場合にのみ復元する**
  （＝ユーザーが`layouts_dir`を別の場所に向けている場合は復元せず、警告ログのみ）。
  ユーザーが指定した任意のディレクトリに同梱`.yab`6本を勝手に書き込むのは、
  round2 B3で問題にした「ユーザーの領域を汚す」と同型の副作用になる。

---

## Major

### M1. 「埋め込み既定値とのバイト一致」はフェイルサイレント。改行コードが噛むうえ、決定3の「加工を挟まない」制約には強制力が無い

決定2のB1対応（＝v3の中核）は、`config.toml`/`.yab`が**バイト一致**することに
全面的に依存している。一致しなくなっても**何のエラーも出ず、復元が静かに発火
しなくなるだけ**で、CIも実機検証も緑のまま通る。壊れたことに気づけるのは
「MSI再インストールしたら設定が戻らなかった」というユーザー報告が来たときだけ。

具体的に噛む要因:

1. **改行コード**。`.gitattributes`は`crates/awase-windows/tests/golden/**`しか
   `eol=lf`に固定しておらず、`config.toml`/`layout/*.yab`は未指定。その
   `.gitattributes`のコメント自身が「Windowsランナーのgit既定`core.autocrlf=true`に
   よるチェックアウト時CRLF変換」を明記している。現状は`include_str!`もdistコピーも
   **同一ジョブ・同一チェックアウト**（`release.yml`の"Prepare distribution"と
   "Build MSI"は同じジョブ内）なので一致するが、この同一性は暗黙の前提である。
   将来「MSIだけ別ジョブでビルド」「`.gitattributes`に`* text eol=lf`を追加」
   「Linuxでクロスビルドしたバイナリを使う」のいずれかで一致が崩れる。
2. **BOM**。`scripts/install.ps1`/`uninstall.ps1`はUTF-8 BOM付きで保存されている
   （`cat`の先頭に`﻿`が出る）。`config.toml`が将来BOM付きで保存/変換されると同様に崩れる。
3. **決定3の「この経路に将来加工を挟まないことを制約とする」には実効性が無い**
   （ADRの文章は`release.yml`の変更をブロックしない）。

**要求**:
- 比較時に**改行正規化（`\r\n`→`\n`）とBOM除去**を行う。1行で書け、上記1・2を丸ごと無効化できる。
- `release.yml`に「`dist/config.toml`が`config.toml`とバイト一致すること」「`dist/layout/*.yab`が
  `layout/*.yab`とバイト一致すること」を検証するステップを足す（`Compare-Object`か
  `Get-FileHash`で数行）。決定3の制約に**機械的な強制力**を与える唯一の手段。
- 決定7に「埋め込み既定値と`layout/`・`config.toml`の実ファイルの一致」を確認する
  ユニットテストを含める（`include_str!`は同じファイルを見るので自明に一致するが、
  将来「埋め込みだけ別ファイルに切り出す」変更を検出できる）。

### M2. 埋め込み既定値の生成に`AppConfig::default()`を使ってはならない。`Default`の値は出荷`config.toml`と食い違っており、使うと起動不能になる

`GeneralConfig::default()`（`src/config.rs:470-480`）は

- `layouts_dir: "config"`
- `default_layout: "nicola.yab"`

だが、出荷される`config.toml`は

- `layouts_dir = "layout"`（`config.toml:4`）
- `default_layout = "nicola_keytop.yab"`（`config.toml:5`）

である。`Default`側の`layouts_dir`は**存在しないディレクトリ**であり、これで
生成すると`LayoutEntry::scan_all`が0件になり`show_no_layouts_dialog`で
起動失敗する（`crates/awase-windows/src/app/bootstrap.rs:236-248`）。

決定3は「リポジトリの実ファイルを`include_str!`」と書いており正しいが、
実装時に「ファイルを埋め込むより`AppConfig::default()`をserializeするほうが
きれい」と判断されるのは十分ありうる誘惑（コメントが落ちるので出力も小さい）。
これは`src/config.rs`のテスト`test_parse_app_config_defaults`が
`layouts_dir == "config"`を**正として固定している**ため、テストからも矛盾に気づけない。

**要求**: 決定3に「生成は必ず埋め込みバイト列の書き出しであり、
`AppConfig::default()`のserializeで代用してはならない（`Default`の
`layouts_dir`/`default_layout`は出荷値と異なる）」と明記する。
決定7のテストに「埋め込み既定値をparseすると`layouts_dir == "layout"`かつ
`default_layout`が埋め込み`.yab`リストに存在する」という不変条件を追加する。

### M3. `ensure_user_data_present()`を「`bootstrap.rs`の先頭で1回だけ」は実装できない。`layouts_dir`は`config.toml`を読んで`validate()`を通すまで確定しない

決定2は復元を起動シーケンスの先頭の1関数にまとめるとしているが、`layouts_dir`は
`config.general.layouts_dir`由来であり、**`config.toml`の復元→ロード→`validate()`**
を経ないと確定しない。しかも`validate()`は`".."`を含む値を`"layout"`へ差し替える
正規化を行う（`src/config.rs:1039-1044`）ため、**生の`layouts_dir`を使うと
実際に読まれるディレクトリと違う場所を触る**。これは
`crates/awase-windows/src/app/mod.rs:195-206`の不具合報告添付処理が
「生の`layouts_dir`をそのまま使うと`validate_layouts`の正規化が反映されず、
実際に読まれている`.yab`と異なる場所を見に行く」として既に一度踏んで直した罠と同型。

**要求**: 決定2を2フェーズに分けて書く。

1. フェーズ1: `config.toml`の復元（引数不要、exe隣固定）
2. `AppConfig::load()` → `validate()`
3. フェーズ2: `layouts_dir`（**validate後の値**をB5の規則で解決したもの）に対する`.yab`の復元

`awase-settings`側も同じ2フェーズが必要（`SettingsApp::new` main.rs:542-556 と
`ensure_layout_loaded` main.rs:1802-1816 の間に挟まる）。

### M4. `.yab`が「存在するが壊れている」ケースの方針が無い。config.tomlより実害が大きいのに決定5はconfigしか扱っていない

決定5は`ConfigLoadState`（`config.toml`専用）の`Dangerous`について
「自動復元しない」と決めているが、`.yab`側には対応する分類が無い。
`.yab`の破損は`LayoutEntry::scan_all`の`Err`または0件となり、
`init_engine_validated`が`Err`を返して**起動そのものが失敗する**
（`bootstrap.rs:236-248`）。config.tomlのDangerousより実害が大きい。

しかもこれは机上の話ではない。ADR-177の実機検証で
「（副次的な発見）壊れた`nicola_keytop.yab`によりエラーダイアログ」という
実例が記録されている（`docs/adr/177-msi-restart-manager-graceful-shutdown.md:257`）。

**要求**: 決定5に`.yab`側の方針を追加する。少なくとも
「parse/lintに失敗した`.yab`は復元対象にするか（config同様にしないのか）」
「`layouts_dir`に有効な`.yab`が1本も無い場合、`show_no_layouts_dialog`で
落とす前に埋め込み既定値から復旧を試みるか」を決めること。
後者は決定2の「解決されること: アプリが起動不能になる事態を防げる」という
主張が実際に成立するかどうかそのもの。

### M5. 未解決事項2（バックアップ処理の配置 (a)/(b)）を「実装時に決める」と保留すると、決定3・決定7の「4箇所同期」が5箇所・6箇所に増える

決定2は復元を`awase.exe`と`awase-settings.exe`の**両方**に置くとしている。
決定3の埋め込み既定値も、復元を行う側が持つ必要がある。配置を保留したままだと、
`include_str!`リストと復元対象リストが2クレートに重複するのが既定路線になる
（`crates/awase-windows`と`crates/awase-settings`は別クレートで、
`find_config_path()`が既に「同型ロジックを2箇所に持つ」形になっており
〈`crates/awase-settings/src/main.rs:5284-5289`のコメントが
「ズレていると実機バグになる」と警告している〉、同じ轍を踏む構図が既にある）。

**要求**: 今決めること。推奨は**(b)の変形**——

> コア`awase`クレートに、埋め込み既定値（`include_str!`）と
> `ensure_user_data_present(config_path: &Path, layouts_dir: &Path) -> RestoreOutcome`
> という**ディレクトリを引数で受け取る純粋関数**を1つだけ置く。
> `%LOCALAPPDATA%`やexe隣の解決（OS依存部分）は呼び出し元（`awase-windows`/
> `awase-settings`）に残す。

これならADR-019（コアのOS非依存）に反せず、M2のテストも決定7の同期テストも
1箇所に書けば済み、2バイナリ間のロジック乖離（既に実機バグを起こした失敗モード）も
構造的に防げる。決定1のバックアップ処理も同じ関数群に同居させれば、B4の順序
不変条件も1関数の中に閉じ込められる。

### M6. 復元自体が失敗したときの挙動が未定義

決定1はバックアップを「ベストエフォート（失敗してもログ警告のみ）」と明記しているが、
**復元側**（決定2）には同等の記述が無い。書き込みが失敗しうる状況は実在する:

- ポータブルZIPを読み取り専用メディア／`Program Files`配下に置いた場合
- 企業ポリシーで`%LOCALAPPDATA%`の一部が書き込み不可
- B5で指摘した「ユーザーが指定した`layouts_dir`」が存在しない/書けない

**要求**: 決定2に、(i) 復元失敗は`unwrap`/`panic`せずログ＋既存のエラー
ダイアログ経路（`crates/awase-windows/src/main.rs:49-53`）に落とすこと、
(ii) さらに踏み込むなら「ディスクに書けなくても、埋め込み既定値を**インメモリで**
使って起動を継続する」フォールバックを採るか、を書くこと。(ii)を採ると
「アプリが起動不能になる事態を防げる」という主張が書き込み権限に依存しなくなる。

---

## Minor

### m1. 決定1の根拠「ポータブルZIP版はexe相対で完結する設計」は半分しか正しい

`scripts/install.ps1:33`は`$installDir = "$env:LOCALAPPDATA\awase"`を
ハードコードしており、**ZIP版の公式インストール手順（`install.ps1`実行）でも
インストール先はMSIと同じ`%LOCALAPPDATA%\awase`**。exe相対で完結するのは
「解凍してそのまま`awase.exe`を実行する」ポータブル運用
（`docs/index.html`の「インストール不要。解凍してすぐ使えます」）だけ。

結論（`<exe_dir>\backup\`にする）は変わらず、むしろ「ZIP版とMSI版が同じ
インストール先ならバックアップも自然に共有される」という追加の利点がある。
根拠の記述だけ正確にしておくと、後から読んだ人が誤った前提で判断しない。

### m2. 決定6の通知タイミング（MSI再インストール直後はトレイがまだ無い）

MSIの`LaunchApplication`カスタムアクション（`wix/main.wxs:235-241`、
`Return="asyncNoWait"`、条件`NOT Installed`）はインストール完了直後に
`awase.exe`を起動する。復元は起動シーケンスの先頭で走るので、
その時点ではトレイアイコンが未生成の可能性が高い（トレイ生成失敗の分岐が
`main.rs`のヒントに存在することからも、生成は後段）。
「通知する」と決めておきながら実際には出ない、を避けるため、
トレイ生成後にdeferする／`awase-settings`のステータス欄に残す、のどちらかを
決定6に書いておくこと。**本ADRで最も通知が必要なのがまさにこの経路**
（MSI再インストール直後の復元）である点に注意。

### m3. 決定7のBlocker2再現手順は、ログオンを伴わずに安く再現できる

「ログオン時の自動起動（Runキー）経由で起動」は実機検証コストが高い。
本質は「CWDがINSTALLDIR以外のときに復元先が正しいか」なので、
`cmd /c "cd /d C:\Windows\System32 && %LOCALAPPDATA%\awase\awase.exe"`
のように**任意CWDから起動**すれば同じことを確認できる。Runキー経由の確認は
1回やれば十分で、回帰確認は安い手順で回せる、と書いておくとよい。

### m4. 契機3を「起動時」に限るのか、再読み込み経路にも適用するのかが未記載

`config.toml`は起動時以外にも再読み込みされる（トレイ経由のリロード、
`awase-settings`の`main.rs:677`の`AppConfig::load`等）。契機3を「起動時のみ」と
限定するのか、「ロード成功のたび」なのかで、手編集の捕捉タイミングが変わる。
どちらでも実害は小さいが、B4の順序不変条件は**すべてのバックアップ更新契機**に
かかるので、契機の集合を確定させておくこと。

### m5. リポジトリ直下の`config.toml`は開発時の実験対象でもあり、それがそのまま埋め込み既定値になる

`config.toml`は`cargo run`時に読まれる開発用設定でもある（`src/paths.rs`の
ワークスペースルートフォールバック）。ローカルで値をいじったままビルドすると、
その値が埋め込み既定値になり、M1のバイト比較の基準もローカル値になる。
配布ビルドはクリーンチェックアウトなので実害は無いが、「ローカルビルドと
配布ビルドで埋め込み内容が変わりうる」ことをどこかに1行残しておくと、
将来「手元では復元が効くのにMSIだと効かない」を調査する人の時間を節約できる。

---

## 総評

v3はround2のBlocker 3件を正しく解消しており、方針（MSIを触らず、アプリが
自分のデータに責任を持つ）とその役割分担（MSI＝アップグレード時に上書きしない、
アプリ＝消えていたら戻す）は妥当である。決定5・6・7の追加により、
「静かに壊れる」経路もかなり塞がれた。

残るB4・B5はいずれも**v3で新たに書き加えた文が原因**であり、決定文の
数行（B4: 順序の不変条件、B5: 読み取り経路と書き込み経路の分離）で解消できる。
M1（フェイルサイレント対策）とM5（配置を今決める）は、実装が始まってからだと
手戻りが大きいので、この段階で決定文に反映しておくことを強く推奨する。

これらを反映すれば、round4は確認のみで収束できる見込み。
