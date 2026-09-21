---
id: ADR-178-companion-178-opus-review-round11
title: |-
  ADR-178（MSIアンインストール時のユーザーデータ保護）Opus敵対的レビュー round11
type: companion-doc
related_adr:
  - "ADR-178"
---

# ADR-178 敵対的レビュー round11（v11）

対象: `docs/adr/178-msi-uninstall-preserve-userdata.md`（v11、コード未実装）
既往: round1〜10（Blocker B1〜B14）

---

## 1. round10指摘の解消確認

| round10 | v11の対応 | 判定 |
| --- | --- | --- |
| **B14** 戻り値に`ConfigLoadState`が無い | `EnsureOutcome{config: Option<AppConfig>, load_state, restore, write_path, guard}`へ拡張（#16、決定1の型定義218-231行、決定2の276-282行、決定5） | **部分解消**。型としては解決したが、`load_state`の扱いが自己矛盾（下記 M3）。さらに`config=None`にしたことで**新しい破壊経路**が開いた（下記 B15） |
| **M1** 絶対`layouts_dir`で「使われないバックアップ」 | #17・契機2・契機3・決定2の書き込み先・「解決されないこと」509-512行・決定7テストに反映 | **解消**。ただし判定対象（生の設定文字列か解決後パスか）が未確定（m2） |
| **M2** `Dangerous`時の復元提案に必要な情報経路が未定義 | #19、決定2ステップ3'（310-314行）、決定5（364-367行）、決定6（398-401行）、決定7テスト | **解消**。ステップ3'を`load_state`の値によらず実行すると明記されており、UI側の再実装禁止も#19に入った |
| **M3** 契機1のパス比較がフェイルサイレント | #18・契機1（188-193行）で比較を廃止し、`EnsureOutcome.write_path`を保存先として直接使う構造に変更 | **不完全**。`AppConfig::save()`しか塞いでおらず、config.tomlを書くもう1つの合流点`AppConfig::save_auto_start()`が対象外（下記 **B16**）。同型のパス比較が契機2にそのまま残存（下記 M1）。開発ビルドでは読み取り先と書き込み先が意図せず食い違う（下記 M2） |
| m1 全バリアント試行の理由 | #12末尾（135-138行）に明記 | 解消 |
| m2 テスト配置分担 | 決定7（410-416行）に書き戻し済み | 解消 |
| m3 `backup_dir`引数 | #2、決定5（380-383行） | 解消 |
| m4 `validate()`二重実行と警告 | #20 | 解消 |
| m5 未解決事項#7 | 「round10以降の記録も同様に」という継続タスクへ書き換え済み（536-537行） | 解消 |

---

## 2. Blocker

### B15. 契機1を「無条件」にしたことで、`load_state != Loaded`のときに**既定値由来のconfigがバックアップを破壊する**。破壊されるのは、決定5がまさに「バックアップから復元しますか？」と提案しようとしている当のファイル

M3対応で契機1は「`AppConfig::save()`成功後、**無条件で**バックアップする」（188行、#18）になった。しかしB14対応で`Dangerous`時は`config = None`を返す設計になったため、`awase-settings`は従来どおり`default_config()`にフォールバックする（決定2の278-280行が明示的にそう指示している）。この2つが噛み合って次が起きる。

再現シーケンス（すべて既存コードの実挙動）:

1. ユーザーが`config.toml`を手編集して壊す → `AppConfig::load`失敗 → `classify_load_error`が`Dangerous`（`src/config.rs:848-857`）。
2. `ensure_user_data_present()`は決定2ステップ2により`config = None`、`.yab`復元と契機3をスキップ。ステップ3'で「バックアップは存在し妥当」と記録（#19）。ここまでは意図どおり。
3. `awase-settings`が起動。`SettingsApp::new`（`crates/awase-settings/src/main.rs:543-551`）は`config = default_config()`で立ち上がる。`default_config()`は`toml::from_str("[general]")`（`main.rs:5473-5475`）＝`GeneralConfig::default()`で、**`layouts_dir = "config"`**（`src/config.rs:477`）。
4. ユーザーが「バックアップから復元しますか？」に気づかず／後で判断しようとして、何か1項目だけ直して「適用」を押す。`main.rs:818`の`clone.save(&config_path)`が**成功する**。
5. 契機1が無条件に発火し、`backup\config.toml`を手順4で保存した内容＝`default_config()`由来の全項目既定値で上書きする。

