# ADR-001: awase アーキテクチャ変遷記録

**ステータス:** 記録済み  
**期間:** 2026-03-28 ～ 2026-05-23（約8週間、751コミット）  
**対象:** NICOLA 親指シフト入力システム（awase）

---

## 概要

本ドキュメントは、awase プロジェクトにおける主要なアーキテクチャ決定を git 履歴から再構成した記録である。  
プロジェクトは「初期機能完成 → 構造改善 → 品質強化」の3フェーズを経て発展した。

---

## フェーズ 1: 初期骨格（2026-03-28 ～ 4月初）

### ADR-001-A: timed-FSM の独立クレート化

**コンテキスト:** NICOLA 同時入力の判定には「N ミリ秒以内に別キーが来たら同時打鍵」という時間付き状態遷移が必要。

**決定:** 有限状態機械フレームワーク `timed-fsm` を独立クレートとして先に実装する（コミット `6776e94`）。

**理由:**
- NICOLA の状態遷移は「タイムアウト付き遷移」が本質であり、汎用ライブラリとして切り出せる
- crates.io への公開を視野に入れ、ドメイン知識を含まない純粋な FSM として設計

**結果:** engine/ はこの FSM を組み合わせて NICOLA ロジックを記述。後に crates.io 公開前提で整備。

---

### ADR-001-B: Platform Traits による OS 分離

**コンテキスト:** NICOLA エンジンのロジックは OS に依存しないが、キーフック・送出・IME 検出は Win32 専用 API が必要。

**決定:** `KeyboardHook`, `KeySender`, `ImeDetector` の3トレイトを awase クレートに定義し、Win32 実装を `awase-windows` クレートに分離する（コミット `b6d0a47`）。

**理由:**
- テスト容易性（モック実装を挿入できる）
- 将来の macOS/Linux 対応への布石

**結果:** `awase` クレートはプラットフォーム非依存。`awase-windows` が Win32 実装を担う。macOS/Linux のスタブは実機環境待ちの状態で残置。

---

### ADR-001-C: シングルスレッド UnsafeCell による Win32 グローバル状態

**コンテキスト:** Win32 メッセージループはシングルスレッドが前提。`PlatformState` を複数のコールバック（フックプロシージャ・WinEvent コールバック・WM_TIMER ハンドラ）から共有する必要がある。

**決定:** `SingleThreadCell<T>` を実装し、`UnsafeCell` で内部可変性を実現する（コミット `adab56c`）。実行時の不変条件「メインスレッドからのみアクセス」を文書化し、unsafe ブロックで明示。

**理由:**
- `Mutex` は不要なオーバーヘッド（シングルスレッドが保証されているため）
- `RefCell` より低コスト（借用チェックが不要な箇所が多い）

**後の修正（Phase 2）:** UnsafeCell の再入時 UB リスクが判明し、RefCell に置き換え（→ADR-001-K）。

---

## フェーズ 2: エンジン再設計（2026-04-09）

### ADR-001-D: FinalizePlan + OutputHistory パターン

**コンテキスト:** 初期実装では状態遷移ごとに出力を直接発行していた。バックスペース数の計算バグ（常に1）や、コンテキスト切替（IME OFF、言語切替）への対処が断片的だった。

**決定:** 全状態遷移の出力を `FinalizePlan` で宣言し、`OutputHistory` で状態を一元管理する（コミット `aea063c`）。

```
Before: 各 handler が直接 send_keys() を呼ぶ
After:  handler → FinalizePlan を組立て → finalize_plan() で一括実行
```

**理由:**
- バックスペース数の正確な計算（IME 合成単位での管理が必要）
- `flush_pending`（コンテキスト無効化）の統一処理
- テストで出力を検査しやすくなる（FinalizePlan を assert できる）

**結果:** テスト 216 ケース追加。エンジンロジックの見通しが大幅改善。

---

### ADR-001-E: IME 制御キーの明示的処理

