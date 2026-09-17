---
id: ADR-178
title: |-
  MSIアンインストール時のユーザーデータ喪失を自己修復（バックアップ+復元）で無害化する
status: |-
  **起草中（v2）。opus-adversarial-consult round1で採用案（Permanent="yes")を
  Blocker2件で却下、round2としてMSI非依存の自己修復方式へ全面差し替え。
  レビューはこれから。**
related_adr:
  - "ADR-099"
  - "ADR-177"
---

# ADR-178: MSIアンインストール時のユーザーデータ喪失を自己修復（バックアップ+復元）で無害化する

## ステータス

**起草中v2（2026-09-17）。opus-adversarial-consultによるレビューはこれから。**

## コンテキスト

[ADR-177](177-msi-restart-manager-graceful-shutdown.md)の実機検証で、MSIの
アンインストール（`msiexec /x`）が`%LOCALAPPDATA%\awase\config.toml`/
`layout/*.yab`を削除することが判明した。これは[ADR-099](099-config-preservation-on-upgrade.md)
決定1がZIP版（`scripts/uninstall.ps1`）に定めた「既定では残す、完全消去は
`-Purge`明示フラグが必要」という方針と非対称であり、ユーザーから
「ユーザーデータ削除するのおかしいね。残してほしい」との明示的な要望があった。

### v1（採用案: `Permanent="yes"`）が却下された経緯

当初、`NeverOverwrite="yes"`が付いている7コンポーネント（`ConfigFile`・
`NicolaYab`等、`wix/main.wxs`）にWiXの`Permanent="yes"`属性を追加する案を
起草したが、opus-adversarial-consult round1でBlocker 2件により却下された:

- **B1**: `Permanent`はコンポーネント**全体**（ファイル＋KeyPathレジストリ値）
  に効く。perUserインストールの制約（ICE38）でKeyPathは必ずレジストリ値
  になるため、アンインストール後も`HKCU\Software\awase\ConfigFile`等7つの
  レジストリ値が残る。この状態で将来ユーザーが手動で`%LOCALAPPDATA%\awase`
  だけを削除して再インストールすると、`NeverOverwrite`が「KeyPathが既に
  存在する＝インストール済み」と誤判定し、**`config.toml`も6本の`.yab`も
  一切配置されず、awase.exeが起動しなくなる**。しかも`Permanent`は
  実質不可逆（一度出荷すると次バージョンで戻しても既存環境には反映
  されない）ため、この不具合は恒久的に残る。
- **B2**: 既にPermanent無しでインストール済みの既存ユーザーに、この
  変更が後から効くかどうか自体が未検証だった。

さらに検討した結果、代替として「MSIコンポーネント自体を`wix/main.wxs`
から削除する」案（当初「選択肢D」と呼んだもの）も、Windows Installerの
一般的挙動として**次のメジャーアップグレードで新バージョンが参照しなく
なったコンポーネントは自動的に削除される**ため、既存ユーザーのデータが
一斉に失われるという別のBlocker級の問題を持つことが判明し、不採用と
した（Microsoft公式ブログのタイトルもずばり「removal of a component
from a feature is not supported」）。

### 方針転換: MSI（`wix/main.wxs`）を一切変更しない

上記の検討を経て、**`wix/main.wxs`のコンポーネント構成は一切変更しない**
（メジャーアップグレード時の既存の保護＝ADR-099決定0をそのまま維持し、
新たなリスクを持ち込まない）方針とし、代わりに**アプリ自身
（`awase.exe`/`awase-settings.exe`）が設定データの保存・復元責任を持つ**
自己修復方式に転換する。

## 決定

### 決定1: 保存の都度、MSI管理外のディレクトリへ自動バックアップする

