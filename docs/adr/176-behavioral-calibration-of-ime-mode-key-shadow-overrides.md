---
id: ADR-176
title: |-
  awase-settingsの明示的な較正UIでモードキーの実効果を測定し、
  未登録時に静的分類を補完する
status: |-
  **2026-09-16: v6をopus round5レビューで即日撤回、v7として観測方式を
  大幅に単純化（round1〜4のBlocker計17件はv5で対応方針確定済み、
  実装着手前）。**

  v6（「較正専用のネイティブWin32子ウィンドウを新設する」という決定3）は
  opus round5レビューで**Blocker 5件**を指摘され、当日中に撤回した。
  最重要指摘: v6の中核前提「eguiメインウィンドウでは`WM_IME_CONTROL`も
  機能しない」は**[ADR-125](125-egui-winit-dynamic-ime-association-focus-model-gap.md)
  自身の実機検証ログ2と正面から矛盾していた**——ADR-125が否定したのは
  手法A（`ImmGetContext`直読み、HIMC=0）だけで、手法B
  （`ImmGetDefaultIMEWnd`+`WM_IME_CONTROL`）は`awase-settings.exe`の
  eguiメインウィンドウ上で実測動作が確認済みだった。加えて
  `ImmGetDefaultIMEWnd`はHWND単位ではなくスレッド単位のため
  「別HWNDを作る」こと自体が無意味、`IACE_CHILDREN`は子ウィンドウを
  巻き込むため「子ウィンドウ」案は自己破壊的、
  `ime_observation_spike.rs`はwinitを含まない環境でしか検証しておらず
  eframe内での可否について反証能力がゼロ、TSF「実測で不採用確定」は
  `GetGlobalCompartment()`というスコープの取り違えの可能性が高い、
  の計5件。指摘全文は
  `/tmp/.../opus-review-adr176-v6-round5.md`（セッション内スクラッチ
  パス、以後のセッションでは再現不可——要点は本status節に転記済み）。

  **決着実験（2026-09-16実施）**: 指摘を受け、`awase-settings.exe`の
  実際のバグ報告画面（eframe/egui、`--bug-report`）にフォーカスした
  状態で、既存の別プロセス観測スパイク
  （`crates/awase-windows/examples/spike_egui_ime_control_probe.rs`、
  ADR-125で作成済みのもの）を使い、手法Bで約28秒間（06:52:40〜
  06:53:08、GJIのIME ON/OFFを説明欄フォーカス時・他ウィジェット
  フォーカス時の両方で反復切替）継続観測した。**結果: 手法Bはこの間
  実際のIME ON/OFF切替を最後まで正しく追跡し続けた**
  （`elapsed_ms`はほぼ全て15ms未満、タイムアウト無し、`None`は1回のみの
  一時的なブレ）。これによりADR-125の実機ログ2が再現・補強され、
  **較正専用ネイティブウィンドウが不要であることが確定した**。

  **v7の決定（decision 3を全面差し替え）**: 観測は較正専用ウィンドウを
  新設せず、**awase.exe本体が決定2で確立した同一のIPC経路上で
  観測も兼ねる**——awase.exe本体は既に`imm.rs::probe_ime_control`
  （手法Bと同一実装、`awase-windows`クレート内の唯一のチョークポイント）
  を持っており、較正モード中にawase-settings.exeのHWND（IPC開始
  メッセージで受け取る）に対してこれをそのまま使い、観測結果を同じ
  `WM_APP+N`応答でawase-settingsへ返す。これにより:
  - awase-settings側に新しいWin32ウィンドウ・新しいwindows-rs
    feature・新しいSendMessageTimeoutW呼び出し点が一切増えない
    （`architecture_guard`/`actuation_call_guard`の対象範囲外に
    複雑性が漏れる懸念（round5 M6）が構造的に消える）。
  - windows-rsのバージョン不一致（`awase-windows`は0.62、
    `awase-settings`は0.58、round5 M5）も問題にならない——Win32型は
    awase-windowsの外に一切出ない。
  - TSFは引き続き非スコープ（「@」機序という独立した却下理由、
    ADR-153/BUG-113。round5 B5の指摘どおり「実測で機能しないことを
    確認済み」という記述は誤りだったため、正しい理由に訂正する）。

  詳細は下記「決定」節3・実装タスクリストT7〜T9参照。

  以下はv6起草時（撤回済み）の記録。手法A/B/Cの実機比較データ自体は
  正しく、v7でも引き続き根拠として使うが、そこから導いた「較正専用
  ネイティブウィンドウが要る」という結論は誤りだった（上記参照）。

  round4完了後、v5のタスクリストレビュー（`176-implementation-
  tasks.md`）で7件のBlockerが新たに見つかり、うち最重要のもの
  （T9のImmGetOpenStatusポーリング方式が
  [ADR-125](125-egui-winit-dynamic-ime-association-focus-model-gap.md)
  で既に反証済み——`awase-settings.exe`はeguiバックエンド`winit`の
  `set_ime_allowed(false)`が`ImmAssociateContextEx(hwnd,0,
  IACE_CHILDREN)`を呼びIMEコンテキストをデタッチするため
  `ImmGetContext`が常にHIMC=0を返す）が、紙の設計イテレーションでは
  解決できない実装可否の問題だったため、実機ミニアプリ
  （`crates/awase-windows/examples/ime_observation_spike.rs`）を
  作り、IME状態観測3手法（A: `ImmGetContext`+`ImmGetOpenStatus`直接、
  B: `ImmGetDefaultIMEWnd`+`WM_IME_CONTROL`——awase本体の`imm.rs::
  probe_ime_control`と同型、C: TSF `ITfThreadMgr`
  `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`）を同時に検証した。

  **実機結果（2026-09-16、実際のGJI IME ON/OFF切替を反復）**:
  手法A・Bは完全に一致してIME状態変化を追跡した（false→true→false
  の遷移を全て正しく検出、`disable_apps`でawase自身をこのプロセスへの
  介入から完全にバイパスした状態で確認）。**手法Cは観測開始から
  終了まで一度もtrueにならず、実際のIME ON/OFF切替を全く反映
  しなかった**（`ITfCompartment::GetValue()`は成功しVARIANTは
  `VT_I4`値0を返し続けた——エラーではなく「常に閉」という値がTSFの
  グローバルコンパートメントから返る。GJIはこの非TSFネイティブな
  ウィンドウに対してTSF経由でIME状態を公開していないと解釈できる）。
  このスパイクは本物のWin32 `EDIT`コントロールを持つ生ウィンドウ
  （eguiを介さない）であり、ADR-125が示した「eguiはIMEコンテキストを
  デタッチする」問題の影響を受けない。

  **v6が導いた結論（撤回済み、経緯記録として残す）**:
  1. 採用手法は**B**（`ImmGetDefaultIMEWnd`+`WM_IME_CONTROL`）に確定
     ——ここはv7でも維持。
  2. TSFは実測で不採用——理由の説明（「機能しないことを確認済み」）は
     round5 B5で誤りと指摘され、v7で訂正した（GetGlobalCompartment()の
     スコープ取り違えの可能性）。
  3. ~~較正専用の本物のネイティブWin32子ウィンドウで観測する~~
     ——**撤回**。ADR-125実機ログ2と決着実験により、
     eguiメインウィンドウ上で手法Bがそのまま機能することが確定した
     ため不要（v7決定3参照）。

  round1〜4の経緯（計17件のBlocker）は本ファイル過去版・関連レビュー
  に記録済み。以下は要約:

  round1〜3の経緯（計13件のBlocker、「バックグラウンド受動学習」から
  「awase-settingsでの明示的な較正UI」への転換）は本ファイル過去版・
  関連レビューに記録済み。**round4（v4）**は方向性は正しいと評価
  されつつBlocker4件が指摘された:

  - **B1（最重要）**: 既定の親指キー（`left_thumb_key="無変換"`/
    `right_thumb_key="変換"`）構成では、較正中にawase自身が
    `shadow_action`→Engine活性化→`ActivationSync`経由で実際に
    `VK_IME_ON`を送信してしまい、`ImmGetOpenStatus`の変化がGJI自身の
    反応かawaseの自作自演か区別できない。config1.dbの分類が
    **間違っている**場合ほどこの汚染で誤分類を追認してしまい、当初の
    動機（誤分類の自己修復）が達成できない。
  - **B2**: Hiragana/Katakanaは`AppImeProfile::Standard`
    （`can_use_imm32_cross_process()==true`）では`transport.rs::plan`
    が無条件Suppressするため、awase-settings上では物理キーがGJIに
    届かず構造的に較正不能。
  - **B3**: 学習結果を`shadow_action`供給層に直接合流させる設計は、
    非親指キー構成で`route_thumb_key_action`の排他振り分けと衝突し、
    ADR-141が既に棄却した「二重登録」（actuation-autoとshadow-toggle
    が同時armedになる）と同型になる。
  - **B4**: 「較正結果 > `keys.ime_detect`」という宣言した優先順位が、
    実際の`intent_kind`解決（`sync_direction`最優先）と逆転している。

  **v5の対応方針（round4レビュアー推奨、必須4点＋望ましい4点）**:

  1. **B1/B2対応**: 較正中は対象プロセス（`awase-settings.exe`）に
     対して`disable_apps`機構（`hook.rs`の`focus_app_disabled`、
     既存の完全バイパス、既定は`mstsc.exe`のみ対象）を適用し、
     awase自身のフック介入・actuationを較正対象から完全に排除する。
  2. **B3対応**: 統合点を`shadow_action`供給層への直接合流ではなく、
     `gate_thumb_key_ime_actions`の出力（`wiring.henkan`/
     `wiring.muhenkan`、型は`ImeToggleKind`）の差し替えに変更する。
     GJI側（`gji_charset_autodetect.rs:768-776`）・MS-IME側
     （`message_handlers.rs:964-966`）の**2箇所**に配線する
     （ADR-119の教訓：片方だけでは不足）。
  3. **B4対応**: 優先順位の宣言から`keys.ime_detect`との比較を削除し、
     BUG-140と同じ「優先順位ではなく構造的除外」に揃える——対象VKが
     `keys.ime_detect`に登録されている場合は較正UIが警告し較正を拒否
     する。
  4. **`ActivationSync`冪等性チェックを「推奨」から必須の前提条件へ
     格上げ**: 較正により新たに`TurnOn`と判定されるVKが増える以上、
     この機構を踏む打鍵は確実に増える。実機A/Bで「@」が再発しないことを
     確認するまで、較正結果の適用を既定ONにしない。
  5. TSF/COM観測案は非スコープ——「@」の機序（ADR-153/BUG-113）が
     独立した却下理由。`ImmGetDefaultIMEWnd`+`WM_IME_CONTROL`
     ポーリング（awase本体と同型）のみを使う。詳細は決定3参照
     （2026-09-16実機スパイクで`GetGlobalCompartment()`経由の観測が
     常に0を返すことを確認したが、これはスコープの取り違えの可能性が
     高く、TSF不採用の一次理由ではない——opus round5 B5指摘）。
  6. （望ましい）較正レコードに測定時点の`config1.db`/レジストリの
     フィンガープリントを同梱し、現在の値と食い違えばstaleとして
     無効化・再較正を促す。
  7. （望ましい）「変化なし」「Toggle（判別不能）」は保存せず静的分類
     にフォールバックする。
  8. **物理キー検知の実装方式を訂正**: `awase-settings`自身がegui
     テキスト欄や新規LLフックで検知するのではなく、**`awase.exe`本体へ
     「較正モード開始（対象VK）」をPostMessage（既存の`WM_APP+N`
     IPCパターン）し、awase.exe本体の既存フック（injected判定・
     修飾キースナップショット・`is_configured_thumb_key`を全て既に
     持つ）に検知と非actuate保証の両方を担わせ、結果をawase-settingsへ
     返す**方式に変更する。egui/`GetAsyncKeyState`はいずれも
     変換/無変換の非注入判定に使えないことが判明したため。

  詳細な実装タスクリストは
  [176-implementation-tasks.md](176-implementation-tasks.md)参照。
  このタスクリストをopus-adversarial-consultでレビューしてから
  実装に着手する。
