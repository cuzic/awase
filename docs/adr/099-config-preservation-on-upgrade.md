# ADR-099: バージョンアップ時の設定消失を防ぐ — MSI/ZIP インストーラーのユーザーデータ分離と load-failure セーフティネット

## ステータス

**実装済み（2026-08-21）。Windows 実機でのアップグレード検証は未実施。**
Opus premortem round1（2026-08-21）で6件の must-fix・7件の should-fix を
受領し、本文へ反映済み。**round1 で「MSI 経路は保護されている」という
当初の前提が誤りと判明し、根本的に書き直した**（旧 決定1〜5 は 決定0〜8
へ再編）。続く round2（2026-08-21）で実コード裏取りに基づく4件の
must-fix・8件の should-fix を受領し、決定0拡張（`NicolaYab` への
`NeverOverwrite` 追加）・決定4の分類ルール精密化・決定8（guard test）新設等
で反映。round3（2026-08-21）で round2 の must-fix 4件（MF-1〜MF-4）が意図
通り閉じられていることを実コード付きで確認し、「実装着手可」の最終判定を
受領。round3 が見つけた4件の非ブロッキングな記述不整合も反映済み。

決定0〜4・6〜8を実装し（決定5は不採用のまま）、Linux 上の `cargo test`
（root/`awase-windows`/`awase-settings` 合計800件超、新規追加は
`wix_installer_guard.rs` 3件・`config::tests` 追加10件・`awase-settings`
追加7件）、`cargo fmt --check`、CI 相当の `cargo clippy --lib`、
`cargo xwin check/clippy/build --tests -p awase-windows -p awase-settings`
（実際の Windows ターゲットへのクロスコンパイル、全てクリーン）を確認
済み。実装完了後の**コードレビュー（Opus）で2件の確定バグ（C1: 決定8の
guard test がタグ範囲外まで文字列検索していたため自らが追加した説明
コメントの文言と一致して検知不能になっていた／C2: `.bak` バックアップの
コピーに失敗しても警告ログのみで保存を続行し原本を上書きしうる余地が
あった）と複数の改善提案が見つかり、全て反映済み**（詳細は各決定の本文・
`docs/known-bugs.md` BUG-71参照）。**PR作成後の8観点コードレビュー
（correctness×3・cleanup×3・altitude・conventions）で追加の確定バグ2件
（`apply_confirmed()` のUIスレッド最大200msブロック／`cancel()` の
`show_dangerous_save_confirm` リセット漏れ）・reuse指摘1件
（`fs_atomic::write_atomic` への切り出しと `gji_charset_write.rs` との
共有）を検出・修正済み**（詳細は決定3節・`docs/known-bugs.md` BUG-71
「コードレビュー追加修正」参照）。

## コンテキスト

### 発端

ユーザーから「バージョンアップすると既存の設定が失われる」という不具合報告が
あった。調査エージェント（Explore）による全体調査、続く直接検証（該当ファイルの
読み込み）、さらに Opus premortem round1 とその指摘の裏取り（WiX ドキュメントの
実地確認・関連コードの再読）を経て、原因を4箇所（うち1箇所は当初「保護済み」と
誤認していた）に特定した。`docs/known-bugs.md`・`docs/experiments.md` にはこの
症状の既存記録は無く、今回が初報告と見られる。

### 保存場所の全体像

`config.toml` は `src/paths.rs::resolve_relative_to_exe` により実行ファイルと
同じディレクトリから解決される。Windows（MSI/ZIP 双方）では
`%LOCALAPPDATA%\awase\config.toml` に固定され、パス自体にバージョン番号は
含まれない（`wix/main.wxs` の `INSTALLDIR` は固定名）。macOS/Linux は
インストーラー自体が未整備（`release.yml` は Windows のみビルド）で実質未
リリース状態のため、本 ADR は Windows の2配布経路（MSI / ZIP）に絞る。

### 確定した事実（F1〜F5）

