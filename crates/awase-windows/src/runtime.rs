use awase::engine::{Engine, EngineCommand, InputContext};
use awase::platform::PlatformRuntime;
use awase::types::{ContextChange, FocusKind, RawKeyEvent, ShadowImeAction, VkCode};

use crate::focus::cache::DetectionSource;
use crate::Preconditions;

/// Config の force_text / force_bypass オーバーライドをチェックする。
/// マッチした場合は強制される FocusKind を返す。
fn check_focus_override(
    overrides: &awase::config::FocusOverrides,
    process_id: u32,
    class_name: &str,
) -> Option<FocusKind> {
    if overrides.force_text.is_empty() && overrides.force_bypass.is_empty() {
        return None;
    }
    let process_name = crate::focus::classify::get_process_name(process_id);
    for entry in &overrides.force_text {
        if entry.process.eq_ignore_ascii_case(&process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
        {
            return Some(FocusKind::TextInput);
        }
    }
    for entry in &overrides.force_bypass {
        if entry.process.eq_ignore_ascii_case(&process_name)
            && entry.class.eq_ignore_ascii_case(class_name)
        {
            return Some(FocusKind::NonText);
        }
    }
    None
}

/// `Preconditions` と `ModifierTiming` から `InputContext` を構築する。
///
/// 修飾キー判定は `GetAsyncKeyState` と `ModifierTiming` の OR で統合。
/// フックベースの追跡により `GetAsyncKeyState` のタイミング問題を回避し、
/// 猶予期間により Ctrl をわずかに早く離した場合でもコンボキーを確実に検出する。
pub fn build_input_context(preconditions: &Preconditions, _timing: &crate::ModifierTiming) -> InputContext {
    let raw = unsafe { crate::observer::focus_observer::read_os_modifiers() };
    // OS 実状態のみ使用。ModifierTiming の grace 猶予は廃止。
    // grace は Ctrl+key コ���ボで Ctrl を早く離した場合の救済だったが、
    // Ctrl+I 直後の親指キーが Ctrl+Nonconvert (IME OFF) と誤検出される問題を引き起こすため除去。
    let modifiers = awase::engine::ModifierState {
        ctrl: raw.ctrl,
        alt: raw.alt,
        shift: raw.shift,
        win: raw.win,
    };
    InputContext {
        ime_on: preconditions.ime_on,
        is_romaji: preconditions.is_romaji,
        is_japanese_ime: preconditions.is_japanese_ime,
        ime_state_reliable: preconditions.ime_detect_miss_count < crate::IME_DETECT_MISS_THRESHOLD,
        modifiers,
        os_modifiers: modifiers,
        left_thumb_down: None,
        right_thumb_down: None,
    }
}
use awase::yab::YabLayout;

use crate::executor::DecisionExecutor;
use crate::hook::CallbackResult;
use crate::ime::HybridProvider;

// ── LayoutEntry（名前付きレイアウトエントリ）──

/// レイアウト設定一式を保持する構造体
#[derive(Debug)]
#[allow(dead_code)] // left/right_thumb_vk はレイアウト切替時に使用予定
pub struct LayoutEntry {
    pub name: String,
    pub layout: YabLayout,
    pub left_thumb_vk: VkCode,
    pub right_thumb_vk: VkCode,
}

// ── FocusDetector（フォーカス検出状態）──

/// フォーカス検出に関するシングルスレッド状態を集約する構造体
#[allow(missing_debug_implementations)]
pub struct FocusDetector {
    pub cache: crate::focus::cache::FocusCache,
    pub overrides: awase::config::FocusOverrides,
    pub last_focus_info: Option<(u32, String)>,
    pub uia_sender: Option<std::sync::mpsc::Sender<crate::focus::uia::SendableHwnd>>,
}

impl FocusDetector {
    pub fn new(overrides: awase::config::FocusOverrides) -> Self {
        Self {
            cache: crate::focus::cache::FocusCache::new(),
            overrides,
            last_focus_info: None,
            uia_sender: None,
        }
    }

    pub fn set_uia_sender(
        &mut self,
        sender: std::sync::mpsc::Sender<crate::focus::uia::SendableHwnd>,
    ) {
        self.uia_sender = Some(sender);
    }
}

/// アプリケーションランタイム。
///
/// Engine (判断) と DecisionExecutor (実行) を保持し、配線する。
/// OS イベントの受け取り → Observer → Engine → Executor のパイプラインを駆動する。
///
/// 注意: 判断ロジックを追加しないこと。判断は Engine が担う。
#[allow(missing_debug_implementations)]
pub struct Runtime {
    pub engine: Engine,
    pub executor: DecisionExecutor,
    #[allow(dead_code)] // IME プロバイダは将来のモード検出で使用予定
    pub ime: HybridProvider,
    pub layouts: Vec<LayoutEntry>,
    /// IME 同期キー（イベント事前分類用）
    pub sync_toggle_keys: Vec<VkCode>,
    pub sync_on_keys: Vec<VkCode>,
    pub sync_off_keys: Vec<VkCode>,
    /// Platform 層の全状態
    pub platform_state: crate::PlatformState,
}

impl Runtime {
    fn build_ctx(&self) -> InputContext {
        build_input_context(&self.platform_state.preconditions, &self.platform_state.modifier_timing)
    }

    /// IME 関連の事前分類情報を sync key 設定で補完する
    pub fn enrich_ime_relevance(&self, event: &mut RawKeyEvent) {
        let vk = event.vk_code;
        let rel = &mut event.ime_relevance;

        if self.sync_toggle_keys.contains(&vk) {
            rel.is_sync_key = true;
            rel.sync_direction = Some(ShadowImeAction::Toggle);
            rel.may_change_ime = true;
        } else if self.sync_on_keys.contains(&vk) {
            rel.is_sync_key = true;
            rel.sync_direction = Some(ShadowImeAction::TurnOn);
            rel.may_change_ime = true;
        } else if self.sync_off_keys.contains(&vk) {
            rel.is_sync_key = true;
            rel.sync_direction = Some(ShadowImeAction::TurnOff);
            rel.may_change_ime = true;
        }
    }

    /// Decision の副作用を実行する（メッセージループ用）。
    pub fn execute_decision(&mut self, decision: awase::engine::Decision) -> CallbackResult {
        self.executor.execute_from_loop(decision)
    }

    /// エンジンの有効/無効を切り替え、Decision を実行する
    pub fn toggle_engine(&mut self) {
        let ctx = self.build_ctx();
        let decision = self.engine.on_command(EngineCommand::ToggleEngine, &ctx);
        self.executor.execute_from_loop(decision);
        // ホットキーコンボ消費後: 猶予を即座にクリアし、
        // 直後のキーが OsModifierHeld でバイパスされるのを防ぐ。
        self.platform_state.modifier_timing.clear_grace();
    }

    /// 外部コンテキスト喪失時にエンジンの保留状態を安全にフラッシュする。
    pub fn invalidate_engine_context(&mut self, reason: ContextChange) {
        let ctx = self.build_ctx();
        let decision = self
            .engine
            .on_command(EngineCommand::InvalidateContext(reason), &ctx);
        self.executor.execute_from_loop(decision);
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
        // ── Phase 1: フォーカス先の検出・分類 ──
        let focus_changed = unsafe { self.detect_and_update_focus() };

        // ── Phase 2: プロセス変更時は Engine に FocusChanged（flush あり）──
        if focus_changed {
            let ctx = self.build_ctx();
            let decision = self.engine.on_command(EngineCommand::FocusChanged, &ctx);
            self.executor.execute_from_loop(decision);
        }

        // ── Phase 3: IME 状態の再取得 ──
        unsafe { crate::observer::ime_observer::observe(&mut self.platform_state.preconditions) };

        // ── Phase 4: Engine に RefreshState（active 遷移検知）──
        let ctx = self.build_ctx();
        let decision = self.engine.on_command(EngineCommand::RefreshState, &ctx);
        self.executor.execute_from_loop(decision);

        // ── Phase 5: 次回ポーリングを自動スケジュール ──
        self.schedule_ime_refresh(u64::from(self.platform_state.ime_poll_interval_ms));
    }

    /// 現在のフォーカス先を検出し、focus_kind / app_kind を更新する。
    ///
    /// 前面プロセスが前回と異なる場合は `true` を返す（flush が必要）。
    /// 同一プロセス内のフォーカス移動では `false` を返す（flush 不要）。
    ///
    /// # Safety
    /// Win32 API を呼び出す。メインスレッドから呼ぶこと。
    unsafe fn detect_and_update_focus(&mut self) -> bool {
        use crate::focus::classify;

        // GetGUIThreadInfo はフォアグラウンドスレッドがハングすると無期限ブロックするため、
        // タイムアウト付きヘルパーを使用する。
        let result = crate::win32::get_gui_thread_info_with_timeout(
            std::time::Duration::from_millis(200),
        );
        let hwnd = result.focused_hwnd;

        if hwnd.0.is_null() {
            return false;
        }

        let process_id = classify::get_window_process_id(hwnd);
        let class_name = classify::get_class_name_string(hwnd);

        // app_kind を更新
        let new_app_kind = crate::observer::focus_observer::detect_app_kind(&class_name);
        if self.platform_state.app_kind != new_app_kind {
            log::info!("AppKind changed: {:?} → {:?} (class={class_name})", self.platform_state.app_kind, new_app_kind);
            self.platform_state.app_kind = new_app_kind;
        }

        // focus_kind を分類
        // Config オーバーライドをチェック
        let (kind, reason, overridden) = if let Some(kind) = check_focus_override(&self.executor.platform.focus.overrides, process_id, &class_name) {
            (kind, "config override".to_string(), true)
        } else if let Some(cached) = self.executor.platform.focus.cache.get(process_id, &class_name) {
            (cached, "cache hit".to_string(), false)
        } else {
            let result = classify::classify_focus(hwnd);
            (result.kind, format!("{}", result.reason), false)
        };

        // focus_kind を更新
        if self.platform_state.focus_kind != kind {
            log::debug!("Focus kind changed: {:?} → {kind:?} (reason={reason})", self.platform_state.focus_kind);
            self.platform_state.focus_kind = kind;
        }

        // キャッシュ格納（オーバーライドでない場合のみ）
        if !overridden {
            self.executor.platform.focus.cache.insert(
                process_id,
                class_name.clone(),
                kind,
                DetectionSource::Automatic,
            );
        }

        // 前面プロセスが変わったかチェック
        let last_pid = self.executor.platform.focus.last_focus_info.as_ref().map(|(pid, _)| *pid);
        let process_changed = last_pid.is_some_and(|last| last != process_id);

        // last_focus_info を更新
        self.executor.platform.focus.last_focus_info = Some((process_id, class_name.clone()));

        // prev_conversion_mode をリセット（異なるウィンドウの conversion_mode を比較しない）
        self.platform_state.preconditions.prev_conversion_mode = 0;

        if process_changed {
            log::debug!("Foreground process changed → FocusChanged (pid={process_id} class={class_name})");

            // UIA 非同期判定が必要かチェック（Undetermined の場合）
            let needs_uia = kind == FocusKind::Undetermined;
            if needs_uia {
                if let Some(sender) = &self.executor.platform.focus.uia_sender {
                    let _ = sender.send(crate::focus::uia::SendableHwnd(hwnd));
                }
            }

            true
        } else {
            // 同一プロセ���内: UIA 判定は必要に応じ���
            if kind == FocusKind::Undetermined {
                if let Some(sender) = &self.executor.platform.focus.uia_sender {
                    let _ = sender.send(crate::focus::uia::SendableHwnd(hwnd));
                }
            }
            false
        }
    }

    /// 統合 IME リフレッシュタイマーをスケジュール（リセット）する。
    ///
    /// 既存のタイマーをキャンセルして `delay_ms` 後に再設定する。
    /// フォーカス変更(50ms) / ポーリング(500ms) / 即時(0ms) を統一的に扱う。
    pub fn schedule_ime_refresh(&mut self, delay_ms: u64) {
        self.executor.platform.timer.set(
            crate::TIMER_IME_REFRESH,
            std::time::Duration::from_millis(delay_ms),
        );
    }

    /// 配列を動的に切り替える
    pub fn switch_layout(&mut self, index: usize) {
        let Some(entry) = self.layouts.get(index) else {
            log::warn!("Layout index {index} out of range");
            return;
        };

        let name = entry.name.clone();
        let decision = self
            .engine
            .on_command(EngineCommand::SwapLayout(entry.layout.clone()), &self.build_ctx());
        self.executor.execute_from_loop(decision);

        self.executor.platform.tray.set_layout_name(&name);

        log::info!("Switched layout to: {name}");
    }

    /// 手動フォーカスオーバーライドのトグル処理
    pub fn toggle_focus_override(&mut self) {
        let current = self.platform_state.focus_kind;
        let new_kind = if current == FocusKind::TextInput {
            FocusKind::NonText
        } else {
            FocusKind::TextInput
        };

        self.platform_state.focus_kind = new_kind;

        // Update learning cache
        if let Some((pid, cls)) = self.executor.platform.focus.last_focus_info.as_ref() {
            self.executor.platform.focus.cache.insert(
                *pid,
                cls.clone(),
                new_kind,
                DetectionSource::UserOverride,
            );
        }

        // If demoted to NonText, flush engine pending
        if new_kind == FocusKind::NonText {
            self.invalidate_engine_context(ContextChange::FocusChanged);
        }

        // バルーン通知を表示
        self.executor.platform.tray.show_balloon(
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
    /// guard 解除 → IME 状態 refresh → バッファキー再処理。
    /// メッセージループ上で呼ぶこと（ブロッキング OK）。
    pub fn process_deferred_keys(&mut self) {
        // Guard を解除
        if self.platform_state.ime_guard.active {
            self.platform_state.ime_guard.active = false;
            log::debug!("IME guard OFF (process_deferred_keys)");
        }

        // Refresh IME state (Observer → Preconditions)
        unsafe { crate::observer::ime_observer::observe(&mut self.platform_state.preconditions) };

        // Drain deferred keys from Platform guard
        let keys: Vec<_> = self.platform_state.ime_guard.deferred_keys.drain(..).collect();
        if keys.is_empty() {
            return;
        }

        log::debug!("Processing {} deferred key(s) after IME toggle", keys.len());

        for (event, _phys) in keys {
            // Build fresh context with updated preconditions
            let ctx = self.build_ctx();
            let decision = self.engine.on_input(event, &ctx);
            self.executor.execute_from_loop(decision);
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
        unsafe { cancel_ime_composition() };
        self.executor.platform.set_ime_open(false);
        self.executor.platform.set_ime_open(true);

        // 3. 全修飾キーの KeyUp を送信（スタック解消）
        send_all_modifier_key_ups();

        // 4. フック再インストール（OS に無言削除されていた場合のリカバリ）
        crate::hook::reinstall_hook();

        // 5. PlatformState を全面リセット
        self.platform_state.preconditions.is_romaji = true;
        self.platform_state.preconditions.ime_on = true; // 安全側: ON
        self.platform_state.preconditions.is_japanese_ime = true;
        self.platform_state.preconditions.prev_conversion_mode = 0;
        self.platform_state.preconditions.ime_detect_miss_count = 0;
        self.platform_state.hook.sent_to_engine = [0u64; 4];
        self.platform_state.hook.track_only_keys = [0u64; 4];
        self.platform_state.hook.in_callback = false;
        self.platform_state.hook.suppress_ctrl_bypass = false;
        self.platform_state.ime_guard.active = false;
        self.platform_state.ime_guard.deferred_keys.clear();
        self.platform_state.modifier_timing = crate::ModifierTiming::new();

        // 6. IME 状態を再取得
        self.refresh_ime_state_cache();

        // 7. バルーン通知
        self.executor
            .platform
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
    const MODIFIER_VKS: [u16; 11] = [
        0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5,
    ];

    let inputs: Vec<INPUT> = MODIFIER_VKS
        .iter()
        .map(|&vk| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: crate::output::INJECTED_MARKER,
                },
            },
        })
        .collect();

    crate::win32::send_input_safe(&inputs);
    log::debug!("Sent KeyUp for all modifier keys");
}

/// IME の未確定文字列をキャンセルする。
///
/// # Safety
/// Win32 IMM API (`ImmGetContext`, `ImmNotifyIME`, `ImmReleaseContext`) を呼び出す。
unsafe fn cancel_ime_composition() {
    use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmNotifyIME, ImmReleaseContext};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let hwnd = GetForegroundWindow();
    if hwnd.0.is_null() {
        return;
    }

    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() {
        return;
    }

    use windows::Win32::UI::Input::Ime::{NOTIFY_IME_ACTION, NOTIFY_IME_INDEX};
    // NI_COMPOSITIONSTR = 0x15, CPS_CANCEL = 0x04
    let _ = ImmNotifyIME(himc, NOTIFY_IME_ACTION(0x15), NOTIFY_IME_INDEX(0x04), 0);
    let _ = ImmReleaseContext(hwnd, himc);
    log::debug!("Cancelled IME composition");
}