related_adr:
  - "ADR-092"
  - "ADR-115"
  - "ADR-119"
  - "ADR-125"
  - "ADR-135"
  - "ADR-140"
  - "ADR-141"
  - "ADR-149"
  - "ADR-153"
  - "ADR-174"
  - "ADR-175"
---

# ADR-176: awase-settingsの明示的な較正UIでモードキーの実効果を測定し、未登録時に静的分類を補完する

## 背景

[ADR-174](174-solo-tap-passthrough-belief-reobservation.md)（BUG-143）で、
GJIの`config1.db`を静的パースして無変換/変換キーのIME意味論
（`ImeToggleKind::On/Off/Toggle`）を判定する`classify_mode_key_ime_action`
（`crates/awase-windows/src/gji_charset_autodetect.rs`）を修正した。
修正自体は実機で正しく動作することを確認済みだが、修正直後にMozc
公式ソース（`google/mozc`）を調査した結果、`config1.db`の
`session_keymap`と`custom_keymap_table`が食い違いうる（GUI実装の
クリア漏れ）という既知の限界が判明した（詳細はBUG-143参照）。

つまり`config1.db`の静的パースは、Google非公開の内部フォーマットを
解釈しているだけでなく、そのフォーマットが実際のGJIバイナリの挙動を
正確に表しているという保証も無い。同様に、MS-IME使用時の判定は
レジストリ値の読み取りに依存しており、これも「設定の記述」と
「実際の挙動」が食い違いうる。

