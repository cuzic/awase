# awase 既知の不具合

> 最終更新: 2026-07-09

---

## 実装アーキテクチャ概要（2026-06-02 時点）

旧実装（固定 sleep / IMM32 ポーリング）は以下に全面置き換え済み。

| 要素 | 役割 |
|---|---|
| `TsfReadinessProbe` | GJI I/O 静止を監視し「composition 受け付け可能か」を判定 |
| `TsfProbeCoro` | probe コルーチン（`probe_fsm.rs`）。`ChromeProbe` → `SacrificialWarmupCoro`（GJI 有効時）または `Transmit + LiteralDetect`（GJI 無効時）を StepCoro で直線記述 |
| `ColdWarmupSequence` | WezTerm TSF cold-start の F2 送信・probe 起動シーケンス |
| `LiteralDetector` | 送信後に GJI SHOW / プロセス I/O 変化を監視して composition 成否を判定 |
| `ColdReason` | cold になった理由。`eager_settle_ms()` / `probe_min_ms()` で探索予算を決定 |

GJI (Google Japanese Input / 候補ウィンドウ) の I/O を `GjiMonitor`（`tsf/gji_monitor.rs`）バックグラウンドスレッドで監視し、
`TSF_OBS.gji_last_io_ms` に記録する。probe はこの timestamp を参照して GJI が settled かどうか判断する。

---

## BUG-01: TSF cold-start — probe バジェット超過で1文字目がリテラルになる (WezTerm)

**症状:** WezTerm でひらがな入力の最初の1文字がリテラル ASCII になる。
例: `かんきょうへんすう` → `kあんきょうへんすう`

**原因:** WezTerm は TSF native app。F2 (VK_DBE_HIRAGANA) 受信後、TSF composition context の
初期化に実測 ~300–936ms かかることがある。awase の romaji SendInput がこの初期化完了前に届くと
1文字目が IME を通らずリテラルになる。

**現在の対策:** `ColdWarmupSequence` + `TsfProbeCoro`（`tsf/probe_fsm.rs`）によるノンブロッキング probe。
`eager_settle_ms`（最大バジェット）を `ColdReason` × `long_idle` の組み合わせで決定する:

| ColdReason | short idle | long idle (>10s) |
|---|---|---|
| `FocusChange` / `SetOpenTrue` / `NativeF2Consumed` | 1500ms | 2000ms |
| `PassthroughConfirmKey` / `ReinjectConfirmKey` | 500ms | 1500ms |
| その他 (`SessionExpired`, `SymbolVkSent` 等) | 500ms | 500ms |

probe 中に GJI が settled になれば早期解放される（タイムアウト待ちにならない）。

**残存リスク:**
- バジェット値は実測ベースの経験値。非常に高負荷な環境では超過する可能性がある。
- `NameChangeWait` での OBJ_NAMECHANGE タイムアウトが長期 idle 時に延長されるが、
  TSF 初期化が残余バジェット全体を超えた場合はリテラル出力が発生しうる。

**関連ファイル:** `tsf/cold_warmup.rs`, `tsf/probe_fsm.rs`, `tsf/probe.rs`, `output/vk_send.rs`

**修正履歴:**
- `8b90725`: long idle (>10s) 時の `FocusChange` / `SetOpenTrue` / `NativeF2Consumed` バジェットを
  1500ms → 2000ms に拡張（`かんきょうへんすうは → kあんきょうへんすうは` バグ修正）

---

## BUG-02: Chrome cold-start — probe タイミング想定外で1文字目がリテラルになる

**症状:** Chrome (VK Batched モード) でひらがな入力の最初の1文字がリテラル ASCII になる。
例: `という` → `toいう`

**原因:** Chrome は F2 受信後に composition context を非同期初期化する。
`ChromeProbe` フェーズは F2 送信時刻 (`f2_sent_ms`) を起点に `probe_min_ms` だけ待機してから
`found_io_after_warmup=false`（Chrome は F2 だけでは GJI I/O を出さない）で即解放するため、
min_ms が短すぎると Chrome の初期化完了前に T+O バッチが届いてリテラルになる。

**現在の対策:** `probe_min_ms` を以下の2段階で切り替える。

| 状況 | probe_min_ms | probe_max_ms | 定数名 |
|---|---|---|---|
| 通常（short idle） | 20ms | 120ms | `CHROME_PROBE_MIN/MAX_MS` |
| keyboard long idle (>10s) または 物理 F2 + GJI long idle | 200ms | 500ms | `CHROME_PROBE_LONG_IDLE_MIN/MAX_MS` |

物理 F2 (F2NonTsf) の場合は `cold_marked_ms`（物理 F2 の時刻）を probe 基準点とし、
プログラム的 F2 の三重送信バグ（`かんりのつごう → kaんりのつごう`）を防ぐ。
`long_idle || f2_gji_long_idle` の両条件で同じ `CHROME_PROBE_LONG_IDLE_MIN/MAX_MS` を使う。

**残存リスク:**
- 「keyboard short idle かつ GJI short idle」の条件下で Chrome が 20ms より長く必要とする
  ケースが存在する場合は対応できていない。
- `long_idle && skip_f2_send=true`（keyboard >10s + 物理 F2）のとき `probe_min_ms=200ms` が
  適用されるが、これが不十分かどうか未検証。

**関連ファイル:** `output/vk_send.rs`, `tsf/probe_fsm.rs`, `tuning.rs`

**修正履歴:**
- `b101153`: Chrome keyboard long idle 時に `CHROME_PROBE_LONG_IDLE_MIN/MAX_MS` を導入
  （`こ → ko` バグ修正）
- `79134f5`: 物理 F2 + GJI long idle 時に `CHROME_PROBE_F2_GJI_IDLE_MIN_MS=350ms` を導入
  （`という → toいう` バグ修正、GJI が12秒休眠後に Chrome の composition context 再初期化に
  ~326ms 必要だった事例）→ 後に `CHROME_PROBE_LONG_IDLE_MIN/MAX_MS` に統合（350ms → 200ms 値変更）

---

## BUG-03: LiteralDetect 偽陽性（false positive CompositionConfirmed）

**症状:** T+O がリテラル ASCII として出力されたにもかかわらず `CompositionConfirmed` と判定され、
BS リカバリが発動しない。結果: `to` + `いう` のように最初の1文字がリテラルのまま残る。

**原因:** `LiteralDetector::check_now` は `was_candidate_visible=false` のとき
`gji_candidate_show.has_changed(baseline)` で判定する。T+O 送信後に Chrome が composition mode に
移行して GJI SHOW が発火した場合、T+O 自体の composition 成否に関わらず `CompositionConfirmed`
と判定される。これは BUG-02 の「物理 F2 後の Chrome 初期化遅延」と組み合わさって発生する。

**現在の対策:** BUG-02 の probe timing 延長により、T+O 送信前に Chrome の初期化を待つことで
LiteralDetect が偽陽性になる状況自体を減らしている。

**SuspectedLiteral 方向の誤検出抑制:** `consecutive_count` チェックにより2回連続
`SuspectedLiteral` が出た場合は false positive とみなして BS リカバリを抑制する
（`probe_io.rs` の `RawTsfLiteralRecovery` dispatcher 参照）。

**残存リスク:**
- 偽陽性 CompositionConfirmed が発生した場合（BUG-02 の対策をすり抜けた場合）に
  BS リカバリが発動しないため、リテラル文字がそのまま残る。
- Chrome 以外のアプリでも同様の GJI SHOW タイミング問題が起きる可能性がある。

**関連ファイル:** `tsf/probe.rs` (`LiteralDetector`), `output/probe_io.rs`

---

## BUG-04: GJI モニター切断時のフォールバック

**症状:** GJI モニタースレッドが切断（`gji_monitor_ok=false`）している場合、
probe は GJI 観測を行わず `max_deadline` に達したら送信するフォールバックに移行する。
また LiteralDetect も起動しない。

**原因:** `TsfReadinessProbe::check_now` 冒頭の判定:
```rust
if !TSF_OBS.gji_monitor_ok.load(Acquire) {
    return now >= max_deadline;
}
```
GJI が使えない場合は固定タイムアウト待ちになる（BUG-01 の旧実装と同等の挙動）。

**影響:**
- probe の品質が低下し、タイムアウト超過が常態化する。
- LiteralDetect が無効化されるため、literal 出力が発生しても BS リカバリが走らない。

**GJI 再アタッチ:** `GjiMonitor` は切断後 `GJI_REATTACH_INTERVAL_MS=3000ms` ごとに
再アタッチを試みる。

**関連ファイル:** `tsf/probe.rs`, `tsf/gji_monitor.rs` (`GjiMonitor`), `tuning.rs`

---

## BUG-05: SessionExpired 閾値 (2000ms) が任意値

**症状:** 前回 SendInput から `COMPOSITION_TIMEOUT_MS=2000ms` 以上経過した後の最初の打鍵で
`SessionExpired` cold-start が発動し F2 warmup が再送信される。

**原因:** composition context が時間経過でいつ無効化されるか Windows API から通知されないため、
保守的な固定値 2000ms を閾値として設定している。

**残存リスク:**
- 2000ms より短い時間でも context が失効するアプリが存在する場合、文字化けが起きうる。
- 逆に 2000ms より長く維持されるアプリでは不要な warmup F2 が送信される（UX 悪化）。

**関連ファイル:** `output/mod.rs` (`assess_warmth`), `tuning.rs`

---

## BUG-06: focus_epoch のオーバーフロー ~~（解消済み）~~

> **2026-07-02 注記:** `focus_epoch: u32` / `composition_warm_epoch: u32` フィールドは
> `WarmEpoch` 構造体の再設計（ADR-069 凝集性リファクタ）で撤去済み。
> フォーカス変更時は `WarmEpoch::on_focus_changed()` が `eager_warmup_sent_ms` /
> `last_unicode_transmit_ms` をリセットするシンプルな方式に置き換わった。
> u32 カウンタによるオーバーフローリスクは構造的に消滅している。

~~**症状:** u32::MAX 回ウィンドウ切り替えを行うと `focus_epoch` がオーバーフローして 1 に戻る。
このタイミングで前のウィンドウの `composition_warm_epoch` と一致した場合、
stale な warm 状態が有効と誤判定される。~~

~~**原因:** `on_focus_changed()` で `focus_epoch.wrapping_add(1).max(1)` を使用。~~

~~**実用上の影響:** u32::MAX ≈ 42億回の切り替えが必要なため、実用上は発生しない。~~

**関連ファイル:** `tsf/probe.rs` (`WarmEpoch`)

---

## BUG-07: Edge/Chrome フォーカス約500ms後に Engine が必ず OFF になる（偽 FocusProbe 観測）

**症状:** MS Edge / Chrome（`Chrome_WidgetWin_1`、Imm32Unavailable プロファイル）に
フォーカスすると、実 IME は ON のまま awase の belief だけが false になり、フォーカスの
約 500ms 後（ポーリング1周期後）に `Engine deactivated (reason=Inactive(ImeOff))`。
以後キーがローマ字のままパススルーされる。ユーザーが同期キーで明示 ON し直すまで回復しない。
フォーカス変更のたびに再発する。

**原因:** `ce45b82`（2026-05-27、Win+X メニューの1文字ショートカットが NICOLA 変換される
バグの修正）が、`settle_tsf_gate_after_refresh()` の bypass 確定パス（非 ForceTsf ウィンドウ）
で **probe を実行していないのに** `write_focus_probe(false)` を毎リフレッシュ注入していた。
コミット本文の前提「非TSFウィンドウには日本語IMEが存在しない」が誤り:
Edge/Chrome は非TSF注入（injection=Unicode）だが日本語 IME は有効。

因果連鎖（2026-07-06 の実ログで確認）:

1. Edge フォーカス（07.269）: FocusChanged で観測クリア、desired=true → belief=true、Engine activated
2. 1回目 refresh 完了直後: `settle_tsf_gate_after_refresh` が**ログなしで** FocusProbe(Low, false) を注入
3. Imm32Unavailable は Blacklist（実観測経路ゼロ）のため、偽 Low false が
   `most_recent_trusted()` フォールバックで `effective_open()` を支配（Medium/High の訂正が来ない）
4. 2回目 refresh（+500ms、07.773）: belief=false を読み Engine deactivated →
   さらに SetOpen(false) を dispatch し 0x1A を送信（実 IME は無反応で ON のまま → 乖離固定）
5. first-key FocusProbe が shadow 値 false を代替観測としてエコー、HwndCache も false を保存
   → 自己強化

一般アプリで顕在化しないのは、ObserverPoll/ImmCrossProbe の実観測（Medium/High）が
偽 Low を上書きするため。**実観測経路を持たない Imm32Unavailable でのみ** Low が belief を
支配する。前日の ObservedEisu 循環デッドロック修正（input_mode 側）とは独立の経路で、
そちらを直しても本症状が残った理由。

**修正:** `write_focus_probe(false)` を撤去（ce45b82 の実質 revert）。ce45b82 の元バグ
（Win+X メニュー）は、現在は `classify.rs` の既知 NonText クラス判定
（`XamlExplorerHostIslandWindow`）+ `message_handlers.rs` の NonText パススルーが
belief と独立に防ぐため再発しない。ime-belief-architecture 規約の禁止パターン2
（観測の偽装）の実例であり、`tests/architecture_guard.rs::focus_probe_observation_is_limited_to_real_probe_path`
が `write_focus_probe` の呼び出し箇所を実 probe 経路（`key_pipeline.rs` の1箇所）に固定して
再発を防止する。

**関連ファイル:** `runtime/mod.rs` (`settle_tsf_gate_after_refresh`),
`state/observation_store.rs` (`most_recent_trusted`), `runtime/key_pipeline.rs` (`apply_effective_ime`)

**修正履歴:**
- `ce45b82` (2026-05-27): 偽観測を導入（Win+X 対策としては当時有効だったが前提が誤り）
- 2026-07-06: 偽観測撤去 + architecture_guard 追加（本修正）

---

## BUG-08: 外部注入 VK_KANA によるかなロックトグルで JIS かな入力化（GJI/Windows Terminal）

**症状:** Windows Terminal（TsfNative × GJI）で突然 JIS かな入力が有効になり、awase の
romaji VK 出力（例: `ko` → `[4B,4F]`）がかな配列として解釈されて出力が壊滅する。
GJI の conv が `Hiragana/roma (0x0019)` → `Hiragana/kana (0x0009)`（ROMAN ビット喪失）に反転。

**原因:** **合成 VK_KANA (0x15) down→up ペア**（実測 135µs〜1ms 間隔 — USB ポーリング
1ms・デバウンス 5ms を下回り物理押下では不可能）が hook に到達し、may_change_ime キー
としてそのまま OS にパススルー。VK_KANA はかなロックをトグルするため、GJI が
ローマ字入力⇔かな入力を反転する。2026-07-06T04:15 の実機ログで 2 回観測
（1回目でかな→ローマ、2回目でローマ→かな）。

**注入元の切り分け（2026-07-06 時点）:**
- **awase 自身ではない（コード監査で確定）**: (1) VK_KANA(0x15) を送るコードが存在しない
  （`VK_IME_ON=0x16`/`VK_IME_OFF=0x1A`、off-by-one もなし）。(2) awase の KEYBDINPUT
  構築箇所は全 6 箇所で、すべて INJECTED/TSF/IME_KANJI マーカー付き。マーカー付きは
  hook の `is_self_injected` が engine-input ログより前で除外するため、観測された
  `extra=0x0` のイベントは awase の SendInput では作れない。
- **GJI のかなロック補正ではない（ユーザー環境の確定情報）**: 当該セッションの実際の
  アクティブ IME は MS-IME（GJI は Converter プロセスが常駐しているだけ）。なお awase は
  このセッションで `ime=GJI` と誤検出していた（BUG-09 参照）。
- **LLKHF_INJECTED フラグの有無は未確認**（当時のログに未記録）。SendInput 系
  （VcXsrv・MS-IME/CTF 自身がプログラム的 IME 制御を「IME On キー = VK_KANA」として
  エコーする挙動・タッチキーボード等）ならフラグ付き、ドライバレベル注入や
  キーボードファームウェアマクロならフラグなし。次回発生時に特定できるよう hook に
  VK_KANA 到達時の診断ログ（injected/scan/extra）を追加済み。

修正前は誰も復元できなかった: idle-conv-check は `is_roman_reliable=false`（TsfNative では
ROMAN ビットを信用しない設計）のため conv=0x0009 を読んでも belief 変更なし・是正なしで、
かな入力のまま固定された。

**修正（二層防御）:**
1. **hook 層（原因遮断 + 診断）**: foreign-injected（`LLKHF_INJECTED` かつ非 self-marker）の
   VK_KANA を swallow する（`hook.rs`）。フラグなし（物理押下含む）は通すが INFO ログを
   必ず残す。注入元がフラグなしの場合はこの層をすり抜けるが、次の層が復元する。
2. **idle-conv-check 層（自己修復）**: `classify_conv_transition` に `restore_roman` を追加。
   engine open 中に「ひらがな conv で ROMAN 無し」を観測したら、conv 権限
   （`conv_mutation_allowed`）確認の上 `set_ime_romaji_mode_with_target_async(None)`
   （現 conv | ROMAN、冪等）で復元する。
   **2026-07-06 追補**: 当初は `conv_mode_changed` 遷移時のみ発火させたが、roma→kana の
   変化検出はフォーカス変更時 refresh の `update_from_conv` が先に消費するため
   idle-conv-check から見た conv は常に steady になり、一度も発火しなかった
   （05:05 実機: WT がセッション中ずっと conv=0x0009 のまま）。steady-state でも
   発火するよう変更し、スパム防止は呼び出し元のレート制限（3s 間隔、
   `last_roman_restore_ms`）に移した。
   **2026-07-06 追補2（撤回）**: steady-state 発火は**撤回**。MS-IME × TsfNative では
   closed/idle 時の conv 読み取りが ROMAN ビットを落として報告する（偽陽性 — 古い
   「TsfNative では ROMAN が常に 0」コメントは正しかった）。復元書き込みが conv を
   0x19⇄0x09 で往復させ、`ObservedEisu` / `NativeToggleShadowOff` を誤発火させて
   **直接入力中の spurious Engine ON + IME ON** を実機で引き起こした（05:28）。
   05:05 の「JISかな残留」も偽陽性の誤診だった可能性が高い（出力は正常だった）。
   `restore_roman` は `is_roman_reliable=true` の文脈のみ発火する仕様に変更 —
   TsfNative idle 経路（常に false）では発火せず、実質 hook 層の VK_KANA swallow が
   本物のかなロックトグルへの防御となる。docs/experiments.md エントリ 03 参照。

**再発防止テスト:** `state/conv_classify.rs` の unit tests（`jiskana_transition_while_open_requests_restore_roman` 等 6 件 + 不変条件）、
`tests/journals/jiskana-vk-kana-injection.json`（実機ログからの リプレイ fixture、Linux でも実行可）。

**関連ファイル:** `hook.rs`（swallow）、`state/conv_classify.rs`（検出）、
`runtime/key_pipeline.rs`（Apply(3) 復元）、`ime.rs::set_ime_romaji_mode_with_target`

**残存リスク:** 注入元が VK_KANA 以外の経路（ImmSetConversionStatus 直叩き等）でかな化
させる場合は hook 層では防げないが、その場合も idle-conv-check 層が数秒で復元する。
物理 VK_KANA を意図的に押した場合も同様に復元される（awase engine ON 中の JIS かな入力は
サポートしない設計判断）。

---

## BUG-09: post_to_main_thread の誤配送 — WM_IME_KIND_CHANGED / WM_FOCUS_KIND_UPDATE がワーカースレッドから main に届かない

**症状:** 2026-07-06T04:15 セッション（Windows Terminal）で、実際のアクティブ IME は
MS-IME（ユーザー確認済み、GJI は Converter プロセス常駐のみ）なのに、awase の出力層は
`[key-output] ... ime=GJI` として GJI 戦略で動作:
`[gji-fsm] StartProbe` → GJI I/O 静止（`gji_idle=200000ms+`）→ `PendingGjiConfirm:
GJI 未応答 → unicode で強制送信`。一方、同ログの起動時検出は
`[tip-detect] initial IME kind: MicrosoftIme` と**正しかった**（ユーザー提供ログで確認）。
つまり「検出は正しいのに出力層に伝わらない」split-brain。

**原因（確定）:** `win32::post_to_main_thread` が `PostMessageW(None, ..)` を使っていた。
hwnd=NULL の `PostMessageW` は「**呼び出しスレッド自身**への `PostThreadMessage`」と
等価（Microsoft docs）。main スレッドから呼ぶ分（`with_app_or_repost` の再 post 等）は
偶然正しく動くが、ワーカースレッドから呼ぶと自分の（誰も読まない）キューに消える:

- **gji-io-monitor worker** → `WM_IME_KIND_CHANGED` 消失 → `handle_wm_ime_kind_changed`
  が一度も走らず、warmup 戦略がデフォルトの `GjiFsm::new()`
  （`TsfWarmupCoordinator::new`）のまま。MS-IME 環境で GJI probe / unicode 強制送信の
  迷走を引き起こした。なお belief 側の `tsf_obs().active_ime_kind()` は atomic 直読みの
  ため正しく、`ime_controller`（MS-IME direct 選択）や GJI observe 判定は正常だった —
  出力層だけが壊れるため気づきにくかった。
- **UIA worker** → `WM_FOCUS_KIND_UPDATE`（UIA 非同期分類の結果）も同様に消失。
  `UIA async: hwnd → TextInput/NonText` のログは出るが main には届いていなかった疑い。
  UIA 由来の focus_kind 更新に依存する挙動が実質無効化されていた。

初期調査の per-thread `GetActiveProfile` 固着仮説は、起動時検出が正しかったことで棄却。

**修正:**
1. `post_to_main_thread(_with)` を `PostThreadMessageW(engine_thread_id(), ..)` に変更。
   どのスレッドから呼んでも main に届く。TID 未設定（ループ開始前）のみ旧動作に
   フォールバック（その時点の呼び出し元は main 自身のため正しい）。
2. `run_message_loop` 先頭で、検出済み IME 種別による warmup 戦略の pull 同期を追加
   （TID 設定前に発行された初回通知の取りこぼし保険）。

**検証方法（実機）:** 起動ログに `[runtime] startup IME kind sync:` と、IME 切替時に
`[runtime] WM_IME_KIND_CHANGED received` / `[output] Switching warmup strategy →` が
出ること。MS-IME で `[gji-fsm]` の probe が走らないこと。

**関連ファイル:** `win32.rs` (`post_to_main_thread`), `app/mod.rs::run_message_loop`,
`tsf/gji_monitor.rs::monitor_loop`, `focus/uia.rs`（UIA worker）,
`output/tsf_warmup_coord.rs` (`set_active_ime_kind`)

---

## BUG-10: MS-IME で物理ひらがなキー（VK_DBE_HIRAGANA）が食い逃げされ IME ON にならない

**症状:** 直接入力中に物理ひらがなキーで IME ON しようとすると、intent は記録され
Engine は ON になるが、実 IME は OFF のまま。以後の親指シフト入力が生 ASCII で出る。
2026-07-06T05:06 実機（Windows Terminal × MS-IME）。

**原因:** `PhysicalKeyDisposition::plan` が TSF mode の物理 F2 (VK_DBE_HIRAGANA) を
**無条件 Suppress** していた。この Suppress は「awase 自身が warmup として F2 を再送する」
GJI 戦略の double-F2 防止契約とセットの設計だが、MsImeStrategy は
`needs_f2_probe()=false` で F2 warmup を送らない（`send_eager_tsf_warmup` が
non-GJI としてスキップ、trace レベルのためログにも写らない）。「消すが代わりを送らない」
食い逃げになり、`EmitWarmup (NativeF2)` の後に `[tsf-eager-warmup] 送信` が一度も出ず、
後続送信も `prepend_f2_warmup=false` のまま。

**修正:** `plan()` に `f2_warmup_owned`（= `needs_f2_probe()`、GJI 戦略か）を渡し、
Suppress を `is_tsf_mode && f2_warmup_owned` に限定。MsImeStrategy では物理 F2 を素通し
（MS-IME は VK_DBE_HIRAGANA をネイティブ処理して IME ON にする）。

**再発防止テスト:** `transport.rs::plan_tests::f2_tsf_mode_msime_strategy_allows_physical_key`
（Windows 実行）。

**関連ファイル:** `runtime/transport.rs` (`plan`), `runtime/key_pipeline.rs` (`kp_stage_execute`),
`output/mod.rs` (`f2_warmup_owned` / `send_eager_tsf_warmup`)

---

## BUG-11: UIA 結果のキャッシュキー取り違えで Edge が永久 NonText（全キーがエンジン素通し）

**症状:** Edge（Chrome_WidgetWin_1）で実 IME は ON なのに NICOLA 変換されない
（「IME ON・Engine OFF」に見える）。2026-07-06T05:12 実機ログ: Edge でのキー入力が
`[engine-input]` なしの `[reinject]` のみ（NonText 全パススルーでエンジン素通し）、
直後の WT 移動時に `Focus kind changed: NonText → TextInput (reason=cache hit)` —
つまり Edge 滞在中ずっと focus_kind=NonText だった。

**原因:** `handle_wm_focus_kind_update`（UIA 非同期分類結果の受信ハンドラ）が、
キャッシュ挿入のキーを **awase 内部の focus 追跡（platform.focus、refresh 経由で
最大数百 ms 遅延）** から取っていた。実フォーカス照合（GetGUIThreadInfo）は
result_hwnd に対して通るため、「Alt+Tab メニュー（XamlExplorerHostIslandWindow）の
NonText 結果」が「まだ Edge を指している追跡状態のキー (msedge, Chrome_WidgetWin_1)」で
`cache_insert` される。以後 Edge は resolve が cache hit で NonText を返し続け、
NonText は Undetermined ではないため UIA 再問い合わせも走らず**自己回復しない**
（awase 再起動まで永続）。

このハンドラは BUG-09 修正（post_to_main_thread の誤配送修正）で**史上初めて実行される
ようになった**コード。BUG-09 以前は WM_FOCUS_KIND_UPDATE 自体が消失していたため
潜在バグが露出しなかった（配送修正の副作用として発症）。

**修正:** 結果の帰属（pid/class）を `result_hwnd` 自身から導出
（GetWindowThreadProcessId + GetClassNameW）。キャッシュはその正しいキーで挿入し、
グローバルな focus_kind / app_kind への反映は「追跡中ウィンドウと pid+class が一致する
場合のみ」に限定。毒入り済みキャッシュはメモリ上のみのため awase 再起動で消える。

**【2026-07-06 追記】この修正では不十分だった** — 帰属を正しくしても、ページ本文
フォーカス時の「正しい NonText」が (pid, class) キーでキャッシュされ、同日 05:28 に
Edge 永久 NonText が再発（`Focus kind changed: TextInput → NonText (reason=cache hit)`）。
粒度の構造的不一致が真因。**BUG-12 で UIA 結果の適用自体を無効化した。**

**関連ファイル:** `runtime/message_handlers.rs` (`handle_wm_focus_kind_update`),
`focus/uia.rs`（結果送信側）, `runtime/focus_tracking.rs::classify_focus_probe`（cache hit 消費側）

---

## BUG-12: UIA 非同期 focus 分類の適用を無効化（(pid,class) キャッシュ粒度がブラウザと構造的に不一致）

**症状:** BUG-11 修正後も Edge で「IME ON・Engine OFF」（全キーがエンジン素通し）が再発
（2026-07-06T05:28 実機: Edge 入場時 `Focus kind changed: TextInput → NonText
(reason=cache hit)`、キー入力が `[engine-input]` なしの `[reinject]` のみ）。

**原因（構造的）:** ブラウザ（Chrome_WidgetWin_1）の focus kind は「ウィンドウ内の
どの要素にフォーカスがあるか」で毎秒変わる。UIA がページ本文フォーカス時に返す
**正しい NonText** であっても、(pid, class) 粒度でキャッシュした瞬間にウィンドウ全体へ
固着する。ウィンドウ内クリックはトップレベルフォーカス変更として観測できないため
再分類されず、自己回復しない。帰属の正確さ（BUG-11 修正）では解決不能な粒度問題。

**経緯:** `handle_wm_focus_kind_update` は BUG-09（post_to_main_thread 誤配送）の修正まで
**一度も実行されたことのない**コードだった。配送を直した途端に BUG-11 → BUG-12 と
2 段階の実害が露出した。システム全体が「UIA 結果は届かない」前提で長期間チューニング
されてきたため、安全に有効化するには hwnd 粒度 + ウィンドウ内フォーカス要素の追跡
（UIA FocusChanged イベント購読）という別設計が必要。

**対処:** handler をログのみ（適用・キャッシュなし）に変更し、配送修正前の実績ある
挙動へ意図的に戻した。sync 分類（既知クラス・WS_EX_NOIME・MSAA）は従来どおり機能する。
BUG-09 修正の本来の成果（`WM_IME_KIND_CHANGED` → warmup 戦略切替、実機検証済み）は維持。

**関連ファイル:** `runtime/message_handlers.rs` (`handle_wm_focus_kind_update`),
`focus/uia.rs`（worker は診断ログ用に稼働継続）

---

## BUG-13: MS-IME cold start — IME ON 遷移直後の送信で先頭文字がリテラル化（「を」→「wお」）

**症状:** MS-IME（Windows Terminal 等の TSF-native アプリ）で、IME OFF→ON の直後
（~300ms 以内）に文字を打つと先頭 VK がリテラル化する。
2026-07-06 実機: IME ON 操作の +122ms 後に「を」(romaji "wo") を送信 → 'W' が
リテラル 'w' として確定し 'O' だけが compose されて「wお」。送信時の診断読みは
`[h1-send] conv=0x00000000 NATIVE=false`（= シグナルは手元にあったのにゲートして
いなかった）。準備完了後の +281ms では conv=0x00000009 で正常。

**原因:** `MsImeStrategy` が「MS-IME の TSF context は常にウォーム」前提で
`is_warm()=true` / `needs_f2_probe()=false` を固定しており、cold-start 保護が皆無
だった。この前提は IME が既に ON の定常状態でのみ正しく、OFF→ON 遷移直後の
~130-300ms（実測）は成り立たない。`mark_composition_cold` の cold マークも MS-IME
経路では誰にも消費されない死にマークだった。GJI には F2 probe（プロセス I/O 観測）
の confirm-then-transmit があるが、MS-IME 側には相当機構がなかった。

**修正（confirm-then-transmit、固定待ちではなく観測ベース）:**
- `ImeModeFsm` に `on_set_open_applied` を追加し、`on_ime_applied`（全 apply 経路の
  ファネル）から belief を unconfirmed 化。MsImeDirect は VK_IME_ON/OFF を送らず
  `on_ime_mode_vk_sent` を経由しないため、ここが唯一の invalidate 点。
- `send_romaji_as_tsf` に `ms_ime_gate_defer` ゲートを追加: MS-IME + TSF mode +
  `ImeModeFsm` NATIVE 未確認なら romaji を `MsImeReadyCoro`（StepCoro, `pending_tsf`）
  に預け、`start_ms_ime_ready_poll` の IMC ポーリング（10ms 間隔）が
  `IMC_GETCONVERSIONMODE` の NATIVE ビットを確認した瞬間に送信。後続キーは既存の
  deferred VK 機構で順序維持。
- `MS_IME_READY_CONFIRM_MS` (400ms) は待ち時間ではなく安全弁（IMC が読めない環境で
  タイピングを止めないための上限）。期限切れは強制送信 + give-up latch
  （`ms_ime_gate_give_up`、フォーカス変更 / 次の IME ON で解除）で毎キー probe 化を防ぐ。

**再発防止テスト:** `tsf/warmup/ms_ime_ready_coro.rs::tests`（確認待ち→Transmit、
期限切れ強制送信、NATIVE 判定の unconfirmed 除外）。

**関連ファイル:** `tsf/warmup/ms_ime_ready_coro.rs`, `output/vk_send.rs`
(`ms_ime_gate_defer`), `output/probe_io.rs` (`start_ms_ime_ready_poll`),
`tsf/ime_mode_fsm.rs` (`on_set_open_applied` / `is_native_ready`), `platform.rs`
(`on_ime_applied`), `tuning.rs` (`MS_IME_READY_CONFIRM_MS`)

**関連バグ:** 発症の前段（belief と OS の silent 乖離）は MS-IME キー割り当て二重
オーナー問題（`msime_key_assignment.rs`、コミット a0a4f68 の検出ポップアップ）。
本修正は乖離が起きた後でも先頭文字リテラル化を防ぐ第二の防衛線。

---

## BUG-14: 外部注入 VK_DBE_HIRAGANA を物理かなキーと誤読し、ユーザーの IME OFF を Engine ON で上書きし続ける

**症状:** MS-IME（Windows Terminal / TsfNative）で Ctrl+無変換により IME OFF に
した後、キーを何も押していないのに Engine が勝手に ON に戻る。手動で OFF に
し直しても繰り返し再発する。ユーザー体感では Shift の使用と相関がある
（2026-07-06 実機報告）。

**実機ログ（2026-07-06T23:22）:**

```
23:22:28.199  Ctrl+無変換 → IME OFF (key combo)（ユーザーの明示 OFF）
23:22:32.731  [hook] IME-mode vk=0xF0 up   self_injected=false scan=0x70 extra=0x0
23:22:32.732  [hook] IME-mode vk=0xF2 down self_injected=false scan=0x70 extra=0x0
23:22:32.733  Shadow IME toggle: OFF → ON (vk=0xF2, source=PhysicalImeKey)
23:22:32.733  Engine activated (ime=true, ...)
```

直前 3.7 秒間キー入力は一切なし。`0xF0 up` → `0xF2 down` の間隔は **0.5ms** で、
物理押下なら down と up の間にホールド時間（数十〜百 ms）が挟まるため物理では
説明できない。

**外部注入と断定できる根拠（VK ペア翻訳のシグネチャ一致):** awase 自身が 4ms 後に
送った IME ON（SendInput で `VK_DBE_HIRAGANA` down+up の 2 イベント）が、hook 上では
まったく同じ `0xF0 up` → `0xF2 down` ペア（self_injected=true, 4ms 間隔）として観測
された。つまり OS は「VK_DBE_HIRAGANA down+up の注入」を LL hook 上でこのペアに
翻訳して報告する。問題の foreign ペアはこの翻訳シグネチャと完全に同型。
scan=0x70（かなキーの scancode）は注入側が MapVirtualKey 相当で scancode を
埋めれば付くため、物理の証拠にならない。

**注入元:** 未確定。BUG-08 の合成 VK_KANA（135µs〜1ms ペア、同じく extra=0x0）と
同一ファミリーとみられ、第一容疑は MS-IME/CTF がプログラム的 IME 制御・入力モード
遷移をキーイベントとしてエコーする挙動。ユーザー報告の「Shift と相関」は、MS-IME の
Shift による英数⇔かな切替時にエコー注入が走る仮説と整合する。BUG-08 当時は
LLKHF_INJECTED をログしておらず特定できなかったため、今回 `[hook] IME-mode` ログに
`injected=` を追加した（次回発生時にフラグの有無で SendInput 由来かドライバレベルかを
確定できる）。

