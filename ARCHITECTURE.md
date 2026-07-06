# awase アーキテクチャ概要

awase の内部設計について説明します。利用者向けの情報は [README.md](README.md) を参照してください。

---

## 非同期アーキテクチャ

awase の中核は Windows メッセージループベースのシングルスレッド非同期エグゼキュータ（`winmsg-executor`）で動作しています。キーボードフック・タイマー・フォーカス検出はすべて `spawn_local` による非同期タスクとして実装されており、スレッドをブロックしません。

### ブロッキング API の隔離

IMM32・MSAA・UI Automation などの一部 Win32/COM API には非同期版が存在せず、応答しないウィンドウに対してそのまま呼び出すと無期限でブロックします。これらは `run_with_timeout`（`crates/win32-async/src/thread_timeout.rs`）により別スレッドで隔離実行します：

1. ワーカースレッドを spawn してブロッキング API を呼び出す
2. 300ms 以内に結果が返れば通常どおり使用する
3. タイムアウトした場合は結果を捨て、スレッドは「孤児スレッドリスト」（`LEAKED_THREADS`）に退避する
4. 次回の呼び出し時に完了済みの孤児スレッドを自動回収（GC）する
5. 孤児スレッドが上限（8 本）に達すると新規 spawn を拒否し、即座に `None` を返す

この設計により、ハングしたウィンドウが存在しても非同期メインループは止まらず、スレッドが際限なく積み上がることもありません。

---

## 耐障害設計

awase は「止まらない・おかしくなっても自動回復する」ことを重視して設計されています。

### フックの死活監視

10 秒ごとにキーボードフックの生存を確認し（`TIMER_HOOK_WATCHDOG`）、応答がなければ自動で再インストールします。OS の都合でフックが外れても入力が止まりません。

### スリープ・復帰からの回復

PC がスリープから復帰すると IME サービスが一時的に不安定になります。awase は電源イベント（`WM_POWERBROADCAST`）を検出してフックを再インストールし、IME 状態を自動的に再同期します。

### IME 検出の多層フォールバック

IME の ON/OFF 状態は複数の方法で検出しています：

1. **シャドウ追跡** — 半角/全角・IME ON/OFF キーをリアルタイムで捕捉（イベント駆動）
2. **OS ポーリング** — 500ms ごとに IMM32 経由で確認（300ms タイムアウト保護付き）
3. **SSOT フォールバック** — 検出が連続して失敗した場合、awase 自身が IME 状態の正とみなして管理を引き取る

IMM32 ブリッジが機能しないアプリ（Chrome・WezTerm 等の TSF ネイティブアプリ）では最初から Layer 1 のシャドウ追跡のみで動作し、Layer 2/3 を迂回します。これにより「検出失敗 = IME オフ」と誤判定することなく、正しい状態を維持します。

SSOT（Single Source of Truth）フォールバックが実際に発動するのは「未知の IMM-broken アプリへの初回フォーカス時」に限られます。Chrome・WezTerm 等の既知アプリはアプリ種別キャッシュ（`imm_cache.toml`）に学習済みのため Layer 3 には到達しません。

### TSF コールドスタートの自動回復

WezTerm 等の TSF ネイティブアプリでは、フォーカス直後や Enter 確定後に IME のコンポジションコンテキストが未初期化（コールド）状態になることがあります。この状態で最初のローマ字キーを送ると、IME 初期化前に処理されて ASCII としてそのまま出力されます（例: 「これで」→「koれで」）。

awase はこれを次の手順で処理します：

1. F2（`VK_DBE_HIRAGANA`）を先行送信して IME 初期化を促す
2. `MsgWaitForMultipleObjects` + WinEvent `OBJ_NAMECHANGE` でコンポジションウィンドウの初期化完了をイベント駆動で検出する（上限 1000ms）
3. 初期化完了後にローマ字を送信する
4. それでも ASCII として漏れた場合は `LiteralDetector` が検出してバックスペースで除去し、正しいローマ字を再送する

---

## アプリ識別と出力方式の自動切替

awase はフォーカスが変わるたびに `AppKindClassifier` がアプリの種別（`AppKind`）を判定し、出力方式を自動で切り替えます。

| `AppKind` | 判定基準 | 出力方式 |
|-----------|---------|---------|
| `Win32` | IMM32 ブリッジが動作する標準 Win32 アプリ | Unicode 直接注入 |
| `TsfNative` | IMM32 不可・TSF 直結（Chrome, VS Code, WezTerm 等） | VK キーストローク |
| `Uwp` | UWP / XAML / DirectUI アプリ | Unicode 直接注入 |