## 目的

`config1.db`/レジストリの静的パースに頼らず、**ユーザーがawase-settings
の専用UIで対象キーを実際に打鍵し、その結果（IMEが実際にON/OFFどちらに
動いたか）を直接測定して**、モードキー（変換/無変換/かな/漢字）の
意味論をawaseが正しく把握できるようにする。

## 対象キー

`ModeKeyCandidate::{Henkan, Muhenkan, Hiragana, Katakana}`
（`crates/awase-windows/src/gji_charset_autodetect.rs:224-229`）を対象
とする。`VK_KANA`等「Win32 API上は固定方向のはず」のキーは非スコープ
とする（対象キー節、下記「非スコープ」参照）。

## 却下した代替案

### 能動的なテストキー送信によるプロービング（通常実行時）

「起動時やGJI検出時に、awase自身が対象キーを合成SendInputで送信し、
その結果を観測してキャリブレーションする」案は**却下**する。ADR-153
ケース3の実機履歴（`docs/known-bugs/BUG-113.md`・`BUG-124.md`）で、
「生キーがGJIへ届くこと」「awase自身が明示IME制御actuationを行う
こと」のどちらか片方だけでも「@」を誘発するのに十分と2回の独立した
実機A/Bで確定している。v5の較正UIは、ユーザーが明示的に較正モードへ
入り、実際に物理キーを押す（awase自身はキーを送信しない）ため、
この却下理由には抵触しない。

