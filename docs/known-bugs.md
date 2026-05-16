# awase 既知の不具合

> 最終更新: 2026-05-15

---

## BUG-01: TSF cold-start — 最初の1文字がリテラル ASCII になる (WezTerm)

**症状:** WezTerm でひらがな入力の最初の1文字が文字化けしてリテラル ASCII になる。
例: 「あいうえお」と打つと「aいうえお」「kおれ」「nい」のようになる。

**原因:** WezTerm は TSF (Text Services Framework) native app であり、F2 (VK_DBE_HIRAGANA) を受信してから TSF composition context の初期化を非同期で行う。この初期化には実測 ~305ms かかることがある。awase の romaji SendInput バッチが初期化完了前に到達すると、1文字目が IME を通らずリテラル ASCII として PTY に直送される。

**現在の対策:** `send_eager_tsf_warmup()` — cold になったタイミングで即座に F2 を送信し、timestamp を記録する。`send_romaji_as_tsf()` が cold を検出したとき:
- `eager_elapsed >= 500ms` → 即送信（TSF は確実に初期化済み）
- `eager_elapsed < 500ms` → `500ms - elapsed` だけ sleep してから送信
- eager warmup なし（Enter/Escape 直後）→ F2 送信 + 40ms sleep

**cold になるトリガー:**

| ColdReason | 発生タイミング | eager warmup |
|---|---|---|
| `FocusChange` | ウィンドウ切り替え | ✅ 即送信 |
| `PassthroughConfirmKey` | Enter/Escape/Space 直接通過 | ✅ 即送信 |
| `ReinjectConfirmKey` | Enter/Escape/Space reinject | ✅ 即送信 |
| `NativeF2Consumed` | 物理 F2 キー押下 | ✅ 即送信 |
| `SetOpenTrue` | IME OFF→ON トグル | ❌ なし（40ms sleep） |
| `SymbolVkSent` | 記号 VK 送信後 | ❌ なし（40ms sleep） |
| `SessionExpired` | 2000ms 沈黙後の次打鍵 | ❌ なし（40ms sleep） |
| `F2NonTsf` | Chrome/Win32 での F2 通過 | ❌ なし（40ms sleep） |

**残存リスク:**
- 500ms の閾値は実測 305ms に余裕を持たせたものだが、マシン負荷が高い場合や別アプリ起動直後に WezTerm へフォーカスが当たるケースなど、500ms を超える初期化時間が発生する可能性がある。
- `SetOpenTrue` / `SymbolVkSent` / `SessionExpired` は eager warmup なし（40ms sleep）のため、高速タイピストには不十分な可能性がある。

**関連ファイル:** `output.rs:send_romaji_as_tsf()`, `output.rs:send_eager_tsf_warmup()`, `executor.rs`, `runtime.rs`

---

## BUG-02: Chrome cold-start — 最初の1文字がリテラル ASCII になる (Chrome/Edge/Electron)

**症状:** BUG-01 と同様だが Chrome/Edge/Electron (VK Batched モード) で発生する。

**原因:** Chrome も F2 受信後の IME 初期化が非同期。BUG-01 と同じ構造。

**現在の対策:** `send_romaji_batched()` — F2-only バッチを先行送信した後、IMM32 conversion mode API (`get_ime_conversion_mode_raw_timeout(10ms)`) を最大 15 回ポーリングして Chrome が F2 を処理済みか確認してからローマ字バッチを送信する。

**WezTerm と異なる点:** Chrome は IMM32 HIMC を持つため IMM32 API による検出が可能。WezTerm は TSF native のため同じ手法が使えない（→ BUG-03 参照）。

**残存リスク:** ポーリング間隔 10ms × 最大 15 回 = 最大 150ms 待機。Chrome の応答が 150ms を超える場合（重負荷時）は文字化けが発生しうる。

**関連ファイル:** `output.rs:send_romaji_batched()`

---

## BUG-03: WezTerm で ImmGetCompositionStringW が常に 0 を返す

**症状:** WezTerm の TSF warm 状態を IMM32 API で検出できない。

**原因:** WezTerm は TSF native app であり、TSF composition string を IMM32 HIMC に propagate しない。そのため `ImmGetCompositionStringW(himc, GCS_COMPSTR, ...)` は TSF composition が active でも常に 0 を返す。

**影響:** probe-and-retry アプローチ（warm 検出後に romaji 送信）は WezTerm では機能しない。実際に試みたところ（commit 558c39f → b643bac で削除）、最大 952ms の待機の後 fallback 送信になるだけで、検出には使えないことが確認された。

**回避策:** なし。固定 sleep ベースの戦略（BUG-01 参照）に依存している。

---

## BUG-04: WinEvent IME フック (IME_START/CHANGE/END) が WezTerm で発火しない可能性

**症状:** `SetWinEventHook(EVENT_OBJECT_IME_START / CHANGE / END)` で TSF composition の開始・変化・終了を検出しようとしているが、WezTerm が実際にこれらのイベントを発行するか未検証。