判定は以下の優先順で行われます：

1. `[app_overrides]` の手動設定（最優先）
2. ウィンドウクラス名による即時判定
3. MSAA（AccessibleObjectFromWindow）による同期判定
4. UI Automation による非同期判定（別スレッド、300ms タイムアウト）
5. IMM32 呼び出し結果からの学習（`imm_capability_cache`）

学習結果はクラス名をキーに `imm_cache.toml` へ永続化されます。再起動後も判定精度が維持されます。

---

## n-gram による同時打鍵判定の精度向上

`ngram_predictive` モードでは、直前に確定したひらがな文字列を文脈として、次に来る文字がどれだけ自然かを対数確率スコアで評価し、同時打鍵の判定閾値を動的に調整します。

- **頻出連鎖**（「する」「です」「ある」「ます」など）→ 閾値を少し広げ、同時打鍵を拾いやすくする
- **稀な連鎖** → 閾値を少し狭め、誤判定を防ぐ
- 3-gram を優先参照し、未知の場合は 2-gram にフォールバック、それでも未知なら閾値変更なし

スコアは tanh で [-1, 1] に正規化してから調整幅（デフォルト ±20ms）に収めるため、極端な値になることはありません。

### データの出所

`data/ngram_hiragana.csv.gz` は日本語 Wikipedia の CirrusSearch ダンプを形態素解析（Sudachi + UniDic）して生成したひらがな n-gram 頻度表です。低頻度エントリは除去済みで、圧縮後のファイルサイズを小さく保っています。

```toml
[general]
confirm_mode              = "ngram_predictive"
ngram_file                = "data/ngram_hiragana.csv.gz"
ngram_adjustment_range_ms = 20   # 閾値の調整幅（±ms）
ngram_min_threshold_ms    = 30   # 調整後の下限
ngram_max_threshold_ms    = 120  # 調整後の上限
```

---

## クレート構成

```
awase/                  ← プラットフォーム非依存のコアエンジン
  src/
    engine/             ← 同時打鍵 FSM・IME 状態管理
    ngram.rs            ← n-gram モデル
    config.rs           ← 設定ファイル構造
crates/
  awase-windows/        ← Windows プラットフォーム実装
    src/
      tsf/              ← TSF 4 層アーキテクチャ（observer/probe/output + warmup/）
      focus/            ← フォーカス検出・AppKind 判定
      observer/         ← キーボードフック・フォーカスイベント観測
  win32-async/          ← 非同期エグゼキュータ + ブロッキング API タイムアウト隔離
  awase-settings/       ← 設定 GUI（eframe/egui）
  timed-fsm/            ← タイマー付き有限状態機械フレームワーク
```

---

## テスト

```sh
cargo test --lib                  # ユニットテスト（エンジン・config・ngram）
cargo test --test scenarios       # シナリオテスト（同時打鍵パターン）
cargo test -p timed-fsm           # timed-fsm フレームワークテスト
```

---

## 設計記録（ADR）

`docs/adr/` に Architecture Decision Records があります。主要な決定を記録しています。

| ADR | 内容 |
|-----|------|
| [0001](docs/adr/0001-ime-detection-strategy.md) | IME 状態検出戦略 |
| [0002](docs/adr/0002-tsf-coldstart-warmup.md) | TSF cold-start warmup 戦略の変遷 |
| [0004](docs/adr/0004-injection-mode-design.md) | InjectionMode 三分岐設計 |
| [029](docs/adr/029-ime-detection-resilience.md) | IME 検出の耐障害性と SSOT 設計 |
| [030](docs/adr/030-tsf-three-layer-architecture.md) | TSF 状態管理の層分離アーキテクチャ（3 層 + warmup 第4層） |
| [031](docs/adr/031-win32-async-crate.md) | win32-async クレートの設計 |
| [032](docs/adr/032-ime-state-reducer-4-layer-model.md) | IME 状態モデルの 4 階層 reducer アーキテクチャ |

開発者ガイド: [docs/layer-boundaries.md](docs/layer-boundaries.md) に
レイヤー境界ルール集（A〜E カテゴリ、計 13 項目）を集約している。
PR レビューと定期 audit のチェックリストとして使う。