### 却下（round1〜3）: 通常実行時のバックグラウンド受動学習

round1（同一アプリ内の弱い代理シグナル）・round2（`ImmGetOpenStatus`
直接読み取り＋`ObserverReported`）・round3（`config1.db`未割当時のみ
補完、`shadow_action`供給層への直接合流）は、いずれも「通常実行時に
バックグラウンドで較正する」という設計だったため、観測チャネルの
到達性・NICOLA親指キーのチョード判定との衝突・awase自身のactuation
による汚染、のいずれか（または複数）が繰り返し発生し、計13件の
Blockerで**却下**した。詳細はgitログの本ファイル過去版参照。

### 却下（round4、v4）: 較正結果を`shadow_action`供給層へ直接合流

v4は較正UIへの転換で観測到達性を解決したが、(a)較正中もawase自身の
actuationが動き続けるため測定が自己成就する（B1/B2）、(b)統合点が
`route_thumb_key_action`の排他振り分けと衝突する（B3）、(c)優先順位の
宣言が実コードと矛盾する（B4）、の3件で**却下**。v5はこれらを
「決定」節のとおり修正する。

## 決定（v5、実装タスクリストは
[176-implementation-tasks.md](176-implementation-tasks.md)参照）

### 1. 較正中はawase自身を完全にバイパスする（B1/B2対応）

較正モード中、`awase-settings.exe`を対象に既存の`disable_apps`機構
（`crates/awase-windows/src/hook.rs:1103-1105`、
`HOOK_STATE.focus_app_disabled`、既定`mstsc.exe`のみ対象の完全
バイパス——「例外なく無効化する」と明記済み）を一時的に適用する。
これにより:

- 較正対象キーの生入力がawaseに一切介入されずGJI/MS-IMEへ直接届く。
- Hiragana/Katakanaも`transport.rs::plan`のSuppress判定に一切
  引っかからず、awase-settings上で正しく較正できる（B2解消）。