**因果連鎖:** `0xF2`（VK_DBE_HIRAGANA）は `ImeKeyKind::Activate` →
`shadow_effect()=TurnOn`（`vk.rs`）。`kp_stage_shadow_ime_toggle`
（`runtime/key_pipeline.rs`）が日本語 IME 環境でこれを `UserIntentSource::PhysicalImeKey`
のユーザー意図として採用 → `write_physical_key(true)` → Engine activated + IME ON
apply。注入が繰り返されるたびにユーザーの明示 OFF が上書きされる。

**修正の試行 1（swallow 一般化 — 即日撤回）:** `hook.rs` の foreign-injected swallow を
VK_KANA 限定から IME モードキー全般に拡張した（`b8467b8`）が、**導入直後から
Windows Terminal × MS-IME で一切入力できなくなり撤回**。撤回時のログで、
**1 打鍵ごとに foreign-injected VK_KANA down+up ペア（injected=true, scan=0x0,
extra=0x0）が到達**していることが判明 — foreign-injected IME モードキーには MS-IME
自身の機能的なキー注入（モード遷移・かな修飾とみられる）が含まれ、hook 層で遮断すると
IME の状態機械が壊れる。conv=0x0009 (ROMAN=false) 固定・エンジン全キー PassThrough の
まま復帰しなかった。詳細は [docs/experiments.md](experiments.md) エントリ 04。

**確定した事実（injected= ログの成果）:** BUG-08 以来未特定だった注入元は
**LLKHF_INJECTED 付き SendInput 由来**（ドライバレベルではない）。VK_KANA swallow
（BUG-08）はこの高頻度エコーを従来から swallow しており実害なし → 維持。

**修正（試行 2 — 遮断ではなく解釈の修正）:**
- `RawKeyEvent` に `injected: bool` を追加（`src/types.rs`）。hook が `LLKHF_INJECTED` を
  伝搬する（awase 自身のマーカー付き注入は従来どおりフック層で除外済みのため、
  true = 他プロセスの SendInput のみ）。
- `kp_stage_shadow_ime_toggle`（`runtime/key_pipeline.rs`）の冒頭で `event.injected` なら
  SyncKey / PhysicalImeKey のユーザー意図に昇格させず return。OS への配送
  （passthrough / reinject）は一切変えないので、MS-IME 自身の機能的注入は壊れない。
- 実 IME 状態への belief 追従は、既存の `may_change_ime` → `schedule_ime_refresh(20ms)`
  の観測経路（confidence 付き）に委ねる — ime-belief-architecture の
  「観測と意図の分離」に沿った形。
- 発動時は `[shadow-toggle] injected IME キー vk=0x.. はユーザー意図に昇格させない (BUG-14)`
  の INFO ログが出る。

**再発防止:** 本エントリ（症状・翻訳シグネチャ・swallow が不可な理由）＋
[docs/experiments.md](experiments.md) エントリ 04。`kp_stage_shadow_ime_toggle` は
Windows cfg 下のため Linux CI での直接テストは不可、injected ガードの退行は
上記 INFO ログと本記録で検知する。

**関連ファイル:** `src/types.rs`（`RawKeyEvent::injected`）、`hook.rs`（injected 伝搬 +
injected= ログ）、`runtime/key_pipeline.rs::kp_stage_shadow_ime_toggle`（injected ガード）、
`vk.rs::ImeKeyKind`

**関連バグ:** BUG-08（同一ファミリーの合成 VK_KANA）、MS-IME 二重オーナー問題
（`msime_key_assignment.rs`）、BUG-15（Shift 単独タップ誤認も同じ二重オーナー構造）

---

## BUG-15: Shift 面使用後の Shift 解放で MS-IME が英数モードに落ち、かな入力が数秒壊れる

**症状:** MS-IME（Windows Terminal / TsfNative）で Shift を押しながら文字キーを打ち
（Shift 面 → 全角英字出力）、Shift を離した後にかな入力へ戻らない。
2026-07-07T00:04 実機: Shift up の 478ms 後に conv=0x0000（半角英数）を観測 →
idle-conv-check が ObservedEisu → DirectInput → **Engine OFF** まで連鎖し、
直後の打鍵が素通り。conv=0x0009 が観測されて NativeToggleShadowOff で
Engine ON に復帰するまで数秒〜十数秒かな入力が壊れた。

**原因（二重オーナー構造）:** awase が Shift 押下中の文字キーをエンジンで consume
するため、OS / MS-IME からは「Shift down → （何もなし） → Shift up」だけが見える。
MS-IME の「Shift キー単独で英数モードに切り替える」がこれを単独タップと誤認して
conv を 0x0000 へ切り替える。ユーザー操作としては Shift+文字入力であり誤爆。
BUG-14 の「Shift と相関する外部注入 VK_DBE_HIRAGANA」も、この英数切替の
復帰側エコーとして整合する。

**修正（2 層）:**
1. **Shift 面の半角リテラル化（`shift_plane_halfwidth`、デフォルト有効）**:
   `KeyAction::Text`（`KEYEVENTF_UNICODE` 直接出力、IME 非経由）を新設し、
   Shift 面の全角英数値を半角化して Text で送る（`nicola_fsm.rs::shift_face_reduce`）。
   「Shift 押下中は半角英数入力」のユーザー要望を満たしつつ、IME の変換モード・
   composition に一切触れない。半角化結果が ASCII 印字文字でない値（かな等）は
   従来の IME 経由 Char を維持し、漢字変換可能性を壊さない。
2. **Shift 解放時の先回り復元（`kp_stage_shift_plane_release`）**: Shift 押下中に
   Shift 面で文字キーを consume していた場合のみ、Shift KeyUp で
   (a) explicit IME action マーク（idle-conv-check の ObservedEisu→DirectInput 連鎖を
   1500ms 抑止）、(b) `ImeModeFsm::unconfirm`（次の kana 送信は msime-ready ゲートが
   IMC の NATIVE を確認してから送信 = 先頭文字リテラル化防止）、
   (c) conv をかな入力（NATIVE|FULLSHAPE|ROMAN、カタカナ中は KATAKANA target）へ
   冪等 write。MS-IME の誤切替タイミングが不定（実測上限 478ms）のため、
   160ms 間隔 ×4 回の verify-retry で NATIVE 確認まで再送する。
   本当に Shift を単独タップした場合（consume なし）は何もしない —
   MS-IME の Shift 単独英数切替を意図的に使う操作は妨げない。

**設定側の恒久対策は不可（2026-07-07 ユーザー確認）:** 「Shift キー単独で英数モードに
切り替える」は旧 IME の詳細設定にのみ存在し、**新 IME（Win11 標準 MS-IME）では
無効化できない**。したがって修正 2 の awase 側カウンターが唯一の防御であり、
「設定を切ればよい」という提案は選択肢にならない（再提案しないこと）。

**追補（2026-07-07 実機）: Shift 押下中の ASCII VK_PACKET は受信側で破棄される。**
修正 1 の初版は Text を素の `KEYEVENTF_UNICODE` で送っていたが、Windows Terminal で
**一切表示されなかった**。ログ上は `actions=[Text("K")]` → `→ Text("K") via Unicode
direct` まで完走しており、送信は行われている。全角 `Ｋ`（U+FF2B）は同じ
「物理 Shift 押下 + VK_PACKET」で届いていたため、**ASCII 文字の VK_PACKET だけが
受信側（Terminal）で Shift+キーとして再解釈され破棄される**と判明。対策として
`KeyInjector::send_text_direct` が物理 Shift 押下中は「Shift 解放 → VK_PACKET 列 →
Shift 復元」を 1 回の SendInput にまとめて bare で届ける（IME モードキー送信の
`HeldModifiers` release/restore と同じ手法）。なお修正 2（Shift 解放時の conv 復元 +
msime-ready ゲート連携）はこの実機ログで正常動作を確認済み
（`[shift-release] conv=0x00000019 NATIVE 確認 (#0) → 復元完了` → 直後のかな入力正常）。

**追補2（2026-07-07 実機）: bare 化しても不達 → VK_PACKET 注入を全面撤回し
「IME-ON 半角英数 hold」方式へ転換（試行 3、現行）。**
Shift 解放/復元付き bare 送信（`[text-direct]` 発動をログで確認）でも Windows
Terminal には一切表示されなかった。**ASCII の VK_PACKET は Shift の有無にかかわらず
Terminal に届かない**（推定: 1 SendInput 内の Shift 復元が GetAsyncKeyState ベースの
修飾判定に間に合わない、または ASCII VK_PACKET 自体を再解釈して破棄）。
注入方式を放棄し、ユーザー確認済みの意図「IME-ON のまま半角英数（直接入力ではない）」
どおり、**IME 自身に打たせる方式**に転換した:
- **Shift KeyDown**（物理・Ctrl/Alt/Win なし・MS-IME・IME ON・エンジン有効・conv 権限）
  → `[shift-eisu]` conv=0x00000000（IME-ON 半角英数）へ切替、`shift_eisu_hold` セット。
- **Shift 面の ASCII キーはエンジン素通し**（`shift_face_reduce` → PassThrough）。
  IME が半角英数モードで直接確定するため、受信側互換性の問題が構造的に消える
  （通常のキーボードで英数モードのまま打つのと同一経路。Shift+K=大文字 K、
  数字・記号も JIS どおり）。かな等の非 ASCII Shift 面は従来どおり Reduce。
- **hold 中は idle-conv-check と IME poll を凍結**（conv=0x0000 は自前の意図的状態。
  ObservedEisu → DirectInput 落ちに反応させない）。
- **Shift KeyUp** → 既存の `[shift-release]` verify-retry でかな入力へ復元
  （実機動作確認済み）。復元は hold したら必ず行う =「Shift を離したらかな」の仕様。
- BUG-15 本体（MS-IME の Shift 単独タップ誤認）もこの方式に吸収される: hold 中は
  awase 自身が英数にしており、解放時に必ず復元するため誤認の余地がない。
  副作用として MS-IME の「Shift 単独タップで英数に切替えっぱなし」は使えなくなる
  （Shift を離すと必ずかなに戻る）が、これはユーザー要望の仕様そのもの。
- 既知の残リスク: Shift down 直後 ~15ms 以内の初回キーは conv 切替が間に合わず
  romaji composition に入る可能性（Shift→初回キーの人間の間隔は通常 50ms 以上で
  実害は未観測。発生したら msime-ready 型の eisu 確認ゲートを追加する）。

**追補3（2026-07-07 実機）: 英数→かな方向の IMC write は実モードに反映されない
（IMM→TSF ブリッジの片方向故障）→ 復元は VK_DBE_HIRAGANA 注入に変更。**
試行 3 初版の Shift 解放復元は IMC write が success を返し、直後の IMC read も
conv=0x00000019/NATIVE を返す（`[shift-release] NATIVE 確認 (#0) → 復元完了`）のに、
**実際の MS-IME は半角英数のまま**だった（ユーザーが物理かなキー
= VK_DBE_HIRAGANA を押すと復帰。01:12 実機ログ）。逆方向（かな→英数、hold 開始側）の
IMC write=0x0000 は実モードに効く — **Windows Terminal の IMM ブリッジは
「英数→かな」方向の書き込みだけ TSF 実モードに反映されない**。
対処: 解放時にユーザーの手動回復と同じ VK_DBE_HIRAGANA（MS-IME ネイティブ処理、
BUG-10 と同じ経路）を `send_ime_mode_key` で注入し、IMC write/verify は保険として維持。
IMC read が実モードと乖離する以上 verify は完全な確認にはならない点に注意
（NATIVE 確認は「IMC エコーの確認」でしかない）。

**追補4（2026-07-07 実機）: scan=0x0 の注入 F2 は MS-IME (TSF) に無視される。**
追補3 の `send_ime_mode_key(VK_DBE_HIRAGANA)` は発火ログが出ているのに実モードが
戻らなかった。効いている経路との差分は **scan code の有無のみ**:
- 効く: 物理かなキーの reinject（scan=0x70）、TSF warmup の F2
  （`make_tsf_key_input`、`MapVirtualKeyW` で scan 算出）、物理 半角/全角（scan=0x29）
- 効かない: `send_ime_mode_key` = `make_key_input_ex`（**scan=0x0**）、IMC write
TSF 経由の MS-IME はモードキーを scancode で検証しているとみられる。復元 F2 を
`make_tsf_key_input`（scan 付き）構築に変更。あわせて、この注入は Shift KeyUp 処理中
（物理 Shift up の reinject 前 = OS 視点で Shift 押下中）に走るため、
**Shift+ひらがなキー = カタカナ切替に化けないよう synthetic Shift up を同一バッチの
先頭に前置**する。
教訓: 「IME モードキー注入が効かない」ときは marker/修飾より先に **scan=0 を疑う**。

**追補5（2026-07-07）: Shift 面の記号は .yab の書き方に従う。**
scan 修正で hold/復元が完動した後、「Shift+1 は全角 `！` にしたい」という要望に対し、
.yab の既存表現力（クォートの有無）で処遇を分けるようにした
（`shift_eisu_disposition`、`nicola_fsm.rs`）:
| .yab の Shift 面セル | 出力 |
|---|---|
| `Ｋ` / `'Ｋ'`（英数字） | 半角 `K`（素通し、IME-ON 半角英数） |
| `！`（クォートなし記号 → 半角化されて KeySequence） | 半角 `!`（素通し） |
| `'！'`（クォート付き全角記号 → Literal） | **全角 `！`**（Text 確定出力、非 ASCII VK_PACKET は届く） |
| `'ウ'` 等のかな literal | `ウ`（Text 確定出力） |
| Special（後/入 等） | 従来どおり |
全角で出したい記号はクォート付き `'！'` で Shift 面に定義する。

**追補7（2026-07-07 実機）: 追補6 の入口 F0/F3 注入は CapsLock を汚染するため撤回。**
`VK_DBE_ALPHANUMERIC`（scan 0x3A = 物理 CapsLock 位置）は、**実 IME が OFF の文脈に
着弾すると kbd106 の素の英数キー処理（CAPLOK）で CapsLock をトグルする**
（実機: belief ON × 実 OFF の窓で Shift 押下のたびに CapsLock 点灯）。
入口は IMC write のみに戻した。初回文字の全角化（追補6 の動機）は既知の限界として
許容（CapsLock 汚染より軽微）。**教訓: IME モードキーの注入は「実 IME が確実に ON」
でない限りしてはならない** — 解放側の F2（scan 0x70 = かなキー位置）も実 IME OFF に
着弾すると kbd106 のかなロックをトグルする同族ハザードを持つ（BUG-08 の JISかな化と
同根の危険。belief×実状態の乖離窓を塞ぐ BUG-16 系修正がこのハザードの暴露率を下げる）。

**追補6（2026-07-07 実機、撤回済み → 追補7）: hold 入口の IMC write は順序保証がなく初回文字が全角化
→ 入口も scan 付きモードキー注入に変更。**
Shift down の `[shift-eisu]` 発火から IMC write 着地まで実測 250ms かかるケースがあり、
その間に届いた最初の Shift+英字が MS-IME 自身の「Shift+英字 → 全角英数」挙動で
全角 `Ａ` になった（write 時の読み値 conv=0x0008=全角英数が証拠。2 文字目以降は
write 着地後で半角）。IMC write（SendMessage チャネル）は入力ストリームとの順序
保証がない。対処: 入口を VK_DBE_ALPHANUMERIC + VK_DBE_SBCSCHAR の scan 付き注入に
変更（`make_tsf_key_input`）。モードキーは後続の文字キー reinject と同じ入力キューを
通るため「切替 → 文字」の順序が構造的に保証される。IMC write は冪等な保険として維持。
出口（VK_DBE_HIRAGANA、追補4）と対称になった。
- `KeyAction::Text` / `send_text_direct` は注入が通るアプリ向けフォールバックとして
  コードは維持（現在エンジンからの producer なし）。

**再発防止テスト:** 撤去済み（追補8参照）。旧テストは `src/engine/tests.rs` の
`test_shift_face_fullwidth_ascii_becomes_halfwidth_text` /
`test_shift_face_halfwidth_disabled_keeps_literal` /
`test_shift_face_kana_stays_ime_routed`（いずれも削除済み）。

**関連ファイル（撤去前）:** `src/types.rs`（`KeyAction::Text`、削除済み）、
`src/engine/nicola_fsm.rs`（半角化、削除済み）、`src/config.rs`
（`shift_plane_halfwidth`、削除済み）、`runtime/key_pipeline.rs`
（`kp_stage_shift_plane_release` という名前で言及していたが実際のコードは
`kp_stage_shift_eisu_hold` の一関数だった。撤去後は後継の
`kp_stage_shift_conv_guard`/`kp_restore_kana_from_half_width` を参照）、
`state/platform_state.rs`（`GateStore::shift_plane_used_in_hold` という名前で
言及していたが実際のフィールド名は `shift_eisu_hold` だった）、
`tsf/ime_mode_fsm.rs::unconfirm`、`output/mod.rs`（Text 送信、削除済み）

**関連バグ:** BUG-14（Shift 相関の外部注入）、MS-IME 二重オーナー問題、BUG-25（撤去先）

---

**追補8（撤去、2026-07-11）: hold 方式を撤去し、左Shift単独タップによる持続トグルへ
置換。撤去の詳細と新機能は BUG-25 参照。**

撤去したのは「Shift 押しっぱなし中は半角英数 ASCII を素通しする」レイヤー
（`shift_plane_halfwidth` / `ShiftEisuDisposition` / `shift_eisu_disposition` /
`KeyAction::Text`）のみ。本エントリの本体である「MS-IME の Shift 単独タップ
誤検知に対する安全網」（Shift 押下→解放のたびに無条件で conv を英数へ→かなへ
書き戻す仕組み）は**撤去していない**——`kp_stage_shift_eisu_hold` を
`kp_stage_shift_conv_guard` に改名・再構成し、L/R 問わず無条件の書き戻しを
維持したまま、左Shift単独タップだけを持続トグルへ分岐させる形にした。この
区別を怠ると、Shift+文字キーのチョード（`.yab` Shift 面、`'！'` 等）で本エントリ
の症状がそのまま再発する（設計時に別エージェントのレビューで発覚、詳細は
BUG-25 参照）。

---

## BUG-16: フォーカス遷移の settle スキップに再試行がなく、belief ON × 実 IME OFF が放置される

**症状:** 仮想デスクトップ切替（Win+Ctrl+→）で Windows Terminal にフォーカスが移った
直後、belief は IME ON / Engine ON なのに実 IME は OFF のままで、最初のかな入力が
リテラル化する（2026-07-07T05:27 実機: 「これで」→「korede」。Ctrl+変換の手動
IME ON で復旧）。ユーザー体感は「遷移してすぐ IME OFF エンジン ON」。

**原因（3 つの穴の重なり）:**
1. 遷移直後の refresh 2 回がいずれも settle 期間内で、`apply_force_on_for_imm_broken`
   （Blacklist アプリへの belief 強制適用）が
   `[focus-settle] ... skipped (settling)` でスキップ。**スキップに再試行がない**。
2. 次の refresh は無保証で、実測では 8 秒後まで走らなかった（最初の打鍵は遷移
   3 秒後 = 無防備の窓）。
3. TsfNative は IME open 状態を読めず（`ime_on=None (preserving state)`）、さらに
   `ImeModeFsm` が focus 直後の conv 読み（0x19）で `initial confirm: Hiragana` して
   しまう — **conv は IME が閉じていても保持される**ため open の証拠にならないが、
   msime-ready ゲートはこれで通過し、romaji が閉じた IME にリテラル着弾した。

**修正:** settle 中にスキップした 3 箇所（`apply_force_on_for_imm_broken` /
`try_force_on_bootstrap` / drift correction）で、settle 明けの refresh 再試行を
スケジュールする（`schedule_ime_refresh(focus_settle_ms + 50ms)`。遅延は settle
残余の上限 = `focus_settle_ms` + タイマー粒度マージン）。settle 明けの force-ON が
0xF2 を送って belief を OS に適用し、無防備の窓を閉じる。

**関連ファイル:** `runtime/mod.rs`（force-on 2 箇所）、`runtime/ime_refresh.rs`
（drift correction）、`state/platform_state.rs`（`focus_settle_ms` アクセサ）

**追補（2026-07-07 実機）: Win キー押下中の IME キー注入スキップが Applied 扱いに
なり、再試行がすべて no-op 化していた。**
settle 明け再試行の導入後も再発（ロック解除 → Win+Ctrl+→ デスクトップ切替 →
Terminal で「これで」→「korede」）。ログで新しい真因を特定:
```
[apply-ime] MS-IME direct: send 0x00F2 (IME ON)
[ime-mode] skipped vk=0xF2 (Win key held — Win+VK_IME triggers Start Menu on Win↑)
[apply-ime] open=true eff=true conf=true → outcome=Applied   ← 送っていないのに Applied
```
`send_ime_mode_key` は Win 押下中に注入をスキップする（スタートメニュー誤起動防止、
正しい挙動）が、呼び出し元 strategy が **スキップを知らず Applied を返し
applied_snapshot がラッチ**。以降の force-ON / settle 明け再試行 / poll がすべて
「適用済み」として無言 no-op になり、belief ON × 実 IME OFF が固定された。
Win+Ctrl+→（仮想デスクトップ切替）はユーザーの常用操作なので、切替直後に engine ON
同期が走ると高確率で踏む。
修正: `send_ime_mode_key` が送信有無を `bool` で返すよう変更し、
GjiDirect / MsImeDirect strategy はスキップ時に `ImeOpenOutcome::UnsafeToToggle`
（= applied_snapshot / state を更新しない既存の意味論）を返す。
`send_engine_state_ime_key` もスキップ時は `on_ime_mode_vk_sent` を呼ばない。
これで Win 解放後の次の refresh / force-ON が実際に再送する。

**追補2（2026-07-07 実機）: force-ON の実体 `platform.set_ime_open` は IMM 専用で、
対象の Blacklist アプリでは常に no-op だった。**
上記 2 修正後も再発（ロック解除 → デスクトップ切替 → Terminal で「koreha」化）。
`apply_force_on_for_imm_broken` / `try_force_on_bootstrap` が呼ぶ
`platform.set_ime_open` は `can_use_imm32_cross_process()==false`（= Imm32Unavailable /
TSF-native、**まさに force-ON の対象アプリ**）で早期 return する実装であり、
**force-ON は導入以来一度も実際の適用を行えていなかった**（settle 明け再試行も
「何もしない関数」の再試行だった）。手動 Ctrl+変換が毎回効いたのは strategy chain
（MsImeDirect の冪等 VK_DBE_HIRAGANA）経由だから。
修正: 両 force-ON を `apply_ime_open_with_belief(true, ...)` +
`on_ime_apply_complete` の strategy chain 経由に変更。あわせて applied が既に ON の
場合は送らないスパムガードを追加（FocusChange が applied=Unknown にリセットするため
フォーカスごとに 1 回だけ発火。Win-held スキップ時は applied 非更新 → 次の refresh が
再試行）。非 TSF-native の Imm32Unavailable（Edge 等）は既存の hard pre-sync が
applied=true を立てるため従来どおり発火しない（VK_KANJI トグル安全性の設計を維持）。

**関連バグ:** BUG-07（focus 遷移系）、原因 3 の「conv confirm は open の証拠に
ならない」は BUG-08 追補2 と同根（IMC 読み値と実状態の乖離）。

**追補3（2026-07-08 実機ログ）: 4つ目の穴 — `Decision`/`Effect` 経由の SetOpen には
settle 明け再試行が一つも実装されていなかった。**
BUG-16 本文の3修正はすべて `Decision`/`Effect` を経由しない直接呼び出し
（`apply_force_on_for_imm_broken` 等）が対象で、`Engine::check_active_transition`
（`FocusChanged`/`RefreshState` で Active 遷移を検知した際に発行する通常の
`Effect::Ime(SetOpen)`）が settle 中に `executor::strip_ime_set_open_if_settling`
で握りつぶされるケースは対象外だった。

症状: UWP テキストフィールド（`Windows.UI.Input.InputSite.WindowClass`、TsfNative
プロファイル）にフォーカスが戻った直後、13.4 秒前の stale な `HwndCache` 復元で
belief が ON に戻り `Engine activated` がログされる。この遷移が発行する
`SetOpen(true)` は、同じフォーカス変更が張ったばかりの settle barrier のせいで
確実にストリップされる（barrier 生成 → 同一 tick 内で `check_active_transition`
評価という順序のため、この経路は原理的に毎回 settle 中に当たる）。ストリップ後、
`Engine::prev_activation` は既に Active へ確定済みのため、後続のどんな入力でも
同じ遷移は二度と検知されず `SetOpen` は自然には再発行されない。一方で
`GjiFsm`（GJI 用ステートマシン）の on/off は `on_ime_applied` 経由の apply 完了
通知でしか同期しないため `OffCold`（エンジン OFF 扱い）のまま固着する。
10 秒後にユーザーが「このせっけい」と入力すると、先頭の `StartComposition`
イベント（こ・の）が `[gji-fsm] StartComposition while engine off — ignored`
で無視され、`probe_io.rs` の raw-tsf-literal 検出が2回連続で発火して
「giving up ... no re-send」に到達し、当該文字が backspace のみで消えて
再送されずに欠落した（「このせっけい」→「せっけい」化）。

修正: `strip_ime_set_open_if_settling` が握りつぶした SetOpen の目標値を
`Option<bool>` で返すよう変更し（`#[must_use]`）、呼び出し元 2 箇所
（`Runtime::execute_decision` / `key_pipeline::kp_run_inner`）で `Some` を
受けたら本文と同じ確立済みパターン（`schedule_ime_refresh(focus_settle_ms + 50)`）
で settle 明け再試行をスケジュールするようにした。

**関連ファイル:** `runtime/executor.rs`（`strip_ime_set_open_if_settling` /
`execute_from_loop`）、`runtime/mod.rs`（`execute_decision`）、
`runtime/key_pipeline.rs`（`kp_run_inner`）

---

## BUG-17: CLSID ベース IME 種別の単発フリップで GjiFsm が丸ごと再構築され、Chrome 入力中に cold が単語ごとに発火し続ける

**症状:** Chrome（`Chrome_WidgetWin_1`、`Imm32Unavailable` プロファイル）で日本語を
連続入力しているだけなのに、単語間隔が `COMPOSITION_TIMEOUT_MS`（2000ms）を大きく
下回っているにもかかわらず `cold_seq` がほぼ毎単語インクリメントし続け、単語ごとに
`VK_IME_OFF→VK_IME_ON` 強制リセット + IMC ポーリング（`chrome-reinit`/`sacr-warmup`）が
繰り返される。2026-07-07 実機ログ（15:11:38.411〜15:11:44.848、"wo","sa","i","yo",
"mi","ko","mi","su/ru","ni","ha" の 9 単語）で `cold_seq` が 392→401 と単語ごとに
climb し、その間 2 回 `[gji-fsm] StartComposition while engine off — ignored`
（15:11:39.094 と 15:11:41.240、**間隔 2146ms**）が観測された。

**原因:** `GjiState::OffCold` に入る経路は `GjiFsm::new()`（新規生成）と
`GjiEvent::ImeOff`（`platform.rs::gji_on_ime_off`、`on_ime_applied(open=false)` 経由）
の 2 つのみ（`tsf/gji_fsm.rs`）。`tsf/gji_monitor.rs::monitor_loop` は
`ITfInputProcessorProfileMgr::GetActiveProfile` を **フォーカスを持たない
`gji-io-monitor` ワーカースレッド**から 2 秒間隔でポーリングし、前回値と異なる
瞬間に `TSF_OBS.set_tsf_active_kind()` → `WM_IME_KIND_CHANGED` を発行する。
受信側 `sync_ime_kind_from_observation`（`runtime/message_handlers.rs`）は
`Output::set_active_ime_kind()`（`output/tsf_warmup_coord.rs`）を呼び、これは
**種別が変わるたびに warmup 戦略（`GjiFsm`/`MsImeStrategy`）を無条件で新規生成**
する。新規生成された `GjiFsm` は必ず `OffCold` から始まるため、確立済みの
`OnWarm`/`OnComposing`（warm 状態）がその場で失われる。

2 回の "StartComposition while engine off" の間隔が CLSID ポーリング周期
（2000ms）とほぼ一致すること、この profile では Chrome cold-start reinit が
実 `VK_IME_OFF→VK_IME_ON` トグルを毎回実際に送信すること（`send_chrome_gji_reinit_and_poll`,
`output/probe_io.rs`）から、「cold reinit が実 IME トグルを送る → 別スレッドの
`GetActiveProfile` が一時的に別種別を誤検出 → `WM_IME_KIND_CHANGED` →
`GjiFsm` 再構築で warm 状態喪失 → 次の単語も cold → 再度 reinit → …」という
自己増幅ループが有力な因果と推定される。`GetActiveProfile` がなぜ一時的に
別種別を返すか（別スレッドの入力コンテキストの仕様上の限界か、実際の TIP
再ネゴシエーションか）は実機の `RUST_LOG=debug` ログ（`[tip-detect]` 系）で
未検証。BUG-09 で一度棄却された「per-thread `GetActiveProfile` 固着」仮説とは
別の症状・別の因果連鎖であり、単発フリップが `WM_IME_KIND_CHANGED` を経由して
**確立済みの composing/warm 状態を破棄する**という、BUG-09 修正後に残っていた
別の構造的弱点。

**修正:** `tsf/gji_monitor.rs` に `ImeKindDebounce` を追加。CLSID ポーリングで
新しい種別が観測されても、**同じ新種別が 2 tick 連続**（= 前回ポーリングでも
候補だった）で観測されるまでは `TSF_OBS` を更新せず `WM_IME_KIND_CHANGED` も
発行しない。単発フリップ（1 tick だけ別種別 → 次 tick で元に戻る）は候補が
クリアされて確定に至らず、`set_active_ime_kind` による破壊的再構築が起きなくなる。
実際の IME 切り替え（ユーザーが手動で GJI ↔ MS-IME を切り替える等）は最大 4 秒
（2 tick 分）で確定するため実用上の遅延は無視できる。

**再発防止テスト:** `tsf/gji_monitor.rs::ime_kind_debounce_tests`（単発フリップの
非確定・2 連続一致での確定・確定後の安定化を検証、Windows ターゲットでのみ
コンパイル対象のため `cargo test -p awase-windows --target x86_64-pc-windows-gnu`
が必要 — 本セッションでは cross-compile での `cargo check`/`cargo test --no-run`
成功とロジックの手動トレースで検証済み。実行確認は Windows 実機/CI 待ち）。

**残存リスク:** `GetActiveProfile` の誤検出が 2 tick（4 秒）以上持続する場合は
本修正でも `GjiFsm` 再構築を防げない。また、正当な種別変化であっても composing
中の warm 状態を無条件で破棄する設計自体（`set_active_ime_kind` の全再構築）は
温存されている — 根本対応には「進行中の composition を戦略切替の前後で引き継ぐ」
設計変更が必要だが、今回は自己増幅ループを断ち切る最小修正に留めた。

**関連ファイル:** `tsf/gji_monitor.rs`（`ImeKindDebounce`, `monitor_loop`）,
`output/tsf_warmup_coord.rs`（`set_active_ime_kind`）, `tsf/gji_fsm.rs`
（`GjiState::OffCold` の 2 経路）, `output/probe_io.rs`
（`send_chrome_gji_reinit_and_poll`）

---

## BUG-18: 無操作中の AppKind (TsfNative⇔Uwp/InputSite) 往復後、再開直後の入力が部分欠落する（修正済み）

**症状:** Chrome（`Chrome_WidgetWin_1`、GJI、`Imm32Unavailable` プロファイル）で
日本語入力中。2026-07-07 実機ログ（ローカル夜間、ログ内タイムスタンプは UTC
2026-07-08T03:11〜03:13）で、「この内容を」（romaji `ko/no/na/i/yo/u/wo`）と
入力したところ一部の文字が欠落した（ユーザー報告）。

タイムライン:
- `03:12:24.891` `IME OFF (key combo)` の後、この抜粋の終端（`03:13:54`）まで
  `Engine activated` ログが一度も出ていない（= awase Engine 側は inactive の
  ままだったはずの区間）。
- その約90秒間（`03:12:16`〜`03:13:46`）、`Hook watchdog: no activity for
  N ms` が継続的に出続けており、**ユーザーの実キー入力が無かったはず**の区間
  にもかかわらず、`AppKind changed: TsfNative → Uwp`
  （`Windows.UI.Input.InputSite.WindowClass`）/ `Uwp → TsfNative`
  （`Chrome_WidgetWin_1`）が複数回（少なくとも4回）発生し、そのたびに
  `HwndCache: restore [...] ime_on=... mode=ObservedRomaji` が走っている。
- `03:13:46.340` `FocusProbe +15ms: ime_on=true(shadow) mode=ObservedRomaji
  [ime=GoogleJapaneseInput ...]` — shadow 側は ON と認識。
- `03:13:46.574`〜`47.010` に `ko`/`no`/`na`/`i`/`yo` を送信。`47.021` に
  candidate SHOW #52 が出るが、直後 `47.027` に
  `[gji-fsm] StartComposition while engine off — ignored`。
- `47.120` `comp-probe partial-literal` → `47.254` `u` 送信 → `47.362`
  `comp-probe confirmed` → `47.363` `wo` 送信 → `47.440`
  `comp-probe partial-literal` に続けて
  `[raw-tsf-literal] cold=35 consecutive raw-tsf-literal (count=2) →
  giving up, backs=2 cleanup only (no re-send)` — **バックスペースで後始末は
  したが再送していない**。
- `49.234` にも再度 `[gji-fsm] StartComposition while engine off — ignored`
  が発生。

**原因（仮説・未確定）:** `src/engine/engine.rs::check_active_transition` は
`ctx.ime_on` 等から Engine の active/inactive を computed する。無操作中の
AppKind 往復（`runtime/focus_tracking.rs` の `AppKind changed` /
`focus/hwnd_cache.rs` の `HwndCache: restore`）のたびに `ime_on`/`intent` が
書き換わっており、実際にはユーザーが IME を再度 ON にしていないのに、キー
入力パイプライン側（`FocusProbe`）は `ime_on=true(shadow)` と判定して romaji
変換を継続実行した形跡がある。一方 `tsf/gji_fsm.rs::GjiFsm` は
`GjiState::OffCold` のままだった — つまり「shadow ime_on」と「`GjiFsm` の
engine 認識」の間に**不一致な期間**が生じ、その窓で最初の数文字の
`StartComposition` が `OffCold` のまま握りつぶされ、`LiteralDetect`
（`output/probe_io.rs`）が raw literal と判定してバックスペースのみで
再送しなかった、というのが最有力仮説。

直前に修正した BUG-17（`8d97e83`）は Chrome の CLSID/`GetActiveProfile`
単発フリップによる `GjiFsm` 再構築ループが原因だったが、今回のトリガーは
CLSID フリップではなく **`AppKind`（`TsfNative`⇔`Uwp`）の往復**であり、
別経路の可能性が高い。この `AppKind` 往復自体が**ユーザー操作なしで**起きて
いる点も未解明（`Windows.UI.Input.InputSite.WindowClass` 自体の automatic
focus churn か、GJI 候補ウィンドウの表示/非表示に伴う副作用か、切り分けに
`RUST_LOG=debug` の `[tip-detect]` 系ログが必要）。BUG-16（フォーカス遷移の
settle スキップで belief×実状態が乖離する）と同系統の「focus 遷移中に
shadow state と実 IME 状態がズレる」構造だが、今回は Engine 自体の
activate/deactivate ログまで巻き込んでいる点で BUG-16 の修正範囲でカバー
されていない可能性がある。

