# ADR-265 段階 2: 実機測定の手順書(草案、前提の一部を別セッションが develop のコードで確認済み・実機では未実行)

[ADR-265](../adr/265-mozc-readable-integration.md) の段階 2(mozc-readable の共有メモリを belief に採る)の前提になる測定の手順です。**草案で、実機ではまだ何も実行していません**。「要確認」と書いた前提は、実行前に確かめます。

## 1. 目的

belief に採ってよいか(Low → Medium)を、**測定の前に決めた基準**で判断する。結果に合わせて基準を変えない。

**基準(2026-10-18 ユーザー決定、測定前に固定)**: 開閉(open)について、受け入れた読み(`verdict=accepted`)の総数 **N ≥ 200** のうち、**操作から決まる正解と食い違う件数が 0**。1 件でも食い違えば Low のまま(journal に記録するだけ)にして、原因を調べる。入力モード(mode)は、open が基準を満たした後に別に判断する。

理由: 開閉の取り違えは非対称な被害になる(ADR-191・INV-45)ので、誤採用は 1 件でも許さない。一致率(99% など)にすると、件数が少ないとき 1 件の誤りで割合が大きく動き、誤採用を許容してしまう。

## 2. 何を測るか(CI の既存の検査との違い)

- **共有メモリそのものの正しさ**(書き出し側が、押したキーに対して正しい値を出すか)は、mozc-readable の CI(`verify.ps1` の state・apps・appcontainer)がすでに、キー操作から決まる期待値と比べて確かめている。N は 1 run あたり数件と少ない。
- **この測定で新しく測るのは、awase を介したときの誤採用**:
  1. awase が**正しいスレッド**の共有メモリを読んでいるか。実装は既存のフォーカス解決の窓のスレッドで、`hwndFocus` が無ければ `hwndActive` へフォールバックする(別セッションの確認。ADR-265 D3 の脚注)。フォールバックで別のスレッドのブロックを読むと、別の窓の状態を採ってしまう。
  2. awase が起動していて、IME キーに介入する(通す/止める/自分で IME を操作する)環境での、共有メモリの値と正解の食い違い。
  3. 古いブロック(スレッド ID の再利用、IME を切り替えた後の残り)を `process_id` 照合と `active` が実際に弾くか。

## 3. 正解の定義と、キーの入れ方(2026-10-18 に決定)

- **正解は操作列から決まる期待値**(mozc-readable の CI と同じ表。第 4 章)。対象アプリのテキストを読む方法(UI Automation)は、実機では未確認で、使わない。
- **キーは注入で入れる**(自動で N ≥ 200 を回すため)。別セッションの確認(develop のコード、実機では未確認): release の awase では SendInput のキーは「注入」扱いで、注入された F0〜F2・Kanji は physical_disposition のテストで **Allow(OS へ通す)** と固定されている。したがって、awase が起動していても、キーは IME に届き、正解は操作列から決められる。
- **注入では awase の belief が追随しない可能性が高い**(打鍵時の予測は物理キーが前提。注入を物理扱いにするのは `AWASE_TEST_INJECTION=1` の debug ビルドと目印 `0x5350494B` だけ)。採用基準は「共有メモリが正解と合うか」の安全条件で、belief の追随には依存しないので、注入で測れる。**ただし、belief が間違いで共有メモリが正しかった件数(利益の指標)は、注入では過大に出るので、この測定では報告しない**。利益の指標が要るなら、人が物理キーで 20 遷移ほど打つ別枠の小さな測定にする(journal のホットキーダンプは、その別枠だけで使う)。
- 取捨選択の禁止: 食い違いが出た窓・操作を、基準を満たすために N から外さない。外すのは、操作列が届かなかったと確認できた場合(キーの送信失敗)だけで、件数と理由を報告する。

## 4. 操作と窓

- 操作: mozc-readable の CI と同じ列を繰り返す。`F2`(ひらがな、ON)→ `F1`(全角カタカナ)→ `F0`(半角英数)→ `F2` → `19`(OFF)。期待される (open, mode) は (1,1) → (1,2) → (1,3) → (1,1) → (0,*)。
- 窓: メモ帳(通常の Win32)、Chrome(TSF ネイティブ。実機では scoop 経由の `C:\Users\cuzic\scoop\apps\googlechrome\current\chrome.exe`、2026-10-18 に確認)、AppContainer 内のプロセス(UWP の代用。`readable/ci/ac_launch.py`)。実在の UWP アプリ(Windows Terminal など)は別枠で数件。
- 1 窓種あたり 20 周(5 遷移 × 20 = 100 件)を目安に、3 窓種で N ≥ 200 に届くようにする。届かなければ周回を増やす。
- 各遷移の前後で入力先を切り替える周回も混ぜる(フォーカス移動直後の読み = スレッド選択の誤りが出やすい場面)。

