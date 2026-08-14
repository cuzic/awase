mod conv_actuation;
pub(crate) mod executor;
mod focus_tracker;
mod focus_tracking;
mod ime_actuation;
mod ime_coordinator;
mod ime_refresh;
mod key_pipeline;
// ADR-089 §2.3 Phase B: ImmCross を機構チェーンの要素として実行する非同期経路。
pub(crate) mod message_handlers;
pub(crate) mod open_chain;
pub(crate) mod outbox;
mod transport;

pub(crate) use transport::{PassthroughQueue, PhysicalKeyDisposition};

use crate::focus::FocusKind;
use awase::config::ValidatedConfig;
use awase::engine::{
    Engine, EngineCommand, InputContext, InputModeState, SpecialKeyCombos, ThumbKeySoloTapGuard,
};
use awase::ngram::NgramModel;
use awase::types::{ContextChange, RawKeyEvent, VkCode};

use crate::focus::cache::DetectionSource;
use crate::focus::classifier::InjectionHint;
use crate::platform::WindowsPlatform;
use crate::runtime::executor::ImeApplyPair;
use crate::vk::VkCodeExt as _;
use awase::platform::PlatformRuntime as _;

/// `GeneralConfig::muhenkan_solo_tap_dedicated_fn_key`（ADR-091 §D3.2）を
/// `VkCode` に解決する。`bootstrap.rs`（起動時）と `apply_config_update`
/// （reload 時）の両方から呼ぶ。
///
/// `Some(name)` なのに `VkCode::from_name` が解決できない場合（誤字・
/// `"F21"` のような短縮形など）は、専用 Fn キー変換が黙って無効化される
/// （＝設定前と同じ挙動に留まる、安全側）が、原因が分かるよう警告ログを出す。
pub(crate) fn resolve_dedicated_fn_key(name: Option<&str>) -> Option<VkCode> {
    let name = name?;
    let resolved = VkCode::from_name(name);
    if resolved.is_none() {
        log::warn!(
            "[config] muhenkan_solo_tap_dedicated_fn_key = {name:?} を VK 名として \
             解決できませんでした。専用 Fn キー変換は無効のままです \
             （\"VK_F18\" のような完全な VK 名が必要、\"F18\" 等の短縮形は不可）"
        );
    }
    resolved
}

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

