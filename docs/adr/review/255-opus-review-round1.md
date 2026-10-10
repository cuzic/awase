---
id: ADR-255-companion-255-opus-review-round1
title: |-
  ADR-255（IME OFF のときだけ無変換/変換を Space にする）Opus敵対的レビュー round1
type: companion-doc
related_adr:
  - "ADR-255"
---

# ADR-255 敵対的レビュー round1(Opus)

(のちに対象ファイルは `255-keymap-ime-state-condition.md` から `255-ime-off-thumb-key-space.md` へ改名した。以下は当時の記録のまま。)

対象: `docs/adr/255-keymap-ime-state-condition.md`(ワークツリー keymap-ime、HEAD `9005e8a7`)。
コードの裏取りは同じワークツリー(origin/develop 由来)で行った。行番号はこの HEAD のもの。

結論の先出し: **未解決の疑問 3(二重配送)は実コード上は起きない**(下記 Q3)。一方 **疑問 1 の三値写像は ADR の書き方のままだと誤った `Closed` を作る経路が少なくとも 4 本あり**、さらに `[[keymap]]` の照合がエンジンの活性判定と**別の値**を見ることで「エンジンは活性(親指として使う)なのに `[[keymap]]` が先に消費する」矛盾が生じうる(Blocker)。設計の筋としては「三値を新設する」より「エンジン自身の非活性判定に乗る」方が安全で単純。

---

## Blocker

### B1. `ImeOpenView` を `resolve_open_at` から作ると、エンジンの活性判定と食い違い、IME ON で親指キーが Space に化ける

- エンジンの活性は `src/engine/engine.rs:269-283 compute_state` が `ctx.ime_on` で決め、`ctx.ime_on` の出どころは `ImeStateHub::effective_open()`(`crates/awase-windows/src/state/platform_state.rs:951-975`)。
- `effective_open_at` は `shadow_model.resolve_open_at(...)` の値に **`IntentStore::resolve_effective_open` の上書き**を重ねている(同 :971-980、「[intent-store] effective_open override 開始」)。つまり **`resolve_open_at().value` と エンジンが使う `effective_open()` は一致しない場合がある**。
- ADR 決定3 は `resolve_open_at` の `DecidedBy` から三値を作るとしている。失敗シナリオ:
  1. フォーカス先で観測が `DeriveHigh(ImmGetOpenStatus)=false`(Closed)。
  2. `IntentStore` に同じ hwnd への明示 ON 意図(TTL 内)が残っており、`effective_open()` は `true` → エンジン活性、無変換は親指キー。
  3. `deliver_key_event`(`runtime/message_handlers.rs:217-221`)は `process_key_event` より**前**に `consume_keymap_match` を呼ぶ → `Closed` 判定で無変換を消費し Space を送る。親指シフトが Space になる。
- ADR-114 決定5 が守ろうとした「同じキーを二つの機構が別々の状態で判断する」問題そのもの。決定4 の「ルールが `Closed` のときしか当たらないので二重管理は起きない」は、**`Closed` の定義がエンジンの非活性と同じ値から導かれる**ときにしか成り立たない。
- 修正案: `ime = "off"` の照合条件を「**この打鍵で `process_key_event` が使うのと同じ `build_ctx()` に対し `engine.compute_state(&ctx) == Inactive(ImeOff)`**」を必要条件にする。そのうえで下記 M1 の観測根拠を AND する。`Inactive(UserDisabled)`/`NotRomajiInput` を含めるかは別判断(S3)。

### B2. `Closed` の誤判定時に、`consume_keymap_match` が未確定文字列を破棄する

- `consume_keymap_match`(`message_handlers.rs:294-298`)は、送信前に `ime_composition_active_now() || is_composition_warm_in_tsf()` なら `cancel_composition` を呼ぶ(ADR-114 実装レビュー MA-2 で「意図的な仕様」)。
- `ime = "off"` ルールで composition が存在する = **「IME は閉じている」という判定と矛盾する直接証拠**。にもかかわらず現行コードのままだと、誤った `Closed`(B1・M1 の経路)のとき、ユーザーの未確定文字列を `CPS_CANCEL` で捨ててから Space を入れる。
- 具体シナリオ: Chrome + GJI で無変換単独タップ(IME OFF の明示意図、`ExplicitIntent` で desired=false)が GJI に効かなかった(BUG-142 型の shadow toggle 空振り)→ 実 IME は ON、エンジンは非活性なので打鍵は生のまま GJI に入り composition ができる → ユーザーが無変換を押す → `Closed` 判定で composition 破棄 + Space。
- 修正案: `ime` 付きルールは「composition あり」を `Unknown` 扱いにして**照合しない**(キャンセル処理に到達させない)。単体テストで固定する。

