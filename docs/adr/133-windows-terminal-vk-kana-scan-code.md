# ADR-133: Windows Terminal で物理 VK_KANA/VK_KANJI が文字化する問題の配送方針

## ステータス

**収束済みドラフト（v4、Sol 2体の敵対的レビュー round4 で収束）。**
2026-09-04、Windows Terminal + 日本語 IME で `VK_KANJI` / `VK_KANA`
押下時に余分な「@」が出る報告を受け、Windows Terminal と日本語 106
キーボード配列の公開ソースを読んだ上で起票した。v1 は
「`VK_KANA` を `VK_DBE_HIRAGANA` + scan 付き送信へ置換する」ことを
決定として書きすぎており、Sol-A/Sol-B の敵対的レビューで blocking 13件
相当の指摘を受けた。v2 では採用決定ではなく **Windows Terminal 限定の
hidden 実験**へ格下げし、所有権・intent・modifier・F2 warmup との分離条件を
明文化した。round2 ではさらに `SendInput` 全件成功と Suppress の結合、
KeyUp latch、`try_hold_key` より前の配置、`MapVirtualKeyW(VK_DBE_HIRAGANA)`
の runtime preflight が不足していると指摘されたため、v3 で反映した。
round3 では部分 `SendInput` 成功時に元キーを consume する逃げ道が残っている
と指摘され、v4 で削除した。Sol-A/Sol-B ともに「Windows Terminal 限定の
hidden 実験計画として収束」と判定した。実機検証は未実施。

## 背景

ユーザー報告では、Windows Terminal 上で `VK_KANJI` または `VK_KANA` を押すと
本来の IME 操作ではなく「@」が出ることがある。awase 側の現状確認では、
主要な IME キー注入経路は `wScan=0` で送っている:

- `ime.rs::send_ime_mode_key(vk)` は `tsf/output.rs::make_key_input_ex()` を
  使い、`KEYBDINPUT.wScan = 0` を設定する。
- `ime.rs::post_kanji_toggle_to_focused()` の `VK_KANJI` 送信も
  `make_key_input_ex(VK_KANJI, ...)` 経由で `wScan=0`。
- `RawKeyEvent::reinject()` も元イベントの `scan_code` を保持せず、
  `wScan=0` で再注入する。

一方で、awase には例外的に scan code 付きで送る経路がある:

- `tsf/output.rs::make_scan_key_input(vk, ...)` は `wVk` を保持したまま
  `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)` で `wScan` を埋める。
- `KEYEVENTF_SCANCODE` は付けない。過去の実機試行で、付けると WezTerm が
  IME をバイパスすることが分かっているためである。
- `VK_DBE_HIRAGANA` は MS-IME TSF の半角英数復元経路で scan 付き送信される。
  コードコメントには、物理かなキーの reinject / TSF warmup の scan 付き F2 は
  効き、`scan=0` の `send_ime_mode_key(VK_DBE_HIRAGANA)` では MS-IME TSF が
  モードキーとして処理しなかった実機結果が記録されている。ただし
  `docs/experiments.md` には standalone 測定で
  `MapVirtualKeyW(VK_DBE_HIRAGANA)` が 0 を返し、実際の hook ログでは
  `scan=0x70` が観測された矛盾も記録されている。したがって
  `make_scan_key_input(VK_DBE_HIRAGANA)` が常に `0x70` を作るとは仮定しない。

## Windows Terminal 側の観察

Windows Terminal の `Terminal::SendKeyEvent` は、受け取った `scanCode` が
0 の場合、`MapVirtualKeyW(vkey, MAPVK_VK_TO_VSC)` で scan code を補完する。
補完後も `sc == 0` の場合は key event として処理せず戻る。非ゼロの scan が
得られた場合だけ `_CharacterFromKeyEvent()` が
`ToUnicodeEx(vkey, scanCode, keyState, ...)` を呼び、文字化できる KeyDown を
文字入力側へ流す。

参照:

