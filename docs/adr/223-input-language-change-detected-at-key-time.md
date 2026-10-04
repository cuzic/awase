---
id: ADR-223
title: |-
  入力言語の切替を、キー入力の時点で観測して Engine を正しい状態にする(案C)。表示の即時更新は切替キーの解放後に 1 回だけ読んで足す(案E2)
summary: |-
  BUG-183(issue #445): 入力言語を切り替えても、awase はフォーカスが動くまで非活性にならず、NICOLA が非日本語の打鍵を変換する。
  通知の購読(HSHELL_LANGUAGE・TSF シンク)は使えない(CI 実測)。決定案: (D0)is_japanese_ime の書き込み元を整理し、言語が「不明」のときは言語も IME の開閉も書き込まない、
  (D1)engine スレッドがキーリングから取り出した直後に、現在のフォーカス窓から都度引いたスレッドの HKL を読み、物理・外部注入を問わず(awase 自身の注入以外の)文字キーの KeyDown で食い違いを検知して更新する、
  (D2)段階 0 は記録のみ、(D3)表示を早めるには、切替キーの解放後に 1 回だけ読む(案E2)。ポーリングは足さない。
status: |-
  起草 r3(2026-10-04)。Opus r1(Blocker 2・Must 6・Should 5)と r2(Blocker 1・Must 5・Should 4)の指摘を反映済み。Opus r3 待ち。実装なし。
related_adr:
  - "ADR-093"
  - "ADR-129"
  - "ADR-187"
  - "ADR-191"
---

# ADR-223: 入力言語の切替をキー入力の時点で観測する

## 背景(事実)

BUG-183(report `01M4047347…`、issue #445): MS-IME で入力言語のホットキー(左Alt+Shift+3)でロシア語に切り替えると、awase のアイコンが
オレンジのまま(Engine が活性のまま)になる。実害は、非日本語の言語に切り替えた後も、NICOLA がその言語の打鍵を日本語の同時打鍵として処理すること。

### 1. 現在の経路と書き込み元(Opus r1 B1、コードで確認済み)
`is_japanese_ime`(`state/belief.rs::ImeBelief`)の書き込み元は次のとおりで、**読んでいるスレッドが揃っていない**。

| 書き込み元 | 読む HKL | 向き |
|---|---|---|
| ① `platform_state.rs:1180` `apply_ime_update`(`read_ime_state_full` 経由、IME の読み取り) | `GetGUIThreadInfo` で得たフォーカススレッド。**`thread_id=0` になる 3 経路(`hwndFocus`/`hwndActive` が共に NULL=`win32.rs:401`、`GetGUIThreadInfo` の失敗=`:406-407`、タイムアウト=`:417-423`)で、呼び出したワーカースレッド自身の HKL を読む**。さらに `classify_poll_outcome` は `is_japanese_ime == Some(false)` で `ObserverReported(false)`(IME OFF を観測した扱い)を作る(`ime_observer.rs:61-69`) | 双方向 |
| ② `key_pipeline.rs:2698` `apply_focus_probe`(直接 `set_is_japanese_ime`) | `read_ime_state_fast` → `keyboard_layout_info()` = **`GetKeyboardLayout(0)`**(`ime.rs:813-821`)をワーカースレッドで呼ぶため、**awase 自身のスレッドの HKL**。さらに非日本語と読むと **`ime_on: Some(false)` を返す**(`ime.rs:839-843`)ので、開閉の観測まで OFF になる | 双方向(grace 中の false は抑止) |
| ③ ADR-093 の昇格 `key_pipeline.rs:933` | (キーの種類が証拠) | true のみ |
| ④ `apply_panic_reset` `platform_state.rs:1140` | — | true のみ |

`vk.rs:468` のコメントも「`is_japanese_ime()` は awase のワーカースレッドの HKL 由来で偽になりうる」と書いている。IME の定期読み取り(500ms)は既にあるが、
**明示意図(`explicit_intent`)があると予約が止まり**(ADR-187、`runtime/mod.rs` の `reschedule_ime_refresh`)、TsfNative の窓でも常に止まる。

### 2. CI 実測(`ci/bug183-lang-switch`、windows-latest、MS-IME + ru-RU、`lang_switch_probe`)
- **言語が切り替わっても、フォーカスが動かない限り Engine は非活性にならない**(run 37171454298、16/16 で 14 秒待っても `Engine deactivated` なし)。切替の 1.5 秒後にフォーカスを移すと、移動の 74〜86ms 後に非活性(8/8)。
  **条件**: どの試行も直前に `VK_IME_ON`(明示意図)を送っている。**明示意図が無い場合の対照は未測定**(Opus r1 F2)。
- **切替の完了は、きっかけのキーから 10〜20ms**(プローブの打鍵間隔 30ms のため、注入開始からは 2〜115ms と見える。Opus r1 F1)。人の次の打鍵には常に間に合う。
- **通知の購読は使えない**: `HSHELL_LANGUAGE`(wparam=8)は前面・背景とも 0 件(run 37172398639・37172857037。他のシェルイベントは届く)。
  TSF の `ITfActiveLanguageProfileNotifySink`・`ITfLanguageProfileNotifySink` は、前面のプローブでは 24/24 で届いたが、前面にならない背景プロセスでは 0 件。
  **原因は、TSF の言語プロファイルがスレッド単位の状態で、通知は購読したスレッド自身の変化しか知らせないため**(Opus r1 F4)。同じ理由で `ITfInputProcessorProfileActivationSink`・`ITfThreadMgrEventSink`・`WM_INPUTLANGCHANGE` を受ける窓も使えない。
- **言語切替ホットキーの読み方**(run 37173478484・37173691419):
  - 全体の切替キー `HKCU\Keyboard Layout\Toggle`(`Hotkey`/`Language Hotkey`): **`1`=Alt+Shift だけ、`2`=Ctrl+Shift だけ、`3`=なし**(`SPI_SETLANGTOGGLE` で即反映。値ごとに 3 試行ずつ、切替の有無が完全に分かれた。注入したのは左 Alt/左 Shift/左 Ctrl)。
    **`4`(`` ` `` 単独、主にタイ語環境)は未測定**。`Layout Hotkey` は同じ言語内のレイアウト切替で別物。
  - 言語ごとの直接切替キー: `HKCU\Control Panel\Input Method\Hot Keys\0000010x`(`Key Modifiers`、`Virtual Key`、`Target IME`=HKL)。実行時の値は `ImmGetHotKey` で読み戻せ、レジストリと同じ値だった
    (例 `0x104`: 修飾 `0xC006`、VK `0x30`、HKL `0xE0010411` = Ctrl+Shift+0 → 日本語。`ImmSetHotKey(0x100, 0x4005, 0x33, ru)` で設定した値も同じ値で読み戻せた)。
    `ImmGetHotKey` は、Windows CE のドキュメントでは確認できる(imm.h、「IME のコントロールパネルが呼ぶ」)が、デスクトップ Windows の現行ドキュメントでは確認できていない。動作は CI で確認した。
    修飾の下位バイトは 1=Alt・2=Ctrl・4=Shift。`0xC000` は左右の指定(プローブのコメントは `MOD_LEFT=0x4000` としたが、imm.h では `MOD_LEFT=0x8000`・`MOD_RIGHT=0x4000` の可能性がある。未確認。注入キー(scan=0)では左右が効かない可能性もある)。
    `Target IME` の HKL(`0xE0010411`)は旧 IMM 形式で、`GetKeyboardLayout` が返す TSF の MS-IME の値(`0x04110411`)と形式が違うため、照合するなら言語 ID(下位 16 ビット)で比べる。
    `Hot Keys\00000010〜12`・`70〜72`・`200〜203` は IME 固有キー。Win+Space は固定。
- 実機で Alt+Shift・Win+Space が即座に灰色になる理由は**未特定**(CI ではこの 2 つも非活性にならない。Win+Space では未知のシェルイベント `wparam=56` が出る)。「左Shift 単独を 2 回で灰色」も、実機 journal でどの経路が非活性にしたかを特定してから範囲を確定する(Opus r1 F3)。

## 検討した案

| 案 | 評価 |
|---|---|
| A. 通知の購読(HSHELL_LANGUAGE、TSF のシンク、その他の TSF シンク、`WM_INPUTLANGCHANGE` の窓) | **不可**(事実 2)。スレッド単位で、背景には届かない |
| B. 定期ポーリング(常時、またはホットキー後 5 秒間 100ms) | 常時はやらない(ユーザー判断)。切替は 10〜20ms で完了するので、5 秒間のポーリングは不要 |
| **C. 打鍵の時点で、保存済みのフォーカススレッドの HKL を読む(採用)** | 問題が起きる打鍵(日本語配列として処理される前)に確実に検知する。非ブロッキングで、ポーリング不要。マウス操作(言語バー・トレイ)の切替も、次の打鍵で拾える |
| E1. 切替キーを `Toggle` と `ImmGetHotKey` で列挙し、一致したキーの押下の約 50ms 後に 1 回だけ読む | 検出が精密。ただし設定変更時の再列挙、修飾キー単独の「押下の完了」の判定、HKL の形式の違い(言語 ID で照合)が要る。E2 の過検知のコストは数 µs で、実害は読み取りの誤り(D0・D1)にしか無いため、**採らない**(Opus r2 N-M3) |
| E2. 次の組み合わせの解放後、約 50ms 後に 1 回だけ読む(Opus r1 の提案、r2 で規則を修正): (a) 修飾キーを 2 つ以上含む組み合わせ、(b) Win を含み Space で終わる組み合わせ(Win+Space)。`` ` `` 単独(Toggle=4)は、頻出するため段階 2 では扱わない | 設定を読まずに、任意に割り当てた言語ホットキー・Alt+Shift・Ctrl+Shift・Win+Space を拾う。読み取りは非ブロッキングの数 µs、タイマーは 1 本をリセットする方式で多重予約しない。過検知(Ctrl+Shift+矢印、Win+Shift+S など)のコストは 1 回の読み取りだけで、言語が変わっていなければ何も起きない |
| F. `SetWinEventHook`(`EVENT_OBJECT_IME_*`、タスクバーの入力インジケーターの名前変化) | 背景でも受けられ、マウス切替も拾える唯一の購読候補。OS・タスクバー設定・ローカライズ依存。E2 で足りないと分かってから測る(ADR に残すが、いまは採らない) |

## 決定案

### D0: 前提の修正(Opus r1 B1)
- HKL を読むスレッドを、必ず明示したスレッドに限る。`tid==0` または `hkl==0` のときは「不明(`None`)」とし、**書き込まない**。②(`GetKeyboardLayout(0)` を使う `read_ime_state_fast`)と、①の `thread_id==0` の 3 経路を直す。
- **言語が `None` のときは、IME の開閉も書かない**(Opus r2 N-M1)。②の「非日本語なら `ime_on: Some(false)`」という短絡(`ime.rs:839-843`)と、①の `known_not_japanese` 分岐(`ime_observer.rs:61-69`、`ObserverReported(false)`)は、言語が「不明」のときは起こさない。言語を誤読すると、`is_japanese_ime` だけでなく IME の開閉の belief まで汚れるため。
- `is_japanese_ime` の言語由来の書き込みを、`ImeStateHub::observe_layout_language(Option<bool>, source)` の 1 つに集約する(①②と D1 が通る)。③(ADR-093)と ④(panic reset)は直接 setter のまま。
  純関数 `classify_layout_language(hkl: u32) -> Option<bool>`(`hkl==0 → None`)を `state/` に置く。`architecture_guard` に `set_is_japanese_ime` の呼び出し元の件数ガードを足す。
- `ImeEvent` は新設しない(`is_japanese_ime` は `ImeModel` の reduce 対象ではなく、規則も求めていない。Opus r1 M3)。ADR-093 に「grace 中の誤答の一部は ② の読み先の誤りで説明できる」と追記して関連づける。

### D1: 打鍵の時点で観測する(案C)
- **読む場所**: キーボードフックは engine とは**別のスレッド**で動き、キーを `HOOK_KEYS`(SPSC リング、`hook.rs:1836`)へ積んで返す。engine スレッドが取り出す(CLAUDE.md の「単一スレッド」はフックについては実態と違う)。
  フック内で読むには、フォーカス窓の情報をスレッドをまたいで渡す共有状態(`HOOK_STATE` への `AtomicU32` など)が要り、書き漏れで古い tid が残る(Opus r2 N-B1 の案 a)。**採らない**。
  読むのは **engine スレッドがリングから取り出した直後(`INPUT_DEFER` へ入れる前)**。drain の再生はこの取り込み口を通らないので、ADR-129 の原則(再生時に「今」を読まない)も守れる。フックから取り込みまでの遅れは µs〜数 ms で、言語切替(きっかけのキーから 10〜20ms)より十分短い。
  読み取りの結果は `RawKeyEvent` の欄に書き、journal の key input レコードにも載せる(段階 0 の記録と再生の検証のため)。
- **どのスレッドを読むか**: tid は保存しない。**現在のフォーカス窓の hwnd から毎回 `GetWindowThreadProcessId` で引く**(非ブロッキング。窓が破棄されていれば `tid=0` → `None`、スレッド終了なら `GetKeyboardLayout` が 0 → `None`、tid の再利用の問題も起きない。Opus r2 N-M2)。
  `GetGUIThreadInfo` は呼ばない(ハングしうる)。前面スレッドへのフォールバックもしない(UWP の `ApplicationFrameHost` の古い言語で誤って下げる危険)。
  **フォーカス変更の直後は「不明」**: フォーカス変更のイベント(WinEvent の受信時点)で hwnd を「未解決」にし、解決(既存の非同期のフォーカス処理)が終わるまで D1 は読まない(`None`)。その間の belief は前の窓の値のまま(今と同じ。既存のフォーカス時の読み取り ① が約 80ms で直す)。
  読まないと、「アプリごとに入力方式を設定する」設定で、前の窓が ru・新しい窓が ja のとき、日本語の窓での最初の数打鍵を誤って非活性にする。
- **反映の場所**: `process_key_event` の先頭、`build_input_context`(`key_pipeline.rs:93-101`)**より前**。同じ打鍵の ctx に間に合い、その打鍵から通過になる。ここで反映するのは、取り込み口で `RawKeyEvent` に載せた値。
- **対象**: awase 自身の注入(`is_self_injected`)**以外のすべての KeyDown**。物理に限らない(PowerToys・AutoHotkey・リモートデスクトップ経由のキーも対象。読む証拠は OS の HKL で、きっかけのキーの出自ではない。Opus r1 B2)。
  Ctrl/Alt/Win を押している間のキーは読まない(PassThrough でエンジンは変換しない。シェルのフライアウト中の前面窓の言語で往復する機会も減らす。Opus r1 S1)。Shift のみは読む。
- **確定は 1 回で行う**(連続 N 回は使わない。N≥2 は切替後の最初の N−1 打鍵が NICOLA 変換される=症状そのもの。Opus r1 M6)。誤検知への対策は読み取りの妥当性(D0)で行う。
- **言語の遷移を検知したら**(**ja→非日本語、非日本語→ja の両方向**)、IME 状態の読み直しを**1 回だけ**予約する(`schedule_ime_refresh(20)`程度。ポーリングではない)。ru 側では読み直しが正しく `ObserverReported(false)` を記録する。
  日本語へ戻したときに `effective_open()` が古い意図(または ru 時の false の観測)で固定されないように、現在の対象の**明示意図を捨てるか**は未確定(下記)。段階 0 の「明示意図なし」対照の結果で決める。
- 既存の `kp_stage_idle_conv_check` に相乗りしない理由: あれは conv を**非同期**で読む仕組みで、最初の打鍵に間に合わない。D1 は**同期**でフックの中で済むことが本質(Opus r1 S2)。

### D2: 段階 0 は記録のみ(合格基準の修正、Opus r1 M5)
段階 0 では更新せず `[lang-check]` を記録する: `fg_tid`・`focus_tid(都度引いた値)`・`hkl(fg)`・`hkl(focus)`・`fg_class`・`belief`・どの書き込み元が何を書いたか。
**量を抑える**(Opus r2 S-1): 値が変わったとき、または食い違ったときだけ 1 行出し、件数は統計として 1 行にまとめる(KeyDown ごとには出さない)。
- 比較相手は belief ではなく、同じ時点のフォーカススレッドの HKL(belief は ② で汚れうる)。「日本語のまま食い違い 0 件」に加え、**ru に切り替えた後の最初の打鍵で false を観測した件数**(陽性)も合格条件にする。
- 追加で測る(`lang_switch_probe`、各数分の CI): ① **明示意図が無い**対照(`set_ja` から `VK_IME_ON` を抜く)、② **マーカーなしの外部注入**(B2 の検証)、③ 前面窓を `ApplicationFrameWindow`(電卓など)と `ConsoleWindowClass`(`conhost.exe` 直起動)にして ja↔ru を往復し、フォーカススレッドと前面スレッドの HKL を並べる、④ 「アプリごとに入力方式を設定する」設定で、ja の窓と ru の窓を交互にフォーカスする試行(CI で切り替えられなければ「アプリごとの設定では未検証」と記録する)。
D2 ①の結果しだいで、事実 2 の第 1 項(「フォーカスが動かない限り非活性にならない」)の書き方を条件つきに直す。実機の Alt+Shift・Win+Space が即座に灰色になる経路の特定は、D1 の実装を止める条件ではない(D1 はそれに依存しない。段階 0 の着手を遅らせない)。

### D3: 表示の即時更新(案E2、段階 2、任意)
D1 だけでは、切替から次の打鍵までアイコンが古い(害は無い)。早めたい場合に、E2(上の規則の組み合わせの解放後、約 50ms で 1 回だけ読む)を足す。設定を読まず、任意の言語ホットキーを拾う。タイマーは 1 本をリセットする方式にする。
E1(`Toggle` + `ImmGetHotKey`)と案F(`SetWinEventHook`)は採らない。E2 の過検知のコストは 1 回の読み取りだけで、実害が出るのは読み取りの誤りだけであり、それは D0・D1 で防ぐため。tuning 定数(50ms)は段階 2 まで足さない。マウスでの切替は、D1 が次の打鍵で拾う。

### D4: 範囲(Opus r2 N-M4 で見積もりを補正)
- 取り込み口での読み取り 1 か所と、`RawKeyEvent` の新しい欄、journal の key input レコードへの記載。
- focus hwnd → tid の解決と、フォーカス変更時の「未解決」化(`GetWindowThreadProcessId` の呼び出しと、状態 1 つ)。
- 純関数 `classify_layout_language` と `observe_layout_language`(`ImeEvent` は新設しない)。
- ①②の読み先の修正(2 か所)と、②の `ime_on: Some(false)` 短絡・①の `known_not_japanese` 分岐の `None` 化。
- 両方向の遷移での refresh 予約(1 行)。
- 段階 2 のみ: E2 のタイマー 1 本。
IME への書き込み(actuation)は増やさない。規模は数十行とテスト。複雑性予算の観点でも、増えるのは観測点 1 つ・欄 1 つ・状態 1 つ。

## 未確定(Opus r2 で詰める)
- 言語遷移時の明示意図の扱い(捨てるか)。M4: 日本語へ戻したとき、IME の開閉状態が古い意図と無関係に決まる。段階 0 の「明示意図なし」対照と合わせて決める。
- フォーカス窓の hwnd から引いたスレッドが、UWP の `CoreWindow` など言語が同期されるスレッドかどうか(D2 ③で測定)。従来のコンソールで `GetKeyboardLayout` が実際の言語を返さない件。
- 「アプリごとに入力方式を設定する」設定での振る舞い(D2 ④。CI で切り替えられなければ未検証のまま)。`Toggle=4`(`` ` ``)は未測定で、段階 2 でも扱わない。
- 実機の Alt+Shift・Win+Space が即座に灰色になる経路(journal で特定)。
