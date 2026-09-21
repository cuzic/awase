---
id: ADR-193
title: |-
  実機E2Eの既存ハーネス(ime_key_matrix_spike / chrome_probe / e2e-ime.yml)を拡張し、Chrome cold-start系の不具合(BUG-002)を検知できるようにする
summary: |-
  当初案(自プロセスRichEditと、`document.title`回収の実Chrome用ハーネスを新規に作る)は、
  opus-adversarial-consult round1で「決定1・2に対応する資産が既にdevelop上にあり、CIにも載っている」と
  指摘され撤回した。既存資産: `examples/ime_key_matrix_spike.rs`(標準EDITとRICHEDIT50Wの2ラウンド)、
  `examples/chrome_probe.rs`(実Chrome+専用プロファイル+ローカルHTTP回収+`SendInput`注入)、
  `.github/workflows/e2e-ime.yml`(windows-latestにGJI/MS-IMEを導入して実機E2E)。
  本ADRの決定: (1) RichEdit用の新規ハーネスは作らない。RICHEDIT50Wは`AppKind::Win32`/
  `AppImeProfile::Standard`で、TsfNative政策経路の代表にならない(コードで確定)。(2) Chromium系の検知は
  既存`chrome_probe.rs`を拡張する。回収の主経路は既存のHTTP POSTで、`document.title`案は撤回する
  (連続更新が合体し、イベント列を取れない)。Tauriは未検証の仮説として不採用。(3) キー注入は目印付き
  `SendInput`+`AWASE_TEST_INJECTION=1`+debugビルドが必須(目印なしはawaseを素通りし、偽陰性になる)。
  (4) 実際に足すのは、まず既存`chrome_probe --settle=`でBUG-002の症状が**現行コードで今も出るか**の確認
  (ステップ0)。BUG-002が記す対策機構(`CHROME_PROBE_*`)は2026-07-18にper-VK confirmへ一本化されて
  物理削除済みで、撤去対象は現行機構から特定し直す。その上で`ablations/`の撤去スクリプトによる
  「撤去あり=FAIL/なし=PASS」の実証と、`chrome_probe`の`e2e-ime.yml`への接続(ビルド・runステップ・
  判定スクリプトの3点)。`bあ`(`9a7e699`)はBUG-002ではなく別バグなので分離する。
status: |-
  **提案(ドラフトv3、opus round1(Blocker4・Major8・Minor5)・round2(新規Blocker1・Major4)反映済み、
  round3再確認待ち)**。未実装。撤去対象の機構は未特定(ステップ0の結果待ち)。未確認事項は末尾に列挙した。
related_adr:
  - "ADR-0002"
  - "ADR-0003"
  - "ADR-186"
---

# ADR-193: 既存の実機E2Eハーネスを拡張してChrome cold-start系を検知する

## 経緯(v1・v2の撤回)

- v1は「自プロセスRichEditを作る」「実Chrome+静的HTML+`document.title`回収の新規ハーネスを作る」という
  内容だった。起票前に`tests/`と`e2e_windows.rs`しか見ておらず、`tools/e2e/`と
  `crates/awase-windows/examples/`の既存資産を棚卸ししていなかった(round1、`193-opus-review-round1.md`)。
- v2は、BUG-002が記す対策定数`CHROME_PROBE_LONG_IDLE_MIN_MS`を撤去する検証計画を立てたが、この定数は
  2026-07-18に機構ごと物理削除済み(`tuning.rs:87-96`、`docs/known-bugs/BUG-024.md`)で、
  `BUG-002.md`の「現在の対策」表が古いまま(stale)だった。v1が実装資産を棚卸ししなかったのと同型の誤りで、
  今度は**docsの記述が現役か**を確認していなかった(round2、`193-opus-review-round2.md`)。
- 教訓(本ADRの規律): **known-bugsの「現在の対策」は、必ず現行の`tuning.rs`/実装で存在を裏取りしてから
  使う**。以下は裏取り済みの事実に基づく。

## 背景: 既存資産の棚卸し

