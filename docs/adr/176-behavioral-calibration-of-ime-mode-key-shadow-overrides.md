---
id: ADR-176
title: |-
  awase-settingsの明示的な較正UIでモードキーの実効果を測定し、
  未登録時に静的分類を補完する
status: |-
  **2026-09-17: 176-T0を「較正機能の必須の前提条件」という決定8の位置づけ
  から外し、実装自体を見送り。** opus-adversarial-consultによる2ラウンドの
  レビューの結果、(1)当初案（`handle_engine_activation_sync`への早期
  return）はBUG-113の実送信を止められず既存dedupを壊す、(2)置き場所を
  修正した第2案（`decision.effects`からのstrip）は方向性としては妥当だが、
  較正機能が実際に増やす送信経路（`applied`が構造的に不一致側にある）には
  そもそも当たらず、効く範囲は`NotRomajiInput`/`NotJapaneseIme`経由の
  Inactive→Active往復という極めて狭いケースのみ、(3)Blind環境
  （TsfNative×GJI、BUG-113の環境そのもの）では`applied`一致を根拠に
  SetOpenを止めると、その前提が誤っていた場合にON方向の是正手段が
  構造的にゼロになる、という3点が判明した。較正機能の実質的な安全装置は
  T0ではなく既存のopt-in（既定OFF、実機A/B確認まで結果を適用しない）
  ゲートであり、これは維持する。較正機能（T8〜T10、結果はログのみで
  IME制御には未反映）はT0を待たずに現状のまま進めてよい。較正が実際に
  増やす送信への対策が必要になった場合は、ADR-149が「別ADR起票の価値が
  ある」とした案C（delegateとshadow-toggleの排他性修復）を優先候補とする。
  詳細は[176-implementation-tasks.md](176-implementation-tasks.md)のT0節
  「設計案の棄却」「T0の見送り」を参照。T0はコード変更ゼロのまま。

  **2026-09-17: 176-T8/T9a/T9bの実機検証完了（dragonflyg4）。**
  較正モード中に物理VK_NONCONVERTをIME ON状態で2回押下（各3秒の
  settle window経過までフォーカス保持）し、awase.exe側で
  `[calibration] 確定: vk=VkCode(29) ImeToggleKind::On`、
  awase-settings.log側で`[calibration] 結果を受信: kind=ConfirmedOn`
  （1ms後）を確認、押下検知→試行確定→IPC通知のエンドツーエンドを
  実機で確認した。詳細は
  [176-implementation-tasks.md](176-implementation-tasks.md)の
  176-T9b節「実機検証完了」を参照。次は176-T10（較正パネルUI）。

  **2026-09-16: v8のround6残論点（M1〜M4）を決着実験v2で実機確定、
  実装着手（T1から）。**

  決着実験v2（`crates/awase-windows/examples/
  spike_calibration_decisive_v2.rs`、`WH_KEYBOARD_LL`を専用スレッドで
  持ちつつメインスレッドで`ImmGetDefaultIMEWnd`+`WM_IME_CONTROL`を
  クロスプロセスポーリングする構成）を、`disable_apps`に
  `awase-settings.exe`を実際に追加してawaseを完全バイパスした状態で
  実行した。

  **重要な訂正**: 最初の試行（`awase-settings.exe`をフォアグラウンドに
  しただけでテキスト欄をクリックしていない状態）では、無変換/変換を
  何十回押しても`open`値が一切変化しなかった。この時点で「awase自身の
  自作自演（ActivationSync経由のVK_IME_ON送信）がv1の決着実験で見えた
  IME切り替えの正体だったのでは」という仮説を立てたが、**これは誤り**
  だった。ユーザー指摘により、単に説明欄（テキスト入力欄）をクリックし
  実際にフォーカスしていなかっただけと判明——`IMC_GETOPENSTATUS`が
  返す値はそのスレッドで**キーボードフォーカスを持つウィンドウの
  入力コンテキスト**の状態であり（round6 M1が理論的に指摘していた点、
  今回実測で裏取りされた）、テキスト欄に実際にフォーカスし直したところ
  `disable_apps`でawaseを完全バイパスしたままGJIが正しく反応し、
  `open`値が正しく切り替わることを確認した。**GJIの生キーへの反応は
  awaseの自作自演ではなく本物であり、ADR-176の較正アプローチの前提は
  成立する。** 較正UIの実装では「テキスト入力欄に実際にフォーカスを
  保持し続ける」ことを必須要件として明記する（下記T9/T10参照）。

  **実測レイテンシ（物理キー押下→観測された`open`遷移）**: 247ms・
  277ms・321ms・341ms・362ms・391ms・529ms・1687ms・2295ms
  （9サンプル、多くは250〜400msだが最大2.3秒のケースもある）。
  `SendMessageTimeoutW`の`elapsed_ms`は全サンプルで20ms未満、
  `send_health::SLOW_THRESHOLD_MS`（100ms）には遠く及ばない
  （round6 B2/M3の実測根拠）。`WH_KEYBOARD_LL`を専用スレッドで
  持ちながらメインスレッドで同期`SendMessageTimeoutW`を発行しても、
  フックの取りこぼし（heartbeatカウントの停滞）は観測されなかった
  （round6 M2の裏取り、ただし本番のフック負荷とは条件が異なる点に
  注意）。

  round1〜7の経緯は以下に要約済み。実装はT1（データ構造）から着手する。

  **round6（v7へのレビュー）の結論**: v6の撤回自体（較正専用ネイティブ
  ウィンドウは不要）は実測と整合しており正しかったが、**観測を
  awase-settingsプロセスからawase.exe本体へ移したことで、v6には
  構造的に存在し得なかった衝突が新たに4件（Blocker）発生していた**。

  - **B1（最重要）**: 較正probeは`runtime/ime_refresh.rs:70-78`の
    `app_disabled`早期return（「例外なく無効化する」と明記、observe/
    notify/drift correction/warmup/probeが全停止）と正面衝突する。
    決定1が「完全バイパス」と呼んでいたものが、v7では「キー処理と
    IME refreshは止まるが較正probeだけは動く」という**部分バイパス**に
    変質していた。
  - **B2**: `imm.rs::probe_ime_control`は`send_health::record`
    （グローバルなI/O健全性サーキットブレーカ）へ無条件に給餌する
    （`is_actuation`による除外対象はprobe/actuation fenceのみ）。
    `runtime/executor.rs:986-995`に、**結果がログにしか使われない
    診断目的のクロスプロセスprobeを、まさにこの理由（send_healthの
    誤作動）で削除した**という直接の前例が残っている。較正probeも
    同じカテゴリであり、無策で実装すると「較正した直後の最初の一打が
    おかしい」という較正機能と結びつけようのない再現困難な症状を
    生む。
  - **B3**: 他プロセス（awase-settings）のHWNDをawase.exe本体が
    IPCで受け取って保持し続ける設計に、ライフサイクル規定が無い
    （較正中にawase-settingsがクラッシュ/終了した場合、awase.exeが
    較正モードのまま/disable_appsを外さないまま/死んだHWNDへの
    ポーリングを続ける恐れ。HWND再利用による誤爆リスクも含む）。
  - **B4**: 「awase-settings側の変更は不要」は、新規ウィンドウ・新規
    windows-rs feature・新規`SendMessageTimeoutW`呼び出しが不要という
    意味では正しいが、**awase-settingsが自身のトップレベルHWNDを
    取得する手段がどのタスクにも割り当てられていない**（見出しの
    断定は誤り）。

  Major6件（M1〜M6）: T9の受け入れ基準に実機A/B手順が無い、
  決着実験とv7設計で「probeを出す側がLLキーボードフックを持つ単一
  スレッドプロセスか」という未検証の差がある、タスクリストの250ms根拠が
  round5が無効と判定したスパイクのままになっている、観測の基準点
  （押下前の値）を誰がいつ取るか未定義、`lints/actuation_call_guard`
  許可リスト更新の記載漏れ、決定2・決定3を統合した較正モード状態の
  所有者（フックコールバック側かランタイム側か）が未決定。

  **v8の対応方針**: 下記「決定」節1〜3・実装タスクT6〜T10で全項目に
  対応する。

  round1〜4の経緯・v6→v7の詳細（Blocker5件、round5レビュー）は
  以下に要約済み。

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