**追補（2026-07-08 実機ログ、原因確定・修正）:** 2026-07-08T03:21〜03:25 の
実機ログ（ユーザー報告「著しく不安定」）で同一パターンを再確認し、原因を確定した。

タイムライン（抜粋）:
- `03:22:35.829` `IME OFF (key combo)` → `tsf/gji_fsm.rs` の `GjiFsm` が
  `GjiState::OffCold` に入る（`GjiEvent::ImeOff`, gji_fsm.rs:588）。
- 以後 `AppKind changed: Uwp → TsfNative (class=Chrome_WidgetWin_1)` /
  `TsfNative → Uwp (class=Windows.UI.Input.InputSite.WindowClass)` が
  ユーザー操作なしに繰り返し発生し、`HwndCache: restore [...] ime_on=true`
  が毎回走る。
- Chrome (`Imm32Unavailable` プロファイル) 側に戻るたびに
  `runtime/focus_tracking.rs::on_focus_process_changed` の「Imm32Unavailable
  hard pre-sync」ブロック（VK_KANJI 二重送信防止のため `effective_open()==true`
  なら `mirror_applied_open(true, ...)` で belief 層の `applied` だけを
  直接 ON 確定させる箇所）が毎回発火する。
- `03:24:43.513` 頃、`[gji-fsm] StartComposition while engine off — ignored`
  が連発（本ログでは少なくとも8回）。

**確定した原因:** `mirror_applied_open` は `ImeModel`（belief 層）の
`applied` state のみを ON にする。`GjiFsm` への通知は
`Runtime::gji_on_ime_on`（`platform.rs:467`）経由でしか行われず、これは
実際に `on_ime_applied(open=true)`（executor の apply 完了時）からしか
呼ばれない。ところが hard pre-sync はまさに「実 apply をスキップして
belief だけ ON にする」ための経路なので、`gji_on_ime_on` が一度も呼ばれず、
直前の実 `IME OFF` で入った `GjiFsm::OffCold` がそのまま残留する。
`GjiEvent::FocusChange` も `OffCold` 中は no-op（gji_fsm.rs:600-605
`if !engine_on { return Response::consume(); }`）なので、AppKind 往復
（フォーカス変更）だけではこの残留状態から抜けられない。結果、belief 層は
「IME ON」を指すのに `GjiFsm` は `OffCold` のままという不一致期間が生じ、
その窓で送られた `StartComposition` が `gji_fsm.rs:753-756` で無条件に
`consume()`（=破棄）され、対応する文字が欠落する。

`Hook watchdog: no activity for Nms` が 30〜47 秒まで単調増加していたのは
実際のフリーズではない（watchdog 自身が `WM_TIMER` 経由でメッセージループ
から出ているため、ループが本当に止まればこのログ自体出なくなる）。単に
その間ユーザーの実キー入力が無かっただけで、無操作中に `AppKind` が
往復し続けていたことが本質。

**修正:** `on_focus_process_changed`（`runtime/focus_tracking.rs`）の
「Imm32Unavailable hard pre-sync」ブロックで `mirror_applied_open(true, ...)`
を呼ぶのと同じ条件下で、`tsf/observer::tsf_obs().active_ime_kind()` が
`GoogleJapaneseInput` の場合に限り `self.platform.gji_on_ime_on(mode)` も
呼ぶよう追加した。`runtime/message_handlers.rs::sync_ime_kind_from_observation`
が既に使っている「belief が ON なら `GjiFsm` にも `ImeOn` を通知する」と
同一パターンで、`GjiFsm` が既に `OffCold` でなければ `ImeOn` ハンドラ側で
no-op になる（gji_fsm.rs:558-565）ため副作用はない。

**テスト:** 本修正は `Runtime`/`WindowsPlatform`（実 HWND・hook・GJI IPC 依存）
の統合経路への配線であり、既存の golden テスト（`golden_scenarios.rs` 等）は
`ImeModel::reduce` のみを対象とする純粋関数テストで `GjiFsm`/`Runtime` 配線を
検証できない。[fix-requires-evidence](../.claude/rules/fix-requires-evidence.md)
に従い、golden テストの代わりに本追記で修正履歴を記録する。Windows 実機での
再現待ち（AppKind 往復自体を意図的に誘発する再現手順が未確立のため）。

**関連ファイル:** `crates/awase-windows/src/runtime/focus_tracking.rs`
（`on_focus_process_changed` の hard pre-sync ブロック）,
`crates/awase-windows/src/platform.rs`（`gji_on_ime_on`, `on_ime_applied`）,
`crates/awase-windows/src/state/platform_state.rs`（`mirror_applied_open`）,
`crates/awase-windows/src/runtime/message_handlers.rs`
（`sync_ime_kind_from_observation`、同型の既存パターン）,
`crates/awase-windows/src/tsf/gji_fsm.rs`（`GjiState::OffCold`,
`StartComposition`, `FocusChange`）

**関連バグ:** BUG-16（focus 遷移の belief×実状態乖離）, BUG-17
（CLSID フリップによる `GjiFsm` 再構築、直前修正・別経路）

---

## BUG-19: 一発だけのカタカナ conv 誤読を warmup が鵜呑みにし、GJI が実際にカタカナへ固定される（修正済み）

**症状:** Chrome/Edge (`Chrome_WidgetWin_1`、GJI、`Imm32Unavailable` プロファイル) で
日本語入力中、2026-07-08 実機ログ（ユーザー報告）で「これでいいかな」と入力したところ、
3通りの壊れ方が発生した: (a) 全部カタカナ化（「コレデイイカナ」）、(b) 先頭の "k" だけ
生のローマ字として残留（「kおれでいいかな」）、(c) 先頭の "ko" だけ生のローマ字として
残留（「koれでいいかな」）。ユーザーは同じ単語を壊れるたびに複数回打ち直しており、
ログ上に同一 romaji 列 `ko/re/de/i/i/ka/na` が短時間に複数回出現するのは内部の再送では
なくユーザー自身の打ち直しであることを確認済み。

**タイムライン（抜粋、2026-07-08T05:01〜05:02）:**
- `05:01:26` 前後、`AppKind changed: TsfNative → Uwp` / `Uwp → TsfNative` により
  `Chrome_WidgetWin_1`（メインコンテンツ）と `Windows.UI.Input.InputSite.WindowClass`
  （GJI 候補ポップアップ等）の間でフォーカスが往復（`FocusChange [20408→9668→20984]`)。
- `05:01:54.387` `[conv-mode] Hiragana/roma → ZenKata/roma (conv=0x0000001B)` —
  ユーザーが何もカタカナ変換操作をしていないのに conv mode がカタカナへ切り替わる。
- 以後 `[idle-conv-check] TsfNative: engine ON 同期 (conv=0x0000001B,
  reason=KatakanaShadowOff)` が `05:02:05.830` / `05:02:09.244` / `05:02:13.816` と
  約3.5〜4.5秒間隔で反復し、そのたびに `IME OFF (key combo)` → `Engine activated`
  の往復が発生する自己強化ループになっていた。

**原因（確定）:** `state/conv_mode.rs::ConvModeMgr::update_from_conv` は
`ImmGetConversionStatus` の raw 値を無条件に信頼し、変化があれば即座に確定していた。
一方 conv 読み取り自体（`ime.rs:423` `get_ime_conversion_mode_raw_timeout` は
`GetForegroundWindow()` 基準）は、フォーカスが `Chrome_WidgetWin_1` と候補ポップアップ
(`Windows.UI.Input.InputSite.WindowClass`) の間を往復する状況下で、一瞬だけ候補
ポップアップ側のコンテキストから誤ったカタカナ conv を拾い得る。この一発誤読が
`ConvModeMgr` に即座に確定されると、次の eager warmup（`output/mod.rs:590-620`
`send_eager_tsf_warmup`）が `self.conv_mode.get()` の charset を見て
`ZenkakuKatakana` 用の warmup キー（`VK_DBE_KATAKANA`, F1 系）を**実際に GJI へ
送信**してしまう。これにより一過性の誤読が GJI の**本当の**状態としてロックインされ、
以後の raw conv 読み取りは「本当にカタカナになった GJI」を正しく反映し続けるため、
単なる誤読では済まなくなる。GJI が実際にカタカナへ固定された結果、(a) 全文カタカナ化が
発生し、さらに `conv_classify.rs::classify_conv_transition` の `KatakanaShadowOff`
救済ロジックが shadow=OFF なタイミングで発火するたびに engine の IME OFF/ON を
往復させ、その往復のたびに生じる cold な再開窓で (b)(c) の先頭文字 literal 漏れが
誘発された（BUG-18 と同系統の「OffCold 残留窓での StartComposition 握りつぶし」）。

BUG-18（同じ AppKind 往復が引き金）とは異なり、こちらは **conv mode（文字種）** の
誤読が実際の IME 状態を書き換えてしまう経路であり、BUG-18 の修正（`f9b10ae`、
`GjiFsm` への `ImeOn` 通知同期）ではカバーされない別経路。

**修正:** `ConvModeMgr::update_from_conv` に、非カタカナ→カタカナへの遷移限定の
デバウンスを追加した（`crates/awase-windows/src/state/conv_mode.rs`）。「同一の
カタカナ値を2回連続で観測するまで `mode` を確定しない」という、BUG-17 の
`ImeKindDebounce`（`tsf/gji_monitor.rs`）と同一パターン。1回目の観測は
`katakana_candidate` に保持するのみで `mode`/`get()` は変更しない（＝eager warmup
はまだ古い確定値を見るため実際の VK 送信は起きない）。2回目に同じ値が来て初めて
確定する。間に矛盾する読み取り（元の charset に戻る等）を挟んだ場合は候補をクリアし、
再び「1回目」からやり直す。初回観測（`mode` がまだ `None`）はデバウンス対象外
（起動直後にカタカナ入力アプリへフォーカスした場合等の正当なケースを即反映するため）。

**なぜこの粒度で十分か:** 誤読は `GetForegroundWindow()` が一瞬だけ候補ポップアップ
側を指す間だけの一過性現象であり、次の読み取り（数百ms以内、typing 中は各キー入力
ごとに複数の呼び出し site から読まれる）では通常フォーカスが正しいウィンドウへ
戻っているため誤ったカタカナ値が連続することは稀。一方、本当にユーザーがカタカナへ
切り替えた場合は同じ値が繰り返し観測されるため、1読み取り分の遅延だけで正しく確定する。

**テスト:** `crates/awase-windows/src/state/conv_mode.rs` に5件のユニットテストを
追加（Linux でも `cargo test -p awase-windows --lib conv_mode` で実行可能・純粋関数）:
`single_spurious_katakana_reading_is_not_committed`,
`katakana_reading_confirmed_after_two_consecutive_observations`,
`intervening_reading_resets_katakana_candidate`,
`first_ever_observation_is_not_debounced_even_if_katakana`,
`non_katakana_transitions_are_unaffected`。Windows 実機（`GetForegroundWindow` の
実際の誤読挙動を含む）での再現待ち。

**関連ファイル:** `crates/awase-windows/src/state/conv_mode.rs`
（`ConvModeMgr::update_from_conv`, `katakana_candidate`）,
`crates/awase-windows/src/output/mod.rs`（`send_eager_tsf_warmup`、
`self.conv_mode.get()` を信頼する消費側）,
`crates/awase-windows/src/state/conv_classify.rs`（`KatakanaShadowOff` の
発火元。当初この経路は `conv_mode_changed: bool` を受け取るのみで raw conv を
直接再解釈しており無防備だったが、下記追補で対処済み）,
`crates/awase-windows/src/ime.rs`（`get_ime_conversion_mode_raw_timeout`、
`GetForegroundWindow()` 基準の読み取り — 読み取り自体は変更せず、
消費側のデバウンスで対処）

**関連バグ:** BUG-17（`ImeKindDebounce` と同一の「2 tick 連続確認」パターンを
conv mode 側に適用）, BUG-18（同じ AppKind 往復が引き金だが別経路・別修正）

**追補（同日、belief/engine-sync 経路も保護）:** 上記修正は `ConvModeMgr`
（warmup のキー選択・`ImmSetConversionStatus` 復元先）だけを保護しており、
`state/conv_classify.rs::classify_conv_transition`（`InputModeObserved` 経由の
belief 更新、および `KatakanaShadowOff`/`NativeToggleShadowOff` による engine
ON/OFF 同期）は raw `conv: u32` を直接 `ConvMode::from_u32` して再解釈しており、
同じ一発誤読に無防備なまま残っていた。実際の報告インシデントでこちらが発火
しなかったのは `effective_open=true`（打鍵中）という**たまたまのタイミング**に
よるもので、構造的な保護ではなかった。

`kp_stage_idle_conv_check`（`runtime/key_pipeline.rs`）の呼び出しを、raw `conv`
ではなく `ConvModeMgr::get()`（直前の `update_from_conv` 済みのデバウンス確定値）
を渡すよう変更し、`classify_conv_transition` の第一引数も `conv: u32` から
`cm: ConvMode` に変更した。これにより warmup 側と belief/engine-sync 側が
**文字通り同じ確定値**を参照するようになり、片方だけ保護されるという構造的な
ズレが解消された。`0f75b5b`（カタカナ+shadow=OFF+conv不変からの回復、
`katakana_shadow_off_conv_unchanged_still_recovers_engine` テストで固定）は、
GJI が本当にカタカナへ持続的に固定された場合は数百ms（1読み取り分）の遅延の後
`ConvModeMgr` が確定するため、従来通り機能する。

`restore_roman`（JISかな化検出）の ROMAN ビット判定も `conv & CONV_ROMAN_BIT`
から `!cm.romaji` に置き換え、`ConvClassifyFixture`/`tests/journals/*.json` の
JSON スキーマ（`conv: u32`）はそのまま維持し、リプレイ側
（`tests/journal_replay.rs`）で `ConvMode::from_u32(fixture.conv)` に変換して
から呼び出す形にした（このリプレイ基盤は conv ビット解釈ロジック自体の回帰検出が
目的で、デバウンスとの相互作用は対象外のため）。`conv_classify.rs` 内の
既存テスト（`classify()` ヘルパー経由・直接呼び出し双方、計28件）を機械的に
`ConvMode::from_u32(...)` でラップし直し、全件通過を確認済み。

**関連ファイル（追補）:** `crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_stage_idle_conv_check`）, `crates/awase-windows/src/state/conv_classify.rs`
（`classify_conv_transition` のシグネチャ）,
`crates/awase-windows/tests/journal_replay.rs`

**追補2（2026-07-08 別インシデントで再発、根治）:** 上記2件の修正（デバウンス +
確定値の一本化）を適用済みの状態でも、Chrome (`Chrome_WidgetWin_1`, `GJI`,
`Imm32Unavailable`) で同一の症状（`conv=0x0000001B` ZenKata 誤読 → engine
勝手に ON）が再発した（実機ログ 2026-07-08T09:07:03〜05）。今回はユーザーが
`IME OFF (key combo)` で明示的に OFF にした **1.6 秒後**に発火しており、
デバウンス自体は機能していた（`ConvModeMgr` の2回連続観測確定を経ていた）。

**根本原因:** `KatakanaShadowOff`/`NativeToggleShadowOff` は
`handle_engine_set_open(true)` → `write_set_open_request(true)` →
`ImeEvent::UserImeSetIntent { source: UserIntentSource::Command }` という、
物理キー押下による正規のユーザートグルと**同じ経路**を通っていた。`Command`
は削除されていない正規の `UserIntentSource` 値のため、`IntentSource::Recovery`
撤去（`6971168`）や `observation_source_guard` dylint では検知できない
「観測がユーザー意図を偽装する」経路になっていた。これにより conv ビットからの
間接推測（GJI 候補ポップアップ `Windows.UI.Input.InputSite.WindowClass` への
フォーカス flicker 起因）が `desired_open` を直接 `true` へ上書きし、
ユーザーの明示 OFF 意図（`last_intent=Some(false)`）を消し去っていた。

さらに、これは単に `.claude/rules/ime-belief-architecture.md` の
「Observer は `desired_open` を直接書き換えない」原則への違反であるだけでなく、
**BUG-20 が同日修正した drift correction（`check_drift_correction` /
`ir_apply_drift_correction`）を機能不全にする**副作用があった: `desired_open`
が `true` に上書きされると `check_drift_correction` から見て
「desired==observed（両方 true）で乖離なし」に見え、本来 `desired=false` を
正しく再送すべき drift correction が発火する前に判断材料そのものが消えていた。

**修正:** `KatakanaShadowOff`/`NativeToggleShadowOff` を `EngineSync::SetOpen`
から新設の `EngineSync::ReportOpenInference` に分離し、`desired_open` を
一切書き換えず `PlatformState::report_conv_open_inference()` 経由で
`ObserverReported { source: ObservationSource::ConvOpenInference,
confidence: Medium }` として記録するだけにした（engine を actuate しない）。
`ConvOpenInference` は `ConvBitsInference`/`GjiIoInference`（input_mode 専用、
`PerSourceObservations` に記録されない設計）とは別に、正式な open/close 観測
として `PerSourceObservations` に配線した。

実際の補正判断はこれで自動的に BUG-20 で修正済みの drift correction へ委譲
される。ただし `check_drift_correction` の `most_recent_trusted()` は
confidence の下限フィルタを持たないため（Low でも「他に観測が無ければ」採用
され得る）、明示的なユーザー意図が一度も無い（起動直後等）状態で
`ConvOpenInference` 単独が `desired_open` のデフォルト値を actuate してしまう
リスクが残る。これを塞ぐため `check_drift_correction` に source-aware gate を
追加した: `trusted.source == ConvOpenInference && explicit_intent.is_none()`
の場合は補正を発火させない。ユーザーの明示意図がある場合（今回の再発シナリオ）
はこの gate を素通りし、`desired`（ユーザーの意図した値）が正しく再適用される。

**テスト:** `state/observation_store.rs`・`state/ime_model.rs`（Linux で
`cargo test -p awase-windows --lib` 実行可能、純粋関数/reducer）に
`ConvOpenInference` の配線・`desired_open`/`last_intent` 非破壊を固定する
ユニットテストを追加。`state/platform_state.rs`（Windows 専用モジュールのため
Linux ではコンパイル検証のみ、`cargo build/test --target
x86_64-pc-windows-gnu` で確認）に `check_drift_correction`/
`report_conv_open_inference` のユニットテスト5件（明示意図一致時は即時補正・
明示意図なしでは補正しない・desired一致時は補正不要・max_age超過観測は無視、
等）を追加。`tests/architecture_guard.rs` に
`katakana_and_native_toggle_shadow_off_never_use_set_open`（`EngineSync::
SetOpen(ConvSyncReason::KatakanaShadowOff/NativeToggleShadowOff)` の組み合わせ
が本番コードに出現しないことを固定）と
`conv_open_inference_source_is_limited_to_report_and_gate`（`ObservationSource::
ConvOpenInference` の参照箇所を2箇所に固定）を追加。

**検証状況:** Linux 上での `cargo build -p awase-windows --target
x86_64-pc-windows-gnu` クロスコンパイルと `cargo test -p awase-windows`
(lib 126件・architecture_guard 10件・golden_scenarios 17件・journal_replay
1件・layer_boundary_guard 8件、全件 pass) を確認済み。`check_drift_correction`
は `platform_state.rs` が `#[cfg(windows)]` 限定のため Linux 上でユニット
テストを実行できず（`--target x86_64-pc-windows-gnu --no-run` でのコンパイル
確認のみ）。Windows 実機（Chrome + GJI、GJI 候補ポップアップへのフォーカス
flicker 再現）での動作確認は未実施。

**関連ファイル（追補2）:** `crates/awase-windows/src/state/conv_classify.rs`
（`EngineSync::ReportOpenInference`）,
`crates/awase-windows/src/state/ime_event.rs`
（`ObservationSource::ConvOpenInference`）,
`crates/awase-windows/src/state/observation_store.rs`
（`PerSourceObservations::conv_open_inference`）,
`crates/awase-windows/src/state/platform_state.rs`
（`report_conv_open_inference`, `check_drift_correction` の source-aware
gate）, `crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_apply_conv_engine_sync`）, `crates/awase-windows/tests/architecture_guard.rs`

**関連バグ:** BUG-20（同日修正、drift correction の OFF 方向修正がこの根治の
前提条件になった）, `.claude/rules/ime-belief-architecture.md`（Observer が
`desired_open` を偽装して書き換える禁止パターンの実例として追記候補）

**追補3（2026-07-09、増幅ループの実体を特定・部分対策、実機検証待ち）:**
上記の対策（デバウンス + 確定値の一本化）は「一発誤読を belief に確定させない」
ことは達成したが、**一度確定してしまった belief（誤りであれ正しいものであれ）を
cold warmup のたびに real IME へ再書き込みし続ける**という、別レイヤーの自己参照
ループが残っていた。`tsf/warmup/cold_warmup.rs::preamble()`（cold warmup のたびに
実行、＝`GjiFsm::FocusChange` の再突入のたびに実行され得る）と
`output/probe_io.rs::send_sacrificial_ime_off_on`（Chrome cold-start
sacrificial warmup 経由）が、`conv_mode.get()` を無条件に real IME へ
`set_ime_romaji_mode_with_target_async` で書き戻していた。フォーカスが
スプリアスに往復するたび（BUG-18 参照）にこの書き戻しが繰り返されるため、
一度誤って確定した belief が「本当の GJI 状態」としてロックインされ続ける
経路になっていた。

`state/conv_mode.rs::ConvModeMgr` に `needs_conv_restore_write` /
`mark_conv_restore_written` を追加し、**同じ `mode` に対する復元書き込みを
1回だけに制限**した（`mode` が本当に変化した場合は改めて書き込み可能）。
`ImC_CMODE` の ROMAN ビット確保のみ（`imm_conv_target` が `None` を返す
ケース）は対象外（誤った charset ビットを注入するリスクがなく、
`conv | ROMAN` は冪等なため）。`docs/adr/078-ime-mode-belief-desired-effective-constraint.md`
Phase 1a に相当する、スコープを絞った部分対策。

**テスト:** `state/conv_mode.rs` にユニットテスト4件追加
（`restore_write_not_needed_before_any_mode_is_confirmed`,
`restore_write_needed_once_then_suppressed_for_same_mode`,
`restore_write_needed_again_after_mode_genuinely_changes`,
`restore_write_unaffected_by_pending_katakana_candidate`）。Linux 上で
`cargo test -p awase-windows`（lib 138件・golden_scenarios 19件・
architecture_guard 10件・journal_replay 1件・layer_boundary_guard 8件、
全件 pass）と `cargo build/clippy -p awase-windows --target
x86_64-pc-windows-gnu`（warning ゼロ）を確認済み。**Windows 実機
（Chrome/Edge + GJI、フォーカス往復での再発有無）での検証は未実施。**

**関連ファイル（追補3）:** `crates/awase-windows/src/state/conv_mode.rs`
（`needs_conv_restore_write`, `mark_conv_restore_written`）,
`crates/awase-windows/src/tsf/warmup/cold_warmup.rs`（`preamble()`）,
`crates/awase-windows/src/output/probe_io.rs`
（`send_sacrificial_ime_off_on`）

**未対応（follow-up）:** `runtime/key_pipeline.rs` の Shift 解放復元経路
（BUG-15 関連）は、物理 Shift キー解放という genuine なユーザー操作起点
のため今回は対象外とした。`DesiredMode`/`EffectiveMode`/`ModeConstraint`
への型分割・トレイの明示的 intent 化・config1.db 対応は ADR-078 の
Phase 1b/2 として未着手。

**追補4（2026-07-11、実機ログで再発を確認・真の根本原因を特定・修正）:**
上記3件の対策を適用済みのビルド（`e7cc6d7` HEAD 相当）でも、Windows Terminal
（`CASCADIA_HOSTING_WINDOW_CLASS` + `Windows.UI.Input.InputSite.WindowClass`、
GJI、`TsfNative`）で通常の日本語入力中に再発した（ユーザー報告「半角空白が
消える」「awase も gji もカタカナになる」）。実機ログ（本セッションで共有、
2026-07-11T00:36〜00:49 に約10回発生）を解析した結果、**今回は一発誤読では
なく、`ConvModeMgr` の 2 回連続観測デバウンスを正規に通過して `mode` が
`ZenKata` へ確定していた**（`[conv-mode] カタカナ遷移候補観測 (1回目、
確定保留)` → 数百ms後に `[conv-mode] Hiragana/roma → ZenKata/roma` で確定、
というログが全 10 件で一致）。

**根本原因（確定）:** 確定後、`output/mod.rs::send_eager_tsf_warmup`
（composition-fsm の `EmitWarmup`、すなわち Enter/Space/Ctrl chord 等の
confirm-key・cold-mark のたびに呼ばれる、非常に高頻度な speculative
warmup 経路）が `conv_mode.get()` の charset を見て毎回無条件に実 VK
（`VK_DBE_KATAKANA` 系）を GJI へ送信していた。ログでは同一エピソード内で
`[tsf-eager-warmup] ZenKata warmup 送信` が 10〜20 秒間に十数〜20回超
連続で発生していた。この関数は BUG-19 の**原本の根本原因分析自体**が
名指ししていた箇所（"次の eager warmup（`output/mod.rs:590-620`
`send_eager_tsf_warmup`）が実際に GJI へ送信してしまう"）だが、追補1〜3
で導入された `ConvModeMgr::needs_conv_restore_write`/`mark_conv_restore_written`
（「同じ確定 mode への復元書き込みは1回だけ」のスロットル）は
`cold_warmup.rs::preamble()` と `probe_io.rs::send_sacrificial_ime_off_on`
の2箇所にしか配線されておらず、**この本命の関数だけが無防備なまま
残っていた**。一度きりの誤読（または本物のカタカナ入力）がデバウンスを
通過して確定した後、EmitWarmup が発火するたびに実 F1 キーが GJI へ
再送され続け、真にロックインされる自己増幅ループになっていた。

**修正:** `send_eager_tsf_warmup` の ZenkakuKatakana/HankakuKatakana/
ZenkakuAlpha/HankakuAlpha 分岐に `needs_conv_restore_write()` ガードを追加し、
実送信時に `mark_conv_restore_written()` を呼ぶよう変更（`crates/awase-windows/src/output/mod.rs`）。
既存の `cold_warmup.rs`/`probe_io.rs` と全く同じスロットル方式であり、
新しい仕組みは導入していない。Hiragana (F2) 分岐は既存の
`conv_target.is_none()` 除外と同じ理由（ROMAN ビット確保のみで冪等）で
対象外のまま。

**テスト:** スロットル本体（`ConvModeMgr::needs_conv_restore_write`/
`mark_conv_restore_written`）は既に `state/conv_mode.rs` の5件のユニット
テストでカバー済み（追補3で追加、Linux で `cargo test -p awase-windows --lib conv_mode`
実行可能）。今回の変更は既存プリミティブを新しい呼び出し箇所に配線した
のみで、`send_eager_tsf_warmup` 自体は実 `SendInput` を伴うため Windows
実機以外でのユニットテストは困難（`cargo build/test -p awase-windows
--target x86_64-pc-windows-gnu` でコンパイル確認・既存 lib 138件/
architecture_guard 10件が全件 pass することを確認済み）。**Windows 実機
での再発有無の検証は未実施（次回セッションでの確認事項）。**

**関連ファイル（追補4）:** `crates/awase-windows/src/output/mod.rs`
（`send_eager_tsf_warmup`）, `crates/awase-windows/src/state/conv_mode.rs`
（`needs_conv_restore_write`/`mark_conv_restore_written`、変更なし・既存
プリミティブを再利用）

**追補5（2026-07-11、ユーザー判断でカタカナ/英数追従そのものを実験的に無効化）:**
追補1〜4はいずれも「観測されたカタカナへ awase が追従して warmup キーを送る」
という設計自体は維持したまま、その追従の頻度・タイミングを調整する対症療法
だった。ユーザーは IME トレイからカタカナ/半角英数を手動選択したことが一度も
なく今後もその予定がないと明言（`927f2a2`/`109b4c9` が保護していたケースに
該当しない）。これを踏まえ、DIAG_DISABLE_PROACTIVE_TSF_WARMUP と同じ「実験用
診断フラグで丸ごと無効化し、実機で何が起きるか観察する」手法を適用した。

新設フラグ `tuning::DIAG_FORCE_HIRAGANA_CHARSET`（`true`）は、
`ConvModeMgr::effective_charset()` を新設し、これが有効な間は常に
`Charset::Hiragana` を返すようにした。以下 3 箇所を `effective_charset()`
経由に置き換え、charset 追従ロジックを丸ごと無効化する:

1. `output/mod.rs::send_eager_tsf_warmup`（eager warmup の charset 選択）
2. `tsf/warmup/cold_warmup.rs::preamble()`（`WarmupContext::charset` と
   `conv_target`、ImmSetConversionStatus 書き戻し先の両方）
3. `output/probe_io.rs::transmit_tsf`（F1 leading warmup 前置判断）

`ConvModeMgr::get()`/`update_from_conv()` 自体は無変更 — 観測・`[conv-mode]`
ログは通常通り継続する。「観測はするが行動には反映しない」形。

**テスト:** 既存 lib(138)/golden_scenarios(19)/architecture_guard(10)/
layer_boundary_guard(8)/journal_replay(1) 全件 pass、clippy(lib) warning
ゼロを確認済み。Windows実機での動作確認（カタカナ観測ログは出るが warmup
キー送信ログが一切出ないこと、実際にカタカナ入力が必要になった場合の
挙動）は未実施。

**関連ファイル（追補5）:** `crates/awase-windows/src/tuning.rs`
（`DIAG_FORCE_HIRAGANA_CHARSET`）, `crates/awase-windows/src/state/conv_mode.rs`
（`ConvModeMgr::effective_charset`）, `crates/awase-windows/src/output/mod.rs`,
`crates/awase-windows/src/tsf/warmup/cold_warmup.rs`,
`crates/awase-windows/src/output/probe_io.rs`

---

