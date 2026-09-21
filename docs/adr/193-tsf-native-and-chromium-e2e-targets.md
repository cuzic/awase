---
id: ADR-193
title: |-
  TSFネイティブ入力先(自プロセスRichEdit)と実Chrome/Edge(静的HTML+title回収)を実機E2Eの検証対象に加える設計
summary: |-
  現状の実機E2E(`e2e_windows.rs`)の入力先は自前のIMM32 `Edit`コントロールとWindows Terminal
  (TsfNative)のみで、Windows Terminalは確定文字列を`WM_GETTEXT`系で読めず、CIでは前面化・
  フォーカスも不安定(`continue-on-error`扱い)。過去に再発を繰り返したChrome系の不具合
  (BUG-002: cold-startのリテラル化、TSF context再初期化~326ms等)は、自プロセスの素直な
  コントロールでは再現しない。決定1: TSFネイティブ検証用に自プロセスRichEdit(`RICHEDIT50W`)を
  追加するが、**先に`AppKind`がTsfNativeに分類されるかを実測**し、Win32のままなら作らない
  (go/no-go)。決定2: Chromium系の検知は**実Chrome/Edgeに静的HTMLを開かせ、`document.title`へ
  状態を書いて`GetWindowTextW`で回収**する(通信・CDP不要)。Tauri(WebView2)は不採用、Electronは
  後回し。決定3: キー入力は必ずOSの`SendInput`(CDPの`Input.dispatchKeyEvent`はawaseのフック・
  IME・TSFを迂回するため禁止)。決定4: ハーネスは`tests/`・`examples/`に閉じ、本体ソースは変更しない。
status: |-
  **提案(ドラフト)**。未実装・未検証。決定1のgo/no-go判定と決定2の再現確認(BUG-002を再現できるか)は
  スパイクの結果を待つ。本ADRの前提のうち「RichEditがTsfNativeに分類されるか」
  「Chrome/EdgeがCI(`windows-latest`)で同条件のcold-startを再現するか」は未確認。
related_adr:
  - "ADR-0002"
  - "ADR-0003"
  - "ADR-159"
  - "ADR-163"
---

# ADR-193: TSFネイティブ入力先とChromium系の実機E2E検証対象

## 背景

awaseの難所はIME状態の追跡・補正であり、特にTSFネイティブ(Chrome/VS Code/WezTerm/Windows
Terminal、Windows 11のメモ帳=`RichEditD2DPT`)で不具合が再発してきた
(例: [BUG-002](../known-bugs/BUG-002.md) Chrome cold-startのリテラル化、
`という→toいう`、`bあ`)。一方、実機E2Eの入力先は次の通りで、これらの再現に穴がある。

- `crates/awase-windows/tests/e2e_windows.rs`は自前の`CreateWindowExW`で作るIMM32
  `Edit`コントロール(503〜555行付近)で、IMM32クロスプロセスが素直に効く。TSFネイティブ固有の
  「IMEが自分の状態を偽る」「context再初期化に~326ms」は起きない。
- TsfNativeの実機検証はWindows Terminal(2571行以降のBUG-13 Vk-mode gap節)のみ。確定文字列を
  プロセス外から読めず、前面化・フォーカスがCIで不安定(`continue-on-error`)。
- Chrome系は実機手動確認とジャーナルリプレイ(`journal_replay.rs`)に依存している。

検知したいのは「Chromeが最終的に何を受け取ったか」(リテラル漏れ・部分リテラル化・欠落)である。

## 決定

### 決定1: 自プロセスRichEditターゲットを、分類の実測を条件に追加する

TSFネイティブ入力先として、ハーネス内に`RICHEDIT50W`(`msftedit.dll`を`LoadLibrary`)を作る。
確定文字列は`EM_GETTEXT`/`WM_GETTEXT`で厳密にassertでき、フォーカスも`SetFocus`で決定的に取れる。
コストは既存の`Edit`生成ヘルパーの共通化で数十〜百行程度の見込み。

**ただしgo/no-goを先に判定する。** `focus/classify.rs`は`RICHEDIT50W`を「既知のテキストクラス」
として`TextInput`にする(101〜105行)が、`AppKind`が`TsfNative`になるかは学習キャッシュや
`Imm32Unavailable`判定次第で未確認。awase常駐下でフォーカスし、`AppKind`をログで確認する。