---

## Must

### M1(疑問1への回答). `DecidedBy` の variant 単位の写像では `Closed` の根拠を選別できない

`crates/awase-windows-core/src/state/ime_model.rs:462-496 resolve_open_at` と `observation_store.rs` を読んだ結果:

1. **ADR の語彙が実コードと合っていない**。`DecidedBy.base`(`BaseDecision`, ime_model.rs:90-111)は `ExplicitIntent / KeyEffectPrediction / DeriveHigh(src) / DeriveMedium{first,second} / MostRecentTrusted(src) / DesiredFallback` の 6 つ。`HeuristicDefault` は `DecidedBy` ではなく **`ObservationSource`**(ime_event.rs:141)。「`HeuristicDefault`(Low)のときは Unknown」は variant 写像では表現できない。
2. **`DeriveMedium{first, second: None}` は単独の BeliefOnly ソースで成立する**。`HwndCache`・`ConvOpenInference` は Medium(evidence.rs:138,142)なので、`derive_any` が無競合なら単独で `DeriveMedium` になる。`HwndCache` は BUG-56/BUG-107 型の汚染が実在するキャッシュ、`ConvOpenInference` は BUG-26 で「同じ conv 値で実 IME 状態が正反対」と確定した推測。どちらも `Closed` の根拠にしてはならない。
3. **`MostRecentTrusted(src)` は Low を除外しない**。`most_recent_trusted`(observation_store.rs:702-713)は Medium 以上を**優先**するだけで、Medium 以上が無ければ `HeuristicDefault`(`assume_closed_for_new_thread` が新スレッドで「閉」を Low で記録、platform_state.rs:1621-1645)や `FocusProbe`(Low)を返す。新規ウィンドウ(新しいタブ・新しいダイアログ)では「閉」の推測だけで `MostRecentTrusted(HeuristicDefault)=false` になる。
4. **`ExplicitIntent` は観測ではなく awase の意図**。ADR は「明示のユーザー意図」を信頼できる根拠に含めているが、`desired_open` は actuation が空振りしても変わらない(B2 のシナリオ)。TSF ネイティブで IME が状態を偽る/遅れる場面で最も外れやすいのがこれ。`IntentStore` の上書き(B1)も同類。
5. **`KeyEffectPrediction`** は ADR が「追加の根拠を足さない」と書く一方、`resolve_open_at` では観測より**優先**される base になる(ime_model.rs:466)。三値でどう扱うかを明記する必要がある。
6. `guard_override` は ON 方向にしか値を変えない(`ForceGuardSet::resolve`)ので、`off` ルールに対しては安全側(Open)。ただし `on` ルールで「guard が強制した Open」を確実な Open とみなすかは要定義。

修正案: 既存の **`ObservationStore::derive_actuating`**(observation_store.rs:817、Actuating プールだけで High 単独/Medium 無競合多数決)を使う。`Closed` = 「B1 のエンジン非活性(ImeOff)」AND「`derive_actuating(now) == Some(false)`」AND「composition なし(B2)」。新しい写像表を `state/` に起こすより、ADR-087 で既に actuation の根拠として審査済みの関数を再利用する方が、INV-39(BeliefOnly が Actuating を上書きしない)を構造的に継承できる。`ObservationAuthority::Actuating` は `ImmGetOpenStatus / ImmCrossProbe / ObserverPoll / Gji / Tsf`(ime_event.rs:175-187)。

### M2. 厳格にした結果「TSF ネイティブでは一度も当たらない」可能性を先に測る

- `derive_*` は `OBSERVATION_FRESH_WINDOW_MS` 内の観測しか使わない(observation_store.rs:832-835)。IME OFF のまま放置した後や、ポーリングの観測が少ない TsfNative/Imm32Unavailable では、actuating な新しい観測が無く `Unknown` → 素通しになりうる。報告者のアプリは ADR に書かれていない。
- 実装前の CI スパイクで、Notepad(Win32)・Chrome(TsfNative)・Windows Terminal 等の IME OFF 状態で無変換を押した瞬間の `resolve_open_at` の `DecidedBy` と `derive_actuating` の結果を `[belief-flip]` 相当のログで数える。「安全だが効かない」機能を出荷しないため。
- ADR 本文に、`Unknown` のとき**黙って素通し**することをユーザーに見える形(設定画面の注記、または debug ログ)で残すこと。報告者が「効かない」と再報告したとき切り分けできない。

