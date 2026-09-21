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
  **提案(ドラフトv4、opus round1(Blocker4・Major8・Minor5)・round2(新規Blocker1・Major4)反映済み、
  round3(Blocker解消・Major3)・実施計画レビューround1(Blocker1・Major4・Minor11)・round2(Blocker1・Major2・Minor7)・round3(Major2・Minor8)・round4(Blocker1・Major1・Minor6)反映済み、
  再確認待ち)**。未実装。**検知対象の不具合(BUG-002型)が
  現行コードで再現するかが未確認**で、再現しなければ本ADRの目標は「再発の予防(回帰検知)」へ変質する
  (2026-07-18の機構削除後、数日の実機ソークで genuine な部分リテラルはゼロ件だった、`docs/experiments.md`)。
  撤去対象の機構は未特定(ステップ0の結果待ち)。
  詳細設計と着手順序は[193-implementation-tasks.md](193-implementation-tasks.md)。未確認事項は末尾に列挙した。
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
- v3までは、`tuning.rs:88-90`のdocコメント(「`CHROME_LONG_IDLE_MS`(5s)が`ColdKind::classify`のcutoffになる」)を
  実装で裏取りせず引いた。実際は`ColdKind::classify`は7s/10sしか見ておらず、docが古かった。round2のレビュー指摘も
  同じdocに依拠しており、実装計画のレビュー(`193-opus-review-plan-round1.md`)で判明した(3回目)。
- 教訓(本ADRの規律): **known-bugsの「現在の対策」だけでなく、`tuning.rs`等のdocコメントも、必ず現行の実装で
  存在・挙動を裏取りしてから使う**。以下は裏取り済みの事実に基づく。

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
  F2事前送信・probe事前待機が削除され、per-VK confirm(`tsf/warmup/probe_fsm.rs::run_per_vk_confirm`(`:454`)、
  部分リテラル判定は`tsf/warmup/literal_detect_fsm.rs`)に一本化された(`output/vk_send.rs`の`[h1-probe] … F2/probe待機省略
  → per-VK confirmへ`)。したがって「BUG-002の修正定数を旧値へ戻す」撤去は成立しない。
- **idleの閾値と、測る量**: awaseがcold種別の判定に使う量はkeyboard idleではなく`gji_idle_ms`(`tsf::observer::gji_idle_ms()`、
  GJIのI/O観測からの経過時間)。`ColdKind::classify`(`gji_fsm.rs:127-136`)のcutoffは`MEDIUM_IDLE_PROBE_MS`=7s(Medium、
  `forces_prepend_f2`=true)と`LONG_IDLE_MS`=10s(Long)のみで、injection modeに依存しない。`CHROME_LONG_IDLE_MS`=5sは
  cutoffではなく、`transition_to_warm`がOnWarm→OnColdへ落とすまでのタイマー長(`gji_fsm.rs:470-479`、`long_idle_ms_for`経由)。
  `tuning.rs:88-90`のdocは「`ColdKind::classify`のcutoffになる」と書いており**実装と食い違うstale記述**(v3までのADRとround2のレビュー指摘は
  これを鵜呑みにしていた。実装計画T6で直す)。`BUG-002.md`の「>10s」も旧記述。

### 既存資産でまだできていないこと

`chrome_probe`はlong-idleを作れないわけではない。`--settle=<ms>`は**モードキー押下の後**に`k`,`a`を打つまでの待ちで、
この間はキーもGJI I/Oも発生しないため、**keyboard idleとGJI idleが同時に進む**。したがって「両方long」
(旧BUG-002対策表の「keyboard long idle (>10s)」側)は今日コード変更なしに作れる(`--settle=11000`等)。
足りないのは次の3点である:

1. **keyboard idleとGJI idleの分離**: 旧表のもう一方の分岐「keyboard short idle かつ GJI long idle」(物理F2+GJI休眠)は、
   待ちをモードキー押下の**前**に入れる(長く待つ→F2→即座に打鍵)必要があり、`chrome_probe`にその位置のフラグ
   (`--pre-settle`相当)が無い。なおChromeのprogrammatic F2事前送信は削除済み(`vk_send.rs:274`)なので、打ち分けの対象は
   「物理F2を押すか否か」だけになる。
