---
title: v2 リリース前の実機確認 手順書（D1〜D3 と、2026-09-29 に develop へ入った CI 未検証の変更）
status: 起草（2026-09-29）。手順は該当 ADR・BUG・PR 本文から起こした。実機での実施は未了。本書の末尾「要確認」は推測で補わずに残した点
created: 2026-09-29
related_adr: ["ADR-178", "ADR-191", "ADR-199", "ADR-202", "ADR-203", "ADR-205", "ADR-206", "ADR-207"]
---

# v2 実機確認 手順書（2026-09-29）

読み手は作者本人（Windows 実機を持っている人）。CI（windows-latest）では作れないもの、つまり **物理キー押下・実 Chrome・実際の日本語 UI** が要るものだけを書く。
根拠は各項目の「戻り先」に書いた ADR・BUG・PR。書かれていない手順は補っていない（不明点は末尾「要確認」）。
関連: [v2-release-checklist-2026-09-29.md](v2-release-checklist-2026-09-29.md)（D1〜D3 の行）、[remaining-work-2026-09-29.md](remaining-work-2026-09-29.md) §1。

## 0. 優先順位と所要時間

| 順 | 項目 | 何を確かめるか | 所要 | 戻り先 |
|---|---|---|---|---|
| 1 | **D2** | OFF→1秒以内 ON→即打鍵で最初の語が cold 経路になる。ON 直後の1語の遅延 | 25分 | ADR-203 / BUG-170・171 |
| 2 | **X5** | 他プロセス注入で閉じた IME に awase が追随する（実 Chrome × GJI） | 30分 | ADR-205 / BUG-172 |
| 3 | **X1** | 無変換/変換の単独タップ。WT + GJI で「@」が出ない。Ctrl↑ で awase が IME を動かさない | 40分 | ADR-206 / BUG-113・124・174 |
| 4 | X4 | MS-IME 本体の無変換/変換=値2（トグル） | 40分 | PR #379 / ADR-199 T17 |
| 5 | D1 | 起動直後の強制 ON が無い。最初の打鍵が欠落しない | 40分（8 セル） | BUG-163 |
| 6 | X2 | 既定 `[keys]` の GJI で Alt+半角/全角（0x19）の能動経路 | 20分 | PR #367 / ADR-202 T16 |
| 7 | X3 | `engine_on/off_ime_key` 撤去のトレイ通知。`ime_detect` 既定空 | 15分 | ADR-207 / PR #373 |
| 8 | D3 | 領域A撤去後の ON 回復の実機 A/B（10回） | 20分 | ADR-178 / 09 の T4 |

D2・X5・X1 を先にやる（v2 のブロッカー〈C1〉か、再発すると直接入力に落ちる/「@」が出る類）。合計は約 3.5 時間。1日で回さず、優先順に分けてよい。

## 1. 共通の準備（全項目）

### 1.1 実機ビルドとコミットの確認

1. 確認したいコミットを **先に push** する（Windows 側は `git pull` するだけなので、push し忘れると古いコードで測る）。対象は **`develop` 先端**（本書作成時 `9ce1c33a`）。
2. `awase-build` スキル（`clipwire exec awase-build`。git pull → awase.exe 停止 → cargo build → 起動）でビルド・起動する。
3. **測る前に必ず確認して結果表に書く**（過去に古いブランチのまま測りかけた）:
   - Windows 側チェックアウトのブランチとコミット: `git -C <awase のチェックアウト> branch --show-current` と `git -C <同> log -1 --format="%h %s"`。
   - `(Get-Item .\awase.exe).LastWriteTime` がビルド直後の時刻であること、`Get-Process awase` が 1 つだけで、PID を控える。
   - バイナリにコミットハッシュは入っていない。トレイ「awase について」に出るのは `Cargo.toml` のバージョンだけ（develop は 1.21.0 のまま）なので、**コミットの証拠にならない**。
4. 設定ファイルは `awase.exe` と同じフォルダの `config.toml`。変える前に `Copy-Item config.toml config.toml.bak` で退避し、項目ごとに「前提」の差分だけ入れる。項目の終わりで戻す。

### 1.2 ログ（`awase.log`）

- 場所: `awase.exe` と同じフォルダの `awase.log`（閾値を超えると `awase.log.old` へ1世代だけ退避）。
- 既定のレベルは **info**。本書の多くの判定は **debug のログ**（`[vk-send]`、`gji fsm transition`、`[ime-io] actuation`）を使うので、debug で起動し直す（PowerShell）:

  ```powershell
  # トレイ → 終了 で awase を止めてから
  Rename-Item awase.log "awase.log.$(Get-Date -Format HHmmss)"   # 項目ごとにログを分ける
  $env:RUST_LOG = "debug"
  Start-Process .\awase.exe                                       # 環境変数は引き継がれる
  ```

  `--debug` はログを **親コンソールの stderr** に出し、`awase.log` には書かない。ファイルで残すなら `RUST_LOG=debug` を使う。
- `awase.log` は `BufWriter` 経由で、**WARN 以上は即 flush、info/debug は溜まる**。読む前にトレイの「不具合を報告...」を開くと flush される（送信しなくてよい）。読む前に末尾が欠けていたら、これをやる。
- よく使う抜き出し:

  ```powershell
  Select-String awase.log -Pattern 'gji fsm transition','\[vk-send\]','\[tsf-send\]','StaleConfirm','flush escape=true','StartComposition while engine off','\[drift\] correction','\[startup-align\]','\[external-change\]','\[shadow-toggle\]','\[ime-io\] actuation' | % Line
  ```

