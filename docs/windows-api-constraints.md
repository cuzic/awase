# Windows API 制約・落とし穴集

> 開発中に実機確認・バグ修正を繰り返して発見した Windows API の制約や挙動の罠を記録する。  
> ADR（設計判断の記録）とは別に、「なぜそうしてはいけないか」を残すことが目的。

---

## 1. IME 制御：アプリ別の API 対応状況

### 1-1. Chrome/Edge では VK_IME_ON/OFF が無効

**症状:** `ImmSetOpenStatus` や `VK_IME_ON (0x16)` / `VK_IME_OFF (0x1A)` を送っても Chrome の IME 状態が変わらない。

**原因:** `Chrome_WidgetWin_1` クラスは IMM-broken (ImmSetOpenStatus が届かない)。Chrome の TSF 実装は `WM_IME_CONTROL` も `VK_IME_ON/OFF` も処理しない。

**解法:** `VK_KANJI (0x19)` の `SendInput` のみ有効。  
ただし VK_KANJI はトグルキーのため、送信前に `shadow_ime_on != desired` を確認して二重トグルを防ぐ。

**実機確認:** 2026-05-22  
**実装:** `ime.rs::post_kanji_toggle_to_focused()`、`ime_controller.rs::KanjiToggleStrategy::apply`

---

### 1-2. Chrome：候補ウィンドウ表示中は bare VK_KANJI でトグルされない

**症状:** 変換候補ウィンドウが表示中に `VK_KANJI` を送ると IME がトグルされず、候補ウィンドウが閉じるだけになる。

**原因:** Chrome の KANJI 処理が候補ウィンドウ表示中とそれ以外で動作が異なる。

**解法:** `gji_candidate_visible() == true` のときは、先に `VK_RETURN (0x0D)` を注入して composition をコミットしてから `VK_KANJI` を送る。  
`VK_ESCAPE` は composition をキャンセルして入力テキストが消えるため **使用禁止**。

**実機確認:** 2026-05-24  
**実装:** `ime_controller.rs::KanjiToggleStrategy::apply`

---

### 1-3. LINE/Qt (ImmCross) では物理 KANJI キーを passthrough すると spurious VK_F3/F4 が発生する

**症状:** KANJI トグル後に「IME-ON Engine-OFF」状態に陥り、以降の `shadow_toggle` が逆転する。

**原因:** 物理 KANJI キー押下に対し OS が `VK_F3 Up → VK_F4 Down → VK_F4 Up` を生成する。LINE/Qt がこれを受け取ると `dwExtraInfo=0` (INJECTED マーカーなし) の spurious VK_F3/F4 を生成し、awase の shadow_toggle を反転させる。WH_KEYBOARD_LL では物理キーと spurious キーを構造的に区別できない。

**解法経緯:** 「抑止する」方向で 4 commit を試みた (d99e3f1 / 77ccf34 / e890a26 / f84c74b) がイタチごっこになった。最終解法は「ImmCross プロファイルでは KANJI 関連 VK (Down/Up 両方) を完全 Consume し LINE に渡さない」こと。awase が `ImmSetOpenStatus` でクロスプロセス制御する形で IME を完全所有する。

**実装:** `key_pipeline.rs::suppress_physical` 判定  
**関連コミット:** `08b8661` + `0e364ea`

---

### 1-4. LINE で Ctrl+無変換後に IME が ON に戻る（ImmCross async の race）

**症状:** LINE で Ctrl+無変換を押しても IME が OFF にならず、直後に ON に戻る。

**原因（連鎖）:**
1. Ctrl+無変換↓ → `write_set_open_request(false)` → 消費 → belief=false
2. ImmCross async（`SendMessageTimeoutW`）が非同期に発火
3. Ctrl↑ が INPUT_DEFER へ退避 → async 完了後の WM_DRAIN で replay
4. Ctrl↑ の Phase 2 (Active→Inactive) で decision に SetOpen(false) が **2回目** として乗る
5. Priority-3 は消費済みのため stale な `observer_poll=true` が belief を上書き
6. 20ms 後の TIMER_IME_REFRESH で engine が再アクティブ化

