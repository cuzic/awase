# ADR 0002: TSF cold-start warmup 戦略

**Status:** 安定（2026-05-19 現在）  
**関連コミット:** `1703fcf`, `4a1cbca`, `babce4c`, `99f56a2`, `2d4d85c`, `7ad60ce`, `b257f96`, `3034bc8`, `558c39f`, `b643bac`, `2b7c9be`, `51018e4`, `48d25f2`, `62ea4f6`, `a56e223`, `07fc40d`, `aea5a25`, `83d5707`, `4249846`, `dbda95f`, `b23c50f`, `c7dc3f2`, `b99053a`, `e7a8bc5`, `f2a36bf`

---

## Context

WezTerm 等の TSF ネイティブアプリでは、Enter 等の確定操作後や
フォーカス切替後に IME composition コンテキストが初期化（cold）される。
この状態で最初のローマ字キーを送ると IME が初期化される前に処理され、
ASCII として出力される（例: 「これで」→「koれで」、「計算」→「keいさん」）。

---

## 決定の変遷

### Phase 1: VK_DBE_HIRAGANA ウォームアップ導入（`1703fcf` 2026-05-04）

F2 (VK_DBE_HIRAGANA) を先行送信することで IME 初期化を促す。
最初は常時送信、後にコールドスタート時のみに限定（`4a1cbca` 2026-05-04）。

### Phase 2: 別 SendInput に分離（`babce4c` 2026-05-05）

F2 とローマ字を同一バッチで送ると IME 初期化前にローマ字が処理されるため、
F2 を独立した `SendInput` 呼び出しにする。

### Phase 3: KEYEVENTF_SCANCODE の試行錯誤（`99f56a2`/`2d4d85c` 2026-04-30）

`KEYEVENTF_SCANCODE` を付加することで WezTerm の認識改善を試みるが
その後不要と判断して除去。

### Phase 4: 固定 sleep → WaitForInputIdle → probe-and-retry のループ

```
固定 sleep 40ms (3034bc8 05-15)
  → WaitForInputIdle (fb824bd 05-15) → WaitForInputIdle + フォールバック (7fcf60f)
  → WaitForInputIdle 廃止 + 固定 40ms (fb824bd)
  → 固定 sleep 300ms → 400ms (e52b235, 651776e 05-15)
  → probe-and-retry (558c39f 05-15)
  → 固定 sleep に戻す (b643bac 05-15)
```

`WaitForInputIdle` は現在の実装では PROCESS_QUERY_INFORMATION が必要で
失敗することがわかり廃止（`fb824bd` 2026-05-15）。

### Phase 5: EAGER_SETTLE_MS のチューニング地獄（★★★）

```
300ms → 400ms → 500ms → 600ms → 1500ms → 500ms → ColdReason 動的決定
```

| コミット | 変更 | トリガー |
|---------|------|---------|
| `e52b235` | 40ms → 300ms | |
| `651776e` | 300ms → 400ms | |
| `aea5a25` | 500ms → 600ms | kおれ バグ |
| `83d5707` | 600ms → 1500ms | kおのぎょ バグ |
| `4249846` | 1500ms → 500ms | adaptive warmup 実装前の基準値に戻す |
| `dbda95f` | ColdReason 動的決定 | NativeF2Consumed=1000ms, others=500ms |

固定値チューニングは本質的な解ではないと認識されていたが、
イベント駆動への移行が完了するまでの暫定措置として機能した。

### Phase 6: VK_IME_ON 実験（`48d25f2` 2026-05-17）

F2 (VK_DBE_HIRAGANA) の代わりに VK_IME_ON (0x16) を warmup キーとして試行。
その後 VK_DBE_HIRAGANA に戻す（`3d49109` 2026-05-18）。

### Phase 7: GJI I/O モニターによる TSF readiness 観測（`b23c50f` 2026-05-18）

Google Japanese Input のプロセス間通信ファイル（`session.ipc`）の
アクセス時刻監視により TSF readiness を観測するシステムを追加。
2フェーズアルゴリズム（`c7dc3f2`）で精度を上げるが、
GJI 以外の IME では機能しないため補助的な位置づけ。

### Phase 8: イベント駆動 reactive 待機（`b99053a`/`e7a8bc5` 2026-05-19）

固定 sleep から MsgWaitForMultipleObjects + WinEvent `OBJ_NAMECHANGE` による
reactive 待機に移行。TSF アプリが F2 を処理すると composition ウィンドウの
名前が変わるため、この WinEvent をトリガーとして実際の初期化完了を検出。

WM_NULL ACK（`e7a8bc5`）を組み合わせて入力キューの排出を確認。

### Phase 9: WM_NULL ループの試行と即日 revert（`d4beb1c`/`00876d2` 2026-05-18）

FocusChange 直後に WM_NULL ループでキュー渋滞を検出してから VK_IME_ON を送信する案を
実装したが「効果なし・むしろ悪化」として同日に revert。

---

## 現在の設計

```
cold start 検出:
  is_composition_warm() == false
  OR session_expired (前回送信から 2000ms 超過)

warmup シーケンス（TSF モード）:
  1. set_ime_romaji_mode() — IMM32 経由で同期的にローマ字モード設定
  2. send_eager_tsf_warmup() — F2 (VK_DBE_HIRAGANA) を SendInput
  3. settle 待機 — MsgWaitForMultipleObjects + OBJ_NAMECHANGE reactive
     (NativeF2Consumed: 1000ms, others: 500ms 上限)
  4. ローマ字バッチ送信
```

---

## Consequences

**良い点:**
- イベント駆動により最小限の待機で composition 初期化完了を検出できる
- ColdReason ごとに settle 時間を調整できる柔軟性がある

**残課題:**
- GJI 以外の IME（MS-IME 等）では OBJ_NAMECHANGE が発火しない場合がある
- フォーカス変更直後の pre-warmup タイミングは依然として経験則に依存している
