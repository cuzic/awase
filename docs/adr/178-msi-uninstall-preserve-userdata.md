---
id: ADR-178
title: |-
  MSIアンインストール時にユーザーデータ(config.toml/layout/)を残す
status: |-
  **起草中。opus-adversarial-consult未実施。**
related_adr:
  - "ADR-099"
  - "ADR-177"
---

# ADR-178: MSIアンインストール時にユーザーデータ(config.toml/layout/)を残す

## ステータス

**起草中（2026-09-17）。opus-adversarial-consultによるレビューはこれから。**

## コンテキスト

[ADR-177](177-msi-restart-manager-graceful-shutdown.md)の実機検証で、MSIの
アンインストール（`msiexec /x`、ARPまたはスタートメニューの「Uninstall
awase」ショートカット経由）が`%LOCALAPPDATA%\awase\config.toml`/
`layout/*.yab`を削除することが判明した。これは[ADR-099](099-config-preservation-on-upgrade.md)
決定1がZIP版（`scripts/uninstall.ps1`）に定めた「既定では
`config.toml`・`layout/`は残す、完全消去は`-Purge`明示フラグが必要」
という方針と非対称であり、ユーザーから見ると以下のように現状の
MSI版だけが不利になっている:

| 経路 | アンインストール既定時の`config.toml`/`layout/` |
| --- | --- |
| ZIP（`scripts/uninstall.ps1`） | 残す（消すには`-Purge`明示フラグが必要） |
| MSI（`msiexec /x`） | **消える**（現状） |

不具合対応でよくある案内「一度アンインストールして入れ直してください」を
MSIユーザーが実行すると、`config.toml`の全設定と配列編集タブで作り込んだ
`layout/*.yab`が警告なく消える。これはADR-099を起票させた元のユーザー
報告「バージョンアップすると既存の設定が失われる」と体感上同じ症状になる。

ユーザーから「ユーザーデータ削除するのおかしいね。残してほしい」との
明示的な要望があり、本ADRで対応方針を検討する。

## 検討した選択肢

### 選択肢A: 該当コンポーネントに`Permanent="yes"`を追加する（採用案）

WiXの`Component/@Permanent`属性（MSIの`msidbComponentAttributesPermanent`
フラグ）を、既に`NeverOverwrite="yes"`が付いている7コンポーネント
（`ConfigFile`・`NicolaYab`・`NicolaKeytopYab`・`NicolaUsYab`・
`NicolaFYab`・`NicolaKb232Yab`・`NicolaKakuteiYab`）に追加する。
これらは全て「ユーザーが配列編集タブ等でその場編集しうるデータ」という
同じ分類に属しており、`NeverOverwrite`（上書きされないファイル）と
`Permanent`（アンインストールで削除されないファイル）は保護したい対象が
一致する。

**利点**: WiX標準機能で完結し、カスタムアクションが不要。実装は
属性追加のみで小さい。