- `TsfNative`になる → Windows Terminal依存のテスト(BUG-13 Vk-mode gap系)の置き換え先と、
  `sc-*`シナリオの入力先の差し替え候補にする。
- `Win32`のまま → TsfNative経路(Vk注入・force-on・warmup)を通らず目的を果たさないので**作らない**。

### 決定2: Chromium系の検知は実Chrome/Edge + 静的HTML + title回収

Chrome/Edgeを`--user-data-dir=<一時>`・`--no-first-run`・`--app=file:///…`(または通常起動)で
起動し、`<textarea>`を持つ静的HTMLを開く。JSは`beforeinput`/`input`/`compositionstart`/
`compositionupdate`/`compositionend`のたびに、textareaのvalueとイベント列を**`document.title`**へ
反映する。ハーネスは`GetWindowTextW`でtitleを読み、確定文字列・`compositionend.data`をassertする。

回収経路の優先順位:

1. `document.title` + `GetWindowTextW`(通信・追加依存なし。titleの長さ制限内の短い文字列向け)
2. ローカルHTTPへPOST(自前`TcpListener`。イベント列の詳細が要るとき)
3. CDP(`--remote-debugging-port`)。**`Runtime.evaluate`による読み取り専用の補助に限る**。

対象ごとの判断:

| 対象 | 判断 | 理由 |
|---|---|---|
| 実Chrome / Edge | **採用** | 壊れている対象そのもの。`windows-latest`に同梱の想定(未確認) |
| Tauri (WebView2) | **不採用** | Chromium TSF実装は共通だが、ホストウィンドウ構成がChrome本体と異なり、Chrome特有のcold-startが同条件で出る保証がない。Tauriビルドがワークスペースに増える |
| Electron(VS Code相当) | **後回し** | 実利用は多いがハーネスとして重い。決定2の効果確認後に検討 |

### 決定3: キー入力は必ずOSの`SendInput`

CDPの`Input.dispatchKeyEvent`はChromium内部へ直接注入するため、awaseのフック・IME・TSFを
すべて迂回し、検出対象のバグが再現しない。**CDP経由のキー注入は禁止**。フォーカスは既存の
強制前面化ヘルパー(`e2e_windows.rs`)で取る。

### 決定4: 複雑性を増やさない置き場所

ハーネス(RichEdit生成、HTML、title読み取り)は`tests/`または`examples/`に閉じ、本体
(`src/`・`crates/awase-windows/src/`)は変更しない。actuation合流点の許可リスト・tuning定数は
触らない(complexity-budgetの対象外)。

## 検証計画(スパイク)

専用worktreeで進める(worktree-per-session規約)。

1. RichEditを作りawase常駐下でフォーカス→`AppKind`をログ確認(決定1のgo/no-go)。
2. 検証HTMLを1枚書き、既存E2Eハーネスから実Chromeを起動、`SendInput`で入力してtitleを読む。
3. **BUG-002(Chrome cold-start)を再現できるか**を確認する。cold-startの再現には、フォーカス直後
   の入力タイミングを制御する必要がある(idle時間を変えて「フォーカス→N ms→入力」を掃引)。
4. 再現できなければ、CDP読み取りで入力context・イベント順を観察する案に進む。

### 成功基準

- 手動で再現していたChromeの症状(`という→toいう`、`bあ`)のうち少なくとも1つを、修正前の
  コミットで**再現でき、修正後で消える**ことをE2Eで示せる。
- 同じ入力を連続実行して結果が安定する(CIでflakeしない)。

## 検討した代替案

- **Tauri/WebView2をハーネスにする**: 上表の通り不採用。「Tauri製アプリ対応」を独立目標にするなら別ADR。
- **CDPで入力もページ操作もする**: 決定3により、入力側は不可。読み取り専用に限れば補助として可。
- **ジャーナルリプレイのみで済ませる**: 引き続き主軸だが、Chromeが実際に受け取った文字列は
  観測できない。E2Eはその補完。

## 制約・未確認事項

- JS側で見えるのは「Chromeが最終的に受け取ったもの」で、原因の切り分けにはawase側ジャーナルとの
  突き合わせが要る。
- `document.title`更新はChromeの再描画を伴う。タイミング(cold-start)に影響しないかを決定2の
  検証で確認する(影響するなら経路2へ切り替える)。
- RichEdit/Chromeが実際にどのIME(GJI/MS-IME)・状態で再現するかは動かして確かめるまで不明。