impl LayoutEntry {
    /// `default_layout`（`config.general.default_layout`、`.yab` 拡張子付き）に
    /// 一致するレイアウトのインデックスを返す。一致するものが無ければ `0` に
    /// フォールバックする（`layouts` が空の場合は呼び出し元の責任で扱うこと）。
    ///
    /// 識別は `name`（ファイル名、拡張子抜き）で行う。`.yab` 内部の名前行は
    /// 自由記述でありファイル名と一致する保証が無いため比較に使わないこと
    /// （2026-07-29 実機バグ: かつて内部名前行で比較しており、default_layout が
    /// 一致する内部名を持つファイルが存在しない場合、常に先頭レイアウトへ
    /// 無言でフォールバックしていた）。起動時（`bootstrap::select_default_layout`）・
    /// 設定リロード時（`Runtime::reload_layouts`）の両方から同じロジックを使う。
    #[must_use]
    pub fn resolve_index(layouts: &[Self], default_layout: &str) -> usize {
        let default_name = default_layout.trim_end_matches(".yab");
        layouts
            .iter()
            .position(|e| e.name == default_name)
            .unwrap_or(0)
    }
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
    /// 進行中の IME actuation 試行（ADR-080）。`desired` 変化・`FocusChanged`・
    /// `Resolution` 確定でのみ破棄・再構築する（`runtime/ime_actuation.rs`）。
    active_actuation: Option<ime_actuation::Actuation>,
    /// open/close 軸の force-write 武装フラグ（ADR-086 §4 INV-15、Phase 3 item 1）。
    ///
    /// `Some(gen)` = 武装済み、`gen` は武装した時点の `Output::ime_mode_focus_gen`。
    /// `None` = 未武装。conv 軸の `Output::force_pending`（Phase 2）とは**統合しない**
    /// ——層（`Output::send_romaji` は `&self`／`with_app` の内側、open 軸の消費は
    /// `&mut Runtime` を要する `on_ime_apply_complete` を必ず経由するため
    /// `kp_run_inner` からしか呼べない）・消費タイミング（open は「キーが届いた瞬間」
    /// まで前倒しする必要がある）・再武装セマンティクス（open の apply は完全同期で
    /// `Aborted` 概念が無く、代わりに `ImeOpenOutcome::UnsafeToToggle` を使う）が
    /// いずれも conv 軸と異なるため（`docs/adr/086-force-write-trigger-and-target-identity.md`
    /// §5 Phase 3 item 1 参照）。
    ///
    /// 武装点は `ir_post_focus_change_snapshot`（`gji_on_focus_change` 直後
    /// ——`ime_mode_focus_gen` が今回のフォーカス変更分だけ進んだ直後の単一
    /// 集約点。`ir_notify_focus_changed` ではない——同関数の実行時点では
    /// gen がまだ古いため）。**生の `FocusChange` イベントハンドラ
    /// （`platform.rs::gji_on_focus_change` 自体の本体）に書いてはいけない**
    /// ——`architecture_guard::force_write_is_not_triggered_by_raw_focus_change`
    /// の走査対象。
    ///
    /// 旧 `last_force_on_resend_ms`（`apply_force_on_for_imm_broken` の
    /// force-policy 経路が使っていた周期レート制限）は本フィールドへの移行に
    /// 伴い撤去済み（2026-08-08、ADR-086 Phase 3 item 1）。
    ///
    /// **訂正（2026-08-08 2回目 opus アドバーサリアルレビュー M4）**: タプルの
    /// 第2要素は `Failed`（Win32 呼び出し自体の失敗）の再武装試行回数
    /// （armed_gen ごとにリセット、上限は `FORCE_OPEN_FAILED_RETRY_LIMIT`）。
    /// 周期フォールバックを撤去した以上、`Failed` を再武装しないと次の
    /// FocusChange まで永久に迂回できなくなるが、無制限に再武装すると
    /// `Failed` が恒久的に返る環境で打鍵のたびに同期 IMC write を伴う
    /// 再試行が延々と続く（`ImeOpenOutcome::UnsafeToToggle` は Win キー
    /// 解放という外部条件で必ず終わるため上限不要）。
    force_open_pending: Option<(u32, u8)>,
    /// force-ON の実送信レート制限（M3 対応、2026-08-08）。最後に
    /// `force_on_and_correct_romaji` を実際に呼んだ tick（ms）。
    ///
    /// フォーカスチャーン環境（Chrome 連続フォーカスイベント=BUG-37、UWP
    /// 2段フォーカス、通知フォーカスチャーン=BUG-57）下で高速タイピングすると
    /// 「毎打鍵で再武装→毎打鍵で発火」＝20〜50ms 間隔になりうる（§1.2 欠陥4
    /// が実機記録した `9c102b02` の連打問題と同じレート、周期版より悪化）。
    /// `ime_poll_interval_ms`（既定500ms、撤去した `last_force_on_resend_ms`
    /// が与えていた下限と同一値）を実送信の下限間隔として使う。新規タイミング
    /// 定数は導入しない（`.claude/rules/tuning-constants.md` 準拠）。
    last_force_open_ms: Option<u64>,
    /// BUG-52 の DBE レンジ Suppress（`VK_DBE_ALPHANUMERIC`/`KATAKANA`/
    /// `SBCSCHAR`/`DBCSCHAR`）を無条件のままにするか、パススルーを許すか。
    /// `config.general.dbe_mode_key_policy` から `apply_config_update`/起動時の
    /// `set_dbe_mode_key_policy` で反映される（ADR-091 §D3.6、既定は `Suppress`
    /// で現状維持）。`PhysicalKeyDisposition::plan` が参照する。
    dbe_mode_key_policy: awase::config::DbeModeKeyPolicy,
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
    #[allow(unsafe_code)] // read_os_modifiers() が Win32 GetKeyState を呼ぶ
    pub(crate) fn build_ctx(&self) -> InputContext {
        // SAFETY: `read_os_modifiers` は Win32 `GetKeyState` を呼ぶのみで副作用はない。
        //         メインスレッドから呼ばれるため、スレッド要件を満たしている。
        let mut modifiers = unsafe { crate::observer::focus_observer::read_os_modifiers() };
        // Alt なりすまし中は本物の Alt 押下を無視する（hook.rs の
        // `is_alt_impersonation_active` doc 参照。ここを直さないと、hook.rs 側の
        // RawKeyEvent.modifier_snapshot は正しく補正されていても、
        // bypass_reason() が実際に見る PhysicalKeyState.modifiers はこの
        // build_ctx() の戻り値から来る（別経路）ため、なりすましたキーが
        // 常に OsModifierHeld でバイパスされてしまう）。
        if crate::hook::is_alt_impersonation_active() {
            modifiers.alt = false;
        }
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

    /// 現在のフォーカスエポック（`probe_admission::ImmLikeTicket::admit` の照合用）。
    #[must_use]
    pub(crate) fn focus_epoch(&self) -> crate::state::probe_admission::FocusEpoch {
        self.platform_state.focus.focus_epoch
    }

    // ── 実 actuation の起案（ADR-090 §2.A A-1、INV-47）────────────────────

    /// 実 actuation 入口が 1 件の指示を起案する（shadow モード）。
    ///
    /// `strategy` は「どの入口が起案したか」を表す識別子で、A-1 の shadow ログ
    /// （`[warrant-shadow]`）と journal から入口を区別するために使う。
    /// **入口ごとに一意な文字列にすること**——A-2 は「`would_have_blocked` が
    /// ゼロだった入口から順に強制へ倒す」ので、入口が識別できないと分割できない
    /// （ADR-090 §6 ステップ 7）。
    ///
    /// 時刻は `state/` 層へ注入する規約に従い、ここ（`runtime/`）で取得する。
    fn issue_actuation_order(
        &self,
        open: bool,
        strategy: &'static str,
    ) -> crate::state::actuation_chain::ActuationOrder {
        let origin = crate::state::event_origin::EventOrigin::new(
            crate::state::event_origin::EventSource::SelfActuated { strategy },
            crate::state::event_origin::Generation::INITIAL,
        );
        self.issue_actuation_order_with_origin(open, origin)
    }

    /// 呼び出し元が既に `EventOrigin` を持っている場合（drift correction 等）。
    fn issue_actuation_order_with_origin(
        &self,
        open: bool,
        origin: crate::state::event_origin::EventOrigin,
    ) -> crate::state::actuation_chain::ActuationOrder {
        let now = std::time::Instant::now();
        let now_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        self.platform_state
            .ime
            .issue_actuation_order(open, origin, now, now_ms)
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
            self.schedule_settle_retry("SetOpen stripped from execute_from_loop decision");
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
    ///
    /// `reason`（ADR-086 §4 INV-18、Phase 3 item 2）は「なぜこの apply が
    /// 起きたか」を申告する必須引数。`Option` にしない——デフォルトを許すと
    /// provenance が欠落する呼び出しが紛れ込む。`record_ime_apply_result` は
    /// `generation.is_some()` のときだけ `ImeEvent` を dispatch するため
    /// （force 系の適用は generation を持たずこの経路を通らない）、`reason` は
    /// ジャーナルへ直接記録することで force 系も含めた全経路の provenance を
    /// 一意に残す。
    pub fn on_ime_apply_complete(
        &mut self,
        open: bool,
        outcome: awase::platform::ImeOpenOutcome,
        generation: Option<u64>,
        reason: crate::state::ime_event::OpenApplyReason,
    ) {
        use awase::platform::{ImeOpenOutcome, TsfComposition as _};

        self.platform_state
            .ime
            .journal
            .record(crate::journal::JournalEntry::ImeOpenApplied {
                open,
                outcome,
                reason,
            });

        // E: UnsafeToToggle でも必ずスケジュールする。UnsafeToToggle は
        // Win-held 等の genuine skip に加え、ADR-086 の `ActuationOutcome::Aborted`
        // （フォーカス移動/世代不一致で書き込みを中止）や capture 失敗も含む。
        // 特に `Aborted(GenStale)`（同一ウィンドウのままフォーカス世代だけ進んだ
        // ケース）は意図（IME open/close）自体は依然有効なのに、それを再試行する
        // 自然なトリガー（新しいフォーカス変更）が発生しない。以前はここで早期
        // return しており、次に無関係なイベントが来るまで無期限に取りこぼされ
        // 得た（opus レビュー指摘 F3、2026-08-08 是正、`ime::
        // set_ime_open_then_conv_for_target` の doc 参照）。
        self.platform.post_ime_refresh();

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
    }

    /// sync path の outcome リストを一括 dispatch する。
    pub(crate) fn dispatch_outcomes(&mut self, outcomes: Vec<ImeApplyPair>) {
        for completion in outcomes {
            self.on_ime_apply_complete(
                completion.open,
                completion.outcome,
                completion.generation,
                completion.reason,
            );
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

    /// settle 期間中に IME apply/decision をスキップしたとき、settle 明けに refresh で
    /// 一度だけ再試行する「確立済みパターン」（`executor::strip_ime_set_open_if_settling`
    /// doc 参照）を一元化する。
    ///
    /// 遅延は settle 残余の上限（= `focus_settle_ms()`）+ タイマー粒度マージン 50ms。
    /// `reason` はログの `[focus-settle] {reason} → ...` に埋め込まれる、呼び出し元ごとの
    /// 説明文（例: `"apply_force_on_for_imm_broken skipped (settling)"`）。
    pub fn schedule_settle_retry(&mut self, reason: &str) {
        let retry_ms = self.platform_state.ime.focus_settle_ms() + 50;
        log::debug!("[focus-settle] {reason} → {retry_ms}ms 後に refresh で再試行");
        self.schedule_ime_refresh(retry_ms);
    }

    /// `ImeEvent::InputModeApplied`（`result` は常に `Applied`）の dispatch を一元化する。
    ///
    /// awase 自身の能動的な input_mode 訂正（`InputModeApplyStrategy` 参照）は、常に
    /// `result: Applied` 固定・5 フィールドの構築が `mode`/`strategy`/`tick_ms` だけ
    /// 違う形で複数箇所に複製されていた。`Skipped` を構築する経路は `state/ime_model.rs`
    /// 内の別経路専用でありここでは扱わない。
    pub fn apply_input_mode_correction(
        &mut self,
        mode: InputModeState,
        strategy: crate::state::ime_event::InputModeApplyStrategy,
        tick_ms: crate::state::TickMs,
    ) {
        self.platform_state.ime.dispatch_event(
            crate::state::ime_event::ImeEvent::InputModeApplied {
                mode,
                strategy,
                result: crate::state::ime_event::InputModeApplyResult::Applied,
                at: tick_ms,
            },
            tick_ms,
        );
    }

    /// ポーリング間隔設定に従って次回 IME リフレッシュをスケジュールする。
    pub fn reschedule_ime_refresh(&mut self) {
        // TsfNative は read_ime_state_full が常に None、GJI も predates-focus-change でスキップ。
        // explicit_intent の有無に関わらずポーリングで得られる情報がないため常に停止する。
        // explicit_intent が確定している他プロファイルも同様に停止。
        // 再開トリガー: フォーカス変更 / may_change_ime キー（20ms タイマー）/
        // `kp_apply_conv_engine_sync` の ReportOpenInference（BUG-51、20ms）。
        //
        // 2026-08-06: `conv_mode_policy = force` のときこの早期 return をスキップする
        // 例外が入った（`apply_force_on_for_imm_broken` の周期 force-ON 再送を同じ
        // リフレッシュ連鎖に相乗りさせるため）。2026-08-08 ADR-086 Phase 3 実装時、
        // force-ON のトリガーを周期からキー入力の直前へ移したため
        // （`kp_run_inner::consume_force_open_pending`）「この連鎖に依存しなくなった」
        // と判断し例外を一度撤去したが、Phase 3 実装完了後の2回目 opus アドバーサリアル
        // レビュー（M5）で、この撤去が `ir_apply_drift_correction`（BUG-20 が追加した
        // non-ImmCross/TsfNative 向け分岐）の周期実行機会も巻き添えで奪っていたと
        // 判明し、例外を復元した。
        //
        // **この復元は「force-ON 用に戻した」わけではない**（force-ON は
        // `apply_force_on_for_imm_broken` が `is_force_policy()` で即 return する
        // ため、この連鎖が復活しても force-ON の周期スパムは再発しない——ただし
        // この安全性は「別関数の早期 return に依存する暗黙の前提」であり、
        // `architecture_guard::is_force_policy_call_sites_are_accounted_for` で
        // `is_force_policy()` の呼び出し箇所数を固定して守っている）。
        // 復元の実体は「Phase 3 が force-ON の周期経路を撤去した際に巻き添えで
        // 落ちた `ir_apply_drift_correction` の周期実行機会の最小復元」であり、
        // **observe policy の TsfNative ユーザーは元々この周期を持っていない**
        // （この `is_tsf_native` 早期 return 自体がポリシー非依存のため）。
        // つまり本復元は「force policy ユーザーだけが周期 drift correction を
        // 持つ」という新たな非対称を意図せず生む。本来はポリシー非依存に
        // 判断すべき論点であり、ADR-086 §7-12 に未解決論点として起票してある
        // （実機ソークで TsfNative × observe 環境の drift 未検出が問題になる
        // ようなら、この例外条件を `is_effectively_tsf_native()` へ広げる
        // ことを検討する）。
        //
        // 注記: この訂正は実機で観測した失敗ではなく、コード読解で判明した
        // 巻き添え（`.claude/rules/experiment-logging.md` が求める実測とは
        // 性質が異なる）。`docs/experiments.md` にもその旨を明記して残す。
        let force_policy = self.platform.output.is_force_policy();
        if !force_policy {
            let is_tsf_native = crate::focus::class_names::is_effectively_tsf_native(
                self.platform.current_app_profile(),
                self.platform.focus.class_name(),
            );
            if is_tsf_native || self.platform_state.ime.explicit_intent().is_some() {
                return;
            }
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
    /// `ir_post_focus_change_snapshot` 内の GJI 強制 ON / IME OFF 強制ブロック,
    /// `consume_force_open_pending`〈ADR-086 Phase 3〉）は
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
    ///
    /// `conv_mode_policy = force` のときは何もしない（ADR-086 Phase 3、2026-08-08）。
    /// force policy 時の force-ON は `kp_run_inner::consume_force_open_pending`
    /// （キー入力直前、入力意図に紐づくトリガー）に移行済み。この関数は
    /// `ir_stage_notify` の周期リフレッシュに相乗りする経路であり、INV-15 が
    /// 禁止する「生の周期タイマー」トリガーに該当するため、force policy を
    /// 使うぶんはもうここを通さない（`reschedule_ime_refresh` も同時にこの関数の
    /// ための force policy 例外を撤去済み）。
    pub fn apply_force_on_for_imm_broken(&mut self) {
        if self.can_use_imm32_cross_process() {
            return;
        }
        if self.platform.output.is_force_policy() {
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
            self.schedule_settle_retry("apply_force_on_for_imm_broken skipped (settling)");
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
        self.force_on_and_correct_romaji(
            crate::state::ime_event::OpenApplyReason::ImmBrokenForceOn,
        );
    }

    /// force-ON を実際に送信し、続けて非ローマ字対応 `input_mode` の補正を行う共通処理。
    ///
    /// `apply_force_on_for_imm_broken`（`conv_mode_policy = observe` 経路）と
    /// `consume_force_open_pending`（ADR-086 Phase 3、`conv_mode_policy = force` 経路）
    /// が共有する。`reason` 以外の主要な挙動は同一だが、呼び出し元が通す
    /// ガード（settle・`AppliedImeState` スロットル等）は異なる——詳細は
    /// 各呼び出し元の doc を参照。
    fn force_on_and_correct_romaji(
        &mut self,
        reason: crate::state::ime_event::OpenApplyReason,
    ) -> awase::platform::ImeOpenOutcome {
        let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
        // N1（2026-08-08 2回目 opus アドバーサリアルレビュー新規指摘）:
        // force-ON の同期 IMC write（`MsImeDirectStrategy`/`ImmCrossProcessStrategy`
        // 内の `set_ime_romaji_mode()`）を、`kp_stage_idle_conv_check` の汚染
        // 再検証ガード（shift ガード・`last_explicit_ime_action_ms` 一致・
        // `last_send` 一致）から見える形にする。呼ばないと、Phase 3 で
        // idle_conv_check の隣（同一キーイベント内）に移動した force-ON 自身の
        // 書き込みが「外部観測」として idle-conv-check に誤読される
        // （`platform_state.rs` の `note_explicit_ime_action` doc 参照）。
        self.platform_state.ime.note_explicit_ime_action(tick_ms);
        // N2（2026-08-08 2回目 opus アドバーサリアルレビュー新規指摘）:
        // `apply_ime_open_with_belief(true, None, belief)` は内部で
        // `belief_input_mode: InputModeState::Unknown` 固定の view を作るため、
        // `MsImeDirectStrategy`/`ImmCrossProcessStrategy` の「ユーザーが
        // 意図的にかな入力を選んでいれば romaji 復元で上書きしない」
        // （`ObservedKana` 保護）ガードが force-ON 経路では一度も効かなかった。
        // `belief_input_mode = input_mode()` を明示的に埋めた view を使うことで
        // 保護を効かせる。`applied` は `shadow_ime_control_view()` の
        // `Some(applied_pair())` ではなく `None` のまま維持する——GJI の
        // `shadow_on` スキップ（`GjiDirectStrategy` が「既に ON」と誤認して
        // VK_IME_ON をスキップする）を意図的に外す既存仕様のため
        // （`ir_post_focus_change_snapshot` の GJI TsfNative 強制 ON と同じ理由）。
        let mut view = self.platform.build_ime_control_view(None);
        view.belief_input_mode = self.platform_state.ime.input_mode();
        let belief = crate::output::OpenBelief {
            effective_open: true,
            confident: true,
        };
        // ADR-090 §2.A A-1（shadow）: 実 actuation 入口は `ActuationOrder` を
        // 起案する。授権が下りなくても書き込みは止めない（A-2 で倒す）。
        let order = self.issue_actuation_order(true, "force_on_and_correct_romaji");
        let outcome = self.platform.apply_ime_open_with_view(order, &view, belief);
        log::info!("force-ON ({reason:?}): apply_ime_open(true) → {outcome:?}");
        self.on_ime_apply_complete(true, outcome, None, reason);
        if !self.platform_state.ime.input_mode().is_romaji_capable() {
            if let Some(new_mode) = self.platform_state.ime.correction_for_imm_broken() {
                log::info!(
                    "force-ON ({reason:?}): input_mode → AssumedRomaji (IMM broken, ime_on=true)"
                );
                let tick_ms = crate::state::TickMs(crate::hook::current_tick_ms());
                self.apply_input_mode_correction(
                    new_mode,
                    crate::state::ime_event::InputModeApplyStrategy::ImmBrokenCorrection,
                    tick_ms,
                );
            } else {
                // romaji-capable は外側の if で除外済みなので None = ObservedEisu のみ
                log::info!(
                    "force-ON ({reason:?}): input_mode スキップ (belief=ObservedEisu, eisu guard)"
                );
            }
        }
        outcome
    }

    /// `force_open_pending`（open/close 軸の force-write 武装フラグ）を
    /// 立てる、またはクリアする（ADR-086 Phase 3 item 1、INV-15）。
    ///
    /// `ir_post_focus_change_snapshot` の `gji_on_focus_change` 呼び出し直後
    /// から呼ぶこと（`ime_mode_focus_gen` が今回のフォーカス変更分だけ進んだ
    /// 直後——武装以外の処理を一切含まない、この専用関数へ抽出したのは
    /// `architecture_guard::force_write_is_not_triggered_by_raw_focus_change`
    /// が「武装のみ許可」を機械的に固定できるようにするため（2026-08-08、
    /// 2回目 opus アドバーサリアルレビュー M2。当初は
    /// `ir_post_focus_change_snapshot` 全体を走査対象にする案だったが、
    /// 同関数は GJI TsfNative VK_IME_ON 強制等の正当な既存書き込みも含むため
    /// ホワイトリスト例外が必要になり、将来別のラッパー経由で force-write が
    /// 紛れ込んでもガードをすり抜けてしまう。本関数は代入以外何もしないため
    /// 素朴な禁止リストで確実に検知できる）。
    ///
    /// `is_force_policy()` でも ImmCross 対応アプリ（force-ON の対象外、
    /// `apply_force_on_for_imm_broken` と同じスコープ判断）では武装しない
    /// （`.then()` により対象外なら明示的に `None` へクリアする）。
    /// 試行回数（タプル第2要素）は新規武装のたびに `0` から始まる。
    pub(crate) fn arm_force_open_pending(&mut self) {
        self.force_open_pending = (self.platform.output.is_force_policy()
            && !self.can_use_imm32_cross_process())
        .then(|| (self.platform.output.ime_mode_focus_gen.get(), 0u8));
    }

    /// `force_open_pending` を消費し、武装済みなら force-ON を起こす
    /// （ADR-086 Phase 3 item 1、INV-15）。
    ///
    /// 呼び出し元は `kp_run_inner`（送信要求という入力意図に紐づく唯一の消費点、
    /// `key_pipeline.rs::is_force_open_consumption_candidate` を満たすキーの
    /// ときだけ呼ばれる）。`try_hold_key`/ime-off-rescue の早期 return より後・
    /// `kp_stage_focus_probe`/`kp_stage_idle_conv_check`/
    /// `kp_stage_shadow_ime_toggle` より後・`build_input_context` より前に
    /// 置く——これより前で消費すると、hold されたキーで武装だけ消費され実際の
    /// 打鍵は再処理時になる（force だけ先に飛んで打鍵が来ない）。
    ///
    /// **`ime_apply_should_defer()`（settle ガード）は呼ばない**（訂正、
    /// 2026-08-08 2回目 opus アドバーサリアルレビュー H1）。呼び出し元の
    /// `is_force_open_consumption_candidate` が「本物の入力意図」を直接判定
    /// することで settle ガードの役割（Alt+Tab 中間ウィンドウへの誤射防止）を
    /// 代替している——本関数がここでさらに settle ガードを呼ぶと、消費点の
    /// 移動先では barrier が既に消費済みのため構造的に常に defer 判定になり、
    /// フォーカス変更後 1 打鍵目を必ず取りこぼす退行を生む（詳細は
    /// `key_pipeline.rs::is_force_open_consumption_candidate` の doc 参照）。
    ///
    /// 未消費のまま return する分岐（非対象状態）は**武装を維持する**——次の
    /// キーイベントで再試行できるようにするため（`apply_force_on_for_imm_broken`
    /// の対応する早期 return とは異なり、こちらは「次の周期リフレッシュ」では
    /// なく「次のキー入力」が再試行のトリガーになる）。
    ///
    /// **既存の `AppliedImeState` スロットル**（`apply_force_on_for_imm_broken`
    /// の非 force 分岐が使う「`Optimistic(true)|Confirmed{open:true}` なら
    /// 送らない」チェック）は**ここでは意図的に読まない**。force の趣旨は
    /// 「applied が誤って ON にラッチされた状態を破ること」であり、このスロットル
    /// を読むと趣旨と矛盾する。後日「重複ガードだ」として誤って足さないこと
    /// （ADR-086 §5 Phase 3 item 1 参照）。
    pub(crate) fn consume_force_open_pending(&mut self) {
        // Failed の再武装に許す最大試行回数（armed_gen ごとにリセット）。
        // UnsafeToToggle は Win キー解放という外部条件で必ず終わるため対象外
        // （M4、無制限に再武装してよい）。値の根拠は ADR-080 の
        // `FeedbackPolicy::Blind{max_attempts}` と同型の「有限リトライで
        // 折り合いをつける」という設計判断であり、実測値ではない
        // （`.claude/rules/tuning-constants.md` はタイミング値の実測義務を
        // 課すもので、この試行回数カウンタには適用されない）。
        const FORCE_OPEN_FAILED_RETRY_LIMIT: u8 = 2;

        let armed = self.force_open_pending;
        let eligible =
            self.engine.is_user_enabled() && self.platform_state.ime.is_eligible_for_ime_force_on();
        let now = crate::hook::current_tick_ms();
        let ms_since_last = self.last_force_open_ms.map(|last| now.saturating_sub(last));
        let interval_ms = u64::from(self.platform_state.focus.ime_poll_interval_ms);
        // M3: 実送信のレート制限。フォーカスチャーン環境で毎打鍵ごとに
        // 再武装→消費が起きても、実際の apply は ime_poll_interval_ms
        // 間隔まで間引く。掛かった場合は武装を維持し、次のキーイベントで
        // 再試行する（破棄すると BUG-16 型のリテラル化取りこぼしに直結する）。
        if !should_attempt_force_open(armed.is_some(), eligible, ms_since_last, interval_ms) {
            return;
        }
        let (armed_gen, attempts) =
            armed.expect("should_attempt_force_open が true の時点で armed は Some");
        // ここから実際に消費する（以降の早期 return は武装を戻さない限り再武装しない）。
        self.force_open_pending = None;
        // 時間軸フェンス: 消費直前（上の各種チェック）から apply 直前までの間に
        // 別の正規の FocusChange が武装し直していないか確認する。本経路は完全に
        // 同期的（await を挟まない）なため、実際にはこの2点の gen は常に一致するが、
        // 将来この経路に非同期処理が挟まれた場合の回帰を防ぐため明示的に確認する。
        if self.platform.output.ime_mode_focus_gen.get() != armed_gen {
            log::debug!("[force-open-pending] gen 不一致 (armed={armed_gen}) → 別の武装に委ねる");
            return;
        }
        let outcome = self.force_on_and_correct_romaji(
            crate::state::ime_event::OpenApplyReason::ForcePolicyResend,
        );
        // AlreadyMatched（未送信）ではレート制限のスタンプを更新しない。
        if outcome != awase::platform::ImeOpenOutcome::AlreadyMatched {
            self.last_force_open_ms = Some(now);
        }
        // ADR-086 Phase 2 の consume_force_pending_and_actuate と同じ大枠の
        // スコープ判断（UnsafeToToggle のみ再武装）に加え、M4（2回目 opus
        // アドバーサリアルレビュー）を受けて Failed も試行回数上限付きで
        // 再武装する——周期フォールバックを撤去した以上、Failed を一切
        // 再武装しないと次の FocusChange まで永久に迂回できなくなるため。
        self.force_open_pending = next_force_open_pending_after_outcome(
            armed_gen,
            attempts,
            outcome,
            FORCE_OPEN_FAILED_RETRY_LIMIT,
        );
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
                self.schedule_settle_retry("try_force_on_bootstrap skipped (settling)");
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
            // ADR-090 §2.A A-1（shadow）。**この入口は差分オラクルが
            // 「判明した中で最大の挙動変化」と記録している old-1 そのもの**
            // （`ImmCross` は `default_feedback = Read` なので Step 4c が
            // 発火せず、観測も意図も guard も無い bootstrap では warrant が
            // `None` になる）。A-2 で倒すのは**最後**に回すこと（ADR-090 §4.9）。
            let order = self.issue_actuation_order(true, "try_force_on_bootstrap");
            let outcome = self
                .platform
                .apply_ime_open_with_belief(order, None, belief);
            log::info!("force-on bootstrap: apply_ime_open(true) → {outcome:?}");
            self.on_ime_apply_complete(
                true,
                outcome,
                None,
                crate::state::ime_event::OpenApplyReason::Bootstrap,
            );
            self.platform_state.ime.set_force_on_broken_app_bootstrap();
        }
    }

    /// 設定リロード時にレイアウト一覧を再スキャンし、`default_layout` に追従させる。
    ///
    /// 設定画面の「適用」（再起動なしの即時反映）でレイアウト切り替えが効かない、
    /// という報告（2026-07-29）に対応するもの。それまで `reload_config` は
    /// スレッショルド・キー設定等は再読込していたが、レイアウトだけは対象外で、
    /// 再起動しない限り反映されなかった。
    ///
    /// レイアウトが実質変わっていない場合は `switch_layout` を呼ばない。
    /// `EngineCommand::SwapLayout` は保留中のキーを flush する副作用があるため、
    /// 内容が変わっていないのに設定リロードのたびにタイピング中のキーを
    /// 確定させてしまうことを避ける。
    pub(crate) fn reload_layouts(&mut self, layouts: Vec<LayoutEntry>, default_layout: &str) {
        if layouts.is_empty() {
            log::warn!("reload_layouts: no layouts found, keeping current layout");
            return;
        }

        let names: Vec<String> = layouts.iter().map(|e| e.name.clone()).collect();
        let index = LayoutEntry::resolve_index(&layouts, default_layout);
        let target_name = layouts[index].name.clone();
        let unchanged = self.platform.tray.current_layout_name() == target_name;

        self.layouts = layouts;
        self.platform.tray.set_layout_names(names);

        if unchanged {
            return;
        }

        self.switch_layout(index);
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
    #[allow(unsafe_code)] // poll_and_classify_ime() が Win32 IMM API を呼ぶ
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
            active_actuation: None,
            force_open_pending: None,
            last_force_open_ms: None,
            dbe_mode_key_policy: awase::config::DbeModeKeyPolicy::default(),
        }
    }

    /// `config.general.dbe_mode_key_policy` を反映する。起動時
    /// （`bootstrap.rs`、`conv_mode.set_policy` と同じ post-construction 経路）と
    /// `apply_config_update`（reload 時）の両方から呼ぶ。
    pub(crate) fn set_dbe_mode_key_policy(&mut self, policy: awase::config::DbeModeKeyPolicy) {
        self.dbe_mode_key_policy = policy;
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

                // BUG-37: この `EVENT_OBJECT_FOCUS` 経路（Ctrl+T 新規タブ等、同一プロセス内の
                // フォーカス移動を含む）は belief（desired_open/effective_open）を一切触らない。
                // 判定ロジックは `should_reprime_on_lightweight_focus_sync` のドキュメント参照
                // （唯一の訂正チャネルである物理 IME キー押下が shadow-toggle の no-op に
                // 握り潰される問題を、真のフォーカス変更と同じ再プライム機構で補う）。
                // cold mark 自体は次に実際に入力するまで何も送信しない遅延フラグなので、
                // Chrome の連続フォーカスイベントで何度呼ばれても実害はない
                // （詳細は docs/known-bugs.md BUG-37）。
                let profile = crate::focus::classify::AppImeProfile::from_class_name(&class_name);
                if crate::focus::class_names::should_reprime_on_lightweight_focus_sync(
                    profile,
                    &class_name,
                    self.platform_state.ime.effective_open(),
                ) {
                    log::debug!(
                        "[focus-sync] belief=ON かつ実状態を問い合わせられないプロファイル \
                         (profile={profile:?}) → 次の入力で再プライムするため cold mark"
                    );
                    self.platform.mark_composition_cold_focus_change();
                }
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
        self.set_dbe_mode_key_policy(config.general.dbe_mode_key_policy);
        crate::hook::set_swallow_alt_kana_mode_switch(
            config.general.swallow_alt_kana_input_method_switch,
        );
        self.platform
            .output
            .conv_mode
            .set_policy(config.general.conv_mode_policy);
        // INV-27（ADR-087 §4）: force⇔observe 切替は `force_open_pending`
        // （および将来の `OpenWarrant` 発行済みキュー）を無効化しなければ
        // ならない。放置すると、force→observe 切替直後に「force policy 時に
        // 武装された pending」が残ったまま `consume_force_open_pending` が
        // 発火し、observe 経路（drift correction 等）と二重に force-ON が
        // 走る窓ができる（§7 round2 M8）。次の正当なトリガー（FocusChange 等）
        // が `arm_force_open_pending` で新しいポリシーに基づき再武装する。
        self.force_open_pending = None;
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
            crate::hook::set_alt_impersonation_enabled(
                left_alt_impersonates,
                right_alt_impersonates,
            );
            let space_thumb_vk = [left, right]
                .into_iter()
                .find(|&vk| vk == crate::vk::VK_SPACE);
            self.engine.set_space_thumb_config(
                space_thumb_vk,
                config.general.space_thumb_ignore_composing_guard,
                config.general.space_thumb_shift_literal,
            );
            let muhenkan_vk = [left, right]
                .into_iter()
                .find(|&vk| vk == crate::vk::VK_NONCONVERT);
            let henkan_vk = [left, right]
                .into_iter()
                .find(|&vk| vk == crate::vk::VK_CONVERT);
            self.engine.set_thumb_key_solo_tap_config(
                muhenkan_vk,
                ThumbKeySoloTapGuard {
                    ignore_composing_guard: config.general.muhenkan_solo_tap_ignore_composing_guard,
                    always_suppress: config.general.muhenkan_solo_tap_always_suppress,
                },
                henkan_vk,
                ThumbKeySoloTapGuard {
                    ignore_composing_guard: config.general.henkan_solo_tap_ignore_composing_guard,
                    always_suppress: config.general.henkan_solo_tap_always_suppress,
                },
            );
            self.engine
                .set_muhenkan_solo_tap_dedicated_fn_key(resolve_dedicated_fn_key(
                    config.general.muhenkan_solo_tap_dedicated_fn_key.as_deref(),
                ));
            let enter_thumb_vk = [left, right]
                .into_iter()
                .find(|&vk| vk == crate::vk::VK_RETURN);
            self.engine.set_enter_thumb_config(
                enter_thumb_vk,
                config.general.enter_thumb_ignore_composing_guard,
                config.general.enter_thumb_shift_literal,
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
    #[allow(unsafe_code)] // cancel_ime_composition() が Win32 IMM API を呼ぶ
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
#[allow(unsafe_code)]
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

// ── ADR-086 Phase 3: force_open_pending 消費判定（純粋関数、L4 対応）──
//
// `consume_force_open_pending` の判定ロジックのうち、Win32/`Runtime` に
// 依存しない部分をここへ切り出す。`Runtime` を構築せずに単体テストできる
// （`.claude/rules/fix-requires-evidence.md` (a)、Phase 2 が `Output` 側に
// 持つ同種のテストと対称）。

/// `consume_force_open_pending` が実際に `force_on_and_correct_romaji` を
/// 呼んでよいかを判定する。`armed`/`eligible` が false、またはレート制限に
/// 掛かっている（`ms_since_last_force_open` が `poll_interval_ms` 未満）
/// 場合は false——このとき呼び出し元は武装を維持すること。
fn should_attempt_force_open(
    armed: bool,
    eligible: bool,
    ms_since_last_force_open: Option<u64>,
    poll_interval_ms: u64,
) -> bool {
    if !armed || !eligible {
        return false;
    }
    ms_since_last_force_open.is_none_or(|elapsed| elapsed >= poll_interval_ms)
}

/// `force_on_and_correct_romaji` の outcome を受けて、次の `force_open_pending`
/// 状態を決める。`UnsafeToToggle` は無条件に再武装、`Failed` は試行回数
/// （`failed_retry_limit` 未満のときのみ）付きで再武装、それ以外
/// （`Applied`/`FallbackSent`/`AlreadyMatched`）は消費済みのままクリアする。
fn next_force_open_pending_after_outcome(
    armed_gen: u32,
    attempts: u8,
    outcome: awase::platform::ImeOpenOutcome,
    failed_retry_limit: u8,
) -> Option<(u32, u8)> {
    use awase::platform::ImeOpenOutcome;
    match outcome {
        ImeOpenOutcome::UnsafeToToggle => Some((armed_gen, attempts)),
        ImeOpenOutcome::Failed if attempts < failed_retry_limit => Some((armed_gen, attempts + 1)),
        _ => None,
    }
}

#[cfg(test)]
mod layout_entry_tests {
    use super::LayoutEntry;
    use awase::scanmap::KeyboardModel;
    use awase::yab::YabLayout;

    fn entry(name: &str) -> LayoutEntry {
        LayoutEntry {
            name: name.to_string(),
            layout: YabLayout::parse("", KeyboardModel::Jis).unwrap(),
        }
    }

    #[test]
    fn resolve_index_matches_by_file_name_with_or_without_yab_suffix() {
        let layouts = [entry("nicola"), entry("my_nicola")];
        assert_eq!(LayoutEntry::resolve_index(&layouts, "my_nicola.yab"), 1);
        assert_eq!(LayoutEntry::resolve_index(&layouts, "my_nicola"), 1);
        assert_eq!(LayoutEntry::resolve_index(&layouts, "nicola.yab"), 0);
    }

    #[test]
    fn resolve_index_falls_back_to_first_entry_when_no_name_matches() {
        let layouts = [entry("nicola"), entry("my_nicola")];
        assert_eq!(
            LayoutEntry::resolve_index(&layouts, "does_not_exist.yab"),
            0
        );
    }
}

/// ADR-086 Phase 3: `force_open_pending` 消費判定の単体テスト（L4 対応）。
#[cfg(test)]
mod force_open_pending_tests {
    use super::{next_force_open_pending_after_outcome, should_attempt_force_open};
    use awase::platform::ImeOpenOutcome;

    // ── should_attempt_force_open ──

    #[test]
    fn attempts_when_armed_eligible_and_never_sent_before() {
        assert!(should_attempt_force_open(true, true, None, 500));
    }

    #[test]
    fn skips_when_not_armed() {
        assert!(!should_attempt_force_open(false, true, None, 500));
    }

    #[test]
    fn skips_when_not_eligible() {
        assert!(!should_attempt_force_open(true, false, None, 500));
    }

    #[test]
    fn skips_when_rate_limited() {
        // 前回送信から 100ms しか経っていないのに poll_interval_ms=500。
        assert!(!should_attempt_force_open(true, true, Some(100), 500));
    }

    #[test]
    fn attempts_when_rate_limit_interval_elapsed() {
        assert!(should_attempt_force_open(true, true, Some(500), 500));
        assert!(should_attempt_force_open(true, true, Some(600), 500));
    }

    // ── next_force_open_pending_after_outcome ──

    #[test]
    fn unsafe_to_toggle_always_rearms_without_consuming_attempts() {
        assert_eq!(
            next_force_open_pending_after_outcome(7, 0, ImeOpenOutcome::UnsafeToToggle, 2),
            Some((7, 0))
        );
        // 試行回数上限に達していても UnsafeToToggle は無条件に再武装する。
        assert_eq!(
            next_force_open_pending_after_outcome(7, 2, ImeOpenOutcome::UnsafeToToggle, 2),
            Some((7, 2))
        );
    }

    #[test]
    fn failed_rearms_with_incremented_attempts_until_limit() {
        assert_eq!(
            next_force_open_pending_after_outcome(7, 0, ImeOpenOutcome::Failed, 2),
            Some((7, 1))
        );
        assert_eq!(
            next_force_open_pending_after_outcome(7, 1, ImeOpenOutcome::Failed, 2),
            Some((7, 2))
        );
    }

    #[test]
    fn failed_gives_up_once_retry_limit_reached() {
        assert_eq!(
            next_force_open_pending_after_outcome(7, 2, ImeOpenOutcome::Failed, 2),
            None
        );
    }

    #[test]
    fn applied_and_already_matched_clear_without_rearming() {
        assert_eq!(
            next_force_open_pending_after_outcome(7, 0, ImeOpenOutcome::Applied, 2),
            None
        );
        assert_eq!(
            next_force_open_pending_after_outcome(7, 0, ImeOpenOutcome::AlreadyMatched, 2),
            None
        );
        assert_eq!(
            next_force_open_pending_after_outcome(7, 0, ImeOpenOutcome::FallbackSent, 2),
            None
        );
    }
}
