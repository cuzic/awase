//! 新 Engine: NicolaFsm + 特殊キー処理を統合するラッパー。
//!
//! `on_input` / `on_timeout` / `on_command` が唯一のエントリポイント。
//! OS API を一切呼ばず、副作用は `Decision` として返す。
//!
//! # 設計方針
//!
//! Engine は near-pure function として設計されている。
//! - 物理キー状態（修飾キー、親指キー）は Platform 層が InputTracker で追跡し、
//!   InputContext 経由で毎回渡す
//! - IME ガード（遷移中のキーバッファリング）は Platform 層が担当する
//! - Engine は InputContext のスナップショットだけで判断する（先読みしない）

use crate::config::ParsedKeyCombo;
use crate::types::{ContextChange, KeyEventType, RawKeyEvent, ShadowImeAction, VkCode};

use super::decision::{
    ActivationState, Decision, Effect, EffectVec, EngineCommand, ImeEffect, InactiveReason,
    InputContext, InputEffect, SetOpenOrigin, SpecialKeyCombos, UiEffect,
};
use super::fsm_adapter::FsmAdapter;
use super::fsm_types::{ComposingHint, ModeKeyConfig, ModifierState, TextKeyConfig};
use super::input_tracker::PhysicalKeyState;
use super::key_lifecycle::KeyLifecycle;
use super::nicola_fsm::NicolaFsm;

/// 特殊キーコンボのマッチ結果
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub(super) enum SpecialKeyMatch {
    EngineOn,
    EngineOff,
    ImeOn,
    ImeOff,
    /// IME の ON/OFF を反転する（ADR-092 決定D Step4a）。
    ImeToggle,
}

/// 統合エンジン: NicolaFsm + 特殊キー処理
///
/// Engine の有効状態は2軸で決まる:
/// - `user_enabled`: ユーザーの意図（ホットキー/トレイで操作）= FSM の `enabled` フラグ
/// - 環境前提条件: `InputContext { ime_on, is_romaji, is_japanese_ime, ... }` — Platform 層が毎回渡す
/// - 実効状態: `compute_state(ctx)` が `ActivationState::Active` を返すとき
///
/// Engine は前提条件を内部にキャッシュしない。毎回の呼び出しで Platform 層から受け取る。
///
/// `on_input` が唯一のキーイベントエントリポイント。
/// OS API を一切呼ばず、副作用は `Decision` として返す。
#[allow(missing_debug_implementations)]
pub struct Engine {
    adapter: FsmAdapter,
    special_keys: SpecialKeyCombos,
    /// 自動検出された IME トグルキー（ADR-092 決定D Step4a/Step4c、MS-IME
    /// レジストリの `KeyAssignmentCtrlSpace`/`KeyAssignmentShiftSpace`、または
    /// GJI config1.db の `GjiImeKeys.toggle` 由来。両ソースは排他——呼び出し元
    /// が IME 種別確定イベントごとにどちらか一方だけを呼ぶ）。
    /// `special_keys.ime_toggle`（ユーザーが `config.toml` に明示設定した分）
    /// とは別に保持し、`config.toml` へは一切書き込まない（決定C: Manual は
    /// 永続化、AutoDetected はライブ計算のみ）。`special_keys.ime_toggle` が
    /// 空の場合のみ参照される（決定C R1、明示>自動）。
    ime_toggle_auto: Vec<ParsedKeyCombo>,
    /// 自動検出された IME ON キー（ADR-092 決定D Step4c、GJI config1.db の
    /// `GjiImeKeys.on` 由来）。`ime_toggle_auto` と同じ規約
    /// （`config.toml` 非書き込み、`special_keys.ime_on` が空の場合のみ参照）。
    ime_on_auto: Vec<ParsedKeyCombo>,
    /// 自動検出された IME OFF キー（ADR-092 決定D Step4c、GJI config1.db の
    /// `GjiImeKeys.off` 由来）。`ime_on_auto` と対称。
    ime_off_auto: Vec<ParsedKeyCombo>,
    /// キーの Down/Up ペア追跡
    lifecycle: KeyLifecycle,
    /// 直前の実効状態（遷移検知用）
    prev_activation: ActivationState,
    /// 直近の `on_timeout` でソロ連打緊急 OFF が発動したかの 1 ショットフラグ。
    /// Platform 層がトレイ通知を出すかどうかの判定に使う（`take_solo_off_notification`）。
    solo_off_notify: bool,
}

impl Engine {
    #[must_use]
    pub const fn new(fsm: NicolaFsm, special_keys: SpecialKeyCombos) -> Self {
        Self {
            adapter: FsmAdapter::new(fsm),
            special_keys,
            ime_toggle_auto: Vec::new(),
            ime_on_auto: Vec::new(),
            ime_off_auto: Vec::new(),
            lifecycle: KeyLifecycle::new(),
            prev_activation: ActivationState::Inactive(InactiveReason::UserDisabled),
            solo_off_notify: false,
        }
    }

    /// MS-IME レジストリ自動検出（Ctrl+Space/Shift+Space）または GJI
    /// config1.db（`GjiImeKeys.toggle`）由来の IME トグルキーを設定する
    /// （ADR-092 決定D Step4a/Step4c）。IME 種別確定イベントのたびに呼び直され、
    /// 呼ばれるたびに丸ごと置き換わる（決定C R2、計算は毎回やり直す）。
    pub fn set_ime_toggle_auto_keys(&mut self, keys: Vec<ParsedKeyCombo>) {
        self.ime_toggle_auto = keys;
    }