**コンテキスト:** 半角/全角キー・カタカナ/ひらがなキーなどは通常の文字入力と扱いが異なる。pending キーをフラッシュしてから OS に渡す必要がある。

**決定:** `is_ime_control_vk()` を設け、`flush_and_pass_through` パスを追加する。

**理由:** pending がある状態で IME 制御キーを受けると文字化けが発生するため。

---

## フェーズ 3: Focus 検出の多層化（2026-04-中旬）

### ADR-001-F: 3フェーズ + Learning Cache による Focus 判定

**コンテキスト:** Windows のウィンドウ種別（テキスト入力可 / バイパス）は、クラス名だけでは判定が難しい。UIA・MSAA などの可用性が環境依存。

**決定:** 判定を3フェーズに段階化する（コミット群 `49eed58`, `b44f954`, `665986e`）。

```
Phase 1: クラス名ヒューリスティック（同期・高速）
Phase 2: MSAA ロール検出（同期・中速）
Phase 3: UIA 非同期検出（ワーカースレッド・低速）
```

加えて `Learning Cache`（コミット `9641029`）で判定結果をキャッシュ。TTL でソース別管理。

**理由:**
- Phase 1 だけではゲーム/ターミナルの誤検知が多い
- Phase 3 は UIA が応答するまで 100ms+ かかる場合があり同期待機できない

**結果:** `win32-async::offload_timeout` でタイムアウト付きワーカー実行パターンを確立。

---

## フェーズ 4: IME 観測・出力パイプラインの成熟（2026-04末 ～ 05初）

### ADR-001-G: TsfObservations / OutputGate によるグローバル構造体化

**コンテキスト:** TSF コールバック（`ITfTextEditSink`, `ITfUIElementSink` 等）は COM スレッドで発火するため、メインスレッドの `PlatformState` に直接アクセスできない。バラバラな `AtomicU64` 型グローバル変数が散在していた。

**決定:** `TsfObservations` 構造体に全 TSF 観測データをまとめ、`static TSF_OBS` として公開する。`OutputGate` で出力中フラグを管理する（コミット `8218dd7`）。

**理由:**
- Atomic 変数の意味をグループ化して可読性向上
- 初期化・テスト時のリセットを構造体単位で行える
- COM スレッドとメインスレッドの通信境界を明確化

---

### ADR-001-H: block_on の撤廃と TIMER 駆動への移行

**コンテキスト:** TSF probe（TSF が組成を受理したかの確認）に `block_on(sleep_ms)` を使用していた。`with_app` の中で `GetMessageA` ループを回すため、再入が発生し UB のリスクがあった。

**決定:** `block_on` を廃止し、`SetTimer`（`TIMER_TSF_PROBE`）でポーリングをメッセージループに委譲する（コミット `89b84df`）。

**理由:**
- Win32 メッセージループは再入禁止（`with_app` 内でメッセージを処理すると再入する）
- タイマー駆動ならスタックが浅くなり、再入の余地がない

**関連:** `win32-async::offload` も `Future` ベースのステートマシンに書き換えて5msポーリングを廃止（コミット `2be783b`）。

---

### ADR-001-I: レイヤ境界の修正（output → focus の逆依存解消）

**コンテキスト:** `output/mod.rs` が `focus::AppKindClassifier` を参照しており、出力層がフォーカス検出層に依存する逆転が発生していた。

**決定:** `AppKindClassifier` を `focus::classifier` に移動し、`output` からの依存を除去する（コミット `0a47411`）。

**理由:**
- レイヤ図: `engine` → `platform` → `focus` → `hook/output` の方向が正しい
- 逆依存があるとモジュール単独のテストが困難になる

---

## フェーズ 5: グローバル状態の安全化（2026-05-21 ～ 22）

### ADR-001-J: with_app 再入の安全な検出

**コンテキスト:** SendMessage（クロスプロセス IME）や `block_on` のネストメッセージループ経由で `with_app` が再入されると、`UnsafeCell` の二重可変借用が発生し UB になる。

