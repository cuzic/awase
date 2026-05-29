//! Windows 実装の `PlatformRuntime`。
//!
//! `Output`, `SystemTray`, フォーカス検出フィールド群, `Win32Timer` を束ね、
//! `PlatformRuntime` トレイトを実装する。

use std::time::Duration;

use awase::platform::PlatformRuntime;
use awase::types::{KeyAction, RawKeyEvent};

use crate::output::Output;
use crate::focus::classifier::{ForceOverrides, ImmCapabilityStore, InjectionHint};
use crate::focus::class_names::AppImeProfile;
use crate::focus::current::CurrentFocus;
use crate::timer::Win32Timer;
use crate::tray::SystemTray;

/// Windows 固有のプラットフォーム実装
pub struct WindowsPlatform {
    pub output: Output,
    pub tray: SystemTray,
    pub timer: Win32Timer,
    /// Engine ON 時に送信する IME モード切り替え VK コード（None で無効）
    pub engine_on_ime_vk: Option<awase::types::VkCode>,
    /// Engine OFF 時に送信する IME モード切り替え VK コード（None で無効）
    pub engine_off_ime_vk: Option<awase::types::VkCode>,
    /// ポーリング/フォーカス変更起因の EngineStateChanged で engine_state_ime_key を
    /// 送らないためのガード。IME 状態変化 → VK 送信 → IME 状態変化の無限ループを防ぐ。
    pub suppress_engine_state_key: bool,
    // ── フォーカス検出フィールド群（旧 AppKindClassifier）──
    pub focus_cache: crate::focus::cache::FocusCache,
    pub focus_overrides: ForceOverrides,
    /// 現在フォーカス中のウィンドウ情報（pid / class_name / app_profile / process_name）。
    pub focus: CurrentFocus,
    pub focus_uia_sender: Option<std::sync::mpsc::Sender<crate::focus::uia::SendableHwnd>>,
    /// IMM 能力の学習・永続化ストア。
    pub imm_learning: ImmCapabilityStore,
    /// per-HWND IME 状態キャッシュ。
    pub hwnd_ime_cache: crate::focus::hwnd_cache::HwndImeCache,
}

impl std::fmt::Debug for WindowsPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowsPlatform").finish_non_exhaustive()
    }
}

impl WindowsPlatform {
    // ── Output 委譲メソッド ──────────────────────────────────────────────────

    /// `applied_ime_on` を指定して eager warmup を送信する。
    pub(crate) fn send_eager_warmup(&self, applied_ime_on: Option<bool>) {
        self.output.send_eager_tsf_warmup(applied_ime_on);
    }

    /// フォーカス変更時の eager warmup（ime_refresh 等から呼ぶ）。
    pub(crate) fn send_eager_warmup_with(&self, applied_ime_on: Option<bool>) {
        self.output.send_eager_tsf_warmup(applied_ime_on);
    }

    /// フォーカス変更時の FocusChange cold マークを Output に通知する（ime_refresh 用）。
    pub(crate) fn mark_composition_cold_focus_change(&self) {
        self.output.mark_composition_cold(crate::output::ColdReason::FocusChange);
    }

    /// フォーカス変更時に injection_mode を更新する（runtime 用）。
    pub(crate) fn update_injection_mode(&mut self, mode: crate::output::types::InjectionMode) {
        self.output.update_injection_mode(mode);
    }

    /// フォーカス変更を Output に通知し、warm epoch をリセットする（runtime 用）。
    pub(crate) fn notify_focus_changed(&self) {
        self.output.on_focus_changed();
    }

    /// TSF モード確定時に TsfGate を Probing に遷移させ、保留キーを返す（runtime 用）。
    pub(crate) fn confirm_tsf(&mut self) -> Vec<awase::types::RawKeyEvent> {
        self.output.confirm_tsf()
    }

    /// 非 TSF モード確定時に TsfGate を Bypass に遷移させ、保留キーを返す（runtime 用）。
    pub(crate) fn bypass_tsf(&mut self) -> Vec<awase::types::RawKeyEvent> {
        self.output.bypass_tsf()
    }