### 1.3 失敗したときに採るもの（全項目共通）

1. **現象の直後に** journal をダンプする（古い記録は押し出される）: ホットキー **Alt+変換 → Alt+無変換 を2回連続**（[journal-replay-guide.md](../journal-replay-guide.md)）。出力は `%TEMP%\awase_journal_<tick_ms>.json`。
2. `awase.log`（と `awase.log.old`）。
3. タスクトレイ **「不具合を報告...」**（ADR-095。journal と `awase.log` 末尾と IME キーマップが添付され `report.awase.cc` に届く）。report_id を控える（後で `bug-report-fetch` スキルで引ける）。
4. メモ: 何時何分何秒に、どのアプリ・IME・どのキーで、何が出たか（ログと突き合わせるため）。

### 1.4 記録の書き方

各項目の末尾の表に、試行ごとに 1 行。実機・ビルド情報（1.1 の 3）は項目の頭に 1 回書く。最後に該当 BUG/ADR の「実機確認」欄と、チェックリストの D 行を更新する。

---

## 2. D2: GjiFsm の OffCold 固着が無い（ADR-203 / BUG-170・171）

**目的**: OFF 前に 1 語確定して OnWarm にし、物理 OFF → 1 秒以内に物理 ON → 即打鍵したとき、ON 後の最初の語が cold 経路（`prepend_f2_warmup=true`）になり、GjiFsm が OffCold に固着しないこと。あわせて ON キー単独タップ直後の 1 語の遅延（ADR-203 の想定 30〜60ms）を再測定する。

**前提**
- IME: Google 日本語入力（GJI）。アプリ: **Edge または Chrome のページ内テキスト入力欄**（BUG-170 の報告環境は Edge / Google Meet、`Imm32Unavailable`）。
- config: 既定のまま。awase は NICOLA 有効（Engine ON）。debug ログで起動（1.2）。
- 使うキー: 普段 IME を **OFF/ON に使う物理キー**（例: OFF=Ctrl+無変換か半角/全角、ON=F2・変換・半角/全角）。ADR-203 の Reopen の発火元は「物理キー予測 ON」「shadow toggle の ON」「`sync_direction` の on キー」の 3 つなので、**使ったキーを結果表に書く**。

**手順**（10 回。ON キーが複数あるなら各キーで 5 回ずつ）
1. 入力欄にフォーカス、IME ON。トレイ/タスクバーで「あ」を確認。
2. 1 語打って **Enter で確定**（例:「これ」。OnWarm にする。入力途中や cold の状態から始めると Reopen が no-op になり判定できない）。
3. 物理 OFF キーを押す。タスクバーが「A」になったことを確認。**時刻を控える**。
4. **1 秒以内に**物理 ON キーを押す。
5. **すぐ**（0.5 秒以内）「これでいい」と打ち、目視で結果を見る。Enter で確定。
6. `awase.log` を 1.2 の抜き出しで見る（手順 3〜5 の時刻の前後）。

**期待結果**
- 画面: 「これでいい」が欠けない。先頭が生ローマ字（`koれ…`）や部分リテラルにならない。
- ログ（debug）:
  - 手順 4 の直後に `gji fsm transition` で `trigger=Reopen(BeliefSync:predict|shadow-noop|shadow-toggle)`（または `ImeOn(BeliefSync:level)`）、`state_before=OnWarm state_after=OnCold…`。
  - 手順 5 の最初の `[vk-send]`（Chrome/Edge の TSF 経路なら `[tsf-send]`）が **`prepend_f2_warmup=true`**。2 語目以降は false でよい。
  - `state_before=OffCold state_after=OffCold` の連続が無い。`StartComposition while engine off`（WARN）が無い。
  - `StaleConfirm`（WARN）と `flush escape=true` が無い（あれば BUG-171 側）。
- **遅延の測定**: 最初の語の `[vk-send]` の行の時刻から、同じ cold の `cold=N per-VK: 全 K VK 確認済み → セッション確認` の行の時刻までの差（ms）を試行ごとに記録する。ADR-203 の想定は **約 30〜60ms**（その間の後続打鍵は OUTPUT_GATE で遅れる）。体感の遅れ（気になる/ならない）も書く。

**失敗の判定**
- 最初の語が `prepend_f2_warmup=false`（warm 経路）で、生ローマ字や部分リテラルが出た → ADR-203 決定2 の既知の穴 **M3**（予測経路の Reopen は belief が変わるときしか出ない）。BUG-170 の M3 欄へ。
- `OffCold->OffCold` の連続・`StartComposition while engine off` が出る → (i)/(ii) が効いていない。BUG-170 へ戻る。
- 「これ」が消える（`escape=true`）→ BUG-171（ADR-203 で直る範囲外。BUG-170 の実機検証で消失 0 でも本バグは残りうる）。
- 入力途中（候補窓が出ている）で ON 系キーを押して未確定文字が消える → **M4**（候補窓の可視性だけで「入力中」を判定している）。
- 遅延が想定（30〜60ms）を大きく超える → 値を BUG-170 の残作業に追記（`tuning-constants.md` により定数の変更は実測付きで）。

