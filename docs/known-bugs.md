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

**追補1（2026-07-25）: `InjectionMode::Vk` への拡張** 当初の修正は
`InjectionMode::Tsf`（WezTerm 等、`force_tsf` 設定アプリ）にしか `ms_ime_gate_defer`
を配線していなかった。`InjectionMode::Vk`（Chrome/Edge/Electron、および `force_tsf`
未設定の既定 TsfNative アプリ = Windows Terminal 等）は `needs_f2_probe()` が
MS-IME 戦略で常に false のため GJI probe 分岐にも入らず、cold-start 保護が構造的に
存在しなかった。`IMC_GETCONVERSIONMODE` の観測自体は injection mode に依存しない
（`send_chrome_gji_reinit_and_poll` が Vk/Chrome 経路で同一 API を既に本番運用している
実績あり）ため、`ms_ime_gate_defer` を `target: TransmitTarget` でパラメータ化し
`send_romaji_batched`（Vk モード）からも呼ぶよう拡張した。`MsImeReadyCoro` の
Phase 2 Transmit も `TransmitTarget::Tsf` 固定から `target` 経由に変更。
**実機検証は未実施**（Vk 対象アプリで実際に本バグと同型のリテラル化が起きるかの
確認、および IMC がその環境で読めるかの確認が必要）。give-up（期限切れ）時は
未確認のまま強制送信するため、IMC が読めない Vk アプリでは引き続き無保護になる
残留リスクがある。新規ファイルの追加は不要（既存の `ms_ime_gate_defer`/
`MsImeReadyCoro` を共用）。回帰テストは
`ms_ime_ready_coro.rs::tests::coro_transmits_via_chrome_target_when_installed_for_vk_mode`。

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

**追補2（2026-08-08、ADR-086 Phase 3 実装時の巻き添え発見）:** この
non-ImmCross 分岐は `runtime/mod.rs::reschedule_ime_refresh` の周期リフレッシュ
連鎖（`is_tsf_native` 早期 return を force_policy のときだけスキップする例外）
に相乗りして呼ばれている。ADR-086 Phase 3 実装時にこの例外を一度撤去した際、
本分岐の周期実行機会も巻き添えで失われるところだった（2回目 opus アドバーサリアル
レビューで発見・例外を復元、詳細は
[ADR-086](adr/086-force-write-trigger-and-target-identity.md) §7-12・
[docs/experiments.md](experiments.md) エントリ13）。

**追補（2026-07-21、修正が dead code で一度も実行されていなかったことが判明・再修正済み）:**

**症状:** Windows Terminal（`Windows.UI.Input.InputSite.WindowClass`、`force-tsf` 判定で
`profile=TsfNative`、GJI）で、症状発生前の直近フォーカスは msedge（`Chrome_WidgetWin_1`,
`Imm32Unavailable`）だった。Windows Terminal 側で `Ctrl+無変換`（vk=0x1D, ctrl=true）を
押下し `IME OFF (key combo)` を送信、`[apply-ime] GJI direct: send 0x001A (open=false)` →
`outcome=Applied` → `SetOpen(false) applied → Off (belief, unconfirmed)` まではログ上
正常に見えるが、以後37秒間（実機ログ 01:16:34〜01:17:11 確認）にわたって実 conv は
一度も `0x00000019`（NATIVE|FULLSHAPE|ROMAN = ひらがなローマ字、IME 実体は ON のまま）
から変化しなかった。この間 `[idle-conv-check]` は
`conv observation open=true reason=NativeToggleShadowOff → ObserverReported として記録
(engine は actuate しない)` を継続的に出力するが、それを補正するはずの
`[drift] correction:` ログは一度も出力されなかった。ユーザーはタイピングを継続していた
（`idle_ms` が `TYPING_IDLE_MS` を割らない）ため、この間ずっと belief 上は
IME OFF（`desired_open=false`）、実 IME は ON のまま、という BUG-20 と同一症状
（IME ON / Engine OFF 系の乖離固着）が再現した。

**原因（確定）:** BUG-20 の修正コミット（`e8ffcd6`）は `ir_apply_drift_correction()`
に `can_use_imm32_cross_process()` による分岐（ImmCross は `set_ime_open`、
non-ImmCross は `apply_ime_open_with_belief` 経由の実 VK 送信）を追加したが、
関数冒頭に **BUG-20 修正前から存在していた** 次のガードを消し忘れていた。

```rust
fn ir_apply_drift_correction(&mut self) {
    if self.ir_resolve_skip_imm_query() {   // = !can_use_imm32_cross_process()
        return;
    }
    ...
```

`ir_resolve_skip_imm_query()` は `!can_use_imm32_cross_process()` そのものであり、
GJI/TsfNative（Windows Terminal・Chrome 等）や Blacklist アプリでは常に `true` を
返す。つまり BUG-20 で追加した non-ImmCross 分岐（`else` 節）は、それが実行される
べき条件のときに **必ずこの早期 `return` で先に抜けてしまい、構造的に到達不能な
dead code** になっていた。`check_drift_correction`（純粋関数、`platform_state.rs`）
自体は `explicit_intent=Some(false)`（Ctrl+無変換 由来）と `desired=false` が一致し
`trusted.open=true` との乖離が 400ms（`DRIFT_CORRECTION_THRESHOLD_MS`）を超えている
ため即座に `Some((false, true, duration_ms))` を返す状態だったが、呼び出し元の
`ir_apply_drift_correction` がその手前で毎回無言で return していたため一度も
`log::warn!("[drift] correction: ...")` すら出力されなかった（早期 return に
ログが無いため、症状発生中のログを見ても「何も起きていない」ようにしか見えない）。

BUG-20 のコミットメッセージ・`known-bugs.md` 本文どちらにも「実機検証は未実施」と
明記されていた通り、この dead code は一度も実機で検証されないまま今日まで残っていた。

**なぜ強制再送は安全か（BUG-19 との関係）:** この修正は「conv が desired と食い違って
いたら強制的に再送する」という、BUG-19 が問題にした挙動（conv の一発誤読だけを根拠に
`desired_open` を書き換えてユーザーの明示 OFF を踏み潰す）の再燃に見えるかもしれないが、
別物である。`check_drift_correction` は BUG-19/BUG-20 で追加された次の二重ガードを
維持したまま呼び出される:

1. `trusted.source == ConvOpenInference && explicit_intent.is_none()` の場合は補正を
   発火させない（`platform_state.rs:442-444`、`desired_open` そのものへの書き込みは
   一切発生しない — この修正は `desired_open` を経由せず、既に確定している `desired`
   を実機に再送するだけ）。
2. 今回のシナリオは `explicit_intent=Some(false)` が `desired=false` と一致する
   （ユーザーが実際に Ctrl+無変換 を押した）ため、上記ガードを素通りし、正しく
   `desired`（ユーザーの意図した値）を再送する設計通りの経路に入る。

**修正:** `ir_apply_drift_correction()` 冒頭の `ir_resolve_skip_imm_query()` 早期
`return` を削除。呼び出し元 `ir_stage_notify()` のコメント（「ImmCross アプリ向け」）
も実態に合わせて更新。

**検証状況:** Linux 上で `cargo build -p awase-windows --target x86_64-pc-windows-gnu`
（クロスコンパイル成功）、`cargo clippy -p awase-windows --target x86_64-pc-windows-gnu
-- -D warnings`（警告なし）、`cargo test -p awase-windows --lib`（135件全通過）、
`cargo test -p awase-windows --test golden_scenarios --test architecture_guard
--test layer_boundary_guard --test journal_replay`（全通過）を確認済み。
`ir_apply_drift_correction` の先（`unsafe` Win32 API 群）は BUG-20 と同じ制約で
Linux 上でのユニットテストが書けないため、実機（Windows Terminal + GJI、Ctrl+無変換
での明示 OFF 後に conv が変化しないケース）での動作確認は引き続き未実施。

**関連ファイル:** `crates/awase-windows/src/runtime/ime_refresh.rs`
（`ir_apply_drift_correction`, `ir_stage_notify`）

**関連バグ:** BUG-19（`check_drift_correction` の `explicit_intent` ガードの根拠）

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

## BUG-33: `Imm32Unavailable` プロファイルでは drift correction が構造的に一度も発火し得ない（belief 自身を「観測」として書き戻す循環）

**症状:** Chrome（`Chrome_WidgetWin_1`、`Imm32Unavailable` プロファイル）で GJI を使用中、
物理 F2 押下直後の通常タイピングで `[raw-tsf-literal] cold=N consecutive raw-tsf-literal
(count=2/3/4) → giving up, backs=1 cleanup only (no re-send)` が4連続で発火し、romaji
再送なしの単発 backspace だけで後始末される「変な感じのバックスペース」をユーザーが
実機で観測した（2026-07-22 実機ログ、`せ` を含む複数文字が literal 化 → backspace のみで
消失・語順崩れ）。全ログを通じて `[drift]` 系のログが一度も出力されていない。

**IME:** GJI（Google 日本語入力）。`Imm32Unavailable`（Chrome/WezTerm/Windows Terminal 等、
`ImmGetOpenStatus` が使えないプロファイル）。

**原因（確定、コード読解で確認）:** `check_drift_correction`（`state/platform_state.rs:407-450`）
は `observations.most_recent_trusted()` が返す観測と `desired_open` が一致していれば
「乖離なし」として即 `None` を返す。ところが `Imm32Unavailable` では以下の経路で
**belief 自身の値がそのまま「観測」として観測ストアに書き戻される**:

1. `apply_focus_probe`（`runtime/key_pipeline.rs:249`）が `shadow_on =
   self.platform_state.ime.effective_open()`（＝現在の belief そのもの）を probe 実行前に
   キャプチャする。
2. `Imm32Unavailable` では `ImmGetOpenStatus` 相当が使えず `probe_ime_on = None` になるため、
   フォールバックとして `apply_effective_ime(shadow_on, ...)` →
   `write_focus_probe(shadow_on, ...)`（`key_pipeline.rs:1462-1480`、コメント曰く
   「shadow の apply 値を代替観測として記録」）が呼ばれ、`ObservationSource::FocusProbe`
   （confidence=Low）として観測ストアに `open = shadow_on` を記録する
   （`platform_state.rs:762-780`）。
3. `most_recent_trusted()`（`observation_store.rs:214-219`）は confidence の下限を設けて
   おらず、他に観測が無ければこの Low 観測がそのまま採用される。
4. この観測は定義上 `open == desired`（同じ値を書き写しただけ）になるため、
   `check_drift_correction` の `if trusted.open == desired { return None; }`
   （`platform_state.rs:445-447`）に毎回引っかかり、**「乖離あり」と判定される入力が
   そもそも生成されない**。

GJI I/O からの実測（`ObservationSource::GjiIoInference`）は input_mode 判定にのみ使われ、
`observation_store.rs:92,111` で明示的に open 観測ストアには記録されない
（`ConvBitsInference | GjiIoInference => None` / `=> {}`）。`ConvOpenInference` も
明示ユーザー意図が無いと gate される（BUG-19 対策、`platform_state.rs:442-444`）ため、
Chrome+GJI で明示 intent なしに belief が実状態から乖離するケース（今回のログのように、
物理 F2 押下と自己注入 F2 の交錯などで乖離が生じるケース）は drift correction では
一切救えない。

**BUG-20 との違い:** BUG-20 は「non-ImmCross アプリでは `set_ime_open` が no-op なのに
belief だけ適用済み扱いにしていた」という *送信側* の欠陥（`57cab1d` で修正済み）。
本バグは *検知側* の欠陥で、そもそも `Imm32Unavailable` に belief を裏付ける本物の
観測ソースが存在しないため、BUG-20 の修正が入っていても drift correction 自体が
発火しない。両方直って初めて non-ImmCross アプリでの drift correction が機能する。

**修正 (2026-07-22):** belief/drift-correction 経路（observation store に本物の open
観測を足す案、下記「検討したが見送った案」）ではなく、**per-VK confirm の give-up 分岐
から実 IME-ON 再送を直接行う**、より小さく効果が即時なパスを採用した。

- `probe_io.rs` の `ProbeAction::RawTsfLiteralRecovery` ハンドラで、`consecutive > 0`
  （2連続失敗＝give-up）のときだけ `io.send_chrome_gji_reinit_and_poll(cold_seq)` を
  追加で呼ぶ。1回目の疑い（`consecutive == 0`）では呼ばない — BUG-29/BUG-30 で分かって
  いる候補ウィンドウ SHOW/HIDE レース由来の偽陽性の可能性がまだ高いため、give-up 自体が
  課している「2連続失敗」の閾値にそのまま相乗りする。
- `send_chrome_gji_reinit_and_poll` は 2026-07-18 の「捨て駒キー」撤去（本ファイル追補8、
  `docs/experiments.md` エントリ10）で削除された `SacrificialWarmupCoro` 等の重量級機構
  とは別物で、Unicode injection mode の long-cold GJI 再起動（`platform.rs:307`）向けに
  既に実装・実運用されていた既存メソッドをそのまま再利用しただけ（新規実装は呼び出しの
  追加のみ）。`VK_IME_OFF→VK_IME_ON` を実際に `SendInput` し、`CHROME_GJI_REINIT_CONFIRM_MS`
  （300ms、実測較正済み）で IMC を async ポーリングして Hiragana 確認まで見届ける。
  「捨て駒キー」削除判断（送信**前**の予防的待機は不要）とは矛盾しない — 本修正は
  give-up **後**の実際の状態修復であり、別の対象を直している。
- give-up が短時間に連発した場合の多重発火を防ぐため、`Output::last_gji_reinit_ms`
  （新規フィールド）で `CHROME_GJI_REINIT_CONFIRM_MS` 未満の再発火をレート制限した
  （`send_chrome_gji_reinit_and_poll` 冒頭に追加）。OFF→ON の最終到達状態は決定論的に
  ON だが、窓内に重ねて送ると瞬間的な OFF ブリップが積み重なり候補ウィンドウを無用に
  揺らしかねないため。

**なぜ n-gram/統計判定が不要か:** 検討時、「literal 化した romaji が本当に日本語の
つもりだったか、それとも英語入力のつもりだったか」を統計的に推測する案も出たが、
コード上不要と判明した。per-VK confirm の literal-detect（`RawTsfLiteralRecovery`）に
入るのは `vk_send.rs::send_char_as_vk` の `CharResolution::Romaji` 分岐だけであり、
これは NICOLA チョード判定（`src/engine/`）が「このかな文字を出力する」と**既に決定した
後**の値にしか発生しない。ユーザーが明示的に半角英数入力したいときはそもそも別分岐
（`CharResolution::Vk`/`Unicode`）を通りこのパイプラインに来ない。つまり give-up が
発火した時点で「日本語のつもりだった」は常に真であり、推測の余地がない。

**なぜ shadow 反転のリスクが無いか:** `GjiDirectStrategy::apply`（`ime_controller.rs`）は
`VK_KANJI` のようなトグルキーではなく、ON/OFF 専用の冪等キー（`VK_IME_ON`/`VK_IME_OFF`）
を使う設計（コメント: 「shadow desync の影響を排除する」）。`send_chrome_gji_reinit_and_poll`
も同じ冪等キーで OFF→ON を送るため、万一 give-up が偽陽性で実際には既に IME ON だった
場合でも、最終到達状態は変わらず ON のまま（OFF は一瞬の経由点であり、二重反転で OFF に
固定される心配は無い）。

**検討したが見送った案:** belief/drift-correction を経由する設計（`RawTsfLiteralRecovery`
give-up と `CompositionConfirmed` から `ObservationSource` 新設 variant で
`ObserverReported` を dispatch し、`check_drift_correction` に本物の観測を与える）。
コード調査で実現可能と確認済み（`ir_apply_drift_correction` は `applied: None` を渡す
ことで `GjiDirectStrategy` の shadow_on skip ガードを迂回し実送信することも確認済み）
だったが、(a) 400ms の `DRIFT_CORRECTION_THRESHOLD_MS` を待つ分レイテンシが乗る、
(b) 新規 `ObservationSource` variant・`observation_store.rs` の分岐追加が要る、という
理由で、per-VK confirm 内で完結する直接呼び出しより変更範囲・待ち時間ともに大きいと
判断し見送った。GJI が**タイピング中でなく**長時間 OFF のまま乖離するケース（per-VK
confirm を一切通らない）にはこの直接呼び出しは効かないため、将来そのようなケースが
実機で確認されたら、この belief 経由の設計を別バグとして再検討すること。

**テスト:** `crates/awase-windows/src/output/probe_io.rs` の
`raw_tsf_literal_recovery_tsf_mode_consecutive_gives_up_with_cold_mark`（既存、
`gji_reinit_call_count == 1` の assertion を追加）と
`raw_tsf_literal_recovery_sets_literal_and_marks_cold_when_first_time`（既存、
`gji_reinit_call_count == 0` の assertion を追加）で「give-up のときだけ実 IME-ON
再送が発火する」ことを検証。`send_chrome_gji_reinit_and_poll` 自体は Win32 API
（`SendInput`）に直結するため Linux 上でのユニットテストは非現実的（既存の `tsf` 系
テストと同じ制約）。`cargo check`/`cargo clippy -p awase-windows --target
x86_64-pc-windows-gnu --lib -- -A clippy::cargo_common_metadata -D warnings`（警告
ゼロ）、`cargo test -p awase-windows --target x86_64-pc-windows-gnu --no-run`（lib・
`tests/*.rs` 全ファイルのコンパイル・リンク）確認済み。wine 未導入のためこのサンドボックス
では `.exe` 実行はできない（実機確認結果は下記「実機ソーク結果」）。

**実機ソーク結果 (2026-07-22、修正コミット `fabe6d6` 反映後):** ユーザーが Chrome+GJI で
通常使用を継続。修正前は本バグ記載の症状（`giving up, backs=1 cleanup only (no re-send)`
ログが連発、体感でも VK_BACK の誤発火が多発）が出ていたが、修正後は `[raw-tsf-literal] ...
giving up` ログ自体が一度も出ていないことを確認（ユーザー報告: 「giving up のログはない
ね」「一時はすごくVK_BACKが誤発火していたけど、なくなってる」）。厳密な時間・シナリオを
区切った多日ソーク（BUG-31/BUG-21 のような）ではなく通常使用中の確認のため、継続して
様子を見ること。give-up 自体が0件ということは、per-VK confirm の初回疑い
（`consecutive == 0`、backspace + romaji 再送）の時点で既に回収できている、または
そもそも suspected literal 自体が発生していない可能性がある。「give-up には至るが
reinit で後続文字が正常化する」ケースを意図的に踏んだ確認ではまだないため、
`send_chrome_gji_reinit_and_poll` 呼び出し自体の実効性（VK_IME_OFF→VK_IME_ON 後に
GJI が実際に ON に戻るか）は次回 give-up が実機で発生した際にログで確認すること。

**関連ファイル:** `crates/awase-windows/src/output/probe_io.rs`
（`RawTsfLiteralRecovery` ハンドラ、`send_chrome_gji_reinit_and_poll` のレート制限）、
`crates/awase-windows/src/output/mod.rs`（`Output::last_gji_reinit_ms` フィールド追加）、
`crates/awase-windows/src/tuning.rs`（`CHROME_GJI_REINIT_CONFIRM_MS` のコメント更新）。
検知側の未解決ギャップ（drift-correction が構造的に発火しない件）自体は本修正後も
残存する: `crates/awase-windows/src/state/platform_state.rs`
（`check_drift_correction`, `write_focus_probe`）、
`crates/awase-windows/src/runtime/key_pipeline.rs`（`apply_focus_probe`,
`apply_effective_ime`）、`crates/awase-windows/src/state/observation_store.rs`
（`most_recent_trusted`, `ObservationSource` ごとの record 分岐）。関連バグ: BUG-20
（送信側の対称バグ）、BUG-19（`ConvOpenInference` gate の元ネタ）、BUG-21（同じ
`send_chrome_gji_reinit_and_poll` 系統の別トリガー元）、BUG-27（give-up 分岐で romaji
を再送しない設計の由来）、BUG-29/BUG-30（literal 誤検知の偽陽性源）、BUG-31/BUG-32
（同じ literal storm 症状の別原因）。

---

## BUG-34: `SendMessageTimeoutW(SMTO_ABORTIFHUNG)` の `timeout_ms` 未保証により、エンジンスレッド上の同期 IME 読み取りが数秒ブロックし打鍵が消える

**症状:** WezTerm（TsfNative、GJI）で連続タイピング中、`[engine-input]` ログの `delay=` が
突然 5000〜6000ms 台に跳ね上がり、その間の打鍵がまとめてバースト処理される
（2026-07-22 実機ログ）。ユーザー視点では「文字が消えて、しばらく待ってから
まとめて出てくる」と観測される。低レベルフックは常に `LRESULT(1)` でキーを消費してから
`PostThreadMessageW` でエンジンスレッドに転送するため、エンジンスレッドのメッセージ
ループが詰まっている間、物理キー入力はどこにも表示されない。

**原因（確定、コード読解 + ログ解析で確認）:** `event.timestamp`（フック捕捉時刻）と
`now_us`（`key_pipeline.rs::kp_run_inner` がエンジン側で処理する時刻）は同一の
`Instant` ベースクロック（`hook::now_timestamp_us()`、`hook.rs:790-799`）を使っており、
時計のズレではなく実際のメッセージループ停止であることが確認できる。

止まっていた間のログは一切出力されないため直接の証拠は無いが、`imm.rs::send_ime_control`
（`SendMessageTimeoutW` のラッパー、`imm.rs:119-141`）のコメントには「`SMTO_ABORTIFHUNG`
により `timeout_ms` 内に制御が戻ることが保証される」とあり、これは **Win32 のよく知られた
誤解**である。`SMTO_ABORTIFHUNG` は送信先スレッドが**既に**ハングとマークされている
場合のみ即座に打ち切る。送信先がその瞬間から重くなり始めた場合、Windows がハングと
判定するまで（`HungAppTimeout`、既定 ~5000ms）呼び出し元は普通にブロックし続け、
呼び出し側が指定した小さな `timeout_ms`（大抵 5〜50ms）はこの一次ブロックには効かない。
観測された `delay=5741ms` 等の値は `HungAppTimeout` の既定値（~5000ms）+αとほぼ一致する。

これらの呼び出しが **エンジンスレッド（メッセージループそのもの）上で同期的に**
実行されていたため、ブロック中はキー処理が完全に止まっていた。

**修正済み（本コミット）:** `runtime/key_pipeline.rs::kp_stage_idle_conv_check`
（TsfNative アイドル時の変換モード確認、通常タイピングの合間ごとに走る頻出経路）が
`crate::ime::get_ime_conversion_mode_raw_timeout`（同期版）を直接呼んでいた箇所を、
既存の `get_ime_conversion_mode_raw_timeout_async`（`ime.rs`、元々は
`probe_io.rs`/`vk_send.rs`/`cold_warmup.rs`/`platform.rs` の warmup 計測用に存在）へ
offload するよう変更。`kp_stage_focus_probe`/`apply_focus_probe`
（`state/probe_admission.rs` の `ImmLikeTicket`/`AcceptedObservation`、ADR-077）と
同型の epoch fencing を追加し、read spawn 後にフォーカスが変わっていれば結果を棄却する。
GJI が実際にハングしている間の断続タイピングで offload 呼び出しが積み上がらないよう、
`GateStore::idle_conv_check_in_flight` で多重 spawn を防止する。

**追補（セカンドオピニオンレビューで発覚・同コミットで修正）:** 上記の offload 化だけでは、
旧同期コードが暗黙に持っていた「読み取り開始から belief 適用までの間に awase 自身の状態が
変化し得ない」というアトミック性が壊れ、新たな race を生んでいた。`kp_stage_idle_conv_check`
は `kp_run_inner` の早い段階（decision 前）で読み取りを spawn するが、同一キーイベントの
後続処理（`kp_stage_post_decision` → `kp_stage_shift_conv_guard`、および `kp_stage_execute`
の warmup 送信）は spawn 後・apply 前に割り込める。旧同期コードでは読み取りが完了してから
これらが走っていたため、以下は構造的に起こり得なかった:

1. spawn 時点では `shift_conv_guard_pending`/`half_width_alnum_toggle_active` が false でも、
   同じキーイベントの `kp_stage_shift_conv_guard` が spawn 後に true へ倒して `conv=0x0000`
   の IMC write を発行し、apply 時にその値を「ユーザーの英数モード」として拾ってしまう
   （awase 自身が立てた保護対象の conv を awase 自身が誤読して IME を強制 OFF する）。
2. 同様に、spawn 後に発生した explicit IME 操作（Ctrl+変換/無変換 等）による抑制が
   apply 時には再評価されず、ユーザーの明示操作を打ち消す方向に belief を書き換えてしまう。
3. cold-start 等で、spawn 直後に awase 自身が warmup（`ImmSetConversionStatus`/VK_DBE_HIRAGANA
   等）を送信すると、offload 先のワーカースレッドが conv を**遷移途中**でサンプリングし、
   汚染された値を belief 適用に使ってしまう。

`ImmLikeTicket`/`FocusEpoch` によるフォーカス変更の棄却だけではこれらを検出できない
（フォーカスは変わっていないため）。対策として `apply_idle_conv_check` の先頭で、
spawn 時にキャプチャしたスナップショット（`output_idle_ms_at_spawn` / `now_tick_at_spawn`）
と apply 時点を突き合わせ、(1)(2) はガード状態・explicit age を再評価して、(3) は
`output_in_flight_ms()` を絶対時刻（最終送信時刻）に換算して spawn 時と apply 時で
一致するかを確認し、いずれか変化していれば読み取り結果を丸ごと破棄するようにした。

**未対応（残存、フォローアップ候補）:** 同じ `SMTO_ABORTIFHUNG` 誤解に基づく同期呼び出しが
他にも残っている。いずれもエンジンスレッド上で同期実行されるため、条件が揃えば同様に
数秒ブロックしうる:

- `runtime/key_pipeline.rs:740,917,1070,1261` 付近（`get_ime_conversion_mode_raw_timeout`
  / `read_ime_state_fast` の同期呼び出し）
- `runtime/key_pipeline.rs::apply_focus_probe` 内（1546 行付近、`get_ime_conversion_mode_raw_timeout(10)`
  の同期呼び出し — `with_app` コールバック内＝メインスレッド上）
- `ime_controller.rs::MsImeDirectStrategy::apply`（KATAKANA チェックの
  `get_ime_conversion_mode_raw_timeout(5)`）
- `runtime/executor.rs:749,793` 付近（`WM_EXECUTE_EFFECTS` ハンドラの post-apply verification）
- `runtime/ime_refresh.rs:492` 付近（`TIMER_IME_REFRESH` ハンドラ）

これらを直す際は、いずれも `get_ime_conversion_mode_raw_timeout_async`
（または `read_ime_state_fast_async`）への置き換え + 呼び出し元の同期前提の再設計
（結果を今すぐ使う必要があるか、後続処理を async continuation に移せるか）が必要になる。
`ime_controller.rs::apply()` 自体は `apply_skipping_imm` 経由の呼び出しでは
`ImmCrossProcessStrategy` を通らないため `SendMessageTimeoutW(IMC_SETOPENSTATUS)` の
ブロックとは無縁だが、`MsImeDirectStrategy` の KATAKANA チェックは
`ImmCrossProcessStrategy` の有無に関わらず独立して `SendMessageTimeoutW` を呼ぶため注意。

**検証状況:** 実機で「この特定の呼び出しが5秒ブロックしていた」ことを示す直接ログ
（呼び出し前後の `Instant` 計測）はまだ取れていない。`fix-requires-evidence.md` 的には
状況証拠（クロック解析 + Win32 API の既知の挙動 + ログの発生アプリ/タイミングの一致）
までで、次回実機検証（該当行前後にタイムスタンプログを仕込んで再現待ち）が望ましい。

**追補2（2026-07-24、実機ログ解析で発覚・本コミットで修正）:** 上記追補の (2) 対策
（`apply_idle_conv_check` で explicit age を再評価する）には、BUG-34 のブロック時間が
長い場合に無効化される抜け穴が残っていた。

**症状（実機ログ、GJI/TsfNative, hwnd class `Windows.UI.Input.InputSite.WindowClass`,
himc_null=true）:** ユーザーが shift-conv-guard（Shift 押下による IME-ON 半角英数の
安全網、GJI では entry 機構が無く実 conv は変化しない）を Shift を長押し（実測 約2.6秒）
して使った直後、実際の IME は `conv=0x00000019`（NATIVE、ひらがな）のまま変化していない
にもかかわらず `Engine deactivated (reason=Inactive(ImeOff))` が発火。以後
`[idle-conv-check] TsfNative: conv observation open=true reason=NativeToggleShadowOff
... → ObserverReported として記録 (engine は actuate しない)` が繰り返し出力され、
`[gji-fsm] StartComposition while engine off — ignored` で打鍵が消失し続けた
（ユーザー通報「なぜか、IME ON Engine OFF になる」、2026-07-24）。

**原因:** `apply_idle_conv_check` の (b) explicit age 再検証は、apply 時点の
「直近の明示的 IME 操作からの経過時間」を `EXPLICIT_IME_SUPPRESS_MS`（1500ms）と
比較するだけだった。Shift 押下（shift-conv-guard 突入、`note_explicit_ime_action`）の
**直前**に spawn された idle-conv-check の読み取りが BUG-34 で長時間（1500ms 超）
ブロックされた場合、apply 時点では「Shift 押下からの経過時間」が既に閾値を超えて
しまっており、この読み取りが shift-conv-guard 突入以降に汚染された `conv` 値
（半角英数扱い＝`ObservedEisu`）を拾っていても素通りしてしまう。`ObservedEisu` は
`classify_conv_transition`（`state/conv_classify.rs`）経由で `EngineSync::DirectInput`
となり、`handle_engine_set_open(false)` が `UserImeSetIntent{Command}` として
`desired_open=false` を確定させる（`state/ime_model.rs`）。この経路は
「ユーザーが明示的に OFF にした」ことを意味する状態になるため、以後
`NativeToggleShadowOff`（実 conv が NATIVE を示す観測）を何度観測しても
BUG-19 再発防止のガード（`state/conv_classify.rs`、`ConvOpenInference` は
明示意図が無い限り drift correction を発火させない設計）により自動復帰しない
（次のフォーカス変更で `last_intent` がクリアされるまで固定）。

**修正（本コミット）:** apply 時の explicit 操作再検証を、経過時間の閾値比較から
spawn 時にキャプチャした `last_explicit_ime_action_ms`（生のタイムスタンプ）と
apply 時点の値の**一致比較**に変更した（`ImeStateHub::last_explicit_ime_action_ms_raw()`
を追加、`kp_stage_idle_conv_check`/`apply_idle_conv_check` に
`explicit_action_ms_at_spawn` を追加）。値が変化していれば「spawn〜apply の間に
明示操作があった」ことが遅延の長さに関わらず確実に分かるため、ブロック時間が
`EXPLICIT_IME_SUPPRESS_MS` を超える場合でも読み取り結果を棄却できる。

**検証状況:** コード読解 + ログ解析による確定。実機での再現待ち（`RUST_LOG=debug` で
`[idle-conv-check] apply 時に spawn 後の explicit IME action を検出 → ... を破棄` が
出るかを確認する）。

**関連ファイル:** `crates/awase-windows/src/imm.rs`（`send_ime_control`）、
`crates/awase-windows/src/ime.rs`（`get_ime_conversion_mode_raw_timeout`,
`get_ime_conversion_mode_raw_timeout_async`）、
`crates/awase-windows/src/runtime/key_pipeline.rs`（`kp_stage_idle_conv_check`,
`apply_idle_conv_check`）、`crates/awase-windows/src/state/probe_admission.rs`
（`ImmLikeTicket`, `AcceptedObservation`, ADR-077）、
`crates/awase-windows/src/state/platform_state.rs`（`ImeStateHub::note_explicit_ime_action`,
`last_explicit_ime_action_ms_raw`）。

---

## BUG-35: per-VK confirm が世代をまたいだ stale な confirm 根拠を現世代の証拠として
誤って採用し、見捨てた世代の backspace が別スコープの確定済み文字を消す（ADR-079、Stage 1: 検出のみ実装）

> 統合ブランチでの注記: 本エントリは当初 `feat/adr079-epoch-fenced-literal-recovery`
> ブランチ側で「BUG-33」として書かれていたが、main 側に既に別の BUG-33
> （`Imm32Unavailable` drift correction 循環）が存在していたため、統合時に
> BUG-35 へ改番した（本文中の自己参照・`docs/adr/079-epoch-fenced-literal-recovery-with-replay.md`
> 内の参照も揃えて更新済み）。

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

**追補3（2026-07-23 実機）: 症状（backspace）ではなく前提条件（無根拠な cold
化）そのものを根治。`GjiFsm::handle_composition_reset` に `gji_idle_ms`
observation ゲートを追加。**

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` →
`Windows.UI.Input.InputSite.WindowClass`、GJI、TSF-native）で「リーク」と
連続入力した直後に `Ctrl+I` を押して「を」を続けたところ、**「リークを」が
「リーを」になった（「ク」が消失）**。

**再現手順（実機ログで確認）:**
1. 「り」「ー」「く」が connected composition として正常に入力される（GJI は
   継続的に warm、候補ウィンドウの開閉も正常）。
2. `Ctrl+I` 押下。`message_handlers.rs` の Ctrl+key パススルー処理が
   `gji_candidate_visible_now()`（＝候補ウィンドウが「今まさに」可視か、
   という素の代理指標）だけを根拠に、IMM32 の `cancel_ime_composition()` 直後
   `Platform::on_ctrl_bypass_composition_cancel()` → `gji_on_composition_reset()`
   → `GjiFsm::handle_composition_reset()` を呼ぶ。ここが実測 `gji_idle_ms()`
   を一切参照せず、`OnWarm`/`OnComposing` から無条件に `OnCold{Short}` へ
   強制していた。
3. 2026-07-18 以降、cold-start は理由（`ColdReason`/`ColdKind`）に関わらず
   即座に F2/probe 事前待機を省略して per-VK confirm（epoch fencing 付き）へ
   入る（`tsf/warmup/cold_warmup.rs::run_start`）。本来 cold-start が一切
   不要な genuinely warm なセッションまでこの経路に送り込まれた。
4. 続く「を」（romaji "wo"）の1文字目 `w` は正しく confirmed、2文字目 `o` が
   「候補ウィンドウ既に可視」ショートカット経由で `StaleConfirm` と判定され、
   `escape_composition=true`（VK_ESCAPE + VK_BACK×1）の回収が発火。ESC が
   直前の未確定 pending composition を破棄した結果、後続の BS×1 が
   「消すべき literal」を失い、代わりに直前の唯一の確定済み文字である
   「ク」を消した。

**原因（本質）:** `GjiEvent::CompositionReset`/`GjiEvent::NativeF2Consumed`
（Warm/Composing 中のフォールバック）が共通で通る
`GjiFsm::handle_composition_reset()` が、composition キャンセルという
弱い代理指標のみを根拠に、実際の GJI 生存証拠（`gji_idle_ms()`）を一切
参照せず無条件に cold へ強制していた。`FocusChange`/`ImeOn` は既に
`gji_idle_ms()` を観測して `ColdKind::classify` で判断しており、
`CompositionReset`/`NativeF2Consumed` だけがこの「observation → decision」
の原則から外れた例外だった。この経路の呼び出し元は実際には4種類ある
（Ctrl+key bypass、記号 VK 送信後 `SymbolVkSent`、Space/Enter/Escape
パススルー `PassthroughConfirmKey`、物理 F2 消費 `NativeF2Consumed` の
Warm/Composing フォールバック）が、いずれも `platform.rs` の
`gji_on_composition_reset()`/`gji_on_native_f2_consumed()` という
たった2つの関数を経由するだけなので、1箇所ずつ個別に直すのではなく
この共通関門を直すことで4経路すべてを一括是正した。

**修正 (2026-07-23):** `GjiEvent::CompositionReset`/`NativeF2Consumed` に
`gji_idle_ms: u64` を必須フィールドとして追加し（`FocusChange`/`ImeOn` と
同じパターン）、呼び出し元（`gji_on_composition_reset`/
`gji_on_native_f2_consumed`）が `crate::tsf::observer::gji_idle_ms()` を
取得して渡すようにした。`handle_composition_reset` は
`ColdKind::classify(gji_idle_ms)` が `Short`（enum 定義上「GJI 確実に生存」）
を返す場合は cold へ倒さず `OnWarm` に留まり（`transition_to_warm`）、
`Medium`/`Long`（genuinely stale）の場合のみ従来どおり `OnCold` へ遷移する。
`OnCold` 状態からの再遷移も、固定値 `ColdKind::Short` ではなく実測
`gji_idle_ms` で再分類するよう改めた。

`mark_composition_cold(ColdReason)` 系（9箇所）は、GJI warm/cold の機能的
SSOT（`GjiFsm::is_warm()`）を一切動かさない別の診断専用構造体
（`tsf/probe.rs::CompositionState`）を更新するだけと判明したため、今回は
変更していない。

**テスト:** `tsf/gji_fsm.rs::tests`（`#[cfg(windows)]` 無し、型としては
Windows 非依存だが親モジュール `tsf` が `#[cfg(windows)]` のため実行には
Windows ターゲットが必要）に3件追加: `composition_reset_while_genuinely_warm_stays_warm`
（実機再現、`gji_idle_ms=63` で `OnWarm` を維持することを固定）、
`composition_reset_while_genuinely_stale_transitions_cold`（過剰防御に
倒れていないことの確認、`gji_idle_ms=8_000` で `OnCold{Medium}` へ正しく
落ちる）、`native_f2_consumed_while_warm_and_fresh_stays_warm`（同じゲートが
`NativeF2Consumed` 経由でも効くことの確認）。既存2件
（`native_f2_consumed_while_medium_cold_continues_probe`/
`native_f2_consumed_while_short_cold_resets_probe`）も新シグネチャに追従。
`cargo check`/`cargo clippy -p awase-windows --lib --target
x86_64-pc-windows-gnu -- -D warnings`（警告ゼロ）、`cargo test -p
awase-windows --lib --target x86_64-pc-windows-gnu --no-run`（リンク確認）
まで実施。wine 未導入のためこのサンドボックスでは実行そのものは
Windows 実機/CI 待ち。