**重大なトレードオフ（[Microsoft Q&A](https://learn.microsoft.com/en-us/answers/questions/1602667/recommended-way-to-uninstall-a-file-that-was-confi)・[Microsoft公式ドキュメント](https://learn.microsoft.com/en-us/windows/win32/msi/installing-permanent-components-files-fonts-registry-keys)で確認）**:
`Permanent`フラグは一度そのコンポーネントがインストールされると、
その設定が`HKEY_CURRENT_USER\...\Installer\UserData\...\Components\
<Component GUID>`（perUserインストールの場合）に記録される。**将来
MSI側で`Permanent="no"`に戻しても、既にこの設定でインストール済みの
ユーザー環境には反映されない**（レジストリに記録された状態が優先される）。
つまり、この変更は実質的に不可逆——一度出荷すると、その後どのバージョン
をインストールしても、対象ファイルはアンインストールで削除されなくなる。
「完全にawaseを削除したい」ユーザー（PC譲渡前、ディスク容量整理等）は
手動で`%LOCALAPPDATA%\awase`を削除する以外の手段が無くなる
（ZIP版の`uninstall.ps1 -Purge`に相当する自動化された完全削除手段は、
MSI版には用意しない）。

### 選択肢B: カスタムアクションで退避→通常アンインストール→復元

アンインストール開始時にカスタムアクションで対象ファイルを一時退避し、
標準のアンインストール処理を実行させた後、別のカスタムアクションで
退避先から`%LOCALAPPDATA%\awase`へ復元する。

**不採用の理由**: カスタムアクションのタイミング制御（`InstallExecuteSequence`
での正確な位置、ロールバック時の扱い）が複雑で、退避・復元自体が
失敗した場合にデータを完全に失うリスクを新たに持ち込む。選択肢Aより
複雑な実装で得られる利点（可逆性）が、実際には運用上ほぼ使われない
見込み（将来「やはりMSIでも完全削除をデフォルトにしたい」と判断する
可能性は低い——ZIP版が既にこの方針を採っており、対称性を取るのが目的）。

### 選択肢C: 現状維持、ドキュメントで案内するのみ

「MSI版でアンインストールする場合、設定を残したいなら事前に
`%LOCALAPPDATA%\awase\config.toml`/`layout/`をバックアップしてください」
とドキュメントに書くだけで済ませる。

**不採用の理由**: ユーザーから明示的に「残してほしい」という要望があり、
ZIP版と同じ体験をMSI版でも提供できる技術的手段（選択肢A）が存在するため、
案内だけで済ませる理由がない。

## 決定

**選択肢Aを採用する。** `ConfigFile`・`NicolaYab`・`NicolaKeytopYab`・
`NicolaUsYab`・`NicolaFYab`・`NicolaKb232Yab`・`NicolaKakuteiYab`の
7コンポーネントに`Permanent="yes"`を追加する。

### 影響範囲の確認

- `NicolaYab`コンポーネントが持つ`RemoveFolder Id="RemoveLayoutDir"
  Directory="LayoutDir" On="uninstall"`は、コンポーネントがPermanentに
  なることで実行されなくなる可能性が高い（要実機確認）。これは意図通り
  ——中身（`*.yab`ファイル）が残るなら、ディレクトリ自体も残ってよい。
- `MainExe`の`RemoveFolder Id="RemoveInstallDir" Directory="INSTALLDIR"
  On="uninstall"`はPermanent化しないため引き続き動作するが、
  `config.toml`や`layout/`が残っている限り`INSTALLDIR`は空にならず、
  `RemoveFolder`の「ディレクトリが空なら削除」という仕様上、どのみち
  フォルダごと残る。これも意図通り。
- `NgramData`（`data/ngram_hiragana.csv.gz`）・`AppShortcut`
  （スタートメニューショートカット）はプログラム資産のため対象外の
  まま。アンインストール時に削除される。
- メジャーアップグレード時（`RemoveExistingProducts`）の挙動は、既に
  `NeverOverwrite="yes"`+GUID不変+`Schedule="afterInstallExecute"`
  （ADR-099決定0、ADR-177で実機確認済み）で保護されているため、
  `Permanent="yes"`の追加による影響はない（アップグレード時は元々
  該当ファイルへの上書き・削除が発生していなかった）。

### ドキュメントへの追記

「完全にawaseを削除したい場合は、アンインストール後に手動で
`%LOCALAPPDATA%\awase`フォルダを削除してください」という一文を
`docs/index.html`（アンインストール手順を案内している箇所）に追記する。

## テスト方針

[fix-requires-evidence](../../.claude/rules/fix-requires-evidence.md)に
従い、`crates/awase-windows/tests/wix_installer_guard.rs`に、対象7
コンポーネントの`Permanent="yes"`存在を固定する回帰テストを追加する
（既存の`config_file_and_nicola_yab_components_have_never_overwrite`と
同型のテキスト走査テスト）。

実機検証（clipwire経由、dragonflyg4）:

1. MSIをクリーンインストールし、`config.toml`/`layout/nicola_keytop.yab`
   を編集。
2. `msiexec /x`でアンインストールし、`%LOCALAPPDATA%\awase\config.toml`/
   `layout/`が削除されずに残ることを確認。
3. 削除される想定のファイル（`awase.exe`・`awase-settings.exe`・
   `data/ngram_hiragana.csv.gz`・スタートメニューショートカット）が
   実際に削除されることを確認（Permanent化の副作用で意図せず残らないか
   の確認を兼ねる）。
4. 同じMSIを再インストールし、`NeverOverwrite`により手順1で編集した
   内容が保持されたまま起動することを確認（Permanentコンポーネントの
   再インストール時の扱いに意図しない副作用が無いかの確認）。

## 未解決事項 / 次のアクション

1. opus-adversarial-consultによるレビュー（不可逆性のトレードオフが
   本当に許容できるか、他の見落としが無いか）。
2. 実機検証（上記テスト方針参照）。
3. `crates/awase-windows/tests/wix_installer_guard.rs`への回帰テスト追加。
