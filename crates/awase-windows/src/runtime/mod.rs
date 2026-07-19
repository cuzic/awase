pub(crate) mod executor;
mod focus_tracker;
mod focus_tracking;
mod ime_coordinator;
mod ime_refresh;
mod key_pipeline;
pub(crate) mod message_handlers;
pub(crate) mod outbox;
mod transport;

pub(crate) use transport::{PassthroughQueue, PhysicalKeyDisposition};

use crate::focus::FocusKind;
use awase::config::ValidatedConfig;
use awase::engine::{Engine, EngineCommand, InputContext, InputModeState, SpecialKeyCombos};
use awase::ngram::NgramModel;
use awase::types::{ContextChange, RawKeyEvent, VkCode};

use crate::focus::cache::DetectionSource;
use crate::focus::classifier::InjectionHint;
use crate::platform::WindowsPlatform;
use crate::runtime::executor::ImeApplyPair;
use awase::platform::PlatformRuntime as _;

/// IME 状態と修飾キースナップショットから `InputContext` を構築する。
///
/// `modifiers` はフック時点でキャプチャした `ModifierState` を渡すこと。
/// タイマー等のイベント非同期パスでは呼び出し元が `read_os_modifiers()` で取得する。
///
/// `ime_on` は呼び出し元が `platform_state.ime.effective_open()` を評価して渡す。
/// `input_mode` は `ImeStateHub::input_mode()`（SSOT = `shadow_model.input_mode`）から取得する。
/// `is_japanese_ime` は `ImeBelief::is_japanese_ime()` から取得する。
/// `composing` は呼び出し元が `tsf::observer::ime_composition_active_now()` を評価して渡す。
#[must_use]
pub const fn build_input_context(
    ime_on: bool,
    input_mode: InputModeState,
    is_japanese_ime: bool,
    composing: bool,
    modifiers: &awase::engine::ModifierState,
) -> InputContext {
    InputContext {
        ime_on,
        input_mode,
        is_japanese_ime,
        composing,
        modifiers: *modifiers,
        left_thumb_down: None,
        right_thumb_down: None,
    }
}
use awase::yab::YabLayout;

use crate::hook::CallbackResult;
use executor::DecisionExecutor;

// ── LayoutEntry（名前付きレイアウトエントリ）──

/// レイアウト設定一式を保持する構造体
#[derive(Debug)]
pub struct LayoutEntry {
    pub name: String,
    pub layout: YabLayout,
}

/// `[[post_bypass]]` 設定のコンパイル済みエントリ。
///
/// Ctrl+`vk` が PassThrough になった直後、`process`/`class` が一致していれば
/// `platform_state.post_bypass_passthrough` フラグをセットする。
#[derive(Debug, Clone)]
pub(crate) struct PostBypassEntry {
    pub(crate) vk: VkCode,
    /// 小文字化済みプロセス名フィルタ（空=全アプリ）
    pub(crate) process: String,
    /// 小文字化済みクラス名フィルタ（空=全クラス）
    pub(crate) class: String,
}

impl PostBypassEntry {
    pub(crate) fn matches(&self, vk: VkCode, process: &str, class: &str) -> bool {
        self.vk == vk
            && (self.process.is_empty() || process.to_lowercase().contains(self.process.as_str()))
            && (self.class.is_empty() || class.to_lowercase().contains(self.class.as_str()))
    }
}

/// アプリケーションランタイム。
///
/// Engine (判断) と DecisionExecutor (実行) を保持し、配線する。
/// OS イベントの受け取り → Observer → Engine → Executor のパイプラインを駆動する。
///
/// # アーキテクチャ（Facade パターン）
///
/// `Runtime` は以下の論理コンポーネントへの Facade として機能する：
///
/// - [`focus_tracker::FocusTracker`] — フォーカス追跡・IMM 能力学習・sync key 補完
/// - [`ime_coordinator::ImeCoordinator`] — IME apply・パニック回復の調停
///
/// コンポーネント間の相互参照はなく、`Runtime` を介してのみ通信する。
///
/// 注意: 判断ロジックを追加しないこと。判断は Engine が担う。
pub struct Runtime {
    engine: Engine,
    executor: DecisionExecutor,
    pub platform: WindowsPlatform,
    layouts: Vec<LayoutEntry>,
    /// フォーカス追跡・IMM 能力学習・sync key 補完
    focus_tracker: focus_tracker::FocusTracker,
    /// Platform 層の全状態
    platform_state: crate::PlatformState,
    /// 全キーマップルール（アプリフィルタ前）
    all_keymaps: crate::keymap::KeymapTable,
    /// post_bypass コンパイル済みルール一覧
    pub(crate) post_bypass_rules: Vec<PostBypassEntry>,
    /// IME apply・パニック回復の調停
    ime_coordinator: ime_coordinator::ImeCoordinator,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime").finish_non_exhaustive()
    }
}

/// `ime_diagnostic` が必要とする Runtime の読み取り専用スナップショット。
#[derive(Clone)]
pub(crate) struct RuntimeDiagnosticSnapshot {
    pub focus_pid: u32,
    pub focus_class: String,
    pub shadow_ime_on: bool,
    pub shadow_is_romaji: bool,
    pub shadow_is_japanese: bool,
    pub last_focus_change_ms: u64,
    pub last_hook_activity_ms: u64,
    pub app_profile: String,
}

impl Runtime {
    pub(crate) fn build_ctx(&self) -> InputContext {
        // SAFETY: `read_os_modifiers` は Win32 `GetKeyState` を呼ぶのみで副作用はない。
        //         メインスレッドから呼ばれるため、スレッド要件を満たしている。
        let modifiers = unsafe { crate::observer::focus_observer::read_os_modifiers() };
        build_input_context(
            self.platform_state.ime.effective_open(),
            self.platform_state.ime.input_mode(),
            self.platform_state.ime.belief.is_japanese_ime(),
            crate::tsf::observer::ime_composition_active_now(),
            &modifiers,
        )
    }

