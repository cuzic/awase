# awase Windows IME 制御 — Architecture Decision Records

## 索引

| ADR | タイトル | ステータス |
|-----|---------|---------|
| [0001](0001-ime-detection-strategy.md) | IME 状態検出戦略 | 安定 |
| [0002](0002-tsf-coldstart-warmup.md) | TSF cold-start warmup 戦略 | 安定 |
| [0003](0003-chrome-vk-injection.md) | Chrome VK injection と F2 warmup | 実験中 |
| [0004](0004-injection-mode-design.md) | InjectionMode 三分岐設計 | 安定 |
| [0005](0005-focus-classification.md) | フォーカス判定と AppKind 設計 | 安定 |
| [021](021-deferred-effect-execution.md) | Effect 遅延実行（bounded ring + guard slot 含む） | 採用済み |
| [030](030-tsf-three-layer-architecture.md) | TSF 状態管理の3層分離アーキテクチャ | 採用済み |
| [031](031-win32-async-crate.md) | win32-async クレートの設計 | 採用済み |
| [032](032-ime-state-reducer-4-layer-model.md) | IME 状態モデルの4階層 reducer アーキテクチャ | 採用済み |
| [033](033-app-ime-profile.md) | AppImeProfile — アプリ別 IME API 互換性分類 | 採用済み |
| [034](034-gji-direct-strategy.md) | GJI Direct Strategy — Google 日本語入力との協調設計 | 採用済み |
| [035](035-decision-executor-pure-state-machine.md) | DecisionExecutor の純粋状態機械化 | 採用済み |
| [036](036-runtime-boundary-api.md) | Runtime フィールド境界 API | 採用済み |
| [037](037-keymap-remap-design.md) | キーマップ再割当設計 | 採用済み |
| [038](038-force-guard-drift-monitor.md) | ForceGuardSet / DriftMonitor 型分解 | 採用済み |
| [039](039-tsf-obs-access-control.md) | TSF_OBS アクセス制御の5フェーズ段階的強化 | 採用済み |
| [040](040-incremental-refactor-strategy.md) | 大規模リファクタリングの段階的遷移戦略 | 採用済み |
| [041](041-hook-reentry-modifier-consistency.md) | フック再入時の修飾キー整合性保証 | 採用済み |
| [042](042-clock-trait-timed-fsm.md) | Clock トレイト抽象化と timed-fsm のテスト可能性 | 採用済み |
| [043](043-app-delivery-profile.md) | アプリ配信プロファイル設計 | 採用済み |
| [044](044-applied-ime-state-confidence.md) | AppliedImeState と decide_kanji_apply — 保守性改善 | 採用済み |
| [045](045-dead-field-detection-policy.md) | Dead Field 検出方針とプレースホルダーフィールド禁止原則 | 採用済み |
| [046](046-gji-fsm-warm-cold-ssot.md) | GjiFsm — warm/cold 状態の FSM 一元管理 | 採用済み |
| [047](047-tickable-fsm-ime-warmup-strategy.md) | TickableFsm / ImeWarmupStrategy — 出力層 FSM 抽象化 | 採用済み |
| [048](048-sacrificial-warmup-chrome-coldstart.md) | SacrificialWarmup — Chrome cold-start の不可視プローブ方式 | 採用済み |
| [049](049-tsf-mode-literal-detect-wezterm-warm.md) | TSF mode LiteralDetect と WezTerm long-idle warm 維持 | 採用済み |
| [050](050-post-bypass-config.md) | post_bypass — バイパス後キーの NICOLA スキップ設定 | 採用済み |
| [051](051-holding-gate-timed-fsm-migration.md) | HoldingGate の timed-fsm クレートへの移植 | 採用済み |
| [052](052-tray-panic-reset.md) | トレイメニューからのパニックリセット | 採用済み |
| [053](053-step-coro-coroutine-pattern.md) | StepCoro — タイマー駆動コルーチンによる FSM チェーン置換 | 採用済み |
| [054](054-physical-key-state-injected-filter.md) | PHYSICAL_KEY_STATE と LLKHF_INJECTED フィルタリング | 採用済み |
| [055](055-engine-off-solo-triple.md) | 無変換3連打によるエンジン OFF 緊急回復 | 採用済み |
| [056](056-panic-reset-trigger-sequence.md) | パニックリセットトリガー: 同一キー連打 → OFF→ON→OFF シーケンス | 採用済み |
| [057](057-gji-keybind-f13f14-to-f21f22.md) | GJI キーバインド F13/F14 → F21/F22 への移行 | ~~採用済み~~ **廃止済み（VK_IME_ON/OFF 移行）** |
| [058](058-injection-mode-cache-toml.md) | InjectionMode の cache.toml 永続化 | 採用済み |
| [059](059-autostart-schtasks-to-hkcu-run.md) | 自動起動: schtasks → HKCU\Run レジストリへの移行 | 採用済み |
| [060](060-competing-software-detection.md) | 競合ソフトウェア起動時チェック | 採用済み |
| [061](061-win-key-ime-injection-skip.md) | Win キー押下中の IME キー注入スキップ | 採用済み |
| [062](062-injection-mode-auto-upgrade.md) | InjectionMode 事後昇格: GJI write_bytes 観測による自動昇格 | 採用済み |
| [063](063-ms-ime-tsf-separation.md) | TSF 共通層と IME 固有層の分離 + MS-IME 対応（案B） | 採用済み |
| [064](064-conv-mode-policy-gate.md) | ConvModePolicy による conv mutation ゲートの導入 | 採用済み |
| [065](065-conv-classifier-pure-fn-and-cfg-ungating.md) | conv 分類の純粋関数化と awase-windows の段階的プラットフォーム非依存化 | 採用済み |
| [066](066-gji-clsid-ime-detection.md) | GJI CLSID ベース IME 種別検出（gji_write_idle_ms ヒューリスティック廃止） | 採用済み |
| [067](067-vk-ime-on-off-migration.md) | F21/F22 → VK_IME_ON/OFF への完全移行と config1.db バインド廃止 | 採用済み |
| [068](068-jiskana-katakana-support.md) | JISかな・カタカナモードの完全サポート | 採用済み |
| [069](069-cohesion-refactor-h1-m5.md) | 凝集性リファクタ H-1〜M-5（循環依存・God Object・Reducer 不変条件） | 採用済み |
| [070](070-open-belief-pure-fn.md) | `reduce_open_belief` — 観測値を純粋関数で単一ビリーフに還元 | 採用済み |
| [071](071-deferred-vk-queue-ownership.md) | deferred VK キュー所有権 → TsfWarmupCoordinator への移管 | 採用済み |
| [072](072-conv-mode-authority-apply-resync.md) | conv_mode_authority を apply 完了ごとに再同期する | 採用済み |
| [073](073-gji-kind-process-lock.md) | GJI 検出後は active_ime_kind をプロセス中固定（MS-IME 降格禁止） | 採用済み |
| [074](074-observed-eisu-auto-direct.md) | ObservedEisu 自動直接入力切替 — idle-conv-check で IME ON 英数を自動 OFF | 採用済み |
| [075](075-imm-cross-probe-belief.md) | ImmCrossProbe による belief 補正 — Qt/GJI フォーカス時の IME 誤認識修正 | 採用済み |