### M3(疑問2への回答). 決定4 の「二重管理は起きない」は不正確。フックの親指ラッチは `[[keymap]]` と無関係に立つ

- `hook.rs:1808-1820` は、エンジンにも `[[keymap]]` にも依存せず、`vk == config.left_thumb_vk` の物理 Down で `HOOK_STATE.left_thumb_down_scan` を武装し、その時刻を以降の**全打鍵**の `RawKeyEvent` に `left_thumb_down_snapshot` として埋め込む(:1855, ADR-129)。これが ADR-114 決定5 の言う「PHYSICAL_KEY_STATE ベースの held 判定」の実体。
- `[[keymap]]` が無変換 Down を消費しても、このラッチは Up まで立ったまま。押している間に IME が ON に変わると(フォーカス移動先が ON、`keys.ime_on` の別キー、アプリ側の自動 ON)、その間の文字キーは「親指が押されている」スナップショットを持ってエンジン(活性化済み)へ届き、**エンジンが Down を一度も見ていない親指キーで同時打鍵判定**される。
- ADR-114 決定5 の根拠を壊さないと言うには、(a) この窓が実害を出さないことの単体テスト(エンジン側が FSM で Down を見ていない親指スナップショットを無視するか)、または (b) `[[keymap]]` が親指キーを消費したときはその打鍵のスナップショットを無効にする手当、のどちらかが要る。「Closed のときしか当たらない」は Down 時点の話で、hold 中の状態遷移を見ていない。

### M4. ラッチが古く残ると、IME ON で親指キーの 1 打鍵が黙って消える(親指キー解禁で新たに届く既存の穴)

- `deliver_key_event` ステップ1(message_handlers.rs:165-183)は latch 済み vk の Down を「自動リピート」として**無条件に** consume し、HOOK_KEYS overflow で latch が stale の場合も例外を設けない(コメントに明記)。
- overflow ラッチ中(hook.rs:1735)や Overflow(:1913)ではフックが Up を `CallNextHookEx` で OS に直接流し、`deliver_key_event` に届かない → `[[keymap]]` latch が残る。`release_all` は app-disable 遷移・watchdog 再インストール・アンロック・panic reset でしか呼ばれない(focus_tracking.rs:535、mod.rs:2086/2397、message_handlers.rs:1056)。**通常のフォーカス変更では解放されない**。
- 従来は from に親指キーを置けなかったのでこの穴は親指シフトに届かなかった。解禁すると、stale latch の次の無変換 Down は **IME ON・エンジン活性でも** 「リピート」として消費され、Up も消費される → 親指シフト 1 回が消える。`ime` 付きルールについては「latch の Down は `was_down`(event.was_down、hook が物理状態から付ける)が true のときだけリピート扱い、false なら latch を捨てて再照合」にするのが最小修正。回帰テスト必須(物理キー押下ラッチ・ファミリー)。

### M5. 他ツールが注入する無変換が Space に化ける(alt-ime-ahk 等)

- `deliver_key_event`/`consume_keymap_match` は `event.injected` を見ない。hook は foreign-injected でも `HOOK_KEYS.produce` する(hook.rs:1905)。
- 日本語ユーザーに広く使われる alt-ime-ahk 系は、左 Alt 単独で `{vk1D}`(無変換)を**注入**して IME OFF にする。IME が既に OFF のときにこの注入が来ると、`ime = "off"` ルールにより Space が入る(左 Alt を押すたびに空白)。MS-IME 自身の機能的なキー注入(BUG-14)も同じ経路に乗る。
- 修正案: `ime` 付きルール(少なくとも親指キーを from にしたルール)は `!event.injected` のときだけ照合する。CI に注入無変換の負の対照を足す。

### M6. 決定6 の警告対象が実在の主要ケースを漏らしている/「既存の衝突警告に載せる」の前提が誤り

