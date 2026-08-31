mod conv_actuation;
#[cfg(windows)]
pub(crate) mod engine_window;
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
    Engine, EngineCommand, InputContext, InputModeState, ModeKeyConfig, SpecialKeyCombos,
    TextKeyConfig,
};
use awase::ngram::NgramModel;
use awase::types::Timestamp;
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
    left_thumb_down: Option<Timestamp>,
    right_thumb_down: Option<Timestamp>,
) -> InputContext {
    InputContext {
        ime_on,
        input_mode,
        is_japanese_ime,
        composing,
        modifiers: *modifiers,
        left_thumb_down,
        right_thumb_down,
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

pub(crate) struct NonEmptyLayouts(Vec<LayoutEntry>);

impl NonEmptyLayouts {
    pub(crate) fn new(layouts: Vec<LayoutEntry>) -> Option<Self> {
        (!layouts.is_empty()).then_some(Self(layouts))
    }

    pub(crate) fn names(&self) -> Vec<String> {
        self.0.iter().map(|e| e.name.clone()).collect()
    }

    pub(crate) fn into_vec(self) -> Vec<LayoutEntry> {
        self.0
    }

    pub(crate) fn as_slice(&self) -> &[LayoutEntry] {
        &self.0
    }
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
/// `platform_state.gate.post_bypass` latch をセットする。
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
    /// BUG-52 の DBE レンジ Suppress（`VK_DBE_ALPHANUMERIC`/`KATAKANA`/
    /// `SBCSCHAR`/`DBCSCHAR`）を無条件のままにするか、パススルーを許すか。
    /// `config.general.dbe_mode_key_policy` から `apply_config_update`/起動時の
    /// `set_dbe_mode_key_policy` で反映される（ADR-091 §D3.6、既定は `Suppress`
    /// で現状維持）。`PhysicalKeyDisposition::plan` が参照する。
    dbe_mode_key_policy: awase::config::DbeModeKeyPolicy,
    /// 左Shift単独タップによる「IME-ON 半角英数」持続トグルの許可範囲。
    /// 既定 `MsImeOnly` で従来動作を維持し、GJI 経路は `All` の明示設定時だけ
    /// `kp_shift_conv_guard_key_up` から発火する。
    half_width_alnum_toggle_policy: awase::config::HalfWidthAlnumTogglePolicy,
    /// `config.general.muhenkan_solo_tap_dedicated_fn_key` がユーザーにより
    /// 明示設定されているか（`Some`）。`true` の間は
    /// `state::gji_charset_autodetect`（ADR-091 §D3.1項目1）が専用Fnキー
    /// 変換モードに一切介入しない（手動設定が常に優先、GJI 検出時の自動
    /// 有効化・離脱時の自動解除いずれも行わない）。`apply_config_update`/
    /// 起動時に反映される。
    muhenkan_dedicated_fn_key_is_manual: bool,
    /// 専用Fnキー変換モードが現在有効か（`set_muhenkan_dedicated_fn_key_config`/
    /// `set_muhenkan_dedicated_fn_key_auto` に渡された最新の値が `Some` か）。
    /// `gji_charset_popup` が「既に有効なら設定支援ポップアップを出さない」
    /// 判定に使う。
    muhenkan_dedicated_fn_key_active: bool,
    /// `!config.general.muhenkan_solo_tap_always_suppress`（無変換単独タップが
    /// 素のパススルー設定になっているか）。GJI向け設定支援ポップアップ
    /// （ADR-091 §D3.2「設定未完了時のポップアップ」、`gji_charset_popup`）が
    /// 「ポップアップを出すべきか」の判定に使う。`apply_config_update`/
    /// 起動時に反映される。
    muhenkan_solo_tap_is_passthrough: bool,
    /// `config.general.left_thumb_key`/`right_thumb_key` のいずれかが
    /// `"VK_SPACE"` か。`true` の場合、MS-IME レジストリ自動検出の
    /// Shift+Space トグルは `engine.set_ime_toggle_auto_keys` へ反映しない
    /// （Space 親指キーの Shift リテラル送出機能との衝突を避けるため、
    /// Opus コードレビュー指摘）。`apply_config_update`/起動時に反映される。
    /// `keys.ime_toggle`（明示設定）とのマッチ判定自体は `Engine` の
    /// `special_keys` が直接持つため、`Runtime` 側に対応するフィールドは
    /// 不要（2026-08-16 ユーザー判断: 明示設定は自動検出キーと併用され、
    /// 一方を排他しない）。
    space_is_thumb_key: bool,
    /// BugReport 診断用: 現在ロード済みの `GeneralConfig.keyboard_model`。
    keyboard_model: awase::scanmap::KeyboardModel,
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
        let (left_thumb_down, right_thumb_down) = crate::hook::thumb_down_timestamps();
        build_input_context(
            self.platform_state.ime.effective_open(),
            self.platform_state.ime.input_mode(),
            self.platform_state.ime.belief.is_japanese_ime(),
            crate::tsf::observer::ime_composition_active_now(),
            &modifiers,
            left_thumb_down,
            right_thumb_down,
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

    /// 現在のフォーカス同一性（epoch + hwnd、`probe_admission::ImmLikeTicket::admit`
    /// の照合用、ADR-106 決定3）。両軸を同時に必要とする呼び出し元はこちらを使う。
    #[must_use]
    pub(crate) fn focus_fence(&self) -> crate::state::probe_admission::FocusFence {
        crate::state::probe_admission::FocusFence {
            epoch: self.platform_state.focus.focus_epoch,
            hwnd: crate::state::ime_event::HwndId(self.platform.focus.current.hwnd),
        }
    }

    /// 現在のフォーカスエポック。`focus_fence().epoch` の薄いラッパー
    /// ——epoch と hwnd のペアリング/鮮度が意味を持たない（片方だけで十分な）
    /// 呼び出し元向け。PR 109 コードレビュー軽微3の指摘により、現時点で
    /// epoch 単独の呼び出し元は無いが API として意図的に残す
    /// （`focus_hwnd()` と対称、Task3-c 参照）。
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn focus_epoch(&self) -> crate::state::probe_admission::FocusEpoch {
        self.focus_fence().epoch
    }

    /// 現在のフォーカス hwnd。`focus_fence().hwnd` の薄いラッパー
    /// ——epoch と hwnd のペアリング/鮮度が意味を持たない（片方だけで十分な）
    /// 呼び出し元向け。
    #[must_use]
    pub(crate) fn focus_hwnd(&self) -> crate::state::ime_event::HwndId {
        self.focus_fence().hwnd
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
        generation: Option<crate::state::ApplyGeneration>,
        reason: crate::state::ime_event::OpenApplyReason,
    ) {
        use awase::platform::TsfComposition as _;

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

        // BUG-34 横展開 D-prep: UnsafeToToggle をここで早期 return すると、
        // generation 付きで立てた pending が record_ime_apply_result まで
        // 届かず永久に残留する（以後の別 generation の完了が全て stale
        // 判定され続ける固着になる）。record_ime_apply_result 自身が
        // UnsafeToToggle を判別して pending だけ解放し applied は動かさない
        // ため、ここでは早期 return せず必ず通す。

        // C+D: ImeModel write-back + generation 照合 dispatch
        let acceptance = self.platform_state.ime.record_ime_apply_result(
            open,
            outcome,
            generation,
            crate::hook::current_tick_ms(),
        );

        // B: composition warm/cold 更新。stale apply 完了は GJI/Composition に伝播させない。
        if acceptance.drives_composition_side_effects() {
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

    /// フォーカス復帰後 resync（report `01M0VGJ2M5KQHD1D9V7HAMBHNT`）のハード期限
    /// タイマーをスケジュールする。resync 完了（`kp_trigger_focus_resync`）が
    /// この期限より先に `FocusResyncGate` を閉じれば、このタイマーが発火しても
    /// `open_if_current` が世代不一致/既 close で `false` を返すため無害。
    pub(crate) fn schedule_focus_resync_deadline(&mut self) {
        self.platform.timer.set(
            crate::TIMER_FOCUS_RESYNC,
            std::time::Duration::from_millis(crate::tuning::FOCUS_RESYNC_DEADLINE_MS),
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
        // NOTE: `conv_mode_policy = force` に応じてこの早期 return をスキップする
        // 例外が過去に存在した（`apply_force_on_for_imm_broken` の周期 force-ON
        // 再送を同じリフレッシュ連鎖に相乗りさせるため）。2026-08-17、ADR-094 で
        // force ポリシー自体を撤去したのに伴い削除した。`apply_force_on_for_imm_broken`
        // は常時この早期 return の影響を受ける（force policy 分岐が無くなった今、
        // 周期リフレッシュに乗るのが唯一の force-ON 経路になった）。
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
    /// `Executor::execute_from_loop` が一括でガードするが、`platform.set_ime_open` を
    /// 直接呼ぶ経路（`apply_force_on_for_imm_broken`, `try_force_on_bootstrap`,
    /// `ir_apply_drift_correction`）は `Decision`/`Effect` という抽象を経由しないため
    /// そちらのガードが効かない。これらの呼び出し元は実行前に必ずこれを確認すること。
    ///
    /// 2026-07-05: Alt+Tab 中間ウィンドウへの一瞬のフォーカス中に、これらの直接呼び出しが
    /// settle 前の不安定な状態に基づいて IME を実際に切り替えてしまうバグの修正。
    ///
    /// **2026-08-21（ADR-098 決定2/4、BUG-69）訂正**: 旧記載にあった
    /// `apply_ime_open_with_applied` / `ir_post_focus_change_snapshot` 内の
    /// 「GJI 強制 ON ブロック」は、到達不能だったため決定2 で撤去済み
    /// （メソッド自体も削除）。同関数内の「IME OFF 強制ブロック」（enforce-OFF）
    /// は、そもそもここに含めるべきではなかった——`Standard`（ImmCross）
    /// でしか実効せず、到達までに `ImeDiagnosticSnapshot::capture()` が
    /// 最大 ~250ms ブロックしうる（ImmCross の settle は 100ms）ため、
    /// defer チェックを足すと「診断キャプチャがどれだけブロックしたか」で
    /// 発火が決まる非決定的な挙動になり、正常時は不発・ハング時だけ発火する
    /// という意図と正反対になる。加えて `ImeDiagnosticSnapshot::capture` は
    /// BUG-34 が撤去/非同期化の対象とする同族の同期 `SendMessageTimeoutW` を
    /// 含むため、BUG-34 が進めばこのゲートは静かに「常に不発」へ反転する。
    /// enforce-OFF ブロック自体はこの関数を呼ばないまま維持する（決定4）。
    pub(crate) fn ime_apply_should_defer(&self) -> bool {
        self.platform_state
            .ime
            .is_focus_transition_settling(std::time::Instant::now())
    }

    /// Blacklist アプリ（Chrome 等）で IME belief が ON のとき OS に force-ON を送る。
    ///
    /// IMM クロスプロセスが使えるアプリ（通常 IMM アプリ）では何もしない。
    ///
    /// NOTE: `conv_mode_policy = force` 時にこの関数を止める早期 return が過去に
    /// 存在した（force-ON を `kp_run_inner::consume_force_open_pending` という
    /// 入力意図に紐づくトリガーへ移行していたため）。2026-08-17、ADR-094 で
    /// force ポリシー自体を撤去したのに伴い削除した。この関数は
    /// `ir_stage_notify` の周期リフレッシュに相乗りする経路であり、
    /// [ADR-086](../../../../docs/adr/086-force-write-trigger-and-target-identity.md)
    /// INV-15 が禁止する「生の周期タイマー」トリガーに該当する既知の逸脱として
    /// 残る（ADR-094 参照。`consume_force_open_pending` という INV-15 準拠の
    /// 代替経路自体も本 ADR で撤去したため、この関数が唯一の force-ON 経路になった）。
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
            self.schedule_settle_retry("apply_force_on_for_imm_broken skipped (settling)");
            return;
        }
        if !(self.engine.is_user_enabled()
            && self.platform_state.ime.is_eligible_for_ime_force_on())
        {
            return;
        }
        // ADR-098 決定1-c（BUG-69）: 従来ここは「applied が既に ON なら送らない」
        // だけの判定だった。決定1-a で TsfNative の `applied` がフォーカス入場後
        // `Unknown` のまま残るようになると、strategy chain が `Failed` を返した
        // 場合に `record_ime_apply_result` が `applied = Confirmed{open:false}`
        // を書き、この従来ガード（Optimistic(true)|Confirmed{open:true} のみ
        // 見る）を素通りする。`on_ime_apply_complete` は outcome によらず
        // `post_ime_refresh()` で 20ms 後の再試行を無条件に張り、TsfNative は
        // それを上書きする周期ポーリングが無い（`reschedule_ime_refresh` が
        // 早期 return する）ため、実効 50Hz の無限再試行ループになる——
        // 毎回 `mark_composition_cold`（打鍵中かどうかを問わない）と 2 発目の
        // eager warmup を伴う BUG-31 族の最悪形。クールダウン + 「未試行なら
        // 必ず通す」で有界化する（BUG-68 の `DRIFT_CORRECTION_BLIND_REARM_
        // COOLDOWN_MS` と同じ形）。試行回数の上限は設けない——理由は
        // `force_on_attempt_allowed`/`FORCE_ON_RETRY_COOLDOWN_MS` の doc 参照。
        let now_ms = crate::hook::current_tick_ms();
        if !self.platform_state.ime.force_on_attempt_allowed(now_ms) {
            return;
        }
        // `platform.set_ime_open` は IMM 専用実装で、Imm32Unavailable / TSF-native
        // プロファイルでは早期 return する — つまり **この関数が対象とする Blacklist
        // アプリで常に no-op だった**（2026-07-07 実機: BUG-16 の settle 明け再試行が
        // 律儀に走っても実 IME OFF が直らず「koreha」リテラル化が再発。
        // 手動 Ctrl+変換 = strategy chain 経由の apply は毎回効いていた）。
        // strategy chain（MsImeDirect の冪等 VK_DBE_HIRAGANA 等）で apply する。
        let outcome = self.force_on_and_correct_romaji(
            crate::state::ime_event::OpenApplyReason::ImmBrokenForceOn,
        );
        // UnsafeToToggle（Win キー保持等の genuine skip）は「送っていない」ので
        // クールダウンの起点にしない——数えると Win 長押し中に次のフォーカス
        // 変更まで再試行できなくなる。この場合 `applied` も更新されないため
        // （`record_ime_apply_result` が pending 解放だけして早期 return する）、
        // 次の 20ms リフレッシュがそのまま再試行する（既存の自己回復を保存）。
        if outcome != awase::platform::ImeOpenOutcome::UnsafeToToggle {
            self.platform_state.ime.note_force_on_attempt(now_ms);
        }
    }

    /// force-ON を実際に送信し、続けて非ローマ字対応 `input_mode` の補正を行う共通処理。
    ///
    /// `apply_force_on_for_imm_broken` から呼ばれる（かつての `conv_mode_policy = force`
    /// 経路 `consume_force_open_pending` は 2026-08-17、ADR-094 で force ポリシー
    /// 撤去に伴い削除済み）。
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
        // （ADR-098 決定2 で撤去済みの `ir_post_focus_change_snapshot` 内
        // TsfNative force-on ブロックも、到達不能になる前は同じ理由で
        // `None` を使っていた）。
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

            // BUG-34 横展開 D: 従来この経路は apply_ime_open_with_belief →
            // ImeController::apply（同期 chain）を経由し、ImmCrossProcessStrategy::apply
            // → set_ime_open_cross_process（150ms 宣言タイムアウトの
            // SendMessageTimeoutW）をエンジンスレッド上で直接ブロックしていた
            // （ADR-089 §9-21 の訂正どおり、Standard プロファイルでも到達しうる）。
            // executor.rs::dispatch_ime_set_open の ImmCross async path と同じ構成で
            // run_open_chain_async へ委譲する: 起案（generation の発行 + pending の
            // 設置 + warrant order + OutputActiveGuard）は spawn_local の**外**で
            // 行う（future の中では with_app 再入で ImeStateHub に届かないため、
            // ADR-090 §4.2）。
            //
            // generation は `allocate_event_generation()` を呼ぶだけでなく、
            // 必ず `ImeApplyRequested` を dispatch して `pending` を実際に立てる
            // （round-2 premortem で判明: generation を割り当てるだけで
            // ImeApplyRequested を dispatch しないと `record_ime_apply_result` の
            // generation 照合が常に不一致になり、完了が全て stale として捨てられる
            // 「空の generation」になる）。D-prep（pending の期限切れパージ・
            // UnsafeToToggle での解放・上書き検出ログ）が入っているため、
            // capture 失敗等で完了が来なかった場合も pending は 1 秒で自然に
            // パージされる。
            let now_ms = crate::hook::current_tick_ms();
            let generation = self.platform_state.ime.allocate_event_generation();
            self.platform_state.ime.dispatch_event(
                crate::state::ime_event::ImeEvent::ImeApplyRequested {
                    target: true,
                    generation,
                    ctrl_held: false,
                },
                crate::state::TickMs(now_ms),
            );
            // ADR-090 §2.A A-1（shadow）。**この入口は差分オラクルが
            // 「判明した中で最大の挙動変化」と記録している old-1 そのもの**
            // （`ImmCross` は `default_feedback = Read` なので Step 4c が
            // 発火せず、観測も意図も guard も無い bootstrap では warrant が
            // `None` になる）。A-2 で倒すのは**最後**に回すこと（ADR-090 §4.9）。
            let order = self.issue_actuation_order(true, "try_force_on_bootstrap");
            let focus_gen = self.platform.output.ime_mode_focus_gen.get();
            // MsImeDirect/ImmCross の ROMAN 補完と同じ判断（executor.rs
            // dispatch_ime_set_open の async path 参照）: ObservedKana（ユーザーが
            // 意図的にかな入力に設定した状態）以外は open と同じ hwnd へ ROMAN
            // ビットを補完する。
            let conv_after_open = if matches!(
                self.platform_state.ime.input_mode(),
                InputModeState::ObservedKana
            ) {
                crate::ime::ConvAfterOpen::Skip
            } else {
                crate::ime::ConvAfterOpen::Write(None)
            };
            let guard = crate::tsf::probe_bridge::OutputActiveGuard::begin();
            win32_async::spawn_local(async move {
                let Some(target) = crate::ime::ActuationTarget::capture(focus_gen).await else {
                    log::debug!(
                        "[force-on-bootstrap] capture 失敗（フォーカス無し） → UnsafeToToggle"
                    );
                    message_handlers::post_async_ime_apply_complete(
                        true,
                        awase::platform::ImeOpenOutcome::UnsafeToToggle,
                        Some(generation),
                        crate::state::ime_event::OpenApplyReason::Bootstrap,
                    );
                    drop(guard);
                    return;
                };
                let outcome = open_chain::run_open_chain_async(
                    order,
                    open_chain::ImmCrossOp::Targeted {
                        target,
                        conv_after_open,
                        focus_gen,
                    },
                )
                .await;
                log::info!("force-on bootstrap: apply_ime_open(true) → {outcome:?}");
                message_handlers::post_async_ime_apply_complete(
                    true,
                    outcome,
                    Some(generation),
                    crate::state::ime_event::OpenApplyReason::Bootstrap,
                );
                drop(guard);
            });
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
        let Some(layouts) = NonEmptyLayouts::new(layouts) else {
            log::warn!("reload_layouts: no layouts found, keeping current layout");
            return;
        };

        let names = layouts.names();
        let index = LayoutEntry::resolve_index(layouts.as_slice(), default_layout);
        let target_name = layouts.as_slice()[index].name.clone();
        let unchanged = self.platform.tray.current_layout_name() == target_name;

        self.layouts = layouts.into_vec();
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
        let accepted =
            crate::state::probe_admission::AcceptedObservation::for_sync(self.focus_fence());
        self.platform_state
            .ime
            .apply_ime_update(&observer_out, tick_ms, accepted);

        // LastAppliedImeState を OS 観測値に同期する。
        // 物理 Kanji キー（sync key）は apply_ime_open を経由しないため last_applied が更新されない。
        // last_applied が stale なまま Engine が activate → SetOpen(true) → KanjiToggleStrategy が
        // last_applied(false) != desired(true) と判定して VK_KANJI を余分に送信し、
        // Chrome では IME が逆転するバグを防ぐ。
        //
        // ADR-098 決定5: この関数（`process_deferred_keys`）自体は `SyncKeyGate::
        // activate()`/`try_push()` の呼び出し元が現状ゼロのため本番到達不能——
        // 到達すれば、直前の `poll_and_classify_ime` の新鮮な観測を経由せず
        // `effective_open()`（belief、explicit-intent 分岐が優先される）を
        // そのまま書く点に注意。将来 sync key gate を復活させる際は
        // `focus_tracking.rs:409` と同型のプロファイル分岐を検討すること。
        let observed_ime_on = self.platform_state.ime.effective_open();
        self.platform_state
            .ime
            .record_confirmed(observed_ime_on, tick_ms.0);
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
            dbe_mode_key_policy: awase::config::DbeModeKeyPolicy::default(),
            half_width_alnum_toggle_policy: awase::config::HalfWidthAlnumTogglePolicy::default(),
            muhenkan_dedicated_fn_key_is_manual: false,
            muhenkan_dedicated_fn_key_active: false,
            muhenkan_solo_tap_is_passthrough: false,
            space_is_thumb_key: false,
            keyboard_model: awase::scanmap::KeyboardModel::default(),
        }
    }

    pub(crate) const fn keyboard_model(&self) -> awase::scanmap::KeyboardModel {
        self.keyboard_model
    }

    pub(crate) const fn set_keyboard_model(&mut self, model: awase::scanmap::KeyboardModel) {
        self.keyboard_model = model;
    }

    /// `config.general.dbe_mode_key_policy` を反映する。起動時
    /// （`bootstrap.rs`、`conv_mode.set_policy` と同じ post-construction 経路）と
    /// `apply_config_update`（reload 時）の両方から呼ぶ。
    pub(crate) fn set_dbe_mode_key_policy(&mut self, policy: awase::config::DbeModeKeyPolicy) {
        self.dbe_mode_key_policy = policy;
    }

    /// `config.general.half_width_alnum_toggle` を反映する。起動時と reload 時の
    /// 両方から呼び、BUG-25 GJI entry の kill switch を即時に効かせる。
    pub(crate) fn set_half_width_alnum_toggle_policy(
        &mut self,
        policy: awase::config::HalfWidthAlnumTogglePolicy,
    ) {
        self.half_width_alnum_toggle_policy = policy;
    }

    /// 専用Fnキー変換モード（`muhenkan_solo_tap_dedicated_fn_key`、ADR-091 §D3.2）
    /// を反映する。`is_manual` は `config.general.muhenkan_solo_tap_dedicated_fn_key`
    /// が `Some` かどうか（`state::gji_charset_autodetect` が介入してよいかの
    /// ゲート）。起動時（`bootstrap.rs`）と `apply_config_update`（reload 時）の
    /// 両方から呼ぶ。
    pub(crate) fn set_muhenkan_dedicated_fn_key_config(
        &mut self,
        vk: Option<VkCode>,
        is_manual: bool,
    ) {
        self.engine.set_muhenkan_solo_tap_dedicated_fn_key(vk);
        self.muhenkan_dedicated_fn_key_is_manual = is_manual;
        self.muhenkan_dedicated_fn_key_active = vk.is_some();
    }

    /// `state::gji_charset_autodetect` が GJI 検出/離脱時に専用Fnキー変換モードを
    /// 自動的に有効化/解除するための入口。手動設定（`muhenkan_dedicated_fn_key_is_manual`）
    /// が有効な間は何もしない（手動設定が常に優先）。
    pub(crate) fn set_muhenkan_dedicated_fn_key_auto(&mut self, vk: Option<VkCode>) {
        if self.muhenkan_dedicated_fn_key_is_manual {
            return;
        }
        self.engine.set_muhenkan_solo_tap_dedicated_fn_key(vk);
        self.muhenkan_dedicated_fn_key_active = vk.is_some();
    }

    /// `state::gji_charset_autodetect` が手動設定かどうかを判定するための読み取り専用アクセサ。
    #[must_use]
    pub(crate) const fn muhenkan_dedicated_fn_key_is_manual(&self) -> bool {
        self.muhenkan_dedicated_fn_key_is_manual
    }

    /// `gji_charset_autodetect` が config1.db から自動検出した IME ON/OFF/
    /// トグルキーを反映するための入口（ADR-092 決定D Step4c）。`Engine`側
    /// （`match_ime_on_off_auto`/`match_ime_toggle_auto`）は手動設定
    /// （`KeysConfig.ime_on`/`ime_off`/`ime_toggle`）の内容に関わらず常に
    /// 自動リストも併用する（2026-08-16 ユーザー判断、明示 ∪ 自動）ため、
    /// ここでは手動設定の有無を確認せずそのまま反映してよい
    /// （`set_muhenkan_dedicated_fn_key_auto`と異なりRuntime側にゲートは不要）。
    pub(crate) fn set_gji_ime_on_off_toggle_auto_keys(
        &mut self,
        on: Vec<awase::config::ParsedKeyCombo>,
        off: Vec<awase::config::ParsedKeyCombo>,
        toggle: Vec<awase::config::ParsedKeyCombo>,
    ) {
        self.engine.set_ime_on_auto_keys(on);
        self.engine.set_ime_off_auto_keys(off);
        self.engine.set_ime_toggle_auto_keys(toggle);
    }

    /// GJI 離脱時、`ime_on_auto`/`ime_off_auto`/`ime_toggle_auto`を全て解除する。
    ///
    /// `ime_toggle_auto`はMS-IME側（`sync_ime_toggle_auto_detect`）とも共有する
    /// フィールドだが、`message_handlers::sync_ime_kind_from_observation`が
    /// GJI側の同期をMS-IME側より**先に**呼ぶ順序になっているため
    /// （Opusコードレビュー指摘で修正、意図的な順序——詳細は呼び出し元の
    /// コメント参照）、ここで解除してもGJI→MS-IME遷移では直後にMS-IME側が
    /// 新しい値で上書きするため破綻しない。GJI→(MS-IMEでもGJIでもない状態)
    /// では、この解除が無いと専用Fnキー同様にF15-F24のバインドが無関係な
    /// IMEの文脈に残留してしまう（過去のレビューでこの解除漏れが実際の
    /// バグとして指摘された）。
    pub(crate) fn clear_gji_ime_on_off_auto_keys(&mut self) {
        self.engine.set_ime_on_auto_keys(Vec::new());
        self.engine.set_ime_off_auto_keys(Vec::new());
        self.engine.set_ime_toggle_auto_keys(Vec::new());
    }

    /// `gji_charset_popup` が「専用Fnキー変換が既に有効なら設定支援ポップアップを
    /// 出さない」判定に使う読み取り専用アクセサ。
    #[must_use]
    pub(crate) const fn muhenkan_dedicated_fn_key_active(&self) -> bool {
        self.muhenkan_dedicated_fn_key_active
    }

    /// `config.general.muhenkan_solo_tap_always_suppress` の反転値を反映する。
    /// 起動時（`bootstrap.rs`）と `apply_config_update`（reload 時）の両方から呼ぶ。
    pub(crate) fn set_muhenkan_solo_tap_is_passthrough(&mut self, is_passthrough: bool) {
        self.muhenkan_solo_tap_is_passthrough = is_passthrough;
    }

    /// `gji_charset_popup`（ADR-091 §D3.2「設定未完了時のポップアップ」）が
    /// 「無変換単独タップが素のパススルー設定になっているか」を判定するための
    /// 読み取り専用アクセサ。
    #[must_use]
    pub(crate) const fn muhenkan_solo_tap_is_passthrough(&self) -> bool {
        self.muhenkan_solo_tap_is_passthrough
    }

    /// `config.general.left_thumb_key`/`right_thumb_key` 由来のキャッシュを
    /// 更新する（ADR-092 決定D Step4a）。起動時（`bootstrap.rs`）と
    /// `apply_config_update`（reload 時）の両方から呼ぶ。
    pub(crate) fn set_space_is_thumb_key(&mut self, space_is_thumb_key: bool) {
        self.space_is_thumb_key = space_is_thumb_key;
    }

    /// `sync_ime_toggle_auto_detect`（`message_handlers.rs`）が Shift+Space の
    /// 自動検出を反映すべきかの判定に使う。
    #[must_use]
    pub(crate) const fn space_is_thumb_key(&self) -> bool {
        self.space_is_thumb_key
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
        self.set_keyboard_model(config.general.keyboard_model);
        self.set_dbe_mode_key_policy(config.general.dbe_mode_key_policy);
        self.set_half_width_alnum_toggle_policy(config.general.half_width_alnum_toggle);
        crate::hook::set_swallow_alt_kana_mode_switch(
            config.general.swallow_alt_kana_input_method_switch,
        );
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
        // disable_apps がリロードで変わった場合に備え、現在のフォーカス先で
        // 無効化状態を再評価する（BUG-78 対策の一部）。
        if self.platform.focus.is_focused() {
            let pid = self.platform.focus.pid();
            self.apply_app_disable_transition(pid, false);
        }
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
                TextKeyConfig {
                    ignore_composing_guard: config.general.space_thumb_ignore_composing_guard,
                    shift_literal: config.general.space_thumb_shift_literal,
                },
            );
            let muhenkan_vk = [left, right]
                .into_iter()
                .find(|&vk| vk == crate::vk::VK_NONCONVERT);
            let henkan_vk = [left, right]
                .into_iter()
                .find(|&vk| vk == crate::vk::VK_CONVERT);
            self.engine.set_thumb_key_solo_tap_config(
                muhenkan_vk,
                ModeKeyConfig::from_legacy_bools(
                    config.general.muhenkan_solo_tap_ignore_composing_guard,
                    config.general.muhenkan_solo_tap_always_suppress,
                ),
                henkan_vk,
                ModeKeyConfig::from_legacy_bools(
                    config.general.henkan_solo_tap_ignore_composing_guard,
                    config.general.henkan_solo_tap_always_suppress,
                ),
            );
            let manual_fn_key = config.general.muhenkan_solo_tap_dedicated_fn_key.as_deref();
            if manual_fn_key.is_some() || self.muhenkan_dedicated_fn_key_is_manual() {
                // 手動設定が今回あるか、直前まで手動設定だった（＝今回外れた）場合
                // のみ反映する。手動設定が既に無い（自動判定/ポップアップに委ねて
                // いる）場合はここで触らない — 無関係な設定リロードのたびに
                // 自動判定/ポップアップが有効化した専用Fnキーを None で
                // 上書きしてしまう回帰を防ぐ（Opus レビュー指摘）。
                self.set_muhenkan_dedicated_fn_key_config(
                    resolve_dedicated_fn_key(manual_fn_key),
                    manual_fn_key.is_some(),
                );
            }
            self.set_muhenkan_solo_tap_is_passthrough(
                ModeKeyConfig::from_legacy_bools(
                    config.general.muhenkan_solo_tap_ignore_composing_guard,
                    config.general.muhenkan_solo_tap_always_suppress,
                )
                .is_passthrough(),
            );
            self.set_space_is_thumb_key(
                config.general.left_thumb_key == "VK_SPACE"
                    || config.general.right_thumb_key == "VK_SPACE",
            );
            let enter_thumb_vk = [left, right]
                .into_iter()
                .find(|&vk| vk == crate::vk::VK_RETURN);
            self.engine.set_enter_thumb_config(
                enter_thumb_vk,
                TextKeyConfig {
                    ignore_composing_guard: config.general.enter_thumb_ignore_composing_guard,
                    shift_literal: config.general.enter_thumb_shift_literal,
                },
            );
            self.engine
                .set_thumb_shift_faces_enabled(crate::app::thumb_shift_faces_enabled_for(
                    left, right,
                ));
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
        // [[keymap]] latch も同じ理由で解放する（ADR-114 決定4「latch
        // 漏れ対策」経路5）。
        self.platform_state.keymap.keymap_latch.release_all();

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
