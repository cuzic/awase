---
id: ADR-178
title: |-
  MSIアンインストール時のユーザーデータ喪失をPermanent化+自己修復で防ぐ
status: |-
  **起草中（v14、全面差し替え）。v1〜v13（バックアップ+復元方式、12ラウンド・
  Blocker20件）を破棄し、round1が当初提案していた方向へ回帰した、
  よりシンプルな設計に作り直した。実装済み・実機検証4項目すべて完了
  （round1 B2の既存ユーザー遡及効果を含めPermanent="yes"の効果を確認、
  かつ実機検証中に発見した「読み取り先/書き込み先」バグ1件を修正し
  修正後も実機確認済み）。opus-adversarial-consultレビュー完了
  （[178-opus-review-v14.md](178-opus-review-v14.md)、総合判定「実装
  やり直し不要」）、Blocker2件・Major推奨5件・フォローアップ3件
  （M4/M6/M7）すべて反映済み。残るのはMinor6件（任意）と、opusレビュー
  後のコード変更分の実機再検証のみ。**
related_adr:
  - "ADR-099"
  - "ADR-177"
---

# ADR-178: MSIアンインストール時のユーザーデータ喪失をPermanent化+自己修復で防ぐ

## ステータス

**起草中v14（2026-09-17）。方針転換により全面差し替え。実装済み。実機検証
（dragonflyg4）4項目すべて完了。opus-adversarial-consultレビュー完了
（[178-opus-review-v14.md](178-opus-review-v14.md)）、Blocker2件・
Major推奨5件・フォローアップ3件（M4/M6/M7）すべて反映済み。残るのは
Minor6件（任意）とopusレビュー後の変更分の実機再検証のみ（未解決事項
参照）。**

## 方針転換の経緯（重要、実装者は必ず読むこと）