**失敗時に採る**: 1.3 の全部。加えて手順 3〜5 の時刻、使ったキー。

| # | ON キー | 画面（欠落/リテラル） | Reopen の trigger | 最初の語 prepend_f2_warmup | 遅延 ms | OffCold 連続/`while engine off` | 備考 |
|---|---|---|---|---|---|---|---|
| 1 | | | | | | | |

---

## 3. X5: 外部から閉じられた IME に awase が追随する（ADR-205 / BUG-172）

**目的**: AutoHotkey 等の他プロセスが注入したキーで GJI が閉じたとき、awase が実状態に追随して **直接入力で一貫**する（awase は開け直さない）。その後モードキーで期待状態へ戻せる。MS-IME では偽 OFF にならない。

**前提**
- IME: GJI、アプリ: **実 Chrome のページ内入力欄**（`Imm32Unavailable`）。追随は **GJI かつ `Imm32Unavailable` のときだけ**（MS-IME・InputRelay・TsfNative は対象外）。
- config: 既定。debug ログで起動（1.2）。
- 注入用の AutoHotkey v2 スクリプト（**本書の提案**。CI は同じキーを注入している: VK_IME_OFF=0x1A、半角/全角=0xF3）:

  ```ahk
  #Requires AutoHotkey v2.0
  F9::Send "{vk1A}"    ; VK_IME_OFF を注入
  F10::Send "{vkF3}"   ; 半角/全角(0xF3) を注入
  F11::Send "{vk16}"   ; VK_IME_ON を注入（開く方向の確認用）
  ```

**手順**
1. Chrome の入力欄にフォーカス、IME ON、Engine ON。`k` `a` を打ち、「きう」になる（NICOLA の k=き・a=う）ことを確認して消す。
2. **F9**（0x1A 注入）を押す。3 秒待つ（CI と同じ間隔）。タスクバーが「A」になる。
3. `k` `a` を打つ。
4. `awase.log` に `[external-change] 監視窓の中で開閉の読みが変わった → 実状態 open=false へ追随` が **1 件**あることを確認する。
5. モードキーで戻す: 既定の **Ctrl+変換**（ime_on）を押し、`k` `a` を打つ。続けて **Ctrl+無変換**（ime_off）を押し、`k` `a` を打つ。
6. 手順 2〜5 を **F10**（0xF3）でも繰り返す。それぞれ 10 回。
7. 半角/全角トグル系のキーで戻す場合: 追随後に押して状態が変わることを見る（2 回押して期待状態になるのは許容。**何度押しても変わらない**のが固着）。
8. （任意）閉じた状態から **F11**（0x16）で開く方向。`open=true` へ追随するか（GJI のみ）。
9. **偽 OFF が無いこと（別枠）**: 追随ログが出ない通常操作で、(a) ページ本文 ↔ 入力欄のフォーカス移動、(b) アドレスバーと入力欄の往復、(c) 500ms 超の間を置いた打鍵、(d) 物理の半角/全角キー押下（従来の shadow-toggle 経路）を各 10 回。`[external-change]` が **0 件**で、IME が意図せず OFF にならないこと。
10. **MS-IME**: Win+Space で MS-IME 本体に切り替え、同じ入力欄で IME ON にして F9/F10 を押す。awase が `[external-change]` を出さないこと、通常の打鍵で Engine だけが OFF になる偽の OFF が起きないこと。GJI で ON の状態を経由してから MS-IME へ切り替えた直後にも F9 を押して確認する（古い基準値との差で偽の OFF にならないこと。ADR-205 D5）。

**期待結果**
- 手順 3: 画面が **`ka`**（IME OFF と一致）。追随しない従来は `kiu`（IME が閉じているのに NICOLA の k=き・a=う がローマ字で出る）。
- 手順 4: 追随ログ 1 件。`[drift] correction`（WARN）が出ない（awase は開け直さない）。
- 手順 5: Ctrl+変換の後 `きう`、Ctrl+無変換の後 `ka`。
- 手順 9・10: 追随ログ 0 件、偽 OFF 0 件。

**失敗の判定**
- 手順 3 が `kiu` のまま／追随ログが出ない → 追随が働いていない（ADR-205 D1〜D3。窓は注入の 300ms 内）。CI は GJI × 実 Chrome で 10/10 だったので、**実機だけ落ちる**なら実機固有（HKL・キーボード・常駐ソフト）。
- 手順 5 でモードキーを押しても状態が変わらない → **固着**。ADR-205 D6・ADR-208（検出できない stale で絶対指定キーが省略され続ける。v2 のブロッカーにはしないが、発生したら BUG を起票）。
- 手順 9・10 で `[external-change]` が出て IME/Engine が意図せず OFF → **偽 OFF**（ADR-205 D3・D5 のリスク表の最大項目）。BUG-172 へ。

**失敗時に採る**: 1.3 の全部。加えて AHK のスクリプト、Chrome のバージョン、押したキーの順と時刻。

| # | 注入キー | 追随ログ | 手順3の画面 | 手順5（戻し）の画面 | 偽OFF | 備考 |
|---|---|---|---|---|---|---|
| 1 | | | | | | |

---

## 4. X1: 無変換/変換の単独タップ（ADR-206 / PR #376。BUG-113・124・174 の再発確認）

