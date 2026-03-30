use windows::Win32::Foundation::HWND;

use awase::engine::{Decision, Effect, Engine, InputContext};
use awase::types::{ContextChange, FocusKind, ImeCacheState, VkCode};
use awase::yab::YabLayout;

use crate::focus;
use crate::focus::cache::{DetectionSource, FocusCache};
use crate::focus::uia::SendableHwnd;
use crate::hook::CallbackResult;
use crate::ime::HybridProvider;
use crate::output::Output;
use crate::tray::SystemTray;

// ── LayoutEntry（名前付きレイアウトエントリ）──

/// レイアウト設定一式を保持する構造体
#[allow(dead_code)] // left/right_thumb_vk はレイアウト切替時に使用予定
pub struct LayoutEntry {
    pub name: String,
    pub layout: YabLayout,
    pub left_thumb_vk: VkCode,
    pub right_thumb_vk: VkCode,
}

// ── FocusDetector（フォーカス検出状態）──

/// フォーカス検出に関するシングルスレッド状態を集約する構造体
pub struct FocusDetector {
    pub cache: FocusCache,
    pub overrides: awase::config::FocusOverrides,
    pub last_focus_info: Option<(u32, String)>,
    pub uia_sender: Option<std::sync::mpsc::Sender<SendableHwnd>>,
}

impl FocusDetector {
    pub fn new(overrides: awase::config::FocusOverrides) -> Self {
        Self {
            cache: FocusCache::new(),
            overrides,
            last_focus_info: None,
            uia_sender: None,
        }
    }

    pub fn set_uia_sender(&mut self, sender: std::sync::mpsc::Sender<SendableHwnd>) {
        self.uia_sender = Some(sender);
    }
}

use crate::{reinject_key, FOCUS_KIND, IME_RELIABILITY, IME_STATE_CACHE};

/// AppState のメソッドが返す副作用指示。
/// メソッドは状態遷移を行い、必要な副作用を AppAction として返す。
/// 呼び出し側が AppAction を実行する。
#[derive(Debug)]
pub enum AppAction {
    /// エンジンの保留状態をフラッシュする
    InvalidateEngineContext(ContextChange),
    /// IME 状態キャッシュを更新する
    RefreshImeStateCache,
}

/// シングルスレッド状態を集約した構造体
pub struct AppState {
    pub engine: Engine,
    pub output: Output,
    #[allow(dead_code)] // IME プロバイダは将来のモード検出で使用予定
    pub ime: HybridProvider,
    pub tray: SystemTray,
    pub layouts: Vec<LayoutEntry>,
    pub focus: FocusDetector,
}

impl AppState {
    /// Decision の副作用を実行する — 唯一の副作用実行ポイント
    pub(crate) fn execute_decision(&mut self, decision: &Decision) -> CallbackResult {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer};