    /// GJI config1.db（`GjiImeKeys.on`）由来の自動検出 IME ON キーを設定する
    /// （ADR-092 決定D Step4c）。`set_ime_toggle_auto_keys` と同じ規約。
    pub fn set_ime_on_auto_keys(&mut self, keys: Vec<ParsedKeyCombo>) {
        self.ime_on_auto = keys;
    }

    /// `set_ime_on_auto_keys` と対称（`GjiImeKeys.off` 由来）。
    pub fn set_ime_off_auto_keys(&mut self, keys: Vec<ParsedKeyCombo>) {
        self.ime_off_auto = keys;
    }

    /// ソロ N 連打でエンジン OFF を発動するキーを設定する。
    /// `VkCode(0)` を渡すと機能を無効にする。
    pub const fn set_engine_off_triple_vk(&mut self, vk: VkCode) {
        self.adapter.set_engine_off_triple_vk(vk);
    }

    /// Space 親指キーのフォールバック挙動を設定する。
    ///
    /// `space_thumb_vk` は `left_thumb_key`/`right_thumb_key` のいずれかが
    /// Space (`VK_SPACE`) に解決された場合の VK コード（Platform 層が判定して渡す。
    /// どちらも Space でなければ `None`）。`ignore_composing_guard`/`shift_literal`
    /// は `GeneralConfig` の同名フィールドにそのまま対応する。
    pub const fn set_space_thumb_config(
        &mut self,
        space_thumb_vk: Option<VkCode>,
        config: TextKeyConfig,
    ) {
        self.adapter.set_space_thumb_config(space_thumb_vk, config);
    }

    /// 無変換/変換キー単独タップの composing 中ガードの扱いを設定する。
    ///
    /// `muhenkan_vk`/`henkan_vk` は `left_thumb_key`/`right_thumb_key` がそれぞれ
    /// 無変換/変換に解決された場合の VK コード（Platform 層が判定して渡す。
    /// 割り当てられていなければ `None`）。各 `ignore_composing_guard` は
    /// `GeneralConfig` の同名フィールドにそのまま対応する。
    /// `muhenkan`/`henkan` の各フィールドは `GeneralConfig` の同名フィールド
    /// （`muhenkan_solo_tap_ignore_composing_guard`/`muhenkan_solo_tap_always_suppress`/
    /// `henkan_solo_tap_ignore_composing_guard`/`henkan_solo_tap_always_suppress`）に
    /// そのまま対応する。
    pub const fn set_thumb_key_solo_tap_config(
        &mut self,
        muhenkan_vk: Option<VkCode>,
        muhenkan: ModeKeyConfig,
        henkan_vk: Option<VkCode>,
        henkan: ModeKeyConfig,
    ) {
        self.adapter
            .set_thumb_key_solo_tap_config(muhenkan_vk, muhenkan, henkan_vk, henkan);
    }

    /// 無変換単独タップの専用 Fn キー変換モード（ADR-091 §D3.2）を設定する。
    /// `GeneralConfig::muhenkan_solo_tap_dedicated_fn_key` を解決した VK コードを
    /// 渡す。`set_thumb_key_solo_tap_config` とは独立して呼び出せる。
    pub const fn set_muhenkan_solo_tap_dedicated_fn_key(&mut self, vk: Option<VkCode>) {
        self.adapter.set_muhenkan_solo_tap_dedicated_fn_key(vk);
    }

    /// 無変換キー単独タップの IME open 軸への肩代わり（ADR-092 決定D Step4b、
    /// MS-IME レジストリ/GJI config1.db の宣言由来）を設定する。
    /// `set_thumb_key_solo_tap_config`/`set_muhenkan_solo_tap_dedicated_fn_key`
    /// とは独立して呼び出せる。
    pub const fn set_muhenkan_delegate_to_open_axis(&mut self, action: Option<ShadowImeAction>) {
        self.adapter.set_muhenkan_delegate_to_open_axis(action);
    }

    /// `set_muhenkan_delegate_to_open_axis` と対称（変換キー用）。
    pub const fn set_henkan_delegate_to_open_axis(&mut self, action: Option<ShadowImeAction>) {
        self.adapter.set_henkan_delegate_to_open_axis(action);
    }

    /// Enter 親指キーのフォールバック挙動を設定する。
    ///
    /// `enter_thumb_vk` は `left_thumb_key`/`right_thumb_key` のいずれかが
    /// Enter (`VK_RETURN`) に解決された場合の VK コード（Platform 層が判定して渡す。
    /// どちらも Enter でなければ `None`）。`ignore_composing_guard`/`shift_literal`
    /// は `GeneralConfig` の同名フィールドにそのまま対応する。
    pub const fn set_enter_thumb_config(
        &mut self,
        enter_thumb_vk: Option<VkCode>,
        config: TextKeyConfig,
    ) {
        self.adapter.set_enter_thumb_config(enter_thumb_vk, config);
    }

    /// InputContext から実効状態を `ActivationState` で返す。
    ///
    /// 判定順: user_enabled → is_japanese_ime → ime_on → is_romaji
    /// 各条件が false のとき対応する `InactiveReason` を返す。
    #[must_use]
    pub const fn compute_state(&self, ctx: &InputContext) -> ActivationState {
        if !self.adapter.is_enabled() {
            return ActivationState::Inactive(InactiveReason::UserDisabled);
        }
        if !ctx.is_japanese_ime {
            return ActivationState::Inactive(InactiveReason::NotJapaneseIme);
        }
        if !ctx.ime_on {
            return ActivationState::Inactive(InactiveReason::ImeOff);
        }
        if !ctx.input_mode.is_romaji_capable() {
            return ActivationState::Inactive(InactiveReason::NotRomajiInput);
        }
        ActivationState::Active
    }

