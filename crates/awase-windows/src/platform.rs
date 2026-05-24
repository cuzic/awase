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
#[allow(missing_debug_implementations)]
pub struct WindowsPlatform {
    pub output: Output,
    pub tray: SystemTray,
    pub focus: AppKindClassifier,
    pub timer: Win32Timer,
    /// Engine ON 時に送信する IME モード切り替え VK コード（None で無効）
    pub engine_on_ime_vk: Option<u16>,
    /// Engine OFF 時に送信する IME モード切り替え VK コード（None で無効）
    pub engine_off_ime_vk: Option<u16>,
    /// ポーリング/フォーカス変更起因の EngineStateChanged で engine_state_ime_key を
    /// 送らないためのガード。IME 状態変化 → VK 送信 → IME 状態変化の無限ループを防ぐ。
    pub suppress_engine_state_key: bool,
}

impl WindowsPlatform {
    /// TIMER_TSF_PROBE ハンドラ。pending_tsf フェーズを進め、完了したらタイマーを kill する。
    pub fn advance_tsf_probe(&mut self) {
        if self.output.advance_tsf_probe() {
            self.timer.kill(crate::TIMER_TSF_PROBE);
            self.output.on_tsf_probe_ready();
        }
    }

    /// WM_DRAIN_OUTPUT_QUEUE ハンドラ用: raw TSF literal 回収 + probe タイマーをセット。
    ///
    /// `output.flush_raw_tsf_literal_recovery()` は内部で `send_romaji_as_tsf` を呼ぶため
    /// cold/warm どちらのパスでも `pending_tsf` に probe が積まれることがある。
    /// `platform.send_keys` を経由しないため、ここでタイマー設定を補完する。
    pub fn flush_raw_tsf_literal_recovery(&mut self) {
        self.output.flush_raw_tsf_literal_recovery();
        if self.output.pending_tsf.borrow().is_some() {
            self.timer.set(crate::TIMER_TSF_PROBE, Duration::from_millis(10));
        }
    }
}

impl PlatformRuntime for WindowsPlatform {
    // ── キー出力 ──

    fn send_keys(&mut self, actions: &[KeyAction]) {
        self.output.send_keys(actions);
        // cold-start 時に pending_tsf が設定された場合は 10ms タイマーを起動してプローブを進める。
        if self.output.pending_tsf.borrow().is_some() {
            self.timer.set(crate::TIMER_TSF_PROBE, Duration::from_millis(10));
        }
    }

    fn reinject_key(&mut self, event: &RawKeyEvent) {
        unsafe { crate::reinject_key(event) };
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
        unsafe { crate::ime::set_ime_open_cross_process(open) }
    }

    fn apply_ime_open(&mut self, open: bool) -> awase::platform::ImeOpenOutcome {
        static CONTROLLER: crate::ime_controller::ImeController =
            crate::ime_controller::ImeController::new();
        let class_name = self
            .focus
            .last_focus_info
            .as_ref()
            .map_or("", |(_, c)| c.as_str());
        let ctx = crate::ime_controller::ImeApplyContext {
            class_name,
            profile: self.focus.current_app_profile(),
            shadow_on: self.output.last_applied_ime_on(),
            candidate_visible: crate::tsf::observer::with_tsf_obs(|obs| obs.gji_candidate_visible()),
        };
        CONTROLLER.apply(open, &ctx)
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
