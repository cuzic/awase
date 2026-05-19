# ADR 0003: Chrome VK injection と F2 warmup

**Status:** 実験中（2026-05-19 現在、`SendMessageTimeout` 方式を試行）  
**関連コミット:** `cfa42b9`, `698f4fd`, `d250ece`, `a444984`, `b61cbf9`, `7ad60ce`, `a852b56`, `c2a6052`, `9027d70`, `0b55b8d`, `907c5ba`, `a3cce29`, `8144412`

---

## Context

Chrome / Edge / Electron 等 Chromium 系アプリは IME composition を
Blink レンダラー（サンドボックス化された子プロセス）で管理する。
Win32 の `Chrome_RenderWidgetHostHWND` はブラウザプロセスの薄い IMM32 シムであり、
レンダラーとの間に IPC ラウンドトリップが存在する。

Unicode 直接注入（`WM_CHAR`）では IME composition が経由されないため
ひらがな変換ができない。VK キーストロークとして注入し Chrome の IME パイプラインを
通す必要がある。

---

## 決定の変遷

### Phase 1: kana→romaji 逆引きテーブル（`cfa42b9` 2026-04-01）

かな文字を IME に通すためにローマ字に逆変換してから VK として送信する方針を採用。
逆引きテーブル（`romaji_to_kana`）を実装。

### Phase 2: VK 送信順序の試行錯誤（★★★）

VK の送信順序（Sequential vs Overlapped）を複数回切り替え。

```
Sequential (M↓M↑ U↓U↑) — 初期実装
  → Overlapped に変更 (c2a6052 2026-04-27): "mうけ" 問題対策
  → Sequential に revert (a3cce29 2026-04-28): WezTerm 向け変更が Chrome を壊した
  → Overlapped に再変更 (0b55b8d 2026-04-27): WezTerm TSF の "mうけ" 問題
  → Sequential に revert (907c5ba 2026-04-29): WezTerm WM_KEYUP コミット対策
```

**Overlapped（重畳順）の根拠:**
WM_KEYUP 受信時に IME がコンポジションをコミットするアプリ（Chrome 等）では、
Sequential だと M↑ 到達時点で IME が 'm' を単独確定し "mう" になる。
Overlapped（全 KeyDown → 全 KeyUp）では M↓U↓ を一組として受け取れる。

最終的に Chrome は Overlapped、WezTerm は TSF Sequential モードに分離（ADR 0004 参照）。

### Phase 3: Chrome VK コールドスタート F2 warmup（`b61cbf9` 2026-05-04、`a852b56` 2026-05-15）

TSF モードと同様に、Chrome でも Enter 後に IME composition が cold になる。
F2 (VK_DBE_HIRAGANA) を先行バッチとして送信し、probe ループで
Chrome が F2 を処理するまで待つ「案A」を実装。

```
SendInput(F2↓ F2↑)
probe loop (最大 15回 × 10ms = 150ms):
  SendMessageTimeout(WM_IME_CONTROL / ICM_GETCONVERSIONMODE, 10ms)
  responsive=true なら break
SendInput(K↓ A↓ K↓ I↓ K↑ A↑ K↑ I↑)
```

### Phase 4: probe 競合の発見と SendMessageTimeout による修正（`8144412` 2026-05-19）

**根本原因の特定:**
Windows メッセージ優先順位 `QS_SENDMESSAGE > QS_INPUT` により、
`SendInput(F2)` で積んだ入力キューイベントより
`SendMessageTimeout(probe)` が先に処理される。

Chrome が probe に応答した時点で F2 はまだ処理されていない可能性があり、
romaji バッチが先に届く → 先頭文字が raw ASCII になる（「かき → kあき」）。

**修正:**
`SendInput(F2)` を `SendMessageTimeout(WM_KEYDOWN, F2)` に変更。
`SendMessageTimeout` は Chrome の wndproc が WM_KEYDOWN を処理するまでブロックするため、
return 後は F2 が処理済みであることが保証される。

```rust
pub unsafe fn send_f2_via_sendmessage() -> bool {
    SendMessageTimeoutW(hwnd, WM_KEYDOWN, VK_DBE_HIRAGANA, lparam_down, SMTO_ABORTIFHUNG, 100, ...)
    SendMessageTimeoutW(hwnd, WM_KEYUP,   VK_DBE_HIRAGANA, lparam_up,   SMTO_ABORTIFHUNG, 100, ...)
}
```

**残課題:**
Chrome が合成 WM_KEYDOWN（`SendMessage` 経由）をリアル入力と同一視するかは
実際に試してみないと不明。`success=false` の場合は別の戦略が必要。

---

## 現在の設計

```
Chrome cold-start warmup:
  1. set_ime_romaji_mode() — IMM32 経由で同期設定
  2. send_f2_via_sendmessage() — SendMessageTimeout(WM_KEYDOWN, F2) で同期配送
  3. SendInput(overlapped romaji batch) — 全 KeyDown → 全 KeyUp
```

---

## Chrome IME の構造的な問題

```
[awase] → SendInput → [OS input queue]
                            ↓
                   [Chrome browser process (Win32 shim)]
                            ↓ IPC
                   [Chrome renderer (Blink IME)]
```

- IMM32 API は browser process の shim に届くが、実際の IME state は renderer が管理
- `ImmGetConversionStatus` の返値は renderer が前回 IPC で送った値（遅延あり）
- `WM_IME_CONTROL / ICM_SETOPENSTATUS` も確実に renderer に反映されるとは限らない

---

## Consequences

**良い点:**
- `SendMessageTimeout` による F2 配送は入力キューを経由しないため probe 競合を回避できる

**トレードオフ:**
- `SendMessage` 経由の WM_KEYDOWN を Chrome が IME キーとして処理するかは未確認
- Chrome の IME 状態は根本的に外部から確実に制御できる手段がない