結果、ユーザーの本物の設定を持っていた唯一のバックアップが消え、決定5のUIはもう提案するものを持たない。B1（MSI再インストール）が起きたときに復元されるのは既定値である。

- 決定1の抑止条件「対象ファイルの内容が現在の埋め込み既定値と一致する場合はバックアップしない」（204-206行）は**効かない**。`default_config()`を`toml::to_string_pretty`した結果はリポジトリの`config.toml`とバイト一致しない（`layouts_dir`が`"config"` vs `"layout"`という差はround10 B14自身が名指しした点）。
- 能力トークンも効かない。`UserDataGuard`は`EnsureOutcome`の一部として`Dangerous`時にも返る（決定1の型定義221-227行に条件が無い）。
- 決定2ステップ4は契機3を`Dangerous`/`FallbackToEmbeddedDefault`でスキップすると明記しているのに、契機1だけ無条件という**非対称**になっている。ステップ4がスキップする理由（信用できない状態の値でバックアップを汚さない）は契機1にもそのまま当てはまる。

同じことが`.yab`側でも起きる（契機2）。`config = None`の状態でも`awase-settings`は配列編集タブを開けるため、`layout_write_to_path()`（`main.rs:758` / `main.rs:1823`）は動く。

**要求**: 契機1・契機2の発火条件に「保存される`AppConfig`が信用できる状態に由来すること」を入れる。具体的には`EnsureOutcome.load_state`が`Loaded`（または復元直後の既知良好状態）である場合に限る、と決定1に書く。無条件にできるのはM3が要求した「パス比較をやめる」部分だけであり、「状態の健全性ゲート」まで一緒に外してはならない。トークンは「復元を通らないプロセス」を止めるものであって「復元は通ったが状態が不明なプロセス」は止めない、と決定1の241-247行が自分で述べているとおり。

### B16. #18/M3が塞いだのは`AppConfig::save()`だけ。config.tomlを書くもう1つの合流点`AppConfig::save_auto_start()`が対象外で、M3が指摘したフェイルサイレントがそのまま残る

`config.toml`への書き込み経路は2つある。

1. `AppConfig::save(&self, path)`（`src/config.rs:890-893`）← #18が対象にしたもの。
2. `AppConfig::save_auto_start(path, value)`（`src/config.rs:910-930`）。内部で`Self::load(path)`→`general.auto_start`を差し替え→`config.save(path)`。呼び出し元は
   - `crates/awase-windows/src/tray.rs:1041-1046`: `crate::app::find_config_path()`の結果を渡す（`crates/awase-windows/src/app/mod.rs:153-167`、CLI引数→`resolve_relative("config.toml")`→存在しなければ`bail!`）。
   - `crates/awase-settings/src/main.rs:2042-2043`: `self.config_path`＝`find_config_path()`（`main.rs:5290-5300`、CLI引数→`resolve_relative_to_exe`→**見つからなければCWD相対の裸パス**）。

`save_auto_start`はチェックリスト#3が「起動後の意図的な再読み込み」として明示的に対象外にしている経路（97行、`src/config.rs:905-909`のdocが理由を書いている）だが、**読み直しを許すことと、書き込み先を野放しにすることは別問題**である。#18は「そのパス以外への保存が構造的に発生しない」と書いているのに、この2経路は`write_path`を一切見ない。