`config.toml`/`layout/*.yab`が実際に書き換えられるタイミングで、
`%LOCALAPPDATA%\awase-backup\`（`awase`ディレクトリの兄弟、MSIの
コンポーネント管理下に一切無い）へ自動的にコピーする。

- `config.toml`: `AppConfig::save()`（`src/config.rs:890`）の書き込み
  成功後にコピーする。呼び出し元は`awase-settings`（`main.rs:818`の
  `clone.save(&config_path)`）と`awase.exe`（`tray.rs:1041`の
  `save_auto_start_config` → `AppConfig::save_auto_start`）の両方が
  あるため、バックアップ処理は`AppConfig::save()`自身、または
  両呼び出し元が共通して通る箇所に実装し、重複実装を避ける。
- `layout/*.yab`: `layout_write_to_path()`
  （`crates/awase-settings/src/main.rs:1598`、配列編集タブの保存処理）
  の書き込み成功後にコピーする。

バックアップはベストエフォート（失敗してもログに警告を出すのみで、
本処理〈設定の保存〉の成否には影響させない）。

### 決定2: 起動時、ファイルが存在しなければバックアップまたは埋め込み既定値から復元する

`config.toml`が存在しない場合:

1. `%LOCALAPPDATA%\awase-backup\config.toml`が存在すれば、そこから
   コピーして復元する（ユーザーが編集した内容を実質的に保持する）。
2. バックアップも無ければ、埋め込み既定値（`include_str!`でビルド時に
   `config.toml`を取り込んだもの）から生成する。

`layout/*.yab`（6ファイル: `nicola.yab`・`nicola_keytop.yab`・
`nicola_us.yab`・`nicola_f.yab`・`nicola_kb232.yab`・`nicola_kakutei.yab`）
も同様に、個別ファイル単位でバックアップ→埋め込み既定値の順に復元する
（`layouts_dir`ディレクトリ自体が存在しなければ`create_dir_all`で作成）。

実装箇所（同一ロジックが複数箇所に重複しないよう、共通ヘルパーへの
切り出しを実装時に検討する）:

- `crates/awase-windows/src/app/mod.rs::find_config_path()`
  （現状は存在しなければ`bail!`するのみ、ここに復元ロジックを追加）
- `crates/awase-settings/src/main.rs::find_config_path()`
  （同型ロジック、awase.exe側と同じ変更を加える）
- `crates/awase-windows/src/app/bootstrap.rs`（237行目付近、
  `layouts_dir`をディレクトリスキャンして`*.yab`を読み込む処理。
  現状はディレクトリが無い/空なら`show_no_layouts_dialog`で
  エラーダイアログを出して終了するため、スキャンの**前**に
  復元ロジックを挟む）

### 決定3: 埋め込み既定値はビルド時にリポジトリのファイルから直接取り込む

`include_str!("../../config.toml")` / `include_str!("../../layout/nicola.yab")`
のように、リポジトリルートの実ファイルを直接参照する（値をコピーして
二重管理しない）。これにより、リポジトリの既定値を更新すれば埋め込み
既定値も自動的に追従する。

### 決定4: 「完全に削除したい」場合の案内を更新する

MSI版・ZIP版ともに、完全削除の案内を「`%LOCALAPPDATA%\awase`と
`%LOCALAPPDATA%\awase-backup`の両方を削除してください」に更新する
（ZIP版`scripts/uninstall.ps1 -Purge`の対象にも`awase-backup`を追加する）。

### 決定5: `wix/main.wxs`は変更しない

決定1〜4はいずれもアプリ側（Rustコード）の変更のみで完結し、MSIの
コンポーネント構成・`Permanent`属性・GUID等には一切触れない。ADR-099
決定0が担うアップグレード時の保護は現状のまま維持される。

## この設計で解決されること・されないこと

**解決されること**:
- MSIアンインストール→再インストール後、`config.toml`/`layout/*.yab`が
  MSI側の挙動によって物理的に削除されても、次回起動時にバックアップ
  から実質的に復元される。
- v1で問題になった「レジストリKeyPathだけ残って再インストールで
  ファイルが配置されない」というシナリオ自体が発生しない
  （`wix/main.wxs`を変更しないため、`NeverOverwrite`とPermanentの
  衝突が起きようがない）。
- 何らかの理由で`config.toml`/`layout/*.yab`がファイルシステムから
  消えた場合（ユーザーの誤削除、破損等）、アプリが起動不能になる
  という最悪の事態を防げる（ADR-099 F4が扱った「load失敗」とは別の、
  「そもそもファイルが無い」ケースへの防御）。

**解決されないこと**:
- MSIのアンインストール自体は引き続き`config.toml`/`layout/*.yab`を
  削除する（`wix/main.wxs`を変更しないため）。「MSIレベルで保護する」
  というADR-099決定1がZIP版に対して実現した体験そのものではなく、
  「消えても実害が出ないようにアプリ側で補う」という間接的な解決。
- バックアップ自体は`%LOCALAPPDATA%`配下にあるため、ユーザーがOSの
  ユーザープロファイルごと初期化する場合はバックアップも失われる
  （これはZIP版の`-Purge`と同じ扱いであり、後退ではない）。
- バックアップと本体のタイミングがずれるケース（保存直後にクラッシュ
  した等）では、最新の編集内容が反映されない可能性がある。

## 未解決事項 / 次のアクション

1. opus-adversarial-consultによるレビュー。
2. `AppConfig::save()`/`layout_write_to_path()`双方から呼ばれる共通
   バックアップヘルパーの配置（プラットフォーム非依存の`src/config.rs`
   に置くか、`awase-windows`/`awase-settings`それぞれに置くか）の設計。
3. `awase-backup`ディレクトリの権限・エラーハンドリング（書き込み
   失敗時のログレベル、繰り返し失敗した場合の扱い）。
4. 既存の`ConfigLoadState`/`classify_load_error`（ADR-099決定4）との
   統合方法——「ファイルが存在しない」は現状`NotFound`分類だが、
   復元ロジックが割り込むことで、ユーザーから見た挙動（警告ダイアログ
   の有無等）がどう変わるかの整理。
5. 回帰テスト（`crates/awase-windows/tests/`または`src/config.rs`内の
   ユニットテスト）: バックアップ→削除→復元のラウンドトリップ検証。
6. 実機検証: MSIアンインストール→再インストール→復元されることの確認。