## 5. 実機での実行(dragonflyg4、clipwire)

### 5-1. 事前調査の結果(2026-10-18、読み取り専用で実行済み、`adr265-probe`・`adr265-probe2`)

- 昇格していない(**MSI のインストールに UAC が要る**。利用者の応答が要る)。Windows 11 Pro、build 22631。
- Python 3.10.12(`C:\Users\cuzic\scoop\shims\python.exe`)あり、`py` ランチャーは無い。mozc-readable の CI のスクリプト(標準ライブラリのみ)が 3.10 で動くかは**未確認**。
- 入力方式: ja に Google 日本語入力と Microsoft IME、en-US。Mozc / Mozc Readable は未導入。Mozc Readable を足すと言語リストが変わる(終了後に元へ戻す。実行前の一覧は事前調査に記録済み)。
- awase は稼働中(`...\awase\target\debug\awase.exe`、10/12 のビルド)。チェックアウトは `develop` の古い `7428362f` で、`config.toml` が変更済み(利用者の設定)、未追跡ファイルが複数。**このチェックアウトには触れない**(`git pull` もしない)。
- Rust(cargo 1.98.1・rustc 1.98.1)と git(2.47.1)あり。`origin` は https://github.com/cuzic/awase.git。
- 利用者は `C:\Users\cuzic\awase-*` に `git worktree` を作る使い方をしている。

### 5-2. 準備(利用者の操作を妨げない部分。2026-10-18 に着手)