- 観測（決定3、`ImmGetDefaultIMEWnd`+`WM_IME_CONTROL`）で見える変化は
  100% GJI/MS-IME自身の反応であり、awaseの自作自演が混入しない
  （B1解消）。「2回一致」は偽陽性
  （flicker等）への防御であり、この自作自演汚染への防御では
  ない点をここで明確に区別する。

### 2. 物理キー検知はawase.exe本体の既存フックに担わせる

`awase-settings`は較正パネルで「較正開始（対象VK指定）」ボタンを
押すと、`awase.exe`本体（トレイウィンドウ、`FindWindowW(w!(
"awase_tray_window"))`）へ`WM_APP+N`（既存のIPCパターン、
`crates/awase-windows/src/lib.rs:299-355`参照）で較正モード開始を
通知する。awase.exe本体は:

- 対象VKの物理（非注入、`LLKHF_INJECTED`で判定）・修飾キー無し
  （Ctrl/Shift/Alt/Win全て`ModifierState`で確認）KeyDownを検知する
  （既存フックがこれらの判定を全て既に持つ）。
- 較正モード中は上記1の`disable_apps`バイパスにより、この検知自体は
  何もactuateしない（フックは検知のみ、通常のshadow-toggle等の処理
  パイプラインには一切入らない）。
- 検知結果（VK、タイムスタンプ）をawase-settingsへ返す
  （`WM_APP+N`応答、または共有メモリ/一時ファイル等、実装タスクで
  詳細化）。

awase-settings側はegui標準のテキスト入力やGetAsyncKeyStateに頼らない
（前者は変換/無変換に対応するegui::Keyが無く、後者は非注入判定が
できないため）。

### 3. 観測はawase.exe本体が決定2と同じIPC経路で兼務する（TSF非スコープ、較正専用ウィンドウ不要）

**2026-09-16の2段階の実機検証で確定**（v6からの訂正、frontmatter
status参照）:

1. 最初のスパイク（`crates/awase-windows/examples/
   ime_observation_spike.rs`）で、`ImmGetContext`+`ImmGetOpenStatus`
   （手法A）と`ImmGetDefaultIMEWnd`+`WM_IME_CONTROL`/
   `IMC_GETOPENSTATUS`（手法B、awase本体の`imm.rs::probe_ime_control`
   と同型）が、生のWin32ウィンドウ上で実際のIME ON/OFF切替を完全に
   一致して正しく追跡することを確認した。TSF
   `ITfThreadMgr::GetGlobalCompartment()`経由の
   `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`は観測期間中一度も実際の
   IME状態変化を反映しなかった（値は常に0固定）——ただしこれは
   「TSFでは観測できない」ことの証明ではなく、`GetGlobalCompartment()`
   というスコープ自体が間違っていた可能性が高い（このGUIDは仕様上
   スレッドマネージャ側のコンパートメントに置かれる。opus round5
   B5指摘）。いずれにせよTSFは「@」機序（GJIのTSFキー横取りとの競合、
   ADR-153/BUG-113）という独立した理由で非スコープのままでよい。
2. このスパイクはwinit/eguiを一切含まない環境で実行されたため、
   「eframeプロセス内で手法Bが機能するか」については何も証明していない
   （opus round5 B4指摘）。そこでADR-125で作成済みの別プロセス観測
   スパイク（`spike_egui_ime_control_probe.rs`）を使い、
   `awase-settings.exe`の実際のバグ報告画面（eframe/egui）に
   フォーカスした状態で、約28秒間（実際のGJI IME ON/OFF切替を説明欄
   フォーカス時・他ウィジェットフォーカス時の両方で反復）手法Bを
   観測した。**結果: 手法Bはこの間ずっと正しくIME状態を追跡し続けた**
   （タイムアウト無し、`elapsed_ms`はほぼ全て15ms未満）。これは
   [ADR-125](125-egui-winit-dynamic-ime-association-focus-model-gap.md)
   の実機検証ログ2の結論（`awase-settings.exe`のeguiメインウィンドウに
   対しても手法Bは正常に機能する）を再現・補強するものであり、
   v6が主張した「eguiメインウィンドウでは機能しない」は**誤りだった**
   （opus round5 B1指摘）。

**採用する設計（v7）**: 較正専用のネイティブウィンドウは新設しない。
決定2で確立したIPC経路（awase-settings→`WM_APP+N`→awase.exe本体）を
そのまま延長し、**awase.exe本体が物理キー検知と観測の両方を兼務する**。

