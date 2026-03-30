use std::sync::atomic::Ordering;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::KillTimer;

use awase::config::ParsedKeyCombo;
use awase::engine::Engine;
use awase::types::{ContextChange, FocusKind, KeyAction, KeyEventType, RawKeyEvent, VkCode};
use awase::yab::YabLayout;
use timed_fsm::dispatch;

use crate::focus;
use crate::focus::cache::{DetectionSource, FocusCache};
use crate::focus::pattern::KeyPatternTracker;
use crate::focus::uia::SendableHwnd;
use crate::hook::CallbackResult;
use crate::ime::{HybridProvider, ImeProvider};
use crate::output::Output;
use crate::tray::SystemTray;

// ── FocusDetector（フォーカス検出状態）──

/// フォーカス検出に関するシングルスレッド状態を集約する構造体
pub struct FocusDetector {
    pub cache: FocusCache,
    pub overrides: awase::config::FocusOverrides,
    pub last_focus_info: Option<(u32, String)>,
    pub pattern_tracker: KeyPatternTracker,
    pub uia_sender: Option<std::sync::mpsc::Sender<SendableHwnd>>,
}

impl FocusDetector {
    pub fn new(overrides: awase::config::FocusOverrides) -> Self {
        Self {
            cache: FocusCache::new(),
            overrides,
            last_focus_info: None,
            pattern_tracker: KeyPatternTracker::new(),
            uia_sender: None,
        }
    }

    pub fn set_uia_sender(&mut self, sender: std::sync::mpsc::Sender<SendableHwnd>) {
        self.uia_sender = Some(sender);
    }
}

// ── KeyBuffer（純粋データ構造）──

/// キーイベントバッファ管理
///
/// フック → メッセージループ間のキーイベント遅延・バッファリングを管理する。
/// OS 副作用は持たず、AppState メソッドがオーケストレーションを行う。
pub struct KeyBuffer {
    /// IME 制御キー直後のガードフラグ（true: 後続キーを遅延処理する）
    pub ime_transition_guard: bool,
    /// フォーカス遷移中のガードフラグ（true: フォーカスが安定するまでキーをバッファ）
    pub focus_transition_guard: bool,
    /// ガード中に遅延されたキーイベント + 物理キー状態のバッファ
    pub deferred_keys: Vec<(RawKeyEvent, awase::engine::input_tracker::PhysicalKeyState)>,
    /// IME OFF 時の記憶バッファ（PassThrough 済みキー）
    pub passthrough_memory: std::collections::VecDeque<RawKeyEvent>,
    /// Undetermined + IME ON 時のバッファリング中フラグ
    pub undetermined_buffering: bool,
}

impl KeyBuffer {
    pub fn new() -> Self {
        Self {
            ime_transition_guard: false,
            focus_transition_guard: false,
            deferred_keys: Vec::new(),
            passthrough_memory: std::collections::VecDeque::new(),
            undetermined_buffering: false,
        }
    }

    pub const fn is_guarded(&self) -> bool {
        self.ime_transition_guard
    }

    pub const fn set_guard(&mut self, on: bool) {
        self.ime_transition_guard = on;
    }

    pub fn push_deferred(
        &mut self,
        event: RawKeyEvent,
        phys: awase::engine::input_tracker::PhysicalKeyState,
    ) {
        self.deferred_keys.push((event, phys));
    }

    #[allow(dead_code)]
    pub fn push_passthrough(&mut self, event: RawKeyEvent) {
        self.passthrough_memory.push_back(event);
        if self.passthrough_memory.len() > 20 {
            self.passthrough_memory.pop_front();
        }
    }

    pub fn drain_deferred(
        &mut self,
    ) -> Vec<(RawKeyEvent, awase::engine::input_tracker::PhysicalKeyState)> {
        std::mem::take(&mut self.deferred_keys)
    }