**決定:** 再入を検出したら `log::error!` を出力して `None` を返す RAII ガードを導入する（コミット群 `3799bbc`, `81c654f`, `c51ec85`, `91594fd`）。

```rust
// with_app_or_repost: 再入時はメッセージを post し直す
pub fn with_app_or_repost<R>(msg: u32, f: impl FnOnce(&mut Runtime) -> R) -> Option<R>
```

**理由:**
- `extern "system"` の FFI 境界を越えて panic を伝播させると UB
- 再入を「失敗」として扱い、呼び出し元が再スケジュールできるパターンが安全

---

### ADR-001-K: SingleThreadCell を UnsafeCell → RefCell に置き換え（Phase 2）

**コンテキスト:** ADR-001-J で再入ガードを追加したが、その実装自体が thread_local + RAII で複雑になっていた。根本的に `UnsafeCell` の二重借用は未定義動作であり、テストで再現困難なバグの温床だった。

**決定:** `UnsafeCell<Option<T>>` を `RefCell<Option<T>>` に置き換える（コミット `e04af9e`）。

```rust
// Before
pub unsafe fn get_mut(&self) -> Option<&mut T>   // UB の可能性

// After
pub fn try_borrow_mut(&self) -> Option<RefMut<Option<T>>>  // 安全
pub fn try_with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R>
```

**トレードオフ:**
- `RefCell` は実行時借用チェックのオーバーヘッドがある
- しかし Win32 メッセージループのサイクル（~60fps）では無視できるコスト

**結果:** `IN_WITH_APP` thread_local と RAII ガードを削除。コードが 110 行削減。二重借用は panic に（UB からの脱却）。

---

### ADR-001-L: APP グローバルを RUNTIME に改名

**コンテキスト:** `APP` という名前はグローバル変数の役割（Win32 ランタイムのシングルスレッド状態）を表現できていなかった。

**決定:** `APP` → `RUNTIME` に改名。`with_app` 関数名は呼び出し側との互換性のため維持（コミット `9d1d10d`）。

---

## フェーズ 6: 用語・命名の統一（2026-05-22 ～ 23）

### ADR-001-M: 6 Wave の用語統一リファクタリング

**コンテキスト:** コードベースに渡って IME/TSF 関連の概念が複数の名前で表現されていた。`belief`（古い概念）、`Guard`（関係ない Gate）、`cold_n`（連番か？）など。

**決定:** 6段階に分けて用語を統一する（コミット群 `93bb00f`, `f67681e`, `4f22ed8`, `fd5f0b2`, `8fb2872`, `0be4c1d`）。

| Wave | 変更前 | 変更後 | 理由 |
|------|--------|--------|------|
| 1 | `belief.rs` | `last_apply.rs` | 「信念」より「最後の適用結果」が正確 |
| 2 | `SuppressSignals` | `FocusProbeGraceFlags` | 抑制ではなく猶予期間フラグ |
| 3 | `cold_n` | `cold_seq` | 連番（sequence）であることを明示 |
| 3 | `suppress_warm_epoch` | `reset_warm_epoch` | リセット操作であることを明示 |
| 3 | `TsfReadinessJudge` | `TsfReadinessProbe` | Probe（観測）が正確 |
| 4 | `is_force_tsf/vk` | `injection_hint()` | bool フラグより列挙型で意図を表現 |
| 4 | `ctrl_bypass_hold` | 同名（新規追加） | 既存 `suppress_ctrl_bypass` を意図明示に |
| 5 | `ImeGuardState` | `ImeGateState` | Guard（番人）より Gate（制御弁）が正確 |
| 5 | `GuardOutcome` | `RoutingOutcome` | ルーティングの結果であることを明示 |
| 6 | `mark_cold_focus_change/confirm_key/ime_toggle` | `mark_cold(PlatformColdReason)` | 3メソッドを統一 enum で一本化 |

**結果:** 557 テストが変更後も全通過。

---

## フェーズ 7: 循環的複雑度の管理（2026-05-22 ～ 23）

