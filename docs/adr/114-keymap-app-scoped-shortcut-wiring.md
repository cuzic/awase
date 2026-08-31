# ADR-114: `[[keymap]]`（アプリ別ショートカット再割当）の未配線を解消する

## ステータス

**採用・実装済み（2026-08-31）。** 設計は Opus 2体による敵対的レビュー r1→r4 で
収束（r1 で新規 Critical 3件、r2 で新規 Critical 1件・Major 3件を検出・修正、
r3→r4 で Down/Up 非対称を解消）。実装タスク分解（`114-implementation-tasks.md`）も
別途 r1→r3 のレビューで収束させ、T1a〜T11 を実装した
（`crates/awase-windows/src/keymap.rs`・`runtime/message_handlers.rs`・
`runtime/mod.rs`・`runtime/focus_tracking.rs`・`output/held_modifiers.rs`（新設）・
`state/keymap_latch.rs`（新設）・`hook.rs`・`vk.rs`・`ime.rs`・`app/mod.rs`・
`app/bootstrap.rs`・`crates/awase-settings/src/main.rs`・`src/config.rs`）。
`cargo xwin check`/`clippy -D warnings`/`fmt`/`machete` 全 green、
`architecture_guard`（61件）・`golden_scenarios`（24件）・`layer_boundary_guard`
（8件）・lib 全ユニットテスト（867件）green。Windows 実機ソークは未実施。

## コンテキスト

`[[keymap]]`（ADR-037「キーマップ再割当設計」、`config.toml` の `[[keymap]]` セクション）は
2026-05-24 に設計・実装され、以下は完成している。

- `src/config.rs::KeymapRule`（`app` / `from` / `to`）
- `crates/awase-windows/src/keymap.rs::KeymapTable`（コンパイル・`filter_active`・`find_match`）
- 設定 GUI（`awase-settings`）でのキャプチャ・編集
- `runtime/focus_tracking.rs:171` — フォーカス変更のたびに
  `platform_state.keymap.active_keymaps = all_keymaps.filter_active(&process_name)`
  を実行し、現在のフォーカス先に適用可能なルールへ絞り込む

しかし **`KeymapTable::find_match` を実際のキー処理経路から呼ぶ箇所がコードベースに
一つも存在しない**（`grep -rn find_match` は定義以外ノーヒット）。つまりユーザーが
`[[keymap]]` を設定しても一切効果がない死んだ機能である。この発見と経緯は
[[project_adr110_key_remap_design_2026_08_28]] に記録済み。本 ADR はこの未配線を
解消する設計を確定する。

**ADR-037 とのスキーマ差分（訂正）**: ADR-037 のサンプルは
`app_class = "Chrome_WidgetWin_1"`（ウィンドウクラス名）だが、実装された
`KeymapRule.app` は**プロセス名**（`src/config.rs`、`filter_active` が
`process_name` と比較）であり、ウィンドウクラスではない。本 ADR は ADR-037 の
「ルール構文・修飾キー解放/復元シーケンス」を原設計として踏襲するが、
スコープ識別子の実装（プロセス名ベース）は ADR-037 のサンプルと異なる点を
ここで明記し、ADR-037 側のサンプルは実装により実質的に superseded 扱いとする。

### ADR-110/111 との関係（訂正: ADR-110 は実装後に完全撤回済み）

ADR-110「単純物理キーの恒久リマップ」（`[[key_remap]]`、CapsLock ⇔ Left Ctrl 等を
フックスレッド最初期で vk 書き換えする恒久的な入れ替え、`Alt なりすまし`
`state/alt_impersonation.rs` と同じ挿入点）は、**実装され（PR #120）、latch 起因の
stuck modifier バグ3件が見つかって修正され（BUG-100, PR #121）、その後
ADR-111「Caps(英数)⇔Ctrl 入れ替え専用プリセット」の r4 決定により
バックエンドごと完全に撤回された（PR #123）**。`state/key_remap.rs`・`hook.rs` の
latch/ダブルバッファ・`config.toml` の `[[key_remap]]` スキーマ・設定 GUI は
現在のコードベースに一切存在しない。撤回理由は「グローバル・静的なリマップという
現行方式は、将来の**アプリケーションごとに動的にキー割当てを変更する機能**
（=本 ADR の対象）に置き換えられる見込みとなったため」（ADR-111 背景6）であり、
**本 ADR-114 はまさにその「将来機能」を引き受けるものである**。ADR-111 は
「必要であれば ADR-110/自身の過去の設計・実装（git 履歴）を参照しつつ作り直す」
ことを明示的に想定しており、本 ADR は BUG-100 の教訓（latch ライフサイクルの
3つの漏れ経路、後述の決定4）をその参照先として引き継ぐ。

`[[keymap]]`（本 ADR）と ADR-110/111 が扱っていた「物理キーの恒久リマップ」は
対象もレイヤーも別物である点は変わらない。「アプリ A では Ctrl+I を F7 として
扱う」という **アプリスコープ付きの一時的なショートカット横取り** は、
フォーカス文脈（`active_keymaps`）が既に確定しているエンジンスレッド側でしか
判定できない。この区別自体は ADR-037 の設計時点で既になされており正しいが、
「じゃあどこで呼ぶか」が未決のまま実装が止まっていた。

### PowerToys Keyboard Manager の参考実装