    #[allow(dead_code)] // 将来のパターン検出再有効化で使用予定
    pub fn drain_passthrough(&mut self) -> Vec<RawKeyEvent> {
        std::mem::take(&mut self.passthrough_memory).into()
    }

    #[allow(dead_code)]
    pub const fn is_buffering(&self) -> bool {
        self.undetermined_buffering
    }

    #[allow(dead_code)]
    pub const fn set_buffering(&mut self, on: bool) {
        self.undetermined_buffering = on;
    }

    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.ime_transition_guard = false;
        self.deferred_keys.clear();
        self.passthrough_memory.clear();
        self.undetermined_buffering = false;
    }
}
use crate::{
    matches_key_combo, reinject_key, SendInputExecutor, Win32TimerRuntime, FOCUS_KIND,
    IME_RELIABILITY, IME_STATE_CACHE, TIMER_UNDETERMINED_BUFFER,
};

/// シングルスレッド状態を集約した構造体
pub(crate) struct AppState {
    pub engine: Engine,
    pub tracker: awase::engine::input_tracker::InputTracker,
    pub output: Output,
    pub ime: HybridProvider,
    pub tray: SystemTray,
    pub layouts: Vec<(String, YabLayout, VkCode, VkCode)>,
    pub key_buffer: KeyBuffer,
    pub focus: FocusDetector,
    pub engine_on_keys: Vec<ParsedKeyCombo>,
    pub engine_off_keys: Vec<ParsedKeyCombo>,
    pub shadow_ime_on: bool,
    pub ime_sync_toggle_keys: Vec<u16>,
    pub ime_sync_on_keys: Vec<u16>,
    pub ime_sync_off_keys: Vec<u16>,
}

/// AppState のメソッドが返す副作用指示。
/// メソッドは状態遷移を行い、必要な副作用を AppAction として返す。
/// 呼び出し側が AppAction を実行する。
#[derive(Debug)]
#[allow(dead_code)] // 将来のフェーズで副作用を戻り値として返す際に使用
pub(crate) enum AppAction {
    /// エンジンの保留状態をフラッシュする
    InvalidateEngineContext(ContextChange),
    /// IME 状態キャッシュを更新する
    RefreshImeStateCache,
    /// PassThrough 済みキーを BS で取り消してエンジンで再処理
    RetractPassthroughMemory,
}

impl AppState {
    /// エンジンの有効/無効を切り替え、トレイアイコンを更新する
    pub(crate) fn toggle_engine(&mut self) {
        let (enabled, flush_resp) = self.engine.toggle_enabled();
        let mut timer_runtime = Win32TimerRuntime;
        let mut action_executor = SendInputExecutor;
        dispatch(&flush_resp, &mut timer_runtime, &mut action_executor);
        log::info!("Engine toggled: {}", if enabled { "ON" } else { "OFF" });
        self.tray.set_enabled(enabled);
    }

    /// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
    ///
    /// IMEオフ、入力言語変更など、エンジンの前提が崩れた場合に呼ぶ。
    /// 全てのコンテキスト無効化経路はこのメソッドを通すこと。
    pub(crate) fn invalidate_engine_context(&mut self, reason: ContextChange) {
        let response = self.engine.flush_pending(reason);
        let mut timer_runtime = Win32TimerRuntime;
        let mut action_executor = SendInputExecutor;
        dispatch(&response, &mut timer_runtime, &mut action_executor);
    }

    /// IME ON/OFF 状態をキャッシュに書き込む。
    ///
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    /// フォーカス変更、WM_INPUTLANGCHANGE、IME トグル後、定期ポーリングで呼ばれる。
    ///
    /// # Safety
    /// Win32 API を呼び出す。メインスレッドから呼ぶこと。
    pub(crate) unsafe fn refresh_ime_state_cache(&self) {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
        };

