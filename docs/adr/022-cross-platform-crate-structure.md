# ADR-022: クロスプラットフォームのクレート構造

## ステータス

採用

## コンテキスト

awase を Windows だけでなく macOS と Linux でも動作させるため、コードベースを複数クレートに分割する必要がある。Engine のロジックはプラットフォーム非依存だが、キーボードフック、キー出力、IME 検出、トレイアイコン、イベントループは OS ごとに異なる。

## 決定

### ワークスペース構造

```
awase/
├── Cargo.toml                    # workspace root
├── src/                          # awase (lib) — プラットフォーム非依存
│   ├── engine/                   # Engine, NicolaFsm, InputTracker, KeyLifecycle
│   ├── config.rs                 # AppConfig, GeneralConfig
│   ├── types.rs                  # RawKeyEvent, KeyAction, SpecialKey, ModifierKey 等
│   ├── scanmap.rs                # PhysicalPos, KeyboardModel
│   ├── yab/                      # .yab レイアウトパーサー
│   ├── kana_table.rs             # ローマ字→かな変換
│   ├── ngram.rs                  # n-gram モデル
│   └── platform.rs               # PlatformRuntime トレイト定義
├── crates/
│   ├── timed-fsm/                # 汎用 FSM フレームワーク
│   ├── awase-windows/            # Windows 実装
│   │   └── src/
│   │       ├── hook.rs           # WH_KEYBOARD_LL + classify_key
│   │       ├── output.rs         # SendInput
│   │       ├── executor.rs       # Effect 実行（2エントリポイント）
│   │       ├── vk.rs             # VK 定数, vk_name_to_code, parse_key_combo
│   │       ├── scanmap.rs        # Windows Set 1 scan_to_pos
│   │       ├── ime.rs            # TSF + IMM32
│   │       ├── tray.rs           # Shell_NotifyIconW
│   │       ├── gui/main.rs       # 設定 GUI (eframe)
│   │       └── main.rs           # エントリポイント
│   ├── awase-macos/              # macOS 実装
│   │   └── src/
│   │       ├── hook.rs           # CGEventTap + classify_key
│   │       ├── output.rs         # CGEventPost
│   │       ├── scanmap.rs        # macOS keycode → PhysicalPos
│   │       ├── vk.rs             # key_name_to_keycode
│   │       ├── ime.rs            # TISCopyCurrentKeyboardInputSource (スタブ)
│   │       ├── tray.rs           # NSStatusBar (スタブ)
│   │       └── main.rs           # エントリポイント
│   └── awase-linux/              # Linux 実装
│       └── src/
│           ├── hook.rs           # evdev + EVIOCGRAB + classify_key
│           ├── output.rs         # uinput
│           ├── scanmap.rs        # evdev keycode → PhysicalPos
│           ├── vk.rs             # key_name_to_evdev
│           ├── event_loop.rs     # epoll + timerfd
│           ├── ime.rs            # IBus/Fcitx D-Bus (スタブ)
│           ├── tray.rs           # StatusNotifierItem (スタブ)
│           ├── x11.rs            # XRecord バックエンド (スタブ)
│           ├── libinput.rs       # libinput バックエンド (スタブ)
│           └── main.rs           # エントリポイント
└── dist/linux/                   # systemd service, install.sh
```

### 新プラットフォーム追加パターン

新しい OS をサポートするには:

1. `crates/awase-{os}/` クレートを作成
2. `classify_key()`, `classify_modifier()`, `classify_ime_relevance()` を実装（OS キーコード → `RawKeyEvent` の分類フィールド）
3. `key_name_to_code()` を実装（config キー名 → OS キーコード）
4. `keycode_to_pos()` を実装（OS キーコード → `PhysicalPos`）
5. `main.rs` で Engine を構築し、OS のイベントループから `engine.on_input()` を呼ぶ

Engine 自体の変更は不要。

### KeyboardModel

JIS/US キーボードの行サイズ（各行のキー数）を `KeyboardModel` enum で定義。`.yab` パーサーが受け取る。config.toml の `keyboard_model = "jis"` で指定。

## 結果

- lib クレートは全プラットフォームで共通（プラットフォーム依存コードゼロ）
- 各プラットフォームクレートは独立してビルド・テスト可能
- Windows: 完全実装（本番動作中）
- Linux: evdev 入力 + uinput 出力 + epoll ループが動作。IME/トレイはスタブ
- macOS: 構造・マッピング・エントリポイント完成。OS API 呼び出しはスタブ