**目的**: 単独タップ再設計の実機確認。**Windows Terminal + GJI で「@」が出ない**こと、**Ctrl↑ で awase が IME を動かさない**こと、GUI T3 の旧 `"off"` 設定の移行後の挙動。「@」の実機 A/B は PR #376 のマージ条件から外れた（未実施）ので、ここが初めての実測になる。

**規則の要約（ADR-206 決定1）**: 親指キー（無変換/変換）の単独タップについて、
- **Suppress（既定）**: IME を動かさない。生キーを飲み込むだけ。ただしエンジン非活性（IME OFF）中は生キーがそのまま IME に届く（所有者が仕様として受け入れた一方向）。
- **Passthrough**: そのキーが IME 側でトグルなら、生キーを抑止し、awase が belief に従う絶対指定の ON/OFF を 1 回だけ inject する。
- **bare `keys.ime_on/off/toggle` に親指を書いた場合（S1）**: 単独タップの設定に関係なく発火する。

**前提（共通）**
- アプリ: **Windows Terminal + PowerShell**（`WindowsTerminal.exe`。BUG-113 の環境）。IME: GJI、キー設定を **CUSTOM** にして、**無変換（と変換）が「IME の有効化/無効化」のトグル**（DirectInput で ON、全ての開状態で OFF、ON/OFF 行あり。ADR-199 決定11 の判定を満たす表）。この作り方は docs に手順が無いので、既に使っている設定でよい（要確認）。
- debug ログで起動（1.2）。「半角状態」= IME OFF。

### X1-1 既定（Suppress）

config: 既定（`[general] muhenkan_solo_tap_always_suppress = true`、`henkan_…` も同じ）。

1. IME ON・Engine ON の状態で、無変換を単独タップ 10 回。
2. IME OFF（半角）にして、無変換を単独タップ 10 回。変換も同じ。
3. 各回、画面に「@」が出るか、IME が開閉するかを記録。

期待: 手順 1 は **IME は動かず**、生キーは飲み込まれる（`[ime-io] actuation SendInput` が出ない）。手順 2 は仕様どおり生キーが GJI に届いて GJI 自身が開く。**「@」が出るかを記録する**（ADR-206「未検証事項」: この構成の「@」の有無は未確認）。

### X1-2 Passthrough（役割由来）

config: `[general] muhenkan_solo_tap_always_suppress = false` と `muhenkan_solo_tap_ignore_composing_guard = true`（変換も `henkan_…` を同様に）。

1. IME OFF（半角）で無変換を単独タップ 10 回。
2. IME ON（未入力）で無変換を単独タップ 10 回。
3. 変換も同様。

期待: 押すたびに **生キーは抑止され**（GJI に届かない）、awase が **1 回だけ**絶対指定を送る（OFF→ON、ON→OFF）。「@」が出ない。ログは `[shadow-toggle]`（info）と `[ime-io] actuation SendInput kind=kanji_marker`（debug）が 1 押下につき 1 回。二重トグル（押して開き、離して閉じる）が無い。

### X1-3 GUI T3 の旧 `"off"` 設定の移行（S1）

config（GUI T3 が書いていた旧主流設定）: `[general] muhenkan_solo_tap_ime_action = "off"` と `muhenkan_solo_tap_always_suppress = true`。

1. awase を起動する。ログ/トレイに「`*_solo_tap_ime_action` は非推奨」の警告が出ることを確認（config.toml は書き換わらない。読込時にメモリ上で bare `ime_off` に「無変換」を足した扱いになる）。
2. IME ON で無変換を単独タップ → 閉じる。
3. IME OFF（半角）で無変換を単独タップ 10 回 → **awase が絶対指定の `VK_IME_OFF` を単発で送る**（旧ケース3改の「抑止のみ」ではない）。**「@」が出るか**を記録。ここが ADR-206 決定3 の代償で、BUG-124 の旧ケース3と同じ構成（未検証）。
4. 設定画面（`awase-settings.exe`）の「IME ON/OFFキー」で「変更内容をプレビュー」→「この置き換えを適用」を押し、`keys.ime_on` に「変換」、`keys.ime_off` に「無変換」が追記される（既存の `Ctrl+…` は残る）ことを確認し、2〜3 を再度行う。

「@」が出た場合の所有者判断の選択肢は ADR-206 決定3（iii）（間に他のキーを挟まず同じ OFF キーを 2 回続けたときだけ送る）。

### X1-4 Ctrl↑ で awase が IME を動かさない（BUG-113/124/174 の再発確認）

config: 既定と X1-2 の両方で。

1. WT + GJI で **Ctrl を押して離す**（単独）を 20 回、**Ctrl+Shift** の操作を 20 回、**Ctrl+無変換**（既定の ime_off）と **Ctrl+変換**（既定の ime_on）を各 10 回（Ctrl を離す動作を含む）。
2. 各回、「@」が出るか、Ctrl↑ の**直後に** awase の IME 操作が走るかをログで見る。

期待: 「@」が出ない。Ctrl↑ の直後に `kanji_marker`/`VK_IME_ON` の送信（`EmitWarmup(CtrlUp)` 相当）が無い（ADR-206 決定 Ctrl↑ 条項。BUG-174 は旧 CtrlUp の eager warmup が Ctrl 押下中に `VK_IME_ON` を注入していた）。範囲外の被疑（ADR-206 round5）: 直前に観測で IME 状態が変わった場合の `ActivationSync`、BUG-175（修飾キー押下中の eager warmup、未マージ）。それらで出た場合は別物として記録する。

