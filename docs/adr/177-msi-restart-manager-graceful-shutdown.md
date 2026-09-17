---
id: ADR-177
title: |-
  常駐中のMSIアップグレードは実機検証の結果コード変更不要と判明（Restart Managerが自律的に処理）
status: |-
  **実機検証2ラウンド完了（2026-09-17）。コード変更不要、ADR-099 MF-4を解消。**
  opus-adversarial-consult round1でBlocker4件（実機観測ゼロで3変更決定・
  検証優先を要求）、round2でBlocker3件（サイレントのみ検証・データ保持
  未検証・実機残留の懸念）を検出。round2指摘を受けてUI付きインストール
  ＋config.toml/layout編集保持の追加検証を実施し、両方とも問題なしを
  確認。決定1〜3は不採用のまま維持。round3レビュー待ち。
related_adr:
  - "ADR-099"
---

# ADR-177: 常駐中のMSIアップグレードは実機検証の結果コード変更不要と判明

## ステータス

**実機検証2ラウンド完了（2026-09-17）。コード変更は行わない。**

当初、`awase.exe`側に`WM_QUERYENDSESSION`/`WM_ENDSESSION`ハンドラや
`RegisterApplicationRestart`を追加する案（旧決定1〜3）を起草したが、
opus-adversarial-consult round1（2026-09-16）で「実機観測ゼロのまま
3つの変更を決めている」という指摘（Blocker B2）を受けて実機検証を先行
実施した（round1検証）。その結果を受けた改訂案をround2レビューに
かけたところ、「検証がサイレントインストールのみで、実際の配布経路
（UI付き）が未検証」「ADR-099 MF-4が求めるユーザーデータ保持の検証を
していない」という指摘（Blocker B1・B2）を受け、追加の実機検証
（round2検証）を実施した。結果、**UI付きインストールでも、
`config.toml`/レイアウトファイルの編集内容を保持したままでも、
Restart Managerが自律的にシャットダウン・再起動を完了する**ことを
確認できたため、コード変更は不要と判断した。詳細は「実機検証の結果」
節を参照。

## コンテキスト

ユーザーから「msi インストールは awase.exe が実行中でもうまく動きますか」
という質問があり、調査した結果、以下が判明した。

### `wix/main.wxs` 側: 実行中プロセスを閉じる明示的な仕組みが無い

`wix/main.wxs` には `util:CloseApplication`（WiX v3の`RMCCPSearch`相当）や
`UIRef` の指定が無く、`MsiRMFilesInUse` ダイアログも組み込まれていない。
awase は `HKCU\...\Run` で自動起動する常駐アプリ（`main.wxs`の`MainExe`
コンポーネント）のため、**アップグレード時に実行中であることがむしろ常態**。

[ADR-099](099-config-preservation-on-upgrade.md) のround2指摘（MF-4）で、
まさに「`awase.exe` が常駐した状態でアップグレードを実行するケース」を
実機で確認すべきと指摘されていたが、ADR-099のステータス欄には「Windows
実機でのアップグレード検証は未実施」と書かれたまま、`docs/known-bugs/`
にもこの検証結果の記録は無かった（本ADRの実機検証がこれを解消する）。

実際の配布経路は`docs/index.html`が案内する「GitHub Releasesから
`.msi`をダウンロードしてダブルクリック」であり、これは`msiexec /qn`
（サイレント）ではなく**UI付き**（既定UIレベルFull）である点に注意
（round2レビューが指摘するまでround1検証はこの区別を見落としていた）。

### `awase.exe` 側: Restart Managerのシャットダウン要求を明示的にはハンドルしていない

