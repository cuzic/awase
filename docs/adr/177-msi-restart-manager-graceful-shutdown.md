---
id: ADR-177
title: |-
  常駐中のMSIアップグレードは実機検証の結果コード変更不要と判明（Restart Managerが自律的に処理）
status: |-
  **確定（2026-09-17）。コード変更不要、ADR-099 MF-4を解消。**
  opus-adversarial-consult round1〜4の4ラウンドを経て収束。round1で
  Blocker4件（実機観測ゼロで3変更決定）、round2でBlocker3件
  （サイレントのみ検証・データ保持未検証・実機残留の懸念）、round3で
  Blocker2件（計測基準点のズレ・解釈の向きが逆／実機残留データ未確認）
  を検出、いずれも追加の実機検証・ログ再解析で解消。round4でBlocker0件
  となり「コード変更不要」の結論が確定した。決定1〜3は不採用。副産物として
  「UI付きとサイレントでRMシャットダウンのコードパスが異なる」
  「MSIアンインストールはユーザーデータを削除する（ZIP版と非対称）」
  という2つの新知見を得た。
related_adr:
  - "ADR-099"
---

# ADR-177: 常駐中のMSIアップグレードは実機検証の結果コード変更不要と判明

## ステータス

**確定（2026-09-17）。コード変更は行わない。**

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
（round1検証時点ではこの区別を見落としており、レビューでの指摘を受けて後から気づいた）。

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

**注意（レビュー指摘により追記）**: `-dVersion`だけを変えたMSIでは
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

**機序（レビュー指摘を受けて追加調査し判明）**: `crates/awase-windows/build.rs`は
マニフェスト埋め込みのみで`awase.exe`にVERSIONINFOリソースを埋め込んで
いない。そのため`awase.exe`はMSIから見て**unversioned file**であり、
上書き判定はファイルバージョン比較ではなく`light.exe`が生成する
`MsiFileHash`テーブル（バイト列のハッシュ比較）が支配する。1.20.1→1.20.2
でコピーがスキップされたのはハッシュ一致のため、1.20.2→1.20.3で
置換されたのはハッシュ不一致のためで一貫して説明できる。
**つまりMSIの`Version`属性を上げるだけでは、`awase.exe`のバイト列が
同じである限りファイル置換自体が起きない。**

#### 観測2: バイナリの中身が変わる場合（1.20.2→1.20.3、サイレント）

`upgrade2.log`の`RESTART MANAGER:`行を全件引用する（`LaunchApplication`
を含む行は検索していない。M4/限界2参照）:

```
[08:25:29:383] RESTART MANAGER: Session opened.
[08:25:30:062] RESTART MANAGER: Will attempt to shut down and restart applications in no UI modes.
[08:25:30:070] RESTART MANAGER: Session opened.
[08:25:30:267] RESTART MANAGER: Successfully shut down all applications in the service's session that held files in use.
[08:25:30:268] RESTART MANAGER: Successfully shut down all applications that held files in use.
[08:25:33:889] RESTART MANAGER: Session opened.
[08:25:35:310] RESTART MANAGER: Previously shut down applications have been restarted.
[08:25:35:311] RESTART MANAGER: Session closed.
[08:25:35:319] RESTART MANAGER: Session closed.
[08:25:35:380] RESTART MANAGER: Previously shut down applications have been restarted.
[08:25:35:383] RESTART MANAGER: Session closed.
```

`Will attempt to shut down...`から`Successfully shut down...`までの
差は**205ms**（詳細な解釈は後続の観測3節、round1検証との比較部分を
参照）。アップグレード後、`awase.exe`のPIDは11488→31624に変わり、StartTimeも
新しくなった。`%LOCALAPPDATA%\awase\awase.exe`のSHA256ハッシュは
新ビルド（マーカー入り）と一致し、**ファイルが確実に新しいバイトへ
置き換わっている**ことを確認した。プロセスは1つだけ生き残っており、
多重起動は発生していない。

### round2検証: UI付きインストール、config.toml/layout編集の保持

1回目の実機検証（round1検証）がサイレントインストールのみで実配布経路を検証しておらず、
ADR-099 MF-4本体が求めるユーザーデータ保持の検証もしていなかったという
レビュー指摘に対応するため、以下を実施した。