**2026-08-01追記（ADR-082決定1実施記録の次の一歩、Linux実行可能化）:**
`tsf/gji_fsm.rs` は windows crate 依存がゼロと判明したため、`tsf/mod.rs`
側で `gji_fsm` サブモジュールだけ `#[cfg(windows)]` を外し（他の10
サブモジュールは個別に `#[cfg(windows)]` を付与、`focus/mod.rs` と同じ
「ungated な親 mod + サブモジュール個別 gate」パターン）、上記3件を含む
既存33件のテストを `cargo test -p awase-windows --lib` からLinuxで常時
実行できるようにした（`InjectionMode` の定義は `state/injection_mode.rs`
へ移設、`InjectionHint` 依存の `From` 実装のみ `output/types.rs` に残置）。
加えて `composition_reset_and_native_f2_consumed_match_cold_kind_classify_across_boundary`
を追加し、上記3件が `gji_idle_ms=63/8_000/50` という特定値のみを点で
押さえていたのに対し、`MEDIUM_IDLE_PROBE_MS`(7000ms)/`LONG_IDLE_MS`(10000ms)
の閾値をまたぐ境界値（off-by-one含む）で `CompositionReset`/
`NativeF2Consumed` の遷移先が常に `ColdKind::classify(gji_idle_ms)` と
一致することをプロパティテストとして固定化した。

**関連ファイル:** `crates/awase-windows/src/tsf/gji_fsm.rs`
（`GjiEvent::CompositionReset`/`NativeF2Consumed` フィールド追加、
`handle_composition_reset` の observation ゲート化）、
`crates/awase-windows/src/platform.rs`（`gji_on_composition_reset`/
`gji_on_native_f2_consumed` が `gji_idle_ms()` を取得して渡す）。関連:
本エントリ追補1・追補2（症状面: per-VK confirm 側の StaleConfirm 処理）、
本追補3は前提条件面（そもそも genuinely warm なセッションを cold-start
経路に送り込まない）の根治。[ADR-079](adr/079-epoch-fenced-literal-recovery-with-replay.md)。

**追補4（2026-07-23 実機、2件確認）: `VK_BACK` に「literal の positive な証拠」
を要求するよう `per_vk_recovery_params` を変更。`StaleConfirm` および
「直前の VK が確認済みの `SuspectedLiteral`」では backspace を送らない。**

**症状:**
1. Windows Terminal で「リーク」に続けて `Ctrl+I` → 「を」を入力したところ
   「リークを」が「リーを」になった（「ク」が消失、per-VK[1/1] が
   `StaleConfirm`、`escape=true`）。
2. 同環境で IME OFF 中に "cold " をパススルー入力→物理 F2 で IME ON→「が」を
   入力したところ「cold が」が「coldが」になった（末尾スペースが消失、
   per-VK[1/1] が `StaleConfirm`、`escape=true`）。
3. 追加で、`StaleConfirm` が連鎖して `consecutive` カウントが 1→2→3→4 と
   積み上がり、その都度 backspace が発火して複数文字が連続して消える
   カスケード障害も観測した（per-VK[0/1] で `StaleConfirm` が3世代連続）。

**原因（本質）:** `per_vk_recovery_params(failed_idx)` は `DetectionResult`
の種別（`SuspectedLiteral` か `StaleConfirm` か）を区別せず、常に
`backs=1`（`failed_idx==0`）または「ESC + backs=1」（`failed_idx>0`）を
返していた。しかし `VK_ESCAPE` の破壊スコープは pending composition
内に閉じているのに対し、`VK_BACK` は「カーソル直前の1文字を無条件に消す」
命令であり composition スコープ**外**の確定済みテキストにも届く。
`escape_composition=true` のケースでは ESC が pending composition を
（それが実在すれば）破棄した後、消すべき literal がもはや存在しない状態で
backspace が発火し、直前の別スコープの確定済み文字を誤って消していた。
`StaleConfirm` は「confirm 根拠が古い」ことの検出であって「literal である」
証拠ではなく、`SuspectedLiteral` であっても `failed_idx>0`（直前の VK が
fresh に confirmed 済み＝GJI が「いま」この語を処理している直接証拠がある）
場合は「次の1VKだけが本当に IME をバイパスする」可能性が実質低い。

**修正:** `per_vk_recovery_params(is_stale: bool, failed_idx: usize)` に
シグネチャ変更し、`backs = if is_stale || failed_idx > 0 { 0 } else { 1 }`
とした。つまり:
- `StaleConfirm`（`is_stale=true`）: idx に関わらず backspace を送らない。
- `SuspectedLiteral` かつ `failed_idx > 0`: backspace を送らず ESC のみ。
- `SuspectedLiteral` かつ `failed_idx == 0`（この語で一度も confirm 済みの
  証拠がない唯一のケース）: 従来どおり `backs=1`。

`tsf/warmup/probe_fsm.rs`（`run_per_vk_confirm`・Chrome inline LiteralDetect
の両方）と `tsf/warmup/literal_detect_fsm.rs`（`LiteralDetectCore::poll` の
`StaleConfirm` 分岐）双方の `StaleConfirm` ハンドラを `backs=0` に統一した
（バッチ経路には `failed_idx` の概念が無いため `is_stale` のみで判定）。

**テスト:** `literal_detect_fsm.rs` に `per_vk_recovery_params` の全パターン
（stale×idx、非stale×idx）を固定するテストを追加・更新。
`probe_fsm.rs` に `chrome_per_vk_suspected_literal_after_confirmed_prior_vk_escapes_without_backspace`
（「cold が」バグの直接回帰テスト、本物の deadline 到達による
`SuspectedLiteral` を `failed_idx=1` で発生させ backspace なしを確認）を
新規追加。既存の stale confirm 系回帰テスト（`chrome_per_vk_stale_confirm_from_leftover_candidate_window_recovers_like_suspected_literal`
等）は `backs=0` を確認するようアサーションを更新。`cargo check`/`cargo
clippy -p awase-windows --lib --target x86_64-pc-windows-gnu -- -D
warnings`（警告ゼロ）、`cargo test -p awase-windows --lib --target
x86_64-pc-windows-gnu --no-run`（リンク確認）まで実施。wine 未導入のため
実行そのものと実機再検証は次回。

**関連ファイル:** `crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs`
（`per_vk_recovery_params` シグネチャ変更、`LiteralDetectCore::poll` の
`StaleConfirm` 分岐）、`crates/awase-windows/src/tsf/warmup/probe_fsm.rs`
（`run_per_vk_confirm`・`tsf_probe_coro_body` 両方の `StaleConfirm`/
`SuspectedLiteral` ハンドラ）。関連: 本エントリ追補1・2・3、BUG-29/BUG-30
（per-VK confirm の suspected-literal 誤判定の既知の限界）、
[ADR-079](adr/079-epoch-fenced-literal-recovery-with-replay.md)。

---

## BUG-36: `RawTsfLiteralRecovery` give-up が Chrome GJI reinit を backspace flush より先に送り、未確定 preedit が commit されて literal 文字が残る

**症状:** Chrome（`Chrome_WidgetWin_1`、`Imm32Unavailable`、GJI）で「たみや」と
連続入力したところ、「た」が literal 化して **"t" だけが残り「tみや」になった**
（2026-07-23 実機ログ、`verify/integrate-unmerged-branches` ブランチでの実機
ソーク中に発見）。BUG-35（ADR-079 Stage 1 epoch fencing）と BUG-33（give-up 時の
Chrome GJI reinit）を統合した直後に再現。

**IME:** Google 日本語入力（GJI）。`Imm32Unavailable`（Chrome/WezTerm/Windows
Terminal 等）。

**再現ログ（要約、`RUST_LOG=debug`）:**
```
[h1-probe] cold=3 ... per-VK confirm へ  ("ta" の VK0='T' 送信)
[tsf-probe] cold=3 per-VK[0/1] candidate window already visible だが
  直近の GJI I/O が送信時刻より前 → stale confirm として扱う (vk=0x54)
[raw-tsf-literal] cold=3 raw TSF literal suspected → backspace ×1 + re-送 "ta"
[raw-tsf-literal] flush escape=false backspace ×1     ← backspace 実送信
[output] re-sending raw TSF literal romaji="ta"        ← "ta" 再送 (cold=4)
[tsf-probe] cold=4 per-VK[0/1] candidate window already visible だが
  直近の GJI I/O が送信時刻より前 → stale confirm として扱う (vk=0x54)  ← 再度同じ VK0 で発火
[raw-tsf-literal] cold=4 consecutive raw-tsf-literal (count=2) → giving up,
  backs=1 cleanup only (no re-send)
[chrome-reinit] cold=4 VK_IME_OFF→VK_IME_ON 強制リセット送信 ...  ← reinit が先に実送信
[hook] IME-mode vk=0x1A down/up (VK_IME_OFF) ... vk=0x16 down/up (VK_IME_ON)
[composition] marked cold reason=RawTsfLiteralRecovery consecutive=2
[raw-tsf-literal] flush escape=false backspace ×1     ← backspace はこの後
```

**原因（確定、コード読解で確認）:** `probe_io.rs` の `RawTsfLiteralRecovery`
ハンドラの give-up 分岐（`consecutive > 0`）は、同じ関数内で

1. `io.set_raw_literal(backs, "", escape_composition)` — backspace 数をグローバル
   （`RAW_TSF_LITERAL`）に**予約するだけ**（実送信は `WM_DRAIN_OUTPUT_QUEUE` →
   `flush_raw_tsf_literal_recovery` → `flush_raw_tsf_literal_backspaces` まで遅延）。
2. `io.send_chrome_gji_reinit_and_poll(cold_seq)` — `VK_IME_OFF`→`VK_IME_ON` を
   **その場で即座に** `SendInput` する（BUG-33 の実装）。

を順に呼んでいた。この2つの間に実行タイミングの前後関係の保証がなく、実際には
2 が 1 より先に OS へ届く。`VK_IME_OFF` は Windows IME の一般的挙動として
**未確定の composition（preedit）を commit してから IME を閉じる**ため、
cold=4 の VK0 送信で開始していた preedit "t" がここで実文字としてドキュメントに
確定してしまう。その後にようやく届く backspace ×1 は、IME が OFF→ON を経て
状態が変わった後の非同期タイミングで発火するため、確定済みの "t" を確実に
消せない（本リポジトリで繰り返し出てくる「reinit 直後の非同期レース」と同型）。

**修正:** give-up 分岐から `send_chrome_gji_reinit_and_poll` の直接呼び出しを
廃止し、新設の `ProbeIo::schedule_chrome_gji_reinit` で `Output::pending_gji_reinit_cold_seq`
に予約するだけに変更した。実際の reinit 送信は `Output::flush_raw_tsf_literal_recovery`
（`flush_raw_tsf_literal_backspaces` の直後、`WM_DRAIN_OUTPUT_QUEUE` ハンドラ内）
に移動し、**backspace が実際に送信された後で** reinit を送るよう順序を保証した。
`send_chrome_gji_reinit_and_poll` 自体は変更していない（`Output::send_f22_f21_reinit`
という別の呼び出し元があり、そちらは backspace flush と無関係な Unicode-mode
long-cold 経路のため、即時実行のままで問題ない）。

**テスト:** `output/probe_io.rs::tests` の
`raw_tsf_literal_recovery_tsf_mode_consecutive_gives_up_with_cold_mark` を、
give-up 時に `send_chrome_gji_reinit_and_poll`（即時実行）が**呼ばれないこと**、
代わりに `schedule_chrome_gji_reinit`（予約）が1回呼ばれることを検証するよう
更新（`FakeProbeIo` に `gji_reinit_scheduled_count` を追加）。
`raw_tsf_literal_recovery_sets_literal_and_marks_cold_when_first_time` にも
初回（consecutive==0）では予約すら行わないことのアサーションを追加。
`cargo check`/`cargo clippy -p awase-windows --target x86_64-pc-windows-gnu -- -D
warnings` クリーン、`cargo test -p awase-windows --target x86_64-pc-windows-gnu
--no-run` でテストバイナリのコンパイル確認済み。wine 未導入のためこのサンドボックス
では実行不可（`#[cfg(windows)]` 系と同じ制約）。実機再検証は次回ソークで実施。

**関連ファイル:** `crates/awase-windows/src/output/probe_io.rs`
（`ProbeIo::schedule_chrome_gji_reinit` 新設、give-up ハンドラの呼び出し先変更）、
`crates/awase-windows/src/output/mod.rs`（`Output::pending_gji_reinit_cold_seq`
フィールド新設、`flush_raw_tsf_literal_recovery` での消化）。関連: BUG-33
（give-up 時の Chrome GJI reinit 自体の導入）、
[ADR-079](adr/079-epoch-fenced-literal-recovery-with-replay.md)/BUG-35
（stale confirm epoch fencing。本バグの引き金となった1文字目 stale confirm の
連発自体はここでは未解決 — なぜ VK0 が2回とも stale と判定されたかは別途調査
の余地がある）。

---

## BUG-37: Ctrl+T 等の同一プロセス内フォーカス移動で IME belief が実状態と乖離しても、唯一の訂正手段（物理 IME キー）が no-op に握り潰される

**症状:** BUG-36 と同一の実機ログ（2026-07-23、Chrome + GJI、`Imm32Unavailable`）の
さらに手前で観測。「た」が literal 化するより前に、ユーザーは Ctrl+T で新規タブを
開いた直後、違和感を覚えて物理サムキーで IME ON を明示的に押していた。しかし:

```
[engine-input] vk=0x54 KeyDown ... gas_ctrl=true phys_ctrl=true   ← Ctrl+T（新規タブ）
[focus-sync] hwnd=0xE8D19D0 class="Chrome_WidgetWin_1" ... → mode=Vk
[tsf-gate] focus change → PendingWarmup (held cleared)
...
[hook] IME-mode vk=0xF2 down self_injected=false injected=false ...   ← 物理キー（自己注入ではない）
[shadow-toggle] no-op: vk=0xF2 action=TurnOn source=PhysicalImeKey
  effective_open は既に true → apply-ime 見送り                        ← 何も送信されなかった
```

ユーザーの明示的な訂正操作が完全に無視され、直後の "た" 入力が literal 化した
（BUG-36 参照。BUG-36 の修正だけでは症状の一部しか直らず、根本原因はこちら）。

**IME:** Google 日本語入力（GJI）。`Imm32Unavailable`（Chrome/Edge 等）および
実質 TSF ネイティブ（WezTerm/Windows Terminal 等）。

**原因（確定、コード読解で確認）:**

1. `kp_stage_shadow_ime_toggle`（`runtime/key_pipeline.rs`）の no-op チェックは
   **無条件**: `effective_open() == 要求値` なら `apply_ime_open` に到達せず、
   実 OS へは何も送信しない。`Imm32Unavailable` 向けの特別扱いは存在しない。
   コード自身のコメントが「belief が既に一致しているため実 IME が別経路で
   乖離していても訂正されない」と既知のギャップとして明記していた。
2. Ctrl+T（同一プロセス内のタブ切替）は `EVENT_OBJECT_FOCUS`（アクセシビリティ
   focus イベント、`app/bootstrap.rs::win_event_proc` → `on_window_focus_event`）
   だけを発火させる。このイベントは `injection_mode`／`TsfGate` のみ更新し、
   `desired_open`／`effective_open`／`observations` などの belief には一切触れない
   （`output/mod.rs:508-511` のコメントは Ctrl+T をこの軽量イベントの発生源として
   既に想定していたが、belief 面の対応はしていなかった）。
3. belief を実際に再検証する唯一の経路（`ImeEvent::FocusChanged`、
   `applied` を `Unknown` にリセットし `apply_force_on_for_imm_broken` を
   解禁する）は `process_changed`（フォーカス元と先で PID が異なる）でしか
   発火しない（`focus_tracking.rs::advance_focus_tracking`）。Ctrl+T の新規タブは
   同一 Chrome プロセス内のため、この経路が発火しない。
4. 仮に発火したとしても、`focus_tracking.rs:341-369`（"Imm32Unavailable hard
   pre-sync"）が `effective_open()==true` のとき `mirror_applied_open(true, ...)`
   で `applied` を即座に再ロックしてしまい、`apply_force_on_for_imm_broken` の
   訂正チャンスを消してしまう。

結果として `Imm32Unavailable`／実質 TSF ネイティブなアプリでは、belief が
一度でも実状態と乖離すると、(a) 唯一の訂正チャネルである物理 IME キー押下は
no-op で握り潰され、(b) 唯一のbelief再検証経路は同一プロセス内フォーカス移動
では発火せず、(c) 発火してもすぐ再ロックされる、という三重に訂正されない状態
になる。`BUG-33`（drift correction が構造的に発火し得ない）と同根の
「observe できないプロファイルは自己確認はできても自己訂正できない」問題の
別の顔。

**修正:** 真のフォーカス変更（`process_changed`）で使われている再プライム機構
（`Output::mark_composition_cold_focus_change` → 次の VK/TSF 送信で
`VK_DBE_HIRAGANA` warmup を先行送信）を、軽量な `on_window_focus_event` からも
条件付きで発火するようにした。`focus::class_names::should_reprime_on_lightweight_focus_sync`
（新設の純粋関数）が「`Imm32Unavailable` または実質 TSF ネイティブなプロファイル」
かつ「belief（`effective_open()`）が既に ON」の場合にのみ `true` を返し、
`on_window_focus_event` はこの条件のときだけ cold mark する。

- 新規コードパスは作らず、既存の cold-mark／eager-warmup 機構をそのまま再利用。
- cold mark 自体は「次に実際に VK/TSF を送信するまで何も OS に送らない」遅延
  フラグのため、Chrome が連続発火させる複数の `EVENT_OBJECT_FOCUS`
  （タブ・アドレスバー・コンテンツ等）で何度呼ばれてもレイテンシ以外の実害はない。
- belief=OFF のときは何もしない（不要な IME ON 化を起こさない）。
- 実状態を確実に問い合わせられる `Standard` プロファイルは対象外（この機構が
  不要な唯一のケース）。

**未解決（Stage 1 の限定的な修正）:** 本修正は「belief=ON のときに実状態を
belief へ追従させる」再プライムのみ。逆方向（belief=OFF なのに実状態が ON の
まま乖離）や、Ctrl+T 以外の同一プロセス内フォーカス移動（タブドラッグ、
複数ウィンドウ間のショートカット等）で同型の問題が起きるかは未検証。
[ADR-028](adr/028-focus-event-redesign.md)（承認済み・未実装）はより広い
「同一プロセス内フォーカス移動でも belief-invalidating しない re-fetch を行う」
設計を提案しており、本修正はその一部を Imm32Unavailable/TSF-native の
片方向ケースに限定して先行実装したものと位置づけられる。

**テスト:** `focus/class_names.rs::tests` に
`should_reprime_on_lightweight_focus_sync` の回帰テスト5件を追加
（Chrome belief=ON/OFF、Windows Terminal、WezTerm、Standard の各ケース）。
純粋関数のため Linux ネイティブで実行可能（`cargo test -p awase-windows --lib
class_names` で確認済み、140 passed）。`on_window_focus_event` 側の実際の
呼び出し配線は `#[cfg(windows)]` のため `cargo test -p awase-windows --target
x86_64-pc-windows-gnu --no-run` でコンパイルのみ確認。実機での Ctrl+T 再現
テストは次回ソークで実施すること。

**関連ファイル:** `crates/awase-windows/src/focus/class_names.rs`
（`cannot_verify_real_ime_state`/`should_reprime_on_lightweight_focus_sync` 新設）、
`crates/awase-windows/src/runtime/mod.rs`（`on_window_focus_event` に配線）。
関連: BUG-36（本バグが引き起こした literal 化の直接症状、別コミットで先行修正済み）、
BUG-33（Imm32Unavailable の drift correction 不発火、同根の問題）、
[ADR-028](adr/028-focus-event-redesign.md)（未実装、より広い設計）、
`.claude/rules/ime-belief-architecture.md`。

---

## BUG-38: `RawTsfLiteralRecovery` の give-up 分岐が `pending_deferred` を flush しないため、probe 実行中に届いた別の打鍵が消失・出力順逆転する

> 統合ブランチでの注記: 本エントリは `main` 側で「BUG-35」として書かれていたが、
> 本ブランチには既に別の BUG-35（per-VK confirm の stale confirm 誤帰属、
> ADR-079 Stage1）が存在していたため、統合時に BUG-38 へ改番した。

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` → `Windows.UI.Input.InputSite.WindowClass`、GJI、TsfNative）で高速タイピング中、「とうろくする」と入力したところ「と」が消え、続く「う」と「ろ」の出力順が入れ替わった（2026-07-22 実機ログ）。ユーザー報告は「なぞの VK_BACK が再発しています」。

**再現手順（ログで確認済み）:**

```
と(to) の2文字目 VK が stale confirm 検出 → RawTsfLiteralRecovery(consecutive=0):
  backspace ×1 + 再送 "to" を予約、mark cold
  （↑ この間に実キー "う" が到着 → probe in-flight のため pending_deferred=[う] に退避）
再送した "to"（新 probe, cold+1）の1文字目 VK も stale confirm 検出
  → RawTsfLiteralRecovery(consecutive=1, give-up): backspace のみ・再送なし
     + Chrome reinit (VK_IME_OFF→VK_IME_ON) 予約
flush_raw_tsf_literal_recovery: backspace → (romaji空なので再送なし) → reinit
  → has_pending_tsf()==false になるが pending_deferred=[う] は誰も flush しない
実キー "ろ"(ro) が到着 → probe in-flight ではないため deferred されず、
  通常の新規 probe を経て即座に送信される（TransmitTsf 分岐で自身の
  送信直後に pending_deferred=[う] を flush）
→ 出力順が "ろ","う" になり、"と" は2回の backspace で消えたまま戻らない
```

**IME:** GJI（Google 日本語入力）。TsfNative プロファイル（Windows Terminal 等）。stale confirm 検出自体は本統合ブランチが取り込んだ epoch-fencing 機能（BUG-35、ADR-079 Stage1）によるものだが、本バグの根本原因（`pending_deferred` を flush し忘れる欠落）はその機能とは独立に `main` に既に存在する。stale confirm はこの欠落を踏みやすくする誘因にすぎない。

**原因（確定、コード読解で確認）:** `dispatch_probe_actions`（`output/probe_io.rs`）の `TransmitTsf`/`TransmitChrome`/`TransmitSingleVk`（`is_last`）の3ハンドラは、いずれも自分の送信直後に `io.send_deferred_vks(&io.take_pending_deferred_vks(), marker)` を呼んで `pending_deferred`（`TsfWarmupCoordinator` 所有、probe 実行中に届いた後続キーの退避キュー）を flush する。しかし `ProbeAction::RawTsfLiteralRecovery` のハンドラだけはこれを一切呼ばない。特に `consecutive > 0`（give-up、romaji 再送なし）の場合、この probe の完了後に新しい probe が張られる保証がないため、`pending_deferred` は誰にも flush されないまま取り残される。次に届いた**全く別の**打鍵が（`has_pending_tsf()==false` になっているため）deferred されずに先に通常送信され、後から取り残された分が flush されるため、出力順が入れ替わる。

**修正:** `Output::flush_raw_tsf_literal_recovery`（`output/mod.rs`、backspace → romaji 再送の実送信が起きる同期ポイント）の最後に `flush_stale_deferred_vks_after_recovery` を追加した。`TsfWarmupCoordinator::take_pending_deferred_if_probe_idle()`（新設）が `has_pending_tsf()` を見て、romaji 再送が新しい probe を張っていれば `None`（その新 probe 自身の既存3ハンドラに flush を委ねる）、probe が本当に終わっていれば取り残された VK を返す。`dispatch_probe_actions` の中（`set_raw_literal` が static に退避するだけで実送信はまだ起きていない時点）で flush すると backspace より先に deferred VK が OS に届いてしまうため、意図的に「実際に SendInput される同期ポイント」である `flush_raw_tsf_literal_recovery` の末尾に置いた。

**未対応（残存、フォローアップ候補）:** `escape_composition=true`（ESC で composition を丸ごと破棄する経路、per-VK confirm の2文字目以降で到達しうる、`main` に既存）の場合、ESC 直後の raw な deferred VK 送信は probe を経由しないため、それ自体が literal 化するリスクが理論上残る。probe を経由した re-entry（romaji へ変換して `send_romaji_as_tsf` 相当に載せる）は ADR-079 Stage2（未実装）のスコープであり、本 fix は「取り残されたまま出力順が入れ替わる」実害の解消に限定した。

**テスト:** `output/tsf_warmup_coord.rs` に `take_pending_deferred_if_probe_idle` の回帰テスト3件を追加（probe in-flight 中は drain しないこと／give-up で probe が終わった後は drain すること／キューが空なら `None` を返すこと）。実際の `Output::flush_stale_deferred_vks_after_recovery` は `SendInput` を伴うため（本クレートの既存の慣習どおり、Win32 呼び出しを伴うコードパスは `ProbeIo`/`FakeProbeIo` 経由か、決定ロジックを純粋関数に分離した上でユニットテストする）、コーディネーターレベルのテストで決定ロジックを直接検証する形にした。Windows cross-compile（`cargo xwin clippy --target x86_64-pc-windows-gnu -- -D warnings` 含む）警告ゼロ確認済み。Wine 等の実行環境がないためテストの実実行（`cargo test --target x86_64-pc-windows-gnu`）は未実施、実機再検証も未実施。

**関連ファイル:** `crates/awase-windows/src/output/mod.rs`（`flush_raw_tsf_literal_recovery`、新設 `flush_stale_deferred_vks_after_recovery`）、`crates/awase-windows/src/output/tsf_warmup_coord.rs`（新設 `take_pending_deferred_if_probe_idle`）、`crates/awase-windows/src/output/probe_io.rs`（`ProbeAction::RawTsfLiteralRecovery` ハンドラ）。関連: BUG-28（同じ `flush_raw_tsf_literal_recovery` 経路の別の drain 漏れ）、BUG-35（stale confirm 誤帰属、ADR-079 Stage1、本バグの誘因）、BUG-36（give-up→Chrome reinit の順序修正）。

---

## BUG-39: `literal_session_confirmed` が FocusChange・長時間 idle・アプリ切替をまたいで持ち越され、新しい cold セッションの先頭文字が literal で漏れても reactive literal-detect が発動しない

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` → `Windows.UI.Input.InputSite.WindowClass`、GJI、TsfNative）で、Chrome で約88秒アイドルしたあと Windows Terminal にフォーカスを移し、物理 F2（IME ON）を押してから「こっか」と入力したところ、1文字目だけ literal ローマ字が漏れて **"koっか"** になった（2026-07-23 実機ログ）。

**再現手順（ログで確認済み、`.claude/rules/experiment-logging.md` 準拠の実測）:**

```
Chrome で ~88s idle（GJI I/O 静止、候補ウィンドウの SHOW/HIDE は一度も観測されない）
  → Windows Terminal へ FocusChange、TsfNative と判定、cold mark（reason=FocusChange）
物理 VK_DBE_HIRAGANA(F2) 押下（self_injected=false）
  → NativeF2Consumed、reason=NativeF2Consumed で追加 cold mark
「こ」romaji="ko" 送信:
  cold_warmup: reason=NativeF2Consumed → F2/probe 待機省略、per-VK confirm へ
  gji-coro: settle 必要（settled=false）→ transmit-plan needs_literal=true
  literal_detect_fsm: 「セッション確認済み → スキップ」 ← ここで reactive 検出が丸ごと無効化
  → "ko" が変換されず literal のまま出力、誰も訂正しない
「っ」「か」以降は正常に「っか」として変換される（新しい cold=301 での probe/warmup 自体は機能していた）
```

**IME:** Google 日本語入力（GJI）。TsfNative プロファイル（Windows Terminal 等）。Chrome の `Imm32Unavailable` でも同じ構造的欠陥のため理論上再現しうる（未確認）。

**原因（確定、コード読解で確認）:** `literal_session_confirmed`（`tsf/observer.rs`）は「同一 IME セッション内では以降の文字の literal-detect をスキップする」BUG-24 由来の最適化フラグで、`mark_literal_session_confirmed()` で `true` になり、本番で唯一 `false` に戻すのは `Output::gji_on_end_composition`（`platform.rs`、候補ウィンドウ HIDE 時）だけだった。しかもこの呼び出しは `gji_current_composition_epoch()` が `Some`（= `GjiFsm` が `OnComposing` のまま）の場合にしかガードを通らない。ところが `GjiFsm` は `FocusChange`/`NativeF2Consumed` 等で `OnComposing` から容易に抜ける（`gji_idle_ms` observation ゲート込みで cold へ倒れる、BUG-33 追補3）。候補ウィンドウの HIDE イベント（`observation_event_proc` → `pending_end_composition`）がドレインされる時点で `GjiFsm` が既に `OnComposing` を抜けていれば `gji_current_composition_epoch()` は `None` を返し、**reset がそのまま握り潰される**。結果、`literal_session_confirmed` は「一度確認できたセッション」の生存期間（候補ウィンドウ HIDE まで）ではなく、**次にたまたま epoch 付きで HIDE がドレインされるまで**という、フォーカス変更・アプリ切替・数十秒〜数分の idle をまたいでも解除されない期間だけ true であり続ける。この状態で新しい cold セッション（別アプリ、別 TSF context）の1文字目が literal 化しても、`gji_warmup_coro`/`LiteralDetectCore` は「セッション確認済み」を理由に検出処理自体をスキップし、誰も backspace+再送で訂正しない。

`.claude/rules/ime-belief-architecture.md` の 2026-07-23 追記（`GjiFsm` の `CompositionReset`/`NativeF2Consumed` が弱い代理指標だけで無条件に belief を書き換えていた、という同種の教訓）と同じ形の欠陥: `literal_session_confirmed` は「蓄積する」値なのに、唯一のリセット経路が **単一の呼び出し口の実行成否**（epoch が取れるか）に依存しており、その呼び出しが握り潰された場合の代替経路が無かった。

**修正（初期案から設計変更）:** 当初は `mark_composition_cold` に `reason.requires_settle()` 分岐でリセットを追加する対症的パッチを検討したが、これは「新しい無条件書き込み条件分岐を1つ増やす」だけで、`.claude/rules/ime-belief-architecture.md` が戒める「観測を無視した蓄積状態の場当たり的な保護」の再演になる（レビュー指摘）。代わりに **`literal_session_confirmed` 自体を「確認したかどうかの真偽値」から「どの `cold_seq`（`WarmEpoch::cold_start_count`、実際に新しい warmup/probe が走るたびに増える世代カウンタ）で確認したか」を保持する値に変更**し、判定を「記録した世代 == 現在の世代」という比較に置き換えた（observation → belief の経路を太らせる形）。具体的には:

- `TsfObservations::literal_session_confirmed: AtomicBool` → `literal_session_confirmed_gen: AtomicU32`（`0` = 未確認の番人値）に変更（`tsf/observer.rs`）。
- `literal_session_confirmed(current_cold_seq: u32) -> bool` / `mark_literal_session_confirmed(cold_seq: u32)` と、確認対象の世代を明示的に引数化した。
- `mark_literal_session_confirmed` の実際の呼び出し元（`ProbeAction::CompositionConfirmed`、`tsf/warmup/probe_fsm.rs`/`literal_detect_fsm.rs`）は元々 `cold_seq` をローカルスコープに持っていたため、`ProbeAction::CompositionConfirmed` に `cold_seq: u32` フィールドを追加して伝搬させるだけで済んだ（`output/probe_io.rs` のディスパッチャがそのまま `mark_literal_session_confirmed(cold_seq)` を呼ぶ）。
- `reset_literal_session_confirmed()`（`gji_on_end_composition`、候補ウィンドウ HIDE 時）は残した。世代比較が正しさの唯一の拠り所になったため、この呼び出しが `gji_current_composition_epoch()==None` で握り潰されても実害はない（次の cold-start で `cold_seq` が進めば自動的に stale になる）。この明示リセットは「同一世代内でも次の1語は律儀に再確認させる」という BUG-24 の保守的な最適化オプトアウトとして意味があるため削除しなかった。

この設計により、`mark_composition_cold`/`ColdReason` 側には一切変更が不要になった（当初案の `requires_settle()` 分岐は追加していない）。「FocusChange 等どの reason で cold になったら reset すべきか」を個別に列挙する近道を取らず、`cold_seq` という既存の実観測値（BUG-33 追補3 で `GjiFsm` にも使われている考え方と同型）に判定を委ねることで、新しい cold-start 契機が将来追加されても automatically 正しく扱われる。

**テスト:** `tsf/observer.rs` に `#[cfg(test)]` モジュールを新設し回帰テスト4件を追加: `unconfirmed_state_is_never_confirmed`、`same_generation_query_is_confirmed`、`new_cold_generation_invalidates_prior_confirmation_without_explicit_reset`（本バグの核心 — `reset_literal_session_confirmed()` を挟まずに `cold_seq` が進むだけで前世代の確認が自動的に無効化されることを確認）、`explicit_reset_invalidates_same_generation_confirmation`（HIDE 経由の明示リセットが引き続き機能することを確認）。Windows cross-compile（`cargo check`/`cargo check --tests`/`cargo clippy --lib -- -D warnings`、`x86_64-pc-windows-gnu`）警告ゼロ確認済み。Wine 等の実行環境がないため `cargo test --target x86_64-pc-windows-gnu` の実実行および実機再検証は未実施（BUG-38 と同様の制約）。

**関連ファイル:** `crates/awase-windows/src/tsf/observer.rs`（`literal_session_confirmed_gen`/`literal_session_confirmed`/`mark_literal_session_confirmed`/`reset_literal_session_confirmed`）、`crates/awase-windows/src/tsf/warmup/probe_fsm.rs`（`ProbeAction::CompositionConfirmed` に `cold_seq` フィールド追加、`run_per_vk_confirm`／Chrome コルーチンの確認呼び出し）、`crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs`／`literal_detect_fsm.rs`（`literal_session_confirmed(cold_seq)` 呼び出し）、`crates/awase-windows/src/output/probe_io.rs`（`CompositionConfirmed` ディスパッチャ）、`crates/awase-windows/src/platform.rs`（`gji_on_end_composition`、明示リセット経路）。関連: BUG-24（`literal_session_confirmed` 導入元）、BUG-33 追補3（`GjiFsm` が `gji_idle_ms` 実観測値をイベントの必須パラメータ化した同型の設計判断）、`.claude/rules/ime-belief-architecture.md`（「蓄積する belief 的状態は実観測値との比較で自己検証させる」という判断基準）。

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
| `stale confirm 検出` / `epoch-fence-stale` | ADR-079/BUG-35: confirm 根拠が前世代由来と判明（追補1で SuspectedLiteral と同じ backspace+再送に変更済み。追補2で「既に可視」ショートカットの猶予漏れによる false positive も修正済み。追補3で `CompositionReset`/`NativeF2Consumed` 自体に `gji_idle_ms` observation ゲートを追加し前提条件面を根治。追補4で backspace 自体を送らない（romaji 再送のみ）方式に変更、literal の positive な証拠がない限り BS を送らない） |
| `[literal-detect] cold=N セッション確認済み → スキップ` | BUG-39: `literal_session_confirmed(N)==true`（確認済み世代が現在の `cold_seq=N` と一致）のため reactive literal-detect 自体をスキップ。修正後は世代不一致で自動的に無効化されるため、このログの `cold=N` は必ず「実際にその N で確認が取れた」世代のはず — もしこの N で一度も `[literal-detect] cold=N composition confirmed` 相当のログが無いのに出ていたら回帰を疑う |

---

## BUG-40: `nc_for_plan` が `gji_settled`（GJI probe の実測結果）を見ずに confirm-key ヒントだけで `nc_fired` を昇格し、genuinely cold なセッションまで reactive literal-detect を丸ごとスキップしていた

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS`、GJI、TsfNative）で、約88秒 GJI I/O が静止した状態から物理 F キー（単独、修飾キーなし）を押したところ、変換されないローマ字 **"ke"** が literal のまま出力された（本来は「け」に変換されるか、少なくとも変換前提の romaji 送信自体が起きないかのどちらかであるべき。2026-07-23 実機ログ）。BUG-39（`06ad210`）修正を取り込んだビルド（未コミット差分なしを `git status`/`git diff` で確認済み）でも再現した — BUG-39 とは別経路の欠陥。

**再現手順（ログで確認済み）:**

```
Ctrl 押下中に複数ショートカットキーを連打（Ctrl+B/H/P/Shift+V 等）→ 通常の passthrough
  Ctrl KeyUp → composition-fsm が EmitWarmup(CtrlUp) → 実 VK_DBE_HIRAGANA(F2) 送信
物理 F キー押下（修飾キーなし）→ 100ms バッファタイマー
  送信予定: Char('け')（NICOLA レイアウト上の F キーのマッピング）
  → send_char_as_tsf: 'け' → romaji "ke"
  cold_warmup: reason=ReinjectConfirmKey → F2/probe 待機省略、per-VK confirm へ
  gji-coro: GjiProbe 完了（16ms, gji_idle=88063ms, settled=false）
    → 「settle 必要 → skip FreshF2, reactive LiteralDetect のみ」とログ（nc_fired=false のはず）
  transmit-plan: needs_literal=false nc_fired=true ← settled=false のはずが nc_fired が true に化けている
  → per-VK confirm も inline LiteralDetect も構造的にスキップされ、
    診断専用の fire-and-forget "skip-verify" タスクだけが実行される（副作用なし）
  → "ke" が変換されず literal のまま出力、誰も訂正しない