    /// フォーカス変更時に TsfGate を PendingWarmup に遷移させる（bootstrap 用）。
    pub(crate) fn on_focus_change_tsf(&mut self) {
        self.output.on_focus_change_tsf();
    }

    /// TIMER_TSF_GATE タイムアウト時に TsfGate を Bypass にフォールバックし、保留キーを返す。
    pub(crate) fn on_tsf_warmup_timeout(&mut self) -> Vec<awase::types::RawKeyEvent> {
        self.output.on_tsf_warmup_timeout()
    }

    /// キーを TsfGate で処理する。`true` = 保留（呼び出し元は Consumed を返すこと）。
    pub(crate) fn try_hold_key(&mut self, event: awase::types::RawKeyEvent) -> bool {
        self.output.try_hold_key(event)
    }

    /// `composition_warm_epoch` のみリセットする（フォーカス遷移直後の最初キー用）。
    pub(crate) fn reset_warm_epoch(&self) {
        self.output.reset_warm_epoch();
    }

    /// eager warmup F2 を送信した時刻 (ms) を返す。0 = 未送信。
    pub(crate) fn eager_warmup_sent_ms(&self) -> u64 {
        self.output.eager_warmup_sent_ms()
    }

    /// 出力モードを切り替える（設定変更時）。
    pub(crate) fn set_output_mode(&mut self, mode: awase::config::OutputMode) {
        self.output.set_mode(mode);
    }

    /// pending_tsf をインストールし、TIMER_TSF_PROBE を起動する（vk_send async パス用）。
    pub(crate) fn install_pending_tsf_and_set_timer(
        &mut self,
        machine: crate::output::TsfProbeMachine,
    ) {
        self.output.install_pending_tsf(machine);
        if let Some(cmd) = self.output.pending_tsf_timer() {
            self.apply_timer_command(cmd);
        }
    }

    // ── TIMER_TSF_PROBE / raw TSF literal ─────────────────────────────────

    /// TIMER_TSF_PROBE ハンドラ。`Output::step_probe` に委譲し、タイマー命令を実行する。
    pub fn advance_tsf_probe(&mut self) {
        let cmd = self.output.step_probe();
        self.apply_timer_command(cmd);
    }

    /// WM_DRAIN_OUTPUT_QUEUE ハンドラ用: raw TSF literal 回収 + probe タイマーをセット。
    ///
    /// `output.flush_raw_tsf_literal_recovery()` は内部で `send_romaji_as_tsf` を呼ぶため
    /// cold/warm どちらのパスでも `pending_tsf` に probe が積まれることがある。
    /// `platform.send_keys` を経由しないため、ここでタイマー設定を補完する。
    pub fn flush_raw_tsf_literal_recovery(&mut self) {
        self.output.flush_raw_tsf_literal_recovery();
        if let Some(cmd) = self.output.pending_tsf_timer() {
            self.apply_timer_command(cmd);
        }
    }

    /// `TimerCommand` を受け取り、Win32 タイマー操作を実行する。
    pub(crate) fn apply_timer_command(&mut self, cmd: crate::output::TimerCommand) {
        match cmd {
            crate::output::TimerCommand::Continue { id, delay } => self.timer.set(id, delay),
            crate::output::TimerCommand::Kill { id } => self.timer.kill(id),
        }
    }
}

impl PlatformRuntime for WindowsPlatform {
    // ── キー出力 ──

    fn send_keys(&mut self, actions: &[KeyAction]) {
        self.output.send_keys(actions);
        // cold-start 時に pending_tsf が設定された場合は 10ms タイマーを起動してプローブを進める。
        if let Some(cmd) = self.output.pending_tsf_timer() {
            self.apply_timer_command(cmd);
        }
    }

    fn reinject_key(&mut self, event: &RawKeyEvent) {
        use crate::RawKeyEventExt as _;
        unsafe { event.reinject() };
    }

