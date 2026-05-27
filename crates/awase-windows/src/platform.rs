//! Windows 実装の `PlatformRuntime`。
//!
//! `Output`, `SystemTray`, `AppKindClassifier`, `Win32Timer` を束ね、
//! `PlatformRuntime` トレイトを実装する。

use std::time::Duration;

use awase::platform::PlatformRuntime;
use awase::types::{KeyAction, RawKeyEvent};

use crate::output::Output;
use crate::focus::classifier::AppKindClassifier;
use crate::timer::Win32Timer;
use crate::tray::SystemTray;

/// Windows 固有のプラットフォーム実装
pub struct WindowsPlatform {
    pub output: Output,
    pub tray: SystemTray,
    pub focus: AppKindClassifier,
    pub timer: Win32Timer,
    /// Engine ON 時に送信する IME モード切り替え VK コード（None で無効）
    pub engine_on_ime_vk: Option<awase::types::VkCode>,
    /// Engine OFF 時に送信する IME モード切り替え VK コード（None で無効）
    pub engine_off_ime_vk: Option<awase::types::VkCode>,
    /// ポーリング/フォーカス変更起因の EngineStateChanged で engine_state_ime_key を
    /// 送らないためのガード。IME 状態変化 → VK 送信 → IME 状態変化の無限ループを防ぐ。
    pub suppress_engine_state_key: bool,
}

impl std::fmt::Debug for WindowsPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowsPlatform").finish_non_exhaustive()
    }
}

impl WindowsPlatform {
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
        if !self.focus.current_app_profile().can_use_imm32_cross_process() {
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
        let view = self.build_ime_control_view();
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

    fn send_engine_state_ime_key(&self, enabled: bool) {
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
        let last_applied = self.output.last_applied_ime_on();
        if last_applied == enabled {
            log::debug!(
                "[engine-state-key] skipped (apply_ime_open aligned ime={enabled}, profile={:?})",
                self.focus.current_app_profile()
            );
            return;
        }
        // VK_KANJI トグルで IME を制御するアプリ（Imm32Unavailable: Chrome/Edge）では
        // apply_ime_open が既に VK_KANJI を送信済み。VK_DBE_SBCSCHAR/DBCSCHAR を追加送信すると:
        //   OFF 時: VK_KANJI でクローズ直後に VK_DBE_SBCSCHAR が IME を再オープンする恐れがある。
        //   ON 時: VK_KANJI で開いた後に VK_DBE_DBCSCHAR を送ると全角カタカナモードになりかねない。
        let profile = self.focus.current_app_profile();
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
}

impl WindowsPlatform {
    /// `apply_ime_open` 用の `ImeControlView` を構築する。
    ///
    /// `WindowsPlatform` が `focus`, `output` の両方を持つため、
    /// 3 ソース（フォーカス分類・観測値・制御ログ）を一箇所で組み立てられる。
    pub(crate) fn build_ime_control_view(&self) -> crate::state::ImeControlView<'_> {
        let class_name = self
            .focus
            .last_focus_info
            .as_ref()
            .map_or("", |(_, c)| c.as_str());
        crate::state::ImeControlView {
            focus: crate::state::FocusFacts {
                class_name,
                profile: self.focus.current_app_profile(),
            },
            observed: crate::state::ObservedState::capture_now(),
            control: crate::state::ControlLog {
                shadow_on: self.output.last_applied_ime_on(),
            },
        }
    }
}