```

**IME:** Google 日本語入力（GJI）。TsfNative プロファイル（Windows Terminal 等）。

**原因（確定、コード読解 + 標準入力での純関数再現で確認）:** `gji_warmup_coro.rs`（`gji_coro_body`）は Phase 1 の GJI probe で `outcome.settled=false` を正しく検出し、「reactive LiteralDetect のみに委ねる」と判断して `nc_fired=false` を確定させていた。しかし Phase 4（transmit plan 決定）の直後の行で:

```rust
let nc_for_plan = nc_fired || (cold_reason.is_confirm_key() && env.is_tsf_mode);
```

という上書きが `outcome.settled` を一切参照せずに `nc_fired` を `true` に昇格させていた。この上書きは元々 `3ffbe66`（2026-06-27, `Enter後 TSF mode での LiteralDetect 誤検出を抑制`）で導入されたもので、WezTerm で Enter/Space 後に NameChange イベントが発火しないが GJI 自体は正常に合成中というケースを救済するためのものだった。**導入当初は `is_confirm_key() && is_tsf_mode && !gji_resumed` という3項条件**で、`gji_resumed`（F2×2 待機行列が GJI からの I/O 応答を確認できたか）が実質的なゲートだった。ところが `629db3b`（2026-07-19）で `gji_resumed` の唯一の生成元（F2×2 待機行列）が `d495649` で物理削除され dead code 化したのを受け、`gji_resumed` 項自体が削除された（cleanup としては正しい）。**このとき、上書きの本来の意図だった「GJI が実際に応答/再開しているか」という実測ゲートが失われたまま、`is_confirm_key() && is_tsf_mode` の2項だけが残った。** 結果、88秒 idle 後の genuinely cold なセッション（`outcome.settled=false`）でも confirm-key 系 cold_reason かつ TSF mode でありさえすれば無条件に救済され、`decide_transmit_plan`（`probe_fsm.rs`）の `needs_literal = !obs.nc_fired && env.is_tsf_mode && env.gji_active` が `false` になり、per-VK confirm・inline `LiteralDetectCore`（Phase 6）の両方が構造的にスキップされた。

`.claude/rules/ime-belief-architecture.md` の 2026-07-23 追記（`GjiFsm` が弱い代理指標だけで無条件に belief を書き換えていた、という同種の教訓）と同じ形の欠陥: `gji_resumed` という実観測値が dead code 削除のタイミングで代理指標無しの盲目的な条件に縮退し、それに気づく機構（型・テスト）が無かった。

**修正:** `nc_for_plan` という `nc_fired` を上書きする1変数に集約する設計をやめ、`ProbeObservations` に生の観測値を追加した:

- `ProbeObservations.nc_fired`: 生の NameChange 発火シグナル（呼び出し元での上書きを禁止、`is_partial_literal` 等が生値として参照するため）。
- `ProbeObservations.gji_settled`: `GjiProbeOutcome.settled`（実測）。
- `ProbeObservations.confirm_key_tsf_hint`: `cold_reason.is_confirm_key() && env.is_tsf_mode`（`3ffbe66` の救済ヒント、`cold_reason` にアクセスできる `gji_warmup_coro.rs` 側でのみ算出可能なため呼び出し元で計算）。

`decide_transmit_plan`（純関数、単独テスト可能）内で `nc_confirmed = obs.nc_fired || (obs.confirm_key_tsf_hint && obs.gji_settled)` として合成し、`used_eager_path`/`needs_literal` の判定はすべて `nc_confirmed` を使う。`gji_settled=true`（GJI が実際に I/O を返している = `3ffbe66` が想定した WezTerm 合成中シナリオ）の場合のみ救済が効き、`gji_settled=false`（本バグの実トレース相当）では救済されず reactive LiteralDetect が機能する。

**未検証のリスク（実機確認が必要、本コミットのスコープ外）:** `3ffbe66` が対象とした WezTerm の「Enter/Space 後に NameChange が発火しないが GJI は合成中」シナリオで `outcome.settled` が実際に `true` になるかは静的解析だけでは確定できない。もし WezTerm 側でも GJI がこの時点で新規 I/O を出さず quiescent（`settled=false`）であれば、本修正は `3ffbe66` の元バグ（Enter 後の先頭文字「な」消失）を再発させる。実機での WezTerm 再現テスト（`gji_warmup_coro.rs:90-96` の `settled=` ログを確認）が必要（Opus によるセカンドオピニオンレビュー、2026-07-23）。ユーザー判断により実機データ取得前に修正を先行実装した（再現困難な bug のため）。

**テスト:** `probe_fsm.rs` の `decide_transmit_plan` 回帰テストに3件追加: `decide_plan_confirm_key_hint_without_settled_keeps_literal`（BUG-40 の核心 — `confirm_key_tsf_hint=true` でも `gji_settled=false` なら救済せず `needs_literal=true` のままであることを確認）、`decide_plan_confirm_key_hint_with_settled_suppresses_literal`（`3ffbe66` の元シナリオ — `gji_settled=true` なら救済され `needs_literal=false` になることを確認）、`decide_plan_confirm_key_hint_without_tsf_mode_has_no_effect`。純関数のため Linux 上で `rustc` 単体抽出により実行結果を事前検証済み（wine 未導入のため `cargo test --target x86_64-pc-windows-gnu` の実実行は BUG-38/BUG-39 と同様に未実施）。`cargo test --target x86_64-pc-windows-gnu --no-run -p awase-windows`（`-D warnings`）・`cargo clippy --target x86_64-pc-windows-gnu -p awase-windows -- -A clippy::cargo_common_metadata -D warnings -W clippy::cognitive_complexity` は警告ゼロ確認済み。

**副次的発見（本コミットでは未修正、別途フォローアップ要）:** 同じ `probe_fsm.rs` の既存テスト `decide_plan_nc_fired_enables_literal_when_gji_active`（`426a7f2`, 2026-07-19 `DIAG_DISABLE_PROACTIVE_TSF_WARMUP` 恒久化リファクタで追加）は、`nc_fired=true` を入力に `plan.needs_literal` が `true` になることを期待しているが、同じ形式で `rustc` 単体抽出して確認した限り現在の `needs_literal` 式では `nc_fired=true` のとき常に `false` になり、このテストはアサーション失敗するはずである。同リファクタで `needs_literal` の第1節（`should_prepend_f2` 由来、削除済み）だけがこのテストの成立根拠だったが、削除時にアサーションが更新されなかった可能性が高い。Windows 実機/CI での実行結果が未確認のため確定はできないが、次にこのファイルに触れるセッションで要確認。

**関連ファイル:** `crates/awase-windows/src/tsf/warmup/probe_fsm.rs`（`ProbeObservations`/`decide_transmit_plan`）、`crates/awase-windows/src/tsf/warmup/gji_warmup_coro.rs`（`gji_coro_body` Phase 1/4、`nc_for_plan` 撤去）、`crates/awase-windows/src/output/vk_send.rs`／`crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs`（`ProbeObservations` 構築箇所の追随）。関連: BUG-39（同じ `gji_warmup_coro.rs` の別経路）、`3ffbe66`（`confirm_key_tsf_hint` 相当ロジックの導入元）、`629db3b`（`gji_resumed` 削除、本バグの直接の引き金）、`.claude/rules/ime-belief-architecture.md`（「蓄積しない・毎回純粋関数で再計算される値は独立した実観測パラメータとして渡す」という判断基準）。

## BUG-41: `decide_alt_impersonation` が KeyUp 時点で「なりすまし発動中」フラグを stuck true のまま持ち越し、後続の無関係な Alt 押下まで modifier 誤補正の対象にしていた

**発覚経緯:** 2026-07-25、`crates/awase-windows` の `cargo test --lib` が GCP Spot self-hosted runner上のWindows実機で初めて実際に実行された（従来はWine未導入によりLinux上での実行・Windows実機での動作確認とも未実施で、`--no-run`のクロスコンパイルチェックのみだった）。この初回実行で`hook::alt_impersonation_tests::keyup_uses_the_decision_recorded_at_keydown`が実機上で初めて失敗し、本バグが発覚した。

**症状（テストで再現）:** Left Alt を親指キーとしてなりすまし設定中、`decide_alt_impersonation`にKeyDown→KeyUpの順で入力すると、KeyUp後も戻り値の`is_impersonating`が`true`のままだった（`assert!(!impersonating_after_up, ...)`が失敗）。この戻り値は`ALT_L_IMPERSONATING`/`ALT_R_IMPERSONATING`（`hook.rs`）に格納され、`is_alt_impersonation_active()`経由で3箇所（`hook.rs`・`runtime/mod.rs`・`runtime/message_handlers.rs`）が`modifiers.alt`を強制falseに補正する判断に使われる。KeyUp後もこのフラグがstuck trueのまま残ると、次に(なりすまし設定の無い)Right Altを押す、あるいはAlt+Tab等を行った際にも`modifiers.alt`が誤ってfalse補正され、`cec4da9`が修正したのと同種のbypass誤爆が再発しうる。

**原因:** `decide_alt_impersonation`が「今回のvk翻訳に使う判定」と「以後保持すべき状態」を同じ1つの値(`impersonating`)で兼用しており、`is_keydown=false`(KeyUp)の場合も無条件に`was_impersonating`をそのまま持ち越していた。KeyUpの瞬間は物理キーが既に離れているため、以後保持する状態は必ずfalseに戻すべきだった。

**修正:** 戻り値の2要素目(以後保持する状態)を`is_keydown`で分岐させ、KeyUpの場合は常に`false`を返すようにした。vk翻訳(1要素目)は従来通り`currently_impersonating`(直前の判定)を使い、KeyDown/KeyUpの対称性は維持している。

**テスト:** 既存の`hook::alt_impersonation_tests::keyup_uses_the_decision_recorded_at_keydown`がそのまま回帰テストになる(新規追加ではなく、既存テストが正しく通るようになった)。`cargo test --target x86_64-pc-windows-gnu --no-run -p awase-windows`（`-D warnings`）・`cargo clippy --target x86_64-pc-windows-gnu -p awase-windows`は警告ゼロ確認済み。**2026-07-25、GCP Spot self-hosted runner(`rust-nicola-builder`)上での実`cargo test --lib -p awase-windows`実行でパス確認済み**(`cargo mutants`のbaselineフェーズが`ok Unmutated baseline in 44s build + 4s test`で全テストパスと報告、GitHub Actions run 30098397721の`mutants-windows` job)。

**2026-08-01追記（ADR-082決定1実施記録の次の一歩、Linux実行可能化）:** 本バグが `#[cfg(windows)]` な `hook.rs` に埋もれたテストを Windows実機CI初回実行まで検出できなかったこと自体が再発の温床だったため、`decide_alt_impersonation`/`resolve_thumb_key`/`classify_alt_side` を `crates/awase-windows/src/state/alt_impersonation.rs`（ungated）へ移設し、既存11件のテストをそのまま Linux で実行可能にした（`cargo test -p awase-windows --lib` で常時実行）。加えて `decide_alt_impersonation` は `(is_keydown, was_down, was_impersonating, engine_enabled)` の bool 4個のみに依存する純粋関数で入力空間が2^4=16通りと有限なため、`decide_alt_impersonation_exhaustive_16_combinations`（網羅テーブル）・`keyup_always_clears_next_impersonating_regardless_of_prior_state`（BUG-41の不変条件そのもの）・`fresh_press_always_matches_engine_enabled` の3件を追加し、本バグの症状（KeyUp後のフラグstuck）が起き得ないことを全入力空間で固定化した。`hook::resolve_thumb_key` は既存呼び出し元（`app/bootstrap.rs`等）を変更せずに済むよう再エクスポートで維持。

**関連ファイル:** `crates/awase-windows/src/state/alt_impersonation.rs`（`decide_alt_impersonation`、旧`hook.rs`から移設）、`crates/awase-windows/src/hook.rs`（`ALT_L_IMPERSONATING`/`ALT_R_IMPERSONATING`、`apply_alt_impersonation`）。関連: `cec4da9`（同種のOsModifierHeldバイパス誤爆の初回修正）。

## 2026-07-25: Windows実機での`cargo test --lib -p awase-windows`初回実行で判明したテスト自体の不具合(実装バグではない)

GCP Spot self-hosted runner導入により、`cargo test --lib -p awase-windows`が実Windows上で初めて実行された（従来はLinux上でのクロスコンパイル`--no-run`チェックのみで、実行そのものは未実施だった）。BUG-41以外に、以下は**実装ではなくテスト自体の不具合**と判明したため、テスト側を修正した:

- `tsf::probe::tests::check_now_returns_stale_confirm_when_write_evidence_predates_epoch`、`tsf::warmup::probe_fsm::tests::chrome_per_vk_stale_confirm_from_leftover_candidate_window_recovers_like_suspected_literal`、`tsf::warmup::literal_detect_fsm::tests::poll_recovers_like_suspected_literal_when_stale_confirm_detected`: いずれも`std::thread::sleep(5ms)`+実`GetTickCount64`(既定解像度~15.6ms)で「epochより前」の時刻を作ろうとしていたが、tick解像度に対してマージンが無く、同一tickに丸まると`evidence_is_fresh`のtie判定(`>=`)が意図せずtrueになりflakyに失敗しうる設計だった。同ファイル内の他のテスト（`check_now_show_only_confirm_becomes_stale_after_grace_expires`等）が既に使っている`saturating_sub(50)`方式に統一し、実時間sleepへの依存を排除した。`EPOCH_FENCE_GRACE_MS`等の本番タイミング定数は変更していない。（`literal_detect_fsm.rs`側は最初の修正時に見落としており、下記ロック統一後の再検証で単独の真の失敗として顕在化し追加修正した。）
- `runtime::executor::tests::confident_when_confirmed_on_desired_on`: `now_ms=100_000`/`at_ms=500`(経過99,500ms)というテスト新設時点(`f7f09bc`, 2026-06-04)から既に300ms窓の外にある入力を使っていた。`chrome_intent_confident`の「Confirmed一致から300ms以内のみconfident」という設計(`7a24442`でOFF方向の永続スキップを廃止した際に確立)自体は正しく、テストの入力値を300ms以内(`at_ms=900`/`now_ms=1000`)に修正した。
- `tsf::warmup::literal_detect_fsm::tests::poll_recovers_like_suspected_literal_when_stale_confirm_detected`（および`poll_vetoes_backspace_while_candidate_visible`のPoisonErrorカスケード）: `TSF_OBS`（プロセス全体のグローバル状態）を保護するはずの`Mutex`が`observer.rs`/`probe.rs`/`literal_detect_fsm.rs`の3ファイルでそれぞれ**別々**の`static`として定義されており(`TEST_LOCK`×2、`VETO_TEST_LOCK`×1)、名前は同じでも異なる`Mutex`インスタンスのため互いに排他できていなかった。`cargo test`のデフォルト並列実行下で、あるファイルのテストが別ファイルのテストの`TSF_OBS`書き換えに巻き込まれ、`gji_last_write_ms`が意図せず0にリセットされる等で本来`StaleConfirm`になるはずの判定が`CompositionConfirmed`に化けていた。`observer.rs`に`TSF_OBS_TEST_LOCK`を1つだけ定義し、3ファイルとも`use ... as TEST_LOCK`でこれを共有するよう統一した。**この統一作業で`probe_fsm.rs`のテスト3件(`chrome_per_vk_*`)がそもそも一切ロックを持たずTSF_OBSを直接操作していた点を見落としており**、統一後の再検証で「probe.rsの他テストがprobe_fsm.rsの無防備な書き換えに巻き込まれて新たに失敗する」という形で発覚し、この3件にも同じ`TSF_OBS_TEST_LOCK`を追加する追加修正が必要だった。
- `tsf::warmup::probe_fsm::tests::decide_plan_nc_fired_enables_literal_when_gji_active`: BUG-40で既に「次にこのファイルに触れるセッションで要確認」と記録されていた通り、`nc_fired=true`時に`needs_literal=true`を期待する古い実装(旧`should_prepend_f2`由来、削除済み)の名残だった。BUG-40で確立された新しい意図(`nc_fired=true`＝NameChange確認済みなら常に`needs_literal=false`)に合わせてテスト名・アサーションを更新した(`decide_plan_nc_fired_suppresses_literal_even_when_gji_active`に改名)。

いずれも`cargo test --target x86_64-pc-windows-gnu --no-run -p awase-windows`（`-D warnings`）・`cargo clippy --target x86_64-pc-windows-gnu -p awase-windows`で警告ゼロ確認済み。**2026-07-25、GCP Spot self-hosted runner(`rust-nicola-builder`)上での実`cargo test --lib -p awase-windows`実行で全パス確認済み**(`cargo mutants`のbaselineフェーズが`ok Unmutated baseline in 44s build + 4s test`で成功、GitHub Actions run 30098397721)。当初1回の修正では`literal_detect_fsm.rs`のsleep依存と`probe_fsm.rs`のロック不備を見落としており、計4コミット・3回の実機再実行を経て全15件の失敗が解消したことを確認した(教訓: クロスファイルでグローバル状態を共有するテスト群は、1箇所直すたびに実機で再実行し、マスクされていた別の失敗が露出しないか確認するまで「直った」と判断しないこと)。

なお`cargo mutants`のフル走査(3296ミュータント)自体はjobのtimeout-minutes(180分)内に完走せず`cancelled`になったが、これはミュータント総数が非常に多いことによるもので、baselineの全パスとは無関係(バグではない)。フル走査を完走させたい場合は`--jobs`を増やすかタイムアウトを延ばすか、`-f`で対象ファイルを絞ること。

## BUG-42: IME ON・Engine OFF から一切復旧できない（Ctrl+Shift+変換 が no-op、トレイ「状態をリセット」が誤ったウィンドウを対象にする）

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` →
`Windows.UI.Input.InputSite.WindowClass`、GJI、TsfNative）で、IME アイコンは
ON のまま awase の NICOLA 変換だけが効かなくなり、Ctrl+Shift+変換（Engine ON
コンボ）を押しても・トレイメニューの「状態をリセット」を選んでも復旧しなかった
（2026-07-24 実機ログ）。ログ上は `Engine user_enabled ON (force, active=true)`
というトレイ経由の成功ログが出ているにもかかわらず、直後に Windows Terminal で
K/A を打っても `belief ObservedRomaji 変更なし` のまま変換されなかった。

**IME:** Google 日本語入力（GJI）。TsfNative プロファイル（Windows Terminal）。
ただし原因1・2はいずれも IME/プロファイルに依存しない汎用的な構造欠陥。

**原因1（確定、コード読解で確認）: `Ctrl+Shift+変換` コンボが `user_enabled=true`
のときは文脈起因の inactive を一切救済できない。**

`SpecialKeyCombos::match_event`（`src/engine/engine.rs`）の EngineOn コンボ判定は
`if !engine_enabled && …` という条件でガードされていた。`engine_enabled` は
`adapter.is_enabled()`（＝`user_enabled`）であり、`compute_active(ctx)` が
false になる理由（`ImeOff` / `NotJapaneseIme` / `NotRomajiInput`）は一切見ない。
つまり `user_enabled=true` のまま**文脈**で `Inactive` に陥っているケース
（ユーザーが明示的に無効化したわけではない、最も典型的な「効かなくなった」
状態）では、`match_event` が `None` を返して `check_special_keys` がヒットせず、
`process_key_event` は `PassThrough` を返す。実機ログでは、この PassThrough に
落ちた `Ctrl+Shift+変換`（vk=0x1C, mods c=true s=true）が、IME 候補ウィンドウが
可視だったために無関係な `[ctrl-bypass]`（Ctrl+key 用の composition キャンセル
ロジック、`runtime/message_handlers.rs::handle_wm_key_from_hook`）に飲み込まれ、
`marked cold reason=CtrlKeyBypass` を出しただけで終わっていた。
`force_enable_and_activate`（`ime_on=false` からの `apply_engine_on_with_ime_recovery`
復旧込み）自体は既に存在し `EngineCommand::ForceEngineOn` からは呼べていたが、
キーコンボ経由では到達不能だった。

**原因2（確定、コード読解で確認）: トレイ「状態をリセット」は、メニュー選択
時点の `GetGUIThreadInfo`/`GetForegroundWindow` を対象にするため、実際には
awase 自身のトレイウィンドウ（またはメニューの一時ウィンドウ）に対して IME ON
を発行してしまう。**

`tray::handle_tray_message`（`crates/awase-windows/src/tray.rs`）は、
`TrackPopupMenu` でコンテキストメニューを出すために
`SetForegroundWindow(hwnd)`（`hwnd` は awase 自身のトレイウィンドウ）を呼ぶ。
この時点でユーザーが実際に入力していたアプリ（Windows Terminal 等）から
フォーカスが奪われる。メニュー選択後の `WM_COMMAND`
（`runtime/message_handlers.rs::handle_wm_command`）で
`ime::set_ime_mode`/`set_ime_open_cross_process` を呼ぶと、その内部の
`GetGUIThreadInfo().hwndFocus` はこの時点でトレイ自身（実機ログでは
`class="awase_tray_window"`、直後に `class="#32768"` のコンテキストメニュー）
を指しており、ユーザーが実際に使っていたアプリには一切作用しない。この経路は
「状態をリセット」だけでなく、トレイの「IME 状態」「JISかな / ローマ字」
サブメニュー全項目（ひらがな/カタカナ/英数/直接入力/ローマ字入力/かな入力）に
共通する構造欠陥だった。

**修正:**

1. `src/engine/engine.rs`: `SpecialKeyCombos::match_event` に `engine_active`
   引数を追加し、EngineOn コンボの判定を `!engine_enabled || !engine_active` に
   拡張した。`engine_active` は呼び出し元の `Engine::match_special_keys` が
   `compute_active(ctx)` から渡す。物理キー経由なので `ctx` は常に「今まさに
   フォーカスされている実アプリ」を指しており、原因2のような対象ウィンドウの
   問題は生じない。
2. `crates/awase-windows/src/tray.rs`: `handle_tray_message` の冒頭
   （`SetForegroundWindow` より前）で `GetGUIThreadInfo` を一度だけ問い合わせ、
   `MENU_TARGET_HWND`（新設 static）に保存する。`tray::menu_target_hwnd()` で
   読み出せる。
3. `crates/awase-windows/src/ime.rs`: `set_ime_open_for_target` /
   `set_ime_mode_for_target` / `set_ime_romaji_mode_state_for_target`
   （いずれも新設）を追加し、ライブクエリではなく明示的な `HWND` を対象にできる
   ようにした。既存の `set_ime_open_cross_process` / `set_ime_mode` /
   `set_ime_romaji_mode_state`（ライブクエリ版）はそのまま残し、
   `ime_controller.rs`（物理キー経由の IME ON/OFF）は変更なし。
4. `crates/awase-windows/src/runtime/message_handlers.rs::handle_wm_command`:
   トレイメニュー由来の IME コマンド全項目（Hiragana/FullKatakana/FullAlpha/
   HalfAlpha/HalfKatakana/Direct/InputRomaji/InputKana/ResetState）を
   `tray::menu_target_hwnd()`（無ければ `GetForegroundWindow()` に最終
   フォールバック）を対象にするよう変更した。

**未解決（残存、フォローアップ候補）:** 原因2の修正は「メニュー表示直前の
フォーカスウィンドウ」を対象にするが、ユーザーがトレイアイコンを右クリックする
**前**に既に Start メニュー等 awase 以外の UI を経由していた場合（今回の実機
ログはこのケースだった: トレイクリック前に `explorer.exe` の `InputSite` へ
フォーカスが移っていた）、捕捉されるのは「直前に経由したウィンドウ」であり
「本来入力したかったアプリ」ではない可能性が残る。これは Windows の
フォーカス管理の限界であり、トレイメニュー自体の設計を変えない限り解消しない。
また `force_engine_on`／`EngineCommand::ForceEngineOn`（トレイ「状態をリセット」
の Engine 部分）は `Runtime::build_ctx()` の現在の belief をそのまま使うため、
原因2と同型の「文脈がずれている」問題が理論上残る。原因1の修正により
Ctrl+Shift+変換 が実アプリ上の物理キーとして機能するようになったため、
実運用上の主要な回復手段はこちらに移った。

**テスト:** `src/engine/tests.rs::engine_integration_tests::
special_key_engine_on_combo_recovers_when_context_inactive_but_user_enabled`
を追加（`user_enabled=true` のまま `ime_off_ctx()` で `Ctrl+Shift+変換` 相当の
コンボを押すと `SetOpen(true)` が発行されることを検証。修正前のコードに対して
実際に FAIL することを確認済み）。`cargo test -p awase --lib engine::` で
Linux ネイティブ実行可能。原因2（`tray.rs`/`ime.rs`/`message_handlers.rs`）は
Win32 メッセージループ・フォーカス遷移に依存するため自動テスト困難。
Windows cross-compile（`cargo check --target x86_64-pc-windows-gnu` /
`cargo clippy --target x86_64-pc-windows-gnu --lib`）警告ゼロ確認済み、
`cargo test -p awase-windows --lib` / `--test architecture_guard` /
`--test layer_boundary_guard` / `--test golden_scenarios` は Linux ネイティブで
全 pass（`user_ime_on_paths_are_paired_with_eisu_reset` を含む）。Wine 等の
実行環境がないため Windows 実機での動作確認は未実施。

**関連ファイル:** `src/engine/engine.rs`（`SpecialKeyCombos::match_event`）、
`crates/awase-windows/src/tray.rs`（`MENU_TARGET_HWND`/`menu_target_hwnd`）、
`crates/awase-windows/src/ime.rs`（`set_ime_open_for_target` 等）、
`crates/awase-windows/src/runtime/message_handlers.rs`（`handle_wm_command`）。
関連: BUG-33/BUG-37（同じ「observe できないプロファイルは自己訂正できない」系統の
別の顔）、[fix-requires-evidence](../.claude/rules/fix-requires-evidence.md)、
[ime-belief-architecture](../.claude/rules/ime-belief-architecture.md)。

## BUG-43: `ir_apply_drift_correction`（Blacklist/TsfNative パス）が observation store を更新しないため、同じ IME-OFF 訂正キーを observe tick ごとに無限再送する

**症状:** Windows Terminal（`process=WindowsTerminal.exe`、`fg_class=CASCADIA_HOSTING_WINDOW_CLASS`、`focused_hwnd` class は `Windows.UI.Input.InputSite.WindowClass`、`app_kind=Uwp`、force-tsf 判定で TsfNative 扱い）で GJI（Google 日本語入力）使用中、`[drift] correction: observed=true ≠ desired=false for ...ms → set_ime_open(false)` → `Blacklist drift correction: apply_ime_open(false) → Applied` のログが 2026-07-25T01:28:31.022〜31.697 の約675msの間に **16回連続**（平均間隔 ~45ms、observe tick の 20ms タイマーとほぼ同期）で発火し、そのたびに `GJI direct: send 0x001A (open=false)`（`VK_IME_OFF`）を `SendInput` し、`composition] marked cold reason=SetOpenFalse` で毎回 warm 状態を破棄していた（次の実出力で `VK_DBE_HIRAGANA` warmup が強制される）。ユーザーはこの間、画面上で「キーが連打されたかのような」挙動を観測した。ログの `duration_ms` は補正のたびにリセットされず単調増加していた（84502ms → 85176ms）ことから、乖離が一度も解消されないまま補正だけが空振りし続けていたと確認できる。

**IME:** GJI（Google 日本語入力）。`Imm32Unavailable`/TsfNative 系（Windows Terminal の `InputSite` window、force-tsf 判定で Blacklist 扱い）。desired_open=false（`explicit_intent=Some(false)`）、conv 由来観測（`ObservationSource::ConvOpenInference`、`NativeToggleShadowOff`）は `open=true` を報告し続け、両者が一致しない状態が続いていた。

**原因（コード読解で確認、`crates/awase-windows/src/runtime/ime_refresh.rs::ir_apply_drift_correction`）:** `check_drift_correction`（`state/platform_state.rs`）は `desired_open` と `observations.most_recent_trusted()` を比較して乖離を判定する。乖離を検知すると `ir_apply_drift_correction` の non-ImmCross 分岐（`else` 側、Blacklist/TsfNative 用）が `apply_ime_open_with_belief` で実際に `VK_IME_OFF` を送信し、`on_ime_apply_complete(desired, outcome, None)` を呼ぶ。ここで `generation` が `None` のため、`record_ime_apply_result` は `mirror_applied_open_with_ts`（`shadow_model.applied` を更新するだけ）は呼ぶが、`observations` ストアを更新する `ImeApplySucceeded`/`ImeApplyFailed` イベントは **generation 必須のため dispatch されない**。加えてこのウィンドウクラスは「Skipping IMM query for known-broken class (shadow state SSOT)」により実 IMM クエリ自体を構造的にスキップするため、`observations` を更新する手段がそもそも他に存在しない。結果、`most_recent_trusted()` は補正後もずっと古い（矛盾したままの）観測を返し続け、`check_drift_correction` は次の observe tick（~20ms タイマー）でも同じ乖離を検知し、`explicit_intent == desired` かつ `last_intent` ありの場合は再送閾値が `DRIFT_CORRECTION_THRESHOLD_MS`（400ms）ではなく即時（0ms）になる高速パスに入るため、tick のたびに無条件で同じ VK を再送する無限ループになっていた。

**BUG-33 との違い:** BUG-33 は drift correction が「構造的に一度も発火し得ない」（belief が自分自身を観測として書き戻す循環で乖離自体が生成されない）バグだった。本バグは逆に、乖離は正しく生成されるが、**補正の適用結果が観測側にフィードバックされないため一度も収束しない**バグ。

**修正:** `observations` ストアへのフィードバック経路を新設する案（`.claude/rules/ime-belief-architecture.md` が禁止する「実際には観測していないのに観測した体で dispatch する」偽装になりかねない）ではなく、BUG-33 の `Output::last_gji_reinit_ms` と同じ手法で、`Runtime` に `last_drift_correction_send: Option<(bool, Instant)>` を追加し、直前に同じ `desired` へ補正を送ってから `DRIFT_CORRECTION_THRESHOLD_MS`（400ms、既存定数を流用、新規実測は不要）未満なら再送をスキップして `schedule_ime_refresh` で残り時間だけ再試行を遅延させるようにした。乖離が実際に続く場合は 400ms 間隔で補正が継続されるため訂正自体は損なわれず、observe tick 間隔（~20-45ms）での tight loop だけを防ぐ。

**追記（恒久対応）:** 上記の `last_drift_correction_send` によるタイムスタンプ手作りクールダウンは暫定策であり、その後 ADR-080（[docs/adr/080-ime-actuation-lifecycle-and-epoch-fenced-drift-correction.md](adr/080-ime-actuation-lifecycle-and-epoch-fenced-drift-correction.md)、Phase 1 実装済み・実機ソーク未実施）の `Actuation`/`FeedbackPolicy` 機構に置き換えられた。ad-hoc な経過時間比較ではなく、型で強制される有界リトライ（`Blind`）／観測確認（`Read`）という設計で終端制御するため、「補正が観測にフィードバックされず収束しない」問題を型レベルで扱う。本バグに該当する TsfNative/Blacklist パスは `Blind` 方針となり、`IME_ACTUATION_BLIND_MAX_ATTEMPTS`（暫定 5、実機未検証）回で `Resolution::GaveUp` に達して以降 `desired` が変わるまで再送しない（observe tick ごとの tight loop が型レベルで不可能になる）。一度 give-up した後も外部で状態が動いた場合に硬直しないよう、`Actuation.gave_up_at` 以降に何らかの trusted 観測が record された（`ObservationStore::most_recent_trusted_after` が `Some` を返す＝**値ではなく鮮度**を見る）ときのみ `Actuation` を破棄して再武装する。これに伴い `Runtime::last_drift_correction_send` フィールドはコードベースから撤去済み。恒久対応の回帰ガードは `crates/awase-windows/tests/architecture_guard.rs`（呼び出し箇所数の count guard ＋ 不変条件6 の「GaveUp/deadline 超過時に観測を書き込まない」ガード）と state 層の単体テスト（`state/ime_actuation.rs`・`state/observation_store.rs`・`state/app_ime_policy.rs`）にあるが、**Windows 実機での実行検証は未実施**（wine 未導入）。次回の Windows セッションで、この症状（連続する `Blacklist drift correction` ログ・似た「キー連打」体感）が再発しないか実機確認すること。

**テスト:** `runtime` モジュールは `#[cfg(windows)]` のため Linux 上の `cargo test -p awase-windows`（native target）には含まれず、`check_drift_correction` 自体（純関数、`state/platform_state.rs`）も本修正では変更していないため既存テストに追加すべき単体テストが無い。`cargo check -p awase-windows --target x86_64-pc-windows-gnu --lib`・`cargo clippy -p awase-windows --target x86_64-pc-windows-gnu --lib -- -A clippy::cargo_common_metadata -D warnings`（警告ゼロ）で確認済み。wine 未導入のためこのサンドボックスでは実行検証不可（他バグと同様の制約）。実機ソークは未実施のため、次回この症状（連続する `Blacklist drift correction` ログ、または似た体感の「キー連打」）が出ないか確認すること。

**関連ファイル:** `crates/awase-windows/src/runtime/ime_refresh.rs`（`ir_apply_drift_correction`）、`crates/awase-windows/src/runtime/mod.rs`（`Runtime::last_drift_correction_send` フィールド追加）、`crates/awase-windows/src/state/platform_state.rs`（`check_drift_correction`、変更なし）。関連: BUG-20（drift correction 送信側の対称バグ）、BUG-33（drift correction 検知側の逆方向バグ）。

## BUG-44: `tray_wnd_proc` の「到達不能」判断が逆で、トレイ右クリックのコンテキストメニューが一切表示されなくなった

**症状:** `develop` ブランチで、システムトレイのアイコンを右クリックしてもコンテキストメニューが一瞬も表示されない（フラッシュすらしない）。ユーザー報告（2026-07-27）。実機ログは未取得。

**IME:** 本バグは IME 制御そのものとは無関係（Win32 メッセージディスパッチのバグ）。ただし波及範囲としてトレイメニュー経由の IME 系コマンド（ひらがな/カタカナ/英数/直接入力/ローマ字入力/かな入力/状態をリセット）もすべて選択不能になる。

**原因（コード読解で確認、`crates/awase-windows/src/tray.rs`）:** `4508231`（2026-07-27、「`tray_wnd_proc` の到達不能な `WM_TRAY_CALLBACK`/`WM_COMMAND` を削除」）が、`tray_wnd_proc` から `WM_TRAY_CALLBACK`（`WM_APP`、Shell からのトレイ通知）と `WM_COMMAND`（`TrackPopupMenu` のメニュー選択確定）のハンドラを削除した。削除の根拠は「`app::run_message_loop` の `match msg.message` が `DispatchMessageW` より先にこれらを横取りするので `tray_wnd_proc` 側は実行されない」という判断だったが、これは逆だった。`WM_COMMAND` については確実に説明できる: `TrackPopupMenu`（`TPM_RETURNCMD` 未指定）はメニュー選択確定時、自身が持つ内部モーダルループから呼び出し元スレッドの `WndProc` へ**同期的に**配送する（`GetMessageW` の戻り値としては一切現れない、sent message 相当の経路）。`WM_TRAY_CALLBACK` についても、`4508231` は `tray_wnd_proc`側のみを変更し `run_message_loop` の `WM_APP` 分岐はそのまま残していたにもかかわらず右クリックでメニューが一切表示されなくなったという実機事実（本バグ）から、少なくとも配送は `tray_wnd_proc`側にしか届いておらず `GetMessageW` の戻り値経由では観測されていないことが確認できる（正確な配送機構が sent message か別経路かは未確定）。つまり `run_message_loop` 側の `WM_APP =>` / `WM_COMMAND =>` 分岐は元々 tray 由来のイベントに対しては到達しない死んだコードで、実際に唯一到達していたのは `tray_wnd_proc` 側のハンドラだった。`4508231` はこの「実際に生きている方」を削除してしまったため、右クリックイベントが `tray::handle_tray_message`（`TrackPopupMenu` を呼ぶ関数）まで一度も届かなくなり、メニューが完全に表示されなくなった。

**`4508231` のコミット文面にあった「挙動変更なし」という判断が誤っていた理由:** 同コミットは「`message_handlers::handle_wm_command` 側だけ 2026-07-24 の `c9d69ad` で修正され、`tray_wnd_proc` 側は旧ロジックのまま乖離していた」ことを削除の傍証にしていたが、`c9d69ad` 自体も「Windows 実機での動作確認は未実施」と明記されており、当時から一度もこの経路が実機で通しで検証されていなかった。`4508231` も同様に実機未検証のまま "clippy/cargo check がゼロ警告だから安全" という基準でマージされていた。**Win32 メッセージ配送の到達可否は静的解析（cargo check/clippy/cross-compile）では検出できず、実機での右クリック動作確認が必須**という教訓。

**修正:** `tray_wnd_proc` に `WM_TRAY_CALLBACK`/`WM_COMMAND` のハンドラを復元した。ロジックの重複・再陳腐化を避けるため、`message_handlers::handle_wm_app_tray`/`handle_wm_command`（現行の正しい実装、`menu_target_hwnd()` によるフォーカスウィンドウ捕捉を含む）へ委譲する形にし、`run_message_loop` 側の `WM_APP`/`WM_COMMAND` 分岐は保険としてそのまま残した（sent message 前提の理解が今回も誤っていた場合に備えたフェイルセーフ、削除しない）。

**テスト:** Win32 メッセージループの実際の配送経路（sent か posted か）は Windows 実機でしか検証できず、Linux 上の `cargo test`/`cargo nextest` では再現不可能（`architecture_guard`/`golden_scenarios`/`layer_boundary_guard` 全 42 件・lib 218 件は pass 済みだが、これらはメッセージディスパッチ機構自体を対象にしていない）。`cargo check`/`cargo xwin clippy --target x86_64-pc-windows-msvc -- -D warnings` は警告ゼロ。**Windows 実機で右クリック→メニュー表示を確認済み（2026-07-27、修正後に動作確認）**。各メニュー項目（設定/学習キャッシュクリア/再起動/自動起動/IME状態切替/状態をリセット/終了）個別の網羅的な動作確認までは実施していないため、いずれかが選択不能な場合は本バグの経路の別の側面を疑うこと。

**関連ファイル:** `crates/awase-windows/src/tray.rs`（`tray_wnd_proc`）、`crates/awase-windows/src/runtime/message_handlers.rs`（関数本体は変更なし、モジュール doc コメントのみ更新）。関連: BUG-39（`c9d69ad` が修正した menu_target_hwnd 導入の経緯）。

---

