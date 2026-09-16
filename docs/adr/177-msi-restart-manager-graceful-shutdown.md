---
id: ADR-177
title: |-
  常駐中のMSIアップグレードをRestart Manager経由でgraceful shutdown対応する
status: |-
  **起草中。opus-adversarial-consult未実施。**
related_adr:
  - "ADR-099"
---

# ADR-177: 常駐中のMSIアップグレードをRestart Manager経由でgraceful shutdown対応する

## ステータス

**起草中（2026-09-16）。opus-adversarial-consultによるレビューはこれから。**

## コンテキスト

ユーザーから「msi インストールは awase.exe が実行中でもうまく動きますか」という
質問があり、調査した結果、以下が判明した。

### `wix/main.wxs` 側: 実行中プロセスを閉じる明示的な仕組みが無い

`wix/main.wxs` には `util:CloseApplication`（WiX v3の`RMCCPSearch`相当）や
`UIRef` の指定が無く、`MsiRMFilesInUse` ダイアログも組み込まれていない。
awase は `HKCU\...\Run` で自動起動する常駐アプリ（`main.wxs`の`MainExe`
コンポーネント）のため、**アップグレード時に実行中であることがむしろ常態**。

[ADR-099](099-config-preservation-on-upgrade.md) のround2指摘（MF-4）で、
まさに「`awase.exe` が常駐した状態でアップグレードを実行するケース」を
実機で確認すべきと指摘されていたが、ADR-099のステータス欄には「Windows
実機でのアップグレード検証は未実施」と書かれたまま、`docs/known-bugs/`
にもこの検証結果の記録は無い。

### `awase.exe` 側: Restart Managerのシャットダウン要求に応答しない

`crates/awase-windows/src/tray.rs::tray_wnd_proc`（メインウィンドウ
プロシージャ）がハンドルしているのは以下の4メッセージのみ:

```rust
match msg {
    WM_TRAY_CALLBACK => { ... }
    WM_COMMAND => { ... }
    WM_CLOSE => {
        tracing::info!("Tray window received WM_CLOSE — shutting down");
        PostQuitMessage(0);
        LRESULT(0)
    }
    WM_DESTROY => { PostQuitMessage(0); LRESULT(0) }
    _ => DefWindowProcW(hwnd, msg, wparam, lparam),
}
```

`WM_QUERYENDSESSION`/`WM_ENDSESSION`（Restart Managerがシャットダウン要求に
使うメッセージ）は一切ハンドリングされておらず、`DefWindowProcW` の既定
動作にフォールバックする。`DefWindowProcW`は`WM_QUERYENDSESSION`に対して
既定でTRUE（終了に同意）を返すが、続く`WM_ENDSESSION`ではawase側が
`PostQuitMessage`等を呼ぶコードが無いため、**awase自身は自発的には
終了しない**。

リポジトリ全体を`grep`しても、`RegisterApplicationRestart`・
`RmJoinSession`・`RmGetList`・`RmShutdown`の呼び出しは1件もない。
`WTSRegisterSessionNotification`（`app/bootstrap.rs`）は使われているが、
これはセッションのロック/アンロック検知（`WM_WTSSESSION_CHANGE`）用の
別機能であり、Restart Managerとは無関係。

### Windows Installer / Restart Managerの一般的な仕様（Web調査で確認）