1. **役割由来の開閉**が抜けている。エンジン非活性時の無変換/変換には `Engine::thumb_open_role_action`(src/engine/engine.rs:947-962)と `thumb_forced_open_actions`(runtime/mod.rs、IME 設定・学習表由来)がある。GJI の MS-IME/ATOK 系プリセットや学習表で「直接入力の変換 = IME ON」の人は、`ime="off"` の変換ルールで **IME を開く手段そのものを失う**(半角/全角しか残らない)。`keys.ime_*` と `muhenkan_solo_tap_dedicated_fn_key` だけでなく、役割由来(`thumb_forced_open_actions` と学習表の予測)も衝突判定に入れること。これは「警告のみ」ではなく**既定で拒否(skip+警告)**にすべき候補。
2. ADR は「`recompute_active_keymaps` の既存の衝突警告(ADR-114 未解決の疑問 4・5)に載せる」と書くが、実コードで `warn_if_vk_conflicts` を呼ぶのは dedicated fn key の 2 箇所だけ(runtime/mod.rs:1691、2318)。`keys.ime_*`/`engine_toggle_hotkey` との衝突警告(疑問4)は**実装されていない**。しかも recompute 側は `KeymapConflictLevel::Debug` で、通常のログには出ない。「既存に載せる」ではなく新規実装として書き直すこと。
3. `ime_off_rescue`/panic 検出: `handle_hook_key_event`(app/mod.rs:626-637)は `deliver_key_event` より**前**に `panic_detect::record_ime_keydown` を呼ぶ。無変換/変換が IME OFF/ON ショートカットと一致する構成で、`[[keymap]]` が Space に変えた無変換・変換を交互に速く打つと panic reset(IME OFF→ON、`release_all` を含む)が発火しうる。衝突時は panic 検出から除くか、少なくとも ADR に記載。

### M7. ADR の事実関係の訂正

1. 「awase は IME OFF の無変換/変換を握らず GJI へ素通しする」は不正確。フックは Accepted 時に常に `LRESULT(1)` で握りつぶし(hook.rs:1909)、エンジン判断後に `ReinjectKey` で**再注入**する(message_handlers.rs:251、executor.rs:336)。GJI が受け取るのは awase が SendInput した(マーカー付き)打鍵。CI で「awase あり/なしで同じ」なので結論は変わらないが、`physical_disposition.rs` の Allow は「再注入する」の意味であって「フックで素通し」ではない。
2. 「CUSTOM 表は読まれ、直接入力の行は効く」の根拠 `ctl-loaded` は `--seq=1A,1D,1C,1D` で、**無変換と変換のどちらで open が 0→1 になったか**を ADR に書くこと。変換だけで開いたなら「`Muhenkan` というキー名が GJI に通じていない」可能性が残り、InsertSpace 系の全滅もそれで説明できてしまう。baseline(`DirectInput,ON,IMEOn` のみ)で 1D/1C の open 遷移が無いことも併記すべき(表には空白の有無しか無い)。
3. 「対照が成立していない」の書き方自体は正しいが、もう一つの未分離の仮説を足すべき: **GJI の直接入力状態は IME がキーを消費しない状態で、割り当て可能なコマンドが IMEOn・入力モード切替・再変換などに限られ、InsertSpace 系はそもそも割り当ての対象外**(設定ダイアログの直接入力行で選べるかを確認すれば 1 回で切れる。未確認)。Precomposition 対照の失敗は別の原因(注入した 0x1C/0x1D をキー名 `Henkan`/`Muhenkan` に解決するか、+1500ms の `tail` 観測)でありうるので、正の対照(Space 0x20 を Precomposition の InsertSpace 既定動作で入れる)を先に取ること。
4. 各セル n=1 は表の直下に明記しているが、要約(frontmatter `summary`)にも n=1 と書くこと。要約だけ読んだ人が「GJI では不可と確定」と読む。

---

## Should

### S1(疑問3への回答). 二重配送は起きない。ただし ADR の問いの前提を直す