## BUG-45: per-VK confirm の literal 判定が「代理指標のタイムアウト」に基づく belief であり、actual な TSF composition 状態と乖離しても検出も訂正もできない

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` → `Windows.UI.Input.InputSite.WindowClass`、GJI、TsfNative）で、物理 F0→F2（IME OFF→ON トグル）直後に「かきの」と入力したところ、「か」だけ literal ローマ字が残り **"kaきの"** になった（2026-07-29 実機ログ）。

**再現手順（ログで確認済み）:**

```
物理 F0 up / F2 down → Shadow IME toggle OFF→ON、reason=SetOpenTrue で composition を cold mark
  [h1-warmup] reason=SetOpenTrue → F2/probe待機省略、per-VK confirm へ
  [gji-coro] settle 必要 (reason=SetOpenTrue, settled=false) → skip FreshF2, reactive LiteralDetect のみ
romaji "ka" 送信、per-VK confirm 開始（cold=585）:
  vk=0x4B('K') 送信 → confirm 締め切りまでに「合成できた」代理証拠
    （候補ウィンドウ SHOW / GJI I/O 増加）が届かず suspected literal(idx=0) 判定
    → RawTsfLiteralRecovery{backs:1} backspace×1 + "ka" 再送 scheduled、mark cold
再送 "ka" も同じ経路（reason=RawTsfLiteralRecovery → per-VK confirm、cold=586）に入り
  vk=0x4B('K') が再び suspected literal(idx=0、consecutive=1)
  → 2連続 literal → give-up: backspace×1のみ（再送なし）+ VK_IME_OFF→VK_IME_ON reinit 予約
flush 時に backspace×1 → reinit(VK_IME_OFF→VK_IME_ON) 実行、IMC poll で Hiragana 確認
  → 以降 "ki" は gji_settled=true で unicode transmit 経由、正常に「き」として出力
最終出力: "ka" が literal のまま残存 + "きの" は正常 = "kaきの"
```

**IME:** Google 日本語入力（GJI）。TsfNative プロファイル（Windows Terminal 等）。

**原因（ログ・コード読解で確認、確定的な裏付けは一部未取得）:** これは単体では「backspace 数が足りない」局所バグではなく、`.claude/rules/ime-belief-architecture.md` が `GjiFsm` について既に指摘している構造と同型の **actual と belief の乖離** が真因と考えられる。

- `is_partial_literal`（`tsf/warmup/literal_detect_fsm.rs:130`）が明記する通り、TSF native アプリ（Windows Terminal 含む）は `HIMC=NULL` のため **IMM32 composition 文字列と実際の画面出力を直接照合する手段が存在しない**。per-VK confirm の「suspected literal」判定は、実際に文字が literal として出力されたかどうかの直接観測ではなく、「confirm 締め切り（`literal_detect_ms`）までに合成成功の代理証拠（候補ウィンドウ SHOW / GJI I/O バイト増加）が届いたか」という**間接プロキシのタイムアウト**でしかない。`SetOpenTrue` cold path はこの代理証拠が届く前提の `FreshF2` warmup 自体を意図的にスキップしている（`skip FreshF2, reactive LiteralDetect のみ`）ため、GJI 側がまだ本当に composition を開始できていないだけ（literal ではなく単に遅い）なのか、本当に literal 化したのかを confirm 時点で区別する情報が構造的に存在しない。
- `RawTsfLiteralRecovery` の `backs`（バックスペース数）は `per_vk_recovery_params(is_stale, failed_idx)`（`tsf/warmup/literal_detect_fsm.rs`）という**純関数の固定値**（`failed_idx==0` なら常に `backs=1`）であり、実際に画面に何文字残っているかを一切観測していない。recovery アクション自体も「1文字だけ literal になったはず」という belief に基づく固定処理であり、actual な出力とすり合わせるフィードバックループがない。
- 2連続 give-up 後に発行される `VK_IME_OFF→VK_IME_ON` reinit（`send_chrome_gji_reinit_and_poll`、`output/probe_io.rs`）は、同ファイルの BUG-36 コメントが自ら明記する通り「未確定の preedit を commit してしまう」。もし1回目・2回目に suspected literal と判定された vk=0x4B の送信が実際には（遅延していただけで）GJI 側の pending composition として溜まっていたのだとすれば、それぞれの backspace(1) はこの未確定状態を正しく除去できず、最終的に reinit の `VK_IME_OFF` がその溜まった preedit を "ka" として commit してしまった、という筋が計算上（2回とも backs=1 ずつなのに実際には2文字とも消えずに残った）最も辻褄が合う。

要するに、この経路には「actual にどう出力されたか」を確認してから次の一手を決める箇所が一つもない: suspected literal 判定も、backspace 数も、give-up 後の reinit も、すべて過去の代理指標から推測した belief の上に belief を積み重ねているだけで、どこかで一度でも belief が実態とズレると訂正する手段がない。

**未対応（残存）:**

- 「actual と belief のズレ」を実機で機械的に切り分ける追加ログ（SendInput 送出タイムスタンプ vs 実際に画面に literal 文字が現れたかの UIA/クリップボード等での確認）は未取得。上記「原因」は状況証拠からの推論であり、SendInput の正確なタイムスタンプと実際の画面バッファ内容の突合せまでは未実施。
- 恒久対策の方向性は要検討・未実装: (a) TSF native アプリでも composition 文字列を照合できる代替手段の探索（`HIMC=NULL` の制約を回避できるか）。(b) 「合成成功の証拠切れ」を即 literal 確定にせず、実際に画面へ literal 文字が出たことを確認してから backspace する設計への変更。(c) `SetOpenTrue` cold path で `FreshF2` skip をやめ、常に一定の settle 待ちを入れる（レイテンシとのトレードオフ、`.claude/rules/tuning-constants.md` の実測義務が伴う）。

**追補1（2026-07-29、3方向の独立解析で「原因」3点目の具体的筋を反証）:** 上記「原因」の3点目（2回とも suspected literal になった vk=0x4B の送信が実は pending composition として溜まっており、give-up 後の reinit の `VK_IME_OFF` がそれを "ka" として commit した、という筋）を、Claude 本体・Fable（Claude 5 系列モデル）・Codex CLI の3系統で独立に検証した。

- **`run_per_vk_confirm`（`tsf/warmup/probe_fsm.rs:399-483`）はコード上、`idx=0`（vk=0x4B='K'）で `SuspectedLiteral` を検出すると `emit_recovery_actions` の後すぐ `return` する**ため、`idx=1`（vk=0x41='A'）には cold=585・cold=586 のどちらの試行でも到達しない。ログ全文を `vk=0x41` で検索しても、バグ発生時間帯（23:35:32〜33.6）には一件も出現しない（出現するのは冒頭 23:35:26 の無関係な Ctrl+A パススルーのみ）。**つまり "ka" の 'A' は awase から一度も SendInput されていない**ことが3系統とも一致して確認できた。
- 一方、この事実は「原因」3点目の "pending composition が 'ka' として commit された" という筋の**具体的な裏付けにはならない**（'A' を送っていないなら、pending composition に 'a' が含まれる余地もそもそも無いはず）。3点目は状況証拠からの推論であり反証はできていないが、支持する具体的証拠も無いことが判明したため、**確度は「推測」に格下げする**。
- 検証の過程で1つ誤仮説が出た: 「cold=586 は実は "ka" の再送ではなく次の文字 "き" の probe であり、"ka" の再送こそが `pending_deferred` に退避されて確認なしで raw 送出された」という仮説（Fable 起源）。**これはログと矛盾し誤り**: `re-sending raw TSF literal romaji="ka"` の直後の行が `[h1-warmup] cold=586 ... reason=RawTsfLiteralRecovery` であり、"き" は逆にその後に `[tsf] probe in flight → deferred 2 VK(s) for "ki"` として（"ka" ではなく "ki" と明示されたログで）退避されている。cold=585/586 とも "ka" の probe である。今後この筋を再検討する場合はこの反証を踏まえること。
- backspace(×1 を2回)が実際に画面上・composition 内で何を消費/削除したかは、`himc_null=true` により composition 文字列を直接読めないため本ログからは断定不能（未観測のまま）。"a" の literal 出力メカニズムは依然として未確定。
- 次に取るべき具体策（実施すればログ範囲を広げるより有効）: (1) backspace flush 直後・reinit 直後のタイミングで実際の画面/バッファ内容を読み取る一時的な診断ログを追加してから再現を取る、(2) "き" 等の後続文字入力との干渉を排除するため、IME トグル直後に「か」だけ入力して数秒待ち、単独でも同じ literal 化が起きるか確認する最小再現を取る。

**関連ファイル:** `crates/awase-windows/src/tsf/warmup/probe_fsm.rs`（`run_per_vk_confirm`）、`crates/awase-windows/src/tsf/warmup/literal_detect_fsm.rs`（`per_vk_recovery_params`、`is_partial_literal`）、`crates/awase-windows/src/output/probe_io.rs`（`ProbeAction::RawTsfLiteralRecovery` ディスパッチャ、BUG-36 コメント）、`crates/awase-windows/src/tsf/warmup/cold_warmup.rs`（`h1-warmup`）。関連: BUG-24（per-VK confirm 導入元）、BUG-33 追補3・4（`GjiFsm` の belief/actual 乖離の同型事例）、BUG-36（reinit が preedit を commit するレース）、BUG-38/BUG-39/BUG-40（cold-start × literal-detect の他の失敗モード）、`.claude/rules/ime-belief-architecture.md`（`GjiFsm` 2026-07-23 追記）。

---

## BUG-46: `PhysicalKeyDisposition::plan` が TsfNative の物理 KANJI 系キーを無条件 Allow するため、GJI/MS-IME 環境で awase 自身の apply-ime actuation と二重に actuate する

**症状:** Windows Terminal（GJI、hwnd class `CASCADIA_HOSTING_WINDOW_CLASS` →
`Windows.UI.Input.InputSite.WindowClass`、`AppImeProfile::TsfNative`）で、物理の
半角/全角キー（hook 上は `VK_DBE_SBCSCHAR (0xF3)` up → `VK_DBE_DBCSCHAR (0xF4)` down
のペアとして届く）を押すと、awase 内部の belief では IME ON になったことになって
いるのに、実際には GJI 側の変換が効かない状態になる（ユーザー通報「なぜか、VK_KANJI
打鍵時に IME ON / Engine OFF になる」、2026-08-01）。

**再現手順（実機ログで確認済み、時系列順）:**

```
[hook] vk=0xF3 up / vk=0xF4 down (物理、self_injected=false, scan=0x29)
Shadow IME toggle: OFF → ON (vk=0xF4, source=PhysicalImeKey)
Engine activated (ime=true, ...)
[dispatch-ime] belief: effective=false confident=true (profile=TsfNative)
[apply-ime] GJI direct: send 0x0016 (open=true)   ← awase 自身の SendInput(VK_IME_ON)
[hook] vk=0x16 down/up self_injected=true          ← 上記の実際の送信
[apply-ime] outcome=Applied
[engine-state-key] skipped (apply_ime_open aligned ime=true, profile=TsfNative)
[reinject] vk=0xf4 down (queued passthrough now firing)  ← 遅延していた「元の」物理キーが
                                                             awase の actuation の後に着弾
[ime-mode] SetOpen(true) applied → Hiragana (belief, unconfirmed)
```

**IME:** Google 日本語入力（GJI）。TsfNative プロファイル（Windows Terminal 等）。

**原因（確定、コード読解 + Claude 本体・Opus（Claude 5 系列）・Codex CLI の3系統
独立検証で一致）:**

- `AppImeProfile::should_pass_physical_key()`（`focus/class_names.rs:177-179`）は
  `TsfNative` で常に `true` を返す。`PhysicalKeyDisposition::plan`
  （`runtime/transport.rs`、旧実装）は KANJI 系イベントの suppress 判定にこの値のみを
  使っており、TsfNative では物理キーを無条件 `Allow`（素通し）していた。
- 一方 `GjiDirectStrategy`（`ime_controller.rs:10,17`）は「GJI 検出済みなら全プロファイル
  で適用される」設計であり、TsfNative でも `SendInput(VK_IME_ON/OFF)` を独自に送る。
  同じく `MsImeDirectStrategy` も `!profile.can_use_imm32_cross_process()`（TsfNative は
  常に該当）であれば適用される。
- つまり同一の1回の物理キー押下に対し、(a) awase 自身の apply-ime SendInput と
  (b) 元の物理キーイベントの reinject という**二重の actuation** が GJI/MS-IME に届く。
  `ImmCross`/`Imm32Unavailable` プロファイルには既にこの種の二重制御を防ぐ suppress
  ロジック（`transport.rs` コメント「二重制御による OS 側 spurious VK_F3/F4 の生成を
  防ぐ」）があったが、TsfNative にはこの保護が欠落していた。
- **順序も確定**（`runtime/executor.rs` の `ReinjectKey` push 位置 + `drain_deferred` の
  FIFO）: 送出順は「awase の apply-ime SendInput → 物理キーの reinject」であり、
  物理キーが最後に着弾する。`VK_DBE_DBCSCHAR (0xF4)` はひらがな変換指定ではなく
  全角プレーン指定（`vk.rs:111,125`、`ShadowImeAction::TurnOn` であり toggle ではない）
  のため、awase がひらがなへ合わせた直後にこれが上書きし、belief は ON のまま実際の
  変換が効かない状態になる、というのが確度「推測」の具体的機構。二重 actuation
  経路そのものの存在は確度「確定」。
- **なぜ TsfNative だけ保護が漏れていたか**: `key_pipeline.rs` の「TSF が KANJI を
  正しく処理するため物理キーを通す」というコメント・実装は、`GjiDirectStrategy` が
  全プロファイル適用に拡張される前の前提のまま残っていた。前提が破綻した後も
  コメントが古い状態を正当化し続けたことが、この種のバグが再発する典型パターン。

**修正（本コミット）:** `PhysicalKeyDisposition::plan` の suppress 判定を
`profile.should_pass_physical_key()` から、`ActiveImeKind` ベースの
`gji_direct_applicable`/`ms_ime_direct_applicable`（`state/key_sequence_policy.rs`、
既存の戦略選択と同じ SSOT）で導出する `ime_actuation_owned` に置き換えた。これにより
TsfNative も Imm32Unavailable と同じ suppress ロジック（shadow_toggle 発火時 KeyDown
+ 全 KeyUp を Suppress）に統一される。`ImmCross` プロファイルの既存の「常に Suppress」
分岐は変更していない。

**未対応（残存、フォローアップ候補）:**

- `state/app_ime_policy.rs::AppImePolicy::owns_physical_kanji` は本バグの suppress
  判定には使われていない別系統の（focus_settle_ms/feedback 方針向けの）静的な
  profile 単位フラグで、TsfNative は `false` のまま（`ActiveImeKind` を見ない）。
  今回の修正で実際の suppress 判定とこのフラグの意味が乖離した可能性があるため、
  `state/ime_profile_driver.rs`（ADR-081/082 の per-profile ドライバ分離、Phase 1a/1b
  時点）側で `owns_physical_kanji` を将来 `ActiveImeKind` 込みで再定義するかどうかは
  未検討。今回はスコープ外として変更していない。
- `transport.rs::PassthroughQueue.deferred_vks` は、0xF4 のような「ペア表現」の
  KANJI 系キーで対応する up が原理的に来ない場合エントリが残留し得る（Opus 指摘の
  副次的所見）。本バグの主因ではないため未対応。
- 実機での再現待ち（`RUST_LOG=debug` で TsfNative+GJI 環境において、物理半角/全角
  キー押下時に KANJI 系イベントが Suppress されるようになったことをログで確認する）。

**検証状況:** コード読解による確定 + Claude 本体・Opus（Claude 5 系列モデル）・
Codex CLI の3系統独立解析で二重 actuation 経路の存在と原因を一致確認（2026-08-01）。
`transport.rs::plan_tests` に回帰テストを追加（TsfNative+GJI/MsIme で
Imm32Unavailable と同じ suppress 挙動になることを固定）。`runtime` モジュールは
`#[cfg(windows)]` のため Linux 上では `cargo check --target x86_64-pc-windows-gnu`
での型検査のみ実施、実機/Windows 環境でのテスト実行は未実施。

**関連ファイル:** `crates/awase-windows/src/runtime/transport.rs`
（`PhysicalKeyDisposition::plan`）、`crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_stage_execute`）、`crates/awase-windows/src/focus/class_names.rs`
（`AppImeProfile::should_pass_physical_key`）、`crates/awase-windows/src/ime_controller.rs`
（`GjiDirectStrategy`/`MsImeDirectStrategy`）、`crates/awase-windows/src/state/key_sequence_policy.rs`
（`gji_direct_applicable`/`ms_ime_direct_applicable`）、`crates/awase-windows/src/runtime/executor.rs`
（`ReinjectKey` 順序）、`crates/awase-windows/src/vk.rs`（`VK_DBE_SBCSCHAR`/`DBCSCHAR`）。
関連: BUG-07/BUG-09/BUG-11/BUG-20/BUG-34（いずれも「IME ON / Engine OFF」という
ユーザー通報文言が既出だが原因系統は異なる）。

## BUG-47: `Vk`/`Tsf` 注入モードで記号（句読点「。」「、」・長音「ー」等）を送ると、cold-start ウォームアップ保護が無いため半角のまま出力される

**症状（2026-08-03 ユーザー報告、v1.12.0）:** キーボード FKB7628-801(Thumb Touch)、
IME Google 日本語入力。`.yab` レイアウトで句読点リテラル「。」「、」を配置しても
半角の「.」「,」が出力される。長音記号「ー」も半角のハイフンマイナス「-」になり、
IME が正しく変換できない（例:「き-」となり「きー」にならない）。

**原因（確定、コード読解で確認）:** `.yab` パース（`src/yab/mod.rs`）・
`KeyAction::Char` への変換（`src/engine/nicola_fsm.rs`）はいずれも正しい。原因は
出力層（`crates/awase-windows/src/output/vk_send.rs`）。`send_char_as_tsf`/
`send_char_as_vk` の `CharResolution::Vk` アーム（記号を `symbol_to_vk` テーブル
経由で生 VK として送る経路）は、通常のローマ字送信（`send_romaji_as_tsf`/
`send_romaji_batched`）が必ず呼ぶ `assess_warmth()` によるcold-startウォームアップ
判定（F2 事前送信・probe設置・MS-IME confirmゲート・LiteralDetect）を一切呼ばず、
IME/TSF が cold（未ウォームアップ）な状態でも記号 VK を無条件送信していた。GJI/TSF
が変換エンジンとして受理する準備ができていないため、送信された VK が変換されず
半角ASCIIのまま素通しされる。`docs/known-bugs.md` の BUG-01/02/03（ローマ字の
最初の1文字がリテラル化する）と同じクラスのバグだが、記号送信経路には最初から
この対策が配線されていなかった。

**修正:** `vk.rs` に `ascii_to_vk` の厳密な逆写像 `vk_pair_to_ascii(vk, needs_shift)
-> Option<char>` を追加し、ASCII 1 文字で表現できる記号（`-`/`.`/`,`/`/` および
英数字。今回報告の3文字はすべて該当）は `send_char_as_tsf`/`send_char_as_vk` の
`CharResolution::Vk` アームから通常のローマ字送信経路（`send_romaji_as_tsf`/
`send_romaji_batched`）へ合流させ、cold-startウォームアップ・probe設置・
LiteralDetectFsm による事後訂正をすべて共有させた。warm パスの送信バイト列は
従来の `send_vk_pair(vk, needs_shift, marker)` と同一になるよう設計してある
（`vk_pair_to_ascii_roundtrips_with_ascii_to_vk` テストで固定）。新規タイミング
定数は追加していない（既存の `assess_warmth()` と判断基準をそのまま再利用）。

**未対応だった点（2026-08-05 追補で解消。下記参照）:** 初回修正時点では Shift
付きの記号（`？`/`！`/`～`/`＋` 等）が `ascii_to_vk` に逆像が無いため対象外だった。
当時は恒久対応に probe のペイロード型を `romaji: String` から
`Vec<(VkCode, bool)>` へ一般化する必要があると見積もっていたが、これは誤りだった
（詳細は下記追補）。

**テスト:** `crates/awase-windows/src/vk.rs` に
`vk_pair_to_ascii_roundtrips_with_ascii_to_vk`（VK 0x00-0xFF × shift 2値の
全網羅ラウンドトリップ）・`vk_pair_to_ascii_rejects_shift`・
`vk_pair_to_ascii_covers_reported_symbols`（今回の3文字を明示的に固定）を追加。
`cargo test -p awase-windows --lib`（271 passed）・`cargo check`/`cargo clippy
--target x86_64-pc-windows-gnu --lib -- -D warnings`・`architecture_guard`/
`layer_boundary_guard`/`golden_scenarios` 全 green を確認。wine 未導入のため
このサンドボックスでは実機での再現・修正確認そのものは未実施（次の Windows
実機セッションで Chrome/Edge + GJI にて「。」「、」「ー」の cold-start 直後の
出力を確認すること）。

**追補（2026-08-05 ユーザー報告: `！`（びっくりマーク）が半角化）:** 上記
「未対応」だった Shift 付き記号も同一機構で解決した。**当初の見積もり
（probe ペイロード型の `Vec<(VkCode, bool)>` 化が必要）は誤りだった** —
`vk_send.rs` の `CharResolution::Vk` アームは既に `vk_pair_to_ascii` の
戻り値だけでルーティングを決めており、`send_vk_run_batch`（`key_injector.rs`）
は要素ごとに `needs_shift` を見て `VK_LSHIFT` down/up を挟む汎用実装だった
（`ascii_to_vk('A') == Some((VkCode(0x41), true))` の大文字対応がこの汎用性の
証拠として既に存在していた）。cold パスの `ProbeAction::TransmitSingleVk` /
`send_single_tsf_vk`/`send_single_chrome_vk` も `needs_shift` を保持したまま
同じ `send_vk_pair` を呼ぶ。つまり `ascii_to_vk`/`vk_pair_to_ascii` の
Shift ガード（`if needs_shift { return None }`）を外して対応表を
`build_symbol_to_vk` の「半角 ASCII 記号」節（21種）まで拡張するだけで、
probe/FSM 側は無改修のまま Shift 付き記号も cold-start 保護に合流した。

修正: `vk.rs` の `ascii_to_vk` に `[` `]` `;` `:` `@` `^` `\` (shift不要) と
`!` `"` `#` `$` `%` `&` `'` `(` `)` `?` `=` `+` `*` `<` `>` `_` `{` `}` `|` `~`
`` ` `` (shift付き、21種) の match arm を追加。`vk_pair_to_ascii` は
`match (vk.0, needs_shift)` のタプルマッチに書き換えて対称に拡張した（`vk.0`
だけで分岐する range match だと、意図せず既存の英大文字/数字アームが
shift 有無を無視して先に一致してしまう実装ミスを避けるため）。

テスト: `vk_pair_to_ascii_covers_shift_symbols`（`！`/`？`/`～` を明示固定）、
`vk_pair_to_ascii_covers_every_build_symbol_to_vk_pair`（`build_symbol_to_vk`
の全 `(VkCode, needs_shift)` ペアが `vk_pair_to_ascii` で `Some` になることを
走査で固定 — 記号が cold-start 保護から漏れたら即座に落ちるドリフト防止）。
`vk_pair_to_ascii_rejects_shift` は前提（「shift は常に None」）が成り立たなく
なったため `vk_pair_to_ascii_rejects_unmapped_vks`（英大文字 Shift・F1・
Backspace が引き続き None）に置き換えた。`cargo test -p awase-windows --lib`
（273 passed）で確認。実機（Chrome/Edge + GJI、`！`/`？`/`～`/`＋` の cold-start
直後出力）での再確認は次の Windows 実機セッションで行うこと。

Opus によるレビュー（2026-08-05）: GO-WITH-CHANGES。指摘事項（タプルマッチ化・
`vk_send.rs` の「Shift付きは未対応」コメント修正・`values()` ベースのドリフト
防止テスト・本追補によるドキュメント訂正）はすべて本追補で反映済み。

**関連ファイル:** `crates/awase-windows/src/vk.rs`（`vk_pair_to_ascii`）、
`crates/awase-windows/src/output/vk_send.rs`（`send_char_as_tsf`/`send_char_as_vk`）。
関連: BUG-01/BUG-02/BUG-03（同クラスの cold-start リテラル化）。

## BUG-48: `Engine::check_active_transition` の対称 `SetOpen` echo がユーザーの明示的な IME OFF 意図（`last_intent`）を上書きし、数百ms〜数秒後に Engine が勝手に ON へ戻る

**症状（2026-08-04 ユーザー報告、v1.12.0、MS-IME/TSF native 環境）:** ユーザーが
無変換キー等で明示的に IME を OFF にした直後（数百ms〜数秒後）、何も操作していないのに
ログに `Engine activated (ime=true, ...)` が出て NICOLA エンジンが勝手に再度アクティブに
なる。実機ログでは `[idle-conv-check] TsfNative: conv observation open=true
reason=NativeToggleShadowOff → ObserverReported として記録 (engine は actuate しない)`
のバーストの直後にこの再活性化が繰り返し発生していた。

**原因（仮説と確定部分が混在。2026-08-04 の Opus セカンドオピニオンレビューで
一部誤りが判明し、このセクションは訂正済み。実機再現ログでの追跡は未実施）:**

`ime-belief-architecture.md` の設計では `effective_open()`
（`state/ime_model.rs`）は「ユーザーの明示的意図（`last_intent`）がある間は観測より
必ず優先する」ため、`report_conv_open_inference` が記録する `ObserverReported`
（Medium confidence）だけでは `desired_open`/`last_intent` は書き換わらない
（BUG-19 再発対策で確認済み）はずだった。

**確定している構造的な欠陥（コード読解で確認済み）:** `src/engine/engine.rs::
transition_activation()` は、Engine が Inactive→Active（またはその逆）に遷移する
**たびに**、理由を問わず `Effect::Ime(ImeEffect::SetOpen{open: now_active})` を
"対称性のため"自動発行する（`// inactive → active: OS IME を強制的に開く`）。
この effect が **キーボード経路**（`kp_run_inner` → `kp_stage_post_decision`、
毎キー入力で `check_active_transition` が呼ばれる）を通る場合、修正前は
それがユーザーの本物のキー操作（IME ON/OFF コンボ）由来なのか、Engine 内部の
遷移が"つじつま合わせ"で出した echo なのかを区別せず、どちらも同じ
`handle_engine_set_open()` → `write_set_open_request()` →
`ImeEvent::UserImeSetIntent{source: Command}` を dispatch していた。これは
`last_intent` を「本物のユーザー意図」として上書きし、以後の drift correction も
この echo を正当な意図として扱ってしまう欠陥であり、実際に存在した
（`ime-belief-architecture.md` が禁止する「観測を偽装した内部補正」と同型）。

**⚠️ 訂正: `EngineCommand::RefreshState`（IME ポーリング/idle-conv-check 由来、
キー入力を経由しない `ctx.ime_on` 変化）経路については、当初「これも
`kp_stage_post_decision` に到達し `last_intent` を汚染する」と書いたが、これは
**誤り**。`ir_notify_engine_refresh`（`runtime/ime_refresh.rs`）が発行する
`RefreshState` の Decision は `Runtime::execute_decision` →
`DecisionExecutor::execute_from_loop`（`runtime/executor.rs`）を通り、この経路は
`ime: &ImeStateHub`（**不変参照**）しか持たないため `handle_engine_set_open`/
`handle_engine_activation_sync` のどちらも呼べない。つまり修正前でも
`RefreshState` 由来の `SetOpen` echo は `last_intent` を汚染していなかった。
それでも `SetOpen` effect 自体は `execute_from_loop` → `execute_one` →
`dispatch_ime_set_open` 経由で **OS へは無条件に適用される**
（`executor.rs::dispatch_effect` が `origin` を捨てて `open` だけ見ている、
`SetOpen { open, .. }` のパターンで確認できる）。つまりこの経路では
「observed=true という怪しい観測 1 発だけで実際に OS の IME が開いてしまう」
という、`last_intent` 汚染とは**別種**の問題が残っている（このコミットでは
未対処。次のセクション「未対処の指摘」参照）。

ユーザー報告ログの「キー入力が無い間に `Engine activated` が出る」バーストが
実際にどちらの経路（キーボード経由の echo による `last_intent` 汚染、または
`RefreshState` 経由の OS-level 無条件適用 + drift correction のせめぎ合い）で
起きていたかは、INFO レベルのログだけでは特定できていない。前者はこのコミットで
修正済み、後者は未修正のまま残っている。

旧 `DecisionOrigin`/`EffectOrigin`（`SetOpen` の発行元を区別する仕組み）は
2026-07-06 の到達不能パス監査で「消費者が存在しない」として撤去されていた
（`src/engine/decision.rs` 冒頭コメント参照）。今回、その消費者の一つ
（`kp_stage_post_decision` が origin で belief 更新経路を分岐する必要）が
実在することが判明したが、`execute_from_loop`（非キーボード経路）側には
まだ消費者を配線していない。

**修正（キーボード経路のみ。`RefreshState`/非キーボード経路は未修正、下記参照）:**
`ImeEffect::SetOpen` に `origin: SetOpenOrigin`（`ExplicitUserAction` /
`ActivationSync`）を追加。`transition_activation()` を呼ぶ4箇所のうち、
`check_active_transition`（毎キー入力・`RefreshState` の両方から呼ばれる汎用経路）
だけを `ActivationSync` とし、明示的なユーザー操作（IME ON/OFF コンボ・
エンジン ON/OFF コンボ・`ToggleEngine`/`ForceEngineOn` コマンド）を起点とする
残り3箇所は `ExplicitUserAction` のままにした。`awase-windows` 側
（`kp_stage_post_decision`、キーボード経路 `kp_run_inner` からのみ呼ばれる）は
origin に応じて分岐する:
- `ExplicitUserAction` → 既存の `handle_engine_set_open()`（`last_intent` を設定）
- `ActivationSync` → 新設の `handle_engine_activation_sync()`
  （`ImeEvent::EngineActivationSync` を dispatch。`PanicReset`/`HwndCacheRestored`
  と同じ「専用イベントで `last_intent` を設定しない」パターン。`desired_open` も
  一切書き換えない — 当初は `has_user_explicit_intent()==false` の間だけ書いて
  いたが、それでも `desired_open := effective_open()` という循環 echo になり、
  元の観測が期限切れで消えた後もこの値が恒久化してしまう問題が Opus レビューで
  指摘され、完全に書かないよう修正した。`last_explicit_ime_action_ms` は
  `handle_engine_set_open` と同様に更新する — 「明示的操作」ではなく「awase 自身が
  能動的に IME へ書き込んだか」を表すフィールドであり、この関数も実際に OS へ
  SetOpen を適用するため）。`lints/ime_event_guard` にも `EngineActivationSync` /
  `handle_engine_activation_sync` を登録し、`PanicReset`/`HwndCacheRestored` と
  同じ dylint 保護下に置いた（当初は登録漏れだった）。

**未対処の指摘（2026-08-04、Opus セカンドオピニオンレビューで判明、このコミットでは
未修正）:**
1. **`execute_from_loop`（`RefreshState`/非キーボード経路）は `origin` を見ない
   まま `SetOpen` effect を無条件に OS へ適用する**（`executor.rs::dispatch_effect`
   が `SetOpen { open, .. }` で `origin` を捨てている）。`ActivationSync` origin
   の SetOpen であっても、`last_intent` こそ汚染されなくなったが、**実際に OS の
   IME を開閉する副作用は防げていない**。単発の Medium confidence 観測だけで
   Engine が Active に遷移すると、その瞬間だけ物理的に IME が ON になり、
   `check_drift_correction`（次の ~500ms ポーリング tick）が気づくまでの間
   belief（`desired_open=false`）と実際の OS 状態（open）が食い違う spurious な
   ブリップが残る。ユーザーから見える症状（NICOLA がキーを一瞬取る等）を完全には
   防げていない可能性がある。対処するなら「`ActivationSync` origin の SetOpen は
   そもそも OS へ適用しない（`transition_activation` 自体が `ctx.ime_on` の
   confidence を見て抑制すべきか）」という設計判断が必要で、次セッションで検討する。
2. **`RefreshState` 経由の SetOpen が `execute_from_loop` へ流れ、
   `handle_engine_set_open`/`handle_engine_activation_sync` のどちらにも
   到達しない**（`ime: &ImeStateHub` が不変参照のため）ことが判明したため、
   本バグの当初の説明（下記「原因」セクション参照）にあった「`RefreshState` も
   `kp_stage_post_decision` を経由して `last_intent` を汚染する」は誤りだった。
   ユーザー報告ログの実際のバーストがキーボード経路（このコミットで修正済み）と
   `RefreshState` 経路（上記1、未修正）のどちらで起きていたかは特定できていない。

**未解明（実機ログのみでは特定できなかった点）:** 最初に `ctx.ime_on` が観測駆動で
true に振れる具体的トリガー（`last_intent` がその時点でなぜ空になっているか）は
INFO レベルのログだけでは確定できていない。`FocusChanged` が `last_intent` を
clear する既知経路はあるが、実機ログの再活性化バーストの直前に `FocusChange` ログは
出ていなかった。次回実機検証時に debug ログ（`[explicit-intent]`/
`[diag-engine-active]`/`[conv-open-inference]`）を有効化し、トリガーを特定すること。
あわせて上記「未対処の指摘」1・2 への対応も次セッションの課題とする。

**⚠️ テスト実行範囲に関する重要な注意（2026-08-04、2巡目の Opus レビュー中に判明）:**
`crates/awase-windows/src/state/mod.rs:80` の `#[cfg(windows)]` により
`state::platform_state`（`handle_engine_set_open`/`handle_engine_activation_sync`
を含む）モジュールまるごとが Windows 以外のターゲットではコンパイル対象から
除外される（`runtime::executor`/`runtime::key_pipeline` も同様に `cfg(windows)`
配下）。このためサンドボックス（Linux、wine 未導入）で `cargo test -p
awase-windows --lib`（target 指定なし = ネイティブ Linux ビルド）を実行しても、
**これらのファイルの `#[test]` は 1 件もコンパイルされず実行もされない**
（クリーンビルド後にカナリアテストで実証確認済み）。「271 passed」はいずれも
`state::ime_model`/`state::force_guard`/`state::observation_store`/`tsf::gji_fsm`
等の Windows 非依存な純粋ロジック部分のみを指しており、`platform_state.rs`/
`key_pipeline.rs`/`executor.rs` に追加した回帰テストの**実行による検証は
できていない**。`cargo check -p awase-windows --target x86_64-pc-windows-gnu
--tests` によるコンパイル確認（型検査・パターンマッチ網羅性等）のみが実施できる
検証であり、アサーションの実行結果までは保証しない。次の Windows 実機/wine
セッションで `cargo test -p awase-windows --target x86_64-pc-windows-gnu --lib`
相当を実際に実行して確認すること（wine 導入を試すか、実機で確認する）。

**テスト:** `src/engine/tests.rs`（Windows 非依存、実行確認済み）に
`refresh_state_transition_emits_activation_sync_origin_not_explicit_user_action`
（`RefreshState` 由来の遷移が `ActivationSync` を使うことを固定 — ただし上記の
とおりこの経路は `execute_from_loop` を通るため、このテストは「Decision の
effect が正しく tag されること」を固定するのみで、「`last_intent` が汚染
されないこと」自体は元々この経路では起きなかった）・
`ime_on_combo_emits_explicit_user_action_origin`（対照: IME-ON コンボは
`ExplicitUserAction`）を追加。`crates/awase-windows/tests/golden_scenarios.rs`
（Windows 非依存、実行確認済み）にシナリオ16・16b（`EngineActivationSync` が
ユーザーの明示 OFF 意図を汚染しないこと、本物のユーザー操作はそのまま
`last_intent` を確定させること）を追加 — ただしこれは `ImeModel::reduce()` に
直接イベントを流すレベルのテストで、`kp_stage_post_decision` の `match origin`
分岐自体（キーボード経路が正しいハンドラを呼ぶこと）を固定するテストは未整備。
`crates/awase-windows/src/state/platform_state.rs`（**上記の注意のとおり
`cfg(windows)` によりこのサンドボックスでは実行不可、コンパイル確認のみ**）に
`handle_engine_activation_sync_filters_when_focus_transition_was_pending`・
`handle_engine_activation_sync_applies_when_focus_transition_not_pending`・
`handle_engine_activation_sync_ctrl_chord_filter_still_works`（`handle_engine_
set_open` の同名テスト3本をコピペ実装した filter が乖離しても検知できるよう鏡写しで
追加）・`handle_engine_activation_sync_never_sets_last_intent_or_desired_open`
を追加。

**確認コマンドと実行可否:**
`cargo test -p awase --lib`（715 passed、Windows 非依存）・
`architecture_guard`/`golden_scenarios`（22 passed、シナリオ16含む。テキスト
走査/`ImeModel` 直接操作のみで Windows 非依存）・`cargo cc`（`-D
clippy::pedantic` 相当の厳格設定、`awase` クレートのみ）は実際に実行し green を
確認。`cargo test -p awase-windows --lib`（271 passed）も実行したが、上記の
注意のとおり本コミットの変更箇所は含まれていない。`cargo check -p awase-windows
--target x86_64-pc-windows-gnu --tests`・`cargo clippy --target
x86_64-pc-windows-gnu --lib -- -D warnings`・`cargo dylint --all -p
awase-windows -- --target x86_64-pc-windows-gnu`（`ime_event_guard`/
`observation_source_guard` 含め warning ゼロ、`EngineActivationSync` の
dylint 登録も確認）はコンパイル/静的解析レベルで green。wine 未導入のため
このサンドボックスでは `platform_state.rs`/`key_pipeline.rs`/`executor.rs`
のテスト実行・実機での再現・修正確認そのものは未実施（次の Windows 実機
セッションで、MS-IME/TSF native アプリで IME OFF 直後に Engine が勝手に ON へ
戻らないことを確認すること）。