- **F1（最重要・round1 で新規発見）: MSI 経路も保護されていない。
  `<MajorUpgrade>` に `Schedule` 属性が無いため既定値 `afterInstallValidate`
  が適用され、新バージョンのファイルをインストールする前に旧バージョンを
  完全アンインストールしてしまう。**
  `wix/main.wxs:10`:
  ```xml
  <MajorUpgrade DowngradeErrorMessage="A newer version is already installed." />
  ```
  WiX の `MajorUpgrade` 要素は `Schedule` 省略時 `afterInstallValidate` が
  既定値で、これは `RemoveExistingProducts`（旧製品の完全アンインストール）を
  新製品のファイルインストールより**前**にスケジュールする
  （[FireGiant MajorUpgrade element docs](https://docs.firegiant.com/wix/schema/wxs/majorupgrade/)）。
  `ConfigFile` コンポーネント（`main.wxs:50-54`）は `NeverOverwrite="yes"`
  だが、これは「**既に存在するファイルを上書きしない**」制御であり、
  旧製品のアンインストールで `config.toml` 自体が一度削除されてしまえば
  何も保護しない。加えて `ConfigFile` の `KeyPath` はファイルではなく
  `HKCU\Software\awase\ConfigFile` レジストリ値（`main.wxs:52-53`、
  perUser インストールで `File` を `KeyPath` にできない ICE38 対策として
  そう設計されている）であり、`NeverOverwrite` の判定対象はこのレジストリ
  値の有無であって `config.toml` の中身の保護ではない。旧製品アンインストール
  でこのレジストリ値も消えるため、保護は実質的に機能しない。
  同様に `layout/nicola.yab`（`NicolaYab` コンポーネント）・
  `data/ngram_hiragana.csv.gz`（`NgramData` コンポーネント）も
  `RemoveFolder On="uninstall"` を伴う通常コンポーネントであり、
  旧製品アンインストール時に丸ごと削除される。
  **`Product Id="*"` により毎ビルドで ProductCode が変わり、`awase` は
  リリースのたびに必ずこの「メジャーアップグレード」経路を通る**ため、
  MSI でインストールした全ユーザーが原理的に影響を受ける。ZIP 版より
  MSI 版の方が一般ユーザー向けの主配布経路である可能性が高く（README/
  リリースノートでの案内順を要確認）、**本 ADR で最優先に修正すべきは
  F1 である**。

- **F2: `scripts/uninstall.ps1` が `%LOCALAPPDATA%\awase` を無条件かつ再帰的に
  削除する。**
  ```powershell
  if (Test-Path $installDir) {
      Remove-Item -Recurse -Force $installDir
  }
  ```
  `config.toml` だけでなく `layout/*.yab`（カスタム配列）・`data/*` も対象。
  ZIP 配布には「アップグレード専用」の手順が用意されておらず、
  `install.ps1`/`uninstall.ps1` という対の名前から見て「アップグレード＝
  アンインストール→インストール」という手順をユーザーが自己判断で踏む
  可能性が高い。この手順を踏んだ場合、`install.ps1` が実行される時点で
  既に `config.toml` は存在しないため、新しいデフォルト設定で再生成される。

- **F3: `scripts/install.ps1` が `config.toml` と `layout/`・`data/` を
  非対称に扱う。**
  ```powershell
  # config.toml: 既存なら上書きしない
  if (-not (Test-Path "$installDir\config.toml")) {
      Copy-Item "config.toml" "$installDir\" -Force
  }
  # layout/data: 常に無条件上書き
  Copy-Item "layout\*" "$installDir\layout\" -Force
  Copy-Item "data\*" "$installDir\data\" -Force
  ```
  **訂正（round1 指摘）**: 当初「`awase-yab-editor` は未実装なのでユーザーは
  `nicola.yab` を手編集する以外にカスタマイズ手段が無い」としていたが誤り。
  実際には `awase-settings` に「配列編集」タブとして**統合済み**
  （`crates/awase-settings/src/main.rs:56` コメント、`layout_do_save`/
  `layout_write_to_path`、同 546-565 行）で、`ensure_layout_loaded` は
  `config.general.layouts_dir` 配下の既定 `.yab`（＝同梱の `nicola.yab`
  そのもの）を開いて編集・上書き保存する GUI 導線が既にある。したがって
  F3 による被害は「手編集した稀なユーザー」限定ではなく、**GUI の配列編集
  機能を一度でも使った全ユーザー**が対象になる。
  一方 `data/ngram_hiragana.csv.gz` はユーザーが編集する手段を持たない
  **プログラム資産**であり、`layout/` とは性質が異なる（決定2で区別する）。

- **F4（最も重大・インストーラーと無関係に発生しうる）: `awase-settings`
  が `AppConfig::load()` 失敗時にデフォルト設定へ静かにフォールバックし、
  その状態で「適用」を押すとデフォルトが `config.toml` に永続化される。**
  `crates/awase-settings/src/main.rs:320-326`:
  ```rust
  let config = match awase::config::AppConfig::load(&config_path) {
      Ok(cfg) => cfg,
      Err(e) => {
          log::warn!("Config load failed: {e}, using defaults");
          default_config()
      }
  };
  ```
  この `config`（読み込み失敗時は事実上 `default_config()`）がそのまま
  `SettingsApp.config` になり、`apply()`（同 379 行）が無条件に
  `self.config.save(&self.config_path)` を呼ぶ。ユーザーには `log::warn!`
  （通常は不可視）しか通知されず、GUI 上で「今デフォルト値を見ている」
  ことを示す表示は無い。設定タブを一つでも開いて「適用」を押せば、
  読み込めなかった実ファイルの中身は完全に上書きされ復元不能になる。
  `AppConfig::save()`（`src/config.rs:551-556`）は `std::fs::write` による
  直接上書きで、一時ファイル経由のアトミック書き込みでもバックアップ
  取得でもない。
  **訂正（round1 指摘）**: 当初「`config.rs` は `#[serde(default)]` を
  **全フィールドに**適用済み」としていたが誤り。`AppConfig` の
  `general: GeneralConfig`（`src/config.rs:520`）にはこの属性が**無い**
  （`keys`/`app_overrides`/`keymaps`/`post_bypass` のみ付いている、同
  521-529 行）。`[general]` セクションを欠く（または壊れた）
  `config.toml` は即座に parse error となり F4 の引き金になる。
  読み込み失敗のもう一つの引き金は「壊れた TOML 構文」（書き込み中の
  クラッシュ・ディスクフルによる不完全書き込みなど）。

- **F5（round1 指摘・本 ADR では未着手のまま記録のみ）: `src/paths.rs`
  の CWD フォールバックが「幻の保存先」を生みうる。**
  `resolve_relative_to`（`src/paths.rs:33-55`）は、exe 隣にもワークスペース
  ルート相対にも対象ファイルが存在しなければ、**`PathBuf::from(path)` を
  そのまま返す**（同 54 行）。すなわち相対パス `"config.toml"` が
  CWD 相対のまま `AppConfig::load`/`save` に渡る。F1/F2 で `config.toml`
  が消えた直後や、`awase-settings.exe` をショートカット以外の経路
  （エクスプローラーでの直接ダブルクリック以外、任意のフォルダからの
  起動等）で実行した場合、意図しない CWD に新規 `config.toml` が
  作られたり、既存の実ファイルとは異なる場所を読み書きし続ける懸念が
  ある。2026-07-19 に実際に確認された「`awase.exe`/`awase-settings.exe`
  が互いに異なる `config.toml` を読み書きする」既知の混乱
  （`crates/awase-settings/src/main.rs:3021` 付近のコメント）と同根。
  本 ADR は F1〜F4 の修正を優先し、F5 の根治（パス解決ロジックの再設計）
  は別 ADR に切り出す。決定7で診断性のみ最小限強化する。

## 決定

### 決定0（round1 追加・最優先）: `wix/main.wxs` の `MajorUpgrade` に
`Schedule="afterInstallExecute"` を指定し、`NicolaYab` コンポーネントにも
`NeverOverwrite="yes"` を付与する

```xml
<MajorUpgrade DowngradeErrorMessage="A newer version is already installed."
              Schedule="afterInstallExecute" />
```

`afterInstallExecute` は「新バージョンのファイルを先にインストールし、
その後に旧バージョンの（新バージョンに引き継がれなかった）コンポーネントだけを
削除する」スケジューリング。`ConfigFile`/`NicolaYab`/`NgramData` は
コンポーネント GUID が新旧バージョン間で不変（ADR ドラフト時点の履歴確認では
一度も変更されていない）ため、新バージョンのインストール時点でこれらの
コンポーネントは「新製品からも参照される」状態になり、続く旧製品の削除処理は
その参照を検知してファイルを消さない
（MSI Component Table の `msidbComponentAttributesNeverOverwrite` の仕様上、
`NeverOverwrite` でファイル配置がスキップされても新製品はそのコンポーネントの
クライアントとして登録される、というのが本決定の論拠。round2 指摘 SF-7）。
これにより `NeverOverwrite="yes"` が本来意図していた「ユーザーの変更を
上書きしない」効果が、**旧製品アンインストールによる削除**に対して初めて
成立する
（[Preparing a major upgrade — WiX 3.6](https://www.oreilly.com/library/view/wix-36-a/9781782160427/ch13s02.html)、
`afterInstallExecute` は "existing files and services are preserved" と明記）。

**round2 指摘 MF-1: `afterInstallExecute` が防ぐのは「旧製品アンインストール
による削除」だけであり、「新製品の `InstallFiles` による上書き」は防がない。**
`wix/main.wxs:59` の `NicolaYab` コンポーネントには元々 `NeverOverwrite` が
付いていない（`ConfigFile` にのみ付与されていた）。F3 訂正により
`layout/nicola.yab` は GUI（`awase-settings` の配列編集タブ）がその場で
上書き保存する実質的なユーザーデータと確定したため、決定0では
`ConfigFile` と同様に `NicolaYab` にも `NeverOverwrite="yes"` を追加する:

```xml
<Component Id="NicolaYab" Guid="9690990E-..." NeverOverwrite="yes">
```

`data/ngram_hiragana.csv.gz`（`NgramData`）はユーザー編集不可のプログラム
資産（決定2参照）なので、`NeverOverwrite` は付けず従来通り MSI の標準的な
バージョン管理下の上書きに任せる。

**この変更単体で F1 は解消するが、コンポーネント GUID を今後も変更しないことが
前提条件になる**。GUID を変更すると Windows Installer はそのコンポーネントを
「別物」とみなし、以後は通常の新規インストール（＝ファイル作成）として扱われて
しまう。この前提はコメント1行では歯止めにならない（round2 指摘 MF-3）ため、
決定8（後述）で機械的なテストを追加する。

### 決定1: `scripts/uninstall.ps1` のデフォルト動作からユーザーデータ削除を除く

デフォルトでは `awase.exe`/`awase-settings.exe`・スタートメニューショート
カット・レジストリ起動エントリのみを削除し、`config.toml`・`layout/`・
`data/` は残す。完全消去したい場合のみ明示フラグを要求する:

```powershell
# awase uninstaller script
param([switch]$Purge)
$ErrorActionPreference = "Stop"
...
if ($Purge) {
    Remove-Item -Recurse -Force $installDir
} else {
    Remove-Item "$installDir\awase.exe" -ErrorAction SilentlyContinue
    Remove-Item "$installDir\awase-settings.exe" -ErrorAction SilentlyContinue
    Remove-Item "$installDir\data" -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item "$installDir\awase.log" -ErrorAction SilentlyContinue
    Write-Host "設定・配列ファイルは保持されました（完全削除は -Purge を指定）"
}
```

**round2 指摘 SF-8**: PowerShell の `param()` はスクリプト内の最初の実行文
（コメント以外）である必要がある。現行 `uninstall.ps1:2` の
`$ErrorActionPreference = "Stop"` より前に `param()` を置く必要がある
（上記スニペットは順序を修正済み）。

**round1 指摘 S1 反映**: 「プログラム資産（exe・`data/`・ログ）は消す／
ユーザーデータ（`config.toml`・`layout/`）だけ残す」という分類をデフォルト
動作にも適用する（当初案は exe 2本しか消さず `data/`・ログが残留していた）。
ZIP 版は「プログラムと機能」に登録されないため、そもそも「アンインストール
した」という体感自体を得にくい。ユーザーが混乱しないよう `Write-Host` の
案内文だけでなく、README にも「ZIP 版のアンインストールは既定でプログラム
本体のみを消し、設定・配列ファイルは残る」ことを明記する（**round3後の
誤記訂正**: 旧稿は誤って「決定6」を参照していたが、これは決定1自身に
付随する周知であり、影響範囲節に既に記載がある）。

決定0によって MSI 経路は既に保護されるため、決定1は ZIP 経由の
「アンインストール→インストール」手順を踏んだ場合の防御に限定される。

### 決定2: `scripts/install.ps1` の `layout/*` コピーのみ「既存なら
上書きしない」方式に変更する（`data/*` は対象外）

```powershell
function Copy-IfAbsent($sourceGlob, $destDir) {
    New-Item -ItemType Directory -Force -Path $destDir | Out-Null
    if (Test-Path $sourceGlob) {
        Get-ChildItem $sourceGlob | ForEach-Object {
            $dest = Join-Path $destDir $_.Name
            if (-not (Test-Path $dest)) {
                Copy-Item $_.FullName $dest
            }
        }
    }
}
Copy-IfAbsent "layout\*" "$installDir\layout"

# data/ はユーザーが編集する手段を持たないプログラム資産なので従来通り無条件上書き
New-Item -ItemType Directory -Force -Path "$installDir\data" | Out-Null
if (Test-Path "data\*") {
    Copy-Item "data\*" "$installDir\data\" -Force
}
```

**round1 指摘 M2/S5 反映**:
- `layout/` は F3 の訂正により GUI 編集の主対象と判明したため非破壊化する。
  `data/`（n-gram テーブル等）はユーザー編集不可のプログラム資産であり、
  非破壊化すると将来の改善が既存ユーザーに永久に届かなくなる・展開失敗で
  壊れたファイルが自己修復されなくなる副作用の方が大きいため対象外とする。
  これにより ZIP と MSI（`data/` は決定0後 `NeverOverwrite` 無しで通常
  上書きコンポーネントのまま）の挙動が一致する。
- `$ErrorActionPreference = "Stop"` 下では `Get-ChildItem` がソースパス
  不在時に terminating error になりうる（`release.yml:47-48` は `data`
  コピー失敗を `|| true` で許容しており、空 `data/` の ZIP が現実に
  作られうる）ため `Test-Path` ガードを追加する。

**トレードオフとして許容する制約**: 同梱デフォルト `nicola.yab` に不具合
修正が入っても、既に `layout\nicola.yab` が存在する環境には自動反映
されなくなる。ユーザーが編集したかどうかを区別する仕組み（ハッシュ比較や
`.rpmnew` 方式の別名保存）を導入すれば回避できるが、現時点でそのような
同梱ファイル側の不具合修正が発生した実績がないため、
[[feedback_dont_provision_ahead_without_consumer_logic]] の教訓（消費
ロジックの無い予備機構を先回りで作らない）に従い見送る。実際に同梱
レイアウトの更新配布が必要になった時点で、ファイル名バージョニング
（例: `nicola-v2.yab` を新規追加しデフォルト参照先を切り替える）で対応する。

**2026-08-31追記（この制約が実際に発生したケース）**: report
`01M15R86FJW24278GGD3ETS9QX`（`docs/bug-reports-triage.md`参照）への対応で、
まさに同梱デフォルト`layout/nicola.yab`の内容変更が必要になった。当初は
既存ファイルへ直接上書きする形で実装したが、Opus敵対的レビューで
「MSIの`NicolaYab`（`NeverOverwrite="yes"`）・ZIP版`install.ps1`
（`Copy-IfAbsent`）のどちらも既存の`layout/nicola.yab`を上書きしないため、
アップグレードでは報告者本人にすら変更が届かない」と指摘され、上記の
想定通りファイル名バージョニング方式（新規ファイル`layout/nicola_keytop.yab`
を追加し、新規インストールの`default_layout`のみそちらを指す）へ設計変更
した。既存ユーザーへは`default_layout`の手動変更を案内する運用とし、
ハッシュ比較等の自動判別機構は今回も見送った（想定通りの対応で十分だった
ため、消費ロジックの無い予備機構を先回りで作らないという方針を今回も
維持した）。

### 決定3: `AppConfig::save()` を Windows の実書き込み失敗も考慮した
アトミック書き込みにする

```rust
pub fn save(&self, path: &Path) -> Result<()> {
    let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
    let tmp_path = path.with_extension(format!("toml.tmp.{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp_path)
            .with_context(|| format!("Failed to create {}", tmp_path.display()))?;
        f.write_all(content.as_bytes())
            .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
        f.sync_all()
            .with_context(|| format!("Failed to fsync {}", tmp_path.display()))?;
    }
    let mut last_err = None;
    for _ in 0..5 {
        match std::fs::rename(&tmp_path, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
    let _ = std::fs::remove_file(&tmp_path);
    Err(last_err.unwrap()).with_context(|| format!("Failed to rename into {}", path.display()))
}
```

**round1 指摘 M3 反映**:
- `sync_all()` を追加。`fs::write` 直後に `rename` するだけでは NTFS が
  データとメタデータの書き込み順序を保証しないため、電源断で「rename 済み・
  中身ゼロ長」が起こりうる。`File::create` → `write_all` → `sync_all` →
  drop → `rename` の順を明示する。
- `tmp_path` にプロセス ID を含め、複数プロセス（例: 将来 CLI ツールと
  GUI が同時に保存する等）が同時に `save()` を呼んでも一時ファイル名が
  衝突しないようにする。
- Windows の `rename`（`MoveFileEx(MOVEFILE_REPLACE_EXISTING)` 相当）は
  宛先が他プロセス（AV スキャナ・OneDrive・検索インデクサ・別のテキスト
  エディタ）に `FILE_SHARE_DELETE` 無しで開かれていると失敗しうる。
  現行の `fs::write` 直書きはこのケースでも成功していたため、単純な
  tmp+rename 化は**新たな保存失敗を持ち込む退行になりかねない**。
  初回試行の後、短いリトライ（50ms間隔で最大4回＝最大200msブロック、
  初回と合わせて最大5回試行）で緩和し、リトライ尽きた場合は一時
  ファイルを掃除してからエラーを返す（残骸が次回の `Test-Path` 系
  ロジックを汚さないように）。**コードレビュー訂正**: 実装（最終試行後は
  スリープしない）は当初の疑似コード（各試行後に無条件でスリープし、
  最終失敗後の1回分が無駄になる）より1回分速く、250msではなく実測200msが
  正しい上限。以下 SF-1 の記述もこれに合わせて訂正。

同一ボリューム内の `rename` は成功すればアトミックであり、書き込み中の
クラッシュ・強制終了・ディスクフルによる不完全な `config.toml`（F4 の
引き金の一つ）を構造的に無くす。

**round2 指摘の反映**:
- **SF-1**: `save()` は `crates/awase-windows/src/tray.rs:823` の
  `save_auto_start_config` からも同期呼び出しされ、これはエンジン
  スレッド（`run_message_loop`）上で実行される。リトライ最大200ms（訂正、
  上記参照）の `thread::sleep` はこの間エンジンスレッドをブロックし、
  打鍵がバースト出力される可能性がある。`hook.rs` が全キーを飲み込み
  エンジンスレッドが再注入する設計のため取りこぼしは起きないが、体感
  遅延が生じうる。`save()` はユーザーが明示的に「適用」を押した時と
  トレイの自動起動切替時のみ呼ばれる低頻度操作であり、リトライは rename
  失敗という稀なケースにのみ発動するため許容するが、実機で体感遅延が
  問題になればリトライ回数を減らす（既知の限界に記載）。
  **コードレビュー追加修正**: 当初 SF-1 は `tray.rs` 経由のエンジン
  スレッド呼び出しのみを分析しており、より高頻度に呼ばれる
  `crates/awase-settings/src/main.rs::SettingsApp::apply_confirmed()`
  （「適用」ボタン押下のたびに egui の UI スレッドから同期呼び出し）が
  未検討だった。バックアップ＋保存をバックグラウンドスレッドへ委譲し
  `poll_pending_save()` で毎フレームノンブロッキングにポーリングする形へ
  変更し、UI スレッドは一切ブロックしないようにした
  （`apply_confirmed_returns_without_blocking_on_slow_save` で回帰
  テスト化）。`tray.rs::save_auto_start_config` 側は元の分析通り許容範囲
  として同期のまま維持する。また `save()` の実体（tmp+fsync+rename+
  リトライ）は `crate::fs_atomic::write_atomic` へ切り出し、同じ問題
  （Windows の rename ロック）を抱えていた
  `crates/awase-windows/src/gji_charset_write.rs` の `config1.db` 書き込み
  からも共有するようにした（reuse 指摘の反映）。あわせて `path` が
  シンボリックリンクの場合はリンク先の実体へ書き込み、既存ファイルの
  パーミッションを引き継ぎ、宛先が読み取り専用の場合はリトライを省略して
  即座にエラーを返すようにした。
- **SF-6**: `use std::io::Write` が必要。`with_extension` は既存拡張子を
  置換する（`config.toml` → `config.toml.tmp.<pid>` で意図通り）。
  プロセス ID ベースの命名は同一プロセス内の複数スレッドからの同時
  `save()` 呼び出し衝突までは防がない。`awase-settings`/`awase-windows`
  いずれも `save()` を単一スレッドから呼ぶ設計を維持する前提とし、
  スレッド間排他は別途保証しない（既知の限界に記載）。

### 決定4: `awase-settings` の load 失敗を「デフォルトへの静かな置換」で
終わらせず、失敗種別ごとに扱いを分け、保存前バックアップを必須にする

**round1 指摘 M5 反映**: load 失敗を一括りにせず、`NotFound`（正当な初回
起動でありうる）と、それ以外の危険な失敗に分ける。

**round2 指摘 MF-2 反映（分類ルールの精密化）**: `AppConfig::load`
（`src/config.rs:538-544`）は `anyhow::Result` を返し、`read_to_string`/
`toml::from_str` それぞれを `with_context` で1層だけ包む。このため
`anyhow::Error::root_cause()`（あるいは `downcast_ref::<std::io::Error>()`）
で下層の `io::Error` を取り出せば `ErrorKind` の判定は機械的に可能だが、
**「NotFound 以外は何でも安全側」に倒す誤りを避けるため、分類は次の
2値ではなく明示的に3値**にする（当初案は「parse error」1種類しか
考慮しておらず、`PermissionDenied`・共有違反（OneDrive 未取得ファイル等）
のような **NotFound でも parse error でもない I/O エラー**の行き先が
enum に無かった）:

```rust
enum ConfigLoadState {
    Loaded,
    NotFound,   // io::ErrorKind::NotFound のときだけ。初回起動など、警告不要
    Dangerous(String), // NotFound 以外の全ての失敗（parse error / permission / 共有違反 等）
}

fn classify_load_error(e: &anyhow::Error) -> ConfigLoadState {
    let is_not_found = e
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
        .is_some_and(|io_err| io_err.kind() == std::io::ErrorKind::NotFound);
    if is_not_found {
        ConfigLoadState::NotFound
    } else {
        ConfigLoadState::Dangerous(e.to_string())
    }
}
```

**「`ErrorKind::NotFound` と確認できた場合のみ `NotFound`、それ以外は
種別を問わず `Dangerous`（警告・バックアップ必須側）に倒す」を分類の
唯一のルールとする。** これにより「素直に実装したら io::Error は全部
NotFound 扱いになった」という誤実装（round2 が指摘した最悪シナリオ）を
構造的に防ぐ。`SettingsApp` に `config_load_state: ConfigLoadState` を
追加し、`new()`（および `cancel()`、round2 指摘 SF-4: 再読み込み成功時に
`Loaded` へ戻す遷移を明記する）で設定する。`NotFound` は既存の
`default_config()` フォールバックのまま（警告不要、初回起動の正常系）。
`Dangerous` の場合のみ以下を必須にする:

1. **常時表示の警告**: `awase-settings` には既に `config_path_panel`
   （`main.rs:1991`、設定ファイルパスを常時表示する上部パネル）がある
   ため、これに「読み込み失敗中・現在表示中は初期値（原因: {エラー
   メッセージ}）」の赤帯警告を追加する。
2. **保存前バックアップ**: `apply()` が保存する直前、`config_path` が
   既に存在するなら **`config.toml.bak` が存在しない場合に限り**
   そこへ退避してから書き込む（round1 指摘 M4: 無条件に `.bak` へコピー
   すると、2回目の「適用」で「壊れた元ファイルのバックアップ」自体が
   「デフォルト値で上書き済みの config.toml」で潰れてしまう。バックアップは
   一度だけ・最初の異常発生時点のものを残す）。**round2 指摘 SF-3**:
   古い `.bak` が半年前の異常のまま残り続けて以後のバックアップを
   永久に抑止する穴が残るため、警告帯に `.bak` の存在とパスを明示し、
   ユーザーが必要なら手動で退避・削除できるようにする（自動ローテーション
   はスコープ外、既知の限界に記載）。
3. **確認ステップ**: `apply()` 実行時、`Dangerous` 状態のままなら
   確認ダイアログを挟む。**round2 指摘 SF-2（訂正）**: 当初「`rfd::
   MessageDialog` を使う」としていたが、`main.rs:568` の既存コードは
   実際には `rfd::AsyncFileDialog` であり、しかも UI スレッドから
   ネイティブダイアログを直接呼ばず `std::thread::spawn` +
   `pollster::block_on` + `join()` で迂回する既存パターンを取っている
   （`main.rs:566-578`）。この既存方針に合わせ、`egui` ネイティブの
   `egui::Window`（モーダル相当、`order(egui::Order::Foreground)` 等で
   実現）で確認 UI を実装する。文言:「設定ファイルの読み込みに
   失敗したため、現在表示中の内容は初期値です。このまま保存すると
   既存の設定を失う可能性があります。続行しますか？」の Yes/No。
   `config_load_state` は保存成功後 `Loaded` に遷移させる（フラグの
   寿命を明示、round1 指摘 M4）。

`NotFound` を確認・バックアップの対象から除外するのは、初回起動の
たびに警告が出るのを避けるため。

**round2 指摘 SF-5（決定6との相互作用）**: 決定6で `general` フィールドに
`#[serde(default)]` を付けると、`[general]` セクション欠落は parse error
でなくなる（＝結果的に改善だが `Dangerous` 分岐に入らなくなる）。決定4の
「危険な失敗」の適用範囲は決定6実施後は狭まる（構文エラー・型不一致・
`PermissionDenied` 等に限定される）ことを明記しておく。

### 決定5（不採用）: `config.toml` へのスキーマバージョン管理・
マイグレーション機構の新設

一般的なベストプラクティスとして検討したが、本 ADR のスコープでは
**採用しない**。理由:

- 決定6（`general` フィールドへの `#[serde(default)]` 付与）により、
  フィールド追加・削除の大半は既にマイグレーション不要で吸収できる。
- ADR-092 は `ThumbKeySoloTapGuard → ModeKeyConfig` のような構造変更でも
  「`from_legacy_bools` ブリッジ方式」で `config.toml` 非破壊を選択して
  おり、破壊的スキーマ変更を避ける実践が既に確立している。
- 実際に破壊的スキーマ変更が必要になった時点で、初めて
  `schema_version` フィールドとマイグレーション関数を追加すればよく、
  今そのための空の枠組みだけを先に作る理由がない
  （[[feedback_dont_provision_ahead_without_consumer_logic]]）。

決定3・4（アトミック書き込み・load-failure セーフティネット）で、
「読み込めないなら少なくとも元ファイルは残す／勝手に上書きしない」
という土台は確保されるため、スキーマバージョニングが無いことによる
実害は限定的と判断する。

### 決定6（round1 追加・低コスト高価値）: `GeneralConfig` フィールドにも
`#[serde(default)]` を付与する

```rust
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    ...
```

**round1 指摘 M6**: `AppConfig::general` にはこの属性が無く、`[general]`
セクションを欠く（あるいは `GeneralConfig` 内の非 `#[serde(default)]`
フィールドを欠く）`config.toml` は即座に parse error となり F4 の
引き金になる。`schema_version`（決定5で不採用）より遥かに安価に
欠損耐性を上げられる。

**round2 指摘 SF-5 訂正**: `GeneralConfig` 自体は既に `#[serde(default)]`
コンテナ属性（`config.rs:78`）と `impl Default`（`config.rs:319` 付近）を
持っている。不足しているのは `AppConfig::general` フィールド宣言側の
`#[serde(default)]` のみであり、「`GeneralConfig` が `Default` 未実装なら
追加する」という当初の書き方は不要な作業を示唆していたため削除する。
上記のフィールド属性1行を足すだけで完結する。

### 決定7（round1 追加・軽量、根治は別 ADR）: パス解決フォールバックの
診断性のみ最小限強化する

F5（`paths.rs::resolve_relative_to` の CWD フォールバック）の根本的な
再設計は本 ADR のスコース外とするが、少なくとも「意図しない CWD に
書き込まれた」ことが事後に追跡できるよう、`resolve_relative_to` が
ステップ4（exe 隣にもワークスペースルートにも見つからず、素の相対パスを
返す）に到達した場合に `log::warn!` を1回出す。

**実装時の訂正（コードレビュー指摘 P5）**: 当初「呼び出し元
（`AppConfig::load`/`save` を呼ぶ側）で warn を出す」としていたが、
`resolve_relative_to` は private 関数で `PathBuf` しか返さないため、
呼び出し元はフォールバックが起きたかどうかを原理的に判定できない
（round2 指摘 MF-4 で既に確定済みの制約）。実装は `resolve_relative_to`
自身のステップ4到達時点で `log::warn!` する形にしており、これは
`AppConfig::load`/`save` に限らずこの関数を経由する全呼び出し
（`awase.exe`/`awase-settings.exe`/`awase-linux`/`awase-macos` 各自の
`layouts_dir` 解決等も含む）に一様に効くため、ADR の当初案より単純かつ
広範囲をカバーする。パス解決ロジック自体の再設計・統一（`awase.exe` と
`awase-settings.exe` の解決結果を一致させる恒久対応）は別 ADR に切り出す。

### 決定8（round2 追加・MF-3）: `wix/main.wxs` の不変条件を機械的な
guard test で固定する

決定0の安全性はコンポーネント GUID が今後も変更されないことに依存するが、
`wix/main.wxs` へのコメント1行では歯止めにならない（round2 指摘 MF-3）。
`crates/awase-windows/tests/architecture_guard.rs` に倣い、`wix/main.wxs`
をテキストとして読み込み、以下を assert する軽量なテストを
`crates/awase-windows/tests/wix_installer_guard.rs`（新規）に追加する:

- `<MajorUpgrade ...>` が `Schedule="afterInstallExecute"` を含むこと。
- `ConfigFile`/`NicolaYab` コンポーネントがいずれも `NeverOverwrite="yes"`
  を持つこと。
- `ConfigFile`/`NicolaYab`/`NgramData`/`MainExe`/`SettingsExe` の
  `Guid` 属性値が、このテストにハードコードした既知の値と一致すること
  （変更したら意図的にテストごと更新する必要がある、という設計）。

このテストは XML の文字列マッチで十分であり、実際の MSI ビルド・実機を
要さず Linux 上の `cargo test -p awase-windows` で完結する。

## 保持するもの（変更しない）

**round3 指摘で訂正**: `NeverOverwrite="yes"` を持つのは `ConfigFile`
（既存）と `NicolaYab`（決定0で新規追加）の2コンポーネントのみ。
`NgramData` は決定0で意図的に `NeverOverwrite` を付けない（プログラム
資産のため、決定2参照）。固定 GUID の維持はこの3コンポーネント全てに
必要だが、「変更しない」節に `NicolaYab` の `NeverOverwrite` を含めるのは
誤りだったため以下のように書き分ける。

- MSI の `ConfigFile`/`NicolaYab`/`NgramData` コンポーネントの固定 GUID。
  決定0・決定8の前提として今後も GUID を変更しないこと。
- MSI の `ConfigFile` コンポーネントの `NeverOverwrite="yes"`（既存、
  変更なし）。`NicolaYab` への同属性追加は決定0の新規変更である
  （「保持」ではなく「変更」に属する）。
- `config.rs` の `#[serde(default)]` によるフィールド互換性維持方針
  （決定6で `general` にも拡大適用する）。
- `crates/awase-windows/src/tray.rs::save_auto_start_config`
  （`tray.rs:815-829`）は調査済みで安全と確認した。load 失敗時は
  `log::error!` を出すのみで `save()` を呼ばないため、F4 と同型の
  欠陥は無い。

## 影響範囲

- `wix/main.wxs`（決定0、最優先）
- `scripts/uninstall.ps1`（決定1）
- `scripts/install.ps1`（決定2）
- `src/config.rs::AppConfig::save`（決定3）・`AppConfig`/`GeneralConfig`
  の `#[serde(default)]`（決定6）・`ConfigLoadState`/`classify_load_error`
  （決定4。**round3 指摘で明記**: 判定ロジック自体は `AppConfig::load`
  と同じ `src/config.rs` に置き、`crates/awase-settings` 側は
  `SettingsApp::new`/`apply`/`cancel` からこれを呼び出して UI 状態
  （警告表示・確認ダイアログ・バックアップ）を制御する分担とする）
- `crates/awase-settings/src/main.rs`（`SettingsApp::new`/`apply`/`cancel`、
  決定4の UI 側）
- `src/paths.rs::resolve_relative_to`（決定7、ログ追加のみ）
- `crates/awase-windows/tests/wix_installer_guard.rs`（新規、決定8。
  **round3 指摘**: `architecture_guard.rs` と同様 `CARGO_MANIFEST_DIR`
  起点でファイルを読むため、`wix/main.wxs` へのパスは
  `../../wix/main.wxs` になる）
- エンドユーザー向けドキュメントへの ZIP 版アップグレード手順追記（決定1に
  伴う周知。「アンインストール→インストール」ではなく「ZIP を上書き展開
  するだけでよい」ことを明記する）。**実装時の訂正（コードレビュー指摘
  C3）**: 当初「README」としていたが、`README.md` はソースビルド手順のみで
  ZIP/MSI 配布のインストール/アップグレード手順を扱っていないと判明した。
  実際にエンドユーザーが参照するのは `docs/index.html`/`docs/usage.html`
  （`awase.cc`、`HOMEPAGE_URL` が指す先）とその英語版であり、ここに追記した
  （`README.md` への追記はスコープ外と判断）。

## テスト方針

`fix-requires-evidence.md` に従い、各決定に対応する回帰テストを置く:

- 決定0: 決定8の guard test（Linux `cargo test` で完結、GUID・
  `Schedule` 属性の存在を機械的に固定）に加え、実機（または
  `msiexec /l*v` ログ、`Orca` でのコンポーネントテーブル確認）で
  「旧バージョンインストール→`config.toml`/`layout/nicola.yab` を
  編集→新バージョンを上書きインストール→編集内容が残っていること」を
  確認する。**round2 指摘 MF-4**: この実機手順は必ず「`awase.exe` が
  常駐（自動起動）した状態でアップグレードを実行する」ケースを含める
  こと。`awase` は `HKCU\...\Run` で自動起動する常駐アプリ
  （`main.wxs:38-42`）であり、アップグレード時に実行中であることが
  常態のため、`afterInstallExecute` によるファイル配置順の変更が
  使用中ファイルの扱い（Restart Manager／再起動保留）に影響しないかを
  この状態で確認する必要がある。実機検証が取れない場合は
  `docs/known-bugs.md` への記録で代替する。
- 決定1・2: PowerShell でありこのリポジトリの `cargo test` 対象外。
  **実装時に判明した訂正**: `scripts/smoke-matrix.ps1`（App×IME×シナリオの
  打鍵検証ツール、`-Execute` で `SendInput` を発行する構造）は
  install/uninstall のファイルシステム検証とは器の形が異なり、この案
  そのままでは追加できない。Pester 等の新規テストフレームワーク導入は
  行わず、以下の手動検証チェックリストを `docs/known-bugs.md`（本件の
  記録）に記載し、実機セッションで実施することとする:
  1. ZIP 版を新規インストール（`install.ps1`）→ `config.toml` を編集・
     `layout/nicola.yab` を awase-settings の配列編集タブで変更・保存。
  2. 新しいバージョンの ZIP を展開し、同じフォルダへ `install.ps1` を
     再実行（`uninstall.ps1` を挟まない）。手順1の編集内容が保持されて
     いることを確認する。
  3. `uninstall.ps1`（`-Purge` なし）を実行 → `config.toml`/`layout/` が
     残り、`awase.exe`/`awase-settings.exe`/`data/`/`awase.log` が
     削除されていることを確認する。
  4. `uninstall.ps1 -Purge` を実行 → インストールディレクトリが丸ごと
     削除されることを確認する。
  実機検証困難な場合も `docs/known-bugs.md` への記録で代替する。
- 決定3: 以下を Linux 上の `cargo test` で固定する（round1 指摘 S7、
  「tmp ファイルの残存有無」は成功時に消えるため rename 経路の検証には
  ならない点に注意）:
  - 正常系: `save()` 後、保存先ファイルのファイル ID／inode が
    保存前と変わっていること（rename 経由になっていることの間接証拠）。
  - 異常系: 宛先パスをディレクトリにするなど rename を意図的に
    失敗させ、(a) 元ファイルが無傷で残ること、(b) 一時ファイルが
    残らないこと（リトライ尽き後のクリーンアップ）を確認する。
  - 事前に同名の `*.toml.tmp.<pid>` が残っていても正常に保存できる
    こと（プロセス ID ベースの命名で衝突しないこと）。
- 決定4: `classify_load_error` が `io::ErrorKind::NotFound` のときのみ
  `NotFound` を返し、それ以外（parse error・`PermissionDenied` 等の
  疑似エラーを含む）はすべて `Dangerous` に分類されることを、まず
  `src/config.rs` 側の純粋関数テストとして固定する（決定4のコード例
  そのものを対象にできるので GUI 起動は不要）。続けて `ConfigLoadState`
  が `Dangerous` のときのみバックアップ・確認フローに入ること、`.bak`
  が既存なら上書きしないこと、保存成功後 `Loaded` へ遷移することを
  `awase-settings` の既存ユニットテストパターン
  （`full_tab_layout_render_with_real_config_does_not_panic` 等）に
  倣って GUI 起動なしのロジック単体テストとして追加する。
- 決定6: `[general]` セクションを完全に欠く `config.toml` が
  parse error にならず `Default` 値で埋まることを既存の
  `config.rs` テスト群に倣って追加する。
- 決定8: `wix_installer_guard.rs` 自体が回帰テストであり、追加のテストは
  不要。

## 既知の限界・未検証事項

- Windows 実機での MSI メジャーアップグレード（決定0適用後、実際に
  `config.toml`/`layout/nicola.yab` が保持されることの実機確認は未実施。
  **本 ADR で最も検証優先度が高い項目**。
- Windows 実機での ZIP 版アップグレード手順（決定1・2適用後）の実機
  確認は未実施。
- 決定3のリトライ幅（初回試行の後、50ms間隔で最大4回＝最大200ms
  ブロック、初回と合わせて最大5回試行。**コードレビュー訂正**:
  当初「50ms×5=250ms」と記載していたが、実装は最終試行後にスリープ
  しないため実測200msが正しい上限）は実測に基づかない初期値。
  [tuning-constants](../../.claude/rules/tuning-constants.md) の対象
  タイマーではない（`tuning.rs` 外）が、実機で AV/OneDrive 等との競合が
  観測された場合は値を見直す。`tray.rs::save_auto_start_config`
  経由ではエンジンスレッドがこの間ブロックされるが、打鍵は
  `hook.rs` が全て飲み込み後で再注入する設計のため取りこぼしは無く、
  体感上は入力のバースト遅延にとどまる想定（実機未検証）。
  `awase-settings::SettingsApp::apply_confirmed()` 経由は**コードレビュー
  追加修正**でバックグラウンドスレッド化済みのため、この上限は UI スレッド
  をブロックしない（詳細は決定3節参照）。
  一時ファイル名（`<path>.tmp.<pid>`）はプロセス間の衝突は防ぐが、同一
  プロセス内の複数スレッドから同時に `save()`/`write_atomic()` が呼ばれる
  場合の排他は保証しない（round2 指摘 SF-6）。`save()` は単一スレッドから
  呼ぶ設計を前提とする（`apply_confirmed()` はバックグラウンドスレッドへの
  委譲中、同一 `SettingsApp` インスタンスからの多重起動を `pending_save`
  で防いでいる）。
- `crate::fs_atomic::write_atomic`（決定3の実体、`gji_charset_write.rs`
  とも共有）は `path` が既存ファイルへのシンボリックリンクの場合は
  `canonicalize` でリンク先の実体へ書き込み、既存ファイルのパーミッション
  を新しいファイルへ引き継ぐ（**コードレビュー追加修正**）。ただし
  `File::create` 直後のパーミッション適用と `rename` の間に短い window
  があり、この間に他プロセスが一時ファイルへアクセスすると意図した
  パーミッションが適用される前の状態を観測しうる（低リスク、単一
  ユーザーのデスクトップアプリという用途では実害は限定的）。
- 決定4の `.bak` は一度作成されると `Dangerous` 状態が解消するまで
  ローテーションされない。半年前の異常時に作られた `.bak` が残ったまま
  次に別の異常が起きても上書きされない（round2 指摘 SF-3）。警告帯に
  `.bak` のパスを表示してユーザーに手動整理を促す方針だが、自動
  ローテーションは本 ADR のスコープ外。
- **round3 指摘**: `NicolaYab` コンポーネントの `NeverOverwrite` 判定対象は
  `config.toml` と同じく `HKCU\Software\awase\NicolaYab` レジストリ値で
  あり `layout\nicola.yab` ファイルそのものではない（F1参照）。決定0
  適用後は「ユーザーが誤って `nicola.yab` を削除しても、MSI の修復
  インストールや再インストールでは復元されない」という**自己修復性の
  喪失**が新たに生じる（`config.toml` は元々この性質を持っていた）。
  本 ADR が解こうとしている問題（意図しない上書き・削除からユーザー
  データを守る）とは逆方向の副作用だが、発生条件が「ユーザー自身による
  ファイル削除」に限られ被害も限定的なため、本 ADR では許容し記録のみ
  残す。
- `AppConfig::save()` は `toml::to_string_pretty` による全体再シリアライズ
  のため、同梱 `config.toml` に含まれる解説コメントは「適用」1回で
  全て失われる（round1 指摘 S2）。これは決定3の対象外（コメント保持には
  TOML の低レベル編集が必要で本 ADR のスコープを超える）。ユーザー体感
  としては「設定ファイルの説明が消えた」という別種の"消失"報告に
  つながりうる点を記録として残す。
- `release.yml:45` がパッケージする `config.toml` はリポジトリルートの
  git 管理ファイルで、`config.sample.toml`（存在する場合）と別に運用
  されている（round1 指摘 S6）。開発者のローカル編集がそのまま配布用
  デフォルトになる経路の整理は本 ADR のスコープ外。
- F5（`paths.rs` の CWD フォールバック）の根治は別 ADR に切り出す
  （決定7は診断ログの追加のみ）。
- macOS/Linux はインストーラー自体が未整備のため本 ADR のスコープ外。
  将来インストーラーを整備する際は F1〜F3 と同型の問題
  （メジャーアップグレードでのユーザーデータ削除、アンインストーラーに
  よるユーザーデータ削除、アップデートスクリプトの無条件上書き）を
  作り込まないよう、決定0〜2 と同じ設計原則（「ユーザーデータは
  デフォルトで消さない・上書きしない」）を最初から適用すること。