    /// output 層が注入モードを決定するために呼ぶ公開 API。
    ///
    /// focus の `injection_hint()` と `platform_state.app_kind` を組み合わせて
    /// `InjectionHint` を返す。output 層はこのメソッドのみを呼び、
    /// focus/classify の内部型に直接アクセスしない。
    #[must_use]
    pub fn injection_hint(&self) -> (InjectionHint, crate::focus::AppKind) {
        (
            self.platform.injection_hint(),
            self.platform_state.focus.app_kind,
        )
    }

    /// 現在フォーカス中のアプリが IMM32 クロスプロセス制御を使えるか返す。
    #[expect(clippy::missing_const_for_fn)]
    #[must_use]
    pub fn can_use_imm32_cross_process(&self) -> bool {
        self.platform
            .current_app_profile()
            .can_use_imm32_cross_process()
    }

    /// IMM 検出の前後ミス数から、クラス名単位の IMM 能力をキャッシュに記録する。
    ///
    /// 判定は [`FocusTracker::decide_imm_capability`]（純粋関数）に委譲し、
    /// ここではクラス名取得・キャッシュ書き込み・ログの I/O のみ行う。
    pub fn learn_imm_capability_from_miss(&mut self, miss_before: u32, miss_after: u32) {
        if !self.platform.focus.is_focused() {
            return;
        }
        let class_name = self.platform.focus.class_name().to_owned();
        let current = self.platform.focus.imm_capability(&class_name);
        if let Some(new_cap) = focus_tracker::FocusTracker::decide_imm_capability(
            miss_before,
            miss_after,
            crate::IME_DETECT_MISS_THRESHOLD,
            current,
        ) {
            log::info!(
                "IMM capability learned: {class_name} → {new_cap:?} (miss {miss_before}→{miss_after})"
            );
            self.platform.learn_imm_capability(class_name, new_cap);
        }
    }

    /// IME 関連の事前分類情報を sync key 設定で補完する。
    ///
    /// 実処理は [`focus_tracker::FocusTracker::enrich_ime_relevance`] に委譲する。
    pub fn enrich_ime_relevance(&self, event: &mut RawKeyEvent) {
        self.focus_tracker.enrich_ime_relevance(event);
    }

    /// Decision の副作用を実行する（メッセージループ用）。
    /// `suppress_engine_state_key = true` で囲んで decision を実行する。
    ///
    /// ポーリング / フォーカス変化起因の RefreshState で使う。
    /// Kanji 等の sync key がすでに IME を正しい状態にしているとき、
    /// `engine_on/off_ime_key`（VK_DBE_DBCSCHAR 等）を追加送信してしまう
    /// フィードバックループを防ぐ。
    pub fn execute_decision_suppressed(
        &mut self,
        decision: awase::engine::Decision,
    ) -> CallbackResult {
        let _guard = self.platform.suppress_engine_state_key_guard();
        self.execute_decision(decision)
    }

    /// `ImeCoordinator::pending_ime_off_rescue` を取り出し、`TIMER_IME_OFF_RESCUE` をキャンセルする。
    ///
    /// `.take()` と `timer.kill()` は常にペアで呼ぶ必要があるため一元化する。
    pub fn take_ime_off_rescue_pending(&mut self) -> Option<RawKeyEvent> {
        self.platform.timer.kill(crate::TIMER_IME_OFF_RESCUE);
        self.ime_coordinator.pending_ime_off_rescue.take()
    }

    /// `ImeCoordinator::pending_ime_off_rescue` をセットし、`TIMER_IME_OFF_RESCUE` を起動する。
    ///
    /// `.pending = Some(event)` と `timer.set()` は常にペアで呼ぶ必要があるため一元化する。
    pub fn set_ime_off_rescue_pending(&mut self, event: RawKeyEvent) {
        self.ime_coordinator.pending_ime_off_rescue = Some(event);
        self.platform.timer.set(
            crate::TIMER_IME_OFF_RESCUE,
            std::time::Duration::from_millis(50),
        );
    }

    pub fn execute_decision(&mut self, decision: awase::engine::Decision) -> CallbackResult {
        let (callback, sync_outcomes, stripped_set_open) =
            self.executor
                .execute_from_loop(&mut self.platform, &self.platform_state.ime, decision);
        self.dispatch_outcomes(sync_outcomes);
        if stripped_set_open.is_some() {
            // settle 中に握りつぶした SetOpen は自然には再発行されない
            // （Engine::prev_activation は遷移確定済みのため）。既存の
            // apply_force_on_for_imm_broken 等と同じ「settle 明けに refresh で再試行」
            // パターンで確実に一度だけ再同期する。
            let retry_ms = self.platform_state.ime.focus_settle_ms() + 50;
            log::debug!(
                "[focus-settle] SetOpen stripped from execute_from_loop decision → \
                 {retry_ms}ms 後に refresh で再試行"
            );
            self.schedule_ime_refresh(retry_ms);
        }
        callback
    }

    /// IME apply 完了後の後処理 SSOT。sync / async 両経路から呼ばれる。
    ///
    /// - D: generation 照合で `ImeApplySucceeded` / `ImeApplyFailed` を dispatch
    /// - E: `post_ime_refresh` で IME 状態ポーリングをスケジュール
    ///
    /// sync 経路では `execute_one` が `post_apply_ime_open`（B）を済ませた後、
    /// 呼び出し元が sync_outcomes ループ経由でここへ来る。
    /// async 経路では spawn_local 内で B を済ませた後に直接呼ばれる。
    pub fn on_ime_apply_complete(
        &mut self,
        open: bool,
        outcome: awase::platform::ImeOpenOutcome,
        generation: Option<u64>,
    ) {
        use awase::platform::{ImeOpenOutcome, TsfComposition as _};

        if outcome == ImeOpenOutcome::UnsafeToToggle {
            return;
        }

        // C+D: ImeModel write-back + generation 照合 dispatch
        let accepted = self.platform_state.ime.record_ime_apply_result(
            open,
            outcome,
            generation,
            crate::hook::current_tick_ms(),
        );

        // B: composition warm/cold 更新。stale apply 完了は GJI/Composition に伝播させない。
        if accepted {
            self.platform.on_ime_applied(open, outcome);
        }

        // E: IME 状態ポーリングをスケジュール
        self.platform.post_ime_refresh();
    }