**失敗の判定と戻り先**
- 「@」が出る → BUG-113（GJI の TSF キー横取り）・BUG-124（旧ケース3の再導入）。どの構成（X1-1/2/3）で出たかを記録して ADR-206 決定3 へ。
- Suppress なのに IME が動く／Passthrough なのに動かない → ADR-206 決定1 の分岐（S1/S2 の入力の取り違え）。
- 1 押下で開閉が 2 回起きる → ADR-206 決定3（リピートで指令を作らない `phase1_held`・`was_down`）。
- 何度押しても状態が変わらない → **固着**（所有者の定義）。ADR-206 決定7・ADR-205 D6・ADR-208。
- Ctrl↑ 直後に IME 操作が走る → BUG-174/175。

| # | 構成(X1-n) | アプリ | 操作 | 「@」 | IME の開閉（前→後） | 押下あたりの送信回数 | 備考 |
|---|---|---|---|---|---|---|---|
| 1 | | | | | | | |

---

## 5. X4: MS-IME 本体の無変換/変換=値2（トグル）（PR #379 / ADR-199 T17 Phase 4）

**目的**: 値 2（IME-オン/オフのトグル）のとき、awase が無変換/変換を役割として扱い、状態に関係なく（直接入力・入力中・変換中・候補窓・確定直後）belief に従う明示 ON/OFF を inject すること。**値 2 は CI で作れず、ホストテストしかない**ので、実機が唯一の検証。入力中に未確定文字列を捨ててよい（所有者了承）。

**前提**
- IME: **MS-IME 本体**（TIP を Microsoft IME に）。**日本語 UI の実機**。
- MS-IME の設定: 設定アプリ（`ms-settings:regionlanguage-jpnime`）→「全般」→「キーとタッチのカスタマイズ」→「キーの割り当て」を ON にし、**無変換 と 変換 を「IME-オン/オフ」**（値 2）にする。同じ画面の **「以前のバージョンの Microsoft IME を使う」は OFF**（ON だと割り当てが効かない。`NoTsf3Override2=1`。ADR-199 T12）。レジストリを直書きしても反映されない。
  確認: `reg query "HKCU\Software\Microsoft\IME\15.0\IMEJP\MSIME"` で `IsKeyAssignmentEnabled=1`、`KeyAssignmentMuhenkan=2`、`KeyAssignmentHenkan=2`、`NoTsf3Override2` が無いか 0。
  UI 操作を自動化する場合は [v2-b4-msime-toggle-phase4-plan-2026-09-29.md](v2-b4-msime-toggle-phase4-plan-2026-09-29.md) §3.1 の `msime_key_assignment_settings_probe`（UIA で設定アプリを操作、`SystemSettings.exe` を強制終了する副作用あり）。終わったら元の値へ戻す。
- awase の config: **役割由来の発火は単独タップが Passthrough のときだけ**（ADR-206 決定1 の訂正）。`[general] muhenkan_solo_tap_always_suppress = false`、`muhenkan_solo_tap_ignore_composing_guard = true`、`henkan_solo_tap_always_suppress = false`、`henkan_solo_tap_ignore_composing_guard = true`。ignore_composing_guard を true にしないと入力中は Suppress になり発火しない。
- アプリ: メモ帳。debug ログで起動（1.2）。

**手順**（無変換/変換 × 各状態を 3 回ずつ。状態ごとに前後の IME 開閉と未確定文字列を記録）
1. 直接入力（IME 閉）→ 単独タップ。
2. アイドル（IME 開・未入力）→ 単独タップ。
3. 入力中（ローマ字で「あい」まで、未確定）→ 単独タップ。
4. 変換中（Space 1 回）→ 単独タップ。
5. 候補窓表示中（Space 2 回）→ 単独タップ。
6. 確定直後（Enter の直後、数秒待ってから）→ 単独タップ。20ms/300ms 後は人手では揃えられないので、確定直後すぐと数秒後の 2 通り。
7. 対照（任意）: awase を止めて同じ手順を行い、MS-IME 本来の挙動（入力中は開閉せず、かな⇔カタカナ変換などになる）を確認しておく。

**期待結果**
- 1: 開く（閉→開）。2: 閉じる（開→閉）。押すたびに反転する。
- 3〜5: awase が絶対指定の OFF/ON を送り、**状態に関係なく開閉が反転する**（未確定文字列は消えてよい）。MS-IME 本来の「入力中は開閉しない」に負けない。
- 6: アイドルと同じく反転する。
- どの状態でも **1 押下で開閉が 2 回起きない**。押しても状態が変わらない状態が無い。2 回押しで期待状態になるのは許容。
- 起動時に、MS-IME のキー割り当ての **値 2 に関する警告が出ない**（PR #379 で `conflict_warning` から値 2 を外した）。
- ログ: `[shadow-toggle]` と `[ime-io] actuation SendInput kind=kanji_marker vk=[16]/[1A]` が 1 押下につき 1 回。