1. **MSI**: mozc-readable の `readable`(`607a6a6`)の CI の成果物を Linux で取得し、Taildrop で実機の `Downloads` へ送った。実機側で `C:\Users\cuzic\Downloads\mozc-readable-607a6a6-x64.msi`(26,595,328 バイト)、SHA-256 `B4795AC7...1F80` が Linux 側と一致することを確認した。測定の直前に、その時点の最新の `readable` の成果物に差し替える。
2. **測定用の awase は、別の worktree**: `C:\Users\cuzic\awase-adr265-measure`(`git worktree add --detach`、`develop` の `d916d18f`、既存のチェックアウトとは別のディレクトリ)。**ADR-265 の記録項目の追加(PR #649)がマージされたら、その SHA へ `git checkout --detach` で進めて再ビルドする**(それまでは依存関係を温めるだけ)。ビルドは `cargo build -p awase-windows --bin awase`(debug)で、clipwire のターゲット `adr265-measure-build-bg` / `adr265-measure-build-log`(ログ `C:\build-adr265-measure.log`)。

### 5-3. 測定の直前(利用者の合図の後。実機を使えなくなる)

1. **既存の awase を止める**(トレイから終了、または `taskkill`)。**止めずに測定用の awase を起動すると、キーボードフックが二重になる**(別セッションの指摘)。測定の最後に、元の awase(`...\awase\target\debug\awase.exe`、10/12 のビルド)を元の場所から起動し直す。
2. **Mozc Readable をインストール**する(`msiexec /i ... /qn`、**UAC**)。入力方式に Mozc Readable を追加する(`Set-WinUserLanguageList`)。**実行前の言語リストを記録する**。
3. **測定用の config.toml**: 測定用の awase の `target\debug\` に置く(別セッションの確認: 引数が無ければ exe の隣の `config.toml` が優先され、無ければ clone のリポジトリ直下。**既存のチェックアウトの `config.toml` には触れない**)。最小限は `[diagnostics]` に `mozc_shm_shadow = true`。他のセクションを省略してよい(serde の既定で補われる見込み)かは**未確認**で、足りなければ既定の `config.toml` を複製して `[diagnostics]` だけ足す。引数で明示する場合は、**第 1 引数にパスを置く**(`awase.exe C:\path\config.toml`、`--` で始まる引数の後に置くと読み飛ばされる)。引数で明示すると、`config.toml` が無くても自己修復で生成されない。
4. 測定用の awase を、測定用のディレクトリから起動する。ログは exe と同じフォルダー(`target\debug\awase.log`)に出る。起動前に、`awase.log` の日付とサイズを記録する。
5. キー注入と読み取りは、mozc-readable の CI のスクリプト(`ui_host.py`・`drive_foreground.py`・`reader.py`・`ac_launch.py`)を流用し、時刻は UTC で記録する。実機の Chrome のパスが CI と違う(scoop)ので、`Run-App` 相当の起動部分は実機用に書く。

### 5-4. 測定の後

1. 測定用の awase を止める。`awase.log` を Linux へ回収する。
2. **Mozc Readable をアンインストール**し、言語リストを記録した元の状態に戻す。
3. **元の awase を起動し直す**(止めた場合)。
4. 測定用の worktree は、集計が終わるまで残し、その後に削除する(利用者に確認)。

### 5-5. 実機の操作との干渉

キー注入とフォーカス移動を行うので、測定中(約 20〜30 分の見込み、未測定)は実機を使えない。時間帯は利用者が合図する。

### 5-6. 記録の取り出し

journal のホットキーダンプ(Alt+変換 → Alt+無変換 を 2 回連続。物理の Alt が要る見込み、`%TEMP%\awase_journal_<tick_ms>.json`)は、自動測定には向かない。`MozcShm` は Timing レーン(2048 件のリング、他の Timing 系と同居)に入り、100 操作ごとに人の手でダンプしないと古い方から追い出される(別セッションの確認)。そこで、**awase.log の `[mozc-shm]` 行を集める**。以下は別セッションが develop のコードを読んで確認した内容(PR #649。**実機では未確認**):

- 出力: `[diagnostics] mozc_shm_shadow = true` だけで、target `awase::mozc_shm` の **info** で、行頭 `[mozc-shm] ` + 記録の全項目の 1 行 JSON(`thread_id`・`process_id` を含む)。`RUST_LOG` の指定は要らない(`awase::journal=debug` は全 journal の debug を出して打鍵を含むため、測定には使わない)。設定の反映は再起動後。
- 時刻: 行頭は UTC の RFC3339(壁時計)。操作側のログも UTC で出して突き合わせる(実機の時計は、事前調査で local と UTC の差が +9 時間であることを確認済み)。
- ローテーション: 20 MiB を超えると `awase.log.old` へ 1 世代だけ移る。
- 性能: `mozc_shm_shadow = true` のときだけ、約 500 ms ごとに共有メモリを 1 回読む。所要は記録の `read_us`(マイクロ秒)で測れる。

## 6. 集計

awase.log の `[mozc-shm]` 行(1 行の JSON: `verdict`、`route`、`reason`、`tsf_open`・`tsf_mode`・`effective_*`、`version`、`profile`、`thread_id`、`process_id`、`belief_open`、`belief_mode`、行頭の UTC の時刻)を、操作側のログ(各キーの送信時刻と期待値)と**時刻で**突き合わせる。記録は共有メモリの結論が変わったとき(と、フォーカスが変わったとき)だけに出るので、各操作の「送信から次の送信まで」の間に出た最後の `accepted` の状態が、その操作の期待値と一致するかを見る(送信から約 61 ms で反映される実測がある。待ちは余裕を見て 1 秒)。

報告する数字:
- N(受け入れた読みの総数)、窓種ごとの内訳。
- 食い違い件数(**基準: 0**)。1 件でもあれば、その窓種・操作・`route`・`reason`・`thread_id`・`process_id`・フォーカスの直前の状態。`thread_id` と `process_id` は、別のスレッドを読んだ(フォールバックなど)かの切り分けに使う。
- 参考: `verdict=rejected`/`failed` の件数と `reason` の内訳(読めなかった割合。採用の可否の条件ではない)。
- 利益の指標(belief が間違いで共有メモリが正しい件数)は、注入では過大に出るので報告しない(第 3 章)。
- **`belief_open`・`belief_mode` の列は、注入の測定では集計の対象外にする**。release の awase は注入キーを物理とみなさないので、belief は追随せず、常に古い値になるはずで、見ると混乱する(別セッションの指摘)。記録は取るが、判定には使わない。

## 7. 担当

- 手順書・実機の操作・集計・ADR の更新: 私(rust-nicola セッション)。
- awase 本体の belief への接続(Low から): 別セッション(rust-nicola-05)。基準を満たすまで Medium には上げない。