    /// InputContext から実効状態を bool で返す（後方互換 API）。
    #[must_use]
    pub const fn compute_active(&self, ctx: &InputContext) -> bool {
        self.compute_state(ctx).is_active()
    }

    /// 実効状態の遷移を検知し、必要な Effect（flush, UI 通知）を返す。
    fn check_active_transition(&mut self, ctx: &InputContext) -> EffectVec {
        let new_state = self.compute_state(ctx);
        let was_active = self.prev_activation.is_active();
        let now_active = new_state.is_active();
        let mut effects = EffectVec::new();

        // [diag-engine-active] BUG-42系（belief と engine active state の乖離、
        // 「IME ON なのに Engine OFF のまま」）の切り分け用の一時的な診断ログ。
        // 既存の `Engine {activated,deactivated}` ログは active/inactive が
        // "遷移した" 場合にしか出ないため、ime_on=true のまま何らかの理由で
        // 非活性が継続している（＝遷移が起きない）ケースを毎キー入力ごとに
        // 可視化する。遷移の有無を問わず出すため log::debug! で十分な頻度に留める。
        if ctx.ime_on && !now_active {
            log::debug!(
                "[diag-engine-active] ime_on=true なのに非活性: reason={:?} \
                 romaji_capable={} japanese={} user_enabled={} was_active={} input_mode={:?}",
                new_state,
                ctx.input_mode.is_romaji_capable(),
                ctx.is_japanese_ime,
                self.adapter.is_enabled(),
                was_active,
                ctx.input_mode,
            );
        }

        if was_active != now_active {
            if !now_active {
                // active → inactive: 保留キーをフラッシュ。
                // ctx.composing はこの呼び出し時点の最新値であり、保留キーが入力された
                // 時点と同一ウィンドウ/コンテキストである保証がない（フォーカス変更に
                // 伴う non-active 化等）ため Unknown を渡し、Space フォールバック例外も
                // 含め無条件 suppress する（ComposingHint の doc 参照）。
                let reason = new_state.to_context_change();
                let flush = self
                    .adapter
                    .flush_to_effects(reason, ComposingHint::Unknown);
                effects.extend(flush);
                // lifecycle をクリア: Engine が consumed した KeyDown の対応 KeyUp が
                // Engine inactive 時に到着しても consumed されないようにする。
                let _ = self.lifecycle.flush_pending_key_ups();
            }
            log::info!(
                "Engine {} (ime={}, romaji={}, japanese={}, user={}, reason={:?})",
                if now_active {
                    "activated"
                } else {
                    "deactivated"
                },
                ctx.ime_on,
                ctx.input_mode.is_romaji_capable(),
                ctx.is_japanese_ime,
                self.adapter.is_enabled(),
                new_state,
            );
        }

        // ここで発行される SetOpen は `check_active_transition`（Phase 2、通常の毎キー
        // 入力経路）由来であり、ユーザーが今このキーで IME ON/OFF を明示的に選んだ
        // わけではない（`ctx.ime_on` が観測駆動で変化しただけでも Active/Inactive は
        // 遷移しうる）。`SetOpenOrigin::ActivationSync` を渡し、Platform 層が
        // `last_intent`（ユーザー明示意図）を汚染しないようにする（`SetOpenOrigin` の
        // doc 参照）。
        let transition_effects =
            self.transition_activation(new_state, SetOpenOrigin::ActivationSync);
        effects.extend(transition_effects);
        effects
    }

    /// 実効状態を新しい状態に遷移させ、変化があった場合に SetOpen + UiEffect を返す。
    ///
    /// inactive → active: OS IME を強制的に開く（"nonaiyo" 問題対策）
    /// active → inactive: OS IME を強制的に閉じる（対称性のため）
    ///   ただし NotRomajiInput（tray での英数モード選択等）の場合は SetOpen(false) を出さない。
    ///   ユーザーが既に望むモード（全角英数等）を選択済みなので、VK_DBE_ALPHANUMERIC を
    ///   追加送信すると全角英数→半角英数のような意図しない conv 変化が起きる。
    /// 同じ状態: 空の EffectVec
    ///
    /// `origin`: 発行する `ImeEffect::SetOpen` に付与する `SetOpenOrigin`。呼び出し元が
    /// 「これは本物のユーザー操作（IME/エンジン ON/OFF コンボ、トレイ操作等）が引き金か、
    /// それとも通常のキー入力経路での自動遷移か」を判断して渡すこと。
    fn transition_activation(
        &mut self,
        new_state: ActivationState,
        origin: SetOpenOrigin,
    ) -> EffectVec {
        let was_active = self.prev_activation.is_active();
        let now_active = new_state.is_active();
        let mut effects = EffectVec::new();

        if was_active != now_active {
            let suppress_set_open = matches!(
                new_state,
                ActivationState::Inactive(InactiveReason::NotRomajiInput)
            );
            if !suppress_set_open {
                effects.push(Effect::Ime(ImeEffect::SetOpen {
                    open: now_active,
                    origin,
                }));
            }
            // NotRomajiInput の場合は SetOpen も engine-state キーも不要。
            // ユーザーが選択した kana/katakana モードをそのまま維持する。
            let suppress_ime_key = suppress_set_open; // 同じ条件
            effects.push(Effect::Ui(UiEffect::EngineStateChanged {
                enabled: now_active,
                send_ime_key: !suppress_ime_key,
            }));
            self.prev_activation = new_state;
        }
        effects
    }