        // Step 1: 対象スレッドの HKL を取得（日本語チェック）
        let mut gui_info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let thread_id = if GetGUIThreadInfo(0, &mut gui_info).is_ok() {
            let fg_hwnd = if gui_info.hwndFocus != HWND::default() {
                gui_info.hwndFocus
            } else {
                gui_info.hwndActive
            };
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg_hwnd, Some(&mut pid))
        } else {
            0
        };

        let hkl = GetKeyboardLayout(thread_id);
        let lang_id = (hkl.0 as u32) & 0xFFFF;
        if lang_id != 0x0411 {
            IME_STATE_CACHE.store(0, Ordering::Release); // 非日本語 → OFF
            return;
        }

        // Step 2: クロスプロセス IME 検出（ブロッキング OK — メッセージループ上）
        let cross_process = crate::ime::detect_ime_open_cross_process();

        // Step 3: ImeReliability を考慮した評価
        let cross_process = if cross_process == Some(false) {
            use awase::types::ImeReliability;
            let reliability = ImeReliability::load(&IME_RELIABILITY);

            // Reliable 以外は CrossProcess=false を信頼しない。
            // Chrome 等の Unknown フレームワークでも CrossProcess が不正確な場合があるため、
            // shadow state にフォールバックする。
            let unreliable = reliability != ImeReliability::Reliable;

            if unreliable { None } else { cross_process }
        } else {
            cross_process
        };

        // Step 4: 最終判定 → キャッシュに書き込み
        let ime_on = match cross_process {
            Some(open) => open,
            None => self.shadow_ime_on, // shadow fallback
        };

        let new_val = if ime_on { 1u8 } else { 0u8 };
        let old_val = IME_STATE_CACHE.swap(new_val, Ordering::AcqRel);
        if old_val != new_val {
            log::debug!(
                "IME state cache updated: {} → {}",
                match old_val { 0 => "OFF", 1 => "ON", _ => "Unknown" },
                if ime_on { "ON" } else { "OFF" },
            );
        }
    }

    /// 配列を動的に切り替える
    pub(crate) fn switch_layout(&mut self, index: usize) {
        let Some((name, layout_template, _l_vk, _r_vk)) = self.layouts.get(index) else {
            log::warn!("Layout index {index} out of range");
            return;
        };

        // YabLayout の各面を clone して新しいレイアウトを構築する
        let new_layout = YabLayout {
            name: layout_template.name.clone(),
            normal: layout_template.normal.clone(),
            left_thumb: layout_template.left_thumb.clone(),
            right_thumb: layout_template.right_thumb.clone(),
            shift: layout_template.shift.clone(),
        };
        let name = name.clone();

        let response = self.engine.swap_layout(new_layout);
        let mut timer_runtime = Win32TimerRuntime;
        let mut action_executor = SendInputExecutor;
        dispatch(&response, &mut timer_runtime, &mut action_executor);

        self.tray.set_layout_name(&name);

        log::info!("Switched layout to: {name}");
    }

    /// エンジン ON/OFF トグルキーをチェックし、一致した場合は状態を変更して結果を返す。
    ///
    /// # Safety
    /// `matches_key_combo` が Win32 API を呼ぶため unsafe。
    pub(crate) unsafe fn check_engine_toggle_keys(&mut self, event: &RawKeyEvent) -> Option<CallbackResult> {
        // エンジン OFF → ON: engine_on_keys（デフォルト: VK_CONVERT）
        if !self.engine.is_enabled() {
            if self.engine_on_keys.iter().any(|k| matches_key_combo(k, event)) {
                let (enabled, flush_resp) = self.engine.set_enabled(true);
                let mut tr = Win32TimerRuntime;
                let mut ae = SendInputExecutor;
                dispatch(&flush_resp, &mut tr, &mut ae);
                log::info!("Engine ON (key combo)");
                self.tray.set_enabled(enabled);
                return Some(CallbackResult::Consumed);
            }
        }

        // エンジン ON → OFF: engine_off_keys（デフォルト: Ctrl+VK_NONCONVERT）
        if self.engine.is_enabled() {
            if self.engine_off_keys.iter().any(|k| matches_key_combo(k, event)) {
                let (enabled, flush_resp) = self.engine.set_enabled(false);
                let mut tr = Win32TimerRuntime;
                let mut ae = SendInputExecutor;
                dispatch(&flush_resp, &mut tr, &mut ae);
                log::info!("Engine OFF (key combo)");
                self.tray.set_enabled(enabled);
                return Some(CallbackResult::Consumed);
            }
        }

        None
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
                    let fg_pid = unsafe { focus::classify::get_window_process_id(fg) };
                    if fg_pid == *prev_pid {
                        log::trace!(
                            "Keeping TextInput (same process {fg_pid}): class={class_name}"
                        );
                        return actions;
                    }
                }
            }
        }

        // UIA 非同期結果のキャッシュ更新用に保存 + パターントラッカーをリセット
        self.focus.last_focus_info = Some((process_id, class_name.to_owned()));
        self.focus.pattern_tracker.clear();
        self.key_buffer.passthrough_memory.clear();
        // Undetermined バッファリング中ならキャンセル
        if self.key_buffer.undetermined_buffering {
            self.key_buffer.undetermined_buffering = false;
            let _ = unsafe { KillTimer(HWND::default(), TIMER_UNDETERMINED_BUFFER) };
            // バッファされたキーは破棄（フォーカスが変わったので無意味）
            self.key_buffer.deferred_keys.clear();
        }

        // IME 信頼度をリセット
        awase::types::ImeReliability::Unknown.store(&IME_RELIABILITY);

        // Config オーバーライド（最高優先度、キャッシュより先に判定）
        if !self.focus.overrides.force_text.is_empty()
            || !self.focus.overrides.force_bypass.is_empty()
        {
            let process_name = unsafe { focus::classify::get_process_name(process_id) };
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
                    actions.push(AppAction::InvalidateEngineContext(ContextChange::FocusChanged));
                    return actions;
                }
            }
        }

        // キャッシュヒット → 即座に結果を適用
        if let Some(cached) = self.focus.cache.get(process_id, class_name) {
            log::trace!(
                "classify_focus: cache hit ({process_id}, {class_name}) → {cached:?}",
            );
            cached.store(&FOCUS_KIND);
            if cached == FocusKind::NonText {
                actions.push(AppAction::InvalidateEngineContext(ContextChange::FocusChanged));
            }
            return actions;
        }

        // Step 1: 評価中は安全側（Undetermined）に設定
        FocusKind::Undetermined.store(&FOCUS_KIND);

        // Step 2: バイパス状態を判定
        let result = unsafe { focus::classify::classify_focus(hwnd) };
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
            actions.push(AppAction::InvalidateEngineContext(ContextChange::FocusChanged));
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
    ///
    /// 現在の `FocusKind` を反転し、学習キャッシュに `UserOverride` で記録する。
    /// `NonText` への降格時はエンジンコンテキストを無効化し、バッファもクリアする。
    pub(crate) unsafe fn toggle_focus_override(&mut self) {
        let current = FocusKind::load(&FOCUS_KIND);
        let new_kind = if current == FocusKind::TextInput {
            FocusKind::NonText
        } else {
            FocusKind::TextInput
        };

        new_kind.store(&FOCUS_KIND);

        // Update learning cache
        if let Some((pid, cls)) = self.focus.last_focus_info.as_ref() {
            self.focus.cache
                .insert(*pid, cls.clone(), new_kind, DetectionSource::UserOverride);
        }

        // If demoted to NonText, flush engine pending
        if new_kind == FocusKind::NonText {
            let response = self.engine.flush_pending(ContextChange::FocusChanged);
            let mut timer_runtime = Win32TimerRuntime;
            let mut action_executor = SendInputExecutor;
            dispatch(&response, &mut timer_runtime, &mut action_executor);
        }

        // Clear any active buffers
        self.key_buffer.deferred_keys.clear();
        self.key_buffer.passthrough_memory.clear();
        self.key_buffer.undetermined_buffering = false;

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

    /// FocusKind を TextInput に昇格させる。
    ///
    /// キャッシュとログの更新を一元化する。
    pub(crate) fn promote_to_text_input(&mut self, source: DetectionSource, reason: &str) {
        let current = FocusKind::load(&FOCUS_KIND);
        if current == FocusKind::TextInput {
            return;
        }
        FocusKind::TextInput.store(&FOCUS_KIND);
        if let Some((pid, cls)) = self.focus.last_focus_info.as_ref() {
            self.focus.cache
                .insert(*pid, cls.clone(), FocusKind::TextInput, source);
        }
        log::info!("Promoting to TextInput: {reason} (source={source:?})");
    }

    /// キー入力パターンを観察し、テキスト入力コンテキストを推定する。
    ///
    /// すべてのキーイベントに対して、FOCUS_KIND バイパスチェックの **前** に呼び出す。
    /// パターンが検出されると `promote_to_text_input` で昇格する。
    #[allow(dead_code)] // 簡略化コールバックからは未使用だが、将来再有効化予定
    pub(crate) fn observe_key_pattern(&mut self, event: &RawKeyEvent) -> Vec<AppAction> {
        let mut actions = Vec::new();

        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );
        if !is_key_down {
            return actions;
        }

        let current = FocusKind::load(&FOCUS_KIND);
        if current == FocusKind::TextInput {
            return actions; // 既に TextInput なら追跡不要
        }

        let is_char = awase::vk::is_modifier_free_char(
            event.vk_code,
            focus::pattern::is_os_modifier_held(),
        );

        if let Some(reason) = self.focus.pattern_tracker.on_key(event.vk_code.0, is_char) {
            self.promote_to_text_input(DetectionSource::TypingPatternInferred, reason);
            self.focus.pattern_tracker.clear();

            // IME OFF + Undetermined で PassThrough 済みキーがある場合、
            // BS で取り消して再処理する
            actions.push(AppAction::RetractPassthroughMemory);
        }

        actions
    }

    /// PassThrough 済みキーを BS で取り消し、エンジンで再処理する。
    ///
    /// IME OFF + Undetermined 状態で PassThrough したキーを、
    /// TextInput に昇格した後に正しく処理し直すために使用する。
    ///
    /// # Safety
    /// Win32 API (`send_keys`, `dispatch`) を呼び出す。メインスレッドから呼ぶこと。
    #[allow(dead_code)] // 将来のパターン検出再有効化で使用予定
    pub(crate) unsafe fn retract_passthrough_memory(&mut self) {
        let keys = self.key_buffer.drain_passthrough();

        if keys.is_empty() {
            return;
        }

        log::debug!(
            "Retracting {} passthrough key(s) with BS + re-process",
            keys.len()
        );

        // BS を送信して PassThrough 済みの文字を取り消す
        {
            let mut bs_actions: Vec<KeyAction> = Vec::new();
            for _ in 0..keys.len() {
                bs_actions.push(KeyAction::Key(0x08)); // VK_BACK down
                bs_actions.push(KeyAction::KeyUp(0x08)); // VK_BACK up
            }
            self.output.send_keys(&bs_actions);
        }

        // エンジンで再処理
        for event in keys {
            let ime_active = self.ime.is_active() && self.ime.get_mode().is_kana_input();

            if ime_active {
                let phys = self.tracker.process(&event);
                let response = self.engine.on_event(event, &phys);
                let mut timer_runtime = Win32TimerRuntime;
                let mut action_executor = SendInputExecutor;
                dispatch(&response, &mut timer_runtime, &mut action_executor);
            }
            // IME OFF のままなら再注入（元々 PassThrough だったので同じ結果）
            // この場合は BS 分が余計だが、IME OFF → パターン検出 → 昇格の流れでは
            // IME が ON になっていることが前提なので通常は engine 経由になる
        }
    }

    /// Undetermined + IME ON バッファリングのタイムアウト処理。
    ///
    /// 300ms 以内にパターン検出されなかった場合、バッファされたキーを
    /// エンジンで処理する（安全側: TextInput として扱う）。
    ///
    /// # Safety
    /// Win32 API (`KillTimer`, `dispatch`) を呼び出す。メインスレッドから呼ぶこと。
    pub(crate) unsafe fn handle_buffer_timeout(&mut self) {
        let _ = KillTimer(HWND::default(), TIMER_UNDETERMINED_BUFFER);
        self.key_buffer.undetermined_buffering = false;
        let keys = self.key_buffer.drain_deferred();

        if keys.is_empty() {
            return;
        }

        log::debug!(
            "Buffer timeout: promoting to TextInput and processing {} buffered key(s)",
            keys.len()
        );

        // タイムアウト → TextInput に昇格してエンジンで処理
        self.promote_to_text_input(
            DetectionSource::TypingPatternInferred,
            "buffer timeout (IME ON + Undetermined)",
        );

        for (event, phys) in keys {
            let response = self.engine.on_event(event, &phys);
            let mut timer_runtime = Win32TimerRuntime;
            let mut action_executor = SendInputExecutor;
            dispatch(&response, &mut timer_runtime, &mut action_executor);
        }
    }

    /// IME 制御キー後に遅延されたキーを再処理する。
    ///
    /// メッセージループから呼ばれるため、この時点で IME 制御キーは OS/IME に
    /// 渡し済みで、IME 状態は最新に更新されている。
    ///
    /// クロスプロセス API で実際の IME 状態を確認し、shadow state も同期する。
    ///
    /// # Safety
    /// Win32 API を呼び出す。メインスレッドから呼ぶこと。
    pub(crate) unsafe fn process_deferred_keys(&mut self) {
        // ガード解除 + バッファからキーを取り出す
        self.key_buffer.set_guard(false);
        let keys = self.key_buffer.drain_deferred();

        if keys.is_empty() {
            return;
        }

        log::debug!("Processing {} deferred key(s) after IME toggle", keys.len());

        // IME 状態キャッシュを更新（メッセージループ上なのでブロッキング OK）
        self.refresh_ime_state_cache();

        // キャッシュから IME 状態を取得
        let cached = IME_STATE_CACHE.load(Ordering::Acquire);
        let ime_on = match cached {
            0 => false,
            1 => true,
            _ => self.shadow_ime_on,
        };

        for (event, phys) in keys {
            if ime_on {
                // IME ON → エンジンで処理（push_deferred 時に保存した phys を使用）
                let response = self.engine.on_event(event, &phys);
                let mut timer_runtime = Win32TimerRuntime;
                let mut action_executor = SendInputExecutor;
                dispatch(&response, &mut timer_runtime, &mut action_executor);
            } else {
                // IME OFF → キーをそのまま再注入（INJECTED_MARKER 付き）
                reinject_key(&event);
            }
        }
    }

    /// Undetermined + IME ON バッファリングのタイムアウトを開始する（初回バッファ時のみ）。
    ///
    /// # Safety
    /// Win32 API (`SetTimer`) を呼び出す。メインスレッドから呼ぶこと。
    #[allow(dead_code)] // 将来の Undetermined バッファリング再有効化で使用予定
    pub(crate) unsafe fn start_buffer_timeout_if_needed(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::SetTimer;
        if !self.key_buffer.undetermined_buffering {
            self.key_buffer.undetermined_buffering = true;
            let _ = SetTimer(HWND::default(), TIMER_UNDETERMINED_BUFFER, 300, None);
        }
    }
}
