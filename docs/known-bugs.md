# awase 既知の不具合

> 最終更新: 2026-07-06

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

**恒久対策（推奨）:** MS-IME 側の「Shift キー単独で英数モードに切り替える」を
無効化すれば誤認の芽そのものが消える（二重オーナー解消。`msime_key_assignment.rs`
の検出ポップアップと同系）。修正 2 はその設定が有効なままでも壊れないための防御。

**再発防止テスト:** `src/engine/tests.rs`（`test_shift_face_fullwidth_ascii_becomes_halfwidth_text` /
`test_shift_face_halfwidth_disabled_keeps_literal` / `test_shift_face_kana_stays_ime_routed`）。
復元側（Windows cfg 下）は本エントリ + `[shift-release]` ログで検知する。

**関連ファイル:** `src/types.rs`（`KeyAction::Text`）、`src/engine/nicola_fsm.rs`
（半角化）、`src/config.rs`（`shift_plane_halfwidth`）、
`runtime/key_pipeline.rs::kp_stage_shift_plane_release`（復元）、
`state/platform_state.rs`（`GateStore::shift_plane_used_in_hold`）、
`tsf/ime_mode_fsm.rs::unconfirm`、`output/mod.rs`（Text 送信）

**関連バグ:** BUG-14（Shift 相関の外部注入）、MS-IME 二重オーナー問題

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
| `[shift-release] Shift 面使用後の解放 → MS-IME 英数誤切替の先回り復元` | BUG-15 カウンター発動（conv をかな入力へ verify-retry 復元） |
| `→ Text("...") via Unicode direct` | Shift 面半角英数の IME 非経由リテラル出力 |