Microsoft PowerToys の Keyboard Manager モジュールは同種の「アプリ別ショートカット
再割当」を持ち、公式 devdocs
（[keyboardmanager.md](https://github.com/microsoft/PowerToys/blob/main/doc/devdocs/modules/keyboardmanager/keyboardmanager.md)、
[keyboardeventhandlers.md](https://github.com/microsoft/PowerToys/blob/main/doc/devdocs/modules/keyboardmanager/keyboardeventhandlers.md)）
に実装が要約されている。参考にした要点は次の4つ。

1. **処理順序**: `HandleKeyboardHookEvent` は「単一キーリマップ → アプリ別ショートカット
   → グローバルショートカット」の順で評価する（devdocs にこの順序自体は明記されている。
   ただし「先に処理しないとショートカット側の判定が壊れる」という因果関係までは
   devdocs に明記されておらず、本 ADR ではこの部分を断定しない）。awase の
   「単一キーの恒久リマップ」（ADR-110/111、現状は撤回済みで存在しない）が将来復活
   した場合に同じ優先順位問題を持ちうる — 本 ADR の決定7で先取りして順序を固定する。
2. **フォーカス取得と UWP フォールバック**: `GetCurrentApplication` は
   `GetForegroundWindow` → `get_process_path` が基本経路だが、全画面 UWP アプリでは
   失敗するため `GetGUIThreadInfo` にフォールバックする。awase は
   `focus/` 層の `AppKind` 分類・学習キャッシュで同種の問題（UWP 往復での取得失敗、
   [[project_bug18_appkind_uwp_flip_dropped_chars]]）に**既存インフラで**対処済みであり、
   本 ADR はそれを再利用するだけでよい（新規のフォールバック実装は不要）。
3. **ショートカット完了までのアプリスコープ持続**: `KeyboardManagerState.activatedApp`
   は「そのショートカット自体が実行中のアプリのフォーカスを奪う」場合（Alt+Tab を
   割り当てた場合など）に備え、**ショートカットが完全に解放されるまでフォーカス変更を
   無視して同じアプリの remap を適用し続ける**。理由は明記されている:
   「さもないと一部のキーが release されないまま残る状態になりうる」。
   これは決定4（KeyUp の扱い）に直接効いている教訓であり、独立に BUG-100
   （ADR-110 `[[key_remap]]` の latch、上述「ADR-110/111 との関係」参照）で
   実際に踏んだ「latch が3つの経路でクリアされず stuck する」失敗と同じ結論に
   到達する。決定4はこの2つの独立した先例（PowerToys の設計判断・BUG-100 の
   実障害）の両方を踏まえる。
4. **自己注入イベントの除外**: remap 出力は `dwExtraInfo` にマーカーを付け、フック
   自身がそれを見て再処理をスキップする（devdocs によれば実際は単一のフラグではなく
   `KEYBOARDMANAGER_INJECTED_FLAG`/`KEYBOARDMANAGER_SHORTCUT_FLAG`/
   `KEYBOARDMANAGER_SINGLEKEY_FLAG` の複数種を用途別に使い分けている）。
   awase には既に同種の仕組みが `INJECTED_MARKER` / `TSF_MARKER` / `IME_KANJI_MARKER` と
   `hook.rs::is_self_injected` として存在するため、新規マーカーは不要でそのまま流用する
   （decision 6）。

一方で PowerToys から意図的に踏襲しない点もある。

- PowerToys は設定ファイルを**起動時に一度だけ読み込み、UI 側からの外部書き換えに
  依存する**（実行時ホットリロードなし）。awase は他の設定項目を `reload_config()`
  でホットリロードする慣習があるため、本 ADR では `[[keymap]]` もそれに揃える
  （decision 8）。
- PowerToys のショートカット→ショートカット remap は複数キー（修飾子込みの完全な
  組み合わせ）を to 側に持てるが、awase の `KeymapRule.to` は単一 VK のみ
  （`CompiledKeymap.send_vk: Option<VkCode>`）。本 ADR はこの制約を変更しない
  （スコープ外、「未解決の疑問」に記載）。

## 決定

### 決定1: 配線の挿入点は `message_handlers.rs::deliver_key_event`

`[[keymap]]` の照合・消費は、LL フック（`hook.rs::hook_callback`、専用フックスレッド）
ではなく、`deliver_key_event`（エンジンスレッド、`with_app` 経由でシングルスレッド
実行が保証される）で行う。理由:

- `active_keymaps` は `filter_active()` により**既にエンジンスレッド側でフォーカス
  フィルタ済み**（`focus_tracking.rs:171`）。フックスレッドには
  `RUNTIME`（フォーカス状態を含む）へのアクセスが構造的にない
  （`hook_callback` のドキュメントコメント参照）ため、そもそも同じ判定をフック側で
  やり直すことはできない。
- 既存の `[[post_bypass]]`（同じ「アプリ + キーでスコープされた横取り」形状の機能）が
  `deliver_key_event` 内 `consume_post_bypass` として同じレイヤーで実装済みであり
  （`ScopedOneShot<ForegroundScope, PostBypassArm>` を使用）、直接の実装前例になる。
  ただしスコープ指定の粒度は完全に同じではない: `PostBypassRule` は
  `process`/`class` の両方を持つのに対し、`KeymapRule` は `process`
  （`app` フィールド、後述 ADR-037 との差分参照）のみでウィンドウクラスを
  持たない。「アプリ + キーでスコープされた横取り」という形状は共通だが、
  スコープの識別子が異なる点は実装時に混同しないよう明記しておく。

**`filter_active` の前方一致は、本 ADR の実装と同時に修正する。**
`KeymapTable::filter_active`（`keymap.rs`）は
`lower.starts_with(a) || lower == a` という前方一致でプロセス名を照合するが、
`state/app_suppression.rs` の `matches_disabled_app` は**まさにこの
`keymap.rs::filter_active` を名指しした反面教師コメント**を持つ:
「前方一致は使わない — `keymap.rs::filter_active` のような `starts_with` 方式は
予期しない過剰マッチ（例: `"note"` が `"notepad.exe"` に誤爆）を招く」。
`[[keymap]]` が死んだ機能である間はこの誤爆が無害だったが、本 ADR で配線すると
実害化する（例: `app = "code"` が `codeblocks.exe` にも誤って一致する）。
`filter_active` の照合を `app_suppression::normalize_process_name`
（小文字化 + 末尾 `.exe` 除去のみで前方一致はしない）と同じ完全一致方式へ揃える。

### 決定2: 挿入順序は「latch チェック（KeyUp 解放 + KeyDown repeat 抑制、
新規・最優先）→ `PumpContext::Nested` 早期return → NonText パススルー →
`[[keymap]]` KeyDown 新規照合（新規）→ `[[post_bypass]]` 消費 →
`process_key_event`（NICOLA エンジン）」

`deliver_key_event` は現状、`PumpContext::Nested` 早期return（L136-139）と NonText
パススルー（L149-159）の2つが `[[keymap]]` 照合より手前に存在する早期return分岐で、
どちらも `app.executor.enqueue_reinject(event)` で即座に OS へ流す。**この2分岐が
`[[keymap]]` の照合より先に来ると、決定4 の latch テーブルに entry が
残っている vk のイベントがこの2分岐でそのまま生の物理イベントとして OS へ
再注入され、latch は解放されないまま stuck する**（BUG-100 の経路1・2 と同型の
構造的リーク。「latch チェック」参照）。

したがって照合は2段階に分ける。**KeyUp の latch 解放と KeyDown の repeat 抑制は
どちらも同じ最優先ステップ（ステップ1）に置き、Nested/NonText より前で対称に
扱う** — 片方だけを先に動かすと、latch が残った状態で NonText フォーカスに
KeyDown だけが先に素通りし、対応する KeyUp だけがステップ1 で食われるという
Down/Up 非対称（OS 側で「押しっぱなし」に見えるイベント）が生じるため。

1. **latch チェック（新規、`deliver_key_event` の一番最初、`origin` や
   `focus_kind` を問わず必ず実行）**: 到着したイベントの vk が決定4の latch
   テーブルにエントリを持つなら、他のどの早期return（`Nested`・`NonText`）よりも
   先にここで処理する。
   - **KeyUp** の場合: consume して latch を解放し `KeyDelivery::Consumed` を返す。
   - **KeyDown** の場合（決定4「自動リピート抑制」）: `find_match` を呼ばず
     黙って consume する（`target_vk` は再送しない、repeat 抑制）。
   latch にエントリが**無い** vk はこのステップでは何もせず素通りする。
   `Nested`/`NonText` の早期returnはこのチェックの**後**に置く。
2. **KeyDown 新規照合（`[[keymap]]` 本体、`find_match` の呼び出し。現行案どおり
   `NonText` パススルーの後・`[[post_bypass]]` 消費の前）**: ステップ1 で
   latch 済みと判定された vk はここに到達しない（ステップ1 で既に consume 済み）
   ため、ここで扱うのは「まだ latch されていない vk の新規 KeyDown」のみである。
   `[[keymap]]` はショートカットの意味そのものを差し替える機能であり、NICOLA
   エンジンに一切見せてはならない（そうしないと `Ctrl+I` の `I` がエンジンの
   同時打鍵判定に巻き込まれうる）。同時に `[[post_bypass]]` より先に評価する。
   理由: `[[post_bypass]]` の armed 判定は「`process_key_event` が `PassThrough`
   を返した Ctrl+key」を起点にする（`cancel_composition_and_arm_post_bypass_on_ctrl`）。
   `[[keymap]]` がその手前で消費すれば `PassThrough` 自体に到達しないため、
   `[[post_bypass]]` の armed 判定と自然に競合しない。

同じ `(app, from)` の組が `[[keymap]]` と `[[post_bypass]]` の両方に設定された場合は
`[[keymap]]` が常に勝つ（`[[post_bypass]]` 側には到達しない）。これはユーザー設定の
矛盾であり検知・警告はしない（`[[post_bypass]]` 自体、`[[keymap]]` と無関係に単独でも
到達不能な組み合わせを作れるため、既存の設定バリデーションの粒度を超える）。

**`CTRL_CONSUMED_SINCE_DOWN` は `[[keymap]]` が消費するキーでも意図的に立てる。**
`hook.rs` はフックスレッド側で、Ctrl held 中の非 Ctrl KeyDown（親指キー以外）に
対して無条件にこのフラグを立てる。これは `[[keymap]]` の照合（エンジンスレッド）
より**時系列で先**に実行され、`[[keymap]]` が後でそのキーを消費してもフラグは
取り消されない。`runtime/key_pipeline.rs` の「Ctrl+無変換 IME OFF ミスタイプ
救済窓」の起点判定がこの値を参照するが、意味的には正しい: `Ctrl+I` が
`[[keymap]]` によってショートカットとして消費されたのであれば、それは
「Ctrl が（NICOLA エンジンではなく `[[keymap]]` によってではあるが）本当に
消費された」ことに変わりないため。これは新しい決定ではなく、既存の
`CTRL_CONSUMED_SINCE_DOWN` の意味論を変えずに済むことの確認である。

**既知の限界（v1 スコープとして明示的に受け入れる）**: KeyDown 照合が NonText
パススルーの**後**にあるため、`[[keymap]]` は `FocusKind::NonText` と分類される
フォーカス先（タスクバー、ブラウザのページ本文、ファイラのリストビュー、ゲーム等）
では一切効かない。ADR-037 が挙げる動機例（Chrome での Ctrl+I 再割当）は、
`Chrome_WidgetWin_1` 全体ではなく実際にはテキスト入力欄にフォーカスがある場合
限定でしか動かないことになり、ユーザーからは「同じアプリなのにフォーカス場所に
よって効いたり効かなかったりする」という再現性の低い挙動に見える。

これは意図的な v1 判断である: NonText 早期returnより前に KeyDown 照合を動かすと、
`[[keymap]]` の送信（決定3、`SendInput` による `target_vk` 注入）が
`FocusKind::NonText` という「awase が一切手を出さない」既存の不変条件
（`[[post_bypass]]` も含め、現行コードの早期returnはすべてこの後段にある）を
初めて破ることになり、タスクバー・タスクスイッチャー等での副作用を個別に
監査する追加コストが発生する。本 ADR はまず `[[keymap]]` を「未配線」から
「Text フォーカスで動作する」状態にすることを優先し、NonText への拡張は
実地でのニーズが確認され次第、別 ADR で NonText 個別ケースの安全性を検証してから
判断する（「未解決の疑問」に追記）。

### 決定3: 判定対象は KeyDown のみ、`to` 指定時は ADR-037 の
`HeldModifiers` パターンで即時送信

`find_match(vk, mods)` は物理キーの **KeyDown** に対してのみ呼ぶ（`is_keydown` ガード）。
マッチした場合:

- 元の物理 KeyDown を consume（NICOLA・OS どちらにも渡さない）。
- `Some(target_vk)` の場合、ADR-037 が規定した手順で送信する: 現在保持中の修飾キー
  を `HeldModifiers::read()` で読み取り → **`from` の `ctrl`/`shift` に対応する分だけ**
  解放 → `target_vk` の Down+Unicode-scan なしの Down/Up ペアを **同一 `SendInput`
  バッチ**で送信 → 解放した分だけ復元。同一バッチにする理由は Chrome cold-start 検出
  （VK_A+BS アトミックバッチ）と同じ（描画前に完結させ、中間状態を外部に見せない、
  `docs/adr/index.md` 内「アトミックバッチ送信は UI の副作用を消せる」の教訓）。
  **Alt は決定5で `from`/`to` の対象キーとしてだけでなく `from` の修飾子として
  指定することも禁止しているため、`[[keymap]]` の送信経路は Alt を解放する必要が
  そもそも生じない**（下記の理由参照）。
- `None`（消費のみ）の場合は何も送信しない。

`HeldModifiers` は現在 `ime.rs` 内の private struct で、VK_KANJI/VK_IME_ON/VK_IME_OFF
の3箇所から呼ばれてはいるが、**呼び出しごとに Alt の扱いが異なり、同じ前提を共有して
いない**（実コード確認）:

- `post_kanji_toggle_to_focused`（VK_KANJI 送信）: `held.push_release(...)` で
  Alt も含めて全解放する。
- `send_ime_mode_key`（VK_IME_ON/OFF 送信）: `let held_skip_alt = HeldModifiers
  { alt: false, ..held };` で **Alt は明示的に解放対象から除外**する。理由は
  同ファイルのコメントに明記されている:「ALT を解放すると ALT+TAB スイッチャーが
  確定してしまうため、ALT は解放しない」。
- 3箇所目も同様に `alt: false` を明示している。

つまり `HeldModifiers::read()` が返す3つの bool を「常に全部解放する」共通ヘルパーは
存在せず、**どの修飾を解放するかは呼び出し側が個別に判断している**。設定 GUI
（`awase-settings/src/main.rs` の `new_keymap_from_alt`）は現状 `from = "Alt+X"`
の作成を許容するが、決定5 でこれを禁止し（`combo.alt == true` の `from` を
`KeymapTable::new` で warn+skip する）、あわせて `new_keymap_from_alt`
チェックボックスも設定 GUI から削除する。理由: Alt 修飾を許すと
(a) 決定3 は `ctrl`/`shift` のみを解放する設計であるため、Alt は held のまま
`target_vk` が送信され、アプリには **Alt+target_vk** が届いてしまう
（ADR-037「なぜ修飾キーを解放するか」節が解決したはずの修飾キー残留問題の再発）、
(b) Alt を解放する設計に変えた場合は、`ime.rs` の該当箇所が「ALT を解放すると
ALT+TAB スイッチャーが確定してしまうため、ALT は解放しない」と明記している
理由がそのまま効き、加えて Alt 単独タップと区別がつかず Windows のシステム
メニュー（`SC_KEYMENU`）が起動して以後の入力がメニューナビゲーションに食われる
（`hook.rs` BUG-62 追補3 と同型の症状）——のいずれの経路を選んでも実害がある
ため。**Alt 修飾自体を `from` から禁止することで、この二択を実装時に迫られる
状況そのものを避ける**（＝上記の食い違いを踏まない設計）。

本 ADR ではこれを `output/` 配下（`output/held_modifiers.rs` 新設、または既存
`key_injector.rs` への統合、実装時に決定）へ `pub(crate)` として切り出すが、
**「共通化」は構造体の型とフィールドの切り出しに留め、「どの修飾を解放するか」は
引き続き各呼び出し元が明示的に指定する**（`push_release` を呼ぶ側が対象を選ぶ
形を維持し、デフォルトで全解放する挙動は持たせない）。「4箇所目の利用者ができた
ため切り出しが妥当」という判断軸自体は変わらないが、3箇所が同一の前提を共有して
いる、という誤った根拠では正当化しない。

送信するキーのマーカーは既存の `INJECTED_MARKER` を再利用する（decision 6）。
`HeldModifiers::push_release`/`push_restore`（`ime.rs`）の現行実装は
`IME_KANJI_MARKER`（`tsf/output.rs:27`、`0x4B45_594A`）をハードコードしている
（IME 漢字キー送信専用に書かれたコードのため）。`is_self_injected` は3種の
マーカーを等価に扱うため機能上の実害はないが、`observer/focus_observer.rs`
等にマーカー名で文脈を説明するコメントがあるため、決定3 で `pub(crate)` 切り出す
際は**マーカーを引数化**し、`[[keymap]]` からの呼び出しでは `INJECTED_MARKER` を
明示的に渡す（`ime.rs` の既存3箇所は `IME_KANJI_MARKER` のまま据え置き）。

**`find_match` は現状 `mods.win` を比較しない既知の穴があり、本 ADR の実装で
併せて塞ぐ。** `ParsedKeyCombo`（`src/config.rs`）は `ctrl`/`shift`/`alt`/`vk` の
4フィールドのみで `win` を持たず、`KeymapTable::find_match`
（`keymap.rs:81-86`）も同じ4項目しか比較しない。一方 `ModifierState`
（`src/types.rs`）には `win: bool` が存在する。そのため現状の実装がそのまま
配線されると、`from = "Ctrl+I"` は **Win+Ctrl+I にも誤ってマッチする**。
`find_match` の比較に `!mods.win`（`from` に Win 修飾を書けない前提、決定5で
Win系 VK を禁止しているのと対称に、Win 修飾との組み合わせもここで弾く）を追加する。

### 決定4: KeyUp の扱いは「物理 vk 単位の latch テーブル」で対応する。
**自動リピート判定とは意図的に分離する**（BUG-100 で踏んだ「兼用」の再発防止）

KeyDown で `[[keymap]]` にマッチした vk は、対応する物理 KeyUp が来るまで
**latch テーブル**（`Vec<VkCode>`/`HashSet<VkCode>`、実行時に同時に latch される
件数は小さいため線形探索で十分、キャパシティ上限は設けない）に記録する。
KeyUp 処理は「その vk が latch されている vk か」だけを見て `target_vk` を
再度参照しないため（下記1参照）、latch は vk の集合で足り、target vk を値として
保持する必要はない（決定3 が `target_vk` を KeyDown 側で Down+Up 同一バッチとして
即時完結させるため、KeyUp 側で `target_vk` を再送する必要がない）。KeyUp 処理
（決定2 のステップ1、`deliver_key_event` 冒頭）では:

1. 現在の vk が latch テーブルに存在すれば、その KeyUp を **無条件で consume**
   （modifier の再照合はしない — KeyDown 時点で判定済みの結果を信頼する。ユーザーが
   `I` を押しっぱなしのまま Ctrl だけ先に離した場合でも、latch が生きている限り
   `I` の KeyUp は横取りされたショートカットの一部として扱う）。
   entry を削除する。
2. 存在しなければ通常の KeyUp 処理（NICOLA エンジン等）に渡す。

latch は **フォーカス変更で消えない**。PowerToys の `activatedApp` 持続ロジック
（コンテキスト節・参考3）と同じ理由: `[[keymap]]` の `to` がフォーカスを奪う操作
（例: Alt+Tab 相当）にマッピングされていた場合、KeyDown 時点のフォーカス先と KeyUp
到達時点のフォーカス先が既にずれている。ここでフォーカス変更を理由に latch を破棄すると
`I` の物理 KeyUp がどこにも消費されず後続の入力に "浮いた KeyUp" として混入する。

#### 自動リピート抑制は latch テーブルの有無で判定する
（ただし BUG-100 とは異なり、5経路中4経路を塞ぎ、残る経路4は限定的な残存
リスクとして明示的に受容した上で採用する）

自動リピート判定は「その vk が既に latch テーブルに存在するか」で行う: KeyDown
到着時に対象 vk が既に latch テーブルにあれば repeat とみなし、`find_match` を
呼ばず黙って consume する（`target_vk` は再送しない）。**この判定は決定2の
ステップ1（`Nested`/`NonText` より前）で行う** — KeyUp 側の latch 解放と同じ
最優先ステップに置くことで、Down/Up どちらか一方だけが先に NonText 等へ素通り
してしまう非対称を避け、経路4（下記）の残存リスクを正味「1打鍵消失」だけに
閉じ込める。

一見 BUG-100（ADR-110 `[[key_remap]]`、上述「ADR-110/111 との関係」参照）と
同じ設計に見えるが、BUG-100 の本質的な誤りは「latch 存在で repeat を判定したこと」
自体ではなく、**latch が3経路（セッションロック中の KeyUp 消失・`HOOK_KEYS`
overflow・`VK_KANA` swallow 分岐）でクリアされずに stuck したまま放置され、
それが以後の新規押下まで無条件に「リピート中」と誤判定し続けた**ことにある
（`docs/known-bugs.md` BUG-100 の症状節）。本 ADR は下記「latch 漏れ対策」で
5経路のうち4経路（経路1・2・3・5）を確実に塞ぐが、**経路4（`HOOK_KEYS`
overflow）だけは塞ぎ切れず、latch が残ったまま次の物理 KeyDown を迎えうる**
（詳細は下記表）。「latch 存在 = repeat」という単純な判定は**この経路4のケースにも
一貫して適用する**（例外を設けない）: overflow で KeyUp を取りこぼした vk に
次の物理 KeyDown が来た場合、それは repeat 扱いとして黙って consume され
`target_vk` は再送されない（＝その1打鍵が消える）。続くその物理 KeyUp が latch を
解放するため、以後は正常に戻る。BUG-100 との決定的な違いは、**stuck が
「以後の新規押下すべてを恒久的に誤判定し続ける」ではなく「次の1打鍵だけが消える」
に限定される**点であり、latch が vk 単位・毎回の KeyUp で必ず解放される設計に
由来する。

**検討して却下した代替案**: 「repeat 判定にはエンジンスレッド側の
`hook::is_physical_key_down(vk)` を使う」という案も検討したが、これは採用しない。
`hook.rs::hook_callback` は `PHYSICAL_KEY_STATE[vk]` を `HOOK_KEYS.produce(event)`
より**前**に更新するため（hook.rs の該当箇所）、エンジンスレッドが `deliver_key_event`
でこの vk の KeyDown を処理する時点では、初回押下であっても
`is_physical_key_down(vk)` は既に true になっている。つまりこの関数は
「今処理中のイベント自身の到着」を「過去の押下」と区別できず、**全ての KeyDown が
無条件に repeat 扱いされ `find_match` が一度も呼ばれなくなる**（`[[keymap]]` が
配線しても症状として「設定しても何も起きない」＝未配線状態と見分けがつかない
まま完全に死ぬ）。latch テーブル自身は `deliver_key_event` 側でしか更新されない
ため、この時刻ズレの問題が起きない。

#### latch 漏れ対策（BUG-100 の3経路 + 本 ADR 固有の2経路、計5経路を個別に検討）

latch を作った後、対応する物理 KeyUp が `[[keymap]]` の KeyUp チェック（決定2
ステップ1）まで届かない経路が存在すると、latch は永久に残る
（ADR-110/BUG-100 が実際に踏んだ失敗の型）。本 ADR の配線（エンジンスレッド側の
latch）で当てはまる経路を洗い出す。

| # | 経路 | 本 ADR での扱い |
|---|---|---|
| 1 | `PumpContext::Nested` 早期return | 決定2 でこのチェックを Nested 早期returnより前に移動したため解消 |
| 2 | NonText フォーカスパススルー | 同上、NonText 早期returnより前に移動したため解消 |
| 3 | `FOCUS_APP_DISABLED`（`hook.rs:869`、既定 `mstsc.exe`）でフックが `CallNextHookEx` 直行 | エンジンスレッドに一切届かないため決定2の対処が効かない。`focus_tracking.rs:472` が disable 遷移時に `hook::clear_hook_latches_for_app_disable(transition)` を呼んでいる箇所と同じ地点（エンジンスレッド側）で、本 ADR の latch テーブルも `keymap_latch.release_all()`（新設）を呼ぶ。**`release_all()` は latch テーブルを空にするだけでよく、`target_vk` の KeyUp を注入する必要はない** — 決定3 が `target_vk` を KeyDown 側で Down+Up 同一バッチとして即時完結させる設計であるため、`target_vk` が「押されたまま」残ることは構造的に無く、ADR-110 の `release_all_latched_remap_targets()`（`target_vk` が押しっぱなしになる設計だったため KeyUp 注入が必要だった）とは前提が異なる。誤って KeyUp を注入すると、押されてもいない `target_vk` の KeyUp が前景アプリへ届くという別の副作用を生む |
| 4 | `HOOK_KEYS` overflow（`hook_channel::ProduceResult::Overflow`） | フックスレッド側の queue 溢れであり、物理 KeyUp イベント自体がエンジンスレッドに一切渡らずそのまま OS へパススルーされる（本 ADR の remap はフックスレッドで vk 書き換えをしないため、ADR-110 のように誤った vk が漏れる実害はない）。**この経路のみ、他の4経路と異なり latch を確実には解放できない。** 上記「自動リピート抑制」の規則をここにも一貫して適用する: latch が残ったまま次の物理 KeyDown が来ると repeat 扱いで黙って consume され（`target_vk` は再送しない＝その1打鍵が消える）、続く物理 KeyUp が latch を解放して以後は正常に戻る。実害は当該 vk の1打鍵消失に限定され、BUG-100 のような恒久固着（以後の新規押下すべてが誤判定され続ける）にはならない。頻度が低い（リングバッファの overflow 自体が既に稀）ため、タイムアウト機構は導入しない |
| 5 | Win+L セッションロック中の KeyUp 消失（`hook::reset_physical_key_state()`、`hook.rs:328`。呼び出し元は `message_handlers.rs:831` の `WTS_SESSION_UNLOCK`（**アンロック**）ハンドラと、`runtime/mod.rs:1633` 付近の panic reset 経路の両方——`WTS_SESSION_LOCK` 側は `invalidate_engine_context` のみで `reset_physical_key_state()` は呼ばない） | `reset_physical_key_state()` は hook.rs 側 `PHYSICAL_KEY_STATE`/`PHYSICAL_KEY_DOWN_AT_MS` のみ対象で、エンジンスレッド側 latch には触れない設計のまま据え置いてよい。ただし `reset_physical_key_state()` の**呼び出し元2箇所**（WTS_SESSION_UNLOCK ハンドラ・panic reset）双方で、本 ADR の latch テーブルの `keymap_latch.release_all()`（経路3と同じ、テーブルを空にするだけの関数を共用）も併せて呼ぶことを決定として明記する |

経路3・5 はいずれも「latch の存在をエンジンスレッド外の理由で強制終了させる」
唯一の正当な例外であり、ADR-037/決定4冒頭の「フォーカス変更では消さない」原則とは
矛盾しない（フォーカス変更ではなく、フック自体が無効化される／セッションが
非対話状態になるという、`[[keymap]]` の前提そのものが崩れるケースのみを対象とする）。

`[[key_remap]]`（ADR-110）が「フックスレッド」かつ「マルチスレッドから読める
`AtomicU16` 配列」を要求したのに対し、本 latch はエンジンスレッド単一実行内で完結する
ため **プレーンな `Vec`/`HashMap` で十分でアトミック型は不要**。根拠は
`crates/awase-windows/src/lib.rs::with_app`（同ファイル L204、`RUNTIME`
——`SingleThreadCell<Runtime>`、L195——の `try_borrow_mut()` を呼ぶ。
`SingleThreadCell` は内部的に `RefCell<Option<T>>` を保持し `unsafe impl Sync` で
`static` 配置を可能にしているだけで `Mutex` ではない、`single_thread_cell.rs`
参照）が再入時に `None` を返してログ警告するだけで待たない実装になっている点——
これは「複数スレッドから同時に呼ばれうる」ケースではなく「同一スレッド上で
ネストして呼ばれた」ケースへの防御であり、`Runtime` 全体がそもそもマルチスレッド
アクセスを想定していないことの傍証になる。latch を操作する `deliver_key_event` は
`with_app` の `f: FnOnce(&mut Runtime)` の内側で `app: &mut Runtime` を直接受け取る
（自分で `with_app` を再度呼ばない）ため、この再入防御にも抵触しない。
この非対称性（同じ「latched-target」教訓が ADR-110 では atomics 必須、本 ADR では
不要）は本質的な違い（フック vs エンジンスレッド）に由来することを、レビューで
明示的に確認してほしい。

### 決定5: `from` / `to` の禁止対象は ADR-110 の一般原則を継承しつつ、
awase 固有の実行時依存キーへ拡張する

ADR-110 決定4 の到達点を踏まえ、個別列挙ではなく一般原則を先に立てる:
**「`from`/`to` に指定できないのは、awase の他のロジックが静的な VK 一覧ではなく
`PHYSICAL_KEY_STATE` ベース（または実行時に決まる／`Runtime` の現在値に依存する）
held 判定・専用処理を持つキー全般」**とする。判明している具体例は以下（網羅リスト
ではなく、この原則に該当する既知のインスタンス集合）:

- 親指キー（`left_thumb_vk`/`right_thumb_vk`）
- IME 制御系 VK（`VK_KANJI`/`VK_DBE_*`/`VK_IME_ON`/`VK_IME_OFF` 等、
  `vk::ImeKeyKind::from_vk` が `Some` を返すもの）
- Alt系（`hook.rs::alt_key_held()` が参照する物理 held 判定の対象）、
  Win系（`hook.rs::win_key_held()` が参照する対象）を `from`/`to` の**対象キー**
  として指定すること
- **Alt を `from` の修飾子（`combo.alt == true`）として指定すること**。決定3で
  詳述する通り、Alt 修飾を許すと「修飾キー残留（Alt+target_vk が届く）」と
  「Alt+Tab スイッチャー誤爆／`SC_KEYMENU` 誤起動」のどちらかを踏む。設定 GUI の
  `new_keymap_from_alt` チェックボックスも削除する
- **Shift を `from` の主キー（修飾子側ではなく被修飾キー）に指定するケース**
  （例: `from = "LShift"`）。左Shift単独タップの半角英数トグル
  （`ime.rs` の `prepend_synthetic_shift_up` 経路、決定3参照）と
  `PHYSICAL_KEY_STATE` 追跡を壊す。Ctrl/Shift を `from` の**修飾子側**として使うのは
  許可する（`to` の単独ターゲットとしては禁止対象に含めない — ADR-037 の
  `HeldModifiers` は Ctrl/Shift/Alt の3つしか扱わないため、`to` に Ctrl/Shift 単体を
  指定するケースは実用上意味がない。積極的に禁止はしないが実装時のレビュー対象）
- **`muhenkan_solo_tap_dedicated_fn_key`（GJI 専用 Fn キー、config 手動指定または
  `config1.db` からの実行時自動検出）**。`gji_charset_autodetect.rs`
  `detect_dedicated_fn_key` / `message_handlers.rs`
  `handle_wm_gji_charset_fn_key_activated` が実行時に決める vk であり、
  `ImeKeyKind::from_vk` のような静的関数では捕まえられない。`to` にこの vk を
  送ると、決定6により `INJECTED_MARKER` 付きで送信され `hook.rs::is_self_injected`
  でフックを素通りして GJI に直接届き、awase が把握しないまま変換モードが変わる
  （conv belief 乖離）。静的な禁止リストでは塞げないため、`KeymapTable::new` では
  なく **実行時**（config reload・フォーカス変化時の `active_keymaps` 再構築時）に
  `Runtime` の現在値を見てチェックする必要がある
- `VK_CAPITAL`。ADR-111 の Scancode Map プリセット（Caps(英数)⇔Ctrl）と
  ドライバレベルで二重に介入しうる
- `engine_toggle_hotkey` / `special_keys.ime_toggle`（`app/mod.rs::parse_key_combos`
  で任意コンボを設定可能、`sync_ime_toggle_auto_detect` が MS-IME レジストリから
  自動追加する分もある）と同一のキーコンボを `[[keymap]]` の `from` に設定した場合、
  決定2 の順序により `[[keymap]]` が先に消費してエンジントグル自体が黙って死ぬ。
  これは「禁止 VK」では表現できない（コンボ単位の衝突のため）。実装時に
  `KeymapTable::filter_active` または `apply_config_update` 内で、エンジン制御系
  コンボと重複する `[[keymap]]` ルールを検出して warning を出す仕組みを追加する
  （「未解決の疑問」に追記）

バリデーションの置き場所は `KeymapTable::new`（`crates/awase-windows/src/keymap.rs`）
であり、`src/config.rs::validate()` ではない。root `awase` crate（`src/`）は
ADR-019 により OS 非依存を維持する制約があり、`Cargo.toml` の `[dependencies]`
にも `awase-windows`/`awase-vkmap` は含まれていない。`VkCode::from_name` /
`vk::ImeKeyKind::from_vk` / `parse_key_combo` はいずれも `crates/awase-windows/src/vk.rs`
側にしかなく、`src/config.rs` からは呼び出せない。`KeymapRule.from`/`to` は
`String` のまま `ValidatedConfig` へ素通しされる現行実装（`src/config.rs`）と
整合させ、静的に判定できる禁止 VK チェックは他のパース失敗ルールと同じく
`KeymapTable::new` 内で `log::warn!` して該当ルールを skip する（実行時にしか
判定できない `muhenkan_solo_tap_dedicated_fn_key` とエンジン制御系コンボの重複は
上述のとおり別の場所で扱う）。

加えて ADR-111 の調査（背景3）が明らかにした通り、IME モード切替系ショートカット
（Shift+CapsLock 等）は**フックより前の層で日本語 IME 自身が読んでいる**ため、
フック/エンジンスレッド側での抑制・リマップが構造的に効かないケースがある。
`[[keymap]]` の `from` に IME 制御系 VK を含む組み合わせ（例:
`Ctrl+VK_DBE_HIRAGANA`）を許可しても同じ壁にぶつかる可能性が高く、上記の
禁止対象に含める判断を補強する。

### 決定6: 自己注入マーカーは既存 `INJECTED_MARKER` を再利用する

新規マーカー種別は追加しない。`hook.rs::is_self_injected` が既に
`INJECTED_MARKER`/`TSF_MARKER`/`IME_KANJI_MARKER` の3種を認識し、該当すれば
`HOOK_KEYS.produce` に一切乗せず `CallNextHookEx` で素通しする。`[[keymap]]` の
送信（decision 3）が `INJECTED_MARKER` を使えば、送信した `target_vk` が
`find_match` に再度食われる心配（無限ループ）は構造的に発生しない。

### 決定7: 単一キー恒久リマップ（ADR-110/111、現状は撤回済みで存在しない）が
将来復活した場合のレイヤー順序を先に確定する

ADR-110 の `[[key_remap]]` は撤回済みだが（上述「ADR-110/111 との関係」）、ADR-111
自身が「将来アプリケーションごとに動的にキー割当てを変更する機能（本 ADR）が
設計された後、必要なら過去の設計・実装を参照しつつグローバル恒久リマップを
作り直す」ことを想定している。もし将来 `[[key_remap]]` 相当の機能が復活する場合、
`hook_callback` 内の Alt なりすましと同じ挿入点（vk 書き換え、フックスレッド
最初期）で動くはずであり、本 ADR の `[[keymap]]` 照合（エンジンスレッド）は
**必ずその後**に実行されることになる（フックスレッド → `HOOK_KEYS` →
エンジンスレッドの順で処理が流れるため、これは追加の配線なしで自動的に成立する）。
つまりその時点では `[[keymap]]` の `from` は「復活した `[[key_remap]]` 適用後の vk」
を基準に書く設定になる。PowerToys が採用している「単一キーリマップ → ショートカット」
という順序と一致し、awase の既存アーキテクチャ（フック→エンジンの一方向パイプライン）
はこれを設計変更なしで既に満たす。**この将来の復活とは独立に、本 ADR は今すぐ着手してよい**（依存は
片方向のみで、`[[keymap]]` 単独で `[[key_remap]]` 不在のまま完全に動作する。
逆に ADR-110/111 の教訓——特に BUG-100 の latch ライフサイクル漏れ——は
本 ADR の決定4に先取りして反映済みであり、将来 `[[key_remap]]` を作り直す際は
本 ADR の latch 設計・漏れ経路の洗い出し表を参照することを推奨する）。

### 決定8: `reload_config()` で `all_keymaps` / `active_keymaps` を再構築する

現状 `apply_config_update`（`runtime/mod.rs:1426`）は `all_keymaps` にも
`post_bypass_rules` にも触れておらず、設定変更後は awase 再起動が必要という
サイレントなギャップがある。本 ADR ではこのギャップを `[[keymap]]` について解消する:
`reload_config()` が `KeymapTable::new(&config.keymaps)` で `self.all_keymaps` を
差し替え、続けて現在のフォーカス先で `active_keymaps = all_keymaps.filter_active(..)`
を再計算する。

`[[post_bypass]]` 側の同種ギャップ（`post_bypass_rules` が reload 未対応）は本 ADR の
スコープ外として個別に `docs/known-bugs.md` へ記録する（「未解決の疑問」参照）。

**進行中の latch は `reload_config()` による `all_keymaps` 差し替えに対して安全である**。
決定4 の latch テーブルは「vk → target vk」という**物理キー単位**のキーであり、
ルールそのもの（テーブル内の位置やルール参照）を保持しない。そのため reload で
当該ルールが `all_keymaps` から消えても、進行中の KeyUp 待ちはそのまま latch
テーブルの記録どおりに回収される。これは BUG-100 が「テーブル参照方式では
config reload 中に latch が破綻する」（`docs/known-bugs.md` の
`any_latched_ctrl` への置き換え理由）と指摘した問題を、vk 単位キーという
latch の設計そのものによって最初から回避している。

**`filter_active` に渡す `process_name` が空文字列になりうる点への留意**:
`app_suppression.rs` のコメントが明記する通り、`get_process_name` はフォーカス
取得失敗時に空文字列を返しうる（`platform.focus.process_name()`）。空文字列は
`starts_with`/完全一致どちらの方式でも `app = None`（全アプリ対象）のルール以外
には一致しないため致命的ではないが、bootstrap 完了前や `reload_config()` が
フォーカス確定前に走った場合はこの空文字列パスを通りうることを実装時に留意する。

## 影響範囲

- `crates/awase-windows/src/runtime/message_handlers.rs`: `deliver_key_event` の
  冒頭に KeyUp latch 解放チェックを追加（決定2 ステップ1）、`NonText` パススルーの
  後・`[[post_bypass]]` 消費の前に `[[keymap]]` KeyDown 照合を追加（決定2 ステップ2）。
- `crates/awase-windows/src/runtime/mod.rs` / `Runtime`: latch テーブルのフィールド追加、
  `reload_config` 経路での `all_keymaps` 再構築（決定8）。
- `crates/awase-windows/src/runtime/focus_tracking.rs:472` 付近（`hook::
  clear_hook_latches_for_app_disable` 呼び出し地点、`FOCUS_APP_DISABLED` 遷移時）、
  `crates/awase-windows/src/runtime/message_handlers.rs:831`（`WTS_SESSION_UNLOCK`
  ハンドラ）、`crates/awase-windows/src/runtime/mod.rs:1633` 付近（panic reset）:
  いずれも `hook::reset_physical_key_state()` 系の呼び出し地点に、エンジンスレッド
  側 latch テーブルを空にする `keymap_latch.release_all()` 呼び出しを追加
  （決定4「latch 漏れ対策」経路3・5）。
- `crates/awase-windows/src/ime.rs`: `HeldModifiers` の `pub(crate)` 切り出し
  （決定3）。
- `crates/awase-windows/src/keymap.rs`: `KeymapTable::new` に禁止 VK チェック追加
  （決定5、`src/config.rs::validate()` ではない — ADR-019 のレイヤー制約のため）。
- `crates/awase-windows/tests/`: latch のライフサイクル（arm / KeyUp 回収 /
  `release_all` / repeat 判定）は**純粋関数として切り出し、Linux 上で実行できる
  ユニットテストを必須とする**（golden 追加の「推奨」ではなく必須。BUG-100
  （ADR-110 `[[key_remap]]`）が「実装 → PR #120 マージ → 事後レビューで latch
  ライフサイクル漏れ3件発覚 → PR #121 で修正 → 最終的に PR #123 で機能ごと
  撤回」という経緯を辿った前例を踏まえる）。加えて `deliver_key_event` の挿入順序
  （決定2）を golden/journal replay で固定する。`docs/known-bugs.md`
  （BUG-100、11971-12011行）と `docs/experiments.md` エントリ17（ADR-110/111 の
  経緯ログ）への参照をテストコメントまたは本 ADR 実装コミットに含める。

**実装時の注意（`crates/awase-windows/tests/architecture_guard.rs`）**: 同ファイルの
`deliver_key_event_nontext_early_return_excludes_ime_off_rescue_replay`
（`extract_fn_body` で `deliver_key_event` 本体を取り出し `FocusKind::NonText`
チェックの内容を文字列検査する）、および `key_events_reach_engine_only_via_
deliver_key_event`（`enqueue_reinject` 呼び出し箇所を「`deliver_key_event` と
文書化済みの pending-replay 例外」に限定するテキスト検査）は、決定2 が
`deliver_key_event` 冒頭に KeyUp latch 解放チェックを挿入することで想定と
食い違う可能性がある。実装時にこれらのテストを読み、必要なら期待値を更新する。

## 未解決の疑問

1. `[[post_bypass]]` の `reload_config` 未対応ギャップを `docs/known-bugs.md` に
   記録するかどうか、記録するなら本 ADR 実装時に同時に行うか別 PR にするか。
2. `KeymapRule.to` を PowerToys 同様の複数キー（修飾子込みショートカット）に拡張する
   ニーズがあるか。現時点では見送り、必要になったら別 ADR。
3. `KeymapTable::new(rules: &[KeymapRule])`（現行シグネチャ、`keymap.rs:29`）は
   `left_thumb_vk`/`right_thumb_vk` を知らない。これらは `config.general` 由来の
   実行時値であり、決定5 が言う「静的に判定できる禁止 VK」には本来含まれない。
   親指キーの禁止チェックを `KeymapTable::new` に置くなら、親指 vk を新たに
   引数として渡す必要がある（実装時にシグネチャを確定する）。
4. 決定5で挙げた「`engine_toggle_hotkey`/`special_keys.ime_toggle` と同一コンボの
   `[[keymap]]` ルールがエンジン制御系ホットキーを黙って無効化する」問題の検出を
   `KeymapTable::filter_active` に入れるか `apply_config_update` に入れるか。
   `sync_ime_toggle_auto_detect` によるレジストリ由来の自動追加分（実行時にしか
   確定しない）まで警告対象に含めるなら、静的な config バリデーション時点では
   検出しきれず、実行時チェックが必要になる可能性がある。
5. 決定5で挙げた `muhenkan_solo_tap_dedicated_fn_key`（実行時に確定する GJI 専用
   Fn キー）の禁止チェックをどこに実装するか。`KeymapTable::new` は config ロード
   時点で1回しか走らないため、`config1.db` からの自動検出タイミング（起動後）と
   ズレる可能性がある。`active_keymaps` の再フィルタ時（フォーカス変更・
   reload_config 時）に都度チェックするのが妥当か、実装時に確定する。
   **実装時に解決**: `KeymapTable::new` のシグネチャに `left_thumb_vk`/
   `right_thumb_vk` を追加（T1b）。dedicated fn key の禁止チェックは
   `Runtime::recompute_active_keymaps()`（フォーカス変更・reload 時に加え、
   `muhenkan_dedicated_fn_key_vk` フィールドを新設して2つの setter からも
   呼ぶことで、setter 呼び出し時点だけでなく「後から新しく衝突するルールが
   有効になった」場合も検出できるようにした）。
6. **実装レビューで判明した既知の限界（本 ADR のスコープでは対処しない）**:
   - `active_keymaps` はフォーカス変更検知後 `focus_debounce_ms`（既定 50ms、
     ADR-007）を挟んだ非同期チェーンの末尾（`enter_focus_scope`）でしか
     更新されない。この窓の間に届いたキーは直前のフォーカス先の
     `active_keymaps` に対して照合される。これは `[[keymap]]` 固有の問題では
     なく、`active_keymaps`/`[[post_bypass]]`/`disable_apps` を含む全ての
     アプリスコープ機能が同じ focus-tracking パイプラインを共有すること由来の
     既存の特性であり、本 ADR で新規に導入したものではない（ADR-114 実装
     レビュー指摘、Angle C）。
   - `KeymapLatch` は vk 単位でアプリスコープを持たない。物理キーを押したまま
     フォーカスを切り替え、切替先で別の `[[keymap]]` ルールに一致する修飾キーを
     追加で押す、という非常に稀な操作をすると、直前のフォーカス先から持ち
     越された押下が切替先のルールにマッチしてしまいうる（実装レビュー
     指摘、Angle A）。物理的に「このタイミングでの押下は直前のフォーカス先
     から持ち越されたものである」という情報を vk + 修飾キー状態だけからは
     区別できないという、キーリマップ全般に共通する構造的な限界であり、
     latch にフォーカス scope（`focus_epoch` 等）を持たせる拡張は将来の
     ニーズが確認されてから検討する。
   - 決定5/6 の衝突警告（`warn_on_engine_hotkey_collision`/
     `warn_if_vk_conflicts`）は `log::warn!` のみで、`StartupDiagnostics`
     （トレイバルーン通知・設定画面のステータス表示、`app/mod.rs`）を経由
     しない。同じファイル内の他の「設定値がおかしい/危険」警告
     （不明なキー名、サムキー/IME コンボの衝突等）はすべて
     `StartupDiagnostics` を経由してユーザーに見える形で表示されるため、
     この2つの警告だけが非対称にログのみになっている（実装レビュー指摘、
     Angle altitude）。`keymap.rs` は `app/mod.rs` の private な
     `StartupDiagnostics` に依存できないため、警告メッセージを
     `Vec<String>` として返す形に変更し呼び出し元（`bootstrap.rs`・
     `app/mod.rs::reload_config`）で `diag.warn(..)` へ転送する改修が必要。
     本 ADR のスコープでは見送り、視認性改善として別 PR で対応する。

## 関連 ADR

- [ADR-037](037-keymap-remap-design.md) — `[[keymap]]` のルール構文・修飾キー解放/復元
  シーケンスの原設計
- [ADR-110](110-simple-physical-key-remap.md)（**撤回済み**、PR #123） —
  グローバル単一キー恒久リマップ。latch ライフサイクル漏れ（BUG-100、
  `docs/known-bugs.md`）の初出であり、本 ADR 決定4の直接の反面教師
- [ADR-111](111-caps-eisu-ctrl-swap-preset.md)（採用・実装済み） — ADR-110 撤回の
  決定そのもの。背景6で「アプリケーションごとに動的にキー割当てを変更する機能」
  として本 ADR-114 の領域を明示的に予告している
- [ADR-048](048-sacrificial-warmup-chrome-coldstart.md) — SendInput 同一バッチ送信で
  中間状態を隠す原則の初出（Chrome cold-start 検出）
- [ADR-0005](0005-focus-classification.md) — フォーカス判定と `AppKind` 設計
  （PowerToys の `GetGUIThreadInfo` UWP フォールバックに相当する既存インフラ）

## 参考文献

- [PowerToys devdocs: Keyboard Manager module](https://github.com/microsoft/PowerToys/blob/main/doc/devdocs/modules/keyboardmanager/keyboardmanager.md)
- [PowerToys devdocs: Keyboard event handlers](https://github.com/microsoft/PowerToys/blob/main/doc/devdocs/modules/keyboardmanager/keyboardeventhandlers.md)