    // ── タイマー ──

    fn set_timer(&mut self, id: usize, duration: Duration) {
        self.timer.set(id, duration);
    }

    fn kill_timer(&mut self, id: usize) {
        self.timer.kill(id);
    }

    // ── IME ──

    fn set_ime_open(&mut self, open: bool) -> bool {
        // IMM API で直接 open/close できないアプリ（Imm32Unavailable / TSF-native）では
        // get_gui_thread_info + send_ime_control が ~200ms タイムアウトしてブロックする。
        // 早期 return して IMM 経由のクロスプロセス呼び出しをスキップする。
        if !self.current_app_profile().can_use_imm32_cross_process() {
            return false;
        }
        // `set_ime_open_cross_process` は SendMessageTimeoutW を含むため、メインスレッドで
        // 同期実行すると `with_app` 再入トリガーになる。ワーカースレッドに offload する
        // async ラッパーを spawn_local で fire-and-forget する。
        // 戻り値の semantics は「dispatch 成功」(= profile 互換) に変更。実際の SendMessage
        // 結果は呼び出し側に届かない（旧 API の sync bool に依存していた診断ログは廃止）。
        win32_async::spawn_local(async move {
            let _ = crate::ime::set_ime_open_cross_process_async(open).await;
        });
        true
    }

    fn apply_ime_open(&mut self, open: bool) -> awase::platform::ImeOpenOutcome {
        let view = self.build_ime_control_view(None);
        crate::ime_controller::CONTROLLER.apply(open, &view)
    }

    fn post_ime_refresh(&mut self) {
        // SetOpen 後の IME 状態反映に数十ms かかるため、即時ではなく
        // 統合タイマー経由で短い遅延後にリフレッシュする。
        // guard が active なら後続キーはバッファされるので安全。
        self.timer.set(
            crate::TIMER_IME_REFRESH,
            Duration::from_millis(20),
        );
    }

    // ── Engine 状態変化時 IME モードキー送信 ──

    fn send_engine_state_ime_key(&self, enabled: bool, applied: Option<bool>) {
        if self.suppress_engine_state_key {
            // ポーリング/フォーカス変化起因の遷移では VK を送らない。
            // 送ると IME 状態が変わり → 次のポーリングでエンジンが逆転 → 無限ループになる。
            log::debug!("[engine-state-key] suppressed (polling/focus-triggered, enabled={enabled})");
            return;
        }
        // apply_ime_open（VK_KANJI or IMM クロスプロセス）が既に IME 状態を確定させている場合、
        // 追加の mode key 送信は不要かつ有害。MS-IME は IME 閉時に VK_DBE_SBCSCHAR を受け取ると
        // 半角英数モードで再オープンする挙動があり、Engine OFF / 実 IME ON の乖離を引き起こす。
        //
        // mode key 送信の本来の用途は「Engine 状態は変わったが IME open/close は変わらない」
        // ケース（例: user_enabled トグルで IME はそのまま）に限定する。
        let last_applied = applied.unwrap_or(false);
        if last_applied == enabled {
            log::debug!(
                "[engine-state-key] skipped (apply_ime_open aligned ime={enabled}, profile={:?})",
                self.current_app_profile()
            );
            return;
        }
        // VK_KANJI トグルで IME を制御するアプリ（Imm32Unavailable: Chrome/Edge）では
        // apply_ime_open が既に VK_KANJI を送信済み。VK_DBE_SBCSCHAR/DBCSCHAR を追加送信すると:
        //   OFF 時: VK_KANJI でクローズ直後に VK_DBE_SBCSCHAR が IME を再オープンする恐れがある。
        //   ON 時: VK_KANJI で開いた後に VK_DBE_DBCSCHAR を送ると全角カタカナモードになりかねない。
        let profile = self.current_app_profile();
        if profile.uses_kanji_toggle() {
            log::debug!("[engine-state-key] skipped (profile={profile:?}, VK_KANJI済み)");
            return;
        }
        let vk = if enabled { self.engine_on_ime_vk } else { self.engine_off_ime_vk };
        if let Some(vk) = vk {
            unsafe { crate::ime::send_ime_mode_key(vk) };
        }
    }