[Microsoft公式ドキュメント](https://learn.microsoft.com/en-us/windows/win32/msi/using-windows-installer-with-restart-manager)によれば:

> Silent UI level installations always shut down applications and
> services, and on Windows Vista, always use Restart Manager.

つまり`msiexec /qn`のようなサイレントインストールでは、Windows Installer
自身がRestart Manager経由でファイルロック元のプロセスを自動的に
シャットダウンしようとする。ただしこれが実際に機能するには:

1. 対象プロセスが`WM_QUERYENDSESSION`/`WM_ENDSESSION`に正しく応答する
   通常のメッセージループアプリであること（Restart Managerはこれらの
   メッセージでgracefulな終了を試みる）。
2. `RegisterApplicationRestart`を呼んでおくと、インストール完了後
   （システム再起動が必要な場合は再起動後）にRestart Managerが
   **自動的にそのプロセスを再起動してくれる**。呼んでいないアプリは
   終了されたままで、復帰は次回ログオン時の自動起動任せになる。

現状のawaseはどちらも満たしていない。`WM_ENDSESSION`に応答しないため、
Restart Managerは最終的にタイムアウト後の強制終了（`TerminateProcess`）
に頼ることになる可能性が高い。強制終了はフック解放等のクリーンアップを
経由しないため、キーボードフックやIME関連リソースが正しく解放されない
リスクがある。また`RegisterApplicationRestart`が無いため、アップグレード
完了後にawaseが自動復帰せず、ユーザーが手動で再起動するか次回ログオンまで
親指シフト入力が使えない空白期間が生じる。

## 決定

以下を実装する。

### 決定1: `WM_QUERYENDSESSION`/`WM_ENDSESSION`をハンドルしgraceful shutdownする

`tray_wnd_proc`に両メッセージのハンドラを追加する。`WM_QUERYENDSESSION`では
既存のシャットダウン経路（`WM_CLOSE`と同様の後片付け）に問題が無いことを
確認した上でTRUEを返し、`WM_ENDSESSION`（`wparam`がTRUEのとき、つまり
実際に終了が確定したとき）で`PostQuitMessage(0)`を呼ぶ。

### 決定2: `RegisterApplicationRestart`を起動時に呼ぶ

`app/bootstrap.rs`の起動シーケンスで`RegisterApplicationRestart`を呼び、
Restart Manager経由で終了させられた場合にアップグレード後（必要なら
システム再起動後）自動的に再起動されるようにする。コマンドラインオプション
の扱い（`RESTART_NO_CRASH`等のフラグ、再起動時の引数）を実装時に詰める。

### 決定3: `wix/main.wxs`にLaunchCondition（保留中の再起動チェック）を追加する

`MsiSystemRebootPending`プロパティを条件にしたLaunchConditionを追加し、
既に別のインストールがシステム再起動待ちの状態での重ねてのインストールを
防ぐ（Microsoft公式が挙げるベストプラクティスの1つ）。

### 決定4（不採用）: カスタムアクションでの明示的な`RmShutdown`呼び出し

Microsoft公式ドキュメントは「カスタムアクションは`RmShutdown`/`RmGetList`/
`RmRestart`を呼ぶべきではない」と明記している（Windows Installer本体の
管轄）。`MsiRMFilesInUse`ダイアログの追加もFull UIレベルでのみ効果があり、
awaseのMSIはUI無し（サイレント運用が前提）のため、追加の効果が薄いと
判断し見送る。

### 決定5: 実機検証

ADR-099 round2 MF-4が要求していた「`awase.exe`が常駐した状態で
`msiexec`アップグレードを実行する」シナリオを実機で検証する。決定1〜3
実装後、以下を確認する:

1. awaseが常駐した状態で新バージョンMSIをサイレントインストールし、
   awaseがgracefulに終了すること（強制終了ログが出ないこと）。
2. アップグレード完了後、awaseが自動的に再起動されること
   （`RegisterApplicationRestart`の効果確認）。
3. `config.toml`/`layout/nicola.yab`等のユーザーデータがADR-099決定0
   の保護通り保持されていること（既存の確認事項との重複確認）。

検証結果は`docs/known-bugs/`または本ADRのステータス欄に記録する。

## 影響範囲

- `crates/awase-windows/src/tray.rs`（決定1）
- `crates/awase-windows/src/app/bootstrap.rs`（決定2）
- `wix/main.wxs`（決定3）
- `crates/awase-windows/tests/wix_installer_guard.rs`（決定3の回帰テスト、
  `MsiSystemRebootPending`条件の存在を固定）

## 未解決事項 / 次のアクション

1. opus-adversarial-consultによるレビュー（設計の妥当性、見落としの洗い出し）。
2. `RegisterApplicationRestart`の再起動時コマンドライン引数の設計
   （設定を引き継ぐ必要があるか、通常起動と同一でよいか）。
3. `WM_ENDSESSION`ハンドラでのクリーンアップ範囲
   （既存の`WM_CLOSE`/`WM_DESTROY`経路との共通化 or 専用化）。
4. 決定5の実機検証手順の具体化（clipwire経由での`msiexec /qn`実行、
   常駐状態の作り方）。