- フックは物理配送の決定をしない。Accepted 時は常に握りつぶし(hook.rs:1905-1916)、OS に届くのはメインスレッドの再注入だけ。`PhysicalKeyDisposition::plan`(physical_disposition.rs:48-70 の無変換/変換=Allow)は `process_key_event` 内のエンジン判断の後で使われるので、`consume_keymap_match` が `Some(Consumed)` を返した打鍵(message_handlers.rs:217-221)には到達しない。KeyUp・リピートも latch 分岐(:165-183)で `Consumed`。**元の無変換と Space の二重はこの経路では起きない**。
- 例外は awase がフックで生のまま流す経路だけ: overflow(hook.rs:1735, 1913)、`focus_app_disabled`(:1613)、ゾンビフック(:1540 等)。これらは `[[keymap]]` 自体が効かない(従来どおり生の無変換が GJI に届く)ので二重ではないが、M4 の stale latch の原因になる。
- ADR の疑問3 は「同期か非同期か」を問うているが、答えは「フックは常に suppress、配送はメインスレッドの再注入のみ」。この事実を本文に書いて疑問を閉じること。
- 遅延経路: `OUTPUT_GATE`/`INPUT_DEFER`(app/mod.rs:641-654)で退避された Down は後で `deliver_key_event` に流れ、`ime` 条件は**再生時点**の状態で評価される。押下時と再生時で IME 状態が変わるのは actuation 中(まさに IME が切り替わっている最中)なので、「再生時に評価」で良いのかを決定2 に一行書く。

### S2. `find_match` の優先順位と `Unknown` の意味を仕様化する

- 現行 `find_match`(keymap.rs:190-201)は先頭一致。`ime` 条件付きと無条件の同じ `from` が並ぶと、`Unknown` のとき「条件付きを飛ばして後続の無条件に当てる」のか「その from は一切当てない」のかで挙動が変わる。親指キー以外では両方書けるので、ADR に規則と単体テストを書くこと(推奨: 条件不成立のルールは飛ばして走査を続ける。ただし同じ from に `on`/`off`/無条件が混在したら警告)。

### S3. 報告者の「IME OFF」が 半角英数(IME 開・英数モード)である可能性

- 多くのユーザーは「半角英数」を IME OFF と呼ぶ。その状態は belief 上 open=true(エンジンは `Inactive(NotRomajiInput)`)なので、`ime = "off"` は当たらず報告は直らない。awase の左 Shift 単独タップ半角英数トグル(ADR-245/BUG-193)利用者も同じ状態。
- 報告者に「タスクバーの表示が A か あ(半角英数)か」を確認するまで、条件を `open` だけで定義するのは早い。エンジンの `InactiveReason` を条件にする代案(下記 A1)だと両方を同じ機構で選べる。

### S4. `ime = "on"` は消費者がいないので今回は入れない

- 動機は IME OFF だけで、`on` の利用例も検証計画もない(検証計画 (b) は「off ルールが IME ON で当たらない」の確認で、`on` ルールの検証ではない)。`on` は親指キーを解禁しないので、既存の `[[keymap]]` の上で何が新しく可能になるかも示されていない。保守対象を増やすだけなので `"off"` のみで始め、必要になってから足す(列挙型にしておけば後方互換に足せる)。

### S5. 期待される「空白入力」との違いを明記する

- `[[keymap]]` の `to` は Down で Down+Up を即時完結し、リピートは latch が黙って消費する(keymap_latch.rs の doc、message_handlers.rs:171-177)。**無変換を押し続けても Space は 1 個しか出ない**。本物の Space キーやGJI の InsertSpace(リピートする)とは違う。報告者の期待と一致するか確認し、ADR と設定画面に書く。

### S6. Alt なりすまし構成との相互作用

- `left/right_alt_impersonates_thumb_key` のとき、フックは Alt を親指 vk(既定の無変換)に書き換える(hook.rs:1762)。条件は `cached_engine_enabled`(`EngineStateChanged` 由来、executor.rs:623)。キャッシュがエンジンの非活性化より遅れる窓で、IME OFF なのに Alt が無変換に書き換わり、`ime="off"` ルールで Alt が Space になりうる。`from` が親指キーのルールでは Alt なりすまし由来の打鍵を除外するか、窓が無いことを確かめる。

### S7. 検証計画の不足

1. M2 の `DecidedBy`/`derive_actuating` 分布の計測を、実装より**前**の段に置く。
2. 負の対照を足す: (i) 注入された 0x1D(M5)、(ii) composition 中の無変換(B2)、(iii) IME ON での親指シフト**文字**(「空白が入らない」だけでなく、無変換+文字キーが親指シフト文字になること)、(iv) `keys.ime_on = 変換` 構成で衝突時に skip されること(M6)。
3. CI の awase ありの構成では、スパイクで入れた GJI の CUSTOM 行(InsertSpace 等)を**入れない**こと。GJI 側も動くと awase の効果と区別できない。
4. 決定5 (c)「Down 後に IME が切り替わっても Up が回収される」は、何で IME を切り替えるのか(0x16 の注入?フォーカス移動?)を書く。0x16 は `ImeKeyKind` で別経路に乗るので、そこで M3 のスナップショット問題が出るかも同時に見る。
5. Linux 単体: 決定3 の写像表ではなく、「`compute_state`×`derive_actuating`×composition×injected×`was_down`」の判定関数を純粋関数として固定する(`state/` に置き、Windows ゲートの外に置くこと。`runtime/` 配下の `#[cfg(test)]` は Linux で存在しない)。

