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
| [091](091-idempotent-charset-axis-gji-recommended-msime-self-responsibility.md) | 冪等キー中心のIME制御 — open/romaji軸は既存対応を追認。charset軸(かな形状)は新しいbeliefを持たず、config1.dbの自動判定を主UXにベストエフォート助言を行う3層構成に収束(`CharsetSlot`によるbelief駆動の絶対制御機構は5ラウンドのpre-mortemと実機検証を経てMS-IME→GJIへ付け替えたのち最終的に不採用)。GJI向けは無変換単独打鍵を専用Fnキー(F21、Composition/Conversionのみ`SwitchKanaType`)へ変換する構成をconfig1.db自動判定で有効化、MS-IME向けは無変換=IME ON/OFFカスタマイズ済みなら決定1のopen軸機構で肩代わり、既定のまま素通しならGJI利用を推奨するポップアップを出す | **決定・実装未着手**（charset軸のSendInputによるGJI到達性・`ImmGetConversionStatus`での確認可能性はF15-F19の実機検証で証明済みだが、採用構成はF21 1キーのみの新設計。MS-IME肩代わり機構は未実装）。**2026-09-02: 自動判定・設定支援ポップアップ・config1.db書き込みは実装後に出荷されたが、実機での誤診断・ユーザー混乱を受けて全撤去(手動設定のみ残存、詳細はADR本文の追記参照)** |
| [092](092-external-key-semantics-absorption-and-thumb-key-restructure.md) | 2026-07-06「MS-IME二重オーナー問題」分析の積み残しを正式設計化。宣言ソースをMS-IMEレジストリ(`KeyAssignmentMuhenkan`/`Henkan`/`CtrlSpace`/`ShiftSpace`の4キー固定)とGJI config1.db(`read_gji_ime_keys`、round1懸念5-1が指摘した「消費者不在」機構に初めて配線。**4キーに限定せず任意VKを対象**、VK種類による安全フィルタ=親指キー役割/非かな生成/かな生成の3分類でルーティング)の両方に対応、`ShadowImeAction{TurnOn,TurnOff,Toggle}`で統一解釈し`UserIntentSource::SyncKey`(`PhysicalImeKey`とは別のwitness、architecture_guard衝突を回避)経由でactuation(冪等キーVK_IME_ON/OFFへの変換送信)まで行う。宣言に「揺れ」(矛盾)を検出した場合は警告+素のパススルーへフォールバック。親指キー8bool設定はTextKeyConfig/ModeKeyConfig(idle/composing総関数)へ再編。優先順位「明示>自動>既定」(R1-R4)を明文化 | **Step1・Step2・Step6実装済み(2026-08-15、Opusコードレビュー2巡反映済み)。Step3-5は次フェーズへ先送り**。engine_on/off_ime_key既定None化(Step1、実機確認は残タスク)、ThumbKeySoloTapGuard→ModeKeyConfig/TextKeyConfig再設計(Step2、config.toml非破壊のfrom_legacy_boolsブリッジ方式でserde移行案から意図的逸脱)、IME検出タブをキー設定タブの上級者向け折りたたみへ統合(Step6)。round1 Opusレビュー→round2ユーザー判断で決定4を着手対象へ格上げ、実機レジストリ確認済み。2026-08-15実機(dragonflyg4)でKeyAssignmentCtrlSpace/ShiftSpaceの実在と、「IME ON/OFF」割当て時の値が4キー共通で`2`(Toggle相当)であることを確認。Step 4a(Ctrl+Space/Shift+Space)・Step 4b(無変換/変換、NicolaFsm Phase3への新分岐)・Step 4c(GJI/MS-IME両ソースの読み取り配線)に分割。GJI側の「かな生成VK判定」ロジックは未実装、BUG-64級のリスクがあるためStep 4c着手前に固める必要あり。**決定A-4追加**: `session_keymap`がCUSTOM以外(ATOK/MS-IME/ことえり等プリセット)の場合はGJI config1.dbではなくMozc公式ソース(`google/mozc` `src/data/keymap/*.tsv`)の組み込みプリセットデータで意味解決(ATOK=DirectInput→IMEOnは単純、Precomposition→CancelAndIMEOffはComposition破棄+IME OFFの複合コマンドで2巡目レビュー指摘によりPrecomposition状態限定に修正、MS-IMEの無変換はcharset軸複合コマンドでADR-091のF21領域に残り対象外、ことえりは該当定義なし。取得元コミットハッシュは未記録で実装前に埋める必要あり)。**「IME検出」独立タブは廃止し`tab_keys`へ統合、「IME制御」→「awase→IME ON/OFFキー」・`ImeDetectConfig`→「IME→awase ON/OFFキー」と送受信の向きが伝わる対称名に改称し、両者を上級者向け折りたたみ(CollapsingHeader)にまとめる**(round4。設定自体を廃止する案はサードパーティIME等で唯一の設定手段を失うため不採用、名称の曖昧さとPCベストプラクティス不足を自己点検して修正。toggle/on/off 3リストは折りたたみ内で個別表示を維持。Step 6、GUIリファクタのみでStep 4の自動判定実装を待たず先行着手可)。**`SettingSource`表示の具体化**: 各キーエントリに`AutoDetected{from}`=青バッジ/`Manual`=緑バッジ/`Default`=グレーバッジ(既存の「変更あり/保存済み」色分けパターン踏襲)、セクション先頭に`color_legend`形式の凡例、`AutoDetected`値の直接編集は即座に`Manual`へ切替、「自動判定に戻す」ボタンで復帰可能。`from`はユーザー向けには内部名(config1.db等)ではなく製品名で表示(「自動: Microsoft IME設定」「自動: Google日本語入力設定」「自動: ATOK設定」、`from`識別子自体はADR本文・コードでは技術名のまま)。`SoloTapAction::DedicatedFnKey`自動選択にも同じバッジ規約を横展開。**決定A-5は2巡目Opusレビューで撤回・大幅縮小**: 複合副作用キー(ひらがな/カタカナ/全角/英数/半角)の観測を`ImeDetectConfig`へ追加する案は、`vk.rs::ImeKeyKind::shadow_effect()`が既に同じ判定を実装済みの重複であり、かつ`key_pipeline.rs`の優先順位規則により`is_japanese_ime()`ゲートを迂回し既存より強いwitnessへ格上げする安全性後退だったと判明(round1が指摘した「既存機構を見落として同型の型を作る」誤りの再発)。ADRの寄与を「GUI候補リストへ半角/全角の2つを追加するのみ」に縮小し、`ImeDetectConfig`へのコード変更・既定値追加は行わない。**決定Cにも修正**: `Vec<String>`設定は要素単位でSettingSourceが異なりうるため`Manual`は`config.toml`永続化・`AutoDetected`はIME種別確定イベントごとのライブ計算という二層設計に変更(R5追加: Manual編集への即時切替を明文化)、これにより「自動判定に戻す」ボタンの復帰先も自然に確保。**決定Bにも修正**: Alt親指なりすまし時の`modifier_key.is_some()`優先ガードが実効表から漏れていたため表の外側の最優先ガードとして追記。自己評価は一時5〜6/10まで下がったが、M1-M6反映後7/10で維持。決定A-5の検討中に見つかった`is_japanese_ime()`ゲートの精度不足は別軸の問題としてADR-093へ切り出した |
| [093](093-dbe-hotkey-observation-upgrades-japanese-ime-belief.md) | ADR-092決定A-5の検討中に発見した別軸の穴。`VK_DBE_HIRAGANA`等5つのIME専用合成VK(0xF0-0xF4)は意味が確実(`vk.rs::ImeKeyKind::shadow_effect()`)なのに、それを採用する前提条件`is_japanese_ime()`がスリープ復帰/フォーカス変更直後のgrace期間中に一時的にfalseを誤答する既知の弱点(`key_pipeline.rs:1940-1943`)があり、その間の観測が黙って捨てられる。ゲートを迂回する(`SyncKey`経由、BUG-51追補3の優先順位昇格リスクを引き込む)のではなく、この5VKの受信を`is_japanese_ime()`の即時true更新トリガーに追加してゲート自体の精度を上げる方針(既存の非対称信頼パターン=trueはいつでも即時反映・falseへのダウングレードのみgrace中に抑制、を踏襲) | **実装済み(2026-08-15)**。`vk.rs::is_synthetic_dbe_ime_hotkey`を追加し`key_pipeline.rs::kp_stage_shadow_ime_toggle`冒頭(BUG-14注入イベント除外より前)で配線。architecture_guard等の件数固定テストへの影響は無し(394+34+22件パス)。実機でのgrace期間中の誤答訂正確認は未実施。CJK他言語IME由来の可能性はプロジェクトのスコープ外として考慮しない判断済み。自己評価7/10 |
| [094](094-charset-axis-and-force-policy-removal.md) | ユーザー要望によりcharset軸(ひらがな/カタカナ×全角/半角)の追跡自体を撤去。ADR-091決定3 §D3.1の原則を実装面でも徹底し、§D3.3が明示的に容認していた`ConvModeMgr`/`has_katakana`の既存観測利用も撤去対象に含めた(BUG-50の原因2がBUG-52と判明したことによる状況変化)。`conv_mode_policy`(observe/force)設定と、それに連動していたADR-086 Phase 2(conv軸)/Phase 3(open/close軸)のforce-write機構を全撤去。eisu/かな二値境界(`state/eisu_recovery.rs`)はADR-091決定3 §D3.4通り別軸として維持 | **実装済み(2026-08-17)**。build/test/clippy(Linux `--lib`+Windows `cargo xwin`)・fmt全緑。Windows実機での動作確認は未実施 |
| [095](095-tray-bug-report-cloudflare-intake.md) | タスクトレイから不具合報告(症状発生時の内部状態を自動添付)を送信する機能。受け口にGitHub Issuesは使わず、`report.awase.cc`をCloudflare Workers+R2の非公開受付として新設(`awase.cc`は既にCloudflare権威DNS配下のためゾーン移管不要)。GCPは無料枠超過時のfail-closed特性でCloudflareに劣ると判断し不採用。Opus round1レビュー(must-fix 8件)を受けユーザー判断でround2化: journalの生打鍵列はマスキングせず送信前プレビュー必須化で対応、ログ添付は既定ON、Turnstileはネイティブアプリ非対応のため不採用しレート制限+サイズ上限のみ。送信主体分離・R2書き込み専用トークン・ペイロードallowlist型化はエンジニアリング判断として決定。codex CLIで`services/report-worker/`(Worker)と`crates/awase-{windows,settings}/src/bug_report.rs`(トレイUI)を実装、Claudeが検証・修正(clippy borrow_as_ptr 4件・TS型エラー1件)しdevelop起点のADRブランチへマージ。Cloudflare実デプロイも完了(R2 `awase-report-bucket`+90日lifecycle・KV `RATE_LIMIT_KV`作成、`report.awase.cc`は手動CNAME(proxied)+Worker routeで疎通、有効ペイロードでのエンドツーエンド疎通確認(201+R2書き込み)済み。R2有効化にカード登録は不要と実地確認)。**決定7・8(2026-08-19追加)**: クラッシュレポーターと異なりawaseのバグはサイレントに違う動作をするため自由記述の価値は残しつつ、症状カテゴリ(10択)を選ぶだけの最小操作送信を可能にした(自由記述は任意化、「その他」選択時のみ必須)。あわせてIME製品名(既存TSFプロファイル列挙のキャッシュ、新規COM呼び出しなし)・keyboard_model設定・Windowsキーボードレイアウト・競合ソフトウェア検出(ADR-060再利用)を追加。schema_versionを2に上げ再デプロイ、エンドツーエンド疎通確認済み | **実装済み・Cloudflare実デプロイ済み(schema v2)**(2026-08-19。Windows実機でのタスクトレイ操作確認のみ未実施) |
| [096](096-journal-priority-tiers-multi-lane-ring-buffer.md) | ADR-095実装後、known-bugs.md全68件+experiments.md全15件を通読し診断に効いた観測値を8カテゴリで集計、`journal.rs`と突き合わせて3つの取りこぼしを発見: (1)GJI/TSF warm-cold・probeタイミングが最頻出診断根拠なのに0%収録、(2)アプリ名(process_name)がFocusChangedにHwndIdしかなく0%収録、(3)`RawKeyEvent.injected`(BUG-08/14等の決め手)がKeyEventSummaryに未コピー。単一VecDeque(容量2048)を重要度別4レーン(state/timing新設/actuation/key_input、容量1024/512/512/512、seq共有・マージ出力でJSON外形は不変)に分割する方針を決定。GjiFsm/TSF層はjournal非依存を維持しplatform.rs側から前後状態を記録、`ImeEvent`本体は変更せずfocus_tracking.rsから新設JournalEntryへprocess_nameを直接記録、という形でレイヤー境界(layer-boundaries.md)を侵さない設計。WindowsPlatformはjournalを直接持たないため保留キュー+drain方式で配線。**round2(2026-08-19)**: Opusアドバーサリアルレビュー(「過去バグ検証に十分な情報があるか」含む)でmust-fix1件+should-fix4件(B-1〜B-5)を発見・Opus設計([docs/design/journal-diagnostic-fidelity-fixes.md](../design/journal-diagnostic-fidelity-fixes.md))に基づきcodexで是正。B-1(must-fix): bug_report.rsが添付ログの先頭(最古)を残し症状発生直前を切り捨てていた致命的バグを、直近seqから予算内に収めるcapped JSONシリアライザに修正。B-2/3: FocusTransition.fromが常にNoneだった問題とプロセス変更時のみ発火だった問題を是正。B-4: 保留キューがdrain時刻で採番され因果順が乱れる問題をJournalStamperで是正。B-5: 無変化probe tickの無条件記録を抑制。**round3(2026-08-19、同一PR)**: round2レビューが「次に大きい穴」と評価したliteral-detect判定結果(`DetectionResult`・per-VK状態・`raw_tsf_literal_consecutive_count`のgive-up分岐、BUG-03/24/27/29/30/36/38/40/45の9件で決め手)の0%収録を、Opus設計([docs/design/journal-literal-detect-capture.md](../design/journal-literal-detect-capture.md))に基づき解消。probe timingを一切変えない制約(yield_step呼び出し回数不変をコードで確認)を守りつつ、verdictをアクションから逆算不可能(`per_vk_recovery_params`がSuspectedLiteral/StaleConfirmを同じ値に潰すことを確認済み)という制約下でProbeAction経由でplatform.rsまでfactsを持ち上げる設計。output/tsf層の`crate::journal`非参照を`architecture_guard`の新規ガードで機械的に強制(33→34件) | **実装済み・round2/round3是正済み**(2026-08-19、codex CLI実装・Claude/Opus検証。test 410件+journal_replay+drift_correction_replay+architecture_guard(34件固定)全green、xwinビルド成功。Windows実機確認は未実施) |
| [098](098-tsfnative-applied-confirmed-laundering-and-force-on-removal.md) | BUG-34追補4(eisuガード撤去)完了直後、「eager warmupもGJIなら不要では」という疑問を機にeager warmup/drift correction/TsfNative force-onブロックの3機構をOpus premortemで監査(BUG-69)。TsfNative force-onブロックは`ir_post_focus_change_snapshot`が常にfocus settle barrier内で呼ばれるため到達不能(F1)、同関数の`mirror_applied_open`が何もapplyせずbeliefを`applied=Confirmed`へ偽装しfocus_tracking.rsの「TsfNativeはapplied=Unknown維持」不変条件に違反、`apply_force_on_for_imm_broken`(BUG-16修正)のスパムガードを誤発火させ恒久的に無効化する(F2、核心)。結果TsfNative+GJIのフォーカス復帰時に発火する唯一のactuationはeager warmupのみとなり(F3)、そのscan付き`VK_DBE_HIRAGANA`はBUG-15追補7が「実IME確実ON時のみ」と禁止する危険な注入形態を無監査で行っていた(F4)。**ADR-087の「`AppliedImeState`がConfirmedに遷移する契機が無い」という前提がF2により誤りと判明**、Phase3配線着手前の再検証が必要。決定: F2修正(mirror_applied_openをTsfNativeで呼ばない)→force-onブロック撤去→eager warmupゲート強化、の順で段階的に実施。drift correctionはKEEP AS-IS | **実装済み（クロスコンパイル検証のみ、Windows実機未検証、2026-08-21）**。決定0/1-a/1-b/1-c/2/4/6-a/6-b/6-cをコード反映済み、`cargo xwin check/build/clippy`全クリーン・Linuxで実行可能なテスト504件全成功。Windows実機での再現・検証・ソークは未実施 |
| [099](099-config-preservation-on-upgrade.md) | ユーザー報告「バージョンアップすると既存の設定が失われる」の原因調査と対策。**round1 Opus premortemで「MSI経路は保護されている」という当初前提が誤りと判明**: `wix/main.wxs`の`<MajorUpgrade>`に`Schedule`属性が無く既定値`afterInstallValidate`のため、新バージョンのファイルインストールより前に旧バージョンが完全アンインストールされ、`ConfigFile`コンポーネントの`NeverOverwrite="yes"`(KeyPathはレジストリ値でファイルではない)は無力化される(F1、MSIでインストールした全ユーザーが対象、最優先で修正)。加えてZIP配布の2箇所と実装共通の1箇所: (F2)`scripts/uninstall.ps1`が`%LOCALAPPDATA%\awase`を無条件再帰削除、(F3)`scripts/install.ps1`が`config.toml`は「既存なら上書きしない」のに`layout/*`は無条件`-Force`上書き(`awase-yab-editor`はawase-settingsの「配列編集」タブとして統合済みと判明、GUI編集ユーザー全員が対象)、(F4・最重要)`awase-settings`の`AppConfig::load()`失敗時に`default_config()`へ静かにフォールバックし「適用」でconfig.tomlへ永続化(`AppConfig::general`に`#[serde(default)]`が無く`[general]`欠落だけでparse失敗する点も発見、F5としてpaths.rsのCWDフォールバックも関連リスクとして記録)。決定0〜8: `<MajorUpgrade Schedule="afterInstallExecute">`＋`NicolaYab`コンポーネントへの`NeverOverwrite="yes"`追加(最優先、round2でMSI上書き経路と削除経路が別物と判明し拡張)、uninstall.ps1のユーザーデータ削除を`-Purge`明示フラグへ分離、install.ps1は`layout/`のみ非破壊化(`data/`はプログラム資産として対象外のまま維持しMSIと挙動を揃える)、`AppConfig::save()`をfsync+リトライ付きアトミック書き込み化、awase-settingsに`NotFound`/`Dangerous`(NotFound以外は全て危険側に倒す明示ルール)分類のload状態を追加しegui内製の確認UIで警告・一度限りバックアップ、`general`フィールドへの`#[serde(default)]`付与、パス解決フォールバックへの診断ログ追加、`wix/main.wxs`のGUID/Schedule/NeverOverwrite不変条件を機械的に固定するguard test新設。schema_versionマイグレーション機構の新設は不採用(既存方針に整合) | **実装済み(2026-08-21)**。round1(6 must-fix)→round2(実コード裏取りで4 must-fix、うちMSIの`layout/nicola.yab`保護漏れ等)→round3(round2 must-fix4件の反映確認)の3ラウンドOpus premortemを経て実装。cargo test(800件超)・fmt・CI相当clippy・`cargo xwin check/clippy/build --tests`(実Windowsターゲット)全緑。実装後Opusコードレビューで2件のCONFIRMEDバグ(guard testが自分のコメント文言で無効化・`.bak`コピー失敗時も保存続行)を検出・修正済み。Windows実機でのアップグレード検証は未実施 |
| [100](100-gji-warmup-vk-ime-on-reinit.md) | ADR-098決定3-c(GJI eager warmupキーをVK_DBE_HIRAGANAからVK_IME_ONへ置換できないかの宿題)とexperiments.mdエントリ16(事前登録)をユーザー発案の2提案とともに引き取り。architect⇔premortem reviewerのOpus2体で2ラウンド討議(ラウンド1: Critical4/Major4/Minor6件、ラウンド2: Major3/Minor5件、いずれも決定は不変で根拠の記述精度のみ訂正)。**F1**: eager warmupは`InjectionMode::Tsf`でのみ発火しChromeは対象外(`AppKind::TsfNative`→`Vk`、実行時学習でも昇格しない)——ADR-098原文の「撤去するとChromeのBUG-02が再燃」という被害例も誤りと判明。**F3/F5/F12**: 初稿はBUG-45実機ログ(自ら引用)と矛盾する誤った事実(「Tsfモードでは撃たれていない」「IMC読み取りは未測定」「give-upはほぼ0件」)を書き、2ラウンドのpremortemで都度訂正(同型の「モード分割の言い切り」誤りを3回犯したことも記録)。決定: 提案1(F2→VK_IME_OFF→VK_IME_ONトグル化)は却下(composition閉鎖の意味論差+confirm実効性の保証欠如+頻度差)、提案2(give-up分岐へのconfirm後retry追加)も却下(完了通知経路が存在しない)だが**代替として案L(give-upで失ったromajiをjournalへ記録、送信ゼロ)を採用**、案J(Unicode退避)・案K(backspace無し)は却下せず保持。決定5〜7は挙動変更ゼロのdoc訂正・known-bugs.md記録。**2026-08-22、決定4-f(`MapVirtualKeyW(VK_IME_ON)`実機測定=0xF2非ゼロ、F17)を経て決定2(`VK_IME_ON`単発、群B)を実機検証(F18、15.6秒・30.3秒放置含むcold=1〜13でgiving up/literal化0件)、ユーザー判断で正式採用・実装済み**(`send_vk_dbe_hiragana_pair`→`send_eager_warmup_vk_pair`に改名、`docs/known-bugs.md`BUG-50追補2)。群A/群Cとの厳密比較は未実施のまま採用、群C(eager warmup完全撤去)はBUG-69依存の懸念により対象外・別課題として保留。BUG-69(ADR-098)修正自体もdragonflyg4で初回実機検証済み(`force-ON (ImmBrokenForceOn)`が物理キー操作なしで自律発火し正しく補正) | **決定2実装済み・決定1/3却下確定**（2026-08-22）。決定5-7(記録・doc訂正)と決定3案L(journal記録)は設計のみで未実装。決定2は実機検証(群Bのみ、群A/C比較なし)を経てdragonflyg4実機ビルドでclippy/test(697件)/architecture_guard(38件)/golden(24件)全green確認のうえ採用。群Cの本格実験・BUG-69ソークは今後の課題として残る |
| [101](101-bug74-giveup-retry-with-focus-guard.md) | BUG-74: RawTsfLiteralRecovery give-up で失われるromajiを、F6 focus世代照合・WM完了通知・Polling中deferred順序保護を前提に通常送信経路で1回だけretryする。ADR-100決定3の却下理由を前提条件として解消し、決定5(F6)も実装する | 採用・実装済み（2026-08-24、実機ソーク未実施） |
| [102](102-startup-key-delivery-one-way-closure.md) | 起動シーケンスの入力消失・クラッシュ防止。Opus2体のドラフト→敵対的レビュー4ラウンド+追加検証で収束した初版を、その後の根本原因分析(ADR-105)を前提に全面改訂(2026-08-26)。OS所有マスク・Altなりすましラッチのフェーズ繰り上げ等リスクの高い決定を撤去し、hookコールバックのBox撤去(SPSCリング)・bootstrap専用focus scope入口による5層モデルの簡略化を追加。layoutsが空でのbootstrap panic対策は変更なし | 実装済み（2026-08-26、Windows実機ソーク未実施） |
| [103](103-warmup-probe-pending-integrity.md) | Warmup/Probe過渡期のpending取りこぼしとFSM整合性。`dispatch_probe_actions`の早期returnがdeferred VKフラッシュとGjiFsm通知の両方を飛ばす問題(BUG-27未解決follow-up)を、**段(stage)の終わりを型で強制する**形で閉じる: `DispatchResult::{Continue, Ended(StageEnd)}`+ラベル付きbreakで関数から`return`を消し、段末の後始末(deferred解放/GjiFsm通知/gate後始末)を`finish_probe_stage`1箇所に閉じる。注入の記録は`impl ProbeIo for Output`の注入メソッド自身が`note_stage_injection`で行い、`mark_cold_raw_tsf`が`note_stage_recovery`を立てる(リテラル回収を出した段はwarmを主張しない、INV-D)。gji_fsmのEndCompositionがColdKindを固定値で捏造する問題は`kind`の運搬と`ColdKind::probe_params()`一元化+`unwrap_or_default()`撤去(INV-C)で解消、pending破棄5箇所に`DiscardPending`を明示。post_bypassは汎用`ScopedOneShot<ForegroundScope, T>`+4値の純関数`classify_post_bypass_key`へ分解。**ラウンド6で根本設計へ転換**: 「6箇所から共通関数を呼ぶ」案は3箇所が出口ではなくflush点で実装不可能(per-VK confirmが全モーラで壊れる)と判明し撤回、per-VK列の輸送手段降格も1 tick 1 VKのため成立せずgateを段の入場条件に限定、`pending_gji_warmup`が段をまたぐ潜在バグ(BUG-83)と`LearnedTsf`のguard/probe_id未解放を新規発見 | 実装済み（2026-08-26、PR #108でdevelopマージ済み、Windows実機ソーク未実施） |
| [104](104-observation-freshness-and-hardening.md) | 非同期観測の鮮度(ObservationTicketへのfocus_hwnd/intent_seq拡張)・drift confidence 3値化・generation=0番兵衝突の解消・key_pipelineの同期conv読み取り追い出し(BUG-34横展開)・SendInput/SetTimer戻り値の型化・型で保証されないunreachable!の除去・候補ウィンドウveto flicker指摘の撤回(再現しないと判明)・ForceOnReason::ProfilePolicy等の死んだ安全弁撤去 | 提案（未実装、2026-08-26） |
| [105](105-engine-thread-notification-via-hwnd.md) | ADR-102の根本原因分析で判明した、PostThreadMessageW(スレッドID宛)がネストしたモーダルポンプ中に恒久消失する構造的脆弱性への対策。post_to_main_thread(唯一の集約点、13箇所)の実装をエンジン専用HWND宛PostMessageWへ差し替え。BUG-09(PostMessageW(None,..)の罠)の再導入ではないことを明記。実機実験(dragonflyg4)でネストポンプ中も配送されることを検証済み。Ctrl+C/--exit-afterが集約点を迂回し同じ脆弱性を新規に露呈していたことも発見 | 実装済み（2026-08-26、Windows実機ソーク未実施） |
| [107](107-bug25-gji-half-width-alnum-entry.md) | BUG-25「左Shift単独タップ→IME-ON半角英数トグル」のGJI側entry機構(3回撤回済み)を4回目としてどう作るか。**追補4の真因記述を訂正する**: `hook.rs::is_self_injected`(INJECTED/TSF/IME_KANJIの3マーカ)がマッチしたイベントは`build_raw_key_event`より前で`CallNextHookEx`に落ちるため、awase自身のマーカ付き注入は`transport::plan`に構造的に到達し得ない——追補4が特定した`dbe_mode_key_policy=Suppress`は**経路5(spikeのunmarked SendInput)のみ**を説明し、追補1/3(TSF_MARKER付き注入)の失敗は説明しない。代わりに原因Bとして**修飾キー文脈**を特定: entryが発火する`kp_shift_conv_guard_key_up`時点で`HeldModifiers::read()`は`PHYSICAL_KEY_STATE`由来のため既に`shift=false`を返し`push_release`がShift↑を出さない一方、awaseが物理Shift↑をまだreinjectしていないためOS/実IMEから見たShiftは押下中——追補2/3の注入は`Shift+VK_DBE_ALPHANUMERIC`として届き、mozcのキーマップに束縛が無くno-opになっていた可能性が高い(復元側`kp_restore_kana_from_half_width`は`prepend_synthetic_shift_up`でこの罠を既に回避しているのにentry側だけ持っていなかった)。決定0(実装前に spike で marker×awase起動×Shift の2×2切り分け計測、これを飛ばして実装に入らない)→決定2(`IME_KANJI_MARKER`+scan=0+synthetic Shift↑前置、最優先案)→決定3(M3失敗時のみ: unmarked注入+`DbeSelfInjectionPass`ワンショット通行証。マーカと通行証の二要素で識別しBUG-52の外部DBEキー保護は無傷、専用マーカを`is_self_injected`に足さないという反直感的契約をguard testで固定)。非冪等な`ToggleAlphanumericMode`の冪等性はawase側の遷移ゲートで担保(INV-A/B)し、**exitは`kp_restore_kana_from_half_width`が無条件に`false`代入+OS書き込みする現在の実装のままだとGJIでは二重復元が「かなへ戻す」ではなく「英数へ反転」する**ため`mem::replace`で真の遷移時のみ送る。Composition/候補表示中はentryを発火せず**ラッチもしない**(追補3の「かな入力が壊れる」実害の再来を防ぐ、INV-D)。置き場所は`Output::send_gji_half_width_alnum_toggle`(vk_send=テキスト送信/conv_actuation=IMM32 conv write/tsf/send=warmup専用、いずれも責務不一致)。キルスイッチ`half_width_alnum_toggle`(`off`/`ms_ime_only`既定/`all`)を新設しGJI経路はオプトイン、`dbe_mode_key_policy`への相乗りは軸違いとして却下。`toggle_entry_supported`判定を純粋関数`plan_half_width_alnum_action`へ切り出しLinuxでテスト可能にする | 実装中（2026-08-27）。決定0完了、Task 1〜8をコード反映中。Task 9のWindows実機検証・ソークは未実施のためBUG-25は未クローズ |
| [108](108-ime-apply-pending-generation-ordering.md) | `/code-review`（develop過去1週間差分、medium effort）でAngle A/Angle Altitudeが独立に発見したbelief破損リスクへの対応。`ImeModel::reduce()`のImeApplyRequestedアームが進行中pendingを警告ログのみで無条件上書きし、上書きされた古いgenerationの完了が`record_ime_apply_result`のgeneration厳密一致チェックで捨てられ`applied_open`が古いまま固定される問題。Opus 2体（提案役/批判役）によるレビューで収束。`applied`更新の可否・pending解決の可否・composition副作用駆動の可否という3つの独立した問いへ分解し、`ImeTransition`にfocus_epochをスタンプ、FocusChanged時点のgeneration watermarkで旧epoch完了の緩和受理を防ぎ、generation不一致でも同一epoch・同一targetの成功完了は`Optimistic`で`applied`更新、戻り値を`ImeApplyAcceptance`型にしてcomposition副作用ゲートは一切緩めない、`reduce()`冒頭のタイムアウトパージをmatch後に移動する。過程で`FocusChanged`が`applied`のみリセットし`pending`を残す既存欠陥（フォーカス跨ぎでforce-ON恒久封鎖）も発見・対象に格上げ | 採用・実装済み（2026-08-28、Windows実機ソーク未実施） |
| [109](109-yab-cv4d-punctuation-auto-confirm.md) | [GitHub Issue #118](https://github.com/cuzic/awase/issues/118)「やまぶきCV4D相当（句読点入力時の変換候補自動確定）」の実現機構を検討。`YabValue`/`KeyAction`に`ConfirmThenSend(Box<Self>)`を新設する個別実装案（確定実体は既存の`SpecialKey::Enter`送信経路の再利用、composing判定はプラットフォームの`send_keys()`直前で行う非対称設計、既定Offの隠しキルスイッチ等）を検討したが、専用variantとして先取り実装せず、**将来実装予定の汎用「打鍵列機能」（1セルに複数キーアクション列を定義できる機能、未着手）の一特殊ケース**として位置づけ直すことにした。本ファイルは調査結果をその設計時の入力資料として保持する | 保留（本ADR単独実装はしない、2026-08-28。ADR-115が`CV4D`の実体(Ctrl+M)をComposing判定なしの直接送信で実装したため、本ADRのConfirmThenSend案は当面採用見送り。将来Composing条件付き確定が必要になった場合の参考資料として保持） |
| [114](114-keymap-app-scoped-shortcut-wiring.md) | `[[keymap]]`（ADR-037、アプリ別ショートカット再割当）は設定GUI・config parse・フォーカス別フィルタ（`active_keymaps`）まで完成しているが`KeymapTable::find_match`を実際のキー処理から呼ぶ箇所が皆無で、設定しても一切効果がない死んだ機能だった（2026-08-28発見）。PowerToys Keyboard Manager devdocsを参考に配線設計を確定。Opus 2体の敵対的レビューr1〜r4で収束：r1でADR-110は未マージではなく実装後にBUG-100で撤回済みという事実誤認、latchと自動リピート判定の兼用がBUG-100を再現する設計、latchが消えない5経路の見落としを検出・修正。r2の改訂でrepeat判定に`is_physical_key_down`を使うと配線後も一切動かなくなる新規Critical、決定4内の自己矛盾（経路4の「無条件上書き」とrepeat抑制規則の衝突）、Altを`from`の修飾子として禁止し忘れ、`release_all()`のKeyUp注入が決定3のDown+Up同一バッチ完結と矛盾、を検出・修正。r3でKeyDown/KeyUpのstep配置が非対称なままだと経路4の残存リスクが「1打鍵消失」に収まらないと指摘されr4で解消。挿入順序はlatchチェック（KeyUp解放+KeyDown repeat抑制、最優先）→Nested/NonText早期return→find_match新規照合→post_bypass消費→NICOLAエンジン、に確定。実装タスク分解(T1a〜T11)もOpus 2体でr1〜r3レビューし収束させ実装完了(cargo xwin check/clippy -D warnings/fmt/machete全green、architecture_guard/golden_scenarios/layer_boundary_guard/lib全ユニットテスト計1516件green)。副次的にkeymap.rsがWindows専用ゲートでLinuxテスト不能だった点も是正 | 採用・実装済み（2026-08-31、Windows実機ソーク未実施） |
| [110](110-simple-physical-key-remap.md) | 秀Caps調査から着想した、物理キー1つを別の物理キーとして常時扱う単純リマップ機能（`[[key_remap]] from/to`）。PR #120で実装・PR #121（BUG-100）でstuck modifier修正まで済ませたが、後継ADR-111（Caps(英数)⇔Ctrl入れ替えプリセット）の設計レビューで、フックベースのキーリマップは日本語IME環境のCapsLock位置キーに対して構造的に危険と判明（`docs/experiments.md`エントリ07/08/09の先例、PowerToys Issue #3397/#32344と同型の問題）。「アプリごとに動的にキー割当てを変更する」将来機能で置き換える構想もあり、バックエンドごと撤回した | 撤回（2026-08-30、ADR-111 r4決定によりrevert。実装・修正まで完了していたが後継検討で機能全体を取り下げ） |
| [111](111-caps-eisu-ctrl-swap-preset.md) | 「人気のある組み合わせに絞ってGUIを簡単にしたい」という要望を受け、ADR-110の汎用`key_remap`をCaps(英数)⇔Ctrl入れ替え1種類のプリセットへ絞り込む設計。Opus 2体の並列敵対的レビューとPowerToys実例調査（Issue #3397/#32344）で、フックベース方式（key_remap）はJIS英数キー位置と日本語IMEのShiftショートカット競合により構造的に危険と判明し、Scancode Map（レジストリ、ドライバレベルのスキャンコード置換）一本化に方針転換（r2）。さらに検討の結果、汎用key_remap GUIエディタも撤去（r3）、ADR-110機構自体をバックエンドごと撤回（r4、PR #123）。Scancode Mapのバイト列パース/生成/マージを純粋関数化しLinuxでテスト、既存値の無条件上書きを避けるマージロジック、`awase-settings.exe`自身を`--scancode-map on\|off`で自己昇格(`ShellExecuteExW`+`SEE_MASK_NOCLOSEPROCESS`)する昇格フローを実装（PR #124） | 採用・実装済み（2026-08-31、Windows実機ソーク未実施） |
| [112](112-keyup-lifecycle-fsm-delivery.md) | `Engine::on_input`のPhase 0（`KeyLifecycle`、2026-03-31混入）が、Consume済みKeyDownに対応するKeyUpを無条件に「OSへ渡さない」だけでなく「FSMへも渡さない」まま握りつぶしていたリグレッションをBUG-101として発見（`feat/confirm-mode-simplify`でのEngine経由テスト作成中に発覚）。`min_overlap_margin_percent`が実運用で常に無効・`KeyAction::Key(vk)`出力キー全般でstuck key・`OutputHistory`が上限なし`Vec`で単調増加、の3実害を確認。Opus 2体の敵対的premortem 2ラウンドで、`OutputHistory`をKeyUp整合性索引(`pending_releases`)とn-gram文脈確定ログ(`committed`)に責務分離してから着手する順序、`min_overlap_margin_percent`既定値を一時的に0へ落として「経路修正」と「判定有効化」を分離、Phase 0を「Consume義務の予約+単一出口での`force_consume`格上げ」に再設計しつつ非活性時専用の`release_only`狭入口で内部状態の取り残しを防ぐ、`UpDuty`は三値案から根拠不成立で二値へ撤回、の4コミット構成に収束。決定0〜2を実装済み | **クローズ**（2026-08-31、Windows実機ソーク完了・不具合報告なし。決定3は実測データ無しのため見送り、`min_overlap_margin_percent`既定0%を恒久化） |
| [115](115-yab-keystroke-sequence.md) | [GitHub Issue #118](https://github.com/cuzic/awase/issues/118)で所有者自身が提案した、`.yab`の1キーに複数`KeyAction`を定義できる汎用「打鍵列機能」。Opus 2体の独立レビューをr1→r6の6ラウンド実施。r1(セル内トークナイザ+`ConfirmIfComposing`案)はCritical4件で「実装に進められない」。r2〜r4は`CtrlChord`+セル内`+`区切り+名前付きマクロレジストリへ再設計する過程でラウンドごとに新設計起因のCriticalが見つかり続けた(平坦化点の列挙漏れ、Issue実データ(33セル中29が単発2要素)による決定前提の反証、投機出力ガードのfalse時フォールバックが打鍵列消失/生VK漏洩/受付窓縮小を招く、キルスイッチOff復元の`serialize()`非可逆性、`InlineSequence`に`Vk`禁止フィルタ未適用でstuck key再発、`InlineSequence`内`MacroRef`展開で`Sequence`ネスト等)。r5でCritical 0件に到達（両エージェント一致）、r6で残ったMajor(空Sequenceの扱い未定義→`YabValue::None`に統一、`Romaji`許可によるn-gram文脈欠落を既知制約として明記、モジュール配置確定)とMinor群(投機ガードの機構記述誤り訂正等)を反映して収束。最終設計: `CtrlChord`/`InlineSequence`は元セル生テキストを`raw`として直接保持しキルスイッチOff時はそれを返すのみ、`KeystrokeMacro.steps: Vec<String>`を既存`YabValue::parse`にそのまま通す、決定2b/2cの許可リストを`InlineSequence`にも適用しマクロ展開は`Sequence`で包まず平坦にextend、投機ガードのfalse時は`PendingChar`を維持し`Phase2Transition`で残り時間を再設定 | 採用・実装済み（2026-08-31、r6でOpus 2体レビュー収束後に実装。パーサ・config・engine・Windows配線・macOS/Linuxスタブ・設定GUI表示まで反映し`cargo test --lib -p awase`919件・fmt・clippy(`-p awase`)全green。実装過程で`release_only`のSuppress扱いに関するADR原文の誤り（既存の意図的pass-through仕様を「バグ」と誤認）を発見しADR本文を訂正、既存挙動を保持する形へ修正済み。実装後Opus `/code-review`（8観点並列）で2周にわたる自己回帰2件（Unicode cold-defer順序判定の過剰な無効化条件、GUIプレビューのresolve混入によるレイアウト保存時の打鍵列セル破壊）とLinux/macOSでのキルスイッチ未配線を検出・修正済み。Windows実機ソーク未実施） |
| [116](116-startup-settings-diagnostics.md) | BUG-104（`.yab`読込失敗の無言フォールバック）の調査を機に、「設定が正しいか診断して警告する」機能を汎用化してほしいとのユーザー要望。r1は独自の`Diagnostic`型・`diagnose()`関数を新設する設計だったが、Opus 2体の敵対的レビューで`LayoutEntry::scan_all`・`config_path_panel`という既存コードとの重複、USキーボードで同梱JIS用レイアウトを恒久的に誤警告する既存バグ、`reload_config`が`config.validate()`警告をユーザーに一切届けていない事実誤認、を検出。r2で新規抽象を撤回し既存の走査点・既存UIへの追加に縮小。r2の再レビューで`mem::take`と`layouts_dir`解決順序の衝突・US誤警告の再混入をr3で解消、architectの実装レビューで`mem::take`書き戻しが決定3の方針とそもそも両立しない構造的欠陥をr4で`clone()`方式に修正。実装完了後、独立したOpus 2体のコードレビューでlint警告がセル単位で通知を埋め尽くす問題・`layouts_dir`不在時の無言フォールバック・テスト不足を検出しr5で解消、実装・テスト完了 | 採用・実装済み（2026-08-31、Windows実機ソーク未実施） |
| [117](117-bug138-msime-composition-diagnostic-logging.md) | [GitHub issue #138](https://github.com/cuzic/awase/issues/138)「MS-IME『直接入力モードを使用しない』を無効化していると、英数キーで入力中の文字が消える」の切り分け用に、挙動は変更せず診断ログのみを追加。r1は`ImmCrossProcessStrategy::apply`にログを足す案だったが、Opus 2体の敵対的レビューで「報告環境の実書き込みは非同期`imm_cross_write`を通り戦略層自体を経由しない」「journal記録がcomposition tear-down後の値になり一次証拠にならない」という致命的欠陥を検出。r2で対象を実際の4送信経路（`imm_cross_write`/`MsImeDirectStrategy`/`KanjiToggleStrategy`/`ImmCrossProcessStrategy`）に絞り直しjournal案は撤回、architectはr2で収束。premortemはr2で送信成否の可視性不足・`fallback_write`経由の値の陳腐化・`composition_active=false`の両義性を指摘しr3で反映、さらに「送信していない」無音分岐3箇所とログ書式の不整合をr4で解消し収束。r4の設計どおり実装完了、check/clippy/fmt/guard・golden(94件)/lib(921件)全green | 採用・実装済み（2026-09-02、Windows実機ソーク未実施） |
| [118](118-teams-kana-lock-detection.md) | [GitHub issue #137](https://github.com/cuzic/awase/issues/137)「Teams(WebView2/MS-IME)でawaseの送信VKがJISかな配列として誤解釈される」への対応。Opus 2体（proposer/critic）の3ラウンド敵対的レビューで収束。r1はconvのROMANビットに基づく検知案だったが、r2でTeams（`Imm32Unavailable`分類）では`kp_stage_idle_conv_check`の経路自体が走らないと判明し検知不能と確定、`GetKeyState(VK_KANA)`直読みへ主軸を転換。r3でBUG-14の高頻度VK_KANAエコーによる通知点滅リスクを指摘されヒステリシス（`KanaLockHysteresis`）を導入し収束。実機スパイク（`spike_kana_lock_probe.rs`）でTeams focus中の言語バー操作による入力方式反転に`GetKeyState(VK_KANA)&1`が追従することを確認。自動復旧はBUG-61/62で不可能と確定済みのためスコープ外、案内のみ。実装はCodex CLIに委譲しOpusが独立レビュー、初回で検知漏れ（`KeyAction::Romaji`しか見ておらずNICOLAの主要出力経路`Char`を取りこぼす）等2件のブロッカーを検出・修正、再レビューで収束。PR #142の`/code-review`（Opus）でWM_APPメッセージの再入時ドロップリスク・ADR未登録・BUG番号衝突（並行PR #141と同一番号）等を検出・修正 | 採用・実装済み（2026-09-02） |
| [119](119-injected-and-relay-key-consumption-invariant.md) | [GitHub Issue #136](https://github.com/cuzic/awase/issues/136)（PowerToys「境界線のないマウス」経由でIME ON/OFFが効かない）。R2実データ検証でBUG-90と同一インシデントかつ独立した2バグの合成と判明: (1)リモート側は`transport.rs::plan`のVK_DBE_* SuppressがBUG-14修正の不変条件「解釈しない入力は消費しない」をBUG-52対応(2026-08-05)が破ったリグレッション (2)ローカル側はMWB中継ウィンドウへのImmCross actuationが宛先ミスマッチで空振り。Opus 2体の敵対的議論で決定1(injected passthrough)+決定4(`AppImeProfile::InputRelay`新設)に収束。設計段階のコードレビューで`caps()`空チェーン案(観測なしにbeliefへ嘘を書く欠陥)を`ImeOpenOutcome::NotOwned`新設へ差し替え、`should_pass_physical_key`が本番デッドコードだった発見でcondition(b)の実装場所を修正、`debug_assert`配置ミス(通常操作でpanicする)を発見・修正。実装完了後のOpus敵対的コードレビューでさらに、gateを`runtime/executor.rs`1点だけに置いた初期実装がissue #136当該操作(物理IMEキー押下)の経路をバイパスしBUG-46型の二重actuationという自己回帰を生んでいたことを発見、実際の合流点4箇所(`ImeController::apply`/`run_open_chain_async`/`fallback_write`/`imm_cross_write`)へgateを置き直して修正。番号衝突（develop側issue #137がADR-118を先に採番）が発覚しADR-119へ改番 | 実装済み（2026-09、Windows実機ソーク・MWBプロセス名確定・リンク(a)フックチェーン順序の実機確認は未実施） |
| [120](120-retroactive-ngram-correction.md) | GitHub issue #140 (b)（PR #141のスコープから除外した事後訂正）。NICOLA 3鍵仲裁の曖昧決定（`three_key_pairing`のPhase2）を、1〜2かな後の文脈で再評価しBACKSPACEで訂正する機構の設計。Opus 2体の敵対的premortem 4ラウンドで収束、各ラウンドで中核前提の実コードとの食い違いを1つずつ発見: r1は決定8のスコア関数が実際は非対称（`score_a`が1かなのみ、`score_b`は`char2_single_kana`を計算すらしない）であること・決定1「BS数=かな数」が既存の「1完全ローマ字=1 BS」モデルと衝突すること・「Phase0は計装のみ」が実際は決定2/8の全実装を含むことを検出。r2は導入した適格述語が実装として逆向き（通常のかな出力を全部弾き訂正が一度も発火しない設計だった）というshowstopperと、候補Aの仮想出力X・Y₂がどの適格条件も通っていないこと、platform→coreの逆流チャネルが存在しないことを検出。r3は代替導入した「1打鍵遅延commit」が`record`は`on_reduce`によりparseループ内で即座に適用されるという実装事実と矛盾することを検出。r4で残った軽微な3点（E7充足率のPhase 0a計測追加・awase-linuxの1 arm要否訂正・乖離の有界性は無害ではないという明記）を反映して収束。最終設計は決定0a（既存決定のカウンタのみ、新設計ゼロ）の実装のみを承認し、決定2〜8（実際の訂正機構）は判断点1のゲート（母数・対照群付きユーザー訂正相関・E1+E2+E7+E9の適格率上界）を通過した場合にのみ実装する条件付き設計。2026-08-30の「n-gram既定化は時期尚早、計装が先」という結論を、Phase 0a/0bの分離という形で履行。番号衝突（並行PR #142/#143が118・119を先に採番）が発覚しADR-120へ改番。**追記（決定0a-report）**: Phase 0aのカウンタの取り出し経路が未規定だったため、既存のタスクトレイ不具合報告機能（ADR-095）への統合を追加（`BugReportRetroEvalStats`新設・既定ON）。単独premortem 1ラウンドで、schema_versionを上げる初稿案がWorkerの厳密等値検証により旧クライアント全員の不具合報告を拒否してしまう（目的と正反対の効果）欠陥を発見しoptional受理へ修正、項目4/7を単一カウンタからヒストグラムへ変更 | **Phase 0a実装済み（2026-09-02、判断点1のデータ収集中）。決定2〜8は判断点1のゲート通過が条件、通過しなければ棄却クローズ** |
| [121](121-explicit-physical-ime-key-idempotent-reassert.md) | BUG-37（既存、物理IME訂正キーが`kp_stage_shadow_ime_toggle`のno-opガードに握り潰される）の部分対策。不具合報告`01M1GVNR840NZ3XWRX0JPDSQR7`（Windows Terminal+MS-IME、「よしっ」→「yosiltu」literal化）の実機解析から起票。当初「物理キーがOSに届かなかった」と誤診断していたが、Opus 2体round1レビューで自己検証の結果「3回ともネイティブにOSへ届いていたがMS-IMEが応答しなかった」と訂正——no-op説は原因の半分に過ぎず、MS-IME側がなぜネイティブキーに無応答だったかは未解決のまま残る（GJI→MS-IME製品切替が候補仮説として浮上、BUG-25の同型既知の限界を参照）。Opus 2体（architect/premortem）敵対的レビューround1〜3で収束。round1: `issue_open_warrant()`直接呼び出しがINV-47/48ガードに衝突し実装不能と判明→`issue_actuation_order()`+`would_have_blocked()`へ変更、デバウンス案（`physical_key_held_ms`）がVK_DBE_HIRAGANAにKeyUpが来ないため既存の3回連打を逆に抑制すると判明→専用クールダウンへ変更、ON/OFF対称化が`issue_open_warrant`のStep0で構造的に崩れると判明→ON方向のみへ縮小。round2: `romaji_pre_write()`のROMANビット書き込みを見落としていたと判明（`view.belief_input_mode`の明示的設定を追加）、0xF1/0xF4の解釈曖昧性（かなキー由来か独立カタカナ要求か未確認）を指摘され初期スコープを0xF2単独へ縮小、settle中の再試行の担い手が無いと指摘されpendingフラグ設計を追加。round3: `on_ime_apply_complete`の4分解のうち`record_ime_apply_result`のみ省略する決定の実装可能性（(4)の発火条件を`outcome ∉ {UnsafeToToggle, NotOwned}`で明示）・適用範囲（`generation==None`の同期経路限定）・省略の根拠（「ゲートBの沈黙が延びる」という当初の機構的主張は誤りと判明し撤回、BUG-69型belief偽装回避のみに一本化）を修正して収束 | **設計完了・両者承認（2026-09-02）。「BUG-37の解決ではなく欠落経路の補填＋診断能力の追加」と位置づけ、実機ソークで観測状態の改善を確認するまで未解決扱い。未実装** |
| [122](122-cold-start-per-vk-confirm-race-recovery.md) | BUG-75（既存、`StaleConfirm`回収がGJI I/Oカウンタのポーリング遅延をliteralの証拠と誤認する）の実機再発。不具合報告`01M1JGJNDJT9ZAEMRAEB58ES5A`（LINE+GJI、42秒アイドル後のcold-startでセッション最初のモーラ「と」の2番目のVK「o」がStaleConfirm誤判定→ESC+全体再送でモーラ重複）から起票。Opus 2体（architect/premortem）敵対的レビュー3ラウンドで大きく収束。round1で提案した「案F（`grace_hold_verdict`の早期確定バグ修正）＋案G（`veto_eligible`拡張）の二段構え」は、round2で案Gがper-VK経路（本incidentの経路）では`LiteralDetectCore::poll`を一切経由しないno-opと判明し崩れた。round3で案Gをループローカル条件・新設ゲートとして再設計した版をレビューしたところ、(1)終端状態「候補可視のまま無回収Done」が2026-07-22「kれでできる」というrevert済みregressionをそのまま再生産する、(2)適用条件に使うpending_confirmは判定地点で常にNone、という2つのblockerが判明し、案G/G'は本ADRのスコープから外し将来課題へ切り出した。あわせて本incidentの実際のdeadlineが500msでなく300ms（`target=Chrome`はlong-idle分岐を通らない）と判明し、案Fの効果も「効く」から「効く可能性がある（未測定）」へ訂正 | **設計継続中（Opus 2体round1〜3完了）。案Fを decision として確定（実装は4前提条件、うち観測フェーズの先行実施が必須）。案G/G'はblocker未解決のため本ADRのスコープ外、別ADR/将来課題へ。[GitHub issue #149](https://github.com/cuzic/awase/issues/149)として追跡、ユーザー判断で実装は一旦ペンディング（2026-09-03）。未実装** |
| [123](123-focus-resync-and-probe-defer-queue-composition-race.md) | [GitHub issue #148](https://github.com/cuzic/awase/issues/148)（Windows Terminal+GJIで「たとえば」が「ばたと」に文字脱落・順序入替）から起票。Opus 2体（architect/premortem）敵対的レビュー2ラウンド完了後、round2で提示された2点の検証項目を`report_id: 01M1JJD54XQXSEJTHHFKV1WKA1`の`app_log_excerpt`（journalでは追えないlog::生ログ層）を直接読んで確定させ、根本原因を確定させた。「た」のgive-upが予約したGJI reinitのポーリング完了を待つ間`pending_deferred`（と+え、3VK）のflushは保留されるが、この保留期間中に到着した「ば」が`pending_deferred`の存在を考慮しない`defer_if_probe_in_flight`（`has_pending_tsf()`のみ判定）を通り独立probeを開始して先に確定、`pending_deferred`が後から来たモーラに追い越される、という機序を特定。round0の「え+ば融合」仮説・round2で浮上した`discard_raw_recovery_if_focus_stale`（focus churn破棄）仮説はいずれも反証・不発火確認済みで棄却 | **根本原因確定・decision確定（未実装）。ユーザー判断により実際の修正（`defer_if_probe_in_flight`のgating拡張）は次回`TsfProbeStarted.pending_deferred_len>0`が実際に観測されるまで見送り、再発時に仮説を機械的に確定できる診断ログ（`TsfProbeStarted.pending_deferred_len`/`probe_id`、`DeferredRecoveryFlush`、`GjiReinitRetryCompleted`）のみを実装（PR #151）** |
| [124](124-tray-update-check.md) | タスクトレイ右クリックを唯一のトリガーにした更新確認。常駐フックプロセス `awase.exe` は通信せず、`awase-settings.exe --check-update` がWinHTTPでWorkerへ問い合わせる。状態は `update_check.json` に最小限だけ保存し、表示は `display()` で導出する。WorkerはGitHub latest releaseをKVでキャッシュし、URLは返さず、クライアントが検証済みSemVerからリリースページを組み立てる | 採用・実装中（2026-09-03） |
| [126](126-caps-as-extra-ctrl-preset.md) | ADR-111（Caps(英数)⇔Left Ctrl 双方向入れ替え）に加え、片方向（Caps(英数)→Left Ctrl のみ、元の Ctrl キーは変更しない＝「Ctrl を2つにする」）のプリセットを追加する設計。実現方式は ADR-111 と同じ Scancode Map（レジストリ）のみを踏襲し、hook ベースの新しい仕組みは導入しない——ADR-111 が確立した「JIS 英数キー位置を hook で扱うのは構造的に危険」という結論を再検証した上で維持。`ScancodeMapPreset` enum でプリセットを一般化し、Swap/CapsAsExtraCtrl を排他選択にする | **採用（Opus 2体4ラウンドの敵対的レビューで収束、実装済み）** |
| [125](125-egui-winit-dynamic-ime-association-focus-model-gap.md) | BUG-107（不具合報告画面・設定画面でテキスト入力の先頭に「あ」が混入）の調査ADR。最初に検討した`disable_apps`へのプロセス名追加案は「eframe/egui全体が非対応という話になる」とユーザーに却下され、対症療法ではなく機構の特定に転換。3回の実機スパイクで段階的に絞り込んだ: (1)「ウィジェット単位のフォーカス移動で同一HWNDのHIMCが脱着される」仮説→専用スパイクで一度も再現せず否定、(2)「`AppImeProfile::Standard`分類・IMM32クロスプロセス制御そのものが機能していない」仮説→awase本体と同一のWin32呼び出し列を再現した別スパイクで、この機構は正常に機能していることが確認され否定、(3)実際に`awase.exe`本体エンジンを起動した状態で症状を再現させ`RUST_LOG=debug`ログを解析した結果、`ImmCapabilityStore`（IMM32制御能力の学習キャッシュ）が`class_name`のみをキーにしており、winitの既定クラス名`"Window Class"`（`with_class_name`で上書きしない限り全winitアプリ共通）を介して、無関係な別プロセス由来の`Imm32Unavailable`誤学習が`awase-settings.exe`に伝播していたことを`cache.toml`直接確認で確定——BUG-56（Qtの汎用クラス名がLINE内の無関係なウィンドウを巻き込んだ事故）と同一機構のプロセス間版。修正方針として学習キャッシュのキーを`class_name`単独から`(process_name, class_name)`へ変更する案（`config.rs::AppOverrideEntry`の既存パターンに整合）を採用し、Codex CLIに実装を委譲、Opusモデルによる敵対的コードレビューを2周行って収束させた（`get_process_name`の無駄な二重呼び出し・空プロセス名によるBUG-107型汚染バケツの再発等、複数の実害ある問題を発見・修正）。副産物としてタスクトレイの「学習キャッシュをクリア」メニューが完全なno-opになっている無関係な既存バグ（BUG-108）も発見・起票した | **根本原因確定・実装済み（実機ソーク未実施）。BUG-107/BUG-108として起票** |
| [127](127-settings-single-apply-principle.md) | ユーザー報告（配列編集画面で「適用」が2つ・「保存」もあり、どれが実際の保存・反映か分かりにくい）を契機に、awase-settings全タブへ「変更を反映する操作は画面全体でただ1つ」というUX原則をゼロベースで適用する監査ADR。Explore agentによる全タブ監査の結果、全般設定・キー設定・上級者向け設定・アプリ無効化・ショートカット主要部分は既に`self.config`への直接バインドで原則に一致しており、原則違反は配列編集タブ1箇所（画面下部共通の「適用」/「キャンセル」が`self.layout`/`layout_modified`を一切対象にしていないため、保存し忘れると反映されず、キャンセルしても編集が破棄されない）に限定されることを確認。Scancode Mapセクション（UAC昇格・レジストリ即時書き込み）はADR-111決定4/7で意図的に例外化済みのため対象外とした。Opus 2体（architect/premortem）の敵対的レビューをround4まで実施。round1でchanged()トリガー案・保存先ダイアログ案が実装不能と判明し再設計、round2ではその再設計（lost_focus()トリガー）自体がeguiのパネル描画順に起因して「適用しても反映されない」を再生産することが両者独立に判明し未収束。コミット処理を描画順から切り離す設計（update()冒頭での無条件コミット判定）に変更したv2も、round3で「バッファの中身＝ユーザーの意図」という前提が崩れる2経路（IME変換中の未確定文字列、ValueKind::Specialの未操作インデックス）が両者独立に判明し未収束。セル選択時点のスナップショット（layout_edit_origin）との差分＋IME合成中でないことを判定条件にしたv3も、round4でlayout_edit_originが文字列のみだったため種別だけの変更（round2 B3）が回帰し、その修正（originにkind追加）だけではADR-115打鍵列セルの保護が種別ラジオ経由で崩れるというトレードオフが両者独立に判明し未収束。ガードの判定順序を組み替えoriginをタプル化した上でADR-115セル専用の独立ガード（layout_edit_origin_is_sequence）を追加したv4も、layout_modified要件撤回自体に両者独立の新blocker（公式のkeyboard_model+default_layout同時変更手順が操作履歴依存で理由不明に弾かれる）が見つかり、キーボードモデル不一致ガードを「データ保護（layout_modified必須）」と「エンジン健全性検証（default_layoutの実ファイルを直接検証、操作履歴非依存）」の2つに分離したv5をarchitectはround1〜4の全指摘解消として収束判定。一方premortemはround5でエンジン健全性ガード(B)を無条件（パース失敗＝常に中止）にしたこと自体がキーボード配列を変更しない適用まで巻き込む新退行(R5-1、ADR-116起動時診断の意図と衝突)と指摘、ガード(B)の中止/警告分岐をモデルが実際に変わるかどうかで条件分けしたv6でround6にてpremortemも「問題なし、収束」と判定、両者の収束が揃った。実装はCodexへ委譲後、Opusによるコード差分の敵対的レビュー2周でblocker2件（ComboBox`response.changed()`がegui 0.31.1で発火せず特殊キーがGUIから設定不能／破棄確認モーダルがダイアログキャンセル時に未保存編集を無言で「保存済み」扱いする）を含む指摘をすべて修正し収束。/code-review指摘4件（確認モーダルの排他制御漏れ2件・診断リスト再計算漏れ・layout_file_path未設定時の誤ったステータス表示）も修正 | **実装完了、PR #157でdevelopマージ済み** |
| [128](128-escape-composition-collateral-deferred-loss.md) | BUG-109（`report_id: 01M1MW0KSY5KWVYSGPGRBTNSPA`、ADR-123修正適用済みビルドで「なまえま」→「なま」に文字消失）から起票。初版は「flushは正しく成功していたが事後のescapeに巻き込まれた」と誤診断したが、opus-adversarial-consult round1が journal の`deferred_flushed: 0`とapp_logの実際の呼び出し順から因果が逆であることを実証: ADR-123決定4-3の`drain_pending_deferred_before_send_if_queue_only`（`output/vk_send.rs:216`/`:392`）が`gate`（`DeferGate::Enforced`/`Exempt`）を見ずに無条件発火するため、GJI reinit retryの再送（`resend_gji_reinit_retry_romaji`、`gate=Exempt`）自身の実送信より**前**にdrainしてしまい、その注入内容が再送自身のper-VK confirmの証拠を汚染して`StaleConfirm`→`VK_ESCAPE`を誘発する**ADR-123決定4-3自身の回帰**と確定。副次的に出力順反転も発生しうることも判明。decision: drainを`DeferGate::Enforced`限定に修正。backspace案は既存invariant（BUG-33追補3・4）と衝突するため却下、確定待ち案は候補シグナルが本件で両方とも事前に無効化されるため保留、計測先行案はADR-100決定4-aの誤引用と判明し独立の計装へ格下げ | **root cause・decision確定、opus-adversarial-consult round1〜3で収束（round3の唯一のblocker反映済み、レビュアーがround4不要と明言）。実装未着手** |
| [129](129-thumb-timestamp-live-requery-during-gate-drain-replay.md) | report `01M1N36MGDDJ5HN8FWRE4ZHS3J`（GJIで「ようするに」→「よゔするに」）から起票。journal/app_log実測でBUG-105（3鍵仲裁ロジック自体のバグ）とは別原因と特定: `key_pipeline.rs:105`の`hook::thumb_down_timestamps()`はWH_KEYBOARD_LLフックが実時間更新するグローバルAtomicU64をその場でライブクエリする実装で、ライブ配送と`OUTPUT_GATE`中に`INPUT_DEFER`へ退避されたイベントのdrain replay（`deliver_key_event(..., KeyOrigin::DeferredReplay)`）の両方から同一コードパスで呼ばれる。drain replayは数百ms前に発生した複数イベントを<2msのバーストで一括処理するため、古いイベント（本件ではA↓、実発生時は1回目の親指押下961165と同時）のreplay時にライブクエリすると「replay実行中の今」の親指状態（既に進行中の2回目の押下313529）を誤って読み、`NicolaFsm::is_thumb_consumed`の消費済み判定（[ADR-010](010-thumb-consumption-timestamp.md)）が不一致となり未消費の親指キーとして誤って同時打鍵確定(RightThumb+A=「ゔ」)する。`RawKeyEvent::modifier_snapshot`（`src/types.rs:206`）が全く同じ問題をCtrl/Shift/Alt/Winについて「capture時点でイベントに埋め込む」方式で既に解決済みであることが判明——本件は新種のバグではなくその修正パターンの適用漏れ。decision: 親指ダウンタイムスタンプも`RawKeyEvent`にcapture時点でスナップショットし、`key_pipeline.rs:105`のライブ再クエリを置き換える。キュー内再構築案・FSM側への時刻引数追加案・drain中は常にNone扱いにする案はいずれも却下 | Draft（root cause確定・opus-adversarial-consult未実施、decision未実装） |
| [130](130-keymap-multistep-shortcut-and-ime-keys.md) | ユーザー要望「打鍵列機能みたいな感じで複数の打鍵を注入できないか」を受け、`[[keymap]]`（ADR-114 ショートカット再割当て機能）の`to`を単一VKから複数ステップの列へ一般化する設計ADR。当初r1は「IME制御系VK（半角/全角/かな等）を明示オプトインで`to`に許可する」機構も同一ADRで扱おうとしたが、Opus 2体（architect/premortem）の独立レビューがそれぞれ別角度から技術的に成立しないと判定: `send_keymap_target`が`INJECTED_MARKER`付き送信のためフックの早期return（`is_self_injected`）で`ImeModel`のbeliefが一切更新されず実IME状態だけが変わる（awase自身のidle-conv-check/drift correctionが誤読して介入し実機で復旧不能になった記録あり）、かつ同じ手法（SendInputでVK_DBE_*を注入）は`docs/experiments.md`で既に3回試されて撤去済みという先例をr1が未引用、GJI config1.db書き込み方式も前日(`fc5898ff`)に撤去済みという事実誤認も発覚。r2でIME制御系VKのopt-inを完全に削除し「通常VKの複数ステップ打鍵列のみ」に純化、ADR-115打鍵列エンジンの転用も棄却（物理修飾キーのrelease/restoreが無くADR-115が「稀」と許容した限界が`[[keymap]]`では100%発生するため）。両者ともr2を「収束」と判定、複数の軽微な追記（TOML例が既定設定で自身の禁止規則により動かない事故・`vk_may_mutate_conv`が`ImeKeyKind`より広くVK_DBE_ROMAN/NOROMANとVK_CONVERTを漏らす穴・OR判定はfrom側には適用しない非対称性等）を反映して確定。IME関連キーのcharset軸切替は別ADRへ切り出し | **採用（r2、Opus 2体の敵対的レビューで収束）。実装未着手** |
| [131](131-deferred-timer-replay-shares-stale-live-phys-snapshot.md) | `deferred_engine_timers`のreplayが物理状態（modifiers/thumb）をpush時点でなくreplay時点でライブ再取得している可能性の計装ADR。挙動を変えず診断ログのみ追加し、実測で乖離頻度を確認してから修正要否を判断する方針 | **採用・実装完了、developマージ済み（診断専用、挙動変更なし）** |
| [132](132-uncorroborated-physical-ime-key-engine-lockout.md) | 不具合報告`01M1MMK8987NT5B2W73PCPZNZ1`（Windows Terminal+PowerShell、GJIで余分な「＠」出力）の根本原因調査から起票。Opus 2体（architect/premortem）敵対的レビューでv1の因果分析（drift correctionが無期限リトライしたため29秒ロックした）を訂正: 実際は`IntentStore`/`last_intent`が物理IMEキー1回の検出で30秒・`FocusChanged`でしか解除されない絶対的権威を獲得し、observedを一切見ずに`desired_open`をピン留めする構造が真因（29秒のうちdrift correctionが説明できるのは9.5秒のみ、残り約17秒はフォーカス変更待ちの純粋な待機。報告ダイアログへのフォーカス移動が実際の解除トリガーだったこともログで確認）。検討した3案(A:証拠強度の分離、B:observedへの追従によるdesired_open訂正、B':明示意図の有界失効)はいずれもblocker判明——Aはtransport.rsのAllow/Suppress判定と連動しBUG-52/BUG-15追補7を再導入、BはBUG-19型再発（今回のobserved=trueの出所`ConvOpenInference`は型レベルでactuationの根拠に使用禁止と宣言済み）、B'は`last_intent`除去後に`derive_any()`がconv 1件だけでbeliefを反転させる既存挙動（BUG-26依拠）によりBと同一の実害に加えdrift correction停止によるリテラル出力固着という新たな悪化を招くと判明。v3でさらに俯瞰し「actuation対象への権威／engine活性化ゲート／証拠確度」の3関心事が1本のスカラーに同居している点を根本原因と再定義、非連続な案を含む5候補（1: engine活性化のbelief分離、2: IntentKind別TTL分割、3: 状態不確実性をUXで即可視化、4: 証拠質フィルタをobserverレイヤーへ引き上げ、5: 単発DBEキーをbelief書き込み源から恒久除外）を提示。ユーザー依頼によりOpus 2体で候補1「矛盾検出中ラッチ」を2ラウンド討論——architectがヒステリシス付きの詳細設計(開く条件1つ・閉じる条件3つ・意図的に閉じない条件5つ)まで具体化したが、premortemの最終検証で(a)区間全体でdrift correctionが actuation-quietにならず変換しながらVK_IME_OFFを撃ち続ける、(b)開閉の非対称の向きが安全側と逆で誤って開いたことを検出できない、という2つのblockerが残ると確定し不採用。「そもそもobserved=trueの方が正しいという前提自体がBUG-68に照らすと偽の可能性が高い」との指摘も | **Phase 1・Phase 2ともに実装済み・敵対的コードレビュー収束済み（v5、develop未マージ、実機ソーク未実施）**。Phase 1: 候補3(UX可視化)+診断ログ7項目のみ採用しCodexへ実装委譲。BUG-110として記録。3件の実機再現を経て根本原因はdesired_open()/effective_open()/warmup_ime_on()という三重SSOTの競合と確定、実IME書き込み全経路の棚卸しをやり直しwarrant非経由の経路が新たに4系統(B1〜B4、最重要はwarmup経由のB1)見つかった。Phase 2: B1(`send_eager_tsf_warmup`)を対象にOpus2体で追加討論——v1(`desired_open`ゲート)はfocus跨ぎでstaleな値を修正根拠に使うblockerで却下、v2(`check_drift_correction()`の戻り値でゲート、INV-B1')で収束・実装。IntentStoreベースの代替案は実測データ(TTL30秒 vs 乖離365秒)から不採用。実装後、独立した読み取り専用Opusエージェントによる敵対的コードレビューを2ラウンド実施——round1で`on_ime_applied`のfrom_actuated経路がゲート未通過だった等9件、round2でそのround1修正自体が持ち込んだdedupロジックのフラップバグ等7件を発見・修正、収束確認済み。#6(`apply_force_on_for_imm_broken`)との競合は未解決のまま残る、B1由来の内訳確定は次回実機報告待ち |
| [136](136-duplicate-immcross-probe-on-focus-change.md) | ユーザー依頼の「二重のactuation/probe」全体調査から出発。`AppImeProfile::Standard`のフォーカス変更で`read_ime_state_full_async()`が経路A(`spawn_ime_refresh`prefetch)・経路B(`on_focus_process_changed`内独立spawn)の2回発行される「無意味な重複」という当初仮説を立てたが、Opus敵対的レビュー（読み取り専用、実コード照合）で反証: 経路B=High confidence(`ImmCrossProbe`)、経路A=Medium confidence(`ObserverPoll`)で対等でない上、Alt-Tab等の典型ケース(idle~50ms<`TYPING_IDLE_MS`=500ms)では`SkipTyping`戦略により経路Aのsnap_Aが丸ごと消費されず経路Bだけが唯一の観測源になることが判明（候補1「Prefetched時に経路Bスキップ」はまさにこの典型ケースを狙い撃ちで潰す誤った決定、候補2「経路B削除」も同様に不採用）。epoch/fence照合も経路Aの方が構造的に弱いと判明。反証の過程でBUG-78(`disable_apps`)対象アプリでも経路Bが抑止されない非対称という別の実害候補を副産物として発見。副次的発見(probe_io.rsのpollループ重複、`ime_mode_focus_gen`のIME種別切替非対応)は当初仮説の反証と無関係に記録を継続 | **却下（変更なし）。副産物のBUG-78非対称は別課題として切り出し** |
| [135](135-generic-thumb-key-ime-toggle-delegate.md) | BUG-115（GJIの「変換」キーでIME復帰しても親指シフトに戻らない不具合報告）の調査から出発し、当初想定より大幅にスコープ拡大。Phase 1: `awase-gji-config`の`session_keymap`フィールド番号誤り(22→41、本家`config.proto`取得で実証)・`overlay_keymaps`(field 68)未対応を修正、Opus敵対的レビュー(コード2ラウンド)で収束。続けてADR-092 Step4b(delegate-to-open-axis)をGJI向けに対称配線する設計をOpus敵対的レビュー(設計2ラウンド)にかけ、overlay/CUSTOM literalトークン/ATOKプリセット静的知識の3情報源を優先順位付きで統合する`classify_mode_key_ime_action`を実装（`Toggle`のみ`gji_thumb_key_ime_toggle`のopt-inゲート対象、`On`/`Off`は常時安全）。ユーザー指摘でHiragana/Katakanaへ拡張（ms-ime.tsv/mobile.tsvがDirectInputでIMEOn、ATOKより発生条件が緩いと判明）、暫定的に親指キー時は警告のみで実装。副次的に設定UIの`keys.ime_detect`消失バグ(stale read-modify-write)も発見・修正。Phase 2: 「ひらがなは一例、変換/無変換以外の全キーを考慮せよ」というユーザー指摘を受け、Mozc本家キー語彙全数(Eisu/Kanji/Hankaku-Zenkaku/ON/OFF含む)を再調査、`nicola_fsm.rs`の`muhenkan_vk`/`henkan_vk`2フィールド決め打ちを「左右親指スロット」汎用フィールドへ書き換える設計を起票（`dedicated_fn_key`/`mode_key_config`は無変換/変換固有のまま維持、delegate-to-open-axisの参照元だけ汎用化）。BUG-14の4エイリアス除外(Kanji/Hankaku-Zenkaku/ON/OFF)はactuation-autoからは維持しつつ、親指キー時はdelegate経由で対応 | **Phase 1: 実装済み・Opus敵対的レビュー(コード2ラウンド+設計2ラウンド+ホリスティック1ラウンド)収束済み、develop未マージ。ホリスティックレビューでHiragana/Katakanaのactuation-auto配線が既存shadow-toggle機構と二重actuationするバグが発覚し撤去済み。Phase 2(当初のnicola_fsm左右親指スロット汎用化案)はOpus設計レビューで前提誤りと判明し撤回、既存shadow_action(vk.rs::ImeKeyKind::from_vk)をGJI検出値でオーバーライドする方式へ再設計。対象をHiragana/Katakanaの2キーに絞り込み(Eisu/Kanji/ON-OFFはリスク・死コード等の理由でスコープ外)、Opus敵対的設計レビュー3ラウンドで収束(1周目でブロッカー発覚: 親指キー構成では毎打鍵IME OFF/反転を作るため、親指キーでない場合のみ適用する条件を追加。2周目で親指キー判定を書き込み時ではなく消費時に移す訂正)。この結果Phase 2はBUG-115の元シナリオ(ひらがな/カタカナを親指キーにしている場合)自体は救えないという限界が判明、将来課題として持ち越し。実装未着手、両Phaseとも実機ソーク未実施** |

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