    /// キーイベントの統合エントリポイント。
    ///
    /// 処理フロー:
    /// 1. KeyUp 自動追跡
    /// 2. 特殊キー（エンジン ON/OFF + IME 制御）
    /// 3. 実効状態チェック + 遷移検知
    /// 4. NicolaFsm 処理
    pub fn on_input(&mut self, event: RawKeyEvent, ctx: &InputContext) -> Decision {
        // Phase 0: KeyUp 自動追跡
        let is_key_down = matches!(event.event_type, KeyEventType::KeyDown);
        if !is_key_down && self.lifecycle.on_key_up(event.vk_code) {
            return Decision::consumed();
        }

        // Phase 1: Special keys (engine toggle + IME control)
        if is_key_down {
            if let Some(decision) = self.check_special_keys(ctx, &event) {
                if decision.is_consumed() {
                    self.lifecycle.on_key_down_consumed(&event);
                }
                return decision;
            }
        }

        // Phase 2: Active state check + transition detection
        let transition_effects = self.check_active_transition(ctx);
        if !self.compute_active(ctx) {
            if transition_effects.is_empty() {
                return Decision::pass_through();
            }
            return Decision::pass_through_with(transition_effects);
        }

        // Phase 3: NicolaFsm
        let phys = PhysicalKeyState::from_ctx(ctx, &event);
        let mut decision = self.adapter.on_event(event, &phys);
        if is_key_down && decision.is_consumed() {
            self.lifecycle.on_key_down_consumed(&event);
        }
        decision.prepend_effects(transition_effects);
        self.apply_ime_open_request(&mut decision, ctx);
        decision
    }

    /// タイマー満了時のエントリポイント。
    pub fn on_timeout(&mut self, timer_id: usize, ctx: &InputContext) -> Decision {
        let phys = PhysicalKeyState::from_ctx_snapshot(ctx);

        // Engine が非活性なら on_timeout せず flush（コンテキスト喪失）。
        // 非活性化の理由（IME OFF・フォーカス変更等）を問わず、保留キーが入力された
        // 時点と同一コンテキストである保証がないため Unknown を渡す。
        if !self.compute_active(ctx) {
            return self
                .adapter
                .flush(ContextChange::ImeOff, ComposingHint::Unknown);
        }

        let mut decision = self.adapter.on_timeout(timer_id, &phys, ctx.composing);

        // ソロ連打によるエンジン OFF トリガー
        if self.adapter.take_engine_off_requested() {
            log::info!("Engine OFF triggered by consecutive solo key presses");
            self.solo_off_notify = true;
            return self.apply_special_key_match(&SpecialKeyMatch::EngineOff, ctx);
        }

        self.apply_ime_open_request(&mut decision, ctx);
        decision
    }

    /// `NicolaFsm::take_ime_open_requested`（ADR-092 決定D Step4b、無変換/変換
    /// 単独タップの IME open 軸への肩代わり）を確認し、あれば `decision` の
    /// 既存の効果（キー抑止・タイマー等）を保ったまま `Effect::Ime(SetOpen)`
    /// を追加する。`origin: ExplicitUserAction` は `Effect::Ime(SetOpen)` の
    /// 既存の消費経路（`awase-windows::key_pipeline::kp_stage_post_decision`）
    /// で `UserIntentSource::Command`（「awase エンジン内部の判断」）として
    /// 記録される——新しい witness 種別は不要（Opus コードレビュー指摘、
    /// 当初案の `SyncKey` witness は無変換/変換の毎打鍵で誤発火する致命的な
    /// 欠陥があった）。
    fn apply_ime_open_request(&mut self, decision: &mut Decision, ctx: &InputContext) {
        let Some(action) = self.adapter.take_ime_open_requested() else {
            return;
        };
        let new_open = match action {
            ShadowImeAction::TurnOn => true,
            ShadowImeAction::TurnOff => false,
            ShadowImeAction::Toggle => !ctx.ime_on,
        };
        log::info!("IME open axis delegated (solo tap, key semantics absorption) → {new_open}");
        // ime_on/ime_off コンボキーと同じ `ime_set_open_effects` を経由する
        // （`prev_activation` を進めて次打鍵での重複 SetOpen を防ぐため必須、
        // 直接 push_effect してはならない。上のdoc参照）。
        for effect in self.ime_set_open_effects(ctx, new_open) {
            decision.push_effect(effect);
        }
    }

    /// `ime_open_requested`（あれば）を適用せずに捨てる。`on_command` の
    /// `ToggleEngine`/`SwapLayout` アーム専用（コメント参照）。
    ///
    /// `on_input`/`on_timeout` は `apply_ime_open_request` を必ず呼ぶため、
    /// このワンショットチャネルが「取り出されないまま残留し、無関係な
    /// 次のイベントで誤発火する」経路を `on_command` の全アームで塞ぐ必要が
    /// ある（Opus コードレビュー指摘: `ToggleEngine`/`SwapLayout` は
    /// `on_command` 経由でのみ到達し、どちらも取り出し漏れがあった）。
    fn discard_ime_open_request(&mut self) {
        let _ = self.adapter.take_ime_open_requested();
    }

    /// 直近の `on_timeout` でソロ連打緊急 OFF が発動したかを取得する（1 ショット）。
    ///
    /// Platform 層がトレイ通知等でユーザーに「engine が緊急停止したこと」と
    /// 復帰方法を知らせるために使う。通常の `Ctrl+Shift+変換/無変換` による
    /// 意図的な engine on/off ではこのフラグは立たない。
    pub fn take_solo_off_notification(&mut self) -> bool {
        std::mem::take(&mut self.solo_off_notify)
    }