2. **入力列の拡張**: 現行の判定入力は`k`,`a`の2打で、先頭1モーラだけがリテラル化する部分リテラル(`という→toいう`)を
   表現できない。自動判定を作る段で多モーラ列(`toiu`→`という`等)へ拡張する。
3. literal化の**自動判定**と、それを判定するチェッカー。

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

(詳細設計・タスク分割・着手順序・判定ゲートは[193-implementation-tasks.md](193-implementation-tasks.md)。以下はその要旨。)

0. **ステップ0(先にやる): 既存`chrome_probe`でBUG-002の症状が現行コードで今も出るかを確認する**。
   既存ケース4(`半角英数→ひらがな=かな`、F2→待ち→打鍵でBUG-002の形と一致)を`--settle`で掃引する
   (`gji_idle_ms`の4帯 <5s / 5〜7s / 7〜10s / ≥10s を踏む3s/6s/8s/11s/14s)。
   **見るのは`Class`ではなくログの生の`t.value`**(`Class`は`k`,`a`の2打で部分リテラルを`Other`等に落とす)。
   コード変更は要らない。目的は、症状が今も出るか、出るならどの機構が防いでいるかの特定。
   **ただし`--settle`は「モードキー押下の後」の待ちで、モードキー直後にGJIがwarmへ戻っている可能性を区別できない。
   ステップ0の結論でG0(出る/出ない)を確定させず、確定はidle-sweep(実装計画T1b)の結果で行う。**
   **「出ない」公算が高い**(2026-07-18の機構削除後、実機ソークでgenuineゼロ件、`docs/experiments.md`)ため、
   「出ない」側を先に設計する:
   - 出る → 出た条件から、撤去すると症状が戻る現行機構を特定する(探索範囲は`probe_fsm.rs::run_per_vk_confirm`、
     `literal_detect_fsm.rs`の部分リテラル判定、`vk_send.rs:279`周辺のcold分岐)。
   - 出ない → 「出ないこと」を成功基準にするのは、次の**3つをすべて満たす場合のみ**CIに載せる。
     不在のassertは、awase未起動・`AWASE_TEST_INJECTION`付け忘れ・Chrome非前面・キー未到達でも緑になるため。
     (a) **ablationは必須**: 現行機構のいずれかを撤去すると症状が戻ることを示す(戻らないなら、そのassertは
     何も守っていないのでCIに載せない)。
     (b) **陽性対照を同一実行内に含める**: `--no-awase`腕(awase停止時は`か`)か、`e2e_windows.rs:2820-2838`型の
     ASCII canary(素のASCIIが届いたかを先に確認)。対照が取れなければFAILではなく**INVALID**(`chrome_probe`の語彙)。
     (c) INVALID条件を維持する(`前提状態にできなかった`・`focus_lost`・物理キー混入=`check_multi.py`)。
1. **BUG-002型シナリオの拡張**(ステップ0で必要と分かった分のみ): 上記「まだできていないこと」の
   keyboard idleとGJI idleの分離(`--pre-settle`)・多モーラ入力列・literal化の自動判定。
   実idleの見積もりは**ケース数 × idle秒 × マトリクス**で明示する(`e2e-ime.yml`は`matrix.cfg` × `run:[1,2,3]`で
   展開され`timeout-minutes: 25`。構成を1つ足すと3ジョブ増える)。載せる前にこの見積もりを出す。
   **`timeout-minutes: 25`を超える場合の方針**: 掃引点を減らす → 構成を分ける → `run:[1,2,3]`を減らす、の順に対処する
   (別ワークフロー化は最後)。
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

- **現行コードでBUG-002型が再現するか**(ステップ0)。再現しなければ、目標は回帰検知へ変質し、決定4-0の3条件が要る。
- keyboard idleとGJI idleを**分離**できるか(`--pre-settle`)、およびCIの25分枠に収まるか。
- `windows-latest`にChrome/Edgeが同梱されているか(GitHubのrunner imageは同梱している想定だが、リポジトリ内では
  実証されていない。決定4-2の`Test-Path`で確認する)。
- `chrome_probe`のイベント記録は`seq`連番付きなので到着順の乱れは受信側で直せる。残る本物の懸念は、
  POST(`fetch(..., {keepalive:true})`)の発行自体がレンダラの処理を遅らせ、cold-startのタイミングを
  変えないか、の1点。
