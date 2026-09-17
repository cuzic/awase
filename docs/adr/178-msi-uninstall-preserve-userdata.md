---
id: ADR-178
title: |-
  MSIアンインストール時のユーザーデータ喪失を自己修復（バックアップ+復元）で無害化する
status: |-
  **起草中（v11）。B1前提を実機検証で確定（MSI再インストール時、
  config.toml/nicola_keytop.yabはMSIパッケージ内蔵の出荷時ファイルと
  SHA256バイト一致で再配置される）。round10のBlocker1件（B14、戻り値に
  ConfigLoadStateが無くADR-099決定4の保証が壊れる）とMajor3件を反映。
  round11レビュー待ち。**
related_adr:
  - "ADR-099"
  - "ADR-177"
---

# ADR-178: MSIアンインストール時のユーザーデータ喪失を自己修復（バックアップ+復元）で無害化する

## ステータス

**起草中v11（2026-09-16）。opus-adversarial-consultによるレビュー継続中
（round1〜10で計16件のBlockerを段階的に検出・解消、v11で反映）。
決定7が要求する実機確認（B1前提）はdragonflyg4で実施済み
（下記「実機検証結果」参照）。**

## 実機検証結果（2026-09-16、dragonflyg4、1.20.6 MSI）

decision7の実機確認手順どおりに実施した:

1. `awase-1.20.6-x64.msi`をクリーンインストール。
2. `%LOCALAPPDATA%\awase\config.toml`の`simultaneous_threshold_ms`を
   `100`→`777`に、`layout\nicola_keytop.yab`に識別用の行
   （`# B14-MARKER-EDIT`）を追記して編集。
3. `msiexec /x awase-1.20.6-x64.msi /qn`でアンインストール。
   → `config.toml`・`layout\nicola_keytop.yab`は削除された
   （`Test-Path`＝`False`）。`%LOCALAPPDATA%\awase`ディレクトリ自体は
   残存（`awase.log`・`awase-settings.log`・`cache.toml`は残る）。
4. **同じMSI**（`awase-1.20.6-x64.msi`）を再インストール。
5. 起動前（`awase.exe`のプロセスが編集を加える前）に採取:
   - `config.toml`が存在し（`True`）、内容は`simultaneous_threshold_ms = 100`
     （編集前の工場出荷値。`777`は失われている）。
   - `layout\nicola_keytop.yab`が存在し、末尾を目視確認したところ
     `# B14-MARKER-EDIT`は消えており工場出荷値に戻っていた。
   - `config.toml`のSHA256ハッシュ（`996E3967...`）が、MSIパッケージ
     自体に同梱されている`config.toml`のハッシュと**完全一致**。

**結論**: B1前提（「MSI再インストール時、`config.toml`/`layout/*.yab`が
埋め込み既定値相当の内容で再配置される」）は真であることが実機で確定した。
NeverOverwrite="yes"は「既存ファイルがあれば上書きしない」という意味で
あり、アンインストールでファイルが消えている以上、再インストール時は
「既存ファイルなし」扱いで工場出荷値が新規配置される——これはMSIの
標準動作であり、追加のコード変更なしに成立する。この事実は、決定1〜5が
前提としてきた「復元条件（config.tomlの内容が埋め込み既定値とバイト
一致すること）」の発火条件がまさにこの動作によって満たされることの
直接証拠であり、設計の骨格を変える必要はない。

## コンテキスト

[ADR-177](177-msi-restart-manager-graceful-shutdown.md)の実機検証で、MSIの
アンインストール（`msiexec /x`）が`%LOCALAPPDATA%\awase\config.toml`/
`layout/*.yab`を削除することが判明した。[ADR-099](099-config-preservation-on-upgrade.md)
決定1がZIP版に定めた「既定では残す」方針と非対称であり、ユーザーから
「ユーザーデータ削除するのおかしいね。残してほしい」との要望があった。

opus-adversarial-consultで10ラウンド、計16件のBlockerを検出・解消して
きた。各ラウンドの詳細な指摘は[178-opus-review-round1.md](178-opus-review-round1.md)〜
[round10.md](178-opus-review-round10.md)としてこのADRと同じディレクトリに
コミットしてある。決定文が変わっても必ず満たすべき制約は、下記
「実装チェックリスト」に独立して保持する。

## 実装チェックリスト（圧縮による欠落を防ぐための独立節）