`crates/awase-windows/src/tray.rs::tray_wnd_proc`（メインウィンドウ
プロシージャ）がハンドルしているのは`WM_TRAY_CALLBACK`/`WM_COMMAND`/
`WM_CLOSE`/`WM_DESTROY`の4つのみで、`WM_QUERYENDSESSION`/`WM_ENDSESSION`
（Restart Managerがシャットダウン要求に使うメッセージ）は一切
ハンドリングされておらず`DefWindowProcW`の既定動作にフォールバックする。
リポジトリ全体を`grep`しても`RegisterApplicationRestart`・
`RmJoinSession`・`RmGetList`・`RmShutdown`の呼び出しは1件もない。

当初はこれを「Restart Managerのシャットダウン要求に応答できず、
強制終了に頼ることになるのでは」という懸念として記録したが、
実機検証で**この懸念は該当しないことが判明した**（下記参照）。

## 実機検証の結果（2026-09-17、dragonflyg4実機）

**注意（round2レビュー指摘M5）**: `-dVersion`だけを変えたMSIでは
`awase.exe`のバイト列が変わらないため、Windows Installerがコピー自体を
スキップし、ファイル置換もRestart Manager経路も一切テストされない
（下記round1検証の観測1参照）。**この種の検証を再現する場合は、
必ずバイナリの中身自体を変えたビルドでMSIを作ること**（`bootstrap.rs`
に1行だけの識別マーカーを加える等）。

### round1検証: サイレントインストール、バイナリ変更の有無

1. WiX Toolsetを導入し、`awase.exe`/`awase-settings.exe`のrelease
   buildから`-dVersion`だけを変えた3つのMSI（1.20.1・1.20.2・1.20.3）
   を作成。
2. 1.20.1をクリーンインストール →`LaunchApplication`カスタムアクション
   により`%LOCALAPPDATA%\awase\awase.exe`が自動起動（常駐状態、PID記録）。
3. 常駐状態のまま`msiexec /i awase-1.20.2-x64.msi /l*v upgrade.log /qn`
   でアップグレード（1.20.1→1.20.2、**バイナリの中身は同一**）。
4. 続けて`bootstrap.rs`に1行だけの識別用マーカー文字列を加えて
   `awase.exe`を再ビルドし、1.20.3としてMSI化。常駐状態
   （1.20.2、PID記録）のまま`msiexec /i awase-1.20.3-x64.msi
   /l*v upgrade2.log /qn`でアップグレード（1.20.2→1.20.3、
   **バイナリの中身が変わる**）。

#### 観測1: バイナリが同一の場合（1.20.1→1.20.2、サイレント）

`upgrade.log`に`RESTART MANAGER: Session opened.`は出るが、
「is using files」「will require a restart」等のファイルロック検出
ログは一切出ない。`InstallFiles`/`RemoveFiles`はエラー・警告なく
数ミリ秒で完了し、アップグレード後もプロセスのPID・StartTimeは
**一切変化しない**。

**機序（round2レビュー指摘M4で判明）**: `crates/awase-windows/build.rs`は
マニフェスト埋め込みのみで`awase.exe`にVERSIONINFOリソースを埋め込んで
いない。そのため`awase.exe`はMSIから見て**unversioned file**であり、
上書き判定はファイルバージョン比較ではなく`light.exe`が生成する
`MsiFileHash`テーブル（バイト列のハッシュ比較）が支配する。1.20.1→1.20.2
でコピーがスキップされたのはハッシュ一致のため、1.20.2→1.20.3で
置換されたのはハッシュ不一致のためで一貫して説明できる。
**つまりMSIの`Version`属性を上げるだけでは、`awase.exe`のバイト列が
同じである限りファイル置換自体が起きない。**

#### 観測2: バイナリの中身が変わる場合（1.20.2→1.20.3、サイレント）

`upgrade2.log`に以下が明確に記録された:

```
[08:25:30:062] RESTART MANAGER: Will attempt to shut down and restart applications in no UI modes.
[08:25:30:267] RESTART MANAGER: Successfully shut down all applications in the service's session that held files in use.
[08:25:35:310] RESTART MANAGER: Previously shut down applications have been restarted.
```

