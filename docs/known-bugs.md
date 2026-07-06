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
   engine open 中に「ひらがな conv から ROMAN が落ちる変化」を観測したら、conv 権限
   （`conv_mutation_allowed`）確認の上 `set_ime_romaji_mode_with_target_async(None)`
   （現 conv | ROMAN、冪等）で復元する。`conv_mode_changed` の遷移時のみ発火するため、
   ROMAN を常に 0 で返す環境でも送信スパムにならない。

**再発防止テスト:** `state/conv_classify.rs` の unit tests（`jiskana_transition_while_open_requests_restore_roman` 等 6 件 + 不変条件）、
`tests/journals/jiskana-vk-kana-injection.json`（実機ログからの リプレイ fixture、Linux でも実行可）。

**関連ファイル:** `hook.rs`（swallow）、`state/conv_classify.rs`（検出）、
`runtime/key_pipeline.rs`（Apply(3) 復元）、`ime.rs::set_ime_romaji_mode_with_target`

**残存リスク:** 注入元が VK_KANA 以外の経路（ImmSetConversionStatus 直叩き等）でかな化
させる場合は hook 層では防げないが、その場合も idle-conv-check 層が数秒で復元する。
物理 VK_KANA を意図的に押した場合も同様に復元される（awase engine ON 中の JIS かな入力は
サポートしない設計判断）。

---

## BUG-09: アクティブ IME 種別の誤検出（実際は MS-IME なのに ime=GJI）【調査中】

**症状:** 2026-07-06T04:15 セッション（Windows Terminal）で、実際のアクティブ IME は
MS-IME（ユーザー確認済み、GJI は Converter プロセス常駐のみ）なのに、awase は
`[key-output] ... ime=GJI` と認識し GJI 戦略で動作した:
`[gji-fsm] StartProbe` → GJI I/O 静止（`gji_idle=200000ms+`）→ `PendingGjiConfirm:
GJI 未応答 → unicode で強制送信`。warmup・IME ON/OFF キー選択の戦略分岐がすべて
誤った側に倒れる。同日 03:43 の別セッション（修正前ビルド）では `ime=MicrosoftIme` と
正しく検出されており、**プロセス再起動をまたいで検出結果が反転している**。

**有力仮説:** `tip_detector::query_active_kind` は gji-io-monitor **ワーカースレッド**から
`ITfInputProcessorProfileMgr::GetActiveProfile` を呼んでいるが、TSF の入力プロファイルは
**スレッドごと**の状態。フォーカス・メッセージポンプを持たないワーカースレッドは
ユーザーのインタラクティブな IME 切替（GJI⇔MS-IME）に追随せず、スレッド生成時点の
プロファイル（またはユーザー既定）を返し続ける。awase の起動タイミングにより
セッション間で結果が反転することも説明できる。

**確認方法（次回セッション）:** ログを `grep "tip-detect"` — `initial IME kind:` が
起動時の判定、`IME kind →` が 2 秒ポーリングでの変化。実 IME を切り替えても
`IME kind →` が一度も出ないなら per-thread 固着で確定。

**影響:** warmup 戦略（GjiFsm vs MsImeStrategy）、IME ON/OFF キー選択
（`characterize_strategy`）、GJI observe の適用可否など、ActiveImeKind 分岐のすべて。

**関連ファイル:** `tsf/tip_detector.rs` (`query_active_kind`),
`tsf/gji_monitor.rs::monitor_loop`（呼び出しスレッド）, `tsf/observer.rs` (`active_ime_kind`)

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