1. `bootstrap.rs`のマーカーをV3に変更して再ビルド、1.20.4としてMSI化。
2. さらにV4に変更して再ビルド、1.20.5としてMSI化。
3. 1.20.4をクリーンインストール →`%LOCALAPPDATA%\awase\awase.exe`が
   自動起動（常駐、PID 30744）。
4. **常駐状態のまま**`%LOCALAPPDATA%\awase\config.toml`の
   `simultaneous_threshold_ms`を`100`→`999`に、
   `layout/nicola_keytop.yab`に識別用の1行を追記して編集。
5. `msiexec /i awase-1.20.5-x64.msi /l*v upgrade3.log`を**`/qn`を
   付けずに**実行（`UILevel=5`＝Full UI相当、実配布経路の再現）。

#### 観測3: UI付きでは、ダイアログを提示できない代わりに（再起動要求ではなく）RMシャットダウンへフォールバックする

`wix/main.wxs`には`UIRef`もDialogテーブルも無いため、`MsiRMFilesInUse`/
`FilesInUse`ダイアログはUIレベルによらずそもそも表示されえない
（authoringされていないダイアログは出せない）。round1検証だけでは
確認できていなかった本当の論点は「尋ねるべきダイアログを出せないとき、MSIは
**再起動をスケジュールして終わる**（ユーザーは成功したと思うが
実際には反映されない、悪い方の結果）のか、それとも**RMのシャットダウンに
黙ってフォールバックする**（良い方の結果）のか」だった。

`upgrade3.log`（`UILevel = 5`）を確認したところ、後者だった。
再起動要求（`ScheduleReboot`/`REBOOT`プロパティ設定等）は出ず、
以下の通り正常完了している:

```
MSI (c) (38:F8) [09:51:55:684]: 製品: awase -- インストールを正しく完了しました。
MSI (c) (38:F8) [09:51:55:685]: Windows インストーラーにより製品がインストールされました。
  製品名: awase、製品バージョン: 1.20.5、…、インストールの成功またはエラーの状態: 0
```

（この「状態: 0」だけでは`ERROR_SUCCESS_REBOOT_REQUIRED`＝3010との
区別がつかないが、`msiexec`呼び出し自体の終了コードは別途記録して
おらず、`upgrade3.log`にも`REBOOT`関連の要求は見当たらなかったこと、
SHA256ハッシュが新ビルドと一致し実際にファイル置換が完了していた
ことから、3010ではなく0だったと判断している——ただしこれは推定であり
確定的な確認ではない）。

**もう一点、round1検証との比較で重要な発見があった。** round1
（サイレント）の`RESTART MANAGER: Session opened.`の直後には
`Will attempt to shut down and restart applications **in no UI
modes**.`という行があり、シャットダウン成功までの差は205msだった。
しかし`upgrade3.log`全体を検索しても、この`in no UI modes`の行は
**一度も出現しない**。つまりUI付き（Full UI）とサイレントでは、
RMシャットダウンに至る**コードパス自体が異なる**。round1の基準点
（`Will attempt to shut down...`）がround2には存在しないため、
205msと以下の秒オーダーの差を単純に並べて比較すること自体ができない。

`upgrade3.log`の`RESTART MANAGER:`行も全件引用する
（`LaunchApplication`を含む行は0件だった）:

```
[09:51:30:366] RESTART MANAGER: Session opened.
[09:51:31:395] RESTART MANAGER: Session opened.
[09:51:50:791] RESTART MANAGER: Successfully shut down all applications in the service's session that held files in use.
[09:51:50:793] RESTART MANAGER: Successfully shut down all applications that held files in use.
[09:51:54:172] RESTART MANAGER: Session opened.
[09:51:55:637] RESTART MANAGER: Previously shut down applications have been restarted.
[09:51:55:637] RESTART MANAGER: Session closed.
[09:51:55:644] RESTART MANAGER: Session closed.
[09:51:55:859] RESTART MANAGER: Previously shut down applications have been restarted.
[09:51:55:862] RESTART MANAGER: Session closed.
```