### S8. 設定名

- `[[keymap]]` の既存キーは `app`/`from`/`to` の名詞1語。`ime` は `[keys] ime_on` 等と字面が近く、「IME を操作する設定」と誤読されやすい。`when_ime = "off"` を推す(将来 `when_app_kind` 等を足す余地も読み取れる)。値は `"off"` のみ(S4)。

### S9. リリースの分け方

- 決定7 の「ADR-230(Scancode Map)と同時リリースを避ける」は妥当。加えて、本機能は親指キーを `[[keymap]]` に初めて開放する変更なので、**オプトイン(ルールを書いた人だけ)であっても**、M3/M4 の回帰テストが通るまでは v2 の次リリースに入れず develop で実機確認を待つこと、を ADR に書く。v1 ラインへの backport はしない(新機能)と明記。

---

## 代案

### A1(推奨). エンジン側で「非活性時の親指キー単独タップの出力」を選べるようにする

- エンジンは既に非活性時の bare 親指キーを `thumb_open_role_action`(engine.rs:947)で扱っており、そこには `compute_active`・`is_japanese_ime`・`is_bare_thumb`(物理・無修飾)・`sync_direction` の除外が揃っている。ここに「非活性理由が `ImeOff`(と任意で `NotRomajiInput`)のとき、開閉の役割が無ければ Space を出す」を足すと:
  - B1 が構造的に消える(判定とエンジン活性が同じ値)。
  - M5 の注入除外は `is_bare_thumb` が既にやっている(`event.injected` を物理扱いしない)。
  - M6-1 の役割との衝突は同じ関数の中で優先順位として書ける(役割があれば役割が勝つ)。
  - Down/Up は `KeyLifecycle` の `UpDuty::Consume` に乗り、`[[keymap]]` latch の stale 問題(M4)を通らない。
  - M3 もエンジンが Down を見ているので「見ていない親指」問題にならない。
- 欠点: 汎用性が無い(無変換/変換→Space 専用)、設定項目が 1 つ増える(`thumb_key_when_ime_off = "space" | "pass"` など)。ただし動機の報告は正にこの 1 ケースで、`[[keymap]]` の汎用条件化は S4 と同じく消費者がいない一般化になっている。B2 の composition ガードは A1 でも必要。

### A2. `[[keymap]]` 案を残すなら最小形

- `when_ime = "off"` のみ、照合条件は B1+M1+B2+M5 の AND、親指キーの解禁は `when_ime` 付きのみ、M4 の `was_down` 修正込み。この形でも疑問1・2 は「エンジン非活性に乗る」ことで閉じる。

### A3. GJI の CUSTOM 表を awase が生成する案(ADR-231 側)

- 本 ADR の CI では InsertSpace 系が直接入力でも Precomposition でも効いていないので、現時点の証拠では**採れない**。M7-3 の確認(直接入力の行に InsertSpace を割り当てられるか)で否定されれば検討から外せる。MS-IME 利用者には効かない点でも A1 に劣る。

---

## 確認に使ったコマンド(HEAD `9005e8a7`)

- `grep -n "fn effective_open\|fn resolve_open_at" -r crates/awase-windows-core/src crates/awase-windows/src`
- `sed -n 930,1010p crates/awase-windows/src/state/platform_state.rs`(IntentStore の上書き)
- `sed -n 702,713p` / `grep -n "fn derive_actuating"` on `crates/awase-windows-core/src/state/observation_store.rs`
- `sed -n 1425,1917p crates/awase-windows/src/hook.rs`(常に `LRESULT(1)`、親指ラッチ、Alt なりすまし)
- `sed -n 148,309p crates/awase-windows/src/runtime/message_handlers.rs`
- `git show origin/ci/e2e-direct-space:.github/workflows/e2e-ime.yml | sed -n 450,482p`(sc-direct-space-* の構成)
- CI の run 38058313464・38059338837 の成果物は見ていない(未確認。M7-2 の「どちらのキーで開いたか」は成果物のスパイクログの KEY 行で確かめられる)。
