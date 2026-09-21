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
  (連続更新が合体し、イベント列を取れない)。Tauri不採用。(3) キー注入は目印付き`SendInput`+
  `AWASE_TEST_INJECTION=1`+debugビルドが必須(目印なしはawaseを素通りし、偽陰性になる)。
  (4) 本ADRで実際に足すのは、BUG-002のlong-idle条件(GJI・keyboard idle >10s)を作るシナリオと、
  `chrome_probe`の`e2e-ime.yml`への接続と、`ablations/`の撤去スクリプトによる「撤去あり=FAIL/なし=PASS」
  の実証。`bあ`(`9a7e699`)はBUG-002ではなく別バグなので対象から分離する。
status: |-
  **提案(ドラフトv2、opus round1(Blocker4・Major8・Minor5)反映済み、round2再確認待ち)**。
  未実装。未確認事項は末尾「未確認事項」節に列挙した。
related_adr:
  - "ADR-0002"
  - "ADR-0003"
  - "ADR-186"
---

# ADR-193: 既存の実機E2Eハーネスを拡張してChrome cold-start系を検知する

## 経緯(v1の撤回)

v1は「TSFネイティブ入力先として自プロセスRichEditを作る」「Chromium検知に、実Chrome+静的HTML+
`document.title`回収の新規ハーネスを作る」という内容だった。起票前に`tests/`と`e2e_windows.rs`しか
見ておらず、`tools/e2e/`と`crates/awase-windows/examples/`の既存資産を棚卸ししていなかった。
round1レビューの指摘(`193-opus-review-round1.md`)を実コードで裏取りし、v1の決定1・2は成立しないと
判断した。以下は裏取り済みの事実に基づく書き直しである。

## 背景: 既存資産の棚卸し

| 資産 | 内容 |
|---|---|
| `crates/awase-windows/examples/ime_key_matrix_spike.rs` | 標準EDITとRICHEDIT50W(`Msftedit.dll`)の2ラウンドでキー効果を測る。`e2e-ime.yml`がビルド・実行する |
| `crates/awase-windows/examples/chrome_probe.rs` | 専用プロファイルの実Chromeを`--app=http://127.0.0.1:{port}/`で起動。検証ページのJSが`keydown`/`composition*`/`beforeinput`/`input`を記録し、標準ライブラリの`TcpListener`へPOST。`--repeat`/`--no-awase`/`--chrome`/`--log`あり |
| `.github/workflows/e2e-ime.yml` | `windows-latest`にGJI(chocolatey)とMS-IME(ja-JP言語機能)を導入し、`ctfmon`再起動まで行って目印付き`SendInput`の実機E2Eを実行。撤去実験(`ablations/`)も接続済み |
| `tools/e2e/ime_key_matrix/ablations/aN-*.sh` | 修正を現在のツリーから撤去するスクリプト。「撤去が差分を作らなければfail」まで含めてCI化されている |
| `hook.rs::TEST_INJECTION_MARKER` / `is_test_injection` | `AWASE_TEST_INJECTION=1`かつ`dwExtraInfo`が目印に一致するときだけ注入キーを物理キー扱いする。**debugビルド限定**(releaseでは常にfalse) |

### 現状認識の訂正(v1の誤り)

- **CI**: `e2e_windows.rs`のWindows Terminal系テスト(`e2e_msime_windows_terminal_vk_mode_coldstart_interactive`)は、
  `is_interactive_session()`が`CI`/`GITHUB_ACTIONS`設定時にfalseを返して**早期returnし、CIで実行されていない**
  (`ci.yml`の`continue-on-error`は`e2e_sendinput`/`e2e_ime_status_detection`が対象で別物)。
  実機E2Eの実行場所は`e2e-ime.yml`側。
- **Windows Terminalの確定文字列**: `WM_GETTEXT`では読めないが、既存テストはPowerShellの`Read-Host`結果を
  `Set-Content`でファイルへ書かせて読んでいる。アプリごとの回収手段(WTはファイル、ChromeはHTTP POST)が要る、が正確。