**失敗の判定と戻り先**
- 何も起きない（awase が発火しない）→ まず config が Passthrough か、`reg query` の値、互換モードを確認。それでも駄目なら PR #379 の `msime_native_key_role` の腕（ADR-199 T17）。
- 入力中/変換中だけ反転しない → ADR-199 決定16・T17 M1（MS-IME 本体は入力中に「IME-オン/オフ」割り当てが発火しない）に awase が負けている。PR #379 の前提（`ctx.composing` は MS-IME 本体で偽のまま、除外を採らない）へ。
- 二重トグル → ADR-206 決定3（リピート/`was_down`）。
- 押しても変わらない → **固着**。ADR-206 決定7。MS-IME × 実 Chrome では awase 自身の `VK_IME_OFF` が効かない記録がある（ADR-205、ADR-208 の前提）ので、メモ帳で通ったら Chrome でも 1 回だけ試す。

| # | キー | 状態 | 前→後（開閉） | 未確定文字列 | 二重トグル | ログ送信回数 | 備考 |
|---|---|---|---|---|---|---|---|
| 1 | | | | | | | |

---

## 6. D1: 起動直後の強制 ON が無い（BUG-163）

**目的**: awase 起動直後に、IME を閉じていても awase が開けに行かないこと（ADR-191 決定1）、IME が開いているときに起動直後の最初の打鍵が欠落しないこと（先同期が最初の観測まで最大約 0.5 秒遅れる）。CI（develop `e174c6f6` の e2e-ime run 36506566832）では起動直後の drift が 0 件、GJI・MS-IME 双方で確認済み。ここで見るのは **実打鍵と体感**。

**前提**
- 8 セル: {GJI, MS-IME 本体} × {メモ帳, 実 Chrome の入力欄} × {IME ON で起動, IME OFF で起動}（Chrome 用の GJI の起動元 TIP を切り替えるときは Win+Space）。MS-IME 本体は互換モードの状態も控える。
- config: 既定。debug ログで起動（1.2）。

**手順**（セルごとに、awase を止めてログを退避 → 対象アプリに IME の状態を作る → awase を起動）
1. `IME ON で起動`: 対象アプリの入力欄に IME ON でフォーカスした状態で awase を起動し、**起動から 1 秒以内に**「これでいい」を打つ。
2. `IME OFF で起動`: IME OFF（「A」）でフォーカスした状態で awase を起動し、**3 秒何もしない**。その間に IME が勝手に開かないこと、`A` のままであることを見る。その後、普段の ON キーで開けて打鍵する。
3. ログ（1.2）を見る。

**期待結果**
- ON で起動: 最初の打鍵から欠落・リテラル化が無い（`これでいい` が全部出る）。
- OFF で起動: **IME は閉じたまま**（awase が開けない）。体感として「起動したら勝手に IME が開いた」が無い。
- ログ:
  - `[drift] correction`（WARN）が起動後 30 秒で **0 件**。
  - `[startup-align] desired_open を最初の成功観測へ揃えた: desired=…` が起動直後に 1 回。ON で起動なら `desired=true`、OFF なら `desired=false`。
  - OFF で起動: `[ime-io] actuation SendInput kind=kanji_marker vk=[1A, 16]`（VK_IME_OFF→VK_IME_ON の reinit）が起動直後に **無い**。
- **観察項目（合否は所有者判断）**: Chrome など「キャッシュの無い窓」への初回フォーカスでは、`Imm32Unavailable entry without trusted cache: 安全デフォルト ON …`（`[focus] Imm32Unavailable hard pre-sync applied=true`）が出て、IME を閉じたままでも awase が開けうる（BUG-163 が「別途実機で確認、未確認のまま」と記載した経路）。OFF で起動した Chrome セルで IME が開いたか、ログに上記が出たかを記録する。

**失敗の判定**
- OFF で起動して IME が開く／`kanji_marker vk=[1A, 16]` が出る → BUG-163 の 2 段目（`desired_is_placeholder`）か、別経路（`reset_stale_ime_on_for_imm_broken`。awase 自身のウィンドウ以外のキャッシュ無し窓）。
- `[drift] correction` が起動直後に出る → BUG-163 の 1 段目（授権が下りない補正の抑止）。
- ON で起動して最初の打鍵が欠ける → 先同期の遅延（約 0.5 秒）が実害になっている。BUG-163「未確認」欄。

**失敗時に採る**: 1.3 の全部、起動から打鍵までの秒数。

| # | IME | アプリ | 起動時の状態 | 最初の打鍵の欠落 | IME が勝手に開いた | drift 件数 | startup-align | 備考 |
|---|---|---|---|---|---|---|---|---|
| 1 | | | | | | | | |

---

## 7. X2: 既定 `[keys]` の GJI で Alt+半角/全角（PR #367 / ADR-202 T16）

**目的**: `keys.ime_toggle` の既定を空にした（旧既定 `VK_KANJI` は GJI の 0x19 の役割判定を常に無効にしていた）ので、**既定設定の GJI で 0x19 の能動経路（Suppress して awase が明示 ON/OFF）が初めて動く**。その実機確認（e2e 常設の `sc-kanji-role-toggle`/`-nontoggle` も既定 `[keys]` で走っていたが受動のまま通っていた可能性が高い）。

