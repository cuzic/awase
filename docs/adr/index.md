# awase Windows IME 制御 — Architecture Decision Records

## 索引

| ADR | タイトル | ステータス |
|-----|---------|---------|
| [0001](0001-ime-detection-strategy.md) | IME 状態検出戦略 | 安定 |
| [0002](0002-tsf-coldstart-warmup.md) | TSF cold-start warmup 戦略 | 安定 |
| [0003](0003-chrome-vk-injection.md) | Chrome VK injection と F2 warmup | 実験中 |
| [0004](0004-injection-mode-design.md) | InjectionMode 三分岐設計 | 安定 |
| [0005](0005-focus-classification.md) | フォーカス判定と AppKind 設計 | 安定 |
| [001](001-ime-reliability-detection.md) | UIA FrameworkId ベースの IME 信頼度判定 | 採用済み |
| [002](002-input-processing-output-layers.md) | 入力・処理・出力の3層分離 | 採用済み |
| [003](003-nonblocking-ime-cache.md) | フックからブロッキング IME 検出を追い出し、キャッシュ化 | 採用済み |
| [004](004-appstate-orchestrator.md) | AppState をオーケストレータとして集約、依存方向の逆転 | 採用済み |
| [005](005-shadow-ime-tracking.md) | Shadow IME 状態追跡と IME トグルキー検出 | 採用済み |
| [006](006-output-mode.md) | 出力モード選択 (per_key / batched / unicode) | 採用済み |
| [007](007-focus-debounce.md) | フォーカス変更時の IME キャッシュ更新デバウンス | 採用済み |
| [008](008-physical-thumb-state-separation.md) | 物理親指キー状態と FSM 解決ロジックの分離 | 採用済み |
| [009](009-data-carrying-engine-state.md) | データ付き enum による FSM 状態表現 | 採用済み |
| [010](010-thumb-consumption-timestamp.md) | Option\<Timestamp\> による親指キー消費追跡 | 採用済み |
| [011](011-raii-win32-resources.md) | RAII ガードによる Win32 リソース管理 | 採用済み |
| [012](012-newtype-vkcode-scancode.md) | VkCode / ScanCode newtype の全面適用 | 採用済み |
| [013](013-unified-effect-model.md) | 統一 Effect モデル（Decision / Effect パターン） | 採用済み |
| [014](014-observer-executor-runtime.md) | Observer / Executor / Runtime の3層分離 | 採用済み |
| [015](015-shift-reduce-parser.md) | NicolaFsm のシフト-リデュースパーサーモデル | 採用済み |
| [016](016-engine-responsibility-separation.md) | Engine 内部の責務分離（5層構造） | 採用済み |
| [017](017-timing-judge.md) | TimingJudge によるタイミング判定の集中化 | 採用済み |
| [018](018-lessons-from-other-emulators.md) | 他の親指シフトエミュレータからの教訓と対策 | 採用済み |
| [019](019-platform-independence.md) | lib クレートのプラットフォーム非依存化 | 採用済み |
| [020](020-key-lifecycle.md) | KeyLifecycle による Down/Up ペア追跡 | 採用済み |
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
| [076](076-sleep-wake-is-japanese-ime-grace.md) | スリープ復帰後 is_japanese_ime 一時 false — grace 保護 | 採用済み |
| [077](077-observation-admission-epoch.md) | ObservationAdmission Layer — FocusEpoch による probe 受理ポリシー | 採用済み |
| [078](078-ime-mode-belief-desired-effective-constraint.md) | IME conv-mode belief の三分割（DesiredMode / EffectiveMode / ModeConstraint）— Imm32Unavailable/TsfNative 限定、Standard は観測駆動を維持 | 提案中 |
| [079](079-epoch-fenced-literal-recovery-with-replay.md) | per-VK confirm の stale confirm 誤帰属 — epoch fencing + ESC ベース recovery + 変換トリガー除外 replay | 提案中 |
| [080](080-ime-actuation-lifecycle-and-epoch-fenced-drift-correction.md) | IME actuation の型付きトランザクション化 — Feedback（Read/Blind）で closed-loop/open-loop を表現し drift correction の無限/皆無ループを根治 | Phase 1 実装済み（実機ソーク未実施） |
| [081](081-per-profile-capability-driver-decomposition.md) | IME 制御をプロファイル別 capability 駆動ドライバへ分離 — 共有ループの分岐面をやめ「アプリA向け修正がアプリBを壊す」波及を構造的に止める | Phase 1a/1b/1c 試験実装済み（未配線・Linux検証済み、実機ソーク未着手） |
| [082](082-journal-structured-replay-and-event-origin.md) | `journal.rs` を事後ログから構造化リプレイ基盤へ格上げ — 出所(source)・世代(epoch)の規律を横断型 `EventOrigin` 1箇所に統合 | 第一歩・Phase 0.5 実装済み（全面適用は未着手） |
| [083](083-injection-mode-per-vk-unification-investigation.md) | `InjectionMode`（文字送信経路）を GJI 専用に per-VK 確認方式へ統一する構想の検討記録 | 検討フェーズ・統一自体は NO-GO（観測専用の診断配線のみ実施済み） |
| [084](084-conv-mode-single-ownership-and-width-ssot.md) | conv-mode の単一所有権と「出力の幅を IME に委譲しない」原則 — 物理シフト面・belief キャッシュ・送信保証の責務再配置 | 提案（北極星仕様、未実装） |
| [085](085-conv-mode-force-policy.md) | `conv_mode_policy = force` — cold 転換時に awase トレイの目標 conv モードを強制する opt-in 設定。ADR-078 全面実装を待たない軽量な緩和策 | 実装済み（デフォルト無効、実機ソーク未実施） |
| [086](086-force-write-trigger-and-target-identity.md) | force-write の単一規律 — 「観測を信じない書き込み」のトリガー条件（arm-on-focus/fire-on-intent）と書き込みターゲット同一性（ActuationTarget）。ADR-084 の姉妹編、INV-12〜19 | Phase 0〜1（INV-14 全経路移行）実装済み、Phase 2〜4 未着手、実機ソーク未実施 |
| [087](087-open-belief-actuation-warrant-separation.md) | IME open/close belief における「内部信念」と「actuation の根拠」の分離 — `effective_open()` の二重用途（engine 挙動決定 と 外部書き込みの授権）を `OpenWarrant`/`WarrantBasis` で型分離。ADR-086 の根拠軸、INV-20〜28 | Phase 0〜2' 純粋ロジック実装・テスト済み・Opus 最終確認 must-fix ゼロ（BUG-63）、Phase 3 配線・実機ソーク未着手 |
| [088](088-ime-axis-capability-and-charset-owner.md) | IME 状態の軸分解（`AxisCapability`）と charset 軸の所有権（`CharsetOwner`）— ADR-087 の根拠軸を open 軸から4軸（open/charset/romaji/engine）へ一般化し、ADR-084 INV-11 が要求した conv 帰属を型にする。あわせて修飾キー汚染ハザードの**未収束**記録・VK モードキー送信口 18 箇所の棚卸し・実機実測トラック中断の経緯を保存。INV-29〜37 | **ドラフト**（軸モデル+`CharsetOwner` は pre-mortem 5ラウンドで収束・**実装未着手**／修飾キー汚染ポリシーは**収束せず**／実機実測トラックは**中断**。コード変更なし） |
| [089](089-ime-typestate-and-capability-const-table.md) | IME 状態制御を Rust の型システムでどう表現するか — 型状態パターンの**局所適用**3箇所（`ObservationStore` の Actuating/BeliefOnly プール分離を関連型で排他化、`Actuation<Requested/Warranted/Verified>` チェーン、`ActuationReceipt` による `GjiFsm` 同期義務のアフィン型化）と、capability を **const 表 `caps(p,k)`** に据える決定。**trait 静的分岐は却下**（§4.1、再提案禁止）。ADR-088 の姉妹編（088=「何が壊れているか」／089=「型でどう表現するか」）。INV-38〜46 | **ドラフト**（Fable×Opus pre-mortem 4ラウンド + 起票後 Opus レビュー(round5、指摘10件反映)で収束。**Phase A/B/C すべて実装済み**（2026-08-12）——ただし `record`/`record_belief` の本番呼び出し元はゼロ（§9-10）、`issue_open_warrant()` も未配線で `warrant_pending_adr087()` が 2 箇所（§9-12）、`ConvergedReceipt` は制御フロー未接続（§9-16）、非同期チェーンは `caps` 未適用（§9-20）。**実機ソーク未実施**（申し送りは §9-17）。残課題の詳細化は ADR-090 が引き取った） |
| [090](090-typestate-effectuation-and-adjacent-adr-closure.md) | ADR-089 の型保護を実効化し、隣接 ADR の後始末を確定する — `issue_open_warrant()` の実配線（`ActuationOrder` による warrant の運搬、shadow→enforce の二段階）、`ConvergedReceipt` の制御フロー配線と `most_recent_trusted_after` の private 化、観測ストアの裏口の可視性縮小と「閉じられない witness」の理由確定、非同期チェーンの `caps` 再抽選化、dylint 2 crate を恒久的に実行時 lint とする決定、**ADR-081 Phase 1d/1e の凍結決定**、golden の stale な関数名。INV-47〜52、P22 | **ドラフト（計画のみ、実装未着手）**（コード変更 0 行。7 項に優先順位と規模を付与——C/B/F/E は Linux 完結・挙動変更なし、A-1 は挙動変更なしの測定配線、D/A-2 は実機ソーク必須） |
| [091](091-idempotent-charset-axis-gji-recommended-msime-self-responsibility.md) | 冪等キー中心のIME制御 — open/romaji軸は既存対応を追認、charset軸はGJIを推奨IMEとしF15-F19のconfig1.dbバインド(`CompositionMode*`)で冪等制御。自前IMM32プローブで5モード全てのconversion_mode一致を実機検証済み。`CharsetSlot`(物理DBEキー→冪等書き込み変換)はMS-IME(composition中判定不能)からGJI(status別解釈をGJI自身に委譲でき判定不要)へ対象を付け替え。MS-IMEはパススルー+自己責任ポリシー | **決定・実装未着手**（charset軸のGJI F15-F19機構のみ実機検証済み、config1.db書き込み機能・CharsetSlot本体は未実装） |