### 1. 較正中はawase自身を完全にバイパスする（B1/B2対応、round6 B1で「完全」の意味を訂正）

較正モード中、`awase-settings.exe`を対象に既存の`disable_apps`機構
（`crates/awase-windows/src/hook.rs:1102-1104`、
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

**round6 B1の訂正**: 決定2・決定3により、較正モード中はawase.exe本体
自身が（a）物理キー検知（`hook.rs`のフックコールバック内）と
（b）IME状態観測（`imm.rs::probe_ime_control`呼び出し）の**2つを
明示的に動かし続ける**。`ime_refresh.rs:70-78`の`app_disabled`早期
returnは「observe/notify/drift correction/warmup/probeが全停止」と
書かれており、これは較正probeも含む（無関係な区別が無い）。つまり
「完全バイパス」の実態は**「通常のIME belief更新・actuationパイプライン
からの完全バイパス」**であり、較正専用の検知・観測コードはこのバイパスの
**明示的な例外**として存在する。この2点を必ず文書化・実装する:

1. 較正probe（決定3）と較正キー検知（決定2）は、`app_disabled`
   ゲート・`hook.rs:1102`の早期returnの**手前**に置く、`hook.rs`
   既存の`physical_key_state`更新ブロック（`hook.rs:1094-1097`）と
   同じ配置パターンを踏襲する。
