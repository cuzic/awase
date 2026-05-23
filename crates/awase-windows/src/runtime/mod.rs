mod ime_refresh;
use ime_refresh::ImeRefreshPipeline;

use awase::engine::{Engine, EngineCommand, InputContext};
use awase::platform::PlatformRuntime;
use awase::types::{ContextChange, FocusKind, RawKeyEvent, ShadowImeAction, VkCode};

use crate::focus::cache::DetectionSource;
use crate::Preconditions;

pub use crate::focus::classifier::{AppKindClassifier, ImmCapability, InjectionHint};
pub(crate) use crate::focus::classifier::{check_app_override, is_force_tsf, is_force_vk};

/// `Preconditions` と修飾キースナップショットから `InputContext` を構築する。
///
/// `modifiers` はフック時点でキャプチャした `ModifierState` を渡すこと。
/// タイマー等のイベント非同期パスでは呼び出し元が `read_os_modifiers()` で取得する。
pub fn build_input_context(
    preconditions: &Preconditions,
    modifiers: &awase::engine::ModifierState,
) -> InputContext {
    InputContext {
        ime_on: preconditions.ime_on(),
        input_mode: preconditions.input_mode(),
        is_japanese_ime: preconditions.is_japanese_ime(),
        modifiers: *modifiers,
        left_thumb_down: None,
        right_thumb_down: None,
    }
}
use awase::yab::YabLayout;

use crate::executor::DecisionExecutor;
use crate::hook::CallbackResult;

// ── LayoutEntry（名前付きレイアウトエントリ）──

/// レイアウト設定一式を保持する構造体
#[derive(Debug)]
pub struct LayoutEntry {
    pub name: String,
    pub layout: YabLayout,
}

/// アプリケーションランタイム。
///
/// Engine (判断) と DecisionExecutor (実行) を保持し、配線する。
/// OS イベントの受け取り → Observer → Engine → Executor のパイプラインを駆動する。
///
/// 注意: 判断ロジックを追加しないこと。判断は Engine が担う。
#[allow(missing_debug_implementations)]
pub struct Runtime {
    pub(crate) engine: Engine,
    pub(crate) executor: DecisionExecutor,
    pub(crate) layouts: Vec<LayoutEntry>,
    /// IME 同期キー（イベント事前分類用）
    pub(crate) sync_toggle_keys: Vec<VkCode>,
    pub(crate) sync_on_keys: Vec<VkCode>,
    pub(crate) sync_off_keys: Vec<VkCode>,
    /// Platform 層の全状態
    pub(crate) platform_state: crate::PlatformState,
}

impl Runtime {
    fn build_ctx(&self) -> InputContext {
        let modifiers = unsafe { crate::observer::focus_observer::read_os_modifiers() };
        build_input_context(&self.platform_state.preconditions, &modifiers)
    }

    /// output 層が注入モードを決定するために呼ぶ公開 API。
    ///
    /// focus の `injection_hint()` と `platform_state.app_kind` を組み合わせて
    /// `InjectionHint` を返す。output 層はこのメソッドのみを呼び、
    /// focus/classify の内部型に直接アクセスしない。
    #[must_use]
    pub fn injection_hint(&self) -> (InjectionHint, awase::types::AppKind) {
        (
            self.executor.platform.focus.injection_hint(),
            self.platform_state.app_kind,
        )
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
        ImeRefreshPipeline::new(self).run();
    }