| 資産 | 内容 |
|---|---|
| `crates/awase-windows/examples/ime_key_matrix_spike.rs` | 標準EDITとRICHEDIT50W(`Msftedit.dll`)の2ラウンドでキー効果を測る。`e2e-ime.yml`がビルド・実行する |
| `crates/awase-windows/examples/chrome_probe.rs` | 専用プロファイルの実Chromeを`--app=http://127.0.0.1:{port}/`で起動。検証ページのJSが`keydown`/`composition*`/`beforeinput`/`input`を連番(`seq`)付きで記録し、標準ライブラリの`TcpListener`へPOST。`--repeat`/`--no-awase`/`--chrome`/`--log`/`--settle=MS`(モードキー後、`k`,`a`を打つまでの待ち)あり |
| `.github/workflows/e2e-ime.yml` | `windows-latest`にGJI(chocolatey)とMS-IME(ja-JP言語機能)を導入し、`ctfmon`再起動まで行って目印付き`SendInput`の実機E2Eを実行。撤去実験(`ablations/`)も接続済み |
| `tools/e2e/ime_key_matrix/ablations/aN-*.sh` | 修正を作業ツリーから撤去するスクリプト。CIが「撤去が差分を作らなければfail」まで検査する |
| `hook.rs::TEST_INJECTION_MARKER` / `is_test_injection` | `AWASE_TEST_INJECTION=1`かつ`dwExtraInfo`が目印に一致するときだけ注入キーを物理キー扱いする。**debugビルド限定**(releaseでは常にfalse) |

### 現状認識の訂正

- **CI**: `e2e_windows.rs`のWindows Terminal系テスト(`e2e_msime_windows_terminal_vk_mode_coldstart_interactive`)は、
  `is_interactive_session()`が`CI`/`GITHUB_ACTIONS`設定時にfalseを返して**早期returnし、CIで実行されていない**
  (`ci.yml`の`continue-on-error`は`e2e_sendinput`/`e2e_ime_status_detection`が対象で別物)。
  実機E2Eの実行場所は`e2e-ime.yml`側。
- **Windows Terminalの確定文字列**: `WM_GETTEXT`では読めないが、既存テストはPowerShellの`Read-Host`結果を
  `Set-Content`でファイルへ書かせて読んでいる。アプリごとの回収手段(WTはファイル、ChromeはHTTP POST)が要る、が正確。
- **`bあ`はBUG-002ではない**: `bあ`は`9a7e699`(`GJI_LONG_IDLE_PROBE_TOTAL_MS`150→350ms、F2×2後にGJIがVK受付
  可能になるまで実測181ms)の症状。BUG-002は`という→toいう`(`b101153`/`79134f5`)。
- **BUG-002の対策機構は削除済み**: `BUG-002.md`の「現在の対策」表(`CHROME_PROBE_MIN/MAX_MS`、
  `CHROME_PROBE_LONG_IDLE_MIN/MAX_MS`)は`tuning.rs`に存在しない。2026-07-18のBUG-24対応で、Chromeの
  F2事前送信・probe事前待機が削除され、per-VK confirm(`tsf/warmup/probe_coro_state.rs::run_per_vk_confirm`、
  `tsf/warmup/literal_detect_fsm.rs`)に一本化された(`output/vk_send.rs`の`[h1-probe] … F2/probe待機省略
  → per-VK confirmへ`)。したがって「BUG-002の修正定数を旧値へ戻す」撤去は成立しない。
- **long-idleの閾値**: Chrome(VK)は`CHROME_LONG_IDLE_MS`=5s(`tuning.rs:100`、`gji_fsm.rs::long_idle_ms_for`が
  `InjectionMode::Vk`で参照)。GJI/TSF経路は`LONG_IDLE_MS`=10s、その間に`MEDIUM_IDLE_PROBE_MS`=7s(`tuning.rs:148`)。
  `BUG-002.md`の「>10s」は旧記述で、Chrome(VK)の`ColdKind`分岐のcutoffは5s。

### 既存資産でまだできていないこと

`chrome_probe`はlong-idleを作れないわけではない。`--settle=<ms>`でモードキー後の待ちを任意に伸ばせる
(コード変更なしに`--settle=6000`や`--settle=11000`を渡せる)。足りないのは次の3点である:

1. GJI休眠の制御(GJIセッションを意図的に休眠させる手段)。
2. 物理F2相当(目印付き`SendInput`のF2)とプログラム的F2の打ち分け。
3. literal化の**自動判定**(`という→toいう`型の検出)と、それを判定するチェッカー。

加えて`chrome_probe`は`e2e-ime.yml`から呼ばれていない(`.github/`に参照なし。参照は
`tools/e2e/ime_key_matrix/README.md`と`clipwire-targets.example.toml`の手動実行のみ)。

## 決定

### 決定1: RichEdit用の新規ハーネスは作らない