2. 較正probe・較正キー検知の結果は、`ImeModel`/`observation_store`
   （`.claude/rules/ime-belief-architecture.md`のObserve→
   `classify_*`→`reduce()`規律が管理する状態）へ**一切dispatchしない**。
   通常の観測経路とは完全に独立したデータパスとして実装する
   （dylintまたは`architecture_guard`相当のテキスト走査で「較正コードが
   belief書き込みAPIを呼んでいない」ことを固定できないか、T9で検討）。

### 2. 物理キー検知はawase.exe本体の既存フックに担わせる

`awase-settings`は較正パネルで「較正開始（対象VK指定）」ボタンを
押すと、`awase.exe`本体（トレイウィンドウ、`FindWindowW(w!(
"awase_tray_window"))`）へ`WM_APP+N`（既存のIPCパターン、
`crates/awase-windows/src/lib.rs:299-355`参照）で較正モード開始を
通知する。このメッセージには対象VKに加えて、**awase-settings自身の
PIDとトップレベルHWND**を含める（round6 B3/B4対応、下記・決定3参照）。

awase-settingsが自身のトップレベルHWNDを取得する手段は、UIスレッドから
`GetActiveWindow`（または`GetForegroundWindow`、自プロセスが
フォアグラウンドであることを確認した上で使う）を呼ぶ——`windows`
0.58の既存feature（`Win32_UI_WindowsAndMessaging`）で足り、新規
feature追加は不要（round6 B4対応）。**`FindWindowW`によるクラス名
検索は採らない**——winitの既定クラス名は汎用の`"Window Class"`であり、
`focus/imm_learning.rs:22-23`がBUG-107の文脈で「プロセス間で衝突する」
と明記している。

awase.exe本体は:

- 対象VKの物理（非注入、`LLKHF_INJECTED`で判定）・修飾キー無し
  （Ctrl/Shift/Alt/Win全て`ModifierState`で確認）KeyDownを検知する
  （既存フックがこれらの判定を全て既に持つ）。**検知コードは
  `hook.rs:1102`の`app_disabled`早期returnより手前に置く**——決定1の
  訂正どおり、この早期returnは較正検知も含めて全停止させるため
  （round6 m3対応、既存の`physical_key_state`更新ブロック
  `hook.rs:1094-1097`と同じ配置パターン）。
- 較正モード中は上記1の`disable_apps`バイパスにより、この検知自体は
  何もactuateしない（フックは検知のみ、通常のshadow-toggle等の処理
  パイプラインには一切入らない）。
- 検知結果（VK、タイムスタンプ）をawase-settingsへ返す
  （`WM_APP+N`応答、または共有メモリ/一時ファイル等、実装タスクで
  詳細化）。

**較正モード状態の所有者（round6 M6対応、round8で訂正）**: 物理キー検知は
`HOOK_STATE`（LLフックコールバック側、atomicsで管理される既存の
世界）で行い、観測ポーリング（決定3）はランタイム側
（`AppState`/`spawn_local`タイマー）で行う——実行文脈が異なる2つの
処理を1つの新しい裸のグローバルstaticにまとめない。較正モードの
ON/OFFと対象VKは`HOOK_STATE`側に持たせる（フックコールバックが
`app_disabled`判定と同じタイミングで読む必要があるため）。**PID・HWNDは
`HOOK_STATE`には持たせない**（round8訂正: フックコールバックがPID/HWNDを
使う場面は無く、フォーカス検証はメインスレッドが`Runtime::
calibration_session_pid()`/`platform.focus.pid()`で行う。HOOK_STATEに
不要な状態を増やすとtorn readの軸が増えるだけでなく、176-T7 round7 S1が
「HWNDは運ばない」と決めた結論とも整合しない）。ランタイム側の観測ループは
`HOOK_STATE`の較正対象VKの状態を都度読み取るだけの関係にする（ADR-164が
集約した「裸のグローバルstaticより既存singletonへの集約を優先する」方針に
従う）。

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

**採用する設計（v8、round7/round9で訂正）**: 較正専用のネイティブ
ウィンドウは新設しない。決定2で確立したIPC経路（awase-settings→
`WM_APP+N`→awase.exe本体）をそのまま延長し、**awase.exe本体が物理キー
検知と観測の両方を兼務する**。

- 較正モード開始時、awase-settingsは自身の**PID**（`WM_CALIBRATION_
  START`）をIPCメッセージに含めてawase.exe本体へ渡す。**HWNDは運ばない**
  ——176-T7実装時のopus-adversarial-consultレビュー（round7 S1）で、
  較正の測定はそもそも「awase-settingsにフォーカスがある間」しか
  成立しないため、対象HWNDはawase.exe自身のライブなフォーカス追跡
  （`self.platform.focus.current.root_hwnd`）から取れば足り、IPCで
  送るとstaleness軸が増えるだけと判断したため（決定2も同じ理由で
  同様に訂正済み）。
- awase.exe本体は、決定2の物理キー検知に加えて、`imm.rs::
  probe_ime_control`（`awase-windows`クレート内の既存の唯一の
  チョークポイント、新規APIを増やさない。較正probe専用の薄いラッパ
  `probe_ime_open_for_calibration`経由）を、観測tickごとに
  ライブなフォーカス先のHWNDに対して呼び出し、IME状態をポーリングする。
- 確定/却下の結果（`ImeToggleKind::On`確定、または`Toggle`の決定的証拠
  による却下）を、新規`WM_CALIBRATION_RESULT`（`WM_APP+31`）で
  awase-settingsへ返す（176-T9b、実装済み）。HWNDと同じ理由で
  こちらもIPCでHWNDを運ばず、awase-settings側に新設した固定クラス名の
  メッセージ専用ウィンドウを`FindWindowW`で探して送る（round8で
  「`with_msg_hook`はOS由来のモーダルループに脆い」と判明したため、
  `HWND_MESSAGE`の自前ウィンドウ+専用WndProcを採用）。

