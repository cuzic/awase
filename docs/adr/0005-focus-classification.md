# ADR 0005: フォーカス判定と AppKind 設計

**Status:** 安定（2026-05-19 現在）  
**関連コミット:** `49eed58`, `b44f954`, `665986e`, `9641029`, `f4ad994`, `00f5711`, `d6442c3`, `a4b34be`, `5747ecc`, `e1babb4`, `ce0dd02`, `41dabe1`

---

## Context

awase はフォーカス中のウィンドウが「テキスト入力を受け付けるか」を判定し、
受け付けない（ゲーム、ブラウザのアドレスバー以外の領域等）場合は
エンジンを無効化して誤変換を防ぐ必要がある。
また、アプリの種類（Win32 / Chrome / UWP）によって出力戦略が変わる（ADR 0004）。

---

## フォーカス判定の設計変遷

### Phase 1: クラス名ヒューリスティック（`49eed58` 2026-03-29）

既知のテキスト入力クラス名（`Edit`, `RichEdit`, `ConsoleWindowClass` 等）と
非テキストクラス名（`Button`, `Static` 等）を静的リストで判定。

### Phase 2: MSAA Role 判定（`b44f954` 2026-03-29）

`IAccessible::get_accRole` で `ROLE_SYSTEM_TEXT`、`ROLE_SYSTEM_COMBOBOX` 等を確認。
クラス名リストに載っていない Web コンポーネント等をカバー。

### Phase 3: UIA 非同期検出（`665986e` 2026-03-29）

UI Automation の `TextPattern` でテキスト入力コントロールを検出。
ワーカースレッドで非同期実行し、結果を学習キャッシュに反映。

### Phase 4: 学習キャッシュ（`9641029`/`f4ad994` 2026-03-29）

クラス名ごとに過去の判定結果を TTL 付きでキャッシュ。
`DetectionSource`（クラス名 / MSAA / UIA）によって TTL を変える。

### Phase 5: AppKind と IMM ブラックリスト（`d6442c3` 2026-04-10）

Chrome 系の IMM32 ブリッジが壊れているクラスを `IMM_BRIDGE_BROKEN_CLASSES` として列挙。

```rust
pub const IMM_BRIDGE_BROKEN_CLASSES: &[&str] = &[
    "Chrome_RenderWidgetHostHWND",
    "Chrome_WidgetWin_0",
    "Chrome_WidgetWin_1",
    "Windows.UI.Core.CoreWindow",
    "ApplicationFrameWindow",
    "PseudoConsoleWindow",
    "CASCADIA_HOSTING_WINDOW_CLASS",
];
```

### Phase 6: IMM capability キャッシュ学習（`a4b34be`/`5747ecc` 2026-04-12）

`IMM_BRIDGE_BROKEN_CLASSES` の静的リストに加えて、実際に `ImmGet*` を試みて
成功/失敗を `imm_cache.toml` に永続化。未知のアプリを自動学習。

### Phase 7: TSF ネイティブウィンドウの識別分離（`ce0dd02` 2026-05-19）

`is_tsf_native_window()` を `IMM_BRIDGE_BROKEN_CLASSES` から独立して定義。

```rust
fn is_tsf_native_window(class: &str) -> bool {
    matches!(
        class,
        "Windows.UI.Core.CoreWindow"
            | "XamlExplorerHostIslandWindow"
            | "Windows.UI.Input.InputSite.WindowClass"
            | "CASCADIA_HOSTING_WINDOW_CLASS"
    )
}
```

**IMM_BRIDGE_BROKEN との違い:**
- `IMM_BRIDGE_BROKEN`: WM_IME_CONTROL が不安定（Chrome 含む）
- `is_tsf_native`: IMM32 を一切使わず TSF で直接処理（Chrome は含まない）

この区別が Windows Terminal での engine 誤 deactivate と
force-IME-ON 誤発火を解消する鍵となった。

---

## AppKind の設計

```rust
enum AppKind {
    Win32,   // 通常の Win32 アプリ → Unicode 注入
    Chrome,  // Chromium 系 → VK Batched 注入
    Uwp,     // UWP/WinUI → Unicode 注入
}
```

`AppKind` は `detect_app_kind(class_name)` で同期的に判定。
`Chrome_*` クラスなら `Chrome`、`ApplicationFrameWindow` なら `Uwp`、それ以外は `Win32`。

---

## 判定階層（現在）

```
classify_focus(hwnd):
  1. hwnd == NULL → NonText
  2. WS_EX_NOIME スタイル → NonText
  3. 既知テキストクラス名（Edit, RichEdit 等）→ TextInput
  4. 既知非テキストクラス名（Button, Static 等）→ NonText
  5. MSAA role → TextInput / NonText / Undetermined

detect_app_kind(class_name):
  Chrome_* → AppKind::Chrome
  ApplicationFrameWindow → AppKind::Uwp
  otherwise → AppKind::Win32
```

IME 検出パスでの TSF-native 判定は `is_tsf_native_window()` で別途実施。

---

## もぐらたたき証跡

| コミット | 変更 |
|---------|------|
| `5a7ee86` | XAML インフラのフォーカスイベントを Windows 11 で無視 |
| `57c0e06` | auto-IME-OFF 削除（通常のウィンドウ切替を壊した） |
| `ad34c17` | スタートメニュー・Windows Search 自動バイパス |
| `a62551b` | フォーカス遷移中に NonText ウィンドウでエンジン bypass |
| `ce0dd02` | CASCADIA_HOSTING_WINDOW_CLASS を TSF-native ガードに追加 |

---

## Consequences

**良い点:**
- 多段階フォールバックにより未知のアプリでも概ね正しく判定できる
- 学習キャッシュにより繰り返しの UIA 問い合わせを回避

**トレードオフ:**
- 判定が誤った場合にユーザーが `force_text` / `force_bypass` で手動修正が必要
- TSF-native / IMM-broken の区別が必要なクラスが増えると静的リストのメンテが必要