`detect_app_kind`(`focus/class_names.rs:347-365`)はクラス名だけの純関数で、`chrome_*`前方一致・
`TeamsWebView`・`MozillaWindowClass`のみ`TsfNative`、それ以外は`Win32`。`RICHEDIT50W`は必ず`Win32`。
`AppImeProfile::from_class_name`(同`:120-129`)も`IMM32_UNAVAILABLE_CLASSES`/`is_tsf_native_window`の
クラス名テーブル一致のみで、`RICHEDIT50W`は`Standard`。したがってRichEditは標準EDITと同じ
ImmCross経路で、awaseのTsfNative政策経路(Vk注入・force-on・warmup)の代表にはならない。
既存の`ime_key_matrix_spike`のRichEditラウンドは「TSF text storeを持つコントロールでのIMM読み取り」
の検証として残す。学習キャッシュ(`imm_learning.rs`)は`AppKind`を書き換えず、`ImmCapabilityStore`にのみ書く。

### 決定2: Chromium系の検知は`chrome_probe.rs`の拡張で行う

- 実Chrome+専用プロファイル+HTTP POST回収(既存)を主経路とする。**`document.title`回収案は撤回**する:
  ハーネスのポーリングでは最新の1件しか読めず、連続更新はブラウザプロセスへのIPCで合体しうるため、
  `compositionend.data`やイベント順を取れない。
- Tauri(WebView2)は**未検証の仮説として**不採用とする(ホスト構成がChrome本体と異なり、cold-startが
  同条件で出る保証がない、という仮説。ワークスペースにビルドが増える)。Electronは後回し。
- CDPは読み取り専用(`Runtime.evaluate`)の補助に限る。**CDPの`Input.*`によるキー注入は禁止**(決定3)。

### 決定3: キー注入は目印付き`SendInput` + `AWASE_TEST_INJECTION=1` + debugビルド

- 注入キーは`dwExtraInfo = hook::TEST_INJECTION_MARKER`を付ける。`e2e_windows.rs::send_key_to_edit`
  (`dwExtraInfo: 0`、doc自身が"bypasses hooks")は、awase経路の検証には使えない。
- awaseはdebugビルドを`AWASE_TEST_INJECTION=1`で起動する(releaseでは目印があっても無効)。
- 目印なしの回を検出する手段として、既存の`check_multi.py`(awaseログの`extra=0x0`でINVALID)を使う。
  これを怠ると、awaseを素通りしたローマ字の素の出力を「リテラル漏れが再現しない=修正が効いている」と
  誤読する偽陰性になる。
- 前面化ヘルパーは`e2e_windows.rs`のテストローカル関数のため`examples/`から共有できない。
  `chrome_probe.rs`が既に持つ独自の前面化(`AttachThreadInput`版)を使い、**3つ目のコピーは作らない**。

### 決定4: 追加する差分と検証方法

0. **ステップ0(先にやる): 既存`chrome_probe --settle=<閾値超>`でBUG-002の症状が現行コードで今も出るかを確認する**。
   `--settle`を、Chrome(VK)の`CHROME_LONG_IDLE_MS`=5sの内外、`MEDIUM_IDLE_PROBE_MS`=7s、`LONG_IDLE_MS`=10s
   の3閾値をまたぐ点(例: 3s/6s/8s/11s)で掃引する。目的は、症状が今も出るか、出るならどの機構が防いでいるかを
   特定すること。
   - 出ない → per-VK confirmが既に十分に防いでおり、BUG-002型は現行では再現しない。撤去実験の対象は
     現行機構(per-VK confirm側)に取り直すか、本ADRの成功基準を「再現しないこと自体の回帰検知」に改める。
   - 出る → 出た条件から、撤去すると症状が戻る現行機構を特定する。
1. **BUG-002型シナリオの拡張**(ステップ0で必要と分かった分のみ): 上記「まだできていないこと」の
   GJI休眠制御・物理F2/プログラム的F2の打ち分け・literal化の自動判定。
   実idleの見積もりは**ケース数 × idle秒 × マトリクス**で明示する(`e2e-ime.yml`は`matrix.cfg` × `run:[1,2,3]`で
   展開され`timeout-minutes: 25`。構成を1つ足すと3ジョブ増える)。載せる前にこの見積もりを出す。
2. **`chrome_probe`を`e2e-ime.yml`に接続する**。次の3点セットで、1行の追加では済まない:
   - ビルド: `cargo build … --example chrome_probe`の追加(現状は`ime_key_matrix_spike`のみ)。
   - runステップ: `ime_key_matrix_spike`用にハードコードされた起動引数と、`cache.toml`への
     `[imm_capability."ime_key_matrix_spike.exe"]`事前投入(CIランナーの初回IMMプローブ遅延による誤学習対策)は
     `chrome_probe`に流用できない。別プロセス名・別クラスなので、起動・環境整備を別に書く。
   - 判定: `plan`ジョブの`check`種別(`expect`/`consistency`/`toggle`/`resync`/`vkprobe`)はいずれも
     `ime_key_matrix_spike`のログ形式前提で、`chrome_probe.log`を判定するチェッカーは無い。新規に書く
     (`check_multi.py`のINVALID判定=`extra=0x0`による人の入力混入検出を再利用できるかを検討項目に入れる)。
   - Chrome同梱の確認は、CIに`Test-Path 'C:\Program Files\Google\Chrome\Application\chrome.exe'`を1行足して行う
     (`chrome_probe.rs`が探索する既定パスと一致するため)。無ければ導入ステップを足す。