シャットダウン成功から`is using files`検出までの差は**205ms**。
アップグレード後、`awase.exe`のPIDは11488→31624に変わり、StartTimeも
新しくなった。`%LOCALAPPDATA%\awase\awase.exe`のSHA256ハッシュは
新ビルド（マーカー入り）と一致し、**ファイルが確実に新しいバイトへ
置き換わっている**ことを確認した。プロセスは1つだけ生き残っており、
多重起動は発生していない。

### round2検証: UI付きインストール、config.toml/layout編集の保持

round2レビューのBlocker B1（サイレントのみで実配布経路を未検証）・
B2（ADR-099 MF-4本体＝ユーザーデータ保持を未検証）に対応するため、
以下を実施した。

1. `bootstrap.rs`のマーカーをV3に変更して再ビルド、1.20.4としてMSI化。
2. さらにV4に変更して再ビルド、1.20.5としてMSI化。
3. 1.20.4をクリーンインストール →`%LOCALAPPDATA%\awase\awase.exe`が
   自動起動（常駐、PID 30744）。
4. **常駐状態のまま**`%LOCALAPPDATA%\awase\config.toml`の
   `simultaneous_threshold_ms`を`100`→`999`に、
   `layout/nicola_keytop.yab`に識別用の1行を追記して編集。
5. `msiexec /i awase-1.20.5-x64.msi /l*v upgrade3.log`を**`/qn`を
   付けずに**実行（`UILevel=5`＝Full UI相当、実配布経路の再現）。

#### 観測3: UI付きインストールでもFilesInUse系ダイアログは出ない

`upgrade3.log`（`UILevel = 5`）を確認したところ、`MsiRMFilesInUse`/
`FilesInUse`ダイアログが表示された形跡はなく、以下の通り正常完了した:

```
MSI (c) (38:F8) [09:51:55:684]: 製品: awase -- インストールを正しく完了しました。
MSI (c) (38:F8) [09:51:55:685]: Windows インストーラーにより製品がインストールされました。
  製品名: awase、製品バージョン: 1.20.5、…、インストールの成功またはエラーの状態: 0
```

一方、**RESTART MANAGER のシャットダウンに要した時間が、round1検証
（サイレント、205ms）と大きく異なった**:

```
[09:51:31:395] RESTART MANAGER: Session opened.
[09:51:50:791] RESTART MANAGER: Successfully shut down all applications in the service's session that held files in use.
```

差は**約19.4秒**。205msでは（M1/M2で指摘の通り）「メッセージを送って
応答を待ちタイムアウトする」経路にはならないが、19.4秒という長さは
graceful待機に近い時間帯であり、round1検証とは異なる終了経路
（gracefulな終了、または単に環境要因によるRM内部の待機）を通った
可能性がある。**この差の原因は特定できていない**
（UI付き/サイレントの違いによるものか、単なる実行時の環境差かは
未確定。イベントビューアの`Microsoft-Windows-RestartManager/Operational`
ログで裏付けを試みたが、チャンネル自体は有効（`IsEnabled=True`）
なのに記録は0件で、判別材料にはならなかった）。

インストール完了後、以下を確認した:

- **`config.toml`の編集内容（`simultaneous_threshold_ms = 999`）が
  保持されていた。**
- **`layout/nicola_keytop.yab`への追記内容も保持されていた**
  （ただし追記した文字列`# MSITEST-EDIT-MARKER`がYABファイルの
  フォーマットとして不正だったため、新しく起動した`awase.exe`が
  「レイアウトの読み込みに失敗しました」というエラーダイアログを
  表示した。**これはテスト手順の不備であり、製品側のバグではない**
  ——逆に、このエラーが起きたこと自体が「編集済み（壊れた）
  レイアウトファイルがアップグレード後も破棄されずに読み込まれた」
  ことの証拠になっている）。
- `%LOCALAPPDATA%\awase\awase.exe`のSHA256ハッシュは1.20.5の新ビルドと
  一致（ファイル置換を確認）。