### ADR-001-N: Cognitive Complexity ≤ 15 の CI ゲート化

**コンテキスト:** `classify_ime_snapshot`（CC 28）、`validate`（CC 25）、`apply_focus_probe_to_app`（CC 24）など、読解困難な関数が複数存在していた。

**決定:** Clippy の `cognitive_complexity` lint を CI ゲートとして追加し、閾値を 15 に設定する（コミット `7c8c8ca`）。

```toml
# clippy.toml
cognitive-complexity-threshold = 15
```

```yaml
# .github/workflows/ci.yml
- run: cargo clippy ... -W clippy::cognitive_complexity
```

**削減前後の実績:**

| 関数 | 削減前 | 削減後 | パターン |
|------|--------|--------|---------|
| `classify_ime_snapshot` | CC 28 | CC 8 | PollOutcome enum で分岐を enum 化 |
| `validate` | CC 25 | CC 8 | バリデーション項目をヘルパーに分割 |
| `apply_focus_probe_to_app` | CC 24 | CC 8 | SuppressSignals 構造体で条件集約 |
| `hook_callback` | CC 21 | CC 2 | is_self_injected / defer_key に分割 |
| `YabEditor::update` | CC 22 | CC 1 | フィールド別ハンドラに分割 |
| `execute_one` | CC 18 | CC 2 | 3ヘルパーに分割 |
| `classify_route` | CC 18 | CC 4 | OsModifiers 構造体化 + ヘルパー分割 |
| `advance_tsf_probe` | CC 23 | CC 6 | フェーズ別メソッドに分割 |

**結果:** `cargo cc`（`cargo clippy --lib -W clippy::cognitive_complexity -D warnings`）をローカル測定コマンドとして整備。

---

## フェーズ 8: unsafe の体系的文書化（2026-05-22 ～ 23）

### ADR-001-O: // SAFETY: コメントを全 unsafe ブロックに付与

**コンテキスト:** 188 箇所の unsafe ブロックのうち、SAFETY コメントがあったのは 33 箇所（17%）のみ。

**決定:** 全 unsafe ブロックに `// SAFETY:` コメントを付与し、安全性の根拠を文書化する（コミット群 `61b0e4f`, `171d708`, `adb5b46`, `4fd5d26`）。

**カバレッジ推移:** 17% → **89%**（169/188 ブロック）

**典型的な根拠パターン:**

```rust
// SAFETY: メインスレッドからのみ呼ばれる。WH_KEYBOARD_LL コールバックの制約。
let state = unsafe { APP.get_ref_unchecked() };

// SAFETY: HWND はメッセージループで取得したもので有効期間内にある。
unsafe { SendMessageW(hwnd, WM_IME_CONTROL, ...) };

// SAFETY: FFI の型要件。Win32 は null ポインタを「無効」として定義している。
let raw = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
```

---

## フェーズ 9: Clippy lint の段階的強化（2026-05-23）

### ADR-001-P: nursery/pedantic の全面有効化と矛盾解消

**コンテキスト:** Cargo.toml に `nursery = "deny"` を追加したところ、`unreachable_pub`（Rust built-in lint）と `redundant_pub_crate`（clippy::nursery）が矛盾することが判明した。

```
pub(crate) struct X in pub(crate) mod foo
→ redundant_pub_crate: 「pub にしろ」
→ unreachable_pub: 「pub(crate) にしろ」（pub にしたら）
```

**決定:** `redundant_pub_crate = "allow"` で片方を抑制し、`pub(crate)` の意味的な明示性を優先する（コミット `16e778b`）。

**理由:**
- `pub(crate)` は「クレート内公開」という意図を明示する
- `unreachable_pub`（Rust built-in）は CI ではより重要度が高い
- 両方を同時に満たすことは不可能

**最終 lint 設定:**

```toml
# Cargo.toml [lints.rust]
unreachable_pub = "warn"        # CI で deny 化

# Cargo.toml [lints.clippy]
all      = { level = "deny", priority = -1 }
pedantic = { level = "deny", priority = -1 }
nursery  = { level = "deny", priority = -1 }
redundant_pub_crate = "allow"   # unreachable_pub と矛盾するため
```