**前提**
- IME: GJI。アプリ: メモ帳と Chrome の入力欄。config: **`keys.ime_toggle` を書かない**（旧 GUI が書いた `ime_toggle = ["VK_KANJI"]` が残っていれば消す。明示値は尊重されて受動のままになる）。debug ログ。
- GJI のキー設定を 2 通り: (a) プリセット（半角/全角がトグル）、(b) CUSTOM で `Hankaku/Zenkaku` 行を `IMEOff` 以外（トグルにしない）にした表。

**手順**（各 10 回）
1. (a) で IME OFF から **Alt+半角/全角**、続けて Alt+半角/全角、を交互に。押すごとの IME の開閉と Engine の追随（かな/英字）を見る。
2. (b) で同じ。
3. 各回のログ `[shadow-toggle]`。

**期待結果**
- (a): 押すたびに **1 回だけ**開閉が反転し、Engine が IME に追随する。二重トグルなし。
- (b): IME が動かない（行がトグルでないので awase も書かない）。Engine と IME がずれない。
- 明示 `VK_KANJI` を持つ既存 config のユーザーは従来どおり受動。

**失敗の判定**: (a) で反転しない/二重に反転/Engine とずれる、(b) で awase が書く → ADR-202 の `kanji_shadow_action`（`Derive`）と `explicit_overlap`、PR #367 の未検証点。
**失敗時に採る**: 1.3、使った GJI の keymap（`config1.db` の該当行）。

| # | GJI キー設定 | アプリ | 操作 | 開閉（前→後） | Engine 追随 | 二重トグル | 備考 |
|---|---|---|---|---|---|---|---|
| 1 | | | | | | | |

---

## 8. X3: `engine_on/off_ime_key` 撤去の通知と `ime_detect` 既定空（ADR-207 / PR #373）

**目的**: 撤去した設定が旧 config に残っているとき、黙って挙動が変わらず**トレイで通知**されること。`keys.ime_detect.on/off` の既定を空にしても 0x16/0x1A の追随が変わらないこと。

**前提**: config に手で `[keys]` の `engine_on_ime_key = "VK_DBE_DBCSCHAR"`、`engine_off_ime_key = "VK_DBE_SBCSCHAR"` を書く。debug ログ。

**手順**
1. awase を起動し、トレイの通知（バルーン）を見る。
2. Engine を `Ctrl+Shift+変換`（ON）/`Ctrl+Shift+無変換`（OFF）で切り替える（既定の `engine_on/off`）。
3. awase-settings.exe で何か 1 項目を変えて保存し、`config.toml` から 2 行が消えたか見る。再起動して通知が出ないことを見る。
4. （キーが手元にあれば）物理の IMEオン/IMEオフ キー（0x16/0x1A）を押し、belief（Engine）が追随するか。

**期待結果**
- 1: 「`keys.engine_on_ime_key` は撤去されました。値は無視されます。…（設定画面で保存しても消えます）」の通知。内容が同じなら再表示しない。
- 2: Engine ON/OFF に合わせた IME モードキー（0xF4/0xF3）の送信が**無い**（`[ime-io] actuation SendInput` が出ない）。
- 3: 2 行が消え、再起動後は通知なし。
- 4: 従来と同じく追随（静的 `shadow_action` が `is_japanese_ime()` を問わず採用される）。

**失敗の判定**: 通知が出ない → `REMOVED_WITH_NOTICE`（`config_load_diag.rs`）。IME モードキーがまだ送られる → 撤去の連鎖削除漏れ（ADR-207 決定2）。0x16/0x1A で追随しない → ADR-207 決定1（`is_static_idempotent_open_key`）。
**失敗時に採る**: 1.3、`config.toml`。

| # | 確認 | 結果 | 備考 |
|---|---|---|---|
| 1 | 通知 | | |

---

## 9. D3: ADR-178 領域A撤去（reassert・force-on）後の ON 回復の実機 A/B（09 の T4・A/B-2）

**目的**: reassert と force-on を撤去した（`f83084b3`・`621bf93c`）あとの TsfNative で、drift correction だけで ON 回復が成り立つか。実施記録はリポジトリ内に無い（未確認）。**物理 Ctrl が要るので実機のみ**（SendInput では作れない）。BUG-163 の修正は develop に入っているので **develop 先端 1 本**で測る。

**位置づけ**: [review-2026-09-24-09](review-2026-09-24-09-remaining-active-writes-inventory.md) の「実機 A/B 手順」A/B-2。CI の代替観測（同文書の「CI での代替観測」）では、実 Chrome × GJI は drift correction が判断に届かない（observed=0）と分かっているので、**Chrome では発火 0 が想定**。結果はそれと突き合わせる。

**前提**
- アプリ: Chrome または VS Code の入力欄（TsfNative）。IME: GJI（できれば MS-IME 本体も）。
- config: 既定（OFF キーは `Ctrl+無変換`）。debug ログ。
- 「ずれ」= OFF 操作のあとも実 IME が ON のまま（または ON に戻る）。実状態は「かな（IME ON）か英字（IME OFF）か」を 1 キー打って判定する（API の成功表示だけで判断しない）。

**手順**（10 回）
1. IME ON で日本語入力できる状態にする。
2. **Ctrl+無変換**（または設定中の OFF キー）で OFF にする。押したキーを記録。
3. 直後（約 0.5 秒）と約 2 秒後に、文字キーを 1 つ打って、かな/英字のどちらかを記録。
4. ログの `[drift] correction`（WARN）と `Blacklist drift correction: apply_ime_open(…)`（info）の有無、発火後の実タイピング結果を記録。
5. 10 回終わったら結果を次で分岐:
   - ずれが作れない → 「作れないので drift correction は撤去せず残す」と `docs/adr/191-calibration-experiments.md` に記録して T4 終了。
   - ずれが作れた → 発火の有無と、発火後に正しく ON/OFF になったか。