- 新プロセスの親プロセスは`msiexec.exe`、コマンドラインは引数なし
  （`"C:\Users\cuzic\AppData\Local\awase\awase.exe"`のみ）、実行ユーザーは
  `DRAGONFLYG4\cuzic`（通常ユーザー、SYSTEM等への昇格は無い）。
  昇格の有無を`TokenElevationType`で直接確認しようとしたが、
  P/Invoke実装の不備で値が取得できず未確定のまま。

### 解釈

**現状のawase.exe（`WM_QUERYENDSESSION`/`WM_ENDSESSION`ハンドラ無し、
`RegisterApplicationRestart`呼び出し無し）のまま、UI付き・サイレント
いずれのインストールでも、Restart Managerが実行中プロセスの検出・
シャットダウン・ファイル置換・再起動という一連の処理を自律的に行い、
ユーザーが編集したデータ（`config.toml`/`layout/`）も保持される。**

一方で、round1検証の205ms・round2検証の19.4秒という2つの異なる
シャットダウン所要時間から、**旧プロセスの終了機序（gracefulな
`WM_CLOSE`経由か、猶予なしの`TerminateProcess`か）は依然として
確定できていない**（round2レビューM1・M2が指摘した通り、
「`WM_QUERYENDSESSION`に既定でTRUEを返すこと」自体はプロセスを
終了させない。実際に終了させたのは`TerminateProcess`か、
awase側の既存`WM_CLOSE`ハンドラ（`tray.rs:1093-1102`、
`PostQuitMessage`）のいずれかで、両者を区別する決め手は
まだ得られていない）。ただし、いずれの経路であっても
**2回の検証を通じてユーザー影響（インストール失敗、データ消失、
二重起動、ダイアログでの停止）は確認されなかった**。

## 決定

### 決定1（旧・`WM_QUERYENDSESSION`/`WM_ENDSESSIONハンドラ追加）: 不採用

実機検証（サイレント・UI付き双方）の結果、Restart Managerは現状の
コードのままでもシャットダウン・再起動を完了できることが確認できたため、
追加のハンドラは不要と判断した。旧プロセスの終了がgraceful/強制終了の
どちらだったか確定できていない点は残るが、`docs/known-bugs/`には
計上しない（実害が確認されていないため）。将来、強制終了に起因する
具体的な症状（トレイアイコンのゴースト等）が不具合報告として上がった
場合に、改めてこの経路を疑うための記録として本ADRを残す。

### 決定2（旧・`RegisterApplicationRestart`呼び出し）: 不採用

実機検証で、`RegisterApplicationRestart`を呼んでいなくてもRestart
Manager自身の再起動機能で復帰することが確認できた。加えて、
`wix/main.wxs`には既に`LaunchApplication`カスタムアクション
（`InstallFinalize`後、`NOT Installed`条件）があり、`Product Id="*"`
により毎ビルドでProductCodeが変わるためメジャーアップグレードも
新規インストール扱いとなり`NOT Installed`が真になる＝**このカスタム
アクションも毎回発火する**（今回の実機検証でも1.20.1新規インストール時
に`LaunchApplication`の発火を確認済み）。`RegisterApplicationRestart`
を追加すると、Restart Manager自身の再起動・`LaunchApplication`という
既存の2経路と合わせて3経路になり、二重起動やコマンドライン引数の
非決定性（`RESTART_NO_CRASH`等のフラグ設計、`UnregisterApplicationRestart`
呼び出し忘れ）のリスクを新たに持ち込むだけで得るものがない。

なお、`msiexec /f`（修復）時は`LaunchApplication`の条件`NOT Installed`
が偽になるため発火しない。この経路だけは`RegisterApplicationRestart`が
効きうる唯一のケースだが、修復はARP（アプリと機能）経由でのみ到達する
導線でありユーザーが日常的に使うものではないため、この1ケースのために
複雑さを持ち込む判断はしない（許容する）。

### 決定3（旧・`MsiSystemRebootPending` LaunchCondition追加）: 不採用