- 較正モード開始時、awase-settingsは自身のトップレベルHWND
  （eguiのメインウィンドウ、`winit`が管理するもの——手法Bが直接
  機能することを上記2で確認済み）をIPCメッセージに含めてawase.exe
  本体へ渡す。
- awase.exe本体は、決定2の物理キー検知に加えて、`imm.rs::
  probe_ime_control`（`awase-windows`クレート内の既存の唯一の
  チョークポイント、新規APIを増やさない）をこのHWNDに対してそのまま
  呼び出し、IME状態をポーリングする。
- 観測結果（VK・物理キー検知タイムスタンプ・観測したIME状態の遷移）を
  同じ`WM_APP+N`応答でawase-settingsへ返す。

この設計により:
- awase-settings側に新しいWin32ウィンドウ・新しい`windows-rs`
  feature・新しい`SendMessageTimeoutW`呼び出し点が一切増えない
  （`tests/architecture_guard.rs`/`lints/actuation_call_guard`の
  走査対象は`awase-windows`のみだが、IMM32呼び出しがそこから一歩も
  出ないため対象範囲外に複雑性が漏れる心配がそもそも無くなる。
  opus round5 M6指摘への対応）。
- `awase-windows`（windows-rs 0.62）と`awase-settings`（windows-rs
  0.58）のバージョン不一致（opus round5 M5指摘）も問題にならない
  ——Win32型は`awase-windows`の外に一切出ない。
- 決定2が確立した「非注入判定はawase.exe本体のフックに担わせる」
  という理由付けと、観測窓口が同一プロセス・同一IPCになることで
  一貫する。

ポーリング間隔・タイムアウトは実機実測の上`tuning-constants.md`に
従い定数化する（`SendMessageTimeoutW`のタイムアウト自体はクロス
プロセス送信のため実効する——awase-settings自身のスレッド内で
同一スレッド宛てに送るのとは違い、opus round5 M4指摘はこの設計には
当てはまらない）。

### 4. 確定条件

同じ遷移結果（`open: false→true`等）を**2回連続で一致**するまで
確定しない（round1レビュアー提案・ユーザー承認済みの方針を維持）。
「変化なし（タイムアウト）」および矛盾する観測パターン（Toggleか
どうか判別できない）は**保存しない**——静的分類（`config1.db`/
レジストリ）にフォールバックする。

### 5. 統合点: `gate_thumb_key_ime_actions`出力の差し替え（B3対応）

学習結果は`shadow_action`供給層（`.or_else()`チェーン）へ直接
合流させるのではなく、**`gate_thumb_key_ime_actions`の出力
（`wiring.henkan`/`wiring.muhenkan`、`ImeToggleKind`型）そのものを
較正結果で差し替える**。統合箇所は2つ、両方に配線する（ADR-119の
教訓：片方だけでは不足）:

- GJI側: `gji_charset_autodetect.rs:768-776`
  （`gate_thumb_key_ime_actions`呼び出し直後、`route_thumb_key_action`
  呼び出し直前）。
- MS-IME側: `runtime/message_handlers.rs:964-966`
  （`delegate_assignment`取得直後）。

この位置で差し替えることで、thumb/非thumbの振り分け・
`mask_auto_detect_for_explicit_config`による明示configマスク・GJI
離脱時のクリアが**すべて既存のまま**効き、新しいactuation合流点は
本当にゼロになる。

### 6. 永続化とstale検出

較正結果は`config.toml`の新設セクションに永続化する（プロセス
再起動を跨ぐ、既存の`henkan_shadow_override`等とは異なるライフサイクル
——詳細は実装タスクリスト参照）。各レコードに、測定時点の
`config1.db`/レジストリのフィンガープリント（`session_keymap`の値・
該当行の内容、またはハッシュ）を同梱する。適用時に現在の値と
突き合わせ、不一致ならstaleとして無効化し静的分類へフォールバック
した上でUIで再較正を促す——これはBUG-143の既知の限界（GUI実装の
クリア漏れによる残留テーブル）の検出手段としても機能する。静的分類
と一致する較正結果は保存しない（差分のみ保存）。

### 7. `keys.ime_detect`との関係（B4対応）