**解法:** boolean フラグ (`ctrl_bypass_hold`) による暫定修正の後、`InputBarrier::CtrlImeChord` 構造体に置換。「Ctrl+IME を 1 トランザクションとして扱い、barrier active 中は二次 SetOpen を filter する」モデルに。

**関連コミット:** `b7d4cdb`（初期修正）→ `4426a4a`（Barrier 化）

---

### 1-5. ImmCross フォールバックで VK_KANJI が逆トグルする

**症状:** GJI 候補ウィンドウ表示中に Ctrl+無変換を押すと、ImmCross の 162ms timeout 中に composition 確定で IME がすでに OFF になっている。shadow=true を信じてそのまま VK_KANJI を送ると OFF→ON に逆トグルし、500ms 後の drift correction まで IME-OFF が遅延する。

**原因:** async ImmCross は数百 ms かかることがある。その間に OS の実 IME 状態が変化しても shadow モデルは古い状態を保持している。shadow を信用してフォールバックを実行すると意図と逆の操作になる。

**解法:** フォールバック実行前に `read_ime_state_fast()` で実際の IME 状態を確認し、`actual == desired` ならフォールバックをスキップ (`AlreadyMatched` を返す)。

**実機確認:** 2026-05-30  
**関連コミット:** `3510a08`

---

## 2. TSF ベース IME（Google 日本語入力）の Cold Start

### 2-1. TSF cold start 中に PassThrough すると物理キーの順序が逆転する

**症状:** GJI probe 実行中 (`PROBE_ACTIVE=true`) に物理キーを PassThrough すると、TSF 注入バッチより先に WezTerm に届いて順序が逆転する。例: `にゅうりょく → ｆにうりょく`

**原因:** `PROBE_ACTIVE=true` 中の物理キーを `PassThrough` すると、`SendInput` の TSF バッチより先に WezTerm に届く。`ゅ` = 左親指 (VK_NONCONVERT) + F キーの組み合わせで、F が probe 中に WezTerm へ生の `ｆ` として届き NICOLA が `ゅ` を生成できない。

**解法:** `PROBE_ACTIVE` 中は `Consumed` を返して `PROBE_KEY_QUEUE` へ退避。probe 解除後に `WM_DRAIN_PROBE_QUEUE` で順序保証リプレイ。

**関連コミット:** `e5d2a30`

---

### 2-2. 物理 F2 (VK_DBE_HIRAGANA) を PassThrough しても TSF はウォームにならない

**症状:** 物理 F2 passthrough 後に F2 を mark_warm にすると「デプロイ → dエプロイ」のように先頭文字がリテラルになる。

**原因:** `VK_DBE_HIRAGANA (F2)` は composition context をリセットするのではなく、ひらがな入力モードに切り替えるキー。物理 F2 は OS 経由で **別メッセージポンプサイクル** で処理され、awase の SendInput バッチ内の warmup F2 と同一コンテキストにならない。

**解法:** F2 passthrough / reinject 後は `mark_composition_cold()` とし、次の NICOLA 出力バッチに warmup F2 を含める。

**実装:** `output.rs::composition_warm: Cell<bool>`、`send_romaji_batched` / `send_romaji_as_tsf`

---

### 2-3. TSF モードの GJI では `EVENT_OBJECT_IME_START/CHANGE/END` が発火しない

**症状:** `SetWinEventHook(EVENT_OBJECT_IME_START/CHANGE/END)` を設置しても GJI の composition イベントを検出できない。

**原因:** `EVENT_OBJECT_IME_SHOW/HIDE/CHANGE` (0x8027〜0x8029) は TSF モードの GJI では **発火しない**。GJI は TSF モードで独自クラス `GoogleJapaneseInputCandidateWindow` を使う。

