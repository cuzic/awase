# ADR 0001: IME 状態検出戦略

**Status:** 安定（2026-05-19 現在）  
**関連コミット:** `09ee3a9`, `58b13b2`, `164acd9`, `e1babb4`, `c8b8b31`, `cf1aeeb`, `82ab4e7`, `ce0dd02`, `41dabe1`

---

## Context

Windows で IME の ON/OFF 状態を外部プロセスから検出する方法は複数存在し、
それぞれ信頼性・レイテンシ・副作用が異なる。awase はキーボードフックから
リアルタイムで IME 状態を知る必要がある。

---

## 決定の変遷

### Phase 1: TSF + IMM32 ハイブリッド（`09ee3a9` 2026-03-28）

`ImeDetector` trait に `TsfProvider`、`ImmProvider`、`HybridProvider` を実装。
TSF の `ITfInputProcessorProfiles` で言語プロファイルを取得し、
IMM32 の `ImmGetOpenStatus` でオープン状態を取得する二重構造。

**問題点:** TSF CompartmentEventSink はクロスプロセスの変化に発火しなかった
（`d0218e7` 2026-04-10 で廃止）。

### Phase 2: クロスプロセス IMM32 検出（`58b13b2` 2026-03-29）

`GetGUIThreadInfo().hwndFocus` でフォーカスウィンドウを特定し、
`ImmGetDefaultIMEWnd` + `SendMessageTimeoutW(WM_IME_CONTROL / ICM_GETOPENSTATUS)` で
IME 状態をクロスプロセスで取得。

`GetForegroundWindow()` ではなく `hwndFocus`（子ウィンドウ）を使うことで
Zoom 等のマルチウィンドウアプリでも正確に動作（`2cb33ba` 2026-04-03）。

### Phase 3: Shadow State + SSOT 設計（`c6db222` 2026-04-10）

検出失敗時に「awase の知る最後の状態」をシャドウとして保持し、
OS 側に書き戻す（SSOT: Single Source of Truth）方式を採用。

検出が連続失敗した場合に engine を deactivate する案（`85c8898` 2026-04-09）は
「force-IME-ON に変更すべき」として翻された（`6531a83` 2026-04-09）。

### Phase 4: 3値意味論の導入（`e1babb4` 2026-04-24）

`ImeSnapshot` の各フィールドを `bool` から `Option<bool>` に変更。

```
Some(v) = 検出成功 → preconditions を更新する
None    = 不明（タイムアウト等） → 前回キャッシュ値を維持する
```

`None` を `false` として扱うコードパスが構造的に排除された。
これが「検出不能 ≠ IME オフ」概念の礎となった。

### Phase 5: IMM capability キャッシュ学習（`a4b34be` 2026-04-12）

ウィンドウクラス名ごとに「IMM32 検出が成功したか」を学習・永続化
（`imm_cache.toml`）。Chrome 等の IMM ブリッジが壊れているクラスを
実績ベースで自動判定する。

### Phase 6: ImeObservations + resolve_and_clear（`82ab4e7` 2026-05-15）

観測（`observe()`）と判断（`resolve_and_clear()`）を分離。
`observer_poll` が `Some(v)` のときのみ `preconditions.ime_on` を更新し、
`None` は「前回値を維持」として扱う契約を型レベルで強制。

### Phase 7: TSF ネイティブウィンドウの構造的識別（`ce0dd02`/`41dabe1` 2026-05-19）

`is_tsf_native_window(class: &str) -> bool` を導入し、
`CASCADIA_HOSTING_WINDOW_CLASS`、`Windows.UI.Input.InputSite.WindowClass` 等を
「IMM32 を使わない（TSF ネイティブ）」として構造的に識別。

`ImeSnapshot.is_tsf_native: bool` フラグにより、
`ime_observer.rs` の `observe()` が miss_count を増やさずに済む。

---

## 現在の設計

```
detect_ime_state() → ImeSnapshot {
    is_japanese_ime: Option<bool>,
    ime_on: Option<bool>,
    is_romaji: Option<bool>,
    conversion_mode: Option<u32>,
    is_tsf_native: bool,   // 構造的に検出不能（miss_count 増加を防ぐ）
}
```

検出パス（優先順位順）:
1. `is_tsf_native_window(class)` → `is_tsf_native=true` で早期リターン
2. `ImmGetDefaultIMEWnd` → `SendMessageTimeoutW(ICM_GETOPENSTATUS)` (50ms)
3. タイムアウト → `None`（前回値を維持）

---

## もぐらたたき証跡

| コミット | 変更 |
|---------|------|
| `85c8898` | 検出失敗時 engine deactivate |
| `6531a83` | 直後に force-IME-ON に変更 |
| `cf1aeeb` | OS probe の deactivate 権限を剥奪 |
| `ce0dd02` | TSF-native ガードで Windows Terminal 誤 deactivate 解消 |
| `41dabe1` | miss_count 誤積算で force-IME-ON 誤発火も解消 |

---

## Consequences

**良い点:**
- 検出不能と検出失敗が型で区別され、誤った状態更新が構造的に防止される
- TSF-native ウィンドウで force-IME-ON が誤発火しなくなった

**トレードオフ:**
- 言語バーのマウス操作や IME ボタンクリックは検出できない（割り切り）
- Chrome は IMM32 シムが存在するが応答が非同期のため依然として不安定