既存の英語 ADR（ADR-009〜029）は `docs/` 直下に別途存在する。本ディレクトリは
Windows IME 制御に特化した日本語 ADR を補完するものである。

### 2026-07-01: 凝集性リファクタと IME apply 精度向上（ADR-069〜074）

2026-06-30〜07-01 に 21 タスクの凝集性リファクタ（ADR-069）と、それに連動した
4つの設計決定（ADR-070〜074）が確定した。

- **ADR-069** — H-1〜M-5 全 21 タスクの凝集性リファクタ。循環依存解消・状態層 OS 依存除去・
  Reducer 不変条件強化・Output→Runtime 逆依存解消・God Object 三連発の分割。
  新設ファイル 10 本（`types.rs`, `key_injector.rs`, `tsf_warmup_coord.rs` 等）。
- **ADR-070** — `OpenBeliefInputs` → `OpenBelief` の純粋関数 `reduce_open_belief`。
  ad-hoc な boolean 判定を一箇所に集約し、`confident=false` で「必ず apply」を表現。
  旧 `kanji_needs_context_override` を統合。
- **ADR-071** — deferred VK キューを各 probe machine から `TsfWarmupCoordinator` へ移管。
  「にゅうりょく→にうりょく」の probe 中打鍵消失を 2 原因同時に解消。
  StepCoro の self-priming tick 追加で空白窓を構造的に排除。
- **ADR-072** — `conv_mode_authority` を `record_ime_apply_result`（sync/async 共通）で
  apply 完了ごとに再同期。`EngineStateChanged` 遷移エッジへの依存を廃止し、
  パニックリセット後の TSF warmup スキップ desync を解消。
- **ADR-073** — GJI が一度確定した後は `active_ime_kind` をプロセス中固定。
  CLSID ポーリングの一時的な読み取り失敗で MS-IME に降格しなくなった。
  デバッグはプロセス再起動で対応。