    /// 外部コマンドの統合エントリポイント。
    ///
    /// `toggle_engine`, `invalidate_engine_context`, `swap_layout` 等の個別メソッドを
    /// 単一のディスパッチに集約する。
    pub fn on_command(&mut self, cmd: EngineCommand, ctx: &InputContext) -> Decision {
        match cmd {
            EngineCommand::ToggleEngine => {
                let old_active = self.compute_active(ctx);
                let (user_enabled, mut decision) = self.adapter.toggle_enabled();
                let new_active = self.compute_active(ctx);
                log::info!(
                    "Engine user_enabled toggled: {} (active: {})",
                    if user_enabled { "ON" } else { "OFF" },
                    if new_active { "ON" } else { "OFF" },
                );
                if user_enabled && !new_active {
                    // ユーザーが明示的に有効化したが ime_on=false 等で active になれない。
                    // pseudo_ctx で IME 強制 ON + tray 更新を行う。
                    self.apply_engine_on_with_ime_recovery(ctx, &mut decision);
                } else {
                    self.apply_active_transition(old_active, new_active, &mut decision);
                }
                // `self.adapter.toggle_enabled()` 内部の flush が保留中の親指キーを
                // 「単独タップ確定」として解決しうるため、ADR-092 Step4b の
                // `ime_open_requested` がセットされている可能性がある（Opus
                // コードレビュー指摘）。しかしこれはユーザーが無変換/変換を実際に
                // タップしたのではなく、トレイ操作等の無関係な外部イベントによって
                // 強制的に解決されたものであり、「単独タップ=IME切替意図」という
                // ただでさえ推定である解釈（決定D Step4bのリスク1参照）をさらに弱める。
                // 適用せず捨てる（次の無関係な打鍵でスプリアスな SetOpen が
                // 発火する回帰を防ぐ）。
                self.discard_ime_open_request();
                decision
            }
            // InvalidateContext は外部コンテキスト喪失（IME OFF・言語切替等）の汎用通知
            // であり、composing が保留キーと同一コンテキストか保証できないため Unknown。
            EngineCommand::InvalidateContext(reason) => {
                self.adapter.flush(reason, ComposingHint::Unknown)
            }
            EngineCommand::SwapLayout(layout) => {
                let decision = self.adapter.swap_layout(layout);
                // ToggleEngine と同じ理由（上記コメント参照）で discard する。
                self.discard_ime_open_request();
                decision
            }
            EngineCommand::ReloadKeys { special } => {
                self.special_keys = special;
                Decision::pass_through()
            }
            EngineCommand::UpdateFsmParams {
                threshold_ms,
                confirm_mode,
                speculative_delay_ms,
            } => {
                self.adapter.set_threshold_ms(threshold_ms);
                self.adapter
                    .set_confirm_mode(confirm_mode, speculative_delay_ms);
                Decision::pass_through()
            }
            EngineCommand::SetNgramModel(model) => {
                self.adapter.set_ngram_model(model);
                Decision::pass_through()
            }
            EngineCommand::RefreshState => {
                // Platform 層がアトミック変数を更新済み。ctx に反映されている。
                let effects = self.check_active_transition(ctx);
                if effects.is_empty() {
                    Decision::pass_through()
                } else {
                    Decision::pass_through_with(effects)
                }
            }
            EngineCommand::FocusChanged => self.handle_focus_changed(ctx),
            EngineCommand::ForceEngineOn => self.force_enable_and_activate(ctx, "force"),
        }
    }

    /// フォーカス変更の観測結果を処理し、コンテキスト無効化等の Decision を返す。
    /// フォーカス変更（前面プロセス変更）の処理。
    ///
    /// デバウンス後に Platform 層が前面プロセスの変化を検出した場合のみ呼ばれる（ADR 028）。
    /// focus_kind / app_kind / last_focus_info / キャッシュの更新は Platform 層で完了済み。
    /// Engine は pending flush と lifecycle 整合のみ担当する。
    fn handle_focus_changed(&mut self, ctx: &InputContext) -> Decision {
        let mut effects = EffectVec::new();

        // アプリ切替: 前のウィンドウで入力途中だったキーを別のウィンドウに持ち越さない。
        // ctx.composing はこの時点で既に新ウィンドウの状態を指しうる
        // （フォーカス切替が先に完了してから build_ctx() が呼ばれるため）ので、
        // Unknown を渡して Space フォールバック例外も含め無条件 suppress する。
        // 生 VK_SPACE 等が別ウィンドウへ誤注入されるのを防ぐ安全側の選択。
        let flush_effects = self
            .adapter
            .flush_to_effects(ContextChange::FocusChanged, ComposingHint::Unknown);
        effects.extend(flush_effects);

        // Consume 済みで KeyUp が来ていないキーの KeyUp を再注入して
        // OS 側のキーボード状態と整合させる。
        let pending_key_ups = self.lifecycle.flush_pending_key_ups();
        for evt in pending_key_ups {
            effects.push(Effect::Input(InputEffect::ReinjectKey(evt)));
        }

        // 実効状態の遷移を検知
        let transition_effects = self.check_active_transition(ctx);
        effects.extend(transition_effects);

        Decision::pass_through_with(effects)
    }

    /// user_enabled のみ
    #[must_use]
    pub const fn is_user_enabled(&self) -> bool {
        self.adapter.is_enabled()
    }

    /// 診断用: 現在の FSM 状態を短い文字列で返す。
    /// `[engine-input]` ログで `on_input` 呼び出し前の状態を可視化するために使用。
    #[must_use]
    pub fn debug_state_label(&self) -> String {
        self.adapter.debug_state_label()
    }

    /// user_enabled を直接設定する（テスト・初期化用）
    pub fn set_user_enabled(&mut self, enabled: bool) {
        let _ = self.adapter.set_enabled(enabled);
    }