- `awase.exe`側はトレイの自動起動トグルのたびに`find_config_path()`を再解決する。この結果が`EnsureOutcome.write_path`と食い違えば、復元・バックアップの対象ではないファイルに書き、症状は**何も出ない**（M3が指摘した症状そのもの）。
- 決定7のM3回帰テスト（431-434行）は「それ以外のパスへの`AppConfig::save()`が存在しないことをソーススキャンで確認する」と書いており、`save_auto_start`という名前を含まないため**この経路を検出しない**。ソーススキャン型のガードは検出対象の列挙がそのまま仕様なので、ここに載っていない＝永久に見逃す。

この「新しいゲート／単一経路化を1箇所に置いて満足し、実際の合流点が複数あった」という失敗は、`.claude/rules/fix-requires-evidence.md`の「IME actuation 合流点」行が5経路を列挙して警告している型と同一であり、このリポジトリでは実機バグとして繰り返し発生している。

**要求**:
- 決定1・#18の対象を「`config.toml`への全書き込み経路」と定義し直し、`AppConfig::save`と`AppConfig::save_auto_start`の**両方**を列挙する。
- `save_auto_start`にも`write_path`を渡す（`tray.rs::save_auto_start_config`が`find_config_path()`を呼ばず、`APP`側に保持した`write_path`を使う形にする）か、あるいは`save_auto_start`経由の保存はバックアップ対象外と**明示的に**決めた上で「そのときバックアップと実ファイルがずれる窓がどれだけ続くか」（次回起動の契機3まで）を「解決されないこと」に書く。
- 決定7のソーススキャンテストの検出対象に`save_auto_start`を含める。

---

## 3. Major

### M1. M3が消したのと同型のパス比較が、契機2（`.yab`）にそのまま残っている。しかも比較対象はユーザーが任意に選べるパス

契機1は比較を廃止したが、契機2は「保存先が現在の`layouts_dir`配下であり、かつファイル名が同梱6ファイルのいずれかと一致し、かつ`layouts_dir`が相対パスである」（194-197行）と、**配下判定＋ファイル名一致**という比較を残している。round10 M3が「比較方法を決めないと実装者ごとに結論が変わり、不一致時は無警告」と述べた懸念がそのまま当てはまる。むしろ条件は悪い:

- 比較の左辺は`self.layout_file_path`（`crates/awase-settings/src/main.rs:751`）で、これは**ユーザーがファイル選択で任意に指定できる**。既存コードは`main.rs:752`で`path != default_layout_path`という素の`PathBuf`比較を行い、不一致なら「エンジンには反映されません」と案内している＝既にこの比較が挙動を左右している場所である。大小文字・`\\?\`プレフィクス・ジャンクション・相対/絶対の混在に弱い点は契機1と同じ。
- 比較の右辺（`layouts_dir`配下）をどう作るかが未定義。`awase-settings`の`resolve_layouts_dir()`（`main.rs:5306-5308`）は`resolve_relative_to_exe`に委ねるため、**相対の`layouts_dir`を渡しても絶対パスが返る**。一方#17の「`layouts_dir`が相対パスである場合のみ」は生の設定文字列を見ないと判定できない。1つの契機の中で2つの異なる表現（生文字列／解決後の絶対パス）を混ぜており、どちらを使うか書いていない。

不一致時の症状はやはり「何も起きない」。B1修正のうち`.yab`側の実効性がここにぶら下がる。

**要求**: 契機1と同じ構造にする。すなわち「`EnsureOutcome`が示す`.yab`の書き込み先（`exe_dir.join(layouts_dir_raw).join(名前)`、`layouts_dir_raw`が相対のときのみ存在）の集合を戻り値で渡し、`layout_write_to_path`の保存先がその集合の要素として**選ばれた**場合にのみバックアップする」と書く。比較を残すなら、左辺・右辺の作り方と正規化方法（`canonicalize`失敗時は警告ログ＋バックアップしない）を明記すること。

### M2. #18により開発ビルドで**読み取り先と書き込み先が恒常的に食い違う**。`cargo run`で設定を1回保存すると、以後リポジトリの`config.toml`の編集が静かに無視される

`src/paths.rs:34-62`の解決順は「絶対→exe隣に**存在すれば**exe隣→`target`祖先のワークスペースルートに存在すればそこ→CWD相対」。開発ビルド（`exe_dir = target/debug`）では通常ワークスペースルートの`config.toml`が読まれる（決定7のB12テストが確認しようとしているのはこの挙動、459-461行）。

ところが#18は保存先を無条件に`EnsureOutcome.write_path`＝`exe_dir.join("config.toml")`にする。決定2の「開発ビルドの除外」（284-286行）が無効化するのは**復元と救済**だけで、`write_path`の決め方には触れていない。したがって開発ビルドでは:

1. 読み取り: `<workspace>/config.toml`
2. `awase-settings`で何か保存: `<workspace>/target/debug/config.toml`が新規作成される
3. 次回起動: `paths.rs:38-43`により**exe隣が優先**されるので、以後`target/debug/config.toml`が読まれる。ワークスペースルートの`config.toml`（リポジトリ追跡対象、開発者が手編集する当のファイル）は二度と読まれない。

「リポジトリのトラッキング済みファイルを汚染しない」（497-498行）という主張自体は保たれるが、開発者から見ると「`config.toml`を編集しても効かない」という無警告の挙動変化になる。これは決定2冒頭の「読み取り先と書き込み先は別物であり…」（#11）が本来防ごうとしていた事故の、方向を逆にしたものである（B2/B5/B6/B12に続く同ファミリーの6回目）。

**要求**: `write_path`の導出にも開発ビルド分岐を入れる（`target`祖先が見つかる場合は従来の解決結果を`write_path`とする）か、#18の適用範囲を「復元機構が有効な環境に限る」と明記する。決定7のB12テストに「保存先も読み取り先と一致すること」を追加すること。

### M3. `ConfigLoadState`の扱いが決定2の中で自己矛盾している。既存型に第4バリアントを足すと、既存の3箇所の`Dangerous`判定が新状態を「安全」と解釈する

決定2の同じ節の中に両立しない2文がある。

- 299行（ステップ2）: 復元書き込みが失敗しインメモリ既定値で継続する場合、「`load_state`を**専用の値**（例: `FallbackToEmbeddedDefault`）にして区別できるようにする」。
- 280-282行: 「`ConfigLoadState`は既存型（`src/config.rs`の`classify_load_error`が返す分類、`Loaded`/`NotFound`/`Dangerous(reason)`相当）を**そのまま使う**」。

専用の値を足すことは「そのまま使う」ことと両立しない。加えて実害がある。`ConfigLoadState`は`src/config.rs:829-838`の**3バリアントenum**（ADR 220行は`pub struct ConfigLoadState { .. }`と書いているが誤り）で、`awase-settings`側の分岐はすべて`matches!(..., Dangerous(_))`型の非網羅判定である:

- `main.rs:786` 保存前の`config.toml.bak`退避
- `main.rs:911`、`main.rs:1021`
- `main.rs:1124` `if let Dangerous(reason)` のUI表示

第4バリアントを足してもコンパイルは通るため、**静かに「Dangerousではない＝安全」として扱われる**。`FallbackToEmbeddedDefault`は「ディスク上の`config.toml`が書けなかった」状態なので、`.bak`退避なしの保存＋B15の無条件バックアップと合流すると被害が重なる。

**要求**: (a)`ConfigLoadState`は触らず`EnsureOutcome`側に別フィールド（例: `used_embedded_fallback: bool`）を置く、(b)新バリアントを足した上で既存4箇所をどう扱うかを決定文に列挙する、のどちらかを選んで書く。あわせて220行の`struct`→`enum`を訂正すること。

### M4. 「実機検証結果」の結論が、採取した証拠より1段強い。証拠が届いているのは「MSI内蔵ファイル」までで、「`include_str!`の埋め込み既定値」までは届いていない

採取された決定的証拠は「再インストール後の`config.toml`のSHA256 == MSIパッケージ内蔵の`config.toml`のSHA256」（43-44行）。これが証明するのは**MSIが出荷時ファイルを再配置すること**であって、そのファイルがコア`awase`の`include_str!("../config.toml")`とバイト一致することではない。復元条件（322-324行）が要求するのは後者である。

橋渡しは現状2つとも未確定:

- `.github/workflows/release.yml:66`の`cp config.toml dist/`という手続きがあるだけで、これを検証するCIステップは決定3が「追加する」と書いた**未実装**のもの（346-347行）。
- `.gitattributes`は`crates/awase-windows/tests/golden/**`しか`eol=lf`で固定しておらず、`config.toml`・`layout/*.yab`は未固定。Windowsチェックアウト（git既定`core.autocrlf=true`）で改行が変わる経路は実在する。決定3の「改行正規化＋BOM除去」がこれを吸収する設計だが、**今回の実機検証はそれを確認していない**（同一マシン・同一チェックアウトから作った`dist/`と`include_str!`を比べているので、この軸では常に一致する）。

「分からないことを推測で安全策に倒さない」という今回の方針に照らすと、46-54行の結論文は「実機で確定した」範囲を超えている。

**要求**: 結論を2段に分ける。(1)実機で確定＝「アンインストールで消え、再インストールで**出荷時ファイル**が再配置される」。(2)未確定＝「出荷時ファイル == 埋め込み既定値（正規化後）」。(2)は決定3のCIステップで担保する、と明記する。あわせて`config.toml`・`layout/*.yab`を`.gitattributes`で`eol=lf`固定するかどうかを未解決事項に足すこと。

### M5. 設計の生死を分けるもう1つの前提「`<exe_dir>\backup\`が`msiexec /x`を生き残る」が、実機検証節にもADR本文にも書かれていない

B1（出荷時ファイルで再配置される）が真でも、`backup\`がアンインストールで消えるなら本ADRの機構は全損する。今回MSIを手元でビルドして検証したのだから、同じ回で`Test-Path`1回で採れた項目である。

静的証拠は揃っている（ADRに書かれていないだけ）:

- `wix/main.wxs:104` `RemoveFolder Id="RemoveInstallDir" Directory="INSTALLDIR"`、`:138` `RemoveLayoutDir`、`:199` `RemoveDataDir`、`:216` `RemoveAppFolder`。いずれも`RemoveFolder`であり、`util:RemoveFolderEx`（再帰削除）は無い。`RemoveFolder`は**空のときだけ**削除する。
- 実測でも`%LOCALAPPDATA%\awase`に`awase.log`・`awase-settings.log`・`cache.toml`が残っている（35-36行）。MSI管理外ファイルが残るという同じ理屈が`backup\`にも適用される。
- `INSTALLDIR`は`LocalAppDataFolder\awase`（`wix/main.wxs:59-63`）＝per-userなので、実行時に`exe_dir`配下へ書ける。これも本設計の前提だが未記載（per-machineインストールへ変えた時点でバックアップ書き込みが権限で失敗する）。

**要求**: 上記の根拠をADRに1段落で残し、実装後の実機確認（538-540行）に「`msiexec /x`後に`backup\`とその中身が残存すること」の採取を加える。

---

## 4. Minor

- **m1.** `AppConfig::validate()`は`self`を消費する（`src/config.rs:1309`）。#3・#16が要求する「`validate()`前の生の値」を`EnsureOutcome.config`として返すには、関数内部で`validate()`前に`clone`が要る。`AppConfig: Clone`であることは既存コード（`main.rs:781`の`self.config.clone()`）で確認済み。1行書いておけば実装時に迷わない。
- **m2.** #17の「相対パス」判定は、生の設定文字列`general.layouts_dir`に対して行うと明記すること。`resolve_layouts_dir()`（`crates/awase-settings/src/main.rs:5306-5308`）は相対入力に対しても絶対`PathBuf`を返すため、解決後の値で判定すると条件が恒常的に偽になり、`.yab`のバックアップ・復元が**一度も動かないまま無警告**になる（M1と同じフェイルサイレント）。
- **m3.** `SettingsApp`を構築しない早期リターンは`--bug-report`（`main.rs:344`）だけでなく`--check-update`（`:348`）・`--scancode-map`（`:356`）の計3つある。#1は「`--bug-report`等」で包含しているが、3つとも列挙したほうが実装漏れが減る。特に`--scancode-map`は昇格プロセス（ADR-111決定4）なので、ここで`ensure_user_data_present()`が動くと昇格した権限でユーザーデータを書くことになる。
- **m4.** #1の「`SettingsApp::new`の直前」は、実際には`startup_failure::run_with_fallback`のクロージャ内（`main.rs:382-384`）を指すことになる。クロージャの外（`run_with_fallback`呼び出し前）に置くか中に置くかで、GUI起動に失敗したときに復元が走るかどうかが変わる。どちらかを決めて書くこと。
- **m5.** 決定7のB14テスト（421-425行）の「読み取り権限を奪ったファイル」は、Linux CIがrootで動く環境では`PermissionDenied`にならず再現しない。選択肢として併記されている「壊れたTOML」を既定にすること。
- **m6.** 未解決事項#2（`RestoreOutcome`の正確な型）は、#19・決定5・決定6・決定7でユースケース（ファイル単位の復元結果＋バックアップ利用可否）が固まった。「実装時に確定」と性格を書き換えておくと、次に読む人が未決事項と誤解しない。

---

## 5. 総評 — round10の「次ラウンドは確認のみで収束」は成立しない

round10のB14・M1・M2は確かに解消され、実機検証を先行させた判断は正しかった（B1という最大の前提が推測でなく事実になった価値は大きい）。一方で**M3の反映が不完全で、かつB14の反映が新しい破壊経路を開いた**ため、もう1ラウンド必要である。

今回のBlocker2件は性格が対照的で、どちらも「v11で初めて具体化した」ものである。

- **B15**は、M3対応（契機1を無条件化）とB14対応（`Dangerous`時に`config=None`）という**独立した2つの修正が交差した箇所**に生まれた。片方だけ見ていると見えない。決定2ステップ4が契機3を`Dangerous`でスキップしているのに契機1だけ無条件、という非対称がシグナルだった。
- **B16**は、M3が「比較をやめて単一経路にする」と決めたのに、その単一経路の定義を`AppConfig::save()`という関数名1つで済ませたために、`save_auto_start`という2つ目の書き込み合流点が漏れたもの。このリポジトリが`.claude/rules/fix-requires-evidence.md`で5経路を列挙して警告している失敗型と同一で、決定7のソーススキャンテストもその名前を含まないため自動検出もされない。

Majorは5件だが、M1・M2はいずれも「読み取り先と書き込み先」ファミリーの再出現（通算6回目）であり、M3対応の適用範囲を`.yab`側と開発ビルドへ広げれば同時に閉じる。M4・M5は実機検証節の書き方の問題で、**証拠の届く範囲を1段狭く書き直すだけ**——採取済みデータで足りる。M3（`ConfigLoadState`）は決定文2文の矛盾なのでどちらかを選ぶだけ。

いずれも数行〜1段落で閉じる範囲であり、設計の骨格を変える必要はない。**B15・B16と Major 5件を反映したv12であれば、次ラウンドは確認のみで収束し実装フェーズへ進める**と見る。ただし今回2ラウンド連続で「型が具体化した瞬間に新しいBlockerが見えた」ため、v12では実装着手前にもう1度だけ確認ラウンドを回すことを勧める。