    /// sync path の outcome リストを一括 dispatch する。
    pub(crate) fn dispatch_outcomes(&mut self, outcomes: Vec<ImeApplyPair>) {
        for completion in outcomes {
            self.on_ime_apply_complete(completion.open, completion.outcome, completion.generation);
        }
    }

    /// 現在の shadow model から `ImeControlView` を構築する。
    pub(crate) fn shadow_ime_control_view(&self) -> crate::state::ImeControlView<'_> {
        let mut view = self
            .platform
            .build_ime_control_view(self.platform_state.ime.model().applied_pair());
        view.belief_input_mode = self.platform_state.ime.input_mode();
        view
    }

    /// エンジンの有効/無効を切り替え、Decision を実行する
    pub fn toggle_engine(&mut self) {
        let ctx = self.build_ctx();
        let decision = self.engine.on_command(EngineCommand::ToggleEngine, &ctx);
        self.execute_decision(decision);
    }

    /// エンジンを無条件で ON にする（トグルではなく強制）。
    /// トレイの「状態をリセット」等、現在の ON/OFF に関わらず必ず有効化したい場合に使う。
    pub fn force_engine_on(&mut self) {
        let ctx = self.build_ctx();
        let decision = self.engine.on_command(EngineCommand::ForceEngineOn, &ctx);
        self.execute_decision(decision);
    }

    /// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
    pub fn invalidate_engine_context(&mut self, reason: ContextChange) {
        let ctx = self.build_ctx();
        let decision = self
            .engine
            .on_command(EngineCommand::InvalidateContext(reason), &ctx);
        self.execute_decision(decision);
    }

    /// IME 状態とフォーカス状態を一括で再観測し、Engine に通知する。
    ///
    /// フォーカスデバウンス後・500ms ポーリング・may_change_ime 後など、
    /// 全ての IME/フォーカス更新がこのメソッドに集約される（ADR 028）。
    ///
    /// 処理フロー:
    /// 1. 現在のフォーカス先を取得・分類（focus_kind, app_kind 更新）
    /// 2. 前面プロセスが変わった場合は Engine に FocusChanged（flush あり）
    /// 3. IME 状態を再取得して Preconditions を更新
    /// 4. Engine に RefreshState（active 状態の遷移検知）
    /// 5. 次回ポーリングを自動スケジュール
    ///
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub fn refresh_ime_state_cache(&mut self) {
        self.run_ime_refresh();
    }

    /// IME リフレッシュを非同期タスクとしてスポーン。
    /// with_app の外でフェッチを行い、完了後に with_app で適用する。
    pub fn spawn_ime_refresh(&mut self) {
        self.platform.timer.kill(crate::TIMER_IME_REFRESH);

        // NOTE: ここで send_eager_tsf_warmup() を呼ばない。
        // focus_transition_pending=true の時点では injection_mode が前ウィンドウ（WezTerm 等）
        // の stale な Tsf のままであり、新しいウィンドウが Chrome/Edge の場合に誤って
        // VK_DBE_HIRAGANA を送信して Chrome の IME を ON にしてしまうバグがあった。
        // eager warmup は post_focus_change_snapshot (run_with_prefetched 内) で injection_mode
        // 確定後に正しく送信される。

        win32_async::spawn_local(async {
            let focus = crate::focus::probe::run_focus_probe_async().await;
            let snap = crate::ime::read_ime_state_full_async().await;
            let _ = crate::with_app(|app| {
                app.run_ime_refresh_with_prefetched(focus, &snap);
                app.settle_tsf_gate_after_refresh();
            });
        });
    }

    /// 統合 IME リフレッシュタイマーをスケジュール（リセット）する。
    ///
    /// 既存のタイマーをキャンセルして `delay_ms` 後に再設定する。
    /// フォーカス変更(50ms) / ポーリング(500ms) / 即時(0ms) を統一的に扱う。
    pub fn schedule_ime_refresh(&mut self, delay_ms: u64) {
        self.platform.timer.set(
            crate::TIMER_IME_REFRESH,
            std::time::Duration::from_millis(delay_ms),
        );
    }

    /// ポーリング間隔設定に従って次回 IME リフレッシュをスケジュールする。
    pub fn reschedule_ime_refresh(&mut self) {
        // TsfNative は read_ime_state_full が常に None、GJI も predates-focus-change でスキップ。
        // explicit_intent の有無に関わらずポーリングで得られる情報がないため常に停止する。
        // explicit_intent が確定している他プロファイルも同様に停止。
        // 再開トリガー: フォーカス変更 / may_change_ime キー（20ms タイマー）
        let is_tsf_native = crate::focus::class_names::is_effectively_tsf_native(
            self.platform.current_app_profile(),
            self.platform.focus.class_name(),
        );
        if is_tsf_native || self.platform_state.ime.explicit_intent().is_some() {
            return;
        }
        self.schedule_ime_refresh(u64::from(self.platform_state.focus.ime_poll_interval_ms));
    }

    /// `spawn_ime_refresh` の async タスク内で IME リフレッシュ後に TsfGate を遷移させる。
    ///
    /// `run_ime_refresh_with_prefetched` 完了後に呼ぶ。`last_focus_info` が更新済みのため
    /// `injection_hint` を読んで正しい TsfGate 状態に遷移できる。
    fn settle_tsf_gate_after_refresh(&mut self) {
        // PendingWarmup 以外（Probing/Ready/Bypass）なら空 Vec が返る。
        // confirm_tsf は PendingWarmup/Bypass → Probing、bypass_tsf は PendingWarmup/Probing → Bypass。
        let is_tsf = matches!(self.platform.injection_hint(), InjectionHint::ForceTsf);
        let held = if is_tsf {
            self.platform.confirm_tsf()
        } else {
            // ここで belief（IME open observation）を書いてはならない。
            // かつて ce45b82 が「非TSFウィンドウには日本語IMEが存在しない」という誤った
            // 前提で write_focus_probe(false) の偽観測を注入していたが、Edge/Chrome
            // （Imm32Unavailable, injection=Unicode）は非TSF注入かつ日本語IME有効であり、
            // 実観測経路を持たないため偽 Low false が most_recent_trusted() 経由で belief を
            // 支配し、フォーカス約500ms後（次ポーリング）に Engine が必ず OFF になった
            // （docs/known-bugs.md BUG-07）。ce45b82 の元バグ（Win+X メニューの1文字
            // ショートカットが NICOLA 変換される）は、現在は classify.rs の既知 NonText
            // クラス判定 + message_handlers.rs の NonText パススルーが belief と独立に防ぐ。
            self.platform.bypass_tsf()
        };
        self.platform.timer.kill(crate::TIMER_TSF_GATE);
        if !held.is_empty() {
            log::debug!(
                "[tsf-gate] draining {} held keys via INPUT_DEFER",
                held.len()
            );
            crate::INPUT_DEFER.replay_later(held);
        }
    }

    /// IME を実際に ON/OFF する直接呼び出し（`Decision`/`Effect` を経由しない経路）が、
    /// フォーカス遷移の settle 期間中に実行されるべきでないかどうかを判定する。
    ///
    /// `execute_decision`/`execute_decision_suppressed` 経由の `Decision` ベースの経路は
    /// `Executor::execute_from_loop` が一括でガードするが、`platform.set_ime_open` や
    /// `apply_ime_open_with_applied` を直接呼ぶ経路（`apply_force_on_for_imm_broken`,
    /// `try_force_on_bootstrap`, `ir_apply_drift_correction`,
    /// `ir_post_focus_change_snapshot` 内の GJI 強制 ON / IME OFF 強制ブロック）は
    /// `Decision`/`Effect` という抽象を経由しないためそちらのガードが効かない。
    /// これらの呼び出し元は実行前に必ずこれを確認すること。
    ///
    /// 2026-07-05: Alt+Tab 中間ウィンドウへの一瞬のフォーカス中に、これらの直接呼び出しが
    /// settle 前の不安定な状態に基づいて IME を実際に切り替えてしまうバグの修正。
    pub(crate) fn ime_apply_should_defer(&self) -> bool {
        self.platform_state
            .ime
            .is_focus_transition_settling(std::time::Instant::now())
    }

    /// Blacklist アプリ（Chrome 等）で IME belief が ON のとき OS に force-ON を送る。
    ///
    /// IMM クロスプロセスが使えるアプリ（通常 IMM アプリ）では何もしない。
    pub fn apply_force_on_for_imm_broken(&mut self) {
        if self.can_use_imm32_cross_process() {
            return;
        }
        if self.ime_apply_should_defer() {
            // settle 中のスキップは必ず settle 明けに refresh で再試行する。
            // 再試行がないと「belief ON × 実 IME OFF」のまま次の refresh（無保証、
            // 実測で 8 秒後）まで放置され、最初の打鍵が閉じた IME にリテラル着弾する
            // （2026-07-07 実機: 仮想デスクトップ切替 → Windows Terminal で
            // 「これで」が「korede」化。TsfNative は open 状態を読めないため
            // 観測での自己修復も効かない）。遅延は settle 残余の上限
            // （= focus_settle_ms）+ タイマー粒度マージン 50ms。
            let retry_ms = self.platform_state.ime.focus_settle_ms() + 50;
            log::debug!(
                "[focus-settle] apply_force_on_for_imm_broken skipped (settling) → \
                 {retry_ms}ms 後に refresh で再試行"
            );
            self.schedule_ime_refresh(retry_ms);
            return;
        }
        if !(self.engine.is_user_enabled()
            && self.platform_state.ime.is_eligible_for_ime_force_on())
        {
            return;
        }
        // applied が既に ON なら送らない（500ms poll ごとの F2 再送スパム防止）。
        // FocusChange が applied=Unknown にリセットするため、フォーカスごとに
        // 1 回だけ force-apply される。Win-held スキップ（UnsafeToToggle）や
        // 失敗時は applied が更新されないため次の refresh が再試行する。
        if matches!(
            self.platform_state.ime.model().applied,
            crate::state::ime_model::AppliedImeState::Optimistic(true)
                | crate::state::ime_model::AppliedImeState::Confirmed { open: true, .. }
        ) {
            return;
        }
        // `platform.set_ime_open` は IMM 専用実装で、Imm32Unavailable / TSF-native
        // プロファイルでは早期 return する — つまり **この関数が対象とする Blacklist
        // アプリで常に no-op だった**（2026-07-07 実機: BUG-16 の settle 明け再試行が
        // 律儀に走っても実 IME OFF が直らず「koreha」リテラル化が再発。
        // 手動 Ctrl+変換 = strategy chain 経由の apply は毎回効いていた）。
        // strategy chain（MsImeDirect の冪等 VK_DBE_HIRAGANA 等）で apply する。
        let belief = crate::output::OpenBelief {
            effective_open: true,
            confident: true,
        };
        let outcome = self.platform.apply_ime_open_with_belief(true, None, belief);
        log::info!("Blacklist force-ON: apply_ime_open(true) → {outcome:?}");
        self.on_ime_apply_complete(true, outcome, None);
        if !self.platform_state.ime.input_mode().is_romaji_capable() {
            if let Some(new_mode) = self.platform_state.ime.correction_for_imm_broken() {
                log::info!(
                    "Blacklist force-ON: input_mode → AssumedRomaji (IMM broken, ime_on=true)"
                );
                let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
                self.platform_state.ime.dispatch_event(
                    crate::state::ime_event::ImeEvent::InputModeApplied {
                        mode: new_mode,
                        strategy:
                            crate::state::ime_event::InputModeApplyStrategy::ImmBrokenCorrection,
                        result: crate::state::ime_event::InputModeApplyResult::Applied,
                        at: tick_ms,
                    },
                    tick_ms,
                );
            } else {
                // romaji-capable は外側の if で除外済みなので None = ObservedEisu のみ
                log::info!(
                    "Blacklist force-ON: input_mode スキップ (belief=ObservedEisu, eisu guard)"
                );
            }
        }
    }

    /// 未知 Imm32Unavailable アプリで IME 検出が連続失敗したとき、一時 force-ON を試みる。
    pub fn try_force_on_bootstrap(&mut self) {
        if self.platform_state.ime.detect_miss_count() >= crate::IME_DETECT_MISS_THRESHOLD
            && self.engine.is_user_enabled()
            && self.platform_state.ime.is_eligible_for_ime_force_on()
            && !self.platform_state.ime.is_force_on_guard_active()
        {
            if self.ime_apply_should_defer() {
                // apply_force_on_for_imm_broken と同じく settle 明けに必ず再試行する。
                let retry_ms = self.platform_state.ime.focus_settle_ms() + 50;
                log::debug!(
                    "[focus-settle] try_force_on_bootstrap skipped (settling) → \
                     {retry_ms}ms 後に refresh で再試行"
                );
                self.schedule_ime_refresh(retry_ms);
                return;
            }
            log::warn!(
                "IME detection failed {} times, forcing OS ime_on=true (shadow=ON)",
                self.platform_state.ime.detect_miss_count()
            );
            // set_ime_open は IMM 専用実装で Imm32Unavailable では常に no-op
            // （apply_force_on_for_imm_broken と同じ穴）。strategy chain で apply する。
            let belief = crate::output::OpenBelief {
                effective_open: true,
                confident: true,
            };
            let outcome = self.platform.apply_ime_open_with_belief(true, None, belief);
            log::info!("force-on bootstrap: apply_ime_open(true) → {outcome:?}");
            self.on_ime_apply_complete(true, outcome, None);
            self.platform_state.ime.set_force_on_broken_app_bootstrap();
        }
    }

    /// 配列を動的に切り替える
    pub fn switch_layout(&mut self, index: usize) {
        let Some(entry) = self.layouts.get(index) else {
            log::warn!("Layout index {index} out of range");
            return;
        };

        let name = entry.name.clone();
        let decision = self.engine.on_command(
            EngineCommand::SwapLayout(entry.layout.clone()),
            &self.build_ctx(),
        );
        self.execute_decision(decision);

        self.platform.tray.set_layout_name(&name);

        log::info!("Switched layout to: {name}");
    }

    /// 手動アプリオーバーライドのトグル処理
    pub fn toggle_app_override(&mut self) {
        let current = self.platform_state.focus.focus_kind;
        let new_kind = if current == FocusKind::TextInput {
            FocusKind::NonText
        } else {
            FocusKind::TextInput
        };

        self.platform_state.focus.focus_kind = new_kind;

        // Update learning cache
        if self.platform.focus.is_focused() {
            let pid = self.platform.focus.pid();
            let cls = self.platform.focus.class_name().to_owned();
            self.platform
                .focus
                .cache_insert(pid, cls, new_kind, DetectionSource::UserOverride);
        }

        // If demoted to NonText, flush engine pending
        if new_kind == FocusKind::NonText {
            self.invalidate_engine_context(ContextChange::FocusChanged);
        }

        // バルーン通知を表示
        self.platform.tray.show_balloon(
            "awase",
            if new_kind == FocusKind::TextInput {
                "テキスト入力モードに切り替えました"
            } else {
                "バイパスモードに切り替えました"
            },
        );

        let mode_str = if new_kind == FocusKind::TextInput {
            "TextInput (engine enabled)"
        } else {
            "NonText (engine bypassed)"
        };
        log::info!("Manual focus override: → {mode_str}");
    }

    /// Sync key 後に遅延されたキーを再処理する。
    ///
    /// sync key で guard が起動された後、KeyUp で OS が IME を切り替えてから呼ばれる。
    /// guard 解除 → IME 状態 refresh → Engine 通知 → バッファキー再処理。
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub fn process_deferred_keys(&mut self) {
        // Guard を解除し、保留キーを回収
        let keys = self.platform_state.gate.sync_key_gate.deactivate();
        log::debug!("IME guard OFF (process_deferred_keys)");

        // Refresh IME state (Observer → ImeObservations → Preconditions)
        // SAFETY: `poll_and_classify_ime` は Win32 IMM API（`ImmGetContext` 等）を呼ぶ unsafe fn。
        //         メッセージループ上（メインスレッド）から呼ばれるためスレッド要件を満たす。
        let observer_out = unsafe {
            crate::observer::ime_observer::poll_and_classify_ime(
                self.platform_state.ime.effective_open(),
                self.platform_state.ime.is_force_on_guard_active(),
                self.platform_state.ime.input_mode(),
                self.platform_state.ime.belief.prev_conversion_mode(),
            )
        };
        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        let accepted = crate::state::probe_admission::AcceptedObservation::for_sync(
            self.platform_state.focus.focus_epoch,
        );
        self.platform_state
            .ime
            .apply_ime_update(&observer_out, tick_ms, accepted);

        // LastAppliedImeState を OS 観測値に同期する。
        // 物理 Kanji キー（sync key）は apply_ime_open を経由しないため last_applied が更新されない。
        // last_applied が stale なまま Engine が activate → SetOpen(true) → KanjiToggleStrategy が
        // last_applied(false) != desired(true) と判定して VK_KANJI を余分に送信し、
        // Chrome では IME が逆転するバグを防ぐ。
        let observed_ime_on = self.platform_state.ime.effective_open();
        self.platform_state
            .ime
            .mirror_applied_open(observed_ime_on, tick_ms);
        log::debug!("[process-deferred] applied_open → {observed_ime_on} (sync with OS poll)");

        // Engine に IME 状態変化を即通知する（deferred keys の有無にかかわらず）。
        // suppress_engine_state_key = true: sync key（Kanji 等）がすでに IME を正しい状態に
        // 設定しているため、engine_on/off_ime_key（VK_DBE_DBCSCHAR 等）を追加送信しない。
        // 送ると IME モードが ひらがな→全角英数 等に意図せず変わる可能性がある。
        {
            let ctx = self.build_ctx();
            let decision = self.engine.on_command(EngineCommand::RefreshState, &ctx);
            self.execute_decision_suppressed(decision);
        }

        if keys.is_empty() {
            return;
        }

        log::debug!("Processing {} deferred key(s) after IME toggle", keys.len());

        for (event, _phys) in keys {
            // Build fresh context with updated preconditions
            let ctx = self.build_ctx();
            let decision = self.engine.on_input(event, &ctx);
            self.execute_decision(decision);
        }
    }

    // ── app/ 境界 API（private フィールドへのアクセスを app/ に許可しない）──

    /// Runtime を初期化して返す。
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn new(
        engine: Engine,
        executor: DecisionExecutor,
        platform: WindowsPlatform,
        layouts: Vec<LayoutEntry>,
        sync_toggle_keys: Vec<VkCode>,
        sync_on_keys: Vec<VkCode>,
        sync_off_keys: Vec<VkCode>,
        platform_state: crate::PlatformState,
        all_keymaps: crate::keymap::KeymapTable,
        post_bypass_rules: Vec<PostBypassEntry>,
    ) -> Self {
        Self {
            engine,
            executor,
            platform,
            layouts,
            focus_tracker: focus_tracker::FocusTracker::new(
                sync_toggle_keys,
                sync_on_keys,
                sync_off_keys,
            ),
            platform_state,
            all_keymaps,
            post_bypass_rules,
            ime_coordinator: ime_coordinator::ImeCoordinator::new(),
        }
    }

    /// 利用可能なレイアウト名の一覧を返す（トレイメニュー表示用）。
    pub(crate) fn layout_names(&self) -> Vec<String> {
        self.layouts.iter().map(|e| e.name.clone()).collect()
    }

    /// トレイアイコンの HWND を返す。
    pub(crate) const fn tray_hwnd(&self) -> windows::Win32::Foundation::HWND {
        self.platform.tray.hwnd()
    }

    /// ウィンドウフォーカス変更イベントを処理する（`win_event_proc` から呼ぶ）。
    pub(crate) fn on_window_focus_event(
        &mut self,
        hwnd_id: crate::state::ime_event::HwndId,
        now: std::time::Instant,
    ) {
        self.platform_state
            .ime
            .try_set_focus_transition_barrier(hwnd_id, now);

        // デバウンスタイマー（~50ms）が完了する前にキーが来た場合でも injection_mode が
        // 正しくなるよう、フォーカス変更直後に新ウィンドウの class/pid から同期更新する。
        // WezTerm(ForceTsf) → Chrome 等の遷移でも hint を新ウィンドウから引くため stale にならない。
        {
            let hwnd = hwnd_id.to_hwnd();
            let class_name = crate::focus::classify::get_class_name_string(hwnd);
            if !class_name.is_empty() {
                let pid = crate::focus::classify::get_window_process_id(hwnd);
                let new_app_kind = crate::observer::focus_observer::detect_app_kind(&class_name);
                let hint = self.platform.injection_hint_for(pid, &class_name);
                let new_mode = crate::output::types::InjectionMode::from((hint, new_app_kind));
                self.platform.update_injection_mode(new_mode);
                log::debug!(
                    "[focus-sync] hwnd=0x{:X} class={class_name:?} \
                     app_kind={new_app_kind:?} hint={hint:?} → mode={new_mode:?}",
                    hwnd_id.0
                );
            }
        }

        self.platform.on_focus_change_tsf();
        self.platform.timer.set(
            crate::TIMER_TSF_GATE,
            std::time::Duration::from_millis(crate::tsf::WARMUP_TIMEOUT_MS),
        );
        let debounce_ms = u64::from(self.platform_state.focus.focus_debounce_ms);
        self.schedule_ime_refresh(debounce_ms);
    }

    /// フックウォッチドッグタイマーを起動する（3 秒）。
    pub(crate) fn start_hook_watchdog(&mut self) {
        self.platform.timer.set(
            crate::TIMER_HOOK_WATCHDOG,
            std::time::Duration::from_secs(3),
        );
    }

    /// UIA ワーカースレッドへの送信チャネルを登録する。
    pub(crate) fn set_uia_sender(
        &mut self,
        tx: std::sync::mpsc::Sender<crate::focus::uia::SendableHwnd>,
    ) {
        self.platform.set_uia_sender(tx);
    }

    /// システムトレイのバルーン通知を表示する。
    pub(crate) fn show_tray_balloon(&mut self, title: &str, text: &str) {
        self.platform.tray.show_balloon(title, text);
    }

    /// IMM 能力学習キャッシュをクリアして削除件数を返す。
    pub(crate) fn clear_imm_learning(&mut self) -> usize {
        self.platform.focus.clear_imm_learning()
    }

    /// 診断画面が必要とする状態を一括スナップショットとして返す。
    pub(crate) fn diagnostic_snapshot(&self) -> RuntimeDiagnosticSnapshot {
        let (focus_pid, focus_class) = if self.platform.focus.is_focused() {
            (
                self.platform.focus.pid(),
                self.platform.focus.class_name().to_owned(),
            )
        } else {
            (0, String::new())
        };
        RuntimeDiagnosticSnapshot {
            focus_pid,
            focus_class,
            shadow_ime_on: self.platform_state.ime.effective_open(),
            shadow_is_romaji: self.platform_state.ime.input_mode().is_romaji_capable(),
            shadow_is_japanese: self.platform_state.ime.belief.is_japanese_ime(),
            last_focus_change_ms: self.platform_state.focus.last_focus_change_ms,
            last_hook_activity_ms: self.platform_state.gate.last_hook_activity_ms,
            app_profile: format!("{:?}", self.platform.current_app_profile()),
        }
    }

    /// 設定リロード時に Runtime の全パラメータを一括更新する。
    ///
    /// FSM パラメータ・出力モード・同期キー・特殊キーコンボ・
    /// アプリオーバーライドをアトミックに適用する。
    pub(crate) fn apply_config_update(
        &mut self,
        config: &ValidatedConfig,
        special_keys: SpecialKeyCombos,
        sync_toggle: Vec<VkCode>,
        sync_on: Vec<VkCode>,
        sync_off: Vec<VkCode>,
    ) {
        let ctx = self.build_ctx();
        let _ = self.engine.on_command(
            EngineCommand::UpdateFsmParams {
                threshold_ms: config.general.simultaneous_threshold_ms,
                confirm_mode: config.general.confirm_mode,
                speculative_delay_ms: config.general.speculative_delay_ms,
            },
            &ctx,
        );
        self.platform_state.focus.focus_debounce_ms = config.general.focus_debounce_ms;
        self.platform_state.focus.ime_poll_interval_ms = config.general.ime_poll_interval_ms;
        self.focus_tracker.sync_toggle_keys = sync_toggle;
        self.focus_tracker.sync_on_keys = sync_on;
        self.focus_tracker.sync_off_keys = sync_off;
        let _ = self.engine.on_command(
            EngineCommand::ReloadKeys {
                special: special_keys,
            },
            &ctx,
        );
        self.platform
            .focus
            .reset_overrides(crate::focus::classifier::ForceOverrides::new(
                config.app_overrides.clone(),
            ));
        self.platform.focus.cache_reset();
        if let (Some((left, left_alt_impersonates)), Some((right, right_alt_impersonates))) = (
            crate::hook::resolve_thumb_key(&config.general.left_thumb_key),
            crate::hook::resolve_thumb_key(&config.general.right_thumb_key),
        ) {
            crate::hook::set_thumb_vk_codes(left, right);
            self.engine.set_thumb_vks(left, right);
            crate::hook::set_alt_impersonation_enabled(
                left_alt_impersonates,
                right_alt_impersonates,
            );
            log::info!(
                "Thumb keys updated: left={:?}, right={:?}",
                config.general.left_thumb_key,
                config.general.right_thumb_key,
            );
        } else {
            log::warn!(
                "Invalid thumb key names: left={:?}, right={:?}",
                config.general.left_thumb_key,
                config.general.right_thumb_key,
            );
        }
        log::info!(
            "Config applied: threshold={}ms, speculative_delay={}ms",
            config.general.simultaneous_threshold_ms,
            config.general.speculative_delay_ms,
        );
    }

    /// n-gram モデルをエンジンに適用する。
    pub(crate) fn set_ngram_model(&mut self, model: NgramModel) {
        let ctx = self.build_ctx();
        let _ = self
            .engine
            .on_command(EngineCommand::SetNgramModel(model), &ctx);
    }

    /// Output が積んだ `RuntimeRequest` を drain して処理する。
    ///
    /// キー処理境界（`WM_EXECUTE_EFFECTS` / `WM_DRAIN_OUTPUT_QUEUE` 末尾）で呼ぶ。
    ///
    /// `Output` はキー注入中に `with_app` を再入させられないため、IME リフレッシュ・
    /// TSF プローブ起動などの Runtime 操作を `RuntimeOutbox` に積んでおき、
    /// ここで一括実行する（H-4-b: `StartTsfProbe` が Chrome cold パスから積まれる）。
    pub(crate) fn drain_runtime_requests(&mut self) {
        use crate::runtime::outbox::RuntimeRequest;
        let requests = self.platform.output.take_pending_requests();
        if requests.is_empty() {
            return;
        }
        log::debug!("[runtime-outbox] {} request(s) を drain", requests.len());
        for request in requests {
            match request {
                RuntimeRequest::StartTsfProbe => {
                    log::debug!("[runtime-outbox] StartTsfProbe → pending TSF timer 適用");
                    if let Some(cmd) = self.platform.output.pending_tsf_timer() {
                        self.platform.apply_timer_command(cmd);
                    }
                }
            }
        }
    }

    /// パニックリセット: IME 関連キー連打で発動する緊急リセット。
    ///
    /// エンジン状態・IME・修飾キー・フック・キャッシュをすべて初期状態に戻す。
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub fn panic_reset(&mut self) {
        log::warn!("Panic reset triggered!");

        // 1. エンジンの保留状態をフラッシュ
        self.invalidate_engine_context(ContextChange::InputLanguageChanged);

        // 2. IME 未確定文字列をキャンセル → OFF → ON
        // SAFETY: `cancel_ime_composition` は Win32 IMM API を呼ぶ unsafe fn。
        //         `panic_reset` はメッセージループ上（メインスレッド）から呼ばれるため安全。
        unsafe { cancel_ime_composition() };
        // OFF → ON を順序保証付きで実行する。`WindowsPlatform::set_ime_open` は
        // 内部で spawn_local して fire-and-forget するため、2 連発で呼ぶと async race で
        // 順序が逆転しうる (true→false の終端で IME OFF のまま残るリスク)。単一の
        // spawn_local タスク内で 2 回 await する形にして OFF → ON を直列化する。
        if self.can_use_imm32_cross_process() {
            win32_async::spawn_local(async {
                let _ = crate::ime::set_ime_open_cross_process_async(false).await;
                let _ = crate::ime::set_ime_open_cross_process_async(true).await;
                // カタカナ・半角カタカナ状態でリセットした場合でもひらがなに戻す
                let _ = crate::ime::set_ime_hiragana_mode_cross_process_async().await;
            });
        }

        // 3. 全修飾キーの KeyUp を送信（スタック解消）
        // send_all_modifier_key_ups() は自己注入 SendInput (INJECTED_MARKER) のため
        // is_self_injected フィルタでフックの PHYSICAL_KEY_STATE 更新まで届かない
        // (ADR-054 由来の隙間、2026-07-09 発見)。OS 側の modifier は解放されるが
        // awase 内部の物理キー shadow は解放されないままだったため、明示的にリセットする。
        send_all_modifier_key_ups();
        crate::hook::reset_physical_key_state();

        // 4. PlatformState を全面リセット
        // panic_reset 直後に refresh_ime_state_cache() が走ると、ここで書いた
        // ime_on=true を stale な observe() 結果が即座に上書きしてしまう。
        // force_on_guard で 1 サイクルだけ保護し、次の検出成功時に自然に解除する。
        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        self.platform_state.ime.apply_panic_reset(tick_ms);
        // Step 4: chord barrier も clear (旧 ctrl_bypass_hold 相当)
        self.platform_state.ime.clear_input_barrier();
        self.platform_state.gate.sync_key_gate.clear();

        // 6. IME 状態を再取得
        self.refresh_ime_state_cache();

        // 7. バルーン通知
        self.platform
            .tray
            .show_balloon("awase", "状態をリセットしました");
    }
}

