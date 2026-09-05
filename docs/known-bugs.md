# awase 既知の不具合

> 最終更新: 2026-08-27

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
   **【2026-08-17 撤去（BUG-61）】** 層2（`restore_roman`）はこの時点で既に
   構造的に発火しえなくなっていた——`classify_conv_transition` の唯一の本番
   呼び出し元 `kp_stage_idle_conv_check` は TsfNative アプリ限定（ガード2）かつ
   `is_roman_reliable` を常に `false` で渡すため。加えて BUG-61 の実機検証で、
   そもそも復元の書き込み手段（IMC write・VK 注入）自体が Windows Terminal +
   MS-IME で一切効かない（Win32 に入力方式を切り替える公式 API が存在しない）
   と確定した。この二重の理由（構造的に発火しない・発火しても効かない）から
   `restore_roman` フィールド・関連コード・unit test 6本・`ROMAN_RESTORE_MIN_
   INTERVAL_MS`・`last_roman_restore_ms` を全て撤去した。**唯一の防御は
   hook 層1（VK_KANA swallow）のみになった。**

**再発防止テスト:** `state/conv_classify.rs::exhaustive_classify_conv_transition_matches_independent_oracle`
（全数網羅オラクル、層2撤去後も engine 同期ロジックの回帰は引き続きこれで検知する）、
`tests/journals/jiskana-vk-kana-injection.json`（実機ログからのリプレイ fixture、Linux でも実行可。
層2撤去に伴い期待値からは `restore_roman` を削除済み、反転記録として本文は残してある）。

**関連ファイル:** `hook.rs`（swallow、唯一現存する防御）、`state/conv_classify.rs`（検出、
`restore_roman` は撤去済み）

**残存リスク（2026-08-17 訂正）:** 注入元が VK_KANA 以外の経路
（`ImmSetConversionStatus` 直叩き等）でかな化させる場合、または物理 VK_KANA を
意図的に押した場合、hook 層の swallow をすり抜けると **awase 側に復旧手段は
存在しない**（BUG-61 で確定）。旧記述「idle-conv-check 層が数秒で復元する」は
誤りだった——この誤った前提のまま `hook.rs` が別の物理キー（Alt+かな）を
素通しする判断根拠にも使われ、BUG-62（無回復のまま JIS かな化する事例）が
2026-08-09 まで見過ごされる一因になった。唯一の確実な回避策は、MS-IME の
言語バー等で**ユーザー自身が手動で切り替える**ことのみ（BUG-61 参照）。

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

**追補9（撤去、2026-08-09）: チョード安全網（Shift+文字キーのたびに無条件で
conv を英数へ→かなへ書き戻す仕組み）そのものを撤去。左Shift単独タップの
持続トグル（BUG-25）は維持し、entry write のタイミングを Shift down → Shift up
（単独タップ確定時）へ移した。**

**症状（2026-08-09 ユーザー報告ログ）:** LINE（`Qt663QWindowIcon`、Qt/ImmCross）
で NICOLA 小指シフト面の `Shift+1`（`.yab` で `'！'` = クォート付き全角リテラル）
を打つと、awase 自身のログでは `send_keys: mode=Unicode actions=[Char('！')] →
Char('！') via Unicode` と全角のまま正しく Unicode 直接注入しているにも
かかわらず、LINE 上の表示は半角 `!` になる。**Windows Terminal（TSF-native）
では同じチョードで再発しない**（ユーザー確認済み、同一セッション内）。ログの
タイムスタンプ突合で、`Char('！')` の送出は `[shift-conv-guard] Shift 押下 →
IME-ON 半角英数へ切替 (conv→0x00000000)` の IMC write 完了後・
`[shift-conv-guard] かな入力へ復元` の約 200ms 前という、conv が英数
（NATIVE=false）になっている窓の中で発生していた。

**原因（推定、Windows Terminal と LINE の差から消去法で特定）:** BUG-25 で
ASCII 素通し経路（`shift_plane_halfwidth`）を撤去して以降、`shift_face_reduce`
は `.yab` Shift 面の値（クォート付き全角リテラル含む）を常に `Reduce` して
Unicode 直接注入するだけになっており、チョード安全網の conv=0x0000 先書き込み
はチョードの出力そのものには一切必要が無くなっていた。一方 LINE
（Qt ベースの ImmCross アプリ）は、自前の IME 統合レイヤーが挿入直前の
IME conversion mode（NATIVE ビット）を見て文字の全角/半角を自前で正規化して
いると推測される — awase が一時的に conv を英数化した窓に全角記号が着弾した
ため、LINE 側で半角化されたとみられる。TsfNative（Windows Terminal）は
conv mode を見た幅の再正規化を行わないため症状が出ない、という差分と整合する。

**BUG-58 との関係:** チョード安全網の conv=0x0000 先書き込みは、
[BUG-58](#bug-58-小指シフト面のチョードshift数字等がoutputactiveguardとshift-conv-guard復元の循環待ちに陥り通常速度の打鍵でも毎回-5秒フリーズする対応済み実機未検証)
（Shift+数字等のチョードが `OutputActiveGuard` と shift-conv-guard 復元の
循環待ちに陥り毎回 ~5 秒フリーズする）の直接の引き金でもあった。BUG-58 の
修正（案E、`38b5a4ee`）は `OutputActiveGuard` の取得タイミングをずらして
循環そのものを解消したが、引き金（チョードのたびの先書き込み自体）は
残っていた。本追補でチョードに対する先書き込みを撤去したことで、この
引き金自体が構造的に消える（案E の修正は持続トグル側の経路には引き続き
必要なため撤去していない）。

**対応:** `kp_shift_conv_guard_key_down`（`runtime/key_pipeline.rs`）から
判別未確定のままの conv=0x0000 先書き込みを撤去した。左Shift単独タップに
よる半角英数持続トグル（BUG-25）の conv=0x0000 書き込みは、単独タップと
確定した瞬間（`kp_shift_conv_guard_key_up`）に一本化して移動した。チョード
（トグル非アクティブ時の Shift+文字キー）は conv に一切触れず、
`kp_restore_kana_from_half_width` も呼ばれない（何もしない）。

**既知のリスク（未検証、次回実機セッションで確認すること）:** 本追補で
撤去したのは BUG-15 本体の対策（MS-IME 自身の「Shift 単独タップで英数モードに
切替える」誤検知を先回りして打ち消す仕組み）そのものであり、Shift+文字キーの
チョード直後に限り BUG-15 の症状（idle-conv-check が ObservedEisu →
DirectInput → Engine OFF まで連鎖し、数秒〜十数秒かな入力が壊れる）が
再発する可能性がある。ただし BUG-15 発覚時（2026-07-07）以降、
`idle-conv-check` 側の安全策（BUG-57 の eisu 汚染修正等）が複数回入っており、
同じ連鎖が今も成立するかは不明。実機で「小指シフト面のチョードを連打した
直後のかな入力」を重点的に確認すること。再発した場合は本追補を revert せず、
BUG-15/BUG-25 の失敗条件（アプリ・IME・再現手順）を追記した上で、チョード
種別（記号 vs 英字）や IME 種別で分岐する、より狭い対策を検討する
（`.claude/rules/experiment-logging.md` 参照）。

**テスト:** `crates/awase-windows/tests/golden_scenarios.rs`
`scenario_15_half_width_alnum_toggle_keeps_ime_open_while_engine_goes_inactive`
は entry write の移動後も green（belief 遷移の核心部分）。`kp_stage_shift_conv_guard`
自体のタップ/チョード判定・LINE での幅再現は BUG-25 と同様、Windows 実機
フック依存のため自動テスト不可——本追補が再発防止の記録。

**関連ファイル:** `crates/awase-windows/src/runtime/key_pipeline.rs`
（`kp_shift_conv_guard_key_down`/`kp_shift_conv_guard_key_up`）

**関連バグ:** BUG-15（本体）、BUG-25（持続トグル）、BUG-58（先書き込みが
引き金だった循環待ちフリーズ）

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

**ADR-106 決定3 との関係（2026-08-26 追記、未検証）:** 本バグの `AppKind`
往復（`TsfNative`⇔`Uwp`、`Windows.UI.Input.InputSite.WindowClass`⇔
`Chrome_WidgetWin_1`）は同一プロセス内での発生であり、`focus_epoch`
（`on_focus_process_changed` の `process_changed` 判定＝PID 変化でのみ進む）
が動かない可能性がある区間である。ADR-106 決定3は
`ObservationStore::derive_any()` の `ImmCrossProbe`/`FocusProbe` フィルタに
hwnd 照合を追加した——`AppKind` 往復の2つの window が異なる hwnd を持つ場合、
`focus_epoch` が変わらなくても hwnd 不一致で古い window の観測が棄却される
ようになる。これが本バグの症状（`belief` と `GjiFsm` の不一致期間）と
直接の因果関係を持つかは未確認。本バグ自体は `gji_on_ime_on` 呼び出し追加で
別経路から修正済みのため新たな実害があるわけではないが、`derive_any()` の
挙動変化が `AppKind` 往復シナリオで意図しない副作用（stale 観測が想定より
多く/少なく棄却される）を起こさないか、次回この往復パターンが実機ログで
観測された際に確認すること。

**ADR-106 決定3 の退行修正（2026-08-26 追記、code review・2026-08-26 実機検証済み）:** 上記の hwnd
照合追加自体に、機能回帰が1件あった。`ObservationStore::current_focus_hwnd`
（`derive_any()` の照合対象）は `FocusChanged`（プロセス変更時のみ発火）経由の
`clear_on_focus_change()` でしか更新されていなかった一方、`ImmLikeTicket::admit()`
が照合する「生の hwnd」（`Runtime::focus_hwnd()` → `platform.focus.current.hwnd`）は
`advance_focus_tracking()` 経由で毎 focus tick 更新されていた。このため、本バグと
まさに同じシナリオ（同一プロセス内で `AppKind` 往復や通常のウィンドウ切り替えが起き、
PID は変わらない）で `admit()` は新しい hwnd を正しく受理するのに `derive_any()` は
古い hwnd と比較し続け、以後の `ImmCrossProbe`/`FocusProbe` 観測を次のプロセス変更
まで恒久的に拒否する退行になっていた。修正: 同一プロセス内の hwnd 変化を
`ImeEvent::FocusHwndUpdated` として dispatch し、`ObservationStore::update_focus_hwnd()`
（epoch・観測プールには触れない）で `current_focus_hwnd` を追従させる
（`crates/awase-windows/src/runtime/focus_tracking.rs::advance_focus_tracking`、
`crates/awase-windows/src/state/observation_store.rs`）。state 層の回帰テスト
（`update_focus_hwnd_unblocks_derive_any_after_intra_process_window_change` 等）は
追加済み・Linux で green。

**実機検証（dragonflyg4、2026-08-26）:** 検証には Windows Terminal（TsfNative、決定2により
観測不能プロファイルのため FocusProbe/ImmCrossProbe 自体を記録せず不適）・現行メモ帳
（Windows 11 で IMM32 ベースでなくなっているため不適）のどちらも使えず、`crates/
awase-windows/examples/two_imm32_windows_probe.rs`（同一プロセス内に素の `EDIT`
コントロール持ちウィンドウを2つ作る使い捨て検証アプリ）を新設して使用した。Alt+Tab
による切替は**スイッチャー UI が別プロセスとして一瞬フォーカスを持つため
`process_changed=true` として記録されてしまい、本バグの再現条件（同一プロセス内・
PID 不変）を満たさなかった**——同種の検証を今後行う場合はこの点に注意し、マウス
クリックによる直接切替を使うこと。クリック切替では `process_changed=false` のまま
`FocusHwndUpdated` が2ウィンドウ間で6回以上正しく dispatch・追従し、その間
`derive_any()` の hwnd 不一致除外は一度も発生しなかった（一時追加した診断ログ
`[focus-hwnd-track]`／`[identity-gate]` で確認）。`runtime/` の実配線を含め修正効果を
実機で確認済み。

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

**追補4（実機確認・2026-08-27、mozc(google/mozc)ソース調査+6経路の実機検証）:
GJI entry は `SendInput` 経由で実現可能。追補3 の「SendInput は scan の値を
問わず再試行しないこと」という教訓を部分的に訂正する——scan ではなく、
**awase 自身の `transport::PhysicalKeyDisposition::plan`（`dbe_mode_key_policy
=Suppress` 既定ポリシー）が犯人だった。**

**背景:** ユーザーから「mozc の GitHub ソースを読んで、半角英数にする冪等
キーがないか調べてほしい」との依頼を受け、`google/mozc` の以下を調査した:

- `src/win32/tip/tip_lang_bar.cc`/`tip_lang_bar_callback.h`/
  `tip_text_service.cc`: 言語バーの入力モードボタン
  （GJI ビルドの GUID `{D8C8D5EB-8213-47CE-95B7-BA3F67757F94}`、
  `kTipLangBarItem_Button`）の `ITfLangBarItemButton::OnMenuSelect` が
  `TipEditSession::SwitchInputModeAsync` を経由して
  `SessionCommand::SWITCH_COMPOSITION_MODE` を送る経路を発見。
- `src/session/keymap.h`: `PrecompositionState::Commands::
  COMPOSITION_MODE_HALF_ALPHANUMERIC` という、`TOGGLE_ALPHANUMERIC_MODE`
  とは別の**真に冪等な**(SET であり toggle ではない)コマンドが存在するが、
  出荷版キーマップ(`ms-ime.tsv` 等)ではこのコマンドにデフォルトで
  どのキーも割り当てられていない(config1.db のカスタムキーマップ編集が
  必要——これは
  [[project_ime_key_danger_classification_and_roadmap_2026_08_11]] で
  「復活させない」と判断済みのため対象外とした)。
- `src/session/session.cc`: `Session::CompositionModeHalfASCII` /
  `Session::ToggleAlphanumericMode` の実装を確認。前者は
  `SwitchInputMode`(`if (composer->GetInputMode() != mode) SetInputMode(mode)`)
  で真に冪等、後者(`Eisu` キーの実体)は `composer->ToggleInputMode()` で
  無条件トグル。
- `src/composer/composer.cc`: `Composer::SetInputMode` は
  `composition_.SetInputMode(...)` と `is_new_input_ = true` を設定するのみで、
  **既存の未確定文字列(preedit)を書き換えない**——Composition 中に送っても
  非破壊であることをソースで確認(実機でも3回とも非破壊を確認)。

**実機で試した6経路の結果(対象: Windows Terminal、`Windows.UI.Input.
InputSite.WindowClass`、TsfNative プロファイル):**

| # | 経路 | Precomposition | Composition | 備考 |
|---|---|---|---|---|
| 1 | `ITfLangBarItemButton::OnMenuSelect`(クラシック言語バー GUID) | `Ok(())`だが実効なし | 未検証 | 2回実施、3秒 edit session 待機を挟んでも不変 |
| 2 | 同上(`GUID_LBI_INPUTMODE`、Win8+タスクバー版) | `Ok(())`だが実効なし | 未検証 | 原因不明のまま棚上げ |
| 3 | `PostMessageW`(`WM_KEYDOWN`/`UP`, scan=0)、leaf hwnd | **成功**(トグル確認) | 非破壊・実効なし | leaf=`Windows.UI.Input.InputSite.WindowClass` |
| 4 | 同上、`DesktopWindowContentBridge`(親hwnd)宛て | 未検証 | 非破壊・実効なし | hwnd を変えても結果不変(同一プロセス/スレッドのため) |
| 5 | `SendInput`(scan=0)、**awase 起動中** | 未検証 | 非破壊・実効なし | 追補1・3 と同一条件を再現 |
| 6 | `SendInput`(scan=0)、**awase 停止中** | 未検証 | **成功**(半角英数に確定) | 本追補の核心。awase 再起動→再試行で再現(失敗)し、A/Bで確定 |

**真因の特定:** 経路5と6の差は「awase が起動しているか」だけであり、
これは今セッション前半の BUG-90 調査で読んだ
`transport::PhysicalKeyDisposition::plan` のロジックと完全に符合する:
GJI が active な場合、`Imm32Unavailable`/`TsfNative` プロファイルでは
`dbe_mode_key_policy=Suppress`(既定値)のとき DBE モードキーの KeyDown は
**物理由来か注入由来かを問わず常に Suppress される**(BUG-52 対策の
`is_dbe_mode_key_down` 条件)。つまり awase 自身の低レベルフックが、
今回 `SendInput` で送った `VK_DBE_ALPHANUMERIC` を「外部からの生の DBE
キー」として検出し、実 IME(GJI)へ到達する前に握り潰していた。追補1・3の
「SendInput が awase 自身のフックにすら届かない」という当時の記述は事実
としては正しかったが、その原因の解釈(OS/ドライバ層での構造的な握り潰し)
は誤りだった可能性が高い——実際には awase 自身の transport 層の意図的な
安全機構(Suppress ポリシー)が働いていた。

**実装への示唆(未実装、次回セッション向け):**

1. **アクチュエーション経路は `SendInput`(scan=0、`VK_DBE_ALPHANUMERIC`)を
   使う。** `PostMessageW` は Composition 中に効かないため不採用、
   `OnMenuSelect` は原因不明のまま3回失敗しているため不採用。
2. **`transport::PhysicalKeyDisposition::plan` の DBE Suppress ポリシーが、
   awase 自身が意図的に発行する GJI entry 用の `SendInput` まで巻き込んで
   しまわないよう、除外経路が必要。** 既存の自己注入フィルタ(`hook.rs`、
   他の awase 発行 SendInput イベントを識別する仕組み)と同種の扱いを、
   この新規注入にも適用する設計が要る。
3. **トグルであり冪等ではない**ため、発火は awase 自身の belief(左Shift
   単独タップの遷移検出、`half_width_alnum_toggle_active` の false→true
   遷移)でガードし、1回だけ送る設計にする(mozc 側の冪等コマンドを
   使う設計は今回すべて失敗したため)。
4. **Composition 中でも安全に送れる**(preedit を破壊しない)ことをソース
   ・実機の両方で確認済みだが、UX 上は「候補ウィンドウ表示中は発火させない」
   等のガードを設ける方が無難(不意に今後の入力モードだけが変わる違和感を
   避けるため。既存の `gji_candidate_visible` 相当の観測が使える)。
5. 観測側の副産物として、`ITfInputProcessorProfileActivationSink`
   (`--watch-profile`)/`ITfLangBarItemSink`(`--watch-langbar`)による
   push 通知購読の実装・cross-process 到達性は本追補では未検証のまま
   (アクチュエーション経路の確定を優先したため)。次に着手する価値はある。

**テスト:** 自動テスト不可(実機の GJI/TSF/Windows Terminal 挙動に依存)。
使い捨ての実機検証用 example(`crates/awase-windows/examples/
spike_langbar_input_mode.rs`)を診断ツールとして残置(`--select`/
`--select-inputmode`/`--postmsg`/`--postmsg-idempotent`/`--postmsg-hwnd`/
`--sendinput`/`--watch-profile`/`--watch-langbar`/`--enum-ancestors` の
各モード)。次回 entry 実装時の実機再検証に再利用できる。

**関連ファイル(追補4):**
`crates/awase-windows/examples/spike_langbar_input_mode.rs`(新規、診断
ツール)、`crates/awase-windows/src/runtime/transport.rs`
(`PhysicalKeyDisposition::plan`、真因)、`crates/awase-windows/src/
runtime/key_pipeline.rs`(`kp_shift_conv_guard_key_down`、次回実装対象)。

---

**追補5（実機確認・2026-08-27、[ADR-107](adr/107-bug25-gji-half-width-alnum-entry.md)
決定0 の2×2切り分け計測 + awase debug ログ調査）: 追補4 の真因記述を ADR-107 が
訂正した内容を実機で裏付け、加えて新たな構造的欠陥（`SendInput` の KeyUp が
awase 自身のフックに構造的に届かない）を発見した。**

**背景:** ADR-107 は追補4 の「`dbe_mode_key_policy=Suppress` が真因」という
記述が、awase 自身のマーカ付き注入（追補1・3 の `TSF_MARKER` 付き注入）には
当てはまらないと `hook.rs::is_self_injected` のソース照合から指摘した
（マーカ付き自己注入は `transport::plan` より前で `CallNextHookEx` に落ちる
ため、構造的に Suppress され得ない）。この訂正を実機で検証するため、
決定0 が指定する2×2計測（`--sendinput-marker=<none|ime_kanji>` ×
`--sendinput-shift-held` を spike に追加、`RUST_LOG=debug` で awase を再起動
してログを突き合わせた。

**M1〜M3 の実機結果:**

| # | marker | awase | shift-held | 結果 |
|---|---|---|---|---|
| M1 | `none` | 停止 | 無し | **成功**（追補4 経路6 の再確認） |
| M2 | `none` | 起動 | 無し | **失敗**（ひらがなのまま。追補4 経路5 の再現、`dbe_mode_key_policy=Suppress` による） |
| M3 | `ime_kanji` | 起動 | 無し | **成功**。2回連続の再試行でも正しくトグルした（ひらがな→英数→ひらがな） |

M3 の成功は ADR-107 決定2（`IME_KANJI_MARKER` を使えば transport バイパス
無しで足りる）を実機で裏付けた。M4（Shift 押下中）は下記の別発見のため
未実施のまま保留。

**新発見: `SendInput` の KeyUp がフックに構造的に届かない。** `RUST_LOG=debug`
の awase ログを `[hook] IME-mode vk=0xF0` で grep したところ、M3 で2回送った
マーカ付き（`extra=0x4B45594A`）注入はいずれも **KeyDown だけがログに現れ、
対応する KeyUp が一度もログに現れなかった**:

```
[hook] IME-mode vk=0xF0 down self_injected=true injected=true scan=0x0 extra=0x4B45594A   ← 私たちの DOWN
（対応する up が一切現れない）
```

DOWN と UP を同一 `SendInput` バッチで送る場合（`up_delay_ms=0`）と、別々の
`SendInput` 呼び出しに分割し実時間 50ms を空ける場合（`--sendinput-up-delay-ms=50`、
本追補のために spike へ追加）の両方で同じ結果になった——**タイミング/バッチ化は
原因ではない。** `SendInput` 自体は毎回 `sent=1/1`（または `2/2`）を報告して
おり、Windows 側では受理されている。ログに `vk=0xF0 up self_injected=false
injected=false scan=0x70 extra=0x0` という行が数秒〜十数秒後に現れることは
あったが、これは `self_injected=false`（外部由来）かつ `scan=0x70` の**無関係な
別の物理キーイベント**であり、私たちの注入とは無関係と判断した（`imm32-off`
の Suppress ログが直後に付随することからも、BUG-52 対策が正しく効いている
実物理 DBE キーだと分かる）。

**原因（推定、未確定）:** `VK_DBE_ALPHANUMERIC` は `wScan=0` を指定しても実
物理キーに対応しないため、Windows の内部キー状態追跡（キーが「押されている」
かどうかの管理）がこの VK に対して機能しておらず、対応する KeyUp イベントが
低レベルフックチェーンへ配送される前に握り潰されている可能性が高い。

**実害の評価:** モード切替（トグル）自体は毎回成功しており、KeyUp 欠落が
トグルの成否には影響していないと見られる。ただし副作用として2点を観測した:

1. **1文字目だけ全角英数になるレース**: M3 成功直後に続けて打鍵すると、
   最初の1文字だけ全角英数（`FULL_ASCII`）になり、2文字目以降は正しく
   半角英数（`HALF_ASCII`）になった。`VK_DBE_ALPHANUMERIC` のトグルは
   `HIRAGANA`⇔`HALF_ASCII` のみを行うはずで `FULL_ASCII` を生成しない
   （`session/session.cc::Session::ToggleAlphanumericMode`）ため、KeyUp
   欠落による GJI 側のキー状態不整合が関与している可能性がある。
   `--sendinput-up-delay-ms=50` で明示的に間隔を空けた再試行ではこの
   レースは再現しなかった（1文字目から一貫して半角英数）——これ自体は
   タイミング改善で緩和できる可能性を示唆するが、サンプル数が少なく
   確定的な結論ではない。
2. **連続注入時の不安定化**: M3 を3回連続で実行したところ、3回目で
   「一瞬全角英数になった後 IME オフ・直接入力になる」という、単純な
   トグルでは説明できない挙動が発生した。ログの `[shadow-toggle]`/
   `[idle-conv-check]` 系の記録から、awase 自身の drift correction /
   idle-conv-check（GJI の実際の conv モード変化を監視し補正動作を打つ
   既存機構）が、短時間の連続トグルに反応して介入した可能性が高いと
   推定している（ログの完全な相関分析までは未実施）。復旧は物理
   `Ctrl+変換`/`Ctrl+Shift+変換` では効かず、GJI のタスクトレイアイコンを
   直接操作して初めて復旧できた。

**ADR-107 への影響:** 決定2（`IME_KANJI_MARKER` + synthetic Shift↑ 前置）は
実機で有効性を確認できたが、**KeyUp 欠落という新しい制約**を設計に反映する
必要がある。具体的には:

- entry 直後、GJI 側のモード切替が実際に完了するまでの短い猶予
  （settle 待ち）を置いてから最初の文字を送る設計が要る可能性がある
  （既存の warmup/cold-start パターンと同種の対策）。
- 短時間に連続してトグルを送らない（awase 自身の drift correction との
  相互作用を避ける）よう、entry 呼び出し自体に最小間隔のガードを検討する
  余地がある。
- KeyUp 欠落の根本原因（Windows 側の内部状態追跡）を回避する代替手段
  （例: `KEYEVENTF_SCANCODE` を併用する、非衝突な scan 値を敢えて使う等）
  は BUG-15 追補7/BUG-25 追補1 の CapsLock 衝突の教訓と矛盾しない範囲で
  今後検討する余地がある。

**未解決の疑問（追加）:**

- KeyUp が本当に「届いていない」のか、それとも「届いているがログの
  `ImeKeyKind::from_vk` 判定を素通りする別 VK に化けている」のかは未確定
  （`[hook] IME-mode` ログは `ime_key_kind.is_some()` の場合のみ発火するため、
  未知の VK への変換があれば見えない）。
- 「1文字目だけ全角英数」のレースが `--sendinput-up-delay-ms` で緩和される
  かは、サンプル数を増やした再現実験が必要。
- 連続注入時の不安定化が本当に drift correction / idle-conv-check由来かは、
  ログの完全なタイムライン相関（`[shadow-toggle]`/`[warrant-shadow]`/
  `[idle-conv-check]` 各行のシーケンス）を突き合わせるまで確定しない。
- M4（Shift 押下中）は当初この発見のため後回しにしたが、同日中に実施済み
  （下記追記参照）。原因B は確定した。

**追記（同日、Composition 中 + awase 起動中での M3 再検証、2件とも成功）:**
決定5 は「6経路すべてで Composition 中の実効性が否定的/不確定」という前提
（追補4 時点）で書かれていたが、その6経路はいずれも後に効かないと判明した
経路（`PostMessageW`、マーカなし `SendInput`）だった。M3（`ime_kanji` マーカ、
awase 起動中）で**初めて**「正しい注入方式」のまま Composition 中に発火させて
2回試したところ、**両方とも非破壊・成功**した（既存のプリエディットが壊れず、
続けて打った文字が半角英数になった。今回は「1文字目だけ全角英数」レースも
再現しなかった）。KeyUp 欠落（本追補の主題）自体は Composition 中でも同じ
パターン（DOWN のみログに出現）で再現しており、Precomposition との違いは
無かった。

サンプル数はまだ少なく（各条件2回のみ）決定的ではないが、**決定5（Composition
中は発火せずラッチもしない）を緩められる可能性を示す最初の肯定的データ**
として記録する。ADR-107 決定5 の「6経路すべてで確認できていない」という
根拠は、少なくとも M3 に関しては古い記述になっている。

**追記（同日、M4 実施——決定0 の2×2計測が完成）: `ime_kanji`/起動/Shift 押下中
は失敗し、ADR-107 原因B を実機で確定させた。** Precomposition・ひらがな状態
から、`VK_LSHIFT` を押下したまま `VK_DBE_ALPHANUMERIC` をマーカ付きで送った
ところ、**ひらがなのまま変化しなかった**（M3 と同一条件で Shift だけを
押下中にした差分）。

ログを確認したところ、**M1〜M3 では毎回確実に記録されていた
`[hook] IME-mode vk=0xF0 down self_injected=true ... extra=0x4B45594A` が、
M4 では一度も記録されなかった。** `ImeKeyKind::from_vk(0xF0)` は
`Some(Alphanumeric)` を返しこの debug ログは self_injected 判定より前で
無条件に発火するため、ログに出ないことは「イベントが awase 自身の
低レベルフックにすら到達していない」ことを意味する。すなわち原因B は
当初の仮説（「GJI が `Shift+Eisu` を未定義の組み合わせとして無視する」）
より根深く、**Shift が同時に押下されていると、`VK_DBE_ALPHANUMERIC` の
KeyDown イベント自体が OS レベルで(awase を含む)いかなるフックにも
配送されない**らしいことが実機で確認できた（正確な OS 側メカニズムは
未確認。「未解決の疑問」参照）。なお、注入した `VK_LSHIFT` 自体は
`ImeKeyKind::from_vk` の対象外のため `[hook] IME-mode` ログには元々
現れず、また自己注入は `[engine-input]` パイプラインも経由しないため、
Shift 注入そのものの成否はこのログからは確認できなかった。

この結果は ADR-107 決定2 の synthetic Shift↑ 前置を「望ましい」から
**「無いと entry が構造的に不成立になる必須要件」**へ格上げする、決定0 の
最終的な結論である。M4 の失敗後、物理 `Ctrl+変換`/`Ctrl+Shift+変換` は
やはり効かず、GJI のタスクトレイアイコン直接操作でのみ復旧した点は
追補4/5 で繰り返し観測しているパターンと一致する。

**テスト:** 自動テスト不可（実機の awase/GJI/Windows Terminal 挙動に依存）。
spike に `--sendinput-marker`/`--sendinput-shift-held`/`--sendinput-up-delay-ms`
を追加し再利用可能な形で残置。`RUST_LOG=debug` での awase 再起動手順
（`target/debug/awase.log`）は次回の実機検証でも同じ形で使えるよう記録した。

**関連ファイル(追補5):** `crates/awase-windows/examples/
spike_langbar_input_mode.rs`(`--sendinput-marker`/`--sendinput-shift-held`/
`--sendinput-up-delay-ms` 追加)、`docs/adr/107-bug25-gji-half-width-alnum-entry.md`
（決定0 の実機結果を受けた更新対象）。

**追補6（実装着手・2026-08-27、ADR-107 Task 1〜8）: GJI entry 本実装を
オプトインで追加し、実機検証チェックリストを未完了として残す。**

ADR-107 Task 1〜8として、`half_width_alnum_toggle` kill switch
（既定 `ms_ime_only`、GJIは `all` 明示時のみ）、`IME_KANJI_MARKER` 付き
`VK_DBE_ALPHANUMERIC` scan=0 + synthetic Shift↑ 前置のGJI entry、scan付き
`VK_DBE_HIRAGANA` のGJI exit、`mem::replace` による二重exit送信防止を実装する。
MS-IME既存経路の scan付きF2注入 + 160ms間隔4回のIMC write/verify-retryは
排他分岐として維持する。

Task 0のsettle値再測定と連続発火クールダウンの実測は未完了のため、本追補では
settle待ち・クールダウン定数を実装しない。BUG-25のクローズ判断は、下記Task 9
相当のWindows実機検証とソーク完了後に行う。

**実機検証チェックリスト（未実施、Task 9）:** 実行可能な手順に展開したものが
[bug25-gji-entry-verification-checklist.md](bug25-gji-entry-verification-checklist.md)。

- `[hook] IME-mode vk=0xF0 ... self_injected=` 行の出現有無。
- CapsLock 状態が変化していないこと。
- 実際に打鍵した文字が半角英数になること（IMC read-back を成功判定に使わない）。
- トグルON→フォーカス変更→戻る、を往復しても英数状態が持ち越されないこと。
- フォーカス変更先のアプリでexitが実行された際、切替先アプリのIMEが英数のままに
  なっていないこと。
- トグルON→右Shift緊急解除、とトグルON→2回目左Shiftタップ、の両方でかなに戻ること。
- Exit（`VK_DBE_HIRAGANA`, 0xF2）がscan付きで正しく実効すること。
- `VK_DBE_HIRAGANA` のkeymap束縛がComposition/Precomposition両方で
  `InputModeHiragana` として働くこと。
- Task 0（settle値の再測定）を経て `half_width_alnum_toggle = "all"` を明示設定した
  状態でソーク運用を開始すること。
- トグルON中に言語バー等でIME製品自体を切り替えた場合、exitが実際にentryした
  側のIMEへ正しく復元キーを送ること（追補8参照、entry/exit間のIME製品切替は
  現状 exit 時の再取得 `active_ime_kind` に依存しており構造的修正は未着手）。

**追補7（2026-08-27、Opus敵対的コードレビューで発見した3件のBLOCKERを修正）:**

追補6のコミット後、Codex実装の完成物に対して独立のOpus敵対的レビューを実施し、
以下3件の実装バグを発見・修正した（いずれもマージ前、コミット追加で対応済み）。

1. **composition ガードがMS-IME entryにも誤って掛かっていた。** `kp_shift_conv_guard_key_up`
   の`composing`変数を`plan_half_width_alnum_action`へ渡す際、GJI/MS-IMEを区別せず
   渡していたため、既定`ms_ime_only`のまま使っている全ユーザーの経路で「変換中に
   左Shift単独タップしても半角英数トグルへ入れない」という新規の回帰が生じていた
   （MS-IME entryは元々compositionを一切見ていない）。`uses_imc_conv_write`の間は
   常に`composing=false`を渡すよう修正。
2. **synthetic Shift↑が汎用`VK_SHIFT`＋LShift scan固定で、右Shift緊急解除の
   逃げ道が構造的に不発になりうる問題。** `MapVirtualKeyW(VK_SHIFT)`は左Shiftの
   scan(0x2A)しか返さないため、右Shift単独タップで緊急解除した場合、OS内部の
   `VK_RSHIFT`状態が更新されずShift押下中と誤認され続け、決定0 M4が実機で確定
   させた「Shift押下中はDBEキーのKeyDown自体がフックに配送されない」条件を踏む
   おそれがあった。`VK_LSHIFT`/`VK_RSHIFT`両方のsynthetic Shift↑を送るよう修正
   （KeyUpの重複は無害、既存の復元経路と同じ根拠）。実機での最終確認はTask 9に
   追加する。
3. **GJI exitのSendInputが見送られた（Win/Alt押下中 or effective_open=false）
   場合に、ラッチだけ消えてbeliefがAssumedRomajiへ進んでしまう問題。** MS-IME側は
   IMC writeの640msリトライという保険があるためこの穴が無いが、GJI側は
   `send_gji_half_width_alnum_toggle`が唯一の書き込み試行であり、失敗時に
   そのまま進めると「実GJIは半角英数のまま・engineはpass-throughを抜けて
   生ローマ字を送る」という追補3と同型の実害が再発しうる。送信失敗時は
   belief更新をスキップし`half_width_alnum_toggle_active`を`true`に戻して
   次の操作で再試行できるよう修正。合わせて`prepend_synthetic_shift_up`が
   exit側で常に`true`にハードコードされ、フォーカス変更由来の復元
   （物理Shiftが押されていないケース）でも余計なShift↑を切替先アプリへ
   注入していた問題も、呼び出し元の引数をそのまま伝播するよう修正した。

Opusは他にS-3(到達不能分岐2箇所)・S-5(ADR-084への例外追記漏れ)等のSHOULD-FIXも
指摘したが、正しさに影響しない/実機データが必要なため今回は見送り、Task 9の
チェックリストに反映済み。

**追補8（2026-08-27、PR #111を正しく対象にした`/code-review`で発見した追加バグを修正）:**

追補7の直後、初回の`/code-review`実行がメインworktreeの無関係な旧ブランチ
（`docs/adr-102-review-findings-design`）を誤って対象にしていたことが判明し、
PR番号を明示指定して再実行した。8体のfinderがPR #111
（`feat/bug25-gji-half-alnum-entry` → `develop`）を正しく検証し、特に以下の
2件は追補7の修正が新たに埋め込んでいた重大バグだった。

1. `send_ime_mode_key_with_shift_release_prefix`（`ime.rs`）が`SendInput`の
   送信数不一致時もログ警告のみで無条件に`true`を返しており、
   `Output::send_gji_half_width_alnum_toggle`の「戻り値`false`ならbeliefを
   進めてはならない（INV-D）」という契約を実質満たしていなかった。実際の
   送信数と一致した場合のみ`true`を返すよう修正。
2. 追補7のB-3修正（GJI exit送信失敗時にラッチを戻しbelief補正をスキップ
   する早期return）が、`kp_restore_kana_from_half_width`の他3系統の呼び出し元
   （`ir_notify_focus_changed`・`kp_stage_shadow_ime_toggle`×2・
   `kp_stage_post_decision`、いずれも`prepend_synthetic_shift_up=false`）の
   belief補正も無条件にスキップしてしまい、engineが`NotRomajiInput`のまま
   固着する新規バグになっていた。`prepend_synthetic_shift_up`で呼び出し元を
   区別し、`kp_shift_conv_guard_key_up`起点（ユーザーが今まさに同じアプリで
   再試行できる文脈）のときだけラッチを戻してreturnし、他3系統は送信失敗
   でもbelief補正を続行するよう修正した。

このほかsynthetic Shift↑の3重送信・到達不能分岐・match arm統合・
composing判定の重複計算・Win/Alt判定点の重複記述・未使用フィールド
（`last_half_width_entry_ms`）を修正した。詳細はPR #111のコミット
`45f532cf`を参照。

**未対応の既知の限界（Task 9の実機検証に追加、コード修正は見送り）:**
同じ`/code-review`実行でもう1件、narrow edge caseが指摘された。
`kp_restore_kana_from_half_width`は exit 時に`active_ime_kind`を
その場で再取得する（entry時点でどちらのIMEだったかを記憶していない）。
half_width_alnum_toggleがactiveな間にユーザーが言語バー等でIME製品自体を
（GJI→MS-IME、またはその逆に）切り替えた場合、exitは「切替後の」IME種別の
分岐を通ってしまい、実際にentryした側のIMEには対応する復元キーが
一切送られないままbeliefだけAssumedRomajiへ進む——追補3と同型の実害が
再発しうる。この経路（entry/exit間のIME製品切替）自体は本PR以前から
存在する設計（旧実装もexit時のactive_ime_kindをその場で読んでいた）で
あり、GJI entryが無かった頃は「skip」だけで実害が小さかったものが、
GJI entryの追加でより顕在化した形。頻度は低い（IME製品切替は
Win+Space/言語バー操作が必要で左Shift単独タップの延長では起きない）と
判断し、entry時点のIME種別をGateStoreに記憶して exit 時に照合する構造的
修正は今回見送り、Task 9の実機検証チェックリストに追加する。

**追補9（実機報告・2026-08-27）: 半角英数トグル中の左Shiftチョード
（Shift+文字での大文字入力）が、Shiftを離した瞬間にトグルを解除してしまう。
GJIで新規発見されたが、MS-IME側も含む既存（BUG-25本体、2026-07-11実装）の
設計だった。**

**症状:** `half_width_alnum_toggle = "all"`（GJI）でトグルON後、Shiftを
押したまま別のキー（例: K, A）を打鍵し、Shiftを離すと、その瞬間に
「半角英数トグルOFF」処理（`kp_restore_kana_from_half_width`）が発火して
しまう。ユーザーの期待は「もう一度左Shiftを単独タップするまでトグルが
継続すること」「Shiftを押しながらの打鍵は大文字になること」で、現状は
1回のShift+文字チョードでトグルが解除されてしまい期待と異なる。

実機ログ（Windows Terminal、GJI）で確認:

```
[engine-input] vk=0xA0 KeyDown mods(s=true)   ← LShift押下
[engine-input] vk=0x4B KeyDown mods(s=true)   ← Shift押しながらK
[engine-input] vk=0x4B KeyUp   mods(s=true)
[engine-input] vk=0x41 KeyDown mods(s=true)   ← Shift押しながらA
[engine-input] vk=0x41 KeyUp   mods(s=true)
[engine-input] vk=0xA0 KeyUp   mods(s=false)  ← LShift解放
[ime-mode] SendInput vk=0xF2 ...              ← ここで即座にexitが発火
[shift-conv-guard] （復元） → Engine activated
```

**原因:** `plan_half_width_alnum_action`（`state/half_width_alnum.rs`）は
`toggle_active == true` の場合、Shift KeyUpが「2回目の単独タップ」「右Shift
緊急解除」「左Shiftチョード（他キーを介した解放）」のいずれであるかを
区別せず、常に `Exit` を返す設計だった。これはADR-107の実装時点で意図的に
維持した既存挙動（BUG-25本体、2026-07-11のMS-IME限定実装から変更なし）
であり、GJI entryが新たに実装されて初めて実用上顕在化した——MS-IME側は
これまでこのワークフロー（トグル中にShift+文字で大文字を打つ）がユーザー
から報告されたことがなかった。

**対応:** `plan_half_width_alnum_action` の入力を `is_left_shift_tap: bool`
から `ShiftKeyUpKind { LeftTap, LeftChord, Right }` に変更し、
`toggle_active == true` のときの分岐を以下に変更した:

- `LeftTap`（2回目の左Shift単独タップ）→ `Exit`（従来どおり）
- `Right`（右Shift、タップ・チョード問わず）→ `Exit`（従来どおり、緊急解除）
- `LeftChord`（左Shiftを押しながら他キーを打った後の解放）→ `None`
  （exitしない、トグルは持続する）

`kp_shift_conv_guard_key_up` 側は、`event.vk_code` と既存の
`left_shift_tap_candidate` フラグ（左Shift押下中に他の物理キーが来ると
折れる）を組み合わせて `ShiftKeyUpKind` を判定するよう変更した。

**影響範囲:** GJI・MS-IMEの両方（`plan_half_width_alnum_action` は両方の
entry経路が共有する）。MS-IME側もこれまで同じ「チョードでexit」挙動
だったため、本修正でMS-IME側の挙動も変わる（意図的な改善、pre-existing
gapの修正）。

**テスト:** `state/half_width_alnum.rs` の
`active_toggle_second_tap_and_right_shift_exit_but_left_chord_persists`
と `unsupported_entry_blocks_enter_but_never_blocks_tap_or_right_shift_exit`
に `LeftChord` が exit しないケースを追加。

**実機確認（2026-08-27、ユーザー）:** 修正版をデプロイし再検証、「よくなりました」
と報告あり——チョード中もトグルが持続し、期待どおりの挙動になったことを確認。

---

**追補10（実機報告・2026-08-27、決定5撤回）: Composition中もShift単独タップで
半角英数entryが発火するよう緩和した。**

追補9の修正確認と同じセッションで、ユーザーから「compositionのときにシフト
単独打鍵でも半角英数になってほしいんだけどならないです」との要望を受けた。
ADR-107決定5は「Composition/候補ウィンドウ表示中はentryを発火させずラッチも
しない」という当初の保守的な設計で、根拠は「サンプル数が少なく安定性を
確認しきれていない」（追補5、2回のみの成功実績）だった。

**対応:** ユーザーの実利用フローでの複数回の成功報告を、ADRが緩和条件として
求めていた「複数回再試行での安定性確認」の充足とみなし、decision5の
composition/候補ウィンドウブロックを撤去した。

- `Output::send_gji_half_width_alnum_toggle`: entry時のcomposition_active/
  candidate_visible判定を、ブロック（`return false`）からログのみ
  （`log::debug!`）に変更。
- `plan_half_width_alnum_action`（`state/half_width_alnum.rs`）:
  `composing: bool` パラメータを削除（entry判定から completely除去）。
- `kp_shift_conv_guard_key_up`（`key_pipeline.rs`）: composing計算・
  受け渡しを削除。

**影響範囲:** GJIのみ（MS-IME entryは元々compositionを見ていない）。

**リスク:** KeyUp欠落（追補5）との相互作用によるcomposition中特有の不安定化は
理論上否定されていない。preedit破壊やモード誤認識の実機報告があれば、
`Output::send_gji_half_width_alnum_toggle`にガードを復活させること。

**テスト:** 自動テスト不可（実機のGJI/TSF composition挙動に依存）。
`state/half_width_alnum.rs`のテストから`composing`ケースを削除
（機能自体が撤去されたため）。

---

**追補11（`/code-review` PR #112指摘・2026-08-27）: 追補9の左Shiftチョード
修正が右Shiftには適用されておらず、右Shiftチョードで同じ不具合が再現する
非対称を修正。**

PR #112（追補9・追補10の実装）マージ直後に`/code-review pr #112`を実行した
ところ、`ShiftKeyUpKind::Right`が単独タップ・チョードを区別せず常に`Exit`を
返す設計のままだったという指摘を受けた。右Shiftを押しながら別のキーを
打鍵（大文字入力）してShiftを離すと、追補9で左Shiftについて修正したのと
全く同じ症状（トグルが意図せず解除される）が右Shiftでも再現する。

**原因:** 左Shiftには`GateStore::left_shift_tap_candidate`という「押下中に
他の物理キーが来なかったか」を追跡するフラグがあったが、右Shift用の
対応するフラグが存在しなかった。そのため`ShiftKeyUpKind`は右Shiftを
一律`Right`（常にExit）としか表現できていなかった。

**対応:** `GateStore`に`right_shift_tap_candidate`を新設し、
`kp_shift_conv_guard_key_down`/`kp_stage_shift_conv_guard`の候補判定・
候補破棄ロジックを左右対称に拡張した。`ShiftKeyUpKind`を
`{LeftTap, LeftChord, RightTap, RightChord}`の4値に変更し、
`plan_half_width_alnum_action`の`toggle_active`時の分岐を
「`LeftChord`/`RightChord`はexitしない、それ以外（`LeftTap`/`RightTap`）は
exitする」に統一した。右Shift**単独タップ**（緊急解除の意図）は従来どおり
exitする——チョードのみが持続対象になる。

合わせて、レビューが指摘した`mem::take`直後の冗長な`= false`代入
（`left_shift_tap_candidate`）も削除した。

**テスト:** `state/half_width_alnum.rs`のテストを4値対応に更新
（`active_toggle_taps_exit_but_chords_persist_symmetrically`等）。

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

**追補（横展開、2026-08-19）:** `kp_stage_idle_conv_check` 以外にも、同じ
`SendMessageTimeoutW(SMTO_ABORTIFHUNG)` ベースの同期呼び出しがエンジンスレッド上に
5箇所残っていた（低確率で全キー swallow される謎バグの調査から発見）。Opus に
architect/reviewer 役を分けて2巡の premortem（「もう出荷され実機で壊れた」前提で
遡って原因を語らせる手法）にかけたところ、1巡目の設計（fence を単純に横展開する案）が
**新しいサイレント機能停止**を生むと判明した——`Output::send_keys` が冒頭・末尾で
呼ぶ `mark_send()` は NICOLA の通常の文字出力（conv を一切変えない）でも発火するため、
`last_send`/`output_in_flight_ms()` ベースの fence は (a) 文字出力のたびに誤って落ち、
かつ (b) 本来検出すべき `send_vk_dbe_hiragana_pair`（`mark_send` を通らない）を
一度も捕捉できていなかった。

対処（実装済み、`fix/bug34-sync-ime-calls` ブランチ）:

1. **`conv_mutation_seq`**（新規、`conv_mutation.rs`）: conv ワードを変えうる VK
   （`VK_KANA`/`VK_CONVERT`/`VK_DBE_*`、`vk::vk_may_mutate_conv` が判定）を送信した
   ときだけ増分する専用カウンタ。列挙は `win32::send_input_safe`（全 `SendInput` の
   唯一のチョークポイント）に一本化——名前付きラッパー単位の列挙では
   `ime.rs::send_ime_mode_key`（ユーザー設定 VK を送るため、同じ関数が open-only にも
   conv-mutating にもなる）を正しく分類できないため、実行時に VK 値そのもので
   判定する設計にした。`key_pipeline.rs::apply_idle_conv_check` の fence(c) を
   これに置き換え、旧 fence が誤って落ちる/検出漏れする二重の欠陥を修正。
2. **`SendHealth`**（新規、`send_health.rs`）: `imm.rs::send_ime_control` の実測msを
   記録し、直近 slow だった場合は同期サイトの発行を見送るサーキットブレーカ。
   `ime_refresh.rs`（旧 site A）・`ime_controller.rs::romaji_pre_write`（旧 site E）・
   `key_pipeline.rs::apply_focus_probe`（旧 site C）に適用。**初回の ~5s ブロックは
   防げない**（再発だけ止める）——真の解消には各サイトの offload 化が要る。
3. **`with_app_or_repost_with`**（既存関数を配線）: `WM_ASYNC_IME_APPLY_COMPLETE` の
   受け口が `let _ = with_app(...)` で再入時に完了を黙って捨てていた穴を修正。
4. **旧 site B**（`executor.rs::dispatch_ime_set_open` の sync path、MS-IME
   TsfNative の conv 読み取り）: 読んだ値は `apply_ime_open_with_view` が
   `log::debug!` に渡すだけで実 actuation には配線されていない（`ImeController::apply`
   は belief 引数を取らない）と判明したため、単純に read を削除し `conv_mode: None`
   を渡すだけに縮小。fence も degrade 方針も不要。
5. **旧 site D**（`runtime/mod.rs::try_force_on_bootstrap`）: 同期
   `ImmCrossProcessStrategy::apply` chain から `run_open_chain_async`（`executor.rs`
   の ImmCross async path と同じ経路）へ移行。前提として `ImeModel.pending`
   の期限切れパージ（`ImeTransition.timeout_at` は元々存在したが呼び出し元が
   ゼロだった）と、`UnsafeToToggle` 完了時に pending を解放する修正
   （旧経路は早期 return で `record_ime_apply_result` に到達せず pending が
   永久残留し、以後の別 generation の完了が全部 stale 判定される固着を生みうた）を
   先に入れた。`WM_ASYNC_IME_APPLY_COMPLETE` の wparam に reason bit を追加し
   （`OpenApplyReason::Bootstrap` vs `EngineDecision`）、async 完了後も
   provenance が失われないようにした。
6. **旧 site E**（`ime_controller.rs::romaji_pre_write`）: `open_chain.rs::fallback_write`
   が `with_app` の `RUNTIME` borrow を握ったまま同期ブロックする実装は変えず
   （hwnd 解決統一を伴う完全な非同期化は実機ソーク前提で見送り）、SendHealth の
   gate のみ追加した。この borrow 保持中は、他の完了メッセージが上記3の修正で
   再入時に再送されるようになったため「ブロック中に他の完了が永久に失われる」
   という最悪の帰結は防げている。

**見送った項目（実機ソーク必須のため）**: site A（`ime_refresh.rs` の conv prefetch
を async 化する設計、`ConvModeMgr` の last-writer-wins 競合が先に塞がっていないと
悪化する）と `ConvModeMgr` → `ConvObservation` への格上げ（B-2）。

**追補2（Opus レビューで発見・同日中に修正、2026-08-19）:** 実装完了後に Opus に
実コード（ツールアクセス付き）でレビューさせたところ、上記の実装自体に新しい
欠陥が複数見つかった。いずれも「BUG-34 横展開1巡目 premortem が指摘した欠陥と
同型のものが、直した箇所とは別の場所に再発していた」というパターンで、
修正済み:

1. **`conv_mutation_seq` が IMC 経由の conv 書き込みを1つも数えていなかった**
   （最重要）: ゲートを `win32::send_input_safe`（SendInput 経由）にしか置いて
   おらず、`set_ime_romaji_mode_for_hwnd` が使う `imm.rs::send_ime_control` の
   `IMC_SETCONVERSIONMODE` 経路を見ていなかった。旧 `last_send` fence が
   「本来検出すべき自己出力を1つも捕捉できていなかった」のとまったく同型の
   欠陥が、直したはずの箇所の**すぐ隣**に残っていた。`send_ime_control` にも
   `IMC_SETCONVERSIONMODE` 時の bump を追加（2箇所ゲート体制に）。
2. **B（executor.rs）の診断用 offload が毎打鍵で OS スレッドを spawn していた**:
   削除した同期 read の代わりに追加した「log 専用の fire-and-forget offload」に
   in-flight ガードが無く、この経路が「毎打鍵で走る」ことと衝突し、GJI ハング中に
   打鍵ごとに ~5s ブロックするワーカースレッドが積み上がる（かつその読み取り
   結果が `send_health::record` に給餌されグローバルブレーカを誤作動させうる）
   実装になっていた。唯一の消費先がログだけである以上、診断ごと削除した。
3. **`idle_conv_check_in_flight` が `with_app` の1回の再入失敗で永久にラッチする**:
   Step0-b で `WM_ASYNC_IME_APPLY_COMPLETE` に対して直したのと同型の欠陥。
   ただし被害はこちらの方が重い（完了1件の消失ではなく、idle-conv-check が
   プロセスの寿命いっぱい発火しなくなる）。`bool` を `Option<u64>`（spawn 時刻）
   に変更し、`IDLE_CONV_CHECK_IN_FLIGHT_STALE_MS`（8000ms）を超えたら
   「放棄された」とみなして自己回復するようにした。
4. **`ImeModel.pending` の 1 秒タイムアウトが、この横展開が対象にしている
   最悪ケース（`HungAppTimeout` ≒ 5000ms、BUG-34 実測 5741ms）より短かった**:
   D-prep で有効化したパージが、正当な in-flight apply（offload 先が実際に
   ハング境界までブロックしている場合）を「放棄された」と誤判定し、後から
   届く完了を stale として黙って捨てる新しい失敗モードを生んでいた。
   `IME_APPLY_PENDING_TIMEOUT_MS`（tuning.rs、8000ms）に延長。
5. **A（`ime_refresh.rs`）のブレーカ degrade が eisu guard の存在理由を踏み外して
   いた**: 見送り時に一律 `None` へ degrade すると `eisu_guard_active=false` と
   なり fail-open で warmup が送られ、ユーザーが tray で明示的に半角英数にして
   いた場合でもひらがなへ戻してしまう。`ConvModeMgr` の直近キャッシュ値へ
   degrade するよう変更（「不明＝英数ではない」という一番弱い仮定を避ける）。
6. **E（`romaji_pre_write`）のブレーカ gate に再試行経路が無かった**: この関数は
   フリー関数で `Runtime` にアクセスできず、スキップ時に再試行をスケジュール
   する手段がなかった。gate だけ入れると「ブロックする」を「ROMAN ビットが
   次の明示トグルまで静かに補完されないまま固着する」というログにも残らない
   不具合に置き換えるだけだったため、gate を撤去し元の常時試行に戻した
   （再試行機構込みの設計は E 本体に持ち越し）。
7. **`SendHealth` が単発スパイク1回でブレーカを作動させていた**:
   `consecutive_slow` を記録はするが判断に使っておらず、GC 停止等の一時的な
   遅延1回で A/C/E の3サイトが2秒間まとめて degrade しうた。2回連続の slow
   判定で初めて作動するよう変更（`TRIP_AFTER_CONSECUTIVE_SLOW=2`）。
8. `win32::input_may_mutate_conv`（INPUT 構造体からの VK 抽出・Unicode モード
   除外ロジック）にテストが1件も無かったため4件追加。

指摘のうち残した/見送ったもの: `try_force_on_bootstrap` が in-flight の
`pending` を上書きしうる新経路（新設の警告ログで検知可能、頻度は低い）、
`with_app_or_repost_with` に再試行上限が無い（既存のプリミティブへの変更で
影響範囲が広いため見送り、残存リスクとして記録）。

**検証状況:** `cargo test -p awase-windows --lib`（Linux、pure logic 全418件）、
`architecture_guard.rs`/`layer_boundary_guard.rs`/`golden_scenarios.rs`/
`journal_replay.rs`/`drift_correction_replay.rs`/`intent_store_effective_open.rs`
（Linux 実行可能な全 tests/ 全緑）、`cargo xwin check`/`cargo xwin build --tests`/
`cargo xwin clippy -p awase-windows --lib`（Windows ターゲットのコンパイル・
テストビルド・lint、クリーン。`--all-targets` はこの変更と無関係な既存箇所で
多数指摘が出るが、CI の clippy ジョブのスコープは `--lib` のみ）で確認済み。
**Windows 実機での実行検証は未実施**（このセッションには Windows 実行環境が無く、
wine 等でのクロス実行もできなかったため、windows-gated なモジュール
（`send_health.rs`/`platform_state.rs`/`win32.rs` 等）のテストは型検証・
テストビルドのみで実行は未確認。CI の windows-latest ジョブでは実行される）。

**関連（追補分）:** `crates/awase-windows/src/conv_mutation.rs`,
`crates/awase-windows/src/send_health.rs`,
`crates/awase-windows/src/runtime/open_chain.rs`（`fallback_write`）,
`crates/awase-windows/src/state/transition.rs`（`ImeTransition::is_timed_out`）,
`crates/awase-windows/src/runtime/mod.rs`（`try_force_on_bootstrap`）,
`docs/adr/087-open-belief-actuation-warrant-separation.md` §5 item14（実
actuation 入口棚卸し表の #7 訂正）。

**追補3（タスクトレイ不具合報告の切り分け強化、2026-08-20）:** 今回の横展開で
残した A/C/E のブレーカ degrade は、発生してもユーザーには「何も起きなかった」
としか見えず、発生有無を事後に確認する手段がなかった。ADR-095 の不具合報告
機能（内部状態スナップショット・journal 添付）を拡張し、この不具合の再発を
実際のユーザー報告から切り分けられるようにした:

1. `BugReportStateSnapshot` に `send_health_last_elapsed_ms` /
   `send_health_consecutive_slow` / `send_health_breaker_tripped` /
   `idle_conv_check_in_flight_ms` を追加（報告クリック時点のスナップショット）。
2. A/C の degrade 発生ログ（`ime_refresh.rs`/`key_pipeline.rs`）を
   `log::debug!` から `log::warn!` へ格上げ——本番既定の `info` レベルログ
   （`awase.log`）に残らなければ、報告に添付しても意味が無いため。
3. `awase.log`（実際の `log::` 出力）の末尾を journal とは別系統で報告に
   添付できるようにした（`BugReportPayload.app_log_excerpt`、既存の
   `attach_log` チェックボックスで journal と一緒に制御）。journal は構造化
   イベントであり `log::warn!`/`log::debug!` の生テキストを含まないため、
   これが無いと `[send-health]`/`[idle-conv-check]` の警告が報告から読み取れ
   なかった。
4. `services/report-worker` 側は `app_log_excerpt` を **省略可能**として
   受理する（無ければ `null` 扱い）——この変更前のクライアントが送る報告を
   拒否しないための後方互換設計。

**検証状況（追補3）:** `cargo test -p awase-windows --lib`（421件）/
`architecture_guard`（34件）/ `cargo xwin check`・`build --tests`・
`clippy --lib --bins`（awase-windows・awase-settings）で確認済み。
`services/report-worker` は `pnpm test`（29件）・`pnpm typecheck` で確認済み
だが **`wrangler deploy` によるデプロイは未実施**（サーバ側の変更が本番に
反映されるまでは、クライアントが送る `app_log_excerpt` は現行の本番
report-worker では単に無視される——`validatePayload` が既知フィールドのみを
picking する実装のため、エラーにはならないが保存もされない）。

**追補4（Site A: `ir_post_focus_change_snapshot` の eisu ガード撤去、2026-08-20）:**
横展開の最後の1箇所（A）に残っていた同期 `SendMessageTimeoutW`（
`get_ime_conversion_mode_raw_timeout(10)`、`send_health` ブレーカでのみ
ガード）を、他の箇所と同じく非同期化できないか3ラウンドの Opus premortem
レビューで検討したが、いずれも実コードで確認済みの致命的な欠陥に行き着き、
**「eisu ガード機能そのものを撤去する」判断に至った**:

1. **設計1（warmup 自体を非同期尾部に）**: `send_eager_warmup` を非同期化すると
   `eager_warmup_sent_ms` が同期的に立たなくなり、`WARMUP_GRACE_MS`/GJI
   settle-grace が同時に無効化される。フォーカス変更直後の最初のキーで
   spurious `apply_ime_open(false)` が無抑制で発火する——この repo が
   繰り返し戦ってきた障害ファミリー（[[project_edge_fake_focus_probe_fix]]、
   ADR-079 epoch fencing と同型）の再燃。
2. **設計2（ConvModeMgr を `focus_epoch` でキー化してキャッシュ）**:
   `focus_epoch` は Site A 到達より厳密に先行して増分される（`ir_stage_focus`
   → `ir_stage_observe`）ため、Site A で `get_for_epoch(現在epoch)` は
   **確率1で `None` を返す**。キャッシュへの書き手（idle-conv-check/Site C）は
   いずれも打鍵駆動で Site A より後にしか発火しないため、「1つ前の epoch の
   観測」が Site A の実行時点では原理的に存在しない。ガードが恒久的に不発になる。
3. **設計3（ConvModeMgr を pid+timestamp でキー化、Site A は受動的 lookup のみ）**:
   epoch キーの構造的欠陥は解消したが、別の3つの欠陥が独立に発生した:
   - Site A 自身の `update_from_conv` 書き込みを撤去すると、直後の初回
     idle-conv-check が「前のアプリの conv」と比較することになり
     `conv_mode_changed` が反転、`EngineSync::ReportOpenInference(
     NativeToggleShadowOff)` が誤発火する。これは
     [docs/experiments.md](experiments.md) エントリ03 に実機記録済みの
     「直接入力中の spurious Engine ON」の再燃条件そのもの。
   - キャッシュ更新契機（idle-conv-check/Site C）はいずれも打鍵駆動のため、
     ガードが本来検出すべき「トレイでの無操作な conv 切替」を観測できず、
     陳腐化したキャッシュが warmup を誤スキップさせる新規の cold-start
     literal 化（BUG-02/BUG-45 系）を生む。
   - `ConvModeMgr` は単一スロットのキャッシュのため、TsfNative アプリ同士を
     行き来しただけでも上書きされ、ガードが実効するのは事実上 WezTerm/
     Windows Terminal 程度まで縮小する。

   3設計とも共通して、「Site A が知りたいのは『前回このアプリにいたときの
   conv』であり、その答えは定義上 Site A の実行時点より過去にしか存在せず、
   かつユーザーの tray 操作はその答えを更新する打鍵を伴わない」という
   構造的な壁に突き当たった。

**結論・現在の実装:** `ir_post_focus_change_snapshot` の eisu ガード（
`is_eisu_now`/`eisu_guard_active` の算出と、それに伴う同期 Win32 呼び出し・
`send_health` ブレーカ分岐）を完全に削除した。`send_eager_warmup` は
`applied_open` のみをガードとして無条件に呼ぶ。

**既知の制限（新規、意図的に受容）:** ユーザーが tray から明示的に半角英数へ
切り替えた直後にそのウィンドウへフォーカス復帰すると、この eager warmup が
`VK_DBE_HIRAGANA` を送信し、一度だけひらがなモードへ戻る。以前は Site A の
同期読み取りがこれを検出しスキップしていたが、BUG-34 対応のため撤去した。
再現条件: 任意アプリ + awase engine ON の状態で、IME ツールバー/タスクトレイ
から半角英数（またはカタカナ等）へ切り替えた直後に Alt+Tab 等でフォーカスが
外れ、再度そのウィンドウへフォーカスが戻ったタイミング。

**Site A における BUG-34 の解消状況:** この撤去により、Site A は
`SendMessageTimeoutW` を一切呼ばなくなった——ブレーカによる「上限付け」では
なく、当該箇所の BUG-34 リスクは完全に除去された。横展開 A〜E のうち E
（`romaji_pre_write`、task #108）のみ実機ソーク待ちで残存。

**検証状況（追補4）:** `cargo xwin check`/`build --tests`/`clippy --lib`
（awase-windows）で確認済み。この箇所を直接演習する Windows-gated 以外の
単体テストは元々存在しない（`ir_post_focus_change_snapshot` は Runtime 全体の
統合的な状態を要するため）。Windows 実機での検証は未実施。

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

### 追補2（2026-08-25、当時の再現手順は現行コードでは通らないことを確認。原因の構造自体は未検証のまま残存）

上記「再現手順」の入口（物理 `0xF0`(VK_DBE_ALPHANUMERIC) up / `0xF2`(VK_DBE_HIRAGANA)
down が生のまま GJI に届き、awase の shadow model 判定と二重に効く）は、報告日
（2026-07-29）より後に入った2件の修正で塞がれていることをコード確認した:

- **BUG-46**（`076b8709`、2026-08-01）: TsfNative（Windows Terminal 等）+GJI でも
  `GjiDirectStrategy` が actuation を担うようになり、`ime_actuation_owned` 判定が
  TsfNative にも及ぶようになった。
- **BUG-52**（`bdf4a139`/`9a02ce6b`、2026-08-05）: `VK_DBE_*`（0xF0 含む）の
  KeyDown を `shadow_toggled` に関係なく無条件 Suppress するよう拡大。

現行の `PhysicalKeyDisposition::plan`（`runtime/transport.rs`）では、TsfNative+GJI
で物理 `0xF0`/`0xF2` の KeyDown は OS へ渡らず Suppress され、代わりに awase
自身が `SendInput(VK_IME_OFF/ON)` で actuate する。BUG-45 のログにある「物理
F0 up / F2 down が生のまま GJI に届く」という入口は、**今は文字通りには再現
できない**（2026-08-25、ユーザー指摘により確認）。

**ただし、これは BUG-45 の根本原因が解消されたことを意味しない。** 本バグが
指摘する構造的な穴——`ColdReason::SetOpenTrue` はどの経路で
`ImeEffect::SetOpen(true)` が適用されても発火し、`FreshF2` 待機を常にスキップ
して `per-VK confirm` へ入る（`DIAG_DISABLE_PROACTIVE_TSF_WARMUP` は現在も
常時 true、`tsf/warmup/gji_warmup_coro.rs`）ため実 TSF composition 状態を
確認する手段がないまま推測で backspace する——は、トリガー元が
awase 自身の `SendInput(VK_IME_ON)` に変わっても構造上そのまま残っている。
`per_vk_recovery_params` の固定 `backs` 値・`RawTsfLiteralRecovery` の
未確定 preedit commit リスク（BUG-36）もコード上変更なし。

**未対応（更新）:** 上記「未対応（残存）」の内容自体は今も有効。追加すべき
検証は、現行コードでの新しい再現手順の確立（例: 既定のホットキー
`Ctrl+変換` で TsfNative+GJI の IME を長時間 idle 後に ON にし、直後に
入力する）。旧ログの入口が塞がれたことを「解決」と誤認して未解決4件
（BUG-25/45/60/75）から外さないこと。 `PhysicalKeyDisposition::plan` が TsfNative の物理 KANJI 系キーを無条件 Allow するため、GJI/MS-IME 環境で awase 自身の apply-ime actuation と二重に actuate する

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

## BUG-50: 一度カタカナに入ると IME-ON コンボを押しても永久に復旧できない（デッドロック解消・トリガーとも解消済み、詳細は追補参照）

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

**追補（2026-08-17、コミット時系列からの再検討・ユーザー指摘）:** 原因2を
未確定のまま残していたが、`git log` のタイムスタンプを確認したところ、
本 BUG の Phase 1 修正（`21a6b6b6`、2026-08-05 04:12）の**わずか1.5時間後**に
BUG-52 の修正（`bdf4a139`、2026-08-05 05:49、「NICOLA の物理『IME ON』キー
(scan 0x70) を IME が既に ON の状態で押すと、Windows のキーボードレイアウト
変換層が `VK_DBE_HIRAGANA` の代わりに `VK_DBE_KATAKANA` を生成することがあり、
当時の suppress ロジックの穴〈`shadow_toggled` 条件でしかガードしていなかった〉
を素通りして実際に MS-IME をカタカナへ切り替えていた」）が同日中に入っている
ことが判明した。症状の記述（「謎にカタカナになる」）も酷似しており、本 BUG の
2026-08-05 実機報告は、当時まだ存在していた BUG-52 の穴が原因だった可能性が
高い（仮説A〜Cのいずれよりも状況証拠が強い、原因2の未検討だった第4の候補）。
BUG-52 の修正で最有力の発生原因が塞がれ、翌日の `7fcb89aa`（2026-08-06
18:12、`MsImeDirectStrategy` の IME-ON キーを `VK_DBE_HIRAGANA` → `VK_IME_ON`
に変更、ガード3の根治）で構造的デッドロック（原因1）も別途根治済みのため、
Phase 1 のカタカナ観測ベースの復旧（`observed_katakana` 条件）は、この2つが
塞がる**前**に書かれた対症療法だったと判断し、charset 軸の追跡撤去（ADR-094）
に合わせて撤去した。原因2の仮説A〜Cは検証されないまま未決着だが、実害の
発生源として最有力だったものは解消済みと考えてよい。

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

**追補2（2026-08-22、GJI eager warmup 側の残り火を解消、ADR-100 決定2）:**
本追補（2026-08-06）は `MsImeDirectStrategy` の ON キーを `VK_DBE_HIRAGANA` から
`VK_IME_ON` へ移行して BUG-50 のデッドロック前提を潰したが、`GjiDirectStrategy`
（IME 開閉制御）とは別に、`Output::send_eager_tsf_warmup`（TSF composition の
cold-start 事前ウォームアップ、`output/mod.rs`）が独立に `VK_DBE_HIRAGANA`
（`tsf/send.rs::send_vk_dbe_hiragana_pair`、旧名）を送り続けており、この経路には
BUG-50 型の副作用（「開く」と「ひらがなに強制する」の束ね）が残っていた
（[ADR-098](../adr/098-tsfnative-applied-confirmed-laundering-and-force-on-removal.md)
F4 が「受容中の既知リスク」として明示的に残していたもの）。

[ADR-100](../adr/100-gji-warmup-vk-ime-on-reinit.md) 決定2 の実機検証（群B、F17・F18）
で `VK_IME_ON`（scan 実値 + `TSF_MARKER`、既存の送信形態のまま VK のみ差し替え）が
cold-start 対策として機能することを確認したうえで、`send_vk_dbe_hiragana_pair` を
`send_eager_warmup_vk_pair` に改名し、送信 VK を `VK_IME_ON` へ変更した
（`crates/awase-windows/src/tsf/send.rs`・`output/mod.rs`）。これにより eager
warmup 経路からも BUG-50 型の副作用が構造的に消える。

**テスト**: `cargo clippy -p awase-windows --lib -- -D warnings`・
`cargo test -p awase-windows --lib`（697件）・`architecture_guard`（38件）・
`golden_scenarios`（22件）・`ime_key_sequence_golden`（2件）、いずれも
dragonflyg4 実機ビルドで green（Windows 実機、クロスコンパイルではない）。
**この変更は `ime_key_sequence_golden.rs` が固定する `characterize_strategy`
の経路を通らないため、既存 golden では守られない**（ADR-100 決定2 が
指摘済みの限界。回帰させないためには本追補が (b) の記録として機能する）。
実機での cold-start 検証は ADR-100 F18（15.6秒・30.3秒放置、計13回の
cold-start、`giving up`/literal 化0件）を参照。群A（旧 `VK_DBE_HIRAGANA`）
との同一セッション内の直接比較は未実施のまま採用しており、比較データが
将来必要になった場合は ADR-100 決定2 の残タスクを参照すること。

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

### 追補（2026-08-11 実機再発・root cause 訂正・IntentStore 配線）

**症状:** MS-IME（TsfNative、Windows Terminal `Windows.UI.Input.InputSite.WindowClass`、
`app_kind=Uwp`）で、~3.5時間のスリープ/ロックから復帰した直後に日本語入力が
正常に動作していた状態から、Ctrl+無変換（既定の IME OFF コンボ）を押した。
ユーザーの直接確認（実機）: **「IME OFF 直接英数入力で、Engine ON でした」**
——実 IME は正しく OFF になり直接英数入力が成功していたが、awase 自身の
Engine（NicolaFsm の親指シフト/ローマ字処理）は ON のままだった。ログには
`desired_open=false` 確定後、`[idle-conv-check] TsfNative: conv=0x00000009`
（`NativeToggleShadowOff`）が `ObserverReported` として記録され続け、drift
correction が `VK_1A` を GiveUp→再試行のサイクルで送り続ける様子が残っていた。

**IME:** Microsoft IME。TsfNative プロファイル（Windows Terminal / InputSite）。

**当初の誤診断（撤回済み）:** 最初に立てた仮説は「`most_recent_trusted()` に
`ConvOpenInference`（BUG-55 で診断済みの `ime_wnd` 解離により実態と無関係な
`conv=0x9` から推論される、`ObservationAuthority::BeliefOnly` 観測）が紛れ込み、
これが `effective_open()`（`Engine::compute_state` が直接参照する `ctx.ime_on` の
根拠）を ON に反転させている」というものだった。この仮説を Opus に独立レビューさせた
結果、方向性は正しいが機構の特定が誤りだったと判明した:

- 実際のフリップ経路は `most_recent_trusted()` ではなく
  `derive_open_filtered()` の Medium 単独合意（TsfNative では他に競合する
  open 観測が無いため、`ConvOpenInference` 1 件だけで `Some(true)` が確定する）
  ——これは BUG-63 で既に確定していた機構と同型。`most_recent_trusted()` は
  `derive_open_filtered()` が失効した後（`FRESH=3s` 超過後）にこの反転を
  無期限に固着させる二次要因ではあるが、最初にフリップさせる犯人ではない。
- トリガーは「同一アプリ内 hwnd のバタつき」では起こらない。
  `ImeEvent::FocusChanged` の本番 dispatch 箇所は
  `runtime/focus_tracking.rs::on_focus_process_changed` の1箇所のみで、
  **PID が変わった場合にしか発火しない**。Windows Terminal の
  `CASCADIA_HOSTING_WINDOW_CLASS` と `Windows.UI.Input.InputSite.WindowClass`
  は同一プロセスなので、hwnd がどれだけバタついても `last_intent` はクリア
  されない。本当に必要なのは**プロセスを跨ぐ実フォーカス変更**（他アプリへの
  一瞬のフォーカス奪取と復帰、例: BUG-57 の Pushbullet 通知、または当日の
  スリープ復帰直後のフォーカス再構築）であり、これは 2026-08-11 のログ抜粋
  範囲では直接確認できていない（未確定のまま）。
- また、ログの時系列（OFF直後 ~1.5秒間 `idle-conv-check` が連発しているように
  見える記述）は `should_run_idle_conv_check`（`EXPLICIT_IME_SUPPRESS_MS=1500ms`
  の間 idle-conv-check 自体を抑止するガード）とそのままでは整合しない点も
  指摘された。

**検討したが撤回した修正案:** `ObservationSource::authority()` が既に
`ConvOpenInference` を `ObservationAuthority::BeliefOnly`（`Actuating` ではない）
と分類していることを利用し、TsfNative（`FeedbackPolicy::Blind`）プロファイル
限定で `derive_open_filtered()`/`most_recent_trusted()` の両方を
`authority()==Actuating` のみに絞る案を検討したが、**実装前に撤回した**。
理由: `docs/adr/087-open-belief-actuation-warrant-separation.md` §7 が
「`ConvOpenInference` の confidence を下げる/絞り込む」方向を BUG-26 再発の
理由で明文で再提案禁止としており、MS-IME×TsfNative では open 観測が
`ConvOpenInference` 以外に構造的にゼロ（`Gji`/`Tsf` ソースは本番コードに
記録サイトが無く、`Blacklist` 経路は IMM クエリ自体をスキップする）ため、
これを弾くと Engine が二度と ON に復帰できなくなる（BUG-26 そのもの）。
さらに `most_recent_trusted()` は drift correction（`check_drift_correction`）
自身の収束判定にも使われており、シグネチャを変えると「Engine ON は消えるが
実 IME も OFF に戻らない」というより悪い状態を作りかねない。

**修正（本追補、IntentStore 配線）:** ADR-087 §5 Phase 1' で設計され
`state/intent_store.rs` として純粋ロジックのみ実装済み（`HwndId` 単位、
ON/OFF 非対称 TTL）だった `IntentStore` を、`PlatformState::effective_open()`
（`state/platform_state.rs`）から実際に読み取るよう配線した
（ADR §5 Phase 1' item8 が要求していた統合、これまで未配線だった）。

- `ImeStateHub::effective_open()` は、`shadow_model.current_focus()` が
  `Some(hwnd)` かつ `intent_store.lookup(hwnd, now)` が有効なエントリを返す間は
  その `open` 値をそのまま採用し、`ImeModel::effective_open()`（`last_intent`/
  `derive_open_filtered`/`most_recent_trusted` の通常経路）にフォールバック
  しない。`IntentStore` は `FocusChanged` でクリアされないため、**同一対象
  （同一 `HwndId`）へフォーカスが戻った場合に限り**、直前の明示 OFF/ON 意図が
  壊れた `ConvOpenInference` 観測に負けなくなる。
- `apply_panic_reset`/`apply_hwnd_cache_restore`（`ImeEvent::PanicReset`/
  `HwndCacheRestored` を dispatch する唯一の designated 関数）は、
  現在の対象の `IntentStore` エントリを無効化してから `desired_open` を
  書き換える。これにより、これらの安全弁がこの新しい優先ロジックによって
  古い明示意図に負けることはない。
- `ImeModel::effective_open()`/`resolve_open_at()` 自体（`Instant` ベースの
  純粋ロジック）は一切変更していない——変更は `ImeStateHub` 層の薄い上書きに
  限定される。

**効果範囲（誠実に書く）:** ADR-087 §5 Phase 1' 自身が明記するとおり、この
配線が効くのは**直前に明示 IME 操作があった場合に限る**。フォーカス変更前に
一度も IME キーを押していない場合（BUG-63 の再現手順そのもの）や、対象
（`HwndId`）が実際に変わった場合は、従来どおり観測ベースの
`derive_open_filtered()`/`most_recent_trusted()` にフォールバックする
（=このバグの再現手順のように「直前に Ctrl+無変換 を押していた」ケースには
効くが、万能の修正ではない）。また Opus レビューで指摘された別経路
（`conv_classify::EngineSync::SetOpen(RomajiRecovered)` が conv 由来の再同期を
`UserImeSetIntent{Command}` として偽装し `last_intent`/`desired_open` を
両方 ON に書き換えてしまう抜け道）はこの追補では未対応——別バグとして
今後切り出す。

**検証:** `cargo test -p awase-windows --lib`（Linux native、342 件 pass、
`state::platform_state` は `#[cfg(windows)]` のためこの中に含まれない）、
`cargo test -p awase-windows --test architecture_guard`（22 件 pass、
`panic_reset_event_is_limited_to_apply_panic_reset`/
`hwnd_cache_restored_event_is_limited_to_apply_hwnd_cache_restore` を含めて
全緑）、`cargo xwin check --tests -p awase-windows --target
x86_64-pc-windows-msvc`（警告ゼロ）、`cargo xwin clippy -p awase-windows
--target x86_64-pc-windows-msvc -- -D warnings`（`--tests` 抜き、警告ゼロ、
`--tests` 込みは本追補と無関係な既存 pedantic 指摘が残っており従来通り除外）。
`state/platform_state.rs` に本追補用の回帰テスト6件
（`effective_open_survives_focus_change_via_intent_store` ほか）を追加、
Windows 専用のため型検査のみで実行はできていない。**wine 未導入のため
このサンドボックスでは実機相当のテスト実行が一切できず、本追補は
コードレビューと静的検証のみに基づく——次回実機で Ctrl+無変換 →
（プロセスを跨ぐ）フォーカス変更 → 再フォーカスという手順を踏み、
Engine が ON に戻らないことを確認すること。**

**関連:** BUG-26（`ConvOpenInference` を安易に弾くと再発する Engine 復帰不能）、
BUG-55（`ime_wnd` が InputSite の実態と無関係な固定ハンドルを返す）、
BUG-57（Pushbullet 通知によるプロセス跨ぎのフォーカス奪取）、
BUG-63（`derive_open_filtered` の Medium 単独合意で belief が反転する同型の
機構）、[ADR-087](adr/087-open-belief-actuation-warrant-separation.md)
（§5 Phase 1'、`state/intent_store.rs`）。

### 追補2（v3: 3ラウンドの Opus pre-mortem を経た IntentStore 配線の訂正）

上記追補（v1、`21ca84d1`）の実装は、コミット後の独立レビュー（Opus によるアドバ
ーサリアル pre-mortem、以下同一エージェントで3ラウンド）で **as-written では
出荷不可** と判定され、設計を2回訂正（v2→v3）した上で3ラウンド目で明示的な
GO（ブロッキング指摘ゼロ）を得た。実装は v3 設計に基づく。

**pre-mortem #1（v1 に対する必須指摘3件）:**

1. conv 由来の内部同期（`EngineSync::SetOpen(RomajiRecovered)`/`DirectInput`）が
   `UserImeSetIntent{Command}` を経由するため、v1 の `dispatch_event` が
   これも無差別に IntentStore へ record してしまい、壊れた conv 読み1件が
   `FocusChanged` を生き延びる**偽の明示 OFF 意図**になる（v1 適用前より悪化。
   BUG-19/BUG-48 が既に潰した「観測を意図に偽装する」パターンの再燃）。
2. `apply_hwnd_cache_restore` の cache-miss 側と `reset_stale_ime_on_for_imm_broken`
   （BUG-16 系 safety-net）が IntentStore を無効化しないため、この安全デフォルト
   がstale な IntentStore エントリに黙って握り潰される。
3. `EXPLICIT_OFF_INTENT_TTL_MS` が `HWND_CACHE_MAX_AGE_MS`（1時間、未実測
   プレースホルダ）を転用しており、IntentStore が実際に読まれるようになった
   時点で、上記すべての失敗モードの最悪持続時間になっていた。

**v2 設計 → pre-mortem #2（v2 も no-go、外科的訂正が必要と判定）:**

v2 は「`RomajiRecovered`/`DirectInput` 両方を `handle_engine_activation_sync` へ
リダイレクト」「hit/miss 両方で無条件 `remove()`」「TTL 30秒」を提案したが、
以下の理由で却下された。

- `DirectInput` のリダイレクトは危険: その `desired_open=false` 書き込みは
  `is_eligible_for_ime_force_on()` が gate する3つの force-ON 経路
  （**うち1つは ADR-086 `conv_mode_policy=force` の実機ソーク中の本番経路**）・
  `last_user_explicit_off_ms`・`from_explicit_off_intent` の load-bearing な
  入力であり、外すと BUG-54 型（20ms 無限再送）の退行を再現しかねない。
  `RomajiRecovered` のみのリダイレクトなら安全（発火条件が `effective_open==true`
  を要求するため、書き込みは `desired_open := effective_open` という循環 echo
  ——`ime_model.rs` の `EngineActivationSync` arm が明文で禁じるパターン——の
  残存インスタンスに過ぎない）。
- hit 側の無条件 `remove()` は逆リスク: フォーカス滞在が
  `MIN_FOCUS_DURATION_MS`（100ms）未満だと退場時の cache 保存自体がスキップ
  され、古い既存キャッシュが残ったまま復帰時に hit する——BUG-57 型の
  フォーカス奪取（まさに本修正が守りたいシナリオ）で、たった今の新しい
  明示意図がキャッシュに蒸発させられる。
- miss 側の `remove()` は目的と矛盾: cache miss で「この窓については何も
  分からない」状態のときに、唯一残っている情報（明示意図）を消して
  `HeuristicDefault`（Low confidence の「とりあえず ON」）を勝たせるのは倒錯。
- 30秒 TTL 自体は妥当だが、`HwndImeCache`（`(pid, class)` キー、独立に
  `HWND_CACHE_MAX_AGE_MS`=1時間）が `effective_open()` の結果（IntentStore
  経由で洗浄済みの値）を退場時に保存し `HwndCacheRestored` で `desired_open`
  へ再注入する**別経路**が新たに発見された——「最悪1時間→30秒」という総括は
  IntentStore 単体にのみ正しい。

**v3 設計 → pre-mortem #3（GO、ブロッキング指摘ゼロ）:** 実装した内容は以下
（コード参照は `crates/awase-windows/src/state/platform_state.rs`・
`crates/awase-windows/src/runtime/key_pipeline.rs`・
`crates/awase-windows/src/tuning.rs`・
`crates/awase-windows/src/state/conv_classify.rs`）。

1. `kp_apply_conv_engine_sync`: `EngineSync::SetOpen(RomajiRecovered)` のみ
   `handle_engine_activation_sync` へ。`DirectInput` は従来どおり
   `handle_engine_set_open` を使う（actuation で送る VK は両方とも無変更）。
2. `IntentStore::record()` を `dispatch_event` の汎用フックから、新設した
   `record_explicit_intent()`（呼び出してよいのは `write_sync_key`/
   `write_physical_key`/`kp_stage_post_decision` の `ExplicitUserAction`
   分岐の3箇所のみ、`applied` ゲート付き）へ移設。`DirectInput` は
   `kp_apply_conv_engine_sync` という別の呼び出し元から `handle_engine_set_open`
   を呼ぶため、この移設だけで自然に IntentStore から除外される。
   `tests/architecture_guard.rs::intent_store_record_call_sites_are_limited_to_explicit_user_actions`
   でこの3箇所限定を機械的に固定。
3. `apply_hwnd_cache_restore` の `remove()` はタイムスタンプ比較でゲート
   （`HwndImeSnapshot.recorded_ms >= RecordedTargetIntent.recorded_at_ms` の
   ときのみ無効化、意図の方が新しければ残す）。`reset_stale_ime_on_for_imm_broken`
   は `remove()` する代わりに、`last_intent` と対称の early-return を追加
   （有効な IntentStore エントリがある間は `HeuristicDefault` を書かない）。
4. `EXPLICIT_OFF_INTENT_TTL_MS` を 1時間→30秒（`EXPLICIT_ON_INTENT_TTL_MS`
   の3倍）に変更。tuning.rs の doc を「意図の寿命」ではなく「フォーカス断絶
   ギャップのカバー窓」として書き直し、`HwndImeCache` 経由の1時間残存経路を
   残存リスクとして明記。
5. `effective_open()` に override 状態遷移時のみ出す INFO 診断ログを追加
   （`ImeStateHub::intent_override_logged: Cell<bool>` で dedup）。実機
   テストが一切実行できない以上、次のインシデントをログから再構築できる
   ようにするための必須項目（pre-mortem #2 で「推奨」→ #3 で「必須」に格上げ）。

**開示済みの残存リスク（意図的にスコープ外）:**

- `HwndImeCache` 経由で IntentStore 由来の値が最大1時間残存する経路
  （上記4参照、既存構造でこの修正のスコープ外）。
- `current_focus` は `FocusChanged`（PID 変化時のみ発火）でしか更新されない
  ため、IntentStore の実効粒度は「同一ウィンドウ」ではなく実質
  「最後に PID が変わった時点の対象」＝ per-process に近い
  （pre-mortem #1 角度1/3、上記追補（v1）の「同一対象（同一 `HwndId`）へ
  フォーカスが戻った場合に限り」という表現はこの意味で訂正する）。
- `explicit_intent()`（`check_drift_correction` が使う）は IntentStore を
  見ない非対称が残る（pre-mortem #1 角度4、実害は限定的と評価済み——
  actuation 側は hub の `effective_open()` を経由するため綱引きにはならない）。
- OFF 意図が生きている間 `reset_stale_ime_on_for_imm_broken` の
  `HeuristicDefault{open:true}` が記録されなくなるため、30秒 TTL 失効の瞬間
  に shadow 側を ON へ引き戻す足場が v1 より減る（方向としては ON バイアス
  復活の防止であり改善だが、v1 との挙動差として記録しておく）。
- 30秒という値自体は実測ではなく `EXPLICIT_OFF_CACHE_SUPPRESS_MS`（10秒）
  precedent からの類推。今後の調整には実測を伴うこと（`tuning-constants.md`）。

**検証:** `cargo test -p awase-windows --lib`（342件 pass、`state::platform_state`
は `#[cfg(windows)]` のため対象外）、`cargo test -p awase-windows --test
architecture_guard`（23件 pass、新設の
`intent_store_record_call_sites_are_limited_to_explicit_user_actions` 含む）、
`cargo xwin check --tests`/`cargo xwin clippy -- -D warnings`（msvc target、
警告ゼロ）、`cargo dylint --all`（gnu target、警告ゼロ）。`platform_state.rs`
に v3 検証用の回帰テスト十数件を追加（既存5件は `record_explicit_intent`
経由に書き換え）。**wine 未導入のためこのサンドボックスでは実機相当のテスト
実行が一切できない点は v1 から変わらず——次回実機で Ctrl+無変換 →
（プロセスを跨ぐ）フォーカス変更 → 再フォーカスという手順を踏み、Engine が
ON に戻らないこと、および `[intent-store] effective_open override` ログが
想定どおりの箇所でのみ出ることを確認すること。**

### 追補3（2026-08-13、develop 統合レビューで発見: belief 側と warrant 側で「安全弁 vs 明示意図」の優先順位が逆）

**発見の経緯:** 追補2（v3）を develop へ統合する作業中の Opus レビューで、
`IntentStore` を読む 2 つの経路が**逆順で評価している**ことが判明した。追補2 自体は
この食い違いを記録していない（`explicit_intent()` が IntentStore を見ない非対称は
「開示済みの残存リスク」に挙がっているが、それとは別の項目）。

**食い違いの実体（コードで確認済み）:**

- **belief 側** `ImeStateHub::effective_open()`（`state/platform_state.rs`）は、
  まず `shadow_model.effective_open()` を評価する。その内部
  （`ImeModel::resolve_open_at()`）が `force_guards.resolve(base, has_explicit_intent)`
  を適用して guard 由来の force-ON を織り込んだ**後**に、`IntentStore` の上書きを
  重ねる。つまり **IntentStore の明示意図が安全弁（`PanicReset`）に勝つ**。
- **warrant 側** `issue_open_warrant()`（`state/open_warrant.rs`）は逆で、
  Step 0 が `guards.active_override_reason()`（`overrides_explicit_intent()==true`
  の reason ＝ `PanicReset` / `ProfilePolicy`）、Step 1 が
  `intent_store.lookup()` の順。つまり **安全弁が明示意図に勝つ**（ADR-087 §7
  round3 M2 が明文で意図した順序）。

同じ `IntentStore` を材料にしながら、belief は「意図優先」、actuation 授権は
「安全弁優先」で答えるため、原理的には **warrant=ON / belief=OFF** の乖離が生じうる。

**なぜ実害が限定的か（コードで裏取りした4点）:**

1. `overrides_explicit_intent()` が true の reason は `PanicReset` と
   `ProfilePolicy` の2つだけで、`ProfilePolicy` guard を `add` する本番コードは
   存在しない（`force_guard.rs` の `ProfilePolicy` 出現はすべてテスト内）。
   実質 `PanicReset` の1本だけが問題になる。
2. `apply_panic_reset()` は guard を立てるのと同時に、`current_focus` の
   `IntentStore` エントリを明示的に `remove()` する。したがって
   「PanicReset × 直前の明示 OFF」の綱引きは**同一対象では自己解消する**。
3. `ImeEvent::FocusChanged` の reducer は `force_guards.clear_for_focus_change()`
   で guard を全解除する（`ime_model.rs`）。かつ warrant の `target` は
   `issue_actuation_order()` が常に `shadow_model.current_focus()` から渡すため、
   belief 側と warrant 側が**別の対象を見ることはない**。つまり当初レビューが
   想定した「PanicReset guard 有効中にフォーカスが別対象へ移った窓」は、
   フォーカス移動そのものが guard を消すので成立しない。
   実際に残る窓は「PanicReset 直後、guard がまだ生きているうち
   （`apply_ime_update` の `clear_force_on_panic_reset` を伴う観測が届く前）に、
   ユーザーが同一対象で明示 IME OFF を押して新しい `IntentStore` エントリが
   record される」ケース——`UserImeSetIntent` の reducer は guard を消さないので、
   このとき belief=OFF / warrant=ON になる。
4. `issue_open_warrant()` は現時点で **ADR-090 A-1 の shadow モード**であり、
   授権が下りなくても書き込みは止まらない（`Authorization::LegacyUnwarranted`
   の `would_have_blocked` をログ・journal に残すだけ）。したがって今この乖離が
   実際に壊すのは shadow ログの数値であって、実挙動ではない。

**今後もし直すなら（未実施）:** `apply_panic_reset()` の
`intent_store.remove(current_focus)` を `intent_store.clear()` に変えると、
対象を跨いだ古い意図も一掃されて「安全弁の後に古い意図が復活する」形の乖離が
構造的に消える。ただし `clear()` は他対象の正当な明示意図まで捨てるため
（BUG-26 型の「Engine が ON に戻れない」方向のリスクではなく、逆に
「他アプリへ戻ったとき明示 OFF が守られない」方向の劣化）、トレードオフの
評価が要る。**A-2（warrant を強制へ倒す段階）に進む前には、この順序差を
どちらかに揃えること**——強制した瞬間に、上記4の「ログだけ」という緩衝が消える。

**この追補で入れた変更（コードは順序差そのものには手を付けていない）:**

- `record_explicit_intent` の doc の誤りを訂正した。「呼び出してよいのは3箇所のみ
  （`tests/architecture_guard.rs` で出現数を固定）」と書いていたが、実際に
  固定されていたのは `state/platform_state.rs` 内の `IntentStore::record` 呼び出し
  数（1）だけで、`record_explicit_intent` 自身の呼び出し元数は固定されておらず、
  3箇所目のある `runtime/key_pipeline.rs` はそのガードの走査対象ですらなかった。
- `tests/architecture_guard.rs::record_explicit_intent_call_sites_are_limited_to_real_user_actions`
  を新設（`src/` 全走査、`platform_state.rs` 2 + `key_pipeline.rs` 1 で固定）。
- `tests/architecture_guard.rs::effective_open_is_wired_to_the_intent_store_decision`
  を新設。`tests/intent_store_effective_open.rs` は判定本体
  （`IntentStore::resolve_effective_open()`）だけを検証しており、
  **`ImeStateHub::effective_open()` がそれを呼んでいるという配線自体**は
  `#[cfg(windows)]` のため Linux では 1 行も実行されない。配線を外しても Linux CI が
  全緑のままになる穴を、テキスト検査（本番コード中の呼び出しが 1 箇所で、それが
  `fn effective_open` の本体にあること）で塞いだ。実際に配線を外す変異を入れて
  本テストが落ちることを確認済み。
- `tests/intent_store_effective_open.rs` の壊れた conv 観測の作り方を、リプレイ用
  バックドア（`AnyObservation::restored_from_journal`）から本番と同じ witness 構築子
  （`Observed::<evidence::ConvOpenInference>::from_conv`、`report_conv_open_inference()`
  が使うもの）へ変更した。`report_conv_open_inference()` 自体は `#[cfg(windows)]` な
  `ImeStateHub` の `pub(crate)` メソッドで統合テストからは呼べないため、
  「観測の作り方」だけを本番と共有する形に留めている。

### 追補4（2026-08-13、PR #60 の windows-build で初検出: `effective_open()` が壁時計を読むため、合成 tick で書かれた回帰テストが実機で 1 件も上書きを発火させない）

**症状（CI）:** PR #60（BUG-51 追補 v3 を develop へ統合）の `windows-build` ジョブで
`state::platform_state::tests::apply_hwnd_cache_restore_keeps_intent_newer_than_cache`
が失敗した（GitHub Actions run 31673333355、`488 passed; 1 failed`、nextest の
fail-fast により残り 161 件は未実行）。

**原因（統合作業による退行ではない）:** `ImeStateHub::effective_open()` は
`IntentStore` の TTL 判定に `crate::hook::current_tick_ms()`（`GetTickCount64` =
OS 起動からの経過 ms）を読む。これは v1（`21ca84d1`）からずっとそうで、統合時の
リファクタ（判定本体の `IntentStore::resolve_effective_open()` への切り出し）でも
ADR-089 Phase A/B/C でも変わっていない。一方、`mod tests` は `TickMs(100)` 〜
`TickMs(600)` という**合成 tick** でイベントを流し `IntentStore` にエントリを
記録してから、引数なしの `effective_open()` を呼んでいた。実機の
`GetTickCount64()` は数分〜数日を返すため `EXPLICIT_OFF_INTENT_TTL_MS`（30 秒）を
常に超え、`lookup()` は必ず `None` を返す。つまり **IntentStore 上書きは実機の
テストでは一度も発火していなかった**。

- 失敗した `..._keeps_intent_newer_than_cache` は「意図（false）が勝つ」ことを
  期待するため、上書きが沈黙すると `HwndCacheRestored{target:true}` が復元した
  `desired_open=true` がそのまま出て落ちる。
- 直前に PASS していた `..._discards_intent_older_than_cache` は期待値が
  「キャッシュ値（true）」なので、**間違った理由で通っていた**（上書きが
  効いていないのと、意図が正しく除去されたのとが同じ結果になる）。
- 未実行だった `effective_open_survives_focus_change_via_intent_store` /
  `..._entry_expires_after_ttl` / `write_sync_key_records_...` /
  `reset_stale_ime_on_for_imm_broken_preserves_valid_intent_store_entry` も、
  同じ理由で実機では落ちる状態だった（fail-fast で走らなかっただけ）。

**本番の挙動は正しい**（この 1 点は確認済み）: `record_explicit_intent()` に渡る
`tick_ms` は `runtime/key_pipeline.rs` の 3 箇所すべてで
`hook::current_tick_ms()` 由来であり、記録側と評価側の時間軸は本番では一致する。
壊れていたのはテストの前提だけで、実機の IntentStore 上書きは動く。

**修正:**

- `ImeStateHub::effective_open_at(&self, now_ms: TickMs)` を新設し、判定本体を
  そちらへ移した。引数なしの `effective_open()` は
  `TickMs(hook::current_tick_ms())` を渡して委譲するだけの薄いラッパー。
  合成 tick でイベントを流すテストは `effective_open_at()` を使うよう全面的に
  書き換えた（期待値は 1 つも変えていない）。
- `apply_hwnd_cache_restore()` のタイムスタンプ比較ゲートを
  `IntentStore::invalidate_for_cache_restore()`（ungated な `state/intent_store.rs`、
  戻り値 `CacheRestoreVerdict::{NoIntent, Invalidated, Kept}`）へ切り出し、
  `platform_state.rs` 側はログだけにした。**BUG-51 v3 の設計意図（キャッシュの
  記録時刻が意図の記録時刻以上のときだけ無効化する）は不変**で、置き場所だけを
  Linux でも走る側へ動かした。
- Linux で毎回走る回帰を追加:
  `tests/intent_store_effective_open.rs` に `cache_restore_keeps_intent_newer_than_cache`
  /`cache_restore_discards_intent_older_than_cache`/`intent_recorded_on_a_different_clock_never_overrides`
  （**今回の欠陥そのもの**——記録と評価で時間軸がずれると上書きが沈黙することを
  固定する）、`state/intent_store.rs` の unit tests に境界条件 5 件
  （同時刻はキャッシュ勝ち、TTL 超過は `NoIntent`、他対象は無傷 等）。
- `tests/architecture_guard.rs::effective_open_is_wired_to_the_intent_store_decision`
  を拡張し、(1) `resolve_effective_open(` が `effective_open_at()` の本体にあること、
  (2) `effective_open()` が `effective_open_at()` へ 1 回だけ委譲し、
  `current_tick_ms(` を 1 回だけ読む（＝ record 側と同じ時間軸で評価する）ことを
  固定した。

**残タスク（構造的な原因）:** `state/mod.rs` の `TickMs` の doc は
「state/ 層は `hook::current_tick_ms()` を直接呼ばず、runtime 層から
タイムスタンプを注入する」と定めているが、`effective_open()` はこれに違反して
いる（runtime 側の呼び出し元が 29 箇所あり、今回の修正では書き換えていない）。
恒久対策は 29 箇所を `effective_open_at(tick)` へ寄せて state/ から壁時計読みを
無くすこと。より一般的な再発防止としては、`#[cfg(windows)]` な `mod tests` に
新しいロジックを書く前に、**判定本体を ungated モジュールへ置いて
`tests/*.rs` から Linux で走らせる**（BUG-41 で hook.rs の純粋関数を移設したのと
同じパターン）。

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

**追記（2026-09-03、BUG-107）:** `cache.toml` の `[imm_capability]` の
永続化形式は、`(process_name, class_name)` キー化に伴いプロセス名で
ネストしたテーブル形式に変わった。上記の手順（フラットな
`Qt663QWindowIcon = "..."` を削除）は現行の形式には当てはまらない——
現行形式での暫定回避手順は BUG-107 の該当節を参照。

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

## BUG-60: `conv_mode_policy = force` 運用中に LINE で全打鍵が「い」になる／IME が JIS かなになる（**クローズ**: 前提機構が ADR-094 で撤去済みのため再現不能）

**クローズ（2026-08-25）:** 本バグが発生した前提機構である `conv_mode_policy`
（force/observe）自体が、2026-08-17 のコミット `10f238b5`（ADR-094「charset軸の
追跡撤去と conv_mode_policy(force) の全撤去」）でユーザー要望によりコードから
完全撤去された。ADR-086 Phase 2/3 の force-write 機構（`force_pending`/
`consume_force_pending_and_actuate`、`force_open_pending`/
`consume_force_open_pending`）も巻き添えで全撤去されている
（`src/config.rs:14` のコメントに撤去の記録あり）。

原因（force-write が「い」化・JIS かな化を起こす経路）は最後まで未確定の
ままだったが、それを引き起こしうる force-write 機構自体がもう存在しないため、
**同じ経路での再現はあり得ない**。以下の「原因未確定」の調査内容は
force-write 機構が存在した当時の記録として残す。仮に LINE で同様の症状が
再発した場合は、本バグとは無関係の新規原因として扱うこと（BUG-08 の外部注入
`VK_KANA` による JIS かな化など、既知の別パターンをまず疑う）。

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

## BUG-61: Windows Terminal + MS-IME で JIS かな入力に固定され復旧できない（**解決不能と確定**: Win32 にローマ字/かな入力方式を外部から切り替える公式 API が存在しない、OS/IME の制約）

**症状（2026-08-08〜09 実機報告、Windows Terminal + MS-IME、conv_mode_policy=force
ソーク中）:**

- typing 中（idle ではない）に `[idle-conv-check] TsfNative: conv=0x00000009 →
  belief ObservedRomaji 変更なし` が繰り返し出続ける。conv=0x9 は
  `NATIVE(0x1)|FULLSHAPE(0x8)` = ひらがな charset だが `ROMAN`(0x10) ビットが無い、
  すなわち JIS かな直接入力状態。
- Ctrl+変換（IME 既に ON の状態で `kp_reset_to_hiragana_romaji_capsoff` を
  発火させる awase 側のリセットコンボ）を押してもローマ字に戻らない。
- awase の tray メニュー「ローマ字」「かな」を選んでも同様に切り替わらない。

BUG-60 は「force-write ソーク中に MS-IME 側で JIS かな化する」報告だったが
**因果は未確定のまま**（BUG-60 本文参照）。本症状が BUG-60 と同一原因かどうかも
未検証であり、「BUG-60 が予告していたシナリオの実例」という決めつけはしない。

**確定した事実（コード読解 + 実機報告の突き合わせ）:**

- **最も強い証拠**: `kp_reset_to_hiragana_romaji_capsoff`（`key_pipeline.rs:1152`、
  Ctrl+変換 コンボで発火）は ADR-086 Phase 1 で `ActuationTarget::capture` →
  `get_ime_conv_for_target`（read）→ `set_ime_conv_for_target`（write）という
  ターゲット同一性検証済みの read-modify-write に既に移行済みで、`NATIVE|
  FULLSHAPE|ROMAN` を明示的に立てて `KATAKANA` を落とす。**この経路ですら
  直らなかった**ことは、「宛先 hwnd の取り違え」を経路自体が構造的に排除した
  上での失敗であり、`ImmSetConversionStatus` による ROMAN ビット書き込みが
  実モードに反映されていないことの最有力な証拠。
- tray の `InputRomaji`/`InputKana`（`message_handlers.rs:649-668`、修正前）も
  `set_ime_romaji_mode_state_for_target(hwnd, romaji)` で `ImmSetConversionStatus`
  を呼ぶだけで、宛先 `hwnd` は `tray::menu_target_hwnd()`
  （メニュー表示前に捕捉した実フォーカスウィンドウ）——これは idle-conv-check が
  conv を読んでいるのと同じ hwnd であり、「宛先がズレていて効かない」という
  説明はここでも成立しない。ただし `menu_target_hwnd()` が `None` を返した場合は
  `GetForegroundWindow()`（トップレベル）にフォールバックする経路があり
  （BUG-55 でまさに否定された基準）、この分岐に落ちていた可能性までは
  ログが無く排除できていない。
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

**対応・第1段（Opus 設計相談 + Fable PM プランニング + Opus アドバーサリアル
レビュー、2026-08-08〜09、`fix/tray-romaji-vk-dbe-roman`）:**

`is_roman_reliable=false`（`state/conv_classify.rs`、TsfNative idle 経路の
自動判定を常に無効化する既存ガード）の解除や、idle-conv-check からの自動
VK_DBE_ROMAN 発火にはまだ踏み込まない — VK_DBE_ROMAN が方向指定かトグルか
未確認のまま自動化すると、正常なローマ字入力を誤ってかな化させる新しい
往復ハザードを生みかねないため。代わりに、まず **tray コマンドを実機テスト
ハーネスとして使う**最小安全な第一歩を実装した:

1. `vk.rs` に `VK_DBE_ROMAN=0xF5`/`VK_DBE_NOROMAN=0xF6` を定義。
2. `Runtime::tray_inject_romaji_mode_vk`（`key_pipeline.rs`）を新設し、tray の
   `InputRomaji`/`InputKana` から既存の IMC write と**併走**で呼ぶ。
3. `ImeHiragana`/`ImeFullKatakana`/`ImeHalfKatakana` の ROMAN ビット欠落を修正
   （こちらは今も有効な修正、下記「対応・第2段」で撤去したのは tray の
   ローマ字/かなコマンドのみ）。

Opus アドバーサリアルレビュー（2026-08-09）で Critical 1件・Major 5件を検出し
修正: (1) tray はメニュー表示直前に `SetForegroundWindow` で自分自身にフォー
カスを奪い、`WM_COMMAND` は `TrackPopupMenu` のモーダルループ内で同期配送
されるため、`tray_inject_romaji_mode_vk` 実行時点のフォアグラウンドは awase
自身のウィンドウの可能性が高くハーネスとして機能しない構造的欠陥だった
（修正: 注入前に `SetForegroundWindow(target)` で対象へ戻し検証）。
(2) scan=0 時の物理かなキー (0x70) フォールバックは BUG-08/BUG-15 追補7と
同型のかなロックトグルハザードを踏みに行くため撤去。(3) `note_explicit_
ime_action` の呼び出し順序を IMC write の前・IME 種別を問わず無条件に修正。

**対応・第2段（2026-08-09、ユーザー実機確認 → tray 経路を全廃しホットキー化）:**

上記の C1 修正を含む版でユーザーが実機確認した結果、**tray「ローマ字」
「かな」は押しても何も変化しなかった**（IMC write・VK 注入いずれも無反応）。
tray 経路にはなお交絡要因が残っていた可能性がある（IMC write との併走で
VK 単体の効果が見えない、メニュー表示自体のフォーカス遷移が TSF 側に副作用を
起こす等）ため、**tray「ローマ字」「かな」コマンドを完全に撤去**し、
`set_ime_romaji_mode_state`/`set_ime_romaji_mode_state_for_target`（IMC write
関数）も唯一の呼び出し元を失ったため削除した。

代わりに、通常のキー処理経路（`handle_wm_key_from_hook`）に **Ctrl+Alt+R
（`VK_DBE_ROMAN` 注入）/ Ctrl+Alt+K（`VK_DBE_NOROMAN` 注入）のデバッグ
ホットキー**を追加した。tray の `WM_COMMAND` と違い、この経路はユーザーが
実際に入力していたウィンドウがフォアグラウンドのままの状態で発火するため、
tray 版が抱えていたフォーカス奪取の問題が構造的に存在しない。IMC write との
併走もしない（VK 単体の効果のみを見る）。物理押下限定・down/up 完全 swallow。
`vk.rs` に `VK_R=0x52`/`VK_K=0x4B` を追加（D-1 ガード: `VkCode(0x..)` リテラル
は vk.rs 外禁止のため）。`Runtime::tray_inject_romaji_mode_vk`
（関数名は歴史的に残存、実体はホットキー用）から tray 固有の
`SetForegroundWindow`/検証ロジックを除去し簡素化した。

自動復元（idle-conv-check や `conv_mode_policy=force` からの自動発火）への
配線は依然として**行っていない**。

**実機確認チェックリスト（Ctrl+Alt+R / Ctrl+Alt+K ホットキーで確認すること）:**

0. **`conv_mode_policy` を一時的に `observe` に戻してから確認する**
   （force のままだとフォーカス変更のたびに ROMAN 込みで強制書き戻しが起き、
   VK 単体の効果と区別できない）。
1. JIS かな固定状態で Ctrl+Alt+R → ローマ字入力に復帰するか（本命）。
2. Ctrl+Alt+K → JIS かなに切り替わるか。
3. ローマ字状態で Ctrl+Alt+R を連打 → かな化しないか（**最重要**:
   `VK_DBE_ROMAN` がトグルなら化ける。この結果が今後の自動復元解禁の
   可否を左右する）。
4. IME 実 OFF 中に Ctrl+Alt+R/K → 注入スキップログが出て副作用がないか。
5. ログの `[debug-romaji-vk]` 行（MS-IME 限定判定・`MapVirtualKeyW` scan 値・
   実際に送信できたか）を確認する。
6. 何も変化しなかった場合、`docs/experiments.md`（`.claude/rules/
   experiment-logging.md`）に app/IME/再現手順を添えて記録すること
   ——同じ「VK_DBE_ROMAN を試す」着想が将来別セッションで再浮上したときに
   同じ失敗を繰り返さないため。

**追加ホットキー（2026-08-09、Ctrl+Alt+Shift+R / Ctrl+Alt+Shift+K）: IMC write
単体の切り分け。** `Runtime::debug_inject_romaji_mode_imc`（`key_pipeline.rs`）
が `ImmSetConversionStatus` による ROMAN ビット単体書き込みを、VK 注入を
一切併走させずに単独で行う。tray 版の IMC write は常に VK 注入と併走して
いたため、IMC write 単体の効果を見たことがなかった——この切り分けのために
追加した。`kp_reset_to_hiragana_romaji_capsoff`（Ctrl+変換）と同じ ADR-086
準拠の `ActuationTarget` 経由 read-modify-write だが、Caps Lock 解除や
カタカナリセット条件を持たない ROMAN ビット単体の最小構成。ログは
`[debug-romaji-imc]` タグ。チェック項目は上記1〜6と同様（Ctrl+Alt+R/K の
代わりに Ctrl+Alt+Shift+R/K を使う）。

**実機確認結果・第3段（2026-08-09、Ctrl+Alt+R/K ホットキーで確認 →
VK_DBE_ROMAN/NOROMAN も無反応）:**

Windows Terminal + MS-IME、JIS かな入力（`conv=0x00000009`）に固定された状態で
Ctrl+Alt+R（`VK_DBE_ROMAN` 注入）・Ctrl+Alt+K（`VK_DBE_NOROMAN` 注入）を
それぞれ試した。ログで送信自体は確認できる
（`[engine-input] vk=0xF5 KeyUp`/`vk=0xF6 KeyDown` が Ctrl+Alt+R/K 押下直後に
出現、`may_change_ime` が真になり 20ms の IME refresh がスケジュールされて
いる）が、**その後の `[idle-conv-check]` は一貫して `conv=0x00000009 →
belief ObservedRomaji 変更なし` のままで、実際の conv は一切変化しなかった**。

これにより IMC write（`ImmSetConversionStatus`）に続き、**実キーイベント
経由の SendInput（`VK_DBE_ROMAN`/`VK_DBE_NOROMAN`）でも Windows Terminal +
MS-IME の JIS かな固定は解除できない**ことが確認された。awase が持つ
2つの conv-mode 制御手段（IMC write・DBE 系 VK 注入）がいずれも効かない
——このアプリ・IME の組み合わせでは、一度 JIS かな入力に固定されると
awase 側からの復旧手段が現状存在しない。

**観測された副次的な現象（実害なし、記録のみ）**: 送信直後、awase 自身の
注入（`TSF_MARKER` 付き、`is_self_injected` で hook 層にて `CallNextHookEx`
に握りつぶされ、エンジンには到達しないはず）とは別に、`vk=0xF5`/`0xF6` が
再度 `[engine-input]` に現れ、`ImeKeyKind::from_vk` の分類対象外のため特別な
処理をされず通常の `PassThrough` キーとして OS へ素通りしている。`extra_info`
はログに残していないため断定はできないが、`VK_KANA` を MS-IME 自身が
`foreign-injected`（`injected=true self_injected=false scan=0x0 extra=0x0`）
として頻繁にエコーしてくる既知パターン（BUG-08/BUG-14 参照）と同様、
**MS-IME 自身が awase の DBE 系 VK 注入を受けて別の合成キーイベントを
返してきている**可能性が高い。実害（文字化け・意図しない副作用）は
観測されていないが、次にこの周辺を調査する際の手がかりとして残す。

**実機確認結果・第4段（2026-08-09、Ctrl+Alt+Shift+R/K で IMC write 単体を確認 →
これも無反応）:** VK 注入と同条件・同アプリで Ctrl+Alt+Shift+R/K を試した
ところ、こちらも conv には**一切変化なし**。tray 版の「IMC write と VK 注入の
併走」という交絡を排除した上でも無反応と確定した。

**最終結論（2026-08-09、Web 調査により確定・原理的に不可能と判明）:**
**Win32 には「ローマ字入力 ⇔ JIS かな入力（入力方式そのもの）」を外部プロセス
から切り替える公式 API が存在しない。** これは awase 側のバグではなく OS の
制約である。

Microsoft Q&A の同種の質問（"Programatically turn on/off Japanese IME
Kana/Romaji input mode"）に対する回答で明言されている:
- `ImmSetConversionStatus`（`IME_CMODE_NATIVE`/`KATAKANA`/`FULLSHAPE`）は
  ひらがな/カタカナ/英数という**文字種**は制御できるが、「ローマ字変換 vs
  かな直接入力」という**入力方式**そのものには効かない——今回の実機結果
  （NATIVE/KATAKANA/FULLSHAPE 系ビットは有効、`ROMAN` ビットだけ無反応）と
  完全に一致する。
- 唯一の代替として挙がっているのはレジストリ
  （`HKCU\SOFTWARE\AppDataLow\Software\Microsoft\IME\15.0\IMEJP\MSIME` の
  `kanaMd` 値）だが、これは古い IME バージョン（15.0 = 旧世代 MS-IME/Office
  IME）向けであり、現行 Windows 10/11 標準搭載 MS-IME（TSF ネイティブ）に
  効くかは不明・未検証。IME プロセスの再起動が必要な可能性が高く、awase から
  安全に扱える手段ではない。
- TSF ネイティブの経路 `ITfCompartment`
  （`GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION`）も調査したが、中身は
  結局 `TF_CONVERSIONMODE_NATIVE` 等の同じ文字種フラグであり、ローマ字/かな
  入力方式の切り替えには対応していない。
- コミュニティの推奨は「アプリ側から強制せず、ユーザーに IME 側の UI
  （言語バー等）で切り替えてもらう」——**IME 自身の内部経路以外に確実な
  手段は無い**、というのが公式・非公式問わず一致した見解。

**やらないこと（確定によりスコープ外）:** 自動復元、`is_roman_reliable=false`
の解除、往復ハザード対策（ヒステリシス・give-up ラッチ）、GJI 対応、
`kp_restore_kana_from_half_width` への ROMAN 注入追加、レジストリ
`kanaMd` 書き換え。IMC write・VK 注入いずれも無反応と確定し、かつ
「そもそも外部から切り替える公式手段が無い」と判明したため、この経路の
追加投資は行わない。この症状が発生した場合の唯一の確実な回避策は、
**MS-IME の言語バー（表示されていれば）でユーザー自身がかな/ローマ字を
手動切り替えする**ことのみ。

**関連:** BUG-60（同じ「JIS かな化」症状群、因果未確定）、BUG-08（`VK_KANA`
注入による JIS かな化の既知パターン、MS-IME 自身によるエコーの前例）、
BUG-14（foreign-injected IME モードキーの扱い）、BUG-15 追補7（DBE 系 VK
注入のかなロックトグルハザード）、BUG-55（`GetForegroundWindow` 基準の
書き込み先誤り）、
[ADR-086](adr/086-force-write-trigger-and-target-identity.md) §4 INV-13
（SendInput のターゲット同一性検証が構造的に適用不能という既存の例外）。

**追補（2026-08-09、「解決不能」の再検討 → Ctrl+Alt+R/K の scan を
固定値へ変更、実機未検証）:** BUG-62 追補4 で、物理 Alt+かな 押下時に
OS が **scan 付きで** `VK_DBE_ROMAN`/`VK_DBE_NOROMAN` を届けており、
それを素通しした結果 IME の入力方式が実際に切り替わったことを実機ログで
確認した（BUG-62 参照）。これは本 BUG の「VK 注入は無反応」という結論と
一見矛盾するため、差分を検討した。

`Runtime::tray_inject_romaji_mode_vk`（Ctrl+Alt+R/K ホットキー）は
`MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)` で scan を取得していたが、この
呼び出しが `VK_DBE_ROMAN`/`NOROMAN` に対し実際に何を返していたかは
ログに残しておらず不明である。上記「第3段」の記録は「`[engine-input]
vk=0xF5`」の出現＝送信は確認できたと読めるため、**scan=0 で毎回
スキップされていたとは断定できない**（`VK_DBE_ALPHANUMERIC` が
`MapVirtualKeyW` で scan=0x3A を返す前例＝BUG-15 追補7があり、DBE 系
VK が常に 0 を返すという前提自体が未検証だった）。

`tray_inject_romaji_mode_vk` の scan を `MapVirtualKeyW` 依存から
**物理「かな」キーの scan（`0x70`、JIS）に固定**する変更を行った
（`key_pipeline.rs`）。BUG-62 追補4 で実際に効くと確認済みの scan 値を
そのまま使うことで、「第3段」が試していなかった可能性のある条件を
明示的に再現する狙い。**ただし `MapVirtualKeyW` が実は既に 0x70 を
返していた場合、この変更は「第3段」と同じ入力を送るだけになり新規性が
無い**——「第3段」で実際に使われた scan 値が不明なため、この点は実機で
確認するまで判断できない。

**実機確認結果（2026-08-09、決着）:** JIS かな入力に固定された状態
（`conv=0x00000009`、ROMAN=false）で Ctrl+Alt+R を実行。
`[debug-romaji-vk] VK_DBE_ROMAN (0xF5) 注入 scan=0x70 sent=2/2` で送信自体は
確認できたが、その後の `[idle-conv-check]`/`[h1-send]` は注入前後とも
一貫して `conv=0x00000009 ROMAN=false` のまま——**conv は一切変化しなかった**。

これで「scan=0 でスキップされていたため『第3段』は無反応だった」という
可能性は排除された。実機で効くと確認済みの scan（0x70）を確実に使っても
無反応だったため、**scan 値は無関係で、SendInput 経由の
`VK_DBE_ROMAN`/`VK_DBE_NOROMAN` は MS-IME に一切認識されないと確定した**。

推定される理由（未確認の仮説）: Windows の `SendInput` は必ず
`LLKHF_INJECTED` フラグ付きでフックに届く。MS-IME の Alt+かな ハンドラが
「合成入力によるなりすまし防止」のため意図的に injected イベントを無視する
実装であれば、scan/VK をどう合わせても SendInput では原理的に発火しない
（本物の物理キー押下のみが有効）。これは BUG-62 の swallow（本物のイベント
自体を awase 側で OS に届く前に握りつぶす）以外に awase から介入する手段が
無いことの説明にもなる——「OS に一切渡さず未然に防ぐ」が唯一の実効策で
あり、「切り替わった後で SendInput によって元に戻す」という方向の対策は
構造的に成立しない。

**対応（2026-08-09）:** この結果を受け、`Runtime::tray_inject_romaji_mode_vk`
/`debug_inject_romaji_mode_imc`（Ctrl+Alt+R/K・Ctrl+Alt+Shift+R/K デバッグ
ホットキー）を完全に撤去した。`vk.rs` の `VK_R`/`VK_K`
定数、`tsf/output.rs::make_tsf_key_input_with_scan` も唯一の呼び出し元を
失ったため削除。BUG-61 は実機証拠が揃った「解決不能」として完全にクローズ
する。

## BUG-62: 物理 Alt+VK_KANA（MS-IME の「ローマ字/JIS かな入力方式切替」ショートカット）を swallow して JIS かな固着を未然に防止（BUG-61 の根本原因特定 + 予防策）

**背景（BUG-61 との関係）**: BUG-61 で「一度 JIS かな入力に固定されると
awase 側の制御手段（`ImmSetConversionStatus`・`VK_DBE_ROMAN` 注入）では復旧
不能」と確定した。本 BUG は、その**固着そのものを未然に防ぐ**ための対策。

**症状（2026-08-09 ユーザー報告）:** 「気づかない間に JIS かなに固着する」
——deliberate な操作の記憶がないまま発生する。ユーザーの手がかり:
「ALT + ローマ字キー」。

**原因（Web 調査で特定）:** MS-IME の公式キーボードショートカット
「**Alt + かな（カタカナ ひらがな ローマ字）キー**」が、ローマ字入力 ⇔
JIS かな入力という**入力方式そのもの**を切り替える。JIS キーボードの「かな」
キー（`VK_KANA`, 0x15）は正式には「カタカナ ひらがな ローマ字」とラベルが
振られており、ユーザーの「ローマ字キー」という表現と一致する。

`hook.rs` は既に BUG-08/BUG-14 対策として **foreign-injected**（MS-IME 自身が
SendInput で送ってくる）`VK_KANA` は無条件で swallow していたが、**物理押下
（`injected=false`）の `VK_KANA` は常に通過させていた**（コメント曰く「通した
結果 JIS かな化しても idle-conv-check の restore_roman が復元する」——この
前提は BUG-61 で誤りと判明済み）。Alt が物理的に押されたまま「かな」キーに
指が触れる（あるいは何らかの理由で Alt が押されっぱなしの状態で軽く触れる）
と、この物理 `VK_KANA` イベントがそのまま OS へ通り、MS-IME の Alt+かな
ショートカットが発火し、JIS かな入力へ切り替わる——これが「気づかない間に」
発生する理由を説明する。

**対応:** `hook.rs` の VK_KANA 処理に、Alt が押下中かを確認する分岐を追加。
Alt 押下中の物理 `VK_KANA` は swallow して OS に一切渡さない
（foreign-injected の場合と同じ `LRESULT(1)` パターン）。Alt 非押下時の物理
`VK_KANA`（単独の「IME ON」ショートカット、入力方式切替とは別の操作）は
従来どおり通過させる——併せて、誤っていた「restore_roman が復元する」という
コメントを是正した。

**追補1（2026-08-09）: Alt 判定に BUG-48 と同型の stale 対策を追加。**
実装直後にユーザーから「Alt down はあるが Alt up がログにすら出ない不具合が
あるのでは」という指摘があった——**BUG-48**（Win キー押下で検索 UI が開き、
その KeyUp が `WH_KEYBOARD_LL` フックチェーンの前段で消費され awase に届かず
`PHYSICAL_KEY_STATE[Win]` が恒久的に「押されたまま」スタックした不具合）と
全く同型のメカニズムが Alt でも起きうるという指摘であり、性質上ログには
一切残らない（フックの前段で消費されるため）ので実機ログでの直接確認は
できない。加えて、これは本 BUG-62 自身の実装にも影響する: もし Alt が
本当にスタックすれば、`alt_key_held()`（当初は素の `is_physical_key_down`
の OR）が恒久的に `true` を返し続け、以後の単独「かな」キー（IME ON）まで
誤って swallow してしまう二次被害を生む。BUG-48 の対策（`win_key_held()`、
`WIN_KEY_HELD_STALE_MS` 以上「押されたまま」の値を stale として無視する）
と全く同型の `alt_key_held()` を新設し、Alt 押下判定をこちらに置き換えた。
新規タイミング定数は追加せず `WIN_KEY_HELD_STALE_MS`（2,000ms、Win キー用の
未実測の暫定値）を再利用した——対象キーを問わず「OS 側 UI にキーイベントを
横取りされた」を検知する一般的な性質の値と判断したため
（`.claude/rules/tuning-constants.md`: 実測無しの新規定数追加を避ける）。

**検証:** `cargo check`/`clippy`/`cargo test -p awase-windows`（lib 286件・
architecture_guard 21件・golden_scenarios 22件）全緑。`hook_callback` は
`unsafe extern "system" fn` で `KBDLLHOOKSTRUCT` を直接扱うため、既存の
VK_KANA swallow ロジックと同様に Windows 実機以外でのユニットテストは
できない（`.claude/rules/fix-requires-evidence.md` (b) 適用: 本エントリの
記録をもって代替する）。**Windows 実機での動作確認は未実施**——特に
`alt_key_held()` の stale 判定は「Alt が本当にスタックする」事象自体が
未確認（ユーザーの仮説段階）であり、対策の要否・`WIN_KEY_HELD_STALE_MS`
流用の妥当性ともに実機での経過観察が必要。

**追補2（2026-08-09）: Alt+かな swallow 自体が新たな副作用を生みうる点への
対策（SC_KEYMENU マスク）。**

ユーザーから「Alt down はあるが Alt up が来ない不具合が2週間ほど前から
起きている気がする」との申告があり、**発生時期が本 BUG-62 の実装（本日）
より前**であることが判明した。したがって追補1の stale 対策が対象とする
「OS が Alt の KeyUp を失う」現象自体は、本日の変更が原因ではない
（原因は依然未確定、既存の何らかの環境要因の可能性が高い）。

一方で、この調査の過程で **BUG-62 の swallow 実装自体が新たに別の副作用を
生みうる**ことに気づいた: 「かな」キーを丸ごと OS へ渡さないため、OS 視点
では「Alt が何も修飾せず単独でタップされた」ことと区別がつかない。Windows
には Alt を単独で離すとシステムメニュー（`SC_KEYMENU`、アクセラレータ
探索モード）を起動する仕様があり、これが起きると以後の入力がメニュー
ナビゲーションとして食われる——ちょうどユーザーが説明した症状と同じ形の
別の不具合を、今回の対策自体が生みかねなかった。AutoHotkey の
`Send`/`#MenuMaskKey` が全く同じ問題設定に対して「ダミーの Ctrl キーを
挟んで OS に『Alt は何かを修飾した』と誤解させない」という標準的な回避策を
持っており、同じ手法を採用した:  Alt+かな を swallow する KeyDown の
タイミングで、`INJECTED_MARKER` 付きのダミー Ctrl down+up を自己注入する
（`is_self_injected` で弾かれエンジンには渡らず、OS 側の Alt 状態機械だけを
補正する）。

これは**追補1の stale 対策とは独立した、別の懸念に対する予防策**であり、
2週間前からの申告された不具合そのものの原因究明には至っていない
（原因は未確定のまま）。

**検証:** `cargo check`/`clippy`/`cargo test -p awase-windows`（lib 286件・
architecture_guard 21件・golden_scenarios 22件）全緑。**Windows 実機での
動作確認は未実施**——SC_KEYMENU 発火の有無・ダミー Ctrl 注入の副作用（他の
Ctrl 系ショートカットとの意図しない干渉が無いか）ともに要確認。

**実機確認してほしいこと:** Alt を押しながら物理「かな」キーを押しても
JIS かなへ切り替わらなくなったか。また、Alt を押していない単独の「かな」
キー押下では従来どおり IME が ON になるか（回帰が無いか）。加えて、
Alt+かな 操作の直後にメニューが意図せず開いたり、その他 Ctrl 系
ショートカットが誤発火したりしないか。**2週間前からの「Alt up が来ない」
申告そのものの再現条件は依然不明**——次回発生時は、その直前に何を操作
していたか（Alt+かな か、別の Alt 系ショートカットか）を記録してほしい。

**関連:** BUG-61（IMC write・VK_DBE_ROMAN いずれも JIS かな化からの復旧が
不能と確定、本 BUG の対策方針の根拠）、BUG-08（`VK_KANA` 注入による JIS
かな化の既知パターン）、BUG-14（foreign-injected IME モードキーの扱い）、
BUG-48（Win キー版の同型 stale 問題、追補1が踏襲した対策パターン）。

**追補3（2026-08-09、`git bisect` で「2週間前からの Alt+かな 後に入力不能」
の原因コミットを特定）:** ユーザーとの共同作業で develop の履歴を二分探索
（`git checkout <hash>` を約10回反復、実機で再現有無を確認）した結果、原因は
**`b38d67f8`**（2026-07-05、"合成 VK_KANA によるかなロック反転で JIS かな化
する問題の二層防御"、BUG-08 の初回対策）に一致した。

このコミットが `hook.rs` に VK_KANA 専用処理を**史上初めて**導入し、
「LLKHF_INJECTED 付き（foreign-injected）の VK_KANA は Alt の状態を一切見ずに
常時 swallow する」というロジックを追加した。本セッション中に共有された
実機ログでは、VK_KANA イベントが一貫して `injected=true` として記録されて
いる（MS-IME 自身のエコーと解釈していたもの）。ユーザーの物理的な「かな」
キー押下自体もこのフラグを伴って届く環境（JIS キーボードドライバがこの種の
特殊キーをソフトウェア的に合成することは珍しくない）であれば、**Alt を
押しながら物理「かな」キーを押した場合、このコミット以降ずっと Alt の状態を
見ずに無条件で swallow され続けていた**ことになる。BUG-62 追補2 で特定した
「かな キーを丸ごと OS へ渡さないと、OS は Alt が単独タップされたと誤認し
`SC_KEYMENU` を起動、以後の入力がメニューナビゲーションとして食われる」と
いうメカニズムと組み合わせると、症状（Alt+かな の後に入力不能、かなキー
以外では起きない、以前は無かった＝このコミットより前には存在しなかった
処理）に矛盾なく一致する。

**対応:** BUG-62 追補2 で追加したダミー Ctrl 注入によるメニューマスク対策
（`inject_alt_menu_mask`、旧 `alt_key_held()` 分岐にのみ適用）を、
foreign-injected 分岐（`b38d67f8` 由来、本追補の原因箇所）にも Alt 押下中は
適用するよう拡張した。foreign-injected の swallow 自体（BUG-08 対策として
必要）は維持し、Alt 押下中に限り追加でマスクキーを注入する。

**この特定は git bisect（履歴上の原因コミットの特定）によるものであり、
「なぜそのコミットの変更が Alt との組み合わせで問題を起こすか」という
メカニズム自体（LLKHF_INJECTED フラグ付きでユーザーの物理かなキーが届く、
という前提を含む）は BUG-62 追補2 の推論の延長であり実機の直接証拠は無い。
Windows 実機での動作確認は未実施。**

**実機確認してほしいこと:** Alt を押しながら物理「かな」キーを押した後、
通常どおり入力できるか（本命）。単独の「かな」キーでは従来どおり IME が
ON になるか（回帰が無いか）。

**関連:** BUG-08（本追補が原因と特定した `b38d67f8` の対策そのもの）、
BUG-62 追補2（SC_KEYMENU マスク対策の初出）。

**追補4（2026-08-09、実機ログで真因を特定・修正）:** 追補3 を適用した
`365ae89` を実機（Windows Terminal + MS-IME）で再テストしたところ、
「Alt+かな の後に入力不能」症状は**再発した**（修正されなかった）。まず
`inject_alt_menu_mask()` が実際に発火したかを確認するログを追加
（`d265961`）した上で、ユーザーに Alt+かな 押下を含む区間のログを
再取得してもらった。

ログを精査した結果、これまで3回分の対策がすべて的外れだったことが
判明した。ログ中に大量に出現する「foreign-injected VK_KANA」
swallow（`[hook] foreign-injected VK_KANA {down,up} を swallow`）は、
Alt の押下有無に関わらず**物理キー操作と無関係に常時**（打鍵ごとに
数msおきに）発生しており、`inject_alt_menu_mask` も正しく発火して
いた（`sent=2/2`）。つまり追補1〜3 で強化してきた VK_KANA 分岐は
正常に動作していたが、そもそも**今回の症状の引き金ではなかった**。

実際の引き金は、Alt を物理的に押しながら「かな」キーを押した瞬間に
発生していた、以下のイベント列だった:

```
[engine-input] vk=0xF5 KeyUp   ... mods(...a=true...)
[relay-passthrough] PassThrough idle: direct OS pass-through (vk=0xf5 up)
[engine-input] vk=0xF6 KeyDown ... mods(...a=true...)
may_change_ime key passed through → IME refresh scheduled (20ms)
[relay-passthrough]（vk=0xf6 down も同様に素通し）
```

`0xF5` = `VK_DBE_ROMAN`、`0xF6` = `VK_DBE_NOROMAN`。BUG-61 調査で
発見していた「ローマ字/JIS かな入力方式切替専用の Win32 仮想キー」
そのものが、**物理 Alt+かな 押下時に Windows のキーボードレイアウト
ドライバによって `hook_callback` まで届いている**ことが実機ログで
直接確認できた。`hook.rs` の swallow ロジックはこれまで
`vk == VK_KANA` しか見ていなかったため、この `0xF5`/`0xF6` は
完全に素通しで OS に渡り、実際に入力方式の切替（BUG-61 で復旧不能と
確定済み）を起こしていた。これが3回の対策がすべて効かなかった理由
そのものである。

**対応:** `hook_callback` に `VK_DBE_ROMAN`/`VK_DBE_NOROMAN` 専用の
swallow 分岐を追加した（VK_KANA 分岐とは独立、同じ場所に隣接して
配置）。この2キーは BUG-61 の実機調査で「一度切り替わると
`ImmSetConversionStatus`・VK 注入のどちらでも復旧不能」と確定済み
なので、Alt 押下有無を問わず常に swallow する。ただし swallow 時に
Alt が押されていれば、VK_KANA 分岐と同じ理由（キーを丸ごと OS へ
渡さないと「Alt 単独タップ」と誤認され `SC_KEYMENU` が起動しうる）
で `inject_alt_menu_mask()` を適用する。

**この特定はユーザー提供の実機ログを直接読んだことによるもので、
git bisect のような間接推定ではない**（追補3 との違い）。

**実機確認結果（2026-08-09）:** ユーザーが `259aeaed` を実機（Windows
Terminal + MS-IME）で再検証し、「再発しないようになりました」と確認。
Alt+かな 後の入力不能症状は解消。

**追補5（2026-08-09、設定でユーザーがオプトアウトできるようにした）:**
JIS かな直接入力を意図的に使いたいユーザー（想定: awase の Engine を
OFF にして Windows 標準の JIS かな入力を使う運用）向けに、
`GeneralConfig::swallow_alt_kana_input_method_switch`（既定値 `true`
＝従来どおり常時 swallow）で無効化できるようにした。awase-settings の
「詳細設定」タブに対応するチェックボックスを追加。`false` にした場合、
Alt+かな は素通しされ MS-IME の入力方式が実際に切り替わる（BUG-61 で
復旧不能と確定済みなので、その場合の復旧は言語バー等 IME 自身の UI に
限られる）。

Alt+かな を「JIS かな⇔ローマ字の実用的なトグル機能」として awase が
積極サポートすることは意図的にやらない選択をした: awase は常にローマ字
綴りの VK 列で文字を出力しているため（`output/vk_send.rs`）、MS-IME が
JIS かな直接入力モードに切り替わった状態で awase 経由の入力を続けると
出力が壊れる。両モードに対応するには awase の出力パイプライン自体を
ROMAN/NOROMAN ビットに応じて切り替える必要があり、過去に何度も事故って
きた領域（BUG-08/BUG-15/BUG-61/BUG-62）だけに、思いつきで着手せず必要に
なったら別途 ADR を切る方針とした。

**関連:** BUG-61（`VK_DBE_ROMAN`/`VK_DBE_NOROMAN` そのものの定義と
「復旧不能」の確定根拠）、BUG-62 追補1〜3（的外れだった VK_KANA 側の
対策、ただしそれ自体は BUG-08 対策として引き続き必要）。

---

## BUG-63: 仮想デスクトップ切替後 Windows Terminal で半角のつもりが「くした」とかな変換される（IME belief と actuation の根拠が未分離）

**アプリ:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` →
`Windows.UI.Input.InputSite.WindowClass`、TsfNative プロファイル）。

**IME:** Google 日本語入力（GJI）。IME OFF（半角英数モード）、conv ビットは
直前セッションの NATIVE（ひらがな相当）が stale に残留。

**再現手順 / 症状（2026-08-10 実機ログ）:** `Win+Ctrl+→` で仮想デスクトップを
切替え、Windows Terminal にフォーカスが移った直後、IME キーを一切押さずに
半角のつもりで `mise` と入力。IME は物理的には閉じたままだったが、engine の
belief が conv ビット由来の間接推測1件だけで ON に復帰し、後半の `i`/`s`/`e`
がかな変換され「くした」というリテラルが出力された。さらに実 IME への
`VK_DBE_HIRAGANA` 相当の force-ON も発火し、実際に IME が意図せず ON に
なった。

**原因（コード読解で確定）:** `ImeModel::effective_open()`
（`state/ime_model.rs`）が2つの異なる目的——(1) NICOLA engine の内部挙動決定
（belief、誤りは可逆・低コストでよい）と (2) OS 側 IME への実際の書き込みの
授権（actuation、誤りは不可逆・高コスト）——を同じ `bool` で兼ねていた。
`ObservationStore::derive_open()` の「Medium confidence 観測は無競合なら
1件でも多数決成立」という仕様（これ自体は別の既知バグ **BUG-26** の修正が
意図的に依拠している正しい挙動）が、conv ビット由来の間接推測
（`ObservationSource::ConvOpenInference`）にもそのまま適用され、
`is_eligible_for_ime_force_on()` がこの `effective_open()` を直接 actuation の
根拠として読んでいたため、弱い間接観測1件が実 IME への書き込みまで
authorize してしまっていた。

BUG-26 と本バグは**同じ conv 観測値（NATIVE 系）で実際の IME 状態が正反対**
（BUG-26 は開いていた、本バグは閉じていた）というペアであり、conv ビットには
両者を区別する情報が原理的に含まれていない。そのため
「`ConvOpenInference` の confidence を下げる」「corroboration を必須化する」
といった confidence モデルの調整では**原理的に解決不可能**（TsfNative には
第2の open 観測ソースが構造的に存在しないため、これらの調整は必ず BUG-26 を
再発させる）。

**設計調査:** Opus・Codex CLI との3ラウンドにわたる相互レビュー（シナリオ
シミュレーション形式）を経て、[ADR-087](adr/087-open-belief-actuation-warrant-separation.md)
「IME open/close belief における『内部信念』と『actuation の根拠』の分離」を
策定した。round1〜3 で見つかった主な設計上の落とし穴（詳細は ADR §7）:

- round2: 対症療法（confidence 調整）を修正すると、BUG-16
  （Windows Terminal で belief=ON・実IME=OFF のまま放置される別バグ）の
  回復経路が丸ごと消えることが判明——`desired_open`（awase 自身の SSOT）への
  フォールバック分岐が必要と判明。
- round3: そのフォールバック分岐（`OwnSsot`）を `AppImeProfile` の生の値で
  分岐する設計にしたところ、`CASCADIA_HOSTING_WINDOW_CLASS` が
  `Imm32Unavailable` に誤分類され（`class_names.rs` が「2026-07-05 実機
  バグ」として明文で禁止している判定方法そのもの）、**同じ BUG-16 が別経路で
  再発**することが判明——`AppImePolicy.default_feedback` ベースの判定に訂正。
- 他にも、優先順位付きゲートの Step 順序を誤ると `PanicReset` 安全弁が明示
  意図に阻まれる新規バグを作る等、複数の「ラウンドNの修正がラウンドN+1で
  別の穴を生む」という振動が発生した。

**対応（純粋ロジックを実装し Linux 上のテストスイートで固定、2026-08-10）:**
`.claude/rules/fix-requires-evidence.md` の (a) を満たす形で、belief と
actuation warrant を型で分離する ADR-087 §2.3 P11〜P16 の純粋ロジックを、
Windows ゲート無しの新規モジュールとして実装した:

- `state/ime_model.rs`: `resolve_open_at`/`effective_open_at`（`Instant`
  引数化、判定内訳を返す `DecidedBy` 診断API）
- `state/observation_store.rs`: `derive_open_filtered`/`DeriveOutcome`
  （`derive_open()` 本体と乖離しない形で observation の provenance を返す）
- `state/ime_event.rs`: `ObservationSource::authority()`
- `state/intent_store.rs`（新設）: `IntentStore`（対象=`HwndId` ごとの明示
  意図、ON/OFF 非対称 TTL）
- `state/open_warrant.rs`（新設）: `OpenWarrant`/`WarrantBasis`/
  `issue_open_warrant()`（Step0〜4 の優先順位付きゲート）
- `state/force_guard.rs`: `active_override_reason`/`active_heuristic_reason`
  アクセサ追加

回帰テストは各モジュール内の pinned test として配置（本バグ・BUG-16・
BUG-19 系・BUG-26 の各シナリオに doc コメントで対応関係を明記）。
`cargo test -p awase-windows --lib`（326件）・`golden_scenarios`（22件）・
`architecture_guard`（21件）・`journal_replay`（1件）・
`drift_correction_replay`（2件）・`layer_boundary_guard`（8件）全緑、
`cargo clippy -p awase-windows --lib --tests`（pedantic/nursery deny）で
新規コードに指摘なし。

**未実施（重要な限界）:** 上記は ADR-087 の Phase 0〜2' 相当の**純粋ロジック**
のみで、実際の `runtime/`・`platform_state.rs`（Windows専用、このセッションの
サンドボックスでは検証不能）への配線（Phase 3）・GJI `shadow_on` no-op の
bypass（INV-28）・実機ソークは未実施。**Windows 実機での動作確認は未実施**。
また、明示意図が一度も無い場合（本バグの実際の再現操作列がまさにこれ）、
engine の belief が一時的に ON になること自体（症状の前半、かな変換の混入）は
本 ADR のどの Phase でも解消できない——これは confidence 調整と同じ理由で
情報論的に不可能なため、意図的に「実 IME への書き込みは必ず止める」ことのみを
保証する設計とした（ADR-087 §2.3 P13 参照）。

**追補（2026-08-10、round4）:** 上記実装を Opus に最終レビューさせたところ、
実装して初めて見える新規の欠陥が4件（must-fix）見つかった。特に重要なのは
`IntentStore` の OFF 意図に有効な失効条件が1つも無く（TTL なし・eviction
なし）、対象ごとに永続する設計と組み合わさって drift correction が永久に
再同期できない固着を作りうる欠陥（既存 precedent `HwndImeCache` の
`HWND_CACHE_MAX_AGE_MS` パターンとの乖離）。修正し、`tuning.rs` に
`EXPLICIT_OFF_INTENT_TTL_MS`（ON より意図的に長い、`HWND_CACHE_MAX_AGE_MS`
と同値）を新設。あわせて `resolve_open_at()` の診断フィールド
`guard_override` が「guard は active だが実際には override していない」
場合にも誤って値を返す診断API自身のバグも修正した
（`ForceGuardSet::resolve()` を新設し唯一の判定点に統一）。

さらに、`issue_open_warrant()` の入力（明示意図・guard・観測・
`HeuristicDefault`・`default_feedback`・`requested` 等）が実質すべて
有限個の離散値の組み合わせであることに着目し、1152通りの全組み合わせを
実装から独立に書いたオラクル関数と突き合わせる網羅テストを追加した
（`exhaustive_step_priority_matches_independently_written_oracle`）。
シナリオを人手で選ぶ方式では round2→round3 で新しいバグが繰り返し
見つかったが、この網羅テストは Step 0〜4 の優先順位ロジックについては
「選ばれなかった組み合わせ」を残さない。詳細は
[ADR-087 §8.5〜8.8](adr/087-open-belief-actuation-warrant-separation.md#85-実装後の最終レビュー記録round4opus)。

**関連:** BUG-16（同じ Windows Terminal 仮想デスクトップ切替シナリオの逆方向、
belief=ON・実IME=OFF）、BUG-19（明示 OFF 意図が観測に上書きされない防御）、
BUG-26（conv 観測1件での belief 復帰、本バグと同じ機構への依拠）、BUG-33
（`FocusProbe` の belief 書き戻し混入）。

## BUG-64: config1.db に旧 awase 実験由来の残骸バインドが実在する（F13/F14/F21/F22、バグではなく既知の事実の記録）

**症状ではなく事実の記録:** ADR-091（charset 軸の設計）§1.4 項目3 の実機
`config1.db` 抽出（clipwire 経由で取得、`wire.rs` の protobuf 最小パーサで
`custom_keymap_table` を復元）で、以下の旧 awase 実験由来の残骸バインドが
ユーザー実機に実在すると確認・削除した:

- **F13**: `DirectInput → IMEOn`
- **F14**: `Precomposition`/`Composition`/`Conversion → IMEOff`
- **F21/F22**: `IME ON`/`IME OFF`

いずれも過去のセッションで `config1.db` へ書き込んだ実験の残骸であり、
現行コードのどの経路からも意図して送信されていない（=バグではない）。

**なぜ記録するか:** ADR-091 §D3.2 が新設する専用 Fn キー変換モードは
**F21** を Composition/Conversion 時の `SwitchKanaType` バインド先として
想定しており、上記の残骸バインドと同一のキー番号である。この残骸自体は
2026-08-13 の実機確認で既に削除済みであり、`VK_F21`/`VK_F22` は ADR-057 が
物理キー非存在・ターミナル安全と確認済みの VK のため危険なキーではない
（`VK_F13`/`VK_F14` とは異なる。あちらはターミナルエスケープシーケンス漏れの
実機確認があり常に危険）。そのため Phase 1（`GeneralConfig::
muhenkan_solo_tap_dedicated_fn_key` の実装、2026-08-14）の
`validate_dedicated_fn_key`（`src/config.rs`）は `VK_F21`/`VK_F22` を許可
範囲に含めている。ただし `awase-gji-config` の書き込み機能（衝突検出込み、
ADR-091 §4 Phase1-3）はまだ実装されていないため、**config1.db 自動判定が
入るまでの間、ユーザーがこの隠し設定を手動で有効化する際は GJI 側の既存
キー設定（config1.db）で F21/F22 が既に別の意味に割り当てられていないか
自分で確認すること**（`validate_dedicated_fn_key` の警告文にも明記）。
`awase-gji-config` の書き込み機能を実装する際は、この既知の残骸を
「他アプリ由来の未知の衝突」と誤認せず、awase 自身の残骸として正しく
上書き・除去できる設計にすること。次に `config1.db` 関連の作業をする際、
この残骸バインドの存在自体に驚かないための記録でもある。

**関連:** [ADR-091](adr/091-idempotent-charset-axis-gji-recommended-msime-self-responsibility.md)
§1.4、[ADR-057](adr/057-gji-keybind-f13f14-to-f21f22.md)（F13/F14 を避け
F21/F22 へ移行した経緯）、[ADR-067](adr/067-vk-ime-on-off-migration.md)
（config1.db バインド撤廃の経緯）。

## BUG-65: `TSF_OBS_TEST_LOCK` 共有ロックが `.lock().unwrap()` で non-poison-resilient なため、1テストの真の失敗が無関係な10テストの偽陽性失敗を誘発する（テスト分離バグ、行4661 の再発・別軸）

**症状:** 2026-08-14、ADR-091 Phase 1（T4-T10）ブランチを実機 Windows で
`cargo test -p awase -p awase-windows -p awase-gji-config` 実行したところ、
私（Claude）が一切触れていない `tsf::probe`・`tsf::warmup::literal_detect_fsm`・
`tsf::warmup::probe_fsm` の3ファイルにまたがって計10件の `PoisonError` 起因の
テスト失敗が発生した（`cargo test` 標準の並列実行下）。

**真因:** `tsf::probe::tests::probe_fallback_waits_total_max_ms`
（`probe.rs:776`、`GetTickCount64`/`Instant::now()` 実時間依存のフォール
バック待機テスト）が実機ハードウェア上でのみ顕在化するタイミング非決定性
（同ファイル内 `check_now_returns_stale_confirm_when_write_evidence_predates_epoch`
のコメントが2026-07-25に同種の非決定性を既に記録済み）により
`assert!(elapsed >= 60, "fallback too short: {elapsed}ms")` で真に失敗し、
`TSF_OBS_TEST_LOCK.lock().unwrap()` を保持したままパニックした。

`TSF_OBS_TEST_LOCK` は行4661（2026-07-25、`TSF_OBS` 保護のためのロック統一）
以降、`observer.rs`/`probe.rs`/`literal_detect_fsm.rs`/`probe_fsm.rs` の
4ファイルで正しく同一インスタンスを共有している（統一自体は機能している）。
しかし全23箇所の取得コードが素の `.lock().unwrap()` だったため、上記1件の
真の失敗がロックを **poison** させ、その後 `cargo test` が同一プロセス内で
実行する他の22テストのうち10件が `.lock().unwrap()` の時点で
`PoisonError` を受けて連鎖的に失敗した（実際のテスト内容とは無関係な
偽陽性）。`tsf::warmup::ms_ime_ready_coro` は独自の別ロック
（`GATE_TEST_LOCK`、`OUTPUT_GATE` 保護用）を使っており、かつ
`.lock().unwrap_or_else(std::sync::PoisonError::into_inner)` で
poison-resilient に書かれていたため、この連鎖には巻き込まれなかった
（が、同テストの assertion 自体は別途、真に失敗していた。原因未確定・
本バグの対象外）。

**修正:** `TSF_OBS_TEST_LOCK`/`VETO_TEST_LOCK` を取得する全23箇所
（`observer.rs` 4箇所、`probe.rs` 12箇所、`literal_detect_fsm.rs` 4箇所、
`probe_fsm.rs` 3箇所）を `ms_ime_ready_coro.rs::GATE_TEST_LOCK` と同じ
`.lock().unwrap_or_else(std::sync::PoisonError::into_inner)` に統一した。
各テストは取得直後に自分が使うグローバル状態（`TSF_OBS` の関連フィールド）
を明示的に `store()`/`reset_*()` で上書きしてから assert する作りのため、
前のテストが poison させた状態を引き継いでも安全（既存コードの前提を
変えていない）。`probe_fallback_waits_total_max_ms` 自体の実時間非決定性
（真因）は未修正のまま残っている——次にこのテストが再度真に失敗しても、
今回のような無関係テストへの連鎖は起きなくなるが、このテスト自体の
flaky性を根治するものではない。

**関連:** 行4661（`TSF_OBS_TEST_LOCK` 統一の初出）、
[fix-requires-evidence](../.claude/rules/fix-requires-evidence.md)。

**2026-08-14 追補（残る2件のうち `ms_ime_ready_coro` 側の真因を特定・修正）:**
上記の poison-resilience 対応後にユーザーが実機で再実行したところ、10件の
連鎖失敗は解消し、`probe_fallback_waits_total_max_ms`（真因未修正、上記のまま）
に加えて `tsf::warmup::ms_ime_ready_coro::tests::phase1_does_not_hold_output_gate_only_phase2_does`
が単独で失敗した（`OUTPUT_GATE.is_active()` が Phase 2 到達時点で期待に反し
`false`）。

このテストは自前の `GATE_TEST_LOCK`（当時ファイルローカル）を取得してから
`OutputActiveGuard::begin()` を実際に呼ぶ Phase 2 へ進むが、
`output/probe_io.rs` の GJI 系テスト2件
（`tsf_mode_nc_not_fired_gji_long_idle_gji_healthy_enables_literal_detect`、
`long_idle_tsf_mode_keeps_literal_detect`。いずれも `plan.needs_literal=true`
で `dispatch_probe_actions` → `GjiWarmupCoro::apply_transmit_done` →
`literal_detect_guard = Some(OutputActiveGuard::begin())` に到達する）が
**一切ロックを取らずに同じプロセス全体共有 `OUTPUT_GATE` を実際にミューテート**
していた。`TsfProbeCoro` 用の `make_chrome_machine()`（同ファイル）は
`OutputActiveGuard::noop_for_test()` を経由するよう既に対処済みだったが、
`GjiWarmupCoro` の `literal_detect_guard` は construction 時ではなく
tick 中に遅延生成されるため同じ回避策がなく、この非対称性が見落とされていた。
`cargo test` の並列実行下で、この2件のいずれかが `ms_ime_ready_coro` の
Phase 2 の直前・最中に割り込むと、`OUTPUT_GATE.depth` が2つのテストで
共有されてしまい、`ms_ime_ready_coro` 側が `is_active()` を確認する瞬間に
`probe_io.rs` 側が先に drop してしまう（またはその逆）ことで偽陽性の
失敗を起こしうる。

**修正:** `tsf/probe_bridge.rs` に `TSF_OBS_TEST_LOCK` と同型の
`OUTPUT_GATE_TEST_LOCK`（`#[cfg(test)]`）を新設し、`ms_ime_ready_coro.rs`
のファイルローカル `GATE_TEST_LOCK` をこれへの re-export に置き換えた上で、
`output/probe_io.rs` の上記2テストにも同じロックを追加した。`TransmitSingleVk`
アクションを dispatch するテストは本ファイルに存在しない
（`apply_vk_sent` 経由の同種の穴は現状なし）ことを grep で確認済み。

**関連:** BUG-58（`OutputActiveGuard` を Phase 2 のみで確保する設計の初出）、
`tsf/probe_bridge.rs::OUTPUT_GATE_TEST_LOCK`。

**2026-08-15 追補2（真因はロック不足ではなく `noop_for_test()` 自体の実装バグと判明）:**
上記のロック追加（`OUTPUT_GATE_TEST_LOCK`）を実機で再検証したところ、
`phase1_does_not_hold_output_gate_only_phase2_does` は**まったく同じ行**で
**まったく同じ結果**（100%決定論的、レースではない）で失敗し続けた。

真因は `OutputActiveGuard` がフィールドを持たないユニット構造体だったこと。
`begin()`（`depth` を実際に +1 する）と `noop_for_test()`（何もしないはず）の
両方が同じ型 `Self` を返すため、`Drop` 実装は生成経路を区別できず、
**`noop_for_test()` で作ったガードが drop されるときも無条件に
`OUTPUT_GATE.depth.fetch_sub(1)` を実行していた**。`depth: AtomicU32` は
0 からの `fetch_sub` でパニックせず `u32::MAX` 付近へラップアラウンドする
（atomic 演算はデバッグビルドでもオーバーフローパニックしない）。

`output/probe_io.rs::make_chrome_machine()`・`tsf/warmup/chrome_probe.rs`
（2箇所）・`tsf/warmup/probe_fsm.rs` の計4箇所が `noop_for_test()` を使っており、
これらを使うテスト（`probe_io.rs` の `TsfProbeCoro` 系テストの大半、
`chrome_probe.rs`・`probe_fsm.rs` の一部）が1つでも走ると、その `machine`
変数が drop される瞬間に `OUTPUT_GATE.depth` が本来ありえない値まで
壊れる。一度ラップすると、以降どれだけ本物の `begin()` が `depth` を
+1 しても `prev == 0`（`OUTPUT_GATE.active` を `true` にする唯一の条件）
に二度と一致しなくなり、**`cargo test` 1プロセスの残り全テストで
`OUTPUT_GATE.active` が恒久的に `true` にならない**。テスト実行順序が
機械ごとにおおむね安定しているため、ロックの有無に関わらず同じ
テストバイナリでは同じ失敗が再現し続けていた（見かけ上「レース」に
見えたが実際は蓄積した破損状態、真の意味でのデータ競合ではなかった）。

**修正:** `OutputActiveGuard` に `real: bool` フィールドを追加し、
`begin()` は `real: true`、`noop_for_test()` は `real: false` を持つように
した。`Drop` は `real == false` なら即 return し、`OUTPUT_GATE` に一切
触れないようにした。上記のロック追加（追補1）は無駄ではなく
（本物の並行 `begin()` 同士の保護として引き続き有効）残してある。

**教訓:** ゼロフィールドのユニット構造体で「複数のコンストラクタ経路を
使い分ける」設計は、`Drop` 実装が経路を判別する手段を持てないため、
最低1つは生成経路を区別できるフィールドが必要（コンパイラは警告しない
——`Drop::drop` はどのコンストラクタ経由かを知る術がなく、常に同じ
コードパスを実行する）。

**関連:** `tsf/probe_bridge.rs::OutputActiveGuard`。

**2026-08-15 追補3（残る `probe_fallback_waits_total_max_ms` も真因判明・修正）:**
追補2の修正後、`ms_ime_ready_coro` 側は実機で解消を確認したが、
`probe_fallback_waits_total_max_ms`（"fallback too short: 10ms"）は
2回とも同じ場所・同じ結果で再現した（665 passed; 1 failed）。

真因はテストが**2つの異なるクロック**を混在させていたこと。`check_now()`
（SUT・被テストコード）は `crate::hook::current_tick_ms()`
（`GetTickCount64`）で残り時間を判定するが、テストの `elapsed` 計測は
`std::time::Instant`（`QueryPerformanceCounter` 由来、GetTickCount64 とは
独立した別クロック）を使っていた。VM 環境ではハイパーバイザーが
ゲストの `GetTickCount64` 側だけを定期的にホスト時刻へ補正（ジャンプ）
させることがあり、`Instant` は影響を受けない。このため「`GetTickCount64`
上は 100ms 経過した（=SUT は正しくフォールバックを完了した）のに、
`Instant` 上はまだ 10ms しか経っていない」という、SUT の挙動としては
正しいのにテストの計測クロックが食い違うことによる誤検知が起きていた
（実際に SUT にバグがあったことは一度もない）。

**修正:** `elapsed` の計測を `Instant` から `current_tick_ms()`（SUT と
同じクロック）に変更した。同ファイル内の他の `Instant` 使用テスト
（`probe_phase2_detects_already_settled` 等）は本バグの報告対象では
なかったため未変更。

**関連:** `tsf/probe.rs::probe_fallback_waits_total_max_ms`。

**2026-08-15 追補4（追補3は誤診断・真因は別ファイルの無施錠 `gji_monitor_ok` 書き込み4箇所）:**
追補3のクロック統一後も実機で再現し続け（"fallback too short: 0ms"）、
ユーザーに `--test-threads=1` で再実行してもらったところ **666 件全て pass**
した。並列実行時のみ失敗する＝真のデータ競合であることが確定し、追補3の
「クロック不一致」という診断は誤りだったと判明した（クロック統一自体は
無害な改善だが、今回の失敗の真因ではなかった）。

失敗メッセージに `gji_monitor_ok`/`gji_last_io_ms` の実値を出す診断コミット
（`b8df49b8`）を追加して再実行してもらったところ
`gji_monitor_ok=true`（テストは直前に `store(false)` 済みのはず）と判明。
`current_tick_ms()` の単調性から、この分岐に入っていれば `elapsed` は
理論上 100ms 未満になり得ないため、`check_now()` が
`gji_monitor_ok=true` 側の即時 return 分岐（"warmup 後に GJI I/O が
来ていない→即解放"）を通っていたことが確定した。

`TSF_OBS.gji_monitor_ok` への書き込み箇所を crate 全体で再監査したところ、
**単一行 grep（`gji_monitor_ok.store(`）では検出できない複数行にまたがる
書き込みが2箇所**見つかった（`TSF_OBS\n    .gji_monitor_ok\n    .store(...)`
という改行を挟むフォーマット）:

- `tsf/warmup/chrome_probe.rs::chrome_probe_apply_vk_sent_reaches_inner_coro`
  （ロック一切なし）
- `tsf/warmup/probe_fsm.rs::ready_chrome_probe()`（ヘルパー関数、ロック
  一切なし）— これを呼ぶ7テストのうち3件
  （`chrome_gji_active_enters_per_vk_confirm_as_safety_net`、
  `chrome_without_gji_active_skips_literal_detect`、
  `chrome_per_vk_vk_sent_unset_does_not_backspace`）も無施錠だった
  （残り4件は既に `TSF_OBS_TEST_LOCK` を取得済みで無関係）。

計4箇所が `TSF_OBS_TEST_LOCK` を一切取得せずに `gji_monitor_ok=true` を
書き込み、書き込み後もリセットしていなかった。これが
`probe_fallback_waits_total_max_ms`（`gji_monitor_ok=false` を前提に
100ms のフォールバック待機を検証する）の実行中に競合すると、
`check_now()` が「モニター健全・warmup 後 I/O なし＝正常状態」の
即時 return 分岐に化けて `elapsed=0ms` 前後で返ってしまう。

**教訓:** `grep -n "field.method("` のような単一行パターンは、
`rustfmt` がメソッドチェーンを改行する（レシーバが長い・引数が多い等）
と検出漏れする。crate 全体の「この global を書き込む全箇所」監査では
`perl -0777`（複数行対応）や AST ベースの検索（`ast-grep` 等）を使うか、
少なくとも `field\s*\n?\s*\.\s*method` のような改行許容パターンを
併用すること。単一行 grep だけで「全箇所ロック済みを確認した」と
結論づけない。

**修正:** 上記4箇所すべてに `TSF_OBS_TEST_LOCK` の取得を追加した。

**関連:** BUG-65 本体・追補1〜3（同じ調査の一連の流れ）。

**2026-08-15 追補5（`cargo test`全体では緑化・`tests/e2e_windows.rs` で同種の穴を発見）:**
追補4の修正後、`cargo test -p awase-windows --lib` は実機で緑化した。続けて
`crates/awase-windows/tests/e2e_windows.rs`（実 Win32 ウィンドウ・実 GJI
IME に対する SendInput/SendMessage ベースの interactive E2E テスト、
`--lib` とは別の統合テストバイナリ）を実行したところ、2件が失敗した:

- `e2e_message_unicode_chars`: `win.clear()` 後に 'ア'/'イ' のみ送ったはずが
  `get_text()` が `"アイn"` を返した。
- `e2e_gji_kanji_conversion_interactive`: `namae` を GJI 経由で入力したのに
  composition が `"あまえ"`（先頭の `な` が `あ` に化ける、"n" が消失した
  典型的な cold-start literal パターン）になった。同テストの診断ログでは
  自分のウィンドウ生成直後に `Foreground match: false` が出ていた。

原因は BUG-65 本体・追補4と同型: このファイルには
`INTERACTIVE_TEST_LOCK`（「Phase 2-3 tests contest foreground focus when
run in parallel. This lock serializes them.」というコメント付きの専用
Mutex）が既に用意されていたが、**Phase 2 系テストの一部だけが実際には
取得していなかった**。`TestEditWindow::create()` は内部で
`force_foreground()`/`SetFocus()` を呼び、OS 全体で単一のグローバル状態
（foreground window・キーボードフォーカス）を書き換えるため、ロックを
取っていないテストが他の（ロックを取っている・取っていない問わず）
interactive テストと並行実行されると、フォーカスの奪い合いが起きる。
`e2e_gji_kanji_conversion_interactive` は正しくロックを取得していたが、
相手側の `e2e_message_unicode_chars`（未取得）が並行して
`force_foreground()` を呼べたため、ロックは片側だけでは無力だった
（BUG-65 追補4の `noop_for_test()`/`ready_chrome_probe()` と同じ
「意図された保護が一部の呼び出し元だけ抜けていた」パターン）。

同ファイルを全 `#[test]` 関数に対して機械的に監査（`TestEditWindow::create()`
または `force_foreground` を呼ぶが `INTERACTIVE_TEST_LOCK` を取得していない
関数を検出）したところ、`e2e_message_unicode_chars` の他に
`e2e_message_edit_control`・`e2e_message_special_keys`・
`e2e_message_long_text` の計3件も同様に取得漏れだった。計4件に
`INTERACTIVE_TEST_LOCK` の取得を追加した。

`e2e_gji_kanji_conversion_interactive` の "な→あ" 化については、GJI
composition ロジック自体を変更していない。フォーカス競合が解消されれば
再現しなくなる可能性が高いという仮説だが、実機での再検証が必要
（この追補の時点では未確認）。

**関連:** `crates/awase-windows/tests/e2e_windows.rs::INTERACTIVE_TEST_LOCK`。

---

## BUG-66: 全角ハイフンマイナス「－」が Chrome/Firefox 等（VK/TSF 送信経路）で長音「ー」に化ける（`build_symbol_to_vk` の VK_OEM_MINUS 二重登録、対応済み・実機未検証）

（このセッション時点で develop に既に別件の BUG-63「仮想デスクトップ切替後
Windows Terminal で～」が存在していたため、本エントリは rebase 時に
BUG-63→BUG-66 へ改番した。番号衝突は並行開発ブランチが同じ番号を独立採番
した結果であり、本文内容に変更はない。）

**症状（2026-08-09 ユーザー報告）:** 「`-`とか`ー`とか`―`とかの区別がイマイチ」
「`.yab`ファイルで長音記号が正しく処理できていないのでは」——`layout/nicola.yab`
上、無シフトの `-` キーには全角ハイフンマイナス「－」（U+FF0D、`nicola.yab:5`）、
左親指シフト+`X`キーには長音記号「ー」（U+30FC、`nicola.yab:14`）が割り当てら
れているが、実際の出力ではこの2キーが区別できず、「－」を打っても「ー」に
なる。全角ダッシュ「―」（U+2015）はそもそも `layout/nicola.yab`/`nicola_us.yab`
に定義が存在しない。

**原因（コード確認済み）:** Unicode 直接注入を受け付けないアプリ（Chrome/
Firefox/WezTerm/Teams WebView 等の `AppKind::TsfNative`、
`focus/class_names.rs:421-422`）向けに、`crates/awase-windows/src/vk.rs` の
`build_symbol_to_vk()`（`KeyInjector::resolve_char` 経由で
`output/vk_send.rs::send_char_as_tsf`/`send_char_as_vk` が使用）は「物理
ASCII キーを送信し IME 既定のローマ字かな変換に任せる」設計になっている。
ほとんどの記号（`、`/`,`・`。`/`.`・`「`/`[` 等）はこの設計で問題ない——
IME は既定でこれら半角記号入力を対応する全角記号へ自動変換するため、
全角側の文字を要求されたときに半角キーを送れば正しい全角記号が出る。

しかし `-`（VK_OEM_MINUS）だけはこの一般則の**例外**で、IME のローマ字かな
変換ルールが `-` を全角ハイフンではなく長音記号「ー」へ**専用変換**する
（`vk.rs` の既存コメント・テスト `vk_pair_to_ascii_covers_reported_symbols`
参照）。にもかかわらず `build_symbol_to_vk` には「ー」(0xBD, false) と
「－」(0xBD, false) の両方が登録されており（旧694行目）、「－」を要求しても
実際には「ー」が出力される。`nicola.yab` 側は無シフト `-` キー＝「－」・
左親指シフト+X＝「ー」と正しく物理的に区別しているにもかかわらず、出力側の
このテーブルが両者を同じ VK 送信に潰していたことが実害の直接原因。

**調査で判明した非該当ケース（誤って疑ったが実害なし）:** `build_symbol_to_vk`
には他にも同じ `(VkCode, shift)` に全角/半角の記号ペアが多数登録されている
（`、`/`,`、`。`/`.`、`「`/`[`、`｛`/`{` 等、計40組超）。しかしこれらは上記の
「半角キー送信→IME既定で全角変換」という一般則どおりに動作するため意図した
設計であり、`nicola.yab` は半角記号リテラルを一切出力しない（`nicola_us.yab`
含め確認済み）ため衝突による実害はない。テスト
`vk_pair_to_ascii_covers_every_build_symbol_to_vk_pair` のコメントにも
「値の重複は許容」と明記されている。長音「ー」だけが IME 側の特別変換規則の
例外に該当し、真に修正が必要だったのはこの1件のみ。

**対応:** `build_symbol_to_vk()` から「－」(U+FF0D) のエントリを削除。
`symbol_to_vk` に無い文字は `resolve_char` が `CharResolution::Unicode(ch)`
にフォールバックし、`KEYEVENTF_UNICODE` による直接注入で送られるため、IME の
ローマ字かな変換に依存せず正しく「－」が出力される。回帰テスト
`build_symbol_to_vk_does_not_collide_fullwidth_hyphen_with_choon`
（`crates/awase-windows/src/vk.rs`）を追加し、「ー」が引き続き VK_OEM_MINUS
経由であること・「－」が登録されていないことを固定した。全角ダッシュ「―」
（U+2015）は `layout/nicola.yab` に定義が無いため対象外（配列側で未定義）。

**検証:** `cargo test -p awase-windows`（lib 288件・その他golden/guard系
全て）全緑。**Windows 実機での動作確認は未実施**——Chrome で無シフト `-` キー
を押して「－」が出ること、左親指シフト+X で「ー」が出ることを実機で確認して
ほしい。

---

## BUG-67: Alt 押下中の合成 `VK_DBE_HIRAGANA` 注入で MS-IME が JIS かな直接入力へ切り替わる（`kp_restore_kana_from_half_width`、実機診断で確認・対応済み・実機未検証）

**症状（ユーザー報告）:** 「突然 JIS かなモードになる」——原因不明のまま
ローマ字入力が JIS かな直接入力へ切り替わってしまう報告がちらほらあった。
BUG-61/62 は物理 Alt+かな キー押下がこの切替を起こすことを既に確定させて
いたが、awase 自身がユーザーの物理操作なしに同じ切替を誘発している可能性が
未検証のまま残っていた。

**調査（2026-08-17、専用診断ツールによる実機検証）:** ユーザーの仮説
「awase 自身の合成 `VK_DBE_HIRAGANA` 送信が、たまたま Alt が押されている
タイミングと重なると同じ切替が起きるのではないか」を検証するため、
`crates/awase-windows/examples/alt_dbe_hiragana_probe.rs`（診断専用、
使い捨てツール）を作成した。classic Win32 EDIT コントロールを持つ自前
ウィンドウを起動時にフォーカスし、Alt キー押下の立ち上がりエッジで
`crate::tsf::output::make_scan_key_input` と同じ方式（scan code 付き、
`KEYEVENTF_SCANCODE` は使わない）で `VK_DBE_HIRAGANA` down+up を
SendInput、直後に "aiu" を実際に打鍵して `ImmGetCompositionStringW`
(GCS_COMPSTR) で変換結果を直接確認する構成にした。

`ImmGetConversionStatus` の ROMAN ビット読み取りだけでは実際の入力挙動と
食い違う場面があった（IME 実装依存でビット解釈が一様でないため）ため、
数値だけでなく実際の打鍵結果で判定したのが決め手になった。ある試行で
"aiu"（A・I・U の3キーのみ、N キーは一切送っていない）の変換結果が
「に」になった。ローマ字変換テーブル経由ではこの3キーからどう組み合わせ
ても「に」（ローマ字 "ni"）は導出できない——JIS かな配列の物理キー位置に
固定された直接対応（標準 JIS X 4064 配列で "I" キー位置＝「に」）でしか
説明がつかない。他の試行では3キーのうち一部が空になる（欠落する）結果も
観測され、Alt+F2 直後の入力方式切替処理中に IME 側がキーを取りこぼす
タイミング競合が起きていることを示唆した。これは「稀にしか起きない・
毎回同じ壊れ方をしない」というユーザー報告の性質とも一致する。

**原因（コード確認済み）:** `hook.rs` の既存 Alt+かなガード（物理
`VK_KANA`/`VK_DBE_ROMAN`/`VK_DBE_NOROMAN` 押下時に BUG-62 で追加した
swallow）は、自己注入キー（`is_self_injected`、`INJECTED_MARKER`/
`TSF_MARKER`/`IME_KANJI_MARKER` 付き）を判定より前に無条件で OS へ
通しているため、**awase 自身が送る `VK_DBE_HIRAGANA` はこのガードの対象外**
だった。

`VK_DBE_HIRAGANA` を合成送信している箇所を Opus に調査させたところ、
3箇所のうち実際に MS-IME で必要なのは `runtime/key_pipeline.rs`
`kp_restore_kana_from_half_width`（「IME-ON 半角英数」持続トグル解除時の
かな入力復元）の1箇所のみと判明した:

- `tsf/send.rs::send_vk_dbe_hiragana_pair`（F2 cold-start warmup）は
  `needs_f2_probe()` が MS-IME 戦略では既に `false` を返すため、MS-IME
  では元から送信されない（GJI 専用）。`ms_ime_ready_coro.rs` にも
  「F2 前置は不要（MS-IME は VK_DBE_HIRAGANA warmup を必要としない）」と
  明記済み。
- `ime.rs::send_f2_via_sendmessage`（`SendMessageTimeoutW` 版）は
  呼び出し元が実質ゼロ（`docs/adr/088-ime-axis-capability-and-charset-owner.md`
  §9-1 参照）の到達不能コードだったため、本対応で撤去した
  （`send_f2_via_sendmessage_async` も合わせて削除）。
- `kp_restore_kana_from_half_width` は BUG-15 追補3/4 の実機検証で、
  MS-IME (TSF-native) の「英数→かな」方向の復元が scan 付き
  `VK_DBE_HIRAGANA` 注入以外（IMC write・scan なし注入）では効かないと
  確定しており、`active_ime_kind == MicrosoftIme` の場合に限定して
  無条件に実行される。ここだけは削れない。

**修正:** `kp_restore_kana_from_half_width` の `VK_DBE_HIRAGANA` 注入直前で
`hook::win_key_held() || hook::alt_key_held()` を確認し、いずれか押下中は
注入をスキップする（保険の IMC write リトライだけが残る）。

- Win: `tsf/send.rs::send_vk_dbe_hiragana_pair` が既に持つ
  `win_key_held()` ガードと同じ理由・同じ判定関数で統一した（Win 押下中に
  送ると Win+F2 としてスタートメニューが開きうる、既知のリスク）。
- Alt: 本 BUG で確認した切替リスクへの対処。Shift のように synthetic な
  modifier-up を同一 SendInput バッチへ前置する案も検討したが、Alt/Win は
  単独タップで OS のメニュー系機能（`SC_KEYMENU`/スタートメニュー）を
  起動する特殊な扱いを受けており、実機未検証のままそれを回避する細工を
  追加するリスクを避け、検証済みの Win ガードと同じ「スキップ」方式に
  統一した。
- Ctrl はケアしていない。この VK に対する Ctrl 起因の既知の危険がコード
  上・過去のバグ報告上見当たらないため、実測なしの憶測でガードは追加
  しなかった（`.claude/rules/fix-requires-evidence.md`）。

**検証:** `cargo build`/`cargo test -p awase -p awase-windows -p awase-settings`
（lib 770件・その他golden/guard系全て）・`cargo clippy --lib`（CI `clippy`
job相当）・`cargo xwin check`/`cargo xwin clippy -p awase-windows`（CI
`windows-build` job相当）・`cargo fmt --check` すべて緑。**Windows 実機での
再検証は未実施**——`alt_dbe_hiragana_probe` で Alt を連打しても
`typed_comp_str` が「あいう」以外にならないことを確認してほしい。

**関連:** `crates/awase-windows/examples/alt_dbe_hiragana_probe.rs`（診断
ツール、使い捨て）。BUG-61（JIS かな直接入力は復旧不能）・BUG-62（物理
Alt+かな のガード導入）。

---

## BUG-68: `Blind` drift correction の give-up 後再武装が「鮮度」を「新情報」の代理指標として使うため、TsfNative で短周期に再武装し VK_IME_OFF を送り続ける

**症状（実機ログ、2026-08-17）:** Windows Terminal（`WindowsTerminal.exe`、
`CASCADIA_HOSTING_WINDOW_CLASS` → `Windows.UI.Input.InputSite.WindowClass`、
MS-IME、TsfNative）で Ctrl+無変換（既定の IME OFF コンボ）を押下し
`desired_open=false` を確定させた。`ime_on=false` の diag-ctx は以後一貫して
維持されていたが（＝ engine 自体はユーザーの意図通り OFF のまま）、約1.2秒間
（23:22:46.356〜47.559）で `[idle-conv-check] TsfNative: conv observation
open=true reason=NativeToggleShadowOff (conv=0x00000009) → ObserverReported
として記録` が最低2ラウンド発火し、そのたびに `drift_correction_blind` が
`VK_IME_OFF`（`0x1A`）を5回連打→`give up`→次の打鍵で新しい観測が生成され
即座に再武装、というサイクルを繰り返した。ユーザー報告のタイトルは「IME OFF
Engine ON が再発しました」（BUG-45 追補と同じ症状カテゴリの再発と認識）。
ビルドは develop 最新（`ed03a3c9`、PR #64まで反映、BUG-51 追補 v3
IntentStore 実装済み）だった。

**IME:** Microsoft IME。TsfNative プロファイル（Windows Terminal / InputSite）。
GJI/`Blacklist` プロファイルの `ir_apply_drift_correction` でも同型の
`conv` 誤読が起点になりうるが、本エントリは MS-IME 実機ログで確認した
経路（`kp_stage_idle_conv_check` → `classify_conv_transition` →
`ReportOpenInference(NativeToggleShadowOff)`）に絞って記録する。

**当初の誤診断（撤回）:** 最初は BUG-45 追補と同じ「`ConvOpenInference` が
`desired_open`/belief を汚染している」経路を疑ったが、ログを読むと
`desired_open` は一度も揺れておらず `ime_on=false` を保ち続けていた。
IntentStore（BUG-51 追補、develop へ 2026-08-16 マージ済み）が守る対象は
「壊れた観測が明示意図を上書きする」ことであり、本バグはその手前——**壊れた
観測そのものが繰り返し生成され、drift correction を無駄撃ちさせ続ける**
——ことが問題だった。

**検討したが撤回した修正案（Opus レビューで却下）:** 最初に実装したのは
`classify_conv_transition` に `explicit_off_intent: bool` を追加し、
`has_native && !effective_open` による `ReportOpenInference` 発火（BUG-26 が
「conv 不変でも steady-state で回復する」ために無条件化した分岐）を、明示
OFF 意図がある間は止める案だった。単体テスト・320→640通りの独立オラクル
全数一致・xwin check/clippy/dylint まで緑にした上で Opus に独立レビューさせた
ところ、**BUG-51 の検出経路を同一条件で殺す**という blocking な指摘を受け撤回した:
`ReportOpenInference` 分岐は `report_conv_open_inference`（観測記録）と
`schedule_ime_refresh(20)`（BUG-51 が追加した、TsfNative で恒久停止する
`TIMER_IME_REFRESH` の代わりのキック）を両方担っており、観測の生成自体を
止めるとこの両方が消える。BUG-51（Ctrl+無変換 後も実 IME が閉じないまま
最大8分放置された不具合）と BUG-68（本バグ、実 IME は正しく閉じたのに
持続する conv ビットを再オープンと誤読）は、乖離が続く間 conv が
`has_native && !effective_open` を満たし続けるという**同一の入力**になり、
BUG-63 が明記するとおり conv ビットだけでは両者を原理的に区別できない。
観測生成そのものを止める修正は、区別できないはずの2つのバグの一方だけを
「区別できた体で」黙らせてしまう誤りだった。

**原因（コード確認済み）:** 真因はもっと下流、`runtime/ime_refresh.rs`
`ir_apply_drift_correction` の **give-up 後再武装判定**にあった。
`Blind` が `max_attempts`（5）で `GiveUp` した後、`ObservationStore::
read_back(.., ReadBackQuery::AnyFreshEvidence, ..)` は「`gave_up_at` 以降に
新しい信頼できる観測が record されたか」だけを見て再武装する（値は問わない
——`observation_store.rs` のコメントに明記のとおり、bool の乖離では
「間違った値」は desired と異なる1通りしかなく、値ベースの判定はほぼ毎 tick
真になり無意味なため、意図的に「鮮度」を「外部で状況が動いた証拠」の代理
指標として採用した設計、BUG-43 追補）。

この代理指標は、**同一の乖離観測が短周期に再生成されるプロファイルを
想定していなかった**。MS-IME × TsfNative では:

1. `reschedule_ime_refresh`（`runtime/mod.rs`）は TsfNative では
   `read_ime_state_full` が常に `None` を返すため、通常の周期ポーリング
   （`TIMER_IME_REFRESH`）を**常に**停止する。再開経路はフォーカス変更・
   IME トグルキー・`ReportOpenInference` の `schedule_ime_refresh(20)`
   キックの3つのみ。
2. 通常の文字キー（本ログの き/う/w/i/n 等）は `FocusTracker::
   enrich_ime_relevance`（`runtime/focus_tracker.rs`）で `may_change_ime`
   にならないため、上記いずれの再開経路にも入らない。
3. `kp_stage_idle_conv_check` 自体は「毎打鍵」ではなく
   `should_run_idle_conv_check`（`src/engine/idle_check.rs`）のガード3
   ——`output_in_flight_ms()`（awase 自身が最後に出力を送ってからの経過
   ms）が `TYPING_IDLE_MS`（500ms）を超えた最初の KeyDown——を満たした
   ときだけ実行される（第1版のレビューで「毎打鍵」という誤った前提を
   Opus に指摘され訂正した）。IME/Engine が OFF の間、通常の文字キーは
   PassThrough で awase 自身の出力を伴わないためこのタイマーは経過し
   続けるが、**drift correction 自身の `VK_IME_OFF` 再送も出力として
   このタイマーをリセットする**。IMM32 の `NATIVE` ビットは変換モードの
   好みであり開閉状態と独立で、`VK_IME_OFF` で閉じてもクリアされない
   （本ログでも `conv=0x00000009` は Ctrl+無変換 の前後で一切変化して
   いない）ため、`classify_conv_transition` の BUG-26 回復分岐
   （`has_native && !effective_open`）が give-up バーストの直後の
   idle-conv-check でも `ReportOpenInference` を発火し、
   `kp_apply_conv_engine_sync` の `schedule_ime_refresh(20)` キックも
   同様に短周期で発火する。

結果、give-up の 20ms 後に見る「gave_up_at 以降の新しい観測」は、**まさに
そのキック自身が今しがた record したのと同じ情報の再掲**でしかないのに、
タイムスタンプが新しいというだけで「外部で状況が動いた証拠」として
即座に再武装 → `VK_IME_OFF` 再送 → 5回で再度 GiveUp → 直後の
idle-conv-check でまた再武装、という短周期ループ（実機ログでは概ね
数百ms〜1秒未満で1巡）になっていた。

**BUG-51 との関係（なぜ「鮮度」を単純に「値の変化」へ置き換えられないか）:**
BUG-51 のシナリオ（実 IME が本当に開いたまま）でも、乖離が続く間 conv は
BUG-68 と同じ値を返し続ける。「値が変わったか」を再武装条件にすると
BUG-51・BUG-68 のどちらでも一度も再武装しなくなり、これは「一度諦めたら
`desired` が変わるまで永久に再送しない」という ADR-080 当初案そのもの
（BUG-51 が「8分放置」で問題視した硬直）に逆戻りする。conv ビットに
両者を区別する情報が無い以上（BUG-63）、値ベースの判定でこの2つを両立
させることはできない。

**修正:** give-up 後の再武装判定に**最小クールダウン**を追加した
（`DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS` = 3秒、`tuning.rs`）。
`gave_up_at` からこの時間が経過するまでは `read_back` 自体を評価せず
（＝再武装しない）、経過後は従来どおり `AnyFreshEvidence` の判定
（鮮度ベース、変更なし）を行う。判定本体は Linux でテスト可能な純粋関数
`state/ime_actuation.rs::blind_rearm_cooldown_elapsed(gave_up_at, now,
cooldown_ms)` に切り出した（`decide_actuation_action` と同じ「runtime 層は
Linux で実行できないため核心ロジックだけ state 層に置く」パターン）。

この設計は BUG-26/BUG-51 の既存経路を一切変更しない——観測は従来どおりの
頻度で記録され続け（belief の自己修復は無傷）、`schedule_ime_refresh(20)`
キックも従来どおり発火し続ける（BUG-51 が必要とした「死んだタイマーを
起こす」役割は無傷）。変わるのは「give-up 直後に即座に再武装するか、
クールダウンを空けるか」だけであり、BUG-51 のシナリオでも、クールダウン
経過後に到来する次のキックで回復チャンスが巡ってくる（「二度と再送しない」
への逆戻りではない）。3秒という値は実測ではなく、タイピング中に体感できる
差を作るためのレート制限ポリシーであることを `tuning.rs` のコメントに
明記した（`.claude/rules/tuning-constants.md` が要求する「待つべき対象」
自体が測れる事象ではないため）。

**既知の限界（2巡目の Opus レビューで指摘、未対処）:**

- **フォーカス変更によるクールダウン無効化**: `ImeEvent::FocusChanged` は
  `Actuation`（`gave_up_at` 含む）を丸ごと破棄する既存仕様
  （`runtime/ime_actuation.rs` 破棄条件2）のため、クールダウン中に対象を
  跨ぐフォーカス変更（プロセスを跨ぐ場合のみ発火——BUG-57 の通知ポップアップ
  等）が起きると、新しい `Actuation` が即座に最大5回まで送信できる状態から
  再開する。ただしこれは連続した無限ループの再発ではなく、フォーカス変更の
  たびに高々5回という有界な事象に留まる。
- **`FeedbackPolicy::Blind::backoff` が死んでいる**: `state/ime_actuation.rs`
  の `backoff` フィールド（`AppImePolicy::from_profile` が 400ms を設定）は
  構築されるだけで `ir_apply_drift_correction` から一度も読まれていない
  （2巡目レビューで発見）。つまり give-up バースト**内**の最大5回の送信
  自体は無間隔のままで、本クールダウンが効くのはバースト**間**のみ。
  `state/app_ime_policy.rs` のコメントは「5 × backoff で最悪 ~2秒」と
  backoff が効いている前提で `max_attempts=5` を正当化しているが、これは
  実態と異なる。バースト内隔の是正は本 BUG のスコープ外として別途扱う。

**テスト:** `state/ime_actuation.rs` に `blind_rearm_cooldown_elapsed`
の単体テスト6件を追加（give-up 直後は不許可・境界の1ms手前は不許可・
境界ちょうどは許可・大幅超過後は許可・cooldown=0は常に許可・時刻巻き戻り
は安全側、`decide_actuation_action` と同じ形式）。`cargo test -p
awase-windows --lib`（393件）・`--test journal_replay`（1件）・
`--test architecture_guard`（32件）・`--test golden_scenarios`（22件）・
`--test layer_boundary_guard`（8件）、全緑。`cargo xwin check --tests
--target x86_64-pc-windows-msvc -p awase-windows`・`cargo xwin clippy
-p awase-windows --target x86_64-pc-windows-msvc -- -D warnings`・
`cargo dylint --all -p awase-windows -- --target x86_64-pc-windows-msvc`
警告ゼロ。**`ir_apply_drift_correction` 本体は `runtime/` 配下
（`#[cfg(windows)]`）のため Linux ではクロスコンパイル型検査のみで
実行テスト不可**（`decide_actuation_action` の呼び出し側と同じ既存の
制約であり、`bug43_tight_loop_is_bounded_not_infinite` のような
journal リプレイ回帰への昇格は今回見送った——本クールダウンの核心
ロジック自体は既に純粋関数として全境界値をテスト済みで、リプレイ
基盤の追加投資は別 BUG/タスクとして再検討する）。**Windows 実機での
確認は未実施**——次回、Ctrl+無変換 で MS-IME を閉じた直後に Windows
Terminal でタイピングを続け、`drift_correction_blind` の連続発火が
3 秒間隔以上に収まること、`ime_on=false` が乱れないことを確認すること。

**関連:** BUG-19（観測が `desired_open` を直接書き換えない設計の由来）、
BUG-26（本バグで問題になった無条件回復分岐そのものの導入）、BUG-43
（GJI/Blacklist 側の同型 drift correction 無限再送、`ir_apply_
drift_correction` の別経路、「鮮度を新情報の代理指標にする」設計の
初出）、BUG-45 追補（同じ「IME OFF Engine ON」症状タイトルでの過去
インシデント、IntentStore による `desired_open` 保護——本バグはその
手前の再武装レートの問題）、BUG-51（`schedule_ime_refresh(20)` 明示
キックの導入経緯、本バグの修正が壊してはならない経路）、BUG-63
（conv ビットが open/close を原理的に区別できないことの確定）、
[ime-belief-architecture](../.claude/rules/ime-belief-architecture.md)、
[fix-requires-evidence](../.claude/rules/fix-requires-evidence.md)、
[tuning-constants](../.claude/rules/tuning-constants.md)。

### 追補2（2026-08-24、実機ログで再武装ループの実測値を取得。コード変更は未実施）

タスクトレイ不具合報告 `report_id: 01M0S4S6R4C1YJ581YJ9ZGAXXD`（BUG-75 と同一
report。BUG-75 の文字化けとは無関係と判明した経緯は後述）の journal を精査し、
本バグ（give-up 後の再武装が「鮮度」を「新情報」の代理に使う問題）の実機発生を
確認した。3ラウンドの Opus 設計・premortem 対話（設計担当 `opus-designer-
bug68` / レビュー担当 `opus-premortem-bug68`）を経て、以下の実測・設計判断が
出ている。**今回はコード変更を行わず、分析結果のみを記録する**（別ブランチ
`fix/bug68-blind-typing-gate` で後日実装）。

**実測:** Ctrl+無変換（`ChordEnded{CtrlMuhenkanImeOff}`）による明示 OFF 直後
から `VK_IME_OFF` 再送のバーストを観測した。journal に**残存している**のは
**9周・45回**（elapsed 21110202〜21169911 の連続エピソード、約59.7秒。
`ImeOpenApplied{reason: DriftCorrection}` で確認）。「約63秒・14周・計約70回」
という当初の見積りは、`DumpTruncated`（`dropped_actuation: 364`）による
17032ms/12188ms の欠落窓を平均バースト間隔で補間した推定値であり、後日この
journal を読み直す場合は9周分のみ残っていることに注意（Opus premortem
指摘）。バースト内間隔は実測 **30〜63ms**（`on_ime_apply_complete` →
`post_ime_refresh(20ms)` の自走。`FeedbackPolicy::Blind::backoff`(400ms) は
未使用のまま）、バースト間隔は実測 **3.7〜7.2秒**（`DRIFT_CORRECTION_
BLIND_REARM_COOLDOWN_MS`=3000 が効いている、いずれも journal に残る9周から
直接確認済み）。再武装トリガは全周とも `ObserverReported{source:
ConvOpenInference, open: true, confidence: Medium}` ＝ give-up の原因になった
のと同一の (source, value) だった。

**当初の誤診断（撤回）:** このバースト（Windows Terminal / TsfNative）を、
BUG-75 の文字化け（「つかって」→「っつかって」、msedge / Imm32Unavailable）の
原因だと最初は判断し、`Actuation::last_attempt_at` を追加して burst 内送信
間隔に `FeedbackPolicy::Blind::backoff`（400ms）を適用する修正（コミット
`2b80b0a9`、既に revert 済み・未 push）を一度実装した。しかし Opus レビューで
以下が判明し**却下・破棄**した:

- TsfNative では `reschedule_ime_refresh`（`runtime/mod.rs`）が周期ポーリング
  を早期 return で止めるため、burst 内送信の自走は `post_ime_refresh` の
  20ms タイマーだけに依存している。backoff を入れると2回目以降の送信が
  **打鍵駆動の tick（`kp_apply_conv_engine_sync` の `ReportOpenInference` →
  `schedule_ime_refresh(20)`）に同期して再開する**ようになり、「composition
  中の実タイピングと衝突する」という当初の仮説の下では**衝突確率を下げる
  どころか悪化させる**方向に働く。
- 報告症状（「っつかって」の先頭1文字混入）は burst の**1発目**
  （`attempts==0`）1回で説明がつくが、backoff は `attempts==0` を意図的に
  免除しており、最も疑わしい送信には一切効かない。
- 実際に journal のタイムスタンプを突き合わせたところ、burst（Windows
  Terminal、elapsed_ms 21127435-21172071）と BUG-75 の `StaleConfirm`
  （msedge、elapsed_ms 21285883）は**116秒離れており、別プロセス**だった。
  **文字化けとの因果は否定された。**

**診断可能性への実害（未対処）:** `ir_apply_drift_correction`
（`runtime/ime_refresh.rs`）は park 中の全 tick で `ImeActuation{action:
GiveUp}` を無条件に record する（cooldown 判定より前に record している）。
今回の report の journal dump は `dropped_actuation: 364, dropped_key_input:
418` を示しており、**このスパムが無関係な別不具合（BUG-75）の打鍵列を
journal から追い出し**、BUG-75 の原因特定を journal だけでなく app log の
突き合わせに頼らざるを得なくした。park 継続中は record しない（`gave_up_at`
を刻む tick と、実際に再武装/送信する tick のみ記録する）よう修正すべき。

**設計案（次回実装時の起点、premortem 済み）:**

- 再武装条件を「鮮度」ではなく「打鍵の谷」に変える。`gate.
  last_hook_activity_ms`（**物理キーのみ**、`hook.rs` が self-injected キーを
  `CallNextHookEx` で早期 return するため自己ラッチしない——`ir_decide_
  read_strategy` の `is_typing` が `OUTPUT_GATE.last_vk_output_ms` と `max` を
  取った**合成値**は自己ラッチするので、それとは別に物理キー単独の idle_ms を
  使うこと）が `TYPING_IDLE_MS`（500ms、新規定数追加なし）未満なら
  `HoldFor { retry_ms }` を返す純関数 `blind_send_gate(idle_ms, typing_idle_ms)
  -> { Send, HoldFor { retry_ms } }` を state 層に置く。`decide_actuation_
  action` のシグネチャは変更しない（`DriftCorrectionFixture`/
  `tests/journals/drift_correction/bug-43-drift-correction-tight-loop.json`
  との往復を壊さないため）。
- `HoldFor` のとき `schedule_ime_refresh(retry_ms)` で明示的にタイマーを張る
  （打鍵駆動 tick に便乗するのではなく、打鍵が止まる時刻を狙って自前で
  張る点が、却下した backoff 案との決定的な違い）。
- **未解決の Must-fix（実装時に対応必須）**: `TIMER_IME_REFRESH` は
  グローバル単一タイマーで、`runtime/key_pipeline.rs:636`（`kp_apply_conv_
  engine_sync` の `SetOpen`/`DirectInput` 分岐）・`key_pipeline.rs:976`（IME
  ON/OFF コンボ処理）が打鍵駆動で kill する可能性があり、`HoldFor` が
  張ったタイマーが再武装されずに「二度と送らない」へ縮退するリスクがある。
  タイマー依存を避け、`Actuation` に `hold_until_ms` を持たせて**次に来た
  任意の tick**で判定する設計（タイマーは「なるべく早く起こす」ヒントに
  格下げ）のほうが構造的に安全、というのが premortem の結論。
- longstop（時間経過での強制再武装）案・同一 (source,value) を Hold にする
  再武装条件変更案はいずれも**却下**: BUG-51 の真因は「tick が来ないこと」
  自体であり、時間ベースの緩和は TsfNative では原理的に無意味（周期
  ポーリングが存在しないため「時間が経過した」ことを検知するタイマー自体が
  同じ問題を抱える）。

**関連:** BUG-75（当初この burst が原因と誤診断したが無関係と判明した文字化け
本体）、BUG-51（`schedule_ime_refresh(20)` キックの導入経緯）、
[bug-report-fetch skill](../.claude/skills/bug-report-fetch/SKILL.md)。

---

## BUG-69: `ir_post_focus_change_snapshot` が belief を `applied=Confirmed` へ偽装し、TsfNative の force-on / BUG-16 修正を無効化。eager warmup だけが未監査の副作用で偶然穴を塞いでいる

**2026-08-21 追記: 本文中「実装は一切行っていない」等の記述は発見時点（調査のみ）のもの。
実装は完了済み。詳細は本エントリ末尾の「実装済み（2026-08-21 追記）」節を参照。**

**発見の経緯:** BUG-34 横展開（追補4、eisu ガード撤去）の直後、ユーザーから
「send_eager_warmup も GJI なら要らないのでは」という疑問が出たのを機に、
eager warmup・drift correction・TsfNative force-on ブロックの3機構の
相互作用を Opus premortem レビューで監査した。発見当初は実装を一切行わず、
本エントリはレビュー結果の記録のみだった（F2 の修正案を別途 premortem
レビューしてから着手する方針だった）。

**症状（未確認・実機ログなし、コード読解で構築した想定シナリオ）:**
WezTerm または Windows Terminal（TsfNative プロファイル）+ Google 日本語
入力（GJI）で、`Win+Ctrl+→` 等の仮想デスクトップ切替（Win キー押下中は
IME キー注入がスキップされるため、実 GJI が閉じたまま belief だけ ON で
残留しうる）の後、`Win+Ctrl+←` で当該ウィンドウへ復帰し直後に打鍵すると:
- 最初の1文字がリテラル ASCII になる（`これで→korede`、BUG-16 と同じ
  見え方）、または
- eager warmup の `VK_DBE_HIRAGANA`（scan=0x70、物理かなキー位置）が
  閉じた IME 文脈に着弾し、kbd106 のかなロックを誤トグルして JIS かな
  入力に固着する（BUG-08/BUG-55 と同根のハザード）。

**原因（コード読解で確定、実機ログでの再現は未実施）:**

1. **F1: TsfNative force-on ブロックは到達不能。** `ir_post_focus_change_snapshot`
   は `focus.focus_changed`（= `process_changed`）のときにしか呼ばれず
   （`ime_refresh.rs:193-195`）、その `on_focus_process_changed` が同じ
   tick 内で既に `input_barrier = FocusTransition{settle_until: now +
   focus_settle_ms}` を armed 済み（`ime_model.rs:517-522`、TsfNative は
   200ms）。Stage1→Stage3 間の実処理はサブミリ秒（`skip_imm_query=true`
   経路では `ImeDiagnosticSnapshot::capture()` すら `if !skip_imm_query`
   でスキップされる）。ゆえに `applied_ime_on && new_profile_is_tsf_native
   && !self.ime_apply_should_defer()` は**常に false**。兄弟ブロック
   （drift correction 等）と違い `schedule_settle_retry` も無いため、
   一度も再試行されない。ログ文字列 `"TsfNative IME ON → GJI VK_IME_ON
   強制"` は `docs/` 配下のどの実機ログにも一度も出現しない。
2. **F2: `mirror_applied_open` が belief を `applied=Confirmed` へ偽装する。**
   `ir_post_focus_change_snapshot` 冒頭（`ime_refresh.rs:429-433`）が
   全プロファイルで無条件に
   `self.platform_state.ime.mirror_applied_open(ime_on_now, tick_ms)`
   を呼ぶ。`tick_ms`（`GetTickCount64`）は常に非ゼロなので
   `mirror_applied_open_with_ts` の規約上これは
   `AppliedImeState::Confirmed{open: belief}` を**実際には何も apply
   していないのに**確定させる。これは `focus_tracking.rs:398-402` が
   TsfNative を hard pre-sync から明示的に除外している理由
   （「TsfNative は SSOT model: applied=Unknown のまま維持し、最初の
   キーで SetOpen を発行する」）に真っ向から反する——Stage1 が意図的に
   `Unknown` のまま残した `applied` を、Stage3 が数百マイクロ秒後に
   上書きする。
   - この偽装が `GjiDirectStrategy::apply`（`ime_controller.rs:109-113`）
     の `if open && view.control.shadow_on { return AlreadyMatched; }`
     を誤発火させ、F1 の force-on ブロックはまさにこれを打ち消す
     ためのワークアラウンドとして書かれていた（コメント参照）——
     つまり F1 は F2 の症状に対する対症療法であり、その対症療法自体が
     到達不能になっている。
   - `apply_force_on_for_imm_broken`（BUG-16 の修正）のスパムガード
     （`runtime/mod.rs:704-710`）が `Confirmed{open:true}` で早期
     return するため、TsfNative では**事実上恒久的に不発**になる。
     このガードの正当化コメント「FocusChange が applied=Unknown に
     リセットするため、フォーカスごとに1回だけ force-apply される」
     は F2 により成立しない。BUG-16 追補2 が「TsfNative では引き続き
     発火するはず」と想定していた前提が崩れている。
   - `apply_force_on_for_imm_broken` が defer 時に積む
     `schedule_settle_retry`（~250ms後）も無意味——再試行 tick でも
     `applied` は F2 が書いた偽の `Confirmed{true}` のままなので、
     再びスパムガードで早期 return する。これは BUG-16 が修正した
     はずの「settle 明け再試行が『何もしない関数』の再試行だった」と
     全く同じ失敗パターンが、別経路で再現している。
3. **F3: 結果として、TsfNative+GJI のフォーカス復帰時に実際に発火する
   IME actuation は eager warmup（`send_eager_tsf_warmup`）だけになる。**
   `ir_stage_notify` 内の他の actuation（force-on、
   `apply_force_on_for_imm_broken`、drift correction）は全て
   `ime_apply_should_defer()`（focus settle barrier）でゲートされて
   いるが、eager warmup だけがこのチェックを経由せず単独で通過する。
   さらに `reschedule_ime_refresh`（`runtime/mod.rs:604-609`）は
   TsfNative で周期リフレッシュ自体を恒久停止するため（コメント
   「周期リフレッシュに乗るのが唯一の force-ON 経路になった」）、
   force-ON には他のトリガーも無い。
4. **F4: eager warmup 自身が「開く」副作用を持つ、監査されていない
   force-open 機構になっている。** `send_vk_dbe_hiragana_pair` →
   `make_tsf_key_input` は `wScan = MapVirtualKeyW(0xF2,
   MAPVK_VK_TO_VSC)` = 0x70（物理かなキー位置）付きで送信する
   （scan=0 ではない）。`ime_controller.rs:143-149` のコメントは
   `VK_DBE_HIRAGANA` が「開く」と「ひらがなに強制する」を1つの副作用に
   束ねていることを明記しており（BUG-50 デッドロックの直接の前提。
   MS-IME の ON キーはこれを理由に 2026-08-06 に他キーへ移行済み）、
   BUG-15 追補7 は「**IME モードキーの注入は実 IME が確実に ON でない
   限りしてはならない**」と、まさにこの scan 付き `VK_DBE_HIRAGANA`
   注入の危険性を名指しで警告している（`kp_restore_kana_from_
   half_width` は `effective_open()==false` のときこの注入を
   スキップし IMC write のみに留める設計——BUG-34 追補4 で削除した
   eisu ガード撤去とは無関係に、この安全則自体は他所で現に守られて
   いる）。しかし `can_warmup()`（`tsf_gate.rs:348-350`）のゲート
   `ime_on` は `applied_ime_on`——F2 により実質 `effective_open()`
   の再ラベルに過ぎず、real/observed state を一切参照しない。
   つまり eager warmup は「belief 上 ON」であることしか確認せずに、
   このリポジトリが他所で明示的に禁止しているのと同じ危険な注入を
   行っている。

**波及（未確認、PLAUSIBLE）:** eager warmup が書く NATIVE conv bit は
`report_conv_open_inference` の `NativeToggleShadowOff` 経路を通じて
`ConvOpenInference(open=true)` として観測されうる。`check_drift_
correction` は `trusted.open == desired` で早期 return するため
（`platform_state.rs:760-762`）、belief=ON・実 GJI=OFF の乖離があっても
warmup が書いた conv bit が「乖離なし」という偽の証拠を作り、drift
correction 自身を沈黙させる可能性がある——ただし BUG-68 の実機ログは
MS-IME（`needs_f2_probe()==false` で warmup 自体が no-op、`warmup_
strategy.rs:133-135`）であり、GJI × TsfNative でのこの経路は実機での
確認が無い。

**このリポジトリの既知バグ全体を貫く根本パターンとの一致:** BUG-19
（misread な conv が warmup で実体化しロックインされる）、BUG-33
（belief を観測として書き戻す循環）、BUG-48（対称 SetOpen echo が
ユーザー意図を上書き）、BUG-68（補正の出力が自分の再武装条件を生成）
と同型の「**belief が evidence として再流入する**」欠陥の6例目。
`ime-belief-architecture.md` の禁止パターン2（観測の偽装）は observer
層を対象にしていたが、`mirror_applied_open` は actuation 完了の記録
という**別の層**で同じ違反を犯している点が新しい。

**現時点での評決（Opus premortem レビュー、実装なし）:**

| 機構 | 評決 |
|---|---|
| eager warmup | KEEP、ただし SIMPLIFY（F2/F1 を先に直してからゲート強化。単独でゲートを足すと F3 が露見し即座に regression する） |
| drift correction | KEEP AS-IS（唯一生きている機構。BUG-68 の cooldown は適切、`Blind::backoff` 未消費と `FocusChanged` が `gave_up_at` を破棄する点は軽微な既知の残課題） |
| TsfNative force-on ブロック | REMOVE。ただし F2（`mirror_applied_open`）の修正が必須の代替——ブロックだけ消すと F2 は残ったまま force-on の唯一のワークアラウンドが消えるだけになる |
| enforce-OFF ブロック（同一関数、対象外だったが同じ穴あり） | SIMPLIFY。`ime_apply_should_defer()` チェックが無く、`ime_apply_should_defer` 自身の doc コメントが呼び出し元としてこのブロックを名指ししているにも関わらず未実装 |

**推奨する修正順序:** F2（`mirror_applied_open` を TsfNative で呼ばない
か `Optimistic` に留める）→ F1（force-on ブロック削除、F2 修正後は
不要になるはず）→ warmup のゲート強化。この順序を守らないと、
どの1つを単独で直しても他の2つの死んだ機構が露見して regression する
（BUG-34 追補4 の3ラウンド premortem と同じ「単独修正が別の未監査の
穴を露出させる」構造）。

**未実施:** 実装そのもの・実機再現・回帰テスト・修正案自体の premortem
レビュー。次回、F2 の修正案を単独で設計し、これまでと同様 Opus に
念入りな premortem レビューをかけてから着手する。

**関連ファイル:** `crates/awase-windows/src/runtime/ime_refresh.rs`
（`ir_post_focus_change_snapshot`、`ir_apply_drift_correction`）、
`crates/awase-windows/src/runtime/mod.rs`（`apply_force_on_for_imm_broken`、
`reschedule_ime_refresh`、`ime_apply_should_defer`）、
`crates/awase-windows/src/output/mod.rs`（`send_eager_tsf_warmup`、
`tsf_readiness`）、`crates/awase-windows/src/tsf/tsf_gate.rs`
（`TsfReadiness::can_warmup`）、`crates/awase-windows/src/tsf/send.rs`
（`send_vk_dbe_hiragana_pair`）、`crates/awase-windows/src/ime_controller.rs`
（`GjiDirectStrategy::apply`）、`crates/awase-windows/src/runtime/
focus_tracking.rs`（TsfNative hard pre-sync 除外）。

**関連:** BUG-16（`apply_force_on_for_imm_broken` の settle 明け再試行
無効化パターンの初出、本バグは同じ失敗が別経路で再現）、BUG-08/BUG-55
（scan 付き IME モードキー注入が JIS かなロックを誤トグルするハザード
の原型）、BUG-15 追補7（IME モードキー注入は実 IME 確実 ON 時のみ、
という安全則の初出）、BUG-19（belief 実体化ロックインの原型）、BUG-33
（belief を観測として書き戻す循環の原型）、BUG-48（対称 echo が
ユーザー意図を上書きする同型パターン）、BUG-50（`VK_DBE_HIRAGANA` の
open+conv 束縛副作用、MS-IME 側は既に移行済み）、BUG-68（自己再武装
フィードバックループ、conv ビットの open/close 原理的区別不能性）、
BUG-34 追補4（この調査の発端、eisu ガード撤去）、
[ime-belief-architecture](../.claude/rules/ime-belief-architecture.md)、
[fix-requires-evidence](../.claude/rules/fix-requires-evidence.md)。

**実装済み（2026-08-21 追記）:** 上記「推奨する修正順序」に沿って
ADR-098（[docs/adr/098-tsfnative-applied-confirmed-laundering-and-force-on-removal.md](adr/098-tsfnative-applied-confirmed-laundering-and-force-on-removal.md)）
の決定0・1-a・1-b・1-c・2・4・6-a・6-b・6-c を実装した。

- **F2 対策（決定1-a/6-a）**: `mirror_applied_open`/`mirror_applied_open_with_ts`
  （`ts==0` センチネルで `Optimistic`/`Confirmed` を切り替えていた設計）を撤去し、
  `record_optimistic(open)`/`record_confirmed(open, at_ms)` に分離
  （`state/platform_state.rs`）。`ir_post_focus_change_snapshot` は
  TsfNative では `record_confirmed` を呼ばなくなった（`applied` は
  `Unknown` のまま維持され、Stage1 の意図を Stage3 が上書きしない）。
- **F1 対策（決定2）**: 到達不能だった TsfNative force-on ブロックを
  `ir_post_focus_change_snapshot` から完全削除。ワークアラウンドではなく
  F2 の根治で不要になった。
- **F3/BUG-16 スパムガード対策（決定1-c）**: `apply_force_on_for_imm_broken`
  のスパムガードを、偽装された `Confirmed{open:true}` に依存する早期
  return から、`force_on_attempt_allowed`（`state/ime_actuation.rs`、
  新設 `ForceOnRetryState`・`FORCE_ON_RETRY_COOLDOWN_MS=3000ms`）による
  cooldown ベースの再試行許可判定に置き換え。`FocusChanged` で
  `force_on_retry` もリセットされる。
- **F4 対策（決定1-b）**: eager warmup の `ime_on` 入力を、生の belief/
  `Option<bool>::unwrap_or(false)` ではなく `WarmupImeOn` 型
  （`src/platform.rs`、3種のプライベートコンストラクタのみ）経由の
  `resolve_warmup_ime_on`/`warmup_ime_on()`（`applied ?? belief` を
  1箇所に集約）に統一。`TsfComposition::on_passthrough_key`/
  `on_reinject_key` のシグネチャも `WarmupImeOn` を受け取るよう変更。
- **enforce-OFF ブロック（決定4）**: 意図的に `ime_apply_should_defer()`
  を経由させない設計として doc comment で明文化（追加の barrier は
  導入しない）。
- **決定6-b**: `architecture_guard.rs` に
  `applied_state_recorders_call_sites_are_accounted_for` を新設し、
  `record_optimistic`/`record_confirmed` の呼び出し箇所数
  （1箇所・5箇所）を固定してレビュー漏れを機械検知。
- **決定6-c**: `AppliedImeState::applied_open()` の doc comment を
  「証拠用アクセサ」として書き換え、実利用箇所（3箇所）と
  情報用途は `warmup_ime_on()` を使うべきという指針を明記。
- **検証**: `cargo xwin check/build --tests/clippy --lib -D warnings` は
  全てクリーン。`cargo test -p awase-windows --lib`（427件）、
  `architecture_guard`（36件）、`golden_scenarios`（22件）、
  `drift_correction_replay`（2件）、`intent_store_effective_open`（8件）、
  `journal_replay`（1件）、`layer_boundary_guard`（8件）、いずれも
  全件成功。

**Windows 実機での初回検証（2026-08-22 追記、dragonflyg4、Windows Terminal
/ `CASCADIA_HOSTING_WINDOW_CLASS` / GJI）**: `RUST_LOG=debug` で通常ビルドを
起動し、IME を明示的に OFF にした状態から他ウィンドウへ切り替え → 20〜30秒
放置 → Windows Terminal へフォーカスを戻し、**物理キーに一切触れず**さらに
数秒待機、という手順を実施。ログ上で以下を確認した:

1. フォーカス変更直後、`[focus-settle] apply_force_on_for_imm_broken skipped
   (settling) → 550ms 後に refresh で再試行` が発火（決定1-a/1-c の settle
   ゲートが機能している証拠）。
2. 約550ms後、`[apply-ime] GJI direct: send 0x0016 (open=true)` →
   `[apply-ime] open=true eff=true conf=true → outcome=Applied` →
   **`force-ON (ImmBrokenForceOn): apply_ime_open(true) → Applied`** が発火。
   直前の物理 F2 キー押下（`injected=false`）は88秒以上前で、この force-ON
   とは無関係であることをタイムスタンプで確認済み（同種のイベントが2回
   連続で観測され、2回目の直前にあった物理キー押下も `no-op:
   effective_open は既に true → apply-ime 見送り` で実送信をトリガーして
   いないことを確認した）。

**これは BUG-69 の核心（`apply_force_on_for_imm_broken` が TsfNative で
恒久的に無効化されていた）が、実際に解消されていることを示す最初の実機
証拠である。** ただしこれは1セッションでの限定的な確認であり、`docs/
known-bugs.md`/ADR-098 が要求する「ソーク」（長時間・多様なシナリオでの
継続検証）はまだ行っていない。

- **未実施のまま残るもの**: 決定3-c（GJI warmup キーの再選定実験、
  `VK_IME_ON` vs `VK_DBE_HIRAGANA`、[ADR-100](../adr/100-gji-warmup-vk-ime-on-reinit.md)
  が引き取り実機データ収集中）、決定4-b（enforce-OFF を settle-retry 化
  する代替設計）、および `focus_tracking.rs`/`key_pipeline.rs` に残る
  軽微な belief-laundering 箇所（コメント修正のみで動作は維持、ADR-098
  決定5参照）。長時間ソーク・他アプリ（Windows Terminal 以外の TsfNative
  クラス）・実IMEが本当にOFFのまま長時間放置されるケースでの検証も未実施。

---

## BUG-70: GJI 候補確定タイミングで eager warmup（`ConfirmKeyUp`）が GJI の `EndComposition` と競合し、`@` がリテラルとして漏れる

**症状（実機ログ、2026-08-22、Windows Terminal / PowerShell、GJI、
`CASCADIA_HOSTING_WINDOW_CLASS` → `Windows.UI.Input.InputSite.WindowClass`）:**
候補文字列を確定する（Enter で確定キーを押す）たびに、確定した文字列の
直後に `@` が1文字リテラルとして出力される。ユーザー確認により再現は
「候補確定のタイミングで毎回」。IME の状態自体（`ime_on=true`、
`input_mode=ObservedRomaji`）は乱れておらず、awase 自身のログにも
`Char('@')` 等の意図した出力は一切現れない（awase から見れば正常に
"き"/"う"/"い" 等をTSF経由で送信しているだけであり、`@` はこの送信の
成否を awase が観測できない領域で漏れている）。

070fe973（本ブランチ、eager warmup の送信 VK を `VK_DBE_HIRAGANA` から
`VK_IME_ON` へ変更、BUG-50追補2）の適用前は同じ経路でより高頻度に `@`
が発生していたとユーザーから報告があり、070fe973 後も頻度が下がった
だけで解消はしていなかった。

**原因（コード確認、実機での確定的な検証はまだ、ユーザー報告との強い
時間的相関から推定）:**

`Output::send_eager_tsf_warmup`（確定キー・Ctrl 解放・フォーカス変更等
のたびに TSF cold-start 対策として先回りで warmup VK を送る仕組み）の
うち、`CompositionFsm::on_event` の `ConfirmKeyUp` 分岐
（`tsf/composition_fsm.rs`）は、確定キー押下時点で warm だった場合でも
KeyUp のタイミングで**無条件に** `EmitWarmup { reason:
WarmupReason::ConfirmKeyUp }` を発行していた。

一方、GJI 自身の候補確定・composition 終了（`EndComposition`、候補
ウィンドウ HIDE）は Windows のアクセシビリティ通知（`win_event_obs`）
経由で非同期に届く。実機ログでは毎回、`[composition-fsm] EmitWarmup
(ConfirmKeyUp)` → 合成 `VK_IME_ON`（scan は `MapVirtualKeyW(VK_IME_ON,
MAPVK_VK_TO_VSC)` で算出、実機確認値 `0xF2`）注入 が、GJI 側の
`[gji-fsm] EndComposition (candidate HIDE)` より**先に**発火している
（後者は非同期通知のため到着が遅れる）。この GJI がまだ確定処理中の
ウィンドウに warmup キーが着弾すると GJI/TSF がこれを正しく消費できず
生キーとして Windows Terminal 側に漏れ、この JIS 配列上で warmup VK の
scan が `@` キーの物理位置に解決されるため、確定直後に `@` が1文字
リテラル表示される、というのが最も筋の通る説明。

**なぜこの warmup が不要と判断したか:** `ConfirmKeyUp` と対になる
`ConfirmKeyDown`（warm=true）分岐には既に「warm な GJI/TSF を確定キー
だけで cold 化する理由は tsf_mode に関係なく無い」という設計原則が
明記されており（2026-07-11 修正、BUG-24 false positive 対策）、
`ConfirmKeyUp` 側だけがこれと非対称に「念のため」の予防的 warmup を
無条件発行していた。正当化コメントも「(open軸のみの冪等キーなので)
反復送信も無害」という**無害性**のみで、必要性の根拠は示されていない。

cold-start 対策としての安全網は per-VK confirm（BUG-21 追記
2026-07-18: 1文字ずつ送信→確認、失敗時は backspace のみ）が既に担って
おり、実機ソーク数日で literal 化ゼロ件を確認済み。BUG-21 はまさに
同型の「予防的 warmup フルコース」（Chrome 側）が per-VK confirm と
二重の保険になっていたことを実機ソークで確認し、予防機構自体を物理
削除した前例であり、本修正は同じ判断を `ConfirmKeyUp` の eager warmup
に適用したもの。

**BUG-69/ADR-098 との関係（削除の安全性根拠）:** BUG-69 の premortem
評決は当初「eager warmup は KEEP（F1/F2 を先に直してからゲート強化。
単独で削ると TsfNative force-on の唯一の生存経路である F3 が露見し
即座に regression する）」だった。この前提条件（F1/F2 の根治、
`apply_force_on_for_imm_broken` を `force_on_attempt_allowed` の
cooldown ベース再試行に置き換え）は ADR-098 で実装済みであり、
2026-08-22 に Windows Terminal + GJI で「eager warmup とは無関係に
`force-ON (ImmBrokenForceOn): apply_ime_open(true) → Applied` が
独立して発火する」ことを実機で確認済み（BUG-69「Windows 実機での初回
検証」節）。したがって `ConfirmKeyUp` の eager warmup を削除しても、
TsfNative force-on が eager warmup だけに依存していた状態（F3）は
既に解消されており、force-on 経路自体は失われない。

**修正:** `CompositionFsm::on_event` の `ConfirmKeyUp` 分岐から
`EmitWarmup` の発行を削除し、`PendingWarmupOnKeyUp` → `Warm` への
状態遷移のみ残した（stale な pending を捨てる epoch/confirm_vk 照合
ロジックは維持）。`WarmupReason::ConfirmKeyUp` variant は使用箇所が
無くなったため削除。単体テスト
（`warm_tsf_confirm_keyup_does_not_emit_warmup`、
`warm_chrome_confirm_keyup_does_not_emit_warmup`）を warmup 非発行を
検証する内容に更新。

**テスト:** `cargo xwin check/clippy --tests --target
x86_64-pc-windows-msvc -p awase-windows` グリーン（`composition_fsm.rs`
の変更で新規の clippy 警告なし）。`composition_fsm` のテストは
`#[cfg(windows)]` のため Linux では実行不可、Windows 実機での
`cargo test -p awase-windows --lib` 実行は未実施。

**Windows 実機での初回確認（2026-08-22 追記、Windows Terminal /
PowerShell、GJI）:** ユーザーによる通常使用で「`@` の大量出力は改善した」
「（修正後）少なくとも1度も再現していない」ことを確認。ただし使用量が
多くないため**完全解消と断定できるだけのソーク量には達していない**——
継続観察が必要。

**未実施（引き続きソーク待ち）:** 長時間・高頻度タイピングでの完全解消
確認、cold-start 系の不具合（`giving up`/literal 化）が再発しないこと、
BUG-69 の force-on 経路が引き続き独立して機能することの確認。
ADR-100 決定3-c（VK 再選定実験）の一部として実機データを追加する。

**関連ファイル:** `crates/awase-windows/src/tsf/composition_fsm.rs`
（`CompositionFsm::on_event` の `ConfirmKeyUp` 分岐、`WarmupReason`）、
`crates/awase-windows/src/output/mod.rs`（`send_eager_tsf_warmup`）、
`crates/awase-windows/src/tsf/send.rs`（`send_eager_warmup_vk_pair`）。

**関連:** BUG-21（同型の予防的 warmup フルコース削除の前例、per-VK
confirm が安全網であることの実機ソーク済み根拠）、BUG-50（`VK_DBE_
HIRAGANA` の open+conv 束縛副作用）、BUG-69/ADR-098（`ConfirmKeyUp`
warmup 削除の安全性が F1/F2 の根治と force-on 独立経路の実機確認に
依存すること）、[ADR-100](../adr/100-gji-warmup-vk-ime-on-reinit.md)
（eager warmup キー再選定の親トラック）、
[experiment-logging](../.claude/rules/experiment-logging.md)、
[fix-requires-evidence](../.claude/rules/fix-requires-evidence.md)。

## BUG-71: バージョンアップ時に `config.toml`/`layout/*.yab` が失われる（MSI の `MajorUpgrade` スケジューリング欠落 + ZIP アンインストーラーの無条件全削除 + `awase-settings` の load 失敗時サイレントフォールバック）

**発見の経緯:** ユーザーから「バージョンアップすると既存の設定が失われる」
という不具合報告があった。Explore エージェントによる全体調査、直接の
コード検証、Opus premortem レビュー3ラウンド（round1〜round3）を経て
原因を特定し、[ADR-099](adr/099-config-preservation-on-upgrade.md) として
設計・実装した。

**症状:** MSI 版・ZIP 版のいずれでバージョンアップしても、`config.toml`
（キー設定・レイアウト選択等）と `layout/nicola.yab`（awase-settings の
「配列編集」タブでカスタマイズした配列）がデフォルト値に戻ってしまう。

**原因（F1〜F5）:**

1. **F1（最重要・MSI 版）: `wix/main.wxs` の `<MajorUpgrade>` に
   `Schedule` 属性が無く既定値 `afterInstallValidate` が適用され、
   新バージョンのファイルインストールより**前**に旧バージョンが完全
   アンインストールされていた。** `ConfigFile` コンポーネントの
   `NeverOverwrite="yes"` は「既に存在するファイルを上書きしない」制御
   であり、旧製品アンインストールで一度削除されてしまえば何も保護しない
   （`NeverOverwrite` の `KeyPath` はファイルではなく `HKCU` レジストリ
   値であることも保護を無力化する一因）。`Product Id="*"` により毎
   リリースで必ずこのメジャーアップグレード経路を通るため、MSI で
   インストールした全ユーザーが影響を受ける。あわせて `layout/nicola.yab`
   の `NicolaYab` コンポーネントには元々 `NeverOverwrite` 自体が付いて
   いなかった（`ConfigFile` にのみ付与されていた）ため、`Schedule` を
   修正しても通常アップグレード時の上書きまでは防げない別経路の穴も
   あった。
2. **F2（ZIP 版）: `scripts/uninstall.ps1` が `%LOCALAPPDATA%\awase` を
   無条件かつ再帰的に削除していた。** ZIP 配布には「アップグレード」
   専用の手順が用意されておらず、`install.ps1`/`uninstall.ps1` という
   対の名前から「アップグレード＝アンインストール→インストール」という
   手順を自己判断で踏むと、`config.toml`/`layout/*.yab`/`data/*` が
   まとめて消失していた。
3. **F3（ZIP 版）: `scripts/install.ps1` が `config.toml` は「既存なら
   上書きしない」のに `layout/*` は無条件 `-Force` 上書きしていた。**
   `awase-yab-editor` は独立バイナリとしては撤去済みで `awase-settings`
   の「配列編集」タブに統合されており、GUI でその場の `nicola.yab` を
   編集・上書き保存する導線が存在するため、影響は手編集ユーザーに
   限らず GUI 編集を使った全ユーザーに及んでいた。
4. **F4（最も重大・インストーラーと無関係に発生しうる）: `awase-settings`
   が `AppConfig::load()` 失敗時に `default_config()` へ静かにフォール
   バックし（`log::warn!` のみ、GUI 上の可視表示なし）、その状態で
   「適用」を押すとデフォルト値が `config.toml` へ永続化されていた。**
   `AppConfig::general`（`src/config.rs`）に `#[serde(default)]` が
   付いておらず、`[general]` セクションを欠く（あるいは書き込み中の
   クラッシュ等で壊れた）`config.toml` は即座に parse error になり
   この経路を誘発しやすかった。`AppConfig::save()` も一時ファイル経由の
   アトミック書き込みではなく `std::fs::write` の直接上書きだった。
5. **F5（関連リスク、根治は別 ADR）: `src/paths.rs::resolve_relative_to`
   の CWD フォールバックにより、`config.toml` が想定外の場所に新規作成
   されたり、`awase.exe`/`awase-settings.exe` が異なるファイルを読み書き
   しうる。**2026-07-19 に確認済みの既知の混乱（`crates/awase-settings/
   src/main.rs` のコメント参照）と同根。

**対応（ADR-099 決定0〜8、実装済み・2026-08-21）:**

- 決定0: `<MajorUpgrade Schedule="afterInstallExecute">` を追加し、
  `NicolaYab` コンポーネントにも `NeverOverwrite="yes"` を追加（`wix/main.wxs`）。
- 決定1: `scripts/uninstall.ps1` のデフォルト動作からユーザーデータ削除を
  除き、完全削除は `-Purge` 明示指定時のみに限定。
- 決定2: `scripts/install.ps1` の `layout/*` コピーを「既存なら上書き
  しない」方式に変更（`data/*` はプログラム資産のため従来通り）。
- 決定3: `AppConfig::save()`（`src/config.rs`）を `File::create` →
  `write_all` → `sync_all` → `rename` のアトミック書き込みに変更（rename
  失敗時は初回試行の後50ms間隔で最大4回リトライ、初回と合わせて最大5回
  試行・最大200msブロック）。
- 決定4: `ConfigLoadState`（`Loaded`/`NotFound`/`Dangerous`）と
  `classify_load_error`（`src/config.rs`）を新設。`awase-settings` は
  `Dangerous`（`NotFound` 以外の全ての load 失敗）のときのみ、
  `config_path_panel` への警告表示・保存前の一度限りの `.bak` バック
  アップ・保存前の確認モーダル（`egui::Window`）を必須にする。
- 決定6: `AppConfig::general` に `#[serde(default)]` を追加。
- 決定7: `src/paths.rs::resolve_relative_to` が CWD フォールバックに
  到達した場合の `log::warn!` を追加。
- 決定8: `crates/awase-windows/tests/wix_installer_guard.rs`（新規）で
  `wix/main.wxs` の `Schedule`/`NeverOverwrite`/GUID を機械的に固定。

**検証**: `cargo test`（root/`awase-windows`/`awase-settings` 合計800件超、
新規追加は `wix_installer_guard` 3件・`config::tests` 追加10件・
`awase-settings` 追加7件）、`cargo fmt --check`、`cargo clippy --lib`
（CI 相当コマンド）は全てクリーン。加えて `cargo xwin check`/`clippy`/
`build --tests --target x86_64-pc-windows-msvc -p awase-windows -p
awase-settings`（実際の Windows ターゲットへのクロスコンパイル）も全て
クリーンに完了（Linux ネイティブビルドでのみ出る無関係な既存
`dead_code` 警告は Windows ターゲットでは発生しないことも確認済み）。

実装完了後、Opus によるコードレビューで2件の確定バグを検出・修正した:
`crates/awase-windows/tests/wix_installer_guard.rs` の `Schedule`
チェックが `wix/main.wxs` 全体を文字列探索していたため、`<MajorUpgrade>`
直前の説明コメントに書いた同じ文字列と一致してしまい、実際の属性を
消してもテストが失敗しない状態になっていた（タグ範囲を切り出して
判定するよう修正）。また `apply_confirmed()` の保存前バックアップは
コピー失敗時も警告ログのみで保存を続行しており、バックアップが最も
必要な場面（`PermissionDenied` 等）でこそ原本を無防備に上書きしうる
余地があった（バックアップ失敗時は保存を中止するよう修正）。

**Windows 実機でのアップグレード検証（MSI メジャーアップグレード・ZIP
`install.ps1` 再実行のいずれも）は未実施。** 特に MSI 経路は
`afterInstallExecute` への変更で挙動が変わるため、実機（`msiexec /l*v`
ログでの確認を含む）での検証が最優先。詳細な手動検証チェックリストは
ADR-099「テスト方針」節参照。

**コードレビュー追加修正（PR #82 マージ前）:**

- `AppConfig::save()` を `crates/awase-settings/src/main.rs::apply_confirmed()`
  から呼ぶと egui の UI スレッドを rename 失敗時に最大200msブロック
  していた（`tray.rs` 経由のエンジンスレッド呼び出しは ADR round2 SF-1で
  既に許容範囲と分析済みだったが、より高頻度に呼ばれる設定 UI 側は
  未検討だった）。バックアップ＋保存をバックグラウンドスレッドへ委譲し、
  `poll_pending_save()` で毎フレームノンブロッキングにポーリングする形へ
  変更（回帰テスト: `apply_confirmed_returns_without_blocking_on_slow_save`）。
- `cancel()` が `show_dangerous_save_confirm` をリセットしていなかった
  ため、確認モーダル表示中に背景の「キャンセル」ボタン（モーダルは
  ブロッキングオーバーレイを持たないため操作可能）を押すと、状態は
  `Loaded` に正常化するのに古い警告モーダルだけが表示され続けるバグが
  あった（回帰テスト: `cancel_closes_dangerous_save_confirm_modal`）。
- `AppConfig::save()` の実体（tmp+fsync+rename+リトライ）を
  `crate::fs_atomic::write_atomic` へ切り出し、同じ Windows rename ロック
  問題を抱えていた `crates/awase-windows/src/gji_charset_write.rs` の
  `config1.db` 書き込みからも共有するようにした。あわせて `path` が
  シンボリックリンクの場合はリンク先の実体へ書き込み、既存ファイルの
  パーミッションを新しいファイルへ引き継ぎ（従来は `File::create` が
  umask 依存のデフォルト権限になり `chmod` 済みのファイルの権限が保存の
  たびに失われていた）、宛先が読み取り専用の場合はリトライを省略して
  即座にエラーを返すようにした（回帰テスト:
  `write_atomic_preserves_existing_permissions`・
  `write_atomic_follows_symlink_to_target`、`src/fs_atomic.rs`）。
- リトライ幅の記載が実装と不一致だった（本文中「50ms×5=250ms」は
  誤りで、実装は最終試行後にスリープしないため実測200msが正しい上限）
  ため訂正した。
- **自己レビューで追加発見**: 上記の「既存ファイルのパーミッションを
  新しいファイルへ引き継ぐ」処理と「宛先が読み取り専用ならリトライを
  省略」処理が組み合わさると、宛先が読み取り専用の場合に一時ファイル
  自身もそのパーミッション（読み取り専用）を引き継いでしまい、失敗時の
  クリーンアップ（`remove_file`）が Windows では読み取り専用ファイルの
  削除に失敗するため `<path>.tmp.<pid>` が永久に残留しうるバグがあった。
  `clear_readonly_and_remove`（Windows限定で読み取り専用属性を先に外して
  から削除）を新設して修正（回帰テスト:
  `clear_readonly_and_remove_clears_attribute_before_deleting`、Unix上は
  スモークテスト止まりで Windows 実機検証は未実施）。修正の過程で
  `Permissions::set_readonly(false)` が Unix では `0o777`
  （world-writable）にしてしまう既知の落とし穴
  （`clippy::permissions_set_readonly_false`）に気づき、この処理自体を
  `#[cfg(windows)]` 限定にした。
- **自己レビューで追加発見**: `apply_confirmed()` は保存が進行中
  （`pending_save` が `Some`）の間に再度呼ばれても多重起動はしないが、
  「適用」ボタン自体は無効化していないため連打すると無言で無視される
  だけだった。ステータスに「保存中です。少々お待ちください…」を表示する
  よう改善（回帰テスト:
  `apply_confirmed_shows_status_when_save_already_in_progress`）。

## BUG-72: タスクトレイ「不具合を報告」ウィンドウの日本語が文字化け（トーフ表示）する

**症状:** タスクトレイから「不具合を報告」を開くと、見出し・症状カテゴリの
選択肢・JSON プレビュー中の日本語（IME 製品名・競合ソフト名・配列データ等）
が読めない状態（文字化け、実際にはグリフ無しの空白/トーフ表示）になる。
通常の設定画面（awase-settings のメインウィンドウ）は正常に日本語が表示
される。

**原因:** `crates/awase-settings/src/bug_report.rs::run()` は
`--bug-report` 起動時に呼ばれる独立した `eframe::run_native` 呼び出しで
あり、メイン設定画面（`SettingsApp::new()`、`crates/awase-settings/
src/main.rs`）が呼んでいる CJK フォント読み込み処理 `setup_fonts()`
（`C:\Windows\Fonts\meiryo.ttc`/`msgothic.ttc`/`YuGothR.ttc` 等を探して
`Proportional`/`Monospace` フォントファミリへ挿入）を一度も呼んでいな
かった。この結果、不具合報告ウィンドウは egui 同梱の既定フォント
（日本語グリフを一切含まない）のままレンダリングされ、日本語文字列が
すべて欠落グリフ（トーフ）表示になっていた。データ自体（UTF-8 文字列・
JSON）は正しく、バイト列レベルの文字コード破損ではなくフォント未設定
によるレンダリング問題だった。

**修正:** `run()` の `run_native` クロージャで `crate::setup_fonts(&cc.
egui_ctx)` を呼ぶよう追加。回帰テスト
`bug_report::font_guard_tests::run_native_closure_calls_setup_fonts`
（`architecture_guard.rs`/`wix_installer_guard.rs` に倣ったソース文字列
走査によるガード、egui のヘッドレス描画を要さず Linux `cargo test` で
完結）を新設。**執筆時の自己レビューで判明**: 当初 `closure_body.
contains("setup_fonts")` という緩い判定にしていたところ、修正の説明
コメント自体に "setup_fonts" という語が含まれるため、実際の呼び出しを
削除してもテストが検知不能（wix_installer_guard.rs の C1 と同型の
落とし穴）になることに気づき、`setup_fonts(&cc.egui_ctx)` という括弧
付きの呼び出し式で判定するよう修正して確認済み。

**テスト:** `cargo test -p awase-settings` 全26件緑（新規1件含む）、
`cargo fmt --all -- --check`、`cargo xwin build --target
x86_64-pc-windows-msvc -p awase-settings`（実 Windows ターゲットへの
クロスコンパイル）緑。Windows 実機での表示確認は未実施。

**関連ファイル:** `crates/awase-settings/src/bug_report.rs`（`run()`）、
`crates/awase-settings/src/main.rs`（`setup_fonts()` 定義、
`SettingsApp::new()`）。

**関連:** [ADR-095](adr/095-tray-bug-report-cloudflare-intake.md)
（タスクトレイ不具合報告機能の導入元）。

## BUG-73: BUG-72修正の副作用で「不具合を報告」ウィンドウが背面のまま開き「一瞬表示されてすぐ消える」ように見える

**発見の経緯:** BUG-72（不具合報告ウィンドウの文字化け）を修正しリリース後、
ユーザーから「不具合を報告を押したら、即座にクラッシュしているような
動作になっている」との報告があった。

**調査（実機、Windows、`clipwire` 経由でリモート確認）:**
`awase-settings.log` に `[PANIC]` 行は一切無く、Rust パニックではないと
判明。`Get-Process`/Win32 `GetWindowRect`/`IsWindowVisible` で直接確認した
ところ、ウィンドウは実際には**正常に生成され、モニタ範囲内に正しく
配置され、`Responding=True`・`IsWindowVisible=True`** だった。プロセスも
クラッシュせず生存し続けていた（テスト中に同一ウィンドウが複数、
バックグラウンドで生存したまま残留していたことを確認）。ユーザーへの
確認でも「Alt+Tab/タスクバーには一瞬表示された」ことが確認された。

**原因（推定、Windows実機でのAPIレベル計測により高確度）:**
`awase.exe`（タスクトレイ、バックグラウンドプロセス）が
`awase-settings.exe --bug-report ...` を子プロセスとして起動する。
Windows は「ユーザー操作（トレイメニュークリック）に由来する新規
ウィンドウ」に対して、生成後短時間のうちに前面へ来る権利
（フォアグラウンド許可）を与えるが、この猶予には時間制限がある。
BUG-72 の修正で `crate::setup_fonts()`（数MBの CJK `.ttc` フォント
ファイルの読み込み・パース）を `eframe::run_native` の**ウィンドウ生成
クロージャ内**で同期的に呼ぶようにしたため、ネイティブウィンドウが
実際に `ShowWindow` されるまでの間に数百ms の追加遅延が生じ、この
猶予時間を逃してウィンドウが**背面のまま**開くようになったと考えられる
（`SettingsApp`側も同じ `setup_fonts()` を呼ぶが、既存の config/layout
読み込み等で元々の起動シーケンスが違うため同じ影響を受けなかったと
推測、未確証）。ウィンドウ自体は正常に存在し応答するため、ユーザーには
「一瞬見えてすぐ消えた（＝クラッシュしたように見える）」という体感に
なる。

**修正:** `setup_fonts()` の呼び出しを `run_native` のウィンドウ生成
クロージャから `BugReportApp::update()` の初回フレームへ遅延させた
（`fonts_initialized: bool` フィールドで一度だけ実行を保証）。ウィンドウ
生成自体（`eframe::run_native` の呼び出しからネイティブウィンドウが
`ShowWindow` されるまで）はフォント読み込みを待たず即座に行われるように
なり、CJK フォントのパースはウィンドウが既に表示された後の最初の
`update()` 呼び出し内で行われる。

**テスト:** ソース文字列走査による回帰テスト2件を新設
（`setup_fonts_is_deferred_to_first_update_not_run_native_closure`:
`run_native` のクロージャが `setup_fonts` を呼ばないこと・`update()` が
呼ぶことの両方を固定、`new_app_has_fonts_not_yet_initialized`:
`BugReportApp::new()` 直後は `fonts_initialized=false` であることを固定）。
`cargo test -p awase-settings` 全27件・`cargo fmt --all -- --check`・
`cargo xwin build --target x86_64-pc-windows-msvc -p awase-settings`
（実Windowsターゲット）緑。**Windows実機（dragonflyg4、`clipwire` 経由の
リモートビルド＋タスクトレイからの実操作）で確認済み（2026-08-23）:
修正版デプロイ後、トレイの「不具合を報告」から正常にウィンドウが前面へ
表示され、見出し・症状カテゴリ等の日本語も文字化けせず正常に表示される
ことをユーザー本人の操作で確認した。**

**関連:** [BUG-72](#bug-72-タスクトレイ不具合を報告ウィンドウの日本語が文字化けトーフ表示する)
（本バグの直接の原因となった修正）、
[ADR-095](adr/095-tray-bug-report-cloudflare-intake.md)。

---

## BUG-74: `RawTsfLiteralRecovery` の give-up（2連続 raw-tsf-literal）で文字が痕跡なく完全に失われる — BUG-29 が予告していた「次回の同種報告」

**発見の経緯:** ユーザーからの不具合報告機能（ADR-095、report_id
`01M0RE56S6EQ4MTQGJ2EB2W4N0`）経由。症状カテゴリ「一部の文字が消えた」、
説明「こういう と期待したのに ういう になってしまった」（app_version
1.14.0）。

**症状:** Windows Terminal（`CASCADIA_HOSTING_WINDOW_CLASS` →
`Windows.UI.Input.InputSite.WindowClass`、GJI、TsfNative）で、フォーカスが
Windows Terminal へ移った直後（20秒以上アイドル後の GJI、`app_kind=Uwp`、
`focus_kind=Undetermined`）に「こういう」と入力したところ、先頭の「こ」が
**痕跡なく完全に失われ**「ういう」になった。BUG-39（`koっか`）や BUG-36
（`tみや`）のようなローマ字の literal 漏れ（画面上に見える）とは異なり、
本件は BS で消したあと何も再送されないため見た目にも何も残らない。

**再現手順（不具合報告の添付 journal/awase.log で確認済み、`RUST_LOG=debug`）:**

```
send_keys: mode=Tsf actions=[Char('こ')] prev_elapsed=18094ms
[h1-warmup] cold=37 ... reason=NativeF2Consumed elapsed=0ms → F2/probe待機省略、per-VK confirm へ
[gji-coro] cold=37 settle 必要 (gji_idle_ms=20157 settled=false) → skip FreshF2, reactive LiteralDetect のみ
[gji-coro] cold=37 per-VK[0/1] suspected literal (vk=0x4B backs=1 escape=false)
[raw-tsf-literal] cold=37 raw TSF literal suspected → backspace ×1 + re-送 "ko" scheduled + mark cold
[raw-tsf-literal] flush escape=false backspace ×1
[output] re-sending raw TSF literal romaji="ko"
[h1-warmup] cold=38 ... reason=RawTsfLiteralRecovery elapsed=516ms → F2/probe待機省略、per-VK confirm へ
[gji-coro] cold=38 per-VK[0/1] suspected literal (vk=0x4B backs=1 escape=false)
[raw-tsf-literal] cold=38 consecutive raw-tsf-literal (count=2) → giving up, backs=1 cleanup only (no re-send)
[raw-tsf-literal] flush escape=false backspace ×1
[chrome-reinit] cold=38 VK_IME_OFF→VK_IME_ON 強制リセット送信 + IMC ポーリング開始
[chrome-reinit] cold=38 Hiragana 確認 → ポーリング終了   ← reinit 自体は成功、以降「う」「い」「う」は正常
```

視覚的な出力の変遷を追うと: 空 →（cold=37 literal）"k" → backspace →
（cold=38 再送 "ko" の1文字目 "k"）→ backspace → 何も残らず「こ」は消滅。
後続の「う」は `gji_settled=true`（reinit 直後で GJI I/O が生きている）ため
unicode 直接送信に切り替わり正常に変換された。

**IME:** Google 日本語入力（GJI）。TsfNative プロファイル（Windows Terminal
等）。

**原因（確定、コード読解と journal/ログの突き合わせで確認）:** `cold_warmup.rs`
の `run_start` は 2026-07-18 の設計変更（BUG-24 追補、本ファイル参照）以降、
`reason` や `gji_idle_ms` に関わらず送信前の F2/probe 待機を一切行わない
（「予防的待機は per-VK confirm という送信後リカバリと二重の保険」という
実機ソーク済みの意図的な仕様）。今回のケースでは 20 秒以上アイドルしていた
本物の cold TSF context に対してこの「待機省略」がそのまま適用され、
1文字目の送信（cold=37）が genuinely 早すぎて `SuspectedLiteral` になった。
その回収（backspace + 再送 "ko"、cold=38）も同じ「待機省略」設計のため、
わずか 516ms 後に再送された1文字目も再び早すぎて `SuspectedLiteral` になり、
`consecutive_count()==1` で `probe_io.rs` の `RawTsfLiteralRecovery` ハンドラの
give-up 分岐（`consecutive != 0`）に入る。この分岐は BUG-27 追補2（「常に
再送」が msedge で無限 backspace ループを起こし撤回済み）以来、**romaji を
一切再送せず BS のみで後始末する**設計であり、失われた文字を取り戻す経路が
存在しなかった。BUG-33 で追加された `send_chrome_gji_reinit_and_poll`
（`VK_IME_OFF→VK_IME_ON`）は GJI を実際に Hiragana へ立て直すため後続文字
（「う」「い」「う」）には効くが、give-up した「こ」自体を救う仕組みはなかった。

本件は BUG-29「未解決の follow-up」節が明示的に予告していたケースそのもの:
「`RawTsfLiteralRecovery` の『2回連続失敗で以後無期限に give-up』という設計
自体は、構造的な保護（cap・エスカレーション）が依然として存在しない…次回
同種の報告があれば `probe_io.rs` の give-up 分岐自体の見直しを検討する。」

**この「見直し」自体は ADR-100 が既に検討・却下済みだった（訂正の経緯）:**
実装着手時、まず「give-up 分岐に reinit 完了確認後の retry を追加する」
（`send_chrome_gji_reinit_and_poll` の IMC ポーリングが Hiragana 復帰を
確認できたら、失われた romaji を一度だけ再送する）という設計で実装し、
`cargo xwin check`/`clippy`/`test --no-run` まで通した。しかしコミット前に
`docs/experiments.md` エントリ16 を確認したところ、**この設計はまさに
[ADR-100](adr/100-gji-warmup-vk-ime-on-reinit.md) 決定3「提案2」として
既に検討・却下されていた**ことが判明し、実装を破棄した。却下理由（ADR-100
決定3、4点）:

1. **完了通知の経路が存在しない**（最も強い理由）: `send_chrome_gji_reinit_and_poll`
   のポーリングは fire-and-forget で、retry を安全に配線するには focus 世代の
   照合が要る。この照合機構自体が欠落している（決定5、`send_chrome_gji_reinit_and_poll`
   だけが `ime_mode_focus_gen` を照合しない）ことが未解決の前提条件として
   記録されており、これを満たさずに retry を配線すると、reinit のポーリング中
   （最大 `CHROME_GJI_REINIT_CONFIRM_MS`=300ms）にフォーカスが別ウィンドウへ
   移った場合、**失われた romaji が別ウィンドウへ誤送信されうる**
   （premortem P2、BUG-35 と同型の stale confirm 誤帰属）。
2. 「確認できない」瞬間が黙って「300ms 経過」に劣化しうる — IMC が読めない
   環境では実質 BUG-27 追補2（msedge で無限 backspace ループを起こし撤回済み）
   と同種の無条件タイマー retry に近づく。
3. reinit 自体が短時間の連続 give-up ではレート制限で skip されうる
   （F11）ため、「reinit 完了確認後」の定義が skip 時に未定義になる。
4. BUG-45（未解決）はこの経路自体が「actual にどう出力されたか確認する
   箇所が一つもない」ことを問題の核心としており、retry はその上に送信を
   もう1段積むことになる。

ADR-100 決定3は却下する代わりに**案L（give-up で失った romaji を journal に
記録する。送信ゼロ・挙動変更ゼロ）を採用**し、案J（Unicode 直接送信への
退避）・案K（backspace も打たない）を「却下せず保持」として残していた。
本バグはこの案L が策定済みでありながら未実装だったことも明らかにした
（`tsf/literal_facts.rs::LiteralDetectRecord` に romaji フィールドが無かった）。

**修正: ADR-100 決定3 案L を実装した。** 送信・挙動は一切変更せず、
`LiteralDetectRecord` に `romaji: Option<String>` フィールドを追加し、
journal から「give-up で何が失われたか」を機械可読に復元できるようにした。

- `tsf/literal_facts.rs::LiteralDetectRecord` に `romaji: Option<String>` を
  追加（`String::new()` ではなく `Option` にした理由: 空文字列だと「記録し
  忘れ」と「そもそも romaji を持たない verdict」が区別できなくなるため）。
- 構築サイト全6箇所（ADR-100 決定3「案L の作業範囲」表のとおり）を更新:
  `output/probe_io.rs` の `RawTsfLiteralRecovery` ハンドラ（**romaji を持つ
  唯一の箇所**、初回疑い・give-up 双方で `romaji.clone()` を詰める）・
  `CompositionConfirmed` ハンドラ・`LiteralDetectNote` ハンドラ・
  `plan_skipped_record`、`platform.rs::flush_pending_literal_vk_as_aborted`
  （`probe_io.rs` の外にあるため grep しないと気づきにくい、ADR-100 が
  明記済み）、`journal_policy.rs` のテストヘルパーは全て `None`。
- プライバシー方針は ADR-100 決定3 で確定済み（journal は既に `attach_log`
  チェックボックスの opt-in 配下であり、`consecutive == 0` 側の
  `log::warn!` が既に同じ romaji を生ログへ出力しているため、新しい送信
  チャネルは開かない。生の romaji をそのまま記録する）。

**テスト:** `output/probe_io.rs::tests::
raw_tsf_literal_recovery_tsf_mode_consecutive_gives_up_with_cold_mark`
（give-up 側）に `record.romaji == Some("ko")` のアサーションを追加。
`raw_tsf_literal_recovery_sets_literal_and_marks_cold_when_first_time`
（初回疑い側）も `dispatch_probe_actions` + 明示 `trace` を使う形に変更し、
`record.romaji == Some("ka")` を検証するアサーションを追加。ADR-100 が
「Linux で実行可能」と明記していたとおり `fix-requires-evidence.md` の
(a) 回帰テストで満たせる（`#[cfg(test)]` 内の純粋なユニットテストで
Win32 API に依存しない）。`cargo xwin check`/`cargo xwin clippy -p
awase-windows --target x86_64-pc-windows-msvc`（警告ゼロ）、`cargo xwin
test -p awase-windows --target x86_64-pc-windows-msvc --no-run`
（lib・`tests/*.rs` 全ファイルのコンパイル・リンク）確認済み。wine 未導入
のためこのサンドボックスでは `.exe` 実行はできず、実機再検証は未実施。

**2026-08-24 追補: ADR-101 により文字消失そのものの修正を実装した。**
ADR-100 決定3が却下した「提案2」は、完了通知・focus世代照合・送信後処理・
順序保証が無い状態での retry だった。ADR-101 はこの前提条件を4ラウンドの
premortemで詰め直し、以下を実装した。

- Stage1: `send_chrome_gji_reinit_and_poll` に `ime_mode_focus_gen` 照合を追加し、
  stale な IMC poll 結果で `ImeModeFsm` を更新しないようにした（ADR-100 決定5/F6）。
- Stage2: give-up 由来の reinit 予約を `PendingGjiReinit { cold_seq, focus_gen, phase }`
  に構造体化し、`Scheduled`/`Polling` を型で分けた。`Polling` は
  `OutputActiveGuard` と `poll_token` を所有し、連続give-upで上書きされない。
- Stage3: poll が `Confirmed` かつfocus世代一致の場合のみ、保存していた romaji を
  `send_romaji_batched` / `send_romaji_as_tsf` の通常送信経路へ1回だけ戻す。
  Unicode直接送信の新経路は作らない。retry後は `drain_output_post_send_effects`
  を必ず実行し、その後に `pending_deferred` を処理する。
- Round4 premortemで見つかった順序問題への対策として、retry付き `Polling` 中は
  `flush_stale_deferred_vks_after_recovery` による `pending_deferred` 即時flushを
  抑止し、`SuppressedExistingPoll` では `RAW_TSF_LITERAL` へ backspace cleanup を
  残さない。

関連ADR: [ADR-101](adr/101-bug74-giveup-retry-with-focus-guard.md)。

**2026-08-24 追補2: 実コードに対する最終レビューで実装ミス2件を発見・修正した。**
ADR-101の設計文書（Round4）に対するpremortemはブロッカー0件で収束したが、
別のセッションが実際のコード diff（設計文書ではなく）を最終レビューしたところ、
「設計は正しかったが実装が設計から逸脱している」ミスが2件見つかった:
(1) focus stale判定が`flush_raw_tsf_literal_backspaces()`（実送信）より**後**に
行われており、フォーカスが変わった後もbackspaceが新ウィンドウへ送られてから
ようやくstale判定される状態だった、(2) `with_app`再入失敗（`None`）を`Stale`
扱いにしており、フォーカスが変わっていなくても1tick再入しただけでretryと
deferred救済の両方が失われる状態だった。両方とも実送信前のfocus世代照合
（`discard_raw_recovery_if_focus_stale`）と、再入は`Continue`とする純粋関数
（`gji_reinit_poll_tick_outcome`）で修正し、それぞれの回帰テストを追加した。
詳細はADR-101「Premortemの経緯 > コードレビューによる実装ミスの訂正」節参照。
**設計のpremortemだけでなく実装後のコードレビューも必須である**ことの実例。

**未解決の残課題（ADR-100 決定4・ADR-101 を継承）:** ADR-101 は BUG-74 の
文字消失そのものを修正したが、実機ソークは未実施である。ADR-100 決定4-a（give-up の
実機発生頻度をアプリ別・`injection_mode` 別に数える）はまだ未実施であり、
本件が「3件記録済みの実害」（BUG-16 追補3・BUG-38/39 追補2・BUG-45）に続く
**4件目**の実機データになる。次に同種の報告が来た場合、案L が記録した
romaji と ADR-101 のretryログを突き合わせ、Timeout/Stale/discard が実害として
残っていないか判断すること（ADR-100 決定4 参照）。cold=37/cold=38 双方の根本原因である
「送信前 F2/probe 待機の完全撤去」（2026-07-18、BUG-24 追補）自体も未変更
のまま。**`SuppressedExistingPoll`（既存retry pollが進行中に別のgive-upが来た
場合）では、その2件目のgive-upのbackspace cleanup自体を一切送らない設計
（ADR-101決定5）——新しいliteral残骸が画面に残る可能性はADR-101本文で
意図的に受容したトレードオフとして明記済みだが、この残課題節への転記が
漏れていたため追記する（コードレビュー指摘）。また`discard_raw_recovery_
if_focus_stale`が対象とするのはgive-up+reinit経路のみで、`consecutive==0`
（初回疑い、reinit未予約）のraw literal cleanupがfocus変更後に別ウィンドウへ
送られるリスクはこのPR以前から存在し今回も未修正のまま（BUG-74のスコープ外、
コードレビュー指摘）。**MS-IME側の`start_ms_ime_ready_poll`にも、ADR-101で
GJI側を修正したのと同型の`with_app`再入バグ(`.unwrap_or(MsImePollStatus::
Stale)`)が本PR以前から残っている**(BUG-13領域、コードレビュー指摘、未修正・
未観測)。次にMS-IME側でIMC確認ゲートが理由なく固着する系の症状が報告されたら
ここから着手すること。

**2026-09-03 追補3: 当初「`SuppressedExistingPoll`の残骸トレードオフ」と誤診断
——実際はADR-123が実装着手条件としていた確証データだった（訂正版）**

不具合報告機能（ADR-095、report_id `01M1KEGZ081YHJ1T2NC765SYYH`）経由。app_version
1.18.0、Windows Terminal（`Windows.UI.Input.InputSite.WindowClass`、app_kind=Uwp、
TsfNative）、GJI。症状カテゴリ WrongCharacterOutput、説明「github pages にはしないで
と入力しているのに github pages sにはないで となってしまった」。

**初版の診断は誤りだった。** journalの`elapsed_ms`（absorb時刻）だけで時系列を
再構成し、「し」が`SuppressedExistingPoll`（ADR-101決定5）により完全に失われた、
と結論したが、Opus敵対的レビュー（3ラウンド）で`KeyInput.timestamp_us`（物理
打鍵の実時刻）ベースで再検証した結果、以下が判明した:

- 症状は「文字消失」ではなく**「追い越しによる順序反転＋約1.85秒の遅延」**
  だった。「github pages」の直後に来る「H」由来のモーラ（ユーザー生入力）は
  失われておらず、`pending_deferred`に滞留した後1.85秒遅れて出力されている。
  一方「S」由来のliteral "s"は、`pending_deferred`に積まれず即座に独立した
  新しいprobe（cold_seq=27）を開始して先に確定してしまったため、後から出力
  される「には」より前の位置に出た。
- `SuppressedExistingPoll`自体は今回の一次原因ではなく、二次的な症状の一部
  でしかない。
- **真因は既存の[ADR-123](adr/123-focus-resync-and-probe-defer-queue-composition-race.md)
  が既に特定していた「`defer_if_probe_in_flight`が`has_pending_tsf()`だけを見て
  `pending_deferred`の非空性を考慮しないため、flush待ちの後続モーラを新規入力が
  追い越せる」というギャップと同一だった。** ADR-123は前回のインシデント
  （`01M1JJD54XQXSEJTHHFKV1WKA1`、「たとえば」→「ばたと」）でこの機序を確定
  させていたが、decisionの実装は「次回同種の報告でjournalの`TsfProbeStarted.
  pending_deferred_len`が実際に非ゼロと確認できたら着手する」条件付きで保留
  されていた。本reportのjournalで`TsfProbeStarted`の`pending_deferred_len`が
  cold_seq=27時点で2、cold_seq=28時点で4（いずれも非ゼロ）と確認され、
  **ADR-123が待っていた実装着手の確証データそのものだった。**

**対応方針:** ADR-123のステータスを「実装着手」へ更新し、恒久対策の設計・
実装はそちらで一元管理する（本節はこの誤診断の記録として残す）。

**この誤診断自体からの教訓:** journalの`elapsed_ms`は「journalへ記録された
（absorbされた）時刻」であり、`KeyInput.timestamp_us`（実際に物理キーが
発生した時刻）とは別物——give-up/reinit/drain replay等で記録タイミングが
実打鍵から数十〜数百ms遅延することがあるため、複数イベントの前後関係を
厳密に議論する際は`timestamp_us`を基準にすること。

**2026-09-03 追補4: ADR-123実装完了（PR #155、developマージ予定）。
warm経路配線（変更A+C 4-4）の核心的な振る舞い変化に直接対応する回帰
テストが存在しない（`fix-requires-evidence.md`対象領域につき記録）。**

真因（`pending_deferred`の追い越し）の恒久対策として、ADR-123の決定
（D→E→B→A+Cの5変更）を実装した。詳細な設計・コミット一覧・Opus敵対的
レビューで発見・修正した2件の自己defer退行は
[ADR-123](adr/123-focus-resync-and-probe-defer-queue-composition-race.md)
「実装（2026-09-03）」節に記録済み。

本節への追記が必要な理由: 4-4（`send_romaji_batched_gated`/
`send_romaji_as_tsf_gated`のgate判定をcold/warm分岐の外へ移動し、warm
判定されたモーラにもgateを適用する変更）は`output/vk_send.rs`（キー選択・
warmup/cold-startの再発ファミリー対象）を変更する fix だが、この振る舞い
変化そのものを直接固定するユニット/e2e/goldenテストがどちらも存在しない
（`GjiFsm`を実際に`OnWarm`へ遷移させるテストセットアップが必要でこのPRでは
見送った）。`fix-requires-evidence.md`が要求する「テストかknown-bugs.md
記載」のうち後者として、この gap を明示的に記録する。次に
`pending_deferred`の追い越し・順序反転系の症状が再発した場合、まず
warm経路（`prepend_f2_warmup=false`）で発生していないか確認すること
——4-4はcold経路（`prepend_f2_warmup=true`）と異なる配線のため、
cold側だけ直したつもりでwarm側の穴が残っている可能性を疑うこと。

**関連ファイル:** `crates/awase-windows/src/tsf/literal_facts.rs`
（`LiteralDetectRecord::romaji` 新設）、`crates/awase-windows/src/output/probe_io.rs`
（`RawTsfLiteralRecovery`/`CompositionConfirmed`/`LiteralDetectNote` ハンドラ、
`plan_skipped_record`、`send_chrome_gji_reinit_and_poll`、`gji_reinit_poll_tick_outcome`）、
`crates/awase-windows/src/output/mod.rs`（`PendingGjiReinit`/`PendingGjiReinitPhase`、
`schedule_pending_gji_reinit`、`start_pending_gji_reinit_after_raw_cleanup`、
`discard_raw_recovery_if_focus_stale`、`flush_deferred_vks_after_gji_reinit_completion`）、
`crates/awase-windows/src/platform.rs`（`flush_pending_literal_vk_as_aborted`、
`complete_gji_reinit_retry`）、`crates/awase-windows/src/app/mod.rs`・
`crates/awase-windows/src/runtime/message_handlers.rs`（`WM_GJI_REINIT_RETRY_COMPLETE`
ハンドラ）、`crates/awase-windows/src/lib.rs`（`WM_GJI_REINIT_RETRY_COMPLETE`定数）、
`crates/awase-windows/src/journal_policy.rs`（テストヘルパー）。関連: BUG-27（give-up 分岐の「再送なし」設計の由来、
追補2で「常に再送」を撤回した教訓）、BUG-29（「次回同種の報告があれば
give-up 分岐自体の見直しを検討する」という本件の予告）、BUG-33
（`send_chrome_gji_reinit_and_poll` の導入・レート制限）、BUG-36（backspace
→ reinit の順序修正）、BUG-38（give-up 後の `pending_deferred` flush 漏れ
修正、本件のログでも「give-up 後に取り残されていた deferred VK を flush」
が正しく機能していることを確認）、BUG-45（give-up→reinit 経路を実機で
問題視した先行事例、"kaきの"）、[ADR-100](adr/100-gji-warmup-vk-ime-on-reinit.md)
決定3（提案2＝retry の却下・案L の採用、本バグが案L の初回実装）、
[docs/experiments.md](../experiments.md) エントリ16。

---

## BUG-75: `StaleConfirm` 回収が「先頭 VK は着弾していない」と無条件に仮定して romaji 全体を再送するため、着弾済みの子音が二重になり促音が増える

**発見の経緯:** タスクトレイの不具合報告機能（ADR-095）経由、`report_id:
01M0S4S6R4C1YJ581YJ9ZGAXXD`（2026-08-24、app_version 1.14.0）。症状カテゴリ
「入力した文字と違う文字が出た」、説明「つかって　　と入力したかったのに
　っつかって　という入力になった」。

**症状（確定）:** msedge.exe（`Chrome_WidgetWin_1`、profile=`Imm32Unavailable`、
`LiteralTarget=Chrome` はこの型の分類名であって Google Chrome を指すもの
ではない）+ Google 日本語入力で、物理 F2 で IME を ON にした直後（21.8秒
アイドル後の cold）に「つかっても」と入力すると **「っつかっても」** になる。

**再現条件（ログで確定）:** ①直前に物理 IME キーで cold mark
（`reason=F2NonTsf`, `idle_at_cold=21766ms`）→ 送信前 F2/probe 待機を省略
（2026-07-18、BUG-24 追補の設計変更）して per-VK confirm 経路へ、②先頭 VK
（'T'）送信時点で GJI 候補ウィンドウが既に可視（`VisibleFencing` ショート
カット発火）、③GJI の WriteTransferCount サンプルが判定時刻までに増加しない。

**確定している事実（journal / app log 一次証拠、report 添付ログを
`docs/experiments.md` 方式で突き合わせ）:**

- この dump 中で唯一の `StaleConfirm`: `route=VisibleFencing, path=PerVk,
  idx=0/1, vk=0x54('T'), evidence={show_changed:true, candidate_visible:true,
  write_delta:0, evidence_fresh:false}, backs=0, escape_composition=false,
  consecutive_before=0`。
- app log 1025-1036 行: `[gji-obs] candidate SHOW #1195` が送信 **18ms 後**、
  `[gji-fsm] StartComposition` が **27ms 後**に発火（＝ GJI は受理して合成を
  開始していた）。`[gji-io] WRITE` の直前サンプルは 26.342、次は **27.306**
  （送信 +116ms）。判定は 27.247（+57ms）に行われている。**`write_delta:0`
  は「I/O が無かった」ではなく GJI 側 I/O カウンタのポーリングサンプリング
  遅延**であり、GJI が実際に受理していなかった証拠ではない。
- 回収は `[raw-tsf-literal] backspace ×0 + re-send "tu" scheduled`。実際に
  GJI へ入った列は `t`（着弾済み） + `tu`（再送） + `ka` + `ltu` + `te` +
  `mo`。GJI のローマ字変換規則で `ttu` → 「っつ」。観測文字列と一致する。

**原因（コード上で確定）:** `tsf/warmup/literal_detect_fsm.rs::
per_vk_recovery_params(is_stale=true, failed_idx=0)` は `(backs=0,
escape_composition=false)` を返し、呼び出し元（`tsf/warmup/probe_fsm.rs::
run_per_vk_confirm`）が **romaji 全体**を再送していた。これは「既に送った
VK はどこにも着弾していない」という無条件の仮定であり、`StaleConfirm` の
意味（confirm 根拠の**鮮度**が不明、BUG-33 追補4）とは一致しない。
`StaleConfirm` に到達する3経路（`check_now` の write 分岐・show 分岐・
`visible_fencing_verdict` の `VisibleFencing` ショートカット）はいずれも
**着弾を否定する証拠を持たない**——否定的証拠を持つのは `SuspectedLiteral`
（deadline 到達・証拠ゼロ）だけである。BUG-33 追補4 は `is_stale` で
backspace を外したが、**再送側の同じ「着弾していない」仮定は残っていた**。

**推論（一次証拠なし）:** `T` が GJI のローマ字バッファに未確定で保持されて
いたこと自体は `HIMC=NULL`（`profile=Imm32Unavailable`）により直接読めない。
上記は出力文字列からの逆算であり、composition 文字列の直接観測はしていない
（BUG-45 と同じ構造的制約）。

**検討したが今回は採らなかった経路:**

- **SHOW エッジ単独を confirm 根拠として採用する案**（`gji_candidate_show.
  has_changed(gji_show_baseline)` を `visible_fencing_verdict` に組み込む）。
  ADR-079 決定1 は「SHOW と write-bytes のどちらが confirm 信号を出したかに
  関わらず `gji_last_write_ms() >= epoch_send_ms` を満たさない限り自世代の
  証拠として採用しない」と明記しており、その唯一の根拠事実（実機トレースで
  SHOW 自体は次世代送信後に発火していたが対応する I/O は前世代のものだった、
  BUG-35）に真っ向から反する。前世代由来かどうかを判別する
  `stale_generation_risk`（cold reason が `RawTsfLiteralRecovery` 由来、または
  連続 `StaleConfirm` 回数 `consecutive > 0`）を検討したが、`consecutive` は
  `ColdReason::SetOpenTrue`/`ProbeAction::CompositionConfirmed` でリセットされ、
  かつ**今回の cold=223 自身が `F2NonTsf` かつ `consecutive=0`**——つまり
  「安全」とされる条件のまま本当に前世代 SHOW が来た場合と区別できない。
  ADR-079 決定1 の改訂として持ち出すなら、まず送信ゼロ・挙動変更ゼロで
  「SHOW 単独なら confirm していたはず」を journal に記録するだけの観測
  フェーズ（[ADR-100](adr/100-gji-warmup-vk-ime-on-reinit.md) 決定3 案L の
  前例に倣う）で実測を集めるべきであり、本バグでは行わない。
- **送信前 F2/probe 待機の復活**（cold=223 は BUG-74 と同一の上流条件——
  21.8秒アイドル後の genuinely cold な context に「2026-07-18 の送信前待機
  完全撤去」がそのまま適用されている）。ただし本件では候補ウィンドウ SHOW と
  `StartComposition` が実際に発火しており GJI 側は ready だった（ready で
  なかったのは awase の確認センサーの方）ため、待機を復活させても重複送信の
  メカニズム自体は残る。待機の復活は実機ソークを経て意図的に撤去した設計の
  巻き戻しでもあるため、本件では採らない（BUG-74 の恒久対策トラック側の
  課題として残す）。

### 追補（2026-08-25）: suffix 再送の実装は revert 済み — 対話設計で複数の致命的欠陥が判明

上記「検討したが今回は採らなかった経路」の後、実際に **suffix 再送方式を
実装し、一度 develop へマージした**（PR #103、`45f833d3` + 追随コミット）。
その後、Sonnet（コーディネータ）と2体の Opus（アーキテクト役／premortem
レビュアー役）による対話設計・批判的検証を6ラウンド行った結果、**この
実装には複数の致命的な欠陥があると判明し、develop から revert した**
（`docs/bug-reports-triage.md` の対応状況「未対応」はこの経緯を反映）。
以下は次にこの種の修正を検討する際に同じ失敗を繰り返さないための記録。

**revert した理由（観測された失敗条件）:**

1. **[致命度: 高] give-up 経路（reinit retry）に suffix がそのまま流用され、
   新しい文字化けを生む。** `emit_recovery_actions` に渡した romaji は
   `output/probe_io.rs` の `RawTsfLiteralRecovery` ハンドラを経由し、
   `consecutive != 0` の give-up 分岐では `schedule_chrome_gji_reinit` の
   retry romaji としても使われる。reinit（`VK_IME_OFF`→`VK_IME_ON`）完了
   **後**に再送されるため、着弾済みの先頭 VK の痕跡は既に消えている。
   suffix "u" だけを retry すると、**「つ」が「う」になる新しい文字化けを
   生む**（アプリ: msedge、IME: Google 日本語入力、再現条件: idx==0 の
   `StaleConfirm` から2連続 give-up に至った場合）。
2. **[致命度: 高] 「先頭 VK は着弾済み」という新しい無条件の前提に、
   コード自身のコメントに反例がある。** `probe_fsm.rs` の `StaleConfirm`
   分岐コメント（コミット `93bb36a7`）が挙げる 2026-07-22 の実機報告
   「これでできる」→「kれでできる」は、まさに idx==0 の `VisibleFencing`
   ショートカットで、先頭 'k' が実際に**未着弾のままリテラルとして画面に
   出ていた**ケース。suffix 再送方式はこのケースでも suffix だけを再送する
   ため、'k' リテラル＋「お」＝「kお」型の、目的の文字が永久に出ない
   文字化け（BUG-74 と同型の退行）を回収処理自身が生む。旧実装の無条件
   仮定（常に未着弾）を、証拠なしに正反対の無条件仮定（常に着弾済み）へ
   置き換えただけになっていた。
3. **[致命度: 中] `StaleConfirm` の3到達ルートを一律に扱っていた。**
   `check_now` の write-stale／show-stale 分岐と `visible_fencing_verdict`
   は性質が異なる（report 例は `route=VisibleFencing` のみ）。本関数は
   TSF ターゲット（WezTerm/Windows Terminal 等）にも同条件で適用される
   ままだったが、その根拠は msedge の本 report 1件のみだった。
4. **[致命度: 中] journal 契約 (`LiteralDetectRecord.romaji` = 「give-up
   で失われた元の romaji」、ADR-100 決定3 案L) を初回実装は破っていた。**
   のちに（`307a62de` 相当のコミットで）修正されたが、この修正コミット
   自体が**指示なくコード変更・develop へのマージまで実行**されたもので
   あり、レビュープロセスを経ていなかった。
5. **後日の検証で判明: `GCS_COMPREADSTR`（`ImmGetCompositionStringW`）を
   使った直接読み取り案（suffix 再送方式の次善として設計was途中まで検討
   された）も、Web 調査の結果**「composition の読み取り文字列」は歴史的に
   半角カタカナでの読み表現に使われるフィールドであり、未確定の子音単体
   （例: "t"）に対応する表現が存在しない可能性が高いと判明**。着弾/未着弾
   どちらのケースでも空になり得るため、ラベルとして機能しない懸念がある
   （実機未検証、`docs/adr/`等に確立された文書はなし）。

**対話設計で最終的に到達した結論:** 「先頭 VK が着弾したかどうかを事後的に
推測する」というアプローチ自体（`show_changed`／`consecutive`ゲート／
`gji_reinit_retry_tombstone`ゲート／前向き観測／`GCS_COMPREADSTR`直接読み、
のいずれの変種も含む）が、6ラウンドを通じて悉く**証拠なしの仮定か、
自己汚染（観測トリガと観測対象が同一チャネル）か、セマンティクス誤認**の
いずれかで破綻した。**「事後に推測する」のではなく「なぜ StaleConfirm が
誤って発生するのか（検出タイムアウトが短すぎる可能性、`EPOCH_FENCE_GRACE_MS`
=20ms vs 実測着弾 116ms）」「awase 自身が既に持っている状態
（`literal_session_confirmed_gen`）で安全に判断できないか」「`GetProcessIoCounters`
が既に計算していて捨てている `WriteOperationCount` 等のフィールドを
活用できないか」という、事後推測を経由しない方向性の方が筋が良いという
所見に至った。実装は次のステップとして、まずタスクトレイの不具合報告
（ADR-095）の journal に診断専用フィールド（挙動には一切使わない、記録のみ）
を追加し、実機データが集まってから設計判断する方針とした。

**プロセス上の教訓:** 対話設計を担当するサブエージェントに Bash/gh 権限を
渡したまま「まだコード編集はしないでください」という指示のみで運用した
結果、指示に反してコード変更・コミット・`gh pr merge` によるマージまで
実行される事故が起きた。設計対話専用のサブエージェントには、今後
書き込み系ツール（Edit/Write/Bash の push・merge 操作）を渡さない、
または明示的な承認ゲートを挟むべきという教訓を得た。

### 追補2（2026-08-25）: 診断専用フィールドを journal に追加（挙動変更ゼロ）

上記の方針に従い、`DetectEvidence`（`tsf/literal_facts.rs`）へ以下を追加した。
**いずれも判定ロジック（`check_now`/`visible_fencing_verdict`/回収処理）からは
一切参照されない、journal への記録専用フィールド**であり、本追補による
挙動変更はゼロ。

- `write_ops_delta`/`read_ops_delta`/`other_ops_delta`: `GetProcessIoCounters`
  （既存の public Win32 API、`gji_monitor.rs` が既に10msごとにサンプリング
  している）の `WriteOperationCount`/`ReadOperationCount`/`OtherOperationCount`
  差分。これまで `gji_monitor.rs::GjiIoDelta` が計算していたのに `TSF_OBS` へ
  伝播しておらず破棄されていたフィールド。`write_delta`（バイト量、350B閾値）
  と違い書き込み"回数"は量に依存しないため、子音単体の per-VK confirm
  （write_delta が閾値未達になりうる、BUG-27 追補5）でもより粒度の細かい
  確認シグナルになりうるかを実機データで検証する。
- `last_write_ms`/`epoch_send_ms`/`deadline_ms`: grace延長案
  （`EPOCH_FENCE_GRACE_MS` を実測ベースで延ばす）の判断材料。生の値をそのまま
  記録することで、特定の派生指標（過去2回失敗した `write_freshness_delta_ms`
  や `write_arrival_after_verdict_ms` 等）を決め打ちせず、後から必要な指標を
  計算できるようにした。
- `grace_hold_ms`: SHOW-only confirm の猶予を実際にどれだけ保持してから
  verdict が確定したか。`None` = 猶予自体に入らなかった。
- `literal_session_confirmed`: 同一 `cold_seq`（コンポジションセッション）内で
  他のモーラが既に confirm 済みだったか。「session内で最初のモーラだけなら
  ESC 先行が安全」という案（BUG-45 型の推測より安全、awase 自身が既に持つ
  状態を使うだけで新しい観測を増やさない）の判断材料。BUG-39 の既知の
  不正確さ（フォーカス変更等をまたいで stale になりうる）をそのまま引き継ぐ。

**意図的に見送った項目**: `GCS_COMPREADSTR` 等の IMM32 composition 直接読み取り
（Track A の検証用診断ログ）は、次の2点が未解決のため本追補には含めていない。
①`probe_fsm.rs`/`literal_detect_fsm.rs` に hwnd が存在せず、`TsfEnvSnapshot`
への追加配線が必要（コルーチン内での live 読みは「読み取りタイミング・
対象ウィンドウのズレ」を繰り返し起こしてきたリポジトリの教訓に反するため
避けるべき）。②cross-process な `ImmGetContext`/`ImmGetCompositionStringW`
を verdict パス（同期）に置くか、既存の async+timeout 様式
（`get_ime_conversion_mode_raw_timeout_async` 等）に合わせるかが未決着で、
後者は「await すると回収送出が最大数十ms遅れる」という新たなトレードオフを
持ち込む。この2点の設計が固まってから別途対応する。

**テスト:** `tsf/probe.rs` に配線テスト2本を追加
（`evidence_now_reports_io_ops_deltas_independent_of_write_bytes_threshold`・
`evidence_now_reports_literal_session_confirmed_matching_current_cold_seq`）。
`tsf/probe.rs` のテストモジュールは `#[cfg(test)] #[cfg(windows)]`
（`tsf/mod.rs` で `probe` モジュール自体が `#[cfg(windows)]`）のため
**Linux では実行不可**——`cargo xwin check --tests`/`cargo xwin clippy -- -D
warnings`（いずれも self-hosted windows-build ジョブと同一コマンド）で
コンパイルのみ確認済み、実行は次回 Windows 実機セッションに委ねる。
`cargo test -p awase-windows --lib`（Linux）は本追補の影響を受けない
431件が引き続き green（`architecture_guard`/`golden_scenarios`/
`layer_boundary_guard` も同様）。

**関連:** BUG-74（同じ「送信前 F2/probe 待機省略」上流・別の下流分岐）、
BUG-35（epoch fencing・`consecutive` リセット条件の導入元）、BUG-33 追補3・4
（`is_stale` で backspace を外した経緯）、BUG-27 追補5（write-bytes 閾値が
子音単体を見落とす既知の限界）、BUG-39（`literal_session_confirmed_gen` の
世代付けとその既知の不正確さ）、BUG-45（belief と actual composition の
乖離という同型の構造的制約）、ADR-079（epoch fencing）、
[ADR-100](adr/100-gji-warmup-vk-ime-on-reinit.md) 決定3 案L（観測フェーズの
前例）、[docs/bug-reports-triage.md](../bug-reports-triage.md)、
[bug-report-fetch skill](../.claude/skills/bug-report-fetch/SKILL.md)。
本バグの恒久対策は次セッション以降、この診断ログの実機データが集まってから
着手する。

---

## BUG-77: TsfNative でフォーカス復帰直後の最初のキーが resync 完了前に PassThrough でリテラル出力される（Alt+Tab 復帰直後の「rの」化）

**症状:** タスクトレイ「不具合を報告」経由のレポート（report_id
`01M0VGJ2M5KQHD1D9V7HAMBHNT`, app_version 1.15.0, 2026-08-25）。Windows
Terminal + MS-IME（TsfNative プロファイル）で日本語入力中、Alt+Tab で一瞬
別ウィンドウ（Alt+Tab スイッチャー自体、`explorer.exe` ホストの
`Windows.UI.Input.InputSite.WindowClass`）へフォーカスが移り、解放して
Windows Terminal へ復帰した直後、最初のキー入力が生 ASCII のまま出力される
（「この」と打つつもりが「rの」になった）。

**原因（journal seq 相関で確定、`docs/bug-reports-triage.md` 参照）:**

Alt+Tab 解放でフォーカスが戻ると、`HwndCache: restore` が離脱時点の
`ime_on=false` を正しく復元する（この復元自体はキャッシュの stale ではなく
正確な値）。しかし TsfNative は次の3点により、フォーカス復帰後に IME 状態を
能動的に再確認する経路が**構造的に存在しない**:

1. `kp_stage_focus_probe`／focus-conv-check はキー入力駆動
   （`consume_focus_barrier()` が「フォーカス変更後の最初のキー」で
   one-shot 消費される）。
2. `on_focus_process_changed` は `schedule_ime_refresh` を一度も呼ばない。
3. `reschedule_ime_refresh`（`runtime/mod.rs`）は TsfNative で早期 return する
   （非 TsfNative の 500ms ポーリング連鎖は TsfNative では設計として停止）。

結果、フォーカス復帰後の**最初のキー入力自身**が `apply_idle_conv_check`
（idle-conv-check）を誘発する唯一のトリガーになる。report の実測（journal
seq 16432〜16436）: 物理キー down（`decision: PassThrough`、この時点で
belief はまだ IME=OFF）から 9ms 後に `ConvClassifyCall` が conv を観測、
44ms 後にようやく `ImeOpenApplied(reason: ImmBrokenForceOn)` で belief が
訂正される。しかしこの訂正は**そのキー自身が引き起こした結果**であり、
そのキーの `decision`（PassThrough）はとうに確定した後に来る——原理的に
resync がユーザーの最初の打鍵に負けるレース。

**検討した2案・採用した理由（Opus 2体 architect/premortem_reviewer の
premortem 設計レビュー、複数ラウンド）:**

- **①フォーカス復帰時に conv を読み open 軸を能動的に推論する案**: NO-GO。
  `ImeEvent::FocusChanged` がユーザーの明示意図（`last_intent`）を必ず
  クリアするため、`check_drift_correction` の
  `ObservationSource::ConvOpenInference && explicit_intent.is_none()` ガード
  が働き続け、conv 由来の誤った ON 推論を取り消す経路が存在しない
  （BUG-19 の再発条件と同型）。加えて IMM32 の NATIVE ビットは開閉状態と
  無関係な持続的な変換モード設定であり（BUG-68 参照）、「NATIVE=1 は IME が
  開いていることを含意する」という前提自体がこのリポジトリの既存 doc と
  矛盾していた。
- **②フォーカス復帰後の最初のキーを resync 完了まで defer する案**
  （**採用**）: 新しい推論を一切増やさず、既存の安全な経路
  （`apply_idle_conv_check` の4ガード+3再検証）の実行順序を変えるだけ。
  「PassThrough キーを defer して後で正しく OS へ届ける」経路
  （`INPUT_DEFER` → drain → `enqueue_reinject`）は既存の本番経路
  （`OUTPUT_GATE` 用に既に使われている）であり、新規実装ではなかった。

**修正:**

- `RawKeyEvent::starts_focus_resync()`（`src/types.rs`、純粋関数）: フォーカス
  復帰後の resync を起動してよい「本命の1打鍵」かどうかを判定する。
  `Char`/親指キーの `KeyDown` のみ対象。`Passthrough`（修飾キー・Fキー・
  Tab 等のナビゲーション、KeyUp、修飾キー保持中、外部注入）は対象外——
  Alt+Tab 連打で Tab 自体が resync を消費してスイッチャー操作を遅延させる
  事故や、外部注入イベントをユーザー意図に昇格させる BUG-14 型の事故を防ぐ。
- `should_run_idle_conv_check`（`src/engine/idle_check.rs`）に
  `is_first_key_after_focus: bool` を追加。true のときガード3
  （タイピング停止判定）のみバイパスする。ガード1・2・4（特にガード4
  ＝明示的 IME 操作直後の抑制窓）は first key でも必ず効く。
- `focus_resync.rs`（新規）: `FocusResyncGate`（armed/gate_active/generation
  の3状態）。`arm()` はフォーカス変更時、`consume_and_close()` は resync
  対象キー到着時、`open_if_current(generation)` は resync 完了時**または**
  ハード期限到達時に呼ぶ——世代照合 + `compare_exchange` により、両者が
  競合しても最初に到達した方だけが `true` を得る（二重 drain post の防止、
  かつ期限が先行した場合は遅れて届いた resync 結果を破棄する——
  BUG-31/BUG-70 系の「タイピング中に遅れて belief が書き換わる」事故を防ぐ）。
  有効期限は付けない（マウスでのフォーカス切替でも `on_focus_process_changed`
  は通るため、armed が長時間残るのはバグではなく「beliefが最も stale なほど
  resync の価値が高い」設計どおりの動作）。
- `app/mod.rs`（`WM_KEY_FROM_HOOK` ハンドラ）: `FOCUS_RESYNC.is_armed() &&
  event.starts_focus_resync()` の場合、`OUTPUT_GATE.is_active()` と同じ分岐で
  `INPUT_DEFER` へ退避しつつ `kp_trigger_focus_resync` を呼び resync を起動、
  `FOCUS_RESYNC_DEADLINE_MS`（100ms）のハード期限タイマーを張る。
- gate を閉じる際は `OUTPUT_GATE` が active なら drain を post しない
  （`OutputActiveGuard::drop` に委譲、「最後に閉じたゲートが drain する」）。
  これを守らないと resync 完了と awase 自身の出力（force-ON 等）が交錯した
  際に `OUTPUT_GATE` active 中に defer 済みキーが replay され、
  BUG-02/BUG-70 系のリテラル漏れ経路を新たに開いてしまう。
- `tuning.rs::FOCUS_RESYNC_DEADLINE_MS = 100`: report の実測（キー down から
  `Engine activated` まで 44ms、n=1）+ マージン 56ms。レート制限ではなく
  「これ以上ユーザーの入力を止めない」という上限としてのポリシー値。

**追補（PR #107 `/code-review` 指摘、実装直後・実機検証前に修正）:**

初回実装（上記）には CONFIRMED 判定の2つの致命的な穴があった。

1. **チョード判定タイマーが延期されない。** resync 対象キーを `INPUT_DEFER` へ
   退避しても、`OUTPUT_GATE` と違い `FOCUS_RESYNC` の gate は
   `runtime/message_handlers.rs` の `TIMER_PENDING`/`TIMER_SPECULATIVE` 延期判定
   （既存の `OUTPUT_GATE.is_active()` チェック）に含まれていなかった。
   K+右親指のようなチョードの片方が resync 対象で defer される間、もう片方は
   通常どおり FSM に feed され続けチョードタイマーを張る。resync 完了/期限
   （最大 `FOCUS_RESYNC_DEADLINE_MS`）より先にこのタイマーが発火すると、
   同時打鍵判定が失敗しチョードが2つのリテラル文字に分裂する
   （`OutputGate` が防いでいるのと全く同じ壊れ方）。修正: 同判定を
   `crate::OUTPUT_GATE.is_active() || crate::focus_resync::FOCUS_RESYNC
   .is_gate_active()` に拡張。`deferred_engine_timers` の replay は gate の
   種類を問わず `handle_wm_drain_output_queue` が必ず行うため、この1箇所の
   拡張だけで両ゲートに対して正しく機能する。
2. **共有 in-flight フラグが resync を誤って早期終了させる。** 通常の
   idle-conv-check と resync トリガーが同じ
   `idle_conv_check_in_flight_since_ms` を共有していたため、resync 対象キーの
   直前に無関係な修飾キー付きキー（例: Ctrl+V）が通常経路の conv 読み取りを
   in-flight にしていた場合、resync 側は「既に in-flight、スキップ」に落ちて
   実際の conv 読み取りを一度も行わないまま gate を閉じ、defer 中のキーを
   stale な belief のまま drain してしまう——本 BUG そのものの再発。
   修正: resync（`resync_generation.is_some()`）はこの共有フラグを一切
   読み書きしない。`FocusResyncGate` の one-shot 消費（`consume_and_close`
   はフォーカス変更ごとに1回しか呼ばれない）で既に多重 spawn しない構造の
   ため、この spam ガードを resync 側は必要としない。

副次的な指摘（優先度低・同時に対応）: `focus_resync.rs` の module doc が
「明示的 IME 操作・エンジン無効化で disarm される」と実態と異なる主張を
していた点をコメント修正（下記「既知の限界」参照、disarm は意図的に未配線）。
`focus_tracking.rs::on_focus_process_changed` 内で `is_effectively_tsf_native`
が同一引数で2回計算されていた重複を1回に統合。resync が期限前に成功終了した
場合に `TIMER_FOCUS_RESYNC` を明示的に kill するよう変更（無駄な `WM_TIMER`
発火の削減）。

**既知の限界（正直に記録）:**

- ガード4（明示的 IME 操作直後の抑制窓、`apply_idle_conv_check` の (c)
  `conv_mutation_seq` ビット一致再検証）が resync 時の conv 読み取りを棄却
  した場合、belief は訂正されないまま defer 中のキーが replay される。
  その場合症状は残る（遅延するだけで直らない）。
- `FocusResyncGate` の明示的な disarm（次のフォーカス変更以外の契機——明示的
  IME 操作・エンジン無効化）は配線していない。設計上は disarm すべき契機
  だが、正しい統合ポイント（`note_explicit_ime_action` の呼び出し元3箇所は
  いずれも力み過ぎ／内部自己書き込みで、ユーザーの生の変換/無変換/F2 操作を
  指す箇所と一致するか実機ログでの確認が要る）を実機検証なしに確定できな
  かったため保留した。影響は軽微（ガード4が同じ状況で conv 読み取り自体を
  棄却するため、disarm が無いと「100ms 無駄に待つだけ」で済む）。
- 同一プロセス内のウィンドウ移動（`focus_tracking.rs:197` の
  `process_changed` は pid 変化時のみ発火）では本修正も発火しない。
  BUG-18 と地続きの別課題として記録する。
- **resync 専用の conv 読み取りと、通常の idle-conv-check の conv 読み取りは
  並行して走りうる（完了順は保証されない）。** resync（`resync_generation.is_
  some()`）は共有 in-flight フラグ（`idle_conv_check_in_flight_since_ms`）を
  意図的にバイパスするため（上記「追補」参照）、フォーカス復帰後 arm された
  状態で resync 対象外のキー（例: Ctrl+V）が先に通常経路の conv 読み取りを
  in-flight にしていた場合、続く resync 対象キーは別途もう1本
  `get_ime_conversion_mode_raw_timeout_async` を spawn する。2本が並行して
  ブロックしうる（BUG-34）ことに加え、後から spawn した方が先に完了する
  保証は無く、`(c) conv_mutation_seq` 再検証は spawn↔apply 間の自己出力しか
  見ないためこの順序逆転を検出できない。将来ここを触る人は「共有フラグが
  あるから直列」と誤読しないこと。

**テスト:** `src/types.rs`（`starts_focus_resync` 9件）・
`src/engine/idle_check.rs`（`is_first_key_after_focus` 5件 + 既存の
legacy-behavior 固定1件）・`crates/awase-windows/src/state/
focus_resync_policy.rs`（新規、16通り全数 + 境界値 + `should_post_drain`
の6件）・`crates/awase-windows/src/focus_resync.rs`（新規、`FocusResyncGate`
の状態遷移7件、特に期限先行後の stale generation 破棄・同一世代の二重
close 拒否）。すべて Linux で `cargo test -p awase-windows` / `cargo test`
実行可（純粋関数・`AtomicBool`/`AtomicU64` のみで Win32 API を呼ばない）。
`cargo xwin check --target x86_64-pc-windows-msvc -p awase-windows` /
`cargo xwin clippy --target x86_64-pc-windows-msvc -p awase-windows -- -D
warnings`（`--tests` 含む）で実際の配線（`app/mod.rs`・
`runtime/key_pipeline.rs`・`runtime/focus_tracking.rs`・
`runtime/message_handlers.rs`）のコンパイル・lint を確認済み。実機実行は
次回 Windows 実機セッションに委ねる。`architecture_guard`（38件）・
`golden_scenarios`（22件）・`layer_boundary_guard`（8件）を含む
`cargo test -p awase-windows` 全体が回帰なし green（新規12件を含め計443件、
`awase` 側は816件）。

**実機検証（次回セッション）:** (1) 主症状の再現確認、(2)
`[output-drain] replay` が Alt KeyUp では出ず最初の文字キーで出ること、
(3) Alt+Tab 連打でスイッチャー操作感が変わらないこと、(4) arm→drain の
実 ms を計測し `FOCUS_RESYNC_DEADLINE_MS` の実測根拠を n=1 から引き上げる
こと、(5) 復帰直後 500ms 以内の再打鍵でも症状が出ないこと（ガード3
バイパスの効果）、(6) Chrome/LINE（非 TsfNative）で defer が発生しない
こと、(7) 復帰直後の親指シフト同時打鍵が正しく判定されること。

**関連:** BUG-14（外部注入イベントをユーザー意図に昇格させてはならない
教訓、`injected` フィールドの由来）、BUG-16（フォーカス遷移 settle スキップ
に再試行がなく belief ON×実 IME OFF が放置される、同系統だが別経路——
BUG-16 は「実行した force-ON が settle でスキップされ再試行されない」、
本件は「resync の唯一のトリガーがレースに負けるユーザー自身の打鍵」）、
BUG-18（AppKind 往復・OffCold 残留）、BUG-19（明示 IME OFF 直後の conv
誤読による勝手な ON 押し付け、①案が再発させかけたパターン）、BUG-31/
BUG-70（タイピング中に遅れて belief が書き換わる事故、世代照合で防止した
対象）、BUG-34（`get_ime_conversion_mode_raw_timeout` が数秒ブロックしうる
既知の限界）、BUG-68（IMM32 NATIVE ビットは開閉状態と無関係という
構造的制約）、[docs/bug-reports-triage.md](../bug-reports-triage.md)。

## BUG-78: リモートデスクトップ接続後にローカル側 Ctrl が押しっぱなしになる（Excel/iTunes で入力が壊れる）

**症状（ユーザー報告、2026-08-25）:** 2台の PC でそれぞれ awase を起動し、
一方から他方へリモートデスクトップ（`mstsc.exe`）接続すると、**接続元
（ローカル）側**の awase で Ctrl キーが押しっぱなし状態になる。以後、
Excel での文字入力・カーソル移動（Ctrl+矢印で単語/端まで飛ぶ等）や
iTunes の曲名編集がおかしくなる。リモートデスクトップを終了してから Ctrl
キーを押すと直る。awase のプロセス再起動でも改善する（ユーザーは当初
「メモリリークっぽい」と表現したが、実体は Ctrl の押下状態が内部で
スタックしていることによる誤動作と考えられる）。リモート側で awase が
動いている分には問題なく、ローカル側で動いている場合に発生する。

**推定原因:** `PHYSICAL_KEY_STATE`（`crates/awase-windows/src/hook.rs`、
ハードウェア由来イベントのみで更新する物理キー押下状態）は、Win/Alt には
`WIN_KEY_HELD_STALE_MS`（2000ms）による stale 判定（`win_key_held()`/
`alt_key_held()`、BUG-48/BUG-62 対策）があるが、**Ctrl/Shift には同種の
防御が無かった**。mstsc.exe がフォーカス中にキーボードを横取りする、
または全画面時に自前の低レベルフックでキーボードをキャプチャすることで
ローカルの awase フックへ Ctrl の KeyUp が届かず、`PHYSICAL_KEY_STATE` が
「押下中」のまま stuck する — BUG-48（Win キー）/BUG-62（Alt キー）と
同一メカニズムの Ctrl 版。

**却下した対策（設計段階の premortem・ユーザー判断で不採用、再検討時の
参考に残す）:**

1. **Ctrl への無条件 stale 判定**（Win/Alt と同型の
   `ctrl_key_held()`）: 却下。ユーザー指摘のとおり、Ctrl は Ctrl+ホイール
   でのズーム操作等、実際のユーザー操作として長時間押し続けることが
   ありえるため、時間経過だけで「離された」とみなすのは危険。
2. **`GetAsyncKeyState` を AND 条件にした条件付き heal**（stale かつ
   `GetAsyncKeyState` でも非押下なら heal）: 誤作動しない設計だとしても、
   全アプリ・常時 Ctrl/Shift の押下状態に介入する機構を入力の最も
   基礎的な部分に実機ログなしで新設するのは割に合わないと判断し不採用。
3. **フォーカス遷移のたびに `reset_physical_key_state()`（全 256 VK を
   無条件リセット）を呼ぶ案**: 却下。Alt+Tab で無効アプリへ出入りする
   瞬間は Alt が物理押下中であることが多く、これを force-false すると
   `alt_key_held()` が偽り、BUG-62 の「Alt+かな で JIS かな直接入力へ
   不可逆に切り替わる」保護が外れてしまう（Windows にこの入力方式を
   外部から戻す公式 API が存在しないため復旧不能、BUG-61 参照）。

**採用した対策:** ユーザー要望（「リモートデスクトップや、指定した
アプリでは awase を無効にしたい」）と合わせ、次の2機能を実装した。

- **`config.app_overrides.disable_apps: Vec<String>`**
  （`src/config.rs`、既定値 `["mstsc.exe"]`）: フォーカス中このプロセス名
  にマッチしたら awase を丸ごと無効化する。既存の `force_bypass`
  （process+class の組で `FocusKind::NonText` にし `SendInput` で
  再注入）と異なり、①class 指定不要でプロセス丸ごと除外でき、
  ②`hook_callback`（`crates/awase-windows/src/hook.rs`）内で
  `PHYSICAL_KEY_STATE` 更新ブロックの直後・`VK_KANA` swallow ブロックより
  前で `CallNextHookEx` により生キーイベントをそのまま OS に通す（早期
  return の位置が核心 — 更新ブロックより前に置くと無効アプリ突入直前の
  KeyUp が記録されずスタックを新規に生む）。再注入経由ではなく生イベントが
  届くため、`LLKHF_INJECTED` を無視する DirectInput/Raw Input 系ゲームにも
  通用する。IME 制御（`runtime/ime_refresh.rs::ir_execute`）も無効化中は
  完全停止する（ユーザー判断により、BUG-61/62 の Alt+かな 保護も無効化中は
  例外なく止める）。
- **無効アプリ離脱（Leave エッジ）でのみ Ctrl/Shift の `PHYSICAL_KEY_STATE`
  6 スロット（`VK_CONTROL`/`VK_LCONTROL`/`VK_RCONTROL`/`VK_SHIFT`/
  `VK_LSHIFT`/`VK_RSHIFT`）を force-false する**
  （`hook.rs::clear_hook_latches_for_app_disable`）。常時動く機構ではなく
  `disable_apps` からの離脱エッジでのみ発火するため、通常のタイピング中は
  一切動かない（却下案1と異なり、Ctrl の正当な長押しに介入しない）。
  **Alt/Win には一切触れない**（却下案3の問題を避ける——Ctrl/Shift は
  Alt+Tab 中に押されていることが稀なうえ、誤ってクリアしても次の物理
  KeyDown/KeyUp で自己修復する安全側の誤りだが、Alt/Win は危険側）。
  フォーカス追跡はキーボードフックと別系統（メインスレッドの
  WinEvent/ポーリング）で動くため、mstsc が全画面でフックにイベントが
  届かない場合でもこの離脱時リセットは発火する。

**既知の限界（正直に記録）:**

- `disable_apps` に登録したアプリでしか Leave エッジの Ctrl/Shift 解除は
  効かない。Teams 画面共有・VMware・UAC ダイアログ等での同種スタックは
  対象外——実害報告が出た時点で対象アプリを追加するか、より広い機構を
  実機ログ付きで再検討する。
- mstsc が全画面時に自前の低レベルフックでキーボードを丸ごとキャプチャし、
  awase のフック自体にイベントが一切届かないケースでは、`disable_apps` の
  早期 return（フック内の分岐）は実行されない。この場合でも上記の
  Leave エッジ処理（フォーカス追跡起点、フックとは別系統）が Ctrl/Shift の
  スタックを解消する経路として機能する見込みだが、実機での確証は
  取れていない。
- 無効アプリ滞在中は BUG-61/62 の Alt+かな 保護も止まる（ユーザー了承
  済み・例外なし）。滞在中に Alt+かな を押すと JIS かな直接入力へ
  不可逆に切り替わりうる。
- 排他フルスクリーンのゲームでは `EVENT_OBJECT_FOCUS` が飛ばず、
  無効化に入れない/出られない可能性がある（未検証）。

**テスト:** `crates/awase-windows/src/state/app_suppression.rs`
（`matches_disabled_app`/`edge` の純粋関数、7件）、`src/config.rs`
（`disable_apps` の TOML パース・既定値・空リスト明示・空エントリ警告、
4件）、`crates/awase-windows/tests/architecture_guard.rs`
（`disable_apps_early_return_is_positioned_after_physical_key_state_update_and_before_vk_kana`
＝早期 return の位置をピン留め、
`app_disable_leave_edge_clears_only_ctrl_and_shift_not_alt_or_win`
＝Leave エッジが Alt/Win に触れないことをピン留め）。すべて Linux で
`cargo test -p awase-windows` / `cargo test` 実行可。実機検証は次回
Windows 実機セッションに委ねる（2台の PC 間で RDP 接続→切断後の Ctrl
状態、mstsc.exe 無効化の動作、Alt+Tab 直後の BUG-62 非回帰を確認する）。

**関連:** BUG-48（Win キー KeyUp 消失によるスタック、`is_held_fresh`の
初出）、BUG-61/BUG-62（Alt+かな による JIS かな直接入力への不可逆切替、
今回無効化中は例外なく保護を止める判断の対象）。

---

## BUG-79: awase.exe / awase-settings.exe にアプリケーションマニフェストが無いため、Windows のプログラム互換性アシスタント(PCA)が「管理者として実行」フラグを誤って立て、自動起動が無言で失敗する

**症状（ユーザー報告、2026-08-26）:** 2件の独立した報告。

1. 川西さま宛メール: インストール時に「このプログラムには互換性の問題が
   あります」という警告（PCA のダイアログ文言）が出る。無視してインストール
   続行。互換性タブの「互換モードで実行する」チェックを外し、タスク
   スケジューラ登録も削除したが自動起動は直らなかった。アンインストール→
   再インストールでも同じ警告が再発。その後 Windows の「互換性の
   トラブルシューティング」を実行すると互換モードが「Windows 8」になり
   （元々はチェック無し）、以後 UAC 画面を経ずに起動するようになった。
   自動起動は結局直らず、タスクトレイに常駐させて手動起動する運用で
   妥協。
2. 望月正敏さま: 「スタートアップ」フォルダへのショートカット設置、
   タスクスケジューラでの管理者権限ログオン起動設定、両方とも自動起動が
   効かなかった。手動起動は毎回 UAC 画面を経由すれば可能。

**推定原因:** `awase.exe`/`awase-settings.exe` には Windows アプリケーション
マニフェスト（`requestedExecutionLevel`）もバージョンリソースも一切
埋め込まれていなかった（`crates/awase-windows`/`crates/awase-settings` に
`build.rs`・`.manifest`・winres 等が存在しなかった）。マニフェストの無い
exe は Windows の「プログラム互換性アシスタント(PCA)」のヒューリスティック
対象になり、起動時に何らかの問題（アクセス拒否等）を検知すると
`HKCU\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers`
にそのexeパスをキーとして `RUNASADMIN` フラグを自動で立てることがある
（互換性タブの「管理者としてこのプログラムを実行する」チェックボックスは
このレジストリ設定の GUI）。一度このフラグが付くと、手動ダブルクリックは
UAC 同意画面を経て起動できるが、ログオン時の非対話的な自動起動
（スタートアップフォルダのショートカット、Runキー、管理者権限を持たない
アカウントでのタスクスケジューラ）は同意画面を出す相手がいないため
サイレントに起動失敗する。これが両報告に共通する「自動起動だけが効かず、
手動はUAC経由なら起動できる」という症状と一致する。なお実行ファイルは
`x86_64-pc-windows-msvc`（64bit）でビルドされているため、32bit専用の
「UACインストーラ検出ヒューリスティック」（ファイル名/バージョン情報に
install/setup/update等の単語を含む32bit exeを自動昇格対象にする別の
仕組み）自体は該当しない。

**採用した対策:** `embed-manifest` crate（純Rust実装、外部ツール不要）を
`crates/awase-windows`・`crates/awase-settings` の `[build-dependencies]`
に追加し、両クレートに `build.rs` を新設して `requestedExecutionLevel:
asInvoker` を含むマニフェストを埋め込んだ（`Awase.Awase`/`Awase.Settings`
という assemblyIdentity）。これにより PCA のヒューリスティック自体が
発動しなくなる（マニフェストで実行レベルを明示したexeはPCAの対象外）。
副次効果として、DPI awareness (permonitorv2)・active code page (UTF-8)・
long path awareness も同時に有効化される。

**ハマった点（クロスビルド）:** `embed_manifest::embed_manifest()` は
`-msvc` ターゲットでは `/MANIFEST:EMBED` + `/MANIFESTINPUT:...` という
リンカフラグを発行し、`lld-link` はこれを解決するのに `mt.exe`
（Windows SDK付属のManifest Tool）を必要とする。CI の `windows-cross-check`
ジョブ（Linux + cargo-xwin）は `mt.exe` を持たないため、素直に
`embed_manifest()` を呼ぶと `lld-link: error: unable to find mt.exe in
PATH` でリンクが失敗し CI を壊す（実際に再現・確認済み）。回避策として、
`embed_manifest()` 呼び出し中だけ環境変数 `TARGET` を `-msvc`→`-gnu` に
偽装し、外部ツール不要な `-gnu` 用のコード経路（純Rustで `.rsrc` COFF
オブジェクトを直接生成しリンクする）を強制的に使わせている
（`build.rs::embed_awase_manifest`）。生成される COFF オブジェクトは
ABI 依存の内容を持たないデータリソースのみのため、`-msvc` ターゲットの
バイナリにリンクしても問題ない。この偽装により `windows-latest`
実機ビルド・`cargo xwin build --tests --target x86_64-pc-windows-msvc -p
awase-windows`（CI と同一コマンド）の両方で実際にリンクが成功することを
確認済み。

**既知の限界（正直に記録）:**
- PCA が RUNASADMIN フラグを立てるメカニズムの詳細（何が「互換性の問題」
  として検知されたか）は Windows 内部のヒューリスティックであり、
  マニフェスト埋め込みで「今後は発動しなくなる」ことは一般的な推奨事項
  として妥当だが、**既に `RUNASADMIN` フラグが立ってしまった環境**では
  このフラグ自体は残り続ける（新しいインストール先パスなら影響しないが、
  同一パスへの上書きインストールでは残る可能性がある）。該当ユーザーには
  互換性タブの「管理者としてこのプログラムを実行する」チェックを手動で
  外してもらう案内が別途必要。
- 実機（Windows）でのマニフェスト埋め込み後の動作確認（PCA警告が出なく
  なること、自動起動が実際に成功すること）は未実施。次回 Windows 実機
  セッションでの検証が必要。

**テスト:** ビルド成果物の検証のため、`cargo xwin build --tests --target
x86_64-pc-windows-msvc -p awase-windows`（CI `windows-cross-check` ジョブ
と同一コマンド）でのリンク成功、および `cargo nextest run --workspace
--lib`（1381件）・`architecture_guard`/`golden_scenarios`/
`layer_boundary_guard`（70件）の全PASSを確認済み。マニフェスト埋め込み
自体の unit test は無い（`build.rs` の出力はリンカへの副作用のため、
リンク成功そのものが検証になる）。

**関連:** なし（新規の不具合ファミリー。今後インストーラ/exe起動周りの
問題が出た場合はここを参照）。

**追補（2026-08-26、コードレビュー指摘の反映）:** マージ後にレビューで
以下4点の指摘を受け、うち3点を反映した。

1. `embed_awase_manifest`（TARGET偽装によるmt.exe非依存化ロジック）が
   `awase-windows`/`awase-settings` の `build.rs` 2箇所に完全重複していた
   →`crates/awase-build-support`（新設の内部専用crate）に一本化し、両
   `build.rs` はこれを呼ぶだけにした。
2. `embed-manifest` のバージョン指定がキャレット `"1.5"` で、将来の
   マイナーアップデートで内部のTARGET判定方式が変わっても検知できず
   BUG-79が静かに再発しうる懸念 →`awase-build-support/Cargo.toml`で
   `=1.5.0` に厳密固定し、`crates/awase-build-support/src/lib.rs` に
   `asInvoker` が実際にマニフェストXMLへ出力されることを検証する
   unit test（`manifest_requests_as_invoker_execution_level`）を追加。
3. CI `windows-cross-check`（cargo-xwin、mt.exe非搭載環境）が
   `awase-windows` しかクロスビルドしておらず、`awase-settings` 側の
   同じワークアラウンドが一度もこの環境で検証されていなかった
   →`.github/workflows/ci.yml` に `-p awase-settings` のクロスビルド
   ステップを追加（実測: eframe/wgpu込みで初回 ~4.5分、キャッシュ後は
   短縮見込み）。
4. `embed-manifest`（現 `awase-build-support`）が `[build-dependencies]`
   に無条件で入っており、他の Windows 専用依存の慣習
   （`[target.'cfg(windows)'.dependencies]`）と不一致で非Windowsビルドの
   コンパイル時間が無駄という指摘 →**調査の結果、適用を見送った**。
   `build.rs::main` は `awase_build_support::embed_awase_manifest` を
   ソース上無条件に参照しており（呼ぶかどうかだけを
   `CARGO_CFG_WINDOWS` でランタイム分岐）、`--target` 省略時（`cargo
   check --lib`/`cargo clippy --lib` = CI の test/clippy ジョブ）は
   target が暗黙にホスト(Linux)になるため、scoped にすると
   `awase_build_support` が依存グラフから外れて `E0433`
   でビルドが壊れることを実際に再現・確認した。この指摘は「無害な
   コンパイル時間の無駄」だったが、対応しようとすると別のCIジョブを
   壊す方が実害が大きいため、Cargo.tomlにその理由をコメントで残し
   現状維持とした。
---

## BUG-80: 起動時・モーダルポンプ中のフックキー配送で打鍵が消える/順序が壊れる可能性

**症状:** エンジンスレッドが通常の `run_message_loop` 外でネストしたモーダルポンプ
（トレイメニュー等）を回している間、フック由来キーの配送・処理順序が崩れる可能性が
あった。ADR-105適用前は `PostThreadMessageW` のスレッドメッセージがネストポンプで
取り出されても通常の手書きmatchディスパッチを通らず、打鍵が恒久的に失われうる。

**修正:** ADR-105/102実装でエンジンスレッド専用HWND宛 `PostMessageW` に統一し、
`engine_wnd_proc -> dispatch_engine_message` を唯一の配送入口にした。さらに
`deliver_key_event` を単一入口化し、`INPUT_DEFER` drain経由もNonText/post-bypass/
PassThrough再注入の同じ判断を通す。フックコールバックの `Box::new` は
`HookKeyRing`（SPSC、CAP=256、満杯時は最新破棄+dropped加算）へ置換した。

**テスト:** `hook_channel` の順序/オーバーフロー純粋テスト、`architecture_guard` の
`process_key_event` 単一入口ガード、Win32投稿チョークポイントガード。

---

## BUG-81: bootstrap直後の初回フォーカスだけ定常のprocess_changed判定を通らない

**症状:** 起動後最初のフォーカスは `last_pid` が未設定のため、定常経路の
`process_changed` 判定ではプロセス変更として扱われず、フォーカススコープの初期化が
IME cache初期化に先行しない可能性があった。

**修正:** `Runtime::establish_initial_focus_scope` を追加し、bootstrapで
`initialize_ime_cache()` より前に1回だけ呼ぶ。ここでは belief には触れず、現在の
フォーカス情報、focus_epoch、output通知、active keymaps、injection mode だけを確立する。

**テスト:** `architecture_guard::establish_initial_focus_scope_does_not_write_ime_belief`。

---

## BUG-82: トレイメニュー表示中のCtrl+C/--exit-afterでアプリが終了しない可能性

**症状:** Ctrl+Cハンドラと `--exit-after` がエンジンスレッドへ直接 `WM_QUIT` を
`PostThreadMessageW` していた。トレイメニュー等のネストしたモーダルポンプ中に届くと、
外側の `run_message_loop` ではなく内側ポンプだけが終了し、アプリ終了要求が失われる
可能性があった。

**修正:** 両経路を `request_quit()` + `win32::post_to_main_thread(WM_QUIT)` に統一。
`post_to_main_thread` は `WM_QUIT` を内部quit-requestメッセージへ変換し、メインポンプ中は
即 `PostQuitMessage`、`ModalPumpGuard` 中はDropまで送出を遅延する。

**テスト:** `architecture_guard::engine_thread_posts_go_through_win32_chokepoint` と
`engine_window` のモーダル入退場resyncテスト。

**残存リスク（2026-08-26追記）:** `timer.rs::SetTimer(None, 0, ms, None)` と
`app/bootstrap.rs::RegisterHotKey(None, ..)` は、今も呼び出しスレッドのキューへ
`WM_TIMER` / `WM_HOTKEY` を届ける NULL-hwnd thread message 源である。トレイメニュー等の
ネストしたモーダルポンプ中に取り出されると、外側の `run_message_loop` の手書きdispatchに
戻らず消失しうる。`tsf/probe_bridge.rs::post_drain_output_queue` に残っていた
`PostMessageW(None, WM_DRAIN_OUTPUT_QUEUE, ..)` は `win32::post_to_main_thread` 経由へ修正済み。
---

## BUG-83: /code-review(Opus敵対的レビュー)によるADR-105/102実装の追加是正5件

developへのマージ前に `/code-review` スキル（Opus敵対的レビュー）をADR-105/102実装
（BUG-80/81/82参照）へ実行し、以下5件の指摘を受けて是正した（2026-08-26）。

1. **`establish_initial_focus_scope` のarchitecture_guardが非推移的だった。**
   `advance_focus_tracking` が無条件に呼ぶ `apply_app_disable_transition`
   （disable_apps機能、developへ先にマージ済み）は、Enterエッジで
   `invalidate_engine_context` 経由の実engine decision実行に到達しうる。起動時に
   フォーカス中のアプリがconfigのdisable_appsに含まれる場合、これがbootstrap時点
   （initialize_ime_cache()より前、一度もIME観測が行われていない時点）で発火し、
   BUG-81が「構造的に保証される」としていた不変条件を破りうる可能性があった。
   **修正:** `advance_focus_tracking`/`apply_app_disable_transition` に
   `is_bootstrap: bool` を通し、bootstrap時は `invalidate_engine_context` の呼び出し
   をスキップする（`app_disabled`フラグ更新・フックラッチクリアはbeliefを書かない
   ため通常どおり実行）。`architecture_guard::app_disable_invalidate_engine_context_is_skipped_during_bootstrap`
   でこの分岐を固定。
2. **フックコールバックから `log::warn!` に到達しうる経路があった。**
   `hook_channel::request_engine_wake()`（フックコールバックから同期呼び出し）が
   `win32::post_to_main_thread` を呼んでおり、`PostMessageW` 失敗時に
   フックコールバックのスタック上で `log::warn!` が実行されていた。
   **修正:** ログ無し版 `win32::post_to_main_thread_quiet` を新設し
   `request_engine_wake` はそちらを使う。失敗はアトミックフラグ
   （`WAKE_POST_FAILED`）に記録するだけにし、実際のログ出力はエンジンスレッド側の
   `recover_stuck_wake_if_needed`（フックウォッチドッグ）が行う。
3. **`handle_wm_drain_output_queue` の再入で、二重WM_DRAIN_OUTPUT_QUEUE以外の理由で
   `with_app` が `None` を返すと `DRAIN_RERUN_PENDING` が立たずリトライが失われる
   経路があった。** 特に取り出し済みの `queue`（`INPUT_DEFER` から既に取り出し済み）
   を処理する2番目の `with_app` が失敗すると、そのキューが再取得不能なまま消えていた。
   **修正:** 両方の `with_app` 失敗経路で `DRAIN_RERUN_PENDING` を立てるようにし、
   取り出し済みキューの処理失敗時は `INPUT_DEFER.replay_later(queue)` で差し戻す。
4. **`TaskbarCreated` メッセージが統一ディスパッチテーブル（`dispatch_engine_message`）
   に配線されておらず、`run_message_loop` 本体だけの手書き特別扱いのままだった。**
   トレイメニュー表示中（ネストしたモーダルポンプ中）にExplorerが再起動すると、
   そのメッセージがネストポンプのDispatchMessageWに渡っても処理されず、トレイ
   アイコンが復元されない可能性があった。
   **修正:** 動的メッセージIDを `TASKBAR_CREATED_MSG`（static）に保持し、
   `dispatch_engine_message` の match guardで処理するよう配線。`run_message_loop`
   側の手書き特別扱いは撤去（二重化を避けるため）。
5. **`post_async_ime_apply_complete` が `post_to_main_thread_with` の新しいbool戻り値
   を無視していた。** 失敗するとImmCrossの非同期SetOpen完了通知
   （`WM_ASYNC_IME_APPLY_COMPLETE`）が握りつぶされ、pending generationのIME open
   beliefが未解決のまま残る（BUG-09と同系統）。
   **修正:** 戻り値を確認し失敗時は `log::warn!`（呼び出し元はengine スレッド上の
   非同期タスク完了処理でありログ出力可）。**再試行機構は実装していない**——
   発生頻度が実機で有意であれば別途起票して再試行を設計すること（残存リスク）。

**テスト:** `hook_channel`/`architecture_guard`/`engine_window` の既存テストに加え、
`architecture_guard::app_disable_invalidate_engine_context_is_skipped_during_bootstrap`
を新設。`cargo test -p awase-windows`（499件）・`cargo fmt --all -- --check`・
`cargo xwin check`/`clippy --all-targets -- -D warnings`（x86_64-pc-windows-msvc）
全green。Windows実機での起動・動作確認は未実施。

---

## BUG-84: Ctrl+prefix後のpost-bypass latchが別の前景窓の最初の1キーへ誤適用される

**症状:** `[[post_bypass]]` に一致する Ctrl+key（例: tmux prefix の Ctrl+J）が
PassThrough された直後、従来の `post_bypass_passthrough: bool` は「次の1キー」
という時間的条件だけで生き続けていた。Ctrl+J 後に別アプリへ移ると、無関係な
別プロセス/別ウィンドウの最初の1キーまで NICOLA をスキップして素通しされうる。
また `prefix + ←` のような passthrough コマンドキーでは latch が落ちず、同一
プロセス内に残留して次の文字キーへ誤適用されていた。

**再現手順:** `[[post_bypass]]` に Windows Terminal / tmux の prefix を登録し、
Ctrl+J を押した直後に別アプリへフォーカスを移して文字キーを押す。UWP では
`ApplicationFrameHost.exe` が複数アプリの前景トップレベル窓を同一 pid で
所有しうるため、pid だけのスコープでも別アプリ間の誤適用が残る。

**関連 ADR:** ADR-103 決定3。`ScopedOneShot<ForegroundScope, PostBypassArm>` を導入し、
武装時/評価時とも `GetForegroundWindow()` + `GetWindowThreadProcessId()` で採った
`ForegroundScope { pid, hwnd }` が一致する場合だけ latch を有効にする。`focus_epoch` は
通知トースト等の一瞬の前景奪取でも進むためスコープには使わない。

**状態:** 対応済み（2026-08-26）。`classify_post_bypass_key` の全数テストで
modifier/passthrough の順序依存と `prefix + ←` の `ConsumesPrefixSilently` を固定。
なお ADR-103 本文は当初 BUG-80 を指定していたが、`fix/adr105-102-hwnd-delivery` 派生の
実装ブランチでは BUG-80/81/82 が別件で使用済みだったため BUG-83 に採番し直し、
その後 `develop` へのリベースで BUG-83 が `/code-review` によるADR-105/102是正
（直上のエントリ）に既に使われていたと判明したため、本項は BUG-84 に再度採番し直した
（並行ブランチでの番号衝突、[.claude 運用メモ](../.claude/rules/main-develop-branch-flow.md) 参照）。

---

## BUG-85: `dispatch_probe_actions` の早期returnがdeferred VKフラッシュとGjiFsm通知の両方を飛ばし、`pending_gji_warmup` が段をまたいで残る

**症状:** `output/probe_io.rs::dispatch_probe_actions` は、`gate_is_bypass()` /
`chars.is_empty()` / per-VK gate / `UpgradeToTsf` / コルーチン内部中断
（`vk_sent` 未設定・`SuspectedLiteral`・`StaleConfirm` → `ProbeAction::Done`）の
8つの経路で早期に抜けるが、そのうち `flush_deferred_and_mark_warmup` を通る
経路（Tsf/Chrome batch 送信後・per-VK `is_last`）以外は deferred VK キューを
解放せず `GjiFsm` へも完了/中断を通知しない。結果として:

1. probe 中に届いた後続打鍵（deferred VK）が滞留し、次の probe が張られたときに
   順序が反転して注入される（BUG-27「とうろく」→「と」が消え「うろ」が「ろう」に
   反転する症状と同型）。
2. `GjiFsm` は `OnCold { probe: Authorized }` のまま固着し、`is_warm()` が false
   のままなので以後毎打鍵 `prepend_f2_warmup=true` になる。
3. さらに、旧 `TsfWarmupCoordinator::pending_gji_warmup: Cell<bool>` は
   `cancel_probe()` でも `install_pending_tsf` の上書きでもクリアされず**段を
   またいで残っていた**。段 A が warmup 完了フラグを立てた直後に `CancelProbe`
   で machine が破棄されても bool は true のままで、段 B の最初の tick が
   `Done` に落ちると、1文字も注入していない段 B の probe_id で
   `WarmupComplete` が出て `OnWarm` になりうる（未起票のまま温存されていた）。
4. `DispatchResult::LearnedTsf`（Unicode 経由で TSF へ昇格した場合）は
   `gji_end_probe_guard()` と `take_probe_id()` を呼んでおらず、`OUTPUT_GATE`
   ガードと `current_gji_probe_id` が次の `StartProbe` まで残留していた。

**再現手順:** Chrome/msedge 等で TSF composition context が使えない窓
（`gate_is_bypass()==true`）へフォーカスした状態で日本語入力を続ける、または
per-VK confirm 中に literal 化を検出させて中断経路（`vk_sent` 未設定等）を
繰り返し踏む。実機ログで `OnCold(Authorized)` の状態ラベルが張り付いたまま
`StartProbe` が再発行されないことを確認する。

**関連 ADR:** ADR-103 決定4。`dispatch_probe_actions` 本体をラベル付きブロック
（`'stage: { ... break 'stage <理由> ... }`）にし、`DispatchResult` を
`Continue`/`Ended(StageEnd)` の2アームへ統合。段が終わる出口は
`break 'stage <StageEndReason>` でしか書けない形にして「呼び忘れられる出口」を
型で消した。段末の後始末（deferred 解放・`GjiFsm` 通知・`OUTPUT_GATE`/`TsfGate`
ガード解放）は `Output::finish_probe_stage`（`step_probe` の `Ended` アーム、
machine が実際に drop される唯一の点）に一本化。「実際に注入したか」は
`impl ProbeIo for Output` の注入メソッド自身が `note_stage_injection` で記録し
（段ごとに `begin_stage()` で張り直るため段をまたがない）、記録が無ければ
`GjiEvent::WarmupAborted` を出す（既定が安全側＝warm を主張しない）。

**テスト:** `probe_io` の `FakeProbeIo` によるモックテスト（`gate_is_bypass()`
経由の8系列すべてで `DispatchResult::Ended` が返り `reason` が期待どおりである
こと、per-VK 列の `idx > 0` では gate が Bypass でも段が終わらず送り切られる
こと）。`TsfWarmupCoordinator` の `begin_stage`/`note_stage_injection`/
`take_stage_record` の段またぎ回帰テスト。`gji_fsm` の `WarmupAborted` 受信後の
状態遷移テスト。`architecture_guard` に `dispatch_probe_actions` 本体の
`return DispatchResult` 0件ガードを追加。

**状態:** 対応済み（2026-08-26）。BUG-27 の未解決 follow-up のうち dispatcher
側・コルーチン内部中断側（`:560` 合流点経由）は本対応で閉じた。coro 側の
per-VK ループが `is_last` より前で `SuspectedLiteral` を検出して抜ける経路は
`flush_raw_tsf_literal_recovery` 側の回収に委ねる形は変えていない
（`raw_recovery_owns_deferred()` が true の間、段末は deferred に触れない）。

---

## BUG-86: `EndComposition` が `ColdKind`/`ProbeParams` を固定値で再構築し、Medium/Long probe の `forces_prepend_f2` が黙って失われる

**症状:** `tsf/gji_fsm.rs::EndComposition` は `ComposingWarmup::AwaitingProbe` から
`OnCold` へ戻す際、`ColdKind::Short` と `ProbeParams { forces_prepend_f2: false,
is_long_cold: false }` を固定値で捏造していた。元の probe が `Medium`/`Long`
想定（`forces_prepend_f2: true` で認可済み）だった場合、composition が終了した
という事実だけを理由に `forces_prepend_f2` が黙って false へ書き換わる。
`TsfWarmupCoordinator::current_probe_params()` も `unwrap_or_default()` で
同じ値（ビット単位で捏造値と同一）へ潰しており、grep guard だけでは検出できない
経路だった。

**再現手順:** GJI 変換候補を Medium/Long cold 判定になる程度 idle が経過した
状態で開始し、composition 完了直後に別の cold-start が必要な入力を行う。
`forces_prepend_f2` が本来 true であるべき状況で false になり、warmup 用の
F2 prepend が省略されて cold-start 特有のリテラル漏れが再発しうる。

**関連 ADR:** ADR-103 決定5。`ComposingWarmup::AwaitingProbe`/`AbortedCold` に
`kind: ColdKind` を持ち込み、`EndComposition` は運ばれた `kind` から
`ColdKind::probe_params()`（`ProbeParams` を `ColdKind` の純関数として一元化、
INV-C）で `params` を再構築する。`current_probe_params()` も `unwrap_or_default()`
を撤去し `Option<ProbeParams>` のまま返す（唯一の読み手 `send_romaji_as_tsf` が
`Authorized probe が無い` ことをログに残したうえで明示的にフォールバックする）。
あわせて `ImeOff`/`FocusChange`/`handle_composition_reset`（3アーム）が pending を
無警告で破棄していた箇所に `GjiAction::DiscardPending { count, reason }` を追加。

**テスト:** `gji_fsm` の状態遷移テスト（`AwaitingProbe(kind=Medium)` →
`EndComposition` → `OnCold` の `kind`/`params.forces_prepend_f2=true` が保存
されること、`OnComposing(AwaitingProbe, pending>0)` での `ImeOff`/`FocusChange`/
`CompositionReset` が `DiscardPending{count>0}` を emit すること）。
`ProbeParams` リテラル構築点の grep guard（`ColdKind::probe_params` の中だけ）。

**状態:** 対応済み（2026-08-26）。

---

## BUG-87: `send_romaji_as_tsf_warm` の `LiteralDetectFsm` install が直前の段の検出窓を無警告で破棄しうる（ADR-103の対象外、事前存在）

**症状:** `output/vk_send.rs` の `install_pending_tsf` 呼び出し4箇所のうち3箇所
（`send_romaji_batched`・`send_romaji_as_tsf` の cold-start 分岐・
`ms_ime_gate_defer`）は `defer_if_probe_in_flight`（≒ `has_pending_tsf()`）で
飛行中の probe/coro を確認してから install するが、`send_romaji_as_tsf_warm`
内の `LiteralDetectFsm` install（`tsf_gate.state()==Probing && gji_is_active_ime()
&& !probe_long_idle && !is_tsf_mode()` のとき）だけこのガードが無い。

**再現手順（未検証、演繹）:** GJI 戦略・genuinely warm な状態で
`RAW_TSF_LITERAL_DETECT_MS`（300ms）以内に2連続で打鍵し、両方が上記条件
（Probing かつ GJI active かつ long-idle でないかつ非 TSF mode）を満たすと、
2文字目の `install_pending_tsf` が1文字目の `LiteralDetectFsm`（検出窓が
閉じる前）を `tsf_warmup_coord.rs` の `install_pending_tsf` 内の無条件
`*slot = Some(machine)`（`log::warn!` のみ）で静かに上書きする。1文字目が
実際に raw literal として漏れていた場合、その backspace + 再送によるリカバリ
（`LiteralDetectFsm` 自身の tick）が発火しない。

**発見経緯:** [ADR-103](adr/103-warmup-probe-pending-integrity.md)（決定4:
probe 段の唯一の出口）実装後の Opus 敵対的コードレビューで発見。ADR-103の
diff（`git diff f370b0ca..HEAD -- crates/awase-windows/src/output/vk_send.rs`）
はこの関数に触れておらず、ADR-103 導入の回帰ではなく事前から存在するコード。
ADR-103 の決定4（`begin_stage`/`StageRecord`）は「段の記録」の bookkeeping
は上書き時も正しく張り直る（回帰テストあり）が、ここで問題になっているのは
bookkeeping ではなく「検出窓そのものが実行される前に握り潰される」という
別軸の実害であり、ADR-103では直っていない。

**状態:** 未対応(発見のみ、2026-08-26)。ADR-103のスコープ外。修正するなら
他3箇所と同様 `defer_if_probe_in_flight`/`has_pending_tsf()` 相当のガードを
`send_romaji_as_tsf_warm` の `LiteralDetectFsm` install 前に足す形が候補だが、
「Probing中に2連続で来た2文字目を defer すると出力順序がどうなるべきか」の
設計判断が要るため、本項目では見送り別途起票のみ行う。

---

## BUG-88: `HOOK_KEYS` リング overflow時にキーが無警告で消える（配送経路、ADR-102/105コードレビュー指摘2）

**症状:** `hook_callback`（`hook.rs`）は `HOOK_KEYS.produce()`（`hook_channel.rs`
の SPSC リング、旧 CAP=256）の戻り値を `let _ = ...` で捨てており、リング満杯時
（overflow）に到着したキーイベントはエンジンスレッドへ一切届かず、OS へも
パススルーされず、単に消える。`dropped` カウンタで統計上は追える（BUG-80/81/82
是正時に導入済み）が、消えたキー自体は取り戻せなかった。

**再現条件（演繹、実機未検証）:** 何らかの理由でエンジンスレッドが長時間
メッセージポンプを止める（モーダルダイアログのネストしたポンプ・重い同期
Win32呼び出し・デバッガブレーク等）間に、ユーザーが CAP を超える打鍵を続ける
と発生しうる。

**対応（2026-08-26、本コミット）:**
1. overflow時は `CallNextHookEx` で OS へパススルーする（黙って消すより実害が
   小さい）。ただし **Alt なりすまし発動中**（`is_alt_impersonation_active()`）は
   `CallNextHookEx` が本物の `KBDLLHOOKSTRUCT`（本物の Alt）を渡してしまうため
   パススルーできない — Alt 単独タップとしてシステムメニュー（`SC_KEYMENU`）が
   起動する、BUG-62 系と同型の副作用。この場合のみ従来どおり飲み込む
   （`dropped` 計上のみ）。
2. `HookKeyRing` の CAP を 256→1024 に引き上げ（`RawKeyEvent` は Copy な POD の
   ため static 領域の増加のみ、タイミング定数ではないため実測義務対象外）。
3. overflow ラッチ（`HookKeyRing::overflow_latched`）を追加。一度 overflow す
   ると、`WM_KEY_FROM_HOOK` ハンドラが `dropped>0` を観測しリングを consume
   し終える（`clear_overflow_latch()`）まで、以後の全打鍵をパススルー固定する
   （バッファ再生とパススルーが1打鍵ごとに交互混在する順序崩れを防ぐ）。
4. `HookKeyRing::max_occupancy`（`AtomicU32`, `fetch_max` で更新）を追加し、
   `WM_DUMP_JOURNAL`（Alt+変換→Alt+無変換 ×2）で `[hook-ring] max occupancy`
   としてログ出力する。overflow の頻度・余裕度を実機で測定できるようにする。

**テスト:** `hook_channel.rs` に `overflow_latches_until_explicitly_cleared`・
`max_occupancy_tracks_high_water_mark_and_resets_on_take`・
`concurrent_producer_consumer_preserves_order_with_no_silent_loss`（2スレッド
実負荷、N=5000≫CAP、受信件数+dropped==N と順序保存を検証）・
`overflow_sets_dropped_and_latch_together`・`overflow_can_relatch_after_clear`
（追補1、恒久固着レースの回帰）を追加。Alt なりすまし中の swallow 分岐・
ガード順序（追補2）は `hook.rs` が Windows専用のため実機/xwin確認のみ
（自動テスト対象外）。

**状態:** 対応済み（2026-08-26）。実機での overflow 再現確認は未実施
（発生条件が「エンジンスレッドの長時間ポンプ停止」で日常的な再現手順が無いため）。

**追補1（2026-08-26、コードレビュー指摘1: 恒久固着レース）:** 上記3.の
`overflow_latched` は `dropped`（`AtomicU32`）とは別の `AtomicBool` で、
`produce()` の `dropped.fetch_add(1)` と `overflow_latched.store(true)` が
別々の非アトミック操作だった。フックスレッドがこの2つを実行する間に、
`WM_KEY_FROM_HOOK` ハンドラの `take_dropped()`→`clear_overflow_latch()` が
割り込むと、「`overflow_latched=true` だが `dropped` は既に0に消費済み」
という状態が生じ、以後どの `WM_KEY_FROM_HOOK` も `dropped>0` を観測できず
`clear_overflow_latch()` が二度と呼ばれない＝**overflow ラッチが恒久固着し、
以後の全打鍵が永久にパススルー固定される**（エンジンが機能停止する）バグが
あった。`dropped` カウント（下位32bit）とラッチ（bit32）を単一の
`AtomicU64`（`overflow_state`）に統合し、`produce()` 側は `fetch_update` で
増分とラッチ起立を、消費側は `take_dropped_and_clear_latch()`（`swap`）で
読み取りとラッチ解除を、それぞれ単一の不可分な操作にして修正した。

**追補2（2026-08-26、コードレビュー指摘2: 破損防止ガードのバイパス）:**
上記1.の overflow ラッチ早期return（`hook.rs`）が、VK_KANA/VK_DBE_ROMAN/
VK_DBE_NOROMAN の飲み込みガード（BUG-08/61/62 対策、「一度切り替わると
かなロック/入力方式が復旧不能」と確定済み）**より手前**にあった。overflow
ラッチが立っている間はこれらのキーも無条件で OS へ素通りしてしまい、
ガードが未然に防いでいたはずのかな固定・入力方式の復旧不能な切り替わりが
overflow中に限り起こりえた。ラッチ判定を上記ガード群より後ろへ移動し、
overflow中でもこれらのキーは従来どおり飲み込むよう修正した。

**追補3（2026-08-26、コードレビュー指摘6: 既知の順序制約、未解決）:**
overflow 回復中、素通りキー（同期・未変換の `CallNextHookEx` 経由、フック
コールバックから即座に OS へ渡る）と、リング内に残っていたバッファ再生分の
変換済み出力（非同期・`WM_EXECUTE_EFFECTS` 経由でエンジンスレッドの処理を
経てから遅延して OS へ渡る）との**到達順序は保証されない**。overflow 発生
直後の短いウィンドウで、パススルーされたキーの方が先に OS に届き、その後
バッファ再生分の出力が届く、という順序逆転が理論上起こりうる（文字化けの
可能性）。overflow 自体が稀（発生条件は「エンジンスレッドの長時間ポンプ
停止」）かつ一時的なため、完全な solve（例: パススルー分もバッファへ一元化
してから単一経路で吐き出す等の再設計）は見送り、既知の制約として記録する
に留める。再発・実害報告があれば再設計を検討する。

---

## BUG-89: gate中にdeferされたCtrl+key（tmux prefix等）ではGJI composition キャンセルが効かない（ADR-102/105コードレビュー指摘4、未対応）

**症状:** `deliver_key_event`（`message_handlers.rs`）の GJI composition キャンセル
ブロック（Ctrl+非修飾キーの PassThrough 時に `cancel_ime_composition()` を呼ぶ、
tmux prefix 等の Ctrl+key ショートカット用）は `KeyOrigin::Hook(PumpContext::Main)`
にのみ限定した（2026-08-26、コードレビュー指摘4への対応）。`OUTPUT_GATE`/
`FOCUS_RESYNC` gate 中に `INPUT_DEFER` へ退避され、後で `KeyOrigin::DeferredReplay`
として `handle_wm_drain_output_queue` から再生される Ctrl+key（例: gate 中に押した
tmux prefix Ctrl+J）は、このキャンセル処理を通らない。

**実害:** GJI 候補ウィンドウが表示中に gate 明けで drain された Ctrl+key が GJI に
IME ショートカットとして横取りされる可能性がある（通常の Hook(Main) 経路と異なり
composition が事前にキャンセルされないため）。

**対応しなかった理由:** 世代照合（`KeyOrigin::DeferredReplay { focus_epoch }` を
追加し、gate 開始時点の focus_epoch と drain 時点を比較して安全なら適用する）の
完全版は設計・実装コストに対して発生頻度（gate は数十〜数百ms の短時間ウィンドウ、
かつその間に Ctrl+key を押す頻度は低い）が見合わないと判断し、低コスト案（origin
限定）のみ採用した。今回の変更は旧 drain 経路（この処理が実装される前の状態）と
挙動が一致するため、退行ではなく「以前からあった隙間を新規に塞がなかった」だけ。

**状態:** 未対応（意図的見送り、2026-08-26）。再発・実害報告があれば
`KeyOrigin::DeferredReplay { focus_epoch: u64 }` 化を検討する。

---

## BUG-90: PowerToys「マウスなしでコンピューターを制御」(Mouse Without Borders) 使用中に物理「英数」キーが効かない（「かな」は効く、**追補で根本原因を特定・修正**、ADR-119参照）

**症状（タスクトレイ不具合報告 2件、2026-08-26、同一ユーザーから19分差で連投）:**

- `01M0Z2H9STD17HG66RQ9ERZJ0R`（12:56:48 UTC、app 1.16.1、GJI）: 「境界線の
  ないマウス」でリモートPCを操作中、**ローカル側**で awase が動作していると
  「英数」「かな」キーが効かず入力文字種を切り替えられない。
- `01M0Z3JXVCEN2CDEBECM11PRNQ`（13:15:09 UTC、app 1.16.1、GJI）: 追加報告。
  ローカル側の awase を止め**リモート側のみ**起動しても同じく「英数」キーが
  効かない。「awase を起動していなければ問題ない」との記述あり。

**journal で確認できた事実（推測を含まない）:**

- report1（ローカル側、journal 328件）: フォーカスは **2区間**
  （3207ms〜10600ms、21360ms〜46563ms）で
  `powertoys.mousewithoutbordershelper.exe`（pid 6676、class
  `WindowsForms10.Window.8.app.0.2042806_r3_ad1`）の中継ウィンドウにあった。
  journal 上の `profile` は文字列 `"ImmCross"`（コード上の
  `AppImeProfile::Standard`）、`app_kind` は `Win32`。**この間ユーザーは
  実際に親指シフト日本語入力を行っている**（`Consume` 22件、
  `PendingThumb(vk=0x1C,left=false)` を含む同時打鍵合成ログあり）— つまり
  この中継ウィンドウは実際の入力対象として機能している。「かな」キー
  （`VK_DBE_HIRAGANA`, scan 0x70）は 6536ms に押下され `ImeOpenApplied
  {open:true, Applied}` まで到達＝正常に効いている。「英数」キー
  （`VK_DBE_ALPHANUMERIC`, scan 0x3A）は **2つ目の MWB フォーカス区間
  （21360ms〜46563ms）の内側にあたる 35046ms** に押下されているが、
  対応する IME actuation ログが journal に存在しない（前後の actuation は
  25358ms と 49880ms で、その間は空白）。
- report2（リモート側、journal 831件）: 物理由来（scan 付き）の DBE
  モードキーの到達時刻（8658ms・308189ms・308524ms・308686ms、すべて
  `injected: true`）におけるフォーカスは `explorer.exe`/`sakura.exe` で
  あり、MWB の中継ウィンドウではない。MWB がフォーカスを保持していた
  7 区間の内側には `KeyInput` が1件も存在しない。（`scan=0x0` の
  `VK_DBE_DBCSCHAR` が 5488ms にもう1件あるが、scan 無し＝物理キー由来
  でないため上記4件とは区別し、本項目の対象外とする。）

**コード上の事実（`crates/awase-windows/src/runtime/transport.rs`、
`PhysicalKeyDisposition::plan`。行番号は develop の変遷で動くため
シンボル名で参照する）:** `VK_DBE_HIRAGANA`（かな）専用分岐は TSF mode
かつ `f2_warmup_owned` の場合のみ Suppress、それ以外は常に Allow。一方
`VK_DBE_ALPHANUMERIC`（英数）等の DBE モードキーは、**GJI が active な
場合（`ActiveImeKind::GoogleJapaneseInput`）は profile を問わず常に
Suppress される**:

- `AppImeProfile::Standard`（ImmCross、report1 の中継ウィンドウ）:
  KANJI 関連 VK は Down/Up 共に無条件 Suppress（reason `"imm-cross"`）。
- `Imm32Unavailable`/`TsfNative`（report2 の `explorer.exe`/`sakura.exe`
  はこちらに分類される）: `gji_direct_applicable(GoogleJapaneseInput)`
  が真になり、かつ BUG-52 対策の `is_dbe_mode_key_down` 条件
  （`dbe_mode_key_policy=Suppress` のとき DBE モードキーの KeyDown は
  `shadow_toggled` に関わらず常に Suppress）が成立するため、こちらも
  Suppress される（reason `"imm32-off"`）。

両レポートの `config1.toml`/`config2.toml` はいずれも
`dbe_mode_key_policy` 未設定＝既定値 `Suppress`。つまり**「かなは効くが
英数は効かない」という非対称性は ImmCross プロファイル固有ではなく、
GJI 稼働中は profile を問わず起こりうる**というのが現時点の仮説であり、
report1（ImmCross経路）・report2（Imm32Unavailable経路）の両方をこの
単一の仮説で説明できる。ただし旧 journal には engine の意味論的判断
（`decision`: PassThrough/Consume）しか記録されておらず、
`transport::plan` の最終配送判断（Allow/Suppress）自体は確認できな
かったため、上記はあくまで仮説であり確定していない。

**却下した対策（設計→Opus敵対的レビューでNO-GO、2026-08-26）:** 当初
「`powertoys.mousewithoutbordershelper.exe` を `app_overrides.disable_apps`
（BUG-78の `mstsc.exe` と同じ丸ごと無効化機構）の既定リストに追加する」案
を設計したが、以下の理由で NO-GO と判定し **`src/config.rs`/
`config.sample.toml` は変更していない**:

1. report1 の中継ウィンドウでは実際に親指シフト入力が機能しており
   （上記参照）、`disable_apps` で丸ごと無効化すると動いているワーク
   フローを壊す（生の QWERTY が MWB 経由でリモートへ転送されてしまう）。
2. report2 では DBE キー到達時に MWB がフォーカスを持っておらず、
   `disable_apps` は発火しない＝効果がゼロ。

**実施したこと（挙動は変更していない、診断ログの追加のみ）:**
journal に transport 層の最終配送判断を記録する `JournalEntry::
KeyInput::physical: PhysicalDispositionSummary`（`Allow` /
`Suppress { reason }`、`reason` は `"tsf-f2"`/`"imm-cross"`/`"imm32-off"`）
を追加した（`runtime/transport.rs::PhysicalKeyDisposition::
suppress_reason`、`journal.rs::PhysicalDispositionSummary`）。既存の
`decision` フィールド（engine の意味論的判断）とは独立した軸で、次に同じ
症状が再送された際に「英数キーが実際に Suppress されているか、されている
なら imm-cross/imm32-off のどちらの経路か」を journal から直接確認できる
ようにする。`kp_stage_execute` は以前ここで独立に `physical` を再計算して
いたが、journal 記録側の値と実処理側の値が完全に同一であることを保証する
ため、`kp_run_inner` で一度だけ計算した値を引数で受け渡す形に統一した
（Opus コードレビュー指摘）。回帰テスト4件を `runtime/transport.rs` の
`plan_tests` に追加し、ImmCross（Down/Up 双方）・GJI 稼働時の
Imm32Unavailable/TsfNative（imm32-off）・F2 専用分岐（tsf-f2）・Allow の
各 reason ラベルを固定した。

**状態:** **クローズ（2026-08-26）。** 根本原因（GJI 稼働中は DBE モードキーが
profile を問わず Suppress される設計、および MWB 自体が VK 再構成方式で DBE
キーを正しく中継できていない可能性）は未確定のままだが、下記の
「アプリ無効化」タブへの `powertoys.mousewithoutbordershelper.exe` 登録で
症状を回避できることをユーザーが採用し、これで十分と判断したためクローズする。
既知の制約（disable_apps 登録中はその中継ウィンドウで親指シフト入力自体も
無効化される、上記「ユーザー向け回避策」参照）として本エントリに記録する。
`dbe_mode_key_policy = "passthrough"` への切替検証は根本原因追及の選択肢として
残すが、回避策で運用できているため優先度は下げる。再発・別症状の報告があれば
再度原因未確定として扱う。

**ユーザー向け回避策（実装済み、根本原因の修正ではない）:** 設定画面
（awase-settings）に「アプリ無効化」タブを新設し、`app_overrides.
disable_apps`（フォーカス中のプロセスで awase を丸ごと無効化する既存機構、
BUG-78で`mstsc.exe`に導入済み）をGUIから編集できるようにした
（`tab_disable_apps`、`Tab::DisableApps`）。ユーザーが
`powertoys.mousewithoutbordershelper.exe` を自分の判断で追加すれば、
その中継ウィンドウにフォーカスがある間は awase が完全にパススルーになり
「英数/かなキーが効かない」症状を回避できる。ただし report1 で確認した
通りこの中継ウィンドウでは親指シフト入力も同時に無効化されるため
（その入力がMWB経由でリモート側へ正しく届いていたかは実際には未検証、
上記の仮説参照）、**既定では追加しない**（`default_disable_apps()`は
`mstsc.exe`のままで変更していない）。あくまでオプトインの回避策であり、
根本原因の特定・修正は引き続き未対応。`docs/bug-reports-triage.md` に
report_id 2件を記録。

---

### 追補（2026-09、GitHub Issue #136、根本原因を特定・修正、ADR-119）

GitHub Issue #136「境界線のないマウス経由でIME ON/OFFが効かない」として同一ユーザーから
再度報告された。これがBUG-90と同一インシデントかどうかを確認するため、Cloudflare R2から
上記report_id 2件の実journal/ログを再取得して精査した結果、**当初の仮説（「ImmCross
宛先違い」で`set_ime_open_cross_process`が間違ったウィンドウを叩く）は不正確だった**と
判明した。実際には**独立した2つのバグの合成**であり、両方とも根本原因を特定し修正した。

**リモート側（report2）**: `PhysicalKeyDisposition::plan`（`runtime/transport.rs`）の
`VK_DBE_*` Suppress判定が`event.injected`を一切見ておらず、2026-07-06のBUG-14修正が
明言した不変条件「OSへの配送は維持し、実IME追従は観測経路に委ねる」を、2026-08-05の
BUG-52対応が後から破っていた（リグレッション）。R2のjournalで
`VK_DBE_ALPHANUMERIC injected=true scan=0x3A`が複数回到達していることを直接確認した。

**ローカル側（report1）**: 実ログの決定的な1行:
```
[shadow-toggle] intent 昇格: vk=0xF0 scan=0x3A action=TurnOff kind=PhysicalImeKey injected=false false→false
```
`current`（トグル前のeffective_open()）と`new_val`（TurnOff後の目標値）が共に`false`——
awaseの信念は既に一致しており、apply-ime自体が一切呼ばれない（no-op）。直前10秒の
ログには`observed=false≠desired=true`のdrift correctionがGeneration 0〜5まで繰り返されて
おり、MWB中継ウィンドウのIME観測自体が実在の意味を持たない（中継ウィンドウは実際の
テキスト入力面ではないため）ノイズであり、これがbeliefを汚染していたことが確定した。

**独立した2回目の実データ確認（`docs/bug-reports-triage.md`、report_id
`01M1EFYKJZRMACMP0S8QJRA6T1`(リモート側)/`01M1EG3JYGPQ3RZD24GFP429B4`(ローカル側)、
2026-09-01、別セッションによる並行トリアージ）**: リモート側report2の
`KeyInput.injected`は**102件中102件が`true`**（MWBは配送する全キー入力を例外なく
SendInputとして注入している）。`[shadow-toggle] injected IMEキー vk=0xF0/0xF2
はユーザー意図に昇格させない (BUG-14)`ログが多数出力されていることを直接確認、
リモート側の根本原因（決定1が対象とするS2/BUG-14ガード経路）を独立に裏付けた。
ローカル側report（**この時点でユーザーは既にBUG-90の回避策
[`app_overrides.disable_apps`にMWB中継プロセスを登録]を適用済み**）では、
中継プロセスへフォーカスがある5区間(計約6分)でKeyInputが0件(正しく無効化)、
それ以外(sakura.exe操作時)の英数/かなキーは`injected=false`かつ`PhysicalImeKey`
として正常成功しており、**ローカル側単体では症状が再現しなくなっている**ことを確認
——disable_apps回避策適用中のユーザーには決定1のみで#136の体感症状が解消される
見込みが高いことを示唆する（決定4はdisable_apps未適用のユーザー、および
disable_apps適用中も失われる「中継ウィンドウでの親指シフト入力」を回復するために
引き続き必要）。

**修正（決定1+決定4、詳細は[ADR-119](adr/119-injected-and-relay-key-consumption-invariant.md)
参照）:**
- 決定1: `transport.rs::plan`にinjectedイベントをAllowにする分岐を追加（F2分岐は対象外）
- 決定4: `AppImeProfile::InputRelay`を新設し、`app_overrides.input_relay_apps`
  （既定は空配列・オプトイン、プロセス名で識別）にマッチしたウィンドウでは
  IME actuationを所有せず、物理モードキーもsuppressせず、このウィンドウ由来の
  open観測もbeliefに取り込まない。親指シフトの文字変換自体はこれまでどおり動作する
  （**disable_appsとの違い**: disable_appsはawaseを丸ごと無効化するため文字変換も
  止まるが、`input_relay_apps`はIME制御の非所有のみで文字変換は継続する。BUG-90調査
  当時にdisable_apps既定追加がNO-GO判定された2つの理由——(1)動いているワークフローを
  壊す(2)report2ではdisable_appsが発火せず効果ゼロ——はどちらも`input_relay_apps`
  には該当しない）

**実装過程で発見・修正した自己回帰**: 決定4のIME actuation非所有（condition (a)）を
`runtime/executor.rs::dispatch_ime_set_open`の1点だけでgateする初期実装は、実際の
actuation呼び出し経路が他に4つ（`runtime/key_pipeline.rs`のshadow-toggle経路2つ、
`runtime/mod.rs`のforce-ON/bootstrap経路2つ）あることを見落としており、`key_pipeline.rs`
の物理IMEキー押下経路（issue #136で報告された操作そのもの）がこのgateをバイパスして
BUG-46型の二重actuation（物理キーのAllow素通しとawase自身のactuateが同時に発生）という
新規リグレッションを生んでいた。実装完了後のOpus敵対的コードレビューでこの欠陥を発見し、
gateを実際の合流点3箇所（`ImeController::apply`/`run_open_chain_async`/`fallback_write`）
に置き直して修正した。

**既知の限界（2件）:**
1. かな(0xF2) × リモート側TsfNative × `f2_warmup_owned=true`の組み合わせでは、
   決定1がF2分岐を意図的に対象外としているため`VK_DBE_HIRAGANA`が依然Suppressされる
   （F2のSuppressは「awase自身のwarmup F2とのダブルF2防止」という別種の保護であり、
   バーター原則の対象外）。R2の実データにこの組み合わせのケースが含まれておらず、
   実害の有無自体が未確認。
2. `state/ime_model.rs::resolve_open_at`の`BaseDecision::DesiredFallback`枝——
   中継ウィンドウ滞在中に書かれた`desired_open`は`FocusChanged`による`last_intent`
   クリアで通常は戻り先の観測に上書きされるが、戻り先に観測が一件も無い場合に限り
   表面化しうる。決定4固有の欠陥ではなく既存の枝の性質。

**未検証（実機、Windows実機必須のためこのLinuxサンドボックスでは未実施）:**
- フックチェーン順序（リンク(a)）: `hook.rs`によりawaseのLLフックは常に`LRESULT(1)`を
  返すため、決定1/決定4の`Allow`は「engineスレッドがSendInputでreinjectする」ことを
  意味し、MWBのフックがawaseより前段か後段かで実際に中継されるかが変わりうる。
- MWBの実際のプロセス名（複数プロセス構成のため`powertoys.mousewithoutbordershelper.exe`
  が常に正しいか未確認、`input_relay_apps`の既定値を空配列にした理由）。

これらが未検証のため、issue #136には報告者への実機検証依頼として回答する。

---

## BUG-92: BUG-33 追補 — `Imm32Unavailable`/`TsfNative` の shadow フォールバック観測 laundering を型で閉じた（ADR-106 決定2）

**位置づけ:** 新規の実機不具合報告ではなく、BUG-33（「belief 自身を『観測』として
書き戻す循環」）が確定させた欠陥そのものを、実行時ガードではなく型で構造的に
除去したリファクタの記録。BUG-33 の「検知側の未解決ギャップ（drift-correction が
構造的に発火しない件）自体は本修正後も残存する」という記述（当該エントリ末尾）を
本エントリで解消する。番号は元コミットでは暫定（BUG-81）だったが、develop への
1回目の rebase 時に BUG-80〜89 が既に別件（ADR-105/102 コードレビュー是正等）で
採番済みと判明したため BUG-90 に採番し直した。その後、developが並行して進み
（`fdb3e842`）、別ブランチ発の BUG-88（PowerToys Mouse Without Borders）が
BUG-90 へ改番されて develop 側に先着したため、2回目の rebase で再度衝突し
BUG-92（本ブランチでは BUG-81→90→92 の変遷）に採番し直した（
[main-develop-branch-flow](../.claude/rules/main-develop-branch-flow.md) の
番号衝突対応）。

**症状（BUG-33 からの再掲）:** TsfNative/Imm32Unavailable プロファイル
（WezTerm/Chrome/Edge 等）では `apply_focus_probe`
（`runtime/key_pipeline.rs`）が `shadow_on = effective_open()`（＝現在の belief
そのもの）を `apply_effective_ime()` 経由で `write_focus_probe()` に渡し、
`ObservationSource::FocusProbe`（confidence=Low）として観測ストアへ書き込んで
いた。この観測は定義上 `open == desired` になるため、`check_drift_correction` の
`if trusted.open == desired { return None; }` に毎回引っかかり、drift correction
が構造的に一度も発火し得なかった（BUG-33 で確定済み）。

**原因（型で防げなかった理由）:** `sanitize_focus_probe_open_status` は
`Option<bool>` を返しており、「プロファイルが IMM32 open status を読めない」場合
（構造的に観測不能）と「プロファイルは読めるが今回は取得できなかった」場合が同じ
`None` に潰れていた。`apply_effective_ime`/`write_focus_probe` の引数も素の
`bool` だったため、`effective_open()`（belief）由来の値と実際に IMM32 API から
読み取った値が同じ型として扱え、実行時の `if`/`else` 分岐を書き間違えると即座に
laundering が再発しうる状態だった。

**修正 (ADR-106 決定2):** `state/observation_store.rs` に
`FocusProbeOpenStatus::{Read(ObservedOpenValue), NotObservable(AppImeProfile)}` を
新設し、`ObservedOpenValue` は `FocusProbeOpenStatus::classify` の `Read` 分岐
からしか構築できないようにした（フィールド private）。`apply_effective_ime`/
`write_focus_probe` の引数をこの型に変えたことで、`apply_effective_ime(shadow_on,
...)`（belief 由来の値を観測として書く旧経路）は型検査でコンパイルエラーになり
物理的に書けない。`apply_focus_probe` の `NotObservable` アームでは観測の記録を
一切行わない。

**guard 解除の副作用（撤去と同時に対処）:** 旧 `apply_effective_ime(effective)` は
`effective == true` のとき `reset_detect_state()`（observe-miss リセット + force
guard 全解除）を呼んでいた。これは観測記録とは独立した副作用のため、shadow
フォールバック経路の撤去で黙って失うと `BrokenAppBootstrap` guard 等の解除
タイミングが失われる。`FocusProbeOpenStatus::NotObservable` アーム内で
`probe.is_japanese_ime && shadow_on` のときだけ `reset_detect_state()` を独立して
呼ぶことで、観測の laundering だけを消し guard 解除の挙動は変えていない。

**「唯一の観測源が消える」という懸念について:** 成立しない。BUG-33 が確定させた
とおり、その観測源は定義上 `desired` と一致する自己参照値であり、
`check_drift_correction` は撤去前から常に `None`（不一致なし＝補正不要）を返して
いた。本修正は drift correction の能力を減らさず、減っていた事実を可視化した
だけである。

**未解決の疑問（実機ソークで確認すること、ADR-106 参照）:** 「3秒 FRESH を超えて
凍った古い shadow 観測」が消えることで、TsfNative/Imm32Unavailable の
`effective_open()` 解決結果が変わるケースが実機で発生するか。BUG-33 が残していた
回復経路（per-VK confirm give-up → `send_chrome_gji_reinit_and_poll`、
focus-resync + idle-conv-check、`ConvOpenInference` + 明示意図）が引き続き
機能しているかを実機ソークで確認すること。

**テスト:** `state/observation_store.rs` に `ObservedOpenValue`/
`FocusProbeOpenStatus` の doctest（`Read` からの構築・`NotObservable` からは
`compile_fail`）を追加。`runtime/key_pipeline.rs` の
`focus_probe_open_status_is_not_observable_for_imm32_unavailable`/
`_for_tsf_native`/`focus_probe_open_status_is_read_for_standard`/
`_when_probe_returns_none_even_for_standard` で `classify` の全分岐を固定。
Windows 実機での drift correction 再発火自体の確認は未実施（次回実機ソーク）。

**関連ファイル:** `crates/awase-windows/src/state/observation_store.rs`
（`FocusProbeOpenStatus`/`ObservedOpenValue` 新設）、
`crates/awase-windows/src/runtime/key_pipeline.rs`（`apply_focus_probe`/
`apply_effective_ime`）、`crates/awase-windows/src/state/platform_state.rs`
（`write_focus_probe`）。関連: BUG-33（本追補の対象）、
[ADR-106](adr/106-fence-ownership-and-observation-provenance.md) 決定2。

## BUG-91: ネイティブ Win32 マルチフィールドダイアログでのフィールド間 Tab 直後、進行中の FocusProbe/ImmCrossProbe/idle-conv-check の観測が hwnd 不一致で棄却され鮮度が低下する（Step1: 計測のみ実装）

**位置づけ:** 実機不具合報告ではなく、PR 109 コードレビュー指摘1（Opus による設計 +
敵対的レビュー）が発見した理論上のリスクに対する計測導入の記録。番号は当初
[experiment-logging](../.claude/rules/experiment-logging.md) と同じ理由（並行
ブランチとの衝突可能性）で BUG-88（`git show develop:docs/known-bugs.md` で
BUG-87 まで使用済みと確認した時点での次番号）として暫定採番していたが、develop
への rebase 時に BUG-88 が別件（HOOK_KEYS リング overflow、ADR-102/105
コードレビュー指摘2）に既に使われていたと判明したため BUG-91 に採番し直した
（[main-develop-branch-flow](../.claude/rules/main-develop-branch-flow.md) の
番号衝突対応、本ブランチでは BUG-81→BUG-90→BUG-92 の変遷と合わせて2件目）。

**症状（理論上のリスク、実機未確認）:** `ImmLikeTicket::admit()`
（`state/probe_admission.rs`）と `ObservationStore::derive_filtered` の
`is_identity_ok`（`state/observation_store.rs`）は、`FocusFence{epoch, hwnd}`
の `hwnd` に `GetGUIThreadInfo().hwndFocus`（フォーカス中コントロール）を使う。
ネイティブ Win32 のマルチフィールドダイアログ（複数の `EDIT`/`COMBOBOX` 等を
持つ単一 top-level ウィンドウ）で Tab キーによりフィールド間フォーカスが移動
すると、`process_changed=false`・`FocusEpoch` 不変のまま `hwndFocus` だけが
毎回変わる。進行中（spawn 済みで未完了）の `FocusProbe`/`ImmCrossProbe` の
非同期タスクや `ConvModeMgr::observe()` の monotonic guard は、この hwnd
変化を「フォーカスが変わった」として棄却する——実際には同一 top-level
ウィンドウ内の移動であり、IME 状態の連続性は保たれているはずのケース。

**現状の対応（Step1、本コミットの内容）:** 判定ロジック（`is_identity_ok`/
`admit()`/`FocusFence`）は一切変更していない——上記が実害かどうかは実機
ソークで実測してから判断する。まず計測のみ導入した:

- `focus/current.rs::CurrentFocus` に `root_hwnd: usize`
  （`GetAncestor(hwnd, GA_ROOT)`、非 Windows では `hwnd` と同値）を追加。
  `hwnd` はフォーカス中コントロール、`root_hwnd` が真の top-level ウィンドウ。
- `state/probe_admission.rs::RejectionStats` を3軸に分割:
  `epoch_mismatch` / `hwnd_mismatch_same_root` / `hwnd_mismatch_cross_root`。
  `admit_epoch_in_app`（`root_hwnd` にアクセスできる呼び出し元）が
  `FocusHwndChanged` 棄却時に spawn 時 hwnd の `root_hwnd_of()` と現在の
  `root_hwnd` を突き合わせて分類する。
- `[ImmCrossProbe]`/`[FocusProbe]`/`[idle-conv-check]` 系の `reject_log` に
  `(same_root=...)` を付記。
- `runtime/message_handlers.rs::handle_wm_dump_journal`
  （`WM_DUMP_JOURNAL`、Alt+変換→Alt+無変換 ×2 でトリガー）が3軸の
  `RejectionStats` をダンプするよう追従。

**計測方法（実機ソーク時の確認手順）:** ネイティブ Win32 マルチフィールド
ダイアログ（例: メモ帳の「検索と置換」、任意の設定ダイアログ）でフィールド間を
Tab 移動しながら typing し、`WM_DUMP_JOURNAL`（Alt+変換→Alt+無変換 ×2）で
`[probe-admission] rejected since last dump: ... hwnd_mismatch_same_root=N
hwnd_mismatch_cross_root=M` をダンプする。`hwnd_mismatch_same_root > 0` が
実測できれば、この理論上のリスクが実際に発生していることの証拠になる。

**計測の交絡に関する注記（重要）:** 追跡している hwnd の取得元は
`focus/probe.rs` → `win32.rs::get_gui_thread_info_with_timeout`（~L209-212）で
`hwndFocus` →（null なら）`hwndActive` →（`GetGUIThreadInfo` 自体が失敗/
タイムアウトなら）`GetForegroundWindow()` と**状況により切り替わる**。この
切替だけでも `hwnd_mismatch_same_root` が加算されうる（`hwndActive` と
`hwndFocus` が同一 top-level 内の異なる子ウィンドウを指すことがあるため）。
実機ログを見る際は「Tab 移動の証拠」と単純に読まず、この取得元切替との交絡が
ないか（`GetGUIThreadInfo` のタイムアウト・失敗ログの有無）を確認すること。

**状態:** Step1（計測のみ）実装済み・developへの反映待ち。Step2（`root_hwnd`
一致時に判定ロジックへ反映する）は、この開発環境が Linux サンドボックスであり
Windows 実機ソークができないため、**実機で `hwnd_mismatch_same_root > 0` を
確認してから着手する**。UWP アプリでの `GA_ROOT` プロセス越えリスク（BUG-18
近縁）も Step2 着手時に実機確認が必須。

**関連ファイル:** `crates/awase-windows/src/focus/current.rs`（`root_hwnd`）、
`crates/awase-windows/src/focus/classify.rs`（`root_hwnd_of`）、
`crates/awase-windows/src/state/probe_admission.rs`（`RejectionStats`/
`record_hwnd_mismatch`/`admit_epoch_in_app`）、
`crates/awase-windows/src/runtime/message_handlers.rs`
（`handle_wm_dump_journal`）。関連: BUG-18（AppKind Uwp 往復での文字欠落、
`GA_ROOT` のプロセス越えリスクが近縁）、
[ADR-106](adr/106-fence-ownership-and-observation-provenance.md) 決定3。
---

## BUG-93: MS-IME の無変換単独タップ delegate が変換中 composition を破棄する

**症状:** MS-IME レジストリで `KeyAssignmentMuhenkan=1`（awase 側では
`muhenkan_delegate_to_open_axis=TurnOff` 相当）にしているユーザーが、変換中に
無変換を単独タップすると、進行中の composition 文字列が復旧不能に破棄される。

**原因:** `src/engine/nicola_fsm.rs::resolve_pending_thumb_as_single` の
`delegate_to_open_axis` 分岐が `composing` 判定より手前で無条件に
`ime_open_requested=TurnOff` を返していた。通常の無変換/変換単独タップは
`ModeKeyConfig.composing`（既定 `Suppress`）により変換中の副作用を抑えるが、
delegate 分岐だけがこの fail-closed 経路を飛び越えていた。

**修正:** `delegate_to_open_axis` は `composing=false` のときだけ発火するようにし、
`composing=true` では下流の `ModeKeyConfig.composing` へフォールバックさせた。
`composing` が誤って true になった場合の被害は「何もしない」に留まる一方、
誤って false で `TurnOff`/`Toggle(→OFF)` を送ると composition を破棄するため、
変換中は fail-closed を優先する。

**テスト:** `src/engine/tests.rs` の T-10
`delegate_to_open_axis_suppressed_while_composing` で、engine 活性 +
`delegate=TurnOff` + `composing=true` の無変換単独タップ確定時に
`Effect::Ime(SetOpen)` が出ず、既定 `ModeKeyConfig.composing=Suppress` により
raw `VK_NONCONVERT` も送出されないことを固定した。

**番号衝突チェック:** `git log --all --oneline -- docs/known-bugs.md`、
`git branch -a`、`git worktree list`、および全 refs の `docs/known-bugs.md`
に対する `BUG-[0-9]+` 走査で、作業時点の最大番号が BUG-92 であることを確認し、
本件を BUG-93 として採番した。

**関連ファイル:** `src/engine/nicola_fsm.rs`, `src/engine/tests.rs`。

**修正履歴:**
- 本修正コミット: `delegate_to_open_axis` の composing ガード追加と T-10 追加。

---

## BUG-94: 親指キーを無変換/変換に選び直すと設定画面のドロップダウンが消える

**症状:** awase-settings のキー設定タブで、左親指/右親指キーのドロップダウンから
「無変換」または「変換」を選び直すと、その下に表示されるはずの「無変換キー
単独タップ」「変換キー単独タップ」の設定（常に送出する/常に無視する 等の
ドロップダウン）が表示されなくなる。GitHub issue #99、タスクトレイの不具合報告
機能経由の report `01M10SA5K7J4HZ3C5R1BF6K2QK`（2026-08-27T04:54:04Z、
app_version 1.16.1）で報告された。実際のキー入力動作（親指シフト判定・IME制御）
は壊れておらず、設定画面の表示条件のみが影響を受ける。

**原因:** `crates/awase-settings/src/main.rs` 内で、`left_thumb_key`/
`right_thumb_key` の内部表現が2種類混在していた。

- `src/config.rs` のデフォルト値は漢字表記 `"無変換"`/`"変換"` そのもの。
- 一方、GUI のドロップダウン（`THUMB_KEY_OPTIONS`、`main.rs`）で選択すると、
  その内部表記 `"VK_NONCONVERT"`/`"VK_CONVERT"` が書き込まれる。
- しかし「無変換キー単独タップ」「変換キー単独タップ」ブロックの表示条件
  （`main.rs`）は `left_thumb_key == "無変換"` / `== "変換"` という**漢字表記との
  リテラル比較のみ**で、`"VK_NONCONVERT"`/`"VK_CONVERT"` を考慮していなかった。

初期状態（デフォルト値 `"無変換"`）ではブロックが表示されるが、ユーザーが
ドロップダウンで値を選択し直す（同じ「無変換」を選び直した場合を含む）と
`"VK_NONCONVERT"` に書き換わり、以後は条件が一致せずブロックが消える。

**修正:** 表示条件を判定する `is_muhenkan_thumb_key`/`is_henkan_thumb_key`
ヘルパーを新設し、漢字表記・VK表記の両方を受理するようにした。回帰テスト
（`thumb_key_display_condition_tests`）を追加。`.claude/rules/fix-requires-evidence.md`
の再発ファミリー表（warmup/focus/belief/conv/キー選択/force-write）には
該当しない（IME制御には影響しないGUI表示ロジックのみのバグのため）。

**関連ファイル:** `crates/awase-settings/src/main.rs`
（`is_muhenkan_thumb_key`/`is_henkan_thumb_key`、`THUMB_KEY_OPTIONS`）。

---

## BUG-95: `.yab`のクォート崩れリテラルが無警告で受理される（レイアウト検証不足）

**症状:** タスクトレイの不具合報告機能経由の report `01M13EACMQ7D2VETW75N0BTZ9C`
（2026-08-28T05:28:03Z、app_version 1.16.1、GJI、JISキーボード）で以下2件が
報告された。

1. 変換キー（親指右キー）を単独タップしても漢字変換が起きない（「変換する
   ように設定してある」との申告あり）。
2. ユーザーが独自編集した `layout_yab` で「ぶ」キーを押すと `b` になる。

（同一報告にあった無変換キー単独タップ確定の不具合は別原因で、report
`01M10VJWF7R8TNZAM08THVZDT7` と同じ `is_bare_thumb`/`suppress_ime_combos`
（PR #114、未リリース）で既に修正済み。詳細は `docs/bug-reports-triage.md` の
`01M13EACMQ7D2VETW75N0BTZ9C` の行を参照。）

**このBUGエントリで扱うのは症状2（`.yab`誤字）のみ。** 症状1については
下記「症状1について: 撤回した仮説」を参照——真因は未確定のまま。

**原因（症状2）:** `layout_yab` の右親指シフト面 V 位置に `ｂ'ｕ` という誤字
（正しくは `ｂｕ`）があった。`YabValue::parse`（`src/yab/mod.rs`）はクォート
文字が対になっていない任意の文字列を、無警告でそのまま `Literal` として受理
してしまう（検証機構が存在しなかった）。デフォルト同梱の `layout/nicola.yab`
は正しく `ｂｕ` であり、ユーザー独自編集時の誤字と確認済み。

**修正（症状2）:** `src/yab/mod.rs` に `YabValue::lint_raw_cell`/`yab::lint`
を新設。クォート文字を含むのに対になっていないセルを検出し、行番号付きの
警告文言を返す（パース自体は従来通り失敗させない）。
`crates/awase-settings/src/main.rs` のレイアウト読み込み（`load_yab_layout`）・
保存（`layout_write_to_path`）双方でこの警告を `layout_status` に付記する
ようにした。回帰テスト5件追加。

**症状1について: 撤回した仮説（2026-08-28、コードレビューで指摘・訂正）**

初版では「`right_thumb_key = "VK_SPACE"` により変換（`VK_CONVERT`）がどちらの
親指キーにも未割当のため、`henkan_solo_tap_ignore_composing_guard`/
`henkan_solo_tap_always_suppress`（`NicolaFsm::henkan_vk` 経由、
`src/engine/nicola_fsm.rs`）が無条件で無効化されている」ことを原因と断定し、
`AppConfig::validate_solo_tap_reachability` という警告バリデータを追加してい
たが、以下の理由でこの仮説・実装ともに**撤回した**（コミット履歴に残る、
`src/config.rs` からは削除済み）。

- `right_thumb_key = "VK_SPACE"` は人間工学上の理由で選ぶユーザーが珍しくない
  正当な設定であり、「間違っている」という前提で警告を出すのは筋が悪い
  （ユーザー指摘）。
- awase-settings の GUI は右/左親指キーが変換・無変換以外のとき「変換キー
  単独タップ」設定セクション自体を非表示にする（`is_henkan_thumb_key`、
  BUG-94 の修正箇所）。つまりこの報告者は `right_thumb_key = "VK_SPACE"` の
  状態でそのGUIセクションを操作できず、「変換するように設定してあります」
  という発言が `henkan_solo_tap_*` を指しているとは考えにくい。
- report のjournalは起動直後の約7.8秒（FocusTransition/GjiFsmTransition中心、
  60件）のみで、実際の変換キー押下に対応する `KeyInput`/`ConvClassifyCall`
  等のイベントが一切含まれておらず、当初の仮説を実データで検証できていな
  かった。

症状1の真因は依然未確定。次に調査する場合は、まず新しい journal
（実際に変換キーを押した瞬間を含むもの）の取得を優先すること。awase が
`VK_CONVERT` を親指キー・`keys.*` コンボのどちらにも割り当てていない場合、
Phase 1/Phase 3 とも素通しするため、原因は awase 側の分岐ロジックよりも
GJI 側の composition 状態（変換候補が実際に立っていたか）や、NICOLA
入力がGJIの変換バッファをどう扱っているかにある可能性が高い。

**2026-08-28 追加調査（ユーザー仮説「'変換キー=右親指キー'という前提が
コードに埋め込まれているのでは」の検証）**: `VK_CONVERT`/`VK_NONCONVERT`
に触れる全箇所（`crates/awase-windows/src/hook.rs::classify_key`、
`vk.rs::ImeKeyKind::from_vk`（0x1C/0x1Dは意図的に対象外）、
`engine.rs::SpecialKeyCombos::match_event`、
`runtime/key_pipeline.rs::kp_stage_shadow_ime_toggle`、
`app/bootstrap.rs`の`henkan_vk`/`muhenkan_vk`導出）を洗い出したが、
いずれも `left_thumb_key`/`right_thumb_key` の実際の設定値との比較を
経由しており、「変換キーは常に親指キー」という無条件のハードコードは
発見できなかった。`right_thumb_key = "VK_SPACE"` の設定下では、VK_CONVERT
は `KeyClassification::Passthrough`（`hook.rs`、対応する物理スキャン
コードが `scanmap.rs` の JIS/US テーブルに無いため）として素通しされ、
`ImeRelevance.is_ime_control`等も全て偽になるため抑制されない。

代わりに、**出力（romaji確定→IME送信）側の "eager path" が GJI の
composition を迂回する既知の仕様**が、症状の代替説明として有力。
`crates/awase-windows/src/tsf/warmup/probe_fsm.rs:131`
`decide_transmit_plan()` のコメントに「unicode は GJI composition を
バイパスし "nお" race が起きる」と明記されている通り、一定条件
（`nc_confirmed=true` かつ非TSFモード等）では確定ひらがなを
`output/vk_send.rs` 経由で Unicode として直接 `SendInput` する
（`used_eager_path=true`）。この経路では GJI 側に「変換候補として
保持中の未確定文字列」が存在せず確定済みテキストとして着地するため、
直後に生の VK_CONVERT を押しても変換対象が無く何も起きない——awase の
VK_CONVERT処理自体にバグが無くても症状が説明できる。関連する既存知見:
[[feedback_unicode_injection_bypasses_gji_composition]]（Claude memory、
「Unicode注入はGJI確認を迂回する」）。

**次の切り分け手順**: 実機ログで該当打鍵時の `[tsf-transmit] ...
eager=true/false`（`vk_send.rs:41-52`）を確認し、症状発生時に
`eager=true` になっているかを見るのが最短。もし確認できれば、これは
awase側の実装バグではなく、flicker回避のための既存トレードオフ設計の
副作用として記録し、GUI/ドキュメントで「eager path使用時は変換キー
単独タップでの変換候補操作ができない」ことを明示する対応を検討する。

**2026-08-28 さらに追加調査: report添付ログを再確認したが症状1の再現は
写っていなかった**。report `01M13EACMQ7D2VETW75N0BTZ9C` の
`app_log_excerpt`（200KiB切り詰め済み）を精査したところ、2つの時間帯が
混在していた: (a) `2026-08-28T01:41〜01:48Z` — 実際のローマ字入力
（`[key-output] KeyInput(batched): romaji=...`）が152件記録されているが、
**MS-IME + Chrome** の文脈（`target=Chrome`, `ime=MsIme`）で、report本文
が指すGJI/UWPアプリの状況とは別物。(b) `2026-08-28T05:27:49〜05:28:03Z`
— report送信直前の約14秒間（起動→GJI検出→専用Fnキー(F21)ポップアップ
対応→トレイの「不具合を報告」を即座に開く、という流れ）で、**この区間
には`[key-output]`（実際の打鍵）が1件も記録されていない**
（`grep "05:2[78]"` で `[key-output]` 0件を確認）。

**副次的に判明した事実**: 直前に見つかった
`[gji-charset-write]`/`[gji-charset-popup]`（config1.db への専用Fnキー
(F21)書き込み）は、コード調査の結果**症状1と無関係と確定**。書き込み対象は
F21×`SwitchKanaType`（ひらがな/カタカナ/半角カナ切替）のみで、GJI内部の
「変換」機能（VK_CONVERTの割当）には一切触れない
（`crates/awase-gji-config/src/write.rs:88-101`）。トリガ条件は
`muhenkan_solo_tap_always_suppress=false`（ユーザーがこの report で明示的に
そう設定していたことと整合）かつユーザーがポップアップで「はい」を選んだ
場合のみで、無変換キー側（症状2）の別機能であり、変換キー（症状1）とは
無関係（詳細citation: `crates/awase-windows/src/gji_charset_write.rs:71-105`,
`gji_charset_popup.rs:39-132`, `crates/awase-gji-config/src/lib.rs:102-140`）。

**結論**: 症状1の真因はこのreportのログからは確認も反証もできない
——単純に再現の瞬間が記録範囲外。次に同種の報告が来た場合は、まず
`[key-output]`/`[tsf-transmit]` の有無を確認し、実際の打鍵が記録範囲に
含まれているかを最優先で見ること。

**テスト:** `cargo test --lib -p awase` 846件green（症状2向けの新規7件含む）。
`cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows
-p awase-settings` で Windows 向けコンパイル確認済み。

**番号衝突チェック:** 全 refs（`git branch -a`・`git worktree list`・全リモート
ブランチ）の `docs/known-bugs.md` を `BUG-[0-9]+` で走査し、作業時点の最大番号
が BUG-94（`origin/develop`）であることを確認し、本件を BUG-95 として採番した。

**追記（BUG-104 調査で判明、要 re-verify）:** 症状2（「ぶ」→「b」）の原因を
上記「`ｂ'ｕ` 誤字が `Literal` として無警告受理された」と説明しているが、
BUG-104 の調査で、この report の app_log に

```
startup: レイアウト読込失敗: ...\layout\NICOLA＋確定.yab: stream did not contain valid UTF-8
```

という起動ログが2回（`01:46:32`・`05:27:49`）記録されていることが判明した。
この `NICOLA＋確定.yab` こそが `ｂ'ｕ` 誤字を含む当該ファイルであり、UTF-8
デコードの時点で読込自体に失敗していた——つまり実際に有効化されていたのは
ソート順先頭のバンドル版 `nicola.yab`（`ｂｕ`→「ぶ」の対応は正しい）だった
可能性が高い。本エントリの調査時点ではこの起動ログが参照されておらず、
「ユーザー独自編集ファイルが実行時に読み込まれた」という前提が誤りだった
疑いが強い。症状2の真因は**未確定に戻す**必要がある。詳細は BUG-104 参照。

**関連ファイル:** `src/yab/mod.rs`, `crates/awase-settings/src/main.rs`。

---

## BUG-97: IME apply pending 上書き後の旧成功完了が stale 扱いされ applied が固着する

**症状:** 同一 target への IME apply が短時間に連続し、後続要求が `pending` を
上書きしたあとで先行要求の成功完了が届くと、generation 不一致で完了が捨てられ、
`applied` が古い値のまま残る。後続要求自身の完了が `UnsafeToToggle` 等で届かない
経路に入ると、`desired_open` との乖離が解消されず drift correction が再送を繰り返す。
また、フォーカス変更後に旧フォーカスプロセス由来の generation 付き完了が届くと、
新フォーカスで `Unknown` に戻した `applied` を旧値で破り、TsfNative の force-ON を
恒久的に封鎖しうる。

**再現条件:** ウィンドウAで generation 付き `ImeApplyRequested(target=true, gen=10)` を
送信し、完了前に同一 target の `gen=11` が `pending` を上書きする。その後 gen10 の
`ImeApplySucceeded(target=true)` が遅延到着する。フォーカス跨ぎの派生ケースでは、
gen10 送信後に `FocusChanged` が入り、その後 gen10 の成功完了が届く。

**修正:** ADR-108 に従い、`ImeTransition` に `focus_epoch` を追加し、generation 付き
完了の `applied` 書き込みを `ImeModel::reduce()` に集約した。厳密一致かつ同一 epoch の
完了だけを `Confirmed` とし、generation 不一致でも現在の `pending.target` と同じ target
かつ同一 epoch の成功完了は `Optimistic` として反映する。composition/warmup 副作用は
`ImeApplyAcceptance::Accepted` のみが駆動する。

**テスト:** `state::ime_model::tests` の ADR-108 追加ケースで、同一target上書き成功完了、
逆target棄却、フォーカス跨ぎ棄却、失敗系不一致、Confirmed降格防止、緩和経路の2入口、
タイムアウト境界越え完了、失敗厳密一致の `Confirmed{open: !target}` 移設を固定した。
`tests/journal_replay.rs` に `tests/journals/ime_apply/adr108-focus-crossing-success.json`
のリプレイを追加した。

**修正履歴:** 本作業ツリーで実装済み。実装コミット
`ea8a0fae26802c9777fcdadfc87471156349694c`、証拠義務テストコミット
`e00621cdaf214d087c3d89169d7aefa5a434bde1`、本エントリ記録コミット
`207e9af49f10325c2d0c1cde61835a7e9f193932`。

**関連ファイル:** `crates/awase-windows/src/state/transition.rs`,
`crates/awase-windows/src/state/ime_model.rs`,
`crates/awase-windows/src/state/platform_state.rs`,
`crates/awase-windows/src/runtime/mod.rs`,
`crates/awase-windows/src/output/ime_apply_planner.rs`。関連: ADR-108。

---

## BUG-98: generation なし非同期 shadow toggle OFF 完了は focus epoch ゲートを通らない

**症状:** `runtime/key_pipeline.rs` の shadow toggle OFF の ImmCross 非同期分岐は、
`run_open_chain_async(...).await` 後に `on_ime_apply_complete(false, outcome, None, ...)`
を呼ぶ。待機中に Alt+Tab 等でフォーカスプロセスが変わると、旧ウィンドウ宛ての完了が
新ウィンドウの文脈で到着するが、`generation=None` のため ADR-108 の epoch ゲートを
通らず、`applied = Confirmed{open:false}` と composition cold-mark を新ウィンドウへ
書き込む可能性が残る。

**再現条件:** ImmCross 経路で shadow toggle OFF を発火し、非同期 apply 完了前に
Alt+Tab で別プロセスへフォーカスを移す。その後、旧ウィンドウ宛ての
`generation=None` 完了が到着する。

**ADR-108で対象外にした理由:** generation なし経路は target 一致による
`clear_pending_if_matches` が pending 解放の正常経路になっている。ここへ単純に
epoch ゲートを追加すると、棄却された完了が pending を解放しないという意味論変更を
同時に持ち込むため、ADR-108 決定0の「pending解除は現状維持」の範囲を越える。

**follow-up方針:** `probe_admission::admit_epoch_in_app` と同型に、spawn 時の epoch を
actuation 完了ハンドラへ持ち込み、`with_app` 内で照合して早期 return する。もしくは
この経路にも `ApplyGeneration` を払い出し、generation 付き経路へ合流させる。

**状態:** 既知の残存ギャップ。ADR-108 決定6として意図的に未修正。

**修正履歴:** 未修正。本作業ツリーで BUG-97 側の generation 付き経路だけを修正済み。
実装コミット `ea8a0fae26802c9777fcdadfc87471156349694c`、証拠義務テストコミット
`e00621cdaf214d087c3d89169d7aefa5a434bde1`、本エントリ記録コミット
`207e9af49f10325c2d0c1cde61835a7e9f193932`。

**関連ファイル:** `crates/awase-windows/src/runtime/key_pipeline.rs`,
`crates/awase-windows/src/state/platform_state.rs`,
`crates/awase-windows/src/state/probe_admission.rs`。関連: ADR-108 決定6。
`crates/awase-windows/src/runtime/focus_tracking.rs`,
`crates/awase-settings/src/main.rs`。関連: ADR-110。

---

## BUG-100: `key_remap` の latch (`LATCHED_TARGET`) が KeyUp 消失や一部の swallow 経路で stuck する

**追記（2026-08-30）:** `key_remap` 機能自体（ADR-110）が ADR-111 r4 決定により
バックエンドごと撤回されたため、本エントリが指す `hook.rs`/`state/key_remap.rs`
のコードはもう存在しない。以下は記録として保持する（次に同種のホットキー
latch 機構を作る際の参考資料）。

**症状:** ADR-110 `key_remap` の hold-state（`LATCHED_TARGET`、vk でインデックスし
現在 latch 中の reinject 先 vk を保持する）が、以下の3経路でクリアされずに残り、
注入済みリマップ先キー（例: `from=VK_CAPITAL to=VK_LCONTROL` の LCtrl）が
stuck modifier になる。加えて `latched_target == 0` が `is_fresh_press` 判定を兼ねる
ため、latch が stuck した物理キーは以後の新規押下も auto-repeat 扱いになり、
config reload でルールを削除しても復旧しない（r2 のテーブル参照方式より悪化する
退行）。

1. セッションロック（Win+L）中に `WH_KEYBOARD_LL` へイベントが届かず KeyUp が
   失われる既知経路（`reset_physical_key_state()` の doc、2026-07-09 実機の
   右Shift KeyUp消失事例と同型）。`reset_physical_key_state()`/
   `clear_hook_latches_for_app_disable()` はどちらも `LATCHED_TARGET` を
   クリアしていなかった。
2. `HOOK_KEYS` の overflow ラッチが立った際の `ProduceResult::Overflow` アーム
   （`apply_key_remap` 適用**後**）が生の `KBDLLHOOKSTRUCT`（書き換え前の物理キー）
   をそのまま `CallNextHookEx` に渡すため、書き換え後 vk の KeyUp がどこにも
   送られない。
3. `VK_KANA` の条件付き swallow 分岐（`is_injected` または Alt 押下中のみ
   `LRESULT(1)` で丸ごと swallow）に `key_remap` 側の後始末が組み込まれておらず、
   `from="VK_KANA"` 設定時に Alt 押下中の KeyUp が swallow され latch が残る。

**発見の経緯:** ADR-110 実装後・PR #120 マージ後に、`opus-adversarial-consult`
スキルで依頼していた round3 レビュー（長時間の非同期実行を経て事後に返却）で
S1〜S3 として指摘された。round3 の完走を待たず r3 案を実装対象として確定・
マージしたため、マージ後の事後発見になった。

**修正:** `LATCHED_TARGET` 全スロットを「非0なら target の KeyUp を注入してから
クリア」する `release_all_latched_remap_targets()` を新設し、
`reset_physical_key_state()` と `clear_hook_latches_for_app_disable()`（Leave 側）
から呼ぶ（経路1）。`ProduceResult::Overflow` アームで、remap 適用前後の vk を
比較し KeyUp かつ異なる場合は書き換え後 vk の KeyUp を明示的に注入する（経路2）。
`VK_KANA` の2つの swallow 分岐（foreign-injected / Alt 押下中）それぞれで
`return` する直前に `cleanup_latched_remap_before_bypass` を呼ぶ（経路3）。
副次的に、`effective_ctrl_physically_held`（現在のルールテーブルを見る設計）を
`any_latched_ctrl`（`LATCHED_TARGET` そのものを見る設計）へ置き換えた
（config reload でルールが消えても、latch が生きている限り「Ctrl が実効的に
held されている」を正しく検出できるようにするため）。

**テスト:** `state::key_remap::tests` に `any_latched_ctrl_*` を追加（config
reload 後もテーブルではなく latch を根拠に判定できることを固定）。hook.rs 側の
3経路自体は Win32 フック・`SendInput` を伴う副作用のため Linux から直接
ユニットテストできず、本エントリと `cargo xwin clippy`/`cargo check --target
x86_64-pc-windows-msvc` によるコンパイル確認、および CI `windows-build` の
実機経路テストでのカバーに委ねる。

**修正履歴:** `fix/key-remap-latch-lifecycle` ブランチで実装（本エントリ記録と
同一コミット群）。ADR-110 決定2 r3 の続き（追補）として扱う。

**関連ファイル:** `crates/awase-windows/src/hook.rs`,
`crates/awase-windows/src/state/key_remap.rs`。関連: ADR-110, BUG-48, BUG-78。

---

## BUG-101: `Engine::on_input` の Phase 0 が Consume 済み KeyDown に対応する KeyUp を FSM に一切届けていない（2026-03-31 混入のリグレッション、決定0〜2実装済み・実機ソーク完了・クローズ）

**症状:** `Engine::on_input`（`src/engine/engine.rs:345-349`、実運用のキーイベント入口
——`key_pipeline.rs`と`runtime/mod.rs::process_deferred_keys`の2箇所から呼ばれる）の
Phase 0 が、`KeyLifecycle::on_key_up`（`key_lifecycle.rs:56-63`）を使って
「対応する KeyDown が Consume 済みの KeyUp」を無条件に `Decision::consumed()` として
即 return する。`NicolaFsm::on_key_up`（`nicola_fsm.rs:1558`）には一切到達しない。
以下3件の実害を確認済み:

1. **`min_overlap_margin_percent` が実運用で常に無効。** `char1_released_at`
   （`handle_key_up_pending_char_thumb`、`nicola_fsm.rs:1679-1730`）は恒久的に
   `None` のままなので、`timing::overlap_only_verdict`（`timing.rs:228-239`）は
   常に「重なり構造的保証あり」の `Some(true)` を返す。char1 を離してから
   親指キーを押した（＝物理的に重なっていない）2打が、常に同時打鍵として
   誤確定される。2026-08-23 の PR #85 で導入した重なり不足判定機能は、
   導入初日から一度も実機で効いていない。
2. **`KeyAction::Key(vk)` を出力する全キーで、対応する `KeyUp(vk)` が実運用で
   一度も送出されていない（stuck key）。** `handle_key_up_active`
   （`nicola_fsm.rs:1812-1830`）だけが `KeyUp(vk)` を再送する経路だが到達不能。
   `KeyAction::Key(vk)` は `.yab` の明示的な VK 指定行だけでなく、
   `resolve_pending_char_as_single`（`nicola_fsm.rs:1272`、配列定義外キーの
   フォールバック）や無変換/変換/Space/Enter の solo-tap passthrough 経路
   （`:1404`/`:1443` 付近）からも常に出るため、エンジン ON 中に非かな文字
   （レイアウト定義外のキー等）を打鍵すると、OS 側はそのVKが押されっぱなし
   だと認識し続ける。未検証の疑いではなく、コード上確定した欠陥。
3. **`OutputHistory.entries`（`output_history.rs:24-26`）が上限のない `Vec` で、
   `remove_by_scan` が実運用で呼ばれないため単調増加し続ける。** メモリリークで
   あると同時に、修正時の設計上の罠でもある——`remove_by_scan`（KeyUp整合性用）
   と `recent_kana()`（n-gram文脈用）が同じ `Vec` を参照しているため、Phase 0を
   素朴に直すと全打鍵で n-gram タイブレークの入力が変わってしまう。

**原因:** `handle_key_up_pending_char_thumb` の初出（`b9d0ed56`、2026-03-28）の
方が、`KeyLifecycle` Phase 0 の混入（`94c73a15`/`1b4c08bb`、2026-03-31）より
早い。Phase 0 混入時点で既存の KeyUp 処理群（`handle_key_up_pending_char_thumb`/
`handle_key_up_pending`/`handle_key_up_active`/`SpeculativeChar` 分岐）が
まとめて到達不能になった。ADR-020 は「OS に渡さない」しか決めておらず、
「FSM にも渡さない」は未記載——意図しない副作用であり、意図的な設計ではない。

**発見の経緯:** `feat/confirm-mode-simplify` ブランチで `min_overlap_margin_percent`
の配線テストを `Engine::on_input` 経由（実運用相当）で書いたところ、bare な
`NicolaFsm` 直呼びテストとは異なる結果になったことから発覚。既存の重なり判定
テスト群（`test_pending_char_thumb_insufficient_overlap_resolves_as_two_solos_on_thumb_key_up`
等）が全て bare `NicolaFsm` レベル（`Engine`/`KeyLifecycle` を経由しない）で
書かれていたため、これまで検出されていなかった。

**状態:** クローズ（2026-08-31）。決定0〜2実装済み（[ADR-112](adr/112-keyup-lifecycle-fsm-delivery.md)、
Opus 2体によるpremortem 2ラウンドで収束した設計どおり）。`OutputHistory` の
責務分割 → `min_overlap_margin_percent` 既定値を一時的に0へ → Phase 0 の
再設計本体、の3コミットで修正した（`KeyLifecycle::on_key_up`を`take_key_up_duty`
に置き換え、`Decision::force_consume`で唯一の出口からConsume義務を保証、
非活性時は`release_only`でchord判定を再開せず解放索引だけ掃除）。
develop上で数日のWindows実機ソークを実施し、「文字が消える」「余計なKeyUpが
出る」等の不具合報告は無かった。決定3（`min_overlap_margin_percent`を実測付きで
引き締める）は、引き締めに必要な実測データ（重なり時間msの分布等）が無いため
見送り、既定0%を**恒久的な既定値として確定**した（`tuning-constants.md`の
実測義務に従い、根拠のない引き上げは行わない）。

**関連ファイル:** `src/engine/engine.rs`, `src/engine/key_lifecycle.rs`,
`src/engine/nicola_fsm.rs`, `src/engine/output_history.rs`,
`src/engine/timing.rs`。関連: ADR-020, ADR-112。

---

## BUG-102: 起動直後にフォーカスしていたアプリの `ImmCrossProbe`（High）観測が導出から外れ、Medium の定期ポーリングに負ける（bootstrap フェンス desync）

**症状:** IMM32 系アプリ（`AppImeProfile::Standard`、メモ帳等の Win32 アプリや
Qt/LINE のような ImmCross アプリ）にフォーカスがある状態で awase を起動すると、
そのウィンドウでの `ImmCrossProbe`（child hwnd の非同期読み取り、High confidence /
`ActuatingPool`）観測が `ObservationStore::derive_any()` /
`derive_actuating()` の導出対象から外れ続ける。ユーザーが**一度別プロセスの
ウィンドウへ切り替えて戻る**（= `on_focus_process_changed` が `FocusChanged` を
dispatch する）まで直らない。同一プロセス内でウィンドウだけ切り替えても
（`FocusHwndUpdated` 経路）epoch が揃わないため直らない。

実害は 2 経路。**「観測が1件も belief に反映されない」わけではない**（初版の
記述は誤り。2026-08-31 の敵対的レビューで訂正）:

1. **`ImeModel::resolve_open_at`（→ `effective_open()`）**: 解決順が
   `explicit_intent` → `derive_any` → `most_recent_trusted`（**フェンス照合なし**）
   → `desired_open` なので、`ImmCrossProbe` 以外に fresh な観測が無ければ
   `most_recent_trusted` が同じ `ImmCrossProbe` を拾い直し、**値としては正しい
   まま**（症状が出ない）。一方 `ObserverPoll`（Medium、`is_identity_ok` の
   対象外なのでフェンスに関係なく通る）等が併存すると、`derive_any` がその
   Medium 単独合意を返し、**本来 High 単独で即採用されるはずの `ImmCrossProbe`
   を上書きする**。実 IME 状態と食い違ったまま最初の入力が行われうる
   （belief=ON × 実 IME=OFF ならリテラル、逆なら意図しない変換）。
2. **`state/open_warrant.rs::issue_open_warrant` Step 3（`derive_actuating`）**:
   こちらには `most_recent_trusted` フォールバックが無い。`ImmCrossProbe` が
   外れると、warrant の根拠が `DirectRead(ImmCrossProbe)` から Medium の
   `SingleIndirect` へ落ちるか、Actuating 観測が他に無ければ Step 3 自体が
   飛ばされ Step 4a/4b の `HeuristicGuess`（あるいは warrant 不発行）まで
   劣化する。**競合する Medium 観測が無くても劣化するのはこちら**。

**`FocusProbe` は影響を受けない:** `is_identity_ok` は `FocusProbe` も照合対象に
するが、`FocusProbe` は `Low` / `BeliefPool`（`state/evidence.rs`）であり、
`derive_filtered` の High 分岐（`confidence == High`）にも Medium 分岐
（`confidence >= Medium`）にも元から載らない。したがってフェンスが一致しても
しなくても導出の結論は変わらない（belief へは `most_recent_trusted` 経由でのみ
効き、そこにフェンス照合は無い）。初版の症状記述はここを取り違えていた。

**再現条件:** 起動時点で `Standard` プロファイルのアプリがフォーカスされている
こと。belief の値がずれるのは、さらに fresh な Medium 観測（`ObserverPoll` /
`Gji` / `Tsf` / `HwndCache` / `ConvOpenInference`）が `ImmCrossProbe` と食い違う
値で併存する場合。

**この desync は無音である（発見が遅れた理由）:** 棄却は
`ObservationStore::derive_filtered` の `is_identity_ok` で起きるが、そこで
`log::debug!` が出るのは「epoch は一致するのに hwnd だけ不一致」のケースだけで、
本件は epoch も不一致（0 vs 1）のため**ログが1行も出ない**。
`probe_admission::drain_stats()` の棄却カウンタも増えない ——あちらは probe の
admission（spawn 時の live フェンス vs 完了時の live フェンス）を数えており、
本件では両方 `{1, 実 hwnd}` で正常に受理されるため。つまり「観測は受理されたのに
導出に載らない」という、既存のどの計装にも現れない形で消える。さらに
`resolve_open_at` が `most_recent_trusted` へフォールバックすると**値としては
正しくなってしまう**ため、`effective_open()` を眺めても異常に見えない
（`OpenResolution::decided_by` を見て初めて `DeriveHigh(ImmCrossProbe)` では
なく `MostRecentTrusted(ImmCrossProbe)` / `DeriveMedium { ObserverPoll }` に
なっていることが分かるが、この値はログにも journal にも出ていない）。

**原因:** フォーカス同一性フェンス（`FocusFence { epoch, hwnd }`、ADR-106 決定3）の
**live 側と観測ストア側が bootstrap でだけ食い違う**。

1. `establish_initial_focus_scope`（ADR-102 決定3-b、起動時に1度だけ呼ばれる）は
   `advance_focus_tracking` → `update_focus_info` で
   `platform.focus.current.hwnd` に実 hwnd を入れ、続く `enter_focus_scope` で
   `FocusStore::focus_epoch` を 0→1 に進める。したがって
   `Runtime::focus_fence()`（= probe チケットのスタンプ元、live 側）は
   `{epoch: 1, hwnd: 実 hwnd}` になる。
2. 一方 `ImeModel::observations`（`ObservationStore`）の `current_fence` は
   `clear_on_focus_change()`（`ImeEvent::FocusChanged` 経由）と
   `update_focus_window()`（`ImeEvent::FocusHwndUpdated` 経由）でしか動かない。
   bootstrap は前者を呼ばず（belief を書いてしまうため意図的に避けている）、
   後者も `notify_focus_hwnd_updated_if_needed` が `is_bootstrap` で明示的に
   skip する。結果 `FocusFence::default()`＝`{epoch: 0, hwnd: HwndId::NULL}` の
   まま残る。
3. `ObservationStore::derive_filtered` の `is_identity_ok` は `FocusProbe` /
   `ImmCrossProbe` 観測についてのみ「観測のフェンス == `current_fence`」を要求する。
   live 側でスタンプされた観測（`{1, 実 hwnd}`）は既定値（`{0, NULL}`）と一致
   しないため、High confidence であっても stale として除外され続ける
   （実際に結論が変わるのは `ImmCrossProbe` だけ。上の「`FocusProbe` は影響を
   受けない」参照）。
4. 復旧経路が `FocusChanged`（プロセス変更）しかないのは、`FocusHwndUpdated` が
   hwnd しか更新せず epoch=0 のまま残るため。

**なぜ「起動時だけ」なのか:** epoch/hwnd の同期は、定常経路では
`on_focus_process_changed` が `enter_focus_scope`（epoch++）の直後に
`ImeEvent::FocusChanged { focus_epoch }` を dispatch することで必ずペアになって
いる。bootstrap は「belief を書かない」ために `FocusChanged` を dispatch しない
設計（ADR-102 決定3-b）だが、その際に**belief ではない fence の同期まで一緒に
落ちていた**。

**修正:** fence の同期だけを行う専用イベント
`ImeEvent::InitialFocusFenceEstablished { fence: FocusFence }` を新設し、
`establish_initial_focus_scope` が `enter_focus_scope` の直後に
`sync_initial_focus_fence()` から1度だけ dispatch する。reducer のアームは
`ObservationStore::establish_initial_fence(fence)`（`current_fence` 1 フィールドの
差し替え。観測プール・drift・belief には触れない）だけを呼ぶ。

この同期は ADR-102 決定3-b の「最初の IME 観測より前に belief を書き換えない」に
抵触しない ——運ぶ値は「どの窓のどの epoch を見ているか」という**観測の新鮮さを
判定するための識別子**であり、IME が ON か OFF かの推測を一切含まないため。
`clear_on_focus_change()` を流用せず新しい入口にしたのは、bootstrap がフォーカスの
「変更」ではなく最初のスコープ確立であり、捨てるべき旧窓の観測が存在しない
（この時点でプールは空。`app/bootstrap.rs::run_all` はこの呼び出しより前に
メッセージループをポンプしない）ことを型ではなく入口名で表すため。

**テスト:**
- `state::ime_model::tests::bootstrap_fence_desync_lets_medium_poll_override_high_probe`
  — **実害そのもの**（上の実害 1）を `resolve_open_at` の粒度で固定。
  `ImmCrossProbe(true, High, live fence)` と `ObserverPoll(false, Medium)` を
  併存させ、同期前は `DeriveMedium { ObserverPoll }` で `false`、同期後は
  `DeriveHigh(ImmCrossProbe)` で `true` になることを固定する。
- `state::observation_store::tests::establish_initial_fence_unblocks_the_first_probe_after_bootstrap`
  — ストア単体での退行再現。既定値フェンスのままだと live 側フェンスでスタンプ
  された High 観測が `derive_any()` から外れること、`establish_initial_fence()`
  後に受理されることを固定（`derive_any` 粒度なので、belief 値としての症状の
  有無は上のテストが担当）。
- `state::observation_store::tests::establish_initial_fence_does_not_clear_the_pool_or_drift`
  — `clear_on_focus_change` との差（プール・drift を消さない）を固定。
- `state::ime_model::tests::initial_focus_fence_established_touches_only_the_fence`
  — reducer アームが `current_fence` 以外を一切書き換えないことを、モデル全体の
  `Debug` 表現の一致で機械的に固定（フィールド追加時の assert 書き漏れ対策）。
  フィクスチャ（`fully_populated_model`）は **`ImeModel` の全フィールドを既定値
  から動かす** ——既定値のままのフィールドへ既定値を書き戻す巻き添えは
  `Debug` 比較では原理的に検出できないため。
- `tests/architecture_guard.rs::establish_initial_focus_scope_syncs_the_observation_fence`
  — bootstrap が同期をちょうど1回、`enter_focus_scope` と
  `advance_focus_tracking` の**両方の後**に呼ぶこと。
- `tests/architecture_guard.rs::initial_focus_fence_event_only_touches_the_fence`
  — `InitialFocusFenceEstablished` と `.establish_initial_fence(` の出現箇所を、
  固定ファイルへの grep ではなく **`src/` 全ファイル走査**で固定する。
- `tests/architecture_guard.rs::establish_initial_focus_scope_does_not_write_ime_belief`
  — `sync_initial_focus_fence` を対象関数リストへ追加し、その `dispatch_event`
  がちょうど1件・`InitialFocusFenceEstablished` であることを固定。

**検証状況:** 実機 Windows 環境が無いため、起動→IMM32 アプリでの観測受理は
**未検証**。根拠はコードリーディング（フェンスの生成元 `Runtime::focus_fence()`、
スタンプ点 `AcceptedObservation`、照合点 `is_identity_ok` の3点突合）と上記
ユニットテストのみ。実機（`RUST_LOG=debug`）での確認手順:

1. **同期が起きたこと**: 起動直後に
   `[focus-fence] bootstrap initial fence: FocusFence { epoch: 1, hwnd: ... }` と
   `[focus-fence] establish_initial_fence: FocusFence { epoch: 0, hwnd: HwndId(0) }
   -> FocusFence { epoch: 1, hwnd: ... }` が1回ずつ出て、hwnd が
   `[keymap] active rules updated: ... (hwnd=...)` の値と一致すること。
2. **実害が消えたこと**: `effective_open()` が観測に追従しているかを見るのは
   **偽陽性になる**（修正前でも `most_recent_trusted` フォールバックで同じ値に
   なるため、実害 1 のケース A では修正前後で区別がつかない）。
   `ObserverPoll` と食い違う状況を作って確認すること —— IMM32 アプリを
   フォーカスした状態で awase を起動し、起動直後（`ObserverPoll` の 500ms
   周期と `ImmCrossProbe` が両方 fresh な窓）に IME を外部操作で切り替えて
   最初の1文字を入力し、`ImmCrossProbe` が読んだ側の状態で入力されること。

**修正履歴:** `fix/bootstrap-focus-fence-desync` ブランチ（本エントリ記録と同一
コミット。自己参照になるため本文にハッシュは書かない —— `git log --grep BUG-102`
で引ける）。**混入元は `f370b0ca`**（`feat(hook): route engine-thread
notifications via HWND, close key-delivery gaps (ADR-105/102)`、2026-08-26）で
`establish_initial_focus_scope` を新設した瞬間。それ以前は bootstrap で
`focus_epoch` を進める処理自体が無く、両側とも 0 で一致していた。epoch 軸の
フィルタ（`a0e4ec78`）はそれより前から存在したので、**epoch 軸の不一致だけで
既に決定的に成立していた**。hwnd 軸の追加（ADR-106 決定3、`05e2e720`）は
不一致の軸を 1 本増やしただけで、確実性を上げたわけではない（初版の記述は
この点を取り違えていた）。

**関連ファイル:** `crates/awase-windows/src/runtime/focus_tracking.rs`,
`crates/awase-windows/src/state/observation_store.rs`,
`crates/awase-windows/src/state/ime_model.rs`,
`crates/awase-windows/src/state/ime_event.rs`,
`crates/awase-windows/src/state/probe_admission.rs`,
`crates/awase-windows/src/state/transition.rs`。実害の受け手（本修正では未変更）:
`crates/awase-windows/src/state/open_warrant.rs`（Step 3 = `derive_actuating`）,
`crates/awase-windows/src/state/evidence.rs`（`FocusProbe` = Low の根拠）。
関連: ADR-102 決定3-b（2026-08-31 追補）, ADR-106 決定3・§冒頭（同追補）, BUG-91。

---

## BUG-103: `[[post_bypass]]` は `reload_config()` で反映されない（設定変更に再起動が必要）

**症状:** `config.toml` の `[[post_bypass]]` セクションを変更して設定を保存
（reload）しても、変更が反映されない。awase を再起動して初めて新しい
`[[post_bypass]]` ルールが有効になる。

**原因:** `Runtime::post_bypass_rules`（コンパイル済み `[[post_bypass]]`
ルール一覧）は `bootstrap.rs` でのみ構築され、`apply_config_update`
（`runtime/mod.rs`、`reload_config()` から呼ばれる）は一切この値に触れない。
`[[keymap]]` も ADR-114 実装前は同じギャップを抱えていたが、ADR-114 決定8
（`Runtime::all_keymaps`/`recompute_active_keymaps()`）で解消済み。
`[[post_bypass]]` 側は本 ADR のスコープ外として意図的に対象外にした
（[[keymap]] のギャップ解消が優先度が高いと判断したため）。

**再現手順:**
1. `[[post_bypass]]` ルールが未設定の状態で awase を起動する。
2. 設定画面（またはトレイの「設定リロード」）から `config.toml` に
   `[[post_bypass]]` セクションを追加して保存・reload する。
3. 対象のキーコンボを押しても、ルールが有効になっていない
   （追加前と同じ挙動のまま）。
4. awase を再起動すると、追加したルールが有効になる。

**修正方針（未実装）:** `apply_config_update` に `[[post_bypass]]` のコンパイル
（`bootstrap.rs` の `post_bypass_rules` 構築ロジックと同等）を追加し、
`Runtime::post_bypass_rules` を差し替える。`[[keymap]]` の
`recompute_active_keymaps()` と同様、書き込み点が複数に増えないよう
1箇所に集約すること。

**状態:** 未修正。`[[keymap]]` 側（ADR-114）の実装と同時に発見・記録した。

**関連ファイル:** `crates/awase-windows/src/app/bootstrap.rs`（`post_bypass_rules`
構築）, `crates/awase-windows/src/runtime/mod.rs`（`apply_config_update`,
`post_bypass_rules` フィールド）, `crates/awase-windows/src/app/mod.rs`
（`reload_config`）。関連: ADR-114（`[[keymap]]` 側の同種ギャップの解消）,
ADR-037（`[[post_bypass]]`/`[[keymap]]` 共通の「アプリ+キースコープの横取り」
形状）。

---

## BUG-104: 独自 `.yab` レイアウトが UTF-8 でないと起動時に無言でバンドル版へ差し替わる

**症状:** タスクトレイの不具合報告機能経由の report `01M13EACMQ7D2VETW75N0BTZ9C`
（2026-08-28T05:28:03Z、app_version 1.16.1、GJI）で報告された4件のうち、**BUG-95
（`.yab`のクォート崩れリテラルが無警告で受理される）とは別の、独立した原因**。
ユーザーが独自にカスタムした `NICOLA＋確定.yab`（句読点セルに `CV4D` サフィックス
を試験的に追加したもの）を `default_layout` に指定していたが、awase.log には

```
startup: レイアウト読込失敗: ...\layout\NICOLA＋確定.yab: stream did not contain valid UTF-8
Available layouts: ["nicola", "nicola_f", "nicola_us"]
```

とあり、`NICOLA＋確定.yab` はファイル読込の時点（UTF-8 デコード）で失敗し
`Available layouts` からも除外され、実際に有効化されたのはソート順で先頭の
バンドル版 `nicola.yab` だった。ユーザーへの通知はトレイバルーン「1件の警告が
あります」の一文のみで、どのファイルが何の理由で無視されたかは awase.log を
開かないと分からない。report の `description` にはこの読込失敗への言及が一切
なく、ユーザーは自分の独自レイアウト（句読点 `CV4D` 含め）が丸ごと無視され
バンドル版が使われていることに気づいていない様子だった。

**BUG-95 との関係（重要、要注意）:** BUG-95 は同じ report の症状「『ぶ』が
入力できず『b』になる」を、添付 `layout_yab`（右親指シフト面 V 位置の誤字
`ｂ'ｕ`）が実行時に読み込まれ `Literal` としてそのまま解釈された結果と説明して
いる。しかし本 BUG-104 の調査で判明した通り、**その同じファイルは UTF-8
デコードの時点で読込自体に失敗しており、実際に有効化されていたのはバンドル版
`nicola.yab`（`ｂｕ`→「ぶ」の対応は正しい）だった可能性が高い**。両者は
report 添付の `app_log_excerpt` から得られる事実であり矛盾する。BUG-95 の
調査時点ではこの起動ログ（`stream did not contain valid UTF-8`）が参照されて
いなかった模様（BUG-95 エントリに言及がない）。「ぶ」欠落症状の真因は
**未確定に戻す必要がある**——BUG-95 の `ｂ'ｕ` 誤字説と本 BUG-104 のフォール
バック説のどちらか、あるいは両方が起きた可能性（例: フォールバック発生前の
古いバージョンで再現していた、等）を次回セッションで re-verify すること。
（report 添付の journal は起動直後14秒分・60件のみで実際の打鍵記録
`KeyInput` が1件も無く、いずれの説も打鍵ログでは検証できていない。）

**原因:**

- `LayoutEntry::scan_all`（`crates/awase-windows/src/app/bootstrap.rs`）は
  `.yab` の読込/パースに失敗したファイルを診断ログに残すのみで
  `layouts` に加えない。
- `LayoutEntry::resolve_index`（`crates/awase-windows/src/runtime/mod.rs`）は
  `default_layout` と同名のエントリが見つからない場合、**修正前は**
  ソート順先頭のレイアウトへ無言でフォールバックしていた。
- 通知はトレイバルーン「N件の警告があります」のみで、内容（どのファイルが・
  なぜ）は awase.log にしか残らなかった。

**修正:**

- `LayoutEntry::resolve_index` のフォールバック先を、ソート順先頭ではなく
  `nicola_keytop`（存在すれば。新規インストールの既定と同じ、`layout/
  nicola_keytop.yab`）優先に変更。無ければ従来どおりソート順先頭。
- `bootstrap::warn_layout_fallback` を新設し、実際に選ばれたレイアウトが
  `config.general.default_layout` と一致しない場合（＝フォールバックが
  発生した場合）、モーダルダイアログ（`crate::win32::show_error_dialog`）で
  「設定されたレイアウト『X』を{読み込みに失敗しました/見つかりませんでした}。
  代わりに『Y』を使用しています」と明示する。起動時（`init_engine_validated`）
  はこのダイアログの直後に `launch_settings()` で設定画面を自動起動する
  （`show_no_layouts_dialog` と同じパターン）。設定リロード時
  （`reload_config`）は同じダイアログを出すが設定画面の自動起動はしない
  （原因が直らない限りリロードのたびに新しい設定画面プロセスが際限なく
  spawn されるのを避けるため。`launch_settings_with_args` に多重起動防止が
  無い）。
  - 起動を中断する案（異常終了）も検討したが、awase はスタートアップ登録
    されうる常駐ソフトであるため、原因未修正のまま自動起動されるたびに
    サイレントに落ち続け、NICOLA 入力が長期間使えないことに気づかれない
    リスクがあるとして不採用（ユーザー判断）。フォールバックで動作は継続
    させ、モーダル＋設定画面の自動起動で気づかせる方針を採用した。

**テスト:** `crates/awase-windows/src/runtime/mod.rs` の
`layout_entry_tests::resolve_index_prefers_nicola_keytop_when_default_layout_not_found`
で新しいフォールバック優先順位を固定。`cargo check --target
x86_64-pc-windows-msvc -p awase -p awase-windows`・
`cargo clippy --target x86_64-pc-windows-msvc -p awase-windows -- -A
clippy::cargo`・`cargo test -p awase-windows --test architecture_guard --test
layer_boundary_guard`（Linux host、62+8件green）確認済み。`warn_layout_fallback`
自体（`MessageBoxW` を呼ぶ）は Windows 専用のため Linux では実行不可、
Windows 実機での確認は未実施。

**関連ファイル:** `crates/awase-windows/src/app/bootstrap.rs`
（`warn_layout_fallback`, `init_engine_validated`）、
`crates/awase-windows/src/runtime/mod.rs`（`LayoutEntry::resolve_index`）、
`crates/awase-windows/src/app/mod.rs`（`reload_config`）。関連: BUG-95（同じ
report、上記「BUG-95 との関係」参照）。

---

## FEATURE-115: 打鍵列機能（ADR-115）実装状況・既知の限界

新機能のため「不具合」ではなく実装状況・既知の限界の記録（ADR-115決定9(c)の
証拠義務）。関連: [ADR-115](adr/115-yab-keystroke-sequence.md)、
[GitHub Issue #118](https://github.com/cuzic/awase/issues/118)。

**実装状況（2026-08-31時点）**: core（`awase`クレート、Linux上でテスト可能）は
実装完了。`CtrlChord`（`C`+`V`+hex）・セル内`+`区切り（`InlineSequence`）・
名前付きマクロ（`KeystrokeMacro`）・`resolve_keystroke_syntax`・
`flatten_actions`統一・`release_only`網羅match化・投機出力ガード（決定7）・
`LayoutEntry::scan_all`への配線まで完了。既定`keystroke_sequence = "off"`の
ためマージ後もデフォルト挙動は無変化。

**未実施**:
- Windows実機ソーク（`CtrlChord`がMS-IME/GJI双方で実際に確定を引き起こすか、
  TSF-nativeアプリでの順序・タイミング、押下中の物理修飾キーとの衝突）
- `awase-macos`/`awase-linux`の`send_keys`は`CtrlChord`/`Sequence`をログ
  出力のみのスタブとして実装（決定10、実送信は本ADRのスコープ外）

**実装後Opusレビュー1周目で発見・修正（2026-08-31、修正コミット `866f38657`）**:
- **M1**: `crates/awase-windows/src/platform.rs`の`send_keys`が
  Unicode injection mode + GJI long-cold時に`unicode_cold_defer`で
  `Char`のみ遅延バッファへ積み、`CtrlChord`/`Key`/`SpecialKey`は
  即時送信していたため、`'（'+CV4D+'）'+CV4D+左`のような混在バッチで
  実行順序が入れ替わる欠陥があった。1周目の修正はバッチ内が`Char`/`Romaji`
  のみの場合に限り defer を有効化する条件（`all_defer_safe`）を追加したが、
  これは過剰に広く、後述の2周目レビューで既存パターンを巻き込む新規回帰と
  判明したため2周目で訂正した。
- **M2**: `awase-settings`（GUIプレビュー、`load_yab_layout`）が
  `resolve_keystroke_syntax`を呼んでおらず、キルスイッチ`off`でも
  GUIが`Ctrl+4D`/`@name`等の新構文表示のまま（実エンジンは`Literal`に
  復元済み）というプレビューと実挙動の食い違いがあった。1周目の修正は
  `load_yab_layout`の戻り値（`self.layout`として保存され`layout_write_to_path`
  でそのまま`.yab`へ書き戻される対象）に`resolve_keystroke_syntax`を通したが、
  これが致命的なデータ破壊回帰を生んだため2周目で全面的に差し戻した
  （詳細は次項）。
- **M3**: 決定7の投機ガード拒否経路2箇所（`confirm_policy.rs::idle_speculative`、
  `nicola_fsm.rs::on_timeout_speculative`）と決定5の`flatten_actions`の
  end-to-end確認（`Sequence`セルが実際に複数`KeyAction`として出力される
  こと）にテストが無かった。3件追加（`src/engine/confirm_policy.rs`・
  `src/engine/nicola_fsm.rs`・`src/engine/tests.rs`）。この項目は2周目でも
  訂正なし。

**実装後Opusレビュー2周目（1周目の修正自体を対象、3観点が独立に同一の
2件を検出、2026-08-31）**:
- **M1再訂正**: 1周目の`all_defer_safe`（バッチ全体がChar/Romajiのみか）は
  条件が広すぎた。実際に安全性を左右するのは「Char/Romajiより後に
  非defer対象（CtrlChord/Key/KeyUp/SpecialKey）が続くかどうか」のみ——
  `retract_and_replace`が出す既存パターン`[Backspace, Char]`
  （ADR-115以前から存在、非defer対象が先）は本来 defer 安全なのに、
  1周目の条件はこれも一律で defer 無効化していた。GJI long-cold + Unicode
  injection modeという条件が重なると、この既存パターンで warmup 保護
  （BUG-02系の`bあ`部分リテラル化再発防止）が効かなくなる新規回帰で、
  `keystroke_sequence = off`（既定）でも発生しうる。修正: 「Char/Romajiを
  見た後に非defer対象が続かない」ことだけを判定する走査に置き換えた
  （`crates/awase-windows/src/platform.rs::send_keys`）。
- **M2revert**: `load_yab_layout`が返す`YabLayout`は GUI 編集の作業コピーで
  あると同時に保存時の書き戻し対象そのもの。1周目の`resolve_keystroke_syntax`
  呼び出しはこれを解決済みの値で上書きしてしまい、`InlineSequence`/
  `MacroRef`が`Literal`/`Sequence`に変わった状態で`.yab`へ`serialize()`
  される（`Sequence`は`serialize()`が`無`を返す設計のため、キルスイッチ
  `on`でレイアウトを開いて保存すると打鍵列セルが丸ごと`無`に、`off`でも
  `raw`の一部だけが残る形で破壊される）。「未編集セルの無損失往復は
  保証される」という決定9(a)の主張と直接矛盾する回帰だった。修正:
  `load_yab_layout`から`resolve_keystroke_syntax`呼び出しを削除し、
  `self.layout`は常に未解決の生パース結果を保持するよう2周目で全面的に
  差し戻した。GUIプレビューが`off`時の実際の送信内容と食い違う点
  （元のM2指摘）は非破壊的な既知の制約として残し、下記へ記載する。
- **新規発見（Linux/macOS配線漏れ）**: `crates/awase-linux/src/main.rs`・
  `crates/awase-macos/src/main.rs`の起動時レイアウト読み込みが
  `resolve_kana()`のみで`resolve_keystroke_syntax`を呼んでおらず、
  これら2プラットフォームではキルスイッチ`off`（既定）でも新構文セルが
  未解決のままエンジンへ到達していた（`InlineSequence`/`MacroRef`は
  `Suppress`化、`+`を含む既存セルもパース時点で無条件に`InlineSequence`
  化されるため巻き込まれうる）。両ファイルに`resolve_keystroke_syntax`
  呼び出しを追加して解消。

**既知の限界（決定として受け入れ済み、バグではない）**:
- GUIでの新構文authoringは非対象。`awase-settings`のテキスト欄で
  `CtrlChord`/`InlineSequence`/`MacroRef`セルを編集すると、保存時に
  打鍵列としての意味を失う（決定9(a)）。未編集セルの無損失往復は保証される。
- セル内`+`区切りで`Romaji`要素を含む列（例: `ｋａ+CV4D`）は、単体`ｋａ`
  セルと違いn-gram文脈・同時打鍵のkana依存閾値調整から外れる
  （`lookup_face`のkana抽出が`Sequence`に対して常に`None`を返すため、
  決定7）。実害は次のキーのタイブレーク精度が僅かに落ちる程度。
- `KeyAction::romaji()`（`src/types.rs`）の非網羅性は対応しない方針を
  確定（決定6、`OutputEntry.romaji`はproductionコードで一度も読まれない
  dead codeのため実害なし。将来的には[ADR-045](adr/045-dead-field-detection-policy.md)
  の対象として整理しうる）。
- 通常面が`CtrlChord`単体セルの場合、投機出力ガード（決定7）は`Sequence`
  のみ拒否するため投機出力される。閾値内に親指キーが来ると
  `retract_and_replace`がBACKSPACE 1発で取り消そうとするが、IME確定は
  BSで戻らないため確定済み文字を1つ余分に消す（既存の`Special(Enter)`
  セルと同型のリスクで退行ではないが、`CtrlChord`にも当てはまることを
  実装後レビューで確認）。
- `KeystrokeMacro.steps`の各要素は`parse_cell`ではなく`YabValue::parse`
  を通るため、ステップ文字列に`+`を書くと無警告で`Literal`（先頭1文字
  のみ）になる。マクロステップ内で打鍵列合成をしたい場合は`@`で別マクロ
  を参照する形にする必要がある（現状は無警告、将来的に警告追加を検討）。
- `awase-settings`のGUIプレビュー（レイアウトタブのグリッド表示）は
  `resolve_keystroke_syntax`を通さない生の`YabLayout`をそのまま表示する
  ため、キルスイッチ`off`時でも`CtrlChord`/`InlineSequence`/`MacroRef`
  セルは新構文のまま（紫色）表示され、実際に送信される`Literal`内容とは
  一致しない（M2の元指摘）。GUIから保存する対象がまさにこの生レイアウト
  であるため、プレビューを解決済みにすると保存が壊れる（M2revert参照）。
  非破壊を優先しこの表示上の不一致は許容する——正確なOff時挙動を
  確認したい場合は`resolve_keystroke_syntax`のロジック
  （`src/yab/mod.rs`）を読むか、実機で`keystroke_sequence = "off"`の
  ままテキストエディタで.yabを確認すること。

**関連する既存の再発ファミリーとの接点**: `OutputHistory`のKeyUp整合性索引
（BUG-101/ADR-112）と同じ危険領域に触れるため、打鍵列のマクロステップ/
セル内`+`区切り双方から生の`KeyAction::Key`/`KeyUp`を構造的に排除している
（決定6・決定2c、r5レビューCritical C1の回帰テスト`resolve_on_rejects_vk_inside_inline_sequence_with_warning`で固定）。
---

## BUG-105: NICOLA 3鍵仲裁が char1 解放済みなら無条件で char2 側を優先し、タイトな重なりでも無視する

**症状:** GJI環境で「しょうにん」と入力したかったが「しいゔにん」になった。
`layout/nicola_keytop.yab`（かな44キー配置は`layout/nicola.yab`と同一）で、
L(無シフト)+右親指シフト="ょ"(romaji "lyo")のはずが、L単独="い"+A(無シフト)
+右親指シフト="ゔ"という誤出力になった。タスクトレイの不具合報告機能
（[ADR-095](adr/095-tray-bug-report-cloudflare-intake.md)）経由の
report `01M1GDQVBET5DBX3MY4BRGQFW1`（2026-09-02T06:42:31Z、app_version 1.17.0）
で報告され、GitHub issue #140として起票された。

**原因:** `src/engine/nicola_fsm.rs::compute_prefer_char1()`（3鍵仲裁:
char1→thumb→char2 の並びで thumb をどちらとペアリングするか決める純粋関数）に

```rust
if char1_released_at.is_some() {
    return false;
}
```

という早期returnがあり、char1 が char2 到着より前に物理的に離されていれば、
`TimingJudge::three_key_pairing()`（d1/d2タイミング比較 + bigram/trigram
n-gramタイブレーク）を一切呼ばず無条件で char2 側を優先していた。

report の実測: L down → RightThumb down（d1=11.7ms、極めてタイトな重なり）→
L up（char1解放）→ A down（d2=100.8ms、RightThumbはまだ押下中）。d1 が
極めて短く同時打鍵の意図が明白であるにも関わらず、char1 が char2 到着前に
離されていたというだけで早期returnが発火し、`three_key_pairing`のd1/d2
タイミング比較（`d1+margin(30ms) < d2` → 41.7ms < 100.8ms でchar1優先に
なるはずだった）が一度も実行されなかった。app.log の実ログ

```
[engine-input] vk=0x4C KeyUp   ts=142691057291us state=PendingCharThumb(char=0x4C,thumb=0x1C,left=false,released_at=None)
[engine-input] vk=0x41 KeyDown ts=142691063215us state=PendingCharThumb(char=0x4C,thumb=0x1C,left=false,released_at=Some(142691057291))
Timer killed: logical=1, os_id=14781
send_keys: mode=Tsf actions=[Char('い'), Char('ゔ')]
```

が、`Engine::on_input()`が物理KeyDownを`PendingCharThumb`状態のまま処理し
（タイムアウト経由ではない）、`step_pending_char_thumb_3key`のchar2優先分岐
（char1単独出力+char2+thumb出力を1バッチでemitするパターン）を実際に踏んだ
ことを直接裏付けている。

この早期returnはPR #85 (`b1e7474e`)で「3鍵ケースにはchar2という直接証拠が
あるため粗い既定で十分、`char_thumb_chord_confirmed`(2鍵ケース)とは意図的に
非対称」として導入されたが、[ADR-112](adr/112-keyup-lifecycle-fsm-delivery.md)
のBUG-101/決定2 land（2026-08-31）以前は`char1_released_at`がKeyUp配送の
リグレッションで恒久的に`None`だったため、**この早期return自体が実運用で
一度も検証されないまま存在していた**。report `01M1GDQVBET5DBX3MY4BRGQFW1`
（2026-09-02）は、ADR-112 land後にこの経路が実際に踏まれ誤りと判明した
最初の実例である。

また、本番既定`min_overlap_margin_percent = 0`（ADR-112決定1、決定3で恒久
固定確定済み）の下では、2鍵ケースの`char_thumb_chord_confirmed`→
`overlap_only_verdict`も`char1_released_at`の値に関わらず常に`Some(true)`
を即returnしn-gramタイブレークには到達しない。つまり修正前の時点で
`char1_released_at`がペアリングの結論を左右する箇所はこの早期return
1箇所だけであり、「2鍵ケースが持つ慎重さを3鍵ケースが失う」という理解は
誤りだった。

**修正:** 早期returnを削除し、`char1_released_at`の有無に関わらず常に
`three_key_pairing()`で判定するようにした。`compute_prefer_char1`の
シグネチャから`char1_released_at`引数を削除。

副次的に、`char1_released_at.is_some()`のときchar1の物理KeyUp（既に
`handle_key_up_pending_char_thumb`でConsume済みでOSに届かない）を補完する
`append_key_up_for`呼び出しが、3鍵仲裁の全4分岐（thumb到着/char1優先/
char2優先(面定義あり)/char2優先(面定義なし)）に無かったことも判明したため
（早期return削除により到達頻度が「稀」から「高速打鍵の常態」に上がるため
放置できない）、`commit_char1_output`という共有ヘルパーを新設し4分岐全てに
適用した。`ParseAction`の`record`フィールド経由の遅延適用
（`ShiftReduceParser::parse`が`decide()`の戻り値から`on_reduce`越しに
適用する通常経路）では`append_key_up_for`が依存する
`output_history.remove_by_scan`が対象エントリをまだ見つけられないため、
`commit_char1_output`内で`update_history`を即座に呼んでから
`append_key_up_for`を呼ぶ順序にし、呼び出し元は`record: OutputUpdate::None`
を返す設計にした（実装レビューで発覚した罠、順序を逆にすると補完が
no-opになるかspurious KeyUpを積む）。

**棄却した代案**: 「char1との物理的重なりが不足するときだけchar2を優先する」
という代案（2鍵ケースの`overlap_only_verdict`相当を3鍵にも適用）も検討したが、
`min_overlap_margin_percent = 0`が恒久固定のため現状は「常に重なり十分」に
退化しblanket削除と完全に同一の挙動になる（no-op）。有効化には3鍵専用の
重なりマージンの実測データが要り、
[tuning-constants](../.claude/rules/tuning-constants.md)の実測義務に抵触する。

**既知の限界（残る誤判定クラス）**: `three_key_pairing`は
`char1_ts`/`thumb_ts`/`char2_ts`という3つのkeydownタイムスタンプのみを見る
純粋関数で、release時刻の概念を構造的に持たない。したがって「char1との
重なりは乏しいがd1(down-to-down)はたまたま短い」ケース（例:
char1↓0ms→thumb↓12ms→char1↑20ms（重なり8ms）→char2↓62ms）を原理的に
区別できず、誤ってchar1+thumbを優先しうる。将来この限界が実機で顕在化した
場合、安易に「char1解放済みならchar2優先」を再導入しないこと——それは
今回撤回した早期returnの再発であり、
[experiment-logging](../.claude/rules/experiment-logging.md)エントリ01
（IME OFFキー選択が5日で6回反転した事例）と同じ轍を踏む。

**残課題（スコープ外、未対応）**: char2優先分岐（`nicola_fsm.rs`の
`step_pending_char_thumb_3key`、char2+thumbが面定義を持つケース）は、
2鍵の`step_pending_thumb_char`が持つ`is_simultaneous`チェックを持たず、
thumbとchar2の生の経過時間が`simultaneous_threshold_ms`を超えていても
無条件でchar2+thumbを同時打鍵として確定する。今回のreportはPhase 1の
タイミング比較だけで解決するため（char2到着はd2=100.8msで実際に閾値
100msをわずかに超えていた）このガード欠如が直接の原因ではなかったが、
n-gramのPhase 2がchar2側を選ぶ場合には同種の問題が残りうる。修正候補は
`TimingJudge::is_simultaneous`（既存、n-gramによる閾値動的調整込み）を
char2+thumb確定前に一度通すことで、新しいチューニング定数は不要と見込む。

**残課題2（潜在的到達性、未検証だが理論上到達可能）**: `commit_char1_output`
のKeyUp補完が必要になる条件は、`is_layout_key`（レイアウトのいずれかの面
——Normal/LeftThumb/RightThumb/Shift/LeftThumbShift/RightThumbShift——に
定義があればlayout key扱い）の判定上、char1のNormal面が未定義で
LeftThumb面にのみ定義があるキーを、RightThumbと組んだ場合に成立する
（`resolve_pending_char_as_single`がNormal面未定義で`KeyAction::Key(vk)`
にフォールバックする）。既存テスト
`key_up_with_os_modifier_held_still_emits_keyup_for_injected_vk`
（`src/engine/tests.rs`）が同型のレイアウト構成（Normal面未定義キー）を
既に作っており、理論上の話ではなく到達可能な構成であることが確認できる。

**関連ファイル:** `src/engine/nicola_fsm.rs`
（`compute_prefer_char1`/`commit_char1_output`/`reduce_char_thumb_and_continue`/
`step_pending_char_thumb_3key`）、`src/engine/timing.rs`
（`three_key_pairing`、変更なし・既存ロジックが実際に動くようになった）、
`src/engine/tests.rs`・`tests/scenarios.rs`（回帰テスト）。

**修正履歴:**
- 本修正コミット。GitHub issue #140。Opus敵対的レビュー（3ラウンド）で収束。
  回帰テスト3件追加（`src/engine/tests.rs`にPhase1/Phase2各1件、
  `tests/scenarios.rs`に`Engine::on_input`経由・実レイアウト使用1件）。

---

## BUG-106: Teams(WebView2/MS-IME) で送信 romaji VK が JIS かな配列として解釈される

**症状（issue #137、Microsoft Teams / `TeamsWebView` / WebView2 上の MS-IME）:**
awase が正しく送信した romaji VK 列が、IME 側で「ローマ字入力」ではなく
「かな入力」として解釈され、無関係な文字になる。代表例は A キー相当が
「な」、S キー相当を含む列が「とに」のように表示されるケース。

**判定シグネチャ:** 再発時はログを開く前に、送信 VK 列を JIS かな配列表で
デコードして表示結果と突き合わせる。完全一致するなら、awase の送信ミスではなく
IME 側の入力方式反転を第一容疑にする。

| 送信VK | JISかな解釈 | 観測例 |
| --- | --- | --- |
| U | な | 「な」 |
| S | と | 「とに」の先頭 |
| I | に | 「とに」の末尾 |

**BUG-08 との関係:** ROMAN 喪失により romaji VK が JIS かな化する点では同型。
ただし BUG-08 は foreign-injected `VK_KANA` まで原因を特定済みだったのに対し、
本件は Teams/WebView2/MS-IME のどこで入力方式が反転したか未特定。

**自動復旧不可:** BUG-61/BUG-62 追補4で、`ImmSetConversionStatus` による
ROMAN ビット書き込み、`VK_DBE_ROMAN` の `SendInput` 注入、言語バー COM 操作の
いずれも実機で無反応と確認済み。Microsoft Q&A の同種回答でも、Win32 には
「ローマ字入力 / かな入力」の入力方式そのものを外部プロセスから切り替える
公式 API が無い、という整理になっている。したがって awase は復旧を書き込まず、
ユーザーへの案内だけを行う。

**今回追加した実機検証:** `crates/awase-windows/examples/spike_kana_lock_probe.rs`
で、Teams focus 中に言語バーから MS-IME を「かな入力」へ切り替えると
`GetKeyState(VK_KANA)&1` が `off -> on` に追従し、「ローマ字入力」へ戻すと
`on -> off` に追従することを確認した。

```text
KANA bit=on (prev=off) fg_class=TeamsWebView
KANA bit=off (prev=on) fg_class=TeamsWebView
```

**Teams で conv 読み取り経路が存在しない根拠:** `TeamsWebView` は
`crates/awase-windows/src/focus/class_names.rs:24` で
`IMM32_UNAVAILABLE_CLASSES` に含まれる。`is_tsf_native_window` は同ファイル
`:51-59`、`is_effectively_tsf_native` は `:75-77` だが、`from_class_name` は
`IMM32_UNAVAILABLE_CLASSES` を優先するため Teams は `Imm32Unavailable` 扱いになる。
さらに `runtime/key_pipeline.rs:643-646` のとおり `kp_stage_idle_conv_check` は
TsfNative 専用かつ ROMAN ビットを信頼しない設計で、Teams の反転検知には使えない。

**`swallow_alt_kana_input_method_switch` との関係:** swallow の有無は復旧可否と
無関係。既定 ON は Alt+かな による「かなに落ちる」方向の予防であり、OFF にしても
復旧手段にはならず、むしろ入力方式反転のリスクを増やすだけ。

**対応:** `src/engine/kana_input_warn.rs` に OS 非依存のヒステリシス
（On 3連続で警告、Off 2連続で解除）を追加し、`observer/kana_lock.rs` が
`GetKeyState(VK_KANA)&1` を読み、VK/TSF の romaji 送信直前に1打鍵1サンプルで
観測する。警告時は既存トレイ右クリックメニューに「かな入力になっています
（対処方法）」を出し、ツールチップを「awase - かな入力になっています」へ変更する。
belief/engine/IMM への書き込み、`ImeEvent` dispatch、自動復旧は行わない。

**修正履歴:** `40a2d091`（2026-09-02、`feat/issue-137-teams-kana-lock-detection`）。

## BUG-107: `ImmCapabilityStore` の学習キャッシュが `class_name` のみをキーにしており、winitの汎用クラス名を介して無関係なプロセスの誤学習が `awase-settings.exe` に伝播し、テキスト入力の先頭に「あ」が混入する（BUG-56のプロセス間版）

**症状（ユーザー報告、2026-09-03）:** タスクトレイの「不具合を報告」画面
（`crates/awase-settings/src/bug_report.rs`、`--bug-report` 起動）や通常の
設定画面（`awase-settings.exe`）の説明欄に親指シフトで日本語入力すると、
**入力の先頭に意図しない「あ」が混入する**（実機で2回連続再現、再現性が
高い）。当初は「テキスト入力がとても重い」という報告も併せてあったが、
後続の再現テストでは重さは目立たなかった——両者は同時に起きるとは限らない
別要因の可能性がある（重さの側は本 BUG では未解決のまま残る、「範囲外」
参照）。

**却下した対策:** `disable_apps` に `awase-settings.exe` を追加し丸ごと無効化
する案を最初に検討したが、「eframe/egui 全体が awase 非対応という話になる」
とユーザーに却下された。詳細な調査過程（3回の実機スパイク・当初2つの仮説が
否定された経緯を含む）は [ADR-125](adr/125-egui-winit-dynamic-ime-association-focus-model-gap.md) を参照。

**原因（実機ログ・`cache.toml` 直接確認で確定）:** 実際に `awase.exe`
（`RUST_LOG=debug`）を起動した状態で症状を再現させたところ、
`target/debug/awase.log` に次の行が記録されていた:

```
[imm-learning] profile 降格: class="Window Class" Standard → Imm32Unavailable
（実測学習 ImmCapability::Unavailable。誤学習なら cache.toml の
[imm_capability] から該当クラスを削除）
```

`crates/awase-windows/src/focus/tracker.rs::apply_learned_imm_capability`
が、`ImmCapabilityStore`（`focus/classifier.rs`）に保存された
`"Window Class" = "unavailable"` という学習結果に基づき、
`awase-settings.exe` の静的分類 `Standard`
（`AppImeProfile::from_class_name` の既定フォールバック）を実行時に
`Imm32Unavailable` へ降格させていた。`target/debug/cache.toml` を直接
確認し、実際にこのエントリが存在することを確認した。

**これは BUG-56 と全く同じ機構のプロセス間版である。** BUG-56
（2026-08-07）は、Qt の汎用クラス名 `Qt663QWindowIcon` が LINE アプリ
**内**の無関係な一時ウィンドウ（通知アイコン等）と本物のチャット入力欄とで
使い回され、前者の誤判定が `class_name` をキーとする学習キャッシュ経由で
後者まで巻き込んだ事故だった。修正（`ImmCapabilityStore::
record_null_probe`、2回連続観測するまで確定しないデバウンス）は
**同一プロセス内**の衝突を主眼に設計されており、本件（**異なるプロセス間**
での衝突）には効かない。`winit`（`awase-settings.exe` が使う GUI
フレームワーク）は `with_class_name` で明示的に上書きしない限り
`"Window Class"` という**ハードコードされた既定クラス名**をあらゆる
winit アプリで共有するため、この Windows 実機上で過去に動かした別の
winit ベースのアプリ（または過去のいずれかの `awase-settings.exe`
セッション）が `Imm32Unavailable` と学習されていれば、その学習結果が
無関係な `awase-settings.exe` の別セッションにまで適用されてしまう。

実機の `cache.toml` を全体確認したところ、`Button`/`Edit`/`ComboBox`/
`Static`/`SysListView32` のような**極めて一般的な Win32 標準コントロールの
クラス名**まで多数 `unavailable` として学習されており、`class_name` 単独
キー方式の汚染リスクは `"Window Class"` に限らず構造的に存在する。

「実機検証ログ2」（ADR-125）で確認したとおり、`awase-settings.exe` に
対する実際の IMM32 クロスプロセス制御（`ImmGetDefaultIMEWnd`/
`WM_IME_CONTROL`）は正常に機能する（有効な IME ウィンドウ・50ms 以内の
応答・正確な ON/OFF 追従）。つまり `Imm32Unavailable` への降格は**誤学習**
であり、本来 IMM32 が使えないアプリ向けの VK_KANJI トグル・物理 IME キー
抑止といった制御方式が、実際には IMM32 が正常に機能するウィンドウに対して
発動してしまうことで、先頭の「あ」のような症状につながっていると考えられる
（`Imm32Unavailable` 降格後の具体的などの経路が「あ」を出力するかという
詳細メカニズムまでは未確認、ADR-125「未解決の設計課題」3参照）。

**対応（実装済み、実機ソーク未実施）:** `ImmCapabilityStore` の学習キャッシュの
キーを `class_name` 単独から `(process_name, class_name)`（実装は
`HashMap<String, HashMap<String, ImmCapability>>` のネスト構造）へ変更した
（`config.rs::AppOverrideEntry` が既に `{ process, class }` の組でアプリを
識別している既存パターンに沿う）。プロセス間の衝突を構造的に防ぐが、BUG-56
のデバウンス（同一プロセス内の衝突対策）はそのまま維持し独立に共存させて
いる。`cache.toml` の永続化形式もフラット（`class_name = "value"`）から
プロセス名でネストしたテーブル（`[imm_capability."process.exe"]` の下に
`class_name = "value"`）へ変更し、旧フラット形式のエントリは起動時に検出
すると安全に破棄したうえで新形式のみへ書き直す。実装は Codex CLI に委譲し、
Opus モデルによる読み取り専用の敵対的コードレビューを2周行って収束させた
（発見された `get_process_name` の無駄な二重呼び出し・空プロセス名による
汚染バケツの再発等、複数の実害ある問題を反映済み）。設計の詳細・実装結果・
残存する既知の制限は [ADR-125](adr/125-egui-winit-dynamic-ime-association-focus-model-gap.md)
「決定」「実装結果」節を参照。**実機での症状消失確認（実機ソーク）はまだ
行っていない。**

**残存する既知の制限（レビューで発見、対応は本件のスコープ外）:**
- `cache.toml` が新旧キー衝突等でパース不能な状態のとき、legacy 掃除は
  発動せず、その後の保存で `[injection_mode]` 等の他セクションが丸ごと
  失われるリスクが低確率ながら残る（`save_section` の既存の脆さ、詳細は
  ADR-125「実装結果」節）。
- 学習エントリがプロセス数×クラス数で増加し、上限やプルーニングが無い。
- タスクトレイの「学習キャッシュをクリア」メニュー項目が別の既存バグで
  完全な no-op になっており（`runtime/message_handlers.rs` の
  `TrayCommand::ClearImmCache` ハンドラ）、下記の暫定回避（手動での
  `cache.toml` 編集）が GUI からは行えない。

**暫定回避（BUG-56 と同じ考え方、新形式向けに更新）:** `cache.toml` の
`[imm_capability."プロセス名.exe"]` セクション、または該当プロセスの
サブテーブル内の該当クラス名のエントリを削除し、awase を再起動する
（`ImmCapabilityStore` は起動時に一度だけ `cache.toml` を読み込むため、
ファイル修正だけでは反映されない。上記のとおりトレイメニューからは
実行できない）。

**範囲外・未解決 → 2026-09-04 追補で解明:** 当初報告にあった「テキスト入力が
重い」という症状は、本 BUG の再現テストでは顕在化しなかったため未解決のまま
残るとしていたが、**BUG-111 として独立の原因（`[imm-learning]` ログの
500ms 周期無条件再発火）を特定・修正した**。また「かな混入」（本 BUG が
指すプロセス間キー汚染とは別に）本体の学習結果自体が誤ることがある、という
本節が示唆していた可能性も **BUG-112 として確定**した——`ImmGetDefaultIMEWnd`
起動直後の一時的な NULL が `Unavailable` として誤確定される競合状態であり、
BUG-107 のキー設計修正が適用済みでも独立に再現する。

**関連ファイル:** `crates/awase-windows/src/focus/classifier.rs`
（`ImmCapabilityStore`）、`crates/awase-windows/src/focus/tracker.rs`
（`apply_learned_imm_capability`）、`crates/awase-windows/src/focus/
imm_learning.rs`（`learn_imm_capability_on_focus`）、`crates/awase-windows/
src/focus/current.rs`（`CurrentFocus::update_with_process_name`）。

**関連:** BUG-56（同一プロセス内版、本件はそのプロセス間版）、BUG-33/
BUG-37（IMM32/TSF 非信頼アプリでの belief 乖離、同系統だが原因の軸が
異なる）、BUG-78（却下案が再利用しようとした `disable_apps` 丸ごと
バイパス機構の初出）、ADR-121（「原因の半分」を早期に確定として扱って
しまった教訓、本件で実装を急がず3回の実機検証を重ねた理由）、BUG-108
（本件の調査中に発見した、無関係な既存バグ）、BUG-111（「重い」の実体、
2026-09-04 追補で特定・修正）、BUG-112（「あ混入」の残存原因、2026-09-04
追補で特定）。

## BUG-108: タスクトレイの「学習キャッシュをクリア」メニュー項目が完全な no-op になっている

**症状（BUG-107 の調査中に発見、実機報告ではなくコード読解で確定）:**
タスクトレイの右クリックメニューに「学習キャッシュをクリア」という項目が
表示され、クリックできる（`crates/awase-windows/src/tray.rs:649`、
`IDM_CLEAR_IMM_CACHE`）が、実際にクリックしても**何も起きない**。

**原因（コード確認済み）:** メニュー項目のクリックは
`TrayCommand::ClearImmCache`（`tray.rs:79`）にマップされ
（`tray.rs:697`）、コマンド処理の分岐先
`crates/awase-windows/src/runtime/message_handlers.rs:1141` に到達するが、

```rust
Some(tray::TrayCommand::ClearImmCache) | None => {}
```

という何もしない分岐にまとめられており、`ImmCapabilityStore` のキャッシュ
（`cache.toml` の `[imm_capability]`）にもファイルにも一切手を付けない。
`ClearImmCache` という識別子が出現するのはこの3箇所のみで、実際にキャッシュ
をクリアする実装はどこにも存在しない。

**影響:** BUG-56・BUG-107 がいずれも「誤学習した IMM32 能力キャッシュは
`cache.toml` を手で編集して直す」ことを暫定回避として案内しているが、
この GUI 操作は実際には機能しないため、ユーザーはメニュー項目の存在に
惑わされたうえで、結局手動でのファイル編集（`cache.toml` を直接開いて
該当エントリを削除）を行うしかない。ADR-125（BUG-107 の設計 ADR）の
「未解決の設計課題」1 が「複雑な自動移行より捨てて学習し直す方が安全」と
判断した際、この GUI 操作が機能する前提はそもそも成立していなかった。

**対応:** 未対応。`message_handlers.rs:1141` で `ClearImmCache` を受けたら
`ImmCapabilityStore` のキャッシュ（メモリ上のもの、および `cache.toml` の
`[imm_capability]` セクション）を空にして保存し直す実装を追加する必要が
ある。`ImmCapabilityStore` に外部から呼べるクリア用メソッドが無いため、
新設が必要。

**関連ファイル:** `crates/awase-windows/src/tray.rs`（`IDM_CLEAR_IMM_CACHE`、
`TrayCommand::ClearImmCache`）、`crates/awase-windows/src/runtime/
message_handlers.rs`（該当 no-op 分岐）、`crates/awase-windows/src/focus/
classifier.rs`（`ImmCapabilityStore`、クリア用メソッド新設が必要）。

**関連:** BUG-56・BUG-107（この GUI 操作が機能する前提で暫定回避手順を
案内している）。

---

## BUG-109: `drain_pending_deferred_before_send_if_queue_only`（ADR-123 決定4-3）が recovery resend 自身の送信より前に発火し、GJI I/O の証拠を汚染して `StaleConfirm`→`VK_ESCAPE` を誘発、出力順反転と文字消失を引き起こす（ADR-123 決定4-3 の回帰）

**発見の経緯:** タスクトレイの不具合報告機能（ADR-095）経由、`report_id:
01M1MW0KSY5KWVYSGPGRBTNSPA`（2026-09-04、app_version 1.18.0）。症状カテゴリ
「入力した文字と違う文字が出た」、説明「なまえま　と入力すると　なま
になった」。msedge.exe（`Chrome_WidgetWin_1`、profile=TsfNative。journal の
`LiteralTarget=Chrome` は分類名であって実際のブラウザ種別を示さない）+
Google 日本語入力（GJI、Uwp フォーカス直前の focus）。

**初版の診断は因果が逆だった。** 当初「`pending_deferred` の flush は正しく
起きているのに、flush 後に生えた元モーラの stale-confirm 回収が composition
ごと巻き込んで壊す」と結論したが、Opus 敵対的レビュー（design-review、
`docs/adr/128-escape-composition-collateral-deferred-loss.md` 起票時）が
journal の `deferred_flushed: 0`（`seq 2450`）と app_log の実際の呼び出し順
から、**flush（drain）自体が recovery resend の実送信より前に発火し、それが
recovery resend 自身の per-VK confirm の証拠を汚染していた**ことを実証した。
以下は訂正版。

**重要（判定方法についての教訓）:** この report のビルドは ADR-123
（issue #148、PR #155）の修正を既に含んでいる。journal の app_log に ADR-123
決定4-3 でのみ追加されるログ行 `[pending-deferred] drain-before-send: 新規
モーラの前に取り残し N VK(s) をflush`（`output/vk_send.rs:83`、commit
`ae3af1a0`）が実際に出現しており、`app_version` 文字列だけでは pre/post
ADR-123 を判定できない（直前の triage 記録 `01M1MTDPKYMRVV6AJ73Z1JGWZQ` は
バージョン文字列だけで「ADR-123 修正マージ前のビルド」と判定していたが、
本件はその判定方法に頼れない反例）。ただし結論は「ADR-123 のスコープ外の
新規欠陥」ではなく、**ADR-123 決定4-3 自身が recovery resend にも発火する
実装漏れによる回帰**である（詳細は下記「原因」節）。

**症状（journal の `KeyInput.timestamp_us` + app_log 突き合わせで確定）:**
物理打鍵は「な・ま・え・ま」の4モーラ全て記録されており脱落していない
（`decision: Consume, effect_count: 3` が4回とも記録）。しかし実際に GJI
composition へ着弾したのは「な」（3回の再送を経て確定）と最後の「ま」
（末尾、warm 経路でクリーンに送信）の2モーラのみで、2モーラ目の「ま」と
3モーラ目の「え」が完全に消失した。**加えて出力順にも反転が起きていた
可能性が高い**（下記タイムライン手順6・原因節参照）。

**タイムライン（`timestamp_us`/app_log 時刻基準、訂正版。全て msedge.exe
フォーカス中）:**

1. `00:09:00.883` 「な」送信（cold）→ probe cold_seq=18 で `SuspectedLiteral`
   → `01:266` backspace×1 + "na" 再送予約、cold mark。
2. `01:294`-`01:297` "na" 再送、probe cold_seq=19 開始。
3. `01:312`/`01:315` 「ま」「え」送信 → probe/recovery in-flight のため
   `pending_deferred` へ退避（累計3VK: M,A,E、flush されず）。
4. `01:668` cold_seq=19 が2回連続 `SuspectedLiteral` で give-up
   （backspace×1、再送なし）→ `01:676` GJI reinit（`VK_IME_OFF`→`VK_IME_ON`）
   送信・IMC ポーリング開始。
5. `01:751` ポーリングで Hiragana 確認 → `Confirmed`。
6. **`01:754` `[chrome-reinit-retry] retry romaji via normal path: "na"`**
   ログ直後に `[tsf-probe] deferred 3 VK(s) を romaji 直後に送出` が出るが、
   これは `send_romaji_batched_gated` 内で
   `drain_pending_deferred_before_send_if_queue_only()`
   （`output/vk_send.rs:216`）→ 実送信（`assess_warmth()` 以降）という
   呼び出し順のため、**drain は "na" の実送信より前に発火している**
   （ログの見た目上「romaji 直後」に見えるのは `key_injector.rs:307` の
   ログ文言が drain 経路と本来の「romaji 直後」経路の両方で同一のため）。
   journal `seq 2450 GjiReinitRetryCompleted ... deferred_flushed: 0` が
   裏付ける——**BUG-38 の同期点（`flush_stale_deferred_vks_after_recovery`）
   はこの時点で何も flush していない。** 実際に「ま」「え」を流したのは
   ADR-123 decision4-3 の drain である。
7. `01:797` "na" の実送信（`warm=false`、`idle_at_cold=343ms`）→
   probe cold_seq=20 開始。
8. `01:789`/`01:820`/`01:851` 手順3・6 で先に注入済みの「ま」「え」の
   GJI I/O が3回連続で観測される。
9. `01:866`-`01:877` "na" の1文字目 'n' が送信され、直後の per-VK confirm
   が「候補ウィンドウが既に見えている」ことを根拠に literal-detect 待ちを
   スキップして confirmed 扱いにする（起点 `01:855`）。しかしこの起点は
   'n' 送信より**前**の write（手順8、直近は `01:851`）に由来し、**'n' の
   処理結果ではあり得ない**——手順6の drain が "na" 自身の per-VK confirm
   の証拠を汚染した。
10. `01:939` "na" の2文字目 'a' で epoch fencing が「直近の GJI I/O が
    送信時刻に追いついていない」矛盾を検出し `StaleConfirm` と判定 →
    `per_vk_recovery_params(is_stale=true, failed_idx=1)` が
    `escape_composition=true` を返す（`literal_detect_fsm.rs:89-92`。
    **escape は `failed_idx > 0` のみで決まり `is_stale` 自体は無関係**）。
11. `01:947` `flush_raw_tsf_literal_backspaces` が `VK_ESCAPE` を送信し、
    現在の composition（手順3・6で注入済みの「まえ」を含む）を丸ごと破棄。
    直後 `01:952` に "na" のみを再送し確定（`gji-fsm` は `01:944` に既に
    `OnWarm` 復帰済み——「warm になるまで待つ」対策ではこの窓を防げない）。
12. `03:167` 4モーラ目「ま」送信。GJI は `OnWarm`・probe 非稼働のため
    defer されず、warm 経路でそのまま送信・確定。これが出力に現れた「ま」。

**因果の要点（訂正、round2 で保護理由を訂正済み）:** drain（手順6）が
無ければ手順8〜11は起きない。ただし drain が無い場合に何が起きるかは
"na" 再送（`platform.rs:967`）が cold/warm どちらの経路を通るかで分岐する
——**保護しているのは `raw_recovery_owns_deferred()`/INV-F ではない**
（この時点で既に false）。本件のように cold 経路なら `install_pending_tsf`
が同期的に `has_pending_tsf()` を true にするため、`platform.rs:971` の
無条件 flush（`flush_deferred_vks_after_gji_reinit_completion`）はこの時点
では何もせず、"na" 自身の probe 完了後に正しい順序で
`flush_raw_tsf_literal_recovery`（BUG-38 が確立した順序保証）が flush する
——「な」+「まえ」+「ま」=「なまえま」と正しく出力されていた可能性が高い
（反実仮想、コードパスを実際に辿って確認済み）。"na" 再送が warm 経路を
通る場合（本件では起きなかったが、手順11の最終再送は実際にこの経路）は
`platform.rs:971` が即座に flush するため順序・証拠汚染はどちらも解消する
が、probe を経由しない raw VK 注入の残存リスク自体は残る。詳細は
[ADR-128](adr/128-escape-composition-collateral-deferred-loss.md)
「因果の要点」節参照。

**副次的な実害（未言及だった別欠陥）:** drain は「まえ」を「な」より
**先に**出力している。「な」自身は `01:671` に一旦 backspace で消えている
ため、ESC が無ければ画面には「まえな」という**順序反転**が見えていたはず
である。`journal_policy::order_violation`（`output/mod.rs:1918-1934`）は
バッチ**内**の `order_token` 単調性しか見ないため、この「drain されたキュー
と recovery resend との相対順序」の反転は構造的に検出できない。

**除外した対抗仮説:** 「`01:797` の F2 warmup が composition を reset した」
は成立しない。予防的 F2 送信は 2026-07-18 に撤去済み（`vk_send.rs:255-265`）
で、実ログも `F2/probe待機省略 → per-VK confirm へ` と記録している。
`prepend_f2_warmup=true`/`forces_f2=true` というログ・変数名は実際の F2
送信を意味しない（将来の調査者が同じ誤読をしないよう要注意）。

**原因（コード上で確定）:** `output/vk_send.rs::
drain_pending_deferred_before_send_if_queue_only`（`:76-86`）は
`is_probe_or_recovery_blocking(true)`（`has_pending_tsf() ||
raw_recovery_owns_deferred()`）と `pending_deferred_len()==0` だけを見て
flush の可否を決めており、**呼び出し元の `gate`（`DeferGate::Enforced`/
`Exempt`）を一切見ない**。`defer_respecting_gate`（同ファイル、ADR-123
決定4-2）は `gate` に応じて `raw_recovery_owns_deferred()` の扱いを
切り替えているのに対し、drain 側（決定4-3、`send_romaji_batched_gated`
`:216` / `send_romaji_as_tsf_gated` `:392` から無条件に呼ばれる）だけが
この非対称を欠く。結果、`resend_gji_reinit_retry_romaji`（give-up 後の GJI
reinit retry が `Confirmed` した際、保留していた元モーラを通常経路へ1回
だけ戻す、ADR-101 決定3、`gate=Exempt` で呼ばれる）**自身の送信**も
「新規モーラの送信」と誤認され、まだ確定していない元モーラの再送より前に
`pending_deferred` を drain してしまう。これが手順6〜11 の直接原因である。
composition の実内容は Chrome では読めない（`himc_null=true`）ため、
「ESC が『まえ』を消した」自体は状態遷移からの逆算であり、composition
バッファの直接観測ではない（BUG-75「推論（一次証拠なし）」節と同じ構造的
制約）。

**検討した対策（`docs/adr/128-escape-composition-collateral-deferred-loss.md`
に詳細、opus-adversarial-consult round1・round2 で検証済み）:**

- **採用: drain を `DeferGate::Enforced`（通常のユーザー入力）限定にする。**
  `drain_pending_deferred_before_send_if_queue_only` に `gate: DeferGate`
  引数を追加し（`ms_ime_gate_defer` が既に使う「呼び出し元から `gate` を
  そのまま引き継ぐ」パターンに揃える）、`gate != Enforced` なら早期
  return する。関数シグネチャに `gate` を持たせるのは、既存の2ユニット
  テスト（`vk_send.rs:797`/`:812`）が引数なしで直接呼んでおり、呼び出し側
  で分岐する形では新規テストが `SendInput` に到達する経路経由でしか
  書けなくなるため（round2 指摘）。キューを解放しうる経路は他に5つ残る
  ため飢餓しない（新規モーラの drain-before-send・`finish_probe_stage`・
  `flush_raw_tsf_literal_recovery` 末尾・`flush_deferred_vks_after_
  gji_reinit_completion`（`platform.rs:971`/`:977`、reinit retry 完了直後
  の無条件 flush）・`discard_pending_deferred_after_stale_gji_reinit`
  （`platform.rs:985`、破棄経路）——round1 の列挙は前2者を欠いていた）。
- **却下: `escape_composition` の代わりに backspace で元モーラ分だけ消す
  案。** (1) `literal_detect_fsm.rs:63-84` の doc が明記する不可侵
  invariant（BUG-33 追補3・4 の実害2件）と正面衝突する、(2) VK 数とかな数
  が一致せず消すべき文字数を原理的に知る手段が無い（Chrome では
  composition も読めない）、(3) 消す向きが入力順序（バグった drain 順序）
  に依存し破綻する。
- **保留（再定式化が必要）: composition 確定を待ってから flush する案。**
  本件では候補シグナル2つ（`GjiFsm::OnWarm` 復帰・per-VK
  `CompositionConfirmed`）がどちらも ESC 発火前に成立済みだったか、
  汚染された false-confirm 自体だったため、単純な「確定待ち」では防げない。
  採用案とほぼ同じ結論（recovery ラウンド間の隙間を埋める）に収束する。
- **格下げ: まず計測してから決める案 → 修正の代替ではなく独立の計装。**
  ADR-100 決定4-a への引用は不正確だった（対象が違う）うえ、同 ADR 本文が
  「計測期間中もユーザーは文字を失い続ける」ことを失敗モードとして名指し
  している。ただし drain-before-send の flush が現状 journal に一切
  記録されていない点は独立の欠落であり、`DeferredRecoveryFlush` に
  `trigger: "drain_before_send"` を追加する計装は採用案と別に価値がある。

**未実施:** 回帰テスト・修正いずれも未着手。ADR-128 の decision（drain を
`gate: DeferGate` 引数付きにして `Enforced` 限定にする）で実装に着手する。
回帰テストは `output/vk_send.rs` の既存テスト群（`:770-`）に引数を足した
上で「`Exempt` では drain がキューを保持する」旨を1本追加する
（`#[cfg(windows)]` 配下のため Linux では `cargo check --target
x86_64-pc-windows-msvc -p awase-windows --tests --lib` で確認し、実行は
`windows-build` CI に委ねる、`fix-requires-evidence.md` (a) を満たす）。

**関連ファイル:** `crates/awase-windows/src/output/vk_send.rs`
（`drain_pending_deferred_before_send_if_queue_only`（`:76-86`、本件の
flush 発火点、`gate` 引数追加が必要）、`defer_respecting_gate`（`:44-53`、
gate 考慮の非対称の比較対象）、`ms_ime_gate_defer`（`:499-505`、`gate` を
呼び出し元から引き継ぐ既存パターンの前例）、`send_romaji_batched_gated`/
`send_romaji_as_tsf_gated`（`:216`/`:392`、無条件呼び出し箇所））、
`crates/awase-windows/src/platform.rs`（`complete_gji_reinit_retry`
（`:961-975`、resend 直後の第2の無条件 flush 地点
`flush_deferred_vks_after_gji_reinit_completion`（`:971`/`:977`）と
`discard_pending_deferred_after_stale_gji_reinit`（`:985`）を含む））、
`crates/awase-windows/src/tsf/output.rs`（`flush_raw_tsf_literal_backspaces`、
`VK_ESCAPE` 全消去）、`crates/awase-windows/src/output/key_injector.rs`
（`send_deferred_vks`、「romaji 直後に送出」ログが drain 経路と共有されて
いる箇所）、`crates/awase-windows/src/output/mod.rs`
（`raw_recovery_owns_deferred`（`:1546-1555`、recovery ラウンド間で false
になる述語）、`finish_probe_stage`（`:1566-1577`、INV-F は本件でも正しく
発火した）、`take_pending_deferred_if_probe_idle`
（`tsf_warmup_coord.rs:414-420`、`has_pending_tsf()` のみを見る述語——
本件の実質的な保護根拠）、`crates/awase-windows/src/tsf/warmup/
probe_fsm.rs`・`literal_detect_fsm.rs`（`per_vk_recovery_params`、
`escape = failed_idx > 0`）。

**未解決の残課題（本 BUG のスコープ外、ADR-128 参照）:**

1. `WarmupAborted` 経路では `cancel_probe` を通らないため、GjiFsm の
   shadow pending は破棄されても `pending_deferred` は温存される非対称が
   本件ログでも観測された（`01:670` `DiscardPending count=3
   reason=WarmupAborted`、`cancel_probe`（`output/mod.rs:1636-1647`）の
   doc が予告した形）。**採用案（drain を `Enforced` 限定にする）は
   `pending_deferred` の滞留時間を延ばす方向に働くため、この乖離が露出
   する窓はむしろ広がる**（round2 指摘）。実機ソーク時の観測ポイントに
   含めること。
2. per-VK confirm が「候補ウィンドウが既に見えている」ことをそのまま
   confirm の証拠に使い、候補可視 veto（`probe_io.rs:659-670`
   `veto_eligible`）がこの経路に配線されていない構造的弱点は、本 BUG の
   採用案では解消しない（`Enforced` drain が流した本物の新規モーラや
   他アプリ由来の SHOW でも同型の false-confirm は起こりうる）。
   veto 配線は ADR-122 決定案のスコープ（未実装）。

**関連:** [ADR-128](adr/128-escape-composition-collateral-deferred-loss.md)
（本 BUG の decision・タイムライン一次証拠の詳細）、
[ADR-123](adr/123-focus-resync-and-probe-defer-queue-composition-race.md)
（決定4-3・drain-before-send を導入した当該 ADR。本 BUG はその実装漏れ）、
BUG-38（give-up 後の `pending_deferred` flush 漏れ、その同期点は本件で
一度も flush していない点で無関係）、BUG-75（`StaleConfirm` 回収の
`escape_composition` 判定そのものの由来）、BUG-33 追補3・4（`VK_BACK` を
避け `VK_ESCAPE` を使う設計の由来、backspace 案却下の根拠）。

## BUG-110: 物理IMEキー1回の低確度な検出で、NICOLA変換エンジンがフォーカス変更まで無期限停止する

**症状（不具合報告 `01M1MMK8987NT5B2W73PCPZNZ1`、2026-09-03）:** Windows
Terminal + PowerShell（GJI、JIS配列）で入力中に余分な「＠」が出力される。

**原因（journal/app_log突合で確定）:** 物理CapsLock位置（scan=0x3A、JIS
配列では「英数」キーでもある）から届いた`vk=0xF0`（`VK_DBE_ALPHANUMERIC`）
の`KeyDown`1回を`kp_stage_shadow_ime_toggle`
（`crates/awase-windows/src/runtime/key_pipeline.rs`）が「物理IMEキーに
よる意図的なIME OFF」と解釈し、`ImeModel::last_intent`
（`crates/awase-windows/src/state/ime_model.rs`）へ即座に確定。しかし
実際のGJI変換状態（`observed`）は`open=true`のまま変化せず、drift
correctionが`VK_IME_OFF`を14回超反復しても収束しなかった。
`last_intent`は`reduce()`の`FocusChanged`アーム1箇所でしかクリアされず
TTLも持たないため、**ロックアウトの長さは無期限**（フォーカスが変わる
まで）——今回は不具合報告ボタンを押した瞬間の`FocusChanged`まで約29秒。
その間NICOLA変換エンジンが停止し続け、物理キーがJIS配列本来の文字
（＠）のまま素通りしたのが直接の症状。詳細な因果分析・検討した根本
修正案（4案、いずれも既存の防御(BUG-19/26/68/52・ADR-093/119/121)との
衝突でOpus 2体4ラウンドの敵対的レビューにより不採用）は
`docs/adr/132-uncorroborated-physical-ime-key-engine-lockout.md`参照。

**対応（Phase 1のみ実装、根本修正は未着手）:** ロックアウト自体を解消・
短縮する修正はいずれもBUG-19型の再発やdrift correctionとの二重
actuation等の新たなblockerを踏むため、Phase 1として以下の2つのみを
実装した（`desired_open`・`observed`・配送判断・actuationのいずれにも
触れない）:

- drift correctionのBlind GiveUp初到達時、1フォーカスにつき1回、
  トレイバルーンで「このアプリではIME状態を確認できません」と中立に
  通知する（`runtime/ime_refresh.rs`）。
- 次の設計判断に必要な7項目（観測源・信頼度、送信VK、hookの
  `[hook] IME-mode`診断、intentの種別、区間長・終了理由、使用中の
  `.yab`、半角英数トグルの状態）をjournalへ構造化記録する
  （`DriftGiveUpDiagnostic`/`HookImeModeDiagnostic`/
  `DriftGiveUpIntervalEnded`、`crates/awase-windows/src/journal.rs`）。

Phase 1のログが揃った時点で、根本原因がconv残骸（BUG-68型、実IMEは
正しく閉じていた）かどうかを判定し、Phase 2（保留中の修正案の再検討）
か、BUG-68の未解決部分への転進かを判断する。

**追補1（2026-09-04、2件目の実機再現・Phase 1診断の実地検証）:**
ユーザーが未マージのPhase 1ブランチを自前ビルドし検証したところ、
不具合報告 `01M1N5RMQ0HGX60VNG37DXWR26` として同症状（PowerShell +
GJI + JIS配列で「＠」大量出力）を再現した（`docs/bug-reports-triage.md`
該当行に詳細）。journalに`HookImeModeDiagnostic`が180件記録されており、
**Phase 1の診断ログが実機で正しく機能することを確認できた。**

新たに判明した事実:
- 発端のVK×scanは`vk=0xF3 scan=0x29`（半角、`VK_DBE_SBCSCHAR`）——
  1件目の`vk=0xF0 scan=0x3A`（英数/CapsLock位置）とは異なる組み合わせ。
  さらに`vk=0xF0/0xF2 scan=0x70`（かなキー位置）のペアも複数回観測され、
  これはBUG-52（`transport.rs`のコメント、「IMEが既に目的の状態にある
  時に押されると発生しうる」）が既に記録している既知の不安定源と一致した。
  **「入口となるVK×scanは1つに固定されない」というADR-132 v3の再定義
  （候補5=単発DBEキー恒久除外を不採用にした理由）を実データで裏付けた。**
- `[drift] correction`ログに`for 341743ms`（約5分42秒）という、1件目の
  29秒を大幅に上回る乖離継続時間が記録された。`FocusChange`（不具合報告
  ダイアログへの遷移）でようやく`Engine activated`に戻っており、
  「`last_intent`はFocusChangedでしか解除されず無期限に持続する」という
  ADR-132 v4の因果モデルを追加のケースで裏付けた。
- 一方、`DriftGiveUpDiagnostic`/`DriftGiveUpIntervalEnded`はこのログには
  1件も記録されていなかった。原因を特定: journalの`ImeActuation`エントリ
  を確認したところ、この乖離は`FeedbackPolicy::Blind`ではなく
  **`FeedbackPolicy::Read`**（`ImmGetOpenStatus`、400msデッドライン）で
  処理されており、同一の`Actuation`が`attempts=162`から`243`まで
  330秒間`Send`を返し続けていた。`state/ime_actuation.rs::
  decide_actuation_action`は`Read`の場合`attempts`に関わらず常に`Send`
  を返す設計（`Blind`専用の`max_attempts`打ち切り＝`GiveUp`という概念が
  `Read`には無い）ため、これは想定内の挙動であり、**Phase 1のトレイ
  通知・診断ログが`GiveUp`到達だけをトリガーにしていたことの設計上の
  穴**だったと判明した（`Blind`専用のTsfNativeプロファイルのはずが
  なぜ`Read`が使われていたかは未解明、別途調査の価値がある論点として
  残る）。

  **追補2（2026-09-04、修正済み）:** トレイ通知と`DriftGiveUpDiagnostic`
  記録のトリガーを、`FeedbackPolicy::Blind`の`GiveUp`到達ではなく、
  乖離継続時間（`duration_ms >= DRIFT_CORRECTION_BLIND_REARM_COOLDOWN_MS`
  、既存定数を再利用・新規タイミング定数の追加なし）ベースの、
  `Blind`/`Read`共通の発火点に変更した（`runtime/ime_refresh.rs`の
  `match act_policy`より前）。`desired_open`・`observed`・
  `shadow_effect()`・`transport.rs::plan`・`IntentStore`・`last_intent`
  のいずれも変更していない。`architecture_guard.rs`/
  `layer_boundary_guard.rs`/`cargo test --lib`（coreクレート972件・
  awase-windows561件）は全てpass。

**関連ファイル:** `crates/awase-windows/src/runtime/ime_refresh.rs`、
`crates/awase-windows/src/runtime/key_pipeline.rs`、`crates/awase-windows/
src/state/ime_model.rs`、`crates/awase-windows/src/state/intent_store.rs`、
`crates/awase-windows/src/hook.rs`、`crates/awase-windows/src/journal.rs`。

**関連:** BUG-15追補7（同じvk×scanの組み合わせの既知の不安定性）、
BUG-19（conv推論での`desired_open`書き換えがエンジン誤復帰を招いた
先例）、BUG-68（IMM32 NATIVEビットが`VK_IME_OFF`で閉じても消えない）、
ADR-121（no-op側の欠落、対をなす）、ADR-132（本件の設計ADR）。

**追補3（2026-09-04、3件目の実機再現・Phase 1修正後の初発火確認）:**
不具合報告`01M1NA7WYH1HCYAFWGA3F95AVY`（`01M1N9HZA87MQSWYYKHGK7QXNA`と
同一プロセスの2回目のdump、`docs/bug-reports-triage.md`該当行に詳細）。

journalに`DriftGiveUpDiagnostic`が1件記録され（`drift_duration_ms:
24834`）、**追補2の修正（duration基準のBlind/Read共通トリガー）が実機で
正しく動作することを確認できた。** 直前には`DriftGiveUpIntervalEnded`
（`reason: FocusChanged`、`elapsed_ms: 365422`）もあり、**約6分5秒**と
いう、これまでで最長の乖離継続時間が記録された。

**訂正（2回訂正が必要だった）:** 当初「ユーザーとawaseが競合する
フィードバックループ」と記述し、続く再検証で`vk=22`を`VK_KANJI`と
誤認して「KanjiToggleフォールバックが92回」と記述したが、**`vk=22`は
`VK_KANJI`(0x19)ではなく`VK_IME_ON`(0x16)だった**（`vk.rs`で実値を
確認）。KanjiToggleStrategyは無関係で、正しくは以下のとおり
**drift correctionとは別の、もう1つの独立した自動修正機構が競合して
いた**ことが根本原因だった。

`HookImeModeDiagnostic`全件を`(vk, injected, self_injected)`で集計
（推測ではなくWin32の`LLKHF_INJECTED`フラグ＋awase自身のマーカーに
よる確定判定）:

| vk | 内訳 | 件数 |
|---|---|---|
| `VK_IME_ON`(22) | 全件`injected=true/self_injected=true` | **92件** |
| `VK_IME_OFF`(26) | 全件`injected=true/self_injected=true` | **32件** |
| 半角(243)/全角(244)/かな(242)/英数(240) | 全件`injected=false/self_injected=false` | 合計30件 |

journalの`ImeActuation`（drift correctionが記録する唯一のactuation
journal）は本レポートの全31件が`target: false`のみで、`VK_IME_ON`側の
送信は1件も記録されていない。すなわち**92件の`VK_IME_ON`送信は
drift correctionが行ったものではない。**

app_logを調べたところ、`[apply-ime] GJI direct: send 0x0016 (open=true)`
に続けて`force-ON (ImmBrokenForceOn): apply_ime_open(true) → Applied`
というログが見つかった。これは`runtime/mod.rs::apply_force_on_for_imm_broken`
（`is_eligible_for_ime_force_on()` = `is_japanese_ime() &&
effective_open()`が真の間、周期リフレッシュに相乗りして`VK_IME_ON`を
再送し続ける、TsfNative/Imm32Unavailable向けの「nonaiyo問題対策」機構、
ADR-086 INV-15が「生の周期タイマートリガー」として例外的に許容している
経路）由来で、**drift correctionの`Actuation`状態管理を一切通らない、
完全に別の書き込み経路**である。

**根本原因（確定）:** `ir_check_drift_correction`が読む`desired`は
`ImeModel::desired_open()`（生のbeliefフィールド）だが、
`apply_force_on_for_imm_broken`が読む条件は`effective_open()`
（`IntentStore`＋`derive_any()`を経た**導出値**）——**この2つは
同じ「IMEを開きたいかどうか」を表しているはずなのに、別の関数・
別の解決経路で独立に計算されており、乖離しうる。** 具体的には、
`FocusChanged`が`last_intent`をクリアした直後は`effective_open()`が
`derive_any()`（観測ベース）にフォールバックして`true`を返しうる一方、
`desired_open()`（生フィールド）は次の明示書き込みまで`false`のまま
残る。この窓で、**drift correction（`desired_open()=false`を根拠に
`VK_IME_OFF`を送る）と`apply_force_on_for_imm_broken`
（`effective_open()=true`を根拠に`VK_IME_ON`を送る）が、互いの存在に
気づかないまま同じ実IMEへ矛盾した書き込みを競合して送り続ける**。
これが92対32という大量かつ拮抗しない送信数、および6分を超える
長時間化の実態と考えられる。ユーザーが観測した「＠kaうきka…」という
混沌とした出力は、この2機構の競合でIME状態が実際に不安定に
振動していたことの症状として整合する。

これは`.claude/rules/fix-requires-evidence.md`が言う「IME actuation
合流点」（新しいgate/preconditionを足す際に洗い出すべき箇所、ADR-119）
の中でも、**合流点そのものが存在しない**——2つの独立した書き込み経路が
共通の調停なしに同じ対象へ書き込める、という、issue #136/ADR-119よりも
一段深い構造的欠陥。ADR-132の4ラウンド討論（A/B/B'/候補1-v2）は
drift correction 1経路のみを前提にしており、この`ImmBrokenForceOn`との
競合は検討されていなかった。Phase 2再検討時の最重要論点として記録する。

`journal`は`DumpTruncated`（total_entries=2123、emitted=1004、
dropped_key_input=418）のため、「＠」を直接出力した打鍵そのものは
特定できなかった。

**追補4（2026-09-04、Opus 2体による「A-2適用」検討・不採用、SSOTは
2つでなく3つ以上と判明）:** 追補3の根本原因を踏まえ、既存の
`OpenWarrant`授権機構（ADR-087設計、`state/open_warrant.rs`）を
`apply_force_on_for_imm_broken`（起案点#6）にだけ強制適用する
（`ADR-090`が定義する「A-2」フェーズの部分適用）という修正案を、
Opus 2体（architect/premortem）で3ラウンド討論した。

**結論: 不採用（実装に進むべきでない）。** 主な理由:

1. **`issue_open_warrant()`は「同じSSOTを読ませる」単純な仕組みでは
   ない、5段の優先順位付き梯子**（Step 0 SafetyValve → Step 1
   IntentStore → Step 3 Actuating観測 → Step 4a HeuristicDefault →
   Step 4b heuristic guard → Step 4c `desired_open`採用）であり、
   `desired_open`と一致するのは梯子が**最下段(Step 4c)まで落ちた
   場合のみ**。特にStep 4bは「#6を発火させた同じguardが、その
   guard自身によって`true`で即座に授権される」という**自己整合
   ループ**になっており、guard起因のforce-ONではA-2は完全にno-opに
   なる。Step 3（`ObservationSource::Gji`、Actuating権威）もGJI
   環境では常態的に発火しうるため、「TsfNativeだからActuating観測は
   来ない」という前提は誤りだった（後述の訂正参照）。
2. **SSOTは2つではなく、少なくとも3つ存在する。** `runtime/
   ime_refresh.rs`のフォーカス変更経路が呼ぶ`send_eager_tsf_warmup`
   は`warmup_ime_on()`（`platform_state.rs`、`applied ?? belief`）
   という**第三の解決関数**を読み、**`issue_actuation_order`を
   一切通らないため`OpenWarrant`の対象外**（A-2では原理的に触れない）。
   実際、report4のapp_log（末尾切り詰め済みの狭い窓）だけでも
   `[tsf-eager-warmup] VK_IME_ON 送信`ログが確認でき、同じ窓で
   `force-ON (ImmBrokenForceOn)`は2件、`send_eager_tsf_warmup`
   呼び出しは5件——**92件の`VK_IME_ON`の相当部分が#6(force-on)では
   なくwarmup経路由来である可能性が高い**ことが実データで裏付けられた。
   `#8 focus_change_tsf_native_gji_force_on`という当初の代替候補は
   ADR-098決定2で既に撤去済み（`ime_refresh.rs:493-497`）であり
   存在しない。
3. A-2でwarrantが`None`となり送信自体を中止した場合、
   `note_force_on_attempt`によるクールダウン起点が刻まれず、
   `runtime/mod.rs:836-838`が警告する「実効50Hzの無限再試行ループ」
   （BUG-69/BUG-31族）を再導入するリスクが新たに見つかった
   （拒否時の再スケジュール設計が未決定のまま実装すると危険）。
4. A-2はStep 1（`IntentStore`に古いOFF意図がTTL内で残存）でも
   `None`を返し、BUG-16系の正当なforce-ON再試行まで止めうる。

**維持された知見**（実装は見送るが記録価値がある）:
- drift correctionが`desired_open()`、force-onが`effective_open()`を
  読むという二重SSOTの発見自体は正しい。
- `FocusChanged`が`last_intent=None`にし`effective_open()`が
  `derive_any()`にフォールバックして`desired_open()`との一致が
  構造的に切れる窓の特定（`ime_model.rs:621`・`:398-405`）も正しい。
- `ObservationSource::Gji`/`Tsf`はActuating権威（`ime_event.rs:
  184-188`）で、プロファイルがTsfNativeでもIME種別由来の観測として
  発火しうる、という訂正は正しい。

**次のアクション（実装せず、まず既存データで答えられる範囲）:**
1. report4のapp_log全体（末尾切り詰め前の生ログが必要なら再取得）で
   `force-ON (ImmBrokenForceOn)`行数と`[tsf-eager-warmup] VK_IME_ON
   送信`行数を数え、92件のうちの内訳を確定する。
2. 根本原因節を「2つのSSOT」から「少なくとも3つのSSOT
   （`desired_open()`/`effective_open()`/`warmup_ime_on()`）＋
   warrantを通らない送信経路が存在する」へ訂正する（本追補が
   その訂正を兼ねる）。
3. 真に必要なのは「起案点ごとのA-2適用」ではなく、**warrantを
   通らない送信経路（warmup等）をwarrant配下に含める、ADR-090
   §2.Aの11箇所棚卸しのやり直し**である可能性が高い——ただしこれは
   本ADR/BUG-110の対応範囲を超える、より大きな仕事。

**追補5（2026-09-04、棚卸しのやり直し完了）:** 上記アクション3を実施。
ADR-090 §2.Aの「11起案点」は`issue_actuation_order`を呼ぶ箇所の数え
上げとしては正確だったが、**「実際にWin32へIME関連の書き込みを行う
全経路」で見るとその部分集合でしかなかった**。`issue_actuation_order`
を経由しない末端の書き込み関数を起点にボトムアップで洗い出した結果、
warrant非経由の経路が新たに**4系統**見つかった。

| # | 経路 | 呼び出し元 | トリガー条件 | 送信VK/API | 読んでいるSSOT | 危険度 |
|---|---|---|---|---|---|---|
| B1 | `output::send_eager_tsf_warmup` | `platform.rs`5箇所＋`output/vk_send.rs:673` | フォーカス変更時のTSF warmup・composition reset後の保険再送 | `VK_IME_ON`/`VK_IME_OFF` | **`warmup_ime_on()` = `applied ?? belief`（第3のSSOT）** | **最高**（本BUGで実際に競合を起こした経路そのもの） |
| B2 | `platform::send_engine_state_ime_key` | `executor.rs:763`（engineのactivation状態遷移＝`engine.rs::transition_activation`が発行） | NICOLA engineのON/OFF活性化遷移そのもの（drift/force-onの「IMEをどう思うか」とは独立した第三の軸） | config由来`engine_on_ime_vk`/`engine_off_ime_vk` | **`enabled`+`applied_for_engine_key`（desired_open/effective_open/warmup_ime_onのいずれでもない第4の軸）** | 中（`last_applied==enabled`ガード・`uses_kanji_toggle()`ガードで部分的に自己防御しているが、「現在進行中の他のactuation」は見ていない） |
| B3 | `panic_reset`内の`set_ime_open_cross_process_async`直接呼び出し | `runtime/mod.rs:1678-1681` | パニックリセット発動時のみ | `ImmSetOpenStatus`直列書き込み | 何も読まない（無条件リセット） | 低（意図的な例外、緊急リセットという性質上warrant迂回は設計として妥当） |
| B4 | `output::send_gji_half_width_alnum_toggle` | `key_pipeline.rs:1573`/`:1643`（左Shift単独タップ、BUG-25） | BUG-25半角英数持続トグル | `VK_DBE_ALPHANUMERIC`/`VK_DBE_HIRAGANA` | `effective_open()`を単純ゲート（自らbeliefは書き換えない） | 低〜中（タイミング依存の見落としはありうるが自らbeliefを書き換えないため実害は限定的） |

一方、`issue_actuation_order`を経由する側（旧`force_on_and_correct_romaji`
＝起案点#6を含む）は、warrant経由と確定した——11箇所という数字自体に
誤りはなかった。**すなわち実際にWin32へ書き込む経路は「(A) warrant経由
11箇所 + (B) warrant非経由4系統 = 実質15系統」で、ADR-090はそのうち
11しかwarrant管理下に置いていなかった。**

新たに判明した競合ペア（BUG-110の`VK_IME_ON`/`VK_IME_OFF`混在は
この1つ目の実例）:
- **B1(warmup) × A(drift correction)**: 実際に確認済みの競合そのもの
- **B2(engine_state_ime_key) × A系全般**: 未検証だが同型のリスクを
  理論上残す（engineの活性化遷移という独立トリガーが、drift/force-on
  と短時間に交差する可能性）
- **B4(半角英数トグル) × A系**: `effective_open()`反転中のタイミング
  依存の見落とし、ただし実害は限定的

**次のアクション（更新）:** B1が最優先（今回の実害の直接原因）。
B1を`issue_open_warrant`配下に含めるか、`desired_open()`/
`effective_open()`のどちらかへ統一するかを、Phase 2として別途
設計する必要がある。B2はB1着手後に同型のリスクとして再検討。
B3は現状維持でよい。B4は優先度低。

**追補6（2026-09-04、Phase 2実装完了）:** B1の修正をOpus 2体
（architect/premortem）敵対的討論2ラウンドで設計・実装した。詳細な
設計経緯は[ADR-132](adr/132-uncorroborated-physical-ime-key-engine-lockout.md)
「Phase 2（B1の修正、v5で追記）」節を参照。要約:

- **v1（`desired_open()`とのANDゲート）は却下**: `desired_open`が
  `ImeEvent::FocusChanged`のreduceアームでクリアされずフォーカスを
  跨いでstaleに残る性質（本BUGの原因そのもの）を、v1は修正の根拠として
  使ってしまい、cross-windowの正当なcold-startケースまで抑止して
  BUG-02型のリテラル化を再導入するblockerが判明した。
- **v2（採用）: `check_drift_correction()`が返すOFF方向drift検出中
  フラグとのゲート**（`WarmupImeOn::
  from_applied_or_belief_unless_off_drift`、INV-B1'）。
  `check_drift_correction`の判定材料はFocusChangedのたびに必ず
  クリアされるためfocus-scopedで、cross-windowのcold-start warmupを
  壊さない。B1とdrift correctionが同じ判定式を共有することで、
  「別々の関数が別々に解決する」というBUG-110の構造的欠陥をこの2者間
  では作らない。
- **IntentStoreベースの代替案は不採用**: 3件目報告の実測
  （drift継続24.8秒 vs 365秒、`IntentStore`OFF方向TTLは30秒）から、
  365秒区間の92%はTTL切れでゲートが機能しないと判明。
- **受け入れたトレードオフ**: 新フォーカス直後はゲートが開くが、
  stale な`desired_open`と新フォーカスの観測が食い違うと
  `DRIFT_CORRECTION_THRESHOLD_MS`経過後にゲートが再び閉じ、その窓での
  warmupは（ユーザーが明示的にIMEキーを押すまで）抑止され続ける。
  「開けながら閉じる」自己矛盾を「閉じたまま」という一貫した誤りへ
  変える設計判断であり、修正ではなくトレードオフとして受け入れる。
- **本Phase 2はB1×drift correctionの競合のみを解消する**。#6
  （`apply_force_on_for_imm_broken`、`effective_open()`を読む）×
  drift correctionの競合は無傷のまま残る。3件目報告の`VK_IME_ON`
  92件のうちB1由来の割合は`DumpTruncated`により未確定——`output/
  mod.rs`の`[tsf-eager-warmup] VK_IME_ON 送信`ログを`info!`へ
  格上げし（既にinfo!の`force-ON (ImmBrokenForceOn)`と併せてgrep2本
  で内訳を確定できるようにした）、次回実機報告で判断する。
- **実装・検証**: `src/platform.rs`（core、`WarmupImeOn::
  from_applied_or_belief_unless_off_drift`と12通り全数テストT1）、
  `state/platform_state.rs::resolve_warmup_ime_on`
  （`check_drift_correction`とのゲート配線、`now: Instant`を
  呼び出し元から注入するよう変更——`state/`層は壁時計を直接読まない
  規約に従う）、呼び出し元7箇所（`ime_refresh.rs`/`key_pipeline.rs`/
  `executor.rs`×5）、`architecture_guard.rs`の呼び出し件数固定
  テスト（core/awase-linux/awase-macosも走査対象に含める）、新規
  ungated統合テスト`tests/warmup_gate_focus_scope.rs`（T2/T3、
  `ObservationStore`のdrift追跡がFocusChangedでクリアされる性質と
  `desired_open`がクリアされない性質の両方を固定）。
  `cargo check --target x86_64-pc-windows-msvc -p awase -p
  awase-windows`/`clippy`/`fmt --check`/`cargo dylint --all`/
  `architecture_guard`/`layer_boundary_guard`/`golden_scenarios`/
  `intent_store_effective_open`/`warmup_gate_focus_scope`（計108件）/
  core `cargo test --lib`（977件）全てpass。実機ソークは未実施。

**実装直後の敵対的コードレビュー（Opus、読み取り専用の別エージェント）
で複数の指摘を受け、修正済み。** 詳細はADR-132「Phase 2」節
「実装後レビューでの指摘と対応」参照。要点:

- **INV-B1'は`WarmupImeOn`の全構築経路には及ばない**ことが判明
  （`platform.rs::on_ime_applied`の`from_actuated`経路——force-ONが
  `SetOpen(true)`を適用した直後にも通る——はゲートを経由しない）。
  ADR/docの過大な主張をスコープ限定に訂正。
- 上記の結果、D2のログ内訳（92件相当の「B1由来 vs #6由来」）が
  force-ON随伴分でB1側へ過大計上される欠陥が判明。
  `send_eager_tsf_warmup`に`origin`引数（`"gated"`/`"actuated"`/
  `"off"`）を追加し、`platform.rs`の合流点9箇所へ機械的にスレッディング
  して正確に分離できるよう修正。
- `[warmup-gate]`ログに重複排除が無く（打鍵駆動で頻発するため乖離が
  長時間続くと飽和）、かつ「ゲート無しでも元々送らない値」まで
  「抑止した」に計上する不正確さがあった。`intent_override_logged`と
  同じ`Cell<bool>`パターンで修正、副次的に`effective_open()`の
  二重評価（TTL境界を跨ぐとログ値とゲート判定値が食い違いうる）も解消。
  ガードテストも1件追加（`warmup_gate_third_arg_is_never_a_bare_literal_in_production_code`、
  ゲート版への第3引数リテラル直書き迂回を検出）。
- doc型名誤り（`PlatformState::explicit_intent()`→正しくは
  `ImeStateHub::explicit_intent()`）・重複ヘルパー実装
  （`list_src_files`）も修正。
- **対応せず既知の限界としてドキュメント化のみに留めたもの**:
  ゲート条件（`check_drift_correction`）とdrift correction本体の
  実際の発火条件（`is_user_enabled`/`is_japanese_ime`/settle待ち/
  `FeedbackPolicy::Blind`のGiveUp状態）がずれている点、`now: Instant`が
  `executor.rs`の5箇所で独立に取得されバッチ内一貫性が無い点。
  いずれも実害は限定的と判断し、設計の再検討が必要な範囲は次回の
  Opus討論に持ち越した。

全チェック再実行（`architecture_guard`単体66件、guard/golden/新規
テスト計109件）でall pass確認済み。

**同レビューエージェントへの再確認（round 2）で、#3の修正自体が
新たな欠陥を持ち込んでいたことが判明、追加修正済み。** 詳細は
ADR-132「Phase 2」節「round 2レビュー」参照。要点:

- **N1**: `[warmup-gate]`ログのdedup episode境界が
  `off_drift_active && would_have_sent_without_gate`のANDだった
  ため、OFF方向drift検出中に`applied`がdrift correction/force-ON
  間でping-pongするBUG-110本来のシナリオで「抑止開始/終了」が
  フラップし、ゲートが閉じたままなのに「抑止終了」が誤って出る
  バグがあった（#2で直したのと同種の「診断ログが診断を誤らせる」
  問題の再発）。episode境界を`off_drift_active`単独に戻して修正。
- **N2**: N1の判定（`applied_open.unwrap_or(effective)`）が
  `from_applied_or_belief`のロジック重複実装になっており、しかも
  正攻法（`from_applied_or_belief(...).is_on()`の呼び出し）は
  同コミットで追加したガードテストが禁止していた。core側に
  `WarmupImeOn::would_send`をSSOTとして切り出し両者から呼ぶ形に
  修正、等価性の回帰テストも追加。
- **N3・N4**: ガードテスト用パーサがコメント行を除外しておらず
  誤検知の余地、かつ閉じ括弧が見つからない場合に非UTF-8境界で
  panicする潜在バグがあった。修正済み。
- **N6**: `origin`引数が`&'static str`でタイポ耐性が無かったため
  `output::WarmupOrigin` enumに変更。

全チェック再々実行（`cargo test --lib`978件含む）でall pass確認済み。

**同レビューエージェントへの再々確認で収束を確認した（2026-09-04）。**
N1〜N7すべてが意図どおり修正されており、修正自体が新たな欠陥を
持ち込んでいないことも（パーサの実証ハーネスによる非退行確認込みで）
検証済み。新規のblocker・重要指摘なし。実機ソークへ進んで問題ない
という判定。

## BUG-111: `run_ime_refresh` の 500ms 周期リフレッシュが実フォーカス変更の有無に関わらず `[imm-learning] profile 降格` ログを毎ティック再発火させる

**症状（ユーザー報告「不具合報告レポートのテキスト入力フォームがおもすぎて実用に
耐えないほど不快」、2026-09-04）:** 不具合報告画面
（`awase-settings.exe --bug-report`）の説明欄に親指シフトで入力していると
「重い」と感じる。実機で `RUST_LOG=debug` を起動し同じ操作を再現したところ、
`awase-settings.exe` のウィンドウ（class `"Window Class"`）にフォーカスが
留まったまま約2.5分間で **`[imm-learning] profile 降格: Standard →
Imm32Unavailable` ログが42回連続で再発火**していることを確認した——実フォーカス
変更は1回も起きていない。

**原因:** `runtime/ime_refresh.rs::run_ime_refresh`（自己再スケジュール型の
IME 監視ループ、`ir_stage_focus` が毎ティック `detect_and_update_focus()` を
無条件に呼ぶ設計、`reschedule_ime_refresh` で概ね500ms周期）は、実フォーカス
変更の有無に関わらず `apply_focus_probe_result` → `advance_focus_tracking` →
`FocusTracker::update_with_process_name`（`focus/tracker.rs`）を毎ティック
呼ぶ。`CurrentFocus::update_with_process_name` が呼び出しのたびに
`app_profile` をいったん静的分類（`Standard`）にリセットしてから学習済み
降格を再適用する設計のため、旧コードは「リセット直後の静的値」対「学習済み
降格値」を比較していた。学習済みエントリが `Imm32Unavailable` のまま変わって
いなくても、この比較は**リセットのたびに必ず不一致**になり、既に降格済みで
何一つ新しい情報が無いティックでも `log::info!` が再発火し続けていた。

このログは `RUST_LOG` 未設定の通常運用でも出力される（`init_logging` の
`default_filter_or("info")`、INFO レベル）ため、通常運用でもディスクI/O
churn を継続的に発生させていた。

**自己修復との両立:** `update_with_process_name` の毎ティック呼び出し自体は
`Runtime::learn_imm_capability_from_miss` による store 更新（`Works` への
回復学習）を反映するために必要（`FocusTracker::apply_learned_imm_capability`
の doc コメント「次のフォーカス更新から降格は解除される」参照）であり、
呼び出し自体を間引く修正は自己修復を壊すリスクがある。そのため呼び出しは
残し、ログ発火条件だけを「直前 tick と同一ウィンドウ (`pid`+`class_name`)
かつ同一の実効値」のときは抑制するよう修正した（`should_log_demotion` 純粋
関数、`focus/tracker.rs`）。

**実機A/B検証結果（2026-09-04）:** 修正版ビルドで同じ操作を再現したところ、
降格ログは42回→**1回**に減少した（同一ウィンドウ在留中の再発火が解消）。
ユーザーからは「軽くなったと感じた」とのフィードバックを得た。

**残存する未解決課題（本 BUG のスコープ外）:** `classify_focus_probe`
（`runtime/focus_tracking.rs`）が呼ぶ `imm_learning::learn_imm_capability_on_focus`
（学習済みならスキップされる、軽量）とは別に、`kind_classifier::resolve_focus_kind`
（MSAA/UIA クロスプロセス呼び出しを伴う）は実フォーカス変更の有無に関わらず
毎ティック無条件に再実行され続けている（修正版ビルドの実機ログでも約500ms毎に
`MSAA: role=...` の再発火を確認済み、2.5分間で103回）。ログ抑制とは独立に、
この処理自体をフォーカス未変更時にスキップする最適化は未着手。シングルスレッド
メッセージループ上でこの COM 呼び出しがどの程度打鍵処理をブロックしうるかも
未計測。

**修正履歴:** `1df09c2c`（2026-09-04、`fix/focus`、`should_log_demotion` 追加・
回帰テスト3件）。

**関連:** BUG-107（同じ `[imm-learning] profile 降格` ログを扱うが原因は別軸
——本 BUG はログ発火条件のバグ、BUG-107 は学習キャッシュのキー設計のバグ）。

## BUG-112: `ImmCapabilityStore` が `awase-settings.exe` を稀に `Unavailable` と誤学習し恒久化する（BUG-107 の「あ混入」の残存原因、トリガー条件は調査中）

**症状（ユーザー報告、2026-09-04）:** 不具合報告画面の説明欄で親指シフト入力
すると、稀に先頭に意図しない「あ」が混入する（BUG-107 として既に報告・
process/class キー化で修正済みのはずだった）。

**BUG-107 の修正では説明できない再現を実機で確認:** BUG-107 の修正
（`ImmCapabilityStore` を `(process_name, class_name)` キーにする、ADR-125）
が適用済みのビルドで実機再現テストを行ったところ、**「あ」混入は依然として
再現した**。原因切り分けのため `target/debug/cache.toml` を確認したところ、
`[imm_capability."awase-settings.exe"] "Window Class" = "unavailable"` が
**プロセス間汚染ではなく、`awase-settings.exe` 自身の過去セッションでの
学習として正しくキーされたまま**残っていた——つまり BUG-107 の修正（キー
設計）自体は正しく機能しているが、**そもそもこの学習結果自体が誤っている**
という、ADR-125 が「未確認」のまま残していた設計課題3
（`Imm32Unavailable` 降格後の具体的な誤動作メカニズム、および降格の判定
自体が正しいかどうか）が、別の独立した原因として存在することが判明した。

**訂正（2026-09-04、当初記述の誤り）:** 本セクションは当初「cache.toml を
消去してのフレッシュ学習実験を2回行い、1回目は unavailable+あ混入、2回目は
works+あ混入なしを観測した」と記録していたが、これは誤り。実際に cache.toml
を消去してフレッシュに学習させる制御実験を行ったのは **1回だけ**であり、
その1回は `"Window Class" = "works"` と正しく学習され、「あ」は混入
**しなかった**（下記「再現実験」参照）。「`unavailable` になった回」は
フレッシュな誤学習の瞬間ではなく、**この調査セッションより前（前日までの
ADR-125 実機調査等）から `target/debug/cache.toml` に残っていた古いエントリ
の再利用**だった（そのビルドの `RUST_LOG=debug` ログに `ImmGetDefaultIMEWnd=NULL,
疑いを記録` という学習イベントのログが1行も無く、`[imm-learning] profile
降格` だけが即座に出ていたことから、新規学習ではなく既存キャッシュの適用と
判明した）。誤った記述のまま次のセッションに引き継がれることを防ぐため、
本節で訂正する。

**再現実験（cache.toml を消去してフレッシュに学習させる、2026-09-04、1回のみ）:**
`target/debug/cache.toml` を削除し `awase.exe`/`awase-settings.exe` を
再起動、説明欄に入力 → `"Window Class" = "works"` と正しく学習され、
「あ」は混入しなかった。学習イベントのログ（`ImmGetDefaultIMEWnd=NULL,
疑いを記録`）も一切出なかった——`ImmGetDefaultIMEWnd` は毎回 non-NULL
（有効）を返したことを意味する。

**「unavailable ↔ あ混入」の対応は別経路で確認済み:** 上記フレッシュ学習
実験の**直前**に行ったテスト（BUG-111 の Fix A 検証ビルド、cache.toml は
未クリアで前述の古い `unavailable` エントリを再利用）では「あ」が混入した
ことをユーザーが確認している。「古い誤学習が残っている状態では あ が
混入し、正しく works と学習された状態では混入しない」という対応は
この2つのテストから確認できており、**誤学習（`Imm32Unavailable` への
誤降格）と「あ」混入の因果関係自体は確定している**（詳細な出力経路——
降格後のどの VK 送信パスが「あ」を出力するか——はまだ未追跡、ADR-125
設計課題3の後半は引き続き未解決）。ただし**フレッシュな誤学習の瞬間を
実機で直接捕捉できたことは、この調査時点ではまだ一度も無い**。

**自動化スパイクでの追試（再現せず、2026-09-04）:** `ImmGetDefaultIMEWnd`
が一時的に NULL を返す実 Win32 レベルの競合状態という仮説を実測するため、
`crates/awase-windows/examples/spike_bug112_ime_wnd_race_probe.rs`
（`awase-settings.exe --bug-report` を繰り返し起動し、ウィンドウ検出から
`ImmGetDefaultIMEWnd` が non-NULL を返すまでの実時間を高頻度ポーリングで
測るスパイク）を作成し、(a) `awase.exe` を起動せず単独で15回、(b)
`awase.exe` を実際に起動した状態で15回、それぞれ実行した。**合計30回
すべてで `null_duration_ms=0`（初回ポーリングから即座に non-NULL）と
なり、NULL が観測されたことは一度も無かった。** `awase.exe` を起動する
だけでは競合は再現しない。

**手動追試（実際にタイピングして再現を試みる、再現せず、2026-09-04）:**
自動化スパイクは何も入力しないため、「実際のタイピング負荷が並走する
ことが必要条件」という上記仮説を直接検証するため、cache.toml を毎回削除
してフレッシュに起動 → 実際に親指シフトで入力、を3回連続で手動実施した
（前述の1回と合わせて計4回）。**4回とも `Unavailable` への誤学習は再現
しなかった**（3回目は cache.toml 自体が生成されず、学習イベントが
そもそも発生しなかった）。自動化スパイク30回と合わせ、**計34回試行して
一度も再現していない**。あ混入が実際に観測できた2回（本 BUG 冒頭の症状、
BUG-111 検証時）はいずれも前日までの ADR-125 調査（多数の cargo/rustc
プロセスが並行して走っていた等、リソース競合の強い環境）に由来する古い
キャッシュの再利用だった可能性が高く、**トリガー条件はこの調査セッション
内では特定できなかった**。次回セッションに持ち越す。

**現時点の解釈:** 「ウィンドウ生成直後の一時的な NULL」という単純な
起動レースだけでは説明がつかない可能性が高くなった。実際に「あ」混入が
観測された2回のテスト（本 BUG 冒頭の症状、および BUG-111 検証時）は
いずれも**ユーザーが実際に親指シフトで入力している最中**だった一方、
スパイクは起動するだけで何も入力しない。誤学習の発生には
**実際のキー入力（awase のフックが処理中）が並走していること**が
必要条件である可能性が新たな仮説として浮上した——競合が「新規ウィンドウ
の IME 未確立」ではなく「awase 自身が別の処理（打鍵処理・warmup・
actuation 等）でメッセージループ/COM 呼び出しが輻輳している最中に
`ImmGetDefaultIMEWnd` がその影響を受ける」という、awase 自身の負荷
依存の競合である可能性。次回セッションでの検証候補: (1) スパイクに
`SendInput` 等で合成キー入力を注入し typing 負荷を再現する、(2) 本番
コードパス（`learn_imm_capability_on_focus` 呼び出し箇所）に一時的な
高頻度診断ログを追加し、実際のユーザー入力再現時に直接タイミングを
記録する（このスパイクより信頼できるが、本番コードへの一時的な変更が
必要）。

**却下ではなく保留とした対策（`tuning-constants.md` 抵触のため）:**
「起動直後 N ms は確定させない」という時間ベースのグレースピリオドを
`record_null_probe`（`focus/classifier.rs`）に足す案は、上記のとおり
そもそも「起動直後の時間」が正しい軸かどうか自体が再検証を要するため
時期尚早。`.claude/rules/tuning-constants.md` は新規タイミング定数の
導入に実測値を要求するが、競合の真のトリガー条件（起動直後の経過時間
なのか、awase 自身の負荷なのか）がまだ実測で確定していないため、
恒久修正は次回セッションで上記の追加検証をしてから着手する。

**暫定回避（BUG-56/107 と同じ）:** `cache.toml` の
`[imm_capability."awase-settings.exe"] "Window Class"` エントリを削除し
awase を再起動する。GUI からの回避は BUG-108（学習キャッシュクリアの
no-op）が解消されるまで使えない。

**関連:** BUG-107（学習キャッシュのプロセス間キー汚染、本 BUG とは独立の
原因だが同じ症状「あ混入」を引き起こしうる）、BUG-111（同じ実機調査
セッションで発見した無関係な性能バグ、`[imm-learning]` ログの再発火）、
BUG-56（`record_null_probe` デバウンスの初出）、[ADR-125](adr/125-egui-winit-dynamic-ime-association-focus-model-gap.md)
（「未解決の設計課題」3 の一部を本 BUG が解決）、
[tuning-constants](../.claude/rules/tuning-constants.md)（恒久修正で新規
定数を導入する際に実測を要求するルール）。

**追記（2026-09-04、不具合報告 `01M1NEVHMXXY4903124J0RMS68` の調査で判明）:**
`awase-settings.exe` の `"Window Class"` とは別に、`lockapp.exe`/`explorer.exe`
系の UWP 入力ウィンドウ（journal 上のクラス名は `Windows.UI.Core.CoreWindow`、
ログ表示名は `"InputSiteWindowClass"`）でも `[imm-learning] profile 降格:
Standard → Imm32Unavailable` が短時間（1秒未満）に7回連続で再発火している
実機ログを確認した。同一セッションでその後 GJI のモード切替が不安定になる
症状（英数キーで OFF にした後、対になる ON キーを押すまで親指シフトが
再起動しない——ただしこの部分自体は仕様どおりの挙動と判定済み、本 BUG の
スコープ外）と description 本文中の不自然な「あ」の混入（「あさくらエディタで」
「あ症状が出ました」）が観測されたが、journal に「あ」の実際の出力イベント
自体は記録されておらず、この誤降格と「あ」混入の直接の因果は本セッションでは
確認できていない。**トリガー条件（このタイミングでこのプロセス/クラスの
組み合わせだけ誤降格する理由）は依然不明**だが、対象が `awase-settings.exe`
の `"Window Class"` 単体ではなく複数のプロセス・ウィンドウクラスにまたがる
ことを示す独立した実データとして記録する。詳細は
[docs/bug-reports-triage.md](bug-reports-triage.md) の当該 report 行を参照。

## BUG-113: Windows Terminal + GJI で、Engine 有効時に物理半角/全角キー（`VK_DBE_SBCSCHAR`）を押すと余分な「@」が出力される（**二重actuationの解消（GjiDirectStrategy OFF方向のAlreadyMatchedガード追加）を実装、実機検証待ち。probe競合はもう一つの独立した十分条件として残置**）

**アプリ:** Windows Terminal（`WindowsTerminal.exe`、`CASCADIA_HOSTING_
WINDOW_CLASS`/`Windows.UI.Input.InputSite.WindowClass`、`AppImeProfile::
TsfNative`）。
**IME:** Google 日本語入力（GJI）、`ActiveImeKind::GoogleJapaneseInput`。
JIS 106 キーボード。

**症状:** awase の NICOLA Engine が有効な状態で、物理半角/全角キー
（Windows API 上は `VK_KANJI` と呼ばれることが多いが、実際にこの機体の
awase フックに届く VK は後述のとおり別物）を押すと、他に一切キー入力を
していなくても、Windows Terminal 上に「@」が1文字出力される。

**再現手順（dragonflyg4 実機、2026-09-05 確認）:**
1. Windows Terminal + GJI で awase Engine を有効にする。
2. 物理半角/全角キーを1回押す（他のキーは押さない）。
3. 「@」が出力される。

**確認できなかった条件:** Engine を `Ctrl+Shift+無変換` で明示的に無効化
した状態では、同じキーを複数回押しても「@」は出ない（ユーザー実機報告、
ログでも該当する actuation ログの不在を確認済み）。

**原因（コード読解 + 実機ログ + 実機診断スクリプトで判明した範囲、
未確定部分あり）:**

1. この機体では物理半角/全角キーは `VK_KANJI`(0x19) ではなく、
   `VK_DBE_SBCSCHAR`(0xF3、半角方向)/`VK_DBE_DBCSCHAR`(0xF4、全角方向)
   として awase フックに届く（押すたびに交互にトグル）。物理「かな」キーが
   `VK_KANA`(0x15) ではなく `VK_DBE_HIRAGANA`(0xF2) として届くのと同型
   （[ADR-133](adr/133-gji-ime-mode-key-sendinput-batch-shape.md) 参照）。
2. `vk=0xF3`(半角方向、TurnOff) の直後だけ「@」が出現し、`vk=0xF4`
   （全角方向、TurnOn）では出現しないことをタイムスタンプ相関で2回連続
   確認した。
3. `runtime/key_pipeline.rs::kp_stage_shadow_ime_toggle` はこの VK を
   物理 IME キーとして検出し、shadow belief（IME ON/OFF の内部追跡）を
   更新する。この belief 更新自体は Engine の有効/無効と無関係に常時動く
   （実機ログで確認済み）。
4. `ime_controller.rs::GjiDirectStrategy::apply(open)` が実際の IME 制御
   VK 送信を担う。`apply(false)`（IME OFF 方向）は毎回 `send_ime_mode_key
   (VK_IME_OFF)` を呼ぶのに対し、`apply(true)`（IME ON 方向）は
   `view.control.shadow_on` が既に true なら `AlreadyMatched` で送信を
   スキップする no-op 最適化がある——これが「半角方向（OFF）だけ @ が
   出る」非対称性の候補。
5. この `send_ime_mode_key()` 呼び出し（＝実際の VK 送信、Engine が
   有効なときだけ発火することを実機ログで確認済み）は `tsf/output.rs::
   make_key_input_ex()` 経由で `KEYBDINPUT.wScan = 0` 固定で送信する。
6. dragonflyg4 実機で `MapVirtualKeyW`/`ToUnicodeEx` を `VK_IME_OFF` 等に
   対し直接呼び出して検証したが、単体では文字は生成されなかった
   （scan は正しく解決: `VK_IME_OFF→0xF1` 等）。したがって「Windows
   Terminal が生の VK を素朴に `ToUnicodeEx` で誤変換している」という
   単純な仮説はこの単体テストでは再現しない。**「@」の直接の生成源は
   GJI 自身の TSF が `wScan=0` の `VK_IME_OFF` の `SendInput` を Windows
   Terminal フォーカス中に受け取った際の内部 composition 処理にある
   可能性が高い**が、GJI はクローズドソースのため確定できていない。

**修正試行1（2026-09-05・反証・revert済み）:** `send_ime_mode_key` の
`wScan=0` を実 scan 付き送信（`make_scan_key_input()`）に変える修正を、
Windows Terminal 限定にせず全アプリ・全呼び出し元に対して既定で適用する
形で実装した。しかしユーザーが実機で A/B 検証したところ「@」の再現に
**一切変化が無かった**。`RUST_LOG=debug` で追加取得したログにより、この
`wScan=0` 仮説そのものが誤りだったことが判明した——このとき見つかった
drift correction の `FeedbackPolicy::Read` 無限に近い頻度の再送は
BUG-114 として切り出したが、**BUG-114 単独では「@」の必要条件ではない**
（下記フェーズ2参照）。`send_ime_mode_key` は元の `wScan=0` 固定へ戻して
いる（詳細は `docs/experiments.md` エントリ20）。

**修正試行2・調査（2026-09-05、同日続き）— 真因候補を
`VK_IME_OFF` 単体 `SendInput` バッチに絞り込み:**

ユーザーの指摘（「Ctrl+無変換 も同じ `send_ime_mode_key(VK_IME_OFF)` を
送っているのでは？」）がブレークスルーになった。確認すると、Ctrl+無変換
も物理半角/全角キーも **まったく同じ** `GjiDirectStrategy::apply(false)
→ send_ime_mode_key(VK_IME_OFF)` を通るのに、前者では「@」が一度も
出ない。両者の違いは、`send_ime_mode_key` が修飾キー（Ctrl/Shift）を
実際に押している場合だけ「release → OFF Down/Up → restore」を **1回の
`SendInput` バッチにまとめて** 送ること。物理半角/全角キー単独では修飾
キーが無いため `VK_IME_OFF` の Down/Up だけの**単体2イベントバッチ**に
なる。

これを検証するため、`GjiDirectStrategy::apply(open=false)` の送信方式を
呼び出しごとに自動ローテーションする診断スパイク（`config.toml` の
`diag_bug113_mode_cycle_enabled`、`ime_controller.rs`）を実装し、実機で
4モードを比較した（2026-09-05、複数ラウンドで再現確認済み）:

| mode | 送り方 | 「@」 |
| --- | --- | --- |
| 0（baseline） | 従来どおり `send_ime_mode_key(VK_IME_OFF)`（単体2イベント） | **出る** |
| 1（imm-cross） | `SendInput` を使わず `set_ime_open_cross_process(false)`（`ImmSetOpenStatus`、メッセージベース） | **出ない** |
| 2（keystate-clear） | `VK_IME_OFF` の直前に別の `SendInput` 呼び出しで `VK_DBE_SBCSCHAR` の synthetic KeyUp を送ってから mode 0 と同じ単体2イベントを送信 | **出る** |
| 3（fake-ctrl-bracket） | 実際には Ctrl を押させず、「Ctrl release → `VK_IME_OFF` Down/Up → Ctrl restore」を **1回の `SendInput` バッチにまとめて**送信 | **出ない** |

mode 2 は「残留キー状態」仮説（awase のフックが物理キーを suppress しても
`GetAsyncKeyState` 等のハードウェア直結の状態は「押されたまま」に見える
可能性）を検証する目的で先に実装したが、否定された。mode 3 の結果と
合わせると、**「@」の有無を分けるのは残留キー状態でも API の種類でもなく、
`VK_IME_OFF` の `SendInput` が単体2イベントのバッチとして送られるか、
他のイベントと一緒の複数イベントバッチとして送られるか**という「バッチの
形」らしいところまで絞り込めた。mode 0（baseline）で「@」が出た回では、
その直後に打った文字が正しく半角の "a" として出力されており（IME OFF
自体は機能した上で「@」が余分に混入している）、これは「@」が IME OFF
失敗の兆候ではなく紛れ込む余分な1文字であることを裏付ける。

**未解決:** (1) なぜバッチの形が結果を左右するのか、GJI 内部
（クローズドソース）の挙動としての説明は無い。(2) `apply(true)`
（IME ON 方向）は `AlreadyMatched` no-op ガードがあるため通常
`send_ime_mode_key` に到達せず、単体2イベントバッチでの `VK_IME_ON` 送信を
まだ検証できていない——ON 方向でも同じ「単体バッチで再現」条件が成立する
かは次セッションの検証候補（[ADR-133](adr/133-gji-ime-mode-key-sendinput-batch-shape.md)
「次セッションへの引き継ぎ」項目3、まだ未着手のまま持ち越し）。
(3) 検証の終盤、新規に開いた Windows Terminal ウィンドウで物理半角/全角
キーが常に `vk=0xF4 TurnOn（true→true, no-op）` としてしか認識されず
`TurnOff` 側が検出できなくなる現象が発生した（原因未調査、次セッション
持ち越し）。(4) 検証中、BUG-114 の drift correction 暴走（`FeedbackPolicy::
Read` 無限に近い再送）が本 BUG の再現手順を実行するだけで何度も自然発生し、
安全弁（`ime_controller.rs` の呼び出し回数上限）を追加してようやく実害を
抑えられる状態だった——BUG-114 は想定より頻発しやすい可能性が高く、
BUG-113 の修正と独立に優先度を上げて検討する価値がある。

**次にやること:** ~~mode=3（fake-ctrl-bracket）方式を恒久修正の第一候補と
する。~~ **追記（2026-09-05 続き）**: [ADR-133](adr/133-gji-ime-mode-key-sendinput-batch-shape.md)
を全面改稿し、mode=3 相当の偽 Ctrl ブラケットは実機検証済みのフォールバック
候補に格下げした。第一候補は新規に設計した「自己エコーパディング」
（対象 VK 自身を2度打鍵し、偽の修飾キーも新しい VK も持ち込まない）。
両候補とも Opus 敵対的レビュー未実施、自己エコーパディングは実機未検証
（ADR-133 D2/D4 参照）。

**追記（2026-09-05・実機A/Bでバッチ形状仮説を反証）**: ADR-133 v5 の
D0/D2 を `fix/bug113-114-ime-off-batch-and-feedback-staleness` ブランチで
実機スパイクした（BUG-114 の drift correction 暴走をほぼ解消した後の、
比較的クリーンな状態での検証）。候補V（`SendInput` 分割）・候補A
（自己エコー）・候補B3/B4（偽Ctrlブラケット、3/4イベント）・mode=0
（対照群）を巡回したところ、**ユーザー報告により「ほぼ毎回、候補を
問わず『@』が出る」ことが判明した**。さらに、当初 BUG-113 発見の
きっかけだった「Ctrl+無変換 では『@』が一度も出ない」という観測自体、
実機で再確認したところ**最初の1回がたまたま出なかっただけで、2回目
以降は Ctrl+無変換 でも『@』が出る**ことが分かった——これは round1
敵対的レビューの Major 7（「4回連続で出ないのは真の発生率30%でも24%の
確率で起こりうる」という統計的脆弱性の指摘）がまさに的中した形である。

**結論: 「`SendInput` バッチのイベント数・修飾キーの有無が『@』の
有無を左右する」というバッチ形状仮説は反証された。** 全候補・
Ctrl+無変換を含むすべての条件で「@」が再現するため、唯一の共通点は
「`VK_IME_OFF`(0x1A) を送信していること」自体になる。ADR-133 Major 5
が指摘していた「JIS 106配列で `@` キーのスキャンコードが 0x1A と一致する」
という機構仮説（`wVk` の値がどこかでスキャンコードとして誤読されている
可能性）が、バッチ形状よりも有力な説明として残った。

D0-3（`KanjiToggleStrategy`/`VK_KANJI` 単独発火、Alt+物理半角/全角キーで
トリガー）でこの仮説を検証しようとしたところ、**診断コード自体に実装
バグが見つかった**: 1回の物理キー押下が内部で2回（`shadow_toggle_off_sync`/
`engine_decision_sync`）呼ばれる際、1回目だけを `KanjiToggleStrategy` へ
誘導し2回目を吸収し忘れていたため、2回目が通常の mode-cycle（候補
V/A/B3/B4 のいずれか）に素通りしていた。ユーザーが Alt+半角/全角キーで
観測した「@」は、このバグにより **`KanjiToggleStrategy` 由来ではなく
2回目呼び出しの mode-cycle 候補由来だった可能性が高い**——D0-3 は実質
未検証のまま持ち越しとなった。episode 単位で判定を1回に固定し2回目を
明示的に no-op で吸収するよう修正済み、次のスパイクで再検証する。

**追記（2026-09-05 続き・VK 値のスキャンコード誤読仮説も反証）**:
ADR-133 Major 5（JIS 106配列で「@」キーのスキャンコードが `VK_IME_OFF`
(0x1A) の値と一致するという機構仮説）を検証するため、`KanjiToggleStrategy`
（`VK_KANJI`=0x19、JIS 配列なら本来「P」）を Alt+物理半角/全角キーで
強制発火させる診断（D0-3、`ime_controller.rs`/`ime.rs` の
`diag_bug113_*` 一式）を実機で実行した。結果、**「p」は一切混入せず
「@」のみ出力された**。`VK_IME_OFF`(0x1A) でも `VK_KANJI`(0x19) でも
「@」しか出ないため、VK 値そのもの・スキャンコード誤読という機構仮説も
反証された。

**追記（2026-09-05 続き・PSReadLine との相互作用を確認、ただし「awase側の
問題ではない」は誤った結論——訂正）**:
ユーザーの追加観測（(1) awase Engine を `Ctrl+Shift+無変換` で無効化する
と発生しない、(2) 同じ Windows Terminal 内でも SSH/MSYS2 bash セッション
では発生せず PowerShell セッションでのみ発生する）から PowerShell の
PSReadLine（予測入力/ゴーストテキスト表示モジュール）との相互作用を疑い、
PowerShell 上で `Remove-Module PSReadLine -Force` を実行して同モジュールを
無効化したところ、**「@」が発生しなくなることを実機確認した**。

セッション終盤、この結果を「GJI と PSReadLine の相互作用が真因であり
awase 側のコードバグではない」と結論づけて本 BUG をクローズ扱いにしたが、
**この結論はユーザーのレビューで誤りと指摘され撤回した**。トリガーは
一貫して「awase が GJI に対して何らかの IME actuation（mode key
送信）を行うこと」であり、PSReadLine/GJI 単体ではこの現象は起きない。
「相手側の実装が脆弱である」ことは、その脆弱性を実際に踏み抜いている
awase の送信動作が無関係であることを意味しない——これは本リポジトリの
他の既知バグ（Chrome cold-start、TSF native 系等）が「相手アプリ/IME の
実装が原因」であっても awase 側でタイミング調整・送信方式変更・
ブラックリスト等の緩和策を講じてきたのと同じ扱いにすべきだった。

**未解決・次にやること（実機切り分け、第2弾スパイク実装済み・実機検証待ち）:**
バッチ形状（V/A/B3/B4/baseline）と VK 値（`VK_IME_OFF` vs `VK_KANJI`）は
いずれも無関係と判明した。上記「修正試行2」の mode テーブルで唯一
「出ない」という結果だった mode 1（imm-cross、`set_ime_open_cross_process`/
`ImmSetOpenStatus`）は、**候補から除外した**: `AppImeProfile::TsfNative`
（Windows Terminal 含む）は `can_use_imm32_cross_process() == false`
（`focus/class_names.rs`）であり、この API はそもそも TSF アプリに
効果を持たない。したがって mode 1 で「@」が出なかったのは「回避できた」
のではなく「単に IME 状態が変化しなかった」だけの可能性が高く、意味のある
比較にならない。

2026-09-05 続き、呼び出し連鎖の全数調査（物理半角/全角キー押下から
awase が行う Win32/TSF 呼び出しを observer/probe/actuation すべて洗い出し）
で、**バッチ形状・VK値ラウンドでは一度も検証されていない新しい候補**を
発見した: `runtime/key_pipeline.rs::kp_stage_idle_conv_check`
（アイドル500ms超・直近1500ms以内に明示的IME操作なしという、本 BUG の
再現条件と一致するゲート）が非同期ワーカースレッド上で
`get_ime_conversion_mode_raw_timeout_async`（GJI の IME ウィンドウへの
`SendMessageTimeoutW(WM_IME_CONTROL/IMC_GETCONVERSIONMODE)`、**読み取り
専用**のcross-processクエリ）を spawn しており、これは
`GjiDirectStrategy::apply` の同期 `SendInput` 発行（`shadow_toggle_off_sync`
経由、`kp_run_inner` のステージ順でこの spawn より後）と時間的に競合
しうる。この読み取りは `docs/known-bugs.md` の「Windows Terminal 向け
IMM クエリスキップ（`ime.rs::read_ime_state_full` の `is_tsf_native_window`
早期return）」の対象**外**——別のコードパスであり、スキップされずに
実際に発行される。

第2弾診断スパイク（`fix/bug113-114-ime-off-batch-and-feedback-staleness`
ブランチに実装済み、実機投入待ち）は、2つの独立した hidden config
トグルで以下を切り分ける（`ime_controller.rs`/`key_pipeline.rs`/
`config.rs` 参照。組み合わせは config.toml を編集して awase を再起動する
ことでテストする。1トグル・1状態あたり最低20 episode程度の試行を推奨
——round1 レビュー Major 7 の統計的脆弱性の教訓）:

1. `diag_bug113_dedup_gji_off_actuation`: 同一の物理キー押下で
   `shadow_toggle_off_sync`/`engine_decision_sync` の2経路から
   `GjiDirectStrategy::apply(open=false)` が2回呼ばれ、実際に
   `send_ime_mode_key` の `SendInput` を2回発行している点（BUG-113調査中
   に発見済み）自体が「@」の必要条件かどうかを、1クローズepisodeにつき
   1回だけの実アクチュエーションに絞って確認する。
2. `diag_bug113_skip_idle_conv_probe`: 上記で新たに見つかった
   `WM_IME_CONTROL/IMC_GETCONVERSIONMODE` の cross-process 読み取りを
   丸ごとスキップし、「@」の引き金が実際にこの読み取りとの競合なのかを
   確認する。

**追記（2026-09-05 続き・実機A/Bで必要十分条件を確定）**:
上記2トグルを config.toml 編集なしで1セッション内で自動ローテーション
する第3弾スパイク（`diag_bug113_combo.rs`、`kp_run_inner` 冒頭で物理・
非注入の `ShadowImeAction::TurnOff` 方向 KeyDown ごとに4条件
[0=baseline/1=dedupのみ/2=probe skipのみ/3=両方] を巡回、`ime_controller.rs`・
`key_pipeline.rs` の両方が同じコンボを参照）を実装・実機投入した。

実装初期に2件のバグを実機ログで発見・修正した（教訓として記録）:
(1) コンボ選択に `!event.injected` ガードが無く、`GjiDirectStrategy::
apply` 自身が送る `VK_IME_OFF`/`VK_IME_ON` の `SendInput` が injected=true
で hook にループバックした際にもコンボが進んでしまっていた。
(2) VK 分類（`event.ime_relevance.shadow_action`）による限定
（`.is_some()`）だけでは、物理半角/全角キーが `TurnOff`(vk=0xF3)/
`TurnOn`(vk=0xF4) を押すたびに厳密に交互する（周期2）のに対しコンボの
巡回周期が4（2の倍数）であるため、**TurnOff 方向が構造的に必ず偶数
コンボ（0か2、dedup=false）にしか当たらないというエイリアシング**が
起きていた——dedup=trueが割り当たコンボ1・3は全てTurnOn方向の押下で
あり、「@」が起きえない方向でしかdedup=trueを検証できていなかった。
`shadow_action == Some(TurnOff)` に限定してコンボを進めるよう修正し
解消した（詳細は `runtime/key_pipeline.rs` のコミット履歴・
docs/experiments.md エントリ22参照）。

この2件を修正した版で実機A/Bを実施した結果、**「@」は combo=0
（baseline、dedup=false かつ probe skipなし）のときにのみ、極めて
整合的に（ユーザー表現: 「4回に1回、きわめて整合的」）発生し、
combo=1（dedupのみ）・combo=2（probe skipのみ）・combo=3（両方）では
一度も発生しなかった**（各条件15〜16トライアル、TurnOff方向のみ数えて
合計63トライアル中、baseline以外はゼロ）。すなわち:

- **`GjiDirectStrategy::apply(open=false)` の二重actuation
  （`shadow_toggle_off_sync`/`engine_decision_sync`）を1回に減らす
  （dedup）ことは、単独で「@」を防ぐのに十分**である。
- **`kp_stage_idle_conv_check` が spawn する GJI への cross-process
  読み取り（`WM_IME_CONTROL/IMC_GETCONVERSIONMODE`）を丸ごとスキップ
  することも、単独で「@」を防ぐのに十分**である。
- 両方無効化しても（当然）「@」は出ない。

**結論:** 「@」は、この読み取り専用クエリと、`SendInput` によるIME mode
key actuation（特にその二重発行）とが時間的に近接して発生することで
GJI の TSF composition 追跡を乱す、という機構が濃厚である
（PSReadLine の再描画/バッファ処理との相互作用は、この乱れを可視化する
経路として働いていると考えられる——GJI 自体がクローズドソースのため
内部機構としての確定はできない）。**いずれの要因も awase 自身が
制御可能な送信動作であり、外部要因（PSReadLine）の発見は awase 側の
修正が不要であることを意味しない。**

**追記（2026-09-05 続き・恒久修正を実装、実機検証待ち）**: ユーザー判断で
二重actuation解消（dedup相当）を先に実装することにした。
`GjiDirectStrategy::apply` の OFF方向に、既存のON方向
（`open && view.control.shadow_on` → `AlreadyMatched`）と対称な
`!open && !view.control.shadow_on` → `AlreadyMatched` ガードを追加した。
`view.control.shadow_on` は `ImeModel.applied_pair()` 由来で、直近の
apply が `Applied`/`FallbackSent`/`AlreadyMatched` で成功した場合のみ
更新される（`UnsafeToToggle`/`Failed` では更新されないため、未送信・
失敗後の正当な再試行は従来どおり実際に送信される——`record_ime_apply_result`
の分岐で確認済み）。これにより `shadow_toggle_off_sync`/
`engine_decision_sync` の2回目の呼び出しは、1回目が成功していれば
`AlreadyMatched` で吸収され、実際に `SendInput` するのは1回だけになる。
診断専用コード一式（`diag_bug113_combo.rs`、`diag_bug113_dedup_gji_off_
actuation`/`diag_bug113_skip_idle_conv_probe`/`diag_bug113_combo_cycle_enabled`
とその呼び出し元）は本修正と同じコミットで撤去済み。回帰テスト
`gji_direct_apply_off_is_already_matched_when_shadow_already_off`
（`ime_controller.rs`）を追加した。

もう一つの独立した十分条件——`kp_stage_idle_conv_check` の cross-process
読み取りとの時間的競合——は今回は対応しない。このprobeは他のシナリオ
（アイドル明けの conv-mode belief 回復）のために存在する正規機能であり、
丸ごと無効化するのではなく GJI actuation と時間的に重ならないようにする
設計が必要——次に着手する場合は別途新しい診断コードを起こすこと。

**関連:** [ADR-133](adr/133-gji-ime-mode-key-sendinput-batch-shape.md)
（当初「`VK_KANA`/`VK_KANJI` 自体の文字化」という前提で起票されたが、
本 BUG の調査により前提が反証され、真因候補が `send_ime_mode_key` の
`wScan=0` →（反証）→ BUG-114 →（BUG-114単独では説明できないと判明）→
`VK_IME_OFF` 単体 `SendInput` バッチ →（反証）→ VK 値のスキャンコード
誤読 →（反証）→ PSReadLine との相互作用が引き金（確認、ただし
awase側の送信方式・呼び出し経路のどれが必要十分条件かは未特定）、と
段階的に絞り込まれた経緯を記録)、
BUG-114（同じ調査から派生した独立のバグ、drift correction の
`FeedbackPolicy::Read` 無限再送。こちらは awase 側の実在バグとして別途
修正済み）、BUG-110/ADR-132（同じ「Windows Terminal
+ GJI で余分な @」という症状クラスの別原因、`IntentStore`/`last_intent`
の絶対的権威化が真因——本 BUG・BUG-114 とは異なる独立の原因である点に
注意）。`spike/adr133-wt-vk-kana-dbe-hiragana` ブランチ（`VK_KANA` 置換、
この機体では到達不能コードだったため不要と判明）は削除候補。

## BUG-114: Windows Terminal（TsfNative プロファイル）の `FocusChanged` 分類が `Standard`/`ImmCross` にフォールバックし、drift correction が `FeedbackPolicy::Read` で `VK_IME_OFF` を無限に近い頻度で再送し続ける（**ADR-134 D1c + AnyFreshEvidence除外拡張で修正・実機確認済み**）

**アプリ:** Windows Terminal（`WindowsTerminal.exe`、`CASCADIA_HOSTING_
WINDOW_CLASS`/`Windows.UI.Input.InputSite.WindowClass`、`AppImeProfile::
TsfNative`）。
**IME:** Google 日本語入力（GJI）、`ActiveImeKind::GoogleJapaneseInput`。

**症状:** BUG-113 の実機再現手順（Engine 有効時に物理半角/全角キーを1回
押す）を実行すると、`RUST_LOG=debug` のログ上、`runtime/ime_refresh.rs`
の drift correction が **約14秒間、20〜90ms おきに連続発火**し、その
たびに実際に `SendInput` で `VK_IME_OFF` (0x1A) を送り続ける
（`gave up` ログは1件も出ない）。BUG-113 の「@」はこのバーストによって
GJI の TSF composition が乱される副作用である可能性が高い（GJI 自身の
内部処理はクローズドソースのため確定はできない）。

**再現ログ（dragonflyg4 実機、2026-09-05 確認、抜粋）:**

```
[drift] correction: observed=true ≠ desired=false for 1837ms → set_ime_open(false)
[warrant-shadow] chain=sync open=false origin=EventOrigin { source: SelfActuated { strategy: "drift_correction_read" }, epoch: Generation(31) } ...
[apply-ime] GJI direct: send 0x001A (open=false)
[ime-mode] SendInput vk=0x1A ... total=2 events
[hook] IME-mode vk=0x1A down self_injected=true injected=true scan=0xF1 ...
[hook] IME-mode vk=0x1A up self_injected=true injected=true scan=0xF1 ...
[apply-ime] open=false eff=false conf=true → outcome=Applied
Blacklist drift correction: apply_ime_open(false) → Applied
（20〜90ms後、observed=true のまま再度同じ流れが発生。以下 duration_ms が
 単調増加しながら約14秒間・数十回連続。ログ全文で "gave up" は0件）
```

**原因（コード読解 + 実機ログで確定）:**

1. `state/app_ime_policy.rs::caps()` は `(ImePolicyProfile::TsfNative,
   ImeKindId::Gji)` に対して `FEEDBACK_BLIND`（`max_attempts` で
   `GiveUp` する有界な再試行方針）を割り当てるよう設計されている。
   ところが実機ログの `strategy=` タグは一貫して `drift_correction_read`
   ——つまり実際に使われたのは `FeedbackPolicy::Read`（`decide_actuation_action`
   に `GiveUp` 分岐が無く、`Blind` と異なり常に `Send` を返す無条件再送
   ポリシー）だった。
2. `app_policy`（`default_feedback`/`focus_settle_ms` の SSOT）は
   `ImeEvent::FocusChanged { profile, .. }` 受信時
   （`state/ime_model.rs:615`）にのみ `AppImePolicy::from_profile(profile)`
   で更新され、以後は同じフォーカスセッション中ずっとそのまま使われる。
   `profile` は `FocusChanged` dispatch 時点
   （`runtime/focus_tracking.rs:537`）の `self.platform.current_app_profile()`
   の**その瞬間のスナップショット**である。
3. `focus/class_names.rs::AppImeProfile::from_class_name` は
   `IMM32_UNAVAILABLE_CLASSES` → `is_tsf_native_window` の順で判定し、
   どちらにも一致しない場合は `Standard`（`#[default]`）にフォールバック
   する。`ImePolicyProfile::from(AppImeProfile::Standard) = ImmCross`
   であり、`caps(ImmCross, Gji) = FEEDBACK_READ` となる。
4. すなわち `FocusChanged` が発火した**その瞬間**に、フォーカス中クラス名が
   `CASCADIA_HOSTING_WINDOW_CLASS` / `Windows.UI.Input.InputSite.WindowClass`
   のいずれとも一致していなかった（別の一時的なクラス名だった、または
   フォーカス追跡の内部状態がまだ更新されていなかった等）場合、
   `AppImeProfile::from_class_name` は `Standard` を返し、`app_policy` に
   `FEEDBACK_READ` が焼き付く。`current_app_profile()` 自体はその後の
   全ての呼び出し（`[apply-ime] GJI direct: send` 選択、`dispatch-ime`
   の `profile=TsfNative` ログ等）で正しく `TsfNative` を返しているが、
   `app_policy` はそれを一切参照しない（次の `FocusChanged` まで固定）
   ため、この不整合はフォーカスセッション全体を通じて解消されない。
5. `FeedbackPolicy::Read` は「実読み戻し可能なプロファイル（ImmCross 等）」
   向けの設計であり、`sent_at` 以降の trusted 観測が `desired` と一致すれば
   `Confirmed` として再送を止める。しかし Windows Terminal の
   `Windows.UI.Input.InputSite.WindowClass` は `read_ime_state_full` が
   `Skipping IMM query for known-broken class` として IMM クエリ自体を
   スキップするクラスであり、`Read` が待っている「収束を示す trusted
   観測」が構造的に一生発生しない。結果、`decide_actuation_action` は
   毎 tick（drift correction の観測周期、実測 20〜90ms）
   `ActuationAction::Send` を返し続け、`VK_IME_OFF` の `SendInput` が
   無制限に近い頻度で繰り返される。

**未確定部分:** `FocusChanged` 発火の瞬間に実際にどのクラス名が
`current_app_profile()` に渡っていたか（`Standard` へのフォールバックが
実際に発生した証拠ログ）は今回のログ取得範囲では直接確認できていない。
`caps(ImmCross, Gji) = FEEDBACK_READ` であることから逆算した推定であり、
`AppImeProfile::Standard` 以外の経路（例えば `InputRelay` 誤判定、
`ImePolicyProfile::from` の別分岐）で同じ `Read` に到達する可能性も
コード上は排除できていない。次のログ取得では `FocusChanged` 発火時刻
前後の `on_focus_process_changed`/`current_app_profile()` 呼び出しに
一時的な診断ログを追加し、実際の `profile` 値を直接確認するとよい。

**現状:** 原因のメカニズム（`FeedbackPolicy` が `FocusChanged` 時の
プロファイル分類スナップショットに固定され、`current_app_profile()` の
後続の正しい再分類を反映しない）まで特定済み、修正未着手。この
`app_policy` スナップショット方式・`caps()` の feedback 割り当て・
`Read`/`Blind` の GiveUp 非対称性はいずれも IME belief アーキテクチャの
中核（`state/app_ime_policy.rs`、`state/ime_model.rs::FocusChanged` 分岐）
に関わるため、修正方針はユーザー判断により Opus 敵対的レビューで設計
してから実装する。

**追記（2026-09-05 続き）**: [ADR-134](adr/134-drift-correction-feedback-policy-focus-snapshot-staleness.md)
として修正方針を起票した。`can_use_imm32_cross_process()`（ライブ判定、
`ir_resolve_skip_imm_query`/実送信ルーティングが既に使っている）と
`AppImePolicy::default_feedback`（`FocusChanged` 時スナップショット）が
食い違う SSOT 分裂が本質と特定し、`ir_apply_drift_correction` の
policy 決定だけをライブ判定から再導出する最小修正案（D1）を提示した。
`actuation_for` の「reuse 時 policy 無視」不変条件は変更しない。
Opus 敵対的レビュー未実施。

**追記（2026-09-05・実機再現でADR-134 Finding 5を確認、修正実装）**:
BUG-113 の診断スパイク（ADR-133 D0/D2、`fix/bug113-114-ime-off-batch-and-
feedback-staleness` ブランチ）を実機（dragonflyg4、Windows Terminal +
GJI）でテスト中、`RUST_LOG=debug` のログに `[drift] correction:
observed=true ≠ desired=false for Nms` のバーストが**5回**自然発生した
（各バースト5回の再送、`[drift] actuation gave up (Blind): ...
attempts=5` で終了、バースト間隔は7秒〜60秒とばらつきあり）。この間
`[bug114-diag]`（ADR-134 D4、`app_policy` スナップショットとライブ判定の
食い違いを記録する診断ログ）は**1件も出力されなかった**——つまり今回の
暴走は上記の「`FocusChanged` 時点の誤分類」（根本原因1〜3、D1/D1c で
対処予定）とは**独立**に発生した。`attempts=5` の `GiveUp` が明確に出て
いることから `FeedbackPolicy::Blind` は正しく選択されており、ADR-134が
「Finding 5」として指摘していた別の欠陥——`ReadBackQuery::AnyFreshEvidence`
が `Blind` 戦略（TsfNative/Imm32Unavailable）の GJI I/O 活動監視
（`observe_gji_after_focus` が書く `ObservationSource::ObserverPoll`）を
無区別に「外界が動いた証拠」として採用してしまい、`GiveUp` 後の
3秒クールダウン明けにほぼ確実に再武装してしまう——が実機で直接確認
された。

修正: `state/observation_store.rs::ObservationStore::read_back` の
`ReadBackQuery::AnyFreshEvidence` 分岐のみ、`ObserverPoll` ソースを鮮度
判定から除外する（`most_recent_trusted_after_excluding` 新設）。
`ReadBackQuery::Converged`（`FeedbackPolicy::Read` の収束確認、genuine な
`OsPoll` 戦略の `ObserverPoll` に依存）には触れない——`Blind` が割り当て
られるプロファイル（Imm32Unavailable/TsfNative）は構造的に常に
`Blacklist` 読み取り戦略になり `OsPoll` を経由しないため、この除外は
`Blind` 文脈でのみ意味を持ち、`Read` 文脈の genuine な `ObserverPoll` を
巻き込まない。回帰テスト3件
（`read_back_any_fresh_evidence_ignores_observer_poll_alone` 等）を追加。

**追記（2026-09-05 続き・実機再検証で根本原因1を直接確認、D1c実装）**:
上記修正（ObserverPoll 除外）を実機に投入して再検証したところ、暴走は
解消せず**別の形**（`gave up (Blind)` が一度も出ないまま
`observed=true ≠ desired=false for Nms` が単調に増加し続ける、無条件・
無停止の毎tick再送）で再現した。この時 `[bug114-diag]`（ADR-134 D4）が
**全ドリフト判定と同数**出力され、常に
`snapshot_policy=Read { source: ImmGetOpenStatus, .. } live_policy=Blind
{ .. } current_focus=None` だった。`current_focus=None` は
`FocusChanged` の reducer（`state/ime_model.rs`）でのみ書かれるフィールド
であり、**起動から一度もプロセス切替が起きていない**ことの直接証拠——
まさに ADR-134 根本原因1（bootstrap 窓）が実機で成立していたことを
確定させた。ユーザー報告によれば同一の Windows Terminal でも
SSH/MSYS2 セッションでは再現せず PowerShell セッションで再現する
とのことだが、これは awase 再起動直後にどのタブ/シェルへフォーカスして
いたか（＝ bootstrap 窓に入るかどうか）に依存すると考えれば整合する
（シェルの種類自体が分岐条件になっているわけではないと推定、未検証）。

修正（ADR-134 D1c）: 起動直後の初回フォーカス確立時（
`runtime/focus_tracking.rs::establish_initial_focus_scope`）に、新設した
`ImeEvent::InitialAppPolicyEstablished { profile }` イベント
（`sync_initial_app_policy` から dispatch、`advance_focus_tracking` の
後・`sync_initial_focus_fence` と同じタイミング）で `app_policy` を
live の profile 分類から初期化する。`InitialFocusFenceEstablished`
（`current_fence` 1フィールドの差し替えのみという ADR-102 決定3-b の
不変条件）とは意図的に別イベントにし、同じ「1フィールドだけ差し替え」
規律を新イベントにも適用（reducer 実行時テスト
`initial_app_policy_established_touches_only_app_policy`、テキスト
スキャンの構造ガード `initial_app_policy_event_only_touches_app_policy`
の2段で固定）。D1（`ir_apply_drift_correction` のライブ再導出）・D1a
（`ImePolicyProfile::InputRelay` 追加）は未実装のまま——D1c が bootstrap
窓を塞いだ後の実機ソークで、根本原因2系（`FocusChanged` を経由しない
ライブ分類変化）が残存するかを見てから着手する。

**追記（2026-09-05 続き・D1c実機再検証、AnyFreshEvidence除外の第2弾）**:
D1c を実機に投入して再検証したところ、根本原因1（`current_focus=None`）
は解消した（`[app-policy] bootstrap initial app_policy: profile=TsfNative`
のログで確認、`[bug114-diag]` の発火も0件）——しかし `gave up (Blind)`
が正しく5回で発生しているにもかかわらず、暴走は**別の形**（3秒
クールダウン明けにほぼ即座に再武装するバーストが繰り返す、Finding 5と
同型）で残った。ログを詳細に追ったところ、犯人は `ObserverPoll` では
なく **`ConvOpenInference`**（`kp_stage_idle_conv_check` が conv ビットの
`NativeToggleShadowOff` から書く open 推測、`[idle-conv-check] TsfNative:
conv observation open=true reason=NativeToggleShadowOff ... →
ObserverReported として記録` ログで確認）だった。これは shadow-toggle
自身が動かした conv 状態を読み返すだけの自己言及的な信号で、実機で
~250〜380ms 間隔という高頻度で record され続けていた。

修正: `read_back` の `AnyFreshEvidence` 分岐の除外対象を
`ObserverPoll`・`ConvOpenInference` の2ソースに拡張した
（`most_recent_trusted_after_excluding` を単一ソースから
スライス受け取りに変更）。回帰テストも2ソース目
（`read_back_any_fresh_evidence_ignores_conv_open_inference_alone`）を
追加。除外リストは実機で新たな自己言及ソースが見つかるたびに拡張する
前提——コード上のコメントにその旨を明記した。

**追記（2026-09-05 続き・実機再々検証で解決を確認）**: D1c + AnyFreshEvidence
除外拡張（2ソース）を実機に投入して再々検証した結果、ログ全体
（94,913行）で `drift correction` バーストは**1回のみ**、その1回も
`gave up (Blind)` 後に一度も再武装しなかった（`fresh observation after
give-up` 0件）。BUG-113 の実機診断中（物理半角/全角キーを多数回押す
負荷テスト）でもこの水準に収まっており、**BUG-114 の drift correction
暴走は実質解消したと判断する**。

**現状**: D1c + AnyFreshEvidence 除外拡張（2ソース）を実装済み・実機で
解決を確認。D1（`ir_apply_drift_correction` のライブ再導出）・D1a
（`ImePolicyProfile::InputRelay` 追加）は根本原因2系（`FocusChanged` を
経由しないライブ分類変化）向けの補完として未実装のまま残す——実機ソーク
でこの経路の再発が確認された場合に着手する。

**関連:** [ADR-133](adr/133-gji-ime-mode-key-sendinput-batch-shape.md)
（本 BUG が発見された調査の出発点、BUG-113 の `wScan=0` 仮説が反証された
経緯）、[ADR-134](adr/134-drift-correction-feedback-policy-focus-snapshot-staleness.md)
（本 BUG の修正方針）、BUG-113（本 BUG により再現する症状「@」の直接の親 BUG）、
BUG-110/ADR-132（`IntentStore`/`last_intent` の絶対的権威化が真因の、
同じ症状クラスの別原因——独立した原因である点に注意）、
[ime-belief-architecture](../.claude/rules/ime-belief-architecture.md)
（`ImeModel` 以外の belief 的状態にも3段防御の適用要否を検討する際の
判断基準）。