    /// 前回の実効状態を直接設定する（テスト・初期化用）。
    pub const fn set_prev_active(&mut self, active: bool) {
        self.prev_activation = if active {
            ActivationState::Active
        } else {
            ActivationState::Inactive(InactiveReason::UserDisabled)
        };
    }

    // ── 内部メソッド ──

    /// user_enabled 変更後の active 遷移を Decision に反映する。
    ///
    /// 呼び出し元（`EngineCommand::ToggleEngine` / `EngineOn`・`EngineOff` コンボ）は
    /// いずれもユーザーの明示操作が引き金のため、`SetOpenOrigin::ExplicitUserAction` を
    /// 使う（`check_active_transition` 由来の `ActivationSync` とは区別する）。
    fn apply_active_transition(
        &mut self,
        old_active: bool,
        new_active: bool,
        decision: &mut Decision,
    ) {
        if old_active != new_active {
            // prev_activation を呼び出し時点の実際の状態に同期してから遷移させる
            self.prev_activation = if old_active {
                ActivationState::Active
            } else {
                ActivationState::Inactive(InactiveReason::UserDisabled)
            };

            let new_state = if new_active {
                ActivationState::Active
            } else {
                ActivationState::Inactive(InactiveReason::UserDisabled)
            };
            let effects = self.transition_activation(new_state, SetOpenOrigin::ExplicitUserAction);
            for e in effects {
                decision.push_effect(e);
            }
        }
    }

    /// エンジン有効化時に IME が OFF で active になれない場合の回復処理。
    ///
    /// `user_enabled=true` だが `ime_on=false` 等で `compute_active` が false のとき、
    /// pseudo_ctx (ime_on=true) で目標状態を計算し `transition_activation` を実行する。
    /// これにより `ImeEffect::SetOpen{true}` と `UiEffect::EngineStateChanged{true}` が
    /// 発行され、IME 強制 ON と tray 更新が行われる。
    ///
    /// `is_japanese_ime=false` 等で IME を ON にしても active になれない場合は
    /// `SetOpen{true}` のみ追加する（意図を Platform 層に伝えるため）。
    ///
    /// EngineOn コンボ・`ForceEngineOn` コマンドいずれもユーザーの明示操作が引き金のため
    /// `SetOpenOrigin::ExplicitUserAction` を使う。
    fn apply_engine_on_with_ime_recovery(&mut self, ctx: &InputContext, decision: &mut Decision) {
        let pseudo_ctx = InputContext {
            ime_on: true,
            ..*ctx
        };
        let target_state = self.compute_state(&pseudo_ctx);
        let effects = self.transition_activation(target_state, SetOpenOrigin::ExplicitUserAction);
        if effects.is_empty() {
            decision.push_effect(Effect::Ime(ImeEffect::SetOpen {
                open: true,
                origin: SetOpenOrigin::ExplicitUserAction,
            }));
        } else {
            for e in effects {
                decision.push_effect(e);
            }
        }
    }

    /// `open` を反映した擬似 `InputContext` で新 `ActivationState` を求め、
    /// `transition_activation` で `SetOpen + EngineStateChanged` を発行する
    /// （ユーザー明示操作起点、`origin: ExplicitUserAction` 固定）。状態が遷移
    /// しない場合（例: `user_enabled=false` で既に Inactive）は `SetOpen` のみを
    /// 明示的に追加する（IME 制御の意図を Platform 層に伝えるため）。
    ///
    /// # 二重 enqueue 防止
    ///
    /// `transition_activation` で `prev_activation` を新状態に推進するため、
    /// 次回の `check_active_transition` は no-op となり、構造的に重複を排除する。
    /// **呼び出し元は必ずこのヘルパー経由で `SetOpen` 効果を生成すること**
    /// （`build_ime_set_open_decision`/`apply_ime_open_request` 共通。Opus
    /// コードレビュー指摘: `apply_ime_open_request` が当初これを経由せず
    /// `Decision::push_effect` で `SetOpen` を直接追加していたため
    /// `prev_activation` が進まず、次の打鍵で `ActivationSync` 起点の重複
    /// `SetOpen` + 不要な `EngineStateChanged` が再発火する回帰があった）。
    fn ime_set_open_effects(&mut self, ctx: &InputContext, open: bool) -> EffectVec {
        let pseudo_ctx = InputContext {
            ime_on: open,
            ..*ctx
        };
        let new_state = self.compute_state(&pseudo_ctx);
        let was_active = self.prev_activation.is_active();
        let now_active = new_state.is_active();

        let mut effects = self.transition_activation(new_state, SetOpenOrigin::ExplicitUserAction);
        if was_active == now_active {
            // 状態遷移なし → transition_activation は空 effects を返す。
            // IME 制御の意図 (SetOpen) は明示的に追加する。
            effects.push(Effect::Ime(ImeEffect::SetOpen {
                open,
                origin: SetOpenOrigin::ExplicitUserAction,
            }));
        }
        effects
    }

    /// IME ON/OFF コンボキーに対する Decision を構築する（`ime_set_open_effects`
    /// 参照）。
    fn build_ime_set_open_decision(&mut self, ctx: &InputContext, open: bool) -> Decision {
        Decision::consumed_with(self.ime_set_open_effects(ctx, open))
    }

    /// 与えられたイベントが IME OFF コンボキーにマッチするかを副作用なしで返す。
    ///
    /// Platform 層が「即時 IME OFF か 50ms 救済窓 か」を判断するための先読み用 API。
    /// 状態は何も変更しないので `&self`。
    #[must_use]
    pub fn matches_ime_off(&self, ctx: &InputContext, event: &RawKeyEvent) -> bool {
        matches!(
            self.match_special_keys(ctx, event),
            Some(SpecialKeyMatch::ImeOff)
        )
    }