`Session opened.`は同一ログ内に3回出現する（クライアント/サービス側の
複数トランザクションに対応するとみられる）。最初の出現を基準にすると
`Successfully shut down...`までは約20.4秒、2番目の出現を基準にすると
約19.4秒——**どちらを基準にしても秒オーダーである点は変わらない**。
`Session opened.`はインストールの早い段階（コスト計算前後）で出るため、
この約20秒には「RMがシャットダウンを試みて待った時間」以外に
「`RemoveExistingProducts`のスケジューリングや`InstallFiles`開始までの
MSI自体の処理時間」も含まれている可能性が高く、そのうち何秒が
シャットダウン待機なのかはこのログだけでは分離できない。**この差の
解釈の向きについても訂正する**: RMのシャットダウンは「メッセージを
送る→アプリの自発終了を待つ→タイムアウトしたら`TerminateProcess`」
という流れなので、時間が**長いほど「待たされた」＝強制終了に近づいた
可能性が上がる**方向に解釈するのが筋であり、旧版が書いていた
「gracefulな待機に近い」という解釈は向きが逆だった。ただし上記の通り
この約20秒のうちどこまでが実際の待機かが未分離なため、これ以上の
結論（gracefulか強制終了か）は出せない。**この差自体の原因（UI付き特有のコードパスによるものか、
実行時の環境差か）は依然として未確定であり、追加調査はしない**
（実機のログは既に削除済みで、再検証には新たなインストールサイクルが
必要になる。実害が顕在化した場合に改めて取り組む）。

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
  `DRAGONFLYG4\cuzic`（通常ユーザー）。`TokenElevationType`の直接取得は
  P/Invoke実装の不備で失敗したが、`InstallScope="perUser"`かつ
  `INSTALLDIR`が`LocalAppDataFolder`配下（`main.wxs:9`, `59-65`、昇格
  不要な構成）であること・インストール中にUACプロンプトが出た形跡が
  無いこと・実行ユーザーが通常ユーザーであることから、**昇格していない
  と判断してよい**。
- （副次的な発見）壊れた`nicola_keytop.yab`によりエラーダイアログを
  表示した新プロセスは、通常起動時と同じ`bootstrap::run_all()`を
  最後まで実行しており、`warn_layout_fallback`のモーダル
  `MessageBoxW`表示・`launch_settings()`による`awase-settings.exe`の
  起動まで含めて完全に走っていた。つまりRM再起動は「静かにプロセスを
  戻す」だけでなく、**通常起動時の全シーケンス（モーダルダイアログ・
  別プロセス起動を含む）をそのまま実行する**。将来「アップグレード
  直後に設定画面が勝手に開く」といった報告が来た場合の手がかりとして
  記録しておく。

### 解釈

**現状のawase.exe（`WM_QUERYENDSESSION`/`WM_ENDSESSION`ハンドラ無し、
`RegisterApplicationRestart`呼び出し無し）のまま、UI付き・サイレント
いずれのインストールでも、Restart Managerが実行中プロセスの検出・
シャットダウン・ファイル置換・再起動という一連の処理を自律的に行い、
ユーザーが編集したデータ（`config.toml`/`layout/`）も保持される。**

round1と round2 は RM シャットダウンに至るコードパス自体が異なり
（観測3参照）、基準点も揃わないため、シャットダウン所要時間として
単純に比較することはできない。したがって**旧プロセスの終了機序
（gracefulな`WM_CLOSE`経由か、猶予なしの`TerminateProcess`か）は
依然として確定できていない**（「`WM_QUERYENDSESSION`に既定でTRUEを
返すこと」自体はプロセスを終了させない。実際に終了させたのは
`TerminateProcess`か、awase側の既存`WM_CLOSE`ハンドラ
（`tray.rs:1093-1102`、`PostQuitMessage`）のいずれかで、両者を
区別する決め手はまだ得られていない）。ただし、いずれの経路であっても
**インストール失敗・データ消失は2回の検証を通じて確認されなかった**
（インストーラのログ上は両回とも正常完了と記録され——終了コード自体は
未記録、「検証の限界」6参照——`config.toml`/`layout/nicola_keytop.yab`
の編集内容は保持され、アンインストール後の残留も無かったことを
確認済み）。一方、**二重起動時に表示されうるバルーンやトレイの
ゴーストアイコンについては目視確認していないため、「実害が無い」
とまでは言い切れない**（「検証の限界」2・3参照）。

## 決定