**round6 B2の対応（`send_health`汚染）**: `imm.rs:263`の
`send_health::record`は probe/actuation を問わず無条件に走る。
`runtime/executor.rs:986-995`には、結果がログにしか使われない診断
目的のクロスプロセスprobeを「`send_health`のグローバルなサーキット
ブレーカを誤作動させる」という理由で削除した前例が残っている。
較正probeは同じカテゴリ（本番のbeliefには流さない測定専用の
クロスプロセスprobeを数秒間短間隔で回す）であるため、**較正probeは
`send_health::record`へ給餌しない**。`send_ime_control_raw`
（クレート唯一のチョークポイント、分割しない方針が明記されている）
自体は変更せず、較正probe専用の薄いラッパ、または`record`呼び出しを
スキップするフラグ引数を追加する形で対応する（実装方式はT9で決定）。

**round6 B3の対応（他プロセスHWNDのライフサイクル、round9で訂正）**:
HWNDをIPCで運ばずライブなフォーカス追跡から都度取得する設計
（上記）にしたことで、「キャッシュしたHWNDが別ウィンドウに化ける」
というHWND再利用そのものの懸念は構造的に発生しない。ただし別の2つの
懸念が残るため、観測tickごとに以下を確認する（opus-adversarial-consult
レビューround9 S7対応）:
- **同一PID・別HWND**: awase-settingsがネイティブのファイルダイアログ
  等を開き、同一PIDのまま別ウィンドウにフォーカスが移るケース。
  該当tickの試行を破棄する（中止はしない）。
- **PID再利用**: awase-settingsが落ちて同じPIDが別プロセスに再利用
  されるケース。`focus.process_name`が`awase-settings.exe`と一致する
  ことも併せて確認する（`calibration_ipc::is_awase_settings_process_
  name`を流用）。不一致なら較正モードを中止する。
- 較正モード全体のタイムアウト（176-T6/T7で実装済み）は、awase-settings
  からのSTART再送（keepalive）が一定時間無い場合に自動的に
  `disable_apps`を戻し較正モードを解除する。

**round6 M2の対応（probeを出す側のスレッド）**: 決着実験
（`spike_egui_ime_control_probe.rs`）はLLキーボードフックを持たない
専用プロセス・専用メッセージループスレッドから`SendMessageTimeoutW`を
送っていたが、awase.exe本体は単一スレッド・メッセージループ駆動
（CLAUDE.md「Concurrency model」）であり、**そのスレッドがLL
キーボードフックのコールバックスレッドでもある**。同一スレッドから
同期`SendMessageTimeoutW`を出すと、その間フックコールバックの応答が
遅れ、`LowLevelHooksTimeout`（既定~300ms）を超えるとフックが黙って
外されるリスクがある（`SMTO_ABORTIFHUNG`は「呼び出し中にハングし
始めた相手」には効かず、宣言タイムアウトを超えて~5sブロックしうる
ことがBUG-34として記録済み）。較正probeは
`win32_async::run_with_timeout`/offload経由でワーカースレッドに
出す（`send_health.rs`が「エンジンスレッド直呼びと offload経由の
両方がある」と記す既存パターンに従う）。