## BUG-20: ドリフト補正の再送が non-ImmCross アプリで no-op のため IME ON / Engine OFF が固定化する（修正済み・実機検証待ち）

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS`、`TsfNative` プロファイル）・
Chrome（`Chrome_WidgetWin_1`、`TsfNative`/`Imm32Unavailable` プロファイル）で GJI
(Google 日本語入力) を使用中、2026-07-08 実機ログ（ユーザー報告）で「IME ON Engine OFF
の状態になった」。Ctrl+無変換 で IME OFF コンボを送信すると awase Engine 側は即座に
内部状態を非活性化する（`Engine::build_ime_set_open_decision` の設計上の楽観的自己遷移、
`src/engine/engine.rs:429-447`）が、Windows IME 側の表示は ON のまま変わらず、
07:41:56〜07:42:06 の約10秒間に `IME OFF (key combo)` → `Engine activated` の反復が
4回発生した（ユーザーが直らないため無変換キーを何度も押し直した痕跡と推定）。

**原因（確定）:** `crates/awase-windows/src/runtime/ime_refresh.rs`
`ir_apply_drift_correction()` は `desired`（awase が望む IME 状態）と `observed`
（実観測、GJI I/O 等から得る）が `DRIFT_CORRECTION_THRESHOLD_MS`（400ms）以上乖離すると
再送を試みる。しかし従来の実装は乖離の方向によらず常に
`self.platform.set_ime_open(desired)`（`platform.rs:670-686`）を呼んでいた。この関数は
`can_use_imm32_cross_process()`（`AppImeProfile::Standard` のみ true）が false のとき
即座に `false` を返す no-op であり、GJI/TsfNative（Windows Terminal・Chrome 等）では
**常に no-op** になる。にもかかわらず戻り値を見ずに `mirror_applied_open_with_ts` で
belief を無条件に「反映済み」とマークしていたため、`[drift] correction:` ログは出力
されるがOSには一切届いていなかった。

ON 方向には対称の実装（`apply_force_on_for_imm_broken`、`runtime/mod.rs:445-521`）が
既にあり、non-ImmCross プロファイルでは strategy chain 経由の `apply_ime_open_with_belief`
（実 VK 送信、GjiDirect/MsImeDirect 等）で確実に force-ON していた。旧
`ir_apply_drift_correction` 直上のコメントには「Blacklist アプリは
`apply_force_on_for_imm_broken` が担当するため除外」とあったが、これは ON 方向のみを
指しており、**OFF 方向の対称実装が存在しなかった**ことが見落とされていた。

**修正:** `ir_apply_drift_correction` に `can_use_imm32_cross_process()` による分岐を
追加。ImmCross 対応アプリは従来通り `set_ime_open`。non-ImmCross では
`apply_force_on_for_imm_broken` と同じ `platform.apply_ime_open_with_belief()` +
`on_ime_apply_complete()`（generation 照合込みの belief 書き戻し）を使う。

**関連ファイル:** `crates/awase-windows/src/runtime/ime_refresh.rs`
（`ir_apply_drift_correction`）, `crates/awase-windows/src/runtime/mod.rs`
（`apply_force_on_for_imm_broken`、`on_ime_apply_complete`、参照実装として流用）,
`crates/awase-windows/src/platform.rs`（`set_ime_open`, `apply_ime_open_with_belief`）,
`crates/awase-windows/src/state/platform_state.rs`（`check_drift_correction`）

**検証状況:** Linux 上で `cargo build -p awase-windows --target x86_64-pc-windows-gnu`
のクロスコンパイルと `cargo test -p awase-windows`（`golden_scenarios` /
`architecture_guard` / `layer_boundary_guard` / `journal_replay`）の既存回帰なしを
確認済み。`ir_apply_drift_correction` の先（`ime_controller::CONTROLLER.apply`）は
`SendInput`/`ImmSetOpenStatus` 系の `unsafe` Win32 API に直結し注入用シームがないため、
Linux 上でのユニットテストは書けない（`ime_key_sequence_golden.rs` と同じ制約）。
実機（Windows Terminal/Chrome + GJI）での動作確認は未実施。

---

## BUG-21: Chrome の cold-start 復帰処理が重症度 (Short/Medium/Long) を無視し、確定キー/IME再有効化のたびに過剰発火する

**症状:** Chrome（`Chrome_WidgetWin_1`、`Imm32Unavailable` プロファイル）で GJI を使い
日本語を連続入力しているだけなのに、単語の区切り（確定キー相当の操作）や
Ctrl+無変換 での IME OFF→ON 再有効化のたびに `cold_seq` がインクリメントし、
`[sacr-warmup] cold=N Chrome reinit: IME Hiragana 確認 → 再送` が数秒に1回のペースで
発生する。2026-07-09 実機ログ（06:27:42〜06:28:22 の約40秒間）で `cold_seq` が
349→362 と 13 回発火し、うち大半が `sacr-timeout`（VK_A probe で warm 未確認 →
`VK_IME_OFF→VK_IME_ON` reinit 実行）だった。cold 1回につき VK_A+BS（probe 用犠牲キー）
+ cleanup BS×1 + （reinit が必要な場合）VK_IME_OFF/ON×2 相当の合成キーが余分に注入され、
ユーザーからは「cold-start の発火頻度が高すぎる」「BS の回数が多すぎる」と報告された。

BUG-17 と症状が類似するが原因は別。BUG-17 は `WM_IME_KIND_CHANGED` 経由の `GjiFsm`
丸ごと再構築が引き金だが、本バグは **正規の** IME OFF/ON トグル・確定キー(Space/Enter/Esc)
操作が引き金であり `[tip-detect]` ログは介在しない。

**原因:** `GjiFsm` は cold を `ColdKind::Short`/`Medium`/`Long`（`gji_idle_ms` から
`ColdKind::classify` が判定、`tsf/gji_fsm.rs`）に正しく分類している。WezTerm/TSF 側の
復帰処理 `GjiWarmupCoro`（`tsf/warmup/gji_warmup_coro.rs:273`）はこの重症度を見て
`ctx.is_long_cold` のときだけ VK_A probe + sacrificial warmup のフルコースに分岐し、
Short/Medium cold は軽量な inline LiteralDetect のみで済ませていた。

一方 Chrome 側の復帰処理 `TsfProbeCoro::new_chrome`
（旧: `tsf/warmup/probe_fsm.rs`）は `ColdKind` を一切受け取っておらず、
`tsf_probe_coro_body` の Phase 2a は `env.gji_active` が true であれば cold の重症度に
関わらず常に `StartSacrificialWarmup`（VK_A+BS probe → 未確認なら
`VK_IME_OFF→VK_IME_ON` reinit + IME確認ポーリング → cleanup BS → 再送）を実行して
いた。さらに、確定キー(Space/Enter/Esc) は `composition_fsm.rs::ConfirmKeyDown` が
warm/cold を問わず常に `GjiCompositionReset`（`handle_composition_reset()` →
強制的に `OnCold(Short)`）を emit しており、`ImeOn`（`OffCold` から）も
「即入力する意図があるため」常に `transition_to_cold_proactive` で cold へ遷移する
（これ自体は `8715731a2` で修正した実バグの再発防止であり妥当）。つまり **cold の
判定自体は正しい** — 確定キーや短い IME OFF/ON のたびに Short/Medium cold へ入るのは
意図通り。バグは **Chrome の復帰処理がこの重症度情報を捨てて毎回 Long cold 相当の
最重量パスを踏んでいた**こと。

**修正:** `send_romaji_batched`（`output/vk_send.rs`）が既に計算していた
`long_idle`（`idle_ms_at_last_cold() > CHROME_LONG_IDLE_MS`）/ `f2_gji_long_idle`
を `is_long_cold` として `ChromeProbe::new` / `TsfProbeCoro::new_chrome`
（`tsf/warmup/chrome_probe.rs`, `tsf/warmup/probe_fsm.rs`）に渡すよう変更。
`tsf_probe_coro_body` の Phase 2a を `env.gji_active && is_long_cold` のときのみ
`StartSacrificialWarmup` に分岐するよう変更し、それ以外（`!gji_active` または
Short/Medium cold）は `Transmit(needs_literal: env.gji_active)` で直接送信しつつ、
`gji_active` なら inline LiteralDetect（Phase 3）を安全網として残す。WezTerm 側
`GjiWarmupCoro` の `is_long_cold` 分岐と対称にした。

合わせて `composition_fsm.rs::ConfirmKeyDown` の `if warm && tsf_mode` を `if warm` に
変更（Chrome にも TSF と同じ「warm なら warmup を KeyUp まで遅延」を適用）。これは
`a3425bf`（2026-05-13、フラグ統合コミット）で WezTerm 専用ルール（`f58b47c`
導入の F2/Enter 競合対策）が `is_tsf_mode()` ガードなしで Chrome に引き継がれた
副作用で、Chrome 固有の根拠は見つからなかった。ただし `GjiCompositionReset` 自体は
両分岐で変わらず emit されるため、この副修正は warmup 送信タイミングの改善に留まり、
今回の主因（Chrome 復帰処理の重症度無視）の修正ではない。

**再発防止テスト:**
`tsf/warmup/probe_fsm.rs::tests::chrome_short_cold_skips_sacrificial_warmup` /
`chrome_long_cold_still_uses_sacrificial_warmup` /
`chrome_short_cold_without_gji_active_skips_literal_detect`（`TsfProbeCoro::new_chrome`
を `is_long_cold` 別に直接 tick して emit される `ProbeAction` を検証）、
`tsf/composition_fsm.rs::tests::warm_chrome_confirm_keydown_defers_warmup_to_keyup`。
いずれも Windows ターゲットでのみコンパイル対象のため
`cargo test -p awase-windows --target x86_64-pc-windows-gnu` が必要。本セッションでは
Linux 上でのクロスコンパイル成功（`cargo test`/`cargo clippy -- -D warnings` とも
変更ファイルにエラーなし）とロジックの手動トレースで検証済み。wine 等の実行環境が
無いためテスト実行そのものは未実施（実機/CI 待ち）。

**残存リスク:** `is_long_cold` の閾値は `CHROME_LONG_IDLE_MS`（既存の 5000ms、
Chrome 固有の実測に基づく）をそのまま流用しており新規のタイミング定数追加はない。
Short/Medium cold で `StartSacrificialWarmup` を省略した結果、まれに Chrome の
composition context が実際に未初期化のまま送信され `RawTsfLiteralRecovery`
（BS 再送によるリカバリ）の発火頻度が増える可能性がある — その場合は
`[raw-tsf-literal] cold=N raw TSF literal suspected` の頻度を実機ログで確認し、
Medium cold まで `is_long_cold` 相当に含めるかを再検討する。

**関連ファイル:** `crates/awase-windows/src/tsf/warmup/probe_fsm.rs`
（`tsf_probe_coro_body`, `TsfProbeCoro::new_chrome`）,
`crates/awase-windows/src/tsf/warmup/chrome_probe.rs`（`ChromeProbe::new`）,
`crates/awase-windows/src/output/vk_send.rs`（`send_romaji_batched`, `is_long_cold` 算出）,
`crates/awase-windows/src/tsf/composition_fsm.rs`（`ConfirmKeyDown`）,
`crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs`（対称な WezTerm 側実装、参照）,
`crates/awase-windows/src/tsf/gji_fsm.rs`（`ColdKind::classify`, `transition_to_cold_proactive`）

**追記（2026-07-18）: `is_long_cold` 分岐・`StartSacrificialWarmup` フルコース自体を
物理削除。** BUG-24 の per-VK confirm（1文字ずつ送信→confirm、失敗時は backspace の
み、捨て駒キーには頼らない）が実機で安定稼働することを確認した後、「送信前に GJI
準備を待つ」予防機構（本 BUG が扱っていた `is_long_cold` 重症度分岐を含む）自体が
per-VK confirm と二重の保険になっているという仮説を `experiment/skip-cold-probe-wait`
ブランチで検証した。実機ソーク数日（cold=61〜74 超、WezTerm/Chrome 双方）で
`suspected literal` が genuine にゼロ件（チェック自体は毎回走って毎回パスしている
ことをログで確認済み、素通りではない）となり、無破損を確認できたため、
`TsfProbeCoro::new_chrome` の `is_long_cold` パラメータ・Phase 2a
（`StartSacrificialWarmup` 分岐）・`SacrificialWarmupCoro`/`ImeOffOnWarmupFsm`
自体を撤去した。本 BUG エントリの「原因」節が説明する重症度分類の**判断**
（`ColdKind::Short`/`Medium`/`Long`）自体は TSF/WezTerm 側 `decide_transmit_plan`
の `is_long_cold` 引数として現役だが、Chrome 側のこのバグが扱っていた分岐先
（フルコース vs 軽量パス）という区別自体が意味を失った（フルコースが無くなった
ため）。上記「再発防止テスト」に列挙した3つの `is_long_cold` 別テストは
`chrome_gji_active_enters_per_vk_confirm_as_safety_net`
（`literal_session_confirmed` のグローバル状態リークを避けるため
`reset_literal_session_confirmed()` を追加）と
`chrome_without_gji_active_skips_literal_detect` に整理統合した。詳細は
BUG-24 の追補7以降を参照。

---

## BUG-22: MS Edge で Uwp⇔TsfNative フォーカス往復後、conv=Eisu(英数) に固着し nicola が入力できなくなる

**症状:** 2026-07-09 実機ログ。MS Edge（`Chrome_WidgetWin_1`、`Imm32Unavailable` プロファイル）で
IME・conv モードは MS-IME。無操作でしばらく放置した後、Edge の親ウィンドウ
（`Chrome_WidgetWin_1`）とその内部 IME 入力ウィンドウ（`Windows.UI.Input.InputSite.WindowClass`、
`Uwp` 扱い）の間でフォーカスが何度も往復し（ユーザー操作なし）、その後 Edge にひらがなで
入力しても `Engine deactivated (reason=Inactive(NotRomajiInput))` のまま活性化せず、
`FocusChanged: input_mode スキップ (belief=ObservedEisu, eisu guard)` が繰り返し出力されて
入力を受け付けなくなった。

**原因（2つの独立した設計不備の重なり）:**

1. `apply_hwnd_cache_restore`（`state/platform_state.rs`）が `HwndImeCache::restore`
   （`focus/hwnd_cache.rs`、TTL `HWND_CACHE_MAX_AGE_MS`=1時間）で取得した
   スナップショットの `input_mode` を、鮮度・confidence チェックなしに
   `ImeEvent::InputModeApplied { strategy: CacheRestore, .. }` として無条件適用していた。
   131 秒前に保存された stale `ObservedEisu` がそのまま復元され、`correction_for_imm_broken`
   （`ObservedEisu` は意図的に対象外 — 受動的経路がユーザーの英数選択を踏み潰さないため）
   では訂正できず、engine が inactive のまま固着した。
2. `eisu_reset_on_ime_on`（`state/eisu_recovery.rs`）は OFF→ON 遷移でのみ発火するため、
   IME が既に open（MS-IME は常時 open のことが多い）な状態でユーザーが TurnOn 系キー
   （ひらがな/かな 等、`ShadowImeAction::TurnOn`）を押しても遷移が起きず、
   `kp_stage_shadow_ime_toggle` の no-op 分岐（`effective_open() == current`）で
   握りつぶされ、手動での復帰手段が構造的に存在しなかった。

**修正:**

1. `state/eisu_recovery::cache_restore_eisu_guard(cached_mode)` を新設。
   `apply_hwnd_cache_restore` はキャッシュ復元前にこの関数を通し、`ObservedEisu` のみ
   `AssumedRomaji { AppKindExcluded }` に倒す（他モードはそのままキャッシュ値を信頼）。
2. `state/eisu_recovery::eisu_reset_on_turn_on_while_open(action_is_turn_on, mode)` を新設し、
   `InputModeApplyStrategy::UserTurnOnEisuReset` として `kp_stage_shadow_ime_toggle` の
   no-op 分岐に配線。`ShadowImeAction::TurnOn` 受信時に belief が `ObservedEisu` なら
   `AssumedRomaji` へ訂正する（OFF→ON 遷移を必要とする `UserImeOnEisuReset` と対になる、
   「IME が既に open」ケース専用の救済）。

`state/eisu_recovery.rs` の module doc の経路×救済対応表に4行目として追記し、
`tests/architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset` /
`input_mode_applied_construction_sites_are_accounted_for` の期待値を更新。

**再発防止テスト:** `state/eisu_recovery.rs` の単体テスト
（`cache_restore_guard_corrects_stale_eisu` 等 4件）、
`tests/golden_scenarios.rs::scenario_13_hwnd_cache_restore_does_not_reinject_stale_eisu` /
`scenario_14_turn_on_while_open_recovers_stale_eisu`（いずれも Linux 上で
`cargo test -p awase-windows` により実行・グリーン確認済み）。
`tests/architecture_guard.rs` の全 10 テストもグリーン。Windows 実機での再現確認は未実施。

**関連:** BUG-18（無操作中の AppKind Uwp⇔TsfNative 往復、文字欠落）と発生源
（無操作時のフォーカス往復）は共通だが、下流の壊れ方が異なる別バグとして扱う。
2026-07-06 の「ObservedEisu 循環デッドロック」修正（`f9f070e`/`1b61efe`、
`UserImeOnEisuReset` / `GjiIoInference` 救済追加）でカバーしていなかった2経路
（キャッシュ復元経路、IME open のまま TurnOn キーを受けるケース）の追補。

**関連ファイル:** `crates/awase-windows/src/state/eisu_recovery.rs`,
`crates/awase-windows/src/state/platform_state.rs`（`apply_hwnd_cache_restore`）,
`crates/awase-windows/src/runtime/key_pipeline.rs`（`kp_stage_shadow_ime_toggle`）,
`crates/awase-windows/src/focus/hwnd_cache.rs`,
`crates/awase-windows/src/state/ime_event.rs`（`InputModeApplyStrategy`）

---

## BUG-23: 画面ロック中に離された修飾キーの KeyUp が失われ、Shift/Ctrl が恒久的に stuck する（修正済み・実機再現確認待ち）

**症状:** 2026-07-09 実機ログ。何もしていない（あるいは離席してロック画面になっていた）
状態から復帰後、Shift/Ctrl を単体で押して離しても `[engine-input]` の
`mods(c=... s=...)` が `true` のまま戻らなくなる。ユーザー体感は「Caps Lock が
ON になったような状態」（打鍵が意図しない大文字/記号として出力される）。
既存の自己診断ログも発火する:

```
[engine-input] CTRL MISMATCH: mods.ctrl=false だが phys_ctrl=true (vk=0xA0 KeyDown)
→ synthetic Ctrl↑ が GetAsyncKeyState を汚染した可能性がある
```

**原因（確定）:** `hook.rs` の `PHYSICAL_KEY_STATE`（VK ごとの物理押下状態、
non-injected な KeyDown/KeyUp でのみ更新）は、`observer::focus_observer::read_os_modifiers()`
で左右のキーを OR 演算して合成される（`shift = is_physical_key_down(VK_LSHIFT) ||
is_physical_key_down(VK_RSHIFT)`）。実機ログで `19:53:07` に `vk=0xA1`（右Shift）の
KeyDown だけが記録され、以降一度も対応する KeyUp が現れないことを確認した。

Windows がロック画面（Secure Desktop）に遷移している間、通常デスクトップに
インストールされた `WH_KEYBOARD_LL` フックはその間のキーイベントを一切観測できない。
ロックの瞬間に修飾キーが押されていた（あるいは離席中の誤タッチ等）場合、KeyDown は
ロック前に捕捉されても対応する KeyUp がロック中に発生し、フックに届かないまま
`PHYSICAL_KEY_STATE` がその VK だけ `true` に stuck する。OR 合成のため、以後
反対側のキーを正しく押して離しても複合された `mods.shift`/`mods.ctrl` は
恒久的に `true` のまま戻らない。

**副次的に発覚した既存の隙間:** ADR-052 の `panic_reset()` は stuck modifier からの
回復を想定して `send_all_modifier_key_ups()`（`SendInput` で全修飾キーの KeyUp を送信）
を実行するが、これは自己注入（`dwExtraInfo=INJECTED_MARKER`）のため `hook_callback` の
`is_self_injected` フィルタ（ADR-054、VcXsrv 由来の stuck Ctrl 対策として後から追加）に
弾かれ、`PHYSICAL_KEY_STATE` の更新まで到達しない。OS 側の modifier は解放されるが
awase 内部の物理キー shadow は解放されないままだった（panic_reset が本来意図していた
動作を ADR-054 が意図せず壊していた regression）。

**修正:** `hook::reset_physical_key_state()`（`PHYSICAL_KEY_STATE` /
`PHYSICAL_KEY_DOWN_AT_MS` の全 256 VK スロットを無条件でクリア）を新設し、以下 2 箇所
から呼ぶ:

1. `runtime/message_handlers.rs::handle_wts_session_change` の `WTS_SESSION_UNLOCK`
   分岐（根本原因への対処。アンロック時点では物理キーはどれも離されていると
   仮定してよい）
2. `runtime/mod.rs::panic_reset()`（`send_all_modifier_key_ups()` の直後。ADR-052 が
   意図していた stuck modifier 回復を実際に機能させる）

トレイメニューの「内部状態をリセット」は既に `WM_PANIC_RESET` → `panic_reset()` と
同一経路（ADR-052）のため、追加の配線なしで同じ修正が適用される。

**テスト:** `hook.rs` は Windows 専用 API に依存しクロスコンパイルのみ
（`cargo build -p awase-windows --target x86_64-pc-windows-gnu` で確認済み、
wine 環境が無いため実行は未実施）。`reset_physical_key_state()` 自体は単純な
atomic 全クリアのため単体テストの価値は低いと判断し、known-bugs.md への本追記で
[fix-requires-evidence](../.claude/rules/fix-requires-evidence.md) の記録要件を満たす。
Windows 実機でのロック→アンロック再現待ち。

**関連ファイル:** `crates/awase-windows/src/hook.rs`（`PHYSICAL_KEY_STATE`,
`reset_physical_key_state`）, `crates/awase-windows/src/runtime/message_handlers.rs`
（`handle_wts_session_change`）, `crates/awase-windows/src/runtime/mod.rs`
（`panic_reset`, `send_all_modifier_key_ups`）,
`crates/awase-windows/src/observer/focus_observer.rs`（`read_os_modifiers`）

**関連 ADR:** ADR-052（トレイパニックリセット）, ADR-054（PHYSICAL_KEY_STATE
injected フィルタ、VcXsrv 由来 stuck Ctrl 対策 — 今回発覚した regression の導入元）

---

## BUG-24: `is_partial_literal()` が romaji 自体の compose 結果ではなく warmup F2 への
応答を代理指標にしており、偽陽性（正しい文字の誤削除）・偽陰性（部分リテラルの
検知漏れ）の両方を構造的に許容している（未修正）

**症状（偽陽性、実例あり）:** `gji_warmup_coro.rs:232-237` のコメントに記録済み。
Enter/Space 等の確定キー操作後、WezTerm では正しく `composited 'な'` として
compose されているのに、`nc_fired=false`（fresh F2 warmup キー自体への
NAMECHANGE 応答が確認できなかった）が真になり `is_partial_literal()` が
誤って true と判定、正しく確定した 'な' が backspace で消される事故が
実際に発生した。対策として `is_confirm_key && is_tsf_mode` の場合のみ
`nc_fired` を強制的に true へ昇格し、この特定条件下での誤検知を抑制する
ピンポイント修正が入っている（`gji_warmup_coro.rs:237`）。

**症状（偽陰性、疑いのみ・未確認）:** `needs_literal=false` と判定されて
`LiteralDetect` フェーズ自体がスキップされた場合、実際には部分リテラルが
発生していても検知されず放置される可能性が疑われている。開発者自身が
この疑いを認識し、`gji_warmup_coro.rs:313-333` に fire-and-forget の
非同期診断ログ（`[gji-coro-diag] ... skip-verify`）を仕込んで事後確認を
試みているが、**この診断ログの出力を実際に分析した記録は一切なく、
偽陰性が本当に起きているかどうかは未確認のまま**である。

**原因:** `is_partial_literal()`（`tsf/warmup/literal_detect_fsm.rs:53-62`）は
`nc_fired`（fresh F2 warmup キー自体への NAMECHANGE 応答があったか）と
`gji_resumed`（F2×2 後に GJI の I/O が応答したか）を代理指標に使っているが、
これらは **送信した romaji 自体が実際に compose されたかどうかとは別の、
warmup 用の F2 キーへの応答の有無**でしかない。「warmup 確認信号が
期限内に届かなかった」ことと「実際に IME が未初期化だった」ことは
論理的に別の主張であり、確認信号が単に遅かっただけ（TSF-native アプリの
HIMC=NULL 制約により、実際の compose 結果を直接読む代替手段が存在しない
ことは ADR-078 前後の調査で確認済み）のケースを「部分リテラル」と
誤診断してしまう構造になっている。

`is_confirm_key && is_tsf_mode` の昇格修正でカバーされているのは確定キー
経由の cold のみで、少なくとも以下 2 経路は同種の偽陽性リスクを未パッチ
のまま残している（2026-07-10 調査）:

1. **`gji_candidate_visible` 早期脱出**（`gji_warmup_coro.rs:176-182`）:
   NameChangeWait 中に候補ウィンドウが既に見えていれば
   `break 'ncwait (false, false)` で即 transmit へ抜けるが、候補が
   見えている状況はむしろ compose が正常進行中である可能性が高い。
   確定キー経由でなければ昇格修正の対象外。
2. **NameChangeWait タイムアウト**（`gji_warmup_coro.rs:184-188, 222`）:
   `nc_fired_now=false && timed_out=true` でも `break 'ncwait
   (nc_fired_now, gji_wrote_after_f2)` に落ちる。WezTerm の UIA
   NAMECHANGE イベントが単に遅延・座標イベントと合流しただけで、
   実際の compose 自体は成功しているケースを排除できていない。

さらに **pre-idle スキップ**（`gji_warmup_coro.rs:134-151`）には、コード
自身のコメントに「GJI が実際には数百 ms 後に応答するケースでは partial
literal の疑い経路を直接誘発しうる」と、既知のリスクとして明記された
まま放置されている箇所もある。

**なぜ偽陰性が実害として顕在化していないと考えられるか（2026-07-10、
ユーザー仮説）:** 現状は cold-start 予防（warmup）が広く・保守的に
かかっているため `needs_literal` がほぼ常に true になり、`LiteralDetect`
自体がスキップされるケースが実運用でほとんど発生していない可能性が高い。
予防のタイミング・適用範囲を絞り込んだ場合に、この偽陰性が顕在化する
可能性がある。**実機での検証でしか確認できない**（Linux 環境では
wine 不在のため実行不可）。

**実機検証（2026-07-11、`DIAG_DISABLE_PROACTIVE_TSF_WARMUP`）:**

WezTerm/TSF 側の3防御層すべて（Phase 2 `SendFreshF2`、Phase 5a
`StartSacrificialWarmup`、`effective_prepend_f2` のバッチ同梱）を診断フラグ
`DIAG_DISABLE_PROACTIVE_TSF_WARMUP`（`tuning.rs`）で無効化し、Windows Terminal
（`CASCADIA_HOSTING_WINDOW_CLASS`）で実機タイピングして予防をなくした状態での
挙動を観測した。

- **`reason=SetOpenTrue`（エンジン再有効化直後、`real_gji_idle_ms` 282〜1188ms）**:
  観測した全件（`cold=1,10,11,12,14`）で、`romaji="ko"`（歴史的な"kお"バグの
  典型パターン）送信後に `is_partial_literal()` が正しく `partial literal` を
  検知し、ESC-based 回収（`4e31b64`、`VK_ESCAPE` + `BS×1` + 再送）が正しく機能
  して文字化けを免れた。予防ゼロでも reactive 検知だけで実害を防げることを
  実機で確認できた。
- **`reason=ReinjectConfirmKey`/`CtrlKeyBypass`（`nc_fired=true`、
  `cold=5,6,9,13`）**: `effective_prepend_f2` を強制 false にした結果
  `needs_literal=false` となり、`LiteralDetect` 自体が一切起動しなかった。
  目視確認の結果、この瞬間の出力（"さ"/"に"）はローマ字のまま残っておらず
  正しく変換されていた——`nc_fired=true` という判定自体がこのケースでは実態と
  一致していたことを示唆する。これは「`nc_fired=false` なのに実際は暖まって
  いた」（"な" バグ）とは逆方向であり、偽陰性の証拠にはならない。

**現時点の結論:** 今回の実機検証（1セッション、上記件数）の範囲では偽陰性の
実害は観測されなかった。ただし条件（より長い idle、他の cold_reason、他アプリ、
複数セッション）を広げていない段階のため、BUG-24 の理論的懸念自体が否定された
わけではない。ユーザーの判断で `DIAG_DISABLE_PROACTIVE_TSF_WARMUP=true` の
まま実運用を継続し、より広い条件下で問題が顕在化するか追加検証中
（2026-07-11〜、進行中）。

**改善オプション（実現性順、2026-07-10 調査、いずれも未実施）:**

1. **昇格条件の横展開（低コスト・対症療法）:** `is_confirm_key` 限定の
   昇格ロジックを、`gji_candidate_visible` 早期脱出パスと NameChangeWait
   タイムアウトパスにも同様に適用する。実装は容易だが本質解決ではない。
2. **経過時間ベースの補助指標追加（中コスト）:** `LiteralDetector`
   （`tsf/probe.rs`）の `check_now`/`DetectionResult` は現状、確認までの
   経過時間を一切保持・返却していない（`Option<DetectionResult>` の
   2値 enum のみ）。`start_ms` を保持させ「確認が極端に速ければ元々
   compose 進行中だった証拠」として偽陽性抑制に使える可能性があるが、
   `COMPOSITION_BYTES_THRESHOLD` 導入時と同様、実機サンプルからの
   閾値較正が必要。
3. **IMM32 文字列突合せの適用範囲拡大（高コスト・効果限定）:**
   `probe_fsm.rs:397-398` の `expected_kana` との実文字列突合せが最も
   直接的だが、WezTerm/Windows Terminal は HIMC=NULL のため適用不可
   ——これが `is_partial_literal` ヒューリスティック導入の前提そのもの
   なので、根本解決にはならない。

**関連ファイル:** `crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs`
（`is_partial_literal`）, `crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs`
（`nc_for_plan` 昇格・`skip-verify` 診断ログ・pre-idle スキップ）,
`crates/awase-windows/src/tsf/warmup/probe_fsm.rs`（`decide_transmit_plan`,
`ProbeObservations`）, `crates/awase-windows/src/tsf/probe.rs`
（`LiteralDetector`, `check_now`, `DetectionResult`）

**関連コミット:** `3ffbe66`（"な" バグ、`nc_fired` 昇格によるピンポイント
修正）, `1f35029`（`skip-verify` 診断ログ導入）, `4e31b64`（partial literal
検出後の回収を VK_ESCAPE ベースに変更——本バグとは独立に、検出後の
「何文字消すか」の精度は改善したが、検出自体の信頼性（本バグ）は未着手）

**追補（2026-07-11、実機ログでユーザー報告「VK_BACK が１回余分」を解析・
偽陽性の真因の一つを特定・部分修正）:** ユーザーから Windows Terminal +
GJI で「余分な BS が非常に多い」との報告。`DIAG_DISABLE_PROACTIVE_TSF_WARMUP`
（実運用中）下のログを解析した結果、以下 2 点が判明した。

1. **`nc_fired`/`gji_resumed` の測定窓が短すぎる:** 該当ケースでは romaji
   "mo" 送信からわずか `real_gji_idle_ms=16`（16ms）で `nc_fired=false`
   と判定されていた。この codebase の他の実測値（GJI round-trip 47〜250ms、
   BUG-08 の ~180ms 等）と比べて明らかに短く、確認信号が「まだ届く時間が
   なかっただけ」を「届かなかった＝失敗」と誤認する構造的リスクがある
   （`DIAG_DISABLE_PROACTIVE_TSF_WARMUP` が有効な間は本質的に避けられない
   トレードオフ——プロアクティブ warmup が提供していた「猶予時間」を
   意図的に無くす実験のため）。
2. **より広範な根本原因（今回修正）:** `composition_fsm.rs::ConfirmKeyDown`
   のコード自身のコメントが「warm な GJI/TSF を確定キーだけで cold 化
   する理由はない」と明記していたにもかかわらず、実装は warm/cold 両分岐で
   `MarkCold`/`GjiCompositionReset` を無条件に発行していた（`on_reinject_key`
   （`platform.rs`）の `ReinjectConfirmKey` 経路も同様、warm チェックなし）。
   結果、連続 typing 中に Enter/Space/Escape を押すたびに実際には何も
   冷えていないのに cold 化され、次の1文字が cold-start 経路
   （warmup+probe+literal-detect）を通ってしまい、上記(1)の false
   positive リスクに繰り返し晒されていた。

**修正:** `composition_fsm.rs::ConfirmKeyDown`（warm=true 分岐）と
`platform.rs::on_reinject_key`（confirm キー・`is_composition_warm()`
ガード追加）の両方で、warm なら `MarkCold`/`GjiCompositionReset` を
一切発行しないよう変更。KeyUp までの warmup 遅延タイミング制御自体は
維持。(1)の測定窓自体は未対応（真に新しい確認信号か、
`DIAG_DISABLE_PROACTIVE_TSF_WARMUP` 実験自体の終了が必要——別記事参照）。

**テスト:** `composition_fsm.rs` に `warm_confirm_keydown_does_not_mark_cold_or_reset_gji`
を追加（warm な ConfirmKeyDown が actions を一切発行しないことを固定）。
Windows-only モジュールのため Linux では `cargo test -p awase-windows --lib
--target x86_64-pc-windows-gnu --no-run` でコンパイル確認のみ（wine 不在で
実行不可、この codebase の既存パターンと同じ）。既存の golden_scenarios(19)・
architecture_guard(10)・layer_boundary_guard(8)・journal_replay(1)・lib(138)
は全件 pass。**Windows 実機での再発有無・BS 頻度の改善確認は未実施。**

**関連ファイル（追補）:** `crates/awase-windows/src/tsf/composition_fsm.rs`
（`ConfirmKeyDown`）, `crates/awase-windows/src/platform.rs`（`on_reinject_key`）

**追補（2026-07-11、IMEセッション単位の literal-detect スキップで(1)の測定窓問題を根治）:**
上記追補が「未対応」と記した(1)（`nc_fired`/`gji_resumed` の測定窓が
`DIAG_DISABLE_PROACTIVE_TSF_WARMUP` 下では構造的に短すぎる問題）に対応した。

**根本原因の再整理:** `is_partial_literal()` は「今回送った romaji 自身の確認信号」
（`DetectionResult::CompositionConfirmed` = 候補ウィンドウ SHOW / GJI I/O 変化）では
なく、送信前に確定していた無関係な代理指標 `nc_fired`/`gji_resumed`（別の F2 warmup
キーへの応答有無）で判定している。`ColdReason::requires_settle()`
（`FocusChange`/`NativeF2Consumed`/`SetOpenTrue` の3つ、IME が既に ON の状態でも
発生しうる）直後は、この代理指標の元になる確認送信が `DIAG_DISABLE_PROACTIVE_TSF_WARMUP`
により無条件でスキップされるため、`nc_fired` は構造的に常に `false` になる。

**修正方針（ユーザー提案）:** 「IME セッション（打鍵開始〜候補ウィンドウ HIDE）の
最初の1文字だけ実際に `CompositionConfirmed` を確認し、確認できたらそのセッションの
残りは literal-detect 自体をスキップして即送信する」という設計に変更した。
`cold_reason` の種類には一切依存せず、「今回のセッションで実際に compose が機能した」
という直接の事実だけを判断材料にする。これにより cold-start の第一文字だけがコストを
払い、以降は反応速度を落とさない。

cold パス（`GjiWarmupCoro` の inline LiteralDetect）と warm パス（`LiteralDetectFsm`）は
既に同一の `LiteralDetectCore::poll` を共有しているため、そこ1箇所にゲートを追加する
だけで両方に適用される — 新しい `ProbeAction` やコルーチンの分岐は不要だった
（当初検討した「VKを1個ずつ送って毎回確認するループ」案は過剰と判断し撤回、
本方式に一本化）。

**実装:**
1. `tsf/observer.rs` に `literal_session_confirmed: AtomicBool` を追加し、
   `literal_session_confirmed()`/`mark_literal_session_confirmed()`/
   `reset_literal_session_confirmed()` の3関数を新設（既存の `candidate_was_seen`
   と同じ命名・実装パターン）。
2. `tsf/warmup/literal_detect_fsm.rs::LiteralDetectCore::poll` の先頭で
   `DIAG_LITERAL_SESSION_SKIP && literal_session_confirmed()` を確認し、`true` なら
   検出処理自体をスキップして即 `[Done]` を返す。`CompositionConfirmed`（かつ
   非 partial-literal）を確認できたときに `mark_literal_session_confirmed()` を呼ぶ。
3. `platform.rs::gji_on_end_composition`（候補ウィンドウ HIDE の dispatch 箇所）で
   `reset_literal_session_confirmed()` を呼び、次のセッションの最初の1文字は
   改めて確認を受けるようにする。
4. `tuning::DIAG_LITERAL_SESSION_SKIP: bool = true` を新設（`DIAG_DISABLE_PROACTIVE_TSF_WARMUP`/
   `DIAG_FORCE_HIRAGANA_CHARSET` と同じ「実験用診断フラグで丸ごと切替可能にし、
   実機で観察する」流儀）。

**テスト:** `cargo build/test -p awase-windows --target x86_64-pc-windows-gnu`
（コンパイル確認、`literal_detect_fsm.rs`/`platform.rs`/`observer.rs` は
Windows専用モジュールのため Linux では実行不可・wine不在）。既存の
golden_scenarios(19)・architecture_guard(10)・layer_boundary_guard(8)・
journal_replay(1)・lib(138) 全件 pass、clippy(lib) warning ゼロを確認済み。

**Windows実機での検証は未実施。** 特に以下2点は実機でしか確認できない:
- セッション内2文字目以降で本当に `[literal-detect] ... partial literal` /
  `suspected literal` が発生しなくなるか（症状の改善確認）。
- セッション判定の起点・終点（HIDE のタイミング）がずれて、本来チェックすべき
  文字をスキップしてしまう偽陰性が起きていないか（`tuning.rs` の
  `DIAG_LITERAL_SESSION_SKIP` のドキュメント参照）。

**関連ファイル:** `crates/awase-windows/src/tsf/observer.rs`
（`literal_session_confirmed` 系3関数）,
`crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs`（`LiteralDetectCore::poll`）,
`crates/awase-windows/src/platform.rs`（`gji_on_end_composition`）,
`crates/awase-windows/src/tuning.rs`（`DIAG_LITERAL_SESSION_SKIP`）

**追補（2026-07-11、実機ログで「セッション最初の1文字自体」が未対応だったと判明・
per-VK 送信+確認ループで根治）:** 上記追補を適用した実機ビルドでも、`SetOpenTrue`
直後の最初の1文字（例: romaji="da"）で `[literal-detect] partial literal` が
再現した。原因は単純で、**`literal_session_confirmed()` はセッション最初の時点では
常に `false`** であり、上記追補のゲート（`if literal_session_confirmed() { skip }`）は
2文字目以降にしか効かない。肝心の「セッション最初の1文字」自体は従来通り
`is_partial_literal()`（無関係な代理指標ベース）を通っており、何も直っていなかった。
実機ログでは候補ウィンドウ SHOW が実際に確認できていた（＝正しく変換されていた）
にもかかわらず、`nc_fired=false`/`gji_resumed=false` により誤って部分リテラル
判定されていた。