- **ADR-074** — `idle_conv_check` で `ObservedEisu` 検出時に自動 IME OFF。
  IME ON 半角英数モードへの陥落から 500ms 以内に自動復帰する。
  `SetOpen(true)` 後の ObservedEisu stale も AssumedRomaji にリセットして engine を即活性化。

### 2026-06-27〜30: MS-IME 対応完了後の連続改善（ADR-064〜068）

ADR-063（MS-IME 対応）の後、GJI/MS-IME 共存環境の安定化・F21/F22 廃止・JISかな/カタカナ完全対応・
テスト可能性向上という5本の大きな改善が続いた。

- **ADR-064** — `ConvModePolicy`（AwaseLocked / UserManaged）で conv mutation 権限を
  明示的型で表現。`EngineStateChanged` を唯一の更新トリガーにする SSOT 化。
- **ADR-065** — conv 分類を nicola クレートの純粋関数に抽出し Linux で 75 件のテストを追加。
  `#![cfg(windows)]` blanket を廃止して純粋モジュール群を段階的 ungated 化。
- **ADR-066** — TSF `EnumProfiles` + `GetActiveProfile` で GJI の CLSID を動的発見し
  `cache.toml` に永続化。`gji_write_idle_ms` ヒューリスティックを CLSID 確定判定に置換。
- **ADR-067** — `VK_IME_ON`/`VK_IME_OFF` が config1.db バインドなしで動作すると判明し、
  F21/F22 と `gji.rs`（428 行）+ 関連コード全体を削除。ADR-057 を廃止。
- **ADR-068** — JISかな・カタカナモードの完全サポート。「カタカナ = ObservedRomaji」を
  中心原則に、belief 更新・conv 保護・ConvModeMgr 型安全化・warmup VK 選択の多層ガードを構築。

### 2026-06-30: conv 制御の構造的改善（ADR-064〜065）

ADR-063（MS-IME 対応）に続いて、conv mode 制御の安全性とテスト可能性を
構造で保証する2本の ADR が追加された。

- **ADR-064** — `ConvModePolicy`（AwaseLocked / UserManaged）で conv mutation 権限を
  明示的型で表現。bool フラグと散在したガード条件を廃止し、`EngineStateChanged` を
  唯一の更新トリガーにする SSOT 化。idle-conv-check による JISかな上書きバグも解消。
- **ADR-065** — `classify_idle_conv` / `classify_conv_transition` / `should_run_idle_conv_check`
  を nicola クレートの純粋関数として抽出し、Linux で 75 件の回帰テストを追加。
  合わせて `#![cfg(windows)]` blanket を廃止し、純粋モジュール群を段階的に ungated 化。

### 2026-06 の進化（ADR-045 完了後）

ADR-045（Dead Field 検出）の後、GJI warm/cold 管理の FSM 一元化と
それに伴う出力層トレイト抽象化が進んだ。v1.3.0 → v1.4.0 に対応する。

- **ADR-046** — GjiFsm が warm/cold の SSOT となり、scattered boolean フラグ
  （gji_long_idle / gji_last_io_ms 等）が ColdKind 分類に集約された。
  Phase 1→3 の debug_assert 段階的移行（ADR-040 パターン）で安全に切り替え。
- **ADR-047** — ImeWarmupStrategy / TickableFsm トレイトにより Output が
  具体的な FSM 型を知らない設計になった。ChromeProbe / LiteralDetectFsm が
  独立して差し込み可能になった。
- **ADR-048** — Chrome cold-start を VK_A+BS アトミックバッチで検出する
  SacrificialWarmup。WriteTransferCount ベースで timing 競合から脱却。
- **ADR-049** — WezTerm long-idle の2文字目リテラル化を「検出して warm 再送」
  パターンで解決。固定タイムアウト延長では競合条件が移るだけという教訓。

### 2026-05 後半の進化（ADR-032 完了後の構造的補強）

ADR-032 で IME 状態モデルが reducer 化されたあと、運用で見つかった
細かい欠陥を構造で塞ぐ refactor が続いた。 これらは新規 ADR ではなく
既存 ADR への追記として記録されている:

- **ADR-021 Phase 2** — input-defer の bounded ring (1024 cap + overflow tracker)、
  executor の guard 待ち専用 slot 分離（純粋 FIFO 保証）、`PendingApplyEvent`
  による sync apply outcome の record 化、 `Mutex` poison 復元による
  silent drop 根絶
- **ADR-032 Phase 3 完了後** — `ImeEvent::from_apply_outcome` で sync/async
  両 path の event 変換を 1 箇所に集約、 `docs/layer-boundaries.md` の
  C-1〜C-6 カテゴリで 6 設計原則を grep audit 化

---

## もぐらたたきが収まった分岐点