        for effect in &decision.effects {
            match effect {
                Effect::SendKeys(actions) => {
                    self.output.send_keys(actions);
                }
                Effect::ReinjectKey(event) => {
                    // SAFETY: reinject_key は Win32 API (SendInput)。メインスレッドから呼ぶ。
                    unsafe { reinject_key(event) };
                }
                Effect::SetTimer { id, duration } => {
                    let ms = u32::try_from(duration.as_millis()).unwrap_or(u32::MAX);
                    // SAFETY: SetTimer は Win32 API。メインスレッドから呼ぶ。
                    unsafe {
                        let _ = SetTimer(HWND::default(), *id, ms, None);
                    }
                }
                Effect::KillTimer(id) => {
                    // SAFETY: KillTimer は Win32 API。メインスレッドから呼ぶ。
                    unsafe {
                        let _ = KillTimer(HWND::default(), *id);
                    }
                }
                Effect::SetImeOpen(open) => {
                    // SAFETY: set_ime_open_cross_process は Win32 API。メインスレッドから呼ぶ。
                    let _ = unsafe { crate::ime::set_ime_open_cross_process(*open) };
                }
                Effect::RequestImeCacheRefresh => {
                    // SAFETY: PostMessageW は Win32 API。メインスレッドから呼ぶ。
                    unsafe {
                        let _ = PostMessageW(
                            HWND::default(),
                            crate::WM_IME_KEY_DETECTED,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
                Effect::UpdateTray { enabled } => {
                    self.tray.set_enabled(*enabled);
                }
            }
        }

        if decision.consumed {
            CallbackResult::Consumed
        } else {
            CallbackResult::PassThrough
        }
    }

    /// エンジンの有効/無効を切り替え、Decision を実行する
    pub(crate) fn toggle_engine(&mut self) {
        let decision = self.engine.toggle_engine();
        self.execute_decision(&decision);
    }

    /// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
    pub(crate) fn invalidate_engine_context(&mut self, reason: ContextChange) {
        let decision = self.engine.invalidate_engine_context(reason);
        self.execute_decision(&decision);
    }

    /// IME ON/OFF 状態をキャッシュに書き込む。
    ///
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub(crate) fn refresh_ime_state_cache(&mut self) {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
        };

        // Step 1: 対象スレッドの HKL を取得（日本語チェック）
        let lang_id = unsafe {
            let mut gui_info = GUITHREADINFO {
                cbSize: size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            let thread_id = if GetGUIThreadInfo(0, &raw mut gui_info).is_ok() {
                let fg_hwnd = if gui_info.hwndFocus == HWND::default() {
                    gui_info.hwndActive
                } else {
                    gui_info.hwndFocus
                };
                let mut pid = 0u32;
                GetWindowThreadProcessId(fg_hwnd, Some(&raw mut pid))
            } else {
                0
            };

            let hkl = GetKeyboardLayout(thread_id);
            (hkl.0 as u32) & 0xFFFF
        };
        if lang_id != awase::vk::LANGID_JAPANESE {
            ImeCacheState::Off.store(&IME_STATE_CACHE);
            return;
        }

        // Step 2: クロスプロセス IME 検出
        let cross_process = unsafe { crate::ime::detect_ime_open_cross_process() };

        // Step 3: ImeReliability を考慮した評価
        let cross_process = if cross_process == Some(false) {
            use awase::types::ImeReliability;
            let reliability = ImeReliability::load(&IME_RELIABILITY);
            let unreliable = reliability != ImeReliability::Reliable;
            if unreliable {
                None
            } else {
                cross_process
            }
        } else {
            cross_process
        };

        // Step 4: 最終判定 → キャッシュに書き込み
        let ime_on = match cross_process {
            Some(open) => open,
            None => self.engine.shadow_ime_on(),
        };

        let new_state = ImeCacheState::from(ime_on);
        let old_state = new_state.swap(&IME_STATE_CACHE);
        if old_state != new_state {
            log::debug!(
                "IME state cache updated: {} → {}",
                old_state.as_str(),
                new_state.as_str(),
            );
            // エンジンを IME 状態に追随させる
            if ime_on && !self.engine.is_fsm_enabled() {
                let _ = self.engine.fsm.set_enabled(true);
                self.tray.set_enabled(true);
                log::info!("Engine auto-enabled (IME ON)");
            } else if !ime_on && self.engine.is_fsm_enabled() {
                // flush_pending + disable を一括で行う
                let decision = self.engine.invalidate_engine_context(ContextChange::ImeOff);
                self.execute_decision(&decision);
                let _ = self.engine.fsm.set_enabled(false);
                self.tray.set_enabled(false);
                log::info!("Engine auto-disabled (IME OFF)");
            }
        }
    }

    /// 配列を動的に切り替える
    pub(crate) fn switch_layout(&mut self, index: usize) {
        let Some(entry) = self.layouts.get(index) else {
            log::warn!("Layout index {index} out of range");
            return;
        };

        let name = entry.name.clone();
        let decision = self.engine.swap_layout(entry.layout.clone());
        self.execute_decision(&decision);

        self.tray.set_layout_name(&name);

        log::info!("Switched layout to: {name}");
    }

    /// フォーカス変更時の状態遷移を行い、必要な副作用を返す。
    pub(crate) fn on_focus_changed(
        &mut self,
        hwnd: HWND,
        process_id: u32,
        class_name: &str,
    ) -> Vec<AppAction> {
        let mut actions = Vec::new();

        // 同一フォアグラウンドウィンドウ内での TextInput → Undetermined 降格を防止。
        {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            let fg = unsafe { GetForegroundWindow() };
            let current_kind = FocusKind::load(&FOCUS_KIND);
            if current_kind == FocusKind::TextInput {
                if let Some((prev_pid, _)) = &self.focus.last_focus_info {
                    let fg_pid = focus::classify::get_window_process_id(fg);
                    if fg_pid == *prev_pid {
                        log::trace!(
                            "Keeping TextInput (same process {fg_pid}): class={class_name}"
                        );
                        return actions;
                    }
                }
            }
        }

        // UIA 非同期結果のキャッシュ更新用に保存
        self.focus.last_focus_info = Some((process_id, class_name.to_owned()));

        // IME 信頼度をリセット
        awase::types::ImeReliability::Unknown.store(&IME_RELIABILITY);

        // Config オーバーライド（最高優先度、キャッシュより先に判定）
        if !self.focus.overrides.force_text.is_empty()
            || !self.focus.overrides.force_bypass.is_empty()
        {
            let process_name = focus::classify::get_process_name(process_id);
            for entry in &self.focus.overrides.force_text {
                if entry.process.eq_ignore_ascii_case(&process_name)
                    && entry.class.eq_ignore_ascii_case(class_name)
                {
                    log::debug!(
                        "classify_focus: config override force_text ({process_name}, {class_name})",
                    );
                    FocusKind::TextInput.store(&FOCUS_KIND);
                    return actions;
                }
            }
            for entry in &self.focus.overrides.force_bypass {
                if entry.process.eq_ignore_ascii_case(&process_name)
                    && entry.class.eq_ignore_ascii_case(class_name)
                {
                    log::debug!(
                        "classify_focus: config override force_bypass ({process_name}, {class_name})",
                    );
                    FocusKind::NonText.store(&FOCUS_KIND);
                    actions.push(AppAction::InvalidateEngineContext(
                        ContextChange::FocusChanged,
                    ));
                    return actions;
                }
            }
        }

        // キャッシュヒット → 即座に結果を適用
        if let Some(cached) = self.focus.cache.get(process_id, class_name) {
            log::trace!("classify_focus: cache hit ({process_id}, {class_name}) → {cached:?}",);
            cached.store(&FOCUS_KIND);
            if cached == FocusKind::NonText {
                actions.push(AppAction::InvalidateEngineContext(
                    ContextChange::FocusChanged,
                ));
            }
            return actions;
        }

        // Step 1: 評価中は安全側（Undetermined）に設定
        FocusKind::Undetermined.store(&FOCUS_KIND);

        // Step 2: バイパス状態を判定
        let result = focus::classify::classify_focus(hwnd);
        let state = result.kind;

        // Step 3: キャッシュに格納し、FOCUS_KIND を更新
        self.focus.cache.insert(
            process_id,
            class_name.to_owned(),
            state,
            DetectionSource::Automatic,
        );
        state.store(&FOCUS_KIND);

        // Step 4: NonText ならエンジンの保留状態をフラッシュ
        if state == FocusKind::NonText {
            actions.push(AppAction::InvalidateEngineContext(
                ContextChange::FocusChanged,
            ));
        }

        // Step 5: UIA 非同期判定をリクエスト
        if let Some(tx) = self.focus.uia_sender.as_ref() {
            let _ = tx.send(SendableHwnd(hwnd));
        }

        log::debug!(
            "Focus changed: hwnd={:?} class={} reason={} → {:?}",
            hwnd,
            class_name,
            result.reason,
            state,
        );

        // フォーカス変更に伴い IME 状態キャッシュを更新
        actions.push(AppAction::RefreshImeStateCache);
        actions
    }

    /// 手動フォーカスオーバーライドのトグル処理
    pub(crate) fn toggle_focus_override(&mut self) {
        let current = FocusKind::load(&FOCUS_KIND);
        let new_kind = if current == FocusKind::TextInput {
            FocusKind::NonText
        } else {
            FocusKind::TextInput
        };

        new_kind.store(&FOCUS_KIND);

        // Update learning cache
        if let Some((pid, cls)) = self.focus.last_focus_info.as_ref() {
            self.focus
                .cache
                .insert(*pid, cls.clone(), new_kind, DetectionSource::UserOverride);
        }

        // If demoted to NonText, flush engine pending
        if new_kind == FocusKind::NonText {
            self.invalidate_engine_context(ContextChange::FocusChanged);
        }

        // Clear any active buffers
        self.engine.key_buffer.deferred_keys.clear();
        // バルーン通知を表示
        self.tray.show_balloon(
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

    /// IME 制御キー後に遅延されたキーを再処理する。
    pub(crate) fn process_deferred_keys(&mut self) {
        // IME 状態キャッシュを更新（メッセージループ上なのでブロッキング OK）
        self.refresh_ime_state_cache();

        let ctx = InputContext {
            ime_cache: ImeCacheState::load(&IME_STATE_CACHE),
        };
        let decisions = self.engine.process_deferred_keys(&ctx);
        for decision in &decisions {
            self.execute_decision(decision);
        }
    }
}