**検討した代替案と却下理由:** (a) `CompositionConfirmed` を無条件に信頼する
（`is_partial_literal` を丸ごと削除）— 先頭1文字だけ literal 化し残りが compose
される「部分リテラル」ケース（例: "ltu"→'l' リテラル+'tu'→'と' 合成）を検知不能に
してしまう。(b) `foreground_comp_char`（IMM32 `GetCompositionString` による実文字列
突合せ、`probe_fsm.rs` の `TsfProbeCoro` で実装済み）を流用 — WezTerm/Windows
Terminal は TSF-native で HIMC=NULL のため、この経路は `None` 固定で機能しない。
(c) GJI I/O バイト数閾値（Chrome の `COMPOSITION_BYTES_THRESHOLD` と同型）で
1文字/2文字処理を判別 — 理論上は可能だが実機計測なしに新しい閾値を導入できない
（`tuning-constants.md`）。

**採用した修正（ユーザー提案）:** セッション最初の1文字に限り、romaji の VK を
**1つずつ** `SendInput` し、送信した VK 自身への `CompositionConfirmed`/
`SuspectedLiteral` を確認してから次の VK を送る（確認できなければ BS して再送）。
D と A をまとめて送るために生じていた「どちらの VK の効果か区別できない」問題は、
そもそも2つの VK 送信の間に意図的な確認ポイントを挟むことで構造的に解消される —
D 単体で GJI 反応がなければ D が漏れたと確定でき、D 確認後に A で反応がなければ
A だけが漏れたと確定できる（この場合は composition が既に実在するため ESC+BS で
回収）。全 VK が個別に確認できたら `mark_literal_session_confirmed()` を呼び、
以降は既存の（前追補の）セッションスキップ機構に委譲する。

**実装:** `tsf/warmup/probe_fsm.rs` に `ProbeAction::TransmitSingleVk` を新設
（`cold_seq, vk, needs_shift, timeout_ms, is_last, observations, plan`）。
`tsf/warmup/tickable_fsm.rs` に `TickableFsm::apply_vk_sent`（no-op デフォルト）を
追加。`tsf/warmup/gji_warmup_coro.rs::gji_coro_body` の Phase 5a と Phase 5b
（既存の一括 `Transmit`、無変更のままフォールバックとして残す）の間に新分岐を挿入:
`DIAG_LITERAL_SESSION_SKIP && plan.needs_literal && !literal_session_confirmed()
&& !plan.should_prepend_f2 && !plan.used_eager_path && env.is_tsf_mode` の場合のみ、
romaji を `crate::output::resolve_ascii_to_vk` で VK 単位に分解し、1つずつ
`ProbeAction::TransmitSingleVk` を yield → `LiteralDetector::check_now` を
ポーリング → `CompositionConfirmed` なら次の VK へ、`SuspectedLiteral` なら
`literal_detect_fsm::per_vk_recovery_params(idx)`（`backs=1`固定、
`escape_composition = idx > 0`）で `emit_recovery_actions` を呼ぶ。
`output/probe_io.rs` に `ProbeIo::send_single_tsf_vk`（`KeyInjector::send_vk_pair`
に委譲、F2 prepend 等の分岐なし）と `dispatch_probe_actions` の
`TransmitSingleVk` ハンドラ（送信直前に `LiteralDetector::new()`、`is_last` の
ときのみ deferred VK フラッシュ + `store_gji_warmup_if_probing`）を追加。

`is_partial_literal()` 自体は変更していない — 従来通りの一括 `Transmit`
経路（`should_prepend_f2`/`used_eager_path` が真のケース、warm パスの
`LiteralDetectFsm` 等）では引き続き使われる。今回の per-VK ループが対象と
するのはセッション最初の1文字の cold-start パスに限る。

**テスト:** `literal_detect_fsm.rs` に `per_vk_recovery_params` の単体テスト2件
（`idx=0→(1,false)`, `idx>0→(1,true)`）を追加。`cargo build/test -p awase-windows
--target x86_64-pc-windows-gnu` でコンパイル確認（`gji_warmup_coro.rs`/
`probe_fsm.rs`/`probe_io.rs`/`tickable_fsm.rs` は Windows専用モジュールのため
Linux では実行不可・wine不在）。既存の golden_scenarios(19)・
architecture_guard(10)・layer_boundary_guard(8)・journal_replay(1)・lib(138)
全件 pass、clippy(lib) warning ゼロを確認済み。

**Windows実機での検証は未実施。** 特に以下は実機でしか確認できない:
- `SendInput` によるVK注入でも、GJIの確認信号（候補SHOW/I-O変化）が本当に
  VK単位で分解して観測できるか（ネイティブ入力での分解能はユーザーが別途確認済み
  だが、`SendInput`注入では未確認）。
- 2VK以上のromaji（例: "da"）でVKごとに確認を挟むことによる体感レイテンシの増加
  （最大で `literal_detect_ms` × VK数まで伸びうる）。
- `idx > 0` の回収経路（今回新規、一度も実行実績なし）が実際に正しく動くか。
- 症状そのもの（`SetOpenTrue` 直後の最初の1文字で不要なBSが本当に収まるか）。

**関連ファイル（追補）:** `crates/awase-windows/src/tsf/warmup/probe_fsm.rs`
（`ProbeAction::TransmitSingleVk`）,
`crates/awase-windows/src/tsf/warmup/tickable_fsm.rs`（`apply_vk_sent`）,
`crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs`（`gji_coro_body` 新分岐、
`VkSentPayload`）,
`crates/awase-windows/src/output/probe_io.rs`（`send_single_tsf_vk`、
`dispatch_probe_actions` の `TransmitSingleVk` アーム）,
`crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs`（`per_vk_recovery_params`）

**追補（2026-07-11、予防的 warmup レイヤーの撤去。v1.8.9 で per-VK confirm
方式が実機確認された後の後片付け）:** 上記の per-VK confirm ループが
`SetOpenTrue` 直後の偽陽性を reactive 側だけで解消できることを実機で
確認できたため（`DIAG_DISABLE_PROACTIVE_TSF_WARMUP` を有効化した実機検証、
上記参照）、無条件に到達不能だった予防的コードパスを撤去した
（`cleanup/remove-proactive-warmup-safeguards` ブランチ）。

- `send_eager_tsf_warmup` のカタカナ/英数 charset 追従（`VK_DBE_KATAKANA`/
  `VK_DBE_ALPHANUMERIC` 系）— `DIAG_FORCE_HIRAGANA_CHARSET`（BUG-19 追補5）
  下で到達不能だった。`transmit_tsf` の katakana leading-warmup 分岐、
  `send_vk_runs_with_leading_warmup`、`cold_warmup.rs` の charset 別
  `conv_target` 復元ロジック、`ConvModeMgr::on_hankata_warmup_sent`、
  `tsf/send.rs` の `send_vk_dbe_katakana_warmup`/`send_vk_dbe_alpha_warmup`
  を撤去。
- `GjiWarmupCoro` の Phase 2 (`SendFreshF2`) + Phase 3
  (`NameChangeWait`/`SecondaryProbe`) — `DIAG_DISABLE_PROACTIVE_TSF_WARMUP`
  下で settle-check 分岐が Phase 2 に到達する前に必ず `break` していた。
  `ProbeAction::SendFreshF2`、`ProbeIo::send_fresh_f2`/`send_extra_f2`、
  `NamechangeBaseline`、`ProbeParams.ncwait_budget_ms`（`ColdKind`
  分類自体は維持）を撤去。
- `GjiWarmupCoro` Phase 5a の proactive `StartSacrificialWarmup`
  （long_cold && is_tsf_mode で犠牲キー escalation を即発行する分岐）
  — 同フラグ下で無条件に到達不能だった。Chrome 側の cold-start パス
  （`probe_fsm.rs::TsfProbeCoro`）と partial-literal 回収パス
  （`literal_detect_fsm.rs`）が発行する同アクションは撤去していない
  （生きた経路）。

いずれも「`DIAG_*` フラグが恒久的に `true` のままである」ことを前提にした
撤去であり、フラグを再度 `false` に戻す場合はこれらのコミットの revert
が必要。`cargo test -p awase-windows --lib`（138 passed）・
`--test golden_scenarios`（19 passed）・`--test architecture_guard`
（10 passed）・`--test layer_boundary_guard`（8 passed）・
`--test journal_replay`（1 passed）・clippy（`-D warnings`）で確認済み。

**追補7（2026-07-16〜17）: Chrome にも per-VK confirm を拡張し、Chrome/TSF 両実装を
`run_per_vk_confirm` に統合。** `experiment/skip-cold-probe-wait` ブランチで
Chrome の cold-start（`probe_fsm.rs::tsf_probe_coro_body` Phase 2c）にも
WezTerm と同型の per-VK confirm を追加（`DIAG_CHROME_USE_PER_VK_CONFIRM` 実験、
デフォルト有効）。並行して `DIAG_COLD_SKIP_F2`/`DIAG_COLD_SKIP_PROBE_WAIT`
（WezTerm 側の予防的 F2 送信・probe 事前待機を個別スキップする `AtomicBool` 実験、
トレイの「実験: cold warmup」から on/off）・Chrome 版の
`DIAG_CHROME_SKIP_F2`/`DIAG_CHROME_SKIP_PROBE_WAIT`/`DIAG_CHROME_SKIP_SACRIFICIAL_WARMUP`
を新設し、デフォルト全 `true`（F2 送信なし・probe 待機なしで即座に per-VK confirm
へ進む、最も大胆な状態）で実機投入した。24時間弱のソークで BUG-26〜29（本ファイル
別項）を発見・修正しつつ、無破損を確認した。

**追補8（2026-07-18）: 上記実験フラグをすべて恒久化し、待機行列・捨て駒キー
機構を物理削除。** 数日間の実機ソーク（cold=61〜74 超、WezTerm/Chrome 双方、
`suspected literal` genuine ゼロ件を `per-VK[...] confirmed` の3点セットログで
確認済み、追補6参照）で問題が起きなかったことを受け、以下を撤去した:

- `tsf/warmup/cold_warmup.rs`: `WarmupKind::FreshF2/ReWarmup/ProbeWithSettle`・
  `run_eager_start`/`run_non_eager_start`・`ColdReason`×`long_idle` の
  `eager_settle_ms`/`probe_min_ms` 行列（`tsf/output.rs::ColdReason::eager_settle_ms`/
  `probe_min_ms` メソッドごと）を削除し、`run_start` を「IMM32 ローマ字モード復元 +
  即座に per-VK confirm へ」の単一経路に単純化。`session_expired` 時のみ
  `DIAG_COLD_SKIP_F2` の値に関係なく無条件で F2 を送っていた抜け穴も閉じた
  （ユーザー確認の上、恒久的に F2 を送らない方針に統一）。
- `output/vk_send.rs::send_romaji_batched`（Chrome）: F2 事前送信（`SendMessageTimeout`
  + `SendInput` の二重送信）・probe 事前待機（`CHROME_PROBE_MIN_MS`/`MAX_MS`/
  `LONG_IDLE_MIN_MS`/`MAX_MS`）の計算・送信コードを削除。
- 捨て駒キー機構一式を物理削除: `ProbeAction::StartSacrificialWarmup`/
  `SacrificialResend`/`SendChromeGjiReinit`（`SendChromeGjiReinit` の実装関数
  `send_chrome_gji_reinit_and_poll` 自体は Unicode injection mode の long-cold
  再初期化 `Output::send_f22_f21_reinit` が直接呼ぶ別経路のため残置）、
  `tsf/warmup/sacr_warmup_coro.rs`（`SacrificialWarmupCoro`）・
  `tsf/warmup/ime_offon_warmup_fsm.rs`（`ImeOffOnWarmupFsm`）ファイルごと、
  `state/key_sequence_policy.rs::SacrificialWarmupKey`/`sacrificial_warmup_key`/
  `warmup_respects_bypass_gate`/`target_needs_sacrificial_cleanup_bs`、
  `probe_fsm.rs::TsfProbeCoro::new_chrome` の `is_long_cold` パラメータと
  Phase 2a 分岐自体（BUG-21 が扱っていた重症度分岐、詳細は BUG-21 追記参照）。
- `DIAG_LITERAL_SESSION_SKIP`（per-VK confirm 自体のゲート）を恒久 `true` 化
  （`gji_warmup_coro.rs`/`literal_detect_fsm.rs`/`platform.rs` のフラグ分岐を削除）。
- 上記に伴い連鎖的に不要となった dead code も削除: `ConvModeMgr::effective_charset`/
  `needs_conv_restore_write`/`mark_conv_restore_written`（+ `restore_written_for`
  フィールド、ADR-078 Phase 1a）、`ProbeIo::increment_consecutive_count`
  （+ `Composition::increment_consecutive_count` ラッパー）、
  `DispatchResult::SwitchMachine`、`TsfEnvSnapshot::gji_candidate_visible`
  フィールド。トレイの「実験: cold warmup (WezTerm/Chrome)」サブメニュー・
  `tray::toggle_diag_flag`・対応する `TrayCommand` バリアントも削除。
- `DIAG_DISABLE_PROACTIVE_TSF_WARMUP`/`DIAG_FORCE_HIRAGANA_CHARSET` は今回の
  スコープ外（別実験）として温存したが、`DIAG_FORCE_HIRAGANA_CHARSET` は
  唯一の実消費者だった `cold_warmup.rs` 側ロジックの削除に伴い**配線先を失い、
  現状値を変えても挙動に一切影響しない**（`tuning.rs` のコメントに記録）。

`cargo check`/`cargo test`/`cargo clippy --lib`（いずれも
`--target x86_64-pc-windows-gnu`、警告ゼロ）、Linux 上の `cargo test -p
awase-windows`（174 passed, 0 failed）で確認済み。ゴールデン
（`tests/golden/ime_key_sequences.txt`）の warmup ドキュメント section も
実装に追従して更新し、`WARMUP_DOC` 定数とバイト単位で diff 一致することを
手動確認した（wine 未導入のためこのサンドボックスでは `.exe` 実行不可、
実機/CI での `cargo test --target x86_64-pc-windows-gnu` 実行が最終確認となる）。
MS-IME 経路（`MsImeReadyCoro` 等）は今回一切変更していない。

**追補9（2026-07-19）: 追補8の副産物として残っていた observation/decision/belief
側の到達不能コードを codex CLI 協力の調査で特定・撤去。** ユーザーの見立て
「cold warmup 整理整頓の副産物として一部の observation/decision/belief が
不要になったはず」を受け、Codex CLI（`codex exec -s read-only`）を2プロセス
並列実行（候補検証パス + 独立発見パス）し、Claude 自身が全 file:line を
再検証（grep 再実行・producer まで遡って追跡）した上で以下を撤去した:

- `ProbeObservations.gji_resumed`（`tsf/warmup/probe_fsm.rs`）撤去。唯一の
  producer（`gji_warmup_coro.rs` の `'initial` ループ）が2分岐とも `false` を
  返しており本番では常に false だった（true になるのは単体テストのみ）。
  `decide_transmit_plan` の `used_eager_path`/`needs_literal` の死んだ分岐、
  `is_partial_literal()` の no-op 節、`WarmupResult.gji_resumed` フィールド、
  `classify_warmup_path` の `GjiResumed` 分岐、死んだ単体テスト3件を連鎖的に
  撤去（`WarmupPath::GjiResumed` 自体は Unicode injection mode 側で別途
  構築されるため enum variant は残置）。名前が似ている
  `LiteralDetector::new_gji_resumed()`（Chrome の Transmit 分岐で現役使用、
  GJI I/O write-bytes 差分ベースの別概念）とは無関係、削除対象ではない。
- `DIAG_FORCE_HIRAGANA_CHARSET`（`tuning.rs`）撤去。追補8時点で既にコメントが
  「配線先を持たず、値を変えても挙動に一切影響しない」と自認していたものを
  物理削除。
- `TsfReadinessProbe::wait_until_ready`（`tsf/probe.rs`）撤去。本番呼び出し
  ゼロ（全4呼び出し元が `probe.rs` 自身の `#[cfg(test)] mod tests` 内）を
  確認。`check_now` 自体のタイミング挙動を検証する4回帰テストは、ループ本体を
  テスト専用ヘルパー `poll_until_ready` として維持し継続。
- `GjiWarmupCoro` の `needs_settle_check` パラメータ撤去。唯一の producer
  （`WarmupStarted`、構築箇所は `cold_warmup.rs::run_start` の1箇所のみ）が
  常に `true` を渡していたため、Phase 1 の settle-check 本体を無条件実行に
  インライン化。
- `DIAG_DISABLE_PROACTIVE_TSF_WARMUP`（`tuning.rs`）を**ユーザー判断で**恒久化。
  このフラグは元々 const true で、本番では既に「romaji バッチへの F2 直接
  同梱（第3の防御層）を無効化」という挙動が確定していたため恒久化自体は現状の
  挙動を変えない。`decide_transmit_plan` から `initial_prepend_f2` パラメータを
  削除し `should_prepend_f2` を恒久的に false 化、`needs_literal` の死んだ
  第1節を削除。`gji_warmup_coro.rs` の `effective_prepend_f2`/`suppress_f2`
  計算・DIAG 分岐、`GjiProbeCtx`/`GjiWarmupCoro::new` の
  `prepend_f2_warmup`/`fresh_f2_at_probe_start`（前者の唯一の入力元）、
  `WarmupStarted.fresh_f2_at_probe_start`（唯一の読み手が消えたため到達不能に）
  も連鎖的に撤去。**既知のフォローアップ（未実装）**: `WarmupOutcome.
  prepend_f2_warmup`（`output/mod.rs`）は `plan.should_prepend_f2` からのみ
  供給されるため恒久的に false になったはずで、`TsfSendPipeline::transmit`
  （`vk_send.rs`）の `outcome.prepend_f2_warmup` 分岐・
  `Output::send_vk_runs_with_leading_f2` が TSF/GJI 経路では到達不能になった
  可能性が高いが、本セッションでは未調査・未削除（次の codex 調査候補）。

**据え置き（削除しなかったもの）**: `TsfReadinessProbe::check_now`
（`tsf/probe.rs`）の `min_ms`/`total_max_ms` 分岐。本番の producer
（`cold_warmup.rs::run_start`・`vk_send.rs` の Chrome cold パス）が現状
両方とも 0 を渡しているため実質常に最初の呼び出しで true を返すが、これは
上記項目のような**静的な到達不能**（コンパイラ/型で保証される dead code）
ではなく、両呼び出し元が**たまたま実行時に 0 を渡しているだけ**の状態。
`check_now` 自体は任意の値に対して汎用的に正しく動作するタイミング
primitive であり、cold-start 待機時間の調整は本リポジトリで過去に何度も
出し入れされてきた領域（[tuning-constants](../.claude/rules/tuning-constants.md)
の釣り上げ履歴: `CHROME_PROBE_MIN_MS` 20→100→200ms 等）。ユーザー確認の上、
削除せずコメントで現状を記録するに留めた（`tsf/probe.rs::check_now` の
doc comment参照）。

調査は Codex 2プロセス完了後、Claude が `verdict: confirmed` の各項目を
file:line 再読み込み・独立 grep 再実行で裏取りしてから実施（セッション中に
codex 側の誤り2件を Claude 自身の直接ソース確認で発見・訂正済み:
`LiteralDetector::new_gji_resumed()` は生きている、`send_romaji_as_tsf` は
`gji_current_probe_params()` を呼ぶ）。各コミットごとに `cargo check`/
`cargo clippy`（`--target x86_64-pc-windows-gnu`、警告ゼロ）、
`cargo test --target x86_64-pc-windows-gnu --no-run` でテストバイナリの
コンパイル（リンク含む）を確認。wine 未導入のためこのサンドボックスでは
実行不可（実機/CI での実行確認が最終）。`docs/experiments.md` エントリ10に
本ソーク全体の経緯を記録。

**追補10（2026-07-19）: 追補9で列挙した dead code 候補（GJI probe/warmup 関連の
全変数）を9並列 opus エージェントで1件ずつ懐疑的に再検証し、削除できる7件を
物理削除。** 追補9が残した「既知のフォローアップ（未実装）」（`WarmupOutcome.
prepend_f2_warmup`）を含め、GJI probe/warmup に関わる変数を洗い出す5並列調査
（一次調査）→ 各候補を反証前提で再検証する9並列 opus エージェント（二次調査、
「反証がないか」を主眼に repo 全体 grep・`impl` 網羅・`git log -p` 確認）の
二段構えで実施した。以下7件を DEAD 確定・削除:

1. `WarmupOutcome.prepend_f2_warmup`（`output/mod.rs`）と
   `TsfSendPipeline::transmit`（`vk_send.rs`）の `outcome.prepend_f2_warmup`
   分岐・`Output::send_vk_runs_with_leading_f2`（`key_injector.rs`）。追補9の
   予想通り `plan.should_prepend_f2` 経由で常に false だった。**注意**: 同名の
   別フィールド `WarmthContext.prepend_f2_warmup`（`mod.rs`、`needs_f2_probe()`
   由来）は現役でありこれとは無関係。削除時に混同しないこと。
2. `tsf/gji_fsm.rs::PendingInput.deferred_vks: Vec<DeferredVk>`。常に空の
   `Vec::new()` で初期化されるのみで push 箇所ゼロ。実データは
   `TsfWarmupCoordinator::pending_deferred` の別系統（同名の `DeferredVk` 型を
   使うだけで無関係）。
3. `WarmupPath` enum・`WarmupResult` struct（`gji_fsm.rs`）・
   `GjiEvent::WarmupComplete.result`・`GjiAction::SendInput.result`。
   `platform.rs` の dispatcher が `GjiAction::SendInput { .. } => {}` で
   フィールドを一切読まず握りつぶしていた（コード自身が
   「shadow tracking 専用、フィールドはテストでのみ検証される」と自認済み
   だったが、実際はテストでも `..` で読み飛ばされ未検証だった）。
   `TsfWarmupCoordinator.pending_gji_warmup` は `Cell<Option<WarmupResult>>`
   → `Cell<bool>` に縮小（`Option` の有無自体は `step_probe` が
   `WarmupComplete` を dispatch するかどうかの分岐に使われており、これは
   到達可能な生きたロジックだったため保持）。連鎖して
   `output/probe_io.rs::classify_warmup_path` と `ProbeAction::Transmit`/
   `TransmitSingleVk` の `observations`/`plan` フィールド（この2アクションの
   dispatcher 側では未使用だった）も削除。
4. `tsf/observer.rs::gji_read_op_count`/`gji_read_bytes`（と対応アクセサ）。
   `gji_monitor.rs` から書き込まれるのみでアクセサ呼び出しゼロ。`git log`
   で「将来の状態推定用に先行導入され、対の free fn 版は既に撤去済み」の
   残骸と判明。
5. `tsf/probe.rs::ColdContext::set_idle_ms_at_last_cold`。呼び出しゼロ
   （`record_cold` が別途 `idle_ms_at_last_cold` を設定済みで不要）。
6. `tsf/probe.rs::ColdContext::cold_marked_ms`（フィールド・メソッド）と
   `CompositionState::cold_marked_ms` ラッパー。`record_cold` が書き込むのみで
   外側アクセサの呼び出しゼロ（導入コミットから一貫して読み手未配線）。
7. `tsf/warmup/tickable_fsm.rs::TickableFsm::notify_start_composition` の
   デフォルト実装と呼び出し（`tsf_warmup_coord.rs`→`output/mod.rs`→
   `platform.rs::drain_pending_composition_events`）。唯一のオーバーライド
   実装者だった `SacrificialWarmupFsm`/`SacrificialWarmupCoro` は `d495649`
   で既に物理削除済みで、現存する全 `TickableFsm` 実装（7種）がデフォルト
   no-op に落ちる撤去漏れのフックだった。

**据え置き再確認**: 追補9で「削除しなかったもの」とした
`TsfReadinessProbe::check_now` の `min_ms`/`total_max_ms` 分岐
（`cold_warmup.rs::WarmupStarted.total_max_ms` が常に 0 になる経路含む）を
独立した opus エージェントで再度反証を試みたが、結論は変わらず**削除しない**。
本番の呼び出し元（`cold_warmup.rs::run_start`・`vk_send.rs` Chrome cold パス）
が現状 `min_ms=0`/`total_max_ms=0` を渡すため `check_now` は実質常に初回
呼び出しで true を返すが、これは静的な到達不能ではなく「たまたま実行時値が
0」の状態であり、`check_now` 自体は任意値に対する汎用タイミング primitive
（`probe_fsm.rs` の3回帰テスト `probe_phase2_detects_already_settled` 等が
非ゼロ値でこの分岐を直接検証している）。削除すると回帰テストが壊れる。

各削除は1項目ずつ `cargo check`/`cargo test --no-run --target
x86_64-pc-windows-gnu`（警告ゼロ）で確認し、最後に `cargo cc`
（`.cargo/config.toml` のプロジェクト規定 clippy エイリアス、`--lib -D
warnings -W clippy::cognitive_complexity`）で最終確認した。wine 未導入のため
このサンドボックスでは実機テスト実行不可（実機/CI での実行確認が最終）。
`docs/experiments.md` エントリ10に追記。

**追補11（2026-07-19）: 追補10と同じ手法（opus エージェントによる一次洗い出し
+ 反証前提の再検証）でもう一段掘り、安全に削除できる2件を追加で物理削除。**
以下を DEAD 確定・削除:

1. `tsf/observer.rs::TsfObservations::gji_last_write_ms()`/`gji_write_bytes()`
   （レシーバ形アクセサ）。前回削除した `gji_read_op_count`/`gji_read_bytes`
   と同型の撤去漏れ孤児メソッドで、repo 全体でレシーバ形の呼び出しがゼロ
   （実際の読み手は同ファイル内の free fn 版 `gji_last_write_ms()`/
   `gji_write_bytes()` と `output/` からの直接フィールド相当アクセス）。
2. `tuning::GJI_LONG_IDLE_PROBE_TOTAL_MS`（350ms、`79134f5`/`9a7e699` 由来の
   実測付き定数）→ `ColdKind::budget_ms()` → `GjiAction::StartProbe.budget_ms`
   フィールドの一連のチェーン。NameChangeWait 機構撤去（追補8）と
   skip-cold-probe-wait 実験（probe_min/max=(0,0) 恒久化）の結果、この値は
   どのタイマー・deadline・分岐も支配しなくなり、唯一の消費先が
   `platform.rs` の `StartProbe` debug ログ文字列（`budget={budget_ms}ms`）
   だけになっていた（挙動デッド）。`ColdKind` enum 自体
   （`forces_prepend_f2`/`is_long`/`is_proactive`/`classify`）は
   `StartProbe` の分岐判断に生きているため残置。`ColdKind` 各 variant の
   doc comment が参照していた `ncwait_budget`（撤去済みの NameChangeWait
   機構の残骸用語）も併せて是正した。値の変更ではなく log-only 化した
   死んだ定数の撤去のため、tuning-constants.md の実測義務（新しい ms を
   実測して決める場合の規約）は適用対象外。

保留判定（削除しなかったもの、いずれも意図的な残置と再確認）:

- `TransmitPlan.should_prepend_f2`（`probe_fsm.rs`） — 本番常に false だが、
  回帰テスト `decide_plan_should_prepend_f2_is_always_false` とコメントで
  「第3防御層の再有効化フック」として明示的に残置（追補9の
  `DIAG_DISABLE_PROACTIVE_TSF_WARMUP` 恒久化の直接残渣）。
- `used_eager_path` パラメータ（`vk_send.rs::send_romaji_as_tsf_warm` /
  `GjiWarmupCoro::new`） — 本番常に false 渡しだが、warm path 側の
  PendingGjiConfirm override 経路のための汎用 plumbing として残置。
- `ime_show_seq`/`ime_change_seq`（`observer.rs`） — reader なしの
  write-mostly カウンタだが、`IME_SHOW #{seq}` 情報ログに埋め込まれる
  実機検証用 monotonic seq（`focus_namechange` と同種の意図的診断）。
- `GjiAction::SendInput`/`SendInputDirect` と `PendingInput.romaji` —
  dispatcher（`platform.rs`）が `.. => {}` で握り潰すが、FSM モデル
  完全性のテスト用 mirror scaffolding として意図的に残置（追補10で
  `.result` フィールドのみ撤去済みの経緯どおり）。

削除2件それぞれで `cargo check`/`cargo clippy -p awase-windows --target
x86_64-pc-windows-gnu --lib -- -D warnings -W clippy::cognitive_complexity`/
`cargo test -p awase-windows --target x86_64-pc-windows-gnu --no-run`
（いずれも警告ゼロ）を確認し、Linux で実行可能な `cargo test -p
awase-windows --lib`（135 passed）と `architecture_guard`/
`golden_scenarios`/`ime_key_sequence_golden`/`layer_boundary_guard`
（Linux 実行分すべて green）も実行した。wine 未導入のためこのサンドボックス
では実機テスト実行不可（実機/CI での実行確認が最終）。`docs/experiments.md`
エントリ10に追記。

---

## BUG-25: 左Shift単独タップによる「IME-ON 半角英数」持続トグル（BUG-15 hold方式の置換）

**背景:** BUG-15 の「Shift 押しっぱなし中は IME-ON 半角英数」（hold 方式）を、
ユーザー要望（2026-07-11）により「左Shiftキー単独タップ（他キーを介さない
押下→解放）でトグル」する方式に置き換えた。目的は同じ（awase が Shift+文字
チョードを consume することで MS-IME の「Shift 単独タップ英数切替」誤検知が
発火する問題を打ち消しつつ、ユーザーが任意に半角英数を使えるようにする）だが、
UXを「押しっぱなし」から「タップでトグル」へ変更している。対象 IME は
MS-IME・GJI 両方（旧 hold 方式は MS-IME 限定だった）。

**設計判断（重要）:** BUG-15 の hold 機構は、実は2つの役割を兼ねていた。

1. Shift 押下→解放のたびに**無条件で** conv を英数へ→かなへ書き戻す
   「安全網」（MS-IME の Shift 単独タップ誤検知を、本当に単独タップだったか
   問わず常に打ち消す）。
2. Shift 押しっぱなし中は ASCII キーを IME 経由で素通しする「hold 中の
   半角英数入力」レイヤー（`shift_plane_halfwidth`）。

新機能実装時、(1) を撤去せず (2) だけ撤去する必要があることが設計検証で
判明した。(1) を撤去すると、「本物の単独タップだけに反応する」新トグルでは
Shift+文字キーのチョード（`.yab` Shift 面、`'！'` 等の全角記号）を engine が
consume する際に MS-IME の誤検知を打ち消す仕組みが無くなり、**BUG-15 の症状
（数秒〜十数秒のかな入力破壊）がそのまま再発する**。詳細は BUG-15 追補8参照。

**実装:**

- `crates/awase-windows/src/runtime/key_pipeline.rs::kp_stage_shift_conv_guard`
  （旧 `kp_stage_shift_eisu_hold` を改名・再構成）: 物理 Shift（L/R 問わず）の
  押下→解放のたびに無条件で conv を書き戻す安全網は維持。左Shift の押下→解放の
  間に他の非注入物理キー（`VK_RSHIFT` を含む）が一切来なかった場合のみ
  「単独タップ」と判定し、この復元をキャンセルして
  `half_width_alnum_toggle_active` を立てる（持続トグルへ移行）。もう一度
  単独タップしたら通常の復元を実行してトグルを解除する。右Shift単独タップは
  常に安全網の復元を実行するため、持続トグル中に右Shiftをタップすると
  「緊急解除」としても働く。
- `kp_restore_kana_from_half_width`: トグルOFF・安全網の復元を共通化した
  ヘルパー。`effective_open()==false` の場合は scan 付き `VK_DBE_HIRAGANA`
  注入をスキップし IMC write のみに留める（BUG-15 追補7の「実 IME が確実に
  ON でない限り IME モードキー注入禁止」を、hold より窓が長い持続トグルにも
  徹底するため）。
- **belief 側の核心**: 左Shift単独タップ（1回目）で
  `InputModeApplied { mode: ObservedEisu, strategy: UserHalfWidthAlnumToggle }`
  を dispatch する。`Engine::compute_state`（`src/engine/engine.rs`）は
  `input_mode.is_romaji_capable()==false` を見て `Inactive(NotRomajiInput)`
  を返し、`transition_activation` は `NotRomajiInput` の場合 `SetOpen` effect
  を出さない（`suppress_set_open` 分岐）。つまり **IME は belief 上 ON の
  まま、engine だけが素通りモードになる** — `set_user_enabled(false)` のような
  「本当に IME を閉じる」副作用を伴わずに持続トグルを実現できる（golden
  シナリオ15 で検証済み、`tests/golden_scenarios.rs`）。
- トグルON中に IME-ON 系キー（`kp_stage_shadow_ime_toggle` の
  `UserImeOnEisuReset`/`UserTurnOnEisuReset`、`kp_stage_post_decision` の
  `PostSetOpenEisuReset`）が発火条件を満たした場合、通常の
  `ObservedEisu→AssumedRomaji` 書き戻しではなく `kp_restore_kana_from_half_width`
  （トグルOFF処理そのもの）を呼ぶ。単に書き戻すと belief だけ romaji-capable に
  戻り実 conv は半角英数のままの壊れた中間状態になるため。
- フォーカス変更時（`ime_refresh.rs::ir_notify_focus_changed`）、トグルON中なら
  即座にトグルOFF処理を発火し、半角英数状態を他アプリへ持ち越さない。
- `InputModeApplyStrategy::UserHalfWidthAlnumToggle`・
  `AssumedReason::UserHalfWidthAlnumToggleOff` を新設。`SetOpen` を経由しない
  ため `state/eisu_recovery.rs` の「IME を ON にする経路」対応表・
  `architecture_guard.rs::user_ime_on_paths_are_paired_with_eisu_reset` の
  対象外（`eisu_recovery.rs` module doc に明記）。

**撤去したもの（BUG-15 hold方式固有）:** `shift_plane_halfwidth` 設定、
`ShiftEisuDisposition`/`shift_eisu_disposition`（`nicola_fsm.rs`）、
`KeyAction::Text`（`src/types.rs` および macOS/Linux/Windows 各出力層、
`send_text_direct`）。`shift_face_reduce` 自体・`should_use_shift_plane`
（Shift 面ルーティング機構、BUG-15 より前の 2026年3月 `72bd118` 由来）は
**撤去していない**。

**未検証（実機検証が必要、Codex レビューでも指摘済み）:**

1. ~~GJI 経路が完全に未検証~~ → **実機確認・撤回済み。詳細は追補1参照。**
2. **フォーカス変更時の安全策**（`ir_notify_focus_changed`）は実機での
   タイミング競合（フォーカス変更直後に IME が既に切り替わっている等）を
   確認していない。
3. **StickyKeys（アクセシビリティ機能）との相互作用は未検証。** StickyKeys
   自体が「Shift 単独タップ」を検出してラッチする機能を持つため、本機能と
   セマンティクスが競合する可能性がある。
4. 右Shift単独タップによる「トグル緊急解除」の実際の使用感（意図せず解除
   されて驚く可能性）は未検証。

**テスト:** `crates/awase-windows/tests/golden_scenarios.rs`
`scenario_15_half_width_alnum_toggle_keeps_ime_open_while_engine_goes_inactive`
（belief 遷移の核心部分のみ。`kp_stage_shift_conv_guard` 自体のタップ/チョード
判定ロジックは Windows 実機フック依存のため、BUG-15 の hold 方式と同様に
自動テスト不可——手動/ログベース検証に頼る）。`src/engine/tests.rs` の
`test_shift_held_uses_shift_face`/`test_shift_face_returns_literal_via_ime`
は撤去後の `shift_face_reduce`（.yab の値をそのまま Reduce）を検証する。
`tests/architecture_guard.rs::input_mode_applied_construction_sites_are_accounted_for`
の期待値を更新済み（`key_pipeline.rs` 内の構築箇所数 3→5）。