/// 全修飾キーの KeyUp を `SendInput` で送信する。
///
/// Shift, Ctrl, Alt, Win の左右それぞれに対して KeyUp を送り、
/// スタックした修飾キー状態を解消する。
fn send_all_modifier_key_ups() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };

    // VK_SHIFT(0x10), VK_CONTROL(0x11), VK_MENU(0x12),
    // VK_LWIN(0x5B), VK_RWIN(0x5C),
    // VK_LSHIFT(0xA0), VK_RSHIFT(0xA1),
    // VK_LCONTROL(0xA2), VK_RCONTROL(0xA3),
    // VK_LMENU(0xA4), VK_RMENU(0xA5)
    use crate::vk::{
        VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU,
        VK_RSHIFT, VK_RWIN, VK_SHIFT,
    };
    const MODIFIER_VKS: [VkCode; 11] = [
        VK_SHIFT,
        VK_CONTROL,
        VK_MENU,
        VK_LWIN,
        VK_RWIN,
        VK_LSHIFT,
        VK_RSHIFT,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_LMENU,
        VK_RMENU,
    ];

    let inputs: Vec<INPUT> = MODIFIER_VKS
        .iter()
        .map(|&vk| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk.0),
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: crate::output::INJECTED_MARKER,
                },
            },
        })
        .collect();

    // OutputActiveGuard: SendInput 実行中にユーザーキーが届いた場合、
    // フックが RUNTIME 借用中（panic_reset の with_app 内）で再入しないよう
    // OUTPUT_GATE.active=true で INPUT_DEFER に退避する。
    let _guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
    let _ = crate::win32::send_input_safe(&inputs);
    log::debug!("Sent KeyUp for all modifier keys");
}