### 決定1（旧・`WM_QUERYENDSESSION`/`WM_ENDSESSIONハンドラ追加）: 不採用

実機検証（サイレント・UI付き双方）の結果、Restart Managerは現状の
コードのままでもシャットダウン・再起動を完了できることが確認できたため、
追加のハンドラは不要と判断した。旧プロセスの終了がgraceful/強制終了の
どちらだったか確定できていない点は残るが、`docs/known-bugs/`には
計上しない（実害を示す兆候〈トレイのゴーストアイコン等〉を確認して
いないが、確認自体もしていないため、現時点では計上しない。
「検証の限界」3参照）。将来、強制終了に起因する
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

## 副次的な発見: MSIアンインストールはユーザーデータを削除する（ADR-099決定1との非対称）

本ADRの後片付け（「検証の限界」7）で、`msiexec /x`によるアンインストール
後に`%LOCALAPPDATA%\awase\config.toml`/`layout/`が**削除されている**
ことを確認した。これはMSIの標準的な挙動（`RemoveFolder`等）としては
自然だが、ADR-099が定めたZIP版の方針とは非対称になっている。

| 経路 | アンインストール既定時の`config.toml`/`layout/` |
| --- | --- |
| ZIP（`scripts/uninstall.ps1`） | **残す**（消すには`-Purge`明示フラグが必要、ADR-099決定1） |
| MSI（`msiexec /x`、ARPまたはスタートメニューの「Uninstall awase」） | **消える**（本ADRで実測） |

ADR-099は当時「決定0によってMSI経路は既に保護されるため、決定1（ZIP版の
非破壊化）はZIP経由の場合に限定される」としてMSI側のアンインストール時
挙動を検討対象から外していたが、これは「アップグレード時の保護」と
「アンインストール時の挙動」を混同していたことになる。

実害の筋道は具体的である: `wix/main.wxs:212-215`はスタートメニューに
「Uninstall awase」ショートカットを置いており、アンインストールは
ワンクリックで到達できる。不具合対応でよくある案内「一度アンインストール
して入れ直してください」をMSIユーザーが実行すると、`config.toml`の
全設定と配列編集タブで作り込んだ`layout/*.yab`が警告なく消え、
入れ直し後は初期状態になる——これはADR-099を起票させた元のユーザー報告
「バージョンアップすると既存の設定が失われる」と体感上同じ症状になる。

**この非対称を「バグ」として修正するかどうかは本ADRのスコープ外の
設計判断**（MSI側に`-Purge`相当の分岐を持たせるかは別途検討が必要）
とし、ここでは事実の記録に留める。ADR-099の「既知の限界・未検証事項」
にも同じ内容を追記した。

## 影響範囲

- コード変更なし。
- [ADR-099](099-config-preservation-on-upgrade.md)のステータス欄
  （「Windows実機でのアップグレード検証は未実施」の記述を、本ADRの
  実機検証結果へのリンクで更新する）と、「既知の限界・未検証事項」節
  （1項目目を「ADR-177で実施済み」に更新し、MSIアンインストール時の
  ユーザーデータ削除を新規項目として追記する）。

## 検証の限界（未解決のまま残す事項）

1. **旧プロセスの終了機序は未確定**（graceful/強制終了）。round1
   （サイレント、`Will attempt to shut down...`から205ms）と
   round2（UI付き、この行自体が出現せず`Session opened`から約19〜20秒、
   基準の取り方で変わる）では、シャットダウンに至るコードパス自体が
   異なることが判明した（「観測3」節参照）。約19〜20秒には「RMが
   シャットダウンを試みて待った時間」以外にMSI自体の処理時間も
   含まれている可能性が高く、この2つを
   単純比較して原因を特定することはできなかった。イベントビューアの
   `Microsoft-Windows-RestartManager/Operational`ログはチャンネル自体
   有効なのに記録が0件で活用できなかった。実害が顕在化しない限り
   追加調査はしないが、将来的に強制終了起因の症状（トレイアイコンの
   ゴースト、`INPUT_DEFER`退避キーの消失等）が不具合報告に上がった
   場合の手がかりとしてこのADRを参照すること。