**関連ファイル:** `crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_stage_shift_conv_guard`/`kp_shift_conv_guard_key_down`/
`kp_shift_conv_guard_key_up`/`kp_restore_kana_from_half_width`）、
`crates/awase-windows/src/state/platform_state.rs`（`GateStore`
の `left_shift_tap_candidate`/`shift_conv_guard_pending`/
`half_width_alnum_toggle_active`）、
`crates/awase-windows/src/state/ime_event.rs`
（`InputModeApplyStrategy::UserHalfWidthAlnumToggle`）、
`src/engine/mode_state.rs`（`AssumedReason::UserHalfWidthAlnumToggleOff`）、
`crates/awase-windows/src/state/eisu_recovery.rs`（対応表対象外の注記）、
`crates/awase-windows/src/runtime/ime_refresh.rs`（フォーカス変更安全策）

**関連バグ:** BUG-15（置換元）、BUG-14（Shift 相関の外部注入）

---

**追補1（実機確認・撤回、2026-07-11）: GJI entry の scan 付き `VK_DBE_ALPHANUMERIC`
注入が CapsLock を汚染し、BUG-15 追補7 が別形で再発した。**

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` フォーカス、実体は
`Windows.UI.Input.InputSite.WindowClass`、TSF-native）× GJI（Google 日本語入力）で
左Shift単独タップを行うと、ユーザー報告の最終状態は「IME ON / **CAPS LOCK ON** /
awase engine OFF（belief 上、意図通り）/ ローマ字入力 / ひらがな」。実 conv は
`0x00000019`（NATIVE|FULLSHAPE|ROMAN、ひらがなローマ字）のまま一切変化せず、
半角英数化は完全に未反映。`あＢＣ` のように、素通しした physical key が GJI 自身の
ローマ字合成やネイティブの Shift+文字→全角変換に巻き込まれる副作用も確認された。

**原因:** entry 実装は GJI 検出時（`gji_is_active_ime()==true`）、既存の TSF warmup
経路 `crate::tsf::send::send_vk_dbe_alpha_warmup(Charset::HankakuAlpha)`
（scan 付き `VK_DBE_ALPHANUMERIC` を `make_tsf_key_input`+`SendInput` で注入）を
流用していた。診断ログ追加後の実機確認:

- `[shift-conv-guard] entry branch 判定: gji_is_active_ime=true active_ime_kind=GoogleJapaneseInput`
  — 分岐判定自体は正しい。
- `[tsf-warmup] alpha warmup (Hankaku) SendInput sent=2/2 events` — `SendInput`
  はOSレベルで成功（戻り値ベースで2/2イベント送信）。
- しかし `[hook] IME-mode vk=0xF0 ...` のログが**一度も出力されない**
  （同じ仕組みで送る `VK_DBE_HIRAGANA`/0xF2 は毎回確実に出力される）。
  `hook.rs` の IME-mode 診断ログは自己注入フィルタより**前**で無条件に出るため、
  これはフックが 0xF0 イベントを一切受け取っていないことを意味する。
- `[shift-conv-guard] entry verify (150ms後): conv=0x00000019 NATIVE=true`
  — 実convは変化なし。

`VK_DBE_ALPHANUMERIC`(0xF0) の `MapVirtualKeyW(..., MAPVK_VK_TO_VSC)` は
scan=0x3A（物理 CapsLock 位置）を返す（BUG-15 追補7で既出）。IME が処理しない
文脈（あるいは GJI の TSF キーイベントシンクがこの単発注入を認識しない文脈）
では、kbd106 の素のキー処理が scan=0x3A を CapsLock として横取りし、
`awase` 自身の低レベルフックにすら vk=0xF0 として届かない
（フック到達前に OS/ドライバレベルで CapsLock トグルへ変換されている）。
BUG-15 追補7は「実 IME が OFF の文脈」を原因としていたが、今回は
`effective_open()==true`（実 IME ON 確認済み）のガード下でも発生した——
GJI という**IME 種別そのもの**が、この単発 F0 注入を認識しない（元々
`send_vk_dbe_alpha_warmup` は「直後に文字 VK を続けて送る」前提の
NICOLA 内部 warmup ヒントであり、standalone トグルとして安全に使える
設計ではなかった）ことが真因と判断した。

**対応:** GJI 分岐を撤去し、entry は IME 種別によらず MS-IME と同じ IMC write
（`set_ime_romaji_mode_with_target_async(Some(0))`）に一本化した
（`kp_shift_conv_guard_key_down`）。IMC write は BUG-15 の運用実績で
CapsLock を汚染しないことが確認済み。GJI + himc_null な TSF-native ウィンドウ
（今回のテスト環境）で IMC write 自体が実 conv に反映されるかは追加の実機
検証が必要——反映されない場合、少なくとも CapsLock 汚染という実害は無くなるが、
「トグルON後も実際には半角英数化されない」という機能不全は残る。GJI 向けの
真に安全な entry 経路（例: config1.db 経由のキーバインド活用等）は今後の課題。

**教訓:** `VK_DBE_ALPHANUMERIC`（scan=0x3A）の scan 付き注入は、実 IME の
ON/OFF 状態にかかわらず、**対象 IME がこの単発注入を実際に処理する保証がない
限り使ってはならない**。`effective_open()` ガードは「実 IME が OFF」由来の
CapsLock 汚染は防ぐが、「IME がこの注入を認識しない」由来の同一症状は防げない。
既存の warmup 用ヘルパー（`send_vk_dbe_alpha_warmup` 等、直後に文字送信が
続く前提で設計されたもの）を、無関係な standalone トグル用途へ転用しない。

**テスト:** 自動テスト不可（実機の kbd106/CapsLock 挙動に依存）。この追補が
再発防止の記録。今後 entry 経路を変更する場合は、必ず実機で
`[hook] IME-mode vk=0xF0` ログの出現と CapsLock 状態を確認すること。

**関連ファイル（追補1）:** `crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_shift_conv_guard_key_down`、GJI 分岐撤去）、
`crates/awase-windows/src/tsf/send.rs`（`send_vk_dbe_alpha_warmup`、
SendInput 戻り値ログ追加。関数自体は元の TSF warmup 用途で存続）

---

**追補2（実機確認・撤回、2026-07-11）: 追補1の IMC write 一本化は GJI では
「読み返すと成功して見える」だけの偽の成功だった。mozc 本家ソース調査に基づき
GJI 専用の scan=0 `VK_DBE_ALPHANUMERIC` 注入へ再度分岐（未検証）。**

**症状:** 追補1の対応（IMC write 一本化）適用後、実機で
`success=true`・verify-read で `conv=0x00000000 NATIVE=false` を確認し、
一度は成功と報告した。しかしユーザーが直後に「あいうえお」を打鍵したところ
実際にはひらがなが出力され、GJI の実コンポーザは半角英数へ一切切り替わって
いなかった（ユーザー報告「え？全然デキてないよ」）。

**原因（mozc 本家ソース `google/mozc` 調査で確認）:** GJI の TIP
（`win32/tip/tip_text_service.cc`）は独自の低レベルフックを持たず、
`ITfKeyEventSink` 経由の TSF キールーティングのみでキーを受け取る。conversion-
mode compartment（`GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION`）への書き込みは
`ITfCompartmentEventSink::OnChange` → `TipEditSession::OnModeChangedAsync`
（`tip_edit_session.cc`）を発火させるが、この経路は UI 表示（言語バー等）の
同期のみを行い、`SessionCommand::SWITCH_COMPOSITION_MODE` を実コンバータへ
送る `SendCommand()` を一切呼ばない。実際にモードが切り替わるのは
`TipEditSession::SwitchInputModeAsync`（`AsyncSwitchInputModeEditSessionImpl`）
経由のみで、これは言語バークリックか本物のキー入力（`win32/base/keyevent_handler.cc`
が `VK_DBE_ALPHANUMERIC` を VK 値だけで `KeyEvent::EISU` に変換し
`Session::ToggleAlphanumericMode`→`mutable_composer()->ToggleInputMode()` を
呼ぶ経路）からしか発火しない。つまり IMC write は GJI にとって
**構造的に一方向の UI ミラーであり、実コンポーザには絶対に届かない**
（読み返しで「成功」が確認できても無意味）。BUG-15 追補3で既知だった
「IMC read は実モードを保証しない」という教訓と同じ形の失敗を、今回は逆方向
（write 側）でも踏んだ。

**対応（未検証）:** GJI 分岐を復活させるが、追補1で撤回した scan 付き注入
（`MapVirtualKeyW` 由来の scan=0x3A、CapsLock 物理位置と衝突）ではなく、
`make_key_input_ex(VK_DBE_ALPHANUMERIC, is_keyup, TSF_MARKER)` で
**scan=0**（非衝突値）の DOWN+UP ペアを直接送る方式に変更した
（`kp_shift_conv_guard_key_down`）。根拠: mozc の `keyevent_handler.cc` は
VK 値のみで判定し scan を見ない。追補1の CapsLock 汚染は scan=0x3A が OS/
kbd106 ドライバ層で CapsLock として横取りされフックにすら届かなかったことが
真因であり、scan=0 はどの物理キーにも対応しないため同じ横取りは起きない
と推測される。`VK_DBE_HIRAGANA` は非衝突 scan=0x70 で TSF 経由の到達・反映が
実績として確認済みであり、DBE 系 VK 自体は TSF ルーティングで機能することは
既に分かっている。MS-IME 経路は影響を受けず、引き続き IMC write を使う
（MS-IME では元々このIMC write が実効的な経路であり、この失敗は GJI 固有）。

**未検証（次回実機テストで確認すること）:**

1. `[hook] IME-mode vk=0xF0 ...` ログが今度こそ出現するか（追補1では
   一度も出現しなかった＝OS レベルで握り潰されていた）。
2. CapsLock が汚染されないか（scan=0 が CapsLock 物理位置と衝突しないことの
   実地確認）。
3. 実際に半角英数の打鍵結果が得られるか（`entry verify` の conv 読み取りは
   GJI では実効性の証明にならないため、必ず実際の打鍵結果で確認する。詳細は
   下記「教訓」）。

**教訓:** GJI に対しては、conversion-mode compartment の読み書き（IMC read/
write）を成否判定に使ってはならない——mozc 側の実装で書き込みは UI ミラーに
すぎず、読み取りも「awase 自身が直前に書いた値をそのまま読み返しているだけ」
になりうる。GJI の mode 切り替えが実際に効いたかどうかは、**必ず実際の
打鍵結果（対象アプリの表示テキスト）でのみ検証する**。IMC read/write の
`success=true` や verify ログを実機確認の代替として扱わないこと。

**テスト:** 自動テスト不可（実機の GJI TIP 挙動・kbd106 挙動に依存）。この
追補が再発防止の記録。次に entry 経路を変更する場合は、必ず実機で
`[hook] IME-mode vk=0xF0` ログの出現・CapsLock 状態・実際の打鍵結果（ローマ字
ではなく英数が出力されるか）の3点をすべて確認すること。IMC の
read-back だけで成功と判断しない。

**関連ファイル（追補2）:** `crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_shift_conv_guard_key_down`、GJI 分岐を scan=0 注入へ変更）、
`crates/awase-windows/src/tsf/output.rs`（`make_key_input_ex`/`TSF_MARKER`、
既存ヘルパーを流用）

---

**追補3（実機確認・撤回、2026-07-11）: scan=0 の `VK_DBE_ALPHANUMERIC` 注入も
awase 自身のフックにすら届かず失敗。GJI entry を全面停止（保留）に変更。**

**症状:** 追補2の scan=0 注入を実機投入。ユーザーが「こんにちはあいうえお」を
入力し「ダメでしたね」と報告。ログ全文を確認したところ:

- `[shift-conv-guard] GJI VK_DBE_ALPHANUMERIC(scan=0) SendInput sent=2/2 events`
  — `SendInput` 自体は OS 的に成功。
- **`[hook] IME-mode vk=0xF0 ...` のログがログ全文を通じて一度も出現しない**
  （追補1の scan=0x3A 注入と同じ症状。同一セッション内で `VK_DBE_HIRAGANA`
  0xF2 は `scan=0x70` で毎回確実に `[hook]` ログに出現しており、フック自体は
  正常に動作している）。
- `[shift-conv-guard] 左Shift単独タップ → 半角英数トグルON` の直後に
  `Engine deactivated (... reason=Inactive(NotRomajiInput))` が発火し、以降の
  ローマ字キー（`vk=0x41`='A', `0x49`='I', `0x45`='E', `0x55`='U`, `0x4F`='O'
  等）はすべて `[relay-passthrough] PassThrough idle: direct OS pass-through`
  として**生のまま GJI へ素通し**されている。しかし GJI 自身の conv は
  scan=0 注入でも一切変化していないため（entry verify を今回は行っていないが、
  前提となる `[hook] vk=0xF0` 到達自体が無いので当然変化していない）、素通しされた
  生ローマ字キーが GJI 自身の**未切替のひらがな変換エンジン**にそのまま入り、
  結果的に「こんにちは」のようなひらがな文字列がそのまま出力された。

**原因（推定）:** `[hook]` ログは自己注入フィルタより前で無条件に出るため、
`VK_DBE_ALPHANUMERIC` の `SendInput` イベントは scan 値（0x3A/0x3A衝突 or
scan=0/非衝突）に関わらず、**awase 自身の `WH_KEYBOARD_LL` フックにすら
到達していない**ことが2回連続で確認された。これは「scan コードが CapsLock と
衝突するから横取りされる」という追補1の仮説（scan 依存の問題）では説明が
つかない——scan=0 は物理キーに対応しないため衝突しないはずだが、それでも
届かない。より根本的な原因として、`KEYEVENTF_SCANCODE` を付けずに
`SendInput` した場合、OS（win32k）が `wScan` の値を無視し、`wVk` から
`MapVirtualKeyW` 相当の内部変換で scan を独自に再計算して
`KBDLLHOOKSTRUCT.scanCode` を構築している可能性がある——だとすれば、我々が
`wScan=0` を指定しても実際にフックへ渡る scan は結局 OS が再計算した値
（0x3A 等）になり、`wScan` フィールドを変えたところで到達性は変わらない。
真因の完全な特定には至っていないが、**「scan を変えれば届く」という仮説は
2回の実機失敗で反証された**。

**対応:** GJI 向けの entry 機構（scan 付き注入・scan=0 注入・IMC write の
いずれも）を全て撤回し、**GJI では entry を一切試みない**方針に変更した
（`kp_shift_conv_guard_key_down`: `active_ime_kind != MicrosoftIme` の場合は
ログのみで `SendInput`/IMC write を送らない）。加えて、**左Shift単独タップの
検出自体は行うが、GJI では持続トグルへ絶対に移行しない**よう
`kp_shift_conv_guard_key_up` にガードを追加した（`toggle_entry_supported =
active_ime_kind == MicrosoftIme` を tap 判定に AND する）。理由: entry が
機能しないまま `half_width_alnum_toggle_active` を立てて engine を
pass-through にすると、生ローマ字キーが GJI 自身の未切替のひらがな変換
エンジンにそのまま入り「かな入力が壊れる」という**新たな実害**が生まれる
（今回まさにこれが発生した）。entry 機構が無い IME 種別では、機能を丸ごと
無効化する方が安全側と判断した。MS-IME 側（IMC write, 既存経路）は変更なし。

**未解決（今後の課題）:** GJI に対して実際に半角英数へ切り替える手段は
まだ見つかっていない。次の候補として、mozc の `TipTextService` が実装する
`ITfLangBarItemButton`（言語バーのモード切替アイコン）を `ITfLangBarItemMgr`
経由で列挙し `OnClick` を呼ぶ案がある——これは本物の UI クリックと同じ
`SwitchInputModeAsync` 経路を通るはずで、`SendInput` によるキーイベント
注入という失敗し続けている手段そのものを迂回できる。COM インターフェースの
呼び出しであり `SendInput`/フックの介在が無いため、今回までの2つの失敗
（scan 依存問題）とは独立した経路になる。未着手・未検証。Windows crate の
`Win32_UI_TextServices` feature は既に有効化済み（`Cargo.toml`）。

**教訓:** 「scan を変えれば届く」という一見もっともらしい仮説も、実機で
2回連続反証されている以上、3回目に同種の「scan の値を変える」バリエーションを
試すべきではない。`SendInput` による `VK_DBE_ALPHANUMERIC` 注入という**手段
そのもの**（scan の値によらず）が機能しないと考えるべきであり、次に検討すべきは
異なる制御チャネル（COM/UI Automation 等）である。また、entry が機能しない
状態のまま持続トグルの belief だけを進めると、「何も起きない」より悪い
「かな入力が壊れる」という新規リグレッションを生む——**機構が実証されるまでは
機能自体を無効化する**方が安全側の設計判断になる。

**テスト:** 自動テスト不可（実機の GJI TIP・OS 入力パイプライン挙動に依存）。
この追補が再発防止の記録。次回 GJI entry を検討する際は、必ず
`ITfLangBarItemButton` のような非 `SendInput` 経路から着手し、`SendInput`
ベースの `VK_DBE_ALPHANUMERIC` 注入（scan の値を問わず）を再試行しないこと。

**関連ファイル（追補3）:** `crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_shift_conv_guard_key_down`: GJI entry を全撤去、`kp_shift_conv_guard_key_up`:
`toggle_entry_supported` ガード追加）

---

## BUG-26: FocusChanged 直後 conv が既に NATIVE の場合、idle-conv-check の steady-state 分岐が engine 復帰を永久に見送る

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` → `Windows.UI.Input.
InputSite.WindowClass`、`WindowsTerminal.exe`）へフォーカスが移ってから最初の
キー入力まで、engine が `Inactive(ImeOff)` のまま復帰せず、NICOLA 変換が一切
発火せずローマ字（英字）がそのまま通る。実機ログでは `[idle-conv-check]
TsfNative: conv=0x00000019 → belief ObservedRomaji 変更なし` が 30 秒以上、
数十回にわたって出力され続けるが、一度も `EngineSync::ReportOpenInference`
（engine 復帰の唯一の経路）が発火しなかった。

**IME:** GJI（Google 日本語入力）。conv=0x00000019（`NATIVE`+`FULLSHAPE`+
`ROMAN`、ひらがなローマ字）で TSF native の conv 読み取りは正しく Hiragana を
示していた（`[ime-mode] initial confirm: Hiragana (conv=0x00000019)`）。つまり
実際の IME はローマ字入力可能な状態であり、awase の belief（`ImeModel::
desired_open`、グローバル単一フラグ）だけが false のまま乖離していた。この
false は当該フォーカス変更より前に、別の Imm32Unavailable ウィンドウの
`HwndCacheRestored`（`last_intent` を設定しない直接書き込み）で仕込まれた
可能性が高いが、確定はできていない（発生源の特定は別途）。

**再現手順（コード上で確認、実機はログのみ）:** (1) 何らかの経路で
`desired_open=false` が設定される（`HwndCacheRestored` 等、`last_intent` 不設定）。
(2) TsfNative なウィンドウへフォーカスが移り、`FocusChanged` が
`observations`/`explicit_intent` をクリアする。(3) `ConvModeMgr` がこの
ウィンドウの conv を初めて読んだ時点で既に NATIVE（例: 0x19）を保持しており、
以後 `update_from_conv` が「変化」を検出しない（`conv_mode_changed` が一度も
`true` にならない）。(4) `crates/awase-windows/src/state/conv_classify.rs::
classify_conv_transition` の steady-state（`conv_mode_changed=false`）分岐は
修正前、`has_katakana && has_native` の場合のみ `EngineSync::
ReportOpenInference` を返し、非カタカナの NATIVE（= 通常のひらがな/JISかな、
まさに今回の 0x19）は無条件で `EngineSync::None` を返していた。「conv 不変:
カタカナ+shadow=OFF のみが唯一の回復経路」という設計コメントが実際にその
通りに実装されており、非カタカナ NATIVE の steady-state 回復手段が存在しな
かった。同じ関数の `conv_mode_changed=true` 分岐は非カタカナ NATIVE でも
`NativeToggleShadowOff` を返すため、ここが唯一の非対称な抜け穴だった。

**修正 (2026-07-17):** `classify_conv_transition` の `input_mode_update=None`
分岐から `conv_mode_changed` によるゲートを撤去し、`has_native && !effective_open`
であれば `conv_mode_changed` の真偽に関わらず `EngineSync::
ReportOpenInference`（`has_katakana` の有無で `KatakanaShadowOff` /
`NativeToggleShadowOff` を選ぶ）を返すようにした。`ReportOpenInference` は
`desired_open` を直接書き換えず `ObserverReported`（`ConvOpenInference`,
confidence=Medium）として記録するだけであり、実際の補正可否は既存の
`check_drift_correction`（`explicit_intent` 必須ゲート、BUG-19/BUG-20 で
すでに堅牢化済み）に委ねられる — つまり今回の変更は「conv 由来の open 推論を
記録する頻度」を広げただけで、`desired_open` への書き込み経路自体は増やして
いない。`effective_open()`（`derive_open()` 経由）は Medium confidence 単独
ソースでも即採用するため、この観測が記録された時点で engine の
`ctx.ime_on` 判定はすぐに真に復帰する。

**テスト:** `crates/awase-windows/src/state/conv_classify.rs::tests::
hiragana_belief_romaji_capable_shadow_off_steady_state_still_syncs_engine`
を追加（conv=0x19, `conv_mode_changed=false`, `effective_open=false` →
`ReportOpenInference(NativeToggleShadowOff)` を期待）。既存の
`smoke_all_major_conv_belief_combinations`（conv×belief×open×changed 全数
スモーク）・`hiragana_belief_romaji_capable_shadow_off_syncs_engine`（変化
あり版）を含め lib 139・architecture_guard 10・golden_scenarios 20・
journal_replay 1・layer_boundary_guard 8 は全通過（Linux、cross-compile の
ため Windows 実機での再現確認は未実施）。

**関連ファイル:** `crates/awase-windows/src/state/conv_classify.rs`
（`classify_conv_transition`）。

---

## BUG-27: per-VK confirm ループが `vk_sent 未設定` を検出すると、リカバリなしで romaji（と巻き込んだ後続文字）を丸ごと失う

**症状:** Chrome で「はだいじょうぶ」と入力したはずが「いじょうぶ」になった（先頭2文字
「は」「だ」が完全に欠落。前半のみリテラル化する BUG-24 系とは異なり、痕跡もなく消える）。
実機ログの核心:

```
[tsf-probe] cold=151 ChromeProbe 完了 (344ms)
Timer set: logical=105, ms=10, os_id=15899
[gji-obs] candidate SHOW #325: last_gji_write=360ms ago
[gji-fsm] StartComposition (candidate SHOW)
[gji-fsm] StartComposition while cold (probe running) → AwaitingProbe
[tsf-probe-tick] cold=151 t=842709406ms
WARN [tsf-probe] cold=151 Chrome per-VK[0/1] vk_sent 未設定 → 中断
```

**IME:** GJI（Google 日本語入力）。`DIAG_CHROME_USE_PER_VK_CONFIRM`（Chrome cold-start
の per-VK confirm 実験、デフォルト有効）が動いている状態。

**原因:** `crates/awase-windows/src/tsf/warmup/probe_fsm.rs::tsf_probe_coro_body`
（Chrome）・`crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs::gji_coro_body`
（TSF/WezTerm）の per-VK confirm ループは、1 VK 送信するたびに dispatcher
（`output/probe_io.rs::dispatch_probe_actions`）が `apply_vk_sent()` を呼んで
`pending_vk_sent` を埋める前提で、次の `tick()` でそれを読み出す:

```rust
let Some(sent) = vk_input.vk_sent else {
    log::warn!("... vk_sent 未設定 → 中断");
    return;  // 修正前: ここにリカバリが一切ない
};
```

この前提が崩れたときの防御分岐（`else`）に、`SuspectedLiteral` 検出時と違って
**一切のリカバリがなかった**。単に `return` するだけなので:

1. 今まさに送信中だった romaji（「は」）自身が、途中の VK（H）で送信が止まり
   `literal_session_confirmed()` も立たないまま放置される。
2. さらに深刻なのは、この probe が in-flight の間に別の文字（「だ」）が来ていた場合、
   `TsfWarmupCoordinator::defer_vks_if_in_flight` で coordinator 側の待避キュー
   （`pending_deferred`）に積まれるが、**このキューが flush されるのは per-VK
   ループの最後の VK（`is_last`）到達時だけ**（`output/probe_io.rs` の
   `TransmitSingleVk` ハンドラ内）。`vk_sent 未設定` は `is_last` 到達前に
   `return` するため、この flush ポイントに二度と到達できず、待避されていた
   後続文字も道連れで失われる。これが「は」だけでなく「だ」まで消えた理由の
   有力な説明（`pending_deferred` が実際にこの経路で失われたことをログから
   直接は確認できていないが、コード上は本経路のみがこの flush をスキップする）。

`vk_sent` がなぜ `None` のまま次の tick に渡るのか（トリガー自体）は未特定。
以下は調査で**否定できた**候補: `target==Tsf` 専用の `gate_is_bypass` 早期
リターン（今回は target=Chrome で非該当）／`notify_start_composition()`
（`TsfProbeCoro` はデフォルト no-op、override しているのは `SacrificialWarmupCoro`
のみ）／GjiFsm の `StartComposition while cold` ハンドリング（`CancelProbe` を
出さないことがテストで保証されている＝probe を破壊しない）。ログ上は
`drain_pending_composition_events()`（`advance_tsf_probe` 冒頭、`step_probe` より前）
が処理する候補ウィンドウ SHOW イベントと同じ WM_TIMER 呼び出し内で発生している
ことは分かっているが、両者が実際に競合する経路は未発見。

**修正 (2026-07-17):** `vk_sent` が `None` の場合を `DetectionResult::
SuspectedLiteral` と同じ扱いにし、`literal_detect_fsm::per_vk_recovery_params(idx)`
で backs/escape_composition を求めて `emit_recovery_actions` 経由の
backspace + romaji 再送リカバリを emit するようにした（Chrome/TSF 両方）。これで
この VK 自身は literal 扱いとして回収され、次の cold パス（per-VK confirm）で
改めて送り直す機会を得る。

**未解決の follow-up（本コミットのスコープ外）:** 上記の「coordinator の
`pending_deferred` が `is_last` 到達前の early-exit で flush されない」構造的な穴は
`vk_sent 未設定` に限らず `SuspectedLiteral`（`is_last` より前の idx で検出された
場合）にも共通して存在する。今回のリカバリは「この VK 自身」の再送は保証するが、
probe 中に来ていた**別の文字**の救済（`pending_deferred` の扱い）までは踏み込んで
いない。次に着手する場合は、per-VK ループの早期 exit 経路すべてで
`take_pending_deferred_vks()` を呼ぶか、リカバリ後の再送 romaji に含める設計が
必要。

**テスト:** `crates/awase-windows/src/tsf/warmup/probe_fsm.rs::tests::
chrome_per_vk_vk_sent_unset_recovers_instead_of_silently_dropping`
を追加（`apply_vk_sent` を呼ばずに次の `tick()` を実行し `vk_sent=None` を
再現、`RawTsfLiteralRecovery{backs:1, escape_composition:false}` + `Done` が
emit されることを確認）。Windows target ビルド・テストコンパイルは警告ゼロで
確認済みだが、cross-compile のため実行はできず、Windows 実機での再現確認は
未実施。`gji_warmup_coro.rs`（TSF/WezTerm 側）には既存のユニットテスト基盤が
無いため、同型の修正はコードレビュー＋本記録のみで担保する。

**関連ファイル:** `crates/awase-windows/src/tsf/warmup/probe_fsm.rs`
（`tsf_probe_coro_body`）、`crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs`
（`gji_coro_body`）。

**追補1（2026-07-17）: `vk_sent` が `None` になるトリガー自体を特定するための
診断ログを追加した。** 次に実機で再現したら `RUST_LOG=trace`（`take_pending_tsf`/
`restore_pending_tsf`/`install_pending_tsf` は trace 級、それ以外は debug 級）で
以下のタグを時系列で突き合わせること:

- `[tsf-probe-vk-sent-trace]` / `[gji-coro-vk-sent-trace]` — `apply_vk_sent SET` と
  `tick consuming pending_vk_sent=...` を cold_seq・t=...ms 付きで出す。
  `apply_vk_sent SET overwritten_unconsumed=true` が出ていれば「1 tick 内で
  `TransmitSingleVk` が2回ディスパッチされ、前回分が上書きされて消えた」ことが
  確定する。`tick consuming pending_vk_sent=false` の直前に対応する
  `apply_vk_sent SET` が無ければ、そもそも `apply_vk_sent` 自体が呼ばれていない
  （`dispatch_probe_actions` 側の分岐漏れ）ことになる。
- `[tsf-probe-coord]` — `take_pending_tsf` → `restore_pending_tsf` の1サイクルが
  cold_seq 込みで正しく対になっているか、`install_pending_tsf`（新規/上書き）が
  意図しないタイミングで挟まっていないかを確認する。`overwriting in-flight probe
  cold=X with new probe cold=Y` の `warn!` が出ていれば、machine 自体が
  途中ですり替わっている（今回の失敗の有力候補の一つ）。

`crates/awase-windows/src/tsf/warmup/probe_fsm.rs::TsfProbeCoro::{tick,apply_vk_sent}`、
`gji_warmup_coro.rs::GjiWarmupCoro::{tick,apply_vk_sent}`、
`output/tsf_warmup_coord.rs::{take_pending_tsf,restore_pending_tsf,install_pending_tsf,
clear_pending_tsf}` が対象。挙動は変えていない（ログ追加のみ、テスト全通過）。

**追補2（実機確認・撤回、2026-07-17）: backspace+再送リカバリが msedge で入力を
全面破壊した。**

**アプリ:** msedge（`Chrome_WidgetWin_1`、hwnd=0x25097a、`profile=Imm32Unavailable`）。

**IME:** GJI（Google 日本語入力）。`DIAG_CHROME_USE_PER_VK_CONFIRM` 動作中。conv 等は
不明（`himc_null=true` のため `[comp-probe]` の open/conv 系フィールドは全て `-`）。

**再現手順 / 症状:** 「書いたそばから Backspace されて、まったく何も入力できません」。
実機ログで `vk_sent 未設定` が **打鍵のたびに毎回**（cold=99,100,101,102,103,104,105...
と1文字ごとに新しい cold_seq で）発火し、`[raw-tsf-literal] consecutive
raw-tsf-literal (count=N)` が 6→7→8→9→10→11→12 と単調増加して一度も 0 に戻らな
かった。`count>0` は「give up, backspace ×1 のみ（再送なし）」分岐（`probe_io.rs`
の `RawTsfLiteralRecovery` ハンドラ）に固定で落ちるため、実質「打鍵→即
backspace ×1→次の打鍵も同様」の繰り返しになり、何も入力できなくなった。
candidate SHOW/HIDE の WinEvent 自体は正常に回っており（`で`→`き`→`て`→`い`→
`る`→`か` の各文字で `StartComposition`/`EndComposition` が観測されている）、
VK 自体は正しく GJI に届いて composition が処理されていた可能性が高い。

**なぜ元に戻すと直るのか:** BUG-27 本編の修正（`vk_sent 未設定` を
`SuspectedLiteral` と同じ backspace+romaji 再送リカバリとして扱う）は、
「はだいじょうぶ」→「いじょうぶ」の1回の実機観測（`consecutive=0` で resend
された）を根拠にしていたが、この追補2の実機では `vk_sent 未設定` が
**信頼できない・むしろ頻発するシグナル**であることが分かった。頻発すると
`consecutive` が 0 に戻る間もなく積み上がり、常に「resend なしの backspace
のみ」に落ちるため、正しく打てていた文字まで機械的に削除し続ける。
`SuspectedLiteral`（実際に literal 化を検出した場合）とは異なり、この防御
分岐は「本当に literal 化したかどうか」を何も確認していないため、
積極的なリカバリ（backspace）はむしろ有害と判断し、無リカバリの `return`
に戻した。

**根治の方針（未着手）:** `vk_sent` が `None` になるトリガー自体（追補1参照）を
特定しない限り、この防御分岐に対する「正しい」リカバリは設計できない。今回
「毎打鍵で発火する」という頻度の情報が新たに得られたことで、まれなレース
ではなく **システマティックな要因**（例: idle-conv-check の
`get_ime_conversion_mode_raw_timeout(10)` が `SendMessageTimeoutW` を同期的に
呼んでおり、そのメッセージポンプ中に `TIMER_TSF_PROBE` が再入し、
`pending_vk_sent` の set/consume 順序を乱している可能性）を疑う次の調査の
足がかりになる。次に着手する場合は `RUST_LOG=trace` で追補1の診断ログ
（`apply_vk_sent SET overwritten_unconsumed=...`／`tick consuming
pending_vk_sent=...`）と `idle-conv-check` のタイミングを突き合わせること。

**テスト:** `probe_fsm.rs::tests::chrome_per_vk_vk_sent_unset_does_not_backspace`
（旧 `chrome_per_vk_vk_sent_unset_recovers_instead_of_silently_dropping` を置換）。
`vk_sent` 未設定時に `RawTsfLiteralRecovery` を一切発行せず `ProbeAction::Done`
のみを返すことを固定する。Windows target ビルド・テストコンパイルは警告ゼロ
で確認済み（cross-compile のため実行はできず、この revert 自体の実機再検証は
未実施）。

**追補3（根本原因確定・修正、2026-07-17）: `vk_sent` が `None` になるトリガーは
レースではなく、`ChromeProbe` ラッパーの委譲漏れという単純なバグだった。**

**発見の経緯:** revert 後も「こんにちはこんばんはありがとう」→
「ｋんにちはこんばんはあｒがとう」（"こ"→"ｋ"のみ、"り"→"ｒ"のみ、いずれも
romaji 2文字のうち1文字目だけが物理送信されて2文字目が送られない）が
msedge / Microsoft Teams (TeamsWebView) で再現し続けた。実機ログで
`vk_sent 未設定` の**直前に必ず出るはずの**追補1の診断ログ
（`[tsf-probe-vk-sent-trace] cold=N apply_vk_sent SET ...`）が**一度も
出ていない**ことに気づいた——同じファイル・同じログレベルの他の debug ログ
（`ChromeProbe 完了` 等）は正常に出ており、`dispatch_probe_actions` の
`TransmitSingleVk` ハンドラを読む限り Chrome ターゲットでは無条件で
`machine.apply_vk_sent(...)` に到達するはずで、静的読解だけでは矛盾を
説明できなかった。Codex CLI（`codex exec -s read-only`）にリポジトリを
読み取り専用で調査させ、数分で特定にたどり着いた。

**真の原因:** `pending_tsf: Box<dyn TickableFsm>` に実際に格納されているのは
`TsfProbeCoro` そのものではなく、`crates/awase-windows/src/tsf/warmup/
chrome_probe.rs` の `ChromeProbe(TsfProbeCoro)` という**ラッパー型**だった。
`ChromeProbe` の `TickableFsm` 実装は `tick` / `cold_seq_hint` /
`apply_transmit_done` の3メソッドは内側の `TsfProbeCoro` へ委譲していたが、
**`apply_vk_sent` の委譲が欠けていた**。`TickableFsm::apply_vk_sent` には
デフォルト no-op（`tickable_fsm.rs`）が定義されているため、コンパイラは
何も警告せず、`dispatch_probe_actions` が呼ぶ `machine.apply_vk_sent(...)`
は静かに `ChromeProbe` のデフォルト no-op に落ちて**何もしないまま**
戻っていた。内側の `TsfProbeCoro::apply_vk_sent`（追補1で診断ログを
仕込んだメソッド）は一度も呼ばれないため `pending_vk_sent` が常に `None` の
ままで、次 tick で per-VK confirm ループが「vk_sent 未設定」を検出していた。
VK 自体は `dispatch_probe_actions` 側の `io.send_single_chrome_vk(...)` で
**物理的には正しく送信されている**ため、これは「レースで時々起きる」
ものではなく、**Chrome per-VK confirm が動くたびに毎回・確実に**
1文字目で発生する構造的バグだった（TSF/WezTerm 側の `GjiWarmupCoro` は
`ChromeProbe` のようなラッパーを介さず直接 `pending_tsf` に格納されるため、
この不具合の対象外——実際、これまで観測された全ての事例が
"Chrome per-VK"（`tsf_probe_coro_body`）だけで、"gji-coro"（WezTerm側）では
一度も再現していない）。

**修正:** `ChromeProbe` の `TickableFsm` 実装に `apply_vk_sent` の委譲を追加。

```rust
fn apply_vk_sent(&mut self, detector: LiteralDetector, deadline_ms: u64) {
    self.0.apply_vk_sent(detector, deadline_ms);
}
```

`tickable_fsm.rs` の実装一覧コメントも更新し、`ChromeProbe` が
`apply_transmit_done`/`apply_vk_sent` を内側へ委譲していることを明記した
（旧コメントは「なし」となっており、この見落としを誘発しやすかった）。

これにより追補2で撤回した backspace リカバリ（BUG-27 本編）が実は不要
だった可能性が高い——`vk_sent` が正しく `apply_vk_sent` に届くようになれば
per-VK confirm はそもそも `vk_sent 未設定` に到達せず、`SuspectedLiteral` /
`CompositionConfirmed` の通常の判定に進むはずである。ただし撤回した
backspace リカバリを**再度有効化する必要はない**——今回の根本修正で
`vk_sent 未設定` の到達頻度自体が激減するはずなので、無リカバリの `return`
のままで実害はほぼ無くなる見込み。

**テスト:** `chrome_probe.rs::tests::chrome_probe_apply_vk_sent_reaches_inner_coro`
を追加。`probe_fsm.rs` の既存テストは `TsfProbeCoro` を**直接**構築するため
`ChromeProbe` の委譲漏れを検出できなかった（テストが通っていたのに実機では
毎回再現した理由）。新テストは本番と同じ `ChromeProbe`（`TickableFsm` トレイト
経由）を使い、`apply_vk_sent` 呼び出し後の `tick()` が「vk_sent 未設定」で
即 `Done` を返さず detection 待ちの polling に入ることを確認する。Windows
target ビルド・テストコンパイルは警告ゼロで確認済み（cross-compile のため
実行はできず、実機再検証は未実施）。

**関連ファイル（追補3）:** `crates/awase-windows/src/tsf/warmup/chrome_probe.rs`
（`ChromeProbe::apply_vk_sent` 追加）、
`crates/awase-windows/src/tsf/warmup/tickable_fsm.rs`（実装一覧コメント更新）。

**追補4（修正、2026-07-17）: `consecutive_count`（連続 literal 失敗カウンタ）が
`CompositionConfirmed` では一度もリセットされず、セッション中に一度でも
literal 化すると以後ずっと give-up＝backspace のみに固定される regression。**

追補3の修正後、実機で再テストしたところ `vk_sent 未設定` は解消されたが、
今度は正当な `DetectionResult::SuspectedLiteral`（本物の検出）が
Microsoft Teams (TeamsWebView) で頻発し、`[raw-tsf-literal] consecutive
raw-tsf-literal (count=N)` が cold=12→13→14 と N=4→5→6 と単調増加し、
一度も0に戻らないことが分かった（ユーザー報告: 「した という風に何度か
入力していますが、バックスペースで消されているかんじがします」）。

`crates/awase-windows/src/tsf/probe.rs` の `ColdContext::reset_consecutive_count()`
の呼び出し元を調べたところ、リセットされるのは `CompositionState::
on_focus_changed()`（フォーカス変更時）と `mark_composition_cold(SetOpenTrue)`
（engine が新たに ON になった時）の2箇所のみで、**「文字が正しく確認できた
（`DetectionResult::CompositionConfirmed`、非 partial）」では一度もリセットされて
いなかった**。`consecutive_count` は「連続 RawTsfLiteralRecovery」抑止用の
カウンタであり、間に本物の confirm が挟まれば連続ではなくなるはずだが、その
リセット経路が存在しなかった。

Codex CLI（`codex exec -s read-only`）に相談し、`ProbeAction` に
`CompositionConfirmed { mark_literal_session: bool }` を追加して dispatcher
（`probe_io.rs::dispatch_probe_actions`）に一元化する方針を確認した。

**修正:** `CompositionState::reset_consecutive_count()`（`ColdContext` への
public wrapper）、`ProbeIo::reset_consecutive_count()` を追加し、
`ProbeAction::CompositionConfirmed { mark_literal_session }` を dispatcher で
処理して `io.reset_consecutive_count()` を必ず呼ぶ（`mark_literal_session=true`
なら `tsf::observer::mark_literal_session_confirmed()` も呼ぶ）ようにした。
呼び出し箇所:

- `literal_detect_fsm.rs::LiteralDetectCore::poll` の非 partial
  `CompositionConfirmed` 分岐（warm パス、Chrome/TSF 共有）。
- `probe_fsm.rs::tsf_probe_coro_body`（Chrome per-VK confirm）: 各 VK の
  confirm で `mark_literal_session=false` のリセットを次の `TransmitSingleVk`
  yield に相乗りさせ、全 VK 確認後にのみ `mark_literal_session=true` を送る
  （1 VK 目は成功したが2 VK 目で `SuspectedLiteral` になったケースでも
  `consecutive` が正しくリセットされている状態から再送判定できるようにするため）。
- `gji_warmup_coro.rs::gji_coro_body`（TSF/WezTerm per-VK confirm）: 同様。

**テスト:** `chrome_per_vk_vk_sent_unset_does_not_backspace` は影響を受けない
（`apply_vk_sent` を呼ばないテストのため per-VK ループの confirm 分岐に
到達しない）ことを確認。個別の `ProbeAction::CompositionConfirmed` dispatch の
単体テストは今回は追加していない（`FakeProbeIo` に `reset_consecutive_called`
フラグは追加済み、今後の回帰テスト追加の土台とする）。lib 139・
architecture_guard 10・golden_scenarios 20・journal_replay 1・
layer_boundary_guard 8 全通過、Windows cross-compile 警告ゼロ確認済み
（実機再検証は未実施）。

**関連ファイル（追補4）:** `crates/awase-windows/src/tsf/probe.rs`
（`CompositionState::reset_consecutive_count`）、
`crates/awase-windows/src/output/probe_io.rs`（`ProbeIo::reset_consecutive_count`、
dispatcher）、`crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs`、
`crates/awase-windows/src/tsf/warmup/probe_fsm.rs`、
`crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs`。

**追補5（根本原因の疑いを再検証・修正、2026-07-17）: Chrome per-VK confirm の
検出方式が候補ウィンドウ SHOW を一切見ておらず、子音単体 VK を誤って
`SuspectedLiteral` と判定していた。**

追補4の修正後もなお、ユーザーから「表層的すぎないか」という指摘があり
再調査した。実機ログで `apply_vk_sent SET` → `tick consuming
pending_vk_sent=true` が正しく出ている（＝追補3の修正は効いている）のに、
約300ms（`RAW_TSF_LITERAL_DETECT_MS`）待った末に `Chrome per-VK[0/1]
suspected literal` と判定されるケースが "し"（romaji "si" の "s"）・
"た"（romaji "ta" の "t"）等、**romaji 2文字の1文字目（子音）で一貫して**
発生していた。ユーザーからは「候補ウィンドウは目で見えているのに検知できて
いないのでは」という指摘があった。

`crates/awase-windows/src/tsf/probe.rs::LiteralDetector::check_now` を確認した
ところ、Chrome ターゲットの per-VK confirm は毎回
`new_gji_resumed_with_pre_send_baseline(gji_write_bytes())` で detector を
生成しており、これは常に `write_bytes_baseline = Some(...)` になる。
`check_now` はこの場合 **`gji_candidate_show`（候補ウィンドウ SHOW イベント）を
一切見ず**、GJIプロセスの WriteTransferCount が
`COMPOSITION_BYTES_THRESHOLD`（350バイト）を超えて増加したかだけで判定していた。
この350バイトという閾値は「VK_A→'あ' のように1VKで完結する1文字」の実測
（5サンプル）に基づく値で、per-VK confirm が子音単体（まだ romaji バッファが
未確定の状態）を送った直後に問い合わせるケースは実測対象外だった。実機ログでは
候補ウィンドウの SHOW イベント自体は正常に観測できていた
（`[gji-obs] candidate SHOW #19` 等）ため、**合成は実際に起きているのに検出方式が
それを拾えていなかった**と判断した。

