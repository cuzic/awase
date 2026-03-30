use windows::Win32::Foundation::HWND;

use awase::config::ParsedKeyCombo;
use awase::engine::Engine;
use awase::types::{ContextChange, FocusKind, ImeCacheState, KeyEventType, RawKeyEvent, VkCode};
use awase::yab::YabLayout;
use timed_fsm::dispatch;

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

// ── KeyBuffer（純粋データ構造）──

/// キーイベントバッファ管理
///
/// フック → メッセージループ間のキーイベント遅延・バッファリングを管理する。
/// OS 副作用は持たず、AppState メソッドがオーケストレーションを行う。
pub struct KeyBuffer {
    /// IME 制御キー直後のガードフラグ（true: 後続キーを遅延処理する）
    pub ime_transition_guard: bool,
    /// ガード中に遅延されたキーイベント + 物理キー状態のバッファ
    pub deferred_keys: Vec<(RawKeyEvent, awase::engine::input_tracker::PhysicalKeyState)>,
}

impl KeyBuffer {
    pub const fn new() -> Self {
        Self {
            ime_transition_guard: false,
            deferred_keys: Vec::new(),
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

    pub fn drain_deferred(
        &mut self,
    ) -> Vec<(RawKeyEvent, awase::engine::input_tracker::PhysicalKeyState)> {
        std::mem::take(&mut self.deferred_keys)
    }
}
use crate::{
    matches_key_combo, reinject_key, SendInputExecutor, Win32TimerRuntime, FOCUS_KIND,
    IME_RELIABILITY, IME_STATE_CACHE,
};

/// IME 同期キー（トグル・ON・OFF）を集約する構造体
pub struct ImeSyncKeys {
    pub toggle: Vec<VkCode>,
    pub on: Vec<VkCode>,
    pub off: Vec<VkCode>,
}

/// エンジン切替・IME 制御の特殊キーコンボを集約する構造体。
pub struct SpecialKeyCombos {
    pub engine_on: Vec<ParsedKeyCombo>,
    pub engine_off: Vec<ParsedKeyCombo>,
    pub ime_on: Vec<ParsedKeyCombo>,
    pub ime_off: Vec<ParsedKeyCombo>,
}

/// シングルスレッド状態を集約した構造体
pub struct AppState {
    pub engine: Engine,
    pub tracker: awase::engine::input_tracker::InputTracker,
    pub output: Output,
    pub ime: HybridProvider,
    pub tray: SystemTray,
    pub layouts: Vec<LayoutEntry>,
    pub key_buffer: KeyBuffer,
    pub focus: FocusDetector,
    pub special_keys: SpecialKeyCombos,
    pub shadow_ime_on: bool,
    pub ime_sync_keys: ImeSyncKeys,
}

/// AppState のメソッドが返す副作用指示。
/// メソッドは状態遷移を行い、必要な副作用を AppAction として返す。
/// 呼び出し側が AppAction を実行する。
#[derive(Debug)]
#[allow(dead_code)] // 将来のフェーズで副作用を戻り値として返す際に使用
pub enum AppAction {
    /// エンジンの保留状態をフラッシュする
    InvalidateEngineContext(ContextChange),
    /// IME 状態キャッシュを更新する
    RefreshImeStateCache,
}

impl AppState {
    /// エンジンの応答を Win32 タイマー + SendInput で実行するヘルパー
    fn dispatch_response(resp: &timed_fsm::Response<awase::types::KeyAction, usize>) {
        let mut tr = Win32TimerRuntime;
        let mut ae = SendInputExecutor;
        dispatch(resp, &mut tr, &mut ae);
    }

    /// エンジンの有効/無効を切り替え、トレイアイコンを更新する
    pub(crate) fn toggle_engine(&mut self) {
        let (enabled, flush_resp) = self.engine.toggle_enabled();
        Self::dispatch_response(&flush_resp);
        log::info!("Engine toggled: {}", if enabled { "ON" } else { "OFF" });
        self.tray.set_enabled(enabled);
    }

    /// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
    ///
    /// IMEオフ、入力言語変更など、エンジンの前提が崩れた場合に呼ぶ。
    /// 全てのコンテキスト無効化経路はこのメソッドを通すこと。
    pub(crate) fn invalidate_engine_context(&mut self, reason: ContextChange) {
        let response = self.engine.flush_pending(reason);
        Self::dispatch_response(&response);
    }

    /// IME ON/OFF 状態をキャッシュに書き込む。
    ///
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    /// フォーカス変更、WM_INPUTLANGCHANGE、IME トグル後、定期ポーリングで呼ばれる。
    pub(crate) fn refresh_ime_state_cache(&mut self) {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
        };

        // Step 1: 対象スレッドの HKL を取得（日本語チェック）
        // SAFETY: Win32 API (GetGUIThreadInfo, GetWindowThreadProcessId, GetKeyboardLayout)。
        //         メインスレッドから呼ぶ。
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
            ImeCacheState::Off.store(&IME_STATE_CACHE); // 非日本語 → OFF
            return;
        }

        // Step 2: クロスプロセス IME 検出（ブロッキング OK — メッセージループ上）
        // SAFETY: detect_ime_open_cross_process は Win32 API。メインスレッドから呼ぶ。
        let cross_process = unsafe { crate::ime::detect_ime_open_cross_process() };

        // Step 3: ImeReliability を考慮した評価
        let cross_process = if cross_process == Some(false) {
            use awase::types::ImeReliability;
            let reliability = ImeReliability::load(&IME_RELIABILITY);

            // Reliable 以外は CrossProcess=false を信頼しない。
            // Chrome 等の Unknown フレームワークでも CrossProcess が不正確な場合があるため、
            // shadow state にフォールバックする。
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
            None => self.shadow_ime_on, // shadow fallback
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
            if ime_on && !self.engine.is_enabled() {
                let _ = self.engine.set_enabled(true);
                self.tray.set_enabled(true);
                log::info!("Engine auto-enabled (IME ON)");
            } else if !ime_on && self.engine.is_enabled() {
                let (_, flush_resp) = self.engine.set_enabled(false);
                Self::dispatch_response(&flush_resp);
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
        let response = self.engine.swap_layout(entry.layout.clone());
        Self::dispatch_response(&response);

        self.tray.set_layout_name(&name);

        log::info!("Switched layout to: {name}");
    }

    /// Shadow IME 状態を更新する（ime_sync キー + IME 制御キー）。
    ///
    /// ime_sync 設定のキーと日本語キーボード固有の IME ON/OFF キーの両方を処理する。
    /// 純粋な状態更新のみ — OS 副作用は呼び出し側が行う。
    pub(crate) fn update_shadow_ime(&mut self, event: &RawKeyEvent) {
        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );
        if !is_key_down {
            return;
        }

        // ── ime_sync 設定キー ──
        let vk = event.vk_code;
        if self.ime_sync_keys.on.contains(&vk) {
            self.shadow_ime_on = true;
            log::debug!("Shadow IME ON (key 0x{:02X})", vk.0);
        }
        if self.ime_sync_keys.off.contains(&vk) {
            self.shadow_ime_on = false;
            log::debug!("Shadow IME OFF (key 0x{:02X})", vk.0);
        }
        if self.ime_sync_keys.toggle.contains(&vk) {
            self.shadow_ime_on = !self.shadow_ime_on;
            log::debug!(
                "Shadow IME toggle → {} (key 0x{:02X})",
                self.shadow_ime_on,
                vk.0
            );
        }

        // ── 日本語キーボード固有の IME ON/OFF キー ──
        if let Some(ime_key) = awase::vk::ImeKeyKind::from_vk(event.vk_code) {
            match ime_key.shadow_effect() {
                awase::vk::ShadowImeEffect::TurnOn => {
                    self.shadow_ime_on = true;
                    log::trace!("Shadow IME ON ({ime_key:?})");
                }
                awase::vk::ShadowImeEffect::TurnOff => {
                    self.shadow_ime_on = false;
                    log::trace!("Shadow IME OFF ({ime_key:?})");
                }
                awase::vk::ShadowImeEffect::Toggle => {
                    self.shadow_ime_on = !self.shadow_ime_on;
                    log::trace!("Shadow IME toggle → {} ({ime_key:?})", self.shadow_ime_on);
                }
            }
        }
    }

    /// IME トグルガードを処理し、キーをバッファリングすべきか判定する。
    ///
    /// IME トグル/ON/OFF キーの直後に続くキーをバッファリングし、
    /// IME の状態遷移が安定してからまとめて処理する。
    ///
    /// 戻り値:
    /// - `Some(CallbackResult)` — 呼び出し側はこれを即座に返すべき
    /// - `None` — ガード処理なし、続行
    pub(crate) fn handle_ime_toggle_guard(
        &mut self,
        event: &RawKeyEvent,
        phys: &awase::engine::input_tracker::PhysicalKeyState,
    ) -> Option<CallbackResult> {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

        let is_key_down = matches!(
            event.event_type,
            KeyEventType::KeyDown | KeyEventType::SysKeyDown
        );

        if is_key_down {
            // Check if current key IS a toggle/on/off key
            let is_toggle_key = self.ime_sync_keys.toggle.contains(&event.vk_code);
            let is_on_key = self.ime_sync_keys.on.contains(&event.vk_code);
            let is_off_key = self.ime_sync_keys.off.contains(&event.vk_code);

            if is_toggle_key || is_on_key || is_off_key {
                // Set guard — next keys will be buffered
                self.key_buffer.set_guard(true);
                log::debug!("IME toggle guard ON (vk=0x{:02X})", event.vk_code.0);
                return Some(CallbackResult::PassThrough); // let IME process the toggle
            }

            // While IME guard active, buffer keys
            if self.key_buffer.is_guarded() {
                self.key_buffer.push_deferred(*event, *phys);
                // SAFETY: PostMessageW は Win32 API。メインスレッドから呼ぶ。
                let _ = unsafe {
                    PostMessageW(
                        HWND::default(),
                        crate::WM_PROCESS_DEFERRED,
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
                return Some(CallbackResult::Consumed);
            }
        }

        // Guard clear on KeyUp of toggle key
        if !is_key_down && self.key_buffer.is_guarded() {
            let is_toggle_key = self.ime_sync_keys.toggle.contains(&event.vk_code);
            let is_on_key = self.ime_sync_keys.on.contains(&event.vk_code);
            let is_off_key = self.ime_sync_keys.off.contains(&event.vk_code);
            if is_toggle_key || is_on_key || is_off_key {
                self.key_buffer.set_guard(false);
                // SAFETY: PostMessageW は Win32 API。メインスレッドから呼ぶ。
                let _ = unsafe {
                    PostMessageW(
                        HWND::default(),
                        crate::WM_PROCESS_DEFERRED,
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
            }
        }

        None
    }

    /// 変換/無変換系の特殊キーを一括チェックし、一致した場合は状態変更して結果を返す。
    ///
    /// チェック順:
    /// 1. エンジン ON/OFF トグルキー（Ctrl+Shift+変換 等）
    /// 2. IME 制御キー（Ctrl+変換 等 → ImmSetOpenStatus）
    ///
    /// 変換/無変換キーは1回だけ consume される。より限定的な修飾キー（Ctrl+Shift）を
    /// 先にチェックし、マッチしなければ緩い修飾（Ctrl のみ）をチェックする。
    pub(crate) fn check_special_keys(&mut self, event: &RawKeyEvent) -> Option<CallbackResult> {
        // エンジントグルを先にチェック（より限定的な修飾キー）
        if !self.engine.is_enabled()
            && self
                .special_keys
                .engine_on
                .iter()
                .any(|k| matches_key_combo(*k, event))
        {
            let (enabled, flush_resp) = self.engine.set_enabled(true);
            Self::dispatch_response(&flush_resp);
            log::info!("Engine ON (key combo)");
            self.tray.set_enabled(enabled);
            return Some(CallbackResult::Consumed);
        }
        if self.engine.is_enabled()
            && self
                .special_keys
                .engine_off
                .iter()
                .any(|k| matches_key_combo(*k, event))
        {
            let (enabled, flush_resp) = self.engine.set_enabled(false);
            Self::dispatch_response(&flush_resp);
            log::info!("Engine OFF (key combo)");
            self.tray.set_enabled(enabled);
            return Some(CallbackResult::Consumed);
        }

        // IME 制御キー（エンジン状態に関わらずチェック）
        if self
            .special_keys
            .ime_on
            .iter()
            .any(|k| matches_key_combo(*k, event))
        {
            let _ = unsafe { crate::ime::set_ime_open_cross_process(true) };
            self.shadow_ime_on = true;
            log::info!("IME ON (ImmSetOpenStatus, key combo)");
            return Some(CallbackResult::Consumed);
        }
        if self
            .special_keys
            .ime_off
            .iter()
            .any(|k| matches_key_combo(*k, event))
        {
            let _ = unsafe { crate::ime::set_ime_open_cross_process(false) };
            self.shadow_ime_on = false;
            log::info!("IME OFF (ImmSetOpenStatus, key combo)");
            return Some(CallbackResult::Consumed);
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
    ///
    /// 現在の `FocusKind` を反転し、学習キャッシュに `UserOverride` で記録する。
    /// `NonText` への降格時はエンジンコンテキストを無効化し、バッファもクリアする。
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
            let response = self.engine.flush_pending(ContextChange::FocusChanged);
            Self::dispatch_response(&response);
        }

        // Clear any active buffers
        self.key_buffer.deferred_keys.clear();
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
    ///
    /// メッセージループから呼ばれるため、この時点で IME 制御キーは OS/IME に
    /// 渡し済みで、IME 状態は最新に更新されている。
    ///
    /// クロスプロセス API で実際の IME 状態を確認し、shadow state も同期する。
    pub(crate) fn process_deferred_keys(&mut self) {
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
        let ime_on = ImeCacheState::load(&IME_STATE_CACHE).resolve_with_shadow(self.shadow_ime_on);

        for (event, phys) in keys {
            if ime_on {
                // IME ON → エンジンで処理（push_deferred 時に保存した phys を使用）
                let response = self.engine.on_event(event, &phys);
                Self::dispatch_response(&response);
            } else {
                // IME OFF → キーをそのまま再注入（INJECTED_MARKER 付き）
                // SAFETY: reinject_key は Win32 API (SendInput)。メインスレッドから呼ぶ。
                unsafe { reinject_key(&event) };
            }
        }
    }
}