1. `awase.exe`・`awase-settings.exe`の**両方**が、`AppConfig::load()`／
   `.yab`の読み込み／`AppConfig::save()`のいずれよりも**前**に
   `ensure_user_data_present()`を呼ぶ。呼び出し位置は`awase.exe`側は
   `bootstrap.rs`の起動シーケンス先頭、`awase-settings`側は
   `SettingsApp::new`の直前とする。**ただし以下は呼ばない**:
   - CLI引数でconfigパスが明示されている場合（下記10）。
   - `--bug-report`等、`SettingsApp`を構築せずに早期リターンする
     サブコマンド経路（`crates/awase-settings/src/main.rs`の
     `main()`冒頭にある、`SettingsApp::new`より前の分岐）。
     これらの経路で呼ぶと、不具合報告を開いただけでユーザー環境の
     `config.toml`/`.yab`が書き換わり、報告しようとした症状の再現性が
     失われる。
2. `ensure_user_data_present()`は`exe_dir`のみを引数に取り、内部で
   `backup_dir`（`exe_dir.join("backup")`）を含む読み取り先・書き込み先の
   両方を導出し、`AppConfig::load()`→`validate()`を行って`layouts_dir`を
   得る（round10 m3、`backup_dir`は`exe_dir`から一意に導出できるため
   引数を分けない）。
3. `ensure_user_data_present()`の戻り値は`EnsureOutcome`構造体とし、
   `config: Option<AppConfig>`（`validate()`前の生の値、`Dangerous`時は
   `None`）と`load_state: ConfigLoadState`を含める（round10 B14、後述
   「戻り値の型」参照）。**このうち`config`を初期ロードとして使う制約は
   「起動シーケンスにおける初期ロード」に限定される**——
   `AppConfig::save_auto_start`（`src/config.rs`、自動起動トグルの度に
   ディスクから意図的に読み直す設計、他プロセスの未保存編集を巻き込ま
   ないための既存の意図的実装）、設定画面の「適用」時のリロード、更新
   チェック（`update_check.rs`）など、**起動後の意図的な再読み込みは
   この制約の対象外**であり従来どおりでよい。
4. バックアップの書き込みは、`ensure_user_data_present()`が返す能力
   トークン（`UserDataGuard`）を持つ場合にのみ実行できる。グローバルな
   暗黙フラグは使わない。**このトークンは`Copy + Send`な非公開フィールド
   のみを持つunit-like structとする**（`awase-settings`の`config.toml`
   保存がワーカースレッドで行われるため、トークンをスレッド間で
   `move`する必要がある。構築手段が`ensure_user_data_present()`のみで
   あることは`Copy`でも維持できる）。
5. バックアップ対象は`config.toml`と同梱6ファイルのみ。ユーザー自作の
   `.yab`は対象外。
6. `config.toml`の復元条件と`.yab`の復元条件は異なる（後述）。
7. `.yab`が1本も有効でない場合の救済は復元条件とは別機構。
8. 復元・救済は、`exe_dir`の祖先に`target`という名前のディレクトリが
   含まれる場合は行わない（開発ビルド除外）。
9. `GeneralConfig::default()`のserializeを埋め込み既定値の代用にしない。
   `validate()`前の生の設定値を復元判定に使わない。
10. CLI引数でconfigパスが明示されている場合、`ensure_user_data_present()`
    は呼ばない。呼び出し元は従来どおり指定パスを`AppConfig::load()`する。
    この判定（`find_config_path()`の実装）は`awase-windows`と
    `awase-settings`の2クレートに同型実装されている既存の依存であり
    （過去に実機バグを起こした前例あり）、変更する場合は両方を同時に
    直すこと。
11. 読み取り先と書き込み先は別物であり、1つの引数・1つのパス値で兼ねて
    はならない。書き込み（復元）先は常に`exe_dir.join("config.toml")`等、
    存在に依存しない値。読み取り先は、復元が発火した場合はその書き込み
    先、発火しなかった場合は従来の解決ロジック（exe隣→ワークスペース
    ルート→見つからなければ諦める）の結果。CWD相対の裸パスには決して
    書き込まない。