Codex CLI に2回目の相談（読み取り専用でコードを再調査させ、上記の分析と
一致することを確認）し、推奨された最小修正（write-bytes 閾値と SHOW
イベントの OR 判定）を採用した。

**修正:** `LiteralDetector::check_now` の `write_bytes_baseline: Some(_)` 分岐に
`gji_candidate_show.has_changed(self.gji_show_baseline)` を OR 条件として追加した。
`gji_show_baseline`/`was_candidate_visible` は `new_gji_resumed_with_pre_send_
baseline` が内部で呼ぶ `Self::new()` で既に取得済みのため、追加のフィールドや
コンストラクタ分岐は不要。この変更は Chrome per-VK confirm だけでなく
`new_gji_resumed`/`new_gji_resumed_with_pre_send_baseline` を使う全経路
（`StartSacrificialWarmup` の Chrome パス含む）に適用される（OR 条件のため
既存の write-bytes 検出を弱めることはなく、より早く／確実に確認できるように
なるだけ）。

**既知の限界:** 直前の VK 送信で候補ウィンドウが既に表示中だった場合、
`gji_candidate_show` は「新規表示」でのみ増分するため、続く VK では SHOW が
増えないケースがあり得る。その場合は従来通り write-bytes 閾値に委ねる
（OR 条件のため、どちらか一方が拾えれば確認できる）。今回の実機症状
（子音単体の1VK目、SHOW が新規に発火するケース）はこれでカバーされる。

**テスト:** `tsf/probe.rs::tests::
check_now_confirms_via_candidate_show_when_write_bytes_below_threshold`
（write-bytes 閾値未達でも SHOW があれば confirmed になることを確認）、
`check_now_still_detects_suspected_literal_when_neither_signal_fires`
（両シグナルとも無ければ従来通り SuspectedLiteral になることを確認、
本物の literal 化検出の回帰防止）を追加。Windows cross-compile 警告ゼロ
確認済み（cross-compile のため実行はできず、実機再検証は未実施）。

**関連ファイル（追補5）:** `crates/awase-windows/src/tsf/probe.rs`
（`LiteralDetector::check_now`）。

---

## BUG-28: `flush_raw_tsf_literal_recovery` が `pending_gji_key_responses` を drain せず、`StartProbe` が数秒〜数十件分まとめて burst 発火する

**症状:** WindowsTerminal（`CASCADIA_HOSTING_WINDOW_CLASS`、TSF mode）で最初の1文字
「な」を送信した直後、実機ログで `[gji-fsm] StartProbe probe_id=ProbeId(N)` が
`N=14`〜`42`（29件）まで**同一ミリ秒内に他のログを一切挟まず連続発火**した。
ユーザー報告は「なぞのバックスペースの無限ループが発生しました」。この burst
自体は backspace ではなく `GjiFsm` の `StartProbe` action だが、直前の約8秒間は
TeamsWebView/Chrome での Chrome per-VK confirm による raw-tsf-literal 回収
（`[raw-tsf-literal] re-sending raw TSF literal romaji="ni"`）と
`VirtualDesktopHotkeySwitcher` 経由の激しいフォーカス切替が続いていた。

**IME:** GJI（Google 日本語入力）。TSF mode（`mode=Tsf`、WindowsTerminal 等）と
Vk mode（`mode=Vk`、Chrome/TeamsWebView 等）の両方に影響する。

**再現手順:** raw-tsf-literal リカバリ（`WM_DRAIN_OUTPUT_QUEUE` ハンドラ経由）が
複数回発生した直後に、通常の `send_keys()`（実際のキー入力）が呼ばれると、
undrained のまま溜まっていた `GjiResponse`（`StartProbe` を含む）が一括で
dispatch・ログ出力される。

**原因:** `GjiEvent::KeyInput` の `Response`（`GjiAction::StartProbe` を含みうる）は
即座に dispatch されず、`Output::push_key_response`（`tsf_warmup_coord.rs`
`pending_gji_key_responses: RefCell<Vec<GjiResponse>>`）に一旦バッファされる。
これを実際に drain・dispatch（`"[gji-fsm] StartProbe probe_id=..."` のログ出力
はここで発生する）するのは `WindowsPlatform::send_keys`
（`platform.rs` 旧656-658行）の中だけだった。

一方、`WindowsPlatform::flush_raw_tsf_literal_recovery`（`platform.rs` 569-574行、
`WM_DRAIN_OUTPUT_QUEUE` ハンドラから呼ばれる）は内部で
`Output::flush_raw_tsf_literal_recovery` → `flush_raw_tsf_literal_romaji` →
`send_romaji_as_tsf`/`send_romaji_batched` を呼び、これが同じく
`push_key_response` で `pending_gji_key_responses` に積む。しかしこの関数は
`send_keys` を経由しないため、`pending_tsf_timer()` の補完だけを行い
（コメントで「`platform.send_keys` を経由しないため、ここでタイマー設定を
補完する」と明記されていたが、これは4つの後処理のうち1つだけだった）、
`drain_pending_gji_key_responses`／`take_composition_reset`／
`drain_pending_composition_events` は**行っていなかった**。

結果として、raw-tsf-literal リカバリが発生するたびに `pending_gji_key_responses`
にエントリが積まれるが、次に本物の `send_keys()`（実際のキー入力）が呼ばれる
まで一切 drain されない。各エントリの `GjiFsm::on_event(KeyInput)` 自体は
push 時点（＝実際に古い時刻）に同期的に評価・状態遷移済みだが、ログ出力と
一部の副作用（`gji_store_probe_id` 等）だけが後から一括で発生するため、
数秒〜数十秒越しの stale な `StartProbe` が同一ミリ秒内に burst するように見える。

**修正 (2026-07-17):** `send_keys` が `output.send_keys(actions)` の直後に行っていた
4つの後処理（`drain_pending_gji_key_responses`+dispatch、
`take_composition_reset`+`gji_on_composition_reset`、
`drain_pending_composition_events`、`pending_tsf_timer`+`apply_timer_command`）を
`WindowsPlatform::drain_output_post_send_effects` として抽出し、
`send_keys` と `flush_raw_tsf_literal_recovery` の両方から呼ぶようにした。

**テスト:** `WindowsPlatform` は実 Win32 タイマー/フック等に依存するため
Linux 上でのユニットテストは非現実的（`golden_scenarios.rs` 等の既存テストは
`Output`/reducer レベルを直接駆動しており `WindowsPlatform::send_keys` 自体は
経由しない）。本記録で代替する。lib 139・architecture_guard 10・
golden_scenarios 20・journal_replay 1・layer_boundary_guard 8 全通過、
Windows cross-compile（build + test --no-run）警告ゼロ確認済み。実機再検証は
未実施。

**関連ファイル:** `crates/awase-windows/src/platform.rs`
（`WindowsPlatform::send_keys`、`flush_raw_tsf_literal_recovery`、新設
`drain_output_post_send_effects`）。

---

## BUG-29: Chrome per-VK confirm が VK1 以降を誤って `SuspectedLiteral` 判定し、
無音で入力が消え続ける

**症状:** Chrome/TeamsWebView（`DIAG_CHROME_USE_PER_VK_CONFIRM` 実験、
`experiment/skip-cold-probe-wait` ブランチ）で、romaji の2文字目以降が実際には
正しく入力できているにもかかわらず `[tsf-probe] cold=N Chrome per-VK[idx/last]
suspected literal` と誤検知され、backspace リカバリ（`RawTsfLiteralRecovery`）が
繰り返し発火する。`raw_tsf_literal_consecutive_count`（`tsf/probe.rs:277`）は
`CompositionConfirmed`/`FocusChange`/`SetOpenTrue` でしか 0 に戻らないため、
2回連続で誤検知すると `probe_io.rs:842-855` の give-up 分岐（backspace のみ、
romaji 再送なし）に落ち、以後フォーカス変更するまで打鍵した文字が無音で
消え続ける。ユーザー実機報告（2026-07-17）:「書いたそばから Backspace されて、
まったく何も入力できません」「入力が全く反映されない/消える」。BUG-27 追補2
（count 6→7→…→12 と単調増加）と同一の外形症状だが、そちらの根治
（`91040ab`/`12c8dda`/`21fdc47`）は個別トリガー（ChromeProbe の `apply_vk_sent`
未委譲）を潰しただけで、本 BUG の検出漏れ自体は温存されていた。

**IME:** Google 日本語入力（GJI）。Chrome/TeamsWebView の cold-start per-VK
confirm 経路のみ（TSF/WezTerm 側の `gji_coro_body` Phase 5b は detector 構築が
異なるため対象外、末尾の follow-up 参照）。

**再現手順:** 複数 VK からなる romaji（例:「ltu」＝L→T→U、「ha」＝H→A）を
per-VK confirm で1文字ずつ送信する。1文字目（VK0）で GJI 候補ウィンドウが
SHOW し `CompositionConfirmed` になった後、2文字目（VK1、特に子音単体で
モーラ未完成の VK）が `SuspectedLiteral` と誤検知される。

**原因:** `LiteralDetector::check_now`（`crates/awase-windows/src/tsf/probe.rs:639-696`）
は Chrome 用に `write_confirmed || show_confirmed` の OR で判定するが、両シグナル
ともに VK1 以降で構造的に機能しない:

1. **SHOW はエッジトリガ**（`crates/awase-windows/src/tsf/win_event_obs.rs:154-156`
   の `EVENT_OBJECT_SHOW` ハンドラが `gji_candidate_show.notify()` を呼ぶのは
   hidden→visible 遷移の瞬間のみ）。VK0 で候補ウィンドウが開いたまま VK1 を送っても
   「開いたまま」なので新規 SHOW は発火しない。各 VK 送信直前に
   `LiteralDetector::new_gji_resumed_with_pre_send_baseline`
   （`crates/awase-windows/src/output/probe_io.rs:600-607`）で新規構築される
   detector の baseline は VK0 の SHOW 増分後の値になるため、VK1 の
   `show_confirmed` は原理的に `true` になり得ない。この限界は
   `probe.rs:676-680` に既知の限界として既に記載されていた。
2. **WriteTransferCount 閾値（350B、`probe.rs:632`）は子音単体 VK では
   原理的に閾値到達しない**。モーラが未完成（例: 「ta」の「t」単体、
   「ltu」の「t」単体）だと GJI 内部の変換候補探索自体が走らず、閾値算出の
   キャリブレーション根拠（`probe.rs:613-619`、完結した1文字の warm 変換で
   実測 ~400B）が前提とする書き込み量が発生しない。この限界も
   `probe.rs:658-667` に既知の限界として既に記載されていた。

**検討したが採用しなかった案:** 「候補ウィンドウが表示中なら `VK_ESCAPE` を送って
強制的に HIDE させ、次の VK で SHOW を人工的に再発火させる」という案を検討したが、
`docs/windows-api-constraints.md` §1-2（2026-05-24 実機確認済み）に
「VK_ESCAPE は composition をキャンセルして入力テキストが消えるため使用禁止」と
明記されており、既存の `escape_composition`（`tsf/warmup/literal_detect_fsm.rs`）
機構もこの破壊的性質に依存して設計されている（ESC 送信は必ず後続の backspace
クリーンアップとセットで、「確定済み composition を丸ごと破棄する」用途専用）。
採用すると VK0 で確定した文字ごと消してしまう危険があるため却下した。

**修正 (2026-07-17):** 候補ウィンドウが「既に表示されている」こと自体を
「warm な composition が継続している」直接証拠とみなし、その場合は
literal-detect の待機・polling を丸ごとスキップして即 `CompositionConfirmed`
とする（`crates/awase-windows/src/tsf/warmup/probe_fsm.rs` の Phase 2c per-VK
confirm ループ、新設した純粋関数 `should_skip_literal_wait(candidate_visible:
bool) -> bool` と、live 状態を返す既存の `crate::tsf::observer::
gji_candidate_visible_now()` を使う）。未表示のとき（cold の可能性がある）だけ
従来通り SHOW/WriteTransferCount の polling を行う。新しいイベント配線や
タイミング定数の追加は不要。

**残存リスク（意図的に許容する trade-off）:** 候補ウィンドウが「表示されたままだが
実際には対象の VK が literal 化した」という理論上のケース（TSF context が
composition 途中で部分的に壊れ、かつウィンドウが古い内容のまま残る）は本修正では
検出できない。ただし HIDE イベントで `gji_candidate_visible_now()` は正しく
`false` に戻るため、次の VK からは通常の polling に自動的にフォールバックする
（自己修復的）。実機ソークテストで実際に問題になるか観察する。

**未解決の follow-up:**
- TSF/WezTerm 側（`gji_coro_body` Phase 5b、`probe_fsm.rs:387-392` のコメントで
  「同じ発想」と言及されている箇所）に同型の検出漏れがあるか未確認。detector
  構築が異なる（`TransmitTarget::Tsf` は `gji_last_io_ms` ベース）ため、同じ
  修正がそのまま当てはまるかは別途確認が必要。
- `RawTsfLiteralRecovery` の「2回連続失敗で以後無期限に give-up」という設計自体
  （`probe_io.rs:825-856`）は、本 BUG の主要トリガーを塞いだことで発火頻度は
  大きく下がるはずだが、構造的な保護（cap・エスカレーション）は依然として
  存在しない。真の TSF 破損など別要因で再発する可能性は残るため、次回同種の
  報告があれば `probe_io.rs` の give-up 分岐自体の見直しを検討する。

**テスト:** `crates/awase-windows/src/tsf/warmup/probe_fsm.rs` の
`#[cfg(test)] mod tests` に `should_skip_literal_wait_when_candidate_already_visible`
/ `should_skip_literal_wait_false_when_candidate_hidden` を追加。純粋関数
`should_skip_literal_wait` の回帰テストであり、コルーチン本体（Win32/GJI 実 I/O
依存）は既存パターンと同様 Linux 上でのユニットテスト対象外
（`tsf` モジュール全体が `#[cfg(windows)]`）。`cargo check -p awase-windows
--target x86_64-pc-windows-gnu` で型チェック確認済み。このサンドボックスに wine
が無いため `cargo test --target x86_64-pc-windows-gnu` の実行（`.exe` 起動）は
できず、テスト実行そのものは Windows 実機/CI 待ち。

**関連ファイル:** `crates/awase-windows/src/tsf/warmup/probe_fsm.rs`（主修正・
テスト追加）、`crates/awase-windows/src/tsf/probe.rs`（参照のみ、無変更）、
`crates/awase-windows/src/tsf/observer.rs`（参照のみ、無変更）。

---

## BUG-30: `LiteralDetectCore::poll`（`run_per_vk_confirm` 以外の literal-detect 経路）が候補ウィンドウ可視でも SHOW イベント未発火だと backspace してしまう

**症状:** BUG-29 と同根の「候補ウィンドウの SHOW/HIDE（binary イベント）と GJI I/O
（連続量、モーラ完結時のみ意味を持つ）が別々のセンサーであるため、confirm 判定が
取りこぼす」という構造的な問題を、`run_per_vk_confirm`（BUG-29 で対処済み）
**以外**の literal-detect 呼び出し元でも確認した。ユーザー（開発者）の指摘:
「候補SHOW というのは SHOW イベントが起きたということをいっていますか？それとも
awase 内部の shadow で visible 状態であるということをいっていますか？その2つは
全然違います」。実際にコードを確認したところ、`gji_candidate_show`（イベント
カウンタ、`ChangeCounter`）と `gji_candidate_visible`（レベル状態、
`AtomicBool`）は別物であり、`LiteralDetector::check_now`
（`crates/awase-windows/src/tsf/probe.rs`）が実際に confirm を出す条件は
常に前者（イベントカウンタ）で、後者（いま可視かどうかのライブ状態）は
どの confirm ロジックを使うかの分岐にしか使われていなかった。

**IME:** GJI（Google 日本語入力）。

**該当する呼び出し元（`LiteralDetectCore::poll` を経由する2経路。
`run_per_vk_confirm` は含まない、別経路）:**

1. `crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs::gji_coro_body`
   Phase 6（Inline LiteralDetect）— TSF mode（WezTerm 等）で `needs_literal`
   かつ `should_prepend_f2`／`used_eager_path` のため Phase 5b（per-VK confirm）
   をバイパスするケース。
2. `crates/awase-windows/src/output/vk_send.rs`（`LiteralDetectFsm::new` 呼び出し
   箇所）— Chrome/Vk mode で `tsf_gate.state()==Probing` かつ長期 idle でない
   場合の「warm パス」post-transmit composition 確認。

**未対象（既知の限界として明記）:** `crates/awase-windows/src/tsf/warmup/
probe_fsm.rs::run_per_vk_confirm` は `LiteralDetectCore::poll` を経由せず、
`sent.detector.check_now(sent.deadline_ms)` を直接ループで呼ぶ独自実装。
BUG-29 の修正（`should_skip_literal_wait`）は `target == TransmitTarget::Chrome`
のみに適用され、TSF ターゲット（WezTerm 等の per-VK confirm、
`gji_coro_body` Phase 5b から呼ばれる経路）には適用されていない
（`probe_fsm.rs:341-343` のコメント「TSF 側はこの早期脱出を経験的に必要と
していない（従来から常時 polling）ため据え置く」）。本 BUG-30 の修正は
この TSF per-VK confirm 経路には効かない。同型の検出漏れが TSF per-VK でも
発生しうるかは未確認（BUG-29 の「未解決の follow-up」と同じ懸念）。

> **【解消済み】追補1 参照**: この「未対象」は下記追補1（`LiteralDetector` の
> TSF/Chrome 検出ロジック統一）で解消した。`should_skip_literal_wait` の
> Chrome 限定ゲートを撤去したことで、TSF per-VK confirm も早期脱出の対象になった。

**設計方針（Opus によるセカンドオピニオン相談を経て決定）:**

1. **veto の場所**: `LiteralDetector::check_now` 自体は confirm/timeout 判定
   専任のまま変更しない。`check_now` に live level read（`gji_candidate_visible_now()`）
   を混ぜると、confirm 判定に第4のシグナル（しかも confirm 用ベースラインと
   非対称なタイミングで読むライブ値）が紛れ込み、本 BUG の発端になった
   「別のセンサーを同じ問いに答えさせる」混同を再発させる。veto は
   `LiteralDetectCore::poll`（`SuspectedLiteral` を受けて回収アクションを
   生成するかどうかの判断）側の責務として実装した。
2. **veto した場合の挙動**: `DetectionResult` に第三の variant は追加しない。
   `poll` が `None` を返せば次 tick も `SuspectedLiteral` が再評価されるため、
   自然に「可視の間は hold し、confirm するか HIDE した瞬間に決着する」動作に
   なる。ただし候補ウィンドウが固着した異常系でタイマーが永久に止まらないよう、
   `GJI_CANDIDATE_VETO_CAP_MS`（`tuning.rs`）で上限を設けた。上限超過時も
   backspace はしない（候補が可視である以上ほぼ確実に compose 成功しており、
   消すと BUG-27 追補5 と同型の regression になるため）— 無回収の `Done` で
   打ち切る。
3. **per-VK パスでは veto を無効化**: `LiteralDetector::veto_eligible()`
   （`write_bytes_baseline.is_none()`）が false のとき（Chrome per-VK confirm 用に
   `new_gji_resumed_with_pre_send_baseline` で構築された detector）は veto を
   適用しない。前の VK が開いた候補ウィンドウが可視のまま残っている状態で
   今回の VK が真にリテラル化するケース（前モーラ由来の誤 veto）を避けるため。
   （※ この分岐は理論上の保険であり、現行コードでは per-VK パスは
   `run_per_vk_confirm` 経由で `LiteralDetectCore::poll` 自体を通らないため
   実際には到達しない。将来 per-VK パスが `LiteralDetectCore` に統合された
   場合の安全装置として残す。）

**修正:**
- `crates/awase-windows/src/tsf/probe.rs`: `LiteralDetector::veto_eligible()`
  を追加（`write_bytes_baseline.is_none()` を返す）。
- `crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs`:
  `LiteralDetectCore` に `veto_started_at_ms: Option<u64>` を追加し、
  `poll()` の `SuspectedLiteral` アームで `veto_decision()`
  （`VetoDecision::{Hold, Expired, NotApplicable}`）を判定してから
  回収するように変更。
- `crates/awase-windows/src/tuning.rs`: `GJI_CANDIDATE_VETO_CAP_MS = 300`
  を追加。

**実測未了（`tuning-constants.md` 要求未達）:** `GJI_CANDIDATE_VETO_CAP_MS`
は実機計測なしの暫定値（`CHROME_GJI_REINIT_CONFIRM_MS` 等、同程度の
「確認待ち」定数からの類推）。「候補ウィンドウ可視 → I/O/SHOW 確定」までの
実測遅延データが無いため、Windows 実機（Chrome/Teams/WezTerm 等）で計測して
から本番投入すること。実測が済むまでは diag フラグ等で無効化した状態で
マージするか、実測を別セッションで行うか要判断。

**テスト:** `crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs` の
`#[cfg(test)] mod tests` に `poll_vetoes_backspace_while_candidate_visible`
（可視時に hold すること）、`poll_gives_up_without_backspace_after_veto_cap_expires`
（上限超過後も backspace しないこと）、`poll_does_not_veto_on_per_vk_confirm_path`
（per-VK パスでは veto が効かないこと）を追加。`cargo check`/`cargo clippy
--target x86_64-pc-windows-gnu`（lib、`-D warnings` 込み）通過、`cargo test
--target x86_64-pc-windows-gnu --no-run` でテストバイナリのコンパイル・
リンクまで確認済み。wine 未導入のためこのサンドボックスでは `.exe` 実行は
できず、実機再検証は未実施。

**関連ファイル:** `crates/awase-windows/src/tsf/probe.rs`
（`LiteralDetector::veto_eligible`）、`crates/awase-windows/src/tsf/warmup/
literal_detect_fsm.rs`（`LiteralDetectCore::veto_decision`、テスト追加）、
`crates/awase-windows/src/tuning.rs`（`GJI_CANDIDATE_VETO_CAP_MS`）。

**追補1（2026-07-19、`LiteralDetector` の TSF/Chrome 検出ロジックを統一）:**
ユーザー（開発者）の指摘: 「TSF の gji io 閾値無しがおかしいと思います。Chrome の
バイト量の閾値にする、方向で統一してください」。

本編で書いた `LiteralDetector::check_now` は target ごとに別ロジックだった:

- TSF（`write_bytes_baseline=None`）: `gji_last_io_ms` の**変化の有無**を
  閾値なしで判定。
- Chrome（`write_bytes_baseline=Some`）: `gji_write_bytes()` の増分が
  [`COMPOSITION_BYTES_THRESHOLD`]（350B）を超えたか **または** SHOW イベント、
  の OR で判定。

`COMPOSITION_BYTES_THRESHOLD` の根拠コメント（実機5サンプル）を読み直すと、
**cold Chrome（未 compose のリテラル 'a'）でも WriteTransferCount が +300B
ほど動く**ことが実測されている。つまり Chrome では「I/O が変化したか」だけの
binary 判定では literal と compose を区別できないため閾値が必要だった。
TSF 側が閾値なしで安全だという前提は、この Chrome の実測に相当する検証が
TSF-native composition（WezTerm 等）に対して一度も行われていない、単なる
「経験的に問題が出ていない」(BUG-29 のコメント参照)という消極的根拠に
過ぎなかった。閾値なし判定は「literal でも何らかの非ゼロ I/O が出るなら
false confirm する」方向に倒れるリスクがある一方、閾値ありは「confirm が
多少遅れる」方向にしか倒れないため、実測なしでも閾値ありに統一する方が
安全側と判断した。

**修正:** `LiteralDetector` から `write_bytes_baseline: Option<u64>` という
target 分岐の型を撤去し、`write_bytes_baseline: u64`（必須）+
`veto_eligible: bool`（構築時に呼び出し元が明示）に変更。`check_now` は
target に関わらず常に `write_confirmed || show_confirmed` の単一ロジックに
なった。`new_gji_resumed()`/`new_gji_resumed_with_pre_send_baseline()` を
`new(veto_eligible)`/`new_with_pre_send_baseline(bytes, veto_eligible)` に統合。

呼び出し元(`probe_io.rs` の `Transmit`/`TransmitSingleVk` ハンドラ)から
target ごとの detector 構築分岐を撤去。veto_eligible は「単語単位のバッチ
確認なら true、per-VK 単体確認（前モーラ由来の誤 veto の恐れ）なら false」
という意味に付け替えた（旧: `write_bytes_baseline` が `Some`/`None` かで
暗黙的に決まっていた）。

**副次効果: TSF per-VK confirm が BUG-29 の恩恵を初めて受ける。**
`probe_fsm.rs::run_per_vk_confirm` の `should_skip_literal_wait`（候補
ウィンドウ可視なら literal-detect polling をスキップする早期脱出）は
これまで `target == TransmitTarget::Chrome` に限定されていた
（「TSF 側は経験的に必要としていない」という未検証の理由）。検出ロジック
自体が統一された以上この分岐を維持する理由もないため、Chrome 限定ゲートを
撤去し両ターゲットに適用した。これにより本 BUG-30 本編が指摘していた
「TSF per-VK confirm には候補ウィンドウ可視性による保護が一切ない」という
ギャップが、per-VK 経由でも解消される。

**未検証のリスク（据え置き）:** `COMPOSITION_BYTES_THRESHOLD`（350B）は
Chrome の SendInput 経路の実測値であり、TSF-native composition で同じ桁の
I/O が出るかは依然未検証。実機で TSF 側の I/O 量が Chrome と大きく異なる
（閾値に届きにくい／届きすぎる）ことが分かった場合は、`TSF_COMPOSITION_
BYTES_THRESHOLD` のような target 別定数に分離すること。