上表の ADR はすべて日本語・本ディレクトリ（`docs/adr/`）配下にある（旧来「ADR-009〜029
は英語版が `docs/` 直下に別途存在する」という記載がここにあったが、実際にはそのような
ファイルは存在しないため削除した。`0001`〜`0005`（4桁採番）と `001`〜`082`（3桁採番）は
由来の異なる2つの採番系列が同じディレクトリに共存しているだけで、`0001`と`001`は
無関係の別 ADR である）。

このほか `docs/adr/ADR-001-architecture-history.md` というファイルが存在するが、これは
番号付き ADR 系列の一部ではなく、2026-03-28〜05-23 の約8週間・751コミットの
アーキテクチャ変遷を振り返る独立した記録文書である。ファイル名・見出しがともに
「ADR-001」を名乗っているため上表の `001`（UIA FrameworkId ベースの IME 信頼度判定）
と紛らわしいが別物なので注意すること。

### 2026-07-25〜28: ADR-081/082 の試験実装（Phase 1a/1b/1c・Phase 0.5・第一歩）

ADR-080（Phase 1、BUG-43 の drift correction 無限再送を型付きトランザクションで
終端化）に続き、Claude Fable 5 との壁打ちから起票した ADR-081/082 の実装が進んだ。
いずれも**ランタイムには未配線**（既存の `AppImePolicy`/`ime_controller.rs`/
`journal.rs` の経路がそのまま動いている）ため挙動への影響はまだ無いが、この
index.md のステータス欄が長期間「提案中」のまま更新されておらず、ADR本体の
「## ステータス」節と実際の実装状況（各ファイルの「実施記録」節）が乖離していた
ため、本追記で同期した。