12. **`.yab`の妥当性検証（契機3のバックアップ対象判定・復元前の検証の
    両方で共通の基準を使う）**: `keyboard_model`に依存しない形で
    **構造検証を行う**。具体的には、`KeyboardModel`の全バリアント
    （`Jis`・`Us`）で`YabLayout::parse`を試し、**いずれか1つでも成功
    すれば妥当**とみなす（バリアント数が2つしかないためコストは無視
    できる）。`awase::yab::lint`は警告の表示・ログ用にのみ使い、妥当性の
    ゲートにはしない（`lint`は構造的に不正な行を意図的に読み飛ばす
    設計であり——`src/yab/mod.rs`のコメントが「構造検証は`parse`の
    責務」と明記している——「致命的エラー」という概念も持たないため、
    単体では検証として機能しない）。**（round10 m1）現状`Jis`の行サイズは
    `Us`の上位互換のため「全バリアント試行」は実質`Jis`単独と等価だが、
    将来モデルが増えたときに自動的に正しくなるようにあえて全バリアント
    を試す。「どうせ`Jis`だけで同じだから」と単純化しないこと。**
13. 契機3の`.yab`読み込みは、`awase-windows`側の`LayoutEntry::scan_all`
    （`cfg(windows)`・`pub(super)`、コアから到達不能かつLinuxホスト
    ターゲットに存在しない）に依存しない。同梱6ファイル名をコア関数が
    直接読み、#12と同じ基準（全`KeyboardModel`のいずれかでパース成功）
    でバックアップ対象とする。
14. 決定6の通知は**ファイル単位**で行い、`backup\<ファイル名>`の削除を
    個別に案内する。
15. 決定7の実機確認1件（MSI再インストール後の内容が埋め込み既定値と
    バイト一致することを、起動前に採取して確認）は自動テストで代替
    できない。**2026-09-16、dragonflyg4・1.20.6 MSIで実施済み
    （上記「実機検証結果」参照）。**
16. 戻り値`EnsureOutcome`は`load_state: ConfigLoadState`を含み、
    `Dangerous`時に`config`（`Option<AppConfig>`）を黙って既定値相当の
    値にせず`None`を返す（round10 B14、ADR-099決定4の「危険な失敗を
    静かに既定値へフォールバックさせない」保証を維持するため）。
17. `.yab`のバックアップ対象・復元対象は、`validate()`後の`layouts_dir`が
    **相対パス**である場合に限る（round10 M1）。絶対パスの環境では
    バックアップ自体を作らない——「バックアップはあるが復元には
    決して使われない」という非対称を避けるため。`config.toml`は
    常にexe隣固定なので対象外。
18. 決定1の契機1（`config.toml`保存後のバックアップ）は、保存先パスと
    復元先パスを**比較して**判定しない（round10 M3）。`awase.exe`・
    `awase-settings`の両方が、`AppConfig`の保存先として
    `ensure_user_data_present()`の戻り値が示す書き込み（復元）先
    パスを直接使う（＝そのパス以外への保存が構造的に発生しない）
    ようにし、契機1は「そのパスへの保存が成功したら常にバックアップ
    する」という無条件の規則にする。
