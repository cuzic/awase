# ADR-034: GJI Direct Strategy — Google 日本語入力との協調設計

## ステータス

採用済み（2026-05-24）、TsfNative 拡張（2026-06-07）、VK_IME_ON/OFF 移行（2026-06-28）

## コンテキスト

Chrome/Edge では `ImmSetOpenStatus` が無効で `VK_KANJI` によるトグルしか機能しない。しかし VK_KANJI は **トグル** キーのため：

- 現在の IME 状態を正確に把握していないと二重トグルが発生する
- shadow_model との乖離が起きると逆効果になる（OFF にしたいのに ON になる）

加えて、Ctrl+Shift+Delete（旧 GJI ショートカット）はブラウザの DevTools ショートカットと衝突し、Ctrl+Shift+M, F22 等の代替を探す試行錯誤が必要だった。

## 決定

Google 日本語入力（GJI）インストール済み環境では、Windows 標準の冪等キーを使って IME を直接制御する。

```
awase 実行時（GJI 環境、全プロファイル共通）
  → IME ON: SendInput(VK_IME_ON=0x16)   // GJI ひらがなへ（冪等）
  → IME OFF: SendInput(VK_IME_OFF=0x1A) // GJI IME-OFF（冪等）
```

GJI 未導入環境では `KanjiToggle` 戦略（VK_KANJI + shadow チェック）にフォールバックする。

### なぜ VK_IME_ON/OFF か

- VK_IME_ON (0x16) / VK_IME_OFF (0x1A) は Windows 標準の冪等キーで、GJI がネイティブに処理する
- config1.db へのパッチ不要（初回セットアップ不要）
- Chrome / WezTerm / Windows Terminal すべてで動作確認済み（2026-06-28）
- IMM32 シム経由（SendMessageTimeout）は Chrome で失敗・タイムアウトが多く不安定

### 旧実装: config1.db パッチ方式（廃止）

旧実装では `awase-gji-setup` ユーティリティで GJI の `config1.db` に
F21→IMEOn / F22→IMEOff エントリを書き込んでいた。
VK_IME_ON/OFF が Chrome・WezTerm で動作することを実機確認し、2026-06-28 に削除した。

## 結果

- Chrome/Edge での IME ON/OFF が安定（shadow desync による逆トグルが解消）
- config1.db パッチ不要でインストール後即使用可能
- GJI 未導入ユーザーも KanjiToggle フォールバックで動作

---

## 拡張: TsfNative アプリへの適用（2026-06-07 / 2026-06-28 確認）

### 経緯

当初 `GjiDirectStrategy` は TsfNative プロファイル（WezTerm / Windows Terminal）を除外していた。
理由は「F13/F14 を SendInput すると VT ターミナルが ESC[25~/ESC[26~ として解釈し、
ターミナルに `~` が入力されてしまう」ためである。

F21/F22 への移行（ADR-057）でターミナル漏れ問題は解消し、2026-06-07 に TsfNative 除外を撤廃した。
さらに 2026-06-28 に VK_IME_ON/OFF へ移行。VK_IME_ON/OFF は Windows 標準キーのため
config1.db 設定なしで WezTerm / Windows Terminal でも動作することを実機確認した。

### 設計変更（2026-06-28 最終状態）

`GjiDirectStrategy::is_applicable()` は TsfNative を除外しない（GJI 検出のみで判定）。

```rust
// 現在の実装
view.observed.active_ime_kind == ActiveImeKind::GoogleJapaneseInput
    && !matches!(view.focus.profile, AppImeProfile::TsfNative)  // ← この除外を削除済み

// → 現在
view.observed.active_ime_kind == ActiveImeKind::GoogleJapaneseInput
```

### 検証結果

| アプリ | キー | 結果 |
|--------|------|------|
| WezTerm (org.wezfurlong.wezterm) | VK_IME_ON/OFF | ✅ 完璧。Ctrl+無変換で IME OFF、再押しで ON に戻らない |
| Windows Terminal | VK_IME_ON/OFF | ✅ 正常動作 |
| Chrome / Brave / Edge | VK_IME_ON/OFF | ✅ 正常動作（2026-06-28 確認） |

## 関連 ADR

- [ADR-0003](0003-chrome-vk-injection.md) — Chrome VK injection
- [ADR-033](033-app-ime-profile.md) — AppImeProfile
- [ADR-044](044-applied-ime-state-confidence.md) — AppliedImeState（300ms ウィンドウ）