較正結果と`keys.ime_detect`の優先順位は宣言しない。代わりに
BUG-140と同じ「構造的除外」に揃える——対象VKが`keys.ime_detect`
（`on`/`off`/`toggle`いずれか）に登録されている場合、較正UIは
「このキーは`keys.ime_detect`に登録されているため較正結果は
反映されません」と警告し、較正の実行自体を拒否する。`keys.ime_on`/
`ime_off`/`ime_toggle`（既定`Ctrl+変換`/`Ctrl+無変換`）についても
同様に、対象VKが素のVKとして登録されている場合は警告する
（Engine Phase 1が較正結果より先に消費するため——`src/config.rs:
601-609`の実害報告例参照）。

### 8. `ActivationSync`冪等性チェック（必須の前提条件）

`Engine::transition_activation`（`src/engine/engine.rs:456-483`）が
belief の inactive→active 遷移で無条件に`SetOpen(true)`を発行する
（BUG-113 ADR-149追記が実機で2〜3回の`VK_IME_ON`送信を確認済み）。
本ADRは較正によって`TurnOn`と判定されるVKを増やす（＝この経路を
踏む打鍵を増やす）ことが目的であるため、**この冗長送信に対する
冪等性チェック（beliefが既に高信頼度で実状態と一致していれば
`SetOpen`を再送しない）を、本ADRの実装より先に、または同時に
入れることを必須の前提条件とする**。これが実機A/Bで「@」が
再発しないことを確認できるまで、較正結果の適用は既定OFF
（opt-in）とする。

## 未解決点（実装タスクリストで詳細化）

1. `config.toml`永続化のスキーマ設計。
2. `awase-settings`⇔`awase.exe`間のIPC詳細（較正モード開始・結果
   返却のメッセージ形式）。
3. ポーリング間隔・タイムアウトの実機実測値。
4. `ActivationSync`冪等性チェックの具体的な実装方針（別BUGとして
   起票するか、本ADRに含めるか）。
5. 回帰テストの配置（`gji_charset_autodetect.rs`の`PipelineOutcome`
   決定表テストを拡張、ADR-141必須条件2と同型の「既存テストが
   素通りする」落とし穴に注意——較正軸を追加する際は`expected_outcome`
   を仕様として書き直すこと）。Hiragana/Katakanaを含めるなら
   `transport.rs::plan_tests`も対象。

## 非スコープ

- 通常実行時のバックグラウンドでの受動的な学習（round1〜3の設計、
  却下済み）。
- ON→OFF方向・Toggle意味論の較正（v5もOFF→ON方向の較正に集中する）。
- `VK_KANA`等「固定方向のはず」のキーの較正。
- TSF/COMインターフェースによる観測（上記「決定3」——「@」機序
  （GJIのTSFキー横取りとの競合、ADR-153/BUG-113）という独立した理由で
  非スコープ。2026-09-16実機スパイクで`GetGlobalCompartment()`経由の
  観測が常に0を返すことを確認したが、これはスコープの取り違えの
  可能性が高く「TSFでは観測不能」の証明ではない——採用しない理由は
  あくまで「@」機序）。
- `ActivationSync`の冪等性チェック自体の実装（上記「決定8」で必須の
  前提条件と位置づけるが、本ADRの実装スコープ自体には含めない——
  別途対応する）。

## 関連

ADR-092（`classify_mode_key_ime_action`の起源）、ADR-115（打鍵列機能）、
ADR-119（IME actuation合流点は複数箇所に配線が要るという教訓）、
ADR-135（Hiragana/Katakanaへの一般化）、ADR-140（probe/actuation競合）、
ADR-141（Henkan/Muhenkan delegate、`shadow_action`/`route_thumb_key_
action`機構そのもの、「actuation-autoへの二重登録」棄却の先例）、
ADR-149（`VK_IME_ON`重複送信の根本原因調査、`ActivationSync`再送信の
既存文脈）、ADR-153（明示config）、ADR-174/BUG-143（本ADRの直接の
動機）、ADR-175（BUG-142、「固定方向のはず」を信じすぎる失敗モードの
先例）、ADR-125（`awase-settings.exe`のeguiバックエンド`winit`が
IMEコンテキストをデタッチするため`ImmGetContext`が常にHIMC=0を返す、
決定3の「較正専用ネイティブウィンドウ」の直接の根拠）、BUG-140
（同じVKが2つの意味づけ機構に登録されると暴発する、「優先順位では
なく構造的除外」という対処方針の先例）。