19. `Dangerous`時に「バックアップから復元しますか？」を提案するために
    必要な情報（バックアップの有無・妥当性）は、`EnsureOutcome`の
    `RestoreOutcome`にファイル単位で含める（round10 M2 (a)案）。
    `awase-settings`のUI側が独自に`backup\`を読んで妥当性判定を
    再実装することを禁止する（判定ロジックはコアの1箇所に集約する
    という既存方針、round7 M4を維持するため）。
20. `ensure_user_data_present()`内部の`validate()`呼び出しは
    `layouts_dir`取得専用とし、警告ログを出力しない（round10 m4）。
    呼び出し元（`bootstrap.rs`／`awase-settings`）が戻り値の`config`に
    対して改めて`validate()`を呼ぶ際に警告を1回だけ出す、という既存の
    責務分担を維持する。

## 決定

### 決定1: バックアップ対象を絞り込んだ上で、保存の都度と起動時ロードの都度にMSI管理外へバックアップする

**バックアップ先**: `<exe_dir>\backup\`（`INSTALLDIR`配下）。

**バックアップ対象範囲**: `config.toml`と同梱6ファイルの`.yab`のみ。

**バックアップ契機**（いずれも「実行のゲート」節の対象）:

1. `config.toml`: `AppConfig::save()`成功後、**無条件で**バックアップする
   （チェックリスト#18、round10 M3）。パスの比較はしない——
   `awase.exe`・`awase-settings`の両方が、保存先として
   `ensure_user_data_present()`の戻り値が示す書き込み（復元）先
   パスを直接使う設計にすることで、「そのパスへ保存が成功した」こと
   自体が「復元先へ保存した」ことの証明になる。
2. `layout/*.yab`: `layout_write_to_path()`成功後。保存先が現在の
   `layouts_dir`配下であり、かつファイル名が同梱6ファイルのいずれかと
   一致し、**かつ`layouts_dir`が相対パスである**（チェックリスト#17、
   round10 M1）場合のみ。
3. 起動時、`ensure_user_data_present()`が内部で読み込んだ
   `config.toml`と同梱6ファイル名（チェックリスト#12・#13の基準で
   パース成功したもの、かつ`.yab`は`layouts_dir`が相対パスの場合のみ
   ——チェックリスト#17）の内容がバックアップと異なれば、バックアップを
   更新する。

**バックアップを作成・更新しない条件**: 対象ファイルの内容が現在の
埋め込み既定値と一致する場合は、バックアップを作成・更新しない
（既存のバックアップも上書きしない）。

**この副作用と、初期化操作を残す方法**: 決定6の通知に、どのファイルを
復元したかを含め、「該当ファイルを`backup\`から削除してください」という
個別の案内を示す。

**書き込み方式**: `crate::fs_atomic::write_atomic`を使う。

**実行のゲート（能力トークン方式）**: バックアップ書き込みは、
`ensure_user_data_present()`が返す明示的な能力トークンを要求する形で
実装する。

```
pub struct UserDataGuard { /* 非公開・Copy + Send、unit-like struct */ }
pub struct ConfigLoadState { .. }   // 既存型（src/config.rs）。Loaded/NotFound/Dangerous(reason)相当
pub struct EnsureOutcome {
    pub config: Option<AppConfig>,      // Dangerous時はNone（チェックリスト#16）
    pub load_state: ConfigLoadState,
    pub restore: RestoreOutcome,        // ファイル単位の復元結果+バックアップ利用可否（チェックリスト#19）
    pub write_path: PathBuf,            // config.tomlの書き込み（復元）先。保存側が直接使う（チェックリスト#18）
    pub guard: UserDataGuard,
}
pub fn ensure_user_data_present(exe_dir: &Path) -> EnsureOutcome
pub fn backup_config(guard: UserDataGuard, ..)   // トークンなしでは呼べない
pub fn backup_layout(guard: UserDataGuard, ..)
```

`UserDataGuard`は`Copy + Send`な非公開フィールドのみを持つunit-like
structとする（チェックリスト#4）——`awase-settings`の`config.toml`保存
（`AppConfig::save()`成功後のバックアップ、契機1）はワーカースレッド
上で行われるため、`SettingsApp`が保持するトークンをそのワーカー
スレッドへ渡す必要がある。`Copy`にしても、構築手段が
`ensure_user_data_present()`のみであるという保証は損なわれない
（非公開フィールドのため外部からは構築できない）。

**この能力トークンが防ぐのは「復元ステップを一度も通らないプロセスが
バックアップを汚す」ことであり、B9で問題になった「両バイナリとも復元は
通るが、片方が復元前の内容を基準に保存し、もう一方の復元条件が不成立に
なってバックアップが誤って上書きされる」という損失は、トークンでは
なくチェックリスト#1（両方が必ず呼ぶ）と#3（戻り値だけを使い、自分で
改めてloadしない、ただし起動後の意図的な再読み込みは対象外）が防いで
いる**——両者の役割は異なる。

（開発ビルドでの実際の挙動: `cargo run`実行時は対象限定判定と既定値
一致判定の両方により、バックアップも実質的にほとんど動作しない。
意図した設計目標ではなく単なる帰結であり、実害はない。）

### 決定2: 復元は単一関数として実装し、読み取り先・書き込み先を関数内部で導出し、両バイナリの起動シーケンスから必ず呼ぶ

**呼び出し要件と例外**: チェックリスト#1参照（両バイナリが必ず呼ぶ、
CLI引数指定時と非GUIサブコマンド経路は除く）。

**読み取り先と書き込み先の分離**: 関数は`exe_dir`のみを受け取り、内部で
以下を導出する（チェックリスト#11）。

- **書き込み（復元）先**: 常に`exe_dir.join("config.toml")`（`.yab`は
  `exe_dir.join(layouts_dir).join(名前)`、`layouts_dir`は`validate()`後の
  相対パスの場合のみ）。
- **読み取り先**: 復元が発火した場合は上記の書き込み先から読む。復元が
  発火しなかった場合は、従来の`resolve_relative_to_exe()`と同じ解決
  ロジック（exe隣→ワークスペースルート→見つからなければ諦める）の
  結果から読む。CWD相対の裸パスには決して書き込まない。

**呼び出し元が改めて`AppConfig::load()`を呼ぶ設計は禁止する（初期ロードに
限定、チェックリスト#3）**: `ensure_user_data_present()`は内部で
`AppConfig::load()`→`validate()`を行い、`EnsureOutcome{ config, load_state, .. }`
を戻り値として返す。呼び出し元はこの戻り値を使い、起動時の初期ロードを
自分で改めて行わない。起動後の意図的な再読み込み（`save_auto_start`の
ディスク読み直し等）はこの制約の対象外。

**`load_state`の扱い（チェックリスト#16、round10 B14）**: `Dangerous`に
分類された場合、`config`は`None`を返し、既定値相当の値を黙って
埋めない。呼び出し元（`awase-settings`）はこれを見て、従来の
`default_config()`を使うか、決定5の「バックアップから復元しますか？」
UIを出すかを判断する。`ConfigLoadState`は既存型（`src/config.rs`の
`classify_load_error`が返す分類、`Loaded`/`NotFound`/`Dangerous(reason)`
相当）をそのまま使う。

**開発ビルドの除外**: 復元（および決定5の救済）は、引数で渡された
`exe_dir`の祖先に`target`という名前のディレクトリが含まれる場合には
行わない。

**実行順序（関数内部）**:

1. `config.toml`の復元（後述「`config.toml`の復元条件」を満たす場合。
   復元元は、バックアップが存在しかつ妥当ならバックアップ、無ければ
   埋め込み既定値）。
2. `AppConfig::load()` → `validate()` → `classify_load_error`相当で
   `load_state`を得る。`load_state`が`Dangerous`の場合、`config = None`、
   以降の`.yab`復元と決定1の契機3は共にスキップする（M2の情報だけは
   ステップ3'として別途埋める、後述）。`load_state`が`NotFound`または
   `Loaded`で、かつ復元書き込み自体が失敗しインメモリの埋め込み既定値で
   起動を継続する場合は、`config`にこのインメモリ既定値を入れつつ
   `load_state`を専用の値（例: `FallbackToEmbeddedDefault`）にして
   区別できるようにする。この場合も`.yab`復元と契機3はスキップする
   （`layouts_dir`が既定値由来のまま書き込みに進むことを避けるため）。
3. 上記の可否判定を満たす場合（`Dangerous`でも
   `FallbackToEmbeddedDefault`でもない場合）、同梱6ファイルそれぞれに
   ついて後述「`.yab`の復元条件」を評価し、満たすものを復元する。復元元は、
   バックアップが存在しかつ妥当ならバックアップ、無ければ埋め込み
   既定値。
   （**別機構**、決定5参照）上記の復元を行ってもなお有効な`.yab`が
   1本も無い場合にのみ、決定5の救済が発動し、存在しない同梱`.yab`を
   埋め込み既定値から書き戻す。
   3'. `load_state`の値によらず（`Dangerous`でスキップした場合も含む）、
   `config.toml`について「バックアップが存在するか・妥当か」を判定し、
   `RestoreOutcome`にファイル単位で記録する（チェックリスト#19、round10
   M2）。これは実際の復元とは独立の読み取り専用チェックであり、
   `awase-settings`の「バックアップから復元しますか？」UIが使う。
4. 決定1の契機3（バックアップ更新）を実行する（ステップ2で`Dangerous`/
   `FallbackToEmbeddedDefault`と判定された場合は行わない）。
5. `EnsureOutcome { config, load_state, restore, write_path, guard }`を
   返す。

**復元条件は`config.toml`と`.yab`で異なる**:

- **`config.toml`の復元条件**: ファイルが「存在しない」**または**
  「内容が埋め込み既定値とバイト一致し、かつバックアップが存在し、
  かつバックアップの内容が既定値と異なる」場合に復元する。
- **`.yab`の復元条件**: 「バックアップが存在し妥当である」場合に限り
  復元する。バックアップが無い`.yab`は、存在しなくても復元しない。

**バックアップの妥当性検証**: チェックリスト#12の基準
（全`KeyboardModel`のいずれかで`YabLayout::parse`が成功）を用いる。
`config.toml`は`toml::from_str`。不正な場合は復元せず警告ログのみ。

**書き込み方式**: `crate::fs_atomic::write_atomic`を使う。

**失敗時の挙動**: 復元の書き込みが失敗しても`panic`せず、ログ警告の
うえ既存のエラーダイアログ経路に委ねる。書き込みが失敗した場合でも
埋め込み既定値をインメモリで使って起動を継続するフォールバックを持つ
（戻り値の扱いはステップ2参照）。

### 決定3: 埋め込み既定値はビルド時にリポジトリのファイルから直接取り込み、生成元を`AppConfig::default()`にしない

`include_str!("../config.toml")` / `include_str!("../layout/nicola.yab")`
等、コア`awase`クレート（`src/`直下）からリポジトリルートの実ファイルを
直接参照する。`GeneralConfig::default()`のserializeを代用しない。

**バイト一致の頑健化**: 比較前に改行正規化（`\r\n`→`\n`）とBOM除去を
行う。`.github/workflows/release.yml`に`dist/`配下と埋め込み既定値の
バイト一致を検証するCIステップを追加する。

### 決定4: 「完全に削除したい」場合の案内を更新する

MSI版・ZIP版ともに、完全削除の案内を「`%LOCALAPPDATA%\awase`を削除
してください」に更新する。案内先は`docs/index.html`・`docs/index.en.html`
にアンインストール手順の節を新設する。ZIP版`scripts/uninstall.ps1 -Purge`
は変更不要。

### 決定5: `.yab`側の救済手段と、実装配置

**`.yab`のDangerous相当ケースの救済手段**: `layouts_dir`に有効な`.yab`
が1本も無い場合、`show_no_layouts_dialog`で起動を諦める前に、埋め込み
既定値からの復旧を試みる。

**`config.toml`のDangerous分類**: `classify_load_error`が`Dangerous`に
分類した場合は自動復元しない。既存の`config.toml.bak`退避に加えて、
`awase-settings`のUIで「バックアップから復元しますか？」と提案する
（`EnsureOutcome.restore`が示す「バックアップが存在し妥当か」を使って
表示可否を決める、チェックリスト#19）。`ConfigLoadState::NotFound`分岐は
削除しない。

**実装配置**: コア`awase`クレートに、埋め込み既定値（`include_str!`群）
と、復元のエントリポイントとなる純粋関数`ensure_user_data_present()`
**ただ1つ**（バックアップ書き込み関数`backup_config`/`backup_layout`は
これとは別にコアへ置く。復元のエントリポイントが1つ、という意味）を置く。

```
pub fn ensure_user_data_present(
    exe_dir: &Path,       // OS依存の解決結果（current_exe().parent()）のみ呼び出し元から
) -> EnsureOutcome        // config: Option<AppConfig>, load_state, restore, write_path, guard
```

`backup_dir`は関数内部で`exe_dir.join("backup")`から導出する（round10
m3、チェックリスト#2）。`config_path`のような読み取り先と書き込み先を
兼ねる引数は持たない。`layouts_dir`は関数内部で得る。`.yab`の読み込み・
妥当性検証は`awase-windows`側の`LayoutEntry::scan_all`に依存せず、
`awase::yab`（コア）のパース関数を直接使う。`AppConfig::load`・
`validate()`・`awase::yab`のパース関数・`ConfigLoadState`はいずれも
コア`awase`クレートにあるため、ADR-019（コアのOS非依存）には抵触しない。

### 決定6: 復元・生成をユーザーに通知する

`tracing::info!`に加えて、トレイ通知または`awase-settings`のステータス
欄で、`EnsureOutcome.restore`が示すファイルごとの結果を表示する。既定値に
戻したい場合は、通知に含めたファイル名を使って「`backup\<ファイル名>`
を削除してください」と案内する。MSI`LaunchApplication`によるインストール
直後の自動起動では、復元時点でトレイアイコンがまだ生成されていない
可能性が高いため、トレイ生成後に通知をdeferする処理を持たせるか、
`awase-settings`側のステータス欄に残す形にするかを実装時に決める。

`load_state`が`Dangerous`の場合は復元自体が発火しないため、この通知
経路では扱わない。`awase-settings`起動時に別途「バックアップから復元
しますか？」の確認UIを出す（決定5参照、`EnsureOutcome.restore`の
バックアップ有無・妥当性情報を使う）。

### 決定7: 回帰テスト

原則としてホストターゲット（Linux、`cargo test --lib`）で実行できる
自動テストとして書く。`ensure_user_data_present()`がコア`awase`クレート
の純粋関数であり、`cfg(windows)`側の型に一切依存しない設計のため、
ホストターゲットでの実行が保証される。

**テストの配置分担**（round7 m3、round10 m2で再確認）: `wix/main.wxs`を
含む照合は`crates/awase-windows/tests/`側（`wix_installer_guard.rs`の
前例と同型）、埋め込み既定値の内部整合（`layouts_dir`不変条件、B1/B8/B13等
の`ensure_user_data_present()`直接呼び出し）はコア`awase`の`#[cfg(test)]`
側、とクレートをまたいで分担する。冒頭の「原則としてホストターゲット」は
後者（コア側の純粋関数テスト）についての記述であり、`wix/main.wxs`照合
まで含意しない。