**原因:** WezTerm は TSF native app。IME_START 等の WinEvent は IMM32 ベースのアプリが発行するものであり、TSF native app では発行されない可能性がある。

**影響:** composition 開始をイベントで検出して warmup を判断する戦略が機能しない可能性。

**確認方法:** awase の `[ime-event]` ログを見て `IME_START / IME_CHANGE / IME_END` が出力されているか確認する。

---

## BUG-05: session timeout (2000ms) の閾値が任意値

**症状:** 前回 SendInput から 2000ms 以上経過した後の最初の打鍵で cold-start と同じ経路（F2 warmup 再送信）が走る。

**原因:** composition context は時間経過で無効化される可能性があるが、正確な timeout 時間は不明のため 2000ms を保守的な閾値として設定している。

**残存リスク:** 
- 2000ms より短い時間でも context が失効するケースがあれば文字化けが起きうる（例: 500ms 沈黙後の入力再開）。
- 2000ms より長い時間でも context が維持されるなら不要な warmup F2 が送信されて UX が悪化する。

**理想:** TSF 側から composition context の有効性をイベントで通知してもらう仕組みが必要だが、WezTerm が IME_END 等を発行しない（BUG-04）ため現状は固定閾値に頼っている。

---

## BUG-06: EAGER_SETTLE_MS = 500ms が全環境で十分かどうか不明

**症状:** 特定の環境（低スペック PC、起動直後の WezTerm、重負荷時）で 500ms を超える TSF 初期化時間が発生した場合、eager warmup ありでも1文字目が文字化けする可能性がある。

**原因:** 実測値は ~305ms だが、保守的に 500ms としている。OS 環境やタスクスケジューラの状況によって変動する可能性がある。

**回避策:** なし。TSF 初期化完了を検出する API が存在しない（BUG-03・BUG-04）ため、閾値の調整以外に対処法がない。

---

## BUG-07: 物理 F2 と warmup F2 の二重送信による IME モード反転リスク

**症状:** WezTerm で物理 F2 を押した直後に NICOLA 文字を入力すると、IME モードが OFF に反転して英字入力になってしまう可能性がある。

**原因:** TSF モードでは awase が物理 F2 を Consume し（二重 F2 防止）、次の romaji バッチに warmup F2 を含める設計。しかし、物理 F2 を Consume する前に eager warmup F2 が既に送信されていた場合、実質的に F2 が2回 WezTerm に届いて IME が toggle（ON→OFF）する可能性がある。

**現在の対策:** `NativeF2Consumed` 時に `eager_warmup_sent_ms` を上書きしない（既存のタイムスタンプを維持）。

**残存リスク:** レースコンディション（物理 F2 処理と eager warmup F2 のタイミングのずれ）が完全に排除できているか不明。

---

## BUG-08: focus_epoch のオーバーフロー

**症状:** 理論上の問題。u32::MAX 回ウィンドウ切り替えを行うと `focus_epoch` がオーバーフローして 1 に戻る（0 は cold の番兵値のためスキップ）。このタイミングで前のウィンドウの `composition_warm_epoch` と一致した場合、stale な warm 状態が有効と誤判定される。

**原因:** `output.rs:on_focus_changed()` で `focus_epoch.wrapping_add(1).max(1)` を使用。

**実用上の影響:** u32::MAX ≈ 42億回の切り替えが必要なため、実用上は発生しない。

---

## BUG-09: SetOpenTrue / SymbolVkSent / SessionExpired で eager warmup なし

**症状:** 以下の cold トリガーでは eager warmup が行われず、F2 + 40ms sleep のみ。高速タイピストには不足する可能性がある。

- `SetOpenTrue`: IME OFF→ON 切り替え直後（半角/全角キー）
- `SymbolVkSent`: 記号を VK で送信した後（TSF context がリセットされる可能性）
- `SessionExpired`: 2000ms 沈黙後の再入力

**原因:** これらのタイミングでは eager warmup の送信タイミングと次打鍵のタイミングの関係が不定であり、FocusChange / Enter 後とは異なる扱いになっている。

**影響度:** 中程度。`SetOpenTrue` は IME toggle 直後なので次打鍵まで自然な間隔があることが多い。`SymbolVkSent` は記号の直後に日本語を打つケース。

---

## デバッグ方法

ログ出力（`RUST_LOG=debug` または awase の設定でデバッグログを有効化）で以下のキーワードを確認する:

| ログキーワード | 意味 |
|---|---|
| `[tsf-warmup]` | TSF cold-start warmup の送受信 |
| `[vk-warmup]` | Chrome VK cold-start warmup |
| `[h1-warmup]` | TSF 固定 sleep warmup の詳細 |
| `[h1-probe]` | Chrome VK probe ループの詳細 |
| `[composition]` | warm/cold マーク変更 |
| `[tsf-eager-warmup]` | eager warmup F2 の送信 |
| `[ime-event]` | WinEvent IME_START/CHANGE/END |
| `session expired` | session timeout による強制 warmup |
| `cold=N` | cold-start 発生回数（セッション識別用） |
