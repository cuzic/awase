# ADR 025: TOML ベースのカスタマイズ設計

## ステータス

提案中

## コンテキスト

awase のカスタマイズ機能（配列定義、キーリマップ、コンボ定義、出力マクロ、アプリ別設定）を、どのような設定言語で実現するかを検討した。

### 検討した選択肢

| 選択肢 | メリット | デメリット |
|--------|---------|-----------|
| Lua (mlua) | 制御フロー（ループ・関数）で DRY に書ける、ユーザー層が広い | ビルド依存増、ほとんどのユーザーは使わない |
| Jsonnet | 静的データ生成に特化、テンプレート機能 | マイナー、ユーザー認知度が低い |
| TOML 拡張 | 学習コスト最低、既存の config.toml を拡張するだけ | ループ・関数がない、動的生成不可 |

### アーキテクチャ上の制約

awase には 2 つの実行コンテキストがあり、それぞれカスタマイズ可能な範囲が異なる。

```
フックコールバック（WH_KEYBOARD_LL）
  - 数百μs 以内に OS に応答が必要
  - スクリプト VM の呼び出し不可
  - 参照するデータは事前にロード済みのテーブルのみ

メッセージループ（GetMessageW）
  - 時間制約なし
  - スクリプト VM 呼び出し可能
  - フック参照テーブルの差し替え可能
```

この制約により、フック内の処理アルゴリズム自体はカスタマイズ不可だが、フックが参照するデータ（テーブル）はメッセージループのタイミングで自由に差し替えられる。

### イベントごとの処理場所とカスタマイズ可否

| イベント | 処理場所 | カスタマイズ |
|---------|---------|------------|
| キー打鍵 | フック | テーブル参照のみ（ロジック変更不可） |
| フォーカス変更 | メッセージループ | 自由 |
| タイマー発火 | メッセージループ | 自由 |
| IME 状態変化 | メッセージループ | 自由 |
| 遅延 Effect 実行 | メッセージループ | 自由（Effect の変換・フィルタ可能） |
| トレイ操作 | メッセージループ | 自由 |

### フックが参照するデータ（メッセージループで差し替え可能）

| データ | 例 |
|--------|-----|
| リマップテーブル | `CapsLock → Ctrl` |
| 配列テーブル | `.yab` 相当の打鍵→文字マッピング |
| 同時打鍵パラメータ | 閾値ms、判定方式 |
| Engine ON/OFF | 有効・無効フラグ |
| コンボ定義テーブル | `Ctrl+Muhenkan → ToggleEngine` |

### カスタマイズ可能な範囲の正確な区分

**できること:**

- アプリごとにリマップ・配列・閾値・コンボを丸ごと切り替える
- IME 状態に応じて設定を変える
- タイマーで一定時間後に設定を変える（アイドル検知等）
- 出力文字の変換・マクロ（遅延 Effect に介入、文脈依存の変換も可能）
- IME の ON/OFF 制御
- トレイメニューにカスタムコマンド追加

**できないこと:**

- 同時打鍵アルゴリズム自体の差し替え（パラメータ変更は可能）
- PassThrough/Consume の判定ロジック変更（フック内で確定済み）
- Filter モードでフック内即実行された Effect の取り消し

## 決定

### TOML + CSS 型カスケードモデルの採用

ユースケースの大半は宣言的なデータ定義でカバーできるため、TOML ベースのカスタマイズを導入する。条件の複合（AND/OR）を自然に表現するため、CSS のセレクタ/宣言分離に倣った **class/style モデル** を採用する。

### class/style モデルの設計思想

CSS と同様に「どの条件で適用するか（class）」と「何をするか（style）」を分離する。

| CSS | awase |
|-----|-------|
| セレクタ（`.chrome`） | `[class.chrome]` — マッチ条件の定義 |
| 宣言ブロック（`{ color: red; }`） | `[style.chrome]` — 適用する設定値 |
| 複合セレクタ（`.chrome.ime-off`） | `[style."chrome ime-off"]` — スペース区切りで AND |
| 詳細度 | class 数が多い style が優先 |
| カスケード | default → 一般 → 具体的 の順に上書き |

これにより、条件の定義と設定値の宣言が分離され、同じ class を複数の style で再利用でき、複合条件も宣言的に書ける。

## 設定ファイル仕様

### 全体構造