2026-03-28 の初コミットから 2026-05-19 現在までに約 **500 コミット**が積まれた。
前半（〜05-14）は同じ箇所を何度も修正するもぐらたたきが続いたが、
05-15 前後から急速に安定した。転換点は以下の三つである。

### 1. リアルタイム debug ログ（`3bc2dcb` 2026-05-19）

`--debug` フラグの追加により、フック内部の動作が初めてリアルタイムで可視化された。
それ以前は「再現した」→「おそらくこれが原因」→「修正」→「別の症状」という
サイクルで、症状への対処しかできていなかった。

### 2. 「検出不能 ≠ IME オフ」という概念の定着（`e1babb4` 2026-04-24、`82ab4e7` 2026-05-15）

`ImeSnapshot` への `Option<bool>` 3値意味論導入（04-24）と
`ImeObservations + resolve_and_clear()` による観測と判断の分離（05-15）により、
「検出できなかった = IME がオフ」という誤った前提が構造的に排除された。

それ以前は TSF/Chrome ウィンドウで `ImmGet*` が `None` を返すたびに
`ime_on = false` と解釈され、engine 誤 deactivate → force-IME-ON 発火 →
TSF 状態破壊 → 1文字目化け、という連鎖が複数の「別バグ」として現れていた。

### 3. TSF ネイティブウィンドウの構造的識別（`ce0dd02`/`41dabe1` 2026-05-19）

`is_tsf_native_window()` 関数と `ImeSnapshot.is_tsf_native` フラグの導入により、
「このウィンドウは構造的に IMM32 で検出不能」と「一時的な検出失敗」が区別された。

これにより:
- Windows Terminal での engine 誤 deactivate が解消
- `ime_detect_miss_count` の誤積算が防止され force-IME-ON の誤発火が止まった
- 「かき → kあき」クラスのバグが根本解消

---

## 長期的な教訓

- **非同期 IPC を挟む API（Chrome IMM32 シム、TSF 経由 IPC）は同期的に見えても遅延する**
- **「検出失敗」と「確定的な情報（TSF-native だから IMM32 不可）」を型で区別する**
- **タイムアウト値（EAGER_SETTLE_MS 等）を定数でチューニングするアプローチは限界がある**
  — イベント駆動（NAMECHANGE、WM_NULL ACK）に移行して根本解決
- **SendInput と SendMessageTimeout は別の配送経路（QS_INPUT vs QS_SENDMESSAGE）を通る**
  — 優先度を意識せずに組み合わせると競合する
- **`belief.ime_on` のような優先度型は「状態の責務分離」を阻む** — ADR-032 で
  「Intent / Observation / Transition / Barrier」の 4 カテゴリに分解した結果、
  observer が intent を破壊する経路が構造的に塞がれた
- **Sideband boolean guard は edge case のたびに増える** —
  `ctrl_bypass_hold` / `focus_transition_pending` / `shadow_toggle_suppressed_vks` 等
  は最終的に `InputBarrier` / `ForceGuardSet` / `DriftMonitor` という型に
  吸収されて消えた（[[project_ctrl_bypass_hold_fix]]）
- **キューと park slot を同じ `VecDeque` に押し込めると順序保証が壊れる** —
  ADR-021 Phase 2 で `queue` (純 FIFO) / `guard_held` (slot) / `pending_apply_events`
  (record) に責務分離して `push_front` を構造的に消した
- **Bounded ring buffer は overflow tracker と組で運用する** —
  drop 累積が早期警告として機能する（`InputDeferQueue::overflow_count`）
- **6 設計原則は文書だけでは守れない、grep audit にする** —
  `docs/layer-boundaries.md` で A-1〜E-1 のカテゴリに分け、検出コマンドと
  期待結果を明示してから PR レビューで実際にチェックされるようになった
- **タイミング競合を固定値で回避しようとすると別の閾値に競合が移るだけ** —
  WezTerm の NameChangeWait 延長（ADR-049）では根本解決できなかった。
  「検出して修復」パターン（LiteralDetect + warm 再送）が本質解
- **scattered boolean フラグは FSM に吸収できる** — `gji_long_idle` /
  `gji_last_io_ms` 等の boolean フラグは最終的に `ColdKind::classify()` +
  `GjiFsm` に吸収された（ADR-046）。フラグが増えてきたら FSM 化のシグナル
- **アトミックバッチ送信は UI の副作用を消せる** — Chrome VK_A+BS を
  同一 SendInput バッチで送ることで描画前に削除が完了し、ユーザーに
  プローブ文字が見えない（ADR-048）。Win32 の SendInput は同一バッチが
  連続キューに積まれる保証がある