- <https://github.com/microsoft/terminal/blob/main/src/cascadia/TerminalCore/Terminal.cpp>
- <https://github.com/microsoft/Windows-driver-samples/blob/main/input/layout/fe_kbds/jpn/106/kbd106.c>

この公開ソースから確実に言えるのはここまでである。つまり
**Windows Terminal は scan 0 を自前補完し、補完できた場合は `ToUnicodeEx` の
文字化判定に使い、補完できなければ key event として扱わない**。これは
「scan 0 のまま IME/NLS VK を渡すと Terminal 側の補完結果に依存する」
という疑いを強めるが、**`VK_KANA` を `VK_DBE_HIRAGANA` に置換すれば
GJI/MS-IME が同じ意味で処理することまでは証明しない**。

日本語 106 キーボード配列では、`@` は `VK_OEM_3` 側の文字表に現れる。
一方、ひらがな/カタカナ系の NLS キーは scan `0x70` 側で扱われる。
`kbd106.c` の NLS function table では、`VK_DBE_HIRAGANA` の Base が
`KBDNLS_HIRAGANA`、Shift が `KBDNLS_KATAKANA`、Alt 系が `KBDNLS_ROMAN` と
定義されている。これは本 ADR の「無修飾 `VK_DBE_HIRAGANA` はひらがな方向、
Shift+かな は別意味なので置換しない」という制約を裏付ける。

このため、実験するなら送信形態は `VK_DBE_HIRAGANA + 実測済み nonzero scan`
が既存実装と整合する。ただし `MapVirtualKeyW(VK_DBE_HIRAGANA)` が実行時に
0 を返す環境では、この実験は仮説を検証できない。v3 以降の preflight は
Windows Terminal 側の `sc == 0` 早期 return とも整合する必須条件である。

## 問題の再定義

これは Windows Terminal の keybinding 設定で吸収する問題ではない。
Terminal の `actions` は `VK_KANA` / `VK_KANJI` / `VK_DBE_HIRAGANA` のような
IME/NLS VK を安定して no-op 化する層ではなく、`unbound` は「捨てる」ではなく
Terminal のショートカット処理を外して下流へ通す設定である。

また `[[keymap]]` で実験する問題でもない。ADR-114 / ADR-130 により、
`[[keymap]]` の `from` / `to` は IME 制御系 VK を禁止している。これは
`send_keymap_target()` が `INJECTED_MARKER` 付きで送信し、フックの
`is_self_injected` 早期 return により `ImeModel` の belief が更新されないまま
実 IME だけが変わる危険を避けるためである。

## 決定

### D1: `VK_KANJI` は既存の冪等 direct strategy を優先し、新しい scan 0 再送経路を作らない

`VK_KANJI` は非冪等トグルであり、可能な限り `GjiDirectStrategy` /
`MsImeDirectStrategy` の `VK_IME_ON` / `VK_IME_OFF` を使う。これは既存方針の
追認であり、`VK_KANJI` の新しい scan 0 再送経路を作らない。

既存方針との関係:

- `GjiDirectStrategy` / `MsImeDirectStrategy` は既に `VK_IME_ON` /
  `VK_IME_OFF` を使う。
- `KanjiToggleStrategy` は Standard/ImmCross 系の最終フォールバックであり、
  到達条件を増やしてはならない。
- `VK_KANJI` 由来の再送は `wScan=0` のまま増やさない。Windows Terminal では
  `scanCode==0` が `ToUnicodeEx` の文字化経路へ進むためである。

### D2: 物理 `VK_KANA` 置換は Windows Terminal 限定の hidden 実験にする

`VK_KANA` は `VK_IME_ON` と異なり、open 軸だけでなく「ひらがな入力へ入る」
意味を持つ。したがって `VK_IME_ON` だけに置換すると意味が落ちる。一方で
`VK_DBE_HIRAGANA` は open と「ひらがなに強制」を束ねる非中立キーであり、
BUG-50 以降、open 軸制御からは意図的に退けられてきた。