```toml
# ═══════════════════════════════════════════
#  デフォルト設定
# ═══════════════════════════════════════════

[layout]
file = "nicola.yab"

[timing]
threshold_ms = 80
method = "d1d2"            # "d1d2" | "window"
long_press_ms = 300

[remap]
CapsLock = "LCtrl"
Muhenkan = "LeftThumb"
Henkan = "RightThumb"

[[combo]]
keys = ["LCtrl", "Muhenkan"]
action = "ToggleEngine"

[[combo]]
keys = ["LCtrl", "Henkan"]
action = "ImeOn"

[[macro]]
trigger = "kk"
replace = "っk"

# ═══════════════════════════════════════════
#  class 定義（マッチ条件）
# ═══════════════════════════════════════════

[class.chrome]
app_kind = "Chrome"

[class.win32]
app_kind = "Win32"

[class.uwp]
app_kind = "Uwp"

[class.vscode]
app = "code*"
app_kind = "Chrome"

[class.gaming]
app = "steam*"

[class.ime-off]
ime = false

[class.ime-on]
ime = true

[class.kana-input]
input_method = "kana"

[class.romaji-input]
input_method = "romaji"

[class.jp-keyboard]
keyboard = "jp"

[class.us-keyboard]
keyboard = "us"

[class.msime]
ime_product = "msime"

[class.google-ime]
ime_product = "google"

# ═══════════════════════════════════════════
#  style 定義（条件付き設定の上書き）
# ═══════════════════════════════════════════

# Chrome 全般: 閾値を短く
[style.chrome]
timing.threshold_ms = 60

# VS Code: 専用配列
[style.vscode]
layout.file = "nicola-vscode.yab"

# Chrome かつ IME OFF: Engine を無効化
[style."chrome ime-off"]
engine = false

# IME OFF: 全アプリ共通で Engine OFF
[style.ime-off]
engine = false

# IME ON: Engine ON に復帰
[style.ime-on]
engine = true

# ゲーム: Engine 無効化
[style.gaming]
engine = false

# かな入力モード: パススルー（Engine が処理しない）
[style.kana-input]
engine = false

# US キーボード: 親指キーを別のキーに割り当て
[style.us-keyboard]
remap.LAlt = "LeftThumb"
remap.RAlt = "RightThumb"

# Google 日本語入力: 閾値を調整
[style.google-ime]
timing.threshold_ms = 70
```

### セクション詳細

#### `[layout]` — 配列定義

```toml
[layout]
file = "nicola.yab"        # .yab 配列ファイルパス
```

#### `[timing]` — 同時打鍵判定パラメータ

```toml
[timing]
threshold_ms = 80          # 同時打鍵判定閾値（ms）
method = "d1d2"            # 判定方式: "d1d2" | "window"
long_press_ms = 300        # 長押し判定閾値（ms）
```

#### `[remap]` — キーリマップ

物理キー → 物理キーの静的マッピング。Engine 処理前にフック内で適用される。

```toml
[remap]
CapsLock = "LCtrl"
Muhenkan = "LeftThumb"
Henkan = "RightThumb"
```

#### `[[combo]]` — コンボ定義

キーの組み合わせ → アクションのマッピング。

```toml
[[combo]]
keys = ["LCtrl", "Muhenkan"]
action = "ToggleEngine"
```

`action` は固定の列挙型:
- `ToggleEngine` — Engine の有効/無効を切り替え
- `ImeOn` — IME を ON にする
- `ImeOff` — IME を OFF にする
- `SwitchProfile` — プロファイルを切り替え

#### `[[macro]]` — 出力後処理

Engine が出力した文字列の末尾パターンをマッチし、置換する。メッセージループの Effect キュー処理時に適用される。

```toml
[[macro]]
trigger = "kk"             # 直近の出力文字列末尾がこれに一致したら
replace = "っk"            # BackSpace + 置換文字を出力
```

Rust 側はリングバッファで直近 N 文字（N = 最長 trigger 長）を保持し、trigger にマッチしたら BackSpace + 置換文字を出力する。

#### `[class.*]` — class 定義（マッチ条件）

条件に名前を付けて定義する。style から参照される。

```toml
[class.chrome]
app_kind = "Chrome"        # AppKind 列挙: "Win32" | "Chrome" | "Uwp"

[class.vscode]
app = "code*"              # プロセス名 glob
app_kind = "Chrome"        # 複数条件は AND

[class.ime-off]
ime = false                # IME の ON/OFF 状態
```

利用可能な条件:

| フィールド | 型 | 説明 | 検出タイミング |
|-----------|-----|------|--------------|
| `app` | glob パターン | プロセス名にマッチ | フォーカス変更時 |
| `app_kind` | `"Win32"` / `"Chrome"` / `"Uwp"` | AppKind 分類にマッチ | フォーカス変更時 |
| `ime` | `true` / `false` | IME の ON/OFF 状態にマッチ | IME 状態変化時 |
| `input_method` | `"romaji"` / `"kana"` | IME の入力方式（ローマ字/かな）にマッチ | IME 状態変化時 |
| `keyboard` | `"jp"` / `"us"` | 接続キーボードのレイアウトにマッチ | 起動時・デバイス変更時 |
| `ime_product` | `"msime"` / `"google"` / `"atok"` / glob | IME 製品名にマッチ | フォーカス変更時 |

同一 class 内の複数条件は AND。

##### 条件フィールドの検出方法

