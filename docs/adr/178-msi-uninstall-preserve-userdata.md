---
id: ADR-178
title: |-
  MSIアンインストール時のユーザーデータ喪失を自己修復（バックアップ+復元）で無害化する
status: |-
  **起草中（v13）。B1前提とbackup\の生存を実機検証で確定。round12で
  Blocker2件（B17: round11 B15のゲートload_state==Loadedが本質を
  捕まえておらず、復元書き込み失敗時や可変フィールド経由の実装で
  同じ破壊が再現する、B18: UserDataGuardの型仕様がRustの可視性規則上
  誰でも構築できてしまう）とMajor6件を検出、v13で反映。round13
  レビュー待ち。round10〜12の3ラウンド連続で「型・ゲートが具体化した
  瞬間に新しい破壊経路が見える」パターンが続いている。**
related_adr:
  - "ADR-099"
  - "ADR-177"
---

# ADR-178: MSIアンインストール時のユーザーデータ喪失を自己修復（バックアップ+復元）で無害化する

## ステータス

**起草中v13（2026-09-17）。opus-adversarial-consultによるレビュー継続中
（round1〜12で計20件のBlockerを段階的に検出・解消、v13で反映）。
決定7が要求する実機確認（B1前提・`backup\`生存）はdragonflyg4で実施済み
（下記「実機検証結果」参照）。round10・round11・round12の3ラウンド
連続で「型・ゲートが具体化した瞬間に新しい破壊経路が見える」パターンが
続いており、v13反映後も実装着手前にround13で確認する。**

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

**結論（round11 M4で2段に分離）**: 採取した証拠が証明する範囲と、
決定2の復元条件が要求する範囲は別物であるため、明確に分ける。

1. **実機で確定した事実**: MSIアンインストールで`config.toml`/
   `layout/*.yab`は削除され、同じMSIの再インストールで**MSIパッケージ
   内蔵の出荷時ファイル**が再配置される（`config.toml`のSHA256が
   MSI内蔵ファイルと完全一致）。NeverOverwrite="yes"は「既存ファイルが
   あれば上書きしない」という意味であり、アンインストールでファイルが
   消えている以上、再インストール時は「既存ファイルなし」扱いで工場
   出荷値が新規配置される——これはMSIの標準動作であり、追加のコード
   変更なしに成立する。
2. **未確定（決定3のCIステップで別途担保する前提）**: 「MSI内蔵の
   出荷時ファイル」が「コア`awase`の`include_str!`埋め込み既定値」と
   （正規化後）バイト一致すること。今回の実機検証は同一マシン・同一
   チェックアウトから作った`dist/`と`include_str!`を比較する構造では
   ないため、この橋渡しを検証していない。`.github/workflows/release.yml`
   の`dist/`生成手順（`cp config.toml dist/`）と`include_str!`が同じ
   ソースファイルを指している事実だけが根拠であり、改行コード差
   （`config.toml`・`layout/*.yab`は`.gitattributes`で`eol=lf`固定
   されていない、Windowsチェックアウトの`core.autocrlf=true`で変わり
   うる）は決定3の「改行正規化＋BOM除去」とCIバイト一致チェックで
   別途担保する。

事実1（MSIが出荷時ファイルを再配置すること）は決定1〜5が前提としてきた
「復元条件の発火条件」の直接証拠であり、設計の骨格を変える必要はない。
事実2は決定3の実装（CIステップ、未実装）で担保する。

**もう1つの前提（round11 M5、追加で実機確認済み）**: `<exe_dir>\backup\`
自体が`msiexec /x`を生き残ることが本設計の生死を分ける。B1が真でも、
`backup\`がアンインストールで消えれば機構は全損する。

- `wix/main.wxs`の`RemoveFolder`（`RemoveInstallDir`・`RemoveLayoutDir`・
  `RemoveDataDir`・`RemoveAppFolder`）はいずれも**空のときだけ削除**する
  （再帰削除する`util:RemoveFolderEx`は使われていない）。
- 実測でも`%LOCALAPPDATA%\awase`に`awase.log`・`awase-settings.log`・
  `cache.toml`（MSI管理外ファイル）が残存した（上記手順3）。
- `INSTALLDIR`は`LocalAppDataFolder\awase`（per-userインストール）なので、
  実行時に`exe_dir`配下（＝`%LOCALAPPDATA%\awase`配下）へ書き込み権限が
  ある。per-machineインストールへ変更された場合はこの前提が崩れる。

**追加実機検証（2026-09-16、dragonflyg4）**: `%LOCALAPPDATA%\awase\backup\`
ディレクトリと`backup\config.toml`（テスト用ダミー内容）を手動で作成した
状態で`msiexec /x awase-1.20.6-x64.msi /qn`を実行し、アンインストール後に
`backup`ディレクトリと`backup\config.toml`の両方が**生存していること**を
`Test-Path`で確認した（いずれも`True`）。静的証拠（`RemoveFolder`の非
再帰・空ディレクトリのみ削除という挙動）と実機結果が一致し、この前提も
確定した。

## コンテキスト

[ADR-177](177-msi-restart-manager-graceful-shutdown.md)の実機検証で、MSIの
アンインストール（`msiexec /x`）が`%LOCALAPPDATA%\awase\config.toml`/
`layout/*.yab`を削除することが判明した。[ADR-099](099-config-preservation-on-upgrade.md)
決定1がZIP版に定めた「既定では残す」方針と非対称であり、ユーザーから
「ユーザーデータ削除するのおかしいね。残してほしい」との要望があった。

opus-adversarial-consultで12ラウンド、計20件のBlockerを検出・解消して
きた。各ラウンドの詳細な指摘は[178-opus-review-round1.md](178-opus-review-round1.md)〜
[round12.md](178-opus-review-round12.md)としてこのADRと同じディレクトリに
コミットしてある。決定文が変わっても必ず満たすべき制約は、下記
「実装チェックリスト」に独立して保持する。

## 実装チェックリスト（圧縮による欠落を防ぐための独立節）

1. `awase.exe`・`awase-settings.exe`の**両方**が、`AppConfig::load()`／
   `.yab`の読み込み／`AppConfig::save()`のいずれよりも**前**に
   `ensure_user_data_present()`を呼ぶ。呼び出し位置は`awase.exe`側は
   `bootstrap.rs`の起動シーケンス先頭、`awase-settings`側は
   `startup_failure::run_with_fallback`の**外**（呼び出し前）とする
   （round11 m4——クロージャ内に置くと、GUI起動失敗時に復元が走らない
   ケースが生じるため、外側に置いて常に復元を通す）。`EnsureOutcome`は
   `Clone`を実装し（`AppConfig`・`ConfigLoadState`は既存の`Clone`実装が
   あり、`RestoreOutcome`にも`#[derive(Clone)]`を追加、`UserDataGuard`は
   `Copy`）、`run_with_fallback`のクロージャ（`Fn`であり最大2回——glow
   失敗時にwgpu版へフォールバック——呼ばれうるため`move`できる値は
   `Clone`が必須、round12 M6）は`clone()`したものを`SettingsApp::new`へ
   渡す。レンダラーフォールバックで`SettingsApp::new`が2回走りうる点は
   決定6にも影響する（後述）。**ただし以下は呼ばない**:
   - CLI引数でconfigパスが明示されている場合（下記10）。
   - `--bug-report`・`--check-update`・`--scancode-map`（`main.rs:344`・
     `:348`・`:356`）の3つ、`SettingsApp`を構築せずに早期リターンする
     サブコマンド経路（round11 m3で列挙を具体化）。これらの経路で
     呼ぶと、不具合報告を開いただけでユーザー環境の`config.toml`/
     `.yab`が書き換わり、報告しようとした症状の再現性が失われる。
     特に`--scancode-map`は昇格プロセス（ADR-111決定4）であり、ここで
     `ensure_user_data_present()`が動くと昇格した権限でユーザー
     データを書くことになる——3つとも確実に除外すること。
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
   暗黙フラグは使わない。**このトークンは`pub struct UserDataGuard(());`
   （非公開のtuple struct、フィールドは`()`1個）として定義する
   （round12 B18）。「非公開フィールドのみを持つunit-like struct」という
   記述は誤りで、`pub struct UserDataGuard;`や`pub struct UserDataGuard
   {}`はどちらもRustの可視性規則上フィールドが0個のため他クレートから
   構築できてしまい、能力トークンとしての保証が消える。「unit-like」
   という語は使わない。** `Copy + Send`はtuple structのままでも
   維持できる（`awase-settings`の`config.toml`保存がワーカースレッドで
   行われるため、トークンをスレッド間で`move`する必要がある）。
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
    常にexe隣固定なので対象外。**この「相対パスである」という判定
    だけは`AppConfig`が保持する生の設定文字列`general.layouts_dir`に
    対して行う（round11 m2）。`resolve_layouts_dir()`（`awase-settings`
    側）は相対入力に対しても絶対`PathBuf`を返すため、解決後の値で
    判定すると条件が恒常的に偽になり、`.yab`のバックアップ・復元が
    一度も動かないまま無警告になる。**
    **判定と構築を混同しないこと（round12 M2）**: 相対/絶対の**判定**は
    生文字列で行うが、実際の`.yab`書き込み先パスの**構築**は
    `validate()`後の`layouts_dir`（`validate_layouts`が`..`を含む値を
    `"layout"`へ書き戻した後の値）を使う。生文字列をそのまま
    `exe_dir.join(生文字列)`してパスを組み立てると、`layouts_dir =
    "../.."`のような値でINSTALLDIRの外へ書き込む一方、アプリ自体は
    `validate()`後の`"layout"`しか読まない、という無警告のズレが生じる。
18. 決定1の契機1（`config.toml`保存後のバックアップ）は、保存先パスと
    復元先パスを**比較して**判定しない（round10 M3）。`awase.exe`・
    `awase-settings`の両方が、`AppConfig`の保存先として
    `ensure_user_data_present()`の戻り値が示す書き込み（復元）先
    パスを直接使う（＝そのパス以外への保存が構造的に発生しない）
    ようにする。**この単一経路化の対象は`AppConfig::save()`と
    `AppConfig::save_auto_start()`の両方（round11 B16）**——
    `save_auto_start`の2つの呼び出し元（`tray.rs`の自動起動トグル、
    `awase-settings`側の同等処理）も`find_config_path()`を独自に
    再解決せず、起動時に保持した`write_path`を使う。契機1は「その
    パスへの保存が成功し、かつチェックリスト#21のゲートを満たせば
    バックアップする」という規則にする（round10 M3で「無条件」とした
    表現は、round11 B15により#21のゲート必須に訂正）。**`write_path`は
    常に定義される値でなければならない（round12 M5）**——CLI引数で
    configパスが指定され`ensure_user_data_present()`をスキップする
    経路（チェックリスト#10）でも、呼び出し元はCLI指定パスを
    `write_path`に持つ薄い`EnsureOutcome`（`guard`は`None`、復元・
    バックアップは動作しない）を構築して使う。こうしないと「`write_path`
    以外へは書かない」という不変条件がCLI経路で破れ、決定7のソース
    スキャンテストが実装不能な要求になる。
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
21. **契機1・契機2のゲートは3条件の連言とする（round12 B17、round11
    B15の対応を訂正）**:
    (i) **`EnsureOutcome.load_state`（起動時スナップショット。
    `awase-settings`が保存成功のたびに書き換える可変フィールド
    `self.config_load_state`ではない——これをゲートの根拠にすることを
    名指しで禁止する）が`Loaded`**。
    (ii) `used_embedded_fallback == false`。
    (iii) **そのファイルについて復元が発火した場合、その書き込みが
    成功していること**（`RestoreOutcome`はファイル単位の成否
    `Restored`/`NotNeeded`/`Failed`を持ち、`Failed`ならそのファイルの
    バックアップ契機を全部止める）。
    「（または復元確定後の既知良好状態）」という曖昧な言い換えは使わない。
    この3条件がすべて揃わない限り、`Dangerous`時に`config = None`と
    なり`awase-settings`が`default_config()`にフォールバックした状態の
    保存や、**復元は発火したが`write_atomic`のrenameが失敗し
    ディスクは工場出荷値のまま残っている状態の保存**（MSI再インストール
    直後、AVスキャナ・OneDriveの干渉で発生しうる実在の失敗モード、
    `src/config.rs:877-884`のリトライ付きdocが記録している）のいずれも
    バックアップしない——バックアップしてしまうと、決定5が「復元し
    ますか？」と提案しようとしている唯一の正しいバックアップを既定値
    相当の内容で上書きしてしまう。この状態健全性ゲートは`UserDataGuard`
    （プロセスが復元ステップを通ったか）とは独立の追加条件であり、
    トークンだけでは防げない。決定7のB15テストは、保存完了直後に
    `self.config_load_state`を`Loaded`へ書き換えた状態でもバックアップ
    が発火しないことを検査すること（`EnsureOutcome`のスナップショット
    のみを参照している証拠として）。
22. `.yab`側の契機2は、パスの**比較ではなく選択**にする（round12
    M3、round11 M1の対応を訂正）。`EnsureOutcome`は
    `fn yab_write_path_for(&self, file_name: &str) -> Option<PathBuf>`
    を持ち（同梱6ファイル名のいずれかであり、かつ`layouts_dir_raw`が
    相対の場合にのみ`Some`）、`awase-settings`は同梱6ファイルを保存
    するときは必ずこの関数の戻り値へ書く。ユーザーがファイル選択
    ダイアログで指定した任意パスはこの関数を通らない＝構造的に
    バックアップ対象外になる、という切り分けを型で作る。既存の
    `main.rs:752`の`path != default_layout_path`という素の`PathBuf`
    比較（別目的、「エンジンには反映されません」の案内用）は流用しない
    （大文字小文字・`\\?\`プレフィクス等に弱い問題が契機1と同型で
    残るため）。**セッション中に`layouts_dir`を変更して保存した場合、
    `yab_write_path_for`は起動時に導出した集合のままなので一致しない
    （round12 M4）。この窓は「解決されないこと」に明記し、次回起動時の
    契機3で追いつく。**
23. `write_path`の導出にも開発ビルド分岐を入れる（round11 M2）。
    `exe_dir`の祖先に`target`が含まれる場合、`write_path`は
    **`<target ディレクトリの親>/config.toml`（＝ワークスペースルート
    直下、存在の有無によらず固定するパス）**にする（round12 M1、
    round11 M2の対応を訂正）。`resolve_relative_to_exe()`の結果を
    そのまま使うのは誤り——この関数の第4分岐は「exe隣にもワークス
    ペースルートにも見つからなければCWD相対の裸パスを返す」
    （`src/paths.rs:52-63`、`tracing::warn!`付き）ため、チェックリスト
    #11が禁じる「CWD相対の裸パスに書き込む」事故をそのまま再現する。
    `write_path`は常にワークスペースルート直下に固定し、この第4分岐へ
    は決して落とさない。これを怠ると、開発ビルドで一度でも設定を
    保存した時点で`target/debug/config.toml`が新規作成され、以後
    ワークスペースルートの`config.toml`（開発者が手編集する対象）が
    二度と読まれなくなる（B2/B5/B6/B12→round11 M2→round12 M1と続く
    「読み取り先/書き込み先」ファミリーの7回目の再発）。
24. `ConfigLoadState`（`src/config.rs:829-838`の**3バリアントenum**、
    `Loaded`/`NotFound`/`Dangerous(reason)`。決定1の型定義が
    `struct`と書いているのは誤りで`enum`が正しい）はそのまま使い、
    新バリアントを追加しない（round11 M3、(a)案採用）。復元書き込み
    自体が失敗しインメモリ既定値で起動を継続する状態は、
    `EnsureOutcome`に別フィールド`used_embedded_fallback: bool`を
    追加して表現する。既存の`awase-settings`側の`matches!(state,
    Dangerous(_))`という非網羅判定4箇所（`main.rs:786`・`911`・
    `1021`・`1124`）が新バリアントを静かに「安全」と誤解釈する事故を
    構造的に避けるため。
25. `AppConfig::validate()`は`self`を消費する（round11 m1）。
    `EnsureOutcome.config`に「`validate()`前の生の値」を入れるには、
    関数内部で`validate()`を呼ぶ前に`clone`する（`AppConfig: Clone`は
    既存コードで確認済み）。
26. 決定7のB14テスト（「`Dangerous`分類となる`config.toml`」の再現）は、
    「読み取り権限を奪ったファイル」ではなく「構造的に壊れたTOML」を
    既定の再現手段にする（round11 m5）。Linux CIがrootで動く環境では
    パーミッション変更が`PermissionDenied`にならず再現しないため。
27. 決定7のB9テストは`compile_fail`を**必須**とする（round12 B18）。
    「型で保証されるためテストは書かず、コードレビュー観点として扱う」
    という選択肢は削除する——`UserDataGuard`の可視性の欠陥（#4参照）は
    コードレビューでは気付きにくく、`compile_fail`テストが無ければ
    検出できない種類の欠陥だったため。
28. 決定2ステップ2の「`load_state`が`NotFound`または`Loaded`で、かつ
    復元書き込み自体が失敗しインメモリ既定値で継続する場合」という
    列挙のうち、**`Loaded`側は論理的に到達不能である**ことを明記する
    （round12 m1）。`AppConfig::load()`が成功した（＝`Loaded`）なら
    インメモリ既定値へ落ちる理由が無い。この組み合わせを「存在する」
    かのように読める書き方が、B17(b)（#21のゲートが`Loaded &&
    used_embedded_fallback`を通してしまうという誤読）の温床になった。
29. `save_auto_start()`は戻り値が`Option<Vec<String>>`で、`None`は
    「読み込み失敗」と「保存失敗」の両方を意味する（`src/config.rs:905-909`
    のdocが、空の`Vec`と区別するために`Option`にしてあると明記）。
    契機1（`save_auto_start`経由）は`Some(_)`が返った場合にのみ発火する
    （round12 m3）。
30. `save_auto_start()`は`AppConfig::validate()`を通した**正規化後**の
    値を保存する（`src/config.rs:922-928`、round12 m4）。したがって
    自動起動トグルを1回操作しただけで、ユーザーが手編集した生の値
    （範囲外の閾値等）が正規化され、その正規化後の内容がバックアップに
    捕捉される。「テキストエディタでの手編集もバックアップへ捕捉
    される」という説明はこの意味で理解すること（生の値そのものが
    保存されるとは限らない）。
31. 復元の「読む→判定→書く」はプロセスをまたいで原子的ではない
    （round12 m5）。`write_atomic`が保証するのは単一の書き込みの
    アトミック性のみ。`awase.exe`と`awase-settings.exe`がほぼ同時に
    起動した場合、一方が復元した直後にユーザーが保存し、遅れて起動した
    他方が起動時に採った古い判断で上書きする窓が理論上ある。書き込み
    直前に復元条件を再判定する（薄いチェックで足りる）。
32. `save_auto_start`を`write_path`固定にする際、`tray.rs:1042`の
    `find_config_path()`失敗時の専用エラー分岐（`config.toml`が見つから
    ない旨の`bail!`、`app/mod.rs:165-168`）が呼ばれなくなり、代わりに
    `save_auto_start`の`None`（読み込み失敗）に統合される（round12 m6）。
    移行時にユーザー向けメッセージの粒度を保つこと。

## 決定

### 決定1: バックアップ対象を絞り込んだ上で、保存の都度と起動時ロードの都度にMSI管理外へバックアップする

**バックアップ先**: `<exe_dir>\backup\`（`INSTALLDIR`配下）。

**バックアップ対象範囲**: `config.toml`と同梱6ファイルの`.yab`のみ。

**バックアップ契機**（いずれも「実行のゲート」節の対象）:

1. `config.toml`: `AppConfig::save()`**および**`AppConfig::save_auto_start()`
   （`Some(_)`が返った場合のみ、チェックリスト#29）（round11 B16、
   チェックリスト#18）成功後、**かつチェックリスト#21の3条件ゲート
   （起動時スナップショットの`load_state == Loaded`・
   `used_embedded_fallback == false`・復元が発火していれば成功して
   いること）を満たす場合に限り**（round12 B17、round11 B15の対応を
   訂正）バックアップする。パスの比較はしない——`awase.exe`・
   `awase-settings`の両方が、保存先として`ensure_user_data_present()`の
   戻り値が示す書き込み（復元）先パスを直接使う設計にすることで、
   「そのパスへ保存が成功した」こと自体が「復元先へ保存した」ことの
   証明になる。`Dangerous`状態で`default_config()`にフォールバックした
   内容の保存、および復元書き込みが失敗しディスクが工場出荷値のまま
   残っている状態での保存は、いずれもこのゲートによりバックアップ
   されない。
2. `layout/*.yab`: `layout_write_to_path()`成功後。保存先が
   `EnsureOutcome::yab_write_path_for(file_name)`が返すパスと一致し
   （＝候補集合から**選ばれた**場合、round12 M3、チェックリスト#22）、
   **かつチェックリスト#21の3条件ゲートを満たす**場合のみ。
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
pub struct UserDataGuard(());   // 非公開tuple struct、フィールドは()1個。Copy + Send（round12 B18、「unit-like」ではない）
pub enum ConfigLoadState { Loaded, NotFound, Dangerous(reason) }   // 既存型（src/config.rs、structではなくenum）。新バリアントは追加しない（round11 M3）
pub enum FileRestoreState { Restored, NotNeeded, Failed }   // ファイル単位の復元成否（round12 B17、チェックリスト#21）
pub struct EnsureOutcome {
    pub config: Option<AppConfig>,        // Dangerous時はNone（チェックリスト#16）
    pub load_state: ConfigLoadState,      // 起動時スナップショット、既存3バリアントのまま（チェックリスト#24）
    pub used_embedded_fallback: bool,     // 復元書き込み失敗時のインメモリ既定値継続を区別（round11 M3、チェックリスト#24）
    pub restore: RestoreOutcome,          // ファイル単位のFileRestoreState+バックアップ利用可否（チェックリスト#19・#21）
    pub write_path: PathBuf,              // config.tomlの書き込み（復元）先。CLI指定時も含め常に定義される（チェックリスト#18）
    pub guard: Option<UserDataGuard>,     // CLI指定パス経由の薄いEnsureOutcomeではNone（チェックリスト#18）
}
impl EnsureOutcome {
    pub fn yab_write_path_for(&self, file_name: &str) -> Option<PathBuf>   // 選択API、比較しない（round12 M3、チェックリスト#22）
}
pub fn ensure_user_data_present(exe_dir: &Path) -> EnsureOutcome
pub fn backup_config(guard: UserDataGuard, ..)   // トークンなしでは呼べない、かつ呼び出し元がチェックリスト#21の3条件を確認済みであること
pub fn backup_layout(guard: UserDataGuard, ..)
```

`UserDataGuard`は非公開のtuple struct（フィールド`()`1個）とし、
「unit-like struct」とは呼ばない（チェックリスト#4、round12 B18）——
`pub struct UserDataGuard;`や`pub struct UserDataGuard {}`はいずれも
Rustの可視性規則上フィールドが0個で他クレートから構築できてしまい、
能力トークンとしての保証が消える。`awase-settings`の`config.toml`保存
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
いる**——両者の役割は異なる。「復元は通ったが状態が不明なプロセス」を
止めるのはチェックリスト#21の3条件ゲートであり、トークンの役割ではない
（round12 B17も同じ非対称を突いたもの）。

（開発ビルドでの実際の挙動: `cargo run`実行時は対象限定判定と既定値
一致判定の両方により、バックアップも実質的にほとんど動作しない。
意図した設計目標ではなく単なる帰結であり、実害はない。）

### 決定2: 復元は単一関数として実装し、読み取り先・書き込み先を関数内部で導出し、両バイナリの起動シーケンスから必ず呼ぶ

**呼び出し要件と例外**: チェックリスト#1参照（両バイナリが必ず呼ぶ、
非GUIサブコマンド経路は除く）。**CLI引数でconfigパスが明示されている
場合（チェックリスト#10）は`ensure_user_data_present()`本体を呼ばないが、
呼び出し元はCLI指定パスを`write_path`に持つ薄い`EnsureOutcome`
（`config`はCLI指定パスから`AppConfig::load()`した値、`guard = None`、
`restore`は空、`yab_write_path_for`は常に`None`を返す）を構築して使う
（round12 M5、チェックリスト#18）——これにより「`write_path`以外へは
書かない」という不変条件が全経路で成立し、決定7のソーススキャンテストが
実装可能になる。**

**読み取り先と書き込み先の分離**: 関数は`exe_dir`のみを受け取り、内部で
以下を導出する（チェックリスト#11）。

- **書き込み（復元）先（`write_path`）**: 通常は`exe_dir.join("config.toml")`。
  **`exe_dir`の祖先に`target`が含まれる開発ビルドの場合は、
  `<target ディレクトリの親>/config.toml`（＝ワークスペースルート直下、
  存在の有無によらず固定するパス）にする（round12 M1、チェックリスト
  #23）。`resolve_relative_to_exe()`の結果をそのまま使わない**——
  この関数の第4分岐（exe隣にもワークスペースルートにも見つからなければ
  CWD相対の裸パスを返す、`src/paths.rs:52-63`）へ落ちると、#11が禁じる
  「CWD相対の裸パスへの書き込み」をそのまま再現してしまう。
- **`.yab`の書き込み先候補**: `EnsureOutcome::yab_write_path_for(name)`
  が、同梱6ファイル名`name`かつ`layouts_dir_raw`（生の設定文字列）が
  相対パスの場合にのみ`Some(exe_dir.join(layouts_dir_validated)
  .join(name))`を返す（round12 M2・M3、チェックリスト#17・#22）。
  相対/絶対の**判定**は生文字列`layouts_dir_raw`、パスの**構築**は
  `validate()`後の`layouts_dir_validated`（`..`を含む値が`"layout"`へ
  書き戻された後の値）を使う——判定・構築の両方を生文字列で行うと、
  `layouts_dir = "../.."`のような値でINSTALLDIRの外へ書き込みかねない。
  開発ビルドでは`write_path`と同じ考え方で従来の解決結果に切り替える。
- **読み取り先**: 復元が発火した場合は上記の書き込み先から読む。復元が
  発火しなかった場合は、従来の`resolve_relative_to_exe()`と同じ解決
  ロジック（exe隣→ワークスペースルート→見つからなければ諦める）の
  結果から読む。CWD相対の裸パスには決して書き込まない。

**呼び出し元が改めて`AppConfig::load()`を呼ぶ設計は禁止する（初期ロードに
限定、チェックリスト#3）**: `ensure_user_data_present()`は内部で
`AppConfig::load()`し、`validate()`を呼ぶ**前に`clone()`**した値を
`EnsureOutcome.config`に入れる（`validate()`は`self`を消費するため、
チェックリスト#25）。`validate()`自体は`layouts_dir`取得専用に内部で
呼ぶ（チェックリスト#20）。呼び出し元はこの戻り値を使い、起動時の
初期ロードを自分で改めて行わない。起動後の意図的な再読み込み
（`save_auto_start`のディスク読み直し等）はこの制約の対象外。

**`load_state`の扱い（チェックリスト#16・#24、round10 B14・round11
M3）**: `Dangerous`に分類された場合、`config`は`None`を返し、既定値
相当の値を黙って埋めない。呼び出し元（`awase-settings`）はこれを見て、
従来の`default_config()`を使うか、決定5の「バックアップから復元します
か？」UIを出すかを判断する。`ConfigLoadState`は既存の3バリアント
enum（`src/config.rs`の`classify_load_error`が返す分類、`Loaded`/
`NotFound`/`Dangerous(reason)`）を**そのまま**使い、新バリアントは
追加しない——`awase-settings`側の既存`matches!(state, Dangerous(_))`
判定4箇所が新バリアントを「安全」と誤解釈する事故を避けるため。復元
書き込み自体が失敗しインメモリ既定値で起動を継続する状態は、
`EnsureOutcome.used_embedded_fallback: bool`という別フィールドで表現
する（`load_state`自体は`NotFound`または`Loaded`のまま）。

**開発ビルドの除外**: 復元（および決定5の救済）は、引数で渡された
`exe_dir`の祖先に`target`という名前のディレクトリが含まれる場合には
行わない。この除外は`write_path`・`yab_write_path_for`の導出にも及ぶ
（上記「読み取り先と書き込み先の分離」参照、round11 M2・round12 M1）。

**実行順序（関数内部）**:

1. `config.toml`の復元（後述「`config.toml`の復元条件」を満たす場合。
   復元元は、バックアップが存在しかつ妥当ならバックアップ、無ければ
   埋め込み既定値）。書き込みの成否を`FileRestoreState`
   （`Restored`/`NotNeeded`/`Failed`）として`RestoreOutcome`に記録する
   （round12 B17、チェックリスト#21）——`write_atomic`のrenameが
   AVスキャナ・OneDrive等の干渉で失敗した場合は`Failed`とする。
2. `AppConfig::load()` → `clone()` → `validate()` → `classify_load_error`
   相当で`load_state`を得る。`load_state`が`Dangerous`の場合、
   `config = None`、以降の`.yab`復元と決定1の契機3は共にスキップする
   （M2の情報だけはステップ3'として別途埋める、後述）。`load_state`が
   `NotFound`で、かつ復元書き込み自体が失敗しインメモリの埋め込み
   既定値で起動を継続する場合は、`config`にこのインメモリ既定値を
   入れ、`used_embedded_fallback = true`にする（`load_state`自体は
   `NotFound`のまま変えない、round11 M3、チェックリスト#24）。
   **`Loaded`かつ`used_embedded_fallback = true`という組み合わせは
   論理的に到達不能である（`load()`が成功したならインメモリ既定値へ
   落ちる理由が無い、round12 m1、チェックリスト#28）**。この場合も
   `.yab`復元と契機3はスキップする（`layouts_dir`が既定値由来のまま
   書き込みに進むことを避けるため）。
3. 上記の可否判定を満たす場合（`Dangerous`でも`used_embedded_fallback`
   でもない場合、＝`load_state == Loaded`かつ埋め込み既定値へフォール
   バックしていない場合）、同梱6ファイルそれぞれについて後述「`.yab`の
   復元条件」を評価し、満たすものを復元する。復元元は、バックアップが
   存在しかつ妥当ならバックアップ、無ければ埋め込み既定値。書き込みの
   成否を`FileRestoreState`としてファイルごとに`RestoreOutcome`に記録
   する（ステップ1と同じ、チェックリスト#21）。
   （**別機構**、決定5参照）上記の復元を行ってもなお有効な`.yab`が
   1本も無い場合にのみ、決定5の救済が発動し、存在しない同梱`.yab`を
   埋め込み既定値から書き戻す。
   3'. `load_state`の値によらず（`Dangerous`でスキップした場合も含む）、
   `config.toml`について「バックアップが存在するか・妥当か」を判定し、
   `RestoreOutcome`にファイル単位で記録する（チェックリスト#19、round10
   M2）。これは実際の復元とは独立の読み取り専用チェックであり、
   `awase-settings`の「バックアップから復元しますか？」UIが使う。
4. 決定1の契機3（バックアップ更新）を実行する（ステップ2で`Dangerous`
   または`used_embedded_fallback`と判定された場合は行わない）。
5. `EnsureOutcome { config, load_state, used_embedded_fallback, restore,
   write_path, guard: Some(guard) }`を返す（`guard`は本関数を実際に
   通った場合は常に`Some`。`None`になるのはチェックリスト#18の
   CLI経由の薄い`EnsureOutcome`のみ）。

**チェックリスト#21のゲート判定に使う値は、この関数が返す
`EnsureOutcome`のスナップショット（`load_state`・`used_embedded_fallback`・
`restore`内の`FileRestoreState`）に限る。`awase-settings`が保存成功のたび
に書き換える可変フィールド`self.config_load_state`をゲートの根拠にして
はならない（round12 B17(c)）**——`awase-settings`で契機1を実装する最も
自然な箇所（保存完了を受け取る`poll_pending_save()`の`Saved`分岐、
`main.rs:846-874`）は、バックアップを書く直前の行（`main.rs:866`）で
`self.config_load_state`を`Loaded`へ書き換えるため、ここを参照すると
ゲートが実装時にno-op化する。

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
（戻り値の扱いはステップ2参照）。**この場合`RestoreOutcome`の該当
ファイルは`FileRestoreState::Failed`になり、チェックリスト#21のゲート
（iii）がその後のバックアップ契機を止める（round12 B17）——ディスク上
の内容が工場出荷値のままでも、ゲートは「復元が完了して手元の値が
ユーザーの本物の設定だと言える」ことまで要求する。**

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
) -> EnsureOutcome        // config, load_state, used_embedded_fallback, restore, write_path, guard: Option<UserDataGuard>
```

`backup_dir`は関数内部で`exe_dir.join("backup")`から導出する（round10
m3、チェックリスト#2）。`config_path`のような読み取り先と書き込み先を
兼ねる引数は持たない。`layouts_dir`は関数内部で得る。`.yab`の読み込み・
妥当性検証は`awase-windows`側の`LayoutEntry::scan_all`に依存せず、
`awase::yab`（コア）のパース関数を直接使う。`AppConfig::load`・
`validate()`・`awase::yab`のパース関数・`ConfigLoadState`はいずれも
コア`awase`クレートにあるため、ADR-019（コアのOS非依存）には抵触しない。

**呼び出し元は`write_path`と`yab_write_path_for()`を保持し続け、
`config.toml`を書くすべての経路（`AppConfig::save()`・
`AppConfig::save_auto_start()`の両方、round11 B16）でこれを使う。**
`tray.rs`側の自動起動トグル処理（現状`find_config_path()`を再解決して
いる）と`awase-settings`側の同等処理を、起動時に`APP`／`SettingsApp`が
保持した`write_path`を使う形に変更する。これを怠ると、`save_auto_start`
が独自解決したパスが`write_path`とずれた場合に「復元・バックアップ
対象ではないファイルへ書く」という無警告の事故が起こる（M3が指摘した
フェイルサイレントがこのバイパス経路で再発する）。CLI引数経由の
薄い`EnsureOutcome`（チェックリスト#18）も同じ`write_path`を持つため、
呼び出し元は`ensure_user_data_present()`を実際に呼んだかどうかで
保存側のコードを分岐させる必要がない。

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

**レンダラーフォールバック（glow失敗→wgpu）で`SettingsApp::new`が
2回走りうる（round12 M6、チェックリスト#1）ため、この通知は冪等にする**
——同じ`EnsureOutcome`から2回通知が出ても、ユーザーには1回分の情報にしか
見えないようにする（例: 直前に同一内容を通知済みなら再送しない）。

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
- B14の直接再現: `Dangerous`分類となる`config.toml`（**構造的に壊れた
  TOML**を既定の再現手段とする、round11 m5——読み取り権限を奪う方式は
  Linux CIがrootで動く環境では`PermissionDenied`にならず再現しない）を
  与えたとき、戻り値の`config`が`None`であり、既定値相当の値
  （`layouts_dir == "config"`等）が紛れ込まないことを確認する。
- round10 M1の直接再現: `layouts_dir`が絶対パスの環境で`.yab`を保存・
  起動しても`backup\`に`.yab`が作られないこと。判定は`general.
  layouts_dir`の生文字列に対して行う（round11 m2、解決後の絶対パスで
  判定すると条件が恒常的に偽になり無警告で機能しないことの回帰確認も
  含む）。
- round10 M2の直接再現: `Dangerous`ケースでも`EnsureOutcome.restore`に
  「バックアップが存在し妥当か」の情報が（復元は発火しなくても）
  埋まっていること。
- round10 M3の回帰確認: `awase.exe`・`awase-settings`双方の保存経路が
  `EnsureOutcome.write_path`由来のパスのみを使い、それ以外のパスへの
  `AppConfig::save()`**および`AppConfig::save_auto_start()`**（round11
  B16）が存在しないことをソーススキャンで確認する
  （`architecture_guard.rs`類似の検出テスト、検出対象に`save_auto_start`
  を含めること）。
- round11 B15の直接再現: `Dangerous`状態から`awase-settings`が
  `default_config()`で起動し、その状態のまま「適用」で保存しても
  `backup\config.toml`が変化しない（既存のバックアップが上書きされ
  ない）こと。
- round11 B16の直接再現: `save_auto_start()`経由の保存が
  `EnsureOutcome.write_path`と異なるパスへ書かない（または書いた場合に
  ソーススキャンで検出される）こと。
- round11 M1の直接再現: `.yab`の保存先が`yab_write_path_for()`の戻り値と
  異なる場合（例: ユーザーがファイル選択で別ディレクトリを指定した場合）
  バックアップされないこと（round12 M3で「選択」APIに訂正）。
- round11 M2の直接再現: `cargo run`相当（開発ビルド）で設定を1回保存
  しても、次回起動時に読み取り先がワークスペースルートの`config.toml`
  のままであること（`target/debug/config.toml`に切り替わらないこと）。
- round12 B17の直接再現（3パターン）:
  (a) 復元条件が成立し復元の書き込みが失敗する状況（`write_atomic`を
  モック等で失敗させる）を作り、`FileRestoreState::Failed`になり、
  その後の保存でバックアップが発火しないこと。
  (b) `awase-settings`が保存完了時に`self.config_load_state`を`Loaded`
  へ書き換えた**後**でも、ゲートが`EnsureOutcome`起動時スナップショット
  を見ているためバックアップが発火しないこと。
  (c) round11 B15の再現手順（`Dangerous`→`default_config()`→1項目
  直して「適用」）でバックアップが変化しないこと（既存のround11 B15
  テストを包含・強化する）。
- round12 B18の直接再現: `pub struct UserDataGuard(());`が、非公開
  フィールドを持つため他クレートから構築できないことを`compile_fail`
  テストで検証する（チェックリスト#27、「レビュー観点として扱う」
  選択肢は削除）。
- round12 M1の直接再現: ワークスペースルートに`config.toml`が存在しない
  開発環境（新規チェックアウト相当）でも、`write_path`がCWD相対の裸
  パスにならず、ワークスペースルート直下の固定パスになること。
- round12 M2の直接再現: `layouts_dir = "../x"`のconfigで、
  `yab_write_path_for()`が`exe_dir`配下から出るパスを返さないこと。
- round12 M5の直接再現: CLI引数でconfigパスを指定した経路でも
  `write_path`が定義され、決定7のソーススキャンテストが違反として
  誤検出しないこと。
- B1の直接再現（config.toml）。
- B1・B8の直接再現（.yab、バックアップ有りの一般規則経路）。
- B6の直接再現（バックアップ無し、決定5の救済経路と明示的に分離した
  独立テスト）。
- B4の直接再現（バックアップ側が上書きされないこと）。
- B7の直接再現（開発ビルドで復元が発火しないこと）。
- B9の直接再現（round12 B18によりチェックリスト#27で`compile_fail`必須
  化、上記round12 B18テストが実質この項目を兼ねる）。
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
- round12 m3の直接再現: `save_auto_start()`が`None`を返した場合（保存
  失敗）に契機1が発火しないこと。
- round12 m4の回帰確認: `save_auto_start()`経由のバックアップに、
  `validate()`後の正規化された値が入ること（生の手編集値そのままでは
  ないこと）。

**実機確認（自動化不可）**: **B1前提（出荷時ファイルの再配置）と
`backup\`のアンインストール生存（round11 M5）は完了**（2026-09-16、
dragonflyg4、1.20.6 MSI。上記「実機検証結果」参照）。B1修正は「MSI
再インストール時、`%LOCALAPPDATA%\awase\config.toml`と`layout\*.yab`が、
埋め込み既定値と（正規化後）バイト一致する内容で再配置される」という
前提の上に成り立つが、実機で確認できたのは「MSI内蔵の出荷時ファイルが
再配置されること」まで（上記「実機検証結果」M4参照、埋め込み既定値との
バイト一致は決定3のCIステップで別途担保）。以下は**未実施**で、実装
完了後の別タスクとして残す:

- 「その後起動し、編集内容が復元されることを確認する」（＝復元ロジック
  自体の実機確認）。コード未実装のため今回は対象外。

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
- セッション中に`layouts_dir`を変更して「適用」した直後の`.yab`保存は、
  起動時に導出した`yab_write_path_for()`の集合と一致しないためバック
  アップされない（round12 M4）。次回起動時の契機3で追いつく。
- 復元の「読む→判定→書く」はプロセスをまたいで原子的ではなく、
  `awase.exe`と`awase-settings.exe`をほぼ同時に起動した場合に理論上の
  競合窓がある（round12 m5、チェックリスト#31で軽減するが完全には
  閉じない）。

## 未解決事項 / 次のアクション

1. opus-adversarial-consult round13（B17・B18・Major6件・Minor6件反映後
   の確認）。round10〜12の3ラウンド連続で「型・ゲートが具体化した瞬間に
   新しい破壊経路が見える」パターンが出ているため、round13でBlocker
   ゼロを確認してから実装着手する。
2. `RestoreOutcome`の正確な型設計（`FileRestoreState`によるファイル単位
   の成否と、「バックアップ利用可否」の両方を含む形で確定する）。
3. `layout_write_to_path()`を`write_atomic`経由にするかどうかの判断。
4. 決定6の通知タイミングの実装方式確定。
5. 決定7のB9テストは`compile_fail`必須に確定済み（チェックリスト#27、
   round12 B18）。
6. `find_config_path()`のCLI引数判定ロジックを`awase::paths`へ共通化
   するかどうかの判断（しない場合は2クレート同時変更を徹底する）。
7. round10以降のレビュー記録（`178-opus-review-round10.md`以降）にも、
   既存のround1〜9と同様にfrontmatter付与と`docs/adr/index.md`補助資料
   節への登録を行う（継続タスク）。
8. B1前提と`backup\`のアンインストール生存（round11 M5）の実機確認は
   完了（上記参照）。**残るのは復元ロジック自体の実機確認**（コード
   実装後、MSIアンインストール→再インストール→
   起動→編集内容が実際に復元されることを確認する1回）。
9. `config.toml`・`layout/*.yab`を`.gitattributes`で`eol=lf`固定する
   かどうかの判断（round11 M4後半・round12 m2、現状`.gitattributes`は
   `crates/awase-windows/tests/golden/**`のみ対象）。