**関連ファイル:** `src/engine/decision.rs`（`SetOpenOrigin`）、
`src/engine/engine.rs`（`transition_activation`/`check_active_transition`/
`apply_active_transition`/`apply_engine_on_with_ime_recovery`/
`build_ime_set_open_decision`）、`crates/awase-windows/src/state/ime_event.rs`
（`ImeEvent::EngineActivationSync`）、`crates/awase-windows/src/state/ime_model.rs`
（`reduce()`）、`crates/awase-windows/src/state/platform_state.rs`
（`handle_engine_activation_sync`）、`crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_stage_post_decision`）、`crates/awase-windows/src/runtime/executor.rs`
（`dispatch_effect` — `origin` を見ずに OS 適用する未対処箇所）、
`crates/awase-windows/src/runtime/ime_refresh.rs`（`ir_notify_engine_refresh`、
`RefreshState` の発火元）、`lints/ime_event_guard/src/lib.rs`。
関連: BUG-19（同型の「観測が意図を偽装する」バグ、`ime-belief-architecture.md` の
起点）、[[project_msime_dual_owner_bugs]]（2026-07-06 に同じ「IME OFF・Engine ON」
症状で調査したが未解決のまま残っていた不具合2）。

**追補（2026-08-04、同日別セッション）: 「未解明」トリガーの1つを特定・修正**

上記「未解明」節にあった「最初に `ctx.ime_on` が観測駆動で true に振れる具体的
トリガー」を、ユーザー提供の実機ログ（本バグの起票根拠と同一ログ）を
`crates/awase-windows/src/runtime/focus_tracking.rs`／`state/platform_state.rs`
のコードと突き合わせて特定した。

**原因:** `Ctrl+無変換`（デフォルトキーバインドの明示 IME OFF）は
`SpecialKeyMatch::ImeOff`（`src/engine/engine.rs:613-616`）→
`handle_engine_set_open` → `write_set_open_request` →
`ImeEvent::UserImeSetIntent{source: UserIntentSource::Command}` を発行する。
一方 `ImeStateHub::dispatch_event`（`platform_state.rs`）は、フォーカス変更を
またいで生存する永続タイムスタンプ `last_user_explicit_off_ms` を
`SyncKey`/`PhysicalImeKey` ソースのときしか更新しておらず、`Command` が対象から
漏れていた。`FocusChanged` は `ime_model.rs` で `last_intent` を無条件クリアする
ため、明示 OFF の数秒後にフォーカスが UWP 系中間ウィンドウ（`ForegroundStaging`・
`Windows.UI.Input.InputSite.WindowClass` 等、`Imm32Unavailable`）を経由すると、
`focus_tracking.rs` の cache-miss 分岐が `persistent_explicit_off_ms()`（常に 0）
を見て `EXPLICIT_OFF_CACHE_SUPPRESS_MS`（10秒）抑制ガードを発動できず、
`reset_stale_ime_on_for_imm_broken` が「明示的意図の証拠なし」と誤判定して
belief を Low confidence の `ObserverReported{open:true}` で ON に戻していた。
ユーザー報告ログの `06:23:19.814 IME OFF (key combo)` → 約11秒後
`06:23:30.845 AppKind changed: Uwp → Win32 (class=ForegroundStaging)` →
同時刻 `Imm32Unavailable entry without trusted cache: 安全デフォルト ON` →
`Engine activated` という並びと完全に一致する。

上記「原因」節が指摘した `transition_activation()` の対称 echo とは別経路
（キーボードから明示 OFF を押した後の *フォーカス遷移* が引き金）であり、
BUG-48 の主修正（`SetOpenOrigin` 分離）はこちらには影響していなかった
（`Command` ソースは修正後も `handle_engine_set_open` からのみ発行されるが、
それが「本物の明示操作」であることが確定した点はむしろこの修正の前提を
補強する）。

**修正:** `platform_state.rs` の `dispatch_event` にある永続タイムスタンプ更新の
`matches!(source, SyncKey | PhysicalImeKey)` に `Command` を追加。BUG-48 修正
（PR #44）により `Command` ソースは `handle_engine_set_open`
（`SetOpenOrigin::ExplicitUserAction`）経由でのみ発行される、つまり
「デフォルトキーバインドでの本物のユーザー操作」専用ソースになったため、
SyncKey/PhysicalImeKey と同列に扱ってよい。

**テスト:** `crates/awase-windows/src/state/platform_state.rs`
（BUG-48 と同じ理由でこのサンドボックスでは `cfg(windows)` によりコンパイル
確認のみ、実行不可）に `command_source_updates_persistent_explicit_off_ms`
（`Command` ソースの `dispatch_event` が永続タイムスタンプを更新/リセットする
ことを直接確認）・`handle_engine_set_open_updates_persistent_explicit_off_ms`
（Ctrl+無変換 が実際にたどる呼び出し経路をエンドツーエンドで確認）を追加。

**確認コマンドと実行可否:** `cargo test -p awase --lib`（715 passed）・
`cargo test -p awase-windows --lib`（271 passed、上記の理由でこのコミットの
変更箇所は含まれない）・`cargo test -p awase-windows --test golden_scenarios
--test architecture_guard`（22 passed）は実行し green を確認。
`cargo check -p awase-windows --target x86_64-pc-windows-gnu --tests`・
`cargo clippy -p awase-windows --target x86_64-pc-windows-gnu --lib -- -D
warnings` も green（`--tests` 付きの clippy pedantic warning は本修正と無関係な
既存debt、`platform_state.rs` を含まない）。実機/wine での動作確認は未実施。

**関連ファイル:** `crates/awase-windows/src/state/platform_state.rs`
（`ImeStateHub::dispatch_event`/`persistent_explicit_off_ms`）、
`crates/awase-windows/src/runtime/focus_tracking.rs`
（`EXPLICIT_OFF_CACHE_SUPPRESS_MS` 抑制ガード）。

## BUG-49: 小指シフト面（物理 Shift）の全角記号が `shift-conv-guard` の conv 書き込みと競合し半角化する（BUG-47 とは別原因、Phase 1・Phase 2 対応済み・実機未検証）

**症状（2026-08-05 ユーザー報告、MS-IME）:** `.yab` の `[ローマ字小指シフト]`
（物理 `VK_LSHIFT`/`VK_RSHIFT` を押しながら文字キーを打つ面）で「！」を入力すると、
全角「！」ではなく半角「!」が出力される。BUG-47 追補（`e99f20df`）適用後の
リビルドで再現することを実機ログで確認済み（`vk_pair_to_ascii` 経由で
`send_romaji_batched("!")` が正しく呼ばれていることをログで確認したため、
BUG-47 のcold-start保護漏れとは**別原因**と判明）。

**原因（確定、実機ログ + コード読解で確認）:** 物理 Shift 押下時、
`kp_shift_conv_guard_key_down`（`runtime/key_pipeline.rs`）が MS-IME の
「Shift単独タップで半角英数へ誤切替する」クセを打ち消すため、判別未確定
（チョードか単独タップか分からない）の時点で先回りして conv=0x0000
（半角英数）を IMC write する。この書き込みは実際に反映される
（150ms後の verify-read で確認済み）が、`ImeModeFsm`（`tsf/ime_mode_fsm.rs`）の
`confirmed` belief はこの時点で無効化されない（`unconfirm("shift-conv-guard
release")` は Shift **解放**時にしか呼ばれない）。この間にチョードが解決して
「！」が出力されると、`ms_ime_gate_defer` は stale な `is_native_ready()==true`
を信じて即時送信し、実際には半角英数モードのままの IME に Shift+1 の VK が
着弾して全角変換されず半角 `!` が出る。非同期 IMC write の着地（実測 ~250ms）と
チョード解決の競合であるため、**再現は確率的**。

**検討して却下した2案（再提案しないこと）:**
- **`ImeModeFsm::unconfirm()` を Shift 押下時にも呼ぶ案**: `ms_ime_gate_defer` は
  defer されるが、確認対象の conv は awase 自身が 0x0000 に保持しているため
  Shift を離すまで NATIVE 確認は原理的に成立しない。`MS_IME_READY_CONFIRM_MS`
  （400ms、打鍵時点起点）を超えて Shift を保持すると期限切れで結局半角化する上、
  `ms_ime_gate_give_up` がラッチされフォーカス変更まで MS-IME cold-start 保護
  （BUG-13）全体が無効化される。
- **全角記号21種を `build_symbol_to_vk`（`vk.rs`）から削除し Unicode 直接注入へ
  切り替える案**: `e99f20df` で全記号は既に `vk_pair_to_ascii` 経由で
  cold-start 保護・順序保証（`defer_vk_if_probe_in_flight`）・composition warmth
  追跡（`mark_composition_cold`）に合流済みであり、削除はこれらを丸ごと失う
  （「ば！」が「！ば」に順序逆転する等）。スコープも誤りで、対象記号の一部
  （？～（）｛｝）は親指シフト面（shift-conv-guard の対象外、現状正常動作）でも
  使われており、そちらまで巻き込んで壊す。

**恒久対応方針:** 個別パッチでは同種の反転を繰り返す構造的な問題（conv-mode
という共有状態への書き込みと belief キャッシュの無効化が不可分になっていない、
物理 Shift の意味づけが未確定なまま外部状態へ投機的に書き込んでいる、記号の
全角/半角が `.yab` の宣言ではなく IME の conv-mode に委譲されている）と判断し、
Opus・Fable・Codex の3系統に独立で北極星仕様の草案を依頼し統合した
[ADR-084](adr/084-conv-mode-single-ownership-and-width-ssot.md) を起票した。
今後この領域に触れる変更は ADR-084 の原則（P1〜P5・INV-1〜INV-10）と
整合させること。

**追補（Phase 1 実装、2026-08-05）:** ADR-084 §5 Phase 1 のうち、以下を実装した
（`kp_shift_conv_guard_key_down`/`kp_restore_kana_from_half_width`、
`crates/awase-windows/src/runtime/key_pipeline.rs`）。

- **同期的 belief 無効化（P1/INV-2 の部分適用）**: entry（MS-IME 経路で
  conv=0x0000 を IMC write する直前）で `ImeModeFsm::unconfirm("shift-conv-guard
  entry")` を **同期的に**（`spawn_local` の外、フックスレッド上で）呼ぶように
  した。これにより、entry write 後・restore 前にチョードが解決して記号が
  送られる場合、`ms_ime_gate_defer` は stale な `is_native_ready()==true` を
  信じず正しく defer するようになる（本バグの直接の修正）。
- **give-up latch の解除**: 同じ箇所で `ms_ime_gate_give_up` も解除する。新たな
  conv actuation が起きた以上、過去の期限切れ判定を持ち越す理由がないため。
- **INV-7（entry/restore の IME 種別対称化）**: `kp_restore_kana_from_half_width`
  の実際の OS 書き込み（`VK_DBE_HIRAGANA` 注入・IMC write リトライループ）を
  entry と対称に MS-IME 限定にした。従来は GJI でも無条件に実行しており
  （GJI は entry 機構が無いため常に「書いていないものを復元する」無意味な
  副作用だった）、Opus レビューで独立に「純粋な改善」と確認済み。

**⚠️ 実装したが撤回した部分（INV-9、再度試みる前に必ず読むこと）:** 当初
`MS_IME_READY_CONFIRM_MS` の期限を「打鍵時点」ではなく「`unconfirm` された
時点」起点に変更する案（`ms_ime_gate_defer` の `deadline_ms` 計算を
`unconfirmed_since() + 400ms` に変更）も実装したが、Opus レビューで
**この実装は却下済みの案A と同じ失敗をする**ことが判明し撤回した（コード上は
現在も元の「送信試行時点起点」のまま）。

理由: `unconfirmed_since()` は常に「送信試行時点」以前（entry write の瞬間）
であり、`unconfirm_time ≤ send_time` が常に成り立つため、
`unconfirmed_since() + 400ms` は `send_time + 400ms` より**早いか同じ**にしか
ならない。つまり期限を「早める」変更であり、Shift を長く保持するケースで
新たに失敗を生む（期限切れ → 強制送信で結局半角化 → `ms_ime_gate_give_up`
ラッチで BUG-13 保護まで無効化）。さらに、実装中に `unconfirmed_since_ms` の
リセットタイミングにもバグがあった（`on_conversion_mode_read` が NATIVE 以外の
状態を確認しても無条件に `0` へリセットしてしまい、Shift 保持中に conv=0x0000
を読み返すたびに「記録なし」に戻って defer の起点が振り出しに戻る）。

正しく実装するには、`unconfirmed_since` を**都度動的に再評価**する必要がある
（restore の `unconfirm("shift-conv-guard release")` が起点を後ろへ押し出す
効果を、`MsImeReadyCoro`/`start_ms_ime_ready_poll` の**両方**が固定値
（現状は defer 時点で一度だけ計算してコルーチンに埋め込む `deadline_ms: u64`）
ではなくポーリングの都度ライブに参照する形に変更する必要がある。`
TsfEnvSnapshot`（`tsf/warmup/probe_fsm.rs`）にタイムスタンプを乗せて
`ms_ime_ready_coro_body` 側からも見えるようにする案が有力だが、未実装）。
INV-9 のこの正しい実装は Phase 1 のスコープ外として次段に持ち越す。

**今回のフィックスで直っていない/確認できていない点:**
- 上記 INV-9 未実装のため、Shift を極端に長く（目安 400ms 超）保持したまま
  記号チョードを確定するケースでは、entry unconfirm による defer は効くが
  期限切れ→強制送信で結局半角化しうる。**期限の計算式自体（送信試行時点起点）は
  develop から変更していないが、「defer 分岐に入ること自体」が新規の副作用を
  持つ点には注意**: develop では entry unconfirm が無いためこの長押しケースは
  そもそも defer 分岐に入らず `MsImeReadyCoro`/`start_ms_ime_ready_poll` も
  起動しなかった（＝ `ms_ime_gate_give_up` が立つこともなかった）。今回の変更で
  defer 分岐に入るようになった結果、Shift 保持中は `on_conversion_mode_read` が
  常に conv=0x0000（`state=Off`）を確認し続けるため `is_native_ready()` が
  原理的に真になり得ず、400ms 超の保持は**決定論的に** `ms_ime_gate_give_up` を
  ラッチするようになった（Opus レビューで指摘）。実害は限定的で、このラッチは
  IME ON 遷移完了（`platform.rs`）・フォーカス変更（`output/mod.rs`）・次回の
  shift-conv-guard entry のいずれでも解除されるため BUG-13 本来の窓
  （IME OFF→ON 直後）は塞がず、Shift 解放直後の1文字目について restore 側の
  `unconfirm("shift-conv-guard release")` が本来担う再確認保護が一時的に
  効かなくなる程度に留まる。とはいえ「defer 分岐は無害な delay に過ぎない」と
  誤解しないこと。
- `MS_IME_READY_CONFIRM_MS`（400ms）は元々 IME OFF→ON 遷移の実測値
  （2026-07-06計測）であり、conv-mode 書き換え後の再確認に必要な時間の実測では
  ない。ADR-084 §5 Phase 1 の実測義務（`.claude/rules/tuning-constants.md`）は
  **未達**（このサンドボックスに Windows 実機が無いため測定不能。次の実機
  セッションで「entry の IMC write 完了 → IMC read で conv=0x0000 が確認できる
  までの実測 ms」と「restore の VK_DBE_HIRAGANA 注入/IMC write から NATIVE 再確認
  までの実測 ms」の双方を計測すること）。
- Shift の auto-repeat（`kp_shift_conv_guard_key_down` は新規押下と repeat を
  区別していない）が entry を再度発火させ、`unconfirm`/`give_up` 解除を
  typematic レートで繰り返す可能性を Opus レビューで指摘された。実害は
  「IMC 不可読環境で give-up latch が本来の目的（連続 probe 抑止）を果たせない」
  程度と評価しているが、実機で Shift の auto-repeat がこの関数に到達するか
  自体が未確認。
- 本修正（症状「！」→「!」の直接原因の解消）自体、wine 未導入のためこの
  サンドボックスでは実機再現・修正確認ができていない。次の Windows 実機
  セッションで MS-IME + 小指シフト面「！」の cold-start/hold 時間を変えた
  再現テストを行うこと。
- 上記に挙げていない conv 書き込み経路（`key_pipeline.rs` の idle-conv-check・
  ime-on-combo リセット、`executor.rs`、`tsf/warmup/cold_warmup.rs`、
  `ime_controller.rs` 等）は今回同期的 unconfirm 化していない（INV-2 は
  `shift-conv-guard` entry のみの部分適用）。ADR-084 の `actuate_conv_mode`
  chokepoint への統合は次段。

**テスト:** `crates/awase-windows/src/tsf/ime_mode_fsm.rs` に
`unconfirm_makes_native_ready_false_without_changing_state`（本バグが依存する
不変条件「unconfirm 後は state が Hiragana のままでも is_native_ready()==false」
を直接固定）・`unconfirm_is_idempotent`・
`on_conversion_mode_read_confirms_native_ready_again` を追加（Windows target
のみでコンパイル・実行可、`cargo check -p awase-windows --target
x86_64-pc-windows-gnu --tests` で型検査済み、wine 未導入のためこのサンドボックス
では実行不可）。`cargo test -p awase-windows`（Linux、golden_scenarios/
architecture_guard 含む）は無影響で全 green（本修正は `ImeModel`/reducer 層では
なく `ImeModeFsm`/`Output` 層のみに触れるため、既存の golden シナリオは
カバー範囲外）。

**関連ファイル:** `crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_stage_shift_conv_guard`/`kp_shift_conv_guard_key_down`/
`kp_restore_kana_from_half_width`）、`crates/awase-windows/src/tsf/ime_mode_fsm.rs`、
`crates/awase-windows/src/output/vk_send.rs`（`ms_ime_gate_defer`）、
`crates/awase-windows/src/output/probe_io.rs`（`start_ms_ime_ready_poll`）、
`layout/nicola.yab`（`[ローマ字小指シフト]`）。
関連: BUG-13（MS-IME cold-start 保護）、BUG-15（shift-conv-guard 導入経緯）、
BUG-25（左Shift単独タップ持続トグル、GJI entry 撤回）、BUG-47（記号 cold-start
半角化、本件と症状は類似だが原因は別）、BUG-58（追補2で導入した
`SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS` 5000ms 安全弁が、その安全弁自身の
前提を破って別のデッドロックを生んだ実例）、ADR-064（`ConvModePolicy`）、
ADR-072（conv authority 再同期）、ADR-078（belief 3分割）、ADR-083
（`InjectionMode` per-VK 統一investigation、NO-GO）。

**追補2（Phase 2 実装、2026-08-05）:** Phase 1 で撤回した INV-9（confirm-gate の
期限を固定値でなく都度動的に再評価する）の正しい実装。Phase 1 が残した実害
（「Shift を 400ms 超保持しただけで defer 分岐入り自体が決定論的に
`ms_ime_gate_give_up` をラッチする」点、上記「今回のフィックスで直っていない/
確認できていない点」参照）を解消する。

- **`Output::confirm_gate_deadline_override_ms`（`Cell<u64>`）**: confirm-gate
  の実効期限を `deadline_ms.max(override)` として消費側（`MsImeReadyCoro`/
  `start_ms_ime_ready_poll`）で評価する。`0` = 上書きなし（従来どおり）。
  entry（`kp_shift_conv_guard_key_down` の MS-IME 分岐）が hold 開始時に
  `SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS`（5000ms、有限キャップ）へ延長し、
  release（`kp_shift_conv_guard_key_up`）が hold 終了時点起点の
  `SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS`（800ms）へ差し替え、続く
  `kp_restore_kana_from_half_width` の冪等リトライループ（0/160/320/480ms）が
  進行中は毎試行この猶予を押し出し続ける。`u64::MAX` の真の無期限にしなかった
  理由: Shift の KeyUp が何らかの理由でフックに届かない場合（ロック画面・
  セキュアデスクトップ遷移等）でも、有限キャップを過ぎれば自動的に通常の
  安全弁（IMC 未確認なら give-up latch）へ復帰させるため。
- **`Output::shift_conv_guard_gen`（`Cell<u32>`）と所有権チェック付きヘルパー
  `extend_confirm_gate_override`/`clear_confirm_gate_override`/
  `bump_shift_conv_guard_gen`**: Opus レビュー（round 5、"pass-5"）で発見した
  blocking な競合の修正。hold #1 の解放直後に hold #2 が始まった場合
  （連続 Shift タップの通常の間隔で起こりうる）、hold #1 の detached
  `spawn_local` リトライタスクが自分の起動時点の世代を `owner_gen` として
  捕獲し、以後の全 override 書き込み（延長・クリア双方）の前提条件にする。
  世代不一致になった時点でそのタスクは即座に自分がもう override の所有者
  でないと分かり、hold #2 の override を誤ってクリア・上書きしない。世代は
  新しい hold の開始・フォーカス変更（`on_ime_mode_focus_changed`）・
  `SetOpen(true)` 適用（`platform.rs`、IME が OFF→ON へサイクルし conv=0
  前提が崩れる）・かな入力コンテキスト前提が崩れた早期 return（entry の
  ガード節）の 4 箇所で進める。
- Opus によるレビュー（初回 GO-WITH-CHANGES、実装後の pass-5 で上記 blocking
  な競合を発見、修正後の round 6 で GO-WITH-CHANGES・非 blocking な
  should-fix 2件（GJI 経路で対応する entry も retry ループも無いまま
  override だけ書き込まれる非対称、コメントの数値不整合）を指摘、反映済み）
  を経て確定。

**定数の実測根拠（`.claude/rules/tuning-constants.md`）:**
- `SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS = 800`: 実測値ではなく、`kp_restore_
  kana_from_half_width` のリトライ 1 試行分の最大所要時間からの導出。
  `set_ime_romaji_mode_with_target_async`（`ime.rs` の `modify_conv_mode`）は
  `IMC_GETCONVERSIONMODE`/`IMC_SETCONVERSIONMODE` を各 50ms タイムアウトで
  最大2回呼ぶ（最大 ~100ms）+ `RETRY_INTERVAL_MS`（160ms）の sleep +
  conv 読み取り（10ms タイムアウト）= 1 試行最大 ≈ 270ms。800ms はこれに
  対する ~2.8 倍のマージン（round 6 レビューで確認済み）。Phase 1 時点の
  MS-IME Shift 単独タップ誤切替の実測（shift up 後 ~478ms）は `MAX_TRIES`
  （4 回）× `RETRY_INTERVAL_MS` を決める根拠であり、この定数自体の根拠では
  ない点に注意（旧版のコメントはこの2つを混同していた）。
- `SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS = 5_000`: 実測値ではなく安全側
  マージン。通常の hold（Shift 押下から解放まで）は実機ログで ~620ms 程度
  （上記「今回のフィックスで直っていない/確認できていない点」の
  `MS_IME_READY_CONFIRM_MS` 実測未達の記述と同様、Windows 実機無しのため
  この 620ms 自体も過去セッションのログからの参照値であり、本セッションで
  再測定はしていない）。

**テスト（Phase 2）:** `output/mod.rs` に `extend_confirm_gate_override`/
`clear_confirm_gate_override`/`bump_shift_conv_guard_gen` の純粋ロジック
テスト5件（同一世代での書き込み成功・stale 世代での no-op を extend/clear
双方で固定 — pass-5 が発見した blocking バグそのものの再発防止）、
`tsf/warmup/ms_ime_ready_coro.rs` に `MsImeReadyCoro` が override 延長中は
`deadline_ms` が過ぎていても待機し続けることを固定するテスト2件を追加。
いずれも `output`/`tsf` モジュールが `#[cfg(windows)]` ゲート下にあるため
Windows target でのみコンパイル対象（`cargo check -p awase-windows --target
x86_64-pc-windows-gnu --tests` で型検査・`cargo clippy` で lint 済み、wine
未導入のためこのサンドボックスでは実行不可）。`cargo test -p awase-windows`
（Linux 実行分、golden_scenarios/architecture_guard 含む 273 件）は無影響で
全 green。

**追補3（2026-08-06、ADR-084 P1/INV-1 `actuate_conv_mode` chokepoint 導入・第一弾）:**
Phase 1 追補が「次段」として残した「ADR-084 の `actuate_conv_mode` chokepoint への
統合」に着手した。conv-mode を書き込む経路は本ファイルの洗い出しで最低6箇所
（`key_pipeline.rs` 4箇所・`cold_warmup.rs`・`executor.rs`）+ `VK_DBE_HIRAGANA`/
`ALPHANUMERIC` 直接送信（`ime.rs`・`ime_controller.rs`・`tsf/send.rs`）に散在して
いることを確認したが、本コミットは意図的に**最小スコープ**（`kp_shift_conv_guard_
key_down` の MS-IME entry 書き込み1箇所のみ）に絞った。

**理由:** `kp_restore_kana_from_half_width` の復元リトライループは
`shift_conv_guard_gen`/`owner_gen`/`confirm_gate_deadline_override_ms` と密結合
しており、Phase 2 追補にあるとおり Opus レビュー round 5〜6（"pass-5"）で発見・
修正された blocking な世代競合を含む、複数回のレビューを経てようやく確立した
挙動である。この領域は「IME OFF キー選択が5日間で6回反転した」
（`docs/experiments.md` エントリ01）のと同じ再発ファミリーであり、chokepoint への
機械的な一括移行は、その過程で世代管理の呼び出し順序を崩し新たな回帰を生む
リスクが高いと判断した。entry 側（比較的自己完結: ガード判定・unconfirm・
give-up 解除・spawn write のみで、世代管理を伴わない）から着手し、1箇所の移行で
`actuate_conv_mode` の実際の型・呼び出し規約を確定させることを優先した。

**実装:**
- `state/conv_mode.rs` に `ConvModeTarget`（`HalfWidthAlnum` のみ、他 variant は
  未移行呼び出し元の移行時に追加）・`ConvMutationReason`（`ShiftSoloTapCounter`
  のみ）・`ConvActuationOutcome`（`Rejected`/`Actuated`）を新設。ADR-084 §2 が
  提案する完全版のうち、実際に使う variant のみを定義し、憶測での API 先行拡張は
  避けた。
- `runtime/conv_actuation.rs`（新設）に `Runtime::actuate_conv_mode` を実装。
  ADR-064 の `conv_mutation_allowed` ゲート確認 → `ImeModeFsm::unconfirm`（INV-2、
  同期）→ `ms_ime_gate_give_up` 解除（同期）→ 実際の IMC write は非同期
  `spawn_local`、の順で行う。既存の `kp_shift_conv_guard_key_down` 内のインライン
  実装と**完全に同じ順序・同じログラベル**（`"shift-conv-guard entry"`）を保つ
  ことで、挙動変化ゼロの純粋なリファクタとして扱えるようにした。
- `kp_shift_conv_guard_key_down` の該当箇所（unconfirm 呼び出し + give-up 解除 +
  `spawn_local` での IMC write）を `self.actuate_conv_mode(ConvModeTarget::
  HalfWidthAlnum, ConvMutationReason::ShiftSoloTapCounter, now_tick)` 1行に置換。
  `confirm_gate_deadline_override_ms` の延長・`bump_shift_conv_guard_gen`・150ms
  診断用 verify-read は元の場所に残置（chokepoint の責務外、Phase 2 の世代管理
  ロジックに属する）。

**未移行（次段のスコープ、変更なし）:** `kp_restore_kana_from_half_width` の復元
リトライループ、`tsf/warmup/cold_warmup.rs::preamble`、`runtime/executor.rs`、
`kp_stage_idle_conv_check` のローマ字復元経路（`key_pipeline.rs` 内3箇所）は
`set_ime_romaji_mode_with_target_async` を直接呼び続けている。したがって
ADR-084 INV-1 が求める「低レベル API を private にしてこの関数だけが呼べるように
する」というコンパイラ強制は、これら全ての移行が完了するまで導入できない
（本コミット時点では `set_ime_romaji_mode_with_target_async` は従来どおり
`pub(crate)` のまま）。INV-11（conv 帰属/provenance、BUG-50 追補参照）も未着手。

**テスト:** `state/conv_mode.rs` に `half_width_alnum_target_maps_to_zero`
（`ConvModeTarget::HalfWidthAlnum` → `conv=0`）・`shift_solo_tap_counter_uses_
existing_unconfirm_label`（ログラベルが移行前と同一であることを固定）を追加
（Linux ネイティブで `cargo test -p awase-windows --lib conv_mode` 実行・確認済み、
12 passed）。`cargo test -p awase-windows --lib`（278 passed）・`--test
golden_scenarios --test architecture_guard`（22 passed）は無影響で全 green。

**検証状況:** `cargo check -p awase-windows --target x86_64-pc-windows-gnu`・
`cargo clippy -p awase-windows --target x86_64-pc-windows-gnu --lib -- -A
clippy::cargo_common_metadata -D warnings -W clippy::cognitive_complexity`
（CI の `clippy` ジョブと同じ引数、warning ゼロ）・`cargo xwin build --tests -p
awase-windows --target x86_64-pc-windows-msvc`（`windows-cross-check` ジョブ相当）
はいずれも green。`cargo dylint` はサンドボックスのディスク逼迫（作業中に
一時的に空き容量が数MBまで低下し `error: No space left on device` でリンクが
落ちる事態が発生、自worktreeのビルドキャッシュ削除で復旧）を踏まえてリスクを
避けるため見送った — 本コミットは `ImeEvent::PanicReset`/`HwndCacheRestored`/
`InputModeObserved`/`ConvBitsInference` のいずれも構築しておらず、既存の
`ime_event_guard`/`observation_source_guard` の検出対象パターンには触れていない。
wine 未導入のためこのサンドボックスでは実機相当の実行・確認は未実施
（entry 書き込みのログ順序・タイミングが変わっていないことのみコード読解で確認）。

**関連ファイル:** `crates/awase-windows/src/state/conv_mode.rs`
（`ConvModeTarget`/`ConvMutationReason`/`ConvActuationOutcome`）、
`crates/awase-windows/src/runtime/conv_actuation.rs`（新設、
`Runtime::actuate_conv_mode`）、`crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_shift_conv_guard_key_down`）。関連: ADR-084（P1/INV-1/INV-2）、
BUG-49 追補1・2（本追補の前提となった entry/restore の既存挙動）。

### BUG-49 追補3（2026-08-08）: `kp_restore_kana_from_half_width` の復元リトライループを ADR-086 INV-14（ターゲット同一性）へ移行

**内容:** `set_ime_romaji_mode_with_target_async`（ライブクエリ版、宛先を自己決定
する低レベル API）への直接呼び出しを、[ADR-086](adr/086-force-write-trigger-and-target-identity.md)
の `ActuationTarget::capture` → `set_ime_conv_for_target` 経由に置き換えた。

**BUG-49 領域への影響評価:** リトライループの既存タイミング制御
（`RETRY_INTERVAL_MS`=160ms・`MAX_TRIES`=4・`extend_confirm_gate_override`
による猶予延長・`shift_conv_guard_gen` を使った `still_owner` チェック）は
**一切変更していない**。`ActuationTarget::capture` はループの外（起案時点、
`owner_gen` 捕獲と同一の同期区間）で1回だけ呼び、全試行で同じ target を使い回す。
毎試行 capture する設計は当初検討したが、opus アドバーサリアルレビュー
（2026-08-08）で「capture と verify が数 ms 差のライブクエリ2連発になり
ほぼ確実に一致してしまい、INV-14 の検証が事実上 no-op 化する」と指摘され
不採用にした。

`ime_mode_focus_gen`（ADR-086 が使う世代）と `shift_conv_guard_gen`（本ループが
使う世代）は同一イベント（`Output::on_ime_mode_focus_changed`）で同時に bump
されるため、フォーカス変更は既存の `still_owner` チェックで先に検知される。
ADR-086 が追加するのは hwnd（空間軸）の検証のみで、時間軸のフェンスは
BUG-49 追補2の時点で既に存在していた。

`SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS`（800ms）のマージン導出（`tuning.rs`）に
`verify_still_current` の hwnd クエリ分（最大 ~30ms）を加算したが、マージン
比率が十分（2.7倍）なため定数値自体は実測なしに変更していない
（`.claude/rules/tuning-constants.md` 準拠）。

**検証:** cargo check/clippy --target x86_64-pc-windows-gnu --lib（警告ゼロ）、
cargo test -p awase-windows --test architecture_guard（17件pass）。
`kp_restore_kana_from_half_width` は実機フック依存で自動テスト不可
（BUG-25 に明記の既存制約）。**Windows 実機での動作確認は未実施。**

**関連ファイル:** `crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_restore_kana_from_half_width`）、`crates/awase-windows/src/ime.rs`
（`ActuationTarget`/`set_ime_conv_for_target`）、`crates/awase-windows/src/tuning.rs`
（`SHIFT_CONV_GUARD_RELEASE_CONFIRM_MS`）。関連:
[ADR-086](adr/086-force-write-trigger-and-target-identity.md) §2.3/§4 INV-14、
BUG-59 とその追補（本移行の発端）。

### BUG-49 追補4（2026-08-08）: 2回目 opus レビュー（F1〜F6）反映

**F1（High、修正済み）:** `kp_restore_kana_from_half_width` のリトライループが
NATIVE 確認（復元完了判定）に使っていた読み取りが、書き込みと同じ検証済み
`target` を経由しない生のライブクエリ（`get_ime_conversion_mode_raw_timeout`）
だった。write が `Aborted`（フォーカス移動で中止）でも、無関係な別ウィンドウ B
から偶然 NATIVE ビットが読めてしまえば「復元完了」と誤判定しかねない。
`get_ime_conv_for_target(target, 10)` に変更し、読み取りも書き込みと同じ
target を経由するようにした。

**F2（High、修正済み）:** `apply_focus_probe` 内の `ImmCrossProbe` かなモード
補正書き込み（romaji 復元）が、6箇所の移行対象一覧から漏れており
`ActuationTarget` を経由しない生の `set_ime_romaji_mode_async()` のままだった。
capture を先頭 await に置く専用の `spawn_local` へ切り出し、
`ActuationTarget::capture` → `set_ime_conv_for_target` 経由に移行した
（7箇所目の移行）。

**F3（High、修正済み）:** `on_ime_apply_complete` は `UnsafeToToggle`
（Win-held 等の genuine skip に加え、`ActuationOutcome::Aborted`/capture 失敗も
含む）で C/D だけでなく E（`post_ime_refresh`）まで早期 return でスキップして
いた。`Aborted(GenStale)`（同一ウィンドウのままフォーカス世代だけ進んだケース）
は意図（IME open/close）自体は有効なのに、これを再試行する自然なトリガー
（新しいフォーカス変更）が発生せず、次に無関係なイベントが来るまで無期限に
取りこぼされ得た。E だけは UnsafeToToggle でも必ず実行するよう
`runtime/mod.rs::on_ime_apply_complete` を修正し、20ms 後の refresh サイクルを
再試行の起点にした。

**F4（Medium、実機計測待ち）:** `ActuationTarget::capture`/`verify_still_current`
はそれぞれ独立に `get_focused_hwnd_async()`（内部で
`get_gui_thread_info_with_timeout(30ms)`）を1回呼ぶ。1回の actuation
（capture 1回 + verify 1回）で GUI query の予算は最大 30ms×2 = 60ms になる。
移行前、宛先を自己決定していたライブクエリ版（`set_ime_romaji_mode_with_target_async`
等、削除済み）は同種のクエリを1回だけ呼んでいた。往復回数が増えた分の
レイテンシ増分は Windows 実機で計測していない。`.claude/rules/tuning-constants.md`
に従い、実測なしに `30ms` の値自体は変更していない——ここに記録するのは
「予算構造が変わった」という事実と実機計測が必要という既知の未検証事項であり、
タスク #17（ADR-086 Phase1a→1b ゲート: 実機計測）が完了するまで未解決として扱う。

**F5（修正済み）:** 上記 F2 の修正で `apply_focus_probe` 内に新たな入れ子の
`spawn_local` ブロックが生まれ、`architecture_guard.rs` の
`actuation_target_capture_is_first_await_in_spawn_local_block` が入れ子ブロックを
検出できず誤判定した。ブロック抽出を再帰化（`collect_balanced_blocks`）し、
外側ブロックの「先頭 await」判定時に入れ子ブロックの中身をマスクする
（`mask_nested_needle_blocks`）よう修正。あわせて `{{`/`}}`（Rust フォーマット
文字列のエスケープ）を波括弧マッチングが誤カウントする問題（文字列リテラル非対応の
`find_balanced_close`）も、文字列リテラル対応の実装に置き換えて修正した。
いずれもミューテーションテスト（バグを一時的に再現 → 検出確認 → revert）で
検出できることを確認済み。

**F6（Low、修正済み）:** `ImmCrossOutcome` に `#[must_use]` を追加。
本追補の「16件pass」を実際のテスト数（17件）に修正。

**検証:** cargo check/clippy --target x86_64-pc-windows-gnu --lib（警告ゼロ）、
cargo test -p awase-windows --test architecture_guard（17件pass、mutation testing
で F1/F5 相当のバグ再現→検出→revert を確認）。**Windows 実機での動作確認は未実施。**

**関連ファイル:** `crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_restore_kana_from_half_width`/`apply_focus_probe`）、
`crates/awase-windows/src/runtime/mod.rs`（`on_ime_apply_complete`）、
`crates/awase-windows/src/ime.rs`（`get_ime_conv_for_target`/`ImmCrossOutcome`）、
`crates/awase-windows/tests/architecture_guard.rs`。関連:
[ADR-086](adr/086-force-write-trigger-and-target-identity.md) §4 INV-14、§7。

## BUG-50: 一度カタカナに入ると IME-ON コンボを押しても永久に復旧できない（デッドロック解消のみ対応済み、トリガー未確定）

**症状:** MS-IME（TSF-native、Windows Terminal / Chrome / UWP アプリ間でフォーカスが
頻繁に切り替わる環境）で、ユーザーが特にカタカナ切替キーを意識的に押した形跡が無い
まま IME がカタカナ変換モードに入り、通常の IME OFF→ON 操作では戻らなくなる
（2026-08-05 ユーザー報告「なぜか、カタカナモードになって、復旧の仕方がわからなく
なる」）。実機ログ抜粋:

```
[08:20:44.420] KeyInput(tsf): romaji="la"
[08:20:45.049] [conv-mode] Hiragana/kana → ZenKata/kana (conv=0x0000000B)   ← 説明のつかない切替
[08:20:46.917] Engine deactivated (reason=Inactive(ImeOff))
[08:20:48.493] [idle-conv-check] TsfNative: conv observation open=true reason=KatakanaShadowOff
                (conv=0x0000000B) → ObserverReported として記録 (engine は actuate しない)
[08:20:49.011] IME ON (key combo)                                          ← ユーザーが復旧を試行
[08:20:49.436] [ime-mode] drift detected: belief=Off → actual=Katakana (conv=0x0000000B)
[08:20:51.067] KeyInput(tsf): romaji="ki"                                   ← カタカナのまま入力継続
```

**IME:** MS-IME（TsfNative）。ただし後述の構造的デッドロックは IME/プロファイルに
依存しない汎用的な設計欠陥。

**原因1（確定、コード読解で確認）: 構造的デッドロック — 4つの個別に合理的なガードが
組み合わさり、一度カタカナに入ると自動でも手動単発操作でも戻れない状態を作る。**

1. `state/conv_classify.rs`（`KatakanaShadowOff` → `EngineSync::ReportOpenInference`）:
   観測は記録のみで `desired_open`/belief を書き換えない（BUG-19 再発防止として単体では
   正当）。
2. `tsf/ime_mode_fsm.rs`（`on_conversion_mode_read`）: belief≠actual の drift を warn
   ログに出すだけで、ON/OFF 側の `check_drift_correction` に相当する自動修正パスが
   conv には無い。しかも `is_native_ready()` は Katakana も「準備完了」扱いするため、
   msime-ready ゲートはカタカナのまま送信を素通しし続ける。
3. `ime_controller.rs`（`MsImeDirectStrategy::apply`）: 実 conv に KATAKANA ビットが
   立っていると「ユーザーの意図的な選択かもしれない」として `VK_DBE_HIRAGANA` 送信を
   意図的にスキップする（`AlreadyMatched`）。
4. `runtime/key_pipeline.rs`（`kp_reset_to_hiragana_romaji_capsoff` の起動条件）:
   唯一の明示的ひらがなリセット経路が `was_open_before`（IME open の belief）を必須と
   していた。本件のように belief が drift で誤って `Off` になっていると、実際の
   ユーザーの IME-ON コンボ操作があってもこのリセットが起動しない。

要するに「カタカナ = ユーザーの神聖な選択」（actuation 側、ガード3）と「カタカナ観測 =
信用しない」（belief 側、ガード1・2）という逆向きの保守判断が、出所（provenance）情報の
欠如によって同時成立し、ガード4がそれに輪をかけて手動復旧経路まで塞いでいた。唯一の
自動復旧経路は `panic_detect.rs` の `RapidPressTracker`（IME OFF→ON→OFF を2秒以内に
連打）だけであり、ユーザーはこれを知らないため発見不能な UX になっていた。

**原因2（未確定・実機検証待ち）: なぜ最初にカタカナへ入ったか（トリガー）。**
3つの仮説が残っており、いずれも今回のログ（INFO レベルのみ）だけでは確定できない
（該当する診断ログは debug レベルで出力されておらず、切り分けには debug ログでの
実機再現が必要）:

- **仮説A（無変換単独タップ×MS-IME既定のかな切替）:** `msime_key_assignment.rs` は
  awase が無変換/変換の単独タップを OS へ素通しすることを明記しているが、
  `conflict_warning()` は `KeyAssignmentMuhenkan`/`KeyAssignmentHenkan` が
  非既定値（IME オフ/オン割当て）の場合のみ警告し、**既定値（`0` = かな切替）は
  無害として警告対象外**にしている。2026-07-06 に見つかった「無変換=IMEオフ」二重
  オーナー問題と同一クラスの衝突が、既定設定のケースだけ検出漏れしている可能性。
- **仮説B（shift-conv-guard の Shift 解放漏れ）:** `kp_restore_kana_from_half_width`
  の呼び出し4箇所のうち3箇所（`key_pipeline.rs` の `TurnOn`/IME ON/post-decision
  `SetOpen(true)`）と `runtime/ime_refresh.rs`（`FocusChanged`）は
  `prepend_synthetic_shift_up=false` で呼ばれる。この状態で物理 Shift が実際に
  押されたままの瞬間に scan 付き `VK_DBE_HIRAGANA` が注入されると、MS-IME 純正の
  「Shift+かなキー＝カタカナ切替」に化ける可能性（`kp_shift_conv_guard_key_up` の
  唯一の `true` 呼び出しも generic `VK_SHIFT` up を使うため、右 Shift 保持時は
  解除されない別の穴もある）。
- **仮説C（`ConvModeMgr` の observed/desired 未分離）:** `state/conv_mode.rs` の
  `ConvModeMgr::mode`（`Cell<Option<ConvMode>>`）は「観測ログ用の確定値」と
  「shift-conv-guard 復元時の書き戻しターゲット」を同一の `Cell` で兼用しており、
  一度誤って確定した観測値がそのまま復元先として自己強化されうる
  （`key_pipeline.rs` の shift-conv-guard 復元処理が `conv_mode.get().imm_conv_target()`
  を書き戻す箇所）。

**修正（原因1のみ、Phase 1）:** `runtime/key_pipeline.rs::kp_stage_post_decision` の
ひらがなリセット起動条件を、`was_open_before`（belief）に加えて `ConvModeMgr` が
現に観測しているカタカナ（`conv_mode.get().charset.is_katakana()`）でも起動するよう
拡張した。これは新しい破壊的動作ではなく、「IME-ON コンボが既にカタカナを含めて
ひらがなへ寄せる」という既存の確立済み挙動（`was_open_before=true` のとき）を、
belief が drift で誤って `Off` になっているケースにも一貫させるだけ。トリガー
（原因2）が仮説A〜Cのどれであっても、このデッドロック解消は独立に効く。

**未解決（原因2、フォローアップ候補）:** ADR-084 に「conv 帰属（provenance）」
invariant を追補し、`ConvModeMgr` の確定値が `UserOriginated` か
`Attributed{by: awase}` かを区別できるようにすることで、ガード3
（`AlreadyMatched` スキップ）を「本当にユーザーが選んだカタカナ」にのみ適用し、
内部の誤 belief 起源のカタカナは自動是正できるようにする方向性が有力（後述の
ADR-084 追補参照）。仮説A〜Cのどれが実際のトリガーかは実機の debug ログでの
再現が必須。

**テスト:** `tests/ime_key_sequence_golden.rs` に、belief=Off かつ観測 conv が
カタカナの状態で IME-ON コンボを押すとひらがなリセットが起動することを固定する
golden ケースを追加（Linux ネイティブで `cargo test -p awase-windows` 実行可能）。
実機（MS-IME/TSF-native、Windows Terminal 等)での動作確認は未実施。

**関連ファイル:** `crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_stage_post_decision`/`kp_reset_to_hiragana_romaji_capsoff`）、
`crates/awase-windows/src/state/conv_mode.rs`（`ConvModeMgr`）、
`crates/awase-windows/src/state/conv_classify.rs`（`KatakanaShadowOff`）、
`crates/awase-windows/src/tsf/ime_mode_fsm.rs`、
`crates/awase-windows/src/ime_controller.rs`（`MsImeDirectStrategy`）、
`crates/awase-windows/src/msime_key_assignment.rs`、
`crates/awase-windows/src/panic_detect.rs`（既存の唯一の自動復旧経路）。
関連: BUG-19（同じ conv-mode 誤確定の系統）、ADR-084（conv-mode 単一所有権と
幅 SSOT 原則、本件の provenance 追補先）、
[fix-requires-evidence](../.claude/rules/fix-requires-evidence.md)、
[experiment-logging](../.claude/rules/experiment-logging.md)。

**追補（2026-08-06、原因1のガード3を根治・ユーザー指摘）:** 原因1で列挙した4つの
ガードのうち「ガード3」（`ime_controller.rs::MsImeDirectStrategy::apply` の
`AlreadyMatched` スキップ）を、provenance 追跡（未解決節で提案していた方向）ではなく
**そもそもの前提を取り除く形**で解消した。

ユーザーから「なぜひらがな/カタカナのキーを送る必要があるのか、代わりに IME OFF/ON
（開閉）を使えないのか」という指摘を受けて調査した結果、これは実は以前一度
「半分だけ」直されていた問題だと判明した: `MsImeDirectStrategy` の OFF は
`48a667a`（2026-06-27頃）で `VK_DBE_ALPHANUMERIC`（モード選択キー、「半角英数
＝IME ON のまま」という誤った意味論で確定 Enter 回数がズレる不具合があった）から
`VK_IME_OFF`（真の開閉キー、DirectInput へ）へ既に移行済みだった。ところが ON 側は
`VK_DBE_HIRAGANA`（モード選択キー）のまま取り残されていた。GJI 向けの
`GjiDirectStrategy` は元から ON/OFF とも `VK_IME_ON`/`VK_IME_OFF` を使っており、
これが TSF-native アプリで問題なく動作することは実証済み（Windows Terminal・Chrome
含む）。

**なぜガード3が消えるか:** `VK_DBE_HIRAGANA` は「IME を開く」と「ひらがなへ強制する」
という2つの副作用を1つのキーに束ねていた。この束ねが、現在カタカナのときに送ると
カタカナを壊す → 壊さないために送信をスキップするガードが要る → そのガードが
「ユーザーの意図的なカタカナ」と「内部の誤った/一時的なカタカナ」を区別できない、
という原因1の連鎖そのものを生んでいた。`VK_IME_ON` は conv-mode（ひらがな/カタカナ・
全角/半角のいずれのビットも）に一切触れない「開くだけ」のキーのため、この束ねが
存在せず、ガード自体が構造的に不要になる。

**修正:** `state/key_sequence_policy.rs::ime_key_for(MsImeDirect, Open)` を
`VK_DBE_HIRAGANA` → `VK_IME_ON` に変更（宣言的テーブルの1行 diff）。
`ime_controller.rs::MsImeDirectStrategy::apply` からガード3（conv 読み取り + KATAKANA
ビットチェック + `AlreadyMatched` return）を削除。ROMAN ビット pre-mode
（`set_ime_romaji_mode`、かな入力の JIS かな化け防止、ガード3とは無関係の既存ロジック）
は維持。

**残る影響（原因2は本追補の対象外）:** 「なぜ最初にカタカナへ入ったか」（原因2、
仮説A〜C）は未解決のまま。ただしガード3が消えたことで、原因2のどの仮説が真であっても
—— 一度カタカナに（誤って、あるいは正当に）入った後、次に IME-ON 経路を通れば
`VK_IME_ON` が送られ、conv-mode は変更されないまま IME が開く。これは「カタカナを
強制的にひらがなへ戻す」わけではないが、「デッドロックする」こともない。むしろ
原因1節にあった「ガード4」（`kp_reset_to_hiragana_romaji_capsoff` の `was_open_before`
依存）が引き続き機能する経路では、IME-ON コンボ押下で明示的にひらがなへリセットされる
（BUG-50 Phase 1 で `observed_katakana` 条件を追加済み）。

**未対応:** ADR-084 の `actuate_conv_mode` chokepoint（INV-11 provenance 含む）は
本追補の対象外。ガード3自体が消えたため provenance によるガード3の高度化はもはや
不要になったが、conv-mode を書き込む他の経路（`kp_restore_kana_from_half_width` 等、
BUG-49 追補3参照）の集約は引き続き有効な将来課題として残る。

**テスト:** `state/key_sequence_policy.rs::tests::ms_ime_direct_keys` を
`VK_DBE_HIRAGANA` → `VK_IME_ON` の期待値に更新。`tests/ime_key_sequence_golden.rs`
の `KEY_DOC`／`tests/golden/ime_key_sequences.txt` を同じ内容に同期
（Windows target でのみコンパイル・実行対象、`cargo check -p awase-windows --target
x86_64-pc-windows-gnu --tests` で型検査済み、wine 未導入のためこのサンドボックスでは
実行不可）。`cargo test -p awase-windows --lib`（278 passed）・`--test golden_scenarios
--test architecture_guard`（22 passed）は無影響で全 green（`key_sequence_policy`
モジュール自体が `#[cfg(windows)]` ゲート下にあり Linux ネイティブでは 0 件しか
コンパイル対象にならないため、その回帰確認はコンパイル検査止まり）。

**検証状況:** `cargo check`/`cargo clippy -p awase-windows --target
x86_64-pc-windows-gnu --lib -- -A clippy::cargo_common_metadata -D warnings -W
clippy::cognitive_complexity`（CI `clippy` ジョブと同じ引数、warning ゼロ）、
`cargo xwin build --tests -p awase-windows --target x86_64-pc-windows-msvc`
（`windows-cross-check` ジョブ相当）いずれも green。wine 未導入のためこのサンドボックス
では実機相当の実行・再現確認は未実施。次の Windows 実機セッションで、MS-IME/TSF-native
（Windows Terminal 等）で IME OFF→ON が引き続き正しく機能すること、カタカナ入力中に
IME を OFF→ON しても（意図的か否かを問わず）conv-mode が変化しないことを確認すること。

**関連ファイル（追補）:** `crates/awase-windows/src/state/key_sequence_policy.rs`
（`ime_key_for`）、`crates/awase-windows/src/ime_controller.rs`
（`MsImeDirectStrategy::apply`）、`crates/awase-windows/tests/ime_key_sequence_golden.rs`、
`crates/awase-windows/tests/golden/ime_key_sequences.txt`。関連: `48a667a`
（OFF 側の同種修正、本追補はこれと ON 側を対称化した）。

## BUG-51: TsfNative の drift correction が `TIMER_IME_REFRESH` の恒久停止で再起動されず、IME OFF で Engine ON のまま最大8分ドリフトする

**症状:** MS-IME（TsfNative）環境で、実機ログに `[ime-off-rescue] 50ms timer expired →
保留 vk=0x1D を IME OFF として発火`（vk=0x1D は `VK_NONCONVERT`＝既定の左親指シフト
キー）が2回記録された（2026-08-04T01:54:37.280 / 01:54:47.730）。その直後から
`[idle-conv-check] TsfNative: conv observation open=true reason=KatakanaShadowOff
(conv=0x00000003) → ObserverReported として記録 (engine は actuate しない)` が
継続的に記録され続け、desired_open=false（IME OFF 済みという engine 内部の認識）と
実際の TSF conv mode（open=true、HanKata）が乖離したまま推移した。最終的に
01:55:06.154 の `Engine activated` 直後、`[ime-mode] drift detected: belief=Off →
actual=Katakana (conv=0x00000003)` というログで乖離が可視化された。この間に記録
された `[drift] correction: observed=true ≠ desired=false for 478988ms → \
set_ime_open(false)`（01:54:58.067）は、乖離が **478988ms（約8分）** 続いていた
ことを示す。ユーザー報告は「IME OFF で Engine ON」（実際は逆方向、desired=OFF・
実 IME=ON のまま Engine だけが再度 ON になった状態）で、Shift キーの関与をユーザー
自身が疑っていた。同種の長時間ドリフト（`for 122751ms`）は BUG-50（カタカナ
ドリフトからの復旧不能）の実機ログでも観測されており、無関係な症状に見えても
根はこの再起動漏れに繋がっている可能性がある。

**IME:** MS-IME（TsfNative、`ime=MsIme`）。config 既定値: `left_thumb_key="無変換"`
（vk=0x1D）、`keys.ime_off=["Ctrl+無変換"]`。

**当初の誤診断（Fix A、撤回済み）と訂正の経緯:** 最初は「`hook.rs` の
`CTRL_CONSUMED_SINCE_DOWN`（`Ctrl↓ → 他キー↓ → 親指キー↓` を検知して
ime-off-rescue の 50ms 救済窓に入れる仕組み）が Shift 単独押下を「Ctrl チョード消費」
に誤カウントしている」ことが根本原因であり、修飾キー単独押下を消費対象から除外すれば
直ると判断し実装・テストまで行った（`vk.rs::counts_as_ctrl_consumption` 新設）。
しかし Opus・Fable による独立レビューで「`ctrl_consumed_since_down()` は
`key_pipeline.rs::kp_run_inner` Phase B で **即時発火するか 50ms 猶予後に発火するかを
分けるだけのゲート** であり、`engine.on_input`（Phase 1、`check_special_keys` が
NicolaFsm 処理より先に無条件実行される、`engine.rs:267-275`）は
`event.modifier_snapshot.ctrl=true` かつ vk が ime_off コンボの vk と一致すれば
`ctrl_consumed_since_down()` の値に関係なく IME OFF を即座に発火する」と指摘された。
`key_pipeline.rs:164-165` の既存コメント（「Ctrl↓ → 直後に 無変換↓ の意図的チョードでは
ctrl_consumed_since_down=false なのでここを通過せず engine が即 IME OFF を発火する」）
と `engine.rs:260-294` の `on_input` 本体を自分でも読み直し、この指摘が正しいことを
確認した。つまり Fix A は「50ms 救済窓に入るかどうか」を変えるだけで、**その先で
IME OFF が発火すること自体は防げない**。むしろ「Ctrl↓ → Shift↓/↑ → 無変換↓ →
50ms 以内に Ctrl↑」という、修正前なら 50ms 猶予中の Ctrl↑ で破棄されていた
（IME OFF もならず thumb shift 化もしない）パターンが、修正後は猶予なしの即時発火に
変わる退行があるとも指摘された。この経緯から Fix A は撤回し、`hook.rs`/`vk.rs` の
変更は本コミットに含めていない。**同種の「修飾キー単独押下の除外」案を再提案する前に
この経緯を必ず確認すること** — `ctrl_consumed_since_down` を弄る修正は、IME OFF の
発火有無ではなく発火タイミングにしか効かない。

**真因（コード読解で確認、Opus/Fable 双方が同じ箇所を有望と評価）:** IME OFF コンボ
自体の発火（Ctrl が真に見えている限り止められない）は今回は不問とし、発火後に
**乖離が検出・補正されるまでの時間が無期限になりうる** 構造的欠陥を直した。
`VK_NONCONVERT`（0x1D）は `vk::may_change_ime` が `false` を返すため、
SetOpen 発行の後処理（`kp_stage_post_decision`）の `may_change_ime` パススルー分岐
（`schedule_ime_refresh(20)`）はそもそも `!decision.is_consumed()` が
前提であり IME OFF の `Decision::consumed_with(..)` には到達しない。かつ同分岐より
前で `TIMER_IME_REFRESH` は明示的に kill される。TsfNative アプリでは
`Runtime::reschedule_ime_refresh` が `is_tsf_native` を理由に周期ポーリングの
再スケジュールを恒久的に停止する設計（「フォーカス変更 / may_change_ime キー」が
再開トリガーという前提）のため、この経路にはどちらのトリガーも来ない。結果、
`ir_apply_drift_correction`（`ime_refresh.rs`、`TIMER_IME_REFRESH` 発火時のみ実行
される）が長時間呼ばれず、`desired_open=false` と実 TSF conv（open=true）の乖離が
検出も補正もされないまま蓄積する。乖離自体は `kp_stage_idle_conv_check`（KeyDown 毎に
独立実行、`EngineSync::ReportOpenInference` 経由で observation store には記録される
が `desired_open` は書き換えない、BUG-19 対策）により observation としては記録され
続けるため、`[drift] correction:` ログの `duration_ms` が単調増加し続ける（BUG-43 と
対称に「検知タイマー自体が止まる」パターン）。この early-return は
`is_tsf_native` だけでなく `explicit_intent().is_some()` 全般に効くため、対象は
TsfNative に限らず explicit_intent が確定する全プロファイルに及ぶ。

**検討したが撤回した修正案（第1版・第2版）:** 本節は同じ近道を将来再提案しないための
記録（[experiment-logging](../.claude/rules/experiment-logging.md) の精神に準拠）。

- **第1版（撤回）:** `hook.rs` の `CTRL_CONSUMED_SINCE_DOWN`（`Ctrl↓ → 他キー↓ →
  親指キー↓` を検知して ime-off-rescue の 50ms 救済窓に入れる仕組み）が Shift 単独
  押下を「Ctrl チョード消費」に誤カウントしていることが根本原因と判断し、修飾キー
  単独押下を消費対象から除外する純粋関数 `vk.rs::counts_as_ctrl_consumption` を
  実装・テストまで行った。しかし Opus・Fable 双方の独立レビューで
  「`ctrl_consumed_since_down()` は `key_pipeline.rs::kp_run_inner` Phase B で
  即時発火するか 50ms 猶予後に発火するかを分けるだけのゲートであり、
  `engine.on_input`（Phase 1、`check_special_keys` が NicolaFsm 処理より先に
  `engine.rs:267-275` で無条件実行される）は `event.modifier_snapshot.ctrl=true`
  かつ vk が ime_off コンボの vk と一致すれば `ctrl_consumed_since_down()` の値に
  関係なく IME OFF を即座に発火するため症状を直さない」と指摘され、`engine.rs` を
  自分でも読み直してこの指摘が正しいことを確認した。さらに「Ctrl↓ → Shift↓/↑ →
  無変換↓ → 50ms 以内に Ctrl↑」という、修正前なら救済されていたパターンが修正後は
  猶予なしの即時発火に変わる退行があるとも指摘された。撤回し、`hook.rs`/`vk.rs` の
  変更は最終的に含めていない。
- **第2版（撤回）:** IME OFF コンボの発火自体は止めず、`state/platform_state.rs`
  に `has_pending_drift(now)` という純粋メソッドを新設し、`runtime/mod.rs::
  reschedule_ime_refresh` がドリフト進行中は TsfNative 等の早期 return より先に
  ポーリングを継続するよう変更し、さらに `kp_stage_post_decision` の SetOpen 処理で
  `TIMER_IME_REFRESH` kill 直後に `EXPLICIT_IME_SUPPRESS_MS`（1500ms）後の確認
  refresh を追加した。第2ラウンドの Codex・Opus・Fable 独立レビューで複数の欠陥が
  指摘された: (a) `EXPLICIT_IME_SUPPRESS_MS` と `DRIFT_CORRECTION_OBS_MAX_AGE_MS`
  が共に 1500ms のため、確認 refresh が発火する頃には唯一存在する trusted 観測が
  ちょうど max_age を超えて stale 判定され、補正が発火しない確率が高い（Opus 指摘）。
  (b) `has_pending_drift` が `check_drift_correction` の持つ閾値・鮮度・
  `is_user_enabled` 等のガードを一切持たないため、TsfNative + MS-IME のように
  drift を「収束」させる観測ソースが無いプロファイルでは drift が clear されず、
  一度ドリフトが立つとフォーカス変更まで `ime_poll_interval_ms` 間隔のポーリングが
  恒久的に回り続ける（Codex・Opus・Fable 全員が同じ懸念に到達）。(c) この 1.5 秒
  one-shot は `may_change_ime` の `schedule_ime_refresh(20)` や focus debounce 等、
  同じ `TIMER_IME_REFRESH` を使う既存の呼び出しに無条件上書きされて消えることがある
  （Codex・Fable 指摘）。Opus・Fable 双方が共通して「`kp_apply_conv_engine_sync` の
  `ReportOpenInference` 記録直後に `schedule_ime_refresh` を呼ぶ event-driven 方式」
  を代替案として推奨しており、これが最終版（下記）の設計になった。

**修正（本コミット、最終版）:** `runtime/key_pipeline.rs::kp_apply_conv_engine_sync`
の `EngineSync::ReportOpenInference` 分岐（`kp_stage_idle_conv_check` から KeyDown
毎に呼ばれる）で、`report_conv_open_inference(true, reason, now_tick)` を呼んで
新しい観測を記録した直後に `self.schedule_ime_refresh(20)` を追加した
（`may_change_ime` パススルーと同じ既存の 20ms 遅延を再利用、新規タイミング定数は
導入していない）。この分岐は「shadow=OFF なのに conv が native/open を示す」という
まさにドリフトそのものを検出した瞬間にのみ実行されるため、これまでのように
`desired_open` を直接書き換えることなく（BUG-19 対策の設計はそのまま維持）、
既存の `ir_apply_drift_correction`／`check_drift_correction`（閾値・鮮度・
`FeedbackPolicy::Blind` の再送上限等、既存のガードをすべてそのまま活用）に
「たった今できたばかりの新鮮な観測」を渡して判断させることができる。第2版の
問題点はすべてこの設計で構造的に解消される: 観測が新鮮なので stale 判定されない
（問題a）、ドリフトが実際に検出されたときだけ 20ms 後に1回チェックが走るだけで
継続的なポーリングにはならない（問題b、`reschedule_ime_refresh` 自体は完全に
未変更のまま）、次に同じ矛盾した観測が来るたびに再度 20ms 後のチェックが自然に
かかる（ユーザーが入力を続けている限り再発しても数十〜数百ms 単位で捕捉される）。
`runtime/mod.rs::reschedule_ime_refresh` はコメントを更新しただけで、判定ロジックは
変更していない（発火する分岐は元のまま）。

**未解明（Fable 指摘、未検証）:** そもそも最初の IME OFF 送信（VK 送信）が、なぜ
実 IME（TSF conv mode）に反映されなかったのか自体は今回のコード読解だけでは
分からない。`desired_open=false` は正しく確定しているのに実 conv が open のまま
だった理由（TSF composition による無視、chord barrier での skip、他の何か）が
未解明であり、本修正は「乖離を早く検知・再送する」対症であって、初回送信が反映され
ない根本原因には踏み込んでいない。また、`EXPLICIT_IME_SUPPRESS_MS`（1500ms）の間は
`kp_stage_idle_conv_check` 自体が抑止されるため、SetOpen 直後〜1500ms の間に
ユーザーが入力を止めた場合、次の KeyDown（＝次の idle-conv-check 実行）まで本修正の
トリガーは発火しない。ただしこれは既存の抑止設計そのものであり、タイピングが
再開されればすぐに捕捉される。次回同症状が再発した場合、初回 SetOpen 送信が実際に
何を送り、なぜ効かなかったかを VK レベルで実機ログ確認すること。また 8 分の乖離の
起点（ログの `duration_ms=478988ms` から逆算すると 01:46:49 付近）に対応する最初の
トリガーのログが今回の抜粋には無く、同じ経路だったかは未検証。

**未修正の残課題:** IME OFF コンボそのものの誤発火（Ctrl が真に見えている間は
`engine.on_input` Phase 1 が無条件でマッチする）は今回触れていない。Ctrl 状態の
信頼性問題（stuck Ctrl 等、`project_ctrl_mismatch_stuck_modifier` 参照）が真因の
可能性があるが、`modifier_snapshot.ctrl`（GetAsyncKeyState 由来）と物理キー状態
（`is_physical_key_down`）の一致を要求する等の対策は未検討・未実装。

**検証状況:** `runtime/key_pipeline.rs`・`runtime/mod.rs` は `#[cfg(windows)]` の
ため Linux 上の `cargo test -p awase-windows`（native target）には含まれず、
`cargo xwin check --tests -p awase-windows --target x86_64-pc-windows-msvc` で
型検査のみ確認済み（警告ゼロ）。`cargo xwin clippy -p awase-windows --target
x86_64-pc-windows-msvc`（`--tests` を除く、警告ゼロ）でも確認済み。wine 未導入の
ためこのサンドボックスでは実機相当のテスト実行は未実施。Codex CLI・Opus・Fable に
よる2ラウンドの独立レビュー（第1版・第2版それぞれで correctness を検証、指摘の
突合せ・自分でのコード再確認済み）を経て最終版に到達している。この修正自体は
2026-08-04 に別ブランチ（`fix/ime-off-rescue-modifier-consumed`）で完成していたが、
develop への統合が漏れたまま同ブランチが ADR-084/BUG-49 等の後続開発に取り残され、
2026-08-05 に BUG-50 の調査中に発覚して本エントリとして develop 最新へ移植した
（ブランチ全体をマージすると 2900 行超の後続修正を巻き戻すため、この1点修正のみを
再適用）。

**関連ファイル:** `crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_apply_conv_engine_sync`）、`crates/awase-windows/src/runtime/mod.rs`
（`reschedule_ime_refresh`）、`crates/awase-windows/src/runtime/ime_refresh.rs`
（`ir_apply_drift_correction`/`check_drift_correction`）。
関連: BUG-19（`desired_open` を書き換えない設計の由来）、BUG-20（OFF方向drift
correctionの修正）、BUG-43（対称に「検知タイマー自体が止まる」パターン）、BUG-50
（同種の長時間ドリフトが観測されたカタカナ復旧不能バグ）、
[experiment-logging](../.claude/rules/experiment-logging.md)、
[tuning-constants](../.claude/rules/tuning-constants.md)。
## BUG-52: `PhysicalKeyDisposition::plan` が `VK_DBE_KATAKANA` の KeyDown を「shadow_toggle 不発なら安全」として素通しし、MS-IME が仕様通りカタカナへ切り替わる

**症状:** WindowsTerminal（Cascadia、GJI/MS-IME、`AppImeProfile::TsfNative`）で
NICOLA の物理「IME ON」キー（scan 0x70）を連打しているうちに、何もしていないのに
入力中の文章が突然カタカナへ変わる（ユーザー通報「また謎にカタカナになる事象が
再発しました」、2026-08-05）。当初「MS-IME 側の自然発生ドリフト」と誤診したが、
`RUST_LOG=debug` の実機ログで物理キーイベントを直接確認した結果、awase 自身の
suppress 漏れであることが判明した。

**再現手順（実機 debug ログで確認済み）:**

```
[hook] IME-mode vk=0xF1 up  self_injected=false injected=false scan=0x70   ← 物理、KeyUp のみ可視
[hook] IME-mode vk=0xF2 down self_injected=false injected=false scan=0x70  ← 同じ物理キーの次打鍵
[imm32-off] key suppress vk=0xf1 KeyUp (physical disposition)
[shadow-toggle] intent 昇格: vk=0xF2 ... action=TurnOn
...（この直前、可視範囲外で 0xF1 の KeyDown が Allow され実IMEに届いていたはず）
[conv-mode] カタカナ遷移候補観測 (1回目、確定保留): Hiragana/kana → ZenKata/kana (conv=0x0000000B)
...
[conv-mode] Hiragana/kana → ZenKata/kana (conv=0x0000000B)   ← 2回連続一致で確定（BUG-19対策は正常動作）
```

`[h1-send]`（GJI/MS-IME プロセス直接 I/O 監視、`ConvModeMgr` とは独立経路）も同時刻に
既に `conv=0x0000000B` を報告しており、2系統の独立観測が一致 — つまり「awase の
ポーリングが誤読した」のではなく、**実IMEの内部状態が本当にカタカナになっていた**。

**IME:** Microsoft IME（MS-IME）。TsfNative プロファイル（Windows Terminal/Cascadia）。

**原因（確定）:** NICOLA の物理「IME ON」キーは scan 0x70（JIS配列の
「カタカナ・ひらがな・ローマ字」キー位置）に割り当てられている。IME が既に ON の
状態でこのキーを押すと、Windows のキーボードレイアウト変換層が `VK_DBE_HIRAGANA`
(0xF2) ではなく **`VK_DBE_KATAKANA` (0xF1)** を生成することがある（同一物理キーに
対する OS 側の状態依存トグル変換、awase の関与しない層）。

`PhysicalKeyDisposition::plan`（`runtime/transport.rs`、BUG-46 修正後の実装）の
「KANJI 関連キー」汎用分岐は、KeyDown の suppress 条件を `shadow_toggled`
（＝そのキーで実際に `effective_open()` が false→true に反転したか）に限定していた。
IME が既に ON の状態で 0xF1 の KeyDown が届くと何もトグルしないため
`shadow_toggled=false` となり、**Suppress されず素通し**していた。`VK_DBE_KATAKANA`
は Windows 標準仕様で「カタカナへ切り替えろ」という能動的な意味を持つ仮想キーの
ため、`VK_KANJI` 等の「素通ししても実害の薄いキー」とは性質が異なり、
「toggle が不発だったから安全」という BUG-46 の前提がここでは成り立たなかった。

この漏れのある汎用分岐は 4 日前の `076b8709`（BUG-46 修正、2026-08-01）で
TsfNative プロファイルにも新規適用されており、ユーザーの「最近のバージョンで
起きるようになった」という証言と時期が一致する。BUG-46 のコードレビュー
（Claude 本体・Opus・Codex CLI の3系統独立検証）は `VK_DBE_SBCSCHAR`/`DBCSCHAR`
(0xF3/0xF4) のみを検証対象としており、`VK_DBE_KATAKANA` (0xF1) 固有の危険性
（素通しが即座に実IMEの能動的なモード変更を引き起こす）は監査対象外だった。

**修正:** `PhysicalKeyDisposition::plan` の KANJI 関連キー分岐に、
`event.vk_code == VK_DBE_KATAKANA && event_type == KeyDown` の場合は
`shadow_toggled` の値に関わらず常に Suppress する例外を追加（`ime_actuation_owned`
の場合のみ、既存のスコープは変更していない）。`VK_KANJI` 等、他の KANJI 系キーの
挙動（shadow_toggled 不発時は Allow）は変更していない。

**未対応（残存）:**

- 実機での再発有無の検証は未実施（次回セッションでの確認事項）。
- 「なぜこの物理キーで 0xF1/0xF2 が交互に生成されるか」という Windows 側の変換
  ロジックの正確な条件（IME の内部状態のどの部分に依存するか）は未解明。今回の
  修正は「0xF1 が来たら常に Suppress」という結果ベースの対処であり、根本的な
  発生条件の理解には至っていない。

**検証状況:** コード読解による確定（`hook.rs`/`transport.rs`/`vk.rs` を Explore
エージェントで独立監査、`vk.rs:48` で `VK_DBE_KATAKANA = 0xF1` の定義を確認）。
`transport.rs::plan_tests` に回帰テストを追加
（`dbe_katakana_keydown_suppressed_even_when_not_shadow_toggled`:
shadow_toggled=false でも Suppress されることを固定、
`owned_actuation_keydown_allowed_when_not_shadow_toggled_is_unaffected_by_dbe_katakana_fix`:
`VK_KANJI` の既存挙動が変わっていないことを固定）。`runtime` モジュールは
`#[cfg(windows)]` のため Linux 上では `cargo build --tests --target
x86_64-pc-windows-gnu -p awase-windows`（warning ゼロ）および `cargo clippy --lib
--target x86_64-pc-windows-gnu -p awase-windows -- -D warnings`（warning ゼロ）
での型検査のみ実施。実機/Windows 環境でのテスト実行・再発確認は未実施。

**関連ファイル:** `crates/awase-windows/src/runtime/transport.rs`
（`PhysicalKeyDisposition::plan`、`plan_tests`）、`crates/awase-windows/src/vk.rs`
（`VK_DBE_KATAKANA`/`VK_DBE_HIRAGANA`）、`crates/awase-windows/src/state/conv_mode.rs`
（`ConvModeMgr::update_from_conv`、今回のバグの「発見経路」となった BUG-19 対策
デバウンス）。関連: BUG-19（カタカナ conv デバウンスの元ネタ、本バグとは別原因）、
BUG-46（本バグの直接の混入元コミット）。

**追補1（2026-08-05、対象を `VK_DBE_KATAKANA` 単独から `VK_DBE_*` 全体へ拡大）:**
ユーザーから「同じ debug ログに `VK_DBE_ALPHANUMERIC` (0xF0) も同じパターンで
出ている」との指摘を受け見直した結果、当初の修正が 0xF1 だけを special-case
していたのは不十分だったと判明した。実機ログには次の並びもあり、0xF0 が
0xF1 と全く同じ「KeyUp だけが可視、対応する KeyDown は範囲外」パターンで
出現していた:

```
[hook] IME-mode vk=0xF0 up   self_injected=false injected=false scan=0x70
[hook] IME-mode vk=0xF2 down self_injected=false injected=false scan=0x70
[imm32-off] key suppress vk=0xf0 KeyUp (physical disposition)
```

`vk.rs::ImeKeyKind::from_vk` を確認すると、`PhysicalKeyDisposition::plan` の
「KANJI 関連キー」汎用分岐（`shadow_toggled` 依存の同じ穴）を通る `VK_DBE_*` は
0xF0 (`Alphanumeric`, `ShadowImeEffect::TurnOff`)・0xF1 (`Katakana`, `TurnOn`)・
0xF3 (`Deactivate`, `TurnOff`)・0xF4 (`ActivatePair`, `TurnOn`) の4種類（0xF2
`Activate`/`VK_DBE_HIRAGANA` のみ専用分岐で別処理済み）。IME が既に目的の状態に
ある時にこれらのいずれかが物理キーから届くと `shadow_toggled=false` となり、
0xF1 と全く同じ理由で素通しされ、実 IME が該当するネイティブ効果（英数/半角/
全角への切替）を能動的に適用してしまう。0xF3/0xF4 は BUG-46 の再現ケースそのもの
（`up 0xF3 → down 0xF4` ペア）だったにもかかわらず、BUG-46 の修正は「二重
actuation の防止」のみを目的としており、この「素通し自体がネイティブ効果を
持つ」というハザードは見落とされていた。

**修正（本追補）:** `is_dbe_katakana_down`（0xF1 単独判定）を
`is_dbe_mode_key_down`（`VK_DBE_ALPHANUMERIC`/`VK_DBE_KATAKANA`/
`VK_DBE_SBCSCHAR`/`VK_DBE_DBCSCHAR` の4種類、KeyDown）に拡張。挙動は
0xF1 のときと同一（`shadow_toggled` に関わらず常に Suppress）。`VK_KANJI` 等
DBE 範囲外のキーの挙動は変更していない。

**検証状況（追補）:** 実機ログで直接確認できているのは 0xF0/0xF1 のみ。0xF3/0xF4
は BUG-46 の再現ケースで同じコードパスを通ることは確認済みだが、「実際に
shadow_toggled=false の状態で漏洩した」ことを示す実機ログはまだ無い（コード
監査による論理的な一般化）。回帰テストは `dbe_mode_vks()` で4種類とも
パラメータ化して固定（`dbe_mode_keydown_suppressed_even_when_not_shadow_toggled`）。
Linux 上のビルド/clippy 確認のみで、実機/Windows 環境でのテスト実行・再発確認は
引き続き未実施。