| フィールド | Windows API | 備考 |
|-----------|-------------|------|
| `app` | `GetModuleFileNameExW` | プロセス名（拡張子除く） |
| `app_kind` | UIA / クラス名分類 | 既存の `classify_app_kind` |
| `ime` | TSF / IMM32 | 既存の IME 状態検出 |
| `input_method` | `ImmGetConversionStatus` (IME_CMODE_ROMAN) | 既存の `detect_kana_input_method` |
| `keyboard` | `GetKeyboardLayout` | 下位 16bit の LANGID + サブタイプで JP/US 判定 |
| `ime_product` | `ImmGetDescription` / TSF プロファイル名 | IME の表示名から製品名を特定 |

#### `[style.*]` — style 定義（条件付き設定上書き）

class 名をキーにして、マッチ時に適用する設定を宣言する。

```toml
# 単一 class: class.chrome にマッチしたとき
[style.chrome]
timing.threshold_ms = 60

# 複合条件: class.chrome AND class.ime-off の両方にマッチしたとき
[style."chrome ime-off"]
engine = false
```

##### style で上書き可能なフィールド

| フィールド | 例 | 説明 |
|-----------|-----|------|
| `timing.*` | `timing.threshold_ms = 60` | 同時打鍵パラメータ |
| `layout.*` | `layout.file = "alt.yab"` | 配列ファイル |
| `remap.*` | `remap.F13 = "ToggleEngine"` | キーリマップの追加・上書き |
| `engine` | `engine = false` | Engine の有効/無効 |

##### 詳細度（Specificity）による優先度解決

CSS と同様に、class 数が多い style が優先される。同じ詳細度の場合はファイル内で後に書かれた方が優先される。

```
詳細度 0: [style.default]          （暗黙のデフォルト）
詳細度 1: [style.chrome]           （class 1個）
詳細度 1: [style.ime-off]          （class 1個）
詳細度 2: [style."chrome ime-off"] （class 2個 → より具体的）
詳細度 2: [style.vscode]           （class定義内に条件2個）
```

##### カスケード（上書き順序）

1. デフォルト設定（`[layout]`, `[timing]`, `[remap]` 等）
2. 詳細度の低い style から順に適用
3. 詳細度の高い style で上書き
4. 同じ詳細度なら後に書かれた方が優先

適用されなかったフィールドはデフォルト設定がそのまま残る（差分上書き方式）。

### class/style モデルの利点

| 観点 | 旧 profile 方式 | class/style 方式 |
|------|----------------|-----------------|
| AND 条件 | profile 内に全条件を列挙 | スペース区切りで自然に書ける |
| 条件の再利用 | 毎回書く | class 定義を使い回せる |
| 同じ設定を複数条件に | profile をコピー | 同じ class を複数の style に含める |
| IME 状態 + アプリの組合せ | 表現不可 | `[style."chrome ime-off"]` |
| 優先度の制御 | 条件数ベース | CSS 同様の詳細度で直感的 |

## 将来の拡張: スクリプト言語の導入

class/style モデルにより条件の複合が TOML で表現可能になったため、スクリプト言語の必要性はさらに低下した。以下の要件が出てきた場合にのみ段階的に導入する。

| 要件 | TOML の限界 | スクリプトで解決 |
|------|-----------|----------------|
| 配列テーブルの動的生成 | ループ・関数がない | 50音テーブルをコードで生成 |
| 複雑な出力変換 | trigger/replace の固定パターンでは表現できない変換 | Lua 関数で状態を持った変換 |
| 任意の条件式 | class のフィールドに収まらない条件 | `if window_title:match("...") then` |

スクリプト言語を導入する場合の設計方針:

- **メッセージループでのみ実行**（フック内では呼ばない）
- **TOML と共存**（スクリプトは TOML で書けない部分だけ担当）
- **VM は起動時生成、設定リロード時に再生成**

## 結果

### メリット

- 学習コスト最低（TOML は広く知られている）
- CSS のメンタルモデルを流用でき、条件の複合も宣言的に書ける
- 条件定義（class）と設定値（style）の分離により、再利用性が高い
- 宣言的なのでバリデーションが容易
- スクリプト言語のビルド依存が不要

### デメリット

- ループ・関数がないため、大量のマッピング定義は冗長になる
- CSS の詳細度モデルに馴染みがないユーザーには style の優先順位が分かりにくい可能性がある
- 将来スクリプト言語を追加する場合、TOML との共存設計が必要

### 実装ファイル（予定）

| ファイル | 内容 |
|----------|------|
| `src/config.rs` | TOML パーサー拡張（class, style, combo, macro セクション） |
| `src/engine/cascade.rs` | class マッチング + 詳細度計算 + カスケード適用 |
| `src/engine/remap.rs` | リマップテーブル管理、差し替え API |
| `src/engine/macro_engine.rs` | リングバッファ + trigger マッチ + 置換処理 |
| `crates/awase-windows/src/runtime.rs` | フォーカス変更・IME 変化時の style 再評価 |