    /// 変換/無変換系の特殊キーのコンボマッチのみを行う純粋判定メソッド（副作用なし）。
    fn match_special_keys(
        &self,
        ctx: &InputContext,
        event: &RawKeyEvent,
    ) -> Option<SpecialKeyMatch> {
        self.special_keys
            .match_event(
                event,
                ctx.modifiers,
                self.adapter.is_enabled(),
                self.compute_active(ctx),
            )
            .or_else(|| self.match_ime_on_off_auto(ctx, event))
            .or_else(|| self.match_ime_toggle_auto(ctx, event))
    }

    /// 自動検出由来の IME ON/OFF キー（`ime_on_auto`/`ime_off_auto`、ADR-092
    /// 決定D Step4c、GJI config1.db の `GjiImeKeys.on`/`off` 由来）との
    /// マッチ判定。ユーザーが `keys.ime_on`/`ime_off` を明示設定している場合は
    /// 一切参照しない（決定C R1、明示>自動）。
    ///
    /// `event.injected` な合成イベントにはマッチしない（BUG-14: MS-IME/CTF
    /// 由来の注入イベントを信用してはならない、という既存原則。手動設定の
    /// `ime_on`/`ime_off`/`engine_on`/`engine_off` は挙動を変えない
    /// （ユーザーがマクロツール等から意図的に注入する運用を妨げないため）
    /// が、自動検出リストはユーザーが存在を意識せず追加されるため、
    /// 注入イベントへの露出を正当化する根拠が無い。Opus コードレビュー
    /// 指摘）。
    fn match_ime_on_off_auto(
        &self,
        ctx: &InputContext,
        event: &RawKeyEvent,
    ) -> Option<SpecialKeyMatch> {
        if event.injected {
            return None;
        }
        if self.special_keys.ime_on.is_empty()
            && self
                .ime_on_auto
                .iter()
                .any(|k| matches_key_combo(*k, event, ctx.modifiers))
        {
            return Some(SpecialKeyMatch::ImeOn);
        }
        if self.special_keys.ime_off.is_empty()
            && self
                .ime_off_auto
                .iter()
                .any(|k| matches_key_combo(*k, event, ctx.modifiers))
        {
            return Some(SpecialKeyMatch::ImeOff);
        }
        None
    }

    /// 自動検出由来の IME トグルキー（`ime_toggle_auto`）とのマッチ判定
    /// （ADR-092 決定D Step4a/Step4c）。ユーザーが `keys.ime_toggle` を
    /// 明示設定している場合は一切参照しない（決定C R1、明示>自動）。
    /// `event.injected`な合成イベントも対象外（`match_ime_on_off_auto`と
    /// 同じ理由、doc参照）。
    fn match_ime_toggle_auto(
        &self,
        ctx: &InputContext,
        event: &RawKeyEvent,
    ) -> Option<SpecialKeyMatch> {
        if event.injected || !self.special_keys.ime_toggle.is_empty() {
            return None;
        }
        self.ime_toggle_auto
            .iter()
            .any(|k| matches_key_combo(*k, event, ctx.modifiers))
            .then_some(SpecialKeyMatch::ImeToggle)
    }

    /// テスト専用: `match_special_keys` を公開する。
    ///
    /// `SpecialKeyCombos::match_event` の `(!engine_enabled || !engine_active)` ガードは、
    /// engine が既に enabled かつ active（通常運用中）のときは `!engine_enabled` 項が
    /// 効いて EngineOn コンボにマッチしないことが不変条件。`Decision`/`Effect` 経由の
    /// 観測では enabled かつ active な状態での再マッチはほぼ無効果（`force_enable_and_activate`
    /// が実質 no-op を返す）で区別できないため、この不変条件を直接検証する脱出口を設ける。
    /// 本番コードから呼んではならない。
    #[cfg(test)]
    pub(super) fn match_special_keys_for_test(
        &self,
        ctx: &InputContext,
        event: &RawKeyEvent,
    ) -> Option<SpecialKeyMatch> {
        self.match_special_keys(ctx, event)
    }

    /// `user_enabled` を無条件で true にし、IME recovery を伴う activate 処理を行う。
    ///
    /// `SpecialKeyMatch::EngineOn`（`Ctrl+Shift+変換` キーコンボ経由）と
    /// `EngineCommand::ForceEngineOn`（トレイの「状態をリセット」等、外部コマンド経由）の
    /// 両方から呼ばれる共通ロジック。`trigger` はログ表示用のラベル。
    fn force_enable_and_activate(&mut self, ctx: &InputContext, trigger: &str) -> Decision {
        let old_active = self.compute_active(ctx);
        let (_, mut decision) = self.adapter.set_enabled(true);
        let new_active = self.compute_active(ctx);
        log::info!("Engine user_enabled ON ({trigger}, active={new_active})");
        if new_active {
            self.apply_active_transition(old_active, new_active, &mut decision);
        } else {
            // ime_on=false 等で active になれない → pseudo_ctx で IME 強制 ON
            self.apply_engine_on_with_ime_recovery(ctx, &mut decision);
        }
        decision
    }