そのため v1 のように「TsfNative 全体で採用」とはしない。最初の実装は
hidden config または診断ビルドの実験として、以下の条件に限定する:

- 対象アプリは `WindowsTerminal.exe` のみ。WezTerm 等の他 TsfNative へ
  広げない。
- 対象イベントは `event.injected == false` の物理 `VK_KANA` のみ。
- 修飾なしに限定する。Shift/Ctrl/Alt/Win 押下中は置換しない。
- `VK_KANA` KeyDown を Suppress する場合、同じ打鍵処理内で replacement
  actuation を必ず発行する。Suppress だけで終わる経路を作らない。
- `VK_KANA` KeyUp は Suppress する。replacement は KeyDown 側で Down/Up
  ペアとして即時完結させる。

replacement actuation の形は以下に限定する:

```text
wVk   = VK_DBE_HIRAGANA (0xF2)
wScan = MapVirtualKeyW(VK_DBE_HIRAGANA, MAPVK_VK_TO_VSC)
        JIS 106 実機では 0x70
flags = 0 / KEYEVENTF_KEYUP
KEYEVENTF_SCANCODE は付けない
dwExtraInfo = IME_KANJI_MARKER または新規の物理VK_KANA置換専用マーカー
```

実装時には、awase.exe 本体の実行時キーボードレイアウト文脈で replacement の
`wScan` を必ず preflight 計測し、INFO ログに出す。`wScan == 0` または
JIS 106 で期待する `0x70` と矛盾する値なら、この実験はそのセッションで
発火させない。standalone プロセスでの `MapVirtualKeyW` 測定は信用しない。

`VK_KANA + scan 0x70` は採用しない。vkey と scan の意味がずれており、
Windows Terminal / IME / キーボードレイアウトのどの層がどちらを信じるかを
不要に曖昧にするためである。`VK_DBE_DBCSCHAR` も採用しない。これは
「全角」方向であり、「ひらがな入力」とは意味が異なる。

### D3: `KEYEVENTF_SCANCODE` は付けない

awase 既存の `make_scan_key_input()` と同じく、`wVk` と `wScan` は両方セットし、
`KEYEVENTF_SCANCODE` は付けない。`KEYEVENTF_SCANCODE` を付けると IME/TSF を
バイパスする実機ハザードが既に記録されているため、今回の修正で同じ経路を
再導入しない。

### D4: 実装は `[[keymap]]` ではなく IME 物理キー配送層に置く

実装候補は `runtime/key_pipeline.rs::process_key_event` 冒頭、
`platform.try_hold_key(event)` より前の専用 early path である。
`runtime/transport.rs::PhysicalKeyDisposition::plan` だけで実装してはならない。
`[[keymap]]` の禁止も緩めない。

必要な性質:

- `InputRelay` は従来どおり最優先で Allow する。ADR-119 の
  「解釈しない入力は消費しない」を壊さない。
- hook レベルの foreign-injected `VK_KANA` swallow / Alt+`VK_KANA` swallow
  は現状維持する。過去に IME モードキー全般の broad swallow は撤回済みであり、
  今回の実験で復活させない。
- early path は、既存の `kp_stage_shadow_ime_toggle` と同等の
  `PhysicalImeKey` / `TurnOn` intent 記録を、元の物理 `VK_KANA` に対して
  先に実行する。`try_hold_key` 後に置くと、`TsfGate::PendingWarmup` 等で
  元イベントが hold/consume され、replacement と intent の片方だけが
  失われる可能性があるためである。
- 置換送信は awase 自身の self-injected としてフック再処理されないようにする。
- 合成 `VK_DBE_HIRAGANA` は `ImeModel` / `IntentStore` / eisu recovery /
  `composition_native_f2_down()` の入力源として期待しない。self-injected は
  hook 早期 return で runtime に入らないためである。