**テスト:** `probe.rs` の `check_now_confirms_via_candidate_show_when_
write_bytes_below_threshold`/`check_now_still_detects_suspected_literal_
when_neither_signal_fires` を新シグネチャに更新。`literal_detect_fsm.rs` の
veto テスト3件、`chrome_probe.rs` のテストも新シグネチャに追従。
`cargo check`/`cargo clippy --lib --target x86_64-pc-windows-gnu -- -D
warnings` 通過、`cargo test --target x86_64-pc-windows-gnu --no-run` で
lib・全 `tests/*.rs`（architecture_guard 含む）のコンパイル・リンクまで
確認済み。wine 未導入のため実行・実機再検証は未実施。

**副次清掃:** `output/vk_send.rs::send_romaji_as_tsf_warm` で `LiteralDetector`
の呼び出しを更新する過程で、構築した `detector` が両分岐（`let _ = (detector,);`
／`let _ = (detector, ze_bs_count);`）で無条件に破棄され、一度も使われていない
死んだ変数だったことに気づいた（本リファクタ以前から存在。
`LiteralDetectFsm::new` が内部で自前の detector を生成するため、この呼び出し元
での構築は元々不要だった）。`LiteralDetector::new` は純粋な atomic 読み取りのみで
副作用が無いことを確認した上で、変数ごと削除した。

**関連ファイル:** `crates/awase-windows/src/tsf/probe.rs`
（`LiteralDetector` 本体・テスト）、`crates/awase-windows/src/output/
probe_io.rs`（`Transmit`/`TransmitSingleVk` ハンドラ）、
`crates/awase-windows/src/tsf/warmup/probe_fsm.rs`
（`should_skip_literal_wait` 呼び出しゲート撤去）、
`crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs`・
`crates/awase-windows/src/output/vk_send.rs`（呼び出し更新 + 死んだ
`detector` 変数の削除）・
`crates/awase-windows/src/tsf/warmup/chrome_probe.rs`（呼び出し更新）、
`crates/awase-windows/src/tsf/observer.rs`（doc 更新のみ）。

---

## BUG-31: `NativeF2Down`（非 TSF）が warm 中でも無条件に cold-mark し、連続 typing の1文字を無用な per-VK confirm レースに晒す

**症状:** Microsoft Teams（`TeamsWebView`、Chrome 系、Vk mode）で、Google 日本語入力
（GJI）を使って通常の連続タイピング中（メッセージ送信直後、1.8秒の自然な間を挟んで
「５せっしょん」と入力）に、`せ` とそれに続く `っし`/`ょ` の一部が無音で消失した
（backspace のみ発生し romaji 再送が起きなかった）。ユーザー実機報告（2026-07-19）:
「Teams で、入力が一部おかしくなりました」「５セッション のような数字が入っている
ところが期待と違う入力になりました」。

**再現手順（実機ログで確認、`experiment/skip-cold-probe-wait` ブランチ）:**
Enter で確定した直後、`self_injected=false, injected=false` の**物理** VK 0xF0↑/0xF2↓
（`IME-mode` タグ、`source=PhysicalImeKey`）イベントが届き、`composition_native_f2_down`
経由で `CompositionFsm::NativeF2Down { tsf_mode: false }` に渡る。この時点で GjiFsm
は `OnWarm`（直前に `LongIdle timer set duration=5000ms` で 5 秒間 warm 維持のはず
だった）にもかかわらず、`NativeF2Down` は無条件に `MarkCold(F2NonTsf)` +
`GjiCompositionReset` を発行し、warm 維持用の LongIdle タイマーを kill する
（`Timer killed: logical=108`）。

その約1.8秒後（何のフォーカス変更も long-idle もない、ごく普通の継続タイピング）に
`せ` を送信する段になっても、composition は上記の stale な cold mark を引きずった
まま `warm=false` と判定され、`idle_at_cold=235ms`（1.8秒前のスナップショット値が
そのまま）で per-VK confirm のcold-startパス（`experiment/skip-cold-probe-wait` で
事前 F2/probe 待機を撤去した後の唯一の安全網）に送り込まれる。ここで無関係な過去の
composition が残していた候補ウィンドウの HIDE が数ms差でこのper-VK confirmチェック
と衝突し（BUG-29/BUG-30 の「残存リスク」節が予告していた再発パターン）、
`suspected literal` の誤検知 → `probe_io.rs` の
「`consecutive raw-tsf-literal (count>=2)` は再送せず backspace のみで諦める」分岐
（BUG-27 参照）に落ちて、`せ` と後続文字の一部が実際に失われた。

**原因（本質）:** `NativeF2Down(tsf_mode=false)` には
`ConfirmKeyDown`（`.claude/rules` 準拠、2026-07-11 修正済み: warm な確定キーは
cold 化しない）と同種の「warm を無条件に cold 化してはいけない」ガードが欠けていた
まま、`a3425bf` 以来放置されていた（`ConfirmKeyDown` 修正時のコメントにある
「warm な GJI/TSF を確定キーだけで cold 化する理由は tsf_mode に関係なく無い」が、
`NativeF2Down` には未適用だった）。過去に `F2NonTsf` cold-mark が実際に必要だった
事例（`3c275a7`/`79134f5`/`b5946bb`）はいずれも GJI long-idle（GjiFsm が既に
`OnCold(Long/Medium)`）由来であり、warm 中に F2 系イベントが来て何かを温め直す
必要があった実測事例は無い。

物理 VK 0xF0↑/0xF2↓ 自体の発火元（`self_injected=false` だが `injected=false` ＝
awase 自身の注入マーカーも LLKHF_INJECTED も無い）は未特定。BUG-14 で記録済みの
外部注入 VK_DBE_HIRAGANA down+up シグネチャに似るが、そちらは `injected=true`
であり一致しない。win32k の JIS NLS キー変換などカーネル/ドライバ側由来の可能性が
高い（同環境で 2026-07-06 にも類似の単独 0xF0 KeyUp を記録済み、`hook.rs:436-439`
参照）。次回発生時は `[hook] IME-mode vk=0xXX dir ... scan=0xXX extra=0xXX` の
`scan`/`extra` 値で切り分けること。

**修正 (2026-07-19):** `CompositionEvent::NativeF2Down` に `warm: bool` を追加し
（`composition_native_f2_down` から `self.output.is_composition_warm()` を渡す）、
`tsf_mode=false` 分岐で `warm` なら `Response::consume()`（no-op、LongIdle タイマー
はそのまま生存）に変更した。`tsf_mode=true`（`NativeF2Consumed`）分岐は変更なし
（Medium/Long cold 維持のための別の設計意図があり、本件のトリガーではない）。

**実機ソーク結果 (2026-07-19、修正コミット `810f33d` 反映後):** ユーザーが Teams
(TeamsWebView) で通常使用を継続し、cold-start 関連の入力不具合は再発していない。
加えて、意図的に「warm 中に物理 F2 キーを押下してすぐ打鍵」を試行し、実機ログで
`reason=F2NonTsf` が発火した3件（`idle=2734ms`・`idle=5218ms`・`idle=78ms`）を
それぞれ直前ログまで遡って検証した結果、**全件とも composition が実際に non-warm
だった**ことを確認した（前2件は直前の本物のフォーカス変更、`idle=78ms`
のケースは直前の Ctrl+無変換 IME OFF から F2 キーでの IME 再 ON、いずれも GjiFsm
が正当に `OnCold` に落ちていた場面）。つまり本修正の `warm` ゲートは、warm を
不必要に cold 化する経路のみを抑止し、genuinely cold な場面では従来通り
`F2NonTsf` cold-mark を発行し続けている（over-suppression していない）ことが
確認できた。3件とも後続の romaji 送信は per-VK confirm を通過して literal
誤検知なく確定しており、文字消失は発生していない。「warm 中に物理 F2 を押下して
literal 化するか」という本来のシナリオ自体はこのソークでは一度も再現しなかった
（＝待避された F2NonTsf cold-mark が発生する場面に遭遇していない）ため、
その意味での実測はまだ0件。継続してソークし、該当シナリオに遭遇した場合は
追補として記録すること。

**検討したが今回は見送った案:** `probe_io.rs` の give-up 分岐（consecutive失敗で
romaji 再送せず backspace のみ）自体の緩和。BUG-27 追補2で「常に再送」は msedge で
無限 backspace ループを起こし撤回済みのため、今回のトリガー（stale cold mark）を
塞ぐ本修正を優先し、give-up 分岐自体は次回同種再発時に改めて検討する
（BUG-29「残存リスク」節を参照）。同様に、外部注入（`injected=true`）F2 の
ユーザー意図昇格を抑制する BUG-14 型ガードを `composition_native_f2_down` 呼び出し
前に追加する案も検討したが、本件の実機ログでは `injected=false` であり本件の
直接原因ではないため、今回のスコープからは除外した。

**テスト:** `crates/awase-windows/src/tsf/composition_fsm.rs` に
`native_f2_non_tsf_while_warm_is_noop` を追加（warm 中の `NativeF2Down` が
`MarkCold`/`GjiCompositionReset` を一切発行しないことを検証）。既存の
`native_f2_in_tsf_consumes_and_warms` / `native_f2_non_tsf_marks_cold_without_consume`
は `warm` フィールド追加に伴い更新（挙動は不変）。`cargo check -p awase-windows
--target x86_64-pc-windows-gnu` 通過確認済み。wine 未導入のためこのサンドボックス
では `.exe` 実行はできず、実機再検証は未実施。

**関連ファイル:** `crates/awase-windows/src/tsf/composition_fsm.rs`
（`NativeF2Down` 主修正・テスト）、`crates/awase-windows/src/platform.rs`
（`composition_native_f2_down` で `warm` を渡す）。

---

## BUG-32: `send_vk_dbe_hiragana_pair` が Win キー押下中のスキップを送信成功と
区別せず返し、GJI に IME-ON 信号が一度も届かないまま belief だけ ON 確定する

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS`、TSF native、GJI）で
物理 F2（`VK_DBE_HIRAGANA`）キーで IME を OFF→ON にした直後、以降の入力が全て
`[raw-tsf-literal] cold=N per-VK[0/1] suspected literal` → backspace リカバリを
繰り返し、2回連続失敗で `giving up`（romaji 再送なし）に落ちて文字が消える。
ユーザー実機報告（2026-07-20）:「入力しても文字がチラついててちゃんと入力できない」
「IME ON だと shadow はなっているけど、実際は IME OFF なんだと思う」。ログで
実際にその通りであることを確認した。

**IME:** GJI（Google 日本語入力）。TsfNative（Windows Terminal・WezTerm 等）。

**再現手順（実機ログで確認）:**
1. 物理 F2（`VK_DBE_HIRAGANA`）KeyDown で shadow が OFF→ON にトグルする
   （`Shadow IME toggle: OFF → ON (vk=0xF2, source=PhysicalImeKey)`）。
2. `f2_warmup_owned=true`（GJI 戦略）のため `PhysicalKeyDisposition::plan` が
   この物理キーを **Suppress**（OS/GJI に配送しない）と判断する
   （`[tsf-f2] key suppress vk=0xf2 KeyDown (physical disposition)`）。
   この設計は「物理キーの代わりに awase 自身が代替の F2 warmup を送る」ことが
   前提（`key_pipeline.rs` のコメント参照、BUG-10 の教訓）。
3. `CompositionFsm::NativeF2Down` → `EmitWarmup` → `Output::send_eager_tsf_warmup`
   → `tsf::send::send_vk_dbe_hiragana_pair` が代替送信を試みるが、この瞬間
   **Win キー押下中**だったため実際には `SendInput` を呼ばずスキップする
   （`[tsf-warmup] skipped VK_DBE_HIRAGANA (Win key held)`）。
4. しかし `send_vk_dbe_hiragana_pair`（旧実装）は送信した場合とスキップした場合の
   **どちらも同じ `current_tick_ms()` を返す**ため、呼び出し元
   `send_eager_tsf_warmup` は区別できず「VK_DBE_HIRAGANA 送信」とログを出し、
   `eager_warmup_sent_ms` を送信済み扱いで更新する
   （`[tsf-eager-warmup] VK_DBE_HIRAGANA 送信, eager_warmup_sent_ms=...ms`）。
5. 一方、`ime_controller::GjiDirectStrategy::apply` は `shadow_on == true` を見て
   「物理キー側で ON 済みのはず」と判断し `VK_IME_ON` の送信自体をスキップする
   （`[apply-ime] GJI direct: shadow ON, skip VK_IME_ON` →
   `outcome=AlreadyMatched`）。
6. 結果: 物理 F2（Suppress）・代替 F2（Win 押下でスキップ）・`VK_IME_ON`
   （shadow ON によりスキップ）の**3経路すべてが実際には GJI に何も送っておらず**、
   それにもかかわらず belief は `effective=true confident=true` で確定する。
7. 2026-07-18 の cold-start 簡素化（BUG-24 追補参照）で「送信前に GJI 準備を待つ」
   予防的待機は撤去済みで、literal 化の検出・回収は完全に per-VK confirm
   （送信後のリカバリ）に一本化されている。per-VK confirm は romaji を再送する
   だけで **IME-ON トグル自体を再試行しない**ため、GJI が実際には OFF のままだと
   何度再送しても必ず literal 判定になり、2回連続失敗で `probe_io.rs` の
   give-up 分岐（backspace のみ、再送なし）に落ちて文字が失われる。

**原因（本質）:** `send_vk_dbe_hiragana_pair` の「Win キー押下中スキップ」が
戻り値レベルで「送信成功」と区別不能だった。これは `crate::ime::send_ime_mode_key`
が BUG-16 追補（2026-07-07）で修正した欠陥
（「スキップを `Applied` 扱いにすると `applied_snapshot` がラッチされ再試行が
全て no-op 化する」）と**全く同型**で、`send_ime_mode_key` 側だけが修正され、
本関数（composition の eager warmup 専用パス）には同種の修正が入っていなかった。

**修正 (2026-07-20):** `send_vk_dbe_hiragana_pair` の戻り値を `u64` から
`Option<u64>`（`#[must_use]`）に変更。実際に `SendInput` した場合のみ
`Some(送信時刻ms)`、Win キー押下中でスキップした場合は `None` を返す。
呼び出し元 `Output::send_eager_tsf_warmup` は `None` のとき
`eager_warmup_sent_ms` を更新せず、「送信した」ログも出さないよう変更した。

**未解決の残課題（今回のスコープ外）:** 本修正は「スキップを送信成功と偽らない」
ことのみを直しており、GJI に実際に IME-ON 信号を届ける再試行機構までは実装して
いない。2026-07-18 の設計方針（pre-send 待機を撤去し per-VK confirm に一本化）は
「literal 化した romaji の再送」は回収するが「IME トグル自体の再送」は回収しない
という非対称性が残っている。次に同種の報告（Win キー押下中に物理 IME キーで
ON にした直後の入力不能）があれば、per-VK confirm の give-up 分岐から
`send_eager_tsf_warmup` を再試行する経路の追加を検討すること。

**テスト:** `send_vk_dbe_hiragana_pair` は `crate::hook::is_physical_key_down` /
`crate::win32::send_input_safe`（実 Win32 API）に依存するため、既存の warmup
パス同様 Linux 上でのユニットテストは非現実的（`tsf` モジュール全体が
`#[cfg(windows)]`）。`cargo check`/`cargo clippy -p awase-windows --lib --target
x86_64-pc-windows-gnu -- -D warnings` 通過、`cargo test -p awase-windows --target
x86_64-pc-windows-gnu --no-run` で lib・全 `tests/*.rs`（architecture_guard 含む）の
コンパイル・リンクまで確認済み。wine 未導入のためこのサンドボックスでは `.exe`
実行はできず、実機再検証は未実施。本記録で代替する。

**関連ファイル:** `crates/awase-windows/src/tsf/send.rs`
（`send_vk_dbe_hiragana_pair` 主修正）、`crates/awase-windows/src/output/mod.rs`
（`send_eager_tsf_warmup` 呼び出し元更新）。関連バグ: BUG-16 追補（同型の欠陥、
`send_ime_mode_key` 側の修正）、BUG-10（f2_warmup_owned=false 側の食い逃げ）、
BUG-24 追補（per-VK confirm への一本化）。

---

## BUG-33: per-VK confirm が世代をまたいだ stale な confirm 根拠を現世代の証拠として
誤って採用し、見捨てた世代の backspace が別スコープの確定済み文字を消す（ADR-079、Stage 1: 検出のみ実装）

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS`、GJI、TSF-native）で
高速に連続入力すると、Ctrl+無変換で IME OFF → `4`→`1`（半角数字、直接パススルー）
→ 物理サムキーで IME 再 ON → 「ふん」と続けて入力したところ、**「41分」と入力
したはずが「4分」になった**（`1` が消失）。消えたのは疑わしいと判定された文字
ではなく、直前の別スコープ（IME OFF 中）で既に確定済みの実文字だった。
ユーザー実機報告・詳細な時系列診断は
[ADR-079](adr/079-epoch-fenced-literal-recovery-with-replay.md) のコンテキスト
節を参照（2026-07-22）。

**IME:** Google 日本語入力（GJI）。Windows Terminal（TSF-native、per-VK confirm
経路）。

**再現手順（実機ログで確認、ADR-079 参照）:**
1. romaji "fu" を per-VK confirm で1文字ずつ送信（cold=263）。VK0（F）は
   candidate SHOW を根拠に confirmed、VK1（U）は 300ms deadline を約41ms
   超過して `SuspectedLiteral` と誤判定（実際には合成は成功していた、false
   positive）。
2. per-VK confirm の recovery（`per_vk_recovery_params(idx=1)`）が
   `backs=1, escape_composition=true` を返し、`VK_ESCAPE`（本物の pending
   composition を破棄）+ `VK_BACK`×1 を送信。composition 側に破棄すべき
   literal は存在しなかったため、`VK_BACK` は代わりに手前の唯一の確定済み
   文字 `1` を消してしまう。
3. romaji "fu" が cold=264 として再送される。
4. 再送後、候補ウィンドウ SHOW イベントが発火し VK0（F）を confirmed 判定するが、
   `last_gji_write` を逆算すると実際の GJI I/O は cold=263（見捨てた世代）の
   ものであり、cold=264 自身の送信より前に起きていた。つまり **前世代の残存
   証拠を現世代の confirm として誤って使い回していた**。

**原因（本質）:** `LiteralDetector::check_now`（`tsf/probe.rs`）および
`await_vk_detection` の「候補ウィンドウ既に可視」ショートカット
（`tsf/warmup/probe_fsm.rs`、BUG-29 由来）は、confirm の根拠（candidate SHOW /
write-bytes 増加）が「どの送信世代に由来するか」を一切区別していなかった。
候補ウィンドウの SHOW/HIDE や write-bytes 増加は、対応する GJI I/O が現在の
送信より後に起きたことを保証しない非同期シグナルであり、Chandra-Toueg の
unreliable failure detector と同型の曖昧さを持つ（詳細は ADR-079「理論的背景」
節）。

**修正 (2026-07-22, Stage 1 — 検出のみ):** epoch fencing を導入した。
- `LiteralDetector` に `epoch_send_ms`（構築時 = VK/バッチ送信時刻）を追加し、
  `DetectionResult` に `StaleConfirm` を新設。`check_now` は confirm 根拠
  （write-bytes 閾値超過 / candidate SHOW）が実際に `gji_last_write_ms()`
  （既存の GJI I/O 最終書き込み時刻）で `epoch_send_ms` 以降に裏付けられて
  いるかを確認する。
- write-bytes 由来の confirm は `gji_last_write_ms` の更新と同一ポーリング
  サンプルで自己整合するため即時判定。candidate SHOW 由来の confirm は
  `EVENT_OBJECT_SHOW` が write-bytes ポーリング（`GJI_SAMPLE_INTERVAL_MS`=10ms）
  より早く届きうる benign なレースがあるため、即断せず最大2ポーリング分
  （`LiteralDetector::EPOCH_FENCE_GRACE_MS`=20ms）だけ `gji_last_write_ms` が
  追いつくのを待ってから再判定する。
- `gji_last_write_ms() == 0`（GJI I/O monitor 未アタッチ等で一度も観測して
  いない）の場合は fencing 自体を無効化し従来通りの confirm 判定に
  フォールバックする（false-negative の温床にしないため）。
- `await_vk_detection` の「候補ウィンドウ既に可視」ショートカット（BUG-29）
  にも epoch 比較（`last_write_ms >= epoch_send_ms`）自体は適用した。まさに
  このショートカットが実機トレースで誤発火した箇所（前世代の合成が残した
  ままの可視状態を、現世代の VK0 送信の confirm として即座に採用していた）。
  ただし `check_now` の SHOW-only 分岐が持つ `EPOCH_FENCE_GRACE_MS` の猶予は
  このショートカットには**移植されておらず**、一発判定のままだった
  （2026-07-23 実機で発覚した regression。追補2参照）。

**本コミットのスコープ（意図的な限定）:** `StaleConfirm` を検出しても
ESC/retype/replay は一切行わず、warn ログ（`[epoch-fence] ...` /
`ime_diagnostic::log_composition_probe(cold_seq, "epoch-fence-stale")`）を
残すのみで現状維持する（**ADR-079 の Stage 1**）。これは実装計画の設計レビュー
（Opus によるセカンドオピニオン）で、当初想定していた「quarantine → ESC →
retype → replay」機構に2件の設計欠陥（(1) リングバッファに「送信した順」で
記録すると pre-edit の未確定合成文字を retype 対象に取り違える、(2)
candidate SHOW 由来の fencing は benign なポーリングレースと本物の stale を
瞬間的に区別できず正常な合成を破壊しかねない）が見つかったため、まず
検出・ログのみを実機にデプロイして `StaleConfirm` の実際の発火頻度・状況を
観測し、信号の質を検証してから Stage 2（quarantine/ESC/retype/replay の実装）
に進む方針とした。

**未解決の follow-up（Stage 2、未実装）:**
- 本修正は「stale confirm を検出してログに残す」までであり、`1` が消える
  実害自体は直っていない（backspace は fencing が検出するより前に既に
  実行されているため）。Stage 2 で、backspace 実行時に「直近の確定済み
  （committed）出力」を quarantine し、`StaleConfirm` 検出時に ESC + retype +
  （変換トリガー系キーが絡まなければ）後続入力の replay を行う機構を追加する
  予定。
- Stage 2 実装には、`Output::send_keys`（決定済み Char/Romaji 出力）と
  `RawKeyEventExt::reinject`（IME OFF 時の直接パススルー、`lib.rs`）という
  2つの独立経路を横断する「直近確定出力履歴」のリングバッファが新規に必要
  （現状はどちらの経路も履歴を残していない）。

**テスト:** `tsf/probe.rs::tests`（`#[cfg(windows)]`）に fencing の5パターン
（fresh write 即時confirm/ stale write 即時stale/ show 猶予後confirm/ show
猶予後stale/ last_write_ms 未観測時のフォールバック）を追加。
`tsf/warmup/probe_fsm.rs::tests`（`#[cfg(windows)]` 無し、Linux でも実行可）に
`await_vk_detection` の「既に可視」ショートカットの fencing 分岐テストを追加。
`tsf/warmup/literal_detect_fsm.rs::tests` に `LiteralDetectCore::poll` の
`StaleConfirm` 分岐テストを追加。`cargo check`/`cargo clippy -p awase-windows
--target x86_64-pc-windows-gnu` で型チェック済み（このサンドボックスに wine が
無いため `#[cfg(windows)]` テストの実行そのものは Windows 実機/CI 待ち）。

**関連ファイル:** `crates/awase-windows/src/tsf/probe.rs`（`LiteralDetector`
主修正）、`crates/awase-windows/src/tsf/warmup/probe_fsm.rs`
（`await_vk_detection`/`run_per_vk_confirm`）、
`crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs`
（`LiteralDetectCore::poll`）。関連: BUG-29/BUG-30（per-VK confirm の
suspected-literal 誤判定の既知の限界）、
[ADR-079](adr/079-epoch-fenced-literal-recovery-with-replay.md)。

**追補（2026-07-22 実機）: 「検出のみ・recovery なし」が未送信 VK の欠落を招く
regression を引き起こしたため、SuspectedLiteral と同じ回収に倒した。**

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS`）から Chrome/msedge
（`Chrome_WidgetWin_1`、Imm32Unavailable、GJI）へフォーカス変更した直後、最初の
1文字「こ」（romaji "ko"）が「k」だけ残って「れ」以降と連結し、「これでできる」が
「kれでできる」になった。

**原因:** per-VK confirm の VK0（'K'）送信直後、BUG-29 由来の「候補ウィンドウ
既に可視」ショートカットが発火し、fencing が `gji_last_write_ms`
（epoch より約1.3秒前）を根拠に正しく `StaleConfirm` と判定した。ここまでは
意図通り。しかし本 Stage 1 の当初実装は `StaleConfirm` を「検出のみ・recovery
なし（ただの `Done`）」として扱っており、per-VK confirm ループがこの時点で
即座に終了してしまっていた。この「既に可視」ショートカットは per-VK confirm
の**1文字目**でも発火しうる（ADR-079 本体が想定していた「同一タイピング中に
一度 backspace した後の世代」ではなく、**フォーカス変更直前からの残留 GJI UI
状態**が原因）ため、まだ VK1（'O'）を一度も送信していない段階で処理が終了し、
既に送信済みの VK0 の生文字「k」だけが取り残された。

**修正:** `StaleConfirm` を「信用できない confirm」として扱い、既存の
`SuspectedLiteral` と全く同じ回収アクション（`per_vk_recovery_params`/
`emit_recovery_actions` によるバックスペース + romaji 再送、あるいは
`LiteralDetectCore::poll`/Chrome inline LiteralDetect の同型パス）を発行する
よう変更した。「信用できないから何もしない」ではなく「信用できないから
今まで通りの安全な回収パスに倒す」方が正しいと判断した。ログタグ
（`epoch-fence-stale`）は区別して残し、実地で `StaleConfirm` がどの程度
発火するかの観測（本来の Stage 1 の目的）は引き続き継続する。

**なぜ最初にこれを見落としたか:** 設計レビュー（Opus）は「retype 対象の取り
違え」「SHOW 由来 fencing のレース」という fencing の**判定ロジック**の欠陥は
指摘したが、判定結果を受けた**per-VK confirm ループ側のアクション**（1文字目
で発火した場合に後続 VK が失われる）までは検証しておらず、実装者（本セッション）
も机上のユニットテストのみで実機投入前の検証を止めていた。実機ログでの
即時発見・修正に留められた。

**追補2（2026-07-23 実機）: 「既に可視」ショートカットに `EPOCH_FENCE_GRACE_MS`
の猶予が移植されておらず、高速タイピング中に正しく合成できていた文字が
false positive の stale confirm で繰り返し失われていた。**

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` → `Windows.UI.Input.
InputSite.WindowClass`、GJI、TSF-native）で NICOLA 同時打鍵により高速に連続
入力すると、`[raw-tsf-literal] cold=N raw TSF literal suspected` / `stale
confirm 検出` による backspace が「なぞに発火する」とユーザーから報告
（2026-07-23）。実機ログでは romaji "de"（「で」）の送信が `cold=45→46→47` の
3世代連続で `epoch-fence-stale` と判定され、2世代目までは backspace+再送で
自己修復したが、3世代目は `consecutive raw-tsf-literal (count=2) → giving up,
backs=1 cleanup only (no re-send)` に落ち、**再送なしで「で」が完全に消失した**。
同セッションの別ログでは「かこ１しゅうかん」と入力したかった文字列で、「しゅ」の
「ゅ」が1つ余分に先行する語順崩れも観測されており、同じ経路が自己修復で
辛うじて即座には露見しなかったケースと考えられる。

**原因（確定、コード読解で確認）:** `probe.rs::LiteralDetector::check_now`
（SHOW-only 分岐、661-677行目当時）は、fencing 判定で `evidence_is_fresh ==
false` でも即座に `StaleConfirm` を返さず、`show_stale_hold_since_ms` +
`EPOCH_FENCE_GRACE_MS`（= `GJI_SAMPLE_INTERVAL_MS × 2` = 20ms）による猶予を
挟んでから再判定していた——「`EVENT_OBJECT_SHOW` は write-bytes ポーリング
（`GJI_SAMPLE_INTERVAL_MS`）より早く届きうる」という benign なレースを
吸収するための設計（追補1以前からの既存機能）。

ところが `probe_fsm.rs::await_vk_detection` の「候補ウィンドウ既に可視」
ショートカット（BUG-29 由来、高速タイピングで候補ウィンドウが開きっぱなしの
場合に毎回通る経路）は、`check_now` を経由せず**独自に同じ epoch 比較を
inline で再実装**しており（Stage 1 導入時のコメントには「同じ fencing 条件を
適用した」とあったが実際には猶予ロジックが移植されていなかった）、
`last_write_ms >= epoch_send_ms` を一発判定するだけで、猶予を一切設けていな
かった。この経路は VK 送信直後の**最初の tick**（10ms 後）で発火するため、
GJI I/O monitor のポーリングサンプルが追いつく前——つまり合成が実際には
成功していても——に構造的に false positive の `StaleConfirm` を返し続ける
状態になっていた。ADR-079 自身が decision #1 で「fencing は `LiteralDetector`
に置く」と明記していたにもかかわらず、このショートカットだけが独自実装で
迂回していた点も設計逸脱だった。

**修正 (2026-07-23):** `LiteralDetector` に、`check_now` の SHOW-only 猶予
ロジックを共通化した `grace_hold_verdict` を新設し、`check_now` 自身もこれを
呼ぶようリファクタした。さらに「既に可視」ショートカット専用の
`visible_fencing_verdict(&self, deadline_ms) -> Option<DetectionResult>` を
追加し、これも `grace_hold_verdict` を共有する（`check_now` に直接委譲する
方式は採用しなかった。既に可視の場合、`check_now` の確定シグナル自体
（write-bytes 閾値・SHOW エッジ）が構造的に発火しないため）。
`await_vk_detection` はこれを `Some` が返るまで tick ごとに呼び直すループに
変更した（`None` の間は猶予中として空 action で待機）。1 detector インスタンス
につき `check_now` 経由か `visible_fencing_verdict` 経由かは
`gji_candidate_visible_now()` で排他的に決まるため、共有する
`show_stale_hold_since_ms` の hold 状態が競合することはない。

**テスト:** `tsf/warmup/probe_fsm.rs::tests` の既存回帰テスト
`chrome_per_vk_stale_confirm_from_leftover_candidate_window_recovers_like_suspected_literal`
を、猶予期間中は action 無し→猶予切れ後に recovery、という2 tick 構成に更新。
新規に `chrome_per_vk_visible_shortcut_confirms_when_write_catches_up_within_grace`
を追加し、猶予期間内に `gji_last_write_ms` が追いつけば `CompositionConfirmed`
となり backspace 回収が一切発行されないことを固定した（今回の regression が
実際に露呈していたはずのケース）。`cargo check`/`cargo clippy -p awase-windows
--lib --target x86_64-pc-windows-gnu -- -D warnings`（警告ゼロ）、`cargo test
-p awase-windows --lib --target x86_64-pc-windows-gnu --no-run`（リンク確認）
まで実施。wine 未導入のためこのサンドボックスでは `#[cfg(windows)]` テストの
実行そのものは Windows 実機/CI 待ち。

**関連ファイル:** `crates/awase-windows/src/tsf/probe.rs`
（`LiteralDetector::grace_hold_verdict`/`visible_fencing_verdict` 新設、
`check_now` リファクタ）、`crates/awase-windows/src/tsf/warmup/probe_fsm.rs`
（`await_vk_detection` をループ化）。関連: BUG-29（ショートカット自体の起源）、
本エントリ追補1（StaleConfirm の recovery 化）、
[ADR-079](adr/079-epoch-fenced-literal-recovery-with-replay.md)。

---

## デバッグ方法

ログ出力（`RUST_LOG=debug`）で以下のキーワードを確認する:

| ログキーワード | 意味 |
|---|---|
| `[composition] marked cold reason=X idle=Yms` | cold-start 発生。reason と idle 時間を確認 |
| `[h1-probe] cold=N long_idle=B f2_gji_long_idle=B idle_at_cold=Xms min=Yms max=Zms` | Chrome probe パラメータ |
| `[h1-warmup] cold=N eager_settle_ms=Xms probe_min_ms=Yms reason=Z` | WezTerm TSF probe パラメータ |
| `[tsf-probe] cold=N ChromeProbe 完了 → batched 送信 (Xms)` | Chrome probe 完了・経過時間 |
| `[tsf-probe] cold=N GjiProbe 完了 (Xms, gji_idle=Yms, settled=B)` | GJI probe 完了 |
| `[tsf-probe] cold=N NameChangeWait → nc_fired=B timed_out=B` | NameChangeWait 状態 |
| `[raw-tsf-literal] cold=N composition confirmed` | LiteralDetect: 正常 composition 判定 |
| `[raw-tsf-literal] cold=N raw TSF literal suspected → BS ×N` | LiteralDetect: literal 疑い → リカバリ |
| `[gji-candidate] SHOW #N` / `HIDE` | GJI 候補ウィンドウ表示/非表示 |
| `[gji-poll] GJI I/O Xms ago predates focus change` | GJI が focus change より前に静止 |
| `[composition] marked warm (epoch=N)` | probe 完了・warm 確定 |
| `[hook] IME-mode vk=0xXX dir self_injected=B injected=B scan=0xXX extra=0xXX` | IME モードキー到達診断（injected=LLKHF_INJECTED、BUG-08/BUG-14 の注入元切り分け） |
| `[hook] foreign-injected VK_KANA dir を swallow` | 外部注入 VK_KANA の遮断（BUG-08 防御。VK_KANA 以外の swallow は BUG-14 で撤回済み） |
| `[shadow-toggle] injected IME キー vk=0xXX はユーザー意図に昇格させない (BUG-14)` | 外部注入 IME モードキーの意図昇格ガードが発動（OS への配送は維持） |
| `[shift-conv-guard] Shift 押下 → IME-ON 半角英数へ切替` | Shift conv 安全網 entry（BUG-15/BUG-25。安全網ブリップか持続トグルの開始か、直後の `[shift-conv-guard] 左Shift単独タップ → 半角英数トグルON` の有無で判別） |
| `[shift-conv-guard] 左Shift単独タップ → 半角英数トグルON (conv=0x0000 維持)` | BUG-25: 左Shift単独タップで持続トグル開始（復元をスキップ） |
| `[shift-conv-guard] かな入力へ復元` | BUG-15/BUG-25: conv をかな入力へ verify-retry 復元（安全網ブリップの終了、またはトグルOFF） |
| `[tip-detect] IME kind candidate X (current=Y), awaiting confirmation next tick` | CLSID 種別フリップの1回目の観測（`ImeKindDebounce`）。次 tick も同じなら確定、元に戻れば破棄 |
| `[tip-detect] IME kind → X` | CLSID 種別変化が2 tick連続で確定し `WM_IME_KIND_CHANGED` を発行（`GjiFsm`/`MsImeStrategy` が再構築される点に注意、BUG-17） |
| `stale confirm 検出` / `epoch-fence-stale` | ADR-079/BUG-33: confirm 根拠が前世代由来と判明（追補1で SuspectedLiteral と同じ backspace+再送に変更済み。追補2で「既に可視」ショートカットの猶予漏れによる false positive も修正済み） |