    /// `SpecialKeyMatch` に応じた状態変更と `Decision` 生成を行う副作用適用メソッド。
    fn apply_special_key_match(&mut self, m: &SpecialKeyMatch, ctx: &InputContext) -> Decision {
        match m {
            SpecialKeyMatch::EngineOn => self.force_enable_and_activate(ctx, "key combo"),
            SpecialKeyMatch::EngineOff => {
                let old_active = self.compute_active(ctx);
                let (_, mut decision) = self.adapter.set_enabled(false);
                let new_active = self.compute_active(ctx);
                log::info!("Engine user_enabled OFF (key combo, active={new_active})");
                self.apply_active_transition(old_active, new_active, &mut decision);
                decision
            }
            SpecialKeyMatch::ImeOn => {
                log::info!("IME ON (key combo)");
                self.build_ime_set_open_decision(ctx, true)
            }
            SpecialKeyMatch::ImeOff => {
                log::info!("IME OFF (key combo)");
                self.build_ime_set_open_decision(ctx, false)
            }
            SpecialKeyMatch::ImeToggle => {
                // `ctx.ime_on` は belief（`InputContext::ime_on`）であり、drift 時は
                // トグル方向が反転しうる——既存の `ImeDetectConfig.toggle` 経由の
                // VK_KANJI トグルと同じ弱点で、新規リスクではない。
                let new_open = !ctx.ime_on;
                log::info!("IME Toggle (key combo) → {new_open}");
                self.build_ime_set_open_decision(ctx, new_open)
            }
        }
    }

    /// 変換/無変換系の特殊キーを一括チェックし、一致した場合は状態変更して結果を返す。
    fn check_special_keys(&mut self, ctx: &InputContext, event: &RawKeyEvent) -> Option<Decision> {
        let m = self.match_special_keys(ctx, event)?;
        Some(self.apply_special_key_match(&m, ctx))
    }
}

#[allow(clippy::suspicious_operation_groupings)]
fn matches_key_combo(combo: ParsedKeyCombo, event: &RawKeyEvent, modifiers: ModifierState) -> bool {
    event.vk_code == combo.vk
        && combo.ctrl == modifiers.ctrl
        && combo.shift == modifiers.shift
        && combo.alt == modifiers.alt
}

impl SpecialKeyCombos {
    /// エンジン有効状態を考慮したうえでコンボマッチを行い、最初に一致した種別を返す。
    ///
    /// 副作用なし。`engine_enabled` は `adapter.is_enabled()` の値を、`engine_active` は
    /// `compute_active(ctx)` の値を渡すこと。
    fn match_event(
        &self,
        event: &RawKeyEvent,
        modifiers: ModifierState,
        engine_enabled: bool,
        engine_active: bool,
    ) -> Option<SpecialKeyMatch> {
        // エンジン ON コンボキー。
        //
        // `!engine_enabled`（ユーザーが明示的に無効化した）だけでなく
        // `!engine_active`（`user_enabled=true` のまま ime_on=false 等の
        // *文脈*で inactive に陥っているケース）でもマッチさせる。後者を
        // 見逃すと、Engine が context 起因で inactive のとき Ctrl+Shift+変換 が
        // 完全に無反応になり（`match_event` が None を返し PassThrough
        // されるだけ）、実測ログで「IME ON だが Engine Off から何をしても
        // 復旧できない」事象の一因になっていた（`force_enable_and_activate` の
        // recovery ロジック自体は存在するのに、この経路からは到達不能だった）。
        if (!engine_enabled || !engine_active)
            && self
                .engine_on
                .iter()
                .any(|k| matches_key_combo(*k, event, modifiers))
        {
            return Some(SpecialKeyMatch::EngineOn);
        }
        if engine_enabled
            && self
                .engine_off
                .iter()
                .any(|k| matches_key_combo(*k, event, modifiers))
        {
            return Some(SpecialKeyMatch::EngineOff);
        }

        // IME 制御キー（エンジン状態に関わらずチェック）
        //
        // 注: 以前は「押されたキーが NICOLA 親指シフトキーなら IME ON/OFF コンボ
        // 判定から除外する」ガードがあったが（Engine の thumb_vks フィールド経由、
        // d8727f5 で導入）、VK_NONCONVERT を親指キーに割り当てた設定では Ctrl+無変換
        // などの特殊コンボが一切効かなくなる回帰を招いたため 9e879cf で除去した。
        // ModifierTiming の grace 猶予廃止（OS 実状態のみ使用）で誤マッチリスクも
        // 解消済み。thumb_vks フィールド自体もその後 write-only の死んだ状態として撤去。
        if self
            .ime_on
            .iter()
            .any(|k| matches_key_combo(*k, event, modifiers))
        {
            log::debug!(
                "[special-key] IME ON match: vk={:#06X} ctrl={} shift={} alt={} extra_info={:#x}",
                event.vk_code,
                modifiers.ctrl,
                modifiers.shift,
                modifiers.alt,
                event.extra_info
            );
            return Some(SpecialKeyMatch::ImeOn);
        }
        if self
            .ime_off
            .iter()
            .any(|k| matches_key_combo(*k, event, modifiers))
        {
            log::debug!(
                "[special-key] IME OFF match: vk={:#06X} ctrl={} shift={} alt={} extra_info={:#x}",
                event.vk_code,
                modifiers.ctrl,
                modifiers.shift,
                modifiers.alt,
                event.extra_info
            );
            return Some(SpecialKeyMatch::ImeOff);
        }
        // ime_on/ime_off（方向固定、明示指定）の後にトグルをチェックする
        // （ADR-092 決定D Step4a、明示方向優先）。
        if self
            .ime_toggle
            .iter()
            .any(|k| matches_key_combo(*k, event, modifiers))
        {
            log::debug!(
                "[special-key] IME Toggle match: vk={:#06X} ctrl={} shift={} alt={} extra_info={:#x}",
                event.vk_code,
                modifiers.ctrl,
                modifiers.shift,
                modifiers.alt,
                event.extra_info
            );
            return Some(SpecialKeyMatch::ImeToggle);
        }

        None
    }
}
