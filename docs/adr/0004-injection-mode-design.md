# ADR 0004: InjectionMode 三分岐設計

**Status:** 安定（2026-05-19 現在）  
**関連コミット:** `3e444b4`, `50c5b4b`, `a444984`, `73b69ebb`, `73ddbb8`, `41d8fee`, `a57fc81`, `281cebd`

---

## Context

日本語入力（ひらがな→漢字変換）のキー注入方法はアプリごとに異なる。
普通の Win32 アプリ、Chrome 系、WezTerm/TSF ネイティブの3種類で
それぞれ最適な注入戦略が異なる。

---

## 選択肢と採用理由

### Unicode 直接注入（Win32 / UWP デフォルト）

`KEYEVENTF_UNICODE` で文字コードを直接送信。IME を経由しない。
ひらがな文字そのものを unicode として注入するため確実。

**採用ケース:** 通常の Win32 アプリ、UWP アプリ。

### VK Batched 注入（Chrome / Edge / Electron）

ローマ字を VK キーストロークとして送信。Chrome 内部の IME パイプラインを通す。
送信順序: **全 KeyDown → 全 KeyUp（重畳順）**

**採用理由:** Chrome の IME は WM_KEYUP でコンポジションをコミットするため、
Sequential（M↓M↑ U↓U↑）では M↑ 到達時点で IME が 'm' を単独確定してしまう。
Overlapped（M↓U↓ M↑U↑）なら一組として処理される。

### VK Sequential 注入（WezTerm / TSF ネイティブアプリ）

ローマ字を VK キーストロークとして送信。送信順序: **M↓M↑ U↓U↑（逐次順）**

**採用理由:** WezTerm の TSF 実装は Overlapped 順では "mう" になる（`73ddbb8` 2026-04-29）。
後に Batched と同じ順序に統一（`41d8fee` 2026-04-30）した経緯あり。

---

## 決定の変遷

### 初期（`3e444b4` 2026-04-01）

`Chrome → VK`、`それ以外 → Unicode` のシンプルな二分岐。

### WezTerm の分類変更を巡る試行錯誤（★★★）

```
WezTerm: Unicode → force_vk=true で Chrome 扱い (a57fc81 2026-04-21)
  → force_vk を削除し Unicode 注入へ移行 (3b69ebb 2026-04-29)
  → TSF Sequential モード追加 (73ddbb8 2026-04-29)
  → TSF Batched（Chrome と同じ重畳順）に変更 (41d8fee 2026-04-30)
```

WezTerm は IMM32 ブリッジを持つが TSF で IME を処理するため、
Unicode 直接注入では変換できない。かといって Chrome と全く同じ挙動でもない。
結果として `Tsf` という専用モードが生まれた。

### 現在の三分岐（`281cebd` 2026-04-01 〜）

```rust
enum InjectionMode {
    Unicode,  // Win32/UWP デフォルト
    Vk,       // Chrome/Edge/Electron — IME composition 経由
    Tsf,      // WezTerm — TSF 直結アプリ向け
}

fn resolve_injection_mode() -> InjectionMode {
    // 1. config の focus_overrides.force_tsf → Tsf
    // 2. config の focus_overrides.force_vk  → Vk
    // 3. AppKind::Chrome                     → Vk
    // 4. それ以外                            → Unicode
}
```

`force_tsf` / `force_vk` config オーバーライドにより、
誤検出アプリをユーザーが手動で修正できる。

---

## Consequences

**良い点:**
- アプリごとの IME パイプラインの違いを吸収する適切な抽象
- config override により edge case をユーザーが修正可能

**トレードオフ:**
- AppKind の自動検出が誤った場合に出力が壊れる
- Tsf モードと Vk モードの境界が明確ではなく、新しいアプリで分類が不確か