- 物理 `VK_KANA` の intent 記録と replacement actuation は同じ early path 内で
  ログに残す。awase が suppress するなら、awase がその置換 actuation の
  所有権も持つ。

この early path は `try_hold_key` より前に置く。`TsfGate` が pending の場合も、
「同じ処理単位」は deferred replay ではなく元の hook delivery である。
replacement を defer/replay へ委譲しない。ON 系物理 IME キーは元キーが OS に
届くことに依存している経路があり、suppress だけだと belief ON × 実 IME OFF を
作りうる。

### D4b: Suppress は replacement の全件成功と per-press latch に結合する

物理 `VK_KANA` KeyDown を consume してよいのは、replacement batch
（必要な modifier release、`VK_DBE_HIRAGANA` Down/Up、必要な restore）が
`SendInput` に全件受理された場合だけである。

- preflight scan が 0/想定外なら、実験は発火せず既存処理へ流す。
- `SendInput` が 0 件なら、実験を abort し、元の `VK_KANA` は既存処理へ流す。
- `SendInput` が部分成功した場合は、同じ marker で
  `VK_DBE_HIRAGANA` KeyUp の best-effort repair を試み、実験をセッション内で
  disable する。元の `VK_KANA` は必ず既存処理へ流す。この戻しが保証できない
  挿入点では、この実験を実装してはならない。この分岐は実機検証で 1 件でも
  出たら実験失敗として撤回する。
- 全件成功した KeyDown のみ、`vk_kana_replacement_latch[VK_KANA] = true`
  を立てて consume する。
- 物理 `VK_KANA` KeyUp は、この latch が true のときだけ consume し、
  latch を clear する。latch が無い KeyUp は既存処理へ流す。
- latch は focus change、panic reset、session unlock、一定 timeout でも
  defensive clear する。target F2 は Down/Up 即時完結なので、clear 時に
  target KeyUp を追加注入しない。

### D5: F2 warmup 所有権と混同しない

`VK_DBE_HIRAGANA` は既に TSF cold-start / GJI warmup / MS-IME 復元の文脈で
使われている。今回の置換送信は、少なくともログ上は以下と区別できなければ
ならない:

- `send_eager_tsf_warmup()` 由来の `VK_IME_ON`
- `composition_native_f2_down()` が扱う物理 `VK_DBE_HIRAGANA`
- half-width alnum 復元由来の scan 付き `VK_DBE_HIRAGANA`
- 物理 `VK_KANA` 置換由来の scan 付き `VK_DBE_HIRAGANA`

既存の `IME_KANJI_MARKER` を使う場合でも、ログには origin を出す。
必要なら専用 marker を追加するが、`hook.rs::is_self_injected` の認識対象に
加えることを忘れてはならない。どちらの場合でも、合成 F2 は warmup ではなく
**physical VK_KANA replacement delivery** である。warmup state の更新を
合成 F2 の hook 再入に期待しない。

### D6: ADR-121 のスコープを広げず、実験結果が出るまで supersede しない

ADR-121 は no-op 時の冪等再送を `VK_DBE_HIRAGANA` の実機証拠に限定し、
`VK_KANA` / `VK_IME_ON` / `VK_JUNJA` へ広げなかった。本 ADR はその判断を
ただちに覆さない。今回の `VK_KANA` は「Windows Terminal で文字化する」
という別症状に対する実験であり、ADR-121 の reassert 対象拡張ではない。

実機で Windows Terminal + GJI / MS-IME の両方が通り、かつ modifier /
composition / kana-lock の退行が無いことが確認できた場合に限り、ADR-121 への
追補または本 ADR の採用化を検討する。

## 実装タスク案

1. hidden config として実験フラグを追加する。既定は off。sample config /
   GUI / migration には出さず、起動時に active なら INFO ログを出す。
2. `WindowsTerminal.exe` かつ物理・非注入・無修飾 `VK_KANA` を検出する
   pure gate を追加する。process 名だけでなく、既存の
   `AppImeProfile::TsfNative` / `CASCADIA_HOSTING_WINDOW_CLASS` と整合させる。
   `InputRelay` では必ず無効にする。
