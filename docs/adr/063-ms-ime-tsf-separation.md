# ADR-063: TSF 共通層と IME 固有層の分離 + MS-IME 対応（案B）

## ステータス

採用済み（2026-06-27 実装、commit e36875d）

## コンテキスト

awase の TSF 制御は Google 日本語入力（GJI）専用設計になっており、
MS-IME 環境では IME ON/OFF が `VK_KANJI` トグルにフォールバックしていた。

`VK_KANJI` はトグルキーであり、shadow desync が発生すると意図と逆の操作になる問題がある。

ユーザーの観察通り、TSF アプリへの制御には以下の2層が存在する：

| 層 | GJI | MS-IME |
|---|---|---|
| **TSF 共通** | VK + ローマ字送信、NAMECHANGE 監視 | 同左 |
| **IME 固有** | VK_IME_ON/OFF による冪等制御（config1.db 不要） | VK_DBE_HIRAGANA / ALPHANUMERIC による冪等制御 |

cold-start probe の有無も IME 固有である：
- **GJI**: SacrificialWarmup (VK_A+BS) + WriteTransferCount 観測が必要
- **MS-IME**: 常に TSF context が warm、probe 不要

## 決定

TSF 共通層はそのまま維持し、IME 固有の ON/OFF 制御と warm/cold 判定を
既存の Strategy パターン（`ImeOpenStrategy` / `ImeWarmupStrategy`）で分岐させる（案B）。

### 変更点 1: `ActiveImeKind` 型 + 検出

```rust
pub(crate) enum ActiveImeKind {
    GoogleJapaneseInput,
    MicrosoftIme,  // GJI 非検出時の推定（冪等 VK_DBE_* を使う）
}
```

`gji_monitor_ok()` から派生（新たな AtomicU8 は不要）。
GJI モニタースレッドが検出状態変化時に `WM_IME_KIND_CHANGED` を post し、
メインスレッドが `Output::set_active_ime_kind()` を呼ぶ。

### 変更点 2: `MsImeDirectStrategy` 追加

```
戦略順序（旧）: ImmCross → GjiDirect → KanjiToggle
戦略順序（新）: ImmCross → GjiDirect → MsImeDirect → KanjiToggle
```

`MsImeDirectStrategy` が適用される条件:
- GJI 非検出（`active_ime_kind == MicrosoftIme`）
- IMM32 クロスプロセス制御が使えない TSF アプリ（Chrome / Edge 等）

`VK_DBE_HIRAGANA` (0xF2) = IME ON（冪等）  
`VK_DBE_ALPHANUMERIC` (0xF0) = IME OFF（冪等）

### 変更点 3: warmup strategy の動的切り替え

`Output.gji_fsm` → `Output.tsf_warmup`（GJI 固有でない命名に変更）

`set_active_ime_kind()`:
- MS-IME → `MsImeStrategy`（常に warm、probe なし）
- GJI → `GjiFsm`（cold probe 機構あり）

これにより MS-IME 環境では不要な SacrificialWarmup が走らなくなる。

## 検討した代替案

**案A: 全モードを一括置換**  
→ 採用しなかった。GJI の cold probe / SacrificialWarmup は複雑で、
  MS-IME 向けに完全別実装を書くよりも既存 Strategy 抽象を使い回すほうが安全。

**案C: KanjiToggle を改善するだけ**  
→ 採用しなかった。VK_KANJI はトグルキーのため shadow desync が構造的に発生する。
  冪等キー（VK_DBE_*）への切り替えが本質的な解決策。

## 変更ファイル

| ファイル | 変更内容 |
|---------|---------|
| `tsf/observer.rs` | `ActiveImeKind` enum、`active_ime_kind()` アクセサ、monitor_loop post |
| `state/ime_decision_view.rs` | `ObservedState.active_ime_kind` フィールド追加 |
| `ime.rs` | `post_ms_ime_on/off()` 追加 |
| `ime_controller.rs` | `MsImeDirectStrategy` 追加、`strategies` を 3→4 |
| `tsf/warmup_strategy.rs` | `MsImeStrategy` の `#[allow(dead_code)]` 撤去 |
| `output/mod.rs` | `gji_fsm` → `tsf_warmup` リネーム、`set_active_ime_kind()` 追加 |
| `lib.rs` | `WM_IME_KIND_CHANGED = WM_APP + 21` |
| `app/mod.rs` | `WM_IME_KIND_CHANGED` ハンドラ arm 追加 |
| `runtime/message_handlers.rs` | `handle_wm_ime_kind_changed()` |

## 先送りした事項（Phase 2）

- `Output` 内の GJI probe state フィールド群（`current_gji_probe_id` 等）は残す
  → MS-IME 時は使われないが害はない
- GJI probe state を `GjiFsm` 内部に移動する深い構造改善

## 関連 ADR

- ADR-047: ImeWarmupStrategy トレイト抽象化（本 ADR の基盤）
- ADR-034: GJI Direct Strategy 設計
- ADR-048: SacrificialWarmup（MS-IME 時は不要になった）