**解法:** `GoogleJapaneseInputCandidateWindow` の `EVENT_OBJECT_SHOW/HIDE` (0x8002/0x8003) を監視して候補ウィンドウ表示状態を追跡する。visible=true のときは GJI I/O カウンタを 15ms ポーリングで変化を検出する。

---

### 2-4. 長期 idle 後の TSF セッションリセットで部分リテラルが発生する

**症状:** 13秒以上の入力なし後に Enter を押すと「このじしょう → kおのじしょう」のように最初の文字がリテラルになり、次の文字が IME に入る。

**原因:** GJI が idle 後に TSF セッションをリセットする。デフォルトの `eager_settle_ms=500ms` では GJI セッション再初期化（約 1000ms 必要）が完了しないまま次のローマ字送信が始まる。

**解法:** `idle_ms_at_last_cold` フィールドで直前の入力からの経過時間を記録。idle > 2000ms の場合は `eager_settle_ms=1500ms`、`probe_min_ms=300ms` に拡張する。

**関連コミット:** `6910881`

---

## 3. WH_KEYBOARD_LL フックの再入問題

### 3-1. SendMessageTimeoutW をフックコールバック内で呼ぶと with_app が再入する

**症状:** フックコールバック内で `SendMessageTimeoutW` を呼ぶと、内部メッセージポンプがフックコールバックを再入させ `in_with_app()` ガードが発火する。`hook.rs` に例外処理・グローバルが累積し続けた。

**原因:** `SendMessageTimeoutW` はメッセージを送りながら内部でポンプを回す。そのポンプが他のフック起因メッセージを処理してフックコールバックを再呼出しする。

**解法:** `with_app` **外** で `win32_async::spawn_local` を使い、`SendMessageTimeoutW` を fire-and-forget async 化する。hook 内の `HOOK_CONFIG` グローバルや `defer_during_with_app` などの例外処理を削除していった。

**派生症状コミット:** `446c402`, `0946318`, `b302586`, `1dd1b88`, `e19c5c6`（全て再入起因）  
**修正コミット:** `f24e024`, `003ffbc`, `779739f`, `57b42ea`, `38e74c2`, `ca519aa`

---

### 3-2. `block_on(sleep_ms)` を with_app 内で呼ぶと内部ループで再入する

**症状:** `block_on(sleep_ms)` 内部の `GetMessageA` ループが `with_app` を再入させて UB が発生する。

**原因:** `block_on` は内部でメッセージループを回す。`with_app` で App を借用中にもう一度 App を借用しようとして未定義動作になる。

**解法:** `with_app` を `UnsafeCell` ベースから `RefCell` ベースに変更して借用チェックで検出可能にする。`block_on(sleep_ms)` は `SetTimer` ベースの `TIMER_OUTPUT_GUARD` に置き換える。

**実装:** `single_thread_cell.rs`

---

## 4. 非同期 IME 制御の race condition

### 4-1. async apply 完了イベントが stale でも新しい意図を上書きする

**症状:** async ImmCross apply 中にユーザーが新しい IME 操作を行うと、古い apply の完了イベントが遅れて到着し新しい意図を上書きする。

**再現シナリオ:**
```
T1: apply true  → gen=10 で async 開始
T2: user intent → desired=false (gen=11)
T3: apply 完了  → gen=10 で ImeApplySucceeded 到着 → desired が true に戻る（誤り）
```

**解法:** apply 要求時に generation を `event_log.next_seq()` から取得して `pending` に格納。async 完了時に closure capture で渡す。reducer で `pending.generation` と event の generation を照合し、不一致なら無視する。

**実装:** `executor.rs`、`state/ime_model.rs::reduce`

---

### 4-2. probe queue の二重 drain で二重入力が発生する