2. **RM再起動と`LaunchApplication`カスタムアクションのどちらが
   実際にプロセスを復帰させたかは未確定。** 両方とも親プロセスは
   `msiexec.exe`になるため、新プロセスの親PIDだけでは区別できない
   （両経路とも`NOT Installed`条件・RM記憶機構によりメジャー
   アップグレード時に毎回発火するため、原理上は2つの起動経路が
   走り、後発が名前付きミューテックス`bootstrap.rs:950-983`で弾かれて
   `exit(1)`し、先行インスタンスへ`WM_DUPLICATE_INSTANCE`を送る設計
   のはず）。この場合に表示される「awase はすでに起動しています」
   バルーン（`message_handlers.rs:1100-1106`）が実際に出たかどうかも、
   今回は目視確認していない。`upgrade.log`/`upgrade2.log`/`upgrade3.log`
   は既に実機から削除済み（「検証の限界」8）のため、この突き合わせは
   今回のログでは行えない。**次にこの検証をやり直す際は**、`msiexec`
   を`/l*v`付きで実行し、`Action start …: LaunchApplication.`とRM再起動
   の時刻を突き合わせ、あわせてアップグレード直後の通知領域を目視する
   こと。
3. **アップグレード直後の通知領域（トレイ）のゴーストアイコンの
   有無は未確認。** 強制終了なら`SystemTray::drop`（`tray.rs:369-379`）
   の`Shell_NotifyIconW(NIM_DELETE)`が走らず死んだアイコンが残る
   はずで、これは「実害があるかどうか」の最も直接的な観測手段だが、
   2ラウンドを通じて一度も目視していない。「実害は確認されなかった」
   という記述は、正確には「実害を示す一次的な兆候は見ていない」に
   留まる。
4. **`awase-settings.exe`を開いたままのアップグレードは未検証。**
   `main.wxs`の`SettingsExe`コンポーネントも同じアップグレードで
   置換対象になるが、eframe/egui（winit）側のウィンドウがRestart
   Managerにどう扱われるかは`awase.exe`とは別問題であり、今回の
   検証範囲には含まれない。
5. **アンインストール（UI付き、`msiexec /x`）・修復（`msiexec /f`）は
   未検証。** 決定2で修復時の`RegisterApplicationRestart`非対応を
   許容すると判断したが、実際の修復時の挙動観察はしていない。
6. `msiexec`呼び出し自体の終了コード（`0`か`3010`か）は記録していない
   （「観測3」節参照。`upgrade3.log`の記述とSHA256一致から`0`だった
   と推定しているが確定的な確認ではない）。
7. 検証に使用したMSI（1.20.1〜1.20.5、いずれも`bootstrap.rs`への
   識別マーカー以外はdevelop相当）は**配布物ではない**。実機の後片付け
   として、アンインストール・`bootstrap.rs`マーカーのrevertに加えて、
   **`%LOCALAPPDATA%\awase\`ディレクトリの中身も確認した**——
   `config.toml`/`layout/`はアンインストール時に正しく削除されており、
   検証用に加えた破壊的編集（`simultaneous_threshold_ms = 999`、
   壊れた`nicola_keytop.yab`）が残留していないことを確認済み
   （ログファイル等MSI管理外のファイルはディレクトリに残っているが、
   `NeverOverwrite`保護の対象ではないため次回インストールに影響しない）。
   実機は通常のdevelop debugビルド常駐状態に復帰済み。
8. `upgrade.log`/`upgrade2.log`/`upgrade3.log`は後片付けの過程で
   実機から削除済み。本ADRに引用したログ抜粋が今後の追確認の
   一次情報になる（全文は保存していない）。

## 次のアクション

opus-adversarial-consult round4で「コード変更不要」の結論がBlocker 0件
で確定した（2026-09-17）。

対応済み: ADR-099のステータス欄・「既知の限界・未検証事項」節の更新
（MSI経路のみ「ADR-177で解消」に書き換え、ZIP経路は未検証のまま区別、
MSIアンインストール時のユーザーデータ削除を新規項目として追記）、
`docs/adr/index.md`のADR-177行更新（一連のADR-177関連コミットで
順次実施）。

残タスク（本ADRのスコープ外、別途判断）:

1. MSIアンインストール時のユーザーデータ削除（「副次的な発見」節）を
   ZIP版と同様に`-Purge`相当の分岐で保護するかどうかの設計判断。