このADRはv1（起草時、[選択肢A: `Permanent="yes"`単独](#旧v1で検討した選択肢アーカイブ)）→
opus round1でBlocker 2件により却下→v2〜v13（「バックアップ+復元」の自己修復方式、
12ラウンドで計20件のBlockerを検出・解消しながら`EnsureOutcome`/`UserDataGuard`/
`FileRestoreState`等の複雑な型を積み上げた）という経緯を辿った。

v13時点でユーザーから「設計が長すぎないか、もっと根本的にシンプルな、業界標準の
やり方はないのか」という指摘があった。round1のレビュー記録
（[178-opus-review-round1.md](178-opus-review-round1.md) M5）を読み直すと、
**round1自身が「選択肢D: ユーザーデータをMSIの管理下から完全に外し、アプリが
自己生成する」という、v2以降とは異なるもっとシンプルな方向を既に提案していた**
ことが分かった。この選択肢Dはv2以降のどのラウンドでも採用されなかった
（v2は「MSI管理を維持しつつバックアップ+復元」という、選択肢Dとは別の複雑な道を
選んだ）。

選択肢Dを素朴に採用する（MSIから`ConfigFile`等のコンポーネント定義を完全に
削除する）ことも検討したが、これには**移行時の重大な欠陥**がある: 新しいMSI
パッケージからコンポーネント定義自体を削除すると、Windows Installerは次回の
メジャーアップグレード時に「この製品はもうこのコンポーネントを持たない」と
判断し、**既存ユーザーの現在のconfig.toml/`.yab`を削除してしまう**
（ADR-099決定0がまさにこの標準動作を「GUID不変」で防いでいたのに、GUID自体を
無くせば防御が効かなくなる）。ユーザーから「移行時も含めて、修正済みの設定
ファイルがそのまま使えるように配慮してね」という明確な要求があったため、
選択肢Dの素朴な採用はこの要求を満たさない。

そこで、**round1のB1要求(b)が既に示していた組み合わせ**——「`Permanent`を
採るなら、同じPRでアプリ側に自己修復（無ければ埋め込み既定値から生成）を
必ず入れる」——を採用する。これは以下2つを両方行うハイブリッド案である。

1. **既存7コンポーネントに`Permanent="yes"`を追加するだけ**（GUID・
   `NeverOverwrite`・`Schedule`は一切変更しない）。ファイルにもレジストリにも
   一切触れないため、**既存ユーザーの現在の設定は移行時に完全に無傷で残る**。
   アンインストール時にも二度と削除されなくなる。
2. **アプリ側に「`config.toml`/`.yab`が存在しなければ埋め込み既定値から
   生成する」自己修復ロジックを追加**（メインの安全網）。round1が指摘した
   `Permanent`固有のリスク（後述のB1「レジストリ残留によるファイル再配置の
   ブロック」）は、この自己修復ロジックがあれば実害化しない——MSIがファイルを
   配置しなくても、アプリ自身が生成するため。

この組み合わせにより、v2〜v13が作り込んだ`EnsureOutcome`/`UserDataGuard`/
`FileRestoreState`/能力トークン/複数ラウンドにわたる「読み取り先・書き込み先」
問題（通算7回再発していた）は、**構造的に発生しなくなる**——自己修復ロジックは
「存在しなければ作る」だけであり、既存ファイルの内容を一切読み書き・比較・
バックアップしないため、これまでの複雑さの大半が前提としていた「バックアップと
実ファイルの整合」という問題自体が存在しない。

## コンテキスト

[ADR-177](177-msi-restart-manager-graceful-shutdown.md)の実機検証で、MSIの
アンインストール（`msiexec /x`）が`%LOCALAPPDATA%\awase\config.toml`/
`layout/*.yab`を削除することが判明した。[ADR-099](099-config-preservation-on-upgrade.md)
決定1がZIP版に定めた「既定では残す」方針と非対称であり、ユーザーから
「ユーザーデータ削除するのおかしいね。残してほしい」との要望があった。

不具合対応でよくある案内「一度アンインストールして入れ直してください」を
MSIユーザーが実行すると、`config.toml`の全設定と配列編集タブで作り込んだ
`layout/*.yab`が警告なく消える。これはADR-099を起票させた元のユーザー
報告「バージョンアップすると既存の設定が失われる」と体感上同じ症状になる。

## 旧v1で検討した選択肢（アーカイブ）

v1で検討した4つの選択肢（A: `Permanent="yes"`単独、B: カスタムアクションで
退避→復元、C: 現状維持、D: MSI管理から完全除外）の詳細な比較検討は
[178-opus-review-round1.md](178-opus-review-round1.md)に記録されている。
v14はA（B1/B2対応済み）とD（の一部、自己修復ロジック）を組み合わせた形になる。

## 実機検証結果（B1前提、2026-09-16、dragonflyg4、1.20.6 MSI。バックアップ+
復元方式v13で実施したものだが、事実自体はv14でも前提として成立する）

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
   - `config.toml`のSHA256ハッシュが、MSIパッケージ自体に同梱されている
     `config.toml`のハッシュと完全一致。

**結論**: MSIアンインストールは現状`config.toml`/`layout/*.yab`を削除し、
再インストールは工場出荷値を再配置する。この事実（B1前提）はv14の設計変更
理由の前提であり続ける。

さらに`<exe_dir>\backup\`ディレクトリと中のダミーファイルを手動作成した状態で
アンインストールし、両方が生存することを確認済み（`wix/main.wxs`の
`RemoveFolder`が非再帰・空ディレクトリのみ削除という静的挙動と一致）——これは
MSI管理外のファイル・ディレクトリがアンインストールで自動的に削除されない
ことの直接証拠であり、v14のPermanent化がこの生存特性を`config.toml`/`.yab`
自体にも適用しようとするものだと理解できる。

## 決定

### 決定1: 既存7コンポーネントに`Permanent="yes"`を追加する

`wix/main.wxs`の`ConfigFile`・`NicolaYab`・`NicolaKeytopYab`・`NicolaUsYab`・
`NicolaFYab`・`NicolaKb232Yab`・`NicolaKakuteiYab`の7コンポーネントに
`Permanent="yes"`を追加する。GUID・`NeverOverwrite="yes"`・
`Schedule="afterInstallExecute"`（ADR-099決定0が定めた3点）は一切変更しない。

**この変更が既存ファイル・レジストリに一切触れないことの確認**: `Permanent`
属性はコンポーネントの「アンインストール時に削除するかどうか」という
インストーラ側の扱いを変えるだけであり、ファイルの内容・配置先・レジストリの
`Name`/`Value`は変更しない。したがって既存ユーザーが現在持っている
`config.toml`/`.yab`のカスタマイズ内容は、このMSIをアップグレード適用しても
一切変化しない（ユーザー要求「移行時も含めて、修正済みの設定ファイルがそのまま
使えるように」を満たす）。

**効果**:
- アンインストール時、この7コンポーネントは削除されなくなる
  （[ProcessComponents action](https://learn.microsoft.com/en-us/windows/win32/msi/processcomponents-action)
  がextra system clientとして登録し、参照カウントが0にならない）。
- アップグレード時は従来どおりNeverOverwrite+GUID不変+Scheduleで保護される
  （変更なし）。

**round1 B1（レジストリKeyPath残留問題）への対応**: `Permanent`はコンポーネント
全体（KeyPathであるレジストリ値`HKCU\Software\awase\{ConfigFile,NicolaYab,...}`
も含む）に効くため、ユーザーが手動で`%LOCALAPPDATA%\awase`を削除した後に
再インストールすると、レジストリのKeyPathが残っている限り`NeverOverwrite`が
働き、ファイルが再配置されない（詳細は[178-opus-review-round1.md](178-opus-review-round1.md)
B1参照）。**この実害は決定2の自己修復ロジックが無効化する**——MSIがファイルを
配置しなくても、アプリが起動時に生成するため、「アプリが起動しない」という
round1 B1の最悪のシナリオは発生しない。

**round1 B2（既存ユーザーへの遡及効果）**: 未検証のまま決定に進まない。決定4の
実機検証で、既存インストール済み環境に対して新MSI（Permanent付き）をアップグレード
適用した場合に実際に効くかどうかを確認する。効かなかった場合、既存ユーザーは
このアップグレードを1回適用するだけでは保護されないが、決定2の自己修復ロジックは
Permanentの成否によらず機能するため、「ユーザーデータが失われて起動不能になる」
という最悪の事態は避けられる。

### 決定2: アプリ側に「存在しなければ埋め込み既定値から生成する」自己修復ロジックを追加する（メインの安全網）

コア`awase`クレートに、以下の性質を持つ関数を追加する。`ensure_user_data_exists()`
のような名前とする（v13の`ensure_user_data_present()`とは異なり、バックアップ・
復元・比較・能力トークンを一切持たない、はるかに単純な関数）。

```
pub fn ensure_config_exists(config_path: &Path) -> Result<()>;
pub fn ensure_layouts_exist(layouts_dir: &Path) -> Result<()>;
```

**`ensure_config_exists`**: `config_path`が存在しなければ、コア`awase`クレート
に埋め込んだ既定値（`include_str!("../config.toml")`、決定3参照）を
`crate::fs_atomic::write_atomic`で書き込む。既に存在する場合は何もしない
（内容の比較・バックアップ・上書きは一切行わない——これが v2〜v13 の複雑さの
大半を排除できる理由: 「既存ファイルとバックアップの整合を取る」という問題が
構造的に存在しない）。

**`ensure_layouts_exist`**: `layouts_dir`ディレクトリを（無ければ）作成し、
同梱6ファイル（`nicola.yab`・`nicola_keytop.yab`・`nicola_us.yab`・
`nicola_f.yab`・`nicola_kb232.yab`・`nicola_kakutei.yab`）のうち、
`layouts_dir`に**拡張子が`.yab`のファイルが1本も**存在しない場合にのみ、
6本全てを埋め込み既定値から生成する。1本でも存在すれば何もしない——
「ユーザーが同梱配列の一部を削除して整理した」状態を復活させないため
（v13決定5の救済条件と同じ考え方だが、判定はシンプルに「拡張子`.yab`の
ファイルが0本かどうか」のみ）。**「有効な」（パース可能かどうか）は判定
しない**（v14 opusレビューM3で訂正——v13が持っていた`KeyboardModel`全
バリアント試行のような妥当性検証は意図的に持たない。壊れた
（0バイト・破損等）`.yab`が1本でもあると、拡張子判定は通ってしまい
自己修復は発動しない。「解決されないこと」参照）。

**呼び出し位置と、生成先パスの決め方（2026-09-17実機検証で修正済み）**:
`awase.exe`は`find_config_path()`が`bail!`する前
（`crates/awase-windows/src/app/mod.rs`）、`.yab`は`LayoutEntry::scan_all`を
呼ぶ前（`crates/awase-windows/src/app/bootstrap.rs::init_engine_validated`）。
`awase-settings.exe`側も同様に`SettingsApp::new`内の2箇所に追加する。

**`ensure_layouts_exist`の生成先は、`resolve_relative()`（＝
`resolve_relative_to_exe`）の解決結果を使わず、`exe_dir.join(layouts_dir_raw)`
（`config.general.layouts_dir`の生文字列をexe隣に結合したもの、絶対パスなら
そのまま使われる）に固定すること。** 実機検証（dragonflyg4、1.20.7 MSI）で
このガードなしのバグを実際に踏んだ: `layout`ディレクトリを丸ごと削除した状態で
`awase.exe`を起動したところ、`config.toml`の自己修復は成功した（`ensure_config_exists`
は`exe_dir.join("config.toml")`固定だったため無事）が、`.yab`の自己修復は
発火せず、`%LOCALAPPDATA%\awase\layout`は生成されなかった。原因は
`resolve_relative(&config.general.layouts_dir)`を先に呼んでいたこと——
`resolve_relative_to_exe`は「exe隣に存在しなければCWD相対の裸パスへ
フォールバックする」ため、`layout`が存在しない時点でこの関数はCWD相対の
`"layout"`という文字列をそのまま返し、`ensure_layouts_exist`はその裸パスへ
（`awase.exe`のプロセスのカレントディレクトリ基準で）書き込もうとしていた。
`awase.log`にはエンジンが正常起動したログしか残らず、無警告のまま
`%LOCALAPPDATA%\awase\layout`だけが生成されない、という気づきにくい症状に
なった。修正: `ensure_default_layouts_exist(&config.general.layouts_dir)`
（生文字列を渡す）を**先に**呼び、生成先を`exe_dir`基準に固定したうえで、
その**後**に`resolve_relative()`で読み取り先を解決する（生成が成功して
いれば`resolve_relative`はexe隣を見つける）という順序に直した。これは
v13までround11・round12で繰り返し検出された「読み取り先/書き込み先」
問題（B2/B5/B6/B12→round11 M2→round12 M1）と同じ形の罠が、v14の
シンプルな設計でも再発したことを示す——「バックアップ・復元機構を無くせば
この種の罠も消える」わけではなく、**どんな設計でも「生成・書き込み先は
exe隣に明示的に固定し、存在依存の解決関数の結果を書き込み先として
使わない」という不変条件は必要**、という教訓として残す。

**CLI引数でconfigパスが明示されている場合**: `ensure_config_exists`は呼ばない
——ユーザーが明示的に指定したパスにアプリが勝手にファイルを生成するのは
意図しない副作用になる（v13チェックリスト#10と同じ理由）。

**開発ビルドの除外**: `exe_dir`（`current_exe().parent()`）の祖先に`target`が
含まれる場合は呼ばない。開発時はワークスペースルートの`config.toml`/`layout/`
（リポジトリ追跡対象）をそのまま使うため、生成ロジックが誤って新規ファイルを
作ると開発者の手元にリポジトリ外の複製が増える。

**失敗時の挙動**: 書き込みが失敗しても`panic`せず、ログ警告のうえ既存のエラー
ダイアログ経路（`find_config_path`の`bail!`／`show_no_layouts_dialog`）に
委ねる——生成ロジックは「うまくいけば起動できるようになる」追加の一手であり、
それ自体が失敗しても現状より状況を悪化させない。

### 決定3: 埋め込み既定値はビルド時にリポジトリのファイルから直接取り込む

`include_str!("../config.toml")` / `include_str!("../layout/nicola.yab")`
等、コア`awase`クレート（`src/`直下）からリポジトリルートの実ファイルを
直接参照する。`GeneralConfig::default()`のserializeを代用しない（構造体の
デフォルト実装は出荷時の`config.toml`と項目が食い違いうるため——実際
`GeneralConfig::default()`は`layouts_dir: "config"`だが出荷時は`"layout"`）。

CIでの一致検証は不要——決定2は「バックアップと埋め込み既定値のバイト一致」を
一切判定しないため（v13決定3が要求していたバイト一致検証・改行正規化・
BOM除去のCIステップは、その判定ロジック自体が無くなったので不要になる）。

### 決定4: 「完全に削除したい」場合の案内を更新する

MSI版・ZIP版ともに、完全削除の案内を「`%LOCALAPPDATA%\awase`フォルダと
`HKCU\Software\awase`レジストリキーの両方を削除してください」に更新する
（round1 M3対応——`Permanent`化されたコンポーネントのKeyPathレジストリ値は
フォルダ削除だけでは残るため、削除手順に明記しないとround1 B1のシナリオ
「フォルダだけ消して再インストールすると二度と配置されない」を踏む）。
案内先は`docs/index.html`・`docs/index.en.html`にアンインストール手順の
節を新設する（現状これらのファイルにアンインストール手順の記述が無いことを
round1 m3が指摘済み、英語版も対象）。ZIP版`scripts/uninstall.ps1 -Purge`は
変更不要（元々MSI管理外の操作のため）。

必須ではないが、`purge.ps1`のようなワンコマンドでフォルダとレジストリの両方を
削除するスクリプトをMSIに同梱する案は、実装コストが小さければ検討する
（round1 M4、未実施でも決定4の文書案内で最低限は足りる）。

### 決定5: 回帰テスト

- `crates/awase-windows/tests/wix_installer_guard.rs`に、対象7コンポーネントの
  `Permanent="yes"`存在を固定する回帰テストを追加する（既存の
  `config_file_and_nicola_yab_components_have_never_overwrite`と同型）。
  assertメッセージには「`Permanent`は消しても既に出荷済みの環境の挙動は
  変わらず、新規インストール環境だけが別挙動になる（環境間で挙動が分岐する）」
  という趣旨を明記する（round1 m4——将来「外しても実害が無さそうだから外す」
  という誤判断を防ぐため）。
- 同ファイルの`known_component_guids_are_unchanged`に`NicolaKb232Yab`・
  `NicolaKakuteiYab`が含まれていない既存の抜けを埋める（round1 M6）。
- コア`awase`クレートに`ensure_config_exists`/`ensure_layouts_exist`の
  単体テスト（ファイルが存在しない場合に生成されること、存在する場合は
  一切変更しないこと、`.yab`が1本でもあれば何もしないこと）を追加する。
  ホストターゲット（Linux、`cargo test --lib`）で実行可能。
- `--bug-report`等の非GUIサブコマンド経路では呼ばれないことを確認する
  （v13 round9 M1の教訓を維持）。

**実機確認（自動化不可）**: **1〜4すべて完了（2026-09-17、dragonflyg4、
awase-1.20.6-x64.msi＝Permanentなし旧版、awase-1.20.7-x64.msi＝Permanent
付きv14版・バグ入り、awase-1.20.8-x64.msi＝`.yab`自己修復バグ修正後）。**

1. **round1 B2の検証（既存ユーザーへの遡及効果）— 完了・成功**: 現行の
   （Permanentなし）1.20.6をクリーンインストールし`config.toml`を編集
   （`simultaneous_threshold_ms = 918`）→ `Permanent="yes"`付きの1.20.7へ
   アップグレード適用 → `msiexec /x`でアンインストール → `config.toml`が
   **残り、編集内容（918）も保持されていた**ことを確認した。round1 B2の
   「効かない側の根拠」は誤りで、「効く側の根拠」（アップグレード適用時点の
   `ProcessComponents`で新製品のPermanent属性に基づき登録される）が
   実機で正しいと確定した。
2. **新規インストールでの確認 — 完了・成功**: `%LOCALAPPDATA%\awase`と
   `HKCU\Software\awase`を完全に削除した状態から1.20.7を新規インストール →
   `config.toml`を編集（`555`）→ アンインストール → 残り、編集内容も
   保持されていたことを確認した。
3. **削除される想定のファイルが実際に削除されることの確認 — 完了・成功**:
   `awase.exe`・`data/ngram_hiragana.csv.gz`は手順1・2のアンインストール後
   いずれも削除されていた（`Test-Path`＝`False`）。
4. **完全削除→再インストールでの自己修復ロジックの確認 — 完了・成功**:
   初回（バグ発見時、1.20.7=バグ入りコード）: 1.20.7を再インストール
   （`config.toml`はNeverOverwriteによりPermanent化前の内容のまま残存＝
   round1 B1が懸念したシナリオを模した状態）→
   `%LOCALAPPDATA%\awase\config.toml`と`layout\`を手動削除 →
   `awase.exe`を起動 → `config.toml`は埋め込み既定値から正しく再生成
   された（自己修復ロジックの効果を確認）が、**`layout\`は生成されな
   かった**——これが上記「生成先パスの決め方」節のバグ発見経緯。
   修正後（1.20.8）の再検証: `%LOCALAPPDATA%\awase`と
   `HKCU\Software\awase`を完全削除 → 1.20.8をクリーンインストール →
   `config.toml`と`layout\nicola.yab`が存在することを確認 →
   `config.toml`と`layout\`を手動削除 → `awase.exe`を起動 → 4秒後、
   **`config.toml`と`layout\`の両方が生成され、`layout\`には同梱6
   ファイルすべて（`Get-ChildItem`のCount=6）が正しく揃っていることを
   確認した**。バグ修正が実機で有効であることが確定した。

## この設計で解決されること・されないこと

**解決されること**:
- MSIアンインストール後、`config.toml`/`layout/*.yab`は削除されなくなる
  （ZIP版と対称）。
- 既存ユーザーが移行時に持っているカスタマイズ済みの設定は、決定1が
  ファイル・レジストリに一切触れないため無傷で残る。
- 万一Permanentが効かない環境・レジストリが不整合な環境でも、決定2の
  自己修復ロジックによりアプリが起動不能になることはない。
- `layouts_dir`に拡張子`.yab`のファイルが1本も無い状態でアプリが起動不能に
  なる事態を、埋め込み既定値からの復旧で防ぐ（v13決定5と同じ効果を、より
  単純な条件で達成する）。
- v2〜v13が抱えていた「バックアップと実ファイルの整合」という問題は
  構造的に発生しなくなる（バックアップ機構自体が無いため）。ただし
  「生成・書き込み先を存在依存の解決関数に委ねない」という不変条件は
  引き続き必要であり、v14でも一度実機で踏んだ（上記「生成先パスの
  決め方」参照、修正済み）。
- **`Permanent="yes"`は不可逆な変更であり、その唯一の解毒剤である自己修復
  配線が壊れると復旧不能になる（v14 opusレビューBlocker B1）。この配線
  ——`load_config()`/`SettingsApp::new`からの呼び出し、生成順序（生成を
  `resolve_relative`より前に行う）、生成先が解決関数の結果に依存しない
  こと——は`crates/awase-windows/tests/architecture_guard.rs`の
  `adr178_self_heal_wiring`モジュール（5テスト）で機械的に固定されている。**

**解決されないこと**:
- `Permanent`は不可逆——一度出荷すると、将来`Permanent="no"`に戻しても
  既にインストール済みの環境には反映されない
  （[Microsoft Q&A](https://learn.microsoft.com/en-gb/answers/questions/1602667/recommended-way-to-uninstall-a-file-that-was-confi)）。
  「完全にawaseを削除したい」ユーザーは、決定4の手動削除案内に従う必要がある
  （MSI経由の自動化された完全削除手段は用意しない）。
- 将来、インストール先（perUser→perMachine、`%LOCALAPPDATA%`→`%APPDATA%`等）
  を変更する場合、Permanent化されたコンポーネントが同じKeyPath名前空間を
  占有し続けるため、新GUID・新KeyPathでの再設計が必要になる（round1 M6）。
- `msiexec /f`（修復）はユーザーデータの復旧経路にならない——`NeverOverwrite`
  によりKeyPathが存在する限り「健全」と判定されるため（round1 M7、
  Permanent以前からの既存挙動）。ただし決定2の自己修復ロジックは、
  `config.toml`自体が消えた場合には機能する。
- ユーザーが`layouts_dir`を明示的に別ディレクトリへ向けている場合、その
  ディレクトリへの自己修復は行わない対象外とする（拡張子`.yab`のファイルが
  1本もそのディレクトリに存在しない場合のみ発動するため、実質的にほぼ
  影響しない）。
- **壊れた（0バイト・破損・途中生成等）`.yab`が1本でもあると、自己修復は
  発動しない（v14 opusレビューM3）**——拡張子のみで判定し中身の妥当性
  （パース可能かどうか）は検証しないため。v13決定5が`KeyboardModel`全
  バリアント試行で対処しようとしていた問題であり、v14はこの複雑さを
  意図的に持たない。
- **`ensure_layouts_exist`の書き込みが途中（6本のうち数本）で失敗すると、
  以後「1本でもあれば何もしない」判定により残りは永久に生成されない
  （v14 opusレビューM5）**。起動不能にはならないが、`default_layout`が
  未生成の場合は毎回フォールバック通知モーダルが出る形で固定される。
- **アンインストール→再インストールで設定を初期状態に戻す、という従来の
  サポート定型句（「一度アンインストールして入れ直してください」）は
  この変更で成立しなくなる（v14 opusレビューM8）**——`config.toml`/
  `layout/*.yab`はPermanentで残り、`NeverOverwrite`で上書きもされない
  ため。初期化したい場合の正しい手順は「`%LOCALAPPDATA%\awase\config.toml`
  と`layout\`を削除してから`awase`を再起動する」であり、決定2の自己修復
  がこれを可能にする（実機検証4番で確認済み）。この手順のドキュメント
  反映は未解決事項参照。

## 未解決事項 / 次のアクション

1. opus-adversarial-consultによる新方針（v14）のレビュー: **完了**
   （[178-opus-review-v14.md](178-opus-review-v14.md)）。総合判定は
   「実装をやり直す必要はない、設計の骨格は正しい」。Blocker2件（B1:
   自己修復配線を守るテストが無かった、B2: `default_config()`の
   `layouts_dir = "config"`が`.yab`の誤生成先に使われうる）を検出、
   いずれも反映済み（B1: `architecture_guard.rs::adr178_self_heal_wiring`
   モジュール5テスト追加、B2: `awase-settings/src/main.rs`の
   `ensure_default_layouts_exist`呼び出しを`config_load_state ==
   Loaded`でゲート）。Major推奨5件（M1: `find_config_path`から副作用を
   分離、M2: `.yab`側にもCLI引数ゲート追加、M3: ADR本文「有効な.yab」
   表現の訂正、M5・M8: 「解決されないこと」節への追記）も反映済み。
   フォローアップ扱いだったM4・M6・M7も**反映済み**:
   - M4: `layout/`実ファイル・`EMBEDDED_LAYOUTS`・`wix/main.wxs`の
     LayoutFiles ComponentGroupの3集合が一致することを固定する
     `wix_installer_guard.rs::embedded_layouts_layout_dir_and_wix_components_are_in_sync`
     を追加。
   - M6: `is_dev_build()`相当の判定を`src/paths.rs::is_dev_build()`
     （内部で`resolve_relative_to`のワークスペースルート解決と同じ
     `find_target_ancestor`を共有）に一本化。`awase-windows`・
     `awase-settings`双方の`is_dev_build()`はこれへの委譲に変更。
   - M7: `scripts/uninstall.ps1 -Purge`に`HKCU:\Software\awase`
     （Permanent化された7コンポーネントのKeyPathレジストリ値）の削除を
     追加。
   Minor6件は未反映のまま（次のADR改訂または別PRで対応）。
2. 実機確認4項目すべて完了（上記参照、`.yab`自己修復バグの修正・再検証
   含む）。**ただしopusレビュー後のB1/B2/M1/M2/M4/M6/M7の全コード変更は
   未再検証**（コンパイル・単体テスト・ソーススキャンガードは確認済みだが、
   実機での動作確認はまだ）。
3. `ensure_config_exists`/`ensure_layouts_exist`の実装・呼び出し位置は
   完了（`awase.exe`・`awase-settings.exe`の両方、コンパイル・単体テスト・
   実機確認済み）。
4. `wix_installer_guard.rs`の`Permanent="yes"`固定テスト追加、GUID固定テストの
   抜け（`NicolaKb232Yab`・`NicolaKakuteiYab`）の解消は完了。
5. `docs/index.html`・`docs/index.en.html`へのドキュメント更新: opusレビュー
   Q3の判断により、決定4の「完全削除」案内より**M8の「初期化したい場合」
   案内を優先する**（実害頻度が高いため）。両方とも未実施。
6. `purge.ps1`同梱: opusレビューQ3の判断により**不要**。代わりに
   `scripts/uninstall.ps1 -Purge`へ`Remove-Item HKCU:\Software\awase
   -Recurse`相当の1行を追加する方が低コストで同じ効果（M7、未実施）。
7. v2〜v13のレビュー記録（`178-opus-review-round2.md`〜`round13.md`）は
   歴史的記録として残す（削除しない）。index.mdの記述は「v1〜v13の経緯」を
   反映するよう更新する。