この設計により:
- awase-settings側に新しいWin32ウィンドウ・新しい`windows-rs`
  feature・新しい`SendMessageTimeoutW`呼び出し点が一切増えない
  （自身のPID/HWND取得のための`GetActiveWindow`呼び出し自体は増える
  ——決定2参照、「変更が一切不要」という意味ではない）。
  `tests/architecture_guard.rs`/`lints/actuation_call_guard`の
  走査対象は`awase-windows`のみだが、IMM32呼び出しがそこから一歩も
  出ないため対象範囲外に複雑性が漏れる心配は無くなる。ただし
  `lints/actuation_call_guard::RESTRICTED_CALLS`の`probe_ime_control`
  許可呼び出し元リスト（現行6件）には較正probeが7件目として増える
  ——ガード**内**の宣言は1件増える（`.claude/rules/complexity-budget.md`
  の1-in-1-out対象、未発効だが実装時のコミット本文に「棚卸しではなく
  新規追加」と明記すること）。
- `awase-windows`（windows-rs 0.62）と`awase-settings`（windows-rs
  0.58）のバージョン不一致は問題にならない——Win32型は`awase-windows`
  の外に一切出ない。
- 決定2が確立した「非注入判定はawase.exe本体のフックに担わせる」
  という理由付けと、観測窓口が同一プロセス・同一IPCになることで
  一貫する。

**round6 M3の対応（ポーリング間隔の実測根拠）**: `ime_observation_
spike.rs`（winit/eguiを含まない生ウィンドウでのスパイク）の
`TIMER_INTERVAL_MS=250`は、eframe環境での可否について反証能力が
無いため根拠に使わない。egui環境で実際に確認できた値——決着実験
（`spike_egui_ime_control_probe.rs`）の`POLL_INTERVAL_MS=100`/
`SEND_IME_CONTROL_TIMEOUT_MS=50`——を出発点とし、T9で実機実測の上
`tuning-constants.md`に従い定数化する。

**round6 M4の対応（観測の基準点と`None`の扱い）**: 観測ポーリングは
較正モード開始（決定2のIPCメッセージ受信）と同時に開始し、物理キー
検知（決定2）をトリガに開始するのではない——押下前の`open`値を
基準点として持っておく必要があるため。`SendMessageTimeoutW`失敗
（`open=None`）は「IMEが変化しなかった」（決定4の「変化なし」）とは
区別し、再試行として扱う（`None`を「変化なし」に潰すと、たまたま
probeが失敗した回が保存されず「何回やっても較正が完了しない」という
症状になる）。

**決着実験の条件・ログ抜粋（round6 m5対応、証跡をリポジトリ内に残す）**:
2026-09-16 06:52:40〜06:53:08、`spike_egui_ime_control_probe.exe`
（別プロセス）を起動し`awase-settings.exe --bug-report`にフォーカス
した状態で、GJIを説明欄フォーカス時・他ウィジェットフォーカス時の
両方で反復的にON/OFF切替した。既知の条件: GJIがアクティブなIME
だった。**未確認・記録漏れの条件**: awase.exe本体（デーモン）が
このとき通常どおり動作していたか、`disable_apps`が
`awase-settings.exe`に適用されていたかは記録していない（=通常運用
どおりawase.exe自身のフックが介入していた可能性がある。決定1が
前提とする「較正中はawase自身を完全バイパスする」状態そのものでは
測定していない）。この点は今後の実機A/B（実装タスクT9/T14）で
`disable_apps`適用下でも同じ結果が再現することを確認すること。

ログ抜粋（`awase-settings.exe`がフォーカスを持っていた区間の冒頭・
ON/OFF遷移の一部・末尾。全文はこのセッションのスクラッチファイルに
あったが永続化されていない）:

```
06:52:40.851 [FOCUS] hwnd 0x270B42 class=Window Class process=awase-settings.exe
06:52:40.853 [IME-CTRL] ime_wnd=0x330C12 open=Some(false) elapsed_ms=0
06:52:43.491 [IME-CTRL] ime_wnd=0x330C12 open=Some(true) elapsed_ms=0   <- OFF→ON
06:52:49.527 [IME-CTRL] ime_wnd=0x330C12 open=Some(false) elapsed_ms=4  <- ON→OFF
06:52:48.698 [IME-CTRL] ime_wnd=0x330C12 open=None elapsed_ms=63        <- 唯一の一時的な観測失敗
06:53:08.545 [FOCUS] hwnd 0x270B42 -> 0x3042C process=WindowsTerminal.exe
```