    /// フォーカスプローブ結果を適用する（blocking なし、with_app 内で呼ぶ）。
    /// detect_and_update_focus の fetch 部分を除いた apply のみ。
    /// async drain 後に with_app 内で呼ぶ用途に使う。
    pub fn apply_focus_probe_result(
        &mut self,
        probe: Option<crate::focus::probe::FocusProbe>,
    ) -> bool {
        use crate::focus::hwnd_cache;
        use crate::focus::imm_learning;
        use crate::focus::kind_classifier;

        let Some(probe) = probe else {
            log::warn!("Focus probe timed out — skipping update this cycle");
            return false;
        };

        if probe.process_id == 0 {
            return false;
        }
        let hwnd = probe.hwnd();
        let process_id = probe.process_id;
        let class_name = probe.class_name;

        // app_kind を更新
        let new_app_kind = crate::observer::focus_observer::detect_app_kind(&class_name);

        // ── Phase 2: IMM 能力キャッシュの初期学習 ──
        unsafe {
            imm_learning::learn_imm_capability_on_focus(
                &mut self.executor.platform.focus,
                hwnd,
                &class_name,
                new_app_kind,
            );
        }

        if self.platform_state.app_kind != new_app_kind {
            log::info!(
                "AppKind changed: {:?} → {:?} (class={class_name})",
                self.platform_state.app_kind,
                new_app_kind
            );
            self.platform_state.app_kind = new_app_kind;
        }

        // ── Phase 3: focus_kind を決定 ──
        let resolution = unsafe {
            kind_classifier::resolve_focus_kind(
                &self.executor.platform.focus,
                &self.executor,
                process_id,
                &class_name,
                hwnd,
            )
        };
        let kind = resolution.kind;
        let reason = resolution.reason;
        let overridden = resolution.overridden;

        // focus_kind を更新
        if self.platform_state.focus_kind != kind {
            log::debug!(
                "Focus kind changed: {:?} → {kind:?} (reason={reason})",
                self.platform_state.focus_kind
            );
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
        let last_pid = self
            .executor
            .platform
            .focus
            .last_focus_info
            .as_ref()
            .map(|(pid, _)| *pid);
        let process_changed = last_pid.is_some_and(|last| last != process_id);

        // フォーカス離脱: 現在の preconditions を per-HWND キャッシュに保存
        if process_changed {
            if let Some((old_pid, old_class)) =
                self.executor.platform.focus.last_focus_info.clone()
            {
                hwnd_cache::save_on_focus_leave(
                    &mut self.executor.platform.focus.hwnd_ime_cache,
                    old_pid,
                    old_class,
                    &self.platform_state.preconditions,
                );
            }
        }

        // last_focus_info を更新
        self.executor.platform.focus.last_focus_info = Some((process_id, class_name.clone()));

        // injection_mode を push — focus + app_kind が確定したタイミングで output に通知する
        {
            let hint = self.executor.platform.focus.injection_hint();
            let new_mode = crate::output::types::resolve_injection_mode_from(hint, self.platform_state.app_kind);
            self.executor.platform.output.update_injection_mode(new_mode);
        }

        // prev_conversion_mode をリセット
        self.platform_state.set_prev_conversion_mode(None);

        if process_changed {
            log::info!(
                "FocusChange [{}→{}] {}: stale ime_on={}({:?}) mode={:?} japanese={}",
                last_pid.map_or_else(|| "?".to_string(), |p| p.to_string()),
                process_id,
                class_name,
                self.platform_state.ime_on(),
                self.platform_state.ime_on_source(),
                self.platform_state.input_mode(),
                self.platform_state.is_japanese_ime(),
            );

            self.platform_state.last_focus_change_ms = crate::hook::current_tick_ms();
            self.executor.platform.output.on_focus_changed();
            self.platform_state.clear_ime_observations_on_focus_change();

            {
                let cache_hit = hwnd_cache::restore_on_focus_enter(
                    &self.executor.platform.focus.hwnd_ime_cache,
                    process_id,
                    &class_name,
                );
                self.platform_state.apply_hwnd_cache_restore(cache_hit);
            }

            if self.platform_state.ime_force_on_guard().is_active()
                || self.platform_state.ime_detect_miss_count() > 0
            {
                log::debug!(
                    "Focus changed: clearing ime_force_on_guard and detect_miss_count \
                     (new window may have different IME state)"
                );
                self.platform_state.reset_ime_detect_state();
            }

            if kind == FocusKind::Undetermined {
                if let Some(sender) = &self.executor.platform.focus.uia_sender {
                    let _ = sender.send(crate::focus::uia::SendableHwnd(hwnd));
                }
            }

            true
        } else {
            if kind == FocusKind::Undetermined {
                if let Some(sender) = &self.executor.platform.focus.uia_sender {
                    let _ = sender.send(crate::focus::uia::SendableHwnd(hwnd));
                }
            }
            false
        }
    }

    /// 現在のフォーカス先を検出し、focus_kind / app_kind を更新する。
    ///
    /// 前面プロセスが前回と異なる場合は `true` を返す（flush が必要）。
    /// 同一プロセス内のフォーカス移動では `false` を返す（flush 不要）。
    ///
    /// # Safety
    /// Win32 API を呼び出す。メインスレッドから呼ぶこと。
    unsafe fn detect_and_update_focus(&mut self) -> bool {
        // フォーカス検出全体をワーカースレッドでタイムアウト付き実行する。
        // 詳細は focus::probe::run_focus_probe() を参照。
        let probe = unsafe { crate::focus::probe::run_focus_probe() };
        self.apply_focus_probe_result(probe)
    }

    /// IME リフレッシュを非同期タスクとしてスポーン。
    /// with_app の外でフェッチを行い、完了後に with_app で適用する。
    pub fn spawn_ime_refresh(&mut self) {
        self.executor.platform.timer.kill(crate::TIMER_IME_REFRESH);

        // phase3_7 (T≈270ms) より先に eager warmup F2 を送る。これにより打鍵時の cold path で
        // used_eager_path=true となり unicode fallback が有効になる（先頭文字欠け対策）。
        if self.platform_state.focus_transition_pending {
            self.executor.platform.output.send_eager_tsf_warmup();
        }

        win32_async::spawn_local(async {
            let focus = crate::focus::probe::run_focus_probe_async().await;
            let snap = crate::ime::detect_ime_state_async().await;
            crate::with_app(|app| {
                ImeRefreshPipeline::new(app).run_with_prefetched(focus, snap);

                // run_with_prefetched 完了後: last_focus_info が更新済みのため
                // is_force_tsf を直接読んで TsfGate を遷移させる。
                // PendingWarmup 以外（Probing/Ready/Bypass）なら空 Vec が返る。
                let is_tsf = app
                    .executor
                    .platform
                    .focus
                    .last_focus_info
                    .as_ref()
                    .is_some_and(|(pid, class)| {
                        is_force_tsf(&app.executor.platform.focus.overrides, *pid, class)
                    });
                let held = if is_tsf {
                    app.executor.platform.output.confirm_tsf()
                } else {
                    app.executor.platform.output.bypass_tsf()
                };
                // PendingWarmup から遷移した場合のみ held が非空 or タイマー kill が必要。
                // 500ms ポーリング等で再呼び出しされても on_tsf_confirmed/on_bypass は
                // PendingWarmup 以外では空 Vec を返すため安全。
                app.executor.platform.timer.kill(crate::TIMER_TSF_GATE);
                if !held.is_empty() {
                    log::debug!("[tsf-gate] draining {} held keys via INPUT_DEFER", held.len());
                    crate::INPUT_DEFER.replay_later(held);
                }
            });
        });
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

    /// 手動アプリオーバーライドのトグル処理
    pub fn toggle_app_override(&mut self) {
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
        if self.platform_state.ime_guard.is_active() {
            self.platform_state.ime_guard.deactivate();
            log::debug!("IME guard OFF (process_deferred_keys)");
        }

        // Refresh IME state (Observer → ImeObservations → Preconditions)
        let observer_out = unsafe {
            crate::observer::ime_observer::observe(
                self.platform_state.ime_on(),
                self.platform_state.ime_force_on_guard(),
                self.platform_state.input_mode(),
                self.platform_state.prev_conversion_mode(),
            )
        };
        self.platform_state.apply_ime_observer_output(&observer_out);
        self.platform_state.apply_ime_observations(self.engine.is_user_enabled());

        // Drain deferred keys from Platform guard
        let keys = self.platform_state.ime_guard.drain_all();
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
        // panic_reset 直後に refresh_ime_state_cache() が走ると、ここで書いた
        // ime_on=true を stale な observe() 結果が即座に上書きしてしまう。
        // force_on_guard で 1 サイクルだけ保護し、次の検出成功時に自然に解除する。
        self.platform_state.apply_panic_reset();
        self.platform_state.hook.reset_routing();
        self.platform_state.hook.leave_callback();
        self.platform_state.hook.set_suppress_ctrl_bypass(false);
        self.platform_state.ime_guard.deactivate();
        self.platform_state.ime_guard.clear();

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
    use windows::Win32::UI::Input::Ime::{ImmNotifyIME, NOTIFY_IME_ACTION, NOTIFY_IME_INDEX};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let Some(hwnd) = crate::win32::non_null_hwnd(GetForegroundWindow()) else {
        return;
    };
    let Some(ctx) = (unsafe { crate::imm::ImmContextGuard::new(hwnd) }) else {
        return;
    };
    // NI_COMPOSITIONSTR = 0x15, CPS_CANCEL = 0x04
    let _ = unsafe { ImmNotifyIME(ctx.himc(), NOTIFY_IME_ACTION(0x15), NOTIFY_IME_INDEX(0x04), 0) };
    log::debug!("Cancelled IME composition");
}