    // ── トレイ ──

    fn update_tray(&mut self, enabled: bool) {
        self.tray.set_enabled(enabled);
    }

    fn show_balloon(&mut self, title: &str, message: &str) {
        self.tray.show_balloon(title, message);
    }

    fn set_tray_layout_name(&mut self, name: &str) {
        self.tray.set_layout_name(name);
    }

    fn composition_output(&self) -> Option<&dyn awase::platform::CompositionOutput> {
        Some(&self.output)
    }

    // ── composition state クエリ / フック ──

    fn output_in_flight_ms(&self) -> u64 {
        self.output.ms_since_last_send()
    }

    fn is_composition_warm(&self) -> bool {
        self.output.is_composition_warm()
    }

    fn is_tsf_mode(&self) -> bool {
        self.output.is_tsf_mode()
    }

    fn on_ime_applied(&mut self, open: bool, outcome: awase::platform::ImeOpenOutcome) {
        use awase::platform::ImeOpenOutcome;
        let effective = match outcome {
            ImeOpenOutcome::Applied | ImeOpenOutcome::FallbackSent | ImeOpenOutcome::AlreadyMatched => open,
            ImeOpenOutcome::Failed => !open,
        };
        crate::tsf::observer::reset_candidate_was_seen();
        if open {
            log::debug!("[composition] ImeEffect::SetOpen(true) → marking cold");
            self.output.mark_composition_cold(crate::output::ColdReason::SetOpenTrue);
            self.output.send_eager_tsf_warmup(Some(effective));
        } else {
            log::debug!("[composition] ImeEffect::SetOpen(false) → marking cold (prevent warm+TSF Enter leak)");
            self.output.mark_composition_cold(crate::output::ColdReason::SetOpenFalse);
        }
    }

    fn on_passthrough_key(
        &mut self,
        vk: awase::types::VkCode,
        is_keydown: bool,
        applied_ime_on: Option<bool>,
    ) -> bool {
        use crate::vk::VkCodeExt as _;
        let applied = applied_ime_on;

        // F2 in TSF mode keydown: NativeF2Consumed (consume decision は executor 側で行う)
        if vk == crate::vk::VK_DBE_HIRAGANA && is_keydown && self.output.is_tsf_mode() {
            log::debug!(
                "[composition] vk=0xf2 passthrough TSF mode → marking cold (NativeF2Consumed)",
            );
            self.output.mark_composition_cold(crate::output::ColdReason::NativeF2Consumed);
            self.output.send_eager_tsf_warmup(applied);
            return false;
        }

        // Confirm key keydown: warm+TSF なら KeyUp まで eager warmup を遅延する
        if is_keydown && vk.is_composition_confirm_key() {
            let was_warm = self.output.is_composition_warm();
            let is_tsf = self.output.is_tsf_mode();
            if was_warm && is_tsf {
                log::debug!(
                    "[composition] passthrough vk={:#04x} KeyDown (warm+TSF) → 変換確定, cold markのみ (eager F2はKeyUpで送信)",
                    vk,
                );
                self.output.mark_composition_cold(crate::output::ColdReason::PassthroughConfirmKey);
                return true; // warmup deferred to KeyUp
            }
            log::debug!(
                "[composition] passthrough vk={:#04x} KeyDown → marking cold + eager warmup",
                vk,
            );
            self.output.mark_composition_cold(crate::output::ColdReason::PassthroughConfirmKey);
            self.output.send_eager_tsf_warmup(applied);
            return false;
        }

        // F2 non-TSF mode keydown
        if vk == crate::vk::VK_DBE_HIRAGANA && is_keydown {
            log::debug!("[composition] vk=0xf2 passthrough direct → marking cold");
            self.output.mark_composition_cold(crate::output::ColdReason::F2NonTsf);
        }
        false
    }

