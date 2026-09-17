---
id: ADR-177
title: |-
  常駐中のMSIアップグレードは実機検証の結果コード変更不要と判明（Restart Managerが自律的に処理）
status: |-
  **実機検証完了（2026-09-17）。コード変更不要、ADR-099 MF-4を解消。**
  opus-adversarial-consult round1で当初案（決定1〜3のコード変更）に
  Blocker4件・Major8件を検出、「決定5（実機検証）を先に単独実施すべき」
  という指摘を受けて実機検証を先行実施した結果、Restart Managerが
  現状のコードのまま自律的にシャットダウン・再起動を処理していることが
  確認できたため、決定1〜3は全て不採用とし、ADRは「検証して変更不要と
  結論した」記録として残す。round2レビュー待ち。
related_adr:
  - "ADR-099"
---

# ADR-177: 常駐中のMSIアップグレードは実機検証の結果コード変更不要と判明

## ステータス

**実機検証完了（2026-09-17）。コード変更は行わない。**

当初、`awase.exe`側に`WM_QUERYENDSESSION`/`WM_ENDSESSION`ハンドラや
`RegisterApplicationRestart`を追加する案（旧決定1〜3）を起草したが、
opus-adversarial-consult round1（2026-09-16）で「実機観測ゼロのまま
3つの変更を決めている」「決定5（実機検証）を先に単独実施すべき」という
指摘（Blocker B2）を受け、先に実機検証を行った。結果、**現状のコードの
まま、Restart Managerが実行中の`awase.exe`を検出・シャットダウン・
再起動する一連の処理を完全に自律的に行っている**ことが確認できたため、
コード変更は不要と判断した。詳細は「実機検証の結果」節を参照。

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

### 手順

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

### 観測1: バイナリが同一の場合（1.20.1→1.20.2）

`upgrade.log`に`RESTART MANAGER: Session opened.`は出るが、
「is using files」「will require a restart」等のファイルロック検出
ログは一切出ない。`InstallFiles`/`RemoveFiles`はエラー・警告なく
数ミリ秒で完了し、アップグレード後もプロセスのPID・StartTimeは
**一切変化しない**（Windows Installerがファイル内容の変更なしと
判断してコピー自体をスキップしたためと考えられる）。

### 観測2: バイナリの中身が変わる場合（1.20.2→1.20.3）

`upgrade2.log`に以下が明確に記録された:

```
[08:25:30:062] RESTART MANAGER: Will attempt to shut down and restart applications in no UI modes.
[08:25:30:267] RESTART MANAGER: Successfully shut down all applications in the service's session that held files in use.
[08:25:35:310] RESTART MANAGER: Previously shut down applications have been restarted.
```

アップグレード後、`awase.exe`のPIDは11488→31624に変わり、StartTimeも
新しくなった。`%LOCALAPPDATA%\awase\awase.exe`のSHA256ハッシュは
新ビルド（マーカー入り）と一致し、**ファイルが確実に新しいバイトへ
置き換わっている**ことを確認した。プロセスは1つだけ生き残っており、
多重起動は発生していない。

### 解釈

**現状のawase.exe（`WM_QUERYENDSESSION`/`WM_ENDSESSION`ハンドラ無し、
`RegisterApplicationRestart`呼び出し無し）のまま、Restart Managerは
実行中プロセスの検出・シャットダウン・ファイル置換・再起動という
一連の処理を完全に自律的に行っている。** これは以下の事実と整合する:

- Restart Managerは、対象プロセスが`WM_QUERYENDSESSION`/`WM_ENDSESSION`
  に明示応答しなくても、`DefWindowProcW`の既定応答（`WM_QUERYENDSESSION`
  への既定TRUE）や、最終手段としての`TerminateProcess`を組み合わせて
  シャットダウンを完了できる。
- Windows Installerの「Silent UI level installations always shut down
  applications and services, and on Windows Vista, always use Restart
  Manager.」という仕様（[Microsoft公式ドキュメント](https://learn.microsoft.com/en-us/windows/win32/msi/using-windows-installer-with-restart-manager)）通り、
  `msiexec /qn`ではRestart Managerが常に介入する。
- 再起動については、`RegisterApplicationRestart`を呼んでいなくても、
  **Restart Manager自身が「MSIが自分でシャットダウンしたプロセスを
  記憶しておき、インストール完了後に自動的に再起動する」機能**
  （ログの`Previously shut down applications have been restarted`）
  を持っており、これが機能した。

一点、`awase.log`側に旧プロセスの`"Exited cleanly"`（正常終了ログ）は
見当たらなかった。これは`app/logging.rs::RotatingLogWriter`がINFOログを
即座にflushしない設計（`flush_on_drop`はWARN以上のみ）ためログが
バッファに残ったまま消えた可能性が高く、実際にgracefulだったか
強制終了だったかはこの観測だけでは判別できない。ただし、いずれの場合も
**最終的にファイル置換・再起動は正常に完了しており、ユーザー影響
（インストール失敗、データ消失、二重起動）は確認されなかった**。

## 決定

### 決定1（旧・`WM_QUERYENDSESSION`/`WM_ENDSESSIONハンドラ追加）: 不採用

実機検証の結果、Restart Managerは現状のコードのままでもシャットダウン・
再起動を完了できることが確認できたため、追加のハンドラは不要と判断した。
旧プロセスの終了がgraceful/強制終了のどちらだったか確定できていない点は
残るが、`docs/known-bugs/`には計上しない（実害が確認されていないため）。
将来、強制終了に起因する具体的な症状（トレイアイコンのゴースト等）が
不具合報告として上がった場合に、改めてこの経路を疑うための記録として
本ADRを残す。

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

### 決定5: 実機検証（実施済み）

上記「実機検証の結果」節の通り実施済み。[ADR-099](099-config-preservation-on-upgrade.md)
round2 MF-4が要求していた「常駐状態でのアップグレード」検証はこれで
解消したため、ADR-099のステータス欄も本ADR完了と合わせて更新する。

## 影響範囲

- コード変更なし。
- [ADR-099](099-config-preservation-on-upgrade.md)のステータス欄
  （「Windows実機でのアップグレード検証は未実施」の記述を、本ADRの
  実機検証結果へのリンクで更新する）。

## 未解決事項 / 次のアクション

1. opus-adversarial-consult round2（本ADRの結論そのものへの再レビュー）。
2. 旧プロセスの終了がgraceful/強制終了のどちらだったかは未確定のまま。
   実害が顕在化しない限り追加調査はしないが、将来的に強制終了起因の
   症状（トレイアイコンのゴースト等）が不具合報告に上がった場合の
   手がかりとしてこのADRを参照すること。
3. ADR-099のステータス欄更新（決定5解消の反映）。
