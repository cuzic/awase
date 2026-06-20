# ADR-034: GJI Direct Strategy — Google 日本語入力との協調設計

## ステータス

採用済み（2026-05-24）、TsfNative 拡張（2026-06-07）

## コンテキスト

Chrome/Edge では `ImmSetOpenStatus` が無効で `VK_KANJI` によるトグルしか機能しない。しかし VK_KANJI は **トグル** キーのため：

- 現在の IME 状態を正確に把握していないと二重トグルが発生する
- shadow_model との乖離が起きると逆効果になる（OFF にしたいのに ON になる）

加えて、Ctrl+Shift+Delete（旧 GJI ショートカット）はブラウザの DevTools ショートカットと衝突し、Ctrl+Shift+M, F22 等の代替を探す試行錯誤が必要だった。

## 決定

Google 日本語入力（GJI）インストール済み環境では、`awase-gji-setup` ユーティリティで GJI の設定ファイル（`config1.db`）に「F22 → IME オフ」「F21 → IME オン」エントリを冪等パッチし、awase はこの既知ショートカットで IME を制御する。

```
awase-gji-setup（初回セットアップ）
  → config1.db に F21/F22 エントリを追加（既存エントリがあればスキップ）

awase 実行時（Chrome/Edge フォーカス + GJI 環境）
  → IME ON: SendInput(F21)
  → IME OFF: SendInput(F22)
```

GJI 未導入環境では `KanjiToggle` 戦略（VK_KANJI + shadow チェック）にフォールバックする。

### なぜ config1.db パッチか

- GJI は独自ショートカット定義を SQLite ベースの `config1.db` で管理する
- F21/F22 は実キーボードに存在せず他ショートカットと衝突しない
- IMM32 シム経由（SendMessageTimeout）は Chrome で失敗・タイムアウトが多く不安定

## 結果

- Chrome/Edge での IME ON/OFF が安定（shadow desync による逆トグルが解消）
- `awase-gji-setup` binary により初期セットアップが 1 コマンドで完結
- GJI 未導入ユーザーも KanjiToggle フォールバックで動作

---

## 拡張: TsfNative アプリへの適用（2026-06-07）

### 経緯

当初 `GjiDirectStrategy` は TsfNative プロファイル（WezTerm / Windows Terminal）を除外していた。
理由は「F13/F14 を SendInput すると VT ターミナルが ESC[25~/ESC[26~ として解釈し、
ターミナルに `~` が入力されてしまう」ためである。

その後 F21/F22（VK=0x84/0x85）に移行した。F21/F22 は実キーボードに存在せず
VT ターミナルもエスケープシーケンスを生成しないため、上記の問題は根本解決された。
また WezTerm 側で実機検証したところ、
**GJI の TSF text service (ITfKeyEventSink) が F22 を消費し IME OFF を実現できる**ことが確認された。

### メカニズム（推定）

TsfNative アプリは TSF を直接使う。GJI は TSF text service として登録されており、
WezTerm の ITfKeystrokeMgr 経由で F22 のキーイベントを受け取る。
GJI の `ITfKeyEventSink::OnKeyDown(F22)` が config1.db の IMEOff エントリを参照して
IME OFF を実行し `TRUE`（消費済み）を返す。これにより WezTerm の WndProc には F22 が届かない。

F21/F22 は VT エスケープシーケンスを持たないため、アプリ側での Nop バインド設定は不要。

### 設計変更

`GjiDirectStrategy::is_applicable()` から TsfNative 除外条件を削除した。

```rust
// Before
view.observed.gji_monitor_ok
    && !matches!(view.focus.profile, AppImeProfile::TsfNative)

// After
view.observed.gji_monitor_ok
```

GJI が TSF 層で F22 を消費しない場合は `Applied` を返すが IME は変わらない。
KanjiToggle への自動フォールスルーは起きないため、動作不良となる可能性がある。
その場合は TsfNative 除外に戻すこと。

### 検証結果

| アプリ | 結果 |
|--------|------|
| WezTerm (org.wezfurlong.wezterm) | ✅ 完璧。Ctrl+無変換で IME OFF、再押しで ON に戻らない |
| Windows Terminal (Windows.UI.Input.InputSite.WindowClass) | ✅ 正常動作 |

## 関連 ADR

- [ADR-0003](0003-chrome-vk-injection.md) — Chrome VK injection
- [ADR-033](033-app-ime-profile.md) — AppImeProfile
- [ADR-044](044-applied-ime-state-confidence.md) — AppliedImeState（300ms ウィンドウ）
