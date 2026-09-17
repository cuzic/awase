# ADR-178 敵対的レビュー round1

対象: `docs/adr/178-msi-uninstall-preserve-userdata.md`（722a4f9c 時点、起草段階・コード未実装）

判定: **採用案（選択肢A: 7コンポーネントへの `Permanent="yes"` 追加）は、このままでは
出荷してはいけない。** 致命的な相互作用（Blocker 1）と、目的そのものが達成されない可能性
（Blocker 2）がある。いずれも実装前に解決できる。

---

## Blocker

### B1. `Permanent` は KeyPath レジストリ値も永続化する → `NeverOverwrite` と噛み合って「再インストールしても設定ファイルが戻らない」状態を恒久的に作る

**事実関係**:

1. 対象7コンポーネントの KeyPath は *ファイルではなくレジストリ値* である
   （`wix/main.wxs:71-73` のコメント「perUser インストールでは File を KeyPath に
   できない（ICE38）。各コンポーネントは HKCU レジストリキーを KeyPath にする」。
   実際 `ConfigFile` は `wix/main.wxs:116-117` の
   `HKCU\Software\awase` `Name="ConfigFile"`、`NicolaYab` は同 `Name="NicolaYab"`、
   以下同様）。
2. `Permanent` は **コンポーネント全体** に効く（[MS Learn: Installing Permanent
   Components](https://learn.microsoft.com/en-us/windows/win32/msi/installing-permanent-components-files-fonts-registry-keys)
   「To install a file, font, or registry key so that it is not removed when the
   product is uninstalled, **the entire component** containing the file, font, or
   registry key must be made permanent」）。つまり `Permanent="yes"` にすると、
   `config.toml`/`*.yab` だけでなく **KeyPath である `HKCU\Software\awase\ConfigFile`
   等7つのレジストリ値もアンインストール後に残る**。ADR本文はこの副作用に
   一切触れていない。
3. `NeverOverwrite` は「KeyPath が存在するならコンポーネントを install も
   reinstall もしない。**存在する場合、インストーラはそのコンポーネントを
   『インストール済み』として登録するだけで、実体は配置しない**」という
   セマンティクス（Component Table `msidbComponentAttributesNeverOverwrite` の
   定義。`wix/main.wxs:144-148` と
   `crates/awase-windows/tests/wix_installer_guard.rs:150-156` に書かれている
   「KeyPath が無い真の新規インストールには影響しない」という理解の裏返し）。
   [FireGiant: KeyPaths explained](https://support.firegiant.com/hc/en-us/articles/230912347-KeyPaths-explained)
   も「KeyPath レジストリ値が存在するが実ファイルが無い場合、インストーラは
   『既にある』と判断して再配置しない」と述べている。

**具体的な失敗シナリオ**（ADR本文が自ら案内している手順そのもの）:

```
1. 新MSI（Permanent付き）をインストール。config.toml / layout/*.yab を編集。
   HKCU\Software\awase\{ConfigFile,NicolaYab,...} の7値が存在する。
2. msiexec /x でアンインストール。
   → 意図通り config.toml / layout/*.yab は残る。
   → 同時に上記7つのレジストリ値も残る（ADR未記載）。
3. ADR「ドキュメントへの追記」節の案内に従い、ユーザーが手動で
   %LOCALAPPDATA%\awase を削除する（＝ADRが唯一の完全削除手段として
   提示している操作）。レジストリには誰も触れない。
4. 後日ユーザーが awase を再インストール（MSI ダブルクリック）。
   → KeyPath の7値が存在する → NeverOverwrite により
      ConfigFile / NicolaYab / NicolaKeytopYab / NicolaUsYab / NicolaFYab /
      NicolaKb232Yab / NicolaKakuteiYab の **全7コンポーネントが配置されない**。
   → config.toml も 6本の .yab も一つもディスクに書かれない。
   → MSI は「インストール成功」を返す。
5. awase.exe 起動 → crates/awase-windows/src/app/mod.rs:153-171 の
   find_config_path() が resolved.exists() で落ち、
   「Config file not found. Place config.toml next to the executable」で bail。
   crates/awase-windows/src/main.rs:49 がこれを拾ってエラーダイアログを出す。
   **アプリが起動しない。復旧手段はレジストリ手編集（一般ユーザーには不可能）
   か、MSIの外で config.toml を手で用意すること。**
```

現状（Permanent無し）はアンインストールで KeyPath 値も消えるため、この手順4は
正常に動く。つまり **この不具合は本ADRが新規に作り込むもの**であり、しかも
`Permanent` の不可逆性ゆえに「次のバージョンで直す」ことができない
（Permanent を外した MSI を出しても、既に登録済みの extra system client は
残るため、ユーザー環境の挙動は変わらない — 選択肢A節が引用している
[MS Q&A 1602667](https://learn.microsoft.com/en-gb/answers/questions/1602667/recommended-way-to-uninstall-a-file-that-was-confi)
の「No recommended solutions for uninstalling a file that was configured as
'Permanent' once」）。

さらに悪いことに、ADRの動機として挙げられている「一度アンインストールして
入れ直してください」という不具合対応の定型案内（ADR本文 34-37行目）は、
手順3を挟まなくても近い罠を踏む: 「config.toml を消して初期状態で試して
ください」→ 再インストール／修復しても config.toml が戻らない。

**要求**: 以下のいずれかを決定に織り込むこと。
- (a) **選択肢A を採らない**（M5 の選択肢D を採る）。
- (b) Permanent を採るなら、**同じPRで awase.exe / awase-settings.exe 側に
  「config.toml / layout/*.yab が無ければ埋め込み既定値から生成する」自己修復を
  必ず入れる**（`include_str!` で `config.toml` 3,629 bytes + `layout/*.yab` 6本
  計 13,854 bytes ≒ 18KB。バイナリサイズ的にも実装量的にも小さい）。この場合
  MSIがファイルを配置しなくても実害が消える。ただし自己修復を入れるなら、
  そもそもMSIがこれらを所有する理由自体が消える（→ M5）。

---

### B2. 「既存インストール済みユーザーに `Permanent` が後から効くか」が未検証で、効かない場合この変更は目的を達成しない（しかも唯一の救済策がデータ破壊操作）

ADR本文は不可逆性（Permanent="no" に戻しても効かない）は明記しているが、**その逆**
——「Permanent 無しでインストール済みの環境に、Permanent 付きMSIをアップグレード
適用したとき Permanent が効くようになるのか」——には一切触れていない。これは
**本ADRの受益者の大半（v1.20.1 以前のMSIでインストール済みの既存ユーザー）に
効果があるかどうか**を決める前提であり、未検証のまま「決定」に進むべきでない。

**両論ある**:

- 効く側の根拠: Permanent の実体は「インストール時に extra system client を
  レジストリへ登録する」こと（[Advanced Installer / MS Q&A](https://learn.microsoft.com/en-gb/answers/questions/1602667/recommended-way-to-uninstall-a-file-that-was-confi)
  「the installer does not remove the component during an uninstall and registers
  an **extra system client** for the component」）。登録は
  [ProcessComponents action](https://learn.microsoft.com/en-us/windows/win32/msi/processcomponents-action)
  が **そのトランザクションのパッケージの Component テーブル属性**に基づいて行う。
  `NeverOverwrite` でファイル配置が省略されても「the installer registers the
  component as being installed」なので、client 登録自体は走るはず。加えて本リポジトリは
  `Schedule="afterInstallExecute"`（`wix/main.wxs:17-18`）なので、新製品の
  ProcessComponents が旧製品の RemoveExistingProducts より **先**に走る。
- 効かない側の根拠: InstallSite の "Permanent components removed during major
  upgrade" スレッドの結論として流通している言明
  「marking them as permanent in the new version is **not sufficient** — they
  should have been marked as permanent in the original installation」
  （http://forum.installsite.net/index4b63.html?showtopic=20732 、
  本レビュー時点でTLSエラーにより直接取得できず、検索結果の要約経由）。

**効かなかった場合に何が起きるか**: 既存ユーザーは新MSIにアップグレードしても
保護されない。保護を得る唯一の手段は「一度アンインストール（＝今まさに問題に
している、設定を全部消す操作）してから新規インストールする」こと。本ADRが
防ごうとしている被害を、本ADRの恩恵を受けるために一度踏まねばならない、という
catch-22 になる。ADRはこの可能性を評価も記載もしていない。

**要求**: 実装前に、実機で以下を確認し結果をADRに書くこと（M1のテスト計画に統合）。
1. v1.20.1 のMSI（Permanent無し）をインストール。
2. 新MSI（Permanent付き）で上書きアップグレード。
3. `HKCU\Software\Microsoft\Installer\Components\<packed GUID>` 配下に
   `00000000000000000000000000000000` という名前の値が出現したかを確認
   （※パスとGUID表記については M2 参照）。あるいは `msiexec /i ... /l*v` の
   verbose ログで対象 ComponentId の ProcessComponents 登録行を確認する。
4. アンインストールして `config.toml`/`layout/` が残るかを確認。

効かないことが判明した場合、選択肢Aは「新規インストールユーザーにしか効かない
不可逆な変更」となり、採用の合理性が大きく下がる（→ M5 の選択肢Dが相対的に優位）。

---

## Major

### M1. テスト方針が false green を出す設計になっている（B1もB2も検出できない）

現行テスト方針（ADR 134-146行）の手順4は「同じMSIを再インストールし、
NeverOverwriteにより手順1で編集した内容が保持されたまま起動することを確認」。
これは **ファイルがディスクに残ったままの再インストール**なので、B1の
「NeverOverwrite がコンポーネント配置をスキップする」挙動は観測されない
（ファイルは元から在るので、配置されなくても中身は正しい）。つまりB1が
実在してもこの手順は「合格」する。B2（アップグレード経路）は手順に存在しない。

**追加すべき手順**:
- 2': アンインストール直後に `HKCU\Software\awase` を確認し、7つの値
  （ConfigFile/NicolaYab/NicolaKeytopYab/NicolaUsYab/NicolaFYab/NicolaKb232Yab/
  NicolaKakuteiYab）が残っていること／いないことを記録する。
- 3': アンインストール後に `%LOCALAPPDATA%\awase` を**手動で完全削除**してから
  MSIを再インストールし、`config.toml` と 6本の `.yab` が実際に配置されるかを確認
  （B1の直接再現手順）。
- 5: v1.20.1 → 新MSI のアップグレード経由でも Permanent が効くか（B2の手順）。
- 6: ARPの「修復」（`msiexec /f`）で、手で消した `config.toml` が復元されるか
  （→ M7）。

### M2. ADRが書いているレジストリパスが perUser インストールでは誤り。実機確認で false negative を生む

ADR 59-63行は
`HKEY_CURRENT_USER\...\Installer\UserData\...\Components\<Component GUID>` と書く。
しかし引用元（MS Q&A）が示しているのは **per-machine** の
`HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData\S-1-5-18\Components\...`
である。本製品は `InstallScope="perUser"`（`wix/main.wxs:9`）であり、
非managed の per-user インストールのコンポーネント登録先は
`HKCU\Software\Microsoft\Installer\Components\<packed GUID>` である
（`UserData` 階層を挟まない）。

さらに、キー名は **packed / squished GUID**（32文字、ハイフン無し、各フィールドを
反転した表現）であって `wix/main.wxs` に書かれている GUID 文字列そのままでは
検索にヒットしない（[Packed GUIDs, Darwin Descriptors and Windows Installer
Reference counting](https://installpac.wordpress.com/2008/03/31/packed-guids-darwin-descriptors-and-windows-installer-reference-counting/)）。

実害: 実機確認担当が ADR のパスと生GUIDで `reg query` して「何も無い →
Permanent は記録されていない」と誤結論する。ADR-177 の検証で実際に
「API/表示を信じて誤判断しかけた」前例がある領域なので、パスは正確に書くこと。
確認手段としては packed GUID への変換か、`/l*v` ログでの ComponentId 検索を推奨。

### M3. ADRが提示する「完全削除」手順が不完全で、そのまま案内するとB1を確実に踏ませる

ADR 120-124行は「アンインストール後に手動で `%LOCALAPPDATA%\awase` フォルダを
削除してください」と案内する予定としている。しかしB1の通り、これだけでは
- `HKCU\Software\awase` の7値（Permanent化されたKeyPath）
- `HKCU\Software\Microsoft\Installer\Components\<packed GUID>` の extra system client

が残り、**しかもその残骸が次回インストールを壊す**。ZIP版 `-Purge`
（`scripts/uninstall.ps1:25-30`、`Remove-Item -Recurse -Force $installDir` の1行）
が完結した操作であるのに対し、MSI版の「同等手順」は一般ユーザーが完遂できない
レジストリ操作を含むことになる。ADRの「対称性を取るのが目的」（80-82行）という
主張と正面から矛盾する。

最低限、案内文には `Remove-Item -Path 'HKCU:\Software\awase' -Recurse` 相当を
含める必要があり、それを案内するくらいなら purge スクリプトを同梱すべき（→M4）。

### M4. 選択肢Bの棄却理由が、検討していない別のカスタムアクション案まで巻き添えで棄却している

ADR 71-82行は「カスタムアクション＝退避→復元」と同一視して棄却している。しかし
`-Purge` 相当を取り戻す手段としては、はるかに単純な
**「削除専用・明示オプトインのカスタムアクション」** がある:

```
msiexec /x {ProductCode} PURGE=1
```
に対して、`InstallExecuteSequence` の `RemoveFiles` 付近で条件
`REMOVE="ALL" AND PURGE=1` の deferred CA（または `RemoveFile` テーブル行）で
`%LOCALAPPDATA%\awase` を削除する。選択肢Bと違い「復元」経路が存在しないため、
「退避・復元自体が失敗してデータを完全に失う」というADRが挙げた棄却理由
（77-80行）は当てはまらない。破壊はユーザーが明示的に要求したときだけ起きる。

**重要**: `Permanent` は静的属性であり **条件付きにできない**。したがって選択肢Aを
採用すると、この purge 経路は「Permanent が優先されて消えない」ため
後から実装できなくなる（CAで直接 `Remove-Item` するなら可能だが、それは
MSIのコンポーネント管理の外で削除することになり、B1のレジストリ残骸は残る）。
**選択肢Aは「後から完全削除手段を足す」道も塞ぐ**、という不可逆性の二重性を
ADRは評価していない。

なお、最も安価な代替は「`scripts/uninstall.ps1 -Purge` 相当の `purge.ps1` を
MSIにも同梱し、ドキュメントで案内する」であり、これは選択肢Aとも併用できる
（ただしB1のレジストリ残骸の掃除も含める必要がある）。

### M5. 未検討の選択肢D:「ユーザーデータをMSIの管理下から外す」——不可逆性もNeverOverwriteも不要になる

ADRが検討したA/B/Cはいずれも「MSIが `config.toml`/`*.yab` を所有し続ける」前提を
共有している。その前提自体を外す案が検討されていない:

> **選択肢D**: MSIは `config.toml`/`layout/*.yab` を**インストールしない**
> （あるいは `%LOCALAPPDATA%\awase\template\` のようなプログラム資産側に
> インストールする）。`awase.exe` は起動時に `config.toml`/`layout/*.yab` が
> 無ければ、埋め込み既定値（`include_str!`）またはテンプレートからコピーして生成する。

利点:
- MSIがそれらのファイルを所有しないので、**アンインストールで削除されない**
  （ADRの目的を達成）。`Permanent` 不要 → **不可逆性の問題が丸ごと消える**。
- `NeverOverwrite` も不要になる → ADR-099 決定0 が抱えている
  「GUID不変 + NeverOverwrite + `Schedule="afterInstallExecute"` の3点を
  `wix_installer_guard.rs` で固定し続ける」という保守負債が縮む
  （`.claude/rules/complexity-budget.md` の精神＝加算より減算、に合致する
  唯一の案でもある）。
- `-Purge` 相当は「フォルダを消すだけ」で完結し、消した後の再インストールも
  正常に動く（アプリが再生成するため）。B1もM3も同時に解決する。
- 実装コスト: 埋め込み対象は `config.toml` 3,629 bytes + `layout/*.yab` 6本
  13,854 bytes ＝ 約18KB。現状 `find_config_path()`
  （`crates/awase-windows/src/app/mod.rs:153-171`）は「無ければ bail」なので、
  ここに生成処理を足すのが主な差分。ZIP版との整合も取りやすい。

留意点（採らない場合の反論材料になりうる）:
- 新しい既定配列を追加したとき（例: 2026-09-13 の `nicola_kakutei.yab`）、
  既存ユーザーに配るには「起動時に無ければ生成」がそのまま効くので、
  現状の NeverOverwrite 挙動と実質同じ。
- `awase-settings.exe` 単体起動時の生成責務をどちらが持つか決める必要がある
  （`crates/awase-settings/src/main.rs:5290` の `find_config_path()` も同様）。

**要求**: 選択肢Dを検討済み選択肢として明記し、採るか、採らない理由を書くこと。
B1/B2を踏まえるとDが最有力に見える。

### M6. `Permanent` は7つのコンポーネントGUIDを「現在のKeyPathに永久に固定」する。将来のインストール先変更が詰む

Permanent 化後は、当該 GUID が `HKCU\Software\awase\<Name>` という KeyPath に
extra system client 付きで恒久登録される。将来、

- perUser → perMachine への変更、
- `%LOCALAPPDATA%\awase` → `%APPDATA%\awase` への移動、
- レジストリ KeyPath の名前変更、

のいずれかを行う場合、コンポーネント規則上は新GUIDが必要になる。ところが
新GUIDのコンポーネントが **同じ KeyPath 名**（`Software\awase\ConfigFile` など）を
使う限り、旧 Permanent コンポーネントが残した値のせいで新コンポーネントも
「既にインストール済み」と判定され（NeverOverwrite）、B1と同型の
「ファイルが配置されない」に陥る。つまり Permanent はGUIDだけでなく
**KeyPath の名前空間ごと凍結する**。ADRはこれに触れていない。

併せて、`crates/awase-windows/tests/wix_installer_guard.rs:111-120` の
`known_component_guids_are_unchanged` は `NicolaKb232Yab` と `NicolaKakuteiYab` を
含んでいない（既存の抜け）。Permanent化でGUID変更のコストが「回帰」から
「不可逆な破損」に格上げされるので、この機会に7件すべてをGUIDガードに含めること。

### M7. 「修復（`msiexec /f` / ARPの修復）」はユーザーデータの復旧経路にならない、と明記すべき

`NeverOverwrite` + レジストリ KeyPath 構成のため、ユーザーが `config.toml` を
誤って消しても修復では戻らない（KeyPath が存在する＝健全と判定される）。これは
Permanent 以前からの既存挙動だが、B1 により「アンインストール後も KeyPath が
残り続ける」ようになるため、修復が効かない期間が恒久化する。サポート案内で
「修復してみてください」と言えないことをADRに書いておくべき（ADR-177 が
`msiexec /f` を検証項目に挙げている以上、読み手は修復が効くと誤解しやすい）。

---

## Minor

### m1. 「完全削除手段の喪失」の重大さ評価が、現状認識の誤りの上に立っている

ADR は暗黙に「現状のMSIアンインストールは `%LOCALAPPDATA%\awase` を完全に消す」と
前提しているが、実際には MSI 管理外のファイルが既に残る:
- `cache.toml`（IME capability 学習キャッシュ。`scripts/uninstall.ps1:34-41` が
  ZIP版で意図的に残すと明記しているもの）
- `awase.log`
- `config.toml.bak`（`crates/awase-settings/src/main.rs:804-806` が初回保存時に作る）

したがって `RemoveFolder Id="RemoveInstallDir"` は現状でも「空でない」ため失敗し、
フォルダは残っている。「MSIなら完全に消える」という対比は元々成立していないので、
選択肢Cの評価と「完全削除手段の喪失」の重大度評価にこの事実を1行足すと、判断の
根拠が正確になる（結論は変わらないかもしれないが、根拠が誤っているのは残さない方がよい）。

### m2. `RemoveFolder Id="RemoveLayoutDir"` の挙動（ADR 102-105行「要実機確認」）はMSI仕様上ほぼ確定している

`RemoveFile`/`RemoveFolder` テーブルの行は所属コンポーネントに紐づき、`RemoveFiles`
アクションは **そのコンポーネントの action state が削除方向のとき**にだけ処理する。
Permanent コンポーネントはアンインストール時に削除方向へ遷移しない（extra system
client が残るため参照が0にならない）ので、`RemoveLayoutDir` は**実行されない**。
「実行されるがディレクトリが空でなく失敗する」経路にはならない。ADRの推測
（「実行されなくなる可能性が高い」）は正しいので、「要実機確認」から
「仕様上そうなる（実機でも確認する）」に格上げしてよい。なお ICE64 は
`RemoveFile` テーブルに行が存在するかどうかだけを見るので、行が実行されなくなっても
ビルド時検証は通る。

### m3. 「ドキュメントへの追記」の追記先が存在しない／英語版が漏れている

ADR 120-124行は「`docs/index.html`（アンインストール手順を案内している箇所）」と
書くが、`docs/index.html` にアンインストール手順の節は無い（`uninstall.ps1` への
言及が643行目に1箇所あるだけで、それも「ZIP版アップグレード時に uninstall.ps1 を
先に実行する必要はない」という文脈）。`README.md`/`README.en.md` にも
アンインストールの記述は無い。また `docs/index.en.html` が存在するので英語版も
対象になる。追記先を新設するのか既存節に足すのかを決めてから書くこと。

### m4. 回帰テストの assert メッセージは「消しても既存環境には戻らない」ことを書くべき

ADR 126-132行が追加予定の `Permanent="yes"` 固定テストは、既存の
`config_file_and_nicola_yab_components_have_never_overwrite`
（`crates/awase-windows/tests/wix_installer_guard.rs:86-104`）と同型でよいが、
メッセージの趣旨が他と異なる: GUID や `Schedule` は「消すと次のアップグレードで
壊れる」ものだが、`Permanent` は **消しても既に出荷済みの環境の挙動は変わらず、
新規インストール環境だけが別挙動になる**（＝環境間で挙動が分岐する）。
テストが守っているものが何かを正確に書かないと、将来「Permanent を外しても
実害が無さそうだから外す」という判断を誘発する。

### m5. `related_adr` に ADR-158 系（複雑性予算）への言及があってよい

`.claude/rules/complexity-budget.md` は未発効なので強制ではないが、本ADRは
「不可逆な宣言を1つ増やす」変更であり、選択肢D（M5）は逆に既存の宣言
（NeverOverwrite × 7 + GUID固定 × 7 + Schedule固定）を減らせる可能性がある。
どちらを選ぶかの判断材料として1行触れておく価値がある。

---

## 参考にした一次情報

- [Installing Permanent Components, Files, Fonts, Registry Keys — Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/msi/installing-permanent-components-files-fonts-registry-keys)
- [Recommended way to uninstall a file that was configured as "Permanent" once — Microsoft Q&A](https://learn.microsoft.com/en-gb/answers/questions/1602667/recommended-way-to-uninstall-a-file-that-was-confi)
- [ProcessComponents Action — Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/msi/processcomponents-action)
- [KeyPaths explained — FireGiant Support Center](https://support.firegiant.com/hc/en-us/articles/230912347-KeyPaths-explained)
- [Packed GUIDs, Darwin Descriptors and Windows Installer Reference counting](https://installpac.wordpress.com/2008/03/31/packed-guids-darwin-descriptors-and-windows-installer-reference-counting/)
- [Component Properties — Advanced Installer](https://docs.advancedinstaller.com/component-properties.html)
- Permanent components removed during major upgrade — InstallSite Forum
  （http://forum.installsite.net/index4b63.html?showtopic=20732 、本レビュー時点で
  TLSエラーにより直接取得不可。検索結果の要約経由のため B2 の根拠としては弱い旨を明記）