3. awase.exe 本体内で `MapVirtualKeyW(VK_DBE_HIRAGANA, MAPVK_VK_TO_VSC)` の
   preflight を行い、非ゼロであることを確認する。JIS で `0x70` 以外なら
   実験を abort してログを出す。
4. `process_key_event` 冒頭、`try_hold_key` より前に early path を追加する。
   この path で元 `VK_KANA` の `PhysicalImeKey/TurnOn` intent 記録と
   replacement send を一体で扱う。
5. `VK_KANA` KeyDown を Suppress する場合、同じ early path 内で
   `VK_DBE_HIRAGANA` の scan 付き Down/Up を `make_scan_key_input()` 相当で送る。
   `send_ime_mode_key()` は使わない。これは `wScan=0` 経路だからである。
6. `SendInput` 全件成功した KeyDown だけ latch し、対応する
   `VK_KANA` KeyUp だけ Suppress して latch を clear する。
7. Shift/Ctrl/Alt/Win 押下中は実験を発火させない。特に Shift+かな は
   カタカナ方向の意味を持つため、ひらがな置換してはならない。
8. `VK_KANA` 置換の INFO ログを追加する。少なくとも original vk/scan、
   replacement vk/scan、profile、active_ime_kind、modifier block の有無、
   suppress と replacement send の成否、latch set/clear を出す。
9. unit test では `PhysicalKeyDisposition` だけでなく、replacement actuation が
   suppress と不可分であること、KeyUp suppress が latch 条件付きであること、
   InputRelay / injected / modifier-held では発火しないことを固定する。
   Windows Terminal 実機では
   `VK_KANA`、`VK_KANJI`、`VK_DBE_HIRAGANA`、`VK_IME_ON` の4パターンを比較する。

## 未解決の疑問

- `VK_KANA` の物理 KeyDown を受けた時点で、Windows/IME がフックより前の層で
  既に何らかの入力方式切替を行っている可能性を実機ログで確認する必要がある。
- MS-IME と GJI で `VK_DBE_HIRAGANA + scan 0x70` の意味が完全に同じかは
  実機で確認する必要がある。公開ソースだけでは証明できない。
- 物理 `VK_KANA` を suppress する前に、Windows/IME がフックより前の層で
  既にかなロックや入力方式切替を済ませている場合、この実験は症状を隠すだけに
  なる可能性がある。
- Windows Terminal 以外の TsfNative（WezTerm 等）へ広げるかは別判断にする。
  WezTerm には `KEYEVENTF_SCANCODE` ハザードの実績があり、同じ修正を広げる前に
  soak が必要である。
- `RawKeyEvent::reinject()` が常に `wScan=0` である点は別問題として残る。
  今回は IME/NLS VK だけを対象にし、一般 reinject の scan 保持化は行わない。

## 採用しない案

### A: Windows Terminal の keybinding で潰す

`VK_KANA` / `VK_KANJI` / `VK_DBE_HIRAGANA` を安定して no-op 化する公式の
keybinding 名が無く、`unbound` は捨てる設定ではないため採用しない。

### B: `VK_KANA` を `VK_IME_ON` に置換する

open 軸だけになり、「ひらがな入力へ入る」という `VK_KANA` の意味が落ちるため
採用しない。

### C: `VK_KANA` を `VK_DBE_DBCSCHAR` に置換する

`VK_DBE_DBCSCHAR` は全角モード選択であり、ひらがな入力選択ではないため採用しない。

### D: `[[keymap]]` の IME VK 禁止を一時的に緩めて実験する

ADR-114 / ADR-130 の禁止理由を再導入するだけなので採用しない。実験は専用の
IME 物理キー配送層で行う。

## 検証計画

Windows 実機で以下を採取する:

1. Windows Terminal + GJI、JIS 配列:
   - 物理 `VK_KANA`
   - 物理 `VK_KANJI`
   - 物理 `VK_DBE_HIRAGANA`
   - awase 置換 `VK_DBE_HIRAGANA + scan`
2. Windows Terminal + MS-IME、JIS 配列で同じ4ケース。
3. 比較項目:
   - 余分な「@」が出ないこと。
   - IME が ON になること。
   - 可能なら romaji 入力が維持されること。
   - カタカナモードからの復帰不能や charset 強制の副作用がないこと。
   - JISかな直接入力へ落ちないこと。
   - candidate/composition が壊れないこと。
   - `journal` の original vk/scan と replacement vk/scan が区別できること。
   - 元の物理 `VK_KANA` が1回だけ観測され、元キーが reinject されず、
     replacement が1回だけ送信されること。
   - replacement の `wScan` が awase.exe 本体内で非ゼロとして計測されること。
   - foreign-injected IME キーがユーザー意図に昇格しないこと。
   - KeyUp が stuck しないこと。

実機確認が取れるまでは、Windows Terminal 以外には展開しない。実験で
「@」消失と引き換えに romaji 喪失、JISかな化、カタカナ固定、composition 破壊、
stuck KeyUp のいずれかが再現した場合は即撤回する。

## Sol round1 の指摘と反映

### Sol-A blocking への対応

- A1: transport-only suppress では実 IME が ON にならない。D4 に
  suppress と replacement actuation の不可分性を追加。
- A2: ADR-121 の対象拡張と衝突する。D6 を追加し、ADR-121 は supersede しない。
- A3: `send_ime_mode_key` は `wScan=0`。実装タスクで使用禁止を明記。
- A4/A5: 合成 F2 は self-injected で runtime に入らない。D4/D5 に
  intent 更新源は元 `VK_KANA` のみと明記。
- A6: Shift+かな の意味を壊す。D2/D4 で無修飾限定に縮小。
- A7: `[[keymap]]` 禁止維持。問題の再定義と採用しない案 D を維持。

### Sol-B blocking への対応

- B1: Windows Terminal OSS だけでは意味論を証明しない。Windows Terminal 側の
  観察を「scan 0 補完 + ToUnicodeEx の事実」に縮小。
- B2: BUG-08/14 の broad swallow を復活させない。D4 に hook ガード現状維持を追加。
- B3: belief/intent 所有権が必要。D4 に元物理イベントだけが intent 源であることを追加。
- B4: F2 は中立 ON キーではない。D2 を hidden 実験に格下げ。
- B5: disposition table が必要。実装タスクと検証計画へ InputRelay、
  injected/non-injected、modifier 条件を追加。
- B6: keymap で検証しない。問題の再定義と採用しない案 D を維持。

## Sol round2 の指摘と反映

### Sol-A blocking への対応

- A1: Suppress が replacement delivery 成功に結合していない。D4b を追加し、
  全件成功時だけ consume/latch することにした。
- A2: KeyUp suppress に per-press latch が無い。D4b で latch 条件付きにした。
- A3: `try_hold_key` より後だと TSF gate に吸われうる。D4 を `process_key_event`
  冒頭・`try_hold_key` 前の early path に変更した。

### Sol-B blocking への対応

- B1: `MapVirtualKeyW(VK_DBE_HIRAGANA)` の scan 前提が不整合。背景と D2 に
  standalone 測定の矛盾を追記し、runtime preflight と scan 0 abort を必須化した。
- B2: Suppress と成功 semantics が弱い。D4b に全件成功・0件・部分成功の扱いを追加。
- B3: KeyUp latch が無い。D4b に latch set/clear と defensive clear を追加。

## Sol round3 の指摘と反映

### Sol-A blocking への対応

- A1: v3 の部分成功分岐に「実装上不可能なら consume」という逃げが残っていた。
  D4b から削除し、部分/ゼロ成功時に元 `VK_KANA` を既存処理へ戻せない挿入点では
  実装禁止とした。