この条件は「RMのシャットダウンに失敗して再起動がスケジュールされた後」
に立つ状態であり、追加すると、まさにアップグレードに失敗した直後の
ユーザーの再インストール試行をブロックしてしまう。今回の実機検証では
シャットダウン失敗自体が発生しなかったため、この対策の必要性を
裏付ける具体的な失敗事例も無い。不採用とする。

### 決定4: カスタムアクションでの明示的な`RmShutdown`呼び出しは不採用（変更なし）

Microsoft公式ドキュメントは「カスタムアクションは`RmShutdown`/
`RmGetList`/`RmRestart`を呼ぶべきではない」と明記している
（Windows Installer本体の管轄）。今回の実機検証でもWindows Installer
自身の処理だけで問題なく完了しており、この方針を変える理由はない。

### 決定5: 実機検証（実施済み、round1・round2の2ラウンド）

上記「実機検証の結果」節の通り実施済み。[ADR-099](099-config-preservation-on-upgrade.md)
round2 MF-4が要求していた「常駐状態でのアップグレード」検証、および
決定0本体が求める「ユーザーが編集したデータの保持」検証の両方を
round2検証で満たしたため、ADR-099のステータス欄も本ADR完了と合わせて
更新する（ただし、これは`config.toml`/`layout/nicola_keytop.yab`を
実際に編集してからのアップグレードで確認したものであり、ADR-099が
挙げる4段階の検証チェックリスト全項目〈ZIP版install.ps1/uninstall.ps1
の`-Purge`挙動等〉を網羅したものではない点に注意）。

## 影響範囲

- コード変更なし。
- [ADR-099](099-config-preservation-on-upgrade.md)のステータス欄
  （「Windows実機でのアップグレード検証は未実施」の記述を、本ADRの
  実機検証結果へのリンクで更新する）。

## 検証の限界（未解決のまま残す事項）

1. **旧プロセスの終了機序は未確定**（graceful/強制終了）。
   round1（205ms）とround2（19.4秒）で異なるシャットダウン所要時間が
   観測されたが、原因は特定できていない。イベントビューアの
   `Microsoft-Windows-RestartManager/Operational`ログは記録が無く
   活用できなかった。実害が顕在化しない限り追加調査はしないが、
   将来的に強制終了起因の症状（トレイアイコンのゴースト、
   `INPUT_DEFER`退避キーの消失等）が不具合報告に上がった場合の
   手がかりとしてこのADRを参照すること。
2. **`awase-settings.exe`を開いたままのアップグレードは未検証。**
   `main.wxs`の`SettingsExe`コンポーネントも同じアップグレードで
   置換対象になるが、eframe/egui（winit）側のウィンドウがRestart
   Managerにどう扱われるかは`awase.exe`とは別問題であり、今回の
   検証範囲には含まれない。
3. **アンインストール（UI付き、`msiexec /x`）・修復（`msiexec /f`）は
   未検証。** 決定2で修復時の`RegisterApplicationRestart`非対応を
   許容すると判断したが、実際の修復時の挙動観察はしていない。
4. **昇格状態（elevation）の直接確認は失敗した。** 新プロセスの
   実行ユーザーが通常ユーザー（`DRAGONFLYG4\cuzic`）であることは
   確認したが、`TokenElevationType`の取得はP/Invoke実装の不備で
   失敗しており、Full/Limited/Defaultのどれだったかは未確定。
5. 検証に使用したMSI（1.20.1〜1.20.5、いずれも`bootstrap.rs`への
   識別マーカー以外はdevelop相当）は**配布物ではない**。実機の後片付け
   （アンインストール、マーカーのrevert）は完了済みで、実機は
   通常のdevelop debugビルド常駐状態に復帰済み。

## 次のアクション

1. opus-adversarial-consult round3（本ADRの追加検証結果への再レビュー）。
2. ADR-099のステータス欄更新（決定5解消の反映、検証範囲を明記）。