- **`bあ`はBUG-002ではない**: `bあ`は`9a7e699`(`GJI_LONG_IDLE_PROBE_TOTAL_MS`150→350ms、F2×2後にGJIがVK受付
  可能になるまで実測181ms)の症状。BUG-002は`という→toいう`(`b101153`/`79134f5`)で、条件はGJI・
  keyboard long idle(>10s)・Chromeのcomposition context再初期化~326ms。

### 既存資産でまだできていないこと

- `chrome_probe`は`e2e-ime.yml`から呼ばれていない(README/ソースのみで参照。CI接続は未実装)。
- BUG-002のlong-idle条件(keyboard idle >10s、GJI休眠)を作るシナリオがない(`chrome_probe`内の待機は
  最大1.5秒程度の固定sleep)。
- 「修正前は再現し、修正後は消える」ことをE2Eで示す手段が、Chrome向けには未整備。

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
- Tauri(WebView2)は引き続き不採用(ホスト構成がChrome本体と異なり、cold-startが同条件で出る保証がない。
  ワークスペースにビルドが増える)。Electronは後回し。
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

1. **BUG-002のlong-idleシナリオ**を`chrome_probe`に追加する。必要条件: GJI、実際のkeyboard idle
   (>10s)、GJI休眠(~12s)、物理F2相当(目印付きSendInputのF2)とプログラム的F2の打ち分け。
   1ケースあたり十数秒の実idleが要るため、`e2e-ime.yml`の`timeout-minutes: 25`への影響を見積もってから載せる。
2. **`chrome_probe`を`e2e-ime.yml`に接続する**(Chrome同梱の有無は未確認。無ければ導入ステップが要る)。
3. **成功基準は旧コミットの再現ではなく撤去スクリプトで示す**: `b101153`/`79134f5`時点のツリーには
   新ハーネスが存在しないため、旧コミットのチェックアウトは実行計画として成立しない。代わりに
   `ablations/`に、BUG-002相当の撤去(`CHROME_PROBE_LONG_IDLE_MIN_MS`等を旧値へ戻す)を1本追加し、
   **撤去あり=FAIL/撤去なし=PASS**がN回安定して出ることを基準にする(既存の「撤去が差分を作らなければfail」
   の仕組みに乗る)。`tuning.rs`自体は変更せず、スクリプトで差分を当てる形なので
   `complexity-budget`の対象外。
4. 本体ソースは**新たには**変更しない。ただし既存のテスト用フック(`TEST_INJECTION_MARKER`、debug限定)には依存する。

## 検討した代替案

- **awaseの送信側だけを決定的に固定する案(未着手・別ADR候補)**: 分類は基本的にクラス名の文字列一致なので、
  ハーネスで`RegisterClassExW`するクラス名を`Chrome_WidgetWin_1`(`Imm32Unavailable`+`TsfNative`)や
  `org.wezfurlong.wezterm`(`TsfNative`)にすれば、外部アプリを起動せずawaseの政策分岐を走らせられ、
  入力欄は自前の`EDIT`のまま`WM_GETTEXT`で厳密にassertできる。限界は明確で、模倣できるのは
  「awaseが何を送るか」までであり、Chromiumが実際にTSFでどう受けるか(cold-startのcontext再初期化)は
  再現しない。**注意**: `detect_app_kind`のdocは「ヒューリスティックでChromeに昇格する場合あり」と書いており、
  `from_class_and_process`はプロセス名(`input_relay_apps`)でも上書きされる。クラス名だけで分岐が決まる
  とは限らないため、採否の前に昇格条件の確認が要る。
- **Tauri/WebView2/Electronをハーネスにする**: 決定2の通り。
- **ジャーナルリプレイのみ**: 主軸のままだが、Chromeが実際に受け取った文字列は観測できない。E2Eはその補完。

## 未確認事項

- `windows-latest`にChrome/Edgeが同梱されているか(`e2e-ime.yml`はIMEの導入・`SendInput`到達は示すが、
  `chrome_probe`はCIに載っておらずChromeの有無は実証されていない)。
- `chrome_probe`のHTTP POSTがcomposition中のイベント順を欠落なく取れるか(POST自体がタイミングに影響しないか)。
- 実idle >10sと、GJI休眠(~12s)をCI上で再現できるか。BUG-002はGJI・実機依存が強い。
- 上記「代替案」の分類ヒューリスティックの昇格条件。