- 埋め込み既定値をparseすると`layouts_dir == "layout"`かつ
  `default_layout`が埋め込み`.yab`リストに存在するという不変条件。
- 3箇所同期テスト（コア`awase`の定数・`layout/`の実ファイル・
  `wix/main.wxs`のコンポーネント）。
- B14の直接再現: `Dangerous`分類となる`config.toml`（例: 読み取り権限を
  奪ったファイル、または`classify_load_error`が`Dangerous`と判定する
  既存の壊れ方）を与えたとき、戻り値の`config`が`None`であり、既定値
  相当の値（`layouts_dir == "config"`等）が紛れ込まないことを確認する。
- round10 M1の直接再現: `layouts_dir`が絶対パスの環境で`.yab`を保存・
  起動しても`backup\`に`.yab`が作られないこと。
- round10 M2の直接再現: `Dangerous`ケースでも`EnsureOutcome.restore`に
  「バックアップが存在し妥当か」の情報が（復元は発火しなくても）
  埋まっていること。
- round10 M3の回帰確認: `awase.exe`・`awase-settings`双方の保存経路が
  `EnsureOutcome.write_path`由来のパスのみを使い、それ以外のパスへの
  `AppConfig::save()`が存在しないことをソーススキャンで確認する
  （`architecture_guard.rs`類似の検出テスト）。
- B1の直接再現（config.toml）。
- B1・B8の直接再現（.yab、バックアップ有りの一般規則経路）。
- B6の直接再現（バックアップ無し、決定5の救済経路と明示的に分離した
  独立テスト）。
- B4の直接再現（バックアップ側が上書きされないこと）。
- B7の直接再現（開発ビルドで復元が発火しないこと）。
- B9の直接再現: `UserDataGuard`を経由しないとバックアップ書き込み
  関数を呼べないことは型システムによる保証であり、通常の`#[test]`では
  検証できない。`trybuild`等のcompile-failテスト、またはdocテストの
  `` ```compile_fail `` 属性のいずれかで表現するか、「型で保証される
  ためテストは書かず、コードレビュー観点として扱う」と明示する
  （実装時に選択）。
- B13の直接再現: `keyboard_model = "us"`環境で、同梱JIS用配列
  （列数超過で`Us`モデルでは`parse`が失敗する）をバックアップとして
  持つ状態から復元し、正しく復元されることを確認する（チェックリスト
  #12の「いずれか1つのモデルで成功すれば妥当」の検証）。破損した
  バックアップ（truncateされたファイル等）は、どのモデルでも
  `parse`が失敗し復元されないことも確認する。
- M1（発火しすぎる副作用）の直接再現: バックアップの無い同梱`.yab`を
  削除した状態で起動しても復活しないこと。一度も編集していない
  `config.toml`を保存してもバックアップが作成されないこと。
- B2（CWD誤書き込み）の直接再現。
- B3（バックアップ汚染）の直接再現。
- B5の回帰確認: `src/paths.rs`の既存テストが変更されないことを条件と
  する。
- B12の直接再現: `cargo run`相当（`exe_dir`が`target/debug`配下）で
  ワークスペースルートの`config.toml`が正しく読み込まれること。
- `--bug-report`経路で`ensure_user_data_present()`が呼ばれず、
  ユーザー環境のファイルが変化しないことを確認する（round9 M1対応）。
- `save_auto_start`が引き続きディスクから読み直す挙動を保つことの
  既存テストが変更されないことを確認する（round9 M2対応）。
- 手編集した`config.toml`が、次回起動時のロード成功をトリガーに
  バックアップへ捕捉されることを確認する。
- release.ymlに、`dist/`配下と埋め込み既定値のバイト一致を検証する
  ステップを追加する。

**実機確認（自動化不可）**: **完了（2026-09-16、dragonflyg4、1.20.6 MSI。
上記「実機検証結果」参照）。** B1修正は「MSI再インストール時、
`%LOCALAPPDATA%\awase\config.toml`と`layout\*.yab`が、埋め込み既定値と
（正規化後）バイト一致する内容で再配置される」という前提の上に成り
立つが、この前提はMSIパッケージ内蔵ファイルとのSHA256完全一致で実証
された。「その後起動し、編集内容が復元されることを確認する」（＝復元
ロジック自体の実機確認）は、コード未実装のため今回は未実施——これは
実装完了後の別タスクとして残す。

## この設計で解決されること・されないこと

**解決されること**:
- MSIアンインストール→再インストール後、`config.toml`/`layout/*.yab`が
  出荷時の既定値で再配置されても、次回起動時にバックアップから実質的に
  復元される。
- `layouts_dir`に有効な`.yab`が1本も無い状態でアプリが起動不能になる
  事態を、埋め込み既定値からの復旧で防ぐ。
- ユーザーが使わない同梱配列を削除して整理しても、毎起動復活しない。
- テキストエディタでの手編集も、次回起動時のロード成功をトリガーに
  バックアップへ捕捉される。
- `awase.exe`・`awase-settings.exe`のどちらを先に起動しても、両方が
  復元ステップを通り、かつ戻り値だけを使う設計により、片方だけが復元前
  の古い内容を基準に保存してバックアップを汚染する事態を防ぐ。
- バックアップの書き込みが能力トークンによって型レベルでゲートされ、
  復元ステップを通らないプロセスはバックアップを一切汚せない
  （ワーカースレッドをまたぐ保存経路でもトークンを受け渡せる）。
- 開発ビルドでは復元・救済が一切動作せず、リポジトリのトラッキング
  済みファイルを汚染しない。
- 不具合報告を開く操作や、自動起動トグルの意図的なディスク再読み直しが、
  自己修復機構によって意図せず妨げられたり、環境の状態を変えたりしない。
- US配列等、`keyboard_model`が既定と異なる環境のカスタマイズされた
  バックアップも、環境差を理由に不正判定されず復元される。壊れた
  （構造として不正な）バックアップは正しく検出され復元されない。

**解決されないこと**:
- MSIのアンインストール自体は引き続き`config.toml`/`layout/*.yab`を
  削除する。
- `Dangerous`分類のケースは自動復元されない。
- ユーザーが`layouts_dir`を明示的に別ディレクトリ（絶対パス）へ向けて
  いる場合、そのディレクトリへの自動復元もバックアップ自体も行わない
  （round10 M1対応、チェックリスト#17。バックアップだけ作られて復元には
  使われない、という無意味な複製を避けるため）。
- ユーザーが`layouts_dir`に自作した（同梱6ファイルに含まれない）
  `.yab`は、バックアップ・復元いずれの対象にもならない。
- ユーザーがconfig.tomlを壊したまま再起動を繰り返す間は、`.yab`復元も
  連動してスキップされる。
- ポータブルZIP運用でexeごと別の場所へ移動すると、`backup\`もその
  exeに付随して移動するため、旧場所のバックアップは参照されなくなる。
- 「意図的に既定値へ戻す」「削除して初期化する」という操作は、
  バックアップが残っている限り無効化される（決定6の通知案内で該当
  ファイルの`backup\`削除手順を示す）。
- インストール先パスに`target`という名前のディレクトリが偶然含まれる
  場合、自己修復機構全体が無効になる（安全側に倒れるため許容）。

## 未解決事項 / 次のアクション

1. opus-adversarial-consult round11（B14・M1〜M3・Minor反映後の確認）。
2. `RestoreOutcome`の正確な型設計（`EnsureOutcome`の一部として、
   ファイル単位の「バックアップ利用可否」フィールドを含む形で確定する）。
3. `layout_write_to_path()`を`write_atomic`経由にするかどうかの判断。
4. 決定6の通知タイミングの実装方式確定。
5. 決定7のB9テストの実装方式（compile-fail vs レビュー観点）確定。
6. `find_config_path()`のCLI引数判定ロジックを`awase::paths`へ共通化
   するかどうかの判断（しない場合は2クレート同時変更を徹底する）。
7. round10以降のレビュー記録（`178-opus-review-round10.md`以降）にも、
   既存のround1〜9と同様にfrontmatter付与と`docs/adr/index.md`補助資料
   節への登録を行う（継続タスク）。
8. B1前提の実機確認は完了（上記参照）。**残るのは復元ロジック自体の
   実機確認**（コード実装後、MSIアンインストール→再インストール→
   起動→編集内容が実際に復元されることを確認する1回）。