**追補2（2026-08-05、ユーザー指摘による因果関係の整理）: 「トグルキー問題」ではなく
「冪等性の前提が送信側にしか成立していなかった」問題として再定義。**
本ファイル 3642 行目付近で既に確立している設計原則「`VK_KANJI` のようなトグル
キーではなく、ON/OFF 専用の冪等キー（`VK_IME_ON`/`VK_IME_OFF`）を使う」は
`VK_DBE_HIRAGANA` (0xF2) にも適用されており（803行目付近「MsImeDirect の冪等
VK_DBE_HIRAGANA」）、この設計判断自体は正しい。

本バグの実際の欠陥は、この冪等性の前提が **awase 自身が能動的に送信する側**
（`SendInput`/`IMC_SETCONVERSIONMODE` で常に同じ VK/値を送る）では正しく
成立していたのに、**物理キーをそのまま素通しする側**（`PhysicalKeyDisposition::
plan` が `shadow_toggled=false` を理由に Allow する側）では、同じ冪等性が
暗黙に成り立つものとして扱われていたこと。実際には素通しされる生の物理
イベントの VK は 0xF2 とは限らず、`ImeKeyKind::from_vk` が同じ「shadow IME
TurnOn/TurnOff」グループへ分類する 0xF0/0xF1/0xF3/0xF4 でもあり得て、これらは
それぞれ「ひらがなにする」とは全く別の（非冪等な）副作用を実IMEに与える。
「トグルキーを避けて冪等キーを選ぶ」という設計判断は、awase が能動的に選んで
送信する VK にしか適用できない保証であり、**同じ意味クラスに分類された物理
キーの生イベントを無検査で通すこと**にまで拡張できる保証ではなかった、という
のが正確な因果関係。
## BUG-53: Win キー押下時に検索UIが開くと KeyUp が失われ `PHYSICAL_KEY_STATE[VK_LWIN]` が恒久的にスタックし、以後 IME ON/OFF の実送信が無期限にスキップされる

**症状:** WindowsTerminal（MS-IME、TsfNative プロファイル）で、Windows キーを
押した際に検索UI（`searchhost.exe`）が開くと、以後 `Ctrl+変換`/`Ctrl+無変換`
（IME control ON/OFF ホットキー）を押しても Engine の belief だけが ON/OFF
切り替わり、**実 IME には一切反映されなくなる**（ユーザー通報「なぜかCtrl＋変換で
IME OFFにならなくなっちゃった」、2026-08-06）。

**再現手順（実機 debug ログで確認済み）:**

```
[apply-ime] MS-IME direct: send 0x001A (DirectInput, 冪等)
[ime-mode] skipped vk=0x1A (Win key held — Win+VK_IME triggers Start Menu on Win↑)
[apply-ime] open=false eff=true conf=true → outcome=UnsafeToToggle
```
上記が `Ctrl+変換`/`Ctrl+無変換` を押すたびに繰り返され、実送信が永久にスキップ
される。ログを遡ると、この直前にユーザーが物理 Windows キーを押下しており、
以後ずっと `vk=0x5B`（VK_LWIN）の KeyUp が一度も観測されていない。その後
ユーザーが**もう一度** Windows キーを押して離す（この直後 `searchhost.exe` への
フォーカス変更ログが確認された）と、直後から `Ctrl+変換` が `outcome=Applied`
で正常に実送信されるようになった。

**IME:** Microsoft IME。TsfNative プロファイル（Windows Terminal 等）。

**原因（確度: 中〜低、状況証拠からの推測。確定ではない）:** `hook.rs` の
`PHYSICAL_KEY_STATE`（`is_physical_key_down` が参照する、非注入・非自己注入の
物理 KeyDown/KeyUp のみで更新される全 VK 用のフラグ配列）は、VK_LWIN/VK_RWIN
専用の特別分岐を持たず、他の全 VK と同じ経路で無条件更新される。実機ログでは
他キー（Ctrl・無変換等）の KeyDown/KeyUp は正常に処理され続けていたため、
`WH_KEYBOARD_LL` フック自体が全面停止したわけではない。Windows キー押下で
検索UIが実際に開くフローに入ったことは確実であり、この経路上で **シェル/検索UI
側が保持する別の低レベルフックが `CallNextHookEx` を呼ばずに Win キーの KeyUp
を消費し、awase 側のフックにイベントが渡らなかった**、という Win32 API の一般的
な仕様（フックチェーンの前段が `CallNextHookEx` を呼ばないと後段に届かない）に
基づく推測が最も有力。ただしこれを直接裏付ける実測ログ・コメントはリポジトリ内
に存在しない。

`crate::hook::win_key_held()`（旧: `tsf/send.rs::win_key_held()` と
`ime.rs::send_ime_mode_key` 内に重複記述されていた `is_physical_key_down(VK_LWIN)
|| is_physical_key_down(VK_RWIN)` の単純 OR）が Win+VK_IME によるスタートメニュー
誤起動を防ぐため（BUG-16 追補、ADR-061）に既存していたが、KeyUp 消失で
`PHYSICAL_KEY_STATE` がスタックすると、このガード自体が「安全策」から「恒久的な
機能停止」に反転してしまう。

**修正:** `PHYSICAL_KEY_STATE` 単純参照ではなく、`PHYSICAL_KEY_DOWN_AT_MS`
（既存の押下開始時刻トラッキング）から算出した保持時間が
`tuning::WIN_KEY_HELD_STALE_MS`（2,000ms、未実測の暫定値）以上続いている場合は
stale とみなし「押されていない」扱いにする。判定の中核ロジック
（`state/win_key_guard.rs::is_held_fresh`）は Win32 API を呼ばない純粋関数として
分離し、`alt_impersonation.rs`（BUG-41 の教訓）と同じ理由で Linux の
`cargo test -p awase-windows --lib` から常時実行できるようにした。
`crate::hook::win_key_held()` を唯一の判定点として新設し、`tsf/send.rs`/`ime.rs`
の重複していた個別チェックをこれに一本化した。

**未対応（残存）:**

- KeyUp 消失の正確なメカニズムは推測のまま（上記「原因」参照）。実機での再現・
  検証は未実施。
- `WIN_KEY_HELD_STALE_MS=2000ms` は実測に基づかない暫定値。実機ソークでの
  調整余地がある（`tuning-constants.md` の実測義務を満たせていない、次回
  実機確認時に要実測）。
- stale 判定は「2秒間 KeyUp が来ない」という間接的なシグナルに依存しており、
  フォーカス変更をトリガーにした即時リセット（`on_window_focus_event` 経由）
  という代替案も検討したが、Alt+Tab 中等に Win キーを本当に長押ししている
  最中の誤リセットを避けるため見送った（両方式ともトレードオフがあり、
  こちらがより保守的と判断）。

**検証状況:** コード読解による確定（`hook.rs`/`ime.rs`/`tsf/send.rs`を Explore
エージェントで独立監査）+ 実機ログでの発生・回復シーケンスの確認。
`state/win_key_guard.rs` に純粋ロジックの単体テスト3件を追加し Linux で
`cargo test -p awase-windows --lib` から実行・pass 確認済み
（`none_is_not_held`/`fresh_hold_is_held`/`stale_hold_is_not_held`）。
`runtime` モジュールは `#[cfg(windows)]` のため Linux 上では `cargo build --tests
--target x86_64-pc-windows-gnu -p awase-windows`（warning ゼロ）および
`cargo clippy --lib --target x86_64-pc-windows-gnu -p awase-windows -- -D
warnings`（warning ゼロ）での型検査のみ実施。実機/Windows 環境での再発確認は
未実施。

**関連ファイル:** `crates/awase-windows/src/hook.rs`（`win_key_held`、
`PHYSICAL_KEY_STATE`/`PHYSICAL_KEY_DOWN_AT_MS`）、
`crates/awase-windows/src/state/win_key_guard.rs`（`is_held_fresh`、新設）、
`crates/awase-windows/src/tuning.rs`（`WIN_KEY_HELD_STALE_MS`、新設）、
`crates/awase-windows/src/tsf/send.rs`（`send_vk_dbe_hiragana_pair`）、
`crates/awase-windows/src/ime.rs`（`send_ime_mode_key`）。関連: BUG-16
（`send_ime_mode_key` スキップ時の `UnsafeToToggle` 伝播、この設計自体は正しく
機能していた）、ADR-061（Win キー押下中の IME キー注入スキップ機能の導入元）、
BUG-41（`alt_impersonation.rs` 移設と同じ「純粋判定を Linux でテスト可能にする」
再発防止パターンの前例）、BUG-23（`reset_physical_key_state` によるセッション
ロック解除時の全 VK リセット、同種の stuck-modifier 対策の前例）。

## BUG-54: `apply_force_on_for_imm_broken` の `conv_mode_policy=force` 経路が20msごとのVK_IME_ON無限再送ループに縮退し、体感遅延・ガタつきを引き起こす

**症状:** Windows Terminal（MS-IME、TsfNative/InputSite プロファイル）で
`conv_mode_policy=force` 運用中、ユーザーから「動きが少しがたついているというか、
遅延がとてもあり、ギリギリ使えなくてもないがかなりストレスフル」という報告
（2026-08-07）。実機 debug ログでは以下のブロックが **~35〜50ms 間隔で無限に
繰り返され**、900ms の抜粋だけで20回以上観測された:

```
[stage-observe] strategy=SkipTyping/Blacklist belief_on=true explicit_intent=Some(true)
[apply-ime] MS-IME direct: send 0x0016 (IME ON)
[ime-mode] SendInput vk=0x16 ...
[apply-ime] open=true eff=true conf=true → outcome=Applied
Blacklist force-ON: apply_ime_open(true) → Applied
[ime-mode] SetOpen(true) applied → Hiragana (belief, unconfirmed)
[composition] ImeEffect::SetOpen(true) → marking cold
[composition] marked cold reason=SetOpenTrue ... → next VK/TSF output will send VK_DBE_HIRAGANA warmup
Timer set: logical=101, ms=20, os_id=...
...(UIA async 問い合わせ等)...
Timer killed: logical=101, os_id=...
read_ime_state_full: ... ime_on=None (preserving state)
```
打鍵の有無に関係なく常時回り続けており、実際にユーザーが打鍵した Enter
（この抜粋の末尾）が挟まってもループ自体は継続していた。

**IME:** Microsoft IME。TsfNative プロファイル（Windows Terminal / InputSite）。

**原因（確定、コード読解 + ログでの因果確認済み）:** `runtime/mod.rs::
apply_force_on_for_imm_broken` は「`applied` が既に ON 記録済みなら送らない」
という自己スロットルを持っていたが、`893254c9`（本ブランチに先立って別セッション
がマージ、`feat(awase-windows): conv_mode_policy=forceをIME ON/OFF軸にも適用`）
が `conv_mode_policy=force` のときこのスロットルを完全にバイパスするよう変更
していた。ところが呼び出し連鎖は次の通りで、このスロットルが同時に
「`on_ime_apply_complete` → `platform.post_ime_refresh()` が無条件に仕込む
20ms 後の確認 refresh チェーン」を1回で自己終了させる安全弁も兼ねていた:

```
TIMER_IME_REFRESH(20ms) 発火
  → run_ime_refresh_with_prefetched()
    → apply_force_on_for_imm_broken()   // force policy でスロットル無効化
      → VK_IME_ON 送信、on_ime_apply_complete()
        → platform.post_ime_refresh()   // 無条件で 20ms 後に TIMER_IME_REFRESH 再セット
          → (最初に戻る、無限)
```

TsfNative ウィンドウでは `reschedule_ime_refresh()` 自体が
`is_tsf_native || explicit_intent().is_some()` で常に早期 return するため、
`post_ime_refresh()` の 20ms 確認チェーンが `apply_force_on_for_imm_broken` を
再度呼ぶ**唯一の経路**になっている。`893254c9` のコミットメッセージが想定していた
「500ms poll ごとに無条件で再送する」という設計意図は TsfNative では成立せず、
実際には 20ms 周期の自己駆動ループに縮退していた。

**修正:** `force_policy` 分岐に、`Runtime` の新規フィールド
`last_force_on_resend_ms: Option<u64>` を使った自前のレート制限を追加した。
`ime_poll_interval_ms`（既定 500ms）未経過なら実送信をスキップし、残り時間
だけ次の refresh を予約して抜ける（force-policy の周期監視自体は止めない）。
`discard_actuation()`（FocusChanged 等で `active_actuation` を破棄する既存の
唯一の口）に併せてこのフィールドもリセットし、新しいフォーカス先で前の待機を
持ち越さないようにした。ADR-080 の `Actuation`（`ir_apply_drift_correction`
専用、`desired`/`FocusChanged`/`Resolution` 確定で破棄・再構築）への統合も
検討したが、`apply_force_on_for_imm_broken` は `check_drift_correction` を
経由しない別経路のため、大きな設計変更なしに転用できず、今回は見送った
（将来の統合候補として残す）。

**検証:** `cargo xwin check`/`cargo xwin clippy -- -D warnings`
（`x86_64-pc-windows-msvc`、いずれも warning ゼロ）、`cargo test -p
awase-windows --lib --test golden_scenarios --test architecture_guard`
（320 件 pass）。wine 未導入のためこのサンドボックスでは実機相当のテスト
実行は未実施。次回実機確認で、当該ログブロックの再送間隔が
`ime_poll_interval_ms` 相当（既定500ms）に戻っていることを確認すること。

**未対応（残存）:** BUG-51 で修正した OFF 方向（`ir_apply_drift_correction`
経由の `KatakanaShadowOff` 誤検知の可能性、実機再現待ち）とは別軸の問題。
`apply_force_on_for_imm_broken` と ADR-080 `Actuation` の設計統合は未実施。

**関連ファイル:** `crates/awase-windows/src/runtime/mod.rs`
（`apply_force_on_for_imm_broken`、`Runtime::last_force_on_resend_ms`）、
`crates/awase-windows/src/runtime/ime_actuation.rs`（`discard_actuation`）、
`crates/awase-windows/src/platform.rs`（`post_ime_refresh`）。関連: BUG-51
（TsfNative drift correction の再起動漏れ、同じ `TIMER_IME_REFRESH`/20ms 系統）、
[ADR-085](adr/085-conv-mode-force-policy.md)（`conv_mode_policy=force` の設計記録。
本文中「ADR-083」表記は誤り、正しくは ADR-085）、ADR-080（`Actuation` 型付き
トランザクション、統合の将来候補）、[ADR-086](adr/086-force-write-trigger-and-target-identity.md)
（本エントリが記録する自己駆動ループが INV-16 の一次証拠）。

## BUG-55: `get_ime_wnd`/`set_ime_romaji_mode` が `GetForegroundWindow()`（トップレベル）基準の `ImmGetDefaultIMEWnd` を使うため、InputSite 子ウィンドウの実際の変換モードとは無関係な標的に書き込み、JISかな入力ロックから復旧できなくなる

**症状:** Windows Terminal（MS-IME、TsfNative/InputSite プロファイル）で
`conv_mode_policy=force` 運用中（BUG-54 の 20ms 無限ループ修正を適用した
ビルドで検証）、「なぜか、JISかなが有効になって、入力不能になった」という
実機報告（2026-08-07）。ログでは awase 側は一貫して「ローマ字モードへの
訂正に成功した」と記録し続けていた:

```
[conv-mode] Hiragana/roma → Hiragana/kana (conv=0x00000009)   // FocusChange 直後、実IMEがかな入力で復元
[imm-romaji] conv 0x00000009 → 0x00000019 success=true         // set_ime_romaji_mode が「成功」
[idle-conv-check] TsfNative: conv=0x00000019 → belief AssumedRomaji 変更なし  // 以後もローマ字のまま、と観測し続ける
```
しかし実際に画面上で見えていた IME は JIS かな入力のままで、awase 経由の
物理キー入力が正しいローマ字として解釈されず入力不能になった。

**IME:** Microsoft IME。TsfNative プロファイル（Windows Terminal / InputSite）。

**原因（コード確認済み、実機での完全な因果特定は未確定）:** BUG-54 の調査で
追加した診断ログ（`[idle-conv-check-diag] foreground_hwnd=... ime_wnd=...`）
により、`get_ime_wnd`（`ImmGetDefaultIMEWnd(GetForegroundWindow())`）が返す
`ime_wnd` が、フォーカスの実体（`Windows.UI.Input.InputSite.WindowClass`,
`HWND(0x20954)`）とは異なる `HWND(0x70a8a)` で、かつセッションを通して
一貫して同じ値のまま変化しないことを確認した。`GetForegroundWindow()` 自体も
常にトップレベルウィンドウ（`HWND(0x20942)`, `CASCADIA_HOSTING_WINDOW_CLASS`）
を返し、実際にテキスト入力を受けている InputSite 子ウィンドウ
（`HWND(0x20954)`）を指していない。

`ime.rs::set_ime_romaji_mode()`（`MsImeDirectStrategy::apply(open=true, ..)`
内で「ROMAN ビットを先に立てる」ために呼ばれる）と `get_ime_conversion_mode_
raw_timeout()`（`idle-conv-check`/`focus-conv-check` が使う conv 読み取り）は
**どちらも同じ `get_ime_wnd(GetForegroundWindow())` 経路**を使っている。
`IMC_SETCONVERSIONMODE`/`IMC_GETCONVERSIONMODE` はレガシー IMM32 互換の
「デフォルト IME ウィンドウ」に送られる `WM_IME_CONTROL` であり、TSF3
ネイティブな InputSite 子ウィンドウの実際の composition/conversion 状態とは
別物の可能性が高い。読み取り・書き込みの双方が一貫して同じ的外れな標的を
指しているため、awase の belief（`AssumedRomaji`、`conv=0x00000019`）と
実際に画面へ反映される IME 状態が食い違ったまま自己整合してしまい、
`success=true` ログが実害の発見を遅らせる。

**未確定な点:** 上記は診断ログから読み取れる構造的な疑わしさであり、
「`ime_wnd`/`foreground_hwnd` が InputSite と無関係な標的である」ことが
JISかなロックそのものの直接の原因と実機で1対1に確認できたわけではない
（`set_ime_romaji_mode` 呼び出し直後に実際の画面表示を確認する追加ログは
まだ入れていない）。

**修正:** `crate::ime::get_focused_hwnd()`（`GetGUIThreadInfo().hwndFocus`
優先・`GetForegroundWindow()` フォールバック、30ms タイムアウト）という
**まさに正しい既存ヘルパーが `send_f2_via_sendmessage` の1箇所でしか使われて
いなかった**ことが判明した。`read_ime_state_full` も同じ `GetGUIThreadInfo`
経路（`get_gui_thread_info_with_timeout`）で `focused_hwnd` を解決しており、
実機ログで `HWND(0x20954)`（InputSite 子）を正しく返している。一方
`get_ime_conversion_mode_raw`/`get_ime_conversion_mode_raw_timeout`/
`set_ime_romaji_mode`/`set_ime_romaji_mode_with_target` の4関数だけが
`GetForegroundWindow()` に取り残されていた。この4関数をすべて
`get_focused_hwnd()` 基準に統一した。

なお `read_ime_state_fast()` は意図的に `GetForegroundWindow()` を使い続けて
いる（`profile.can_read_imm32_open_status()` で読み取り不能プロファイルを
別途ガードしており、「トップレベル hwnd の方が TSF 互換ブリッジに応答
しやすい」場合があるという別の設計意図によるもの、doc コメント参照）。
今回の変更対象には含めていない。

**修正が届く範囲:** `set_ime_romaji_mode`/`set_ime_romaji_mode_with_target`
は `MsImeDirectStrategy::apply(open=true, ..)` と
`tsf/warmup/cold_warmup.rs::ColdWarmupSequence::run_start`（`conv_mode_policy`
の observe/force 両方）の双方から呼ばれているため、この2経路すべてに修正が
及ぶ。`get_ime_conversion_mode_raw_timeout` は `idle-conv-check`/
`focus-conv-check`（`KatakanaShadowOff` 等の判定根拠）にも使われており、
BUG-51/BUG-54 で扱った conv 観測の信頼性そのものにも波及する可能性がある。

**検証:** `cargo xwin check`/`cargo xwin clippy -- -D warnings`
（`x86_64-pc-windows-msvc`、いずれも warning ゼロ）、`cargo test -p
awase-windows --lib --test golden_scenarios --test architecture_guard --test
ime_key_sequence_golden`（284+22+14 件 pass、golden は `#[cfg(windows)]` の
ため Linux では 0 件）。wine 未導入のためこのサンドボックスでは実機相当の
テスト実行は未実施。**次回実機確認が必須**: `conv_mode_policy=force` で
Windows Terminal に再度フォーカスを移し、(a) JISかなロックが再発しないこと、
(b) `[imm-romaji] conv ... → ... success=true` 直後に画面上の IME が実際に
ローマ字入力へ切り替わっていること、(c) `[idle-conv-check-diag]
focused_hwnd=...` が InputSite 子 hwnd（`read_ime_state_full` の
`focused_hwnd` と一致）を指すことを確認すること。「上記は診断ログから
読み取れる構造的な疑わしさであり、実機で1対1に確認できたわけではない」
（旧稿の「未確定な点」）は本コミット時点でもまだ解消していない。

**関連ファイル:** `crates/awase-windows/src/ime.rs`（`set_ime_romaji_mode`、
`set_ime_romaji_mode_with_target`、`get_ime_conversion_mode_raw`、
`get_ime_conversion_mode_raw_timeout`、`get_focused_hwnd`）、
`crates/awase-windows/src/imm.rs`（`get_ime_wnd`）、
`crates/awase-windows/src/ime_controller.rs`（`MsImeDirectStrategy::apply`）、
`crates/awase-windows/src/tsf/warmup/cold_warmup.rs`
（`ColdWarmupSequence::run_start`）。関連: BUG-54（同じ実機セッションで先に
発見された `apply_force_on_for_imm_broken` の無限ループ、本バグの発見は
その修正ビルドの実機検証中に判明）、[ADR-085](adr/085-conv-mode-force-policy.md)
（`conv_mode_policy=force` の設計記録。本文中「ADR-083」表記は誤り、正しくは
ADR-085）。

## BUG-56: `learn_imm_capability_on_focus` が `ImmGetDefaultIMEWnd`=NULL を1回観測しただけで `Unavailable` を確定し、ジェネリックなクラス名を共有する本物のテキスト入力欄まで巻き込んで物理IMEキーが漏れ文字が重複コミットされる

**症状:** LINE（Qt663QWindowIcon、`AppImeProfile::Standard`）で「なにをうっても
でででになる」「はさささ→はははは」という実機報告（2026-08-07、BUG-54/BUG-55
修正ビルドでの再テスト中）。debug ログでは、awase 自身の `send_keys` は該当文字を
**1回しか送信していない**のに、画面には同じ仮名が複数回コミットされていた。

**IME:** Microsoft IME。LINE（Qt ベース、`class="Qt663QWindowIcon"`）。

**原因（コード確認済み）:** `focus/imm_learning.rs::learn_imm_capability_on_focus`
は、フォーカスされたウィンドウの `class_name` に対して `ImmGetDefaultIMEWnd` が
NULL を返した**その場**で、`ImmCapability::Unavailable` を即座に確定・永続化
（`cache.toml` の `[imm_capability]`）していた。`cache.toml` を確認したところ
実際に `Qt663QWindowIcon = "unavailable"` が記録されていた。

Qt はウィンドウクラス名をアプリ内の複数の異なるウィジェット（本物のチャット入力欄・
通知アイコン絡みの一時ウィンドウ等）で使い回すことがある。フォーカス解決に使う
`GetGUIThreadInfo().hwndFocus` が、たまたまテキスト入力とは無関係な一時ウィンドウ
（同じクラス名）を指した瞬間に `ImmGetDefaultIMEWnd` が NULL を返すと、それが
**単発の観測だけで即確定**し、`class_name` をキーとする学習キャッシュ経由で
本物のチャット入力欄（同じクラス名）まで巻き込んで `Imm32Unavailable` に降格
した。この降格により、LINE 向けに歴史的に確立していた「ImmCross アプリには
物理 IME キーを見せない」設計原則
（[[feedback_immcross_owns_kanji]]、`project_kanji_imecross_spurious_vk3.md`）が
崩れ、`Blacklist force-ON`（`VK_IME_ON` を物理キーとして LINE へ定期送信）が
発火するようになった。物理 IME キーが LINE 側の composition/commit ロジックへ
漏れたことが、同じ文字の重複コミット（「でででで」）の実害だったと推測される
（正確な二重コミットの内部メカニズムまでは未確認）。

**暫定回避（実機で確認済み）:** `cache.toml` の `[imm_capability]` セクションから
`Qt663QWindowIcon` のエントリを削除し、awase を再起動（`ImmCapabilityStore` は
起動時に一度だけ `cache.toml` を読み込むため、ファイル修正だけでは反映されない）。
これにより「でででで」「はははは」は再発しなくなった（ユーザー確認済み、
2026-08-07）。ただしこれは学習し直しの起点をリセットしただけで、同じ一時
ウィンドウが再度 NULL を返せば再発しうる対症療法。

**恒久修正:** `ImmCapabilityStore` に `pending_unavailable: HashMap<String, u32>`
（永続化しないセッション内カウンタ）を追加し、`record_null_probe()`／
`clear_pending_unavailable()` を新設。NULL 観測は即確定せず、同じ `class_name` で
`UNAVAILABLE_CONFIRM_THRESHOLD`（= 2）回**連続**観測して初めて `Unavailable` を
確定・永続化するようにした（BUG-19 の「非カタカナ→カタカナ遷移を2回連続観測する
まで確定させない」デバウンスと同じ考え方、`.claude/rules/ime-belief-architecture.md`
参照）。途中で non-NULL 観測（本物の入力欄が応答した）が挟まればカウントをクリア
する。`learn_imm_capability_on_focus` はこの2メソッドに委譲するだけに変更した。

これは対症療法ではなく根本原因（単発誤判定への脆弱性）への対応だが、完全な解決
ではない — 同じ一時ウィンドウが2回連続でたまたま先にフォーカスされれば依然として
誤確定しうる（閾値を上げれば緩和されるが、真に IMM32 が使えないアプリの検出が
遅れるトレードオフがある）。

**検証:** `cargo xwin check --tests`/`cargo xwin clippy --tests -- -D warnings`
（`x86_64-pc-windows-msvc`、いずれも warning ゼロ、pre-existing の
`e2e_windows.rs`/`gji_fsm.rs` 等の pedantic 警告とは無関係と確認）。
`focus/classifier.rs` に `imm_capability_store_tests` を新設し、単発 NULL では
確定しないこと・2回連続で確定すること・non-NULL 観測でカウントがリセットされる
ことを検証する4件のユニットテストを追加した。`focus::classifier` モジュールは
`#[cfg(windows)]` のためこのサンドボックス（Linux, wine 未導入）ではコンパイル
検査のみで実行はできていない。次回実機確認で、LINE 通知ポップアップ等の連続
フォーカスでも `Qt663QWindowIcon` が誤って `Unavailable` に降格しないことを
確認すること。

**関連ファイル:** `crates/awase-windows/src/focus/imm_learning.rs`
（`learn_imm_capability_on_focus`）、
`crates/awase-windows/src/focus/classifier.rs`（`ImmCapabilityStore`、
`record_null_probe`/`clear_pending_unavailable`/`imm_capability_store_tests`）、
`crates/awase-windows/src/focus/tracker.rs`（`FocusTracker` 薄いラッパー）、
`crates/awase-windows/src/platform.rs`（`WindowsPlatform` 薄いラッパー）。
関連: BUG-54・BUG-55（同じ実機検証セッションで連鎖的に発見）、
[[feedback_immcross_owns_kanji]]、`project_kanji_imecross_spurious_vk3.md`
（LINE に物理 IME キーを見せてはいけない設計原則の由来）。

## BUG-57: `classify_ime_snapshot` の `OsPoll` 観測が `ime_on` を見ずに `conv` だけで英数(`ObservedEisu`)判定するため、一瞬フォーカスを奪った無関係な窓の観測が次のウィンドウまで残留し1文字目がリテラル化する

**症状:** Windows Terminal（TsfNative、MS-IME）で59秒ほど無操作の後、最初の
一文字だけ意図した文字（「と」）ではなく生のリテラル（全角「ｊ」）が出力された。
2文字目以降は正常。

**再現手順:** ①どこかのウィンドウで日本語入力中に59秒以上操作しない → ②その間に
`pushbullet_client.exe`（Windows通知アプリ）の通知ポップアップが一瞬フォーカスを
奪い、直後に元のウィンドウ（Windows Terminal）へフォーカスが戻る → ③25秒以上
さらに無操作 → ④最初の1文字を入力すると、意図した文字ではなく生の物理キーが
そのまま出力される（IME はネイティブ日本語モードのままなので全角文字として
確定してしまう）。

**原因:** `crates/awase-windows/src/observer/ime_observer.rs::classify_ime_snapshot`
の英数判定分岐（旧: `snap.conversion_mode.is_some_and(|conv|
ConvMode::from_u32(conv).is_eisu())`）が `snap.ime_on` を一切参照していなかった。
`pushbullet_client.exe` へフォーカスが移った瞬間、`ir_stage_observe` の `OsPoll`
戦略がそのウィンドウの実 IME 状態を読み取り、`ime_on=Some(false)` かつ
`conv=Some(0x0)` という観測を得た。IME が閉じている窓の `conv=0`（NATIVE ビット
無し）は `is_eisu()` が真になるが、これは「ユーザーが英数を選んだ」ことの証拠
ではなく IME が閉じているという自明な事実の副産物に過ぎない。この誤読が
`InputModeObserved { mode: ObservedEisu, confidence: Medium }` として dispatch され
belief を書き換えた。

`InputModeObserved` は `focus_epoch` を持たず、`input_mode` はフォーカス変更時に
無条件でクリアされるスカラ値のため（ON/OFF 側の `ObserverReported` が
`observation_store.rs::clear_on_focus_change` で窓ごとに破棄されるのとは非対称）、
直後にフォーカスが Windows Terminal（TsfNative）へ戻った際も汚染された
`ObservedEisu` がそのまま残留した。`focus_tracking.rs` の TsfNative/SSOT
「cache restore スキップ」（`HwndCache` からの復元を意図的にスキップする設計、
仮想デスクトップ切替時の Engine OFF desync 対策として `37883d09` で導入）は
この汚染を訂正する経路ではなく、素通しするだけだったため訂正されなかった。
25秒後の最初のキー入力時、`awase::engine::engine` が `ime_on=true なのに非活性:
reason=Inactive(NotRomajiInput)`（`input_mode=ObservedEisu` のため romaji 非対応と
誤判定）と判定し、そのキーを生の物理キーとして OS へ素通しした。実際の OS 側 IME
はネイティブ日本語モードのままだったため、素通しされた生キーがそのまま全角文字
として確定した。同じキーの KeyUp 時に `idle-conv-check` が実際の conv 値を
読み直して belief を訂正・Engine を再 activate したが、1文字目の後だった。

**修正:** `src/engine/conv.rs` に `ConvMode::is_eisu_evidence(ime_on: Option<bool>,
conv: Option<u32>) -> Option<bool>` を新設。`ime_on == Some(false)` のときは
`None`（判定不能）を返し、`conv` だけでの英数判定を行わないようにした。
`ime_on` が `Some(true)` または `None`（TsfNative 等 open 状態不明）のときは
従来どおり `conv` から判定する（トレイの半角英数コマンド等、既存の正当な
`ObservedEisu` 遷移は変更なし）。`classify_ime_snapshot` の英数判定分岐をこの
関数の呼び出しに置き換えた。`HwndCache` のスキップ設計自体は触っていない
（`37883d09` が塞いだ Engine OFF desync が再燃するリスクを避けるため）。

**検証:** `src/engine/conv.rs` に5件のユニットテスト
（`is_eisu_evidence_ignores_conv_zero_when_ime_off` 等）を追加、
`cargo test -p awase --lib conv::` で Linux 上で実行・全件green（クロスプラット
フォームの純粋関数のため実機不要）。`cargo build/clippy --target
x86_64-pc-windows-gnu -p awase-windows`・`cargo test -p awase-windows --lib`
（284件）・`architecture_guard`/`golden_scenarios`/`layer_boundary_guard`
（全件green）で既存挙動への回帰が無いことを確認。Windows 実機での再現確認
（Pushbullet 通知ポップアップを実際に発生させての検証）は未実施。

**未対応:** ON/OFF 側の `ObserverReported` と `InputModeObserved` の非対称
（前者は `focus_epoch` を持ちフォーカス変更で自動的に無効化されるが、後者は
持たない）自体は解消していない。同種の「一瞬だけ通り過ぎた無関係な窓の観測が
belief に残留する」問題が、英数判定以外の経路で再発する可能性は残る
（follow-up 案として `InputModeObserved` への `focus_epoch` 導入が検討されたが、
`ime_event.rs` の variant 変更と `architecture_guard.rs` の期待値更新を伴う
中リスクの変更のため、本 fix ではスコープ外とした）。

**関連ファイル:** `src/engine/conv.rs`（`ConvMode::is_eisu_evidence`）、
`crates/awase-windows/src/observer/ime_observer.rs`（`classify_ime_snapshot`）。
関連: BUG-11（UIA キャッシュ汚染）、BUG-18（AppKind Uwp 往復での文字欠落） —
いずれも「無関係なフォーカス遷移が belief/キャッシュを汚す」という同じテーマの
別発生箇所。

## BUG-58: 小指シフト面のチョード（Shift+数字等）が `OutputActiveGuard` と `shift-conv-guard` 復元の循環待ちに陥り、通常速度の打鍵でも毎回 ~5 秒フリーズする（対応済み・実機未検証）

**症状（2026-08-07 ユーザー報告、MS-IME/TSF-native、Windows Terminal）:** 「よくなりましたね！」
「き！ほげ」のように文中に小指シフト面の記号（`!` = Shift+1）を打つと、`!` が
画面に表示されるまで **約5秒間、キー入力が一切反映されない**（他のキーを押しても
何も起きない）。フリーズが解けた瞬間、その間に押していた文字（`!` を含む後続の
文字列や Enter まで）が一気にまとめて出力される。実機ログで2回再現・確認済み
（1回目: `!` 単体、2回目: `き!ほげ` の `!` 部分）。

**原因（確定、実機ログ2件 + コード読解 + Opus によるコードベース検証で確認）:**
以下の循環待ちが発生している。

1. 物理 `Shift` 押下（`!` を打つための Shift+1 チョードの一部）で
   `kp_shift_conv_guard_key_down`（`runtime/key_pipeline.rs:1219`）が MS-IME の
   「Shift単独タップで英数へ誤切替する」クセを打ち消すため、判別未確定の時点で
   先回りして `actuate_conv_mode(HalfWidthAlnum)` を呼び conv を `0x00000000`
   （英数）へ書き換える。同時に BUG-49 追補2（ADR-084 Phase 2）の安全弁として
   `confirm_gate_deadline_override_ms` を `押下時刻 + SHIFT_CONV_GUARD_ENTRY_
   SUSPEND_CAP_MS`（5000ms、`tuning.rs:185`）にセットする。
2. `!` の romaji 送信は `ms_ime_gate_defer`（`output/vk_send.rs:356`）に捕まる
   （conv が NATIVE ではないため `is_native_ready()==false`）。ここで
   `MsImeReadyCoro`（`tsf/warmup/ms_ime_ready_coro.rs:152`）が
   `OutputActiveGuard::begin()` を保持し `OUTPUT_GATE.active=true` にする。
3. `OUTPUT_GATE.active=true` の間、フックから来る**物理キーイベントは
   `handle_wm_key_from_hook` に一切到達しない**（`app/mod.rs:406-407`、
   無条件で `INPUT_DEFER.defer_during_output(event)` へ退避。抜け道なしを
   コード読解で確認済み）。
4. conv を `0x0`→NATIVE に戻せる経路は、チョード（Shift単独タップではない）の
   場合 `kp_shift_conv_guard_key_up`（`key_pipeline.rs:1297`、Shift KeyUp 契機）
   → `kp_restore_kana_from_half_width`（同1371、`VK_DBE_HIRAGANA` 注入 + IMC
   write の非同期リトライ）**一択**（他の呼び出し口は全て
   `half_width_alnum_toggle_active` ガード付きで、チョード時はこのフラグが
   立たないため通らない）。
5. しかしその Shift KeyUp 自体が手順3により `INPUT_DEFER` に退避されており、
   `OUTPUT_GATE` が解除されるまで `kp_restore_kana_from_half_width` は
   **絶対に起動しない**。結果、`env_native_ready()`
   （`ms_ime_ready_coro.rs:52`）はこの経路を通る限り原理的に真になり得ず、
   `deadline_ms.max(override)`（同95）＝ entry がセットした
   `SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS`（5000ms）の満了を待つしかない。
   実機ログの `delta=4730ms`（Shift KeyUp の受理時刻と drain 時刻の差）から
   逆算すると Shift の保持自体はごく普通の速度（~270ms）であり、そこから
   期限切れまでの ~4.7 秒がまるごと無駄なフリーズになっている。

`SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS`（5000ms）は BUG-49 追補2で
「Shift の KeyUp が**何らかの理由でフックに届かない異常時**（ロック画面・
セキュアデスクトップ遷移等）」への安全弁として導入されたものだが、
**その「届かない」状況を `OutputActiveGuard` 自身が作り出している**という
自己矛盾が本バグの本質。BUG-49 が想定していた「ユーザーが意図的/誤って
Shift を長時間保持し続けた場合の劣化」とは異なり、**普通の速さの
Shift+数字チョードで毎回・決定論的に発生する**。

**回避されるケース:** GJI（MS-IME 以外）は `kp_shift_conv_guard_key_down` が
conv を書き込まないため対象外（`key_pipeline.rs:1289`）。entry の早期 return
条件（`effective_open`/`is_japanese_ime`/`is_user_enabled`/
`conv_mutation_allowed` のいずれかが偽）に該当する場合も対象外。

**検討した修正案（採否）:** Opus によるコードベース検証を経て以下を検討し、
「案E」を採用した。他の案は **NO-GO と判断済み・再提案しないこと**:

- **案B（`OUTPUT_GATE` の責務を「OS への実出力」だけに絞る大改造）**: NO-GO。
  `OUTPUT_GATE` は「送出中の再入防止」と「reinject 順序保証」の2責務を
  同時に担っており（`app/mod.rs:409-419` の Ctrl↑ 順序バグの記録参照）、
  分離は広域リファクタになる。BUG-49 が pass-5 まで要した領域で同時に
  大改造するのは危険。
- **案C（`SHIFT_CONV_GUARD_ENTRY_SUSPEND_CAP_MS` を 5000ms から短縮する対症療法）**:
  NO-GO。`.claude/rules/tuning-constants.md` の禁止パターン（実測なしの
  エスカレーション/対症療法）そのもの。この値は「Shift KeyUp が本当に
  届かない異常時」の安全弁として意味があり、復元リトライ（0/160/320/480ms）
  の実所要 ~640ms を割ると BUG-49 が release 側で再発する。**800ms 未満には
  構造的に下げられない**ため再提案しないこと。
- **案D（`VK_LSHIFT`/`VK_RSHIFT` の KeyUp を無条件で `OUTPUT_GATE` の defer 対象から
  除外する）**: NO-GO。物理イベントを即座に `handle_wm_key_from_hook` に通すと
  `PassThrough` 判定時にその場で OS へ再注入され、`INPUT_DEFER` の先行キーを
  追い越す。`app/mod.rs:409-419` が記録している Ctrl↑ 順序バグと同型の実害が
  Shift↑ について新たに起きる。

**修正（2026-08-07、案E: `OutputActiveGuard` を Phase 2 直前へ遅延取得、
コミット `38b5a4ee`）:**
`MsImeReadyCoro`（`tsf/warmup/ms_ime_ready_coro.rs`）の Phase 1（IMC 観測待ち、
無出力）は SendInput を一切行わないにも関わらず、`MsImeReadyCoro::new()` の
時点で `OutputActiveGuard` を確保し Phase 1 の間ずっと保持していたことが
循環待ちの直接原因だった。`OutputActiveGuard::begin()` の呼び出しを
Phase 2（`ProbeAction::Transmit` を yield する直前）のローカル変数
（`let _guard = OutputActiveGuard::begin();`、コルーチン完了まで生存）に
移し、Phase 1 では一切 `OUTPUT_GATE` を触らないようにした。これにより
Phase 1 中も物理キー分配（`handle_wm_key_from_hook`）がブロックされなくなり、
conv を Off→NATIVE に戻す唯一の経路（`kp_shift_conv_guard_key_up`、物理
Shift KeyUp 契機）が実時間で起動する。循環そのものが構造的に解消され、
BUG-49 の核心（`ms_ime_gate_defer` の NATIVE 確認待ち）は一切変更していない。

この修正が成立する前提として、`kp_stage_post_decision`
（`key_pipeline.rs:214`、内部で `kp_stage_shift_conv_guard` を呼ぶ）が
`kp_stage_execute`（同227、`run_passthrough_pipeline` 経由で物理 Shift KeyUp を
reinject キューへ退避しうる）**より必ず先に**実行される、という既存の
暗黙の順序に依存している。物理イベント自体が defer されても、conv 復元の
副作用（`kp_shift_conv_guard_key_up` の呼び出し）はこの順序のおかげで
defer 有無に関係なく発火する。この順序不変条件は `kp_stage_shift_conv_guard`
の doc コメントに明記した（Opus レビュー指摘）。**この呼び出し順を変更する
場合は本バグが再発しないか必ず確認すること。**

**ハイブリッド対応（PassThrough キーの追い越し対策）:**
`runtime/executor.rs::run_passthrough_pipeline` の output guard defer 判定
（step C）に `platform.has_pending_tsf_work()` を OR で追加した。これにより
Phase 1 待機中の PassThrough キーは `check_output_guard_defer` で
ReinjectKey 化されるようになり、既存の `reinject_wait_remaining`
（`Enter`/`Space`/`Escape` の KeyDown＝composition 確定キー限定で
`has_pending_tsf_work()` が下りるまで park する仕組み）が Phase 1 待機中にも
初めて実効化する。**ただしそれ以外の PassThrough（矢印キー・Tab・Ctrl+C 等）は
`OUTPUT_GUARD_MS` 窓を過ぎていれば依然として即 reinject され、Phase 1 待機
（実測 ~180ms）中にまだ送信されていない romaji を追い越しうる**（残存する
既知の限界。BUG-58 のフリーズ解消と比べて実害は小さいと判断、PassThrough
全般を defer する対応は将来課題）。

**テスト:** `tsf/warmup/ms_ime_ready_coro.rs` に
`phase1_does_not_hold_output_gate_only_phase2_does`（Phase 1 中は
`OUTPUT_GATE.is_active()` が変化せず、Transmit と同時に true になることを
直接固定）を追加。`OUTPUT_GATE` はプロセス全体の static で `cargo test` は
既定でマルチスレッド実行のため、同ファイル内で Phase 2 に到達する既存4テスト
（`coro_waits_until_confirmed_then_transmits` 等）と合わせて計5テストを
`GATE_TEST_LOCK`（`std::sync::Mutex<()>`）で直列化した（クレート全体の他
モジュール——`GjiWarmupCoro`/`ChromeProbe`/`LiteralDetectFsm` 等——も同じ
static を触るため、クレート全体での競合までは排除できない点はコメントに
明記済み）。`output`/`tsf` モジュールが `#[cfg(windows)]` ゲート下にあるため
Windows target でのみコンパイル対象（`cargo check -p awase-windows --target
x86_64-pc-windows-gnu --tests` で型検査・`cargo clippy -p awase-windows
--target x86_64-pc-windows-gnu --lib -- -A clippy::cargo_common_metadata -D
warnings -W clippy::cognitive_complexity` で lint、いずれも green。wine
未導入のためこのサンドボックスでは実行不可 — **Linux 側の green は本修正を
一切検証していない点に注意**）。`cargo test -p awase-windows`（Linux 実行分、
lib 284件・golden_scenarios 22件・architecture_guard 14件・
layer_boundary_guard 8件）は無影響で全 green（本修正が触れる `tsf`/`runtime`
モジュールの一部は Linux ネイティブビルドでは対象外のため、無回帰確認としては
限定的）。実機での再現確認・修正確認は未実施（Windows 実機セッション必須）。

**未確認・次のセッションでの実機確認事項:**
- 「よくなりましたね！」「き！ほげ」の再現手順で、実際にフリーズが解消される
  （Phase 1 待機が実測 ~180ms 程度に短縮される）ことの実機確認。
- PassThrough キー追い越し（矢印キー等）が実害として観測されるか。観測された
  場合は上記ハイブリッド対応の拡大を検討。
- レビュー時に確認した「`kp_stage_post_decision` → `kp_stage_execute` の順序」
  という暗黙の前提が、将来のリファクタで意図せず崩れていないかの継続的な注意。

**レビュー:** Opus による2ラウンドの設計・実装レビューを経て確定
（1ラウンド目: 案A〜Dを提示し案E採用を決定、2ラウンド目: 実装差分を
GO-WITH-CHANGES→上記の順序コメント追加・executor.rs コメント是正・
テスト直列化を反映）。

**関連ファイル:** `crates/awase-windows/src/tsf/warmup/ms_ime_ready_coro.rs`
（`MsImeReadyCoro`、`OutputActiveGuard` 遅延取得の本体）、
`crates/awase-windows/src/runtime/executor.rs`（`run_passthrough_pipeline`、
`has_pending_tsf_work()` 追加）、`crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_stage_shift_conv_guard` の順序不変条件コメント、
`kp_shift_conv_guard_key_down`/`kp_shift_conv_guard_key_up`/
`kp_restore_kana_from_half_width`）、`crates/awase-windows/src/app/mod.rs`
（`OUTPUT_GATE` ディスパッチ）、`crates/awase-windows/src/tsf/probe_bridge.rs`
（`OutputActiveGuard`/`OUTPUT_GATE`）、`crates/awase-windows/src/output/vk_send.rs`
（`ms_ime_gate_defer`）。関連: BUG-13（MS-IME cold-start 保護）、
BUG-49（本バグの直接の前提となった Phase 1・Phase 2、特に追補2の5000ms安全弁）、
ADR-084。
## BUG-59: `ImeModeFsm::on_conversion_mode_read` が FocusChange 直後の cold 判定用ポーリング（1回読み）だけで `confirmed=true` を確定させ、BUG-13 の confirm-then-transmit ゲートを無効化して先頭文字がリテラル化する（対応済み・実機未検証）

**症状（2026-08-07 実機、いずれも Windows Terminal / MS-IME / TsfNative・InputSite）:**

- **1件目（05:36〜05:38）:** Chrome から Windows Terminal へフォーカス移動後、
  69 秒間無操作（`Hook watchdog: no activity` が継続）ののち最初の文字を入力 →
  「英数字がそのまま出た」（ユーザー確認済み）。
- **2件目（20:45、同日）:** Chrome から Windows Terminal へフォーカス移動後
  **わずか 975ms** で最初の文字を入力 → `し`・`ち`・`て`・`つ` の4文字が英数字
  のまま出力され、ユーザーが `vk=0x08`（Backspace）を7回連続で送って手動で
  削除した（`20:45:06.579`〜`07.961` の一連の `[relay-passthrough] PassThrough
  idle: direct OS pass-through (vk=0x08 ...)` として実機ログに記録、ユーザー
  確認済み: 「英数字(ローマ字)がそのまま出たのを消した」）。

2件はフォーカス変更から最初の文字入力までの間隔が 69000ms 対 975ms と大きく
異なるにもかかわらず同じ症状を起こしており、**「長時間 idle で劣化する」という
仮説（当初の調査方向）ではこの2件を一貫して説明できない**。両者に共通するのは
「フォーカス変更後、最初の文字送信より前に一度だけ `conv` を読んでいる」という
点であり、これが真因であることをコード読解で確認した。

**IME:** Microsoft IME（TsfNative、Windows Terminal / `Windows.UI.Input.
InputSite.WindowClass`）。

**原因（コード確認済み）:** `ImeModeFsm::on_conversion_mode_read`
（`tsf/ime_mode_fsm.rs:147-171`）は、`state == Unknown`（フォーカス変更直後の
初期値）のときに `IMC_GETCONVERSIONMODE` の読み取りが1回でも成功すると、
分岐に関係なく無条件で `self.confirmed = true` をセットする（170行目、
`(ImeModeState::Unknown, _) => { ログのみ }` という枝分かれの後、共通コードで
即確定）。grace period も複数回一致の要求も無い。

`is_native_ready()`（同90-92行）は `confirmed && (Hiragana|Katakana)` を返し、
`ms_ime_gate_defer`（`output/vk_send.rs:356-394`）が「MS-IME への romaji 送信を
安全に行ってよいか」を判断する**唯一のゲート**としてこれを使う（370-372行:
`if fsm.is_native_ready() { return false; }` = defer せず即送信）。この設計
自体は BUG-13（OFF→ON 遷移直後の cold-start リテラル化）を塞ぐために導入された
正しい仕組みである。

問題は `confirmed` フラグの**書き込み元が1系統ではない**こと。BUG-13 が意図した
「安全に送信してよいと確認された」という意味とは別に、`platform.rs::
gji_on_focus_change`（433-473行）が **FocusChange のたびに無条件で** 1回だけ
`IMC_GETCONVERSIONMODE`（タイムアウト50ms）を投げ、結果を
`update_ime_mode_from_imc` 経由で同じ `on_conversion_mode_read` に渡している
（458行のコメント: 「FocusChange 直後に IMC を1回ポーリングして初期状態を
Unknown → 実値に更新する。sacr-warmup 開始前から Off/Hiragana が判明するため
**cold 判定の精度が上がる**」）。この呼び出しの本来の目的は cold-start
warmup戦略の初期値を決めるための**参考情報収集**であり、「TSF composition が
実際に compose 可能な状態まで初期化済み」を保証するものではない。しかし
`on_conversion_mode_read` はこの呼び出しと `MsImeReadyCoro`（BUG-13 の
confirm-then-transmit ゲート本体）からの呼び出しを区別せず、どちらも同じ
`confirmed` フラグに書き込む。

FocusChange 直後の conv 読み取り（`ImmGetDefaultIMEWnd` 経由、BUG-55 参照）は
IMM32 互換レイヤーが保持している値を返すため、**IME が以前 Hiragana モードで
あった記憶が残っているだけで NATIVE=true を返しうる**。実際に InputSite 子
ウィンドウ側で TSF の compose sink が新しいフォーカスに対して再アタッチ・
準備完了しているかどうかとは別物のはずだが、`on_conversion_mode_read` は
値が読めた事実だけで `confirmed=true` にしてしまうため、`ms_ime_gate_defer`
は「安全」と誤認して即座に romaji を送信する。969ms 後でも 69 秒後でも、
この一度きりの読み取りが先に完了して `confirmed=true` を立てていれば、
実際の compose sink 準備状況に関わらず同じ穴を通る。

**未確定な点:** 「conv=NATIVE=true という読み取り自体が正しく、TSF compose
sink 側の準備だけが遅れている」という因果関係は、`IMC_GETCONVERSIONMODE` と
実際の compose 可能性を独立に検証するログがまだ無いため確定していない
（BUG-55 で懸念された「`ImmGetDefaultIMEWnd` が返す互換ウィンドウがそもそも
InputSite の実体と無関係」という可能性も依然排除できていない）。次回実機
確認時は、リテラル化した瞬間の `conv` 読み取り値と、直後に UI Automation 等
実際の compose 状態を独立に取得できるログを追加できるとよい。

**修正（2026-08-07、上記候補の「フラグ分離」案を採用）:** `ImeModeFsm` に
`on_conversion_mode_hint(mode: Option<u32>)` を新設した。`on_conversion_mode_read`
と異なり `state` は更新するが `confirmed` は一切変更しない。`Output` に対応する
薄いラッパー `update_ime_mode_hint_from_imc` を追加し、`platform.rs::
gji_on_focus_change` の FocusChange 直後 cold 判定用ポーリング（1回限り、
`IMC_GETCONVERSIONMODE` タイムアウト50ms）の呼び出し先を
`update_ime_mode_from_imc` からこちらへ差し替えた。

これにより「参考情報収集のための1回読み」と「BUG-13 の confirm-then-transmit
ゲートが要求する、実際に安全と確認された読み」が同じ `confirmed` フラグを
共有しなくなる。FocusChange 直後は `state` が正しく `Hiragana`/`Off` に
更新される（Unicode cold-start 観測ゲート等の既存消費者には従来どおり効く）が
`is_native_ready()` は `false` のままなので、その後の最初の romaji 送信は
`ms_ime_gate_defer` → `start_ms_ime_ready_poll`/`MsImeReadyCoro` の
confirm-then-transmit ゲートを必ず通過するようになり、BUG-13 が本来意図した
「実際に compose 可能と確認できてから送信する」という保護が FocusChange
直後にも及ぶ。

検討した他の2案（`on_conversion_mode_read` への引数追加、BUG-56 型の2回連続
デバウンス）は不採用。前者はフラグ分離と実質同じ効果をより煩雑な呼び出し規約
（呼び出し元ごとに bool を渡し忘れるリスク）で実現するだけで利点が薄く、
後者は「2回目のポーリングをいつ・誰が発行するか」を新たに設計する必要があり
今回のスコープに対して過剰だった。

**検証:** `cargo check`/`cargo clippy -p awase-windows --target
x86_64-pc-windows-gnu --lib -- -D warnings`（warning ゼロ）、`cargo test -p
awase-windows --lib --test golden_scenarios --test architecture_guard --test
layer_boundary_guard --test ime_key_sequence_golden`（14+22+8 件 pass、
`ime_key_sequence_golden` は `#[cfg(windows)]` のため Linux では 0 件）。
`tsf/ime_mode_fsm.rs` に `conversion_mode_hint_updates_state_without_confirming`
（ヒントは `state` を更新するが `is_native_ready()` を true にしないことを
直接固定）と `conversion_mode_hint_ignores_none` を追加。この2件は
`#[cfg(windows)]` のため `--target x86_64-pc-windows-gnu --lib --no-run` で
コンパイル成功のみ確認済み（wine 未導入のためこのサンドボックスでは実行不可）。
**Windows 実機での再現確認・修正確認は未実施** — 次回実機で、Windows Terminal
へのフォーカス変更直後（idle 時間を問わず）に最初の文字が正しく compose
されることを確認すること。

**関連ファイル:** `crates/awase-windows/src/tsf/ime_mode_fsm.rs`
（`on_conversion_mode_read`, `on_conversion_mode_hint`（新設）, `is_native_ready`）、
`crates/awase-windows/src/platform.rs`（`gji_on_focus_change`、FocusChange 直後の
1回限り IMC ポーリング）、`crates/awase-windows/src/output/mod.rs`
（`update_ime_mode_from_imc`, `update_ime_mode_hint_from_imc`（新設））、
`crates/awase-windows/src/output/vk_send.rs`（`ms_ime_gate_defer`）、
`crates/awase-windows/src/tsf/warmup/ms_ime_ready_coro.rs`
（`MsImeReadyCoro`、confirm-then-transmit ゲート本体）。関連: BUG-13（本バグが
無効化してしまっていた confirm-then-transmit ゲートの導入元）、BUG-55
（同じ `IMC_GETCONVERSIONMODE`/`ImmGetDefaultIMEWnd` 経路の hwnd ターゲット
問題、本バグとは独立な懸念として残存）、BUG-56（単発観測での即確定を2回連続
デバウンスに直した前例、不採用案の検討材料として参照）。

### BUG-59 追補（revert 済み）: `conv_mode_policy = Force` の FocusChange 強制書き込みを MS-IME にも配線した変更（`9c102b02`）は、ターゲットウィンドウ競合により revert された

**経緯:** `9c102b02`（`feat(awase-windows): conv_mode_policy=Force の FocusChange
強制書き込みを MS-IME にも配線（BUG-59 追補）`、2026-08-07、実機未検証のまま
`develop` にマージ）は、`platform.rs::gji_on_focus_change` に「`conv_mode_policy
= force` のとき FocusChange のたびに `desired_mode` を実 IME へ強制書き込みする」
ロジックを追加した。翌日（2026-08-08）、ユーザーから実機報告があった:
「LINE で何を押しても『い』になる」「突然 IME がローマ字ではなく JIS かなになった」。

**アプリ:** LINE（Qt、`ImmCross` プロファイル） / Windows Terminal（TsfNative）。
**IME:** Microsoft IME、`conv_mode_policy = force` 有効（デフォルト `observe` の
opt-in 設定、試験運用中に発生）。
**再現手順と症状:** `conv_mode_policy = force` 有効時に、Windows Terminal 等へ
フォーカス移動 → 直後に LINE 等別ウィンドウへフォーカスが戻る、という往復操作の
あとで LINE 側の入力が壊れる（全打鍵が「い」になる／IME が JIS かなになる）。

**原因（コード読解で確定、詳細は
[ADR-086](adr/086-force-write-trigger-and-target-identity.md) §1.2）:**
実際に IME へ書き込む `set_ime_romaji_mode_with_target`（`ime.rs:782`）は、
実行された**その瞬間**に `get_focused_hwnd()` をライブクエリして書き込み先を
決める。`gji_on_focus_change` 側の世代カウンタ（`ime_mode_focus_gen`）チェックは
ディスパッチ直前の陳腐化しか検知できず、その後 `spawn_local` → `offload_unsafe`
（ワーカースレッド）を経て実際に書き込まれるまでの非同期の間隙で別ウィンドウへ
フォーカスが移っても検知できない。結果、あるウィンドウ向けに計算した conv bits
が無関係な別ウィンドウの IME コンテキストへ書き込まれ得る。UWP アプリは
親ウィンドウ→`InputSite` 子ウィンドウの2段でフォーカスが確定するため、1回の
ユーザー操作で `gji_on_focus_change` が数 ms 間隔で複数回走り、この間隙は実機で
構造的に開いている。

**なお、force-write 自身が「JIS かな化」を直接起こすことはできない**（`to_conv_bits()`
は `romaji: true` のとき必ず `IME_CMODE_ROMAN` を含み、`desired_mode` は常に
`romaji: true` で書き込まれるため）。JIS かな化との因果は未確定であり、
BUG-60 として別途起票した。

**対応: revert した。** `9c102b02` の差分（`platform.rs` の force 書き込みブロック、
本エントリの直前にあった「BUG-59 追補」節）を取り消した。埋めようとしていた穴
（`MsImeStrategy::needs_f2_probe()` が常に `false` のため、`conv_mode_policy =
force` が MS-IME では一度も発火しない）という**指摘自体は正しい**。この穴は
[ADR-086](adr/086-force-write-trigger-and-target-identity.md) Phase 2 の
「arm-on-focus / fire-on-intent」方式（FocusChange は武装フラグを立てるだけで、
実際の書き込みは次の送信直前まで遅延させる）で、ターゲット競合を起こさない形で
再導入する予定。**それまで `conv_mode_policy = force` は MS-IME（TsfNative）の
FocusChange 直後のドリフト訂正を持たない**（`cold_warmup.rs::run_start` 経由の
GJI 側のみ有効）。

**関連:** [ADR-086](adr/086-force-write-trigger-and-target-identity.md)
（本件が発端。§1.2 欠陥1〜2 に原因の詳細、§5 に revert の判断根拠）、
[ADR-085](adr/085-conv-mode-force-policy.md)（`conv_mode_policy = force`
本体）、BUG-60（LINE「い」化・JIS かな化の未確定な症状、本件と同時期の報告）。

## BUG-60: `conv_mode_policy = force` 運用中に LINE で全打鍵が「い」になる／IME が JIS かなになる（原因未確定）

**症状（2026-08-08 実機報告、ユーザー口頭）:**

- LINE（Qt、`ImmCross` プロファイル）で何を押しても「い」になる。
- 突然 IME がローマ字ではなく JIS かなになった。

`conv_mode_policy = force`（ADR-085）を試験運用中に発生。BUG-59 追補
（`9c102b02`、ターゲットウィンドウ競合を持つ force 書き込み、revert 済み）と
同時期の報告のため関連が疑われるが、**因果はログで確定できていない**。

**確定した事実（コード読解、原因の切り分けに使う）:**

- force-write 自身は「JIS かな化」を直接起こせない。`ConvMode::to_conv_bits()`
  （`src/engine/conv.rs:158`）は `romaji == true` のとき必ず `IME_CMODE_ROMAN` を
  含み、`desired_mode` の唯一の書き込み点である
  `message_handlers.rs::set_desired_conv_mode`（:528-536）は常に `romaji: true`
  を渡す。したがって「force が ROMAN ビットを消した」という説明は成立しない。
- むしろ疑うべきは逆方向: BUG-59 追補が持っていたターゲットウィンドウ競合
  （[ADR-086](adr/086-force-write-trigger-and-target-identity.md) §1.2 欠陥1）
  により、BUG-08 の JIS かな復元系（`[idle-conv-check] JISかな化を検出 →
  ローマ字入力を復元`）も別ウィンドウへ書き込んでいて効いていなかった、という筋。
  BUG-59 追補は revert 済みのため、この経路自体は次回発生時には無くなっている
  はずだが、LINE の「い」全打鍵化の機構は未解明のまま残っている。

**再現時に取るべきログ（次回実機で再現した場合）:**

1. 書き込み直前・直後の hwnd とウィンドウクラス名
2. 書き込み後の `IMC_GETCONVERSIONMODE` 再読み値
3. `[idle-conv-check] JISかな化を検出` ログの有無とタイミング
4. `[relay-passthrough]` に記録される実際の VK 列（「い」を出している打鍵が
   本当に別の文字キーなのか、それとも VK 列自体が「い」に対応するものなのかを
   区別する）

**未対応:** 原因未確定。BUG-59 追補の revert により再発するかどうかの
経過観察が最初のステップ。

**追補1（2026-08-08、ADR-086 Phase 2 実装完了・実機ソーク待ち）: MS-IME で
force-write が発火するのは Phase 2 が初めて。**

コード読解で判明: `9c102b02` revert 後の `develop` では、force-write の唯一の
発火点が `cold_warmup.rs::run_start`（cold 転換時）であり、そこへ到達するには
`prepend_f2_warmup`（`warmup_coord.needs_f2_probe()`）が必要だった。
`MsImeStrategy::needs_f2_probe()` は常に `false` を返すため、**Phase 2 実装
着手前の `develop` では MS-IME + force-write が構造的に一度も発火していない**。
つまり本バグ報告があった時点（2026-08-07〜08）で force-write を疑うなら、
唯一発火しうるのは GJI 系ストラテジ（`needs_f2_probe()=true`）経由のみで、
LINE の症状が MS-IME 由来なら §1.3 の「BUG-59 追補のターゲット競合で BUG-08 の
ROMAN 復元系が別ウィンドウに書いていて効いていなかった」筋の方が可能性が高い
（BUG-59 追補は revert 済みのため、この経路自体は解消されているはず）。

[ADR-086](adr/086-force-write-trigger-and-target-identity.md) Phase 2
（`Output::send_romaji`/`send_kana_char` を消費点とする `force_pending` 機構）の
実装により、**MS-IME を含む全ストラテジで force-write が初めて発火するように
なる**。これは本バグの唯一の現実的な再現機会でもある —— Phase 2 の実機ソーク
（タスク #17 と同一セッション）で LINE × MS-IME × `conv_mode_policy = force` の
組み合わせを試し、上記「再現時に取るべきログ」を収集すること。ソーク前に
`conv_mode_policy = force` を有効にしても、MS-IME 環境では実質 observe と
同じ（force-write が発火しない）状態が続く点に注意。

**追補2（2026-08-08、ADR-086 Phase 3 実装完了・実機ソーク未実施）: open/close 軸の
force-ON トリガーを周期リフレッシュからキー入力直前へ移行。**

`Runtime::apply_force_on_for_imm_broken`（TsfNative/Blacklist アプリ向けの
force-ON）は従来 `ir_stage_notify`（周期リフレッシュ、既定 500ms 間隔）から
呼ばれていた。ADR-086 Phase 3（INV-15）により、この関数の force-policy 分岐を
撤去し、`Runtime::force_open_pending`（`ir_post_focus_change_snapshot` で武装、
`kp_run_inner::consume_force_open_pending` で消費）という新しい武装/消費モデルへ
置き換えた。実際の force-ON は最初のキー入力の直前まで前倒しされる
（詳細は [ADR-086](adr/086-force-write-trigger-and-target-identity.md) §5 Phase 3）。

**テスト状況（`.claude/rules/fix-requires-evidence.md` (b) 適用）**: `Runtime`
全体を構築する既存のテストヘルパーが本リポジトリに無く（`Output::new()` 相当の
軽量コンストラクタが `Runtime` には存在しない）、`consume_force_open_pending`/
`ir_post_focus_change_snapshot` の武装・消費・再武装ロジックを単体テストで
直接検証できていない。`tests/architecture_guard.rs::
force_write_is_not_triggered_by_raw_focus_change` が「武装点が生の FocusChange
ハンドラ本体で直接書き込みを行っていないこと」という構造的な性質のみを
機械的に固定しており、実際の発火タイミング・レイテンシ・再武装の正しさは
Windows 実機での検証が必須。

**実機ソークで確認すべき項目**（§5 Phase 3 参照、Phase 2 のソーク #17 の
**後に**別セッションで実施し、conv 軸由来か open 軸由来かの副作用を
切り分けること）:
1. フォーカス後 1 打鍵目の追加レイテンシと `MS_IME_READY_CONFIRM_MS`
   （400ms）到達率（TsfNative × MS-IME/GJI）。
2. 周期撤去後に「フォーカス不変のまま IME が OFF に落ちる」
   （2026-08-06 実機報告のロック解除後静寂期パターン）が再現するか
   ——再現すれば案 F（idle 明け武装）等の追加トリガーを検討する。
3. 1 打鍵目のリテラル化（`bあ`/`korede` 系）が増えないか。
4. `UnsafeToToggle`（Win キー押下中等）による再武装の頻度。

**追補3（2026-08-08、2回目 opus アドバーサリアルレビュー）: Phase 3 実装の
訂正 + item 0 未達の記録。**

Phase 3 実装完了直後、実装内容そのものを2回目のアドバーサリアルレビューに
かけた結果、High 2件・Medium 5件・Low 4件、レビュー中の新規指摘2件を検出し、
順次修正した（詳細な経緯は
[ADR-086](adr/086-force-write-trigger-and-target-identity.md) §7-11）。
特に記録しておくべき事実:

- **ADR item 0（`ime_controller.rs` の同期ライブクエリ IMC write）は
  未移行のまま記録のみで item 1 を先行投入していた**——item 3 着手前に
  必須としていた自ら定めた前提を満たせていなかった。この同期 IMC write
  （実測根拠は `tuning.rs` の導出コメントより最大 ~100ms）は force-ON
  発火のたびに打鍵ホットパスに乗る。非同期化（`ImeOpenStrategy::apply`
  自体の非同期化）は Phase 3 のスコープを超える大規模改修のため、
  この事実を記録した上で見送りを維持している。
- 消費点の配置ミス（`kp_stage_focus_probe` より前だったため、フォーカス
  変更後 1 打鍵目を必ず取りこぼし 2 打鍵目で発火する）を修正。
- 消費点を settle ガード（`ime_apply_should_defer`）で守る当初設計は、
  `kp_stage_focus_probe` の後ろに置くと構造的に無効化される（barrier 消費
  後は時間に関わらず常に false）ため、Alt+Tab 中間ウィンドウへの誤射防止
  （2026-07-05 修正）が意図せず無効化されるところだった。入力意図の
  直接判定（KeyDown・非注入・修飾キー非押下・IME モードキー自体を除外）
  へ置き換えた。
- 周期レート制限撤去がフォーカスチャーン環境（Chrome 連続フォーカス
  イベント=BUG-37、UWP 2段フォーカス、通知フォーカスチャーン=BUG-57）で
  撤去前より高頻度（20〜50ms 間隔）の force-ON 連打を招く恐れがあったため、
  実送信のレート制限を追加した。
- force-ON が `note_explicit_ime_action` を呼んでおらず
  `kp_stage_idle_conv_check` の汚染防止ガードを素通りしていた問題、
  および force-ON 経路が常に `belief_input_mode: Unknown` を使うため
  `ObservedKana` 保護が一度も効いていなかった問題を修正した。

**実機ソークで確認すべき項目に追加**（上記1〜4に加えて）:
5. force-ON 1回あたりの `kp_run_inner` 滞在時間（item 0 未移行の影響、
   ~100ms 程度が乗る想定）。
6. Alt+Tab で Tab を連打したとき、force-ON（`ForcePolicyResend`）ログが
   中間ウィンドウ宛に出ていないか。
7. TsfNative × force で `[drift] correction:` が周期で実際に発火するか
   （`reschedule_ime_refresh` 例外復元の効果確認）。

**関連:** BUG-59 とその追補（同時期の報告、ターゲット競合の疑い）、BUG-08
（外部注入 `VK_KANA` による JIS かな化の既知パターン）、
[ADR-086](adr/086-force-write-trigger-and-target-identity.md) §1.3（未確定の
仮説として同じ整理を記載）・§5 Phase 2（MS-IME で force-write が初めて発火する
実装、追補1参照）・§5 Phase 3（open/close 軸のトリガー是正、追補2・3参照）・
§7-11（2回目レビューの詳細な経緯）・§7-12（M5 の未解決論点）。

## BUG-61: Windows Terminal + MS-IME で JIS かな入力に固定され、Ctrl+変換も tray の
ローマ字/かなコマンドも復旧できない（`ImmSetConversionStatus` 書き込みが実モードに
反映されない疑い、手動 VK 注入ハーネスを追加・自動復元は未実施）

**症状（2026-08-08〜09 実機報告、Windows Terminal + MS-IME、conv_mode_policy=force
ソーク中）:**

- typing 中（idle ではない）に `[idle-conv-check] TsfNative: conv=0x00000009 →
  belief ObservedRomaji 変更なし` が繰り返し出続ける。conv=0x9 は
  `NATIVE(0x1)|FULLSHAPE(0x8)` = ひらがな charset だが `ROMAN`(0x10) ビットが無い、
  すなわち JIS かな直接入力状態。
- Ctrl+変換（IME ON 中のローマ字/かな切替キー）を押してもローマ字に戻らない。
- awase の tray メニュー「ローマ字」「かな」を選んでも同様に切り替わらない。

BUG-60 が「force-write ソーク中に MS-IME 側で JIS かな化する」と予告していた
シナリオが、ADR-086 Phase 2/3 実装後に実際に踏まれた最初の実例と位置づけられる。

**確定した事実（コード読解 + 実機報告の突き合わせ）:**

- tray の `InputRomaji`/`InputKana`（`message_handlers.rs:641-650`、修正前）は
  `set_ime_romaji_mode_state_for_target(hwnd, romaji)` で `ImmSetConversionStatus`
  を呼ぶだけで、宛先 `hwnd` は `tray::menu_target_hwnd()`
  （メニュー表示前に捕捉した実フォーカスウィンドウ）。これは idle-conv-check が
  conv を読んでいるのと**同じ hwnd**であり、「宛先がズレていて効かない」という
  説明は成立しない。ユーザーが tray 操作でも直せなかったことは、
  **`ImmSetConversionStatus` による ROMAN ビット書き込みそのものが実モードに
  反映されていない**ことの強い証拠になる。
- 別途、tray の `ImeHiragana`/`ImeFullKatakana`/`ImeHalfKatakana`（修正前）は
  `IME_CMODE_ROMAN` に一切触れておらず、JIS かな状態で選んでも JIS かなのまま
  だった（`ResetState` だけが ROMAN を明示的に立てていた）。これは今回の主症状
  とは独立した見落としだが、同じコミットで修正した（下記「対応」参照）。
- `conv_mode_policy=force` の書き込み値自体は無罪: `ConvMode::to_conv_bits()`
  （`src/engine/conv.rs:158`）は `romaji==true` のとき必ず `IME_CMODE_ROMAN` を
  含み、`desired_mode` の唯一の書き込み点 `set_desired_conv_mode` は常に
  `romaji: true` を渡す（BUG-60 追補1で確認済みの事実の再確認）。force-write が
  ROMAN ビットを消しているのではない。
- `VK_DBE_ROMAN`(0xF5)/`VK_DBE_NOROMAN`(0xF6) が MS-IME の TSF ハンドラ上で
  「方向指定」なのか「トグル」なのかは**未確認**。既存の類似実装
  `kp_restore_kana_from_half_width`（shift-conv-guard 解放時、MS-IME 限定）は
  `VK_DBE_HIRAGANA` を scan コード付き SendInput で注入して charset 切替を
  復旧させており、scan=0 では MS-IME/TSF がモードキーとして処理しない
  （2026-07-07 実機確認）という制約が既知。

**対応（Opus 設計相談 + Fable PM プランニング、2026-08-08〜09、
`fix/tray-romaji-vk-dbe-roman`）:**

`is_roman_reliable=false`（`state/conv_classify.rs`、TsfNative idle 経路の
自動判定を常に無効化する既存ガード）の解除や、idle-conv-check からの自動
VK_DBE_ROMAN 発火にはまだ踏み込まない — VK_DBE_ROMAN が方向指定かトグルか
未確認のまま自動化すると、正常なローマ字入力を誤ってかな化させる新しい
往復ハザードを生みかねないため。代わりに、まず **tray コマンドを実機テスト
ハーネスとして使う**最小安全な第一歩を実装した:

1. `vk.rs` に `VK_DBE_ROMAN=0xF5`/`VK_DBE_NOROMAN=0xF6` を定義。
2. `Runtime::tray_inject_romaji_mode_vk`（`key_pipeline.rs`）を新設し、tray の
   `InputRomaji`/`InputKana` から既存の IMC write と**併走**で呼ぶ。MS-IME 限定
   （GJI 未検証）、`effective_open()==false` なら注入をスキップ（BUG-15 追補7の
   かなロックトグルハザード対策）、scan コードは `MapVirtualKeyW` から取得し
   scan=0 なら `VK_KANA` の JIS scan（0x70）にフォールバック。
3. `ImeHiragana`/`ImeFullKatakana`/`ImeHalfKatakana` の ROMAN ビット欠落を修正。

自動復元（idle-conv-check や `conv_mode_policy=force` からの自動発火）への
配線は**まだ行っていない**。

**実機確認チェックリスト（ユーザーが tray 操作で確認すること）:**

1. JIS かな固定状態で tray「ローマ字」→ ローマ字入力に復帰するか（本命）。
2. tray「かな」→ JIS かなに切り替わるか。
3. ローマ字状態で「ローマ字」を連打 → かな化しないか（**最重要**:
   `VK_DBE_ROMAN` がトグルなら化ける。この結果が自動復元解禁の可否を左右する）。
4. 「ひらがな」コマンドでも JIS かなから復帰するか（ROMAN ビット修正の確認、
   兼 IMC write 単体の有効性の追加証拠）。
5. IME 実 OFF 中に tray 操作 → 注入スキップログが出て副作用がないか。
6. ログの `MapVirtualKeyW` scan 値（0 ならフォールバック経路が動いたか）。
7. `conv_mode_policy=force` 運用中、tray で復帰後に別ウィンドウへ切り替えると
   再度 JIS かな化しないか（force のフォーカス変更時強制書き込みが ROMAN
   込みであることは上記で確認済みだが、念のため実機で観察）。

**やらないこと（スコープ外の明示）:** 自動復元、`is_roman_reliable=false` の
解除、往復ハザード対策（ヒステリシス・give-up ラッチ）、GJI 対応、
`kp_restore_kana_from_half_width` への ROMAN 注入追加。いずれも上記チェック
リストで VK_DBE_ROMAN の挙動（方向指定/トグル）が確定してから着手する。

**関連:** BUG-60（同じ「JIS かな化」症状群、force-write ソーク中の予告
シナリオ）、BUG-08（`VK_KANA` 注入による JIS かな化の既知パターン）、
BUG-15 追補7（DBE 系 VK 注入のかなロックトグルハザード）、
[ADR-086](adr/086-force-write-trigger-and-target-identity.md) §4 INV-13
（SendInput のターゲット同一性検証が構造的に適用不能という既存の例外）。