**症状:** `wait_until_ready` と `wait_for_tsf_cold_settle` の両方が `post_drain_probe_queue()` を呼んでいたため、cold な composition 状態で drain が走って新たな probe が起動し「ん」が二重入力になる（例: `ぜんんぜ` / `でくはな`）。

**原因:** probe queue の drain 契機が複数箇所に存在し、cold な composition 状態で drain が走ると再帰的に probe を起動する。

**解法:** `wait_until_ready` / `wait_for_tsf_cold_settle` から `post_drain_probe_queue()` を削除し、`mark_composition_warm()` の直後にのみ移動する。

**関連コミット:** `22f1cd8`

---

## 5. 親指シフトと修飾キーの組み合わせバグ

### 5-1. Ctrl+無変換の救済機構が NICOLA 親指シフトに化ける

**症状:** Ctrl を早離しした場合に「Ctrl+無変換 IME-OFF」が NICOLA FSM の `PendingThumb`（左親指シフト）に化ける。

**原因（2 段階）:**
1. `hook.rs` で無変換↓ 自身が `CTRL_CONSUMED_SINCE_DOWN=true` にセットしていたため、Ctrl+無変換でも常に 50ms 救済ウィンドウが発動していた
2. 救済パスで「Ctrl↑ within 50ms」のとき `ctrl=false` に書き換えて IME-OFF を発火させると、NICOLA FSM が無変換を単独の親指キー `PendingThumb` として処理する

**解法:**
1. `CTRL_CONSUMED_SINCE_DOWN` の更新から左右親指キーを除外して救済を不発動にする
2. 「Ctrl↑ within 50ms」パスでは `ctrl=false` 発火をやめ、保留キーを swallow する

**関連コミット:** `ba5e4d3`

---

### 5-2. NICOLA 出力後に GetAsyncKeyState で modifier 状態が capture 時と乖離する

**症状:** NICOLA 出力後に Ctrl+I を操作すると IME-OFF コンボ（Ctrl+NONCONVERT）が誤発火する。

**原因:** `WM_DRAIN_OUTPUT_QUEUE` が `build_input_context` → `GetAsyncKeyState` を drain 時点で呼ぶため、キーイベント capture 時と異なる modifier 状態（Ctrl 押下）で評価される。

**解法:** `RawKeyEvent` に `modifier_snapshot` フィールドを追加し、フック時点で modifier を capture して以後はそのスナップショットを使う。drain タイミングで `GetAsyncKeyState` を呼ぶことをやめる。

---

## まとめ：発見したルール

| ルール | 理由 |
|---|---|
| Chrome に VK_IME_ON/OFF を送ってはいけない | Chrome TSF は処理しない。VK_KANJI のみ有効 |
| Chrome の候補ウィンドウ中は VK_RETURN → VK_KANJI の順で送る | bare VK_KANJI は候補を閉じるだけ |
| ImmCross アプリに物理 KANJI キーを passthrough してはいけない | spurious VK_F3/F4 が shadow_toggle を反転させる |
| フォールバック実行前に read_ime_state_fast() で実状態を確認する | async 中に実状態が変化し逆トグルになる |
| PROBE_ACTIVE 中は物理キーを passthrough してはいけない | TSF バッチより先に届いて順序が逆転する |
| 物理 F2 passthrough 後は composition を cold にリセットする | passthrough F2 と SendInput F2 は別ポンプサイクルで処理される |
| TSF GJI の composition 検出には EVENT_OBJECT_IME_* は使えない | TSF モードでは発火しない |
| SendMessageTimeoutW を with_app 内で呼んではいけない | 内部ポンプでフック再入が発生する |
| block_on(sleep_ms) を with_app 内で呼んではいけない | 内部 GetMessageA がフック再入して UB になる |
| async apply 完了は generation で照合してから反映する | stale な完了が新しい intent を上書きする |
| GetAsyncKeyState は drain タイミングで呼んではいけない | capture 時と modifier 状態が乖離する |