    fn on_reinject_key(
        &mut self,
        vk: awase::types::VkCode,
        is_keydown: bool,
        applied_ime_on: Option<bool>,
    ) {
        use crate::vk::VkCodeExt as _;
        let applied = applied_ime_on;

        if vk == crate::vk::VK_DBE_HIRAGANA && is_keydown && self.output.is_tsf_mode() {
            log::debug!(
                "[reinject-tsf] vk=0xf2 KeyDown TSF mode → marking cold (NativeF2Consumed)",
            );
            self.output.mark_composition_cold(crate::output::ColdReason::NativeF2Consumed);
            self.output.send_eager_tsf_warmup(applied);
            return;
        }

        if is_keydown && vk.is_composition_confirm_key() {
            log::debug!(
                "[composition] reinject KeyDown vk={:#04x} → marking cold + eager warmup",
                vk,
            );
            self.output.mark_composition_cold(crate::output::ColdReason::ReinjectConfirmKey);
            self.output.send_eager_tsf_warmup(applied);
        }
    }
}

impl WindowsPlatform {
    /// `apply_ime_open` 用の `ImeControlView` を構築する。
    ///
    /// `applied` には呼び出し元が持つ `ImeModel.applied_open` と `applied_at_ms` のペアを渡す。
    /// `None` を渡した場合は `(false, 0)`（未適用）として扱う。
    pub(crate) fn build_ime_control_view(
        &self,
        applied: Option<(bool, u64)>,
    ) -> crate::state::ImeControlView<'_> {
        let class_name = if self.focus.is_focused() {
            self.focus.class_name.as_str()
        } else {
            ""
        };
        let (shadow_on, applied_at_ms) = applied.unwrap_or((false, 0));
        crate::state::ImeControlView {
            focus: crate::state::FocusFacts {
                class_name,
                profile: self.current_app_profile(),
            },
            observed: crate::state::ObservedState::capture_now(),
            control: crate::state::ControlLog {
                shadow_on,
                applied_at_ms,
            },
        }
    }

    /// `applied` を明示的に渡す `apply_ime_open` 実装。
    ///
    /// executor が `ImeControlView` の `shadow_on` / `applied_at_ms` を
    /// `applied_snapshot` から直接提供するために使う。
    /// `None` を渡すと `build_ime_control_view_latch()` のフォールバックと同じ動作になる。
    pub(crate) fn apply_ime_open_with_applied(
        &mut self,
        open: bool,
        applied: Option<(bool, u64)>,
    ) -> awase::platform::ImeOpenOutcome {
        let view = self.build_ime_control_view(applied);
        crate::ime_controller::CONTROLLER.apply(open, &view)
    }

    // ── 旧 AppKindClassifier メソッド群 ──

    /// フォーカス中アプリの IME 制御プロファイルを返す。
    #[must_use]
    pub fn current_app_profile(&self) -> AppImeProfile {
        self.focus.app_profile
    }

    /// 現在のフォーカス先に対する注入ヒントを返す。
    #[must_use]
    pub fn injection_hint(&self) -> InjectionHint {
        if !self.focus.is_focused() {
            return InjectionHint::Default;
        }
        self.focus_overrides.injection_hint(self.focus.pid, &self.focus.class_name)
    }

    /// フォーカス情報と `AppImeProfile` キャッシュをアトミックに更新する。
    pub fn update_focus_info(&mut self, process_id: u32, class_name: String) {
        self.focus.update(process_id, class_name);
    }

    /// IMM 能力キャッシュに学習結果を追加し、ファイルに永続化する。
    pub fn learn_imm_capability(&mut self, class_name: String, cap: crate::focus::classifier::ImmCapability) {
        self.imm_learning.learn(class_name, cap);
    }

    /// UIA ワーカーへの送信チャネルを設定する。
    pub fn set_uia_sender(
        &mut self,
        sender: std::sync::mpsc::Sender<crate::focus::uia::SendableHwnd>,
    ) {
        self.focus_uia_sender = Some(sender);
    }
}