**期待結果**: 分岐のどちらでもよい（結果の記録が目的）。ただし OFF 操作が効かない（実 IME が ON のまま）なら、drift correction の発火の有無で回復可否が決まる。Chrome では発火 0 が想定（CI 観測）。

**失敗の判定**: OFF が効かずに ON のまま固定され、drift correction も発火せず回復しない → 領域A撤去の回復力低下（ADR-178 領域A、ADR-179〈旧178〉）。**撤去を取り下げる（revert する）場合に限り `docs/experiments.md` に 1 行**足し、コミット本文に観測した失敗（アプリ・IME・症状）を書く（`experiment-logging.md`）。
**失敗時に採る**: 1.3 の全部。**結果の記録先**: `docs/adr/191-calibration-experiments.md`（1 試行 1 行: 日時・アプリ・IME・押したキー・約 0.5 秒後/約 2 秒後の一致・drift 発火の有無）。

| # | アプリ | IME | 押したキー | 約0.5秒後 | 約2秒後 | drift 発火 | 発火後の実タイピング | 備考 |
|---|---|---|---|---|---|---|---|---|
| 1 | | | | | | | | |

---

## 10. 実施後に更新するもの

- チェックリスト [v2-release-checklist-2026-09-29.md](v2-release-checklist-2026-09-29.md) の D1〜D3。
- BUG-163・BUG-170（残作業の「実機確認（最優先）」欄）・BUG-172 の frontmatter/本文の「実機未検証」。
- ADR-203・205・206・207 の status の「実機確認は未実施」。ADR-206 の「未検証事項」（「@」の有無）。
- `remaining-work-2026-09-29.md` §1。

## 要確認（推測で補わず残した点）

1. **`awase-build` が debug ログで起動するか**、Windows 側のチェックアウトの場所とブランチ。本書は「止めて `RUST_LOG=debug` で手動起動」に倒した。`awase-build` スキルはこのリポジトリの追跡下にない（メインのチェックアウトの `.claude/skills/awase-build` にある）。
2. **`awase.log` を読む前の flush 方法**。「不具合を報告」を開くと flush される（コードのコメントによる）が、通常の終了で flush されるかは未確認。
3. **journal ダンプのホットキー**は「Alt+変換 → Alt+無変換 を 2 回連続」（journal-replay-guide と `handle_wm_dump_journal` のコメント）。「連続」の意味（1 組を 2 回か）と、現行ビルドで動くことは未確認。
4. **D2 の ON キーの種類**。ADR-203 の CI 版は注入 0xF3 等。実機の普段のキーが 3 つの発火元（物理キー予測・shadow toggle・`sync_direction`）のどれに当たるか、M3 の穴（belief が既に ON の予測経路では Reopen が出ない）に当たるかは環境依存。
5. **D2 の遅延の測り方**。ADR-203 は「`[vk-send]` から `セッション確認` まで」。Chrome/Edge の TSF 経路のログ名が `[vk-send]` か `[tsf-send]` か、`log_tag` が何かは実ログで確認が要る。
6. **D1 のブラインド窓（Chrome 初回フォーカス）で awase が IME を開けるか**は BUG-163 が未確認としており、合否基準が無い（観察項目にした）。
7. **X1/X4 の Passthrough 前提**。PR #379 の本文は「状態に関係なく」とするが、コード（`thumb_open_role_action`）とADR-206 決定1 では **役割由来（S2）は単独タップが Passthrough のときだけ発火**。**既定（Suppress）では MS-IME 値2の役割は発火しない**。既定でも動かしたい意図なら設計の食い違い。所有者の確認が要る。
8. **X1 の GJI CUSTOM 表の作り方**（無変換=トグルにする画面操作）は docs に無い。ADR-199 決定11 の判定条件だけ引いた。
9. **X1-3 の GUI ボタン**の表示条件（`adr192_replacement_controls_visible`。「状態依存のキーを検出した警告」が前提）。ボタンが出ないときの扱いは未確認。
10. **X3 の物理 0x16/0x1A**: 一般的な JIS キーボードに無い。injected は BUG-14 で空振りするので、キーを持つ実機が無ければ手順 4 は実施不能。トレイ通知の表示形式（バルーン/トースト）も未確認。
11. **X5 の AutoHotkey スクリプトは本書の提案**。ADR-205 は「AutoHotkey や CTF の注入」、CI は 0x1A・0xF3 の注入。`Send` が AHK の設定によって injected 扱いになる/ならないか（目印なしの注入として awase に見えるか）は実機で `[hook] IME-mode … injected=true` を debug ログで確認すること。
12. **D3 は手動では +100/+400/+1500ms を測れない**ので、約 0.5 秒/約 2 秒で代替した（09 の記録形式との差）。
13. **X4 の確定直後（20ms/300ms 後）** は人手では揃えられない。ハーネス（`msime_native_composing_probe --matrix`）は測定用ブランチ `ci/b4-msime-toggle-probe` にあり develop に無い。