（`elapsed_ms`はほぼ全区間で15ms未満、タイムアウトによる`None`は
上記1回のみ。実測根拠として、M3のポーリング間隔決定・M4の再試行
ロジック実装時に参照すること。）

### 4. 確定条件

同じ遷移結果（`open: false→true`等）を**2回連続で一致**するまで
確定しない（round1レビュアー提案・ユーザー承認済みの方針を維持）。
「変化なし（押下前後で`open`値が変わらなかった、タイムアウトではない）」
および矛盾する観測パターン（Toggleかどうか判別できない）は**保存
しない**——静的分類（`config1.db`/レジストリ）にフォールバックする。
`SendMessageTimeoutW`失敗（`open=None`）は「変化なし」と区別し
再試行する（決定3 round6 M4対応参照、混同すると「何回やっても
較正が完了しない」症状になる）。

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

### 8. `ActivationSync`冪等性チェック（前提条件から撤回、opt-inゲートを実質的な安全装置とする）

`Engine::transition_activation`（`src/engine/engine.rs:456-483`）が
belief の inactive→active 遷移で無条件に`SetOpen(true)`を発行する
（BUG-113 ADR-149追記が実機で2〜3回の`VK_IME_ON`送信を確認済み）。
当初は本ADRが較正によって`TurnOn`と判定されるVKを増やす（＝この経路を
踏む打鍵を増やす）ことを理由に、この冗長送信への冪等性チェック
（176-T0）を実装より先に入れる必須の前提条件としていた。

**2026-09-17、opus-adversarial-consultによる2ラウンドのレビューで
この因果関係自体が成立しないと判明し、前提条件から外した**
（詳細は[176-implementation-tasks.md](176-implementation-tasks.md)の
T0節）。較正が実際に増やす送信経路（物理キー→shadow-toggle→belief
false→true→`ActivationSync`）では、その時点の`applied`（awase自身が
最後に送ったコマンドの記録）は常に不一致側にあり、`applied`一致を
根拠とする冪等性チェックは構造的に発火しない。加えてTsfNative×GJI
（BUG-113の環境そのもの）は`FeedbackPolicy::Blind`のため、`applied`は
「実際にIMEが開いた」ことの証拠にはならず、これを根拠にSetOpenを
止めるとON方向の是正手段（`apply_force_on_for_imm_broken`）が同じ
条件で既に止まっているため構造的にゼロになるリスクがある。

**較正結果の適用が実機A/Bで「@」が再発しないことを確認できるまで
既定OFF（opt-in）とする**、という条項は維持する——これが本ADRの
実質的な安全装置であり、T0の有無に依存しない。176-T0自体
（ActivationSyncの冗長SetOpen抑止、効果範囲は`NotRomajiInput`/
`NotJapaneseIme`経由のInactive→Active往復のみに限られる独立した
クリーンアップ）は今回見送り、実装しない。較正が実際に増やす送信への
対策が必要になった場合は、ADR-149の案C（delegateとshadow-toggleの
排他性修復）を優先候補とする。

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
- `ActivationSync`の冪等性チェック（176-T0）自体の実装。上記「決定8」
  参照のとおり2026-09-17に前提条件から外し、見送りとした。

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
IMEコンテキストをデタッチするため`ImmGetContext`（手法A）が常に
HIMC=0を返す一方、`ImmGetDefaultIMEWnd`+`WM_IME_CONTROL`（手法B）は
正常に機能することを実証済み——決定3がawase-settingsのeguiメイン
ウィンドウをそのまま観測対象にできる直接の根拠）、BUG-140
（同じVKが2つの意味づけ機構に登録されると暴発する、「優先順位では
なく構造的除外」という対処方針の先例）。