---

## フェーズ 10: 出力モジュールの分割

### ADR-001-Q: output/mod.rs の 1766 行問題と分割

**コンテキスト:** `output/mod.rs` が 1766 行に達し、VK 解決・送信・キャラクター処理が混在していた。

**決定:** `output/resolve.rs`（文字→VK コード解決）と `output/vk_send.rs`（VK 送信ロジック）に分割する（コミット `1844e7a`）。

```
output/
├── mod.rs        1766行 → 1056行
├── resolve.rs    160行  (新規: ascii_to_vk, special_key_to_vk, resolve_char)
├── vk_send.rs    563行  (新規: make_key_input, TsfSendPipeline, send_*系)
├── sender.rs     (既存: InjectionSender trait)
└── types.rs      (既存: InjectionMode enum)
```

**結果:** 各ファイルが単一責任を持ち、Clippy の cognitive_complexity 閾値を通過。

---

## クレート構成の最終形

```
awase/                     ← プラットフォーム非依存 core
├── src/engine/            ← NICOLA FSM + timed-fsm 使用
├── src/platform.rs        ← OS 抽象トレイト
└── src/yab/               ← yamabuki 互換設定形式

awase-windows/             ← Windows 専用実装
├── src/hook.rs            ← WH_KEYBOARD_LL フック
├── src/output/            ← SendInput / TSF 注入
├── src/ime/               ← TSF + IMM32 ハイブリッド検出
├── src/tsf/               ← TSF 観測・probe・warm/cold 管理
├── src/focus/             ← フォーカス判定（3 Phase + Cache）
├── src/runtime/           ← シングルスレッドランタイム統合
├── src/state/             ← PlatformState 集約
└── src/app/               ← Win32 メッセージループ・起動

crates/timed-fsm/          ← 汎用タイムド FSM（crates.io 公開予定）
crates/win32-async/        ← offload / race_with_timeout / sleep
crates/win32-worker/       ← ShutdownToken 付きワーカースレッド管理
```

---

## 学んだ設計原則

### 1. 「安全でない部分を小さく保つ」
UnsafeCell から RefCell への移行（ADR-001-K）が示すように、unsafe の範囲は可能な限り縮小する。不変条件が自明でない場合は SAFETY コメントで根拠を記録する。

### 2. 「大きな関数は症状、原因を直す」
CC 20+ の関数は、複数の概念が混在している証拠。`FinalizePlan`、`PollOutcome`、`SuppressSignals` など、新しい型・enum を導入することで分岐を整理できる。

### 3. 「用語は概念と 1:1 に対応させる」
`Guard` と `Gate` は違う概念（番人 vs 制御弁）。コードの名前が概念と一致していないと、レビューや議論で認知コストが高くなる。用語統一は機能追加と同じくらい重要。

### 4. 「グローバル変数は構造体にまとめる」
`TsfObservations`、`OutputGate`、`ImeStateHub` への集約は、散在した atomic 変数を型安全にし、テスト・デバッグを容易にした。

### 5. 「CI でコード品質を強制する」
Cognitive Complexity の CI ゲート化により、新機能追加のたびに CC 測定が自動化される。人間のレビューに依存しない品質維持の仕組みが重要。

---

## 統計サマリ

| 指標 | 値 |
|------|-----|
| 総コミット数 | 751 |
| 期間 | 2026-03-28 ～ 2026-05-23（57日） |
| refactor コミット数 | 109 |
| fix コミット数 | 108 |
| テスト数（最終） | 557 件 all passed |
| Clippy エラー（最終） | 0 |
| SAFETY コメントカバレッジ | 89%（169/188 ブロック） |
| 最大 CC 削減 | classify_ime_snapshot: CC 28 → 8 |
| output/mod.rs 削減 | 1766 行 → 1056 行（+サブモジュール） |
