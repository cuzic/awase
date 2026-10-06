# awase Windows IME 制御 — Architecture Decision Records

> 2026-09-11: 各ADRファイル先頭にYAML frontmatter（`id`/`title`/`status`/`related_adr`、
> 旧indexの記述が長かったものは`summary`も）を追加した。以下の「タイトル」「ステータス」列は
> 機械的に短縮したもの（長かったものはfrontmatterの`summary`/`status`に全文を保持）。
> 判断が必要な場合は必ずファイル本文（またはfrontmatter）を開くこと。

## 索引

| ADR | タイトル | ステータス |
|-----|---------|---------|
| [0001](0001-ime-detection-strategy.md) | IME 状態検出戦略 | 履歴文書(2026-05-19 時点のスナップショット)。方針(shadow・3値意味論・IMM能力キャッシュ・TSFネイティブ識別)は… |
| [0002](0002-tsf-coldstart-warmup.md) | TSF cold-start warmup 戦略 | 履歴文書(2026-05-19 時点のスナップショット)。「現在の設計」節の warmup(eager F2 送信+EAGER_SETT… |
| [0003](0003-chrome-vk-injection.md) | Chrome VK injection と F2 warmup | 実験の記録(「実験中」は解消済み、2026-10-04 確認)。F2 先行送信+probe(案A)と send_f2_via_sendm… |
| [0004](0004-injection-mode-design.md) | InjectionMode 三分岐設計 | 採用・実装済み(InjectionMode の Unicode/Vk/Tsf 3分岐は state/injection_mode.rs … |
| [0005](0005-focus-classification.md) | フォーカス判定と AppKind 設計 | 採用・実装済み(一部置換、2026-10-04 確認)。AppKind は現行 Win32/TsfNative/Uwp(focus/ki… |
| [001](001-ime-reliability-detection.md) | UIA FrameworkId ベースの IME 信頼度判定 | 置換(部分)。UIA FrameworkId による Reliable/Unreliable/Unknown の信頼度(ImeRelia… |
| [002](002-input-processing-output-layers.md) | 入力・処理・出力の3層分離 | 採用・実装済み(InputTracker は src/engine/input_tracker.rs に現存、2026-10-04 確認)。 |
| [003](003-nonblocking-ime-cache.md) | フックからブロッキング IME 検出を追い出し、キャッシュ化 | 方針は現存、実装は置換(2026-10-04 確認)。IME_STATE_CACHE(AtomicU8)は b623523e で撤去され… |
| [004](004-appstate-orchestrator.md) | AppState をオーケストレータとして集約、依存方向の逆転 | 採用・実装済み(改名、2026-10-04 確認)。AppState は bc85d358 で Runtime に改名(crates/a… |
| [005](005-shadow-ime-tracking.md) | Shadow IME 状態追跡と IME トグルキー検出 | 実装済み(一部撤去、2026-10-04 確認)。config の ime_sync は現行コードに無い。shadow 追跡は shad… |
| [006](006-output-mode.md) | 出力モード選択 (per_key / batched / unicode) | 置換(2026-10-04 確認)。config の output_mode(per_key/batched/unicode)は 202… |
| [007](007-focus-debounce.md) | フォーカス変更時の IME キャッシュ更新デバウンス | 実装済み(統合、2026-10-04 確認)。50ms フォーカスデバウンスは focus_debounce_ms(runtime/mo… |
| [008](008-physical-thumb-state-separation.md) | 物理親指キー状態と FSM 解決ロジックの分離 | 採用・実装済み(物理親指状態は InputTracker 側 left/right_thumb_down、FSM は PendingTh… |
| [009](009-data-carrying-engine-state.md) | データ付き enum による FSM 状態表現 | 採用・実装済み(EngineState は src/engine/fsm_types.rs 等に現存、EnginePhase は現行コー… |
| [010](010-thumb-consumption-timestamp.md) | Option\<Timestamp\> による親指キー消費追跡 | 採用・実装済み(left/right_thumb_consumed: Option<Timestamp> は src/engine/in… |
| [011](011-raii-win32-resources.md) | RAII ガードによる Win32 リソース管理 | 実装済み(一部、2026-10-04 確認)。HookGuard・HotKeyGuard・WinEventHookGuard は現存、T… |
| [012](012-newtype-vkcode-scancode.md) | VkCode / ScanCode newtype の全面適用 | 採用・実装済み(VkCode/ScanCode newtype は awase-vkmap・src/types.rs・awase-win… |
| [013](013-unified-effect-model.md) | 統一 Effect モデル（Decision / Effect パターン） | 採用・実装済み(Effect/Decision・execute_decision は現存、2026-10-04 確認)。 |
| [014](014-observer-executor-runtime.md) | Observer / Executor / Runtime の3層分離 | 実装済み(移動・改名あり、2026-10-04 確認)。observer/ は crates/awase-windows/src/obs… |
| [015](015-shift-reduce-parser.md) | NicolaFsm のシフト-リデュースパーサーモデル | 採用・実装済み(ShiftReduceParser/ParseAction は crates/timed-fsm/src/parser.… |
| [016](016-engine-responsibility-separation.md) | Engine 内部の責務分離（5層構造） | 実装済み(一部変更、2026-10-04 確認)。engine.rs・fsm_adapter.rs・nicola_fsm.rs・conf… |
| [017](017-timing-judge.md) | TimingJudge によるタイミング判定の集中化 | 採用・実装済み(TimingJudge の is_simultaneous/three_key_pairing は src/engine… |
| [018](018-lessons-from-other-emulators.md) | 他の親指シフトエミュレータからの教訓と対策 | 調査記録(採用済み、2026-10-04)。個別対応項目の現存は網羅確認していない。ping 方式のフック監視は ADR-024 のとお… |
| [019](019-platform-independence.md) | lib クレートのプラットフォーム非依存化 | 採用・実装済み(KeyClassification/ModifierKey/ImeRelevance による事前分類は現存、2026-1… |
| [020](020-key-lifecycle.md) | KeyLifecycle による Down/Up ペア追跡 | 実装済み(一部撤去、2026-10-04 確認)。KeyLifecycle は src/engine/key_lifecycle.rs … |
| [021](021-deferred-effect-execution.md) | Effect 遅延実行によるフックタイムアウト防止 | 採用・実装済み(execute_from_hook・WM_EXECUTE_EFFECTS・drain_deferred は runtim… |
| [022](022-cross-platform-crate-structure.md) | クロスプラットフォームのクレート構造 | 採用・実装済み(awase-windows/linux/macos・timed-fsm 等のクレート構造は現存、2026-10-04 確… |
| [023](023-adaptive-output-and-kana-bypass.md) | アプリ適応出力とかな入力バイパス | 実装済み(その後変更、2026-10-04 確認)。AppKind は現行 Win32/TsfNative/Uwp で、本 ADR の … |
| [024](024-modifier-key-passthrough-and-ping-watchdog.md) | 修飾キーの PassThrough 保証と ping ベースフック監視 | 一部実装(一部置換、2026-10-04 確認)。Ctrl/Alt/Win の Engine バイパスは現存。ping ベースのフック監… |
| [025](025-toml-customization-design.md) | TOML ベースのカスタマイズ設計 | 未実装(提案のまま、2026-10-04 確認)。本文の TOML+CSS 型 [class.*]/[style.*] カスケード設定は… |
| [026](026-preconditions-and-key-routing.md) | Preconditions モデルと一元的キールーティング | 実装済み(一部撤去、2026-10-04 確認)。user_enabled・InputContext による Preconditions… |
| [027](027-ime-state-refresh-and-control.md) | IME 状態リフレッシュと IME 制御キーの設計 | 採用・実装済み(統合 IME リフレッシュタイマー TIMER_IME_REFRESH は現存、旧 TIMER_IME_POLL/TIM… |
| [028](028-focus-event-redesign.md) | フォーカスイベント処理の再設計 | 未実装(2026-10-04 確認)。本文の『デバウンス後のみ処理し即時の Engine 通知を削除』は設計どおりには入っていない: a… |
| [029](029-ime-detection-resilience.md) | IME 状態検出の耐障害性と SSOT 設計 | 採用・実装済み(一部撤去、2026-10-04 確認)。多層防御の Layer1/2(shadow 追跡・run_with_timeou… |
| [030](030-tsf-three-layer-architecture.md) | TSF 状態管理の3層分離アーキテクチャ | 採用・実装済み(拡張、2026-10-04 確認)。tsf/{observer,probe,output,probe_bridge}.r… |
| [031](031-win32-async-crate.md) | win32-async クレートの設計 | 採用・実装済み(crates/win32-async は run_with_timeout・block_on 等とともに現存、2026-… |
| [032](032-ime-state-reducer-4-layer-model.md) | IME 状態モデルの4階層 reducer アーキテクチャ | 採用・実装済み(一部撤去、2026-10-04 確認)。shadow_model/ImeModel::reduce と Observer… |
| [033](033-app-ime-profile.md) | AppImeProfile — アプリ別 IME API 互換性分類 | 採用・実装済み(列挙は変更、2026-10-04 確認)。AppImeProfile は現行 Standard/Imm32Unavail… |
| [034](034-gji-direct-strategy.md) | GJI Direct Strategy — Google 日本語入力との協調設計 | 採用・実装済み(一部撤去、2026-10-04 確認)。GjiDirect による冪等 VK_IME_ON/OFF 制御は現存。『GJI… |
| [035](035-decision-executor-pure-state-machine.md) | DecisionExecutor の純粋状態機械化 | 採用・実装済み(DecisionExecutor は runtime/executor.rs に現存し applied_snapshot… |
| [036](036-runtime-boundary-api.md) | Runtime フィールド境界 API | 採用・実装済み(Runtime のフィールドは大半 private、platform のみ pub、2026-10-04 確認)。 |
| [037](037-keymap-remap-design.md) | キーマップ再割当設計 | 採用・実装済み(表記更新、2026-10-04 確認)。アプリ別キー再割当・HeldModifiers は現存。設定キーは現行 [[ke… |
| [038](038-force-guard-drift-monitor.md) | ForceGuardSet / DriftMonitor 型分解 | 実装済み(改名・縮小、2026-10-04 確認)。DriftMonitor は ObserveMissMonitor に改名(a1ec… |
| [039](039-tsf-obs-access-control.md) | TSF_OBS アクセス制御の5フェーズ段階的強化 | 採用・実装済み(一部名称変更、2026-10-04 確認)。TSF_OBS は pub(in crate::tsf) で tsf/ 内に… |
| [040](040-incremental-refactor-strategy.md) | 大規模リファクタリングの段階的遷移戦略 | 採用・実施済み(ADR-032 Phase 1〜3 に適用した段階的遷移戦略、旧コードは削除済み、2026-10-04 確認)。 |
| [041](041-hook-reentry-modifier-consistency.md) | フック再入時の修飾キー整合性保証 | 実装済み(一部別構造、2026-10-04 確認)。OUTPUT_GATE・guard_held は現存。本文のフック側 is_modi… |
| [042](042-clock-trait-timed-fsm.md) | Clock トレイト抽象化と timed-fsm のテスト可能性 | 採用・実装済み(Clock・MonotonicClock・ManualClock は crates/timed-fsm に現存、2026… |
| [043](043-app-delivery-profile.md) | アプリ配信プロファイル設計 | 未実装(提案のまま、2026-10-04 確認)。AppDeliveryProfile は現行コードに存在しない(旧 status『採用… |
| [044](044-applied-ime-state-confidence.md) | AppliedImeState と decide_kanji_apply — 保守性改善 | 実装済み(一部撤去、2026-10-04 確認)。AppliedImeState は state/ime_model.rs に現存。de… |
| [045](045-dead-field-detection-policy.md) | Dead Field 検出方針とプレースホルダーフィールド禁止原則 | 採用・実施済み(dead field 検出方針、2026-10-04 確認)。 |
| [046](046-gji-fsm-warm-cold-ssot.md) | GjiFsm — warm/cold 状態の FSM 一元管理 | 実装済み(一部撤去、2026-10-04 確認)。GjiFsm は tsf/gji_fsm.rs に現存(OffCold/OnCold/… |
| [047](047-tickable-fsm-ime-warmup-strategy.md) | TickableFsm / ImeWarmupStrategy — 出力層 FSM 抽象化 | 実装済み(一部撤去、2026-10-04 確認)。TickableFsm・ImeWarmupStrategy は tsf/warmup/… |
| [048](048-sacrificial-warmup-chrome-coldstart.md) | SacrificialWarmup — Chrome cold-start の不可視プローブ方式 | 撤去済み(d4956490、2026-07-18: 捨て駒キー機構〈StartSacrificialWarmup/Sacrificial… |
| [049](049-tsf-mode-literal-detect-wezterm-warm.md) | TSF mode LiteralDetect と WezTerm long-idle warm 維持 | 一部撤去(2026-10-04 確認)。RawTsfLiteralRecovery・SuspectedLiteral による liter… |
| [050](050-post-bypass-config.md) | post_bypass — バイパス後キーの NICOLA スキップ設定 | 採用・実装済み([[post_bypass]] は src/config.rs・PostBypassEntry に現存、2026-10-… |
| [051](051-holding-gate-timed-fsm-migration.md) | HoldingGate の timed-fsm クレートへの移植 | 採用・実装済み(HoldingGate/GateAction は crates/timed-fsm/src/gate.rs に移植済みで… |
| [052](052-tray-panic-reset.md) | トレイメニューからのパニックリセット | 採用・実装済み(WM_PANIC_RESET によるトレイからのパニックリセットは現存、2026-10-04 確認)。 |
| [053](053-step-coro-coroutine-pattern.md) | StepCoro — タイマー駆動コルーチンによる FSM チェーン置換 | 実装済み(一部撤去、2026-10-04 確認)。StepCoro は crates/timed-fsm に現存、GjiWarmupCo… |
| [054](054-physical-key-state-injected-filter.md) | PHYSICAL_KEY_STATE と LLKHF_INJECTED フィルタリング | 採用・実装済み(PHYSICAL_KEY_STATE・LLKHF_INJECTED/INJECTED_MARKER フィルタは hook… |
| [055](055-engine-off-solo-triple.md) | 無変換3連打によるエンジン OFF 緊急回復 | 採用・実装済み(改称、2026-10-04 確認)。設定名は engine_off_solo_triple から engine_off_… |
| [056](056-panic-reset-trigger-sequence.md) | パニックリセットトリガー: 同一キー連打 → OFF→ON→OFF シーケンス | 採用・実装済み(RapidPressTracker・PanicTriggerCombo は panic_detect.rs・app/mo… |
| [057](057-gji-keybind-f13f14-to-f21f22.md) | GJI キーバインド F13/F14 → F21/F22 への移行 | 廃止済み(2026-06-28、ADR-067 に置換)。v2.0.0 時点でも gji.rs・GjiSetup/GjiTeardown… |
| [058](058-injection-mode-cache-toml.md) | InjectionMode の cache.toml 永続化 | 採用・実装済み(InjectionModeStore は focus/classifier.rs、cache.toml の [injec… |
| [059](059-autostart-schtasks-to-hkcu-run.md) | 自動起動: schtasks → HKCU\Run レジストリへの移行 | 採用・実装済み(HKCU\Run 自動起動と migrate_from_schtasks は autostart.rs に現存、2026… |
| [060](060-competing-software-detection.md) | 競合ソフトウェア起動時チェック | 採用・実装済み(競合ソフト検出 CONFLICTS は app/bootstrap.rs に現存、2026-10-04 確認)。 |
| [061](061-win-key-ime-injection-skip.md) | Win キー押下中の IME キー注入スキップ | 実装済み(一部撤去、2026-10-04 確認)。send_ime_mode_key の Win キー押下中スキップ(hook::win… |
| [062](062-injection-mode-auto-upgrade.md) | InjectionMode 事後昇格: GJI write_bytes 観測による自動昇格 | 採用・実装済み(UnicodeLiteralObserverFsm・InjectionHint::ForceTsf は現存、2026-1… |
| [063](063-ms-ime-tsf-separation.md) | TSF 共通層と IME 固有層の分離 + MS-IME 対応（案B） | 採用・実装済み(一部撤去、2026-10-04 確認)。ActiveImeKind・MsImeDirectStrategy・ImeWar… |
| [064](064-conv-mode-policy-gate.md) | ConvModePolicy による conv mutation ゲートの導入 | 撤去済み(ADR-094 で ConvModePolicy を全撤去、10f238b5、2026-08-17、2026-10-04 確認… |
| [065](065-conv-classifier-pure-fn-and-cfg-ungating.md) | conv 分類の純粋関数化と awase-windows の段階的プラットフォーム非依存化 | 実装済み(一部撤去、2026-10-04 確認)。conv 分類の純粋関数化・should_run_idle_conv_check は現… |
| [066](066-gji-clsid-ime-detection.md) | GJI CLSID ベース IME 種別検出（gji_write_idle_ms ヒューリスティック廃止） | 採用・実装済み(CLSID ベースの active_ime_kind 判定、gji_write_idle_ms は現行コードに無い、20… |
| [067](067-vk-ime-on-off-migration.md) | F21/F22 → VK_IME_ON/OFF への完全移行と config1.db バインド廃止 | 採用・実装済み(VK_IME_ON/OFF 移行、config1.db パッチ系の gji_keybinds_ok・GjiSetup/G… |
| [068](068-jiskana-katakana-support.md) | JISかな・カタカナモードの完全サポート | 一部実装・一部撤去(2026-10-04 確認)。JISかな検出(ObservedKana)・ローマ字扱い(ObservedRomaji… |
| [069](069-cohesion-refactor-h1-m5.md) | 凝集性リファクタ H-1〜M-5（循環依存・God Object・Reducer 不変条件） | 採用・実装済み(一部撤去、2026-10-04 確認)。ModifierState の engine 側移設・RuntimeOutbox… |
| [070](070-open-belief-pure-fn.md) | `reduce_open_belief` — 観測値を純粋関数で単一ビリーフに還元 | 撤去済み(ADR-216 R1、fbe90204、2026-10-04 確認)。OpenBelief/OpenBeliefInputs/… |
| [071](071-deferred-vk-queue-ownership.md) | deferred VK キュー所有権 → TsfWarmupCoordinator への移管 | 実装済み(一部撤去、2026-10-04 確認)。deferred VK キューの TsfWarmupCoordinator 所有(pe… |
| [072](072-conv-mode-authority-apply-resync.md) | conv_mode_authority を apply 完了ごとに再同期する | 撤去済み(再同期は現存しない、2026-10-04 確認)。ADR-088 §1.7 が bf8727ac 時点で apply 完了ごと… |
| [073](073-gji-kind-process-lock.md) | GJI 検出後は active_ime_kind をプロセス中固定（MS-IME 降格禁止） | 置換(2026-10-04 確認)。『GJI 検出後は active_ime_kind をプロセス中固定(MS-IME への降格禁止)』… |
| [074](074-observed-eisu-auto-direct.md) | ObservedEisu 自動直接入力切替 — idle-conv-check で IME ON 英数を自動 OFF | 一部撤去・一部現存(2026-10-05 棚卸し): 変更1(idle-conv-check が ObservedEisu を見て IME を自動 OFF にする EngineSync::DirectInput)は AD |
| [075](075-imm-cross-probe-belief.md) | ImmCrossProbe による belief 補正 — Qt/GJI フォーカス時の IME 誤認識修正 | 採用・実装済み(ObservationStore::derive_open・ImmCrossProbe による belief 補正は現存… |
| [076](076-sleep-wake-is-japanese-ime-grace.md) | スリープ復帰後 is_japanese_ime 一時 false — grace 保護 | 採用・実装済み(compute_focus_probe_grace・apply_focus_probe の grace 保護は現存、20… |
| [077](077-observation-admission-epoch.md) | ObservationAdmission Layer — FocusEpoch による probe 受理ポリシー | 採用・実装済み(AcceptedObservation・FocusEpoch による admission は state/ 配下に現存、… |
| [078](078-ime-mode-belief-desired-effective-constraint.md) | IME conv-mode belief の三分割（DesiredMode / EffectiveMode / ModeConstraint）と観測駆動書き込みの排除 | 一部実装→縮小(2026-10-04 確認)。DesiredMode/EffectiveMode/ModeConstraint/Mode… |
| [079](079-epoch-fenced-literal-recovery-with-replay.md) | per-VK confirm の stale confirm 誤帰属 — epoch fencing + ESC ベース recovery + 変換トリガー除外 replay | 一部実装(2026-10-04 確認)。Stage 1(StaleConfirm 検出と回収: per_vk_recovery_para… |
| [080](080-ime-actuation-lifecycle-and-epoch-fenced-drift-correction.md) | IME actuation（VK送信/IMM32呼び出し）を型付きトランザクション化し、closed-loop/open-loop の区別と有限終端を構造で強制する | Phase 1 実装済み(2026-10-04 確認)。FeedbackPolicy(Read/Blind)・decide_actuat… |
| [081](081-per-profile-capability-driver-decomposition.md) | IME 制御ロジックをプロファイル別 capability 駆動ドライバへ分離し、汎用ループの分岐面を止める | 完了(縮小して現存、2026-10-05 棚卸し): `ImeProfileDriver` trait と 3 つの ZST ドライバ(ImmCrossDriver・Imm32UnavailableDriver・TsfN |
| [082](082-journal-structured-replay-and-event-origin.md) | `journal.rs` を事後ログから構造化リプレイ基盤へ格上げ — 出所(source)・世代(epoch)の規律を横断型 `EventOrigin` 1箇所に統合 | 一部実装(決定1・決定2・Phase 0.5 は実装済み、リプレイの全面適用は未完、2026-10-04 コード確認)。`EventOr… |
| [083](083-injection-mode-per-vk-unification-investigation.md) | `InjectionMode`（文字送信経路）を GJI 専用に per-VK 確認方式へ統一する構想の検討記録 | 見送り(`InjectionMode` のGJI専用 per-VK 統一は NO-GO のまま、2026-10-04 時点で統一は未実施… |
| [084](084-conv-mode-single-ownership-and-width-ssot.md) | conv-mode の単一所有権と「出力の幅を IME に委譲しない」原則 — 物理シフト面・belief キャッシュ・送信保証の責務再配置 | 北極星仕様のまま・一部の前提が変更。`actuate_conv_mode` 単一窓口(INV-1)は現存(output/conv_act… |
| [085](085-conv-mode-force-policy.md) | `conv_mode_policy = force` — cold 転換時に awase トレイの目標 conv モードを強制する opt-in 設定 | 撤去済み(ADR-094 で `conv_mode_policy` 設定と force ポリシーを全撤去、2026-08-17)。現行コ… |
| [086](086-force-write-trigger-and-target-identity.md) | force-write の単一規律 — 「観測を信じない書き込み」のトリガー条件と書き込みターゲット同一性 | 一部撤去・一部現存(2026-10-06 裏取り): force-write のトリガー側と reassert・force-on は ADR-094・ADR-179 で撤去、ActuationTarget(ime.rs) |
| [087](087-open-belief-actuation-warrant-separation.md) | IME open/close belief における「内部信念」と「actuation の根拠」の分離（根拠軸の規律） | 一部実装・配線済み(2026-10-04 コード確認)。Phase 0〜2' の純粋ロジック(`issue_open_warrant`、… |
| [088](088-ime-axis-capability-and-charset-owner.md) | IME 状態の軸分解（`AxisCapability`）と charset 軸の所有権（`CharsetOwner`）— および修飾キー汚染ハザードの未収束記録 | 却下・見送り(トラック A の `CharsetOwner` は ADR-094 で撤回、現行コードに無い)。トラック B(修飾キー汚染… |
| [089](089-ime-typestate-and-capability-const-table.md) | IME 状態制御を Rust の型システムでどう表現するか — 型状態パターンの局所適用と capability const 表（trait 静的分岐の却下） | 一部実装(Phase A/B/C 実装済み、2026-08-12。残課題は ADR-090 が引き取り、2026-10-04 コード確認… |
| [090](090-typestate-effectuation-and-adjacent-adr-closure.md) | ADR-089 の型保護を実効化し、隣接 ADR の後始末を確定する — warrant 実配線 / 読み戻し API / 裏口の可視性 / 非同期 caps / dylint 方… | 一部実装(2026-10-04 本文の実施記録で確認)。項A(A-1 shadow 配線 e3bf7af2・A-2 `into_actu… |
| [091](091-idempotent-charset-axis-gji-recommended-msime-self-responsibility.md) | 冪等キー中心のIME制御 — open/romaji/charset 3軸の結論、GJI推奨・かな形状は設定+ベストエフォート助言(新規beliefなし)、MS-IME自己責任ポリ… | 一部実装・一部撤去(2026-10-04 確認)。決定(F21 1キー構成の推奨・MS-IME は自己責任)は本文のまま。自動判定・設定… |
| [092](092-external-key-semantics-absorption-and-thumb-key-restructure.md) | 外部ソース由来のキー意味論の吸収と、親指キー設定群の再編 | 一部実装(Step1・2・6 実装済み、Step3-5 は先送りのまま、2026-10-04 確認)。`ModeKeyConfig`/`… |
| [093](093-dbe-hotkey-observation-upgrades-japanese-ime-belief.md) | IME 専用ホットキーの受信を `is_japanese_ime()` の即時真更新トリガーにする | 実装済み(コード確認のみ、2026-10-04)。`vk.rs::is_synthetic_dbe_ime_hotkey` と `key… |
| [094](094-charset-axis-and-force-policy-removal.md) | charset 軸の追跡撤去と `conv_mode_policy`（force ポリシー）の全撤去 | 実装済み(コード確認、2026-10-04)。`ConvModePolicy`・`has_katakana` は現行コードに無い。Win… |
| [095](095-tray-bug-report-cloudflare-intake.md) | タスクトレイからの不具合報告機能 — Cloudflare Workers + R2 による非公開受付 | 実装済み(コード確認のみ、2026-10-04)。`bug_report.rs` が現存し、ADR-222(PR #451、v2.0.0… |
| [096](096-journal-priority-tiers-multi-lane-ring-buffer.md) | journal の優先度別 複数リングバッファ化と3つの取りこぼし解消 | 実装済み(コード確認のみ、2026-10-04)。journal.rs に multi-lane(`lane_kind`/`evicte… |
| [097](097-thumb-pinky-shift-chord.md) | 親指シフト×小指シフトの複合面（左親指小指シフト面／右親指小指シフト面） | **実装済み（Phase0/0.5/1、2026-08-19）だが、UIタブは2026-08-23時点で
一時的に非表示。** 実機確認で、… |
| [098](098-tsfnative-applied-confirmed-laundering-and-force-on-removal.md) | TsfNative フォーカス復帰時の `applied` 偽装確定を止め、到達不能な force-on ブロックを撤去する（BUG-69） | 一部撤去・残りは実装済み(2026-10-04 確認)。決定0/1-a/1-b/2/4/6 の applied 偽装確定の停止・到達不能… |
| [099](099-config-preservation-on-upgrade.md) | バージョンアップ時の設定消失を防ぐ — MSI/ZIP インストーラーのユーザーデータ分離と load-failure セーフティネット | 実装済み(2026-08-21)。2026-10-04 時点で `.bak` 退避が awase-settings に現存することのみ確… |
| [100](100-gji-warmup-vk-ime-on-reinit.md) | GJI eager warmup キーの再選定と give-up 分岐の retry — 提案の却下・縮小版の実験登録・前提条件の切り出し | 一部実装・一部撤去(2026-10-06 裏取り): 決定5(reinit ポーリングの focus 世代ガード)と案L(give-up で失う romaji を journal へ、5ddbbb08)は実装済みだが、決 |
| [101](101-bug74-giveup-retry-with-focus-guard.md) | BUG-74 give-up retry と focus guard | 撤去(2026-10-05 棚卸し、ADR-212 P3・P5): 決定1〜5 が前提とした GJI reinit/poll/retry(`send_chrome_gji_reinit_and_poll`・`Pendin |
| [102](102-startup-key-delivery-one-way-closure.md) | 起動シーケンスとキー配送の一方通行を閉じる | 実装済み(コード確認のみ、2026-10-04)。ADR-105 の HWND 通知(runtime/engine_window.rs)… |
| [103](103-warmup-probe-pending-integrity.md) | Warmup/Probe 過渡期の pending 取りこぼしと FSM 整合性 | 実装済み(PR #108)・一部機構は後続で撤去(2026-10-04 確認)。probe/pending の機構(output/pro… |
| [104](104-observation-freshness-and-hardening.md) | 非同期観測の鮮度・Win32 戻り値・死んだ安全弁の整理 | 一部置換・多くは未実装(2026-10-06 裏取り): 決定6-a・6-c は ADR-106 で置換。決定7(同期 conv 読み取りの追い出し)は ADR-106 決定4 が前提工事のみで本体は見送り。未実装(コー |
| [105](105-engine-thread-notification-via-hwnd.md) | エンジンスレッドへの通知はHWND宛のPostMessageWに統一する | 実装済み(コード確認のみ、2026-10-04)。`runtime/engine_window.rs` が現存。実機ソークの記録は確認で… |
| [106](106-fence-ownership-and-observation-provenance.md) | fence 識別子の所有権是正と観測プロブナンスの型強制 | 一部実装(決定1〜4 実装済み・決定5 は未実装、2026-10-06 裏取り): `FocusFence`(state/probe_admission.rs)が現存し観測の受理は一本化済み。決定5(`Lease<P>` |
| [107](107-bug25-gji-half-width-alnum-entry.md) | BUG-25 GJI 半角英数 entry の実現機構（自己注入の識別・修飾キー文脈・一度きりのトグル） | 実装済み(コード確認のみ、2026-10-04)。GJI 半角英数トグル(`HalfWidthAlnumState`、`send_gji… |
| [108](108-ime-apply-pending-generation-ordering.md) | IME apply 完了の受理判定を「pending 一致」から3つの独立した問いへ分解する | 実装済み(コード確認のみ、2026-10-04)。`ime_model.rs` に決定1(focus_epoch)・決定2/5・決定4 … |
| [109](109-yab-cv4d-punctuation-auto-confirm.md) | `.yab` 句読点確定サフィックス（やまぶき `CV4D` 相当）の実現機構 | 一部解決(2026-09-13 から変更なし、2026-10-04 確認)。確定付き `layout/nicola_kakutei.ya… |
| [114](114-keymap-app-scoped-shortcut-wiring.md) | `[[keymap]]`（アプリ別ショートカット再割当）の未配線を解消する | 実装済み(コード確認のみ、2026-10-04)。`[[keymap]]` の設定(src/config.rs)と keymap.rs(… |
| [110](110-simple-physical-key-remap.md) | 物理キー単純リマップ機能（`key_remap`） | 撤回済み(2026-08-30、ADR-111 r4 決定による)。2026-10-04 時点で `key_remap` 機構は現行コー… |
| [111](111-caps-eisu-ctrl-swap-preset.md) | Caps(英数)⇔Ctrl 入れ替え専用プリセット（Scancode Map 一本化） | 実装済み(コード確認のみ、2026-10-04)。Scancode Map 方式(`crates/awase-windows/src/s… |
| [112](112-keyup-lifecycle-fsm-delivery.md) | `Engine::on_input` Phase 0 が KeyUp を FSM に一切届けていない欠陥の修正 | クローズ(2026-08-31、変更なし)。`UpDuty`(src/engine/key_lifecycle.rs)と `min_ov… |
| [115](115-yab-keystroke-sequence.md) | `.yab` 打鍵列機能（1キーに複数の `KeyAction` を定義する） | 実装済み(コード確認のみ、2026-10-04)。`keystroke_sequence` 設定が現存。決定8追補(既定 On・GUI … |
| [116](116-startup-settings-diagnostics.md) | 起動時設定診断（awase / awase-settings 共通） | 実装済み(コード確認のみ、2026-10-04)。起動時の設定診断(awase-windows の config_diagnostics… |
| [117](117-bug138-msime-composition-diagnostic-logging.md) | MS-IME「直接入力モード許可」時の英数キー文字消失（issue #138）切り分け用ログ | 実装済み(2026-09-02、挙動変更なしの診断ログ)。`ime_controller.rs` に `composition_acti… |
| [118](118-teams-kana-lock-detection.md) | Teams(WebView2/MS-IME) のかな入力ロック検知と通知 | 実装済み(コード確認のみ、2026-10-04)。`WM_KANA_LOCK_WARNING_CHANGED`(lib.rs)と `tr… |
| [119](119-injected-and-relay-key-consumption-invariant.md) | 注入キーイベントの取り扱い — 解釈しないものは消費もしない | 実装済み(コード確認のみ、2026-10-04)。`AppImeProfile::InputRelay` が現存し、物理キー配送の判定(… |
| [120](120-retroactive-ngram-correction.md) | n-gram 事後訂正 — 後続文脈による曖昧決定の再評価と BACKSPACE 書き換え | 一部実装(Phase 0a のカウンタ・不具合報告への統合のみ、2026-10-06 裏取り): src/engine/retro_eval_stats.rs(f61f1092)と bug_report.rs の統合は実 |
| [121](121-explicit-physical-ime-key-idempotent-reassert.md) | 物理 IME 訂正キーの no-op 時に、冪等な再送を追加で試みる（BUG-37 部分対策） | 撤去済み(2026-10-04 確認)。D1 の reassert(`reassert_explicit_physical_key`)は… |
| [122](122-cold-start-per-vk-confirm-race-recovery.md) | GJI コールドスタート直後の per-VK confirm が「確認遅延」を「未着弾」と誤認し、回収送信が GJI 自身の非同期処理と競合してモーラが重複する（BUG-75 追加… | 保留(未実装、2026-10-04 確認)。案F(`grace_hold_verdict` の早期確定の修正)の実装コミットは無く、`g… |
| [123](123-focus-resync-and-probe-defer-queue-composition-race.md) | `pending_deferred` の flush ガードが GJI reinit-retry 完了しか見ていないため、reinit 完了を待つ間に到着した別モーラが独立 pro… | 一部実装(診断ログのみ、2026-10-04 確認)。診断(`TsfProbeStarted.pending_deferred_len`… |
| [124](124-tray-update-check.md) | タスクトレイからの更新確認 | 実装済み(コード確認のみ、2026-10-04)。rev.12 の設計どおり通信主体は `awase-settings`(crates/… |
| [126](126-caps-as-extra-ctrl-preset.md) | Caps(英数) を「追加の Ctrl」にするプリセット（Ctrl を2つにする） | 実装済み(コード確認のみ、2026-10-04)。Caps を追加 Ctrl にするプリセットの Scancode Map 実装(awa… |
| [125](125-egui-winit-dynamic-ime-association-focus-model-gap.md) | egui/winit アプリのウィジェット単位 IME 許可切替と、awase のフォーカスモデルの構造的ギャップ | 実装済み(コード確認のみ、2026-10-04)。BUG-107/BUG-108 は docs/known-bugs 側でコード確認済み… |
| [127](127-settings-single-apply-principle.md) | 設定画面（awase-settings）の「単一の適用」原則統一——配列編集タブの反映漏れ解消 | 実装済み(PR #157 で develop マージ、2026-10-04 時点で変更の記録なし)。設定 GUI の保存・適用の個別コー… |
| [128](128-escape-composition-collateral-deferred-loss.md) | recovery resend が自分自身の実送信より前に `pending_deferred` を drain し、出力順を反転させたうえ直後の per-VK confirm の… | 実装済み(`1b5ca721`、PR #160)。BUG-109 は docs/known-bugs 側でコード確認済みの解決として記録… |
| [129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md) | OUTPUT_GATE drain replay 中、親指キー押下タイムスタンプがイベント捕捉時点ではなくリプレイ実行時点のライブ値で再構築され、既に消費済みの押下と無関係な後続押… | 実装済み(コード確認のみ、2026-10-04)。BUG-127 は docs/known-bugs 側で、`RawKeyEvent` … |
| [130](130-keymap-multistep-shortcut-and-ime-keys.md) | `[[keymap]]` の `to` を複数キーの打鍵列へ一般化する | 実装済み(コード確認のみ、2026-10-04)。`[[keymap]]` の `to` の打鍵列(src/config.rs の `d… |
| [132](132-uncorroborated-physical-ime-key-engine-lockout.md) | 物理IMEキー1回による明示意図が、失敗しても所有権を返さない問題 | 実装済み(v2.0.0 に含まれる、2026-10-04 確認。IntentStore/last_intent/check_drift_… |
| [133](133-gji-ime-mode-key-sendinput-batch-shape.md) | `send_ime_mode_key` が送る `SendInput` バッチの形状が GJI の「@」誤出力を左右する（BUG-113 恒久修正） | 実装済み・実機確認済み(2026-09-07)、v2.0.0 に含まれる(2026-10-04 コード確認: `send_ime_mod… |
| [134](134-drift-correction-feedback-policy-focus-snapshot-staleness.md) | `app_policy` の `FeedbackPolicy` が正しく初期化・再導出されず、読み戻し不能な状態で `FeedbackPolicy::Read` の無条件再送に陥る… | 実装済み・実機確認済み(2026-09-05)、v2.0.0 に含まれる(develop 履歴上、2026-10-04 確認)。ただし … |
| [135](135-generic-thumb-key-ime-toggle-delegate.md) | 親指キー単独タップのIME ON/OFF/トグル意味論への汎用対応（BUG-115） | 撤去済み(delegate 機構は ADR-191 `06483afd` で撤去。ADR-206・ADR-199 が後継)。shadow… |
| [136](136-duplicate-immcross-probe-on-focus-change.md) | フォーカス変更時の「二重IME probe」仮説はOpus敵対的レビューで反証・却下（副産物としてBUG-78非対称を発見） | 却下(変更なし)、v2.0.0 時点でも同じ。副産物のBUG-78非対称は別課題として切り出し済み。 (2026-10-04 更新) |
| [137](137-shift-katakana-dbe-mode-key-suppression-regression.md) | `VK_DBE_*` KeyDown 無条件 Suppress（BUG-52対策）が Shift+かな→カタカナ変換を巻き添えで殺している（BUG-116） | 一部置換・一部撤去(2026-10-06 裏取り): 決定1(Shift 時のみ 0xF1 を Allow)は ADR-191(73877f52)で特例ごと撤去され、現行は 0xF0/0xF1 とも常に Allow。決定 |
| [138](138-ime-probe-actuation-witness-app-rejected.md) | IME probe/actuation 検証用ウィットネスアプリは Opus 敵対的レビューで却下、発信源タグ計装+最小スパイクへ縮小 | 却下(フルスコープ案)。縮小案(決定2 発信源タグ計装・決定3 スパイク)は ADR-138 を参照する実装コミットが git log … |
| [139](139-tracing-metrics-observability-migration.md) | ログ/メトリクス基盤を `log` から `tracing`/`metrics` エコシステムへ移行する | 採用・実装済み(PR #172、v2.0.0 に含まれる、2026-10-04 確認)。決定4第2項は ADR-215 で一部上書き(下… |
| [140](140-ime-probe-actuation-quiet-window.md) | IME probe/actuation の発行競合 — Step 0（診断ログ）・Step 1（排他機構）とも実装・実機確認済み | 採用・実装済み・実機確認済み(Step 0/1/1b、PR #175/#176)、v2.0.0 に含まれる(2026-10-04 コード… |
| [141](141-henkan-muhenkan-delegate-inactive-recovery.md) | 無変換/変換 delegate-to-open-axis の TurnOn 方向構造的到達不能問題（C2）の解消 | 撤去済み(delegate 機構は ADR-191 `06483afd` で撤去、2026-10-04 確認。`route_thumb_… |
| [147](147-thumb-key-delegate-defers-to-user-passthrough.md) | 無変換/変換の `delegate_to_open_axis` は、ユーザーが明示的に選んだ単独タップ「パススルー」設定に道を譲る | 撤去済み(delegate 機構〈`delegate_to_open_axis`〉は ADR-191 `06483afd` で撤去。単独… |
| [148](148-bug-report-ime-keymap-attachment.md) | 不具合報告へのIME別キーマップ/キー割り当て設定の添付 | 実装済み(Phase 1: PR #179、Phase 2: PR #181)、v2.0.0 に含まれる(2026-10-04 コード確… |
| [149](149-physical-ime-key-activation-defers-forced-set-open.md) | 半角状態での物理IMEキー単独タップによるIME ON遷移で、awase自身が`VK_IME_ON`を3回重複送信する問題 | 撤去済み(対象の随伴 warmup〈`should_send_accompanying_warmup`〉は ADR-212 P4 `03… |
| [151](151-actuation-delegate-by-default-drift-scoped-to-ownership.md) | delegate 対象キーの actuation を belief 追随のみに倒す方向（[ADR-149](149-physical-ime-key-activation-defe… | 保留(未実装)のまま、実質見送り: 前提としていた force-on(`apply_force_on_for_imm_broken`)・… |
| [152](152-keystroke-step-source-sink-pipeline.md) | 打鍵を source → sink のパイプラインとして再構成する構想 | 保留(未実装)、2026-10-04 コード確認: `StepOwnership`・`KeyStrokeStepDispatcher` … |
| [153](153-gji-keymap-aware-safe-vk-substitution-for-mode-keys.md) | 無変換/変換キー単独タップの IME ON/OFF/Toggle を、GJI 側のキーマップ設定に頼らずユーザーが awase 側で直接指定できるようにする | 置換済み・互換読込のみ残存(2026-10-05 棚卸し): 決定1 は ADR-206 で置換、自動採用(`*_delegate_to_open_axis`)は ADR-191 で撤去。`*_solo_tap_ime_ |
| [154](154-delegate-shadow-toggle-exclusivity-off-to-on-transition.md) | `delegate_owned` ゲートの排他性を OFF→ON 遷移の打鍵でも成立させる（ADR-149「案C」続報） | 撤去済み(対象の delegate 所有権判定〈`auto_delegate_open_axis_consumed`〉は ADR-191… |
| [155](155-timer-path-live-thumb-requery-during-deferred-timer-replay.md) | タイマー経路の親指タイムスタンプ問題（ADR-129 が未着手のまま残した部分）— クローズ（未実装、failure scenario 未確立） | クローズ・未実装(却下相当)、v2.0.0 時点でも変更なし(BUG-126 に記録、再オープン条件は本文参照)。 (2026-10-0… |
| [156](156-unify-deferred-execution-queues.md) | 遅延実行キューの解放条件管理 — 観察記録と軽量な対策（将来構想、大規模統合は不採用） | 不採用(大規模統合)・軽量策のみ実装済み、v2.0.0 時点でも同じ(2026-10-04 コード確認: `pending_deferr… |
| [157](157-symmetric-target-resolution-for-drift-correction-and-force-on.md) | force-ON が drift correction に道を譲る調停案（不採用・撤回） | 撤回済み(不採用)、v2.0.0 時点でも同じ。前提だった force-on 側(`apply_force_on_for_imm_bro… |
| [158](158-complexity-reduction-north-star.md) | アーキテクチャ複雑性根絶の北極星 — 記録・再生基盤／非スコープ宣言／単一仕様生成／ガバナンス反転（Bは棄却） | 一部実装(2026-10-04 確認): 北極星として子ADR 159〜164 が起票・順次実装され、ADR-159 段階0・1、ADR… |
| [159](159-existing-io-boundary-inventory.md) | 既存の送受信境界を棚卸しし、記録・再生・シャドー実行の土台にする | 一部実装(残り: 段階2 TF2 の蓄積・突合せ〈意図的に撤回し未着手〉、TH1e)。段階0(TB0〜TB2、`lints/actuat… |
| [160](160-explicit-non-scope-declaration.md) | 非スコープを決定する会議体を持つ（C1: IME一本化／C2: アプリホワイトリスト化／C3: conv-mode追跡全廃） | 見送り(2026-10-05): C1〜C3 は実施しない。再オープン条件は本文参照 |
| [161](161-single-source-spec-generation.md) | 散文の権威を剥奪し、機械可読な単一仕様から生成する＋純粋層にモデル検査をかける | 一部実装・残りは見送り(2026-10-05、所有者判断): D1 は dylint による宣言の強制(`lints/actuation_call_guard`)まで実施、表の生成機構(xtask)は見送り。D2 の p |
| [162](162-governance-reversal.md) | ガバナンスを反転する — 複雑性予算制・ADRのTTL・敵対的レビューの向き先変更 | 一部実装: ADR-158 TH2/TH3(ADR TTL・CI チェック等、`6637bed8`)は実装済み。E1(複雑性予算制)は … |
| [163](163-actuation-decision-io-separation-and-replay-harness.md) | actuation合流点の「決定」と「実I/O」の分離、および決定点ジャーナル再生ハーネス | 一部実装(2026-10-04 確認): Part A〜D(TH1a〜TH1d')実装済み・v2.0.0 に含まれる。TH1e(Part… |
| [164](164-global-static-argument-threading-plan.md) | グローバルstatic縮小 — 引数引き回し優先＋残りは単一singleton集約の段階的リファクタ計画 | ほぼ実装済み(v2.0.0 に含まれる、2026-10-04 確認): フェーズ1・2・4・5・6・8 は develop マージ済み、… |
| [165](165-tsf-cache-restore-recency-guard.md) | TsfNativeキャッシュ復元にhwnd一致を要求し、無関係な窓の誤ON復元とforce-ON誤発火を防ぐ (BUG-128) | 実装済み・実機確認済み(PR #203、`98b04e12`)、v2.0.0 に含まれる(2026-10-04 コード確認: `focu… |
| [166](166-physical-key-disposition-decision-table.md) | PhysicalKeyDisposition::plan()の全数決定表化、DBEモードキーDown/Up vk非対称ハザードの明文化 (BUG-131) | 実装済み(PR #206、windows-build CI 実行済み)、v2.0.0 に含まれる。その後 `plan()` 本体は AD… |
| [167](167-standard-profile-warmup-double-send.md) | Standardプロファイル×ImmCross失敗フォールバック時の随伴warmup重複送信 (BUG-133) | 実装済み(PR #207)、v2.0.0 に含まれる(`ImeOpenOutcome::AppliedWithoutSendInput`… |
| [168](168-actuation-boundary-small-cleanups.md) | Clojure風protocol/transducer案(型システム全面置換・決定軸registry統一)の却下記録 + ADR-088の`post_*_direct`4関数保持決定の反転 | 実装済み(PR #213、v2.0.0 に含まれる、2026-10-04 確認)。 (2026-10-04 更新) |
| [169](169-journal-key-input-repeat-coalescing.md) | journal `KeyInput`レーンのOS auto-repeat畳み込みでダンプ予算窓を圧縮する | 実装済み(決定1・1-b、v2.0.0 に含まれる、2026-10-04 コード確認: `journal.rs` の repeat_co… |
| [170](170-codesmell-hotspot-decomposition.md) | コードスメル解消: belief reduce()の大きい4分岐をprivateヘルパーへ抽出(決定1のみ実施) | 一部実装: 決定1(`ImeModel::reduce()` の分岐抽出)は実装済み(#216、`ab116864`、develop に… |
| [171](171-gji-candidate-reopen-after-off-observation.md) | `gji_direct_already_matches`が候補ウィンドウ再表示(`candidate_was_seen`)を無視して再送を握り潰す不具合を修正(BUG-141)。belief経由の自動補正案はround1/2で計8件のBlockerが出て見送り、`candidate_visible`併用案もround4でBUG-113再導入Blockerと判明し撤回 | 実装済み(案Z、`040536bf`、PR #218 で develop マージ済み)・実機A/B実施済み、v2.0.0 に含まれる(2… |
| 172 | TsfNative向けON方向救済4系統（force-on/drift/warmup/reassert）の整理。opus round1〜4で収束し「コード変更なし」で確定 | アーカイブ（タグ `archive/adr172-tsfnative-rescue-consolidation`、develop未収録・本文ファイル無し） |
| 173 | 無変換/変換ソロタップのIME動作をプロセス名で限定する `app_overrides.solo_tap_ime_action_apps`（ADR-174で却下、インフラは残置） | アーカイブ（タグ `archive/adr173-solo-tap-ime-action-by-process-name`、develop未収録・本文ファイル無し） |
| [174](174-solo-tap-passthrough-belief-reobservation.md) | 無変換/変換ソロタップでGJIが実際にIMEを開いてもEngineが追従しない問題。round1〜3(観測ベースの新設計)は全てBlockerで破綻、round4でclassify_mode_key_ime_actionのsession_keymapゲート不具合(BUG-143)と判明 | 実装・実機確認済み(`f4317675`、BUG-143、2026-09-15)だったが、v2.0.0 では置換: GJI/MS-IME… |
| [175](175-physical-dbe-key-stuck-direction-recovery.md) | 物理半角/全角キー(VK_DBE_SBCSCHAR/DBCSCHAR)の固定方向マッピングをやめToggle解決に変えIME ON固着を解消(BUG-142)。config変更のみで実機A/B確定済み | 置換(ADR-199 T4 の役割由来 `shadow_action` により置換)。本ADRの方式〈`keys.ime_detect.… |
| [176](176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md) | awase-settingsに専用較正UIを新設し、モードキーの実効果をユーザー協力の下で明示的に測定、gate_thumb_key_ime_actions出力を差し替えて静的分類を補完する。BUG-143の静的パースの限界を補完 | 撤去済み(ADR-195 学習に置換、ADR-198 決定3、2026-09-24 に撤去)。v2.0.0 にも較正UI・適用側は存在し… |
| [177](177-msi-restart-manager-graceful-shutdown.md) | 常駐中のMSIアップグレードは実機検証の結果コード変更不要と判明(Restart Managerが現状コードのまま自律的にシャットダウン・再起動を処理、UI付き・データ保持も確認)。副産物でMSIアンインストール時のユーザーデータ削除を発見 | 確定(コード変更不要、2026-09-17)、v2.0.0 時点でも同じ。 (2026-10-04 更新) |
| [178](178-msi-uninstall-preserve-userdata.md) | MSIアンインストール時のユーザーデータ喪失をPermanent="yes"+アプリ側自己修復(無ければ埋め込み既定値から生成)で防ぐ。v1〜v13の「バックアップ+復元」方式(12ラウンド・Blocker20件)は複雑化しすぎたため破棄し全面差し替え | 実装済み・実機確認済み(v14、2026-09-17): `wix/main.wxs` に `Permanent="yes"`・`Nev… |
| [179](179-mode-key-actuation-follow-only-vs-toggle-ownership.md) | 無変換/変換の非親指キー時actuation-autoを撤去し`ModeKeyActuationOwner`列挙へ統一。元178番、developマージ済みの別ADR-178(msi-uninstall)と衝突し179へ採番し直し | 一部実装・中核撤去(2026-10-04 確認): 決定1・2 は実装済みだったが、決定2 の `ModeKeyActuationOwn… |
| [180](180-actuation-gate-recheck-deduplication.md) | 領域B(IME actuation合流点)の深い統一を検討、ADR-106決定5が既に軸統合を却下済みと判明し「新fence型ではなく共有ヘルパー関数への機械的重複除去」に縮小 | 実装済み(decision1: `is_input_relay()` へのゲート再検証統合、`c8bc1adc`、v2.0.0 に含まれ… |
| [181](181-gji-atok-keymap-hiragana-key-external-echo-reverts-ime-off.md) | GJI(ATOKキーマッププリセット)がVK_DBE_HIRAGANAを自己注入マーカー無しで周期送信し、IME OFF直後にkp_stage_shadow_ime_toggleが誤って物理意図として再actuateしIME ONへ戻る不具合 | 見送り・却下相当(2026-10-05): 現行コードに本文の機序なし。再オープン条件は BUG-187 |
| [182](182-char-then-thumb-gap-gate-misjudges-modekey-chord-as-solo-tap.md) | 文字→親指(無変換/変換)の押下間隔が閾値をわずかに超えると重なったチョードが「文字単独+無変換単独タップ」に割れ、生の無変換がGJIへ届いて半角英数化・エンジン非活性へ連鎖する不具合 | 一部実装(決定1・1b・1c は実装済み・実機A/B確認済み〈2026-09-19、BUG-145、f2eb1efe/17890b87/… |
| [183](183-vk-kana-physical-delivery-passthrough.md) | VK_KANA(かなキー)をADR-179の`PhysicalDelivery`へ合流させKeyUp無条件Suppressの非対称を解消する設計 | **撤回済み（2026-09-19、実機検証により前提誤りと確定）**。opus-adversarial-consult |
| [184](184-gji-atok-muhenkan-toggle-awase-owned-eisu-hiragana.md) | GJI(ATOKキーマップ)の無変換/変換Toggleを、ADR-179決定2の既存分岐へ配線し直すだけの最小修正 | 置換(ADR-186〈実機マトリクスで前提『IME ON のまま半角英数』が入力中のみ正しいと訂正〉・ADR-191〈delegate_… |
| [185](185-directinput-open-axis-write-teardown.md) | 半角英数(ObservedEisu)検出時にawase自身がIME OFFを送る`EngineSync::DirectInput`を撤去(BUG-146、ADR-179（旧178）撤去プロジェクトの領域C) | 実装済み(f5338edc、v2.0.0 に含まれる、2026-10-04 確認。EngineSync::DirectInput は現行… |
| [186](186-gji-atok-mode-key-measured-matrix-and-belief-follow.md) | GJI(ATOK)のモードキー動作を実機で測定し、無変換/変換の開閉トグルを「KeyUpで解決する」既存delegate経路で押下時点にbelief追随させる(実機E2E+撤去実験で必要/不要な仕組みを確定) | **v4(実装済み・実機E2Eで検証、撤去実験の結果を反映)**。実装ブランチ`feat/adr186-nonconvert-toggl… |
| [187](187-atok-passthrough-mode-key-observed-belief-follow.md) | ATOKで無変換/変換をパススルーするとき、生キー通過直後に実IMEを読み直し、古い明示意図を捨ててEngineを観測に追随させる(follow方式、awaseはactuateしない)。Toggleをactuateする案(PR #227)はcomposingが推定でしかないため見送り | **決定・実装済み(developマージ済み: `c949ba33`)**。スパイク(`spike/adr187-follow-obse… |
| [188](188-tsfnative-conv-only-mode-key-engine-follow.md) | Chrome等(Imm32Unavailable/TsfNative)で、convだけを変えるモードキー(ひらがな、Shift+無変換)の後にEngineを追随させる(モードキー後の遅延conv読み取り、実機A/Bで4案を比較) | 未実装(v2.0.0 時点、コード確認: 実験パッチ e3 の『モードキー後の遅延 conv 強制チェック』に当たる実装は無い)。BUG… |
| [189](189-gji-hankaku-zenkaku-belief-toggle.md) | GJIの半角/全角キー(0xF3/0xF4)をVKで方向を決め打たず、beliefに基づく開閉トグルとしてawaseがactuateする(同じVKの連続で反転しない問題、CI `--hz`で4/8手順) | **[ADR-199 で役割判定に一般化（T7、2026-09-27）]** 本ADRの「VK基準で常にトグル」は、[ADR-199](… |
| [190](190-msime-immcross-failure-fallback-idempotent-vk-ime-on.md) | Microsoft IMEでImmCrossが失敗したとき、非冪等なVK_KANJIトグルでなく冪等なVK_IME_ON/OFF(MsImeDirect)へフォールバックする(BUG-152) | **実装済み(developマージ済み: `feb49ffd`)・CI実機E2Eで検証済み**。opus round1〜3で収束(rou… |
| [191](191-ime-is-source-of-truth-observe-not-write.md) | IMEの状態はIME自身を正とし、awaseは書き込まず観測・予測に追随する（開閉のみに作用するキーは例外）。キー効果は設定読取・注入学習・検証の3段階で表にする | 実装済み(撤去ブランチ PR #240 ほか、v2.0.0 に含まれる、2026-10-04 確認)。決定6 の EngineDecis… |
| [192](192-state-dependent-mode-key-warning-and-guided-override.md) | 状態依存のIMEモードキーを検出して警告し、awaseの明示config（冪等なON/OFF）への置き換えを案内する | **ADR-206（2026-09-29）追記: 決定3b の優先順位2（`*_solo_tap_ime_action`）は撤去した。G… |
| [193](193-richedit-superclass-tsf-native-e2e-target.md) | TSFネイティブ相当の入力先を RichEdit のスーパークラス化で決定的に用意する（実機E2Eの検証対象拡張） | 採用・実装済み(スパイク成功に加え CI 配線済み: .github/workflows/e2e-ime.yml の入力先 tsf=Ri… |
| 194 | IME時間依存ロジックのシミュレーション・リプレイハーネス（`feat/ime-sim-harness`ブランチのみに存在、develop未マージ） | 破棄（Opusレビューで既存単体テスト以上の実証価値なしと判明、ユーザー判断で試作破棄。**番号194は本行で予約のみ**、developにファイル無しのためリンクなし） |
| [195](195-keymap-learn-productization.md) | カスタムキーマップ対応のため、IMEキー効果の学習(awase-keymap-learn)を独立プロセスとして製品化する（設定読取→独立プロセスでの巡回学習→自己検証→永続化→実行時読込→ADR-176統合） | 実装済み(段階0/1/2/3/4/5/6/8 は develop マージ済み〈PR #250〜#258〉、v2.0.0 に含まれる。段階… |
| [196](196-keymap-learn-truth-priority.md) | ADR-195の段階4/6/8を修正し、既知プリセット構成でも内蔵表を審査官にせず学習結果を優先する。陳腐化は失効でなく要再検証とし、フィンガープリントにIME本体バージョンを追加する | 実装済み(v2.0.0 に含まれる)。『採用の仕組みのずれ』は PR #305(5%判定の廃止・カバレッジ分母の修正、docs/task… |
| [197](197-msime-legacy-custom-keymap-runtime-warning.md) | MS-IME旧UI(互換モード限定キーカスタマイズ)の調査。実行時警告は前提(無変換キーへのCEトグル)が実機で否定され撤回、ADR-196向け互換モードフラグ読み取り(決定4)のみ採用 | **部分撤回済み（決定1〜3、2026-09-23）・決定4のみ実装済み。** |
| [198](198-persistence-destination-classification.md) | 永続化先の分類(config.toml/cache.toml/学習表JSON)とv2でのcalibrationの扱い | 採用・実装済み(決定3 手動キャリブレーションの廃止=PR #304、AppConfig::calibration 撤去済み、決定2 学… |
| [199](199-derive-key-roles-from-user-ime-keymap.md) | キーの役割をユーザーのIMEキー設定から逆算し原則受動、能動はIME ON/OFFトグルの役割のキーと awase の ime_on/off 設定だけ | 実装済み(v2.0.0 に含まれる)。T1〜T13・T16・T17 の主要部分は develop 実装済み、T14 は撤回し ADR-2… |
| [200](200-reinit-must-not-run-during-live-composition.md) | chrome-reinit(VK_IME_OFF→ON)は SuspectedLiteral の証拠が2回そろったときだけ送る(StaleConfirm では reinit しない) | 撤去済み(決定1 が対象とした give-up 後の VK_IME_OFF→ON reinit は ADR-212 P3〈PR #402… |
| [201](201-config-key-resolution-and-load-diagnostics.md) | 設定のキー名解決を from_name に集約して寛容にし、握りつぶしを既存診断へ流す。toml_edit保存(三者比較)・GUI候補×読み手のCI検証 | **採用・実装済み（2026-09-26 所有者承認。opus round1〜4 で収束。段階0〜3を同日実装）。** 段階0=PR #… |
| [202](202-kanji-0x19-role-from-hankaku-zenkaku-row.md) | 0x19（Alt+半角/全角）を GJI では CUSTOM 表の Hankaku/Zenkaku 行から役割判定する（ADR-199 決定14 の実装設計） | 採用・実装済み（T16-1〜3・5・6）
| [203](203-gji-fsm-follows-belief-open-transitions.md) | GjiFsm を belief の開閉変化に追随させる(BUG-170、OffCold 固着→毎打鍵 per-VK→StaleConfirm→ESC) | 実装済み(2026-09-29)・CI 検証済み(e2e の sc-reopen-*/--require-sync が GjiFsm 同… |
| [204](204-gji-sync-obligation-cannot-be-silently-dropped.md) | ImeOpenOutcome の処遇の重複を1メソッドへ集約し legacy_gji_sync_obligation を網羅化する最小版(初稿の D2/D3/D4 は取り下げ) | 採択(縮小版)・未実装のまま(v2.0.0 時点、コード確認: ImeOpenOutcome::ALL は存在せず、legacy_gji… |
| [205](205-observe-external-ime-close-in-imm32-unavailable-windows.md) | Imm32Unavailable(Chrome)で外部注入の IME キーによる close を窓内の 1→0 遷移観測で実状態へ追随する(BUG-172) | 採択・実装済み(PR #377 fd41bf88、v2.0.0 に含まれる)。CI 検証済み(GJI×実 Chrome で追随 10/1… |
| [206](206-thumb-solo-tap-follows-role-suppress-and-inject.md) | 無変換/変換の単独タップ再設計(役割があれば生キー抑止＋belief に従う明示注入、なければ Suppress/Passthrough。`*_solo_tap_ime_action` 撤去) | 実装済み(PR #376 ほか、v2.0.0 に含まれる)。実機確認は X1 を 2026-09-30 に実施: 既定(Suppress… |
| [207](207-keys-ime-detect-default-empty-and-remove-engine-ime-keys.md) | keys.ime_detect.on/off の既定を空に、engine_on/off_ime_key(Engine ON/OFF 時の IME モードキー能動送信)を撤去 | 採択・実装済み(2026-09-29、v2.0.0 に含まれる)。CI/実機での確認(MS-IME ジョブ、フォーカス変更直後の物理 0… |
| [208](208-absolute-ime-keys-must-not-be-elided-on-stale-applied-in-blind-windows.md) | 明示的な IME キーを押すと、内部状態が何であっても最大2回の押下で実 IME が一致する(固着ゼロの保証) | 一部実装(L0・L1・L2・L3a 実装済み。L3 の Chrome〈Imm32Unavailable〉適用は L1+L3a で成立済み… |
| [209](209-predict-open-effect-from-gji-config-when-not-learned.md) | GJI の MS-IME プリセットでは TSF の窓で変換が IME を開く(モードは閉じる前のまま)。読めない窓の打鍵時予測に「変換で開く」を足し、素通しの変換に Engine を追随させる(ADR-186/191/199/206) | 実装済み(2026-09-30、設定 general.predict_henkan_open_in_unreadable_windows… |
| [210](210-learned-table-hidden-state-converting-and-last-key.md) | 学習表の状態に「変換中」と「直前キーの文脈」を加える | 保留(2026-09-30)。Opus 敵対的レビュー(1周目)で、決定1・2は現案のままでは採用不可と判明した(評価方法が学習データ内… |
| [211](211-predict-open-close-from-gji-keymap-for-passive-keys.md) | GJI の MS-IME/MOBILE プリセットの F13(閉状態からだけ IME を開く受動のキー)で Engine が追随するよう打鍵時に「開く」と予測する。キーマップ全体からの一般化は需要確認まで見送り(実 Chrome CI の実測、Opus round1、ADR-199/209) | 採用(2026-09-30)。Opus round3 で収束(新しい Major なし。Minor m8〜m11 は反映済み)。決定1・… |
| [212](212-remove-preventive-and-corrective-ime-actuation-in-phases.md) | ユーザー操作を引き金にしない予防的・補正的な IME への書き込みを段階的に撤去する | ほぼ実装済み(v2.0.0 に含まれる)。P0・P1・P3・P4・P5・P6(b)(c) は develop 実装済み、P2〈Activ… |
| [213](213-shadow-toggle-off-to-on-explicit-actuation-then-remove-activation-sync.md) | shadow toggle の OFF→ON を明示 actuation にし、ActivationSync を撤去する(ADR-212 P2 再開) | 実装済み(P2a〜P2c・P2d-1・P2d-2 が v2.0.0 に含まれる。ActivationSync は現行コードに無い、202… |
| [214](214-split-sent-from-confirmed-and-declare-skip-eligibility-per-path.md) | IME への書き込みの「送った」と「観測で確認した」を型で分け、送信を省略してよい根拠を経路ごとに宣言する | 保留(2026-10-02、所有者判断)。決定0(経路の確認)まで実施し、P1(トレイ起点の Engine コマンドの `SetOpen… |
| [215](215-derive-variant-name-and-display-tables.md) | variant 名・Display の手書き対応表を strum/thiserror の derive に置き換える(ADR-139 決定4の一部を上書き) | 実装済み・developマージ済み(PR #427 a612a832、v2.0.0 に含まれる)。 |
| [216](216-remove-diagnostic-only-open-belief-and-unread-apply-arguments.md) | 診断ログ専用の OpenBelief と、読まれない applied の時刻・常に None の引数を撤去する | 実装済み(R1〜R4、PR #428 ce532490 でマージ済み、v2.0.0 に含まれる)。windows-build の E2E… |
| [217](217-remove-dead-compat-and-unused-parameters.md) | 後方互換の名目だけが残るコード・未使用引数・古くなった dead_code allow を撤去する | 実装済み(A・B・C1・C3、PR #431 でマージ済み。続きの PR #432 で Linux dead_code・awase-se… |
| [218](218-shared-helper-for-architecture-guard-call-site-pins.md) | architecture_guard の呼び出し元固定ガードの重複を共通ヘルパー1本で除く(宣言テーブルは見送り) | 提案のまま未実装(v2.0.0 時点、コード確認: assert_production_call_sites は architectur… |
| [219](219-engine-test-helpers-instead-of-scenario-dsl.md) | エンジンテストは既存ヘルパーの使い回しと純粋な対応表の撤去で読みやすくする(DSLは見送り) | 提案のまま未実装(v2.0.0 時点、コード確認: ms() ヘルパーは src/engine/tests.rs に無い)。テキスト D… |
| [220](220-key-name-tables-keep-as-is-add-capture-table-test.md) | キー名表の単一ソース化は見送り、キャプチャ表の検証テスト1本だけ足す | 見送り(2026-10-02)。D2 のテスト(`egui_capture_names_are_accepted_by_their_re… |
| [221](221-msime-ime-off-composition-loss-measure-first.md) | MS-IME の英数キー IME OFF で未確定文字が消える件は、修正の前に OS 側の挙動を実測する | 調査完了(2026-10-04)。D0/D1 の実測で現行機構では未確定文字の消失は再現せず。修正は選ばない。別セッションの OFF 補… |
| [222](222-bug-report-log-gzip-ring-dump.md) | 不具合報告のログは ring の中身を gzip して送る(打鍵は最低10分) | 採用・実装済み(2026-10-04、PR #451、Opus round1/round2 反映・所有者決定: gzip・プレビュー読み… |
| [223](223-input-language-change-detected-at-key-time.md) | 入力言語の切替を、打鍵の時点でフォーカス窓のスレッドの言語を読んで検知する(案C)。表示の即時更新は切替キー解放後に 1 回だけ読む(案E2)。購読もポーリングも使わない | 一部実装(段階 0 の測定は合格〈PR #452〉、段階 1 = 打鍵の取り込み時に読んだ入力言語で is_japanese_ime を… |
| [224](224-closed-loop-hub-ungate-or-extract.md) | 閉ループが写している ImeStateHub の配線を、ungate(案A)か純粋関数への切り出し(案C)か。見逃しが出るまで着手しない | 起草(2026-10-04、未決定・実装なし) |
| [225](225-journal-replay-ci-closed-loop-integration.md) | 報告 journal・CI 実機・閉ループ・replay の連携は、まず実害を測り(SP0)、抽出手順の縮小版だけを候補に残す | 見送り(2026-10-04、SP0: 直近10件で検知可能 0 件・弱い候補 2 件) |
| [226](226-ci-realmachine-closed-loop-oracle-sharing.md) | CI 実機・閉ループ・replay の接続(oracle 共有 A〜D は見送り、ログ構造化 E は未レビュー) | A〜D 見送り(2026-10-04、Opus round1)。E は未決定 |
| [227](227-bug074-giveup-as-closed-ime-evidence.md) | BUG-074: give-up で文字が痕跡なく消える件。先に測り(D0)、方向(再オープン/案K/通知/追随)は所有者が決める | (i) 実装済み(PR #480・`900a5f3e`、2026-10-05、CI 検証済み・実機未検証): `Imm32Unavailable`×GJI の give-up を読み直しのきっかけにする(`take_gi |
| [228](228-bug-report-legacy-msime-custom-key-table.md) | 不具合報告に旧互換 MS-IME のキー表の中身を、許可リスト+固定長の読める形で添付するか(ADR-148 の「読めず削れない」反対理由への答えが要る) | 見送り(2026-10-04、Opus r1: 案 B は動機の 2 件の調査を前に進めない・実装なし) |
| [229](229-os-independent-core-and-dumb-windows-executor.md) | awase-windows を「Windows の dumb な実行役」に絞り、判断を Linux でテストできる側へ寄せる層の引き直し | 起草(2026-10-06)。Opus round1(着手単位の見定め)反映。着手しやすい単位(T2・T1・T3・T5・T4・T7)だけ先行、他は未着手 |
| [229](229-os-independent-core-and-dumb-windows-executor.md) | awase-windows を dumb な実行役に絞り、判断を Linux でテストできる側へ寄せる層の引き直し(ポート S/B/A、同一 crate 内の ungate 先行、契約テスト)。ADR-224 の着手条件は未達なので所有者判断が要る | 起草(2026-10-06、未レビュー、実装なし) |

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

## 補助資料（番号付きADR系列ではないもの）

> Opusレビュー記録は `review/` サブディレクトリに置く（2026-10-05、番号重複の解消）。

番号付きADR本体ではないが `docs/adr/` 配下にあり、frontmatterを持つファイル:

| 資料 | 種別 | 関連ADR |
|---|---|---|
| [ADR-001-architecture-history.md](ADR-001-architecture-history.md) | アーキテクチャ変遷の記録文書（`001`はADR-001とは無関係の別ファイル） | - |
| [114-implementation-tasks.md](114-implementation-tasks.md) | ADR-114 実装タスクリスト | [114](114-keymap-app-scoped-shortcut-wiring.md) |
| [158-implementation-tasks.md](158-implementation-tasks.md) | ADR-158〜162 実装タスクリスト | [158](158-complexity-reduction-north-star.md) |
| [158-complexity-inventory-2026-09-10.md](158-complexity-inventory-2026-09-10.md) | ADR-158 複雑性インベントリ | [158](158-complexity-reduction-north-star.md) |
| [163-implementation-tasks.md](163-implementation-tasks.md) | ADR-163 実装タスクリスト | [163](163-actuation-decision-io-separation-and-replay-harness.md) |
| [176-implementation-tasks.md](176-implementation-tasks.md) | ADR-176 実装タスクリスト | [176](176-behavioral-calibration-of-ime-mode-key-shadow-overrides.md) |
| [193-implementation-tasks.md](193-implementation-tasks.md) | ADR-193 実装タスクリスト（Chrome idle-sweep E2E、保留・参考） | [193](193-richedit-superclass-tsf-native-e2e-target.md) |
| [191-calibration-experiments.md](191-calibration-experiments.md) | ADR-191 較正・予測の実験の経緯と実測結果（格子・通知購読・CI高速化・文献調査・巡回シミュレータ） | [191](191-ime-is-source-of-truth-observe-not-write.md) |
| [191-gji-state-scope-spec.md](191-gji-state-scope-spec.md) | ADR-191 GJI/MS-IME の開閉・変換モードの保持範囲（仕様調査：Mozc読解とCI実測） | [191](191-ime-is-source-of-truth-observe-not-write.md) |
| [review/178-opus-review-round1.md](review/178-opus-review-round1.md)〜[round12.md](review/178-opus-review-round12.md) | ADR-178 v1〜v13（バックアップ+復元方式、破棄済み）敵対的レビュー記録（Opus round1〜12） | [178](178-msi-uninstall-preserve-userdata.md) |
| [review/178-opus-review-v14.md](review/178-opus-review-v14.md) | ADR-178 v14（Permanent+自己修復方式、現行）敵対的レビュー記録 | [178](178-msi-uninstall-preserve-userdata.md) |
| [review/183-opus-review-round1.md](review/183-opus-review-round1.md) | ADR-183 Opus敵対的レビュー記録（round1） | [183](183-vk-kana-physical-delivery-passthrough.md) |
| [review/184-opus-review-round1.md](review/184-opus-review-round1.md)〜[round6.md](review/184-opus-review-round6.md) | ADR-184 Opus敵対的レビュー記録（round1〜6） | [184](184-gji-atok-muhenkan-toggle-awase-owned-eisu-hiragana.md) |
| [review/186-opus-review-round1.md](review/186-opus-review-round1.md)〜[round2.md](review/186-opus-review-round2.md) | ADR-186 Opus敵対的レビュー記録（round1〜2） | [186](186-gji-atok-mode-key-measured-matrix-and-belief-follow.md) |
| [review/187-opus-review-round1.md](review/187-opus-review-round1.md)〜[round2.md](review/187-opus-review-round2.md) | ADR-187 Opus敵対的レビュー記録（round1〜2） | [187](187-atok-passthrough-mode-key-observed-belief-follow.md) |
| [review/190-opus-review-round1.md](review/190-opus-review-round1.md)〜[round3.md](review/190-opus-review-round3.md) | ADR-190 Opus敵対的レビュー記録（round1〜3） | [190](190-msime-immcross-failure-fallback-idempotent-vk-ime-on.md) |
| [review/193-opus-review-round1.md](review/193-opus-review-round1.md)〜[round3.md](review/193-opus-review-round3.md)、[plan-round1.md](review/193-opus-review-plan-round1.md)〜[plan-round5.md](review/193-opus-review-plan-round5.md) | ADR-193 Opus敵対的レビュー記録（本体3ラウンド＋計画5ラウンド） | [193](193-richedit-superclass-tsf-native-e2e-target.md) |
| [review/196-opus-review-round1.md](review/196-opus-review-round1.md)〜[round5.md](review/196-opus-review-round5.md) | ADR-196 Opus敵対的レビュー記録（round1〜5） | [196](196-keymap-learn-truth-priority.md) |
| [review/197-opus-review-round1.md](review/197-opus-review-round1.md) | ADR-197 Opus敵対的レビュー記録（round1） | [197](197-msime-legacy-custom-keymap-runtime-warning.md) |

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
  吸収されて消えた（ADR-038参照）
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