/// IME の未確定文字列をキャンセルする。
///
/// # Safety
/// Win32 IMM API (`ImmGetContext`, `ImmNotifyIME`, `ImmReleaseContext`) を呼び出す。
/// メインスレッドから呼ぶこと。
unsafe fn cancel_ime_composition() {
    use std::mem::size_of;
    use windows::Win32::UI::Input::Ime::{ImmNotifyIME, NOTIFY_IME_ACTION, NOTIFY_IME_INDEX};
    use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO};

    // `GetForegroundWindow()` は外側の CASCADIA_HOSTING_WINDOW_CLASS を返すが、
    // WezTerm などでは実際の IME コンテキストは子ウィンドウ
    // (Windows.UI.Input.InputSite.WindowClass) に紐付いている。
    // `GetGUIThreadInfo(0)` でフォアグラウンドスレッドの hwndFocus を取得することで
    // InputSite HWND を得る。
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `GetGUIThreadInfo` はメインスレッドから呼ぶ安全なクエリ。
    //         tid=0 はフォアグラウンドスレッドを意味する。
    if unsafe { GetGUIThreadInfo(0, &raw mut info) }.is_err() {
        return;
    }
    let hwnd = info.hwndFocus;
    if hwnd.0.is_null() {
        return;
    }
    // SAFETY: `hwnd` は直上で NULL でないことを確認済み。
    //         `ImmContextGuard` は RAII で `ImmReleaseContext` を呼ぶため、
    //         コンテキストリークは発生しない。
    let Some(ctx) = (unsafe { crate::imm::ImmContextGuard::new(hwnd) }) else {
        log::debug!("[ctrl-bypass] ImmGetContext returned NULL for hwnd={hwnd:?}, cancel skipped");
        return;
    };
    // NI_COMPOSITIONSTR = 0x15, CPS_CANCEL = 0x04
    // SAFETY: `ctx.himc()` は `ImmContextGuard` が保持する有効な HIMC。
    //         `NI_COMPOSITIONSTR`/`CPS_CANCEL` は未確定文字列キャンセルの標準的な呼び出し。
    let ok = unsafe {
        ImmNotifyIME(
            ctx.himc(),
            NOTIFY_IME_ACTION(0x15),
            NOTIFY_IME_INDEX(0x04),
            0,
        )
    };
    log::debug!(
        "[ctrl-bypass] ImmNotifyIME(CPS_CANCEL) hwnd={hwnd:?} → {}",
        ok.as_bool()
    );
}