- **ADR-082 第一歩** — `EventOrigin`/`Generation`/`EventSource` の最小実装。
  「誰が(source)・何回目の試行か(epoch)」を型で表現する土台。
- **ADR-082 Phase 0.5** — `JournalEntry::ImeActuation` 構造化 variant を追加し、
  `runtime::ime_actuation::Actuation`（ADR-080）に `EventOrigin` を配線。
  `tests/drift_correction_replay.rs`（BUG-43）が新 variant 経由で green。
  ADR-081 が `ir_apply_drift_correction` を書き換える前に journal リプレイ
  回帰網を張ることが目的で、ADR-081 より先行実施した。
- **ADR-081 Phase 0** — `known-bugs.md` 43件の分類 + `ImmCrossDriver` 試験実装
  （PR #31、Limited Go 判断）。
- **ADR-081 Phase 1a/1b/1c** — `Imm32UnavailableDriver`/`TsfNativeDriver` +
  ドライバレジストリ + contract test 5件を試験実装（Linux検証済み、
  `cargo test -p awase-windows --lib` 172件 green）。GJI 直接制御は
  「共有機構1箇所（design B）」として `gji_direct_mechanism.rs` に集約し、
  各ドライバは `uses_gji_direct()` の静的宣言のみを持つ設計で確定。

**残作業:** ADR-081 Phase 1d（実機ソーク必須の strangler-fig 配線、1プロファイル
ずつ read-only shadow 並走 → ソーク合格ごとに旧経路撤去）・1e（旧経路撤去の完了
確認）はこのサンドボックス（wine 未導入）では実行できず未着手。次に Windows
実機での複数アプリ×複数IMEソークが取れるセッションで着手すること。

### 2026-07-03: ObservationAdmission Layer による probe 受理ポリシー集約（ADR-077）

ALT+TAB ウィンドウ切替時の Engine OFF バグ修正を契機に、probe の「信用できる観測か」の
判断を一元化する ObservationAdmission Layer を実装。時間ベースの shadow grace を撤廃し、
FocusEpoch による正確な epoch 照合に移行した。

- **ADR-077** — `FocusEpoch`（フォーカス変更カウンタ）を `FocusStore` に導入。
  `ImmLikeTicket::admit()` が spawn 時と完了時の epoch を照合し、stale な観測を棄却。
  `AcceptedObservation` トークンにより `write_*` 関数の admission bypass をコンパイル時に禁止。
  `derive_open()` に epoch フィルタを追加し、`ImmCrossProbe` / `FocusProbe` の
  stale 観測を読み出し時にも排除（GJI / ObserverPoll / TSF はイベント駆動のため対象外）。

### 2026-07-02: スリープ復帰 IME 固定バグ修正（ADR-076）

PC スリープ復帰後、Windows Terminal 等の TsfNative アプリで IME が OFF に固定されるバグを修正。

- **ADR-076** — `apply_focus_probe` 内で `is_japanese_ime` の false ダウングレードを
  shadow grace active 中に抑制。`compute_focus_probe_grace` を `set_is_japanese_ime` より
  前に移動し、`imc_open` と `is_japanese_ime` の grace 保護を対称化。

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