3. **成功基準は旧コミットの再現ではなく撤去スクリプトで示す**: `b101153`/`79134f5`時点のツリーには
   新ハーネスが存在しないため、旧コミットのチェックアウトは成立しない。代わりに`ablations/`へ、
   **ステップ0で特定した現行機構**の撤去を1本追加し、**撤去あり=FAIL/撤去なし=PASS**がN回安定して
   出ることを基準にする(CIの「撤去が差分を作らなければfail」に乗る)。
4. **本体ソース**: リポジトリにコミットされる本体ソースは新たには変更しない。撤去スクリプトはCI実行時に
   作業ツリーへ差分を当てるだけで、リポジトリに増えるのは`ablations/*.sh`のみ。既存のテスト用フック
   (`TEST_INJECTION_MARKER`、debug限定)には依存する。`tuning.rs`の定数を新設・変更しないため
   `complexity-budget`の対象外。
5. **`docs/known-bugs/BUG-002.md`の「現在の対策」表は古い**(上記)。本ADRの実施時に、削除済み機構である旨を
   追記する(`fix-requires-evidence`(b)の「人間可読な再発防止」が、存在しない定数名を指したまま
   放置されている状態を直す)。本コミットでは注記のみ入れ、対策の書き直しはステップ0の結果を待つ。

## 検討した代替案

- **awaseの送信側だけを決定的に固定する案(未着手・別ADR候補)**: 分類は基本的にクラス名の文字列一致なので、
  ハーネスで`RegisterClassExW`するクラス名を`Chrome_WidgetWin_1`(`Imm32Unavailable`+`TsfNative`)や
  `org.wezfurlong.wezterm`(`TsfNative`)にすれば、外部アプリを起動せずawaseの政策分岐を走らせられ、
  入力欄は自前の`EDIT`のまま`WM_GETTEXT`で厳密にassertできる。限界は明確で、模倣できるのは
  「awaseが何を送るか」までであり、Chromiumが実際にTSFでどう受けるか(cold-startのcontext再初期化)は
  再現しない。
  **実リスク(採否の前に解決が要る)**: `InjectionModeStore`(`focus/classifier.rs:429-453`)は
  class_name単独キーの`HashSet`で、`learn_tsf()`が`cache.toml`に**永続化**し、`has_tsf(class_name)`が
  trueなら`InjectionHint::ForceTsf`を返す(`focus/tracker.rs:115-123`)。事後昇格
  (`tsf/warmup/probe_fsm.rs:242`、`platform.rs:482-484`)が一度でも発火すると、ハーネスが名乗った
  `Chrome_WidgetWin_1`が恒久的に書かれ、**実物のChromeがForceTsfに固定される**(BUG-107で
  `ImmCapabilityStore`が`(process_name, class_name)`キーに直されたが、この`InjectionModeStore`は
  未修正で同型の汚染が起きる)。採用するなら、キャッシュ保存先(`app/bootstrap.rs`が渡す`base_dir`)を
  テスト専用に差し替える必要がある。実在しないクラス名では分類テーブルに一致せず偽装の目的を達しない
  というジレンマもある。なお`detect_app_kind`のdocコメント「ヒューリスティックでChromeに昇格する場合あり」は、
  関数本体に該当ロジックが無くstaleである(分類は文字列一致のみ)。
- **Tauri/WebView2/Electronをハーネスにする**: 決定2の通り。
- **ジャーナルリプレイのみ**: 主軸のままだが、Chromeが実際に受け取った文字列は観測できない。E2Eはその補完。

## 未確認事項

- `windows-latest`にChrome/Edgeが同梱されているか(GitHubのrunner imageは同梱している想定だが、リポジトリ内では
  実証されていない。決定4-2の`Test-Path`で確認する)。
- `chrome_probe`のイベント記録は`seq`連番付きなので到着順の乱れは受信側で直せる。残る本物の懸念は、
  POST(`fetch(..., {keepalive:true})`)の発行自体がレンダラの処理を遅らせ、cold-startのタイミングを
  変えないか、の1点。
- 現行コードでBUG-002型が再現するか(ステップ0)。再現しない場合、成功基準の前提が変わる。
- 実idleと、GJI休眠をCI上で再現できるか(GJI・実機依存が強い)。
