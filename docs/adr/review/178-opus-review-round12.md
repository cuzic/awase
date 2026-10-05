---
id: ADR-178-companion-178-opus-review-round12
title: |-
  ADR-178（MSIアンインストール時のユーザーデータ保護）Opus敵対的レビュー round12
type: companion-doc
related_adr:
  - "ADR-178"
---

# ADR-178 敵対的レビュー round12（v12）

対象: `docs/adr/178-msi-uninstall-preserve-userdata.md`（v12、697行、コード未実装）
既往: round1〜11（Blocker B1〜B16）
本ラウンドの新規: **Blocker 2件（B17・B18）／Major 6件／Minor 6件**

判定は末尾「5. 総評」。結論だけ先に書くと、**「確認のみで収束」の水準には達していない**。
B17は round11 B15 と同じ結末（唯一の正しいバックアップが既定値相当の内容で破壊される）
へ到る経路が v12 にも2本残っており、B15対応は実質未完である。B18は決定1の中核である
能力トークンの型仕様が、Rust の可視性規則上、意図した保証を1つも与えない。

---

## 1. round11指摘の解消確認

| round11 | v12の対応 | 判定 |
| --- | --- | --- |
| **B15** 契機1無条件化＋`Dangerous=None`の交差 | #21・決定1契機1/契機2に`load_state == Loaded`ゲート追加 | **不完全**。ゲートの主語が未定義で、最も自然な実装点では B15 の再現シーケンスがそのまま通る。さらに「復元が発火したが書き込みに失敗した」状態を捕捉しない（下記 **B17**） |
| **B16** `save_auto_start`漏れ | #18・決定1契機1・決定5（510-518行）・決定7（569-572行、578-579行） | **解消**。HEADで`config.toml`への書き込み合流点が`AppConfig::save`（`src/config.rs:890`）と`AppConfig::save_auto_start`（`:910`）の2つで全部であること、呼び出し元が`tray.rs:1046`・`main.rs:818`・`main.rs:2043`の3箇所で全部であることを確認した（下記 3.1） |
| **M1** 契機2のパス比較残存 | #22・契機2・`yab_write_paths` | **不完全**。集合の作り方（`exe_dir.join(raw)`）と実際の保存先の作り方（`resolve_relative_to_exe`）が別物で照合が外れうる。かつ「要素であり」（比較）と「要素として選ばれた」（選択）で表現が割れている（下記 **M3**） |
| **M2** 開発ビルドで読み書き食い違い | #23・決定2「読み取り先と書き込み先の分離」 | **不完全**。#23が指す`resolve_relative_to_exe()`はCWD相対の裸パスを返しうるため、#11の「CWD相対の裸パスには決して書き込まない」と正面から衝突（下記 **M1**） |
| **M3** `ConfigLoadState`自己矛盾 | #24・決定2（391-402行）・型定義コメント（326行） | **解消**。`enum`への訂正、新バリアント不追加、`used_embedded_fallback: bool`への分離はいずれも妥当。ただし決定2ステップ2の状態列挙に到達不能な組み合わせが残る（下記 m1、これがB17(b)の誤読源） |
| **M4** 実機検証の結論が証拠より強い | 49-74行で2段分離 | **解消**。ただし要求の後半「`config.toml`・`layout/*.yab`を`.gitattributes`で`eol=lf`固定するかを未解決事項に足す」が未反映（下記 m2） |
| **M5** `backup\`のアンインストール生存が未検証 | 76-95行、追加実機検証 | **解消**。静的根拠も裏取り済み（`wix/main.wxs:104`/`:138`/`:199`/`:216`はいずれも`RemoveFolder`、`util:RemoveFolderEx`は不在） |
| m1 `validate()`が`self`消費 | #25、決定2（384-386行） | 解消（`src/config.rs:1309`で確認） |
| m2 相対判定は生文字列 | #17末尾 | **副作用あり**。判定を生文字列に寄せた結果、パス構築まで生文字列で行う記述になり`..`エスケープが開いた（下記 **M2**） |
| m3 早期リターン3経路 | #1（列挙済み） | 解消（`main.rs:344`/`:348`/`:356`で確認） |
| m4 `run_with_fallback`の外 | #1 | 解消（配置としては正しい）。ただし実装可能性に問題（下記 **M6**） |
| m5 壊れたTOMLを既定 | #26・決定7 | 解消 |
| m6 未解決事項#2の性格 | 684-685行 | 解消 |

---

## 2. Blocker

### B17. `load_state == Loaded`ゲートは B15 を塞いでいない。**「復元が発火したが書き込みに失敗した」**状態と、**ゲートの主語の未定義**という2本の経路が残っている

round11 B15 の本質は「**既定値相当の内容を持ったプロセスの保存が、唯一の正しいバックアップを
上書きする**」であって、「`Dangerous`のときに上書きする」ではない。v12 は後者だけを塞いだ。

#### (a) 復元の書き込みが失敗したとき、`load_state`は`Loaded`のまま・`used_embedded_fallback`も`false`

ADR が最も重視している当のシナリオ（MSIアンインストール→再インストール直後）で成立する:

1. 再インストール直後。`config.toml`は工場出荷値、`backup\config.toml`にはユーザーの本物の設定。
   → 決定2の復元条件2番目（「内容が埋め込み既定値とバイト一致し、かつバックアップが存在し、
   かつバックアップの内容が既定値と異なる」、444-446行）が成立し、**復元が発火する**。
2. 復元の書き込み（`write_atomic`）が失敗する。これは架空の想定ではない——
   `src/config.rs:877-884`の`save()`のdocが「Windowsの`rename`は宛先がAVスキャナ・OneDrive等に
   開かれていると失敗しうる」と明記し、50ms×4回のリトライまで入れている既知の実挙動である。
   MSIインストール直後（`LaunchApplication`で自動起動）は、AVがまさに新規配置ファイルを
   スキャンしている時間帯であり、最も当たりやすい。
3. 決定2ステップ2で`AppConfig::load()`を実行。`config.toml`は**存在し読める**（工場出荷値のまま）
   ので**成功**し、`load_state = Loaded`。インメモリ既定値へ落ちる必要もないので
   `used_embedded_fallback = false`（ADR 330行・418-423行の定義上そうなる）。
4. 決定2ステップ4の契機3は動くが、「対象ファイルの内容が現在の埋め込み既定値と一致する場合は
   バックアップを作成・更新しない」（310-312行）に救われて何もしない。**ここまでは無事**。
5. ユーザーが設定画面を開き、工場出荷値の状態で1項目だけ直して「適用」。保存成功。
   保存された内容は既定値と**バイト一致しない**（1項目変えたため）ので抑止条件は効かない。
6. #21のゲートを評価する: `load_state == Loaded` → **真**。契機1が発火し、
   `backup\config.toml`がユーザーの本物の設定から「工場出荷値＋1項目」へ上書きされる。

結末は round11 B15 と完全に同一（唯一の正しいバックアップが消え、決定5のUIは提案するものを
持たない）。ゲートに選んだ`load_state`が「ディスクが読めたか」しか表さず、「**復元が意図
どおり完了して、いま手元にある値がユーザーの本物の設定だと言えるか**」を表していないことが原因。

#### (b) 決定2ステップ2が`Loaded && used_embedded_fallback == true`を存在する状態として書いている

418-423行は「`load_state`が`NotFound`**または**`Loaded`で、かつ復元書き込み自体が失敗し
インメモリの埋め込み既定値で起動を継続する場合は、`config`にこのインメモリ既定値を入れ、
`used_embedded_fallback = true`にする（`load_state`自体は`NotFound`/`Loaded`のまま変えない）」
と書いている。この列挙を読んだ実装者は`Loaded && used_embedded_fallback`という組み合わせが
存在すると解釈する。そしてその状態で #21 のゲート（`load_state == Loaded`のみ）は**通る**。
(a)と同じ結末になる。

（論理的には`load()`が成功したならインメモリ既定値へ落ちる理由が無いので、この列挙の`Loaded`は
到達不能のはずである——下記 m1。しかし#21が`used_embedded_fallback`を一切参照していないため、
「到達不能だから安全」には依存できない。）

#### (c) ゲートの主語が未定義で、最も自然な実装点では B15 がそのまま再現する

決定1契機1（296-299行）は「`load_state == Loaded`**（または復元確定後の既知良好状態）**」と
書いており、この`load_state`が`EnsureOutcome`の起動時スナップショットなのか、
`awase-settings`が持つ生きた`self.config_load_state`なのかを指定していない。括弧書きの
「復元確定後の既知良好状態」は後者を許すようにも読める。

`awase-settings`で契機1を実装する最も自然な場所は、保存完了を受け取る
`poll_pending_save()`の`Saved`分岐（`crates/awase-settings/src/main.rs:846-874`）である。
そしてその分岐の中、**バックアップを書くならその直前に当たる`main.rs:866`が
`self.config_load_state = awase::config::ConfigLoadState::Loaded;`** である。つまり:

- round11 B15 の再現手順（`Dangerous`→`default_config()`→1項目直して「適用」）を実行すると、
  保存成功の瞬間に`self.config_load_state`が`Loaded`へ書き換わり、
- 直後に評価されるゲートは通り、
- **B15 対応が実装時に丸ごと no-op になる**。

`EnsureOutcome.load_state`（起動時スナップショット、この場合`Dangerous`のまま）を読めと
明示していない限り、実装者がこの罠を踏む確率は高い。ADR-178 がここまで11ラウンドかけて
潰してきた「1箇所だけ直して満足する／判定の主語が曖昧」という型そのものである。

**要求**:
1. ゲートを3条件の連言として書く。(i) **`EnsureOutcome.load_state`（起動時スナップショット）**
   が`Loaded`、(ii) `used_embedded_fallback == false`、(iii) **そのファイルについて復元が
   発火した場合は、その書き込みが成功している**。(iii)のために`RestoreOutcome`に
   ファイル単位の成否（`Restored` / `NotNeeded` / `Failed`）を持たせ、`Failed`ならその
   ファイルのバックアップ契機を全部止める、と決定1・決定2に書く。
2. 「（または復元確定後の既知良好状態）」という含みのある括弧書きを削除する。
3. 「`awase-settings`の`self.config_load_state`（保存成功時に`main.rs:866`で`Loaded`へ
   書き換わる可変フィールド）をゲートの根拠にしてはならない」と名指しで禁止する。
   決定7に、B15テストの前提として「`self.config_load_state`を`Loaded`に書き換えた後でも
   バックアップが発火しないこと」を追加する（現在の576行のテスト文面は、保存完了前の
   状態だけを見ていれば通ってしまう書き方になっている）。

### B18. `UserDataGuard`の型仕様が、Rust の可視性規則上、意図した保証を1つも与えない

#4（145-151行）は「**非公開フィールドのみを持つunit-like struct**」、決定1の型定義（325行）は
`pub struct UserDataGuard { /* 非公開・Copy + Send、unit-like struct */ }`と書いている。
この2つの記述は両立しないうえ、どちらの読み方をしても保証が消える:

- `pub struct UserDataGuard;`（unit-like struct）: 値名前空間に`UserDataGuard`という
  コンストラクタが**structと同じ`pub`可視性**で入るため、他クレートから`UserDataGuard`と
  書くだけで構築できる。
- `pub struct UserDataGuard {}`（フィールド0個のbraced struct）: 構築を阻む非公開フィールドが
  存在しないため、他クレートから`UserDataGuard {}`と書けば構築できる。

「非公開フィールドのみを持つ」が効くのは**フィールドが1つ以上ある**場合だけである。
実装者がこの記述どおりに書くと、決定1の「実行のゲート（能力トークン方式）」（320-322行）、
B9対策、「解決されること」節の「バックアップの書き込みが能力トークンによって**型レベルで
ゲートされ**、復元ステップを通らないプロセスはバックアップを一切汚せない」（649-651行）が
すべて空文になる。しかも**何のエラーも警告も出ない**ため、決定7のB9テスト（593-597行）で
`compile_fail`を選んだ場合にのみ気付ける（「レビュー観点として扱う」を選ぶと永久に気付けない）。

**要求**: 型を`pub struct UserDataGuard(());`（または`pub struct UserDataGuard { _private: () }`）
と具体的に書き、「unit-like」という語を削除する。決定7のB9テストは`compile_fail`を
**必須**にする（この欠陥は`compile_fail`テストが無ければ検出できない種類のものである）。

---

## 3. Major

### 3.1 （先に確認できたこと）B16の列挙は完全

HEADの全ソースを走査した結果、`config.toml`へ書き込む経路は以下ですべてである。v12の列挙に
漏れは無い。

- `AppConfig::save(&self, path)`（`src/config.rs:890-893`）
  - `crates/awase-settings/src/main.rs:818` `clone.save(&config_path)`（ワーカースレッド内）
  - `AppConfig::save_auto_start`内部（`src/config.rs:927`）
- `AppConfig::save_auto_start(path, value)`（`src/config.rs:910-930`）
  - `crates/awase-windows/src/tray.rs:1046`（`crate::app::find_config_path()`経由、`app/mod.rs:153`）
  - `crates/awase-settings/src/main.rs:2043`（`self.config_path`＝`find_config_path()`、`main.rs:5290`）

`awase.exe`側は`AppConfig::save()`を一度も呼ばない（`save_auto_start`のみ）ことも確認した。
また**`save_auto_start`のdocが述べる意図（他プロセスの未保存編集を巻き込まない）は「内容を
ディスクから読み直す」ことについての規約であり、「どのパスを渡すか」とは直交する**ので、
`write_path`固定化はこの意図と矛盾しない（チャットで問われた点への回答）。

### M1. #11 と #23 が同じ値について正反対を命じている（開発ビルドのCWD書き込み）

- #11（166-171行）: 「書き込み（復元）先は常に…**存在に依存しない値**。…**CWD相対の裸パスには
  決して書き込まない**。」
- #23（252-260行）／決定2（372-376行）: 開発ビルドでは`write_path`を「読み取り先と同じ従来の
  解決結果（`resolve_relative_to_exe()`の結果）」にする。

ところが`resolve_relative_to_exe()`の実体（`src/paths.rs:52-63`）は、exe隣にもワークスペース
ルートにも見つからなければ**`PathBuf::from(path)`＝CWD相対の裸パスを返す**（`tracing::warn!`
付きで、コメントが「意図しない場所に新規ファイルを作る典型的な事故の入口」と明記している）。
つまり#23をそのまま実装すると、ワークスペースルートに`config.toml`が無い開発環境（新規
チェックアウト直後、`config.toml`を`.gitignore`していない本リポジトリでは起きにくいが、
`layouts_dir`側では容易に起きる）で、#11が禁じた裸パスが`write_path`になる。
「存在に依存しない値」という#11の要求とも衝突する（`resolve_relative_to_exe`は`.exists()`
分岐そのものである）。

**要求**: #23を具体化する。「`exe_dir`の祖先に`target`がある場合、`write_path`は
`<target の親>/config.toml`（＝ワークスペースルート直下、存在の有無によらず固定）とする。
`resolve_relative_to_exe()`の第4分岐（CWD相対フォールバック）へは決して落とさない」。
決定7のM2再現テスト（583-585行）に「ワークスペースルートに`config.toml`が無い状態でも
CWD相対の裸パスが`write_path`にならないこと」を追加する。

### M2. #17（生文字列で判定）が#22（生文字列でパス構築）へ滑っており、`layouts_dir = "../.."`系でINSTALLDIR外へ書く

round11 m2 が要求したのは「**相対かどうかの判定**を生の設定文字列に対して行う」ことだけ
だった。v12はそこから進んで、`yab_write_paths`の**構築式**まで生文字列にしている
（#22の`exe_dir.join(layouts_dir_raw).join(名前)`、決定2の370-371行も同じ）。

`src/config.rs:1039-1045`の`validate_layouts`は、`layouts_dir`に`..`が含まれる場合に警告を
出したうえで`layouts_dir = "layout"`へ**書き戻す**。したがって:

- アプリが実際に`.yab`を読む先: `validate()`後の`"layout"`（`resolve_layouts_dir`経由）。
- ADRの復元が書く先: `exe_dir.join("../../どこか")` ＝ INSTALLDIRの外。

復元は毎起動、アプリが二度と読まないディレクトリへファイルを撒き続け、症状は無警告。
契機2のバックアップ側も同じ集合を使うため、同じくズレる。

**要求**: #17・#22・決定2で「**判定（相対か絶対か）は生の`general.layouts_dir`、パスの
**構築**は`validate()`後の`layouts_dir`」と分けて明記する。決定7に「`layouts_dir = "../x"`の
config で、`yab_write_paths`が`exe_dir`配下から出ないこと」の再現テストを足す。

### M3. `yab_write_paths`の作り方が、実際の保存先の作られ方と別系統。照合が恒常的に外れる環境がある

`yab_write_paths`の定義は`exe_dir.join(layouts_dir).join(名前)`（決定2の370-371行、#22）。
一方、実際の保存先は次のいずれかで、どれも`resolve_layouts_dir()`＝`resolve_relative_to_exe()`
（`crates/awase-settings/src/main.rs:5306-5308`）を通る:

- `main.rs:716` `default_layout_path`（「適用」時の`.yab`保存先）
- `main.rs:1804` `ensure_layout_loaded`の読み込み先
- `main.rs:1823` `layout_pending_save_as`（ファイルダイアログで選んだ任意パス）

`resolve_relative_to_exe`は「exe隣に**存在すれば**exe隣、無ければワークスペースルート、
それも無ければCWD相対」という存在依存の解決である。ユーザーが`layout`ディレクトリごと
削除した環境・ポータブル運用でCWDが違う環境では、保存先がexe隣以外へ解決される一方、
`yab_write_paths`はexe隣固定なので**集合照合が恒常的に外れ、無警告でバックアップされない**。
round11 M1が閉じようとしたフェイルサイレントの再発である。加えてrfd（`main.rs:1823`経由の
「名前を付けて保存」）が返すパスは大文字小文字や`\\?\`が正規化されうるため、素の`PathBuf`
比較が弱いというM1の指摘そのものが残る。

さらに表現が割れている: 決定1契機2（300-303行）は「保存先が…候補集合の**要素であり**」＝
**比較**、#22（243-248行）は「実際の保存先がこの集合の要素として**選ばれた**場合」＝**選択**。
round11 M1 が要求したのは後者（比較を残すなら正規化方法まで書け）である。

**要求**: 保存側が集合から**選ぶ**構造に一本化する。例: `EnsureOutcome`に
`fn yab_write_path_for(&self, file_name: &str) -> Option<PathBuf>`を持たせ、
`awase-settings`は同梱6ファイルを保存するときは必ずこの関数の戻り値へ書く（ユーザーが
ダイアログで選んだ任意パスはこの関数を通らない＝バックアップ対象外、という切り分けが
構造的に成立する）。決定1契機2の「要素であり」という比較表現を削除する。

### M4. `yab_write_paths`は起動時スナップショットだが、`layouts_dir`はセッション中に編集できる

`layouts_dir`は設定画面の編集可能テキストフィールドである
（`crates/awase-settings/src/main.rs:3576` `ui.text_edit_singleline(&mut self.config.general.layouts_dir)`）。
`EnsureOutcome`は起動時に1度だけ作られるので、ユーザーが`layouts_dir`を変更して「適用」した
後の`.yab`保存先は、起動時に導出した`yab_write_paths`のどの要素とも一致しない。結果、
そのセッション中は`.yab`のバックアップが無警告で止まる（次回起動の契機3まで窓が続く）。
`config.toml`側の`write_path`はexe隣固定なのでこの問題は無く、`.yab`側にだけ生じる非対称。

**要求**: 「`layouts_dir`を変更した保存が成功したら`yab_write_paths`を再導出する」と決定1に
書く（`EnsureOutcome`を可変に持つか、再導出関数を公開する）。採らないなら、この窓を
「解決されないこと」に明記する。

### M5. CLI引数でconfigパスを指定した経路には`write_path`が存在しないのに、決定5とソーススキャンテストがその存在を要求している

#10（160-165行）は「CLI引数でconfigパスが明示されている場合、`ensure_user_data_present()`は
呼ばない」。一方、決定5（510-518行）は「呼び出し元は`write_path`／`yab_write_paths`を保持し
続け、`config.toml`を書く**すべての経路**でこれを使う」、決定7（569-572行）は
「`EnsureOutcome.write_path`由来のパスのみを使い、それ以外のパスへの`AppConfig::save()`
および`AppConfig::save_auto_start()`が存在しないことをソーススキャンで確認する」。

CLI指定時は`EnsureOutcome`が無いので`tray.rs:1042`／`main.rs:2043`は`find_config_path()`に
戻るしかないが、それは決定5の文面上は禁止で、ソーススキャンガードには**違反として検出される**。
仕様とガードが実装不能な状態を要求している。

**要求**: `write_path`を「常に定義される値」にする。すなわち`ensure_user_data_present()`を
スキップする場合も、CLI指定パスを`write_path`に入れた`EnsureOutcome`（復元・バックアップは
無効、`guard`も渡さない）を返す薄い経路を用意する、と決定2に書く。そうすれば
「`write_path`以外へは書かない」という不変条件が全経路で成立し、ソーススキャンも成立する。

### M6. #1（`run_with_fallback`の外で呼ぶ）は、現在の`run_with_fallback`のシグネチャでは素直に実装できない

`crates/awase-settings/src/startup_failure.rs:49-55`:

```rust
pub(crate) fn run_with_fallback(
    app_name: &str,
    viewport: eframe::egui::ViewportBuilder,
    app_creator: impl Fn(&eframe::CreationContext<'_>) -> Box<dyn eframe::App> + 'static,
) -> eframe::Result<()>
```

`Fn`（`FnOnce`ではない）であり、`Rc`に包んだうえで**最大2回**呼ばれる
（`:70` glow版、失敗したら`:94` wgpu版）。したがって`EnsureOutcome`をクロージャへ`move`すると
クロージャが`FnOnce`になりコンパイルしない。`EnsureOutcome`（`AppConfig`・`RestoreOutcome`・
`PathBuf`×N・`UserDataGuard`）を`Clone`にするか`Rc`で包む必要がある。

副次的に、**`SettingsApp::new`が2回走りうる**（glow失敗→wgpu）ことにも注意が要る。決定6の
復元通知が二重に出る、契機3相当の処理を`new`側に置くと二重に走る、といった影響がある。

**要求**: #1に「`EnsureOutcome: Clone`（`AppConfig`・`ConfigLoadState`は既に`Clone`、
`RestoreOutcome`にも`derive(Clone)`が要る、`UserDataGuard`は`Copy`）とし、クロージャ内では
`clone()`して`SettingsApp::new`へ渡す」と1行書く。決定6に「レンダラーフォールバックで
`SettingsApp::new`が2回走りうるため、通知は冪等にする」を足す。

---

## 4. Minor

- **m1.** 決定2ステップ2（418-423行）の「`load_state`が`NotFound`**または**`Loaded`で、かつ
  …インメモリ既定値で起動を継続する場合」のうち`Loaded`は論理的に到達不能（`load()`が
  成功したならインメモリ既定値へ落ちる理由が無い）。B17(b)の誤読源なので`NotFound`のみに
  絞るか、到達不能である旨を明記すること。
- **m2.** round11 M4 の要求の後半——「`config.toml`・`layout/*.yab`を`.gitattributes`で
  `eol=lf`固定するかどうかを未解決事項に足す」——が未反映。実機検証結果の本文（66-70行）で
  触れてはいるが、未解決事項1〜8に項目が無い。現状`.gitattributes:4`は
  `crates/awase-windows/tests/golden/**`のみ。
- **m3.** 契機1の「保存成功後」の定義が関数ごとに違う。`AppConfig::save()`は`Result<()>`だが、
  `AppConfig::save_auto_start()`は`Option<Vec<String>>`で、`None`は「**読み込み失敗**または
  保存失敗」の両方を意味する（`src/config.rs:905-909`のdocが「空の`Vec`と区別するため
  `Option`にしてある」と明記）。決定1に「`save_auto_start`は`Some(_)`のときのみ契機1」と書く。
- **m4.** `save_auto_start`は`config.validate()`を通した**正規化後**の値を保存する
  （`src/config.rs:922-928`）。したがって自動起動トグル1回で、ユーザーが手編集した生の値
  （範囲外の閾値等）が正規化され、その正規化後の内容がバックアップに入る。ADRの
  「テキストエディタでの手編集も…バックアップへ捕捉される」（644-645行）という表現と
  ずれるので1行注記が要る。
- **m5.** 復元の read-decide-write は**プロセスをまたいで原子的でない**。`write_atomic`は
  単一の書き込みのアトミック性しか保証しない。`awase.exe`と`awase-settings.exe`をほぼ同時に
  起動した場合、A が復元→ユーザーが保存→遅れて B が起動時に採った古い判断で上書き、という
  窓が理論上ある。決定2に「書き込み直前に復元条件を再判定する」の1文を足すのが安い。
- **m6.** `save_auto_start`を`write_path`固定にすると、`tray.rs:1042`の
  `let Ok(config_path) = crate::app::find_config_path() else { ... }`（`config.toml`が
  見つからない旨の専用エラー分岐、`app/mod.rs:165-168`の`bail!`）が消え、代わりに
  `save_auto_start`の`None`（load失敗）に化ける。ユーザー向けメッセージの粒度が落ちるので、
  移行時にメッセージを保つこと。

---

## 5. 総評 — 「確認のみで収束」には達していない。v13でもう1ラウンド要る

v12 は round11 の指摘のうち **B16・M3・M4・M5・m1・m3・m4・m5・m6 を確実に閉じた**。
とくに B16 は、HEAD の全ソースを走査した結果として列挙が完全であることを確認できた
（`AppConfig::save` / `save_auto_start` の2関数、呼び出し元3箇所）。M5 の静的根拠と実機
追試も噛み合っており、設計の2大前提（出荷時ファイルの再配置・`backup\`の生存）は事実に
なった。骨格は正しい。

一方で、**B15 の対応は実質未完である**。v12 が選んだゲート`load_state == Loaded`は
「ディスクが読めたか」しか表しておらず、B15 の本質「既定値相当の内容を持ったプロセスの保存が
唯一の正しいバックアップを壊す」を捕まえられていない。B17(a) の経路——復元が発火したが
`write_atomic`の`rename`が失敗し、ディスクは工場出荷値のまま読める——は、ADR が最重視する
MSI再インストール直後というシナリオそのもので成立し、しかも `src/config.rs` 自身が
「AVスキャナ・OneDriveで`rename`が失敗しうる」と記録している実挙動の上に乗っている。
B17(c) に至っては、`awase-settings` で契機1を実装する最も自然な行の直前が
`self.config_load_state = Loaded;`（`main.rs:866`）であり、B15 対応が実装時に no-op 化する
配置になっている。

B18 は性格が違うが、より静かに効く。`pub struct UserDataGuard { /* 非公開 */ }`／
「unit-like struct」という記述のままでは、能力トークンは誰でも構築でき、決定1のゲートも
「解決されること」節の「型レベルでゲートされる」という主張も成立しない。しかも
`compile_fail`テストを書かない限り気付けない。

Major 6件のうち M1・M2・M3 は「読み取り先と書き込み先」ファミリーの**通算7回目**の再出現で
ある。B2→B5→B6→B12→（round11 M1・M2）→今回、と毎ラウンド形を変えて出続けており、
**個別に潰すのではなく「パスを作る関数を1つに決め、比較を一切しない」という不変条件を
チェックリストの最上位に1本立てる**ことを勧める（M3 の要求にある
`yab_write_path_for(name) -> Option<PathBuf>`のような「選択」API に寄せれば、M3・M4 は同時に
閉じる）。M5・M6 は実装可能性の穴で、いずれも数行。

いずれも設計の骨格を変える必要はなく、記述の具体化で閉じる範囲である。ただし
**round10・round11 に続き3ラウンド連続で「型とゲートが具体化した瞬間に新しい破壊経路が
見えた」**という同じパターンが出ているため、v13 反映後にもう1ラウンド（round13）を回し、
そこで Blocker ゼロを確認してから実装着手すること。とくに B17 は、ゲートの主語を
`EnsureOutcome`のスナップショットに固定し、復元失敗を第3条件として入れた上で、決定7の
B15テストが「`self.config_load_state`を`Loaded`にしても発火しない」ことまで検査する形に
なっているかを、次ラウンドで名指しで確認する必要がある。
